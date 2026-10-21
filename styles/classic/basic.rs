// basic -- the classic focus visuals, buttons, check box, radio, switch, slider, progress
// bar, separator                                                   [styles/classic/basic.s]
//
//   focus_ring(r, radius)               dotted rectangle 3 DIP inside the control
//   focus_rect(x, y, w, h)              dotted rectangle 2 DIP inside (StyleList, list.rs)
//   button_size / button                23 DIP (compact 20), 8 DIP padding, 16 DIP icon + 4
//                                       gap, labelled push buttons at least 72 DIP; ST_LARGE:
//                                       24 DIP icon over a caption, 52 high; raised grey face
//   checkbox_size / checkbox, radio_size / radio, switch_size / switch
//                                       13 DIP mark (switch: 28 x 14 track), 6 DIP gap, 20 high
//   slider_size / slider, trackbar      sunken groove and a raised thumb (shared with value.rs)
//   progress_size / progress            sunken frame with navy blocks
//   separator_size / separator          etched line
//
// Contract: ASMLIB/inc/style.inc (lib/ui/style.rs).

use ui::consts::*;
use ui::keys::IC_CHECK_MARK;
use ui::theme::color_mix;
use ui::{Sp, StyleBasic, Ui};

use crate::common::*;
use crate::Classic;

const BTN_H: i32 = 23; // push button height (DIP)
const BTN_H_COMPACT: i32 = 20;
const BTN_PAD: i32 = 8;
const BTN_MIN_W: i32 = 72;
const CHECK_BOX: i32 = 13; // check box / radio size (DIP)
const ROW_H: i32 = 20; // height of check box / radio / switch rows

impl StyleBasic for Classic {
    fn focus_ring(&self, ui: &mut Ui, r: [i32; 4], radius_dip: i32) {
        focus_ring(ui, r, radius_dip)
    }
    fn button_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        button_size(ui, sp)
    }
    fn button(&self, ui: &mut Ui, sp: &Sp) {
        button(ui, sp)
    }
    fn checkbox_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        row_size(ui, sp, CHECK_BOX, 6)
    }
    fn checkbox(&self, ui: &mut Ui, sp: &Sp) {
        checkbox(ui, sp)
    }
    fn radio_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        row_size(ui, sp, CHECK_BOX, 6)
    }
    fn radio(&self, ui: &mut Ui, sp: &Sp) {
        radio(ui, sp)
    }
    fn switch_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        // 28 x 14 DIP track, 6 DIP gap, 20 high; s[2] = track height
        let wh = row_size(ui, sp, 28, 6);
        sp.s[2] = ui.dp(14);
        wh
    }
    fn switch(&self, ui: &mut Ui, sp: &Sp) {
        switch(ui, sp)
    }
    fn slider_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        // 200 x 24 DIP (swapped for value = vertical); s[0] = thumb length along the axis
        // (11 DIP), s[1] = vertical
        sp.s[1] = sp.value;
        sp.s[0] = ui.dp(11);
        let a = ui.dp(200);
        let b = ui.dp(24);
        if sp.s[1] != 0 { (b, a) } else { (a, b) }
    }
    fn slider(&self, ui: &mut Ui, sp: &Sp) {
        // trackbar (value thumb offset, s[1] vertical)
        trackbar(ui, sp.rect(), sp.s[0], sp.value, sp.state, sp.s[1] == 0);
    }
    fn progress_size(&self, ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        // 200 x 16 DIP
        let w = ui.dp(200);
        (w, ui.dp(16))
    }
    fn progress(&self, ui: &mut Ui, sp: &Sp) {
        progress(ui, sp)
    }
    fn separator_size(&self, ui: &mut Ui, _sp: &mut Sp) -> i32 {
        // 3 + 2 lines + 3
        let t = ui.hair() * 2;
        ui.dp(6) + t
    }
    fn separator(&self, ui: &mut Ui, sp: &Sp) {
        separator(ui, sp.rect(), sp.value)
    }
}

// ------------------------------------------------------------------ focus

/// sty_focus_ring(r, radius_dip): dotted rectangle 3 DIP inside the control (radius unused)
pub(crate) fn focus_ring(ui: &mut Ui, r: [i32; 4], _radius_dip: i32) {
    let d = ui.dp(3);
    dots(ui, inset(r, d));
}

/// sty_focus_rect(x, y, w, h): dotted rectangle 2 DIP inside the rectangle
pub(crate) fn focus_rect(ui: &mut Ui, x: i32, y: i32, w: i32, h: i32) {
    let d = ui.dp(2);
    dots(ui, inset([x, y, w, h], d));
}

// ------------------------------------------------------------------ buttons

/// sty_button_size -> (w, h); keeps s[0] text width, s[1] icon px
fn button_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    let flags = sp.flags;
    ui.ts_font(if flags & ST_LARGE != 0 { TS_CAPTION } else { TS_BODY });
    let tw = match sp.label {
        Some(l) if flags & ST_ICON_ONLY == 0 => ui.text.width(l),
        _ => 0,
    };
    sp.s[0] = tw;
    if flags & ST_LARGE != 0 {
        // 24 DIP icon over a caption, 12 DIP padding, 52 high
        let ip = ui.dp(24);
        sp.s[1] = ip;
        let mut w = tw;
        if w < ip {
            w = ip;
        }
        w += ui.dp(12);
        return (w, ui.dp(52));
    }
    let ip = ui.dp(16);
    sp.s[1] = ip;
    let mut w = ui.dp(BTN_PAD) * 2 + tw;
    let hd = if flags & ST_COMPACT != 0 { BTN_H_COMPACT } else { BTN_H };
    if sp.icon != 0 {
        w += ip;
        if tw == 0 {
            // a square icon button
            w = ui.dp(hd);
            return (w, ui.dp(hd));
        }
        w += ui.dp(4);
    }
    if tw != 0 && flags & ST_SUBTLE == 0 {
        let m = ui.dp(BTN_MIN_W);
        if w < m {
            w = m;
        }
    }
    (w, ui.dp(hd))
}

/// sty_button: grey face with a raised frame (pressed: dark frame, checked: sunken and
/// lighter), default (accent) buttons with an extra black outline; ST_SUBTLE: flat, a thin
/// raised frame on hover, thin sunken when pressed / checked
fn button(ui: &mut Ui, sp: &Sp) {
    let mut face_r = sp.rect();
    let (state, flags) = (sp.state, sp.flags);
    // the pressed look: pressed or checked
    let pl = state == 2 || flags & ST_CHECKED != 0;
    if flags & ST_SUBTLE == 0 {
        let mut face = pal(ui, CP_FACE);
        if flags & ST_CHECKED != 0 {
            face = color_mix(face, 0xFFFF_FFFF, 128);
        }
        fill(ui, face_r, face);
        if flags & ST_ACCENT != 0 {
            // default button: black outline
            frame(ui, face_r, FR_BLACK);
            let l = ui.hair();
            face_r = inset(face_r, l);
        }
        let mut kind = if state == 2 { FR_PRESSED } else { FR_RAISED };
        if flags & ST_CHECKED != 0 {
            kind = FR_SUNKEN;
        }
        frame(ui, face_r, kind);
    } else if !pl {
        if state == 1 {
            frame(ui, face_r, FR_THIN_RAISED);
        }
    } else if state != 3 {
        if flags & ST_CHECKED != 0 {
            let face = pal(ui, CP_FACE);
            let c = color_mix(face, 0xFFFF_FFFF, 128);
            fill(ui, sp.rect(), c);
        }
        frame(ui, sp.rect(), FR_THIN_SUNKEN);
    }
    // colours and the pressed offset
    let tc = ui.color(if state == 3 { T_TEXT_DISABLED } else { T_TEXT });
    let off = if pl { ui.hair() } else { 0 };
    let (tw, ip) = (sp.s[0], sp.s[1]);
    if flags & ST_LARGE != 0 {
        if sp.icon != 0 {
            ui.draw_color(tc);
            let x = (sp.w.wrapping_sub(ip) >> 1).wrapping_add(sp.x).wrapping_add(off);
            let y = sp.y.wrapping_add(ui.dp(6)).wrapping_add(off);
            ui.draw_icon(sp.icon, x, y, ip << 8);
        }
        let Some(l) = sp.label else { return };
        ui.ts_font(TS_CAPTION);
        let w = ui.text.width(l);
        let x = (sp.w.wrapping_sub(w) >> 1).wrapping_add(sp.x).wrapping_add(off);
        let base = ui.ts_base(TS_CAPTION);
        let y = ui.dp(34).wrapping_add(base).wrapping_add(sp.y).wrapping_add(off);
        text(ui, x, y, l, tc, 0);
        return;
    }
    // icon + gap + label, centred
    let mut cw = tw;
    if sp.icon != 0 {
        cw += ip;
        if tw != 0 {
            cw += ui.dp(4);
        }
    }
    let mut x = (sp.w.wrapping_sub(cw) >> 1).wrapping_add(sp.x).wrapping_add(off);
    if sp.icon != 0 {
        ui.draw_color(tc);
        let y = (sp.h.wrapping_sub(ip) >> 1).wrapping_add(sp.y).wrapping_add(off);
        ui.draw_icon(sp.icon, x, y, ip << 8);
        x += ip;
        x += ui.dp(4);
    }
    if tw == 0 {
        return;
    }
    ui.ts_font(TS_BODY);
    let mut lr = sp.rect();
    lr[1] = lr[1].wrapping_add(off);
    if state == 3 {
        // disabled: white emboss first
        let l = ui.hair();
        let ex = x.wrapping_add(l);
        lr[1] = lr[1].wrapping_add(l);
        let hi = pal(ui, CP_HI);
        text_at(ui, ex, lr, sp.label, hi, TS_BODY);
        let l = ui.hair();
        lr[1] = lr[1].wrapping_sub(l);
    }
    text_at(ui, x, lr, sp.label, tc, TS_BODY);
}

// ------------------------------------------------------------------ check box, radio, switch

// row_size(sp, mark_dip, gap_dip) -> (w, h): mark + gap + label, ROW_H high; keeps s[0]
// mark px, s[1] label width
fn row_size(ui: &mut Ui, sp: &mut Sp, mark: i32, gap: i32) -> (i32, i32) {
    ui.ts_font(TS_BODY);
    let tw = match sp.label {
        Some(l) => ui.text.width(l),
        None => 0,
    };
    sp.s[1] = tw;
    let m = ui.dp(mark);
    sp.s[0] = m;
    let mut w = m;
    if tw != 0 {
        w += ui.dp(gap);
        w += tw;
    }
    (w, ui.dp(ROW_H))
}

// row_label(sp, x): the label of a check box / radio / switch row from x (disabled: grey)
fn row_label(ui: &mut Ui, sp: &Sp, x: i32) {
    if sp.label.is_none() {
        return;
    }
    ui.ts_font(TS_BODY);
    let c = ui.color(if sp.state == 3 { T_TEXT_DISABLED } else { T_TEXT });
    text_at(ui, x, sp.rect(), sp.label, c, TS_BODY);
}

/// sty_checkbox: white sunken box (grey when disabled or pressed), check mark / square
fn checkbox(ui: &mut Ui, sp: &Sp) {
    let b = sp.s[0];
    let br = [sp.x, (sp.h.wrapping_sub(b) >> 1).wrapping_add(sp.y), b, b];
    let f = pal(ui, if sp.state < 2 { CP_FIELD } else { CP_FACE });
    fill(ui, br, f);
    frame(ui, br, FR_SUNKEN);
    if sp.value != 0 {
        let c = ui.color(if sp.state == 3 { T_TEXT_DISABLED } else { T_TEXT });
        if sp.value == 2 {
            // indeterminate: a small square
            ui.draw_color(c);
            let d = ui.dp(4);
            let s = b.wrapping_sub(d).wrapping_sub(d);
            ui.draw_rect(br[0].wrapping_add(d), br[1].wrapping_add(d), s, s);
        } else {
            icon_mid(ui, br, IC_CHECK_MARK, 10, c);
        }
    }
    let x = ui.dp(6).wrapping_add(b).wrapping_add(sp.x);
    row_label(ui, sp, x);
}

/// sty_radio: sunken round well (grey / white rings), black dot when selected
fn radio(ui: &mut Ui, sp: &Sp) {
    let b = sp.s[0];
    let mut rad = b << 7; // radius (fx)
    let cx = (sp.x << 8).wrapping_add(rad);
    let cy = (sp.h << 7).wrapping_add(sp.y << 8);
    // outer ring: grey
    let c = pal(ui, CP_SH);
    ui.draw_color(c);
    ui.draw_circle(cx, cy, rad, 0);
    // white ring
    let c = pal(ui, CP_HI);
    ui.draw_color(c);
    rad = rad.wrapping_sub(ui.dp_fx(256));
    ui.draw_circle(cx, cy, rad, 0);
    let c = pal(ui, if sp.state < 2 { CP_FIELD } else { CP_FACE });
    ui.draw_color(c);
    rad = rad.wrapping_sub(ui.dp_fx(256));
    ui.draw_circle(cx, cy, rad, 0);
    if sp.value != 0 {
        let c = ui.color(if sp.state == 3 { T_TEXT_DISABLED } else { T_TEXT });
        ui.draw_color(c);
        let r = ui.dp_fx(2 * 256);
        ui.draw_circle(cx, cy, r, 0);
    }
    let x = ui.dp(6).wrapping_add(b).wrapping_add(sp.x);
    row_label(ui, sp, x);
}

/// sty_switch: sunken track (navy when on) with a raised square knob sliding with the
/// animation value2 (0 .. 256)
fn switch(ui: &mut Ui, sp: &Sp) {
    let (tw, th) = (sp.s[0], sp.s[2]); // track width / height
    let tr = [sp.x, (sp.h.wrapping_sub(th) >> 1).wrapping_add(sp.y), tw, th];
    let mut c = pal(ui, CP_FIELD);
    if sp.value != 0 {
        c = ui.color(if sp.state == 3 { T_ACCENT_DISABLED } else { T_ACCENT });
    }
    fill(ui, tr, c);
    frame(ui, tr, FR_SUNKEN);
    // knob: a square of the track height, sliding over the travel
    let kx = (tw.wrapping_sub(th).wrapping_mul(sp.value2) >> 8).wrapping_add(tr[0]);
    let knob = [kx, tr[1], th, th];
    let c = pal(ui, CP_FACE);
    fill(ui, knob, c);
    frame(ui, knob, FR_RAISED);
    let x = ui.dp(6).wrapping_add(tw).wrapping_add(sp.x);
    row_label(ui, sp, x);
}

// ------------------------------------------------------------------ slider

/// cls_trackbar(sp, horizontal): sunken 4 DIP groove along the travel and a raised thumb
/// (offset `off`, length `thumb`; pressed frame in state 2); shared with the value sliders
pub(crate) fn trackbar(ui: &mut Ui, r: [i32; 4], thumb: i32, off: i32, state: u32, horiz: bool) {
    let g = ui.dp(4); // groove thickness
    let half = ((thumb as u32) >> 1) as i32;
    let (groove, th);
    if horiz {
        groove = [half.wrapping_add(r[0]), (r[3].wrapping_sub(g) >> 1).wrapping_add(r[1]), r[2].wrapping_sub(thumb), g];
        // thumb: thumb length x height - 4 DIP
        let d = ui.dp(2);
        th = [r[0].wrapping_add(off), r[1].wrapping_add(d), thumb, r[3].wrapping_sub(d).wrapping_sub(d)];
    } else {
        groove = [(r[2].wrapping_sub(g) >> 1).wrapping_add(r[0]), half.wrapping_add(r[1]), g, r[3].wrapping_sub(thumb)];
        let d = ui.dp(2);
        // max at the top
        let y = r[1].wrapping_add(r[3]).wrapping_sub(thumb).wrapping_sub(off);
        th = [r[0].wrapping_add(d), y, r[2].wrapping_sub(d).wrapping_sub(d), thumb];
    }
    let c = pal(ui, CP_FIELD);
    fill(ui, groove, c);
    frame(ui, groove, FR_THIN_SUNKEN);
    let c = pal(ui, CP_FACE);
    fill(ui, th, c);
    frame(ui, th, if state == 2 { FR_PRESSED } else { FR_RAISED });
}

// ------------------------------------------------------------------ progress, separator

/// sty_progress: sunken frame with navy blocks up to value / value2
fn progress(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let c = pal(ui, CP_FACE);
    fill(ui, r, c);
    frame(ui, r, FR_THIN_SUNKEN);
    let d = ui.dp(2);
    let inner = inset(r, d);
    let max = sp.value2;
    if max <= 0 {
        return;
    }
    let mut v = sp.value;
    if v <= 0 {
        return;
    }
    if v > max {
        v = max;
    }
    let fw = v.wrapping_mul(inner[2]) / max; // filled width
    let bw = ui.dp(8); // block width
    let gap = ui.dp(2);
    let c = ui.color(T_ACCENT);
    let mut i = 0;
    while i < fw {
        let mut w = fw - i;
        if w > bw {
            w = bw;
        }
        rect(ui, inner[0].wrapping_add(i), inner[1], w, inner[3], c);
        i = i.wrapping_add(bw).wrapping_add(gap);
    }
}

/// sty_separator: etched line (grey over white), horizontal = 1: a horizontal line
pub(crate) fn separator(ui: &mut Ui, r: [i32; 4], horizontal: i32) {
    let t = ui.hair();
    let sh = pal(ui, CP_SH);
    let hi = pal(ui, CP_HI);
    if horizontal != 0 {
        let y = (r[3].wrapping_sub(t) >> 1).wrapping_add(r[1]).wrapping_sub(t);
        rect(ui, r[0], y, r[2], t, sh);
        rect(ui, r[0], y.wrapping_add(t), r[2], t, hi);
    } else {
        let x = (r[2].wrapping_sub(t) >> 1).wrapping_add(r[0]).wrapping_sub(t);
        rect(ui, x, r[1], t, r[3], sh);
        rect(ui, x.wrapping_add(t), r[1], t, r[3], hi);
    }
}
