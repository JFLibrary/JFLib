// curve -- curves on the CPU (decision 25; docs/RENDER2D.md "Kurven"): Bézier chains, ellipses and splines (B-splines up
// to degree 3, rational ones included) as rational Bézier spans in f64, flattened into segments per zoom band within
// FLAT_TOL px; where the whole curve would take too many segments, only what lies in view plus a margin is flattened
// finely, the rest as the chords of whole sub-spans (they lie in their control hulls, which lie outside the window).
//
//   Curve::bezier / ellipse / spline / polyline, ring    the spans (homogeneous control points relative to an origin)
//   flatten(&curve, tol, window, max_leaf, budget)         the points, each with its length along the curve (for dashes)
//
// A span is subdivided at t = 1/2 until its control polygon lies within the tolerance of its chord (the factor of the
// bound: a quadratic's curve keeps half its control point's distance, a cubic's three quarters, a rational quadratic
// w / (1 + w) in standard form, any other rational one the whole hull); the subdivision depends on the curve and the
// tolerance only, so a window keeps the vertices a full flattening would have.  A vertex is evaluated from the span
// itself at its dyadic parameter (repeated halving would add its roundings up).  The length at a vertex (for dash
// patterns) is the length along the chords, as for a polyline; where a windowed curve's fine part begins, the curve's
// arc length there (adaptive Gauss-Legendre on the spans), so that the pattern stays put when the window moves.

use alloc::vec::Vec;

use crate::math::{sincos, sqrt};

/// the flattening's tolerance at the finest scale of a zoom band (px): a chord lies at most this far inside its curve
/// (2/3 of it on average); within a band and its hysteresis at most HYST times it (docs/RENDER2D.md "Kurven")
pub const FLAT_TOL: f64 = 1.0 / 32.0;
/// a curve is flattened again when the scale leaves its band [2^b, 2^(b+1)) by more than this factor
pub const HYST: f64 = 1.189_207_115_002_721;
/// a curve whose whole flattening takes more segments than this is flattened in view plus a margin only
pub const FULL_MAX: usize = 4096;
/// the most segments of one curve (beyond: a coarser tolerance)
pub const CURVE_MAX: usize = 16384;

/// a rational Bézier span of degree 1 to 3: homogeneous control points (w x, w y, w) relative to the curve's origin
#[derive(Clone, Copy, Debug)]
pub struct Span {
    pub deg: u8,
    pub p: [[f64; 3]; 4],
}

/// a curve: its origin (a point of it, so that the spans' numbers stay small), its spans end to end, closed (its end
/// joins its start)
#[derive(Clone, Debug)]
pub struct Curve {
    pub origin: [f64; 2],
    pub spans: Vec<Span>,
    pub closed: bool,
}

/// the flattened curve: points (absolute), the arc length at each from the curve's start, and for a closed one the
/// length all around (where the closing segment ends)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Flat {
    pub pts: Vec<[f64; 2]>,
    pub lens: Vec<f64>,
    pub total: f64,
    pub closed: bool,
}

fn lerp3(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn h(p: [f64; 2], w: f64) -> [f64; 3] {
    [p[0] * w, p[1] * w, w]
}

impl Span {
    pub fn line(a: [f64; 2], b: [f64; 2]) -> Span {
        Span { deg: 1, p: [h(a, 1.0), h(b, 1.0), [0.0; 3], [0.0; 3]] }
    }

    fn proj(&self, k: usize) -> [f64; 2] {
        let q = self.p[k];
        [q[0] / q[2], q[1] / q[2]]
    }

    /// the point at t and the homogeneous point and derivative (de Casteljau)
    fn eval_h(&self, t: f64) -> ([f64; 3], [f64; 3]) {
        let n = self.deg as usize;
        let mut q = self.p;
        for lvl in (1..n).rev() {
            for k in 0..lvl + 1 {
                q[k] = lerp3(q[k], q[k + 1], t);
            }
        }
        // q[0], q[1]: the last level's two points
        let pt = lerp3(q[0], q[1], t);
        let d = [(q[1][0] - q[0][0]) * n as f64, (q[1][1] - q[0][1]) * n as f64, (q[1][2] - q[0][2]) * n as f64];
        (pt, d)
    }

    pub fn eval(&self, t: f64) -> [f64; 2] {
        if t <= 0.0 {
            return self.proj(0);
        }
        if t >= 1.0 {
            return self.proj(self.deg as usize);
        }
        let (p, _) = self.eval_h(t);
        [p[0] / p[2], p[1] / p[2]]
    }

    /// |C'(t)|
    fn speed(&self, t: f64) -> f64 {
        let (p, d) = self.eval_h(t);
        let w = p[2];
        let x = (d[0] * w - p[0] * d[2]) / (w * w);
        let y = (d[1] * w - p[1] * d[2]) / (w * w);
        sqrt(x * x + y * y)
    }

    /// the halves at t = 1/2
    fn split(&self) -> (Span, Span) {
        let n = self.deg as usize;
        let mut l = *self;
        let mut r = *self;
        let mut q = self.p;
        l.p[0] = q[0];
        r.p[n] = q[n];
        for lvl in 1..n + 1 {
            for k in 0..n + 1 - lvl {
                q[k] = lerp3(q[k], q[k + 1], 0.5);
            }
            l.p[lvl] = q[0];
            r.p[n - lvl] = q[n - lvl];
        }
        (l, r)
    }

    /// within tol of its chord: the bound of the curve's distance from the chord (see the head)
    fn flat(&self, tol: f64) -> bool {
        let n = self.deg as usize;
        if n == 1 {
            return true;
        }
        let a = self.proj(0);
        let b = self.proj(n);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let ll = dx * dx + dy * dy;
        let mut d2 = 0.0f64;
        for k in 1..n {
            let p = self.proj(k);
            let (px, py) = (p[0] - a[0], p[1] - a[1]);
            // the distance from the chord as a segment
            let t = if ll > 0.0 { ((px * dx + py * dy) / ll).clamp(0.0, 1.0) } else { 0.0 };
            let (ex, ey) = (px - t * dx, py - t * dy);
            d2 = d2.max(ex * ex + ey * ey);
        }
        let w0 = self.p[0][2];
        let poly = (1..n + 1).all(|k| (self.p[k][2] - w0).abs() <= 1e-12 * w0.abs());
        let f = if poly {
            if n == 2 { 0.5 } else { 0.75 }
        } else if n == 2 {
            // standard form: w1 / sqrt(w0 w2)
            let w = self.p[1][2] / sqrt(self.p[0][2] * self.p[2][2]);
            w / (1.0 + w)
        } else {
            1.0
        };
        d2 * f * f <= tol * tol
    }

    /// the box of the projected control points (the curve lies inside: positive weights)
    fn hull(&self) -> [f64; 4] {
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for k in 0..self.deg as usize + 1 {
            let p = self.proj(k);
            b[0] = b[0].min(p[0]);
            b[1] = b[1].min(p[1]);
            b[2] = b[2].max(p[0]);
            b[3] = b[3].max(p[1]);
        }
        b
    }

    /// the arc length from ta to tb (adaptive Gauss-Legendre, to about 1e-14 relative)
    fn length(&self, ta: f64, tb: f64) -> f64 {
        if tb <= ta {
            return 0.0;
        }
        if self.deg == 1 {
            let (a, b) = (self.eval(ta), self.eval(tb));
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            return sqrt(dx * dx + dy * dy);
        }
        let whole = self.gl(ta, tb);
        self.adapt(ta, tb, whole, 0)
    }

    fn gl(&self, a: f64, b: f64) -> f64 {
        // 8 points
        const X: [f64; 4] = [0.183_434_642_495_649_8, 0.525_532_409_916_329, 0.796_666_477_413_626_7, 0.960_289_856_497_536_3];
        const W: [f64; 4] = [0.362_683_783_378_362, 0.313_706_645_877_887_3, 0.222_381_034_453_374_5, 0.101_228_536_290_376_3];
        let (m, r) = (0.5 * (a + b), 0.5 * (b - a));
        let mut s = 0.0;
        for k in 0..4 {
            s += W[k] * (self.speed(m - r * X[k]) + self.speed(m + r * X[k]));
        }
        s * r
    }

    fn adapt(&self, a: f64, b: f64, whole: f64, depth: u32) -> f64 {
        let m = 0.5 * (a + b);
        let (l, r) = (self.gl(a, m), self.gl(m, b));
        if depth >= 40 || (l + r - whole).abs() <= 1e-14 * (l + r).abs() + 1e-300 {
            return l + r;
        }
        self.adapt(a, m, l, depth + 1) + self.adapt(m, b, r, depth + 1)
    }
}

impl Curve {
    fn new(origin: [f64; 2]) -> Curve {
        Curve { origin, spans: Vec::new(), closed: false }
    }

    fn rel(&self, p: [f64; 2]) -> [f64; 2] {
        [p[0] - self.origin[0], p[1] - self.origin[1]]
    }

    /// the curve's first and last point (relative)
    fn ends(&self) -> Option<([f64; 2], [f64; 2])> {
        let a = self.spans.first()?;
        let b = self.spans.last()?;
        Some((a.proj(0), b.proj(b.deg as usize)))
    }

    /// closed: a straight segment from the end back to the start where they differ
    fn close(&mut self, closed: bool) {
        if closed {
            if let Some((a, b)) = self.ends() {
                if a != b {
                    self.spans.push(Span::line(b, a));
                }
            }
        }
        self.closed = closed;
    }

    /// a chain of Bézier pieces: the start, then per piece its control points and its end (degree 2 or 3)
    pub fn bezier(points: &[[f64; 2]], deg: usize, closed: bool) -> Curve {
        let mut c = Curve::new(points[0]);
        let mut k = 0;
        while k + deg < points.len() {
            let mut s = Span { deg: deg as u8, p: [[0.0; 3]; 4] };
            for j in 0..deg + 1 {
                s.p[j] = h(c.rel(points[k + j]), 1.0);
            }
            c.spans.push(s);
            k += deg;
        }
        c.close(closed);
        c
    }

    /// straight segments through the points (fills' rings; repeated points count once)
    pub fn polyline(points: &[[f64; 2]], closed: bool) -> Curve {
        let mut c = Curve::new(points[0]);
        for w in points.windows(2) {
            if w[0] != w[1] {
                c.spans.push(Span::line(c.rel(w[0]), c.rel(w[1])));
            }
        }
        if c.spans.is_empty() {
            // (a single point: a span of length 0)
            c.spans.push(Span::line([0.0; 2], [0.0; 2]));
        }
        c.close(closed);
        c
    }

    /// an elliptic arc: the point at the parameter t is centre + R(rotation) (rx cos t, ry sin t); a sweep of a full turn
    /// or more is closed.  Rational quadratic spans of at most a quarter turn: exact
    pub fn ellipse(center: [f64; 2], radii: [f64; 2], rotation: f64, start: f64, sweep: f64) -> Curve {
        let tau = core::f64::consts::TAU;
        let full = sweep.abs() >= tau;
        let sweep = if full { tau.copysign(sweep) } else { sweep };
        let n = ((sweep.abs() / core::f64::consts::FRAC_PI_2 - 1e-9).max(0.0) as usize + 1).min(4);
        let (sr, cr) = sincos(rotation);
        let map = |u: [f64; 2]| -> [f64; 2] {
            let (x, y) = (radii[0] * u[0], radii[1] * u[1]);
            [cr * x - sr * y, sr * x + cr * y]
        };
        let (s0, c0) = sincos(start);
        let first = map([c0, s0]);
        let mut c = Curve::new([center[0] + first[0], center[1] + first[1]]);
        let phi = sweep / n as f64;
        let (_, cw) = sincos(0.5 * phi);
        let mut prev = [0.0, 0.0];
        for k in 0..n {
            let a = start + phi * k as f64;
            let (sm, cm) = sincos(a + 0.5 * phi);
            let end = if full && k + 1 == n {
                [0.0, 0.0]
            } else {
                let (se, ce) = sincos(a + phi);
                let e = map([ce, se]);
                [e[0] - first[0], e[1] - first[1]]
            };
            let m = map([cm / cw, sm / cw]);
            let mid = [m[0] - first[0], m[1] - first[1]];
            c.spans.push(Span { deg: 2, p: [h(prev, 1.0), h(mid, cw), h(end, 1.0), [0.0; 3]] });
            prev = end;
        }
        c.closed = full;
        c
    }

    /// a B-spline of degree 1 to 3, rational with weights (empty: all 1), knots (empty: clamped uniform, or periodic
    /// uniform when closed), as Bézier spans by knot insertion (Boehm).  closed: its end joins its start (a straight segment
    /// where they differ).  The caller has checked the numbers (Scene::geom)
    pub fn spline(deg: usize, points: &[[f64; 2]], weights: &[f64], knots: &[f64], closed: bool) -> Curve {
        let origin = points[0];
        let w = |k: usize| if weights.is_empty() { 1.0 } else { weights[k] };
        let mut cp: Vec<[f64; 3]> = (0..points.len()).map(|k| h([points[k][0] - origin[0], points[k][1] - origin[1]], w(k))).collect();
        let mut u: Vec<f64> = knots.to_vec();
        let periodic = knots.is_empty() && closed;
        if knots.is_empty() {
            if periodic {
                for k in 0..deg {
                    cp.push(cp[k]);
                }
                u = (0..cp.len() + deg + 1).map(|k| k as f64).collect();
            } else {
                let n = cp.len();
                u = Vec::with_capacity(n + deg + 1);
                for _ in 0..deg + 1 {
                    u.push(0.0);
                }
                for k in 1..n - deg {
                    u.push(k as f64);
                }
                for _ in 0..deg + 1 {
                    u.push((n - deg) as f64);
                }
            }
        }
        let n = cp.len();
        let (lo, hi) = (u[deg], u[n]);
        // every knot value of the domain to multiplicity deg
        let mut vals: Vec<f64> = u.iter().copied().filter(|&v| v >= lo && v <= hi).collect();
        vals.dedup();
        for v in vals {
            loop {
                let mult = u.iter().filter(|&&x| x == v).count();
                if mult >= deg {
                    break;
                }
                // Boehm: v in the span [u[k], u[k + 1]): k the last index with u[k] <= v, at the domain's end the last with
                // u[k] < v (the span it ends); within the points
                let k = if v < hi { (0..cp.len()).rev().find(|&k| u[k] <= v) } else { (0..cp.len()).rev().find(|&k| u[k] < v) };
                let Some(k) = k.filter(|&k| k >= deg) else { break };
                let mut q = Vec::with_capacity(cp.len() + 1);
                for i in 0..cp.len() + 1 {
                    if i + deg <= k {
                        q.push(cp[i]);
                    } else if i > k {
                        q.push(cp[i - 1]);
                    } else {
                        let den = u[i + deg] - u[i];
                        let a = if den > 0.0 { (v - u[i]) / den } else { 0.0 };
                        q.push(lerp3(cp[i - 1], cp[i], a));
                    }
                }
                cp = q;
                u.insert(k + 1, v);
            }
        }
        let mut c = Curve::new(origin);
        for k in deg..cp.len() {
            if u[k] < u[k + 1] && u[k] >= lo && u[k + 1] <= hi {
                let mut s = Span { deg: deg as u8, p: [[0.0; 3]; 4] };
                for j in 0..deg + 1 {
                    s.p[j] = cp[k - deg + j];
                }
                c.spans.push(s);
            }
        }
        if c.spans.is_empty() {
            c.spans.push(Span::line([0.0; 2], [0.0; 2]));
        }
        // (the curve's start is the origin only when the spline passes through its first control point)
        if let Some((a, _)) = c.ends() {
            let o = [origin[0] + a[0], origin[1] + a[1]];
            for s in &mut c.spans {
                for k in 0..s.deg as usize + 1 {
                    let ww = s.p[k][2];
                    s.p[k][0] -= a[0] * ww;
                    s.p[k][1] -= a[1] * ww;
                }
            }
            c.origin = o;
        }
        if periodic {
            // (exactly closed: the last span's end in its start's bits)
            if let Some(last) = c.spans.last_mut() {
                let d = last.deg as usize;
                last.p[d] = [0.0, 0.0, last.p[d][2]];
            }
            c.closed = true;
        } else {
            c.close(closed);
        }
        c
    }

    /// a fill's ring: closed, a straight segment from the end back to the start where they differ
    pub fn ring(mut self) -> Curve {
        self.closed = false;
        self.close(true);
        self
    }

    /// the curve's box (its control hulls), absolute
    pub fn bounds(&self) -> [f64; 4] {
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for s in &self.spans {
            let hb = s.hull();
            b[0] = b[0].min(hb[0]);
            b[1] = b[1].min(hb[1]);
            b[2] = b[2].max(hb[2]);
            b[3] = b[3].max(hb[3]);
        }
        [b[0] + self.origin[0], b[1] + self.origin[1], b[2] + self.origin[0], b[3] + self.origin[1]]
    }

    /// appends another curve's spans (a ring of a fill made of pieces): moved to this origin, joined by a straight segment
    /// where the other does not start at this one's end
    pub fn append(&mut self, o: &Curve) {
        let (dx, dy) = (o.origin[0] - self.origin[0], o.origin[1] - self.origin[1]);
        let end = self.ends().map(|e| e.1);
        let mut first = true;
        for s in &o.spans {
            let mut s = *s;
            for k in 0..s.deg as usize + 1 {
                let w = s.p[k][2];
                s.p[k][0] += dx * w;
                s.p[k][1] += dy * w;
            }
            if first {
                if let Some(e) = end {
                    let a = s.proj(0);
                    if a != e {
                        self.spans.push(Span::line(e, a));
                    }
                }
                first = false;
            }
            self.spans.push(s);
        }
    }
}

/// what flatten keeps track of
struct Run<'a> {
    c: &'a Curve,
    tol: f64,
    /// relative to the origin
    win: Option<[f64; 4]>,
    max_leaf: f64,
    budget: usize,
    out: &'a mut Flat,
    /// the last vertex's span and parameter, the curve's arc length there (windowed only), the length along the chords
    last: (usize, f64),
    arc: f64,
    len: f64,
    /// the last vertex closed a chord outside the window
    coarse: bool,
    over: bool,
}

impl Run<'_> {
    /// a vertex at t of span si; coarse: it ends a chord wholly outside the window.  The length at a vertex is the length
    /// along the chords (both segments at a vertex then agree on where a dash ends, to the bit); a windowed curve's fine
    /// run starts at the curve's arc length there instead (the coarse chords outside are far shorter than the curve), so
    /// that its pattern stays where it was when the window moves
    fn emit(&mut self, si: usize, t: f64, coarse: bool) {
        if self.out.pts.len() >= self.budget {
            self.over = true;
            return;
        }
        // (the arc length at the vertex before, then at this one)
        let arc0 = self.arc;
        if self.win.is_some() {
            let (ls, lt) = self.last;
            if ls == si {
                self.arc += self.c.spans[si].length(lt, t);
            } else {
                self.arc += self.c.spans[ls].length(lt, 1.0);
                for k in ls + 1..si {
                    self.arc += self.c.spans[k].length(0.0, 1.0);
                }
                self.arc += self.c.spans[si].length(0.0, t);
            }
        }
        self.last = (si, t);
        let p = self.c.spans[si].eval(t);
        let q = [self.c.origin[0] + p[0], self.c.origin[1] + p[1]];
        if self.out.pts.last() == Some(&q) {
            return;
        }
        if let Some(&prev) = self.out.pts.last() {
            if self.win.is_some() && self.coarse && !coarse {
                // (a fine run begins after a chord outside the window: its first vertex at the curve's length)
                let n = self.out.lens.len();
                self.out.lens[n - 1] = arc0;
                self.len = arc0;
            }
            let (dx, dy) = (q[0] - prev[0], q[1] - prev[1]);
            self.len += sqrt(dx * dx + dy * dy);
        }
        self.coarse = coarse;
        self.out.pts.push(q);
        self.out.lens.push(self.len);
    }

    fn rec(&mut self, si: usize, s: &Span, t0: f64, t1: f64, depth: u32) {
        if self.over {
            return;
        }
        if let Some(w) = self.win {
            let b = s.hull();
            let m = self.tol;
            if b[2] + m < w[0] || b[0] - m > w[2] || b[3] + m < w[1] || b[1] - m > w[3] {
                // (wholly outside the window: its chord, which lies in its hull)
                self.emit(si, t1, true);
                return;
            }
        }
        let leaf_ok = self.win.is_none() || {
            let (a, b) = (s.proj(0), s.proj(s.deg as usize));
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            dx * dx + dy * dy <= self.max_leaf * self.max_leaf
        };
        if depth >= 48 || (s.flat(self.tol) && leaf_ok) {
            self.emit(si, t1, false);
            return;
        }
        let (l, r) = s.split();
        let tm = 0.5 * (t0 + t1);
        self.rec(si, &l, t0, tm, depth + 1);
        self.rec(si, &r, tm, t1, depth + 1);
    }
}

/// the curve's points within tol (drawing units) of it; window (absolute, None: all of it): finely only where the spans
/// meet it, max_leaf the longest chord there; None when it takes more than budget points
pub fn flatten(c: &Curve, tol: f64, window: Option<[f64; 4]>, max_leaf: f64, budget: usize) -> Option<Flat> {
    let mut out = Flat { pts: Vec::new(), lens: Vec::new(), total: 0.0, closed: c.closed };
    let win = window.map(|w| [w[0] - c.origin[0], w[1] - c.origin[1], w[2] - c.origin[0], w[3] - c.origin[1]]);
    let mut run = Run { c, tol, win, max_leaf, budget, out: &mut out, last: (0, 0.0), arc: 0.0, len: 0.0, coarse: true, over: false };
    run.emit(0, 0.0, true);
    for (si, s) in c.spans.iter().enumerate() {
        run.rec(si, s, 0.0, 1.0, 0);
        if run.over {
            return None;
        }
    }
    if c.closed && out.pts.len() >= 2 {
        // the last point is the first one again: dropped, its length the length all around
        let n = out.pts.len();
        let first = out.pts[0];
        if out.pts[n - 1] == first {
            out.pts.pop();
            out.total = out.lens.pop().unwrap_or(0.0);
        } else {
            out.total = out.lens[n - 1] + {
                let (a, b) = (out.pts[n - 1], first);
                let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
                sqrt(dx * dx + dy * dy)
            };
        }
    } else {
        out.total = out.lens.last().copied().unwrap_or(0.0);
    }
    Some(out)
}

/// floor(log2(x)) for x > 0 (normal numbers), from its exponent
pub fn band_of(x: f64) -> i32 {
    ((x.to_bits() >> 52) & 0x7ff) as i32 - 1023
}

/// 2^b
pub fn pow2(b: i32) -> f64 {
    f64::from_bits(((1023 + b.clamp(-1022, 1023)) as u64) << 52)
}
