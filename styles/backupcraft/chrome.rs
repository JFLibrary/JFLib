// chrome -- BackupCraft metrics, content dialogs, info bars, status bar, caption buttons,
// title bars and frames of windows
//
//   metric(id)          SM_* values: dialogs 340 .. 560 DIP with 24 DIP padding and the footer
//                       set apart (WinUI), primary button first; 40 DIP title bar with a left
//                       title; 28 DIP status bar
//   dialog_frame        10 DIP corners, deep soft shadow, card surface, the footer on the
//                       alternate background with a divider
//   infobar             severity tint, 6 DIP corners, a coloured bar at the left, icon disc,
//                       title, message, close button
//   statusbar           panel fill with a divider at the top
//   caption_button      WinUI: full-height 46 DIP buttons, a shade on hover, close red
//   window_titlebar     the panel colour with a divider at the bottom (the navigation pane
//                       continues it)

use ui::consts::*;
use ui::keys::{IC_CANCEL, IC_CHECK_MARK, IC_CHROME_CLOSE, IC_CHROME_MAX, IC_CHROME_MIN, IC_CHROME_RESTORE};
use ui::theme::color_mix;
use ui::{Sp, StyleChrome, Ui};

use crate::common::{baseline, fill_rrect, shadow, stroke_rrect, text, R_DIALOG};
use crate::BackupCraft;

const IB_BG: [u32; 4] = [T_INFO_BG, T_OK_BG, T_WARN_BG, T_CRIT_BG];
const IB_FG: [u32; 4] = [T_INFO, T_OK, T_WARN, T_CRIT];

const SM_TAB: [i32; SM_COUNT as usize] = [
    340,                         // SM_DLG_MIN
    560,                         // SM_DLG_MAX
    24,                          // SM_DLG_MARGIN
    (BOX_V | PAD_24) as i32,     // SM_DLG_CONTENT_BOX
    12,                          // SM_DLG_CONTENT_GAP
    TS_SUBTITLE as i32,          // SM_DLG_TITLE_TS
    (BOX_H | PAD_20) as i32,     // SM_DLG_FOOTER_BOX
    8,                           // SM_DLG_FOOTER_GAP
    28,                          // SM_SB_H
    12,                          // SM_SB_PAD
    16,                          // SM_SB_GAP
    40,                          // SM_TB_H
    48,                          // SM_TB_H_LARGE
    12,                          // SM_TB_PAD
    18,                          // SM_TB_ICON
    TS_BODY_STRONG as i32,       // SM_TB_TITLE_TS
    46,                          // SM_CAP_W
    36,                          // SM_CAP_W_TOOL
    12,                          // SM_WC_TITLE_PAD
    8,                           // SM_WC_TITLE_GAP
    5,                           // SM_GRIP
    12,                          // SM_GRIP_CORNER
    32,                          // SM_WIN_TBH_TOOL
    36,                          // SM_WIN_TBH_DIALOG
    0,                           // SM_CAP_LEFT
    0,                           // SM_DLG_BTN_ORDER
    32,                          // SM_DOCK_HDR
    28,                          // SM_DOCK_BTN
];

impl StyleChrome for BackupCraft {
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
        (ui.dp(16 + 18 + 12 + 12 + 8 + 36 + 6) + tw + mw, ui.dp(44))
    }
    fn infobar(&self, ui: &mut Ui, sp: &mut Sp) {
        infobar(ui, sp)
    }
    fn infobar_close(&self, ui: &mut Ui, sp: &Sp) {
        let r = sp.rect();
        let ev = sp.ev;
        if ev & EV_HOVER != 0 {
            let c = ui.color(if ev & EV_HELD != 0 { T_SUBTLE_PRESSED } else { T_SUBTLE_HOVER });
            let rad = ui.dp(4);
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
        let c = ui.color(T_DIVIDER);
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
        let c = ui.color(T_DIVIDER);
        ui.draw_color(c);
        let h = ui.hair();
        ui.draw_rect(sp.x, sp.y + sp.h - h, sp.w, h);
    }
    fn window_border(&self, ui: &mut Ui, sp: &Sp) {
        let t = sp.value;
        let (w, h) = (sp.w, sp.h);
        let c = ui.color(T_CTRL_STROKE2);
        ui.draw_color(c);
        ui.draw_rect(0, 0, w, t);
        ui.draw_rect(0, h - t, w, t);
        ui.draw_rect(0, t, t, h - t * 2);
        ui.draw_rect(w - t, t, t, h - t * 2);
    }
    fn window_dialog_fill(&self, ui: &mut Ui, sp: &Sp) {
        let c = ui.color(T_CARD) | 0xFF00_0000;
        ui.draw_color(c);
        ui.draw_rect(sp.x, sp.y, sp.w, sp.h);
    }
}

/// dialog_frame: card surface with the footer (value = its height) on the alternate
/// background, 10 DIP corners, deep shadow, hairline
fn dialog_frame(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let rad = ui.dp(R_DIALOG);
    let depth = ui.dp(24);
    shadow(ui, r, rad, depth, 0x38, 0x90);
    let card = ui.color(T_CARD) | 0xFF00_0000;
    fill_rrect(ui, r[0], r[1], r[2], r[3], rad, card);
    let fh = sp.value;
    if fh > 0 {
        let alt = ui.color(T_BG_ALT) | 0xFF00_0000;
        let alt = color_mix(card, alt, 160);
        ui.draw_clip(r[0], r[1] + r[3] - fh, r[2], fh);
        fill_rrect(ui, r[0], r[1], r[2], r[3], rad, alt);
        ui.draw_unclip();
        let c = ui.color(T_DIVIDER);
        ui.draw_color(c);
        let h = ui.hair();
        ui.draw_rect(r[0], r[1] + r[3] - fh, r[2], h);
    }
    let stroke = ui.color(T_FLYOUT_STROKE);
    let t = ui.hair();
    stroke_rrect(ui, r[0], r[1], r[2], r[3], rad, t, stroke);
}

/// infobar: severity tint, a 3 DIP bar in the severity colour at the left, an 18 DIP disc
/// with a glyph, title (strong) and message; s[2] / s[3] = close button x / size
fn infobar(ui: &mut Ui, sp: &mut Sp) {
    let r = sp.rect();
    let sev = (sp.flags & 3) as usize;
    let tw = sp.s[0];
    let bg = ui.color(IB_BG[sev]);
    let fg = ui.color(IB_FG[sev]);
    let rad = ui.dp(6);
    fill_rrect(ui, r[0], r[1], r[2], r[3], rad, bg);
    let t = ui.hair();
    let edge = (fg & 0x00FF_FFFF) | 0x5000_0000;
    stroke_rrect(ui, r[0], r[1], r[2], r[3], rad, t, edge);
    // the bar
    ui.draw_clip(r[0], r[1], ui.dp(3), r[3]);
    fill_rrect(ui, r[0], r[1], r[2], r[3], rad, fg);
    ui.draw_unclip();
    // icon disc
    ui.draw_color(fg);
    let left = r[0] + ui.dp(16);
    let crad = ui.dp_fx(9 * 256);
    let cx = (left << 8) + crad;
    let cy = (r[1] << 8) + (r[3] << 7);
    ui.draw_circle(cx, cy, crad, 0);
    let on = 0xFFFF_FFFF;
    ui.draw_color(on);
    let sv = sev as u32;
    if sv == IB_SUCCESS || sv == IB_ERROR {
        let sz = ui.dp(10);
        let ic = if sv == IB_SUCCESS { IC_CHECK_MARK } else { IC_CANCEL };
        ui.draw_icon(ic, (cx >> 8) - (sz >> 1), (cy >> 8) - (sz >> 1), sz << 8);
    } else {
        let size = ui.dp_fx(12 * 256);
        ui.text.set_font(FONT_BOLD, size);
        let s = if sv == IB_WARNING { "!" } else { "i" };
        let w = ui.text.width(s);
        let asc = ui.text.ascent();
        text(ui, (cx >> 8) - (w >> 1), (cy >> 8) + (asc * 70 >> 8), s, on, 0);
    }
    let cs = ui.dp(28);
    let cxb = r[0] + r[2] - ui.dp(8) - cs;
    let mut tx = r[0] + ui.dp(16 + 18 + 12);
    if let Some(title) = sp.label {
        let y = baseline(ui, TS_BODY_STRONG, r[1], r[3]);
        let c = ui.color(T_TEXT);
        text(ui, tx, y, title, c, 0);
        tx += ui.dp(12) + tw;
    }
    if let Some(msg) = sp.label2 {
        let y = baseline(ui, TS_BODY, r[1], r[3]);
        let c = ui.color(T_TEXT2);
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
