// jfOffice: Small drawing helpers shared by the local paint modules.
// Uses jfguiLib drawing primitives; local implementations derived from the
// existing native widget contract. No WordCraft code or assets.

use ui::{Ui, consts::FONT_REG};

/// Ribbon/menu typography uses a separate 11.5-DIP control rung.
pub(crate) fn control_font(ui: &mut Ui) {
    ui.text.set_font(FONT_REG, ui.dp_fx(11 * 256 + 128));
}

/// Baseline centered using the selected face's own ascender and descender.
pub(crate) fn current_baseline(ui: &mut Ui, y: i32, h: i32) -> i32 {
    let asc = ui.text.ascent();
    let desc = ui.text.descent();
    y + (h - asc - desc) / 2 + asc
}

/// w_fill_rrect(x, y, w, h, r, argb): filled rounded rectangle (skipped when transparent)
#[allow(clippy::too_many_arguments)]
pub(crate) fn fill_rrect(ui: &mut Ui, x: i32, y: i32, w: i32, h: i32, r: i32, argb: u32) {
    if argb & 0xFF00_0000 == 0 {
        return;
    }
    ui.cmd_rrect(x, y, w, h, r, 0, argb, 0, 0);
}

/// w_stroke_rrect(x, y, w, h, r, t, argb): rounded border of thickness t
#[allow(clippy::too_many_arguments)]
pub(crate) fn stroke_rrect(ui: &mut Ui, x: i32, y: i32, w: i32, h: i32, r: i32, t: i32, argb: u32) {
    ui.cmd_rrect(x, y, w, h, r, t, 0, argb, argb);
}

/// w_baseline(ts, y, h) -> baseline of style ts centred in y .. y + h (selects the font)
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
