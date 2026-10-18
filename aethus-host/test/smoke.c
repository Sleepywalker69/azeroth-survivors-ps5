/*
 * A smoke test of the JS side of the host: boots the web build's glue and the wasm2c translation
 * under QuickJS with a stand-in embedder that has no window, no GPU, no sound device and no game
 * data, and runs the event loop for a while. It shows how far the client gets and which of the
 * embedder's operations it asks for, in order, before the real (Rust) embedder exists.
 *
 *   smoke <js_root> <web_root> [seconds]
 */
#include <stdarg.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef _WIN32
#include <windows.h>
static void sleep_ms(double ms)
{
   if (ms >= 1)
      Sleep((DWORD)ms);
}
#else
#include <time.h>
#define _strdup strdup
static void sleep_ms(double ms)
{
   if (ms <= 0)
      return;
   struct timespec ts = {(time_t)(ms / 1000), (long)((ms - (double)(time_t)(ms / 1000) * 1000) * 1e6)};
   nanosleep(&ts, NULL);
}
#endif

#include "host.h"

static bool g_quit;
static double g_audio_start = -1;

/* ---- operations ----------------------------------------------------------------------------- */

#define MAX_OPS 256
static char *g_op_names[MAX_OPS];
static unsigned g_op_calls[MAX_OPS];
static int g_op_count = 1; /* 0 is resolve */

static void set_string(AethusValue *ret, const char *text)
{
   size_t len = strlen(text);
   ret->kind = AETHUS_STRING;
   ret->ptr = malloc(len ? len : 1);
   memcpy(ret->ptr, text, len);
   ret->len = len;
}

static void set_number(AethusValue *ret, double value)
{
   ret->kind = AETHUS_NUMBER;
   ret->number = value;
}

static void arg_text(const AethusValue *args, uint32_t argc, uint32_t i, char *out, size_t size)
{
   out[0] = 0;
   if (i >= argc)
      return;
   const AethusValue *a = &args[i];
   if (a->kind == AETHUS_STRING) {
      size_t n = a->len < size - 1 ? a->len : size - 1;
      memcpy(out, a->ptr, n);
      out[n] = 0;
   } else if (a->kind == AETHUS_NUMBER || a->kind == AETHUS_BOOL) {
      snprintf(out, size, "%g", a->number);
   } else if (a->kind == AETHUS_BYTES) {
      snprintf(out, size, "<%zu bytes>", a->len);
   } else {
      snprintf(out, size, "<kind %u>", a->kind);
   }
}

static int g_next_token = 1;

void aethus_rust_call(uint32_t op, const AethusValue *args, uint32_t argc, AethusValue *ret)
{
   char text[512];
   if (op == 0) {
      arg_text(args, argc, 0, text, sizeof text);
      for (int i = 1; i < g_op_count; i++) {
         if (!strcmp(g_op_names[i], text)) {
            set_number(ret, i);
            return;
         }
      }
      if (g_op_count == MAX_OPS) {
         set_number(ret, 0);
         return;
      }
      g_op_names[g_op_count] = _strdup(text);
      fprintf(stderr, "[op] first use of %s\n", text);
      set_number(ret, g_op_count++);
      return;
   }
   if (op >= (uint32_t)g_op_count)
      return;
   g_op_calls[op]++;
   const char *name = g_op_names[op];

   if (!strcmp(name, "config")) {
      set_string(ret, "{\"viewport\":[1280,720],\"dpr\":1,\"platform\":\"smoke\",\"language\":\"en-US\","
                      "\"rogue\":{},\"env\":{},\"addonsDir\":\"\"}");
   } else if (!strcmp(name, "poll_events") || !strcmp(name, "gpu_request_adapter") || !strcmp(name, "audio_poll")) {
      set_string(ret, "");
   } else if (!strcmp(name, "gamepads") || !strcmp(name, "fs_list")) {
      set_string(ret, "[]");
   } else if (!strcmp(name, "storage_load")) {
      set_string(ret, "{}");
   } else if (!strcmp(name, "data_read") || !strcmp(name, "fs_read") || !strcmp(name, "save_pick")) {
      if (g_op_calls[op] <= 20) {
         arg_text(args, argc, 0, text, sizeof text);
         fprintf(stderr, "[op] %s %s -> none\n", name, text);
      }
      ret->kind = AETHUS_NULL;
   } else if (!strcmp(name, "data_exists")) {
      ret->kind = AETHUS_BOOL;
      ret->number = 0;
   } else if (!strcmp(name, "audio_open")) {
      g_audio_start = aethus_now_ms();
      set_string(ret, "{\"handle\":1,\"sampleRate\":48000,\"maxChannelCount\":2,\"baseLatency\":0.01,\"outputLatency\":0.02}");
   } else if (!strcmp(name, "audio_time")) {
      set_number(ret, g_audio_start < 0 ? 0 : (aethus_now_ms() - g_audio_start) / 1000);
   } else if (!strcmp(name, "audio_buffer_create") || !strcmp(name, "audio_play") || !strcmp(name, "audio_decode")) {
      set_number(ret, g_next_token++);
   } else if (!strcmp(name, "progress") || !strcmp(name, "fatal")) {
      arg_text(args, argc, 0, text, sizeof text);
      fprintf(stderr, "[op] %s: %s\n", name, text);
      if (!strcmp(name, "fatal"))
         g_quit = true;
   } else if (!strcmp(name, "quit")) {
      g_quit = true;
   } else if (g_op_calls[op] <= 3) {
      char a0[128], a1[128];
      arg_text(args, argc, 0, a0, sizeof a0);
      arg_text(args, argc, 1, a1, sizeof a1);
      fprintf(stderr, "[op] %s(%s, %s) -> undefined\n", name, a0, a1);
   }
}

void aethus_rust_free(uint8_t *ptr, size_t len)
{
   (void)len;
   free(ptr);
}

void aethus_log(int level, const char *text, size_t len)
{
   static const char *const names[] = {"debug", "log", "warn", "error"};
   fprintf(stderr, "[%s] %.*s\n", names[level & 3], (int)len, text);
}

/* ---- the loop ------------------------------------------------------------------------------- */

int main(int argc, char **argv)
{
   if (argc < 3) {
      fprintf(stderr, "usage: smoke <js_root> <web_root> [seconds]\n");
      return 2;
   }
   double seconds = argc > 3 ? atof(argv[3]) : 10;
   double t0 = aethus_now_ms();
   if (aethus_js_init(argv[1], argv[2])) {
      fprintf(stderr, "aethus_js_init failed\n");
      return 1;
   }
   fprintf(stderr, "[smoke] init took %.0f ms\n", aethus_now_ms() - t0);
   double end = aethus_now_ms() + seconds * 1000, next_frame = 0;
   unsigned frames = 0;
   while (!g_quit && aethus_now_ms() < end) {
      double now = aethus_now_ms();
      double wait = aethus_js_tick(now);
      if (now >= next_frame) {
         aethus_js_frame(now);
         frames++;
         next_frame = now + 1000.0 / 60;
      }
      aethus_js_collect();
      double until = next_frame - aethus_now_ms();
      if (wait >= 0 && wait < until)
         until = wait;
      sleep_ms(until > 4 ? 4 : until);
   }
   fprintf(stderr, "[smoke] %u frames in %.1f s%s\n", frames, (aethus_now_ms() - t0) / 1000, g_quit ? " (quit)" : "");
   for (int i = 1; i < g_op_count; i++)
      fprintf(stderr, "[smoke] %6u %s\n", g_op_calls[i], g_op_names[i]);
   return 0;
}
