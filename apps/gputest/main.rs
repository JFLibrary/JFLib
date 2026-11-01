// gputest -- self tests and measurements of lib/gpu (Vulkan on Windows; docs/GPU.md)
//
//   gputest --info                  adapters, driver, API version, level, features, limits
//   gputest --test NAME [--validate]
//        info        as --info, plus instance / device creation, a device limited to Basis
//        compute     a compute shader writes f(i) = (i * mul + add) ^ (i >> 5) into a storage
//                    buffer (1000 and 1M elements, the elements after n untouched); read back
//                    through a Readback buffer; queue.write_buffer / read_buffer
//        copies      buffer -> texture -> buffer (odd size, padded rows), write_texture into
//                    every mip and layer of a 2D array, read back per mip
//        graphics    two triangles (vertex pulling, index buffer, back-face culling, depth test)
//                    into an offscreen texture: pixels checked (orientation, depth, clear colour)
//        indirect    a compute shader writes the arguments: dispatch_indirect, then
//                    draw_indexed_indirect; on Voll also _multi and _count
//        timestamps  monotonic over two submissions, plausible against the CPU clock,
//                    NotReady before they are written
//        bindless    (Voll) storage buffers and textures indexed from one table; a slot changed
//                    after the GPU finished
//        window      a plat window with a swapchain: FIFO frames (cleared colour + triangles,
//                    the presented image read back), resize, minimise / restore, IMMEDIATE and
//                    MAILBOX frame times
//        errors      jfgui_GPU=0, a missing loader (jfgui_GPU_DLL), jfgui_GPU_ADAPTER without a
//                    match, invalid calls (Err, no crash), a lost device (Device::test_lose)
//                    and a new device after it
//        syncval     (--validate only) two copies into one buffer without a barrier: the
//                    validation layer must report the hazard (synchronization validation on)
//        spec        pipeline constants (t_spec.rs): lib/shader's spec_fill as SPIR-V 1.3
//                    (WorkgroupSize built-in) and 1.6 (LocalSizeId) with its defaults and with
//                    constants for every override (workgroup sizes 1 .. the limit) = its CPU
//                    reference; invalid constants and workgroup sizes are Err(Invalid)
//        textures    textures in shaders (t_texture.rs) against CPU references: OpImageFetch,
//                    sampling a 2D array / cube / cube array, storage textures (write, read,
//                    read-write, 8 formats), a texture + sampler in a fragment shader (nearest
//                    explicit and implicit LOD, bilinear, trilinear), depth comparison (also
//                    PCF), size / level queries, (Voll) Bindless textures and samplers
//        scene       the benchmark scene (t_scene.rs): move = move_all bit for bit, cull =
//                    cull_ref, an image of known cubes (orientation, lit colour, depth order,
//                    back faces culled) in both modes, indirect and single pictures compared
//        bench_smoke short runs of the scene's frame loop (offscreen, a window) with --verify
//        all         every test above
//   --validate      with VK_LAYER_KHRONOS_validation (jfgui_GPU_VALIDATE, synchronization
//                   validation); a validation error fails the test.  Without the layer: a note,
//                   the test runs as without --validate.
//   gputest --bench                 instance / device / swapchain creation, FIFO / MAILBOX /
//                                   IMMEDIATE frame times of a clear-and-present loop, the 1M
//                                   compute round trip
//   gputest --bench N [--res 1080p|4k|WxH] [--mode indirect|single] [--window | --fullscreen]
//                     [--present immediate|fifo|mailbox] [--frames F] [--warmup W] [--inflight K]
//                     [--verify] [--shot FILE.png] [--validate]
//                                   the benchmark scene with N cubes (measure.rs, scene.rs;
//                                   docs/GPU.md "Messszene"): CPU, GPU per pass and frame times,
//                                   median and p99, a RESULT line for scripts
//
// Without Vulkan or a usable GPU every test prints "SKIP" and passes (the error paths are
// tested with jfgui_GPU_DLL).  Exit code 1 when a check failed.  build_rs.py test gputest runs
// tests/*.uis (the script itself is ignored: --script).  The window is a plain plat window
// with its own event loop that is never given to plat::win_present, so the CPU presentation
// never paints over the swapchain (plat::win_native, win_api.rs).
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use gpu::*;

mod bench;
mod measure;
mod scene;
mod shaders;
mod t_compute;
mod t_render;
mod t_scene;
mod t_spec;
mod t_texture;
mod t_window;

rt::main!(run());

/// the test state: failures so far, validation wanted
pub struct T {
    pub fails: u32,
    pub validate: bool,
    /// validation errors a test provoked on purpose (syncval)
    pub expected: u32,
}

pub fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
    plat::write_stdout(b"\n");
}

impl T {
    /// a check: "  ok" or "FAIL" with its text
    pub fn ok(&mut self, cond: bool, what: &str) -> bool {
        if cond {
            out(&format!("  ok   {what}"));
        } else {
            self.fails += 1;
            out(&format!("FAIL {what}"));
        }
        cond
    }

    pub fn fail(&mut self, what: &str) {
        self.ok(false, what);
    }

    /// Some(value), or a failure with the error
    pub fn res<V>(&mut self, r: gpu::Result<V>, what: &str) -> Option<V> {
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                self.fail(&format!("{what}: {e}"));
                None
            }
        }
    }
}

/// instance, adapter and device of a test
pub struct Gpu {
    pub inst: Instance,
    pub adapter: Adapter,
    pub dev: Device,
}

/// the GPU for a test; None (SKIP, not a failure) without Vulkan or a usable GPU
pub fn setup(t: &mut T) -> Option<Gpu> {
    let inst = match Instance::with(&InstanceDesc { app_name: "gputest", validate: t.validate }) {
        Ok(i) => i,
        Err(e @ (GpuError::NoLoader | GpuError::NoAdapter | GpuError::Disabled)) => {
            out(&format!("SKIP: {e}"));
            return None;
        }
        Err(e) => {
            t.fail(&format!("Instance::with: {e}"));
            return None;
        }
    };
    if t.validate && !inst.validating() {
        out("note: validation layer not available, running without it");
    }
    let adapter = match inst.adapter() {
        Ok(a) => a,
        Err(e @ (GpuError::NoAdapter | GpuError::Unsupported(_))) => {
            out(&format!("SKIP: {e}"));
            return None;
        }
        Err(e) => {
            t.fail(&format!("Instance::adapter: {e}"));
            return None;
        }
    };
    let dev = t.res(adapter.create_device(&DeviceDesc::DEFAULT), "create_device")?;
    Some(Gpu { inst, adapter, dev })
}

/// submit one command buffer and wait for it (10 s)
pub fn run_wait(t: &mut T, dev: &Device, cmd: gpu::Result<CommandBuffer>, what: &str) -> Option<()> {
    let cmd = t.res(cmd, &format!("{what}: finish"))?;
    let id = t.res(dev.queue().submit([cmd]), &format!("{what}: submit"))?;
    let done = t.res(dev.wait(id, 10_000_000_000), &format!("{what}: wait"))?;
    if !done {
        t.fail(&format!("{what}: GPU did not finish within 10 s"));
        return None;
    }
    Some(())
}

pub fn ms_since(t0: u64) -> f64 {
    (plat::time_us() - t0) as f64 / 1000.0
}

/// the reference of the fill shader
pub fn fill_ref(i: u32, mul: u32, add: u32) -> u32 {
    i.wrapping_mul(mul).wrapping_add(add) ^ (i >> 5)
}

fn info(t: &mut T) {
    let t0 = plat::time_us();
    let inst = match Instance::with(&InstanceDesc { app_name: "gputest", validate: t.validate }) {
        Ok(i) => i,
        Err(e) => {
            out(&format!("SKIP: {e}"));
            return;
        }
    };
    out(&format!("instance: {:.1} ms (validation: {})", ms_since(t0), inst.validating()));
    let Some(list) = t.res(inst.adapters(), "adapters") else { return };
    t.ok(!list.is_empty(), "at least one adapter");
    for (i, a) in list.iter().enumerate() {
        let n = a.info();
        out(&format!(
            "adapter {i}: {} [{:?}, {} {:04x}:{:04x}] Vulkan {}.{}.{}, driver {} {} ({}), level {:?}{}",
            n.name,
            n.kind,
            n.vendor,
            n.vendor_id,
            n.device_id,
            n.api_version.0,
            n.api_version.1,
            n.api_version.2,
            n.driver,
            n.driver_info,
            n.driver_version,
            a.level(),
            n.missing.map_or(String::new(), |m| format!(", unusable: needs {m}"))
        ));
    }
    let Some(a) = t.res(inst.adapter(), "adapter") else { return };
    let t1 = plat::time_us();
    let Some(dev) = t.res(a.create_device(&DeviceDesc::DEFAULT), "create_device") else { return };
    out(&format!("device: {:.1} ms, {} at level {:?}", ms_since(t1), dev.info().name, dev.level()));
    t.ok(dev.level() == a.level(), "device level = the adapter's highest");
    let f = dev.features();
    let flags: [(&str, bool); 25] = [
        ("timestamps", f.timestamps),
        ("multi_draw_indirect", f.multi_draw_indirect),
        ("draw_indirect_first_instance", f.draw_indirect_first_instance),
        ("draw_indirect_count", f.draw_indirect_count),
        ("bindless", f.bindless),
        ("subgroups", f.subgroups),
        ("draw_parameters", f.draw_parameters),
        ("shader_f16", f.shader_f16),
        ("shader_f64", f.shader_f64),
        ("shader_i16", f.shader_i16),
        ("shader_i64", f.shader_i64),
        ("shader_i8", f.shader_i8),
        ("storage_16bit", f.storage_16bit),
        ("storage_8bit", f.storage_8bit),
        ("texture_bc", f.texture_bc),
        ("anisotropy", f.anisotropy),
        ("depth_clamp", f.depth_clamp),
        ("vertex_stores", f.vertex_stores),
        ("fragment_stores", f.fragment_stores),
        ("storage_write_without_format", f.storage_write_without_format),
        ("buffer_address", f.buffer_address),
        ("swapchain", f.swapchain),
        ("mesh_shader (reported)", f.mesh_shader),
        ("ray_query (reported)", f.ray_query),
        ("ray_pipeline (reported)", f.ray_pipeline),
    ];
    let on: Vec<&str> = flags.iter().filter(|x| x.1).map(|x| x.0).collect();
    let off: Vec<&str> = flags.iter().filter(|x| !x.1).map(|x| x.0).collect();
    out(&format!("features: {}", on.join(" ")));
    out(&format!("missing:  {}", if off.is_empty() { String::from("-") } else { off.join(" ") }));
    let l = dev.limits();
    out(&format!(
        "limits: texture 2D {} / 3D {} / layers {}, storage binding {} MiB, uniform binding {} KiB, push constants {} B, \
         bind groups {}",
        l.max_texture_2d,
        l.max_texture_3d,
        l.max_texture_layers,
        l.max_storage_binding >> 20,
        l.max_uniform_binding >> 10,
        l.max_push_constants,
        l.max_bind_groups
    ));
    out(&format!(
        "        workgroup {:?} ({} threads, {} KiB shared), workgroups {:?}, subgroup {}, colour targets {}, \
         indirect count {}",
        l.max_compute_workgroup_size,
        l.max_compute_invocations,
        l.max_compute_shared_memory >> 10,
        l.max_compute_workgroups,
        l.subgroup_size,
        l.max_color_targets,
        l.max_draw_indirect_count
    ));
    out(&format!(
        "        alignment uniform {} / storage {}, anisotropy {}, timestamp period {} ns, bindless textures {} / \
         samplers {} / buffers {} / storage textures {}, device memory {} MiB",
        l.min_uniform_offset_alignment,
        l.min_storage_offset_alignment,
        l.max_anisotropy,
        l.timestamp_period_ns,
        l.max_bindless_textures,
        l.max_bindless_samplers,
        l.max_bindless_buffers,
        l.max_bindless_storage_textures,
        l.device_memory >> 20
    ));
    t.ok(l.max_push_constants >= PUSH_CONSTANTS_MAX, "push constants >= PUSH_CONSTANTS_MAX");
    t.ok(l.max_bind_groups == BIND_GROUPS_MAX, "4 bind groups");
    drop(dev);
    // a device limited to Basis: the Voll features off, compute still works
    let Some(basis) = t.res(a.create_device(&DeviceDesc { max_level: Level::Basis }), "Basis device") else { return };
    let bf = basis.features();
    t.ok(basis.level() == Level::Basis && !bf.bindless && !bf.draw_indirect_count && !bf.multi_draw_indirect,
         "a device limited to Basis: no bindless, multi-draw or indirect count");
    let bl = basis.create_bindless(&BindlessDesc { textures: 4, samplers: 0, buffers: 4, storage_textures: 0 });
    t.ok(matches!(bl, Err(GpuError::Unsupported(_))), "Basis: create_bindless = Unsupported");
    t_compute::fill_check(t, &basis, 1000, "Basis device: compute");
}

fn errors(t: &mut T) {
    // jfgui_GPU=0
    plat::setenv("jfgui_GPU", Some("0"));
    let r = Instance::new();
    plat::setenv("jfgui_GPU", None);
    t.ok(matches!(r, Err(GpuError::Disabled)), "jfgui_GPU=0: Instance::new = Err(Disabled)");
    // a missing loader, by name and by path
    for name in ["jfgui-no-such-vulkan.dll", "C:\\jfgui-no-such-dir\\vulkan-1.dll"] {
        plat::setenv("jfgui_GPU_DLL", Some(name));
        let r = Instance::new();
        plat::setenv("jfgui_GPU_DLL", None);
        t.ok(matches!(r, Err(GpuError::NoLoader)), &format!("jfgui_GPU_DLL={name}: Err(NoLoader)"));
    }
    let Some(g) = setup(t) else { return };
    // adapter selection
    for (sel, want_ok) in [("no-such-gpu-xyz", false), ("99", false), ("0", true)] {
        plat::setenv("jfgui_GPU_ADAPTER", Some(sel));
        let r = g.inst.adapter();
        plat::setenv("jfgui_GPU_ADAPTER", None);
        let ok = if want_ok { r.is_ok() } else { matches!(r, Err(GpuError::NoAdapter)) };
        t.ok(ok, &format!("jfgui_GPU_ADAPTER={sel}: {}", if want_ok { "Ok" } else { "Err(NoAdapter)" }));
    }
    let dev = &g.dev;
    let inv = |r: gpu::Result<()>| matches!(r, Err(GpuError::Invalid(_)));
    t.ok(inv(dev.create_buffer(&BufferDesc { size: 0, usage: BufferUsage::STORAGE, memory: MemoryKind::Device }).map(|_| ())),
         "buffer size 0: Invalid");
    let huge = dev.create_buffer(&BufferDesc { size: 1 << 40, usage: BufferUsage::STORAGE, memory: MemoryKind::Device });
    t.ok(matches!(huge, Err(GpuError::OutOfMemory)), &format!("1 TiB buffer: {:?}", huge.as_ref().err()));
    if let Some(b) = t.res(dev.create_buffer(&BufferDesc { size: 64, usage: BufferUsage::STORAGE, memory: MemoryKind::Device }),
                           "buffer")
    {
        t.ok(inv(b.write(0, &[1u32])), "write to a Device buffer: Invalid");
        t.ok(inv(dev.queue().write_buffer(&b, 60, &[1u32, 2])), "write_buffer past the end: Invalid");
        let rb = dev.create_buffer(&BufferDesc { size: 64, usage: BufferUsage::NONE, memory: MemoryKind::Readback });
        if let Some(rb) = t.res(rb, "readback buffer") {
            t.ok(inv(rb.read(32, &mut [0u32; 9])), "read past the end: Invalid");
            if let Some(mut e) = t.res(dev.encoder(), "encoder") {
                e.copy_buffer(&b, 0, &rb, 32, 64);
                t.ok(matches!(e.finish(), Err(GpuError::Invalid(_))), "copy_buffer out of range: finish = Invalid");
            }
        }
    }
    t.ok(matches!(ShaderModule::from_spirv(dev, &[1, 2, 3]), Err(GpuError::Invalid(_))), "not SPIR-V: Invalid");
    t.ok(matches!(ShaderModule::from_spirv(dev, &[0x0723_0203, 0x0001_0700, 0, 1, 0]), Err(GpuError::Unsupported(_))),
         "SPIR-V 1.7: Unsupported");
    if let Some(mut e) = t.res(dev.encoder(), "encoder") {
        e.dispatch(1, 1, 1);
        t.ok(matches!(e.finish(), Err(GpuError::Invalid(_))), "dispatch without pipeline: Invalid");
    }
    if let Some(mut e) = t.res(dev.encoder(), "encoder") {
        e.draw(3, 1, 0, 0);
        t.ok(matches!(e.finish(), Err(GpuError::Invalid(_))), "draw outside a render pass: Invalid");
    }
    if let Some(fp) = t_compute::Fill::new(t, dev) {
        if let Some(mut e) = t.res(dev.encoder(), "encoder") {
            e.set_compute_pipeline(&fp.pipe);
            e.push_constants(8, &[1u32, 2]);
            t.ok(matches!(e.finish(), Err(GpuError::Invalid(_))), "push constants past the layout: Invalid");
        }
        let other = dev.create_bind_group_layout(&[Binding { binding: 3, kind: BindingKind::UniformBuffer, stages: Stages::ALL }]);
        let ub = dev.create_buffer(&BufferDesc { size: 256, usage: BufferUsage::UNIFORM, memory: MemoryKind::Upload });
        if let (Some(other), Some(ub)) = (t.res(other, "layout"), t.res(ub, "uniform buffer")) {
            let bg = dev.create_bind_group(&BindGroupDesc {
                layout: &other,
                entries: &[BindEntry { binding: 3, resource: Resource::Buffer { buffer: &ub, offset: 0, size: 0 } }],
            });
            if let Some(bg) = t.res(bg, "bind group") {
                let mut e = dev.encoder().ok();
                if let Some(e) = e.as_mut() {
                    e.set_compute_pipeline(&fp.pipe);
                    e.set_bind_group(0, &bg);
                }
                t.ok(e.is_some_and(|e| matches!(e.finish(), Err(GpuError::Invalid(_)))), "bind group of another layout: Invalid");
            }
            let wrong = dev.create_bind_group(&BindGroupDesc {
                layout: &other,
                entries: &[BindEntry { binding: 3, resource: Resource::Buffer { buffer: &ub, offset: 4, size: 0 } }],
            });
            t.ok(matches!(wrong, Err(GpuError::Invalid(_))), "misaligned uniform offset: Invalid");
        }
    }
    let tex = dev.create_texture(&TextureDesc { mips: 8, ..TextureDesc::d2(16, 16, Format::Rgba8Unorm, TextureUsage::SAMPLED) });
    t.ok(matches!(tex, Err(GpuError::Invalid(_))), "more mips than the size has: Invalid");
    t.ok(matches!(dev.wait(SubmitId(1 << 40), 0), Err(GpuError::Invalid(_))), "wait for a submission not made: Invalid");
    if dev.features().timestamps {
        if let Some(ts) = t.res(dev.create_timestamps(2), "timestamps") {
            t.ok(matches!(ts.read_ms(0, 2), Err(GpuError::NotReady)), "unwritten timestamps: NotReady");
        }
    }
    // a lost device: every call fails with DeviceLost; a new device works
    let b = dev.create_buffer(&BufferDesc { size: 256, usage: BufferUsage::STORAGE, memory: MemoryKind::Device });
    dev.test_lose();
    t.ok(dev.is_lost(), "test_lose: is_lost");
    t.ok(matches!(dev.encoder().map(|_| ()), Err(GpuError::DeviceLost)), "lost: encoder = Err(DeviceLost)");
    t.ok(matches!(dev.create_buffer(&BufferDesc { size: 4, usage: BufferUsage::NONE, memory: MemoryKind::Device }).map(|_| ()),
                  Err(GpuError::DeviceLost)), "lost: create_buffer = Err(DeviceLost)");
    t.ok(matches!(dev.queue().flush(), Err(GpuError::DeviceLost)), "lost: submit = Err(DeviceLost)");
    t.ok(matches!(dev.wait_idle(), Err(GpuError::DeviceLost)), "lost: wait = Err(DeviceLost)");
    drop(b);
    let Gpu { inst, adapter, dev } = g;
    drop(dev);
    let Some(dev2) = t.res(adapter.create_device(&DeviceDesc::DEFAULT), "a new device after the lost one") else { return };
    t.ok(!dev2.is_lost(), "the new device is not lost");
    t_compute::fill_check(t, &dev2, 5000, "new device: compute");
    drop(inst);
}

fn syncval(t: &mut T) {
    if !t.validate {
        out("SKIP: needs --validate");
        return;
    }
    let Some(g) = setup(t) else { return };
    if !g.inst.validating() {
        out("SKIP: no validation layer");
        return;
    }
    let dev = &g.dev;
    let Some(src) = t_compute::storage(t, dev, 1024, BufferUsage::NONE) else { return };
    let Some(dst) = t_compute::storage(t, dev, 1024, BufferUsage::NONE) else { return };
    let (e0, _) = gpu::validation_messages();
    let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
    e.copy_buffer(&src, 0, &dst, 0, 1024);
    e.fill_buffer(&dst, 0, 1024, 7); // no barrier: write after write
    run_wait(t, dev, e.finish(), "hazard");
    let (e1, _) = gpu::validation_messages();
    t.expected += e1 - e0;
    out(&format!("  (the {} validation error(s) above are expected)", e1 - e0));
    t.ok(e1 > e0, "synchronization validation reports a missing barrier");
}

fn run() -> i32 {
    let args = plat::args();
    let validate = args.iter().any(|a| a == "--validate");
    let mut t = T { fails: 0, validate, expected: 0 };
    if validate {
        plat::setenv("jfgui_GPU_VALIDATE", Some("1"));
    }
    if args.iter().any(|a| a == "--bench") {
        if let Some(n) = plat::arg_value("--bench").and_then(|v| v.parse::<u32>().ok()).filter(|&n| n > 0) {
            return measure::bench_scene(n, validate);
        }
        bench::bench();
        return 0;
    }
    let test = if args.iter().any(|a| a == "--info") { Some(String::from("info")) } else { plat::arg_value("--test") };
    let Some(test) = test else {
        plat::write_stderr(b"gputest: --info, --test NAME [--validate], --bench or --bench N (see apps/gputest/main.rs)\n");
        return 2;
    };
    let all = test == "all";
    let tests: [(&str, fn(&mut T)); 14] = [
        ("info", info),
        ("compute", t_compute::compute),
        ("copies", t_compute::copies),
        ("graphics", t_render::graphics),
        ("indirect", t_render::indirect),
        ("timestamps", t_compute::timestamps),
        ("bindless", t_compute::bindless),
        ("spec", t_spec::spec),
        ("textures", t_texture::textures),
        ("window", t_window::window),
        ("errors", errors),
        ("syncval", syncval),
        ("scene", t_scene::scene),
        ("bench_smoke", t_scene::bench_smoke),
    ];
    let mut ran = false;
    for (name, f) in tests {
        if all || test == name {
            out(&format!("== {name}{}", if validate { " (validation)" } else { "" }));
            f(&mut t);
            ran = true;
        }
    }
    if !ran {
        plat::write_stderr(format!("gputest: no test {test}\n").as_bytes());
        return 2;
    }
    if validate {
        let (e, w) = gpu::validation_messages();
        let e = e - t.expected;
        t.ok(e == 0, &format!("validation layer: {e} errors (besides the {} provoked), {w} warnings", t.expected));
    }
    if t.fails > 0 {
        out(&format!("{} check(s) failed", t.fails));
        1
    } else {
        out("all checks passed");
        0
    }
}
