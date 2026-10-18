#!/usr/bin/env bash
# Compile the wasm2c translation of the web build, with wasm2c's runtime, for the PlayStation 5
# into one static library.
#
#   ps5/build-wasm.sh C_DIR RT_DIR WORK [JOBS]   ->  WORK/libwenilla.a
#
# C_DIR is wasm2c's output (wenilla.h, wenilla_N.c), RT_DIR wabt's wasm2c folder (wasm-rt.h and
# the runtime). wasm-memory.c reserves a stable address range backed by direct memory, with
# calloc/realloc as a fallback. WASM_RT_USE_MMAP=0 keeps explicit bounds checks in translated
# accesses; it does not select the title's allocator. There is no call-depth counting. Files that
# include wasm-rt.h (host.c, the bridge) must be compiled with the same defines (WASM_DEFINES).
# AETHUS_C_OPT_FLAGS selects compiler optimization (default -O2). Objects are reused only when
# their source is older and their compiler flags, wrapper and shared headers still match.
set -euo pipefail
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
re=${AZEROTH_RE:-$(cd -- "$here/../../azeroth-survivors-re" && pwd)}
export PS5_VULKAN=${PS5_VULKAN:-$HOME/ps5/PS5_Vulkan}
export PS5_PAYLOAD_SDK=${PS5_PAYLOAD_SDK:-$HOME/ps5/PS5_Ghostship/.deps/native/ps5-payload-sdk}
c_dir=$1 rt_dir=$2 work=$3 jobs=${4:-8}
mkdir -p "$work/obj-wasm"

export WASM_CC="$re/ps5/tools/cc.sh"
export WASM_FLAGS="${AETHUS_C_OPT_FLAGS:--O2} -fPIC -w -fno-strict-aliasing -D_setjmp=setjmp -D_longjmp=longjmp \
-DWASM_RT_USE_MMAP=0 -DWASM_RT_MEMCHECK_BOUNDS_CHECK=1 -DWASM_RT_NONCONFORMING_UNCHECKED_STACK_EXHAUSTION=1 \
-I$rt_dir -I$c_dir"
export WASM_OBJ="$work/obj-wasm"
WASM_CONFIG=$( { printf '%s\n' "$WASM_CC" "$WASM_FLAGS" "$PS5_PAYLOAD_SDK";
    sha256sum "$here/build-wasm.sh" "$WASM_CC" "$PS5_VULKAN/tooling/prospero-clang18" "$c_dir"/*.h "$rt_dir"/*.h "$rt_dir"/*.inc;
} | sha256sum | cut -d' ' -f1)
export WASM_CONFIG
printf 'translation compiler flags: %s\n' "$WASM_FLAGS"

compile() {
    local src=$1 obj
    obj="$WASM_OBJ/$(basename "${src%.c}").o"
    if [[ $obj -nt $src && -f $obj.config && $(cat "$obj.config") == "$WASM_CONFIG" ]]; then
        return 0
    fi
    local t0=$SECONDS
    # shellcheck disable=SC2086
    if sh "$WASM_CC" $WASM_FLAGS -c "$src" -o "$obj.tmp" 2> "$obj.log"; then
        mv "$obj.tmp" "$obj"
        printf '%s\n' "$WASM_CONFIG" > "$obj.config"
        echo "$(basename "$src") $((SECONDS - t0))s"
    else
        echo "FAILED $(basename "$src")"
        tail -20 "$obj.log"
        return 1
    fi
}
export -f compile

# Largest first, so the long poles start early. A failed directory listing must
# stop the build, rather than silently creating an archive of only the runtime.
sources=$(python3 -I -c '
from pathlib import Path
import sys
directory, runtime = map(Path, sys.argv[1:])
units = [directory / f"wenilla_{i}.c" for i in range(128)]
units.sort(key=lambda path: path.stat().st_size, reverse=True)
units += [runtime / name for name in ("wasm-rt-impl.c", "wasm-rt-mem-impl.c", "wasm-rt-exceptions-impl.c")]
for path in units:
    if not path.is_file() or path.stat().st_size == 0:
        raise ValueError(f"missing or empty translation input: {path}")
print("\n".join(map(str, units)))
' "$c_dir" "$rt_dir")
t0=$SECONDS
printf '%s\n' "$sources" | xargs -P "$jobs" -I{} bash -c 'compile "$1"' _ {}
objects=()
while IFS= read -r src; do
    objects+=("$WASM_OBJ/$(basename "${src%.c}").o")
done <<<"$sources"
ar=$(command -v llvm-ar || command -v llvm-ar-18)
rm -f "$work/libwenilla.a"
"$ar" rcs "$work/libwenilla.a" "${objects[@]}"
echo "wrote $work/libwenilla.a in $((SECONDS - t0))s"
