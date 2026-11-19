// format_quick -- (JD11) Schnellformatvorlagen (TH-06; RS-19's gallery in a small form): the
// quick styles of the page's design for the selection, opened from Shape formatieren (the
// Start register has no room for a gallery at the window's usual width)
//
//   flyout(app, ui)        the menu's content: the variant's four styles (QuickStyle 100 .. 103
//                          for colour and matrices), then the matrix of six intensities (the
//                          style lists' entries 1 .. 6) by seven colours (dark 1, accents 1 .. 6),
//                          each tile a box drawn in that quick style; a click sets the eight
//                          QuickStyle cells of the selected shapes (one undo step)
//
// The tiles are drawn the way the page is: a small document with the page's design part and
// variant, one box whose QuickStyle cells are set for the tile, diagram_view; kept per design,
// variant and size.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::design::apply::{self, Scope};
use diagram_core::formula::Engine;
use diagram_core::ops::{self, Basic};
use diagram_core::render::Options;
use diagram_core::{DocKind, SheetRef, Value};
use diagram_io::vsdx::{self, Measure};
use diagram_view::PageView;
use gfx::Surface;
use ui::i18n::t;
use ui::*;

use crate::format_cells::{self as fc, F};
use crate::Diagrammer;

const ID: u32 = 0x5EA0_0000;

/// the quick styles' tiles, kept
#[derive(Default)]
pub(crate) struct QuickTiles {
    key: (u32, u32, i32),
    tiles: Vec<Rc<Surface>>,
}

// colour, matrix of every tile: the variant's four styles, then 6 x 7
fn quick_list() -> Vec<(u32, u32)> {
    let mut v: Vec<(u32, u32)> = (0..4).map(|k| (100 + k, 100 + k)).collect();
    for m in 1..=6 {
        for c in [0, 2, 3, 4, 5, 6, 7] {
            v.push((c, m));
        }
    }
    v
}

// the tiles of the page's design and variant, w x h pixels each
fn tiles(app: &mut Diagrammer, w: i32, h: i32) -> Option<Vec<Rc<Surface>>> {
    let (id, var) = crate::design_gallery::current(app)?;
    if app.quick.key == (id, var, w) && !app.quick.tiles.is_empty() {
        return Some(app.quick.tiles.clone());
    }
    let part = {
        let o = app.opened.as_ref()?;
        o.doc.themes.get(apply::find_part(&o.doc, id)?)?.data.clone()
    };
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Us);
    let page = doc.pages().next()?;
    let mut eng = Engine::new();
    let mut tx = doc.transaction("");
    let ps = SheetRef::Page(page);
    for (n, v) in [("PageWidth", 1.0), ("PageHeight", 0.75)] {
        let a = tx.single(n);
        tx.force_constant(ps, &a, Value::Num(v), None).ok()?;
    }
    apply::apply(&mut tx, &mut eng, id, &part, Scope::Page(page), var).ok()?;
    let s = ops::new_shape(&mut tx, &mut eng, diagram_core::Owner::Page(page), Basic::Rectangle, [0.12, 0.12], [0.88, 0.63]).ok()?;
    let _ = tx.commit();
    let mut fonts = app.fonts.clone();
    let mut out = Vec::new();
    for (c, m) in quick_list() {
        let mut tx = doc.transaction("");
        for (n, v) in [
            ("QuickStyleLineColor", c),
            ("QuickStyleFillColor", c),
            ("QuickStyleShadowColor", c),
            ("QuickStyleFontColor", c),
            ("QuickStyleLineMatrix", m),
            ("QuickStyleFillMatrix", m),
            ("QuickStyleEffectsMatrix", m),
            ("QuickStyleFontMatrix", m),
        ] {
            let a = tx.single(n);
            let _ = tx.force_constant(SheetRef::Shape(s), &a, Value::Num(v as f64), None);
        }
        let sf = {
            let d = tx.doc();
            let mut v = PageView::new(d, page, &mut fonts, &Options::default());
            Rc::new(v.render(w.max(8) as u32, h.max(8) as u32))
        };
        tx.rollback();
        out.push(sf);
    }
    app.quick = QuickTiles { key: (id, var, w), tiles: out.clone() };
    Some(out)
}

fn set(app: &mut Diagrammer, ui: &mut Ui, c: u32, m: u32) {
    let v = |x: u32| format!("{x}");
    // the formatting by hand gives way, as in Visio (fonts and sizes stay)
    let clear: Vec<F> = ["LineColor", "LineWeight", "LinePattern", "LineColorTrans", "FillForegnd", "FillBkgnd", "FillPattern", "FillForegndTrans", "ShdwForegnd", "ShdwPattern", "ShdwForegndTrans"]
        .into_iter()
        .map(F::Cell)
        .chain([F::Char("Color")])
        .collect();
    fc::restyle(
        app,
        ui,
        t("Schnellformatvorlage"),
        &clear,
        &[
            (F::Cell("QuickStyleLineColor"), v(c)),
            (F::Cell("QuickStyleFillColor"), v(c)),
            (F::Cell("QuickStyleShadowColor"), v(c)),
            (F::Cell("QuickStyleFontColor"), v(c)),
            (F::Cell("QuickStyleLineMatrix"), v(m)),
            (F::Cell("QuickStyleFillMatrix"), v(m)),
            (F::Cell("QuickStyleEffectsMatrix"), v(m)),
            (F::Cell("QuickStyleFontMatrix"), v(m)),
        ],
    );
}

/// the menu's content (in the open flyout)
pub(crate) fn flyout(app: &mut Diagrammer, ui: &mut Ui) {
    let (w, h) = (ui.dp(40), ui.dp(30));
    let Some(list) = tiles(app, w, h) else {
        ui.text(TS_CAPTION | TX_SECONDARY, t("Die Seite hat kein Design."));
        return;
    };
    let cur = (fc::num(app, F::Cell("QuickStyleFillColor")).map(|v| v as u32), fc::num(app, F::Cell("QuickStyleFillMatrix")).map(|v| v as u32));
    let quick = quick_list();
    let tile = |app: &mut Diagrammer, ui: &mut Ui, k: usize, name: String| {
        let (c, m) = quick[k];
        ui.next_size(44, 36);
        ui.next_flags(if cur == (Some(c), Some(m)) { ST_CHECKED } else { 0 });
        if ui.ribbon_image_tile(ID + k as u32, &name, &list[k], k as u32) & EV_ACT != 0 {
            set(app, ui, c, m);
            ui.popup_close();
        }
    };
    ui.text(TS_CAPTION | TX_SECONDARY, t("Variantenstile"));
    ui.box_begin(0, BOX_H, 2);
    for k in 0..4 {
        tile(app, ui, k, format!("{} {}", t("Variantenstil"), k + 1));
    }
    ui.box_end();
    ui.spacer(4);
    ui.text(TS_CAPTION | TX_SECONDARY, t("Designstile"));
    let colors = [t("Dunkel"), t("Akzent 1"), t("Akzent 2"), t("Akzent 3"), t("Akzent 4"), t("Akzent 5"), t("Akzent 6")];
    for row in 0..6 {
        ui.box_begin(0, BOX_H, 2);
        for col in 0..7 {
            let k = 4 + row * 7 + col;
            tile(app, ui, k, format!("{}, {} {}", colors[col], t("Stärke"), row + 1));
        }
        ui.box_end();
    }
}
