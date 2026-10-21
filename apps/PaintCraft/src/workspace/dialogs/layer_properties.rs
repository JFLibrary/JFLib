// dlg_layerprops -- PaintCraft: "Ebeneneigenschaften" (Ebenen > Ebeneneigenschaften…, F4,
// double click on a row of the layers window) as a modal dialog window like paint.net 5:
//   Allgemein      Name (text box), Sichtbar (check box)
//   Mischmodus:    the 14 paint.net blend modes (LayerBlendMode.* German names)
//   Deckfähigkeit  0..255 (slider + number box)
// Every change shows at once on the layer (live preview, no history).  OK records ONE
// history step "Ebeneneigenschaften" whose other state is the values from before the
// dialog (the layer is set back, hist_push_layer_props, the new values applied); nothing
// changed = no step.  Abbrechen / Esc / the close button restore the old values and the
// image's modified flag.  Sub-module of layer.rs (agent A4).
//                                              [apps/PaintCraft/src/dlg_layerprops.s]
//
//   lprops_open(doc, i)     show the dialog for layer i (ignored while one is open)
//   lprops_frame()          the window (lay_frame calls it every frame)
//   lprops_done(ok)         CMD_LAY_PROPS_DONE: commit (ok = true) or restore
//   lprops_is_open()        while the dialog is open
// Window WINID_LPROPS, widgets ID_LP_* (pn_lay.inc).

use alloc::string::String;

use gfx::blendmode::BM_XOR;
use ui::*;

use crate::app::constants::*;
use crate::document::model::{doc_index, doc_set_modified, DocId};
use crate::document::layers::{lay_find_uid, lay_rec, lay_set_blend, lay_set_name, lay_set_opacity, lay_set_visible};
use crate::app::formatting::fmt_cap;
use crate::app::Pn;

const LBL_W: i32 = 64; // "Name" column (DIP)
const RANGE_OPAC: Range = Range::new(0, 255, 1, 0);
// paint.net LayerBlendMode.* (German), in BM_* order
static BLEND_LIST: [&str; 14] = [
    "Normal",
    "Multiplizieren",
    "Additiv",
    "Farbig nachbelichten",
    "Farbig abwedeln",
    "Reflektieren",
    "Leuchten",
    "Überlagern",
    "Differenz",
    "Negierung",
    "Aufhellen",
    "Abdunkeln",
    "Raster",
    "Xor",
];

/// the state of dlg_layerprops.s (its g_* globals)
pub(crate) struct LProps {
    doc: Option<DocId>,
    uid: u64,
    open: Shared<i32>,
    pending: bool,
    vis: i32,
    blend: i32,
    opac: i32,
    ovis: i32,
    oblend: i32,
    oopac: i32,
    omod: bool,
    name: String,
    oname: String,
    rect: Shared<WinRect>,
}

impl Default for LProps {
    fn default() -> LProps {
        LProps {
            doc: None,
            uid: 0,
            open: Shared::new(0),
            pending: false,
            vis: 0,
            blend: 0,
            opac: 0,
            ovis: 0,
            oblend: 0,
            oopac: 0,
            omod: false,
            name: String::new(),
            oname: String::new(),
            rect: Shared::new(WinRect::new(0, 0, 400, 0, 0)),
        }
    }
}

/// lprops_is_open()
pub(crate) fn lprops_is_open(l: &LProps) -> bool {
    l.pending
}

/// lprops_open(doc, i) -- remember the layer and its values, show the window
pub(crate) fn lprops_open(p: &mut Pn, ui: &mut Ui, doc: DocId, i: i32) {
    if p.lprops.pending {
        return;
    }
    let Some(d) = p.doc.get(doc) else { return };
    let Some(l) = lay_rec(d, i) else { return };
    let g = &mut p.lprops;
    g.doc = Some(doc);
    g.uid = l.uid;
    g.name = fmt_cap(l.name.clone(), LNAME_CAP as usize);
    g.oname = g.name.clone();
    g.vis = (l.flags & LYF_VISIBLE as u32) as i32;
    g.ovis = g.vis;
    g.blend = if l.sf.blend as u32 <= BM_XOR { l.sf.blend as i32 } else { 0 };
    g.oblend = g.blend;
    g.opac = l.sf.opac as i32;
    g.oopac = g.opac;
    g.omod = d.flags & DF_MODIFIED as u32 != 0;
    g.open.set(1);
    g.pending = true;
    ui.repass();
}

// lp_layer() -> (doc, index) of the dialog's layer | None (image closed / layer gone)
fn lp_layer(p: &Pn) -> Option<(DocId, i32)> {
    let doc = p.lprops.doc?;
    if doc_index(&p.doc, doc) < 0 {
        return None;
    }
    let i = lay_find_uid(p.doc.d(doc), p.lprops.uid)?;
    Some((doc, i))
}

// lp_set(name, vis, blend, opac) -- give the dialog's layer these values (only what differs;
// no history)
fn lp_set(p: &mut Pn, name: &str, vis: i32, blend: i32, opac: i32) {
    let Some((doc, i)) = lp_layer(p) else { return };
    let cur = lay_rec(p.doc.d(doc), i).map(|l| l.name.clone()).unwrap_or_default();
    if cur != name {
        lay_set_name(p, doc, i, name);
    }
    let Some(l) = lay_rec(p.doc.d(doc), i) else { return };
    let (lv, lb, lo) = ((l.flags & LYF_VISIBLE as u32) as i32, l.sf.blend as i32, l.sf.opac as i32);
    if lv != vis {
        lay_set_visible(p, doc, i, vis != 0);
    }
    if lb != blend {
        lay_set_blend(p, doc, i, blend as u32);
    }
    if lo != opac {
        lay_set_opacity(p, doc, i, opac);
    }
}

// lp_apply() -- the dialog's values on the layer (live preview)
fn lp_apply(p: &mut Pn) {
    let g = &p.lprops;
    let (n, v, b, o) = (g.name.clone(), g.vis & 1, g.blend, g.opac);
    lp_set(p, &n, v, b, o);
}

// lp_restore() -- the values from before the dialog
fn lp_restore(p: &mut Pn) {
    let g = &p.lprops;
    let (n, v, b, o) = (g.oname.clone(), g.ovis, g.oblend, g.oopac);
    lp_set(p, &n, v, b, o);
}

/// lprops_done(ok) -- OK: one history step with the old values as its other state (none when
/// nothing changed); cancel: the old values and the old modified flag
pub(crate) fn lprops_done(p: &mut Pn, ui: &mut Ui, ok: bool) {
    p.lprops.pending = false;
    p.lprops.open.set(0);
    if let Some((doc, i)) = lp_layer(p) {
        let g = &p.lprops;
        let changed = (g.vis & 1) != g.ovis || g.blend != g.oblend || g.opac != g.oopac || g.name != g.oname;
        if ok && changed {
            lp_restore(p);
            // false = out of memory: keep the old values
            if crate::document::history::hist_push_layer_props(p, doc, i, "Ebeneneigenschaften", IC_EDIT) {
                lp_apply(p);
            }
        } else {
            lp_restore(p);
            let m = p.lprops.omod;
            doc_set_modified(p, doc, m);
        }
    }
    ui.repass();
}

/// lprops_frame() -- the dialog window
pub(crate) fn lprops_frame(p: &mut Pn, ui: &mut Ui) {
    if p.lprops.pending && p.lprops.open.get() == 0 {
        p.lprops.pending = false; // Esc / close button: Abbrechen
        ui.cmd_run(CMD_LAY_PROPS_DONE, 0);
    }
    let open = p.lprops.open.clone();
    let rect = p.lprops.rect.clone();
    if ui.window_begin(WINID_LPROPS, crate::preferences::localization::tr("Ebeneneigenschaften"), Some(&open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&rect)) {
        let g = &mut p.lprops;
        let mut changed = false;
        ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Allgemein"));
        // Name
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
        ui.next_size(LBL_W, 0);
        ui.text(TS_BODY, crate::preferences::localization::tr("Name"));
        ui.next_size(-1, 0);
        if ui.textbox(ID_LP_NAME, None, &mut g.name, LNAME_CAP as u32) & EV_ACT != 0 {
            changed = true;
        }
        ui.box_end();
        if ui.checkbox(ID_LP_VIS, crate::preferences::localization::tr("Sichtbar"), &mut g.vis) & EV_ACT != 0 {
            changed = true;
        }
        ui.spacer(8);
        ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Mischmodus:"));
        ui.next_size(-1, 0);
        if ui.combobox(ID_LP_BLEND, &mut g.blend, &BLEND_LIST) & EV_ACT != 0 {
            changed = true;
        }
        ui.spacer(8);
        if ui.prop_slider(ID_LP_OPAC, "Deckfähigkeit", &mut g.opac, &RANGE_OPAC, i32::MIN) & (EV_ACT | EV_COMMIT) != 0 {
            changed = true;
        }
        if changed {
            lp_apply(p);
        }
        // (U3: no extra room for the blend mode list any more, it may leave the window)
        let r = ui.dialog_buttons(Some(crate::preferences::localization::tr("OK")), None, Some(crate::preferences::localization::tr("Abbrechen")));
        if r != DLG_NONE {
            p.lprops.pending = false;
            ui.cmd_run(CMD_LAY_PROPS_DONE, (r == DLG_PRIMARY) as u32);
        }
    }
    ui.window_end();
}
