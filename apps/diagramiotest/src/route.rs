// route -- (JD21, CN-04, CN-05, SS-19, SS-40) the routing styles with their directions, shapes
// connectors may cross and a way by hand, in a drawing built as JFDiagrammer builds it: a page per
// style, the connectors routed after a move; the cells written and read back; the corpus' cells
//
//   check(h)          group "route": the ways of every page (the sides the ends use, straight or
//                     right-angled, L shapes of the simple styles, ways through permeable shapes,
//                     the way by hand kept after a move); written and read back: RouteStyle,
//                     LineRouteExt, LineAdjustFrom/To, DynamicsOff, ShapeRouteStyle, ConFixedCode,
//                     ConLineRouteExt, ShapePermeableX/Y/Place and the ways unchanged; routed again
//                     after reading the same ways (the styles come from the file); the local
//                     corpus' connectors (if there): all right-angled, ConFixedCode 0 or 3 .. 6
//                     (never 1 or 2: none is kept, all route freely here)
//   write(dir)        --write-jd21-route DIR: JD21-Gegenprobe-Routing.vsdx and a PNG of each page
//
// The pages: A above (4, 6), B lower left (2, 3), C lower right (6.5, 3.2), connectors A -> B and
// A -> C glued to the whole shapes, the page's RouteStyle set, then C moved down a little and the
// connectors routed again (Visio routes when something moves); a title box says what Visio should
// show.  The last pages: a shape in the way that connectors may cross horizontally / vertically,
// and a connector with a piece moved by hand and a vertex of its own (ConFixedCode 1) whose shape
// moved afterwards.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::formula::Engine;
use diagram_core::ops::{self, Basic};
use diagram_core::render::Options;
use diagram_core::route_edit::{self, Edit};
use diagram_core::route_styles::{self, Kind, RouteStyle};
use diagram_core::{glue, CellAddr, DocKind, Document, Owner, PageRef, ShapeRef, SheetRef, Text, Value};
use diagram_io::vsdx::{self, Measure, WriteOptions};
use diagram_view::export::raster;
use diagram_view::PageView;

use crate::H;

#[derive(Copy, Clone, PartialEq)]
enum Extra {
    None,
    /// a shape between A and C that connectors may cross horizontally
    PassX,
    /// a shape between A and B that connectors may cross vertically
    PassY,
    /// A -> C's middle piece moved by hand, a vertex added and moved, then C moved
    ByHand,
}

struct Spec {
    name: &'static str,
    code: i64,
    extra: Extra,
    title: &'static str,
}

// i18n: none (the file is for the user's look in Visio, German like the instructions)
const SPECS: [Spec; 25] = [
    Spec { name: "Rechtwinklig", code: 1, extra: Extra::None, title: "RouteStyle 1 rechtwinklig: die günstigsten Seiten" },
    Spec { name: "Gerade", code: 2, extra: Extra::None, title: "RouteStyle 2 gerade: eine Linie zwischen den Umrissen" },
    Spec { name: "Organigramm oben-unten", code: 3, extra: Extra::None, title: "3 Organigramm von oben nach unten: unten heraus, oben hinein, Knick auf halber Höhe" },
    Spec { name: "Organigramm links-rechts", code: 4, extra: Extra::None, title: "4 Organigramm von links nach rechts: rechts heraus, links hinein (B liegt links: Umweg)" },
    Spec { name: "Flussdiagramm oben-unten", code: 5, extra: Extra::None, title: "5 Flussdiagramm von oben nach unten: unten heraus, oben hinein" },
    Spec { name: "Flussdiagramm links-rechts", code: 6, extra: Extra::None, title: "6 Flussdiagramm von links nach rechts: zu C rechts heraus, links hinein; zu B freie Seiten" },
    Spec { name: "Baum oben-unten", code: 7, extra: Extra::None, title: "7 Baum von oben nach unten: Knick gleich unter A (gemeinsame Linie)" },
    Spec { name: "Baum links-rechts", code: 8, extra: Extra::None, title: "8 Baum von links nach rechts: Knick gleich rechts von A" },
    Spec { name: "Netzwerk", code: 9, extra: Extra::None, title: "9 Netzwerk: gerade Linien (angenommen)" },
    Spec { name: "Organigramm unten-oben", code: 10, extra: Extra::None, title: "10 Organigramm von unten nach oben: oben heraus, unten hinein (Umweg)" },
    Spec { name: "Organigramm rechts-links", code: 11, extra: Extra::None, title: "11 Organigramm von rechts nach links" },
    Spec { name: "Flussdiagramm unten-oben", code: 12, extra: Extra::None, title: "12 Flussdiagramm von unten nach oben: gegen die Richtung freie Seiten" },
    Spec { name: "Flussdiagramm rechts-links", code: 13, extra: Extra::None, title: "13 Flussdiagramm von rechts nach links: zu B links heraus, rechts hinein" },
    Spec { name: "Baum unten-oben", code: 14, extra: Extra::None, title: "14 Baum von unten nach oben" },
    Spec { name: "Baum rechts-links", code: 15, extra: Extra::None, title: "15 Baum von rechts nach links" },
    Spec { name: "Mittig zu Mittig", code: 16, extra: Extra::None, title: "16 Mittig zu Mittig: gerade Linien auf die Mitten zu" },
    Spec { name: "Einfach oben-unten", code: 17, extra: Extra::None, title: "17 Einfach von oben nach unten: unten heraus, oben hinein, um nichts herum" },
    Spec { name: "Einfach links-rechts", code: 18, extra: Extra::None, title: "18 Einfach von links nach rechts" },
    Spec { name: "Einfach unten-oben", code: 19, extra: Extra::None, title: "19 Einfach von unten nach oben" },
    Spec { name: "Einfach rechts-links", code: 20, extra: Extra::None, title: "20 Einfach von rechts nach links" },
    Spec { name: "Einfach waagerecht-senkrecht", code: 21, extra: Extra::None, title: "21 Einfach waagerecht-senkrecht: seitlich heraus, von oben/unten hinein (ein L)" },
    Spec { name: "Einfach senkrecht-waagerecht", code: 22, extra: Extra::None, title: "22 Einfach senkrecht-waagerecht: oben/unten heraus, seitlich hinein" },
    Spec { name: "Durchlaessig waagerecht", code: 1, extra: Extra::PassX, title: "Das graue Shape: ShapePermeableX 1 -- A zu C waagerecht hindurch" },
    Spec { name: "Durchlaessig senkrecht", code: 1, extra: Extra::PassY, title: "Das graue Shape: ShapePermeableY 1 -- A zu B senkrecht hindurch" },
    Spec { name: "Von Hand", code: 1, extra: Extra::ByHand, title: "A zu C von Hand: senkrechtes Stück verschoben, eigener Knick; danach C bewegt (ConFixedCode 1)" },
];

/// a page as built
struct Built {
    page: PageRef,
    shapes: [ShapeRef; 3],
    conns: [ShapeRef; 2],
    extra: Option<ShapeRef>,
}

fn err(e: diagram_core::EditError) -> String {
    format!("{e:?}")
}

fn set(tx: &mut diagram_core::Transaction, sheet: SheetRef, cell: &str, v: f64) -> Result<(), String> {
    let a = tx.single(cell);
    tx.set_constant(sheet, &a, Value::Num(v), None).map_err(err)
}

fn rect(tx: &mut diagram_core::Transaction, eng: &mut Engine, owner: Owner, at: [f64; 2], size: [f64; 2], text: &str) -> Result<ShapeRef, String> {
    let (a, b) = ([at[0] - size[0] * 0.5, at[1] - size[1] * 0.5], [at[0] + size[0] * 0.5, at[1] + size[1] * 0.5]);
    let s = ops::new_shape(tx, eng, owner, Basic::Rectangle, a, b).map_err(err)?;
    ops::set_text(tx, eng, s, Text::plain(text)).map_err(err)?;
    Ok(s)
}

fn centre(doc: &Document, s: ShapeRef) -> [f64; 2] {
    ops::bounds(doc, &[s]).map_or([0.0, 0.0], |b| [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5])
}

/// the drawing: the pages of SPECS (the first one renamed, the rest added like it)
fn build() -> Result<(Document, Vec<Built>), String> {
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Metric);
    let first = doc.pages().next().ok_or("no page")?;
    let mut eng = Engine::new();
    let mut out = Vec::new();
    for (k, spec) in SPECS.iter().enumerate() {
        let mut tx = doc.transaction("Seite");
        let page = if k == 0 {
            tx.rename_page(first, spec.name, spec.name).map_err(err)?;
            first
        } else {
            tx.add_page_like(first, spec.name, spec.name, false, None).map_err(err)?
        };
        let owner = Owner::Page(page);
        set(&mut tx, SheetRef::Page(page), "RouteStyle", spec.code as f64)?;
        let a = rect(&mut tx, &mut eng, owner, [4.0, 6.0], [1.6, 0.8], "A")?;
        let b = rect(&mut tx, &mut eng, owner, [2.0, 3.0], [1.4, 0.8], "B")?;
        let c = rect(&mut tx, &mut eng, owner, [6.5, 3.2], [1.4, 0.8], "C")?;
        // a grey shape in the way: A -> C runs level through the tall one (y 6), A -> B upright
        // through the flat one (x 2)
        let wall = match spec.extra {
            Extra::PassX => Some(([5.6, 4.6], [0.5, 3.0], "ShapePermeableX")),
            Extra::PassY => Some(([2.0, 4.6], [1.6, 0.5], "ShapePermeableY")),
            _ => None,
        };
        let extra = match wall {
            Some((at, size, cell)) => {
                let w = rect(&mut tx, &mut eng, owner, at, size, "")?;
                set(&mut tx, SheetRef::Shape(w), cell, 1.0)?;
                let f = tx.single("FillForegnd");
                tx.set_formula(SheetRef::Shape(w), &f, "RGB(200,200,200)").map_err(err)?;
                eng.recalc(&mut tx, &[SheetRef::Shape(w)]).map_err(err)?;
                Some(w)
            }
            None => None,
        };
        let mut conns = [a, a];
        for (n, to) in [b, c].into_iter().enumerate() {
            let (pa, pb) = (centre(tx.doc(), a), centre(tx.doc(), to));
            conns[n] = glue::new_connector(&mut tx, &mut eng, owner, (pa, glue::Target::Shape(a)), (pb, glue::Target::Shape(to))).map_err(err)?;
        }
        if spec.extra == Extra::ByHand {
            // the upright piece of A -> C one inch to the right, a vertex on its first piece
            // dragged up into a step
            let w = route_edit::way(tx.doc(), conns[1]).ok_or("no way")?;
            if let Some(i) = (0..w.pts.len() - 1).find(|&i| (w.pts[i][0] - w.pts[i + 1][0]).abs() < 1e-9) {
                let p = w.pts[i];
                route_edit::edit(&mut tx, &mut eng, conns[1], Edit::Piece(i, [p[0] + 1.0, p[1]]), 0.0).map_err(err)?;
            }
            let w = route_edit::way(tx.doc(), conns[1]).ok_or("no way")?;
            let at = [(w.pts[0][0] + w.pts[1][0]) * 0.5, (w.pts[0][1] + w.pts[1][1]) * 0.5];
            route_edit::edit(&mut tx, &mut eng, conns[1], Edit::Add(at), 0.0).map_err(err)?;
            route_edit::edit(&mut tx, &mut eng, conns[1], Edit::Bend(1, [at[0], at[1] + 0.4]), 0.0).map_err(err)?;
        }
        // C moves down a little: what is glued to it routes again
        ops::move_shapes(&mut tx, &mut eng, &[c], [0.0, -0.2]).map_err(err)?;
        let deps = glue::dependents(tx.doc(), owner, &[c]);
        glue::update(&mut tx, &mut eng, &deps).map_err(err)?;
        // the title above the shapes
        let t = ops::new_shape(&mut tx, &mut eng, owner, Basic::Rectangle, [0.5, 7.4], [7.8, 8.0]).map_err(err)?;
        ops::set_text(&mut tx, &mut eng, t, Text::plain(&format!("{}\n", spec.title))).map_err(err)?;
        let _ = tx.commit();
        out.push(Built { page, shapes: [a, b, c], conns, extra });
    }
    Ok((doc, out))
}

fn heading(p: [f64; 2], q: [f64; 2]) -> [f64; 2] {
    let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
    if dx.abs() >= dy.abs() {
        [dx.signum(), 0.0]
    } else {
        [0.0, dy.signum()]
    }
}

fn upright_or_level(p: &[[f64; 2]]) -> bool {
    p.windows(2).all(|w| (w[0][0] - w[1][0]).abs() < 1e-9 || (w[0][1] - w[1][1]).abs() < 1e-9)
}

// whether box b lies at least `gap` further along d than box a
fn downstream(a: [f64; 4], b: [f64; 4], d: [f64; 2], gap: f64) -> bool {
    let far = |x: [f64; 4]| [[x[0], x[1]], [x[2], x[3]], [x[0], x[3]], [x[2], x[1]]].iter().map(|p| p[0] * d[0] + p[1] * d[1]).fold(f64::NEG_INFINITY, f64::max);
    let near = |x: [f64; 4]| [[x[0], x[1]], [x[2], x[3]], [x[0], x[3]], [x[2], x[1]]].iter().map(|p| p[0] * d[0] + p[1] * d[1]).fold(f64::INFINITY, f64::min);
    near(b) - far(a) >= gap
}

// what a page's ways must show -> a note of what is wrong (empty: fine)
fn judge(doc: &Document, spec: &Spec, b: &Built) -> String {
    let style = RouteStyle::from_code(spec.code).unwrap_or(RouteStyle::RIGHT_ANGLE);
    let mut notes = String::new();
    for (n, &c) in b.conns.iter().enumerate() {
        let p = glue::path(doc, c);
        let k = p.len();
        if k < 2 {
            notes.push_str(&format!("conn {n}: no way; "));
            continue;
        }
        let (first, last) = (heading(p[0], p[1]), heading(p[k - 2], p[k - 1]));
        if style.kind.straight() {
            if k != 2 {
                notes.push_str(&format!("conn {n}: not straight {p:?}; "));
            }
            continue;
        }
        if !upright_or_level(&p) {
            notes.push_str(&format!("conn {n}: slanted {p:?}; "));
        }
        let d = style.dir.map(|d| d.vector());
        let ok = match style.kind {
            Kind::OrgChart | Kind::Tree | Kind::Simple => d.is_some_and(|d| first == d && last == d),
            Kind::Flowchart => {
                let d = d.unwrap_or([0.0, -1.0]);
                let (ba, bb) = (ops::bounds(doc, &[b.shapes[0]]), ops::bounds(doc, &[b.shapes[n + 1]]));
                match (ba, bb) {
                    (Some(x), Some(y)) if downstream(x, y, d, 0.3) => first == d && last == d,
                    _ => true,
                }
            }
            Kind::SimpleHV => first[1] == 0.0 && last[0] == 0.0 && k == 3,
            Kind::SimpleVH => first[0] == 0.0 && last[1] == 0.0 && k == 3,
            _ => true,
        };
        if !ok {
            notes.push_str(&format!("conn {n}: sides {first:?} -> {last:?} {p:?}; "));
        }
    }
    match spec.extra {
        // A -> C level through the tall permeable shape, A -> B upright through the flat one,
        // each an L (one bend)
        Extra::PassX | Extra::PassY => {
            let k = if spec.extra == Extra::PassX { 1 } else { 0 };
            let p = glue::path(doc, b.conns[k]);
            let w = b.extra.and_then(|w| ops::bounds(doc, &[w]));
            let through = w.is_some_and(|w| p.windows(2).any(|s| diagram_core::route::hits_box(s[0], s[1], &w)));
            if !through || diagram_core::route::bends(&p) != 1 {
                notes.push_str(&format!("not through the permeable shape {p:?}; "));
            }
        }
        Extra::ByHand => {
            let code = num(doc, SheetRef::Shape(b.conns[1]), "ConFixedCode");
            let bends = diagram_core::route::bends(&glue::path(doc, b.conns[1]));
            if code != Some(1.0) || bends < 4 {
                notes.push_str(&format!("the way by hand: ConFixedCode {code:?}, {bends} bends; "));
            }
        }
        Extra::None => {}
    }
    notes
}

fn num(doc: &Document, s: SheetRef, cell: &str) -> Option<f64> {
    doc.sym(cell).and_then(|c| doc.resolve(s, &CellAddr::single(c)).num())
}

/// group "route"
pub(crate) fn check(h: &mut H, corpus: Option<String>) {
    let (doc, built) = match build() {
        Ok(b) => b,
        Err(e) => {
            h.check("route: build the drawing", false, e);
            return;
        }
    };
    for (spec, b) in SPECS.iter().zip(&built) {
        let notes = judge(&doc, spec, b);
        h.check(&format!("route: page {}", spec.name), notes.is_empty(), notes);
    }
    // written and read back
    let bytes = match vsdx::write(&doc, &WriteOptions::DEFAULT) {
        Ok(b) => b,
        Err(e) => {
            h.check("route: write", false, format!("{e}"));
            return;
        }
    };
    let mut back = match vsdx::read(&bytes, &vsdx::Limits::VSDX) {
        Ok(l) => l.doc,
        Err(e) => {
            h.check("route: read back", false, format!("{e}"));
            return;
        }
    };
    let pages: Vec<PageRef> = back.pages().collect();
    let page_cells = ["RouteStyle", "LineRouteExt", "LineAdjustFrom", "LineAdjustTo", "DynamicsOff"];
    let shape_cells = ["ShapeRouteStyle", "ConFixedCode", "ConLineRouteExt", "ShapePermeableX", "ShapePermeableY", "ShapePermeablePlace"];
    let (mut cells_ok, mut ways_ok, mut n) = (true, true, 0usize);
    let mut again: Vec<(PageRef, Vec<ShapeRef>, Vec<Vec<[f64; 2]>>)> = Vec::new();
    for (b, &pb) in built.iter().zip(&pages) {
        for c in page_cells {
            cells_ok &= num(&doc, SheetRef::Page(b.page), c) == num(&back, SheetRef::Page(pb), c);
        }
        let mut list = Vec::new();
        let mut ways = Vec::new();
        for &s in b.conns.iter().chain(b.shapes.iter()).chain(b.extra.iter()) {
            let id = doc.shape(s).map_or(0, |x| x.id());
            let Some(t) = back.shape_by_id(Owner::Page(pb), id) else {
                cells_ok = false;
                continue;
            };
            for c in shape_cells {
                cells_ok &= num(&doc, SheetRef::Shape(s), c) == num(&back, SheetRef::Shape(t), c);
            }
            if glue::is_connector(&back, t) {
                let (p, q) = (glue::path(&doc, s), glue::path(&back, t));
                ways_ok &= p.len() == q.len() && p.iter().zip(&q).all(|(x, y)| (x[0] - y[0]).abs() < 1e-6 && (x[1] - y[1]).abs() < 1e-6);
                n += 1;
                list.push(t);
                ways.push(p);
            }
        }
        again.push((pb, list, ways));
    }
    h.check("route: routing cells of pages and shapes read back", cells_ok, String::new());
    h.check("route: the ways read back unchanged", ways_ok && n == 50, format!("{n} connectors"));
    // routed again from the file's cells: the same ways (the way by hand kept)
    let mut eng = Engine::new();
    let mut same = true;
    let mut tx = back.transaction("neu");
    for (_, list, ways) in &again {
        if glue::update(&mut tx, &mut eng, list).is_err() {
            same = false;
        }
        for (t, p) in list.iter().zip(ways) {
            let q = glue::path(tx.doc(), *t);
            same &= p.len() == q.len() && p.iter().zip(&q).all(|(x, y)| (x[0] - y[0]).abs() < 1e-6 && (x[1] - y[1]).abs() < 1e-6);
        }
    }
    let _ = tx.commit();
    h.check("route: routed again after reading, the same ways", same, String::new());
    // the local corpus: Visio's connectors on the pages are right-angled, ConFixedCode 0 or 3 .. 6
    let Some(dir) = crate::corpus::folder(corpus) else {
        crate::out("skip route corpus: no apps/JFDiagrammer/context/files\n");
        return;
    };
    let (mut conns, mut right, mut codes) = (0usize, 0usize, [0usize; 7]);
    for f in ["Zeichnung2.vsdx", "Zeichnung 3.vsdx"] {
        let Some(bytes) = plat::read_file(&format!("{dir}/{f}")) else { continue };
        let Ok(l) = vsdx::read(&bytes, &vsdx::Limits::VSDX) else { continue };
        let d = l.doc;
        for p in d.pages().collect::<Vec<_>>() {
            for s in d.all_shapes(Owner::Page(p)).collect::<Vec<_>>() {
                if !glue::is_connector(&d, s) {
                    continue;
                }
                conns += 1;
                right += (route_styles::of(&d, s) == RouteStyle::RIGHT_ANGLE) as usize;
                let c = num(&d, SheetRef::Shape(s), "ConFixedCode").unwrap_or(0.0) as usize;
                codes[c.min(6)] += 1;
            }
        }
    }
    if conns == 0 {
        crate::out("skip route corpus: no connectors read\n");
        return;
    }
    h.check("route: corpus connectors right-angled (RouteStyle and ShapeRouteStyle 0)", right == conns, format!("{right} of {conns}"));
    // none kept by hand (1, 2): every one routes freely here, as before JD21
    h.check("route: corpus ConFixedCode 0 or 3 .. 6, never 1 or 2 (Visio's own routing: free here)", codes[1] + codes[2] == 0 && codes[3] + codes[4] + codes[5] + codes[6] > 0, format!("{codes:?}"));
}

/// --write-jd21-route DIR: the drawing for the Visio counter-check and a PNG of each page
pub(crate) fn write(dir: &str) -> i32 {
    img_core::io::register(img_core::IMGF_PNG, Some(&img_png::CODEC));
    let (doc, built) = match build() {
        Ok(b) => b,
        Err(e) => {
            crate::out(&format!("build: {e}\n"));
            return 1;
        }
    };
    let bytes = match vsdx::write(&doc, &WriteOptions::DEFAULT) {
        Ok(b) => b,
        Err(e) => {
            crate::out(&format!("write: {e}\n"));
            return 1;
        }
    };
    let path = format!("{dir}/JD21-Gegenprobe-Routing.vsdx");
    if !plat::write_file(&path, &bytes) {
        crate::out(&format!("cannot write {path}\n"));
        return 1;
    }
    crate::out(&format!("wrote {path}\n"));
    let mut fonts = crate::render::fonts();
    let mut status = 0;
    for (k, (spec, b)) in SPECS.iter().zip(&built).enumerate() {
        let mut v = PageView::new(&doc, b.page, &mut fonts, &Options::default());
        let img = raster::render(&mut v, &raster::RasterOptions { dpi: 100.0, ..Default::default() });
        let name: String = spec.name.chars().map(|c| if c == ' ' { '-' } else { c }).collect();
        let png_path = format!("{dir}/JD21-Routing-{:02}-{name}.png", k + 1);
        let ok = img.as_ref().and_then(|i| raster::png(i, 100.0)).is_some_and(|p| plat::write_file(&png_path, &p));
        crate::out(&format!("wrote {png_path}{}\n", if ok { "" } else { " (FAILED)" }));
        if !ok {
            status = 1;
        }
    }
    status
}
