// doc_file -- (JD16) what a drawing file brings besides its pages: Visio's older XML formats
// (.vdx, .vsx, .vtx: DF-20 .. DF-22), the macro project (.vsdm, .vssm, .vstm: DF-04, DF-07),
// the document properties on Datei › Info (DF-09) and the window state (DF-10)
//
//   read(bytes, path) -> Result<Loaded, Error>   a file of the VSDX family, or of the XML format
//                                     of 2003-2010 by its extension (or its first bytes)
//   is_xml(path)                      .vdx, .vsx, .vtx: opened, never written back (a save asks
//                                     for a new name and writes the VSDX family)
//   ask_macros(app, ui, path)         Speichern in a format without macros while the drawing has a
//                                     macro project: the question first (DF-04)
//   dialog(app, ui)                   that question (every pass)
//   opened(app, ui)                   after a drawing was installed: its stencil windows open, its
//                                     drawing window's page and zoom are shown (DF-10)
//   frame(app, ui)                    every pass: the zoom of the file once the page was fitted
//   before_save(app)                  an own file (written by JFLib) gets the drawing window and
//                                     the open stencils into its windows part; files of others
//                                     keep theirs as they are (DF-10)
//   props_page(app, ui)               Datei › Info: the drawing's properties (DF-09)
//
// Macros are kept as bytes and never run (diagram_io keeps vbaProject.bin as a part); opening
// tells so (viewer.rs).  Saving a macro drawing as .vsdx asks: without macros (vsdx::
// remove_macros), as .vsdm, or not at all.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{CellAddr, DocKind, SheetRef};
use diagram_io::props::{self, Props};
use diagram_io::template::StencilWindow;
use diagram_io::vsdx::{self, Format, Loaded};
use diagram_io::windows::{self, DrawingWindow};
use diagram_io::{stencil, vdx};
use ui::i18n::{t, tf};
use ui::*;

use crate::stencils::{self, Origin};
use crate::viewer::DIP;
use crate::Diagrammer;

const ID_ASK: u32 = 0x7E00;
const ID_PROPS: u32 = 0x7E10;

/// the state of this module in the app
#[derive(Default)]
pub(crate) struct DocFile {
    /// the question about macros: open, the path asked for
    pub ask: Shared<i32>,
    pub ask_path: String,
    /// the drawing window of the file opened last, applied once the page was fitted; passes to
    /// wait
    pending: Option<(DrawingWindow, u32)>,
    /// Datei › Info: the properties shown (of the drawing with this serial)
    shown: Option<(u32, Props)>,
}

/// whether a file name is one of the XML formats of 2003-2010
pub(crate) fn is_xml(path: &str) -> bool {
    ext(path).is_some_and(|e| vdx::kind_of(e).is_some())
}

fn ext(path: &str) -> Option<&str> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.rsplit_once('.').map(|x| x.1)
}

/// read a drawing, stencil or template: the XML format of 2003-2010 by its extension (or, for
/// another name, by its first bytes), else the VSDX family
pub(crate) fn read(bytes: &[u8], path: &str) -> Result<Loaded, vsdx::Error> {
    let kind = ext(path).and_then(vdx::kind_of);
    match kind {
        Some(k) => vdx::read(bytes, k, &vsdx::Limits::VSDX),
        None if vdx::sniff(bytes) => vdx::read(bytes, DocKind::Drawing, &vsdx::Limits::VSDX),
        None => vsdx::read(bytes, &vsdx::Limits::VSDX),
    }
}

// ------------------------------------------------------------------ macros (DF-04)

/// Speichern as `path` in a format without macros while the drawing has a macro project: ask
pub(crate) fn ask_macros(app: &mut Diagrammer, ui: &mut Ui, path: &str) {
    app.docfile.ask_path = String::from(path);
    app.docfile.ask.set(1);
    ui.repass();
}

/// the question (every pass)
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.docfile.ask.clone();
    if !ui.dialog_begin(ID_ASK, t("Makros speichern?"), &open) {
        return;
    }
    let path = app.docfile.ask_path.clone();
    let name = String::from(plat::display_name(&path));
    ui.next_size(420, 0);
    ui.text(TS_BODY | TX_WRAP, &tf("Die Zeichnung enthält Makros (VBA). „{}“ ist ein Format ohne Makros: Beim Speichern gehen sie verloren.", &[&name]));
    ui.spacer(6);
    ui.next_size(420, 0);
    ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, t("Als Zeichnung mit Makros (.vsdm) bleiben sie erhalten. JFDiagrammer führt Makros nie aus."));
    let r = ui.dialog_buttons(Some(t("Ohne Makros speichern")), Some(t("Mit Makros speichern")), Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_NONE {
        return;
    }
    app.docfile.ask.set(0);
    match r {
        DLG_PRIMARY => {
            if let Some(o) = app.opened.as_mut() {
                vsdx::remove_macros(&mut o.doc);
            }
            crate::edit_doc::save_to(app, ui, &path);
        }
        DLG_SECONDARY => {
            // Speichern unter again with the same name in the format of the same kind with
            // macros: the user confirms the name (PERMISSIONS fs.user writes what was chosen)
            let kind = Format::from_extension(ext(&path).unwrap_or("")).map_or(DocKind::Drawing, |f| f.kind);
            let with = Format { kind, macros: true };
            let stem = path.rsplit_once('.').filter(|x| !x.1.contains(['/', '\\'])).map_or(path.as_str(), |x| x.0);
            app.save_path = format!("{stem}.{}", with.extension());
            app.save_dlg.set(1);
        }
        _ => {}
    }
    ui.repass();
}

// ------------------------------------------------------------------ the window state (DF-10)

/// after a drawing was installed (opened from a file): its stencils, its page and zoom
pub(crate) fn opened(app: &mut Diagrammer, ui: &mut Ui, dir: Option<&str>) {
    let Some(o) = app.opened.as_ref() else { return };
    let list = diagram_io::template::stencil_windows(&o.doc);
    let win = windows::drawing_window(&o.doc);
    if !list.is_empty() {
        open_stencils(app, ui, &list, dir);
    }
    let Some(w) = win else { return };
    // the page shown
    if let (Some(id), Some(o)) = (w.page, app.opened.as_ref()) {
        if let Some(k) = o.doc.page_by_id(id).and_then(|p| o.tabs.iter().position(|tab| tab.page == p)) {
            app.page_tab = k as i32;
        }
    }
    // the zoom once the page was fitted (Visio's -1 and -2 are the whole page: the fit)
    if w.view_scale.is_some_and(|z| z > 0.0) {
        app.docfile.pending = Some((w, 1));
    }
    ui.repass();
}

// the stencils of the file's windows, in their order, the active one unfolded; those not found
// are left out (Visio does the same); the stencils open before are closed when the file names any
fn open_stencils(app: &mut Diagrammer, ui: &mut Ui, list: &[StencilWindow], dir: Option<&str>) {
    let mut keys: Vec<(u32, bool)> = Vec::new();
    for w in list {
        let name = String::from(w.document.rsplit(['/', '\\']).next().unwrap_or(&w.document));
        // a built-in stencil by its file name, in the measure the name says
        let builtin = stencil::parse_file_name(&name).and_then(|(stem, m)| stencils::BUILTIN.iter().position(|b| b.file.eq_ignore_ascii_case(&stem)).map(|k| (k, m)));
        // else one of the own stencils (data://shapes); other files only with the user's choice
        // (PERMISSIONS: fs.user), as the drawing's folder is not given with it
        let key = if let Some((b, m)) = builtin {
            stencils::open_builtin_in(app, ui, b, m)
        } else {
            let own = format!("data://shapes/{name}");
            let _ = dir;
            plat::read_file(&own).is_some().then(|| {
                stencils::open_file(app, ui, &own);
                app.stencils.loaded.iter().find(|l| l.origin == Origin::File(own.clone())).map(|l| l.key)
            }).flatten()
        };
        if let Some(k) = key {
            keys.push((k, w.active));
        }
    }
    if keys.is_empty() {
        return;
    }
    // the panel shows the file's stencils alone
    for key in app.stencils.open.clone() {
        if !keys.iter().any(|k| k.0 == key) {
            stencils::close(app, ui, key);
        }
    }
    if let Some(k) = keys.iter().find(|k| k.1).or(keys.first()) {
        app.stencils.panel.active = k.0;
    }
}

/// every pass: the zoom of the file's drawing window once the fit of the page is done
pub(crate) fn frame(app: &mut Diagrammer, ui: &mut Ui) {
    let Some((w, n)) = app.docfile.pending.take() else { return };
    if app.fit_again || n > 0 {
        app.docfile.pending = Some((w, if app.fit_again { n } else { n - 1 }));
        ui.repass();
        return;
    }
    let Some(z) = w.view_scale.filter(|z| *z > 0.0) else { return };
    app.set_zoom((z * 100.0 + 0.5) as i32);
    if let (Some((x, y)), Some(h)) = (w.center, page_height(app)) {
        app.canvas.vp.cx = x * DIP;
        app.canvas.vp.cy = (h - y) * DIP;
    }
    ui.repass();
}

// the height of the page shown, inches
fn page_height(app: &Diagrammer) -> Option<f64> {
    let o = app.opened.as_ref()?;
    let p = o.tabs.get(app.page_tab.max(0) as usize)?.page;
    let h = o.doc.sym("PageHeight")?;
    o.doc.resolve(SheetRef::Page(p), &CellAddr::single(h)).value().num()
}

/// before saving: an own file (written by JFLib) gets the drawing window and the open stencils
pub(crate) fn before_save(app: &mut Diagrammer, format: Format) {
    let stencil_list = crate::templates::windows_of_panel(app);
    let h = page_height(app);
    let vp = app.canvas.vp;
    let (rulers, grid, guides, breaks) = (app.view_rulers, app.view_grid, app.view_guides, app.view_breaks);
    let tab = app.page_tab.max(0) as usize;
    let Some(o) = app.opened.as_mut() else { return };
    if props::read(&o.doc).application != "JFLib" {
        return;
    }
    let page = o.tabs.get(tab).and_then(|t| o.doc.page(t.page)).map(|p| p.header.id);
    let center = h.map(|h| (vp.cx / DIP, h - vp.cy / DIP));
    let w = DrawingWindow {
        page,
        view_scale: Some(vp.zoom as f64),
        center,
        show_rulers: Some(rulers),
        show_grid: Some(grid),
        show_page_breaks: Some(breaks),
        show_guides: Some(guides),
        show_connection_points: None,
    };
    windows::set_drawing_window(&mut o.doc, &w);
    // a template keeps its stencils already (edit_doc::save_to); a drawing names its own
    if format.kind == DocKind::Drawing {
        diagram_io::template::set_stencil_windows(&mut o.doc, &stencil_list);
    }
}

// ------------------------------------------------------------------ properties (DF-09)

// (label, getter) of the fields that can be edited, in Visio's order
type Field = (&'static str, fn(&mut Props) -> &mut String);
// i18n
const FIELDS: [Field; 9] = [
    ("Titel", |p| &mut p.title),
    ("Thema", |p| &mut p.subject),
    ("Autor", |p| &mut p.creator),
    ("Manager", |p| &mut p.manager),
    ("Firma", |p| &mut p.company),
    ("Kategorie", |p| &mut p.category),
    ("Stichwörter", |p| &mut p.keywords),
    ("Kommentare", |p| &mut p.description),
    ("Hyperlinkbasis", |p| &mut p.hyperlink_base),
];

/// Datei › Info: the drawing's properties, editable; the dates and the template as they are
pub(crate) fn props_page(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(o) = app.opened.as_ref() else { return };
    let serial = o.serial;
    let macros = o.doc.macros;
    let xml = o.ed.path.as_deref().is_some_and(is_xml);
    if app.docfile.shown.as_ref().is_none_or(|s| s.0 != serial) {
        app.docfile.shown = Some((serial, props::read(&o.doc)));
    }
    let Some((_, mut p)) = app.docfile.shown.take() else { return };
    let before = p.clone();
    ui.text(TS_TITLE, t("Eigenschaften"));
    ui.spacer(6);
    // a row per field: its name, the text box
    for (k, (label, get)) in FIELDS.iter().enumerate() {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(140, 0);
        ui.text(TS_BODY, t(label));
        ui.next_size(420, 0);
        ui.textbox(ID_PROPS + k as u32, Some(t(label)), get(&mut p), 1024);
        ui.box_end();
    }
    ui.spacer(6);
    let dates = [(t("Erstellt"), &p.created), (t("Geändert"), &p.modified), (t("Zuletzt gedruckt"), &p.last_printed)];
    for (label, v) in dates {
        if !v.is_empty() {
            ui.next_size(420, 0);
            ui.text(TS_BODY | TX_SECONDARY, &format!("{label}: {}", date_text(v)));
        }
    }
    if !p.template.is_empty() {
        ui.next_size(420, 0);
        ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, &format!("{}: {}", t("Vorlage"), plat::display_name(&p.template)));
    }
    if macros {
        ui.next_size(420, 0);
        ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, t("Die Zeichnung enthält Makros (VBA). Sie bleiben beim Speichern als .vsdm erhalten und werden nie ausgeführt."));
    }
    if xml {
        ui.next_size(420, 0);
        ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, t("Geöffnet aus dem XML-Format von Visio 2003–2010; gespeichert wird als .vsdx."));
    }
    ui.spacer(16);
    if p != before {
        if let Some(o) = app.opened.as_mut() {
            props::write(&mut o.doc, &p);
        }
    }
    app.docfile.shown = Some((serial, p));
}

// a W3CDTF date as the region shows dates ("2026-11-23T10:00:00Z" -> its date and time)
fn date_text(v: &str) -> String {
    let d = v.split('T').next().unwrap_or(v);
    let time = v.split('T').nth(1).map(|x| x.trim_end_matches('Z')).and_then(|x| x.get(..5));
    let p: Vec<&str> = d.split('-').collect();
    match (p.as_slice(), time) {
        ([y, m, dd], Some(t)) => format!("{dd}.{m}.{y} {t}"),
        ([y, m, dd], None) => format!("{dd}.{m}.{y}"),
        _ => String::from(v),
    }
}
