/* Measurements for wasm2c's realloc-based linear memory on the PS5. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ps5platform/heap.h>
#include "host.h"
#include "wasm-rt.h"

uint64_t __real_wasm_rt_grow_memory(wasm_rt_memory_t *memory, uint64_t delta);
static uint64_t grows, moves, copied;
static double grow_ms, grow_max;

uint64_t __wrap_wasm_rt_grow_memory(wasm_rt_memory_t *memory, uint64_t delta)
{
   static int profile = -1;
   if (profile < 0) {
      const char *value = getenv("AETHUS_PROFILE");
      profile = value && strcmp(value, "1") == 0;
   }
   if (!profile)
      return __real_wasm_rt_grow_memory(memory, delta);
   const uintptr_t old_data = (uintptr_t)memory->data;
   const uint64_t old_size = memory->size;
   const double start = aethus_now_ms();
   const uint64_t result = __real_wasm_rt_grow_memory(memory, delta);
   const double elapsed = aethus_now_ms() - start;
   grows++;
   grow_ms += elapsed;
   if (elapsed > grow_max) grow_max = elapsed;
   if ((uintptr_t)memory->data != old_data) {
      moves++;
      copied += old_size;
   }
   return result;
}

void aethus_ps5_memory_report(void)
{
   struct ps5_heap_stats stats;
   ps5_heap_stats(&stats);
   char line[400];
   int n = snprintf(line, sizeof line,
       "memory: heap %.1fMiB (peak %.1fMiB), libc fallbacks %llu; "
       "wasm growth %llu calls, %.1fms total, %.1fms max, %llu moves/%.1fMiB copied",
       (double)stats.mapped_bytes / 1048576.0, (double)stats.peak_bytes / 1048576.0,
       stats.libc_fallbacks, (unsigned long long)grows, grow_ms, grow_max,
       (unsigned long long)moves, (double)copied / 1048576.0);
   if (n > 0) aethus_log(1, line, (size_t)n < sizeof line ? (size_t)n : sizeof line - 1);
   grows = moves = copied = 0;
   grow_ms = grow_max = 0;
}
