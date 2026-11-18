// stencils_drop -- JD9: shapes from the Shapes panel onto the page (FEATURES SL-03, MS-02,
// LG-04): dragged and dropped, click + click (the tile armed, then a click on the page), Enter
// on a tile (the middle of the view)
//
//   DT_SHAPE                    the drag kind of the panel's tiles (stencil key, tile index)
//   tool(app)                   the canvas' tool: DG_PLACE while a tile is armed
//   canvas(app, ui, id, &events)   after ui.diagram: the canvas as the tiles' drop target, the
//                               click of DG_PLACE, Esc disarms
//   place_center(app, ui, key, k)   the shape in the middle of the view
//
// Each drop is one transaction of the drawing (one undo step once JD7 brings the history):
// the master is taken into the drawing if it is not there (instance::import_master, matched
// by UniqueID), the instance goes to the point (instance::place: Name in the current language,
// NameU the master's), the formula engine recalculates the new shapes, and the page is drawn
// again.  Until JD7 makes the empty drawing a document of its own, it becomes one as soon as a
// stencil is open (ensure_document: vsdx::new_document in the region's units, viewer::adopt),
// so the page is shown and fitted before the first shape arrives.
//
// Coordinates: the canvas' world unit is a DIP at 100 %, 96 of them a paper inch, y down from
// the page's top left corner (viewer.rs); the page's inches have y up from the bottom.

use alloc::vec::Vec;

use diagram_core::instance;
use diagram_core::{DocKind, ShapeRef, SheetRef};
use diagram_io::vsdx;
use ui::i18n::{t, tf};
use ui::*;

use crate::viewer::DIP;
use crate::Diagrammer;

/// the drag kind of the panel's tiles
pub(crate) const DT_SHAPE: u32 = DT_USER0 << 2;

/// the canvas' tool: placing while a tile is armed (SL-03)
pub(crate) fn tool(app: &Diagrammer) -> u32 {
    if app.stencils.armed.is_some() { DG_PLACE } else { DG_SELECT }
}

/// after ui.diagram (its rectangle is the last one): drops of tiles, the click of an armed
/// tile, Esc
pub(crate) fn canvas(app: &mut Diagrammer, ui: &mut Ui, id: u32, events: &[DEv]) {
    if ui.drop_target(id, DT_SHAPE) == DS_DROP {
        let d = ui.drop_data();
        if d.app & DT_SHAPE != 0 {
            if let Some((key, k)) = stencil_drag_decode(&d.bytes) {
                let (x, y) = ui.drop_pos_px();
                let at = app.canvas.vp.to_world(x as f32, y as f32);
                place_world(app, ui, key, k as usize, at);
            }
        }
    }
    let Some((key, k)) = app.stencils.armed else { return };
    for ev in events {
        if let DEv::Click { at } = ev {
            place_world(app, ui, key, k, *at);
            // Shift keeps the tile armed for the next shape
            if ui.mods() & MOD_SHIFT == 0 {
                app.stencils.armed = None;
            }
            ui.repass();
        }
    }
    if app.stencils.armed.is_some() && ui.key(VK_ESCAPE) {
        app.stencils.armed = None;
        ui.repass();
    }
}

/// the shape of tile (key, k) in the middle of the view (Enter on a tile)
pub(crate) fn place_center(app: &mut Diagrammer, ui: &mut Ui, key: u32, k: usize) {
    let vp = &app.canvas.vp;
    let at = [vp.cx as f32, vp.cy as f32];
    place_world(app, ui, key, k, at);
}

// the shape of tile (key, k) at world point `at`
fn place_world(app: &mut Diagrammer, ui: &mut Ui, key: u32, k: usize, at: [f32; 2]) {
    let Some(page) = ensure_document(app, ui) else { return };
    let lang = crate::stencils::lang();
    let st = &app.stencils;
    let Some((l, e)) = st.entry(key, k) else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let h = o.doc.page(page).and_then(|p| o.doc.strings().get("PageHeight").and_then(|s| p.sheet.cell(s))).and_then(|c| c.value.num()).unwrap_or(8.5);
    let (x, y) = (at[0] as f64 / DIP, h - at[1] as f64 / DIP);
    let name = e.texts.name(lang);
    let mut tx = o.doc.transaction(&tf("{} ablegen", &[name]));
    let placed = (|| {
        let m = instance::import_master(&mut tx, &l.doc, e.master)?;
        let shape = instance::place(&mut tx, page, m, x, y, lang)?;
        // the new shapes' values (their formulas come from the master)
        let mut sheets = Vec::new();
        collect(tx.doc(), shape, &mut sheets);
        diagram_core::formula::Engine::new().recalc(&mut tx, &sheets)?;
        Ok::<ShapeRef, diagram_core::EditError>(shape)
    })();
    match placed {
        Ok(shape) => {
            let said = tx.doc().shape(shape).map(|s| s.name()).unwrap_or_default();
            tx.commit();
            crate::viewer::changed(app);
            // screen readers (and scripts) hear what was dropped
            ui.announce(&tf("„{}“ abgelegt", &[&said]), LIVE_POLITE);
        }
        Err(err) => {
            drop(tx);
            app.stencils.error = Some(tf("Das Shape lässt sich nicht ablegen: {}", &[&alloc::format!("{err:?}")]));
        }
    }
    ui.repass();
}

// a shape and its subshapes as sheets
fn collect(doc: &diagram_core::Document, s: ShapeRef, out: &mut Vec<SheetRef>) {
    out.push(SheetRef::Shape(s));
    if let Some(sh) = doc.shape(s) {
        for c in sh.children() {
            collect(doc, c, out);
        }
    }
}

/// the page shapes go to: the page shown; the empty drawing becomes a document first (the
/// panel calls it as soon as a stencil is open, so the page is there before the first drop)
pub(crate) fn ensure_document(app: &mut Diagrammer, ui: &mut Ui) -> Option<diagram_core::PageRef> {
    if app.opened.is_none() {
        let mut doc = vsdx::new_document(DocKind::Drawing, crate::stencils::measure(app));
        // the page's name in the current language, NameU stays Page-1 (LG-04)
        let first = doc.pages().next();
        if let Some(p) = first {
            let mut tx = doc.transaction(t("Seite umbenennen"));
            if tx.rename_page(p, &tf("Seite-{}", &["1"]), "Page-1").is_ok() {
                tx.commit();
            }
        }
        crate::viewer::adopt(app, ui, doc, &tf("Zeichnung{}", &["1"]));
    }
    let o = app.opened.as_ref()?;
    let tab = o.tabs.get(app.page_tab.max(0) as usize).or(o.tabs.first())?;
    Some(tab.page)
}
