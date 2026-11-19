// stencils_drop -- JD9: shapes from the Shapes panel onto the page (FEATURES SL-03, MS-02,
// LG-04): dragged and dropped, click + click (the tile armed, then a click on the page), Enter
// on a tile (the middle of the view)
//
//   DT_SHAPE                    the drag kind of the panel's tiles (stencil key, tile index)
//   drop_target(app, ui, id)    (edit_canvas::canvas, right after ui.diagram) the canvas as the
//                               tiles' drop target; Esc disarms an armed tile
//   clicked(app, ui, at)        (edit_canvas::apply) the click of an armed tile: ui.shape_edit
//                               runs SE_PLACE while one is armed (edit_canvas::tool_of)
//   place_center(app, ui, key, k)   the shape in the middle of the view
//
// Each drop is one edit of the drawing (edit_doc::edit, JD7: one undo step, the drawing's
// formula engine): the master is taken into the drawing if it is not there
// (instance::import_master, matched by UniqueID), the instance goes to the point
// (instance::place: Name in the current language, NameU the master's), the engine
// recalculates the new shapes, connectors follow (edit_canvas::follow); then the new shape is
// selected and viewer::sync draws the page again (a new master: the whole page).
//
// Coordinates: world points of the canvas (DIP at 100 %, y down) become the page's inches with
// y up through edit_canvas's Geo (app.model.geo, which knows the drawing scale).

use alloc::vec::Vec;

use diagram_core::instance;
use diagram_core::{Owner, ShapeRef, SheetRef};
use ui::i18n::tf;
use ui::*;

use crate::edit_canvas;
use crate::Diagrammer;

/// the drag kind of the panel's tiles
pub(crate) const DT_SHAPE: u32 = DT_USER0 << 2;

/// right after ui.diagram (its rectangle is the last one): drops of tiles; Esc disarms
pub(crate) fn drop_target(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
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
    if app.stencils.armed.is_some() && ui.key(VK_ESCAPE) {
        app.stencils.armed = None;
        ui.repass();
    }
}

/// the click of an armed tile at world point `at`; Shift keeps the tile armed for the next one
pub(crate) fn clicked(app: &mut Diagrammer, ui: &mut Ui, at: [f32; 2]) {
    let Some((key, k)) = app.stencils.armed else { return };
    place_world(app, ui, key, k, at);
    if ui.mods() & MOD_SHIFT == 0 {
        app.stencils.armed = None;
    }
    ui.repass();
}

/// the shape of tile (key, k) in the middle of the view (Enter on a tile)
pub(crate) fn place_center(app: &mut Diagrammer, ui: &mut Ui, key: u32, k: usize) {
    let vp = &app.canvas.vp;
    let at = [vp.cx as f32, vp.cy as f32];
    place_world(app, ui, key, k, at);
}

// the shape of tile (key, k) at world point `at`
fn place_world(app: &mut Diagrammer, ui: &mut Ui, key: u32, k: usize, at: [f32; 2]) {
    let (Some(geo), Some(page)) = (app.model.geo, app.model.page) else { return };
    let [x, y] = geo.page(at);
    let lang = crate::stencils::lang();
    let st = &app.stencils;
    let Some((l, e)) = st.entry(key, k) else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let label = tf("{} ablegen", &[e.texts.name(lang)]);
    // (JD10) the structure masters: a dropped lane may bring its flowchart
    let masters = crate::structures::masters(st);
    let placed = crate::edit_doc::edit(o, &label, true, |tx, eng| {
        let m = instance::import_master(tx, &l.doc, e.master)?;
        let shape = instance::place(tx, page, m, x, y, lang)?;
        // the new shapes' values (their formulas come from the master), connectors that follow
        let mut sheets = Vec::new();
        collect(tx.doc(), shape, &mut sheets);
        eng.recalc(tx, &sheets)?;
        edit_canvas::follow(tx, eng, Owner::Page(page), &[shape])?;
        // (JD10) it joins the containers it lies in, a callout ties to the shape under it
        crate::structures::dropped(tx, eng, &masters, Owner::Page(page), shape)?;
        Ok(shape)
    });
    match placed {
        Ok(shape) => {
            let said = app.opened.as_ref().and_then(|o| o.doc.shape(shape)).map(|s| s.name()).unwrap_or_default();
            app.se.select(&[edit_canvas::key_of(shape)]);
            // screen readers (and scripts) hear what was dropped
            ui.announce(&tf("„{}“ abgelegt", &[&said]), LIVE_POLITE);
        }
        Err(err) => {
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
