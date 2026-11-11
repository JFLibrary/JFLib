// cad -- the measuring scene: what a CAD drawing looks like, not a random walk.  A town plan in metres at surveyed
// coordinates (E 500 000, N 5 400 000): blocks in a grid with kerbs and pavements, streets as long polylines, parcels,
// buildings with double walls, inner walls, doors, windows, stairs, furniture and hatching, trees, labels written with
// short strokes, long utility lines through the town, contour lines of many short segments.  Deterministic: the same
// numbers give the same drawing on every machine and in the browser.
//
//   build(&mut scene, segments, seed, styled) -> Cad   exactly `segments` records
//   views(&cad, size)                                  the overview (all of it), a district, the working zoom

use alloc::vec::Vec;

use gpu::Result;
use render2d::math::sincos;
use render2d::{Camera, Cap, Join, Layer, LayerId, Scene, Shape, Style, StyleId, Unit};

use crate::gallery::Rng;

/// where the drawing lies (metres): a surveyed coordinate, so the hi / lo pairs are in use
pub const ORIGIN: [f64; 2] = [500_000.0, 5_400_000.0];

/// what build made
#[derive(Clone, Copy, Debug)]
pub struct Cad {
    pub segments: u32,
    pub entities: u32,
    /// the drawing's size (metres), its lower left corner at ORIGIN
    pub width: f64,
    pub height: f64,
}

struct Pen {
    l: LayerId,
    s: StyleId,
}

/// the layers of the drawing in draw order, with their styles
struct Pens {
    contour: Pen,
    utility: Pen,
    street: Pen,
    centre: Pen,
    kerb: Pen,
    parcel: Pen,
    hatch: Pen,
    outer: Pen,
    inner: Pen,
    detail: Pen,
    furniture: Pen,
    tree: Pen,
    label: Pen,
}

struct Sink<'a> {
    s: &'a mut Scene,
    left: u32,
    ents: u32,
}

impl Sink<'_> {
    /// (a polyline's records; dropped when the count is reached, cut short if only part fits)
    fn poly(&mut self, p: &Pen, pts: &[[f64; 2]], closed: bool) -> Result<()> {
        if self.left == 0 || pts.len() < 2 {
            return Ok(());
        }
        let n = if closed { pts.len() } else { pts.len() - 1 } as u32;
        if n > self.left {
            let keep = self.left as usize + 1;
            self.s.insert(p.l, p.s, &Shape::Polyline { points: &pts[..keep.min(pts.len())], closed: false })?;
            self.left = 0;
        } else {
            self.s.insert(p.l, p.s, &Shape::Polyline { points: pts, closed })?;
            self.left -= n;
        }
        self.ents += 1;
        Ok(())
    }

    fn line(&mut self, p: &Pen, a: [f64; 2], b: [f64; 2]) -> Result<()> {
        if self.left == 0 {
            return Ok(());
        }
        self.s.insert(p.l, p.s, &Shape::Line { a, b })?;
        self.left -= 1;
        self.ents += 1;
        Ok(())
    }
}

fn pens(s: &mut Scene, styled: bool) -> Result<Pens> {
    let mut order = 0;
    let mut pen = |s: &mut Scene, color: u32, width: f32, unit: Unit, cap: Cap, join: Join| -> Result<Pen> {
        let l = s.add_layer(&Layer::new(order))?;
        order += 1;
        let mut st = Style::new(color);
        if styled {
            st.width = width;
            st.unit = unit;
            st.cap = cap;
            st.join = join;
        }
        Ok(Pen { l, s: s.add_style(&st)? })
    };
    let (b, r, m) = (Cap::Butt, Cap::Round, Join::Miter);
    let jr = Join::Round;
    Ok(Pens {
        contour: pen(s, 0xFF9F_B39A, 0.0, Unit::Px, b, m)?,
        utility: pen(s, 0xFFC0_6070, 2.0, Unit::Px, b, m)?,
        street: pen(s, 0xFF55_5555, 3.0, Unit::Px, b, m)?,
        centre: pen(s, 0xA060_6060, 0.0, Unit::Px, b, m)?,
        kerb: pen(s, 0xFF70_7070, 1.0, Unit::Px, b, jr)?,
        parcel: pen(s, 0xFFA8_9070, 0.0, Unit::Px, b, m)?,
        hatch: pen(s, 0x6030_4050, 0.0, Unit::Px, b, m)?,
        outer: pen(s, 0xFF20_2830, 0.3, Unit::Drawing, b, m)?,
        inner: pen(s, 0xFF40_4850, 0.12, Unit::Drawing, b, m)?,
        detail: pen(s, 0xFF50_60A0, 0.0, Unit::Px, b, m)?,
        furniture: pen(s, 0xFF90_9090, 0.0, Unit::Px, b, m)?,
        tree: pen(s, 0xFF30_8040, 0.0, Unit::Px, r, jr)?,
        label: pen(s, 0xFF20_2020, 1.0, Unit::Px, r, jr)?,
    })
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> [[f64; 2]; 4] {
    [[x, y], [x + w, y], [x + w, y + h], [x, y + h]]
}

/// a footprint: a rectangle with notches on its sides, counter-clockwise
fn footprint(rng: &mut Rng, x: f64, y: f64, w: f64, h: f64, out: &mut Vec<[f64; 2]>) {
    out.clear();
    out.push([x, y]);
    // the bottom side, the right side, the top, the left, each with a notch or not
    let mut side = |rng: &mut Rng, a: [f64; 2], b: [f64; 2], n: [f64; 2]| {
        if rng.below(2) == 0 {
            let (t0, t1) = (rng.range(0.2, 0.45), rng.range(0.55, 0.8));
            let d = rng.range(0.6, 1.8);
            let p = |t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            let (q0, q1) = (p(t0), p(t1));
            out.push(q0);
            out.push([q0[0] + n[0] * d, q0[1] + n[1] * d]);
            out.push([q1[0] + n[0] * d, q1[1] + n[1] * d]);
            out.push(q1);
        }
        out.push(b);
    };
    side(rng, [x, y], [x + w, y], [0.0, 1.0]);
    side(rng, [x + w, y], [x + w, y + h], [-1.0, 0.0]);
    side(rng, [x + w, y + h], [x, y + h], [0.0, -1.0]);
    side(rng, [x, y + h], [x, y], [1.0, 0.0]);
    out.pop();
}

fn parcel(k: &mut Sink, rng: &mut Rng, p: &Pens, x: f64, y: f64, w: f64, h: f64) -> Result<()> {
    // the building
    let m = rng.range(1.2, 3.0);
    let (bx, by, bw, bh) = (x + m, y + m, w - 2.0 * m, h - 2.0 * m);
    let mut pts = Vec::new();
    footprint(rng, bx, by, bw, bh, &mut pts);
    k.poly(&p.outer, &pts, true)?;
    let inset: Vec<[f64; 2]> = pts.iter().map(|q| [q[0] + (bx + 0.5 * bw - q[0]).signum() * 0.3, q[1] + (by + 0.5 * bh - q[1]).signum() * 0.3]).collect();
    k.poly(&p.inner, &inset, true)?;
    // inner walls
    for _ in 0..6 + rng.below(6) {
        let (px, py) = (rng.range(bx + 0.5, bx + bw - 0.5), rng.range(by + 0.5, by + bh - 0.5));
        let l = rng.range(1.5, 6.0);
        if rng.below(2) == 0 {
            k.line(&p.inner, [px, py], [(px + l).min(bx + bw - 0.3), py])?;
        } else {
            k.line(&p.inner, [px, py], [px, (py + l).min(by + bh - 0.3)])?;
        }
    }
    // doors: the leaf and its swing, six short segments
    for _ in 0..3 + rng.below(4) {
        let (px, py) = (rng.range(bx + 1.0, bx + bw - 1.0), rng.range(by + 1.0, by + bh - 1.0));
        let r = rng.range(0.75, 1.0);
        let a0 = rng.below(4) as f64 * core::f64::consts::FRAC_PI_2;
        let (s, c) = sincos(a0);
        k.line(&p.detail, [px, py], [px + r * c, py + r * s])?;
        let arc: Vec<[f64; 2]> = (0..7).map(|j| {
            let (s, c) = sincos(a0 + j as f64 / 6.0 * core::f64::consts::FRAC_PI_2);
            [px + r * c, py + r * s]
        }).collect();
        k.poly(&p.detail, &arc, false)?;
    }
    // windows on the outer walls: three short lines each
    for _ in 0..8 + rng.below(7) {
        let t = rng.range(0.1, 0.9);
        let (px, py) = if rng.below(2) == 0 { (bx + t * bw, if rng.below(2) == 0 { by } else { by + bh }) } else { (if rng.below(2) == 0 { bx } else { bx + bw }, by + t * bh) };
        let l = rng.range(0.6, 1.4);
        for j in 0..3 {
            let off = j as f64 * 0.1 - 0.1;
            k.line(&p.detail, [px - 0.5 * l, py + off], [px + 0.5 * l, py + off])?;
        }
    }
    // a stair
    if rng.below(3) == 0 {
        let (px, py) = (rng.range(bx + 1.0, bx + bw - 3.5), rng.range(by + 1.0, by + bh - 2.5));
        for j in 0..11 {
            k.line(&p.detail, [px + j as f64 * 0.26, py], [px + j as f64 * 0.26, py + 1.1])?;
        }
    }
    // furniture
    for _ in 0..3 + rng.below(6) {
        let (fw, fh) = (rng.range(0.5, 2.0), rng.range(0.5, 1.2));
        let r = rect(rng.range(bx + 0.5, bx + bw - 2.5), rng.range(by + 0.5, by + bh - 1.5), fw, fh);
        k.poly(&p.furniture, &r, true)?;
    }
    // hatching over part of it: diagonal lines of a few metres
    let (hx, hy) = (rng.range(bx, bx + 0.4 * bw), rng.range(by, by + 0.4 * bh));
    for j in 0..8 + rng.below(13) {
        let t = j as f64 * 0.45;
        let l = rng.range(2.0, 9.0).min(bw * 0.6).min(bh * 0.6);
        k.line(&p.hatch, [hx + t, hy], [hx + t + l, hy + l])?;
    }
    Ok(())
}

fn block(k: &mut Sink, rng: &mut Rng, p: &Pens, x0: f64, y0: f64, w: f64, h: f64) -> Result<()> {
    // kerb and pavement: closed polylines with rounded corners
    for inset in [0.0, 2.5] {
        let (x, y, bw, bh) = (x0 + inset, y0 + inset, w - 2.0 * inset, h - 2.0 * inset);
        let r = 4.0 - inset * 0.8;
        let mut pts = Vec::with_capacity(16);
        for (cx, cy, a0) in [(x + bw - r, y + r, -core::f64::consts::FRAC_PI_2), (x + bw - r, y + bh - r, 0.0), (x + r, y + bh - r, core::f64::consts::FRAC_PI_2), (x + r, y + r, core::f64::consts::PI)] {
            for j in 0..4 {
                let (s, c) = sincos(a0 + j as f64 / 3.0 * core::f64::consts::FRAC_PI_2);
                pts.push([cx + r * c, cy + r * s]);
            }
        }
        k.poly(&p.kerb, &pts, true)?;
    }
    // parcels in two rows
    let cols = 6 + rng.below(5);
    let (ix, iy, iw, ih) = (x0 + 3.0, y0 + 3.0, w - 6.0, h - 6.0);
    let pw = iw / cols as f64;
    for row in 0..2 {
        let py = iy + row as f64 * 0.5 * ih;
        for c in 0..cols {
            let px = ix + c as f64 * pw;
            k.poly(&p.parcel, &rect(px, py, pw, 0.5 * ih), true)?;
            parcel(k, rng, p, px, py, pw, 0.5 * ih)?;
        }
    }
    // trees: a twelve-sided crown and its cross, along the pavement
    for _ in 0..8 + rng.below(9) {
        let (tx, ty) = (rng.range(x0 - 4.0, x0 + w + 4.0), if rng.below(2) == 0 { y0 - 3.0 } else { y0 + h + 3.0 });
        let r = rng.range(1.5, 4.0);
        let crown: Vec<[f64; 2]> = (0..12).map(|j| {
            let (s, c) = sincos(j as f64 / 12.0 * core::f64::consts::TAU);
            [tx + r * c, ty + r * s]
        }).collect();
        k.poly(&p.tree, &crown, true)?;
        k.line(&p.tree, [tx - 0.5 * r, ty], [tx + 0.5 * r, ty])?;
        k.line(&p.tree, [tx, ty - 0.5 * r], [tx, ty + 0.5 * r])?;
    }
    // labels: a character is one stroke of a few short pieces in a box of 1 by 1.6 m
    for _ in 0..6 + rng.below(5) {
        let (mut lx, ly) = (rng.range(x0 + 4.0, x0 + w - 12.0), rng.range(y0 + 4.0, y0 + h - 4.0));
        for _ in 0..4 + rng.below(5) {
            let n = 4 + rng.below(3) as usize;
            let ch: Vec<[f64; 2]> = (0..n).map(|_| [lx + rng.range(0.0, 1.0), ly + rng.range(0.0, 1.6)]).collect();
            k.poly(&p.label, &ch, false)?;
            lx += 1.4;
        }
    }
    Ok(())
}

/// a polyline along x from x0 to x1 that wanders by amp around y, a point every `step`
fn contour(rng: &mut Rng, x0: f64, x1: f64, y: f64, amp: f64, step: f64, pts: &mut Vec<[f64; 2]>) {
    pts.clear();
    let (mut x, mut v) = (x0, 0.0f64);
    let mut yy = y;
    while x < x1 {
        pts.push([x, yy]);
        v = (v + rng.range(-0.25, 0.25) * amp).clamp(-amp, amp);
        yy += v;
        x += step;
    }
}

/// a drawing of exactly `n` segments (fewer entities than a random walk has: a CAD drawing has long lines and many small
/// details).  styled: the widths of a plan (streets 3 px, walls in metres, ...), else every stroke a hairline
pub fn build(s: &mut Scene, n: u32, seed: u32, styled: bool) -> Result<Cad> {
    let p = pens(s, styled)?;
    let mut rng = Rng(seed);
    let (pitch_x, pitch_y) = (140.0, 100.0);
    // about 2500 segments a block; contour lines, streets and utilities take the rest
    let blocks = (0.88 * n as f64 / 2500.0) as u32 + 1;
    let cols = ((render2d::math::sqrt(1.27 * blocks as f64) + 0.999) as u32).max(1);
    let rows = blocks.div_ceil(cols);
    let (width, height) = (cols as f64 * pitch_x + 20.0, rows as f64 * pitch_y + 20.0);
    let mut k = Sink { s, left: n, ents: 0 };
    let at = |x: f64, y: f64| [ORIGIN[0] + x, ORIGIN[1] + y];
    // streets: both edges and the centre line of each, long polylines with a point per block
    let road = 12.0;
    for c in 0..=cols {
        let x = 10.0 + c as f64 * pitch_x - 0.5 * road;
        for (dx, pen) in [(0.0, &p.street), (0.5 * road, &p.centre), (road, &p.street)] {
            let pts: Vec<[f64; 2]> = (0..=rows).map(|r| at(x + dx + rng.range(-0.2, 0.2), 10.0 + r as f64 * pitch_y)).collect();
            k.poly(pen, &pts, false)?;
        }
    }
    for r in 0..=rows {
        let y = 10.0 + r as f64 * pitch_y - 0.5 * road;
        for (dy, pen) in [(0.0, &p.street), (0.5 * road, &p.centre), (road, &p.street)] {
            let pts: Vec<[f64; 2]> = (0..=cols).map(|c| at(10.0 + c as f64 * pitch_x, y + dy + rng.range(-0.2, 0.2))).collect();
            k.poly(pen, &pts, false)?;
        }
    }
    // long utility lines through the town, straight: a few hundred metres to its full size
    for _ in 0..n / 400 {
        let a = [rng.range(0.0, width), rng.range(0.0, height)];
        let l = rng.range(150.0, 0.5 * (width + height));
        let (sn, cs) = sincos(rng.range(0.0, core::f64::consts::TAU));
        let b = [(a[0] + l * cs).clamp(0.0, width), (a[1] + l * sn).clamp(0.0, height)];
        k.line(&p.utility, at(a[0], a[1]), at(b[0], b[1]))?;
    }
    // the blocks
    for r in 0..rows {
        for c in 0..cols {
            if r * cols + c >= blocks {
                break;
            }
            let (x0, y0) = (10.0 + c as f64 * pitch_x + 0.5 * road + 4.0, 10.0 + r as f64 * pitch_y + 0.5 * road + 4.0);
            // (the block's content in drawing coordinates: moved by the origin on its way out of the sink)
            block(&mut k, &mut rng, &p, ORIGIN[0] + x0, ORIGIN[1] + y0, pitch_x - road - 8.0, pitch_y - road - 8.0)?;
        }
    }
    // whatever is left: contour lines over the whole of it, a point every few metres
    let mut pts = Vec::new();
    let mut y = 0.0;
    while k.left > 0 {
        let (start, end, step) = (rng.range(0.0, 0.2 * width), width - rng.range(0.0, 0.2 * width), rng.range(2.0, 6.0));
        contour(&mut rng, start, end, y, 0.8, step, &mut pts);
        let moved: Vec<[f64; 2]> = pts.iter().map(|q| at(q[0], q[1])).collect();
        k.poly(&p.contour, &moved, false)?;
        y += rng.range(0.5, 5.0);
        if y > height {
            y -= height;
        }
    }
    Ok(Cad { segments: n, entities: k.ents, width, height })
}

/// a named view of the drawing in a target of `size`
pub struct View {
    pub name: &'static str,
    pub cam: Camera,
}

/// the whole drawing, a district (a sixteenth of the area), the working zoom (10 cm per pixel at full HD: two blocks)
pub fn views(c: &Cad, size: (u32, u32)) -> [View; 3] {
    let centre = [ORIGIN[0] + 0.5 * c.width, ORIGIN[1] + 0.5 * c.height];
    let all = (c.width / size.0 as f64).max(c.height / size.1 as f64) * 1.02;
    let work = 0.1 * 1920.0 / size.0 as f64;
    [
        View { name: "overview", cam: Camera::new(centre, all) },
        View { name: "district", cam: Camera::new([centre[0] + 0.13 * c.width, centre[1] - 0.09 * c.height], 0.25 * all) },
        View { name: "working zoom", cam: Camera::new([centre[0] + 0.13 * c.width, centre[1] - 0.09 * c.height], work) },
    ]
}
