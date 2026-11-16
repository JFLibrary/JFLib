// JFDiagrammer -- diagrams of the Visio kind: drawings, stencils and templates (.vsdx family)
//
// The app of FEATURES.md and PLAN.md (this folder).  It builds on the public jflib API only
// (ui, and later diagram_core / diagram_io) and moves to a repository of its own later.  Look:
// the NetLab style (STYLE), a ribbon on top, the Shapes sidebar on the left, the drawing canvas
// in the middle, the status bar below.
//
// Stage JD0 (the frame): the window with placeholders, no functions yet:
//   src/main.rs     the entry, the catalogue, the app's state
//   src/shell.rs    the window: register row, ribbon, Shapes sidebar, canvas, status bar
//   src/ribbon.rs   the placeholder groups of the registers Start, Einfügen, Entwurf, Ansicht
//   src/icons.rs    the ribbon icons (JD14: own artwork in icons/*.svg, mkicons.py)
//
// Languages: German source texts, the catalogue locales/ui.tsv with the columns de, en, es,
// it, fr (FEATURES.md 3.17).  The language follows --lang / jfgui_LANG / the system (the
// library negotiates it with the columns; a language without a column gets English).  Texts
// go through t() / tc() at the place they are shown and are never kept, so a switch at run
// time (JD8, the options) shows at once.  Scripts run German unless their #!args say --lang.
//
//   python build_rs.py run JFDiagrammer
//   python build_rs.py test JFDiagrammer          tests/*.uis
//   python build_rs.py i18n JFDiagrammer          the catalogue against the sources
//   build/win/JFDiagrammer.exe --shot build/JFDiagrammer.png --scale 150
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

mod icons;
mod ribbon;
mod shell;

use ui::i18n::{self, Catalog};
use ui::*;

const DESC: AppDesc = AppDesc {
    title: "JFDiagrammer",
    flags: AF_RESIZABLE,
    w: 1280,
    h: 800,
    min_w: 640,
    min_h: 420,
    ..AppDesc::NONE
};

/// the register shown in the ribbon (index into ribbon::TABS)
pub(crate) const TAB_START: i32 = 0;

pub(crate) struct Diagrammer {
    /// the selected register of the ribbon
    tab: i32,
    /// the canvas' view and selection (an empty page until JD4 brings the model)
    canvas: DiagramState,
    /// the ribbon icons' hook (set in init)
    icons: Option<icons::Icons>,
}

impl Diagrammer {
    fn new() -> Diagrammer {
        Diagrammer { tab: TAB_START, canvas: DiagramState::new(), icons: None }
    }
}

impl App for Diagrammer {
    fn init(&mut self, ui: &mut Ui) -> bool {
        self.icons = Some(icons::Icons::install(ui));
        true
    }

    fn frame(&mut self, ui: &mut Ui) {
        if let Some(icons) = &self.icons {
            icons.update(ui); // the theme may have changed (the system, high contrast)
        }
        shell::frame(self, ui);
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
