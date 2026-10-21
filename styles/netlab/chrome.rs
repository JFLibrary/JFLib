// chrome -- NetLab metrics, content dialogs, info bars, status bar, caption buttons,
// title bars and frames of windows
//
//   metric(id)          SM_* values: dialogs 320 .. 520 DIP with 16 DIP padding, the buttons
//                       right-aligned at their natural width with the default first; 32 DIP
//                       title bar with a left title; 24 DIP status bar; 26 DIP dock tab
//                       strips with 22 DIP buttons
//   dialog_frame        4 DIP corners, the one soft shadow, the panel surface, the footer on
//                       the window colour below a 1 DIP line, a 1 DIP frame
//   infobar             severity tint with a 1 DIP frame in the severity colour, 3 DIP
//                       corners, a 16 DIP status icon, title, message, close button
//   statusbar           panel fill with a frame line at the top
//   caption_button      full-height 40 DIP buttons, a shade on hover, close red
//   window_titlebar     the panel colour with a frame line at the bottom (inactive: the
//                       window colour)

use ui::consts::*;
use ui::keys::{
    IC_CANCEL, IC_CHECK_CIRCLE_FILL, IC_CHROME_CLOSE, IC_CHROME_MAX, IC_CHROME_MIN, IC_CHROME_RESTORE,
    IC_ERROR_FILL, IC_INFO, IC_WARNING,
};
use ui::theme::color_mix;
use ui::{Sp, StyleChrome, Ui};

use crate::common::{baseline, fill_rrect, framed, shadow, text, R_CTRL, R_DIALOG};
use crate::Netlab;

const IB_BG: [u32; 4] = [T_INFO_BG, T_OK_BG, T_WARN_BG, T_CRIT_BG];
const IB_FG: [u32; 4] = [T_INFO, T_OK, T_WARN, T_CRIT];
const IB_ICON: [u32; 4] = [IC_INFO, IC_CHECK_CIRCLE_FILL, IC_WARNING, IC_ERROR_FILL];

const SM_TAB: [i32; SM_COUNT as usize] = [
    320,                         // SM_DLG_MIN
    520,                         // SM_DLG_MAX
    16,                          // SM_DLG_MARGIN
    (BOX_V | PAD_16) as i32,     // SM_DLG_CONTENT_BOX
    10,                          // SM_DLG_CONTENT_GAP
    TS_SUBTITLE as i32,          // SM_DLG_TITLE_TS
    (BOX_H | PAD_12) as i32,     // SM_DLG_FOOTER_BOX
    8,                           // SM_DLG_FOOTER_GAP
    24,                          // SM_SB_H
    8,                           // SM_SB_PAD
    12,                          // SM_SB_GAP
    32,                          // SM_TB_H
    40,                          // SM_TB_H_LARGE
    8,                           // SM_TB_PAD
    16,                          // SM_TB_ICON
    TS_BODY_STRONG as i32,       // SM_TB_TITLE_TS
    40,                          // SM_CAP_W
    30,                          // SM_CAP_W_TOOL
    10,                          // SM_WC_TITLE_PAD
    6,                           // SM_WC_TITLE_GAP
    5,                           // SM_GRIP
    12,                          // SM_GRIP_CORNER
    26,                          // SM_WIN_TBH_TOOL
    30,                          // SM_WIN_TBH_DIALOG
    0,                           // SM_CAP_LEFT
    2,                           // SM_DLG_BTN_ORDER
    26,                          // SM_DOCK_HDR
    22,                          // SM_DOCK_BTN
];

impl StyleChrome for Netlab {
    fn metric(&self, id: u32) -> i32 {
        if id < SM_COUNT { SM_TAB[id as usize] } else { 0 }
    }
    fn navigation_surface(&self, ui: &Ui) -> u32 {
        ui.color(T_LAYER)
    }
    fn dialog_smoke(&self, ui: &mut Ui, sp: &Sp) {
        let c = ui.color(T_SMOKE);
        ui.draw_color(c);
        ui.draw_rect(sp.x, sp.y, sp.w, sp.h);
    }
    fn dialog_frame(&self, ui: &mut Ui, sp: &Sp) {
        dialog_frame(ui, sp)
    }
    fn infobar_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        ui.ts_font(TS_BODY_STRONG);
        let tw = sp.label.map_or(0, |t| ui.text.width(t));
        ui.ts_font(TS_BODY);
        let mw = sp.label2.map_or(0, |m| ui.text.width(m));
        sp.s[0] = tw;
        sp.s[1] = mw;
        (ui.dp(12 + 16 + 8 + 10 + 8 + 24 + 6) + tw + mw, ui.dp(36))
    }
    fn infobar(&self, ui: &mut Ui, sp: &mut Sp) {
        infobar(ui, sp)
    }
    fn infobar_close(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let ev = sp.ev;
        if ev & EV_HOVER != 0 {
            let c = ui.color(if ev & EV_HELD != 0 { T_SUBTLE_PRESSED } else { T_SUBTLE_HOVER });
            let rad = ui.dp(2);
            fill_rrect(ui, r[0], r[1], r[2], r[3], rad, c);
        }
        let c = ui.color(T_TEXT2);
        ui.draw_color(c);
        let sz = ui.dp(12);
        let d = (r[2] - sz) >> 1;
        ui.draw_icon(IC_CANCEL, r[0] + d, r[1] + ((r[3] - sz) >> 1), sz << 8);
    }
    fn statusbar(&self, ui: &mut Ui, sp: &Sp) {
        let c = ui.color(T_LAYER);
        ui.draw_color(c);
        ui.draw_rect(sp.x, sp.y, sp.w, sp.h);
        let c = ui.color(T_LAYER_STROKE);
        ui.draw_color(c);
        let t = ui.hair();
        ui.draw_rect(sp.x, sp.y, sp.w, t);
    }
    fn caption_button(&self, ui: &mut Ui, sp: &Sp) {
        caption_button(ui, sp)
    }
    fn window_titlebar(&self, ui: &mut Ui, sp: &Sp) {
        let c = ui.color(if sp.value != 0 { T_LAYER } else { T_BG });
        ui.draw_color(c);
        ui.draw_rect(sp.x, sp.y, sp.w, sp.h);
        let c = ui.color(T_LAYER_STROKE);
        ui.draw_color(c);
        let h = ui.hair();
        ui.draw_rect(sp.x, sp.y + sp.h - h, sp.w, h);
    }
    fn window_border(&self, ui: &mut Ui, sp: &Sp) {
        let t = sp.value;
        let (w, h) = (sp.w, sp.h);
        let c = ui.color(T_STRONG_STROKE);
        ui.draw_color(c);
        ui.draw_rect(0, 0, w, t);
        ui.draw_rect(0, h - t, w, t);
        ui.draw_rect(0, t, t, h - t * 2);
        ui.draw_rect(w - t, t, t, h - t * 2);
    }
    fn window_dialog_fill(&self, ui: &mut Ui, sp: &Sp) {
        let c = ui.color(T_LAYER) | 0xFF00_0000;
        ui.draw_color(c);
        ui.draw_rect(sp.x, sp.y, sp.w, sp.h);
    }
}

/// dialog_frame: the panel surface with the footer (value = its height) on the window colour
/// below a frame line, 4 DIP corners, the soft shadow, a 1 DIP frame
fn dialog_frame(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let rad = ui.dp(R_DIALOG);
    let depth = ui.dp(16);
    shadow(ui, r, rad, depth, 0x30, 0x90);
    let panel = ui.color(T_LAYER) | 0xFF00_0000;
    fill_rrect(ui, r[0], r[1], r[2], r[3], rad, panel);
    let fh = sp.value;
    if fh > 0 {
        let bg = ui.color(T_BG) | 0xFF00_0000;
        ui.draw_clip(r[0], r[1] + r[3] - fh, r[2], fh);
        fill_rrect(ui, r[0], r[1], r[2], r[3], rad, bg);
        ui.draw_unclip();
        let c = ui.color(T_LAYER_STROKE);
        ui.draw_color(c);
        let h = ui.hair();
        ui.draw_rect(r[0], r[1] + r[3] - fh, r[2], h);
    }
    let stroke = ui.color(T_FLYOUT_STROKE);
    framed(ui, r[0], r[1], r[2], r[3], rad, 0, stroke);
}

/// infobar: severity tint with a frame in the severity colour, a 16 DIP status icon, title
/// (strong) and message; s[2] / s[3] = close button x / size
fn infobar(ui: &mut Ui, sp: &mut Sp) {
    let r = sp.rect();
    let sev = (sp.flags & 3) as usize;
    let tw = sp.s[0];
    let bg = ui.color(IB_BG[sev]) | 0xFF00_0000;
    let fg = ui.color(IB_FG[sev]) | 0xFF00_0000;
    let rad = ui.dp(R_CTRL);
    let edge = color_mix(bg, fg, 120);
    framed(ui, r[0], r[1], r[2], r[3], rad, bg, edge);
    // status icon
    let px = ui.dp(16);
    let left = r[0] + ui.dp(12);
    ui.draw_color(fg);
    ui.draw_icon(IB_ICON[sev], left, r[1] + ((r[3] - px) >> 1), px << 8);
    let cs = ui.dp(24);
    let cxb = r[0] + r[2] - ui.dp(6) - cs;
    let mut tx = left + px + ui.dp(8);
    if let Some(title) = sp.label {
        let y = baseline(ui, TS_BODY_STRONG, r[1], r[3]);
        let c = ui.color(T_TEXT);
        ui.ts_font(TS_BODY_STRONG);
        text(ui, tx, y, title, c, 0);
        tx += ui.dp(10) + tw;
    }
    if let Some(msg) = sp.label2 {
        let y = baseline(ui, TS_BODY, r[1], r[3]);
        let c = ui.color(T_TEXT2);
        ui.ts_font(TS_BODY);
        text(ui, tx, y, msg, c, (cxb - ui.dp(8) - tx).max(1));
    }
    sp.s[2] = cxb;
    sp.s[3] = cs;
}

/// caption_button: a shade on hover (close: red with a white glyph); glyphs: minimise,
/// maximise / restore, close (inactive window: disabled colour)
fn caption_button(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (ev, flags) = (sp.ev, sp.flags);
    let mut gc = ui.color(if sp.value != 0 { T_TEXT } else { T_TEXT_DISABLED });
    if ev & EV_HOVER != 0 {
        let c = if flags & HB_CLOSE != 0 {
            gc = 0xFFFF_FFFF;
            let c = ui.color(T_CLOSE_HOVER);
            if ev & EV_HELD != 0 { color_mix(c, 0xFF00_0000, 40) } else { c }
        } else {
            ui.color(if ev & EV_HELD != 0 { T_SUBTLE_PRESSED } else { T_SUBTLE_HOVER })
        };
        fill_rrect(ui, r[0], r[1], r[2], r[3], 0, c);
    }
    ui.draw_color(gc);
    let sz = ui.dp(10);
    let cp = if flags & HB_CLOSE != 0 {
        IC_CHROME_CLOSE
    } else if flags & HB_MIN != 0 {
        IC_CHROME_MIN
    } else if sp.icon == IC_CHROME_RESTORE {
        IC_CHROME_RESTORE
    } else {
        IC_CHROME_MAX
    };
    ui.draw_icon(cp, r[0] + ((r[2] - sz) >> 1), r[1] + ((r[3] - sz) >> 1), sz << 8);
}
