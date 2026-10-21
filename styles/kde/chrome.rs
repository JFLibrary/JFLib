// chrome -- the kde metrics, dialogs, message widgets (info bar), status bar, Breeze window
// decoration, secondary window frames                                [styles/kde/chrome.s]
//
//   metric(id)                          SM_* layout values (style.inc): dialog buttons right-
//                                       aligned, default first (SM_DLG_BTN_ORDER 2), centred
//                                       titles
//   dialog_smoke, dialog_frame          the dim layer behind a content dialog; deep soft
//                                       shadow, one window-coloured surface, 5 DIP corners,
//                                       hairline
//   infobar_size / infobar / infobar_close   severity card, icon disc, title, message,
//                                       28 DIP round close button
//   statusbar                           layer fill with a hairline at the top
//   caption_button                      Breeze decoration button: chevron / x glyph, round
//                                       plate on hover (close: red)
//   window_titlebar, window_border, window_dialog_fill   frames of secondary windows
//
// Contract: ASMLIB/inc/style.inc (lib/ui/style.rs).

use ui::consts::*;
use ui::keys::{IC_CANCEL, IC_CHECK_MARK, IC_CHEV_DOWN, IC_CHEV_UP, IC_CHEV_UP_DOWN, IC_CHROME_CLOSE, IC_CHROME_RESTORE};
use ui::theme::color_mix;
use ui::{Sp, StyleChrome, Ui};

use crate::common::{baseline, fill_rrect, stroke_rrect, text};
use crate::Kde;

const DLG_STROKE: u32 = 0x4080_8080; // hairline around alerts and windows (both themes)
const DLG_SHADOW_A: u32 = 0x40; // shadow alpha at the alert edge (light / dark theme)
const DLG_SHADOW_A_DARK: u32 = 0x80;

// info bar colours per severity (IB_INFO, IB_SUCCESS, IB_WARNING, IB_ERROR)
const IB_BG: [u32; 4] = [T_INFO_BG, T_OK_BG, T_WARN_BG, T_CRIT_BG];
const IB_FG: [u32; 4] = [T_INFO, T_OK, T_WARN, T_CRIT];

// the SM_* table
const SM_TAB: [i32; SM_COUNT as usize] = [
    300,                         // SM_DLG_MIN
    500,                         // SM_DLG_MAX
    24,                          // SM_DLG_MARGIN
    (BOX_V | PAD_16) as i32,     // SM_DLG_CONTENT_BOX
    10,                          // SM_DLG_CONTENT_GAP
    TS_SUBTITLE as i32,          // SM_DLG_TITLE_TS
    (BOX_H | PAD_16) as i32,     // SM_DLG_FOOTER_BOX
    6,                           // SM_DLG_FOOTER_GAP
    28,                          // SM_SB_H
    8,                           // SM_SB_PAD
    12,                          // SM_SB_GAP
    30,                          // SM_TB_H
    48,                          // SM_TB_H_LARGE
    8,                           // SM_TB_PAD
    16,                          // SM_TB_ICON
    (TS_BODY | TX_CENTER) as i32, // SM_TB_TITLE_TS
    30,                          // SM_CAP_W
    26,                          // SM_CAP_W_TOOL
    10,                          // SM_WC_TITLE_PAD
    6,                           // SM_WC_TITLE_GAP
    5,                           // SM_GRIP
    12,                          // SM_GRIP_CORNER
    26,                          // SM_WIN_TBH_TOOL
    30,                          // SM_WIN_TBH_DIALOG
    0,                           // SM_CAP_LEFT
    2,                           // SM_DLG_BTN_ORDER
    32,                          // SM_DOCK_HDR
    28,                          // SM_DOCK_BTN
];

impl StyleChrome for Kde {
    fn metric(&self, id: u32) -> i32 {
        // 0 for an unknown id
        if id < SM_COUNT { SM_TAB[id as usize] } else { 0 }
    }
    fn dialog_smoke(&self, ui: &mut Ui, sp: &Sp) {
        // the dim layer over the window behind a content dialog
        let c = ui.color(T_SMOKE);
        ui.draw_color(c);
        ui.draw_rect(sp.x, sp.y, sp.w, sp.h);
    }
    fn dialog_frame(&self, ui: &mut Ui, sp: &Sp) {
        dialog_frame(ui, sp)
    }
    fn infobar_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        infobar_size(ui, sp)
    }
    fn infobar(&self, ui: &mut Ui, sp: &mut Sp) {
        infobar(ui, sp)
    }
    fn infobar_close(&self, ui: &mut Ui, sp: &Sp) {
        infobar_close(ui, sp)
    }
    fn statusbar(&self, ui: &mut Ui, sp: &Sp) {
        // layer fill with a hairline at the top
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
        window_titlebar(ui, sp)
    }
    fn window_border(&self, ui: &mut Ui, sp: &Sp) {
        window_border(ui, sp)
    }
    fn window_dialog_fill(&self, ui: &mut Ui, sp: &Sp) {
        // dialog windows show the window colour everywhere
        let c = ui.color(T_BG);
        ui.draw_color(c);
        ui.draw_rect(sp.x, sp.y, sp.w, sp.h);
    }
}

// ---------------------------------------------------------------- content dialogs

/// sty_dialog_frame: alert panel: deep soft shadow, one window-coloured surface (the footer
/// is not set apart), 5 DIP corners, hairline
fn dialog_frame(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let rad = ui.dp(5);
    // soft shadow: cmd_shadow stacks `depth` layers, so the layer alpha is chosen to give
    // about DLG_SHADOW_A at the edge: a = A * 4d / ((d + 1)(d + 2))
    let depth = ui.dp(16);
    let a0 = if ui.theme_dark() { DLG_SHADOW_A_DARK } else { DLG_SHADOW_A };
    let num = ((depth << 2) as u32).wrapping_mul(a0);
    let den = (depth.wrapping_add(1).wrapping_mul(depth.wrapping_add(2))) as u32;
    let a = ((num / den) as i32).clamp(1, 255);
    ui.draw_color((a as u32) << 24);
    ui.cmd_shadow(r[0], r[1], r[2], r[3], rad, depth);
    let bg = ui.color(T_BG) | 0xFF00_0000;
    fill_rrect(ui, r[0], r[1], r[2], r[3], rad, bg);
    let t = ui.hair();
    stroke_rrect(ui, r[0], r[1], r[2], r[3], rad, t, DLG_STROKE);
}

// ---------------------------------------------------------------- info bar

/// sty_infobar_size: 16 + 16 icon + 12 + title (body strong) + 12 + message + 8 + 38 close
/// + 5, 40 high; keeps s[0] / s[1] = title / message width
fn infobar_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    ui.ts_font(TS_BODY_STRONG);
    let tw = match sp.label {
        Some(t) => ui.text.width(t),
        None => 0,
    };
    ui.ts_font(TS_BODY);
    let mw = match sp.label2 {
        Some(m) => ui.text.width(m),
        None => 0,
    };
    let w = ui.dp(16 + 16 + 12 + 12 + 8 + 38 + 5).wrapping_add(tw).wrapping_add(mw);
    let h = ui.dp(40);
    sp.s[0] = tw;
    sp.s[1] = mw;
    (w, h)
}

/// sty_infobar: card in the severity tint with an outline in the severity colour (4 DIP
/// corners), 16 DIP icon disc with a glyph, title and message; sets s[2] / s[3] = x / size of
/// the close button (a square, vertically centred)
fn infobar(ui: &mut Ui, sp: &mut Sp) {
    let r = sp.rect();
    let sev = (sp.flags & 3) as usize; // severity IB_INFO .. IB_ERROR
    let tw = sp.s[0];
    // background card in the severity tint, outline in the severity colour
    let bg = ui.color(IB_BG[sev]);
    let fg = ui.color(IB_FG[sev]);
    let rad = ui.dp(4);
    fill_rrect(ui, r[0], r[1], r[2], r[3], rad, bg);
    let t = ui.hair();
    let rad = ui.dp(4);
    stroke_rrect(ui, r[0], r[1], r[2], r[3], rad, t, fg);
    // icon: 16 DIP disc in the severity colour with a knocked-in glyph
    let c = ui.color(IB_FG[sev]);
    ui.draw_color(c);
    let left = r[0].wrapping_add(ui.dp(16)); // disc left
    let crad = ui.dp_fx(8 * 256); // radius (24.8)
    let cx = (left << 8).wrapping_add(crad);
    let cy = (r[3] << 7).wrapping_add(r[1] << 8);
    ui.draw_circle(cx, cy, crad, 0);
    let on = ui.color(T_ON_ACCENT);
    ui.draw_color(on);
    let sv = sev as u32;
    if sv == IB_SUCCESS || sv == IB_ERROR {
        let sz = ui.dp(10);
        let x = (cx >> 8).wrapping_sub(sz >> 1);
        let y = (cy >> 8).wrapping_sub(sz >> 1);
        let ic = if sv == IB_SUCCESS { IC_CHECK_MARK } else { IC_CANCEL };
        ui.draw_icon(ic, x, y, sz << 8);
    } else {
        // "i" / "!" in bold
        let size = ui.dp_fx(12 * 256);
        ui.text.set_font(FONT_BOLD, size);
        let s = if sv == IB_WARNING { "!" } else { "i" };
        let w = ui.text.width(s);
        let x = (cx >> 8).wrapping_sub(w >> 1);
        let asc = ui.text.ascent();
        let y = (cy >> 8).wrapping_add(asc.wrapping_mul(70) >> 8); // cap height ~ 0.7 ascent
        text(ui, x, y, s, on, 0);
    }
    // close button (28 DIP, 6 DIP from the right edge)
    let cs = ui.dp(28);
    let cxb = r[0].wrapping_add(r[2]).wrapping_sub(ui.dp(6)).wrapping_sub(cs);
    // title and message
    let mut tx = ui.dp(16 + 16 + 12).wrapping_add(r[0]);
    if let Some(title) = sp.label {
        let y = baseline(ui, TS_BODY_STRONG, r[1], r[3]);
        let c = ui.color(T_TEXT);
        text(ui, tx, y, title, c, 0);
        tx = tx.wrapping_add(ui.dp(12).wrapping_add(tw));
    }
    if let Some(msg) = sp.label2 {
        let y = baseline(ui, TS_BODY, r[1], r[3]);
        let c = ui.color(T_TEXT);
        let mut mw = cxb.wrapping_sub(ui.dp(8)).wrapping_sub(tx);
        if mw <= 0 {
            mw = 1;
        }
        text(ui, tx, y, msg, c, mw);
    }
    sp.s[2] = cxb;
    sp.s[3] = cs;
}

/// sty_infobar_close: round subtle hover / pressed disc, cancel glyph (ev)
fn infobar_close(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let ev = sp.ev;
    if ev & EV_HOVER != 0 {
        let c = ui.color(if ev & EV_HELD != 0 { T_SUBTLE_PRESSED } else { T_SUBTLE_HOVER });
        let rad = ((r[2] as u32) >> 1) as i32;
        fill_rrect(ui, r[0], r[1], r[2], r[3], rad, c);
    }
    let c = ui.color(if ev & EV_HELD != 0 { T_TEXT2 } else { T_TEXT });
    ui.draw_color(c);
    let sz = ui.dp(10);
    let d = r[2].wrapping_sub(sz) >> 1;
    ui.draw_icon(IC_CANCEL, d.wrapping_add(r[0]), d.wrapping_add(r[1]), sz << 8);
}

// ---------------------------------------------------------------- caption buttons

/// sty_caption_button: Breeze decoration button: glyph (minimise: chevron down, maximise:
/// chevron up, restore: up / down, close: x) in the text colour (inactive window: disabled
/// colour); on hover a round 18 DIP plate behind it: close red with a white glyph, the others
/// a translucent text-coloured plate; darker while pressed.  ev, flags HB_*, icon, value 1 =
/// active
fn caption_button(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (ev, flags) = (sp.ev, sp.flags);
    // geometry (fx)
    let rad = ui.dp_fx(9 * 256);
    let cx = (r[0] << 8).wrapping_add(r[2] << 7);
    let cy = (r[1] << 8).wrapping_add(r[3] << 7);
    // glyph colour
    let mut gc = ui.color(if sp.value != 0 { T_TEXT } else { T_TEXT_DISABLED });
    if ev & EV_HOVER != 0 {
        // hover plate
        let mut c;
        if flags & HB_CLOSE != 0 {
            c = ui.color(T_CLOSE_HOVER);
            gc = 0xFFFF_FFFF;
        } else {
            c = (ui.color(T_TEXT) & 0x00FF_FFFF) | 0x2E00_0000;
        }
        if ev & EV_HELD != 0 {
            c = color_mix(c, 0xFF00_0000, 48);
        }
        ui.draw_color(c);
        ui.draw_circle(cx, cy, rad, 0);
    }
    ui.draw_color(gc);
    let sz = ui.dp(10);
    let x = (cx >> 8).wrapping_sub(sz >> 1);
    let y = (cy >> 8).wrapping_sub(sz >> 1);
    let cp = if flags & HB_CLOSE != 0 {
        IC_CHROME_CLOSE
    } else if flags & HB_MIN != 0 {
        IC_CHEV_DOWN
    } else if sp.icon == IC_CHROME_RESTORE {
        IC_CHEV_UP_DOWN
    } else {
        IC_CHEV_UP
    };
    ui.draw_icon(cp, x, y, sz << 8);
}

// ---------------------------------------------------------------- secondary windows

/// sty_window_titlebar: title bar: the bar colour while active (the window colour when
/// inactive) with a hairline separator at the bottom.  value 1 = active
fn window_titlebar(ui: &mut Ui, sp: &Sp) {
    let c = ui.color(if sp.value != 0 { T_LAYER } else { T_BG });
    ui.draw_color(c);
    ui.draw_rect(sp.x, sp.y, sp.w, sp.h);
    let c = ui.color(T_DIVIDER);
    ui.draw_color(c);
    let h = ui.hair();
    ui.draw_rect(sp.x, sp.y.wrapping_add(sp.h).wrapping_sub(h), sp.w, h);
}

/// sty_window_border: the value px frame around a secondary window's surface
fn window_border(ui: &mut Ui, sp: &Sp) {
    let t = sp.value;
    let (w, h) = (sp.w, sp.h);
    ui.draw_color(DLG_STROKE);
    ui.draw_rect(0, 0, w, t); // top
    ui.draw_rect(0, h.wrapping_sub(t), w, t); // bottom
    ui.draw_rect(0, t, t, h.wrapping_sub(t).wrapping_sub(t)); // left
    ui.draw_rect(w.wrapping_sub(t), t, t, h.wrapping_sub(t).wrapping_sub(t)); // right
}
