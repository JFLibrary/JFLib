// wnd_tools -- PaintCraft: the "Werkzeuge" tool window (paint.net 5 look: two columns of
// tool icons in the TOOL_* order, the active tool highlighted, tooltips "Name (Taste)").
// Owner: agent A2.
//                                              [apps/PaintCraft/src/wnd_tools.s]
//
//   wtools_init()          size of the window record
//   wtools_frame()         the window while chrome_wnd_open(WND_TOOLS) != 0
//                          (WF_TOOL | WF_AUTOHEIGHT; the close button clears the flag): two
//                          columns of square buttons, ten columns in two rows while the
//                          main window is low (dock_horiz, wnd_dock.rs places the window)
//   wtools_reset_pos()     the next frame puts the window at its default place again
// A click selects the tool through the tool module (ui.cmd_run(CMD_TOOL0 + i)); the icons
// come from the command table (cmds.rs).  Widget ids: ID_WTOOLS_FIRST + TOOL_*.

use ui::*;

use crate::app::constants::*;
use crate::app::Pn;

const WT_BW: i32 = 32; // square buttons (DIP)
const WT_BH: i32 = 32;
const WT_GAP: i32 = 2; // between buttons
const WT_PAD: u32 = 4; // around the grid
const WT_W: i32 = 2 * WT_BW + WT_GAP + 2 * WT_PAD as i32; // two columns
const WT_W_ROWS: i32 = 10 * WT_BW + 9 * WT_GAP + 2 * WT_PAD as i32; // ten columns, two rows
const WT_X: i32 = 8; // default place (DIP from the main client origin):
const WT_Y: i32 = 140; // left edge, below the tool bars

// tooltips (tool name + key), TOOL_* order
static WT_TIPS: [&str; 19] = [
    "Rechteckige Auswahl (S)",
    "Pixel verschieben (M)",
    "Lasso-Auswahl (S)",
    "Auswahl verschieben (M)",
    "Elliptische Auswahl (S)",
    "Zoomen (Z)",
    "Zauberstab (S)",
    "Ansicht verschieben (H)",
    "Farbeimer (F)",
    "Farbverlauf (G)",
    "Farbpinsel (B)",
    "Radierer (E)",
    "Zeichenstift (P)",
    "Farbauswahl (K)",
    "Klonen (Stempel) (L)",
    "Neu färben (R)",
    "Text (T)",
    "Linie / Kurve (O)",
    "Formen (O)",
];

/// the state of wnd_tools.s (its g_* globals)
#[derive(Default)]
pub(crate) struct WTools {
    /// g_wt_rect: WINRECT (persisted by the settings module)
    rect: Shared<WinRect>,
    /// g_wt_cur: copy of tool_cur() for ui.tool_button
    cur: i32,
}

/// wtools_init() -- window record (width fixed, height = content)
pub(crate) fn wtools_init(p: &mut Pn, _ui: &mut Ui) {
    let mut r = p.wtools.rect.get();
    r.w = WT_W;
    r.h = 0;
    r.flags = 0;
    p.wtools.rect.set(r);
}

/// wtools_reset_pos() -- default place on the next frame
pub(crate) fn wtools_reset_pos(p: &mut Pn) {
    let mut r = p.wtools.rect.get();
    r.flags &= !WRF_POS;
    p.wtools.rect.set(r);
    crate::workspace::panels::dock::dock_reset(&mut p.dock, WND_TOOLS);
}

/// wtools_frame() -- the window: ten rows of two tool buttons
pub(crate) fn wtools_frame(p: &mut Pn, ui: &mut Ui) {
    let mut r = p.wtools.rect.get();
    if r.flags & WRF_POS == 0 {
        r.x = WT_X;
        r.y = WT_Y;
        r.flags |= WRF_POS;
    }
    // two columns, or ten in two rows when the main window is low
    let (cols, w) = if crate::workspace::panels::dock::dock_horiz(&p.dock) { (10, WT_W_ROWS) } else { (2, WT_W) };
    r.w = w;
    p.wtools.rect.set(r);
    if crate::workspace::panels::dock::dock_hidden(&p.dock, WND_TOOLS) {
        return;
    }
    p.wtools.cur = crate::tools::tool_cur(&p.tool);
    let open = crate::workspace::chrome::chrome_wnd_open(&p.chrome, WND_TOOLS);
    // (two columns: too narrow for the title)
    let title = if cols == 2 { "" } else { "Werkzeuge" };
    let rect = p.wtools.rect.clone();
    if ui.window_begin(WINID_TOOLS, title, Some(&open), WF_TOOL | WF_AUTOHEIGHT, Some(&rect)) {
        ui.box_begin(0, BOX_V | BOX_XSTART | (WT_PAD << PAD_SHIFT), WT_GAP);
        let mut i = 0;
        while i < TOOL_COUNT {
            ui.box_begin(0, BOX_H, WT_GAP);
            let mut c = 0;
            while c < cols && i < TOOL_COUNT {
                wt_button(p, ui, i);
                i += 1;
                c += 1;
            }
            ui.box_end();
        }
        ui.box_end();
    }
    ui.window_end();
}

// wt_button(i) -- the button of tool i (checked while it is the current tool)
fn wt_button(p: &mut Pn, ui: &mut Ui, i: i32) {
    if i as u32 >= TOOL_COUNT as u32 {
        return;
    }
    ui.next_size(WT_BW, WT_BH);
    let icon = p.cmds.iter().find(|c| c.id == CMD_TOOL0 + i as u32).map_or(0, |c| c.icon);
    let ev = ui.tool_button(ID_WTOOLS_FIRST + i as u32, icon, WT_TIPS[i as usize], &mut p.wtools.cur, i);
    if ev & EV_ACT != 0 {
        ui.cmd_run(CMD_TOOL0 + i as u32, 0);
    }
}

/// wtools_rect() -> the WINRECT of the window (settings.rs saves / restores it)
pub(crate) fn wtools_rect(w: &WTools) -> Shared<WinRect> {
    w.rect.clone()
}
