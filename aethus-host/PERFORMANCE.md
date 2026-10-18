# Performance investigation — 2026-10-18

Target: PS5, PPSA99320, 0.96.0 game build, 1920x1080 surface, FIFO presentation.
Host architecture: wasm2c C, QuickJS-ng 0.17.0, Rust/wgpu 27, PS5 RADV.

## First measured baseline

| Observation | Measured result |
| --- | --- |
| Settled home screen | approximately 31 frame callbacks/second |
| Frame callback | approximately 28–29 ms each |
| Native command encoding | approximately 1.4–1.5 ms/frame |
| Native resource creation | approximately 0.7 ms/frame |
| Initial loading | some frame/tick callbacks stall for several seconds |
| File reads and native graphics during those stalls | a small fraction of elapsed time |

For example, at 212 seconds the five-second window contained 157 frames, 4469.5 ms
of frame callbacks, 216.9 ms of native encoding, and 105.8 ms of native resource
creation. These totals are nested and must not be added together. This is an
instrumented home-screen measurement, not a gameplay benchmark or GPU timestamp.

Earlier logs also recorded terrain collision builds up to 255 ms, hundreds of
pipeline creations and audio starvation while loading. Those observations alone
do not explain the sustained frame cost. The timing data does not yet isolate
translated game computation from JavaScript bridge/runtime work.

## Next measurements

The deeper timing build reports exclusive wasm and JS/import time, JS/linear
memory sizes, direct-heap statistics and time spent in `wasm_rt_grow_memory`,
including relocation counts and estimated bytes copied. It identified the
dominant loading cost: repeated whole-heap relocation on small memory growth.

In the 23.5-second window ending at 79 seconds, 430 growth calls consumed 23,224.8 ms
and all 430 moved the heap, copying approximately 290,497.6 MiB cumulatively. The
final logical memory size was only 705.7 MiB. Other windows show the same pattern.
The allocator recorded zero libc fallbacks. A fix that grows within a stable
reserved address range is being implemented; no improvement is claimed yet.

The title already uses the SDK's direct-memory heap. Its flexible-memory setting
is not the total memory budget. The PS5 wasm runtime currently uses calloc/realloc
for linear memory, whereas the Windows host reserves/commits virtual memory.
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
