//! The host on a desktop (Windows): a winit window that the page's canvas fills, its input as the
//! page's platform events, and the loop that runs the page's tasks and animation frames.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, KeyCode, KeyLocation, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{CursorGrabMode, CursorIcon, CustomCursor, Fullscreen, Window, WindowId};

use crate::audio::Audio;
use crate::data::Data;
use crate::ffi;
use crate::files::Files;
use crate::gpu::Gpu;
use crate::log::{fail, say, warn};
use crate::ops::{self, Cursor, Host, Request};
use crate::storage::Storage;

const USAGE: &str = "\
usage: aethus-host [options]
  --js DIR          the host's scripts (env.js, gpu.js, audio.js, boot.mjs)
  --web DIR         the web build (wenilla.js and its snippets)
  --data DIR        a folder of MPQs; repeat for more, later folders win (default: $WOW_DATA)
  --home DIR        the player's folder: settings, saves, AddOns, the log
  --size WxH        the window's size in logical pixels (default 1280x720)
  --fullscreen      start in borderless full screen
  --backend NAME    vulkan, dx12 or all (default: vulkan and dx12)
  --present MODE    vsync, fifo, mailbox, immediate or novsync (default vsync)
  --rogue KEY=VALUE a quick-start setting (glue, bg, class, race, name, gender)
  --env KEY=VALUE   a client setting (CVar) on top of the page's defaults";

struct Options {
    js: PathBuf,
    web: PathBuf,
    data: Vec<PathBuf>,
    home: PathBuf,
    size: (u32, u32),
    fullscreen: bool,
    backends: wgpu::Backends,
    present: wgpu::PresentMode,
    rogue: Map<String, Value>,
    env: Map<String, Value>,
}

fn key_value(s: &str) -> Result<(String, Value), String> {
    let (k, v) = s.split_once('=').ok_or_else(|| format!("{s:?} is not KEY=VALUE"))?;
    Ok((k.to_owned(), Value::String(v.to_owned())))
}

impl Options {
    fn parse() -> Result<Options, String> {
        // Beside the executable when the host is installed; the source tree's when it is run
        // from its build folder.
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default();
        let tree = Path::new(env!("CARGO_MANIFEST_DIR"));
        let pick = |beside: &str, in_tree: PathBuf| {
            let p = exe_dir.join(beside);
            if p.exists() {
                p
            } else {
                in_tree
            }
        };
        let home = std::env::var_os("APPDATA").map_or_else(|| exe_dir.join("userdata"), |d| PathBuf::from(d).join("AethusHost"));
        let mut o = Options {
            js: pick("js", tree.join("../js")),
            web: pick("web", tree.join("../../aethus-web-0.96.0")),
            data: Vec::new(),
            home,
            size: (1280, 720),
            fullscreen: false,
            backends: wgpu::Backends::VULKAN | wgpu::Backends::DX12,
            present: wgpu::PresentMode::AutoVsync,
            rogue: Map::new(),
            env: Map::new(),
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value\n{USAGE}"));
            match arg.as_str() {
                "--js" => o.js = value()?.into(),
                "--web" => o.web = value()?.into(),
                "--data" => o.data.push(value()?.into()),
                "--home" => o.home = value()?.into(),
                "--size" => {
                    let v = value()?;
                    let (w, h) = v.split_once('x').ok_or_else(|| format!("{v:?} is not WxH"))?;
                    o.size = (w.parse().map_err(|_| format!("{v:?} is not WxH"))?, h.parse().map_err(|_| format!("{v:?} is not WxH"))?);
                }
                "--fullscreen" => o.fullscreen = true,
                "--windowed" => o.fullscreen = false,
                "--backend" => {
                    o.backends = match value()?.as_str() {
                        "vulkan" => wgpu::Backends::VULKAN,
                        "dx12" => wgpu::Backends::DX12,
                        "all" => wgpu::Backends::all(),
                        other => return Err(format!("{other:?} is not a backend\n{USAGE}")),
                    }
                }
                "--present" => {
                    o.present = match value()?.as_str() {
                        "vsync" => wgpu::PresentMode::AutoVsync,
                        "novsync" => wgpu::PresentMode::AutoNoVsync,
                        "fifo" => wgpu::PresentMode::Fifo,
                        "mailbox" => wgpu::PresentMode::Mailbox,
                        "immediate" => wgpu::PresentMode::Immediate,
                        other => return Err(format!("{other:?} is not a present mode\n{USAGE}")),
                    }
                }
                "--rogue" => {
                    let (k, v) = key_value(&value()?)?;
                    o.rogue.insert(k, v);
                }
                "--env" => {
                    let (k, v) = key_value(&value()?)?;
                    o.env.insert(k, v);
                }
                "--help" | "-h" => return Err(USAGE.into()),
                _ => return Err(format!("{arg:?} is not an option\n{USAGE}")),
            }
        }
        if o.data.is_empty() {
            o.data.push(std::env::var_os("WOW_DATA").map_or_else(|| exe_dir.join("Data"), PathBuf::from));
        }
        Ok(o)
    }
}

/// Starts the host and runs it until the window closes; the process's exit code.
pub fn run() -> i32 {
    let options = match Options::parse() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            return 2;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&options.home) {
        eprintln!("{}: {e}", options.home.display());
        return 1;
    }
    crate::log::to_file(&options.home.join("aethus-host.log"));
    say!("host: the player's folder is {}", options.home.display());
    let event_loop = match EventLoop::new() {
        Ok(event_loop) => event_loop,
        Err(e) => {
            fail!("host: there is no event loop: {e}");
            return 1;
        }
    };
    let mut app = App::new(options);
    if let Err(e) = event_loop.run_app(&mut app) {
        fail!("host: the event loop failed: {e}");
        return 1;
    }
    app.exit_code
}

struct App {
    options: Options,
    window: Option<Arc<Window>>,
    running: bool,
    exit_code: i32,
    /// Physical pixels to a CSS pixel.
    scale: f64,
    /// The pointer, in CSS pixels.
    pointer: (f64, f64),
    modifiers: u32,
    pointer_locked: bool,
    /// The page set the cursor 'none'.
    cursor_hidden: bool,
    /// Whether frames are drawn: the window is neither minimised nor covered.
    visible: bool,
    minimized: bool,
    /// A display refresh, and when the next frame is due (the page's clock, ms).
    interval: f64,
    next_frame: f64,
    cursors: HashMap<u64, CustomCursor>,
}

impl App {
    fn new(options: Options) -> App {
        App {
            options,
            window: None,
            running: false,
            exit_code: 0,
            scale: 1.0,
            pointer: (0.0, 0.0),
            modifiers: 0,
            pointer_locked: false,
            cursor_hidden: false,
            visible: true,
            minimized: false,
            interval: 1000.0 / 60.0,
            next_frame: 0.0,
            cursors: HashMap::new(),
        }
    }

    fn start(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        let o = &self.options;
        let mut attributes = Window::default_attributes()
            .with_title("Azeroth Survivors (Aethus)")
            .with_inner_size(LogicalSize::new(o.size.0, o.size.1));
        if o.fullscreen {
            attributes = attributes.with_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        let window = Arc::new(event_loop.create_window(attributes).map_err(|e| format!("the window could not be made: {e}"))?);
        self.scale = window.scale_factor();
        self.interval = window
            .current_monitor()
            .and_then(|m| m.refresh_rate_millihertz())
            .map_or(1000.0 / 60.0, |mhz| 1_000_000.0 / f64::from(mhz.max(1000)));
        let size = window.inner_size();

        let target = window.clone();
        let gpu = Gpu::new(o.backends, o.present, move |instance| match instance.create_surface(target) {
            Ok(surface) => Some(surface),
            Err(e) => {
                warn!("gpu: the window has no surface: {e}");
                None
            }
        });
        let config = json!({
            "viewport": [f64::from(size.width) / self.scale, f64::from(size.height) / self.scale],
            "dpr": self.scale,
            "userAgent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AethusHost/1.0",
            "platform": "Win32",
            "language": "en-US",
            "rogue": o.rogue,
            "env": o.env,
            "addonsDir": o.home.join("AddOns").display().to_string(),
        });
        ops::install(Host {
            config: config.to_string(),
            data: Data::mount(&o.data),
            storage: Storage::open(o.home.join("localStorage.json")),
            files: Files::new(&o.home),
            gpu,
            audio: Audio::new(),
            events: Vec::new(),
            gamepads: String::new(),
            requests: Vec::new(),
            quit: None,
            stage: String::new(),
        });
        self.window = Some(window);
        say!("host: the page is {} with the scripts in {}", o.web.display(), o.js.display());
        ffi::js_init(&o.js, &o.web)?;
        self.running = true;
        self.next_frame = ffi::now_ms();
        Ok(())
    }

    fn push(&self, event: Value) {
        ops::with(|h| h.event(event));
    }

    fn resize_event(&self, width: u32, height: u32) -> Value {
        json!({ "t": "resize", "width": f64::from(width) / self.scale, "height": f64::from(height) / self.scale, "dpr": self.scale })
    }

    fn quit(&mut self, event_loop: &ActiveEventLoop, reason: &str) {
        say!("host: quitting: {reason}");
        let now = ffi::now_ms();
        ops::with(|h| h.storage.flush(now, true));
        self.running = false;
        event_loop.exit();
    }

    fn show_cursor(&self, window: &Window) {
        window.set_cursor_visible(!self.cursor_hidden && !self.pointer_locked);
    }

    /// Carries out what the page asked of the window.
    fn apply(&mut self, event_loop: &ActiveEventLoop, request: Request) {
        let Some(window) = self.window.clone() else { return };
        match request {
            Request::Cursor(Cursor::Named(name)) => {
                self.cursor_hidden = name == "none";
                if !self.cursor_hidden {
                    window.set_cursor(name.parse::<CursorIcon>().unwrap_or_default());
                }
                self.show_cursor(&window);
            }
            Request::Cursor(Cursor::Image { rgba, width, height, hot_x, hot_y }) => {
                let mut hasher = DefaultHasher::new();
                (&rgba, width, height, hot_x, hot_y).hash(&mut hasher);
                let key = hasher.finish();
                let cursor = match self.cursors.get(&key) {
                    Some(cursor) => cursor.clone(),
                    None => {
                        let clamp = |n: u32| u16::try_from(n).unwrap_or(u16::MAX);
                        match CustomCursor::from_rgba(rgba, clamp(width), clamp(height), clamp(hot_x), clamp(hot_y)) {
                            Ok(source) => {
                                let cursor = event_loop.create_custom_cursor(source);
                                self.cursors.insert(key, cursor.clone());
                                cursor
                            }
                            Err(e) => {
                                warn!("host: a {width}x{height} cursor image cannot be used: {e}");
                                return;
                            }
                        }
                    }
                };
                self.cursor_hidden = false;
                window.set_cursor(cursor);
                self.show_cursor(&window);
            }
            Request::PointerLock(on) => {
                if on {
                    let grab = window.set_cursor_grab(CursorGrabMode::Locked).or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined));
                    if let Err(e) = grab {
                        warn!("host: the pointer cannot be held in the window: {e}");
                    }
                } else if let Err(e) = window.set_cursor_grab(CursorGrabMode::None) {
                    warn!("host: the pointer cannot be let go: {e}");
                }
                self.pointer_locked = on;
                self.show_cursor(&window);
            }
            Request::Fullscreen(on) => window.set_fullscreen(on.then_some(Fullscreen::Borderless(None))),
        }
    }
}

fn modifier_bits(state: ModifiersState) -> u32 {
    u32::from(state.shift_key()) | u32::from(state.control_key()) << 1 | u32::from(state.alt_key()) << 2 | u32::from(state.super_key()) << 3
}

/// KeyboardEvent.code: winit's key codes are the DOM's names, but for the Meta keys.
fn dom_code(key: PhysicalKey) -> String {
    match key {
        PhysicalKey::Code(KeyCode::SuperLeft) => "MetaLeft".into(),
        PhysicalKey::Code(KeyCode::SuperRight) => "MetaRight".into(),
        PhysicalKey::Code(code) => format!("{code:?}"),
        PhysicalKey::Unidentified(_) => String::new(),
    }
}

/// KeyboardEvent.key.
fn dom_key(key: &Key) -> String {
    match key {
        Key::Character(s) => s.to_string(),
        Key::Named(NamedKey::Space) => " ".into(),
        Key::Named(NamedKey::Super) => "Meta".into(),
        Key::Named(named) => format!("{named:?}"),
        Key::Dead(_) => "Dead".into(),
        Key::Unidentified(_) => "Unidentified".into(),
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        if let Err(e) = self.start(event_loop) {
            fail!("host: {e}");
            self.exit_code = 1;
            event_loop.exit();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let m = self.modifiers;
        match event {
            WindowEvent::CloseRequested => self.quit(event_loop, "the window was closed"),
            WindowEvent::Resized(size) => {
                self.minimized = size.width == 0 || size.height == 0;
                if !self.minimized {
                    self.push(self.resize_event(size.width, size.height));
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor;
                if let Some(size) = self.window.as_ref().map(|w| w.inner_size()) {
                    self.push(self.resize_event(size.width, size.height));
                }
            }
            WindowEvent::Focused(focused) => self.push(json!({ "t": "focus", "focused": focused })),
            WindowEvent::Occluded(occluded) => {
                self.visible = !occluded;
                self.push(json!({ "t": "visible", "visible": !occluded }));
            }
            WindowEvent::CursorEntered { .. } => {
                self.push(json!({ "t": "over", "x": self.pointer.0, "y": self.pointer.1, "m": m }));
            }
            WindowEvent::CursorLeft { .. } => self.push(json!({ "t": "out", "m": m })),
            WindowEvent::CursorMoved { position, .. } => {
                // While the pointer is locked, its motion comes as raw deltas (`device_event`).
                if self.pointer_locked {
                    return;
                }
                let (x, y) = (position.x / self.scale, position.y / self.scale);
                let (dx, dy) = (x - self.pointer.0, y - self.pointer.1);
                self.pointer = (x, y);
                self.push(json!({ "t": "move", "x": x, "y": y, "dx": dx, "dy": dy, "m": m }));
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let b = match button {
                    MouseButton::Left => 0,
                    MouseButton::Middle => 1,
                    MouseButton::Right => 2,
                    MouseButton::Back => 3,
                    MouseButton::Forward => 4,
                    MouseButton::Other(_) => return,
                };
                let t = if state == ElementState::Pressed { "down" } else { "up" };
                self.push(json!({ "t": t, "b": b, "x": self.pointer.0, "y": self.pointer.1, "m": m }));
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // The DOM's deltas point the other way to winit's.
                let (dx, dy, mode) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (-f64::from(x), -f64::from(y), 1),
                    MouseScrollDelta::PixelDelta(p) => (-p.x / self.scale, -p.y / self.scale, 0),
                };
                self.push(json!({ "t": "wheel", "dx": dx, "dy": dy, "mode": mode, "m": m }));
            }
            WindowEvent::KeyboardInput { event, is_synthetic, .. } => {
                // winit makes up presses for keys held when the window gains focus; a page sees
                // none.
                if is_synthetic && event.state == ElementState::Pressed {
                    return;
                }
                let location = match event.location {
                    KeyLocation::Standard => 0,
                    KeyLocation::Left => 1,
                    KeyLocation::Right => 2,
                    KeyLocation::Numpad => 3,
                };
                self.push(json!({
                    "t": "key",
                    "down": event.state == ElementState::Pressed,
                    "key": dom_key(&event.logical_key),
                    "code": dom_code(event.physical_key),
                    "location": location,
                    "repeat": event.repeat,
                    "m": m,
                }));
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifier_bits(modifiers.state()),
            _ => {}
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta: (dx, dy) } = event {
            if self.pointer_locked {
                self.push(json!({ "t": "move", "dx": dx, "dy": dy, "m": self.modifiers }));
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if !self.running {
            return;
        }
        let now = ffi::now_ms();
        let mut wait = ffi::js_tick(now);
        let drawing = self.visible && !self.minimized;
        if drawing && now >= self.next_frame {
            ffi::js_frame(now);
            // Paced a little ahead of the display: with vsync, acquiring the next image waits
            // for it.
            self.next_frame = (self.next_frame + self.interval).max(ffi::now_ms() + self.interval * 0.25) - 1.0;
            wait = ffi::js_tick(ffi::now_ms());
        }
        ffi::js_collect();

        let now = ffi::now_ms();
        let Some((requests, quit, busy)) = ops::with(|h| {
            h.gpu.maintain();
            h.storage.flush(now, false);
            (std::mem::take(&mut h.requests), h.quit.clone(), h.audio.busy() || h.gpu.busy())
        }) else {
            return;
        };
        for request in requests {
            self.apply(event_loop, request);
        }
        if let Some(reason) = quit {
            self.exit_code = if reason.starts_with("the client could not start") { 1 } else { 0 };
            self.quit(event_loop, &reason);
            return;
        }

        let mut next = if drawing { self.next_frame } else { now + 100.0 };
        if wait >= 0.0 {
            next = next.min(now + wait);
        }
        if busy {
            // Ended voices, finished decodes and GPU maps are delivered on a tick.
            next = next.min(now + 2.0);
        }
        let delay = next - now;
        event_loop.set_control_flow(if delay <= 0.0 {
            ControlFlow::Poll
        } else {
            ControlFlow::WaitUntil(Instant::now() + Duration::from_secs_f64(delay / 1000.0))
        });
    }
}
