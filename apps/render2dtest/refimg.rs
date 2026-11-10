// refimg -- the reference on the CPU: the same drawing through lib/gfx (dash_stroke's outline, filled non-zero with
// its exact-area coverage, blended in sRGB codes), and how two images are compared.
//
// What may differ, and why (docs/RENDER2D.md "Tests"):
//   - both sides compute the exact area of a pixel inside a polygon (render2d: the box filter of an edge, the square
//     clipped at corners; lib/gfx: signed areas); lib/gfx keeps it as a byte and blends in integers, the GPU in
//     floats: 1 to 2 codes (Tol::STRAIGHT)
//   - a circle: the GPU cuts it by its tangent at the pixel, lib/gfx draws a polygon whose edges lie up to 0.05 px
//     inside it (dash.rs: arc steps of 2 sqrt(0.1 / hw) rad, a sagitta of hw (1 - cos(sqrt(0.1 / hw))) ~ 0.05 px):
//     up to 13 codes along every arc, more at a circle of half a pixel's radius (a hairline's round cap) (Tol::round)
//   - inner joins: lib/gfx's outline pivots through the vertex and its non-zero fill sums the windings of a pixel
//     before it clamps, so where the outline overlaps itself a pixel's share can come out too high, up to full; the
//     GPU computes the union (checked pixel by pixel against the exact share of a supersampled union: GPU 0.6026,
//     exact 0.6025, lib/gfx 1.0) (Tol::corners); a polyline that folds back over itself does the same in many places,
//     its pieces that meet away from a join blend twice on the GPU, once in lib/gfx's fill, and the GPU's union of a
//     short segment between two sharp turns is an approximation (docs/RENDER2D.md) (Tol::folded)
//   - so: almost every pixel within `small` codes, a bounded number beyond it, none beyond `max`, and the ink (the
//     sum of every pixel's distance from the background) within `ink` of the reference's

use alloc::vec::Vec;

use gfx::dash::{dash_stroke, DStyle};
use gfx::path::Path;
use gfx::raster::Raster;
use gfx::{Dc, Surface, FMT_XRGB, RULE_NONZERO};
use render2d::{Camera, Cap, Join, Style, Unit};

/// an entity of a test drawing
#[derive(Clone)]
pub enum Geo {
    Line([f64; 2], [f64; 2]),
    Poly(Vec<[f64; 2]>, bool),
}

#[derive(Clone)]
pub struct Item {
    pub layer: usize,
    pub style: Style,
    pub geo: Geo,
}

/// layers (visible, order) and the entities in the order of insertion
#[derive(Clone, Default)]
pub struct Drawing {
    pub layers: Vec<(bool, i32)>,
    pub items: Vec<Item>,
}

impl Drawing {
    pub fn one_layer() -> Drawing {
        Drawing { layers: alloc::vec![(true, 0)], items: Vec::new() }
    }

    pub fn add(&mut self, style: Style, geo: Geo) {
        self.items.push(Item { layer: 0, style, geo });
    }

    /// the items in draw order: visible layers by order (then as added), within a layer as inserted
    pub fn draw_order(&self) -> Vec<&Item> {
        let mut ls: Vec<usize> = (0..self.layers.len()).filter(|&l| self.layers[l].0).collect();
        ls.sort_by_key(|&l| (self.layers[l].1, l));
        let mut out = Vec::new();
        for l in ls {
            out.extend(self.items.iter().filter(|i| i.layer == l));
        }
        out
    }
}

/// a style's width in pixels as render2d draws it (at least one)
pub fn width_px(s: &Style, cam: &Camera) -> f64 {
    let w = match s.unit {
        Unit::Drawing => s.width as f64 / cam.scale,
        _ => s.width as f64,
    };
    w.max(1.0)
}

/// the drawing as lib/gfx draws it on a background (rows of B, G, R, x)
pub fn draw(d: &Drawing, cam: &Camera, size: (u32, u32), bg: u32) -> Vec<u8> {
    let (w, h) = (size.0 as i32, size.1 as i32);
    let mut sf = Surface::new(w, h, FMT_XRGB);
    let mut dc = Dc::new(w, h);
    dc.begin(&sf);
    dc.clear(&mut sf, bg);
    let mut ras = Raster::default();
    for it in d.draw_order() {
        let mut p = Path::default();
        let pt = |q: [f64; 2]| {
            let s = cam.to_screen(q, size);
            (s[0] as f32, s[1] as f32)
        };
        match &it.geo {
            Geo::Line(a, b) => {
                let (a, b) = (pt(*a), pt(*b));
                p.move_f(a.0, a.1);
                p.line_f(b.0, b.1);
            }
            Geo::Poly(v, closed) => {
                for (k, q) in v.iter().enumerate() {
                    let q = pt(*q);
                    if k == 0 {
                        p.move_f(q.0, q.1);
                    } else {
                        p.line_f(q.0, q.1);
                    }
                }
                if *closed {
                    p.close();
                }
            }
        }
        let s = &it.style;
        let cap = match s.cap {
            Cap::Round => gfx::dash::LCAP_ROUND,
            Cap::Square => gfx::dash::LCAP_SQUARE,
            _ => gfx::dash::LCAP_FLAT,
        };
        let join = match s.join {
            Join::Round => gfx::JOIN_ROUND,
            Join::Bevel => gfx::JOIN_BEVEL,
            _ => gfx::JOIN_MITER,
        };
        let st = DStyle { width: width_px(s, cam), pat: &[], join, cap0: cap, cap1: cap, dcap: cap, offset: 0.0, miter: s.miter_limit as f64 };
        if dash_stroke(&mut p, &st) != 0 {
            continue;
        }
        dc.set_color(s.color);
        p.fill(&mut ras, &dc, &mut sf, RULE_NONZERO);
    }
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        let at = y * sf.stride as usize;
        out.extend_from_slice(&sf.px[at..at + w as usize * 4]);
    }
    out
}

/// what may differ (see the file's head)
#[derive(Clone, Copy, Debug)]
pub struct Tol {
    pub small: u8,
    pub over: u32,
    pub max: u8,
    pub ink: f64,
}

impl Tol {
    /// straight edges and polygonal corners: what blending in integers against floats costs
    pub const STRAIGHT: Tol = Tol { small: 2, over: 8, max: 4, ink: 0.001 };

    /// round caps and joins (n of them): along the arcs 16 codes, beyond that at most 4 pixels each
    pub const fn round(n: u32) -> Tol {
        Tol { small: 16, over: 4 * n, max: 48, ink: 0.006 }
    }

    /// inner joins and tips (n of them): 24 pixels each beyond 2 codes, at most 96 codes (lib/gfx's winding sum)
    pub const fn corners(n: u32) -> Tol {
        Tol { small: 2, over: 24 * n, max: 96, ink: 0.005 }
    }

    /// random polylines (n entities) that fold back over themselves
    pub const fn folded(n: u32) -> Tol {
        Tol { small: 2, over: 30 + 24 * n, max: 128, ink: 0.01 }
    }
}

/// two images side by side
#[derive(Clone, Copy, Debug, Default)]
pub struct Cmp {
    /// pixels beyond `small` codes in a channel, the largest difference, where
    pub over: u32,
    pub max: u8,
    pub at: (u32, u32),
    /// the sum of every pixel's distance from the background (three channels), GPU and reference
    pub ink: [f64; 2],
}

impl Cmp {
    pub fn fine(&self, t: &Tol) -> bool {
        let ink = (self.ink[0] - self.ink[1]).abs() <= t.ink * self.ink[1].max(1.0);
        self.over <= t.over && self.max <= t.max && ink
    }

    pub fn text(&self) -> alloc::string::String {
        alloc::format!("{} pixels beyond, largest {} at {:?}, ink {:.0} against {:.0} ({:+.3} %)", self.over, self.max, self.at, self.ink[0], self.ink[1],
                       100.0 * (self.ink[0] - self.ink[1]) / self.ink[1].max(1.0))
    }
}

/// rows of B, G, R, x of both (the fourth byte means nothing)
pub fn compare(gpu: &[u8], reference: &[u8], w: u32, bg: u32, small: u8) -> Cmp {
    let mut c = Cmp::default();
    let bgc = [(bg & 255) as i32, ((bg >> 8) & 255) as i32, ((bg >> 16) & 255) as i32];
    for (i, (p, q)) in gpu.chunks_exact(4).zip(reference.chunks_exact(4)).enumerate() {
        let mut d = 0u8;
        for k in 0..3 {
            d = d.max(p[k].abs_diff(q[k]));
            c.ink[0] += (p[k] as i32 - bgc[k]).abs() as f64;
            c.ink[1] += (q[k] as i32 - bgc[k]).abs() as f64;
        }
        if d > small {
            c.over += 1;
        }
        if d > c.max {
            c.max = d;
            c.at = (i as u32 % w, i as u32 / w);
        }
    }
    c
}
