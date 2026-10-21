// select -- Kritzel selection ("Auswahl"): rectangle and free-form selection, floating pixels
// (move, resize, rotate, flip), clipboard, crop to selection  [apps/paint/src/select.s]
//
//   select_input(p, ui, in) -> true      canvas input (UiInput) while the select tool is active
//   select_overlay(p, ui)                floating pixels, marching ants, handles; finishes the
//                                        selection when another tool takes over; Enter
//                                        commits, Esc cancels (Ctrl+X / C / V / A and Delete
//                                        come as commands from the accelerator table)
//   select_ribbon(p, ui)                 chevron under the big tool button: flyout with
//                                        rectangle / free-form, select all, invert,
//                                        keyed-out background colour, delete
//   select_cmd(p, ui, id, arg) -> bool   CMD_CUT / COPY / PASTE / SELECT_ALL / DELETE / CROP /
//                                        SEL_INVERT / SEL_TRANSP / SEL_MODE (arg 0 rect, 1 free)
//   select_pre_cmd(p, ui, id, arg) -> bool  hook at the start of pt_cmd: floating pixels are
//                                        committed before other commands, Ctrl+Z cancels them,
//                                        rotate / flip / crop act on the selection (true =
//                                        consumed)
//   select_active(p) -> bool             a selection exists
//   select_cursor(p, ui) -> bits         canvas pointer flags (move / sizing / crosshair)
//   select_icon(p) -> icon               of the big tool button (rectangle / lasso)
//   select_status(p) -> text             status bar text "W × Hpx" of the selection ("" if none)
//
// Model: a selection is a box (sx, sy, sw, sh in doc px) plus an optional A8 mask (free-form
// and inverted selections) of the original size ow x oh.  The outline is the box, a
// rectangle inside it (ix.. for inverted rectangles) or the lasso polygon (pts, 24.8
// relative to the box at the original size); all of it is scaled by sw / ow when the
// floating pixels are resized.
// Dragging inside lifts the pixels of the current layer into a floating PARGB surface
// (fsrc); the hole gets colour 2 (background layer) or transparency (upper layers).
// fimg is what is shown and committed: fsrc keyed (colour 2 -> transparent in transparent
// mode) and scaled to sw x sh.  Lift .. commit is one undo step: hist_begin on lift,
// hist_end with the union of hole and destination on commit; Esc (or Ctrl+Z while
// floating) puts the original pixels back.
//
// The floating pixels and the marching ants are drawn at render time from a snapshot of
// the pass that recorded them (FloatDraw, AntsDraw).

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use gfx::path::Path;
use gfx::raster::Raster;
use gfx::{Dc, Surface, FMT_A8, RULE_NONZERO};
use img_core::ops;
use ui::*;

use crate::consts::*;
use crate::Kritzel;

const BTN_L_MASK: u32 = 1 << BTN_LEFT;
const D_NONE: u32 = 0; // drag: nothing
const D_RECT: u32 = 1; // drawing a rectangle
const D_LASSO: u32 = 2; // drawing a free-form outline
const D_MOVE: u32 = 3; // moving the selection
const D_SIZE: u32 = 4; // dragging a handle
const SK_RECT: u32 = 0; // sel_shape
const SK_FREE: u32 = 1;
const LASSO_MAX: usize = 8192; // free-form points
const HANDLE_DIP: i32 = 7;
const DASH: i32 = 4; // marching ants: 4 px dark, 4 px light
const ANT_DARK: u32 = 0xFF00_0000;
const ANT_LIGHT: u32 = 0xFFFF_FFFF;
const CLIP_LEVEL: u32 = 1; // deflate level of clipboard PNGs (fast)
const ID_SEL_RECT: u32 = ID_SELECT + 1;
const ID_SEL_FREE: u32 = ID_SELECT + 2;
const ID_SEL_ALL: u32 = ID_SELECT + 3;
const ID_SEL_INV: u32 = ID_SELECT + 4;
const ID_SEL_TRANSP: u32 = ID_SELECT + 5;
const ID_SEL_DEL: u32 = ID_SELECT + 6;

// commands that do not finish the selection
const KEEP_CMDS: [u32; 16] = [
    CMD_CUT,
    CMD_COPY,
    CMD_PASTE,
    CMD_SELECT_ALL,
    CMD_DELETE,
    CMD_SEL_INVERT,
    CMD_SEL_TRANSP,
    CMD_SEL_MODE,
    CMD_ZOOM_IN,
    CMD_ZOOM_OUT,
    CMD_ZOOM_100,
    CMD_ZOOM_FIT,
    CMD_THEME,
    CMD_TOGGLE_LAYERS,
    CMD_EDIT_COLOR,
    CMD_SWAP_COLORS,
];
// handle positions (column, row) in halves of the box: same order as the canvas handles
const SEL_HPOS: [[i32; 2]; 8] = [[0, 0], [1, 0], [2, 0], [0, 1], [2, 1], [0, 2], [1, 2], [2, 2]];
// pointer shape per handle
const SEL_HCUR: [u32; 8] =
    [CUR_SIZE_NWSE, CUR_SIZE_NS, CUR_SIZE_NESW, CUR_SIZE_WE, CUR_SIZE_WE, CUR_SIZE_NESW, CUR_SIZE_NS, CUR_SIZE_NWSE];

/// the box and the view: every doc <-> screen mapping of the module (view.s transforms and
/// the selection's own)
#[derive(Clone, Copy, Default)]
struct Geo {
    sx: i32,
    sy: i32,
    sw: i32,
    sh: i32,
    ow: i32,
    oh: i32,
    zoom: i32,
    cx: i32,
    cy: i32,
}

impl Geo {
    // doc_to_view(x, y) -> sx, sy
    fn d2v(&self, x: i32, y: i32) -> (i32, i32) {
        (
            (x.wrapping_mul(self.zoom) / 1000).wrapping_add(self.cx),
            (y.wrapping_mul(self.zoom) / 1000).wrapping_add(self.cy),
        )
    }

    // fx_scr(x, y) -> sx, sy -- doc 24.8 -> screen px (floor)
    fn fx_scr(&self, x: i32, y: i32) -> (i32, i32) {
        let z = self.zoom as i64;
        let a = (x as i64).wrapping_mul(z).div_euclid(256000) as i32;
        let b = (y as i64).wrapping_mul(z).div_euclid(256000) as i32;
        (a.wrapping_add(self.cx), b.wrapping_add(self.cy))
    }

    // pt_scr(px, py) -> sx, sy -- outline point (24.8, relative to the box at the original
    // size) -> screen px
    fn pt_scr(&self, px: i32, py: i32) -> (i32, i32) {
        let ow = if self.ow == 0 { 1 } else { self.ow as i64 };
        let oh = if self.oh == 0 { 1 } else { self.oh as i64 };
        let x = (self.sx << 8).wrapping_add(((px as i64).wrapping_mul(self.sw as i64) / ow) as i32);
        let y = (self.sy << 8).wrapping_add(((py as i64).wrapping_mul(self.sh as i64) / oh) as i32);
        self.fx_scr(x, y)
    }

    // sel_scr() -> {x0, y0, x1, y1} -- the selection box on screen
    fn scr(&self) -> [i32; 4] {
        let (a, b) = self.d2v(self.sx, self.sy);
        let (c, d) = self.d2v(self.sx.wrapping_add(self.sw), self.sy.wrapping_add(self.sh));
        [a, b, c, d]
    }
}

// handle_xy(i, rect) -> x, y -- centre of handle i (0..7) of a screen rect x0, y0, x1, y1
fn handle_xy(i: usize, r: &[i32; 4]) -> (i32, i32) {
    let [c, w] = SEL_HPOS[i];
    (
        (r[2].wrapping_sub(r[0]).wrapping_mul(c) >> 1).wrapping_add(r[0]),
        (r[3].wrapping_sub(r[1]).wrapping_mul(w) >> 1).wrapping_add(r[1]),
    )
}

/// the module's state (the asm's .bss)
pub(crate) struct Sel {
    mask: Option<Surface>,      // A8 selection mask (ow x oh) or none
    fsrc: Option<Surface>,      // floating pixels as lifted / pasted (PARGB)
    fimg: Option<Rc<Surface>>,  // shown / committed: keyed and scaled to sw x sh
    orig: Option<Surface>,      // layer pixels under the lift box (for Esc)
    vrect: [i32; 4],            // canvas widget rectangle (px)
    lb: [i32; 4],               // lasso bounds (doc 24.8) x0, y0, x1, y1
    r0: [i32; 4],               // box when a handle drag started (x0, y0, x1, y1)
    sel_on: bool,
    sel_shape: u32,
    sel_inv: bool,
    sx: i32,
    sy: i32,
    sw: i32,
    sh: i32,
    ow: i32,
    oh: i32,
    ix: i32,
    iy: i32,
    iw: i32,
    ih: i32,
    float: bool,
    lifted: bool,
    hist_open: bool,
    keep: bool,
    flayer: i32,
    hx: i32,
    hy: i32,
    hw: i32,
    hh: i32,
    drag: u32,
    handle: usize,
    grab_x: i32,
    grab_y: i32,
    ax: i32,
    ay: i32,
    ver: u32,
    vrect_ok: bool,
    /// 0 rectangle, 1 free-form
    pub(crate) mode: u32,
    /// background colour keyed out ("Hintergrundfarbe aussparen")
    pub(crate) transp: u32,
    pts: Vec<[i32; 2]>,
    float_draw: Rc<FloatDraw>,
    ants_draw: Rc<AntsDraw>,
}

impl Default for Sel {
    fn default() -> Sel {
        Sel {
            mask: None,
            fsrc: None,
            fimg: None,
            orig: None,
            vrect: [0; 4],
            lb: [0; 4],
            r0: [0; 4],
            sel_on: false,
            sel_shape: SK_RECT,
            sel_inv: false,
            sx: 0,
            sy: 0,
            sw: 0,
            sh: 0,
            ow: 0,
            oh: 0,
            ix: 0,
            iy: 0,
            iw: 0,
            ih: 0,
            float: false,
            lifted: false,
            hist_open: false,
            keep: false,
            flayer: 0,
            hx: 0,
            hy: 0,
            hw: 0,
            hh: 0,
            drag: D_NONE,
            handle: 0,
            grab_x: 0,
            grab_y: 0,
            ax: 0,
            ay: 0,
            ver: 0,
            vrect_ok: false,
            mode: 0,
            transp: 0,
            pts: Vec::new(),
            float_draw: Rc::new(FloatDraw::default()),
            ants_draw: Rc::new(AntsDraw::default()),
        }
    }
}

// ------------------------------------------------------------------ small helpers

/// select_active() -> true while a selection exists
#[allow(dead_code)] // public in the asm too, without a caller
pub(crate) fn select_active(pt: &Kritzel) -> bool {
    pt.sel.sel_on
}

/// select_icon() -> icon of the big tool button (rectangle or lasso mode)
pub(crate) fn select_icon(pt: &Kritzel) -> u32 {
    if pt.sel.mode == 0 { IC_SELECT_RECT } else { IC_LASSO }
}

/// select_status() -> status bar text: "W × Hpx" of the selection, "" without one
pub(crate) fn select_status(pt: &mut Kritzel) -> String {
    let s = &pt.sel;
    if s.sel_on || s.drag == D_RECT {
        format!("{} × {}px", s.sw, s.sh) // the rectangle being drawn counts
    } else {
        String::new()
    }
}

impl Sel {
    fn geo(&self, pt: &Kritzel) -> Geo {
        let (zoom, cx, cy, _, _) = view(pt);
        Geo { sx: self.sx, sy: self.sy, sw: self.sw, sh: self.sh, ow: self.ow, oh: self.oh, zoom, cx, cy }
    }

    // set_box(x, y, w, h) -- a plain rectangular selection
    fn set_box(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.sx = x;
        self.sy = y;
        self.sw = w;
        self.sh = h;
        self.ow = w;
        self.oh = h;
        self.iw = w;
        self.ih = h;
        self.ix = 0;
        self.iy = 0;
        self.sel_shape = SK_RECT;
        self.sel_inv = false;
        self.pts.clear();
        self.mask = None;
        self.sel_on = true;
        self.ver = self.ver.wrapping_add(1);
    }

    // sel_clear() -- forget the selection (floating pixels must be gone already)
    fn clear(&mut self) {
        self.mask = None;
        self.sel_on = false;
        self.pts.clear();
        self.sel_inv = false;
        self.sel_shape = SK_RECT;
        self.drag = D_NONE;
        self.ver = self.ver.wrapping_add(1);
    }

    // float_free() -- release the floating pixels
    fn float_free(&mut self) {
        self.fsrc = None;
        self.fimg = None;
        self.orig = None;
        self.float = false;
        self.hist_open = false;
        self.lifted = false;
        self.ver = self.ver.wrapping_add(1);
    }

    // shift_pts(dx, dy) -- move every outline point by dx, dy (24.8)
    fn shift_pts(&mut self, dx: i32, dy: i32) {
        for q in self.pts.iter_mut() {
            q[0] = q[0].wrapping_add(dx);
            q[1] = q[1].wrapping_add(dy);
        }
    }

    // sel_hit(sx, sy, dp7, dp3) -> -1 outside, 0..7 handle, 8 inside the box
    fn hit(&self, g: &Geo, x: i32, y: i32, dp7: i32, dp3: i32) -> i32 {
        let r = g.scr();
        if !self.sel_inv {
            // inverted: no handles
            let rad = ((dp7 as u32) >> 1) as i32 + dp3; // grab radius
            for i in 0..8 {
                let (hx, hy) = handle_xy(i, &r);
                if hx.wrapping_sub(x).wrapping_abs() <= rad && hy.wrapping_sub(y).wrapping_abs() <= rad {
                    return i as i32;
                }
            }
        }
        if x >= r[0] && x < r[2] && y >= r[1] && y < r[3] { 8 } else { -1 }
    }
}

// fill_argb(layer) -> straight colour for emptied pixels: colour 2 on the background
// layer, transparent above
fn fill_argb(pt: &Kritzel, layer: i32) -> u32 {
    if layer == 0 { colors(pt).1 } else { 0 }
}

// mark_hole() -- the lift rectangle changed
fn mark_hole(pt: &mut Kritzel) {
    let s = &pt.sel;
    let (x, y, w, h) = (s.hx, s.hy, s.hw, s.hh);
    doc_mark(pt, x, y, x.wrapping_add(w), y.wrapping_add(h));
}

// ------------------------------------------------------------------ pixel helpers

// sel_fill_region(sf, argb) -- set the selected pixels (mask or box at sx, sy, size
// ow x oh) of a layer to a straight colour, without blending
fn sel_fill_region(mask: &Option<Surface>, r: [i32; 4], dc: &mut Dc, sf: &mut Surface, argb: u32) {
    let [sx, sy, ow, oh] = r;
    let Some(m) = mask else {
        dc.begin(sf);
        dc.clip_push(sx, sy, ow, oh);
        dc.clear(sf, argb);
        dc.clip_pop();
        dc.end();
        return;
    };
    let pc = premul(argb).to_le_bytes();
    // the mask columns inside the layer: sx + x in 0 .. sf.w
    let x0 = (sx.wrapping_neg()).max(0).min(m.w.max(0));
    let x1 = (sf.w as i64 - sx as i64).clamp(x0 as i64, m.w.max(0) as i64) as i32;
    if x1 <= x0 {
        return;
    }
    let (ms, ss) = (m.stride as usize, sf.stride as usize);
    for row in 0..m.h.max(0) {
        let y = sy.wrapping_add(row);
        if y as u32 >= sf.h as u32 {
            continue; // also negative rows
        }
        let mrow = &m.px[row as usize * ms + x0 as usize..row as usize * ms + x1 as usize];
        let d0 = y as usize * ss + (sx + x0) as usize * 4;
        let drow = &mut sf.px[d0..d0 + mrow.len() * 4];
        for (d, &v) in drow.chunks_exact_mut(4).zip(mrow) {
            if v != 0 {
                d.copy_from_slice(&pc);
            }
        }
    }
}

// apply_mask(sf) -- clear the pixels of sf (aligned with the mask) outside the mask
fn apply_mask(mask: &Option<Surface>, sf: &mut Surface) {
    let Some(m) = mask else { return };
    let rows = sf.h.min(m.h).max(0) as usize;
    let cols = sf.w.min(m.w).max(0) as usize;
    let (ms, ss) = (m.stride as usize, sf.stride as usize);
    for y in 0..rows {
        let mrow = &m.px[y * ms..y * ms + cols];
        let drow = &mut sf.px[y * ss..y * ss + cols * 4];
        for (d, &v) in drow.chunks_exact_mut(4).zip(mrow) {
            if v == 0 {
                d.copy_from_slice(&[0; 4]);
            }
        }
    }
}

// key_out(sf, pargb) -- pixels equal to pargb become transparent
fn key_out(sf: &mut Surface, pargb: u32) {
    let (w, ss) = (sf.w.max(0) as usize, sf.stride as usize);
    let key = pargb.to_le_bytes();
    for y in 0..sf.h.max(0) as usize {
        for d in sf.px[y * ss..y * ss + w * 4].chunks_exact_mut(4) {
            if *d == key {
                d.copy_from_slice(&[0; 4]);
            }
        }
    }
}

// copy_surf(src) -> a new copy of src
fn copy_surf(src: &Surface) -> Option<Surface> {
    ops::crop(src, 0, 0, src.w, src.h).ok()
}

// sel_rebuild() -- fimg = fsrc (colour 2 keyed out in transparent mode) at sw x sh
fn sel_rebuild(pt: &mut Kritzel) {
    let col2 = colors(pt).1;
    let s = &mut pt.sel;
    s.fimg = None;
    if let Some(src) = &s.fsrc {
        let mut keyed = None;
        if s.transp != 0 {
            // out of memory: show it unkeyed
            if let Some(mut k) = copy_surf(src) {
                key_out(&mut k, premul(col2));
                keyed = Some(k);
            }
        }
        let src = keyed.as_ref().unwrap_or(src);
        let img = if s.sw == src.w && s.sh == src.h { copy_surf(src) } else { ops::scale(src, s.sw, s.sh, true).ok() };
        s.fimg = img.map(Rc::new);
    }
    s.ver = s.ver.wrapping_add(1);
}

// ------------------------------------------------------------------ lift / commit / cancel

// sel_lift() -> 0 ok / -1 -- lift the selected pixels of the current layer into the
// floating surface (no-op when already floating); starts the undo step
fn sel_lift(pt: &mut Kritzel, ui: &mut Ui) -> i32 {
    if pt.sel.float {
        return 0;
    }
    if !pt.sel.sel_on {
        return -1;
    }
    pt.sel.float_free();
    let layer = cur_layer(pt);
    pt.sel.flayer = layer;
    if hist_begin(pt, layer) != 0 {
        return -1;
    }
    pt.sel.hist_open = true;
    let (x, y, w, h) = (pt.sel.sx, pt.sel.sy, pt.sel.ow, pt.sel.oh);
    let lsf = layer_mut(pt, layer);
    let (Ok(orig), Ok(mut fsrc)) = (ops::crop(lsf, x, y, w, h), ops::crop(lsf, x, y, w, h)) else {
        hist_cancel(pt);
        pt.sel.float_free();
        return -1;
    };
    apply_mask(&pt.sel.mask, &mut fsrc);
    pt.sel.orig = Some(orig);
    pt.sel.fsrc = Some(fsrc);
    // the hole
    let fill = fill_argb(pt, layer);
    fill_region_layer(pt, ui, layer, fill);
    let s = &mut pt.sel;
    s.hx = s.sx;
    s.hy = s.sy;
    s.hw = s.ow;
    s.hh = s.oh;
    s.lifted = true;
    s.float = true;
    s.keep = true;
    mark_hole(pt);
    sel_rebuild(pt);
    ui.repass();
    0
}

// sel_fill_region on layer `layer` of the document
fn fill_region_layer(pt: &mut Kritzel, ui: &mut Ui, layer: i32, argb: u32) {
    let mask = pt.sel.mask.take();
    let r = [pt.sel.sx, pt.sel.sy, pt.sel.ow, pt.sel.oh];
    let sf = layer_mut(pt, layer);
    sel_fill_region(&mask, r, &mut ui.dc, sf, argb);
    pt.sel.mask = mask;
}

// sel_commit(keep) -- blend the floating pixels into their layer (closes the undo step);
// keep keeps the selection outline when it still describes the pixels, otherwise (and
// always without keep) the selection is gone afterwards
fn sel_commit(pt: &mut Kritzel, ui: &mut Ui, keep: bool) {
    let mut keep = keep;
    if pt.sel.float {
        let mut layer = pt.sel.flayer;
        if layer as u32 >= nlayers(pt) as u32 {
            layer = cur_layer(pt);
        }
        if !pt.sel.hist_open {
            hist_begin(pt, layer);
        }
        let fimg = pt.sel.fimg.clone();
        let (sx, sy) = (pt.sel.sx, pt.sel.sy);
        {
            let dc = &mut ui.dc;
            let sf = layer_mut(pt, layer);
            dc.begin(sf);
            dc.set_opacity(255);
            if let Some(f) = &fimg {
                dc.blit(sf, f, sx, sy);
            }
            dc.end();
        }
        // changed area: destination and hole
        let s = &pt.sel;
        let (mut x0, mut y0) = (s.sx, s.sy);
        let (mut x1, mut y1) = (s.sx.wrapping_add(s.sw), s.sy.wrapping_add(s.sh));
        if s.lifted {
            x0 = x0.min(s.hx);
            y0 = y0.min(s.hy);
            x1 = x1.max(s.hx.wrapping_add(s.hw));
            y1 = y1.max(s.hy.wrapping_add(s.hh));
        }
        doc_mark(pt, x0, y0, x1, y1);
        hist_end(pt, x0, y0, x1, y1);
        // the outline survives only at the original size
        let s = &pt.sel;
        let k = s.sw == s.ow && s.sh == s.oh && s.keep;
        keep &= k;
        pt.sel.float_free();
        ui.repass();
    }
    if !keep {
        pt.sel.clear();
    }
}

// sel_done() -- commit and deselect
fn sel_done(pt: &mut Kritzel, ui: &mut Ui) {
    sel_commit(pt, ui, false);
}

// sel_cancel() -- drop the floating pixels and put the original ones back; deselect
fn sel_cancel(pt: &mut Kritzel, ui: &mut Ui) {
    if pt.sel.float {
        let layer = pt.sel.flayer;
        if pt.sel.lifted && (layer as u32) < nlayers(pt) as u32 {
            if let Some(orig) = pt.sel.orig.take() {
                let (hx, hy) = (pt.sel.hx, pt.sel.hy);
                ops::blit_copy(layer_mut(pt, layer), &orig, hx, hy);
                pt.sel.orig = Some(orig);
            }
            mark_hole(pt);
        }
        if pt.sel.hist_open {
            hist_cancel(pt);
        }
        pt.sel.float_free();
        ui.repass();
    }
    pt.sel.clear();
}

// sel_delete() -- empty the selected pixels (colour 2 / transparent) and deselect
fn sel_delete(pt: &mut Kritzel, ui: &mut Ui) {
    if !pt.sel.sel_on {
        return;
    }
    if pt.sel.float {
        // floating: the hole already is the result
        if pt.sel.hist_open {
            let s = &pt.sel;
            let (x, y, w, h) = (s.hx, s.hy, s.hw, s.hh);
            hist_end(pt, x, y, x.wrapping_add(w), y.wrapping_add(h));
        }
        pt.sel.float_free();
    } else {
        let layer = cur_layer(pt);
        hist_begin(pt, layer);
        let fill = fill_argb(pt, layer);
        fill_region_layer(pt, ui, layer, fill);
        let s = &pt.sel;
        let (x0, y0) = (s.sx, s.sy);
        let (x1, y1) = (s.sx.wrapping_add(s.ow), s.sy.wrapping_add(s.oh));
        doc_mark(pt, x0, y0, x1, y1);
        hist_end(pt, x0, y0, x1, y1);
    }
    pt.sel.clear();
    ui.repass();
}

// ------------------------------------------------------------------ clipboard

// sel_grab() -> the selected pixels as a new PARGB surface
fn sel_grab(pt: &mut Kritzel) -> Option<Surface> {
    if pt.sel.float {
        return pt.sel.fimg.as_ref().and_then(|f| copy_surf(f));
    }
    let (x, y, w, h) = (pt.sel.sx, pt.sel.sy, pt.sel.ow, pt.sel.oh);
    let layer = cur_layer(pt);
    let mut sf = ops::crop(layer_mut(pt, layer), x, y, w, h).ok()?;
    apply_mask(&pt.sel.mask, &mut sf);
    Some(sf)
}

// sel_copy() -- the selection as PNG on the clipboard
fn sel_copy(pt: &mut Kritzel, ui: &mut Ui) {
    if !pt.sel.sel_on {
        return;
    }
    let Some(sf) = sel_grab(pt) else { return };
    if let Some(png) = img_png::enc::encode_classic(&sf, CLIP_LEVEL) {
        ui.clip_set(CLIP_PNG, &png);
    }
}

// sel_paste() -> true if pasted -- a PNG from the clipboard becomes floating pixels at the
// top-left of the visible canvas (the canvas grows when the image is larger)
fn sel_paste(pt: &mut Kritzel, ui: &mut Ui) -> bool {
    let Some(data) = ui.clip_get(CLIP_PNG) else { return false };
    let Ok(sf) = img_core::io::decode(&data) else { return false };
    if sf.w <= 0 || sf.h <= 0 {
        return false;
    }
    // the canvas may grow to the image, but never beyond DOC_MAX
    if sf.w > DOC_MAX || sf.h > DOC_MAX {
        paint_error(pt, ui, "Einfügen: Das Bild ist größer als 16384 × 16384 Pixel.");
        return false;
    }
    sel_done(pt, ui);
    tool_set(pt, ui, TOOL_SELECT);
    // top-left of the visible part of the image
    let (mut x, mut y) = (0, 0);
    if pt.sel.vrect_ok {
        let (vx, vy) = view_to_doc(pt, pt.sel.vrect[0], pt.sel.vrect[1]);
        if vx > 0 {
            x = vx;
        }
        if vy > 0 {
            y = vy;
        }
    }
    // a larger image enlarges the canvas (keeps the top-left)
    let (dw, dh) = (doc_w(pt), doc_h(pt));
    let nw = dw.max(sf.w);
    let nh = dh.max(sf.h);
    if nw != dw || nh != dh {
        op_canvas(pt, ui, nw, nh);
    }
    // stay inside the canvas where possible
    x = x.min(nw - sf.w);
    y = y.min(nh - sf.h);
    let layer = cur_layer(pt);
    let s = &mut pt.sel;
    s.float_free();
    s.set_box(x, y, sf.w, sf.h);
    s.fsrc = Some(sf);
    s.flayer = layer;
    s.float = true;
    s.keep = true;
    sel_rebuild(pt);
    ui.repass();
    true
}

// ------------------------------------------------------------------ selection commands

// sel_select_all() -- the whole canvas (switches to the select tool)
fn sel_select_all(pt: &mut Kritzel, ui: &mut Ui) {
    sel_done(pt, ui);
    tool_set(pt, ui, TOOL_SELECT);
    let (w, h) = (doc_w(pt), doc_h(pt));
    pt.sel.set_box(0, 0, w, h);
    ui.repass();
}

// sel_invert() -- select what is not selected (and back)
fn sel_invert(pt: &mut Kritzel, ui: &mut Ui) {
    if !pt.sel.sel_on {
        return;
    }
    if pt.sel.float {
        sel_commit(pt, ui, true);
        if !pt.sel.sel_on {
            return;
        }
    }
    let (dw, dh) = (doc_w(pt), doc_h(pt));
    let s = &mut pt.sel;
    let new;
    if !s.sel_inv {
        // ---- normal -> inverted: canvas mask, 255 except the old selection
        let mut m = Surface::new(dw, dh, FMT_A8);
        m.px.fill(255);
        let ms = m.stride as usize;
        // the old columns inside the canvas: sx + x in 0 .. dw
        let x0 = s.sx.wrapping_neg().max(0).min(s.ow.max(0));
        let x1 = (dw as i64 - s.sx as i64).clamp(x0 as i64, s.ow.max(0) as i64) as i32;
        for row in 0..s.oh.max(0) {
            let y = s.sy.wrapping_add(row);
            if y as u32 >= dh as u32 || x1 <= x0 {
                continue;
            }
            let d0 = y as usize * ms + (s.sx + x0) as usize;
            let drow = &mut m.px[d0..d0 + (x1 - x0) as usize];
            match &s.mask {
                None => drow.fill(0),
                Some(o) => {
                    let oo = row as usize * o.stride as usize;
                    for (d, &v) in drow.iter_mut().zip(&o.px[oo + x0 as usize..oo + x1 as usize]) {
                        if v != 0 {
                            *d = 0;
                        }
                    }
                }
            }
        }
        let (dx, dy) = (s.sx << 8, s.sy << 8);
        s.shift_pts(dx, dy);
        s.ix = s.sx;
        s.iy = s.sy;
        s.iw = s.ow;
        s.ih = s.oh;
        s.sx = 0;
        s.sy = 0;
        s.sw = dw;
        s.ow = dw;
        s.sh = dh;
        s.oh = dh;
        s.sel_inv = true;
        new = Some(m);
    } else {
        // ---- inverted -> normal: the inner shape
        let mut nm = None;
        if s.sel_shape == SK_FREE {
            let mut m = Surface::new(s.iw, s.ih, FMT_A8);
            let ms = m.stride as usize;
            if let Some(old) = &s.mask {
                // the inner columns inside the old mask: ix + x in 0 .. old.w
                let x0 = s.ix.wrapping_neg().max(0).min(s.iw.max(0));
                let x1 = (old.w as i64 - s.ix as i64).clamp(x0 as i64, s.iw.max(0) as i64) as i32;
                for row in 0..s.ih.max(0) {
                    let y = s.iy.wrapping_add(row);
                    if y as u32 >= old.h as u32 || x1 <= x0 {
                        continue;
                    }
                    let oo = y as usize * old.stride as usize + (s.ix + x0) as usize;
                    let orow = &old.px[oo..oo + (x1 - x0) as usize];
                    let d0 = row as usize * ms + x0 as usize;
                    for (d, &v) in m.px[d0..d0 + (x1 - x0) as usize].iter_mut().zip(orow) {
                        if v == 0 {
                            *d = 255;
                        }
                    }
                }
            }
            let (dx, dy) = ((s.ix << 8).wrapping_neg(), (s.iy << 8).wrapping_neg());
            s.shift_pts(dx, dy);
            nm = Some(m);
        }
        s.sx = s.sx.wrapping_add(s.ix);
        s.sy = s.sy.wrapping_add(s.iy);
        s.sw = s.iw;
        s.ow = s.iw;
        s.sh = s.ih;
        s.oh = s.ih;
        s.ix = 0;
        s.iy = 0;
        s.sel_inv = false;
        new = nm;
    }
    s.mask = new;
    s.ver = s.ver.wrapping_add(1);
    ui.repass();
}

// sel_crop() -> true if there was a selection -- crop the image to the selection box
fn sel_crop(pt: &mut Kritzel, ui: &mut Ui) -> bool {
    if !pt.sel.sel_on {
        return false;
    }
    let s = &pt.sel;
    let (mut x0, mut y0) = (s.sx, s.sy);
    let (mut x1, mut y1) = (s.sx.wrapping_add(s.sw), s.sy.wrapping_add(s.sh));
    sel_done(pt, ui);
    x0 = x0.max(0);
    y0 = y0.max(0);
    x1 = x1.min(doc_w(pt));
    y1 = y1.min(doc_h(pt));
    let w = x1.wrapping_sub(x0);
    let h = y1.wrapping_sub(y0);
    if w > 0 && h > 0 {
        op_crop(pt, ui, x0, y0, w, h);
    }
    true
}

// sel_xform(cmd) -> true -- rotate / flip the selected pixels (they float afterwards)
fn sel_xform(pt: &mut Kritzel, ui: &mut Ui, cmd: u32) -> bool {
    if sel_lift(pt, ui) != 0 {
        return true;
    }
    let s = &mut pt.sel;
    if let Some(src) = s.fsrc.as_mut() {
        match cmd {
            CMD_FLIP_H => ops::flip_h(src),
            CMD_FLIP_V => ops::flip_v(src),
            CMD_ROT_180 => ops::rotate180(src),
            _ => {
                let dir = if cmd == CMD_ROT_R { 1 } else { -1 };
                if let Ok(r) = ops::rotate90(src, dir) {
                    *src = r;
                    // turn the box around its centre
                    let cx2 = s.sx.wrapping_add(s.sx).wrapping_add(s.sw); // 2 * centre x
                    let cy2 = s.sy.wrapping_add(s.sy).wrapping_add(s.sh); // 2 * centre y
                    let (w, h) = (s.sw, s.sh);
                    s.sw = h;
                    s.sh = w;
                    s.sx = cx2.wrapping_sub(s.sw) >> 1;
                    s.sy = cy2.wrapping_sub(s.sh) >> 1;
                }
            }
        }
    }
    // the outline becomes the box of the new pixels
    let (w, h) = s.fsrc.as_ref().map_or((0, 0), |f| (f.w, f.h));
    s.ow = w;
    s.iw = w;
    s.oh = h;
    s.ih = h;
    s.ix = 0;
    s.iy = 0;
    s.sel_shape = SK_RECT;
    s.sel_inv = false;
    s.pts.clear();
    s.keep = false;
    s.mask = None;
    sel_rebuild(pt);
    ui.repass();
    true
}

// ------------------------------------------------------------------ interaction

// corner_at(sx, sy) -> x, y -- doc pixel corner nearest to a screen point
fn corner_at(pt: &Kritzel, sx: i32, sy: i32) -> (i32, i32) {
    let (zoom, cx, cy, _, _) = view(pt);
    // floor((2000 * off + zoom) / (2 * zoom))
    let axis = |off: i32| {
        let n = off.wrapping_mul(2000).wrapping_add(zoom);
        let d = zoom.wrapping_add(zoom);
        let q = n.wrapping_div(d);
        if n.wrapping_rem(d) < 0 { q - 1 } else { q }
    };
    (axis(sx.wrapping_sub(cx)), axis(sy.wrapping_sub(cy)))
}

// corner_clamped(sx, sy) -> x, y -- corner_at limited to the canvas
fn corner_clamped(pt: &Kritzel, sx: i32, sy: i32) -> (i32, i32) {
    let (x, y) = corner_at(pt, sx, sy);
    (x.max(0).min(doc_w(pt)), y.max(0).min(doc_h(pt)))
}

// lasso_add(sx, sy) -- append a screen point to the free-form outline being drawn
fn lasso_add(pt: &mut Kritzel, sx: i32, sy: i32) {
    let (x, y) = view_to_doc_fx(pt, sx, sy);
    let x = x.max(0).min(doc_w(pt) << 8);
    let y = y.max(0).min(doc_h(pt) << 8);
    let s = &mut pt.sel;
    let n = s.pts.len();
    if n == 0 {
        s.lb = [x, y, x, y];
    } else {
        if n >= LASSO_MAX || s.pts[n - 1] == [x, y] {
            return;
        }
        if x < s.lb[0] {
            s.lb[0] = x;
        }
        if y < s.lb[1] {
            s.lb[1] = y;
        }
        if x > s.lb[2] {
            s.lb[2] = x;
        }
        if y > s.lb[3] {
            s.lb[3] = y;
        }
    }
    s.pts.push([x, y]);
    s.ver = s.ver.wrapping_add(1);
}

// build_lasso_mask() -> selected pixel count -- mask (ow x oh) from pts
fn build_lasso_mask(s: &mut Sel, dc: &mut Dc, path: &mut Path, ras: &mut Raster) -> u32 {
    s.mask = None;
    let mut m = Surface::new(s.ow, s.oh, FMT_A8);
    if m.w != s.ow || m.h != s.oh {
        return 0;
    }
    dc.begin(&m);
    dc.set_opacity(255);
    dc.set_color(0xFF00_0000);
    path.begin(dc);
    if let Some(q) = s.pts.first() {
        path.move_fx(q[0], q[1]);
    }
    for q in s.pts.iter().skip(1) {
        path.line_fx(q[0], q[1]);
    }
    path.close();
    path.fill(ras, dc, &mut m, RULE_NONZERO);
    dc.end();
    // hard edges: pixels at least half covered
    let mut count = 0;
    let ms = m.stride as usize;
    for y in 0..m.h.max(0) as usize {
        for v in m.px[y * ms..y * ms + m.w.max(0) as usize].iter_mut() {
            if *v >= 128 {
                *v = 255;
                count += 1;
            } else {
                *v = 0;
            }
        }
    }
    s.mask = Some(m);
    count
}

// lasso_finish() -- the free-form outline becomes the selection
fn lasso_finish(pt: &mut Kritzel, ui: &mut Ui) {
    let s = &mut pt.sel;
    let ok = 'f: {
        if s.pts.len() < 3 {
            break 'f false;
        }
        let x0 = s.lb[0] >> 8; // floor
        let y0 = s.lb[1] >> 8;
        let x1 = s.lb[2].wrapping_add(255) >> 8; // ceil
        let y1 = s.lb[3].wrapping_add(255) >> 8;
        let w = x1.wrapping_sub(x0);
        let h = y1.wrapping_sub(y0);
        if w <= 0 || h <= 0 {
            break 'f false;
        }
        s.shift_pts((x0 << 8).wrapping_neg(), (y0 << 8).wrapping_neg());
        s.sx = x0;
        s.sy = y0;
        s.sw = w;
        s.ow = w;
        s.iw = w;
        s.sh = h;
        s.oh = h;
        s.ih = h;
        s.ix = 0;
        s.iy = 0;
        s.sel_inv = false;
        s.sel_shape = SK_FREE;
        let Ui { dc, text, .. } = ui;
        if build_lasso_mask(s, dc, &mut text.path, &mut text.ras) == 0 {
            break 'f false;
        }
        s.sel_on = true;
        s.ver = s.ver.wrapping_add(1);
        true
    };
    if !ok {
        pt.sel.clear();
    }
}

// rect_update(sx, sy, mods) -- the rectangle being drawn follows the pointer (Shift: square)
fn rect_update(pt: &mut Kritzel, x: i32, y: i32, mods: u32) {
    let (cx, cy) = corner_clamped(pt, x, y);
    let s = &mut pt.sel;
    let mut dx = cx.wrapping_sub(s.ax);
    let mut dy = cy.wrapping_sub(s.ay);
    if mods & MOD_SHIFT != 0 {
        let side = dx.wrapping_abs().min(dy.wrapping_abs());
        dx = if dx < 0 { side.wrapping_neg() } else { side };
        dy = if dy < 0 { side.wrapping_neg() } else { side };
    }
    let (mut x0, mut w) = (s.ax, dx);
    if w < 0 {
        x0 = x0.wrapping_add(w);
        w = w.wrapping_neg();
    }
    s.sx = x0;
    s.sw = w;
    let (mut y0, mut h) = (s.ay, dy);
    if h < 0 {
        y0 = y0.wrapping_add(h);
        h = h.wrapping_neg();
    }
    s.sy = y0;
    s.sh = h;
    s.ver = s.ver.wrapping_add(1);
}

// move_update(sx, sy) -- drag the selection (lifts it on the first real move)
fn move_update(pt: &mut Kritzel, ui: &mut Ui, x: i32, y: i32) {
    let (dx, dy) = view_to_doc(pt, x, y);
    let nx = dx.wrapping_sub(pt.sel.grab_x);
    let ny = dy.wrapping_sub(pt.sel.grab_y);
    if nx == pt.sel.sx && ny == pt.sel.sy {
        return;
    }
    if sel_lift(pt, ui) != 0 {
        pt.sel.drag = D_NONE;
        return;
    }
    let s = &mut pt.sel;
    s.sx = nx;
    s.sy = ny;
    s.ver = s.ver.wrapping_add(1);
}

// size_update(sx, sy) -- drag handle `handle` (lifts on the first change)
fn size_update(pt: &mut Kritzel, ui: &mut Ui, x: i32, y: i32) {
    let (px, py) = corner_at(pt, x, y);
    let s = &pt.sel;
    let [mut x0, mut y0, mut x1, mut y1] = s.r0;
    let [col, row] = SEL_HPOS[s.handle & 7];
    if col == 0 {
        x0 = px.min(x1.wrapping_sub(1));
    }
    if col == 2 {
        x1 = px.max(x0.wrapping_add(1));
    }
    if row == 0 {
        y0 = py.min(y1.wrapping_sub(1));
    }
    if row == 2 {
        y1 = py.max(y0.wrapping_add(1));
    }
    let w = x1.wrapping_sub(x0);
    let h = y1.wrapping_sub(y0);
    if x0 == s.sx && y0 == s.sy && w == s.sw && h == s.sh {
        return;
    }
    if sel_lift(pt, ui) != 0 {
        pt.sel.drag = D_NONE;
        return;
    }
    let s = &mut pt.sel;
    s.sx = x0;
    s.sy = y0;
    s.sw = w;
    s.sh = h;
    s.ver = s.ver.wrapping_add(1);
}

// drag_finish() -- pointer released
fn drag_finish(pt: &mut Kritzel, ui: &mut Ui) {
    let d = pt.sel.drag;
    pt.sel.drag = D_NONE;
    match d {
        D_RECT => {
            let s = &mut pt.sel;
            if s.sw > 0 && s.sh > 0 {
                let (x, y, w, h) = (s.sx, s.sy, s.sw, s.sh);
                s.set_box(x, y, w, h);
            } else {
                s.clear();
            }
        }
        D_LASSO => lasso_finish(pt, ui),
        D_SIZE => {
            if pt.sel.float {
                sel_rebuild(pt); // smooth resample at the final size
            }
        }
        _ => {}
    }
    pt.sel.ver = pt.sel.ver.wrapping_add(1);
    ui.repass();
}

/// select_input(in) -> true -- one frame of canvas input for the select tool
pub(crate) fn select_input(pt: &mut Kritzel, ui: &mut Ui, inp: &UiInput) -> bool {
    pt.sel.vrect = inp.rect;
    pt.sel.vrect_ok = true;
    let mx = inp.x.wrapping_add(inp.rect[0]);
    let my = inp.y.wrapping_add(inp.rect[1]);
    if pt.sel.drag == D_NONE {
        if inp.pressed & BTN_L_MASK == 0 || inp.ev & EV_HOVER == 0 {
            return true;
        }
        // the canvas takes the keyboard: Enter / Delete must not go to a ribbon button
        if ui.focused() != 0 {
            ui.focus(0);
        }
        let px = inp.press_x.wrapping_add(inp.rect[0]);
        let py = inp.press_y.wrapping_add(inp.rect[1]);
        if pt.sel.sel_on {
            let g = pt.sel.geo(pt);
            let h = pt.sel.hit(&g, px, py, ui.dp(HANDLE_DIP), ui.dp(3));
            if h == 8 {
                let (dx, dy) = view_to_doc(pt, px, py);
                let s = &mut pt.sel;
                s.grab_x = dx.wrapping_sub(s.sx);
                s.grab_y = dy.wrapping_sub(s.sy);
                s.drag = D_MOVE;
                return true;
            }
            if h >= 0 {
                // a handle: resize from the current box
                let s = &mut pt.sel;
                s.handle = h as usize;
                s.r0 = [s.sx, s.sy, s.sx.wrapping_add(s.sw), s.sy.wrapping_add(s.sh)];
                s.drag = D_SIZE;
                return true;
            }
            sel_done(pt, ui);
        }
        if pt.sel.mode == 1 {
            pt.sel.pts.clear();
            pt.sel.drag = D_LASSO;
            lasso_add(pt, px, py);
        } else {
            let (cx, cy) = corner_clamped(pt, px, py);
            let s = &mut pt.sel;
            s.ax = cx;
            s.ay = cy;
            s.sx = cx;
            s.sy = cy;
            s.sw = 0;
            s.sh = 0;
            s.drag = D_RECT;
            s.ver = s.ver.wrapping_add(1);
        }
        return true;
    }
    if pt.sel.drag == D_LASSO {
        // every recorded pointer position extends the outline
        for q in inp.pts.iter() {
            lasso_add(pt, q[0].wrapping_add(inp.rect[0]), q[1].wrapping_add(inp.rect[1]));
        }
    }
    match pt.sel.drag {
        D_RECT => rect_update(pt, mx, my, inp.mods),
        D_LASSO => lasso_add(pt, mx, my),
        D_MOVE => move_update(pt, ui, mx, my),
        D_SIZE => size_update(pt, ui, mx, my),
        _ => {}
    }
    if inp.btn & BTN_L_MASK == 0 && pt.sel.drag != D_NONE {
        drag_finish(pt, ui);
    }
    true
}

// sel_keys() -- Enter commits, Esc cancels the selection (Ctrl+X / C / V / A and Delete
// arrive as commands from the accelerator table)
fn sel_keys(pt: &mut Kritzel, ui: &mut Ui) {
    if tool(pt) == TOOL_TEXT {
        return;
    }
    if pt.sel.sel_on && ui.key(VK_RETURN) {
        sel_done(pt, ui);
    }
    if (pt.sel.sel_on || pt.sel.drag != D_NONE) && ui.key(VK_ESCAPE) {
        sel_cancel(pt, ui);
    }
}

/// select_cursor() -> next_flags value for the canvas pointer: move inside the selection,
/// sizing arrows over the handles, otherwise a crosshair
pub(crate) fn select_cursor(pt: &mut Kritzel, ui: &mut Ui) -> u32 {
    let s = &pt.sel;
    let cur = match s.drag {
        D_MOVE => CUR_MOVE,
        D_SIZE => SEL_HCUR[s.handle & 7],
        D_NONE if s.sel_on => {
            let (mx, my) = ui.mouse();
            let g = s.geo(pt);
            match s.hit(&g, mx, my, ui.dp(HANDLE_DIP), ui.dp(3)) {
                8 => CUR_MOVE,
                h if h >= 0 => SEL_HCUR[h as usize],
                _ => CUR_CROSS,
            }
        }
        _ => CUR_CROSS,
    };
    cur << HB_CURSOR_SHIFT
}

// ------------------------------------------------------------------ drawing

/// draw_float: the cmd_custom callback of the floating pixels in view scale (the image and
/// its screen box of the recording pass)
#[derive(Default)]
struct FloatDraw {
    img: RefCell<Option<Rc<Surface>>>,
    scr: RefCell<[i32; 4]>,
}

impl Draw for FloatDraw {
    fn draw(&self, g: &mut Gfx, _r: [i32; 4]) {
        let img = self.img.borrow();
        let Some(f) = img.as_ref() else { return };
        let r = *self.scr.borrow();
        g.dc.blit_scaled(g.sf, f, r[0], r[1], r[2] - r[0], r[3] - r[1]);
    }
}

/// what draw_ants reads (the selection's outline and the view of the recording pass)
#[derive(Clone, Default)]
struct AntSnap {
    g: Geo,
    drag: u32,
    sel_inv: bool,
    sel_shape: u32,
    ix: i32,
    iy: i32,
    iw: i32,
    ih: i32,
    pts_ver: Option<u32>,
    pts: Vec<[i32; 2]>,
}

/// draw_ants: the cmd_custom callback of the selection outline
#[derive(Default)]
struct AntsDraw {
    snap: RefCell<AntSnap>,
}

// the target of draw_ants: surface, clip, dash phase
struct Ants<'a, 'b> {
    dc: &'a mut Dc,
    sf: &'b mut Surface,
    clip: [i32; 4],
    n: u32,
}

impl Ants<'_, '_> {
    // ants_line(a, b, other, vert): dashed 1 px line (horizontal: x a..b at y = other)
    fn line(&mut self, a: i32, b: i32, other: i32, vert: bool) {
        if b <= a {
            return;
        }
        let k = vert as usize;
        let fill = |dc: &mut Dc, sf: &mut Surface, s: i32, l: i32| {
            if vert {
                dc.fill(sf, other, s, 1, l);
            } else {
                dc.fill(sf, s, other, l, 1);
            }
        };
        self.dc.set_color(ANT_LIGHT);
        fill(self.dc, self.sf, a, b - a);
        self.dc.set_color(ANT_DARK);
        // first dark dash at or before the start of the clip
        let mut pos = a;
        let d = self.clip[k].wrapping_sub(a);
        if d > 0 {
            pos = pos.wrapping_add(d & -(2 * DASH));
        }
        while pos < b && pos < self.clip[k + 2] {
            let l = (b - pos).min(DASH);
            fill(self.dc, self.sf, pos, l);
            pos += 2 * DASH;
        }
    }

    // ants_rect(x0, y0, x1, y1) -- marching ants just outside the screen rectangle
    fn rect(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        self.line(x0 - 1, x1 + 1, y0 - 1, false);
        self.line(x0 - 1, x1 + 1, y1, false);
        self.line(y0, y1, x0 - 1, true);
        self.line(y0, y1, x1, true);
    }

    // ants_seg(x0, y0, x1, y1) -- dashed Bresenham line into the target, end point
    // excluded; the dash phase continues in n
    fn seg(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        let adx = x1.wrapping_sub(x0).wrapping_abs();
        let ndy = y1.wrapping_sub(y0).wrapping_abs().wrapping_neg();
        let c = self.clip;
        // completely outside the clip: only advance the dash phase
        if x0.max(x1) < c[0] || x0.min(x1) >= c[2] || y0.max(y1) < c[1] || y0.min(y1) >= c[3] {
            let m = ndy.wrapping_neg().max(adx);
            self.n = self.n.wrapping_add(m as u32);
            return;
        }
        let stx = if x0 < x1 { 1 } else { -1 };
        let sty = if y0 < y1 { 1 } else { -1 };
        let mut err = adx.wrapping_add(ndy);
        let (mut x, mut y) = (x0, y0);
        let stride = self.sf.stride as usize;
        loop {
            if x == x1 && y == y1 {
                break;
            }
            if x >= c[0] && x < c[2] && y >= c[1] && y < c[3] {
                let v = if self.n & DASH as u32 != 0 { ANT_LIGHT } else { ANT_DARK };
                let o = y as usize * stride + x as usize * 4;
                self.sf.px[o..o + 4].copy_from_slice(&v.to_le_bytes());
            }
            self.n = self.n.wrapping_add(1);
            let e2 = err.wrapping_add(err);
            if e2 >= ndy {
                err = err.wrapping_add(ndy);
                x = x.wrapping_add(stx);
            }
            if e2 <= adx {
                err = err.wrapping_add(adx);
                y = y.wrapping_add(sty);
            }
        }
    }

    // ants_poly(closed, absolute) -- the outline points: absolute doc 24.8 (lasso being
    // drawn) or relative to the box (finished selection)
    fn poly(&mut self, s: &AntSnap, closed: bool, absolute: bool) {
        if s.pts.is_empty() {
            return;
        }
        let pt = |i: usize| {
            let q = s.pts[i];
            if absolute { s.g.fx_scr(q[0], q[1]) } else { s.g.pt_scr(q[0], q[1]) }
        };
        let first = pt(0);
        let mut prev = first;
        for i in 1..s.pts.len() {
            let p = pt(i);
            self.seg(prev.0, prev.1, p.0, p.1);
            prev = p;
        }
        if closed {
            self.seg(prev.0, prev.1, first.0, first.1);
        }
    }
}

impl Draw for AntsDraw {
    fn draw(&self, g: &mut Gfx, _r: [i32; 4]) {
        let s = self.snap.borrow();
        let clip = g.dc.clip;
        let mut a = Ants { dc: &mut *g.dc, sf: &mut *g.sf, clip, n: 0 };
        if s.drag == D_LASSO {
            a.poly(&s, false, true);
            return;
        }
        let r = s.g.scr();
        if s.drag != D_RECT && s.sel_inv {
            a.rect(r[0], r[1], r[2], r[3]);
            if s.sel_shape == SK_FREE {
                a.poly(&s, true, false);
                return;
            }
            // inner rectangle
            let (x0, y0) = s.g.pt_scr(s.ix << 8, s.iy << 8);
            let (x1, y1) = s.g.pt_scr(s.ix.wrapping_add(s.iw) << 8, s.iy.wrapping_add(s.ih) << 8);
            a.rect(x0, y0, x1, y1);
            return;
        }
        if s.drag != D_RECT && s.sel_shape == SK_FREE {
            a.poly(&s, true, false);
            return;
        }
        a.rect(r[0], r[1], r[2], r[3]);
    }
}

// draw_sel_handles() -- the eight resize handles of the box
fn draw_sel_handles(ui: &mut Ui, g: &Geo) {
    let hs = ui.dp(HANDLE_DIP);
    let r = g.scr();
    for i in 0..8 {
        let (hx, hy) = handle_xy(i, &r);
        let half = ((hs as u32) >> 1) as i32;
        let x = hx - half;
        let y = hy - half;
        ui.draw_color(0xFF40_4040);
        ui.draw_rect(x, y, hs, hs);
        ui.draw_color(0xFFFF_FFFF);
        ui.draw_rect(x + 1, y + 1, hs - 2, hs - 2);
    }
}

/// select_overlay() -- floating pixels, outline and handles; keyboard shortcuts; another
/// tool finishes the selection
pub(crate) fn select_overlay(pt: &mut Kritzel, ui: &mut Ui) {
    if tool(pt) != TOOL_SELECT {
        pt.sel.drag = D_NONE;
        if pt.sel.sel_on {
            sel_done(pt, ui);
        }
    }
    sel_keys(pt, ui);
    let drag = pt.sel.drag;
    if drag != D_RECT && drag != D_LASSO {
        if !pt.sel.sel_on {
            return;
        }
        // floating pixels, clipped to the image
        if pt.sel.float {
            let g = pt.sel.geo(pt);
            let r = g.scr();
            let c = canvas_rect(pt);
            let x0 = r[0].max(c[0]);
            let y0 = r[1].max(c[1]);
            let x1 = c[0].wrapping_add(c[2]).min(r[2]);
            let y1 = c[1].wrapping_add(c[3]).min(r[3]);
            let w = x1.wrapping_sub(x0);
            let h = y1.wrapping_sub(y0);
            if w > 0 && h > 0 {
                let s = &pt.sel;
                *s.float_draw.img.borrow_mut() = s.fimg.clone();
                *s.float_draw.scr.borrow_mut() = r;
                let f: DrawRef = s.float_draw.clone();
                ui.cmd_custom(x0, y0, w, h, Some(f), 0, s.ver);
            }
        }
    }
    // outline: bounding box on screen (+ 2 px for the ants outside it)
    let s = &pt.sel;
    let g = s.geo(pt);
    let (a, b) = if s.drag == D_LASSO {
        (g.fx_scr(s.lb[0], s.lb[1]), g.fx_scr(s.lb[2], s.lb[3]))
    } else {
        let r = g.scr();
        ((r[0], r[1]), (r[2], r[3]))
    };
    {
        let mut sn = s.ants_draw.snap.borrow_mut();
        sn.g = g;
        sn.drag = s.drag;
        sn.sel_inv = s.sel_inv;
        sn.sel_shape = s.sel_shape;
        sn.ix = s.ix;
        sn.iy = s.iy;
        sn.iw = s.iw;
        sn.ih = s.ih;
        if sn.pts_ver != Some(s.ver) {
            sn.pts.clear();
            sn.pts.extend_from_slice(&s.pts);
            sn.pts_ver = Some(s.ver);
        }
    }
    let f: DrawRef = s.ants_draw.clone();
    ui.cmd_custom(a.0 - 2, a.1 - 2, b.0 - a.0 + 5, b.1 - a.1 + 5, Some(f), 0, s.ver);
    // handles
    if s.sel_on && s.drag != D_RECT && !s.sel_inv {
        draw_sel_handles(ui, &g);
    }
}

// ------------------------------------------------------------------ ribbon

/// select_ribbon() -- a chevron in the lower half of the big "Auswahl" button (placed just
/// before) opens the flyout: rectangle / free-form, select all, invert, key out
/// colour 2, delete
pub(crate) fn select_ribbon(pt: &mut Kritzel, ui: &mut Ui) {
    let r = ui.last_rect();
    let ch = ui.dp(22); // chevron height
    let m = ui.dp(2);
    ui.next_rect_px(r[0] + m, r[1] + r[3] - ch - m, r[2] - m - m, ch);
    ui.next_flags(ST_NOFOCUS);
    if ui.icon_button(ID_SELECT, IC_CHEV_DOWN_SMALL, "Auswahloptionen") & EV_ACT != 0 {
        ui.popup_open(ID_SELECT);
    }
    let s = &pt.sel;
    if ui.flyout_begin(ID_SELECT, FL_BELOW) {
        // shapes
        let mark_mode = |ui: &mut Ui, icon: u32, mode: u32| {
            ui.next_icon(icon);
            if mode == s.mode {
                ui.next_flags(ST_CHECKED);
            }
        };
        let need_sel = |ui: &mut Ui| {
            if !s.sel_on {
                ui.next_flags(ST_DISABLED);
            }
        };
        mark_mode(ui, IC_SELECT_RECT, 0);
        if ui.menu_item(ID_SEL_RECT, "Rechteckige Auswahl") & EV_ACT != 0 {
            ui.cmd_run(CMD_SEL_MODE, 0);
        }
        mark_mode(ui, IC_LASSO, 1);
        if ui.menu_item(ID_SEL_FREE, "Freihandauswahl") & EV_ACT != 0 {
            ui.cmd_run(CMD_SEL_MODE, 1);
        }
        ui.menu_sep();
        // whole image
        ui.next_icon(IC_SELECT_ALL);
        if ui.menu_item(ID_SEL_ALL, "Alle auswählen") & EV_ACT != 0 {
            ui.cmd_run(CMD_SELECT_ALL, 0);
        }
        need_sel(ui);
        ui.next_icon(IC_SWITCH);
        if ui.menu_item(ID_SEL_INV, "Auswahl umkehren") & EV_ACT != 0 {
            ui.cmd_run(CMD_SEL_INVERT, 0);
        }
        ui.menu_sep();
        // options
        if s.transp != 0 {
            ui.next_flags(ST_CHECKED);
        }
        ui.next_icon(IC_GRID);
        if ui.menu_item(ID_SEL_TRANSP, "Hintergrundfarbe aussparen") & EV_ACT != 0 {
            ui.cmd_run(CMD_SEL_TRANSP, 0);
        }
        need_sel(ui);
        ui.next_icon(IC_DELETE);
        if ui.menu_item(ID_SEL_DEL, "Löschen") & EV_ACT != 0 {
            ui.cmd_run(CMD_DELETE, 0);
        }
    }
    ui.flyout_end();
}

// ------------------------------------------------------------------ commands

/// select_cmd(id, arg) -> true handled -- the edit commands are left to the other tool modules
/// while the text tool is active, and without a selection (copy, cut, delete) or without an
/// image on the clipboard (paste)
pub(crate) fn select_cmd(pt: &mut Kritzel, ui: &mut Ui, id: u32, arg: u32) -> bool {
    if (CMD_CUT..=CMD_DELETE).contains(&id) {
        if tool(pt) == TOOL_TEXT {
            return false;
        }
        if id == CMD_PASTE {
            return sel_paste(pt, ui);
        }
        if id == CMD_SELECT_ALL {
            sel_select_all(pt, ui);
            return true;
        }
        if !pt.sel.sel_on {
            return false;
        }
        if id != CMD_DELETE {
            sel_copy(pt, ui);
            if id != CMD_CUT {
                return true;
            }
        }
        sel_delete(pt, ui);
        return true;
    }
    match id {
        CMD_CROP => sel_crop(pt, ui),
        CMD_SEL_INVERT => {
            sel_invert(pt, ui);
            true
        }
        CMD_SEL_TRANSP => {
            let s = &mut pt.sel;
            s.transp ^= 1;
            s.ver = s.ver.wrapping_add(1);
            if s.float {
                sel_rebuild(pt);
            }
            true
        }
        CMD_SEL_MODE => {
            pt.sel.mode = (arg != 0) as u32;
            tool_set(pt, ui, TOOL_SELECT);
            true
        }
        _ => false,
    }
}

/// select_pre_cmd(id, arg) -> true when the command was consumed -- called by pt_cmd before it
/// handles anything
pub(crate) fn select_pre_cmd(pt: &mut Kritzel, ui: &mut Ui, id: u32, _arg: u32) -> bool {
    if !pt.sel.sel_on || id >= 0x7FFF_0000 {
        return false; // UIC_* notifications
    }
    if id == CMD_UNDO {
        // Ctrl+Z: a floating selection goes back; otherwise just deselect and undo
        if pt.sel.float {
            sel_cancel(pt, ui);
            return true;
        }
        pt.sel.clear();
        return false;
    }
    if (CMD_ROT_R..=CMD_FLIP_V).contains(&id) {
        return sel_xform(pt, ui, id);
    }
    if id == CMD_CROP {
        return sel_crop(pt, ui);
    }
    // commands of this module and view-only commands keep the selection
    if !KEEP_CMDS.contains(&id) {
        sel_done(pt, ui);
    }
    false
}

/// ------------------------------------------------------------------ paint-core glue

fn colors(pt: &Kritzel) -> (u32, u32) {
    (pt.tools.col1, pt.tools.col2)
}

// zoom, canvas x, y, w, h
fn view(pt: &Kritzel) -> (i32, i32, i32, i32, i32) {
    let v = &pt.view;
    (v.zoom, v.cx, v.cy, v.cw, v.ch)
}

fn canvas_rect(pt: &Kritzel) -> [i32; 4] {
    crate::view::view_canvas_rect(&pt.view)
}

fn tool(pt: &Kritzel) -> i32 {
    pt.tools.tool
}

fn tool_set(pt: &mut Kritzel, _ui: &mut Ui, t: i32) {
    crate::tools::tool_set(&mut pt.tools, t);
}

fn premul(argb: u32) -> u32 {
    crate::tools::premul(argb)
}

fn view_to_doc(pt: &Kritzel, x: i32, y: i32) -> (i32, i32) {
    crate::view::view_to_doc(&pt.view, x, y)
}

fn view_to_doc_fx(pt: &Kritzel, x: i32, y: i32) -> (i32, i32) {
    crate::view::view_to_doc_fx(&pt.view, x, y)
}

fn doc_w(pt: &Kritzel) -> i32 {
    crate::doc::doc_w(&pt.doc)
}

fn doc_h(pt: &Kritzel) -> i32 {
    crate::doc::doc_h(&pt.doc)
}

fn cur_layer(pt: &Kritzel) -> i32 {
    pt.doc.cur_layer
}

fn nlayers(pt: &Kritzel) -> i32 {
    pt.doc.nlayers
}

fn layer_mut(pt: &mut Kritzel, i: i32) -> &mut Surface {
    crate::doc::doc_layer(&mut pt.doc, i)
}

fn doc_mark(pt: &mut Kritzel, x0: i32, y0: i32, x1: i32, y1: i32) {
    crate::doc::doc_mark(&mut pt.doc, x0, y0, x1, y1);
}

fn hist_begin(pt: &mut Kritzel, layer: i32) -> i32 {
    crate::history::hist_begin(pt, layer)
}

fn hist_end(pt: &mut Kritzel, x0: i32, y0: i32, x1: i32, y1: i32) {
    crate::history::hist_end(pt, x0, y0, x1, y1);
}

fn hist_cancel(pt: &mut Kritzel) {
    crate::history::hist_cancel(pt);
}

fn op_canvas(pt: &mut Kritzel, _ui: &mut Ui, w: i32, h: i32) {
    crate::imageops::op_canvas(pt, w, h);
}

fn op_crop(pt: &mut Kritzel, _ui: &mut Ui, x: i32, y: i32, w: i32, h: i32) {
    crate::imageops::op_crop(pt, x, y, w, h);
}

fn paint_error(pt: &mut Kritzel, _ui: &mut Ui, msg: &'static str) {
    crate::fileio::paint_error(pt, msg);
}
