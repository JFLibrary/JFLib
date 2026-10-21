// layers_ui -- the "Ebenen" side panel: thumbnails, selection, visibility, actions
//                                                         [apps/paint/src/layers_ui.s]
//
//   layers_panel(p, ui)    draw the panel (call inside the canvas row, after the canvas)
//   layers_visible(l)      true while the panel is shown
//   layers_toggle(l)       show / hide the panel
//
// Thumbnails are regenerated when the document changed (doc_version).

use alloc::format;
use alloc::rc::Rc;

use gfx::Surface;
use ui::*;

use crate::consts::*;
use crate::doc::{doc_h, doc_mark_all, doc_version, doc_w};
use crate::imageops::layer_toggle;
use crate::{fmt_cap, Kritzel};

const ID_LROW0: u32 = 1600; // + layer index: row
const ID_LVIS0: u32 = 1620; // + layer index: visibility button
const ID_LADD: u32 = 1640;
const ID_LDEL: u32 = 1641;
const ID_LDUP: u32 = 1642;
const ID_LMERGE: u32 = 1643;
const ID_LUP: u32 = 1644;
const ID_LDOWN: u32 = 1645;
const ID_LOPAC: u32 = 1646;
const THUMB_W: i32 = 64; // DIP
const THUMB_H: i32 = 44;
const RANGE_LOP: Range = Range::new(0, 100, 1, 0);

/// the panel's state (the .data / .bss of layers_ui.s)
pub(crate) struct LayersUi {
    show: bool,
    lop: i32,
    thumb_version: u32,
    thumb_dpi: i32,
    thumbs: [Option<Rc<Surface>>; MAX_LAYERS],
}

impl Default for LayersUi {
    fn default() -> LayersUi {
        LayersUi { show: false, lop: 100, thumb_version: u32::MAX, thumb_dpi: 0, thumbs: Default::default() }
    }
}

/// layers_visible() -> true while the panel is shown
pub(crate) fn layers_visible(l: &LayersUi) -> bool {
    l.show
}

/// layers_toggle() -- show / hide the panel
pub(crate) fn layers_toggle(l: &mut LayersUi) {
    l.show = !l.show;
}

// thumbs_update() -- rescale every layer into its thumbnail when the document changed
fn thumbs_update(p: &mut Kritzel, ui: &mut Ui) {
    let v = doc_version(&p.doc);
    let l = &mut p.lay;
    if v == l.thumb_version && ui.dpi() == l.thumb_dpi {
        return;
    }
    l.thumb_version = v;
    l.thumb_dpi = ui.dpi();
    // thumbnail size keeps the aspect ratio inside THUMB_W x THUMB_H DIP
    let mut tw = ui.dp(THUMB_W);
    let mut th = ui.dp(THUMB_H);
    let (w, h) = (doc_w(&p.doc), doc_h(&p.doc));
    // tw = THUMB_W, th = h * tw / w; if th > THUMB_H: th = THUMB_H, tw = w * th / h
    let t = h.wrapping_mul(tw) / w;
    if t > th {
        tw = w.wrapping_mul(th) / h;
    } else {
        th = t;
    }
    let tw = tw.max(1);
    let th = th.max(1);
    for i in 0..MAX_LAYERS {
        l.thumbs[i] = None;
        if i < p.doc.nlayers as usize {
            l.thumbs[i] = img_core::ops::scale(&p.doc.layers[i], tw, th, true).ok().map(Rc::new);
        }
    }
}

/// layers_panel() -- right side panel
pub(crate) fn layers_panel(p: &mut Kritzel, ui: &mut Ui) {
    if !p.lay.show {
        return;
    }
    thumbs_update(p, ui);
    ui.next_size(200, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_BLOCK | PAD_8, 4);
    // header: title + add
    ui.box_begin(0, BOX_H | BOX_XCENTER, 4);
    ui.next_size(-1, 0);
    ui.text(TS_BODY_STRONG, "Ebenen");
    if ui.icon_button(ID_LADD, IC_ADD, "Ebene hinzufügen") & EV_ACT != 0 {
        ui.cmd_run(CMD_LAYER_ADD, 0);
    }
    ui.box_end();
    // rows, top layer first
    let mut i = p.doc.nlayers;
    while i > 0 {
        i -= 1;
        layer_row(p, ui, i);
    }
    ui.spacer(-1);
    // opacity of the current layer (percent)
    let cur = p.doc.cur_layer as usize;
    p.lay.lop = (p.doc.layer_opac[cur] as i32 * 100 + 127) / 255;
    let t = fmt_cap(format!("Deckkraft: {} %", p.lay.lop), 32);
    ui.text(TS_CAPTION, &t);
    if ui.slider(ID_LOPAC, &mut p.lay.lop, &RANGE_LOP) & EV_ACT != 0 {
        p.doc.layer_opac[cur] = (p.lay.lop.wrapping_mul(255).wrapping_add(50) / 100) as u8;
        doc_mark_all(&mut p.doc);
    }
    // actions
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_CENTER, 2);
    action_button(ui, ID_LUP, IC_UP, "Nach oben", CMD_LAYER_UP);
    action_button(ui, ID_LDOWN, IC_DOWN, "Nach unten", CMD_LAYER_DOWN);
    action_button(ui, ID_LDUP, IC_COPY, "Duplizieren", CMD_LAYER_DUP);
    if p.doc.cur_layer == 0 {
        ui.next_flags(ST_DISABLED);
    }
    action_button(ui, ID_LMERGE, IC_DOWNLOAD, "Nach unten zusammenführen", CMD_LAYER_MERGE);
    if p.doc.nlayers == 1 {
        ui.next_flags(ST_DISABLED);
    }
    action_button(ui, ID_LDEL, IC_DELETE, "Ebene löschen", CMD_LAYER_DEL);
    ui.box_end();
    ui.box_end();
}

// draw_checker(r) -- white/grey checkerboard behind the last placed rectangle (transparent
// layers); 6 DIP squares
fn draw_checker(ui: &mut Ui, r: [i32; 4]) {
    ui.draw_color(0xFFFF_FFFF);
    ui.draw_rect(r[0], r[1], r[2], r[3]);
    ui.draw_color(0xFFCC_CCCC);
    let s = ui.dp(6); // square size
    let mut y = 0;
    let mut par = 0;
    while y < r[3] {
        let mut x = par * s; // odd rows start one square in
        while x < r[2] {
            let w = (r[2] - x).min(s);
            let h = (r[3] - y).min(s);
            ui.draw_rect(r[0] + x, r[1] + y, w, h);
            x += s * 2;
        }
        y += s;
        par ^= 1;
    }
}

// action_button(id, icon, label, cmd) -- icon button that queues cmd
fn action_button(ui: &mut Ui, id: u32, icon: u32, label: &str, cmd: u32) {
    if ui.icon_button(id, icon, label) & EV_ACT != 0 {
        ui.cmd_run(cmd, 0);
    }
}

// layer_row(i) -- one row for layer i (thumbnail, name, eye)
fn layer_row(p: &mut Kritzel, ui: &mut Ui, i: i32) {
    let id = ID_LROW0 + i as u32;
    ui.next_size(-1, 56);
    ui.box_begin(id, BOX_H | BOX_XCENTER | PAD_4, 8);
    // the row itself is clickable (registered before its children)
    let rr = ui.last_rect();
    let ev = ui.behave(id, HB_FOCUS);
    if ev & EV_ACT != 0 {
        p.doc.cur_layer = i;
        ui.repass();
    }
    // background: selected = subtle fill + accent pill, hover = subtle hover
    let sel = i == p.doc.cur_layer;
    let fill = if sel {
        Some(ui.color(T_SUBTLE_PRESSED) | 0x0800_0000)
    } else if ev & EV_HOVER != 0 {
        Some(ui.color(T_SUBTLE_HOVER))
    } else {
        None
    };
    if let Some(c) = fill {
        ui.draw_color(c);
        let r4 = ui.dp(4);
        ui.draw_rrect(rr[0], rr[1], rr[2], rr[3], r4);
        if sel {
            // accent pill at the left
            let c = ui.color(T_ACCENT);
            ui.draw_color(c);
            let pw = ui.dp(3);
            let ph = ui.dp(16);
            let y = (rr[3].wrapping_sub(ph) >> 1).wrapping_add(rr[1]);
            ui.draw_rrect(rr[0], y, pw, ph, (pw as u32 >> 1) as i32);
        }
    }
    ui.spacer(6);
    // thumbnail with a hairline frame
    if let Some(t) = p.lay.thumbs[i as usize].clone() {
        let r = ui.place(t.w, t.h);
        draw_checker(ui, r);
        ui.draw_image(&t, p.lay.thumb_version, r[0], r[1], r[2], r[3]);
        let c = ui.color(T_CTRL_STROKE2);
        ui.draw_color(c);
        ui.draw_frame(r[0], r[1], r[2], r[3], 0, 1);
    }
    let name = fmt_cap(format!("Ebene {}", i + 1), 32);
    ui.next_size(-1, 0);
    ui.text(TS_BODY, &name);
    // visibility
    let ic = if p.doc.layer_vis[i as usize] != 0 { IC_VIEW } else { IC_HIDE };
    if ui.icon_button(ID_LVIS0 + i as u32, ic, "Sichtbarkeit") & EV_ACT != 0 {
        layer_toggle(p, i);
    }
    ui.box_end();
    let s = ui.sem(K_LISTITEM, id, Some(&name), i);
    s.rect = rr;
    s.state = if i == p.doc.cur_layer { SS_SELECTED } else { 0 };
}
