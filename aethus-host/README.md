# Aethus native web-build host (experimental)

This host runs the published Aethus WebAssembly game on Windows and PlayStation 5.
wasm2c translates the game to C; QuickJS-ng runs its wasm-bindgen glue and the browser
API shims; the Rust host supplies graphics, sound, MPQ reads, storage and input.
The PS5 package uses title ID **PPSA99320**.

[Aethus 0.98.0 for PS5](https://github.com/Sleepywalker69/azeroth-survivors-ps5/releases/tag/ps5-0.98.0-20261018162554)
includes a normal title-folder ZIP, checksums and a live console update feed. It fixes
GitHub update error `0x80431073`; affected older hosts need this manual update once.
This release was built locally in WSL. The earlier 0.97.2 GitHub build passed in
**34 minutes 59 seconds**. Builds are manual only; no scheduled workflow is enabled.
Existing base game data and `patch.MPQ` are still required.

The console has updated from 0.96.0 to **0.97.0** and confirmed a successful first boot;
the corrected GitHub-feed installation of 0.98.0 still awaits console confirmation.
The current host measures approximately **47 FPS on the settled home screen** with
detailed profiling off. The original timing build measured about 31 FPS. Stable memory
growth substantially reduced loading stalls; zone transitions and gameplay performance
still need work. See the measurements below for the limits of these comparisons.

## Source layout

This directory is a publication snapshot of the working host. The verified build
layout currently places it **beside** this repository, not inside it. For a fresh
workspace, copy this directory out before building:

```powershell
git clone --branch ps5-port https://github.com/Sleepywalker69/azeroth-survivors-ps5 azeroth-survivors-re
Copy-Item -Recurse .\azeroth-survivors-re\aethus-host .\aethus-host
```

The workspace then contains:

```text
azeroth-survivors-re/     MPQ crate, patched wgpu-hal and PS5 platform source
aethus-host/             this host
aethus-web-0.96.0/        separately obtained web build, including snippets
aethus-build/0.96.0/      generated wasm2c/ and gen/; Windows libraries
tools/wabt/wabt-1.0.42/   wasm2c.exe, wasm2c runtime headers/source
tools/quickjs-ng/src/    QuickJS-ng 0.17.0 amalgamation and quickjs.h
```

Game downloads, MPQs, generated translation, build output, saves and runtime logs
are not included in this snapshot. Supply your own permitted game data.

## Build

The existing [PS5 build documentation](../ps5/README.md) describes the native
toolchain. This host was built in WSL with Rust 1.98.1, Clang 18,
[PS5_Vulkan](https://github.com/mihawk-99/PS5_Vulkan) and the
[PS5_PayloadSDK fork](https://github.com/mihawk-99/PS5_PayloadSDK).
The tested local checkouts were f410e67 and b5efad5 respectively; the installed SDK
and RADV archives are separate build inputs, so those source IDs alone do not
reproduce an installed toolchain.

Generate the translation and bridge, then run the PS5 build inside WSL:

```sh
# wasm2c: -n wenilla --num-outputs=128 -o <build>/wasm2c/wenilla.c
python3 -I aethus-host/gen/gen_bridge.py aethus-build/0.96.0/wasm2c/wenilla.h aethus-build/0.96.0/gen/bridge_gen.c
bash aethus-host/ps5/build.sh
```

`AETHUS_BUILD`, `AETHUS_WEB`, `AETHUS_WORK`, `PS5_VULKAN` and `PS5_PAYLOAD_SDK`
override the paths documented at the top of `ps5/build.sh`. The default package
is `~/ps5/aethus-ps5-build/dist/PPSA99320`. Translation compilation resumes by
skipping unchanged objects only when compiler flags, shared headers and wrapper
fingerprints also match. Use a separate `AETHUS_WORK` for comparison builds.
`AETHUS_C_OPT_FLAGS="-O3 -march=znver2 -ffp-contract=off"` selects the current CPU
optimization candidate. PS5 packaging also applies exact, reviewed string-helper
rewrites for native UTF-8 operations; unfamiliar helpers stop the build for review.

The Windows host uses `tools/build_wasm_lib.py` and `tools/build_host_lib.py` to
produce its C libraries; their argument documentation is in the scripts. Then,
from the workspace parent in PowerShell:

```powershell
aethus-host\tools\cargo-msvc.cmd build --offline --manifest-path aethus-host\rust\Cargo.toml
```

Use Python's `-I` flag when processing downloaded input. `sync_offline_registry.py`
can copy lockfile-verified crates and sparse index entries from an existing Windows
Cargo cache into WSL without downloading packages.

## Profiling and controls

Add `AETHUS_PROFILE=1` to the title's `env.txt`. The host writes five-second timing
windows to `userdata/aethus-host.log`: phase/host-operation totals, exclusive wasm
and JavaScript import time, memory usage, and linear-memory growth/copy costs.
Phase and operation totals overlap; bridge lane timings are exclusive. Profiling
adds overhead and is disabled by default. `AETHUS_FPS=1` with detailed profiling
disabled logs only frame cadence, avoiding per-import/operation timing overhead.

R3 toggles the right stick between pointer and camera mode. Camera mode holds the
right mouse button, centers and hides the pointer, and sends relative movement.
Options returns to pointer mode. Cross/R2 click, L2 holds the right mouse button,
left stick moves, and L1/R1 scroll. R3 transition tests passed on Windows; the
camera interaction is still awaiting console confirmation at this checkpoint. The
latest source fixes repeated toggles while R3 is held. Hold L3 + Options for two
seconds to check for an update or install a staged update and close.

See [PERFORMANCE.md](PERFORMANCE.md) for the measured baseline and open questions.

## Update and deploy tooling

`tools/update-pc.cmd --check` reads upstream metadata. `--download --build` downloads
the web build and any missing patches, checks imports/snippets against the reviewed
host compatibility policy, generates wasm2c/bridge output and builds offline in WSL.
Use `--web <existing-folder> --build` to rebuild an existing candidate offline.
Add `--cpu zen2` to select the CPU optimization candidate in a separate build cache.
Unknown executable glue changes stop for review; there is no silent fallback.

`--deploy` additionally requires `--build --title-closed`. The separate
`tools/deploy_ps5.py` uploads the icon first, verifies every staged file by full FTP
SHA-256 readback, and retains replaced files in a timestamped remote backup.
It does not close a running game. Do not use `--title-closed` while the title runs.

The PC updater's 0.97.0 download, compatibility checks and generic CPU build passed;
its direct FTP deployment proof is pending at this checkpoint. The user requested
that the next game upgrade run through the console auto-updater instead.

The console updater has downloaded and installed 0.97.0 and confirmed its first boot.
The initial copy dropped executable permissions; that launch failure was repaired, and
the current host preserves those permissions. Thirteen transaction/configuration tests
pass. On console, cleanup removes consumed staging and retains the previous backup.
The larger-buffer installer completed a console update that the tester reported as quick;
the exact duration was not retained across relaunch.
See [AUTO_UPDATE.md](ps5/AUTO_UPDATE.md) for controls, feed configuration, release
preparation and recovery details. The default GitHub feed is live.
[CLOUD_BUILD.md](ps5/CLOUD_BUILD.md) describes the verified GitHub publisher and how to
start a build manually. The PS5 can download published updates while the PC is off.

Existing source notices remain in place. Third-party license texts needed by the
PS5 package are in `ps5/licenses/`.
