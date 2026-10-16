# wgpu-hal 27.0.4, patched for the PlayStation 5

The crates.io release of wgpu-hal 27.0.4, unchanged but for these hunks, each behind
`target_env = "ps5"` (the custom target `ps5/x86_64-ps5-freebsd.json` sets `"env": "ps5"`), so
every other target builds the upstream code:

- `src/vulkan/ps5.rs` (new): the instance's entry points from `vk_icdGetInstanceProcAddr`, RADV's
  ICD entry point linked into the title (PS5_Vulkan's release archive), instead of `dlopen`ing a
  loader the console does not have; and `create_surface_ps5_display`, a VK_KHR_display plane
  surface on the first display in the mode `$PS5_DISPLAY_MODE` names (else the largest,
  3840x2160), at the refresh closest to 60 Hz; and `display_size`, that mode's
  size asked of a short-lived instance, exported as `vulkan::ps5_display_size` so the window can
  be given it before the renderer starts.
- `src/vulkan/mod.rs`: declares the module and re-exports `ps5_display_size`.
- `src/vulkan/instance.rs`: `init_with_callback` takes `ps5::entry()`; `create_surface` answers a
  `RawWindowHandle::Drm` with the display surface (`benilla-app/src/ps5/window.rs` gives the
  window that handle).

`examples/` and the `[[example]]` targets are dropped from the vendored copy. To check the hunks
against a newer wgpu-hal, diff this folder against the registry's copy of the same version.
