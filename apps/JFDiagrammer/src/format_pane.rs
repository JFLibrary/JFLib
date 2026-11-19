// format_pane -- (JD11) the task pane Shape formatieren (AB-05, FM-01 .. FM-04 simple): the
// format of the selection's first shape, changed for every selected shape
//
//   Füllung und Linie   fill: none / solid, colour (colour menu), transparency; line: none /
//                       solid, colour, transparency, weight, dash, cap, line ends and their
//                       size, rounding
//   Effekte             shadow: none / outer, colour, transparency, offset; the 3D effects
//                       (P3) stay disabled
//   Text                (later: the dialog Text, RS-09)
//
// Every control shows the value the engine computes (a "Themed" cell's design value) and
// writes through format_cells::set: one undo step, guarded cells left alone.

use alloc::format;
use alloc::string::String;

use ui::i18n::t;
use ui::*;

use crate::format_cells::{self as fc, F};
use crate::format_menus::{self as fm, Target};
use crate::Diagrammer;

const ID: u32 = 0x5E80_0000;

fn heading(ui: &mut Ui, s: &str) {
    ui.spacer(6);
    ui.text(TS_BODY_STRONG, s);
    ui.spacer(2);
}

fn label(ui: &mut Ui, s: &str) {
    ui.next_size(110, 0);
    ui.text(TS_BODY, s);
}

// a colour well that opens the colour menu
fn color_row(app: &mut Diagrammer, ui: &mut Ui, id: u32, target: Target, cell: F) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    label(ui, t("Farbe"));
    let cur = fc::color(app, cell);
    ui.next_size(44, 22);
    if ui.swatch(id, 0xFF00_0000 | cur.unwrap_or(0xFFFFFF), false) & EV_ACT != 0 {
        ui.popup_open(id + 1);
    }
    ui.tooltip(&cur.map_or(String::new(), |c| format!("#{c:06X}")));
    if ui.flyout_begin(id + 1, 0) {
        if let Some(f) = fc::color_menu(app, ui, id + 0x100, cur) {
            fm::set_color(app, ui, target, f);
            ui.popup_close();
        }
    }
    ui.flyout_end();
    ui.box_end();
}

// a transparency slider in per cent
fn trans_row(app: &mut Diagrammer, ui: &mut Ui, id: u32, cell: &'static str, what: &str) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    label(ui, t("Transparenz"));
    let mut v = (fc::num(app, F::Cell(cell)).unwrap_or(0.0) * 100.0 + 0.5) as i32;
    ui.next_size(-1, 0);
    let ev = ui.slider(id, &mut v, &Range::new(0, 100, 1, 0));
    if ev & EV_COMMIT != 0 {
        fc::set(app, ui, what, &[(F::Cell(cell), format!("THEMEGUARD({v}%)"))]);
    }
    ui.next_size(44, 0);
    ui.text(TS_BODY, &format!("{v} %"));
    ui.box_end();
}

fn number_row(app: &mut Diagrammer, ui: &mut Ui, id: u32, name: &str, cell: &'static str, unit: (&str, f64), rf: &RangeF, what: &str) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    label(ui, name);
    let mut v = fc::num(app, F::Cell(cell)).unwrap_or(0.0) * unit.1;
    ui.next_size(90, 0);
    let ev = ui.number_box_f(id, &mut v, rf);
    ui.text(TS_BODY, unit.0);
    if ev & EV_COMMIT != 0 || (ev & EV_ACT != 0 && ui.focused() != id) {
        let u = match unit.0 {
            "mm" => "MM",
            "in" => "IN",
            _ => "PT",
        };
        fc::set(app, ui, what, &[(F::Cell(cell), format!("THEMEGUARD({v}{u})"))]);
    }
    ui.box_end();
}

fn choice_row(app: &mut Diagrammer, ui: &mut Ui, id: u32, name: &str, cell: &'static str, values: &[u32], labels: &[&str], what: &str) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    label(ui, name);
    let cur = fc::num(app, F::Cell(cell)).map(|v| v as u32);
    let mut sel = cur.and_then(|c| values.iter().position(|v| *v == c)).map_or(-1, |k| k as i32);
    let before = sel;
    ui.next_size(-1, 0);
    ui.combobox(id, &mut sel, labels);
    if sel != before && sel >= 0 {
        fc::set(app, ui, what, &[(F::Cell(cell), format!("THEMEGUARD({})", values[sel as usize]))]);
    }
    ui.box_end();
}

// none / on as two radio buttons over a pattern cell
fn on_off(app: &mut Diagrammer, ui: &mut Ui, id: u32, cell: &'static str, off: &str, on: &str, what: &str) -> bool {
    let p = fc::num(app, F::Cell(cell)).unwrap_or(1.0);
    let mut v = i32::from(p != 0.0);
    let before = v;
    ui.radio(id, off, &mut v, 0);
    ui.radio(id + 1, on, &mut v, 1);
    if v != before {
        fc::set(app, ui, what, &[(F::Cell(cell), format!("THEMEGUARD({v})"))]);
    }
    v != 0
}

fn fill(app: &mut Diagrammer, ui: &mut Ui) {
    heading(ui, t("Füllung"));
    if on_off(app, ui, ID + 0x10, "FillPattern", t("Keine Füllung"), t("Einfarbige Füllung"), t("Füllung")) {
        color_row(app, ui, ID + 0x20, Target::Fill, F::Cell("FillForegnd"));
        trans_row(app, ui, ID + 0x12, "FillForegndTrans", t("Transparenz der Füllung"));
    }
}

fn line(app: &mut Diagrammer, ui: &mut Ui) {
    heading(ui, crate::tc("Linie|Abschnitt"));
    if !on_off(app, ui, ID + 0x30, "LinePattern", t("Keine Linie"), t("Durchgezogene Linie"), crate::tc("Linie|Format")) {
        return;
    }
    color_row(app, ui, ID + 0x40, Target::Line, F::Cell("LineColor"));
    trans_row(app, ui, ID + 0x32, "LineColorTrans", t("Transparenz der Linie"));
    number_row(app, ui, ID + 0x33, t("Stärke"), "LineWeight", ("pt", 72.0), &RangeF::new(0.0, 72.0, 0.25, 2, 0), t("Linienstärke"));
    let dashes: alloc::vec::Vec<u32> = fm::DASHES.iter().map(|d| d.0).collect();
    let pics: alloc::vec::Vec<&str> = fm::DASHES.iter().map(|d| d.1).collect();
    choice_row(app, ui, ID + 0x34, t("Strichtyp"), "LinePattern", &dashes, &pics, t("Strichtyp"));
    choice_row(app, ui, ID + 0x35, t("Abschlusstyp"), "LineCap", &[0, 1, 2], &[t("Rund"), t("Quadratisch"), t("Flach")], t("Abschlusstyp"));
    let ends: alloc::vec::Vec<u32> = (0..=45).collect();
    let names: alloc::vec::Vec<String> = ends.iter().map(|k| if *k == 0 { String::from(t("Ohne")) } else { format!("{k}") }).collect();
    let names: alloc::vec::Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    let sizes = [t("Sehr klein"), t("Klein"), t("Mittel"), t("Groß"), t("Extra groß"), t("Riesig"), t("Kolossal")];
    choice_row(app, ui, ID + 0x36, t("Anfangspfeil"), "BeginArrow", &ends, &names, t("Pfeile"));
    choice_row(app, ui, ID + 0x37, t("Anfangsgröße"), "BeginArrowSize", &[0, 1, 2, 3, 4, 5, 6], &sizes, t("Pfeile"));
    choice_row(app, ui, ID + 0x38, t("Endpfeil"), "EndArrow", &ends, &names, t("Pfeile"));
    choice_row(app, ui, ID + 0x39, t("Endgröße"), "EndArrowSize", &[0, 1, 2, 3, 4, 5, 6], &sizes, t("Pfeile"));
    let (u, k) = if crate::edit_canvas::metric(app) { ("mm", 25.4) } else { ("in", 1.0) };
    number_row(app, ui, ID + 0x3A, t("Abrundung"), "Rounding", (u, k), &RangeF::new(0.0, 1000.0, 0.5, 2, 0), t("Abrundung"));
}

fn shadow(app: &mut Diagrammer, ui: &mut Ui) {
    heading(ui, t("Schatten"));
    let on = on_off(app, ui, ID + 0x50, "ShdwPattern", t("Kein Schatten"), t("Schatten außen"), t("Schatten"));
    if !on {
        return;
    }
    color_row(app, ui, ID + 0x60, Target::Shadow, F::Cell("ShdwForegnd"));
    trans_row(app, ui, ID + 0x52, "ShdwForegndTrans", t("Transparenz des Schattens"));
    let (u, k) = if crate::edit_canvas::metric(app) { ("mm", 25.4) } else { ("in", 1.0) };
    number_row(app, ui, ID + 0x53, t("Abstand X"), "ShapeShdwOffsetX", (u, k), &RangeF::new(-1000.0, 1000.0, 0.5, 2, 0), t("Schatten"));
    number_row(app, ui, ID + 0x54, t("Abstand Y"), "ShapeShdwOffsetY", (u, k), &RangeF::new(-1000.0, 1000.0, 0.5, 2, 0), t("Schatten"));
}

/// the pane
pub(crate) fn pane(app: &mut Diagrammer, ui: &mut Ui, id_pane: u32) {
    ui.next_size(-1, -1);
    ui.scroll_begin(id_pane + 0x10, SC_V | BOX_V | pad(10));
    ui.next_size(-1, 28);
    ui.tabs(id_pane + 0x11, &mut app.format_tab, &[t("Füllung und Linie"), t("Effekte"), t("Text")]);
    ui.spacer(6);
    let sel = !fc::selection(app).is_empty();
    if sel && app.format_tab == 0 {
        fill(app, ui);
        line(app, ui);
    } else if sel && app.format_tab == 1 {
        shadow(app, ui);
    }
    // the parts that come later (P2, P3), disabled
    let later: &[&str] = match app.format_tab {
        1 => &["Spiegelung", "Leuchten", "Weiche Kanten", "3D-Format", "3D-Drehung"], // i18n
        2 => &["Textfeld", "Textblock"], // i18n
        _ => &[],
    };
    for (k, s) in later.iter().enumerate() {
        ui.next_size(-1, 28);
        ui.next_flags(ST_SUBTLE | ST_ALIGN_START | ST_DISABLED);
        ui.next_icon(IC_CHEV_RIGHT);
        ui.button(id_pane + 0x18 + k as u32, crate::tc(s));
    }
    if !sel {
        ui.spacer(8);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Kein Shape ausgewählt."));
    }
    ui.scroll_end();
}
