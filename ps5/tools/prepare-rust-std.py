#!/usr/bin/env python3
# Azeroth Survivors for the PlayStation 5 - the one change Rust's std needs for the console's
# FreeBSD 11 layout: `dirent.d_fileno` is 32 bits there and std's FreeBSD backend expects the
# 64-bit `d_ino`. Patches the toolchain's rust-src in place, once; refuses if the context moved.
# The same patch as ps5-ai-cli's tools/prepare-rust-std.py (Apache-2.0).
# SPDX-License-Identifier: MIT
from pathlib import Path
import subprocess

sysroot = Path(subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip())
source = sysroot / "lib/rustlib/src/rust/library/std/src/sys/fs/unix.rs"
before = "d_ino: (*entry_ptr).d_fileno,"
after = "d_ino: (*entry_ptr).d_fileno.into(),"
text = source.read_text()
if after not in text:
    if text.count(before) != 1:
        raise SystemExit("Rust std patch context changed; refusing to patch")
    source.write_text(text.replace(before, after))
    print("patched", source)
