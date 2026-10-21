// common -- drawing helpers the BackupCraft widget files share
//
//   fill_rrect(x, y, w, h, r, argb)          filled rounded rectangle (skipped when transparent)
//   stroke_rrect(x, y, w, h, r, t, argb)     rounded border of thickness t
//   baseline(ts, y, h) -> px                 baseline of a text style centred in y .. y + h
//   text(x, baseline, s, argb, maxw)         text in the current font (maxw 0 = unclipped)
//   shadow(r, radius, depth, alpha_light, alpha_dark)   a soft drop shadow
//   radius constants: R_CTRL 5, R_CARD 8, R_FLYOUT 8, R_DIALOG 10 (DIP)

use ui::Ui;

pub(crate) const R_CTRL: i32 = 5;
pub(crate) const R_CARD: i32 = 8;
pub(crate) const R_FLYOUT: i32 = 8;
pub(crate) const R_DIALOG: i32 = 10;

/// filled rounded rectangle (skipped when transparent)
#[allow(clippy::too_many_arguments)]
pub(crate) fn fill_rrect(ui: &mut Ui, x: i32, y: i32, w: i32, h: i32, r: i32, argb: u32) {
    if argb & 0xFF00_0000 == 0 || w <= 0 || h <= 0 {
        return;
    }
    ui.cmd_rrect(x, y, w, h, r, 0, argb, 0, 0);
}

/// rounded border of thickness t
#[allow(clippy::too_many_arguments)]
pub(crate) fn stroke_rrect(ui: &mut Ui, x: i32, y: i32, w: i32, h: i32, r: i32, t: i32, argb: u32) {
    if argb & 0xFF00_0000 == 0 || w <= 0 || h <= 0 {
        return;
    }
    ui.cmd_rrect(x, y, w, h, r, t, 0, argb, argb);
}

/// baseline of style ts centred in y .. y + h (selects the font)
pub(crate) fn baseline(ui: &mut Ui, ts: u32, y: i32, h: i32) -> i32 {
    let line = ui.ts_line(ts);
    let y = y.wrapping_add(h.wrapping_sub(line) >> 1);
    ui.ts_base(ts).wrapping_add(y)
}

/// text in the current font (maxw 0 = unclipped)
pub(crate) fn text(ui: &mut Ui, x: i32, y: i32, s: &str, argb: u32, maxw: i32) {
    let (font, size) = ui.text.get_font();
    let b = s.as_bytes();
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    ui.cmd_text(x, y, &b[..n], font, size, argb, maxw);
}

/// a soft drop shadow under r: the edge alpha is about `light` / `dark` (theme)
pub(crate) fn shadow(ui: &mut Ui, r: [i32; 4], rad: i32, depth: i32, light: u32, dark: u32) {
    let a0 = if ui.theme_dark() { dark } else { light };
    // cmd_shadow stacks `depth` layers: a = A * 4d / ((d + 1)(d + 2)) gives about A at the edge
    let num = ((depth << 2) as u32).wrapping_mul(a0);
    let den = (depth.wrapping_add(1).wrapping_mul(depth.wrapping_add(2))) as u32;
    let a = if den == 0 { 0 } else { ((num / den) as i32).clamp(1, 255) };
    ui.draw_color((a as u32) << 24);
    ui.cmd_shadow(r[0], r[1].wrapping_add(ui.dp(2)), r[2], r[3], rad, depth);
}
