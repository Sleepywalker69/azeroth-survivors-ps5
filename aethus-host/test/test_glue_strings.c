/* Run the string-helper regression/benchmark with the host's actual QuickJS and native
 * text functions. Dead-section elimination discards the unrelated wasm/GPU host functions. */
#include "../c/host.c"

static wasm_rt_memory_t test_memory;
static JSValue test_memory_owner = JS_UNDEFINED;
wasm_rt_memory_t *bridge_memory(void) { return &test_memory; }

static JSValue test_set_memory(JSContext *ctx, JSValueConst self, int argc, JSValueConst *argv)
{
   (void)self;
   if (argc != 1) return JS_ThrowTypeError(ctx, "expected memory view");
   size_t size;
   uint8_t *data = bytes_of(ctx, argv[0], &size);
   if (!data) return JS_EXCEPTION;
   JS_FreeValue(ctx, test_memory_owner);
   test_memory_owner = JS_DupValue(ctx, argv[0]);
   test_memory.data = data;
   test_memory.size = size;
   g_instantiated = true;
   return JS_UNDEFINED;
}

static JSValue test_print(JSContext *ctx, JSValueConst self, int argc, JSValueConst *argv)
{
   (void)self;
   for (int i = 0; i < argc; i++) {
      const char *s = JS_ToCString(ctx, argv[i]);
      if (!s) return JS_EXCEPTION;
      printf("%s%s", i ? " " : "", s);
      JS_FreeCString(ctx, s);
   }
   puts("");
   return JS_UNDEFINED;
}

int main(int argc, char **argv)
{
   if (argc != 2) return 2;
   size_t length;
   char *source = read_file(argv[1], &length);
   if (!source) return 2;
   JSRuntime *rt = JS_NewRuntime();
   JSContext *ctx = JS_NewContext(rt);
   JSValue global = JS_GetGlobalObject(ctx);
   JS_SetPropertyStr(ctx, global, "encode", JS_NewCFunction(ctx, native_text_encode, "encode", 1));
   JS_SetPropertyStr(ctx, global, "encodeInto", JS_NewCFunction(ctx, native_text_encode_into, "encodeInto", 2));
   JS_SetPropertyStr(ctx, global, "decode", JS_NewCFunction(ctx, native_text_decode, "decode", 1));
   JS_SetPropertyStr(ctx, global, "setMemory", JS_NewCFunction(ctx, test_set_memory, "setMemory", 1));
   JSValue native = JS_NewObject(ctx);
   JS_SetPropertyStr(ctx, native, "textDecodeWasm", JS_NewCFunction(ctx, native_text_decode_wasm, "textDecodeWasm", 2));
   JS_SetPropertyStr(ctx, global, "__native", native);
   JS_SetPropertyStr(ctx, global, "print", JS_NewCFunction(ctx, test_print, "print", 1));
   JS_FreeValue(ctx, global);
   JSValue result = JS_Eval(ctx, source, length, argv[1], JS_EVAL_TYPE_GLOBAL);
   free(source);
   int failed = JS_IsException(result);
   if (failed) {
      JSValue error = JS_GetException(ctx);
      const char *message = JS_ToCString(ctx, error);
      fprintf(stderr, "%s\n", message ? message : "JS exception");
      JS_FreeCString(ctx, message);
      JS_FreeValue(ctx, error);
   }
   JS_FreeValue(ctx, result);
   JS_FreeValue(ctx, test_memory_owner);
   JS_FreeContext(ctx);
   JS_FreeRuntime(rt);
   return failed;
}
