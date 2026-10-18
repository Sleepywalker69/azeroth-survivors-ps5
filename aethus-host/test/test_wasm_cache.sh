#!/usr/bin/env bash
# Verify that a resumed PS5 translation build respects flags and shared headers.
set -euo pipefail
host=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
rt=${WASM2C_RT:-$host/../tools/wabt/wabt-1.0.42/wasm2c}
scratch=$(mktemp -d /tmp/aethus-cache-test.XXXXXX)
trap 'rm -rf -- "$scratch"' EXIT
mkdir -p "$scratch/src"
printf '/* cache test */\n' > "$scratch/src/wenilla.h"
printf 'int cache_test(void) { return 7; }\n' > "$scratch/src/wenilla_0.c"
build() {
    AETHUS_C_OPT_FLAGS=$1 bash "$host/ps5/build-wasm.sh" "$scratch/src" "$rt" "$scratch/work" 2 > "$scratch/result.log" 2>&1
    local count
    count=$(grep -cE '\.c [0-9]+s$' "$scratch/result.log" || true)
    if [[ $count != "$2" ]]; then cat "$scratch/result.log"; echo "expected $2 compiles, got $count" >&2; exit 1; fi
}
build '-O2' 4
build '-O2' 0
build '-O3 -march=znver2 -ffp-contract=off' 4
build '-O3 -march=znver2 -ffp-contract=off' 0
printf '/* changed header */\n' >> "$scratch/src/wenilla.h"
build '-O3 -march=znver2 -ffp-contract=off' 4
printf 'int cache_test(void) { return 8; }\n' > "$scratch/src/wenilla_0.c"
build '-O3 -march=znver2 -ffp-contract=off' 1
echo 'PASS: resumes unchanged objects; rebuilds changed flags, headers and sources'
