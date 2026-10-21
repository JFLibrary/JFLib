// common -- the classic style's drawing helpers shared by its widget files
//                                                                [styles/classic/common.s]
//
//   pal(i) -> argb                      3D palette: CP_HI white, CP_FACE, CP_SH grey, CP_DK,
//                                       CP_FIELD
//   fill(r, argb), rect(x, y, w, h, argb)   filled rectangle (nothing when w or h <= 0)
//   bevel(r, tl, br)                    1-line border: top / left in tl, bottom / right in br
//   frame(r, kind)                      3D frames FR_* (2 lines deep, or 1 for the thin kinds)
//   inset(r, d)                         r shrunk by d px on every side
//   text_mid(r, s, argb, ts)            one line centred in the rectangle
//   text_at(x, r, s, argb, ts)          one line from x, vertically centred
//   icon_mid(r, cp, dip, argb)          icon glyph centred in the rectangle
//   dots(r)                             dotted focus rectangle (the classic focus visual)
//   baseline(ts, y, h), text(x, y, s, argb, maxw)   w_baseline / w_text of the core (ui_list.s)
//
// All coordinates are px; lines are ui.hair() thick.  (CP_* / FR_*: classic.inc)

use ui::consts::*;
use ui::Ui;

pub(crate) const CP_HI: u32 = 0; // highlight (white)
pub(crate) const CP_FACE: u32 = 1; // 3D face
pub(crate) const CP_SH: u32 = 2; // shadow
pub(crate) const CP_DK: u32 = 3; // dark shadow
pub(crate) const CP_FIELD: u32 = 4; // field (text boxes, lists)

pub(crate) const FR_RAISED: u32 = 0; // frame kinds
pub(crate) const FR_SUNKEN: u32 = 1;
pub(crate) const FR_PRESSED: u32 = 2;
pub(crate) const FR_THIN_RAISED: u32 = 3;
pub(crate) const FR_THIN_SUNKEN: u32 = 4;
pub(crate) const FR_ETCHED: u32 = 5;
pub(crate) const FR_BLACK: u32 = 6;

//                       hi          face        shadow      dark        field
static PAL_LIGHT: [u32; 8] = [0xFFFFFFFF, 0xFFD4D0C8, 0xFF808080, 0xFF404040, 0xFFFFFFFF, 0, 0, 0];
static PAL_DARK: [u32; 8] = [0xFF6A6A6A, 0xFF404040, 0xFF202020, 0xFF101010, 0xFF2A2A2A, 0, 0, 0];

/// cls_pal(i): colour of the light or dark 3D palette
pub(crate) fn pal(ui: &Ui, i: u32) -> u32 {
    let t = if ui.theme_dark() { &PAL_DARK } else { &PAL_LIGHT };
    t[(i & 7) as usize]
}

/// cls_rect(x, y, w, h, argb): filled rectangle (nothing when w or h <= 0)
pub(crate) fn rect(ui: &mut Ui, x: i32, y: i32, w: i32, h: i32, argb: u32) {
    if w <= 0 || h <= 0 {
        return;
    }
    ui.draw_color(argb);
    ui.draw_rect(x, y, w, h);
}

/// cls_fill(r, argb)
pub(crate) fn fill(ui: &mut Ui, r: [i32; 4], argb: u32) {
    rect(ui, r[0], r[1], r[2], r[3], argb);
}

/// cls_bevel(r, tl, br): top and left lines in tl, bottom and right lines in br
pub(crate) fn bevel(ui: &mut Ui, r: [i32; 4], tl: u32, br: u32) {
    let l = ui.hair();
    rect(ui, r[0], r[1], r[2], l, tl); // top
    rect(ui, r[0], r[1], l, r[3], tl); // left
    rect(ui, r[0], r[1].wrapping_add(r[3]).wrapping_sub(l), r[2], l, br); // bottom
    rect(ui, r[0].wrapping_add(r[2]).wrapping_sub(l), r[1], l, r[3], br); // right
}

/// cls_inset(r, d): r shrunk by d on every side
pub(crate) fn inset(r: [i32; 4], d: i32) -> [i32; 4] {
    [
        r[0].wrapping_add(d),
        r[1].wrapping_add(d),
        r[2].wrapping_sub(d).wrapping_sub(d),
        r[3].wrapping_sub(d).wrapping_sub(d),
    ]
}

/// cls_frame(r, kind): FR_RAISED / FR_SUNKEN / FR_PRESSED: two lines; FR_THIN_*, FR_ETCHED
/// (two thin frames), FR_BLACK: one line
pub(crate) fn frame(ui: &mut Ui, r: [i32; 4], kind: u32) {
    let hi = pal(ui, CP_HI);
    let sh = pal(ui, CP_SH);
    let dk = pal(ui, CP_DK);
    let l = ui.hair();
    let inner = inset(r, l);
    match kind {
        FR_RAISED => {
            // outer: white / black, inner: face / grey
            bevel(ui, r, hi, dk);
            let face = pal(ui, CP_FACE);
            bevel(ui, inner, face, sh);
        }
        FR_SUNKEN => {
            // outer: grey / white, inner: black / face
            bevel(ui, r, sh, hi);
            let face = pal(ui, CP_FACE);
            bevel(ui, inner, dk, face);
        }
        FR_PRESSED => {
            // outer: black all round, inner: grey
            bevel(ui, r, dk, dk);
            bevel(ui, inner, sh, sh);
        }
        FR_THIN_RAISED => bevel(ui, r, hi, sh),
        FR_THIN_SUNKEN => bevel(ui, r, sh, hi),
        FR_ETCHED => {
            bevel(ui, r, sh, hi);
            bevel(ui, inner, hi, sh);
        }
        _ => bevel(ui, r, dk, dk), // FR_BLACK
    }
}

/// w_baseline(ts, y, h): baseline of style ts centred in y .. y + h (selects the font)
pub(crate) fn baseline(ui: &mut Ui, ts: u32, y: i32, h: i32) -> i32 {
    let line = ui.ts_line(ts);
    let y = y.wrapping_add(h.wrapping_sub(line) >> 1);
    ui.ts_base(ts).wrapping_add(y)
}

/// w_text(x, baseline, s, argb, maxw): text in the current font (maxw 0 = unclipped)
pub(crate) fn text(ui: &mut Ui, x: i32, y: i32, s: &str, argb: u32, maxw: i32) {
    let (font, size) = ui.text.get_font();
    let b = s.as_bytes();
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    ui.cmd_text(x, y, &b[..n], font, size, argb, maxw);
}

/// cls_text_mid(r, s, argb, ts): centred one-line text (None: nothing)
pub(crate) fn text_mid(ui: &mut Ui, r: [i32; 4], s: Option<&str>, argb: u32, ts: u32) {
    let Some(s) = s else { return };
    ui.ts_font(ts);
    let tw = ui.text.width(s);
    let x = (r[2].wrapping_sub(tw) >> 1).wrapping_add(r[0]);
    let y = baseline(ui, ts, r[1], r[3]);
    text(ui, x, y, s, argb, 0);
}

/// cls_text_at(x, r, s, argb, ts): text from x, vertically centred in the rectangle (None:
/// nothing)
pub(crate) fn text_at(ui: &mut Ui, x: i32, r: [i32; 4], s: Option<&str>, argb: u32, ts: u32) {
    let Some(s) = s else { return };
    let y = baseline(ui, ts, r[1], r[3]);
    text(ui, x, y, s, argb, 0);
}

/// cls_icon_mid(r, cp, dip, argb): icon of dip size centred in the rectangle
pub(crate) fn icon_mid(ui: &mut Ui, r: [i32; 4], cp: u32, dip: i32, argb: u32) {
    ui.draw_color(argb);
    let s = ui.dp(dip);
    let x = (r[2].wrapping_sub(s) >> 1).wrapping_add(r[0]);
    let y = (r[3].wrapping_sub(s) >> 1).wrapping_add(r[1]);
    ui.draw_icon(cp, x, y, s << 8);
}

/// cls_dots(r): dotted rectangle in the text colour (every other hairline)
pub(crate) fn dots(ui: &mut Ui, r: [i32; 4]) {
    let c = ui.color(T_TEXT);
    ui.draw_color(c);
    let dot = ui.hair();
    let step = dot + dot;
    // top and bottom rows
    let mut i = 0;
    while i < r[2] {
        ui.draw_rect(r[0].wrapping_add(i), r[1], dot, dot);
        ui.draw_rect(r[0].wrapping_add(i), r[1].wrapping_add(r[3]).wrapping_sub(dot), dot, dot);
        i += step;
    }
    // left and right columns
    let mut i = 0;
    while i < r[3] {
        ui.draw_rect(r[0], r[1].wrapping_add(i), dot, dot);
        ui.draw_rect(r[0].wrapping_add(r[2]).wrapping_sub(dot), r[1].wrapping_add(i), dot, dot);
        i += step;
    }
}
