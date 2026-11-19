// render -- the viewer's tests (JD4): display lists of the self-built drawings (drawings.rs),
// golden images through render2d's CPU path, the local corpus as images and SVG
//
//   run(h)                      group "render": the display-list checks of every drawing, then
//                               every foreground page against tests/golden/<name>-<n>.png
//   write_golden(dir)           --write-golden DIR: the golden images anew (look at them first)
//   write_drawings(dir)         --write-drawings DIR: the drawings as .vsdx files
//   render_corpus(out, corpus)  --render-corpus DIR: the local files' pages as PNG
//                               (<stem>-<n>.png) and SVG (<stem>.p<n>.svg, for svgcmp.py)
//   corpus(h, corpus)           in group "render/corpus": every local page built and drawn
//                               (numbers only; skipped without the files)
//   fonts()                     the test fonts: Inter (sans, the NetLab style's), DejaVu Serif
//                               and DejaVu Sans Mono (doc_view's), regular and bold
//
// The images: each page fitted into 640 pixels across (the height by its aspect), on the canvas
// grey with the page in its colour.  render2d's CPU path draws the same bytes on every target;
// the comparison still allows a pixel in a thousand to differ by more than 16 per channel, so
// that a rounding of a future toolchain does not fail the suite while a moved line does.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::render::{ItemKind, Options, Paint, Role};
use diagram_core::{DocKind, Document, Owner, PageRef};
use diagram_view::image::Decoded;
use diagram_view::{Class, FontSet, PageView};
use diagram_io::vsdx;
use gfx::Surface;

use crate::drawings;
use crate::{out, H};

const WIDTH: u32 = 640;
pub(crate) const FAR: u32 = 16;
pub(crate) const PER_MILLE: f64 = 1.0;

static INTER: &[u8] = include_bytes!("../../../styles/netlab/assets/Inter-Regular.ttf");
static INTER_BOLD: &[u8] = include_bytes!("../../../styles/netlab/assets/Inter-SemiBold.ttf");
static SERIF: &[u8] = include_bytes!("../../../lib/doc_view/assets/DejaVuSerif.ttf");
static SERIF_BOLD: &[u8] = include_bytes!("../../../lib/doc_view/assets/DejaVuSerif-Bold.ttf");
static MONO: &[u8] = include_bytes!("../../../lib/doc_view/assets/DejaVuSansMono.ttf");
static MONO_BOLD: &[u8] = include_bytes!("../../../lib/doc_view/assets/DejaVuSansMono-Bold.ttf");

/// the fonts of the tests (no installed fonts: the same on every machine)
pub fn fonts() -> FontSet {
    let mut f = FontSet::new();
    f.add(Class::Sans, false, false, INTER);
    f.add(Class::Sans, true, false, INTER_BOLD);
    f.add(Class::Serif, false, false, SERIF);
    f.add(Class::Serif, true, false, SERIF_BOLD);
    f.add(Class::Mono, false, false, MONO);
    f.add(Class::Mono, true, false, MONO_BOLD);
    f
}

fn load(bytes: &[u8]) -> Option<Document> {
    let mut lim = vsdx::Limits::VSDX;
    lim.zip.ratio = 2000;
    vsdx::read(bytes, &lim).ok().map(|l| l.doc)
}

/// the folder above the current one that ends in `rel`
pub fn find_up(rel: &str) -> Option<String> {
    let mut dir = plat::cwd().replace('\\', "/");
    for _ in 0..12 {
        let cand = format!("{dir}/{rel}");
        if plat::fs::read_dir(&cand).is_some() {
            return Some(cand);
        }
        match dir.rsplit_once('/') {
            Some((up, _)) if !up.is_empty() => dir = String::from(up),
            _ => break,
        }
    }
    None
}

fn image_size(v: &PageView) -> (u32, u32) {
    let d = v.drawing();
    let h = (WIDTH as f64 * d.height / d.width.max(1e-6) + 0.5) as u32;
    (WIDTH, h.clamp(16, 4 * WIDTH))
}

/// the PNG of an XRGB surface
pub fn png(sf: &Surface) -> Vec<u8> {
    img_png::encode(&sf.px, sf.w as usize, sf.h as usize, sf.stride as usize, false)
}

/// (pixels differing by more than FAR in a channel, the largest difference)
pub(crate) fn diff(a: &Surface, b: &Surface) -> Option<(u64, u32)> {
    if a.w != b.w || a.h != b.h {
        return None;
    }
    let mut far = 0u64;
    let mut max = 0u32;
    for y in 0..a.h as usize {
        let (ra, rb) = (&a.px[y * a.stride as usize..], &b.px[y * b.stride as usize..]);
        for x in 0..a.w as usize {
            let mut m = 0u32;
            for k in 0..3 {
                m = m.max((ra[x * 4 + k] as i32 - rb[x * 4 + k] as i32).unsigned_abs());
            }
            max = max.max(m);
            if m > FAR {
                far += 1;
            }
        }
    }
    Some((far, max))
}

fn foreground(doc: &Document) -> Vec<PageRef> {
    doc.foreground_pages().collect()
}

// ---------------------------------------------------------------- display-list checks

fn shape_items<'a>(v: &'a PageView, doc: &Document, page: PageRef, id: u32) -> Vec<&'a diagram_core::render::Item> {
    let Some(r) = doc.shape_by_id(Owner::Page(page), id) else { return Vec::new() };
    v.drawing().items.iter().filter(|i| i.shape == r).collect()
}

fn near(a: [f64; 2], b: [f64; 2], tol: f64) -> bool {
    (a[0] - b[0]).abs() <= tol && (a[1] - b[1]).abs() <= tol
}

fn first_start(items: &[&diagram_core::render::Item]) -> Option<[f64; 2]> {
    items.iter().find_map(|i| match &i.kind {
        ItemKind::Fill { path, .. } | ItemKind::Stroke { path, .. } => path.subs.first().map(|s| s.start),
        _ => None,
    })
}

fn fill_color(items: &[&diagram_core::render::Item]) -> Option<u32> {
    items.iter().find_map(|i| match (&i.kind, i.role) {
        (ItemKind::Fill { paint: Paint::Solid(c), .. }, Role::Shape) => Some(*c),
        _ => None,
    })
}

fn stroke_of<'a>(items: &[&'a diagram_core::render::Item]) -> Option<&'a diagram_core::render::Stroke> {
    items.iter().find_map(|i| match (&i.kind, i.role) {
        (ItemKind::Stroke { stroke, .. }, Role::Shape) => Some(stroke),
        _ => None,
    })
}

fn texts<'a>(items: &[&'a diagram_core::render::Item]) -> Vec<&'a diagram_core::render::TextItem> {
    items.iter().filter_map(|i| if let ItemKind::Text(t) = &i.kind { Some(&**t) } else { None }).collect()
}

fn checks(h: &mut H, name: &str, doc: &Document, page: PageRef, v: &PageView) {
    let n = |id| shape_items(v, doc, page, id);
    let deg = |d: f64| d * core::f64::consts::PI / 180.0;
    let (s30, c30) = (0.5, 0.866_025_403_784_438_6);
    match name {
        "geometry" => {
            h.check("render/geometry/all", (1..=12).all(|id| !n(id).is_empty()), format!("every shape draws something: {:?}", (1..=12).map(|id| n(id).len()).collect::<Vec<_>>()));
            h.check("render/geometry/open-not-filled", !n(10).iter().any(|i| matches!(i.kind, ItemKind::Fill { .. })), String::from("an open path with a fill colour has no fill"));
            let frame = n(9);
            let fill_subs = frame.iter().find_map(|i| if let ItemKind::Fill { path, .. } = &i.kind { Some(path.subs.len()) } else { None });
            let bb = frame.iter().map(|i| i.bbox).reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]);
            h.check("render/geometry/sections", fill_subs == Some(2) && bb.is_some_and(|b| b[0] > 0.3 && b[2] < 1.7), format!("two sections filled together, the NoShow one left out: {fill_subs:?} {bb:?}"));
            let s5 = n(5);
            let end = s5.iter().find_map(|i| if let ItemKind::Stroke { path, .. } = &i.kind { path.subs.first().map(|s| s.end()) } else { None });
            h.check("render/geometry/nurbs", first_start(&s5).is_some_and(|p| near(p, [0.3, 2.5], 1e-9)) && end.is_some_and(|p| near(p, [1.7, 3.5], 1e-9)), format!("NURBSTo from the current point to X, Y: {:?} {end:?}", first_start(&s5)));
            let inf = n(12).iter().map(|i| i.bbox[2] - i.bbox[0]).fold(0.0, f64::max);
            h.check("render/geometry/infinite", inf > 1000.0, format!("InfiniteLine drawn {inf:.0} in long"));
            let arcs = n(2).iter().find_map(|i| if let ItemKind::Fill { path, .. } = &i.kind { Some(path.subs[0].segs.iter().filter(|s| matches!(s, diagram_core::geometry::Seg::Cubic(..))).count()) } else { None });
            h.check("render/geometry/arcto", arcs == Some(4), format!("four rounded corners as curves: {arcs:?}"));
        }
        "transform" => {
            let p2 = first_start(&n(2));
            let want2 = [2.6 + (-0.5 * c30 + 0.5 * s30), 4.0 + (-0.5 * s30 - 0.5 * c30)];
            h.check("render/transform/angle", p2.is_some_and(|p| near(p, want2, 1e-9)), format!("Angle 30 about the pin: {p2:?} (want {want2:?})"));
            let p3 = first_start(&n(3));
            h.check("render/transform/flipx", p3.is_some_and(|p| near(p, [4.7, 3.5], 1e-9)), format!("FlipX about the local pin: {p3:?}"));
            let p4 = first_start(&n(4));
            h.check("render/transform/flipy", p4.is_some_and(|p| near(p, [5.3, 4.5], 1e-9)), format!("FlipY: {p4:?}"));
            let p5 = first_start(&n(5));
            h.check("render/transform/locpin", p5.is_some_and(|p| near(p, [7.0, 3.5], 1e-9)), format!("LocPin at the corner: {p5:?}"));
            let (s20, c20) = (0.342_020_143_325_668_7, 0.939_692_620_785_908_4);
            let want7 = [2.0 + (-1.0 * c20 + 0.45 * s20), 1.5 + (-1.0 * s20 - 0.45 * c20)];
            let p7 = first_start(&n(7));
            h.check("render/transform/group", p7.is_some_and(|p| near(p, want7, 1e-9)), format!("a child in a turned group: {p7:?} (want {want7:?})"));
            let one_d = n(9);
            let arrow = one_d.iter().any(|i| i.role == Role::Arrow);
            h.check("render/transform/1d", first_start(&one_d).is_some_and(|p| near(p, [4.0, 0.8], 1e-9)) && arrow, format!("1D line from Begin with a line end: {:?} {arrow}", first_start(&one_d)));
            let tx = |id| texts(&n(id)).first().map(|t| (t.xform.angle(), t.xform.det()));
            h.check("render/transform/text-flipx", tx(3).is_some_and(|(a, d)| a.abs() < 1e-9 && d > 0.0), format!("FlipX keeps the text readable: {:?}", tx(3)));
            h.check("render/transform/text-flipy", tx(4).is_some_and(|(a, d)| (a.abs() - core::f64::consts::PI).abs() < 1e-9 && d > 0.0), format!("FlipY turns it over: {:?}", tx(4)));
            h.check("render/transform/text-angle", tx(2).is_some_and(|(a, _)| (a - deg(30.0)).abs() < 1e-9), format!("text turns with the shape: {:?}", tx(2)));
        }
        "lines" => {
            let with_arrows = v.drawing().items.iter().filter(|i| i.role == Role::Arrow).map(|i| i.shape).collect::<alloc::collections::BTreeSet<_>>().len();
            h.check("render/lines/arrows", with_arrows == 22, format!("22 lines with line ends: {with_arrows}"));
            let dash = |id| stroke_of(&n(id)).map(|s| s.dash.len());
            h.check("render/lines/patterns", dash(1) == Some(0) && dash(2) == Some(2) && dash(4) == Some(4), format!("solid, dash, dash dot: {:?} {:?} {:?}", dash(1), dash(2), dash(4)));
            let cap = |id| stroke_of(&n(id)).map(|s| s.cap);
            use diagram_core::render::Cap;
            h.check("render/lines/caps", cap(20) == Some(Cap::Round) && cap(21) == Some(Cap::Butt) && cap(22) == Some(Cap::Square), format!("LineCap 0 1 2: {:?} {:?} {:?}", cap(20), cap(21), cap(22)));
            let curved = n(60).iter().any(|i| matches!(&i.kind, ItemKind::Stroke { path, .. } if path.subs[0].segs.iter().any(|s| matches!(s, diagram_core::geometry::Seg::Cubic(..)))));
            h.check("render/lines/rounding", curved, String::from("Rounding turns corners into arcs"));
            let a = stroke_of(&n(62)).map(|s| s.color >> 24);
            h.check("render/lines/transparency", a == Some(128), format!("LineColorTrans 0.5: alpha {a:?}"));
            let sizes: Vec<f64> = (50..=56).map(|id| n(id).iter().filter(|i| i.role == Role::Arrow).map(|i| i.bbox[2] - i.bbox[0]).fold(0.0, f64::max)).collect();
            h.check("render/lines/arrow-sizes", sizes.windows(2).all(|w| w[1] > w[0]), format!("line ends grow with their size: {sizes:.3?}"));
        }
        "fills" => {
            h.check("render/fills/hidden-layer", n(16).is_empty(), format!("a shape on a hidden layer is not drawn: {} items", n(16).len()));
            h.check("render/fills/two-layers", !n(18).is_empty(), String::from("a shape on a visible and a hidden layer is drawn"));
            let red = fill_color(&n(17));
            h.check("render/fills/layer-colour", red == Some(0xFFFF_0000), format!("the layer's colour: {red:08x?}"));
            let grads: Vec<bool> = v.drawing().items.iter().filter_map(|i| if let ItemKind::Fill { paint: Paint::Gradient(g), .. } = &i.kind { Some(g.radial) } else { None }).collect();
            h.check("render/fills/gradients", grads.len() == 5 && grads.iter().filter(|r| **r).count() == 2, format!("three old and two new gradients, two radial: {grads:?}"));
            let hatched = (3..=8).filter(|&id| n(id).iter().any(|i| matches!(&i.kind, ItemKind::Fill { paint: Paint::Hatch { .. }, .. }))).count();
            h.check("render/fills/hatches", hatched == 6, format!("FillPattern 2 .. 7 hatched: {hatched}"));
            let sh = n(15);
            h.check("render/fills/shadow", sh.first().is_some_and(|i| i.role == Role::Shadow), String::from("the shadow comes first"));
            let idx = fill_color(&n(19));
            h.check("render/fills/colour-index", idx == Some(0xFF2E_75B6), format!("colour index 24 from the document's table: {idx:08x?}"));
            let t = fill_color(&n(1)).map(|c| c >> 24);
            h.check("render/fills/transparency", t == Some(128), format!("FillForegndTrans 0.5: alpha {t:?}"));
        }
        "text" => {
            let runs = |id| texts(&n(id)).into_iter().flat_map(|t| t.runs.iter().cloned()).collect::<Vec<_>>();
            let r4 = runs(4);
            let find = |s: &str| r4.iter().find(|r| r.text.trim() == s);
            let ok = find("fett").is_some_and(|r| r.bold)
                && find("kursiv rot").is_some_and(|r| r.italic && r.color == 0xFFC0_0000)
                && find("gross").is_some_and(|r| (r.size - 0.25).abs() < 1e-9)
                && find("Serif durch").is_some_and(|r| r.family == "Times New Roman")
                && find("Courier").is_some_and(|r| r.family == "Courier New");
            h.check("render/text/runs", ok, format!("character rows per run: {:?}", r4.iter().map(|r| (r.text.as_str(), r.bold, r.italic, r.family.as_str())).collect::<Vec<_>>()));
            let mut ys: Vec<i64> = runs(5).iter().map(|r| (r.at[1] * 1e6) as i64).collect();
            ys.dedup();
            h.check("render/text/wrap", ys.len() >= 3, format!("a long text wraps: {} lines", ys.len()));
            let ang = |id| texts(&n(id)).first().map(|t| t.xform.angle());
            h.check("render/text/txtangle", ang(6).is_some_and(|a| (a - core::f64::consts::FRAC_PI_2).abs() < 1e-9), format!("TxtAngle: {:?}", ang(6)));
            h.check("render/text/vertical", ang(10).is_some_and(|a| (a + core::f64::consts::FRAC_PI_2).abs() < 1e-9), format!("TextDirection 1: {:?}", ang(10)));
            h.check("render/text/field", runs(11).iter().any(|r| r.text.contains("17.11.2026")), String::from("a field shows its cached text"));
            h.check("render/text/background", n(12).iter().any(|i| i.role == Role::Text && matches!(i.kind, ItemKind::Fill { .. })), String::from("TextBkgnd behind the text"));
            h.check("render/text/bullet", runs(13).iter().filter(|r| r.text == "\u{2022}").count() == 2, String::from("a bullet per paragraph"));
            let top = texts(&n(1)).first().map(|t| t.runs[0].at[1]);
            let bottom = texts(&n(3)).first().map(|t| t.runs[0].at[1]);
            h.check("render/text/valign", top.zip(bottom).is_some_and(|(t, b)| t > 0.6 && b < 0.4), format!("top and bottom aligned baselines in their 1 in blocks: {top:?} {bottom:?}"));
            h.check("render/text/substitution", v.substitutions().iter().any(|s| s == "Calibri"), format!("Calibri is substituted: {:?}", v.substitutions()));
        }
        "pictures" => {
            let imgs: Vec<&str> = v.drawing().items.iter().filter_map(|i| if let ItemKind::Image(img) = &i.kind { Some(img.format.as_str()) } else { None }).collect();
            h.check("render/pictures/items", imgs == ["png", "png", "jpeg", "emf", "png", "png"], format!("six pictures: {imgs:?}"));
            let kinds: Vec<bool> = v
                .drawing()
                .items
                .iter()
                .filter_map(|i| if let ItemKind::Image(img) = &i.kind { Some(diagram_view::image::bytes(doc, &img.source).map(|b| matches!(diagram_view::image::decode(&b), Decoded::Picture(_))).unwrap_or(false)) } else { None })
                .collect();
            h.check("render/pictures/decode", kinds == [true, true, true, false, true, true], format!("PNG and JPEG decode, EMF is a placeholder: {kinds:?}"));
        }
        "pages" => {
            let d = v.drawing();
            if d.scale < 0.5 {
                let bb = n(1).iter().find(|i| matches!(i.kind, ItemKind::Fill { .. })).map(|i| i.bbox);
                let w = stroke_of(&n(1)).map(|s| s.width);
                h.check("render/pages/scale", (d.scale - 0.1).abs() < 1e-12 && (d.width - 6.0).abs() < 1e-9 && bb.is_some_and(|b| near([b[0], b[1]], [1.0, 1.5], 1e-9) && near([b[2], b[3]], [3.0, 2.5], 1e-9)) && w.is_some_and(|w| (w - 0.75 / 72.0).abs() < 1e-6),
                    format!("1:10: the page 6 in wide, a 20 in box 2 in on paper, its line 0.75 pt: {} {bb:?} {w:?}", d.width));
            } else {
                let first_owner = d.items.first().map(|i| i.shape.owner);
                h.check("render/pages/background", first_owner.is_some_and(|o| o != Owner::Page(page)), String::from("the background page lies under the page"));
                let s1 = n(1);
                let fill = fill_color(&s1);
                let line = stroke_of(&s1).map(|s| (s.color, s.width));
                let family = texts(&s1).first().map(|t| t.runs[0].family.clone());
                h.check("render/pages/themed", fill == Some(0xFFC0_0000) && line.is_some_and(|(c, w)| c == 0xFF00_8080 && (w - 38100.0 / 914_400.0).abs() < 1e-9) && family.as_deref() == Some("Consolas"),
                    format!("Themed values from the page's theme (ThemeIndex): fill {fill:08x?}, line {line:?}, font {family:?}"));
            }
        }
        _ => {}
    }
}

/// group "render": display lists and golden images
pub fn run(h: &mut H) {
    let golden = find_up("apps/diagramiotest/tests/golden");
    h.check("render/golden-folder", golden.is_some(), format!("{golden:?}"));
    // the font classes of the substitution, base64 of inline pictures (diagram_view has no unit
    // tests of its own: rt's panic handler keeps cargo test from building it)
    use diagram_view::class_of;
    let classes = [class_of("Times New Roman"), class_of("Consolas"), class_of("Calibri"), class_of("DejaVu Sans"), class_of("DejaVu Serif")];
    h.check("render/fonts/classes", classes == [Class::Serif, Class::Mono, Class::Sans, Class::Sans, Class::Serif], format!("{classes:?}"));
    let b64 = (diagram_view::image::base64(b"SGFs\nbG8="), diagram_view::image::base64(b"AA=="), diagram_view::image::base64(b"A*"));
    h.check("render/pictures/base64", b64 == (Some(b"Hallo".to_vec()), Some(alloc::vec![0u8]), None), format!("{b64:?}"));
    let mut fonts = fonts();
    for (name, pkg) in drawings::all() {
        let Some(doc) = load(&pkg.bytes()) else {
            h.check(&format!("render/{name}/load"), false, String::from("the drawing loads"));
            continue;
        };
        for (k, page) in foreground(&doc).into_iter().enumerate() {
            let t0 = plat::time_us();
            let mut v = PageView::new(&doc, page, &mut fonts, &Options::default());
            checks(h, name, &doc, page, &v);
            h.check(&format!("render/{name}-{}/accepted", k + 1), v.refused() == 0, format!("render2d took every item: {} refused", v.refused()));
            let (w, hh) = image_size(&v);
            let img = v.render(w, hh);
            let ms = plat::time_us().wrapping_sub(t0) / 1000;
            let Some(dir) = &golden else { continue };
            let path = format!("{dir}/{name}-{}.png", k + 1);
            let want = plat::read_file(&path).and_then(|b| img_png::decode_pargb(&b));
            let what = match want.as_ref().and_then(|g| diff(&img, g)) {
                Some((far, max)) => {
                    let limit = (img.w as f64 * img.h as f64 * PER_MILLE / 1000.0) as u64;
                    (far <= limit, format!("{far} pixels off by more than {FAR} (at most {limit}), largest difference {max}, {} items, {ms} ms", v.drawing().items.len()))
                }
                None => (false, format!("{path}: missing or another size (write it with --write-golden)")),
            };
            h.check(&format!("render/{name}-{}/golden", k + 1), what.0, what.1);
        }
    }
}

/// --write-golden DIR
pub fn write_golden(dir: &str) -> i32 {
    let mut fonts = fonts();
    for (name, pkg) in drawings::all() {
        let Some(doc) = load(&pkg.bytes()) else {
            out(&format!("error: {name} does not load\n"));
            return 1;
        };
        for (k, page) in foreground(&doc).into_iter().enumerate() {
            let mut v = PageView::new(&doc, page, &mut fonts, &Options::default());
            let (w, h) = image_size(&v);
            let img = v.render(w, h);
            let path = format!("{dir}/{name}-{}.png", k + 1);
            if !plat::write_file(&path, &png(&img)) {
                out(&format!("error: cannot write {path}\n"));
                return 1;
            }
            out(&format!("wrote {path} ({} items)\n", v.drawing().items.len()));
        }
    }
    0
}

/// --write-drawings DIR: the self-built drawings as .vsdx (to open them in Visio, and the app's
/// test file apps/JFDiagrammer/tests/viewer.vsdx, which is pages.vsdx)
pub fn write_drawings(dir: &str) -> i32 {
    for (name, pkg) in drawings::all() {
        let path = format!("{dir}/{name}.vsdx");
        if !plat::write_file(&path, &pkg.bytes()) {
            out(&format!("error: cannot write {path}\n"));
            return 1;
        }
        out(&format!("wrote {path}\n"));
    }
    0
}

/// the local files' pages as PNG and SVG into `dir` (width px across)
pub fn render_corpus(dir: &str, corpus: Option<String>, width: u32) -> i32 {
    let Some(src) = corpus.or_else(|| find_up("apps/JFDiagrammer/context/files")) else {
        out("no local corpus (apps/JFDiagrammer/context/files): nothing to draw\n");
        return 0;
    };
    let mut fonts = fonts();
    for f in ["Zeichnung2.vsdx", "Zeichnung 3.vsdx", "Favoriten.vssx"] {
        let Some(bytes) = plat::read_file(&format!("{src}/{f}")) else { continue };
        let Some(doc) = load(&bytes) else {
            out(&format!("{f}: does not load\n"));
            continue;
        };
        let stem = f.rsplit_once('.').map_or(f, |x| x.0);
        if doc.kind == DocKind::Stencil {
            out(&format!("{f}: a stencil, no pages\n"));
            continue;
        }
        for (k, page) in foreground(&doc).into_iter().enumerate() {
            let t0 = plat::time_us();
            let mut v = PageView::new(&doc, page, &mut fonts, &Options::default());
            let t1 = plat::time_us();
            let d = v.drawing();
            let h = (width as f64 * d.height / d.width.max(1e-6) + 0.5) as u32;
            let img = v.render(width, h.clamp(16, 4 * width));
            let t2 = plat::time_us();
            let png_path = format!("{dir}/{stem}-{}.png", k + 1);
            let svg_path = format!("{dir}/{stem}.p{}.svg", k + 1);
            let ok = plat::write_file(&png_path, &png(&img)) && plat::write_file(&svg_path, crate::svgdump::svg(&doc, v.drawing()).as_bytes());
            let d = v.drawing();
            out(&format!(
                "{f} page {}: {} items, {:.2} x {:.2} in, built {} ms, drawn {} ms{}; substituted {:?}; notes {:?}\n",
                k + 1,
                d.items.len(),
                d.width,
                d.height,
                (t1 - t0) / 1000,
                (t2 - t1) / 1000,
                if ok { "" } else { " (NOT WRITTEN)" },
                v.substitutions(),
                d.notes
            ));
        }
    }
    0
}

/// group "render/corpus": every local page built and drawn (skipped without the files)
pub fn corpus(h: &mut H, dir: Option<String>) {
    let Some(src) = dir.or_else(|| find_up("apps/JFDiagrammer/context/files")) else {
        out("skip render/corpus: no local corpus\n");
        return;
    };
    let mut fonts = fonts();
    for f in ["Zeichnung2.vsdx", "Zeichnung 3.vsdx"] {
        let Some(bytes) = plat::read_file(&format!("{src}/{f}")) else { continue };
        let Some(doc) = load(&bytes) else {
            h.check(&format!("render/corpus/{f}"), false, String::from("loads"));
            continue;
        };
        for (k, page) in foreground(&doc).into_iter().enumerate() {
            let t0 = plat::time_us();
            let mut v = PageView::new(&doc, page, &mut fonts, &Options::default());
            let img = v.render(800, 600);
            let ms = plat::time_us().wrapping_sub(t0) / 1000;
            let shapes = doc.page(page).map_or(0, |p| p.shapes.iter().count());
            let items = v.drawing().items.len();
            let ink = img.px.chunks_exact(4).filter(|p| p[0] != p[1] || p[1] != p[2]).count();
            h.check(&format!("render/corpus/{f}-{}", k + 1), (shapes == 0 || items > 0) && ms < 20_000 && v.refused() == 0,
                format!("{shapes} shapes -> {items} items, {} refused, {ink} coloured pixels of 800 x 600, {ms} ms", v.refused()));
        }
    }
}
