// bench -- gputest --bench: creation times, frame times per present mode, the 1M compute round
// trip (the numbers of docs/GPU.md)

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;

use crate::t_compute::{ADD, Fill, MUL, TAIL};
use crate::t_window::{open_window, run_frames, stats};
use crate::{T, ms_since, out};

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    if v.is_empty() { 0.0 } else { v[v.len() / 2] }
}

pub fn bench() {
    let mut t = T { fails: 0, validate: false, expected: 0 };
    // instance and device creation: the first time (driver load) and the median of 5 more
    let mut inst_ms = Vec::new();
    let mut dev_ms = Vec::new();
    let mut enum_ms = Vec::new();
    for _ in 0..6 {
        let t0 = plat::time_us();
        let inst = match Instance::new() {
            Ok(i) => i,
            Err(e) => {
                out(&format!("no GPU: {e}"));
                return;
            }
        };
        inst_ms.push(ms_since(t0));
        let t1 = plat::time_us();
        let Ok(a) = inst.adapter() else { return };
        enum_ms.push(ms_since(t1));
        let t2 = plat::time_us();
        let Ok(d) = a.create_device(&DeviceDesc::DEFAULT) else { return };
        dev_ms.push(ms_since(t2));
        drop(d);
    }
    let (i0, e0, d0) = (inst_ms[0], enum_ms[0], dev_ms[0]);
    out(&format!("instance: first {i0:.2} ms, then median {:.2} ms", median(&mut inst_ms[1..])));
    out(&format!("adapter enumeration: first {e0:.2} ms, then median {:.2} ms", median(&mut enum_ms[1..])));
    out(&format!("device: first {d0:.2} ms, then median {:.2} ms", median(&mut dev_ms[1..])));
    let Some(g) = crate::setup(&mut t) else { return };
    let dev = &g.dev;
    out(&format!("{} ({}), level {:?}", dev.info().name, dev.info().driver_info, dev.level()));
    // 1M compute round trip: fill (dispatch), copy into a readback buffer, submit, wait, read
    if let Some(fp) = Fill::new(&mut t, dev) {
        let n = 1u32 << 20;
        let bytes = (n + TAIL) as u64 * 4;
        let buf = dev.create_buffer(&BufferDesc { size: bytes, usage: BufferUsage::STORAGE, memory: MemoryKind::Device });
        let rb = dev.create_buffer(&BufferDesc { size: bytes, usage: BufferUsage::NONE, memory: MemoryKind::Readback });
        let ts = dev.create_timestamps(4).ok();
        if let (Ok(buf), Ok(rb)) = (buf, rb) {
            let bg = fp.group(&mut t, dev, &buf);
            let mut out_v = vec![0u32; (n + TAIL) as usize];
            let (mut total, mut gpu_fill, mut gpu_copy, mut read) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for _ in 0..20 {
                let t0 = plat::time_us();
                let Ok(mut e) = dev.encoder() else { break };
                if let Some(ts) = &ts {
                    e.reset_timestamps(ts, 0, 4);
                    e.timestamp(ts, 0);
                }
                e.set_compute_pipeline(&fp.pipe);
                if let Some(bg) = &bg {
                    e.set_bind_group(0, bg);
                }
                e.push_constants(0, &[n, MUL, ADD]);
                e.dispatch(n.div_ceil(64), 1, 1);
                if let Some(ts) = &ts {
                    e.timestamp(ts, 1);
                }
                e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
                if let Some(ts) = &ts {
                    e.timestamp(ts, 2);
                }
                e.copy_buffer(&buf, 0, &rb, 0, bytes);
                if let Some(ts) = &ts {
                    e.timestamp(ts, 3);
                }
                e.barrier(Access::COPY_WRITE, Access::HOST_READ);
                let Ok(cmd) = e.finish() else { break };
                let Ok(id) = dev.queue().submit([cmd]) else { break };
                let _ = dev.wait(id, 10_000_000_000);
                let t1 = plat::time_us();
                let _ = rb.read(0, &mut out_v);
                read.push(ms_since(t1));
                total.push(ms_since(t0));
                if let Some(ms) = ts.as_ref().and_then(|ts| ts.read_ms(0, 4).ok()) {
                    gpu_fill.push(ms[1] - ms[0]);
                    gpu_copy.push(ms[3] - ms[2]);
                }
            }
            let ok = (0..n).all(|i| out_v[i as usize] == crate::fill_ref(i, MUL, ADD));
            out(&format!(
                "1M compute round trip (record, submit, wait, read 4 MiB): median {:.3} ms (first {:.3}); GPU fill {:.3} ms, \
                 GPU copy {:.3} ms, host read {:.3} ms; values {}",
                median(&mut total[1..].to_vec()),
                total.first().copied().unwrap_or(0.0),
                median(&mut gpu_fill),
                median(&mut gpu_copy),
                median(&mut read),
                if ok { "correct" } else { "WRONG" }
            ));
        }
    }
    // window: swapchain creation, then a clear-and-present loop per present mode
    if !dev.features().swapchain {
        return;
    }
    let Some(w) = open_window(&mut t) else { return };
    if let Ok(surf) = Surface::new(&g.inst, w) {
        for (mode, frames) in [(PresentMode::Fifo, 300), (PresentMode::Mailbox, 500), (PresentMode::Immediate, 2000)] {
            let t0 = plat::time_us();
            let desc = SwapchainDesc { present: mode, ..SwapchainDesc::DEFAULT };
            let Ok(mut sc) = dev.create_swapchain(&surf, &desc) else { continue };
            let create = ms_since(t0);
            let Ok(times) = run_frames(dev, &mut sc, None, frames) else { continue };
            let (mean, med, p99) = stats(&times);
            out(&format!(
                "{mode:?} (got {:?}, {} images, {}x{}): swapchain {create:.2} ms; {} frames: {mean:.3} ms mean = {:.0} fps, \
                 {med:.3} median, {p99:.3} p99",
                sc.present_mode(),
                sc.image_count(),
                sc.size().0,
                sc.size().1,
                times.len(),
                if mean > 0.0 { 1000.0 / mean } else { 0.0 },
            ));
            let _ = dev.wait_idle();
        }
    }
    plat::win_destroy(w);
}
