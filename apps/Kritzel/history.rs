// history -- undo / redo  [apps/paint/src/history.s]
//
//   hist_begin(p, layer) -> 0 / -1      before editing a layer: snapshot it
//   hist_end(p, x0, y0, x1, y1)          after editing: keep only the changed rectangle
//   hist_cancel(p)                       drop the snapshot without recording
//   hist_doc_push(p) -> 0 / -1           before a whole-document change (resize, rotate,
//                                        layer add/remove...): save every layer
//   hist_undo(p) -> true if something was undone
//   hist_redo(p) -> true if something was redone
//   hist_can_undo(h)   hist_can_redo(h)
//   hist_clear(h)
//
// Entry: type (1 = rect, 2 = document), layer, x, y, w, h, the pixels before (packed), the
// pixels after (packed, taken when the entry is first undone), the document snapshot
// (type 2: w, h, nlayers, cur, vis[16], opac[16], copies of every layer surface).

use alloc::boxed::Box;
use alloc::vec::Vec;

use gfx::{Surface, FMT_PARGB, SFF_OWNED};

use crate::consts::*;
use crate::doc::{doc_h, doc_layer, doc_mark, doc_mark_all, doc_set_size, doc_w};
use crate::tools::tool_abort;
use crate::Kritzel;

const H_MAX: usize = 64;

/// a document snapshot (DS_*)
struct Snap {
    w: i32,
    h: i32,
    n: i32,
    cur: i32,
    vis: [u8; MAX_LAYERS],
    opac: [u8; MAX_LAYERS],
    layers: [Surface; MAX_LAYERS],
}

/// one history entry (HE_*)
#[derive(Default)]
struct Entry {
    typ: u32,
    layer: i32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
    doc: Option<Box<Snap>>,
}

/// the undo history (the .bss of history.s); entries.len() = g_hist_n
#[derive(Default)]
pub(crate) struct Hist {
    entries: Vec<Entry>,
    pos: usize,
    /// the snapshot of hist_begin: the whole layer, packed (w * 4 bytes per row)
    snap: Vec<u8>,
    snap_layer: i32,
    snap_w: i32,
    snap_h: i32,
    snap_valid: bool,
}

/// hist_clear()
pub(crate) fn hist_clear(h: &mut Hist) {
    h.entries.clear();
    h.pos = 0;
}

// drop_redo() -- forget entries after the current position
fn drop_redo(h: &mut Hist) {
    h.entries.truncate(h.pos);
}

// new_entry() -> index of a cleared entry at the end (drops the oldest when full)
fn new_entry(h: &mut Hist) -> usize {
    drop_redo(h);
    if h.entries.len() >= H_MAX {
        h.entries.remove(0);
        h.pos -= 1;
    }
    h.entries.push(Entry::default());
    h.pos += 1;
    h.entries.len() - 1
}

// copy_rect(packed, sf, x, y, w, h, to_surface) -- to_surface false: surface rect -> packed
// buffer; true: packed buffer -> surface rect
fn copy_out(buf: &mut [u8], sf: &Surface, x: i32, y: i32, w: i32, h: i32) {
    let n = w as usize * 4;
    let s = sf.stride as usize;
    for r in 0..h as usize {
        let o = (y as usize + r) * s + x as usize * 4;
        buf[r * n..r * n + n].copy_from_slice(&sf.px[o..o + n]);
    }
}

// copy_rect(.., to_surface = 0) into a new packed buffer
fn pack_rect(sf: &Surface, x: i32, y: i32, w: i32, h: i32) -> Vec<u8> {
    let n = w as usize * 4;
    let s = sf.stride as usize;
    let mut v = Vec::with_capacity(n * h as usize);
    for r in 0..h as usize {
        let o = (y as usize + r) * s + x as usize * 4;
        v.extend_from_slice(&sf.px[o..o + n]);
    }
    v
}

fn copy_in(buf: &[u8], sf: &mut Surface, x: i32, y: i32, w: i32, h: i32) {
    let n = w as usize * 4;
    let s = sf.stride as usize;
    for r in 0..h as usize {
        let o = (y as usize + r) * s + x as usize * 4;
        sf.px[o..o + n].copy_from_slice(&buf[r * n..r * n + n]);
    }
}

/// hist_begin(layer) -> 0 / -1 -- snapshot the whole layer
pub(crate) fn hist_begin(p: &mut Kritzel, layer: i32) -> i32 {
    let h = &mut p.hist;
    h.snap_layer = layer;
    let sf = doc_layer(&mut p.doc, layer);
    let size = (sf.w as usize) * (sf.h as usize) * 4;
    if size > h.snap.len() {
        h.snap = alloc::vec![0u8; size];
    }
    copy_out(&mut h.snap, sf, 0, 0, sf.w, sf.h);
    h.snap_w = sf.w; // the snapshot's geometry (hist_end checks it)
    h.snap_h = sf.h;
    h.snap_valid = true;
    0
}

/// hist_cancel()
pub(crate) fn hist_cancel(p: &mut Kritzel) {
    p.hist.snap_valid = false;
}

/// hist_end(x0, y0, x1, y1) -- record the changed rectangle of the snapshot layer
pub(crate) fn hist_end(p: &mut Kritzel, x0: i32, y0: i32, x1: i32, y1: i32) {
    let h = &mut p.hist;
    if !h.snap_valid {
        return;
    }
    h.snap_valid = false;
    // the snapshot must still describe this layer: an undo, resize or layer operation during
    // the stroke may have replaced it
    if h.snap_layer as u32 >= p.doc.nlayers as u32 {
        return;
    }
    let sf = doc_layer(&mut p.doc, h.snap_layer);
    if sf.w != h.snap_w || sf.h != h.snap_h {
        return;
    }
    // clamp
    let x0 = x0.max(0);
    let y0 = y0.max(0);
    let w = x1.min(sf.w).wrapping_sub(x0);
    if w <= 0 {
        return;
    }
    let hh = y1.min(sf.h).wrapping_sub(y0);
    if hh <= 0 {
        return;
    }
    let i = new_entry(h);
    // copy the rectangle out of the snapshot (a packed layer image)
    let n = w as usize * 4;
    let mut before = Vec::with_capacity(n * hh as usize);
    let sw = sf.w as usize;
    for r in 0..hh as usize {
        let o = ((y0 as usize + r) * sw + x0 as usize) * 4;
        before.extend_from_slice(&h.snap[o..o + n]);
    }
    let e = &mut h.entries[i];
    e.typ = 1;
    e.layer = h.snap_layer;
    e.x = x0;
    e.y = y0;
    e.w = w;
    e.h = hh;
    e.before = Some(before);
}

// snap_doc() -> document snapshot
fn snap_doc(p: &mut Kritzel) -> Box<Snap> {
    let d = &p.doc;
    let mut s = Box::new(Snap {
        w: doc_w(d),
        h: doc_h(d),
        n: d.nlayers,
        cur: d.cur_layer,
        vis: d.layer_vis,
        opac: d.layer_opac,
        layers: core::array::from_fn(|_| Surface::default()),
    });
    for i in 0..d.nlayers as usize {
        // an exact copy (surf_alloc + mem_copy of stride * h bytes; no zero fill first)
        let l = &d.layers[i];
        s.layers[i] = Surface { px: l.px.clone(), w: l.w, h: l.h, stride: l.stride, fmt: FMT_PARGB, flags: SFF_OWNED, opac: 255, blend: 0 };
    }
    s
}

// snap_swap(snap) -- exchange the document state with a snapshot (both directions)
fn snap_swap(p: &mut Kritzel, s: &mut Snap) {
    // swap layer descriptors (owned pixel blocks move with them)
    let d = &mut p.doc;
    core::mem::swap(&mut s.layers, &mut d.layers);
    core::mem::swap(&mut s.n, &mut d.nlayers);
    core::mem::swap(&mut s.cur, &mut d.cur_layer);
    core::mem::swap(&mut s.vis, &mut d.layer_vis);
    core::mem::swap(&mut s.opac, &mut d.layer_opac);
    // size: resize the composite if it changed
    let (w, h) = (doc_w(d), doc_h(d));
    let (nw, nh) = (s.w, s.h);
    s.w = w;
    s.h = h;
    doc_set_size(p, nw, nh);
    doc_mark_all(&mut p.doc);
}

// swap with the snapshot of entry i (taken out while the document changes)
fn entry_swap(p: &mut Kritzel, i: usize) {
    if let Some(mut s) = p.hist.entries[i].doc.take() {
        snap_swap(p, &mut s);
        p.hist.entries[i].doc = Some(s);
    }
}

/// hist_doc_push() -> 0 / -1 -- record the whole document before a structural change
pub(crate) fn hist_doc_push(p: &mut Kritzel) -> i32 {
    let s = snap_doc(p);
    let i = new_entry(&mut p.hist);
    let e = &mut p.hist.entries[i];
    e.typ = 2;
    e.doc = Some(s);
    0
}

pub(crate) fn hist_can_undo(h: &Hist) -> bool {
    h.pos != 0
}

pub(crate) fn hist_can_redo(h: &Hist) -> bool {
    h.pos < h.entries.len()
}

/// hist_undo() -> true if undone
pub(crate) fn hist_undo(p: &mut Kritzel) -> bool {
    tool_abort(p); // a stroke in progress ends here
    if p.hist.pos == 0 {
        return false;
    }
    p.hist.pos -= 1;
    let i = p.hist.pos;
    if p.hist.entries[i].typ == 2 {
        entry_swap(p, i);
        return true;
    }
    let e = &mut p.hist.entries[i];
    let sf = doc_layer(&mut p.doc, e.layer);
    // keep the current pixels for redo (once)
    if e.after.is_none() {
        e.after = Some(pack_rect(sf, e.x, e.y, e.w, e.h));
    }
    if let Some(b) = &e.before {
        copy_in(b, sf, e.x, e.y, e.w, e.h);
    }
    let (x, y, w, h) = (e.x, e.y, e.w, e.h);
    doc_mark(&mut p.doc, x, y, x + w, y + h);
    true
}

/// hist_redo() -> true if redone
pub(crate) fn hist_redo(p: &mut Kritzel) -> bool {
    tool_abort(p);
    let i = p.hist.pos;
    if i >= p.hist.entries.len() {
        return false;
    }
    p.hist.pos += 1;
    if p.hist.entries[i].typ == 2 {
        entry_swap(p, i);
        return true;
    }
    let e = &p.hist.entries[i];
    let Some(a) = &e.after else { return false };
    let sf = doc_layer(&mut p.doc, e.layer);
    copy_in(a, sf, e.x, e.y, e.w, e.h);
    let (x, y, w, h) = (e.x, e.y, e.w, e.h);
    doc_mark(&mut p.doc, x, y, x + w, y + h);
    true
}
