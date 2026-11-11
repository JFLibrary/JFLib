// tests -- render2dtest's tests on Vulkan (main.rs lists them)

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render2d::{Camera, Cap, Join, Layer, Renderer, RendererDesc, Scene, Shape, Style};

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
                           (render2d::STROKE_WGSL, &[("VS", "vs")][..], &exact), (render2d::STROKE_WGSL, &[("FS", "fs")][..], &opt)] {
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
    let mut s = String::from("// written by render2dtest --gen-shaders from common.wgsl + cull.wgsl, scatter.wgsl, stroke.wgsl (shader::BASIS, no robustness; COUNT, SCAN, WRITE and VS exact); do not edit\n");
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
    let table: [(&str, &[u32]); 6] = [("COUNT", render2d::COUNT_SPV), ("SCAN", render2d::SCAN_SPV), ("WRITE", render2d::WRITE_SPV), ("SCATTER", render2d::SCATTER_SPV),
                                      ("VS", render2d::VS_SPV), ("FS", render2d::FS_SPV)];
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
    let list: [(&str, fn(&mut T)); 14] = [("shaders", shaders), ("caps", caps), ("joins", joins), ("short", short), ("hairlines", hairlines), ("units", units), ("overlap", overlap),
                                          ("layers", layers), ("changes", crate::scenes::changes), ("churn", crate::scenes::churn), ("visibility", crate::scenes::visibility),
                                          ("coords", crate::scenes::coords), ("levels", levels), ("errors", errors)];
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

