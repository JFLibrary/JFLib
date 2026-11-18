// svg -- a page as an SVG document (EX-03)
//
//   write(doc, drawing, &SvgOptions) -> String     the display list of one page (PageView::
//                                                  drawing: measured with the fonts drawn)
//
// The document: width and height in inches, the viewBox in points (1/72 inch) with y down;
// the page's colour as a rectangle (unless transparent); per top-level shape a group
// (id "shape<ID>", data-name, optionally its shape data as data-prop-* attributes), the
// shape's hyperlinks as <a> around what the linked shape draws.  Items: fills as paths with
// fill-rule evenodd (solid, a linear or radial gradient in user space, a hatch as one pattern
// per line family: lines through the page's origin, a pixel at 96 DPI wide), lines as paths
// with width, dashes, caps and joins (a hairline stays one pixel: non-scaling-stroke), texts
// as <text> per run with its font family (and a generic family after it), size, weight, style,
// colour and the measured width as textLength (lengthAdjust spacingAndGlyphs, so a reader with
// another font keeps the layout), pictures as <image> with the file's own PNG / JPEG / GIF /
// BMP bytes (data: URI) cut at the shape's box, metafiles as the screen's placeholder.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

use diagram_core::geometry::{Path, Seg, Xform};
use diagram_core::render::{Cap, Drawing, Gradient, ImageItem, Item, ItemKind, Join, Paint, Stroke, TextItem};
use diagram_core::{Document, ShapeRef, SheetRef};

use super::{links, Link, LinkTarget, HATCH_WIDTH};
use crate::fonts::{class_of, Class};

/// what write puts into the SVG
#[derive(Clone, Debug, PartialEq)]
pub struct SvgOptions {
    /// the page's colour under the shapes (false: transparent)
    pub background: bool,
    /// the shapes' hyperlinks as <a> elements (links to other pages as "#page-<n>")
    pub links: bool,
    /// the shapes' shape data as data-prop-<name> attributes of their groups
    pub shape_data: bool,
    /// the document's <title> (empty: none)
    pub title: String,
}

impl Default for SvgOptions {
    fn default() -> SvgOptions {
        SvgOptions { background: true, links: true, shape_data: false, title: String::new() }
    }
}

/// the page of `drawing` as an SVG document
pub fn write(doc: &Document, drawing: &Drawing, opts: &SvgOptions) -> String {
    let mut w = Svg { o: String::new(), h: drawing.height, ids: 0 };
    let (pw, ph) = (drawing.width, drawing.height);
    w.o.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"no\"?>\n");
    w.o.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" version=\"1.1\"");
    let _ = write!(w.o, " width=\"{}in\" height=\"{}in\" viewBox=\"0 0 {} {}\">\n", n(pw), n(ph), n(pw * 72.0), n(ph * 72.0));
    if !opts.title.is_empty() {
        w.o.push_str("<title>");
        esc(&mut w.o, &opts.title, false);
        w.o.push_str("</title>\n");
    }
    if opts.background {
        let _ = writeln!(w.o, "<rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"{}\"/>", n(pw * 72.0), n(ph * 72.0), hex(drawing.color));
    }
    let links = if opts.links { links(doc, drawing) } else { Vec::new() };
    let pages = super::foreground_pages(doc);
    let items = &drawing.items;
    let mut i = 0;
    while i < items.len() {
        let top = items[i].top;
        let mut j = i;
        while j < items.len() && items[j].top == top {
            j += 1;
        }
        w.group_open(doc, top, opts.shape_data);
        // runs of items under the same linked shape
        let mut k = i;
        while k < j {
            let link = link_of(doc, &links, items[k].shape, top);
            let mut m = k;
            while m < j && link_of(doc, &links, items[m].shape, top) == link {
                m += 1;
            }
            if let Some(l) = link {
                w.link_open(&links[l], &pages);
            }
            for it in &items[k..m] {
                w.item(doc, it);
            }
            if link.is_some() {
                w.o.push_str("</a>\n");
            }
            k = m;
        }
        w.o.push_str("</g>\n");
        i = j;
    }
    w.o.push_str("</svg>\n");
    w.o
}

/// the link of the nearest shape from `shape` up to `top` that has one
fn link_of(doc: &Document, links: &[Link], shape: ShapeRef, top: ShapeRef) -> Option<usize> {
    if links.is_empty() {
        return None;
    }
    let mut cur = Some(shape);
    let mut depth = 0;
    while let Some(s) = cur {
        if let Some(k) = links.iter().position(|l| l.shape == s) {
            return Some(k);
        }
        if s == top || depth > 64 {
            break;
        }
        depth += 1;
        cur = doc.shape(s).and_then(|x| x.parent());
    }
    None
}

struct Svg {
    o: String,
    /// the page's height (inches): y is flipped
    h: f64,
    /// the next id of a definition
    ids: u32,
}

impl Svg {
    /// paper inches (y up) -> SVG points (y down)
    fn p(&self, p: [f64; 2]) -> [f64; 2] {
        [p[0] * 72.0, (self.h - p[1]) * 72.0]
    }

    /// the map paper inches -> SVG points
    fn page_map(&self) -> Xform {
        Xform { a: 72.0, b: 0.0, c: 0.0, d: -72.0, e: 0.0, f: self.h * 72.0 }
    }

    fn id(&mut self, prefix: &str) -> String {
        self.ids += 1;
        alloc::format!("{prefix}{}", self.ids)
    }

    fn group_open(&mut self, doc: &Document, top: ShapeRef, data: bool) {
        let Some(s) = doc.shape(top) else {
            self.o.push_str("<g>\n");
            return;
        };
        let _ = write!(self.o, "<g id=\"shape{}\"", s.id());
        let name = s.name();
        if !name.is_empty() {
            self.o.push_str(" data-name=\"");
            esc(&mut self.o, &name, true);
            self.o.push('"');
        }
        if data {
            let mut seen: Vec<String> = Vec::new();
            for row in doc.shape_data(SheetRef::Shape(top)) {
                let Some(key) = row.name() else { continue };
                let attr = data_name(key);
                if attr.is_empty() || seen.contains(&attr) {
                    continue;
                }
                let value = match row.get("Value").value() {
                    diagram_core::Value::Text(t) => String::from(doc.str(t)),
                    diagram_core::Value::Num(v) => num_text(v),
                    _ => continue,
                };
                let _ = write!(self.o, " data-prop-{attr}=\"");
                esc(&mut self.o, &value, true);
                self.o.push('"');
                seen.push(attr);
            }
        }
        self.o.push_str(">\n");
    }

    fn link_open(&mut self, l: &Link, pages: &[diagram_core::PageRef]) {
        let href = match &l.target {
            LinkTarget::Uri(u) => u.clone(),
            LinkTarget::Page(p) => alloc::format!("#page-{}", pages.iter().position(|x| x == p).map_or(0, |k| k + 1)),
        };
        self.o.push_str("<a xlink:href=\"");
        esc(&mut self.o, &href, true);
        self.o.push('"');
        if !l.description.is_empty() {
            self.o.push_str(" xlink:title=\"");
            esc(&mut self.o, &l.description, true);
            self.o.push('"');
        }
        self.o.push_str(">\n");
    }

    fn item(&mut self, doc: &Document, it: &Item) {
        match &it.kind {
            ItemKind::Fill { path, paint } => self.fill(path, paint),
            ItemKind::Stroke { path, stroke } => self.stroke(path, stroke),
            ItemKind::Text(t) => self.text(t),
            ItemKind::Image(img) => self.image(doc, img),
        }
    }

    /// the d attribute of a path; a fill closes every sub-path (as the screen fills them)
    fn d(&mut self, path: &Path, fill: bool) -> bool {
        let start = self.o.len();
        self.o.push_str(" d=\"");
        let mut any = false;
        for sub in &path.subs {
            if sub.segs.is_empty() {
                continue;
            }
            any = true;
            let p = self.p(sub.start);
            let _ = write!(self.o, "M{} {}", n(p[0]), n(p[1]));
            for s in &sub.segs {
                match *s {
                    Seg::Line(q) => {
                        let q = self.p(q);
                        let _ = write!(self.o, "L{} {}", n(q[0]), n(q[1]));
                    }
                    Seg::Cubic(a, b, q) => {
                        let (a, b, q) = (self.p(a), self.p(b), self.p(q));
                        let _ = write!(self.o, "C{} {} {} {} {} {}", n(a[0]), n(a[1]), n(b[0]), n(b[1]), n(q[0]), n(q[1]));
                    }
                }
            }
            if sub.closed || fill {
                self.o.push('Z');
            }
        }
        self.o.push('"');
        if !any {
            self.o.truncate(start);
        }
        any
    }

    fn fill(&mut self, path: &Path, paint: &Paint) {
        match paint {
            Paint::Solid(c) => {
                let mark = self.o.len();
                self.o.push_str("<path");
                if !self.d(path, true) {
                    self.o.truncate(mark);
                    return;
                }
                let _ = write!(self.o, " fill=\"{}\"", hex(*c));
                opacity(&mut self.o, "fill-opacity", *c);
                self.o.push_str(" fill-rule=\"evenodd\"/>\n");
            }
            Paint::Gradient(g) => {
                let id = self.gradient(g);
                let mark = self.o.len();
                self.o.push_str("<path");
                if !self.d(path, true) {
                    self.o.truncate(mark);
                    return;
                }
                let _ = writeln!(self.o, " fill=\"url(#{id})\" fill-rule=\"evenodd\"/>");
            }
            Paint::Hatch { color, lines } => {
                for &(angle, spacing) in lines {
                    if !(spacing.is_finite() && spacing > 1e-4 && angle.is_finite()) {
                        continue;
                    }
                    let id = self.hatch(*color, angle, spacing);
                    let mark = self.o.len();
                    self.o.push_str("<path");
                    if !self.d(path, true) {
                        self.o.truncate(mark);
                        return;
                    }
                    let _ = writeln!(self.o, " fill=\"url(#{id})\" fill-rule=\"evenodd\"/>");
                }
            }
        }
    }

    fn gradient(&mut self, g: &Gradient) -> String {
        let id = self.id("grad");
        let (a, b) = (self.p(g.from), self.p(g.to));
        if g.radial {
            let r = dist(a, b);
            let _ = write!(self.o, "<radialGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" cx=\"{}\" cy=\"{}\" r=\"{}\">", n(a[0]), n(a[1]), n(r.max(1e-3)));
        } else {
            let _ = write!(self.o, "<linearGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\">", n(a[0]), n(a[1]), n(b[0]), n(b[1]));
        }
        for &(t, c) in &g.stops {
            let _ = write!(self.o, "<stop offset=\"{}\" stop-color=\"{}\"", n(t.clamp(0.0, 1.0)), hex(c));
            opacity(&mut self.o, "stop-opacity", c);
            self.o.push_str("/>");
        }
        self.o.push_str(if g.radial { "</radialGradient>\n" } else { "</linearGradient>\n" });
        id
    }

    /// a pattern of lines at `angle` (radians counter-clockwise, y up) `spacing` inches apart
    /// through the page's origin
    fn hatch(&mut self, color: u32, angle: f64, spacing: f64) -> String {
        let id = self.id("hatch");
        let s = spacing * 72.0;
        let half = (HATCH_WIDTH * 72.0 * 0.5).min(s * 0.5);
        // pattern space: x along the lines, y across; y down in SVG turns the angle clockwise
        let deg = -angle * 180.0 / core::f64::consts::PI;
        let _ = write!(
            self.o,
            "<pattern id=\"{id}\" patternUnits=\"userSpaceOnUse\" x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" patternTransform=\"translate(0 {}) rotate({})\">",
            n(s),
            n(s),
            n(self.h * 72.0),
            n(deg)
        );
        let _ = write!(self.o, "<rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"{}\"", n(s), n(half), hex(color));
        opacity(&mut self.o, "fill-opacity", color);
        self.o.push_str("/>");
        let _ = write!(self.o, "<rect x=\"0\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\"", n(s - half), n(s), n(half), hex(color));
        opacity(&mut self.o, "fill-opacity", color);
        self.o.push_str("/></pattern>\n");
        id
    }

    fn stroke(&mut self, path: &Path, st: &Stroke) {
        let mark = self.o.len();
        self.o.push_str("<path");
        if !self.d(path, false) {
            self.o.truncate(mark);
            return;
        }
        let _ = write!(self.o, " fill=\"none\" stroke=\"{}\"", hex(st.color));
        opacity(&mut self.o, "stroke-opacity", st.color);
        if st.width > 0.0 && st.width.is_finite() {
            let _ = write!(self.o, " stroke-width=\"{}\"", n(st.width * 72.0));
        } else {
            self.o.push_str(" stroke-width=\"1\" vector-effect=\"non-scaling-stroke\"");
        }
        self.o.push_str(match st.cap {
            Cap::Round => " stroke-linecap=\"round\"",
            Cap::Butt => "",
            Cap::Square => " stroke-linecap=\"square\"",
        });
        self.o.push_str(match st.join {
            Join::Round => " stroke-linejoin=\"round\"",
            Join::Miter => " stroke-miterlimit=\"4\"",
            Join::Bevel => " stroke-linejoin=\"bevel\"",
        });
        if !st.dash.is_empty() && st.dash.iter().all(|v| v.is_finite() && *v >= 0.0) && st.dash.iter().sum::<f64>() > 0.0 {
            self.o.push_str(" stroke-dasharray=\"");
            for (k, v) in st.dash.iter().enumerate() {
                if k > 0 {
                    self.o.push(' ');
                }
                self.o.push_str(&n(v * 72.0));
            }
            self.o.push('"');
        }
        self.o.push_str("/>\n");
    }

    fn text(&mut self, t: &TextItem) {
        for run in &t.runs {
            if !super::run_visible(&run.text, run.size) {
                continue;
            }
            // run space: points, x along the baseline, y down -> frame (inches, y up) -> page -> SVG
            let local = Xform { a: 1.0 / 72.0, b: 0.0, c: 0.0, d: -1.0 / 72.0, e: run.at[0], f: run.at[1] };
            let m = local.then(&t.xform).then(&self.page_map());
            let _ = write!(self.o, "<text xml:space=\"preserve\" transform=\"matrix({} {} {} {} {} {})\"", n6(m.a), n6(m.b), n6(m.c), n6(m.d), n(m.e), n(m.f));
            self.o.push_str(" font-family=\"");
            let fam = run.family.replace(['\'', '"', ';'], "");
            if !fam.is_empty() {
                self.o.push('\'');
                esc(&mut self.o, &fam, true);
                self.o.push_str("', ");
            }
            self.o.push_str(match class_of(&fam) {
                Class::Serif => "serif",
                Class::Mono => "monospace",
                Class::Sans => "sans-serif",
            });
            let _ = write!(self.o, "\" font-size=\"{}\"", n(run.size * 72.0));
            if run.bold {
                self.o.push_str(" font-weight=\"bold\"");
            }
            if run.italic {
                self.o.push_str(" font-style=\"italic\"");
            }
            let _ = write!(self.o, " fill=\"{}\"", hex(run.color));
            opacity(&mut self.o, "fill-opacity", run.color);
            if run.spacing != 0.0 && run.spacing.is_finite() {
                let _ = write!(self.o, " letter-spacing=\"{}\"", n(run.spacing * 72.0));
            }
            if run.width > 0.0 && run.width.is_finite() {
                let _ = write!(self.o, " textLength=\"{}\" lengthAdjust=\"spacingAndGlyphs\"", n(run.width * 72.0));
            }
            self.o.push('>');
            esc(&mut self.o, &run.text, false);
            self.o.push_str("</text>\n");
        }
    }

    fn image(&mut self, doc: &Document, img: &ImageItem) {
        let clip = self.id("clip");
        let c: Vec<[f64; 2]> = img.clip.iter().map(|p| self.p(*p)).collect();
        let _ = write!(self.o, "<clipPath id=\"{clip}\"><path d=\"M{} {}L{} {}L{} {}L{} {}Z\"/></clipPath>\n", n(c[0][0]), n(c[0][1]), n(c[1][0]), n(c[1][1]), n(c[2][0]), n(c[2][1]), n(c[3][0]), n(c[3][1]));
        let bytes = crate::image::bytes(doc, &img.source);
        let mime = bytes.as_deref().and_then(mime_of);
        // the picture's unit square, row 0 at the top (SVG) -> the item's square (y up) -> page -> SVG
        let flip = Xform { a: 1.0, b: 0.0, c: 0.0, d: -1.0, e: 0.0, f: 1.0 };
        let m = flip.then(&img.xform).then(&self.page_map());
        match (bytes, mime) {
            (Some(b), Some(mime)) => {
                let _ = write!(self.o, "<g clip-path=\"url(#{clip})\"><image x=\"0\" y=\"0\" width=\"1\" height=\"1\" preserveAspectRatio=\"none\" transform=\"matrix({} {} {} {} {} {})\" xlink:href=\"data:{mime};base64,", n6(m.a), n6(m.b), n6(m.c), n6(m.d), n(m.e), n(m.f));
                base64(&mut self.o, &b);
                self.o.push_str("\"/></g>\n");
            }
            _ => {
                // the screen's placeholder (direct.rs): a light grey box with a frame and a cross
                let q: Vec<[f64; 2]> = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].iter().map(|p| self.p(img.xform.apply(*p))).collect();
                let _ = writeln!(
                    self.o,
                    "<g clip-path=\"url(#{clip})\"><path d=\"M{} {}L{} {}L{} {}L{} {}Z\" fill=\"#e4e6ea\" stroke=\"#9aa0a8\" stroke-width=\"1\" vector-effect=\"non-scaling-stroke\"/><path d=\"M{} {}L{} {}M{} {}L{} {}\" stroke=\"#9aa0a8\" stroke-width=\"1\" vector-effect=\"non-scaling-stroke\"/></g>",
                    n(q[0][0]), n(q[0][1]), n(q[1][0]), n(q[1][1]), n(q[2][0]), n(q[2][1]), n(q[3][0]), n(q[3][1]),
                    n(q[0][0]), n(q[0][1]), n(q[2][0]), n(q[2][1]), n(q[1][0]), n(q[1][1]), n(q[3][0]), n(q[3][1])
                );
            }
        }
    }
}

/// the MIME type of picture bytes SVG readers show
fn mime_of(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if b.starts_with(b"GIF8") {
        Some("image/gif")
    } else if b.starts_with(b"BM") {
        Some("image/bmp")
    } else {
        None
    }
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    crate::sqrt((a[0] - b[0]) * (a[0] - b[0]) + (a[1] - b[1]) * (a[1] - b[1]))
}

/// a data-* attribute name: lower case letters, digits, '-', '_', '.'
fn data_name(key: &str) -> String {
    let mut s = String::new();
    for c in key.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-' {
            s.push(c);
        } else if !s.ends_with('-') {
            s.push('-');
        }
    }
    String::from(s.trim_matches('-'))
}

/// a number as shape data shows it (no trailing zeros)
fn num_text(v: f64) -> String {
    if !v.is_finite() {
        return String::new();
    }
    if v == crate::floor(v) && v.abs() < 1e15 {
        return alloc::format!("{}", v as i64);
    }
    let mut s = alloc::format!("{v:.6}");
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    s
}

/// #rrggbb of 0xAARRGGBB
fn hex(c: u32) -> String {
    alloc::format!("#{:06x}", c & 0xFF_FFFF)
}

/// an opacity attribute when the colour is not opaque
fn opacity(o: &mut String, name: &str, c: u32) {
    let a = c >> 24;
    if a < 255 {
        let _ = write!(o, " {name}=\"{}\"", n(a as f64 / 255.0));
    }
}

/// a number with at most 3 decimals, no trailing zeros
pub(crate) fn n(v: f64) -> String {
    fixed(v, 1000.0, 3)
}

/// with 6 decimals (matrix factors)
fn n6(v: f64) -> String {
    fixed(v, 1_000_000.0, 6)
}

fn fixed(v: f64, scale: f64, places: usize) -> String {
    if !v.is_finite() || v.abs() > 1e12 {
        return String::from("0");
    }
    let i = crate::floor(v * scale + 0.5) as i64;
    if i == 0 {
        return String::from("0");
    }
    let neg = i < 0;
    let a = i.unsigned_abs();
    let p = scale as u64;
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    let _ = write!(s, "{}", a / p);
    let frac = a % p;
    if frac != 0 {
        let mut f = alloc::format!("{frac:0places$}");
        while f.ends_with('0') {
            f.pop();
        }
        s.push('.');
        s.push_str(&f);
    }
    s
}

/// XML text (attr: also quotes); characters XML 1.0 forbids are left out
fn esc(o: &mut String, s: &str, attr: bool) {
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' if attr => o.push_str("&quot;"),
            '\'' if attr => o.push_str("&apos;"),
            '\t' | '\n' | '\r' if attr => {
                let _ = write!(o, "&#{};", c as u32);
            }
            '\t' | '\n' | '\r' => o.push(c),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
            c => o.push(c),
        }
    }
}

/// RFC 4648 base64 with padding
fn base64(o: &mut String, b: &[u8]) {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    o.reserve(b.len() / 3 * 4 + 4);
    for ch in b.chunks(3) {
        let v = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
        o.push(A[(v >> 18) as usize & 63] as char);
        o.push(A[(v >> 12) as usize & 63] as char);
        o.push(if ch.len() > 1 { A[(v >> 6) as usize & 63] as char } else { '=' });
        o.push(if ch.len() > 2 { A[v as usize & 63] as char } else { '=' });
    }
}
