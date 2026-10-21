// pathdata -- path data: a growable list of absolute path commands, the SVG / XAML path
// mini-language and its output into a Path                                 [src/gfx/pathdata.s]
//
//   PData::new() / clear()             empty (rule non-zero, error cleared)
//   move_to(x, y), line_to(x, y)       (an implicit move to the current point opens a subpath)
//   cubic(&[f64; 6]), quad(&[f64; 4])  (a quadratic Bezier is stored as the cubic)
//   close()
//   arc(&[f64; 9])                     SVG elliptical arc {x1, y1, rx, ry, rotation (deg),
//                                      large-arc 0/1, sweep 0/1, x2, y2} as up to 4 cubics
//   rrect(&[f64; 5])                   closed rounded rectangle {x0, y0, x1, y1, r}
//   parse(text) -> (0 / PDE_* / ERR_LIMIT, byte offset)   M L H V C S Q T A Z (upper case
//                                      absolute, lower case relative), implicit repetition,
//                                      "F0" / "F1" (XAML even-odd / non-zero)
//   emit(path, Option<&Matrix>) -> 0 / ERR_FAIL      the commands into the path (curves flattened
//                                      to 0.05 px), the matrix applied first
//   bounds(Option<&Matrix>) -> Option<[f64; 4]>      of the transformed, finely flattened outline
//   bounds_box(Option<&Matrix>) -> (any, [f64; 4])   the same as the assembly leaves it
//
// Every builder call checks its coordinates (finite, |v| <= COORD_MAX) and the size
// (MAX_SLOTS); the first failure is kept in err and later calls do nothing.  The commands
// are slots like the assembly's (op, then f64 arguments), so the limits are the same.

use alloc::vec::Vec;

use super::path::Path;
use super::xform::{sincos, Matrix};
use super::{ERR_FAIL, ERR_LIMIT};
use rt::fx::{cvttd, maxsd, minsd, sqrtd};
use rt::x87::atan2;

pub const PDO_MOVE: u64 = 1;
pub const PDO_LINE: u64 = 2;
pub const PDO_CUBIC: u64 = 3;
pub const PDO_CLOSE: u64 = 4;
pub const MAX_SLOTS: usize = 0x40_0000; // 4 Mi slots per PData
pub const MAX_TEXT: usize = 0x100_0000; // parse input limit (16 MB)
pub const COORD_MAX: f64 = 10_000_000.0; // |coordinate| limit of the builder
pub const PDE_SYNTAX: i32 = -40; // unexpected character / missing command
pub const PDE_NUMBER: i32 = -41; // a number was expected
pub const PDE_RANGE: i32 = -42; // a number / coordinate out of range (NaN, > COORD_MAX)

const SIGN: u64 = 0x8000_0000_0000_0000;
const TWO_O3: f64 = 0.6666666666666666;
const FOUR_O3: f64 = 1.3333333333333333;
const TWO_PI: f64 = 6.283185307179586;
const PIO2: f64 = 1.5707963267948966;
const DEG2RAD: f64 = 0.017453292519943295;
const KAPPA: f64 = 0.5522847498307936;
const POW10: [f64; 23] = [
    1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18,
    1e19, 1e20, 1e21, 1e22,
];
// parameters per command letter a..z (0 = none)
const ARGS: [u8; 26] = [7, 0, 6, 0, 0, 0, 0, 1, 0, 0, 0, 2, 2, 0, 0, 0, 4, 0, 4, 2, 0, 1, 0, 0, 0, 0];
// rounded rectangle corners: corner x / y index (into l t r b), start and end direction
const RR_TAB: [[i8; 6]; 4] = [[2, 1, -1, 0, 0, 1], [2, 3, 0, -1, -1, 0], [0, 3, 1, 0, 0, -1], [0, 1, 0, 1, 1, 0]];
const RR_CORNER: [[usize; 2]; 4] = [[2, 1], [2, 3], [0, 3], [0, 1]];

#[inline]
fn abs(v: f64) -> f64 {
    f64::from_bits(v.to_bits() & !SIGN)
}

#[inline]
fn neg(v: f64) -> f64 {
    f64::from_bits(v.to_bits() ^ SIGN)
}

// finite and |v| <= COORD_MAX  [pd_okv]
#[inline]
fn okv(v: f64) -> bool {
    abs(v) <= COORD_MAX
}

/// path data (the PDATA record)
#[derive(Clone, Default)]
pub struct PData {
    slots: Vec<u64>,
    pub rule: u32,
    pub err: i32,
    pub ncmd: u32,
    open: bool,
    cx: f64,
    cy: f64,
    sx: f64,
    sy: f64,
}

impl PData {
    pub fn new() -> PData {
        PData::default()
    }

    /// no commands, no error  [pd_reset]
    pub fn clear(&mut self) {
        self.slots.clear();
        self.rule = 0;
        self.err = 0;
        self.ncmd = 0;
        self.open = false;
        self.cx = 0.0;
        self.cy = 0.0;
        self.sx = 0.0;
        self.sy = 0.0;
    }

    /// the slots used (op words and f64 arguments)
    pub fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// the current point
    pub fn current(&self) -> (f64, f64) {
        (self.cx, self.cy)
    }

    // room for k slots, or false (err set)
    fn room(&mut self, k: usize) -> bool {
        if self.err != 0 {
            return false;
        }
        if self.slots.len() + k > MAX_SLOTS {
            self.err = ERR_LIMIT;
            return false;
        }
        true
    }

    fn range_err(&mut self) {
        if self.err == 0 {
            self.err = PDE_RANGE;
        }
    }

    fn put_move(&mut self, x: f64, y: f64) {
        if !self.room(3) {
            return;
        }
        self.slots.extend_from_slice(&[PDO_MOVE, x.to_bits(), y.to_bits()]);
        self.ncmd += 1;
        (self.cx, self.cy, self.sx, self.sy) = (x, y, x, y);
        self.open = true;
    }

    fn open_sub(&mut self) {
        if !self.open {
            let (x, y) = (self.cx, self.cy);
            self.put_move(x, y);
        }
    }

    /// start a subpath  [pd_move]
    pub fn move_to(&mut self, x: f64, y: f64) {
        if !okv(x) || !okv(y) {
            self.range_err();
            return;
        }
        if self.err == 0 {
            self.put_move(x, y);
        }
    }

    /// straight segment  [pd_line]
    pub fn line_to(&mut self, x: f64, y: f64) {
        if !okv(x) || !okv(y) {
            self.range_err();
            return;
        }
        if self.err != 0 {
            return;
        }
        self.open_sub();
        if !self.room(3) {
            return;
        }
        self.slots.extend_from_slice(&[PDO_LINE, x.to_bits(), y.to_bits()]);
        (self.cx, self.cy) = (x, y);
        self.ncmd += 1;
    }

    /// cubic Bezier from the current point: c1, c2, end  [pd_cubic]
    pub fn cubic(&mut self, p: &[f64; 6]) {
        if !p.iter().all(|&v| okv(v)) {
            self.range_err();
            return;
        }
        if self.err != 0 {
            return;
        }
        self.open_sub();
        if !self.room(7) {
            return;
        }
        self.slots.push(PDO_CUBIC);
        self.slots.extend(p.iter().map(|v| v.to_bits()));
        (self.cx, self.cy) = (p[4], p[5]);
        self.ncmd += 1;
    }

    /// quadratic Bezier (control, end) as the equivalent cubic  [pd_quad]
    pub fn quad(&mut self, q: &[f64; 4]) {
        let c = [
            (q[0] - self.cx) * TWO_O3 + self.cx,
            (q[1] - self.cy) * TWO_O3 + self.cy,
            (q[0] - q[2]) * TWO_O3 + q[2],
            (q[1] - q[3]) * TWO_O3 + q[3],
            q[2],
            q[3],
        ];
        self.cubic(&c);
    }

    /// close the open subpath; the current point returns to its start  [pd_close]
    pub fn close(&mut self) {
        if self.err != 0 || !self.open {
            return;
        }
        if !self.room(1) {
            return;
        }
        self.slots.push(PDO_CLOSE);
        self.ncmd += 1;
        self.open = false;
        (self.cx, self.cy) = (self.sx, self.sy);
    }

    /// elliptical arc {x1, y1, rx, ry, phi (deg), large, sweep, x2, y2} (SVG 1.1 F.6.5 /
    /// F.6.6: radii scaled up when too small, 0 radius = line)  [pd_arc]
    pub fn arc(&mut self, p: &[f64; 9]) {
        let (x1, y1, x2, y2) = (p[0], p[1], p[7], p[8]);
        let mut rx = abs(p[2]);
        let mut ry = abs(p[3]);
        if rx.is_nan() || rx == 0.0 || ry.is_nan() || ry == 0.0 {
            self.line_to(x2, y2);
            return;
        }
        if x1 == x2 && y1 == y2 {
            return; // equal end points (ordered): nothing
        }
        if !okv(rx) || !okv(ry) || !(abs(p[4]) <= 1e6) {
            self.range_err();
            return;
        }
        let (sphi, cphi) = sincos(p[4] * DEG2RAD);
        let dx2 = (x1 - x2) * 0.5;
        let dy2 = (y1 - y2) * 0.5;
        let x1p = cphi * dx2 + sphi * dy2;
        let y1p = cphi * dy2 - sphi * dx2;
        let a = x1p / rx;
        let b = y1p / ry;
        let lambda = a * a + b * b;
        if lambda > 1.0 {
            let s = sqrtd(lambda);
            rx = rx * s;
            ry = ry * s;
        }
        let rx2 = rx * rx;
        let ry2 = ry * ry;
        let t1 = y1p * y1p * rx2;
        let t2 = x1p * x1p * ry2;
        let num = rx2 * ry2 - t1 - t2;
        let den = t1 + t2;
        let mut co = if den > 0.0 { sqrtd(maxsd(num / den, 0.0)) } else { 0.0 };
        if !(p[5] < p[6] || p[5] > p[6]) {
            co = neg(co); // large-arc == sweep
        }
        let cxp = co * rx * y1p / ry;
        let cyp = neg(co * ry * x1p / rx);
        let ccx = cphi * cxp - sphi * cyp + (x1 + x2) * 0.5;
        let ccy = sphi * cxp + cphi * cyp + (y1 + y2) * 0.5;
        let ux = (x1p - cxp) / rx;
        let uy = (y1p - cyp) / ry;
        let theta1 = atan2(uy, ux);
        let vx = (neg(x1p) - cxp) / rx;
        let vy = (neg(y1p) - cyp) / ry;
        let mut dth = atan2(ux * vy - uy * vx, ux * vx + uy * vy);
        if !(p[6] < 0.0 || p[6] > 0.0) {
            if dth > 0.0 {
                dth = dth - TWO_PI;
            }
        } else if !(dth >= 0.0) {
            dth = dth + TWO_PI;
        }
        // n = ceil(|dtheta| / (pi / 2) - 1e-9), 1 .. 4
        let q = abs(dth) / PIO2 - 1e-9;
        let mut n = cvttd(q);
        if q > n as f64 {
            n += 1;
        }
        let n = n.clamp(1, 4);
        let delta = dth / n as f64;
        let (sd, cd) = sincos(delta * 0.25);
        let k = sd / cd * FOUR_O3;
        // E(t) and E'(t) of the ellipse
        let pt = |t: f64| -> [f64; 4] {
            let (st, ct) = sincos(t);
            [
                rx * ct * cphi - ry * st * sphi + ccx,
                rx * ct * sphi + ry * st * cphi + ccy,
                neg(rx * st * cphi + ry * ct * sphi),
                ry * ct * cphi - rx * st * sphi,
            ]
        };
        for i in 0..n {
            let a = i as f64 * delta + theta1;
            let ea = pt(a);
            let mut eb = pt(a + delta);
            if i + 1 == n {
                eb[0] = x2; // the last segment ends exactly at (x2, y2)
                eb[1] = y2;
            }
            let c = [ea[2] * k + self.cx, ea[3] * k + self.cy, eb[0] - eb[2] * k, eb[1] - eb[3] * k, eb[0], eb[1]];
            self.cubic(&c);
            if self.err != 0 {
                return;
            }
        }
    }

    /// closed rounded rectangle {x0, y0, x1, y1, r} (corners in any order, r clamped to 0 ..
    /// half the shorter side), clockwise on screen from the top edge  [pd_rrect]
    pub fn rrect(&mut self, p: &[f64; 5]) {
        let l = minsd(p[0], p[2]);
        let r = maxsd(p[0], p[2]);
        let w = r - l;
        let t = minsd(p[1], p[3]);
        let b = maxsd(p[1], p[3]);
        let h = b - t;
        let half = minsd(w, h) * 0.5;
        let rad = minsd(maxsd(p[4], 0.0), half);
        let kr = rad * KAPPA;
        let ltrb = [l, t, r, b];
        self.move_to(l + rad, t);
        for (c, tab) in RR_TAB.iter().enumerate() {
            if rad > 0.0 {
                let (cx, cy) = (ltrb[tab[0] as usize & 3], ltrb[tab[1] as usize & 3]);
                let f = |v: i8| v as f64;
                let sx = f(tab[2]) * rad + cx;
                let sy = f(tab[3]) * rad + cy;
                let cub = [
                    f(tab[2]) * rad - f(tab[2]) * kr + cx,
                    f(tab[3]) * rad - f(tab[3]) * kr + cy,
                    f(tab[4]) * rad - f(tab[4]) * kr + cx,
                    f(tab[5]) * rad - f(tab[5]) * kr + cy,
                    f(tab[4]) * rad + cx,
                    f(tab[5]) * rad + cy,
                ];
                self.line_to(sx, sy);
                self.cubic(&cub);
            } else {
                self.line_to(ltrb[RR_CORNER[c][0]], ltrb[RR_CORNER[c][1]]);
            }
        }
        self.close();
    }

    /// append SVG / XAML path data: (0 or the error, byte offset of the error or len)  [pd_parse]
    pub fn parse(&mut self, text: &[u8]) -> (i32, usize) {
        if text.len() > MAX_TEXT {
            return (ERR_LIMIT, 0);
        }
        if self.err != 0 {
            return (self.err, 0);
        }
        let end = text.len();
        let mut p = Parser { pd: self, s: text, i: 0, cur: (0.0, 0.0), start: (0.0, 0.0), ctrl: (0.0, 0.0), last: 0 };
        let r = p.run();
        let i = p.i;
        let code = match r {
            Ok(()) => self.err,
            Err(e) => {
                if self.err == 0 {
                    self.err = e;
                }
                e
            }
        };
        (code, i.min(end))
    }

    // pw_walk: the commands as points: into the path (bounds None) or the bounds
    fn walk(&self, m: Option<&Matrix>, mut path: Option<&mut Path>, bounds: &mut [f64; 4]) -> Result<bool, i32> {
        let m = m.copied().unwrap_or(Matrix::IDENTITY);
        let tol = if path.is_some() { 0.05 } else { 0.02 };
        *bounds = [1e300, 1e300, -1e300, -1e300];
        let mut any = false;
        let mut point = |x: f64, y: f64, line: bool, path: &mut Option<&mut Path>| match path {
            Some(path) => {
                let fx = minsd(maxsd(x, -4e6), 4e6) as f32;
                let fy = minsd(maxsd(y, -4e6), 4e6) as f32;
                if line {
                    path.line_f(fx, fy);
                } else {
                    path.move_f(fx, fy);
                }
            }
            None => {
                bounds[0] = minsd(bounds[0], x);
                bounds[1] = minsd(bounds[1], y);
                bounds[2] = maxsd(bounds[2], x);
                bounds[3] = maxsd(bounds[3], y);
                any = true;
            }
        };
        let s = &self.slots;
        let n = s.len();
        let f = |k: usize| f64::from_bits(s[k]);
        let mut cur = (0.0f64, 0.0f64);
        let mut i = 0;
        while i < n {
            match s[i] {
                PDO_CLOSE => {
                    i += 1;
                    if let Some(p) = path.as_deref_mut() {
                        p.close();
                    }
                }
                op @ (PDO_MOVE | PDO_LINE) => {
                    if i + 3 > n {
                        return Err(ERR_FAIL);
                    }
                    cur = m.apply(f(i + 1), f(i + 2));
                    point(cur.0, cur.1, op == PDO_LINE, &mut path);
                    i += 3;
                }
                PDO_CUBIC => {
                    if i + 7 > n {
                        return Err(ERR_FAIL);
                    }
                    let c1 = m.apply(f(i + 1), f(i + 2));
                    let c2 = m.apply(f(i + 3), f(i + 4));
                    let p3 = m.apply(f(i + 5), f(i + 6));
                    let p0 = cur;
                    let ax = p0.0 - (c1.0 + c1.0) + c2.0;
                    let ay = p0.1 - (c1.1 + c1.1) + c2.1;
                    let d1 = ax * ax + ay * ay;
                    let bx = c1.0 - (c2.0 + c2.0) + p3.0;
                    let by = c1.1 - (c2.1 + c2.1) + p3.1;
                    let d2 = bx * bx + by * by;
                    let segs = minsd(sqrtd(sqrtd(maxsd(d1, d2)) * 0.75 / tol), 999.0);
                    let segs = ((cvttd(segs) + 1) as u32).min(1000);
                    for k in 1..=segs {
                        let t = k as f64 / segs as f64;
                        let u = 1.0 - t;
                        let b0 = u * u * u;
                        let b1 = u * u * t * 3.0;
                        let b2 = u * t * t * 3.0;
                        let b3 = t * t * t;
                        let mut x = p0.0 * b0 + c1.0 * b1 + c2.0 * b2 + p3.0 * b3;
                        let mut y = p0.1 * b0 + c1.1 * b1 + c2.1 * b2 + p3.1 * b3;
                        if k == segs {
                            (x, y) = p3; // the last point exactly
                        }
                        cur = (x, y);
                        point(x, y, true, &mut path);
                    }
                    i += 7;
                }
                _ => return Err(ERR_FAIL),
            }
        }
        Ok(any)
    }

    /// append the commands to the path (matrix applied first)  [pd_emit]
    pub fn emit(&self, path: &mut Path, m: Option<&Matrix>) -> i32 {
        let mut b = [0.0; 4];
        match self.walk(m, Some(path), &mut b) {
            Ok(_) => 0,
            Err(e) => e,
        }
    }

    /// {min x, min y, max x, max y} of the (transformed, finely flattened) outline  [pd_bounds]
    pub fn bounds(&self, m: Option<&Matrix>) -> Option<[f64; 4]> {
        match self.bounds_box(m) {
            (true, b) => Some(b),
            _ => None,
        }
    }

    /// pd_bounds as the assembly leaves its output: (any point, box); without a point the box
    /// is {1e300, 1e300, -1e300, -1e300}
    pub fn bounds_box(&self, m: Option<&Matrix>) -> (bool, [f64; 4]) {
        let mut b = [0.0; 4];
        let any = self.walk(m, None, &mut b).unwrap_or(false);
        (any, b)
    }
}

// blanks: space, 9, 10, 12, 13 (not 11)
#[inline]
fn blank(c: u8) -> bool {
    c == 32 || (9..=13).contains(&c) && c != 11
}

struct Parser<'a> {
    pd: &'a mut PData,
    s: &'a [u8],
    i: usize,
    cur: (f64, f64),
    start: (f64, f64),
    ctrl: (f64, f64),
    last: u32, // 1 cubic, 2 quad
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && blank(self.s[self.i]) {
            self.i += 1;
        }
    }

    // blanks, at most one comma, blanks
    fn sep(&mut self) {
        let mut comma = false;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c == b',' {
                if comma {
                    return;
                }
                comma = true;
            } else if !blank(c) {
                return;
            }
            self.i += 1;
        }
    }

    // [+-] (digits [. digits] | . digits) [(e|E) [+-] digits] -> Ok(v) / Err(1 none, 2 range)
    fn number(&mut self) -> Result<f64, u32> {
        let s = self.s;
        let end = s.len();
        let start = self.i;
        let mut i = self.i;
        let mut neg_ = false;
        let mut m: u64 = 0;
        let mut e: i32 = 0;
        let mut digits = 0;
        if i >= end {
            return Err(1);
        }
        if s[i] == b'-' {
            neg_ = true;
            i += 1;
        } else if s[i] == b'+' {
            i += 1;
        }
        while i < end && s[i].wrapping_sub(b'0') <= 9 {
            digits += 1;
            let d = (s[i] - b'0') as u64;
            i += 1;
            if m < 100_000_000_000_000_000 {
                m = m * 10 + d;
            } else if e < 100000 {
                e += 1;
            }
        }
        if i < end && s[i] == b'.' {
            i += 1;
            while i < end && s[i].wrapping_sub(b'0') <= 9 {
                digits += 1;
                let d = (s[i] - b'0') as u64;
                i += 1;
                if m < 100_000_000_000_000_000 {
                    m = m * 10 + d;
                    e -= 1;
                }
            }
        }
        if digits == 0 {
            self.i = start;
            return Err(1);
        }
        // the exponent only when a digit follows
        if i < end && s[i] | 32 == b'e' {
            let mut j = i + 1;
            let mut eneg = false;
            if j < end && (s[j] == b'-' || s[j] == b'+') {
                eneg = s[j] == b'-';
                j += 1;
            }
            if j < end && s[j].wrapping_sub(b'0') <= 9 {
                let mut ed: i32 = 0;
                while j < end && s[j].wrapping_sub(b'0') <= 9 {
                    let d = (s[j] - b'0') as i32;
                    j += 1;
                    if ed < 100000 {
                        ed = ed * 10 + d;
                    }
                }
                i = j;
                e += if eneg { -ed } else { ed };
            }
        }
        self.i = i;
        let mut v = m as i64 as f64;
        if m != 0 {
            if e > 0 {
                if e > 22 {
                    return Err(2);
                }
                v = v * POW10[e as usize];
            } else if e < 0 {
                if e < -400 {
                    v = 0.0;
                } else {
                    while e < -22 {
                        v = v / POW10[22];
                        e += 22;
                    }
                    v = v / POW10[(-e) as usize];
                }
            }
        }
        if v > 1e9 {
            return Err(2);
        }
        Ok(if neg_ { neg(v) } else { v })
    }

    fn run(&mut self) -> Result<(), i32> {
        let end = self.s.len();
        let mut cmd: u8 = 0; // lower case, 0 = none
        self.ws();
        if self.i >= end {
            return Ok(());
        }
        if self.s[self.i] | 32 == b'f' {
            self.i += 1;
            self.ws();
            if self.i >= end {
                return Err(PDE_SYNTAX);
            }
            let d = self.s[self.i].wrapping_sub(b'0');
            if d > 1 {
                return Err(PDE_SYNTAX);
            }
            self.pd.rule = (d ^ 1) as u32; // F0 even-odd, F1 non-zero
            self.i += 1;
        }
        loop {
            self.ws();
            if self.i >= end {
                return Ok(());
            }
            let c = self.s[self.i];
            let lc = c | 32;
            let nargs = if lc.wrapping_sub(b'a') <= 25 { ARGS[(lc - b'a') as usize] } else { 0 };
            if nargs == 0 && lc != b'z' {
                return Err(PDE_SYNTAX); // numbers are consumed with their command
            }
            let rel = c & 32 != 0;
            if cmd == 0 && lc != b'm' {
                return Err(PDE_SYNTAX); // the first command must be a move
            }
            cmd = lc;
            self.i += 1;
            if cmd == b'z' {
                self.pd.close();
                self.cur = self.start;
                self.last = 0;
                if self.pd.err != 0 {
                    return Err(self.pd.err);
                }
                continue;
            }
            // parameter groups of this command
            loop {
                let nargs = ARGS[(cmd - b'a') as usize] as usize;
                let mut args = [0f64; 7];
                for k in 0..nargs {
                    self.sep();
                    if cmd == b'a' && (k == 3 || k == 4) {
                        if self.i >= end {
                            return Err(PDE_NUMBER);
                        }
                        let d = self.s[self.i].wrapping_sub(b'0');
                        if d > 1 {
                            return Err(PDE_NUMBER);
                        }
                        args[k] = d as f64;
                        self.i += 1;
                    } else {
                        match self.number() {
                            Ok(v) => args[k] = v,
                            Err(1) => return Err(PDE_NUMBER),
                            Err(_) => return Err(PDE_RANGE),
                        }
                    }
                }
                // relative coordinates: add the current point
                if rel {
                    match cmd {
                        b'h' => args[0] = args[0] + self.cur.0,
                        b'v' => args[0] = args[0] + self.cur.1,
                        b'a' => {
                            args[5] = args[5] + self.cur.0;
                            args[6] = args[6] + self.cur.1;
                        }
                        _ => {
                            let mut j = 0;
                            while j < nargs && j < 6 {
                                args[j] = args[j] + self.cur.0;
                                args[j + 1] = args[j + 1] + self.cur.1;
                                j += 2;
                            }
                        }
                    }
                }
                self.exec(&mut cmd, &mut args);
                if self.pd.err != 0 {
                    return Err(self.pd.err);
                }
                // another parameter group of the same command when a number follows
                self.sep();
                if self.i >= end {
                    return Ok(());
                }
                let c = self.s[self.i];
                if !(c.wrapping_sub(b'0') <= 9 || c == b'+' || c == b'-' || c == b'.') {
                    break;
                }
            }
        }
    }

    fn exec(&mut self, cmd: &mut u8, a: &mut [f64; 7]) {
        match *cmd {
            b'm' => {
                self.pd.move_to(a[0], a[1]);
                self.start = (a[0], a[1]);
                *cmd = b'l'; // further pairs are lines
                self.cur = (a[0], a[1]);
                self.last = 0;
            }
            b'h' | b'v' | b'l' => {
                if *cmd == b'h' {
                    a[1] = self.cur.1;
                } else if *cmd == b'v' {
                    a[1] = a[0];
                    a[0] = self.cur.0;
                }
                self.pd.line_to(a[0], a[1]);
                self.cur = (a[0], a[1]);
                self.last = 0;
            }
            b'c' | b's' => {
                let c = if *cmd == b'c' {
                    [a[0], a[1], a[2], a[3], a[4], a[5]]
                } else {
                    let mut c1 = self.cur;
                    if self.last == 1 {
                        c1 = (self.cur.0 + self.cur.0 - self.ctrl.0, self.cur.1 + self.cur.1 - self.ctrl.1);
                    }
                    [c1.0, c1.1, a[0], a[1], a[2], a[3]]
                };
                self.pd.cubic(&c);
                self.ctrl = (c[2], c[3]);
                self.cur = (c[4], c[5]);
                self.last = 1;
            }
            b'q' | b't' => {
                let q = if *cmd == b'q' {
                    [a[0], a[1], a[2], a[3]]
                } else {
                    let mut c = self.cur;
                    if self.last == 2 {
                        c = (self.cur.0 + self.cur.0 - self.ctrl.0, self.cur.1 + self.cur.1 - self.ctrl.1);
                    }
                    [c.0, c.1, a[0], a[1]]
                };
                self.pd.quad(&q);
                self.ctrl = (q[0], q[1]);
                self.cur = (q[2], q[3]);
                self.last = 2;
            }
            _ => {
                // b'a'
                let p = [self.cur.0, self.cur.1, a[0], a[1], a[2], a[3], a[4], a[5], a[6]];
                self.pd.arc(&p);
                self.cur = (a[5], a[6]);
                self.last = 0;
            }
        }
    }
}
