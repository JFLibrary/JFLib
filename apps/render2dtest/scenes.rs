// scenes -- render2dtest's tests of the retained scene: changes, visibility, large coordinates (main.rs lists them)

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render2d::{Camera, Cap, EntityId, Join, Layer, Renderer, RendererDesc, Scene, Style, Unit};

use crate::refimg::{self, Drawing, Geo, Item, Tol};
use crate::tests::{build, gpu_image, on_levels, shape_of, shot, style, Off, Rng, BG, T};

// ---------------------------------------------------------------- changes

fn random_geo(rng: &mut Rng, w: f64, h: f64) -> Geo {
    let a = [rng.range(5.0, w - 5.0), rng.range(5.0, h - 5.0)];
    match rng.below(3) {
        0 => Geo::Line(a, [rng.range(5.0, w - 5.0), rng.range(5.0, h - 5.0)]),
        k => {
            let n = 2 + rng.below(7) as usize;
            let mut p = vec![a];
            for _ in 1..n {
                let q = p[p.len() - 1];
                p.push([(q[0] + rng.range(-50.0, 50.0)).clamp(2.0, w - 2.0), (q[1] + rng.range(-50.0, 50.0)).clamp(2.0, h - 2.0)]);
            }
            Geo::Poly(p, k == 2 && n >= 3)
        }
    }
}

/// (opaque: a random polyline may cross itself, and away from its joins its pieces blend each where lib/gfx's fill
/// blends once; the overlap and layers tests take translucent colours)
fn random_style(rng: &mut Rng) -> Style {
    let c = 0xFF00_0000 | (rng.next() & 0x00FF_FFFF);
    let caps = [Cap::Butt, Cap::Round, Cap::Square];
    let joins = [Join::Miter, Join::Bevel, Join::Round];
    style(c, rng.range(0.0, 10.0) as f32, caps[rng.below(3) as usize], joins[rng.below(3) as usize])
}

/// a scene and the drawing it should show (the model the changes are applied to as well)
struct Pair {
    s: Scene,
    d: Drawing,
    /// per item of the drawing: its entity and its style
    ents: Vec<(EntityId, render2d::StyleId)>,
    layers: Vec<render2d::LayerId>,
}

impl Pair {
    fn insert(&mut self, layer: usize, st: Style, g: Geo) -> gpu::Result<()> {
        let sid = self.s.add_style(&st)?;
        let e = self.s.insert(self.layers[layer], sid, &shape_of(&g))?;
        self.d.items.push(Item { layer, style: st, geo: g });
        self.ents.push((e, sid));
        Ok(())
    }

    fn update(&mut self, k: usize, g: Geo) -> gpu::Result<()> {
        self.s.update(self.ents[k].0, &shape_of(&g))?;
        self.d.items[k].geo = g;
        Ok(())
    }

    fn remove(&mut self, k: usize) -> gpu::Result<()> {
        self.s.remove(self.ents[k].0)?;
        self.d.items.remove(k);
        self.ents.remove(k);
        Ok(())
    }

    /// the item's own style changed (set_style)
    fn restyle(&mut self, k: usize, st: Style) -> gpu::Result<()> {
        self.s.set_style(self.ents[k].1, &st)?;
        self.d.items[k].style = st;
        Ok(())
    }

    /// the item takes another style (set_entity_style: a new one)
    fn swap_style(&mut self, k: usize, st: Style) -> gpu::Result<()> {
        let sid = self.s.add_style(&st)?;
        self.s.set_entity_style(self.ents[k].0, sid)?;
        self.ents[k].1 = sid;
        self.d.items[k].style = st;
        Ok(())
    }
}

/// the scene's image against that of the same drawing built anew (byte for byte) and against lib/gfx
#[allow(clippy::too_many_arguments)]
fn judge(t: &mut T, dev: &Device, r: &mut Renderer, off: &mut Off, p: &mut Pair, cam: &Camera, what: &str, gfx: bool) -> Option<render2d::UploadInfo> {
    let img = shot(t, dev, off, r, &mut p.s, what)?;
    let info = p.s.upload_info();
    let fresh = gpu_image(t, dev, r, off, &p.d, cam, what)?;
    let n = img.chunks_exact(4).zip(fresh.chunks_exact(4)).filter(|(a, b)| a[..3] != b[..3]).count();
    t.ok(n == 0, &format!("{what}: against the scene built anew: {n} pixels differ ({} entities, {} records sent in {} runs{}{})", p.d.items.len(), info.records, info.runs,
                          if info.scatter { ", scatter" } else { "" }, if info.rebuild { format!(", a new pool, {} records copied", info.moved) } else { String::new() }));
    if gfx {
        let reference = refimg::draw(&p.d, cam, (off.w, off.h), BG);
        let tol = Tol::folded(p.d.items.len() as u32);
        let c = refimg::compare(&img, &reference, off.w, BG, tol.small);
        let name: String = what.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        crate::tests::dump(&format!("{name}_gpu"), &img, off.w, off.h);
        crate::tests::dump(&format!("{name}_ref"), &reference, off.w, off.h);
        t.ok(c.fine(&tol), &format!("{what}: against lib/gfx: {}", c.text()));
    }
    Some(info)
}

use alloc::string::String;

pub fn changes(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = changes_on(t, dev, level); });
}

fn changes_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (320u32, 240u32);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    let cam = crate::tests::pixel_camera(w, h);
    let mut rng = Rng(11);
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    t.res(s.set_camera(&cam), "camera")?;
    let d = Drawing { layers: vec![(true, 1), (true, 0), (true, 2)], items: Vec::new() };
    let b = t.res(build(&mut s, &d), "build")?;
    let mut p = Pair { s, d, ents: Vec::new(), layers: b.layers };
    for k in 0..40 {
        let (st, g) = (random_style(&mut rng), random_geo(&mut rng, w as f64, h as f64));
        t.res(p.insert(k % 3, st, g), "insert")?;
    }
    judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: 40 entities on three layers"), true)?;
    // inserted at the ends of their layers
    for k in 0..6 {
        let (st, g) = (random_style(&mut rng), random_geo(&mut rng, w as f64, h as f64));
        t.res(p.insert(k % 3, st, g), "insert")?;
    }
    let info = judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: six inserted"), false)?;
    t.ok(!info.rebuild && info.runs <= 6, &format!("{level}: inserts go up as their records ({} runs, rebuild {})", info.runs, info.rebuild));
    // an update in place (the same records): only its records
    let k = 5;
    let n = match &p.d.items[k].geo {
        Geo::Line(..) => 1,
        Geo::Poly(v, c) => if *c { v.len() } else { v.len() - 1 },
    };
    let moved: Geo = match &p.d.items[k].geo {
        Geo::Line(a, b) => Geo::Line([a[0] + 7.0, a[1] - 3.0], [b[0] + 7.0, b[1] - 3.0]),
        Geo::Poly(v, c) => Geo::Poly(v.iter().map(|q| [q[0] + 7.0, q[1] - 3.0]).collect(), *c),
    };
    t.res(p.update(k, moved), "update")?;
    let info = judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: an entity moved"), false)?;
    t.ok(info.records as usize == n && info.runs == 1 && !info.rebuild, &format!("{level}: an update in place sends its {n} records ({} records, {} runs)", info.records, info.runs));
    // the last entity of a layer grows: in place
    let last = p.d.items.len() - 1;
    let big: Vec<[f64; 2]> = (0..30).map(|i| [20.0 + 9.0 * i as f64, 200.0 + 15.0 * ((i % 2) as f64)]).collect();
    t.res(p.update(last, Geo::Poly(big, false)), "update")?;
    let info = judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: the last of a layer grew to 29 segments"), false)?;
    t.ok(!info.rebuild && info.records == 29, &format!("{level}: it grows in place ({} records, rebuild {})", info.records, info.rebuild));
    // one in the middle grows: its layer is laid out anew, it keeps its place in the order
    let big: Vec<[f64; 2]> = (0..40).map(|i| [10.0 + 7.5 * i as f64, 30.0 + 40.0 * ((i % 3) as f64)]).collect();
    t.res(p.update(1, Geo::Poly(big, true)), "update")?;
    let info = judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: one in the middle grew to 40 segments"), true)?;
    t.ok(info.rebuild && info.records >= 40, &format!("{level}: its layer laid out anew ({} records sent, {} copied)", info.records, info.moved));
    // removes: free places; a style changed (no records), another style taken (its records)
    for k in [3usize, 10, 20] {
        t.res(p.remove(k), "remove")?;
    }
    let mut st = p.d.items[0].style;
    st.color ^= 0x0080_8080;
    st.width = 7.0;
    st.join = Join::Round;
    t.res(p.restyle(0, st), "set_style")?;
    let info = judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: three removed, a style changed"), true)?;
    t.ok(!info.rebuild, &format!("{level}: removes leave free places ({} records of zeros, rebuild {})", info.records, info.rebuild));
    t.res(p.swap_style(2, style(0xC0FF_2020, 5.0, Cap::Round, Join::Round)), "set_entity_style")?;
    let info = judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: an entity took another style"), false)?;
    t.ok(info.runs == 1 && !info.rebuild, &format!("{level}: its records go up ({} records, {} runs)", info.records, info.runs));
    // many changes at once: thousands of lines, every second one moved (the scatter shader), then most removed (the
    // layer laid out anew without its free places)
    let base = p.d.items.len();
    for _ in 0..3000 {
        let a = [rng.range(5.0, 315.0), rng.range(5.0, 235.0)];
        let st = style(0x30_000000 | (rng.next() & 0x00FF_FFFF), 0.0, Cap::Butt, Join::Miter);
        t.res(p.insert(1, st, Geo::Line(a, [a[0] + rng.range(-9.0, 9.0), a[1] + rng.range(-9.0, 9.0)])), "insert")?;
    }
    judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: 3000 short lines inserted"), false)?;
    for k in (base..base + 3000).step_by(2) {
        let g = match &p.d.items[k].geo {
            Geo::Line(a, b) => Geo::Line([a[0] + 1.5, a[1]], [b[0] + 1.5, b[1]]),
            g => g.clone(),
        };
        t.res(p.update(k, g), "update")?;
    }
    let info = judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: 1500 of them moved"), false)?;
    t.ok(info.scatter && info.records == 1500, &format!("{level}: many scattered changes go through the scatter shader ({} records, {} runs)", info.records, info.runs));
    for k in (base..base + 2600).rev() {
        t.res(p.remove(k), "remove")?;
    }
    let info = judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: 2600 removed"), true)?;
    t.ok(info.rebuild, &format!("{level}: a layer mostly free is laid out anew ({} records copied, {} sent)", info.moved, info.records));
    // a layer removed with its entities; a prepare that never reached a frame (everything goes up again)
    let gone = p.layers[2];
    t.res(p.s.remove_layer(gone), "remove_layer")?;
    let mut k = 0;
    while k < p.d.items.len() {
        if p.d.items[k].layer == 2 {
            p.d.items.remove(k);
            p.ents.remove(k);
        } else {
            k += 1;
        }
    }
    p.d.layers[2].0 = false;
    t.res(p.s.prepare(&mut r, (w, h)), "prepare without a frame")?;
    let info = judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: a layer removed, a prepare without a frame before"), true)?;
    t.ok(info.rebuild && info.moved == 0, &format!("{level}: after a prepare that never reached a frame everything goes up from the CPU ({} records)", info.records));
    // nothing changed: the view only
    let info = judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: nothing changed"), false)?;
    t.ok(info.records == 0 && !info.rebuild && !info.tables && info.bytes == 64, &format!("{level}: a frame without changes sends the view only ({} bytes)", info.bytes));
    Some(())
}

// ---------------------------------------------------------------- visibility

/// what the CPU's exact test says of a record
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Seen {
    Yes,
    Doubt,
    No,
}

/// cull.wgsl's test in f64: the box around the segment, grown by the stroke's reach (a miter's bound only where the
/// record has a neighbour: joined); the GPU adds a margin for f32's rounding, within it a record may go either way
fn exact(a: [f64; 2], b: [f64; 2], st: &Style, joined: bool, cam: &Camera, size: (u32, u32)) -> Seen {
    let (s, c) = render2d::math::sincos(cam.rotation);
    let px = |p: [f64; 2]| {
        let (x, y) = (p[0] - cam.center[0], p[1] - cam.center[1]);
        [(c * x - s * y) / cam.scale, -(s * x + c * y) / cam.scale]
    };
    let (p0, p1) = (px(a), px(b));
    let wpx = match st.unit {
        Unit::Drawing => st.width as f64 / cam.scale,
        _ => st.width as f64,
    };
    let hw = (0.5 * wpx).max(0.5);
    let bound = if st.join == Join::Miter && joined { render2d::math::sqrt((st.miter_limit as f64 * st.miter_limit as f64 - 1.0).max(0.0)).max(1.0) } else { 1.0 };
    let reach = hw * bound + 1.0;
    let len0 = |v: [f64; 2]| render2d::math::sqrt(v[0] * v[0] + v[1] * v[1]);
    let margin = 1e-3 + 4e-6 * (len0(p0) + len0(p1));
    let half = [0.5 * size.0 as f64, 0.5 * size.1 as f64];
    let dv = [p1[0] - p0[0], p1[1] - p0[1]];
    let len = len0(dv);
    let d = if len > 0.0 { [dv[0] / len, dv[1] / len] } else { [1.0, 0.0] };
    let n = [-d[1], d[0]];
    let cc = [0.5 * (p0[0] + p1[0]), 0.5 * (p0[1] + p1[1])];
    let e = 0.5 * len + reach;
    let dot = |u: [f64; 2], v: [f64; 2]| u[0] * v[0] + u[1] * v[1];
    let sep = [
        cc[0].abs() - (half[0] + e * d[0].abs() + reach * n[0].abs()),
        cc[1].abs() - (half[1] + e * d[1].abs() + reach * n[1].abs()),
        dot(cc, d).abs() - (e + half[0] * d[0].abs() + half[1] * d[1].abs()),
        dot(cc, n).abs() - (reach + half[0] * n[0].abs() + half[1] * n[1].abs()),
    ];
    let m = sep.iter().fold(f64::MIN, |a, &b| a.max(b));
    if m <= 0.0 {
        Seen::Yes
    } else if m > 2.0 * margin + 0.01 {
        Seen::No
    } else {
        Seen::Doubt
    }
}

/// the visible lists after a frame: per visible layer (first, count), and the records in them
fn read_lists(t: &mut T, dev: &Device, s: &Scene) -> Option<(Vec<[u32; 4]>, Vec<u32>)> {
    let layers = s.drawn_layers().len();
    let rb = |size: u64| dev.create_buffer(&BufferDesc { size: size.max(16), usage: BufferUsage::NONE, memory: MemoryKind::Readback });
    let lb = t.res(rb(layers as u64 * 16), "read-back")?;
    let vb = t.res(rb(s.visible_buffer().size()), "read-back")?;
    let mut e = t.res(dev.encoder(), "encoder")?;
    if layers > 0 {
        e.copy_buffer(s.lists_buffer(), 0, &lb, 0, layers as u64 * 16);
    }
    e.copy_buffer(s.visible_buffer(), 0, &vb, 0, s.visible_buffer().size());
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    let cmd = t.res(e.finish(), "finish")?;
    let id = t.res(dev.queue().submit([cmd]), "submit")?;
    t.res(dev.wait(id, 10_000_000_000), "wait")?;
    let mut lists = vec![[0u32; 4]; layers];
    if layers > 0 {
        t.res(lb.read(0, &mut lists), "read")?;
    }
    let mut vis = vec![0u32; (s.visible_buffer().size() / 4) as usize];
    t.res(vb.read(0, &mut vis), "read")?;
    Some((lists, vis))
}

pub fn visibility(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = visibility_on(t, dev, level); });
}

fn visibility_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let size = (200u32, 150u32);
    let mut off = t.res(Off::new(dev, size.0, size.1), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    for (seed, n, spread) in [(1u32, 5000u32, 600.0f64), (2, 20000, 3000.0), (3, 3000, 250.0), (4, 8000, 1.0e6)] {
        let mut rng = Rng(seed);
        let mut s = t.res(Scene::new(&r), "Scene::new")?;
        let layers: Vec<_> = (0..3).map(|k| s.add_layer(&Layer::new(k as i32))).collect::<gpu::Result<Vec<_>>>().ok()?;
        let mut cam = Camera::new([rng.range(-100.0, 100.0), rng.range(-100.0, 100.0)], rng.range(0.4, 2.5) * spread / 600.0);
        cam.rotation = rng.range(-3.0, 3.0);
        t.res(s.set_camera(&cam), "camera")?;
        let mut styles = Vec::new();
        for k in 0..12 {
            let mut st = style(0xFF00_0000 | rng.next(), rng.range(0.0, 12.0) as f32, [Cap::Butt, Cap::Round, Cap::Square][k % 3], [Join::Miter, Join::Bevel, Join::Round][(k / 3) % 3]);
            if k % 4 == 3 {
                st.unit = Unit::Drawing;
                st.width = (rng.range(0.0, 0.02) * spread) as f32;
            }
            st.miter_limit = rng.range(1.0, 12.0) as f32;
            styles.push((t.res(s.add_style(&st), "add_style")?, st));
        }
        let mut ents = Vec::new();
        for _ in 0..n {
            let a = [cam.center[0] + rng.range(-spread, spread), cam.center[1] + rng.range(-spread, spread)];
            let m = 2 + rng.below(4) as usize;
            let mut pts = vec![a];
            for _ in 1..m {
                let q = pts[pts.len() - 1];
                let step = spread * 0.05;
                pts.push([q[0] + rng.range(-step, step), q[1] + rng.range(-step, step)]);
            }
            let (sid, st) = styles[rng.below(12) as usize];
            let closed = rng.below(5) == 0;
            let id = t.res(s.insert(layers[rng.below(3) as usize], sid, &render2d::Shape::Polyline { points: &pts, closed }), "insert")?;
            ents.push((id, pts, closed, st));
        }
        // a few removed: their places must not show up
        for k in (0..ents.len()).step_by(17) {
            t.res(s.remove(ents[k].0), "remove")?;
        }
        let _ = shot(t, dev, &mut off, &mut r, &mut s, "frame")?;
        let (lists, vis) = read_lists(t, dev, &s)?;
        let mut seen = Vec::new();
        let mut unordered = 0;
        for l in &lists {
            let part = &vis[l[0] as usize..(l[0] + l[1]) as usize];
            unordered += part.windows(2).filter(|w| w[0] >= w[1]).count();
            seen.extend_from_slice(part);
        }
        seen.sort_unstable();
        let twice = seen.windows(2).filter(|w| w[0] == w[1]).count();
        let (mut yes, mut doubt, mut missing, mut extra) = (0u32, 0u32, 0u32, 0u32);
        let mut known = Vec::new();
        for (k, (id, pts, closed, st)) in ents.iter().enumerate() {
            let removed = k % 17 == 0;
            let Some((first, count)) = s.locate(*id).or(if removed { Some((u32::MAX, 0)) } else { None }) else {
                t.ok(false, "locate");
                return None;
            };
            if removed {
                continue;
            }
            let m = pts.len();
            let segs = if *closed { m } else { m - 1 };
            if segs as u32 != count {
                t.ok(false, &format!("an entity of {m} points holds {count} records"));
                return None;
            }
            for j in 0..segs {
                let v = exact(pts[j], pts[(j + 1) % m], st, segs > 1, &cam, size);
                let place = first + j as u32;
                known.push(place);
                let got = seen.binary_search(&place).is_ok();
                match v {
                    Seen::Yes => {
                        yes += 1;
                        missing += !got as u32;
                    }
                    Seen::Doubt => doubt += 1,
                    Seen::No => extra += got as u32,
                }
            }
        }
        known.sort_unstable();
        let foreign = seen.iter().filter(|p| known.binary_search(p).is_err()).count();
        t.ok(missing == 0 && extra == 0 && twice == 0 && foreign == 0 && unordered == 0,
             &format!("{level}: seed {seed}: {n} polylines over {spread}, rotation {:.2}: GPU {} visible, CPU {yes} (+{doubt} within a hair of an edge): {missing} missing, {extra} too many, \
                       {twice} doubled, {foreign} free or unknown, {unordered} out of order", cam.rotation, seen.len()));
        if seed == 1 {
            // culled = not culled
            let a = shot(t, dev, &mut off, &mut r, &mut s, "culled")?;
            s.set_culling(false);
            let b = shot(t, dev, &mut off, &mut r, &mut s, "not culled")?;
            let nd = a.chunks_exact(4).zip(b.chunks_exact(4)).filter(|(x, y)| x[..3] != y[..3]).count();
            t.ok(nd == 0, &format!("{level}: culled against not culled: {nd} pixels differ"));
        }
    }
    // records exactly at each edge of the target: touching stays, a hair beyond goes (axis-aligned camera, hairlines:
    // the reach is 0.5 + 1 px)
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    let l = t.res(s.add_layer(&Layer::new(0)), "add_layer")?;
    let cam = Camera::new([0.0, 0.0], 1.0);
    t.res(s.set_camera(&cam), "camera")?;
    let st = Style::new(0xFF00_0000);
    let sid = t.res(s.add_style(&st), "add_style")?;
    let (hx, hy, reach) = (100.0, 75.0, 1.5);
    let mut cases = Vec::new();
    for (name, gap, want) in [("touching", 0.0, true), ("0.05 px beyond", 0.05, false)] {
        let g = reach + gap;
        for (edge, a, b) in [("left", [-hx - g, -20.0], [-hx - g, 20.0]), ("right", [hx + g, -20.0], [hx + g, 20.0]), ("bottom", [-20.0, -hy - g], [20.0, -hy - g]),
                             ("top", [-20.0, hy + g], [20.0, hy + g])] {
            let id = t.res(s.insert(l, sid, &render2d::Shape::Line { a, b }), "insert")?;
            cases.push((id, edge, name, want));
        }
    }
    let _ = shot(t, dev, &mut off, &mut r, &mut s, "edges")?;
    let (lists, vis) = read_lists(t, dev, &s)?;
    let part = &vis[lists[0][0] as usize..(lists[0][0] + lists[0][1]) as usize];
    for (id, edge, name, want) in cases {
        let place = s.locate(id)?.0;
        let got = part.contains(&place);
        t.ok(got == want, &format!("{level}: a hairline {name} the {edge} edge: {}", if got { "in the list" } else { "culled" }));
    }
    Some(())
}

// ---------------------------------------------------------------- large coordinates

/// the share of a pixel inside a half plane whose edge lies s from its centre (outward unit normal m), in f64: what
/// stroke.wgsl's cov() computes
fn box_cov(s: f64, m: [f64; 2]) -> f64 {
    let (a, b) = (m[0].abs().min(m[1].abs()), m[0].abs().max(m[1].abs()));
    let x = -s;
    let outer = 0.5 * (a + b);
    if x >= outer {
        return 1.0;
    }
    if x <= -outer {
        return 0.0;
    }
    if a < 1e-9 {
        return (0.5 + x / b).clamp(0.0, 1.0);
    }
    let inner = 0.5 * (b - a);
    if x < -inner {
        let t = x + outer;
        return t * t / (2.0 * a * b);
    }
    if x > inner {
        let t = outer - x;
        return 1.0 - t * t / (2.0 * a * b);
    }
    0.5 + x / b
}

pub fn coords(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = coords_on(t, dev, level); });
}

/// a drawing of every kind of stroke around a point
fn sample(at: [f64; 2]) -> Drawing {
    let mut d = Drawing::one_layer();
    let p = |x: f64, y: f64| [at[0] + x, at[1] + y];
    d.add(style(0xFF20_3040, 6.0, Cap::Round, Join::Round), Geo::Poly(vec![p(-80.0, -50.0), p(-20.0, 40.0), p(30.0, -30.0), p(90.0, 45.0)], false));
    d.add(style(0x9030_8040, 11.0, Cap::Square, Join::Miter), Geo::Poly(vec![p(-90.0, 10.0), p(80.0, 20.0), p(-60.0, 35.0)], false));
    d.add(style(0xFF80_2020, 0.0, Cap::Butt, Join::Miter), Geo::Line(p(-100.0, -60.0), p(100.0, 55.5)));
    let mut st = style(0xC020_40C0, 0.8, Cap::Butt, Join::Bevel);
    st.unit = Unit::Drawing;
    d.add(st, Geo::Poly(vec![p(-40.0, -40.0), p(40.0, -40.0), p(40.0, 30.0), p(-40.0, 30.0)], true));
    d
}

fn coords_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (240u32, 160u32);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    // the same drawing at the origin and at +-1e7, the camera with it (rotated too)
    let base = {
        let mut c = Camera::new([0.25, -0.5], 0.9);
        c.rotation = 0.3;
        gpu_image(t, dev, &mut r, &mut off, &sample([0.0, 0.0]), &c, "at the origin")?
    };
    for at in [[1.0e7, 1.0e7], [-9_999_999.5, 3_141_592.653_589_793], [9_876_543.210_987, -9_999_000.0]] {
        let mut c = Camera::new([at[0] + 0.25, at[1] - 0.5], 0.9);
        c.rotation = 0.3;
        let img = gpu_image(t, dev, &mut r, &mut off, &sample(at), &c, "far away")?;
        crate::tests::dump(&format!("coords_{}_{level}", at[0] as i64), &img, w, h);
        crate::tests::dump(&format!("coords_origin_{level}"), &base, w, h);
        let (mut n, mut m) = (0u32, 0u8);
        for (a, b) in img.chunks_exact(4).zip(base.chunks_exact(4)) {
            if a[..3] != b[..3] {
                n += 1;
                for k in 0..3 {
                    m = m.max(a[k].abs_diff(b[k]));
                }
            }
        }
        t.ok(m <= 1 && n <= (w * h) / 1000, &format!("{level}: the drawing at ({}, {}) against at the origin: {n} pixels differ, largest {m}", at[0], at[1]));
    }
    // a deep zoom: 1e-6 units per pixel near (-9 876 543.21, 6 543 210.987), tiny pans; white lines on black, each
    // line's place from the coverage of its rows against where it belongs (f64): within 1/16 px
    let (dw, dh) = (200u32, 160u32);
    let mut off = t.res(Off::new(dev, dw, dh), "target")?;
    off.bg = 0xFF00_0000;
    let at = [-9_876_543.21, 6_543_210.987];
    let k = 1e-6;
    // (a, b, along y): lines through the view, short ones (ends inside), long ones (ends 0.3 and 1e7 units away)
    let lines: [([f64; 2], [f64; 2]); 6] = [
        ([at[0] - 40.0 * k, at[1] - 50.0 * k], [at[0] - 35.0 * k, at[1] + 55.0 * k]),
        ([at[0] + 20.3 * k, at[1] - 0.3], [at[0] + 32.9 * k, at[1] + 0.3]),
        ([at[0] + 61.0 * k - 1.0e7 * 0.02, at[1] - 1.0e7], [at[0] + 61.0 * k + 1.0e7 * 0.02, at[1] + 1.0e7]),
        ([at[0] - 0.7, at[1] + 30.25 * k - 0.7 * 0.03], [at[0] + 0.7, at[1] + 30.25 * k + 0.7 * 0.03]),
        ([at[0] - 1.0e7, at[1] - 41.6 * k - 0.005], [at[0] + 1.0e7, at[1] - 41.6 * k + 0.005]),
        ([at[0] - 70.0 * k, at[1] + 5.0 * k], [at[0] - 10.0 * k, at[1] + 62.0 * k]),
    ];
    let mut d = Drawing::one_layer();
    for (a, b) in lines {
        d.add(style(0xFFFF_FFFF, 2.5, Cap::Butt, Join::Miter), Geo::Line(a, b));
    }
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    t.res(build(&mut s, &d), "build")?;
    let (mut worst, mut checks, mut lost) = (0.0f64, 0u32, 0u32);
    let mut per_line = [0.0f64; 6];
    for step in 0..24 {
        // pans of 0.37 px and of a few pixels, with a rotation now and then
        let mut c = Camera::new([at[0] + step as f64 * 0.37 * k, at[1] - step as f64 * 0.23 * k], k);
        c.rotation = if step >= 16 { 0.25 } else { 0.0 };
        t.res(s.set_camera(&c), "camera")?;
        let img = shot(t, dev, &mut off, &mut r, &mut s, "deep zoom")?;
        let cov = |x: usize, y: usize| img[(y * dw as usize + x) * 4 + 1] as f64 / 255.0;
        for (li, (a, b)) in lines.into_iter().enumerate() {
            let (sa, sb) = (c.to_screen(a, (dw, dh)), c.to_screen(b, (dw, dh)));
            let vertical = (sb[1] - sa[1]).abs() > (sb[0] - sa[0]).abs();
            // rows (or columns) inside the view and away from the ends
            for j in (12..(if vertical { dh } else { dw }) as usize - 12).step_by(9) {
                let y = j as f64 + 0.5;
                let (u0, v0, u1, v1) = if vertical { (sa[0], sa[1], sb[0], sb[1]) } else { (sa[1], sa[0], sb[1], sb[0]) };
                // (between the ends, 3 px away from them: a butt end crosses its last rows)
                if (y - v0) * (y - v1) > 0.0 || (y - v0).abs() < 3.0 || (y - v1).abs() < 3.0 {
                    continue;
                }
                let want = u0 + (y - v0) * (u1 - u0) / (v1 - v0);
                let lim = if vertical { dw } else { dh } as f64;
                if want < 8.0 || want > lim - 8.0 {
                    continue;
                }
                // (no other line within 3 px of the window: it would pull the centroid)
                let near = lines.iter().enumerate().any(|(lj, &(oa, ob))| {
                    if lj == li {
                        return false;
                    }
                    let (pa, pb) = (c.to_screen(oa, (dw, dh)), c.to_screen(ob, (dw, dh)));
                    let (dx, dy) = (pb[0] - pa[0], pb[1] - pa[1]);
                    let l = render2d::math::sqrt(dx * dx + dy * dy);
                    (-6..=6).any(|o| {
                        let (x, y) = if vertical { (want + o as f64, y) } else { (y, want + o as f64) };
                        ((x - pa[0]) * dy - (y - pa[1]) * dx).abs() / l < 1.25 + 3.0
                    })
                });
                if near {
                    continue;
                }
                // the centroid of the window's coverage, against that of the exact coverage of the line there (the strip's
                // share of each pixel in f64, as cov() in stroke.wgsl: binning the coverage biases a centroid by up to
                // 0.05 px, the same for both)
                let (dx, dy) = (sb[0] - sa[0], sb[1] - sa[1]);
                let l = render2d::math::sqrt(dx * dx + dy * dy);
                let n = [-dy / l, dx / l];
                let (mut sum, mut mom, mut esum, mut emom) = (0.0, 0.0, 0.0, 0.0);
                for i in (want as i64 - 6).max(0) as usize..((want as i64 + 7) as usize).min(lim as usize) {
                    let (px, py) = if vertical { (i, j) } else { (j, i) };
                    let c = cov(px, py);
                    sum += c;
                    mom += c * (i as f64 + 0.5);
                    let v = (px as f64 + 0.5 - sa[0]) * n[0] + (py as f64 + 0.5 - sa[1]) * n[1];
                    let e = (box_cov(v - 1.25, n) + box_cov(-v - 1.25, n) - 1.0).max(0.0);
                    esum += e;
                    emom += e * (i as f64 + 0.5);
                }
                checks += 1;
                if sum < 1.0 {
                    lost += 1;
                    continue;
                }
                let want = emom / esum;
                worst = worst.max((mom / sum - want).abs());
                per_line[li] = per_line[li].max((mom / sum - want).abs());
            }
        }
    }
    t.ok(checks > 300 && lost == 0 && worst <= 1.0 / 16.0,
         &format!("{level}: 1e-6 units per pixel near ({}, {}), 24 pans of 0.37 px and a rotation, six lines up to 2e7 long: {checks} rows, the largest distance from where the line belongs \
                   {worst:.4} px (1/16 = 0.0625; per line {}), {lost} rows without the line", at[0], at[1],
                   per_line.iter().map(|v| format!("{v:.4}")).collect::<Vec<_>>().join(", ")));
    Some(())
}

