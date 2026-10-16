#!/bin/sh
# Azeroth Survivors for the PlayStation 5 - the C (and, as cxx.sh, C++) compiler cc-rs builds the
# game's C dependencies with (Lua, and whatever else a crate compiles): PS5_Vulkan's PS5 clang
# wrapper, with the --target cc-rs derives from the Rust target dropped so the console's
# x86_64-sie-ps5 stays. Needs PS5_PAYLOAD_SDK and PS5_VULKAN (tools/build.sh exports both).
# SPDX-License-Identifier: MIT
set -eu
skip=0
for argument in "$@"; do
    shift
    if [ $skip = 1 ]; then skip=0; continue; fi
    case $argument in
    --target=*) continue ;;
    -target | --target) skip=1; continue ;;
    esac
    set -- "$@" "$argument"
done
case $(basename "$0") in
cxx.sh) PS5_CLANG=${PS5_CLANGXX:-$(command -v clang++ || command -v clang++-18)} ;;
esac
export PS5_CLANG=${PS5_CLANG:-}
exec sh "$PS5_VULKAN/tooling/prospero-clang18" -mno-red-zone "$@"
