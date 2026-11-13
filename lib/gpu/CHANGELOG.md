# Changelog of jflib-gpu

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

## [0.2.0] - 2026-11-13

### Added

- A Metal backend for macOS and iOS (`aarch64-apple-darwin`, `aarch64-apple-ios`,
  `aarch64-apple-ios-sim`) with the same types and methods as on Vulkan. `ShaderModule::from_spirv`
  translates the SPIR-V to the Metal Shading Language through `shader::msl`; the device's timeline
  is an `MTLSharedEvent`, and a command buffer that ends in an error loses the device as a driver
  reset does on Vulkan. `jfgui_GPU_VALIDATE=1` turns on Metal's API validation. Not on Metal yet:
  timestamps and a compute queue of its own.
- Swapchains on Metal sit on a `CAMetalLayer` (BGRA8, the sRGB colour space, three drawables);
  they present with `Fifo` only (`Mailbox` and `Immediate` become `Fifo`), and `try_acquire` never
  waits.
- Level `Voll` on Metal for devices with Metal 3 (A14, M1 and newer): bindless tables as Metal 3
  argument buffers, `draw_indexed_indirect_multi` as a loop of indirect draws,
  `draw_indexed_indirect_count` through a compute pass while the render pass is suspended, draw
  parameters and subgroups. Other devices and the iOS simulator get `Basis`.
- A queue of its own for compute and copies on Vulkan, only when asked for:
  `Adapter::create_device_with(&DeviceDesc, &DeviceOptions)` with `DeviceOptions::compute_queue`
  (`DeviceOptions` is `#[non_exhaustive]`: start from `DeviceOptions::DEFAULT` or use
  `with_compute_queue()`). `Device::compute_queue()` gives `Option<&ComputeQueue>` with `encoder`,
  `submit` and `submit_after`: a queue family without graphics where the device has one, `None`
  in the browser, on Metal, when not asked for and with `jfgui_GPU_COMPUTE=0`. With it, buffers and
  textures other than render targets are shared by both families; render passes stay on the
  device's queue. `create_device` keeps one queue and exclusive resources, as in 0.1.0.
- `Queue::submit_after(&[id], ..)`: a submission that waits for earlier ones of either queue
  through their timelines. `Device::wait`, `is_done` and `wait_idle` cover both queues.
- `TextureView::usage` (what its texture may be used for) and `TextureView::same` (the same view
  or a clone of it, not merely one of the same range: a key for a cache).

### Changed

- Validation with `jfgui_GPU_VALIDATE=1` on Vulkan also checks what shaders read and write through
  their bindings (the layer's `syncval_shader_accesses_heuristic`), so a missing barrier between
  two shaders no longer goes unreported. The layer infers these accesses from the SPIR-V and can
  report false positives.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
