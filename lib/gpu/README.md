# jflib-gpu

Direct access to the graphics card for [JFLib](https://jflib.org): compute shaders, rendering with
a depth test, indirect draws, timestamps and swapchains for windows, behind one explicit API in
the manner of Vulkan and WebGPU. The backend is chosen per target when the program is compiled:
Vulkan on Windows, Linux and Android, Metal on macOS and iOS, WebGPU in the browser. There is no
dynamic dispatch, and the same code runs on all of them. JFLib's UI presents its windows through
this crate on most systems, and JFLib's 2D and 3D renderers are built on it.

## How it is used

The objects are the familiar ones: an `Instance` lists the `Adapter`s (discrete GPUs first), an
adapter creates a `Device`, the device creates buffers, textures, samplers, shader modules, bind
groups and pipelines, a `CommandEncoder` records copies, barriers, dispatches and render passes,
and the device's `Queue` submits them. Each submission returns a `SubmitId`, its place on the
device's timeline, which `Device::wait` and `Device::is_done` take.

```rust
use jflib::gpu;

let inst = gpu::Instance::new()?;          // Err(NoLoader) without Vulkan, Err(Disabled) with jfgui_GPU=0
let adapter = inst.adapter()?;             // a discrete GPU first; jfgui_GPU_ADAPTER picks another
let dev = adapter.create_device(&gpu::DeviceDesc::DEFAULT)?;
let shader = gpu::ShaderModule::from_spirv(&dev, CS_FILL)?;   // SPIR-V words, e.g. from jflib-wgsl
let bgl = dev.create_bind_group_layout(&[gpu::Binding {
    binding: 0, kind: gpu::BindingKind::StorageBuffer, stages: gpu::Stages::COMPUTE }])?;
let layout = dev.create_pipeline_layout(&gpu::PipelineLayoutDesc { groups: &[&bgl], push_constants: 12 })?;
let pipe = dev.create_compute_pipeline(&gpu::ComputePipelineDesc {
    layout: &layout, shader: gpu::ShaderStage::new(&shader, "main") })?;
let buf = dev.create_buffer(&gpu::BufferDesc { size: 4 << 20, usage: gpu::BufferUsage::STORAGE,
                                               memory: gpu::MemoryKind::Device })?;
let rb = dev.create_buffer(&gpu::BufferDesc { size: 4 << 20, usage: gpu::BufferUsage::NONE,
                                              memory: gpu::MemoryKind::Readback })?;
let group = dev.create_bind_group(&gpu::BindGroupDesc { layout: &bgl, entries: &[gpu::BindEntry {
    binding: 0, resource: gpu::Resource::Buffer { buffer: &buf, offset: 0, size: 0 } }] })?;

let mut e = dev.encoder()?;
e.set_compute_pipeline(&pipe);
e.set_bind_group(0, &group);
e.push_constants(0, &[1u32 << 20, 3, 7]);
e.dispatch(1 << 14, 1, 1);
e.barrier(gpu::Access::COMPUTE_WRITE, gpu::Access::COPY_READ);
e.copy_buffer(&buf, 0, &rb, 0, 4 << 20);
e.barrier(gpu::Access::COPY_WRITE, gpu::Access::HOST_READ);
let id = dev.queue().submit([e.finish()?])?;
dev.wait(id, 1_000_000_000)?;
let mut out = vec![0u32; 1 << 20];
rb.read(0, &mut out)?;
```

The conventions are WebGPU's on every backend: normalized device coordinates with y up and depth
from 0 to 1, the origin of framebuffers and textures at the top left, front faces counter-clockwise,
group g and binding b of a layout matching WGSL's `@group(g) @binding(b)`, and one block of push
constants of at most 128 bytes. Textures are in explicit states (`Layout`) that the caller names
in its barriers; render passes move their attachments themselves.

## Shaders

Vulkan and Metal take SPIR-V (`ShaderModule::from_spirv`); the Metal backend translates it to the
Metal Shading Language with [`jflib-shader`](https://crates.io/crates/jflib-shader). The browser
compiles WGSL itself, so there `ShaderModule::from_wgsl` takes the text. An app writes its shaders
once in WGSL, compiles them to SPIR-V with [`jflib-wgsl`](https://crates.io/crates/jflib-wgsl),
ahead of time or at run time, and hands the same text to the browser. A pipeline sets the shader's
overrides per stage (`ShaderStage::constants`); they are converted and checked against the module
as WebGPU does, and so is the workgroup size against the device's limits.

## Two levels

`Level::Basis` is what WebGPU and Vulkan 1.1 can do: compute shaders, storage buffers, single
indirect draws and dispatches, texture arrays, timestamps where the device has them. `Level::Voll`
adds bindless tables, `draw_indexed_indirect_count`, multi-draw, draw parameters and subgroup
operations; on Vulkan it needs version 1.3 with descriptor indexing. `device.level()` and
`device.features()` say what a device has, and `DeviceDesc { max_level: Level::Basis }` forces the
smaller level to test those paths on large hardware.

## Errors and lifetimes

Every call that can fail returns a `Result`; nothing panics. Misuse the backend can detect
cheaply, such as a size, an alignment, a state, a draw without a pipeline or a `Voll` command on
`Basis`, is `GpuError::Invalid`, and recorded commands report the first one from `finish`. A lost
device (a driver reset, a removed GPU) gives `DeviceLost` on every later call: drop the `Device`
and create a new one from the adapter.

Resources are reference-counted handles. A dropped resource is destroyed once the GPU has finished
every submission made before the drop, and the CPU waits only where it is asked to (`wait`,
`read_buffer`, reading timestamps, rebuilding a swapchain).

## Windows

`Surface::new` takes a window of [`jflib-plat`](https://crates.io/crates/jflib-plat),
`create_swapchain` puts a swapchain on it, and each frame is `acquire`, render into
`view(&frame)`, `present`. `try_acquire` never waits and never rebuilds, for a render loop that
must not block. The swapchain rebuilds itself after a resize or when the system says the old one is
out of date; `Fifo` (vertical sync) is the default, `Mailbox` and `Immediate` are there where the
system has them.

## Platforms

- **Windows, Linux and Android:** Vulkan. The loader (`vulkan-1.dll`, `libvulkan.so.1`,
  `libvulkan.so`) is loaded at run time, so a program starts on a machine without Vulkan and gets
  `GpuError::NoLoader`. On Linux the drivers are glibc libraries: only a program linked against
  glibc reaches the GPU (`--cfg jfgui_glibc`, or a program with std; see
  [`jflib-build`](https://crates.io/crates/jflib-build)). Its frames reach the window as dma-bufs,
  with the same `Swapchain` type.
- **macOS and iOS:** Metal. Devices with Metal 3 (A14, M1 and newer) reach `Voll`, other devices and
  the iOS simulator `Basis`. Swapchains sit on a `CAMetalLayer` and present with `Fifo` only.
  Timestamps are not built on Metal yet.
- **The browser:** WebGPU, level `Basis`. Adapter and device arrive later, and `instance.prepare()`
  says when they are there. The worker the app runs in cannot wait for the GPU, so `wait` answers
  at once, `wait_idle` and `Queue::read_buffer` are `Unsupported`, and `Buffer::read` says
  `NotReady` until the submission has finished. Errors the browser finds come back from the next
  `submit`, `try_acquire` or `poll`.

On Vulkan a device can also have a queue of its own for compute and copies, when asked for:
`Adapter::create_device_with` with `DeviceOptions::compute_queue`, then `Device::compute_queue()`
gives `Option<&ComputeQueue>`. It is `None` where there is no such queue, so code that checks the
`Option` runs everywhere.

`jfgui_GPU=0` turns the GPU off. `jfgui_GPU_VALIDATE=1` turns on validation: on Vulkan the Khronos
layer with synchronization validation where the Vulkan SDK is installed, its messages on stderr; on
Apple systems Metal's API validation.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::gpu`. All of JFLib's crates share one
version and are released together. To show a scene of your own inside a window between widgets,
[`jflib-ui`](https://crates.io/crates/jflib-ui) has `ui.gpu_view`, which hands your content the
UI's own device.

## What's new in 0.2.0

jflib-gpu now runs on macOS and iOS through Metal, with the same types and methods as on Vulkan:
shaders stay SPIR-V and are translated by jflib-shader, and devices with Metal 3 reach level
`Voll`. On Vulkan a device can have a second queue for compute and copies when asked for
(`Adapter::create_device_with` with `DeviceOptions::compute_queue`), while `create_device` stays
as it was. `TextureView::usage` and `TextureView::same` are new, and validation with
`jfgui_GPU_VALIDATE` now also sees what shaders read and write through their bindings. Everything
is in the [changelog](https://docs.rs/crate/jflib-gpu/latest/source/CHANGELOG.md).

## Documentation

- [JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html): how an app is written
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#gpu): every public
  item of `gpu`, also on [docs.rs](https://docs.rs/jflib-gpu)
- [GPU.md](https://github.com/JFLibrary/JFLib/blob/main/docs/GPU.md) (in German): the API, the
  levels, the compute queue, the output of the UI on each system, validation, the environment
  variables and the known gaps

## License

MIT or Apache-2.0, at your choice.
