// adj_levels -- PaintCraft: Korrekturen > Manuelle Anpassung... (Levels, agent A6), paint.net
// 5.1's Levels dialog as a modal dialog window:
//   Eingabehistogramm | Eingabe (white / black number boxes, a vertical slider with two
//   triangles) | Ausgabe (white / black number boxes, a slider with three triangles: white,
//   grey = gamma, black) | Ausgabehistogramm; Ausgangsgamma; Automatisch, Zurücksetzen,
//   R G B check boxes; OK / Abbrechen.
// Sub-module of fx.s:
//   levels_open()   input histogram of the layer (fx_hist_rgba over the area / selection),
//                   placement
//   levels_frame()  the window (fx_frame calls it while g_fx + FS_OPEN)
//   levels_close()  free the histogram images
// Parameters (fx_adj.s levels): 0 input_black, 1 input_white, 2 output_black, 3 output_white
// (colours, bytes B G R), 4..6 gamma red / green / blue, 7..9 red / green / blue (kernel
// channel mask, always 1 here).  The R / G / B check boxes work like paint.net's: they choose
// which channels an edit changes (the numbers show the first checked channel); unchecked
// channels keep their own values.  Grey triangle: output value of the input's mid-tone,
// out_lo + (out_hi - out_lo) * 2^-gamma.  Automatisch = paint.net HistogramRgb.MakeLevelsAuto
// with FXA's auto_level numbers (percentiles 0.005 / 0.995, mean, f32 gamma), so Levels after
// Automatisch gives exactly Korrekturen > Automatisch.  The output histogram = the input
// histogram through the kernel's table (fx_levels_lut).
// Widget ids ID_DLG_LEVELS + 1 in white, + 2 in black, + 3 out white, + 4 out black, + 5 gamma,
// + 6 input slider, + 7 output slider, + 8 Automatisch, + 9 Zurücksetzen, + 10..12 R G B,
// + 13 / + 14 histograms; window WINID_LEVELS.
//                                              [apps/PaintCraft/src/adj_levels.s]

use alloc::rc::Rc;

use ::fx::lut::{levels_lut, Lvl};
use ::fx::{FxArgs, Params};
use gfx::{Surface, FMT_PARGB};
use rt::fx::{cvtd, cvtd64, cvtt, maxsd, minsd};
use ui::*;

use crate::app::constants::*;
use crate::effects::{fx_params_changed, fx_place_dialog, FSM_DIALOG};
use crate::effects::controls::fxui_status;
use crate::app::Pn;

const LV_W: i32 = 500; // dialog width (DIP)
const HIST_W: i32 = 112; // histogram columns
const COL_W: i32 = 88; // input / output columns
const BAR_H: i32 = 256; // histograms and sliders (1 DIP per value at 100 %)
const NB_H: i32 = 32; // height of a number box row (alignment spacer)
const ID_LV_INHI: u32 = ID_DLG_LEVELS + 1;
const ID_LV_INLO: u32 = ID_DLG_LEVELS + 2;
const ID_LV_OUTHI: u32 = ID_DLG_LEVELS + 3;
const ID_LV_OUTLO: u32 = ID_DLG_LEVELS + 4;
const ID_LV_GAMMA: u32 = ID_DLG_LEVELS + 5;
const ID_LV_INSL: u32 = ID_DLG_LEVELS + 6;
const ID_LV_OUTSL: u32 = ID_DLG_LEVELS + 7;
const ID_LV_AUTO: u32 = ID_DLG_LEVELS + 8;
const ID_LV_RESET: u32 = ID_DLG_LEVELS + 9;
const ID_LV_RED: u32 = ID_DLG_LEVELS + 10;
const ID_LV_GREEN: u32 = ID_DLG_LEVELS + 11;
const ID_LV_BLUE: u32 = ID_DLG_LEVELS + 12;
const ID_LV_HIN: u32 = ID_DLG_LEVELS + 13;
const ID_LV_HOUT: u32 = ID_DLG_LEVELS + 14;
// handles (drag): input white, input black, output white, grey, black
const H_INWHITE: usize = 1;
const H_INBLACK: usize = 2;
const H_OUTWHITE: usize = 3;
const H_GREY: usize = 4;
const H_OUTBLACK: usize = 5;
const SLOT_INLO: usize = 0;
const SLOT_INHI: usize = 1;
const SLOT_OUTLO: usize = 2;
const SLOT_OUTHI: usize = 3;
const C_TMIN: f64 = 0.001;
const C_TMAX: f64 = 0.999;
const C_GMIN: f64 = 0.1;
const C_GMAX: f64 = 10.0;
const C_P005: f64 = 0.004999999888241291; // 0.005f (FXA auto_level)
const C_P995: f64 = 0.9950000047683716; // 0.995f
const RNG_BYTE: Range = Range::new(0, 255, 1, 0);
const RNGF_GAMMA: RangeF = RangeF::new(0.1, 10.0, 0.01, 2, 0);
/// triangle fill per handle (0 unused, input white, input black, output white, grey, black)
static LV_TRI_COL: [u32; 6] = [0, 0xFFFFFFFF, 0xFF000000, 0xFFFFFFFF, 0xFF808080, 0xFF000000];
/// histogram pixel per channel bits (1 blue, 2 green, 4 red), premultiplied, alpha 0xB0
static LV_HCOL: [u32; 8] = [0x00000000, 0xB00000B0, 0xB000B000, 0xB000B0B0, 0xB0B00000, 0xB0B000B0, 0xB0B0B000, 0xB0808080];

/// the state of adj_levels.s (its g_* globals)
pub(crate) struct Levels {
    rect: Shared<WinRect>,
    /// input / output histograms (fx_hist_rgba: B G R A)
    hin: [[u32; 256]; 4],
    hout: [[u32; 256]; 4],
    lut: [u8; 1024],
    sfin: Option<Rc<Surface>>,
    sfout: Option<Rc<Surface>>,
    grad: Option<Rc<Surface>>,
    count: i64,
    outkey: u64,
    inkey: u32,
    /// g_lv_ver: the SF_AUX of the last histogram image; the images' versions
    ver: u32,
    ver_in: u32,
    ver_out: u32,
    /// number boxes (shown channel): in white, in black, out white, out black; gamma
    v: [i32; 4],
    v0: [i32; 4],
    gamma: f64,
    gamma0: f64,
    /// handle y px (index H_*)
    hy: [i32; 8],
    /// R G B check boxes
    mask: [i32; 3],
    /// H_* being dragged, 0 = none
    drag: usize,
}

impl Default for Levels {
    fn default() -> Levels {
        Levels {
            rect: Shared::new(WinRect::new(0, 0, LV_W, 0, 0)),
            hin: [[0; 256]; 4],
            hout: [[0; 256]; 4],
            lut: [0; 1024],
            sfin: None,
            sfout: None,
            grad: None,
            count: 0,
            outkey: 0,
            inkey: 0,
            ver: 0,
            ver_in: 0,
            ver_out: 0,
            v: [0; 4],
            v0: [0; 4],
            gamma: 0.0,
            gamma0: 0.0,
            hy: [0; 8],
            mask: [0; 3],
            drag: 0,
        }
    }
}

/// levels_open() -- the input histogram, check boxes, placement
pub(crate) fn levels_open(p: &mut Pn, ui: &mut Ui) {
    let l = &mut p.levels;
    l.mask = [1, 1, 1];
    l.drag = 0;
    let g = p.fx.core.g;
    let mut n = 0;
    if let Some(src) = crate::effects::fx_src(&p.fx) {
        let mut a = FxArgs::new(src, [g.bx0, g.by0, g.bx1, g.by1], &p.fx.param);
        a.sel = crate::effects::fx_sel(&p.fx);
        n = ::fx::hist::hist_rgba(&a, &mut l.hin).max(0);
    }
    l.count = n;
    l.inkey = u32::MAX;
    l.outkey = u64::MAX;
    let r = l.rect.clone();
    fx_place_dialog(p, ui, &r, LV_W);
}

/// levels_close() -- free the histogram images
pub(crate) fn levels_close(p: &mut Pn, _ui: &mut Ui) {
    let l = &mut p.levels;
    l.sfin = None;
    l.sfout = None;
    l.grad = None;
}

// lv_chan() -> byte index (2 R, 1 G, 0 B) of the first checked channel (R when none)
fn lv_chan(l: &Levels) -> usize {
    if l.mask[0] != 0 {
        2
    } else if l.mask[1] != 0 {
        1
    } else if l.mask[2] != 0 {
        0
    } else {
        2
    }
}

// lv_get(slot) -> the byte of colour slot 0..3 for the shown channel
fn lv_get(l: &Levels, prm: &Params, slot: usize) -> i32 {
    prm.byte(slot & 3, lv_chan(l)) as i32
}

// lv_set(slot, v) -- the byte of colour slot 0..3 = v for every checked channel
fn lv_set(l: &Levels, prm: &mut Params, slot: usize, v: i32) {
    let s = slot & 3;
    let v = v.clamp(0, 255) as u8;
    if l.mask[0] != 0 {
        prm.set_byte(s, 2, v);
    }
    if l.mask[1] != 0 {
        prm.set_byte(s, 1, v);
    }
    if l.mask[2] != 0 {
        prm.set_byte(s, 0, v);
    }
}

// lv_gamma_get() -> gamma of the shown channel (slot 6 - byte index)
fn lv_gamma_get(l: &Levels, prm: &Params) -> f64 {
    prm.f(6 - lv_chan(l))
}

// lv_gamma_set(g) -- gamma of every checked channel
fn lv_gamma_set(l: &Levels, prm: &mut Params, g: f64) {
    if l.mask[0] != 0 {
        prm.set_f(4, g);
    }
    if l.mask[1] != 0 {
        prm.set_f(5, g);
    }
    if l.mask[2] != 0 {
        prm.set_f(6, g);
    }
}

/// levels_frame() -- the Manuelle Anpassung window
pub(crate) fn levels_frame(p: &mut Pn, ui: &mut Ui) {
    let open = p.fx.open.clone();
    let rect = p.levels.rect.clone();
    let mut prev: Option<Params> = None;
    if ui.window_begin(WINID_LEVELS, p.fx.name, Some(&open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&rect)) {
        prev = Some(p.fx.param);
        lv_body(p, ui);
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

// the window's content up to the status row
fn lv_body(p: &mut Pn, ui: &mut Ui) {
    lv_images(p, ui);
    let desc = p.fx.desc;
    let l = &mut p.levels;
    let prm = &mut p.fx.param;
    // the shown channel's values
    l.v = [lv_get(l, prm, SLOT_INHI), lv_get(l, prm, SLOT_INLO), lv_get(l, prm, SLOT_OUTHI), lv_get(l, prm, SLOT_OUTLO)];
    l.gamma = lv_gamma_get(l, prm);
    l.gamma0 = l.gamma;
    l.v0 = l.v;
    // four columns
    ui.box_begin(0, BOX_H, 16);
    // -- Eingabehistogramm
    lv_col_begin(ui, HIST_W, "Eingabehistogramm");
    ui.spacer(NB_H);
    ui.next_size(HIST_W, BAR_H);
    lv_image(ui, ID_LV_HIN, l.sfin.as_ref(), l.ver_in);
    ui.tooltip("Farbverteilung des Bildes vor der Korrektur.");
    ui.box_end();
    // -- Eingabe
    lv_col_begin(ui, COL_W, "Eingabe");
    lv_numbox(ui, ID_LV_INHI, &mut l.v[0]);
    ui.tooltip("Eingangsweißwert");
    lv_slider(l, prm, ui, ID_LV_INSL, false);
    lv_numbox(ui, ID_LV_INLO, &mut l.v[1]);
    ui.tooltip("Eingangsschwarzwert");
    ui.box_end();
    // -- Ausgabe
    lv_col_begin(ui, COL_W, "Ausgabe");
    lv_numbox(ui, ID_LV_OUTHI, &mut l.v[2]);
    ui.tooltip("Ausgangsweißwert");
    lv_slider(l, prm, ui, ID_LV_OUTSL, true);
    lv_numbox(ui, ID_LV_OUTLO, &mut l.v[3]);
    ui.tooltip("Ausgangsschwarzwert");
    ui.box_end();
    // -- Ausgabehistogramm
    lv_col_begin(ui, HIST_W, "Ausgabehistogramm");
    ui.spacer(NB_H);
    ui.next_size(HIST_W, BAR_H);
    lv_image(ui, ID_LV_HOUT, l.sfout.as_ref(), l.ver_out);
    ui.tooltip("Farbverteilung, wie sie nach der Korrektur aussehen wird.");
    ui.box_end();
    ui.box_end();
    // Ausgangsgamma
    ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
    ui.text(TS_BODY, crate::preferences::localization::tr("Ausgangsgamma:"));
    ui.next_size(COL_W, 0);
    ui.number_box_f(ID_LV_GAMMA, &mut l.gamma, &RNGF_GAMMA);
    ui.box_end();
    // apply edited numbers to the checked channels
    lv_apply_numbers(l, prm);
    // Automatisch, Zurücksetzen, R G B
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    if ui.button(ID_LV_AUTO, crate::preferences::localization::tr("Automatisch")) & EV_ACT != 0 {
        lv_auto(l, prm);
    }
    ui.tooltip("Schwarz- und Weißpunkt der Eingabe sowie das Gamma selbst bestimmen lassen");
    if ui.button(ID_LV_RESET, crate::preferences::localization::tr("Zurücksetzen")) & EV_ACT != 0 {
        if let Some(d) = desc {
            ::fx::defaults(d, prm);
        }
        l.mask = [1, 1, 1];
    }
    ui.spacer(-1);
    ui.checkbox(ID_LV_RED, crate::preferences::localization::tr("R"), &mut l.mask[0]);
    ui.tooltip("Rotkanal bearbeiten (ein/aus)");
    ui.checkbox(ID_LV_GREEN, crate::preferences::localization::tr("G"), &mut l.mask[1]);
    ui.tooltip("Grünkanal bearbeiten (ein/aus)");
    ui.checkbox(ID_LV_BLUE, crate::preferences::localization::tr("B"), &mut l.mask[2]);
    ui.tooltip("Blaukanal bearbeiten (ein/aus)");
    ui.box_end();
}

// lv_image(id, sf) -- a histogram image (an empty space when it could not be made)
fn lv_image(ui: &mut Ui, id: u32, sf: Option<&Rc<Surface>>, ver: u32) {
    match sf {
        Some(s) => {
            ui.image(id, s, ver);
        }
        None => ui.spacer(BAR_H),
    }
}

// lv_col_begin(w, caption) -- a column: BOX_V of w DIP with a caption (close with box_end)
fn lv_col_begin(ui: &mut Ui, w: i32, caption: &str) {
    ui.next_size(w, 0);
    ui.box_begin(0, BOX_V, 8);
    ui.text(TS_CAPTION | TX_ELLIPSIS, caption);
}

// lv_numbox(id, &i32) -- a 0..255 number box over the column's width
fn lv_numbox(ui: &mut Ui, id: u32, v: &mut i32) {
    ui.next_size(-1, 0);
    ui.number_box(id, v, &RNG_BYTE);
}

// lv_apply_numbers() -- number boxes whose value changed in this pass write it to the checked
// channels (input white stays above input black)
fn lv_apply_numbers(l: &Levels, prm: &mut Params) {
    if l.v[0] != l.v0[0] {
        let lo = lv_get(l, prm, SLOT_INLO) + 1;
        let v = l.v[0].max(lo).min(255);
        lv_set(l, prm, SLOT_INHI, v);
    }
    if l.v[1] != l.v0[1] {
        let hi = lv_get(l, prm, SLOT_INHI) - 1;
        let v = l.v[1].min(hi).max(0);
        lv_set(l, prm, SLOT_INLO, v);
    }
    if l.v[2] != l.v0[2] {
        lv_set(l, prm, SLOT_OUTHI, l.v[2]);
    }
    if l.v[3] != l.v0[3] {
        lv_set(l, prm, SLOT_OUTLO, l.v[3]);
    }
    if l.gamma != l.gamma0 {
        lv_gamma_set(l, prm, l.gamma);
    }
}

// lv_slider(id, output) -- the vertical slider canvas of the input (2 triangles) or output
// (3 triangles) column with its mouse handling
fn lv_slider(l: &mut Levels, prm: &mut Params, ui: &mut Ui, id: u32, output: bool) {
    ui.next_size(-1, BAR_H);
    let inp = ui.canvas(id, None, 0, 0).clone();
    let r = inp.rect;
    // value of the pointer
    let pv = 255 - lv_px2val(inp.y, r[3]);
    // handles of this slider: value -> y px
    lv_handles(l, prm, r);
    // press: the nearest handle
    if inp.ev & EV_HOVER != 0 && inp.pressed & (1 << BTN_LEFT) != 0 {
        let mut best = i32::MAX;
        let mut h = 0;
        let (h0, h1) = if output { (H_OUTWHITE, H_OUTBLACK) } else { (H_INWHITE, H_INBLACK) };
        for k in h0..=h1 {
            let d = l.hy[k].wrapping_sub(inp.y).wrapping_sub(r[1]).wrapping_abs();
            if d < best {
                best = d;
                h = k;
            }
        }
        l.drag = h;
    }
    // only the slider that owns the dragged handle
    if l.drag != 0 && (l.drag >= H_OUTWHITE) == output {
        if inp.btn & (1 << BTN_LEFT) != 0 {
            lv_drag_to(l, prm, l.drag, pv);
            lv_handles(l, prm, r);
        } else {
            l.drag = 0;
        }
    }
    lv_slider_draw(l, ui, r, output);
}

// lv_handles(rect) -- hy[h] = absolute y (px) of every handle on the slider canvas, from
// FS_PARAM (shown channel)
fn lv_handles(l: &mut Levels, prm: &Params, r: [i32; 4]) {
    l.hy[H_INWHITE] = lv_sety(lv_get(l, prm, SLOT_INHI), r);
    l.hy[H_INBLACK] = lv_sety(lv_get(l, prm, SLOT_INLO), r);
    let hi = lv_get(l, prm, SLOT_OUTHI);
    l.hy[H_OUTWHITE] = lv_sety(hi, r);
    let lo = lv_get(l, prm, SLOT_OUTLO);
    l.hy[H_OUTBLACK] = lv_sety(lo, r);
    // grey: out_lo + (out_hi - out_lo) * 2^-gamma
    let g = lv_gamma_get(l, prm);
    let t = rt::x87::exp2(0.0 - g);
    l.hy[H_GREY] = lv_sety(cvtd(t * (hi - lo) as f64 + lo as f64), r);
}

// lv_sety(value, rect) -> y px of value on the slider
fn lv_sety(v: i32, r: [i32; 4]) -> i32 {
    (255i32.wrapping_sub(v)).wrapping_mul(r[3] - 1) / 255 + r[1]
}

// lv_px2val(px, size) -> round(px * 255 / (size - 1)) clamped
fn lv_px2val(px: i32, size: i32) -> i32 {
    if size < 2 {
        return 0;
    }
    let s = size - 1;
    ((px.wrapping_mul(510).wrapping_add(s)) / s >> 1).clamp(0, 255)
}

// lv_drag_to(handle, value) -- move a handle (input white / black keep in_lo < in_hi; the
// grey handle sets gamma = -log2((v - lo) / (hi - lo)), 0.1 .. 10, 2 decimals)
fn lv_drag_to(l: &Levels, prm: &mut Params, h: usize, v: i32) {
    match h {
        H_INWHITE => {
            let v = v.max(lv_get(l, prm, SLOT_INLO) + 1);
            lv_set(l, prm, SLOT_INHI, v);
        }
        H_INBLACK => {
            let v = v.min(lv_get(l, prm, SLOT_INHI) - 1);
            lv_set(l, prm, SLOT_INLO, v);
        }
        H_OUTWHITE => lv_set(l, prm, SLOT_OUTHI, v),
        H_OUTBLACK => lv_set(l, prm, SLOT_OUTLO, v),
        _ => {
            let lo = lv_get(l, prm, SLOT_OUTLO);
            let d = lv_get(l, prm, SLOT_OUTHI) - lo; // hi - lo
            if d == 0 {
                return;
            }
            let t = minsd(maxsd((v - lo) as f64 / d as f64, C_TMIN), C_TMAX);
            let g = minsd(maxsd(0.0 - rt::x87::log2(t), C_GMIN), C_GMAX);
            let g = cvtd64(g * 100.0) as f64 / 100.0;
            lv_gamma_set(l, prm, g);
        }
    }
}

// lv_slider_draw(rect, output) -- gradient bar and triangles
fn lv_slider_draw(l: &mut Levels, ui: &mut Ui, r: [i32; 4], output: bool) {
    // gradient bar (black at the bottom, white at the top): 12 DIP at the left
    lv_grad(l);
    let bw = ui.dp(12);
    let c = ui.color(T_CTRL_STROKE);
    ui.draw_color(c);
    ui.draw_frame(r[0], r[1], bw, r[3], 0, 1);
    if let Some(g) = &l.grad {
        ui.draw_image(g, 1, r[0] + 1, r[1] + 1, bw - 2, r[3] - 2);
    }
    // triangles right of the bar, pointing at it
    let (h0, h1) = if output { (H_OUTWHITE, H_OUTBLACK) } else { (H_INWHITE, H_INBLACK) };
    for h in h0..=h1 {
        lv_triangle(ui, r[0] + bw + 2, l.hy[h], LV_TRI_COL[h]);
    }
}

// lv_triangle(x, y, argb) -- a left-pointing triangle with its tip at (x, y): 10 x 12 DIP,
// outlined in the text colour
fn lv_triangle(ui: &mut Ui, x: i32, y: i32, fill: u32) {
    let hh = ui.dp(6);
    let len = ui.dp(11);
    let c = ui.color(T_TEXT);
    ui.draw_color(c);
    for s in 0..2 {
        // 0 outline, 1 fill (1 px smaller)
        let half = hh - s;
        for dy in -half..=half {
            if half == 0 {
                continue;
            }
            let rl = (half - dy.wrapping_abs()).wrapping_mul(len) / half; // rows shrink towards the tip
            let w = rl - s - s;
            if w <= 0 {
                continue;
            }
            ui.draw_rect(x + len - rl + s, y + dy, w, 1); // rows end at the triangle's base
        }
        if s == 0 {
            ui.draw_color(fill);
        }
    }
}

// lv_grad() -- grad: 1 x 256 PARGB, white at the top, black at the bottom (once)
fn lv_grad(l: &mut Levels) {
    if l.grad.is_some() {
        return;
    }
    let mut g = Surface::new(1, 256, FMT_PARGB);
    if g.px.is_empty() {
        return;
    }
    let st = g.stride as usize;
    for y in 0..256u32 {
        let v = (255 - y).wrapping_mul(0x010101) | 0xFF00_0000;
        g.px[y as usize * st..y as usize * st + 4].copy_from_slice(&v.to_le_bytes());
    }
    l.grad = Some(Rc::new(g));
}

// lv_auto() -- Automatisch: per channel lo / hi = 0.5 % / 99.5 % percentiles, gamma from the
// mean (FXA auto_level), output 0 / 255
fn lv_auto(l: &Levels, prm: &mut Params) {
    for c in 0..3 {
        let h = &l.hin[c];
        let lo = ::fx::hist::hist_percentile(h, l.count, C_P005);
        let hi = ::fx::hist::hist_percentile(h, l.count, C_P995);
        let md = (cvtt(::fx::hist::hist_mean(h) as f32 + 0.5f32) & 255) as u32;
        prm.set_byte(SLOT_INLO, c, lo as u8);
        prm.set_byte(SLOT_INHI, c, hi as u8);
        prm.set_byte(SLOT_OUTLO, c, 0);
        prm.set_byte(SLOT_OUTHI, c, 255);
        let mut g = 1.0f64;
        if lo < md && md < hi {
            let t = (md - lo) as i32 as f32 / (hi - lo) as i32 as f32;
            let lg = rt::x87::log2(t as f64);
            g = minsd(maxsd(-1.0 / lg, C_GMIN), C_GMAX) as f32 as f64;
        }
        prm.set_f(6 - c, g);
    }
}

// fnv(data, seed) -- hash_fnv64 (seed 0 = the offset basis)
fn fnv(d: &[u8], seed: u64) -> u64 {
    let mut h = if seed == 0 { 0xCBF2_9CE4_8422_2325 } else { seed };
    for &b in d {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01B3);
    }
    h
}

// lv_images() -- (re)build the histogram images when the dpi or the parameters changed
fn lv_images(p: &mut Pn, ui: &mut Ui) {
    let w = ui.dp(HIST_W);
    let h = ui.dp(BAR_H);
    let key = (w as u32) << 16 | h as u32; // size key
    let l = &mut p.levels;
    // input histogram: only the size
    if key != l.inkey {
        l.inkey = key;
        l.sfin = lv_hist_image(&l.hin, w, h);
        if l.sfin.is_some() {
            l.ver = l.ver.wrapping_add(1);
            l.ver_in = l.ver;
        }
    }
    // output histogram: the input through the current parameters
    let mut bytes = [0u8; 256];
    for (i, s) in p.fx.param.0.iter().enumerate() {
        bytes[i * 8..i * 8 + 8].copy_from_slice(&s.to_le_bytes());
    }
    let hk = fnv(&bytes, key as u64);
    if hk != l.outkey {
        l.outkey = hk;
        lv_out_hist(l, &p.fx.param);
        l.sfout = lv_hist_image(&l.hout, w, h);
        if l.sfout.is_some() {
            l.ver = l.ver.wrapping_add(1);
            l.ver_out = l.ver;
        }
    }
}

// lv_out_hist() -- hout = hin mapped through the kernel's levels table
fn lv_out_hist(l: &mut Levels, prm: &Params) {
    let lo32 = |s: usize| (prm.0[s] as u32).to_le_bytes();
    let mut lvl = Lvl { inlo: lo32(0), inhi: lo32(1), outlo: lo32(2), outhi: lo32(3), gamma: [0.0; 3] };
    for c in 0..3 {
        let g = prm.f(6 - c);
        let g = if g.is_nan() { 1.0 } else { g };
        lvl.gamma[c] = minsd(maxsd(g, C_GMIN), C_GMAX) as f32;
    }
    levels_lut(&mut l.lut, &lvl);
    l.hout = [[0; 256]; 4];
    for c in 0..3 {
        // unchecked kernel channel (slots 7 red, 8 green, 9 blue): identity
        let ident = prm.0[9 - c] == 0;
        for v in 0..256 {
            let d = if ident { v } else { l.lut[c * 256 + v] as usize };
            l.hout[c][d] = l.hout[c][d].wrapping_add(l.hin[c][v]);
        }
    }
}

// lv_hist_image(hist, w, h) -- the B G R histograms as a PARGB image of w x h px: value 255 at
// the top, counts to the right (scaled by the largest count, square-root scale), channels
// overlaid
fn lv_hist_image(hist: &[[u32; 256]; 4], w: i32, h: i32) -> Option<Rc<Surface>> {
    let mut sf = Surface::new(w, h, FMT_PARGB);
    if sf.px.is_empty() {
        return None;
    }
    let mut max = hist[..3].iter().flat_map(|c| c.iter()).copied().max().unwrap_or(0);
    if max == 0 {
        max = 1;
    }
    let scale = w as f64 / rt::fx::sqrtd(max as f64);
    let len = |c: u32| rt::fx::cvttd(rt::fx::sqrtd(c as f64) * scale) as u32;
    let st = sf.stride as usize;
    for row in 0..h as u32 {
        let v = 255 - ((row << 8) / h as u32) as usize;
        let (lb, lg, lr) = (len(hist[0][v]), len(hist[1][v]), len(hist[2][v]));
        let line = &mut sf.px[row as usize * st..];
        for x in 0..w as u32 {
            let k = (x < lb) as usize | ((x < lg) as usize) << 1 | ((x < lr) as usize) << 2;
            line[x as usize * 4..x as usize * 4 + 4].copy_from_slice(&LV_HCOL[k].to_le_bytes());
        }
    }
    Some(Rc::new(sf))
}
