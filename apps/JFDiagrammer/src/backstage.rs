// backstage -- the File register's pages (FEATURES.md 3.1: UI-04 .. UI-18, P1 part) in the
// library's backstage frame (ui.backstage_begin)
//
//   ITEMS / PAGE_*             the pane: Neu, Öffnen, Speichern, Speichern unter, Drucken,
//                              Exportieren, Schließen; at its bottom Info and Optionen
//   frame(app, ui)             the page shown (app.backstage) and the pane; back or Esc
//                              returns to the drawing
//
// Pages today: Neu is the template catalogue (templates_ui.rs, JD13: categories, rendered
// previews, starter diagrams, units, Erstellen; UI-04, VL-01).  Öffnen (JD4) browses
// with the file dialog and lists the drawings opened last; they open read-only (viewer.rs).
// Speichern and Speichern unter write the drawing (JD7), Speichern unter also as a template
// into the own template folder (JD13, EX-11); Drucken
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
        PAGE_NEW => crate::templates_ui::page(app, ui),
        PAGE_OPEN => open_page(app, ui),
        PAGE_SAVE | PAGE_SAVE_AS => save_page(app, ui, page == PAGE_SAVE_AS),
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

// (JD7) Speichern / Speichern unter: the file it goes to, Durchsuchen opens the file dialog
fn save_page(app: &mut Diagrammer, ui: &mut Ui, as_new: bool) {
    title(ui, t(if as_new { "Speichern unter" } else { "Speichern" }));
    let path = app.opened.as_ref().and_then(|o| o.ed.path.clone());
    ui.next_size(480, 0);
    match (&path, as_new) {
        (Some(p), false) => ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, &format!("{} {}", t("Speichert in"), p)),
        _ => ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Wählen Sie den Ort und den Dateityp.")),
    }
    ui.spacer(8);
    ui.box_begin(ID_PAGE, BOX_H | BOX_XCENTER, 16);
    let open = app.opened.is_some();
    if !as_new && path.is_some() {
        ui.next_size(96, 66);
        if ui.ribbon_button(ID_PAGE + 3, t("Speichern"), JD_SAVE_L, ST_LARGE | if open { 0 } else { ST_DISABLED }) & EV_ACT != 0 {
            app.backstage = None;
            crate::edit_doc::save(app, ui, false);
        }
    }
    ui.next_size(96, 66);
    if ui.ribbon_button(ID_PAGE + 1, t("Durchsuchen"), JD_SAVE_AS_L, ST_LARGE | if open { 0 } else { ST_DISABLED }) & EV_ACT != 0 {
        app.backstage = None;
        crate::edit_doc::save(app, ui, true);
    }
    // (JD13) as a template into the own template folder (EX-11, VL-03)
    if as_new {
        ui.next_size(96, 66);
        if ui.ribbon_button(ID_PAGE + 2, t("Als Vorlage"), JD_NEW_L, ST_LARGE | if open { 0 } else { ST_DISABLED }) & EV_ACT != 0 {
            crate::templates::save_as(app, ui);
        }
        ui.tooltip(t("Die Zeichnung als Vorlage (.vstx) im Ordner Ihrer Vorlagen speichern; die offenen Schablonen gehen mit"));
    }
    let names: alloc::vec::Vec<&str> = FILE_TYPES.iter().map(|s| t(s)).collect();
    let _ = names;
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
