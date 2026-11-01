// t_compute -- gputest's compute, copy, timestamp and bindless tests (main.rs lists them)

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;

use crate::shaders::{CS_BINDLESS, CS_FILL};
use crate::{T, fill_ref, ms_since, out, run_wait, setup};

pub const MUL: u32 = 2_654_435_761;
pub const ADD: u32 = 12345;
/// elements after n, filled with GUARD first: the shader must leave them alone
pub const TAIL: u32 = 64;
pub const GUARD: u32 = 0xDEAD_BEEF;

/// the fill shader: bind group layout (one storage buffer) and pipeline (12 bytes of push
/// constants; the pipeline keeps its layout)
pub struct Fill {
    pub bgl: BindGroupLayout,
    pub pipe: ComputePipeline,
}

impl Fill {
    pub fn new(t: &mut T, dev: &Device) -> Option<Fill> {
        let m = t.res(ShaderModule::from_spirv(dev, CS_FILL), "fill shader")?;
        let bgl = t.res(
            dev.create_bind_group_layout(&[Binding { binding: 0, kind: BindingKind::StorageBuffer, stages: Stages::COMPUTE }]),
            "fill bind group layout",
        )?;
        let layout = t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: 12 }), "fill layout")?;
        let pipe = t.res(
            dev.create_compute_pipeline(&ComputePipelineDesc { layout: &layout, shader: ShaderStage::new(&m, "main") }),
            "fill pipeline",
        )?;
        Some(Fill { bgl, pipe })
    }

    pub fn group(&self, t: &mut T, dev: &Device, buf: &Buffer) -> Option<BindGroup> {
        t.res(
            dev.create_bind_group(&BindGroupDesc {
                layout: &self.bgl,
                entries: &[BindEntry { binding: 0, resource: Resource::Buffer { buffer: buf, offset: 0, size: 0 } }],
            }),
            "fill bind group",
        )
    }
}

pub fn storage(t: &mut T, dev: &Device, bytes: u64, usage: BufferUsage) -> Option<Buffer> {
    t.res(dev.create_buffer(&BufferDesc { size: bytes, usage: BufferUsage::STORAGE | usage, memory: MemoryKind::Device }),
          "storage buffer")
}

pub fn readback(t: &mut T, dev: &Device, bytes: u64) -> Option<Buffer> {
    t.res(dev.create_buffer(&BufferDesc { size: bytes, usage: BufferUsage::NONE, memory: MemoryKind::Readback }), "readback buffer")
}

/// the elements 0 .. n + TAIL of a fill against the reference (first difference reported)
pub fn verify_fill(t: &mut T, got: &[u32], n: u32, what: &str) -> bool {
    let bad = (0..n + TAIL).find(|&i| got[i as usize] != if i < n { fill_ref(i, MUL, ADD) } else { GUARD });
    match bad {
        None => t.ok(true, &format!("{what}: {n} values = f(i), the {TAIL} after them untouched")),
        Some(i) => {
            let want = if i < n { fill_ref(i, MUL, ADD) } else { GUARD };
            t.ok(false, &format!("{what}: element {i} = {:#x}, expected {want:#x}", got[i as usize]))
        }
    }
}

/// fill n values on the GPU, read them back and compare
pub fn fill_check(t: &mut T, dev: &Device, n: u32, what: &str) -> Option<f64> {
    let fp = Fill::new(t, dev)?;
    let bytes = (n + TAIL) as u64 * 4;
    let buf = storage(t, dev, bytes, BufferUsage::NONE)?;
    let rb = readback(t, dev, bytes)?;
    let bg = fp.group(t, dev, &buf)?;
    let t0 = plat::time_us();
    let mut e = t.res(dev.encoder(), "encoder")?;
    e.fill_buffer(&buf, 0, bytes, GUARD);
    e.barrier(Access::COPY_WRITE, Access::COMPUTE_WRITE);
    e.set_compute_pipeline(&fp.pipe);
    e.set_bind_group(0, &bg);
    e.push_constants(0, &[n, MUL, ADD]);
    e.dispatch(n.div_ceil(64), 1, 1);
    e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
    e.copy_buffer(&buf, 0, &rb, 0, bytes);
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    run_wait(t, dev, e.finish(), what)?;
    let mut got = vec![0u32; (n + TAIL) as usize];
    t.res(rb.read(0, &mut got), "read")?;
    let ms = ms_since(t0);
    verify_fill(t, &got, n, what);
    Some(ms)
}

pub fn compute(t: &mut T) {
    let Some(g) = setup(t) else { return };
    let dev = &g.dev;
    fill_check(t, dev, 1000, "compute 1000");
    if let Some(ms) = fill_check(t, dev, 1 << 20, "compute 1M") {
        out(&format!("  1M compute round trip (fill, dispatch, copy, submit, wait, read): {ms:.2} ms"));
    }
    // staged writes into a device-local buffer and the read_buffer path
    let Some(buf) = storage(t, dev, 4096, BufferUsage::NONE) else { return };
    let data: Vec<u32> = (0..1000).map(|i| i * 7 + 1).collect();
    t.res(dev.queue().write_buffer(&buf, 16, &data), "write_buffer");
    let mut back = vec![0u32; 1000];
    if t.res(dev.queue().read_buffer(&buf, 16, &mut back), "read_buffer").is_some() {
        t.ok(back == data, "write_buffer / read_buffer round trip (1000 u32 at offset 16)");
    }
    let before = dev.memory_reserved();
    drop(buf);
    t.res(dev.wait_idle(), "wait_idle");
    t.res(dev.poll(), "poll");
    out(&format!("  device memory in blocks: {} MiB", before >> 20));
}

pub fn copies(t: &mut T) {
    let Some(g) = setup(t) else { return };
    let dev = &g.dev;
    let q = dev.queue();
    // buffer -> texture -> buffer: 61 x 37 RGBA8, rows padded to 256 bytes up, 512 back
    let (w, h) = (61u32, 37u32);
    let usage = TextureUsage::COPY_SRC | TextureUsage::COPY_DST | TextureUsage::SAMPLED;
    let Some(tex) = t.res(dev.create_texture(&TextureDesc::d2(w, h, Format::Rgba8Unorm, usage)), "texture") else { return };
    let up_bpr = 256u32;
    let Some(up) = t.res(dev.create_buffer(&BufferDesc { size: (up_bpr * h) as u64, usage: BufferUsage::NONE,
                                                          memory: MemoryKind::Upload }), "upload buffer") else { return };
    let mut src = vec![0u8; (up_bpr * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let o = (y * up_bpr + x * 4) as usize;
            src[o..o + 4].copy_from_slice(&[(x * 3) as u8, (y * 5) as u8, (x ^ y) as u8, 255 - x as u8]);
        }
    }
    t.res(up.write(0, &src), "upload write");
    let down_bpr = 512u32;
    let Some(rb) = crate::t_compute::readback(t, dev, (down_bpr * h) as u64) else { return };
    let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
    e.texture_barrier(&tex, Layout::Undefined, Layout::CopyDst);
    e.copy_buffer_to_texture(&up, DataLayout { offset: 0, bytes_per_row: up_bpr, rows_per_image: 0 }, TextureRegion::mip(&tex, 0));
    e.texture_barrier(&tex, Layout::CopyDst, Layout::CopySrc);
    e.copy_texture_to_buffer(TextureRegion::mip(&tex, 0), &rb, DataLayout { offset: 0, bytes_per_row: down_bpr, rows_per_image: 0 });
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    if run_wait(t, dev, e.finish(), "texture round trip").is_some() {
        let mut got = vec![0u8; (down_bpr * h) as usize];
        t.res(rb.read(0, &mut got), "read");
        let bad = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).find(|&(x, y)| {
            let a = (y * up_bpr + x * 4) as usize;
            let b = (y * down_bpr + x * 4) as usize;
            src[a..a + 4] != got[b..b + 4]
        });
        t.ok(bad.is_none(), &format!("buffer -> texture -> buffer 61x37 RGBA8 (row pitch 256 / 512): first difference {bad:?}"));
    }
    // write_texture into every mip and layer of a 32 x 32 x 3 layers x 6 mips R32Uint array
    let (layers, mips) = (3u32, 6u32);
    let desc = TextureDesc { layers, mips, ..TextureDesc::d2(32, 32, Format::R32Uint, usage) };
    let Some(arr) = t.res(dev.create_texture(&desc), "array texture") else { return };
    let val = |m: u32, l: u32, i: u32| (m << 24) | (l << 16) | i;
    for m in 0..mips {
        let s = 32 >> m;
        for l in 0..layers {
            let data: Vec<u32> = (0..s * s).map(|i| val(m, l, i)).collect();
            let region = TextureRegion { texture: &arr, mip: m, origin: [0; 3], size: [s, s, 1], base_layer: l, layers: 1 };
            t.res(q.write_texture(&region, gpu::bytes_of(&data), DataLayout::default(), Layout::Undefined, Layout::CopySrc),
                  "write_texture");
        }
    }
    let total: u32 = (0..mips).map(|m| (32 >> m) * (32 >> m) * layers).sum();
    let Some(rb2) = crate::t_compute::readback(t, dev, total as u64 * 4) else { return };
    let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
    let mut off = 0u64;
    for m in 0..mips {
        let s = 32 >> m;
        e.copy_texture_to_buffer(TextureRegion::mip(&arr, m), &rb2, DataLayout { offset: off, bytes_per_row: 0, rows_per_image: 0 });
        off += (s * s * layers) as u64 * 4;
    }
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    if run_wait(t, dev, e.finish(), "mip / array read-back").is_some() {
        let mut got = vec![0u32; total as usize];
        t.res(rb2.read(0, &mut got), "read");
        let mut k = 0usize;
        let mut bad = None;
        for m in 0..mips {
            let s = 32 >> m;
            for l in 0..layers {
                for i in 0..s * s {
                    if bad.is_none() && got[k] != val(m, l, i) {
                        bad = Some((m, l, i, got[k]));
                    }
                    k += 1;
                }
            }
        }
        t.ok(bad.is_none(), &format!("write_texture: 6 mips x 3 layers R32Uint read back per mip (first difference {bad:?})"));
    }
    // a view of one mip and layer
    let v = arr.create_view(&ViewDesc { dim: ViewDim::D2, base_mip: 2, mips: 1, base_layer: 1, layers: 1 });
    t.ok(v.as_ref().is_ok_and(|v| v.size() == (8, 8)), "view of mip 2, layer 1: 8 x 8");
    let bad_view = arr.create_view(&ViewDesc { dim: ViewDim::D2, base_mip: 0, mips: 0, base_layer: 0, layers: 0 });
    t.ok(matches!(bad_view, Err(GpuError::Invalid(_))), "a 2D view of 3 layers: Invalid");
}

pub fn timestamps(t: &mut T) {
    let Some(g) = setup(t) else { return };
    let dev = &g.dev;
    if !dev.features().timestamps {
        out("SKIP: no timestamps on this device");
        return;
    }
    let Some(fp) = Fill::new(t, dev) else { return };
    let n = 1u32 << 20;
    let bytes = (n + TAIL) as u64 * 4;
    let Some(buf) = storage(t, dev, bytes, BufferUsage::NONE) else { return };
    let Some(rb) = readback(t, dev, bytes) else { return };
    let Some(bg) = fp.group(t, dev, &buf) else { return };
    let Some(ts) = t.res(dev.create_timestamps(4), "timestamps") else { return };
    let c0 = plat::time_us();
    let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
    e.reset_timestamps(&ts, 0, 4);
    e.timestamp(&ts, 0);
    e.set_compute_pipeline(&fp.pipe);
    e.set_bind_group(0, &bg);
    e.push_constants(0, &[n, MUL, ADD]);
    e.dispatch(n.div_ceil(64), 1, 1);
    e.timestamp(&ts, 1);
    let Some(c1) = t.res(e.finish(), "finish") else { return };
    let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
    e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
    e.timestamp(&ts, 2);
    e.copy_buffer(&buf, 0, &rb, 0, bytes);
    e.timestamp(&ts, 3);
    let Some(c2) = t.res(e.finish(), "finish") else { return };
    let Some(id1) = t.res(dev.queue().submit([c1]), "submit 1") else { return };
    let Some(id2) = t.res(dev.queue().submit([c2]), "submit 2") else { return };
    t.ok(id2 > id1, "submit ids increase");
    t.res(dev.wait(id2, 10_000_000_000), "wait");
    let cpu = ms_since(c0);
    let Some(ms) = t.res(ts.read_ms(0, 4), "read_ms") else { return };
    t.ok(ms.windows(2).all(|w| w[0] <= w[1]), &format!("monotonic over two submissions: {ms:.4?}"));
    let fill = ms[1] - ms[0];
    let copy = ms[3] - ms[2];
    t.ok(fill > 0.0 && fill < 100.0, &format!("1M fill on the GPU: {fill:.4} ms (0 .. 100)"));
    t.ok(copy > 0.0 && copy < 100.0, &format!("4 MiB copy on the GPU: {copy:.4} ms (0 .. 100)"));
    t.ok(ms[3] - ms[0] <= cpu + 1.0, &format!("GPU span {:.3} ms <= CPU span {cpu:.3} ms", ms[3] - ms[0]));
    let ticks = ts.read_ticks(0, 4);
    t.ok(ticks.is_ok_and(|v| v[0] > 0 && v[3] > v[0]), "raw ticks increase");
}

pub fn bindless(t: &mut T) {
    let Some(g) = setup(t) else { return };
    let dev = &g.dev;
    if !dev.features().bindless {
        out(&format!("SKIP: no bindless tables at level {:?}", dev.level()));
        t.ok(matches!(dev.create_bindless(&BindlessDesc { textures: 4, samplers: 0, buffers: 4, storage_textures: 0 }),
                      Err(GpuError::Unsupported(_))), "create_bindless on Basis: Unsupported");
        return;
    }
    let q = dev.queue();
    let Some(table) = t.res(dev.create_bindless(&BindlessDesc { textures: 64, samplers: 4, buffers: 64, storage_textures: 4 }),
                            "create_bindless") else { return };
    let k = 3u32;
    let mut bufs = Vec::new();
    let mut texs = Vec::new();
    for i in 0..k {
        let Some(b) = storage(t, dev, 256, BufferUsage::NONE) else { return };
        t.res(q.write_buffer(&b, 0, &[1000 + i]), "write_buffer");
        let Some(tx) = t.res(dev.create_texture(&TextureDesc::d2(1, 1, Format::R32Uint, TextureUsage::SAMPLED | TextureUsage::COPY_DST)),
                             "texture") else { return };
        let region = TextureRegion::mip(&tx, 0);
        t.res(q.write_texture(&region, gpu::bytes_of(&[2000 + i]), DataLayout::default(), Layout::Undefined, Layout::Sampled),
              "write_texture");
        t.res(table.set_buffer(i, &b, 0, 0), "set_buffer");
        t.res(table.set_texture(i, tx.view()), "set_texture");
        bufs.push(b);
        texs.push(tx);
    }
    if let Some(s) = t.res(dev.create_sampler(&SamplerDesc::LINEAR), "sampler") {
        t.res(table.set_sampler(0, &s), "set_sampler");
    }
    t.ok(matches!(table.set_buffer(64, &bufs[0], 0, 0), Err(GpuError::Invalid(_))), "slot outside the table: Invalid");
    let Some(m) = t.res(ShaderModule::from_spirv(dev, CS_BINDLESS), "bindless shader") else { return };
    let Some(obgl) = t.res(dev.create_bind_group_layout(&[Binding { binding: 0, kind: BindingKind::StorageBuffer,
                                                                    stages: Stages::COMPUTE }]), "layout") else { return };
    let Some(pl) = t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[table.layout(), &obgl], push_constants: 0 }),
                         "pipeline layout") else { return };
    let Some(pipe) = t.res(dev.create_compute_pipeline(&ComputePipelineDesc { layout: &pl, shader: ShaderStage::new(&m, "main") }),
                           "pipeline") else { return };
    let Some(obuf) = storage(t, dev, 64, BufferUsage::NONE) else { return };
    let Some(rb) = readback(t, dev, 64) else { return };
    let Some(obg) = t.res(dev.create_bind_group(&BindGroupDesc { layout: &obgl, entries: &[BindEntry { binding: 0,
        resource: Resource::Buffer { buffer: &obuf, offset: 0, size: 0 } }] }), "bind group") else { return };
    let run = |t: &mut T, what: &str| -> Option<Vec<u32>> {
        let mut e = t.res(dev.encoder(), "encoder")?;
        e.set_compute_pipeline(&pipe);
        e.set_bind_group(0, table.group());
        e.set_bind_group(1, &obg);
        e.dispatch(k, 1, 1);
        e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
        e.copy_buffer(&obuf, 0, &rb, 0, 64);
        e.barrier(Access::COPY_WRITE, Access::HOST_READ);
        run_wait(t, dev, e.finish(), what)?;
        let mut got = vec![0u32; (2 * k) as usize];
        t.res(rb.read(0, &mut got), "read")?;
        Some(got)
    };
    if let Some(got) = run(t, "bindless") {
        t.ok(got == [1000, 2000, 1001, 2001, 1002, 2002], &format!("buffers and textures through the table: {got:?}"));
    }
    // a slot changed after the GPU finished with the table
    let Some(nb) = storage(t, dev, 256, BufferUsage::NONE) else { return };
    t.res(q.write_buffer(&nb, 0, &[5000u32]), "write_buffer");
    t.res(table.set_buffer(1, &nb, 0, 0), "set_buffer again");
    drop(bufs);
    if let Some(got) = run(t, "bindless again") {
        t.ok(got == [1000, 2000, 5000, 2001, 1002, 2002], &format!("slot 1 replaced: {got:?}"));
    }
}
