// widgets -- building blocks of the console's pages: cards, KPI tiles, form fields, loading
// and empty states, a search field; schedule.rs (the schedule editor), picker.rs (remote file
// trees with check boxes)

pub mod picker;
pub mod schedule;

use ui::*;

use crate::tr::t;

/// a card with a title (TS_BODY_STRONG) and an optional caption; call card_end
pub fn card_begin(ui: &mut Ui, title: &str, caption: &str) {
    ui.box_begin(0, BOX_V | BOX_CARD | pad(16), 10);
    if !title.is_empty() {
        ui.box_begin(0, BOX_V, 2);
        ui.text(TS_BODY_STRONG, title);
        if !caption.is_empty() {
            ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, caption);
        }
        ui.box_end();
    }
}

pub fn card_end(ui: &mut Ui) {
    ui.box_end();
}

/// a card whose title row ends with a link -> clicked
pub fn card_begin_action(ui: &mut Ui, title: &str, action_id: u32, action: &str) -> bool {
    ui.box_begin(0, BOX_V | BOX_CARD | pad(16), 10);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(-1, 0);
    ui.text(TS_BODY_STRONG | TX_ELLIPSIS, title);
    let clicked = ui.link(action_id, action, TS_BODY) & EV_ACT != 0;
    ui.box_end();
    clicked
}

/// a KPI tile: caption, big value, sparkline (empty: none), a line below
#[allow(clippy::too_many_arguments)]
pub fn kpi(ui: &mut Ui, id: u32, icon: u32, title: &str, value: &str, sub: &str, spark: &[f32], color: u32) -> u32 {
    ui.box_begin(0, BOX_V | BOX_CARD | pad(16), 4);
    let r = ui.box_rect();
    let ev = ui.hit_rect(id, r[0], r[1], r[2], r[3], 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    if icon != 0 {
        ui.icon(icon, 16);
    }
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, title);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XEND, 8);
    ui.text(TS_TITLE, value);
    ui.spacer(-1);
    if !spark.is_empty() {
        ui.next_size(100, 34);
        ui.sparkline(id + 1, spark, color);
    }
    ui.box_end();
    ui.text(TS_CAPTION | TX_TERTIARY | TX_ELLIPSIS, sub);
    ui.box_end();
    ev
}

/// a form field: the label above the control the caller emits next (hint below it)
pub fn label(ui: &mut Ui, text: &str) {
    ui.text(TS_BODY_STRONG, text);
}

/// a row of controls at their natural width (segmented controls, buttons) inside a column
/// that would otherwise stretch them; call inline_end
pub fn inline_begin(ui: &mut Ui) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
}

pub fn inline_end(ui: &mut Ui) {
    ui.box_end();
}

/// a secondary hint line
pub fn hint(ui: &mut Ui, text: &str) {
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, text);
}

/// an error line under a field
pub fn error(ui: &mut Ui, text: &str) {
    if text.is_empty() {
        return;
    }
    ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
    ui.icon(IC_ERROR, 14);
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_WRAP, text);
    ui.box_end();
}

/// a notice whose text wraps, for messages of any length (an infobar keeps to one line);
/// kind: IB_INFO, IB_SUCCESS, IB_WARNING or IB_ERROR
pub fn notice(ui: &mut Ui, kind: u32, title: &str, text: &str) {
    let (bg, fg, icon) = match kind {
        IB_ERROR => (T_CRIT_BG, T_CRIT, IC_ERROR),
        IB_WARNING => (T_WARN_BG, T_WARN, IC_WARNING),
        IB_SUCCESS => (T_OK_BG, T_OK, IC_CHECK_CIRCLE_FILL),
        _ => (T_INFO_BG, T_INFO, IC_INFO),
    };
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | pad(12), 10);
    let r = ui.box_rect();
    let p = ui.dp(12);
    let (fill, edge) = (ui.color(bg), (ui.color(fg) & 0x00FF_FFFF) | 0x5000_0000);
    let (rad, hair) = (ui.dp(6), ui.hair());
    ui.cmd_rrect(r[0] - p, r[1] - p, r[2] + p * 2, r[3] + p * 2, rad, hair, fill, edge, edge);
    let s = ui.dp(16);
    let ir = ui.place(s, s);
    let c = ui.color(fg);
    ui.draw_color(c);
    ui.draw_icon(icon, ir[0], ir[1], s << 8);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V, 2);
    if !title.is_empty() {
        ui.text(TS_BODY_STRONG | TX_WRAP, title);
    }
    if !text.is_empty() {
        ui.text(TS_BODY | TX_WRAP, text);
    }
    ui.box_end();
    ui.box_end();
}

/// a loading row (spinner and text)
pub fn loading(ui: &mut Ui, id: u32) {
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_CENTER | pad(16), 10);
    ui.spinner(id, 18);
    ui.text(TS_BODY | TX_SECONDARY, t("Wird geladen …"));
    ui.box_end();
}

/// a search field (icon, placeholder) -> changed
pub fn search(ui: &mut Ui, id: u32, buf: &mut alloc::string::String, placeholder: &str, width: i32) -> bool {
    ui.next_size(width, 0);
    let before = buf.len();
    let ev = ui.textbox_icon(id, IC_SEARCH, Some(placeholder), buf, 120);
    ev & EV_ACT != 0 || buf.len() != before
}

/// a small "key: value" grid row set (labels 160 DIP)
pub fn props(ui: &mut Ui, rows: &[(&str, &str)]) {
    ui.box_begin(0, BOX_V, 6);
    for (k, v) in rows {
        ui.key_value(k, v, 150);
    }
    ui.box_end();
}

/// the page header row: title and subtitle at the left, the caller's buttons at the right
/// (call page_head_end after them)
pub fn page_head(ui: &mut Ui, title: &str, subtitle: &str) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
    ui.next_size(-1, 0);
    ui.page_header(title, subtitle);
}

pub fn page_head_end(ui: &mut Ui) {
    ui.box_end();
}

/// whether a text contains a filter (case-insensitive; empty filter: always)
pub fn matches(text: &str, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    let f = filter.to_lowercase();
    text.to_lowercase().contains(f.as_str())
}
