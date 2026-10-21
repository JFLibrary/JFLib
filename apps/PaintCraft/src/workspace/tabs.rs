// tabs -- PaintCraft: the image list in the title row (paint.net 5): one thumbnail tab per open
// image, the active one highlighted, a dot for unsaved changes, tooltip = file path (or
// name), close button on the active / hovered tab, middle click closes, drag to reorder,
// Strg+Tab / Strg+Umschalt+Tab, and a "Bilderliste" drop-down when the tabs do not fit.
//                                                                   [apps/PaintCraft/src/tabs.s]
//
// Module contract (app/lifecycle.rs):
//   tabs_init()  tabs_states()  tabs_cmd(id, arg) -> handled   (706-709, 800-849, UIC_TIMER)
//   tabs_frame()        the list; chrome.rs gives its outer box the free width of the title row
// Widget ids: tab i = ID_TAB0 + i (K_TAB, label = document name, value = i, "selected" for the
// active image), close button ID_TAB_CLOSE0 + i (K_BUTTON "Schließen"), list button
// ID_TAB_LISTBTN, list rows tab_list_id(i).
// Thumbnails (DOC_THUMB, compose.rs doc_thumb) are made at once for new tabs and refreshed
// 250 ms after the last change (timer TMR_TABS).  ui.draw_image wants an Rc<Surface>: the
// tabs keep an Rc copy of every document's thumbnail, renewed when its version (SF_AUX)
// changes.

use alloc::rc::Rc;
use alloc::vec::Vec;

use gfx::Surface;
use ui::*;

use crate::document::composite::doc_thumb;
use crate::app::constants::*;
use crate::document::model::{doc_at, doc_count, doc_cur, doc_move, doc_name, doc_set_active, Doc, DocId};
use crate::app::commands::pn_cmd_set;
use crate::app::Pn;

const THUMB_W: i32 = 44; // thumbnail box (DIP)
const THUMB_H: i32 = 30;
const TAB_H: i32 = 40;
const TAB_PAD: i32 = 8; // left + right padding around the thumbnail
const TAB_GAP: i32 = 4;
const CLOSE_DIP: i32 = 16;
const LISTBTN_DIP: i32 = 32;
const THUMB_CELL_DIP: i32 = 4;
const TMR_THUMBS: u32 = TMR_TABS;
const ID_TAB_LIST0: u32 = 1264; // list rows 0..35 (36..63: ID_TAB_LIST1 + i - 36)
const ID_TAB_LIST1: u32 = 1364;
const ID_TAB_LISTBTN: u32 = 1392;
const ID_TAB_LISTPOP: u32 = 1393;
const NDOC: usize = DOC_MAX as usize;

/// the state of tabs.s
pub(crate) struct Tabs {
    /// g_close_hot: document index whose close button is hovered, -1
    close_hot: i32,
    /// g_drag_from: tab being dragged, -1
    drag_from: i32,
    /// g_tab_first: first visible tab (overflow)
    tab_first: i32,
    /// g_tab_end: last drawn tab + 1
    tab_end: i32,
    overflow: bool,
    thumb_sum: u32,
    tab_left: i32,
    tab_top: i32,
    tab_hpx: i32,
    /// g_tabw / g_tabcx / g_tabr: tab widths, centres, right edges (px, this frame)
    tabw: [i32; NDOC],
    tabcx: [i32; NDOC],
    tabr: [i32; NDOC],
    /// the thumbnails as the renderer gets them: (document, its thumb_aux, the copy)
    thumbs: Vec<(DocId, u32, Rc<Surface>)>,
}

impl Default for Tabs {
    fn default() -> Tabs {
        Tabs {
            close_hot: -1,
            drag_from: -1,
            tab_first: 0,
            tab_end: 0,
            overflow: false,
            thumb_sum: u32::MAX,
            tab_left: 0,
            tab_top: 0,
            tab_hpx: 0,
            tabw: [0; NDOC],
            tabcx: [0; NDOC],
            tabr: [0; NDOC],
            thumbs: Vec::new(),
        }
    }
}

/// tabs_init()
pub(crate) fn tabs_init(p: &mut Pn, _ui: &mut Ui) {
    let t = &mut p.tabs;
    t.close_hot = -1;
    t.drag_from = -1;
    t.tab_first = 0;
    t.thumb_sum = u32::MAX;
}

/// tabs_states() -- Nächstes / Vorheriges Bild with more than one image, Bilderliste with one
pub(crate) fn tabs_states(p: &mut Pn, ui: &mut Ui) {
    let n = doc_count(&p.doc);
    let f = if n >= 2 { 0 } else { CF_DISABLED };
    pn_cmd_set(ui, CMD_IMG_NEXT, f);
    pn_cmd_set(ui, CMD_IMG_PREV, f);
    pn_cmd_set(ui, CMD_IMG_LIST, if n != 0 { 0 } else { CF_DISABLED });
}

/// tabs_cmd(id, arg) -> handled
pub(crate) fn tabs_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    match id {
        CMD_TAB_SELECT => {
            if let Some(d) = doc_at(&p.doc, arg as i32) {
                doc_set_active(p, d);
            }
        }
        CMD_TAB_CLOSE => {
            if let Some(d) = doc_at(&p.doc, arg as i32) {
                crate::integrations::file_io::fio_close_doc(p, ui, d);
            }
        }
        CMD_TAB_MOVE => doc_move(p, (arg & 0xFFFF) as i32, (arg >> 16) as i32),
        CMD_TAB_COPY_PATH => {
            if let Some(path) = doc_at(&p.doc, arg as i32).and_then(|d| p.doc.d(d).path.as_deref()) {
                ui.clip_set(CLIP_TEXT, path.as_bytes());
            }
        }
        CMD_IMG_NEXT | CMD_IMG_PREV => {
            let n = doc_count(&p.doc);
            if n != 0 {
                let dir = if id == CMD_IMG_NEXT { 1 } else { -1 };
                let i = (doc_cur(&p.doc).wrapping_add(dir).wrapping_add(n) as u32) % n as u32;
                if let Some(d) = doc_at(&p.doc, i as i32) {
                    doc_set_active(p, d);
                }
            }
        }
        CMD_IMG_LIST => ui.popup_open(ID_TAB_LISTPOP),
        UIC_TIMER if arg == TMR_THUMBS => refresh_thumbs(p, ui),
        _ => return false,
    }
    true
}

// refresh_thumbs() -- redraw every thumbnail whose image changed (same size)
fn refresh_thumbs(p: &mut Pn, ui: &mut Ui) {
    let cell = ui.dp(THUMB_CELL_DIP);
    let mut i = 0;
    while i < doc_count(&p.doc) {
        let d = &p.doc.docs[i as usize];
        i += 1;
        if d.thumb.px.is_empty() || d.ver_px.wrapping_add(d.ver_lay) == d.thumb_ver {
            continue;
        }
        let (id, w, h) = (d.id, d.thumb.w, d.thumb.h);
        doc_thumb(p, id, w, h, cell);
    }
    ui.repass();
}

// tab_list_id(i) -> widget id of row i of the image list flyout
fn tab_list_id(i: u32) -> u32 {
    if i < 36 { i + ID_TAB_LIST0 } else { i + ID_TAB_LIST1 - 36 }
}

// thumb_size(doc) -> (w, h): the image aspect inside THUMB_W x THUMB_H DIP (px)
fn thumb_size(ui: &Ui, d: &Doc) -> (i32, i32) {
    let w = d.w.max(1) as u64;
    let h = d.h.max(1) as u64;
    let bw = ui.dp(THUMB_W) as u32;
    let bh = ui.dp(THUMB_H) as u32;
    // tw = w * box_h / h; if tw > box_w: tw = box_w, th = h * box_w / w
    let tw = (w * bh as u64 / h) as u32;
    let (tw, th) = if tw > bw { (bw, (h * bw as u64 / w) as u32) } else { (tw, bh) };
    ((tw as i32).max(1), (th as i32).max(1))
}

/// tabs_frame() -- the thumbnail tabs (and the list button when they do not fit)
pub(crate) fn tabs_frame(p: &mut Pn, ui: &mut Ui) {
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_CLIP, TAB_GAP);
    let r = ui.box_rect();
    let strip_w = r[2];
    let lbtn = ui.dp(LISTBTN_DIP + TAB_GAP);
    let n = doc_count(&p.doc).min(DOC_MAX);
    let cur = doc_cur(&p.doc);
    // widths, thumbnails, version sum
    let mut sum = 0u32;
    let mut total = 0i32;
    for i in 0..n {
        let d = &p.doc.docs[i as usize];
        sum = sum.wrapping_add(d.ver_px.wrapping_add(d.ver_lay).wrapping_add(i as u32));
        let (tw, th) = thumb_size(ui, d);
        // a missing / resized thumbnail is made now
        if d.thumb.px.is_empty() || d.thumb.w != tw || d.thumb.h != th {
            let id = d.id;
            let cell = ui.dp(THUMB_CELL_DIP);
            doc_thumb(p, id, tw, th, cell);
        }
        let w = ui.dp(TAB_PAD * 2) + tw;
        p.tabs.tabw[i as usize] = w;
        total += w + ui.dp(TAB_GAP);
    }
    // changed images: refresh their thumbnails 250 ms after the last change
    let t = &mut p.tabs;
    if sum != t.thumb_sum {
        t.thumb_sum = sum;
        ui.timer(TMR_THUMBS, 250);
    }
    // overflow: keep the active tab visible, leave room for the list button
    t.overflow = false;
    if total > strip_w {
        t.overflow = true;
        if cur >= 0 {
            if cur < t.tab_first {
                t.tab_first = cur;
            }
            // width of first..cur must fit
            loop {
                let mut w = 0;
                for k in t.tab_first..=cur {
                    w += t.tabw[k as usize];
                }
                if w + lbtn <= strip_w || t.tab_first >= cur {
                    break;
                }
                t.tab_first += 1;
            }
        }
    } else {
        t.tab_first = 0;
    }
    if t.tab_first as u32 >= n as u32 {
        t.tab_first = 0;
    }
    let mut i = p.tabs.tab_first;
    let mut used = 0;
    ui.focus_group_begin(0); // the tabs: one Tab stop, the arrows switch images
    while i < n {
        if p.tabs.overflow && p.tabs.tabw[i as usize] + used + lbtn > strip_w {
            break; // the rest is behind the list button
        }
        tab_item(p, ui, i);
        used += p.tabs.tabw[i as usize] + ui.dp(TAB_GAP);
        i += 1;
    }
    ui.focus_group_end();
    p.tabs.tab_end = i;
    if p.tabs.overflow {
        ui.spacer(-1);
        ui.next_flags(ST_SUBTLE | ST_ICON_ONLY);
        if ui.icon_button(ID_TAB_LISTBTN, IC_CHEV_DOWN, "Bilderliste") & EV_ACT != 0 {
            ui.popup_open(ID_TAB_LISTPOP);
        }
    }
    list_flyout(p, ui);
    tab_drag(p, ui);
    ui.box_end();
}

// list_flyout() -- "Bilderliste": every open image, the active one checked
fn list_flyout(p: &mut Pn, ui: &mut Ui) {
    if ui.flyout_begin(ID_TAB_LISTPOP, FL_BELOW | FL_ALIGN_END) {
        let mut i = 0;
        while i < doc_count(&p.doc) && i < DOC_MAX {
            if doc_cur(&p.doc) == i {
                ui.next_flags(ST_CHECKED);
            }
            if ui.menu_item(tab_list_id(i as u32), doc_name(&p.doc, i)) & EV_ACT != 0 {
                ui.cmd_run(CMD_TAB_SELECT, i as u32);
            }
            i += 1;
        }
    }
    ui.flyout_end();
}

// tab_drag() -- a tab dragged with the left button: insertion marker, move on release
fn tab_drag(p: &mut Pn, ui: &mut Ui) {
    let t = &mut p.tabs;
    let from = t.drag_from;
    if from < 0 {
        return;
    }
    let mx = ui.mouse().0;
    // insertion index: first tab whose centre is right of the pointer
    let mut k = t.tab_first;
    while k < t.tab_end && mx >= t.tabcx[k as usize] {
        k += 1;
    }
    if ui.mbtn() & (1 << BTN_LEFT) == 0 {
        // released: move from -> to
        t.drag_from = -1;
        let to = if k > from { k - 1 } else { k };
        if to != from {
            ui.cmd_run(CMD_TAB_MOVE, (to as u32) << 16 | from as u32);
        }
        return;
    }
    // an accent bar where the tab would land
    let x = if k as u32 > t.tab_first as u32 { t.tabr[(k - 1) as usize] } else { t.tab_left };
    let c = ui.color(T_ACCENT);
    ui.draw_color(c);
    let w = ui.dp(2);
    ui.draw_rect(x, t.tab_top, w, t.tab_hpx);
}

// tab_thumb(doc) -> the renderer's copy of doc's thumbnail (renewed when it was re-rendered)
fn tab_thumb(t: &mut Tabs, d: &Doc) -> Rc<Surface> {
    if let Some(e) = t.thumbs.iter().find(|e| e.0 == d.id && e.1 == d.thumb_aux) {
        return e.2.clone();
    }
    let s = &d.thumb;
    let sf = Rc::new(Surface {
        px: s.px.clone(),
        w: s.w,
        h: s.h,
        stride: s.stride,
        fmt: s.fmt,
        flags: s.flags,
        opac: s.opac,
        blend: s.blend,
    });
    t.thumbs.retain(|e| e.0 != d.id);
    t.thumbs.push((d.id, d.thumb_aux, sf.clone()));
    sf
}

// tab_item(i) -- one tab: background, thumbnail, modified dot, close button
fn tab_item(p: &mut Pn, ui: &mut Ui, i: i32) {
    let iu = i as usize;
    let d = &p.doc.docs[iu];
    let t = &mut p.tabs;
    let sel = doc_cur(&p.doc) == i;
    let (tw, th) = if d.thumb.px.is_empty() { thumb_size(ui, d) } else { (d.thumb.w, d.thumb.h) };
    let th_px = ui.dp(TAB_H);
    let r = ui.place(t.tabw[iu], th_px);
    // remember the geometry for dragging
    t.tabcx[iu] = ((r[2] as u32 >> 1) as i32) + r[0];
    t.tabr[iu] = r[0] + r[2];
    if i == t.tab_first {
        t.tab_left = r[0];
        t.tab_top = r[1];
        t.tab_hpx = r[3];
    }
    let ev = ui.behave(ID_TAB0 + i as u32, HB_FOCUS | if sel { HB_CURRENT } else { 0 });
    // the arrows of the tab group select the image they reach
    if ui.group_arrived(ID_TAB0 + i as u32) && !sel {
        ui.cmd_run(CMD_TAB_SELECT, i as u32);
    }
    if ev & EV_DRAG != 0 && t.drag_from < 0 {
        t.drag_from = i;
        ui.cmd_run(CMD_TAB_SELECT, i as u32); // the dragged image becomes the active one
    }
    // background: selected = control fill with a border, hover = subtle fill
    let bg = if sel {
        Some(if ev & EV_HOVER != 0 { T_CTRL_HOVER } else { T_CTRL })
    } else if ev & EV_HOVER != 0 {
        Some(T_SUBTLE_HOVER)
    } else {
        None
    };
    if let Some(tok) = bg {
        let c = ui.color(tok);
        ui.draw_color(c);
        let rr = ui.dp(4);
        ui.draw_rrect(r[0], r[1], r[2], r[3], rr);
        if sel {
            let c = ui.color(T_CTRL_STROKE);
            ui.draw_color(c);
            let rr = ui.dp(4);
            let hair = ui.hair();
            ui.draw_frame(r[0], r[1], r[2], r[3], rr, hair);
        }
    }
    // thumbnail centred, with a hairline shadow
    let x = ((r[2] - tw) >> 1) + r[0];
    let y = ((r[3] - th) >> 1) + r[1];
    let hair = ui.hair();
    ui.draw_color(0x3000_0000);
    ui.draw_rect(x - hair, y - hair, tw + hair * 2, th + hair * 2);
    if !d.thumb.px.is_empty() {
        let sf = tab_thumb(t, d);
        ui.draw_image(&sf, d.thumb_aux, x, y, tw, th);
    } else {
        ui.draw_color(0xFFFF_FFFF);
        ui.draw_rect(x, y, tw, th);
    }
    ui.focus_ring_draw(ev);
    // semantics and tooltip (the tab rect again: drawing changed nothing of it)
    let st = ui.last_state() | if sel { SS_SELECTED as u32 } else { 0 };
    ui.last_set(r, st);
    ui.sem(K_TAB, ID_TAB0 + i as u32, Some(&d.name), i);
    let tip = d.path.as_deref().unwrap_or(&d.name);
    ui.tooltip(tip);
    if ev & EV_ACT != 0 {
        ui.cmd_run(CMD_TAB_SELECT, i as u32);
    }
    // middle click closes
    if ev & EV_HOVER != 0 && ui.released() & (1 << BTN_MIDDLE) != 0 {
        ui.cmd_run(CMD_TAB_CLOSE, i as u32);
    }
    // close button: on the active tab, while the tab or the button is hovered
    if sel || ev & EV_HOVER != 0 || i == t.close_hot {
        let cs = ui.dp(CLOSE_DIP);
        let m = ui.dp(2);
        let x = r[0] + r[2] - cs - m;
        let y = r[1] + m;
        let cev = ui.hit_rect(ID_TAB_CLOSE0 + i as u32, x, y, cs, cs, 0);
        // remember the hovered close button (keeps it visible when the tab loses the hover)
        if cev & EV_HOVER != 0 {
            t.close_hot = i;
        } else if i == t.close_hot {
            t.close_hot = -1;
        }
        // round button: neutral disc with a border, red while hovered (white glyph)
        let (mut fill, mut glyph) = (ui.color(T_FLYOUT), ui.color(T_TEXT));
        if cev & EV_HOVER != 0 {
            fill = ui.color(T_CLOSE_HOVER);
            glyph = 0xFFFF_FFFF;
        }
        let c = ui.color(T_CTRL_STROKE2);
        ui.draw_color(c);
        ui.draw_rrect(x, y, cs, cs, (cs as u32 >> 1) as i32);
        ui.draw_color(fill);
        let h = ui.hair();
        let inner = cs - h - h;
        ui.draw_rrect(x + h, y + h, inner, inner, (inner as u32 >> 1) as i32);
        ui.draw_color(glyph);
        let s = ui.dp(8) << 8;
        let o = ui.dp(4);
        ui.draw_icon(IC_CANCEL, x + o, y + o, s);
        // semantics of the close button
        ui.last_set([x, y, cs, cs], ui.last_state());
        ui.sem(K_BUTTON, ID_TAB_CLOSE0 + i as u32, Some("Schließen"), 0);
        if cev & EV_ACT != 0 {
            ui.cmd_run(CMD_TAB_CLOSE, i as u32);
        }
        return;
    }
    // unsaved changes: a small dot where the close button appears
    if d.flags & DF_MODIFIED as u32 != 0 {
        let c = ui.color(T_TEXT2);
        ui.draw_color(c);
        let cx = r[0] + r[2] - ui.dp(10);
        let cy = r[1] + ui.dp(10);
        let rad = ui.dp(3) << 8;
        ui.draw_circle(cx << 8, cy << 8, rad, 0);
    }
}
