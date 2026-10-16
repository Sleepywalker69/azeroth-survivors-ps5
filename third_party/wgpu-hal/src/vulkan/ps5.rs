//! The PlayStation 5 (benilla carry, `third_party/wgpu-hal/BENILLA.md`).
//!
//! A title has no Vulkan loader to `dlopen`: RADV (PS5_Vulkan's archive, built from PS5_Mesa) is
//! linked into the title, and its ICD entry point is the instance's `vkGetInstanceProcAddr`. The
//! console has one output and no windows: a surface is a VK_KHR_display plane surface on the
//! first display, which RADV's PS5 winsys presents through VideoOut.

use alloc::{format, string::String};
use core::ffi::c_char;

use ash::{khr, vk};

unsafe extern "system" {
    /// RADV's loader-interface entry point, the same symbol PS5_Vulkan's titles bind volk to.
    fn vk_icdGetInstanceProcAddr(instance: vk::Instance, name: *const c_char)
        -> vk::PFN_vkVoidFunction;
}

/// The entry points, from the RADV linked into the title.
pub(super) fn entry() -> ash::Entry {
    // SAFETY: `vk_icdGetInstanceProcAddr` is RADV's `vkGetInstanceProcAddr`, valid for the
    // process's lifetime.
    unsafe {
        ash::Entry::from_static_fn(ash::StaticFn {
            get_instance_proc_addr: vk_icdGetInstanceProcAddr,
        })
    }
}

/// `$PS5_DISPLAY_MODE` as `WIDTHxHEIGHT`, the size the title's start-up chose and gave the
/// window; `None` when unset or unreadable.
fn requested_size() -> Option<(u32, u32)> {
    let value = std::env::var("PS5_DISPLAY_MODE").ok()?;
    let (w, h) = value.trim().split_once(['x', 'X'])?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

/// Which of the display's modes to present in: the requested size when it has a mode, else the
/// display's largest (3840x2160 on the console, which VideoOut scales to whatever the screen
/// takes); at that size the refresh closest to 60 Hz,
/// which the game's frame pacing is built around (a 120 Hz mode would double every frame's CPU
/// cost for nothing it shows).
fn choose_mode(modes: &[vk::DisplayModePropertiesKHR]) -> Option<vk::DisplayModePropertiesKHR> {
    let area = |m: &vk::DisplayModePropertiesKHR| {
        u64::from(m.parameters.visible_region.width) * u64::from(m.parameters.visible_region.height)
    };
    let offered = |&(w, h): &(u32, u32)| {
        modes.iter().any(|m| {
            m.parameters.visible_region.width == w && m.parameters.visible_region.height == h
        })
    };
    let size = requested_size()
        .filter(offered)
        .or_else(|| {
            modes.iter().max_by_key(|m| area(m)).map(|m| {
                (
                    m.parameters.visible_region.width,
                    m.parameters.visible_region.height,
                )
            })
        })?;
    modes
        .iter()
        .filter(|m| {
            (
                m.parameters.visible_region.width,
                m.parameters.visible_region.height,
            ) == size
        })
        .min_by_key(|m| m.parameters.refresh_rate.abs_diff(60_000))
        .copied()
}

impl super::Instance {
    /// A plane surface on the console's display. `plane` is the window handle's plane, used when
    /// no plane says it can show the display.
    pub(super) unsafe fn create_surface_ps5_display(
        &self,
        plane: u32,
    ) -> Result<super::Surface, crate::InstanceError> {
        let fail = |what: &str, e: vk::Result| {
            crate::InstanceError::with_source(format!("PS5 display surface: {what}"), e)
        };
        let display_instance = khr::display::Instance::new(&self.shared.entry, &self.shared.raw);
        let physical = unsafe { self.shared.raw.enumerate_physical_devices() }
            .map_err(|e| fail("vkEnumeratePhysicalDevices", e))?
            .first()
            .copied()
            .ok_or_else(|| crate::InstanceError::new(String::from("PS5: no physical device")))?;
        let displays = unsafe { display_instance.get_physical_device_display_properties(physical) }
            .map_err(|e| fail("vkGetPhysicalDeviceDisplayPropertiesKHR", e))?;
        let display = displays
            .first()
            .map(|d| d.display)
            .ok_or_else(|| crate::InstanceError::new(String::from("PS5: no display")))?;
        let modes = unsafe { display_instance.get_display_mode_properties(physical, display) }
            .map_err(|e| fail("vkGetDisplayModePropertiesKHR", e))?;
        let mode = choose_mode(&modes)
            .ok_or_else(|| crate::InstanceError::new(String::from("PS5: the display has no mode")))?;

        // The first plane that can show this display, else the handle's.
        let planes = unsafe { display_instance.get_physical_device_display_plane_properties(physical) }
            .map(|p| p.len() as u32)
            .unwrap_or(0);
        let plane_index = (0..planes)
            .find(|&i| {
                unsafe { display_instance.get_display_plane_supported_displays(physical, i) }
                    .is_ok_and(|shown| shown.contains(&display))
            })
            .unwrap_or(plane);

        log::info!(
            "PS5 display surface: {}x{} at {} mHz on plane {plane_index} ({} modes)",
            mode.parameters.visible_region.width,
            mode.parameters.visible_region.height,
            mode.parameters.refresh_rate,
            modes.len()
        );

        let create_info = vk::DisplaySurfaceCreateInfoKHR::default()
            .display_mode(mode.display_mode)
            .image_extent(mode.parameters.visible_region)
            .transform(vk::SurfaceTransformFlagsKHR::IDENTITY)
            .alpha_mode(vk::DisplayPlaneAlphaFlagsKHR::OPAQUE)
            .global_alpha(1.0)
            .plane_index(plane_index);
        let surface = unsafe { display_instance.create_display_plane_surface(&create_info, None) }
            .map_err(|e| fail("vkCreateDisplayPlaneSurfaceKHR", e))?;
        Ok(self.create_surface_from_vk_surface_khr(surface))
    }
}

/// The size [`super::Instance::create_surface_ps5_display`] will present at, asked of a
/// short-lived instance of its own: the window must be given it before the renderer's instance
/// exists, since a display surface is exactly its mode's size. `None` when RADV gives no display.
pub fn display_size() -> Option<(u32, u32)> {
    let entry = entry();
    let extensions = [khr::surface::NAME.as_ptr(), khr::display::NAME.as_ptr()];
    let app = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_1);
    let info = vk::InstanceCreateInfo::default()
        .application_info(&app)
        .enabled_extension_names(&extensions);
    // SAFETY: a plain instance with the two extensions RADV's PS5 winsys always offers; destroyed
    // before returning, and nothing made from it outlives it.
    unsafe {
        let instance = entry.create_instance(&info, None).ok()?;
        let size = (|| {
            let display_instance = khr::display::Instance::new(&entry, &instance);
            let physical = *instance.enumerate_physical_devices().ok()?.first()?;
            let display = display_instance
                .get_physical_device_display_properties(physical)
                .ok()?
                .first()?
                .display;
            let modes = display_instance
                .get_display_mode_properties(physical, display)
                .ok()?;
            let mode = choose_mode(&modes)?;
            Some((
                mode.parameters.visible_region.width,
                mode.parameters.visible_region.height,
            ))
        })();
        instance.destroy_instance(None);
        size
    }
}
