// doc -- the parsed document: elements with their kind, attributes, cascaded properties,
// pre-built path data, the id map, the intrinsic size and the embedded raster images
//
// The cascade runs once at parse time: presentation attributes, then the matching rules of
// every <style> sheet (by specificity, then order), then the style attribute; later
// declarations win.  Inheritance and "inherit" are resolved while rendering (render.rs),
// because `use` makes an element inherit from the referencing element.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::pathdata::PData;
use gfx::xform::Matrix;
use gfx::Surface;
use rt::sync::SpinLock;

use crate::css;
use crate::values::{self, Len, Par};
use crate::xml::{self, Item, NONE};

/// the element kinds the renderer knows
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Svg,
    G,
    Defs,
    Use,
    Symbol,
    Switch,
    A,
    Path,
    Rect,
    Circle,
    Ellipse,
    Line,
    Polyline,
    Polygon,
    Text,
    Tspan,
    Image,
    LinearGradient,
    RadialGradient,
    Stop,
    ClipPath,
    Style,
    Title,
    Other,
}

fn kind_of(name: &str) -> Kind {
    match name {
        "svg" => Kind::Svg,
        "g" => Kind::G,
        "defs" => Kind::Defs,
        "use" => Kind::Use,
        "symbol" => Kind::Symbol,
        "switch" => Kind::Switch,
        "a" => Kind::A,
        "path" => Kind::Path,
        "rect" => Kind::Rect,
        "circle" => Kind::Circle,
        "ellipse" => Kind::Ellipse,
        "line" => Kind::Line,
        "polyline" => Kind::Polyline,
        "polygon" => Kind::Polygon,
        "text" => Kind::Text,
        "tspan" | "textPath" => Kind::Tspan,
        "image" => Kind::Image,
        "linearGradient" => Kind::LinearGradient,
        "radialGradient" => Kind::RadialGradient,
        "stop" => Kind::Stop,
        "clipPath" => Kind::ClipPath,
        "style" => Kind::Style,
        "title" => Kind::Title,
        _ => Kind::Other,
    }
}

// the properties (index = id); presentation attributes of the same name set them too
pub(crate) const P_FILL: u8 = 0;
pub(crate) const P_FILL_OPACITY: u8 = 1;
pub(crate) const P_FILL_RULE: u8 = 2;
pub(crate) const P_STROKE: u8 = 3;
pub(crate) const P_STROKE_WIDTH: u8 = 4;
pub(crate) const P_STROKE_OPACITY: u8 = 5;
pub(crate) const P_LINECAP: u8 = 6;
pub(crate) const P_LINEJOIN: u8 = 7;
pub(crate) const P_MITER: u8 = 8;
pub(crate) const P_DASHARRAY: u8 = 9;
pub(crate) const P_DASHOFFSET: u8 = 10;
pub(crate) const P_OPACITY: u8 = 11;
pub(crate) const P_COLOR: u8 = 12;
pub(crate) const P_DISPLAY: u8 = 13;
pub(crate) const P_VISIBILITY: u8 = 14;
pub(crate) const P_CLIP_PATH: u8 = 15;
pub(crate) const P_CLIP_RULE: u8 = 16;
pub(crate) const P_STOP_COLOR: u8 = 17;
pub(crate) const P_STOP_OPACITY: u8 = 18;
pub(crate) const P_FONT_SIZE: u8 = 19;
pub(crate) const P_FONT_FAMILY: u8 = 20;
pub(crate) const P_FONT_WEIGHT: u8 = 21;
pub(crate) const P_FONT_STYLE: u8 = 22;
pub(crate) const P_TEXT_ANCHOR: u8 = 23;
pub(crate) const P_OVERFLOW: u8 = 24;
const PROP_NAMES: [&str; 25] = [
    "fill",
    "fill-opacity",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-opacity",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "stroke-dasharray",
    "stroke-dashoffset",
    "opacity",
    "color",
    "display",
    "visibility",
    "clip-path",
    "clip-rule",
    "stop-color",
    "stop-opacity",
    "font-size",
    "font-family",
    "font-weight",
    "font-style",
    "text-anchor",
    "overflow",
];

fn prop_id(name: &str) -> Option<u8> {
    PROP_NAMES.iter().position(|&n| n == name).map(|k| k as u8)
}

pub(crate) struct Node {
    pub kind: Kind,
    pub attrs: Vec<(String, String)>,
    /// cascaded declarations, later ones win
    pub props: Vec<(u8, String)>,
    pub kids: Vec<Item>,
    pub parent: u32,
    /// the transform attribute (gradientTransform for gradients, transform for clipPath)
    pub tf: Option<Matrix>,
    /// path data of path / polyline / polygon (built once)
    pub pd: Option<PData>,
    /// an embedded raster image (index into Svg::images)
    pub img: Option<u32>,
}

impl Node {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    /// href or xlink:href
    pub fn href(&self) -> Option<&str> {
        self.attr("href").or_else(|| self.attr("xlink:href"))
    }

    /// a length attribute
    pub fn len(&self, name: &str) -> Option<Len> {
        self.attr(name).and_then(values::length)
    }

    /// the last declaration of property p
    pub fn prop(&self, p: u8) -> Option<&str> {
        self.props.iter().rev().find(|(k, _)| *k == p).map(|(_, v)| v.as_str())
    }

    /// the element's own text (title, style)
    pub fn text(&self) -> String {
        let mut s = String::new();
        for k in &self.kids {
            if let Item::Text(t) = k {
                s.push_str(t);
            }
        }
        s
    }
}

/// an embedded raster image: its file bytes, decoded at the first use (None: not yet,
/// Some(None): undecodable)
pub(crate) struct Img {
    data: Vec<u8>,
    sf: SpinLock<Option<Option<Surface>>>,
}

/// a parsed SVG document (immutable; render it as often as needed)
pub struct Svg {
    pub(crate) nodes: Vec<Node>,
    pub(crate) ids: BTreeMap<String, u32>,
    pub(crate) images: Vec<Img>,
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) view_box: Option<[f64; 4]>,
    pub(crate) par: Par,
    title: String,
    truncated: bool,
}

const DEFAULT_W: f64 = 300.0;
const DEFAULT_H: f64 = 150.0;
const SIZE_MAX: f64 = 1.0e6;
const CASCADE_STEPS: u32 = 50_000_000;

// an absolute length of the root (em at 16 px; % and invalid values: None)
fn abs_len(v: Option<&str>) -> Option<f64> {
    let l = values::length(v?)?;
    let px = match l.u {
        values::Unit::Px => l.v,
        values::Unit::Em => l.v * 16.0,
        values::Unit::Ex => l.v * 8.0,
        values::Unit::Pct => return None,
    };
    if px > 0.0 { Some(px.min(SIZE_MAX)) } else { None }
}

impl Svg {
    /// parse an SVG document (UTF-8 XML; gzip data (.svgz) is inflated first):
    /// Err(ERR_HDR) when it is no SVG, ERR_LIMIT beyond the size limits
    pub fn parse(data: &[u8]) -> Result<Svg, i32> {
        let inflated;
        let data = if data.len() >= 3 && data[0] == 0x1F && data[1] == 0x8B && data[2] == 8 {
            inflated = crate::gunzip(data)?;
            &inflated[..]
        } else {
            data
        };
        let dom = xml::parse(data)?;
        Svg::from_dom(dom)
    }

    fn from_dom(mut dom: xml::Dom) -> Result<Svg, i32> {
        if dom.els.first().map(|e| e.name.as_str()) != Some("svg") {
            return Err(img_core::ERR_HDR);
        }
        // the style sheets
        let mut rules: Vec<css::Rule> = Vec::new();
        let mut order = 0;
        for e in &dom.els {
            if e.name == "style" && e.attr("type").is_none_or(|t| t.trim().is_empty() || t.trim().eq_ignore_ascii_case("text/css")) {
                let mut s = String::new();
                for k in &e.kids {
                    if let Item::Text(t) = k {
                        s.push_str(t);
                    }
                }
                css::sheet(&s, &mut order, &mut rules);
            }
        }
        rules.sort_by(|a, b| (a.sel.spec, a.order).cmp(&(b.sel.spec, b.order)));
        // the cascade
        let mut steps = CASCADE_STEPS;
        let mut props_all: Vec<Vec<(u8, String)>> = Vec::with_capacity(dom.els.len());
        for k in 0..dom.els.len() {
            let e = &dom.els[k];
            let mut props: Vec<(u8, String)> = Vec::new();
            for (n, v) in &e.attrs {
                if let Some(p) = prop_id(n) {
                    props.push((p, v.clone()));
                }
            }
            for r in &rules {
                if steps == 0 {
                    break;
                }
                if r.sel.matches(&dom, k as u32, &mut steps) {
                    for (n, v) in &r.decls {
                        if let Some(p) = prop_id(n) {
                            props.push((p, v.clone()));
                        }
                    }
                }
            }
            if let Some(st) = e.attr("style") {
                for (n, v) in css::decls(st) {
                    if let Some(p) = prop_id(&n) {
                        props.push((p, v));
                    }
                }
            }
            // keep the last declaration of each property
            let mut out: Vec<(u8, String)> = Vec::with_capacity(props.len());
            for (p, v) in props.into_iter().rev() {
                if !out.iter().any(|(q, _)| *q == p) {
                    out.push((p, v));
                }
            }
            out.reverse();
            props_all.push(out);
        }
        // the nodes
        let mut nodes = Vec::with_capacity(dom.els.len());
        let mut ids = BTreeMap::new();
        let mut images = Vec::new();
        for (k, (e, props)) in dom.els.drain(..).zip(props_all).enumerate() {
            let kind = kind_of(&e.name);
            let tf_attr = match kind {
                Kind::LinearGradient | Kind::RadialGradient => "gradientTransform",
                _ => "transform",
            };
            let tf = e.attr(tf_attr).and_then(values::transform);
            let pd = match kind {
                Kind::Path => e.attr("d").map(crate::pathd::parse), // up to the first error
                Kind::Polyline | Kind::Polygon => e.attr("points").map(|p| points(p, kind == Kind::Polygon)),
                _ => None,
            };
            let img = if kind == Kind::Image {
                let href = e.attr("href").or_else(|| e.attr("xlink:href"));
                match href.and_then(crate::data_uri_bytes) {
                    Some(data) => {
                        images.push(Img { data, sf: SpinLock::new(None) });
                        Some(images.len() as u32 - 1)
                    }
                    None => None,
                }
            } else {
                None
            };
            if let Some(id) = e.attr("id") {
                ids.entry(String::from(id)).or_insert(k as u32);
            }
            nodes.push(Node { kind, attrs: e.attrs, props, kids: e.kids, parent: e.parent, tf, pd, img });
        }
        // the root's size and viewBox
        let root = &nodes[0];
        let view_box = root.attr("viewBox").and_then(values::view_box);
        let par = root.attr("preserveAspectRatio").map(values::par).unwrap_or_default();
        let (mut w, mut h) = (abs_len(root.attr("width")), abs_len(root.attr("height")));
        if let Some(vb) = view_box {
            match (w, h) {
                (None, None) => {
                    w = Some(vb[2]);
                    h = Some(vb[3]);
                }
                (Some(x), None) => h = Some(x * vb[3] / vb[2]),
                (None, Some(y)) => w = Some(y * vb[2] / vb[3]),
                _ => {}
            }
        }
        let width = w.unwrap_or(DEFAULT_W).clamp(1.0e-3, SIZE_MAX);
        let height = h.unwrap_or(DEFAULT_H).clamp(1.0e-3, SIZE_MAX);
        // the title: the root's first <title>
        let mut title = String::new();
        for kid in &root.kids {
            if let Item::El(c) = kid {
                if let Some(n) = nodes.get(*c as usize) {
                    if n.kind == Kind::Title {
                        title = collapse(&n.text());
                        break;
                    }
                }
            }
        }
        Ok(Svg { nodes, ids, images, width, height, view_box, par, title, truncated: dom.truncated })
    }

    /// the intrinsic size in px (CSS px = DIP): width / height of the root, else its viewBox,
    /// else 300 x 150
    pub fn size(&self) -> (f64, f64) {
        (self.width, self.height)
    }

    /// the root's viewBox (x, y, w, h), if it has one
    pub fn view_box(&self) -> Option<[f64; 4]> {
        self.view_box
    }

    /// the text of the root's <title> (blanks collapsed; empty without one)
    pub fn title(&self) -> &str {
        &self.title
    }

    /// the file ended early or a limit dropped elements (what was read is drawn)
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// the number of elements
    pub fn element_count(&self) -> usize {
        self.nodes.len()
    }

    /// f(the decoded embedded image i) (decoded at the first call)
    pub(crate) fn with_image<R>(&self, i: u32, f: impl FnOnce(&Surface) -> R) -> Option<R> {
        let img = self.images.get(i as usize)?;
        let mut g = img.sf.lock();
        if g.is_none() {
            *g = Some(crate::decode_image(&img.data));
        }
        match &*g {
            Some(Some(sf)) => Some(f(sf)),
            _ => None,
        }
    }

    pub(crate) fn node(&self, k: u32) -> Option<&Node> {
        self.nodes.get(k as usize)
    }

    /// the element with id (a "#id" fragment or a bare id)
    pub(crate) fn by_id(&self, id: &str) -> Option<u32> {
        let id = id.trim();
        let id = id.strip_prefix('#').unwrap_or(id);
        self.ids.get(id).copied()
    }

    /// the element an href points to ("#id" only: other documents are not loaded)
    pub(crate) fn href_target(&self, n: &Node) -> Option<u32> {
        let h = n.href()?.trim();
        if !h.starts_with('#') {
            return None;
        }
        self.by_id(h)
    }

    #[allow(dead_code)]
    pub(crate) fn parent_of(&self, k: u32) -> u32 {
        self.node(k).map_or(NONE, |n| n.parent)
    }
}

/// blanks collapsed to one space, trimmed
pub(crate) fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for w in s.split_ascii_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    out
}

// points="x,y x,y ..." as path data (an odd number drops the last value)
fn points(v: &str, close: bool) -> PData {
    let s = v.as_bytes();
    let mut pd = PData::new();
    let mut i = 0;
    let mut first = true;
    loop {
        values::sep(s, &mut i);
        let Some(x) = values::number(s, &mut i) else { break };
        values::sep(s, &mut i);
        let Some(y) = values::number(s, &mut i) else { break };
        if first {
            pd.move_to(x, y);
            first = false;
        } else {
            pd.line_to(x, y);
        }
    }
    if close && !first {
        pd.close();
    }
    pd.err = 0;
    pd
}
