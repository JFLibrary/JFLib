// msdf -- multi-channel signed distance fields of glyph outlines on the CPU (G6c, decision 27; docs/RENDER2D.md "Text").
//
// A glyph's contours (lines, quadratic and cubic Bézier segments in em units, y up) become a tile of RGBA texels: red,
// green and blue each the signed pseudo-distance to the nearest edge of its colour, alpha the true signed distance to the
// nearest edge of any colour (an MTSDF), positive inside, 0.5 on the outline, range / 2 texels on either side clamped.
// The median of red, green and blue keeps a corner sharp at any magnification: edge colouring gives the two edges at a
// corner different colours, so that two channels meet there as two straight half planes.  As msdfgen (Chlumsky, MIT;
// rewritten from its description, nothing taken over as code):
//
//   edge colouring   per contour the corners (a turn sharper than CORNER_CROSS, or back); none: every edge white (all
//                    channels); one: three colours along the contour (an edge split in thirds where it has fewer than
//                    three); more: cyan, magenta, yellow switching at each corner, the last spline different from the first
//   distances        per texel and channel the edge of that colour with the least true distance (ties: the one met more
//                    squarely), then its pseudo-distance (an end's tangent extended beyond the edge); alpha the least
//                    true distance.  The nearest point of a curve: samples, then a safe Newton step in the bracket around
//                    each sampled minimum (no trigonometry: the same bits on every target)
//   sign correction  each texel's centre inside or out by the non-zero winding of a scanline; where the median says
//                    otherwise every channel is mirrored at 0.5 (wrong signs from overlapping or touching contours)
//   error correction a texel whose channels clash with a neighbour's (two channels jumping across 0.5 by more than a
//                    texel's worth while the third stays: interpolating between them would make a false edge, at corners
//                    and where contours come near each other) takes its median in all three channels; then the same for
//                    the diagonal neighbours
//
// Everything is f64 with +, -, *, / and a square root by Newton's method (math::sqrt): the same tile bytes on every
// target (Windows, Linux, wasm32, Android), which the browser's and Vulkan's images rely on.

use alloc::vec;
use alloc::vec::Vec;

use crate::math::sqrt;

pub type P = [f64; 2];

/// a segment of a contour (em units, y up)
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Line(P, P),
    Quad(P, P, P),
    Cubic(P, P, P, P),
}

/// a corner is a turn whose cross product of the unit directions exceeds this (sin 3 rad, msdfgen's default angle) or
/// that turns back (dot <= 0)
pub const CORNER_CROSS: f64 = 0.141_120_008_059_867_22;

const RED: u8 = 1;
const GREEN: u8 = 2;
const BLUE: u8 = 4;
// (cyan, then by switch_color magenta = red | blue, yellow = red | green)
const CYAN: u8 = GREEN | BLUE;
const WHITE: u8 = 7;

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}

fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}

fn mul(a: P, s: f64) -> P {
    [a[0] * s, a[1] * s]
}

fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn cross(a: P, b: P) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

fn len(a: P) -> f64 {
    sqrt(dot(a, a))
}

fn norm(a: P) -> P {
    let l = len(a);
    if l > 0.0 { mul(a, 1.0 / l) } else { [0.0, 0.0] }
}

fn lerp(a: P, b: P, t: f64) -> P {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// msdfgen's nonZeroSign: 1 above 0, else -1
fn nz_sign(x: f64) -> f64 {
    if x > 0.0 { 1.0 } else { -1.0 }
}

/// x rounded down (|x| below 2^52)
pub(crate) fn floor(x: f64) -> f64 {
    if !(x.abs() < 4.5e15) {
        return x;
    }
    let t = x as i64 as f64;
    if t > x { t - 1.0 } else { t }
}

pub(crate) fn ceil(x: f64) -> f64 {
    -floor(-x)
}

impl Seg {
    pub fn start(&self) -> P {
        match *self {
            Seg::Line(a, _) | Seg::Quad(a, _, _) | Seg::Cubic(a, _, _, _) => a,
        }
    }

    pub fn end(&self) -> P {
        match *self {
            Seg::Line(_, b) | Seg::Quad(_, _, b) | Seg::Cubic(_, _, _, b) => b,
        }
    }

    /// the point at t (de Casteljau)
    pub fn point(&self, t: f64) -> P {
        match *self {
            Seg::Line(a, b) => lerp(a, b, t),
            Seg::Quad(a, b, c) => lerp(lerp(a, b, t), lerp(b, c, t), t),
            Seg::Cubic(a, b, c, d) => {
                let (ab, bc, cd) = (lerp(a, b, t), lerp(b, c, t), lerp(c, d, t));
                lerp(lerp(ab, bc, t), lerp(bc, cd, t), t)
            }
        }
    }

    /// the derivative at t
    fn deriv(&self, t: f64) -> P {
        match *self {
            Seg::Line(a, b) => sub(b, a),
            Seg::Quad(a, b, c) => mul(lerp(sub(b, a), sub(c, b), t), 2.0),
            Seg::Cubic(a, b, c, d) => {
                let (u, v, w) = (sub(b, a), sub(c, b), sub(d, c));
                mul(lerp(lerp(u, v, t), lerp(v, w, t), t), 3.0)
            }
        }
    }

    fn deriv2(&self, t: f64) -> P {
        match *self {
            Seg::Line(..) => [0.0, 0.0],
            Seg::Quad(a, b, c) => mul(add(sub(c, mul(b, 2.0)), a), 2.0),
            Seg::Cubic(a, b, c, d) => {
                let u = add(sub(c, mul(b, 2.0)), a);
                let v = add(sub(d, mul(c, 2.0)), b);
                mul(lerp(u, v, t), 6.0)
            }
        }
    }

    /// the direction at its start (a control point on the end: the next one), msdfgen's direction(0)
    pub fn dir_start(&self) -> P {
        match *self {
            Seg::Line(a, b) => sub(b, a),
            Seg::Quad(a, b, c) => {
                let d = sub(b, a);
                if d == [0.0, 0.0] { sub(c, a) } else { d }
            }
            Seg::Cubic(a, b, c, d) => {
                let t = sub(b, a);
                if t != [0.0, 0.0] {
                    t
                } else if sub(c, a) != [0.0, 0.0] {
                    sub(c, a)
                } else {
                    sub(d, a)
                }
            }
        }
    }

    pub fn dir_end(&self) -> P {
        match *self {
            Seg::Line(a, b) => sub(b, a),
            Seg::Quad(a, b, c) => {
                let d = sub(c, b);
                if d == [0.0, 0.0] { sub(c, a) } else { d }
            }
            Seg::Cubic(a, b, c, d) => {
                let t = sub(d, c);
                if t != [0.0, 0.0] {
                    t
                } else if sub(d, b) != [0.0, 0.0] {
                    sub(d, b)
                } else {
                    sub(d, a)
                }
            }
        }
    }

    /// the part from t0 to t1 (de Casteljau twice)
    fn part(&self, t0: f64, t1: f64) -> Seg {
        match *self {
            Seg::Line(a, b) => Seg::Line(lerp(a, b, t0), lerp(a, b, t1)),
            Seg::Quad(a, b, c) => {
                // (blossoms: f(t0, t0), f(t0, t1), f(t1, t1))
                let f = |u: f64, v: f64| lerp(lerp(a, b, u), lerp(b, c, u), v);
                Seg::Quad(f(t0, t0), f(t0, t1), f(t1, t1))
            }
            Seg::Cubic(a, b, c, d) => {
                let f = |u: f64, v: f64, w: f64| {
                    let (ab, bc, cd) = (lerp(a, b, u), lerp(b, c, u), lerp(c, d, u));
                    lerp(lerp(ab, bc, v), lerp(bc, cd, v), w)
                };
                Seg::Cubic(f(t0, t0, t0), f(t0, t0, t1), f(t0, t1, t1), f(t1, t1, t1))
            }
        }
    }

    /// its control hull's box: x0, y0, x1, y1
    pub fn bbox(&self) -> [f64; 4] {
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        let mut g = |p: P| b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
        match *self {
            Seg::Line(a, c) => {
                g(a);
                g(c);
            }
            Seg::Quad(a, c, d) => {
                g(a);
                g(c);
                g(d);
            }
            Seg::Cubic(a, c, d, e) => {
                g(a);
                g(c);
                g(d);
                g(e);
            }
        }
        b
    }

    /// the signed distance from o (positive on the right of the direction, y up) and the parameter of the nearest point
    /// (beyond 0 or 1 where an end is nearest: how far o lies along that end's tangent, for the pseudo-distance)
    fn signed_distance(&self, o: P) -> (Sd, f64) {
        if let Seg::Line(a, b) = *self {
            let aq = sub(o, a);
            let ab = sub(b, a);
            let l2 = dot(ab, ab);
            let param = if l2 > 0.0 { dot(aq, ab) / l2 } else { 0.0 };
            let eq = sub(if param > 0.5 { b } else { a }, o);
            let ep = len(eq);
            if param > 0.0 && param < 1.0 {
                let ortho = cross(aq, ab) / sqrt(l2);
                if ortho.abs() < ep {
                    return (Sd { d: ortho, dot: 0.0 }, param);
                }
            }
            return (Sd { d: nz_sign(cross(aq, ab)) * ep, dot: dot(norm(ab), norm(eq)).abs() }, param);
        }
        let (t, _) = self.nearest(o);
        if t > 0.0 && t < 1.0 {
            let q = sub(self.point(t), o);
            return (Sd { d: nz_sign(cross(self.deriv(t), q)) * len(q), dot: 0.0 }, t);
        }
        if t <= 0.0 {
            let (a, dir) = (self.start(), self.dir_start());
            let q = sub(a, o);
            let dd = dot(dir, dir);
            let param = if dd > 0.0 { dot(sub(o, a), dir) / dd } else { 0.0 };
            return (Sd { d: nz_sign(cross(dir, q)) * len(q), dot: dot(norm(dir), norm(q)).abs() }, param.min(0.0));
        }
        let (b, dir) = (self.end(), self.dir_end());
        let q = sub(b, o);
        let dd = dot(dir, dir);
        let param = if dd > 0.0 { 1.0 + dot(sub(o, b), dir) / dd } else { 1.0 };
        (Sd { d: nz_sign(cross(dir, q)) * len(q), dot: dot(norm(dir), norm(q)).abs() }, param.max(1.0))
    }

    /// the parameter of a curve's point nearest to o and the squared distance: samples, each sampled minimum refined by
    /// a safe Newton iteration on (B(t) - o) . B'(t) in the bracket of its neighbours; the ends count as they are
    fn nearest(&self, o: P) -> (f64, f64) {
        let n = if matches!(self, Seg::Cubic(..)) { 12 } else { 8 };
        let mut f = [0.0f64; 13];
        for (k, v) in f.iter_mut().enumerate().take(n + 1) {
            let q = sub(self.point(k as f64 / n as f64), o);
            *v = dot(q, q);
        }
        let (mut best_t, mut best_f) = if f[n] < f[0] { (1.0, f[n]) } else { (0.0, f[0]) };
        let g = |t: f64| dot(sub(self.point(t), o), self.deriv(t));
        for k in 1..n {
            if !(f[k] <= f[k - 1] && f[k] <= f[k + 1]) {
                continue;
            }
            let (mut a, mut b) = ((k - 1) as f64 / n as f64, (k + 1) as f64 / n as f64);
            let (ga, gb) = (g(a), g(b));
            let mut t = k as f64 / n as f64;
            if ga <= 0.0 && gb >= 0.0 {
                for _ in 0..40 {
                    let p = sub(self.point(t), o);
                    let d1 = self.deriv(t);
                    let gt = dot(p, d1);
                    if gt == 0.0 {
                        break;
                    }
                    if gt < 0.0 {
                        a = t;
                    } else {
                        b = t;
                    }
                    let gp = dot(d1, d1) + dot(p, self.deriv2(t));
                    let mut tn = if gp > 0.0 { t - gt / gp } else { 0.5 * (a + b) };
                    if !(tn > a && tn < b) {
                        tn = 0.5 * (a + b);
                    }
                    if tn == t || b - a < 1e-15 {
                        break;
                    }
                    t = tn;
                }
            }
            let q = sub(self.point(t), o);
            let ft = dot(q, q);
            if ft < best_f {
                best_f = ft;
                best_t = t;
            }
        }
        (best_t, best_f)
    }

    /// msdfgen's distanceToPseudoDistance: beyond an end, the distance to that end's tangent line where it is nearer
    fn pseudo(&self, sd: Sd, o: P, param: f64) -> f64 {
        if param < 0.0 {
            let dir = norm(self.dir_start());
            let aq = sub(o, self.start());
            if dot(aq, dir) < 0.0 {
                let pd = cross(aq, dir);
                if pd.abs() <= sd.d.abs() {
                    return pd;
                }
            }
        } else if param > 1.0 {
            let dir = norm(self.dir_end());
            let bq = sub(o, self.end());
            if dot(bq, dir) > 0.0 {
                let pd = cross(bq, dir);
                if pd.abs() <= sd.d.abs() {
                    return pd;
                }
            }
        }
        sd.d
    }

    /// the parameters where y is extreme inside (0, 1): the segment is monotone in y between them
    fn y_splits(&self, out: &mut Vec<f64>) {
        out.clear();
        match *self {
            Seg::Line(..) => {}
            Seg::Quad(a, b, c) => {
                let den = a[1] - 2.0 * b[1] + c[1];
                if den != 0.0 {
                    let t = (a[1] - b[1]) / den;
                    if t > 0.0 && t < 1.0 {
                        out.push(t);
                    }
                }
            }
            Seg::Cubic(a, b, c, d) => {
                // y'(t) / 3 = A t^2 + B t + C
                let (u, v, w) = (b[1] - a[1], c[1] - b[1], d[1] - c[1]);
                let qa = u - 2.0 * v + w;
                let qb = 2.0 * (v - u);
                let qc = u;
                if qa.abs() < 1e-300 {
                    if qb != 0.0 {
                        let t = -qc / qb;
                        if t > 0.0 && t < 1.0 {
                            out.push(t);
                        }
                    }
                } else {
                    let disc = qb * qb - 4.0 * qa * qc;
                    if disc >= 0.0 {
                        let s = sqrt(disc);
                        // (the stable pair of roots)
                        let qq = -0.5 * (qb + if qb >= 0.0 { s } else { -s });
                        let mut r = [qq / qa, if qq != 0.0 { qc / qq } else { -1.0 }];
                        if r[0] > r[1] {
                            r.swap(0, 1);
                        }
                        for t in r {
                            if t > 0.0 && t < 1.0 && out.last() != Some(&t) {
                                out.push(t);
                            }
                        }
                    }
                }
            }
        }
    }
}

/// a signed distance and how squarely the nearest point is met (0: perpendicular; msdfgen's SignedDistance)
#[derive(Clone, Copy, Debug)]
struct Sd {
    d: f64,
    dot: f64,
}

impl Sd {
    const FAR: Sd = Sd { d: -1e300, dot: 1.0 };

    fn less(&self, o: &Sd) -> bool {
        let (a, b) = (self.d.abs(), o.d.abs());
        a < b || (a == b && self.dot < o.dot)
    }
}

/// how a size class of the atlas makes its tiles: texels per em, the distance range (texels, both sides together), the
/// texels around the glyph's box
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub em: f64,
    pub range: f64,
    pub pad: u32,
}

/// a glyph's field: w x h texels of RGBA (row 0 the lowest), texel (i, j) centred at ((x0 + i + 0.5) / em, (y0 + j + 0.5)
/// / em) em units
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tile {
    pub w: u32,
    pub h: u32,
    pub x0: i32,
    pub y0: i32,
    pub px: Vec<u8>,
}

/// an edge and its colour (RED | GREEN | BLUE bits)
#[derive(Clone, Copy, Debug)]
struct Edge {
    seg: Seg,
    color: u8,
    bbox: [f64; 4],
}

fn is_corner(a: P, b: P) -> bool {
    dot(a, b) <= 0.0 || cross(a, b).abs() > CORNER_CROSS
}

/// msdfgen's switchColor with seed 0 (deterministic: cyan first, then magenta, yellow, cyan ...); banned: a colour the
/// last spline may not share two channels with
fn switch_color(c: u8, banned: u8) -> u8 {
    let combined = c & banned;
    if combined == RED || combined == GREEN || combined == BLUE {
        return combined ^ WHITE;
    }
    if c == 0 || c == WHITE {
        return CYAN;
    }
    let s = (c as u32) << 1;
    ((s | s >> 3) & WHITE as u32) as u8
}

/// the colours of a contour's edges (edges split in thirds where a teardrop has fewer than three)
fn colour_contour(segs: &[Seg], out: &mut Vec<Edge>) {
    let m = segs.len();
    if m == 0 {
        return;
    }
    let mut corners = Vec::new();
    for i in 0..m {
        let prev = segs[(i + m - 1) % m];
        if is_corner(norm(prev.dir_end()), norm(segs[i].dir_start())) {
            corners.push(i);
        }
    }
    let push = |out: &mut Vec<Edge>, seg: Seg, color: u8| out.push(Edge { seg, color, bbox: seg.bbox() });
    if corners.is_empty() {
        for &s in segs {
            push(out, s, WHITE);
        }
        return;
    }
    if corners.len() == 1 {
        // a teardrop: three colours along the contour, the corner between the first and the last
        let c0 = switch_color(WHITE, 0);
        let c2 = switch_color(c0, 0);
        let colors = [c0, WHITE, c2];
        let corner = corners[0];
        if m >= 3 {
            for i in 0..m {
                // msdfgen: (colors + 1)[int(3 + 2.875 i / (m - 1) - 1.4375 + .5) - 3]
                let k = (3.0 + 2.875 * i as f64 / (m - 1) as f64 - 1.4375 + 0.5) as i64 - 3;
                let col = colors[(k + 1).clamp(0, 2) as usize];
                push(out, segs[(corner + i) % m], col);
            }
        } else {
            let thirds = |s: Seg| [s.part(0.0, 1.0 / 3.0), s.part(1.0 / 3.0, 2.0 / 3.0), s.part(2.0 / 3.0, 1.0)];
            let mut parts: Vec<Seg> = Vec::new();
            if m == 1 {
                parts.extend_from_slice(&thirds(segs[0]));
                push(out, parts[0], colors[0]);
                push(out, parts[1], colors[1]);
                push(out, parts[2], colors[2]);
            } else {
                // two edges: six parts, the corner's edge first
                let (e0, e1) = (segs[corner], segs[(corner + 1) % 2]);
                parts.extend_from_slice(&thirds(e0));
                parts.extend_from_slice(&thirds(e1));
                let cols = [colors[0], colors[0], colors[1], colors[1], colors[2], colors[2]];
                for (p, c) in parts.iter().zip(cols) {
                    push(out, *p, c);
                }
            }
        }
        return;
    }
    let n = corners.len();
    let mut spline = 0usize;
    let start = corners[0];
    let mut color = switch_color(WHITE, 0);
    let initial = color;
    for i in 0..m {
        let index = (start + i) % m;
        if spline + 1 < n && corners[spline + 1] == index {
            spline += 1;
            color = switch_color(color, if spline == n - 1 { initial } else { 0 });
        }
        push(out, segs[index], color);
    }
}

/// the signed area of the contours (y up: counter-clockwise positive), each segment as 16 chords
fn signed_area(contours: &[Vec<Seg>]) -> f64 {
    let mut a = 0.0;
    for c in contours {
        for s in c {
            let mut p = s.start();
            for k in 1..=16 {
                let q = s.point(k as f64 / 16.0);
                a += cross(p, q);
                p = q;
            }
        }
    }
    0.5 * a
}

fn median(a: f64, b: f64, c: f64) -> f64 {
    a.min(b).max(a.max(b).min(c))
}

/// the box of the contours' control hulls (None: no segment)
pub fn bounds(contours: &[Vec<Seg>]) -> Option<[f64; 4]> {
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    let mut any = false;
    for c in contours {
        for s in c {
            let o = s.bbox();
            b = [b[0].min(o[0]), b[1].min(o[1]), b[2].max(o[2]), b[3].max(o[3])];
            any = true;
        }
    }
    if any { Some(b) } else { None }
}

/// the tile's texel box for contours of this box: (x0, y0, w, h) in texels of the class
pub fn tile_box(b: [f64; 4], p: &Params) -> (i32, i32, u32, u32) {
    let pad = p.pad as i32;
    let x0 = floor(b[0] * p.em) as i32 - pad;
    let y0 = floor(b[1] * p.em) as i32 - pad;
    let x1 = ceil(b[2] * p.em) as i32 + pad;
    let y1 = ceil(b[3] * p.em) as i32 + pad;
    (x0, y0, (x1 - x0).max(1) as u32, (y1 - y0).max(1) as u32)
}

/// the non-zero winding inside test of a row of texel centres at y: per centre x (ascending) whether it is inside
fn scanline(edges: &[Edge], y: f64, xs: &[f64], inside: &mut [bool]) {
    let mut hits: Vec<(f64, i32)> = Vec::new();
    let mut splits = Vec::new();
    for e in edges {
        if e.bbox[1] > y || e.bbox[3] < y {
            continue;
        }
        e.seg.y_splits(&mut splits);
        let mut ts = vec![0.0];
        ts.extend_from_slice(&splits);
        ts.push(1.0);
        for w in ts.windows(2) {
            let (t0, t1) = (w[0], w[1]);
            let (p0, p1) = (e.seg.point(t0), e.seg.point(t1));
            let (ya, yb) = (p0[1], p1[1]);
            if ya == yb {
                continue;
            }
            // (half open: a piece holds y in [min, max), so a vertex on the row counts once)
            let (lo, hi) = if ya < yb { (ya, yb) } else { (yb, ya) };
            if !(y >= lo && y < hi) {
                continue;
            }
            let up = yb > ya;
            let (mut a, mut b) = (t0, t1);
            for _ in 0..60 {
                let m = 0.5 * (a + b);
                if m <= a || m >= b {
                    break;
                }
                let ym = e.seg.point(m)[1];
                if (ym < y) == up {
                    a = m;
                } else {
                    b = m;
                }
            }
            let x = e.seg.point(0.5 * (a + b))[0];
            hits.push((x, if up { 1 } else { -1 }));
        }
    }
    for (k, &x) in xs.iter().enumerate() {
        let mut w = 0;
        for &(hx, d) in &hits {
            if hx > x {
                w += d;
            }
        }
        inside[k] = w != 0;
    }
}

/// msdfgen 1.6's pixelClash: two texels both inside or both outside whose two changing channels jump across 0.5 by at
/// least `threshold` each; only the one farther from the outline is flagged
fn clash(a: [f64; 3], b: [f64; 3], threshold: f64) -> bool {
    let ins = |p: [f64; 3]| (p[0] > 0.5) as u32 + (p[1] > 0.5) as u32 + (p[2] > 0.5) as u32 >= 2;
    if ins(a) != ins(b) {
        return false;
    }
    let all = |p: [f64; 3]| (p[0] > 0.5 && p[1] > 0.5 && p[2] > 0.5) || (p[0] < 0.5 && p[1] < 0.5 && p[2] < 0.5);
    if all(a) || all(b) {
        return false;
    }
    let flips = |k: usize| (a[k] > 0.5) != (b[k] > 0.5) && (a[k] < 0.5) != (b[k] < 0.5);
    let (i, j, c) = if flips(0) {
        if flips(1) {
            (0, 1, 2)
        } else if flips(2) {
            (0, 2, 1)
        } else {
            return false;
        }
    } else if flips(1) && flips(2) {
        (1, 2, 0)
    } else {
        return false;
    };
    (a[i] - b[i]).abs() >= threshold && (a[j] - b[j]).abs() >= threshold && (a[c] - 0.5).abs() >= (b[c] - 0.5).abs()
}

/// a glyph's tile (None: no outline).  contours in em units, y up, closed (each segment starts where the one before
/// ended); fill rule non-zero
pub fn tile(contours: &[Vec<Seg>], p: &Params) -> Option<Tile> {
    let b = bounds(contours)?;
    let (x0, y0, w, h) = tile_box(b, p);
    let mut edges = Vec::new();
    for c in contours {
        colour_contour(c, &mut edges);
    }
    if edges.is_empty() {
        return None;
    }
    // (positive inside: the right of the direction where the outer contours run clockwise, as TrueType's; CFF's run the
    // other way)
    let flip = if signed_area(contours) > 0.0 { -1.0 } else { 1.0 };
    let (wu, hu) = (w as usize, h as usize);
    let mut v = vec![[0.0f64; 4]; wu * hu];
    let em = p.em;
    let mut order: Vec<(f64, u16)> = Vec::with_capacity(edges.len());
    let xs: Vec<f64> = (0..wu).map(|i| (x0 as f64 + i as f64 + 0.5) / em).collect();
    let mut inside = vec![false; wu];
    for j in 0..hu {
        let y = (y0 as f64 + j as f64 + 0.5) / em;
        scanline(&edges, y, &xs, &mut inside);
        for i in 0..wu {
            let o = [xs[i], y];
            // the edges by how near their boxes come: those beyond every channel's best cannot win
            order.clear();
            for (k, e) in edges.iter().enumerate() {
                let dx = (e.bbox[0] - o[0]).max(o[0] - e.bbox[2]).max(0.0);
                let dy = (e.bbox[1] - o[1]).max(o[1] - e.bbox[3]).max(0.0);
                order.push((dx * dx + dy * dy, k as u16));
            }
            order.sort_unstable_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal).then(a.1.cmp(&b.1)));
            let mut best = [Sd::FAR; 3];
            let mut near: [Option<(usize, f64)>; 3] = [None; 3];
            let mut tru = Sd::FAR;
            for &(bd, k) in &order {
                let worst = best.iter().map(|s| s.d.abs()).fold(tru.d.abs(), f64::max);
                if bd > worst * worst {
                    break;
                }
                let e = &edges[k as usize];
                let (sd, param) = e.seg.signed_distance(o);
                if sd.less(&tru) {
                    tru = sd;
                }
                for ch in 0..3 {
                    if e.color & (1 << ch) != 0 && sd.less(&best[ch]) {
                        best[ch] = sd;
                        near[ch] = Some((k as usize, param));
                    }
                }
            }
            let mut px = [0.0f64; 4];
            for ch in 0..3 {
                let d = match near[ch] {
                    Some((k, param)) => edges[k].seg.pseudo(best[ch], o, param),
                    None => -1e300,
                };
                px[ch] = 0.5 + flip * d * em / p.range;
            }
            px[3] = 0.5 + flip * tru.d * em / p.range;
            // sign correction: the centre's side by the winding rule decides
            let inn = inside[i];
            if (median(px[0], px[1], px[2]) > 0.5) != inn {
                for c in px.iter_mut().take(3) {
                    *c = 1.0 - *c;
                }
            }
            if (px[3] > 0.5) != inn {
                px[3] = 1.0 - px[3];
            }
            v[j * wu + i] = px;
        }
    }
    // error correction: clashing texels take their median, first against the four neighbours, then the diagonals
    let threshold = 1.001 / p.range;
    for pass in 0..2 {
        let mut flagged = Vec::new();
        for j in 0..hu {
            for i in 0..wu {
                let a = v[j * wu + i];
                let a3 = [a[0], a[1], a[2]];
                let at = |x: usize, y: usize| {
                    let q = v[y * wu + x];
                    [q[0], q[1], q[2]]
                };
                let hit = if pass == 0 {
                    (i > 0 && clash(a3, at(i - 1, j), threshold))
                        || (i + 1 < wu && clash(a3, at(i + 1, j), threshold))
                        || (j > 0 && clash(a3, at(i, j - 1), threshold))
                        || (j + 1 < hu && clash(a3, at(i, j + 1), threshold))
                } else {
                    let t2 = 2.0 * threshold;
                    (i > 0 && j > 0 && clash(a3, at(i - 1, j - 1), t2))
                        || (i + 1 < wu && j > 0 && clash(a3, at(i + 1, j - 1), t2))
                        || (i > 0 && j + 1 < hu && clash(a3, at(i - 1, j + 1), t2))
                        || (i + 1 < wu && j + 1 < hu && clash(a3, at(i + 1, j + 1), t2))
                };
                if hit {
                    flagged.push(j * wu + i);
                }
            }
        }
        for k in flagged {
            let q = &mut v[k];
            let m = median(q[0], q[1], q[2]);
            q[0] = m;
            q[1] = m;
            q[2] = m;
        }
    }
    let mut px = Vec::with_capacity(wu * hu * 4);
    for q in &v {
        for &c in q {
            px.push((c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
        }
    }
    Some(Tile { w, h, x0, y0, px })
}

/// the outline as quadratic and cubic segments from lib/text's outline buffer (font units, TrueType contours with their
/// implied on-curve midpoints, CFF contours of cubics), scaled by `s` (1 / units per em: em units)
pub fn contours_of(pts: &[(f32, f32)], tags: &[u8], ends: &[u32], s: f64) -> Vec<Vec<Seg>> {
    let mut out = Vec::new();
    let mut first = 0usize;
    for &e in ends {
        let last = e as usize;
        if last < first || last >= pts.len() {
            first = last.wrapping_add(1);
            continue;
        }
        let p = |i: usize| [pts[i].0 as f64 * s, pts[i].1 as f64 * s];
        let tag = |i: usize| tags.get(i).copied().unwrap_or(1);
        let n = last - first + 1;
        let mut segs = Vec::new();
        let cubic = (first..=last).any(|i| tag(i) == 2);
        if cubic {
            // CFF: an on-curve point first, then per piece two control points and an end (or a line to an on point)
            let mut k = 0usize;
            let start = p(first);
            let mut cur = start;
            while k + 1 < n {
                let i = first + k + 1;
                if tag(i) == 2 && k + 3 < n + 1 {
                    let c1 = p(i);
                    let c2 = p(first + (k + 2) % n);
                    let e = p(first + (k + 3) % n);
                    segs.push(Seg::Cubic(cur, c1, c2, e));
                    cur = e;
                    k += 3;
                } else {
                    let e = p(i);
                    segs.push(Seg::Line(cur, e));
                    cur = e;
                    k += 1;
                }
            }
            if cur != start {
                segs.push(Seg::Line(cur, start));
            }
        } else {
            // TrueType: on and off points, two offs imply an on point between them
            let on = |i: usize| tag(i) & 1 != 0;
            let (start, mut next, mut left);
            if on(first) {
                start = p(first);
                next = first + 1;
                left = n - 1;
            } else if on(last) {
                start = p(last);
                next = first;
                left = n - 1;
            } else {
                start = lerp(p(last), p(first), 0.5);
                next = first;
                left = n;
            }
            let mut cur = start;
            let mut pending: Option<P> = None;
            while left > 0 {
                left -= 1;
                let i = next;
                next += 1;
                let q = p(i);
                if on(i) {
                    match pending.take() {
                        Some(c) => segs.push(Seg::Quad(cur, c, q)),
                        None => segs.push(Seg::Line(cur, q)),
                    }
                    cur = q;
                } else {
                    if let Some(c) = pending {
                        let mid = lerp(c, q, 0.5);
                        segs.push(Seg::Quad(cur, c, mid));
                        cur = mid;
                    }
                    pending = Some(q);
                }
            }
            match pending {
                Some(c) => segs.push(Seg::Quad(cur, c, start)),
                None => {
                    if cur != start {
                        segs.push(Seg::Line(cur, start));
                    }
                }
            }
        }
        // (degenerate segments carry no direction: dropped)
        segs.retain(|s| match *s {
            Seg::Line(a, b) => a != b,
            Seg::Quad(a, b, c) => !(a == b && b == c),
            Seg::Cubic(a, b, c, d) => !(a == b && b == c && c == d),
        });
        if !segs.is_empty() {
            out.push(segs);
        }
        first = last + 1;
    }
    out
}

/// the area the contours fill (non-zero; em units squared), each segment as 16 chords: what a glyph's ink weighs
pub fn ink_area(contours: &[Vec<Seg>]) -> f64 {
    signed_area(contours).abs()
}
