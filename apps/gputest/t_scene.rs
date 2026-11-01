// t_scene -- gputest's tests of the benchmark scene (scene.rs, measure.rs; main.rs lists them)
//
//   scene        move = move_all bit for bit (three times, the guard record after the objects
//                untouched); cull = cull_ref (instance count, sorted list, nothing written past
//                it; three cameras, and after a GPU move in the same command buffer); an image
//                of known cubes in both modes (background, a lit face's colour, up / down and
//                left / right, depth order, back faces culled: a cube around the camera stays
//                invisible); the indirect and the single picture of the moving scene compared
//                (--shot DIR: the pictures as PNG files there)
//   bench_smoke  short runs of the frame loop (measure.rs) with --verify: indirect and single
//                offscreen, a window with FIFO and IMMEDIATE

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::Cell;

use gpu::*;
use shader::bench::{self, Camera, Object};

use crate::measure::{self, Config, compare_words};
use crate::scene::{Mode, Scene, bench_camera};
use crate::t_compute::{GUARD, readback};
use crate::{T, out, run_wait, setup};

/// the float words of objects
fn words(objs: &[Object]) -> Vec<u32> {
    bench::objects_bytes(objs).chunks_exact(4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
}

const ALL_BEFORE: Access = Access(
    Access::COPY_READ.0 | Access::COPY_WRITE.0 | Access::COMPUTE_READ.0 | Access::COMPUTE_WRITE.0 | Access::INDIRECT.0
        | Access::GRAPHICS_READ.0,
);

fn move_check(t: &mut T, dev: &Device, n: u32) -> Option<()> {
    let s = Scene::new(t, dev, n, Format::Rgba8Unorm, 1)?;
    let bytes = (n as u64 + 1) * bench::OBJECT_SIZE as u64;
    let rb = readback(t, dev, bytes)?;
    let nw = n as usize * 12;
    let mut e = t.res(dev.encoder(), "encoder")?;
    e.barrier(ALL_BEFORE, Access::COPY_WRITE);
    e.fill_buffer(&s.objects, n as u64 * bench::OBJECT_SIZE as u64, bench::OBJECT_SIZE as u64, GUARD);
    run_wait(t, dev, e.finish(), "guard record")?;
    for time in [0.0f32, 12.5, 977.3125] {
        let mut e = t.res(dev.encoder(), "encoder")?;
        e.barrier(ALL_BEFORE, Access::COMPUTE_WRITE);
        s.move_at(&mut e, time);
        e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
        e.copy_buffer(&s.objects, 0, &rb, 0, bytes);
        e.barrier(Access::COPY_WRITE, Access::HOST_READ);
        run_wait(t, dev, e.finish(), "move")?;
        let mut got = vec![0u32; nw + 12];
        t.res(rb.read(0, &mut got), "read objects")?;
        let want = words(&bench::move_all(&s.push(time)));
        let (eq, fl, bad, first) = compare_words(&got[..nw], &want);
        let at = first.map_or(alloc::string::String::new(), |i| {
            format!(": object {} word {} = {:#010x}, CPU {:#010x}", i / 12, i % 12, got[i], want[i])
        });
        t.ok(bad == 0, &format!("move t = {time}: {n} objects = move_all bit for bit ({eq} floats equal, {fl} denormals \
                                 flushed, {bad} different{at})"));
        t.ok(got[nw..].iter().all(|&w| w == GUARD), &format!("move t = {time}: the guard record after the objects untouched"));
    }
    Some(())
}

/// an offscreen target with depth and a read-back of its pixels
struct Img {
    w: u32,
    h: u32,
    color: Texture,
    depth: Texture,
    rb: Buffer,
    used: Cell<bool>,
}

impl Img {
    fn new(t: &mut T, dev: &Device, w: u32, h: u32) -> Option<Img> {
        let color = t.res(dev.create_texture(&TextureDesc::d2(w, h, Format::Rgba8Unorm,
                                                              TextureUsage::COLOR_TARGET | TextureUsage::COPY_SRC)), "colour target")?;
        let depth = t.res(dev.create_texture(&TextureDesc::d2(w, h, Format::Depth32Float, TextureUsage::DEPTH_TARGET)),
                          "depth target")?;
        let rb = readback(t, dev, (w * h * 4) as u64)?;
        Some(Img { w, h, color, depth, rb, used: Cell::new(false) })
    }

    fn px(&self, p: &[u8], x: u32, y: u32) -> [u8; 4] {
        crate::t_render::px(p, self.w * 4, x.min(self.w - 1), y.min(self.h - 1))
    }
}

pub const CLEAR: [f32; 4] = [0.2, 0.2, 0.4, 1.0];
const BG: [u8; 4] = [51, 51, 102, 255];

/// one frame of s into img (camera cam, slot 0) -> its pixels (RGBA)
fn frame_image(t: &mut T, dev: &Device, s: &Scene, img: &Img, mode: Mode, time: Option<f32>, cam: &Camera,
               fill_visible: bool) -> Option<Vec<u8>> {
    t.res(s.set_camera(0, cam), "camera")?;
    let used = img.used.replace(true);
    let mut e = t.res(dev.encoder(), "encoder")?;
    if fill_visible {
        e.barrier(ALL_BEFORE, Access::COPY_WRITE);
        e.fill_buffer(&s.visible, 0, s.visible.size(), 0xFFFF_FFFF);
    }
    let ca = ColorAttachment { view: img.color.view(), load: Load::Clear, clear: CLEAR, store: true,
                               before: if used { Layout::CopySrc } else { Layout::Undefined }, after: Layout::CopySrc };
    let da = DepthAttachment { view: img.depth.view(), load: Load::Clear, clear: 1.0, store: false,
                               before: if used { Layout::DepthTarget } else { Layout::Undefined }, after: Layout::DepthTarget };
    s.record(&mut e, 0, mode, time, ca, da);
    e.copy_texture_to_buffer(TextureRegion::mip(&img.color, 0), &img.rb, DataLayout::default());
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    run_wait(t, dev, e.finish(), "frame")?;
    let mut p = vec![0u8; (img.w * img.h * 4) as usize];
    t.res(img.rb.read(0, &mut p), "read pixels")?;
    Some(p)
}

/// the visible list cull wrote (instance_count entries) and the entry after it
fn read_visible(t: &mut T, dev: &Device, s: &Scene, count: u32) -> Option<(Vec<u32>, u32)> {
    let mut v = vec![0u32; count as usize + 1];
    t.res(dev.queue().read_buffer(&s.visible, 0, &mut v), "read visible list")?;
    let after = v.pop().unwrap_or(0);
    Some((v, after))
}

fn cull_check(t: &mut T, dev: &Device, n: u32) -> Option<()> {
    let s = Scene::new(t, dev, n, Format::Rgba8Unorm, 1)?;
    let img = Img::new(t, dev, 64, 36)?;
    let time = 7.25f32;
    let objs = bench::move_all(&s.push(time));
    t.res(dev.queue().write_buffer(&s.objects, 0, &bench::objects_bytes(&objs)), "upload objects")?;
    let e = s.extent;
    let cams = [
        ("the bench camera", bench_camera(time, e, 16.0 / 9.0)),
        ("outside, the whole disc", Camera::look_at([0.0, e * 0.3, e * 1.2], [0.0, 0.0, 0.0], 1.0, 16.0 / 9.0, 0.1, e * 4.0)),
        ("looking up from above, nothing", Camera::look_at([0.0, e, 0.0], [0.5, e * 3.0, 0.5], 1.0, 16.0 / 9.0, 0.1, e * 4.0)),
    ];
    for (what, cam) in cams {
        frame_image(t, dev, &s, &img, Mode::Indirect, None, &cam, true)?;
        let d = s.draw_cmd(0);
        let want = bench::cull_ref(&objs, &cam);
        let (mut list, after) = read_visible(t, dev, &s, d.instance_count)?;
        list.sort_unstable();
        t.ok(d.instance_count as usize == want.len() && d.index_count == bench::CUBE_INDICES,
             &format!("cull, {what}: instance_count {} = cull_ref ({}), index_count {}", d.instance_count, want.len(), d.index_count));
        t.ok(list == want, &format!("cull, {what}: the sorted visible list = cull_ref"));
        t.ok(after == 0xFFFF_FFFF, &format!("cull, {what}: nothing written after the list"));
    }
    // move and cull in one command buffer (the frame's barriers)
    let t2 = 31.0f32;
    let cam = bench_camera(t2, e, 16.0 / 9.0);
    frame_image(t, dev, &s, &img, Mode::Indirect, Some(t2), &cam, true)?;
    let objs = bench::move_all(&s.push(t2));
    let want = bench::cull_ref(&objs, &cam);
    let d = s.draw_cmd(0);
    let (mut list, _) = read_visible(t, dev, &s, d.instance_count)?;
    list.sort_unstable();
    t.ok(list == want, &format!("move + cull in one frame (t = {t2}): visible list = cull_ref(move_all) ({} of {n}, {:.1} %)",
                                want.len(), want.len() as f64 * 100.0 / n as f64));
    Some(())
}

/// pixel coordinates of world point p (NDC y up, origin top left)
fn project(cam: &Camera, p: [f32; 3], w: u32, h: u32) -> (u32, u32) {
    let m = |c: usize, r: usize| cam.view_proj[c * 4 + r] as f64;
    let clip: Vec<f64> = (0..4).map(|r| m(0, r) * p[0] as f64 + m(1, r) * p[1] as f64 + m(2, r) * p[2] as f64 + m(3, r)).collect();
    let (x, y) = (clip[0] / clip[3], clip[1] / clip[3]);
    (((x + 1.0) * 0.5 * w as f64) as u32, ((1.0 - y) * 0.5 * h as f64) as u32)
}

/// what fs_lambert gives a face with normal (0, 0, 1) of colour c, as Rgba8Unorm
fn lit_front(cam: &Camera, c: [f32; 3]) -> [u8; 4] {
    let l = cam.light_dir;
    let k = cam.ambient as f64 + (1.0 - cam.ambient as f64) * (l[2] as f64).max(0.0);
    let u = |x: f32| (x as f64 * k * 255.0 + 0.5) as u8;
    [u(c[0]), u(c[1]), u(c[2]), 255]
}

/// --shot DIR: the test's pictures as PNG files there
fn save(t: &mut T, px: &[u8], w: u32, h: u32, name: &str) {
    if let Some(dir) = plat::arg_value("--shot") {
        let path = format!("{dir}/{name}");
        t.ok(plat::write_file(&path, &measure::png_rgba(px, w, h)), &format!("--shot: {path}"));
    }
}

fn near(a: [u8; 4], b: [u8; 4], tol: i32) -> bool {
    a.iter().zip(b.iter()).all(|(x, y)| (*x as i32 - *y as i32).abs() <= tol)
}

fn cube(pos: [f32; 3], scale: f32, color: [f32; 3]) -> Object {
    Object { pos, scale, rot: [0.0, 0.0, 0.0, 1.0], color, radius: scale * 1.732_050_8 }
}

/// known cubes in front of a fixed camera, both modes
fn image_check(t: &mut T, dev: &Device) -> Option<()> {
    let white = [1.0, 1.0, 1.0];
    // in draw order (Mode::Single): the near cube before the far one it covers, so a missing
    // depth test would show; the last one surrounds the camera: only its back faces face it
    let objs = [
        cube([1.0, -1.0, 3.0], 0.5, white),               // D: near, in front of C
        cube([0.8, -0.8, -2.0], 1.5, [0.3, 0.6, 0.9]),     // C: far
        cube([0.0, 3.0, 0.0], 1.0, [1.0, 0.5, 0.25]),      // A: above the centre
        cube([3.0, 0.0, 0.0], 1.0, [0.25, 1.0, 0.5]),      // B: right of it
        cube([-3.0, -3.0, 0.0], 1.0, [0.5, 0.25, 1.0]),    // E: lower left
        cube([0.0, 0.0, 10.0], 40.0, [1.0, 0.0, 1.0]),     // around the camera
    ];
    let s = Scene::new(t, dev, objs.len() as u32, Format::Rgba8Unorm, 1)?;
    t.res(dev.queue().write_buffer(&s.objects, 0, &bench::objects_bytes(&objs)), "upload cubes")?;
    let cam = Camera::look_at([0.0, 0.0, 10.0], [0.0, 0.0, 0.0], 1.047_197_6, 1.0, 0.1, 100.0);
    let img = Img::new(t, dev, 256, 256)?;
    let (w, h) = (img.w, img.h);
    let mut pics = Vec::new();
    for mode in [Mode::Indirect, Mode::Single] {
        let name = if mode == Mode::Indirect { "indirect" } else { "single" };
        let p = frame_image(t, dev, &s, &img, mode, None, &cam, false)?;
        if mode == Mode::Indirect {
            let d = s.draw_cmd(0);
            t.ok(d.instance_count == 6 && bench::cull_ref(&objs, &cam).len() == 6,
                 &format!("image, {name}: all 6 cubes pass cull (instance_count {})", d.instance_count));
        }
        let at = |pt: [f32; 3]| project(&cam, pt, w, h);
        let probe = |t: &mut T, (x, y): (u32, u32), want: [u8; 4], what: &str| {
            let got = img.px(&p, x, y);
            t.ok(near(got, want, 2), &format!("image, {name}: {what} at ({x}, {y}) = {got:?}, expected {want:?}"));
        };
        // background in the corners and between the cubes: the cube around the camera is culled
        for (x, y) in [(4, 4), (w - 5, 4), (4, h - 5), (w - 5, h - 5), (w - 6, h / 2), (w / 2, h - 6)] {
            probe(t, (x, y), BG, "background (back faces culled)");
        }
        let (ax, ay) = at([0.0, 3.0, 1.0]);
        t.ok(ay < h / 2 && ax.abs_diff(w / 2) < 2, &format!("image: the cube above the centre projects into the upper half ({ax}, {ay})"));
        probe(t, (ax, ay), lit_front(&cam, objs[2].color), "the cube above, its lit front face");
        probe(t, (ax, h - 1 - ay), BG, "mirrored below the centre: nothing");
        let (bx, by) = at([3.0, 0.0, 1.0]);
        t.ok(bx > w / 2, &format!("image: the cube on the right projects into the right half ({bx}, {by})"));
        probe(t, (bx, by), lit_front(&cam, objs[3].color), "the cube on the right");
        probe(t, (w - 1 - bx, by), BG, "mirrored to the left: nothing");
        let (ex, ey) = at([-3.0, -3.0, 1.0]);
        t.ok(ex < w / 2 && ey > h / 2, &format!("image: the cube at the lower left there ({ex}, {ey})"));
        probe(t, (ex, ey), lit_front(&cam, objs[4].color), "the cube at the lower left");
        probe(t, at([1.0, -1.0, 3.5]), lit_front(&cam, white), "depth: the near cube in front of the far one");
        probe(t, at([0.0, 0.0, -0.5]), lit_front(&cam, objs[1].color), "the far cube beside the near one");
        let lit = p.chunks(4).filter(|c| !near([c[0], c[1], c[2], c[3]], BG, 1)).count();
        out(&format!("  image, {name}: {lit} of {} pixels show cubes", w * h));
        save(t, &p, w, h, &format!("scene_image_{name}.png"));
        pics.push(p);
    }
    let diff = pics[0].chunks(4).zip(pics[1].chunks(4)).filter(|(a, b)| a != b).count();
    t.ok(diff == 0, &format!("image: indirect and single draw the same picture ({diff} pixels differ)"));
    Some(())
}

/// the moving scene drawn by both paths (small n): the same picture
fn modes_check(t: &mut T, dev: &Device, n: u32) -> Option<()> {
    let s = Scene::new(t, dev, n, Format::Rgba8Unorm, 1)?;
    let img = Img::new(t, dev, 480, 270)?;
    let time = 5.0f32;
    let cam = bench_camera(time, s.extent, 16.0 / 9.0);
    let a = frame_image(t, dev, &s, &img, Mode::Indirect, Some(time), &cam, false)?;
    let vis = s.visible(0);
    let b = frame_image(t, dev, &s, &img, Mode::Single, Some(time), &cam, false)?;
    save(t, &a, img.w, img.h, "scene_modes_indirect.png");
    save(t, &b, img.w, img.h, "scene_modes_single.png");
    let (mut diff, mut worst) = (0usize, 0i32);
    for (x, y) in a.chunks(4).zip(b.chunks(4)) {
        if x != y {
            diff += 1;
            worst = worst.max(x.iter().zip(y).map(|(p, q)| (*p as i32 - *q as i32).abs()).max().unwrap_or(0));
        }
    }
    let lit = a.chunks(4).filter(|c| !near([c[0], c[1], c[2], c[3]], BG, 1)).count();
    let total = (img.w * img.h) as usize;
    t.ok(lit > total / 20, &format!("modes: the moving scene covers {lit} of {total} pixels ({vis} of {n} cubes visible)"));
    t.ok(diff * 1000 <= total, &format!("modes: indirect (culled) and single (all {n}) pictures: {diff} pixels differ \
                                         (largest channel difference {worst}; allowed: 0.1 % of the pixels)"));
    Some(())
}

pub fn scene(t: &mut T) {
    let Some(g) = setup(t) else { return };
    let dev = &g.dev;
    let n = if t.validate { 20_011 } else { 100_003 };
    move_check(t, dev, n);
    cull_check(t, dev, n);
    image_check(t, dev);
    modes_check(t, dev, 3000);
}

pub fn bench_smoke(t: &mut T) {
    let Some(g) = setup(t) else { return };
    let n = if t.validate { 5000 } else { 20_000 };
    let swap = g.dev.features().swapchain;
    for (mode, window) in [(Mode::Indirect, None), (Mode::Single, None), (Mode::Indirect, Some(PresentMode::Fifo)),
                           (Mode::Single, Some(PresentMode::Immediate))] {
        if window.is_some() && !swap {
            out("SKIP: no swapchains on this device");
            continue;
        }
        let c = Config { n, w: 640, h: 360, mode, window, frames: 30, warmup: 5, inflight: 3, verify: true, shot: None,
                         fullscreen: false };
        let Some(r) = measure::run(t, &g, &c) else { continue };
        measure::print(&c, &r);
        let what = format!("bench {mode:?} {}", r.out);
        let vis = r.visible.med / n as f64;
        t.ok(r.frames == 30, &format!("{what}: 30 frames measured ({})", r.frames));
        let vis_ok = if mode == Mode::Indirect { vis > 0.25 && vis < 0.85 } else { vis == 1.0 };
        t.ok(vis_ok, &format!("{what}: visible fraction {vis:.3} plausible"));
        t.ok(r.frame.med > 0.0 && r.cpu.med > 0.0 && r.cpu.med <= r.frame.p99 * 4.0 + 1.0,
             &format!("{what}: frame {:.3} ms, CPU {:.3} ms", r.frame.med, r.cpu.med));
        if r.timestamps {
            t.ok(r.gpu_move.med > 0.0 && r.gpu_draw.med > 0.0 && r.gpu_total.med >= r.gpu_draw.med,
                 &format!("{what}: GPU times from timestamps (move {:.3}, draw {:.3}, total {:.3} ms)", r.gpu_move.med,
                          r.gpu_draw.med, r.gpu_total.med));
        }
    }
}
