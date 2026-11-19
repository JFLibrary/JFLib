// edit_view -- the page view while editing (JD7): PageView::update against a page built anew,
// the formula engine's text sizes against the drawn text, the time an edit takes
//
//   run(h)                      group "editview":
//     editview/<edit>/list      after an edit, update's display list equals PageView::new's
//     editview/<edit>/pixels    and its image the same (render2d's CPU path, 640 px across)
//     editview/<edit>/area      the box update names holds the shape's old and new items
//     editview/order            a shape that came or changed its order: None, built anew
//     editview/metrics-width    TEXTWIDTH(TheText) through diagram_view::Metrics = the drawn
//                               line's width (plus the margins)
//     editview/metrics-height   TEXTHEIGHT(TheText, Width) = the drawn lines' height
//     editview/time             1000 shapes, one moved: update, the whole view drawn, only the
//                               changed box drawn (Dev profile; printed, bounds generous)
//
// The pages are built here with the model's API (rectangles with texts, shadows, old-style
// gradients that gfx draws between render2d's layers); nothing comes from a Visio file.  The
// fonts are render.rs's (Inter for Calibri, scaled by the sample width).

use alloc::format;
use alloc::rc::Rc;
use alloc::vec::Vec;

use diagram_core::formula::{Engine, Outcome, Val};
use diagram_core::render::{ItemKind, Options};
use diagram_core::{Cell, CellAddr, DocKind, Document, Owner, Page, PageHeader, PageRef, Row, RowKey, SecKey, Section, ShapeData, ShapeKind, ShapeRef, SheetRef, Text, Value};
use diagram_view::{FontSet, Metrics, PageView};
use gfx::{Dc, Surface, FMT_XRGB};

use crate::render::fonts;
use crate::{out, H};

const WIDTH: u32 = 640;

// ---- pages

fn cell(doc: &mut Document, n: &str, v: f64) -> Cell {
    Cell::constant(doc.intern(n), Value::Num(v), None)
}

fn rect_geometry(doc: &mut Document, w: f64, h: f64) -> Section {
    let geo = doc.intern("Geometry");
    let mut sec = Section::new(SecKey::one(geo));
    for (i, p) in [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h], [0.0, 0.0]].iter().enumerate() {
        let mut r = Row::new(RowKey::Index(i as u32 + 1));
        r.kind = Some(doc.intern(if i == 0 { "MoveTo" } else { "LineTo" }));
        r.cells.push(cell(doc, "X", p[0]));
        r.cells.push(cell(doc, "Y", p[1]));
        sec.rows.push(r);
    }
    sec
}

/// a rectangle with cells and a text at pin of size w x h
fn add_shape(doc: &mut Document, page: PageRef, id: u32, pin: [f64; 2], size: [f64; 2], cells: &[(&str, f64)], text: Option<&str>) -> ShapeRef {
    let mut d = ShapeData::new(ShapeKind::Shape);
    for (n, v) in [("PinX", pin[0]), ("PinY", pin[1]), ("Width", size[0]), ("Height", size[1]), ("LocPinX", size[0] * 0.5), ("LocPinY", size[1] * 0.5)].iter().chain(cells.iter()) {
        let c = cell(doc, n, *v);
        d.sheet.cells.push(c);
    }
    let g = rect_geometry(doc, size[0], size[1]);
    d.sheet.sections.push(g);
    if let Some(t) = text {
        d.text = Some(alloc::boxed::Box::new(Text::plain(t)));
    }
    let slot = doc.page_mut(page).and_then(|p| p.shapes.insert(None, id, d).ok()).unwrap_or(0);
    ShapeRef { owner: Owner::Page(page), slot }
}

/// a page of cols x rows shapes on a w x h page: texts, every 13th with a shadow, every 25th
/// with an old-style gradient (drawn by gfx between two layers)
fn grid_page(cols: u32, rows: u32) -> (Document, PageRef, Vec<ShapeRef>) {
    let mut doc = Document::new(DocKind::Drawing);
    let page = doc.insert_page(Page::new(PageHeader { id: 0, ..PageHeader::default() })).unwrap_or_else(|_| unreachable!());
    let (pw, ph) = (cols as f64 * 1.0 + 0.5, rows as f64 * 0.75 + 0.5);
    {
        let (wc, hc) = (cell(&mut doc, "PageWidth", pw), cell(&mut doc, "PageHeight", ph));
        if let Some(p) = doc.page_mut(page) {
            p.sheet.cells.push(wc);
            p.sheet.cells.push(hc);
        }
    }
    let mut shapes = Vec::new();
    for k in 0..cols * rows {
        let (c, r) = (k % cols, k / cols);
        let pin = [0.75 + c as f64, 0.625 + r as f64 * 0.75];
        let mut cells: Vec<(&str, f64)> = alloc::vec![("FillForegnd", 3.0 + (k % 5) as f64)];
        if k % 13 == 7 {
            cells.push(("ShdwPattern", 1.0));
        }
        if k % 25 == 12 {
            cells.push(("FillPattern", 28.0));
        }
        let text = format!("Shape {}", k + 1);
        shapes.push(add_shape(&mut doc, page, k + 1, pin, [0.8, 0.5], &cells, Some(&text)));
    }
    (doc, page, shapes)
}

fn set(doc: &mut Document, s: ShapeRef, name: &str, v: f64) -> bool {
    let mut tx = doc.transaction("edit");
    let a = tx.single(name);
    let ok = tx.set_constant(SheetRef::Shape(s), &a, Value::Num(v), None).is_ok();
    tx.commit();
    ok
}

fn set_text(doc: &mut Document, s: ShapeRef, text: Option<&str>) -> bool {
    let mut tx = doc.transaction("text");
    let ok = tx.set_text(s, text.map(Text::plain)).is_ok();
    tx.commit();
    ok
}

/// pixels that differ at all, and by more than 16 in a channel
fn diff(a: &Surface, b: &Surface) -> (u64, u64) {
    if a.w != b.w || a.h != b.h {
        return (u64::MAX, u64::MAX);
    }
    let (mut any, mut far) = (0u64, 0u64);
    for y in 0..a.h as usize {
        let (ra, rb) = (&a.px[y * a.stride as usize..], &b.px[y * b.stride as usize..]);
        for x in 0..a.w as usize {
            let m = (0..3).map(|k| (ra[x * 4 + k] as i32 - rb[x * 4 + k] as i32).unsigned_abs()).max().unwrap_or(0);
            any += (m > 0) as u64;
            far += (m > 16) as u64;
        }
    }
    (any, far)
}

fn image_size(v: &PageView) -> (u32, u32) {
    let d = v.drawing();
    let h = (WIDTH as f64 * d.height / d.width.max(1e-6) + 0.5) as u32;
    (WIDTH, h.clamp(16, 4 * WIDTH))
}

/// v after an update against a view built anew: the list, the pixels
fn compare(h: &mut H, name: &str, doc: &Document, page: PageRef, fonts: &mut FontSet, v: &mut PageView) {
    let mut fresh = PageView::new(doc, page, fonts, &Options::default());
    let (a, b) = (&v.drawing().items, &fresh.drawing().items);
    let first = a.iter().zip(b.iter()).position(|(x, y)| x != y);
    h.check(&format!("editview/{name}/list"), a.len() == b.len() && first.is_none(), format!("{} items, built anew {}, first different {first:?}", a.len(), b.len()));
    let (w, hh) = image_size(v);
    let (ia, ib) = (v.render(w, hh), fresh.render(w, hh));
    let (any, far) = diff(&ia, &ib);
    let limit = (w as u64 * hh as u64) / 1000;
    h.check(&format!("editview/{name}/pixels"), far <= limit && v.refused() == fresh.refused(), format!("{any} pixels differ, {far} by more than 16 (at most {limit}), refused {} / {}", v.refused(), fresh.refused()));
}

fn boxes_of(v: &PageView, top: ShapeRef) -> Option<[f64; 4]> {
    let d = v.drawing();
    d.range_of(top)?.map(|i| d.items[i].bbox).reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
}

fn holds(outer: [f64; 4], inner: Option<[f64; 4]>) -> bool {
    inner.is_none_or(|b| outer[0] <= b[0] + 1e-12 && outer[1] <= b[1] + 1e-12 && outer[2] >= b[2] - 1e-12 && outer[3] >= b[3] - 1e-12)
}

// ---- checks

fn edits(h: &mut H, fonts: &mut FontSet) {
    let (mut doc, page, s) = grid_page(10, 8);
    let mut v = PageView::new(&doc, page, fonts, &Options::default());
    // a move: the same items, each entity in place
    let before = boxes_of(&v, s[33]);
    set(&mut doc, s[33], "PinX", 4.4);
    set(&mut doc, s[33], "PinY", 5.1);
    let area = v.update(&doc, fonts, &[s[33]]);
    let after = boxes_of(&v, s[33]);
    h.check("editview/move/area", area.is_some_and(|a| holds(a, before) && holds(a, after)), format!("{area:?} holds {before:?} and {after:?}"));
    compare(h, "move", &doc, page, fonts, &mut v);
    // a longer text: more runs (wrapping), the layer's entities renewed or the layer remade
    set_text(&mut doc, s[40], Some("ein deutlich längerer Text, der umbricht"));
    let r = v.update(&doc, fonts, &[s[40]]);
    h.check("editview/text/some", r.is_some(), format!("{r:?}"));
    compare(h, "text", &doc, page, fonts, &mut v);
    // fewer items: no line, no text
    set(&mut doc, s[41], "LinePattern", 0.0);
    set_text(&mut doc, s[41], None);
    v.update(&doc, fonts, &[s[41]]);
    compare(h, "fewer", &doc, page, fonts, &mut v);
    // a gradient comes (an item for gfx in the middle of a layer) and goes elsewhere
    set(&mut doc, s[50], "FillPattern", 30.0);
    set(&mut doc, s[12], "FillPattern", 1.0);
    v.update(&doc, fonts, &[s[50], s[12]]);
    compare(h, "gradient", &doc, page, fonts, &mut v);
    // several shapes on one layer, counts changing, a shadow added
    set(&mut doc, s[2], "ShdwPattern", 1.0);
    set_text(&mut doc, s[3], Some("noch ein langer Text mit Umbruch"));
    set(&mut doc, s[4], "PinX", 9.0);
    set_text(&mut doc, s[5], None);
    v.update(&doc, fonts, &[s[5], s[3], s[2], s[4]]);
    compare(h, "several", &doc, page, fonts, &mut v);
    // the last gradient and every shadow gone: the layers all shown again
    let mut all = Vec::new();
    for (k, &x) in s.iter().enumerate() {
        if k % 25 == 12 || k == 50 {
            set(&mut doc, x, "FillPattern", 1.0);
            all.push(x);
        }
    }
    v.update(&doc, fonts, &all);
    compare(h, "no-gradient", &doc, page, fonts, &mut v);
    // the order: None, built anew
    {
        let mut tx = doc.transaction("front");
        let _ = tx.bring_to_front(s[0]);
        tx.commit();
    }
    let r = v.update(&doc, fonts, &[s[0]]);
    h.check("editview/order", r.is_none(), format!("{r:?}"));
    compare(h, "order", &doc, page, fonts, &mut v);
}

fn metrics(h: &mut H, fonts: &mut FontSet) {
    let mut doc = Document::new(DocKind::Drawing);
    let page = doc.insert_page(Page::new(PageHeader { id: 0, ..PageHeader::default() })).unwrap_or_else(|_| unreachable!());
    let m = [("LeftMargin", 0.05), ("RightMargin", 0.07), ("TopMargin", 0.03), ("BottomMargin", 0.04)];
    let one = add_shape(&mut doc, page, 1, [3.0, 3.0], [4.0, 1.0], &m, Some("Hallo Welt, Breite"));
    let wrapped = add_shape(&mut doc, page, 2, [3.0, 6.0], [1.2, 2.0], &m, Some("ein langer Text, der in der schmalen Form mehrmals umbricht"));
    let v = PageView::new(&doc, page, fonts, &Options::default());
    let runs = |s: ShapeRef| -> Vec<diagram_core::render::TextRun> {
        let d = v.drawing();
        d.items.iter().filter(|i| i.shape == s).filter_map(|i| if let ItemKind::Text(t) = &i.kind { Some(t.runs.clone()) } else { None }).flatten().collect()
    };
    let drawn_w: f64 = runs(one).iter().map(|r| r.width).sum();
    let mut lines: Vec<i64> = runs(wrapped).iter().map(|r| (r.at[1] * 1e9) as i64).collect();
    lines.dedup();
    let size = 12.0 / 72.0;
    let mut eng = Engine::with_metrics(Rc::new(Metrics::new(fonts)));
    let addr = CellAddr::single(doc.intern("Width"));
    let num = |o: Result<Outcome, _>| match o {
        Ok(Outcome::Value(Val::Num(x))) => Some(x),
        _ => None,
    };
    let tw = num(eng.evaluate_text(&doc, SheetRef::Shape(one), &addr, "TEXTWIDTH(TheText)"));
    let want = drawn_w + 0.05 + 0.07;
    h.check("editview/metrics-width", tw.is_some_and(|w| (w - want).abs() < 1e-9), format!("TEXTWIDTH {tw:?}, drawn {want} (the line {drawn_w} + margins)"));
    let th = num(eng.evaluate_text(&doc, SheetRef::Shape(wrapped), &addr, "TEXTHEIGHT(TheText, Width)"));
    let want_h = lines.len() as f64 * 1.2 * size + 0.03 + 0.04;
    h.check("editview/metrics-height", th.is_some_and(|x| (x - want_h).abs() < 1e-9) && lines.len() >= 3, format!("TEXTHEIGHT {th:?}, drawn {} lines = {want_h}", lines.len()));
    // the estimate the engine uses without Metrics is off
    let mut est = Engine::new();
    let te = num(est.evaluate_text(&doc, SheetRef::Shape(one), &addr, "TEXTWIDTH(TheText)"));
    out(&format!("     TEXTWIDTH: Metrics {tw:?}, estimate {te:?}, drawn {want:.6}\n"));
}

fn time(h: &mut H, fonts: &mut FontSet) {
    let (mut doc, page, s) = grid_page(40, 25);
    let t0 = plat::time_us();
    let mut v = PageView::new(&doc, page, fonts, &Options::default());
    let built = plat::time_us().wrapping_sub(t0);
    let (w, hh) = (1600u32, 1000u32);
    let cam = v.fit((w, hh), 8.0);
    let mut sf = Surface::new(w as i32, hh as i32, FMT_XRGB);
    let mut dc = Dc::new(w as i32, hh as i32);
    dc.begin(&sf);
    dc.clear(&mut sf, 0xFFFF_FFFF);
    v.draw(&mut dc, &mut sf, [0, 0, w as i32, hh as i32], &cam);
    // one shape moved in steps (two shapes: a plain one, one with a gradient gfx draws): update,
    // the whole view, the changed box only (the view clipped to it)
    let mut times: Vec<u64> = Vec::new();
    let (mut full, mut part, mut steps) = (0u64, 0u64, 0u64);
    let (mut areas_ok, mut part_same) = (true, true);
    let mut whole = Surface::new(w as i32, hh as i32, FMT_XRGB);
    for &n in &[500usize, 512] {
        let target = s[n];
        let x0 = 0.75 + (n % 40) as f64;
        for k in 0..15 {
            set(&mut doc, target, "PinX", x0 + 0.03 * (k + 1) as f64);
            let t1 = plat::time_us();
            let area = v.update(&doc, fonts, &[target]);
            let t2 = plat::time_us();
            dc.clear(&mut sf, 0xFFFF_FFFF);
            v.draw(&mut dc, &mut sf, [0, 0, w as i32, hh as i32], &cam);
            let t3 = plat::time_us();
            let Some(a) = area else {
                areas_ok = false;
                continue;
            };
            whole.px.copy_from_slice(&sf.px);
            let (p, q) = (cam.to_screen([a[0], a[3]], (w, hh)), cam.to_screen([a[2], a[1]], (w, hh)));
            let (px0, py0) = ((p[0] as i32 - 2).max(0), (p[1] as i32 - 2).max(0));
            let (px1, py1) = ((q[0] as i32 + 3).min(w as i32), (q[1] as i32 + 3).min(hh as i32));
            let t4 = plat::time_us();
            dc.clip_push(px0, py0, (px1 - px0).max(0), (py1 - py0).max(0));
            dc.set_color(0xFFFF_FFFF);
            dc.fill(&mut sf, px0, py0, (px1 - px0).max(0), (py1 - py0).max(0));
            v.draw(&mut dc, &mut sf, [0, 0, w as i32, hh as i32], &cam);
            dc.clip_pop();
            let t5 = plat::time_us();
            // the box drawn alone: the pixels of the whole view
            part_same &= diff(&sf, &whole).1 == 0;
            times.push(t2 - t1);
            full += t3 - t2;
            part += t5 - t4;
            steps += 1;
        }
    }
    times.sort_unstable();
    let med = times.get(times.len() / 2).copied().unwrap_or(0);
    let max = times.last().copied().unwrap_or(0);
    // a text that grows (more runs): its layers remade; the first time with glyphs the page
    // had not drawn yet, the second time without
    let target = s[500];
    let long = "Shape 501 mit einem längeren Text, der umbricht";
    let grow = |doc: &mut Document, v: &mut PageView, fonts: &mut FontSet, text: &str| {
        set_text(doc, target, Some(text));
        let t = plat::time_us();
        let r = v.update(doc, fonts, &[target]);
        (plat::time_us().wrapping_sub(t), r.is_some())
    };
    let (g1, ok1) = grow(&mut doc, &mut v, fonts, long);
    let (_, ok2) = grow(&mut doc, &mut v, fonts, "Shape 501");
    let (g2, ok3) = grow(&mut doc, &mut v, fonts, long);
    dc.end();
    let ms = |us: u64| us as f64 / 1000.0;
    let items = v.drawing().items.len();
    out(&format!(
        "     1000 shapes ({items} items): built {:.1} ms; a move: update median {:.3} ms (at most {:.3}), the whole view {:.1} ms, the changed box {:.2} ms ({w} x {hh} px); a longer text {:.3} ms (new glyphs {:.3} ms)\n",
        ms(built),
        ms(med),
        ms(max),
        ms(full / steps.max(1)),
        ms(part / steps.max(1)),
        ms(g2),
        ms(g1)
    ));
    h.check("editview/part-pixels", part_same && steps == 30, format!("the changed box drawn alone has the whole view's pixels ({steps} steps)"));
    h.check("editview/time", areas_ok && ok1 && ok2 && ok3 && med < 10_000 && g2 < 50_000, format!("update {:.3} ms a step (median), {:.3} ms for a longer text", ms(med), ms(g2)));
}

/// the local files' pages (when apps/JFDiagrammer/context/files lies above): every top-level
/// shape edited in turn through update (moved, its text longer or its line off, a shadow), then
/// the view against one built anew; numbers only
fn corpus(h: &mut H, fonts: &mut FontSet) {
    let Some(src) = crate::render::find_up("apps/JFDiagrammer/context/files") else {
        out("skip editview/corpus: no local corpus\n");
        return;
    };
    for f in ["Zeichnung2.vsdx", "Zeichnung 3.vsdx"] {
        let Some(bytes) = plat::read_file(&format!("{src}/{f}")) else { continue };
        let mut lim = diagram_io::vsdx::Limits::VSDX;
        lim.zip.ratio = 2000;
        let Ok(loaded) = diagram_io::vsdx::read(&bytes, &lim) else { continue };
        let mut doc = loaded.doc;
        let pages: Vec<PageRef> = doc.foreground_pages().collect();
        for (k, page) in pages.into_iter().enumerate() {
            let mut v = PageView::new(&doc, page, fonts, &Options::default());
            let tops: Vec<ShapeRef> = doc.top_shapes(Owner::Page(page)).collect();
            let (mut nones, mut t) = (0u32, 0u64);
            for (i, &s) in tops.iter().enumerate() {
                let pin = doc.value_of(SheetRef::Shape(s), "PinX").num().unwrap_or(0.0);
                match i % 3 {
                    0 => {
                        set(&mut doc, s, "PinX", pin + 0.2);
                    }
                    1 if doc.shape(s).is_some_and(|x| x.text().is_some()) => {
                        set_text(&mut doc, s, Some("ein längerer Ersatztext, der in der Form umbricht"));
                    }
                    1 => {
                        set(&mut doc, s, "LinePattern", 0.0);
                    }
                    _ => {
                        set(&mut doc, s, "ShdwPattern", 1.0);
                    }
                }
                let t0 = plat::time_us();
                nones += v.update(&doc, fonts, &[s]).is_none() as u32;
                t += plat::time_us().wrapping_sub(t0);
            }
            compare(h, &format!("corpus/{f}-{}", k + 1), &doc, page, fonts, &mut v);
            out(&format!("     {f} page {}: {} top-level shapes edited, {nones} built anew, {:.3} ms an update\n", k + 1, tops.len(), t as f64 / 1000.0 / tops.len().max(1) as f64));
        }
    }
}

/// group "editview"
pub fn run(h: &mut H) {
    let mut f = fonts();
    edits(h, &mut f);
    metrics(h, &mut f);
    time(h, &mut f);
    corpus(h, &mut f);
}
