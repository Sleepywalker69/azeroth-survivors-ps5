/* Stable wasm32 linear memory: reserve its address range once, then map direct memory in
 * chunks as the logical memory grows. Thousands of small memory.grow calls must not copy
 * the entire game heap. Explicit wasm2c bounds checks still enforce the logical size. */
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifndef AETHUS_MEMORY_TEST
#include <ps5platform/heap.h>
#include <ps5platform/kernel.h>
#endif
#include "host.h"
#include "wasm-rt.h"

uint64_t __real_wasm_rt_grow_memory(wasm_rt_memory_t *memory, uint64_t delta);
void __real_wasm_rt_allocate_memory(wasm_rt_memory_t *memory, uint64_t initial_pages,
                                  uint64_t max_pages, bool is64, uint32_t page_size);
void __real_wasm_rt_free_memory(wasm_rt_memory_t *memory);

#define COMMIT_CHUNK ((uint64_t)64 << 20)
#define DIRECT_UNIT ((size_t)2 << 20)
#define MAX_BLOCKS 64
static struct {
   wasm_rt_memory_t *memory;
   uint8_t *base;
   uint64_t reserved, committed;
   unsigned blocks;
   struct { int64_t physical; size_t size; } block[MAX_BLOCKS];
} arena;
static uint64_t grows, moves, copied;
static double grow_ms, grow_max;

static void message(const char *text) { aethus_log(1, text, strlen(text)); }

/* Failed growth may retain a successfully mapped prefix for the next request, but never
 * changes the module's logical size. A failing individual map releases its physical block. */
static bool commit_until(uint64_t size)
{
   while (arena.committed < size) {
      uint64_t remaining = arena.reserved - arena.committed;
      size_t bytes = (size_t)(remaining < COMMIT_CHUNK ? remaining : COMMIT_CHUNK);
      if (!bytes || arena.blocks == MAX_BLOCKS) return false;
      int64_t physical;
      if (sceKernelAllocateDirectMemory(0, sceKernelGetDirectMemorySize(), bytes, DIRECT_UNIT,
                                       PS5_KERNEL_DIRECT_TYPE_CPU, &physical) != 0)
         return false;
      void *at = arena.base + arena.committed;
      void *mapped = at;
      int result = sceKernelMapDirectMemory(&mapped, bytes,
          PS5_KERNEL_PROT_CPU_READ | PS5_KERNEL_PROT_CPU_WRITE, PS5_KERNEL_MAP_FIXED,
          physical, DIRECT_UNIT);
      if (result != 0 || mapped != at) {
         if (result == 0) sceKernelMunmap(mapped, bytes);
         sceKernelReleaseDirectMemory(physical, bytes);
         return false;
      }
      arena.block[arena.blocks].physical = physical;
      arena.block[arena.blocks++].size = bytes;
      arena.committed += bytes;
   }
   return true;
}

static void release_arena(void)
{
   if (arena.base) sceKernelMunmap(arena.base, (size_t)arena.reserved);
   for (unsigned i = 0; i < arena.blocks; i++)
      sceKernelReleaseDirectMemory(arena.block[i].physical, arena.block[i].size);
   memset(&arena, 0, sizeof arena);
}

void __wrap_wasm_rt_allocate_memory(wasm_rt_memory_t *memory, uint64_t initial_pages,
                                  uint64_t max_pages, bool is64, uint32_t page_size)
{
   const char *legacy = getenv("AETHUS_WASM_REALLOC");
   /* Only the one unshared wasm32 memory this host runs. Other memory shapes keep WABT's
    * original behavior. Reserve/map failure also leaves a usable legacy allocation. */
   if (!arena.memory && !is64 && page_size == 65536 && initial_pages <= max_pages &&
       max_pages > 0 && max_pages <= 65536 && !(legacy && strcmp(legacy, "1") == 0)) {
      uint64_t maximum = max_pages * (uint64_t)page_size;
      uint64_t initial = initial_pages * (uint64_t)page_size;
      uint64_t reserved = (maximum + DIRECT_UNIT - 1) & ~(uint64_t)(DIRECT_UNIT - 1);
      void *base = NULL;
      if (sceKernelReserveVirtualRange(&base, (size_t)reserved, 0, DIRECT_UNIT) == 0) {
         arena.base = base;
         arena.reserved = reserved;
         if (commit_until(initial)) {
            arena.memory = memory;
            memory->data = base;
            memory->data_end = memory->data + initial;
            memory->size = initial;
            memory->pages = initial_pages;
            memory->max_pages = max_pages;
            memory->page_size = page_size;
            memory->is64 = false;
            if (initial) memset(base, 0, (size_t)initial);
            message("memory: stable wasm32 address range; direct memory committed in 64MiB chunks");
            return;
         }
         release_arena();
      }
      message("memory: stable reservation unavailable; using legacy realloc growth");
   }
   __real_wasm_rt_allocate_memory(memory, initial_pages, max_pages, is64, page_size);
}

static uint64_t grow(wasm_rt_memory_t *memory, uint64_t delta)
{
   if (memory != arena.memory) return __real_wasm_rt_grow_memory(memory, delta);
   uint64_t old_pages = memory->pages;
   if (delta > memory->max_pages - old_pages) return UINT64_MAX;
   uint64_t pages = old_pages + delta;
   uint64_t size = pages * (uint64_t)memory->page_size;
   if (!commit_until(size)) return UINT64_MAX;
   if (size > memory->size)
      memset(memory->data + memory->size, 0, (size_t)(size - memory->size));
   memory->pages = pages;
   memory->size = size;
   memory->data_end = memory->data + size;
   return old_pages;
}

void __wrap_wasm_rt_free_memory(wasm_rt_memory_t *memory)
{
   if (memory != arena.memory) {
      __real_wasm_rt_free_memory(memory);
      return;
   }
   release_arena();
   memory->data = memory->data_end = NULL;
   memory->size = memory->pages = 0;
}

uint64_t __wrap_wasm_rt_grow_memory(wasm_rt_memory_t *memory, uint64_t delta)
{
   static int profile = -1;
   if (profile < 0) {
      const char *value = getenv("AETHUS_PROFILE");
      profile = value && strcmp(value, "1") == 0;
   }
   if (!profile)
      return grow(memory, delta);
   const uintptr_t old_data = (uintptr_t)memory->data;
   const uint64_t old_size = memory->size;
   const double start = aethus_now_ms();
   const uint64_t result = grow(memory, delta);
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
   n = snprintf(line, sizeof line, "memory: wasm direct committed %.1fMiB / reserved %.1fMiB",
       (double)arena.committed / 1048576.0, (double)arena.reserved / 1048576.0);
   if (n > 0) aethus_log(1, line, (size_t)n);
   grows = moves = copied = 0;
   grow_ms = grow_max = 0;
}
