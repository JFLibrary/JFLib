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
// Test aids: --shell-preview shape|container|text stands in for a selection of that kind until
// JD7 brings real ones (context registers, the shape's menu, the measures in the status bar);
// --config PATH keeps the settings in that file (headless runs keep them in memory otherwise).
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
mod icons;
mod lang;
mod options;
mod panes;
mod ribbon;
mod shell;

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
        }
    }

    /// the kind of the selection (ribbon::SEL_*): until JD7 only the shell preview
    pub(crate) fn selection_kind(&self) -> u8 {
        self.preview
    }

    /// the drawing's name in the title bar
    pub(crate) fn drawing_name(&self) -> alloc::string::String {
        i18n::tf("Zeichnung{}", &["1"])
    }

    /// open the File register on `page`
    pub(crate) fn open_backstage(&mut self, ui: &mut Ui, page: usize) {
        self.backstage = Some(page);
        self.last_page = page;
        self.template = None;
        ui.keytips_off();
        ui.repass();
    }

    /// Neu › Leere Zeichnung: back to the (empty) drawing
    pub(crate) fn new_blank(&mut self, ui: &mut Ui) {
        self.backstage = None;
        self.template = None;
        self.canvas.request_fit();
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
        true
    }

    fn frame(&mut self, ui: &mut Ui) {
        if let Some(icons) = &self.icons {
            icons.update(ui); // the theme may have changed (the system, high contrast)
        }
        commands::states(self, ui);
        shell::frame(self, ui);
    }

    fn command(&mut self, ui: &mut Ui, id: u32, _arg: u32) -> bool {
        commands::run(self, ui, id)
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
    i18n::set_catalog(Catalog::from_tsv(include_str!("../locales/ui.tsv")));
    ui::launch(&DESC.translated(), Diagrammer::new(), &style::STYLE)
});
