//! Undo/redo operations and their memory accounting.
pub(crate) mod diagnostics;
pub(crate) mod memory;

// hist -- PaintCraft: Verlauf (undo / redo) -- one history per document (DOC_HIST block) with
// swap semantics: an entry holds the "other" state (pixel tiles, layer properties, the LAYER
// array + document size, a selection blob); undo and redo exchange it with the document, so
// every entry costs the changed data once.      [apps/PaintCraft/src/hist.s, inc/pn_hist.inc]
//
// Module (main.rs tables):
//   hist_modinit()  hist_states()  hist_frame()  hist_cmd(id, arg) -> handled
//                   (CMD_UNDO, CMD_REDO, CMD_HIST_CLEAR, CMD_HIST_REWIND, CMD_HIST_FFWD,
//                   CMD_HIST_JUMP (arg = entry index) of the active document)
// Recording (call BEFORE the change; false = out of memory: do not make it, the message is
// shown already):
//   hist_px_begin(doc, layer) -> 0 | ERR_*   hist_px_touch(doc, x0, y0, x1, y1) -> 0 | ERR_NOMEM
//   hist_px_end(doc, name, icon) -> step     hist_px_cancel(doc)   hist_px_active(doc)
//   hist_push_layer_props(doc, i, name, icon)   hist_push_layers / hist_push_doc(doc, name, icon)
//   hist_push_sel(doc, name, icon)   hist_begin_compound(doc)   hist_end_compound(doc, name, icon)
// Navigation and queries: hist_init, hist_undo, hist_redo, hist_jump, hist_count, hist_pos,
//   hist_can_undo, hist_can_redo, hist_entry, hist_entry_name, hist_entry_icon, hist_clear,
//   hist_free, hist_mark_clean
// Resources: hist_release(doc, rec) -- see below
// Memory (hist_mem.rs): hist_enforce, hist_reclaim; messages: hist_notify_set, hist_message,
//   hist_message_clear
//
// Layer resources (pn_hist.inc RESOURCE OWNERSHIP).  In the asm, structure entries keep LAYER
// records by value and share the pixel buffers with the document.  Here a Layer owns its
// surface, so an entry keeps an `HLayer`: the record's fields and the identity of its pixel
// buffer (`tok` = the buffer's address, the asm's SF_PX).  A buffer the entries refer to lives
// either in a document layer or -- once the document stopped using it (hist_release, or a
// layer array swapped out by undo / redo) -- in the document's history pool (`DocHist::pool`);
// applying an entry takes the buffers it names from the outgoing layers / the pool.  The pool
// keeps only buffers some entry still names (checked when an entry dies), like the asm's
// hist_ref.  Names and metadata blobs are copied (immutable in the document), caches never
// enter entries.  The byte accounting counts what the asm counts (HE_XB: surface bytes only
// the history keeps; the LAYER arrays with the asm's DOC_LCAP growth).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gfx::Surface;
use img_core::{ERR_FAIL, ERR_NOMEM};
use ui::*;

use crate::app::constants::*;
use crate::document::model::{doc_active, doc_mark_layers, doc_mark_px, doc_resized, doc_set_modified, Doc, DocId};
use crate::document::history::memory::{hm_init, tile_equal, tile_get, tile_put, tile_swap, HTile, HistMem, Pay, HE_SIZE, HTF_NOZ, HT_SIZE, TILE_BYTES, TST_RAW};
use crate::document::layers::Layer;
use crate::document::selection::{sel_hrestore, sel_hsize, sel_hsnap, SelBlob};
use crate::app::commands::pn_cmd_set;
use crate::app::Pn;

const HCAP0: i32 = 64; // first capacity of the entry pointer array
const CHILD0: i32 = 8; // first capacity of a compound's child array
const BATCH_MAX: usize = 16; // layers with a pending doc_mark_px per navigation
const PRED_Z: u32 = 1; // hs_oldest: pixel entry with raw tiles
const PRED_SPILL: u32 = 2; // hs_oldest: pixel entry with tiles in RAM
const LY_SIZE: i64 = 96; // bytes of a LAYER record (accounting)
const LCAP0: i32 = 8; // the asm's first DOC_LCAP (doc_new / lay_insert_rec)
pub(crate) const HDF_CAPTURE: u32 = 1; // a pixel capture is open

// entry kinds (HK_*)
pub(crate) const HK_BASE: u8 = 0; // first entry ("Neues Bild"): no data
pub(crate) const HK_PX: u8 = 1; // pixel tiles of one layer
pub(crate) const HK_PROPS: u8 = 2; // layer properties
pub(crate) const HK_LAYERS: u8 = 3; // LAYER array + document size / resolution
pub(crate) const HK_SEL: u8 = 4; // selection blob
pub(crate) const HK_COMP: u8 = 5; // compound: child entries

const S_NEW_IMAGE: &str = "Neues Bild";
pub(crate) const S_EDIT: &str = "Bearbeiten";
const S_OOM: &str = "Der Arbeitsspeicher reicht für diesen Schritt nicht aus.";
const S_OOM_CLEAR: &str = "Arbeitsspeicher knapp: Der Verlauf wurde geleert.";
const S_DROPPED: &str = "Ältere Verlaufsschritte wurden verworfen, um Arbeitsspeicher freizugeben.";
const S_APPLY_FAIL: &str = "Der Verlaufsschritt konnte nicht wiederhergestellt werden.";

/// a LAYER record kept by a structure entry: the record's fields and the identity of its pixel
/// buffer (the buffer itself is in a document layer or in DocHist::pool)
#[derive(Default)]
pub(crate) struct HLayer {
    /// the pixel buffer's address (0 = no pixels)
    pub(crate) tok: usize,
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) stride: i32,
    pub(crate) fmt: u8,
    pub(crate) sflags: u8,
    pub(crate) opac: u8,
    pub(crate) blend: u8,
    pub(crate) name: String,
    pub(crate) flags: u32,
    pub(crate) uid: u64,
    pub(crate) meta: Vec<u8>,
}

/// the identity of a surface's pixel buffer (SF_PX; 0 = none)
#[inline]
pub(crate) fn sf_tok(sf: &Surface) -> usize {
    if sf.px.is_empty() { 0 } else { sf.px.as_ptr() as usize }
}

impl HLayer {
    // the record of a document layer (caches are never kept)
    fn of(l: &Layer) -> HLayer {
        HLayer {
            tok: sf_tok(&l.sf),
            w: l.sf.w,
            h: l.sf.h,
            stride: l.sf.stride,
            fmt: l.sf.fmt,
            sflags: l.sf.flags,
            opac: l.sf.opac,
            blend: l.sf.blend,
            name: l.name.clone(),
            flags: l.flags,
            uid: l.uid,
            meta: l.meta.clone(),
        }
    }
}

/// one history entry (HE_*): the fields of every kind side by side (the asm overlays them)
#[derive(Default)]
pub(crate) struct HistEntry {
    pub(crate) kind: u8,
    pub(crate) icon: u32,
    /// the name ("" = none: "Bearbeiten")
    pub(crate) name: &'static str,
    /// global creation sequence (oldest = smallest)
    pub(crate) seq: u64,
    /// HE_BYTES: RAM bytes accounted to this entry (wrapping like the asm's u64)
    pub(crate) bytes: u64,
    /// HE_DISK: bytes in the spill file
    pub(crate) disk: u64,
    /// pixels, props: layer index (refreshed from the uid) and uid
    pub(crate) layer: i32,
    pub(crate) uid: u64,
    // ---- HK_PX
    pub(crate) tiles: Vec<HTile>,
    pub(crate) x0: i32,
    pub(crate) y0: i32,
    pub(crate) x1: i32,
    pub(crate) y1: i32,
    pub(crate) lw: i32,
    pub(crate) lh: i32,
    /// tiles in TST_RAW state without HTF_NOZ
    pub(crate) nraw: i32,
    /// tiles holding RAM (TST_RAW + TST_Z)
    pub(crate) nmem: i32,
    // ---- HK_PROPS
    pub(crate) pname: String,
    pub(crate) popac: u8,
    pub(crate) pblend: u8,
    pub(crate) pvis: u32,
    // ---- HK_LAYERS
    pub(crate) lays: Vec<HLayer>,
    pub(crate) lcur: i32,
    pub(crate) lcap: i32,
    pub(crate) dw: i32,
    pub(crate) dh: i32,
    pub(crate) dunit: u8,
    pub(crate) dpux: f64,
    pub(crate) dpuy: f64,
    /// HE_XB: surface bytes only the history keeps
    pub(crate) xb: u64,
    // ---- HK_SEL
    pub(crate) blob: Option<Box<SelBlob>>,
    // ---- HK_COMP
    pub(crate) kids: Vec<Box<HistEntry>>,
    pub(crate) ccap: i32,
}

/// the capture record of a document (HD_CAPT): a pixel entry header and the touched-tile bits
#[derive(Default)]
struct Capt {
    e: HistEntry,
    /// HC_CAP: capacity of the tile array (grows x2 from 64: an allocation point)
    cap: i32,
    /// HC_FAIL: a tile could not be saved
    fail: bool,
    tw: i32,
    th: i32,
    /// HC_BITS (its length = HC_BCAP)
    bits: Vec<u8>,
}

/// the per-document history (DOC_HIST block).  Part of Doc: Send + Sync.
#[derive(Default)]
pub(crate) struct DocHist {
    /// HD_ENT (len = HD_N): entries 0..pos-1 are applied (entry 0 = the base)
    ent: Vec<Box<HistEntry>>,
    /// HD_CAP: capacity of the entry pointer array (an allocation point when it grows)
    cap: i32,
    /// HD_POS
    pos: i32,
    /// HD_CLEAN: pos of the saved state, -1 = unreachable
    clean: i32,
    capt: Option<Box<Capt>>,
    /// HD_COMP: the open compound
    comp: Option<Box<HistEntry>>,
    /// HD_CDEP: compound nesting depth
    cdep: i32,
    /// HD_FLAGS: HDF_*
    pub(crate) flags: u32,
    /// pixel buffers the document no longer uses that entries still name
    pool: Vec<Surface>,
    /// the entry hist_add is adding (it names buffers while the redo branch is destroyed)
    adding: Option<Box<HistEntry>>,
    /// the asm's DOC_LCAP of the document's layer array (0 = LCAP0; accounting only)
    lcap: i32,
}

/// the state of hist.s (its g_* globals)
#[derive(Default)]
pub(crate) struct Hist {
    /// g_hs_seq: last entry sequence number
    seq: u64,
    /// g_hs_last: sequence number of the entry added last (never dropped by the budget)
    last: u64,
    msg: Option<&'static str>,
    msg_sev: u32,
    notify: Option<fn(&mut Pn, u32, &str)>,
    /// g_hs_apply: entries are being applied (every push is ignored)
    apply: bool,
    /// the selection hooks are registered (sel_init)
    sel_on: bool,
    /// g_bt: pending doc_mark_px per layer {layer, x0, y0, x1, y1}
    bt: Vec<[i32; 5]>,
    bt_struct: bool,
    bt_resized: bool,
}

// ================================================================== module

/// hist_modinit() -- once at start-up (main.rs mod_init)
pub(crate) fn hist_modinit(p: &mut Pn, _ui: &mut Ui) {
    hm_init(p);
}

/// hist_states() -- enable Rückgängig / Wiederholen / Alles ... / Verlauf löschen
pub(crate) fn hist_states(p: &mut Pn, ui: &mut Ui) {
    let (mut u, mut r, mut c) = (false, false, false);
    if let Some(doc) = doc_active(&p.doc) {
        hs_base(p, doc); // every shown document has its first entry
        let d = p.doc.d(doc);
        u = hist_can_undo(d);
        r = hist_can_redo(d);
        c = hs_nav_ok(&d.hist) && d.hist.ent.len() > 1;
    }
    let f = |ok: bool| if ok { 0 } else { CF_DISABLED };
    pn_cmd_set(ui, CMD_UNDO, f(u));
    pn_cmd_set(ui, CMD_HIST_REWIND, f(u));
    pn_cmd_set(ui, CMD_REDO, f(r));
    pn_cmd_set(ui, CMD_HIST_FFWD, f(r));
    pn_cmd_set(ui, CMD_HIST_CLEAR, f(c));
}

/// hist_frame() -- the test probe window (hist_diag.rs; only in test mode)
pub(crate) fn hist_frame(p: &mut Pn, ui: &mut Ui) {
    crate::document::history::diagnostics::hist_diag_frame(p, ui);
}

/// hist_cmd(id, arg) -> handled (ids 150-159 of the active document)
pub(crate) fn hist_cmd(p: &mut Pn, _ui: &mut Ui, id: u32, arg: u32) -> bool {
    if !(CMD_HIST_FIRST..=CMD_HIST_LAST).contains(&id) {
        return false;
    }
    let Some(doc) = doc_active(&p.doc) else { return true };
    match id {
        CMD_UNDO => {
            hist_undo(p, doc);
        }
        CMD_REDO => {
            hist_redo(p, doc);
        }
        CMD_HIST_CLEAR => hist_clear(p, doc),
        CMD_HIST_REWIND => {
            hist_jump(p, doc, 0);
        }
        CMD_HIST_FFWD => {
            let n = p.doc.d(doc).hist.ent.len() as i32;
            hist_jump(p, doc, n - 1);
        }
        CMD_HIST_JUMP => {
            hist_jump(p, doc, arg as i32);
        }
        _ => {}
    }
    true
}

// ================================================================== entries

/// ent_new(kind, name, icon) -> entry | None -- HE_BYTES = HE_SIZE, next sequence number
fn ent_new(h: &mut Hist, hm: &mut HistMem, kind: u8, name: &'static str, icon: u32) -> Option<Box<HistEntry>> {
    if hm.fail() {
        return None;
    }
    h.seq += 1;
    hm.ram = hm.ram.wrapping_add(HE_SIZE);
    Some(Box::new(HistEntry { kind, name, icon, seq: h.seq, bytes: HE_SIZE, ..HistEntry::default() }))
}

// does e (or a child) name buffer tok?
fn ent_refs(e: &HistEntry, tok: usize) -> bool {
    match e.kind {
        HK_LAYERS => e.lays.iter().any(|l| l.tok == tok),
        HK_COMP => e.kids.iter().any(|k| ent_refs(k, tok)),
        _ => false,
    }
}

// the pixel buffers e (and its children) name
fn ent_toks(e: &HistEntry, out: &mut Vec<usize>) {
    match e.kind {
        HK_LAYERS => out.extend(e.lays.iter().map(|l| l.tok).filter(|&t| t != 0)),
        HK_COMP => {
            for k in &e.kids {
                ent_toks(k, out);
            }
        }
        _ => {}
    }
}

// hist_ref for a pixel buffer: does an entry of the document (its list, the open compound,
// the entry being added) name tok?
fn dh_refs(dh: &DocHist, tok: usize) -> bool {
    dh.ent.iter().any(|e| ent_refs(e, tok))
        || dh.comp.as_ref().is_some_and(|c| ent_refs(c, tok))
        || dh.adding.as_ref().is_some_and(|c| ent_refs(c, tok))
}

/// ent_destroy(doc, e) -- free an entry that is in no list any more; the pixel buffers it
/// names that the document or another entry still uses are kept (hist_release of its
/// records in the asm), the others leave the pool
fn ent_destroy(h: &mut Hist, hm: &mut HistMem, dh: &mut DocHist, mut e: Box<HistEntry>) {
    let mut toks = Vec::new();
    ent_toks(&e, &mut toks);
    ent_free(h, hm, &mut e);
    drop(e);
    if toks.is_empty() || dh.pool.is_empty() {
        return;
    }
    let dead: Vec<usize> = toks.into_iter().filter(|&t| !dh_refs(dh, t)).collect();
    if !dead.is_empty() {
        dh.pool.retain(|s| !dead.contains(&sf_tok(s)));
    }
}

// ent_destroy without the pool: the accounting of e and its children
fn ent_free(h: &mut Hist, hm: &mut HistMem, e: &mut HistEntry) {
    match e.kind {
        HK_PX => {
            for i in 0..e.tiles.len() {
                hm.ht_release(e, i);
            }
            let a = e.tiles.len() as u64 * HT_SIZE;
            e.tiles = Vec::new();
            hm.acc(e, -(a as i64));
        }
        HK_COMP => {
            // (the children from the last one on, like the asm)
            while let Some(mut k) = e.kids.pop() {
                ent_free(h, hm, &mut k);
            }
        }
        _ => {}
    }
    hm.ram = hm.ram.wrapping_sub(e.bytes);
    hm.disk = hm.disk.wrapping_sub(e.disk);
    if h.last == e.seq {
        h.last = 0;
    }
}

/// hist_release(doc, rec) -- the document no longer uses the resources of layer record rec
/// (it already left the document's layer list): the history keeps the pixel buffer when an
/// entry still names it, the rest is dropped
pub(crate) fn hist_release(p: &mut Pn, doc: DocId, rec: Layer) {
    let Some(d) = p.doc.get_mut(doc) else { return };
    let tok = sf_tok(&rec.sf);
    if tok != 0 && dh_refs(&d.hist, tok) {
        d.hist.pool.push(rec.sf);
    }
}

// the asm's DOC_LCAP of the document's layer array: 8, doubled whenever an insert finds the
// array full (layer.s lay_insert_rec), exchanged with the entry's by undo / redo
fn doc_lcap(d: &mut Doc) -> i32 {
    let n = d.layers.len() as i32;
    let mut c = if d.hist.lcap == 0 { LCAP0 } else { d.hist.lcap };
    while n > c {
        c = (c * 2).max(LCAP0);
    }
    d.hist.lcap = c;
    c
}

// ================================================================== the entry list

/// hs_cap(doc) -> 0 | -1 -- room for one more entry pointer
fn hs_cap(hm: &mut HistMem, dh: &mut DocHist) -> i32 {
    if (dh.ent.len() as i32) < dh.cap {
        return 0;
    }
    let n = if dh.cap == 0 { HCAP0 } else { dh.cap * 2 };
    if hm.fail() {
        return -1;
    }
    dh.ent.reserve_exact((n as usize).saturating_sub(dh.ent.len()));
    dh.cap = n;
    0
}

/// hs_new_base(doc, name, icon) -> 0 | -1 -- entry 0 of an empty history
fn hs_new_base(p: &mut Pn, doc: DocId, name: &'static str, icon: u32) -> i32 {
    let Pn { hist, hm, doc: docs, .. } = p;
    let Some(e) = ent_new(hist, hm, HK_BASE, name, icon) else { return -1 };
    let d = docs.dm(doc);
    if hs_cap(hm, &mut d.hist) != 0 {
        ent_destroy(hist, hm, &mut d.hist, e);
        return -1;
    }
    d.hist.ent.push(e);
    d.hist.pos = 1;
    d.hist.clean = 1;
    d.ver_hist = d.ver_hist.wrapping_add(1);
    0
}

/// hs_base(doc) -> 0 | -1 -- start the history ("Neues Bild") if nobody called hist_init
fn hs_base(p: &mut Pn, doc: DocId) -> i32 {
    if !p.doc.d(doc).hist.ent.is_empty() {
        return 0;
    }
    hs_new_base(p, doc, S_NEW_IMAGE, IC_PAGE)
}

/// hist_init(doc, name, icon) -- drop the old history, first entry = name
pub(crate) fn hist_init(p: &mut Pn, doc: DocId, name: &'static str, icon: u32) {
    if p.hist.apply {
        return;
    }
    hs_drop_all(p, doc);
    hs_new_base(p, doc, name, icon);
}

/// hs_drop_all(doc) -- destroy every entry, the open compound and the capture's tiles
fn hs_drop_all(p: &mut Pn, doc: DocId) {
    hs_capt_release(p, doc);
    let Pn { hist, hm, doc: docs, .. } = p;
    let d = docs.dm(doc);
    let dh = &mut d.hist;
    dh.flags &= !HDF_CAPTURE;
    if let Some(c) = dh.comp.take() {
        ent_destroy(hist, hm, dh, c);
    }
    dh.cdep = 0;
    while let Some(e) = dh.ent.pop() {
        ent_destroy(hist, hm, dh, e);
    }
    dh.pos = 0;
    dh.clean = -1;
    d.ver_hist = d.ver_hist.wrapping_add(1);
}

/// hist_add(doc, e) -> true | false (e destroyed) -- a new step: child of the open compound,
/// else a top-level entry after the current one (the redo branch is discarded, HIST-008)
fn hist_add(p: &mut Pn, doc: DocId, e: Box<HistEntry>) -> bool {
    let seq = e.seq;
    if p.doc.d(doc).hist.comp.is_some() {
        let Pn { hist, hm, doc: docs, .. } = p;
        let dh = &mut docs.dm(doc).hist;
        let c = dh.comp.as_mut().unwrap();
        if c.kids.len() as i32 >= c.ccap {
            let n = (c.ccap * 2).max(CHILD0);
            if hm.fail() {
                ent_destroy(hist, hm, dh, e);
                return false;
            }
            let grow = (n - c.ccap) as i64 * 8;
            c.ccap = n;
            hm.acc(c, grow);
        }
        c.kids.push(e);
    } else {
        if hs_base(p, doc) != 0 {
            let Pn { hist, hm, doc: docs, .. } = p;
            ent_destroy(hist, hm, &mut docs.dm(doc).hist, e);
            return false;
        }
        // (the redo branch dies while e is in no list: e's buffers stay referenced)
        p.doc.dm(doc).hist.adding = Some(e);
        hs_discard_redo(p, doc);
        let Pn { hist, hm, doc: docs, .. } = p;
        let dh = &mut docs.dm(doc).hist;
        let e = dh.adding.take().unwrap();
        if hs_cap(hm, dh) != 0 {
            ent_destroy(hist, hm, dh, e);
            return false;
        }
        dh.ent.push(e);
        dh.pos = dh.ent.len() as i32;
        if dh.ent.len() as i32 > HIST_MAX_DEFAULT + 1 {
            hs_drop_first(p, doc);
        }
    }
    p.hist.last = seq;
    let d = p.doc.dm(doc);
    d.ver_hist = d.ver_hist.wrapping_add(1);
    hist_enforce(p);
    true
}

/// hs_discard_redo(doc) -- destroy the entries after the current one
fn hs_discard_redo(p: &mut Pn, doc: DocId) {
    let Pn { hist, hm, doc: docs, .. } = p;
    let dh = &mut docs.dm(doc).hist;
    while dh.ent.len() as i32 > dh.pos {
        let e = dh.ent.pop().unwrap();
        ent_destroy(hist, hm, dh, e);
    }
    if dh.clean > dh.pos {
        dh.clean = -1; // the saved state was in the redo branch
    }
}

/// hs_drop_first(doc) -- forget the oldest applied step (entry 1); needs pos >= 2
fn hs_drop_first(p: &mut Pn, doc: DocId) {
    let Pn { hist, hm, doc: docs, .. } = p;
    let d = docs.dm(doc);
    let dh = &mut d.hist;
    if dh.pos < 2 {
        return;
    }
    let e = dh.ent.remove(1);
    dh.pos -= 1;
    if dh.clean == 1 {
        dh.clean = -1;
    } else if dh.clean > 1 {
        dh.clean -= 1;
    }
    ent_destroy(hist, hm, dh, e);
    d.ver_hist = d.ver_hist.wrapping_add(1);
}

/// hs_drop_last(doc) -- forget the newest redo step
fn hs_drop_last(p: &mut Pn, doc: DocId) {
    let Pn { hist, hm, doc: docs, .. } = p;
    let d = docs.dm(doc);
    let dh = &mut d.hist;
    if dh.ent.len() as i32 <= dh.pos {
        return;
    }
    let e = dh.ent.pop().unwrap();
    if dh.clean > dh.ent.len() as i32 {
        dh.clean = -1;
    }
    ent_destroy(hist, hm, dh, e);
    d.ver_hist = d.ver_hist.wrapping_add(1);
}

// ================================================================== pixel capture

/// hs_layer(doc, index, uid) -> the index of the layer with that uid (the index is tried
/// first) | None
fn hs_layer(d: &Doc, index: i32, uid: u64) -> Option<usize> {
    let ls = &d.layers;
    if (index as u32 as usize) < ls.len() && ls[index as usize].uid == uid {
        return Some(index as usize);
    }
    ls.iter().position(|l| l.uid == uid)
}

/// ht_geom(e, ht) -> byte offset of the tile's first pixel in the layer, vw, vh (valid size of
/// the tile, from the layer size recorded in the entry)
#[inline]
fn ht_geom(e: &HistEntry, ht: &HTile, stride: i32) -> (usize, i32, i32) {
    let x = (ht.tx as i32) << TILE_SH;
    let y = (ht.ty as i32) << TILE_SH;
    let vw = (e.lw - x).min(TILE);
    let vh = (e.lh - y).min(TILE);
    (y as usize * stride as usize + x as usize * 4, vw, vh)
}

/// hist_px_begin(doc, layer) -> 0 | ERR_* -- start saving tiles of layer (-1 = current)
pub(crate) fn hist_px_begin(p: &mut Pn, doc: DocId, layer: i32) -> i32 {
    if p.hist.apply {
        return 0;
    }
    if p.doc.d(doc).hist.flags & HDF_CAPTURE != 0 {
        hist_px_end(p, doc, S_EDIT, IC_EDIT); // a capture is still open: commit it
    }
    let Pn { hm, doc: docs, .. } = p;
    let d = docs.dm(doc);
    let n = d.layers.len() as i32;
    if n <= 0 {
        return ERR_FAIL;
    }
    let mut i = if layer < 0 { d.cur } else { layer };
    if i >= n {
        i = n - 1;
    }
    if i < 0 {
        i = 0;
    }
    if d.hist.capt.is_none() {
        if hm.fail() {
            return ERR_NOMEM;
        }
        let mut c = Box::<Capt>::default();
        c.e.kind = HK_PX;
        d.hist.capt = Some(c);
    }
    let l = &d.layers[i as usize];
    let c = d.hist.capt.as_mut().unwrap();
    c.e.layer = i;
    c.e.uid = l.uid;
    c.e.lw = l.sf.w;
    c.e.lh = l.sf.h;
    c.e.tiles.clear();
    c.e.nraw = 0;
    c.e.nmem = 0;
    c.fail = false;
    c.e.bytes = 0;
    c.e.disk = 0;
    c.tw = (l.sf.w + TILE - 1) >> TILE_SH;
    c.th = (l.sf.h + TILE - 1) >> TILE_SH;
    let nb = (((c.tw.wrapping_mul(c.th).wrapping_add(7)) as u32 >> 3) + 1) as usize;
    if nb > c.bits.len() {
        c.bits = Vec::new();
        if hm.fail() {
            return ERR_NOMEM;
        }
        c.bits = vec![0u8; nb];
    } else {
        c.bits[..nb].fill(0);
    }
    d.hist.flags |= HDF_CAPTURE;
    0
}

/// hs_capt_tile(doc, tx, ty) -> 0 | ERR_NOMEM -- save one tile of the captured layer
fn hs_capt_tile(p: &mut Pn, doc: DocId, tx: i32, ty: i32) -> i32 {
    {
        let Pn { hm, doc: docs, .. } = p;
        let c = docs.dm(doc).hist.capt.as_mut().unwrap();
        if c.e.tiles.len() as i32 >= c.cap {
            let n = (c.cap * 2).max(64);
            if hm.fail() {
                return ERR_NOMEM;
            }
            c.e.tiles.reserve_exact((n as usize).saturating_sub(c.e.tiles.len()));
            c.cap = n;
        }
    }
    let t = match p.hm.tile_alloc() {
        Some(t) => t,
        None => {
            hist_reclaim(p);
            match p.hm.tile_alloc() {
                Some(t) => t,
                None => return ERR_NOMEM,
            }
        }
    };
    let Pn { hm, doc: docs, .. } = p;
    let d = docs.dm(doc);
    let c = d.hist.capt.as_mut().unwrap();
    c.e.tiles.push(HTile { tx: tx as u16, ty: ty as u16, st: TST_RAW, fl: 0, len: 0, color: 0, pay: Pay::Raw(t) });
    c.e.nraw += 1;
    c.e.nmem += 1;
    hm.acc(&mut c.e, TILE_BYTES as i64);
    let l = &d.layers[c.e.layer as usize];
    let k = c.e.tiles.len() - 1;
    let (off, vw, vh) = ht_geom(&c.e, &c.e.tiles[k], l.sf.stride);
    tile_get(&mut c.e.tiles[k].raw_mut()[..], &l.sf.px[off..], l.sf.stride, vw, vh);
    0
}

/// hist_px_touch(doc, x0, y0, x1, y1) -> 0 | ERR_NOMEM -- save the tiles of the rectangle that
/// are not saved yet (call before writing pixels there)
pub(crate) fn hist_px_touch(p: &mut Pn, doc: DocId, x0: i32, y0: i32, x1: i32, y1: i32) -> i32 {
    if p.hist.apply {
        return 0;
    }
    let d = p.doc.d(doc);
    if d.hist.flags & HDF_CAPTURE == 0 {
        return 0;
    }
    let c = d.hist.capt.as_ref().unwrap();
    let (x0, y0) = (x0.max(0), y0.max(0));
    let x1 = x1.min(c.e.lw);
    let y1 = y1.min(c.e.lh);
    if x0 >= x1 || y0 >= y1 {
        return 0;
    }
    // still the captured layer (same index, uid and size)?
    let li = c.e.layer as u32 as usize;
    if li >= d.layers.len() {
        return 0;
    }
    let l = &d.layers[li];
    if l.uid != c.e.uid || l.sf.w != c.e.lw || l.sf.h != c.e.lh {
        return 0;
    }
    let (tx0, tx1) = (x0 >> TILE_SH, (x1 - 1) >> TILE_SH);
    let (ty0, ty1) = (y0 >> TILE_SH, (y1 - 1) >> TILE_SH);
    let tw = c.tw;
    for ty in ty0..=ty1 {
        for tx in tx0..=tx1 {
            let b = (ty * tw + tx) as usize;
            let c = p.doc.dm(doc).hist.capt.as_mut().unwrap();
            if c.bits[b >> 3] & (1 << (b & 7)) != 0 {
                continue;
            }
            c.bits[b >> 3] |= 1 << (b & 7);
            if hs_capt_tile(p, doc, tx, ty) != 0 {
                p.doc.dm(doc).hist.capt.as_mut().unwrap().fail = true;
                return ERR_NOMEM;
            }
        }
    }
    0
}

/// hs_capt_release(doc) -- free the tiles of the capture (no restore)
fn hs_capt_release(p: &mut Pn, doc: DocId) {
    let Pn { hm, doc: docs, .. } = p;
    let Some(c) = docs.dm(doc).hist.capt.as_mut() else { return };
    for i in 0..c.e.tiles.len() {
        hm.ht_release(&mut c.e, i);
    }
    c.e.tiles.clear();
    c.e.nraw = 0;
    c.e.nmem = 0;
    hm.ram = hm.ram.wrapping_sub(c.e.bytes); // 0 unless the accounting drifted
    c.e.bytes = 0;
}

/// hist_px_end(doc, name, icon) -> step recorded -- unchanged tiles are dropped, one-colour
/// tiles shrink to a colour; no changed tile = no entry
pub(crate) fn hist_px_end(p: &mut Pn, doc: DocId, name: &'static str, icon: u32) -> bool {
    if p.hist.apply || p.doc.d(doc).hist.flags & HDF_CAPTURE == 0 {
        return false;
    }
    p.doc.dm(doc).hist.flags &= !HDF_CAPTURE;
    let ok = 'cap: {
        let Pn { hm, doc: docs, .. } = p;
        let Doc { layers, hist: dh, .. } = docs.dm(doc);
        let c = dh.capt.as_mut().unwrap();
        if c.fail {
            break 'cap false;
        }
        let Some(li) = hs_layer_in(layers, c.e.layer, c.e.uid) else { break 'cap false };
        c.e.layer = li as i32;
        let l = &layers[li];
        if l.sf.w != c.e.lw || l.sf.h != c.e.lh {
            break 'cap false;
        }
        let stride = l.sf.stride;
        let e = &mut c.e;
        e.x0 = 0x7FFF_FFFF;
        e.y0 = 0x7FFF_FFFF;
        e.x1 = 0;
        e.y1 = 0;
        let mut w = 0usize;
        for r in 0..e.tiles.len() {
            let (off, vw, vh) = ht_geom(e, &e.tiles[r], stride);
            if tile_equal(&e.tiles[r].raw()[..], &l.sf.px[off..], stride, vw, vh) {
                hm.ht_release(e, r);
                continue;
            }
            hm.ht_solidify(e, r, vw, vh);
            if w != r {
                e.tiles.swap(w, r);
            }
            let (tx, ty) = ((e.tiles[w].tx as i32) << TILE_SH, (e.tiles[w].ty as i32) << TILE_SH);
            if tx < e.x0 {
                e.x0 = tx;
            }
            if tx + vw > e.x1 {
                e.x1 = tx + vw;
            }
            if ty < e.y0 {
                e.y0 = ty;
            }
            if ty + vh > e.y1 {
                e.y1 = ty + vh;
            }
            w += 1;
        }
        e.tiles.truncate(w);
        if w == 0 {
            return false;
        }
        true
    };
    if ok {
        // the entry: the capture's header fields and a right-sized copy of its tile array
        let Pn { hist, hm, doc: docs, .. } = p;
        if let Some(mut e) = ent_new(hist, hm, HK_PX, name, icon) {
            let dh = &mut docs.dm(doc).hist;
            if !hm.fail() {
                let c = dh.capt.as_mut().unwrap();
                let n = c.e.tiles.len();
                e.tiles = c.e.tiles.drain(..).collect(); // the capture keeps its capacity
                e.layer = c.e.layer;
                e.uid = c.e.uid;
                (e.x0, e.y0, e.x1, e.y1) = (c.e.x0, c.e.y0, c.e.x1, c.e.y1);
                (e.lw, e.lh, e.nraw, e.nmem) = (c.e.lw, c.e.lh, c.e.nraw, c.e.nmem);
                e.bytes = e.bytes.wrapping_add(c.e.bytes); // the tiles (already in the RAM total)
                hm.acc(&mut e, (n as u64 * HT_SIZE) as i64);
                c.e.nraw = 0;
                c.e.nmem = 0;
                c.e.bytes = 0;
                if hist_add(p, doc, e) {
                    return true;
                }
                hs_oom_clear(p, doc); // pixels changed without a step
                return false;
            }
            ent_destroy(hist, hm, dh, e);
        }
    }
    hs_capt_release(p, doc);
    hs_oom_clear(p, doc);
    false
}

/// hist_px_cancel(doc) -- put the saved tiles back into the layer, no entry (HIST-019)
pub(crate) fn hist_px_cancel(p: &mut Pn, doc: DocId) {
    if p.hist.apply || p.doc.d(doc).hist.flags & HDF_CAPTURE == 0 {
        return;
    }
    let mut mark = None;
    {
        let d = p.doc.dm(doc);
        d.hist.flags &= !HDF_CAPTURE;
        let Doc { layers, hist, .. } = d;
        let c = hist.capt.as_ref().unwrap();
        if let Some(li) = hs_layer_in(layers, c.e.layer, c.e.uid) {
            let l = &mut layers[li];
            if l.sf.w == c.e.lw && l.sf.h == c.e.lh {
                let stride = l.sf.stride;
                let mut b = [0x7FFF_FFFF, 0x7FFF_FFFF, 0, 0];
                for t in &c.e.tiles {
                    let (off, vw, vh) = ht_geom(&c.e, t, stride);
                    tile_put(&t.raw()[..], &mut l.sf.px[off..], stride, vw, vh);
                    let (tx, ty) = ((t.tx as i32) << TILE_SH, (t.ty as i32) << TILE_SH);
                    if tx < b[0] {
                        b[0] = tx;
                    }
                    if tx + vw > b[2] {
                        b[2] = tx + vw;
                    }
                    if ty < b[1] {
                        b[1] = ty;
                    }
                    if ty + vh > b[3] {
                        b[3] = ty + vh;
                    }
                }
                if b[2] > 0 {
                    mark = Some((li, b));
                }
            }
        }
    }
    if let Some((li, b)) = mark {
        doc_mark_px(p, doc, li as i32, b[0], b[1], b[2], b[3]);
    }
    hs_capt_release(p, doc);
}

// hs_layer on a layer list
fn hs_layer_in(ls: &[Layer], index: i32, uid: u64) -> Option<usize> {
    if (index as u32 as usize) < ls.len() && ls[index as usize].uid == uid {
        return Some(index as usize);
    }
    ls.iter().position(|l| l.uid == uid)
}

/// hist_px_active(doc) -> true while a capture is open
pub(crate) fn hist_px_active(d: &Doc) -> bool {
    d.hist.flags & HDF_CAPTURE != 0
}

// ================================================================== other records

/// hist_push_layer_props(doc, i, name, icon) -> false = out of memory: do not change
pub(crate) fn hist_push_layer_props(p: &mut Pn, doc: DocId, i: i32, name: &'static str, icon: u32) -> bool {
    if p.hist.apply {
        return true;
    }
    let n = p.doc.d(doc).layers.len() as i32;
    if n <= 0 {
        return true;
    }
    let i = i.min(n - 1).max(0);
    let Pn { hist, hm, doc: docs, .. } = p;
    let Some(mut e) = ent_new(hist, hm, HK_PROPS, name, icon) else {
        hs_oom_msg(p);
        return false;
    };
    let l = &docs.d(doc).layers[i as usize];
    e.layer = i;
    e.uid = l.uid;
    e.pname = l.name.clone();
    e.popac = l.sf.opac;
    e.pblend = l.sf.blend;
    e.pvis = l.flags & LYF_VISIBLE as u32;
    if hist_add(p, doc, e) {
        return true;
    }
    hs_oom_msg(p);
    false
}

/// hist_push_layers(doc, name, icon) -> false = out of memory -- the LAYER array (by value:
/// the records and the identity of their pixel buffers; caches never), DOC_CUR, DOC_W/H and
/// the resolution
pub(crate) fn hist_push_layers(p: &mut Pn, doc: DocId, name: &'static str, icon: u32) -> bool {
    if p.hist.apply {
        return true;
    }
    let Pn { hist, hm, doc: docs, .. } = p;
    let Some(mut e) = ent_new(hist, hm, HK_LAYERS, name, icon) else {
        hs_oom_msg(p);
        return false;
    };
    let d = docs.dm(doc);
    let n = d.layers.len() as i32;
    let cap = doc_lcap(d).max(n).max(1);
    if hm.fail() {
        ent_destroy(hist, hm, &mut d.hist, e);
        hs_oom_msg(p);
        return false;
    }
    e.lcap = cap;
    hm.acc(&mut e, cap as i64 * LY_SIZE);
    e.lays = d.layers.iter().map(HLayer::of).collect();
    e.lcur = d.cur;
    e.dw = d.w;
    e.dh = d.h;
    e.dunit = d.dpu_unit;
    e.dpux = d.dpu_x;
    e.dpuy = d.dpu_y;
    if hist_add(p, doc, e) {
        return true;
    }
    hs_oom_msg(p);
    false
}

/// hist_push_doc(doc, name, icon) -> false = out of memory -- the same record as
/// hist_push_layers (the layer array, DOC_CUR, DOC_W/H, resolution)
pub(crate) fn hist_push_doc(p: &mut Pn, doc: DocId, name: &'static str, icon: u32) -> bool {
    hist_push_layers(p, doc, name, icon)
}

/// hist_push_sel(doc, name, icon) -> false = out of memory -- the selection (sel.rs hooks)
pub(crate) fn hist_push_sel(p: &mut Pn, doc: DocId, name: &'static str, icon: u32) -> bool {
    if p.hist.apply {
        return true;
    }
    let blob = if p.hist.sel_on { sel_hsnap(p.doc.d(doc)) } else { None };
    let Pn { hist, hm, .. } = p;
    let Some(mut e) = ent_new(hist, hm, HK_SEL, name, icon) else {
        hs_oom_msg(p);
        return false;
    };
    let sz = blob.as_deref().map_or(0, sel_hsize);
    e.blob = blob;
    hm.acc(&mut e, sz as i64);
    if hist_add(p, doc, e) {
        return true;
    }
    hs_oom_msg(p);
    false
}

/// hist_begin_compound(doc) -- everything recorded until the matching hist_end_compound
/// becomes one step (nestable)
pub(crate) fn hist_begin_compound(p: &mut Pn, doc: DocId) {
    if p.hist.apply {
        return;
    }
    let Pn { hist, hm, doc: docs, .. } = p;
    let dh = &mut docs.dm(doc).hist;
    if dh.cdep == 0 {
        dh.comp = ent_new(hist, hm, HK_COMP, "", 0); // None (no memory): single steps then
    }
    dh.cdep += 1;
}

/// hist_end_compound(doc, name, icon) -> step recorded
pub(crate) fn hist_end_compound(p: &mut Pn, doc: DocId, name: &'static str, icon: u32) -> bool {
    {
        if p.hist.apply {
            return false;
        }
        let dh = &mut p.doc.dm(doc).hist;
        if dh.cdep == 0 {
            return false;
        }
        dh.cdep -= 1;
        if dh.cdep != 0 {
            return false;
        }
    }
    if p.doc.d(doc).hist.flags & HDF_CAPTURE != 0 {
        hist_px_end(p, doc, name, icon); // an open capture belongs to the compound
    }
    let Pn { hist, hm, doc: docs, .. } = p;
    let dh = &mut docs.dm(doc).hist;
    let Some(mut c) = dh.comp.take() else { return false };
    if c.kids.is_empty() {
        ent_destroy(hist, hm, dh, c);
        return false;
    }
    let mut e = if c.kids.len() == 1 {
        let k = c.kids.pop().unwrap(); // one record: it is the step itself
        ent_free(hist, hm, &mut c);
        k
    } else {
        c
    };
    e.name = name;
    e.icon = icon;
    if hist_add(p, doc, e) {
        return true;
    }
    hs_oom_clear(p, doc); // the document changed without a step
    false
}

// ================================================================== applying entries

/// hs_apply(doc, e, dir) -> 0 | ERR_* -- exchange the entry's state with the document
/// (dir < 0 undo, > 0 redo: the order of compound children)
fn hs_apply(p: &mut Pn, doc: DocId, e: &mut HistEntry, dir: i32) -> i32 {
    match e.kind {
        HK_PX => hs_apply_px(p, doc, e),
        HK_PROPS => hs_apply_props(p, doc, e),
        HK_LAYERS => hs_apply_layers(p, doc, e),
        HK_SEL => hs_apply_sel(p, doc, e),
        HK_COMP => {
            // children forward for redo, backward for undo; a failure swaps back what was done
            let n = e.kids.len();
            let idx = |k: usize| if dir > 0 { k } else { n - k - 1 };
            let mut done = 0;
            while done < n {
                let r = hs_apply(p, doc, &mut e.kids[idx(done)], dir);
                if r != 0 {
                    while done > 0 {
                        done -= 1;
                        hs_apply(p, doc, &mut e.kids[idx(done)], -dir);
                    }
                    return r;
                }
                done += 1;
            }
            0
        }
        _ => 0,
    }
}

/// hs_apply_px(doc, e) -> 0 | ERR_* -- all tiles into RAM first (nothing changes when that
/// fails), then swap them with the layer
fn hs_apply_px(p: &mut Pn, doc: DocId, e: &mut HistEntry) -> i32 {
    let Some(li) = hs_layer(p.doc.d(doc), e.layer, e.uid) else { return ERR_FAIL };
    e.layer = li as i32;
    {
        let l = &p.doc.d(doc).layers[li];
        if l.sf.w != e.lw || l.sf.h != e.lh {
            return ERR_FAIL;
        }
    }
    let r = p.hm.load_entry(e);
    if r != 0 {
        return r;
    }
    let Pn { hm, doc: docs, .. } = p;
    let l = &mut docs.dm(doc).layers[li];
    let stride = l.sf.stride;
    for i in 0..e.tiles.len() {
        let (off, vw, vh) = ht_geom(e, &e.tiles[i], stride);
        tile_swap(&mut e.tiles[i].raw_mut()[..], &mut l.sf.px[off..], stride, vw, vh);
        if e.tiles[i].fl & HTF_NOZ != 0 {
            e.tiles[i].fl &= !HTF_NOZ; // new content: try deflate again
            e.nraw += 1;
        }
        hm.ht_solidify(e, i, vw, vh);
    }
    bt_px(p, doc, li as i32, e.x0, e.y0, e.x1, e.y1);
    0
}

/// hs_apply_props(doc, e) -> 0 | ERR_FAIL -- exchange name, opacity, blend mode, visibility
fn hs_apply_props(p: &mut Pn, doc: DocId, e: &mut HistEntry) -> i32 {
    let d = p.doc.dm(doc);
    let Some(li) = hs_layer(d, e.layer, e.uid) else { return ERR_FAIL };
    e.layer = li as i32;
    let l = &mut d.layers[li];
    core::mem::swap(&mut l.name, &mut e.pname);
    core::mem::swap(&mut l.sf.opac, &mut e.popac);
    core::mem::swap(&mut l.sf.blend, &mut e.pblend);
    let v = l.flags & LYF_VISIBLE as u32;
    l.flags = (l.flags & !(LYF_VISIBLE as u32)) | e.pvis;
    e.pvis = v;
    p.hist.bt_struct = true;
    0
}

/// hs_apply_layers(doc, e) -> 0 -- exchange the LAYER arrays and the document fields.
/// Incoming records take the caches (thumbnail, dirty bit set) of the same layer (uid) when
/// the size stays; caches of layers that leave the document are freed.  The pixel buffers of
/// the outgoing layers go to the pool, the incoming records take theirs from it.
fn hs_apply_layers(p: &mut Pn, doc: DocId, e: &mut HistEntry) -> i32 {
    bt_flush_px(p, doc); // pending pixel marks use the old indices
    let Pn { hist, hm, doc: docs, .. } = p;
    let d = docs.dm(doc);
    let same = e.dw == d.w && e.dh == d.h;
    let lcap = doc_lcap(d);
    let mut out = core::mem::take(&mut d.layers);
    // the incoming records with the caches of the same layer
    let mut inc: Vec<Layer> = Vec::with_capacity(e.lays.len());
    for hl in &e.lays {
        let mut l = Layer {
            sf: Surface { px: Vec::new(), w: hl.w, h: hl.h, stride: hl.stride, fmt: hl.fmt, flags: hl.sflags, opac: hl.opac, blend: hl.blend },
            name: String::new(),
            flags: hl.flags,
            uid: hl.uid,
            ver: 1,
            ..Layer::default()
        };
        if same {
            if let Some(o) = out.iter_mut().find(|o| o.uid == hl.uid) {
                l.thumb = o.thumb.take();
                l.tdirty = core::mem::take(&mut o.tdirty);
                l.thumb_ver = o.thumb_ver;
                o.thumb_ver = 0;
                l.ver = o.ver.wrapping_add(1); // the thumbnail is rebuilt
            }
        }
        inc.push(l);
    }
    // the outgoing records: their buffers to the pool, their fields to the entry
    let mut outh: Vec<HLayer> = Vec::with_capacity(out.len());
    for mut o in out.drain(..) {
        let mut h = HLayer::of(&o);
        h.name = core::mem::take(&mut o.name);
        h.meta = core::mem::take(&mut o.meta);
        if h.tok != 0 {
            d.hist.pool.push(core::mem::take(&mut o.sf));
        }
        outh.push(h); // (caches freed with o)
    }
    // the incoming buffers
    for (l, hl) in inc.iter_mut().zip(e.lays.iter_mut()) {
        l.name = core::mem::take(&mut hl.name);
        l.meta = core::mem::take(&mut hl.meta);
        if hl.tok == 0 {
            continue;
        }
        match d.hist.pool.iter().position(|s| sf_tok(s) == hl.tok) {
            Some(k) => {
                let s = d.hist.pool.swap_remove(k);
                l.sf.px = s.px;
            }
            None => {
                // (a buffer the document dropped without hist_release: transparent instead)
                l.sf.px = vec![0u8; (hl.stride.max(0) as usize * hl.h.max(0) as usize).max(64)];
            }
        }
    }
    d.layers = inc;
    e.lays = outh;
    let el = e.lcap;
    e.lcap = lcap;
    d.hist.lcap = el;
    hm.acc(e, (lcap - el) as i64 * LY_SIZE); // the entry's array grew by this many records
    core::mem::swap(&mut d.cur, &mut e.lcur);
    core::mem::swap(&mut d.w, &mut e.dw);
    core::mem::swap(&mut d.h, &mut e.dh);
    core::mem::swap(&mut d.dpu_unit, &mut e.dunit);
    core::mem::swap(&mut d.dpu_x, &mut e.dpux);
    core::mem::swap(&mut d.dpu_y, &mut e.dpuy);
    hist.bt_struct = true;
    hist.bt_resized |= !same;
    0
}

/// hs_apply_sel(doc, e) -> 0 -- exchange the selection with the entry's blob
fn hs_apply_sel(p: &mut Pn, doc: DocId, e: &mut HistEntry) -> i32 {
    if !p.hist.sel_on {
        return 0;
    }
    let cur = sel_hsnap(p.doc.d(doc));
    sel_hrestore(p, doc, e.blob.as_deref());
    let old = e.blob.as_deref().map_or(0, sel_hsize);
    p.hm.acc(e, -(old as i64));
    let sz = cur.as_deref().map_or(0, sel_hsize);
    e.blob = cur;
    p.hm.acc(e, sz as i64);
    0
}

// ------------------------------------------------------------------ batched invalidation

/// bt_px(doc, layer, x0, y0, x1, y1) -- remember a changed rectangle of a layer
fn bt_px(p: &mut Pn, doc: DocId, layer: i32, x0: i32, y0: i32, x1: i32, y1: i32) {
    if let Some(b) = p.hist.bt.iter_mut().find(|b| b[0] == layer) {
        b[1] = b[1].min(x0);
        b[2] = b[2].min(y0);
        b[3] = b[3].max(x1);
        b[4] = b[4].max(y1);
        return;
    }
    if p.hist.bt.len() >= BATCH_MAX {
        bt_flush_px(p, doc);
    }
    p.hist.bt.push([layer, x0, y0, x1, y1]);
}

/// bt_flush_px(doc) -- doc_mark_px for every remembered rectangle
fn bt_flush_px(p: &mut Pn, doc: DocId) {
    let mut bt = core::mem::take(&mut p.hist.bt);
    for b in &bt {
        doc_mark_px(p, doc, b[0], b[1], b[2], b[3], b[4]);
    }
    bt.clear();
    p.hist.bt = bt;
}

/// bt_end(doc) -- flush the pixel marks; one doc_mark_layers after structure changes, or
/// doc_resized when the image size changed
fn bt_end(p: &mut Pn, doc: DocId) {
    bt_flush_px(p, doc);
    if p.hist.bt_resized {
        p.hist.bt_resized = false;
        p.hist.bt_struct = false;
        if doc_resized(p, doc) != 0 {
            hs_oom_msg(p);
        }
    } else if p.hist.bt_struct {
        p.hist.bt_struct = false;
        doc_mark_layers(p, doc);
    }
}

// ================================================================== navigation

/// hs_nav_ok(doc) -> true when undo / redo / jump / clear may run
fn hs_nav_ok(dh: &DocHist) -> bool {
    !dh.ent.is_empty() && dh.flags & HDF_CAPTURE == 0 && dh.cdep == 0
}

/// hist_can_undo(doc)
pub(crate) fn hist_can_undo(d: &Doc) -> bool {
    hs_nav_ok(&d.hist) && d.hist.pos > 1
}

/// hist_can_redo(doc)
pub(crate) fn hist_can_redo(d: &Doc) -> bool {
    hs_nav_ok(&d.hist) && d.hist.pos < d.hist.ent.len() as i32
}

/// hist_undo(doc) -> changed (HIST-002)
pub(crate) fn hist_undo(p: &mut Pn, doc: DocId) -> bool {
    let d = p.doc.d(doc);
    if !hist_can_undo(d) {
        return false;
    }
    let t = d.hist.pos - 2;
    hs_goto(p, doc, t)
}

/// hist_redo(doc) -> changed (HIST-003)
pub(crate) fn hist_redo(p: &mut Pn, doc: DocId) -> bool {
    let d = p.doc.d(doc);
    if !hist_can_redo(d) {
        return false;
    }
    let t = d.hist.pos;
    hs_goto(p, doc, t)
}

/// hist_jump(doc, index) -> changed -- entry `index` becomes the current state (0 = the first
/// entry = everything undone; clamped)
pub(crate) fn hist_jump(p: &mut Pn, doc: DocId, index: i32) -> bool {
    let dh = &p.doc.d(doc).hist;
    if !hs_nav_ok(dh) {
        return false;
    }
    let n = dh.ent.len() as i32;
    let mut i = index.max(0);
    if i >= n {
        i = n - 1;
    }
    hs_goto(p, doc, i)
}

/// hs_goto(doc, index) -> changed -- step undo / redo until pos = index + 1, then one batch of
/// invalidations, the modified flag, the budget
fn hs_goto(p: &mut Pn, doc: DocId, index: i32) -> bool {
    let target = index + 1;
    if target == p.doc.d(doc).hist.pos {
        return false;
    }
    p.hist.apply = true;
    p.hist.bt.clear();
    p.hist.bt_struct = false;
    p.hist.bt_resized = false;
    let mut err = 0;
    while p.doc.d(doc).hist.pos > target {
        let k = (p.doc.d(doc).hist.pos - 1) as usize;
        let mut e = core::mem::take(&mut p.doc.dm(doc).hist.ent[k]);
        let r = hs_apply(p, doc, &mut e, -1);
        p.doc.dm(doc).hist.ent[k] = e;
        if r != 0 {
            err = r;
            break;
        }
        p.doc.dm(doc).hist.pos -= 1;
    }
    while err == 0 && p.doc.d(doc).hist.pos < target {
        let k = p.doc.d(doc).hist.pos as usize;
        let mut e = core::mem::take(&mut p.doc.dm(doc).hist.ent[k]);
        let r = hs_apply(p, doc, &mut e, 1);
        p.doc.dm(doc).hist.ent[k] = e;
        if r != 0 {
            err = r;
            break;
        }
        p.doc.dm(doc).hist.pos += 1;
    }
    p.hist.apply = false;
    bt_end(p, doc);
    hs_modified(p, doc);
    let d = p.doc.dm(doc);
    d.ver_hist = d.ver_hist.wrapping_add(1);
    if err != 0 {
        hs_msg(p, IB_ERROR, S_APPLY_FAIL);
    }
    hist_enforce(p);
    true
}

/// hs_modified(doc) -- DF_MODIFIED unless the current state is the saved one
fn hs_modified(p: &mut Pn, doc: DocId) {
    let dh = &p.doc.d(doc).hist;
    let m = dh.pos != dh.clean;
    doc_set_modified(p, doc, m);
}

// ================================================================== queries

/// hist_count(doc) -> entries (0 = history not started)
pub(crate) fn hist_count(d: &Doc) -> i32 {
    d.hist.ent.len() as i32
}

/// hist_pos(doc) -> entries 0..pos-1 are applied (current entry = pos - 1)
pub(crate) fn hist_pos(d: &Doc) -> i32 {
    d.hist.pos
}

/// hist_clean(doc) -> HD_CLEAN (pos of the saved state, -1 = unreachable)
pub(crate) fn hist_clean(d: &Doc) -> i32 {
    d.hist.clean
}

/// hist_entry(doc, i) -> entry record | None
pub(crate) fn hist_entry(d: &Doc, i: i32) -> Option<&HistEntry> {
    d.hist.ent.get(i as u32 as usize).map(|e| &**e)
}

/// hist_entry_name(doc, i) -> name ("Bearbeiten" when none)
pub(crate) fn hist_entry_name(d: &Doc, i: i32) -> &str {
    match hist_entry(d, i) {
        Some(e) if !e.name.is_empty() => e.name,
        _ => S_EDIT,
    }
}

/// hist_entry_icon(doc, i) -> IC_* (IC_EDIT when none)
pub(crate) fn hist_entry_icon(d: &Doc, i: i32) -> u32 {
    match hist_entry(d, i) {
        Some(e) if e.icon != 0 => e.icon,
        _ => IC_EDIT,
    }
}

// ================================================================== clear / free / saved state

/// hs_clear(doc) -- keep the current state as the only entry (the first entry's name stays)
fn hs_clear(p: &mut Pn, doc: DocId) {
    let Pn { hist, hm, doc: docs, .. } = p;
    let d = docs.dm(doc);
    let dh = &mut d.hist;
    if dh.ent.is_empty() {
        return;
    }
    while dh.ent.len() > 1 {
        let e = dh.ent.pop().unwrap();
        ent_destroy(hist, hm, dh, e);
    }
    dh.clean = if dh.pos == dh.clean { 1 } else { -1 };
    dh.pos = 1;
    d.ver_hist = d.ver_hist.wrapping_add(1);
}

/// hist_clear(doc) -- Verlauf löschen (HIST-017); not while a capture / compound is open
pub(crate) fn hist_clear(p: &mut Pn, doc: DocId) {
    if hs_nav_ok(&p.doc.d(doc).hist) {
        hs_clear(p, doc);
    }
}

/// hist_free(doc) -- free the whole history (doc_close, before the layers are freed)
pub(crate) fn hist_free(p: &mut Pn, doc: DocId) {
    hs_drop_all(p, doc);
    p.doc.dm(doc).hist = DocHist::default();
}

/// hist_mark_clean(doc) -- the document was saved in its current state
pub(crate) fn hist_mark_clean(p: &mut Pn, doc: DocId) {
    hs_base(p, doc);
    let d = p.doc.dm(doc);
    d.hist.clean = d.hist.pos;
    d.ver_hist = d.ver_hist.wrapping_add(1);
}

// ================================================================== messages, hooks

/// hs_msg(severity, str) -- remember a message and pass it to the notify hook
fn hs_msg(p: &mut Pn, sev: u32, s: &'static str) {
    p.hist.msg = Some(s);
    p.hist.msg_sev = sev;
    if let Some(f) = p.hist.notify {
        f(p, sev, s);
    }
    p.invalidate();
}

/// hs_oom_msg() -- "not enough memory" (the change was not made)
fn hs_oom_msg(p: &mut Pn) {
    hs_msg(p, IB_ERROR, S_OOM);
}

/// hs_oom_clear(doc) -- the document changed but no step could be recorded: drop the
/// history (the current state stays) and say so (HIST-014)
fn hs_oom_clear(p: &mut Pn, doc: DocId) {
    {
        let Pn { hist, hm, doc: docs, .. } = p;
        let dh = &mut docs.dm(doc).hist;
        if let Some(c) = dh.comp.take() {
            ent_destroy(hist, hm, dh, c);
        }
    }
    hs_clear(p, doc);
    hs_msg(p, IB_ERROR, S_OOM_CLEAR);
}

/// hist_sel_hooks(snapshot, restore, free, size) -- selection history (sel_init); here the
/// hooks are sel_hsnap / sel_hrestore / sel_hfree / sel_hsize, `on` enables them
pub(crate) fn hist_sel_hooks(h: &mut Hist, on: bool) {
    h.sel_on = on;
}

/// hist_notify_set(fn) -- fn(severity, msg) for history messages
pub(crate) fn hist_notify_set(h: &mut Hist, f: Option<fn(&mut Pn, u32, &str)>) {
    h.notify = f;
}

/// hist_message() -> (message, severity IB_*) | None
pub(crate) fn hist_message(h: &Hist) -> Option<(&str, u32)> {
    h.msg.map(|m| (m, h.msg_sev))
}

/// hist_message_clear()
pub(crate) fn hist_message_clear(h: &mut Hist) {
    h.msg = None;
}

// ================================================================== memory budget

/// hist_enforce() -- keep all histories under the budget: deflate the oldest pixel entries,
/// then spill them to the temp file, then drop the oldest entries (HIST-012..014)
pub(crate) fn hist_enforce(p: &mut Pn) {
    if p.hist.apply {
        return;
    }
    hs_recount(p);
    if p.hm.ram <= p.hm.budget {
        return;
    }
    p.hm.tiles_trim();
    loop {
        let Some(seq) = hs_oldest(p, PRED_Z) else { break };
        let Pn { hm, doc: docs, .. } = p;
        let Some(e) = find_seq(docs, seq) else { break };
        let before = e.nraw;
        hm.compress_entry(e);
        if e.nraw == before {
            break; // no progress (out of memory)
        }
        if hm.ram <= hm.budget {
            return;
        }
    }
    loop {
        let Some(seq) = hs_oldest(p, PRED_SPILL) else { break };
        let Pn { hm, doc: docs, .. } = p;
        let Some(e) = find_seq(docs, seq) else { break };
        if hm.spill_entry(e) != 0 {
            break;
        }
        if hm.ram <= hm.budget {
            return;
        }
    }
    let mut dropped = 0;
    while p.hm.ram > p.hm.budget {
        if !hs_drop_oldest(p) {
            break;
        }
        dropped += 1;
    }
    if dropped != 0 {
        hs_msg(p, IB_WARNING, S_DROPPED);
    }
}

/// hist_reclaim() -- an allocation failed: squeeze the histories to half their RAM
pub(crate) fn hist_reclaim(p: &mut Pn) {
    let b = p.hm.budget;
    p.hm.budget = p.hm.ram >> 1;
    hist_enforce(p);
    p.hm.budget = b;
}

// ho_visit: the oldest pixel entry of e and its children that matches pred
fn ho_visit(e: &HistEntry, pred: u32, best: &mut u64) {
    match e.kind {
        HK_COMP => {
            for k in &e.kids {
                ho_visit(k, pred, best);
            }
        }
        HK_PX => {
            let v = if pred == PRED_Z { e.nraw } else { e.nmem };
            if v != 0 && e.seq < *best {
                *best = e.seq;
            }
        }
        _ => {}
    }
}

/// hs_oldest(pred) -> the sequence number of the oldest pixel entry of all documents with raw
/// tiles (PRED_Z) or with tiles in RAM (PRED_SPILL)
fn hs_oldest(p: &Pn, pred: u32) -> Option<u64> {
    let mut best = u64::MAX;
    for d in p.doc.docs.iter() {
        for e in &d.hist.ent {
            ho_visit(e, pred, &mut best);
        }
        if let Some(c) = &d.hist.comp {
            ho_visit(c, pred, &mut best);
        }
    }
    if best == u64::MAX { None } else { Some(best) }
}

// the entry with sequence number seq (e or one of its children)
fn find_in(e: &mut HistEntry, seq: u64) -> Option<&mut HistEntry> {
    if e.seq == seq {
        return Some(e);
    }
    if e.kind == HK_COMP {
        for k in e.kids.iter_mut() {
            if let Some(f) = find_in(k, seq) {
                return Some(f);
            }
        }
    }
    None
}

// the entry with sequence number seq in any document
fn find_seq(docs: &mut crate::document::model::Docs, seq: u64) -> Option<&mut HistEntry> {
    for d in docs.docs.iter_mut() {
        let dh = &mut d.hist;
        for e in dh.ent.iter_mut() {
            if let Some(f) = find_in(e, seq) {
                return Some(f);
            }
        }
        if let Some(c) = dh.comp.as_mut() {
            if let Some(f) = find_in(c, seq) {
                return Some(f);
            }
        }
    }
    None
}

// rc_visit: HE_XB of a structure entry = bytes of the surfaces it keeps that are not in its
// document any more
fn rc_visit(hm: &mut HistMem, doc_layers: &[Layer], e: &mut HistEntry) {
    match e.kind {
        HK_COMP => {
            for k in e.kids.iter_mut() {
                rc_visit(hm, doc_layers, k);
            }
        }
        HK_LAYERS => {
            let mut b = 0u64;
            for l in &e.lays {
                if l.tok == 0 || doc_layers.iter().any(|o| sf_tok(&o.sf) == l.tok) {
                    continue;
                }
                b = b.wrapping_add((l.stride as i64 * l.h as i64) as u64);
            }
            let dlt = b.wrapping_sub(e.xb);
            e.xb = b;
            hm.acc(e, dlt as i64);
        }
        _ => {}
    }
}

/// hs_recount() -- HE_XB of every structure entry (they count against the budget)
fn hs_recount(p: &mut Pn) {
    let Pn { hm, doc: docs, .. } = p;
    for d in docs.docs.iter_mut() {
        let Doc { layers, hist: dh, .. } = &mut **d;
        for e in dh.ent.iter_mut() {
            rc_visit(hm, layers, e);
        }
        if let Some(c) = dh.comp.as_mut() {
            rc_visit(hm, layers, c);
        }
    }
}

/// hs_drop_oldest() -> true when an entry was dropped: the oldest applied step of any
/// document, else the newest redo step; never the step that was added last
fn hs_drop_oldest(p: &mut Pn) -> bool {
    let last = p.hist.last;
    let mut best: Option<(DocId, u64)> = None;
    for d in p.doc.docs.iter() {
        let dh = &d.hist;
        if dh.pos < 2 {
            continue;
        }
        let e = &dh.ent[1];
        if e.seq == last {
            continue;
        }
        if best.is_none_or(|b| e.seq < b.1) {
            best = Some((d.id, e.seq));
        }
    }
    if let Some((id, _)) = best {
        hs_drop_first(p, id);
        return true;
    }
    let mut redo = None;
    for d in p.doc.docs.iter() {
        let dh = &d.hist;
        let n = dh.ent.len() as i32;
        if n <= dh.pos || dh.ent[n as usize - 1].seq == last {
            continue;
        }
        redo = Some(d.id);
        break;
    }
    if let Some(id) = redo {
        hs_drop_last(p, id);
        return true;
    }
    false
}
