/*
 * The web build's WASI imports, answered the way the page's wasi_stubs.js answers them: no
 * preopened directories (EBADF ends libc's scan), an empty environment, no files and no WASI
 * clock (ENOSYS). The engine reads the time and its settings through the glue instead.
 *
 * One addition: text written to stdout or stderr, which the browser drops (the write fails with
 * EBADF and Rust's std treats that as a sink), is logged a line at a time before the same EBADF
 * is returned, so panics and libc diagnostics are visible.
 */
#include <stdbool.h>
#include <stdio.h>

#include "bridge.h"
#include "host.h"

enum { WASI_ESUCCESS = 0, WASI_EBADF = 8, WASI_ENOSYS = 52 };

static bool in_memory(u64 ptr, u64 len)
{
   wasm_rt_memory_t *memory = bridge_memory();
   return ptr <= memory->size && len <= memory->size - ptr;
}

static void store_u32(u32 ptr, u32 value)
{
   if (in_memory(ptr, 4))
      memcpy(bridge_memory()->data + ptr, &value, 4); /* the wasm and both hosts are little-endian */
}

static u32 load_u32(u64 ptr)
{
   u32 value = 0;
   if (in_memory(ptr, 4))
      memcpy(&value, bridge_memory()->data + ptr, 4);
   return value;
}

typedef struct {
   char text[4096];
   size_t len;
} LineBuffer;

static LineBuffer g_lines[3];

static void log_text(u32 fd, const uint8_t *data, size_t len)
{
   LineBuffer *line = &g_lines[fd];
   for (size_t i = 0; i < len; i++) {
      char c = (char)data[i];
      if (c == '\n' || line->len == sizeof line->text) {
         aethus_log(fd == 2 ? 2 : 1, line->text, line->len);
         line->len = 0;
         if (c == '\n')
            continue;
      }
      line->text[line->len++] = c;
   }
}

u32 w2c_wasi__snapshot__preview1_fd_write(struct w2c_wasi__snapshot__preview1 *env, u32 fd, u32 iovs, u32 iovs_len,
                                          u32 nwritten)
{
   (void)env;
   (void)nwritten;
   if (fd == 1 || fd == 2) {
      for (u32 i = 0; i < iovs_len; i++) {
         u64 iov = (u64)iovs + 8u * (u64)i;
         u32 buf = load_u32(iov), buf_len = load_u32(iov + 4);
         if (!in_memory(iov, 8) || !in_memory(buf, buf_len))
            break;
         log_text(fd, bridge_memory()->data + buf, buf_len);
      }
   }
   return WASI_EBADF;
}

u32 w2c_wasi__snapshot__preview1_environ_sizes_get(struct w2c_wasi__snapshot__preview1 *env, u32 count, u32 size)
{
   (void)env;
   store_u32(count, 0);
   store_u32(size, 0);
   return WASI_ESUCCESS;
}

/* (Not `environ`: the CRT defines it as a macro.) */
u32 w2c_wasi__snapshot__preview1_environ_get(struct w2c_wasi__snapshot__preview1 *env, u32 environ_ptrs, u32 buf)
{
   (void)env;
   (void)environ_ptrs;
   (void)buf;
   return WASI_ESUCCESS;
}

u32 w2c_wasi__snapshot__preview1_clock_time_get(struct w2c_wasi__snapshot__preview1 *env, u32 id, u64 precision,
                                                u32 time)
{
   (void)env;
   (void)id;
   (void)precision;
   (void)time;
   return WASI_ENOSYS;
}

u32 w2c_wasi__snapshot__preview1_fd_close(struct w2c_wasi__snapshot__preview1 *env, u32 fd)
{
   (void)env;
   (void)fd;
   return WASI_EBADF;
}

u32 w2c_wasi__snapshot__preview1_fd_fdstat_get(struct w2c_wasi__snapshot__preview1 *env, u32 fd, u32 stat)
{
   (void)env;
   (void)fd;
   (void)stat;
   return WASI_EBADF;
}

u32 w2c_wasi__snapshot__preview1_fd_fdstat_set_flags(struct w2c_wasi__snapshot__preview1 *env, u32 fd, u32 flags)
{
   (void)env;
   (void)fd;
   (void)flags;
   return WASI_EBADF;
}

u32 w2c_wasi__snapshot__preview1_fd_prestat_get(struct w2c_wasi__snapshot__preview1 *env, u32 fd, u32 prestat)
{
   (void)env;
   (void)fd;
   (void)prestat;
   return WASI_EBADF;
}

u32 w2c_wasi__snapshot__preview1_fd_prestat_dir_name(struct w2c_wasi__snapshot__preview1 *env, u32 fd, u32 path,
                                                     u32 path_len)
{
   (void)env;
   (void)fd;
   (void)path;
   (void)path_len;
   return WASI_EBADF;
}

u32 w2c_wasi__snapshot__preview1_fd_read(struct w2c_wasi__snapshot__preview1 *env, u32 fd, u32 iovs, u32 iovs_len,
                                         u32 nread)
{
   (void)env;
   (void)fd;
   (void)iovs;
   (void)iovs_len;
   (void)nread;
   return WASI_EBADF;
}

u32 w2c_wasi__snapshot__preview1_fd_renumber(struct w2c_wasi__snapshot__preview1 *env, u32 fd, u32 to)
{
   (void)env;
   (void)fd;
   (void)to;
   return WASI_EBADF;
}

u32 w2c_wasi__snapshot__preview1_fd_seek(struct w2c_wasi__snapshot__preview1 *env, u32 fd, u64 offset, u32 whence,
                                         u32 new_offset)
{
   (void)env;
   (void)fd;
   (void)offset;
   (void)whence;
   (void)new_offset;
   return WASI_EBADF;
}

u32 w2c_wasi__snapshot__preview1_path_open(struct w2c_wasi__snapshot__preview1 *env, u32 fd, u32 dirflags, u32 path,
                                           u32 path_len, u32 oflags, u64 rights_base, u64 rights_inheriting,
                                           u32 fdflags, u32 opened)
{
   (void)env;
   (void)fd;
   (void)dirflags;
   (void)path;
   (void)path_len;
   (void)oflags;
   (void)rights_base;
   (void)rights_inheriting;
   (void)fdflags;
   (void)opened;
   return WASI_ENOSYS;
}

u32 w2c_wasi__snapshot__preview1_path_remove_directory(struct w2c_wasi__snapshot__preview1 *env, u32 fd, u32 path,
                                                       u32 path_len)
{
   (void)env;
   (void)fd;
   (void)path;
   (void)path_len;
   return WASI_ENOSYS;
}

u32 w2c_wasi__snapshot__preview1_path_rename(struct w2c_wasi__snapshot__preview1 *env, u32 fd, u32 old_path,
                                             u32 old_len, u32 new_fd, u32 new_path, u32 new_len)
{
   (void)env;
   (void)fd;
   (void)old_path;
   (void)old_len;
   (void)new_fd;
   (void)new_path;
   (void)new_len;
   return WASI_ENOSYS;
}

u32 w2c_wasi__snapshot__preview1_path_unlink_file(struct w2c_wasi__snapshot__preview1 *env, u32 fd, u32 path,
                                                  u32 path_len)
{
   (void)env;
   (void)fd;
   (void)path;
   (void)path_len;
   return WASI_ENOSYS;
}

/* wasi_stubs.js throws `new Error('proc_exit ' + code)`, which unwinds the wasm as a JS
 * exception; so does this. */
void w2c_wasi__snapshot__preview1_proc_exit(struct w2c_wasi__snapshot__preview1 *env, u32 code)
{
   (void)env;
   char message[32];
   snprintf(message, sizeof message, "proc_exit %d", (int32_t)code);
   JSValue error = JS_NewError(g_ctx);
   JS_SetPropertyStr(g_ctx, error, "message", JS_NewString(g_ctx, message));
   JS_Throw(g_ctx, error);
   bridge_throw_into_wasm();
}
