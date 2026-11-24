// data_textdraw -- (JD18, IM-08, AS-20) Daten › Zeichnung aus Text: a CSV or text file with Shape
// and Link records (Visio's text import; the format and its decisions: diagram_io::textdraw)
// becomes shapes and connectors -- the file dialog, then a wizard of two steps, Vorschau (the
// records as a table, the problems with their line numbers, the separator and the unit of the
// numbers) and Master-Zuordnung (each master name of the file to a master of the drawing or of
// the open stencils; the page), and the drawing in one undo step
//
//   TextDrawState         the file dialog, the wizard, the text read, the plan, the choices
//   open(app, ui)         DATA_TEXT_DRAWING: the file dialog (Textdateien: csv, txt)
//   dialog(app, ui)       every pass: the file dialog, then the wizard; Fertigstellen builds
//
// The file is decoded by sheet_io::csv::decode (a BOM, UTF-16, UTF-8, else Windows-1252); its
// records are read by diagram_io::textdraw's own tokenizer (the format is not a table).
//
// What the app decides (FEATURES IM-08, AS-20; UG chapter 21 names neither):
// * The separator: detected from the records (textdraw::detect_sep), else the extension's (.txt a
//   tab, else a comma); the wizard offers Komma, Tabulator and Semikolon instead.
// * Numbers without a unit are millimetres on a metric page, inches on a US one (the page's
//   unit, as the rulers show it); the wizard offers mm, cm, Zoll and Punkt.
// * Masters: the candidates are the masters of the drawing, then those of the open stencils of
//   the Shapes panel (the panel's order), 2-D ones for the names of Shape records and 1-D ones
//   for those of Link records.  Preselected is the first candidate whose NameU or name in one of
//   the five languages equals the file's name (case and accents ignored, instance::fold), else
//   the default: Einfaches Rechteck (a rectangle of the drawing's default styles) or Dynamischer
//   Verbinder.  A stencil master is taken into the drawing when the drawing is built.
// * The page: a new page (after the page shown, with its settings, as Neue Seite makes it) when
//   the page shown has shapes, else the page shown -- the wizard lets the user choose.  Visio's
//   text import made a drawing of its own; a new page leaves the user's drawing as it was, and
//   one undo takes the page back with its content.
// * The sizes the file does not give (a rectangle 35 x 20 mm, US 1.5 x 0.75 in), the gaps of
//   the layout (10 mm across, 15 mm down; US 0.4 and 0.6 in) and the margin at the page's top
//   left corner (10 mm, US 0.5 in) are lengths on paper: on a page with a drawing scale they
//   grow with it.
// * Afterwards the page is fitted into the window (an automatically sized page has grown around
//   the drawing); screen readers and scripts hear "{n} Shapes und {m} Verbinder erstellt".
// Test aid: --data-dir DIR is where the file dialog starts (scripts; relative to the working
// folder).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::instance::{self, fold, MasterTexts};
use diagram_core::{glue, pagesetup, Document, MasterRef, Owner, PageRef, ShapeRef, SheetRef};
use diagram_io::textdraw::{self, Build, Issue, Limits, NumField, Options, Plan, Problem};
use sheet_io::csv::Encoding;
use ui::i18n::{t, tf};
use ui::*;

use crate::commands::{self, DATA_TEXT_DRAWING};
use crate::{edit_doc, tc, Diagrammer};

const ID_FILE: u32 = 0x8B40;
const ID: u32 = 0x7B60_0000;
const ID_GRID: u32 = ID + 0x1000;
const ID_MAP: u32 = ID + 0x1_0000;
/// the wizard's content width (DIP; a content dialog is at most 548 wide)
const W: i32 = 500;

/// where a candidate master is
#[derive(Clone, Copy, Debug, PartialEq)]
enum Src {
    /// the default: a plain rectangle, the dynamic connector
    Plain,
    /// a master of the drawing
    Doc(MasterRef),
    /// a master of an open stencil (its panel key)
    Stencil(u32, MasterRef),
}

/// a master the wizard offers
struct Cand {
    label: String,
    src: Src,
    /// NameU and the names in the five languages, folded (for the preselection)
    names: Vec<String>,
}

/// the wizard's state
pub(crate) struct TextDrawState {
    file_dlg: Shared<i32>,
    path: String,
    wizard: Shared<i32>,
    /// 0 Vorschau, 1 Master-Zuordnung
    step: u8,
    /// the file's name as shown
    name: String,
    /// the file is a .txt (the separator's fallback is a tab)
    txt: bool,
    text: String,
    encoding: &'static str,
    /// the separator detected (else the extension's)
    auto_sep: char,
    /// 0 detected, 1 comma, 2 tab, 3 semicolon
    sep_pick: i32,
    /// 0 mm, 1 cm, 2 in, 3 pt
    unit_pick: i32,
    plan: Plan,
    /// the preview's tab: Shapes, Verbinder, Probleme
    tab: i32,
    grid: GridState,
    /// the candidates for shape names (2-D) and link names (1-D); the first is the default
    cands2: Vec<Cand>,
    cands1: Vec<Cand>,
    /// the candidate chosen per Plan::masters entry
    pick: Vec<i32>,
    /// the user's choices by (folded name, link), kept when the plan is read again
    chosen: BTreeMap<(String, bool), i32>,
    /// 0 the page shown, 1 a new page
    new_page: i32,
}

impl Default for TextDrawState {
    fn default() -> TextDrawState {
        TextDrawState {
            file_dlg: Shared::new(0),
            path: String::new(),
            wizard: Shared::new(0),
            step: 0,
            name: String::new(),
            txt: false,
            text: String::new(),
            encoding: "UTF-8",
            auto_sep: ',',
            sep_pick: 0,
            unit_pick: 0,
            plan: Plan::default(),
            tab: 0,
            grid: GridState::default(),
            cands2: Vec::new(),
            cands1: Vec::new(),
            pick: Vec::new(),
            chosen: BTreeMap::new(),
            new_page: 0,
        }
    }
}

/// Daten › Zeichnung aus Text: the file dialog
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui) {
    let st = &mut app.textdraw;
    if st.path.is_empty() {
        if let Some(d) = plat::arg_value("--data-dir") {
            let d = if d.starts_with('/') || d.get(1..2) == Some(":") { d } else { format!("{}/{}", plat::cwd(), d) };
            st.path = format!("{}/", d.trim_end_matches(['/', '\\']));
        }
    }
    st.file_dlg.set(1);
    ui.repass();
}

/// every pass: the file dialog, then the wizard
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let dlg = app.textdraw.file_dlg.clone();
    if dlg.get() != 0 {
        let exts = [t("csv;txt=Textdateien"), t("=Alle Dateien")];
        if ui.filedlg(ID_FILE, FD_OPEN, &dlg, &mut app.textdraw.path, 4096, &exts) & EV_ACT != 0 {
            let path = ui.filedlg_paths().into_iter().next().unwrap_or_else(|| app.textdraw.path.clone());
            load(app, ui, &path);
        }
    }
    wizard(app, ui);
}

// ---------------------------------------------------------------- reading

/// a text file's bytes as text (sheet_io::csv::decode: BOM, UTF-16, UTF-8, else Windows-1252)
/// -> (text, the encoding's name)
fn decode(bytes: &[u8]) -> (String, &'static str) {
    let (text, enc) = sheet_io::csv::decode(bytes);
    let name = match enc {
        Encoding::Utf8 => "UTF-8",
        Encoding::Utf8Bom => "UTF-8 (BOM)",
        Encoding::Utf16Le => "UTF-16 LE",
        Encoding::Utf16Be => "UTF-16 BE",
        Encoding::Windows1252 => "Windows-1252",
    };
    (text, name)
}

// a chosen file: read, decoded, parsed, the wizard opened
fn load(app: &mut Diagrammer, ui: &mut Ui, path: &str) {
    let name = String::from(plat::display_name(path));
    let title = tf("„{}“ kann nicht gelesen werden.", &[&name]);
    let Some(bytes) = plat::read_file(path) else {
        app.error = Some((title, String::from(t("Die Datei lässt sich nicht lesen."))));
        ui.repass();
        return;
    };
    if bytes.len() > Limits::DEFAULT.max_bytes {
        app.error = Some((title, String::from(t("Die Datei ist zu groß (höchstens 16 MB)."))));
        ui.repass();
        return;
    }
    let (text, encoding) = decode(&bytes);
    let metric = crate::edit_canvas::metric(app);
    let page_used = crate::pages::shown(app).zip(app.opened.as_ref()).is_some_and(|(p, o)| pagesetup::content(&o.doc, p).is_some());
    let st = &mut app.textdraw;
    st.txt = name.to_ascii_lowercase().ends_with(".txt");
    st.auto_sep = textdraw::detect_sep(&text, if st.txt { '\t' } else { ',' }).0;
    st.name = name;
    st.text = text;
    st.encoding = encoding;
    st.sep_pick = 0;
    st.unit_pick = if metric { 0 } else { 2 };
    st.step = 0;
    st.tab = 0;
    st.grid = GridState::default();
    st.chosen.clear();
    st.new_page = page_used as i32;
    candidates(app);
    reparse(&mut app.textdraw);
    app.textdraw.wizard.set(1);
    ui.repass();
}

// drawing inches per number without a unit
fn per_number(pick: i32) -> f64 {
    match pick {
        1 => 1.0 / 2.54,
        2 => 1.0,
        3 => 1.0 / 72.0,
        _ => 1.0 / 25.4,
    }
}

// the plan read again with the wizard's choices; the masters preselected (or as chosen)
fn reparse(st: &mut TextDrawState) {
    let sep = match st.sep_pick {
        1 => Some(','),
        2 => Some('\t'),
        3 => Some(';'),
        _ => None,
    };
    let opts = Options { sep, fallback_sep: if st.txt { '\t' } else { ',' }, per_number: per_number(st.unit_pick) };
    st.plan = textdraw::parse(&st.text, &opts, &Limits::DEFAULT);
    st.pick = st
        .plan
        .masters
        .iter()
        .map(|u| {
            let key = (fold(u.name.trim()), u.link);
            if let Some(&k) = st.chosen.get(&key) {
                return k;
            }
            let list = if u.link { &st.cands1 } else { &st.cands2 };
            let f = &key.0;
            if f.is_empty() {
                return 0;
            }
            list.iter().position(|c| c.names.iter().any(|n| n == f)).map_or(0, |k| k as i32)
        })
        .collect();
    st.grid = GridState::default();
}

// the masters of the drawing and of the open stencils, 2-D and 1-D apart
fn candidates(app: &mut Diagrammer) {
    let lang = crate::stencils::lang();
    let mut two = alloc::vec![Cand { label: String::from(t("Einfaches Rechteck")), src: Src::Plain, names: Vec::new() }];
    let mut one = alloc::vec![Cand { label: String::from(t("Dynamischer Verbinder")), src: Src::Plain, names: Vec::new() }];
    let mut add = |doc: &Document, m: MasterRef, label: String, src: Src| {
        let texts = MasterTexts::of(doc, m);
        let names: Vec<String> = core::iter::once(&texts.name_u).chain(texts.names.iter()).map(|n| fold(n.trim())).filter(|n| !n.is_empty()).collect();
        let c = Cand { label, src, names };
        if one_d(doc, m) {
            one.push(c);
        } else {
            two.push(c);
        }
    };
    if let Some(o) = app.opened.as_ref() {
        for m in o.doc.masters() {
            let name = String::from(MasterTexts::of(&o.doc, m).name(lang));
            add(&o.doc, m, tf("Zeichnung: {}", &[&name]), Src::Doc(m));
        }
    }
    let st = &app.stencils;
    for &key in &st.open {
        let Some(l) = st.get(key) else { continue };
        let title = l.title(lang);
        for e in &l.entries {
            add(&l.doc, e.master, format!("{}: {}", title, e.texts.name(lang)), Src::Stencil(key, e.master));
        }
    }
    app.textdraw.cands2 = two;
    app.textdraw.cands1 = one;
}

// whether a master's shape is 1-D
fn one_d(doc: &Document, m: MasterRef) -> bool {
    doc.master(m).and_then(|x| x.master_shape()).is_some_and(|slot| glue::is_one_d(doc, ShapeRef { owner: Owner::Master(m), slot }))
}

// ---------------------------------------------------------------- the wizard

// a length in the unit of the numbers (two decimals at most)
fn len_text(v: Option<f64>, pick: i32) -> String {
    let Some(v) = v else { return String::new() };
    let s = format!("{:.2}", v / per_number(pick));
    let s = s.trim_end_matches('0').trim_end_matches('.');
    String::from(if s == "-0" { "0" } else { s })
}

// a text in one line for the table
fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

fn field_name(f: NumField) -> &'static str {
    match f {
        NumField::X => "X",
        NumField::Y => "Y",
        NumField::Width => t("Breite"),
        NumField::Height => t("Höhe"),
    }
}

/// a problem in words
fn message(p: &Problem) -> String {
    let s = |x: &str| String::from(x);
    match &p.issue {
        Issue::TooBig => s(t("Die Datei ist zu groß (höchstens 16 MB).")),
        Issue::TooManyRecords => s(t("Zu viele Datensätze: der Rest wird nicht gelesen.")),
        Issue::TooManyProblems => s(t("Weitere Probleme werden nicht aufgeführt.")),
        // the record types are the file's words, never translated
        Issue::UnknownRecord(k) => tf("Unbekannte Datensatzart „{}“ (erwartet: {} oder {}).", &[k, "Shape", "Link"]),
        Issue::UnclosedQuote => s(t("Ein Anführungszeichen wird nicht geschlossen.")),
        Issue::TooManyFields => s(t("Zu viele Felder: die übrigen werden übergangen.")),
        Issue::FieldTooLong => s(t("Ein Feld ist zu lang und wird gekürzt.")),
        Issue::IdTooLong => s(t("Die ID ist zu lang.")),
        Issue::NoId => s(t("Shape ohne ID: es wird gezeichnet, aber kein Link erreicht es.")),
        Issue::DuplicateId(id) => tf("Die ID „{}“ hat schon ein anderes Shape.", &[id]),
        Issue::DuplicateLinkId(id) => tf("Die ID „{}“ hat schon ein anderer Link.", &[id]),
        Issue::NoEnd => s(t("Dem Link fehlt „Von“ oder „Nach“.")),
        Issue::UnknownEnd(id) => tf("Kein Shape hat die ID „{}“.", &[id]),
        Issue::SelfLink(id) => tf("Der Link führt von „{}“ zu sich selbst.", &[id]),
        Issue::BadNumber(f, v) => tf("{}: „{}“ ist keine Zahl.", &[field_name(*f), v]),
        Issue::OutOfRange(f, v) => tf("{}: „{}“ liegt außerhalb des erlaubten Bereichs.", &[field_name(*f), v]),
        Issue::HalfPosition => s(t("X ohne Y oder Y ohne X: das Shape wird automatisch angeordnet.")),
        Issue::TooManyProps => s(t("Zu viele Shape-Daten: die übrigen werden übergangen.")),
    }
}

fn sep_name(c: char) -> &'static str {
    match c {
        '\t' => t("Tabulator"),
        ';' => t("Semikolon"),
        _ => t("Komma"),
    }
}

// the wizard (every pass)
fn wizard(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.textdraw.wizard.clone();
    if !ui.dialog_begin(ID, commands::label(DATA_TEXT_DRAWING), &open) {
        return;
    }
    let step = app.textdraw.step;
    ui.next_size(W, 0);
    ui.box_begin(ID + 1, BOX_V, 8);
    if step == 0 {
        preview(app, ui);
    } else {
        mapping(app, ui);
    }
    ui.box_end();
    let empty = app.textdraw.plan.shapes.is_empty();
    let r = if step == 0 { ui.dialog_buttons(Some(t("Weiter")), None, Some(t("Abbrechen"))) } else { ui.dialog_buttons(Some(t("Fertigstellen")), Some(t("Zurück")), Some(t("Abbrechen"))) };
    ui.dialog_end();
    // (any result of dialog_buttons closes the dialog: Weiter and Zurück open it again)
    match (r, step) {
        (DLG_PRIMARY, 0) => {
            if !empty {
                app.textdraw.step = 1;
            }
            app.textdraw.wizard.set(1);
            ui.repass();
        }
        (DLG_PRIMARY, _) => {
            app.textdraw.wizard.set(0);
            finish(app, ui);
            release(&mut app.textdraw);
            ui.repass();
        }
        (DLG_SECONDARY, _) => {
            app.textdraw.step = 0;
            app.textdraw.wizard.set(1);
            ui.repass();
        }
        (DLG_CLOSE, _) => {
            app.textdraw.wizard.set(0);
            release(&mut app.textdraw);
            ui.repass();
        }
        _ => {}
    }
}

// the wizard closed: the text and the plan are not kept (a file may be large)
fn release(st: &mut TextDrawState) {
    st.text = String::new();
    st.plan = Plan::default();
    st.pick.clear();
    st.cands1.clear();
    st.cands2.clear();
}

// step 1: the records and the problems
fn preview(app: &mut Diagrammer, ui: &mut Ui) {
    let st = &mut app.textdraw;
    ui.next_size(W, 0);
    ui.text(TS_CAPTION | TX_SECONDARY, &tf("Schritt {} von {}: {}", &["1", "2", t("Vorschau")]));
    ui.next_size(W, 0);
    ui.text(TS_BODY | TX_ELLIPSIS, &tf("Datei: {} ({})", &[&st.name, st.encoding]));
    // the separator and the unit, side by side
    let auto = tf("Automatisch ({})", &[sep_name(st.auto_sep)]);
    let seps = [auto.as_str(), t("Komma"), t("Tabulator"), t("Semikolon")];
    let (sep_was, unit_was) = (st.sep_pick, st.unit_pick);
    ui.box_begin(0, BOX_H, 16);
    ui.box_begin(0, BOX_V, 4);
    ui.text(TS_BODY, t("Trennzeichen"));
    ui.next_size(220, 0);
    ui.combobox(ID + 2, &mut st.sep_pick, &seps);
    ui.box_end();
    ui.box_begin(0, BOX_V, 4);
    ui.text(TS_BODY, t("Einheit der Zahlen"));
    ui.next_size(220, 0);
    ui.combobox(ID + 3, &mut st.unit_pick, &[t("Millimeter"), t("Zentimeter"), t("Zoll"), t("Punkt")]);
    ui.box_end();
    ui.box_end();
    if st.sep_pick != sep_was || st.unit_pick != unit_was {
        reparse(st);
        ui.repass();
    }
    let p = &st.plan;
    let (ns, nl, ne, nw) = (p.shapes.len(), p.links.len(), p.errors(), p.warnings());
    ui.next_size(W, 0);
    ui.text(TS_BODY_STRONG, &tf("{} Shapes, {} Verbinder, {} Fehler, {} Warnungen", &[&format!("{ns}"), &format!("{nl}"), &format!("{ne}"), &format!("{nw}")]));
    if ns == 0 {
        ui.next_size(W, 0);
        ui.text(TS_BODY | TX_WRAP, t("Die Datei enthält keine Shapes: es gibt nichts zu zeichnen."));
    }
    let tabs = [tf("Shapes ({})", &[&format!("{ns}")]), tf("Verbinder ({})", &[&format!("{nl}")]), tf("Probleme ({})", &[&format!("{}", p.problems.len())])];
    let names: Vec<&str> = tabs.iter().map(|s| s.as_str()).collect();
    let tab_was = st.tab;
    // (its items are ID + 0x21 ..)
    ui.tabs(ID + 0x20, &mut st.tab, &names);
    if st.tab != tab_was {
        st.grid = GridState::default();
        ui.repass();
    }
    ui.next_size(W, 220);
    let per = st.unit_pick;
    match st.tab {
        0 => {
            let cols = [
                GridCol::new(t("Zeile"), 44).min(40),
                GridCol::new(t("ID"), 72),
                GridCol::new(t("Master-Shape"), 92),
                GridCol::new(t("Text"), 110).flags(GC_FILL),
                GridCol::new("X, Y", 70),
                GridCol::new(t("Größe"), 70),
            ];
            let keys: Vec<u64> = (0..p.shapes.len() as u64).collect();
            let (a, b) = ui.grid_begin(ID_GRID, &mut st.grid, &cols, &keys);
            for i in a..b {
                let s = &p.shapes[i as usize];
                ui.grid_row(&mut st.grid, i);
                let g = &st.grid;
                ui.grid_text(g, 0, &format!("{}", s.line));
                ui.grid_text(g, 1, &s.id);
                ui.grid_text(g, 2, &p.masters[s.master].name);
                ui.grid_text(g, 3, &one_line(&s.text));
                let pos = s.pos.map_or(String::new(), |q| format!("{}; {}", len_text(Some(q[0]), per), len_text(Some(q[1]), per)));
                ui.grid_text(g, 4, &pos);
                let size = if s.size.iter().any(Option::is_some) { format!("{} × {}", len_text(s.size[0], per), len_text(s.size[1], per)) } else { String::new() };
                ui.grid_text(g, 5, &size);
            }
            ui.grid_end(&mut st.grid);
            // the shape data of the current row (the first one until a row is chosen)
            let row = st.grid.cur.map_or(0, |k| k as usize);
            if let Some(s) = p.shapes.get(row) {
                let data: Vec<String> = s.props.iter().map(|x| format!("{}={}", x.label, x.value)).collect();
                let data = if data.is_empty() { String::from("–") } else { data.join("; ") };
                ui.next_size(W, 0);
                ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &tf("Shape-Daten in Zeile {}: {}", &[&format!("{}", s.line), &data]));
            }
        }
        1 => {
            let cols = [GridCol::new(t("Zeile"), 48).min(40), GridCol::new(t("ID"), 64), GridCol::new(t("Master-Shape"), 100), GridCol::new(t("Text"), 96).flags(GC_FILL), GridCol::new(t("Von"), 88), GridCol::new(tc("Nach|Linkziel"), 88)];
            let keys: Vec<u64> = (0..p.links.len() as u64).collect();
            let (a, b) = ui.grid_begin(ID_GRID, &mut st.grid, &cols, &keys);
            for i in a..b {
                let l = &p.links[i as usize];
                ui.grid_row(&mut st.grid, i);
                let g = &st.grid;
                ui.grid_text(g, 0, &format!("{}", l.line));
                ui.grid_text(g, 1, &l.id);
                ui.grid_text(g, 2, &p.masters[l.master].name);
                ui.grid_text(g, 3, &one_line(&l.text));
                ui.grid_text(g, 4, &p.shapes[l.from].id);
                ui.grid_text(g, 5, &p.shapes[l.to].id);
            }
            ui.grid_end(&mut st.grid);
        }
        _ => {
            let cols = [GridCol::new(t("Zeile"), 48).min(40), GridCol::new(t("Typ"), 80), GridCol::new(t("Meldung"), 360).flags(GC_FILL)];
            let keys: Vec<u64> = (0..p.problems.len() as u64).collect();
            let (a, b) = ui.grid_begin(ID_GRID, &mut st.grid, &cols, &keys);
            for i in a..b {
                let x = &p.problems[i as usize];
                ui.grid_row(&mut st.grid, i);
                let g = &st.grid;
                ui.grid_text(g, 0, &if x.line == 0 { String::new() } else { format!("{}", x.line) });
                ui.grid_text(g, 1, if x.skipped { t("Fehler") } else { t("Warnung") });
                ui.grid_text(g, 2, &message(x));
            }
            ui.grid_end(&mut st.grid);
        }
    }
}

// step 2: a master for each master name of the file, the page
fn mapping(app: &mut Diagrammer, ui: &mut Ui) {
    let st = &mut app.textdraw;
    ui.next_size(W, 0);
    ui.text(TS_CAPTION | TX_SECONDARY, &tf("Schritt {} von {}: {}", &["2", "2", t("Master-Zuordnung")]));
    ui.next_size(W, 0);
    ui.text(TS_BODY | TX_WRAP, t("Wählen Sie für jeden Master-Namen der Datei ein Master-Shape der Zeichnung oder einer geöffneten Schablone."));
    let labels2: Vec<&str> = st.cands2.iter().map(|c| c.label.as_str()).collect();
    let labels1: Vec<&str> = st.cands1.iter().map(|c| c.label.as_str()).collect();
    ui.next_size(W, 260);
    ui.scroll_begin(ID + 5, SC_V | BOX_V);
    for (k, u) in st.plan.masters.iter().enumerate() {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        let name = if u.name.is_empty() { String::from(t("(kein Master-Shape)")) } else { format!("„{}“", u.name) };
        ui.box_begin(0, BOX_V, 0);
        ui.next_size(220, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &name);
        let count = format!("{}", u.count);
        ui.next_size(220, 0);
        ui.text(TS_CAPTION | TX_SECONDARY, &if u.link { tf("für {} Verbinder", &[&count]) } else { tf("für {} Shapes", &[&count]) });
        ui.box_end();
        if let Some(pick) = st.pick.get_mut(k) {
            let was = *pick;
            ui.next_size(250, 0);
            ui.combobox(ID_MAP + k as u32, pick, if u.link { &labels1 } else { &labels2 });
            if *pick != was {
                st.chosen.insert((fold(u.name.trim()), u.link), *pick);
                ui.repass();
            }
        }
        ui.box_end();
    }
    ui.scroll_end();
    ui.separator();
    ui.text(TS_BODY_STRONG, t("Zeichnen auf"));
    ui.radio(ID + 6, t("der angezeigten Seite"), &mut st.new_page, 0);
    ui.radio(ID + 7, t("einer neuen Seite"), &mut st.new_page, 1);
    ui.next_size(W, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Shapes ohne X und Y werden in Stufen von oben nach unten angeordnet, Links werden zu dynamischen Verbindern."));
}

// ---------------------------------------------------------------- building

// the drawing in one undo step (see the file header)
fn finish(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(cur) = crate::pages::shown(app) else { return };
    let metric = crate::edit_canvas::metric(app);
    let lang = crate::stencils::lang();
    let st = &app.textdraw;
    let srcs: Vec<Src> = st
        .plan
        .masters
        .iter()
        .enumerate()
        .map(|(k, u)| {
            let list = if u.link { &st.cands1 } else { &st.cands2 };
            st.pick.get(k).and_then(|&p| list.get(p as usize)).map_or(Src::Plain, |c| c.src)
        })
        .collect();
    let new_page = st.new_page == 1;
    let plan = core::mem::take(&mut app.textdraw.plan);
    let stencils = &app.stencils;
    let Some(o) = app.opened.as_mut() else {
        app.textdraw.plan = plan;
        return;
    };
    // paper lengths in drawing units (a page with a drawing scale)
    let scale = pagesetup::scale(&o.doc, cur);
    let per = if scale.is_finite() && scale > 0.0 { 1.0 / scale } else { 1.0 };
    let mm = 1.0 / 25.4;
    let opts = if metric {
        Build { lang, default_size: [35.0 * mm * per, 20.0 * mm * per], gap: [10.0 * mm * per, 15.0 * mm * per], margin: 10.0 * mm * per }
    } else {
        Build { lang, default_size: [1.5 * per, 0.75 * per], gap: [0.4 * per, 0.6 * per], margin: 0.5 * per }
    };
    // a new page after the page shown (after the last foreground page when a background page is
    // shown), as Neue Seite makes it
    let doc = &o.doc;
    let background = |p: PageRef| doc.page(p).is_some_and(|pg| pg.header.background);
    let pos = if background(cur) {
        doc.pages().enumerate().filter(|(_, p)| !background(*p)).map(|(i, _)| i + 1).last().unwrap_or(0)
    } else {
        doc.page_position(cur).map_or(doc.page_count(), |i| i + 1)
    };
    let (name, name_u) = crate::pages::free_names(doc, false);
    let r = edit_doc::edit(o, commands::label(DATA_TEXT_DRAWING), true, |tx, eng| {
        let page = if new_page {
            let p = tx.add_page_like(cur, &name, &name_u, false, Some(pos))?;
            eng.recalc(tx, &[SheetRef::Page(p)])?;
            p
        } else {
            cur
        };
        let mut masters: Vec<Option<MasterRef>> = Vec::with_capacity(srcs.len());
        for s in &srcs {
            masters.push(match *s {
                Src::Plain => None,
                Src::Doc(m) => Some(m),
                Src::Stencil(key, m) => match stencils.get(key) {
                    Some(l) => Some(instance::import_master(tx, &l.doc, m)?),
                    None => None,
                },
            });
        }
        textdraw::apply(tx, eng, page, &plan, &masters, &opts)
    });
    app.textdraw.plan = plan;
    match r {
        Ok(built) => {
            if new_page {
                crate::pages::sync(app);
            }
            app.se.select(&[]);
            app.canvas.request_fit();
            app.fit_again = true;
            let (ns, nc) = (format!("{}", built.shapes.len()), format!("{}", built.connectors.len()));
            ui.announce(&tf("{} Shapes und {} Verbinder erstellt", &[&ns, &nc]), LIVE_POLITE);
        }
        Err(e) => {
            app.error = Some((String::from(t("Die Zeichnung lässt sich nicht erstellen.")), format!("{e:?}")));
        }
    }
}
