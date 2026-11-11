// tests -- render2dtest's tests on Vulkan (main.rs lists them)

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render2d::{Camera, Cap, Join, Layer, Renderer, RendererDesc, Scene, Shape, Style, Unit};

use crate::gallery::*;
use crate::refimg::{self, Tol};

/// the test state
pub struct T {
    pub fails: u32,
    pub validate: bool,
}

pub fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
    plat::write_stdout(b"\n");
}

impl T {
    pub fn ok(&mut self, cond: bool, what: &str) -> bool {
        if cond {
            out(&format!("  ok   {what}"));
        } else {
            self.fails += 1;
            out(&format!("FAIL {what}"));
        }
        cond
    }

    pub fn res<V>(&mut self, r: gpu::Result<V>, what: &str) -> Option<V> {
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                self.ok(false, &format!("{what}: {e}"));
                None
            }
        }
    }
}

/// instance and adapter of a run
pub struct Gpu {
    /// (held so the adapter's instance lives as long)
    #[allow(dead_code)]
    pub inst: Instance,
    pub adapter: Adapter,
}

/// None (SKIP, not a failure) without Vulkan or a usable GPU
pub fn setup(t: &mut T) -> Option<Gpu> {
    let inst = match Instance::with(&InstanceDesc { app_name: "render2dtest", validate: t.validate }) {
        Ok(i) => i,
        Err(e @ (GpuError::NoLoader | GpuError::NoAdapter | GpuError::Disabled)) => {
            out(&format!("SKIP: {e}"));
            return None;
        }
        Err(e) => {
            t.ok(false, &format!("Instance::with: {e}"));
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
            t.ok(false, &format!("Instance::adapter: {e}"));
            return None;
        }
    };
    Some(Gpu { inst, adapter })
}

/// a test on the adapter's own level and on a device limited to Basis
pub fn on_levels(t: &mut T, f: &mut dyn FnMut(&mut T, &Device, &str)) {
    let Some(g) = setup(t) else { return };
    if g.adapter.level() == Level::Voll {
        if let Some(d) = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "device") {
            f(t, &d, "Voll");
        }
    }
    if let Some(d) = t.res(g.adapter.create_device(&DeviceDesc { max_level: Level::Basis }), "Basis device") {
        f(t, &d, "Basis");
    }
}

/// submit, wait, read the image
pub fn shot(t: &mut T, dev: &Device, off: &mut Off, r: &mut Renderer, s: &mut Scene, what: &str) -> Option<Vec<u8>> {
    let id = t.res(off.submit(r, s), what)?;
    if !t.res(dev.wait(id, 10_000_000_000), what)? {
        t.ok(false, &format!("{what}: the GPU did not finish within 10 s"));
        return None;
    }
    t.res(off.read(), what)
}

/// a drawing on the GPU in a new scene: its image
pub fn gpu_image(t: &mut T, dev: &Device, r: &mut Renderer, off: &mut Off, d: &Drawing, cam: &Camera, what: &str) -> Option<Vec<u8>> {
    let mut s = t.res(Scene::new(r), "Scene::new")?;
    t.res(build(&mut s, d), what)?;
    t.res(s.set_camera(cam), what)?;
    shot(t, dev, off, r, &mut s, what)
}

/// optional dumps of images for a person to look at: R2D_DUMP=DIR (raw B, G, R, A rows; tools convert them)
pub fn dump(name: &str, img: &[u8], w: u32, h: u32) {
    if let Some(dir) = plat::env("R2D_DUMP") {
        let mut v = Vec::with_capacity(img.len() + 8);
        v.extend_from_slice(&w.to_le_bytes());
        v.extend_from_slice(&h.to_le_bytes());
        v.extend_from_slice(img);
        let _ = plat::write_file(&format!("{dir}/{name}.bgra"), &v);
    }
}

/// a drawing against lib/gfx
pub fn against_ref(t: &mut T, dev: &Device, r: &mut Renderer, off: &mut Off, d: &Drawing, cam: &Camera, tol: &Tol, what: &str) -> Option<refimg::Cmp> {
    let img = gpu_image(t, dev, r, off, d, cam, what)?;
    let reference = refimg::draw(d, cam, (off.w, off.h), off.bg);
    let c = refimg::compare(&img, &reference, off.w, off.bg, tol.small);
    let name: String = what.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    dump(&format!("{name}_gpu"), &img, off.w, off.h);
    dump(&format!("{name}_ref"), &reference, off.w, off.h);
    t.ok(c.fine(tol), &format!("{what}: against lib/gfx: {}", c.text()));
    Some(c)
}

// ---------------------------------------------------------------- strokes against lib/gfx

fn caps(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = caps_on(t, dev, level); });
}

fn caps_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (320u32, 240u32);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    let cam = pixel_camera(w, h);
    for cap in CAPS {
        for width in [1.0f32, 2.5, 6.0, 15.0] {
            let d = caps_drawing(cap, width);
            let tol = if cap == Cap::Round { Tol::round(26) } else { Tol::STRAIGHT };
            against_ref(t, dev, &mut r, &mut off, &d, &cam, &tol, &format!("{level}: {cap:?} caps, width {width}"))?;
        }
    }
    Some(())
}

fn joins(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = joins_on(t, dev, level); });
}

fn joins_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (360u32, 260u32);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    let cam = pixel_camera(w, h);
    for join in JOINS {
        for width in [1.0f32, 3.0, 9.0, 18.0] {
            let d = joins_drawing(join, width);
            against_ref(t, dev, &mut r, &mut off, &d, &cam, &Tol::corners(40), &format!("{level}: {join:?} joins, width {width}"))?;
        }
    }
    // the miter limit: a turn whose miter is 3.9 and one whose miter is 4.1 widths long, limits 4 and 10
    for limit in [4.0f32, 10.0] {
        let d = miter_drawing(limit);
        against_ref(t, dev, &mut r, &mut off, &d, &cam, &Tol::STRAIGHT, &format!("{level}: miter limit {limit}"))?;
    }
    Some(())
}

fn short(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = short_on(t, dev, level); });
}

/// short segments and sharp turns: the near-join pieces of every kind (tiles, unions, chords crossing inside a segment)
fn short_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (320u32, 240u32);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    let cam = pixel_camera(w, h);
    for (seed, width, join) in [(1u32, 0.0f32, Join::Miter), (2, 1.0, Join::Miter), (3, 3.0, Join::Round), (4, 1.0, Join::Bevel), (5, 2.0, Join::Round), (6, 0.0, Join::Bevel)] {
        let d = short_drawing(seed, width, join);
        // (random polylines of sub-pixel-scale turns fold over themselves everywhere: lib/gfx fills them with its winding sums,
        // render2d's union differs from them by design, RENDER2D.md "Tests"; the join geometry itself is checked by the images
        // of the browser test against Vulkan and by the other tests here)
        against_ref(t, dev, &mut r, &mut off, &d, &cam, &Tol { small: 2, over: 2600, max: 255, ink: 0.06 }, &format!("{level}: short segments, {join:?} joins, width {width}"))?;
    }
    Some(())
}

fn hairlines(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = hairlines_on(t, dev, level); });
}

fn hairlines_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (300u32, 300u32);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    let cam = pixel_camera(w, h);
    for (width, what) in [(0.0f32, "width 0"), (0.4, "width 0.4 (drawn as 1)"), (1.0, "width 1")] {
        let d = hairlines_drawing(width);
        against_ref(t, dev, &mut r, &mut off, &d, &cam, &Tol::STRAIGHT, &format!("{level}: hairlines, {what}"))?;
    }
    Some(())
}

fn units(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = units_on(t, dev, level); });
}

fn units_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (320u32, 240u32);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    let d = units_drawing();
    for (scale, rot) in UNIT_VIEWS {
        let cam = units_camera(scale, rot);
        against_ref(t, dev, &mut r, &mut off, &d, &cam, &Tol::corners(24), &format!("{level}: widths in units and pixels, scale {scale}, rotation {rot}"))?;
    }
    Some(())
}

fn overlap(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = overlap_on(t, dev, level); });
}

fn overlap_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (320u32, 240u32);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    let cam = pixel_camera(w, h);
    for join in JOINS {
        for alpha in [0x80u32, 0x40] {
            let d = overlap_drawing(join, alpha);
            let what = format!("{level}: translucent ({alpha:#x}) polylines with {join:?} joins at sharp corners");
            let Some(img) = gpu_image(t, dev, &mut r, &mut off, &d, &cam, &what) else { return None };
            let reference = refimg::draw(&d, &cam, (w, h), BG);
            let tol = Tol::corners(16);
            let cmp = refimg::compare(&img, &reference, w, BG, tol.small);
            t.ok(cmp.fine(&tol), &format!("{what}: against lib/gfx (blended once): {}", cmp.text()));
            // no pixel darker than the colour over the background once: blended twice would be
            let twice = blended_twice(&img, alpha);
            t.ok(twice == 0, &format!("{what}: no pixel beyond one blend of the colour: {twice}"));
        }
    }
    Some(())
}

// ---------------------------------------------------------------- arcs and circles

fn arcs(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = arcs_on(t, dev, level); });
}

fn arcs_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (320u32, 240u32);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    let cam = pixel_camera(w, h);
    for cap in CAPS {
        for width in [1.0f32, 3.0, 9.0] {
            let d = arcs_drawing(cap, width);
            against_ref(t, dev, &mut r, &mut off, &d, &cam, &refimg::Tol::ARCS, &format!("{level}: arcs and circles, {cap:?} caps, width {width}"))?;
        }
    }
    // translucent rings and arcs: where two pieces of an arc meet, nothing blends twice
    for alpha in [0x80u32, 0x40] {
        let d = arcs_overlap_drawing(alpha);
        let what = format!("{level}: translucent ({alpha:#x}) rings and arcs");
        let img = gpu_image(t, dev, &mut r, &mut off, &d, &cam, &what)?;
        let reference = refimg::draw(&d, &cam, (w, h), BG);
        let tol = refimg::Tol::ARCS;
        let cmp = refimg::compare(&img, &reference, w, BG, tol.small);
        dump(&format!("arcs_overlap_{alpha:x}_{level}_gpu"), &img, w, h);
        dump(&format!("arcs_overlap_{alpha:x}_{level}_ref"), &reference, w, h);
        t.ok(cmp.fine(&tol), &format!("{what}: against lib/gfx (blended once): {}", cmp.text()));
        let twice = blended_twice(&img, alpha);
        t.ok(twice == 0, &format!("{what}: no pixel beyond one blend of the colour: {twice}"));
    }
    // what a circle filling a full-HD target costs the fragment shader: the pixels its pieces' quads cover (vs_arc's
    // quads, computed again on the CPU) against its ring and against its bounding square
    for (rp, h) in [(540.0f64, 0.5f64), (540.0, 2.0), (60.0, 0.5), (5000.0, 0.5)] {
        let (quads, ring, square) = arc_quad_area(rp, h, (1920, 1080));
        t.ok(quads <= (if rp < 1100.0 { 5.0 } else { 10.0 }) * ring, &format!("a circle of radius {rp} px, half width {h} px in 1920 x 1080: its quads cover {quads:.0} px, its ring and the smoothed edge {ring:.0} px ({:.2} times), \
                                           its bounding square {square:.0} px", quads / ring));
    }
    // a layer with an arc draws with fs_full; without it again with fs: both as a scene built anew
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    let mut d = joins_drawing(Join::Round, 3.0);
    let b = t.res(build(&mut s, &d), "build")?;
    t.res(s.set_camera(&cam), "camera")?;
    let sid = t.res(s.add_style(&style(INK[2], 4.0, Cap::Round, Join::Round)), "add_style")?;
    let arc = t.res(s.insert(b.layers[0], sid, &Shape::Arc { center: [180.0, 130.0], radius: 50.0, start: 0.5, sweep: 2.5 }), "insert")?;
    d.add(style(INK[2], 4.0, Cap::Round, Join::Round), Geo::Arc([180.0, 130.0], 50.0, 0.5, 2.5));
    let img = shot(t, dev, &mut off, &mut r, &mut s, "with an arc")?;
    let fresh = gpu_image(t, dev, &mut r, &mut off, &d, &cam, "with an arc, anew")?;
    let (n1, _) = differ(&img, &fresh);
    t.res(s.remove(arc), "remove")?;
    d.items.pop();
    let img = shot(t, dev, &mut off, &mut r, &mut s, "the arc removed")?;
    let fresh = gpu_image(t, dev, &mut r, &mut off, &d, &cam, "without it, anew")?;
    let (n2, _) = differ(&img, &fresh);
    t.ok(n1 == 0 && n2 == 0, &format!("{level}: a layer takes fs_full with an arc and fs without: against scenes built anew {n1} and {n2} pixels differ"));
    // solid strokes drawn by fs_full (their layer holds an arc, out of view) against fs: near joins the union stands for
    // the tiles (the same area where the tiles hold: joins and caps the same to a code or two).  Short segments with sharp
    // turns: both take the union, with other approximations where a short segment is near both its joins (RENDER2D.md
    // "Nicht gebaut"); there both against lib/gfx with the tolerance of the test short
    let cases = [
        ("miter joins, width 3", joins_drawing(Join::Miter, 3.0), false),
        ("round joins, width 9", joins_drawing(Join::Round, 9.0), false),
        ("bevel joins, width 1", joins_drawing(Join::Bevel, 1.0), false),
        ("round caps, width 9", caps_drawing(Cap::Round, 9.0), false),
        ("short segments, width 1", short_drawing(2, 1.0, Join::Miter), true),
        ("short segments, width 3", short_drawing(3, 3.0, Join::Round), true),
    ];
    for (name, d, short) in cases {
        let a = gpu_image(t, dev, &mut r, &mut off, &d, &cam, name)?;
        let reference = refimg::draw(&d, &cam, (w, h), BG);
        let mut d = d;
        d.add(style(INK[2], 1.0, Cap::Butt, Join::Miter), Geo::Arc([-1e4, -1e4], 5.0, 0.0, 1.0));
        let b = gpu_image(t, dev, &mut r, &mut off, &d, &cam, name)?;
        let (n, m) = differ(&a, &b);
        let over = a.chunks(4).zip(b.chunks(4)).filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > 2)).count();
        let what = format!("{level}: solid strokes in a layer of fs_full against fs, {name}: {n} pixels differ, {over} by more than 2 codes, largest {m}");
        if short {
            let tol = Tol { small: 2, over: 2600, max: 255, ink: 0.06 };
            let ca = refimg::compare(&a, &reference, w, BG, tol.small);
            let cb = refimg::compare(&b, &reference, w, BG, tol.small);
            t.ok(cb.fine(&tol), &format!("{what}; against lib/gfx fs {}, fs_full {}", ca.text(), cb.text()));
        } else {
            t.ok(m <= 2 || over <= 8, &what);
        }
    }
    Some(())
}

// ---------------------------------------------------------------- dash patterns

fn dashes(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = dashes_on(t, dev, level); });
}

fn dashes_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (360u32, 270u32);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    let cam = pixel_camera(w, h);
    for (cap, join, width) in DASH_CASES {
        let d = dash_drawing(cap, join, width);
        let tol = if cap == Cap::Round { refimg::Tol::DASH_ROUND } else { refimg::Tol::DASH };
        against_ref(t, dev, &mut r, &mut off, &d, &cam, &tol, &format!("{level}: dash patterns, {cap:?} caps, {join:?} joins, width {width}"))?;
        if level == "Voll" {
            // the reference's own dashes (dash_pieces) against lib/gfx's dash_stroke, on the open lines
            let mut open = Drawing::one_layer();
            for it in &d.items {
                if matches!(it.geo, Geo::Line(..)) || matches!(it.geo, Geo::Poly(_, false)) {
                    open.add(it.style, it.geo.clone());
                }
            }
            let a = refimg::draw(&open, &cam, (w, h), BG);
            let b = refimg::draw_with(&open, &cam, (w, h), BG, true);
            let c = refimg::compare(&a, &b, w, BG, 1);
            t.ok(c.max <= 2, &format!("the reference's dashes (dash_pieces) against lib/gfx's dash_stroke, {cap:?} caps, width {width}: {}", c.text()));
        }
    }
    // patterns in drawing units
    let (uw, uh) = (320u32, 240u32);
    let mut uoff = t.res(Off::new(dev, uw, uh), "target")?;
    for (scale, rot) in UNIT_VIEWS {
        let d = units_dash_drawing();
        against_ref(t, dev, &mut r, &mut uoff, &d, &units_camera(scale, rot), &refimg::Tol::DASH_ROUND, &format!("{level}: dash patterns in drawing units, scale {scale}, rotation {rot}"))?;
    }
    // flat tones: the ink of lib/gfx's real dashes; along the lines a pan of 0.37 px changes nothing where the tone is
    // flat, and the dashes where they are exact
    let (fw, fh) = (320u32, 200u32);
    let mut foff = t.res(Off::new(dev, fw, fh), "target")?;
    for period in FLAT_PERIODS {
        let d = flat_drawing(period, 2.0, Cap::Butt);
        let cam0 = pixel_camera(fw, fh);
        let img = gpu_image(t, dev, &mut r, &mut foff, &d, &cam0, "flat")?;
        let reference = refimg::draw(&d, &cam0, (fw, fh), BG);
        dump(&format!("flat_{period}_{level}_gpu"), &img, fw, fh);
        dump(&format!("flat_{period}_{level}_ref"), &reference, fw, fh);
        let c = refimg::compare(&img, &reference, fw, BG, 2);
        let ink = (c.ink[0] - c.ink[1]) / c.ink[1];
        t.ok(ink.abs() <= 0.01, &format!("{level}: dashes of a period of {period} px: ink against lib/gfx's real dashes {:+.2} %", 100.0 * ink));
        let mut s = t.res(Scene::new(&r), "Scene::new")?;
        t.res(build(&mut s, &d), "build")?;
        let mut moved = 0u32;
        let mut prev: Option<Vec<u8>> = None;
        for step in 0..6 {
            let mut c2 = cam0;
            c2.pan(0.37 * step as f64, 0.0);
            t.res(s.set_camera(&c2), "camera")?;
            let img = shot(t, dev, &mut foff, &mut r, &mut s, "pan")?;
            if let Some(p) = &prev {
                // (the lines' middle: 20 px from their ends)
                for y in 0..fh as usize {
                    for x in 32..fw as usize - 32 {
                        let k = (y * fw as usize + x) * 4;
                        moved += (p[k..k + 3] != img[k..k + 3]) as u32;
                    }
                }
            }
            prev = Some(img);
        }
        if period <= 2.0 {
            t.ok(moved == 0, &format!("{level}: a period of {period} px is a flat tone: five pans of 0.37 px along the lines change {moved} pixels of their middles"));
        } else if period >= 4.0 {
            t.ok(moved > 0, &format!("{level}: a period of {period} px is drawn exactly: the pans move the dashes ({moved} pixels change)"));
        }
    }
    // a style made dashed and solid again (set_style): its layer changes pipeline; each image as a scene built anew
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    let mut d = joins_drawing(Join::Miter, 3.0);
    t.res(build(&mut s, &d), "build")?;
    t.res(s.set_camera(&cam), "camera")?;
    let mut ok = true;
    for (k, pat) in [&[14.0f32, 5.0][..], &[][..], &[6.0, 3.0, 1.0][..]].into_iter().enumerate() {
        let st = d.items[0].style.dashed(pat, Unit::Px, 2.0 * k as f32);
        t.res(s.set_style(render2d::StyleId(0), &st), "set_style")?;
        d.items[0].style = st;
        let img = shot(t, dev, &mut off, &mut r, &mut s, "restyled")?;
        let fresh = gpu_image(t, dev, &mut r, &mut off, &d, &cam, "restyled, anew")?;
        ok &= differ(&img, &fresh).0 == 0;
    }
    t.ok(ok, &format!("{level}: a style dashed, solid and dashed again (set_style): each image as a scene built anew"));
    Some(())
}

// ---------------------------------------------------------------- layers and order

fn layers(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = layers_on(t, dev, level); });
}

fn layers_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (256u32, 192u32);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    let cam = pixel_camera(w, h);
    // three layers out of order, one hidden; translucent bars crossing in every layer
    let mut d = Drawing { layers: vec![(true, 5), (true, -2), (false, 0), (true, 1)], items: Vec::new() };
    let mut rng = Rng(7);
    for k in 0..60 {
        let layer = rng.below(4) as usize;
        let a = [rng.range(10.0, 246.0), rng.range(10.0, 182.0)];
        let b = [rng.range(10.0, 246.0), rng.range(10.0, 182.0)];
        let c = ((0x60 + rng.below(0x9F)) << 24) | (rng.next() & 0x00FF_FFFF);
        let st = style(c, rng.range(2.0, 14.0) as f32, CAPS[k % 3], JOINS[k % 3]);
        let geo = if k % 3 == 0 { Geo::Poly(vec![a, [rng.range(10.0, 246.0), rng.range(10.0, 182.0)], b], false) } else { Geo::Line(a, b) };
        d.items.push(Item { layer, style: st, geo });
    }
    against_ref(t, dev, &mut r, &mut off, &d, &cam, &Tol::corners(150), &format!("{level}: 60 translucent entities on four layers (one hidden), orders 5, -2, 1"))?;
    // the same scene, its layers turned around and shown, through set_layer
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    let b = t.res(build(&mut s, &d), "build")?;
    t.res(s.set_camera(&cam), "camera")?;
    let _ = shot(t, dev, &mut off, &mut r, &mut s, "first frame")?;
    let mut d2 = d.clone();
    d2.layers = vec![(true, 0), (true, 9), (true, 3), (true, 3)];
    for (k, &(v, o)) in d2.layers.iter().enumerate() {
        let mut l = Layer::new(o);
        l.visible = v;
        t.res(s.set_layer(b.layers[k], &l), "set_layer")?;
    }
    let img = shot(t, dev, &mut off, &mut r, &mut s, "after set_layer")?;
    let info = s.upload_info();
    let reference = refimg::draw(&d2, &cam, (w, h), BG);
    let tol = Tol::corners(150);
    let cmp = refimg::compare(&img, &reference, w, BG, tol.small);
    t.ok(cmp.fine(&tol), &format!("{level}: the layers ordered anew and all shown (set_layer): against lib/gfx: {}", cmp.text()));
    t.ok(info.records == 0 && !info.rebuild, &format!("{level}: set_layer sends no records ({} records, rebuild {})", info.records, info.rebuild));
    Some(())
}

// ---------------------------------------------------------------- Voll and Basis

fn levels(t: &mut T) {
    let Some(g) = setup(t) else { return };
    if g.adapter.level() != Level::Voll {
        out("SKIP: the adapter has no level Voll");
        return;
    }
    let (w, h) = (300u32, 220u32);
    let cam = pixel_camera(w, h);
    let mut d = Drawing { layers: vec![(true, 1), (true, 0), (true, 2)], items: Vec::new() };
    let mut rng = Rng(3);
    for k in 0..300 {
        let a = [rng.range(0.0, 300.0), rng.range(0.0, 220.0)];
        let pts: Vec<[f64; 2]> = (0..1 + rng.below(6)).map(|_| [a[0] + rng.range(-40.0, 40.0), a[1] + rng.range(-40.0, 40.0)]).collect();
        let c = ((0x50 + rng.below(0xB0)) << 24) | (rng.next() & 0x00FF_FFFF);
        d.items.push(Item { layer: k % 3, style: style(c, rng.range(0.0, 9.0) as f32, CAPS[k % 3], JOINS[(k / 3) % 3]), geo: Geo::Poly(pts, k % 5 == 0) });
    }
    let mut imgs = Vec::new();
    for max_level in [Level::Voll, Level::Basis] {
        let Some(dev) = t.res(g.adapter.create_device(&DeviceDesc { max_level }), "device") else { return };
        let Some(mut off) = t.res(Off::new(&dev, w, h), "target") else { return };
        let Some(mut r) = t.res(Renderer::new(&dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new") else { return };
        out(&format!("  ({max_level:?}: multi-draw {})", r.multi_draw()));
        let Some(img) = gpu_image(t, &dev, &mut r, &mut off, &d, &cam, "levels") else { return };
        imgs.push(img);
    }
    let n = imgs[0].chunks_exact(4).zip(imgs[1].chunks_exact(4)).filter(|(a, b)| a[..3] != b[..3]).count();
    t.ok(n == 0, &format!("Voll and Basis: 300 entities on three layers, {n} pixels differ"));
}

// ---------------------------------------------------------------- errors

fn errors(t: &mut T) {
    let Some(g) = setup(t) else { return };
    let Some(dev) = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "device") else { return };
    let bad = |r: gpu::Result<()>| matches!(r, Err(GpuError::Invalid(_)));
    t.ok(matches!(Renderer::new(&dev, &RendererDesc::new(Format::Bgra8Srgb)), Err(GpuError::Invalid(_))), "an sRGB target is refused");
    let mut d0 = RendererDesc::new(Format::Bgra8Unorm);
    d0.frames_in_flight = 4;
    t.ok(matches!(Renderer::new(&dev, &d0), Err(GpuError::Invalid(_))), "4 frames in flight are refused");
    let Some(mut r) = t.res(Renderer::new(&dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new") else { return };
    let Some(mut s) = t.res(Scene::new(&r), "Scene::new") else { return };
    let mut st = Style::new(0xFF00_0000);
    st.width = -1.0;
    t.ok(matches!(s.add_style(&st), Err(GpuError::Invalid(_))), "a negative width is refused");
    st.width = 1.0;
    st.miter_limit = 0.5;
    t.ok(matches!(s.add_style(&st), Err(GpuError::Invalid(_))), "a miter limit below 1 is refused");
    let Some(sid) = t.res(s.add_style(&Style::new(0xFF00_0000)), "add_style") else { return };
    let Some(l) = t.res(s.add_layer(&Layer::new(0)), "add_layer") else { return };
    t.ok(matches!(s.insert(l, render2d::StyleId(9), &Shape::Line { a: [0.0; 2], b: [1.0; 2] }), Err(GpuError::Invalid(_))), "a style the scene does not have is refused");
    t.ok(matches!(s.insert(l, sid, &Shape::Line { a: [f64::NAN, 0.0], b: [1.0; 2] }), Err(GpuError::Invalid(_))), "a point that is not finite is refused");
    t.ok(matches!(s.insert(l, sid, &Shape::Polyline { points: &[], closed: false }), Err(GpuError::Invalid(_))), "a polyline without points is refused");
    let Some(e) = t.res(s.insert(l, sid, &Shape::Line { a: [0.0; 2], b: [1.0; 2] }), "insert") else { return };
    t.ok(s.remove(e).is_ok() && bad(s.remove(e)), "a removed entity's id is dead");
    let Some(e2) = t.res(s.insert(l, sid, &Shape::Line { a: [0.0; 2], b: [1.0; 2] }), "insert") else { return };
    t.ok(e2.slot == e.slot && bad(s.update(e, &Shape::Line { a: [0.0; 2], b: [2.0; 2] })), "its place goes to the next one, the old id stays dead");
    t.ok(s.remove_layer(l).is_ok() && bad(s.remove(e2)) && matches!(s.insert(l, sid, &Shape::Line { a: [0.0; 2], b: [1.0; 2] }), Err(GpuError::Invalid(_))),
         "a removed layer takes its entities with it, its id is dead");
    t.ok(matches!(s.set_camera(&Camera::new([0.0; 2], 0.0)), Err(GpuError::Invalid(_))), "a camera of scale 0 is refused");
    // record before prepare, a target of another size, a change after prepare
    let Some(mut off) = t.res(Off::new(&dev, 64, 48), "target") else { return };
    let Some(enc) = t.res(dev.encoder(), "encoder") else { return };
    let mut e = enc;
    let color = ColorAttachment { view: off.color.view(), load: Load::Clear, clear: [0.0; 4], store: true, before: Layout::Undefined, after: Layout::CopySrc };
    t.ok(bad(r.record(&mut e, &s, color)), "record before prepare is refused");
    drop(e);
    let Some(()) = t.res(s.prepare(&mut r, (32, 32)), "prepare") else { return };
    let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
    t.ok(bad(r.record(&mut e, &s, color)), "a target of another size than prepare's is refused");
    drop(e);
    let Some(()) = t.res(s.prepare(&mut r, (64, 48)), "prepare") else { return };
    let Some(l2) = t.res(s.add_layer(&Layer::new(0)), "add_layer") else { return };
    let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
    t.ok(bad(r.record(&mut e, &s, color)), "a change after prepare: record is refused until the next prepare");
    drop(e);
    let _ = l2;
    // an empty scene: the background only (also after prepares that never reached a frame)
    let Some(img) = shot(t, &dev, &mut off, &mut r, &mut s, "empty") else { return };
    let bgc = [(BG & 255) as u8, (BG >> 8) as u8, (BG >> 16) as u8];
    t.ok(img.chunks_exact(4).all(|p| p[..3] == bgc), "an empty scene: the background");
    let Some(s2) = t.res(Scene::new(&r), "Scene::new") else { return };
    let Some(r2) = t.res(Renderer::new(&dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new") else { return };
    let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
    let color = ColorAttachment { view: off.color.view(), load: Load::Clear, clear: [0.0; 4], store: true, before: Layout::Undefined, after: Layout::CopySrc };
    t.ok(bad(r2.record(&mut e, &s2, color)), "a scene of another renderer is refused");
    drop(e);
    t.res(dev.wait_idle(), "wait_idle");
}

// ---------------------------------------------------------------- the shaders

/// (the passes that subtract the camera's hi / lo, the visibility and the vertex shader, are compiled exact: NoContraction
/// on every float operation, so that the driver neither fuses nor reassociates the two-sums of common.wgsl, which it
/// did: the lo part was lost (docs/RENDER2D.md "Koordinaten"))
fn compile_shaders() -> Option<Vec<(&'static str, Vec<u32>)>> {
    let opt = wgsl::CompileOptions { robust: false, ..wgsl::CompileOptions::new(shader::BASIS) };
    let exact = wgsl::CompileOptions { exact: true, ..opt };
    let mut outv = Vec::new();
    for (src, list, o) in [(render2d::CULL_WGSL, &[("COUNT", "count"), ("SCAN", "scan"), ("WRITE", "write")][..], &exact), (render2d::SCATTER_WGSL, &[("SCATTER", "main")][..], &opt),
                           (render2d::STROKE_WGSL, &[("VS", "vs"), ("VS_FULL", "vs_full")][..], &exact), (render2d::STROKE_WGSL, &[("FS", "fs"), ("FS_FULL", "fs_full")][..], &opt)] {
        match wgsl::compile(src, o) {
            Ok(o) => {
                for (name, entry) in list {
                    outv.push((*name, o.entry(entry)?.words.clone()));
                }
            }
            Err(d) => {
                for x in d.iter().take(8) {
                    out(&format!("FAIL {}: {x}", list[0].0));
                }
                return None;
            }
        }
    }
    Some(outv)
}

fn gen_shaders(path: &str) -> i32 {
    let Some(v) = compile_shaders() else { return 1 };
    let mut s = String::from("// written by render2dtest --gen-shaders from common.wgsl + cull.wgsl, scatter.wgsl, stroke.wgsl (shader::BASIS, no robustness; COUNT, SCAN, WRITE, VS and VS_FULL exact); do not edit\n");
    for (name, w) in &v {
        s.push_str(&format!("pub const {name}: &[u32] = &[\n"));
        for row in w.chunks(10) {
            s.push_str("    ");
            for x in row {
                s.push_str(&format!("{x:#x}, "));
            }
            s.truncate(s.trim_end().len());
            s.push('\n');
        }
        s.push_str("];\n");
    }
    if plat::write_file(path, s.as_bytes()) {
        out(&format!("{path}: {}", v.iter().map(|x| format!("{} {} words", x.0, x.1.len())).collect::<Vec<_>>().join(", ")));
        0
    } else {
        out(&format!("FAIL cannot write {path}"));
        1
    }
}

fn shaders(t: &mut T) {
    let Some(v) = compile_shaders() else {
        t.ok(false, "the WGSL compiles");
        return;
    };
    let table: [(&str, &[u32]); 8] = [("COUNT", render2d::COUNT_SPV), ("SCAN", render2d::SCAN_SPV), ("WRITE", render2d::WRITE_SPV), ("SCATTER", render2d::SCATTER_SPV),
                                      ("VS", render2d::VS_SPV), ("VS_FULL", render2d::VS_FULL_SPV), ("FS", render2d::FS_SPV), ("FS_FULL", render2d::FS_FULL_SPV)];
    for (name, embedded) in table {
        let made = v.iter().find(|x| x.0 == name).map_or(&[][..], |x| &x.1[..]);
        t.ok(made == embedded, &format!("{name}: {} words embedded, as wgsl::compile makes them (if not: render2dtest --gen-shaders lib/render2d/shaders_spv.rs)", embedded.len()));
    }
}

/// gallery::shots() on both levels into DIR (tests/engine_web.mjs compares the browser's with them)
fn dump_all(dir: &str) -> i32 {
    let mut t = T { fails: 0, validate: false };
    let mut files = 0;
    on_levels(&mut t, &mut |t, dev, level| {
        let Some(mut r) = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new") else { return };
        for shot in shots() {
            let Some(mut off) = t.res(Off::new(dev, shot.w, shot.h), "target") else { return };
            let Some(img) = gpu_image(t, dev, &mut r, &mut off, &shot.d, &shot.cam, &shot.name) else { return };
            let path = format!("{dir}/{}_{}.bgra", level.to_lowercase(), shot.name);
            if !plat::write_file(&path, &img) {
                t.ok(false, &format!("cannot write {path}"));
                return;
            }
            files += 1;
        }
    });
    out(&format!("{files} images in {dir}"));
    if t.fails > 0 { 1 } else { 0 }
}

pub fn run() -> i32 {
    let args = plat::args();
    let validate = args.iter().any(|a| a == "--validate");
    if validate {
        plat::setenv("jfgui_GPU_VALIDATE", Some("1"));
    }
    if let Some(p) = plat::arg_value("--gen-shaders") {
        return gen_shaders(&p);
    }
    if let Some(dir) = plat::arg_value("--dump") {
        return dump_all(&dir);
    }
    if args.iter().any(|a| a == "--demo") {
        return crate::demo::launch();
    }
    if let Some(n) = plat::arg_value("--bench").and_then(|v| v.parse::<u32>().ok()).filter(|&n| n > 0) {
        return crate::bench::bench(n);
    }
    let Some(test) = plat::arg_value("--test") else {
        plat::write_stderr(b"render2dtest: --test NAME [--validate], --bench N, --demo, --gen-shaders FILE (see apps/render2dtest/main.rs)\n");
        return 2;
    };
    let mut t = T { fails: 0, validate };
    let list: [(&str, fn(&mut T)); 17] = [("shaders", shaders), ("caps", caps), ("joins", joins), ("short", short), ("hairlines", hairlines), ("units", units), ("overlap", overlap),
                                          ("arcs", arcs), ("dashes", dashes), ("layers", layers), ("changes", crate::scenes::changes), ("churn", crate::scenes::churn),
                                          ("visibility", crate::scenes::visibility), ("coords", crate::scenes::coords), ("phase", crate::scenes::phase), ("levels", levels),
                                          ("errors", errors)];
    let mut ran = false;
    for (name, f) in list {
        if test == "all" || test == name {
            out(&format!("== {name}{}", if validate { " (validation)" } else { "" }));
            f(&mut t);
            ran = true;
        }
    }
    if test == "window" {
        out(&format!("== window{}", if validate { " (validation)" } else { "" }));
        return crate::demo::window_test(validate);
    }
    if !ran {
        plat::write_stderr(format!("render2dtest: no test {test}\n").as_bytes());
        return 2;
    }
    if validate {
        let (e, w) = gpu::validation_messages();
        t.ok(e == 0, &format!("validation layer: {e} errors, {w} warnings"));
    }
    if t.fails > 0 {
        out(&format!("{} check(s) failed", t.fails));
        1
    } else {
        out("all checks passed");
        0
    }
}
