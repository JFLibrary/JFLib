// dlg_text -- (JD8 rest) the dialog Text (RS-09, KB-10: F11, the launchers of Start ›
// Schriftart and Absatz, Formatieren › Text formatieren): font, paragraph and text block of the
// selected shapes
//
//   TextDlg                     the dialog's state: register, shapes, the values at the start
//                               and now
//   open(app, ui)               TEXT_DIALOG, FORMAT_TEXT: the first selected shape's values (the
//                               text being edited ends first)
//   dialog(app, ui)             every pass; OK writes what changed into every shape (one undo
//                               step, format_cells::set_on)
//
// Registers and Visio's cells (Character, Paragraph, Text Block Format; [MS-VSDX], the same as
// format_menus.rs writes):
//   Schriftart  Char.Font (THEMEGUARD(FONT("…"))), Char.Size (pt), Char.Style (bits 1 bold,
//               2 italic, 4 underline; the other bits stay), Char.Strikethru, Char.Color
//               (format_cells::color_formula)
//   Zeichen     (JD15) Char.Pos (0 normal, 1 superscript, 2 subscript), Char.Case (0 normal,
//               1 all capitals, 2 initial capitals), Char.FontScale (%), Char.Letterspace (pt)
//   Absatz      Para.HorzAlign (0 left .. 3 justified), Para.IndLeft, IndRight, IndFirst (the
//               page's unit), Para.SpBefore, SpAfter (pt), Para.SpLine (a percentage of the font
//               size: written negative, -120% = 1.2 lines, Visio's rule for SpLine below 0)
//   Textblock   VerticalAlign (0 top, 1 middle, 2 bottom), TopMargin, BottomMargin, LeftMargin,
//               RightMargin (pt), TextBkgnd (0 none, else a colour), TextBkgndTrans
// Only the values the user changed are written: cells that follow the design stay so.  Tab
// stops and bullets (Visio's registers Tabstopps and Aufzählungszeichen) are P2 and not here.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::formula::{Color, Val};
use diagram_core::ShapeRef;
use ui::i18n::t;
use ui::*;

use crate::format_cells::{self as fc, F};
use crate::Diagrammer;

const ID: u32 = 0x7810_0000;
const ID_COLOR: u32 = ID + 0x100;
const ID_BKCOLOR: u32 = ID + 0x200;

/// the values of the dialog
#[derive(Clone, PartialEq, Default, Debug)]
struct Vals {
    font: i32,
    size: f64,
    bold: i32,
    italic: i32,
    underline: i32,
    strike: i32,
    color: u32,
    halign: i32,
    indent: [f64; 3],
    space: [f64; 2],
    line: f64,
    valign: i32,
    margins: [f64; 4],
    bk: i32,
    bk_color: u32,
    bk_trans: f64,
    /// (JD15, RS-09) Zeichen: Char.Pos, Char.Case, Char.FontScale (%), Char.Letterspace (pt)
    pos: i32,
    case: i32,
    scale: f64,
    letterspace: f64,
}

/// the dialog's state
#[derive(Default)]
pub(crate) struct TextDlg {
    pub open: Shared<i32>,
    pub tab: i32,
    shapes: Vec<ShapeRef>,
    fonts: Vec<String>,
    style: u32,
    v0: Vals,
    v: Vals,
    color: Option<String>,
    bk_color: Option<String>,
    metric: bool,
}

/// Text (TEXT_DIALOG, FORMAT_TEXT): the dialog with the first selected shape's values
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui) {
    if app.text.is_some() {
        crate::edit_text::finish(app, ui);
    }
    let shapes = fc::selection(app);
    let Some(&first) = shapes.first() else { return };
    let metric = crate::edit_canvas::metric(app);
    let k = if metric { 25.4 } else { 1.0 };
    let fonts = crate::format_menus::font_list(app);
    let num = |app: &mut Diagrammer, f: F| match fc::value(app, first, f) {
        Some(Val::Num(x)) if x.is_finite() => Some(x),
        _ => None,
    };
    let font = match fc::value(app, first, F::Char("Font")) {
        Some(Val::Str(s)) | Some(Val::Font(s)) => fonts.iter().position(|f| f.eq_ignore_ascii_case(&s)).map_or(-1, |p| p as i32),
        _ => -1,
    };
    let style = num(app, F::Char("Style")).unwrap_or(0.0).max(0.0) as u32;
    let size = num(app, F::Char("Size")).unwrap_or(12.0 / 72.0) * 72.0;
    let line = match num(app, F::Para("SpLine")) {
        Some(x) if x < 0.0 => -x * 100.0,
        Some(x) if x > 0.0 && size > 0.0 => x * 72.0 / size * 100.0,
        _ => 120.0,
    };
    let bk_raw = fc::value(app, first, F::Cell("TextBkgnd"));
    let (bk, bk_color) = match bk_raw {
        Some(Val::Num(x)) if x >= 1.0 => (1, palette_color(app, x as u32 - 1)),
        Some(Val::Color(Color::Rgb(r, g, b))) => (1, (r as u32) << 16 | (g as u32) << 8 | b as u32),
        Some(Val::Color(Color::Index(i))) if i >= 1 => (1, palette_color(app, i - 1)),
        Some(Val::Str(s)) => match diagram_core::render::parse_color(&s) {
            Some(c) => (1, c),
            None => (0, 0xFFFFFF),
        },
        _ => (0, 0xFFFFFF),
    };
    let v = Vals {
        font,
        size: round2(size),
        bold: (style & 1 != 0) as i32,
        italic: (style & 2 != 0) as i32,
        underline: (style & 4 != 0) as i32,
        strike: num(app, F::Char("Strikethru")).is_some_and(|x| x != 0.0) as i32,
        color: fc::color(app, F::Char("Color")).unwrap_or(0),
        halign: num(app, F::Para("HorzAlign")).unwrap_or(1.0).clamp(0.0, 3.0) as i32,
        indent: [
            round2(num(app, F::Para("IndLeft")).unwrap_or(0.0) * k),
            round2(num(app, F::Para("IndRight")).unwrap_or(0.0) * k),
            round2(num(app, F::Para("IndFirst")).unwrap_or(0.0) * k),
        ],
        space: [round2(num(app, F::Para("SpBefore")).unwrap_or(0.0) * 72.0), round2(num(app, F::Para("SpAfter")).unwrap_or(0.0) * 72.0)],
        line: round2(line),
        valign: num(app, F::Cell("VerticalAlign")).unwrap_or(1.0).clamp(0.0, 2.0) as i32,
        margins: ["TopMargin", "BottomMargin", "LeftMargin", "RightMargin"].map(|c| round2(num(app, F::Cell(c)).unwrap_or(0.0) * 72.0)),
        bk,
        bk_color,
        bk_trans: round2(num(app, F::Cell("TextBkgndTrans")).unwrap_or(0.0) * 100.0),
        pos: num(app, F::Char("Pos")).unwrap_or(0.0).clamp(0.0, 2.0) as i32,
        case: num(app, F::Char("Case")).unwrap_or(0.0).clamp(0.0, 2.0) as i32,
        scale: round2(num(app, F::Char("FontScale")).unwrap_or(1.0) * 100.0),
        letterspace: round2(num(app, F::Char("Letterspace")).unwrap_or(0.0) * 72.0),
    };
    let d = &mut app.txt.text;
    d.shapes = shapes;
    d.fonts = fonts;
    d.style = style;
    d.v0 = v.clone();
    d.v = v;
    d.color = None;
    d.bk_color = None;
    d.metric = metric;
    d.open.set(1);
    ui.repass();
}

fn round2(x: f64) -> f64 {
    diagram_core::formula::math::round(x * 100.0) / 100.0
}

fn palette_color(app: &Diagrammer, i: u32) -> u32 {
    let doc = app.opened.as_ref().map(|o| &o.doc);
    doc.and_then(|d| d.colors.iter().find(|c| c.ix == i).and_then(|c| diagram_core::render::parse_color(d.str(c.rgb))))
        .unwrap_or_else(|| diagram_core::render::style::PALETTE.get(i as usize).copied().unwrap_or(0))
}

fn label(ui: &mut Ui, s: &str) {
    ui.next_size(170, 0);
    ui.text(TS_BODY, s);
}

fn number(ui: &mut Ui, id: u32, name: &str, v: &mut f64, rf: &RangeF, unit: &str) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    label(ui, name);
    ui.next_size(100, 0);
    ui.number_box_f(id, v, rf);
    ui.text(TS_BODY, unit);
    ui.box_end();
}

// a colour swatch that opens the colour menu -> the formula picked
fn color_row(app: &mut Diagrammer, ui: &mut Ui, id: u32, name: &str, rgb: u32) -> Option<(String, u32)> {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    label(ui, name);
    let ev = ui.swatch(id, 0xFF00_0000 | rgb, false);
    ui.tooltip(name);
    if ev & EV_ACT != 0 {
        ui.popup_open(id + 1);
    }
    let mut out = None;
    if ui.flyout_begin(id + 1, 0) {
        let p = fc::palette(app);
        let names = fc::theme_names();
        let recent = app.format.recent.clone();
        let m = ColorMenu {
            theme: &p.theme,
            theme_names: &names,
            variant: &p.variant,
            standard: &fc::STANDARD,
            recent: &recent,
            captions: [t("Designfarben"), t("Variantenfarben"), t("Standardfarben"), t("Zuletzt verwendet")],
            current: Some(rgb),
        };
        if let Some((pick, c)) = ui.color_menu(id + 2, &m) {
            fc::used(app, c);
            out = Some((fc::color_formula(app, pick, c), c));
            ui.popup_close();
        }
    }
    ui.flyout_end();
    ui.box_end();
    out
}

/// the dialog (every pass)
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.txt.text.open.clone();
    if !ui.dialog_begin(ID, crate::tc("Text|Dialogfeld"), &open) {
        return;
    }
    // (JD15, RS-09) Zeichen second, as in Visio
    let regs = [t("Schriftart"), t("Zeichen"), t("Absatz"), t("Textblock")];
    ui.next_size(480, 0);
    let mut tab = app.txt.text.tab;
    ui.tabs(ID + 1, &mut tab, &regs);
    app.txt.text.tab = tab;
    ui.next_size(480, 290);
    ui.box_begin(ID + 2, BOX_V, 8);
    let mut v = app.txt.text.v.clone();
    let unit = if app.txt.text.metric { "mm" } else { "in" };
    match tab {
        0 => {
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            label(ui, t("Schriftart"));
            let names: Vec<&str> = app.txt.text.fonts.iter().map(|s| s.as_str()).collect();
            ui.next_size(220, 0);
            ui.combobox(ID + 10, &mut v.font, &names);
            ui.box_end();
            number(ui, ID + 11, t("Schriftgrad"), &mut v.size, &RangeF::new(1.0, 1638.0, 1.0, 1, 0), "pt");
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            label(ui, t("Schriftschnitt"));
            ui.checkbox(ID + 12, crate::commands::label(crate::commands::BOLD), &mut v.bold);
            ui.checkbox(ID + 13, crate::commands::label(crate::commands::ITALIC), &mut v.italic);
            ui.box_end();
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            label(ui, "");
            ui.checkbox(ID + 14, crate::commands::label(crate::commands::UNDERLINE), &mut v.underline);
            ui.checkbox(ID + 15, crate::commands::label(crate::commands::STRIKE), &mut v.strike);
            ui.box_end();
            if let Some((f, c)) = color_row(app, ui, ID_COLOR, crate::commands::label(crate::commands::FONT_COLOR), v.color) {
                app.txt.text.color = Some(f);
                v.color = c;
            }
        }
        1 => {
            // (JD15, RS-09) Zeichen: position, case, scale, spacing
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            label(ui, t("Position|Zeichen"));
            ui.next_size(160, 0);
            ui.combobox(ID + 40, &mut v.pos, &[t("Normal"), t("Hochgestellt"), t("Tiefgestellt")]);
            ui.box_end();
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            label(ui, t("Groß-/Kleinschreibung"));
            ui.next_size(160, 0);
            ui.combobox(ID + 41, &mut v.case, &[t("Normal"), t("Großbuchstaben"), t("Erster Buchstabe groß")]);
            ui.box_end();
            number(ui, ID + 42, t("Skalierung"), &mut v.scale, &RangeF::new(1.0, 600.0, 10.0, 0, 0), "%");
            number(ui, ID + 43, t("Zeichenabstand"), &mut v.letterspace, &RangeF::new(-100.0, 1000.0, 0.5, 1, 0), "pt");
        }
        2 => {
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            label(ui, t("Ausrichtung"));
            ui.next_size(160, 0);
            ui.combobox(ID + 20, &mut v.halign, &[t("Linksbündig"), t("Zentriert"), t("Rechtsbündig"), t("Blocksatz")]);
            ui.box_end();
            let rl = RangeF::new(-100.0, 1000.0, if app.txt.text.metric { 1.0 } else { 0.1 }, 2, 0);
            ui.text(TS_BODY_STRONG, t("Einzug"));
            number(ui, ID + 21, crate::tc("Links|Rand"), &mut v.indent[0], &rl, unit);
            number(ui, ID + 22, t("Rechts"), &mut v.indent[1], &rl, unit);
            number(ui, ID + 23, t("Erste Zeile"), &mut v.indent[2], &rl, unit);
            ui.text(TS_BODY_STRONG, t("Abstand"));
            let rs = RangeF::new(0.0, 1000.0, 1.0, 1, 0);
            number(ui, ID + 24, t("Vor"), &mut v.space[0], &rs, "pt");
            number(ui, ID + 25, t("Nach"), &mut v.space[1], &rs, "pt");
            number(ui, ID + 26, t("Zeilenabstand"), &mut v.line, &RangeF::new(10.0, 1000.0, 10.0, 0, 0), "%");
        }
        _ => {
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            label(ui, t("Vertikale Ausrichtung"));
            ui.next_size(160, 0);
            ui.combobox(ID + 30, &mut v.valign, &[t("Oben"), t("Mitte"), t("Unten")]);
            ui.box_end();
            ui.text(TS_BODY_STRONG, t("Ränder"));
            let rm = RangeF::new(0.0, 1000.0, 1.0, 1, 0);
            number(ui, ID + 31, t("Oben"), &mut v.margins[0], &rm, "pt");
            number(ui, ID + 32, t("Unten"), &mut v.margins[1], &rm, "pt");
            number(ui, ID + 33, crate::tc("Links|Rand"), &mut v.margins[2], &rm, "pt");
            number(ui, ID + 34, t("Rechts"), &mut v.margins[3], &rm, "pt");
            ui.text(TS_BODY_STRONG, t("Texthintergrund"));
            ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
            ui.radio(ID + 35, t("Keiner"), &mut v.bk, 0);
            ui.radio(ID + 36, t("Farbe"), &mut v.bk, 1);
            ui.box_end();
            if v.bk == 1 {
                if let Some((f, c)) = color_row(app, ui, ID_BKCOLOR, t("Hintergrundfarbe"), v.bk_color) {
                    app.txt.text.bk_color = Some(f);
                    v.bk_color = c;
                }
                number(ui, ID + 37, t("Transparenz"), &mut v.bk_trans, &RangeF::new(0.0, 100.0, 5.0, 0, 0), "%");
            }
        }
    }
    ui.box_end();
    app.txt.text.v = v;
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        apply(app, ui);
    }
    if r != DLG_NONE {
        app.txt.text.open.set(0);
        ui.repass();
    }
}

// what changed, into every shape of the dialog (one undo step)
fn apply(app: &mut Diagrammer, ui: &mut Ui) {
    let d = &app.txt.text;
    let (v, v0) = (&d.v, &d.v0);
    let unit = if d.metric { "mm" } else { "in" };
    let mut ch: Vec<(F, String)> = Vec::new();
    if v.font != v0.font && v.font >= 0 {
        if let Some(name) = d.fonts.get(v.font as usize) {
            ch.push((F::Char("Font"), format!("THEMEGUARD(FONT(\"{}\"))", name.replace('"', ""))));
        }
    }
    if v.size != v0.size {
        ch.push((F::Char("Size"), format!("{}PT", v.size)));
    }
    if (v.bold, v.italic, v.underline) != (v0.bold, v0.italic, v0.underline) {
        let bits = (d.style & !7) | v.bold as u32 | (v.italic as u32) << 1 | (v.underline as u32) << 2;
        ch.push((F::Char("Style"), format!("{bits}")));
    }
    if v.strike != v0.strike {
        ch.push((F::Char("Strikethru"), String::from(if v.strike != 0 { "TRUE" } else { "FALSE" })));
    }
    if let Some(f) = &d.color {
        ch.push((F::Char("Color"), f.clone()));
    }
    // (JD15, RS-09) Zeichen
    if v.pos != v0.pos {
        ch.push((F::Char("Pos"), format!("{}", v.pos)));
    }
    if v.case != v0.case {
        ch.push((F::Char("Case"), format!("{}", v.case)));
    }
    if v.scale != v0.scale {
        ch.push((F::Char("FontScale"), format!("{}%", v.scale)));
    }
    if v.letterspace != v0.letterspace {
        ch.push((F::Char("Letterspace"), format!("{} pt", v.letterspace)));
    }
    if v.halign != v0.halign {
        ch.push((F::Para("HorzAlign"), format!("{}", v.halign)));
    }
    for (k, cell) in ["IndLeft", "IndRight", "IndFirst"].iter().enumerate() {
        if v.indent[k] != v0.indent[k] {
            ch.push((F::Para(cell), format!("{} {unit}", v.indent[k])));
        }
    }
    for (k, cell) in ["SpBefore", "SpAfter"].iter().enumerate() {
        if v.space[k] != v0.space[k] {
            ch.push((F::Para(cell), format!("{} pt", v.space[k])));
        }
    }
    if v.line != v0.line {
        ch.push((F::Para("SpLine"), format!("-{}%", v.line)));
    }
    if v.valign != v0.valign {
        ch.push((F::Cell("VerticalAlign"), format!("{}", v.valign)));
    }
    for (k, cell) in ["TopMargin", "BottomMargin", "LeftMargin", "RightMargin"].iter().enumerate() {
        if v.margins[k] != v0.margins[k] {
            ch.push((F::Cell(cell), format!("{} pt", v.margins[k])));
        }
    }
    if v.bk == 0 && v0.bk != 0 {
        ch.push((F::Cell("TextBkgnd"), String::from("0")));
    }
    if v.bk == 1 {
        match &d.bk_color {
            Some(f) => ch.push((F::Cell("TextBkgnd"), f.clone())),
            None if v0.bk == 0 => {
                let c = v.bk_color;
                ch.push((F::Cell("TextBkgnd"), format!("THEMEGUARD(RGB({},{},{}))", (c >> 16) & 255, (c >> 8) & 255, c & 255)));
            }
            None => {}
        }
        if v.bk_trans != v0.bk_trans {
            ch.push((F::Cell("TextBkgndTrans"), format!("{}%", v.bk_trans)));
        }
    }
    if ch.is_empty() {
        return;
    }
    let shapes = d.shapes.clone();
    fc::set_on(app, ui, crate::tc("Text|Dialogfeld"), &shapes, &ch);
}
