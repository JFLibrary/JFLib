// format_menus -- (JD11) Start › Schriftart, Absatz, Formenarten and Format übertragen: the
// commands on the selection's cells (format_cells.rs), the ribbon's colour menus, the font boxes
//
//   FormatState                 the colours used last, the format painter, Weitere Farben
//   flyout(app, ui, pid)        the flyouts this module fills: Füllung, Linie, Schriftfarbe's
//                               arrow (colour menu, no fill / no line, weights, dashes, arrows)
//                               -> handled
//   font_boxes(app, ui, ...)    Schriftart and Schriftgrad (RS-04): the design's fonts first
//   bold / italic / ... / halign / valign / fill_none / line_none   the handlers
//   painter(app, ui)            Format übertragen (FM-07): a click takes the format of the
//                               first selected shape, the next shape clicked gets it; a second
//                               click within half a second keeps it on until Esc or the
//                               pointer tool
//   after(app, ui)              every pass: the format painter, the dialog Weitere Farben
//
// Cells (Visio's): Char.Style bits 1 bold, 2 italic, 4 underline; Char.Strikethru;
// Para.HorzAlign 0 left .. 3 justified; VerticalAlign 0 top, 1 middle, 2 bottom; FillPattern
// and LinePattern 0 = none; colours as format_cells::color_formula writes them.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{CellAddr, Formula, ShapeRef, SheetRef, Value};
use ui::i18n::t;
use ui::*;

use crate::commands::*;
use crate::format_cells::{self as fc, F};
use crate::Diagrammer;

const SPLIT: u32 = 0x8000;
const ID_WEIGHTS: u32 = 0x5E00_0000;
const ID_DASHES: u32 = 0x5E10_0000;
const ID_ARROWS: u32 = 0x5E20_0000;
const ID_COLORS: u32 = 0x5E30_0000;
const ID_MORE_DLG: u32 = 0x5E40_0000;

/// where Weitere Farben writes
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Target {
    #[default]
    Fill,
    Line,
    Font,
    Shadow,
}

/// the format painter's cells and mode
pub(crate) struct Painter {
    pub cells: Vec<(F, String)>,
    pub sticky: bool,
    // made sticky by a double click in this pass: the click's queued command passes by
    fresh: bool,
    sel: alloc::collections::BTreeSet<u64>,
    started: u64,
}

/// the state of the format commands
pub(crate) struct FormatState {
    pub recent: Vec<u32>,
    pub painter: Option<Painter>,
    /// Weitere Farben: the dialog, its colour, where it goes
    pub more: Shared<i32>,
    pub more_argb: u32,
    pub more_for: Target,
    /// the first selected shape's Char.Style bits, VerticalAlign, Para.HorzAlign (the toggles)
    pub style_now: u32,
    pub valign_now: Option<u32>,
    pub halign_now: Option<u32>,
    /// the ribbon saw a double click on Format übertragen
    pub double: bool,
    /// the line menu's choices to open next (Stärke, Striche, Pfeile)
    pub line_sub: Option<u32>,
}

impl Default for FormatState {
    fn default() -> FormatState {
        FormatState { recent: Vec::new(), painter: None, more: Shared::new(0), more_argb: 0xFF1F_5FA8, more_for: Target::Fill, style_now: 0, valign_now: None, halign_now: None, double: false, line_sub: None }
    }
}

fn cells_of(t: Target) -> (F, F) {
    match t {
        Target::Fill => (F::Cell("FillForegnd"), F::Cell("FillPattern")),
        Target::Line => (F::Cell("LineColor"), F::Cell("LinePattern")),
        Target::Font => (F::Char("Color"), F::Char("Color")),
        Target::Shadow => (F::Cell("ShdwForegnd"), F::Cell("ShdwPattern")),
    }
}

/// a colour (formula) for the selection's fill, line, text or shadow; a fill or line that was
/// off comes on
pub(crate) fn set_color(app: &mut Diagrammer, ui: &mut Ui, target: Target, formula: String) {
    let (c, pattern) = cells_of(target);
    let mut changes = alloc::vec![(c, formula)];
    if pattern != c && fc::num(app, pattern).is_none_or(|p| p == 0.0) {
        changes.push((pattern, String::from("THEMEGUARD(1)")));
    }
    let label = match target {
        Target::Fill => t("Füllfarbe"),
        Target::Line => t("Linienfarbe"),
        Target::Font => t("Schriftfarbe"),
        Target::Shadow => t("Schattenfarbe"),
    };
    fc::set(app, ui, label, &changes);
}

// ---------------------------------------------------------------- the ribbon's flyouts

const WEIGHTS: [(f64, &str); 9] = [(0.25, "¼ pt"), (0.5, "½ pt"), (0.75, "¾ pt"), (1.0, "1 pt"), (1.5, "1½ pt"), (2.25, "2¼ pt"), (3.0, "3 pt"), (4.5, "4½ pt"), (6.0, "6 pt")];
/// Visio's line patterns with a picture in text: (pattern, picture)
pub(crate) const DASHES: [(u32, &str); 8] = [(1, "———————"), (2, "– – – – –"), (3, "· · · · · · ·"), (4, "— · — · —"), (5, "— · · — · ·"), (9, "——  ——  ——"), (10, "- - - - - - -"), (16, "—   —   —   —")];
/// line ends offered at once: (begin, end, picture); (JD15, RS-21) 13 as Visio's, the last
/// three the begin-only forms of 4, 1 and 20
pub(crate) const ARROWS: [(u32, u32, &str); 13] = [
    (0, 0, "————"),
    (0, 13, "———→"),
    (13, 0, "←———"),
    (13, 13, "←——→"),
    (0, 4, "———→  4"),
    (4, 4, "←——→  4"),
    (0, 1, "———>  1"),
    (1, 1, "<——>  1"),
    (0, 20, "———•"),
    (20, 20, "•——•"),
    (4, 0, "←———  4"),
    (1, 0, "<———  1"),
    (20, 0, "•———"),
];

/// a flyout of the ribbon this module fills -> handled
pub(crate) fn flyout(app: &mut Diagrammer, ui: &mut Ui, pid: u32) -> bool {
    let target = match pid {
        FILL => Target::Fill,
        LINE_FORMAT => Target::Line,
        x if x == FONT_COLOR | SPLIT => Target::Font,
        _ => return false,
    };
    if target == Target::Line {
        line_choices(app, ui);
    }
    if ui.flyout_begin(pid, 0) {
        let (c, _) = cells_of(target);
        let current = fc::color(app, c);
        if let Some(f) = fc::color_menu(app, ui, ID_COLORS + pid, current) {
            set_color(app, ui, target, f);
            ui.popup_close();
        }
        ui.menu_sep();
        match target {
            Target::Fill => item(app, ui, FILL_NONE),
            Target::Line => item(app, ui, LINE_NONE),
            _ => {}
        }
        if ui.menu_item(pid + 0x100, crate::commands::label(MORE_COLORS)) & EV_ACT != 0 {
            open_more(app, ui, target);
        }
        if target == Target::Line {
            ui.menu_sep();
            for (sub, cmd) in [(ID_WEIGHTS, LINE_WEIGHT), (ID_DASHES, LINE_DASH), (ID_ARROWS, LINE_ARROWS)] {
                ui.next_icon(IC_CHEV_RIGHT);
                if ui.menu_item(sub + 0xFF, crate::commands::label(cmd)) & EV_ACT != 0 {
                    app.format.line_sub = Some(sub);
                    ui.repass();
                }
            }
            ui.menu_sep();
            item(app, ui, LINE_OPTIONS);
        }
        if target == Target::Fill {
            item(app, ui, FILL_OPTIONS);
        }
    }
    ui.flyout_end();
    true
}

fn item(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
    ui.next_flags(if crate::commands::usable(app, id) { 0 } else { ST_DISABLED });
    if ui.menu_item(id, crate::commands::label(id)) & EV_ACT != 0 {
        ui.cmd_run(id, 0);
        ui.popup_close();
    }
}

/// (JD8 rest) the commands Stärke, Striche, Pfeile: their flyout opens at the Linie button the
/// next time the Start register is drawn
pub(crate) fn open_line_choice(app: &mut Diagrammer, cmd: u32) {
    app.format.line_sub = match cmd {
        LINE_WEIGHT => Some(ID_WEIGHTS),
        LINE_DASH => Some(ID_DASHES),
        LINE_ARROWS => Some(ID_ARROWS),
        _ => None,
    };
}

// the choices of the line menu (Stärke, Striche, Pfeile): each a flyout of its own at the
// Linie button, opened after the line menu closed (a flyout inside the menu would lie under
// its other rows)
fn line_choices(app: &mut Diagrammer, ui: &mut Ui) {
    if let Some(p) = app.format.line_sub.take() {
        ui.popup_open(p);
    }
    for pid in [ID_WEIGHTS, ID_DASHES, ID_ARROWS] {
        choices(app, ui, pid);
    }
}

fn choices(app: &mut Diagrammer, ui: &mut Ui, pid: u32) {
    if ui.flyout_begin(pid, 0) {
        match pid {
            ID_WEIGHTS => {
                let cur = fc::num(app, F::Cell("LineWeight")).map(|w| w * 72.0);
                for (k, (pt, name)) in WEIGHTS.iter().enumerate() {
                    ui.next_flags(if cur.is_some_and(|c| (c - pt).abs() < 0.01) { ST_CHECKED } else { 0 });
                    if ui.menu_item(pid + 1 + k as u32, name) & EV_ACT != 0 {
                        fc::set(app, ui, t("Linienstärke"), &[(F::Cell("LineWeight"), format!("THEMEGUARD({pt}PT)"))]);
                        ui.popup_close();
                    }
                }
            }
            ID_DASHES => {
                let cur = fc::num(app, F::Cell("LinePattern")).map(|p| p as u32);
                for (k, (p, pic)) in DASHES.iter().enumerate() {
                    ui.next_flags(if cur == Some(*p) { ST_CHECKED } else { 0 });
                    if ui.menu_item(pid + 1 + k as u32, pic) & EV_ACT != 0 {
                        fc::set(app, ui, t("Strichtyp"), &[(F::Cell("LinePattern"), format!("THEMEGUARD({p})"))]);
                        ui.popup_close();
                    }
                }
            }
            _ => {
                let cur = (fc::num(app, F::Cell("BeginArrow")).map(|p| p as u32), fc::num(app, F::Cell("EndArrow")).map(|p| p as u32));
                for (k, (b, e, pic)) in ARROWS.iter().enumerate() {
                    ui.next_flags(if cur == (Some(*b), Some(*e)) { ST_CHECKED } else { 0 });
                    if ui.menu_item(pid + 1 + k as u32, pic) & EV_ACT != 0 {
                        fc::set(app, ui, t("Pfeile"), &[(F::Cell("BeginArrow"), format!("THEMEGUARD({b})")), (F::Cell("EndArrow"), format!("THEMEGUARD({e})"))]);
                        ui.popup_close();
                    }
                }
            }
        }
    }
    ui.flyout_end();
}

// ---------------------------------------------------------------- Weitere Farben

fn open_more(app: &mut Diagrammer, ui: &mut Ui, target: Target) {
    let (c, _) = cells_of(target);
    app.format.more_argb = 0xFF00_0000 | fc::color(app, c).unwrap_or(0x1F5FA8);
    app.format.more_for = target;
    app.format.more.set(1);
    ui.popup_close();
    ui.repass();
}

/// Weitere Farben (the command): for the fill
pub(crate) fn more_colors(app: &mut Diagrammer, ui: &mut Ui) {
    open_more(app, ui, Target::Fill);
}

fn more_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.format.more.clone();
    if !ui.dialog_begin(ID_MORE_DLG, t("Weitere Farben"), &open) {
        return;
    }
    let mut c = app.format.more_argb;
    ui.next_size(320, 260);
    ui.color_picker_ex(ID_MORE_DLG + 1, &mut c, CP_FIELDS);
    app.format.more_argb = c;
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        let rgb = c & 0xFF_FFFF;
        fc::used(app, rgb);
        let f = format!("THEMEGUARD(RGB({},{},{}))", (rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255);
        let target = app.format.more_for;
        set_color(app, ui, target, f);
    }
    if r != DLG_NONE {
        app.format.more.set(0);
    }
}

// ---------------------------------------------------------------- commands

/// Keine Füllung
pub(crate) fn fill_none(app: &mut Diagrammer, ui: &mut Ui) {
    fc::set(app, ui, t("Keine Füllung"), &[(F::Cell("FillPattern"), String::from("THEMEGUARD(0)"))]);
}

/// Keine Linie
pub(crate) fn line_none(app: &mut Diagrammer, ui: &mut Ui) {
    fc::set(app, ui, t("Keine Linie"), &[(F::Cell("LinePattern"), String::from("THEMEGUARD(0)"))]);
}

/// Schriftfarbe (the split button's action): the colour used last, else the design's dark
pub(crate) fn font_color(app: &mut Diagrammer, ui: &mut Ui) {
    let f = match app.format.recent.first() {
        Some(&c) => format!("THEMEGUARD(RGB({},{},{}))", (c >> 16) & 255, (c >> 8) & 255, c & 255),
        None => String::from("THEMEGUARD(THEMEVAL(\"Dark\"))"),
    };
    set_color(app, ui, Target::Font, f);
}

fn style_bits(app: &mut Diagrammer) -> u32 {
    fc::num(app, F::Char("Style")).map_or(0, |v| v.max(0.0) as u32)
}

fn toggle_style(app: &mut Diagrammer, ui: &mut Ui, bit: u32, label: &str) {
    let v = style_bits(app) ^ bit;
    fc::set(app, ui, label, &[(F::Char("Style"), format!("{v}"))]);
}

pub(crate) fn bold(app: &mut Diagrammer, ui: &mut Ui) {
    toggle_style(app, ui, 1, t("Fett"));
}
pub(crate) fn italic(app: &mut Diagrammer, ui: &mut Ui) {
    toggle_style(app, ui, 2, t("Kursiv"));
}
pub(crate) fn underline(app: &mut Diagrammer, ui: &mut Ui) {
    toggle_style(app, ui, 4, t("Unterstrichen"));
}
pub(crate) fn strike(app: &mut Diagrammer, ui: &mut Ui) {
    let on = fc::num(app, F::Char("Strikethru")).is_some_and(|v| v != 0.0);
    fc::set(app, ui, t("Durchgestrichen"), &[(F::Char("Strikethru"), String::from(if on { "FALSE" } else { "TRUE" }))]);
}

/// the toggles' state for the ribbon (the first selected shape)
pub(crate) fn style_on(app: &Diagrammer, bit: u32) -> bool {
    app.format.style_now & bit != 0
}

/// the format commands want shapes selected
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    let sel = app.opened.is_some() && !app.se.sel.is_empty();
    match id {
        FILL_NONE | LINE_NONE | MORE_COLORS | FONT_COLOR | BOLD | ITALIC | UNDERLINE | STRIKE | VALIGN_TOP | VALIGN_MIDDLE
        | VALIGN_BOTTOM | TEXT_LEFT | TEXT_CENTER | TEXT_RIGHT | TEXT_JUSTIFY | FORMAT_PAINTER | FILL | LINE_FORMAT => sel,
        _ => true,
    }
}

/// open Shape formatieren on a register
pub(crate) fn pane(app: &mut Diagrammer, tab: i32) {
    app.format_tab = tab;
    app.panes.show(crate::panes::P_FORMAT);
}

pub(crate) fn halign(app: &mut Diagrammer, ui: &mut Ui, v: u32) {
    fc::set(app, ui, t("Ausrichtung"), &[(F::Para("HorzAlign"), format!("{v}"))]);
}
pub(crate) fn valign(app: &mut Diagrammer, ui: &mut Ui, v: u32) {
    fc::set(app, ui, t("Ausrichtung"), &[(F::Cell("VerticalAlign"), format!("{v}"))]);
}

// ---------------------------------------------------------------- the font boxes

const SIZES: [u32; 16] = [6, 7, 8, 9, 10, 11, 12, 14, 16, 18, 20, 24, 28, 36, 48, 72];
const FAMILIES: [&str; 11] = ["Arial", "Calibri", "Cambria", "Consolas", "Courier New", "Georgia", "Segoe UI", "Tahoma", "Times New Roman", "Trebuchet MS", "Verdana"];

/// the fonts offered: the design's (headings, body) first, then the document's, then common ones
pub(crate) fn font_list(app: &Diagrammer) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |s: &str| {
        if !s.is_empty() && !out.iter().any(|x| x.eq_ignore_ascii_case(s)) {
            out.push(String::from(s));
        }
    };
    if let Some(o) = app.opened.as_ref() {
        if let Some((id, _)) = crate::design_gallery::current(app) {
            if let Some(k) = diagram_core::design::apply::find_part(&o.doc, id) {
                if let Some(th) = o.doc.themes.get(k).and_then(|p| diagram_core::render::ThemeInfo::parse(&p.data)) {
                    add(&th.major_font);
                    add(&th.minor_font);
                }
            }
        }
        let name = o.doc.sym("NameU");
        for f in &o.doc.faces {
            if let Some((_, v)) = f.attrs.iter().find(|(k, _)| Some(*k) == name) {
                add(o.doc.str(*v));
            }
        }
    }
    add("Inter");
    for f in FAMILIES {
        add(f);
    }
    out
}

// the list index of the design's body font (font_list puts it second, after the headings')
fn theme_body(app: &Diagrammer) -> i32 {
    match crate::design_gallery::current(app) {
        Some(_) if font_list(app).len() > 1 => {
            let l = font_list(app);
            // headings and body may be the same face: then it is the first entry
            let o = app.opened.as_ref();
            let minor = o.and_then(|o| {
                let (id, _) = crate::design_gallery::current(app)?;
                let k = diagram_core::design::apply::find_part(&o.doc, id)?;
                diagram_core::render::ThemeInfo::parse(&o.doc.themes.get(k)?.data).map(|t| t.minor_font)
            });
            minor.and_then(|m| l.iter().position(|f| f.eq_ignore_ascii_case(&m))).map_or(-1, |k| k as i32)
        }
        _ => -1,
    }
}

/// Schriftart and Schriftgrad
pub(crate) fn font_boxes(app: &mut Diagrammer, ui: &mut Ui, font_w: i32, size_w: i32, h: i32) {
    let on = !fc::selection(app).is_empty();
    let fonts = font_list(app);
    let cur = if on { fc::text(app, F::Char("Font")) } else { None };
    let mut sel = cur.as_ref().and_then(|c| fonts.iter().position(|f| f.eq_ignore_ascii_case(c))).map_or(-1, |k| k as i32);
    let names: Vec<&str> = fonts.iter().map(|s| s.as_str()).collect();
    ui.next_size(font_w, h);
    ui.next_flags(ST_COMPACT | if on { 0 } else { ST_DISABLED });
    let before = sel;
    ui.combobox(FONT, &mut sel, &names);
    if sel != before && sel >= 0 {
        let name = names[sel as usize].replace('"', "");
        if sel == theme_body(app) {
            // the design's body font: the shapes follow the design again
            fc::inherit(app, ui, t("Schriftart"), &[F::Char("Font")]);
        } else {
            fc::set(app, ui, t("Schriftart"), &[(F::Char("Font"), format!("THEMEGUARD(FONT(\"{name}\"))"))]);
        }
    }
    ui.tooltip(&crate::commands::tip(ui, FONT));
    ui.keytip(2, crate::commands::label(FONT));
    let size = if on { fc::num(app, F::Char("Size")).map(|s| s * 72.0) } else { None };
    let labels: Vec<String> = SIZES.iter().map(|s| format!("{s}")).collect();
    let labels: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
    let mut ss = size.and_then(|s| SIZES.iter().position(|x| (*x as f64 - s).abs() < 0.05)).map_or(-1, |k| k as i32);
    let before = ss;
    ui.next_size(size_w, h);
    ui.next_flags(ST_COMPACT | if on { 0 } else { ST_DISABLED });
    ui.combobox(FONT_SIZE, &mut ss, &labels);
    if ss != before && ss >= 0 {
        fc::set(app, ui, t("Schriftgrad"), &[(F::Char("Size"), format!("{}PT", SIZES[ss as usize]))]);
    }
    ui.tooltip(&crate::commands::tip(ui, FONT_SIZE));
    ui.keytip(2, crate::commands::label(FONT_SIZE));
}

// ---------------------------------------------------------------- Format übertragen

// the cells the painter carries: the quick styles and design indices first, then line, fill,
// shadow, text block, character and paragraph formats
const PAINT: [&str; 47] = [
    "QuickStyleLineColor", "QuickStyleFillColor", "QuickStyleShadowColor", "QuickStyleFontColor", "QuickStyleLineMatrix",
    "QuickStyleFillMatrix", "QuickStyleEffectsMatrix", "QuickStyleFontMatrix", "QuickStyleType", "QuickStyleVariation",
    "ColorSchemeIndex", "EffectSchemeIndex", "ConnectorSchemeIndex", "FontSchemeIndex", "ThemeIndex", "VariationColorIndex",
    "VariationStyleIndex", "EmbellishmentIndex", "LineWeight", "LineColor", "LinePattern", "Rounding", "LineCap", "LineColorTrans",
    "CompoundType", "BeginArrow", "EndArrow", "BeginArrowSize", "EndArrowSize", "FillForegnd", "FillBkgnd", "FillPattern",
    "FillForegndTrans", "FillBkgndTrans", "ShdwForegnd", "ShdwPattern", "ShdwForegndTrans", "ShapeShdwType", "ShapeShdwOffsetX",
    "ShapeShdwOffsetY", "ShapeShdwBlur", "FillGradientEnabled", "VerticalAlign", "TextBkgnd", "LeftMargin", "RightMargin", "TopMargin",
];
const PAINT_CHAR: [&str; 6] = ["Font", "Color", "Style", "Size", "Strikethru", "ColorTrans"];
const PAINT_PARA: [&str; 1] = ["HorzAlign"];

// a cell's effective formula as text: its expression, else its value written as a formula
fn formula_of(doc: &diagram_core::Document, sheet: SheetRef, a: &CellAddr) -> Option<String> {
    let r = doc.resolve(sheet, a);
    if let Formula::Expr(f) = r.formula() {
        return Some(String::from(doc.str(f)));
    }
    match r.value() {
        Value::Num(x) => Some(format!("{x}")),
        Value::Text(s) => {
            let v = doc.str(s);
            if v.eq_ignore_ascii_case("Themed") {
                Some(String::from("THEMEVAL()"))
            } else if let Some(c) = diagram_core::render::parse_color(v).filter(|_| v.starts_with('#')) {
                Some(format!("RGB({},{},{})", (c >> 16) & 255, (c >> 8) & 255, c & 255))
            } else {
                Some(format!("\"{}\"", v.replace('"', "\"\"")))
            }
        }
        Value::Absent => None,
    }
}

/// the format of a shape as (cell, formula)
pub(crate) fn format_of(doc: &diagram_core::Document, s: ShapeRef) -> Vec<(F, String)> {
    let sheet = SheetRef::Shape(s);
    let mut out = Vec::new();
    for n in PAINT {
        if let Some(c) = doc.sym(n) {
            if let Some(f) = formula_of(doc, sheet, &CellAddr::single(c)) {
                out.push((F::Cell(n), f));
            }
        }
    }
    for (sec, list, mk) in [("Character", &PAINT_CHAR[..], F::Char as fn(&'static str) -> F), ("Paragraph", &PAINT_PARA[..], F::Para)] {
        let Some(sk) = doc.sym(sec).map(diagram_core::SecKey::one) else { continue };
        let Some(row) = doc.rows(sheet, sk).first().map(|r| r.key) else { continue };
        for n in list {
            if let Some(c) = doc.sym(n) {
                if let Some(f) = formula_of(doc, sheet, &CellAddr::row(sk, row, c)) {
                    out.push((mk(n), f));
                }
            }
        }
    }
    out
}

/// Format übertragen: take the first selected shape's format; a second click soon after makes
/// it stay on
pub(crate) fn painter(app: &mut Diagrammer, ui: &mut Ui) {
    let now = plat::time_us();
    if let Some(p) = app.format.painter.as_mut() {
        if core::mem::take(&mut p.fresh) {
            // the second click of a double click (after() kept it on)
            return;
        }
        if now.wrapping_sub(p.started) < 500_000 {
            p.sticky = true;
        } else {
            app.format.painter = None;
        }
        ui.repass();
        return;
    }
    let sel = fc::selection(app);
    let (Some(&s), Some(o)) = (sel.first(), app.opened.as_ref()) else { return };
    let cells = format_of(&o.doc, s);
    app.format.painter = Some(Painter { cells, sticky: false, fresh: false, sel: app.se.sel.clone(), started: now });
    ui.repass();
}

/// every pass after the canvas: the painter gives its format to the shapes selected next;
/// Esc ends it; and the dialog Weitere Farben
pub(crate) fn after(app: &mut Diagrammer, ui: &mut Ui) {
    // the toggles' state of the first selected shape (B, I, U)
    let sel = !fc::selection(app).is_empty();
    app.format.style_now = if sel { style_bits(app) } else { 0 };
    app.format.valign_now = if sel { fc::num(app, F::Cell("VerticalAlign")).map(|v| v as u32) } else { None };
    app.format.halign_now = if sel { fc::num(app, F::Para("HorzAlign")).map(|v| v as u32) } else { None };
    more_dialog(app, ui);
    // (EV_DOUBLE comes with the second press, its EV_ACT with the release a pass later: the
    // mark stays until that click's command has passed by)
    if core::mem::take(&mut app.format.double) {
        if app.format.painter.is_none() {
            painter(app, ui);
        }
        if let Some(p) = app.format.painter.as_mut() {
            p.sticky = true;
            p.fresh = true;
        }
    }
    let Some(p) = app.format.painter.as_ref() else { return };
    if ui.key_first() == VK_ESCAPE || app.tool != POINTER {
        if ui.key_first() == VK_ESCAPE {
            ui.key_take();
        }
        app.format.painter = None;
        ui.repass();
        return;
    }
    // Esc (the canvas takes it first and clears the selection) or a click on the empty page
    // ends it, as in Visio
    if app.se.sel.is_empty() {
        app.format.painter = None;
        ui.repass();
        return;
    }
    if app.se.sel == p.sel {
        return;
    }
    let cells = p.cells.clone();
    let sticky = p.sticky;
    if let Some(p) = app.format.painter.as_mut() {
        p.fresh = false;
    }
    let shapes = fc::selection(app);
    fc::set_on(app, ui, t("Format übertragen"), &shapes, &cells);
    match app.format.painter.as_mut() {
        Some(p) if sticky => p.sel = app.se.sel.clone(),
        _ => app.format.painter = None,
    }
}
