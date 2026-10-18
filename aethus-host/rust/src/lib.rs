//! The Aethus host: Azeroth Survivors as wow.aethus.pl serves it (the wenilla web build), run
//! natively. The build's wasm is translated to C with wasm2c and runs under QuickJS (`../c`)
//! with the scripts in `../js` standing in for the browser; this crate answers the platform
//! questions those scripts ask: the game data, input, WebGPU with wgpu, Web Audio with a mixer of
//! its own, storage and the player's files.

mod audio;
mod data;
mod ffi;
mod files;
mod gpu;
mod log;
mod ops;
mod storage;
#[cfg(any(target_env = "ps5", test))]
mod update;

#[cfg(windows)]
pub mod desktop;
#[cfg(target_env = "ps5")]
pub mod ps5;

#[cfg(all(test, not(target_env = "ps5")))]
#[path = "ps5/input.rs"]
#[allow(dead_code)]
mod ps5_input_tests;
