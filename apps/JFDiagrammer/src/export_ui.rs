// export_ui -- Datei › Exportieren (JD12: UI-11, EX-01, EX-03, EX-04, EX-05, EX-14)
//
//   ExportState                the chosen format and its options, the save dialog, the result
//   page(app, ui)              the backstage page: the formats on the left (PDF, SVG, PNG, JPEG;
//                              web page and the drawing file types greyed out), the options of
//                              the chosen one on the right (pages: all, the current one, from ..
//                              to; background pages; PDF: links, bookmarks, document
//                              properties; SVG: shape data, the page's colour, links; PNG and
//                              JPEG: resolution, background or transparency, quality), Exportieren
//                              (the save dialog), Als Bild kopieren and Als SVG kopieren (the page on the clipboard)
//   dialogs(app, ui)           the save dialog, every pass
//   export(app, path) -> Result<message, message>   write the files (several pages as SVG, PNG or
//                              JPEG: one file each, "<name>-<n>.<ext>")
//   copy_page(app, ui, svg)    EX-14 for the page shown: a picture (PNG with a DIB) or the SVG
//                              as text; a selection comes with editing (JD7)
//
// The files come from diagram_view::export (PDF through lib/pdf, SVG, the CPU path for pixels):
// pages as the screen draws them, texts measured with the same fonts.  Test aid: --export-dir
// DIR puts DIR in front of the name the save dialog proposes.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::render::Options;
use diagram_view::export::{self, pdf as epdf, raster, svg, Pages};
use diagram_view::PageView;
use ui::i18n::{t, tf};
use ui::*;

use crate::icons::*;
use crate::Diagrammer;

const ID: u32 = 0x6600;
const ID_SAVE_DLG: u32 = 0x6700;

/// the formats: label, description
// i18n
static FORMATS: [(&str, &str); 4] = [
    ("PDF-Dokument erstellen", "Vektorgrafik mit eingebetteten Schriften, Text zum Suchen und Kopieren, Hyperlinks und einem Lesezeichen je Seite."),
    ("SVG-Grafik", "Vektorgrafik für Browser und Grafikprogramme: Pfade, Text, Bilder und Verläufe."),
    ("PNG-Grafik", "Rastergrafik ohne Verluste, mit wählbarer Auflösung und auf Wunsch transparentem Hintergrund."),
    ("JPEG-Grafik", "Rastergrafik mit wählbarer Qualität, klein für Fotos und Webseiten."),
];

/// the formats' file extensions
static EXT: [&str; 4] = ["pdf", "svg", "png", "jpg"];

/// the resolutions on offer (DPI; 0: own)
static DPIS: [i32; 4] = [96, 150, 300, 0];

/// the page's and the dialog's state
pub(crate) struct ExportState {
    /// FORMATS index
    pub format: i32,
    /// 0 every page, 1 the current one, 2 from .. to
    pub range: i32,
    pub from: i32,
    pub to: i32,
    pub background: i32,
    pub links: i32,
    pub bookmarks: i32,
    pub props: i32,
    pub shape_data: i32,
    pub paper: i32,
    /// DPIS index; own: dpi
    pub dpi_pick: i32,
    pub dpi: i32,
    pub transparent: i32,
    pub quality: i32,
    pub dlg: Shared<i32>,
    pub path: String,
    /// the last result: severity (0 information, 3 error), message
    pub status: Option<(u32, String)>,
    serial: u32,
}

impl ExportState {
    pub(crate) fn new() -> ExportState {
        ExportState {
            format: 0,
            range: 0,
            from: 1,
            to: 1,
            background: 1,
            links: 1,
            bookmarks: 1,
            props: 1,
            shape_data: 0,
            paper: 1,
            dpi_pick: 0,
            dpi: 200,
            transparent: 0,
            quality: 90,
            dlg: Shared::new(0),
            path: String::new(),
            status: None,
            serial: 0,
        }
    }

    fn dpi(&self) -> f64 {
        match DPIS.get(self.dpi_pick as usize).copied() {
            Some(0) | None => self.dpi.clamp(10, 2400) as f64,
            Some(d) => d as f64,
        }
    }
}

fn sync(app: &mut Diagrammer) {
    let Some(o) = app.opened.as_ref() else { return };
    if app.export.serial != o.serial {
        app.export.serial = o.serial;
        let n = export::foreground_pages(&o.doc).len().max(1) as i32;
        app.export.from = 1;
        app.export.to = n;
        app.export.status = None;
    }
}

/// the pages the export takes
fn chosen_pages(app: &Diagrammer) -> Vec<diagram_core::PageRef> {
    let Some(o) = app.opened.as_ref() else { return Vec::new() };
    let sel = match app.export.range {
        1 => match o.tabs.get(app.page_index()).or_else(|| o.tabs.first()) {
            Some(tab) => Pages::Current(tab.page),
            None => return Vec::new(),
        },
        2 => Pages::Range { from: app.export.from.max(1) as u32, to: app.export.to.max(1) as u32 },
        _ => Pages::All,
    };
    export::pages(&o.doc, &sel)
}

// ------------------------------------------------------------------ Datei › Exportieren

pub(crate) fn page(app: &mut Diagrammer, ui: &mut Ui) {
    sync(app);
    ui.text(TS_TITLE, t("Exportieren"));
    ui.spacer(12);
    let has = app.opened.is_some();
    ui.box_begin(ID, BOX_H, 32);
    // ---- the formats
    ui.next_size(250, 0);
    ui.box_begin(0, BOX_V, 2);
    let icons = [JD_SEND_PDF_L, JD_PICTURES, JD_PICTURES, JD_PICTURES];
    for (k, (name, _)) in FORMATS.iter().enumerate() {
        ui.next_size(-1, 36);
        ui.next_flags(ST_SUBTLE | ST_ALIGN_START | if app.export.format == k as i32 { ST_CHECKED } else { 0 });
        ui.next_icon(icons[k]);
        if ui.button(ID + 1 + k as u32, t(name)) & EV_ACT != 0 {
            app.export.format = k as i32;
            app.export.status = None;
            ui.repass();
        }
    }
    ui.spacer(8);
    for (k, name) in [t("Als Webseite speichern"), t("Dateityp ändern")].iter().enumerate() {
        ui.next_size(-1, 36);
        ui.next_flags(ST_SUBTLE | ST_ALIGN_START | ST_DISABLED);
        ui.next_icon([JD_FILE_DRAWING_L, JD_BS_EXPORT][k]);
        ui.button(ID + 8 + k as u32, name);
        ui.tooltip(&format!("{} · {}", name, t("Noch nicht verfügbar")));
    }
    ui.box_end();
    // ---- the options of the format
    let f = app.export.format.clamp(0, FORMATS.len() as i32 - 1) as usize;
    ui.next_size(440, 0);
    ui.box_begin(ID + 0x10, BOX_V, 8);
    ui.text(TS_SUBTITLE, t(FORMATS[f].0));
    ui.next_size(440, 0);
    ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, t(FORMATS[f].1));
    ui.spacer(4);
    if !has {
        ui.next_size(440, 0);
        ui.text(TS_BODY | TX_WRAP, t("Öffnen Sie eine Zeichnung, um sie zu exportieren."));
    }
    ui.text(TS_BODY_STRONG, t("Seiten"));
    ui.next_size(260, 0);
    ui.combobox(ID + 0x11, &mut app.export.range, &[t("Alle Seiten"), t("Aktuelle Seite"), t("Seitenbereich")]);
    if app.export.range == 2 {
        let n = app.opened.as_ref().map_or(1, |o| export::foreground_pages(&o.doc).len().max(1)) as i32;
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.text(TS_BODY, t("Von"));
        ui.next_size(80, 0);
        ui.number_box(ID + 0x12, &mut app.export.from, &Range::new(1, n, 1, 0));
        ui.text(TS_BODY, t("bis"));
        ui.next_size(80, 0);
        ui.number_box(ID + 0x13, &mut app.export.to, &Range::new(1, n, 1, 0));
        ui.box_end();
    }
    ui.checkbox(ID + 0x14, t("Hintergrundseiten einschließen"), &mut app.export.background);
    match f {
        0 => {
            ui.text(TS_BODY_STRONG, t("Optionen"));
            ui.checkbox(ID + 0x20, t("Hyperlinks übernehmen"), &mut app.export.links);
            ui.checkbox(ID + 0x21, t("Lesezeichen je Seite"), &mut app.export.bookmarks);
            ui.checkbox(ID + 0x22, t("Dokumenteigenschaften übernehmen"), &mut app.export.props);
        }
        1 => {
            ui.text(TS_BODY_STRONG, t("Optionen"));
            ui.checkbox(ID + 0x23, t("Seitenfarbe als Hintergrund"), &mut app.export.paper);
            ui.checkbox(ID + 0x24, t("Hyperlinks übernehmen"), &mut app.export.links);
            ui.checkbox(ID + 0x25, t("Shape-Daten als Attribute"), &mut app.export.shape_data);
        }
        _ => {
            ui.text(TS_BODY_STRONG, t("Auflösung"));
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            ui.next_size(220, 0);
            ui.combobox(ID + 0x26, &mut app.export.dpi_pick, &[t("Bildschirm (96 dpi)"), t("Mittel (150 dpi)"), t("Drucker (300 dpi)"), t("Benutzerdefiniert")]);
            if DPIS.get(app.export.dpi_pick as usize) == Some(&0) {
                ui.next_size(90, 0);
                ui.number_box(ID + 0x27, &mut app.export.dpi, &Range::new(10, 2400, 10, 0));
                ui.text(TS_BODY, "dpi");
            }
            ui.box_end();
            if f == 2 {
                ui.checkbox(ID + 0x28, t("Transparenter Hintergrund"), &mut app.export.transparent);
            } else {
                ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
                ui.text(TS_BODY, t("Qualität"));
                ui.next_size(90, 0);
                ui.number_box(ID + 0x29, &mut app.export.quality, &Range::new(1, 100, 5, 0));
                ui.text(TS_BODY, "%");
                ui.box_end();
            }
            if let Some(size) = pixel_size(app) {
                ui.text(TS_BODY | TX_SECONDARY, &size);
            }
        }
    }
    ui.spacer(8);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
    ui.next_size(96, 66);
    if ui.ribbon_button(ID + 0x30, t("Exportieren"), JD_EXPORT_L, ST_LARGE | if has { 0 } else { ST_DISABLED }) & EV_ACT != 0 && has {
        let stem = app.drawing_name();
        let dir = plat::arg_value("--export-dir").map(|d| format!("{}/", d.trim_end_matches(['/', '\\']))).unwrap_or_default();
        app.export.path = format!("{dir}{stem}.{}", EXT[f]);
        app.export.dlg.set(1);
        ui.repass();
    }
    ui.tooltip(t("Die gewählten Seiten in eine Datei schreiben"));
    ui.next_size(96, 66);
    if ui.ribbon_button(ID + 0x31, t("Als Bild kopieren"), JD_SEND_COPY_L, ST_LARGE | if has { 0 } else { ST_DISABLED }) & EV_ACT != 0 && has {
        copy_page(app, ui, false);
    }
    ui.tooltip(t("Die aktuelle Seite als Bild (PNG) in die Zwischenablage kopieren"));
    ui.next_size(96, 66);
    if ui.ribbon_button(ID + 0x32, t("Als SVG kopieren"), JD_PICTURES_L, ST_LARGE | if has { 0 } else { ST_DISABLED }) & EV_ACT != 0 && has {
        copy_page(app, ui, true);
    }
    ui.tooltip(t("Die aktuelle Seite als SVG-Text in die Zwischenablage kopieren (für Grafikprogramme und Webseiten)"));
    ui.box_end();
    if let Some((sev, msg)) = app.export.status.clone() {
        ui.next_size(440, 0);
        ui.text(TS_BODY | TX_WRAP | if sev == 3 { TX_ACCENT } else { TX_SECONDARY }, &msg);
    }
    ui.box_end();
    ui.box_end();
}

/// "Größe: W × H Pixel" of the current page at the chosen resolution
fn pixel_size(app: &Diagrammer) -> Option<String> {
    let o = app.opened.as_ref()?;
    let page = o.tabs.get(app.page_index())?.page;
    let doc = &o.doc;
    let num = |name: &str| doc.sym(name).and_then(|c| doc.resolve(diagram_core::SheetRef::Page(page), &diagram_core::CellAddr::single(c)).num()).filter(|v| v.is_finite());
    let ps = num("PageScale").filter(|v| *v > 0.0).unwrap_or(1.0);
    let ds = num("DrawingScale").filter(|v| *v > 0.0).unwrap_or(1.0);
    let (w, h) = (num("PageWidth")? * ps / ds, num("PageHeight")? * ps / ds);
    let dpi = app.export.dpi();
    Some(tf("Aktuelle Seite: {} × {} Pixel", &[&format!("{}", (w * dpi).ceil_u()), &format!("{}", (h * dpi).ceil_u())]))
}

trait CeilU {
    fn ceil_u(self) -> u64;
}

impl CeilU for f64 {
    fn ceil_u(self) -> u64 {
        if !(self > 0.0) {
            return 0;
        }
        let t = self as u64;
        if (t as f64) < self - 1e-6 { t + 1 } else { t }
    }
}

/// the save dialog (every pass)
pub(crate) fn dialogs(app: &mut Diagrammer, ui: &mut Ui) {
    let dlg = app.export.dlg.clone();
    let f = app.export.format.clamp(0, FORMATS.len() as i32 - 1) as usize;
    // i18n
    static EXTS: [&str; 4] = ["pdf=PDF-Dokument", "svg=SVG-Grafik", "png=PNG-Grafik", "jpg;jpeg=JPEG-Grafik"];
    let exts = [t(EXTS[f])];
    let mut path = core::mem::take(&mut app.export.path);
    let ev = ui.filedlg(ID_SAVE_DLG, FD_SAVE, &dlg, &mut path, 4096, &exts);
    app.export.path = path;
    if ev & EV_ACT != 0 {
        let p = app.export.path.clone();
        app.export.status = Some(match export(app, &p) {
            Ok(m) => (0, m),
            Err(m) => (3, m),
        });
        ui.repass();
    }
}

/// `path` with "-<n>" before its extension
fn numbered(path: &str, n: usize) -> String {
    let cut = path.rfind(['/', '\\']).map_or(0, |k| k + 1);
    match path[cut..].rfind('.') {
        Some(k) => format!("{}-{n}{}", &path[..cut + k], &path[cut + k..]),
        None => format!("{path}-{n}"),
    }
}

/// write the export into `path`: Ok(what was written) or the error to show
pub(crate) fn export(app: &mut Diagrammer, path: &str) -> Result<String, String> {
    let pages = chosen_pages(app);
    if pages.is_empty() {
        return Err(String::from(t("Es sind keine Seiten zum Exportieren gewählt.")));
    }
    let name = app.drawing_name();
    let st = &app.export;
    let f = st.format.clamp(0, FORMATS.len() as i32 - 1) as usize;
    let opts = Options { background: st.background != 0, hidden_layers: false };
    let (links, bookmarks, props, paper, shape_data) = (st.links != 0, st.bookmarks != 0, st.props != 0, st.paper != 0, st.shape_data != 0);
    let (dpi, transparent, quality) = (st.dpi(), st.transparent != 0, st.quality);
    let Some(o) = app.opened.as_ref() else { return Err(String::from(t("Es ist keine Zeichnung geöffnet."))) };
    let doc = &o.doc;
    let display = String::from(plat::display_name(path));
    if f == 0 {
        let info = if props {
            pdf::Info { title: Some(name.clone()), creator: Some(String::from("JFDiagrammer")), ..Default::default() }
        } else {
            pdf::Info { creator: Some(String::from("JFDiagrammer")), ..Default::default() }
        };
        let po = epdf::PdfOptions { pages: pages.clone(), render: opts, links, bookmarks, info, language: String::new() };
        let bytes = epdf::write(doc, &mut app.fonts, &po).map_err(|e| tf("Das PDF ließ sich nicht schreiben: {}", &[&format!("{e}")]))?;
        if !plat::write_file(path, &bytes) {
            return Err(tf("„{}“ ließ sich nicht schreiben.", &[&display]));
        }
        return Ok(tf("Gespeichert: {} (Seiten: {})", &[&display, &format!("{}", pages.len())]));
    }
    let mut written = Vec::new();
    for (k, &page) in pages.iter().enumerate() {
        let file = if pages.len() > 1 { numbered(path, k + 1) } else { String::from(path) };
        let mut view = PageView::new(doc, page, &mut app.fonts, &opts);
        let bytes = match f {
            1 => {
                let so = svg::SvgOptions { background: paper, links, shape_data, title: export::page_name(doc, page) };
                Some(svg::write(doc, view.drawing(), &so).into_bytes())
            }
            _ => {
                let bg = if transparent && f == 2 { raster::Background::Transparent } else { raster::Background::Page };
                let ro = raster::RasterOptions { dpi, region: None, background: bg };
                match raster::render(&mut view, &ro) {
                    Some(sf) if f == 2 => raster::png(&sf, dpi),
                    Some(sf) => raster::jpeg(&sf, quality, dpi, view.drawing().color),
                    None => return Err(String::from(t("Das Bild wäre zu groß; wählen Sie eine kleinere Auflösung."))),
                }
            }
        };
        let Some(bytes) = bytes else { return Err(tf("„{}“ ließ sich nicht kodieren.", &[&String::from(plat::display_name(&file))])) };
        if !plat::write_file(&file, &bytes) {
            return Err(tf("„{}“ ließ sich nicht schreiben.", &[&String::from(plat::display_name(&file))]));
        }
        written.push(file);
    }
    Ok(if written.len() == 1 { tf("Gespeichert: {}", &[&display]) } else { tf("Gespeichert: {} und weitere (Dateien: {})", &[&String::from(plat::display_name(&written[0])), &format!("{}", written.len())]) })
}

/// EX-14: the page shown on the clipboard as a picture (PNG + DIB, 150 DPI) or as SVG text (the
/// clipboard takes a picture or texts in one go; SVG as its own format, image/svg+xml, needs the
/// platform layer)
pub(crate) fn copy_page(app: &mut Diagrammer, ui: &mut Ui, as_svg: bool) {
    let Some(o) = app.opened.as_ref() else { return };
    let Some(tab) = o.tabs.get(app.page_index()).or_else(|| o.tabs.first()) else { return };
    let page = tab.page;
    let doc = &o.doc;
    let mut view = PageView::new(doc, page, &mut app.fonts, &Options::default());
    let ok = if as_svg {
        let text = svg::write(doc, view.drawing(), &svg::SvgOptions { title: export::page_name(doc, page), ..Default::default() });
        ui.clip_set(CLIP_TEXT, text.as_bytes()) == 0
    } else {
        let ro = raster::RasterOptions { dpi: 150.0, region: None, background: raster::Background::Page };
        raster::render(&mut view, &ro).is_some_and(|sf| ui.clip_set_image(&sf) == 0)
    };
    app.export.status = Some(match (ok, as_svg) {
        (true, false) => (0, String::from(t("Die Seite ist als Bild in der Zwischenablage."))),
        (true, true) => (0, String::from(t("Die Seite ist als SVG in der Zwischenablage."))),
        _ => (3, String::from(t("Die Seite ließ sich nicht kopieren."))),
    });
    ui.repass();
}
