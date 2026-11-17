// geom -- matrices, colours, paths and the stroke parameters of a canvas

use alloc::vec::Vec;

/// an affine matrix as PDF writes it (`a b c d e f cm`): x' = a x + c y + e, y' = b x + d y + f
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Matrix {
    pub const IDENTITY: Matrix = Matrix { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    pub const fn new(a: f64, b: f64, c: f64, d: f64, e: f64, f: f64) -> Matrix {
        Matrix { a, b, c, d, e, f }
    }

    pub const fn translate(tx: f64, ty: f64) -> Matrix {
        Matrix { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: tx, f: ty }
    }

    pub const fn scale(sx: f64, sy: f64) -> Matrix {
        Matrix { a: sx, b: 0.0, c: 0.0, d: sy, e: 0.0, f: 0.0 }
    }

    /// a rotation by `angle` radians: counter-clockwise where y points up, clockwise on a canvas
    /// whose y points down
    pub fn rotate(angle: f64) -> Matrix {
        let (s, c) = gfx::xform::sincos(angle);
        Matrix { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 }
    }

    /// self first, then `next` (PDF's self × next)
    pub fn then(&self, next: &Matrix) -> Matrix {
        Matrix {
            a: self.a * next.a + self.b * next.c,
            b: self.a * next.b + self.b * next.d,
            c: self.c * next.a + self.d * next.c,
            d: self.c * next.b + self.d * next.d,
            e: self.e * next.a + self.f * next.c + next.e,
            f: self.e * next.b + self.f * next.d + next.f,
        }
    }

    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }

    pub fn det(&self) -> f64 {
        self.a * self.d - self.b * self.c
    }

    /// the inverse; None when the matrix is singular or not finite
    pub fn invert(&self) -> Option<Matrix> {
        let det = self.det();
        if !(det.is_finite() && det.abs() > 1e-300) {
            return None;
        }
        let i = 1.0 / det;
        let m = Matrix {
            a: self.d * i,
            b: -self.b * i,
            c: -self.c * i,
            d: self.a * i,
            e: (self.c * self.f - self.d * self.e) * i,
            f: (self.b * self.e - self.a * self.f) * i,
        };
        [m.a, m.b, m.c, m.d, m.e, m.f].iter().all(|v| v.is_finite()).then_some(m)
    }

    pub(crate) fn is_finite(&self) -> bool {
        [self.a, self.b, self.c, self.d, self.e, self.f].iter().all(|v| v.is_finite())
    }
}

/// gfx's matrix (x' = a x + b y + c, y' = d x + e y + f) as PDF writes it
impl From<gfx::xform::Matrix> for Matrix {
    fn from(m: gfx::xform::Matrix) -> Matrix {
        Matrix { a: m.a, b: m.d, c: m.b, d: m.e, e: m.c, f: m.f }
    }
}

/// an RGB colour, each part 0 to 1 (DeviceRGB)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

impl Rgb {
    pub const BLACK: Rgb = Rgb { r: 0.0, g: 0.0, b: 0.0 };
    pub const WHITE: Rgb = Rgb { r: 1.0, g: 1.0, b: 1.0 };

    pub const fn new(r: f64, g: f64, b: f64) -> Rgb {
        Rgb { r, g, b }
    }

    /// from 0..255 parts
    pub fn from_u8(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r: r as f64 / 255.0, g: g as f64 / 255.0, b: b as f64 / 255.0 }
    }

    /// from 0xRRGGBB
    pub fn hex(rgb: u32) -> Rgb {
        Rgb::from_u8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
    }

    pub(crate) fn clamped(&self) -> Rgb {
        let c = |v: f64| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        Rgb { r: c(self.r), g: c(self.g), b: c(self.b) }
    }
}

/// which parts of a path a fill covers (PDF `f` / `f*`)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

/// the ends of open subpaths and dashes (PDF `J`)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

/// the corners of a stroke (PDF `j`)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// how text is painted (PDF `Tr`)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum TextMode {
    #[default]
    Fill,
    Stroke,
    FillStroke,
    /// not painted, but found by search and copied (e.g. text over a scanned page)
    Invisible,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Seg {
    Move(f64, f64),
    Line(f64, f64),
    Cubic(f64, f64, f64, f64, f64, f64),
    Rect(f64, f64, f64, f64),
    Close,
}

/// the circle constant of cubic arcs: 4/3 (sqrt 2 - 1)
const KAPPA: f64 = 0.552_284_749_830_793_6;

/// a path: subpaths of lines and cubic curves, built once and drawn on any canvas
/// (Canvas::append_path) or built on the canvas itself (its move_to, line_to, ...)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    pub(crate) segs: Vec<Seg>,
    cur: Option<(f64, f64)>,
    start: (f64, f64),
}

impl Path {
    pub fn new() -> Path {
        Path::default()
    }

    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }

    pub fn clear(&mut self) {
        self.segs.clear();
        self.cur = None;
    }

    /// the current point (None before the first move_to and after clear)
    pub fn current(&self) -> Option<(f64, f64)> {
        self.cur
    }

    /// starts a subpath
    pub fn move_to(&mut self, x: f64, y: f64) -> &mut Path {
        self.segs.push(Seg::Move(x, y));
        self.cur = Some((x, y));
        self.start = (x, y);
        self
    }

    /// a straight line from the current point (without one: a move_to)
    pub fn line_to(&mut self, x: f64, y: f64) -> &mut Path {
        if self.cur.is_none() {
            return self.move_to(x, y);
        }
        self.segs.push(Seg::Line(x, y));
        self.cur = Some((x, y));
        self
    }

    /// a cubic Bézier curve with control points (x1, y1) and (x2, y2) to (x, y)
    pub fn curve_to(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, x: f64, y: f64) -> &mut Path {
        if self.cur.is_none() {
            self.move_to(x1, y1);
        }
        self.segs.push(Seg::Cubic(x1, y1, x2, y2, x, y));
        self.cur = Some((x, y));
        self
    }

    /// a quadratic Bézier curve (written as the same cubic)
    pub fn quad_to(&mut self, x1: f64, y1: f64, x: f64, y: f64) -> &mut Path {
        let (x0, y0) = match self.cur {
            Some(p) => p,
            None => {
                self.move_to(x1, y1);
                (x1, y1)
            }
        };
        let c1 = (x0 + (x1 - x0) * (2.0 / 3.0), y0 + (y1 - y0) * (2.0 / 3.0));
        let c2 = (x + (x1 - x) * (2.0 / 3.0), y + (y1 - y) * (2.0 / 3.0));
        self.curve_to(c1.0, c1.1, c2.0, c2.1, x, y)
    }

    /// closes the subpath with a line to its start
    pub fn close(&mut self) -> &mut Path {
        if self.cur.is_some() {
            self.segs.push(Seg::Close);
            self.cur = Some(self.start);
        }
        self
    }

    /// a closed rectangle as its own subpath (PDF `re`)
    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64) -> &mut Path {
        self.segs.push(Seg::Rect(x, y, w, h));
        self.cur = Some((x, y));
        self.start = (x, y);
        self
    }

    /// a closed ellipse around (cx, cy) as four cubic curves
    pub fn ellipse(&mut self, cx: f64, cy: f64, rx: f64, ry: f64) -> &mut Path {
        let (kx, ky) = (rx * KAPPA, ry * KAPPA);
        self.move_to(cx + rx, cy);
        self.curve_to(cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry);
        self.curve_to(cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy);
        self.curve_to(cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry);
        self.curve_to(cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy);
        self.close()
    }

    pub fn circle(&mut self, cx: f64, cy: f64, r: f64) -> &mut Path {
        self.ellipse(cx, cy, r, r)
    }

    /// a closed rectangle with corners rounded by r (at most half the shorter side)
    pub fn rounded_rect(&mut self, x: f64, y: f64, w: f64, h: f64, r: f64) -> &mut Path {
        let r = r.max(0.0).min(w.abs() / 2.0).min(h.abs() / 2.0);
        if r <= 0.0 || !r.is_finite() {
            return self.rect(x, y, w, h);
        }
        let (x0, x1) = if w < 0.0 { (x + w, x) } else { (x, x + w) };
        let (y0, y1) = if h < 0.0 { (y + h, y) } else { (y, y + h) };
        let k = r * (1.0 - KAPPA);
        self.move_to(x0 + r, y0);
        self.line_to(x1 - r, y0);
        self.curve_to(x1 - k, y0, x1, y0 + k, x1, y0 + r);
        self.line_to(x1, y1 - r);
        self.curve_to(x1, y1 - k, x1 - k, y1, x1 - r, y1);
        self.line_to(x0 + r, y1);
        self.curve_to(x0 + k, y1, x0, y1 - k, x0, y1 - r);
        self.line_to(x0, y0 + r);
        self.curve_to(x0, y0 + k, x0 + k, y0, x0 + r, y0);
        self.close()
    }

    /// an elliptical arc around (cx, cy) from angle `start` over `sweep` radians (positive:
    /// from +x towards +y), as cubic curves of at most 90 degrees; it continues the subpath
    /// with a line to its start, or starts one
    pub fn arc(&mut self, cx: f64, cy: f64, rx: f64, ry: f64, start: f64, sweep: f64) -> &mut Path {
        if !(start.is_finite() && sweep.is_finite()) {
            return self;
        }
        let sweep = sweep.clamp(-core::f64::consts::TAU, core::f64::consts::TAU);
        let n = ((sweep.abs() / core::f64::consts::FRAC_PI_2) - 1e-9).max(0.0) as usize + 1;
        let step = sweep / n as f64;
        let (sq, cq) = gfx::xform::sincos(step / 4.0);
        let k = 4.0 / 3.0 * sq / cq;
        let (s0, c0) = gfx::xform::sincos(start);
        let p0 = (cx + rx * c0, cy + ry * s0);
        if self.cur.is_some() {
            self.line_to(p0.0, p0.1);
        } else {
            self.move_to(p0.0, p0.1);
        }
        let mut a = start;
        for _ in 0..n {
            let (sa, ca) = gfx::xform::sincos(a);
            let (sb, cb) = gfx::xform::sincos(a + step);
            self.curve_to(
                cx + rx * (ca - k * sa),
                cy + ry * (sa + k * ca),
                cx + rx * (cb + k * sb),
                cy + ry * (sb - k * cb),
                cx + rx * cb,
                cy + ry * sb,
            );
            a += step;
        }
        self
    }

    /// appends another path's subpaths
    pub fn append(&mut self, other: &Path) -> &mut Path {
        self.segs.extend_from_slice(&other.segs);
        if other.cur.is_some() {
            self.cur = other.cur;
            self.start = other.start;
        }
        self
    }

    /// every point mapped by m (a rectangle becomes four lines when m turns it)
    pub fn transform(&mut self, m: &Matrix) {
        let mut out = Vec::with_capacity(self.segs.len());
        let keeps_rect = m.b == 0.0 && m.c == 0.0;
        for s in &self.segs {
            let seg = match *s {
                Seg::Move(x, y) => {
                    let p = m.apply(x, y);
                    Seg::Move(p.0, p.1)
                }
                Seg::Line(x, y) => {
                    let p = m.apply(x, y);
                    Seg::Line(p.0, p.1)
                }
                Seg::Cubic(a, b, c, d, e, f) => {
                    let p1 = m.apply(a, b);
                    let p2 = m.apply(c, d);
                    let p3 = m.apply(e, f);
                    Seg::Cubic(p1.0, p1.1, p2.0, p2.1, p3.0, p3.1)
                }
                Seg::Rect(x, y, w, h) if keeps_rect => {
                    let p = m.apply(x, y);
                    Seg::Rect(p.0, p.1, w * m.a, h * m.d)
                }
                Seg::Rect(x, y, w, h) => {
                    let pts = [m.apply(x, y), m.apply(x + w, y), m.apply(x + w, y + h), m.apply(x, y + h)];
                    out.push(Seg::Move(pts[0].0, pts[0].1));
                    out.push(Seg::Line(pts[1].0, pts[1].1));
                    out.push(Seg::Line(pts[2].0, pts[2].1));
                    out.push(Seg::Line(pts[3].0, pts[3].1));
                    Seg::Close
                }
                Seg::Close => Seg::Close,
            };
            out.push(seg);
        }
        self.segs = out;
        self.cur = self.cur.map(|(x, y)| m.apply(x, y));
        self.start = m.apply(self.start.0, self.start.1);
    }
}
