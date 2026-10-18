/*
 * The JS side of the Aethus host (see host.h and bridge.h).
 *
 * - The value table: what an externref is. wasm-bindgen keeps every JS value Rust holds in the
 *   module's externref table (`__wbindgen_externrefs`), so a value the wasm references outside
 *   that table only lives between an import's return and the store that follows it. A handle
 *   here is a slot index plus one; aethus_js_collect frees the slots the table no longer points
 *   at, between tasks, when no wasm frame is on the stack.
 * - Traps and exceptions crossing between the wasm and JS (bridge.h).
 * - WebAssembly.Instance: the glue's imports looked up by name, the translation instantiated,
 *   its exports wrapped (bridge_gen.c), its memory and externref table as JS objects.
 * - The natives env.js builds the browser on (__native).
 * - The module loader: `host/...` is js_root, `web/...` is web_root (the published build).
 */
#ifdef _WIN32
#define _CRT_RAND_S
#endif
#include <stdarg.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#else
#include <time.h>
#endif

#include "bridge.h"
#include "host.h"

JSContext *g_ctx;
w2c_wenilla g_inst;
JSValue *g_imports;
int g_wasm_depth;

static JSRuntime *g_rt;
static bool g_instantiated;
static char *g_js_root, *g_web_root;
static JSValue g_host_tick, g_host_frame;

/* The instance pointers the translation hands each import; never dereferenced. */
static char g_glue_env, g_wasi_env;

/* ---- logging ---------------------------------------------------------------------------- */

static void say(int level, const char *format, ...)
{
   char text[4096];
   va_list args;
   va_start(args, format);
   int n = vsnprintf(text, sizeof text, format, args);
   va_end(args);
   if (n < 0)
      return;
   if ((size_t)n >= sizeof text)
      n = (int)sizeof text - 1;
   aethus_log(level, text, (size_t)n);
}

static void log_exception(JSContext *ctx, JSValueConst exc, const char *where)
{
   const char *message = JS_ToCString(ctx, exc);
   if (!message)
      JS_FreeValue(ctx, JS_GetException(ctx));
   JSValue stack = JS_IsObject(exc) ? JS_GetPropertyStr(ctx, exc, "stack") : JS_UNDEFINED;
   const char *trace = JS_IsString(stack) ? JS_ToCString(ctx, stack) : NULL;
   say(3, "%s: %s%s%s", where, message ? message : "(unprintable exception)", trace ? "\n" : "",
       trace ? trace : "");
   if (trace)
      JS_FreeCString(ctx, trace);
   if (message)
      JS_FreeCString(ctx, message);
   JS_FreeValue(ctx, stack);
}

static void log_pending_exception(const char *where)
{
   JSValue exc = JS_GetException(g_ctx);
   log_exception(g_ctx, exc, where);
   JS_FreeValue(g_ctx, exc);
}

/* ---- the clock ---------------------------------------------------------------------------- */

double aethus_now_ms(void)
{
#ifdef _WIN32
   static LARGE_INTEGER frequency, start;
   LARGE_INTEGER now;
   if (!frequency.QuadPart) {
      QueryPerformanceFrequency(&frequency);
      QueryPerformanceCounter(&start);
   }
   QueryPerformanceCounter(&now);
   return (double)(now.QuadPart - start.QuadPart) * 1000.0 / (double)frequency.QuadPart;
#else
   static struct timespec start;
   struct timespec now;
   if (!start.tv_sec && !start.tv_nsec)
      clock_gettime(CLOCK_MONOTONIC, &start);
   clock_gettime(CLOCK_MONOTONIC, &now);
   return (double)(now.tv_sec - start.tv_sec) * 1000.0 + (double)(now.tv_nsec - start.tv_nsec) / 1e6;
#endif
}

/* ---- the value table ---------------------------------------------------------------------- */

/* Attribute each time interval to exactly one side of the bridge. Nested wasm callbacks and
 * exception unwinds restore the saved lane, so an import does not count its wasm callback as
 * JavaScript time. -3 is idle, -2 is JS outside an import, -1 is translated wasm. */
typedef struct { double ms; uint64_t calls; } BridgeProfile;
static BridgeProfile *g_profile;
static int g_profile_lane = -3;
static double g_profile_last;

static void profile_switch(int lane)
{
   if (!g_profile)
      return;
   double now = aethus_now_ms();
   if (g_profile_lane >= -2)
      g_profile[g_profile_lane + 2].ms += now - g_profile_last;
   g_profile_last = now;
   g_profile_lane = lane;
}

void aethus_js_profile_report(void)
{
   if (!g_profile)
      return;
   say(1, "bridge perf: wasm %.1fms, JS outside imports %.1fms (exclusive)",
       g_profile[1].ms, g_profile[0].ms);
   for (int n = 0; n < 12; n++) {
      int best = -1;
      for (int i = 0; i < bridge_import_count; i++)
         if (g_profile[i + 2].ms > 0 && (best < 0 || g_profile[i + 2].ms > g_profile[best + 2].ms))
            best = i;
      if (best < 0)
         break;
      say(1, "bridge perf: %s: %llu calls, %.1fms exclusive", bridge_import_names[best],
          (unsigned long long)g_profile[best + 2].calls, g_profile[best + 2].ms);
      g_profile[best + 2].ms = 0;
   }
   memset(g_profile, 0, (bridge_import_count + 2) * sizeof *g_profile);
   JSMemoryUsage usage;
   JS_ComputeMemoryUsage(g_rt, &usage);
   say(1, "memory: wasm %.1fMiB, JS allocated %.1fMiB, JS objects %lld, GC threshold %.1fMiB",
       g_instantiated ? (double)bridge_memory()->size / 1048576.0 : 0,
       (double)usage.malloc_size / 1048576.0, (long long)usage.obj_count,
       (double)JS_GetGCThreshold(g_rt) / 1048576.0);
}

/* Slots 0..2 are undefined, true and false, never collected. */
#define FIXED_SLOTS 3
enum { SLOT_FREE, SLOT_USED, SLOT_MARKED };

static JSValue *slot_value;
static uint8_t *slot_state;
static uint32_t *slot_next;
static uint32_t slot_capacity, slot_top, slot_free_head = UINT32_MAX, slot_live;

/* Handles carried by a JS exception in flight through the wasm. */
static wasm_rt_externref_t *pinned;
static int pinned_count, pinned_capacity;

static void slots_grow(void)
{
   uint32_t capacity = slot_capacity ? slot_capacity * 2 : 1u << 16;
   slot_value = realloc(slot_value, capacity * sizeof *slot_value);
   slot_state = realloc(slot_state, capacity * sizeof *slot_state);
   slot_next = realloc(slot_next, capacity * sizeof *slot_next);
   if (!slot_value || !slot_state || !slot_next) {
      say(3, "out of memory growing the value table to %u slots", capacity);
      abort();
   }
   memset(slot_state + slot_capacity, SLOT_FREE, capacity - slot_capacity);
   slot_capacity = capacity;
}

static void slots_init(void)
{
   slots_grow();
   slot_value[0] = JS_UNDEFINED;
   slot_value[1] = JS_TRUE;
   slot_value[2] = JS_FALSE;
   slot_state[0] = slot_state[1] = slot_state[2] = SLOT_USED;
   slot_top = FIXED_SLOTS;
}

wasm_rt_externref_t bridge_js_to_ref(JSValueConst value)
{
   switch (JS_VALUE_GET_TAG(value)) {
   case JS_TAG_NULL:
      return NULL;
   case JS_TAG_UNDEFINED:
      return (wasm_rt_externref_t)(uintptr_t)1;
   case JS_TAG_BOOL:
      return (wasm_rt_externref_t)(uintptr_t)(JS_VALUE_GET_BOOL(value) ? 2 : 3);
   default:
      break;
   }
   uint32_t slot;
   if (slot_free_head != UINT32_MAX) {
      slot = slot_free_head;
      slot_free_head = slot_next[slot];
   } else {
      if (slot_top == slot_capacity)
         slots_grow();
      slot = slot_top++;
   }
   slot_value[slot] = JS_DupValue(g_ctx, value);
   slot_state[slot] = SLOT_USED;
   slot_live++;
   return (wasm_rt_externref_t)(uintptr_t)(slot + 1);
}

JSValue bridge_ref_to_js(wasm_rt_externref_t ref)
{
   uintptr_t handle = (uintptr_t)ref;
   if (!handle)
      return JS_NULL;
   uint32_t slot = (uint32_t)(handle - 1);
   if (slot >= slot_top || slot_state[slot] == SLOT_FREE) {
      say(3, "externref %u is not a live value (collected too early?)", slot);
      return JS_UNDEFINED;
   }
   return JS_DupValue(g_ctx, slot_value[slot]);
}

static void pin(wasm_rt_externref_t ref)
{
   if (pinned_count == pinned_capacity) {
      pinned_capacity = pinned_capacity ? pinned_capacity * 2 : 16;
      pinned = realloc(pinned, pinned_capacity * sizeof *pinned);
   }
   pinned[pinned_count++] = ref;
}

static void unpin(wasm_rt_externref_t ref)
{
   for (int i = pinned_count - 1; i >= 0; i--) {
      if (pinned[i] == ref) {
         pinned[i] = pinned[--pinned_count];
         return;
      }
   }
}

static void mark(wasm_rt_externref_t ref)
{
   uintptr_t handle = (uintptr_t)ref;
   if (handle > FIXED_SLOTS && handle - 1 < slot_top && slot_state[handle - 1] == SLOT_USED)
      slot_state[handle - 1] = SLOT_MARKED;
}

void aethus_js_collect(void)
{
   if (!g_instantiated || g_wasm_depth)
      return;
   wasm_rt_externref_table_t *table = bridge_externref_table();
   for (uint32_t i = 0; i < table->size; i++)
      mark(table->data[i]);
   for (int i = 0; i < pinned_count; i++)
      mark(pinned[i]);
   uint32_t top = slot_top;
   for (uint32_t slot = FIXED_SLOTS; slot < top; slot++) {
      if (slot_state[slot] == SLOT_MARKED) {
         slot_state[slot] = SLOT_USED;
      } else if (slot_state[slot] == SLOT_USED) {
         JSValue value = slot_value[slot];
         slot_value[slot] = JS_UNDEFINED;
         slot_state[slot] = SLOT_FREE;
         slot_next[slot] = slot_free_head;
         slot_free_head = slot;
         slot_live--;
         JS_FreeValue(g_ctx, value);
      }
   }
}

/* ---- traps and exceptions ----------------------------------------------------------------- */

/* The tag of a JS exception thrown into the wasm; its payload is the value's handle. */
static const char g_js_exception_tag_storage = 0;
#define JS_EXCEPTION_TAG ((wasm_rt_tag_t)&g_js_exception_tag_storage)

/* A wasm exception carried through JS frames, rethrown with its own tag if it re-enters. */
typedef struct {
   wasm_rt_tag_t tag;
   uint32_t size;
   uint8_t data[256];
} WasmExceptionPayload;

static JSClassID g_payload_class;

static void payload_finalizer(JSRuntime *rt, JSValueConst value)
{
   (void)rt;
   free(JS_GetOpaque(value, g_payload_class));
}

void bridge_enter(BridgeSaved *saved)
{
   saved->profile_lane = g_profile_lane;
   profile_switch(-1);
   memcpy(&saved->jmp, &g_wasm_rt_jmp_buf, sizeof saved->jmp);
   saved->unwind_target = wasm_rt_get_unwind_target();
   wasm_rt_set_unwind_target(&g_wasm_rt_jmp_buf);
   g_wasm_depth++;
}

void bridge_leave(const BridgeSaved *saved)
{
   profile_switch(saved->profile_lane);
   g_wasm_depth--;
   memcpy(&g_wasm_rt_jmp_buf, &saved->jmp, sizeof saved->jmp);
   wasm_rt_set_unwind_target(saved->unwind_target);
}

void bridge_after_wasm(void)
{
   bridge_mem_check();
}

JSValue bridge_wasm_failure(JSContext *ctx, int code)
{
   bridge_mem_check();
   if (code == WASM_RT_TRAP_UNCAUGHT_EXCEPTION) {
      wasm_rt_tag_t tag = wasm_rt_exception_tag();
      if (tag == JS_EXCEPTION_TAG) {
         wasm_rt_externref_t ref;
         memcpy(&ref, wasm_rt_exception(), sizeof ref);
         unpin(ref);
         return JS_Throw(ctx, bridge_ref_to_js(ref));
      }
      WasmExceptionPayload *payload = malloc(sizeof *payload);
      payload->tag = tag;
      payload->size = wasm_rt_exception_size();
      memcpy(payload->data, wasm_rt_exception(), payload->size);
      JSValue carrier = JS_NewObjectClass(ctx, g_payload_class);
      JS_SetOpaque(carrier, payload);
      JSValue error = JS_NewError(ctx);
      JS_SetPropertyStr(ctx, error, "name", JS_NewString(ctx, "WebAssembly.Exception"));
      JS_SetPropertyStr(ctx, error, "message", JS_NewString(ctx, "wasm exception"));
      JS_SetPropertyStr(ctx, error, "__wasmException", carrier);
      return JS_Throw(ctx, error);
   }
   say(3, "wasm trap: %s", wasm_rt_strerror((wasm_rt_trap_t)code));
   JSValue error = JS_NewError(ctx);
   JS_SetPropertyStr(ctx, error, "name", JS_NewString(ctx, "RuntimeError"));
   JS_SetPropertyStr(ctx, error, "message", JS_NewString(ctx, wasm_rt_strerror((wasm_rt_trap_t)code)));
   JS_SetPropertyStr(ctx, error, "__wasmTrap", JS_NewInt32(ctx, code));
   return JS_Throw(ctx, error);
}

WASM_RT_NO_RETURN void bridge_throw_into_wasm(void)
{
   JSValue exc = JS_GetException(g_ctx);
   if (JS_IsObject(exc)) {
      JSValue trap = JS_GetPropertyStr(g_ctx, exc, "__wasmTrap");
      if (JS_IsNumber(trap)) {
         int32_t code = WASM_RT_TRAP_UNREACHABLE;
         JS_ToInt32(g_ctx, &code, trap);
         JS_FreeValue(g_ctx, exc);
         wasm_rt_trap((wasm_rt_trap_t)code);
      }
      JS_FreeValue(g_ctx, trap);
      JSValue carrier = JS_GetPropertyStr(g_ctx, exc, "__wasmException");
      WasmExceptionPayload *payload = JS_GetOpaque(carrier, g_payload_class);
      if (payload) {
         WasmExceptionPayload copy = *payload;
         JS_FreeValue(g_ctx, carrier);
         JS_FreeValue(g_ctx, exc);
         wasm_rt_load_exception(copy.tag, copy.size, copy.data);
         wasm_rt_throw();
      }
      JS_FreeValue(g_ctx, carrier);
   }
   /* An import that is not wrapped in handleError threw: in the browser too this unwinds the
    * wasm as a foreign exception. It is worth seeing. */
   log_exception(g_ctx, exc, "a JS exception is unwinding the wasm");
   wasm_rt_externref_t ref = bridge_js_to_ref(exc);
   pin(ref);
   JS_FreeValue(g_ctx, exc);
   wasm_rt_load_exception(JS_EXCEPTION_TAG, sizeof ref, &ref);
   wasm_rt_throw();
}

JSValue bridge_call_import(int idx, int argc, JSValue *argv)
{
   int lane = g_profile_lane;
   profile_switch(idx);
   if (g_profile)
      g_profile[idx + 2].calls++;
   bridge_mem_check();
   JSValue result = JS_Call(g_ctx, g_imports[idx], JS_UNDEFINED, argc, argv);
   for (int i = 0; i < argc; i++)
      JS_FreeValue(g_ctx, argv[i]);
   profile_switch(lane);
   if (JS_IsException(result))
      bridge_throw_into_wasm();
   return result;
}

JSValue bridge_multi_ii(JSContext *ctx, struct wasm_multi_ii r)
{
   JSValue array = JS_NewArray(ctx);
   JS_SetPropertyUint32(ctx, array, 0, JS_NewInt32(ctx, (int32_t)r.i0));
   JS_SetPropertyUint32(ctx, array, 1, JS_NewInt32(ctx, (int32_t)r.i1));
   return array;
}

/* ---- linear memory and the externref table as JS objects --------------------------------- */

static JSValue g_memory_buffer = JS_UNDEFINED;
static uint8_t *g_memory_data;
static uint64_t g_memory_size;

void bridge_mem_check(void)
{
   if (!g_instantiated)
      return;
   wasm_rt_memory_t *memory = bridge_memory();
   if (memory->data == g_memory_data && memory->size == g_memory_size)
      return;
   if (!JS_IsUndefined(g_memory_buffer)) {
      JS_DetachArrayBuffer(g_ctx, g_memory_buffer);
      JS_FreeValue(g_ctx, g_memory_buffer);
      g_memory_buffer = JS_UNDEFINED;
   }
   g_memory_data = memory->data;
   g_memory_size = memory->size;
}

static JSValue memory_buffer(JSContext *ctx, JSValueConst this_val)
{
   (void)this_val;
   bridge_mem_check();
   if (JS_IsUndefined(g_memory_buffer)) {
      g_memory_buffer = JS_NewArrayBuffer(ctx, g_memory_data, (size_t)g_memory_size, 0, NULL, NULL, false);
      if (JS_IsException(g_memory_buffer)) {
         g_memory_buffer = JS_UNDEFINED;
         return JS_EXCEPTION;
      }
   }
   return JS_DupValue(ctx, g_memory_buffer);
}

static JSValue memory_grow(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv)
{
   (void)this_val;
   uint32_t delta;
   if (JS_ToUint32(ctx, &delta, argc > 0 ? argv[0] : JS_UNDEFINED))
      return JS_EXCEPTION;
   uint64_t old = wasm_rt_grow_memory(bridge_memory(), delta);
   if (old == (uint64_t)-1)
      return JS_ThrowRangeError(ctx, "WebAssembly.Memory.grow(): could not grow by %u pages", delta);
   bridge_mem_check();
   return JS_NewInt64(ctx, (int64_t)old);
}

static const JSCFunctionListEntry memory_functions[] = {
   JS_CGETSET_DEF("buffer", memory_buffer, NULL),
   JS_CFUNC_DEF("grow", 1, memory_grow),
};

static JSValue table_get(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv)
{
   (void)this_val;
   uint32_t index;
   if (JS_ToUint32(ctx, &index, argc > 0 ? argv[0] : JS_UNDEFINED))
      return JS_EXCEPTION;
   wasm_rt_externref_table_t *table = bridge_externref_table();
   if (index >= table->size)
      return JS_ThrowRangeError(ctx, "WebAssembly.Table.get(): index %u out of bounds", index);
   return bridge_ref_to_js(table->data[index]);
}

static JSValue table_set(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv)
{
   (void)this_val;
   uint32_t index;
   if (JS_ToUint32(ctx, &index, argc > 0 ? argv[0] : JS_UNDEFINED))
      return JS_EXCEPTION;
   wasm_rt_externref_table_t *table = bridge_externref_table();
   if (index >= table->size)
      return JS_ThrowRangeError(ctx, "WebAssembly.Table.set(): index %u out of bounds", index);
   table->data[index] = bridge_js_to_ref(argc > 1 ? argv[1] : JS_UNDEFINED);
   return JS_UNDEFINED;
}

static JSValue table_grow(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv)
{
   (void)this_val;
   uint32_t delta;
   if (JS_ToUint32(ctx, &delta, argc > 0 ? argv[0] : JS_UNDEFINED))
      return JS_EXCEPTION;
   wasm_rt_externref_t init = bridge_js_to_ref(argc > 1 ? argv[1] : JS_UNDEFINED);
   uint32_t old = wasm_rt_grow_externref_table(bridge_externref_table(), delta, init);
   if (old == UINT32_MAX)
      return JS_ThrowRangeError(ctx, "WebAssembly.Table.grow(): could not grow by %u", delta);
   return JS_NewUint32(ctx, old);
}

static JSValue table_length(JSContext *ctx, JSValueConst this_val)
{
   (void)this_val;
   return JS_NewUint32(ctx, bridge_externref_table()->size);
}

static const JSCFunctionListEntry table_functions[] = {
   JS_CFUNC_DEF("get", 1, table_get),
   JS_CFUNC_DEF("set", 2, table_set),
   JS_CFUNC_DEF("grow", 2, table_grow),
   JS_CGETSET_DEF("length", table_length, NULL),
};

/* __native.instantiate(importObject): the glue's WebAssembly.Instance. */
static JSValue native_instantiate(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv)
{
   (void)this_val;
   if (g_instantiated)
      return JS_ThrowInternalError(ctx, "the module is already instantiated (one instance per process)");
   if (argc < 1 || !JS_IsObject(argv[0]))
      return JS_ThrowTypeError(ctx, "WebAssembly.Instance(): the import object is not an object");
   JSValue module = JS_GetPropertyStr(ctx, argv[0], "./wenilla_bg.js");
   if (!JS_IsObject(module)) {
      JS_FreeValue(ctx, module);
      return JS_ThrowTypeError(ctx, "WebAssembly.Instance(): no imports for module \"./wenilla_bg.js\"");
   }
   JSValue *imports = calloc((size_t)bridge_import_count, sizeof *imports);
   for (int i = 0; i < bridge_import_count; i++) {
      JSValue fn = JS_GetPropertyStr(ctx, module, bridge_import_names[i]);
      if (!JS_IsFunction(ctx, fn)) {
         JS_FreeValue(ctx, fn);
         for (int k = 0; k < i; k++)
            JS_FreeValue(ctx, imports[k]);
         free(imports);
         JS_FreeValue(ctx, module);
         return JS_ThrowTypeError(ctx, "WebAssembly.Instance(): Import #%d \"./wenilla_bg.js\" \"%s\": function import requires a callable",
                                  i, bridge_import_names[i]);
      }
      imports[i] = fn;
   }
   JS_FreeValue(ctx, module);
   g_imports = imports;

   /* Instantiation copies the data segments and runs no code of the module's (wasm-bindgen
    * modules start through __wbindgen_start), but a segment out of bounds traps. */
   BRIDGE_GUARDED(ctx, wasm2c_wenilla_instantiate(&g_inst, (struct w2c_0x2E0x2Fwenilla__bg0x2Ejs *)&g_glue_env,
                                                  (struct w2c_wasi__snapshot__preview1 *)&g_wasi_env));
   g_instantiated = true;
   bridge_mem_check();

   JSValue exports = JS_NewObject(ctx);
   for (int i = 0; i < bridge_export_count; i++) {
      const BridgeExport *e = &bridge_exports[i];
      JS_SetPropertyStr(ctx, exports, e->name, JS_NewCFunction2(ctx, e->fn, e->name, e->length, JS_CFUNC_generic, 0));
   }
   JSValue memory = JS_NewObject(ctx);
   JS_SetPropertyFunctionList(ctx, memory, memory_functions, sizeof memory_functions / sizeof memory_functions[0]);
   JS_SetPropertyStr(ctx, exports, bridge_memory_export, memory);
   JSValue table = JS_NewObject(ctx);
   JS_SetPropertyFunctionList(ctx, table, table_functions, sizeof table_functions / sizeof table_functions[0]);
   JS_SetPropertyStr(ctx, exports, bridge_table_export, table);
   say(1, "wasm instantiated: %d imports, %d exports, %llu bytes of memory", bridge_import_count,
       bridge_export_count, (unsigned long long)bridge_memory()->size);
   return exports;
}

/* ---- natives ------------------------------------------------------------------------------ */

/* A typed array's (or ArrayBuffer's) bytes; NULL with *len = 0 when it is neither or detached. */
static uint8_t *bytes_of(JSContext *ctx, JSValueConst value, size_t *len)
{
   *len = 0;
   if (!JS_IsObject(value))
      return NULL;
   if (JS_GetTypedArrayType(value) >= 0) {
      size_t offset, length, per_element;
      JSValue buffer = JS_GetTypedArrayBuffer(ctx, value, &offset, &length, &per_element);
      if (JS_IsException(buffer)) {
         JS_FreeValue(ctx, JS_GetException(ctx));
         return NULL;
      }
      size_t size;
      uint8_t *data = JS_GetArrayBuffer(ctx, &size, buffer);
      JS_FreeValue(ctx, buffer);
      if (!data) {
         if (JS_HasException(ctx))
            JS_FreeValue(ctx, JS_GetException(ctx));
         return NULL;
      }
      *len = length;
      return data + offset;
   }
   if (JS_IsArrayBuffer(value)) {
      size_t size;
      uint8_t *data = JS_GetArrayBuffer(ctx, &size, value);
      if (!data) {
         if (JS_HasException(ctx))
            JS_FreeValue(ctx, JS_GetException(ctx));
         return NULL;
      }
      *len = size;
      return data;
   }
   return NULL;
}

static JSValue native_log(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv)
{
   (void)this_val;
   int32_t level = 1;
   if (argc > 0)
      JS_ToInt32(ctx, &level, argv[0]);
   size_t len = 0;
   const char *text = argc > 1 ? JS_ToCStringLen2(ctx, &len, argv[1], false) : NULL;
   if (!text)
      return argc > 1 ? JS_EXCEPTION : JS_UNDEFINED;
   aethus_log(level, text, len);
   JS_FreeCString(ctx, text);
   return JS_UNDEFINED;
}

static JSValue native_now(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv)
{
   (void)this_val;
   (void)argc;
   (void)argv;
   return JS_NewFloat64(ctx, aethus_now_ms());
}

/* TextEncoder.encode: a fresh Uint8Array of the string's UTF-8. */
static JSValue native_text_encode(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv)
{
   (void)this_val;
   size_t len;
   const char *text = JS_ToCStringLen2(ctx, &len, argc > 0 ? argv[0] : JS_UNDEFINED, false);
   if (!text)
      return JS_EXCEPTION;
   JSValue array = JS_NewUint8ArrayCopy(ctx, (const uint8_t *)text, len);
   JS_FreeCString(ctx, text);
   return array;
}

/* TextEncoder.encodeInto(string, view) -> [read (UTF-16 units), written (bytes)]: as many whole
 * characters as fit. */
static JSValue native_text_encode_into(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv)
{
   (void)this_val;
   size_t len;
   const char *text = JS_ToCStringLen2(ctx, &len, argc > 0 ? argv[0] : JS_UNDEFINED, false);
   if (!text)
      return JS_EXCEPTION;
   size_t room;
   uint8_t *out = bytes_of(ctx, argc > 1 ? argv[1] : JS_UNDEFINED, &room);
   size_t written = 0, read = 0;
   while (written < len) {
      uint8_t lead = (uint8_t)text[written];
      size_t n = lead < 0x80 ? 1 : lead < 0xE0 ? 2 : lead < 0xF0 ? 3 : 4;
      if (written + n > len || written + n > room)
         break;
      written += n;
      read += n == 4 ? 2 : 1;
   }
   if (written)
      memcpy(out, text, written);
   JS_FreeCString(ctx, text);
   JSValue result = JS_NewArray(ctx);
   JS_SetPropertyUint32(ctx, result, 0, JS_NewInt64(ctx, (int64_t)read));
   JS_SetPropertyUint32(ctx, result, 1, JS_NewInt64(ctx, (int64_t)written));
   return result;
}

/* TextDecoder.decode(view) for UTF-8. */
static JSValue native_text_decode(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv)
{
   (void)this_val;
   size_t len;
   uint8_t *data = bytes_of(ctx, argc > 0 ? argv[0] : JS_UNDEFINED, &len);
   if (!data)
      return JS_NewStringLen(ctx, "", 0);
   return JS_NewStringLen(ctx, (const char *)data, len);
}

/* Reviewed wasm-bindgen getStringFromWasm0 path. Avoid creating a typed-array view and
 * crossing several interpreted helpers for every small string. Read the current memory
 * pointer each time (the Windows allocator and PS5 fallback may move on growth). */
static JSValue native_text_decode_wasm(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv)
{
   (void)this_val;
   uint32_t ptr, len;
   if (argc < 2)
      return JS_ThrowTypeError(ctx, "textDecodeWasm requires a pointer and length");
   if (JS_ToUint32(ctx, &ptr, argv[0]) || JS_ToUint32(ctx, &len, argv[1]))
      return JS_EXCEPTION;
   if (!g_instantiated)
      return JS_ThrowTypeError(ctx, "wasm memory is not instantiated");
   wasm_rt_memory_t *memory = bridge_memory();
   if ((uint64_t)ptr + len > memory->size)
      return JS_ThrowRangeError(ctx, "string is outside wasm memory");
   return JS_NewStringLen(ctx, len ? (const char *)memory->data + ptr : "", len);
}

/* A latin-1 decode (TextDecoder('latin1'), really windows-1252 for 0x80..0x9F, which callers
 * here never meet). */
static JSValue native_latin1_decode(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv)
{
   (void)this_val;
   size_t len;
   uint8_t *data = bytes_of(ctx, argc > 0 ? argv[0] : JS_UNDEFINED, &len);
   if (!data)
      return JS_NewStringLen(ctx, "", 0);
   char *utf8 = malloc(len * 2 + 1);
   size_t n = 0;
   for (size_t i = 0; i < len; i++) {
      uint8_t b = data[i];
      if (b < 0x80) {
         utf8[n++] = (char)b;
      } else {
         utf8[n++] = (char)(0xC0 | (b >> 6));
         utf8[n++] = (char)(0x80 | (b & 0x3F));
      }
   }
   JSValue s = JS_NewStringLen(ctx, utf8, n);
   free(utf8);
   return s;
}

/* An XMLHttpRequest's responseText under overrideMimeType('text/plain; charset=x-user-defined'):
 * byte b < 0x80 is U+00bb, b >= 0x80 is U+F780 + (b - 0x80). */
static JSValue native_x_user_defined(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv)
{
   (void)this_val;
   size_t len;
   uint8_t *data = bytes_of(ctx, argc > 0 ? argv[0] : JS_UNDEFINED, &len);
   if (!data)
      return JS_NewStringLen(ctx, "", 0);
   char *utf8 = malloc(len * 3 + 1);
   if (!utf8)
      return JS_ThrowOutOfMemory(ctx);
   size_t n = 0;
   for (size_t i = 0; i < len; i++) {
      uint8_t b = data[i];
      if (b < 0x80) {
         utf8[n++] = (char)b;
      } else {
         uint32_t c = 0xF780u + (b - 0x80u);
         utf8[n++] = (char)(0xE0 | (c >> 12));
         utf8[n++] = (char)(0x80 | ((c >> 6) & 0x3F));
         utf8[n++] = (char)(0x80 | (c & 0x3F));
      }
   }
   JSValue s = JS_NewStringLen(ctx, utf8, n);
   free(utf8);
   return s;
}

static JSValue native_random_fill(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv)
{
   (void)this_val;
   size_t len;
   uint8_t *data = bytes_of(ctx, argc > 0 ? argv[0] : JS_UNDEFINED, &len);
   if (!data)
      return JS_ThrowTypeError(ctx, "getRandomValues: not a typed array");
#ifdef _WIN32
   for (size_t i = 0; i < len;) {
      unsigned int r;
      rand_s(&r);
      for (int k = 0; k < 4 && i < len; k++, i++)
         data[i] = (uint8_t)(r >> (8 * k));
   }
#else
   arc4random_buf(data, len);
#endif
   return JS_DupValue(ctx, argv[0]);
}

static void drain_jobs(void)
{
   for (;;) {
      JSContext *job_ctx;
      int r = JS_ExecutePendingJob(g_rt, &job_ctx);
      if (r == 0)
         break;
      if (r < 0) {
         JSValue exc = JS_GetException(job_ctx);
         log_exception(job_ctx, exc, "uncaught (in a promise job)");
         JS_FreeValue(job_ctx, exc);
      }
   }
}

/* __native.drainJobs(): a microtask checkpoint, for env.js to run between the tasks it drives. */
static JSValue native_drain_jobs(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv)
{
   (void)ctx;
   (void)this_val;
   (void)argc;
   (void)argv;
   drain_jobs();
   return JS_UNDEFINED;
}

#define MAX_CALL_ARGS 64

/* __native.call(op, ...args): an operation of the embedder's. */
static JSValue native_call(JSContext *ctx, JSValueConst this_val, int argc, JSValueConst *argv)
{
   (void)this_val;
   uint32_t op;
   if (argc < 1 || JS_ToUint32(ctx, &op, argv[0]))
      return JS_EXCEPTION;
   int n = argc - 1;
   if (n > MAX_CALL_ARGS)
      return JS_ThrowRangeError(ctx, "__native.call: too many arguments (%d)", n);
   AethusValue args[MAX_CALL_ARGS];
   const char *strings[MAX_CALL_ARGS];
   int string_count = 0;
   for (int i = 0; i < n; i++) {
      JSValueConst v = argv[i + 1];
      AethusValue *a = &args[i];
      memset(a, 0, sizeof *a);
      switch (JS_VALUE_GET_TAG(v)) {
      case JS_TAG_UNDEFINED:
         a->kind = AETHUS_UNDEFINED;
         break;
      case JS_TAG_NULL:
         a->kind = AETHUS_NULL;
         break;
      case JS_TAG_BOOL:
         a->kind = AETHUS_BOOL;
         a->number = JS_VALUE_GET_BOOL(v) ? 1 : 0;
         break;
      case JS_TAG_INT:
         a->kind = AETHUS_NUMBER;
         a->number = JS_VALUE_GET_INT(v);
         break;
      case JS_TAG_FLOAT64:
         a->kind = AETHUS_NUMBER;
         a->number = JS_VALUE_GET_FLOAT64(v);
         break;
      case JS_TAG_STRING: {
         size_t len;
         const char *s = JS_ToCStringLen2(ctx, &len, v, false);
         if (!s) {
            for (int k = 0; k < string_count; k++)
               JS_FreeCString(ctx, strings[k]);
            return JS_EXCEPTION;
         }
         strings[string_count++] = s;
         a->kind = AETHUS_STRING;
         a->ptr = (uint8_t *)s;
         a->len = len;
         break;
      }
      case JS_TAG_OBJECT: {
         size_t len;
         uint8_t *data = bytes_of(ctx, v, &len);
         if (data || JS_GetTypedArrayType(v) >= 0 || JS_IsArrayBuffer(v)) {
            a->kind = AETHUS_BYTES;
            a->ptr = data;
            a->len = len;
         } else {
            a->kind = AETHUS_OBJECT;
         }
         break;
      }
      default: {
         double d;
         if (JS_ToFloat64(ctx, &d, v)) {
            JS_FreeValue(ctx, JS_GetException(ctx));
            a->kind = AETHUS_OBJECT;
         } else {
            a->kind = AETHUS_NUMBER;
            a->number = d;
         }
         break;
      }
      }
   }
   AethusValue ret;
   memset(&ret, 0, sizeof ret);
   aethus_rust_call(op, args, (uint32_t)n, &ret);
   for (int k = 0; k < string_count; k++)
      JS_FreeCString(ctx, strings[k]);
   JSValue result;
   switch (ret.kind) {
   case AETHUS_NULL:
      result = JS_NULL;
      break;
   case AETHUS_BOOL:
      result = JS_NewBool(ctx, ret.number != 0);
      break;
   case AETHUS_NUMBER:
      result = JS_NewFloat64(ctx, ret.number);
      break;
   case AETHUS_STRING:
      result = JS_NewStringLen(ctx, (const char *)ret.ptr, ret.len);
      break;
   case AETHUS_BYTES:
      result = JS_NewUint8ArrayCopy(ctx, ret.ptr, ret.len);
      break;
   case AETHUS_ERROR: {
      JSValue error = JS_NewError(ctx);
      JS_SetPropertyStr(ctx, error, "message", JS_NewStringLen(ctx, (const char *)ret.ptr, ret.len));
      result = JS_Throw(ctx, error);
      break;
   }
   default:
      result = JS_UNDEFINED;
      break;
   }
   if (ret.ptr)
      aethus_rust_free(ret.ptr, ret.len);
   return result;
}

static const JSCFunctionListEntry native_functions[] = {
   JS_CFUNC_DEF("log", 2, native_log),
   JS_CFUNC_DEF("now", 0, native_now),
   JS_CFUNC_DEF("textEncode", 1, native_text_encode),
   JS_CFUNC_DEF("textEncodeInto", 2, native_text_encode_into),
   JS_CFUNC_DEF("textDecode", 1, native_text_decode),
   JS_CFUNC_DEF("textDecodeWasm", 2, native_text_decode_wasm),
   JS_CFUNC_DEF("latin1Decode", 1, native_latin1_decode),
   JS_CFUNC_DEF("xUserDefined", 1, native_x_user_defined),
   JS_CFUNC_DEF("randomFill", 1, native_random_fill),
   JS_CFUNC_DEF("drainJobs", 0, native_drain_jobs),
   JS_CFUNC_DEF("call", 1, native_call),
   JS_CFUNC_DEF("instantiate", 1, native_instantiate),
};

/* ---- modules ------------------------------------------------------------------------------ */

/* The glue imports the WASI functions as an ES module (an import map points the page at its
 * stubs). The wasm calls WASI in C (wasi.c); the glue only needs the module to resolve. */
static const char wasi_module_source[] =
   "const EBADF = 8, ENOSYS = 52;\n"
   "export function fd_prestat_get() { return EBADF; }\n"
   "export function fd_prestat_dir_name() { return EBADF; }\n"
   "export function environ_sizes_get() { return 0; }\n"
   "export function environ_get() { return 0; }\n"
   "export function args_sizes_get() { return 0; }\n"
   "export function args_get() { return 0; }\n"
   "export function clock_time_get() { return ENOSYS; }\n"
   "export function fd_close() { return EBADF; }\n"
   "export function fd_fdstat_get() { return EBADF; }\n"
   "export function fd_fdstat_set_flags() { return EBADF; }\n"
   "export function fd_read() { return EBADF; }\n"
   "export function fd_renumber() { return EBADF; }\n"
   "export function fd_seek() { return EBADF; }\n"
   "export function fd_write() { return EBADF; }\n"
   "export function path_open() { return ENOSYS; }\n"
   "export function path_remove_directory() { return ENOSYS; }\n"
   "export function path_rename() { return ENOSYS; }\n"
   "export function path_unlink_file() { return ENOSYS; }\n"
   "export function proc_exit(code) { throw new Error('proc_exit ' + code); }\n"
   "export function bind() {}\n";

static char *read_file(const char *path, size_t *len)
{
   FILE *file = fopen(path, "rb");
   if (!file)
      return NULL;
   fseek(file, 0, SEEK_END);
   long size = ftell(file);
   fseek(file, 0, SEEK_SET);
   char *data = malloc((size_t)size + 1);
   if (data && fread(data, 1, (size_t)size, file) != (size_t)size) {
      free(data);
      data = NULL;
   }
   fclose(file);
   if (data) {
      data[size] = 0;
      *len = (size_t)size;
   }
   return data;
}

/* host/<path> is js_root/<path>; web/<path> is web_root/<path>. Query strings are dropped. */
static char *module_path(const char *name)
{
   const char *root = NULL, *rest = NULL;
   if (!strncmp(name, "host/", 5)) {
      root = g_js_root;
      rest = name + 5;
   } else if (!strncmp(name, "web/", 4)) {
      root = g_web_root;
      rest = name + 4;
   }
   if (!root)
      return NULL;
   size_t rest_len = strcspn(rest, "?#");
   size_t root_len = strlen(root);
   char *path = malloc(root_len + 1 + rest_len + 1);
   memcpy(path, root, root_len);
   path[root_len] = '/';
   memcpy(path + root_len + 1, rest, rest_len);
   path[root_len + 1 + rest_len] = 0;
   return path;
}

static JSModuleDef *load_module(JSContext *ctx, const char *name, void *opaque)
{
   (void)opaque;
   char *owned = NULL;
   const char *source;
   size_t len;
   if (!strcmp(name, "wasi_snapshot_preview1")) {
      source = wasi_module_source;
      len = sizeof wasi_module_source - 1;
   } else {
      char *path = module_path(name);
      owned = path ? read_file(path, &len) : NULL;
      if (!owned) {
         JS_ThrowReferenceError(ctx, "could not load module '%s'%s%s", name, path ? " from " : "", path ? path : "");
         free(path);
         return NULL;
      }
      free(path);
      source = owned;
   }
   JSValue fn = JS_Eval(ctx, source, len, name, JS_EVAL_TYPE_MODULE | JS_EVAL_FLAG_COMPILE_ONLY);
   free(owned);
   if (JS_IsException(fn))
      return NULL;
   JSModuleDef *module = JS_VALUE_GET_PTR(fn);
   JSValue meta = JS_GetImportMeta(ctx, module);
   char url[1024];
   snprintf(url, sizeof url, "https://aethus.invalid/%s", name);
   JS_SetPropertyStr(ctx, meta, "url", JS_NewString(ctx, url));
   JS_FreeValue(ctx, meta);
   JS_FreeValue(ctx, fn);
   return module;
}

static void rejection_tracker(JSContext *ctx, JSValueConst promise, JSValueConst reason, bool is_handled, void *opaque)
{
   (void)promise;
   (void)opaque;
   if (!is_handled)
      log_exception(ctx, reason, "unhandled promise rejection");
}

/* ---- the embedder's entry points ---------------------------------------------------------- */

static JSValue host_function(const char *name)
{
   JSValue global = JS_GetGlobalObject(g_ctx);
   JSValue host = JS_GetPropertyStr(g_ctx, global, "__host");
   JSValue fn = JS_GetPropertyStr(g_ctx, host, name);
   JS_FreeValue(g_ctx, host);
   JS_FreeValue(g_ctx, global);
   if (!JS_IsFunction(g_ctx, fn))
      say(3, "env.js defines no __host.%s", name);
   return fn;
}

/* strdup, which MSVC's C library names _strdup. */
static char *copy_string(const char *s)
{
   size_t n = strlen(s) + 1;
   char *copy = malloc(n);
   if (copy)
      memcpy(copy, s, n);
   return copy;
}

int aethus_js_init(const char *js_root, const char *web_root)
{
   const char *profile = getenv("AETHUS_PROFILE");
   if (profile && strcmp(profile, "1") == 0)
      g_profile = calloc(bridge_import_count + 2, sizeof *g_profile);
   g_js_root = copy_string(js_root);
   g_web_root = copy_string(web_root);
   slots_init();
   wasm_rt_init();

   g_rt = JS_NewRuntime();
   /* JS runs nested in deep wasm frames; the thread's own stack bounds it, not QuickJS's guess. */
   JS_SetMaxStackSize(g_rt, 0);
   JS_SetModuleLoaderFunc(g_rt, NULL, load_module, NULL);
   JS_SetHostPromiseRejectionTracker(g_rt, rejection_tracker, NULL);
   JS_NewClassID(g_rt, &g_payload_class);
   JSClassDef payload_class = {.class_name = "WasmExceptionPayload", .finalizer = payload_finalizer};
   JS_NewClass(g_rt, g_payload_class, &payload_class);
   g_ctx = JS_NewContext(g_rt);

   JSValue global = JS_GetGlobalObject(g_ctx);
   JSValue native = JS_NewObject(g_ctx);
   JS_SetPropertyFunctionList(g_ctx, native, native_functions, sizeof native_functions / sizeof native_functions[0]);
   JS_SetPropertyStr(g_ctx, global, "__native", native);
   JS_FreeValue(g_ctx, global);

   /* The page the glue expects, as global scripts: env.js first (gpu.js and audio.js build on
    * what it leaves on __aethus), and all of them before the glue, which captures the Web Audio
    * constructors when it loads. */
   static const char *const scripts[] = {"env.js", "gpu.js", "audio.js"};
   for (size_t i = 0; i < sizeof scripts / sizeof scripts[0]; i++) {
      char path[4096], name[64];
      snprintf(path, sizeof path, "%s/%s", js_root, scripts[i]);
      snprintf(name, sizeof name, "host/%s", scripts[i]);
      size_t len;
      char *source = read_file(path, &len);
      if (!source) {
         say(3, "could not read %s", path);
         return -1;
      }
      JSValue r = JS_Eval(g_ctx, source, len, name, JS_EVAL_TYPE_GLOBAL);
      free(source);
      if (JS_IsException(r)) {
         log_pending_exception(scripts[i]);
         return -1;
      }
      JS_FreeValue(g_ctx, r);
      drain_jobs();
   }
   g_host_tick = host_function("tick");
   g_host_frame = host_function("frame");

   static const char start[] = "import './boot.mjs';\n";
   JSValue promise = JS_Eval(g_ctx, start, sizeof start - 1, "host/<start>", JS_EVAL_TYPE_MODULE);
   if (JS_IsException(promise)) {
      log_pending_exception("boot.mjs");
      return -1;
   }
   drain_jobs();
   if (JS_PromiseState(g_ctx, promise) == JS_PROMISE_REJECTED) {
      JSValue reason = JS_PromiseResult(g_ctx, promise);
      log_exception(g_ctx, reason, "boot.mjs");
      JS_FreeValue(g_ctx, reason);
      JS_FreeValue(g_ctx, promise);
      return -1;
   }
   JS_FreeValue(g_ctx, promise);
   aethus_js_collect();
   return 0;
}

double aethus_js_tick(double now_ms)
{
   profile_switch(-2);
   JSValue arg = JS_NewFloat64(g_ctx, now_ms);
   JSValue r = JS_Call(g_ctx, g_host_tick, JS_UNDEFINED, 1, &arg);
   double next = -1;
   if (JS_IsException(r))
      log_pending_exception("__host.tick");
   else
      JS_ToFloat64(g_ctx, &next, r);
   JS_FreeValue(g_ctx, r);
   drain_jobs();
   profile_switch(-3);
   return next;
}

void aethus_js_frame(double now_ms)
{
   profile_switch(-2);
   JSValue arg = JS_NewFloat64(g_ctx, now_ms);
   JSValue r = JS_Call(g_ctx, g_host_frame, JS_UNDEFINED, 1, &arg);
   if (JS_IsException(r))
      log_pending_exception("__host.frame");
   JS_FreeValue(g_ctx, r);
   drain_jobs();
   profile_switch(-3);
}
