// cpucmp -- the CPU path (G6e1, Scene::draw_cpu) against the GPU: every image of the gallery (gallery::shots, what --dump
// writes) on the device's own level, by category: pixels beyond 2 codes, the largest difference, the ink; a scene without a
// device (Scene::new_cpu) draws the same bytes as the CPU path of the same scene on a device; a scene taken off its device
// and put on again (detach, attach) draws the same image on the GPU.  The measuring scene's views: bench.rs (--cpu).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::{Dc, Surface, FMT_XRGB};
use gpu::*;
use render2d::{Camera, Renderer, RendererDesc, Scene};

use crate::gallery::*;
use crate::refimg::{self, Cmp};
use crate::tests::{setup, shot, T};

/// lib/gfx's parallel loop on plat's worker pool, as lib/ui sets it (the CPU path's row bands; tests and the bench run without
/// lib/ui's runtime)
pub fn par(n: usize, f: &(dyn Fn(usize) + Sync)) {
    let _ = plat::parallel_for(n, &|i, _| f(i));
}

/// the category of a shot: its name up to the first '_'
fn category(name: &str) -> &str {
    name.split('_').next().unwrap_or(name)
}

/// a category's sum of comparisons
#[derive(Default, Clone)]
struct Sum {
    name: String,
    images: u32,
    pixels: u64,
    over: u64,
    max: u8,
    at: String,
    ink: [f64; 2],
    /// (drawings of plain entities) the GPU and the CPU path each against lib/gfx's independent reference (refimg.rs):
    /// pixels beyond 2 codes, the largest difference, the ink of both and of the reference
    refs: u32,
    ref_over: [u64; 2],
    ref_max: [u8; 2],
    ref_ink: [f64; 3],
}

/// --test cpu
pub fn cpu(t: &mut T) {
    let Some(g) = setup(t) else { return };
    let Some(dev) = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "device") else { return };
    let Some(mut r) = t.res(Renderer::new(&dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new") else { return };
    let mut sums: Vec<Sum> = Vec::new();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let (mut same_cpu, mut shots_n, mut same_bands) = (0u32, 0u32, 0u32);
    gfx::set_parallel(par);
    for sh in shots() {
        let Some(mut off) = t.res(Off::new(&dev, sh.w, sh.h), "target") else { return };
        let Some(mut s) = t.res(Scene::new(&r), "Scene::new") else { return };
        if t.res(s.set_camera(&sh.cam), &sh.name).is_none() || t.res(sh.build(&mut s), &sh.name).is_none() {
            return;
        }
        let Some(gpu_img) = shot(t, &dev, &mut off, &mut r, &mut s, &sh.name) else { return };
        s.set_cpu_single(true);
        let cpu = cpu_image(&mut s, sh.w, sh.h, BG);
        // (row bands on lib/gfx's parallel loop: the same bytes)
        s.set_cpu_single(false);
        let banded = cpu_image(&mut s, sh.w, sh.h, BG);
        if banded == cpu {
            same_bands += 1;
        } else {
            let (n, m) = differ(&banded, &cpu);
            t.ok(false, &format!("{}: the CPU path in row bands draws other bytes than on one thread ({n} pixels, largest {m})", sh.name));
        }
        // a scene without a device: the same bytes
        let Some(alone) = t.res(cpu_shot(&sh), &sh.name) else { return };
        shots_n += 1;
        if alone == cpu {
            same_cpu += 1;
        } else {
            let (n, m) = differ(&alone, &cpu);
            t.ok(false, &format!("{}: a scene without a device draws other bytes than the CPU path of the same scene on a device ({n} pixels, largest {m})", sh.name));
        }
        hash = fnv(hash, &cpu);
        crate::tests::dump(&format!("cpu_{}", sh.name), &cpu, sh.w, sh.h);
        crate::tests::dump(&format!("gpu_{}", sh.name), &gpu_img, sh.w, sh.h);
        let c: Cmp = refimg::compare(&gpu_img, &cpu, sh.w, BG, 2);
        let cat = category(&sh.name);
        let k = match sums.iter().position(|x| x.name == cat) {
            Some(k) => k,
            None => {
                sums.push(Sum { name: cat.into(), ..Sum::default() });
                sums.len() - 1
            }
        };
        let su = &mut sums[k];
        su.images += 1;
        su.pixels += (sh.w * sh.h) as u64;
        su.over += c.over as u64;
        if c.max >= su.max {
            su.max = c.max;
            su.at = format!("{} {:?}", sh.name, c.at);
        }
        su.ink[0] += c.ink[0];
        su.ink[1] += c.ink[1];
        crate::tests::out(&format!("        {}: GPU against CPU {}", sh.name, c.text()));
        // (plain drawings) both against lib/gfx's reference, which neither path shares any code with
        if sh.b.is_none() && sh.t.is_none() && sh.x.is_none() {
            let reference = refimg::draw(&sh.d, &sh.cam, (sh.w, sh.h), BG);
            let (rg, rc) = (refimg::compare(&gpu_img, &reference, sh.w, BG, 2), refimg::compare(&cpu, &reference, sh.w, BG, 2));
            su.refs += 1;
            su.ref_over[0] += rg.over as u64;
            su.ref_over[1] += rc.over as u64;
            su.ref_max[0] = su.ref_max[0].max(rg.max);
            su.ref_max[1] = su.ref_max[1].max(rc.max);
            su.ref_ink[0] += rg.ink[0];
            su.ref_ink[1] += rc.ink[0];
            su.ref_ink[2] += rg.ink[1];
        }
    }
    t.ok(same_cpu == shots_n, &format!("a scene without a device (Scene::new_cpu) draws byte for byte what the CPU path draws of the same scene on a device: {same_cpu} of {shots_n} images"));
    t.ok(same_bands == shots_n, &format!("the CPU path in row bands on lib/gfx's parallel loop draws byte for byte what it draws on one thread: {same_bands} of {shots_n} images"));
    // the demo's scene (720 x 480, 1 unit a pixel) on the CPU: what the window without a device paints per frame
    {
        let mut s = Scene::new_cpu();
        let _ = crate::demo::fill(&mut s);
        let _ = s.set_camera(&Camera::new([0.0, 0.0], 1.0));
        let (ms1, _, _, _) = bench_view_single(&mut s, true, (720, 480), 9);
        let (ms2, _, _, _) = bench_view_single(&mut s, false, (720, 480), 9);
        crate::tests::out(&format!("  the demo's scene on the CPU path, 720 x 480: {ms1:.2} ms per frame on one thread, {ms2:.2} ms in row bands (median of 9)"));
    }
    crate::tests::out(&format!("  the CPU path's images: FNV-1a {hash:016x} (the same on every target)"));
    let (mut over, mut px) = (0u64, 0u64);
    for su in &sums {
        over += su.over;
        px += su.pixels;
        let ink = 100.0 * (su.ink[0] - su.ink[1]) / su.ink[1].max(1.0);
        // what may differ per category (docs/RENDER2D.md "Der CPU-Weg"): the share of pixels beyond 2 codes, the largest
        // difference, the ink (%): straight edges and fills alike; round caps and small circles the GPU's tangent; short
        // segments its fast way; dash patterns its three dashes per pixel and side; dashed curves its caps two segments on;
        // fills' edges through the middle of a pixel's side; text MSDF against outlines
        let (share, most, inkl) = match su.name.as_str() {
            "miter" | "hairlines" | "flat" | "fills" | "overlap" => (0.0005, 4, 0.05),
            "joins" => (0.002, 16, 0.1),
            "hatch" => (0.004, 16, 0.1),
            "caps" | "arcs" => (0.002, 64, 0.2),
            "blocks" => (0.012, 48, 0.4),
            "units" | "dashes" => (0.006, 96, 0.3),
            "curves" => (0.006, 150, 0.3),
            "short" => (0.02, 210, 0.3),
            "transparency" | "highlight" | "overlay" => (0.005, 160, 0.3),
            _ => (0.012, 135, 0.5),
        };
        let fine = (su.over as f64) <= share * su.pixels as f64 && su.max <= most && ink.abs() <= inkl;
        t.ok(fine, &format!("{:<13} {:>2} images: {:>6} of {:>8} pixels beyond 2 codes, largest {:>3} ({}), ink GPU against CPU {:+.3} % (at most {:.1} %, {most}, {inkl} %)", su.name, su.images, su.over,
                            su.pixels, su.max, su.at, ink, 100.0 * share));
        if su.refs > 0 {
            let ri = |k: usize| 100.0 * (su.ref_ink[k] - su.ref_ink[2]) / su.ref_ink[2].max(1.0);
            crate::tests::out(&format!("                against lib/gfx ({} images): GPU {} pixels beyond 2 codes, largest {}, ink {:+.3} %; CPU {} pixels, largest {}, ink {:+.3} %", su.refs, su.ref_over[0], su.ref_max[0], ri(0),
                                       su.ref_over[1], su.ref_max[1], ri(1)));
        }
    }
    crate::tests::out(&format!("  all: {over} of {px} pixels beyond 2 codes ({:.3} %)", 100.0 * over as f64 / px.max(1) as f64));
    // a scene taken off its device and put on another renderer: everything goes up again, the same image on the GPU
    attach_again(t, &dev, &mut r);
    coords(t);
}

/// a drawing's image on the CPU path, a scene without a device (bg: its background)
fn cpu_drawing(d: &Drawing, cam: &Camera, size: (u32, u32), bg: u32) -> Option<Vec<u8>> {
    let mut s = Scene::new_cpu();
    s.set_camera(cam).ok()?;
    build(&mut s, d).ok()?;
    Some(cpu_image(&mut s, size.0, size.1, bg))
}

/// the CPU path at large coordinates, as the GPU's coords and phase tests judge it: the same drawing at +-1e7 as at the origin
/// (lines, arcs and dashes, curves, fills and hatches, as a block); at 1e-6 units per pixel near 1e7 lines, rings, curves, a
/// hatch, inserts and dash patterns where they belong within 1/16 px over 24 pans (the guard band: lib/gfx sees small numbers)
fn coords(t: &mut T) {
    let (w, h) = FAR_SIZE;
    for (what, f) in [("the drawing", sample as fn([f64; 2]) -> Drawing), ("arcs, circles and dash patterns", arc_sample), ("curves, a fill with a hole and a hatched fill", curve_sample)] {
        let Some(base) = cpu_drawing(&f([0.0, 0.0]), &sample_camera([0.0, 0.0]), (w, h), BG) else {
            t.ok(false, "the CPU path at the origin");
            return;
        };
        for at in FAR_POINTS {
            let Some(img) = cpu_drawing(&f(at), &sample_camera(at), (w, h), BG) else { continue };
            let (n, m) = differ(&img, &base);
            t.ok(m <= 1 && n <= (w * h) / 1000, &format!("CPU path: {what} at ({}, {}) against at the origin: {n} pixels differ, largest {m}", at[0], at[1]));
        }
    }
    let (dw, dh) = DEEP_SIZE;
    let black = 0xFF00_0000;
    // lines
    let mut s = Scene::new_cpu();
    let _ = build(&mut s, &deep_drawing());
    let mut st = DeepStats::default();
    for step in 0..DEEP_STEPS {
        let c = deep_camera(step);
        let _ = s.set_camera(&c);
        deep_judge(&cpu_image(&mut s, dw, dh, black), &c, &mut st);
    }
    t.ok(st.fine(), &format!("CPU path: {}", st.text()));
    // rings
    let mut s = Scene::new_cpu();
    let _ = build(&mut s, &deep_arc_drawing());
    let mut st = DeepStats::default();
    for step in 0..DEEP_STEPS {
        let c = deep_camera(step);
        let _ = s.set_camera(&c);
        deep_arc_judge(&cpu_image(&mut s, dw, dh, black), &c, &mut st);
    }
    t.ok(st.fine_arcs(), &format!("CPU path: {}", st.arc_text()));
    // curves (flattened in a window)
    let (d, fine) = deep_curves();
    let mut s = Scene::new_cpu();
    let _ = s.set_camera(&deep_camera(0));
    let _ = build(&mut s, &d);
    let mut st = DeepStats::default();
    for step in 0..DEEP_STEPS {
        let c = deep_camera(step);
        let _ = s.set_camera(&c);
        deep_curve_judge(&cpu_image(&mut s, dw, dh, black), &c, &fine, &mut st);
    }
    t.ok(st.checks > 100 && st.lost == 0 && st.worst <= 1.0 / 16.0, &format!("CPU path: curves at 1e-6 units per pixel near 1e7 (an ellipse of radii 1e7 and 6e6, a Bézier with controls 1e6 away), 24 pans: {} \
                                                                        crossings, the largest distance from where the curve belongs {:.4} px, {} without it", st.checks, st.worst, st.lost));
    // a hatch: its lines and outline
    let (mut sh, mut ss) = (Scene::new_cpu(), Scene::new_cpu());
    let _ = sh.set_camera(&deep_camera(0));
    let _ = ss.set_camera(&deep_camera(0));
    let _ = build(&mut sh, &deep_hatch(false));
    let _ = build(&mut ss, &deep_hatch(true));
    let (mut lines, mut edge) = (DeepStats::default(), DeepStats::default());
    for step in 0..DEEP_STEPS {
        let c = deep_camera(step);
        let _ = sh.set_camera(&c);
        let _ = ss.set_camera(&c);
        let (a, b) = (cpu_image(&mut sh, dw, dh, black), cpu_image(&mut ss, dw, dh, black));
        deep_hatch_judge(&a, &b, &c, &mut lines, &mut edge);
    }
    t.ok(lines.checks > 100 && lines.lost == 0 && lines.worst <= 1.0 / 16.0 && edge.checks > 100 && edge.worst <= 1.0 / 16.0,
         &format!("CPU path: a hatch at 1e-6 units per pixel near 1e7, 24 pans: the lines' centroids at most {:.4} px from where they belong ({} columns), the outline's edge at most {:.4} px ({} rows)",
                  lines.worst, lines.checks, edge.worst, edge.checks));
    // inserts: the deep lines as a block moved there, scaled by 1e5 and turned, mirrored too
    for (name, scale, rot, mirror) in [("moved there", 1.0f64, 0.0f64, false), ("scaled by 1e5, turned", 1.0e5, 0.3, false), ("scaled by 1e5, turned, mirrored", 1.0e5, -1.1, true)] {
        let mut s = Scene::new_cpu();
        let _ = s.set_camera(&deep_camera(0));
        let _ = build_blocks(&mut s, &deep_insert_drawing(scale, rot, mirror));
        let mut st = DeepStats::default();
        for step in 0..DEEP_STEPS {
            let c = deep_camera(step);
            let _ = s.set_camera(&c);
            deep_judge(&cpu_image(&mut s, dw, dh, black), &c, &mut st);
        }
        t.ok(st.fine(), &format!("CPU path: an insert {name}: {}", st.text()));
    }
    // dash patterns far away (phase)
    for k in 0..PHASE_CASES {
        let mut s = Scene::new_cpu();
        let _ = build(&mut s, &phase_case(k, 0).d);
        let (mut worst, mut checks, mut name) = (0.0f64, 0u32, "");
        for step in 0..24 {
            let case = phase_case(k, step);
            name = case.name;
            let _ = s.set_camera(&case.cam);
            phase_judge(&cpu_image(&mut s, PHASE_SIZE.0, PHASE_SIZE.1, black), &case, k, &mut worst, &mut checks);
        }
        t.ok(checks >= 100 && worst <= 1.0 / 16.0, &format!("CPU path: dashes at 1e-6 units per pixel, {name}: {checks} pixel lines over 24 pans, the largest distance of a dash's end from where it belongs \
                                                             {worst:.4} px (1/16 = 0.0625)"));
    }
}

/// detach and attach: the image on the GPU before, after attaching to a new renderer, and of a scene made without a device
/// then attached
fn attach_again(t: &mut T, dev: &Device, r: &mut Renderer) {
    for name in ["joins_Round_9", "blocks", "text_1", "highlight", "hatch_7", "dashes_Round_Round_3"] {
        let Some(sh) = shots().into_iter().find(|x| x.name == name) else {
            t.ok(false, &format!("no shot {name}"));
            continue;
        };
        let Some(mut off) = t.res(Off::new(dev, sh.w, sh.h), "target") else { return };
        let Some(mut s) = t.res(Scene::new(r), "Scene::new") else { return };
        if t.res(s.set_camera(&sh.cam), name).is_none() || t.res(sh.build(&mut s), name).is_none() {
            return;
        }
        let Some(before) = shot(t, dev, &mut off, r, &mut s, name) else { return };
        s.detach();
        let refused = s.prepare(r, (sh.w, sh.h)).is_err();
        let Some(mut r2) = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new") else { return };
        if t.res(s.attach(&r2), name).is_none() {
            return;
        }
        let Some(again) = shot(t, dev, &mut off, &mut r2, &mut s, name) else { return };
        // made without a device, attached afterwards
        let mut s3 = Scene::new_cpu();
        let _ = s3.set_camera(&sh.cam);
        if t.res(sh.build(&mut s3), name).is_none() || t.res(s3.attach(r), name).is_none() {
            return;
        }
        let Some(late) = shot(t, dev, &mut off, r, &mut s3, name) else { return };
        let (n1, m1) = differ(&before, &again);
        let (n2, m2) = differ(&before, &late);
        t.ok(refused && n1 == 0 && n2 == 0, &format!("{name}: detached, prepare refused ({refused}); attached to another renderer: {n1} pixels differ (largest {m1}); made without a device and attached: {n2} pixels \
                                                      differ (largest {m2})"));
    }
}

/// the CPU path's time of a scene at its camera, on one thread or in row bands (the median of `frames`)
pub fn bench_view_single(s: &mut Scene, single: bool, size: (u32, u32), frames: usize) -> (f64, Option<Cmp>, render2d::CpuInfo, Vec<u8>) {
    s.set_cpu_single(single);
    let cam = *s.camera();
    let r = bench_view(s, &cam, size, frames, None, BG);
    s.set_cpu_single(false);
    r
}

/// the CPU path's time and its image against the GPU's for a view of a scene (bench.rs): ms per frame (the median of
/// `frames`), the comparison
pub fn bench_view(s: &mut Scene, cam: &Camera, size: (u32, u32), frames: usize, gpu_img: Option<&[u8]>, bg: u32) -> (f64, Option<Cmp>, render2d::CpuInfo, Vec<u8>) {
    let _ = s.set_camera(cam);
    let mut img = Vec::new();
    let mut times = Vec::new();
    let mut sf = Surface::new(size.0 as i32, size.1 as i32, FMT_XRGB);
    let mut dc = Dc::new(size.0 as i32, size.1 as i32);
    dc.begin(&sf);
    for k in 0..frames.max(1) + 1 {
        dc.clear(&mut sf, bg | 0xFF00_0000);
        let t0 = plat::time_us();
        s.draw_cpu(&mut dc, &mut sf, [0, 0, size.0 as i32, size.1 as i32]);
        let us = plat::time_us() - t0;
        if k > 0 {
            times.push(us as f64 / 1000.0);
        }
    }
    for y in 0..size.1 as usize {
        for p in sf.px[y * sf.stride as usize..][..size.0 as usize * 4].chunks_exact(4) {
            img.extend_from_slice(&[p[0], p[1], p[2], 255]);
        }
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let med = times[times.len() / 2];
    let cmp = gpu_img.map(|g| refimg::compare(g, &img, size.0, bg, 2));
    (med, cmp, s.cpu_info(), img)
}
