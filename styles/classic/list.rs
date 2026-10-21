// list -- the classic focus rectangles, scroll bars, list rows, tabs, expanders
//                                         [styles/classic/list.s; focus_rect: basic.s]
//
//   focus_rect(x, y, w, h)              dotted rectangle 2 DIP inside (basic.rs)
//   scrollbar_thickness / margin / min_thumb -> 16 / 0 / 8 DIP, scrollbar(sp)
//   list_drop_line(sp), list_row(sp)    black insertion line; navy selected rows
//   tab_metrics(sp), tab(sp), tab_pill(sp)   raised tabs (the selected one 2 DIP higher)
//   expander_size(sp), expander_header(sp), expander_body(sp)   raised bar, etched body
//
// Contract: ASMLIB/inc/style.inc (lib/ui/style.rs).

use ui::consts::*;
use ui::keys::{IC_CHEV_DOWN_SMALL, IC_CHEV_UP_SMALL};
use ui::theme::color_mix;
use ui::{Sp, StyleList, Ui};

use crate::basic::focus_rect;
use crate::common::*;
use crate::Classic;

impl StyleList for Classic {
    fn focus_rect(&self, ui: &mut Ui, x: i32, y: i32, w: i32, h: i32) {
        focus_rect(ui, x, y, w, h)
    }
    fn scrollbar_thickness(&self, ui: &mut Ui) -> i32 {
        ui.dp(16)
    }
    fn scrollbar_margin(&self, _ui: &mut Ui) -> i32 {
        0 // the thumb runs over the whole bar
    }
    fn scrollbar_min_thumb(&self, ui: &mut Ui) -> i32 {
        ui.dp(8)
    }
    fn scrollbar(&self, ui: &mut Ui, sp: &Sp) {
        scrollbar(ui, sp)
    }
    fn list_drop_line(&self, ui: &mut Ui, sp: &Sp) {
        // 2 DIP black insertion line centred on y
        let t = ui.dp(2);
        let c = ui.color(T_TEXT);
        rect(ui, sp.x, sp.y.wrapping_sub(((t as u32) >> 1) as i32), sp.w, t, c);
    }
    fn list_row(&self, ui: &mut Ui, sp: &Sp) {
        list_row(ui, sp)
    }
    fn tab_metrics(&self, ui: &mut Ui, sp: &mut Sp) {
        // label padding 8 DIP, height 22 DIP, no pill
        sp.s[0] = ui.dp(8);
        sp.s[1] = ui.dp(22);
        sp.s[2] = 0;
    }
    fn tab(&self, ui: &mut Ui, sp: &Sp) {
        tab(ui, sp)
    }
    // the selected tab shows itself: nothing more to draw
    fn tab_pill(&self, _ui: &mut Ui, _sp: &Sp) {}
    fn expander_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        // 8 + label + 32, 24 DIP high
        ui.ts_font(TS_BODY);
        let tw = ui.text.width(sp.label.unwrap_or(""));
        let w = tw + ui.dp(40);
        (w, ui.dp(24))
    }
    fn expander_header(&self, ui: &mut Ui, sp: &Sp) {
        expander_header(ui, sp)
    }
    fn expander_body(&self, ui: &mut Ui, sp: &Sp) {
        // etched group frame around the content
        frame(ui, sp.rect(), FR_ETCHED);
    }
}

/// sty_scrollbar(sp): light track over the whole bar, raised grey thumb (flags 1 =
/// vertical, s[0] track start, s[1] thumb length, value2 thumb offset)
fn scrollbar(ui: &mut Ui, sp: &Sp) {
    let hi = pal(ui, CP_HI);
    let c = color_mix(hi, 0xFFD4_D0C8, 128);
    fill(ui, sp.rect(), c);
    let p = sp.s[0].wrapping_add(sp.value2);
    let th = if sp.flags != 0 { [sp.x, p, sp.w, sp.s[1]] } else { [p, sp.y, sp.s[1], sp.h] };
    let c = pal(ui, CP_FACE);
    fill(ui, th, c);
    frame(ui, th, FR_RAISED);
}

/// sty_list_row(sp): selected rows filled navy with white text, dotted focus rectangle,
/// label 4 DIP from the left (ev, value selected, value2 disabled, flags 1 dragged, 2 focus)
fn list_row(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let mut tok = T_TEXT;
    if sp.value != 0 {
        let c = ui.color(if sp.value2 != 0 { T_ACCENT_DISABLED } else { T_ACCENT });
        fill(ui, r, c);
        tok = T_ON_ACCENT;
    } else if sp.flags & 1 != 0 {
        // the dragged row
        let c = pal(ui, CP_SH);
        fill(ui, r, c);
    }
    if sp.value2 != 0 && sp.value == 0 {
        tok = T_TEXT_DISABLED;
    }
    let c = ui.color(tok);
    ui.ts_font(TS_BODY);
    let x = ui.dp(4).wrapping_add(sp.x);
    text_at(ui, x, r, sp.label, c, TS_BODY);
    if sp.flags & 2 != 0 {
        dots(ui, r);
    }
}

/// sty_tab(sp): raised tab (the selected one 2 DIP higher, without its bottom line), label
fn tab(ui: &mut Ui, sp: &Sp) {
    let mut t = sp.rect();
    if sp.value == 0 {
        let d = ui.dp(2);
        t[1] = t[1].wrapping_add(d);
        t[3] = t[3].wrapping_sub(d);
    }
    let c = pal(ui, CP_FACE);
    fill(ui, t, c);
    // top and left white, right dark (no bottom)
    let hi = pal(ui, CP_HI);
    let dk = pal(ui, CP_DK);
    let l = ui.hair();
    rect(ui, t[0], t[1], t[2], l, hi);
    rect(ui, t[0], t[1], l, t[3], hi);
    rect(ui, t[0].wrapping_add(t[2]).wrapping_sub(l), t[1], l, t[3], dk);
    let c = ui.color(if sp.value2 != 0 { T_TEXT_DISABLED } else { T_TEXT });
    text_mid(ui, t, sp.label, c, TS_BODY);
}

/// sty_expander_header(sp): raised bar (sunken while pressed), label, small triangle
fn expander_header(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let c = pal(ui, CP_FACE);
    fill(ui, r, c);
    let held = sp.ev & (EV_HOVER | EV_HELD) == EV_HOVER | EV_HELD;
    frame(ui, r, if held { FR_THIN_SUNKEN } else { FR_THIN_RAISED });
    let c = ui.color(if sp.value2 != 0 { T_TEXT_DISABLED } else { T_TEXT });
    ui.ts_font(TS_BODY);
    let x = ui.dp(8).wrapping_add(sp.x);
    text_at(ui, x, r, sp.label, c, TS_BODY);
    // glyph square at the right end
    let s = ui.dp(24);
    let g = [sp.x.wrapping_add(sp.w).wrapping_sub(s), sp.y, s, sp.h];
    let cp = if sp.value != 0 { IC_CHEV_UP_SMALL } else { IC_CHEV_DOWN_SMALL };
    icon_mid(ui, g, cp, 10, c);
}
