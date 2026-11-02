// t_window -- gputest's window test: a plain plat window (its own event loop, never given to
// plat::win_present) with a swapchain (main.rs lists the checks)

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use plat::{PW_DEFAULT, PW_KEEP, PW_NOOWNER, PWF_RESIZABLE, PWF_SYSFRAME, PWS_SHOW, WCMD_MINIMIZE, WCMD_RESTORE, WST_MINIMIZED};

use crate::t_compute::readback;
use crate::t_render::Scene;
use crate::{T, ms_since, out, setup};

/// handle the window's messages for ms milliseconds (0: what is queued)
pub fn pump(ms: u64) {
    let dl = plat::time_us() + ms * 1000;
    loop {
        plat::wait(if ms == 0 { 0 } else { dl });
        while plat::evq_pop().is_some() {}
        if plat::time_us() >= dl {
            return;
        }
    }
}

/// pump until cond (at most ms) -> whether it became true
pub fn pump_until(ms: u64, cond: impl Fn() -> bool) -> bool {
    let dl = plat::time_us() + ms * 1000;
    while plat::time_us() < dl {
        pump(10);
        if cond() {
            return true;
        }
    }
    cond()
}

/// one frame: clear, the scene, optionally the presented image into rb -> Some(id) presented,
/// None: no image (minimised)
pub fn frame(dev: &Device, sc: &mut Swapchain, scene: Option<&Scene>, clear: [f32; 4], rb: Option<&Buffer>)
             -> gpu::Result<Option<SubmitId>> {
    let Some(fr) = sc.acquire()? else { return Ok(None) };
    let sc = &*sc;
    let mut e = dev.encoder()?;
    let after = if rb.is_some() { Layout::CopySrc } else { Layout::Present };
    e.begin_render(&RenderPassDesc {
        color: &[ColorAttachment { view: sc.view(&fr), load: Load::Clear, clear, store: true, before: Layout::Undefined, after }],
        depth: None,
    });
    if let Some(s) = scene {
        s.bind(&mut e);
        e.draw_indexed(6, 1, 0, 0, 0);
    }
    e.end_render();
    if let Some(rb) = rb {
        let tex = sc.texture(&fr);
        e.copy_texture_to_buffer(TextureRegion::mip(tex, 0), rb, DataLayout::default());
        e.texture_barrier(tex, Layout::CopySrc, Layout::Present);
        e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    }
    let cmd = e.finish()?;
    sc.present(fr, [cmd]).map(Some)
}

/// n frames (clear colours cycling) -> the intervals between presents (ms)
pub fn run_frames(dev: &Device, sc: &mut Swapchain, scene: Option<&Scene>, n: usize) -> gpu::Result<Vec<f64>> {
    let mut times = Vec::with_capacity(n);
    let mut last = plat::time_us();
    for k in 0..n {
        pump(0);
        let c = (k % 64) as f32 / 63.0;
        if frame(dev, sc, scene, [c, 0.2, 1.0 - c, 1.0], None)?.is_some() {
            let now = plat::time_us();
            times.push((now - last) as f64 / 1000.0);
            last = now;
        }
    }
    Ok(times)
}

/// mean, median, 99th percentile of frame intervals (the first 10 skipped)
pub fn stats(v: &[f64]) -> (f64, f64, f64) {
    let v = if v.len() > 20 { &v[10..] } else { v };
    if v.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let mean = s.iter().sum::<f64>() / s.len() as f64;
    (mean, s[s.len() / 2], s[(s.len() * 99 / 100).min(s.len() - 1)])
}

#[cfg(not(windows))]
fn platform_wayland() -> bool {
    plat::disp_backend() == plat::BK_WAYLAND
}

#[cfg(windows)]
fn platform_wayland() -> bool {
    false
}

pub fn open_window(t: &mut T) -> Option<i32> {
    let w = plat::win_create("gputest", 320, 240, PWF_SYSFRAME | PWF_RESIZABLE, PW_NOOWNER, PW_DEFAULT, PW_DEFAULT);
    if !t.ok(w >= 0, "plat window") {
        return None;
    }
    plat::win_show(w, PWS_SHOW);
    pump(150);
    Some(w)
}

fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (*x as i32 - *y as i32).abs() <= 1)
}

pub fn window(t: &mut T) {
    let Some(g) = setup(t) else { return };
    let dev = &g.dev;
    if !dev.features().swapchain {
        out("SKIP: no swapchains on this device");
        return;
    }
    let Some(w) = open_window(t) else { return };
    window_checks(t, &g, w);
    plat::win_destroy(w);
    pump(50);
}

fn window_checks(t: &mut T, g: &crate::Gpu, w: i32) -> Option<()> {
    let dev = &g.dev;
    let surf = t.res(Surface::new(&g.inst, w), "Surface::new")?;
    let t0 = plat::time_us();
    let desc = SwapchainDesc { present: PresentMode::Fifo, copy_src: true, ..SwapchainDesc::DEFAULT };
    let mut sc = t.res(dev.create_swapchain(&surf, &desc), "create_swapchain")?;
    let (ww, wh) = plat::win_size(w);
    out(&format!("  swapchain: {:.2} ms, {:?}, {:?}, {} images, {}x{}", ms_since(t0), sc.format(), sc.present_mode(),
                 sc.image_count(), sc.size().0, sc.size().1));
    t.ok(sc.size() == (ww as u32, wh as u32), &format!("swapchain size = window client size {ww}x{wh}"));
    t.ok(sc.present_mode() == PresentMode::Fifo, "FIFO");
    let scene = Scene::new(t, dev, sc.format(), None)?;
    // FIFO frames
    let times = t.res(run_frames(dev, &mut sc, Some(&scene), 60), "FIFO frames")?;
    let (fifo, med, p99) = stats(&times);
    out(&format!("  FIFO: {} frames, {fifo:.2} ms mean, {med:.2} median, {p99:.2} p99", times.len()));
    t.ok(times.len() == 60, "60 FIFO frames presented");
    t.ok(fifo > 2.0 && fifo < 50.0, "FIFO frame interval plausible (vsync: 2 .. 50 ms)");
    // the presented image read back
    let (sw, sh) = sc.size();
    let rb = readback(t, dev, (sw * sh * 4) as u64)?;
    let clear = [0.2f32, 0.4, 0.6, 1.0];
    let id = t.res(frame(dev, &mut sc, Some(&scene), clear, Some(&rb)), "frame with read-back")?;
    if let Some(id) = id {
        t.res(dev.wait(id, 10_000_000_000), "wait");
        let mut px = vec![0u8; (sw * sh * 4) as usize];
        t.res(rb.read(0, &mut px), "read");
        let bgra = matches!(sc.format(), Format::Bgra8Unorm | Format::Bgra8Srgb);
        let sw4 = sw * 4;
        let at = |x: u32, y: u32| crate::t_render::px(&px, sw4, x, y);
        let (c, r) = (at(2, 2), at(sw / 5, sh * 3 / 4));
        let want_c = if bgra { [153, 102, 51, 255] } else { [51, 102, 153, 255] };
        let want_r = if bgra { [0, 0, 255, 255] } else { [255, 0, 0, 255] };
        t.ok(near(c, want_c), &format!("presented image: clear colour at (2, 2) = {c:?}, expected {want_c:?}"));
        t.ok(near(r, want_r), &format!("presented image: red triangle at ({}, {}) = {r:?}", sw / 5, sh * 3 / 4));
    } else {
        t.fail("frame with read-back: no image");
    }
    // resize
    plat::win_set_rect(w, PW_KEEP, PW_KEEP, 400, 300);
    pump(200);
    let p = t.res(frame(dev, &mut sc, Some(&scene), clear, None), "frame after resize")?;
    let (ww, wh) = plat::win_size(w);
    t.ok(p.is_some() && sc.size() == (400, 300) && (ww, wh) == (400, 300),
         &format!("resize to 400x300: swapchain {:?}, window {ww}x{wh}", sc.size()));
    // the other present modes (measurements; FIFO when the surface lacks one)
    for mode in [PresentMode::Immediate, PresentMode::Mailbox] {
        sc.set_present_mode(mode);
        let times = t.res(run_frames(dev, &mut sc, Some(&scene), 200), "frames")?;
        let (mean, med, p99) = stats(&times);
        out(&format!("  {mode:?} -> {:?}: {} frames, {mean:.3} ms mean, {med:.3} median, {p99:.3} p99", sc.present_mode(),
                     times.len()));
        t.ok(times.len() == 200, &format!("{mode:?}: 200 frames presented"));
        if mode == PresentMode::Immediate && sc.present_mode() == mode && fifo > 4.0 {
            t.ok(mean < fifo, "IMMEDIATE faster than FIFO");
        }
    }
    // minimise / restore
    plat::win_cmd(w, WCMD_MINIMIZE);
    let min = pump_until(2000, || plat::win_state(w) & WST_MINIMIZED != 0);
    t.ok(min, "window minimised");
    let r = sc.acquire();
    t.ok(matches!(r, Ok(None)), "minimised: acquire = Ok(None)");
    drop(r);
    plat::win_cmd(w, WCMD_RESTORE);
    let wayland = cfg!(not(windows)) && platform_wayland();
    let back = pump_until(if wayland { 200 } else { 2000 }, || plat::win_state(w) & WST_MINIMIZED == 0);
    pump(100);
    let p = t.res(frame(dev, &mut sc, Some(&scene), clear, None), "frame after restore")?;
    // (a Wayland client cannot undo a minimise: the compositor shows the window again, nobody says so)
    if back || !wayland {
        t.ok(back && p.is_some(), "restored: frames again");
    } else {
        out("  note: Wayland: the window stays minimised until somebody activates it");
    }
    t.res(dev.wait_idle(), "wait_idle")?;
    Some(())
}
