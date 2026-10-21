// imageops -- document-wide image operations and layer management (undoable)
//                                                         [apps/paint/src/imageops.s]
//
//   op_flip_h(p)   op_flip_v(p)   op_rotate180(p)   op_rotate90(p, dir)
//   op_scale(p, w, h) -> 0 / -1         resample every layer (bilinear)
//   op_canvas(p, w, h) -> 0 / -1        change the canvas size (keeps the top-left)
//   op_crop(p, x, y, w, h) -> 0 / -1
//   op_skew(p, kx, ky) -> 0 / -1        shear in 1/1000 (tan of the angle)
//   layer_add(p) -> index / -1          new transparent layer above the current one
//   layer_delete(p)                     remove the current layer (never the last one)
//   layer_duplicate(p) -> index / -1
//   layer_merge_down(p)                 current layer onto the one below
//   layer_move(p, dir)                  +1 = up, -1 = down
//   layer_toggle(p, i)                  visibility

use gfx::{Dc, Surface, FMT_PARGB};
use img_core::ops;

use crate::consts::*;
use crate::doc::{doc_h, doc_mark_all, doc_set_size, doc_w};
use crate::history::hist_doc_push;
use crate::tools::premul;
use crate::Kritzel;

// each_layer_inplace(fn) -- call fn(layer_sf) for every layer
fn each_layer_inplace(p: &mut Kritzel, f: fn(&mut Surface)) {
    hist_doc_push(p);
    for i in 0..p.doc.nlayers as usize {
        f(&mut p.doc.layers[i]);
    }
    doc_mark_all(&mut p.doc);
}

pub(crate) fn op_flip_h(p: &mut Kritzel) {
    each_layer_inplace(p, ops::flip_h);
}

pub(crate) fn op_flip_v(p: &mut Kritzel) {
    each_layer_inplace(p, ops::flip_v);
}

pub(crate) fn op_rotate180(p: &mut Kritzel) {
    each_layer_inplace(p, ops::rotate180);
}

/// replace_layers' operations
#[derive(Clone, Copy)]
enum Op {
    Rotate90(i32),
    Scale(i32, i32),
    Canvas(i32, i32),
    /// x, y, w | h << 16
    Crop(i32, i32, u32),
    Skew(i32, i32),
}

// replace_layers(op) -> 0 / -1 -- rebuild every layer through an img_* call.  A result larger
// than DOC_MAX x DOC_MAX (or smaller than 1 x 1) is refused before anything changes: several
// buffers (brush rows, ...) are sized for DOC_MAX.
fn replace_layers(p: &mut Kritzel, op: Op) -> i32 {
    if !result_size_ok(p, op) {
        return -1;
    }
    if hist_doc_push(p) != 0 {
        return -1;
    }
    for i in 0..p.doc.nlayers as usize {
        // the background layer grows with the secondary colour, others transparent
        let fill = if i == 0 { premul(p.tools.col2) } else { 0 };
        let src = &p.doc.layers[i];
        let r = match op {
            Op::Rotate90(dir) => ops::rotate90(src, dir),
            Op::Scale(w, h) => ops::scale(src, w, h, true),
            Op::Canvas(w, h) => ops::resize_canvas(src, w, h, fill),
            Op::Crop(x, y, c) => ops::crop(src, x, y, (c & 0xFFFF) as i32, (c >> 16) as i32),
            Op::Skew(kx, ky) => ops::skew(src, kx, ky, fill),
        };
        if let Ok(n) = r {
            p.doc.layers[i] = n; // out of memory: keep the old layer
        }
    }
    let (w, h) = (p.doc.layers[0].w, p.doc.layers[0].h);
    doc_set_size(p, w, h);
    0
}

// result_size_ok(op) -> true if the operation gives a document within 1..DOC_MAX on both sides
fn result_size_ok(p: &Kritzel, op: Op) -> bool {
    let (w, h) = match op {
        Op::Scale(w, h) | Op::Canvas(w, h) => (w, h),
        Op::Skew(kx, ky) => {
            // like img_skew: w + |kx * h / 1000|, h + |ky * w / 1000| (64-bit)
            let dw = doc_w(&p.doc) as u32 as i64;
            let dh = doc_h(&p.doc) as u32 as i64;
            let a = (kx as i64).wrapping_mul(dh) / 1000;
            let nw = a.wrapping_abs().wrapping_add(dw);
            let b = (ky as i64).wrapping_mul(dw) / 1000;
            let nh = b.wrapping_abs().wrapping_add(dh);
            if nw as u64 > DOC_MAX as u64 || nh as u64 > DOC_MAX as u64 {
                return false;
            }
            (nw as i32, nh as i32)
        }
        _ => return true, // rotate / crop never grow
    };
    (1..=DOC_MAX).contains(&w) && (1..=DOC_MAX).contains(&h)
}

/// op_rotate90(dir)
pub(crate) fn op_rotate90(p: &mut Kritzel, dir: i32) {
    replace_layers(p, Op::Rotate90(dir));
}

/// op_scale(w, h) -> 0 / -1 (larger than DOC_MAX: refused)
pub(crate) fn op_scale(p: &mut Kritzel, w: i32, h: i32) -> i32 {
    replace_layers(p, Op::Scale(w, h))
}

/// op_canvas(w, h) -> 0 / -1 (larger than DOC_MAX: refused)
pub(crate) fn op_canvas(p: &mut Kritzel, w: i32, h: i32) -> i32 {
    replace_layers(p, Op::Canvas(w, h))
}

/// op_crop(x, y, w, h) -> 0 / -1
pub(crate) fn op_crop(p: &mut Kritzel, x: i32, y: i32, w: i32, h: i32) -> i32 {
    replace_layers(p, Op::Crop(x, y, ((h as u32) << 16) | w as u32))
}

/// op_skew(kx, ky) -> 0 / -1 (result larger than DOC_MAX: refused)
pub(crate) fn op_skew(p: &mut Kritzel, kx: i32, ky: i32) -> i32 {
    replace_layers(p, Op::Skew(kx, ky))
}

// ------------------------------------------------------------------ layers

/// layer_add() -> new index or -1
pub(crate) fn layer_add(p: &mut Kritzel) -> i32 {
    if p.doc.nlayers as usize >= MAX_LAYERS {
        return -1;
    }
    hist_doc_push(p);
    let i = (p.doc.cur_layer + 1) as usize; // insert above the current layer
    make_room(p, i);
    let d = &mut p.doc;
    d.layers[i] = Surface::new(doc_w(d), doc_h(d), FMT_PARGB);
    d.layer_vis[i] = 1;
    d.layer_opac[i] = 255;
    d.cur_layer = i as i32;
    doc_mark_all(d);
    i as i32
}

// make_room(i) -- shift layers i.. up by one, nlayers++
fn make_room(p: &mut Kritzel, i: usize) {
    let d = &mut p.doc;
    let n = d.nlayers as usize;
    let mut s = n;
    while s > i {
        // move_layer_slot(s - 1): descriptor, vis, opacity to slot s
        d.layers.swap(s - 1, s);
        d.layer_vis[s] = d.layer_vis[s - 1];
        d.layer_opac[s] = d.layer_opac[s - 1];
        s -= 1;
    }
    d.layers[i] = Surface::default();
    d.nlayers += 1;
}

// remove slot i: shift the slots above down, nlayers--, the freed top slot empty
fn remove_slot(p: &mut Kritzel, i: usize) {
    let d = &mut p.doc;
    d.layers[i] = Surface::default();
    let mut s = i;
    while s + 1 < d.nlayers as usize {
        d.layers.swap(s, s + 1);
        d.layer_vis[s] = d.layer_vis[s + 1];
        d.layer_opac[s] = d.layer_opac[s + 1];
        s += 1;
    }
    d.nlayers -= 1;
    d.layers[d.nlayers as usize] = Surface::default();
}

/// layer_delete() -- remove the current layer
pub(crate) fn layer_delete(p: &mut Kritzel) {
    if p.doc.nlayers as u32 <= 1 {
        return;
    }
    hist_doc_push(p);
    let i = p.doc.cur_layer as usize;
    remove_slot(p, i);
    let d = &mut p.doc;
    if d.cur_layer > d.nlayers - 1 {
        d.cur_layer = d.nlayers - 1;
    }
    doc_mark_all(d);
}

/// layer_duplicate() -> new index or -1
pub(crate) fn layer_duplicate(p: &mut Kritzel) -> i32 {
    let r = p.doc.cur_layer as usize;
    let i = layer_add(p);
    if i < 0 {
        return i;
    }
    let d = &mut p.doc;
    let n = d.layers[r].stride as usize * d.layers[r].h as usize;
    let (a, b) = d.layers.split_at_mut(i as usize);
    b[0].px[..n].copy_from_slice(&a[r].px[..n]);
    d.layer_opac[i as usize] = d.layer_opac[r];
    doc_mark_all(d);
    i
}

/// layer_merge_down() -- blend the current layer onto the one below, then delete it
pub(crate) fn layer_merge_down(p: &mut Kritzel) {
    let b = p.doc.cur_layer as usize;
    if b == 0 {
        return;
    }
    hist_doc_push(p);
    let d = &mut p.doc;
    let (lo, hi) = d.layers.split_at_mut(b);
    let dst = &mut lo[b - 1];
    let mut dc = Dc::new(dst.w, dst.h);
    dc.set_opacity(d.layer_opac[b] as u32);
    dc.blit(dst, &hi[0], 0, 0);
    // delete without a second history entry
    remove_slot(p, b);
    p.doc.cur_layer -= 1;
    doc_mark_all(&mut p.doc);
}

/// layer_move(dir) -- +1 moves the current layer up, -1 down
pub(crate) fn layer_move(p: &mut Kritzel, dir: i32) {
    let b = p.doc.cur_layer;
    let t = b.wrapping_add(dir); // target index
    if t < 0 || t >= p.doc.nlayers {
        return;
    }
    hist_doc_push(p);
    let d = &mut p.doc;
    let (b, t) = (b as usize, t as usize);
    d.layers.swap(b, t);
    d.layer_vis.swap(b, t);
    d.layer_opac.swap(b, t);
    d.cur_layer = t as i32;
    doc_mark_all(d);
}

/// layer_toggle(i) -- show / hide
pub(crate) fn layer_toggle(p: &mut Kritzel, i: i32) {
    p.doc.layer_vis[i as usize] ^= 1;
    doc_mark_all(&mut p.doc);
}
