// wnd_history -- PaintCraft: the "Verlauf" tool window (HIST-005..007): the entries of the
// active document with icons (the first one "Neues Bild" / "Datei öffnen"), the current
// state selected, redo entries greyed (like paint.net), a click jumps there (keys stay with
// the main window: Strg+Z / Strg+Y; a tool window only takes the keyboard for text boxes),
// bottom bar Zurück zum Anfang / Rückgängig / Wiederholen / Vor bis zum Ende.
// History messages (out of memory, dropped steps) show above the list until closed.
// Owner: agent A2.
//                                              [apps/PaintCraft/src/wnd_history.s]
//
//   whist_init()           window record
//   whist_frame()          the window while chrome_wnd_open(WND_HISTORY) != 0
//                          (WF_TOOL | WF_RESIZABLE; the close button clears the flag)
//   whist_reset_pos()      default place (top right of the main window) on the next frame
// Widget ids: ID_HIST_LIST (rows ID_HIST_LIST + 1 + i), ID_WH_* below.

use ui::*;

use crate::app::constants::*;
use crate::document::model::DocId;
use crate::document::history::{hist_can_redo, hist_can_undo, hist_count, hist_entry_icon, hist_entry_name, hist_pos};
use crate::app::formatting::fmt_cap;
use crate::app::Pn;

const ID_WH_REWIND: u32 = ID_WHIST_FIRST + 1; // Zurück zum Anfang (Zum Anfang)
const ID_WH_UNDO: u32 = ID_WHIST_FIRST + 2; // Rückgängig
const ID_WH_REDO: u32 = ID_WHIST_FIRST + 3; // Wiederholen
const ID_WH_FFWD: u32 = ID_WHIST_FIRST + 4; // Vor bis zum Ende (Zum Ende)
const ID_WH_MSGX: u32 = ID_WHIST_FIRST + 5; // close button of the message
const WH_W: i32 = 208; // client size (DIP)
const WH_H: i32 = 260;
const WH_ROW: i32 = 28; // list row height (DIP)
const WH_ICON: i32 = 16;
const WH_Y: i32 = 140; // default place: below the tool bars,
const WH_MARGIN: i32 = 12; // this far from the right edge
const WRAP_CAP: usize = 256; // longest line of a wrapped message (bytes)

/// the state of wnd_history.s (its g_* globals)
#[derive(Default)]
pub(crate) struct WHist {
    /// g_wh_doc: document shown last frame
    doc: Option<DocId>,
    /// g_wh_rect: WINRECT (persisted by the settings module)
    rect: Shared<WinRect>,
    /// g_wh_sel: list selection = current entry (pos - 1)
    sel: i32,
    /// g_wh_ver: DOC_VER_HIST shown last frame
    ver: u32,
    /// g_wh_scroll: scroll the current entry into view
    scroll: bool,
    /// g_wh_view: list height (DIP) of the last pass
    view: i32,
}

/// whist_init() -- window record
pub(crate) fn whist_init(p: &mut Pn, _ui: &mut Ui) {
    let mut r = p.whist.rect.get();
    r.w = WH_W;
    r.h = WH_H;
    r.flags = 0;
    p.whist.rect.set(r);
}

/// whist_reset_pos() -- default place on the next frame
pub(crate) fn whist_reset_pos(p: &mut Pn) {
    let mut r = p.whist.rect.get();
    r.flags &= !WRF_POS;
    p.whist.rect.set(r);
    crate::workspace::panels::dock::dock_reset(&mut p.dock, WND_HISTORY);
}

/// whist_frame() -- the window (main window context: from pn_frame's module loop)
pub(crate) fn whist_frame(p: &mut Pn, ui: &mut Ui) {
    let mut r = p.whist.rect.get();
    if r.flags & WRF_POS == 0 {
        r.x = (ui.client_size().0 - (WH_W + 2 + WH_MARGIN)).max(0);
        r.y = WH_Y;
        r.flags |= WRF_POS;
        p.whist.rect.set(r);
    }
    let doc = crate::document::model::doc_active(&p.doc);
    let mut n = 0;
    let mut pos = 0;
    if let Some(id) = doc {
        let d = p.doc.d(id);
        n = hist_count(d);
        pos = hist_pos(d);
        let ver = d.ver_hist;
        if p.whist.doc != Some(id) || ver != p.whist.ver {
            // new document or new history state: scroll the current entry into view
            p.whist.doc = Some(id);
            p.whist.ver = ver;
            p.whist.scroll = true;
        }
    }
    p.whist.sel = pos - 1;
    if crate::workspace::panels::dock::dock_hidden(&p.dock, WND_HISTORY) {
        return; // no room in the main window (wnd_dock.rs)
    }
    let open = crate::workspace::chrome::chrome_wnd_open(&p.chrome, WND_HISTORY);
    let rect = p.whist.rect.clone();
    if ui.window_begin(WINID_HISTORY, crate::preferences::localization::tr("Verlauf"), Some(&open), WF_TOOL | WF_RESIZABLE, Some(&rect)) {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_FIXED, 0);
        wh_message(p, ui);
        ui.next_size(-1, -1);
        let mut sel = p.whist.sel;
        let (first, end) = ui.list_begin(ID_HIST_LIST, &mut sel, n, WH_ROW as u32);
        for i in first..end {
            wh_row(p, ui, doc, i, pos);
        }
        let (ev, s) = ui.list_end(&mut sel);
        p.whist.sel = s;
        if ev & EV_ACT != 0 {
            ui.cmd_run(CMD_HIST_JUMP, p.whist.sel as u32); // a click / key selected an entry
        }
        p.whist.view = ui.px2dip(ui.last_rect()[3]);
        wh_autoscroll(p, ui, pos);
        ui.separator();
        wh_bar(p, ui, doc);
        ui.box_end();
    }
    ui.window_end();
}

// wh_row(doc, i, pos) -- list row i: icon + name, greyed when i >= pos (redo entry)
fn wh_row(p: &Pn, ui: &mut Ui, doc: Option<DocId>, i: i32, pos: i32) {
    ui.list_item_begin(i);
    ui.next_size(-1, -1);
    let mut f = BOX_H | BOX_XCENTER;
    if i >= pos {
        f |= BOX_DISABLED;
    }
    ui.box_begin(0, f, 8);
    let (icon, name) = match doc.and_then(|id| p.doc.get(id)) {
        Some(d) => (hist_entry_icon(d, i), hist_entry_name(d, i)),
        None => (0, ""),
    };
    ui.icon(icon, WH_ICON);
    ui.text(TS_BODY, name);
    ui.box_end();
    ui.list_item_end();
}

// wh_autoscroll(pos) -- after a history change: scroll the current entry into view once the
// list has a size
fn wh_autoscroll(p: &mut Pn, ui: &mut Ui, pos: i32) {
    if !p.whist.scroll {
        return;
    }
    let view = p.whist.view;
    if view <= 0 {
        return; // not laid out yet: keep the request
    }
    p.whist.scroll = false;
    let top = pos - 1;
    if top < 0 {
        return;
    }
    let top = top.wrapping_mul(WH_ROW); // top of the current row (DIP)
    let (_, sy) = ui.scroll_pos(ID_HIST_LIST);
    let y = if top < sy {
        top
    } else {
        let bot = top + WH_ROW;
        if bot <= sy + view {
            return;
        }
        bot - view
    };
    ui.scroll_to(ID_HIST_LIST, -1, y);
    ui.repass();
}

// wh_bar(doc) -- the bottom bar
fn wh_bar(p: &Pn, ui: &mut Ui, doc: Option<DocId>) {
    let (un, re) = match doc.and_then(|id| p.doc.get(id)) {
        Some(d) => (hist_can_undo(d), hist_can_redo(d)),
        None => (false, false),
    };
    ui.box_begin(0, BOX_H | BOX_XCENTER | PAD_4, 4);
    ui.focus_group_begin(0); // one Tab stop, the arrows move inside
    wh_btn(ui, ID_WH_REWIND, IC_PREVIOUS, "Zurück zum Anfang", un, CMD_HIST_REWIND);
    wh_btn(ui, ID_WH_UNDO, IC_UNDO, "Einen Schritt zurück", un, CMD_UNDO);
    wh_btn(ui, ID_WH_REDO, IC_REDO, "Einen Schritt vor", re, CMD_REDO);
    wh_btn(ui, ID_WH_FFWD, IC_NEXT, "Vor bis zum Ende", re, CMD_HIST_FFWD);
    ui.focus_group_end();
    ui.box_end();
}

// wh_btn(id, icon, tooltip, enabled, cmd) -- subtle icon button that runs cmd
fn wh_btn(ui: &mut Ui, id: u32, icon: u32, tip: &str, on: bool, cmd: u32) {
    if !on {
        ui.next_flags(ST_DISABLED);
    }
    if ui.icon_button(id, icon, tip) & EV_ACT != 0 {
        ui.cmd_run(cmd, 0);
    }
}

// wh_message() -- the last history message (icon, wrapped text, close button)
fn wh_message(p: &mut Pn, ui: &mut Ui) {
    let Some((msg, kind)) = crate::document::history::hist_message(&p.hist) else { return };
    let msg = alloc::string::String::from(msg);
    let icon = if kind == IB_ERROR { IC_ERROR } else { IC_WARNING };
    ui.box_begin(0, BOX_H | BOX_XSTART | BOX_LAYER | PAD_8, 8);
    ui.icon(icon, WH_ICON);
    ui.next_size(-1, 0); // the text wraps in its own column
    ui.box_begin(0, BOX_V, 0);
    // window width - icon, close, paddings
    let w = (p.whist.rect.get().w - (WH_ICON + 40 + 4 * 8)).max(40);
    let maxw = ui.dp(w);
    wh_wrap(ui, &msg, maxw);
    ui.box_end();
    if ui.icon_button(ID_WH_MSGX, IC_CANCEL, "Schließen") & EV_ACT != 0 {
        crate::document::history::hist_message_clear(&mut p.hist);
        ui.repass();
    }
    ui.box_end();
}

// wh_wrap(msg, maxw) -- msg as TS_CAPTION lines of at most maxw px, broken at spaces (ui.text
// has no word wrap); a single word longer than a line stays whole
fn wh_wrap(ui: &mut Ui, msg: &str, maxw: i32) {
    let b = msg.as_bytes();
    let mut s = 0usize; // start of the current line
    while s < b.len() {
        let mut sp: Option<usize> = None; // last space that may end the line
        let mut e = s;
        loop {
            while e < b.len() && b[e] != b' ' {
                e += 1;
            }
            ui.ts_font(TS_CAPTION);
            let w = ui.text.width_n(&b[s..], e - s);
            if w > maxw {
                if let Some(k) = sp {
                    // too wide: the line ends at the last space
                    ww_emit(ui, &b[s..k]);
                    s = k + 1;
                    break;
                }
            }
            if e >= b.len() {
                ww_emit(ui, &b[s..e]);
                return;
            }
            sp = Some(e);
            e += 1;
        }
    }
}

// ww_emit(line) -- one line of the wrapped message
fn ww_emit(ui: &mut Ui, line: &[u8]) {
    let t = alloc::string::String::from_utf8_lossy(line).into_owned();
    ui.text(TS_CAPTION, &fmt_cap(t, WRAP_CAP));
}

/// whist_rect() -> the WINRECT of the window (settings.rs saves / restores it)
pub(crate) fn whist_rect(w: &WHist) -> Shared<WinRect> {
    w.rect.clone()
}
