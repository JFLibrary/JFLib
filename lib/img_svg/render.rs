// render -- draw a parsed document through gfx: styles with inheritance, the element walk
// (svg, g, a, switch, use + symbol, shapes, text, image), paints (colours, gradients),
// group opacity in layers, clip paths as 8-bit masks
//
// Coordinates: every element is drawn with the matrix user space -> device pixels; paths are
// flattened in device space (gfx::pathdata, 0.05 px), strokes are outlined there by
// gfx::dash (width = stroke-width x sqrt(|det|) of the matrix: non-uniform scaling draws an
// even width), filled by gfx::raster with the fill rule.  Opacity of an element that paints
// one thing (only fill or only stroke) multiplies that paint; groups and elements that paint
// both are drawn into a layer (the bounds of the content, inside the clip) and composited.
// Text needs a TextHook (the app's font engine): one line per text chunk, tspans with their
// own style and x / y / dx / dy (first values), text-anchor; without a hook text is skipped.
// Limits: MAX_DRAWS element visits per render, MAX_NEST nesting (groups, use), MAX_LAYERS
// layers alive at once (deeper opacity is applied to the paints), use cycles are cut.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::dash::{dash_stroke, DStyle, DASH_MAXPAT};
use gfx::path::Path;
use gfx::pathdata::PData;
use gfx::raster::Raster;
use gfx::xform::Matrix;
use gfx::{Surface, FMT_PARGB, JOIN_BEVEL, JOIN_MITER, JOIN_ROUND, RULE_EVENODD, RULE_NONZERO};
use img_core::xform::{affine, invert, mul, XF_TRANSPARENT};
use rt::fx::sqrtd;

use crate::doc::*;
use crate::paint::{self, Grad, Mask, Shader, Tgt};
use crate::values::{self, Color, Len, Paint, Unit};
use crate::xml::{Item, NONE};

const MAX_DRAWS: u32 = 1_000_000;
const MAX_NEST: u32 = 64;
const MAX_LAYERS: u32 = 8;
// element visits of all bounds queries per render (a use DAG makes one query exponential)
const MAX_BOUNDS: u32 = 4_000_000;
const MAX_LAYER_PX: u64 = 1 << 26;
const MAX_STOPS: usize = 1024;
const KAPPA: f64 = 0.5522847498307936;
// the bounds of content whose extent is not followed (everything)
const ALL: [f64; 4] = [-4.0e6, -4.0e6, 4.0e6, 4.0e6];

/// the font engine for <text> (an app's; ui::SvgText wraps the UI's): glyph outlines and
/// advances of one line
pub trait TextHook {
    /// append the outline of `s` to `path` (flattened closed contours in path pixels): the
    /// baseline starts at (x, y), `px` pixels per em; returns the advance in pixels
    fn outline(&mut self, path: &mut Path, s: &str, font: &FontSpec, px: f32, x: f32, y: f32) -> f32;
    /// the advance of `s` in pixels at `px` pixels per em
    fn advance(&mut self, s: &str, font: &FontSpec, px: f32) -> f32;
}

/// what a text run asks of the font engine
#[derive(Clone, Copy, Debug)]
pub struct FontSpec<'a> {
    /// the font-family list as written ("Arial, sans-serif")
    pub family: &'a str,
    /// 100 .. 900 (400 normal, 700 bold)
    pub weight: u16,
    pub italic: bool,
}

/// render options
pub struct Opts<'a> {
    /// the colour of currentColor at the root (0xAARRGGBB)
    pub color: u32,
    /// the font engine for text (None: text is not drawn)
    pub text: Option<&'a mut dyn TextHook>,
}

impl Default for Opts<'_> {
    fn default() -> Self {
        Opts { color: 0xFF00_0000, text: None }
    }
}

// ------------------------------------------------------------------ styles

/// the inherited properties of an element
#[derive(Clone)]
pub(crate) struct Style {
    fill: Paint,
    fill_op: f64,
    fill_rule: u32,
    stroke: Paint,
    stroke_op: f64,
    sw: Len,
    cap: u32,
    join: u32,
    miter: f64,
    dash: Option<Rc<[Len]>>,
    dash_off: Len,
    color: u32,
    visible: bool,
    clip_rule: u32,
    fs: f64,
    family: Rc<str>,
    weight: u16,
    italic: bool,
    anchor: u8,
}

impl Style {
    fn root(color: u32) -> Style {
        Style {
            fill: Paint::Color(Color::Argb(0xFF00_0000)),
            fill_op: 1.0,
            fill_rule: RULE_NONZERO,
            stroke: Paint::None,
            stroke_op: 1.0,
            sw: Len::px(1.0),
            cap: 0,
            join: JOIN_MITER,
            miter: 4.0,
            dash: None,
            dash_off: Len::ZERO,
            color,
            visible: true,
            clip_rule: RULE_NONZERO,
            fs: 16.0,
            family: Rc::from("sans-serif"),
            weight: 400,
            italic: false,
            anchor: 0,
        }
    }

    /// the style of an element whose parent has style self
    fn child(&self, n: &Node) -> Style {
        let mut s = self.clone();
        let rule = |v: &str| match v {
            "evenodd" => Some(RULE_EVENODD),
            "nonzero" => Some(RULE_NONZERO),
            _ => None,
        };
        // font-size first: em lengths of the other properties use it
        if let Some(v) = n.prop(P_FONT_SIZE) {
            let v = v.trim();
            let named = match v {
                "xx-small" => Some(9.0),
                "x-small" => Some(10.0),
                "small" => Some(13.0),
                "medium" => Some(16.0),
                "large" => Some(18.0),
                "x-large" => Some(24.0),
                "xx-large" => Some(32.0),
                "larger" => Some(self.fs * 1.2),
                "smaller" => Some(self.fs / 1.2),
                _ => None,
            };
            if let Some(f) = named.or_else(|| values::length(v).map(|l| l.get(self.fs, self.fs))) {
                if f >= 0.0 {
                    s.fs = f.min(1.0e6);
                }
            }
        }
        for (p, v) in &n.props {
            let v = v.trim();
            if v == "inherit" {
                continue;
            }
            match *p {
                P_FILL => {
                    if let Some(x) = values::paint(v) {
                        s.fill = x;
                    }
                }
                P_FILL_OPACITY => {
                    if let Some(o) = values::opacity(v) {
                        s.fill_op = o;
                    }
                }
                P_FILL_RULE => {
                    if let Some(r) = rule(v) {
                        s.fill_rule = r;
                    }
                }
                P_STROKE => {
                    if let Some(x) = values::paint(v) {
                        s.stroke = x;
                    }
                }
                P_STROKE_OPACITY => {
                    if let Some(o) = values::opacity(v) {
                        s.stroke_op = o;
                    }
                }
                P_STROKE_WIDTH => {
                    if let Some(l) = values::length(v).filter(|l| l.v >= 0.0) {
                        s.sw = l;
                    }
                }
                P_LINECAP => match v {
                    "butt" => s.cap = 0,
                    "round" => s.cap = 1,
                    "square" => s.cap = 2,
                    _ => {}
                },
                P_LINEJOIN => match v {
                    "miter" | "miter-clip" | "arcs" => s.join = JOIN_MITER,
                    "round" => s.join = JOIN_ROUND,
                    "bevel" => s.join = JOIN_BEVEL,
                    _ => {}
                },
                P_MITER => {
                    if let Some(m) = values::num_only(v).filter(|&m| m >= 1.0) {
                        s.miter = m;
                    }
                }
                P_DASHARRAY => {
                    if v == "none" {
                        s.dash = None;
                    } else if let Some(d) = values::lengths(v) {
                        // negative values: an error (solid); all zero: solid
                        s.dash = if d.iter().any(|l| l.v < 0.0) || d.iter().all(|l| l.v == 0.0) || d.is_empty() { None } else { Some(Rc::from(d)) };
                    }
                }
                P_DASHOFFSET => {
                    if let Some(l) = values::length(v) {
                        s.dash_off = l;
                    }
                }
                P_COLOR => match values::color(v) {
                    Some(Color::Argb(c)) => s.color = c,
                    _ => {}
                },
                P_VISIBILITY => match v {
                    "visible" => s.visible = true,
                    "hidden" | "collapse" => s.visible = false,
                    _ => {}
                },
                P_CLIP_RULE => {
                    if let Some(r) = rule(v) {
                        s.clip_rule = r;
                    }
                }
                P_FONT_FAMILY => s.family = Rc::from(v),
                P_FONT_WEIGHT => {
                    s.weight = match v {
                        "normal" => 400,
                        "bold" => 700,
                        "bolder" => (self.weight + 300).min(900),
                        "lighter" => self.weight.saturating_sub(300).max(100),
                        _ => values::num_only(v).map_or(s.weight, |w| w.clamp(1.0, 1000.0) as u16),
                    }
                }
                P_FONT_STYLE => match v {
                    "italic" | "oblique" => s.italic = true,
                    "normal" => s.italic = false,
                    _ => {}
                },
                P_TEXT_ANCHOR => match v {
                    "start" => s.anchor = 0,
                    "middle" => s.anchor = 1,
                    "end" => s.anchor = 2,
                    _ => {}
                },
                _ => {}
            }
        }
        s
    }
}

// non-inherited properties
fn opacity_of(n: &Node) -> f64 {
    n.prop(P_OPACITY).and_then(values::opacity).unwrap_or(1.0)
}

fn display_none(n: &Node) -> bool {
    n.prop(P_DISPLAY).is_some_and(|v| v.trim() == "none")
}

fn overflow_visible(n: &Node) -> bool {
    n.prop(P_OVERFLOW).is_some_and(|v| matches!(v.trim(), "visible" | "auto"))
}

/// the viewport of percentages: width, height (user units)
#[derive(Clone, Copy)]
struct Vp {
    w: f64,
    h: f64,
}

impl Vp {
    fn diag(&self) -> f64 {
        sqrtd((self.w * self.w + self.h * self.h) * 0.5)
    }
}

fn translate(x: f64, y: f64) -> Matrix {
    Matrix { c: x, f: y, ..Matrix::IDENTITY }
}

/// device pixels per user unit of m (the square root of |det|)
fn scale_of(m: &Matrix) -> f64 {
    sqrtd((m.a * m.e - m.b * m.d).abs())
}

fn inter(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])]
}

fn empty(r: [i32; 4]) -> bool {
    r[2] <= r[0] || r[3] <= r[1]
}

// a float box (x0, y0, x1, y1) rounded outwards to device pixels (+1 px of anti-aliasing)
fn ibox(b: [f64; 4]) -> [i32; 4] {
    let c = |v: f64| v.clamp(-4.0e6, 4.0e6);
    [values::floor(c(b[0])) as i32 - 1, values::floor(c(b[1])) as i32 - 1, values::floor(c(b[2])) as i32 + 2, values::floor(c(b[3])) as i32 + 2]
}

fn union(a: Option<[f64; 4]>, b: Option<[f64; 4]>) -> Option<[f64; 4]> {
    match (a, b) {
        (Some(a), Some(b)) => Some([a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]),
        (a, None) => a,
        (None, b) => b,
    }
}

// the box of a rectangle x, y, w, h under m
fn rect_box(m: &Matrix, r: [f64; 4]) -> [f64; 4] {
    let pts = [m.apply(r[0], r[1]), m.apply(r[0] + r[2], r[1]), m.apply(r[0], r[1] + r[3]), m.apply(r[0] + r[2], r[1] + r[3])];
    let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for (x, y) in pts {
        b = [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)];
    }
    b
}

/// shape geometry: the node's own path data or one built for the shape
enum Geo<'a> {
    Borrowed(&'a PData),
    Owned(PData),
}

impl Geo<'_> {
    fn pd(&self) -> &PData {
        match self {
            Geo::Borrowed(p) => p,
            Geo::Owned(p) => p,
        }
    }
}

fn ellipse(pd: &mut PData, cx: f64, cy: f64, rx: f64, ry: f64) {
    let (kx, ky) = (rx * KAPPA, ry * KAPPA);
    pd.move_to(cx + rx, cy);
    pd.cubic(&[cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry]);
    pd.cubic(&[cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy]);
    pd.cubic(&[cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry]);
    pd.cubic(&[cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy]);
    pd.close();
}

/// the path data of a basic shape / path (None: nothing to draw)
fn geometry<'a>(n: &'a Node, cs: &Style, vp: Vp) -> Option<Geo<'a>> {
    let fs = cs.fs;
    let lx = |name: &str, d: f64| n.len(name).map_or(d, |l| l.get(fs, vp.w));
    let ly = |name: &str, d: f64| n.len(name).map_or(d, |l| l.get(fs, vp.h));
    let ld = |name: &str| n.len(name).map(|l| l.get(fs, vp.diag()));
    let mut pd = PData::new();
    match n.kind {
        Kind::Path | Kind::Polyline | Kind::Polygon => return n.pd.as_ref().map(Geo::Borrowed),
        Kind::Rect => {
            let (x, y, w, h) = (lx("x", 0.0), ly("y", 0.0), lx("width", 0.0), ly("height", 0.0));
            if !(w > 0.0 && h > 0.0) {
                return None;
            }
            let rx = n.len("rx").map(|l| l.get(fs, vp.w)).filter(|&v| v >= 0.0);
            let ry = n.len("ry").map(|l| l.get(fs, vp.h)).filter(|&v| v >= 0.0);
            let (rx, ry) = match (rx, ry) {
                (Some(a), Some(b)) => (a, b),
                (Some(a), None) => (a, a),
                (None, Some(b)) => (b, b),
                (None, None) => (0.0, 0.0),
            };
            let (rx, ry) = (rx.min(w * 0.5), ry.min(h * 0.5));
            if rx > 0.0 && ry > 0.0 {
                let arc = |pd: &mut PData, x2: f64, y2: f64| {
                    let (cx, cy) = pd.current();
                    pd.arc(&[cx, cy, rx, ry, 0.0, 0.0, 1.0, x2, y2]);
                };
                pd.move_to(x + rx, y);
                pd.line_to(x + w - rx, y);
                arc(&mut pd, x + w, y + ry);
                pd.line_to(x + w, y + h - ry);
                arc(&mut pd, x + w - rx, y + h);
                pd.line_to(x + rx, y + h);
                arc(&mut pd, x, y + h - ry);
                pd.line_to(x, y + ry);
                arc(&mut pd, x + rx, y);
                pd.close();
            } else {
                pd.move_to(x, y);
                pd.line_to(x + w, y);
                pd.line_to(x + w, y + h);
                pd.line_to(x, y + h);
                pd.close();
            }
        }
        Kind::Circle => {
            let r = ld("r").unwrap_or(0.0);
            if !(r > 0.0) {
                return None;
            }
            ellipse(&mut pd, lx("cx", 0.0), ly("cy", 0.0), r, r);
        }
        Kind::Ellipse => {
            let rx = n.len("rx").map(|l| l.get(fs, vp.w));
            let ry = n.len("ry").map(|l| l.get(fs, vp.h));
            let (rx, ry) = match (rx, ry) {
                (Some(a), Some(b)) => (a, b),
                (Some(a), None) => (a, a),
                (None, Some(b)) => (b, b),
                _ => return None,
            };
            if !(rx > 0.0 && ry > 0.0) {
                return None;
            }
            ellipse(&mut pd, lx("cx", 0.0), ly("cy", 0.0), rx, ry);
        }
        Kind::Line => {
            pd.move_to(lx("x1", 0.0), ly("y1", 0.0));
            pd.line_to(lx("x2", 0.0), ly("y2", 0.0));
        }
        _ => return None,
    }
    pd.err = 0;
    Some(Geo::Owned(pd))
}

// ------------------------------------------------------------------ the renderer

struct R<'a, 'h> {
    doc: &'a Svg,
    ras: Raster,
    path: Path,
    buf: Vec<u8>,
    text: Option<&'h mut dyn TextHook>,
    draws: u32,
    bvis: u32,
    depth: u32,
    layers: u32,
    layer_px: u64, // pixels of the layers alive (bounded by MAX_LAYER_PX)
    active: Vec<u32>, // use targets being drawn (cycles)
    op_mul: f64,      // the opacity of groups beyond MAX_LAYERS
}

/// a text run: its string, style and position changes
struct Run {
    s: String,
    st: Style,
    x: Option<f64>,
    y: Option<f64>,
    dx: f64,
    dy: f64,
}

impl<'a, 'h> R<'a, 'h> {
    fn fill_shader(&mut self, t: &mut Tgt, rule: u32, sh: &Shader, mask: Option<&Mask>) {
        let clip = match mask {
            Some(m) => inter(t.clip, m.r),
            None => t.clip,
        };
        if empty(clip) {
            return;
        }
        self.path.fill_lines(&mut self.ras);
        let buf = &mut self.buf;
        self.ras.fill_rows(clip, rule, &mut |y, x, cov| paint::span(t, y, x, cov, sh, mask, buf));
    }

    // the style chain of a node from the root (clipPath children, gradient stops)
    fn style_of(&self, k: u32, root: &Style) -> Style {
        let mut chain = Vec::new();
        let mut p = k;
        while p != NONE && chain.len() < 512 {
            chain.push(p);
            p = self.doc.node(p).map_or(NONE, |n| n.parent);
        }
        let mut s = root.clone();
        for &c in chain.iter().rev() {
            if let Some(n) = self.doc.node(c) {
                s = s.child(n);
            }
        }
        s
    }

    // ---- paints

    /// the shader of a paint (None: nothing to paint)
    #[allow(clippy::too_many_arguments)]
    fn shader(&mut self, p: &Paint, op: f64, cs: &Style, m: &Matrix, vp: Vp, bbox: &mut dyn FnMut(&mut Self) -> Option<[f64; 4]>) -> Option<Shader> {
        match p {
            Paint::None => None,
            Paint::Color(Color::Argb(c)) => paint::solid(*c, op),
            Paint::Color(Color::Current) => paint::solid(cs.color, op),
            Paint::Url(id, fb) => match self.gradient(id, op, cs, m, vp, bbox) {
                Ok(s) => s,
                Err(()) => match fb {
                    Some(Color::Argb(c)) => paint::solid(*c, op),
                    Some(Color::Current) => paint::solid(cs.color, op),
                    None => None,
                },
            },
        }
    }

    /// a gradient by id: Err when the reference is not a gradient (the fallback applies)
    #[allow(clippy::too_many_arguments)]
    fn gradient(&mut self, id: &str, op: f64, cs: &Style, m: &Matrix, vp: Vp, bbox: &mut dyn FnMut(&mut Self) -> Option<[f64; 4]>) -> Result<Option<Shader>, ()> {
        let doc = self.doc;
        let k = doc.by_id(id).ok_or(())?;
        let n = doc.node(k).ok_or(())?;
        let radial = match n.kind {
            Kind::LinearGradient => false,
            Kind::RadialGradient => true,
            _ => return Err(()),
        };
        // the href chain
        let mut chain = alloc::vec![k];
        let mut cur = n;
        while chain.len() < 16 {
            let Some(next) = doc.href_target(cur) else { break };
            if chain.contains(&next) {
                break;
            }
            let Some(nn) = doc.node(next) else { break };
            if !matches!(nn.kind, Kind::LinearGradient | Kind::RadialGradient) {
                break;
            }
            chain.push(next);
            cur = nn;
        }
        let attr = |name: &str| chain.iter().filter_map(|&c| doc.node(c)).find_map(|g| g.attr(name));
        let tf = chain.iter().filter_map(|&c| doc.node(c)).find(|g| g.attr("gradientTransform").is_some()).and_then(|g| g.tf);
        let user = attr("gradientUnits").is_some_and(|u| u.trim() == "userSpaceOnUse");
        let spread = match attr("spreadMethod").map(|s| s.trim()) {
            Some("reflect") => 1,
            Some("repeat") => 2,
            _ => 0,
        };
        // the stops of the first gradient in the chain that has some
        let root = Style::root(cs.color);
        let mut stops: Vec<(f64, u32)> = Vec::new();
        for &c in &chain {
            let Some(g) = doc.node(c) else { continue };
            for kid in &g.kids {
                let Item::El(s) = kid else { continue };
                let Some(sn) = doc.node(*s) else { continue };
                if sn.kind != Kind::Stop || stops.len() >= MAX_STOPS {
                    continue;
                }
                let off = sn.attr("offset").and_then(values::length).map_or(0.0, |l| if l.u == Unit::Pct { l.v * 0.01 } else { l.v });
                let off = off.clamp(0.0, 1.0).max(stops.last().map_or(0.0, |l| l.0));
                let col = match sn.prop(P_STOP_COLOR).and_then(values::color) {
                    Some(Color::Argb(c)) => c,
                    Some(Color::Current) => self.style_of(*s, &root).color,
                    None => 0xFF00_0000,
                };
                let so = sn.prop(P_STOP_OPACITY).and_then(values::opacity).unwrap_or(1.0);
                let a = ((col >> 24) as f64 * so + 0.5) as u32;
                stops.push((off, a.min(255) << 24 | (col & 0xFF_FFFF)));
            }
            if !stops.is_empty() {
                break;
            }
        }
        if stops.is_empty() {
            return Ok(None);
        }
        let last = stops[stops.len() - 1].1;
        if stops.len() == 1 {
            return Ok(paint::solid(last, op));
        }
        // the gradient space -> device
        let mut g = *m;
        let (rw, rh, rd) = if user {
            (vp.w, vp.h, vp.diag())
        } else {
            let Some(b) = bbox(self) else { return Ok(None) };
            let (bw, bh) = (b[2] - b[0], b[3] - b[1]);
            if !(bw > 0.0 && bh > 0.0) {
                return Ok(None);
            }
            g = mul(&g, &Matrix { a: bw, b: 0.0, c: b[0], d: 0.0, e: bh, f: b[1] });
            (1.0, 1.0, 1.0)
        };
        if let Some(t) = tf {
            g = mul(&g, &t);
        }
        let Some(inv) = invert(&g) else { return Ok(None) };
        let fs = cs.fs;
        let num = |name: &str, d: f64, of: f64| -> f64 {
            match attr(name).and_then(values::length) {
                Some(l) if user => l.get(fs, of),
                Some(l) => match l.u {
                    Unit::Pct => l.v * 0.01,
                    _ => l.v,
                },
                None => d,
            }
        };
        let lut = paint::lut(&stops, op);
        let mut gr = Grad { radial, lin: [0.0; 3], inv, f: (0.0, 0.0), fr: 0.0, cd: (0.0, 0.0), dr: 0.0, a: 0.0, spread, lut };
        if !radial {
            let (x1, y1) = (num("x1", 0.0, rw), num("y1", 0.0, rh));
            let (x2, y2) = (num("x2", if user { vp.w } else { 1.0 }, rw), num("y2", 0.0, rh));
            let (dx, dy) = (x2 - x1, y2 - y1);
            let l2 = dx * dx + dy * dy;
            if !(l2 > 0.0) {
                return Ok(paint::solid(last, op));
            }
            let i = &inv;
            gr.lin = [(i.a * dx + i.d * dy) / l2, (i.b * dx + i.e * dy) / l2, ((i.c - x1) * dx + (i.f - y1) * dy) / l2];
        } else {
            let half = |of: f64| if user { of * 0.5 } else { 0.5 };
            let (cx, cy, r) = (num("cx", half(rw), rw), num("cy", half(rh), rh), num("r", half(rd), rd));
            if !(r > 0.0) {
                return Ok(paint::solid(last, op));
            }
            let (mut fx, mut fy) = (num("fx", cx, rw), num("fy", cy, rh));
            let fr = num("fr", 0.0, rd).clamp(0.0, r);
            // a focal point outside the circle moves onto its edge (SVG 1.1)
            let (ddx, ddy) = (fx - cx, fy - cy);
            let d = sqrtd(ddx * ddx + ddy * ddy);
            if d > r * 0.999 {
                let k = r * 0.999 / d;
                fx = cx + ddx * k;
                fy = cy + ddy * k;
            }
            gr.f = (fx, fy);
            gr.fr = fr;
            gr.cd = (cx - fx, cy - fy);
            gr.dr = r - fr;
            gr.a = gr.cd.0 * gr.cd.0 + gr.cd.1 * gr.cd.1 - gr.dr * gr.dr;
        }
        Ok(Some(Shader::Grad(gr)))
    }

    // ---- bounds

    /// the bounds of element k (and its content) under m; stroke: widened by the stroke
    fn bounds(&mut self, k: u32, ps: &Style, m: &Matrix, vp: Vp, stroke: bool, depth: u32) -> Option<[f64; 4]> {
        self.bounds_in(k, ps, m, vp, stroke, depth, true)
    }

    /// bounds; own_tf false: without the element's own transform (its user box)
    #[allow(clippy::too_many_arguments)]
    fn bounds_in(&mut self, k: u32, ps: &Style, m: &Matrix, vp: Vp, stroke: bool, depth: u32, own_tf: bool) -> Option<[f64; 4]> {
        let doc = self.doc;
        let n = doc.node(k)?;
        if depth > MAX_NEST || display_none(n) || self.bvis >= MAX_BOUNDS {
            return None;
        }
        self.bvis += 1;
        let cs = ps.child(n);
        let m2 = match n.tf {
            Some(t) if own_tf && !matches!(n.kind, Kind::LinearGradient | Kind::RadialGradient) => mul(m, &t),
            _ => *m,
        };
        match n.kind {
            // a nested viewport: its mapping is not followed here (the whole area)
            Kind::Svg if k != 0 => Some(ALL),
            Kind::G | Kind::A | Kind::Switch | Kind::Svg | Kind::Symbol => {
                let mut b = None;
                for kid in &n.kids {
                    if let Item::El(c) = kid {
                        b = union(b, self.bounds(*c, &cs, &m2, vp, stroke, depth + 1));
                    }
                }
                b
            }
            Kind::Use => {
                let tgt = doc.href_target(n)?;
                if self.active.contains(&tgt) || depth > 16 {
                    return None;
                }
                if doc.node(tgt).is_some_and(|x| matches!(x.kind, Kind::Symbol | Kind::Svg)) {
                    return Some(ALL); // a new viewport (not followed)
                }
                let (x, y) = (n.len("x").map_or(0.0, |l| l.get(cs.fs, vp.w)), n.len("y").map_or(0.0, |l| l.get(cs.fs, vp.h)));
                let m3 = mul(&m2, &translate(x, y));
                self.active.push(tgt);
                let b = self.bounds(tgt, &cs, &m3, vp, stroke, depth + 1);
                self.active.pop();
                b
            }
            Kind::Text => {
                let (x, y) = (first_len(n, "x", cs.fs, vp.w), first_len(n, "y", cs.fs, vp.h));
                let w = self.text_width(n, &cs);
                let x0 = match cs.anchor {
                    1 => x - w * 0.5,
                    2 => x - w,
                    _ => x,
                };
                Some(rect_box(&m2, [x0, y - cs.fs, w.max(cs.fs * 0.5), cs.fs * 1.3]))
            }
            Kind::Image => {
                let size = doc.with_image(n.img?, |s| (s.w, s.h))?;
                let r = image_rect(n, &cs, vp, size)?;
                Some(rect_box(&m2, r))
            }
            _ => {
                let g = geometry(n, &cs, vp)?;
                let mut b = g.pd().bounds(Some(&m2))?;
                if stroke && cs.stroke != Paint::None {
                    let w = cs.sw.get(cs.fs, vp.diag()) * scale_of(&m2);
                    let k = if cs.join == JOIN_MITER { cs.miter.max(1.0) } else { 1.0 };
                    let e = (w * 0.5 * k).min(1.0e6) + 1.0;
                    b = [b[0] - e, b[1] - e, b[2] + e, b[3] + e];
                }
                Some(b)
            }
        }
    }

    // ---- clipping

    /// the mask of clip-path reference id for an element drawn with matrix m (bbox: its user
    /// box); None = no clipping (an invalid reference), Some(mask) else
    #[allow(clippy::too_many_arguments)]
    fn clip_mask(&mut self, id: &str, m: &Matrix, vp: Vp, area: [i32; 4], bbox: Option<[f64; 4]>, parent: Option<&Mask>, depth: u32) -> Option<Mask> {
        let doc = self.doc;
        let k = doc.by_id(id)?;
        let n = doc.node(k)?;
        if n.kind != Kind::ClipPath {
            return None;
        }
        let mut area = area;
        if let Some(p) = parent {
            area = inter(area, p.r);
        }
        let mut mm = *m;
        if n.attr("clipPathUnits").is_some_and(|u| u.trim() == "objectBoundingBox") {
            match bbox {
                Some(b) if b[2] > b[0] && b[3] > b[1] => {
                    mm = mul(&mm, &Matrix { a: b[2] - b[0], b: 0.0, c: b[0], d: 0.0, e: b[3] - b[1], f: b[1] });
                }
                _ => return Some(Mask::new([0, 0, 0, 0])),
            }
        }
        if let Some(t) = n.tf {
            mm = mul(&mm, &t);
        }
        let root = Style::root(0xFF00_0000);
        let cs = self.style_of(k, &root);
        // the bounds of the clip shapes
        let mut b = None;
        for kid in &n.kids {
            if let Item::El(c) = kid {
                b = union(b, self.bounds(*c, &cs, &mm, vp, false, depth + 1));
            }
        }
        let Some(b) = b else { return Some(Mask::new([0, 0, 0, 0])) };
        let r = inter(area, ibox(b));
        if empty(r) {
            return Some(Mask::new([0, 0, 0, 0]));
        }
        let mut mask = Mask::new(r);
        for kid in &n.kids {
            if let Item::El(c) = kid {
                self.clip_shape(&mut mask, *c, &cs, &mm, vp, 0);
            }
        }
        // the clipPath's own clip-path
        if depth < 8 {
            if let Some(inner) = n.prop(P_CLIP_PATH).and_then(values::url_id) {
                if inner != id {
                    if let Some(im) = self.clip_mask(inner, m, vp, r, bbox, None, depth + 1) {
                        mask.intersect(&im);
                    }
                }
            }
        }
        if let Some(p) = parent {
            mask.intersect(p);
        }
        Some(mask)
    }

    // a child of a clipPath into the mask (shapes, text, use of those)
    fn clip_shape(&mut self, mask: &mut Mask, k: u32, ps: &Style, m: &Matrix, vp: Vp, depth: u32) {
        let doc = self.doc;
        let Some(n) = doc.node(k) else { return };
        if display_none(n) || depth > 8 {
            return;
        }
        let cs = ps.child(n);
        if !cs.visible {
            return;
        }
        let m2 = match n.tf {
            Some(t) => mul(m, &t),
            None => *m,
        };
        match n.kind {
            Kind::Use => {
                let Some(tgt) = doc.href_target(n) else { return };
                let (x, y) = (n.len("x").map_or(0.0, |l| l.get(cs.fs, vp.w)), n.len("y").map_or(0.0, |l| l.get(cs.fs, vp.h)));
                self.clip_shape(mask, tgt, &cs, &mul(&m2, &translate(x, y)), vp, depth + 1);
            }
            Kind::Text => {
                if self.text_path(n, &cs, &m2, vp) {
                    self.mask_fill(mask, cs.clip_rule);
                }
            }
            _ => {
                let Some(g) = geometry(n, &cs, vp) else { return };
                self.path.reset();
                if g.pd().emit(&mut self.path, Some(&m2)) != 0 {
                    return;
                }
                self.mask_fill(mask, cs.clip_rule);
            }
        }
    }

    fn mask_fill(&mut self, mask: &mut Mask, rule: u32) {
        self.path.fill_lines(&mut self.ras);
        let r = mask.r;
        self.ras.fill_rows(r, rule, &mut |y, x, cov| mask.add(y, x, cov));
    }

    // a rectangular clip (nested svg viewports) as a mask
    fn rect_mask(&mut self, m: &Matrix, r: [f64; 4], area: [i32; 4], parent: Option<&Mask>) -> Mask {
        let mut pd = PData::new();
        pd.move_to(r[0], r[1]);
        pd.line_to(r[0] + r[2], r[1]);
        pd.line_to(r[0] + r[2], r[1] + r[3]);
        pd.line_to(r[0], r[1] + r[3]);
        pd.close();
        let mut area = inter(area, ibox(rect_box(m, r)));
        if let Some(p) = parent {
            area = inter(area, p.r);
        }
        let mut mask = Mask::new(if empty(area) { [0, 0, 0, 0] } else { area });
        if !empty(area) {
            self.path.reset();
            if pd.emit(&mut self.path, Some(m)) == 0 {
                self.mask_fill(&mut mask, RULE_NONZERO);
            }
            if let Some(p) = parent {
                mask.intersect(p);
            }
        }
        mask
    }

    // ---- the element walk

    /// draw element k; ps = the parent's style
    #[allow(clippy::too_many_arguments)]
    fn el(&mut self, t: &mut Tgt, k: u32, ps: &Style, m: &Matrix, vp: Vp, mask: Option<&Mask>) {
        let doc = self.doc;
        let Some(n) = doc.node(k) else { return };
        let drawable = matches!(
            n.kind,
            Kind::Svg | Kind::G | Kind::A | Kind::Switch | Kind::Use | Kind::Path | Kind::Rect | Kind::Circle | Kind::Ellipse | Kind::Line
                | Kind::Polyline | Kind::Polygon | Kind::Text | Kind::Image
        );
        if !drawable || display_none(n) || self.draws >= MAX_DRAWS || self.depth >= MAX_NEST || empty(t.clip) {
            return;
        }
        self.draws += 1;
        let cs = ps.child(n);
        // the element's user space (its transform applied); m2 adds a nested viewport
        let mu = match n.tf {
            Some(tf) => mul(m, &tf),
            None => *m,
        };
        let mut m2 = mu;
        let mut vp2 = vp;
        // a nested svg: its viewport
        let mut viewport: Option<[f64; 4]> = None;
        if n.kind == Kind::Svg && k != 0 {
            let x = n.len("x").map_or(0.0, |l| l.get(cs.fs, vp.w));
            let y = n.len("y").map_or(0.0, |l| l.get(cs.fs, vp.h));
            let w = n.len("width").map_or(vp.w, |l| l.get(cs.fs, vp.w));
            let h = n.len("height").map_or(vp.h, |l| l.get(cs.fs, vp.h));
            if !(w > 0.0 && h > 0.0) {
                return;
            }
            if !overflow_visible(n) {
                viewport = Some([x, y, w, h]);
            }
            match n.attr("viewBox").and_then(values::view_box) {
                Some(vb) => {
                    let p = n.attr("preserveAspectRatio").map(values::par).unwrap_or_default();
                    m2 = mul(&m2, &values::view_matrix(vb, [x, y, w, h], p));
                    vp2 = Vp { w: vb[2], h: vb[3] };
                }
                None => {
                    m2 = mul(&m2, &translate(x, y));
                    vp2 = Vp { w, h };
                }
            }
        }
        // opacity and clip-path
        let op = opacity_of(n);
        if op <= 0.0 {
            return;
        }
        let own_mask = match n.prop(P_CLIP_PATH).and_then(values::url_id) {
            Some(id) => {
                let id = String::from(id);
                // the element's user box (objectBoundingBox clip units)
                let bb = self.bounds_in(k, ps, &Matrix::IDENTITY, vp, false, 0, false);
                self.clip_mask(&id, &mu, vp, t.clip, bb, mask, 0)
            }
            None => None,
        };
        let vmask = viewport.map(|r| self.rect_mask(&mu, r, t.clip, own_mask.as_ref().or(mask)));
        let mask: Option<&Mask> = vmask.as_ref().or(own_mask.as_ref()).or(mask);
        if mask.is_some_and(|mk| empty(mk.r)) {
            return;
        }
        // a leaf that paints one thing takes the opacity into that paint; groups and the
        // others are drawn into a layer (beyond MAX_LAYERS the opacity goes to every paint)
        let leaf = !matches!(n.kind, Kind::Svg | Kind::G | Kind::A | Kind::Switch | Kind::Use);
        let paints = (cs.fill != Paint::None) as u32 + (cs.stroke != Paint::None) as u32;
        let layer = op < 1.0 && (!leaf || paints > 1 || n.kind == Kind::Text) && self.layers < MAX_LAYERS && self.layer_px < MAX_LAYER_PX;
        let saved_mul = self.op_mul;
        let fold = if layer {
            self.op_mul
        } else if leaf {
            op * self.op_mul
        } else {
            self.op_mul *= op;
            self.op_mul
        };
        self.depth += 1;
        if layer {
            let b = self.bounds(k, ps, m, vp, true, 0);
            let mut r = t.clip;
            if let Some(b) = b {
                r = inter(r, ibox(b));
            }
            if let Some(mk) = mask {
                r = inter(r, mk.r);
            }
            if !empty(r) {
                let mut sf = Surface::new(r[2] - r[0], r[3] - r[1], FMT_PARGB);
                if !sf.px.is_empty() {
                    self.layers += 1;
                    let lpx = sf.w as u64 * sf.h as u64;
                    self.layer_px += lpx;
                    {
                        let mut lt = Tgt { sf: &mut sf, ox: r[0], oy: r[1], clip: r };
                        self.content(&mut lt, n, &cs, &m2, vp2, None, fold);
                    }
                    self.layers -= 1;
                    self.layer_px -= lpx;
                    paint::composite(t, &sf, r[0], r[1], (op * 255.0 + 0.5) as u32, mask);
                }
            }
        } else {
            self.content(t, n, &cs, &m2, vp2, mask, fold);
        }
        self.op_mul = saved_mul;
        self.depth -= 1;
    }

    /// the content of element n with its style and matrix (op: opacity folded into the paints)
    #[allow(clippy::too_many_arguments)]
    fn content(&mut self, t: &mut Tgt, n: &Node, cs: &Style, m: &Matrix, vp: Vp, mask: Option<&Mask>, op: f64) {
        let doc = self.doc;
        match n.kind {
            Kind::Svg | Kind::G | Kind::A => {
                for kid in &n.kids {
                    if let Item::El(c) = kid {
                        self.el(t, *c, cs, m, vp, mask);
                    }
                }
            }
            Kind::Switch => {
                // the first child that can be drawn (no conditional attributes evaluated)
                for kid in &n.kids {
                    if let Item::El(c) = kid {
                        if doc.node(*c).is_some_and(|x| !matches!(x.kind, Kind::Other | Kind::Title | Kind::Style)) {
                            self.el(t, *c, cs, m, vp, mask);
                            break;
                        }
                    }
                }
            }
            Kind::Use => {
                let Some(tgt) = doc.href_target(n) else { return };
                if self.active.contains(&tgt) || self.active.len() >= 16 {
                    return; // a cycle (or a chain too long)
                }
                let x = n.len("x").map_or(0.0, |l| l.get(cs.fs, vp.w));
                let y = n.len("y").map_or(0.0, |l| l.get(cs.fs, vp.h));
                let m3 = mul(m, &translate(x, y));
                let Some(tn) = doc.node(tgt) else { return };
                self.active.push(tgt);
                if tn.kind == Kind::Symbol {
                    // the symbol: a new viewport of the use's size
                    if !display_none(tn) {
                        let ss = cs.child(tn);
                        let w = n.len("width").or_else(|| tn.len("width")).map_or(vp.w, |l| l.get(cs.fs, vp.w));
                        let h = n.len("height").or_else(|| tn.len("height")).map_or(vp.h, |l| l.get(cs.fs, vp.h));
                        if w > 0.0 && h > 0.0 {
                            let (mut m4, mut vp3) = (m3, Vp { w, h });
                            if let Some(vb) = tn.attr("viewBox").and_then(values::view_box) {
                                let p = tn.attr("preserveAspectRatio").map(values::par).unwrap_or_default();
                                m4 = mul(&m3, &values::view_matrix(vb, [0.0, 0.0, w, h], p));
                                vp3 = Vp { w: vb[2], h: vb[3] };
                            }
                            let vmask = if overflow_visible(tn) { None } else { Some(self.rect_mask(&m3, [0.0, 0.0, w, h], t.clip, mask)) };
                            let mk = vmask.as_ref().or(mask);
                            if !mk.is_some_and(|x| empty(x.r)) {
                                for kid in &tn.kids {
                                    if let Item::El(c) = kid {
                                        self.el(t, *c, &ss, &m4, vp3, mk);
                                    }
                                }
                            }
                        }
                    }
                } else {
                    self.el(t, tgt, cs, &m3, vp, mask);
                }
                self.active.pop();
            }
            Kind::Text => self.text(t, n, cs, m, vp, mask, op),
            Kind::Image => self.image(t, n, cs, m, vp, mask, op),
            _ => {
                if !cs.visible {
                    return;
                }
                let Some(g) = geometry(n, cs, vp) else { return };
                let pd = g.pd();
                let mut bb = |_: &mut Self| pd.bounds(None);
                // fill
                if cs.fill != Paint::None {
                    if let Some(sh) = self.shader(&cs.fill, cs.fill_op * op, cs, m, vp, &mut bb) {
                        self.path.reset();
                        if pd.emit(&mut self.path, Some(m)) == 0 {
                            self.fill_shader(t, cs.fill_rule, &sh, mask);
                        }
                    }
                }
                // stroke
                if cs.stroke != Paint::None {
                    if let Some(sh) = self.shader(&cs.stroke, cs.stroke_op * op, cs, m, vp, &mut bb) {
                        self.path.reset();
                        if pd.emit(&mut self.path, Some(m)) == 0 && self.stroke_outline(cs, m, vp) {
                            self.fill_shader(t, RULE_NONZERO, &sh, mask);
                        }
                    }
                }
            }
        }
    }

    /// replace the path (device space) by the outline of its stroke; false: nothing to draw
    fn stroke_outline(&mut self, cs: &Style, m: &Matrix, vp: Vp) -> bool {
        let wu = cs.sw.get(cs.fs, vp.diag());
        let sc = scale_of(m);
        let w = wu * sc;
        if !(w > 0.0) || !(w < 1.0e6) {
            return false;
        }
        let mut pat: Vec<f64> = Vec::new();
        let mut off = 0.0;
        if let Some(d) = &cs.dash {
            let vals: Vec<f64> = d.iter().map(|l| l.get(cs.fs, vp.diag())).collect();
            let mut full = vals.clone();
            if full.len() % 2 == 1 {
                full.extend_from_slice(&vals);
            }
            let period: f64 = full.iter().sum::<f64>() * sc;
            // a pattern far below a pixel: drawn solid
            if period > 0.25 && full.len() <= DASH_MAXPAT {
                pat = full.iter().map(|v| v / wu).collect();
                off = cs.dash_off.get(cs.fs, vp.diag()) / wu;
            }
        }
        let st = DStyle { width: w, pat: &pat, join: cs.join, cap0: cs.cap, cap1: cs.cap, dcap: cs.cap, offset: off, miter: cs.miter };
        dash_stroke(&mut self.path, &st) == 0
    }

    // ---- images

    #[allow(clippy::too_many_arguments)]
    fn image(&mut self, t: &mut Tgt, n: &Node, cs: &Style, m: &Matrix, vp: Vp, mask: Option<&Mask>, op: f64) {
        let doc = self.doc;
        if !cs.visible {
            return;
        }
        let Some(i) = n.img else { return };
        doc.with_image(i, |src| self.image_with(t, n, cs, m, vp, mask, op, src));
    }

    #[allow(clippy::too_many_arguments)]
    fn image_with(&mut self, t: &mut Tgt, n: &Node, cs: &Style, m: &Matrix, vp: Vp, mask: Option<&Mask>, op: f64, src: &Surface) {
        let Some(r) = image_rect(n, cs, vp, (src.w, src.h)) else { return };
        let p = n.attr("preserveAspectRatio").map(values::par).unwrap_or_default();
        let place = values::view_matrix([0.0, 0.0, src.w as f64, src.h as f64], r, p);
        let f = mul(m, &place);
        let mut area = inter(t.clip, ibox(rect_box(m, r)));
        area = inter(area, ibox(rect_box(&f, [0.0, 0.0, src.w as f64, src.h as f64])));
        if let Some(mk) = mask {
            area = inter(area, mk.r);
        }
        if empty(area) {
            return;
        }
        let Some(inv) = invert(&f) else { return };
        let mut tmp = Surface::new(area[2] - area[0], area[3] - area[1], FMT_PARGB);
        if tmp.px.is_empty() {
            return;
        }
        // tmp pixel (x, y) is device (x + area[0], y + area[1])
        let a = mul(&inv, &translate(area[0] as f64, area[1] as f64));
        // shrunk images: 16 samples per pixel (no aliasing down to about 1/4)
        let filter = if scale_of(&f) < 0.7 { img_core::resample::RS_MULTISAMPLE } else { img_core::resample::RS_BILINEAR };
        if affine(src, &mut tmp, &a, filter, XF_TRANSPARENT, None) != 0 {
            return;
        }
        // slice: the part outside the viewport rectangle is clipped
        let vm;
        let mk = if p.slice {
            vm = self.rect_mask(m, r, area, mask);
            Some(&vm)
        } else {
            mask
        };
        paint::composite(t, &tmp, area[0], area[1], (op * 255.0 + 0.5) as u32, mk);
    }

    // ---- text

    // the runs of a text element (its text and tspans, blanks collapsed)
    fn runs(&self, n: &Node, cs: &Style, out: &mut Vec<Run>, depth: u32) {
        if depth > 16 {
            return;
        }
        for kid in &n.kids {
            match kid {
                Item::Text(s) => {
                    let mut t = String::with_capacity(s.len());
                    for ch in s.chars() {
                        match ch {
                            '\n' | '\r' => {}
                            '\t' => t.push(' '),
                            c => t.push(c),
                        }
                    }
                    out.push(Run { s: t, st: cs.clone(), x: None, y: None, dx: 0.0, dy: 0.0 });
                }
                Item::El(c) => {
                    let Some(cn) = self.doc.node(*c) else { continue };
                    if !matches!(cn.kind, Kind::Tspan | Kind::A) || display_none(cn) {
                        continue;
                    }
                    let st = cs.child(cn);
                    let start = out.len();
                    self.runs(cn, &st, out, depth + 1);
                    if let Some(first) = out.get_mut(start) {
                        first.x = first_opt(cn, "x", st.fs, 0.0);
                        first.y = first_opt(cn, "y", st.fs, 0.0);
                        first.dx = first_opt(cn, "dx", st.fs, 0.0).unwrap_or(0.0);
                        first.dy = first_opt(cn, "dy", st.fs, 0.0).unwrap_or(0.0);
                    }
                }
            }
        }
    }

    // the runs with the blanks collapsed across them (xml:space default)
    fn text_runs(&self, n: &Node, cs: &Style) -> Vec<Run> {
        let mut runs = Vec::new();
        self.runs(n, cs, &mut runs, 0);
        let mut prev_space = true;
        for r in runs.iter_mut() {
            let mut t = String::with_capacity(r.s.len());
            for ch in r.s.chars() {
                if ch == ' ' {
                    if !prev_space {
                        t.push(' ');
                    }
                    prev_space = true;
                } else {
                    t.push(ch);
                    prev_space = false;
                }
            }
            r.s = t;
        }
        // the trailing blank of the last run with text
        if let Some(last) = runs.iter_mut().rev().find(|r| !r.s.is_empty()) {
            if last.s.ends_with(' ') {
                last.s.pop();
            }
        }
        runs
    }

    fn text_width(&mut self, n: &Node, cs: &Style) -> f64 {
        let runs = self.text_runs(n, cs);
        let Some(h) = self.text.as_mut() else { return 0.0 };
        let mut w = 0.0;
        for r in &runs {
            let f = FontSpec { family: &r.st.family, weight: r.st.weight, italic: r.st.italic };
            w += h.advance(&r.s, &f, r.st.fs as f32) as f64 + r.dx;
        }
        w
    }

    // the outline of a whole text element into the path (device space; clip paths)
    fn text_path(&mut self, n: &Node, cs: &Style, m: &Matrix, vp: Vp) -> bool {
        self.path.reset();
        let runs = self.text_runs(n, cs);
        let pos = self.layout(n, cs, vp, &runs);
        let mut any = false;
        for (i, r) in runs.iter().enumerate() {
            any |= self.run_outline(r, pos[i], m, false);
        }
        any
    }

    // the start of every run (user units), text-anchor applied per chunk
    fn layout(&mut self, n: &Node, cs: &Style, vp: Vp, runs: &[Run]) -> Vec<(f64, f64)> {
        let mut pos = Vec::with_capacity(runs.len());
        let (mut x, mut y) = (first_len(n, "x", cs.fs, vp.w), first_len(n, "y", cs.fs, vp.h));
        x += first_opt(n, "dx", cs.fs, vp.w).unwrap_or(0.0);
        y += first_opt(n, "dy", cs.fs, vp.h).unwrap_or(0.0);
        let mut chunk_start = 0;
        let mut chunk_x0 = x;
        let mut anchor = cs.anchor;
        for (i, r) in runs.iter().enumerate() {
            if (r.x.is_some() || r.y.is_some()) && i > 0 {
                self.anchor_chunk(&mut pos, chunk_start, x - chunk_x0, anchor);
                chunk_start = i;
                anchor = r.st.anchor;
                if let Some(v) = r.x {
                    x = v;
                }
                if let Some(v) = r.y {
                    y = v;
                }
                chunk_x0 = x;
            } else if i == 0 {
                if let Some(v) = r.x {
                    x = v;
                    chunk_x0 = x;
                }
                if let Some(v) = r.y {
                    y = v;
                }
            }
            x += r.dx;
            y += r.dy;
            if r.dx != 0.0 && i == chunk_start {
                chunk_x0 = x;
            }
            pos.push((x, y));
            let w = match self.text.as_mut() {
                Some(h) => {
                    let f = FontSpec { family: &r.st.family, weight: r.st.weight, italic: r.st.italic };
                    h.advance(&r.s, &f, r.st.fs as f32) as f64
                }
                None => 0.0,
            };
            x += w;
        }
        self.anchor_chunk(&mut pos, chunk_start, x - chunk_x0, anchor);
        pos
    }

    fn anchor_chunk(&self, pos: &mut [(f64, f64)], from: usize, w: f64, anchor: u8) {
        let d = match anchor {
            1 => -w * 0.5,
            2 => -w,
            _ => return,
        };
        for p in pos.iter_mut().skip(from) {
            p.0 += d;
        }
    }

    // the outline of one run appended to the path (device space): false when nothing came
    fn run_outline(&mut self, r: &Run, at: (f64, f64), m: &Matrix, reset: bool) -> bool {
        if r.s.is_empty() || !(r.st.fs > 0.0) {
            return false;
        }
        let sc = scale_of(m);
        if !(sc > 0.0) {
            return false;
        }
        if reset {
            self.path.reset();
        }
        let Some(h) = self.text.as_mut() else { return false };
        let n0 = self.path.pts_mut().len();
        let f = FontSpec { family: &r.st.family, weight: r.st.weight, italic: r.st.italic };
        let px = (r.st.fs * sc).min(1.0e5) as f32;
        h.outline(&mut self.path, &r.s, &f, px, (at.0 * sc) as f32, (at.1 * sc) as f32);
        // scaled user space -> device
        let inv = 1.0 / sc;
        let pts = self.path.pts_mut();
        if pts.len() <= n0 {
            return false;
        }
        for p in pts[n0..].iter_mut() {
            let (x, y) = m.apply(p.0 as f64 * inv, p.1 as f64 * inv);
            *p = (x.clamp(-4.0e6, 4.0e6) as f32, y.clamp(-4.0e6, 4.0e6) as f32);
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn text(&mut self, t: &mut Tgt, n: &Node, cs: &Style, m: &Matrix, vp: Vp, mask: Option<&Mask>, op: f64) {
        if self.text.is_none() {
            return;
        }
        let runs = self.text_runs(n, cs);
        let pos = self.layout(n, cs, vp, &runs);
        // the user box of the whole text (objectBoundingBox paints)
        let mut bx: Option<[f64; 4]> = None;
        for (i, r) in runs.iter().enumerate() {
            let w = match self.text.as_mut() {
                Some(h) => {
                    let f = FontSpec { family: &r.st.family, weight: r.st.weight, italic: r.st.italic };
                    h.advance(&r.s, &f, r.st.fs as f32) as f64
                }
                None => 0.0,
            };
            let (x, y) = pos[i];
            bx = union(bx, Some([x, y - r.st.fs * 0.8, x + w, y + r.st.fs * 0.2]));
        }
        for (i, r) in runs.iter().enumerate() {
            let st = &r.st;
            if !st.visible || r.s.is_empty() {
                continue;
            }
            let mut bb = |_: &mut Self| bx;
            if st.fill != Paint::None {
                if let Some(sh) = self.shader(&st.fill, st.fill_op * op, st, m, vp, &mut bb) {
                    if self.run_outline(r, pos[i], m, true) {
                        self.fill_shader(t, RULE_NONZERO, &sh, mask);
                    }
                }
            }
            if st.stroke != Paint::None {
                if let Some(sh) = self.shader(&st.stroke, st.stroke_op * op, st, m, vp, &mut bb) {
                    if self.run_outline(r, pos[i], m, true) && self.stroke_outline(st, m, vp) {
                        self.fill_shader(t, RULE_NONZERO, &sh, mask);
                    }
                }
            }
        }
    }
}

// the first value of a coordinate list attribute ("x", "dx" of text)
fn first_opt(n: &Node, name: &str, fs: f64, of: f64) -> Option<f64> {
    let v = n.attr(name)?;
    let s = v.as_bytes();
    let mut i = 0;
    values::sep(s, &mut i);
    values::length_at(s, &mut i).map(|l| l.get(fs, of))
}

fn first_len(n: &Node, name: &str, fs: f64, of: f64) -> f64 {
    first_opt(n, name, fs, of).unwrap_or(0.0)
}

// the viewport rectangle of an <image> (width / height default to the image's size)
fn image_rect(n: &Node, cs: &Style, vp: Vp, size: (i32, i32)) -> Option<[f64; 4]> {
    let x = n.len("x").map_or(0.0, |l| l.get(cs.fs, vp.w));
    let y = n.len("y").map_or(0.0, |l| l.get(cs.fs, vp.h));
    let w = n.len("width").map_or(size.0 as f64, |l| l.get(cs.fs, vp.w));
    let h = n.len("height").map_or(size.1 as f64, |l| l.get(cs.fs, vp.h));
    if w > 0.0 && h > 0.0 { Some([x, y, w, h]) } else { None }
}

impl Svg {
    /// draw the document: its viewport (the intrinsic size, viewBox mapped by
    /// preserveAspectRatio) through m (intrinsic px -> surface px); clip: the surface
    /// rectangle x0, y0, x1, y1 (exclusive) that may change (None: all of it).  Content
    /// outside the viewport is clipped (to its box under m).  The surface: FMT_PARGB, or the
    /// window's FMT_XRGB (blended the same way)
    pub fn render(&self, sf: &mut Surface, m: &Matrix, clip: Option<[i32; 4]>, opts: &mut Opts) {
        self.render_vp(sf, m, self.width, self.height, clip, opts);
    }

    /// draw the document into the rectangle x, y, w, h of the surface as its viewport (the
    /// viewBox mapped into it by preserveAspectRatio: sharp at any size)
    #[allow(clippy::too_many_arguments)]
    pub fn render_rect(&self, sf: &mut Surface, x: f64, y: f64, w: f64, h: f64, clip: Option<[i32; 4]>, opts: &mut Opts) {
        if !(w > 0.0 && h > 0.0) {
            return;
        }
        self.render_vp(sf, &translate(x, y), w, h, clip, opts);
    }

    /// a new FMT_PARGB surface w x h with the document drawn into it (render_rect);
    /// Err(ERR_DIMS) for sides outside 1 .. 65535, Err(ERR_LIMIT) beyond IMG_MAXPX_DEF pixels
    pub fn rasterize(&self, w: i32, h: i32, opts: &mut Opts) -> Result<Surface, i32> {
        if !(1..=65535).contains(&w) || !(1..=65535).contains(&h) {
            return Err(img_core::ERR_DIMS);
        }
        if w as u64 * h as u64 > img_core::IMG_MAXPX_DEF {
            return Err(img_core::ERR_LIMIT);
        }
        let mut sf = Surface::new(w, h, FMT_PARGB);
        if sf.px.is_empty() {
            return Err(img_core::ERR_NOMEM);
        }
        self.render_rect(&mut sf, 0.0, 0.0, w as f64, h as f64, None, opts);
        Ok(sf)
    }

    // the root: viewport vw x vh under m
    fn render_vp(&self, sf: &mut Surface, m: &Matrix, vw: f64, vh: f64, clip: Option<[i32; 4]>, opts: &mut Opts) {
        let mut area = [0, 0, sf.w, sf.h];
        if let Some(c) = clip {
            area = inter(area, c);
        }
        area = inter(area, ibox(rect_box(m, [0.0, 0.0, vw, vh])));
        if empty(area) || self.nodes.is_empty() {
            return;
        }
        let (rm, vp) = match self.view_box {
            Some(vb) => (mul(m, &values::view_matrix(vb, [0.0, 0.0, vw, vh], self.par)), Vp { w: vb[2], h: vb[3] }),
            None => (*m, Vp { w: vw, h: vh }),
        };
        let mut r = R {
            doc: self,
            ras: Raster::default(),
            path: Path::default(),
            buf: Vec::new(),
            text: opts.text.as_mut().map(|h| &mut **h as &mut dyn TextHook),
            draws: 0,
            bvis: 0,
            depth: 0,
            layers: 0,
            layer_px: 0,
            active: Vec::new(),
            op_mul: 1.0,
        };
        // the root's rectangle clip when m rotates / skews (an axis-aligned m is exact)
        let rot = m.b != 0.0 || m.d != 0.0;
        let mut t = Tgt { sf, ox: 0, oy: 0, clip: area };
        let root = Style::root(opts.color);
        if rot {
            let mk = r.rect_mask(m, [0.0, 0.0, vw, vh], area, None);
            r.el(&mut t, 0, &root, &rm, vp, Some(&mk));
        } else {
            r.el(&mut t, 0, &root, &rm, vp, None);
        }
    }
}
