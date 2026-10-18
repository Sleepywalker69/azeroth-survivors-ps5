# Performance investigation â€” 2026-10-18

Target: PS5, PPSA99320, 0.96.0 game build, 1920x1080 surface, FIFO presentation.
Host architecture: wasm2c C, QuickJS-ng 0.17.0, Rust/wgpu 27, PS5 RADV.

## First measured baseline

| Observation | Measured result |
| --- | --- |
| Settled home screen | approximately 31 frame callbacks/second |
| Frame callback | approximately 28â€“29 ms each |
| Native command encoding | approximately 1.4â€“1.5 ms/frame |
| Native resource creation | approximately 0.7 ms/frame |
| Initial loading | some frame/tick callbacks stall for several seconds |
| File reads and native graphics during those stalls | a small fraction of elapsed time |

For example, at 212 seconds the five-second window contained 157 frames, 4469.5 ms
of frame callbacks, 216.9 ms of native encoding, and 105.8 ms of native resource
creation. These totals are nested and must not be added together. This is an
instrumented home-screen measurement, not a gameplay benchmark or GPU timestamp.

Earlier logs also recorded terrain collision builds up to 255 ms, hundreds of
pipeline creations and audio starvation while loading. Those observations alone
do not explain the sustained frame cost. Deeper measurements below separate translated game computation from bridge/runtime work.

## Next measurements

The deeper timing build reports exclusive wasm and JS/import time, JS/linear
memory sizes, direct-heap statistics and time spent in `wasm_rt_grow_memory`,
including relocation counts and estimated bytes copied. It identified the
dominant loading cost: repeated whole-heap relocation on small memory growth.

In the 23.5-second window ending at 79 seconds, 430 growth calls consumed 23,224.8 ms
and all 430 moved the heap, copying approximately 290,497.6 MiB cumulatively. The
final logical memory size was only 705.7 MiB. Other windows show the same pattern.
The allocator recorded zero libc fallbacks. The stable-address fix below removes these repeated copies.

The title already uses the SDK's direct-memory heap. Its flexible-memory setting
is not the total memory budget. The original PS5 wasm runtime used calloc/realloc for linear memory, whereas the
Windows host reserves/commits virtual memory.
Repeated relocation is now measured as the dominant loading stall. The settled
frame cost still needs independent attribution and comparison.

Do not claim a performance improvement without comparing the same scene and game
version after loading, and checking the result with profiling disabled as well.

## Verification at this checkpoint

- PS5 0.96.0 build and package integrity checks pass; deployed files were read back
  and SHA-256 verified. User confirms the game reaches the main screen and gameplay.
- Initial timing eboot SHA-256:
  `d3b363a75feeb0550e7de81e7d44235dd442ab412e517ad4519a11a901df9c77`.
- Deeper timing/R3 eboot SHA-256:
  `166870dfe6b3af7c8d2e51a1241f3815034caaf654620eec8816fbfeb9435b6e`.
- Windows Rust build passes. Updated Windows C host libraries also compile.
- Two input transition tests and eight updater validation tests pass.
- Steady gameplay performance, R3 camera behavior, the PC updater's complete
  0.97.0 flow and console auto-update remain unproved.

## Stable memory: console result

The PS5 allocator now reserves 4 GiB of virtual address space and commits direct
memory in 64 MiB chunks. This is virtual reservation, not a 4 GiB physical allocation.
Explicit wasm bounds checks remain enabled. Failed initial reservation or mapping
falls back to the original allocator; `AETHUS_WASM_REALLOC=1` forces that old path.

The same 0.96.0 build now completes its large initial heap-growth burst by roughly
15 seconds, rather than roughly 115 seconds in the earlier run. The first 15 seconds
contain 2,427 growth calls taking 47.0 ms total, with **zero relocations or copied
heap bytes**. Logical memory reaches 1,162.9 MiB, with 1,216 MiB committed. User
feedback confirms loading is much better, with some remaining lag.

Settled home-screen windows measured 32.7-32.8 frame callbacks/second; later
in-world windows were around 25. This is still instrumented and is not a claim
of smooth gameplay. Zone entry also has a separate 6.3-second frame: the containing
window spent 2.99 seconds in string-to-wasm conversion, 0.89 seconds in `has`,
and 0.70 seconds converting strings from wasm. A later window spent 1.24 seconds
creating 346 render pipelines. CPU and bridge optimization remains in progress.

The fix passed native allocator tests for stable addresses, zero fill, data
preservation, limits, allocation/map failures and cleanup, then ran on the console.
Eboot SHA-256: `d00f62432b25387e96c492b63b9ce0cb6ac381243b8b67bf2fcb83ae9dc4a525`.
Evidence retained locally: `runtime-096-stable-01.log`, `memory-tests-01.log`,
and `stable-memory-deploy/deploy-20261018T090807695048Z.json`.

## String-copy result and next CPU candidate

An encoder-only rewrite uses the existing native TextEncoder and bulk typed-array
copy instead of wasm-bindgen's interpreted ASCII byte loop. With the same 0.96.0
executable, PS5 string_get time fell from approximately 4.2 to 2.3 microseconds per
call. The largest zone-entry frame in these two runs fell from 6.31 to 5.10 seconds.
A settled home-screen window reached 33.9 FPS, but early gameplay remained around
25 FPS and a longer run later ranged around 18-22 FPS. Scene differences limit
comparisons; this is not a substantial gameplay FPS fix by itself.

A separate candidate compiles the translated game and QuickJS with
`-O3 -march=znver2 -ffp-contract=off`, adds bounds-checked direct UTF-8 reads from
current wasm memory, and removes the extra quarter-frame delay after slow frames.
It has built successfully, passed package integrity checks and run on the console.
Settled home-screen windows now measure about 40-41 FPS (previously about 31).
Recent gameplay windows were approximately 27-32 FPS, with detailed profiling still
enabled. Scene and imported-save differences limit gameplay comparisons.
Evidence: runtime-096-zen2-01.log. Eboot SHA-256:
`bdb9ee04753c4e72ba91c88c6415efcc7d2e7dd464cfd9064f14c467fd566a16`.
Prepared 0.96.0 glue SHA-256:
`9735310524234da28d99ab620b2fb397122325cce73ade47926ed43847a06938`.

Native QuickJS regression tests cover exact-helper rejection, ASCII, empty/NUL
strings, Unicode/surrogates, allocation exceptions, moved memory, malformed UTF-8,
BOM behavior and bounds checks. The encoder microbenchmark on the build PC took
763 ms before and 267 ms after for 200,000 short transfers. This is a focused CPU
test, not the console's frame rate. Windows C/Rust builds also pass. Compiler-cache
tests verify reuse and invalidation on flag, header and source changes; nine updater
validation tests pass.


## Cadence without detailed profiling

The 0.96.0 updater build with the CPU, string and frame-pacing fixes measured
45.3-46.8 frame callbacks/second on the home screen while checking files and
downloading a 262 MB update. `AETHUS_PROFILE=0`, `AETHUS_FPS=1`; the initial
five-second loading window was 11.2 FPS. This removes the detailed per-import
timing overhead present in the earlier 40-41 FPS measurements. It is not a
controlled gameplay comparison. Evidence: `runtime-updater-path-02.log`.
Eboot SHA-256:
`8838134ef1ba491e3e33f1b081a0d6c10d9c598b6404497c23df99778e153bf7`.
