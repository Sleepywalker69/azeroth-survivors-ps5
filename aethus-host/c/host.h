/*
 * The JS side of the Aethus host: QuickJS running the web build's wasm-bindgen glue against the
 * wasm2c translation, with a browser environment (js/env.js) whose platform work is done by the
 * embedder (the Rust host) through aethus_rust_call.
 *
 * Threading: everything here runs on one thread, the one that called aethus_js_init.
 */
#pragma once

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* A value crossing between JS and the embedder. */
enum {
   AETHUS_UNDEFINED = 0,
   AETHUS_NULL = 1,
   AETHUS_BOOL = 2,   /* number is 0 or 1 */
   AETHUS_NUMBER = 3,
   AETHUS_STRING = 4, /* ptr/len: UTF-8 */
   AETHUS_BYTES = 5,  /* ptr/len: a typed array's or ArrayBuffer's bytes (writable in arguments) */
   AETHUS_OBJECT = 6, /* an argument that is some other object: no payload */
   AETHUS_ERROR = 7,  /* a result only: ptr/len is the message, thrown as a JS Error */
};

typedef struct {
   uint32_t kind;
   uint32_t reserved;
   double number;
   uint8_t *ptr;
   size_t len;
} AethusValue;

/* Implemented by the embedder. `op` indexes its operation table. A string or bytes result is
 * the embedder's allocation; it is copied into JS and handed back to aethus_rust_free. */
extern void aethus_rust_call(uint32_t op, const AethusValue *args, uint32_t argc, AethusValue *ret);
extern void aethus_rust_free(uint8_t *ptr, size_t len);
/* One log line (UTF-8, no newline). level: 0 debug, 1 log/info, 2 warn, 3 error. */
extern void aethus_log(int level, const char *text, size_t len);

/* Creates the JS context, runs `<js_root>/env.js`, `gpu.js` and `audio.js`, then starts the boot
 * module `<js_root>/boot.mjs`, which loads the glue from `web_root`. 0 on success. */
int aethus_js_init(const char *js_root, const char *web_root);
/* Runs the tasks that are due: platform events, timers, posted messages. Returns the number of
 * milliseconds until the next timer, or -1 when none is pending. */
double aethus_js_tick(double now_ms);
/* Runs the animation frame callbacks (one display refresh). */
void aethus_js_frame(double now_ms);
/* Releases JS values the wasm no longer references; call between tasks. */
void aethus_js_collect(void);
/* Opt-in exclusive wasm / JS import timing totals, reset after each report. */
void aethus_js_profile_report(void);
/* Milliseconds on the host's monotonic clock (performance.now). */
double aethus_now_ms(void);

#ifdef __cplusplus
}
#endif
