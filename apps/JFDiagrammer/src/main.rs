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
//   build.rs, build_steps.rs, stencils/   (JD9) the stencil sources and their build into .vssx
//                     when the app is built (README there; build_rs.py's derived builds for the
//                     browser and Android run build_steps.rs too)
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
// puts DIR in front of the name the export's save dialog proposes (JD12).
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

mod backstage;
mod commands;
mod design_gallery;
mod design_store;
mod edit_canvas;
mod edit_cmds;
mod edit_doc;
mod edit_text;
mod export_ui;
mod format_cells;
mod format_menus;
mod format_pane;
mod icons;
mod lang;
mod options;
mod panes;
mod print_layout;
mod print_ui;
mod ribbon;
mod shell;
mod stencils;
mod stencils_drop;
mod stencils_panel;
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
    /// Neu: the template whose preview is shown
    pub template: Option<usize>,
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
    /// (JD4) the drawing shown, read-only (None: the empty drawing)
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
}

impl Diagrammer {
    fn new() -> Diagrammer {
        Diagrammer {
            reg: ribbon::Reg::Start,
            backstage: None,
            last_page: backstage::PAGE_NEW,
            template: None,
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
        ui.keytips_off();
        ui.repass();
    }

    /// Neu › Leere Zeichnung: a new empty drawing
    pub(crate) fn new_blank(&mut self, ui: &mut Ui) {
        self.template = None;
        edit_cmds::new_drawing(self, ui);
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
            edit_doc::new_drawing(self, ui, false);
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
        commands::states(self, ui);
        stencils_panel::states(self, ui);
        shell::frame(self, ui);
        design_gallery::after(self, ui);
        design_store::dialog(self, ui);
        format_menus::after(self, ui);
        export_ui::dialogs(self, ui);
        print_ui::dialogs(self, ui);
    }

    fn command(&mut self, ui: &mut Ui, id: u32, _arg: u32) -> bool {
        stencils::command(self, ui, id) || commands::run(self, ui, id)
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
    i18n::set_catalog(Catalog::from_tsv(include_str!("../locales/ui.tsv")));
    ui::launch(&DESC.translated(), Diagrammer::new(), &style::STYLE)
});
