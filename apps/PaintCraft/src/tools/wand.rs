// t_wand -- PaintCraft: Zauberstab (magic wand, agent T2).  Sub-module of sel.rs.
//                                              [apps/PaintCraft/src/t_wand.s]
//
// A click selects the pixels of similar colour around the clicked one: flood_fill_mask with
// paint.net's tolerance (Toleranz), Füllungsmodus zusammenhängend / global (Umschalt =
// global), sampled from the current layer or the whole image (Ebene / Bild).  The region's
// pixel-edge outline (tm_trace) becomes a polygon op, combined like the other selection tools
// (Auswahlmodus, Strg / Alt / right button: tsel_mode), history step "Zauberstab".  Like
// paint.net 5 the result stays editable: an origin handle can be dragged and changing the
// tolerance / mode / sampling re-runs the fill (same history step) until the selection
// changes otherwise, the pixels change or another tool is chosen.
//
//   twand_init()                  register TOOL_MAGIC_WAND
//   twand_state() -> text         test record: "x,y" of the editable origin or ""

use alloc::format;
use alloc::string::String;

use gfx::flood::{flood_fill_mask, FF_GLOBAL};
use gfx::tmask::TMask;
use gfx::Surface;
use ui::*;

use crate::document::composite::{comp_sf, comp_update};
use crate::app::constants::*;
use crate::document::model::{doc_active, doc_cur_layer, DocId};
use crate::document::history::hist_push_sel;
use crate::workspace::nubs::{nub_set, nubs_damage, nubs_draw, nubs_hit, nubs_hover, Nub};
use crate::document::selection::{sel_combine, sel_edit_last, sel_poly_trace, sel_poly_unref, PolyRef};
use crate::tools::select::tsel_mode;
use crate::tools::*;
use crate::app::Pn;

/// the state of t_wand.s (its g_* globals)
#[derive(Default)]
pub(crate) struct TWand {
    doc: Option<DocId>,
    /// flood result (image size)
    mask: Option<TMask>,
    nub: Nub,
    /// the last wand op is editable
    ed: bool,
    /// the origin is dragged
    drag: bool,
    /// origin (seed pixel)
    x: i32,
    y: i32,
    shift: u32,
    /// TC_VER of the last run
    cver: u32,
    /// document versions after the last run
    vsel: u32,
    vpx: u32,
    vlay: u32,
    vcur: i32,
}

static DESC_WAND: ToolDesc = ToolDesc {
    flags: (TF_SELMODE | TF_TOL | TF_SAMPLE) as u32,
    deactivate: Some(tw_deactivate),
    input: Some(tw_input),
    options: Some(tw_options),
    cursor: Some(tw_cursor),
    docswitch: Some(tw_docswitch),
    overlay: Some(tw_overlay),
    tick: Some(tw_tick),
    ..ToolDesc::NONE
};

/// twand_init()
pub(crate) fn twand_init(p: &mut Pn, _ui: &mut Ui) {
    tool_register(&mut p.tool, TOOL_MAGIC_WAND, Some(&DESC_WAND));
}

/// twand_state() -> "x,y" of the editable origin, "" when none
pub(crate) fn twand_state(p: &Pn) -> String {
    if !tw_editable(p, doc_active(&p.doc)) {
        return String::new();
    }
    format!("{},{}", p.twand.x, p.twand.y)
}

// tw_editable(doc) -> the last wand selection can still be changed
fn tw_editable(p: &Pn, doc: Option<DocId>) -> bool {
    let t = &p.twand;
    if !t.ed || doc.is_none() || doc != t.doc {
        return false;
    }
    let Some(d) = p.doc.get(doc.unwrap()) else { return false };
    d.ver_sel == t.vsel && d.ver_px == t.vpx && d.ver_lay == t.vlay && d.cur == t.vcur && tool_cur(&p.tool) == TOOL_MAGIC_WAND
}

// tw_hide(): the origin handle disappears
fn tw_hide(p: &mut Pn, ui: &mut Ui) {
    let t = &mut p.twand;
    if !t.ed {
        return;
    }
    t.ed = false;
    t.drag = false;
    t.nub.flags &= !(NBF_DRAG as u32);
    nubs_damage(&p.view, ui, core::slice::from_mut(&mut t.nub));
}

// tw_remember(doc): versions of the document after a wand change
fn tw_remember(p: &mut Pn, doc: DocId) {
    let d = p.doc.d(doc);
    let t = &mut p.twand;
    t.vsel = d.ver_sel;
    t.vpx = d.ver_px;
    t.vlay = d.ver_lay;
    t.vcur = d.cur;
}

// tw_run(doc) -> POLY of the region around (x, y) | None
fn tw_run(p: &mut Pn, doc: DocId) -> Option<PolyRef> {
    let c = p.tool.cfg;
    // the sampled pixels
    let image = c.sample == SMP_IMAGE as u32;
    if image {
        comp_update(p, doc);
    }
    let d = p.doc.d(doc);
    let src: &Surface = if image { comp_sf(d) } else { &doc_cur_layer(d)?.sf };
    if src.px.is_empty() {
        return None;
    }
    // a mask of the image size
    let t = &mut p.twand;
    if !matches!(&t.mask, Some(m) if m.w as i32 == src.w && m.h as i32 == src.h) {
        t.mask = None;
        t.mask = TMask::new(src.w as u32, src.h as u32);
    }
    let m = t.mask.as_mut()?;
    let fl = (c.flood | t.shift) & FF_GLOBAL;
    if flood_fill_mask(src, t.x, t.y, c.tol as i32, fl, m) != 0 {
        return None;
    }
    sel_poly_trace(m)
}

// tw_rerun(): the editable wand op follows a new origin / new options
fn tw_rerun(p: &mut Pn, ui: &mut Ui) {
    let Some(doc) = p.twand.doc else { return };
    let Some(poly) = tw_run(p, doc) else { return };
    sel_edit_last(p, doc, &poly);
    sel_poly_unref(Some(poly));
    tw_remember(p, doc);
    p.twand.cver = p.tool.cfg.ver;
    // an empty result (not possible: the seed matches itself) would have cleared it
    if p.doc.d(doc).sel.nops == 0 {
        tw_hide(p, ui);
    }
}

// tw_input(doc, ui, x_fx, y_fx) -> changed
fn tw_input(p: &mut Pn, ui: &mut Ui, doc: DocId, inp: &UiInput, x_fx: i32, y_fx: i32) -> bool {
    let sx = inp.x + inp.rect[0];
    let sy = inp.y + inp.rect[1];
    if p.twand.drag {
        // dragging the origin
        if Some(doc) == p.twand.doc && inp.btn & (1 << BTN_LEFT) != 0 {
            let d = p.doc.d(doc);
            let x = (x_fx >> 8).max(0).min(d.w - 1);
            let y = (y_fx >> 8).max(0).min(d.h - 1);
            if x == p.twand.x && y == p.twand.y {
                return false;
            }
            p.twand.x = x;
            p.twand.y = y;
            tw_nub_update(p, ui);
            tw_rerun(p, ui);
            return true;
        }
        let t = &mut p.twand;
        t.drag = false;
        t.nub.flags &= !(NBF_DRAG as u32);
        nubs_damage(&p.view, ui, core::slice::from_mut(&mut t.nub));
        return true;
    }
    let btn = if inp.pressed & (1 << BTN_LEFT) != 0 {
        BTN_LEFT
    } else if inp.pressed & (1 << BTN_RIGHT) != 0 {
        BTN_RIGHT
    } else {
        if !tw_editable(p, Some(doc)) {
            return false;
        }
        return nubs_hover(&p.view, ui, core::slice::from_mut(&mut p.twand.nub), sx, sy);
    };
    if btn == BTN_LEFT && tw_editable(p, Some(doc)) && nubs_hit(&p.view, ui, core::slice::from_ref(&p.twand.nub), sx, sy) >= 0 {
        let t = &mut p.twand;
        t.drag = true;
        t.nub.flags |= NBF_DRAG as u32;
        nubs_damage(&p.view, ui, core::slice::from_mut(&mut t.nub));
        return true;
    }
    // a click inside the image: a new wand op
    let x = x_fx >> 8;
    let y = y_fx >> 8;
    let d = p.doc.d(doc);
    if x as u32 >= d.w as u32 || y as u32 >= d.h as u32 {
        return false;
    }
    tw_hide(p, ui);
    let t = &mut p.twand;
    t.doc = Some(doc);
    t.x = x;
    t.y = y;
    t.shift = (inp.mods & MOD_SHIFT != 0) as u32; // Umschalt: global (FF_GLOBAL = 1)
    let mode = tsel_mode(p, inp, btn);
    let Some(poly) = tw_run(p, doc) else { return true };
    if hist_push_sel(p, doc, "Zauberstab", IC_MAGIC_SELECT) {
        sel_combine(p, doc, &poly, mode, SQ_AA as u32);
        if p.doc.d(doc).sel.nops != 0 {
            tw_remember(p, doc);
            p.twand.cver = p.tool.cfg.ver;
            p.twand.ed = true;
            p.twand.nub.kind = NK_CIRCLE as u32;
            p.twand.nub.flags = 0;
            tw_nub_update(p, ui);
        }
    }
    sel_poly_unref(Some(poly));
    true
}

// tw_nub_update(): the origin handle on the centre of the seed pixel
fn tw_nub_update(p: &mut Pn, ui: &mut Ui) {
    let t = &mut p.twand;
    nub_set(&p.view, ui, &mut t.nub, (t.x << 8) + 128, (t.y << 8) + 128);
    nubs_damage(&p.view, ui, core::slice::from_mut(&mut t.nub)); // (shown again at an unchanged place)
}

// tw_tick(doc): option changes re-run the editable op; the handle goes when it no longer
// applies
fn tw_tick(p: &mut Pn, ui: &mut Ui, doc: Option<DocId>) {
    if !p.twand.ed {
        return;
    }
    if !tw_editable(p, doc) {
        tw_hide(p, ui);
    } else if p.tool.cfg.ver != p.twand.cver {
        tw_rerun(p, ui);
    }
}

// tw_deactivate(doc) / tw_docswitch(old, new)
fn tw_deactivate(p: &mut Pn, ui: &mut Ui, _doc: Option<DocId>) {
    tw_hide(p, ui);
}

fn tw_docswitch(p: &mut Pn, ui: &mut Ui, _old: Option<DocId>, _new: Option<DocId>) {
    tw_hide(p, ui);
}

// tw_overlay(doc, &rect): the origin handle (hidden with damage by tw_tick)
fn tw_overlay(p: &mut Pn, g: &mut Gfx, doc: DocId, _rect: [i32; 4]) {
    if p.twand.ed && Some(doc) == p.twand.doc {
        nubs_draw(&p.view, g, core::slice::from_mut(&mut p.twand.nub));
    }
}

// tw_cursor(x_fx, y_fx) -> CUR_MOVE over / while dragging the origin, else the cross
fn tw_cursor(p: &mut Pn, _x: i32, _y: i32) -> u32 {
    if p.twand.ed && p.twand.nub.flags & (NBF_HOVER | NBF_DRAG) as u32 != 0 {
        return CUR_MOVE;
    }
    CUR_CROSS
}

// tw_options(): Auswahlmodus, Füllungsmodus, Toleranz, Ebene / Bild
fn tw_options(p: &mut Pn, ui: &mut Ui) {
    opt_selmode(p, ui);
    opt_sep(ui);
    opt_flood(p, ui);
    opt_sep(ui);
    opt_tolerance(p, ui);
    opt_sep(ui);
    opt_sampling(p, ui);
}
