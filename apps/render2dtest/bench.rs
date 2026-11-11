// bench -- a first measurement of render2d (G6a1): N segments offscreen (polylines of 2 to 33 points, a random walk
// over the target at 1 unit per pixel), per level and width: inserting them, the first frame (everything goes up),
// frames with everything in view and zoomed in (a tenth in view), frames with 1000 entities changed each
//
//   render2dtest --bench N [--res 1080p|4k|WxH] [--frames F] [--basis]
//
// GPU times are the scene's timestamps (FrameStats): uploads, visibility (the three compute passes), the draw; each
// frame is waited for before the next (no frames in flight: the times are the frame's own), medians over F frames.

use alloc::format;
use alloc::vec::Vec;

use gpu::*;
use render2d::{Camera, Cap, EntityId, Join, Layer, Renderer, RendererDesc, Scene, Shape, Style};

use crate::cad;
use crate::gallery::{bg_clear, Rng, BG};
use crate::tests::out;

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    v[v.len() / 2]
}

struct Target {
    color: Texture,
    used: bool,
}

/// one frame, waited for: (prepare's CPU ms, the frame's stamps relative to its start: uploads, visibility, draw ms,
/// records visible)
fn frame(dev: &Device, r: &mut Renderer, s: &mut Scene, t: &mut Target, size: (u32, u32)) -> Result<(f64, [f64; 3], u64)> {
    let t0 = plat::time_us();
    s.prepare(r, size)?;
    let prep = (plat::time_us() - t0) as f64 / 1000.0;
    let mut e = dev.encoder()?;
    let color = ColorAttachment { view: t.color.view(), load: Load::Clear, clear: bg_clear(BG), store: true, before: if t.used { Layout::ColorTarget } else { Layout::Undefined },
                                  after: Layout::ColorTarget };
    r.record(&mut e, s, color)?;
    let id = dev.queue().submit([e.finish()?])?;
    s.submitted(id);
    t.used = true;
    if !dev.wait(id, 30_000_000_000)? {
        return Err(GpuError::Timeout);
    }
    s.collect_stats();
    let st = s.take_stats();
    let f = st.last().copied();
    let (stamps, vis) = match f {
        Some(f) => (f.stamps.map_or([0.0; 3], |v| [v[1] - v[0], v[2] - v[1], v[3] - v[2]]), f.visible),
        None => ([0.0; 3], 0),
    };
    Ok((prep, stamps, vis))
}

pub fn bench(n: u32) -> i32 {
    let size = match plat::arg_value("--res").as_deref() {
        Some("4k") => (3840u32, 2160u32),
        Some(v) if v.contains('x') => {
            let mut it = v.split('x').filter_map(|x| x.parse().ok());
            (it.next().unwrap_or(1920), it.next().unwrap_or(1080))
        }
        _ => (1920, 1080),
    };
    let frames: usize = plat::arg_value("--frames").and_then(|v| v.parse().ok()).unwrap_or(40);
    let basis_only = plat::args().iter().any(|a| a == "--basis");
    let Ok(inst) = Instance::new() else {
        out("SKIP: no GPU");
        return 0;
    };
    let Ok(adapter) = inst.adapter() else {
        out("SKIP: no adapter");
        return 0;
    };
    let cad_scene = plat::arg_value("--scene").as_deref() == Some("cad");
    out(&format!("render2d bench: {n} segments, {} x {}, {frames} frames each, {}{}", size.0, size.1, adapter.info().name, if cad_scene { ", the town plan" } else { "" }));
    let mut levels = Vec::new();
    if adapter.level() == Level::Voll && !basis_only {
        levels.push(Level::Voll);
    }
    levels.push(Level::Basis);
    for level in levels {
        let dev = match adapter.create_device(&DeviceDesc { max_level: level }) {
            Ok(d) => d,
            Err(e) => {
                out(&format!("FAIL device: {e}"));
                return 1;
            }
        };
        let result = if cad_scene { run_cad(&dev, n, size, frames, level) } else { run(&dev, n, size, frames, level) };
        if let Err(e) = result {
            out(&format!("FAIL {level:?}: {e}"));
            return 1;
        }
    }
    0
}

/// the town plan (cad.rs): hairlines and the plan's widths, each at the overview, a district and the working zoom
fn run_cad(dev: &Device, n: u32, size: (u32, u32), frames: usize, level: Level) -> Result<()> {
    let mut r = Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm))?;
    for (wname, styled) in [("hairlines", false), ("styled", true)] {
        let mut s = Scene::new(&r)?;
        s.set_stats(true);
        let t0 = plat::time_us();
        let c = cad::build(&mut s, n, 5, styled)?;
        let insert_ms = (plat::time_us() - t0) as f64 / 1000.0;
        let mut t = Target { color: dev.create_texture(&TextureDesc::d2(size.0, size.1, Format::Bgra8Unorm, TextureUsage::COLOR_TARGET))?, used: false };
        let views = cad::views(&c, size);
        s.set_camera(&views[0].cam)?;
        let (prep, st0, vis0) = frame(dev, &mut r, &mut s, &mut t, size)?;
        let up = s.upload_info();
        out(&format!("{level:?}, {wname}: {} entities, {} segments, {:.0} x {:.0} m; insert {insert_ms:.1} ms (CPU); first frame: prepare {prep:.1} ms (CPU, {:.1} MB up), GPU uploads {:.3} ms, \
                      visibility {:.3} ms, draw {:.3} ms ({vis0} visible)", c.entities, c.segments, c.width, c.height, up.bytes as f64 / 1048576.0, st0[0], st0[1], st0[2]));
        for v in &views {
            let (mut a, mut b, mut d, mut p) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            let mut vis = 0;
            for k in 0..frames + 3 {
                // (the camera moves a little: a frame of a pan)
                let mut cam = v.cam;
                cam.center[0] += k as f64 * 0.37 * cam.scale;
                s.set_camera(&cam)?;
                let (prep, st, n) = frame(dev, &mut r, &mut s, &mut t, size)?;
                if k >= 3 {
                    a.push(st[0]);
                    b.push(st[1]);
                    d.push(st[2]);
                    p.push(prep);
                    vis = n;
                }
            }
            let (ma, mb, mc, mp) = (median(&mut a), median(&mut b), median(&mut d), median(&mut p));
            out(&format!("{level:?}, {wname}, {} (scale {:.3}): GPU per frame {:.3} ms (uploads {ma:.3}, visibility {mb:.3}, draw {mc:.3}); prepare {mp:.3} ms CPU; {vis} visible",
                         v.name, v.cam.scale, ma + mb + mc));
        }
        dev.wait_idle()?;
    }
    Ok(())
}

fn run(dev: &Device, n: u32, size: (u32, u32), frames: usize, level: Level) -> Result<()> {
    let mut r = Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm))?;
    for (wname, width) in [("hairlines", 0.0f32), ("3 px, round joins", 3.0)] {
        let mut s = Scene::new(&r)?;
        s.set_stats(true);
        let l = s.add_layer(&Layer::new(0))?;
        let mut st = Style::new(0xFF30_4050);
        st.width = width;
        if width > 0.0 {
            st.cap = Cap::Round;
            st.join = Join::Round;
        }
        let sid = s.add_style(&st)?;
        let (w, h) = (size.0 as f64, size.1 as f64);
        let mut rng = Rng(5);
        let mut polys: Vec<(EntityId, Vec<[f64; 2]>)> = Vec::new();
        let mut segs = 0u32;
        let mut pts: Vec<[f64; 2]> = Vec::new();
        let t0 = plat::time_us();
        while segs < n {
            let m = (2 + rng.below(32)).min(n - segs + 1) as usize;
            pts.clear();
            pts.push([rng.range(-0.5 * w, 0.5 * w), rng.range(-0.5 * h, 0.5 * h)]);
            for _ in 1..m {
                let q = pts[pts.len() - 1];
                pts.push([q[0] + rng.range(-12.0, 12.0), q[1] + rng.range(-12.0, 12.0)]);
            }
            let id = s.insert(l, sid, &Shape::Polyline { points: &pts, closed: false })?;
            if polys.len() < 1 << 16 {
                polys.push((id, pts.clone()));
            }
            segs += m as u32 - 1;
        }
        let insert_ms = (plat::time_us() - t0) as f64 / 1000.0;
        let mut t = Target { color: dev.create_texture(&TextureDesc::d2(size.0, size.1, Format::Bgra8Unorm, TextureUsage::COLOR_TARGET))?, used: false };
        s.set_camera(&Camera::new([0.0, 0.0], 1.0))?;
        // the first frame: every record goes up
        let (prep, st0, vis0) = frame(dev, &mut r, &mut s, &mut t, size)?;
        let up = s.upload_info();
        out(&format!("{level:?}, {wname}: {} entities, {segs} segments; insert {insert_ms:.1} ms (CPU); first frame: prepare {prep:.1} ms (CPU, {:.1} MB up), GPU uploads {:.3} ms, \
                      visibility {:.3} ms, draw {:.3} ms ({vis0} visible)", s.len(), up.bytes as f64 / 1048576.0, st0[0], st0[1], st0[2]));
        // everything in view, then a tenth of it
        for (vname, scale) in [("all in view", 1.0f64), ("zoomed in (a tenth in view)", 0.316)] {
            s.set_camera(&Camera::new([0.0, 0.0], scale))?;
            let (mut a, mut b, mut c, mut p) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            let mut vis = 0;
            for k in 0..frames + 3 {
                // (the camera moves a little: a frame of a pan)
                s.set_camera(&Camera::new([k as f64 * 0.37, 0.0], scale))?;
                let (prep, st, v) = frame(dev, &mut r, &mut s, &mut t, size)?;
                if k >= 3 {
                    a.push(st[0]);
                    b.push(st[1]);
                    c.push(st[2]);
                    p.push(prep);
                    vis = v;
                }
            }
            let (ma, mb, mc, mp) = (median(&mut a), median(&mut b), median(&mut c), median(&mut p));
            out(&format!("{level:?}, {wname}, {vname}: GPU per frame {:.3} ms (uploads {ma:.3}, visibility {mb:.3}, draw {mc:.3}); prepare {mp:.3} ms CPU; {vis} visible",
                         ma + mb + mc));
        }
        // 1000 entities changed per frame
        s.set_camera(&Camera::new([0.0, 0.0], 1.0))?;
        let (mut a, mut p, mut bytes) = (Vec::new(), Vec::new(), 0u64);
        let mut rec = 0u32;
        for k in 0..frames {
            let t0 = plat::time_us();
            for j in 0..1000usize {
                let i = (k * 7919 + j * 104_729) % polys.len();
                let (id, ref mut q) = polys[i];
                for v in q.iter_mut() {
                    v[0] += 0.5;
                }
                s.update(id, &Shape::Polyline { points: q, closed: false })?;
            }
            let upd = (plat::time_us() - t0) as f64 / 1000.0;
            let (prep, st, _) = frame(dev, &mut r, &mut s, &mut t, size)?;
            let info = s.upload_info();
            bytes = info.bytes;
            rec = info.records;
            a.push(st[0]);
            p.push(prep + upd);
        }
        out(&format!("{level:?}, {wname}, 1000 entities changed per frame ({rec} records, {:.2} MB): update + prepare {:.3} ms CPU, GPU uploads {:.3} ms",
                     bytes as f64 / 1048576.0, median(&mut p), median(&mut a)));
        dev.wait_idle()?;
    }
    Ok(())
}
