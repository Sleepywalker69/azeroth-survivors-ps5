#!/usr/bin/env bash
# Azeroth Survivors for the PlayStation 5 - build the title folder.
#
#   ps5/tools/build.sh            ->  dist/PPSA99320/ (eboot.bin, sce_sys/, sce_module/)
#
# 1. The game, Rust, as a static library for the console (crates/survivors-ps5): std built from
#    source for x86_64-ps5-freebsd.json (FreeBSD 11's libc layout, no thread locals, abort on
#    panic), its C dependencies by tools/cc.sh.
# 2. The title's C: src/main.c (start-up), src/platform.c (klog, pad, mouse, keyboard, sound,
#    the exit), src/shims.c and src/rust-compat.c (libc names a title lacks).
# 3. tools/link-title.sh: RADV, the CRT, the console's libraries, eboot.bin and the folder.
#
# Run inside Linux (WSL works) with PS5_Vulkan bootstrapped (its tools/setup-native-dependencies.sh,
# rebuild-libc.sh and build-radv.sh release) and a payload SDK at the title pin:
#   PS5_VULKAN       default ~/ps5/PS5_Vulkan
#   PS5_PAYLOAD_SDK  default ~/ps5/PS5_Ghostship/.deps/native/ps5-payload-sdk
#   PROFILE          cargo profile, default release (ship = fat LTO)
#
# SPDX-License-Identifier: MIT
set -euo pipefail
ps5=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
root=$(dirname "$ps5")
export PS5_VULKAN=${PS5_VULKAN:-$HOME/ps5/PS5_Vulkan}
export PS5_PAYLOAD_SDK=${PS5_PAYLOAD_SDK:-$HOME/ps5/PS5_Ghostship/.deps/native/ps5-payload-sdk}
profile=${PROFILE:-release}
work=${PS5_BUILD_DIR:-$root/build/ps5}
mkdir -p "$work/obj"

for file in "$PS5_VULKAN/tooling/prospero-clang18" "$PS5_PAYLOAD_SDK/bin/prospero-lld"; do
    [[ -e $file ]] || { echo "missing: $file (bootstrap PS5_Vulkan and the SDK first)" >&2; exit 2; }
done

echo "==> the game (cargo $profile, x86_64-ps5-freebsd)"
python3 "$ps5/tools/prepare-rust-std.py"
export RUSTC_BOOTSTRAP=1
export RUST_LIBC_UNSTABLE_FREEBSD_VERSION=11
export CC_x86_64_ps5_freebsd="$ps5/tools/cc.sh"
export CXX_x86_64_ps5_freebsd="$ps5/tools/cxx.sh"
export AR_x86_64_ps5_freebsd=${AR_PS5:-$(command -v llvm-ar || command -v llvm-ar-18)}
# Lua 5.1 unwinds with _setjmp/_longjmp (LUA_USE_ULONGJMP, FreeBSD's luaconf); the console's C
# library has setjmp/longjmp only, which also keep the signal mask a title never changes.
export CFLAGS_x86_64_ps5_freebsd="-O2 -D_setjmp=setjmp -D_longjmp=longjmp"
export CXXFLAGS_x86_64_ps5_freebsd="-O2"
export RUSTFLAGS="--cfg libc_unstable_freebsd_version=\"11\" -C target-cpu=znver2 ${RUSTFLAGS_EXTRA:-}"
target_dir=${CARGO_TARGET_DIR:-$root/target}
cargo build --manifest-path "$root/Cargo.toml" -p survivors-ps5 --profile "$profile" \
    --target "$ps5/x86_64-ps5-freebsd.json" -Z json-target-spec \
    -Z build-std=std,panic_abort --target-dir "$target_dir"
case $profile in dev) out=debug ;; *) out=$profile ;; esac
game="$target_dir/x86_64-ps5-freebsd/$out/libsurvivors_ps5.a"
[[ -f $game ]] || { echo "no $game" >&2; exit 1; }

# Native libraries a crate builds but leaves out of the archive (`static:-bundle`, as mlua-sys
# does Lua): the title's link takes them beside it.
native=()
while IFS= read -r name; do
    # The newest, when old build-script runs left copies behind.
    found=$(find "$target_dir/x86_64-ps5-freebsd/$out/build" -name "lib$name.a" -printf '%T@ %p\n' |
        sort -rn | head -1 | cut -d' ' -f2-)
    [[ -n $found ]] || { echo "no lib$name.a for the link" >&2; exit 1; }
    native+=("$found")
done < <(cat "$target_dir"/x86_64-ps5-freebsd/"$out"/build/*/output 2>/dev/null |
    sed -n 's/^cargo:rustc-link-lib=static:-bundle=//p; s/^cargo::rustc-link-lib=static:-bundle=//p' | sort -u)

echo "==> the title's C"
cc() { sh "$ps5/tools/cc.sh" "$@"; }
objects=()
for source in main platform shims rust-compat; do
    cc -std=gnu11 -O2 -fPIC -Wall -I"$ps5/src" -DPS5_TITLE_NAME='"AzerothSurvivors"' \
        -c "$ps5/src/$source.c" -o "$work/obj/$source.o"
    objects+=("$work/obj/$source.o")
done

echo "==> link"
PS5_DIST_DIR=${PS5_DIST_DIR:-$root/dist} bash "$ps5/tools/link-title.sh" "$work" "$PS5_VULKAN" \
    "$PS5_PAYLOAD_SDK" "${objects[@]}" "$game" "${native[@]}"
