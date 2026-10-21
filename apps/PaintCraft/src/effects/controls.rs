// fxui -- PaintCraft: the generic effect dialog (agent A6), like paint.net's IndirectUI: a
// modal dialog window (title = effect name) built from the effect's FXPROP list, placed
// next to the image; the canvas behind it shows the live preview.  Sub-module of fx.s.
//   fxui_open()      prepare the rows of the running effect (g_fx): ranges, defaults, pan
//                    thumbnail, window placement
//   fxui_frame()     the window (fx_frame calls it while g_fx + FS_OPEN)
//   fxui_close()     free the thumbnail
//   fxui_status()    the progress row "Rendern... n%" (also used by adj_curves / adj_levels)
// Rows (paint.net layout):
//   FP_INT     ui_prop_slider: label, slider, number box, reset arrow
//   FP_DEC     ui_prop_slider_f (step 10^-decimals)
//   FP_BOOL    check box
//   FP_CHOICE  label + radio buttons (<= 3 choices) or a combo box
//   FP_ANGLE   label + dial (ui_angle; ANG_360 when min >= 0) + number box + reset
//   FP_PAN     label + ui_pan over a thumbnail of the layer + X / Y number boxes + reset
//   FP_COLOR   label + swatch (click: an inline colour picker with alpha and fields) + reset
//   FP_SEED    label + "Randomisieren" (a new seed)
//   FP_BLOB    not shown (custom dialogs only)
// Every change of FS_PARAM (compared after the widgets) calls fx_params_changed; OK /
// Abbrechen queue CMD_FX_OK / CMD_FX_CANCEL.  FPF_LOG is shown linearly (the library slider
// has no logarithmic mode; no built-in effect uses it).
// Widget ids: property i = ID_FX_PROP0 + i * 16 + part (0 main, 1 angle number box, 2 reset,
// 3 / 4 pan X / Y, 4..6 radio buttons, 8 colour picker); window WINID_FX.
//                                              [apps/PaintCraft/src/fxui.s]

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use ::fx::{Params, FP_ANGLE, FP_BOOL, FP_CHOICE, FP_COLOR, FP_DEC, FP_INT, FP_PAN, FP_SEED, FX_NSLOTS};
use gfx::{Surface, FMT_PARGB};
use rt::fx::{cvttd, maxsd, minsd};
use ui::*;

use crate::app::constants::*;
use crate::effects::{fx_choices, fx_label, fx_params_changed, fx_place_dialog, fx_progress, FSM_DIALOG, FSP_PREPARE, FSP_RENDER};
use crate::app::formatting::fmt_cap;
use crate::app::Pn;

const UI_W: i32 = 360; // dialog width (DIP)
const NB_W: i32 = 88; // number boxes
const THUMB_DIP: i32 = 160; // pan thumbnail: longest side
const STATUS_H: i32 = 20; // progress row height (DIP)
const STATUS_CAP: usize = 64;
const STATUS_TXT_W: i32 = 110; // width of the progress text (DIP)
static C_STEPS: [f64; 5] = [1.0, 0.1, 0.01, 0.001, 0.0001];

/// the state of fxui.s (its g_* globals)
pub(crate) struct FxUi {
    rect: Shared<WinRect>,
    /// g_ui_thumb: pan thumbnail (FMT_PARGB)
    thumb: Option<Rc<Surface>>,
    thumbver: u32,
    /// RANGE / RANGE_F per property, f64 / i32 defaults
    rng: Vec<Range>,
    rngf: Vec<RangeF>,
    deff: Vec<f64>,
    defi: Vec<i32>,
    /// 1 = colour picker of the property shown
    cpop: Vec<bool>,
    seedn: u32,
}

impl Default for FxUi {
    fn default() -> FxUi {
        FxUi {
            rect: Shared::new(WinRect::new(0, 0, UI_W, 0, 0)),
            thumb: None,
            thumbver: 0,
            rng: Vec::new(),
            rngf: Vec::new(),
            deff: Vec::new(),
            defi: Vec::new(),
            cpop: Vec::new(),
            seedn: 0,
        }
    }
}

/// fxui_open() -- ranges and defaults of every property, the pan thumbnail, the placement
pub(crate) fn fxui_open(p: &mut Pn, ui: &mut Ui) {
    let Some(desc) = p.fx.desc else { return };
    let u = &mut p.fxui;
    u.rng = vec![Range::new(0, 0, 1, 0); FX_NSLOTS];
    u.rngf = vec![RangeF::new(0.0, 0.0, 1.0, 0, 0); FX_NSLOTS];
    u.deff = vec![0.0; FX_NSLOTS];
    u.defi = vec![0; FX_NSLOTS];
    u.cpop = vec![false; FX_NSLOTS];
    let mut pan = false; // an FP_PAN property exists
    for (i, pr) in desc.props.iter().enumerate().take(FX_NSLOTS) {
        // RANGE (FP_INT)
        u.rng[i] = Range::new(cvttd(pr.min), cvttd(pr.max), 1, 0);
        u.defi[i] = cvttd(pr.def);
        // RANGE_F (FP_DEC, FP_ANGLE, FP_PAN): step 10^-decimals
        let dec = (pr.dec as usize).min(4);
        u.rngf[i] = RangeF::new(pr.min, pr.max, C_STEPS[dec], dec as u32, 0);
        u.deff[i] = pr.def;
        if pr.typ == FP_PAN {
            pan = true;
        }
    }
    if pan {
        fxui_thumb(p, ui);
    }
    let r = p.fxui.rect.clone();
    fx_place_dialog(p, ui, &r, UI_W);
}

// fxui_thumb() -- the layer (FS_SRC) fitted into THUMB_DIP (FMT_PARGB, over a checkerboard)
fn fxui_thumb(p: &mut Pn, ui: &mut Ui) {
    let Some(src) = crate::effects::fx_src(&p.fx) else { return };
    let long = ui.dp(THUMB_DIP) as u32;
    let (sw, sh) = (src.w as u32, src.h as u32);
    if sw == 0 || sh == 0 {
        return;
    }
    let (w, h) = if sw >= sh { (long, sh.wrapping_mul(long) / sw) } else { (sw.wrapping_mul(long) / sh, long) };
    let (w, h) = ((w as i32).max(1), (h as i32).max(1));
    p.fxui.thumb = None;
    let mut t = Surface::new(w, h, FMT_PARGB);
    if t.px.is_empty() {
        return;
    }
    crate::document::composite::thumb_render(&mut t, src, 4);
    p.fxui.thumbver = p.fxui.thumbver.wrapping_add(1);
    p.fxui.thumb = Some(Rc::new(t));
}

/// fxui_close() -- free the thumbnail
pub(crate) fn fxui_close(p: &mut Pn, _ui: &mut Ui) {
    p.fxui.thumb = None;
}

/// fxui_frame() -- the dialog window of the running effect
pub(crate) fn fxui_frame(p: &mut Pn, ui: &mut Ui) {
    let Some(desc) = p.fx.desc else { return };
    let open = p.fx.open.clone();
    let rect = p.fxui.rect.clone();
    let mut prev: Option<Params> = None; // FS_PARAM before the widgets (the rows were built)
    if ui.window_begin(WINID_FX, p.fx.name, Some(&open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&rect)) {
        prev = Some(p.fx.param);
        for i in 0..desc.props.len().min(FX_NSLOTS) {
            fxui_prop_row(p, ui, i);
        }
        fxui_status(p, ui);
        let r = ui.dialog_buttons(Some(crate::preferences::localization::tr("OK")), None, Some(crate::preferences::localization::tr("Abbrechen")));
        if r == DLG_PRIMARY {
            ui.cmd_run(CMD_FX_OK, 0);
        } else if r == DLG_CLOSE {
            ui.cmd_run(CMD_FX_CANCEL, 0);
        }
    }
    ui.window_end();
    if let Some(prev) = prev {
        if p.fx.mode == FSM_DIALOG && prev.0 != p.fx.param.0 {
            fx_params_changed(p, ui);
        }
    }
}

// lo32 / set_lo32 -- the low dword of a slot (the widgets edit an i32 / u32 in place)
fn set_lo32(slot: &mut u64, v: u32) {
    *slot = (*slot & !0xFFFF_FFFF) | v as u64;
}

// fxui_prop_row(i) -- the widgets of property i
fn fxui_prop_row(p: &mut Pn, ui: &mut Ui, i: usize) {
    let Some(desc) = p.fx.desc else { return };
    let pr = &desc.props[i];
    let s = pr.slot as usize & (FX_NSLOTS - 1);
    let id = ID_FX_PROP0 + (i as u32) * 16; // widget id
    let label = fx_label(desc, i);
    let u = &mut p.fxui;
    let slot = &mut p.fx.param.0;
    match pr.typ {
        FP_INT => {
            // slider row (the widget edits an i32 copy)
            let mut v = slot[s] as u32 as i32;
            ui.prop_slider(id, label, &mut v, &u.rng[i], u.defi[i]);
            slot[s] = v as i64 as u64;
        }
        FP_DEC => {
            let mut v = f64::from_bits(slot[s]);
            ui.prop_slider_f(id, label, &mut v, &u.rngf[i], Some(u.deff[i]));
            slot[s] = v.to_bits();
        }
        FP_BOOL => {
            let mut v = slot[s] as u32 as i32;
            ui.checkbox(id, label, &mut v);
            set_lo32(&mut slot[s], v as u32);
        }
        FP_CHOICE => {
            // radio buttons (<= 3) or a combo box
            ui.text(TS_BODY, label);
            let ch = fx_choices(desc, i);
            if ch.is_empty() {
                return;
            }
            let mut v = slot[s] as u32 as i32;
            if ch.len() <= 3 {
                for (k, c) in ch.iter().enumerate() {
                    ui.radio(id + 4 + k as u32, c, &mut v, k as i32);
                }
            } else {
                ui.next_size(-1, 0);
                ui.combobox(id, &mut v, ch);
            }
            set_lo32(&mut slot[s], v as u32);
        }
        FP_ANGLE => {
            // dial + number box + reset
            ui.text(TS_BODY, label);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
            let flags = if 0.0 > pr.min { 0 } else { ANG_360 }; // min >= 0: 0 .. 360
            let mut v = f64::from_bits(slot[s]);
            ui.angle(id, &mut v, flags);
            v = minsd(maxsd(v, pr.min), pr.max); // the dial wraps; keep the property's range
            ui.next_size(NB_W, 0);
            ui.number_box_f(id + 1, &mut v, &u.rngf[i]);
            if fxui_reset(ui, id + 2, v == u.deff[i]) {
                v = u.deff[i];
            }
            slot[s] = v.to_bits();
            ui.box_end();
        }
        FP_PAN => {
            // control over the thumbnail + X / Y number boxes + reset
            ui.text(TS_BODY, label);
            let s1 = (s + 1).min(FX_NSLOTS - 1);
            let mut x = f64::from_bits(slot[s]);
            let mut y = f64::from_bits(slot[s1]);
            ui.pan(id, &mut x, &mut y, u.thumb.as_ref(), u.thumbver, 0);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            ui.text(TS_BODY, crate::preferences::localization::tr("X:"));
            ui.next_size(NB_W, 0);
            ui.number_box_f(id + 3, &mut x, &u.rngf[i]);
            ui.text(TS_BODY, crate::preferences::localization::tr("Y:"));
            ui.next_size(NB_W, 0);
            ui.number_box_f(id + 4, &mut y, &u.rngf[i]);
            // default: x = FXP_DEF, y = the f64 in FXP_CHOICES
            if fxui_reset(ui, id + 2, x == pr.def && y == pr.def_y) {
                x = pr.def;
                y = pr.def_y;
            }
            slot[s] = x.to_bits();
            slot[s1] = y.to_bits();
            ui.box_end();
        }
        FP_COLOR => {
            // swatch (toggles an inline picker) + hex text + reset
            ui.text(TS_BODY, label);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
            ui.next_size(32, 32);
            if ui.swatch(id, slot[s] as u32, u.cpop[i]) & EV_ACT != 0 {
                u.cpop[i] = !u.cpop[i];
                ui.repass();
            }
            let hex = format!("#{:08X}", slot[s] as u32);
            ui.text(TS_BODY, &hex);
            let def = pr.def_color();
            if fxui_reset(ui, id + 2, slot[s] as u32 == def) {
                slot[s] = def as u64;
            }
            ui.box_end();
            if u.cpop[i] {
                let mut c = slot[s] as u32;
                ui.color_picker_ex(id + 8, &mut c, CP_ALPHA | CP_FIELDS);
                set_lo32(&mut slot[s], c);
            }
        }
        FP_SEED => {
            // a new random seed
            ui.text(TS_BODY, label);
            ui.next_icon(IC_SHUFFLE);
            if ui.button(id, crate::preferences::localization::tr("Randomisieren")) & EV_ACT != 0 {
                u.seedn = u.seedn.wrapping_add(1);
                slot[s] = (::fx::hash32(slot[s] as u32, u.seedn, 0x5EED, 0) & 0x7FFF_FFFF) as u64;
            }
        }
        _ => {}
    }
}

// fxui_reset(id, is_default) -> true clicked -- the reset arrow of a property row
fn fxui_reset(ui: &mut Ui, id: u32, is_default: bool) -> bool {
    if is_default {
        ui.next_flags(ST_DISABLED);
    }
    ui.icon_button(id, IC_UNDO, "Zurücksetzen") & EV_ACT != 0
}

/// fxui_status() -- a row of STATUS_H DIP: "Rendern... n%" and a progress bar while the
/// effect renders, empty when it is complete.  Every child has a fixed or fill size, so the
/// changing text never changes the layout (the progress moves between the passes of a frame)
pub(crate) fn fxui_status(p: &mut Pn, ui: &mut Ui) {
    ui.next_size(-1, STATUS_H);
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_FIXED, 12);
    let (done, total, phase) = fx_progress(&p.fx);
    let busy = phase == FSP_PREPARE || phase == FSP_RENDER;
    let mut status = String::new();
    if busy {
        let pct = if total != 0 { done.wrapping_mul(100) / total } else { 0 };
        status = fmt_cap(format!("Rendern... {}%", pct as i32), STATUS_CAP);
    }
    ui.next_size(STATUS_TXT_W, STATUS_H);
    ui.text(TS_CAPTION | TX_SECONDARY, &status);
    ui.next_size(-1, 12);
    if busy {
        ui.progress(ID_FX_PROG, done as i32, total as i32);
    } else {
        ui.text(TS_CAPTION, crate::preferences::localization::tr(""));
    }
    ui.box_end();
}
