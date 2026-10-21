// chrome -- the classic metrics, content dialogs, info bar, status bar, title bars (navy
// gradient), caption buttons, window frames                       [styles/classic/chrome.s]
//
//   metric(id)                          SM_* layout values (style.inc)
//   dialog_smoke, dialog_frame          no dim layer; grey face with a raised frame
//   infobar_size / infobar / infobar_close   severity background with a thin sunken frame,
//                                       coloured square with a glyph, bold title, message,
//                                       18 DIP flat close button
//   statusbar                           grey face with a thin raised top edge
//   window_titlebar                     navy-to-light-blue gradient (16 bands), greys inactive
//   caption_button                      small raised button inset 2 DIP in the bar
//   window_border, window_dialog_fill   raised frame; grey face
//
// Contract: ASMLIB/inc/style.inc (lib/ui/style.rs).

use ui::consts::*;
use ui::keys::{IC_CANCEL, IC_CHECK_MARK, IC_INFO, IC_WARNING};
use ui::theme::color_mix;
use ui::{Sp, StyleChrome, Ui};

use crate::common::*;
use crate::Classic;

// info bar colours / glyphs per severity (IB_INFO, IB_SUCCESS, IB_WARNING, IB_ERROR)
const IB_BG: [u32; 4] = [T_INFO_BG, T_OK_BG, T_WARN_BG, T_CRIT_BG];
const IB_FG: [u32; 4] = [T_INFO, T_OK, T_WARN, T_CRIT];
const IB_GLYPH: [u32; 4] = [IC_INFO, IC_CHECK_MARK, IC_WARNING, IC_CANCEL];

// the SM_* table
const SM_TAB: [i32; SM_COUNT as usize] = [
    280,                                      // SM_DLG_MIN
    520,                                      // SM_DLG_MAX
    24,                                       // SM_DLG_MARGIN
    (BOX_V | PAD_12) as i32,                  // SM_DLG_CONTENT_BOX
    8,                                        // SM_DLG_CONTENT_GAP
    TS_BODY_STRONG as i32,                    // SM_DLG_TITLE_TS
    (BOX_H | PAD_12) as i32,                  // SM_DLG_FOOTER_BOX
    6,                                        // SM_DLG_FOOTER_GAP
    22,                                       // SM_SB_H
    6,                                        // SM_SB_PAD
    12,                                       // SM_SB_GAP
    22,                                       // SM_TB_H
    30,                                       // SM_TB_H_LARGE
    4,                                        // SM_TB_PAD
    16,                                       // SM_TB_ICON
    (TS_BODY_STRONG | TX_ON_ACCENT) as i32,   // SM_TB_TITLE_TS
    22,                                       // SM_CAP_W
    20,                                       // SM_CAP_W_TOOL
    6,                                        // SM_WC_TITLE_PAD
    4,                                        // SM_WC_TITLE_GAP
    4,                                        // SM_GRIP
    12,                                       // SM_GRIP_CORNER
    20,                                       // SM_WIN_TBH_TOOL
    22,                                       // SM_WIN_TBH_DIALOG
    0,                                        // SM_CAP_LEFT
    0,                                        // SM_DLG_BTN_ORDER
    32,                                       // SM_DOCK_HDR
    28,                                       // SM_DOCK_BTN
];

impl StyleChrome for Classic {
    fn metric(&self, id: u32) -> i32 {
        // 0 for an unknown id
        if id < SM_COUNT { SM_TAB[id as usize] } else { 0 }
    }
    // classic dialogs do not dim the window behind them
    fn dialog_smoke(&self, _ui: &mut Ui, _sp: &Sp) {}
    fn dialog_frame(&self, ui: &mut Ui, sp: &Sp) {
        // grey face with a raised window frame
        let c = ui.color(T_BG);
        fill(ui, sp.rect(), c);
        frame(ui, sp.rect(), FR_RAISED);
    }
    fn infobar_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        infobar_size(ui, sp)
    }
    fn infobar(&self, ui: &mut Ui, sp: &mut Sp) {
        infobar(ui, sp)
    }
    fn infobar_close(&self, ui: &mut Ui, sp: &Sp) {
        // small flat button (thin raised on hover, sunken pressed), x glyph
        let ev = sp.ev;
        if ev & EV_HOVER != 0 {
            frame(ui, sp.rect(), if ev & EV_HELD != 0 { FR_THIN_SUNKEN } else { FR_THIN_RAISED });
        }
        let c = ui.color(T_TEXT);
        icon_mid(ui, sp.rect(), IC_CANCEL, 8, c);
    }
    fn statusbar(&self, ui: &mut Ui, sp: &Sp) {
        // grey face with a thin raised top edge
        let c = ui.color(T_LAYER);
        fill(ui, sp.rect(), c);
        frame(ui, sp.rect(), FR_THIN_RAISED);
    }
    fn caption_button(&self, ui: &mut Ui, sp: &Sp) {
        caption_button(ui, sp)
    }
    fn window_titlebar(&self, ui: &mut Ui, sp: &Sp) {
        window_titlebar(ui, sp)
    }
    fn window_border(&self, ui: &mut Ui, sp: &Sp) {
        // raised frame around a secondary window
        frame(ui, sp.rect(), FR_RAISED);
    }
    fn window_dialog_fill(&self, ui: &mut Ui, sp: &Sp) {
        // dialog windows show the grey face everywhere
        let c = ui.color(T_BG);
        fill(ui, sp.rect(), c);
    }
}

/// sty_infobar_size -> (w, h): 8 + 16 icon + 8 + title (bold) + 8 + message + 8 + 18 close
/// + 4, 30 DIP high; s[0] / s[1] = title / message width
fn infobar_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_BODY_STRONG);
    let tw = match sp.label {
        Some(t) => ui.text.width(t),
        None => 0,
    };
    sp.s[0] = tw;
    ui.ts_font(TS_BODY);
    let mw = match sp.label2 {
        Some(m) => ui.text.width(m),
        None => 0,
    };
    sp.s[1] = mw;
    let w = tw + mw + ui.dp(8 + 16 + 8 + 8 + 8 + 18 + 4);
    (w, ui.dp(30))
}

/// sty_infobar: severity background with a thin sunken frame, a coloured square with "i" /
/// "!" / a glyph, bold title, message; sets s[2] / s[3] = close button x / size
fn infobar(ui: &mut Ui, sp: &mut Sp) {
    let r = sp.rect();
    let sev = (sp.flags & 3) as usize;
    let c = ui.color(IB_BG[sev]);
    fill(ui, r, c);
    frame(ui, r, FR_THIN_SUNKEN);
    // icon: 16 DIP square in the severity colour, white glyph
    let d8 = ui.dp(8);
    let x = d8.wrapping_add(sp.x);
    let s = ui.dp(16);
    let sq = [x, (sp.h.wrapping_sub(s) >> 1).wrapping_add(sp.y), s, s];
    let c = ui.color(IB_FG[sev]);
    fill(ui, sq, c);
    icon_mid(ui, sq, IB_GLYPH[sev], 10, 0xFFFF_FFFF);
    // title and message
    let mut x = sq[0].wrapping_add(sq[2]).wrapping_add(d8);
    let tc = ui.color(T_TEXT);
    if sp.label.is_some() {
        ui.ts_font(TS_BODY_STRONG);
        text_at(ui, x, r, sp.label, tc, TS_BODY_STRONG);
        x = x.wrapping_add(sp.s[0].wrapping_add(d8));
    }
    if sp.label2.is_some() {
        ui.ts_font(TS_BODY);
        text_at(ui, x, r, sp.label2, tc, TS_BODY);
    }
    sp.s[3] = ui.dp(18);
    let d4 = ui.dp(4);
    sp.s[2] = sp.x.wrapping_add(sp.w).wrapping_sub(d4).wrapping_sub(sp.s[3]);
}

/// sty_window_titlebar(sp): navy-to-light-blue gradient while active (value 1), greys
/// otherwise (16 vertical bands)
fn window_titlebar(ui: &mut Ui, sp: &Sp) {
    let (a, b) = if sp.value != 0 { (0xFF0A_246A, 0xFFA6_CAF0) } else { (0xFF80_8080, 0xFFC0_C0C0) };
    for i in 0..16 {
        let c = color_mix(a, b, i << 4);
        let s = sp.w.wrapping_mul(i) >> 4; // band start
        let e = sp.w.wrapping_mul(i + 1) >> 4;
        rect(ui, sp.x.wrapping_add(s), sp.y, e.wrapping_sub(s), sp.h, c);
    }
}

/// sty_caption_button(sp): small raised button inset 2 DIP in the bar, glyph (pressed: dark
/// frame)
fn caption_button(ui: &mut Ui, sp: &Sp) {
    let d = ui.dp(2);
    let b = inset(sp.rect(), d);
    let c = pal(ui, CP_FACE);
    fill(ui, b, c);
    let held = sp.ev & (EV_HOVER | EV_HELD) == EV_HOVER | EV_HELD;
    frame(ui, b, if held { FR_PRESSED } else { FR_RAISED });
    let c = ui.color(T_TEXT);
    icon_mid(ui, b, sp.icon, 8, c);
}
