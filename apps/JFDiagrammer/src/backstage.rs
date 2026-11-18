// backstage -- the File register's pages (FEATURES.md 3.1: UI-04 .. UI-18, P1 part) in the
// library's backstage frame (ui.backstage_begin)
//
//   ITEMS / PAGE_*             the pane: Neu, Öffnen, Speichern, Speichern unter, Drucken,
//                              Exportieren, Schließen; at its bottom Info and Optionen
//   frame(app, ui)             the page shown (app.backstage) and the pane; back or Esc
//                              returns to the drawing
//
// Pages today: Neu shows the P1 templates as placeholder previews (rendered previews come with
// the templates, JD13); a click opens the template's preview with its description, metric or
// US units and "Erstellen" (UI-04), which works for the blank drawing.  Öffnen (JD4) browses
// with the file dialog and lists the drawings opened last; they open read-only (viewer.rs).
// Speichern shows its layout with the commands disabled until writing exists (JD7); Drucken
// and Exportieren are print_ui.rs and export_ui.rs (JD12).  Info names the version, the licences and the place of the
// AWS attribution.  Optionen lists its pages at the side (options.rs).

use alloc::format;
use ui::i18n::{t, tf};
use ui::*;

use crate::commands;
use crate::icons::*;
use crate::options;
use crate::Diagrammer;

pub(crate) const PAGE_NEW: usize = 0;
pub(crate) const PAGE_OPEN: usize = 1;
pub(crate) const PAGE_SAVE: usize = 2;
pub(crate) const PAGE_SAVE_AS: usize = 3;
pub(crate) const PAGE_PRINT: usize = 4;
pub(crate) const PAGE_EXPORT: usize = 5;
pub(crate) const PAGE_CLOSE: usize = 6;
pub(crate) const PAGE_INFO: usize = 7;
pub(crate) const PAGE_OPTIONS: usize = 8;

const ID: u32 = 0x6000;
const ID_PAGE: u32 = 0x6100;

// the P1 templates (F11): name, description
// i18n
static TEMPLATES: [(&str, &str); 7] = [
    ("Leere Zeichnung", "Eine leere Seite ohne Schablonen."),
    ("Standarddiagramm", "Einfache Formen, Pfeile und Beschriftungen für allgemeine Diagramme."),
    ("Standardflussdiagramm", "Prozessschritte, Entscheidungen und Verbinder nach ISO 5807."),
    ("Funktionsübergreifendes Flussdiagramm", "Ein Ablauf in Bahnen je Abteilung oder Rolle."),
    ("Standardnetzwerk", "Rechner, Server und Netzwerkgeräte mit Verbindungen."),
    ("Detailliertes Netzwerk", "Netzwerkpläne mit Geräten, Racks und Verkabelung im Detail."),
    ("Rack-Diagramm", "Geräte in Racks nach Höheneinheiten."),
];

/// the pane's items, in the order of PAGE_*
fn items() -> [BackstageItem<'static>; 9] {
    [
        BackstageItem::new(t("Neu"), JD_NEW),
        BackstageItem::new(t("Öffnen"), JD_OPEN),
        BackstageItem::new(t("Speichern"), JD_SAVE).flags(BS_SEP),
        BackstageItem::new(t("Speichern unter"), 0),
        BackstageItem::new(t("Drucken"), JD_PRINT),
        BackstageItem::new(t("Exportieren"), 0),
        BackstageItem::new(t("Schließen"), 0).flags(BS_ACTION).enabled(commands::enabled(commands::CLOSE)),
        BackstageItem::new(t("Info"), 0).flags(BS_FOOTER),
        BackstageItem::new(t("Optionen"), 0).flags(BS_FOOTER),
    ]
}

/// the File register: the pane and the page app.backstage names
pub(crate) fn frame(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(mut page) = app.backstage else { return };
    let items = items();
    let mut sel = page as i32;
    let out = ui.backstage_begin(ID, t("Zurück"), &items, &mut sel, 200);
    page = sel.max(0) as usize;
    match out {
        Some(-1) => {
            app.backstage = None;
            ui.repass();
        }
        Some(i) if i as usize == PAGE_CLOSE => ui.cmd_run(commands::CLOSE, 0),
        Some(_) => app.template = None,
        None => {}
    }
    if app.backstage.is_some() {
        app.backstage = Some(page);
    }
    match page {
        PAGE_NEW => new_page(app, ui),
        PAGE_OPEN => open_page(app, ui),
        PAGE_SAVE | PAGE_SAVE_AS => save_page(ui, page == PAGE_SAVE_AS),
        PAGE_PRINT => crate::print_ui::page(app, ui),
        PAGE_EXPORT => crate::export_ui::page(app, ui),
        PAGE_INFO => info_page(ui),
        _ => options_page(app, ui),
    }
    ui.backstage_end();
}

// the page's title
fn title(ui: &mut Ui, s: &str) {
    ui.text(TS_TITLE, s);
    ui.spacer(12);
}

// a disabled large action with its QuickInfo
fn unavailable(ui: &mut Ui, id: u32, label: &str, icon: u32) {
    ui.next_size(96, 66);
    ui.ribbon_button(id, label, icon, ST_LARGE | ST_DISABLED);
    ui.tooltip(&format!("{} · {}", label, t("Noch nicht verfügbar")));
}

// ------------------------------------------------------------------ Neu

fn new_page(app: &mut Diagrammer, ui: &mut Ui) {
    if let Some(k) = app.template {
        preview(app, ui, k);
        return;
    }
    title(ui, t("Neu"));
    ui.text(TS_BODY_STRONG, t("Vorlagen"));
    ui.spacer(8);
    ui.box_begin(ID_PAGE, BOX_H | BOX_WRAP, 16);
    for (k, (name, _)) in TEMPLATES.iter().enumerate() {
        let id = ID_PAGE + 0x10 + k as u32 * 2;
        ui.next_size(152, 0);
        ui.box_begin(0, BOX_V, 4);
        ui.next_size(152, 112);
        let ev = thumb(ui, id, k, t(name));
        if ev & EV_ACT != 0 {
            app.template = Some(k);
            ui.repass();
        }
        if ev & EV_DOUBLE != 0 && k == 0 {
            app.new_blank(ui);
        }
        ui.next_size(152, 0);
        ui.text(TS_BODY | TX_WRAP, t(name));
        ui.box_end();
    }
    ui.box_end();
}

// a template's placeholder preview: a white page on the card with the template's motif drawn
// from rectangles and lines (rendered previews come with the templates)
fn thumb(ui: &mut Ui, id: u32, kind: usize, name: &str) -> u32 {
    let r = ui.place(ui.dp(152), ui.dp(112));
    let ev = ui.behave(id, HB_FOCUS);
    let st = ui.style;
    let mut sp = Sp { flags: ST_SUBTLE, ..Sp::default() };
    sp.place(r);
    sp.ev = ev;
    sp.state = if ev & EV_HELD != 0 { 2 } else if ev & EV_HOVER != 0 { 1 } else { 0 };
    st.button(ui, &sp);
    let m = ui.dp(10);
    let page = [r[0] + m, r[1] + m, r[2] - 2 * m, r[3] - 2 * m];
    let frame = ui.color(T_CTRL_STROKE);
    ui.cmd_rrect(page[0], page[1], page[2], page[3], 0, ui.hair(), 0xFFFF_FFFF, frame, frame);
    let ink = 0xFF5A_6470;
    let acc = 0xFF1F_5FA8;
    let s100 = ui.dp(100);
    let u = |v: i32| v * s100 / 100;
    let (x, y) = (page[0], page[1]);
    let boxes: &[(i32, i32, i32, i32, u32)] = match kind {
        1 => &[(14, 16, 28, 18, 0), (60, 16, 28, 18, 1), (37, 48, 28, 18, 0)],
        2 => &[(50, 6, 32, 12, 1), (50, 30, 32, 14, 0), (50, 58, 32, 14, 0)],
        3 => &[(4, 4, 124, 26, 2), (4, 34, 124, 26, 2), (20, 10, 26, 14, 1), (70, 40, 26, 14, 0)],
        4 => &[(12, 14, 24, 18, 0), (96, 14, 24, 18, 0), (54, 54, 24, 18, 1)],
        5 => &[(10, 10, 22, 16, 0), (55, 10, 22, 16, 0), (100, 10, 22, 16, 0), (32, 54, 22, 16, 1), (78, 54, 22, 16, 1)],
        6 => &[(46, 4, 40, 84, 2), (50, 10, 32, 8, 1), (50, 22, 32, 8, 0), (50, 34, 32, 8, 0), (50, 46, 32, 8, 1)],
        _ => &[],
    };
    for &(bx, by, bw, bh, k) in boxes {
        let (fill, stroke) = match k {
            1 => (0xFFD6_E4F5, acc),
            2 => (0x00FF_FFFF, ink),
            _ => (0xFFF0_F2F4, ink),
        };
        let h = ui.hair();
        ui.cmd_rrect(x + u(bx), y + u(by), u(bw), u(bh), 0, h, fill, stroke, stroke);
    }
    ui.focus_ring_draw(ev);
    ui.sem(K_BUTTON, id, Some(name), 0);
    ui.tooltip(name);
    ev
}

// a template's preview (UI-04): picture, name, description, units, Erstellen
fn preview(app: &mut Diagrammer, ui: &mut Ui, k: usize) {
    let (name, about) = TEMPLATES[k];
    ui.next_flags(ST_SUBTLE);
    ui.next_icon(IC_BACK);
    if ui.button(ID_PAGE + 1, t("Zurück zu den Vorlagen")) & EV_ACT != 0 {
        app.template = None;
        ui.repass();
    }
    ui.spacer(8);
    ui.box_begin(ID_PAGE + 2, BOX_H, 24);
    ui.next_size(304, 224);
    ui.box_begin(0, BOX_V | BOX_CARD | pad(8), 0);
    ui.next_size(-1, -1);
    thumb(ui, ID_PAGE + 3, k, t(name));
    ui.box_end();
    ui.next_size(320, 0);
    ui.box_begin(0, BOX_V, 10);
    ui.text(TS_SUBTITLE, t(name));
    ui.next_size(320, 0);
    ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, t(about));
    ui.spacer(4);
    ui.radio(ID_PAGE + 4, t("Metrische Einheiten"), &mut app.units_us, 0);
    ui.radio(ID_PAGE + 5, t("US-Einheiten"), &mut app.units_us, 1);
    ui.spacer(4);
    let ready = k == 0;
    ui.next_flags(ST_ACCENT | if ready { 0 } else { ST_DISABLED });
    if ui.button(ID_PAGE + 6, t("Erstellen")) & EV_ACT != 0 && ready {
        app.new_blank(ui);
    }
    if !ready {
        ui.tooltip(&format!("{} · {}", t("Erstellen"), t("Noch nicht verfügbar")));
    }
    ui.box_end();
    ui.box_end();
}

// ------------------------------------------------------------------ Öffnen, Speichern

fn open_page(app: &mut Diagrammer, ui: &mut Ui) {
    title(ui, t("Öffnen"));
    ui.box_begin(ID_PAGE, BOX_H, 24);
    ui.next_size(96, 66);
    if ui.ribbon_button(ID_PAGE + 1, t("Durchsuchen"), JD_OPEN_L, ST_LARGE) & EV_ACT != 0 {
        app.open_dlg.set(1);
        ui.repass();
    }
    ui.tooltip(t("Eine Zeichnung, Schablone oder Vorlage öffnen (schreibgeschützt)"));
    ui.next_size(420, 0);
    ui.box_begin(0, BOX_V, 6);
    ui.text(TS_BODY_STRONG, t("Zuletzt verwendet"));
    let recent = ui.recent_files();
    if recent.is_empty() {
        ui.next_size(420, 0);
        ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Hier erscheinen die Zeichnungen, die Sie zuletzt geöffnet oder gespeichert haben."));
    }
    let mut chosen = None;
    for (k, path) in recent.iter().take(12).enumerate() {
        ui.next_size(420, 0);
        ui.next_flags(ST_SUBTLE | ST_ALIGN_START);
        ui.next_icon(JD_OPEN);
        if ui.button(ID_PAGE + 0x10 + k as u32, &plat::display_name(path)) & EV_ACT != 0 {
            chosen = Some(path.clone());
        }
        ui.tooltip(path);
    }
    ui.box_end();
    ui.box_end();
    if let Some(p) = chosen {
        crate::viewer::open(app, ui, &p);
    }
}

// i18n
static FILE_TYPES: [&str; 4] = ["Visio-Zeichnung (.vsdx)", "Visio-Schablone (.vssx)", "Visio-Vorlage (.vstx)", "Visio-Zeichnung mit Makros (.vsdm)"];

fn save_page(ui: &mut Ui, as_new: bool) {
    title(ui, t(if as_new { "Speichern unter" } else { "Speichern" }));
    ui.next_size(480, 0);
    ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Zeichnungen lassen sich noch nicht speichern."));
    ui.spacer(8);
    ui.box_begin(ID_PAGE, BOX_H | BOX_XCENTER, 16);
    unavailable(ui, ID_PAGE + 1, t("Durchsuchen"), JD_SAVE_AS_L);
    let mut sel = 0;
    ui.text(TS_BODY, t("Dateityp"));
    ui.next_size(260, 0);
    ui.next_flags(ST_DISABLED);
    let names: alloc::vec::Vec<&str> = FILE_TYPES.iter().map(|s| t(s)).collect();
    ui.combobox(ID_PAGE + 2, &mut sel, &names);
    ui.box_end();
}

// ------------------------------------------------------------------ Info, Optionen

fn info_page(ui: &mut Ui) {
    title(ui, t("Info"));
    ui.text(TS_SUBTITLE, "JFDiagrammer");
    ui.text(TS_BODY, &tf("Version {}", &[ui::app_version!()]));
    ui.text(TS_BODY | TX_SECONDARY, &format!("JFLib {}", ui::VERSION));
    ui.spacer(12);
    ui.text(TS_BODY_STRONG, t("Lizenzen"));
    ui.next_size(520, 0);
    ui.text(TS_BODY | TX_WRAP, t("JFDiagrammer und JFLib stehen unter der MIT- oder der Apache-2.0-Lizenz, nach Ihrer Wahl."));
    ui.spacer(8);
    ui.text(TS_BODY_STRONG, t("Namensnennungen"));
    ui.next_size(520, 0);
    ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, t("Hier stehen die Namensnennungen mitgelieferter Symbole, sobald Schablonen dazukommen (etwa die AWS-Architektursymbole)."));
}

fn options_page(app: &mut Diagrammer, ui: &mut Ui) {
    title(ui, t("Optionen"));
    ui.box_begin(ID_PAGE, BOX_H, 24);
    ui.next_size(170, 0);
    ui.box_begin(0, BOX_V, 2);
    for (k, name) in options::PAGES.iter().enumerate() {
        ui.next_size(-1, 30);
        ui.next_flags(ST_SUBTLE | ST_ALIGN_START | if app.opt_page == k { ST_CHECKED } else { 0 });
        if ui.button(ID_PAGE + 0x10 + k as u32, t(name)) & EV_ACT != 0 {
            app.opt_page = k;
            ui.repass();
        }
    }
    ui.box_end();
    ui.box_begin(0, BOX_V, 0);
    options::page(app, ui, app.opt_page);
    ui.box_end();
    ui.box_end();
}
