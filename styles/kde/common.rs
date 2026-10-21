// common -- the core's small drawing helpers the kde widget files call    [src/ui/ui_list.s]
//
//   fill_rrect(x, y, w, h, r, argb)     w_fill_rrect: filled rounded rectangle (skipped when
//                                       transparent)
//   stroke_rrect(x, y, w, h, r, t, argb)    w_stroke_rrect: rounded border of thickness t
//   baseline(ts, y, h) -> px            w_baseline: baseline of a text style centred in
//                                       y .. y + h (selects the font)
//   text(x, baseline, s, argb, maxw)    w_text: text in the current font (maxw 0 = unclipped)
//
// (The asm style calls these core functions; the Rust core keeps them private, so the style
// has its own copies, as styles/winui does.)

use ui::Ui;

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
