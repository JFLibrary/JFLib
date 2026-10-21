// adj_curves -- PaintCraft: Korrekturen > Kurven... (agent A6), paint.net 5.1's Curves dialog
// as a modal dialog window: "Transferzuordnung" Leuchtkraft / RGB, a 256 x 256 curve editor
// (click adds a control point, drag moves it, right click or dragging it out of the graph
// removes it, the pointer position shows as "(x, y)"), R / G / B check boxes in RGB mode,
// Zurücksetzen, OK / Abbrechen.  Sub-module of fx.s:
//   curves_open()   read the curves of FS_PARAM (FXA encoding), place the window
//   curves_frame()  the window (fx_frame calls it while g_fx + FS_OPEN)
//   curves_close()  nothing to free
// Parameter encoding (fx_adj.s, EFFECTS_ADJUST.md): slot 0 mode (0 luminosity, 1 rgb); blobs
// of 5 slots: luminosity 1, red 6, green 11, blue 16 = u8 n (<= 19, 0 = identity), n x
// {u8 x, u8 y}; the kernel interpolates with fx_spline_lut (the editor draws the same table).
// A curve that is exactly (0, 0) - (255, 255) is stored as n = 0.  In RGB mode an edit goes to
// the first checked channel and is copied to the other checked channels (their curves become
// equal), unchecked channels keep their curves.
// Widget ids ID_DLG_CURVES + 1 mode, + 2 graph (canvas), + 3..5 R G B, + 6 Zurücksetzen,
// + 7 coordinates text; window WINID_CURVES.
//                                              [apps/PaintCraft/src/adj_curves.s]

use alloc::format;
use alloc::string::String;

use ::fx::{Params, FX_NSLOTS};
use ui::*;

use crate::app::constants::*;
use crate::effects::{fx_params_changed, fx_place_dialog, FSM_DIALOG};
use crate::effects::controls::fxui_status;
use crate::app::formatting::fmt_cap;
use crate::app::Pn;

const CV_W: i32 = 312; // dialog width (DIP)
const GRAPH_DIP: i32 = 256; // editor size (1 DIP per value at 100 %)
const CV_MAXPTS: usize = 19;
const HIT_DIP: i32 = 6; // pick radius of a point
const OUT_DIP: i32 = 16; // dragging this far out of the graph removes the point
const ID_CV_MODE: u32 = ID_DLG_CURVES + 1;
const ID_CV_GRAPH: u32 = ID_DLG_CURVES + 2;
const ID_CV_RED: u32 = ID_DLG_CURVES + 3;
const ID_CV_GREEN: u32 = ID_DLG_CURVES + 4;
const ID_CV_BLUE: u32 = ID_DLG_CURVES + 5;
const ID_CV_RESET: u32 = ID_DLG_CURVES + 6;
const COORD_CAP: usize = 32;
static CV_COLORS: [u32; 3] = [0xFFE81123, 0xFF16A01E, 0xFF0078D4];
static CV_SLOTS: [usize; 4] = [1, 6, 11, 16];
static CV_MODES: [&str; 2] = ["Leuchtkraft", "RGB"];

/// the state of adj_curves.s (its g_* globals)
pub(crate) struct Curves {
    rect: Shared<WinRect>,
    /// g_cv_pts / g_cv_n: points per channel (x, y bytes; 20 points room)
    pts: [[u8; 40]; 4],
    n: [usize; 4],
    /// g_cv_mask: R G B check boxes
    mask: [i32; 3],
    /// dragged point of the edited channel, -1
    drag: i32,
    lut: [u8; 256],
    coord: String,
}

impl Default for Curves {
    fn default() -> Curves {
        Curves {
            rect: Shared::new(WinRect::new(0, 0, CV_W, 0, 0)),
            pts: [[0; 40]; 4],
            n: [0; 4],
            mask: [0; 3],
            drag: -1,
            lut: [0; 256],
            coord: String::new(),
        }
    }
}

/// curves_open() -- the four curves from FS_PARAM, all channels checked, placement
pub(crate) fn curves_open(p: &mut Pn, ui: &mut Ui) {
    for ch in 0..4 {
        cv_decode(&mut p.curves, &p.fx.param, ch);
    }
    let c = &mut p.curves;
    c.mask = [1, 1, 1];
    c.drag = -1;
    c.coord.clear();
    let r = c.rect.clone();
    fx_place_dialog(p, ui, &r, CV_W);
}

/// curves_close() -- nothing to free
pub(crate) fn curves_close(_p: &mut Pn, _ui: &mut Ui) {}

// cv_decode(ch) -- points / n of channel ch from its blob (n = 0: (0,0), (255,255)); points
// sorted by x, repeated x: the last one wins (as fx_spline_lut)
fn cv_decode(c: &mut Curves, prm: &Params, ch: usize) {
    let s = CV_SLOTS[ch & 3];
    let n = prm.byte(s, 0) as usize;
    if n >= 2 {
        let n = n.min(CV_MAXPTS);
        c.n[ch] = 0;
        for i in 0..n {
            cv_put(c, prm.byte(s, 1 + i * 2) as i32, prm.byte(s, 2 + i * 2) as i32, ch);
        }
        if c.n[ch] >= 2 {
            return;
        }
    }
    c.n[ch] = 2;
    c.pts[ch][..4].copy_from_slice(&[0, 0, 255, 255]);
}

// cv_put(x, y, ch) -> index -- insert (x, y) into channel ch sorted by x; an existing point
// with the same x is moved instead; -1 when full
fn cv_put(c: &mut Curves, x: i32, y: i32, ch: usize) -> i32 {
    let ch = ch & 3;
    let n = c.n[ch];
    let p = &mut c.pts[ch];
    let mut k = 0;
    while k < n {
        let px = p[k * 2] as i32;
        if px == x {
            p[k * 2 + 1] = y as u8;
            return k as i32;
        }
        if px > x {
            break;
        }
        k += 1;
    }
    if n >= CV_MAXPTS {
        return -1;
    }
    let mut i = n; // shift points k .. n-1 up
    while i > k {
        p[i * 2] = p[i * 2 - 2];
        p[i * 2 + 1] = p[i * 2 - 1];
        i -= 1;
    }
    p[k * 2] = x as u8;
    p[k * 2 + 1] = y as u8;
    c.n[ch] += 1;
    k as i32
}

// cv_remove(ch, k) -- delete point k of channel ch (keeps at least 2)
fn cv_remove(c: &mut Curves, ch: usize, k: usize) {
    let ch = ch & 3;
    let n = c.n[ch];
    if n <= 2 || k >= n {
        return;
    }
    let p = &mut c.pts[ch];
    for i in k + 1..n {
        p[i * 2 - 2] = p[i * 2];
        p[i * 2 - 1] = p[i * 2 + 1];
    }
    c.n[ch] -= 1;
}

// cv_encode(ch) -- the blob of channel ch from its points ((0,0)-(255,255) -> n = 0)
fn cv_encode(c: &Curves, prm: &mut Params, ch: usize) {
    let ch = ch & 3;
    let s = CV_SLOTS[ch];
    for k in 0..5 {
        prm.0[s + k] = 0;
    }
    let n = c.n[ch];
    let p = &c.pts[ch];
    if n == 2 && p[..4] == [0, 0, 255, 255] {
        return;
    }
    prm.set_byte(s, 0, n as u8);
    for i in 0..n {
        prm.set_byte(s, 1 + i * 2, p[i * 2]);
        prm.set_byte(s, 2 + i * 2, p[i * 2 + 1]);
    }
}

// cv_primary() -> channel being edited: 0 in Leuchtkraft mode, else the first checked of
// R G B (1..3); -1 = none checked
fn cv_primary(c: &Curves, prm: &Params) -> i32 {
    if prm.0[0] as u32 == 0 {
        return 0;
    }
    match c.mask.iter().position(|&m| m != 0) {
        Some(k) => k as i32 + 1,
        None => -1,
    }
}

// cv_commit(ch) -- the edited channel ch goes into its blob; in RGB mode its points are copied
// to the other checked channels too
fn cv_commit(c: &mut Curves, prm: &mut Params, ch: usize) {
    cv_encode(c, prm, ch);
    if ch == 0 {
        return;
    }
    for s in 1..4 {
        if s == ch || c.mask[s - 1] == 0 {
            continue;
        }
        c.n[s] = c.n[ch];
        c.pts[s] = c.pts[ch];
        cv_encode(c, prm, s);
    }
}

/// curves_frame() -- the Kurven window
pub(crate) fn curves_frame(p: &mut Pn, ui: &mut Ui) {
    let open = p.fx.open.clone();
    let rect = p.curves.rect.clone();
    let mut prev: Option<Params> = None;
    if ui.window_begin(WINID_CURVES, p.fx.name, Some(&open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&rect)) {
        prev = Some(p.fx.param);
        // Transferzuordnung
        ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
        ui.text(TS_BODY, crate::preferences::localization::tr("Transferzuordnung:"));
        ui.next_size(140, 0);
        let mut m = p.fx.param.0[0] as u32 as i32;
        if ui.combobox(ID_CV_MODE, &mut m, &CV_MODES) & EV_ACT != 0 {
            p.curves.drag = -1;
        }
        p.fx.param.0[0] = (p.fx.param.0[0] & !0xFFFF_FFFF) | m as u32 as u64;
        ui.box_end();
        // the graph
        ui.next_size(GRAPH_DIP, GRAPH_DIP);
        let inp = ui.canvas(ID_CV_GRAPH, None, 0, 0).clone();
        cv_input(p, ui, &inp);
        cv_draw(p, ui, inp.rect);
        // R G B
        let c = &mut p.curves;
        if p.fx.param.0[0] as u32 != 0 {
            ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
            ui.checkbox(ID_CV_RED, crate::preferences::localization::tr("Rot"), &mut c.mask[0]);
            ui.checkbox(ID_CV_GREEN, crate::preferences::localization::tr("Grün"), &mut c.mask[1]);
            ui.checkbox(ID_CV_BLUE, crate::preferences::localization::tr("Blau"), &mut c.mask[2]);
            ui.box_end();
        }
        ui.next_size(-1, 0);
        ui.text(TS_BODY, &c.coord);
        ui.text(TS_CAPTION | TX_SECONDARY, crate::preferences::localization::tr("Tipp: Ein Rechtsklick auf einen"));
        ui.text(TS_CAPTION | TX_SECONDARY, crate::preferences::localization::tr("Kurvenpunkt löscht ihn."));
        ui.box_begin(0, BOX_H, 8);
        if ui.button(ID_CV_RESET, crate::preferences::localization::tr("Zurücksetzen")) & EV_ACT != 0 {
            cv_reset(c, &mut p.fx.param);
        }
        ui.box_end();
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

// cv_reset() -- Zurücksetzen: every curve of the current mode back to the straight line
fn cv_reset(c: &mut Curves, prm: &mut Params) {
    for ch in 0..4 {
        c.n[ch] = 2;
        c.pts[ch][..4].copy_from_slice(&[0, 0, 255, 255]);
        cv_encode(c, prm, ch);
    }
    c.drag = -1;
}

// cv_input(ui) -- mouse on the graph: add / move / remove points of the edited channel, the
// coordinates text
fn cv_input(p: &mut Pn, ui: &mut Ui, inp: &UiInput) {
    let (sx, sy) = (inp.x, inp.y);
    let (rw, rh) = (inp.rect[2], inp.rect[3]);
    // value coordinates (1 value per DIP at 256 DIP: exact at 100 %)
    let vx = px_to_val(sx, rw);
    let vy = 255 - px_to_val(sy, rh);
    let hit_r = ui.dp(HIT_DIP);
    let out = ui.dp(OUT_DIP);
    let c = &mut p.curves;
    let prm = &mut p.fx.param;
    let ch = cv_primary(c, prm);
    let mut changed = false;
    if ch >= 0 {
        let chu = ch as usize;
        // left press: pick or add a point
        if inp.ev & EV_HOVER != 0 {
            if inp.pressed & (1 << BTN_LEFT) != 0 {
                let mut k = cv_hit(c, chu, sx, sy, rw, rh, hit_r);
                if k < 0 {
                    k = cv_put(c, vx, vy, chu);
                    if k >= 0 {
                        changed = true;
                    }
                }
                if k >= 0 {
                    c.drag = k;
                }
            } else if inp.pressed & (1 << BTN_RIGHT) != 0 {
                let k = cv_hit(c, chu, sx, sy, rw, rh, hit_r);
                if k >= 0 {
                    cv_remove(c, chu, k as usize);
                    c.drag = -1;
                    changed = true;
                }
            }
        }
        if c.drag >= 0 {
            if inp.btn & (1 << BTN_LEFT) != 0 {
                // dragged far out of the graph: the point goes away
                if sx < -out || sy < -out || sx > rw + out || sy > rh + out {
                    if c.n[chu] > 2 {
                        cv_remove(c, chu, c.drag as usize);
                        changed = true;
                        c.drag = -1;
                    }
                } else {
                    let k = c.drag as usize;
                    changed |= cv_move(c, chu, k, vx, vy);
                }
            } else {
                c.drag = -1;
            }
        }
        if changed {
            cv_commit(c, prm, chu);
            ui.repass();
        }
    }
    // "(x, y)" while the pointer is over the graph or a point is dragged
    if inp.ev & EV_HOVER != 0 || c.drag >= 0 {
        c.coord = fmt_cap(format!("({}, {})", vx.clamp(0, 255), vy.clamp(0, 255)), COORD_CAP);
    } else {
        c.coord.clear();
    }
}

// px_to_val(px, size px) -> round(px * 255 / (size - 1)) clamped to 0..255
fn px_to_val(px: i32, size: i32) -> i32 {
    if size < 2 {
        return 0;
    }
    let s = size - 1;
    ((px.wrapping_mul(510).wrapping_add(s)) / s >> 1).clamp(0, 255)
}

// val_to_fx(value, size px) -> offset in 24.8 px of value 0..255
fn val_to_fx(v: i32, size: i32) -> i32 {
    (v.wrapping_mul(size - 1) << 8) / 255
}

// cv_hit(ch, x, y) -> index of the point of ch within HIT_DIP of (x, y) px, -1 = none
fn cv_hit(c: &Curves, ch: usize, x: i32, y: i32, rw: i32, rh: i32, r: i32) -> i32 {
    let r2 = r.wrapping_mul(r);
    let mut best = i32::MAX;
    let mut k = -1;
    let p = &c.pts[ch & 3];
    for i in 0..c.n[ch & 3] {
        let dx = (val_to_fx(p[i * 2] as i32, rw) >> 8) - x;
        let dy = (val_to_fx(255 - p[i * 2 + 1] as i32, rh) >> 8) - y;
        let d = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
        if d <= r2 && d < best {
            best = d;
            k = i as i32;
        }
    }
    k
}

// cv_move(ch, k, vx, vy) -> true changed -- point k to (vx, vy), x kept strictly between its
// neighbours
fn cv_move(c: &mut Curves, ch: usize, k: usize, vx: i32, vy: i32) -> bool {
    let n = c.n[ch];
    if k >= n {
        return false;
    }
    let p = &mut c.pts[ch];
    // x range: prev.x + 1 .. next.x - 1 (0 / 255 at the ends)
    let lo = if k == 0 { 0 } else { p[k * 2 - 2] as i32 + 1 };
    let hi = if k + 1 < n { p[k * 2 + 2] as i32 - 1 } else { 255 };
    let x = vx.max(lo);
    let x = if x > hi { hi } else { x };
    let y = vy.clamp(0, 255);
    if p[k * 2] == x as u8 && p[k * 2 + 1] == y as u8 {
        return false;
    }
    p[k * 2] = x as u8;
    p[k * 2 + 1] = y as u8;
    true
}

// dp_fx256(dip_fx) -> 24.8 px of a 24.8 DIP length
fn dp_fx256(ui: &Ui, v: i32) -> i32 {
    ui.dpi().wrapping_mul(v) / 96
}

// cv_draw(rect) -- grid, the curves (fx_spline_lut, as the kernel), the points of the edited
// channel
fn cv_draw(p: &mut Pn, ui: &mut Ui, r: [i32; 4]) {
    let (rx, ry, rw, rh) = (r[0], r[1], r[2], r[3]);
    // background and frame
    let c = ui.color(T_CTRL);
    ui.draw_color(c);
    ui.draw_rect(rx, ry, rw, rh);
    let c = ui.color(T_DIVIDER);
    ui.draw_color(c);
    for i in 1..4u32 {
        let x = rx + ((rw as u32).wrapping_mul(i) >> 2) as i32;
        ui.draw_rect(x, ry, 1, rh);
        let y = ry + ((rh as u32).wrapping_mul(i) >> 2) as i32;
        ui.draw_rect(rx, y, rw, 1);
    }
    let c = ui.color(T_CTRL_STROKE);
    ui.draw_color(c);
    ui.draw_frame(rx, ry, rw, rh, 0, 1);
    // clip the curves to the graph
    ui.draw_clip(rx, ry, rw, rh);
    let cv = &mut p.curves;
    let primary = cv_primary(cv, &p.fx.param);
    // curves: luminosity (text colour) or R G B (unchecked ones faint)
    if p.fx.param.0[0] as u32 == 0 {
        let c = ui.color(T_TEXT);
        cv_curve(cv, ui, 0, c, r);
    } else {
        for ch in 1..4 {
            let mut c = CV_COLORS[ch - 1];
            if cv.mask[ch - 1] == 0 {
                c = (c & 0x00FF_FFFF) | 0x6000_0000;
            }
            cv_curve(cv, ui, ch, c, r);
        }
    }
    ui.draw_unclip(); // the points may reach over the frame
    // the points of the edited channel
    if primary < 0 {
        return;
    }
    let rad = dp_fx256(ui, 1024); // 4 DIP
    let ch = primary as usize;
    for k in 0..cv.n[ch] {
        let x = val_to_fx(cv.pts[ch][k * 2] as i32, rw) + (rx << 8) + 128;
        let y = val_to_fx(255 - cv.pts[ch][k * 2 + 1] as i32, rh) + (ry << 8) + 128;
        // ring in the text colour, filled with the control colour (accent while dragged)
        let c = ui.color(T_TEXT);
        ui.draw_color(c);
        ui.draw_circle(x, y, rad, 0);
        let c = ui.color(if k as i32 == cv.drag { T_ACCENT } else { T_CTRL });
        ui.draw_color(c);
        ui.draw_circle(x, y, rad - ((rad as u32) >> 2) as i32, 0);
    }
}

// cv_curve(ch, argb) -- the curve of channel ch through the kernel's spline table, as line
// segments every 4 values over the graph
fn cv_curve(cv: &mut Curves, ui: &mut Ui, ch: usize, argb: u32, r: [i32; 4]) {
    ui.draw_color(argb);
    let n = cv.n[ch];
    ::fx::lut::spline_lut(&mut cv.lut, &cv.pts[ch][..n * 2], n);
    let lw = dp_fx256(ui, 384); // 1.5 DIP lines
    let mut prev: Option<(i32, i32)> = None;
    let mut xv = 0usize;
    loop {
        let v = xv.min(255);
        let x = val_to_fx(v as i32, r[2]) + (r[0] << 8) + 128;
        let y = val_to_fx(255 - cv.lut[v] as i32, r[3]) + (r[1] << 8) + 128;
        if let Some((px, py)) = prev {
            ui.draw_line(px, py, x, y, lw);
        }
        prev = Some((x, y));
        if xv >= 255 {
            break;
        }
        xv += 4;
        if xv > 255 {
            xv = 255;
        }
    }
}

const _: () = assert!(FX_NSLOTS >= 21);
