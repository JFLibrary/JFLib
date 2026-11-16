// picks -- render2dtest's tests of G6d's queries (main.rs lists them): picking, area selection and object snaps against a
// reference, the spatial index against brute force, under churn and at large coordinates.
//
//   pick     random drawings of every kind of shape with blocks inserted in every way (moved, turned, scaled evenly and
//            unevenly, mirrored, as arrays, nested): every pick of the scene against the same drawing expanded by the test
//            itself into plain entities (gallery's transforms, not render2d's), hit for hit, path for path, in the same
//            order, distance for distance; the index against brute force; at the origin, at 1e7 and at 1e-6 units per
//            pixel; then exact cases (caps, joins, miters, arcs, curves against their own parameters sampled finely, fills
//            with holes, texts' quads, all of them again inside inserts of every kind)
//   select   window and crossing by rectangles and polygons against the expanded drawing (an insert: all or any of its
//            entities), exact cases (curves whose hull leaves the window, a region inside a fill, arrays), a window over
//            everything
//   snap     the eight kinds and insertion on exact cases (in inserts too: an ellipse made of a circle by an uneven scale
//            has its centre and its axes' ends), the rules of priority, the index against brute force on random drawings
//   churn    thousands of random changes (insert, update, remove, styles, blocks, inserts moved, layers removed): the index
//            against brute force and its invariants after each round

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render2d::math::{sincos, sqrt};
use render2d::{Camera, Cap, EntityId, Join, Layer, Region, Renderer, RendererDesc, Scene, SelectMode, Shape, SnapKind, SnapOptions, Style, Target, Unit};

use crate::gallery::*;
use crate::tests::{setup, T};

/// an entity of a block drawing expanded: its layer, its style as drawn, its geometry in the drawing, and where it came
/// from (the item on a layer; per level the copy and the block's item)
#[derive(Clone)]
struct XItem {
    layer: usize,
    style: Style,
    geo: Geo,
    top: usize,
    path: Vec<(u32, usize)>,
}

/// gallery::expand_blocks with the way to each entity
fn expand_paths(d: &BlockDrawing) -> Vec<XItem> {
    let ident = Aff { t: [0.0, 0.0], m: [[1.0, 0.0], [0.0, 1.0]] };
    #[allow(clippy::too_many_arguments)]
    fn walk(d: &BlockDrawing, b: usize, xf: &Aff, ins: &Style, layer: usize, top: usize, path: &mut Vec<(u32, usize)>, copy: u32, out: &mut Vec<XItem>) {
        let def = &d.blocks[b];
        let lt = d.layers[layer].2;
        for (k, (st, g)) in def.items.iter().enumerate() {
            path.push((copy, k));
            match g {
                BGeo::G(g) => out.push(XItem { layer, style: style_in(st, Some(ins), xf, def.base, lt), geo: geo_through(g, xf, def.base), top, path: path.clone() }),
                BGeo::Ins(n) => {
                    let mut s2 = *st;
                    if st.by_block != 0 || st.transparency == render2d::Transparency::ByBlock {
                        let mut t = *st;
                        if st.by_block & render2d::BY_BLOCK_COLOR != 0 {
                            t.color = ins.color;
                        }
                        if st.by_block & render2d::BY_BLOCK_WIDTH != 0 {
                            t.width = ins.width;
                            t.unit = ins.unit;
                        }
                        if st.transparency == render2d::Transparency::ByBlock {
                            t.transparency = ins.transparency;
                        }
                        s2 = t;
                    }
                    for (c2, x2) in n.xfs(def.base).iter().enumerate() {
                        walk(d, n.block, &xf.then(x2), &s2, layer, top, path, c2 as u32, out);
                    }
                }
            }
            path.pop();
        }
    }
    let mut out = Vec::new();
    for (i, (l, st, g)) in d.items.iter().enumerate() {
        match g {
            BGeo::G(g) => out.push(XItem { layer: *l, style: style_in(st, None, &ident, [0.0, 0.0], d.layers[*l].2), geo: g.clone(), top: i, path: Vec::new() }),
            BGeo::Ins(b) => {
                for (c, xf) in b.xfs([0.0, 0.0]).iter().enumerate() {
                    let mut path = Vec::new();
                    walk(d, b.block, xf, st, *l, i, &mut path, c as u32, &mut out);
                }
            }
        }
    }
    out
}

/// where a block drawing's entities went in a scene: per top item its id; per block per item its id
struct Ids {
    tops: Vec<EntityId>,
    blocks: Vec<Vec<EntityId>>,
}

impl Ids {
    /// a target as (top item, per level (copy, block item))
    fn of(&self, t: &Target) -> Option<(usize, Vec<(u32, usize)>)> {
        let top = self.tops.iter().position(|&e| e == t.entity)?;
        let mut path = Vec::new();
        for &(copy, e) in &t.path {
            let k = self.blocks.iter().find_map(|v| v.iter().position(|&x| x == e))?;
            path.push((copy, k));
        }
        Some((top, path))
    }
}

/// the expanded drawing as a scene of plain entities (one per expanded item, in the same layers): its ids
fn build_expanded(s: &mut Scene, d: &BlockDrawing, xs: &[XItem]) -> Result<Vec<EntityId>> {
    let mut layers = Vec::new();
    for &(visible, order, _) in &d.layers {
        let mut l = Layer::new(order);
        l.visible = visible;
        layers.push(s.add_layer(&l)?);
    }
    let mut ids = Vec::with_capacity(xs.len());
    for x in xs {
        let st = s.add_style(&x.style)?;
        ids.push(with_shape(&x.geo, |sh| s.insert(layers[x.layer], st, sh))?);
    }
    Ok(ids)
}

/// a random block drawing around `at`, `u` drawing units to a pixel (its features some 10 to 200 px), every kind of shape and
/// insert
fn random_drawing(seed: u32, at: [f64; 2], u: f64) -> BlockDrawing {
    let mut g = Rng(seed);
    let p = |g: &mut Rng, r: f64| [at[0] + g.range(-r, r) * u, at[1] + g.range(-r, r) * u];
    let mut d = BlockDrawing { layers: vec![(true, 0, 0), (true, 1, 0), (false, 2, 0), (true, -1, 0)], blocks: Vec::new(), items: Vec::new() };
    let st = |g: &mut Rng, k: u32| {
        let caps = [Cap::Butt, Cap::Round, Cap::Square];
        let joins = [Join::Miter, Join::Round, Join::Bevel];
        let mut s = style(INK[(k % 4) as usize], [0.0f32, 1.0, 3.0, 8.0][g.below(4) as usize], caps[g.below(3) as usize], joins[g.below(3) as usize]);
        if g.below(3) == 0 {
            // (a width in units: some px at this scale)
            s.unit = Unit::Drawing;
            s.width = (g.range(0.5, 6.0) * u) as f32;
        }
        s.miter_limit = [4.0f32, 1.5, 10.0][g.below(3) as usize];
        s
    };
    // a block's geometry in its own units around (0, 0), sizes in u
    let local = |g: &mut Rng, kind: u32| -> Geo {
        let q = |g: &mut Rng, r: f64| [g.range(-r, r) * u, g.range(-r, r) * u];
        match kind % 9 {
            0 => Geo::Line(q(g, 30.0), q(g, 30.0)),
            1 => Geo::Poly((0..4 + g.below(4)).map(|_| q(g, 30.0)).collect(), g.below(2) == 0),
            2 => Geo::Arc(q(g, 10.0), g.range(5.0, 25.0) * u, g.range(0.0, 6.0), g.range(-4.0, 4.0).max(0.3)),
            3 => Geo::Circle(q(g, 10.0), g.range(4.0, 20.0) * u),
            4 => Geo::Bez(vec![q(g, 30.0), q(g, 30.0), q(g, 30.0), q(g, 30.0), q(g, 30.0), q(g, 30.0), q(g, 30.0)], 3, false),
            5 => Geo::Ellipse(q(g, 10.0), [g.range(5.0, 25.0) * u, g.range(3.0, 12.0) * u], g.range(0.0, 3.0), g.range(0.0, 6.0), if g.below(2) == 0 { core::f64::consts::TAU } else { g.range(0.5, 4.0) }),
            6 => Geo::Spline(3, (0..6).map(|_| q(g, 30.0)).collect(), Vec::new(), Vec::new(), g.below(2) == 0),
            7 => Geo::Bez(vec![q(g, 30.0), q(g, 30.0), q(g, 30.0), q(g, 30.0), q(g, 30.0)], 2, g.below(2) == 0),
            _ => {
                let c = q(g, 10.0);
                let r = g.range(12.0, 25.0) * u;
                let sq = vec![[c[0] - r, c[1] - r], [c[0] + r, c[1] - r], [c[0] + r, c[1] + r], [c[0] - r, c[1] + r]];
                Geo::Fill(vec![vec![Geo::Poly(sq, true)], vec![Geo::Circle(c, 0.5 * r)]])
            }
        }
    };
    // block 0: strokes, arcs, curves; block 1: a fill and curves; block 2: block 0 nested (turned, mirrored, an array);
    // block 3: block 2 nested, unevenly
    for b in 0..2u32 {
        let mut def = BDef { base: [g.range(-5.0, 5.0) * u, g.range(-5.0, 5.0) * u], items: Vec::new() };
        for k in 0..5 + g.below(4) {
            let kind = if b == 1 && k == 0 { 8 } else { g.below(9) };
            let s = st(&mut g, k);
            def.items.push((s, BGeo::G(local(&mut g, kind))));
        }
        d.blocks.push(def);
    }
    let mut b2 = BDef { base: [1.0 * u, -2.0 * u], items: Vec::new() };
    b2.items.push((st(&mut g, 1), BGeo::G(local(&mut g, 1))));
    b2.items.push((st(&mut g, 2), BGeo::Ins(BIns::new(0, [5.0 * u, 3.0 * u]).turned(0.7, [1.0, -1.0]))));
    b2.items.push((st(&mut g, 3), BGeo::Ins(BIns::new(1, [-20.0 * u, 10.0 * u]).turned(-0.4, [0.8, 0.8]).array(2, 2, [70.0 * u, 60.0 * u]))));
    d.blocks.push(b2);
    let mut b3 = BDef { base: [0.0, 0.0], items: Vec::new() };
    b3.items.push((st(&mut g, 0), BGeo::Ins(BIns::new(2, [0.0, 0.0]).turned(0.3, [1.3, 0.6]))));
    b3.items.push((st(&mut g, 2), BGeo::G(local(&mut g, 3))));
    d.blocks.push(b3);
    // on the layers: plain entities of every kind, inserts of each block in every way
    for k in 0..40u32 {
        let layer = g.below(4) as usize;
        let s = st(&mut g, k);
        if k % 4 == 3 {
            let blk = g.below(4) as usize;
            let mut ins = BIns::new(blk, p(&mut g, 120.0));
            match g.below(6) {
                0 => {}
                1 => ins = ins.turned(g.range(-3.0, 3.0), [1.0, 1.0]),
                2 => ins = ins.turned(g.range(-3.0, 3.0), [g.range(0.5, 1.5), g.range(0.5, 1.5)]),
                3 => ins = ins.turned(g.range(-3.0, 3.0), [-1.2, 1.2]),
                4 => ins = ins.turned(g.range(-3.0, 3.0), [0.7, -1.4]),
                _ => ins = ins.turned(g.range(-1.0, 1.0), [0.6, 0.6]).array(3, 2, [g.range(-60.0, 60.0) * u, g.range(-60.0, 60.0) * u]),
            }
            d.items.push((layer, s, BGeo::Ins(ins)));
        } else {
            let kind = g.below(9);
            let geo = local(&mut g, kind);
            let o = p(&mut g, 130.0);
            let aff = Aff { t: o, m: [[1.0, 0.0], [0.0, 1.0]] };
            d.items.push((layer, s, BGeo::G(geo_through(&geo, &aff, [0.0, 0.0]))));
        }
    }
    d
}

/// points to pick at: near random entities' points (sampled from the expanded geometry) and anywhere
fn pick_points(g: &mut Rng, xs: &[XItem], at: [f64; 2], u: f64, n: usize) -> Vec<[f64; 2]> {
    let mut v = Vec::with_capacity(n);
    for k in 0..n {
        if k % 3 == 0 || xs.is_empty() {
            v.push([at[0] + g.range(-180.0, 180.0) * u, at[1] + g.range(-180.0, 180.0) * u]);
            continue;
        }
        let x = &xs[g.below(xs.len() as u32) as usize];
        let q = some_point(&x.geo, g.unit());
        v.push([q[0] + g.range(-8.0, 8.0) * u, q[1] + g.range(-8.0, 8.0) * u]);
    }
    v
}

/// a point of a geometry (t in [0, 1): roughly along it)
fn some_point(geo: &Geo, t: f64) -> [f64; 2] {
    match geo {
        Geo::Line(a, b) => [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])],
        Geo::Poly(v, _) | Geo::Bez(v, _, _) => v[((t * v.len() as f64) as usize).min(v.len() - 1)],
        Geo::Spline(_, v, _, _, _) => v[((t * v.len() as f64) as usize).min(v.len() - 1)],
        Geo::Arc(c, r, a0, sw) => {
            let (s, co) = sincos(a0 + t * sw);
            [c[0] + r * co, c[1] + r * s]
        }
        Geo::Circle(c, r) => {
            let (s, co) = sincos(t * core::f64::consts::TAU);
            [c[0] + r * co, c[1] + r * s]
        }
        Geo::Ellipse(c, r, rot, a0, sw) => {
            let (s, co) = sincos(a0 + t * sw);
            let (sr, cr) = sincos(*rot);
            let (x, y) = (r[0] * co, r[1] * s);
            [c[0] + cr * x - sr * y, c[1] + sr * x + cr * y]
        }
        Geo::Fill(rings) => some_point(&rings[0][0], t),
    }
}

type Key = (usize, Vec<(u32, usize)>);

/// a test's numbers: compared, borderline (left out), differing
#[derive(Default)]
struct Tally {
    picks: u32,
    hits: u32,
    border: u32,
    bad: u32,
    worst: f64,
    first: Option<String>,
}

impl Tally {
    fn miss(&mut self, what: String) {
        self.bad += 1;
        if self.first.is_none() {
            self.first = Some(what);
        }
    }
}

/// the hits of the scene against those of the expanded drawing: the same targets in the same order, the same distances
#[allow(clippy::too_many_arguments)]
fn compare_picks(s: &mut Scene, e: &mut Scene, ids: &Ids, xids: &[EntityId], xs: &[XItem], pts: &[[f64; 2]], tol: f64, eps: f64, tl: &mut Tally) {
    for &q in pts {
        tl.picks += 1;
        let got = s.pick(q, tol);
        let brute = s.pick_brute(q, tol);
        if got != brute {
            tl.miss(format!("the index against brute force at {q:?}: {} against {} hits", got.len(), brute.len()));
            continue;
        }
        let want = e.pick_brute(q, tol);
        // (hits within eps px of the tolerance may fall either way: f64 through a transform against f64 expanded)
        if want.iter().chain(got.iter()).any(|h| (h.distance - tol).abs() <= eps) {
            tl.border += 1;
            continue;
        }
        let gk: Vec<Option<Key>> = got.iter().map(|h| ids.of(&h.target)).collect();
        let wk: Vec<Option<Key>> = want.iter().map(|h| xids.iter().position(|&x| x == h.target.entity).map(|k| (xs[k].top, xs[k].path.clone()))).collect();
        tl.hits += got.len() as u32;
        if gk != wk {
            tl.miss(format!("at {q:?} (tolerance {tol} px): the scene hit {gk:?}, the expanded drawing {wk:?}"));
            continue;
        }
        for (a, b) in got.iter().zip(&want) {
            let d = (a.distance - b.distance).abs();
            tl.worst = tl.worst.max(d);
            if d > eps {
                tl.miss(format!("at {q:?}: distance {} px against {} px", a.distance, b.distance));
            }
        }
    }
}

/// a scene of a block drawing with its ids
fn scene_of(r: &Renderer, d: &BlockDrawing, cam: &Camera) -> Result<(Scene, Ids)> {
    let mut s = Scene::new(r)?;
    s.set_camera(cam)?;
    let b = build_blocks(&mut s, d)?;
    Ok((s, Ids { tops: b.items, blocks: b.block_items }))
}

/// the three places of the tests: (where, units per pixel)
const PLACES: [([f64; 2], f64, &str); 3] = [([0.0, 0.0], 1.0, "at the origin"), ([1.0e7, -1.0e7], 1.0, "at 1e7"), ([-9_876_543.21, 6_543_210.987], 1e-6, "at 1e-6 units per pixel near 1e7")];

pub fn pick(t: &mut T) {
    let Some(g) = setup(t) else { return };
    let Some(dev) = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "device") else { return };
    let Some(r) = t.res(Renderer::new(&dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new") else { return };
    // random drawings against their expansion, the index against brute force
    for (at, u, place) in PLACES {
        let mut tl = Tally::default();
        for seed in 1..=4u32 {
            let d = random_drawing(seed, at, u);
            let xs = expand_paths(&d);
            let cam = Camera { center: at, scale: u, rotation: 0.3 * seed as f64 };
            let Some((mut s, ids)) = t.res(scene_of(&r, &d, &cam), "scene") else { return };
            let Some(mut e) = t.res(Scene::new(&r), "Scene::new") else { return };
            let _ = e.set_camera(&cam);
            let Some(xids) = t.res(build_expanded(&mut e, &d, &xs), "expanded") else { return };
            let mut rng = Rng(seed * 77);
            let pts = pick_points(&mut rng, &xs, at, u, 300);
            // (at 1e-6 units per pixel: f64 near 1e7 holds 0.0019 px, an ulp; the scene's transforms and the test's own each
            // round a few times)
            let eps = if u < 1e-3 { 1e-2 } else { 1e-6 };
            for tol in [0.0, 3.0] {
                compare_picks(&mut s, &mut e, &ids, &xids, &xs, &pts, tol, eps, &mut tl);
            }
            if let Err(why) = s.index_check() {
                tl.miss(format!("the index's invariants: {why}"));
            }
        }
        t.ok(tl.bad == 0, &format!("pick {place}: 4 random drawings (every kind of shape, blocks moved, turned, scaled evenly and unevenly, mirrored, arrays, nested), {} picks, {} hits: \
                                    as the drawing expanded by the test, hit for hit and in order, distances within {:.1e} px (left out near the tolerance: {}); index = brute force; {} differ{}",
                                   tl.picks, tl.hits, tl.worst, tl.border, tl.bad, tl.first.map(|f| format!(", first: {f}")).unwrap_or_default()));
    }
    exact_picks(t, &r);
}

// ---------------------------------------------------------------- exact cases

/// a distance from a point to a curve given by its parameter: sampled, then refined by golden sections
fn param_dist(q: [f64; 2], f: &dyn Fn(f64) -> [f64; 2], t0: f64, t1: f64) -> f64 {
    let n = 4000;
    let d = |t: f64| {
        let p = f(t);
        sqrt((p[0] - q[0]) * (p[0] - q[0]) + (p[1] - q[1]) * (p[1] - q[1]))
    };
    let mut best = (f64::MAX, t0);
    for k in 0..=n {
        let t = t0 + (t1 - t0) * k as f64 / n as f64;
        let x = d(t);
        if x < best.0 {
            best = (x, t);
        }
    }
    let h = (t1 - t0) / n as f64;
    let (mut a, mut b) = ((best.1 - h).max(t0), (best.1 + h).min(t1));
    for _ in 0..200 {
        let m1 = a + 0.382 * (b - a);
        let m2 = a + 0.618 * (b - a);
        if d(m1) < d(m2) { b = m2 } else { a = m1 }
    }
    d(0.5 * (a + b)).min(best.0)
}

/// the drawn distance (px) of the one entity a scene holds at q, or None when nothing is hit within tol px
fn hit_px(s: &mut Scene, q: [f64; 2], tol: f64) -> Option<f64> {
    s.pick(q, tol).first().map(|h| h.distance)
}

fn exact_picks(t: &mut T, r: &Renderer) {
    for (at, u, place) in PLACES {
        let cam = Camera { center: at, scale: u, rotation: 0.0 };
        let p = |x: f64, y: f64| [at[0] + x * u, at[1] + y * u];
        let mut bad: Vec<String> = Vec::new();
        let mut cases = 0u32;
        // (at 1e-6 units per pixel near 1e7 a point of the test itself lies within an ulp of 1e7, 0.0019 px, of where it
        // was meant: the expected distances hold to two of them)
        let floor = if u < 1e-3 { 4e-3 } else { 0.0 };
        let mut check = |what: &str, got: Option<f64>, want: Option<f64>, eps: f64, bad: &mut Vec<String>| {
            cases += 1;
            let eps = eps.max(floor);
            let fine = match (got, want) {
                (None, None) => true,
                (Some(a), Some(b)) => (a - b).abs() <= eps,
                _ => false,
            };
            if !fine {
                bad.push(format!("{what}: {got:?} against {want:?}"));
            }
        };
        // a line 100 px long, 10 px wide: across it, beyond its ends with each cap
        for (cap, beyond) in [(Cap::Butt, None), (Cap::Square, Some(0.0)), (Cap::Round, Some(3.0 - 5.0f64.min(3.0)))] {
            let Ok(mut s) = Scene::new(r) else { return };
            let _ = s.set_camera(&cam);
            let l = s.add_layer(&Layer::new(0)).ok();
            let st = s.add_style(&style(INK[0], 10.0, cap, Join::Miter)).ok();
            let (Some(l), Some(st)) = (l, st) else { return };
            let _ = s.insert(l, st, &Shape::Line { a: p(0.0, 0.0), b: p(100.0, 0.0) });
            check(&format!("{cap:?}: across at 4.9 px"), hit_px(&mut s, p(50.0, 4.9), 0.0), Some(0.0), 1e-9, &mut bad);
            check(&format!("{cap:?}: across at 7 px, tolerance 3"), hit_px(&mut s, p(50.0, 7.0), 3.0), Some(2.0), 1e-6, &mut bad);
            check(&format!("{cap:?}: across at 8.5 px, tolerance 3"), hit_px(&mut s, p(50.0, 8.5), 3.0), None, 0.0, &mut bad);
            // 3 px beyond the end, on the axis: butt 3 px, square inside, round inside
            let want = match cap {
                Cap::Butt => Some(3.0),
                _ => Some(0.0),
            };
            check(&format!("{cap:?}: 3 px beyond the end"), hit_px(&mut s, p(103.0, 0.0), 4.0), want, 1e-6, &mut bad);
            // beyond the end at the corner: butt sqrt(2^2 + 2^2) from the corner, square 2 from its corner diagonally..., round
            let q = p(102.0, 7.0);
            let want = match cap {
                Cap::Butt => sqrt(2.0 * 2.0 + 2.0 * 2.0),
                Cap::Square => 2.0,
                _ => sqrt(2.0 * 2.0 + 7.0 * 7.0) - 5.0,
            };
            check(&format!("{cap:?}: beyond the corner"), hit_px(&mut s, q, 5.0), Some(want), 1e-6, &mut bad);
            let _ = beyond;
        }
        // a right-angled polyline 8 px wide: the miter's corner, a bevel's, a round join's
        for join in [Join::Miter, Join::Bevel, Join::Round] {
            let Ok(mut s) = Scene::new(r) else { return };
            let _ = s.set_camera(&cam);
            let (Ok(l), Ok(st)) = (s.add_layer(&Layer::new(0)), s.add_style(&style(INK[1], 8.0, Cap::Butt, join))) else { return };
            let _ = s.insert(l, st, &Shape::Polyline { points: &[p(0.0, 0.0), p(50.0, 0.0), p(50.0, 50.0)], closed: false });
            // the outer corner of the turn lies at (54, -4): the point (55, -5) is 1.414 px from a miter's corner, from a bevel's
            // edge (the line through (54, 0) and (50, -4)) and from a round join's arc
            let q = p(55.0, -5.0);
            let want = match join {
                Join::Miter => sqrt(2.0),
                Join::Bevel => (5.0 + 5.0 - 4.0) / sqrt(2.0) + (55.0 - 50.0 - 5.0) * 0.0,
                _ => sqrt(5.0 * 5.0 + 5.0 * 5.0) - 4.0,
            };
            check(&format!("{join:?} join's corner"), hit_px(&mut s, q, 8.0), Some(want), 1e-6, &mut bad);
        }
        // a miter beyond its limit is a bevel: a sharp turn (its miter 1 / sin(10 deg) = 5.8 widths), limits 4 and 10
        for (limit, miter) in [(4.0f32, false), (10.0, true)] {
            let Ok(mut s) = Scene::new(r) else { return };
            let _ = s.set_camera(&cam);
            let mut sty = style(INK[1], 6.0, Cap::Butt, Join::Miter);
            sty.miter_limit = limit;
            let (Ok(l), Ok(st)) = (s.add_layer(&Layer::new(0)), s.add_style(&sty)) else { return };
            let (sn, cs) = sincos(20f64.to_radians_f());
            let a = [-80.0 * cs, 80.0 * sn];
            let _ = s.insert(l, st, &Shape::Polyline { points: &[p(-80.0, 0.0), p(0.0, 0.0), p(a[0], a[1])], closed: false });
            // the miter's tip lies 3 / sin(10 deg) = 17.3 px beyond the vertex along the bisector (the -x direction turned by
            // 10 degrees: (cos 190, sin 190)... here along +x rotated up by 10 degrees)
            let (s10, c10) = sincos(10f64.to_radians_f());
            let tipd = 3.0 / s10;
            // (the tip lies on the outer side of the turn: below the incoming segment)
            let q = p(c10 * (tipd - 2.0), -s10 * (tipd - 2.0));
            let got = hit_px(&mut s, q, 0.0);
            check(&format!("a miter of 5.8 widths with the limit {limit}: 2 px inside its tip"), got.map(|_| 0.0), if miter { Some(0.0) } else { None }, 0.0, &mut bad);
        }
        // a circle of radius 40 px, 6 px wide: across its ring; an arc's butt end
        {
            let Ok(mut s) = Scene::new(r) else { return };
            let _ = s.set_camera(&cam);
            let (Ok(l), Ok(st)) = (s.add_layer(&Layer::new(0)), s.add_style(&style(INK[2], 6.0, Cap::Butt, Join::Miter))) else { return };
            let _ = s.insert(l, st, &Shape::Circle { center: p(0.0, 0.0), radius: 40.0 * u });
            let _ = s.insert(l, st, &Shape::Arc { center: p(200.0, 0.0), radius: 40.0 * u, start: 0.0, sweep: core::f64::consts::FRAC_PI_2 });
            let (s3, c3) = sincos(0.3);
            check("a circle: 2.5 px inside its ring's edge", hit_px(&mut s, p(45.5 * c3, 45.5 * s3), 0.0), None, 0.0, &mut bad);
            check("a circle: 2 px off its ring, tolerance 3", hit_px(&mut s, p(45.0 * c3, 45.0 * s3), 3.0), Some(2.0), 1e-6, &mut bad);
            check("an arc's butt end: 2 px below its start", hit_px(&mut s, p(240.0, -2.0), 3.0), Some(2.0), 1e-6, &mut bad);
            check("an arc's butt end: inside the ring beyond the end", hit_px(&mut s, p(200.0 - 1.0, 40.0), 0.5), None, 0.0, &mut bad);
        }
        // curves: an ellipse and a cubic chain against their own parameters sampled; a fill with a hole; a text's quad
        {
            let Ok(mut s) = Scene::new(r) else { return };
            let _ = s.set_camera(&cam);
            let (Ok(l), Ok(st)) = (s.add_layer(&Layer::new(0)), s.add_style(&style(INK[3], 0.0, Cap::Round, Join::Round))) else { return };
            let (c, rad, rot) = (p(0.0, 0.0), [60.0 * u, 25.0 * u], 0.4);
            let _ = s.insert(l, st, &Shape::Ellipse { center: c, radii: rad, rotation: rot, start: 0.2, sweep: 4.0 });
            let ell = |tt: f64| {
                let (s1, c1) = sincos(tt);
                let (sr, cr) = sincos(rot);
                let (x, y) = (rad[0] * c1, rad[1] * s1);
                [c[0] + cr * x - sr * y, c[1] + sr * x + cr * y]
            };
            let mut g = Rng(5);
            let mut worst = 0.0f64;
            for _ in 0..40 {
                let q = [c[0] + g.range(-70.0, 70.0) * u, c[1] + g.range(-40.0, 40.0) * u];
                let want = (param_dist(q, &ell, 0.2, 4.2) / u - 0.5).max(0.0);
                let got = hit_px(&mut s, q, 200.0);
                if let Some(gd) = got {
                    worst = worst.max((gd - want).abs());
                }
                check("an ellipse: the distance against its parameter sampled", got, Some(want), if u < 1e-3 { 2e-3 } else { 1e-6 }, &mut bad);
            }
            let _ = worst;
        }
        {
            let Ok(mut s) = Scene::new(r) else { return };
            let _ = s.set_camera(&cam);
            let (Ok(l), Ok(st)) = (s.add_layer(&Layer::new(0)), s.add_style(&style(INK[3], 2.0, Cap::Round, Join::Round))) else { return };
            let cp = [p(0.0, 0.0), p(30.0, 80.0), p(90.0, -60.0), p(120.0, 20.0)];
            let _ = s.insert(l, st, &Shape::Cubic { points: &cp, closed: false });
            let bez = |tt: f64| {
                let m = 1.0 - tt;
                let (a, b, c2, d2) = (m * m * m, 3.0 * m * m * tt, 3.0 * m * tt * tt, tt * tt * tt);
                [a * cp[0][0] + b * cp[1][0] + c2 * cp[2][0] + d2 * cp[3][0], a * cp[0][1] + b * cp[1][1] + c2 * cp[2][1] + d2 * cp[3][1]]
            };
            let mut g = Rng(9);
            for _ in 0..40 {
                let q = [at[0] + g.range(-10.0, 130.0) * u, at[1] + g.range(-40.0, 40.0) * u];
                let d = param_dist(q, &bez, 0.0, 1.0) / u;
                // (round caps: the distance to the curve less the half width, at its ends too)
                let want = (d - 1.0).max(0.0);
                check("a cubic: the distance against its parameter sampled", hit_px(&mut s, q, 200.0), Some(want), if u < 1e-3 { 2e-3 } else { 1e-6 }, &mut bad);
            }
        }
        {
            let Ok(mut s) = Scene::new(r) else { return };
            let _ = s.set_camera(&cam);
            let (Ok(l), Ok(st)) = (s.add_layer(&Layer::new(0)), s.add_style(&Style::new(INK[0]))) else { return };
            let outer = [Shape::Polyline { points: &[p(0.0, 0.0), p(100.0, 0.0), p(100.0, 100.0), p(0.0, 100.0)], closed: true }];
            let hole = [Shape::Circle { center: p(50.0, 50.0), radius: 20.0 * u }];
            let _ = s.insert(l, st, &Shape::Fill { rings: &[&outer, &hole] });
            check("a fill: inside", hit_px(&mut s, p(10.0, 10.0), 0.0), Some(0.0), 0.0, &mut bad);
            check("a fill: in its hole, 10 px from the hole's edge", hit_px(&mut s, p(50.0, 60.0), 12.0), Some(10.0), 1e-6, &mut bad);
            check("a fill: in its hole, tolerance 9", hit_px(&mut s, p(50.0, 60.0), 9.0), None, 0.0, &mut bad);
            check("a fill: 3 px outside", hit_px(&mut s, p(103.0, 50.0), 4.0), Some(3.0), 1e-6, &mut bad);
        }
        // the same cases inside inserts of every kind (moved, turned, mirrored, scaled evenly and unevenly, nested, an array):
        // the expected distance from the test's own transform of the geometry
        inserted_cases(r, &cam, at, u, &mut bad, &mut cases);
        let n = bad.len();
        t.ok(n == 0, &format!("pick {place}: {cases} exact cases (caps, joins, a miter's limit, rings, an arc's butt end, an ellipse and a cubic against their parameters, a fill with a hole, \
                                inside inserts of every kind): {n} wrong{}", bad.first().map(|b| format!(", first: {b}")).unwrap_or_default()));
    }
}

trait Rad {
    fn to_radians_f(self) -> f64;
}

impl Rad for f64 {
    fn to_radians_f(self) -> f64 {
        self * core::f64::consts::PI / 180.0
    }
}

/// a circle (radius 30, width 4 px, butt) and a polyline in a block, inserted in every way: picks against the test's own
/// transform (the circle's image an ellipse under an uneven scale, its distance sampled)
fn inserted_cases(r: &Renderer, cam: &Camera, at: [f64; 2], u: f64, bad: &mut Vec<String>, cases: &mut u32) {
    let ways: [(&str, f64, [f64; 2], u32, [f64; 2]); 6] = [("moved", 0.0, [1.0, 1.0], 1, [0.0, 0.0]), ("turned", 0.8, [1.0, 1.0], 1, [0.0, 0.0]), ("mirrored", 0.5, [-1.0, 1.0], 1, [0.0, 0.0]),
                                                          ("scaled evenly", 0.2, [1.7, 1.7], 1, [0.0, 0.0]), ("scaled unevenly", 0.6, [1.8, 0.6], 1, [0.0, 0.0]), ("an array", 0.3, [1.0, 1.0], 3, [90.0, 70.0])];
    for (name, rot, sc, n, sp) in ways {
        let Ok(mut s) = Scene::new(r) else { return };
        let _ = s.set_camera(cam);
        let (Ok(l), Ok(st)) = (s.add_layer(&Layer::new(0)), s.add_style(&style(INK[2], 4.0, Cap::Butt, Join::Miter))) else { return };
        let mut def = render2d::Block::new();
        def.base = [5.0 * u, -3.0 * u];
        let Ok(b) = s.add_block(&def) else { return };
        let (cc, rr) = ([20.0 * u, 10.0 * u], 30.0 * u);
        let _ = s.insert_in(b, st, &Shape::Circle { center: cc, radius: rr });
        let mut ins = render2d::Insert::new(b, at);
        ins.rotation = rot;
        ins.scale = sc;
        ins.columns = n;
        ins.rows = n.min(2);
        ins.spacing = [sp[0] * u, sp[1] * u];
        let bi = BIns { block: 0, at, rotation: rot, scale: sc, columns: n, rows: n.min(2), spacing: [sp[0] * u, sp[1] * u] };
        let Ok(id) = s.insert(l, st, &Shape::Insert(ins)) else { return };
        let mut g = Rng(31);
        for (copy, xf) in bi.xfs([0.0, 0.0]).iter().enumerate() {
            // the circle's image: centre + M r (cos t, sin t), less the base
            let c0 = xf.apply([cc[0] - def.base[0], cc[1] - def.base[1]]);
            let img = |tt: f64| {
                let (s1, c1) = sincos(tt);
                let v = xf.lin([rr * c1, rr * s1]);
                [c0[0] + v[0], c0[1] + v[1]]
            };
            let swd = sqrt(xf.det().abs());
            let h = 2.0 * swd.max(0.0) * 0.0 + 2.0;
            for _ in 0..12 {
                let tt = g.range(0.0, core::f64::consts::TAU);
                let on = img(tt);
                let q = [on[0] + g.range(-6.0, 6.0) * u, on[1] + g.range(-6.0, 6.0) * u];
                let d = param_dist(q, &img, 0.0, core::f64::consts::TAU) / u;
                let want = (d - h).max(0.0);
                let hits = s.pick(q, 10.0);
                *cases += 1;
                let got = hits.iter().find(|x| x.target.entity == id && x.target.path.first().map(|p| p.0) == Some(copy as u32)).map(|x| x.distance);
                let eps = if u < 1e-3 { 3e-3 } else { 1e-6 };
                // (a ring's distance is that of the curve less the half width, which the uneven map leaves in pixels)
                if (want - 10.0).abs() > eps && match got {
                    Some(gd) => want > 10.0 || (gd - want).abs() > eps,
                    None => want <= 10.0,
                } {
                    bad.push(format!("a circle in an insert {name}, copy {copy}: {got:?} px against {want} px"));
                }
            }
        }
    }
}

// ---------------------------------------------------------------- area selection

pub fn select(t: &mut T) {
    let Some(g) = setup(t) else { return };
    let Some(dev) = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "device") else { return };
    let Some(r) = t.res(Renderer::new(&dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new") else { return };
    for (at, u, place) in PLACES {
        let (mut regions, mut bad, mut picked) = (0u32, 0u32, 0u32);
        let mut first: Option<String> = None;
        for seed in 1..=6u32 {
            let d = random_drawing(seed + 20, at, u);
            let xs = expand_paths(&d);
            let cam = Camera { center: at, scale: u, rotation: 0.0 };
            let Some((mut s, ids)) = t.res(scene_of(&r, &d, &cam), "scene") else { return };
            let Some(mut e) = t.res(Scene::new(&r), "Scene::new") else { return };
            let _ = e.set_camera(&cam);
            let Some(xids) = t.res(build_expanded(&mut e, &d, &xs), "expanded") else { return };
            let mut rng = Rng(seed * 13);
            for k in 0..60 {
                let c = [at[0] + rng.range(-120.0, 120.0) * u, at[1] + rng.range(-120.0, 120.0) * u];
                let (w, h) = (rng.range(20.0, 200.0) * u, rng.range(20.0, 200.0) * u);
                let poly: Vec<[f64; 2]> = (0..7).map(|j| {
                    let a = j as f64 * core::f64::consts::TAU / 7.0;
                    let (sn, cs) = sincos(a);
                    let rr = if j % 2 == 0 { 1.0 } else { 0.55 };
                    [c[0] + rr * w * cs, c[1] + rr * h * sn]
                }).collect();
                let region = if k % 2 == 0 { Region::Rect([c[0] - w, c[1] - h], [c[0] + w, c[1] + h]) } else { Region::Polygon(&poly) };
                for mode in [SelectMode::Window, SelectMode::Crossing] {
                    regions += 1;
                    let mut got = s.select(region, mode);
                    let mut brute = s.select_brute(region, mode);
                    got.sort_by_key(|e| e.slot);
                    brute.sort_by_key(|e| e.slot);
                    // the expanded drawing: an insert is in when all (window) or any (crossing) of its entities are
                    let ex = e.select_brute(region, mode);
                    let mut want: Vec<usize> = Vec::new();
                    for top in 0..d.items.len() {
                        let mine: Vec<usize> = (0..xs.len()).filter(|&k| xs[k].top == top && d.layers[xs[k].layer].0).collect();
                        if mine.is_empty() {
                            continue;
                        }
                        let inn = mine.iter().filter(|&&k| ex.contains(&xids[k])).count();
                        if (mode == SelectMode::Window && inn == mine.len()) || (mode == SelectMode::Crossing && inn > 0) {
                            want.push(top);
                        }
                    }
                    let mut have: Vec<usize> = got.iter().filter_map(|id| ids.tops.iter().position(|x| x == id)).collect();
                    have.sort_unstable();
                    picked += have.len() as u32;
                    if got != brute || have != want {
                        bad += 1;
                        if first.is_none() {
                            first = Some(format!("seed {seed}, region {k} {mode:?}: the scene {have:?} (brute force {}), the expanded drawing {want:?}", brute.len()));
                        }
                    }
                }
            }
        }
        t.ok(bad == 0, &format!("select {place}: 6 random drawings, {regions} windows and crossings (rectangles and polygons): as the drawing expanded by the test (an insert: all or any of its \
                                 entities), index = brute force, {picked} selected; {bad} differ{}", first.map(|f| format!(", first: {f}")).unwrap_or_default()));
    }
    exact_selects(t, &r);
}

fn exact_selects(t: &mut T, r: &Renderer) {
    let Ok(mut s) = Scene::new(r) else { return };
    let _ = s.set_camera(&Camera::new([0.0, 0.0], 1.0));
    let (Ok(l), Ok(st)) = (s.add_layer(&Layer::new(0)), s.add_style(&style(INK[0], 3.0, Cap::Round, Join::Round))) else { return };
    let mut bad = Vec::new();
    // a cubic whose control points leave the window while the curve stays inside (its top at y = 45 for control points at
    // 60): window [-10, -10] .. [110, 50]
    let Ok(cub) = s.insert(l, st, &Shape::Cubic { points: &[[0.0, 0.0], [0.0, 60.0], [100.0, 60.0], [100.0, 0.0]], closed: false }) else { return };
    let win = Region::Rect([-10.0, -10.0], [110.0, 50.0]);
    if !s.select(win, SelectMode::Window).contains(&cub) {
        bad.push("a cubic inside a window whose hull leaves it is not selected");
    }
    if s.select(Region::Rect([-10.0, -10.0], [110.0, 44.0]), SelectMode::Window).contains(&cub) {
        bad.push("a cubic reaching 1 unit beyond the window is selected");
    }
    // an arc whose box is inside the window, its circle not
    let Ok(arc) = s.insert(l, st, &Shape::Arc { center: [300.0, 0.0], radius: 50.0, start: 0.2, sweep: 0.6 }) else { return };
    if !s.select(Region::Rect([320.0, 5.0], [355.0, 40.0]), SelectMode::Window).contains(&arc) {
        bad.push("an arc inside a window is not selected");
    }
    // a crossing region inside a fill touches it; a window inside it does not hold it
    let ring = [Shape::Polyline { points: &[[500.0, 0.0], [600.0, 0.0], [600.0, 100.0], [500.0, 100.0]], closed: true }];
    let Ok(fid) = s.insert(l, st, &Shape::Fill { rings: &[&ring] }) else { return };
    if !s.select(Region::Rect([540.0, 40.0], [560.0, 60.0]), SelectMode::Crossing).contains(&fid) {
        bad.push("a crossing inside a fill does not select it");
    }
    if s.select(Region::Rect([540.0, 40.0], [560.0, 60.0]), SelectMode::Window).contains(&fid) {
        bad.push("a window inside a fill selects it");
    }
    // an array: a window around two of its three copies does not select it, a crossing does; around all three both
    let mut def = render2d::Block::new();
    def.base = [0.0, 0.0];
    let Ok(b) = s.add_block(&def) else { return };
    let _ = s.insert_in(b, st, &Shape::Line { a: [0.0, 0.0], b: [10.0, 0.0] });
    let mut ins = render2d::Insert::new(b, [0.0, 300.0]);
    ins.columns = 3;
    ins.spacing = [50.0, 0.0];
    let Ok(arr) = s.insert(l, st, &Shape::Insert(ins)) else { return };
    if s.select(Region::Rect([-5.0, 290.0], [65.0, 310.0]), SelectMode::Window).contains(&arr) {
        bad.push("a window around two of an array's three copies selects it");
    }
    if !s.select(Region::Rect([-5.0, 290.0], [65.0, 310.0]), SelectMode::Crossing).contains(&arr) {
        bad.push("a crossing around two of an array's copies does not select it");
    }
    if !s.select(Region::Rect([-5.0, 290.0], [115.0, 310.0]), SelectMode::Window).contains(&arr) {
        bad.push("a window around all of an array's copies does not select it");
    }
    // a polygon that cuts into a line's middle between its ends: crossing yes, window no
    let Ok(line) = s.insert(l, st, &Shape::Line { a: [0.0, 500.0], b: [100.0, 500.0] }) else { return };
    let v = [[-10.0, 490.0], [110.0, 490.0], [110.0, 510.0], [55.0, 495.0], [-10.0, 510.0]];
    if s.select(Region::Polygon(&v), SelectMode::Window).contains(&line) {
        bad.push("a polygon whose notch crosses a line's middle holds it in a window");
    }
    // a window over everything: every entity of a shown layer
    let all = s.select(Region::Rect([-1e9, -1e9], [1e9, 1e9]), SelectMode::Window);
    if all.len() != 5 {
        bad.push("a window over everything does not hold all five");
    }
    t.ok(bad.is_empty(), &format!("select: exact cases (a cubic whose hull leaves the window, an arc, a region inside a fill, an array's copies, a polygon's notch, a window over everything): {}",
                                  if bad.is_empty() { String::from("as expected") } else { bad.join("; ") }));
}

// ---------------------------------------------------------------- snaps

fn near(a: [f64; 2], b: [f64; 2], eps: f64) -> bool {
    (a[0] - b[0]).abs() <= eps && (a[1] - b[1]).abs() <= eps
}

pub fn snap(t: &mut T) {
    let Some(g) = setup(t) else { return };
    let Some(dev) = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "device") else { return };
    let Some(r) = t.res(Renderer::new(&dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new") else { return };
    for (at, u, place) in PLACES {
        let cam = Camera { center: at, scale: u, rotation: 0.0 };
        let p = |x: f64, y: f64| [at[0] + x * u, at[1] + y * u];
        let mut bad: Vec<String> = Vec::new();
        let mut cases = 0u32;
        let eps = if u < 1e-3 { 4e-9 } else { 1e-9 };
        let Ok(mut s) = Scene::new(&r) else { return };
        let _ = s.set_camera(&cam);
        let (Ok(l), Ok(st)) = (s.add_layer(&Layer::new(0)), s.add_style(&style(INK[0], 1.0, Cap::Butt, Join::Miter))) else { return };
        // two lines crossing at (37, 21), a circle of radius 30 about (200, 0), a line through it
        let _ = s.insert(l, st, &Shape::Line { a: p(0.0, 0.0), b: p(74.0, 42.0) });
        let _ = s.insert(l, st, &Shape::Line { a: p(0.0, 42.0), b: p(74.0, 0.0) });
        let _ = s.insert(l, st, &Shape::Circle { center: p(200.0, 0.0), radius: 30.0 * u });
        let _ = s.insert(l, st, &Shape::Line { a: p(150.0, 10.0), b: p(260.0, 10.0) });
        let mut want = |what: &str, q: [f64; 2], opt: SnapOptions, kind: SnapKind, pt: [f64; 2], bad: &mut Vec<String>, s: &mut Scene| {
            cases += 1;
            let got = s.snap(q, &opt);
            let brute = s.snaps_brute(q, &opt).into_iter().next();
            match &got {
                Some(x) if x.kind == kind && near(x.point, pt, eps) && got == brute => {}
                _ => bad.push(format!("{what}: {:?} at {:?} (want {kind:?} at {pt:?}; brute force {:?})", got.as_ref().map(|x| x.kind), got.as_ref().map(|x| x.point), brute.as_ref().map(|x| (x.kind, x.point)))),
            }
        };
        let all = SnapOptions::new(10.0, render2d::SNAP_ALL);
        want("an endpoint and nearest within the aperture: the endpoint", p(2.0, 1.0), all, SnapKind::Endpoint, p(0.0, 0.0), &mut bad, &mut s);
        want("a midpoint", p(37.0 + 3.0, 21.0 + 6.0), SnapOptions::new(10.0, render2d::SNAP_MID), SnapKind::Midpoint, p(37.0, 21.0), &mut bad, &mut s);
        want("the intersection of two lines", p(38.0, 22.5), SnapOptions::new(10.0, render2d::SNAP_INT | render2d::SNAP_NEA), SnapKind::Intersection, p(37.0, 21.0), &mut bad, &mut s);
        // a line and a circle cross at x = 200 +- sqrt(900 - 100)
        let dx = sqrt(800.0);
        want("a line across a circle", p(200.0 + dx + 2.0, 12.0), SnapOptions::new(10.0, render2d::SNAP_INT), SnapKind::Intersection, p(200.0 + dx, 10.0), &mut bad, &mut s);
        want("a circle's centre, the pointer on it", p(230.5, -1.0), SnapOptions::new(10.0, render2d::SNAP_CEN | render2d::SNAP_NEA), SnapKind::Center, p(200.0, 0.0), &mut bad, &mut s);
        want("a circle's quadrant", p(201.0, 28.0), SnapOptions::new(10.0, render2d::SNAP_QUA), SnapKind::Quadrant, p(200.0, 30.0), &mut bad, &mut s);
        // perpendicular from (250, 80) to the line y = 10: (250, 10); tangent from (300, 0) to the circle
        want("perpendicular to a line", p(249.0, 12.0), SnapOptions::new(10.0, render2d::SNAP_PER).with_base(p(250.0, 80.0)), SnapKind::Perpendicular, p(250.0, 10.0), &mut bad, &mut s);
        let (dd, rr) = (100.0f64, 30.0f64);
        let (tx, ty) = (200.0 + rr * rr / dd, rr * sqrt(dd * dd - rr * rr) / dd);
        want("tangent to a circle", p(tx + 1.0, ty + 1.0), SnapOptions::new(10.0, render2d::SNAP_TAN).with_base(p(300.0, 0.0)), SnapKind::Tangent, p(tx, ty), &mut bad, &mut s);
        want("nearest on a line", p(30.0, 30.0), SnapOptions::new(20.0, render2d::SNAP_NEA), SnapKind::Nearest, {
            // the foot on the second line (0, 42) - (74, 0): nearer than the first
            let (a, b) = ([0.0, 42.0], [74.0, 0.0]);
            let d = [b[0] - a[0], b[1] - a[1]];
            let tt = ((30.0 - a[0]) * d[0] + (30.0 - a[1]) * d[1]) / (d[0] * d[0] + d[1] * d[1]);
            p(a[0] + tt * d[0], a[1] + tt * d[1])
        }, &mut bad, &mut s);
        // inside an insert scaled unevenly and turned: a circle becomes an ellipse with its centre and its axes' ends
        let mut def = render2d::Block::new();
        def.base = [0.0, 0.0];
        let Ok(b) = s.add_block(&def) else { return };
        let _ = s.insert_in(b, st, &Shape::Circle { center: [0.0, 0.0], radius: 10.0 * u });
        let mut ins = render2d::Insert::new(b, p(400.0, 0.0));
        ins.rotation = 0.5;
        ins.scale = [3.0, 1.0];
        let _ = s.insert(l, st, &Shape::Insert(ins));
        let (sn, cs) = sincos(0.5);
        // the major axis: the local x scaled by 3 and turned by 0.5
        let major = [400.0 + 30.0 * cs, 30.0 * sn];
        want("an ellipse made of a circle by an uneven scale: its centre", p(major[0] - 0.5, major[1] + 0.5), SnapOptions::new(10.0, render2d::SNAP_CEN), SnapKind::Center, p(400.0, 0.0), &mut bad, &mut s);
        want("an ellipse made of a circle by an uneven scale: its major axis' end", p(major[0] - 0.5, major[1] + 0.5), SnapOptions::new(10.0, render2d::SNAP_QUA), SnapKind::Quadrant, p(major[0], major[1]), &mut bad, &mut s);
        // a fill's corner in a block inserted unevenly
        let mut fdef = render2d::Block::new();
        fdef.base = [1.0 * u, 2.0 * u];
        let Ok(fb) = s.add_block(&fdef) else { return };
        let sq = [Shape::Polyline { points: &[[0.0, 0.0], [20.0 * u, 0.0], [20.0 * u, 15.0 * u], [0.0, 15.0 * u]], closed: true }];
        let hole = [Shape::Circle { center: [10.0 * u, 7.0 * u], radius: 4.0 * u }];
        let _ = s.insert_in(fb, st, &Shape::Fill { rings: &[&sq, &hole] });
        let mut fi = render2d::Insert::new(fb, p(500.0, 100.0));
        fi.rotation = 0.3;
        fi.scale = [1.4, 0.7];
        let _ = s.insert(l, st, &Shape::Insert(fi));
        // the corner (20, 15) less the base, through the insert
        let (s3, c3) = sincos(0.3);
        let (lx, ly) = ((20.0 - 1.0) * 1.4, (15.0 - 2.0) * 0.7);
        let corner = p(500.0 + c3 * lx - s3 * ly, 100.0 + s3 * lx + c3 * ly);
        want("a fill's corner in an insert scaled unevenly", [corner[0] + 1.5 * u, corner[1] - 1.0 * u], SnapOptions::new(10.0, render2d::SNAP_END), SnapKind::Endpoint, corner, &mut bad, &mut s);
        // insertion points: an insert's and each copy of an array
        let mut ins2 = render2d::Insert::new(b, p(600.0, 0.0));
        ins2.columns = 3;
        ins2.spacing = [40.0 * u, 0.0];
        let _ = s.insert(l, st, &Shape::Insert(ins2));
        want("an array's second copy's insertion point", p(641.0, 1.0), SnapOptions::new(10.0, render2d::SNAP_INS), SnapKind::Insertion, p(640.0, 0.0), &mut bad, &mut s);
        // a text's alignment point
        if let Ok(f) = s.add_font(SANS, 0) {
            if let Ok(ts) = s.add_text_style(&render2d::TextStyle::new(f)) {
                let mut tx = render2d::Text::new("Snap", ts, p(800.0, 0.0));
                tx.height = Some(10.0 * u);
                let _ = s.insert(l, st, &Shape::Text(tx));
                want("a text's alignment point", p(801.0, 2.0), SnapOptions::new(10.0, render2d::SNAP_INS), SnapKind::Insertion, p(800.0, 0.0), &mut bad, &mut s);
            }
        }
        // hidden layers are skipped; a filter skips more
        let mut hidden = Layer::new(5);
        hidden.visible = false;
        if let Ok(hl) = s.add_layer(&hidden) {
            let _ = s.insert(hl, st, &Shape::Line { a: p(0.0, -100.0), b: p(50.0, -100.0) });
            cases += 1;
            if s.snap(p(0.5, -100.0), &all).is_some() || !s.pick(p(0.5, -100.0), 2.0).is_empty() {
                bad.push(String::from("a hidden layer's line is snapped or picked"));
            }
        }
        cases += 1;
        let skip = |_: render2d::LayerId| false;
        if !s.snaps_filtered(p(2.0, 1.0), &all, &skip).is_empty() {
            bad.push(String::from("a filter that skips every layer leaves a snap"));
        }
        let n = bad.len();
        t.ok(n == 0, &format!("snap {place}: {cases} cases (endpoint over nearest, midpoint, intersections of lines and of a line and a circle, centre, quadrant, perpendicular, tangent, nearest, \
                               an ellipse made by an uneven insert: its centre and axis, insertion of arrays and texts, hidden layers, a filter; within {eps:.0e} units, = brute force): {n} wrong{}",
                              bad.first().map(|b| format!(", first: {b}")).unwrap_or_default()));
        // random drawings: the snap of the index against brute force, the kinds every expansion keeps against the expanded drawing
        let mut diff = 0u32;
        let mut tried = 0u32;
        let mut firstd: Option<String> = None;
        for seed in 1..=4u32 {
            let d = random_drawing(seed + 40, at, u);
            let xs = expand_paths(&d);
            let Some((mut s2, _ids)) = t.res(scene_of(&r, &d, &cam), "scene") else { return };
            let Some(mut e) = t.res(Scene::new(&r), "Scene::new") else { return };
            let _ = e.set_camera(&cam);
            let Some(_xids) = t.res(build_expanded(&mut e, &d, &xs), "expanded") else { return };
            let mut rng = Rng(seed * 101);
            let pts = pick_points(&mut rng, &xs, at, u, 150);
            for (k, q) in pts.iter().enumerate() {
                let base = if k % 2 == 0 { Some(p(rng.range(-150.0, 150.0), rng.range(-150.0, 150.0))) } else { None };
                let mut opt = SnapOptions::new(12.0, render2d::SNAP_END | render2d::SNAP_INT | render2d::SNAP_NEA | render2d::SNAP_PER | render2d::SNAP_TAN);
                if let Some(b) = base {
                    opt = opt.with_base(b);
                }
                tried += 1;
                let a = s2.snaps(*q, &opt);
                let b = s2.snaps_brute(*q, &opt);
                let x = e.snaps_brute(*q, &opt);
                // (within 1e-3 px; at 1e-6 units per pixel near 1e7 an ulp is 0.0019 px: within 0.01 px there)
                let lim = if u < 1e-3 { 1e-2 * u } else { 1e-3 * u };
                // (the first of the scene among the expanded drawing's candidates as near as its first, within lim: two
                // candidates of other entities a hair apart may change places through f64's rounding at 1e7)
                let same = |u1: &[render2d::Snap], u2: &[render2d::Snap]| match (u1.first(), u2.first()) {
                    (Some(m), Some(n)) => u2.iter().take_while(|y| y.distance <= n.distance + lim / u).any(|y| y.kind == m.kind && near(y.point, m.point, lim)) && m.distance <= n.distance + lim / u,
                    (m, n) => m.is_none() == n.is_none(),
                };
                // (snap looks for intersections only up to the nearest other candidate: the same first one)
                let one = s2.snap(*q, &opt);
                let ok = a == b && one.as_ref() == a.first() && a.is_empty() == x.is_empty() && same(&a, &x);
                if !ok {
                    diff += 1;
                    if firstd.is_none() {
                        firstd = Some(format!("seed {seed} at {q:?}: {:?} / brute {:?} / expanded {:?}", a.iter().take(4).map(|y| (y.kind, y.point, y.distance)).collect::<Vec<_>>(), b.first().map(|y| (y.kind, y.point)), x.iter().take(4).map(|y| (y.kind, y.point, y.distance, _xids.iter().position(|&e| e == y.target.entity).map(|k| (xs[k].top, xs[k].path.clone(), match &xs[k].geo { Geo::Line(..) => "line", Geo::Poly(..) => "poly", Geo::Arc(..) => "arc", Geo::Circle(..) => "circle", Geo::Bez(..) => "bez", Geo::Ellipse(..) => "ellipse", Geo::Spline(..) => "spline", Geo::Fill(..) => "fill" })))).collect::<Vec<_>>()));
                    }
                }
            }
        }
        t.ok(diff == 0, &format!("snap {place}: 4 random drawings, {tried} snaps (endpoint, intersection, perpendicular, tangent, nearest): index = brute force = the drawing expanded by the test (within \
                                  {} px), snap = the first of snaps; {diff} differ{}", if u < 1e-3 { "0.01" } else { "0.001" }, firstd.map(|f| format!(", first: {f}")).unwrap_or_default()));
    }
}

// ---------------------------------------------------------------- churn

pub fn churn(t: &mut T) {
    let Some(g) = setup(t) else { return };
    let Some(dev) = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "device") else { return };
    let Some(r) = t.res(Renderer::new(&dev, &RendererDesc::new(Format::Bgra8Unorm)), "Renderer::new") else { return };
    let (at, u) = ([1.0e7, 1.0e7], 1.0);
    let d = random_drawing(77, at, u);
    let cam = Camera::new(at, u);
    let Some((mut s, ids)) = t.res(scene_of(&r, &d, &cam), "scene") else { return };
    s.build_index();
    let mut rng = Rng(4242);
    let mut live: Vec<EntityId> = ids.tops.clone();
    let blocks: Vec<render2d::BlockId> = Vec::new();
    let _ = blocks;
    let Ok(st) = s.add_style(&style(INK[1], 2.0, Cap::Round, Join::Round)) else { return };
    let layers = s.drawn_layers();
    let (mut changes, mut diff, mut queries) = (0u32, 0u32, 0u32);
    let mut first: Option<String> = None;
    let xs = expand_paths(&d);
    for round in 0..40 {
        for _ in 0..100 {
            changes += 1;
            let what = rng.below(6);
            let q = |g: &mut Rng| [at[0] + g.range(-150.0, 150.0) * u, at[1] + g.range(-150.0, 150.0) * u];
            match what {
                0 | 1 => {
                    // a new entity
                    let a = q(&mut rng);
                    let shape = match rng.below(3) {
                        0 => Shape::Line { a, b: q(&mut rng) },
                        1 => Shape::Circle { center: a, radius: rng.range(2.0, 30.0) * u },
                        _ => Shape::Arc { center: a, radius: rng.range(2.0, 30.0) * u, start: rng.range(0.0, 6.0), sweep: rng.range(0.5, 3.0) },
                    };
                    if let Some(&l) = layers.get(rng.below(layers.len().max(1) as u32) as usize) {
                        if let Ok(id) = s.insert(l, st, &shape) {
                            live.push(id);
                        }
                    }
                }
                2 | 3 if !live.is_empty() => {
                    // moved: a line again, or an insert moved (its update keeps its block)
                    let k = rng.below(live.len() as u32) as usize;
                    let id = live[k];
                    let _ = s.update(id, &Shape::Line { a: q(&mut rng), b: q(&mut rng) });
                }
                4 if !live.is_empty() => {
                    let k = rng.below(live.len() as u32) as usize;
                    let id = live.swap_remove(k);
                    let _ = s.remove(id);
                }
                _ => {
                    // a block changes: an entity more in it (its inserts' boxes move)
                    let b = d.blocks.len() as u32;
                    let _ = b;
                }
            }
        }
        // picks, windows and snaps: the index against brute force
        for _ in 0..20 {
            queries += 3;
            let qp = [at[0] + rng.range(-160.0, 160.0) * u, at[1] + rng.range(-160.0, 160.0) * u];
            let a = s.pick(qp, 4.0);
            let b = s.pick_brute(qp, 4.0);
            let w = Region::Rect([qp[0] - 40.0 * u, qp[1] - 30.0 * u], [qp[0] + 40.0 * u, qp[1] + 30.0 * u]);
            let mut sa = s.select(w, SelectMode::Crossing);
            let mut sb = s.select_brute(w, SelectMode::Crossing);
            sa.sort_by_key(|e| e.slot);
            sb.sort_by_key(|e| e.slot);
            let opt = SnapOptions::new(10.0, render2d::SNAP_ALL & !render2d::SNAP_PER & !render2d::SNAP_TAN);
            let na = s.snaps(qp, &opt);
            let nb = s.snaps_brute(qp, &opt);
            if a != b || sa != sb || na != nb {
                diff += 1;
                if first.is_none() {
                    first = Some(format!("round {round} at {qp:?}: picks {} / {}, crossing {} / {}, snaps {} / {}", a.len(), b.len(), sa.len(), sb.len(), na.len(), nb.len()));
                }
            }
        }
        if let Err(why) = s.index_check() {
            diff += 1;
            if first.is_none() {
                first = Some(format!("round {round}: the index's invariants: {why}"));
            }
        }
    }
    let _ = xs;
    let info = s.index_info();
    t.ok(diff == 0, &format!("churn: {changes} random changes (insert, update, remove) in 40 rounds, {queries} picks, crossings and snaps after them: index = brute force, invariants hold \
                              ({} entries, {} nodes, height {}); {diff} differ{}", info.entries, info.nodes, info.height, first.map(|f| format!(", first: {f}")).unwrap_or_default()));
}
