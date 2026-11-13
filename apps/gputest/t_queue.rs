// t_queue -- (G5d) gputest --test queues: the compute queue (Device::compute_queue; docs/GPU.md "Rechen-Warteschlange").
// Rounds of a compute shader on one buffer alternate between the two queues, each waiting for the round before
// through the timeline (submit_after), all submitted before the CPU waits once: the values against the CPU.  The
// first round reads what queue.write_buffer staged (the compute queue's submission flushes it on the device queue and
// waits).  A storage texture written on the compute queue (its layout changed there) and read on the device queue.
// Waiting and is_done for the compute queue's ids, wait_idle over both, a buffer dropped while the compute queue uses
// it, barriers with graphics stages on the compute queue, timestamps there.  What is refused: command buffers of the
// other family, a render pass on the compute queue, waiting for a submission not made.  The queue only when asked
// for (Adapter::create_device_with, DeviceOptions::compute_queue): create_device has none; jfgui_GPU_COMPUTE=0: none
// even when asked for (the fallback).  With --validate and the layer: two dispatches on one buffer without a barrier in
// a command buffer of the compute queue must be reported by synchronization validation (exactly one error, provoked
// on purpose); a missing wait between the queues it does not see, so the values tell (a long dispatch, then the other
// queue on its buffer, with and without submit_after).  A rough look at whether the two queues run at once on this GPU
// (two long dispatches, apart and together).

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;

use crate::t_compute::{readback, storage};
use crate::{T, ms_since, out, setup};

const SRC: &str = "
struct P { n: u32, k: u32 }
@group(0) @binding(0) var<storage, read_write> b: array<u32>;
var<immediate> pc: P;

@compute @workgroup_size(64)
fn step(@builtin(global_invocation_id) id: vec3u) {
    if id.x < pc.n {
        b[id.x] = b[id.x] * 3u + pc.k;
    }
}

@compute @workgroup_size(64)
fn spin(@builtin(global_invocation_id) id: vec3u) {
    if id.x < pc.n {
        var v = b[id.x];
        for (var j = 0u; j < pc.k; j++) {
            v = v * 1664525u + 1013904223u;
        }
        b[id.x] = v;
    }
}";

const TEX: &str = "
@group(0) @binding(0) var img: texture_storage_2d<r32float, write>;
@compute @workgroup_size(8, 8)
fn paint(@builtin(global_invocation_id) id: vec3u) {
    if id.x < 40u && id.y < 24u {
        textureStore(img, vec2i(id.xy), vec4f(f32(id.x * 1000u + id.y), 0.0, 0.0, 1.0));
    }
}";

const LOOK: &str = "
@group(0) @binding(0) var img: texture_2d<f32>;
@group(0) @binding(1) var<storage, read_write> out: array<f32>;
@compute @workgroup_size(8, 8)
fn look(@builtin(global_invocation_id) id: vec3u) {
    if id.x < 40u && id.y < 24u {
        out[id.y * 40u + id.x] = textureLoad(img, vec2i(id.xy), 0).x;
    }
}";

/// a compute pipeline of one entry point of WGSL with the bindings given
fn pipeline(t: &mut T, dev: &Device, src: &str, entry: &str, kinds: &[BindingKind], push: u32) -> Option<(BindGroupLayout, ComputePipeline)> {
    let words = match wgsl::compile(src, &wgsl::CompileOptions::new(shader::BASIS)) {
        Ok(o) => o.entry(entry)?.words.clone(),
        Err(e) => {
            t.fail(&format!("{entry}: {:?}", e.first().map(|x| format!("{x}"))));
            return None;
        }
    };
    let m = t.res(ShaderModule::from_spirv(dev, &words), entry)?;
    let b: Vec<Binding> = kinds.iter().enumerate().map(|(i, &kind)| Binding { binding: i as u32, kind, stages: Stages::COMPUTE }).collect();
    let bgl = t.res(dev.create_bind_group_layout(&b), entry)?;
    let pl = t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: push }), entry)?;
    let p = t.res(dev.create_compute_pipeline(&ComputePipelineDesc { layout: &pl, shader: ShaderStage::new(&m, entry) }), entry)?;
    Some((bgl, p))
}

fn group<'a>(t: &mut T, dev: &Device, bgl: &BindGroupLayout, res: &[Resource<'a>]) -> Option<BindGroup> {
    let e: Vec<BindEntry> = res.iter().enumerate().map(|(i, &resource)| BindEntry { binding: i as u32, resource }).collect();
    t.res(dev.create_bind_group(&BindGroupDesc { layout: bgl, entries: &e }), "bind group")
}

pub fn queues(t: &mut T) {
    let Some(g) = setup(t) else { return };
    // without asking: one queue, as in 0.1.0
    t.ok(g.dev.compute_queue().is_none(), "create_device (no DeviceOptions::compute_queue): Device::compute_queue is None");
    let asked = DeviceOptions::DEFAULT.with_compute_queue();
    if let Some(dev) = t.res(g.adapter.create_device_with(&DeviceDesc::DEFAULT, &asked), "device with a compute queue asked for") {
        match dev.compute_queue() {
            Some(cq) => {
                out("  the device has a compute queue (a family without graphics)");
                let _ = with_queue(t, &dev, cq);
            }
            None => out("  note: this device has no family without graphics: no compute queue (everything on Device::queue)"),
        }
    }
    // the fallback: jfgui_GPU_COMPUTE=0, even when asked for
    plat::setenv("jfgui_GPU_COMPUTE", Some("0"));
    let d2 = t.res(g.adapter.create_device_with(&DeviceDesc::DEFAULT, &asked), "device without a compute queue");
    plat::setenv("jfgui_GPU_COMPUTE", None);
    if let Some(d2) = d2 {
        t.ok(d2.compute_queue().is_none(), "jfgui_GPU_COMPUTE=0 with the queue asked for: Device::compute_queue is None");
        let _ = rounds(t, &d2, None, true, "one queue");
    }
}

/// rounds 1 ..= 8 of b = 3 b + k, odd ones on the compute queue (cq), even ones on the device queue, each after the
/// one before (wait false: rounds 4 and 5 without); the values against the CPU when waited for
fn rounds(t: &mut T, dev: &Device, cq: Option<&ComputeQueue>, wait: bool, what: &str) -> Option<()> {
    let n = 1u32 << 16;
    let (bgl, step) = pipeline(t, dev, SRC, "step", &[BindingKind::StorageBuffer], 8)?;
    let buf = storage(t, dev, n as u64 * 4, BufferUsage::NONE)?;
    let rb = readback(t, dev, n as u64 * 4)?;
    let bg = group(t, dev, &bgl, &[Resource::Buffer { buffer: &buf, offset: 0, size: 0 }])?;
    // what the first round reads: staged by queue.write_buffer, not submitted yet
    let start: Vec<u32> = (0..n).map(|i| i ^ 0x5a5a).collect();
    t.res(dev.queue().write_buffer(&buf, 0, &start), "write_buffer")?;
    let mut last: Option<SubmitId> = None;
    for k in 1..=8u32 {
        let on_compute = k % 2 == 1 && cq.is_some();
        let mut e = match (on_compute, cq) {
            (true, Some(q)) => t.res(q.encoder(), "compute encoder")?,
            _ => t.res(dev.encoder(), "encoder")?,
        };
        e.set_compute_pipeline(&step);
        e.set_bind_group(0, &bg);
        e.push_constants(0, &[n, k]);
        e.dispatch(n.div_ceil(64), 1, 1);
        if k == 8 {
            e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
            e.copy_buffer(&buf, 0, &rb, 0, n as u64 * 4);
            e.barrier(Access::COPY_WRITE, Access::HOST_READ);
        }
        let cmd = t.res(e.finish(), "finish")?;
        let after: Vec<SubmitId> = if !wait && k == 5 { Vec::new() } else { last.into_iter().collect() };
        let id = match (on_compute, cq) {
            (true, Some(q)) => t.res(q.submit_after(&after, [cmd]), "compute submit_after")?,
            _ => t.res(dev.queue().submit_after(&after, [cmd]), "submit_after")?,
        };
        last = Some(id);
    }
    let last = last?;
    t.res(dev.wait(last, 10_000_000_000), "wait")?;
    t.res(dev.wait_idle(), "wait_idle")?;
    if !wait {
        return Some(());
    }
    let mut got = vec![0u32; n as usize];
    t.res(rb.read(0, &mut got), "read")?;
    let mut want = start.clone();
    for k in 1..=8u32 {
        for v in want.iter_mut() {
            *v = v.wrapping_mul(3).wrapping_add(k);
        }
    }
    let bad = got.iter().zip(&want).filter(|(a, b)| a != b).count();
    t.ok(bad == 0, &format!("{what}: 8 rounds on one buffer{}, each after the one before, the first reading what write_buffer staged: {bad} of {n} values wrong",
                            if cq.is_some() { " alternating between the queues" } else { "" }));
    Some(())
}

fn with_queue(t: &mut T, dev: &Device, cq: &ComputeQueue) -> Option<()> {
    rounds(t, dev, Some(cq), true, "two queues")?;
    // waiting for the compute queue's ids
    let (bgl, step) = pipeline(t, dev, SRC, "step", &[BindingKind::StorageBuffer], 8)?;
    let buf = storage(t, dev, 1 << 20, BufferUsage::NONE)?;
    let bg = group(t, dev, &bgl, &[Resource::Buffer { buffer: &buf, offset: 0, size: 0 }])?;
    let mut e = t.res(cq.encoder(), "compute encoder")?;
    e.set_compute_pipeline(&step);
    e.set_bind_group(0, &bg);
    e.push_constants(0, &[1u32 << 18, 1]);
    e.dispatch(1 << 12, 1, 1);
    // graphics stages on the compute queue fall away (the validation layer would report them)
    e.barrier(Access::COMPUTE_WRITE | Access::GRAPHICS_READ | Access::INDEX_VERTEX, Access::COMPUTE_READ | Access::GRAPHICS_READ | Access::INDIRECT);
    let cmd = t.res(e.finish(), "finish")?;
    let id = t.res(cq.submit([cmd]), "compute submit")?;
    // a buffer the submission uses, dropped before the GPU is done: it waits for the compute queue's timeline
    drop(bg);
    drop(buf);
    let done = t.res(dev.wait(id, 10_000_000_000), "wait for the compute queue")?;
    t.ok(done && dev.is_done(id), &format!("Device::wait and is_done take the compute queue's id ({:#x})", id.0));
    // a storage texture written on the compute queue, read on the device queue
    let tex = t.res(dev.create_texture(&TextureDesc::d2(40, 24, Format::R32Float, TextureUsage::STORAGE | TextureUsage::SAMPLED)), "texture")?;
    let (pbgl, paint) = pipeline(t, dev, TEX, "paint", &[BindingKind::StorageTexture], 0)?;
    let (lbgl, look) = pipeline(t, dev, LOOK, "look", &[BindingKind::Texture, BindingKind::StorageBuffer], 0)?;
    let pg = group(t, dev, &pbgl, &[Resource::StorageTexture(tex.view())])?;
    let outb = storage(t, dev, 40 * 24 * 4, BufferUsage::NONE)?;
    let rb = readback(t, dev, 40 * 24 * 4)?;
    let lg = group(t, dev, &lbgl, &[Resource::Texture(tex.view()), Resource::Buffer { buffer: &outb, offset: 0, size: 0 }])?;
    let mut e = t.res(cq.encoder(), "compute encoder")?;
    e.texture_barrier(&tex, Layout::Undefined, Layout::General);
    e.set_compute_pipeline(&paint);
    e.set_bind_group(0, &pg);
    e.dispatch(5, 3, 1);
    e.texture_barrier(&tex, Layout::General, Layout::Sampled);
    let cmd = t.res(e.finish(), "finish")?;
    let painted = t.res(cq.submit([cmd]), "compute submit")?;
    let mut e = t.res(dev.encoder(), "encoder")?;
    e.set_compute_pipeline(&look);
    e.set_bind_group(0, &lg);
    e.dispatch(5, 3, 1);
    e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
    e.copy_buffer(&outb, 0, &rb, 0, 40 * 24 * 4);
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    let cmd = t.res(e.finish(), "finish")?;
    let id = t.res(dev.queue().submit_after(&[painted], [cmd]), "submit_after")?;
    t.res(dev.wait(id, 10_000_000_000), "wait")?;
    let mut got = vec![0f32; 40 * 24];
    t.res(rb.read(0, &mut got), "read")?;
    let bad = (0..40 * 24).filter(|&i| got[i] != ((i % 40) * 1000 + i / 40) as f32).count();
    t.ok(bad == 0, &format!("a storage texture written on the compute queue (its layouts changed there), read on the device queue after it: {bad} of 960 texels wrong"));
    // timestamps on the compute queue, where its family has them
    if dev.features().timestamps {
        let ts = t.res(dev.create_timestamps(2), "timestamps")?;
        let mut e = t.res(cq.encoder(), "compute encoder")?;
        e.reset_timestamps(&ts, 0, 2);
        e.timestamp(&ts, 0);
        e.timestamp(&ts, 1);
        match e.finish() {
            Ok(cmd) => {
                let id = t.res(cq.submit([cmd]), "compute submit")?;
                t.res(dev.wait(id, 10_000_000_000), "wait")?;
                let v = ts.read_ms(0, 2);
                t.ok(v.as_ref().is_ok_and(|v| v[1] >= v[0]), &format!("timestamps on the compute queue: {:?}", v.map(|v| v[1] - v[0])));
            }
            Err(e) => {
                t.ok(matches!(e, GpuError::Invalid(_)), &format!("timestamps on the compute queue: its family has none ({e})"));
            }
        };
    }
    // what is refused
    let inv = |r: gpu::Result<SubmitId>| matches!(r, Err(GpuError::Invalid(_)));
    let e = t.res(dev.encoder(), "encoder")?;
    let main_cmd = t.res(e.finish(), "finish")?;
    let a = inv(cq.submit([main_cmd]));
    let e = t.res(cq.encoder(), "compute encoder")?;
    let comp_cmd = t.res(e.finish(), "finish")?;
    let b = inv(dev.queue().submit([comp_cmd]));
    let e = t.res(cq.encoder(), "compute encoder")?;
    let comp_cmd = t.res(e.finish(), "finish")?;
    let c = inv(cq.submit_after(&[SubmitId(id.0 + 100)], [comp_cmd]));
    let target = t.res(dev.create_texture(&TextureDesc::d2(8, 8, Format::Rgba8Unorm, TextureUsage::COLOR_TARGET)), "target")?;
    let mut e = t.res(cq.encoder(), "compute encoder")?;
    e.begin_render(&RenderPassDesc { color: &[ColorAttachment { view: target.view(), load: Load::Clear, clear: [0.0; 4], store: true, before: Layout::Undefined,
                                                                 after: Layout::ColorTarget }], depth: None });
    e.end_render();
    let d = matches!(e.finish(), Err(GpuError::Invalid(_)));
    t.ok(a && b && c && d, &format!("refused: a device queue's buffer on the compute queue {a}, the other way {b}, waiting for a submission not made {c}, a render pass on the compute queue {d}"));
    t.res(dev.wait_idle(), "wait_idle")?;
    // a missing synchronization: (1) a missing barrier between two dispatches in a command buffer of the compute
    // queue, which synchronization validation must report; (2) a missing wait between the queues: a long dispatch on
    // the device queue, then the compute queue on its buffer, with and without submit_after -- the layer (1.4.363)
    // reports no hazard between submissions (not even on one queue), so the values tell: sampled against the CPU
    if t.validate && dev.validating() {
        let big = storage(t, dev, 1 << 20, BufferUsage::NONE)?;
        let gb = group(t, dev, &bgl, &[Resource::Buffer { buffer: &big, offset: 0, size: 0 }])?;
        let e0 = t.provoke("queues: two dispatches writing one buffer on the compute queue, no barrier between them");
        let mut e = t.res(cq.encoder(), "compute encoder")?;
        e.set_compute_pipeline(&step);
        e.set_bind_group(0, &gb);
        e.push_constants(0, &[1u32 << 18, 1]);
        e.dispatch(1 << 12, 1, 1);
        e.dispatch(1 << 12, 1, 1); // no barrier: write after write, in the shader
        t.provoked(e0, "a missing barrier between two dispatches on the compute queue");
        let cmd = t.res(e.finish(), "finish")?;
        let id = t.res(cq.submit([cmd]), "compute submit")?;
        t.res(dev.wait(id, 10_000_000_000), "wait")?;
    }
    let (sbgl, spin) = pipeline(t, dev, SRC, "spin", &[BindingKind::StorageBuffer], 8)?;
    let n = 1u32 << 20;
    let big = storage(t, dev, n as u64 * 4, BufferUsage::NONE)?;
    let rb = readback(t, dev, n as u64 * 4)?;
    let gs = group(t, dev, &sbgl, &[Resource::Buffer { buffer: &big, offset: 0, size: 0 }])?;
    let gb = group(t, dev, &bgl, &[Resource::Buffer { buffer: &big, offset: 0, size: 0 }])?;
    let steps = 20_000u32;
    for wait in [true, false] {
        let start: Vec<u32> = (0..n).collect();
        t.res(dev.queue().write_buffer(&big, 0, &start), "write_buffer")?;
        let (e0, _) = gpu::validation_messages();
        let mut e = t.res(dev.encoder(), "encoder")?;
        e.set_compute_pipeline(&spin);
        e.set_bind_group(0, &gs);
        e.push_constants(0, &[n, steps]);
        e.dispatch(n.div_ceil(64), 1, 1);
        let cmd = t.res(e.finish(), "finish")?;
        let first = t.res(dev.queue().submit([cmd]), "submit")?;
        let mut e = t.res(cq.encoder(), "compute encoder")?;
        e.set_compute_pipeline(&step);
        e.set_bind_group(0, &gb);
        e.push_constants(0, &[n, 1u32]);
        e.dispatch(n.div_ceil(64), 1, 1);
        let cmd = t.res(e.finish(), "finish")?;
        let second = if wait { t.res(cq.submit_after(&[first], [cmd]), "submit_after")? } else { t.res(cq.submit([cmd]), "submit")? };
        let running = !dev.is_done(first);
        t.res(dev.wait(second, 10_000_000_000), "wait")?;
        t.res(dev.wait(first, 10_000_000_000), "wait")?;
        let (e1, _) = gpu::validation_messages();
        let mut e = t.res(dev.encoder(), "encoder")?;
        e.copy_buffer(&big, 0, &rb, 0, n as u64 * 4);
        e.barrier(Access::COPY_WRITE, Access::HOST_READ);
        let cmd = t.res(e.finish(), "finish")?;
        let id = t.res(dev.queue().submit_after(&[second], [cmd]), "submit_after")?;
        t.res(dev.wait(id, 10_000_000_000), "wait")?;
        let mut got = vec![0u32; n as usize];
        t.res(rb.read(0, &mut got), "read")?;
        // every 2048th element against the CPU
        let mut wrong = 0;
        for i in (0..n).step_by(2048) {
            let mut v = i;
            for _ in 0..steps {
                v = v.wrapping_mul(1664525).wrapping_add(1013904223);
            }
            wrong += (got[i as usize] != v.wrapping_mul(3).wrapping_add(1)) as u32;
        }
        if wait {
            t.ok(wrong == 0 && e1 == e0, &format!("a long dispatch on the device queue (still running at the second submission: {running}), then the compute queue on its buffer after it: {wrong} of 512 sampled values wrong, {} validation errors", e1 - e0));
        } else {
            t.ok(true, &format!("the same without the wait: {wrong} of 512 sampled values wrong (the second ran into the first), {} validation errors (the layer sees no hazard between submissions)", e1 - e0));
        }
    }
    // do the two queues run at once?  two long dispatches on two buffers, one after the other, then one on each queue
    let b1 = storage(t, dev, n as u64 * 4, BufferUsage::NONE)?;
    let b2 = storage(t, dev, n as u64 * 4, BufferUsage::NONE)?;
    let g1 = group(t, dev, &sbgl, &[Resource::Buffer { buffer: &b1, offset: 0, size: 0 }])?;
    let g2 = group(t, dev, &sbgl, &[Resource::Buffer { buffer: &b2, offset: 0, size: 0 }])?;
    let mut times = [0.0f64; 2];
    for (k, together) in [false, true, false, true].into_iter().enumerate() {
        let t0 = plat::time_us();
        let mut ids = Vec::new();
        for (i, gr) in [&g1, &g2].into_iter().enumerate() {
            let mut e = if together && i == 1 { t.res(cq.encoder(), "compute encoder")? } else { t.res(dev.encoder(), "encoder")? };
            e.set_compute_pipeline(&spin);
            e.set_bind_group(0, gr);
            e.push_constants(0, &[n, 2000u32]);
            e.dispatch(n.div_ceil(64), 1, 1);
            let cmd = t.res(e.finish(), "finish")?;
            ids.push(if together && i == 1 { t.res(cq.submit([cmd]), "submit")? } else { t.res(dev.queue().submit([cmd]), "submit")? });
        }
        for id in ids {
            t.res(dev.wait(id, 10_000_000_000), "wait")?;
        }
        if k >= 2 {
            times[together as usize] = ms_since(t0);
        }
    }
    out(&format!("  two long dispatches (1M x 2000 steps each): one after the other on the device queue {:.2} ms, one on each queue {:.2} ms", times[0], times[1]));
    Some(())
}
