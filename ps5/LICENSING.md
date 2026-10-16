# Licences: the PlayStation 5 title

This repository's own code is offered under MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE` at
the top of the repository). The PS5 port adds files under the licences below. A **built
`eboot.bin`** also links code taken from other projects. Some of that code is GPL-3.0, so a title
as distributed (`prebuilt/PPSA99320.zip` and any `dist/PPSA99320/`) is offered under the
**GNU General Public License, version 3** ([`licenses/GPL-3.0.txt`](licenses/GPL-3.0.txt)).
MIT and Apache-2.0 code may be combined into a GPL-3.0 work. The source files themselves keep
their own licences and may still be used under them alone.

This is a reading of the licences, not legal advice.

## Files in this repository

| Path | Licence | From |
| --- | --- | --- |
| `crates/`, except `third_party/` | MIT OR Apache-2.0 | this repository |
| `third_party/wgpu-hal/` | MIT OR Apache-2.0 (its `LICENSE.MIT`, `LICENSE.APACHE`) | [wgpu](https://github.com/gfx-rs/wgpu) 27.0.4, with a PS5 path added (`BENILLA.md`) |
| `ps5/src/platform.c`, `ps5/src/platform.h` | MIT | the PS5 Vulkan Template by Mihawk, by way of [PS5_Ghostship](https://github.com/griches/PS5_Ghostship) |
| `ps5/src/shims.c`, `ps5/tools/link-title.sh` | MIT | adapted from [PS5_Ghostship](https://github.com/griches/PS5_Ghostship) |
| `ps5/tools/png-to-title-dds.py` | GPL-3.0-or-later | the BC7 encoder from PS5 RetroArch by Mihawk, by way of PS5_Ghostship |
| The rest of `ps5/` (start-up, compat shims, build scripts, the Rust target, title art) | MIT | written for this port |

## Built from elsewhere

None of the following is stored in this repository. The build fetches each one from its own
project, under that project's licence. The revisions listed are the ones the prebuilt title was
built from.

| Part | Project | Revision | Licence |
| --- | --- | --- | --- |
| The Vulkan driver (RADV, Mesa's common Vulkan runtime and the VideoOut window system) | [mihawk-99/PS5_Mesa](https://github.com/mihawk-99/PS5_Mesa) | `1ba9874f478` | MIT, with other licences on individual files ([`licenses/Mesa-license.rst`](licenses/Mesa-license.rst), [`licenses/Mesa-MIT.txt`](licenses/Mesa-MIT.txt)) |
| The driver's link recipe, the title start-up (`app_crt`), `sce_module/libc.prx`, and the ELF converter and fake-SELF writer | [mihawk-99/PS5_Vulkan](https://github.com/mihawk-99/PS5_Vulkan) | `f410e67` | GPL-3.0-or-later |
| The CRT, the console libraries' link stubs and headers | [mihawk-99/PS5_PayloadSDK](https://github.com/mihawk-99/PS5_PayloadSDK), a fork of [ps5-payload-dev/sdk](https://github.com/ps5-payload-dev/sdk) | `611893f` | GPL-3.0-or-later |
| The title layout and link this port follows | [griches/PS5_Ghostship](https://github.com/griches/PS5_Ghostship) | `4cb586e` | GPL-3.0 (the repository); MIT (the files taken) |
| Rust's standard library, built from source | [rust-lang/rust](https://github.com/rust-lang/rust) | the pinned toolchain | MIT OR Apache-2.0 |
| The game's crates (Bevy, wgpu, mlua with Lua 5.1, and the rest of `Cargo.lock`) | crates.io | `Cargo.lock` | MIT, Apache-2.0, Zlib, BSD and similar; each crate's own |

## The corresponding source

The source for a prebuilt title is the following:
- This repository, at the commit that added that `prebuilt/PPSA99320.zip`.
- The projects above, at the revisions listed.

[`README.md`](README.md) describes how to build them into the same folder.

## No game data

The title contains no Blizzard data. The player supplies the game's `Data` folder (MPQs).
