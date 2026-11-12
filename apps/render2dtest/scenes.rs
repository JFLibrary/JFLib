// scenes -- render2dtest's tests of the retained scene: changes, visibility, large coordinates (main.rs lists them)

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render2d::{Camera, Cap, EntityId, Join, Layer, Renderer, RendererDesc, Scene, Shape, Style, Unit};

use crate::gallery::*;
use crate::refimg::{self, Tol};
use crate::tests::{gpu_image, on_levels, shot, T};

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
        let e = with_shape(&g, |sh| self.s.insert(self.layers[layer], sid, sh))?;
        self.d.items.push(Item { layer, style: st, geo: g });
        self.ents.push((e, sid));
        Ok(())
    }

    fn update(&mut self, k: usize, g: Geo) -> gpu::Result<()> {
        with_shape(&g, |sh| self.s.update(self.ents[k].0, sh))?;
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
    let cam = pixel_camera(w, h);
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
    let n = records_of(&p.d.items[k].geo);
    let moved: Geo = match &p.d.items[k].geo {
        Geo::Line(a, b) => Geo::Line([a[0] + 7.0, a[1] - 3.0], [b[0] + 7.0, b[1] - 3.0]),
        Geo::Poly(v, c) => Geo::Poly(v.iter().map(|q| [q[0] + 7.0, q[1] - 3.0]).collect(), *c),
        Geo::Arc(c, r, a0, sw) => Geo::Arc([c[0] + 7.0, c[1] - 3.0], *r, *a0, *sw),
        Geo::Circle(c, r) => Geo::Circle([c[0] + 7.0, c[1] - 3.0], *r),
        g => g.clone(),
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
    // a growth in the middle of a layer is only noted until the next frame: what comes before that frame must go by what
    // the layer really holds.  Grown and removed before a frame: the neighbours stay
    let first = p.d.items.len();
    let wide = |c: u32| style(c, 9.0, Cap::Butt, Join::Miter);
    t.res(p.insert(1, wide(0xFFFF_0000), Geo::Line([20.0, 20.0], [300.0, 220.0])), "insert")?;
    t.res(p.insert(1, wide(0xFF00_FF00), Geo::Line([20.0, 220.0], [300.0, 20.0])), "insert")?;
    t.res(p.insert(1, wide(0xFF00_00FF), Geo::Line([160.0, 10.0], [160.0, 230.0])), "insert")?;
    judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: three crossing lines"), false)?;
    let zig = |n: usize| -> Vec<[f64; 2]> { (0..n).map(|i| [20.0 + 280.0 * i as f64 / (n - 1) as f64, 120.0 + if i % 2 == 0 { 25.0 } else { -25.0 }]).collect() };
    t.res(p.update(first, Geo::Poly(zig(12), false)), "update")?;
    t.res(p.remove(first), "remove")?;
    judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: grown and removed before the frame"), false)?;
    // grown twice before a frame (the first time just up to the end of the layer): it stays under the line after it
    t.res(p.update(first, Geo::Poly(zig(3), false)), "update")?;
    t.res(p.update(first, Geo::Poly(zig(30), false)), "update")?;
    judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: grown twice before the frame"), false)?;
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
    // curves and fills (G6b1): inserted, a fill grown in the middle of its layer, a curve changed into another kind, a
    // fill's style hatched, removed
    let first = p.d.items.len();
    let sq = |x: f64, y: f64, s: f64| vec![Geo::Poly(vec![[x, y], [x + s, y], [x + s, y + s], [x, y + s]], true)];
    t.res(p.insert(0, style(0xFF20_6040, 0.0, Cap::Butt, Join::Miter), Geo::Fill(vec![sq(30.0, 30.0, 60.0), sq(45.0, 45.0, 20.0)])), "insert")?;
    let hatch = |c: u32| style(c, 0.0, Cap::Butt, Join::Miter).hatched(&[render2d::HatchLine::new(0.6, 5.0)], 1.0);
    t.res(p.insert(0, hatch(0xFF60_2080), Geo::Fill(vec![vec![Geo::Ellipse([200.0, 150.0], [50.0, 30.0], 0.3, 0.0, core::f64::consts::TAU)]])), "insert")?;
    t.res(p.insert(0, style(INK[1], 3.0, Cap::Round, Join::Round), Geo::Bez(vec![[20.0, 200.0], [100.0, 120.0], [180.0, 260.0], [300.0, 180.0]], 3, false)), "insert")?;
    judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: a fill with a hole, a hatched fill and a curve inserted"), true)?;
    t.res(p.update(first, Geo::Fill(vec![sq(30.0, 30.0, 60.0), sq(45.0, 45.0, 20.0), vec![Geo::Circle([60.0, 55.0], 6.0)]])), "update")?;
    t.res(p.update(first + 2, Geo::Spline(3, vec![[20.0, 200.0], [60.0, 150.0], [100.0, 230.0], [180.0, 140.0], [260.0, 220.0], [300.0, 180.0]], vec![], vec![], false)), "update")?;
    judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: a fill grown by an island, a curve changed into a spline"), true)?;
    t.res(p.restyle(first, hatch(0xFF20_6040)), "set_style")?;
    judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: a fill's style hatched"), false)?;
    t.res(p.remove(first + 1), "remove")?;
    judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: a hatched fill removed"), false)?;
    // nothing changed: the view only
    let info = judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: nothing changed"), false)?;
    t.ok(info.records == 0 && !info.rebuild && !info.tables && info.bytes == 80, &format!("{level}: a frame without changes sends the view only ({} bytes)", info.bytes));
    Some(())
}

// ---------------------------------------------------------------- many changes between two frames

pub fn churn(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = churn_on(t, dev, level); });
}

/// a polyline of m points somewhere in a target of w x h
fn zigzag(rng: &mut Rng, m: usize, w: f64, h: f64) -> Geo {
    let (mut x, mut y) = (rng.range(5.0, w - 5.0), rng.range(5.0, h - 5.0));
    let mut p = Vec::with_capacity(m);
    for _ in 0..m {
        p.push([x, y]);
        x = (x + rng.range(-40.0, 40.0)).clamp(2.0, w - 2.0);
        y = (y + rng.range(-40.0, 40.0)).clamp(2.0, h - 2.0);
    }
    Geo::Poly(p, false)
}

fn churn_style(rng: &mut Rng) -> Style {
    let mut st = random_style(rng);
    // (a third translucent: the order of the entities shows)
    if rng.below(3) == 0 {
        st.color = (st.color & 0x00FF_FFFF) | 0x8000_0000;
    }
    st
}

/// the records an item has (Geo::Poly open or closed; an arc: its head and pieces)
fn records_of(g: &Geo) -> usize {
    match g {
        Geo::Line(..) => 1,
        Geo::Poly(v, c) => if *c { v.len() } else { v.len() - 1 },
        Geo::Arc(_, _, _, sw) => ((sw.abs() / render2d::ARC_STEP - 1e-9).max(0.0) as usize + 1).min(16) + 1,
        Geo::Circle(..) => 17,
        _ => 1,
    }
}

fn churn_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = (320u32, 240u32);
    let (fw, fh) = (w as f64, h as f64);
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    let cam = pixel_camera(w, h);
    for seed in [101u32, 202, 303, 404, 505, 606] {
        let mut rng = Rng(seed);
        let mut s = t.res(Scene::new(&r), "Scene::new")?;
        t.res(s.set_camera(&cam), "camera")?;
        let d = Drawing { layers: vec![(true, 1), (true, 0), (true, 2)], items: Vec::new() };
        let b = t.res(build(&mut s, &d), "build")?;
        let mut p = Pair { s, d, ents: Vec::new(), layers: b.layers };
        for k in 0..24 {
            let (st, g) = (churn_style(&mut rng), random_geo(&mut rng, fw, fh));
            t.res(p.insert(k % 3, st, g), "insert")?;
        }
        judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: seed {seed}: the start"), false)?;
        let (mut grown_twice, mut grown_removed) = (0u32, 0u32);
        for round in 0..16 {
            let mut ops = String::new();
            for _ in 0..3 + rng.below(18) {
                let n = p.d.items.len() as u32;
                let k = if n > 0 { rng.below(n) as usize } else { 0 };
                match if n == 0 { 0 } else { rng.below(8) } {
                    0 => {
                        let (st, g) = (churn_style(&mut rng), random_geo(&mut rng, fw, fh));
                        let layer = rng.below(3) as usize;
                        t.res(p.insert(layer, st, g), "insert")?;
                        ops.push('i');
                    }
                    1 => {
                        // no more records than it had: in place
                        let m = records_of(&p.d.items[k].geo).max(1) as u32;
                        let pts = 2 + rng.below(m) as usize;
                        let g = zigzag(&mut rng, pts, fw, fh);
                        t.res(p.update(k, g), "update")?;
                        ops.push('u');
                    }
                    2 => {
                        // grows
                        let m = records_of(&p.d.items[k].geo) + 2 + rng.below(24) as usize;
                        let g = zigzag(&mut rng, m, fw, fh);
                        t.res(p.update(k, g), "update")?;
                        ops.push('g');
                    }
                    3 => {
                        // grows twice before the frame
                        let m = records_of(&p.d.items[k].geo) + 1 + rng.below(6) as usize;
                        let g = zigzag(&mut rng, m, fw, fh);
                        t.res(p.update(k, g), "update")?;
                        let pts = m + 3 + rng.below(30) as usize;
                        let g = zigzag(&mut rng, pts, fw, fh);
                        t.res(p.update(k, g), "update")?;
                        grown_twice += 1;
                        ops.push('G');
                    }
                    4 => {
                        // grows, and is gone right after
                        let m = records_of(&p.d.items[k].geo) + 2 + rng.below(16) as usize;
                        let g = zigzag(&mut rng, m, fw, fh);
                        t.res(p.update(k, g), "update")?;
                        t.res(p.remove(k), "remove")?;
                        grown_removed += 1;
                        ops.push('x');
                    }
                    5 => {
                        t.res(p.remove(k), "remove")?;
                        ops.push('r');
                    }
                    6 => {
                        let st = churn_style(&mut rng);
                        t.res(p.restyle(k, st), "set_style")?;
                        ops.push('s');
                    }
                    _ => {
                        let st = churn_style(&mut rng);
                        t.res(p.swap_style(k, st), "set_entity_style")?;
                        ops.push('e');
                    }
                }
            }
            judge(t, dev, &mut r, &mut off, &mut p, &cam, &format!("{level}: seed {seed}, round {round} ({ops})"), false)?;
        }
        t.ok(grown_twice > 0 && grown_removed > 0, &format!("{level}: seed {seed}: the cases came up: grown twice {grown_twice}, grown and removed {grown_removed}"));
    }
    Some(())
}

// ---------------------------------------------------------------- visibility

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
    let size = VIS_SIZE;
    let mut off = t.res(Off::new(dev, size.0, size.1), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    for (seed, n, spread) in VIS_CASES {
        let (mut s, cam, ents) = t.res(vis_scene(&r, seed, n, spread), "the scene")?;
        let _ = shot(t, dev, &mut off, &mut r, &mut s, "frame")?;
        let (lists, vis) = read_lists(t, dev, &s)?;
        let Some(res) = vis_judge(&s, &lists, &vis, &ents, &cam, size) else {
            t.ok(false, "an entity's records");
            return None;
        };
        t.ok(res.fine(), &format!("{level}: seed {seed}: {n} polylines over {spread}, rotation {:.2}: {}", cam.rotation, res.text()));
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
    // a wide arc with square caps: the corner of a cap lies beyond the box around the chord (the tangent at the piece's
    // end turns up to ARC_STEP / 2 away from the chord).  Placed so that only that corner reaches into the target, at
    // its top left corner: the arc's start (177 - 377 + 200, 152) = (23 - 120, 152), its cap from y 152 down to 72, from
    // x -177 + 80 = -97 inwards; the target's corner (-100, 75) lies 3 px inside the cap's corner
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    let l = t.res(s.add_layer(&Layer::new(0)), "add_layer")?;
    t.res(s.set_camera(&cam), "camera")?;
    let mut st = Style::new(0xFF00_0000);
    st.width = 160.0;
    st.cap = Cap::Square;
    let sid = t.res(s.add_style(&st), "add_style")?;
    t.res(s.insert(l, sid, &render2d::Shape::Arc { center: [-377.0, 152.0], radius: 200.0, start: 0.0, sweep: render2d::ARC_STEP }), "insert")?;
    let a = shot(t, dev, &mut off, &mut r, &mut s, "a square cap's corner, culled")?;
    s.set_culling(false);
    let b = shot(t, dev, &mut off, &mut r, &mut s, "a square cap's corner, not culled")?;
    let ink = b.chunks_exact(4).filter(|p| p[0] < 0x80 && p[1] < 0x80 && p[2] < 0x80).count();
    let nd = a.chunks_exact(4).zip(b.chunks_exact(4)).filter(|(x, y)| x[..3] != y[..3]).count();
    t.ok(ink >= 4 && nd == 0, &format!("{level}: a wide arc's square cap reaching into a corner of the target: {ink} dark pixels drawn, culled against not culled: {nd} pixels differ"));
    // fills and curves with only their extreme part in the target (cull.wgsl: a fill's edge its box and 2 px, a triangle the
    // box of its corners and 2 px; a curve's segments as any): a fill whose outline lies 0.3 px inside the target's left edge
    // (its smoothed edge alone in view), a triangle whose tip alone reaches 1.5 px into the bottom right corner, a cubic whose
    // bulge alone reaches a pixel into the top, a hatched fill with only its edge in view
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    let l = t.res(s.add_layer(&Layer::new(0)), "add_layer")?;
    t.res(s.set_camera(&cam), "camera")?;
    let fill = t.res(s.add_style(&Style::new(0xFF00_0000)), "add_style")?;
    let mut st = Style::new(0xFF00_0000);
    st.width = 2.0;
    let line = t.res(s.add_style(&st), "add_style")?;
    fn ring(v: &[[f64; 2]]) -> [Shape<'_>; 1] {
        [Shape::Polyline { points: v, closed: true }]
    }
    let a = ring(&[[-160.0, -20.0], [-99.7, -20.0], [-99.7, 20.0], [-160.0, 20.0]]);
    t.res(s.insert(l, fill, &Shape::Fill { rings: &[&a] }), "insert")?;
    let b = ring(&[[160.0, -130.0], [98.5, -73.5], [175.0, -95.0]]);
    t.res(s.insert(l, fill, &Shape::Fill { rings: &[&b] }), "insert")?;
    t.res(s.insert(l, line, &Shape::Cubic { points: &[[-150.0, 90.0], [-50.0, 68.0], [50.0, 68.0], [150.0, 90.0]], closed: false }), "insert")?;
    let hst = t.res(s.add_style(&Style::new(0xFF00_0000).hatched(&[render2d::HatchLine::new(0.3, 3.0)], 1.0)), "add_style")?;
    let c = ring(&[[-40.0, -160.0], [40.0, -160.0], [40.0, -74.6], [-40.0, -74.6]]);
    t.res(s.insert(l, hst, &Shape::Fill { rings: &[&c] }), "insert")?;
    let a = shot(t, dev, &mut off, &mut r, &mut s, "extreme parts, culled")?;
    s.set_culling(false);
    let b = shot(t, dev, &mut off, &mut r, &mut s, "extreme parts, not culled")?;
    let (w, _) = VIS_SIZE;
    let at = |img: &[u8], x0: usize, x1: usize, y0: usize, y1: usize| (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| img[(y * w as usize + x) * 4] < 0xD0).count();
    let nd = differ(&a, &b).0;
    let (n1, n2, n3, n4) = (at(&a, 0, 2, 50, 100), at(&a, 196, 200, 146, 150), at(&a, 50, 150, 0, 3), at(&a, 50, 150, 147, 150));
    t.ok(nd == 0 && n1 > 10 && n2 >= 1 && n3 > 10 && n4 > 3,
         &format!("{level}: only extreme parts in the target: a fill's smoothed edge {n1} pixels, a triangle's tip {n2}, a curve's bulge {n3}, a hatched fill's edge {n4}; culled against not culled: {nd} pixels differ"));
    visibility_blocks(t, dev, &mut r, level)
}

// ---------------------------------------------------------------- large coordinates

pub fn coords(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = coords_on(t, dev, level); });
}

fn coords_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let (w, h) = FAR_SIZE;
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    // the same drawing at the origin and at +-1e7, the camera with it (rotated too)
    let base = gpu_image(t, dev, &mut r, &mut off, &sample([0.0, 0.0]), &sample_camera([0.0, 0.0]), "at the origin")?;
    for at in FAR_POINTS {
        let img = gpu_image(t, dev, &mut r, &mut off, &sample(at), &sample_camera(at), "far away")?;
        crate::tests::dump(&format!("coords_{}_{level}", at[0] as i64), &img, w, h);
        crate::tests::dump(&format!("coords_origin_{level}"), &base, w, h);
        let (n, m) = differ(&img, &base);
        t.ok(m <= 1 && n <= (w * h) / 1000, &format!("{level}: the drawing at ({}, {}) against at the origin: {n} pixels differ, largest {m}", at[0], at[1]));
    }
    // arcs, circles and dash patterns: the same
    let base = gpu_image(t, dev, &mut r, &mut off, &arc_sample([0.0, 0.0]), &sample_camera([0.0, 0.0]), "arcs at the origin")?;
    for at in FAR_POINTS {
        let img = gpu_image(t, dev, &mut r, &mut off, &arc_sample(at), &sample_camera(at), "arcs far away")?;
        crate::tests::dump(&format!("coords_arcs_{}_{level}", at[0] as i64), &img, w, h);
        crate::tests::dump(&format!("coords_arcs_origin_{level}"), &base, w, h);
        let (n, m) = differ(&img, &base);
        t.ok(m <= 1 && n <= (w * h) / 1000, &format!("{level}: arcs, circles and dash patterns at ({}, {}) against at the origin: {n} pixels differ, largest {m}", at[0], at[1]));
    }
    // a deep zoom: 1e-6 units per pixel near (-9 876 543.21, 6 543 210.987), tiny pans; white lines on black, each
    // line's place from the coverage of its rows against where it belongs (f64): within 1/16 px
    let mut off = t.res(Off::new(dev, DEEP_SIZE.0, DEEP_SIZE.1), "target")?;
    off.bg = 0xFF00_0000;
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    t.res(build(&mut s, &deep_drawing()), "build")?;
    let mut st = DeepStats::default();
    for step in 0..DEEP_STEPS {
        let c = deep_camera(step);
        t.res(s.set_camera(&c), "camera")?;
        let img = shot(t, dev, &mut off, &mut r, &mut s, "deep zoom")?;
        deep_judge(&img, &c, &mut st);
    }
    t.ok(st.fine(), &format!("{level}: {}", st.text()));
    // the same for rings: circles of radius 1e7, 3e5 and 33 px and an arc of 1e7 through the view
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    t.res(build(&mut s, &deep_arc_drawing()), "build")?;
    let mut st = DeepStats::default();
    for step in 0..DEEP_STEPS {
        let c = deep_camera(step);
        t.res(s.set_camera(&c), "camera")?;
        let img = shot(t, dev, &mut off, &mut r, &mut s, "deep zoom of arcs")?;
        if step == 0 {
            crate::tests::dump(&format!("deep_arcs_{level}"), &img, DEEP_SIZE.0, DEEP_SIZE.1);
        }
        deep_arc_judge(&img, &c, &mut st);
    }
    t.ok(st.fine_arcs(), &format!("{level}: {}", st.arc_text()));
    coords_curves(t, dev, &mut r, level)?;
    coords_blocks(t, dev, &mut r, level)
}

/// curves, fills and hatches far away (G6b1): the same image at +-1e7 as at the origin; at 1e-6 units per pixel an ellipse of
/// radii 1e7 and 6e6 and a Bézier whose controls lie 1e6 away (flattened in a window: records bounded) where they belong
/// within 1/16 px, also after a pan out of the window; a hatch at 1e7 whose lines and outline stay put over the pans
fn coords_curves(t: &mut T, dev: &Device, r: &mut Renderer, level: &str) -> Option<()> {
    let (w, h) = FAR_SIZE;
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    let base = gpu_image(t, dev, r, &mut off, &curve_sample([0.0, 0.0]), &sample_camera([0.0, 0.0]), "curves at the origin")?;
    for at in FAR_POINTS {
        let img = gpu_image(t, dev, r, &mut off, &curve_sample(at), &sample_camera(at), "curves far away")?;
        crate::tests::dump(&format!("coords_curves_{}_{level}", at[0] as i64), &img, w, h);
        let (n, m) = differ(&img, &base);
        t.ok(m <= 1 && n <= (w * h) / 1000, &format!("{level}: curves, a fill with a hole and a hatched fill at ({}, {}) against at the origin: {n} pixels differ, largest {m}", at[0], at[1]));
    }
    // the deep zoom of curves
    let mut off = t.res(Off::new(dev, DEEP_SIZE.0, DEEP_SIZE.1), "target")?;
    off.bg = 0xFF00_0000;
    let (d, fine) = deep_curves();
    let mut s = t.res(Scene::new(r), "Scene::new")?;
    t.res(s.set_camera(&deep_camera(0)), "camera")?;
    let layer = t.res(s.add_layer(&Layer::new(0)), "add_layer")?;
    let mut ids = Vec::new();
    for it in &d.items {
        let sid = t.res(s.add_style(&it.style), "add_style")?;
        ids.push(t.res(with_shape(&it.geo, |sh| s.insert(layer, sid, sh)), "insert")?);
    }
    let mut st = DeepStats::default();
    let mut most = 0u32;
    for step in 0..DEEP_STEPS {
        let c = deep_camera(step);
        t.res(s.set_camera(&c), "camera")?;
        let img = shot(t, dev, &mut off, r, &mut s, "deep zoom of curves")?;
        if step == 0 {
            crate::tests::dump(&format!("deep_curves_{level}"), &img, DEEP_SIZE.0, DEEP_SIZE.1);
        }
        deep_curve_judge(&img, &c, &fine, &mut st);
    }
    // (the records of the two curves)
    let counts: Vec<u32> = ids.iter().filter_map(|&id| s.locate(id).map(|x| x.1)).collect();
    for &c in &counts {
        most = most.max(c);
    }
    t.ok(st.checks > 100 && st.lost == 0 && st.worst <= 1.0 / 16.0 && most <= render2d::curve::CURVE_MAX as u32,
         &format!("{level}: curves at 1e-6 units per pixel near ({}, {}), 24 pans and a rotation, an ellipse of radii 1e7 and 6e6 and a Bézier with controls 1e6 away: {} crossings, \
                   the largest distance from where the curve belongs {:.4} px (1/16 = 0.0625; per curve {:.4}, {:.4}), {} without it; their records {counts:?} (at most {})",
                  DEEP_AT[0], DEEP_AT[1], st.checks, st.worst, st.per_line[0], st.per_line[1], st.lost, render2d::curve::CURVE_MAX));
    // a pan out of the window: flattened again, still in place
    let c = deep_camera(0);
    let mut far = deep_camera(0);
    far.pan(3000.0, 0.0);
    t.res(s.set_camera(&far), "camera")?;
    let _ = shot(t, dev, &mut off, r, &mut s, "panned away")?;
    let re1 = s.reflattened();
    t.res(s.set_camera(&c), "camera")?;
    let img = shot(t, dev, &mut off, r, &mut s, "panned back")?;
    let re2 = s.reflattened();
    let mut st2 = DeepStats::default();
    deep_curve_judge(&img, &c, &fine, &mut st2);
    t.ok(re1 == 2 && re2 == 2 && st2.checks > 4 && st2.lost == 0 && st2.worst <= 1.0 / 16.0,
         &format!("{level}: a pan of 3000 px and back: {re1} and {re2} curves flattened again (their windows), then {} crossings, the largest distance {:.4} px", st2.checks, st2.worst));
    // the hatch at 1e7: its lines and its outline over 24 pans
    let mut sh = t.res(Scene::new(r), "Scene::new")?;
    let mut ss = t.res(Scene::new(r), "Scene::new")?;
    t.res(sh.set_camera(&deep_camera(0)), "camera")?;
    t.res(ss.set_camera(&deep_camera(0)), "camera")?;
    t.res(build(&mut sh, &deep_hatch(false)), "build")?;
    t.res(build(&mut ss, &deep_hatch(true)), "build")?;
    let (mut lines, mut edge) = (DeepStats::default(), DeepStats::default());
    for step in 0..DEEP_STEPS {
        let c = deep_camera(step);
        t.res(sh.set_camera(&c), "camera")?;
        t.res(ss.set_camera(&c), "camera")?;
        let a = shot(t, dev, &mut off, r, &mut sh, "deep hatch")?;
        let b = shot(t, dev, &mut off, r, &mut ss, "deep fill")?;
        if step == 0 {
            crate::tests::dump(&format!("deep_hatch_{level}"), &a, DEEP_SIZE.0, DEEP_SIZE.1);
        }
        deep_hatch_judge(&a, &b, &c, &mut lines, &mut edge);
    }
    t.ok(lines.checks > 100 && lines.lost == 0 && lines.worst <= 1.0 / 16.0 && edge.checks > 100 && edge.worst <= 1.0 / 16.0,
         &format!("{level}: a hatch at 1e-6 units per pixel near ({}, {}) (lines 25 px apart, their base 1e7 away; a fill of 2e7 units), 24 pans and a rotation: the lines' centroids at most {:.4} px \
                   from where they belong ({} columns), the outline's edge at most {:.4} px ({} rows; 1/16 = 0.0625)", DEEP_AT[0], DEEP_AT[1], lines.worst, lines.checks, edge.worst, edge.checks));
    Some(())
}

// ---------------------------------------------------------------- the dash pattern's place far away

pub fn phase(t: &mut T) {
    on_levels(t, &mut |t, dev, level| { let _ = phase_on(t, dev, level); });
}

/// a dashed polyline at 1e7 with segments 1e7 long and a dashed circle of radius 1e7, at 1e-6 units per pixel, the pattern
/// in pixels and in drawing units: 24 pans of 0.37 px, every dash's end where it belongs within 1/16 px
fn phase_on(t: &mut T, dev: &Device, level: &str) -> Option<()> {
    let mut off = t.res(Off::new(dev, PHASE_SIZE.0, PHASE_SIZE.1), "target")?;
    off.bg = 0xFF00_0000;
    let mut r = t.res(Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new")?;
    for k in 0..PHASE_CASES {
        let mut s = t.res(Scene::new(&r), "Scene::new")?;
        t.res(build(&mut s, &phase_case(k, 0).d), "build")?;
        let (mut worst, mut checks) = (0.0f64, 0u32);
        let mut name = "";
        for step in 0..24 {
            let case = phase_case(k, step);
            name = case.name;
            t.res(s.set_camera(&case.cam), "camera")?;
            let img = shot(t, dev, &mut off, &mut r, &mut s, "phase")?;
            if step == 0 {
                crate::tests::dump(&format!("phase_{k}_{level}"), &img, PHASE_SIZE.0, PHASE_SIZE.1);
            }
            phase_judge(&img, &case, k, &mut worst, &mut checks);
        }
        t.ok(checks >= 100 && worst <= 1.0 / 16.0, &format!("{level}: dashes at 1e-6 units per pixel, {name}: {checks} pixel lines over 24 pans, the largest distance of a dash's end from where \
                                                      it belongs {worst:.4} px (1/16 = 0.0625)"));
    }
    Some(())
}

// ---------------------------------------------------------------- blocks: culling of inserts, large coordinates (G6b2)

/// inserts with only an extreme part in the target (cull.wgsl: an instance by its block's box grown by its strokes' reach,
/// then its records as any): a turned and scaled insert whose miter's tip alone reaches in, a fill's smoothed edge alone
/// 0.3 px inside, a nested insert's ring alone touching the top, a mirrored insert's curve bulging a pixel into the bottom;
/// culled against not culled; an insert far away puts nothing in the visible list
pub fn visibility_blocks(t: &mut T, dev: &Device, r: &mut Renderer, level: &str) -> Option<()> {
    let size = VIS_SIZE;
    let (w, _) = size;
    let mut off = t.res(Off::new(dev, size.0, size.1), "target")?;
    let cam = Camera::new([0.0, 0.0], 1.0);
    let mut bd = BlockDrawing { layers: vec![(true, 0, 0)], blocks: Vec::new(), items: Vec::new() };
    // 0: a sharp turn pointing to +x at (0, 0), 6 units wide, miter limit 10: its interior angle 28.07 degrees, its miter's
    // tip 3 / sin(14.04 deg) = 12.37 units beyond the vertex
    let mut mst = style(0xFF00_0000, 6.0, Cap::Butt, Join::Miter);
    mst.unit = Unit::Drawing;
    mst.miter_limit = 10.0;
    bd.blocks.push(BDef { base: [0.0, 0.0], items: vec![(mst, BGeo::G(Geo::Poly(vec![[-40.0, 10.0], [0.0, 0.0], [-40.0, -10.0]], false)))] });
    // 1: a fill 20 by 12
    bd.blocks.push(BDef { base: [0.0, 0.0], items: vec![(style(0xFF00_0000, 0.0, Cap::Butt, Join::Miter), BGeo::G(Geo::Fill(vec![vec![Geo::Poly(vec![[0.0, 0.0], [20.0, 0.0], [20.0, 12.0], [0.0, 12.0]], true)]])))] });
    // 2: a ring of radius 10, 8 px wide; 3: block 2 nested (its base on the ring's centre)
    bd.blocks.push(BDef { base: [0.0, 0.0], items: vec![(style(0xFF00_0000, 8.0, Cap::Butt, Join::Miter), BGeo::G(Geo::Circle([0.0, 0.0], 10.0)))] });
    bd.blocks.push(BDef { base: [5.0, 5.0], items: vec![(style(0xFF00_0000, 1.0, Cap::Butt, Join::Miter), BGeo::Ins(BIns::new(2, [5.0, 5.0])))] });
    // 4: a cubic 2 px wide whose bulge reaches 16.5 units above its ends
    bd.blocks.push(BDef { base: [0.0, 0.0], items: vec![(style(0xFF00_0000, 2.0, Cap::Butt, Join::Miter), BGeo::G(Geo::Bez(vec![[-50.0, 0.0], [-20.0, 22.0], [20.0, 22.0], [50.0, 0.0]], 3, false)))] });
    let one = style(0xFF00_0000, 1.0, Cap::Butt, Join::Miter);
    // (the turn scaled 2.5 and turned half round: its tip at 129.4 - 30.9 = 98.47, 1.5 px inside the right edge, the rest outside)
    bd.items.push((0, one, BGeo::Ins(BIns::new(0, [129.4, 20.0]).turned(core::f64::consts::PI, [2.5, 2.5]))));
    // (the fill turned a quarter and scaled 2.5: x from -129.7 to -99.7, its edge 0.3 px inside the left edge)
    bd.items.push((0, one, BGeo::Ins(BIns::new(1, [-99.7, -20.0]).turned(core::f64::consts::FRAC_PI_2, [2.5, 2.5]))));
    // (the ring scaled 3 through the nested insert: radius 30, half width 4: its lowest point 0.8 px below the top edge)
    bd.items.push((0, one, BGeo::Ins(BIns::new(3, [-40.0, 108.2]).turned(0.0, [3.0, 3.0]))));
    // (the cubic mirrored left to right (turned half round, y scaled by -1): its ends 15.5 px below the bottom edge, its
    // bulge a pixel above it)
    bd.items.push((0, one, BGeo::Ins(BIns::new(4, [30.0, -75.0 - 15.5]).turned(core::f64::consts::PI, [1.0, -1.0]))));
    let mut s = t.res(Scene::new(r), "Scene::new")?;
    t.res(s.set_camera(&cam), "camera")?;
    t.res(build_blocks(&mut s, &bd), "build")?;
    let a = shot(t, dev, &mut off, r, &mut s, "inserts' extreme parts, culled")?;
    s.set_culling(false);
    let b = shot(t, dev, &mut off, r, &mut s, "inserts' extreme parts, not culled")?;
    crate::tests::dump(&format!("vis_inserts_{level}"), &a, size.0, size.1);
    let at = |img: &[u8], x0: usize, x1: usize, y0: usize, y1: usize| (y0..y1).flat_map(|y| (x0..x1).map(move |x| (x, y))).filter(|&(x, y)| img[(y * w as usize + x) * 4] < 0xD0).count();
    let nd = differ(&a, &b).0;
    let (n1, n2, n3, n4) = (at(&a, 196, 200, 40, 70), at(&a, 0, 2, 40, 120), at(&a, 40, 120, 0, 2), at(&a, 0, 200, 147, 150));
    t.ok(nd == 0 && n1 >= 1 && n2 > 10 && n3 >= 4 && n4 > 3,
         &format!("{level}: inserts with only extreme parts in the target: a miter's tip {n1} pixels, a fill's smoothed edge {n2}, a nested ring {n3}, a mirrored curve's bulge {n4}; culled against not culled: {nd} pixels differ"));
    // an insert far away: none of its records in the visible list (the instance is culled before its records)
    let mut s = t.res(Scene::new(r), "Scene::new")?;
    t.res(s.set_camera(&cam), "camera")?;
    s.set_stats(true);
    let mut far = bd.clone();
    far.items = vec![(0, one, BGeo::Ins(BIns::new(2, [0.0, 0.0]))), (0, one, BGeo::Ins(BIns::new(2, [1000.0, 0.0]).array(50, 50, [30.0, 30.0])))];
    t.res(build_blocks(&mut s, &far), "build")?;
    let _ = shot(t, dev, &mut off, r, &mut s, "far inserts")?;
    t.res(dev.wait_idle(), "wait_idle")?;
    s.collect_stats();
    let vis = s.take_stats().last().map_or(0, |f| f.visible);
    t.ok(vis == 16, &format!("{level}: a circle inserted in view and 2500 copies far away: {vis} records visible (the circle's 16 pieces)"));
    Some(())
}

/// a block inserted far away, and at 1e-6 units per pixel near 1e7: its lines and rings where they belong within 1/16 px,
/// for an insert that moves them there and for one that scales a block of small units by 1e5 (turned, mirrored)
pub fn coords_blocks(t: &mut T, dev: &Device, r: &mut Renderer, level: &str) -> Option<()> {
    let (w, h) = FAR_SIZE;
    let mut off = t.res(Off::new(dev, w, h), "target")?;
    // the sample drawing as a block, inserted at the origin and at +-1e7 (its base at its own origin)
    let base = gpu_image(t, dev, r, &mut off, &sample([0.0, 0.0]), &sample_camera([0.0, 0.0]), "at the origin")?;
    let def = BDef { base: [0.0, 0.0], items: sample([0.0, 0.0]).items.into_iter().map(|it| (it.style, BGeo::G(it.geo))).collect() };
    for at in FAR_POINTS {
        let bd = BlockDrawing { layers: vec![(true, 0, 0)], blocks: vec![def.clone()], items: vec![(0, style(0xFF00_0000, 1.0, Cap::Butt, Join::Miter), BGeo::Ins(BIns::new(0, at)))] };
        let img = crate::blocks::block_image(t, dev, r, &mut off, &bd, &sample_camera(at), "far away")?;
        let (n, m) = differ(&img, &base);
        t.ok(m <= 1 && n <= (w * h) / 1000, &format!("{level}: the sample inserted at ({}, {}) against drawn at the origin: {n} pixels differ, largest {m}", at[0], at[1]));
    }
    // the deep zoom: the lines and rings of the deep tests as blocks
    let mut off = t.res(Off::new(dev, DEEP_SIZE.0, DEEP_SIZE.1), "target")?;
    off.bg = 0xFF00_0000;
    let t0 = [-9_876_543.0, 6_543_211.0];
    for (name, scale, rot, mirror) in [("moved there", 1.0f64, 0.0f64, false), ("scaled by 1e5, turned", 1.0e5, 0.3, false), ("scaled by 1e5, turned, mirrored", 1.0e5, -1.1, true)] {
        // local = M^-1 (world - t), M = R(rot) diag(scale, +-scale)
        let (sn, cs) = render2d::math::sincos(rot);
        let sy = if mirror { -scale } else { scale };
        let local = |p: [f64; 2]| {
            let (dx, dy) = (p[0] - t0[0], p[1] - t0[1]);
            [(cs * dx + sn * dy) / scale, (-sn * dx + cs * dy) / sy]
        };
        let ins = BIns::new(0, t0).turned(rot, [scale, sy]);
        let bd = deep_insert_drawing(scale, rot, mirror);
        let mut s = t.res(Scene::new(r), "Scene::new")?;
        t.res(s.set_camera(&deep_camera(0)), "camera")?;
        t.res(build_blocks(&mut s, &bd), "build")?;
        let mut st = DeepStats::default();
        for step in 0..DEEP_STEPS {
            let c = deep_camera(step);
            t.res(s.set_camera(&c), "camera")?;
            let img = shot(t, dev, &mut off, r, &mut s, "deep zoom of an insert")?;
            deep_judge(&img, &c, &mut st);
        }
        t.ok(st.fine(), &format!("{level}: an insert {name}: {}", st.text()));
        // the rings (circles keep their angle 0 where the turn takes it; a mirror reverses them: not here)
        if mirror {
            continue;
        }
        let mut rings = BDef { base: [0.0, 0.0], items: Vec::new() };
        for (k, (c, rr)) in deep_circles().into_iter().enumerate() {
            let st = style(0xFFFF_FFFF, 2.5, Cap::Butt, Join::Miter);
            let geo = if k == 4 { Geo::Arc(local(c), rr / scale, core::f64::consts::FRAC_PI_2 - 0.1 - rot, 0.2) } else { Geo::Circle(local(c), rr / scale) };
            rings.items.push((st, BGeo::G(geo)));
        }
        let bd = BlockDrawing { layers: vec![(true, 0, 0)], blocks: vec![rings], items: vec![(0, style(0xFF00_0000, 1.0, Cap::Butt, Join::Miter), BGeo::Ins(ins))] };
        let mut s = t.res(Scene::new(r), "Scene::new")?;
        t.res(s.set_camera(&deep_camera(0)), "camera")?;
        t.res(build_blocks(&mut s, &bd), "build")?;
        let mut st = DeepStats::default();
        for step in 0..DEEP_STEPS {
            let c = deep_camera(step);
            t.res(s.set_camera(&c), "camera")?;
            let img = shot(t, dev, &mut off, r, &mut s, "deep zoom of an insert's rings")?;
            deep_arc_judge(&img, &c, &mut st);
        }
        t.ok(st.fine_arcs(), &format!("{level}: an insert {name}: {}", st.arc_text()));
    }
    Some(())
}
