// geometry -- the Geometry sections of a shape as paths, and the transforms of shapes (JD4)
//
//   Xform                            an affine map (x, y) -> (a x + c y + e, b x + d y + f)
//   Path { subs } / Sub { start, segs, closed } / Seg::Line | Seg::Cubic
//                                    paths of straight lines and cubic Bézier curves: every row
//                                    type ends up as one of the two (arcs and ellipses as cubic
//                                    pieces of at most 90 degrees, NURBS and splines exactly as
//                                    Bézier pieces where they are polynomial, else sampled)
//   doc.geometry_paths(sheet, w, h)  the Geometry sections of a sheet with their flags (NoFill,
//                                    NoLine, NoShow, NoSnap) and paths in local coordinates
//   doc.geometry_tagged(sheet, w, h) (JD21) the same, and per piece the row (IX) it comes from
//   Conic / row_conic                (JD21) the ellipse an ArcTo, EllipticalArcTo, Ellipse or
//                                    RelEllipticalArcTo row draws: c + u cos t + v sin t
//   doc.local_xform(sheet)           local -> parent: Pin, LocPin, Angle, FlipX, FlipY
//   doc.page_xform(shape)            local -> page: the chain of groups up to the page
//   round_corners(path, r)           the Rounding cell: corners between straight lines rounded
//   nurbs_segments / parse_args      NURBS(...) and POLYLINE(...) values as the file keeps them
//
// Everything comes from the cached values (V) through inheritance (doc.resolve): nothing is
// recalculated (PLAN.md, "Fallen").  Local coordinates: inches, the origin at the bottom left
// corner of the shape's box, y up.  The row types follow [MS-VSDX] (Geometry section):
// MoveTo, LineTo, ArcTo (bow A: positive turns counter-clockwise, the arc's middle lies right
// of the chord), EllipticalArcTo (through the control point A, B; C the angle of the major
// axis, D major over minor), Ellipse (centre X, Y; ends of the two axes A, B and C, D),
// NURBSTo (E = NURBS(knotLast, degree, xType, yType, x, y, knot, weight, ...): the current
// point is the first control point with knot C and weight D, X, Y the last with knot A and
// weight B; type 0 means a fraction of Width / Height), PolylineTo (A = POLYLINE(xType, yType,
// x, y, ...)), SplineStart / SplineKnot, InfiniteLine (drawn 1000 inches each way), and the
// relative rows RelMoveTo, RelLineTo, RelCubBezTo, RelQuadBezTo, RelEllipticalArcTo in
// fractions of Width and Height.  Visio fills only closed sub-paths (end = start).
//
// A shape's transform to its parent (MS-VSDX, "Shape Transform"): move the local pin to the
// origin, flip (FlipX mirrors x, FlipY mirrors y), turn by Angle, move to the pin -- the pin
// lies in the parent's coordinates (the page, or the group's local ones).

use alloc::vec::Vec;

use crate::document::{Document, ShapeRef, SheetRef};
use crate::names;
use crate::sheet::{CellAddr, SecKey};
use crate::strings::Sym;

/// an affine map: (x, y) -> (a x + c y + e, b x + d y + f)
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Xform {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Default for Xform {
    fn default() -> Xform {
        Xform::IDENTITY
    }
}

impl Xform {
    pub const IDENTITY: Xform = Xform { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    pub const fn translate(x: f64, y: f64) -> Xform {
        Xform { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: x, f: y }
    }

    pub const fn scale(sx: f64, sy: f64) -> Xform {
        Xform { a: sx, b: 0.0, c: 0.0, d: sy, e: 0.0, f: 0.0 }
    }

    /// a turn by `angle` radians, counter-clockwise (y up)
    pub fn rotate(angle: f64) -> Xform {
        let (s, c) = sincos(angle);
        Xform { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 }
    }

    /// first `self`, then `next`
    pub fn then(&self, next: &Xform) -> Xform {
        let m = next;
        Xform {
            a: m.a * self.a + m.c * self.b,
            b: m.b * self.a + m.d * self.b,
            c: m.a * self.c + m.c * self.d,
            d: m.b * self.c + m.d * self.d,
            e: m.a * self.e + m.c * self.f + m.e,
            f: m.b * self.e + m.d * self.f + m.f,
        }
    }

    pub fn apply(&self, p: [f64; 2]) -> [f64; 2] {
        [self.a * p[0] + self.c * p[1] + self.e, self.b * p[0] + self.d * p[1] + self.f]
    }

    /// a direction (without the translation)
    pub fn apply_vec(&self, v: [f64; 2]) -> [f64; 2] {
        [self.a * v[0] + self.c * v[1], self.b * v[0] + self.d * v[1]]
    }

    pub fn det(&self) -> f64 {
        self.a * self.d - self.b * self.c
    }

    /// the inverse (None: singular)
    pub fn inverse(&self) -> Option<Xform> {
        let det = self.det();
        if !(det.is_finite() && det.abs() > 1e-300) {
            return None;
        }
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Some(Xform { a, b, c, d, e: -(a * self.e + c * self.f), f: -(b * self.e + d * self.f) })
    }

    /// the angle the x axis is turned to (radians, counter-clockwise)
    pub fn angle(&self) -> f64 {
        atan2(self.b, self.a)
    }

    /// how much lengths grow (the square root of |det|)
    pub fn scale_factor(&self) -> f64 {
        sqrt(self.det().abs())
    }
}

/// a piece of a sub-path: to a point, or a cubic Bézier curve (two control points, the end)
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum Seg {
    Line([f64; 2]),
    Cubic([f64; 2], [f64; 2], [f64; 2]),
}

impl Seg {
    pub fn end(&self) -> [f64; 2] {
        match *self {
            Seg::Line(p) | Seg::Cubic(_, _, p) => p,
        }
    }

    fn map(&self, m: &Xform) -> Seg {
        match *self {
            Seg::Line(p) => Seg::Line(m.apply(p)),
            Seg::Cubic(a, b, p) => Seg::Cubic(m.apply(a), m.apply(b), m.apply(p)),
        }
    }
}

/// a sub-path: a start and pieces end to end; closed when it ends where it starts
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Sub {
    pub start: [f64; 2],
    pub segs: Vec<Seg>,
    pub closed: bool,
}

impl Sub {
    pub fn end(&self) -> [f64; 2] {
        self.segs.last().map_or(self.start, |s| s.end())
    }

    /// the same under a transform
    pub fn map(&self, m: &Xform) -> Sub {
        Sub { start: m.apply(self.start), segs: self.segs.iter().map(|s| s.map(m)).collect(), closed: self.closed }
    }

    /// the same run backwards
    pub fn reversed(&self) -> Sub {
        let mut segs = Vec::with_capacity(self.segs.len());
        let mut prev = self.start;
        let mut pts = Vec::with_capacity(self.segs.len());
        for s in &self.segs {
            pts.push((prev, *s));
            prev = s.end();
        }
        for (from, s) in pts.into_iter().rev() {
            segs.push(match s {
                Seg::Line(_) => Seg::Line(from),
                Seg::Cubic(a, b, _) => Seg::Cubic(b, a, from),
            });
        }
        Sub { start: self.end(), segs, closed: self.closed }
    }

    /// the box of its points and control points (x0, y0, x1, y1)
    pub fn hull(&self) -> [f64; 4] {
        let mut b = [self.start[0], self.start[1], self.start[0], self.start[1]];
        let mut add = |p: [f64; 2]| {
            b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
        };
        for s in &self.segs {
            match *s {
                Seg::Line(p) => add(p),
                Seg::Cubic(a, c, p) => {
                    add(a);
                    add(c);
                    add(p);
                }
            }
        }
        b
    }

    /// the points along it, curves cut into `per_curve` straight pieces
    pub fn flatten(&self, per_curve: usize, out: &mut Vec<[f64; 2]>) {
        out.push(self.start);
        let mut prev = self.start;
        for s in &self.segs {
            match *s {
                Seg::Line(p) => out.push(p),
                Seg::Cubic(a, b, p) => {
                    let n = per_curve.max(1);
                    for i in 1..=n {
                        out.push(cubic_at(prev, a, b, p, i as f64 / n as f64));
                    }
                }
            }
            prev = s.end();
        }
    }
}

/// a path: sub-paths drawn together (filled even-odd)
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Path {
    pub subs: Vec<Sub>,
}

impl Path {
    pub fn map(&self, m: &Xform) -> Path {
        Path { subs: self.subs.iter().map(|s| s.map(m)).collect() }
    }

    pub fn is_empty(&self) -> bool {
        self.subs.iter().all(|s| s.segs.is_empty())
    }

    /// the box of all points and control points (None: no points)
    pub fn hull(&self) -> Option<[f64; 4]> {
        let mut out: Option<[f64; 4]> = None;
        for s in &self.subs {
            let h = s.hull();
            out = Some(match out {
                None => h,
                Some(b) => [b[0].min(h[0]), b[1].min(h[1]), b[2].max(h[2]), b[3].max(h[3])],
            });
        }
        out
    }
}

/// one Geometry section of a shape: its flags and its sub-paths in local coordinates
#[derive(Clone, PartialEq, Debug, Default)]
pub struct GeoSection {
    /// IX of the section (Geometry1 is 0)
    pub index: u32,
    pub no_fill: bool,
    pub no_line: bool,
    pub no_show: bool,
    pub no_snap: bool,
    pub path: Path,
}

/// how far an infinite line is drawn each way (inches)
pub const INFINITE: f64 = 1000.0;

// ---------------------------------------------------------------- reading the rows

impl Document {
    /// the Geometry sections `sheet` really has (inherited ones included, deleted ones left out)
    /// with their rows as paths in local coordinates; `w` and `h` are the shape's Width and
    /// Height (the relative rows and the relative points of NURBS and POLYLINE need them)
    pub fn geometry_paths(&self, sheet: SheetRef, w: f64, h: f64) -> Vec<GeoSection> {
        self.geometry_tagged(sheet, w, h).into_iter().map(|(g, _)| g).collect()
    }

    /// (JD21) geometry_paths and, per sub-path, the IX of the row each piece comes from (a row
    /// that draws several pieces -- an arc, a NURBS curve, a polyline -- gives each of them its IX)
    pub fn geometry_tagged(&self, sheet: SheetRef, w: f64, h: f64) -> Vec<(GeoSection, Vec<Vec<u32>>)> {
        let mut out = Vec::new();
        for sec in self.geometry(sheet) {
            let flag = |c: Sym| self.resolve(sheet, &CellAddr::section(sec, c)).num().is_some_and(|v| v != 0.0);
            let mut g = GeoSection {
                index: sec.index,
                no_fill: flag(names::NO_FILL),
                no_line: flag(names::NO_LINE),
                no_show: flag(names::NO_SHOW),
                no_snap: flag(names::NO_SNAP),
                path: Path::default(),
            };
            let mut tags = Vec::new();
            self.section_path(sheet, sec, w, h, &mut g.path, &mut tags);
            out.push((g, tags));
        }
        out
    }

    fn section_path(&self, sheet: SheetRef, sec: SecKey, w: f64, h: f64, path: &mut Path, tags: &mut Vec<Vec<u32>>) {
        let rows = self.rows(sheet, sec);
        let mut b = Builder::new(path, tags);
        let mut spline: Option<Spline> = None;
        for row in &rows {
            let ix = match row.key {
                crate::sheet::RowKey::Index(i) => i,
                crate::sheet::RowKey::Named(_) => 0,
            };
            let kind = row.kind.map(|k| self.str(k)).unwrap_or("");
            let num = |c: Sym| self.resolve(sheet, &CellAddr::row(sec, row.key, c)).num();
            let text = |c: Sym| match self.resolve(sheet, &CellAddr::row(sec, row.key, c)).value() {
                crate::cell::Value::Text(s) => Some(self.str(s)),
                _ => None,
            };
            let x = num(names::X).unwrap_or(0.0);
            let y = num(names::Y).unwrap_or(0.0);
            let p = [x, y];
            // a spline goes on while SplineKnot rows follow
            if kind != "SplineKnot" {
                if let Some(s) = spline.take() {
                    s.finish(&mut b);
                }
            }
            b.tag = ix;
            match kind {
                "MoveTo" => b.move_to(p),
                "LineTo" => b.line_to(p),
                "ArcTo" => {
                    let a = num(names::A).unwrap_or(0.0);
                    let from = b.cur();
                    b.arc_to_bow(from, p, a);
                }
                "EllipticalArcTo" => {
                    let ctl = [num(names::A).unwrap_or(0.0), num(names::B).unwrap_or(0.0)];
                    let angle = num(names::C).unwrap_or(0.0);
                    let ratio = num(names::D).unwrap_or(1.0);
                    let from = b.cur();
                    b.elliptical_arc(from, ctl, p, angle, ratio, &Xform::IDENTITY);
                }
                "Ellipse" => {
                    let u = [num(names::A).unwrap_or(0.0) - x, num(names::B).unwrap_or(0.0) - y];
                    let v = [num(names::C).unwrap_or(0.0) - x, num(names::D).unwrap_or(0.0) - y];
                    b.ellipse(p, u, v);
                }
                "InfiniteLine" => {
                    let q = [num(names::A).unwrap_or(0.0), num(names::B).unwrap_or(0.0)];
                    let d = [q[0] - x, q[1] - y];
                    let l = sqrt(d[0] * d[0] + d[1] * d[1]);
                    if l > 0.0 && l.is_finite() {
                        let k = INFINITE / l;
                        b.move_to([x - d[0] * k, y - d[1] * k]);
                        b.line_to([x + d[0] * k, y + d[1] * k]);
                    }
                }
                "PolylineTo" => {
                    if let Some(args) = text(names::A).and_then(|t| parse_args(t, "POLYLINE")) {
                        let rel = (args.first().copied().unwrap_or(1.0) == 0.0, args.get(1).copied().unwrap_or(1.0) == 0.0);
                        for q in args.get(2..).unwrap_or(&[]).chunks_exact(2) {
                            b.line_to([if rel.0 { q[0] * w } else { q[0] }, if rel.1 { q[1] * h } else { q[1] }]);
                        }
                    }
                    b.line_to(p);
                }
                "NURBSTo" => {
                    let from = b.cur();
                    let e = text(names::E).and_then(|t| parse_args(t, "NURBS"));
                    match e.and_then(|args| Nurbs::from_row(&args, from, p, num(names::A), num(names::B), num(names::C), num(names::D), w, h)) {
                        Some(n) => {
                            let mut segs = Vec::new();
                            n.segments(&mut segs);
                            b.extend(&segs);
                            // the curve ends at its last control point only when clamped there
                            b.line_to_far(p);
                        }
                        None => b.line_to(p),
                    }
                }
                "SplineStart" => {
                    let from = b.cur();
                    spline = Some(Spline {
                        tag: ix,
                        degree: num(names::D).unwrap_or(3.0),
                        points: alloc::vec![from, p],
                        knots: alloc::vec![num(names::B).unwrap_or(0.0), num(names::A).unwrap_or(0.0)],
                        last: num(names::C).unwrap_or(0.0),
                    });
                }
                "SplineKnot" => match spline.as_mut() {
                    Some(s) => {
                        s.points.push(p);
                        s.knots.push(num(names::A).unwrap_or(0.0));
                    }
                    None => b.line_to(p),
                },
                "RelMoveTo" => b.move_to([x * w, y * h]),
                "RelLineTo" => b.line_to([x * w, y * h]),
                "RelCubBezTo" => {
                    let c1 = [num(names::A).unwrap_or(0.0) * w, num(names::B).unwrap_or(0.0) * h];
                    let c2 = [num(names::C).unwrap_or(0.0) * w, num(names::D).unwrap_or(0.0) * h];
                    b.cubic_to(c1, c2, [x * w, y * h]);
                }
                "RelQuadBezTo" => {
                    let c = [num(names::A).unwrap_or(0.0) * w, num(names::B).unwrap_or(0.0) * h];
                    let from = b.cur();
                    let to = [x * w, y * h];
                    b.cubic_to(lerp(from, c, 2.0 / 3.0), lerp(to, c, 2.0 / 3.0), to);
                }
                "RelEllipticalArcTo" => {
                    // the arc is defined in the relative space, then stretched by Width and Height
                    let from = b.cur();
                    if w != 0.0 && h != 0.0 {
                        let rel = [from[0] / w, from[1] / h];
                        let ctl = [num(names::A).unwrap_or(0.0), num(names::B).unwrap_or(0.0)];
                        b.elliptical_arc(rel, ctl, p, num(names::C).unwrap_or(0.0), num(names::D).unwrap_or(1.0), &Xform::scale(w, h));
                    } else {
                        b.line_to([x * w, y * h]);
                    }
                }
                // a row type this crate does not know: its point, if it has one
                _ => {
                    if num(names::X).is_some() || num(names::Y).is_some() {
                        b.line_to(p);
                    }
                }
            }
        }
        if let Some(s) = spline.take() {
            s.finish(&mut b);
        }
        b.finish();
    }

    /// the transform of a shape's local coordinates to its parent's (the page, or the local
    /// coordinates of its group): T(Pin) R(Angle) Flip T(-LocPin)
    pub fn local_xform(&self, sheet: SheetRef) -> Xform {
        let n = |c: Sym| self.resolve(sheet, &CellAddr::single(c)).num();
        let pin = [n(names::PIN_X).unwrap_or(0.0), n(names::PIN_Y).unwrap_or(0.0)];
        let loc = [n(names::LOC_PIN_X).unwrap_or(0.0), n(names::LOC_PIN_Y).unwrap_or(0.0)];
        let angle = n(names::ANGLE).unwrap_or(0.0);
        let fx = if n(names::FLIP_X).is_some_and(|v| v != 0.0) { -1.0 } else { 1.0 };
        let fy = if n(names::FLIP_Y).is_some_and(|v| v != 0.0) { -1.0 } else { 1.0 };
        let mut m = Xform::translate(-loc[0], -loc[1]).then(&Xform::scale(fx, fy));
        if angle != 0.0 && angle.is_finite() {
            m = m.then(&Xform::rotate(angle));
        }
        m.then(&Xform::translate(pin[0], pin[1]))
    }

    /// the transform of a shape's local coordinates to its page's (or master's): its own,
    /// then those of the groups around it
    pub fn page_xform(&self, shape: ShapeRef) -> Xform {
        let mut m = Xform::IDENTITY;
        let mut cur = Some(shape);
        let mut depth = 0;
        while let (Some(r), true) = (cur, depth < 1024) {
            m = m.then(&self.local_xform(SheetRef::Shape(r)));
            cur = self.shape(r).and_then(|s| s.parent());
            depth += 1;
        }
        m
    }
}

// ---------------------------------------------------------------- building paths

struct Builder<'p> {
    path: &'p mut Path,
    sub: Option<Sub>,
    /// the current point (also after an ellipse, which leaves no open sub-path)
    at: Option<[f64; 2]>,
    /// (JD21) the row the pieces come from now, and per sub-path the row of each piece
    tag: u32,
    tags: &'p mut Vec<Vec<u32>>,
    sub_tags: Vec<u32>,
}

impl<'p> Builder<'p> {
    fn new(path: &'p mut Path, tags: &'p mut Vec<Vec<u32>>) -> Builder<'p> {
        Builder { path, sub: None, at: None, tag: 0, tags, sub_tags: Vec::new() }
    }

    fn cur(&self) -> [f64; 2] {
        self.at.unwrap_or([0.0, 0.0])
    }

    fn flush(&mut self) {
        let tags = core::mem::take(&mut self.sub_tags);
        if let Some(mut s) = self.sub.take() {
            if !s.segs.is_empty() {
                s.closed = near(s.start, s.end());
                self.path.subs.push(s);
                self.tags.push(tags);
            }
        }
    }

    fn move_to(&mut self, p: [f64; 2]) {
        self.flush();
        self.sub = Some(Sub { start: p, segs: Vec::new(), closed: false });
        self.at = Some(p);
    }

    fn ensure(&mut self, p: [f64; 2]) {
        if self.sub.is_none() {
            // a section that does not start with MoveTo starts at its first point
            let start = self.at.unwrap_or(p);
            self.sub = Some(Sub { start, segs: Vec::new(), closed: false });
        }
    }

    fn push(&mut self, s: Seg) {
        let end = s.end();
        self.ensure(end);
        if let Some(sub) = self.sub.as_mut() {
            sub.segs.push(s);
            self.sub_tags.push(self.tag);
        }
        self.at = Some(end);
    }

    fn line_to(&mut self, p: [f64; 2]) {
        if self.sub.is_none() && self.at.is_none() {
            self.move_to(p);
            return;
        }
        self.push(Seg::Line(p));
    }

    /// a line to p unless the path is there already
    fn line_to_far(&mut self, p: [f64; 2]) {
        if self.at.is_none_or(|a| !near(a, p)) {
            self.line_to(p);
        }
    }

    fn cubic_to(&mut self, a: [f64; 2], b: [f64; 2], p: [f64; 2]) {
        self.push(Seg::Cubic(a, b, p));
    }

    fn extend(&mut self, segs: &[Seg]) {
        for s in segs {
            self.push(*s);
        }
    }

    /// ArcTo: a circular arc from `from` to `to` whose middle lies `bow` right of the chord
    fn arc_to_bow(&mut self, from: [f64; 2], to: [f64; 2], bow: f64) {
        let d = [to[0] - from[0], to[1] - from[1]];
        let l = sqrt(d[0] * d[0] + d[1] * d[1]);
        if !(bow.is_finite() && l > 0.0) || bow.abs() < 1e-9 * l.max(1e-9) {
            self.line_to(to);
            return;
        }
        let m = [(from[0] + to[0]) * 0.5 + d[1] / l * bow, (from[1] + to[1]) * 0.5 - d[0] / l * bow];
        let mut segs = Vec::new();
        if arc3(from, m, to, &mut segs) {
            self.extend(&segs);
        } else {
            self.line_to(to);
        }
    }

    /// EllipticalArcTo (also the relative one, then `out` stretches the relative space): an
    /// arc of an ellipse whose major axis turns `angle` and is `ratio` times its minor one,
    /// from `from` through `ctl` to `to`
    fn elliptical_arc(&mut self, from: [f64; 2], ctl: [f64; 2], to: [f64; 2], angle: f64, ratio: f64, out: &Xform) {
        let end = out.apply(to);
        if !(ratio.is_finite() && ratio > 1e-12 && angle.is_finite()) {
            self.line_to(end);
            return;
        }
        // into a space where the ellipse is a circle: turn back by angle, divide x by ratio
        let into = Xform::rotate(-angle).then(&Xform::scale(1.0 / ratio, 1.0));
        let back = Xform::scale(ratio, 1.0).then(&Xform::rotate(angle)).then(out);
        let mut segs = Vec::new();
        if arc3(into.apply(from), into.apply(ctl), into.apply(to), &mut segs) {
            let segs: Vec<Seg> = segs.iter().map(|s| s.map(&back)).collect();
            self.extend(&segs);
        } else {
            self.line_to(end);
        }
    }

    /// Ellipse: a closed sub-path of its own, centre c, conjugate half axes u and v
    fn ellipse(&mut self, c: [f64; 2], u: [f64; 2], v: [f64; 2]) {
        self.flush();
        let m = Xform { a: u[0], b: u[1], c: v[0], d: v[1], e: c[0], f: c[1] };
        let mut segs = Vec::new();
        arc_cubics([0.0, 0.0], 1.0, 0.0, core::f64::consts::TAU, &mut segs);
        let start = m.apply([1.0, 0.0]);
        let segs: Vec<Seg> = segs.iter().map(|s| s.map(&m)).collect();
        self.tags.push(alloc::vec![self.tag; segs.len()]);
        self.path.subs.push(Sub { start, segs, closed: true });
        self.sub = None;
        self.at = Some(c);
    }

    fn finish(&mut self) {
        self.flush();
    }
}

struct Spline {
    tag: u32,
    degree: f64,
    points: Vec<[f64; 2]>,
    knots: Vec<f64>,
    last: f64,
}

impl Spline {
    fn finish(self, b: &mut Builder) {
        b.tag = self.tag;
        let end = *self.points.last().unwrap_or(&[0.0, 0.0]);
        let deg = self.degree as usize;
        let mut knots = self.knots;
        knots.push(self.last);
        // (the degree comes from the document: past DEGREE_MAX the curve is not drawn, and no
        // knots are made for it -- 1e12 would have asked for 8 TB)
        if (1..=DEGREE_MAX).contains(&deg) {
            while knots.len() < self.points.len() + deg + 1 {
                knots.push(self.last);
            }
        }
        let n = Nurbs { degree: deg, weights: alloc::vec![1.0; self.points.len()], points: self.points, knots };
        if n.valid() {
            let mut segs = Vec::new();
            n.segments(&mut segs);
            b.extend(&segs);
        }
        b.line_to_far(end);
    }
}

// ---------------------------------------------------------------- arcs

/// a circular arc from `a` through `m` to `c` as cubic pieces (false: the points are on a line)
fn arc3(a: [f64; 2], m: [f64; 2], c: [f64; 2], out: &mut Vec<Seg>) -> bool {
    let Some((ctr, r)) = circle3(a, m, c) else { return false };
    let t0 = atan2(a[1] - ctr[1], a[0] - ctr[0]);
    let tm = atan2(m[1] - ctr[1], m[0] - ctr[0]);
    let t1 = atan2(c[1] - ctr[1], c[0] - ctr[0]);
    let tau = core::f64::consts::TAU;
    // counter-clockwise sweep from t0 to t1, and whether the middle lies on it
    let ccw = rem(t1 - t0, tau);
    let mid = rem(tm - t0, tau);
    let sweep = if mid <= ccw { ccw } else { ccw - tau };
    if sweep == 0.0 {
        return false;
    }
    let mut segs = Vec::new();
    arc_cubics(ctr, r, t0, sweep, &mut segs);
    // the exact end, so that closed paths close
    if let Some(Seg::Cubic(_, _, p)) = segs.last_mut() {
        *p = c;
    }
    out.extend(segs);
    true
}

/// the circle through three points (None: on a line, or not finite)
pub(crate) fn circle3(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Option<([f64; 2], f64)> {
    let (bx, by) = (b[0] - a[0], b[1] - a[1]);
    let (cx, cy) = (c[0] - a[0], c[1] - a[1]);
    let d = 2.0 * (bx * cy - by * cx);
    let scale = (bx * bx + by * by).max(cx * cx + cy * cy);
    if !(d.abs() > 1e-12 * scale) {
        return None;
    }
    let b2 = bx * bx + by * by;
    let c2 = cx * cx + cy * cy;
    let ux = (cy * b2 - by * c2) / d;
    let uy = (bx * c2 - cx * b2) / d;
    let r = sqrt(ux * ux + uy * uy);
    (r.is_finite() && ux.is_finite() && uy.is_finite()).then_some(([a[0] + ux, a[1] + uy], r))
}

/// a circular arc (centre, radius, start angle, sweep in radians, positive counter-clockwise)
/// as cubic pieces of at most 90 degrees
pub fn arc_cubics(c: [f64; 2], r: f64, start: f64, sweep: f64, out: &mut Vec<Seg>) {
    let n = ceil((sweep.abs() / core::f64::consts::FRAC_PI_2) - 1e-9).max(1.0) as usize;
    let n = n.min(64);
    let step = sweep / n as f64;
    let k = 4.0 / 3.0 * tan(step / 4.0);
    let mut t = start;
    let (mut s0, mut c0) = sincos(t);
    for _ in 0..n {
        let t1 = t + step;
        let (s1, c1) = sincos(t1);
        let p0 = [c[0] + r * c0, c[1] + r * s0];
        let p1 = [c[0] + r * c1, c[1] + r * s1];
        let a = [p0[0] - k * r * s0, p0[1] + k * r * c0];
        let b = [p1[0] + k * r * s1, p1[1] - k * r * c1];
        out.push(Seg::Cubic(a, b, p1));
        t = t1;
        s0 = s1;
        c0 = c1;
    }
}

// ---------------------------------------------------------------- the ellipse of an arc row (JD21)

/// An ellipse and an arc of it: the points c + u cos t + v sin t for t from `t0` to `t0 + sweep`
/// (radians, positive counter-clockwise from u towards v; u and v are conjugate half axes, the
/// half axes themselves for an upright ellipse).  An affine map keeps the form: map() maps c, u
/// and v and the parameters stay, so an arc of a turned, flipped or stretched shape is a Conic
/// on the page too.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Conic {
    pub c: [f64; 2],
    pub u: [f64; 2],
    pub v: [f64; 2],
    pub t0: f64,
    pub sweep: f64,
}

impl Conic {
    /// the point at parameter t
    pub fn at(&self, t: f64) -> [f64; 2] {
        let (s, c) = sincos(t);
        [self.c[0] + self.u[0] * c + self.v[0] * s, self.c[1] + self.u[1] * c + self.v[1] * s]
    }

    /// the whole ellipse (a sweep of a full turn)
    pub fn is_full(&self) -> bool {
        self.sweep.abs() >= core::f64::consts::TAU - 1e-9
    }

    /// the same under an affine map
    pub fn map(&self, m: &Xform) -> Conic {
        Conic { c: m.apply(self.c), u: m.apply_vec(self.u), v: m.apply_vec(self.v), ..*self }
    }

    /// the parameter of the point of the ellipse that `p` projects to from the centre (in the
    /// space where the ellipse is the unit circle); None for a degenerate ellipse
    pub fn param(&self, p: [f64; 2]) -> Option<f64> {
        let m = Xform { a: self.u[0], b: self.u[1], c: self.v[0], d: self.v[1], e: self.c[0], f: self.c[1] };
        let q = m.inverse()?.apply(p);
        (q[0] != 0.0 || q[1] != 0.0).then(|| atan2(q[1], q[0]))
    }

    /// the principal axes: the angle of the major axis (radians), the major and the minor half
    /// length (the singular value decomposition of the matrix [u v])
    pub fn axes(&self) -> (f64, f64, f64) {
        let (a, b, c, d) = (self.u[0], self.v[0], self.u[1], self.v[1]);
        let (e, f, g, h) = ((a + d) * 0.5, (a - d) * 0.5, (c + b) * 0.5, (c - b) * 0.5);
        let q = sqrt(e * e + h * h);
        let r = sqrt(f * f + g * g);
        let phi = (atan2(h, e) + atan2(g, f)) * 0.5;
        (phi, q + r, (q - r).abs())
    }

    /// the box of the arc (x0, y0, x1, y1): its ends and the extremes it passes
    pub fn bounds(&self) -> [f64; 4] {
        let (p, q) = (self.at(self.t0), self.at(self.t0 + self.sweep));
        let mut b = [p[0].min(q[0]), p[1].min(q[1]), p[0].max(q[0]), p[1].max(q[1])];
        let tau = core::f64::consts::TAU;
        for k in 0..2 {
            let base = atan2(self.v[k], self.u[k]);
            for t in [base, base + core::f64::consts::PI] {
                // whether t lies on the arc
                let d = if self.sweep >= 0.0 { rem(t - self.t0, tau) } else { rem(self.t0 - t, tau) };
                if d <= self.sweep.abs() || self.is_full() {
                    let x = self.at(t);
                    b = [b[0].min(x[0]), b[1].min(x[1]), b[2].max(x[0]), b[3].max(x[1])];
                }
            }
        }
        b
    }
}

// an arc from a through m to c as a Conic of the circle through them (None: on a line)
fn conic3(a: [f64; 2], m: [f64; 2], c: [f64; 2]) -> Option<Conic> {
    let (ctr, r) = circle3(a, m, c)?;
    let tau = core::f64::consts::TAU;
    let t0 = atan2(a[1] - ctr[1], a[0] - ctr[0]);
    let tm = atan2(m[1] - ctr[1], m[0] - ctr[0]);
    let t1 = atan2(c[1] - ctr[1], c[0] - ctr[0]);
    let ccw = rem(t1 - t0, tau);
    let mid = rem(tm - t0, tau);
    let sweep = if mid <= ccw { ccw } else { ccw - tau };
    (sweep != 0.0).then_some(Conic { c: ctr, u: [r, 0.0], v: [0.0, r], t0, sweep })
}

impl Document {
    /// (JD21) the ellipse arc a Geometry row draws, in local coordinates, the way section_path
    /// draws it: ArcTo, EllipticalArcTo and RelEllipticalArcTo from `from` (the current point) to
    /// their X, Y, Ellipse as a whole; `w`, `h` the shape's Width and Height.  None for other rows
    /// and for arcs that are drawn as a line (a bow of 0, three points on a line)
    pub fn row_conic(&self, sheet: SheetRef, sec: SecKey, key: crate::sheet::RowKey, from: [f64; 2], w: f64, h: f64) -> Option<Conic> {
        let row = self.rows(sheet, sec).into_iter().find(|r| r.key == key)?;
        let kind = row.kind.map(|k| self.str(k)).unwrap_or("");
        let num = |c: Sym| self.resolve(sheet, &CellAddr::row(sec, key, c)).num();
        let (x, y) = (num(names::X).unwrap_or(0.0), num(names::Y).unwrap_or(0.0));
        let (a, b, c, d) = (num(names::A), num(names::B), num(names::C), num(names::D));
        match kind {
            "ArcTo" => {
                let (to, bow) = ([x, y], a.unwrap_or(0.0));
                let dd = [to[0] - from[0], to[1] - from[1]];
                let l = sqrt(dd[0] * dd[0] + dd[1] * dd[1]);
                if !(bow.is_finite() && l > 0.0) || bow.abs() < 1e-9 * l.max(1e-9) {
                    return None;
                }
                let m = [(from[0] + to[0]) * 0.5 + dd[1] / l * bow, (from[1] + to[1]) * 0.5 - dd[0] / l * bow];
                conic3(from, m, to)
            }
            "EllipticalArcTo" | "RelEllipticalArcTo" => {
                let rel = kind == "RelEllipticalArcTo";
                let (angle, ratio) = (c.unwrap_or(0.0), d.unwrap_or(1.0));
                if !(ratio.is_finite() && ratio > 1e-12 && angle.is_finite()) || (rel && (w == 0.0 || h == 0.0)) {
                    return None;
                }
                let out = if rel { Xform::scale(w, h) } else { Xform::IDENTITY };
                let from = if rel { [from[0] / w, from[1] / h] } else { from };
                let ctl = [a.unwrap_or(0.0), b.unwrap_or(0.0)];
                let into = Xform::rotate(-angle).then(&Xform::scale(1.0 / ratio, 1.0));
                let back = Xform::scale(ratio, 1.0).then(&Xform::rotate(angle)).then(&out);
                Some(conic3(into.apply(from), into.apply(ctl), into.apply([x, y]))?.map(&back))
            }
            "Ellipse" => {
                let u = [a.unwrap_or(0.0) - x, b.unwrap_or(0.0) - y];
                let v = [c.unwrap_or(0.0) - x, d.unwrap_or(0.0) - y];
                Some(Conic { c: [x, y], u, v, t0: 0.0, sweep: core::f64::consts::TAU })
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------- NURBS

/// the numbers of a NURBS(...) or POLYLINE(...) value (None: not that function, or a part that
/// is no number)
pub fn parse_args(text: &str, name: &str) -> Option<Vec<f64>> {
    let t = text.trim();
    let open = t.find('(')?;
    if !t[..open].trim().eq_ignore_ascii_case(name) {
        return None;
    }
    let inner = t[open + 1..].trim_end().strip_suffix(')')?;
    let mut out = Vec::new();
    for part in inner.split(',') {
        let p = part.trim();
        if p.is_empty() {
            continue;
        }
        let v: f64 = p.parse().ok()?;
        if !v.is_finite() {
            return None;
        }
        out.push(v);
    }
    Some(out)
}

/// the highest degree of a spline or NURBS that is drawn
const DEGREE_MAX: usize = 9;

/// a B-spline (rational where the weights differ) on a full knot vector
#[derive(Clone, Debug, PartialEq)]
pub struct Nurbs {
    pub degree: usize,
    pub points: Vec<[f64; 2]>,
    pub weights: Vec<f64>,
    pub knots: Vec<f64>,
}

impl Nurbs {
    /// a NURBSTo row: E's numbers, the current point, the end (X, Y), A, B, C, D
    #[allow(clippy::too_many_arguments)]
    fn from_row(args: &[f64], from: [f64; 2], to: [f64; 2], a: Option<f64>, b: Option<f64>, c: Option<f64>, d: Option<f64>, w: f64, h: f64) -> Option<Nurbs> {
        let (&last, rest) = args.split_first()?;
        let degree = *rest.first()? as usize;
        // checked before the knots are filled up to it (valid() checks it again)
        if !(1..=DEGREE_MAX).contains(&degree) {
            return None;
        }
        let rel =(rest.get(1).copied().unwrap_or(1.0) == 0.0, rest.get(2).copied().unwrap_or(1.0) == 0.0);
        let quads = rest.get(3..).unwrap_or(&[]);
        let mut points = alloc::vec![from];
        let mut knots = alloc::vec![c.unwrap_or(0.0)];
        let mut weights = alloc::vec![d.unwrap_or(1.0)];
        for q in quads.chunks_exact(4) {
            points.push([if rel.0 { q[0] * w } else { q[0] }, if rel.1 { q[1] * h } else { q[1] }]);
            knots.push(q[2]);
            weights.push(q[3]);
        }
        points.push(to);
        knots.push(a.unwrap_or(last));
        weights.push(b.unwrap_or(1.0));
        knots.push(last);
        while knots.len() < points.len() + degree + 1 {
            knots.push(last);
        }
        let n = Nurbs { degree, points, weights, knots };
        n.valid().then_some(n)
    }

    /// degree 1 to 9, a positive weight per point, n + degree + 1 knots that do not fall
    pub fn valid(&self) -> bool {
        let n = self.points.len();
        (1..=DEGREE_MAX).contains(&self.degree)
            && n > self.degree
            && n <= 100_000
            && self.weights.len() == n
            && self.knots.len() == n + self.degree + 1
            && self.weights.iter().all(|w| w.is_finite() && *w > 0.0)
            && self.knots.iter().all(|k| k.is_finite())
            && self.knots.windows(2).all(|k| k[0] <= k[1])
            && self.knots[self.degree] < self.knots[n]
            && self.points.iter().all(|p| p[0].is_finite() && p[1].is_finite())
    }

    /// clamped at both ends, every interior knot strictly inside and at most `degree` times
    fn clamped(&self) -> bool {
        let p = self.degree;
        let n = self.points.len();
        let (lo, hi) = (self.knots[p], self.knots[n]);
        let ends = self.knots[..=p].iter().all(|&k| k == lo) && self.knots[n..].iter().all(|&k| k == hi);
        let inner = &self.knots[p + 1..n];
        ends && inner.iter().all(|&u| u > lo && u < hi && inner.iter().filter(|&&k| k == u).count() <= p)
    }

    /// the curve as pieces: exact cubic Béziers for a clamped polynomial spline of degree 1 to
    /// 3 (knots inserted until every piece is a Bézier), else straight pieces sampled on every
    /// knot span; the first piece starts at the curve's start
    pub fn segments(&self, out: &mut Vec<Seg>) {
        let polynomial = self.weights.iter().all(|&w| (w - self.weights[0]).abs() <= 1e-12 * self.weights[0]);
        if polynomial && self.degree <= 3 && self.clamped() {
            self.bezier_pieces(out);
        } else {
            self.sampled(out);
        }
    }

    fn bezier_pieces(&self, out: &mut Vec<Seg>) {
        let p = self.degree;
        let mut pts = self.points.clone();
        let mut knots = self.knots.clone();
        // every interior knot to multiplicity p
        let interior: Vec<f64> = {
            let n = pts.len();
            let mut v: Vec<f64> = knots[p + 1..n].to_vec();
            v.dedup();
            v
        };
        for u in interior {
            let mult = knots.iter().filter(|&&k| k == u).count();
            for _ in mult..p {
                insert_knot(&mut pts, &mut knots, p, u);
            }
        }
        // now pieces of p + 1 points sharing their ends; spans of length 0 give no piece
        let mut i = 0;
        while i + p < pts.len() {
            let piece = &pts[i..=i + p];
            match p {
                1 => out.push(Seg::Line(piece[1])),
                2 => out.push(Seg::Cubic(lerp(piece[0], piece[1], 2.0 / 3.0), lerp(piece[2], piece[1], 2.0 / 3.0), piece[2])),
                _ => out.push(Seg::Cubic(piece[1], piece[2], piece[3])),
            }
            i += p;
        }
    }

    fn sampled(&self, out: &mut Vec<Seg>) {
        let p = self.degree;
        let n = self.points.len();
        for span in p..n {
            let (a, b) = (self.knots[span], self.knots[span + 1]);
            if b <= a {
                continue;
            }
            for i in 1..=16 {
                let t = a + (b - a) * i as f64 / 16.0;
                out.push(Seg::Line(self.at(t)));
            }
        }
    }

    /// the point at parameter t (de Boor, in homogeneous coordinates)
    pub fn at(&self, t: f64) -> [f64; 2] {
        let p = self.degree;
        let n = self.points.len();
        let t = t.clamp(self.knots[p], self.knots[n]);
        let mut k = p;
        while k + 1 < n && self.knots[k + 1] <= t {
            k += 1;
        }
        let mut d: Vec<[f64; 3]> = (0..=p)
            .map(|j| {
                let i = k + j - p;
                let w = self.weights[i];
                [self.points[i][0] * w, self.points[i][1] * w, w]
            })
            .collect();
        for r in 1..=p {
            for j in (r..=p).rev() {
                let i = k + j - p;
                let den = self.knots[i + p + 1 - r] - self.knots[i];
                let alpha = if den > 0.0 { (t - self.knots[i]) / den } else { 0.0 };
                for c in 0..3 {
                    d[j][c] = (1.0 - alpha) * d[j - 1][c] + alpha * d[j][c];
                }
            }
        }
        let q = d[p];
        if q[2] != 0.0 { [q[0] / q[2], q[1] / q[2]] } else { [q[0], q[1]] }
    }
}

/// Boehm's knot insertion (polynomial points)
fn insert_knot(pts: &mut Vec<[f64; 2]>, knots: &mut Vec<f64>, p: usize, u: f64) {
    let n = pts.len();
    let mut k = p;
    while k + 1 < n && knots[k + 1] <= u {
        k += 1;
    }
    let mut new = Vec::with_capacity(n + 1);
    new.extend_from_slice(&pts[..=k - p]);
    for i in k - p + 1..=k {
        let den = knots[i + p] - knots[i];
        let a = if den > 0.0 { (u - knots[i]) / den } else { 0.0 };
        new.push(lerp(pts[i - 1], pts[i], a));
    }
    new.extend_from_slice(&pts[k..]);
    *pts = new;
    knots.insert(k + 1, u);
}

// ---------------------------------------------------------------- corners

/// the corners between two straight pieces rounded with radius r (the Rounding cell; at most
/// half of each piece is taken), in every sub-path
pub fn round_corners(path: &Path, r: f64) -> Path {
    if !(r > 0.0 && r.is_finite()) {
        return path.clone();
    }
    Path { subs: path.subs.iter().map(|s| round_sub(s, r)).collect() }
}

fn round_sub(s: &Sub, r: f64) -> Sub {
    let n = s.segs.len();
    if n < 2 || !s.segs.iter().any(|g| matches!(g, Seg::Line(_))) {
        return s.clone();
    }
    // the corner at the end of piece i (between i and i + 1; for a closed path also between
    // the last and the first)
    let starts: Vec<[f64; 2]> = core::iter::once(s.start).chain(s.segs.iter().map(|g| g.end())).collect();
    let line = |i: usize| matches!(s.segs[i], Seg::Line(_));
    let corner = |i: usize| -> Option<([f64; 2], [f64; 2], [f64; 2], [f64; 2])> {
        let j = if i + 1 < n { i + 1 } else if s.closed { 0 } else { return None };
        if !(line(i) && line(j)) {
            return None;
        }
        let (a, v, b) = (starts[i], starts[i + 1], s.segs[j].end());
        let d1 = [a[0] - v[0], a[1] - v[1]];
        let d2 = [b[0] - v[0], b[1] - v[1]];
        let (l1, l2) = (sqrt(d1[0] * d1[0] + d1[1] * d1[1]), sqrt(d2[0] * d2[0] + d2[1] * d2[1]));
        if l1 <= 0.0 || l2 <= 0.0 {
            return None;
        }
        let u1 = [d1[0] / l1, d1[1] / l1];
        let u2 = [d2[0] / l2, d2[1] / l2];
        let cos = (u1[0] * u2[0] + u1[1] * u2[1]).clamp(-1.0, 1.0);
        let half = acos(cos) * 0.5;
        if !(half > 1e-6 && half < core::f64::consts::FRAC_PI_2 - 1e-6) {
            return None;
        }
        // the tangent points' distance from the vertex
        let mut t = r / tan(half);
        t = t.min(l1 * 0.5).min(l2 * 0.5);
        let p1 = [v[0] + u1[0] * t, v[1] + u1[1] * t];
        let p2 = [v[0] + u2[0] * t, v[1] + u2[1] * t];
        // the cubic of a circular arc between the tangent points
        let sweep = core::f64::consts::PI - 2.0 * half;
        let k = 4.0 / 3.0 * tan(sweep / 4.0) * (t * tan(half));
        let c1 = [p1[0] - u1[0] * k, p1[1] - u1[1] * k];
        let c2 = [p2[0] - u2[0] * k, p2[1] - u2[1] * k];
        Some((p1, c1, c2, p2))
    };
    let corners: Vec<Option<_>> = (0..n).map(corner).collect();
    let mut out = Sub { start: s.start, segs: Vec::with_capacity(n * 2), closed: s.closed };
    // a closed path whose last corner is rounded starts at that corner's end point
    if let Some(Some((_, _, _, p2))) = corners.last().copied().filter(|_| s.closed) {
        out.start = p2;
    }
    for i in 0..n {
        match corners[i] {
            Some((p1, c1, c2, p2)) => {
                out.segs.push(Seg::Line(p1));
                out.segs.push(Seg::Cubic(c1, c2, p2));
            }
            None => out.segs.push(s.segs[i]),
        }
    }
    out
}

// ---------------------------------------------------------------- small maths

fn near(a: [f64; 2], b: [f64; 2]) -> bool {
    let tol = 1e-6 * (1.0 + a[0].abs().max(a[1].abs()));
    (a[0] - b[0]).abs() <= tol && (a[1] - b[1]).abs() <= tol
}

pub(crate) fn lerp(a: [f64; 2], b: [f64; 2], t: f64) -> [f64; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// the point of a cubic Bézier curve at t
pub fn cubic_at(p0: [f64; 2], p1: [f64; 2], p2: [f64; 2], p3: [f64; 2], t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    [a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0], a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1]]
}

// libm-free maths for no_std (the crate has no dependencies): f64 methods that need std are
// not available, so the few functions are written out
pub(crate) fn sqrt(x: f64) -> f64 {
    if !(x > 0.0) {
        return if x == 0.0 { 0.0 } else { f64::NAN };
    }
    if !x.is_finite() {
        return x;
    }
    // Newton from a guess by halving the exponent
    let bits = x.to_bits();
    let mut y = f64::from_bits((bits >> 1) + (1023u64 << 51));
    for _ in 0..6 {
        y = 0.5 * (y + x / y);
    }
    y
}

/// (sin, cos), with the argument reduced to [-pi/4, pi/4]
pub(crate) fn sincos(x: f64) -> (f64, f64) {
    if !x.is_finite() {
        return (f64::NAN, f64::NAN);
    }
    let half_pi = core::f64::consts::FRAC_PI_2;
    let k = round(x / half_pi);
    // two-part reduction keeps the error small for angles of a few turns
    let r = (x - k * 1.570_796_326_794_896_558) - k * 6.123_233_995_736_766e-17;
    let r2 = r * r;
    let s = r * (1.0 + r2 * (-1.0 / 6.0 + r2 * (1.0 / 120.0 + r2 * (-1.0 / 5040.0 + r2 * (1.0 / 362_880.0 + r2 * (-1.0 / 39_916_800.0 + r2 * (1.0 / 6_227_020_800.0 - r2 / 1_307_674_368_000.0)))))));
    let c = 1.0 + r2 * (-0.5 + r2 * (1.0 / 24.0 + r2 * (-1.0 / 720.0 + r2 * (1.0 / 40320.0 + r2 * (-1.0 / 3_628_800.0 + r2 * (1.0 / 479_001_600.0 + r2 * (-1.0 / 87_178_291_200.0 + r2 / 20_922_789_888_000.0)))))));
    match (k as i64).rem_euclid(4) {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    }
}

pub(crate) fn round(x: f64) -> f64 {
    if !(x.abs() < 4.5e15) {
        return x;
    }
    let t = x as i64 as f64;
    let f = x - t;
    if f >= 0.5 {
        t + 1.0
    } else if f <= -0.5 {
        t - 1.0
    } else {
        t
    }
}

/// the largest whole number not above x (no_std: f64::floor needs std)
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

/// x modulo m in [0, m)
pub(crate) fn rem(x: f64, m: f64) -> f64 {
    let r = x - floor(x / m) * m;
    if r >= m { r - m } else if r < 0.0 { r + m } else { r }
}

pub(crate) fn tan(x: f64) -> f64 {
    let (s, c) = sincos(x);
    s / c
}

/// atan for |x| <= 1 by argument halving and a series
fn atan_small(x: f64) -> f64 {
    // atan(x) = 2 atan(x / (1 + sqrt(1 + x^2))), twice, then the series
    let mut y = x;
    let mut mul = 1.0;
    for _ in 0..3 {
        y /= 1.0 + sqrt(1.0 + y * y);
        mul *= 2.0;
    }
    let y2 = y * y;
    let mut term = y;
    let mut sum = y;
    let mut k = 1.0;
    for _ in 0..12 {
        term *= -y2;
        k += 2.0;
        sum += term / k;
    }
    sum * mul
}

pub(crate) fn atan2(y: f64, x: f64) -> f64 {
    use core::f64::consts::{FRAC_PI_2, PI};
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    if !(x.is_finite() && y.is_finite()) {
        return f64::NAN;
    }
    if x.abs() >= y.abs() {
        let a = atan_small(y / x);
        if x > 0.0 {
            a
        } else if y >= 0.0 {
            a + PI
        } else {
            a - PI
        }
    } else {
        let a = atan_small(x / y);
        if y > 0.0 { FRAC_PI_2 - a } else { -FRAC_PI_2 - a }
    }
}

pub(crate) fn acos(x: f64) -> f64 {
    let x = x.clamp(-1.0, 1.0);
    atan2(sqrt((1.0 - x) * (1.0 + x)), x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn maths() {
        for i in -40..40 {
            let x = i as f64 * 0.37;
            let (s, c) = sincos(x);
            assert!(close(s * s + c * c, 1.0, 1e-14), "{x} {s} {c}");
            assert!(close(atan2(s, c), x - core::f64::consts::TAU * round(x / core::f64::consts::TAU), 1e-12), "{x} {}", atan2(s, c));
        }
        assert!(close(sincos(core::f64::consts::FRAC_PI_6).0, 0.5, 1e-15));
        assert!(close(sqrt(2.0) * sqrt(2.0), 2.0, 1e-15));
        assert!(close(acos(0.5), core::f64::consts::FRAC_PI_3, 1e-14));
    }

    #[test]
    fn xforms_compose() {
        let m = Xform::translate(-1.0, -1.0).then(&Xform::rotate(core::f64::consts::FRAC_PI_2)).then(&Xform::translate(5.0, 5.0));
        let p = m.apply([2.0, 1.0]);
        assert!(close(p[0], 5.0, 1e-12) && close(p[1], 6.0, 1e-12));
        let inv = m.inverse().unwrap();
        let q = inv.apply(p);
        assert!(close(q[0], 2.0, 1e-12) && close(q[1], 1.0, 1e-12));
    }

    #[test]
    fn bow_turns_counter_clockwise_when_positive() {
        // a rounded corner of a counter-clockwise box: from (0, 0) to (1, 1), bulging outwards
        let mut path = Path::default();
        let mut tags = Vec::new();
        let mut b = Builder::new(&mut path, &mut tags);
        b.move_to([0.0, 0.0]);
        b.arc_to_bow([0.0, 0.0], [1.0, 1.0], 1.0 - core::f64::consts::FRAC_1_SQRT_2);
        b.finish();
        let mut pts = Vec::new();
        path.subs[0].flatten(8, &mut pts);
        // the middle of the arc lies at (1 - cos 45, sin 45) of the circle around (0, 1)
        let mid = pts[pts.len() / 2];
        assert!(close(mid[0], core::f64::consts::FRAC_1_SQRT_2, 1e-3) && close(mid[1], 1.0 - core::f64::consts::FRAC_1_SQRT_2, 1e-3), "{mid:?}");
        for p in pts {
            assert!(close(dist(p, [0.0, 1.0]), 1.0, 1e-3), "{p:?}");
        }
    }

    fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
        sqrt((a[0] - b[0]) * (a[0] - b[0]) + (a[1] - b[1]) * (a[1] - b[1]))
    }

    #[test]
    fn spline_degree_from_the_file_is_checked_first() {
        // NURBSTo's E and SplineStart's D name the degree: the knots were filled up to it before
        // valid() looked at it (1e12 asked for 8 TB, third review); now such a curve is not
        // drawn and the row ends in a straight line
        for d in [1e12, f64::INFINITY, f64::NAN, 0.0, 10.0] {
            assert!(Nurbs::from_row(&[1.0, d, 0.0, 0.0], [0.0, 0.0], [1.0, 1.0], None, None, None, None, 1.0, 1.0).is_none(), "{d}");
            let mut path = Path::default();
            let mut b = Builder::new(&mut path);
            b.move_to([0.0, 0.0]);
            Spline { degree: d, points: alloc::vec![[0.0, 0.0], [1.0, 1.0]], knots: alloc::vec![0.0], last: 1.0 }.finish(&mut b);
            b.finish();
            assert!(!path.subs.is_empty(), "{d}");
        }
    }

    #[test]
    fn nurbs_bezier_pieces_match_de_boor() {
        // two cubic pieces on clamped knots
        let n = Nurbs {
            degree: 3,
            points: alloc::vec![[0.0, 0.0], [1.0, 2.0], [2.0, 2.0], [3.0, 0.0], [4.0, -1.0], [5.0, 1.0], [6.0, 0.0]],
            weights: alloc::vec![1.0; 7],
            knots: alloc::vec![0.0, 0.0, 0.0, 0.0, 0.4, 0.4, 0.7, 1.0, 1.0, 1.0, 1.0],
        };
        assert!(n.valid());
        let mut segs = Vec::new();
        n.segments(&mut segs);
        let sub = Sub { start: [0.0, 0.0], segs, closed: false };
        let mut pts = Vec::new();
        sub.flatten(32, &mut pts);
        // every flattened point lies near the curve: compare with dense de Boor samples
        let samples: Vec<[f64; 2]> = (0..=2000).map(|i| n.at(i as f64 / 2000.0)).collect();
        for p in &pts {
            let d = samples.iter().map(|q| dist(*p, *q)).fold(f64::MAX, f64::min);
            assert!(d < 5e-3, "{p:?} {d}");
        }
        assert!(close(sub.end()[0], 6.0, 1e-12) && close(sub.end()[1], 0.0, 1e-12));
    }

    #[test]
    fn nurbs_row_of_the_visio_kind() {
        // NURBS(1, 3, 0, 0, ...): knots C, k1 .. kn, A, last, padded with `degree` copies of last
        let args = parse_args("NURBS(1, 3, 0, 0, 0.25,1,0,1, 0.75,1,0,1)", "NURBS").unwrap();
        let n = Nurbs::from_row(&args, [0.0, 0.0], [2.0, 0.0], Some(0.0), Some(1.0), Some(0.0), Some(1.0), 2.0, 1.0).unwrap();
        assert_eq!(n.knots, alloc::vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]);
        assert_eq!(n.points[1], [0.5, 1.0]);
        let mut segs = Vec::new();
        n.segments(&mut segs);
        assert_eq!(segs, alloc::vec![Seg::Cubic([0.5, 1.0], [1.5, 1.0], [2.0, 0.0])]);
    }

    #[test]
    fn rational_nurbs_is_sampled_on_the_curve() {
        // a quarter circle as a rational quadratic
        let w = core::f64::consts::FRAC_1_SQRT_2;
        let n = Nurbs { degree: 2, points: alloc::vec![[1.0, 0.0], [1.0, 1.0], [0.0, 1.0]], weights: alloc::vec![1.0, w, 1.0], knots: alloc::vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0] };
        let mut segs = Vec::new();
        n.segments(&mut segs);
        assert_eq!(segs.len(), 16);
        for s in segs {
            let p = s.end();
            assert!(close(dist(p, [0.0, 0.0]), 1.0, 1e-12), "{p:?}");
        }
    }

    #[test]
    fn corners_round_into_arcs() {
        let sq = Path { subs: alloc::vec![Sub { start: [0.0, 0.0], segs: alloc::vec![Seg::Line([2.0, 0.0]), Seg::Line([2.0, 2.0]), Seg::Line([0.0, 2.0]), Seg::Line([0.0, 0.0])], closed: true }] };
        let r = round_corners(&sq, 0.5);
        let s = &r.subs[0];
        assert_eq!(s.segs.len(), 8);
        assert!(close(s.start[0], 0.5, 1e-12) && close(s.start[1], 0.0, 1e-12), "{:?}", s.start);
        assert!(close(s.end()[0], s.start[0], 1e-12) && close(s.end()[1], s.start[1], 1e-12));
        // the corner at (2, 0): the arc's middle 0.5 (1 - 1/sqrt 2) * sqrt 2 inside
        let mut pts = Vec::new();
        Sub { start: [1.5, 0.0], segs: alloc::vec![s.segs[1]], closed: false }.flatten(2, &mut pts);
        let c = [1.5, 0.5];
        assert!(close(dist(pts[1], c), 0.5, 1e-3), "{:?}", pts[1]);
    }

    #[test]
    fn args_parse() {
        assert_eq!(parse_args(" POLYLINE(0, 0, 0.5,1, 1,0.5)", "POLYLINE"), Some(alloc::vec![0.0, 0.0, 0.5, 1.0, 1.0, 0.5]));
        assert_eq!(parse_args("NURBS(1, 3)", "POLYLINE"), None);
        assert_eq!(parse_args("NURBS(1, x)", "NURBS"), None);
    }
}
