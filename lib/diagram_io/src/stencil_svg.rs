// stencil_svg -- an SVG drawing as Geometry sections of a stencil master (JD9, README "svg:")
//
//   import(file, text) -> Art { vb, layers }   every drawn element as sub-paths of lines and
//                                 cubic Bézier curves in fractions of the shape's box (x right,
//                                 y up), in layers of one format (fill, line, line width):
//                                 consecutive elements of the same format share one, so the
//                                 layers keep the document's order
//
// What it takes: path (M L H V C S Q T A Z, both cases), rect (rx, ry), circle, ellipse, line,
// polyline, polygon, g; transform on any of them (translate, scale, matrix, rotate, skewX,
// skewY); fill, stroke, stroke-width, fill-opacity, stroke-opacity, opacity, stroke-linecap,
// display as attributes or in style="..."; colours #rgb, #rrggbb, rgb(), a few names.  title,
// desc, metadata and elements of other namespaces (editor data) are skipped; everything else
// (text, image, use, gradients ...) is an error naming the line.  Quadratic curves are raised
// to cubic ones exactly, arcs are split into pieces of at most 90 degrees (the usual cubic
// approximation, error below 0.03 % of the radius).  A sub-path that is filled and stroked but
// not closed becomes two sections: a closed one with NoLine (the fill SVG closes implicitly) and
// the open one with NoFill, so Visio draws exactly what SVG does.
//
// Bounded: the reader's nesting (64), 50000 elements, 200000 segments; no recursion, no panic.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::formula::math;

use crate::stencil::SourceError;
use crate::stencil_src::err;
use crate::xml::{Event, Reader, Start};

const MAX_ELEMENTS: usize = 50_000;
const MAX_SEGMENTS: usize = 200_000;

/// a piece of a sub-path (fractions of the box, y up)
#[derive(Copy, Clone, PartialEq, Debug)]
pub(crate) enum Seg {
    Line([f64; 2]),
    Cubic([f64; 2], [f64; 2], [f64; 2]),
}

/// one Geometry section
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Sec {
    pub start: [f64; 2],
    pub segs: Vec<Seg>,
    pub no_fill: bool,
    pub no_line: bool,
}

/// the format of a set of elements
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Fmt {
    /// #RRGGBB, None: no fill
    pub fill: Option<String>,
    /// 0 .. 1 (1: opaque)
    pub fill_alpha: f64,
    /// #RRGGBB, None: no line
    pub stroke: Option<String>,
    pub stroke_alpha: f64,
    /// the line width in viewBox units (0 without line)
    pub width: f64,
    /// Visio's LineCap (0 round, 1 square, 2 flat) when the SVG names one
    pub cap: Option<u8>,
}

/// the elements of one format
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Layer {
    pub fmt: Fmt,
    pub sections: Vec<Sec>,
}

/// an imported drawing
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct Art {
    /// the viewBox: x, y, width, height
    pub vb: [f64; 4],
    pub layers: Vec<Layer>,
}

// an affine map p -> (a x + c y + e, b x + d y + f)
#[derive(Copy, Clone, PartialEq, Debug)]
struct M {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl M {
    const ID: M = M { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    // self · n (n first)
    fn mul(&self, n: &M) -> M {
        M {
            a: self.a * n.a + self.c * n.b,
            b: self.b * n.a + self.d * n.b,
            c: self.a * n.c + self.c * n.d,
            d: self.b * n.c + self.d * n.d,
            e: self.a * n.e + self.c * n.f + self.e,
            f: self.b * n.e + self.d * n.f + self.f,
        }
    }

    fn apply(&self, p: [f64; 2]) -> [f64; 2] {
        [self.a * p[0] + self.c * p[1] + self.e, self.b * p[0] + self.d * p[1] + self.f]
    }

    fn scale(&self) -> f64 {
        math::sqrt((self.a * self.d - self.b * self.c).abs())
    }
}

// a paint: inherit, none, or a colour
#[derive(Clone, PartialEq, Debug)]
enum Paint {
    None,
    Color(String),
}

#[derive(Clone, Debug)]
struct St {
    m: M,
    fill: Paint,
    stroke: Paint,
    width: f64,
    fill_op: f64,
    stroke_op: f64,
    opacity: f64,
    cap: Option<u8>,
}

// raw sub-paths in user units, y down
#[derive(Clone, Debug, Default)]
struct Raw {
    start: [f64; 2],
    segs: Vec<Seg>,
    closed: bool,
}

fn line_of(text: &str, offset: usize) -> u32 {
    let end = offset.min(text.len());
    text.as_bytes()[..end].iter().filter(|&&b| b == b'\n').count() as u32 + 1
}

// ---------------------------------------------------------------- numbers and lists

fn is_sep(c: u8) -> bool {
    c == b' ' || c == b',' || c == b'\t' || c == b'\n' || c == b'\r'
}

struct Nums<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Nums<'a> {
    fn new(s: &'a str) -> Self {
        Nums { b: s.as_bytes(), i: 0 }
    }

    fn skip(&mut self) {
        while self.i < self.b.len() && is_sep(self.b[self.i]) {
            self.i += 1;
        }
    }

    fn at_end(&mut self) -> bool {
        self.skip();
        self.i >= self.b.len()
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip();
        self.b.get(self.i).copied()
    }

    // a number in SVG's grammar ("1.5.5" is 1.5 and .5, "-1-2" two numbers)
    fn num(&mut self) -> Option<f64> {
        self.skip();
        let s = self.i;
        let b = self.b;
        let mut i = s;
        if i < b.len() && (b[i] == b'-' || b[i] == b'+') {
            i += 1;
        }
        let ds = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i < b.len() && b[i] == b'.' {
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
        if i == ds || (i == ds + 1 && b[ds] == b'.') {
            return None;
        }
        if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
            let mut j = i + 1;
            if j < b.len() && (b[j] == b'-' || b[j] == b'+') {
                j += 1;
            }
            let k = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j > k {
                i = j;
            }
        }
        let x: f64 = core::str::from_utf8(&b[s..i]).ok()?.parse().ok()?;
        self.i = i;
        x.is_finite().then_some(x)
    }

    // an arc flag: 0 or 1, also without separator
    fn flag(&mut self) -> Option<bool> {
        self.skip();
        let c = *self.b.get(self.i)?;
        self.i += 1;
        match c {
            b'0' => Some(false),
            b'1' => Some(true),
            _ => None,
        }
    }
}

// a length: a number with px, pt, pc, mm, cm, in or nothing -> user units
fn length(s: &str) -> Option<f64> {
    let t = s.trim();
    let (num, k) = [("px", 1.0), ("pt", 96.0 / 72.0), ("pc", 16.0), ("mm", 96.0 / 25.4), ("cm", 96.0 / 2.54), ("in", 96.0)]
        .iter()
        .find_map(|&(u, k)| t.strip_suffix(u).map(|n| (n, k)))
        .unwrap_or((t, 1.0));
    let x: f64 = num.trim().parse().ok()?;
    let v = x * k;
    v.is_finite().then_some(v)
}

fn hex2(s: &str) -> Option<u8> {
    u8::from_str_radix(s, 16).ok()
}

// a colour -> Some(Paint) (None: not understood)
fn color(s: &str) -> Result<Option<Paint>, String> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("none") || t.eq_ignore_ascii_case("transparent") {
        return Ok(Some(Paint::None));
    }
    if t.eq_ignore_ascii_case("inherit") {
        return Ok(None);
    }
    if t.starts_with("url(") {
        return Err(format!("Verläufe und Muster werden nicht unterstützt ({t})"));
    }
    let rgb = |r: u8, g: u8, b: u8| Ok(Some(Paint::Color(format!("#{r:02X}{g:02X}{b:02X}"))));
    if let Some(h) = t.strip_prefix('#') {
        if h.len() == 6 && h.is_ascii() {
            if let (Some(r), Some(g), Some(b)) = (hex2(&h[0..2]), hex2(&h[2..4]), hex2(&h[4..6])) {
                return rgb(r, g, b);
            }
        }
        if h.len() == 3 && h.is_ascii() {
            if let (Some(r), Some(g), Some(b)) = (hex2(&h[0..1]), hex2(&h[1..2]), hex2(&h[2..3])) {
                return rgb(r * 17, g * 17, b * 17);
            }
        }
        return Err(format!("Farbe {t} nicht lesbar"));
    }
    if let Some(inner) = t.strip_prefix("rgb(").and_then(|r| r.strip_suffix(')')) {
        let mut ch = [0u8; 3];
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        if parts.len() != 3 {
            return Err(format!("Farbe {t} nicht lesbar"));
        }
        for (k, p) in parts.iter().enumerate() {
            let v = match p.strip_suffix('%') {
                Some(pc) => pc.trim().parse::<f64>().ok().map(|x| x * 2.55),
                None => p.parse::<f64>().ok(),
            };
            let Some(v) = v.filter(|v| v.is_finite()) else { return Err(format!("Farbe {t} nicht lesbar")) };
            ch[k] = math::round(v.clamp(0.0, 255.0)) as u8;
        }
        return rgb(ch[0], ch[1], ch[2]);
    }
    const NAMES: &[(&str, &str)] = &[
        ("black", "#000000"), ("white", "#FFFFFF"), ("red", "#FF0000"), ("green", "#008000"), ("blue", "#0000FF"),
        ("yellow", "#FFFF00"), ("gray", "#808080"), ("grey", "#808080"), ("silver", "#C0C0C0"), ("maroon", "#800000"),
        ("navy", "#000080"), ("teal", "#008080"), ("olive", "#808000"), ("purple", "#800080"), ("fuchsia", "#FF00FF"),
        ("magenta", "#FF00FF"), ("aqua", "#00FFFF"), ("cyan", "#00FFFF"), ("lime", "#00FF00"), ("orange", "#FFA500"),
    ];
    match NAMES.iter().find(|(n, _)| n.eq_ignore_ascii_case(t)) {
        Some((_, h)) => Ok(Some(Paint::Color(String::from(*h)))),
        None => Err(format!("Farbe „{t}“ wird nicht unterstützt")),
    }
}

fn opacity(s: &str) -> Option<f64> {
    let t = s.trim();
    let v = match t.strip_suffix('%') {
        Some(p) => p.trim().parse::<f64>().ok()? / 100.0,
        None => t.parse::<f64>().ok()?,
    };
    v.is_finite().then(|| v.clamp(0.0, 1.0))
}

// transform="..." -> the map (applied to the element's own coordinates)
fn transform(s: &str) -> Result<M, String> {
    let mut m = M::ID;
    let mut rest = s.trim();
    while !rest.is_empty() {
        let open = rest.find('(').ok_or_else(|| format!("transform „{s}“ nicht lesbar"))?;
        let close = rest.find(')').ok_or_else(|| format!("transform „{s}“ nicht lesbar"))?;
        if close < open {
            return Err(format!("transform „{s}“ nicht lesbar"));
        }
        let name = rest[..open].trim_matches(|c: char| c.is_whitespace() || c == ',');
        let mut nums = Nums::new(&rest[open + 1..close]);
        let mut args = Vec::new();
        while !nums.at_end() {
            args.push(nums.num().ok_or_else(|| format!("transform „{s}“ nicht lesbar"))?);
            if args.len() > 6 {
                return Err(format!("transform „{s}“ nicht lesbar"));
            }
        }
        let t = match (name, args.as_slice()) {
            ("matrix", &[a, b, c, d, e, f]) => M { a, b, c, d, e, f },
            ("translate", &[x]) => M { e: x, ..M::ID },
            ("translate", &[x, y]) => M { e: x, f: y, ..M::ID },
            ("scale", &[k]) => M { a: k, d: k, ..M::ID },
            ("scale", &[x, y]) => M { a: x, d: y, ..M::ID },
            ("rotate", &[deg]) => rotate(deg),
            ("rotate", &[deg, cx, cy]) => M { e: cx, f: cy, ..M::ID }.mul(&rotate(deg)).mul(&M { e: -cx, f: -cy, ..M::ID }),
            ("skewX", &[deg]) => M { c: math::tan(deg.to_radians()), ..M::ID },
            ("skewY", &[deg]) => M { b: math::tan(deg.to_radians()), ..M::ID },
            _ => return Err(format!("transform {name}({}) wird nicht unterstützt", args.len())),
        };
        m = m.mul(&t);
        rest = rest[close + 1..].trim_start_matches(|c: char| c.is_whitespace() || c == ',');
    }
    Ok(m)
}

fn rotate(deg: f64) -> M {
    let (s, c) = math::sincos(deg.to_radians());
    M { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 }
}

// ---------------------------------------------------------------- path building

struct Pb {
    subs: Vec<Raw>,
    cur: [f64; 2],
    start: [f64; 2],
    open: bool,
    count: usize,
}

impl Pb {
    fn new() -> Self {
        Pb { subs: Vec::new(), cur: [0.0; 2], start: [0.0; 2], open: false, count: 0 }
    }

    fn move_to(&mut self, p: [f64; 2]) {
        self.subs.push(Raw { start: p, segs: Vec::new(), closed: false });
        self.cur = p;
        self.start = p;
        self.open = true;
    }

    fn seg(&mut self, s: Seg) -> Result<(), String> {
        if !self.open {
            // drawing on after Z starts a new sub-path at the start point
            let p = self.start;
            self.move_to(p);
        }
        self.count += 1;
        if self.count > MAX_SEGMENTS {
            return Err(String::from("zu viele Pfadstücke"));
        }
        if let Some(r) = self.subs.last_mut() {
            r.segs.push(s);
        }
        self.cur = match s {
            Seg::Line(p) | Seg::Cubic(_, _, p) => p,
        };
        Ok(())
    }

    fn line(&mut self, p: [f64; 2]) -> Result<(), String> {
        self.seg(Seg::Line(p))
    }

    fn cubic(&mut self, c1: [f64; 2], c2: [f64; 2], p: [f64; 2]) -> Result<(), String> {
        self.seg(Seg::Cubic(c1, c2, p))
    }

    fn close(&mut self) {
        if let Some(r) = self.subs.last_mut() {
            if self.open {
                r.closed = true;
            }
        }
        self.cur = self.start;
        self.open = false;
    }

    // an elliptical arc (SVG's endpoint form, implementation notes F.6.5) as cubic pieces of
    // at most 90 degrees
    fn arc(&mut self, rx: f64, ry: f64, phi_deg: f64, large: bool, sweep: bool, p1: [f64; 2]) -> Result<(), String> {
        let p0 = self.cur;
        if p0 == p1 {
            return Ok(());
        }
        let (mut rx, mut ry) = (rx.abs(), ry.abs());
        if rx < 1e-12 || ry < 1e-12 {
            return self.line(p1);
        }
        let (s, c) = math::sincos(phi_deg.to_radians());
        let dx = (p0[0] - p1[0]) / 2.0;
        let dy = (p0[1] - p1[1]) / 2.0;
        let x1 = c * dx + s * dy;
        let y1 = -s * dx + c * dy;
        let lambda = x1 * x1 / (rx * rx) + y1 * y1 / (ry * ry);
        if lambda > 1.0 {
            let k = math::sqrt(lambda);
            rx *= k;
            ry *= k;
        }
        let num = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
        let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
        let mut coef = if den > 0.0 { math::sqrt((num / den).max(0.0)) } else { 0.0 };
        if large == sweep {
            coef = -coef;
        }
        let cxp = coef * rx * y1 / ry;
        let cyp = -coef * ry * x1 / rx;
        let cx = c * cxp - s * cyp + (p0[0] + p1[0]) / 2.0;
        let cy = s * cxp + c * cyp + (p0[1] + p1[1]) / 2.0;
        let u = [(x1 - cxp) / rx, (y1 - cyp) / ry];
        let v = [(-x1 - cxp) / rx, (-y1 - cyp) / ry];
        let theta1 = math::atan2(u[1], u[0]);
        let mut delta = math::atan2(u[0] * v[1] - u[1] * v[0], u[0] * v[0] + u[1] * v[1]);
        if !sweep && delta > 0.0 {
            delta -= 2.0 * core::f64::consts::PI;
        } else if sweep && delta < 0.0 {
            delta += 2.0 * core::f64::consts::PI;
        }
        if !delta.is_finite() || !cx.is_finite() || !cy.is_finite() {
            return self.line(p1);
        }
        let n = (math::ceil(delta.abs() / core::f64::consts::FRAC_PI_2 - 1e-9) as usize).clamp(1, 4);
        let step = delta / n as f64;
        let k = 4.0 / 3.0 * math::tan(step / 4.0);
        let at = |t: f64| -> ([f64; 2], [f64; 2]) {
            let (st, ct) = math::sincos(t);
            let (ex, ey) = (rx * ct, ry * st);
            let (tx, ty) = (-rx * st, ry * ct);
            ([cx + c * ex - s * ey, cy + s * ex + c * ey], [c * tx - s * ty, s * tx + c * ty])
        };
        for i in 0..n {
            let t1 = theta1 + step * i as f64;
            let t2 = t1 + step;
            let (a, da) = at(t1);
            let (b, db) = at(t2);
            let end = if i + 1 == n { p1 } else { b };
            let start = if i == 0 { p0 } else { a };
            self.cubic([start[0] + k * da[0], start[1] + k * da[1]], [end[0] - k * db[0], end[1] - k * db[1]], end)?;
        }
        Ok(())
    }

    // a whole ellipse, four quarter pieces from (cx + rx, cy)
    fn ellipse(&mut self, cx: f64, cy: f64, rx: f64, ry: f64) -> Result<(), String> {
        const K: f64 = 0.552_284_749_830_793_4; // 4/3 tan(pi/8)
        let p = |x: f64, y: f64| [cx + x, cy + y];
        self.move_to(p(rx, 0.0));
        self.cubic(p(rx, K * ry), p(K * rx, ry), p(0.0, ry))?;
        self.cubic(p(-K * rx, ry), p(-rx, K * ry), p(-rx, 0.0))?;
        self.cubic(p(-rx, -K * ry), p(-K * rx, -ry), p(0.0, -ry))?;
        self.cubic(p(K * rx, -ry), p(rx, -K * ry), p(rx, 0.0))?;
        self.close();
        Ok(())
    }
}

// the d attribute of a path
fn path_data(d: &str, pb: &mut Pb) -> Result<(), String> {
    let mut n = Nums::new(d);
    let mut cmd: u8 = 0;
    // the last control point for S and T (reflected), and which kind of curve set it
    let mut last_c: Option<[f64; 2]> = None;
    let mut last_q: Option<[f64; 2]> = None;
    let bad = || format!("Pfaddaten nicht lesbar: „{}“", d.chars().take(40).collect::<String>());
    loop {
        let Some(c) = n.peek() else { break };
        if c.is_ascii_alphabetic() {
            cmd = c;
            n.i += 1;
            if cmd == b'Z' || cmd == b'z' {
                pb.close();
                last_c = None;
                last_q = None;
                continue;
            }
        } else if cmd == 0 || cmd == b'Z' || cmd == b'z' {
            return Err(bad());
        }
        let rel = cmd.is_ascii_lowercase();
        let base = if rel { pb.cur } else { [0.0, 0.0] };
        let pt = |n: &mut Nums| -> Option<[f64; 2]> {
            let x = n.num()?;
            let y = n.num()?;
            Some([base[0] + x, base[1] + y])
        };
        let (mut nc, mut nq) = (None, None);
        match cmd.to_ascii_uppercase() {
            b'M' => {
                let p = pt(&mut n).ok_or_else(bad)?;
                pb.move_to(p);
                // further pairs are line-tos
                cmd = if rel { b'l' } else { b'L' };
            }
            b'L' => {
                let p = pt(&mut n).ok_or_else(bad)?;
                pb.line(p)?;
            }
            b'H' => {
                let x = n.num().ok_or_else(bad)?;
                let p = [if rel { pb.cur[0] + x } else { x }, pb.cur[1]];
                pb.line(p)?;
            }
            b'V' => {
                let y = n.num().ok_or_else(bad)?;
                let p = [pb.cur[0], if rel { pb.cur[1] + y } else { y }];
                pb.line(p)?;
            }
            b'C' => {
                let c1 = pt(&mut n).ok_or_else(bad)?;
                let c2 = pt(&mut n).ok_or_else(bad)?;
                let p = pt(&mut n).ok_or_else(bad)?;
                pb.cubic(c1, c2, p)?;
                nc = Some(c2);
            }
            b'S' => {
                let c2 = pt(&mut n).ok_or_else(bad)?;
                let p = pt(&mut n).ok_or_else(bad)?;
                let cur = pb.cur;
                let c1 = last_c.map_or(cur, |l| [2.0 * cur[0] - l[0], 2.0 * cur[1] - l[1]]);
                pb.cubic(c1, c2, p)?;
                nc = Some(c2);
            }
            b'Q' => {
                let q = pt(&mut n).ok_or_else(bad)?;
                let p = pt(&mut n).ok_or_else(bad)?;
                quad(pb, q, p)?;
                nq = Some(q);
            }
            b'T' => {
                let p = pt(&mut n).ok_or_else(bad)?;
                let cur = pb.cur;
                let q = last_q.map_or(cur, |l| [2.0 * cur[0] - l[0], 2.0 * cur[1] - l[1]]);
                quad(pb, q, p)?;
                nq = Some(q);
            }
            b'A' => {
                let rx = n.num().ok_or_else(bad)?;
                let ry = n.num().ok_or_else(bad)?;
                let phi = n.num().ok_or_else(bad)?;
                let large = n.flag().ok_or_else(bad)?;
                let sweep = n.flag().ok_or_else(bad)?;
                let p = pt(&mut n).ok_or_else(bad)?;
                pb.arc(rx, ry, phi, large, sweep, p)?;
            }
            _ => return Err(format!("Pfadbefehl {} wird nicht unterstützt", cmd as char)),
        }
        last_c = nc;
        last_q = nq;
    }
    Ok(())
}

// a quadratic curve raised to a cubic one
fn quad(pb: &mut Pb, q: [f64; 2], p: [f64; 2]) -> Result<(), String> {
    let c = pb.cur;
    let c1 = [c[0] + 2.0 / 3.0 * (q[0] - c[0]), c[1] + 2.0 / 3.0 * (q[1] - c[1])];
    let c2 = [p[0] + 2.0 / 3.0 * (q[0] - p[0]), p[1] + 2.0 / 3.0 * (q[1] - p[1])];
    pb.cubic(c1, c2, p)
}

fn attr<'s>(s: &'s Start, name: &str) -> Option<&'s str> {
    s.attrs.iter().find(|a| a.local == name && a.qname == name).map(|a| a.value.as_ref())
}

fn len_attr(s: &Start, name: &str) -> Result<f64, String> {
    match attr(s, name) {
        None => Ok(0.0),
        Some(v) => length(v).ok_or_else(|| format!("{name}=\"{v}\" nicht lesbar")),
    }
}

fn points(s: &str, pb: &mut Pb, close: bool) -> Result<(), String> {
    let mut n = Nums::new(s);
    let mut first = true;
    while !n.at_end() {
        let (Some(x), Some(y)) = (n.num(), n.num()) else { return Err(format!("points=\"{s}\" nicht lesbar")) };
        if first {
            pb.move_to([x, y]);
            first = false;
        } else {
            pb.line([x, y])?;
        }
    }
    if close && !first {
        pb.close();
    }
    Ok(())
}

// the sub-paths of a drawing element in its own coordinates
fn element(s: &Start) -> Result<Vec<Raw>, String> {
    let mut pb = Pb::new();
    match s.local {
        "path" => path_data(attr(s, "d").unwrap_or(""), &mut pb)?,
        "rect" => {
            let (x, y, w, h) = (len_attr(s, "x")?, len_attr(s, "y")?, len_attr(s, "width")?, len_attr(s, "height")?);
            if w > 0.0 && h > 0.0 {
                // a radius given alone stands for both
                let rxa = attr(s, "rx").map(|_| len_attr(s, "rx")).transpose()?;
                let rya = attr(s, "ry").map(|_| len_attr(s, "ry")).transpose()?;
                let rx = rxa.or(rya).unwrap_or(0.0);
                let ry = rya.or(rxa).unwrap_or(0.0);
                let (rx, ry) = (rx.clamp(0.0, w / 2.0), ry.clamp(0.0, h / 2.0));
                if rx > 0.0 && ry > 0.0 {
                    pb.move_to([x + rx, y]);
                    pb.line([x + w - rx, y])?;
                    pb.arc(rx, ry, 0.0, false, true, [x + w, y + ry])?;
                    pb.line([x + w, y + h - ry])?;
                    pb.arc(rx, ry, 0.0, false, true, [x + w - rx, y + h])?;
                    pb.line([x + rx, y + h])?;
                    pb.arc(rx, ry, 0.0, false, true, [x, y + h - ry])?;
                    pb.line([x, y + ry])?;
                    pb.arc(rx, ry, 0.0, false, true, [x + rx, y])?;
                } else {
                    pb.move_to([x, y]);
                    pb.line([x + w, y])?;
                    pb.line([x + w, y + h])?;
                    pb.line([x, y + h])?;
                    pb.line([x, y])?;
                }
                pb.close();
            }
        }
        "circle" => {
            let r = len_attr(s, "r")?;
            if r > 0.0 {
                pb.ellipse(len_attr(s, "cx")?, len_attr(s, "cy")?, r, r)?;
            }
        }
        "ellipse" => {
            let (rx, ry) = (len_attr(s, "rx")?, len_attr(s, "ry")?);
            if rx > 0.0 && ry > 0.0 {
                pb.ellipse(len_attr(s, "cx")?, len_attr(s, "cy")?, rx, ry)?;
            }
        }
        "line" => {
            pb.move_to([len_attr(s, "x1")?, len_attr(s, "y1")?]);
            pb.line([len_attr(s, "x2")?, len_attr(s, "y2")?])?;
        }
        "polyline" => points(attr(s, "points").unwrap_or(""), &mut pb, false)?,
        "polygon" => points(attr(s, "points").unwrap_or(""), &mut pb, true)?,
        _ => {}
    }
    Ok(pb.subs)
}

// presentation attributes and style="..." onto the state
fn style(s: &Start, st: &mut St) -> Result<bool, String> {
    let mut props: Vec<(String, String)> = Vec::new();
    for a in &s.attrs {
        if a.qname == a.local {
            props.push((String::from(a.local), String::from(a.value.as_ref())));
        }
    }
    if let Some(css) = attr(s, "style") {
        for decl in css.split(';') {
            if let Some((k, v)) = decl.split_once(':') {
                props.push((String::from(k.trim()), String::from(v.trim())));
            }
        }
    }
    let mut shown = true;
    for (k, v) in &props {
        match k.as_str() {
            "fill" => {
                if let Some(p) = color(v)? {
                    st.fill = p;
                }
            }
            "stroke" => {
                if let Some(p) = color(v)? {
                    st.stroke = p;
                }
            }
            "stroke-width" => st.width = length(v).filter(|w| *w >= 0.0).ok_or_else(|| format!("stroke-width=\"{v}\" nicht lesbar"))?,
            "fill-opacity" => st.fill_op = opacity(v).ok_or_else(|| format!("fill-opacity=\"{v}\" nicht lesbar"))?,
            "stroke-opacity" => st.stroke_op = opacity(v).ok_or_else(|| format!("stroke-opacity=\"{v}\" nicht lesbar"))?,
            "opacity" => st.opacity *= opacity(v).ok_or_else(|| format!("opacity=\"{v}\" nicht lesbar"))?,
            "stroke-linecap" => {
                st.cap = match v.trim() {
                    "butt" => Some(2),
                    "round" => Some(0),
                    "square" => Some(1),
                    _ => st.cap,
                }
            }
            "display" if v.trim() == "none" => shown = false,
            "visibility" if v.trim() == "hidden" => shown = false,
            _ => {}
        }
    }
    if let Some(t) = attr(s, "transform") {
        st.m = st.m.mul(&transform(t)?);
    }
    Ok(shown)
}

/// an SVG file as layers of sections (see the module comment)
pub(crate) fn import(file: &str, text: &str) -> Result<Art, SourceError> {
    let mut r = Reader::new(text, 64, 256);
    let mut stack: Vec<St> = Vec::new();
    let mut vb: Option<[f64; 4]> = None;
    let mut layers: Vec<Layer> = Vec::new();
    let mut elements = 0usize;
    let mut segments = 0usize;
    let fail = |at: usize, m: String| err(file, line_of(text, at), m);
    loop {
        let ev = r.next().map_err(|(e, at)| fail(at, format!("SVG nicht wohlgeformt ({e:?})")))?;
        match ev {
            Event::Eof => break,
            Event::Text(_) => {}
            Event::End { .. } => {
                stack.pop();
            }
            Event::Start(s) => {
                elements += 1;
                if elements > MAX_ELEMENTS {
                    return Err(fail(s.at, String::from("zu viele SVG-Elemente")));
                }
                let foreign = s.qname.contains(':') && !s.qname.starts_with("svg:");
                let Some(parent) = stack.last().cloned() else {
                    // the root
                    if s.local != "svg" {
                        return Err(fail(s.at, format!("kein SVG: das Wurzelelement ist <{}>", s.qname)));
                    }
                    let v = attr(&s, "viewBox").ok_or_else(|| fail(s.at, String::from("das SVG braucht eine viewBox")))?;
                    let mut n = Nums::new(v);
                    let (Some(x), Some(y), Some(w), Some(h)) = (n.num(), n.num(), n.num(), n.num()) else {
                        return Err(fail(s.at, format!("viewBox=\"{v}\" nicht lesbar")));
                    };
                    if !(w > 0.0 && h > 0.0) || !n.at_end() {
                        return Err(fail(s.at, format!("viewBox=\"{v}\": Breite und Höhe müssen positiv sein")));
                    }
                    vb = Some([x, y, w, h]);
                    let mut st = St { m: M::ID, fill: Paint::Color(String::from("#000000")), stroke: Paint::None, width: 1.0, fill_op: 1.0, stroke_op: 1.0, opacity: 1.0, cap: None };
                    // the root's own transform is not SVG; its presentation attributes are
                    style(&s, &mut st).map_err(|m| fail(s.at, m))?;
                    st.m = M::ID;
                    stack.push(st);
                    continue;
                };
                if foreign || matches!(s.local, "title" | "desc" | "metadata") {
                    let _ = r.skip(&s).map_err(|(e, at)| fail(at, format!("SVG nicht wohlgeformt ({e:?})")))?;
                    continue;
                }
                let mut st = parent;
                let shown = style(&s, &mut st).map_err(|m| fail(s.at, m))?;
                match s.local {
                    "g" => {
                        if !shown {
                            let _ = r.skip(&s).map_err(|(e, at)| fail(at, format!("SVG nicht wohlgeformt ({e:?})")))?;
                            continue;
                        }
                        stack.push(st);
                    }
                    "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon" => {
                        let raws = element(&s).map_err(|m| fail(s.at, m))?;
                        // its content (title, desc) and its end tag
                        let _ = r.skip(&s).map_err(|(e, at)| fail(at, format!("SVG nicht wohlgeformt ({e:?})")))?;
                        if !shown {
                            continue;
                        }
                        let Some(vb) = vb else { continue };
                        segments += raws.iter().map(|r| r.segs.len()).sum::<usize>();
                        if segments > MAX_SEGMENTS {
                            return Err(fail(s.at, String::from("zu viele Pfadstücke")));
                        }
                        add(&mut layers, &st, &raws, vb);
                    }
                    other => return Err(fail(s.at, format!("SVG-Element <{other}> wird nicht unterstützt"))),
                }
            }
        }
    }
    let vb = vb.ok_or_else(|| err(file, 1, "kein SVG-Element"))?;
    Ok(Art { vb, layers })
}

// the format of an element's state
fn fmt_of(st: &St) -> Fmt {
    let fill = match &st.fill {
        Paint::Color(c) => Some(c.clone()),
        Paint::None => None,
    };
    let stroke = match &st.stroke {
        Paint::Color(c) if st.width > 0.0 => Some(c.clone()),
        _ => None,
    };
    let q = |x: f64| math::round(x * 1e6) / 1e6;
    Fmt {
        fill_alpha: if fill.is_some() { q(st.fill_op * st.opacity) } else { 1.0 },
        stroke_alpha: if stroke.is_some() { q(st.stroke_op * st.opacity) } else { 1.0 },
        width: if stroke.is_some() { q(st.width * st.m.scale()) } else { 0.0 },
        cap: if stroke.is_some() { st.cap } else { None },
        fill,
        stroke,
    }
}

// an element's sub-paths into the last layer when it has its format, else into a new one
fn add(layers: &mut Vec<Layer>, st: &St, raws: &[Raw], vb: [f64; 4]) {
    let fmt = fmt_of(st);
    if fmt.fill.is_none() && fmt.stroke.is_none() {
        return;
    }
    let map = |p: [f64; 2]| -> [f64; 2] {
        let q = st.m.apply(p);
        [(q[0] - vb[0]) / vb[2], 1.0 - (q[1] - vb[1]) / vb[3]]
    };
    let mut secs = Vec::new();
    for raw in raws {
        if raw.segs.is_empty() {
            continue;
        }
        let start = map(raw.start);
        let segs: Vec<Seg> = raw
            .segs
            .iter()
            .map(|s| match *s {
                Seg::Line(p) => Seg::Line(map(p)),
                Seg::Cubic(a, b, p) => Seg::Cubic(map(a), map(b), map(p)),
            })
            .collect();
        let end = segs.last().map_or(start, |s| match *s {
            Seg::Line(p) | Seg::Cubic(_, _, p) => p,
        });
        let mut closed_segs = segs.clone();
        if end != start {
            closed_segs.push(Seg::Line(start));
        }
        match (fmt.fill.is_some(), fmt.stroke.is_some(), raw.closed) {
            (true, true, true) => secs.push(Sec { start, segs: closed_segs, no_fill: false, no_line: false }),
            (true, true, false) => {
                secs.push(Sec { start, segs: closed_segs, no_fill: false, no_line: true });
                secs.push(Sec { start, segs, no_fill: true, no_line: false });
            }
            (true, false, _) => secs.push(Sec { start, segs: closed_segs, no_fill: false, no_line: true }),
            (false, true, true) => secs.push(Sec { start, segs: closed_segs, no_fill: true, no_line: false }),
            (false, true, false) => secs.push(Sec { start, segs, no_fill: true, no_line: false }),
            (false, false, _) => {}
        }
    }
    if secs.is_empty() {
        return;
    }
    // (JD13) in document order like JFNetworker's symbols (tools/vsym.py): an element joins the
    // last layer only when it has the same format; a format that comes back later is a layer of
    // its own above what lies between
    // A filled element that overlaps a filled section of that layer gets a layer of its own too:
    // Visio fills a shape's sections together by the even-odd rule, the overlap would be a hole
    // (JFNetworker fills every element by itself)
    let filled = |x: &Sec| !x.no_fill;
    let overlaps = |l: &Layer| secs.iter().filter(|x| filled(x)).any(|n| l.sections.iter().filter(|x| filled(x)).any(|o| boxes_meet(bbox(n), bbox(o))));
    match layers.last_mut().filter(|l| l.fmt == fmt && !overlaps(l)) {
        Some(l) => l.sections.extend(secs),
        None => layers.push(Layer { fmt, sections: secs }),
    }
}

// a section's box from its points and control points (x0, y0, x1, y1), a little generous
fn bbox(s: &Sec) -> [f64; 4] {
    let mut b = [s.start[0], s.start[1], s.start[0], s.start[1]];
    let mut add = |p: [f64; 2]| b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
    for g in &s.segs {
        match *g {
            Seg::Line(p) => add(p),
            Seg::Cubic(c1, c2, p) => {
                add(c1);
                add(c2);
                add(p);
            }
        }
    }
    b
}

// whether two boxes share more than an edge
fn boxes_meet(a: [f64; 4], b: [f64; 4]) -> bool {
    const E: f64 = 1e-9;
    a[0] < b[2] - E && b[0] < a[2] - E && a[1] < b[3] - E && b[1] < a[3] - E
}
