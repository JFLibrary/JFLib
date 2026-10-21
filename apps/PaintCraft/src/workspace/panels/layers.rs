// wnd_layers -- PaintCraft: the "Ebenen" tool window like paint.net 5 (LAYER-007, 011..013,
// 019): one row per layer of the active image, the top layer first: thumbnail over a
// checkerboard (image aspect), name, visibility check box.  The current layer is selected
// (accent pill + shade); a click selects it (CMD_LAYER_SELECT), a double click opens
// Ebeneneigenschaften, dragging a row moves the layer (CMD_LAYER_REORDER = one history step
// "Ebene verschieben"; the moved layer becomes the current one), the check box toggles the
// visibility (CMD_LAYER_SET_VIS, history "Ebene Sichtbarkeit"), a right click opens the layer
// menu (the layer becomes the current one; the bar's commands with the labels and shortcuts
// of the command table: since U3 the menu may leave the tool window).  Bottom bar: Neue Ebene
// hinzufügen, Ebene löschen, Ebene duplizieren, Nach unten vereinen, Ebene eine Stufe
// höher / tiefer, Eigenschaften (tooltips: the command label + the shortcut).
// Thumbnails are lazy: rendered only for rows in view (lay_thumb, cached in LY_THUMB); a
// layer whose pixels changed keeps its old thumbnail until 250 ms after the last change of
// the image (TMR_WLAY wakes the loop); new / resized thumbnails are made at once.
// Owner: agent A4.
//                                              [apps/PaintCraft/src/wnd_layers.s]
//
//   wlay_init()            window record
//   wlay_frame()           the window while chrome_wnd_open(WND_LAYERS) != 0
//                          (WF_TOOL | WF_RESIZABLE; the close button clears the flag)
//   wlay_reset_pos()       default place (bottom right, below the Verlauf window) next frame
//   wlay_thumb_count()     thumbnails rendered so far (test probe #4393)
// Widget ids: ID_LAYER_LIST (row r = ID_LAYER_LIST + 1 + r, row 0 = top layer; its check box
// part_id(row id, 1)), ID_WL_* (pn_lay.inc).
// ui.draw_image wants an Rc<Surface>: the window keeps an Rc copy of every thumbnail it
// shows, renewed when lay_thumb rendered it again (its version never repeats, U3).

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::Surface;
use ui::*;

use crate::app::constants::*;
use crate::document::model::{Doc, DocId};
use crate::document::layers::lay_rec;
use crate::app::commands::pn_cmd_flags;
use crate::app::Pn;

const WL_W: i32 = 208; // client size (DIP)
const WL_H: i32 = 240;
const WL_HMIN: i32 = 120;
const WL_ROW: i32 = 44; // list row height (DIP)
const TB_W: i32 = 44; // thumbnail box (DIP)
const TB_H: i32 = 34;
const WL_MARGIN: i32 = 12; // default place: from the right edge
const WH_BOTTOM: i32 = 140 + 260 + 30; // the Verlauf window's default bottom (wnd_history.rs)
const SB_H: i32 = 32; // status bar of the main window
const WL_OUTER: i32 = 30; // tool window title bar + border
const THUMB_DELAY: u32 = 250; // ms after the last pixel change
const TIP_CAP: usize = 96;
const NBTN: usize = 7;
const BTN_DIP: i32 = 28; // bottom bar buttons (7 x 28 fit 208 DIP)

// bottom bar: widget id, icon, command
static WL_BTN_TAB: [(u32, u32, u32); NBTN] = [
    (ID_WL_ADD, IC_ADD, CMD_LAYER_ADD),
    (ID_WL_DEL, IC_DELETE, CMD_LAYER_DEL),
    (ID_WL_DUP, IC_COPY, CMD_LAYER_DUP),
    (ID_WL_MERGE, IC_DOWNLOAD, CMD_LAYER_MERGE),
    (ID_WL_UP, IC_UP, CMD_LAYER_UP),
    (ID_WL_DOWN, IC_DOWN, CMD_LAYER_DOWN),
    (ID_WL_PROPS, IC_EDIT, CMD_LAYER_PROPS),
];

// tooltips of the bottom bar (command labels)
static WL_TIP_TEXT: [&str; NBTN] = [
    "Neue Ebene",
    "Ebene löschen",
    "Ebene duplizieren",
    "Nach unten vereinen",
    "Ebene eine Stufe höher",
    "Ebene eine Stufe tiefer",
    "Eigenschaften",
];

// right click on a row: the bottom bar's commands with their full labels and shortcuts (the
// menu may leave the tool window since U3's OS popup windows)
static M_WL_CTX: [Mi; 9] = [
    Mi::Cmd(CMD_LAYER_ADD),
    Mi::Cmd(CMD_LAYER_DEL),
    Mi::Cmd(CMD_LAYER_DUP),
    Mi::Cmd(CMD_LAYER_MERGE),
    Mi::Sep,
    Mi::Cmd(CMD_LAYER_UP),
    Mi::Cmd(CMD_LAYER_DOWN),
    Mi::Sep,
    Mi::Cmd(CMD_LAYER_PROPS),
];

/// a thumbnail as the renderer gets it: (document, layer uid, the layer's thumb_aux it was
/// copied from, the version passed to draw_image, the copy)
struct Thumb {
    doc: DocId,
    uid: u64,
    aux: u32,
    ver: u32,
    sf: Rc<Surface>,
}

/// the state of wnd_layers.s (its g_* globals)
#[derive(Default)]
pub(crate) struct WLay {
    /// g_wl_doc: image shown last frame
    doc: Option<DocId>,
    /// g_wl_rect: WINRECT (persisted by the settings module)
    rect: Shared<WinRect>,
    /// g_wl_sel: list selection = row of the current layer
    sel: i32,
    /// g_wl_cur: DOC_CUR shown last frame
    cur: i32,
    /// g_wl_n: layers shown last frame
    n: i32,
    /// g_wl_scroll: scroll the current row into view
    scroll: bool,
    /// g_wl_view: list height (DIP) of the last pass
    view: i32,
    /// g_wl_vsum: wl_vsum of the image
    vsum: u32,
    /// g_wl_tchg: ui.now() of its last change
    tchg: u32,
    /// g_wl_nthumb: thumbnails rendered (probe)
    nthumb: i32,
    /// g_wl_tver: thumbnail image versions (U3)
    tver: u32,
    /// g_wl_tips: the bottom bar's tooltips (made once)
    tips: Vec<String>,
    /// the renderer's thumbnail copies
    thumbs: Vec<Thumb>,
}

/// wlay_init() -- window record
pub(crate) fn wlay_init(p: &mut Pn, _ui: &mut Ui) {
    let mut r = p.wlay.rect.get();
    r.w = WL_W;
    r.h = WL_H;
    r.flags = 0;
    p.wlay.rect.set(r);
    p.wlay.cur = -1;
}

/// wlay_reset_pos() -- default place on the next frame
pub(crate) fn wlay_reset_pos(p: &mut Pn) {
    let mut r = p.wlay.rect.get();
    r.flags &= !WRF_POS;
    p.wlay.rect.set(r);
    crate::workspace::panels::dock::dock_reset(&mut p.dock, WND_LAYERS);
}

/// wlay_thumb_count()
pub(crate) fn wlay_thumb_count(w: &WLay) -> i32 {
    w.nthumb
}

// wl_place() -- default place: bottom right of the main window, below the Verlauf window
fn wl_place(w: &mut WLay, ui: &Ui) {
    let (cw, ch) = ui.client_size();
    let mut r = w.rect.get();
    r.x = (cw - (WL_W + 2 + WL_MARGIN)).max(0);
    let mut y = ch - (SB_H + WL_MARGIN + WL_OUTER + WL_H);
    if y < WH_BOTTOM + 8 {
        y = WH_BOTTOM + 8;
        // shorter window when the main one is low
        r.h = (ch - (SB_H + WL_MARGIN + WL_OUTER + WH_BOTTOM + 8)).max(WL_HMIN).min(WL_H);
    }
    r.y = y;
    r.flags |= WRF_POS;
    w.rect.set(r);
}

// wl_tips() -- the tooltips "Neue Ebene (Strg+Umschalt+N)" (once)
fn wl_tips(w: &mut WLay, ui: &Ui) {
    if !w.tips.is_empty() {
        return;
    }
    for (k, b) in WL_BTN_TAB.iter().enumerate() {
        let acc = ui.accel_text(b.2);
        let t = if acc.is_empty() {
            String::from(WL_TIP_TEXT[k])
        } else {
            format!("{} ({})", WL_TIP_TEXT[k], acc)
        };
        w.tips.push(crate::app::formatting::fmt_cap(t, TIP_CAP));
    }
}

// wl_vsum(doc) -> sum of the layers' pixel versions (changes when any layer changed)
fn wl_vsum(d: Option<&Doc>) -> u32 {
    let mut s = 0u32;
    if let Some(d) = d {
        for (i, l) in d.layers.iter().enumerate() {
            s = s.wrapping_add(l.ver).wrapping_add(i as u32);
        }
    }
    s
}

/// wlay_frame() -- the window (main window context: from pn_frame's module loop)
pub(crate) fn wlay_frame(p: &mut Pn, ui: &mut Ui) {
    if p.wlay.rect.get().flags & WRF_POS == 0 {
        wl_place(&mut p.wlay, ui);
    }
    wl_tips(&mut p.wlay, ui);
    let doc = crate::document::model::doc_active(&p.doc);
    let dref = doc.and_then(|id| p.doc.get(id));
    let n = dref.map_or(0, |d| d.layers.len() as i32);
    // pixel changes: thumbnails refresh THUMB_DELAY ms after the last one
    let vs = wl_vsum(dref);
    let now = ui.now();
    let w = &mut p.wlay;
    if doc != w.doc {
        w.doc = doc;
        w.vsum = vs;
        w.tchg = now.wrapping_sub(1000); // another image: at once
        w.scroll = true;
    } else if vs != w.vsum {
        w.vsum = vs;
        w.tchg = now;
        ui.timer(TMR_WLAY, THUMB_DELAY + 10);
    }
    // the current layer is the selected row (row 0 = top layer)
    let mut sel = -1;
    if let Some(d) = dref {
        sel = n - 1 - d.cur;
        if d.cur != w.cur || n != w.n {
            w.cur = d.cur;
            w.n = n;
            w.scroll = true;
        }
    }
    w.sel = sel;
    // forget the copies of layers that are gone
    if let Some(d) = dref {
        w.thumbs.retain(|t| t.doc == d.id && d.layers.iter().any(|l| l.uid == t.uid));
    } else {
        w.thumbs.clear();
    }
    if crate::workspace::panels::dock::dock_hidden(&p.dock, WND_LAYERS) {
        return; // no room in the main window (wnd_dock.rs)
    }
    let open = crate::workspace::chrome::chrome_wnd_open(&p.chrome, WND_LAYERS);
    let rect = p.wlay.rect.clone();
    if ui.window_begin(WINID_LAYERS, crate::preferences::localization::tr("Ebenen"), Some(&open), WF_TOOL | WF_RESIZABLE, Some(&rect)) {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_FIXED, 0);
        ui.next_size(-1, -1);
        let mut sel = p.wlay.sel;
        let (first, end) = ui.list_begin(ID_LAYER_LIST, &mut sel, n, WL_ROW as u32 | LF_REORDER);
        if let Some(id) = doc {
            for r in first..end {
                wl_row(p, ui, id, r, n);
            }
        }
        let (ev, v) = ui.list_end(&mut sel);
        p.wlay.sel = sel;
        if doc.is_some() {
            if ev & EV_DRAG != 0 {
                // a row was dropped: rows from / to -> layer indexes (row 0 = top layer)
                let from = n - 1 - (v as u32 & 0xFFFF) as i32;
                let to = n - 1 - (v as u32 >> 16) as i32;
                if to >= 0 && from >= 0 {
                    ui.cmd_run(CMD_LAYER_REORDER, (to as u32) << 16 | from as u32);
                    ui.cmd_run(CMD_LAYER_SELECT, to as u32);
                }
            } else if ev & EV_ACT != 0 && v >= 0 {
                let l = n - 1 - v;
                if l >= 0 {
                    ui.cmd_run(CMD_LAYER_SELECT, l as u32);
                }
            }
        }
        p.wlay.view = ui.px2dip(ui.last_rect()[3]);
        wl_autoscroll(&mut p.wlay, ui);
        ui.separator();
        wl_bar(p, ui);
        ui.box_end();
    }
    ui.window_end();
}

// wl_row(doc, row, n) -- list row `row` (layer n - 1 - row): thumbnail, name, check box
fn wl_row(p: &mut Pn, ui: &mut Ui, doc: DocId, row: i32, n: i32) {
    let li = n - 1 - row;
    let ev = ui.list_item_begin(row);
    // right click: the layer menu (for this layer)
    if ui.context_menu(&M_WL_CTX) & EV_OPEN != 0 && li != p.doc.d(doc).cur {
        ui.cmd_run(CMD_LAYER_SELECT, li as u32);
    }
    if ev & EV_DOUBLE != 0 {
        ui.cmd_run(CMD_LAYER_SELECT, li as u32);
        ui.cmd_run(CMD_LAYER_PROPS, 0);
    }
    if lay_rec(p.doc.d(doc), li).is_some() {
        // thumbnail
        ui.next_size(TB_W, TB_H);
        let r = ui.place(0, 0);
        wl_thumb(p, ui, doc, li, r);
        // name
        let l = lay_rec(p.doc.d(doc), li).expect("layer");
        let name = l.name.clone();
        let mut vis = (l.flags & LYF_VISIBLE as u32) as i32;
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &name);
        // visibility
        let cid = part_id(ID_LAYER_LIST + 1 + row as u32, 1);
        if ui.checkbox(cid, crate::preferences::localization::tr(""), &mut vis) & EV_ACT != 0 {
            ui.cmd_run(CMD_LAYER_SET_VIS, li as u32);
        }
    }
    ui.list_item_end();
}

// wl_thumb(doc, i, rect) -- layer i's thumbnail (image aspect) centred in rect (px) with a
// hairline frame; re-rendered when missing / resized, or when stale and the image has not
// changed for THUMB_DELAY ms
fn wl_thumb(p: &mut Pn, ui: &mut Ui, doc: DocId, i: i32, r: [i32; 4]) {
    let hair = ui.hair();
    let bw = (r[2] - hair - hair).max(1);
    let bh = (r[3] - hair - hair).max(1);
    // image aspect inside the box
    let (dw, dh) = {
        let d = p.doc.d(doc);
        (d.w as i64, d.h as i64)
    };
    let (mut tw, mut th) = (bw, bh);
    if dw * bh as i64 >= dh * bw as i64 {
        if dw != 0 {
            th = (dh * bw as i64 / dw) as i32; // wide: th = bw * H / W
        }
    } else if dh != 0 {
        tw = (dw * bh as i64 / dh) as i32; // tall: tw = bh * W / H
    }
    let tw = tw.max(1);
    let th = th.max(1);
    // render when needed
    let (render, uid) = {
        let Some(l) = lay_rec(p.doc.d(doc), i) else { return };
        let render = match &l.thumb {
            None => true,
            Some(t) if t.px.is_empty() || t.w != tw || t.h != th => true,
            Some(_) => l.ver.wrapping_add(1) != l.thumb_ver && ui.now().wrapping_sub(p.wlay.tchg) >= THUMB_DELAY,
        };
        (render, l.uid)
    };
    if render {
        let cell = ui.dp(4);
        let made = crate::document::composite::lay_thumb(p, doc, i, tw, th, cell).is_some();
        p.wlay.nthumb += 1;
        if made {
            // (U3) a version that never repeats
            p.wlay.tver = p.wlay.tver.wrapping_add(0x10000);
        }
    }
    // frame (hairline shadow), then the image
    let x = ((r[2] - tw) >> 1) + r[0];
    let y = ((r[3] - th) >> 1) + r[1];
    ui.draw_color(0x4000_0000);
    ui.draw_rect(x - hair, y - hair, tw + hair * 2, th + hair * 2);
    let Some(l) = lay_rec(p.doc.d(doc), i) else { return };
    let Some(t) = l.thumb.as_deref() else { return };
    if t.px.is_empty() {
        return;
    }
    let aux = l.thumb_aux;
    let w = &mut p.wlay;
    let k = match w.thumbs.iter().position(|e| e.doc == doc && e.uid == uid) {
        Some(k) if w.thumbs[k].aux == aux => k,
        k => {
            if !render {
                w.tver = w.tver.wrapping_add(0x10000);
            }
            let e = Thumb {
                doc,
                uid,
                aux,
                ver: w.tver,
                sf: Rc::new(Surface {
                    px: t.px.clone(),
                    w: t.w,
                    h: t.h,
                    stride: t.stride,
                    fmt: t.fmt,
                    flags: t.flags,
                    opac: t.opac,
                    blend: t.blend,
                }),
            };
            match k {
                Some(k) => {
                    w.thumbs[k] = e;
                    k
                }
                None => {
                    w.thumbs.push(e);
                    w.thumbs.len() - 1
                }
            }
        }
    };
    let e = &w.thumbs[k];
    ui.draw_image(&e.sf, e.ver, x, y, tw, th);
}

// wl_autoscroll() -- after the current layer / image changed: its row into view (once the
// list has a size)
fn wl_autoscroll(w: &mut WLay, ui: &mut Ui) {
    if !w.scroll {
        return;
    }
    let view = w.view;
    if view <= 0 {
        return;
    }
    w.scroll = false;
    if w.sel < 0 {
        return;
    }
    let top = w.sel.wrapping_mul(WL_ROW); // top of the row (DIP)
    let (_, sy) = ui.scroll_pos(ID_LAYER_LIST);
    let y = if top < sy {
        top
    } else {
        let bot = top + WL_ROW;
        if bot <= sy + view {
            return;
        }
        bot - view
    };
    ui.scroll_to(ID_LAYER_LIST, -1, y);
    ui.repass();
}

// wl_bar() -- the bottom bar: seven subtle icon buttons for the Ebenen commands
fn wl_bar(p: &mut Pn, ui: &mut Ui) {
    ui.box_begin(0, BOX_H | BOX_XCENTER | PAD_4, 0);
    ui.focus_group_begin(0); // one Tab stop, the arrows move inside
    for (k, b) in WL_BTN_TAB.iter().enumerate() {
        if pn_cmd_flags(ui, b.2) & CF_DISABLED != 0 {
            ui.next_flags(ST_DISABLED);
        }
        ui.next_size(BTN_DIP, BTN_DIP);
        if ui.icon_button(b.0, b.1, &p.wlay.tips[k]) & EV_ACT != 0 {
            ui.cmd_run(b.2, 0);
        }
    }
    ui.focus_group_end();
    ui.box_end();
}

/// wlay_rect() -> the WINRECT of the window (settings.rs saves / restores it)
pub(crate) fn wlay_rect(w: &WLay) -> Shared<WinRect> {
    w.rect.clone()
}
