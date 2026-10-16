//! The PlayStation 5's window and main loop, the parts `bevy_winit` is everywhere else.
//!
//! The console has one screen and no window system: RADV presents a VK_KHR_display plane surface
//! through VideoOut (`third_party/wgpu-hal`'s `vulkan/ps5.rs`). The primary window gets a
//! `RawWindowHandle::Drm`, which that patched wgpu-hal answers with the display surface, and the
//! size of the display mode the surface will use ([`display_mode`]), since a display surface is
//! always exactly the mode's size. With no event loop to hand frames
//! out, [`ScheduleRunnerPlugin`] runs the app back to back and the FIFO present paces it to the
//! display's refresh.
//!
//! Added straight after `WindowPlugin` ([`crate::boot::tuned_default_plugins`]), so the handle is
//! on the primary window before `RenderPlugin::build` reads it to pick its adapter.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use bevy::window::{
    PrimaryWindow, RawHandleWrapper, RawHandleWrapperHolder, WindowMode, WindowResolution,
    WindowWrapper,
};
use raw_window_handle::{
    DisplayHandle, DrmDisplayHandle, DrmWindowHandle, HandleError, HasDisplayHandle,
    HasWindowHandle, RawDisplayHandle, RawWindowHandle, WindowHandle,
};

/// The logical height the UI is laid out for: a 1080p screen is scale 1, a 4K one scale 2, so
/// the game's logical-pixel layout reads the same on either.
const LOGICAL_HEIGHT: f32 = 1080.0;

/// The size used when start-up named none and the display could not be asked: the console's
/// largest mode, the one the surface takes by default.
const FALLBACK: (u32, u32) = (3840, 2160);

/// `$PS5_DISPLAY_MODE` as `WIDTHxHEIGHT`.
fn requested_mode() -> Option<(u32, u32)> {
    std::env::var("PS5_DISPLAY_MODE")
        .ok()
        .and_then(|v| {
            let (w, h) = v.trim().split_once(['x', 'X'])?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .filter(|&(w, h)| w > 0 && h > 0)
}

/// The size the screen presents at: `$PS5_DISPLAY_MODE` (`/app0/env.txt` may set it), else the
/// mode the patched wgpu-hal will choose, asked of the display once and written back to
/// `$PS5_DISPLAY_MODE` so the surface and the window agree.
pub fn display_mode() -> (u32, u32) {
    static MODE: std::sync::OnceLock<(u32, u32)> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| {
        if let Some(mode) = requested_mode() {
            return mode;
        }
        let (w, h) = wgpu_hal::vulkan::ps5_display_size().unwrap_or_else(|| {
            warn!("ps5: the display reported no mode; assuming {FALLBACK:?}");
            FALLBACK
        });
        // Set once, on the main thread, before the render and task-pool threads exist.
        std::env::set_var("PS5_DISPLAY_MODE", format!("{w}x{h}"));
        (w, h)
    })
}

/// The console's screen as a window: nothing to own, only the handle the patched wgpu-hal reads.
struct Screen;

impl HasWindowHandle for Screen {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // SAFETY: a DRM handle names a plane by number; there is no pointer to outlive.
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Drm(DrmWindowHandle::new(0))) })
    }
}

impl HasDisplayHandle for Screen {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        // SAFETY: as above; the descriptor is never opened (the plane surface needs none).
        Ok(unsafe { DisplayHandle::borrow_raw(RawDisplayHandle::Drm(DrmDisplayHandle::new(-1))) })
    }
}

/// The window, the handle and the main loop.
pub struct Ps5DisplayPlugin;

impl Plugin for Ps5DisplayPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ScheduleRunnerPlugin::run_loop(Duration::ZERO));

        let (w, h) = display_mode();
        let world = app.world_mut();
        let Ok((entity, holder)) = world
            .query_filtered::<(Entity, &RawHandleWrapperHolder), With<PrimaryWindow>>()
            .single(world)
            .map(|(e, h)| (e, h.clone()))
        else {
            warn!("ps5: no primary window to put on the screen");
            return;
        };
        let handle = match RawHandleWrapper::new(&WindowWrapper::new(Screen)) {
            Ok(handle) => handle,
            Err(e) => {
                error!("ps5: the screen's window handle: {e}");
                return;
            }
        };
        *holder.0.lock().unwrap() = Some(handle.clone());
        let mut window = world.entity_mut(entity);
        window.insert(handle);
        if let Some(mut window) = window.get_mut::<Window>() {
            pin(&mut window, w, h);
        }
        info!("ps5: screen {w}x{h}, UI scale {:.3}", h as f32 / LOGICAL_HEIGHT);
        // Nothing may resize it: the swapchain is the display mode's size or nothing.
        app.add_systems(Last, keep_pinned);
    }
}

fn pin(window: &mut Window, w: u32, h: u32) {
    window.mode = WindowMode::Windowed;
    window.resolution =
        WindowResolution::new(w, h).with_scale_factor_override(h as f32 / LOGICAL_HEIGHT);
    window.focused = true;
}

/// Undo any resize or mode change the game's video options make; on the console the screen is
/// the screen.
fn keep_pinned(mut windows: Query<&mut Window, (With<PrimaryWindow>, Changed<Window>)>) {
    let (w, h) = display_mode();
    for mut window in &mut windows {
        let res = &window.resolution;
        if res.physical_width() != w
            || res.physical_height() != h
            || window.mode != WindowMode::Windowed
        {
            pin(&mut window, w, h);
        }
    }
}
