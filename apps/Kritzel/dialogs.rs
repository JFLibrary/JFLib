// dialogs -- "Größe ändern und schräg stellen", "Bildeigenschaften", "Farben bearbeiten"
//                                                         [apps/paint/src/dialogs.s]
//
//   dialogs_frame(p, ui)             the dialogs (call at the end of the frame)
//   dialogs_cmd(p, ui, id, arg) -> bool  CMD_RESIZE / CMD_PROPERTIES / CMD_EDIT_COLOR open
//                                    a dialog, CMD_*_OK apply it
//   dialogs_close(p)                 close every dialog of this module (another one takes over)

use alloc::format;

use ui::*;

use crate::consts::*;
use crate::doc::{doc_h, doc_w};
use crate::fileio::{doc_name, paint_error};
use crate::imageops::{op_canvas, op_scale, op_skew};
use crate::view::view_fit;
use crate::{fmt_cap, Kritzel};

const ID_DLG_RESIZE: u32 = 1720; // + 1 .. 15
const ID_DLG_PROPS: u32 = 1740;
const ID_DLG_COLOR: u32 = 1760;
const LABEL_W: i32 = 96; // DIP, label column of the form rows
const PAL_CUSTOM: usize = 20; // first custom slot of the palette (3rd row)
const PAL_NCUSTOM: usize = 10;

const RANGE_SIZE: Range = Range::new(1, DOC_MAX, 1, 0);
const RANGE_PX: Range = Range::new(1, DOC_MAX, 1, 0);
const RANGE_SKEW: Range = Range::new(-89, 89, 1, 0);
static TAN_TABLE: [i32; 90] = [
    0, 17, 35, 52, 70, 87, 105, 123, 141, 158, 176, 194, 213, 231, 249, 268, 287, 306, 325, 344, 364, 384, 404, 424, 445,
    466, 488, 510, 532, 554, 577, 601, 625, 649, 675, 700, 727, 754, 781, 810, 839, 869, 900, 933, 966, 1000, 1036, 1072,
    1111, 1150, 1192, 1235, 1280, 1327, 1376, 1428, 1483, 1540, 1600, 1664, 1732, 1804, 1881, 1963, 2050, 2145, 2246,
    2356, 2475, 2605, 2747, 2904, 3078, 3271, 3487, 3732, 4011, 4331, 4705, 5145, 5671, 6314, 7115, 8144, 9514, 11430,
    14301, 19081, 28636, 57290,
];

/// the dialogs' state (the .data / .bss of dialogs.s)
pub(crate) struct Dialogs {
    rs_keep: i32,
    rs_open: Shared<i32>,
    rs_mode: i32,
    rs_prev: i32,
    rs_w: i32,
    rs_h: i32,
    rs_skx: i32,
    rs_sky: i32,
    pr_open: Shared<i32>,
    pr_w: i32,
    pr_h: i32,
    ec_open: Shared<i32>,
    ec_col: u32,
}

impl Default for Dialogs {
    fn default() -> Dialogs {
        Dialogs {
            rs_keep: 1,
            rs_open: Shared::new(0),
            rs_mode: 0,
            rs_prev: 0,
            rs_w: 0,
            rs_h: 0,
            rs_skx: 0,
            rs_sky: 0,
            pr_open: Shared::new(0),
            pr_w: 0,
            pr_h: 0,
            ec_open: Shared::new(0),
            ec_col: 0,
        }
    }
}

/// dialogs_frame()
pub(crate) fn dialogs_frame(p: &mut Kritzel, ui: &mut Ui) {
    resize_dialog(p, ui);
    props_dialog(p, ui);
    color_dialog(p, ui);
}

/// dialogs_cmd(id, arg) -> true handled
pub(crate) fn dialogs_cmd(p: &mut Kritzel, ui: &mut Ui, id: u32, _arg: u32) -> bool {
    let g = &mut p.dlg;
    match id {
        CMD_RESIZE => {
            g.rs_mode = 0;
            g.rs_prev = 0;
            g.rs_w = 100;
            g.rs_h = 100;
            g.rs_skx = 0;
            g.rs_sky = 0;
            g.rs_open.set(1);
        }
        CMD_RESIZE_OK => apply_resize(p, ui),
        CMD_PROPERTIES => {
            g.pr_w = doc_w(&p.doc);
            g.pr_h = doc_h(&p.doc);
            g.pr_open.set(1);
        }
        CMD_PROPS_OK => {
            if doc_w(&p.doc) != g.pr_w || doc_h(&p.doc) != g.pr_h {
                let (w, h) = (g.pr_w, g.pr_h);
                op_canvas(p, w, h);
            }
        }
        CMD_EDIT_COLOR => {
            g.ec_col = if p.active_col == 2 { p.tools.col2 } else { p.tools.col1 };
            g.ec_open.set(1);
        }
        CMD_COLOR_OK => apply_color(p),
        _ => return false,
    }
    true
}

/// dialogs_close() -- close every dialog of this module (another one takes over)
pub(crate) fn dialogs_close(p: &mut Kritzel) {
    p.dlg.rs_open.set(0);
    p.dlg.pr_open.set(0);
    p.dlg.ec_open.set(0);
}

// ------------------------------------------------------------------ helpers

// form_row(label) -- begin a row "label: <control>" (end it with box_end); the control that
// follows fills the rest
fn form_row(ui: &mut Ui, label: &str) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(LABEL_W, 0);
    ui.text(TS_BODY, label);
    ui.next_size(-1, 0);
}

// scale_div(v, mul, div) -> round(v * mul / div), at least 1
fn scale_div(v: i32, mul: i32, div: i32) -> i32 {
    let n = v.wrapping_mul(mul).wrapping_add((div as u32 >> 1) as i32);
    n.wrapping_div(div).max(1)
}

// ------------------------------------------------------------------ resize and skew

// resize_dialog()
fn resize_dialog(p: &mut Kritzel, ui: &mut Ui) {
    let open = p.dlg.rs_open.clone();
    if ui.dialog_begin(ID_DLG_RESIZE, "Größe ändern und schräg stellen", &open) {
        ui.text(TS_BODY_STRONG, "Größe ändern");
        // percent / pixel
        ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
        let g = &mut p.dlg;
        ui.radio(ID_DLG_RESIZE + 1, "Prozent", &mut g.rs_mode, 0);
        ui.radio(ID_DLG_RESIZE + 2, "Pixel", &mut g.rs_mode, 1);
        if g.rs_mode != g.rs_prev {
            g.rs_prev = g.rs_mode;
            let (dw, dh) = (doc_w(&p.doc), doc_h(&p.doc));
            if g.rs_mode == 0 {
                // pixels -> percent
                g.rs_w = scale_div(g.rs_w, 100, dw);
                g.rs_h = scale_div(g.rs_h, 100, dh);
            } else {
                // percent -> pixels
                g.rs_w = scale_div(g.rs_w, dw, 100);
                g.rs_h = scale_div(g.rs_h, dh, 100);
            }
        }
        ui.box_end();
        // horizontal / vertical size
        form_row(ui, "Horizontal:");
        let ev = ui.number_box(ID_DLG_RESIZE + 3, &mut p.dlg.rs_w, &RANGE_SIZE);
        ui.box_end();
        if ev & EV_ACT != 0 && p.dlg.rs_keep != 0 {
            keep_aspect(p, 0);
        }
        form_row(ui, "Vertikal:");
        let ev = ui.number_box(ID_DLG_RESIZE + 4, &mut p.dlg.rs_h, &RANGE_SIZE);
        ui.box_end();
        if ev & EV_ACT != 0 && p.dlg.rs_keep != 0 {
            keep_aspect(p, 1);
        }
        if ui.checkbox(ID_DLG_RESIZE + 5, "Seitenverhältnis beibehalten", &mut p.dlg.rs_keep) & EV_ACT != 0
            && p.dlg.rs_keep != 0
        {
            keep_aspect(p, 0);
        }
        ui.spacer(8);
        ui.text(TS_BODY_STRONG, "Schräg stellen (Grad)");
        form_row(ui, "Horizontal:");
        ui.number_box(ID_DLG_RESIZE + 6, &mut p.dlg.rs_skx, &RANGE_SKEW);
        ui.box_end();
        form_row(ui, "Vertikal:");
        ui.number_box(ID_DLG_RESIZE + 7, &mut p.dlg.rs_sky, &RANGE_SKEW);
        ui.box_end();
        if ui.dialog_buttons(Some("OK"), None, Some("Abbrechen")) == DLG_PRIMARY {
            ui.cmd_run(CMD_RESIZE_OK, 0);
        }
    }
    ui.dialog_end();
}

// keep_aspect(changed) -- 0: width changed -> adapt height, 1: the other way round
fn keep_aspect(p: &mut Kritzel, changed: i32) {
    let g = &mut p.dlg;
    if g.rs_mode == 0 {
        // percent: both the same
        let v = if changed == 0 { g.rs_w } else { g.rs_h };
        g.rs_w = v;
        g.rs_h = v;
        return;
    }
    let (dw, dh) = (doc_w(&p.doc), doc_h(&p.doc));
    if changed == 0 {
        g.rs_h = scale_div(g.rs_w, dh, dw); // h = w * doc_h / doc_w
    } else {
        g.rs_w = scale_div(g.rs_h, dw, dh); // w = h * doc_w / doc_h
    }
}

// apply_resize() -- scale, then skew
fn apply_resize(p: &mut Kritzel, ui: &mut Ui) {
    let (mut w, mut h) = (p.dlg.rs_w, p.dlg.rs_h);
    if p.dlg.rs_mode == 0 {
        w = scale_div(w, doc_w(&p.doc), 100);
        h = scale_div(h, doc_h(&p.doc), 100);
    }
    w = w.min(DOC_MAX);
    h = h.min(DOC_MAX);
    if doc_w(&p.doc) != w || doc_h(&p.doc) != h {
        op_scale(p, w, h);
    }
    let kx = tan1000(p.dlg.rs_skx);
    let ky = tan1000(p.dlg.rs_sky);
    if kx | ky == 0 {
        return;
    }
    if op_skew(p, kx, ky) != 0 {
        paint_error(p, "Schräg stellen: Das Ergebnis wäre größer als 16384 × 16384 Pixel.");
    }
    view_fit(p, ui);
}

// tan1000(deg) -> tan(deg) * 1000 for -89 .. 89
fn tan1000(deg: i32) -> i32 {
    let a = (deg.wrapping_abs() as u32).min(89);
    let t = TAN_TABLE[a as usize];
    if deg < 0 { -t } else { t }
}

// ------------------------------------------------------------------ properties

// props_dialog()
fn props_dialog(p: &mut Kritzel, ui: &mut Ui) {
    let open = p.dlg.pr_open.clone();
    if ui.dialog_begin(ID_DLG_PROPS, "Bildeigenschaften", &open) {
        let t = fmt_cap(format!("Datei: {}", doc_name(&p.doc)), 256);
        ui.text(TS_BODY | TX_ELLIPSIS, &t);
        let t = fmt_cap(format!("Auflösung: 96 DPI · Ebenen: {}", p.doc.nlayers), 256);
        ui.text(TS_BODY | TX_SECONDARY, &t);
        ui.spacer(8);
        ui.text(TS_BODY_STRONG, "Bildgröße (Pixel)");
        form_row(ui, "Breite:");
        ui.number_box(ID_DLG_PROPS + 1, &mut p.dlg.pr_w, &RANGE_PX);
        ui.box_end();
        form_row(ui, "Höhe:");
        ui.number_box(ID_DLG_PROPS + 2, &mut p.dlg.pr_h, &RANGE_PX);
        ui.box_end();
        if ui.dialog_buttons(Some("OK"), None, Some("Abbrechen")) == DLG_PRIMARY {
            ui.cmd_run(CMD_PROPS_OK, 0);
        }
    }
    ui.dialog_end();
}

// ------------------------------------------------------------------ colours

// color_dialog()
fn color_dialog(p: &mut Kritzel, ui: &mut Ui) {
    let open = p.dlg.ec_open.clone();
    if ui.dialog_begin(ID_DLG_COLOR, "Farben bearbeiten", &open) {
        ui.color_picker(ID_DLG_COLOR + 1, &mut p.dlg.ec_col);
        if ui.dialog_buttons(Some("OK"), None, Some("Abbrechen")) == DLG_PRIMARY {
            ui.cmd_run(CMD_COLOR_OK, 0);
        }
    }
    ui.dialog_end();
}

// apply_color() -- the picked colour becomes the active colour and the first custom palette
// slot (the others move one to the right; an existing equal entry is removed)
fn apply_color(p: &mut Kritzel) {
    let c = p.dlg.ec_col | 0xFF00_0000;
    if p.active_col == 2 {
        p.tools.col2 = c;
    } else {
        p.tools.col1 = c;
    }
    let pal = &mut p.palette[PAL_CUSTOM..PAL_CUSTOM + PAL_NCUSTOM];
    // the slot to drop: an equal entry, else the last one
    let mut k = pal.iter().position(|&x| x == c).unwrap_or(PAL_NCUSTOM - 1);
    while k > 0 {
        pal[k] = pal[k - 1];
        k -= 1;
    }
    pal[0] = c;
}
