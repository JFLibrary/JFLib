// tests of pagesetup and the page edits next to it (cargo test -p jflib-diagram-core --lib page)
//
//   Transaction::sheets, add_page_like (the settings of the active page, no layers, the
//   background page), duplicate_page (shapes, IDs, glue, UniqueIDs, the place after the
//   original, a copy of its own), orientation, a size of the menu, the automatic size's cells,
//   fitting the page to its drawing, the automatic size growing on every side, moving the
//   drawing, shrinking back, a page off the tiles' grid, the tiles of the corpus; every edit is
//   undone and redone
//
// Every document is built here; nothing comes from a Visio file.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use super::*;
use crate::cell::{Cell, Formula};
use crate::glue::{self, Target};
use crate::sheet::{Row, RowKey, SecKey, Section};
use crate::{Change, DocKind, Page, PageHeader, ShapeData, ShapeRef, Style, StyleRefs, Sym};

const A4: [f64; 2] = [210.0 / 25.4, 297.0 / 25.4];

struct Fx {
    doc: Document,
    page: PageRef,
    eng: Engine,
    next: u32,
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * (1.0 + b.abs())
}

fn cnum(doc: &mut Document, name: &str, v: f64) -> Cell {
    Cell::constant(doc.intern(name), Value::Num(v), None)
}

fn cf(doc: &mut Document, name: &str, f: &str, v: f64) -> Cell {
    let (n, f) = (doc.intern(name), doc.intern(f));
    Cell { name: n, value: Value::Num(v), formula: Formula::Expr(f), unit: None, error: None, stale: false }
}

fn row(key: u32, kind: Option<crate::Sym>, cells: Vec<Cell>) -> Row {
    let mut r = Row::new(RowKey::Index(key));
    r.kind = kind;
    r.cells = cells;
    r
}

// a drawing with one A4 landscape page (0.25 in margins, automatic size on unless `cells` say
// otherwise) and the two styles a connector needs
fn fx(cells: &[(&str, f64)]) -> Fx {
    let mut doc = Document::new(DocKind::Drawing);
    for (id, name, base) in [(0u32, "No Style", None), (1, "Normal", Some(0u32)), (2, "Connector", Some(1))] {
        let mut st = Style::default();
        st.header.id = id;
        st.header.name_u = Some(doc.intern(name));
        st.header.name = Some(doc.intern(name));
        if let Some(b) = base {
            st.header.based_on = StyleRefs::all(b);
        }
        doc.insert_style(st).unwrap();
    }
    let mut ph = PageHeader { id: 0, ..PageHeader::default() };
    ph.name_u = Some(doc.intern("Page-1"));
    ph.name = Some(doc.intern("Seite-1"));
    let mut page = Page::new(ph);
    let mut list: Vec<(&str, f64)> = vec![
        ("PageWidth", A4[1]),
        ("PageHeight", A4[0]),
        ("PageScale", 1.0),
        ("DrawingScale", 1.0),
        ("DrawingSizeType", 0.0),
        ("DrawingResizeType", 1.0),
        ("PrintPageOrientation", 2.0),
        ("PageLeftMargin", 0.25),
        ("PageRightMargin", 0.25),
        ("PageTopMargin", 0.25),
        ("PageBottomMargin", 0.25),
    ];
    for &(n, v) in cells {
        match list.iter_mut().find(|c| c.0 == n) {
            Some(c) => c.1 = v,
            None => list.push((n, v)),
        }
    }
    page.sheet.cells = list.iter().map(|&(n, v)| cnum(&mut doc, n, v)).collect();
    let page = doc.insert_page(page).unwrap();
    Fx { doc, page, eng: Engine::new(), next: 1 }
}

impl Fx {
    fn owner(&self) -> Owner {
        Owner::Page(self.page)
    }

    // an upright rectangle x0, y0, x1, y1 with Visio's connection points in the middle of
    // each side (rows 0 left, 1 right, 2 bottom, 3 top)
    fn rect(&mut self, b: [f64; 4]) -> ShapeRef {
        let doc = &mut self.doc;
        let (w, h) = (b[2] - b[0], b[3] - b[1]);
        let mut d = ShapeData::new(crate::ShapeKind::Shape);
        d.sheet.cells = vec![
            cnum(doc, "PinX", (b[0] + b[2]) * 0.5),
            cnum(doc, "PinY", (b[1] + b[3]) * 0.5),
            cnum(doc, "Width", w),
            cnum(doc, "Height", h),
            cf(doc, "LocPinX", "Width*0.5", w * 0.5),
            cf(doc, "LocPinY", "Height*0.5", h * 0.5),
        ];
        let mut geo = Section::new(SecKey::one(crate::names::GEOMETRY));
        geo.ix_written = true;
        for (i, p) in [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h], [0.0, 0.0]].iter().enumerate() {
            let kind = if i == 0 { crate::names::MOVE_TO } else { crate::names::LINE_TO };
            geo.rows.push(row(i as u32 + 1, Some(kind), vec![Cell::constant(crate::names::X, Value::Num(p[0]), None), Cell::constant(crate::names::Y, Value::Num(p[1]), None)]));
        }
        d.sheet.sections.push(geo);
        let mut con = Section::new(SecKey::one(crate::names::CONNECTION));
        for (i, (fx, fy, dx, dy)) in [(0.0, 0.5, 1.0, 0.0), (1.0, 0.5, -1.0, 0.0), (0.5, 0.0, 0.0, 1.0), (0.5, 1.0, 0.0, -1.0)].into_iter().enumerate() {
            let cells = vec![
                cf(doc, "X", &alloc::format!("Width*{fx}"), w * fx),
                cf(doc, "Y", &alloc::format!("Height*{fy}"), h * fy),
                cnum(doc, "DirX", dx),
                cnum(doc, "DirY", dy),
                cnum(doc, "Type", 0.0),
            ];
            con.rows.push(row(i as u32, None, cells));
        }
        d.sheet.sections.push(con);
        let id = self.next;
        self.next += 1;
        let slot = self.doc.page_mut(self.page).unwrap().shapes.insert(None, id, d).unwrap();
        ShapeRef { owner: self.owner(), slot }
    }

    // a dynamic connector from the right side of `a` to the left side of `b`
    fn connector(&mut self, a: ShapeRef, b: ShapeRef) -> ShapeRef {
        let (pa, pb) = (self.pin(a), self.pin(b));
        let owner = self.owner();
        self.edit(|tx, eng| glue::new_connector(tx, eng, owner, (pa, Target::Point { shape: a, row: 1 }), (pb, Target::Point { shape: b, row: 0 })))
    }

    fn value(&self, sheet: SheetRef, cell: &str) -> f64 {
        let a = CellAddr::single(self.doc.sym(cell).expect(cell));
        self.doc.resolve(sheet, &a).num().expect(cell)
    }

    fn pin(&self, s: ShapeRef) -> [f64; 2] {
        [self.value(SheetRef::Shape(s), "PinX"), self.value(SheetRef::Shape(s), "PinY")]
    }

    fn page_cell(&self, cell: &str) -> f64 {
        self.value(SheetRef::Page(self.page), cell)
    }

    // one transaction; it is undone and redone once to see that it round-trips
    fn edit<T>(&mut self, f: impl FnOnce(&mut Transaction, &mut Engine) -> Result<T, EditError>) -> T {
        let mut tx = self.doc.transaction("test");
        let r = f(&mut tx, &mut self.eng).expect("edit");
        let c: Option<Change> = tx.commit();
        if let Some(mut c) = c {
            let pages = self.pages();
            assert!(c.undo(&mut self.doc));
            assert!(c.redo(&mut self.doc));
            assert_eq!(self.pages(), pages, "undo and redo give the same pages");
        }
        r
    }

    // the pages in order: ID, Name, background, page sheet cells, shapes, glue
    fn pages(&self) -> Vec<(u32, String, bool, crate::Sheet, usize, Vec<crate::Connect>)> {
        self.doc
            .pages()
            .map(|p| {
                let pg = self.doc.page(p).unwrap();
                let name = pg.header.name.map_or(String::new(), |n| String::from(self.doc.str(n)));
                (pg.header.id, name, pg.header.background, pg.sheet.clone(), pg.shapes.len(), pg.connects.clone())
            })
            .collect()
    }
}

#[test]
fn transaction_names_its_sheets() {
    let mut f = fx(&[]);
    let s = f.rect([1.0, 1.0, 2.0, 2.0]);
    let mut tx = f.doc.transaction("t");
    assert!(tx.sheets().is_empty());
    let a = tx.single("PinX");
    tx.set_length(SheetRef::Shape(s), &a, 3.0, Unit::Inches).unwrap();
    assert_eq!(tx.sheets(), vec![SheetRef::Shape(s)]);
    let c = tx.commit().unwrap();
    assert_eq!(c.sheets(), vec![SheetRef::Shape(s)]);
}

#[test]
fn a_new_page_takes_the_settings_of_the_active_one() {
    let mut f = fx(&[("PrintPageOrientation", 1.0)]);
    // a layer, a background page, a theme on the active page
    let bg = f.edit(|tx, _| tx.add_page("Hintergrund-1", "Background-1", true, None));
    let page = f.page;
    f.edit(|tx, _| tx.set_background(page, Some(bg)));
    let layer = f.doc.intern("Layer");
    f.doc.page_mut(page).unwrap().sheet.sections.push(Section::new(SecKey::one(layer)));
    f.doc.themes.push(crate::Theme { part: Sym::EMPTY, data: Vec::new() });
    f.doc.page_mut(page).unwrap().theme = Some(0);
    let p = f.edit(|tx, _| tx.add_page_like(page, "Seite-2", "Page-2", false, Some(1)));
    let pg = f.doc.page(p).unwrap();
    assert_eq!(f.doc.page_position(p), Some(1));
    assert_eq!(pg.header.back_page, f.doc.page(bg).map(|b| b.header.id));
    assert_eq!(pg.theme, Some(0));
    assert!(!pg.header.background);
    assert!(pg.shapes.is_empty());
    assert!(pg.sheet.sections.iter().all(|s| s.name != layer), "no layers");
    assert_ne!(pg.header.id, 0);
    assert_eq!(f.doc.str(pg.header.name_u.unwrap()), "Page-2");
    assert!(close(f.value(SheetRef::Page(p), "PageWidth"), A4[1]));
    assert!(close(f.value(SheetRef::Page(p), "PrintPageOrientation"), 1.0));
    // a background page: the settings, no background of its own
    let b2 = f.edit(|tx, _| tx.add_page_like(page, "Hintergrund-2", "Background-2", true, None));
    let pg = f.doc.page(b2).unwrap();
    assert!(pg.header.background);
    assert_eq!(pg.header.back_page, None);
    assert_eq!(f.doc.page_position(b2), Some(3));
}

#[test]
fn a_duplicate_keeps_shapes_glue_and_ids() {
    let mut f = fx(&[]);
    let a = f.rect([1.0, 1.0, 2.0, 2.0]);
    let b = f.rect([4.0, 1.0, 5.0, 2.0]);
    let c = f.connector(a, b);
    let uid = f.doc.intern("{0000}");
    f.doc.page_mut(f.page).unwrap().shapes.data_mut(a.slot).unwrap().header.unique_id = Some(uid);
    let bg = f.edit(|tx, _| tx.add_page("Hintergrund-1", "Background-1", true, None));
    let page = f.page;
    let before = f.pages();
    let d = f.edit(|tx, _| tx.duplicate_page(page, "Seite-2", "Page-2", None));
    // right after the original, before the background page
    assert_eq!(f.doc.pages().collect::<Vec<_>>(), vec![page, d, bg]);
    let (src, dup) = (f.doc.page(page).unwrap(), f.doc.page(d).unwrap());
    assert_eq!(dup.shapes.len(), src.shapes.len());
    assert_eq!(dup.connects, src.connects);
    assert_eq!(dup.sheet, src.sheet);
    assert_eq!(dup.part, None);
    assert_ne!(dup.header.id, src.header.id);
    let ids = |o: Owner| f.doc.all_shapes(o).map(|s| f.doc.shape(s).unwrap().id()).collect::<Vec<_>>();
    assert_eq!(ids(Owner::Page(d)), ids(Owner::Page(page)));
    assert!(f.doc.all_shapes(Owner::Page(d)).all(|s| f.doc.shape(s).unwrap().header().unique_id.is_none()));
    assert_eq!(f.doc.shape(a).unwrap().header().unique_id, Some(uid), "the original keeps it");
    // the copy's connector follows the copy's shape, the original's stays
    let (b2, c2) = (ShapeRef { owner: Owner::Page(d), slot: b.slot }, ShapeRef { owner: Owner::Page(d), slot: c.slot });
    let end0 = f.value(SheetRef::Shape(c), "EndX");
    f.edit(|tx, eng| {
        ops::move_shapes(tx, eng, &[b2], [1.0, 0.0])?;
        let deps = glue::dependents(tx.doc(), Owner::Page(d), &[b2]);
        glue::update(tx, eng, &deps)
    });
    assert!(close(f.value(SheetRef::Shape(c2), "EndX"), end0 + 1.0));
    assert!(close(f.value(SheetRef::Shape(c), "EndX"), end0));
    // undone: the document as before
    let mut tx = f.doc.transaction("Seite löschen");
    tx.delete_page(d).unwrap();
    tx.commit().unwrap();
    assert_eq!(f.pages(), before);
}

#[test]
fn orientation_swaps_the_page_and_sets_the_print_orientation() {
    let mut f = fx(&[]);
    assert!(landscape(&f.doc, f.page));
    let page = f.page;
    assert!(f.edit(|tx, eng| set_orientation(tx, eng, page, false)));
    assert!(close(f.page_cell("PageWidth"), A4[0]));
    assert!(close(f.page_cell("PageHeight"), A4[1]));
    assert!(close(f.page_cell("PrintPageOrientation"), 1.0));
    assert!(!landscape(&f.doc, page));
    // already portrait: nothing to do
    let mut tx = f.doc.transaction("t");
    assert!(!set_orientation(&mut tx, &mut f.eng, page, false).unwrap());
    assert!(tx.commit().is_none());
    assert!(f.edit(|tx, eng| set_orientation(tx, eng, page, true)));
    assert!(close(f.page_cell("PageWidth"), A4[1]));
}

#[test]
fn a_size_of_the_menu_keeps_the_orientation_and_the_scale() {
    // 1:10 (PageScale 1 in = DrawingScale 10 in)
    let mut f = fx(&[("DrawingScale", 10.0), ("PageWidth", A4[1] * 10.0), ("PageHeight", A4[0] * 10.0)]);
    let page = f.page;
    let a3 = [297.0 / 25.4, 420.0 / 25.4];
    assert!(f.edit(|tx, eng| set_paper_size(tx, eng, page, a3)));
    assert!(close(f.page_cell("PageWidth"), a3[1] * 10.0));
    assert!(close(f.page_cell("PageHeight"), a3[0] * 10.0));
    assert!(close(f.page_cell("DrawingSizeType"), 2.0));
    assert!(!auto_size(&f.doc, page), "a fixed size turns the automatic size off");
}

#[test]
fn the_automatic_size_follows_visios_cells() {
    for (resize, size, on) in [(1.0, 3.0, true), (2.0, 0.0, false), (0.0, 0.0, true), (0.0, 3.0, false)] {
        let f = fx(&[("DrawingResizeType", resize), ("DrawingSizeType", size)]);
        assert_eq!(auto_size(&f.doc, f.page), on, "DrawingResizeType {resize}, DrawingSizeType {size}");
    }
    let mut f = fx(&[]);
    let page = f.page;
    assert!(f.edit(|tx, _| set_auto_size(tx, page, false)));
    assert!(close(f.page_cell("DrawingResizeType"), 2.0));
    assert!(f.edit(|tx, _| set_auto_size(tx, page, true)));
    assert!(close(f.page_cell("DrawingResizeType"), 1.0));
}

#[test]
fn fit_to_drawing_moves_the_shapes_onto_the_page() {
    let mut f = fx(&[]);
    let a = f.rect([3.0, 3.0, 4.0, 4.0]);
    let b = f.rect([6.0, 3.5, 7.0, 4.5]);
    let c = f.connector(a, b);
    let page = f.page;
    let m = [0.25, 0.5, 0.25, 0.5];
    assert!(f.edit(|tx, eng| fit_to_drawing(tx, eng, page, m)));
    // the box 3..7 x 3..4.5 with the margins: 4.5 x 2.5, starting at the left and bottom margin
    assert!(close(f.page_cell("PageWidth"), 4.5));
    assert!(close(f.page_cell("PageHeight"), 2.5));
    assert!(close(f.page_cell("DrawingSizeType"), 1.0));
    let p = f.pin(a);
    assert!(close(p[0], 0.75) && close(p[1], 1.0), "{p:?}");
    let bx = content(&f.doc, page).unwrap();
    assert!(close(bx[0], 0.25) && close(bx[1], 0.5) && close(bx[2], 4.25) && close(bx[3], 2.0), "{bx:?}");
    // the glued connector came along
    assert!(close(f.value(SheetRef::Shape(c), "BeginX"), 1.25));
    assert!(!auto_size(&f.doc, page));
}

#[test]
fn the_tiles_agree_with_the_corpus() {
    // Zeichnung2.vsdx: A4 landscape, 0.25 in margins, two by two tiles -> 22.886 x 16.035 in
    let t = Tiles::new([A4[1], A4[0]], [0.25; 4], 1.0);
    assert!(close(t.base[0] + t.step[0], 22.885_826_771_653_54));
    assert!(close(t.base[1] + t.step[1], 16.035_433_070_866_14));
    let z = Tiles::new([A4[1], A4[0]], [0.25; 4], 2.0);
    assert!(close(z.base[0], A4[1] / 2.0));
}

fn tiles() -> Tiles {
    Tiles::new([A4[1], A4[0]], [0.25; 4], 1.0)
}

#[test]
fn the_page_grows_by_tiles_to_the_right_and_up() {
    let mut f = fx(&[]);
    let a = f.rect([10.0, 7.0, 13.0, 9.0]);
    let page = f.page;
    let t = tiles();
    let d = f.edit(|tx, eng| grow(tx, eng, page, &t));
    assert_eq!(d, Some([0.0, 0.0]), "growing right and up moves nothing");
    assert!(close(f.page_cell("PageWidth"), t.base[0] + t.step[0]));
    assert!(close(f.page_cell("PageHeight"), t.base[1] + t.step[1]));
    assert!(close(f.pin(a)[0], 11.5));
    // fits now: nothing to do
    let mut tx = f.doc.transaction("t");
    assert_eq!(grow(&mut tx, &mut f.eng, page, &t).unwrap(), None);
    assert!(tx.commit().is_none());
}

#[test]
fn the_page_grows_to_the_left_and_down_and_moves_the_drawing() {
    let mut f = fx(&[]);
    let a = f.rect([-1.0, -0.5, 1.0, 0.5]);
    let b = f.rect([5.0, 3.0, 6.0, 4.0]);
    let c = f.connector(a, b);
    let page = f.page;
    let t = tiles();
    let d = f.edit(|tx, eng| grow(tx, eng, page, &t)).unwrap();
    assert!(close(d[0], t.step[0]) && close(d[1], t.step[1]), "{d:?}");
    assert!(close(f.page_cell("PageWidth"), t.base[0] + t.step[0]));
    assert!(close(f.page_cell("PageHeight"), t.base[1] + t.step[1]));
    let p = f.pin(b);
    assert!(close(p[0], 5.5 + t.step[0]) && close(p[1], 3.5 + t.step[1]), "{p:?}");
    assert!(close(f.value(SheetRef::Shape(c), "EndX"), 5.0 + t.step[0]), "the glued end follows");
}

#[test]
fn the_page_shrinks_back_when_a_tile_is_empty() {
    let mut f = fx(&[]);
    let t = tiles();
    let w2 = t.base[0] + t.step[0];
    let mut f2 = fx(&[("PageWidth", w2), ("PageHeight", t.base[1] + 2.0 * t.step[1])]);
    // the drawing in the upper half of three tiles high: the lowest tile goes, the drawing
    // moves down by a tile; the right tile goes
    let a = f2.rect([1.0, 2.0 * t.step[1] + 1.0, 2.0, 2.0 * t.step[1] + 2.0]);
    let page = f2.page;
    let d = f2.edit(|tx, eng| grow(tx, eng, page, &t)).unwrap();
    assert!(close(d[0], 0.0) && close(d[1], -2.0 * t.step[1]), "{d:?}");
    assert!(close(f2.page_cell("PageWidth"), t.base[0]));
    assert!(close(f2.page_cell("PageHeight"), t.base[1]));
    assert!(close(f2.pin(a)[1], 1.5));
    // an empty page: back to one tile, nothing moves
    let page = f.page;
    f.edit(|tx, _| put_size(tx, page, w2, t.base[1]));
    assert_eq!(f.edit(|tx, eng| grow(tx, eng, page, &t)), Some([0.0, 0.0]));
    assert!(close(f.page_cell("PageWidth"), t.base[0]));
}

#[test]
fn a_page_off_the_grid_stays_while_its_drawing_fits() {
    // A3 landscape while the paper is A4: no empty tile to take away, the drawing fits
    let mut f = fx(&[("PageWidth", 420.0 / 25.4), ("PageHeight", 297.0 / 25.4)]);
    f.rect([14.0, 9.0, 15.0, 10.0]);
    let page = f.page;
    let t = tiles();
    let mut tx = f.doc.transaction("t");
    assert_eq!(grow(&mut tx, &mut f.eng, page, &t).unwrap(), None);
    drop(tx);
    // automatic size off: nothing, whatever juts out
    let mut g = fx(&[("DrawingResizeType", 2.0)]);
    g.rect([20.0, 1.0, 21.0, 2.0]);
    let page = g.page;
    let mut tx = g.doc.transaction("t");
    assert_eq!(grow(&mut tx, &mut g.eng, page, &t).unwrap(), None);
}
