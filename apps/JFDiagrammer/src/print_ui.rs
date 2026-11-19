// print_ui -- Datei › Drucken and the dialog Seite einrichten (JD12: UI-09, PR-01 .. PR-09)
//
//   PrintState                 the print setup of the open drawing, the range, the copies, the
//                              preview's sheet and picture, the dialog's working values
//   page(app, ui)              the backstage page: Drucken with Exemplare, the printer (the
//                              system's dialog chooses it), the range (all pages, the current
//                              one, from .. to), background pages, paper, orientation, zoom,
//                              colour or grey, Seite einrichten; on the right the preview of a
//                              sheet with the tiles of its page (PR-01, PR-03)
//   open_setup(app, ui, tab)   Seite einrichten on one of its registers (Entwurf › Seite
//                              einrichten, Layout und Routing)
//   dialogs(app, ui)           the dialog, every pass
//
// Seite einrichten: Drucken einrichten (paper, orientation, zoom or n x m sheets, margins,
// centring: the print setup, PR-05), Seitengröße (as the printer's paper, a predefined size or
// an own one, orientation, PR-06), Zeichnungsmaßstab (none, predefined, own; the page's size in
// drawing units, PR-07), Seiteneigenschaften (type, name, background page, measuring unit, the
// shadow direction shown, PR-08), Layout und Routing (the page's routing cells, PR-09 and
// PG-11; layout_ui.rs since JD10, its own undo step).  OK writes what changed of the page registers into
// the page as one undo step of the editing layer (edit_doc::edit, JD7): Strg+Z takes it back,
// Speichern writes it.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::render::Options;
use diagram_core::{CellAddr, PageRef, SheetRef, Unit};
use diagram_view::PageView;
use gfx::Surface;
use ui::i18n::{t, tf};
use ui::*;

use crate::icons::*;
use crate::print_layout::{self, Fit, Job, PrintSetup, Tiling, PAPERS};
use crate::Diagrammer;

const ID: u32 = 0x6400;
const ID_DLG: u32 = 0x6500;

/// the print page's and the dialog's state
pub(crate) struct PrintState {
    pub setup: PrintSetup,
    /// the drawing the setup was read from (its serial)
    serial: u32,
    /// 0 every page, 1 the current one, 2 from .. to
    pub range: i32,
    pub from: i32,
    pub to: i32,
    pub copies: i32,
    /// the preview's sheet (over the job's sheets, from 0)
    pub sheet: i32,
    /// views of the pages for the preview: (drawing and edit version, page, background, view)
    views: Vec<(u64, PageRef, bool, PageView)>,
    /// the preview's picture and what it shows
    preview: Option<(String, Rc<Surface>)>,
    ver: u32,
    /// the last job's result: severity (0 information, 3 error), message
    pub status: Option<(u32, String)>,
    /// Seite einrichten
    pub dlg: Shared<i32>,
    pub tab: i32,
    edit: Option<PageEdit>,
}

impl PrintState {
    pub(crate) fn new() -> PrintState {
        PrintState {
            setup: PrintSetup::default(),
            serial: 0,
            range: 0,
            from: 1,
            to: 1,
            copies: 1,
            sheet: 0,
            views: Vec::new(),
            preview: None,
            ver: 0,
            status: None,
            dlg: Shared::new(0),
            tab: 0,
            edit: None,
        }
    }
}

/// the page shown (a tab of the open drawing)
fn current_page(app: &Diagrammer) -> Option<PageRef> {
    let o = app.opened.as_ref()?;
    o.tabs.get(app.page_index()).or_else(|| o.tabs.first()).map(|t| t.page)
}

/// the setup belongs to the open drawing: read anew when another one opened
fn sync(app: &mut Diagrammer) {
    let Some(o) = app.opened.as_ref() else { return };
    if app.print.serial != o.serial {
        app.print.serial = o.serial;
        let page = o.tabs.first().map(|t| t.page);
        app.print.setup = page.map_or_else(PrintSetup::default, |p| PrintSetup::from_page(&o.doc, p));
        app.print.views.clear();
        app.print.preview = None;
        app.print.sheet = 0;
        app.print.status = None;
        let n = diagram_view::export::foreground_pages(&o.doc).len().max(1) as i32;
        app.print.from = 1;
        app.print.to = n;
    }
}

/// the pages the job prints
fn job_pages(app: &Diagrammer) -> Vec<PageRef> {
    let Some(o) = app.opened.as_ref() else { return Vec::new() };
    let sel = match app.print.range {
        1 => match current_page(app) {
            Some(p) => diagram_view::export::Pages::Current(p),
            None => return Vec::new(),
        },
        2 => diagram_view::export::Pages::Range { from: app.print.from.max(1) as u32, to: app.print.to.max(1) as u32 },
        _ => diagram_view::export::Pages::All,
    };
    diagram_view::export::pages(&o.doc, &sel)
}

/// the drawing and its edits (JD7): a preview built before an edit is stale after it
fn version(o: &crate::viewer::Opened) -> u64 {
    (o.serial as u64) << 32 | o.ed.rev as u64
}

/// the view of a page for the preview (kept while the drawing and the background flag stay)
fn view_index(app: &mut Diagrammer, page: PageRef) -> Option<usize> {
    let o = app.opened.as_ref()?;
    let bg = app.print.setup.background;
    let ver = version(o);
    if let Some(k) = app.print.views.iter().position(|v| v.0 == ver && v.1 == page && v.2 == bg) {
        return Some(k);
    }
    app.print.views.retain(|v| v.0 == ver);
    if app.print.views.len() > 8 {
        app.print.views.remove(0);
    }
    let view = PageView::new(&o.doc, page, &mut app.fonts, &Options { background: bg, hidden_layers: false });
    app.print.views.push((ver, page, bg, view));
    Some(app.print.views.len() - 1)
}

/// the tiling of a page on the setup's paper (no printer edges: the preview's)
fn page_tiling(app: &mut Diagrammer, page: PageRef) -> Option<(usize, Tiling)> {
    let k = view_index(app, page)?;
    let d = app.print.views[k].3.drawing();
    let t = print_layout::tiling((d.width, d.height), &app.print.setup, app.print.setup.paper_size(), [0.0; 4]);
    Some((k, t))
}

// ------------------------------------------------------------------ Datei › Drucken

pub(crate) fn page(app: &mut Diagrammer, ui: &mut Ui) {
    sync(app);
    ui.text(TS_TITLE, t("Drucken"));
    ui.spacer(12);
    let has = app.opened.is_some();
    ui.box_begin(ID, BOX_H, 32);
    // ---- the settings
    ui.next_size(300, 0);
    ui.box_begin(0, BOX_V, 8);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
    ui.next_size(96, 66);
    let go = ui.ribbon_button(ID + 1, t("Drucken"), JD_PRINT_L, ST_LARGE | if has { 0 } else { ST_DISABLED }) & EV_ACT != 0;
    ui.tooltip(t("Die gewählten Seiten drucken (Strg+P)"));
    ui.box_begin(0, BOX_V, 4);
    ui.text(TS_BODY, t("Exemplare"));
    ui.next_size(90, 0);
    ui.number_box(ID + 2, &mut app.print.copies, &Range::new(1, 999, 1, 0));
    ui.box_end();
    ui.box_end();
    ui.text(TS_BODY_STRONG, t("Drucker"));
    let mut printer = 0;
    ui.next_size(-1, 0);
    ui.combobox(ID + 3, &mut printer, &[t("Systemdrucker")]);
    ui.next_size(300, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Drucker, Papierfach und Seitenbereich wählen Sie im Druckdialog des Systems."));
    ui.spacer(4);
    ui.text(TS_BODY_STRONG, t("Einstellungen"));
    let old = app.print.setup.clone();
    ui.next_size(-1, 0);
    ui.combobox(ID + 4, &mut app.print.range, &[t("Alle Seiten drucken"), t("Aktuelle Seite drucken"), t("Seitenbereich drucken")]);
    if app.print.range == 2 {
        let n = app.opened.as_ref().map_or(1, |o| diagram_view::export::foreground_pages(&o.doc).len().max(1)) as i32;
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.text(TS_BODY, t("Seiten"));
        ui.next_size(80, 0);
        ui.number_box(ID + 5, &mut app.print.from, &Range::new(1, n, 1, 0));
        ui.text(TS_BODY, t("bis"));
        ui.next_size(80, 0);
        ui.number_box(ID + 6, &mut app.print.to, &Range::new(1, n, 1, 0));
        ui.box_end();
    }
    let mut bg = app.print.setup.background as i32;
    ui.checkbox(ID + 7, t("Hintergrundseiten drucken"), &mut bg);
    app.print.setup.background = bg != 0;
    let mut paper = app.print.setup.paper as i32;
    let names: Vec<&str> = PAPERS.iter().map(|p| p.0).collect();
    ui.next_size(-1, 0);
    ui.combobox(ID + 8, &mut paper, &names);
    app.print.setup.paper = paper.clamp(0, PAPERS.len() as i32 - 1) as usize;
    let mut orient = app.print.setup.landscape as i32;
    ui.next_size(-1, 0);
    ui.combobox(ID + 9, &mut orient, &[t("Hochformat"), t("Querformat")]);
    app.print.setup.landscape = orient != 0;
    let mut fit = match app.print.setup.fit {
        Fit::Percent(p) if (p - 1.0).abs() < 1e-9 => 0,
        Fit::Sheets(1, 1) => 1,
        _ => 2,
    };
    let fit_old = fit;
    ui.next_size(-1, 0);
    ui.combobox(ID + 10, &mut fit, &[t("Tatsächliche Größe"), t("Auf ein Blatt anpassen"), t("Benutzerdefiniert (Seite einrichten)")]);
    if fit != fit_old {
        match fit {
            0 => app.print.setup.fit = Fit::Percent(1.0),
            1 => app.print.setup.fit = Fit::Sheets(1, 1),
            _ => open_setup(app, ui, 0),
        }
    }
    let mut gray = app.print.setup.gray as i32;
    ui.next_size(-1, 0);
    ui.combobox(ID + 11, &mut gray, &[t("Farbe"), t("Graustufen")]);
    app.print.setup.gray = gray != 0;
    if ui.link(ID + 12, t("Seite einrichten"), TS_BODY) & EV_ACT != 0 {
        open_setup(app, ui, 0);
    }
    if let Some((sev, msg)) = app.print.status.clone() {
        ui.next_size(300, 0);
        ui.text(TS_BODY | TX_WRAP | if sev == 3 { TX_ACCENT } else { TX_SECONDARY }, &msg);
    }
    ui.box_end();
    if app.print.setup != old {
        app.print.preview = None;
        app.print.status = None;
    }
    // ---- the preview
    preview(app, ui);
    ui.box_end();
    if go {
        print_now(app, ui);
    }
}

/// print the job now (Drucken, Strg+P on the page)
pub(crate) fn print_now(app: &mut Diagrammer, ui: &mut Ui) {
    let pages = job_pages(app);
    let title = app.drawing_name();
    let job = Job { pages, copies: app.print.copies.max(1) as u32, setup: app.print.setup.clone(), title };
    app.print.status = Some(match print_layout::print(app, &job) {
        Ok(n) => (0, tf("Gedruckte Blätter: {}", &[&format!("{n}")])),
        Err(msg) => (3, msg),
    });
    ui.repass();
}

/// the sheets of the job: (page, sheet of that page)
fn job_sheets(app: &mut Diagrammer) -> Vec<(PageRef, usize, usize)> {
    let mut out = Vec::new();
    for p in job_pages(app) {
        if let Some((_, t)) = page_tiling(app, p) {
            for k in 0..t.sheets.len() {
                out.push((p, k, t.sheets.len()));
            }
        }
        if out.len() > 2000 {
            break;
        }
    }
    out
}

fn preview(app: &mut Diagrammer, ui: &mut Ui) {
    ui.next_size(340, 0);
    ui.box_begin(ID + 20, BOX_V | BOX_XCENTER, 8);
    ui.next_size(340, 440);
    ui.box_begin(ID + 21, BOX_V | BOX_CARD | BOX_CENTER | BOX_XCENTER | pad(12), 0);
    let sheets = if app.opened.is_some() { job_sheets(app) } else { Vec::new() };
    let n = sheets.len() as i32;
    app.print.sheet = app.print.sheet.clamp(0, (n - 1).max(0));
    let mut caption = String::new();
    if let Some(&(page, k, of)) = sheets.get(app.print.sheet as usize) {
        let (pw, ph) = app.print.setup.paper_size();
        // the sheet as large as the card allows
        let (bw, bh) = (ui.dp(316) as f64, ui.dp(380) as f64);
        let dpi = (bw / pw).min(bh / ph);
        let key = format!("{:?}|{:?}|{}|{}|{}", page, app.print.setup, k, dpi as i64, app.opened.as_ref().map_or(0, version));
        if app.print.preview.as_ref().is_none_or(|p| p.0 != key) {
            if let Some((vk, tl)) = page_tiling(app, page) {
                let gray = app.print.setup.gray;
                if let Some(sf) = print_layout::render_sheet(&mut app.print.views[vk].3, &tl, k, dpi, None, gray) {
                    app.print.ver = app.print.ver.wrapping_add(1);
                    app.print.preview = Some((key, Rc::new(sf)));
                }
            }
        }
        if let Some((_, sf)) = app.print.preview.clone() {
            let r = ui.place(sf.w, sf.h);
            let frame = ui.color(T_CTRL_STROKE);
            ui.cmd_rrect(r[0] - 1, r[1] - 1, r[2] + 2, r[3] + 2, 0, ui.hair(), 0, frame, frame);
            ui.draw_image(&sf, app.print.ver, r[0], r[1], r[2], r[3]);
            ui.sem(K_IMAGE, ID + 22, Some(t("Seitenansicht")), 0);
        }
        let name = app.opened.as_ref().map_or(String::new(), |o| diagram_view::export::page_name(&o.doc, page));
        caption = if of > 1 { tf("{}: Blatt {} von {}", &[&name, &format!("{}", k + 1), &format!("{of}")]) } else { name };
    } else {
        ui.text(TS_BODY | TX_SECONDARY, t("Keine Seite zum Drucken"));
    }
    ui.box_end();
    // sheet navigation, the tiles of the page
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_flags(ST_SUBTLE | ST_ICON_ONLY | if app.print.sheet > 0 { 0 } else { ST_DISABLED });
    ui.next_icon(IC_BACK);
    if ui.button(ID + 23, t("Vorheriges Blatt")) & EV_ACT != 0 && app.print.sheet > 0 {
        app.print.sheet -= 1;
        ui.repass();
    }
    ui.text(TS_BODY, &tf("Blatt {} von {}", &[&format!("{}", (app.print.sheet + 1).min(n.max(1))), &format!("{}", n.max(1))]));
    ui.next_flags(ST_SUBTLE | ST_ICON_ONLY | if app.print.sheet + 1 < n { 0 } else { ST_DISABLED });
    ui.next_icon(IC_FORWARD);
    if ui.button(ID + 24, t("Nächstes Blatt")) & EV_ACT != 0 && app.print.sheet + 1 < n {
        app.print.sheet += 1;
        ui.repass();
    }
    ui.box_end();
    if !caption.is_empty() {
        ui.text(TS_CAPTION | TX_SECONDARY, &caption);
    }
    // the tiles of the page (PR-03): a small map of its sheets, the one shown marked
    if let Some(&(page, k, of)) = sheets.get(app.print.sheet as usize) {
        if of > 1 {
            if let Some((_, tl)) = page_tiling(app, page) {
                let (pw, ph) = tl.paper;
                let cell = |v: f64| ui.dp((v * 2.0) as i32).max(3);
                let (cw, ch) = (cell(pw), cell(ph));
                let (cols, rows) = (tl.cols.min(20) as i32, tl.rows.min(20) as i32);
                let r = ui.place(cols * (cw + 2), rows * (ch + 2));
                let (line, acc) = (ui.color(T_CTRL_STROKE), ui.color(T_ACCENT));
                for row in 0..rows {
                    for col in 0..cols {
                        let on = tl.sheets.get(k).is_some_and(|s| s.col == col as u32 && s.row == row as u32);
                        let fill = if on { acc } else { 0xFFFF_FFFF };
                        ui.cmd_rrect(r[0] + col * (cw + 2), r[1] + row * (ch + 2), cw, ch, 0, ui.hair(), fill, line, line);
                    }
                }
                ui.sem(K_IMAGE, ID + 25, Some(&tf("Kacheln: {} × {} Blätter", &[&format!("{}", tl.cols), &format!("{}", tl.rows)])), 0);
            }
        }
    }
    ui.box_end();
}

// ------------------------------------------------------------------ Seite einrichten

/// the dialog's working values (the page registers)
#[derive(Clone)]
struct PageEdit {
    page: PageRef,
    setup: PrintSetup,
    /// Drucken einrichten: 0 zoom, 1 n x m sheets
    zoom_mode: i32,
    zoom: f64,
    sheets: [i32; 2],
    margins_mm: [f64; 4],
    center: [i32; 2],
    /// Seitengröße: 0 as the paper, 1 predefined, 2 own
    size_mode: i32,
    size_pick: i32,
    size_mm: [f64; 2],
    page_landscape: i32,
    /// Zeichnungsmaßstab: 0 none, 1 predefined, 2 own (page mm = drawing mm)
    scale_mode: i32,
    scale_pick: i32,
    scale_own: [f64; 2],
    /// Seiteneigenschaften
    name: String,
    background: i32,
    unit: i32,
}

/// predefined page sizes (mm, portrait)
// i18n
static SIZES: [(&str, f64, f64); 8] = [
    ("A4: 210 × 297 mm", 210.0, 297.0),
    ("A3: 297 × 420 mm", 297.0, 420.0),
    ("A5: 148 × 210 mm", 148.0, 210.0),
    ("A2: 420 × 594 mm", 420.0, 594.0),
    ("A1: 594 × 841 mm", 594.0, 841.0),
    ("Letter: 8,5 × 11 in", 215.9, 279.4),
    ("Legal: 8,5 × 14 in", 215.9, 355.6),
    ("Tabloid: 11 × 17 in", 279.4, 431.8),
];

/// predefined scales: name, page units per drawing unit
// i18n
static SCALES: [(&str, f64); 12] = [
    ("Metrisch 1:2", 0.5),
    ("Metrisch 1:5", 0.2),
    ("Metrisch 1:10", 0.1),
    ("Metrisch 1:20", 0.05),
    ("Metrisch 1:50", 0.02),
    ("Metrisch 1:100", 0.01),
    ("Metrisch 1:200", 0.005),
    ("Metrisch 1:500", 0.002),
    ("Metrisch 1:1000", 0.001),
    ("Maschinenbau 2:1", 2.0),
    ("Maschinenbau 5:1", 5.0),
    ("Maschinenbau 10:1", 10.0),
];

/// the measuring units on offer
// i18n
static UNITS: [(&str, Unit); 6] = [("Millimeter", Unit::Millimeters), ("Zentimeter", Unit::Centimeters), ("Meter", Unit::Meters), ("Zoll", Unit::Inches), ("Fuß", Unit::Feet), ("Punkt", Unit::Points)];

/// open Seite einrichten on register `tab`
pub(crate) fn open_setup(app: &mut Diagrammer, ui: &mut Ui, tab: i32) {
    sync(app);
    let Some(page) = current_page(app) else {
        app.print.edit = None;
        app.print.tab = 0;
        app.print.dlg.set(1);
        ui.repass();
        return;
    };
    let o = app.opened.as_ref().expect("a page implies a drawing");
    let doc = &o.doc;
    let num = |name: &str| doc.sym(name).and_then(|c| doc.resolve(SheetRef::Page(page), &CellAddr::single(c)).num()).filter(|v| v.is_finite());
    let s = app.print.setup.clone();
    let (zoom_mode, zoom, sheets) = match s.fit {
        Fit::Percent(p) => (0, p * 100.0, [1, 1]),
        Fit::Sheets(a, b) => (1, 100.0, [a as i32, b as i32]),
    };
    let ps = num("PageScale").filter(|v| *v > 0.0).unwrap_or(1.0);
    let ds = num("DrawingScale").filter(|v| *v > 0.0).unwrap_or(1.0);
    let (w, h) = (num("PageWidth").unwrap_or(8.5) * ps / ds, num("PageHeight").unwrap_or(11.0) * ps / ds);
    let mm = |v: f64| v * 25.4;
    let ratio = ps / ds;
    let scale_pick = SCALES.iter().position(|(_, r)| (r - ratio).abs() < 1e-9);
    let pg = doc.page(page);
    let name = pg.and_then(|p| p.header.name.or(p.header.name_u)).map(|n| String::from(doc.str(n))).unwrap_or_default();
    let backs = background_pages(app, page);
    let back = pg.and_then(|p| p.header.back_page);
    let background = backs.iter().position(|(_, id, _)| Some(*id) == back).map_or(0, |k| k as i32 + 1);
    let size_pick = SIZES.iter().position(|(_, a, b)| ((mm(w) - a).abs() < 0.5 && (mm(h) - b).abs() < 0.5) || ((mm(w) - b).abs() < 0.5 && (mm(h) - a).abs() < 0.5));
    app.print.edit = Some(PageEdit {
        page,
        zoom_mode,
        zoom,
        sheets,
        margins_mm: [mm(s.margins[0]), mm(s.margins[1]), mm(s.margins[2]), mm(s.margins[3])],
        center: [s.center[0] as i32, s.center[1] as i32],
        setup: s,
        size_mode: if size_pick.is_some() { 1 } else { 2 },
        size_pick: size_pick.unwrap_or(0) as i32,
        size_mm: [mm(w), mm(h)],
        page_landscape: (w > h) as i32,
        scale_mode: if (ratio - 1.0).abs() < 1e-9 { 0 } else if scale_pick.is_some() { 1 } else { 2 },
        scale_pick: scale_pick.unwrap_or(2) as i32,
        scale_own: [1.0, 1.0 / ratio],
        name,
        background,
        unit: 0,
    });
    crate::layout_ui::load(app, page);
    app.print.tab = tab;
    app.print.dlg.set(1);
    ui.repass();
}

/// the background pages a page may take: (page, ID, name), not the page itself
fn background_pages(app: &Diagrammer, page: PageRef) -> Vec<(PageRef, u32, String)> {
    let Some(o) = app.opened.as_ref() else { return Vec::new() };
    o.doc
        .pages()
        .filter(|p| *p != page)
        .filter_map(|p| {
            let pg = o.doc.page(p)?;
            pg.header.background.then(|| (p, pg.header.id, diagram_view::export::page_name(&o.doc, p)))
        })
        .collect()
}

/// the dialog (every pass)
pub(crate) fn dialogs(app: &mut Diagrammer, ui: &mut Ui) {
    let dlg = app.print.dlg.clone();
    if !ui.dialog_begin(ID_DLG, t("Seite einrichten"), &dlg) {
        return;
    }
    let Some(mut e) = app.print.edit.clone() else {
        ui.next_size(420, 0);
        ui.text(TS_BODY | TX_WRAP, t("Öffnen Sie eine Zeichnung, um ihre Seiten einzurichten."));
        ui.dialog_buttons(None, None, Some(t("Schließen")));
        ui.dialog_end();
        return;
    };
    let regs = [t("Drucken einrichten"), t("Seitengröße"), t("Zeichnungsmaßstab"), t("Seiteneigenschaften"), t("Layout und Routing")];
    ui.next_size(500, 0);
    ui.tabs(ID_DLG + 1, &mut app.print.tab, &regs);
    ui.next_size(500, 300);
    ui.box_begin(ID_DLG + 2, BOX_V, 8);
    match app.print.tab {
        0 => tab_print(app, ui, &mut e),
        1 => tab_size(app, ui, &mut e),
        2 => tab_scale(app, ui, &mut e),
        3 => tab_props(app, ui, &mut e),
        _ => crate::layout_ui::tab(app, ui, ID_DLG + 60),
    }
    ui.box_end();
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    app.print.edit = Some(e.clone());
    if r == DLG_PRIMARY {
        crate::layout_ui::commit(app, ui);
        apply(app, ui, &e);
    }
    if r != DLG_NONE {
        app.print.edit = None;
    }
}

fn row_label(ui: &mut Ui, s: &str) {
    ui.next_size(150, 0);
    ui.text(TS_BODY, s);
}

fn tab_print(_app: &mut Diagrammer, ui: &mut Ui, e: &mut PageEdit) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row_label(ui, t("Druckerpapier"));
    let mut paper = e.setup.paper as i32;
    let names: Vec<&str> = PAPERS.iter().map(|p| p.0).collect();
    ui.next_size(160, 0);
    ui.combobox(ID_DLG + 10, &mut paper, &names);
    e.setup.paper = paper.clamp(0, PAPERS.len() as i32 - 1) as usize;
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
    row_label(ui, "");
    let mut l = e.setup.landscape as i32;
    ui.radio(ID_DLG + 11, t("Hochformat"), &mut l, 0);
    ui.radio(ID_DLG + 12, t("Querformat"), &mut l, 1);
    e.setup.landscape = l != 0;
    ui.box_end();
    ui.text(TS_BODY_STRONG, t("Druckzoom"));
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.radio(ID_DLG + 13, t("Anpassen auf"), &mut e.zoom_mode, 0);
    ui.next_size(90, 0);
    ui.number_box_f(ID_DLG + 14, &mut e.zoom, &RangeF::new(1.0, 1000.0, 5.0, 0, 0));
    ui.text(TS_BODY, "%");
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.radio(ID_DLG + 15, t("Anpassen an"), &mut e.zoom_mode, 1);
    ui.next_size(70, 0);
    ui.number_box(ID_DLG + 16, &mut e.sheets[0], &Range::new(1, 99, 1, 0));
    ui.text(TS_BODY, t("Blätter breit und"));
    ui.next_size(70, 0);
    ui.number_box(ID_DLG + 17, &mut e.sheets[1], &Range::new(1, 99, 1, 0));
    ui.text(TS_BODY, t("hoch"));
    ui.box_end();
    e.setup.fit = if e.zoom_mode == 1 { Fit::Sheets(e.sheets[0].max(1) as u32, e.sheets[1].max(1) as u32) } else { Fit::Percent(e.zoom / 100.0) };
    ui.text(TS_BODY_STRONG, t("Ränder (mm)"));
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    for (k, label) in [crate::tc("Links|Rand"), t("Oben"), t("Rechts"), t("Unten")].iter().enumerate() {
        ui.text(TS_BODY, label);
        ui.next_size(70, 0);
        ui.number_box_f(ID_DLG + 18 + k as u32, &mut e.margins_mm[k], &RangeF::new(0.0, 100.0, 1.0, 1, 0));
        e.setup.margins[k] = e.margins_mm[k] / 25.4;
    }
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
    ui.checkbox(ID_DLG + 22, t("Horizontal zentrieren"), &mut e.center[0]);
    ui.checkbox(ID_DLG + 23, t("Vertikal zentrieren"), &mut e.center[1]);
    e.setup.center = [e.center[0] != 0, e.center[1] != 0];
    ui.box_end();
}

fn tab_size(app: &mut Diagrammer, ui: &mut Ui, e: &mut PageEdit) {
    let (pw, ph) = e.setup.paper_size();
    ui.radio(ID_DLG + 30, &tf("Wie Druckerpapier ({})", &[&mm_pair(pw * 25.4, ph * 25.4)]), &mut e.size_mode, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.radio(ID_DLG + 31, t("Vordefinierte Größe"), &mut e.size_mode, 1);
    let names: Vec<&str> = SIZES.iter().map(|s| t(s.0)).collect();
    ui.next_size(220, 0);
    ui.next_flags(if e.size_mode == 1 { 0 } else { ST_DISABLED });
    ui.combobox(ID_DLG + 32, &mut e.size_pick, &names);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.radio(ID_DLG + 33, t("Benutzerdefinierte Größe"), &mut e.size_mode, 2);
    let dis = if e.size_mode == 2 { 0 } else { ST_DISABLED };
    ui.next_size(90, 0);
    ui.next_flags(dis);
    ui.number_box_f(ID_DLG + 34, &mut e.size_mm[0], &RangeF::new(1.0, 100_000.0, 1.0, 1, 0));
    ui.text(TS_BODY, "×");
    ui.next_size(90, 0);
    ui.next_flags(dis);
    ui.number_box_f(ID_DLG + 35, &mut e.size_mm[1], &RangeF::new(1.0, 100_000.0, 1.0, 1, 0));
    ui.text(TS_BODY, "mm");
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
    row_label(ui, t("Seitenausrichtung"));
    ui.radio(ID_DLG + 36, t("Hochformat"), &mut e.page_landscape, 0);
    ui.radio(ID_DLG + 37, t("Querformat"), &mut e.page_landscape, 1);
    ui.box_end();
    let (w, h) = page_size_mm(e);
    ui.text(TS_BODY | TX_SECONDARY, &tf("Neue Seitengröße: {}", &[&mm_pair(w, h)]));
    let _ = app;
}

/// the page size the register Seitengröße asks for (mm, the orientation applied)
fn page_size_mm(e: &PageEdit) -> (f64, f64) {
    let (a, b) = match e.size_mode {
        0 => {
            let (w, h) = e.setup.paper_size();
            (w * 25.4, h * 25.4)
        }
        1 => {
            let s = SIZES[e.size_pick.clamp(0, SIZES.len() as i32 - 1) as usize];
            (s.1, s.2)
        }
        _ => (e.size_mm[0], e.size_mm[1]),
    };
    let (lo, hi) = (a.min(b), a.max(b));
    if e.size_mode == 2 {
        (a, b)
    } else if e.page_landscape != 0 {
        (hi, lo)
    } else {
        (lo, hi)
    }
}

fn mm_pair(w: f64, h: f64) -> String {
    format!("{} × {} mm", (w + 0.5) as i64, (h + 0.5) as i64)
}

fn tab_scale(_app: &mut Diagrammer, ui: &mut Ui, e: &mut PageEdit) {
    ui.radio(ID_DLG + 40, t("Kein Maßstab (1:1)"), &mut e.scale_mode, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.radio(ID_DLG + 41, t("Vordefinierter Maßstab"), &mut e.scale_mode, 1);
    let names: Vec<&str> = SCALES.iter().map(|s| t(s.0)).collect();
    ui.next_size(200, 0);
    ui.next_flags(if e.scale_mode == 1 { 0 } else { ST_DISABLED });
    ui.combobox(ID_DLG + 42, &mut e.scale_pick, &names);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.radio(ID_DLG + 43, t("Benutzerdefinierter Maßstab"), &mut e.scale_mode, 2);
    let dis = if e.scale_mode == 2 { 0 } else { ST_DISABLED };
    ui.next_size(80, 0);
    ui.next_flags(dis);
    ui.number_box_f(ID_DLG + 44, &mut e.scale_own[0], &RangeF::new(0.001, 100_000.0, 1.0, 3, 0));
    ui.text(TS_BODY, "=");
    ui.next_size(80, 0);
    ui.next_flags(dis);
    ui.number_box_f(ID_DLG + 45, &mut e.scale_own[1], &RangeF::new(0.001, 100_000.0, 1.0, 3, 0));
    ui.box_end();
    let r = scale_ratio(e);
    let (w, h) = page_size_mm(e);
    ui.text(TS_BODY | TX_SECONDARY, &tf("Seitengröße in Zeichnungseinheiten: {}", &[&mm_pair(w / r, h / r)]));
}

/// page units per drawing unit the register Zeichnungsmaßstab asks for
fn scale_ratio(e: &PageEdit) -> f64 {
    match e.scale_mode {
        0 => 1.0,
        1 => SCALES[e.scale_pick.clamp(0, SCALES.len() as i32 - 1) as usize].1,
        _ => {
            let r = e.scale_own[0] / e.scale_own[1];
            if r.is_finite() && r > 0.0 { r } else { 1.0 }
        }
    }
}

fn tab_props(app: &mut Diagrammer, ui: &mut Ui, e: &mut PageEdit) {
    let background_page = app.opened.as_ref().and_then(|o| o.doc.page(e.page)).is_some_and(|p| p.header.background);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
    row_label(ui, t("Typ"));
    let mut typ = background_page as i32;
    ui.next_flags(ST_DISABLED);
    ui.radio(ID_DLG + 50, t("Vordergrund"), &mut typ, 0);
    ui.next_flags(ST_DISABLED);
    ui.radio(ID_DLG + 51, t("Hintergrund"), &mut typ, 1);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row_label(ui, t("Name"));
    ui.next_size(240, 0);
    ui.textbox(ID_DLG + 52, None, &mut e.name, 128);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row_label(ui, t("Hintergrund"));
    let backs = background_pages(app, e.page);
    let mut names: Vec<String> = alloc::vec![String::from(t("Kein"))];
    names.extend(backs.iter().map(|b| b.2.clone()));
    let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    ui.next_size(240, 0);
    ui.next_flags(if background_page { ST_DISABLED } else { 0 });
    ui.combobox(ID_DLG + 53, &mut e.background, &refs);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row_label(ui, t("Maßeinheit"));
    let units: Vec<&str> = UNITS.iter().map(|u| t(u.0)).collect();
    ui.next_size(240, 0);
    ui.combobox(ID_DLG + 54, &mut e.unit, &units);
    ui.box_end();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    row_label(ui, t("Schattenrichtung"));
    let (dx, dy) = app.opened.as_ref().map_or((0.125, -0.125), |o| {
        let doc = &o.doc;
        let num = |name: &str, d: f64| doc.sym(name).and_then(|c| doc.resolve(SheetRef::Page(e.page), &CellAddr::single(c)).num()).filter(|v| v.is_finite()).unwrap_or(d);
        (num("ShdwOffsetX", 0.125), num("ShdwOffsetY", -0.125))
    });
    ui.text(TS_BODY | TX_SECONDARY, &shadow_direction(dx, dy));
    ui.box_end();
}

fn shadow_direction(dx: f64, dy: f64) -> String {
    let h = if dx > 1e-9 { t("rechts") } else if dx < -1e-9 { t("links") } else { "" };
    let v = if dy > 1e-9 { t("oben") } else if dy < -1e-9 { t("unten") } else { "" };
    match (h.is_empty(), v.is_empty()) {
        (true, true) => String::from(t("kein Versatz")),
        (false, true) => String::from(h),
        (true, false) => String::from(v),
        _ => format!("{v} {h}"),
    }
}

/// OK: the print setup, then what changed of the page's size, scale, name, background and unit
fn apply(app: &mut Diagrammer, ui: &mut Ui, e: &PageEdit) {
    app.print.setup = e.setup.clone();
    app.print.preview = None;
    let backs = background_pages(app, e.page);
    let Some(o) = app.opened.as_mut() else { return };
    let page = e.page;
    let sheet = SheetRef::Page(page);
    let unit = UNITS[e.unit.clamp(0, UNITS.len() as i32 - 1) as usize].1;
    let r = scale_ratio(e);
    let (w, h) = page_size_mm(e);
    // what the page has now (paper mm, page units per drawing unit)
    let (w0, h0, r0) = {
        let doc = &o.doc;
        let num = |name: &str| doc.sym(name).and_then(|c| doc.resolve(sheet, &CellAddr::single(c)).num()).filter(|v| v.is_finite());
        let ps = num("PageScale").filter(|v| *v > 0.0).unwrap_or(1.0);
        let ds = num("DrawingScale").filter(|v| *v > 0.0).unwrap_or(1.0);
        (num("PageWidth").unwrap_or(8.5) * ps / ds * 25.4, num("PageHeight").unwrap_or(11.0) * ps / ds * 25.4, ps / ds)
    };
    let resize = (w - w0).abs() > 0.05 || (h - h0).abs() > 0.05 || (r - r0).abs() > 1e-9 || e.unit != 0;
    let old_bg = o.doc.background_of(page);
    let want_bg = if e.background > 0 { backs.get(e.background as usize - 1).map(|b| b.0) } else { None };
    let name_now = o.doc.page(page).and_then(|p| p.header.name.or(p.header.name_u)).map(|n| String::from(o.doc.str(n))).unwrap_or_default();
    let rename = !e.name.trim().is_empty() && e.name.trim() != name_now;
    if !resize && !rename && want_bg == old_bg {
        ui.repass();
        return;
    }
    // one undo step through the editing layer (JD7): saved with the drawing, undone with Strg+Z
    let r = crate::edit_doc::edit(o, t("Seite einrichten"), true, |tx, eng| {
        if resize {
            // drawing units: the paper size over the scale, in the chosen unit
            let to_unit = |inches: f64| diagram_core::convert(inches, Unit::Inches, unit).unwrap_or(inches);
            let pw = tx.single("PageWidth");
            let ph = tx.single("PageHeight");
            let pscale = tx.single("PageScale");
            let dscale = tx.single("DrawingScale");
            tx.set_length(sheet, &pw, to_unit(w / 25.4 / r), unit)?;
            tx.set_length(sheet, &ph, to_unit(h / 25.4 / r), unit)?;
            // PageScale / DrawingScale = r: one page unit stands for 1 / r drawing units
            tx.set_length(sheet, &pscale, to_unit(1.0), unit)?;
            tx.set_length(sheet, &dscale, to_unit(1.0 / r), unit)?;
        }
        if rename {
            tx.rename_page(page, e.name.trim(), e.name.trim())?;
        }
        if want_bg != old_bg {
            tx.set_background(page, want_bg)?;
        }
        // what depends on the page's cells (ThePage!PageWidth ...)
        eng.recalc(tx, &[sheet])?;
        Ok(())
    });
    if r.is_err() {
        app.print.status = Some((3, String::from(t("Die Seite ließ sich nicht ändern."))));
    }
    // the tabs show the pages' names; a new size is fitted into the window
    for tab in o.tabs.iter_mut() {
        if let Some(n) = o.doc.page(tab.page).and_then(|pg| pg.header.name.or(pg.header.name_u)) {
            tab.name = String::from(o.doc.str(n));
        }
    }
    if resize {
        app.canvas.request_fit();
        app.fit_again = true;
    }
    app.print.views.clear();
    ui.repass();
}
