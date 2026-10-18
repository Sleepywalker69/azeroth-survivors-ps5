/* Exercise real allocator logic with controlled mapping failures, without a console. */
#define AETHUS_MEMORY_TEST
#include "memory-platform.h"
#include "../ps5/wasm-memory.c"
#include <assert.h>
#include <time.h>

static void *reservation;
static size_t reservation_size;
static unsigned physical_live, map_calls, legacy_allocations;
static bool fail_reserve, fail_map, fail_allocate;

int64_t sceKernelGetDirectMemorySize(void) { return (int64_t)12 << 30; }
int32_t sceKernelReserveVirtualRange(void **at, size_t size, int flags, size_t alignment)
{
    assert(!reservation && flags == 0 && alignment == DIRECT_UNIT);
    if (fail_reserve) return -1;
    reservation = malloc(size);
    assert(reservation);
    reservation_size = size;
    *at = reservation;
    return 0;
}
int32_t sceKernelAllocateDirectMemory(int64_t first, int64_t last, size_t size, size_t alignment, int type, int64_t *physical)
{
    assert(first == 0 && last > 0 && size % DIRECT_UNIT == 0 && alignment == DIRECT_UNIT && type == 12);
    if (fail_allocate) return -1;
    *physical = ++physical_live;
    return 0;
}
int32_t sceKernelMapDirectMemory(void **at, size_t size, int prot, int flags, int64_t physical, size_t alignment)
{
    assert((uint8_t *)*at >= (uint8_t *)reservation);
    assert((uint8_t *)*at + size <= (uint8_t *)reservation + reservation_size);
    assert(prot == 3 && flags == PS5_KERNEL_MAP_FIXED && physical > 0 && alignment == DIRECT_UNIT);
    map_calls++;
    return fail_map ? -1 : 0;
}
int32_t sceKernelReleaseDirectMemory(int64_t physical, size_t size)
{
    assert(physical > 0 && size && physical_live);
    physical_live--;
    return 0;
}
int32_t sceKernelMunmap(void *at, size_t size)
{
    assert(at == reservation && size == reservation_size);
    free(reservation);
    reservation = NULL;
    return 0;
}
void ps5_heap_stats(struct ps5_heap_stats *stats) { memset(stats, 0, sizeof *stats); }
void aethus_log(int level, const char *text, size_t len) { (void)level; (void)text; (void)len; }
double aethus_now_ms(void) { return (double)clock() * 1000.0 / CLOCKS_PER_SEC; }

void __real_wasm_rt_allocate_memory(wasm_rt_memory_t *m, uint64_t initial, uint64_t maximum, bool is64, uint32_t page_size)
{
    legacy_allocations++;
    m->pages = initial; m->max_pages = maximum; m->page_size = page_size; m->is64 = is64;
    m->size = initial * page_size; m->data = calloc(1, m->size); m->data_end = m->data + m->size;
}
uint64_t __real_wasm_rt_grow_memory(wasm_rt_memory_t *m, uint64_t delta)
{
    if (delta > m->max_pages - m->pages) return UINT64_MAX;
    uint64_t old = m->pages;
    uint64_t size = (old + delta) * m->page_size;
    uint8_t *data = realloc(m->data, size);
    if (!data && size) return UINT64_MAX;
    memset(data + m->size, 0, size - m->size);
    m->data = data; m->pages += delta; m->size = size; m->data_end = data + size;
    return old;
}
void __real_wasm_rt_free_memory(wasm_rt_memory_t *m) { free(m->data); }

int main(void)
{
    wasm_rt_memory_t m = {0};
    __wrap_wasm_rt_allocate_memory(&m, 1, 2048, false, 65536);
    uint8_t *base = m.data;
    assert(!legacy_allocations && m.size == 65536 && physical_live == 1);
    memset(base, 0x5a, 65536);
    for (uint64_t i = 1; i < 1024; i++) {
        assert(__wrap_wasm_rt_grow_memory(&m, 1) == i);
        assert(m.data == base && m.pages == i + 1 && m.size == (i + 1) * 65536);
        for (size_t j = 0; j < 65536; j++) assert(base[i * 65536 + j] == 0);
        base[i * 65536] = 0x7b;
    }
    assert(map_calls == 1 && physical_live == 1);
    fail_map = true;
    assert(__wrap_wasm_rt_grow_memory(&m, 1) == UINT64_MAX);
    assert(m.pages == 1024 && m.size == COMMIT_CHUNK && physical_live == 1);
    fail_map = false;
    fail_allocate = true;
    assert(__wrap_wasm_rt_grow_memory(&m, 1) == UINT64_MAX);
    assert(m.pages == 1024 && physical_live == 1);
    fail_allocate = false;
    assert(__wrap_wasm_rt_grow_memory(&m, 1) == 1024 && m.data == base);
    assert(physical_live == 2 && base[COMMIT_CHUNK] == 0);
    for (size_t j = 0; j < 65536; j++) assert(base[j] == 0x5a);
    for (uint64_t i = 1; i < 1024; i++) assert(base[i * 65536] == 0x7b);
    assert(__wrap_wasm_rt_grow_memory(&m, 0) == 1025);
    assert(__wrap_wasm_rt_grow_memory(&m, UINT64_MAX) == UINT64_MAX);
    assert(__wrap_wasm_rt_grow_memory(&m, 1024) == UINT64_MAX);
    assert(m.pages == 1025 && m.data == base);
    __wrap_wasm_rt_free_memory(&m);
    assert(!physical_live && !reservation && !arena.memory);

    /* Initial mapping failures unwind the reservation and fall back cleanly. */
    fail_map = true;
    __wrap_wasm_rt_allocate_memory(&m, 1, 32, false, 65536);
    assert(legacy_allocations == 1 && !physical_live && !reservation);
    assert(__wrap_wasm_rt_grow_memory(&m, 1) == 1);
    __wrap_wasm_rt_free_memory(&m);
    fail_map = false; fail_reserve = true;
    __wrap_wasm_rt_allocate_memory(&m, 1, 32, false, 65536);
    assert(legacy_allocations == 2 && !physical_live && !reservation);
    __wrap_wasm_rt_free_memory(&m);
    puts("PASS: stable growth, zero fill, data preservation, limits, map/allocate failures, fallback and cleanup");
}
