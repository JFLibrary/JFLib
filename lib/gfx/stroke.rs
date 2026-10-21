// stroke -- stroke outlines of a path                                     [src/gfx/stroke.s]
//
//   path.stroke(ras, dc, sf, width, style)   width 24.8 fixed pixels; style = join | cap << 4
//                                           (JOIN_MITER (limit 4, else bevel) / ROUND / BEVEL,
//                                           CAP_BUTT / ROUND / SQUARE)
//   path.stroke_lines(ras, width, style)     only the outline line list (for other targets)
//
// Every flattened subpath becomes closed contours (outer side forward, inner side
// backward, with joins and caps) that are filled with the non-zero rule.  Inner joins
// pivot through the vertex; the non-zero fill absorbs the overlap.  Offsets use
// n = (dy, -dx) * half_width, so with y pointing down a positive cross product din x dout
// means the +n side is the outer side of the turn.  The float operations follow the
// assembly one by one (0 - x instead of -x, comparisons with its NaN behaviour).

use alloc::vec::Vec;

use super::path::{zf, Path};
use super::raster::Raster;
use super::{Dc, Surface, CAP_BUTT, CAP_ROUND, CAP_SQUARE, JOIN_MITER, JOIN_ROUND};
use rt::fx::{maxss, minss, sqrt};

const EPS: f32 = 0.0001;

struct St<'a> {
    hw: f32,
    p: (f32, f32),
    di: (f32, f32),
    dout: (f32, f32),
    cs: f32,
    sn: f32,
    join: u32,
    cap: u32,
    pts: &'a [(f32, f32)],
    base: usize,
    n: usize,
    v: (f32, f32),
    t: (f32, f32),
    spts: Vec<(f32, f32)>,
}

impl St<'_> {
    fn pt(&self, i: usize) -> (f32, f32) {
        self.pts[self.base + i]
    }

    // unit vector from point i to point j ((1, 0) when degenerate)
    fn dir(&self, i: usize, j: usize) -> (f32, f32) {
        let a = self.pt(i);
        let b = self.pt(j);
        let dx = b.0 - a.0;
        let dy = b.1 - a.1;
        let len = sqrt(dx * dx + dy * dy);
        if zf(len, 0.0) {
            return (1.0, 0.0);
        }
        (dx / len, dy / len)
    }

    // sp_add: append a contour point (exact duplicates skipped)
    fn add(&mut self, x: f32, y: f32) {
        if let Some(&l) = self.spts.last() {
            if zf(x, l.0) && zf(y, l.1) {
                return;
            }
        }
        self.spts.push((x, y));
    }

    // P + sign * n(d) * hw, n = (dy, -dx)
    fn emit_off(&mut self, sign: f32, d: (f32, f32)) {
        let x = d.1 * self.hw * sign + self.p.0;
        let y = (0.0 - d.0) * self.hw * sign + self.p.1;
        self.add(x, y);
    }

    // join at P between di (in) and dout (out) on the +n side
    fn join(&mut self) {
        let cross = self.di.0 * self.dout.1 - self.di.1 * self.dout.0;
        let dot = self.di.0 * self.dout.0 + self.di.1 * self.dout.1;
        if !(cross.abs() >= EPS) && dot >= 0.0 {
            let d = self.dout;
            self.emit_off(1.0, d); // nearly straight: one point
            return;
        }
        if !(cross >= 0.0) {
            // inner side: pivot through the vertex
            let di = self.di;
            self.emit_off(1.0, di);
            let p = self.p;
            self.add(p.0, p.1);
            let d = self.dout;
            self.emit_off(1.0, d);
            return;
        }
        let di = self.di;
        self.emit_off(1.0, di);
        if self.join == JOIN_ROUND {
            self.v = (self.di.1, 0.0 - self.di.0);
            self.t = (self.dout.1, 0.0 - self.dout.0);
            self.arc();
        } else if self.join == JOIN_MITER {
            let m = dot + 1.0;
            if m >= 0.125 {
                // P + (nin + nout) * hw / (1 + dot)
                let nx = self.di.1 + self.dout.1;
                let ny = 0.0 - (self.di.0 + self.dout.0);
                let k = self.hw / m;
                let p = self.p;
                self.add(nx * k + p.0, ny * k + p.1);
            }
        }
        let d = self.dout;
        self.emit_off(1.0, d);
    }

    // P + v * hw while rotating v towards t (t itself is not emitted)
    fn arc(&mut self) {
        for _ in 0..64 {
            let nx = self.v.0 * self.cs - self.v.1 * self.sn;
            let ny = self.v.0 * self.sn + self.v.1 * self.cs;
            if !(nx * self.t.1 - ny * self.t.0 >= EPS) {
                return;
            }
            self.v = (nx, ny);
            let p = self.p;
            self.add(nx * self.hw + p.0, ny * self.hw + p.1);
        }
    }

    // cap at P for travel direction d: from +n via d to -n (the +n point is already there)
    fn cap(&mut self, d: (f32, f32)) {
        let (dx, dy) = d;
        if self.cap == CAP_SQUARE {
            let p = self.p;
            self.add((dy + dx) * self.hw + p.0, (dy - dx) * self.hw + p.1);
            self.add((dx - dy) * self.hw + p.0, (dy + dx) * self.hw + p.1);
        } else if self.cap == CAP_ROUND {
            self.v = (dy, 0.0 - dx);
            self.t = (dx, dy);
            self.arc();
            let p = self.p;
            self.add(dx * self.hw + p.0, dy * self.hw + p.1);
            self.v = (dx, dy);
            self.t = (0.0 - dy, dx);
            self.arc();
        }
        self.emit_off(-1.0, d);
    }

    // sp_flush: the contour as closed lines
    fn flush(&mut self, ras: &mut Raster) {
        let n = self.spts.len();
        if n >= 2 {
            for i in 0..n {
                let a = self.spts[i];
                let b = self.spts[if i + 1 < n { i + 1 } else { 0 }];
                ras.emit_line(a.0, a.1, b.0, b.1);
            }
        }
        self.spts.clear();
    }

    // outline of an open polyline
    fn open(&mut self, ras: &mut Raster) {
        self.spts.clear();
        let last = self.n - 1;
        let d0 = self.dir(0, 1);
        self.p = self.pt(0);
        self.emit_off(1.0, d0);
        for i in 1..last {
            self.di = self.dir(i - 1, i);
            self.dout = self.dir(i, i + 1);
            self.p = self.pt(i);
            self.join();
        }
        let dl = self.dir(last - 1, last);
        self.p = self.pt(last);
        self.emit_off(1.0, dl);
        self.cap(dl);
        let mut i = last as isize - 1;
        while i >= 1 {
            let k = i as usize;
            self.di = self.dir(k + 1, k);
            self.dout = self.dir(k, k - 1);
            self.p = self.pt(k);
            self.join();
            i -= 1;
        }
        self.p = self.pt(0);
        let back = (0.0 - d0.0, 0.0 - d0.1);
        self.emit_off(1.0, back);
        self.cap(back);
        self.flush(ras);
    }

    // two contours of a closed polyline
    fn closed(&mut self, ras: &mut Raster) {
        let n = self.n;
        if n < 2 {
            return;
        }
        self.spts.clear();
        for i in 0..n {
            self.di = self.dir((i + n - 1) % n, i);
            self.dout = self.dir(i, (i + 1) % n);
            self.p = self.pt(i);
            self.join();
        }
        self.flush(ras);
        self.spts.clear();
        for i in (0..n).rev() {
            self.di = self.dir((i + 1) % n, i);
            self.dout = self.dir(i, (i + n - 1) % n);
            self.p = self.pt(i);
            self.join();
        }
        self.flush(ras);
    }

    // a single point: round cap = disc, square cap = square, butt = nothing
    fn dot(&mut self, ras: &mut Raster) {
        self.p = self.pt(0);
        if self.cap == CAP_BUTT {
            return;
        }
        self.spts.clear();
        self.emit_off(1.0, (1.0, 0.0));
        self.cap((1.0, 0.0));
        self.cap((-1.0, 0.0));
        self.flush(ras);
    }
}

impl Path {
    /// the outline lines of path_stroke (ras.lines_begin + the contours); the caller fills
    /// them with the non-zero rule
    pub fn stroke_lines(&self, ras: &mut Raster, width: i32, style: u32) {
        let hw = maxss(width as f32 * 0.001953125, 0.25); // half width, at least 0.25 px
        // arc step s = min(0.5, 2 * sqrt(0.4 / hw)); cos / sin by Taylor series
        let mut s = sqrt(0.4 / hw);
        s = s + s;
        let s = minss(s, 0.5);
        let s2 = s * s;
        let cs = (s2 * 0.041666668 - 0.5) * s2 + 1.0;
        let sn = ((s2 * 0.008333334 - 0.16666667) * s2 + 1.0) * s;
        let mut st = St {
            hw,
            p: (0.0, 0.0),
            di: (0.0, 0.0),
            dout: (0.0, 0.0),
            cs,
            sn,
            join: style & 15,
            cap: (style >> 4) & 15,
            pts: &self.pts,
            base: 0,
            n: 0,
            v: (0.0, 0.0),
            t: (0.0, 0.0),
            spts: Vec::new(),
        };
        ras.lines_begin();
        for sub in &self.subs {
            let mut n = sub.count;
            if n == 0 {
                continue;
            }
            st.base = sub.start;
            // a closed subpath whose last point repeats the first: drop it
            if n >= 2 && sub.closed {
                let a = self.pts[sub.start];
                let b = self.pts[sub.start + n - 1];
                if zf(a.0, b.0) && zf(a.1, b.1) {
                    n -= 1;
                }
            }
            st.n = n;
            if n == 1 {
                st.dot(ras);
            } else if sub.closed {
                st.closed(ras);
            } else {
                st.open(ras);
            }
        }
    }

    /// path_stroke: the outline filled (non-zero) with the current colour
    pub fn stroke(&self, ras: &mut Raster, dc: &Dc, sf: &mut Surface, width: i32, style: u32) {
        self.stroke_lines(ras, width, style);
        ras.fill(dc, sf, 0);
    }
}
