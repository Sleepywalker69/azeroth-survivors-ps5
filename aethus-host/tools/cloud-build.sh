#!/usr/bin/env bash
# Run from Linux with an extracted, verified kit and the host beside the source checkout.
# cloud-build.sh KIT SOURCE_CHECKOUT WORK [cloud_release.py prepare options...]
set -euo pipefail
kit=$(realpath "$1")
repo=$(realpath "$2")
work=$(realpath -m "$3")
shift 3
host=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
toolchain=$(sed -n 's/^channel = "\(.*\)"/\1/p' "$repo/rust-toolchain.toml")
[[ $toolchain =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo 'Unexpected Rust toolchain version' >&2; exit 1; }
[[ $("$kit/rust/bin/rustc" --version) == "rustc $toolchain "* ]] || { echo 'Kit Rust version differs from source' >&2; exit 1; }
for binary in clang-18 clang++-18 llvm-config-18 llvm-ar-18 llvm-nm-18 ld.lld-18 rustup; do
    command -v "$binary" >/dev/null || { echo "Missing build prerequisite: $binary" >&2; exit 1; }
done
mkdir -p "$work/rustup/toolchains"
link="$work/rustup/toolchains/$toolchain-x86_64-unknown-linux-gnu"
if [[ ! -e $link ]]; then ln -s "$kit/rust" "$link"; fi
[[ $(readlink -f "$link") == "$kit/rust" ]] || { echo 'Toolchain link points outside this kit' >&2; exit 1; }
export RUSTUP_HOME="$work/rustup" RUSTUP_TOOLCHAIN="$toolchain"
export CARGO_HOME="$kit/cargo" CARGO_TARGET_DIR="$work/target" CARGO_BUILD_JOBS=4
export AETHUS_RELEASES="$work/releases"
export LLVM_CONFIG="$(command -v llvm-config-18)" PS5_CLANG=clang-18 PS5_CLANGXX=clang++-18
export AZEROTH_RE="$repo" PS5_PAYLOAD_SDK="$kit/sdk" PS5_VULKAN="$kit/vulkan"
export WASM2C="$kit/wabt/bin/wasm2c" WASM2C_RT="$kit/wabt/wasm2c" QUICKJS_SRC="$kit/quickjs"
python3 -I "$host/tools/cloud_release.py" prepare "$@"
