//! The PlayStation 5 title's input: the DualSense, and a USB or Bluetooth keyboard and mouse,
//! read through the title's platform layer (`ps5/src/platform.c`) and written as the same bevy
//! messages winit writes on a computer, so every system downstream (the bindings, picking, the
//! survivors controls) reads them unchanged.
//!
//! The window, the screen and the main loop are `benilla_world::ps5_display`'s. With no window
//! system to draw it, the cursor is drawn by the game ([`cursor`]).
//!
//! The pad drives a virtual mouse and the keys the survivors mode reads:
//!
//! | Pad                | As                                   |
//! |--------------------|--------------------------------------|
//! | left stick         | W A S D                              |
//! | right stick        | the cursor (and mouse motion)        |
//! | cross              | left click and Enter                 |
//! | circle             | Space (jump)                         |
//! | square             | F (mobility)                         |
//! | triangle           | G                                    |
//! | d-pad left/up/right| 1 / 2 / 3 (cards)                    |
//! | d-pad down         | Enter                                |
//! | R2 / L2            | left / right mouse button (held)     |
//! | R1 / L1            | wheel in / out (zoom)                |
//! | R3                 | C (camera lock)                      |
//! | options            | Escape (pause)                       |

mod cursor;
mod input;

use bevy::prelude::*;

/// Reads the console's controllers each frame.
pub(crate) struct Ps5InputPlugin;

impl Plugin for Ps5InputPlugin {
    fn build(&self, app: &mut App) {
        input::open();
        app.init_resource::<input::PadState>()
            .add_systems(
                PreUpdate,
                input::poll.before(bevy::input::InputSystems),
            )
            .add_systems(Startup, cursor::spawn)
            .add_systems(PostUpdate, cursor::follow.before(bevy::ui::UiSystems::Layout));
    }
}
