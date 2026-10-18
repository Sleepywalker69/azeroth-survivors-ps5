/*
 * The bridge between the wasm2c translation of the web build (wenilla.h) and the QuickJS
 * context that runs its wasm-bindgen glue (wenilla.js).
 *
 * The glue's imports reach JS through bridge_gen.c's generated trampolines; the glue reaches
 * the wasm through the exports object instantiation builds (host.c). Values cross the way a
 * browser's JS API converts them: i32 as a signed Number, i64 as a BigInt, f32/f64 as Numbers,
 * externref as the JS value itself (here a handle into host.c's value table).
 *
 * Traps and exceptions: every call from the host into the wasm runs under BRIDGE_GUARDED, which
 * catches a trap or an uncaught wasm exception and turns it into a JS exception. A JS exception
 * thrown by an import is thrown into the wasm as an exception with the host's own tag, so the
 * wasm's handlers see a foreign exception exactly as they do in a browser.
 */
#pragma once

#include <setjmp.h>
#include <stdint.h>
#include <string.h>

#include "quickjs.h"
#include "wenilla.h"
#include "wasm-rt-impl.h"

#ifdef __cplusplus
extern "C" {
#endif

extern JSContext *g_ctx;
extern w2c_wenilla g_inst;
/* The glue's import functions, in bridge_import_names order. */
extern JSValue *g_imports;
/* How many host-to-wasm calls are on the stack; value-table collection waits for zero. */
extern int g_wasm_depth;

typedef struct {
   const char *name;
   JSCFunction *fn;
   int length;
} BridgeExport;

extern const int bridge_import_count;
extern const char *const bridge_import_names[];
extern const int bridge_export_count;
extern const BridgeExport bridge_exports[];
extern const char *const bridge_memory_export;
extern const char *const bridge_table_export;
wasm_rt_memory_t *bridge_memory(void);
wasm_rt_externref_table_t *bridge_externref_table(void);

/* externref <-> JS value. A null externref is JS null. */
JSValue bridge_ref_to_js(wasm_rt_externref_t ref);
wasm_rt_externref_t bridge_js_to_ref(JSValueConst value);

/* Calls glue import `idx`, consuming argv. On a JS exception it does not return: the exception
 * is thrown into the wasm. */
JSValue bridge_call_import(int idx, int argc, JSValue *argv);
/* Throws the context's pending JS exception into the wasm. */
WASM_RT_NO_RETURN void bridge_throw_into_wasm(void);

/* Detaches the JS view of linear memory if the wasm grew it (a browser detaches the old
 * ArrayBuffer on memory.grow; the glue checks for that before reusing its typed arrays). */
void bridge_mem_check(void);

static inline JSValue bridge_to_js_u32(u32 v) { return JS_NewInt32(g_ctx, (int32_t)v); }
static inline JSValue bridge_to_js_u64(u64 v) { return JS_NewBigInt64(g_ctx, (int64_t)v); }
static inline JSValue bridge_to_js_f32(f32 v) { return JS_NewFloat64(g_ctx, (double)v); }
static inline JSValue bridge_to_js_f64(f64 v) { return JS_NewFloat64(g_ctx, v); }
static inline JSValue bridge_to_js_wasm_rt_externref_t(wasm_rt_externref_t v) { return bridge_ref_to_js(v); }

static inline void bridge_drop_result(JSValue r) { JS_FreeValue(g_ctx, r); }

static inline u32 bridge_result_u32(JSValue r) {
   if (JS_VALUE_GET_TAG(r) == JS_TAG_INT)
      return (u32)JS_VALUE_GET_INT(r);
   int32_t v;
   int failed = JS_ToInt32(g_ctx, &v, r);
   JS_FreeValue(g_ctx, r);
   if (failed)
      bridge_throw_into_wasm();
   return (u32)v;
}

static inline f64 bridge_result_f64(JSValue r) {
   double v;
   int failed = JS_ToFloat64(g_ctx, &v, r);
   JS_FreeValue(g_ctx, r);
   if (failed)
      bridge_throw_into_wasm();
   return v;
}

static inline f32 bridge_result_f32(JSValue r) { return (f32)bridge_result_f64(r); }

static inline u64 bridge_result_u64(JSValue r) {
   int64_t v;
   int failed = JS_ToBigInt64(g_ctx, &v, r);
   JS_FreeValue(g_ctx, r);
   if (failed)
      bridge_throw_into_wasm();
   return (u64)v;
}

static inline wasm_rt_externref_t bridge_result_wasm_rt_externref_t(JSValue r) {
   wasm_rt_externref_t ref = bridge_js_to_ref(r);
   JS_FreeValue(g_ctx, r);
   return ref;
}

/* Export arguments: ToWebAssemblyValue; -1 with a pending JS exception on failure. */
static inline int bridge_arg_u32(JSContext *ctx, u32 *out, JSValueConst v) {
   int32_t x;
   if (JS_ToInt32(ctx, &x, v))
      return -1;
   *out = (u32)x;
   return 0;
}

static inline int bridge_arg_u64(JSContext *ctx, u64 *out, JSValueConst v) {
   int64_t x;
   if (JS_ToBigInt64(ctx, &x, v))
      return -1;
   *out = (u64)x;
   return 0;
}

static inline int bridge_arg_f64(JSContext *ctx, f64 *out, JSValueConst v) {
   return JS_ToFloat64(ctx, out, v);
}

static inline int bridge_arg_f32(JSContext *ctx, f32 *out, JSValueConst v) {
   double d;
   if (JS_ToFloat64(ctx, &d, v))
      return -1;
   *out = (f32)d;
   return 0;
}

static inline int bridge_arg_wasm_rt_externref_t(JSContext *ctx, wasm_rt_externref_t *out, JSValueConst v) {
   (void)ctx;
   *out = bridge_js_to_ref(v);
   return 0;
}

JSValue bridge_multi_ii(JSContext *ctx, struct wasm_multi_ii r);

/* The state a guarded call saves and restores around itself. */
typedef struct {
   wasm_rt_jmp_buf jmp;
   WASM_RT_UNWIND_TARGET *unwind_target;
   int profile_lane;
} BridgeSaved;

void bridge_enter(BridgeSaved *saved);
void bridge_leave(const BridgeSaved *saved);
/* Turns a trap (or an uncaught wasm exception, code WASM_RT_TRAP_UNCAUGHT_EXCEPTION) into a
 * pending JS exception; returns JS_EXCEPTION. */
JSValue bridge_wasm_failure(JSContext *ctx, int code);
/* After a guarded call returns normally. */
void bridge_after_wasm(void);

/* Runs `stmt` (a call into the wasm) with traps and exceptions caught. setjmp must be called in
 * the frame that stays live while the wasm runs, so this is a macro, not a function. The
 * exception flavour of setjmp skips saving the signal mask: nothing here uses signals. */
#define BRIDGE_GUARDED(ctx, stmt)                                       \
   do {                                                                \
      BridgeSaved bridge_saved_;                                       \
      bridge_enter(&bridge_saved_);                                    \
      int bridge_code_ = WASM_RT_SETJMP_EXN(g_wasm_rt_jmp_buf);        \
      if (bridge_code_ == 0) {                                         \
         stmt;                                                         \
      }                                                                \
      bridge_leave(&bridge_saved_);                                    \
      if (bridge_code_ != 0)                                           \
         return bridge_wasm_failure((ctx), bridge_code_);              \
      bridge_after_wasm();                                             \
   } while (0)

#ifdef __cplusplus
}
#endif
