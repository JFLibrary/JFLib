// chrome -- the winui metrics, dialogs, info bars, status / title bars, caption buttons,
// window frames                                                   [styles/winui/chrome.s]
//
//   metric(id)                          SM_* layout values (style.inc)
//   dialog_smoke, dialog_frame          the dim layer behind a content dialog; its shadow,
//                                       surface (content on the alt layer colour above the
//                                       footer), 8 DIP corners, stroke
//   infobar_size / infobar / infobar_close   severity card, icon disc, title, message,
//                                       38 DIP close button
//   statusbar                           layer fill with a hairline at the top
//   caption_button                      full-height caption button (close: red hover)
//   window_titlebar, window_border, window_dialog_fill   frames of secondary windows
//
// Contract: ASMLIB/inc/style.inc (lib/ui/style.rs).

use ui::consts::*;
use ui::keys::{IC_CANCEL, IC_CHECK_MARK};
use ui::{Sp, StyleChrome, Ui};

use crate::WinUi;

const DLG_STROKE: u32 = 0x6675_7575; // SurfaceStrokeColorDefault (both themes)
const DLG_SHADOW_A: u32 = 0x30; // shadow alpha at the dialog edge (light / dark theme)
const DLG_SHADOW_A_DARK: u32 = 0x60;

// info bar colours per severity (IB_INFO, IB_SUCCESS, IB_WARNING, IB_ERROR)
const IB_BG: [u32; 4] = [T_INFO_BG, T_OK_BG, T_WARN_BG, T_CRIT_BG];
const IB_FG: [u32; 4] = [T_INFO, T_OK, T_WARN, T_CRIT];

// the SM_* table
const SM_TAB: [i32; SM_COUNT as usize] = [
    320,                         // SM_DLG_MIN
    548,                         // SM_DLG_MAX
    24,                          // SM_DLG_MARGIN
    (BOX_V | PAD_24) as i32,     // SM_DLG_CONTENT_BOX
    12,                          // SM_DLG_CONTENT_GAP
    TS_SUBTITLE as i32,          // SM_DLG_TITLE_TS
    (BOX_H | PAD_24) as i32,     // SM_DLG_FOOTER_BOX
    8,                           // SM_DLG_FOOTER_GAP
    32,                          // SM_SB_H
    12,                          // SM_SB_PAD
    16,                          // SM_SB_GAP
    32,                          // SM_TB_H
    48,                          // SM_TB_H_LARGE
    16,                          // SM_TB_PAD
    16,                          // SM_TB_ICON
    TS_CAPTION as i32,           // SM_TB_TITLE_TS
    46,                          // SM_CAP_W
    40,                          // SM_CAP_W_TOOL
    12,                          // SM_WC_TITLE_PAD
    8,                           // SM_WC_TITLE_GAP
    5,                           // SM_GRIP
    12,                          // SM_GRIP_CORNER
    28,                          // SM_WIN_TBH_TOOL
    32,                          // SM_WIN_TBH_DIALOG
    0,                           // SM_CAP_LEFT
    0,                           // SM_DLG_BTN_ORDER
    32,                          // SM_DOCK_HDR
    28,                          // SM_DOCK_BTN
];

impl StyleChrome for WinUi {
    fn metric(&self, id: u32) -> i32 {
        if id < SM_COUNT { SM_TAB[id as usize] } else { 0 }
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
        infobar_size(ui, sp)
    }
    fn infobar(&self, ui: &mut Ui, sp: &mut Sp) {
        infobar(ui, sp)
    }
    fn infobar_close(&self, ui: &mut Ui, sp: &Sp) {
        infobar_close(ui, sp)
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
    // WinUI title bars show the window background (nothing to draw)
    fn window_titlebar(&self, _ui: &mut Ui, _sp: &Sp) {}
    fn window_border(&self, ui: &mut Ui, sp: &Sp) {
        window_border(ui, sp)
    }
    fn window_dialog_fill(&self, ui: &mut Ui, sp: &Sp) {
        // content area of a dialog window on the alt layer colour (LayerFillColorAlt)
        let c = if ui.theme_dark() { 0x0DFF_FFFF } else { 0xFFFF_FFFF };
        ui.draw_color(c);
        ui.draw_rect(sp.x, sp.y, sp.w, sp.h);
    }
}

// ---------------------------------------------------------------- helpers (ui_list.s w_*)

/// w_fill_rrect(x, y, w, h, r, argb): filled rounded rectangle (skipped when transparent)
fn fill_rrect(ui: &mut Ui, r: [i32; 4], rad: i32, argb: u32) {
    if argb & 0xFF00_0000 != 0 {
        ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, 0, argb, 0, 0);
    }
}

/// w_stroke_rrect(x, y, w, h, r, t, argb): rounded border of thickness t
#[allow(clippy::too_many_arguments)]
fn stroke_rrect(ui: &mut Ui, r: [i32; 4], rad: i32, t: i32, argb: u32) {
    ui.cmd_rrect(r[0], r[1], r[2], r[3], rad, t, 0, argb, argb);
}

/// w_text(x, baseline, s, argb, maxw): text in the current font (maxw 0 = unclipped)
fn text(ui: &mut Ui, x: i32, y: i32, s: &str, argb: u32, maxw: i32) {
    let (font, size) = ui.text.get_font();
    let b = s.as_bytes();
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    ui.cmd_text(x, y, &b[..n], font, size, argb, maxw);
}

/// w_baseline(ts, y, h): baseline of style ts centred in y .. y + h (selects the font)
fn baseline(ui: &mut Ui, ts: u32, y: i32, h: i32) -> i32 {
    let l = ui.ts_line(ts);
    let y = y + ((h - l) >> 1);
    ui.ts_base(ts) + y
}

// ---------------------------------------------------------------- content dialogs

/// sty_dialog_frame: soft shadow, background surface (the content area on the alt layer
/// colour above the footer, value = footer height px), 8 DIP corners, stroke
fn dialog_frame(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let fh = sp.value;
    let rad = ui.dp(8);
    // soft shadow: cmd_shadow stacks `depth` layers, so the layer alpha is chosen to give
    // about DLG_SHADOW_A at the edge: a = A * 4d / ((d + 1)(d + 2))
    let depth = ui.dp(16);
    let a0 = if ui.theme_dark() { DLG_SHADOW_A_DARK } else { DLG_SHADOW_A };
    let num = ((depth << 2) as u32).wrapping_mul(a0);
    let den = ((depth + 1).wrapping_mul(depth + 2)) as u32;
    let a = ((num / den) as i32).clamp(1, 255);
    ui.draw_color((a as u32) << 24);
    ui.cmd_shadow(r[0], r[1], r[2], r[3], rad, depth);
    let bg = ui.color(T_BG) | 0xFF00_0000;
    fill_rrect(ui, r, rad, bg);
    let top = r[3] - fh;
    if top > 0 {
        ui.draw_clip(r[0], r[1], r[2], top);
        let c = if ui.theme_dark() { 0x0DFF_FFFF } else { 0xFFFF_FFFF }; // LayerFillColorAlt
        // (height = top area + radius: the command itself changes with the footer height,
        // the dirty-region hash does not see clip commands)
        let h = if fh == 0 { r[3] } else { r[3] - fh + rad };
        fill_rrect(ui, [r[0], r[1], r[2], h], rad, c);
        ui.draw_unclip();
    }
    let t = ui.hair();
    stroke_rrect(ui, r, rad, t, DLG_STROKE);
}

// ---------------------------------------------------------------- info bar

/// sty_infobar_size: 16 + 16 icon + 12 + title (body strong) + 12 + message + 8 + 38 close
/// + 5, 48 high; keeps s[0] / s[1] = title / message width
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
    let w = ui.dp(16 + 16 + 12 + 12 + 8 + 38 + 5) + tw + mw;
    let h = ui.dp(48);
    sp.s[0] = tw;
    sp.s[1] = mw;
    (w, h)
}

/// sty_infobar: card in the severity colour (4 DIP corners), 16 DIP icon disc with a glyph,
/// title and message; sets s[2] / s[3] = x / size of the close button
fn infobar(ui: &mut Ui, sp: &mut Sp) {
    let r = sp.rect();
    let sev = (sp.flags & 3) as usize;
    let tw = sp.s[0];
    // background card in the severity colour
    let bg = ui.color(IB_BG[sev]);
    let stroke = ui.color(T_CARD_STROKE);
    let rad = ui.dp(4);
    fill_rrect(ui, r, rad, bg);
    let t = ui.hair();
    let rad = ui.dp(4);
    stroke_rrect(ui, r, rad, t, stroke);
    // icon: 16 DIP disc in the severity colour with a knocked-in glyph
    let fg = ui.color(IB_FG[sev]);
    ui.draw_color(fg);
    let left = r[0] + ui.dp(16);
    let crad = ui.dp_fx(8 * 256);
    let cx = (left << 8) + crad;
    let cy = (r[3] << 7) + (r[1] << 8);
    ui.draw_circle(cx, cy, crad, 0);
    let on = ui.color(T_ON_ACCENT);
    ui.draw_color(on);
    if sev as u32 == IB_SUCCESS || sev as u32 == IB_ERROR {
        let sz = ui.dp(10);
        let x = (cx >> 8) - (sz >> 1);
        let y = (cy >> 8) - (sz >> 1);
        let ic = if sev as u32 == IB_SUCCESS { IC_CHECK_MARK } else { IC_CANCEL };
        ui.draw_icon(ic, x, y, sz << 8);
    } else {
        // "i" / "!" in bold
        let size = ui.dp_fx(12 * 256);
        ui.text.set_font(FONT_BOLD, size);
        let s = if sev as u32 == IB_WARNING { "!" } else { "i" };
        let w = ui.text.width(s);
        let x = (cx >> 8) - (w >> 1);
        let asc = ui.text.ascent();
        let y = (cy >> 8) + ((asc * 70) >> 8); // cap height ~ 0.7 ascent
        text(ui, x, y, s, on, 0);
    }
    // close button (38 DIP, 5 DIP from the right edge)
    let cs = ui.dp(38);
    let cxb = r[0] + r[2] - ui.dp(5) - cs;
    // title and message
    let mut tx = ui.dp(16 + 16 + 12) + r[0];
    if let Some(title) = sp.label {
        let y = baseline(ui, TS_BODY_STRONG, r[1], r[3]);
        let c = ui.color(T_TEXT);
        text(ui, tx, y, title, c, 0);
        tx += ui.dp(12) + tw;
    }
    if let Some(msg) = sp.label2 {
        let y = baseline(ui, TS_BODY, r[1], r[3]);
        let c = ui.color(T_TEXT);
        let mut mw = cxb - ui.dp(8) - tx;
        if mw <= 0 {
            mw = 1;
        }
        text(ui, tx, y, msg, c, mw);
    }
    sp.s[2] = cxb;
    sp.s[3] = cs;
}

/// sty_infobar_close: subtle hover / pressed square, cancel glyph
fn infobar_close(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let ev = sp.ev;
    if ev & EV_HOVER != 0 {
        let c = ui.color(if ev & EV_HELD != 0 { T_SUBTLE_PRESSED } else { T_SUBTLE_HOVER });
        let rad = ui.dp(4);
        fill_rrect(ui, r, rad, c);
    }
    let c = ui.color(if ev & EV_HELD != 0 { T_TEXT2 } else { T_TEXT });
    ui.draw_color(c);
    let sz = ui.dp(16);
    let d = (r[2] - sz) >> 1;
    ui.draw_icon(IC_CANCEL, d + r[0], d + r[1], sz << 8);
}

// ---------------------------------------------------------------- caption buttons

/// sty_caption_button: subtle hover / pressed fill (close: red with a white glyph), 10 DIP
/// glyph; value 1 = the window is active
fn caption_button(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (ev, flags) = (sp.ev, sp.flags);
    if ev & EV_HOVER != 0 {
        let fill = if flags & HB_CLOSE != 0 {
            let c = ui.color(T_CLOSE_HOVER);
            if ev & EV_HELD != 0 { (c & 0xFF_FFFF) | 0xE600_0000 } else { c }
        } else {
            ui.color(if ev & EV_HELD != 0 { T_SUBTLE_PRESSED } else { T_SUBTLE_HOVER })
        };
        ui.draw_color(fill);
        ui.draw_rect(r[0], r[1], r[2], r[3]);
    }
    let mut tok = if sp.value != 0 { T_TEXT } else { T_TEXT_DISABLED };
    if ev & EV_HELD != 0 {
        tok = T_TEXT2;
    }
    let mut c = ui.color(tok);
    if flags & HB_CLOSE != 0 && ev & EV_HOVER != 0 {
        c = 0xFFFF_FFFF;
    }
    ui.draw_color(c);
    let sz = ui.dp(10);
    let x = ((r[2] - sz) >> 1) + r[0];
    let y = ((r[3] - sz) >> 1) + r[1];
    ui.draw_icon(sp.icon, x, y, sz << 8);
}

// ---------------------------------------------------------------- secondary windows

/// sty_window_border: the value px frame around a secondary window's surface
fn window_border(ui: &mut Ui, sp: &Sp) {
    let t = sp.value;
    let (w, h) = (sp.w, sp.h);
    ui.draw_color(DLG_STROKE);
    ui.draw_rect(0, 0, w, t); // top
    ui.draw_rect(0, h - t, w, t); // bottom
    ui.draw_rect(0, t, t, h - t - t); // left
    ui.draw_rect(w - t, t, t, h - t - t); // right
}
