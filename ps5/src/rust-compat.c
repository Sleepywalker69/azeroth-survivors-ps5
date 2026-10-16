/*
 * Azeroth Survivors for the PlayStation 5 - what Rust's standard library asks of
 * the C library that the console answers under another name.
 *
 * Rust's std is built for FreeBSD 11 (`RUST_LIBC_UNSTABLE_FREEBSD_VERSION=11`,
 * the layout the console's kernel keeps), whose libc names the stat family and
 * readdir by their FBSD_1.0 symbol versions; the console's modules export them
 * unversioned. The same adaptation as ps5-ai-cli's platform/freebsd11.c
 * (Apache-2.0), cut to what a title needs.
 *
 * SPDX-License-Identifier: MIT
 */
#include <dirent.h>
#include <errno.h>
#include <limits.h>
#include <pthread.h>
#include <pthread_np.h>
#include <stdlib.h>
#include <sys/event.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

int ps5_freebsd11_stat(const char *path, struct stat *buffer) { return stat(path, buffer); }
int ps5_freebsd11_fstat(int fd, struct stat *buffer) { return fstat(fd, buffer); }
int ps5_freebsd11_lstat(const char *path, struct stat *buffer) { return lstat(path, buffer); }
int ps5_freebsd11_fstatat(int fd, const char *path, struct stat *buffer, int flags)
{
   return fstatat(fd, path, buffer, flags);
}
struct dirent *ps5_freebsd11_readdir(DIR *directory) { return readdir(directory); }
int ps5_freebsd11_kevent(int fd, const struct kevent *changes, int nchanges, struct kevent *events,
                         int nevents, const struct timespec *timeout)
{
   return kevent(fd, changes, nchanges, events, nevents, timeout);
}
__asm__(".symver ps5_freebsd11_stat,stat@FBSD_1.0");
__asm__(".symver ps5_freebsd11_fstat,fstat@FBSD_1.0");
__asm__(".symver ps5_freebsd11_lstat,lstat@FBSD_1.0");
__asm__(".symver ps5_freebsd11_fstatat,fstatat@FBSD_1.1");
__asm__(".symver ps5_freebsd11_readdir,readdir@FBSD_1.0");
__asm__(".symver ps5_freebsd11_kevent,kevent@FBSD_1.0");

/* Rust's FreeBSD backend uses the newer spelling; the console exports the old one. */
int pthread_setname_np(pthread_t thread, const char *name)
{
   pthread_set_name_np(thread, name);
   return 0;
}

/* The platform CSPRNG, never a time-seeded fallback. */
int getentropy(void *buffer, size_t size)
{
   if (size > 256) {
      errno = EIO;
      return -1;
   }
   if (size != 0)
      arc4random_buf(buffer, size);
   return 0;
}

/* FreeBSD's flags: GRND_NONBLOCK=1, GRND_RANDOM=2; the CSPRNG serves both. */
ssize_t getrandom(void *buffer, size_t size, unsigned flags)
{
   if ((flags & ~3u) != 0 || size > SSIZE_MAX) {
      errno = EINVAL;
      return -1;
   }
   if (size != 0)
      arc4random_buf(buffer, size);
   return (ssize_t)size;
}

/* ------------------------------------------------------------------------
 * Names std, rustix and nix link against that no module a title loads
 * exports. The game calls none of them (no sockets, child processes, FIFOs or
 * symbolic links on the console); each answers as FreeBSD does for an
 * operation it cannot do, or is built from calls the console has.
 */
#include <fcntl.h>
#include <signal.h>
#include <sys/mount.h>
#include <sys/socket.h>
#include <sys/wait.h>

int killpg(pid_t group, int signal_number)
{
   if (group < 0) {
      errno = EINVAL;
      return -1;
   }
   return kill(-group, signal_number);
}

/* pipe2 and dup3: the plain call, then the descriptor flags asked for. */
static int apply_fd_flags(int fd, int flags)
{
   if ((flags & O_CLOEXEC) && fcntl(fd, F_SETFD, FD_CLOEXEC) != 0)
      return -1;
   if ((flags & O_NONBLOCK) && fcntl(fd, F_SETFL, fcntl(fd, F_GETFL) | O_NONBLOCK) != 0)
      return -1;
   return 0;
}

int pipe2(int fds[2], int flags)
{
   if (flags & ~(O_CLOEXEC | O_NONBLOCK)) {
      errno = EINVAL;
      return -1;
   }
   if (pipe(fds) != 0)
      return -1;
   if (apply_fd_flags(fds[0], flags) != 0 || apply_fd_flags(fds[1], flags) != 0) {
      const int saved = errno;
      close(fds[0]);
      close(fds[1]);
      errno = saved;
      return -1;
   }
   return 0;
}

int dup3(int old_fd, int new_fd, int flags)
{
   if (old_fd == new_fd || (flags & ~O_CLOEXEC)) {
      errno = EINVAL;
      return -1;
   }
   if (dup2(old_fd, new_fd) < 0)
      return -1;
   if (apply_fd_flags(new_fd, flags) != 0)
      return -1;
   return new_fd;
}

/* Relative to the working directory only: a title has no other directory descriptor. */
int faccessat(int fd, const char *path, int mode, int flags)
{
   (void)flags;
   if (fd != AT_FDCWD && path[0] != '/') {
      errno = ENOTSUP;
      return -1;
   }
   return access(path, mode);
}

/* A title's folders hold no symbolic links. */
ssize_t readlinkat(int fd, const char *path, char *buffer, size_t size)
{
   (void)fd;
   (void)path;
   (void)buffer;
   (void)size;
   errno = EINVAL;
   return -1;
}

/* POSIX returns the error number itself from this one. */
int posix_fadvise(int fd, off_t offset, off_t length, int advice)
{
   (void)fd;
   (void)offset;
   (void)length;
   (void)advice;
   return EOPNOTSUPP;
}

#define REFUSED(declaration) \
   declaration \
   { \
      errno = ENOSYS; \
      return -1; \
   }

REFUSED(int accept4(int fd, struct sockaddr *address, socklen_t *length, int flags))
REFUSED(int getpeereid(int fd, uid_t *uid, gid_t *gid))
REFUSED(int mkfifo(const char *path, mode_t mode))
REFUSED(int setgid(gid_t gid))
REFUSED(int procctl(idtype_t type, id_t id, int command, void *data))
REFUSED(pid_t rfork(int flags))
REFUSED(int waitid(idtype_t type, id_t id, siginfo_t *info, int options))
REFUSED(int ps5_freebsd11_fstatfs(int fd, struct statfs *buffer))
REFUSED(int ps5_freebsd11_mknodat(int fd, const char *path, mode_t mode, unsigned dev))
__asm__(".symver ps5_freebsd11_fstatfs,fstatfs@FBSD_1.0");
__asm__(".symver ps5_freebsd11_mknodat,mknodat@FBSD_1.1");

/* Imported only through stubs no module a title loads backs (null at run time):
 * a title owns no users, groups, process groups or links to change. */
REFUSED(int chown(const char *path, uid_t owner, gid_t group))
REFUSED(int lchown(const char *path, uid_t owner, gid_t group))
REFUSED(int fchownat(int fd, const char *path, uid_t owner, gid_t group, int flags))
REFUSED(int chroot(const char *path))
REFUSED(int fchdir(int fd))
REFUSED(int setpgid(pid_t pid, pid_t group))
REFUSED(int linkat(int fd1, const char *path1, int fd2, const char *path2, int flags))
REFUSED(int symlinkat(const char *target, int fd, const char *path))

/* std's Command names these weakly (a newer FreeBSD's chdir for a spawned child). The console
 * spawns nothing, and the posix_spawn family reports failure as its return value. */
int posix_spawn_file_actions_addchdir(void *actions, const char *path)
{
   (void)actions;
   (void)path;
   return ENOSYS;
}

int posix_spawn_file_actions_addchdir_np(void *actions, const char *path)
{
   (void)actions;
   (void)path;
   return ENOSYS;
}
