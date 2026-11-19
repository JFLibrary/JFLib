// svgdump -- a page's display list as SVG in the coordinates of Visio's SVG export, for the
// comparison with svgcmp.py (PLAN.md "Testen": geometry and position within a tolerance)
//
//   svg(&doc, &drawing) -> String
//
// Points (1/72 inch), y down, the origin at the page's top left corner; one
// <g v:groupContext="foregroundPage"> (and "backgroundPage" for background pages) titled with
// the page's name, a <g v:mID="ID" v:groupContext="shape"> per run of a shape's items, every
// path in page coordinates; line ends carry data-jf="arrow" (svgcmp leaves them out as Visio's
// markers are left out); shadows, text backgrounds and underlines are not written.  Not an export
// of JFDiagrammer (that is JD12's): only what the comparison reads.

use alloc::format;
use alloc::string::String;

use diagram_core::geometry::{Path, Seg};
use diagram_core::render::{Drawing, ItemKind, Paint, Role};
use diagram_core::{Document, Owner};

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn color(c: u32) -> String {
    format!("#{:06x}", c & 0xFF_FFFF)
}

fn alpha(c: u32) -> f64 {
    (c >> 24) as f64 / 255.0
}

pub fn svg(doc: &Document, d: &Drawing) -> String {
    let (w, h) = (d.width * 72.0, d.height * 72.0);
    let pt = |p: [f64; 2]| format!("{:.3} {:.3}", p[0] * 72.0, h - p[1] * 72.0);
    let path_d = |p: &Path| {
        let mut s = String::new();
        for sub in &p.subs {
            s.push_str(&format!("M{}", pt(sub.start)));
            for seg in &sub.segs {
                match *seg {
                    Seg::Line(q) => s.push_str(&format!(" L{}", pt(q))),
                    Seg::Cubic(a, b, q) => s.push_str(&format!(" C{} {} {}", pt(a), pt(b), pt(q))),
                }
            }
            if sub.closed {
                s.push_str(" Z");
            }
        }
        s
    };
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:v=\"http://schemas.microsoft.com/visio/2003/SVGExtensions/\" width=\"{:.4}in\" height=\"{:.4}in\" viewBox=\"0 0 {w:.3} {h:.3}\">\n",
        d.width, d.height
    );
    let page_of = |o: Owner| match o {
        Owner::Page(p) => Some(p),
        Owner::Master(_) => None,
    };
    let mut cur_page = None;
    let mut cur_shape = None;
    for it in &d.items {
        let pg = page_of(it.shape.owner);
        if pg != cur_page {
            if cur_shape.is_some() {
                out.push_str("</g>\n");
                cur_shape = None;
            }
            if cur_page.is_some() {
                out.push_str("</g>\n");
            }
            cur_page = pg;
            let (ctx, name) = match pg.and_then(|p| doc.page(p)) {
                Some(p) => {
                    let n = p.header.name.or(p.header.name_u).map(|s| String::from(doc.str(s))).unwrap_or_default();
                    (if p.header.background { "backgroundPage" } else { "foregroundPage" }, n)
                }
                None => ("foregroundPage", String::new()),
            };
            out.push_str(&format!("<g v:groupContext=\"{ctx}\"><title>{}</title>\n", esc(&name)));
        }
        if cur_shape != Some(it.shape) {
            if cur_shape.is_some() {
                out.push_str("</g>\n");
            }
            cur_shape = Some(it.shape);
            let id = doc.shape(it.shape).map_or(0, |s| s.id());
            out.push_str(&format!("<g v:mID=\"{id}\" v:groupContext=\"shape\">"));
        }
        // shadows, text backgrounds and underlines are no geometry to compare (Visio puts its
        // shadows in groups of their own, which svgcmp leaves out)
        if it.role == Role::Shadow || (it.role == Role::Text && !matches!(it.kind, ItemKind::Text(_))) {
            continue;
        }
        let mark = if it.role == Role::Arrow { " data-jf=\"arrow\"" } else { "" };
        match &it.kind {
            ItemKind::Fill { path, paint } => {
                let (fill, op) = match paint {
                    Paint::Solid(c) => (color(*c), alpha(*c)),
                    Paint::Hatch { color: c, .. } => (color(*c), alpha(*c)),
                    Paint::Gradient(g) => g.stops.first().map_or((String::from("#000000"), 1.0), |s| (color(s.1), alpha(s.1))),
                };
                out.push_str(&format!("<path d=\"{}\" fill=\"{fill}\" fill-opacity=\"{op:.3}\" fill-rule=\"evenodd\" stroke=\"none\"{mark}/>", path_d(path)));
            }
            ItemKind::Stroke { path, stroke } => {
                let dash = if stroke.dash.is_empty() { String::new() } else { format!(" stroke-dasharray=\"{}\"", stroke.dash.iter().map(|v| format!("{:.3}", v * 72.0)).collect::<alloc::vec::Vec<_>>().join(" ")) };
                out.push_str(&format!(
                    "<path d=\"{}\" fill=\"none\" stroke=\"{}\" stroke-opacity=\"{:.3}\" stroke-width=\"{:.3}\"{dash}{mark}/>",
                    path_d(path),
                    color(stroke.color),
                    alpha(stroke.color),
                    (stroke.width * 72.0).max(0.24)
                ));
            }
            ItemKind::Text(t) => {
                for run in &t.runs {
                    let p = t.xform.apply(run.at);
                    out.push_str(&format!("<text x=\"{:.3}\" y=\"{:.3}\" font-family=\"{}\" font-size=\"{:.3}\" fill=\"{}\">{}</text>", p[0] * 72.0, h - p[1] * 72.0, esc(&run.family), run.size * 72.0, color(run.color), esc(&run.text)));
                }
            }
            // (Visio exports a picture as <image>, which the comparison does not read)
            ItemKind::Image(_) => {}
        }
    }
    if cur_shape.is_some() {
        out.push_str("</g>\n");
    }
    if cur_page.is_some() {
        out.push_str("</g>\n");
    }
    out.push_str("</svg>\n");
    out
}
