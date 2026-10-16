//! The poll: the platform layer's pad, mouse and keyboard, written as winit's messages.

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::{MouseButtonInput, MouseMotion, MouseScrollUnit, MouseWheel};
use bevy::input::ButtonState;
use bevy::math::DVec2;
use bevy::prelude::*;
use bevy::window::{CursorMoved, PrimaryWindow, WindowEvent};

// ---------------------------------------------------------------------------------------------
// The platform layer (`ps5/src/platform.h`)

const PAD_R3: u32 = 0x000004;
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

const MOUSE_LEFT: u32 = 0x01;
const MOUSE_RIGHT: u32 = 0x02;
const MOUSE_MIDDLE: u32 = 0x04;
const MOUSE_BUTTON4: u32 = 0x08;
const MOUSE_BUTTON5: u32 = 0x10;

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

unsafe extern "C" {
    fn pad_open() -> bool;
    fn pad_poll(pad: *mut Pad);
    fn mouse_open() -> bool;
    fn mouse_poll(mouse: *mut Mouse) -> bool;
    fn keyboard_open() -> bool;
    fn keyboard_poll(keyboard: *mut Keyboard) -> bool;
}

/// Opens the first user's pad, mouse and keyboard; each may be missing (a mouse or keyboard may
/// be plugged in later, and the poll picks it up).
pub(super) fn open() {
    // SAFETY: plain calls into the platform layer, on the main thread, before the first poll.
    let (pad, mouse, keyboard) = unsafe { (pad_open(), mouse_open(), keyboard_open()) };
    info!("ps5 input: pad {pad}, mouse {mouse}, keyboard {keyboard}");
}

// ---------------------------------------------------------------------------------------------
// The mapping

/// Below this a stick is at rest for the WASD it stands in for.
const STICK_KEY: f32 = 0.35;
/// The virtual cursor's top speed, logical pixels a second at full tilt.
const CURSOR_SPEED: f32 = 1400.0;
/// Below this a trigger is up.
const TRIGGER: f32 = 0.4;

/// The keys and buttons the pad holds down, a bit each, so a change writes one press or release.
#[derive(Resource, Default)]
pub(super) struct PadState {
    keys: Vec<KeyCode>,
    buttons: Vec<MouseButton>,
    /// The virtual cursor, physical pixels; `None` until placed in the middle of the screen.
    cursor: Option<DVec2>,
    /// The keyboard's keys down, so the pad does not release a key the keyboard holds.
    keyboard_held: Vec<KeyCode>,
}

/// The keys the pad holds now.
fn pad_keys(pad: &Pad) -> Vec<KeyCode> {
    let mut keys = Vec::new();
    let held = |bit| pad.held & bit != 0;
    if pad.left_y < -STICK_KEY {
        keys.push(KeyCode::KeyW);
    }
    if pad.left_y > STICK_KEY {
        keys.push(KeyCode::KeyS);
    }
    if pad.left_x < -STICK_KEY {
        keys.push(KeyCode::KeyA);
    }
    if pad.left_x > STICK_KEY {
        keys.push(KeyCode::KeyD);
    }
    for (bit, key) in [
        (PAD_CROSS, KeyCode::Enter),
        (PAD_DOWN, KeyCode::Enter),
        (PAD_CIRCLE, KeyCode::Space),
        (PAD_SQUARE, KeyCode::KeyF),
        (PAD_TRIANGLE, KeyCode::KeyG),
        (PAD_LEFT, KeyCode::Digit1),
        (PAD_UP, KeyCode::Digit2),
        (PAD_RIGHT, KeyCode::Digit3),
        (PAD_R3, KeyCode::KeyC),
        (PAD_OPTIONS, KeyCode::Escape),
    ] {
        if held(bit) && !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

fn pad_buttons(pad: &Pad) -> Vec<MouseButton> {
    let mut buttons = Vec::new();
    if pad.held & PAD_CROSS != 0 || pad.r2 > TRIGGER {
        buttons.push(MouseButton::Left);
    }
    if pad.l2 > TRIGGER {
        buttons.push(MouseButton::Right);
    }
    buttons
}

/// What a key reads as text and as a logical key: enough for the bindings and the edit boxes.
fn logical(key: KeyCode, shift: bool) -> (Key, Option<char>) {
    let letter = |c: char| {
        let c = if shift { c.to_ascii_uppercase() } else { c };
        (Key::Character(c.to_string().into()), Some(c))
    };
    let symbol = |plain: char, shifted: char| {
        let c = if shift { shifted } else { plain };
        (Key::Character(c.to_string().into()), Some(c))
    };
    use KeyCode as K;
    match key {
        K::KeyA => letter('a'),
        K::KeyB => letter('b'),
        K::KeyC => letter('c'),
        K::KeyD => letter('d'),
        K::KeyE => letter('e'),
        K::KeyF => letter('f'),
        K::KeyG => letter('g'),
        K::KeyH => letter('h'),
        K::KeyI => letter('i'),
        K::KeyJ => letter('j'),
        K::KeyK => letter('k'),
        K::KeyL => letter('l'),
        K::KeyM => letter('m'),
        K::KeyN => letter('n'),
        K::KeyO => letter('o'),
        K::KeyP => letter('p'),
        K::KeyQ => letter('q'),
        K::KeyR => letter('r'),
        K::KeyS => letter('s'),
        K::KeyT => letter('t'),
        K::KeyU => letter('u'),
        K::KeyV => letter('v'),
        K::KeyW => letter('w'),
        K::KeyX => letter('x'),
        K::KeyY => letter('y'),
        K::KeyZ => letter('z'),
        K::Digit1 => symbol('1', '!'),
        K::Digit2 => symbol('2', '@'),
        K::Digit3 => symbol('3', '#'),
        K::Digit4 => symbol('4', '$'),
        K::Digit5 => symbol('5', '%'),
        K::Digit6 => symbol('6', '^'),
        K::Digit7 => symbol('7', '&'),
        K::Digit8 => symbol('8', '*'),
        K::Digit9 => symbol('9', '('),
        K::Digit0 => symbol('0', ')'),
        K::Minus => symbol('-', '_'),
        K::Equal => symbol('=', '+'),
        K::BracketLeft => symbol('[', '{'),
        K::BracketRight => symbol(']', '}'),
        K::Backslash => symbol('\\', '|'),
        K::Semicolon => symbol(';', ':'),
        K::Quote => symbol('\'', '"'),
        K::Backquote => symbol('`', '~'),
        K::Comma => symbol(',', '<'),
        K::Period => symbol('.', '>'),
        K::Slash => symbol('/', '?'),
        K::Space => (Key::Space, Some(' ')),
        K::Enter => (Key::Enter, None),
        K::Escape => (Key::Escape, None),
        K::Backspace => (Key::Backspace, None),
        K::Tab => (Key::Tab, None),
        K::Delete => (Key::Delete, None),
        K::Home => (Key::Home, None),
        K::End => (Key::End, None),
        K::PageUp => (Key::PageUp, None),
        K::PageDown => (Key::PageDown, None),
        K::Insert => (Key::Insert, None),
        K::ArrowUp => (Key::ArrowUp, None),
        K::ArrowDown => (Key::ArrowDown, None),
        K::ArrowLeft => (Key::ArrowLeft, None),
        K::ArrowRight => (Key::ArrowRight, None),
        K::ShiftLeft | K::ShiftRight => (Key::Shift, None),
        K::ControlLeft | K::ControlRight => (Key::Control, None),
        K::AltLeft | K::AltRight => (Key::Alt, None),
        K::SuperLeft | K::SuperRight => (Key::Super, None),
        K::CapsLock => (Key::CapsLock, None),
        K::F1 => (Key::F1, None),
        K::F2 => (Key::F2, None),
        K::F3 => (Key::F3, None),
        K::F4 => (Key::F4, None),
        K::F5 => (Key::F5, None),
        K::F6 => (Key::F6, None),
        K::F7 => (Key::F7, None),
        K::F8 => (Key::F8, None),
        K::F9 => (Key::F9, None),
        K::F10 => (Key::F10, None),
        K::F11 => (Key::F11, None),
        K::F12 => (Key::F12, None),
        _ => (Key::Unidentified(bevy::input::keyboard::NativeKey::Unidentified), None),
    }
}

/// A USB HID keyboard usage as the key in that place on a US keyboard.
fn hid_key(usage: u8) -> Option<KeyCode> {
    use KeyCode as K;
    const LETTERS: [KeyCode; 26] = [
        K::KeyA, K::KeyB, K::KeyC, K::KeyD, K::KeyE, K::KeyF, K::KeyG, K::KeyH, K::KeyI,
        K::KeyJ, K::KeyK, K::KeyL, K::KeyM, K::KeyN, K::KeyO, K::KeyP, K::KeyQ, K::KeyR,
        K::KeyS, K::KeyT, K::KeyU, K::KeyV, K::KeyW, K::KeyX, K::KeyY, K::KeyZ,
    ];
    const DIGITS: [KeyCode; 10] = [
        K::Digit1, K::Digit2, K::Digit3, K::Digit4, K::Digit5, K::Digit6, K::Digit7, K::Digit8,
        K::Digit9, K::Digit0,
    ];
    const FKEYS: [KeyCode; 12] = [
        K::F1, K::F2, K::F3, K::F4, K::F5, K::F6, K::F7, K::F8, K::F9, K::F10, K::F11, K::F12,
    ];
    Some(match usage {
        0x04..=0x1d => LETTERS[usize::from(usage - 0x04)],
        0x1e..=0x27 => DIGITS[usize::from(usage - 0x1e)],
        0x28 => K::Enter,
        0x29 => K::Escape,
        0x2a => K::Backspace,
        0x2b => K::Tab,
        0x2c => K::Space,
        0x2d => K::Minus,
        0x2e => K::Equal,
        0x2f => K::BracketLeft,
        0x30 => K::BracketRight,
        0x31 => K::Backslash,
        0x33 => K::Semicolon,
        0x34 => K::Quote,
        0x35 => K::Backquote,
        0x36 => K::Comma,
        0x37 => K::Period,
        0x38 => K::Slash,
        0x39 => K::CapsLock,
        0x3a..=0x45 => FKEYS[usize::from(usage - 0x3a)],
        0x49 => K::Insert,
        0x4a => K::Home,
        0x4b => K::PageUp,
        0x4c => K::Delete,
        0x4d => K::End,
        0x4e => K::PageDown,
        0x4f => K::ArrowRight,
        0x50 => K::ArrowLeft,
        0x51 => K::ArrowDown,
        0x52 => K::ArrowUp,
        0x58 => K::NumpadEnter,
        _ => return None,
    })
}

/// The modifier bits (`KEYBOARD_LEFT_CTRL` …) in the HID's order, as keys.
const MODIFIERS: [KeyCode; 8] = [
    KeyCode::ControlLeft,
    KeyCode::ShiftLeft,
    KeyCode::AltLeft,
    KeyCode::SuperLeft,
    KeyCode::ControlRight,
    KeyCode::ShiftRight,
    KeyCode::AltRight,
    KeyCode::SuperRight,
];

// ---------------------------------------------------------------------------------------------
// The poll

/// Everything one frame writes, kept in winit's order: the typed message and the aggregate
/// `WindowEvent` picking reads.
#[derive(Default)]
struct Out {
    events: Vec<WindowEvent>,
    motion: Vec2,
}

impl Out {
    fn key(&mut self, window: Entity, key: KeyCode, down: bool, shift: bool) {
        let (logical_key, text) = logical(key, shift);
        self.events.push(WindowEvent::KeyboardInput(KeyboardInput {
            key_code: key,
            logical_key,
            state: if down {
                ButtonState::Pressed
            } else {
                ButtonState::Released
            },
            text: if down { text.map(|c| c.to_string().into()) } else { None },
            repeat: false,
            window,
        }));
    }

    fn button(&mut self, window: Entity, button: MouseButton, down: bool) {
        self.events.push(WindowEvent::MouseButtonInput(MouseButtonInput {
            button,
            state: if down {
                ButtonState::Pressed
            } else {
                ButtonState::Released
            },
            window,
        }));
    }
}

/// Moves `held` to `now`, writing a press for each new entry and a release for each gone one,
/// except where `other` (the other device) still holds it.
fn diff<T: PartialEq + Copy>(
    held: &mut Vec<T>,
    now: Vec<T>,
    other: &[T],
    mut write: impl FnMut(T, bool),
) {
    for &gone in held.iter().filter(|k| !now.contains(k)) {
        if !other.contains(&gone) {
            write(gone, false);
        }
    }
    for &new in now.iter().filter(|k| !held.contains(k)) {
        if !other.contains(&new) {
            write(new, true);
        }
    }
    *held = now;
}

pub(super) fn poll(
    time: Res<Time<Real>>,
    mut state: ResMut<PadState>,
    mut windows: Query<(Entity, &mut Window), With<PrimaryWindow>>,
    mut window_events: MessageWriter<WindowEvent>,
    mut keyboard_events: MessageWriter<KeyboardInput>,
    mut button_events: MessageWriter<MouseButtonInput>,
    mut cursor_events: MessageWriter<CursorMoved>,
    mut wheel_events: MessageWriter<MouseWheel>,
    mut motion_events: MessageWriter<MouseMotion>,
) {
    let Ok((window, mut win)) = windows.single_mut() else {
        return;
    };
    let state = &mut *state;
    let mut out = Out::default();

    let mut pad = Pad::default();
    let mut mouse = Mouse::default();
    let mut keyboard = Keyboard {
        connected: false,
        modifiers: 0,
        event_count: 0,
        events: [KeyboardEvent::default(); 64],
    };
    // SAFETY: each poll fills the struct it is handed, laid out as `platform.h` declares it.
    let (mouse_live, keyboard_live) = unsafe {
        pad_poll(&mut pad);
        (mouse_poll(&mut mouse), keyboard_poll(&mut keyboard))
    };

    // The keyboard first: its keys are what the pad must not release.
    let shift = keyboard_live && keyboard.modifiers & (0x02 | 0x20) != 0;
    if keyboard_live {
        let mut keys = state.keyboard_held.clone();
        let count = keyboard.event_count.clamp(0, 64) as usize;
        for ev in &keyboard.events[..count] {
            let Some(key) = hid_key(ev.usage) else {
                continue;
            };
            if ev.down && !keys.contains(&key) {
                keys.push(key);
                if !state.keys.contains(&key) {
                    out.key(window, key, true, shift);
                }
            } else if !ev.down && keys.contains(&key) {
                keys.retain(|k| *k != key);
                if !state.keys.contains(&key) {
                    out.key(window, key, false, shift);
                }
            }
        }
        for (bit, key) in MODIFIERS.iter().enumerate() {
            let down = keyboard.modifiers & (1 << bit) != 0;
            if down != keys.contains(key) {
                if down {
                    keys.push(*key);
                } else {
                    keys.retain(|k| k != key);
                }
                out.key(window, *key, down, shift);
            }
        }
        state.keyboard_held = keys;
    } else if !state.keyboard_held.is_empty() {
        // Unplugged with keys down: let them all go.
        for key in std::mem::take(&mut state.keyboard_held) {
            if !state.keys.contains(&key) {
                out.key(window, key, false, false);
            }
        }
    }

    let keyboard_held = state.keyboard_held.clone();
    diff(&mut state.keys, pad_keys(&pad), &keyboard_held, |key, down| {
        out.key(window, key, down, shift)
    });

    // The cursor: the mouse moves it by counts, the right stick by speed.
    let (w, h) = (
        f64::from(win.resolution.physical_width()),
        f64::from(win.resolution.physical_height()),
    );
    let scale = f64::from(win.resolution.scale_factor());
    let mut at = state.cursor.unwrap_or(DVec2::new(w / 2.0, h / 2.0));
    let dt = time.delta_secs().min(0.1);
    let stick = Vec2::new(pad.right_x, pad.right_y);
    let mut delta = Vec2::ZERO;
    if stick.length() > 0.0 {
        // A curve, so small tilts aim finely and full tilt crosses the screen in about a second.
        let curved = stick * stick.length();
        delta += curved * CURSOR_SPEED * dt * scale as f32;
    }
    if mouse_live {
        delta += Vec2::new(mouse.dx as f32, mouse.dy as f32);
    }
    if delta != Vec2::ZERO || state.cursor.is_none() {
        let moved = (at + delta.as_dvec2()).clamp(DVec2::ZERO, DVec2::new(w - 1.0, h - 1.0));
        let logical_delta = ((moved - at) / scale).as_vec2();
        at = moved;
        state.cursor = Some(at);
        win.set_physical_cursor_position(Some(at));
        out.events.push(WindowEvent::CursorMoved(CursorMoved {
            window,
            position: (at / scale).as_vec2(),
            delta: Some(logical_delta),
        }));
        out.motion += delta;
    }

    // Buttons: the mouse's and the pad's, each released only when neither holds it.
    let mouse_buttons: Vec<MouseButton> = if mouse_live {
        [
            (MOUSE_LEFT, MouseButton::Left),
            (MOUSE_RIGHT, MouseButton::Right),
            (MOUSE_MIDDLE, MouseButton::Middle),
            (MOUSE_BUTTON4, MouseButton::Back),
            (MOUSE_BUTTON5, MouseButton::Forward),
        ]
        .into_iter()
        .filter(|(bit, _)| mouse.buttons & bit != 0)
        .map(|(_, b)| b)
        .collect()
    } else {
        Vec::new()
    };
    let mut union = pad_buttons(&pad);
    for b in &mouse_buttons {
        if !union.contains(b) {
            union.push(*b);
        }
    }
    diff(&mut state.buttons, union, &[], |b, down| out.button(window, b, down));

    // The wheel: the mouse's notches, and R1/L1 as a notch each press.
    let mut notches = if mouse_live { mouse.wheel as f32 } else { 0.0 };
    if pad.pressed & PAD_R1 != 0 {
        notches += 1.0;
    }
    if pad.pressed & PAD_L1 != 0 {
        notches -= 1.0;
    }
    if notches != 0.0 {
        out.events.push(WindowEvent::MouseWheel(MouseWheel {
            unit: MouseScrollUnit::Line,
            x: 0.0,
            y: notches,
            window,
        }));
    }

    for ev in &out.events {
        match ev.clone() {
            WindowEvent::KeyboardInput(e) => {
                keyboard_events.write(e);
            }
            WindowEvent::MouseButtonInput(e) => {
                button_events.write(e);
            }
            WindowEvent::CursorMoved(e) => {
                cursor_events.write(e);
            }
            WindowEvent::MouseWheel(e) => {
                wheel_events.write(e);
            }
            _ => {}
        }
    }
    if !out.events.is_empty() {
        window_events.write_batch(out.events);
    }
    if out.motion != Vec2::ZERO {
        motion_events.write(MouseMotion { delta: out.motion });
    }
}
