// dash -- dashed strokes with caps (flat / round / square / arrow / filled arrow) as a path:
// the path's flattened centre lines are replaced by the outline of their stroke, closed
// contours to fill with RULE_NONZERO                                          [src/gfx/dash.s]
//
//   dash_pattern(style) -> &[f64]         DASH_* (DASH_SOLID / unknown: empty)
//   dash_stroke(path, &DStyle) -> 0 / ERR_LIMIT
//
// Geometry (f64): every open subpath (or dash piece) becomes ONE contour: the +n side
// forward with joins, the end cap, the -n side backward, the start cap (n = (dy, -dx)).
// Outer joins are mitred (limit miter, else bevelled), round or bevelled; inner joins pivot
// through the vertex (the non-zero fill absorbs the overlap).  A closed subpath becomes two
// contours (outer and inner offset loop, oriented so that the ring winds +1).  Every open
// contour is oriented to a positive area, so overlapping pieces unite under non-zero.
// Dashes: the pattern (entries x width) runs along each subpath from its start (phase
// offset x width), a closed subpath's pattern runs once around; the ends of the pieces get
// dcap except at the path's own start / end (cap0 / cap1).  A piece of length 0 is a dot.
// Arrow caps (open subpaths): head length L = 3 w + 4, half width L / 2 at the end point,
// pointing along the last segment; the open arrow is the polyline (base+, tip, base-)
// stroked with the line's width and join; the filled arrow a triangle, and the line ends
// L / 2 before the tip.  Round arcs use steps of 2 sqrt(0.1 / hw) rad (<= 45 degrees).
// f64 operations in the order of the assembly, so the outline is bit-identical.

use alloc::vec::Vec;
use core::mem;

use super::path::Path;
use super::xform::sincos;
use super::{ERR_LIMIT, JOIN_BEVEL, JOIN_ROUND};
use rt::fx::{cvttd, cvttd64, maxsd, minsd, sqrtd};
use rt::x87::atan2;

pub const LCAP_FLAT: u32 = 0; // = CAP_BUTT
pub const LCAP_ROUND: u32 = 1;
pub const LCAP_SQUARE: u32 = 2;
pub const LCAP_ARROW: u32 = 3; // open arrow head (two strokes of the line width)
pub const LCAP_ARROW_FILLED: u32 = 4; // filled triangle
pub const LCAP_COUNT: u32 = 5;
pub const DASH_SOLID: u32 = 0;
pub const DASH_DASH: u32 = 1; // 3 1
pub const DASH_DOT: u32 = 2; // 1 1
pub const DASH_DASHDOT: u32 = 3; // 3 1 1 1
pub const DASH_DASHDOTDOT: u32 = 4; // 3 1 1 1 1 1
pub const DASH_COUNT: u32 = 5;
pub const DASH_MAXPAT: usize = 32; // entries of a custom pattern

/// a stroke style (the DSTYLE record)
#[derive(Clone, Copy)]
pub struct DStyle<'a> {
    pub width: f64,     // line width (px, >= 0.05)
    pub pat: &'a [f64], // on / off lengths in units of the width (empty = solid; odd counts twice)
    pub join: u32,      // JOIN_*
    pub cap0: u32,      // LCAP_* at the start of every open subpath
    pub cap1: u32,      // LCAP_* at its end
    pub dcap: u32,      // LCAP_FLAT / ROUND / SQUARE at the inner ends of dashes
    pub offset: f64,    // dash phase (units of the width)
    pub miter: f64,     // miter limit (ratio of miter length to width; < 1 = 4)
}

static PAT_DASH: [f64; 2] = [3.0, 1.0];
static PAT_DOT: [f64; 2] = [1.0, 1.0];
static PAT_DASHDOT: [f64; 4] = [3.0, 1.0, 1.0, 1.0];
static PAT_DASHDOTDOT: [f64; 6] = [3.0, 1.0, 1.0, 1.0, 1.0, 1.0];

/// the predefined pattern of style DASH_* (units of the width)  [dash_pattern]
pub fn dash_pattern(style: u32) -> &'static [f64] {
    match style {
        DASH_DASH => &PAT_DASH,
        DASH_DOT => &PAT_DOT,
        DASH_DASHDOT => &PAT_DASHDOT,
        DASH_DASHDOTDOT => &PAT_DASHDOTDOT,
        _ => &[],
    }
}

type P = (f64, f64);

const SIGN: u64 = 0x8000_0000_0000_0000;
const PB_MAX: usize = 0x100_0000; // points per buffer
const TOTAL_MAX: u32 = 0x80_0000; // points emitted per call
const PIECES_MAX: u32 = 0x20_0000; // dash pieces per call
const PI: f64 = 3.141592653589793;
const TWO_PI: f64 = 6.283185307179586;
const PIO4: f64 = 0.7853981633974483;
const EPS2: f64 = 1.0e-12;
const TINY: f64 = 1.0e-12;
const ARCMAX: f64 = 4096.0;
// square dot corners: signs of (d, n)
const DOT_SIGNS: [(i8, i8); 4] = [(1, 1), (1, -1), (-1, -1), (-1, 1)];

#[inline]
fn abs(v: f64) -> f64 {
    f64::from_bits(v.to_bits() & !SIGN)
}

#[inline]
fn neg(v: f64) -> f64 {
    f64::from_bits(v.to_bits() ^ SIGN)
}

// append a point; 0 or the (sticky) error  [pb_push]
fn pb_push(v: &mut Vec<P>, err: &mut i32, p: P) -> i32 {
    if *err != 0 {
        return *err;
    }
    if v.len() >= PB_MAX {
        *err = ERR_LIMIT;
        return ERR_LIMIT;
    }
    v.push(p);
    0
}

// pb_push unless the point repeats the last one (distance^2 <= 1e-12)  [pb_push_nd]
fn pb_push_nd(v: &mut Vec<P>, err: &mut i32, p: P) -> i32 {
    if let Some(&l) = v.last() {
        let dx = p.0 - l.0;
        let dy = p.1 - l.1;
        if !(dx * dx + dy * dy > EPS2) {
            return 0;
        }
    }
    pb_push(v, err, p)
}

// unit(b - a), (1, 0) when a = b  [dc_dir]
fn dir(a: P, b: P) -> P {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let l = sqrtd(dx * dx + dy * dy);
    if l > TINY { (dx / l, dy / l) } else { (1.0, 0.0) }
}

// shoelace sum x_i y_(i+1) - x_(i+1) y_i of the closed polygon
fn area(p: &[P]) -> f64 {
    let n = p.len();
    let mut s = 0.0;
    for i in 0..n {
        let j = if i + 1 < n { i + 1 } else { 0 };
        s = s + (p[i].0 * p[j].1 - p[j].0 * p[i].1);
    }
    s
}

// the context (DC_*)
struct Ctx<'p> {
    w: f64,
    hw: f64,
    step: f64,
    mlim: f64,
    join: u32,
    dcap: u32,
    err: i32,
    npat: usize,
    phase: f64,
    total: u32,
    pieces: u32,
    pc: Vec<P>, // dash piece / arrow legs
    ct: Vec<P>, // contour
    cap0: u32,  // caps of the current subpath body
    cap1: u32,
    pat: [f64; 64],
    path: &'p mut Path,
}

impl Ctx<'_> {
    fn ct_push(&mut self, p: P) -> i32 {
        pb_push(&mut self.ct, &mut self.err, p)
    }

    // CT += P + sign * hw * n(d), n(d) = (dy, -dx)  [dc_offpt]
    fn offpt(&mut self, p: P, d: P, sign: i32) {
        let s = sign as f64 * self.hw;
        let q = (d.1 * s + p.0, p.1 - d.0 * s);
        self.ct_push(q);
    }

    // CT += P + R(k angle / n) v for k = 1 .. n - 1 (n = ceil(|angle| / step))  [dc_arc]
    fn arc(&mut self, p: P, v: P, angle: f64) {
        let q = minsd(abs(angle) / self.step, ARCMAX);
        let mut n = cvttd(q);
        if q > n as f64 {
            n += 1;
        }
        if (n as u32) < 2 {
            return;
        }
        let (s, c) = sincos(angle / n as f64);
        let mut v = v;
        for _ in 1..n {
            v = (v.0 * c - v.1 * s, v.1 * c + v.0 * s);
            if self.ct_push((v.0 + p.0, v.1 + p.1)) != 0 {
                return;
            }
        }
    }

    // the +n side at the vertex P  [dc_join]
    fn join(&mut self, p: P, din: P, dout: P) {
        let cross = din.0 * dout.1 - din.1 * dout.0;
        let dot = din.0 * dout.0 + din.1 * dout.1;
        if abs(cross) > 1e-9 {
            if !(cross > 0.0) {
                // inner side: pivot through the vertex
                self.offpt(p, din, 1);
                self.ct_push(p);
                self.offpt(p, dout, 1);
                return;
            }
        } else if dot > 0.0 {
            self.offpt(p, din, 1); // straight on
            return;
        }
        // outer join (or a U-turn)
        self.offpt(p, din, 1);
        if self.join == JOIN_ROUND {
            let v = (din.1 * self.hw, neg(din.0) * self.hw);
            let mut a = atan2(cross, dot);
            if !(abs(cross) > 1e-9) {
                a = PI;
            }
            self.arc(p, v, a);
        } else if self.join != JOIN_BEVEL {
            // miter: ratio 1 / cos(phi / 2) = sqrt(2 / (1 + dot)) <= limit
            let d1 = dot + 1.0;
            if d1 > 1e-9 && !(sqrtd(2.0 / d1) > self.mlim) {
                let s = self.hw / d1;
                let q = ((din.1 + dout.1) * s + p.0, p.1 - (din.0 + dout.0) * s);
                self.ct_push(q);
            }
        }
        self.offpt(p, dout, 1);
    }

    // CT += the +n offset side of the m >= 2 points p[base], p[base + stride], ... with
    // joins; closed: joins at every vertex  [dc_side]
    fn side(&mut self, p: &[P], base: usize, stride: isize, m: usize, closed: bool) {
        let at = |k: usize| p[(base as isize + k as isize * stride) as usize];
        for k in 0..m {
            if self.err != 0 {
                return;
            }
            let pk = at(k);
            let prev = at(if k == 0 { m } else { k } - 1);
            let next = at(if k + 1 < m { k + 1 } else { 0 });
            if closed || (k != 0 && k != m - 1) {
                self.join(pk, dir(prev, pk), dir(pk, next));
            } else if k == 0 {
                self.offpt(pk, dir(pk, next), 1);
            } else {
                self.offpt(pk, dir(prev, pk), 1);
            }
        }
    }

    // the cap at E (d = outward direction) between the +n and the -n side  [dc_cap]
    fn cap(&mut self, e: P, d: P, cap: u32) {
        if cap == LCAP_SQUARE {
            let q = (d.0 * self.hw + e.0, d.1 * self.hw + e.1);
            self.offpt(q, d, 1);
            self.offpt(q, d, -1);
        } else if cap == LCAP_ROUND {
            let v = (d.1 * self.hw, neg(d.0) * self.hw);
            self.arc(e, v, PI);
        }
    }

    // CT -> the path as one closed contour (reversed when normalize and its area is
    // negative); CT emptied  [dc_flush]
    fn flush(&mut self, normalize: bool) {
        let ct = mem::take(&mut self.ct);
        let n = ct.len();
        if n >= 3 && self.err == 0 {
            self.total = self.total.wrapping_add(n as u32);
            if self.total > TOTAL_MAX {
                self.err = ERR_LIMIT;
            } else {
                let rev = normalize && !(area(&ct) >= 0.0);
                for i in 0..n {
                    let q = ct[if rev { n - 1 - i } else { i }];
                    let x = minsd(maxsd(q.0, -4.0e6), 4.0e6) as f32;
                    let y = minsd(maxsd(q.1, -4.0e6), 4.0e6) as f32;
                    if i == 0 {
                        self.path.move_f(x, y);
                    } else {
                        self.path.line_f(x, y);
                    }
                }
                self.path.close();
            }
        }
        self.ct = ct;
        self.ct.clear();
    }

    // one contour for the open polyline p (m >= 2 points)  [dc_open]
    fn open(&mut self, p: &[P], cap0: u32, cap1: u32) {
        self.ct.clear();
        let m = p.len();
        if m < 2 {
            return;
        }
        self.side(p, 0, 1, m, false);
        let d = dir(p[m - 2], p[m - 1]);
        self.cap(p[m - 1], d, cap1);
        self.side(p, m - 1, -1, m, false);
        let d = dir(p[1], p[0]);
        self.cap(p[0], d, cap0);
        self.flush(true);
    }

    // a closed polyline (m >= 2): outer + inner offset loop, positive ring  [dc_ring]
    fn ring(&mut self, p: &[P]) {
        let m = p.len();
        if m < 2 {
            return;
        }
        let (mut fb, mut fs, mut bb, mut bs) = (0usize, 1isize, m - 1, -1isize);
        if !(area(p) >= 0.0) {
            mem::swap(&mut fb, &mut bb);
            mem::swap(&mut fs, &mut bs);
        }
        self.ct.clear();
        self.side(p, fb, fs, m, true);
        self.flush(false);
        self.side(p, bb, bs, m, true);
        self.flush(false);
    }

    // a zero-length piece: circle (round) or square (square cap)  [dc_dot]
    fn dot(&mut self, p: P, d: P, cap: u32) {
        self.ct.clear();
        if cap == LCAP_ROUND {
            self.ct_push((self.hw + p.0, p.1));
            self.arc(p, (self.hw, 0.0), TWO_PI);
        } else if cap == LCAP_SQUARE {
            for &(sd, sn) in DOT_SIGNS.iter() {
                let a = sd as f64 * self.hw;
                let b = sn as f64 * self.hw;
                let q = (d.0 * a + d.1 * b + p.0, d.1 * a - d.0 * b + p.1);
                self.ct_push(q);
            }
        } else {
            return;
        }
        self.flush(true);
    }

    // stroke the dash piece in PC (1 point: a dot)  [dc_piece]
    fn piece(&mut self, caps: u32, cape: u32, d: P) {
        self.pieces += 1;
        if self.pieces > PIECES_MAX {
            if self.err == 0 {
                self.err = ERR_LIMIT;
            }
            return;
        }
        match self.pc.len() {
            0 => {}
            1 => {
                let cap = if caps == LCAP_ROUND || caps == LCAP_SQUARE { caps } else { cape };
                let q = self.pc[0];
                self.dot(q, d, cap);
            }
            _ => {
                let pc = mem::take(&mut self.pc);
                self.open(&pc, caps, cape);
                self.pc = pc;
            }
        }
    }

    // the dash pieces of the polyline p (caps cap0 / cap1 at its ends, dcap inside;
    // closed: the pattern runs once around)  [dc_dash]
    fn dash(&mut self, p: &[P], closed: bool) {
        let m = p.len();
        let npat = self.npat;
        let segs = if closed { m } else { m - 1 };
        // the phase: skip whole entries
        let mut x = self.phase;
        let mut k = 0usize;
        let mut guard = 0usize;
        loop {
            let e = self.pat[k];
            if !(x >= e) {
                break;
            }
            guard += 1;
            if guard > npat * 2 {
                break;
            }
            x = x - e;
            k += 1;
            if k >= npat {
                k = 0;
            }
        }
        let mut rem = maxsd(self.pat[k] - x, 0.0);
        let mut on = k & 1 == 0;
        let mut at_start = true;
        self.pc.clear();
        let mut d: P = (1.0, 0.0);
        if on {
            pb_push(&mut self.pc, &mut self.err, p[0]);
        }
        for s in 0..segs {
            if self.err != 0 {
                return;
            }
            let a = p[s];
            let b = p[if s + 1 < m { s + 1 } else { 0 }];
            d = dir(a, b);
            let dx = b.0 - a.0;
            let dy = b.1 - a.1;
            let len = sqrtd(dx * dx + dy * dy);
            let mut t = 0.0;
            // does the current entry end inside this segment?
            while len - t > rem {
                t = t + rem;
                let q = (d.0 * t + a.0, d.1 * t + a.1);
                if on {
                    pb_push_nd(&mut self.pc, &mut self.err, q);
                    let caps = if at_start { self.cap0 } else { self.dcap };
                    self.piece(caps, self.dcap, d);
                    self.pc.clear();
                } else {
                    self.pc.clear();
                    pb_push(&mut self.pc, &mut self.err, q);
                }
                at_start = false;
                on = !on;
                k = if k + 1 < npat { k + 1 } else { 0 };
                rem = self.pat[k & 63];
                if self.err != 0 {
                    return;
                }
            }
            rem = rem - (len - t);
            if on {
                pb_push_nd(&mut self.pc, &mut self.err, b);
            }
        }
        if on {
            let caps = if at_start { self.cap0 } else { self.dcap };
            let cape = if closed { self.dcap } else { self.cap1 };
            self.piece(caps, cape, d);
        }
    }

    // stroke the line ln (solid or dashed)  [dc_body]
    fn body(&mut self, ln: &[P], closed: bool) {
        if ln.len() < 2 {
            return;
        }
        if self.npat == 0 {
            if closed {
                self.ring(ln);
            } else {
                let (c0, c1) = (self.cap0, self.cap1);
                self.open(ln, c0, c1);
            }
        } else {
            self.dash(ln, closed);
        }
    }

    // L / 2 = (3 w + 4) / 2  [dc_headtrim]
    fn headtrim(&self) -> f64 {
        (self.w * 3.0 + 4.0) * 0.5
    }

    // arrow head at tip pointing along d  [dc_arrow]
    fn arrow(&mut self, tip: P, d: P, filled: bool) {
        let l = self.w * 3.0 + 4.0;
        let h = l * 0.5;
        let bx = tip.0 - d.0 * l;
        let by = tip.1 - d.1 * l;
        let nx = d.1 * h;
        let ny = neg(d.0 * h);
        let left = (bx + nx, by + ny);
        let right = (bx - nx, by - ny);
        let buf = if filled { &mut self.ct } else { &mut self.pc };
        buf.clear();
        for q in [left, tip, right] {
            pb_push(buf, &mut self.err, q);
        }
        if filled {
            self.flush(true);
        } else {
            let pc = mem::take(&mut self.pc);
            self.open(&pc, LCAP_FLAT, LCAP_FLAT);
            self.pc = pc;
        }
    }
}

// shorten the line by dist at its start (end false) or its end (end true); fewer than 2
// points left: emptied  [dc_trim]
fn trim(ln: &mut Vec<P>, end: bool, dist: f64) {
    let n = ln.len();
    let mut dist = dist;
    if n < 2 {
        ln.clear();
        return;
    }
    if !end {
        let mut i = 0;
        while i + 1 < n {
            let dx = ln[i + 1].0 - ln[i].0;
            let dy = ln[i + 1].1 - ln[i].1;
            let l = sqrtd(dx * dx + dy * dy);
            if l > dist {
                let f = dist / l;
                ln[i] = (dx * f + ln[i].0, dy * f + ln[i].1);
                ln.drain(..i);
                return;
            }
            dist = dist - l;
            i += 1;
        }
    } else {
        let mut j = n - 1;
        while j > 0 {
            let dx = ln[j - 1].0 - ln[j].0;
            let dy = ln[j - 1].1 - ln[j].1;
            let l = sqrtd(dx * dx + dy * dy);
            if l > dist {
                let f = dist / l;
                ln[j] = (dx * f + ln[j].0, dy * f + ln[j].1);
                ln.truncate(j + 1);
                return;
            }
            dist = dist - l;
            j -= 1;
        }
    }
    ln.clear();
}

/// replace the path by the outline of its stroke (see the file header)  [dash_stroke]
pub fn dash_stroke(path: &mut Path, st: &DStyle) -> i32 {
    // width (>= 0.05, <= 1e6), half width, arc step, miter limit, joins, caps
    let w = minsd(maxsd(st.width, 0.05), 1.0e6);
    let hw = w * 0.5;
    let s = sqrtd(0.1 / hw);
    let step = maxsd(minsd(s + s, PIO4), 0.005);
    let mlim = minsd(if st.miter >= 1.0 { st.miter } else { 4.0 }, 1.0e6);
    let join = if st.join > JOIN_BEVEL { 0 } else { st.join };
    let dcap = if st.dcap > LCAP_SQUARE { 0 } else { st.dcap };
    let saved_pts = mem::take(&mut path.pts);
    let saved_subs = mem::take(&mut path.subs);
    let mut c = Ctx {
        w,
        hw,
        step,
        mlim,
        join,
        dcap,
        err: 0,
        npat: 0,
        phase: 0.0,
        total: 0,
        pieces: 0,
        pc: Vec::new(),
        ct: Vec::new(),
        cap0: 0,
        cap1: 0,
        pat: [0.0; 64],
        path,
    };
    // the dash pattern (px), odd counts twice, period >= 0.25 px else solid
    let np = st.pat.len();
    if np != 0 && np <= DASH_MAXPAT {
        let mut sum = 0.0;
        for i in 0..np {
            let mut v = minsd(maxsd(st.pat[i], 0.0), 1.0e6) * w;
            if !(v >= 0.0001) {
                v = 0.0;
            }
            c.pat[i] = v;
            sum = sum + v;
        }
        let mut n = np;
        if np & 1 != 0 {
            for i in 0..np {
                c.pat[i + np] = c.pat[i];
            }
            sum = sum + sum;
            n += n;
        }
        if sum >= 0.25 {
            c.npat = n;
            // phase = offset * w mod period
            let o = minsd(maxsd(st.offset, -1.0e6), 1.0e6) * w;
            let q = o / sum;
            let mut f = cvttd64(q);
            let mut fd = f as f64;
            if fd > q {
                f -= 1;
                fd = f as f64;
            }
            c.phase = maxsd(o - fd * sum, 0.0);
        }
    }
    c.path.reset();
    // every subpath of the (saved) path
    let mut ln: Vec<P> = Vec::new();
    for sub in saved_subs.iter() {
        if c.err != 0 {
            break;
        }
        // the points (f64), consecutive duplicates dropped
        ln.clear();
        for i in 0..sub.count {
            let idx = sub.start + i;
            if idx >= saved_pts.len() {
                break;
            }
            let q = saved_pts[idx];
            pb_push_nd(&mut ln, &mut c.err, (q.0 as f64, q.1 as f64));
        }
        let mut n = ln.len();
        if n == 0 {
            continue;
        }
        if sub.closed {
            // drop last points repeating the first
            while n >= 2 {
                let dx = ln[n - 1].0 - ln[0].0;
                let dy = ln[n - 1].1 - ln[0].1;
                if dx * dx + dy * dy > EPS2 {
                    break;
                }
                n -= 1;
            }
            ln.truncate(n);
            if n != 1 {
                c.cap0 = c.dcap;
                c.cap1 = c.dcap;
                c.body(&ln, true);
                continue;
            }
        }
        if n == 1 {
            // a single point: a dot with the start cap (round / square), else the end cap
            let cap = if st.cap0 == LCAP_ROUND || st.cap0 == LCAP_SQUARE { st.cap0 } else { st.cap1 };
            c.dot(ln[0], (1.0, 0.0), cap);
            continue;
        }
        let c0 = if st.cap0 < LCAP_COUNT { st.cap0 } else { 0 };
        let c1 = if st.cap1 < LCAP_COUNT { st.cap1 } else { 0 };
        // arrow tips and directions from the untrimmed line
        let tip0 = ln[0];
        let d0 = dir(ln[1], ln[0]);
        let tip1 = ln[n - 1];
        let d1 = dir(ln[n - 2], ln[n - 1]);
        // filled arrows: the line ends in the middle of the triangle (L / 2 before the tip)
        if c0 == LCAP_ARROW_FILLED {
            trim(&mut ln, false, c.headtrim());
        }
        if c1 == LCAP_ARROW_FILLED {
            trim(&mut ln, true, c.headtrim());
        }
        c.cap0 = if c0 >= LCAP_ARROW { 0 } else { c0 };
        c.cap1 = if c1 >= LCAP_ARROW { 0 } else { c1 };
        c.body(&ln, false);
        if c0 >= LCAP_ARROW {
            c.arrow(tip0, d0, c0 == LCAP_ARROW_FILLED);
        }
        if c1 >= LCAP_ARROW {
            c.arrow(tip1, d1, c1 == LCAP_ARROW_FILLED);
        }
    }
    c.err
}
