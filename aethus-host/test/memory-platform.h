/* Host-test stand-ins for the small PS5 API surface used by wasm-memory.c. */
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#define PS5_KERNEL_DIRECT_TYPE_CPU 12
#define PS5_KERNEL_PROT_CPU_READ 1
#define PS5_KERNEL_PROT_CPU_WRITE 2
#define PS5_KERNEL_MAP_FIXED 0x10
struct ps5_heap_stats {
    uintptr_t range_base;
    size_t range_bytes, mapped_bytes, peak_bytes;
    unsigned segments;
    unsigned long long libc_fallbacks;
    unsigned arenas;
};
void ps5_heap_stats(struct ps5_heap_stats *stats);
int64_t sceKernelGetDirectMemorySize(void);
int32_t sceKernelReserveVirtualRange(void **at, size_t size, int flags, size_t alignment);
int32_t sceKernelAllocateDirectMemory(int64_t first, int64_t last, size_t size, size_t alignment, int type, int64_t *physical);
int32_t sceKernelMapDirectMemory(void **at, size_t size, int prot, int flags, int64_t physical, size_t alignment);
int32_t sceKernelReleaseDirectMemory(int64_t physical, size_t size);
int32_t sceKernelMunmap(void *at, size_t size);
