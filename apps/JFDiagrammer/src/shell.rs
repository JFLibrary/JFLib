// shell -- the main window: register row and ribbon on top, the Shapes sidebar on the left,
// the drawing canvas in the middle, the status bar below (FEATURES.md 3, UI-01)
//
//   frame(app, ui)    the whole window, built on every pass
//
// Stage JD0: placeholders.  The sidebar shows the empty state (no stencil open, SL-09), the
// canvas is an empty ui.diagram with the grid, the status bar the page and the zoom.  The
// widths and ids are the frame JD8 (the shell) and JD9 (the sidebar) fill in.

use alloc::format;
use alloc::vec::Vec;
use ui::i18n::{t, tf};
use ui::*;

use crate::{ribbon, Diagrammer};

const ID_TABS: u32 = 100;
const ID_SIDEBAR: u32 = 400;
const ID_MORE_SHAPES: u32 = 401;
const ID_CANVAS: u32 = 300;
const SIDEBAR_W: i32 = 240; // DIP; JD9 makes it resizable and dockable (SL-01)

/// the main window
pub(crate) fn frame(app: &mut Diagrammer, ui: &mut Ui) {
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 0);
    // the register row: the names in the current language, the selection kept by index
    let names: Vec<&str> = ribbon::TABS.iter().map(|s| t(s)).collect();
    ui.next_size(-1, 30);
    ui.tabs_scrolling(ID_TABS, &mut app.tab, &names);
    ribbon::draw(ui, app.tab);
    ui.separator();
    // sidebar and canvas side by side, they take the height between ribbon and status bar
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_H, 0);
    sidebar(ui);
    canvas(app, ui);
    ui.box_end();
    status(app, ui);
    ui.box_end();
}

// the Shapes sidebar: its title and the empty state (no stencil open yet)
fn sidebar(ui: &mut Ui) {
    ui.next_size(SIDEBAR_W, -1);
    ui.box_begin(ID_SIDEBAR, BOX_V | BOX_BG | pad(12), 8);
    ui.text(TS_SUBTITLE, t("Shapes"));
    ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Keine Schablone geöffnet."));
    ui.next_flags(ST_DISABLED);
    ui.button(ID_MORE_SHAPES, t("Weitere Shapes"));
    ui.tooltip(t("Noch nicht verfügbar"));
    ui.box_end();
    ui.separator(); // in the row: a vertical line
}

// the drawing canvas: an empty diagram (the page and its shapes come with JD4)
fn canvas(app: &mut Diagrammer, ui: &mut Ui) {
    let st = &mut app.canvas;
    // the name screen readers and scripts see (canvas:"Zeichenfläche"), in the current language
    let name = t("Zeichenfläche");
    if st.name != name {
        st.name = name.into();
    }
    let model = DModel { nodes: &[], ports: &[], edges: &[], wps: &[], ver: 0 };
    ui.next_size(-1, -1);
    ui.diagram(ID_CANVAS, st, &model, DG_SELECT, None, 0);
}

// the status bar: the page and the zoom (SB-01, SB-04)
fn status(app: &Diagrammer, ui: &mut Ui) {
    let zoom = (app.canvas.vp.zoom * 100.0 + 0.5) as i32;
    ui.statusbar_begin();
    ui.text(TS_CAPTION, &tf("Seite {} von {}", &["1", "1"]));
    ui.spacer(-1); // the zoom on the right
    ui.text(TS_CAPTION, &tf("{} %", &[&format!("{}", zoom.max(1))]));
    ui.statusbar_end();
}
