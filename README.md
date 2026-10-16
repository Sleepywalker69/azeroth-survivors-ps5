<img width="1920" height="1080" alt="brag" src="https://github.com/user-attachments/assets/fa574bb6-7bd4-4e78-8b72-14c58e3c716e" />

<div align="center">

# Azeroth Survivors

**A free fan-made WoW roguelike by [Aethus](https://wow.aethus.pl/), run locally on a
from-scratch WoW 1.12.1 client engine (Rust + [Bevy](https://bevy.org)).**

[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-blue.svg)](LICENSE-MIT)

</div>

> **Not an original project.** *Azeroth Survivors* is a free, fan-made WoW roguelike by
> **Aethus** — play the original in the browser at **[wow.aethus.pl](https://wow.aethus.pl/)**
> ([r/wowservers announcement](https://www.reddit.com/r/wowservers/comments/1x0yidl/azeroth_survivors_roguelike_wow_singleplayer_game/)
> · [Discord](https://discord.gg/UzxaZuSAk)). This repository is a from-source local build of the
> game, running on the [benilla](#how-its-built) / [wenilla](#how-its-built) engine, for personal use.

Pick a class and a battleground, then survive: WASD to move, your abilities cast themselves,
enemies swarm in waves, you level up and pick upgrades, and a boss shows up every couple of
minutes. **No server, no login, no account** — fully offline and self-contained.

<table>
  <tr>
    <td width="50%"><img src="docs/screenshots/menu.jpg" alt="The pick screen: race, gender and class on the left, sixteen zones on the right, the hero in between"></td>
    <td width="50%"><img src="docs/screenshots/combat.jpg" alt="A Mage holding off Elwynn Forest's wolves with frost, fire and arcane"></td>
  </tr>
  <tr>
    <td><img src="docs/screenshots/level-up.jpg" alt="Level up: three spell cards, coloured by rarity, each showing exactly what its next rank changes"></td>
    <td><img src="docs/screenshots/results.jpg" alt="The results screen after a death in Redridge Mountains"></td>
  </tr>
</table>

> **New in this fork: a PlayStation 5 version.** It is a home-screen title for jailbroken
> consoles. See [PlayStation 5](#playstation-5) for the download, install steps, controls and
> credits.

### In this build

- **A character-create pick screen** built from the client's own glue art: eight races, both
  genders, every class the race can be, and **sixteen zones**, from the starting zones to
  Naxxramas and the Gates of Ahn'Qiraj. Each zone fights with its own creatures and bosses, taken
  from the zone's real spawns. Your pick is remembered across reloads.
- **Level-up cards** that deal in, tilt toward the cursor and lift on hover. Their WoW item-quality
  colour says how strong a pick is, and each card lists exactly what changes, like
  `Damage 20 » 26` or `Targets 2 » 3`.
- **A class movement spell on F** from level 5, aimed at the cursor: Blink, Charge, Sprint, Dash,
  Fade, Ghost Wolf, Aspect of the Cheetah, Blessing of Freedom, Howl of Terror.
- **A camera you can free:** locked top-down by default with the hero facing the cursor. **C**
  unlocks it to swing and tilt, and the wheel zooms from over the shoulder out to 70 yards.
- A light sharpen-and-colour pass over the picture (**G** toggles it).

## Play it

```bash
git clone https://github.com/OutThisLife/azeroth-survivors-re
cd azeroth-survivors-re
make build     # fetch game data + install toolchain + build the wasm (~5.5 GB download)
make start     # → open http://127.0.0.1:8090/
```

`make build` chains three re-runnable scripts, each skipping what's already done:

| Step | What it does |
| --- | --- |
| `scripts/fetch-data.sh` | downloads the WoW 1.12.1 client and extracts its MPQs |
| `scripts/web-setup.sh` | installs the wasm toolchain (wasi-sdk, wasm-bindgen, binaryen) |
| `scripts/web-build.sh` | builds the client into `web/dist/` |

**Prerequisites:** [Rust](https://rustup.rs/), `make`, `curl`, `unzip`, and a bash shell. On Windows, run everything from Git Bash; PowerShell and cmd can't run the scripts.

Run `make` (or `make help`) to list every target.

## Controls

See [CONTROLS.md](CONTROLS.md). The short version: **WASD** to move, **Space** to jump, **F**
for your class's movement spell, the **mouse wheel** to zoom, **C** to unlock the camera, **Esc**
to pause, **1/2/3** to pick a level-up card.

## PlayStation 5

This fork adds a **home-screen title for a jailbroken PS5**. It is a real title with its own tile
and background, not a payload, and it is played with a DualSense. A USB or Bluetooth keyboard and
mouse also work.

> **Untested on hardware so far.** The title builds and links, but nobody has run it on a console
> yet. Reports are welcome in Issues.

| | |
| --- | --- |
| Download | [`ps5/prebuilt/PPSA99320.zip`](ps5/prebuilt/PPSA99320.zip) (46 MB) |
| Title ID | `PPSA99320` |
| Target firmware | 13.60 |
| Resolution | 3840x2160 by default; VideoOut scales it to your TV |

### What the console needs

- A jailbroken PS5 running a homebrew enabler, such as etaHEN or an equivalent. On 13.60 that is
  kstuff-lite.
- [ShadowMountPlus](https://github.com/drakmor/ShadowMountPlus), which shows the folders in
  `/data/homebrew` as titles on the home screen.
- An FTP server on the console to copy files with.

### Install

1. Download [`ps5/prebuilt/PPSA99320.zip`](ps5/prebuilt/PPSA99320.zip) and unzip it. It holds
   one folder, `PPSA99320`.
2. Copy that folder to the console as `/data/homebrew/PPSA99320/`. Include `sce_sys/icon0.png`
   in the first upload, because ShadowMountPlus will not register a title without its icon.
3. Copy the game's `Data` folder, holding the MPQs, to `/data/homebrew/PPSA99320/Data/`. You
   supply this folder; the zip contains no game data. The Windows release's own `Data` folder
   (`patch.MPQ`, `patch-2.MPQ`) works, and so does a WoW 1.12.1 client's.
4. Wait a few seconds for **Azeroth Survivors** to appear on the home screen, then start it.

If it does not appear, check `/data/shadowmount/debug.log`. Settings and logs are written to
`/data/homebrew/PPSA99320/userdata/`.

To change options, write `NAME=value` lines to `/data/homebrew/PPSA99320/env.txt`. For example,
`PS5_DISPLAY_MODE=1920x1080` renders at 1080p instead of 4K (`2560x1440` also works).

### Pad controls

| Pad | Does |
| --- | --- |
| Left stick | Move |
| Right stick | Move the cursor (aim, menus) |
| Cross | Click; start a run |
| Circle | Jump |
| Square | Mobility skill |
| D-pad left / up / right | Pick level-up card 1 / 2 / 3 |
| R2 / L2 | Left / right mouse button |
| R1 / L1 | Zoom in / out |
| R3 | Lock or unlock the camera |
| Options | Pause |

### Building it yourself

The title is built on Linux; WSL 2 works. The build needs a bootstrapped
[PS5_Vulkan](https://github.com/mihawk-99/PS5_Vulkan), the payload SDK and the pinned Rust
toolchain. When those are set up, run `ps5/tools/build.sh` and the title lands in
`dist/PPSA99320/`. [ps5/README.md](ps5/README.md) has the full steps and explains how the port
works.

### PS5 credits

The PS5 port would not exist without these projects:

- [mihawk-99/PS5_Vulkan](https://github.com/mihawk-99/PS5_Vulkan) and
  [mihawk-99/PS5_Mesa](https://github.com/mihawk-99/PS5_Mesa) by **Mihawk** provide Vulkan
  (RADV) on the console. They also supply the title start-up, `libc.prx`, and the tool that writes
  `eboot.bin`.
- [mihawk-99/PS5_PayloadSDK](https://github.com/mihawk-99/PS5_PayloadSDK), a fork of
  [ps5-payload-dev/sdk](https://github.com/ps5-payload-dev/sdk), provides the toolchain, the C
  runtime and the console's library stubs.
- [griches/PS5_Ghostship](https://github.com/griches/PS5_Ghostship) provides the title layout,
  the pad, mouse, keyboard and sound layer, and the link this port follows.
- [drakmor/ShadowMountPlus](https://github.com/drakmor/ShadowMountPlus) registers the title on
  the console.
- [Bevy](https://bevy.org) and [wgpu](https://github.com/gfx-rs/wgpu) are the engine and the
  renderer. The port adds a PS5 path to wgpu-hal in `third_party/wgpu-hal`.

### PS5 licence

A built title links GPL-3.0 code from PS5_Vulkan and the payload SDK, so **the prebuilt zip, and
any `eboot.bin` you build, is distributed under GPL-3.0**. The source in this repository keeps
the MIT or Apache-2.0 licence described in [Legal](#legal). The one exception is
`ps5/tools/png-to-title-dds.py`, which is GPL-3.0. [ps5/LICENSING.md](ps5/LICENSING.md) lists
every upstream project, the revision the prebuilt title was built from, and its licence.

## The game data (not in this repo)

This repo ships **no Blizzard content** — no art, models, sounds, maps, MPQ contents, or
FrameXML. The game reads a WoW 1.12.1 client's `Data` folder at runtime, which you supply.

`make build` (via `scripts/fetch-data.sh`) downloads the **English WoW 1.12.1 (build 5875)**
client from [archive.org](https://archive.org/details/WoW_1.12.1-5875) (~5.5 GB) and extracts the
MPQs into `wow-data/Data/` (git-ignored, resumable, skipped when already present). Or point at a
client you already have with `--data` / `WOW_DATA`.

### Changing the download source

If the archive.org item moves or you have your own mirror, override the URL without editing
anything:

```bash
make build WOW_DATA_URL=https://your-mirror.example/client.zip
```

Or set it permanently by editing the single clearly-marked `DEFAULT_DATA_URL` line at the top of
`scripts/fetch-data.sh`. Either way the URL must be a direct link to a zip holding the client's
`Data/*.MPQ` files.

## How it's built

The survivors mode is `crates/benilla-app/src/survivors/`, running on two upstream projects:

- [benilla](https://github.com/samwhosung/benilla) — a from-scratch WoW 1.12.1 client.
- [wenilla](https://github.com/Arnesen/wenilla) — benilla compiled to WebAssembly, with a local
  host and realm service.

## Legal

*Azeroth Survivors* is a free fan game by **Aethus**, not affiliated with or endorsed by
Blizzard Entertainment, and it ships no Blizzard game files. The screenshots in `docs/screenshots/`
show World of Warcraft art rendered from a user-supplied client; that art is © Blizzard
Entertainment. World of Warcraft is a trademark of Blizzard Entertainment, Inc. The engine code in
this repository is licensed under
[MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option; the vendored components under
`third_party/` keep their own upstream licenses. The PS5 title as built is GPL-3.0; see
[PS5 licence](#ps5-licence).
