// JFDiagrammer -- diagrams of the Visio kind: drawings, stencils and templates (.vsdx family)
//
// The app of FEATURES.md and PLAN.md (this folder).  It builds on the public jflib API only
// (ui, base, and later diagram_core / diagram_io) and moves to a repository of its own later.
// Look: the NetLab style (STYLE), the app's own title bar with the quick access toolbar, a
// ribbon on top, the Shapes panel on the left, the canvas in the middle, task panes on the
// right, the status bar below.
//
// Stage JD8 (the shell):
//   src/main.rs       the entry, the app's state, App
//   src/shell.rs      the window: title bar, register row, ribbon, dock area, status bar
//   src/commands.rs   every command: label, icons, handler (when its function exists), state,
//                     the library's command table per language with its shortcuts
//   src/ribbon.rs     the registers Start, Einfügen, Entwurf, Ansicht and the context
//                     registers Container-Tools › Format and Textwerkzeuge › Text
//   src/backstage.rs  the File register's pages
//   src/options.rs    the settings (base::settings) and the options pages
//   src/panes.rs      task panes (dock), canvas and page tabs, status bar, context menus
//   src/lang.rs       the five languages, switching at run time, the letter icons
//   src/icons.rs      the ribbon icons (JD14: own artwork in icons/*.svg, mkicons.py)
//   src/viewer.rs     (JD4) the drawing opened: loading, pages, the canvas' paint
//                     (diagram_core's display list drawn by diagram_view on render2d)
//   src/export_ui.rs  (JD12) Datei › Exportieren: PDF, SVG, PNG, JPEG, the page on the clipboard
//   src/print_ui.rs   (JD12) Datei › Drucken with its preview, the dialog Seite einrichten
//   src/print_layout.rs  (JD12) the print setup, sheets and tiles, the job (plat::PrintDoc)
//   src/edit_doc.rs   (JD7) editing: transactions with recalculation, the live preview of a
//                     drag, undo / redo, a new drawing, saving
//   src/edit_canvas.rs  (JD7) the canvas: ui.shape_edit's model, gestures -> diagram_core
//                     (ops, glue), rulers, guides, page breaks
//   src/edit_cmds.rs  (JD7) the editing commands: clipboard, arrange, group, tools, toggles
//   src/edit_text.rs  (JD7) the text of a shape edited in place (ui.richtext)
//   src/stencils.rs   (JD9) the stencils of the Shapes panel: built-in (build.rs builds them
//                     from stencils/), files, favourites, "Weitere Shapes", the search
//   src/stencils_panel.rs   (JD9) the panel (ui.stencil_panel), previews, its menus
//   src/stencils_drop.rs    (JD9) shapes onto the page: drag and drop, click + click, Enter
//   src/design_gallery.rs   (JD11) Entwurf › Designs and Varianten: galleries, live preview,
//                     applying (diagram_core::design)
//   src/design_store.rs     (JD11) the user's saved designs
//   src/format_cells.rs     (JD11) the selection's format cells read and written, the colour menu
//   src/format_menus.rs     (JD11) Start › Schriftart, Absatz, Formenarten, Format übertragen
//   src/format_pane.rs      (JD11) the task pane Shape formatieren
//   src/format_quick.rs     (JD11) Schnellformatvorlagen (from Shape formatieren)
//   src/structures.rs       (JD10) containers, lists, swimlanes, callouts on the canvas: the
//                     gestures with them (diagram_core::structure), the glow, the insert marks
//   src/structures_cmds.rs  (JD10) Einfügen › Container / Beschriftung, Container-Tools ›
//                     Format, Shapes verbinden, the Verbindungspfeile
//   src/structures_cff.rs   (JD10) the register Funktionsübergreifendes Flussdiagramm
//   src/layout_ui.rs  (JD10) Layout-Verhalten per shape, Seite einrichten › Layout und Routing
//   src/jumps_ui.rs   (JD21) line jumps: Linienübergänge anzeigen, the dialog Verbinder-Verhalten
//   src/templates.rs  (JD13) the templates: built-in (build.rs builds them from templates/ into
//                     .vstx), the own ones, a new drawing from one, Speichern als Vorlage
//   src/templates_ui.rs  (JD13) Datei › Neu: the catalogue with rendered previews
//   src/doc_file.rs   (JD16) Visio's XML formats of 2003-2010 (.vdx, .vsx, .vtx) opened, the
//                     question before a macro project is left out, Datei › Info's properties,
//                     the window state (page, zoom, stencils) read and written for own files
//                     (JD17) and the binary stencils of 2003-2010 (.vss) converted for the
//                     Shapes panel: import only, never written back (a save writes .vssx)
//   src/cmds_arrange.rs  (JD8 rest) Automatisch ausrichten
//   src/layout_cmds.rs   (JD21) Seitenlayout neu anordnen (diagram_core::layout) with the live
//                     preview of its rows, Positionieren: Automatischer Abstand, Automatische
//                     Ausrichtung und Abstände, page breaks, Diagramm drehen
//   src/dlg_layout.rs (JD21) Layout konfigurieren, Abstandsoptionen, Shapes verteilen
//   src/cmds_later.rs    (JD8 rest) the commands left for later with their reasons, the check
//   src/cmds_myshapes.rs (JD8 rest) Zu meinen Shapes hinzufügen (shapes of the page into Favoriten)
//   src/pane_sizepos.rs, pane_data.rs, pane_panzoom.rs  (JD8 rest) the task panes Größe und
//                     Position, Shape-Daten, Verschieben und Zoomen; Shape-Daten definieren
//   src/qat.rs        (JD8 rest) the quick access toolbar and its customization
//   src/cmds_shell.rs (JD8 rest) menu buttons' commands open their menus; Schrift, Linie,
//                     Schnellformatvorlagen, the dialog Zoom
//   src/pages.rs      (JD8 rest) the page tabs kept in step with the pages, Neue Seite,
//                     Hintergrundseite, duplicate, delete, rename, reorder (dialogs)
//   src/page_size.rs  (JD8 rest) Entwurf › Ausrichtung, Größe, Automatisch anpassen, the
//                     automatic size after a gesture (diagram_core::pagesetup)
//   src/cmds_draw.rs  (JD8 rest) Zeichentools: the split button, the freeform and arc tools
//   src/cmds_picture.rs  (JD8 rest) Einfügen › Bilder › Bild aus Datei
//   src/cmds_paste.rs (JD8 rest) Inhalte einfügen, Als Link einfügen, Strg+V from the system
//   src/layers.rs     (JD8 rest) Ebene zuweisen, Ebeneneigenschaften, the layers on the canvas
//   src/select_type.rs  (JD8 rest) Markieren › Nach Typ auswählen
//   src/cmds_text.rs  (JD8 rest) the text, field, link and search commands: state, target, the
//                     fields' locale and clock, the vertical text box, a shape into view
//   src/dlg_text.rs   (JD8 rest) the dialog Text (font, paragraph, text block)
//   src/dlg_field.rs  (JD8 rest) the dialog Feld (fields in the text, TX-06)
//   src/dlg_link.rs   (JD8 rest) the dialog Hyperlinks, following links, the link pointer
//   src/dlg_find.rs   (JD8 rest) the dialog Suchen und Ersetzen
//   src/ask_save.rs   (JD15) Speichern-Rückfrage before Schließen, Neu, Öffnen and the window's
//                     close (UI-12); Öffnen schreibgeschützt and als Kopie (DM-09, UI-06, UI-05)
//   src/dlg_shape_name.rs  (JD15) the dialog Shape-Name (SP-01)
//   src/dlg_snap.rs   (JD15) the dialog Ausrichten und Kleben
//   src/data_*.rs     (JD18) external data: data_cmds (when the commands run), data_source
//                     (reading .xlsx/.csv), data_import (Benutzerdefinierter Import), data_pane
//                     (Externe Daten), data_refresh (aktualisieren, Intervall, Konflikte),
//                     data_link (Daten verknüpfen), data_ask (Abfrage beim Ablegen),
//                     data_textdraw (Zeichnung aus Text)
//   src/shapeops_cmds.rs  (JD21) the shape operations and Ausrichtungsfeld aktualisieren
//                     (Start › Anordnen › Gruppieren › Vorgänge until JD23)
//   build.rs, build_steps.rs, stencils/   (JD9) the stencil sources and their build into .vssx
//                     when the app is built (README there; build_rs.py's derived builds for the
//                     browser and Android run build_steps.rs too); templates/ (JD13) the
//                     template sources, built into .vstx after the stencils
//   lang/shortcuts.tsv   the shortcuts of each language (Office conventions, LG-08)
//   lang/glossary.tsv    the binding terms in five languages (LG-06)
//   check_lang.py        the language check (LG-11): catalogue, glossary, shortcuts
//
// Languages: German source texts, the catalogue locales/ui.tsv with the columns de, en, es,
// it, fr (FEATURES.md 3.17).  The language follows the option File › Optionen › Sprache, by
// default the system's (--lang / jfgui_LANG in scripts); a change applies at once.  Texts go
// through t() / tc() where they are shown and are never kept, so the next pass is all in the
// new language.  Scripts run German unless their #!args say --lang.
//
// Test aids: --shell-preview shape|container|text stands in for a selection of that kind (the
// context registers without a drawing; JD7 brought the real selection);
// --config PATH keeps the settings in that file (headless runs keep them in memory otherwise);
// --save-dir DIR is where Speichern unter starts for a new drawing (JD7, scripts);
// a drawing on the command line opens at start (JD4); --no-system-fonts draws with the style's
// fonts only (the same on every machine); --write-stencils DIR writes the built-in stencils
// (.vssx, metric and US) into DIR and ends (JD9: the user's check in Visio); --export-dir DIR
// puts DIR in front of the name the export's save dialog proposes (JD12); --write-templates DIR
// writes the built-in templates (.vstx) into DIR and ends, --template-dir DIR is the folder of
// the own templates (JD13); --picture-dir DIR is where Bild aus Datei's dialog starts (JD8 rest);
// --check-commands shows which commands still lack a function as the notice (JD8 rest,
// cmds_later.rs).
// (JD18) --data-dir DIR is where Benutzerdefinierter Import's file dialog starts (scripts).
//
//   python build_rs.py run JFDiagrammer
//   python build_rs.py test JFDiagrammer          tests/*.uis
//   python build_rs.py i18n JFDiagrammer          the catalogue against the sources
//   python apps/JFDiagrammer/check_lang.py        catalogue, glossary, shortcuts (LG-11)
//   build/win/JFDiagrammer.exe --shot build/JFDiagrammer.png --scale 150
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

// (JD15) the question before changes are lost, the ways a file opens
mod ask_save;
// (JD21 autoconn) the connector automatics: splitting, healing, the quick shapes bar
mod autoconn;
mod backstage;
mod cmds_actions;
mod cmds_arrange;
mod cmds_later;
mod cmds_myshapes;
mod cmds_shell;
mod cmds_draw;
mod cmds_paste;
mod cmds_picture;
mod cmds_text;
mod commands;
// (JD18) data: the register Daten, the pane Externe Daten, import, link, refresh, drawing from text
mod data_ask;
mod data_cmds;
mod data_import;
mod data_link;
mod data_pane;
mod data_refresh;
mod data_source;
mod data_textdraw;
mod design_gallery;
mod design_store;
mod dlg_field;
mod dlg_find;
mod dlg_link;
// (JD15) Shape-Name (SP-01)
mod dlg_shape_name;
// (JD15) Ausrichten und Kleben (RA-07)
mod dlg_snap;
mod dlg_text;
mod doc_file;
mod edit_canvas;
mod edit_cmds;
mod edit_doc;
mod edit_text;
mod export_ui;
mod format_cells;
mod format_menus;
mod format_pane;
mod format_quick;
mod icons;
mod lang;
mod layers;
// (JD21 layout) Seitenlayout neu anordnen, Positionieren: spacing, page breaks, Diagramm drehen
mod dlg_layout;
mod layout_cmds;
mod layout_ui;
// (JD21 jumps) Linienübergänge anzeigen, Verbinder-Verhalten
mod jumps_ui;
// (JD13) the legend of a network plan counts its devices
mod legend;
mod options;
mod pane_data;
mod pane_panzoom;
mod pane_sizepos;
mod page_size;
mod pages;
mod panes;
mod print_layout;
mod print_ui;
mod qat;
// (JD13) equipment snaps onto the units of a rack
mod rack;
mod ribbon;
mod select_type;
// (JD21) Vorgänge: Gesamtmenge, Kombinieren, In Einzelmengen zerlegen, Schnittmenge bilden,
// Subtrahieren, Ausrichtungsfeld aktualisieren
mod shapeops_cmds;
mod shell;
mod stencils;
mod stencils_drop;
mod stencils_panel;
mod structures;
mod structures_cff;
mod structures_cmds;
mod templates;
mod templates_ui;
mod viewer;

use base::settings::Settings;
use ui::i18n::{self, Catalog};
use ui::*;

const DESC: AppDesc = AppDesc {
    title: "JFDiagrammer",
    flags: AF_RESIZABLE | AF_CUSTOM_TITLEBAR,
    w: 1280,
    h: 800,
    min_w: 640,
    min_h: 420,
    ..AppDesc::NONE
};

pub(crate) struct Diagrammer {
    /// the register shown in the ribbon
    pub reg: ribbon::Reg,
    /// the File register's page while it is open
    pub backstage: Option<usize>,
    /// the page it opens on next time
    pub last_page: usize,
    /// Neu: the template whose preview is shown (an index into the catalogue)
    pub template: Option<usize>,
    /// (JD13) Neu: the template catalogue
    pub catalog: templates::Catalog,
    /// the template preview's units: 0 metric, 1 US
    pub units_us: i32,
    /// the options page shown
    pub opt_page: usize,
    /// the canvas' view and selection (an empty page until JD4 brings the model)
    pub canvas: DiagramState,
    /// the dock layout: the Shapes panel and the task panes
    pub panes: DockState,
    /// the settings (File › Optionen)
    pub opts: Settings,
    /// the active tool (POINTER)
    pub tool: u32,
    /// Ansicht › Gitternetz
    pub view_grid: bool,
    /// the page tab shown
    pub page_tab: i32,
    /// Shape formatieren: its register
    pub format_tab: i32,
    /// --shell-preview: the selection kind it stands in for (ribbon::SEL_*)
    pub preview: u8,
    /// the ribbon icons' hook (set in init)
    icons: Option<icons::Icons>,
    /// (JD4) the drawing shown, editable since JD7 (None: no drawing, after Datei › Schließen)
    pub opened: Option<viewer::Opened>,
    /// a file that could not be opened: title and message of the notice
    pub error: Option<(alloc::string::String, alloc::string::String)>,
    /// the fonts pages are drawn with
    pub fonts: diagram_view::FontSet,
    /// Backstage › Öffnen: the file dialog, its path
    pub open_dlg: Shared<i32>,
    pub open_path: alloc::string::String,
    /// Alle Seiten: open the list of pages next pass
    pub pages_menu: bool,
    /// fit the page once more after the canvas was laid out (a notice that came with the
    /// drawing changes the canvas' height after the first fit)
    pub fit_again: bool,
    /// (JD12) Datei › Exportieren
    pub export: export_ui::ExportState,
    /// (JD12) Datei › Drucken, Seite einrichten
    pub print: print_ui::PrintState,
    /// (JD7) the edit layer's selection and gesture, its model of the page shown
    pub se: SeState,
    pub model: edit_canvas::Model,
    /// Ansicht › Lineal, Seitenumbrüche; Ausrichten und Kleben (snap to the grid)
    pub view_rulers: bool,
    pub view_breaks: bool,
    pub view_guides: bool,
    pub snap: bool,
    /// a guide being dragged out of a ruler: horizontal, world coordinate
    pub guide_drag: Option<(bool, f32)>,
    /// the text being edited; a request to start editing (shape key, typed character)
    pub text: Option<edit_text::TextEdit>,
    pub text_req: Option<(u64, Option<char>, bool)>,
    /// Speichern unter: the file dialog, its path
    pub save_dlg: Shared<i32>,
    pub save_path: alloc::string::String,
    /// shapes copied inside the app; how many grid steps the next paste is moved
    pub clip: Option<diagram_core::ops::Clip>,
    pub clip_offset: u32,
    /// (JD9) the Shapes panel: stencils, search, favourites, click-placing
    pub stencils: stencils::Stencils,
    /// (JD11) Entwurf › Designs and Varianten; the dialog that saves a design
    pub designs: design_gallery::Designs,
    pub design_save: design_store::SaveDialog,
    /// (JD11) the format commands: colours used last, the format painter, Weitere Farben
    pub format: format_menus::FormatState,
    /// (JD11) the quick styles' tiles
    pub quick: format_quick::QuickTiles,
    /// (JD10) structures: a running move, the selection's order, the list marks; the layout
    /// dialog and register
    pub structs: structures::Structs,
    pub layout: layout_ui::LayoutState,
    /// (JD8 rest) the dialog Zoom
    pub shell: cmds_shell::ShellState,
    /// (JD8 rest) Größe und Position, Shape-Daten (the field being typed into, the dialog)
    pub sizepos: pane_sizepos::SizePos,
    pub data: pane_data::DataState,
    /// (JD8 rest) Verschieben und Zoomen: the overview's picture, the drag
    pub panzoom: pane_panzoom::PanZoom,
    /// (JD8 rest) the quick access toolbar's dialog
    pub qat: qat::QatState,
    /// (JD8 rest) the dialogs Seite umbenennen, Seiten neu anordnen
    pub pages: pages::PagesState,
    /// (JD8 rest) Zeichentools, Bild aus Datei, Inhalte einfügen, Ebenen, Nach Typ auswählen
    pub draw: cmds_draw::DrawState,
    pub pictures: cmds_picture::PictureState,
    pub paste: cmds_paste::PasteState,
    pub layers: layers::LayerState,
    pub select: select_type::SelectState,
    /// (JD8 rest) the text, field, link and search commands (cmds_text.rs)
    pub txt: cmds_text::TextCmds,
    /// (JD16) the older XML formats, macros, properties, window state (doc_file.rs)
    pub docfile: doc_file::DocFile,
    /// (JD15) the question before changes are lost; how Öffnen opens a file
    pub ask: ask_save::AskSave,
    pub shape_name: dlg_shape_name::NameDialog,
    /// (JD15) Ausrichten und Kleben: snapping to the grid (Ausrichten an › Raster), the dialog
    pub snap_grid: bool,
    pub snap_dlg: dlg_snap::SnapDialog,
    /// (JD15, ZF-08) the view of each page of the drawing (serial, page: zoom, middle), the page
    /// the canvas showed last
    pub page_views: alloc::vec::Vec<(u32, diagram_core::PageRef, f32, f64, f64)>,
    pub view_page: Option<(u32, diagram_core::PageRef)>,
    /// (JD18) external data: the pane, the data selector, refreshing, linking, asking on
    /// drop, the drawing from text (data_*.rs)
    pub ext: data_pane::ExtState,
    pub dimport: data_import::ImportState,
    pub drefresh: data_refresh::RefreshState,
    pub dlink: data_link::LinkState,
    pub dask: data_ask::AskState,
    pub textdraw: data_textdraw::TextDrawState,
    /// (JD21 layout) the layout commands: live preview, Seitenumbrüche vermeiden, the dialogs
    pub lay: layout_cmds::LayoutCmds,
    /// (JD21 jumps) Verbinder-Verhalten, Linienübergänge anzeigen (jumps_ui.rs)
    pub jumps: jumps_ui::JumpsState,
    /// (JD21 autoconn) the connector automatics: a move's split, the tool "once", marked tiles,
    /// the quick shapes bar
    pub autoconn: autoconn::AutoConn,
}

impl Diagrammer {
    fn new() -> Diagrammer {
        Diagrammer {
            reg: ribbon::Reg::Start,
            backstage: None,
            last_page: backstage::PAGE_NEW,
            template: None,
            catalog: templates::Catalog::default(),
            units_us: 0,
            opt_page: 0,
            canvas: DiagramState::new(),
            panes: panes::initial(),
            opts: Settings::new(options::schema()),
            tool: commands::POINTER,
            view_grid: false,
            page_tab: 0,
            format_tab: 0,
            preview: ribbon::SEL_NONE,
            icons: None,
            opened: None,
            error: None,
            fonts: diagram_view::FontSet::new(),
            open_dlg: Shared::new(0),
            open_path: alloc::string::String::new(),
            pages_menu: false,
            fit_again: false,
            export: export_ui::ExportState::new(),
            print: print_ui::PrintState::new(),
            se: SeState::new(),
            model: edit_canvas::Model::default(),
            view_rulers: true,
            view_breaks: false,
            view_guides: true,
            snap: true,
            guide_drag: None,
            text: None,
            text_req: None,
            save_dlg: Shared::new(0),
            save_path: alloc::string::String::new(),
            clip: None,
            clip_offset: 1,
            stencils: stencils::Stencils::new(),
            designs: design_gallery::Designs::default(),
            design_save: design_store::SaveDialog::default(),
            format: format_menus::FormatState::default(),
            quick: format_quick::QuickTiles::default(),
            structs: structures::Structs::default(),
            layout: layout_ui::LayoutState::default(),
            shell: cmds_shell::ShellState::default(),
            sizepos: pane_sizepos::SizePos::default(),
            data: pane_data::DataState::default(),
            panzoom: pane_panzoom::PanZoom::default(),
            qat: qat::QatState::default(),
            pages: pages::PagesState::default(),
            draw: cmds_draw::DrawState::default(),
            pictures: cmds_picture::PictureState::default(),
            paste: cmds_paste::PasteState::default(),
            layers: layers::LayerState::default(),
            select: select_type::SelectState::default(),
            txt: cmds_text::TextCmds::default(),
            docfile: doc_file::DocFile::default(),
            ask: ask_save::AskSave::default(),
            shape_name: dlg_shape_name::NameDialog::default(),
            snap_grid: true,
            snap_dlg: dlg_snap::SnapDialog::default(),
            ext: data_pane::ExtState::default(),
            dimport: data_import::ImportState::default(),
            drefresh: data_refresh::RefreshState::default(),
            dlink: data_link::LinkState::default(),
            dask: data_ask::AskState::default(),
            textdraw: data_textdraw::TextDrawState::default(),
            lay: layout_cmds::LayoutCmds::default(),
            jumps: jumps_ui::JumpsState::default(),
            autoconn: autoconn::AutoConn::default(),
            page_views: alloc::vec::Vec::new(),
            view_page: None,
        }
    }

    /// the kind of the selection (ribbon::SEL_*): the text being edited, a container, shapes;
    /// without a drawing the shell preview (--shell-preview)
    pub(crate) fn selection_kind(&self) -> u8 {
        if self.preview != ribbon::SEL_NONE {
            return self.preview;
        }
        if self.text.is_some() {
            return ribbon::SEL_TEXT;
        }
        if self.se.sel.is_empty() {
            return ribbon::SEL_NONE;
        }
        if edit_canvas::container_selected(self) { ribbon::SEL_CONTAINER } else { ribbon::SEL_SHAPE }
    }

    /// the drawing's name in the title bar
    pub(crate) fn drawing_name(&self) -> alloc::string::String {
        match &self.opened {
            Some(o) => o.name.clone(),
            None => i18n::tf("Zeichnung{}", &["1"]),
        }
    }

    /// the page tab shown, as an index into the open drawing's tabs
    pub(crate) fn page_index(&self) -> usize {
        self.page_tab.max(0) as usize
    }

    /// the next (1) or previous (-1) page tab
    pub(crate) fn page_step(&mut self, ui: &mut Ui, dir: i32) {
        let n = self.opened.as_ref().map_or(1, |o| o.tabs.len().max(1)) as i32;
        let k = (self.page_tab + dir).rem_euclid(n);
        if k != self.page_tab {
            self.page_tab = k;
            self.canvas.request_fit();
            self.fit_again = true;
            ui.repass();
        }
    }

    /// open the File register on `page`
    pub(crate) fn open_backstage(&mut self, ui: &mut Ui, page: usize) {
        self.backstage = Some(page);
        self.last_page = page;
        self.template = None;
        // (JD13) the own templates are read again
        self.catalog.own_read = false;
        // (JD15, LG-07) Neu offers the region's units first
        self.units_us = (stencils::measure(self) == diagram_io::vsdx::Measure::Us) as i32;
        ui.keytips_off();
        ui.repass();
    }

    /// zoom a step in (1) or out (-1) around the canvas' middle
    pub(crate) fn zoom_step(&mut self, dir: i32) {
        let r = self.canvas.vp.rect;
        self.canvas.vp.step(dir, r[0] as f32 + r[2] as f32 * 0.5, r[1] as f32 + r[3] as f32 * 0.5);
    }

    /// zoom to `percent` around the canvas' middle
    pub(crate) fn set_zoom(&mut self, percent: i32) {
        let r = self.canvas.vp.rect;
        self.canvas.vp.zoom_at(percent as f32 / 100.0, r[0] as f32 + r[2] as f32 * 0.5, r[1] as f32 + r[3] as f32 * 0.5);
    }

    /// the language and the region of the options, applied (also at start)
    pub(crate) fn apply_language(&mut self, ui: &mut Ui) {
        let col = lang::column(self.opts.get(&options::UI_LANG));
        lang::apply(ui, col, self.opts.ini());
        lang::region(self.opts.get(&options::REGION), col);
    }

    /// keep the dock layout in the settings
    pub(crate) fn save_layout(&mut self) {
        let text = self.panes.save().replace('\n', ";");
        self.opts.ini_mut().set("window", "dock", &text);
        self.opts.save();
    }

    fn load_layout(&mut self) {
        if let Some(text) = self.opts.ini().get("window", "dock") {
            if let Some(d) = DockState::load(&text.replace(';', "\n")) {
                self.panes = d;
            }
        }
    }
}

impl App for Diagrammer {
    fn init(&mut self, ui: &mut Ui) -> bool {
        self.icons = Some(icons::Icons::install(ui));
        self.opts = options::open(ui);
        self.load_layout();
        // (JD21 layout) Abstandsoptionen's spacing
        dlg_layout::load_spacing(self);
        self.preview = match plat::arg_value("--shell-preview").as_deref() {
            Some("shape") => ribbon::SEL_SHAPE,
            Some("container") => ribbon::SEL_CONTAINER,
            Some("text") => ribbon::SEL_TEXT,
            _ => ribbon::SEL_NONE,
        };
        self.apply_language(ui);
        let theme = self.opts.get(&options::THEME);
        if theme != 0 && !ui.headless() {
            ui.theme_set(options::theme_mode(theme));
        }
        self.fonts = viewer::fonts();
        stencils::init(self);
        // a drawing named on the command line, else a new empty one (JD7)
        let opened = match viewer::command_line_file() {
            Some(path) => viewer::open(self, ui, &path),
            None => false,
        };
        if !opened && self.preview == ribbon::SEL_NONE {
            // (JD15, DF-15) a file that could not be opened keeps its notice over the new drawing
            let error = self.error.take();
            // (JD15, LG-07) in the region's units
            let us = stencils::measure(self) == diagram_io::vsdx::Measure::Us;
            edit_doc::new_drawing(self, ui, us);
            self.error = error;
        }
        self.units_us = (stencils::measure(self) == diagram_io::vsdx::Measure::Us) as i32;
        // (JD8 rest) --check-commands: which commands still lack a function (cmds_later.rs)
        if plat::args().iter().any(|a| a == "--check-commands") {
            self.error = Some((alloc::format!("Befehlsprüfung: {}", cmds_later::report_text()), alloc::string::String::new()));
        }
        true
    }

    fn frame(&mut self, ui: &mut Ui) {
        if let Some(icons) = &self.icons {
            icons.update(ui); // the theme may have changed (the system, high contrast)
        }
        if let Some((key, ch, fresh)) = self.text_req.take() {
            edit_text::begin_with(self, ui, key, ch, fresh);
        }
        // (JD8 rest) the page tabs follow the pages (also after undo and redo)
        pages::sync(self);
        // (JD8 rest) what the system's clipboard offers (Einfügen, Inhalte einfügen)
        cmds_paste::poll(self, ui);
        commands::states(self, ui);
        // (JD15) the lane commands show in a lane's menu only (ST-05); QuickInfos off (UI-13)
        structures_cff::menu_states(self, ui);
        // (JD21 jumps) Verbinder-Verhalten in a connector's menu only
        jumps_ui::menu_states(self, ui);
        ui.tooltips_set(self.opts.get(&options::TIPS) != 2);
        stencils_panel::states(self, ui);
        shell::frame(self, ui);
        design_gallery::after(self, ui);
        // (JD21 layout) a layout's live preview goes when its row is left; the dialogs
        layout_cmds::after(self, ui);
        dlg_layout::dialogs(self, ui);
        design_store::dialog(self, ui);
        format_menus::after(self, ui);
        export_ui::dialogs(self, ui);
        print_ui::dialogs(self, ui);
        layout_ui::dialog(self, ui);
        cmds_shell::zoom_dialog(self, ui);
        pane_data::define_dialog(self, ui);
        qat::dialog(self, ui);
        pages::dialogs(self, ui);
        // (JD8 rest) the dialogs of pictures, pasting, layers, select by type
        cmds_picture::dialog(self, ui);
        cmds_paste::dialog(self, ui);
        layers::assign_dialog(self, ui);
        layers::props_dialog(self, ui);
        select_type::dialog(self, ui);
        cmds_text::frame(self, ui);
        // (JD16) the question about macros, the file's zoom after the fit
        doc_file::dialog(self, ui);
        doc_file::frame(self, ui);
        // (JD15) Speichern-Rückfrage (UI-12), Shape-Name (SP-01)
        ask_save::dialog(self, ui);
        dlg_shape_name::dialog(self, ui);
        // (JD15) Ausrichten und Kleben
        dlg_snap::dialog(self, ui);
        // (JD15, SP-12) the rows of the selected shape's Actions section for its menu
        cmds_actions::frame(self, ui);
        // (JD18) the data dialogs, the refresh interval
        data_import::dialog(self, ui);
        data_pane::dialog(self, ui);
        data_refresh::dialog(self, ui);
        data_link::dialog(self, ui);
        data_ask::dialog(self, ui);
        data_textdraw::dialog(self, ui);
        // (JD21 jumps) Verbinder-Verhalten
        jumps_ui::dialog(self, ui);
    }

    fn command(&mut self, ui: &mut Ui, id: u32, _arg: u32) -> bool {
        // (JD15, SP-12) a row of the shape's Actions section
        if cmds_actions::command(self, ui, id) {
            return true;
        }
        // (JD15, UI-12) Schließen and the window's close ask first when there are changes
        if ask_save::command(self, ui, id) {
            return true;
        }
        cmds_text::command(self, ui, id) || stencils::command(self, ui, id) || commands::run(self, ui, id)
    }
}

/// a text in the current language with a context after '|' for German words that have two
/// meanings ("Einfügen|Zwischenablage" is Paste, "Einfügen" the register Insert): the catalogue
/// row carries the context, the screen never shows it
pub(crate) fn tc(s: &'static str) -> &'static str {
    let r = i18n::t(s);
    r.split('|').next().unwrap_or(r)
}

ui::main!({
    // (JD9) --write-stencils DIR: the built-in stencils into a folder, no window
    if let Some(dir) = plat::arg_value("--write-stencils") {
        let written = stencils::write_builtin(&dir);
        for p in &written {
            plat::write_stdout(alloc::format!("{p}\n").as_bytes());
        }
        return if written.len() == 2 * stencils::BUILTIN.len() { 0 } else { 1 };
    }
    // (JD13) --write-templates DIR: the built-in templates into a folder, no window
    if let Some(dir) = plat::arg_value("--write-templates") {
        let written = templates::write_builtin(&dir);
        for p in &written {
            plat::write_stdout(alloc::format!("{p}\n").as_bytes());
        }
        return if written.len() == 2 * templates::BUILTIN.len() { 0 } else { 1 };
    }
    i18n::set_catalog(Catalog::from_tsv(include_str!("../locales/ui.tsv")));
    ui::launch(&DESC.translated(), Diagrammer::new(), &style::STYLE)
});
