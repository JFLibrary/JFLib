// shading -- gradients (axial and radial shadings, ISO 32000-1 8.7.4.5) and hatches for
// Canvas::shade, fill_shading and fill_hatch
//
// A gradient becomes a shading dictionary of type 2 (axial) or 3 (radial) in DeviceRGB.  Its
// colour function: one Type 2 function (exponential, N 1) from colour to colour, or for more stops
// a Type 3 stitching function of such pieces, one per stretch between two stops; stops at the same
// t (a hard edge) leave out the empty stretch between them, so Bounds always rise.  Before the
// first stop and after the last the colour stays the first / last stop's.  The stops' alpha:
// all the same -> the constant fill alpha (ExtGState ca); different -> a luminosity soft mask whose
// group paints the same geometry as a DeviceGray shading of the alphas (Canvas builds it).
//
// Hatches are stroked lines, not tiling patterns: pdfium draws the cells of a rotated tiling
// pattern with hairline gaps and one-pixel steps where they meet, and both pdfium and Poppler round
// the cell step to whole device pixels, so the lines drift from where they belong.  A hatch clips
// to the path and strokes parallel lines over its bounding box in one path (one `S`, so a hatch's
// alpha applies once).

use alloc::vec::Vec;

use crate::canvas::alpha_key;
use crate::geom::Rgb;
use crate::obj::{self, num};

/// the geometry of a gradient, in the user space of the canvas at the time it is painted
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShadingKind {
    /// colour runs from `from` (t = 0) to `to` (t = 1), constant across (PDF type 2)
    Axial { from: [f64; 2], to: [f64; 2] },
    /// circles from (from, r0) at t = 0 to (to, r1) at t = 1 (PDF type 3); radii >= 0
    Radial { from: [f64; 2], r0: f64, to: [f64; 2], r1: f64 },
}

/// a gradient for Canvas::shade and Canvas::fill_shading
#[derive(Clone, Debug, PartialEq)]
pub struct Shading {
    pub kind: ShadingKind,
    /// (t in 0..=1 ascending, colour, alpha 0..=1); at least one stop.  Stops with a NaN t are
    /// dropped, t is clamped to 0..=1 and the stops sorted by it (stable: two stops at the same t
    /// make a hard edge); a NaN alpha is opaque
    pub stops: Vec<(f64, Rgb, f64)>,
    /// paint beyond t < 0 / t > 1 with the end colours (PDF /Extend), default (true, true)
    pub extend: (bool, bool),
}

impl Shading {
    /// an axial gradient from `from` to `to`, extended at both ends
    pub fn axial(from: [f64; 2], to: [f64; 2], stops: &[(f64, Rgb, f64)]) -> Shading {
        Shading { kind: ShadingKind::Axial { from, to }, stops: stops.to_vec(), extend: (true, true) }
    }

    /// a radial gradient from the circle (from, r0) to (to, r1), extended at both ends
    pub fn radial(from: [f64; 2], r0: f64, to: [f64; 2], r1: f64, stops: &[(f64, Rgb, f64)]) -> Shading {
        Shading { kind: ShadingKind::Radial { from, r0, to, r1 }, stops: stops.to_vec(), extend: (true, true) }
    }
}

/// parallel lines for Canvas::fill_hatch; two hatches over the same path cross
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hatch {
    /// direction of the lines in the canvas's user space, radians from +x towards +y: counter-
    /// clockwise on the page where y points up (Origin::BottomLeft), clockwise where it points down
    /// (Origin::TopLeft, as Canvas::rotate)
    pub angle: f64,
    /// distance between neighbouring lines (user space units), > 0
    pub spacing: f64,
    /// line width (user space units), > 0; at least `spacing` fills the path solid
    pub width: f64,
    pub color: Rgb,
    /// 0 to 1, times the canvas's fill alpha
    pub alpha: f64,
    /// a point one of the lines runs through (user space)
    pub origin: [f64; 2],
}

impl Hatch {
    /// opaque lines through (0, 0)
    pub fn new(angle: f64, spacing: f64, width: f64, color: Rgb) -> Hatch {
        Hatch { angle, spacing, width, color, alpha: 1.0, origin: [0.0, 0.0] }
    }
}

/// t in millionths (exact as a PDF number), colour, alpha in thousandths
#[derive(Clone, Copy)]
struct Stop {
    t: u32,
    c: Rgb,
    a: u16,
}

const T_ONE: u32 = 1_000_000;

/// what a gradient comes to
pub(crate) enum Prepared {
    /// nothing to paint (no stops, coordinates not finite, a negative radius)
    Invalid,
    /// degenerate geometry (axial from = to, radial with both radii 0): the whole area in the last
    /// stop's colour and alpha, as SVG does
    Solid(Rgb, u16),
    /// a radial gradient between two equal circles: only the disk, in the last stop's colour
    Disk([f64; 2], f64, Rgb, u16),
    /// a shading dictionary (without << >>) and its alpha
    Shade(Vec<u8>, Alpha),
}

pub(crate) enum Alpha {
    /// one alpha for all stops, thousandths (1000: opaque)
    Const(u16),
    /// the alphas differ: the DeviceGray shading of the soft mask's group
    Mask(Vec<u8>),
}

/// the same bytes when written: what a reader sees as equal
fn same(a: f64, b: f64) -> bool {
    let (mut x, mut y) = (Vec::new(), Vec::new());
    num(&mut x, a);
    num(&mut y, b);
    x == y
}

fn sanitize(s: &Shading) -> Vec<Stop> {
    let mut v: Vec<Stop> = s
        .stops
        .iter()
        .filter(|(t, ..)| !t.is_nan())
        .map(|&(t, c, a)| Stop { t: (t.clamp(0.0, 1.0) * T_ONE as f64 + 0.5) as u32, c: c.clamped(), a: alpha_key(a) })
        .collect();
    v.sort_by_key(|s| s.t);
    v
}

pub(crate) fn prepare(s: &Shading) -> Prepared {
    let stops = sanitize(s);
    let Some(&last) = stops.last() else { return Prepared::Invalid };
    match s.kind {
        ShadingKind::Axial { from, to } => {
            if !from.iter().chain(to.iter()).all(|v| v.is_finite()) {
                return Prepared::Invalid;
            }
            if same(from[0], to[0]) && same(from[1], to[1]) {
                return Prepared::Solid(last.c, last.a);
            }
        }
        ShadingKind::Radial { from, r0, to, r1 } => {
            if !from.iter().chain(to.iter()).chain([r0, r1].iter()).all(|v| v.is_finite()) || r0 < 0.0 || r1 < 0.0 {
                return Prepared::Invalid;
            }
            if same(r0, 0.0) && same(r1, 0.0) {
                return Prepared::Solid(last.c, last.a);
            }
            if same(from[0], to[0]) && same(from[1], to[1]) && same(r0, r1) {
                return Prepared::Disk(from, r0.max(r1), last.c, last.a);
            }
        }
    }
    // the stops padded to t = 0 and t = 1 with the end colours
    let mut pts: Vec<Stop> = Vec::with_capacity(stops.len() + 2);
    if stops[0].t > 0 {
        pts.push(Stop { t: 0, ..stops[0] });
    }
    pts.extend_from_slice(&stops);
    if last.t < T_ONE {
        pts.push(Stop { t: T_ONE, ..last });
    }
    let rgb = |p: &Stop| [p.c.r, p.c.g, p.c.b];
    let mut func = Vec::new();
    function(&mut func, &pts, &rgb);
    let dict = shading_dict(&s.kind, b"DeviceRGB", &func, s.extend);
    let alpha = if stops.iter().all(|p| p.a == stops[0].a) {
        Alpha::Const(stops[0].a)
    } else {
        let mut func = Vec::new();
        function(&mut func, &pts, &|p: &Stop| [p.a as f64 / 1000.0]);
        Alpha::Mask(shading_dict(&s.kind, b"DeviceGray", &func, s.extend))
    };
    Prepared::Shade(dict, alpha)
}

/// `<< /ShadingType .. >>` without the brackets
fn shading_dict(kind: &ShadingKind, cs: &[u8], func: &[u8], extend: (bool, bool)) -> Vec<u8> {
    let mut o = Vec::new();
    match *kind {
        ShadingKind::Axial { from, to } => {
            o.extend_from_slice(b"/ShadingType 2 /ColorSpace ");
            obj::name(&mut o, cs);
            o.extend_from_slice(b" /Coords ");
            obj::nums(&mut o, &[from[0], from[1], to[0], to[1]]);
        }
        ShadingKind::Radial { from, r0, to, r1 } => {
            o.extend_from_slice(b"/ShadingType 3 /ColorSpace ");
            obj::name(&mut o, cs);
            o.extend_from_slice(b" /Coords ");
            obj::nums(&mut o, &[from[0], from[1], r0, to[0], to[1], r1]);
        }
    }
    o.extend_from_slice(b" /Extend [");
    o.extend_from_slice(if extend.0 { b"true " } else { b"false " });
    o.extend_from_slice(if extend.1 { b"true]" } else { b"false]" });
    o.extend_from_slice(b" /Function ");
    o.extend_from_slice(func);
    o
}

/// a Type 2 function (N 1) from c0 to c1 over 0..1
fn type2(o: &mut Vec<u8>, c0: &[f64], c1: &[f64]) {
    o.extend_from_slice(b"<< /FunctionType 2 /Domain [0 1] /C0 ");
    obj::nums(o, c0);
    o.extend_from_slice(b" /C1 ");
    obj::nums(o, c1);
    o.extend_from_slice(b" /N 1 >>");
}

/// the function through the points (t from 0 to 1, rising, equal t allowed): one Type 2 function
/// for one stretch or one value, else a Type 3 stitching function of one Type 2 per stretch of
/// positive length
fn function<const N: usize>(o: &mut Vec<u8>, pts: &[Stop], val: &dyn Fn(&Stop) -> [f64; N]) {
    let first = val(&pts[0]);
    if pts.iter().all(|p| val(p) == first) {
        type2(o, &first, &first);
        return;
    }
    let segs: Vec<usize> = (0..pts.len() - 1).filter(|&i| pts[i].t < pts[i + 1].t).collect();
    if let [i] = segs[..] {
        type2(o, &val(&pts[i]), &val(&pts[i + 1]));
        return;
    }
    o.extend_from_slice(b"<< /FunctionType 3 /Domain [0 1] /Functions [");
    for (k, &i) in segs.iter().enumerate() {
        if k > 0 {
            o.push(b' ');
        }
        type2(o, &val(&pts[i]), &val(&pts[i + 1]));
    }
    o.extend_from_slice(b"] /Bounds [");
    for (k, &i) in segs[1..].iter().enumerate() {
        if k > 0 {
            o.push(b' ');
        }
        num(o, pts[i].t as f64 / T_ONE as f64);
    }
    o.extend_from_slice(b"] /Encode [");
    for k in 0..segs.len() {
        if k > 0 {
            o.push(b' ');
        }
        o.extend_from_slice(b"0 1");
    }
    o.extend_from_slice(b"] >>");
}

/// the most lines one hatch draws (beyond that it is ignored)
pub(crate) const MAX_HATCH_LINES: f64 = 100_000.0;

fn floor(x: f64) -> f64 {
    let i = x as i64 as f64;
    if i > x { i - 1.0 } else { i }
}

fn ceil(x: f64) -> f64 {
    let i = x as i64 as f64;
    if i < x { i + 1.0 } else { i }
}

/// the hatch's lines over the box [x0, y0, x1, y1] (user space) as (start, end) pairs; None when
/// there would be more than MAX_HATCH_LINES (or the numbers run away)
pub(crate) fn hatch_lines(h: &Hatch, bb: [f64; 4]) -> Option<Vec<[f64; 4]>> {
    let (s, c) = gfx::xform::sincos(h.angle);
    let (d, n) = ([c, s], [-s, c]);
    let corners = [[bb[0], bb[1]], [bb[2], bb[1]], [bb[2], bb[3]], [bb[0], bb[3]]];
    let along = |p: &[f64; 2], v: [f64; 2]| (p[0] - h.origin[0]) * v[0] + (p[1] - h.origin[1]) * v[1];
    let (mut n0, mut n1, mut t0, mut t1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for p in &corners {
        n0 = n0.min(along(p, n));
        n1 = n1.max(along(p, n));
        t0 = t0.min(along(p, d));
        t1 = t1.max(along(p, d));
    }
    // lines up to half a width outside still reach into the box; the ends a width beyond it
    let half = h.width / 2.0;
    let (k0, k1) = ((n0 - half) / h.spacing, (n1 + half) / h.spacing);
    if !(k0.is_finite() && k1.is_finite() && k0.abs() < 1e15 && k1.abs() < 1e15) || k1 - k0 > MAX_HATCH_LINES {
        return None;
    }
    let (k0, k1) = (ceil(k0), floor(k1));
    let (t0, t1) = (t0 - h.width, t1 + h.width);
    let mut out = Vec::new();
    let mut k = k0;
    while k <= k1 {
        let o = [h.origin[0] + n[0] * k * h.spacing, h.origin[1] + n[1] * k * h.spacing];
        out.push([o[0] + d[0] * t0, o[1] + d[1] * t0, o[0] + d[0] * t1, o[1] + d[1] * t1]);
        k += 1.0;
    }
    Some(out)
}
