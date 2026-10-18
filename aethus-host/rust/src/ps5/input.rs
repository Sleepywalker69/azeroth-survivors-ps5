//! The console's pad, mouse and keyboard (`ps5/src/platform.h` in azeroth-survivors-re) as the
//! DOM events a browser would fire, with the pad standing in for a keyboard and mouse:
//!
//! | Pad | Page sees |
//! | --- | --- |
//! | Left stick | W A S D |
//! | Right stick | the mouse (the cursor; while the page holds the pointer, motion alone) |
//! | Cross | left button |
//! | Circle | Space |
//! | Square | F |
//! | Triangle | R |
//! | D-pad left, up, right | 1, 2, 3 |
//! | D-pad down | Enter |
//! | Options | Escape |
//! | R2, L2 | left, right button |
//! | R1, L1 | the wheel, a notch up or down a press |
//! | R3 | toggle right stick between pointer and camera (latched right mouse button) |
//! | L3 + Options (hold 2 seconds) | check for updates, or install a staged update and close |
//!
//! Keys are their USB HID usage throughout (the keyboard's own numbering; the modifier keys are
//! 0xe0..=0xe7), so the pad and the keyboard holding the same key press it once.

use serde_json::{json, Value};

const PAD_R3: u32 = 0x000004;
const PAD_L3: u32 = 0x000002;
const PAD_OPTIONS: u32 = 0x000008;
const PAD_UP: u32 = 0x000010;
const PAD_RIGHT: u32 = 0x000020;
const PAD_DOWN: u32 = 0x000040;
const PAD_LEFT: u32 = 0x000080;
const PAD_L1: u32 = 0x000400;
const PAD_R1: u32 = 0x000800;
const PAD_TRIANGLE: u32 = 0x001000;
const PAD_CIRCLE: u32 = 0x002000;
const PAD_CROSS: u32 = 0x004000;
const PAD_SQUARE: u32 = 0x008000;

#[repr(C)]
#[derive(Default)]
struct Pad {
    held: u32,
    pressed: u32,
    left_x: f32,
    left_y: f32,
    right_x: f32,
    right_y: f32,
    l2: f32,
    r2: f32,
}

#[repr(C)]
#[derive(Default)]
struct Mouse {
    dx: i32,
    dy: i32,
    wheel: i32,
    buttons: u32,
    connected: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct KeyboardEvent {
    usage: u8,
    down: bool,
}

#[repr(C)]
struct Keyboard {
    connected: bool,
    modifiers: u8,
    event_count: i32,
    events: [KeyboardEvent; 64],
}

extern "C" {
    fn pad_open() -> bool;
    fn pad_poll(pad: *mut Pad);
    fn mouse_open() -> bool;
    fn mouse_poll(mouse: *mut Mouse) -> bool;
    fn keyboard_open() -> bool;
    fn keyboard_poll(keyboard: *mut Keyboard) -> bool;
}

/// Below this a stick is at rest for the keys it stands in for.
const STICK_KEY: f32 = 0.35;
/// The cursor's top speed under the right stick, CSS pixels a second.
const CURSOR_SPEED: f64 = 1400.0;
/// Below this a trigger is up.
const TRIGGER: f32 = 0.4;

const KEY_W: u8 = 0x1a;
const KEY_A: u8 = 0x04;
const KEY_S: u8 = 0x16;
const KEY_D: u8 = 0x07;
const KEY_F: u8 = 0x09;
const KEY_R: u8 = 0x15;
const DIGIT_1: u8 = 0x1e;
const DIGIT_2: u8 = 0x1f;
const DIGIT_3: u8 = 0x20;
const ENTER: u8 = 0x28;
const ESCAPE: u8 = 0x29;
const SPACE: u8 = 0x2c;
/// The modifier keys, in the order of their bits in `Keyboard::modifiers`.
const MODIFIERS: [u8; 8] = [0xe0, 0xe1, 0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7];

/// A key as KeyboardEvent names it.
struct KeyName {
    code: String,
    key: String,
    shifted: String,
    location: u8,
}

/// KeyboardEvent's `code`, `key` (plain and shifted) and `location` for each HID usage, as on a
/// US keyboard.
fn key_names() -> Vec<Option<KeyName>> {
    let mut table: Vec<Option<KeyName>> = (0..256).map(|_| None).collect();
    let mut put = |usage: u8, code: &str, key: &str, shifted: &str, location: u8| {
        table[usize::from(usage)] = Some(KeyName { code: code.into(), key: key.into(), shifted: shifted.into(), location });
    };
    for (i, c) in ('a'..='z').enumerate() {
        let upper = c.to_ascii_uppercase().to_string();
        put(0x04 + i as u8, &format!("Key{upper}"), &c.to_string(), &upper, 0);
    }
    for (i, (d, s)) in "1234567890".chars().zip("!@#$%^&*()".chars()).enumerate() {
        put(0x1e + i as u8, &format!("Digit{d}"), &d.to_string(), &s.to_string(), 0);
    }
    for (usage, code, key, shifted) in [
        (0x2c, "Space", " ", " "),
        (0x2d, "Minus", "-", "_"),
        (0x2e, "Equal", "=", "+"),
        (0x2f, "BracketLeft", "[", "{"),
        (0x30, "BracketRight", "]", "}"),
        (0x31, "Backslash", "\\", "|"),
        (0x33, "Semicolon", ";", ":"),
        (0x34, "Quote", "'", "\""),
        (0x35, "Backquote", "`", "~"),
        (0x36, "Comma", ",", "<"),
        (0x37, "Period", ".", ">"),
        (0x38, "Slash", "/", "?"),
    ] {
        put(usage, code, key, shifted, 0);
    }
    for (usage, name) in [
        (0x28, "Enter"),
        (0x29, "Escape"),
        (0x2a, "Backspace"),
        (0x2b, "Tab"),
        (0x39, "CapsLock"),
        (0x49, "Insert"),
        (0x4a, "Home"),
        (0x4b, "PageUp"),
        (0x4c, "Delete"),
        (0x4d, "End"),
        (0x4e, "PageDown"),
        (0x4f, "ArrowRight"),
        (0x50, "ArrowLeft"),
        (0x51, "ArrowDown"),
        (0x52, "ArrowUp"),
    ] {
        put(usage, name, name, name, 0);
    }
    for i in 0..12u8 {
        let name = format!("F{}", i + 1);
        put(0x3a + i, &name, &name, &name, 0);
    }
    put(0x58, "NumpadEnter", "Enter", "Enter", 3);
    for (i, (code, key)) in [("ControlLeft", "Control"), ("ShiftLeft", "Shift"), ("AltLeft", "Alt"), ("MetaLeft", "Meta")].into_iter().enumerate() {
        put(0xe0 + i as u8, code, key, key, 1);
        put(0xe4 + i as u8, &code.replace("Left", "Right"), key, key, 2);
    }
    table
}

/// The pad's and the devices' state between polls.
pub struct Input {
    names: Vec<Option<KeyName>>,
    /// Keys each device holds down; a key is down for the page while either holds it.
    pad_keys: Vec<u8>,
    keyboard_keys: Vec<u8>,
    /// The page's mouse buttons down (0 left, 1 middle, 2 right, 3 back, 4 forward).
    buttons: Vec<u8>,
    /// The cursor, CSS pixels, and the page's size it stays within.
    cursor: (f64, f64),
    viewport: (f64, f64),
    /// The keyboard's modifier keys (its HID bits).
    modifiers: u8,
    last_poll: Option<f64>,
    camera_mode: bool,
    pointer_stash: Option<(f64, f64)>,
    previous_pad: u32,
    update_hold: Option<f64>,
    update_latched: bool,
    update_action: bool,
}

impl Input {
    /// Opens the first user's pad, mouse and keyboard; any may be missing, and a mouse or
    /// keyboard plugged in later is picked up by the poll.
    pub fn open(viewport: (f64, f64)) -> Input {
        // SAFETY: plain calls into the platform layer, before the first poll.
        let (pad, mouse, keyboard) = unsafe { (pad_open(), mouse_open(), keyboard_open()) };
        crate::log::say!("input: pad {pad}, mouse {mouse}, keyboard {keyboard}");
        Self::new(viewport)
    }

    fn new(viewport: (f64, f64)) -> Input {
        Input {
            names: key_names(),
            pad_keys: Vec::new(),
            keyboard_keys: Vec::new(),
            buttons: Vec::new(),
            cursor: (viewport.0 / 2.0, viewport.1 / 2.0),
            viewport,
            modifiers: 0,
            last_poll: None,
            camera_mode: false,
            pointer_stash: None,
            previous_pad: 0,
            update_hold: None,
            update_latched: false,
            update_action: false,
        }
    }

    /// The cursor, CSS pixels.
    pub fn cursor(&self) -> (f64, f64) {
        self.cursor
    }

    pub fn camera_mode(&self) -> bool { self.camera_mode }

    pub fn take_update_action(&mut self) -> bool { std::mem::take(&mut self.update_action) }

    /// The DOM's modifier bits: shift 1, ctrl 2, alt 4, meta 8.
    fn m(&self) -> u32 {
        let any = |left: u8, right: u8| self.modifiers & (left | right) != 0;
        u32::from(any(0x02, 0x20)) | u32::from(any(0x01, 0x10)) << 1 | u32::from(any(0x04, 0x40)) << 2 | u32::from(any(0x08, 0x80)) << 3
    }

    fn key_event(&self, usage: u8, down: bool) -> Option<Value> {
        let name = self.names[usize::from(usage)].as_ref()?;
        let shift = self.modifiers & (0x02 | 0x20) != 0;
        Some(json!({
            "t": "key",
            "down": down,
            "key": if shift { &name.shifted } else { &name.key },
            "code": name.code,
            "location": name.location,
            "repeat": false,
            "m": self.m(),
        }))
    }

    /// Reads every device and appends the events since the last poll. `locked`: the page holds
    /// the pointer, so the mouse's motion is sent without a position and the cursor stays put.
    pub fn poll(&mut self, now: f64, locked: bool, events: &mut Vec<Value>) {
        let mut pad = Pad::default();
        let mut mouse = Mouse::default();
        let mut keyboard = Keyboard { connected: false, modifiers: 0, event_count: 0, events: [KeyboardEvent::default(); 64] };
        // SAFETY: each poll fills the struct it is handed, laid out as platform.h declares it.
        let (mouse_live, keyboard_live) = unsafe {
            pad_poll(&mut pad);
            (mouse_poll(&mut mouse), keyboard_poll(&mut keyboard))
        };
        self.poll_devices(now, locked, &pad, &mouse, mouse_live, &keyboard, keyboard_live, events);
    }

    fn poll_devices(&mut self, now: f64, locked: bool, pad: &Pad, mouse: &Mouse, mouse_live: bool,
                    keyboard: &Keyboard, keyboard_live: bool, events: &mut Vec<Value>) {
        let dt = self.last_poll.map_or(0.0, |last| ((now - last) / 1000.0).clamp(0.0, 0.1));
        self.last_poll = Some(now);

        // platform.c calculates `pressed` from the previous contents of its output struct.
        // This host uses a fresh Pad for each poll, so derive edges from retained held bits.
        let pressed = pad.held & !self.previous_pad;
        self.previous_pad = pad.held;
        let update_combo = pad.held & (PAD_L3 | PAD_OPTIONS) == (PAD_L3 | PAD_OPTIONS);
        if update_combo {
            let began = *self.update_hold.get_or_insert(now);
            if !self.update_latched && now - began >= 2000.0 {
                self.update_action = true;
                self.update_latched = true;
            }
        } else {
            self.update_hold = None;
            self.update_latched = false;
        }
        let toggle = pressed & PAD_R3 != 0;
        let escape = !update_combo && pressed & PAD_OPTIONS != 0;
        if toggle || (escape && self.camera_mode) {
            self.camera_mode = !escape && !self.camera_mode;
            if self.camera_mode {
                self.pointer_stash = Some(self.cursor);
                // Begin a world drag away from the UI at the saved pointer position.
                self.cursor = (self.viewport.0 / 2.0, self.viewport.1 / 2.0);
            } else if let Some(cursor) = self.pointer_stash.take() {
                self.cursor = cursor;
            }
            events.push(json!({"t": "move", "x": self.cursor.0, "y": self.cursor.1, "dx": 0, "dy": 0, "m": self.m()}));
            // Pointer mode must also restore the DOM pointer position after the game releases
            // its grab; a move dispatched while locked is deliberately ignored by the DOM.
            if !self.camera_mode {
                events.push(json!({"t": "pointer_mode", "x": self.cursor.0, "y": self.cursor.1}));
            }
            crate::log::say!("input: right stick {} mode", if self.camera_mode { "camera" } else { "pointer" });
        }

        // The keyboard first: the pad does not release a key it holds.
        let mut keyboard_keys = std::mem::take(&mut self.keyboard_keys);
        if keyboard_live {
            for (bit, &usage) in MODIFIERS.iter().enumerate() {
                let down = keyboard.modifiers & (1 << bit) != 0;
                if down != keyboard_keys.contains(&usage) {
                    self.modifiers = if down { self.modifiers | 1 << bit } else { self.modifiers & !(1 << bit) };
                    if down {
                        keyboard_keys.push(usage);
                    } else {
                        keyboard_keys.retain(|k| *k != usage);
                    }
                    events.extend(self.key_event(usage, down));
                }
            }
            let count = keyboard.event_count.clamp(0, 64) as usize;
            for ev in &keyboard.events[..count] {
                if self.names[usize::from(ev.usage)].is_none() || ev.down == keyboard_keys.contains(&ev.usage) {
                    continue;
                }
                if ev.down {
                    keyboard_keys.push(ev.usage);
                } else {
                    keyboard_keys.retain(|k| *k != ev.usage);
                }
                if !self.pad_keys.contains(&ev.usage) {
                    events.extend(self.key_event(ev.usage, ev.down));
                }
            }
        } else {
            // Unplugged with keys down: let them all go.
            self.modifiers = 0;
            for usage in keyboard_keys.drain(..) {
                if !self.pad_keys.contains(&usage) {
                    events.extend(self.key_event(usage, false));
                }
            }
        }
        self.keyboard_keys = keyboard_keys;

        let held = |bit: u32| pad.held & bit != 0;
        let mut pad_keys = Vec::new();
        for (on, usage) in [
            (pad.left_y < -STICK_KEY, KEY_W),
            (pad.left_y > STICK_KEY, KEY_S),
            (pad.left_x < -STICK_KEY, KEY_A),
            (pad.left_x > STICK_KEY, KEY_D),
            (held(PAD_CIRCLE), SPACE),
            (held(PAD_SQUARE), KEY_F),
            (held(PAD_TRIANGLE), KEY_R),
            (held(PAD_LEFT), DIGIT_1),
            (held(PAD_UP), DIGIT_2),
            (held(PAD_RIGHT), DIGIT_3),
            (held(PAD_DOWN), ENTER),
            (held(PAD_OPTIONS) && !update_combo, ESCAPE),
        ] {
            if on {
                pad_keys.push(usage);
            }
        }
        for &usage in &self.pad_keys {
            if !pad_keys.contains(&usage) && !self.keyboard_keys.contains(&usage) {
                events.extend(self.key_event(usage, false));
            }
        }
        for &usage in &pad_keys {
            if !self.pad_keys.contains(&usage) && !self.keyboard_keys.contains(&usage) {
                events.extend(self.key_event(usage, true));
            }
        }
        self.pad_keys = pad_keys;

        // Motion: the mouse's counts and the right stick's speed, curved so a small tilt aims
        // finely and full tilt crosses the screen in about a second.
        let (sx, sy) = (f64::from(pad.right_x), f64::from(pad.right_y));
        let tilt = (sx * sx + sy * sy).sqrt();
        let (mut dx, mut dy) = (sx * tilt * CURSOR_SPEED * dt, sy * tilt * CURSOR_SPEED * dt);
        if mouse_live {
            dx += f64::from(mouse.dx);
            dy += f64::from(mouse.dy);
        }
        let m = self.m();
        if dx != 0.0 || dy != 0.0 {
            if locked || self.camera_mode {
                events.push(json!({ "t": "move", "x": self.cursor.0, "y": self.cursor.1, "dx": dx, "dy": dy, "m": m }));
            } else {
                let (x, y) = (
                    (self.cursor.0 + dx).clamp(0.0, self.viewport.0 - 1.0),
                    (self.cursor.1 + dy).clamp(0.0, self.viewport.1 - 1.0),
                );
                let (mx, my) = (x - self.cursor.0, y - self.cursor.1);
                self.cursor = (x, y);
                events.push(json!({ "t": "move", "x": x, "y": y, "dx": mx, "dy": my, "m": m }));
            }
        }

        // Buttons: the pad's and the mouse's, each released when neither holds it.
        let mut buttons = Vec::new();
        if held(PAD_CROSS) || pad.r2 > TRIGGER {
            buttons.push(0);
        }
        if self.camera_mode || pad.l2 > TRIGGER {
            buttons.push(2);
        }
        if mouse_live {
            for (bit, b) in [(0x01, 0), (0x04, 1), (0x02, 2), (0x08, 3), (0x10, 4)] {
                if mouse.buttons & bit != 0 && !buttons.contains(&b) {
                    buttons.push(b);
                }
            }
        }
        let (x, y) = self.cursor;
        for &b in &self.buttons {
            if !buttons.contains(&b) {
                events.push(json!({ "t": "up", "b": b, "x": x, "y": y, "m": m }));
            }
        }
        for &b in &buttons {
            if !self.buttons.contains(&b) {
                events.push(json!({ "t": "down", "b": b, "x": x, "y": y, "m": m }));
            }
        }
        self.buttons = buttons;

        // The wheel, in lines: the DOM's deltas point down, so a notch up (R1) is -1.
        let mut notches = if mouse_live { mouse.wheel } else { 0 };
        if pressed & PAD_R1 != 0 {
            notches += 1;
        }
        if pressed & PAD_L1 != 0 {
            notches -= 1;
        }
        if notches != 0 {
            events.push(json!({ "t": "wheel", "dx": 0, "dy": -notches, "mode": 1, "m": m }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn step(input: &mut Input, now: f64, pad: Pad) -> Vec<Value> {
        let mut events = Vec::new();
        let keyboard = Keyboard { connected: false, modifiers: 0, event_count: 0, events: [KeyboardEvent::default(); 64] };
        input.poll_devices(now, false, &pad, &Mouse::default(), false, &keyboard, false, &mut events);
        events
    }
    #[test]
    fn r3_latches_camera_releases_it_and_restores_pointer() {
        let mut input = Input::new((1920.0, 1080.0));
        input.cursor = (100.0, 200.0);
        let events = step(&mut input, 0.0, Pad { held: PAD_R3, pressed: PAD_R3, ..Default::default() });
        assert!(input.camera_mode());
        assert!(events.iter().any(|e| e["t"] == "down" && e["b"] == 2));
        assert!(!events.iter().any(|e| e["code"] == "KeyC"));
        let events = step(&mut input, 16.0, Pad { held: PAD_R3, pressed: PAD_R3, right_x: 1.0, ..Default::default() });
        assert!(input.camera_mode()); // holding R3 does not repeatedly toggle
        assert_eq!(input.cursor(), (960.0, 540.0));
        assert!(events.iter().any(|e| e["t"] == "move" && e["dx"].as_f64().unwrap_or(0.0) > 0.0));
        step(&mut input, 32.0, Pad::default());
        let events = step(&mut input, 48.0, Pad { held: PAD_R3, pressed: PAD_R3, ..Default::default() });
        assert!(!input.camera_mode());
        assert_eq!(input.cursor(), (100.0, 200.0));
        assert!(events.iter().any(|e| e["t"] == "up" && e["b"] == 2));
        assert!(events.iter().any(|e| e["t"] == "pointer_mode"));
    }
    #[test]
    fn options_returns_to_pointer_without_releasing_a_physical_trigger() {
        let mut input = Input::new((1920.0, 1080.0));
        step(&mut input, 0.0, Pad { held: PAD_R3, pressed: PAD_R3, ..Default::default() });
        let events = step(&mut input, 16.0, Pad { pressed: PAD_OPTIONS, held: PAD_OPTIONS, l2: 1.0, ..Default::default() });
        assert!(!input.camera_mode());
        assert!(!events.iter().any(|e| e["t"] == "up" && e["b"] == 2));
        let events = step(&mut input, 32.0, Pad::default());
        assert!(events.iter().any(|e| e["t"] == "up" && e["b"] == 2));
    }
    #[test]
    fn update_chord_requires_two_seconds_and_release_before_repeating() {
        let mut input = Input::new((1920.0, 1080.0));
        for now in [0.0, 1000.0, 1999.0] {
            let events = step(&mut input, now, Pad { held: PAD_L3 | PAD_OPTIONS, ..Default::default() });
            assert!(!input.take_update_action());
            assert!(!events.iter().any(|e| e["code"] == "Escape"));
        }
        step(&mut input, 2000.0, Pad { held: PAD_L3 | PAD_OPTIONS, ..Default::default() });
        assert!(input.take_update_action());
        step(&mut input, 5000.0, Pad { held: PAD_L3 | PAD_OPTIONS, ..Default::default() });
        assert!(!input.take_update_action());
        step(&mut input, 5016.0, Pad::default());
        step(&mut input, 5032.0, Pad { held: PAD_L3 | PAD_OPTIONS, ..Default::default() });
        step(&mut input, 7032.0, Pad { held: PAD_L3 | PAD_OPTIONS, ..Default::default() });
        assert!(input.take_update_action());
    }
}
