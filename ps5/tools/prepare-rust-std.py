#!/usr/bin/env python3
# Azeroth Survivors for the PlayStation 5 - the changes Rust's std needs for the console.
# Patches the toolchain's rust-src in place, once each; refuses if a context moved.
#
# - `dirent.d_fileno` is 32 bits in the console's FreeBSD 11 layout, and std's FreeBSD backend
#   expects the 64-bit `d_ino` (the same patch as ps5-ai-cli's tools/prepare-rust-std.py,
#   Apache-2.0).
# - std's FreeBSD backend finds `environ` with dlsym(RTLD_DEFAULT, "environ"), which is NULL on
#   the console, so the first std::env::vars() read address 0 (seen on the console: SIGSEGV in
#   std::sys::env::unix::env at start-up). The console's libkernel exports `environ` and the
#   title imports it like any POSIX target.
# SPDX-License-Identifier: MIT
from pathlib import Path
import subprocess

sysroot = Path(subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip())
library = sysroot / "lib/rustlib/src/rust/library/std/src/sys"

PATCHES = [
    (
        "fs/unix.rs",
        "d_ino: (*entry_ptr).d_fileno,",
        "d_ino: (*entry_ptr).d_fileno.into(),",
    ),
    (
        "env/unix.rs",
        '// On FreeBSD, environ comes from CRT rather than libc\n#[cfg(target_os = "freebsd")]',
        '// On FreeBSD, environ comes from CRT rather than libc\n'
        '#[cfg(all(target_os = "freebsd", not(target_env = "ps5")))]',
    ),
    (
        "env/unix.rs",
        '#[cfg(not(any(target_os = "freebsd", target_vendor = "apple")))]\npub unsafe fn environ()',
        '#[cfg(not(any(all(target_os = "freebsd", not(target_env = "ps5")), target_vendor = "apple")))]\n'
        "pub unsafe fn environ()",
    ),
]

for name, before, after in PATCHES:
    source = library / name
    text = source.read_text()
    if after in text:
        continue
    if text.count(before) != 1:
        raise SystemExit(f"Rust std patch context changed in {name}; refusing to patch")
    source.write_text(text.replace(before, after))
    print("patched", source)
