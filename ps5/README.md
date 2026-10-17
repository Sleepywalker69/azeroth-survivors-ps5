# Azeroth Survivors for the PlayStation 5

The survivors mode of benilla as a home-screen title for a jailbroken PS5 (target:
firmware 13.60; runs on a 13.60 console at 3840x2160 and 60 fps). It is a real title in its own folder, not a payload: it shows on the home screen
with its own tile and background, starts like any game, and closes with the PS button.

| | |
| --- | --- |
| Title ID | `PPSA99320` |
| Title folder | `/data/homebrew/PPSA99320/` (the title sees it as `/app0`) |
| Game data | `/data/homebrew/PPSA99320/Data/` |
| Settings, session, logs, crash reports | `/data/homebrew/PPSA99320/userdata/` |
| Log | klog, every line prefixed `[AzerothSurvivors]` |

## What the console needs

- A PS5 that runs homebrew, with etaHEN (or an equivalent) loaded.
- ShadowMountPlus, which shows the folders in `/data/homebrew` as titles on the home screen.
- An FTP server on the console to copy files with (etaHEN's listens on port 1337; some setups
  use 2121).

## Install

1. Download [`ps5/prebuilt/PPSA99320.zip`](prebuilt/PPSA99320.zip) and unzip it. The zip holds
   one folder, `PPSA99320`, which renders at 3840x2160 by default. You can also build the folder
   yourself (see Build), which gives you `dist/PPSA99320/`.
2. Copy the whole folder to the console as `/data/homebrew/PPSA99320/`:

   ```text
   /data/homebrew/PPSA99320/eboot.bin
   /data/homebrew/PPSA99320/sce_sys/          (param.json, icon0.png, pic0.dds, pic1.dds)
   /data/homebrew/PPSA99320/sce_module/libc.prx
   /data/homebrew/PPSA99320/licenses/
   ```

   Copy `sce_sys/icon0.png` with the first upload: ShadowMountPlus will not register a title
   without its icon.
3. Copy a full WoW 1.12.1 (build 5875) client's `Data` folder beside `eboot.bin`, about 5 GB.
   `scripts/fetch-data.sh` downloads one into `wow-data/Data/`. The Aethus Windows release's
   two-file `Data` folder is not enough: its `patch.MPQ` holds only that build's arenas, and
   this build waits on the loading screen for terrain that is not there. The folder looks like
   this:

   ```text
   /data/homebrew/PPSA99320/Data/terrain.MPQ
   /data/homebrew/PPSA99320/Data/model.MPQ
   /data/homebrew/PPSA99320/Data/texture.MPQ
   /data/homebrew/PPSA99320/Data/...           (the rest of the client's MPQs)
   /data/homebrew/PPSA99320/Data/patch.MPQ
   /data/homebrew/PPSA99320/Data/patch-2.MPQ
   ```

4. Wait a few seconds for "Azeroth Survivors" to appear on the home screen, and start it.

If the title does not appear, look in `/data/shadowmount/debug.log`. "AppInstallAll hook is no
longer installed" means ShadowMountPlus has to be restarted before it will register a new title.

### Options

`/data/homebrew/PPSA99320/env.txt`, one `NAME=value` per line, sets the game's environment
before it starts. Useful ones:

| Line | Effect |
| --- | --- |
| `PS5_DISPLAY_MODE=1920x1080` | Render at this size instead of 3840x2160 (`2560x1440` also works); VideoOut scales it to the screen |
| `WOW_DATA=/app0/OtherData` | Read the MPQs from elsewhere in the title folder |
| `WOW_THREADS=1` | Run the frame's systems on one thread (a diagnostic) |

## Controls

A DualSense drives the game; a USB or Bluetooth keyboard and mouse work too, as on a computer.

| Pad | Does |
| --- | --- |
| Left stick | Move (W A S D) |
| Right stick | Move the cursor (aim, menus) |
| Cross | Click; start a run (Enter) |
| Circle | Jump (Space) |
| Square | Mobility skill (F) |
| Triangle | G |
| D-pad left / up / right | Pick card 1 / 2 / 3 on a level-up |
| D-pad down | Enter |
| R2 / L2 | Left / right mouse button (hold to swing the unlocked camera) |
| R1 / L1 | Zoom in / out |
| R3 | Lock or unlock the camera (C) |
| Options | Pause (Escape) |

## Build

Linux (WSL 2 works). The toolchain is PS5_Vulkan's: RADV statically linked into the title,
presenting through VideoOut.

1. Clone, side by side: [PS5_Vulkan](https://github.com/mihawk-99/PS5_Vulkan),
   [PS5_Mesa](https://github.com/mihawk-99/PS5_Mesa) and a payload SDK at the title pin
   (PS5_Ghostship's `ps5/tools/setup-sdk.sh` fetches and builds `611893f`).
2. Bootstrap PS5_Vulkan: `tools/setup-native-dependencies.sh`, `tools/rebuild-libc.sh`, and
   `PS5_MESA_FORK=<PS5_Mesa> tools/build-radv.sh release`.
3. Install the Rust toolchain this repository pins (`rust-toolchain.toml`) with the `rust-src`
   component, and `llvm-ar` and `clang` 18.
4. Build:

   ```bash
   PS5_VULKAN=~/ps5/PS5_Vulkan PS5_PAYLOAD_SDK=<sdk> ps5/tools/build.sh
   ```

   The title folder lands in `dist/PPSA99320/`. `PROFILE=ship` builds with fat LTO.

### How the port is put together

- `crates/survivors-ps5`: the game as a static library with one C entry point,
  `survivors_ps5_main`, which runs `benilla_app::run_survivors`.
- `ps5/x86_64-ps5-freebsd.json`: the Rust target. It is FreeBSD 11's libc layout with
  `env = "ps5"`, no thread locals, aborting on panic, and no red zone. std is built from source
  (`-Z build-std`) with one patch (`tools/prepare-rust-std.py`).
- `third_party/wgpu-hal`: wgpu-hal 27.0.4 with a PS5 Vulkan path (`BENILLA.md` there). The
  instance comes from RADV's `vk_icdGetInstanceProcAddr`. A `RawWindowHandle::Drm` window is a
  VK_KHR_display plane surface on the console's display.
- `crates/benilla-world/src/ps5_display.rs`: the primary window and the main loop that winit
  provides elsewhere. The window gets a DRM handle, the display mode's size and a UI scale that
  keeps the layout at 1080 logical lines.
- `crates/benilla-app/src/ps5`: the pad, keyboard and mouse, written as the bevy messages winit
  writes.
- `crates/benilla-app/src/sound/output/ps5.rs`: the mixer's output device, libSceAudioOut at
  48 kHz stereo.
- `ps5/src`:
  - `main.c`: start-up. It sends standard output and error to klog, dismisses the splash, reads
    `env.txt`, sets `WOW_DATA` and `BENILLA_HOME`, and runs the game on a 64 MB stack.
  - `platform.c` and `shims.c`: from Ghostship (MIT).
  - `rust-compat.c`: the libc names std expects that the console exports under other names.
- `ps5/tools/link-title.sh`: Ghostship's link. It links RADV, the CRT and the console's
  libraries, refuses null imports, signs `eboot.bin` and lays out the folder.

## Credits

The port stands on these projects. Their licences are listed in [LICENSING.md](LICENSING.md).

- [Azeroth Survivors](https://wow.aethus.pl/) by Aethus: the game.
- [OutThisLife/azeroth-survivors-re](https://github.com/OutThisLife/azeroth-survivors-re): the
  local build this fork is based on. It runs on [benilla](https://github.com/samwhosung/benilla)
  and [wenilla](https://github.com/Arnesen/wenilla).
- [mihawk-99/PS5_Vulkan](https://github.com/mihawk-99/PS5_Vulkan) and
  [mihawk-99/PS5_Mesa](https://github.com/mihawk-99/PS5_Mesa) by Mihawk: Vulkan (RADV) on the
  console, the title start-up, `libc.prx`, and the converter that writes `eboot.bin`.
- [mihawk-99/PS5_PayloadSDK](https://github.com/mihawk-99/PS5_PayloadSDK), a fork of
  [ps5-payload-dev/sdk](https://github.com/ps5-payload-dev/sdk): the toolchain, CRT and the
  console's library stubs.
- [griches/PS5_Ghostship](https://github.com/griches/PS5_Ghostship): the title layout, the
  platform layer (pad, mouse, keyboard, sound) and the link this port follows.
- [Bevy](https://bevy.org) and [wgpu](https://github.com/gfx-rs/wgpu): the engine and renderer.
- [drakmor/ShadowMountPlus](https://github.com/drakmor/ShadowMountPlus): registers the title on
  the console.

## Licence

The source in this repository is MIT OR Apache-2.0, except
`ps5/tools/png-to-title-dds.py`, which is GPL-3.0-or-later. A built title links GPL-3.0 code
from PS5_Vulkan and the payload SDK, so `prebuilt/PPSA99320.zip` and any `eboot.bin` you build
are distributed under GPL-3.0. [LICENSING.md](LICENSING.md) explains which parts come from
where.
