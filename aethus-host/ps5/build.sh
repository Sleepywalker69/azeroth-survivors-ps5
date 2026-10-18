#!/usr/bin/env bash
# Azeroth Survivors (Aethus) for the PlayStation 5 - build the title folder.
#
#   ps5/build.sh   ->  $AETHUS_DIST/PPSA99320/ (eboot.bin, sce_sys/, sce_module/, js/, web/)
#
# 1. The wasm2c translation of the web build and wasm2c's runtime (build-wasm.sh).
# 2. The JS host: QuickJS, host.c, wasi.c and the generated bridge.
# 3. The Rust host (../rust) as a static library for the console: std built from source for
#    azeroth-survivors-re's x86_64-ps5-freebsd.json.
# 4. The title's C: main.c here, azeroth-survivors-re's platform.c, shims.c and rust-compat.c.
# 5. tools/link-title.sh: RADV, the CRT, the console's libraries, eboot.bin and the folder; then
#    the host's scripts and the web build beside it. The MPQs (Data/) are copied separately.
#
# Run inside WSL with the old port's toolchain (azeroth-survivors-re/ps5/README.md, Build):
#   PS5_VULKAN, PS5_PAYLOAD_SDK  as azeroth-survivors-re/ps5/tools/build.sh has them
#   AETHUS_BUILD   the translation and bridge (default ../../aethus-build/0.96.0)
#   AETHUS_WEB     the web build (default ../../aethus-web-0.96.0)
#   AETHUS_WORK    objects and archives (default ~/ps5/aethus-ps5-build)
#   AETHUS_DIST    where the title folder goes (default $AETHUS_WORK/dist)
#   JOBS           parallel compiles of the translation (default 8)
#   AETHUS_C_OPT_FLAGS  translated game and JS compiler options (default -O2); use a separate
#                       AETHUS_WORK for comparison builds, e.g. -O3 -march=znver2 -ffp-contract=off
set -euo pipefail
# Non-login WSL launches (including the PC updater) need rustup's tools as well.
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
ps5=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
host=$(dirname "$ps5")
repos=$(dirname "$host")
re=${AZEROTH_RE:-$repos/azeroth-survivors-re}
build=${AETHUS_BUILD:-$repos/aethus-build/0.96.0}
web=${AETHUS_WEB:-$repos/aethus-web-0.96.0}
work=${AETHUS_WORK:-$HOME/ps5/aethus-ps5-build}
dist=${AETHUS_DIST:-$work/dist}
rt=${WASM2C_RT:-$repos/tools/wabt/wabt-1.0.42/wasm2c}
quickjs=${QUICKJS_SRC:-$repos/tools/quickjs-ng/src}
export PS5_VULKAN=${PS5_VULKAN:-$HOME/ps5/PS5_Vulkan}
export PS5_PAYLOAD_SDK=${PS5_PAYLOAD_SDK:-$HOME/ps5/PS5_Ghostship/.deps/native/ps5-payload-sdk}
toolchain=$(sed -n 's/^channel = "\(.*\)"/\1/p' "$re/rust-toolchain.toml")
mkdir -p "$work/obj"
cc() { sh "$re/ps5/tools/cc.sh" "$@"; }
ar=$(command -v llvm-ar || command -v llvm-ar-18)
wasm_defines=(-DWASM_RT_USE_MMAP=0 -DWASM_RT_MEMCHECK_BOUNDS_CHECK=1 -DWASM_RT_NONCONFORMING_UNCHECKED_STACK_EXHAUSTION=1)
read -r -a c_opt_flags <<< "${AETHUS_C_OPT_FLAGS:--O2}"

echo "==> reviewed native string helpers"
python3 -I "$host/tools/prepare_glue.py" "$web/wenilla.js" "$work/wenilla-host.js"

echo "==> the translation"
bash "$ps5/build-wasm.sh" "$build/wasm2c" "$rt" "$work" "${JOBS:-8}"

echo "==> the JS host"
js_objects=()
for unit in "$quickjs/quickjs-amalgam.c:quickjs:-w" "$build/gen/bridge_gen.c:bridge_gen:-Wall" \
        "$host/c/host.c:host:-Wall" "$host/c/wasi.c:wasi:-Wall"; do
    IFS=: read -r source name warnings <<<"$unit"
    object="$work/obj/$name.o"
    config=$( { printf '%s\n' "${c_opt_flags[*]}" "$build" "$rt" "$quickjs";
        sha256sum "$ps5/build.sh" "$re/ps5/tools/cc.sh" "$PS5_VULKAN/tooling/prospero-clang18" \
            "$host/c/host.h" "$host/c/bridge.h" "$quickjs"/*.h "$rt"/*.h "$build/wasm2c/wenilla.h";
    } | sha256sum | cut -d' ' -f1)
    if [[ ! $object -nt $source || ! -f $object.config || $(cat "$object.config") != "$config" ]]; then
        cc -std=gnu11 "${c_opt_flags[@]}" -fPIC "$warnings" -Wno-unused-parameter -D_setjmp=setjmp -D_longjmp=longjmp \
            "${wasm_defines[@]}" -I"$host/c" -I"$build/wasm2c" -I"$rt" -I"$quickjs" -c "$source" -o "$object"
        printf '%s\n' "$config" > "$object.config"
    fi
    js_objects+=("$object")
done
rm -f "$work/libaethus_js.a"
"$ar" rcs "$work/libaethus_js.a" "${js_objects[@]}"

echo "==> the Rust host (cargo +$toolchain, x86_64-ps5-freebsd)"
target_dir=${CARGO_TARGET_DIR:-$HOME/ps5/aethus-target}
prepared=$(python3 -I "$re/ps5/tools/prepare-rust-std.py")
printf '%s\n' "$prepared" >&2
if grep -q '^patched' <<<"$prepared"; then
    rm -rf "$target_dir"/x86_64-ps5-freebsd/*/.fingerprint/std-*
fi
export RUSTC_BOOTSTRAP=1
export RUST_LIBC_UNSTABLE_FREEBSD_VERSION=11
export CC_x86_64_ps5_freebsd="$re/ps5/tools/cc.sh"
export CXX_x86_64_ps5_freebsd="$re/ps5/tools/cxx.sh"
export AR_x86_64_ps5_freebsd=$ar
export CFLAGS_x86_64_ps5_freebsd="-O2 -D_setjmp=setjmp -D_longjmp=longjmp"
export CXXFLAGS_x86_64_ps5_freebsd="-O2"
export RUSTFLAGS="--cfg libc_unstable_freebsd_version=\"11\" -C target-cpu=znver2 ${RUSTFLAGS_EXTRA:-}"
cargo "+$toolchain" rustc --offline --lib --crate-type staticlib --release \
    --manifest-path "$host/rust/Cargo.toml" --target "$re/ps5/x86_64-ps5-freebsd.json" \
    -Z json-target-spec -Z build-std=std,panic_abort --target-dir "$target_dir"
rust="$target_dir/x86_64-ps5-freebsd/release/libaethus_host.a"
[[ -f $rust ]] || { echo "no $rust" >&2; exit 1; }

echo "==> the title's C"
objects=()
for source in "$ps5/main.c" "$re/ps5/src/platform.c" "$re/ps5/src/shims.c" "$re/ps5/src/rust-compat.c"; do
    object="$work/obj/$(basename "${source%.c}").o"
    cc -std=gnu11 -O2 -fPIC -Wall -I"$re/ps5/src" -DPS5_TITLE_NAME='"AzerothAethus"' -c "$source" -o "$object"
    objects+=("$object")
done

echo "==> link"
cc -std=gnu11 -O2 -fPIC -Wall "${wasm_defines[@]}" -I"$host/c" -I"$rt" \
    -c "$ps5/wasm-memory.c" -o "$work/obj/wasm-memory.o"
objects+=("$work/obj/wasm-memory.o")
PS5_DIST_DIR=$dist bash "$ps5/tools/link-title.sh" "$work" "$PS5_VULKAN" "$PS5_PAYLOAD_SDK" \
    "${objects[@]}" "$work/libaethus_js.a" "$work/libwenilla.a" "$rust"

app="$dist/$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["titleId"])' "$ps5/sce_sys/param.json")"
echo "==> the page: $app/js, $app/web"
rm -rf "$app/js" "$app/web"
mkdir -p "$app/js" "$app/web"
cp "$host"/js/*.js "$host"/js/*.mjs "$app/js/"
cp "$work/wenilla-host.js" "$app/web/wenilla.js"
[[ -d $web/snippets ]] && cp -r "$web/snippets" "$app/web/"
echo "==> $app (copy Data/ beside eboot.bin: patch.MPQ, patch-2.MPQ ... patch-5.MPQ)"
