// layer -- PaintCraft: layer model and the Ebenen menu.  A1 (M1): records, add, delete,
// duplicate, merge down, move, go to, visibility.  A4 (M3, second half of this file): Aus
// Datei importieren (images and .pdn, "larger than the canvas" question), Horizontal /
// Vertikal spiegeln, Um 180° rotieren, Rotationszoom (dlg_rotzoom.rs), Ebeneneigenschaften
// (dlg_layerprops.rs), Zusammenfassen (Bild menu; imgops.rs forwards CMD_FLATTEN), test hooks.
//                                                                  [apps/PaintCraft/src/layer.s]
//
// Module contract (app/lifecycle.rs):
//   lay_init()  lay_states()  lay_frame()  lay_cmd(id, arg) -> handled
// A4 (other modules):
//   lay_rec(doc, i) -> &Layer (None when i is out of range; no clamping)
//   lay_find_uid(doc, uid) -> index | None
//   lay_flip(doc, i, kind) -> 1 step         kind 0 horizontal, 1 vertical, 2 180 degrees
//   lay_flatten(doc) -> 0 / -1               every visible layer into one (history step)
//   lay_flatten_cmd()  lay_flat_states()     CMD_FLATTEN for imgops.rs (iop_cmd / iop_states)
//   lay_import_paths(doc, paths) -> n        import files as layers above the current one
//   lay_lines(style, str)                    ui.text per '\n'-separated line (dialogs)
// Model (every change calls doc_mark_layers; history is recorded by lay_cmd, not here):
//   lay_new(doc, index, name) -> index       transparent layer at index (name None = "Ebene n");
//                                            layers at >= index move up, DOC_CUR follows its
//                                            layer
//   lay_add_sf(doc, index, sf, name) -> index    same, taking over the FMT_ARGB surface sf
//                                            (Err(sf) gives it back on failure)
//   lay_delete(doc, i) -> 0 / -1             (never the last layer)
//   lay_move(doc, from, to)                  DOC_CUR follows the moved layer
//   lay_dup(doc, i) -> index                 copy above i ("<Name> Kopie")
//   lay_merge_down(doc, i) -> 0 / -1         blend layer i into i - 1 with its mode and
//                                            opacity, delete it, DOC_CUR = i - 1
//   lay_set_name(doc, i, str) -> 0 / -1      lay_set_opacity(doc, i, 0..255)
//   lay_set_blend(doc, i, BM_*)              lay_set_visible(doc, i, on)
//   lay_free_rec(layer)                      (drop)
//
// A LAYER* of the asm is an index into `Doc::layers` here (valid until the next layer list
// change, like the pointer).  A Layer owns its surface: history entries refer to the
// document's layers by uid and take a resource over through hist_release when the document
// stops using it (pn_hist.inc RESOURCE OWNERSHIP).  The `*_d` functions work on a document
// record without Pn (doc_new builds documents with them before they are in the list).

use alloc::string::String;
use alloc::vec::Vec;

use gfx::blendmode::{bm_row_inplace, BM_XOR};
use gfx::{Surface, FMT_ARGB};
use img_core::{ImgDec, ERR_DIMS, ERR_IO, ERR_LIMIT, ERR_NOMEM, ERR_UNSUPPORTED, IFC_DECODE, IMGD_STRAIGHT, IMGF_COUNT, IMGF_PDN};
use rt::sync::SpinLock;
use ui::*;

use crate::app::constants::*;
use crate::document::model::{doc_active, doc_index, doc_mark_layers_d, doc_mark_px, doc_resized, Doc, DocId};
use crate::document::history::{hist_count, hist_entry_name, hist_pos, hist_push_doc, hist_push_layer_props, hist_push_layers, hist_px_begin, hist_px_cancel, hist_px_end, hist_px_touch, hist_release};
use crate::app::formatting::fmt_cap;
use crate::app::commands::pn_cmd_set;
use crate::app::Pn;

const LINE_CAP: usize = 256;
const A4_PROBE_CAP: usize = 640;
const IMPX_CAP: usize = 8192; // file type list of the import dialog
const FIXT_N: usize = 8; // test fixtures (A4_FIXTURES)

/// LAYER (pn_doc.inc).  Must stay Send + Sync (part of Doc).
#[derive(Default)]
pub(crate) struct Layer {
    /// LY_SF: FMT_ARGB straight pixels; opac = layer opacity 0..255, blend = BM_*
    pub(crate) sf: Surface,
    /// LY_NAME
    pub(crate) name: String,
    /// LY_FLAGS: LYF_*
    pub(crate) flags: u32,
    /// LY_UID: unique id within the document (the history refers to it)
    pub(crate) uid: u64,
    /// LY_THUMB: thumbnail (FMT_PARGB), a cache (lay_thumb)
    pub(crate) thumb: Option<alloc::boxed::Box<Surface>>,
    /// LY_THUMB_VER: version the thumbnail was made from
    pub(crate) thumb_ver: u32,
    /// the thumbnail's SF_AUX: bumped on every render (the renderer's image version)
    pub(crate) thumb_aux: u32,
    /// LY_VER: pixel version of this layer
    pub(crate) ver: u32,
    /// LY_TDIRTY: tile dirty bitset (64 x 64 tiles), a cache (empty = none)
    pub(crate) tdirty: Vec<u64>,
    /// LY_META: .pdn per-layer metadata blob (empty = none)
    pub(crate) meta: Vec<u8>,
}

/// an import record (IL_*): one decoded image / .pdn layer waiting to become a layer
#[derive(Default)]
struct ImpRec {
    /// IL_SF: FMT_ARGB
    sf: Surface,
    /// IL_NAME (None = no memory in the asm: the layer gets "Ebene n")
    name: Option<String>,
    opac: u8,
    blend: u8,
    vis: u8,
    meta: Vec<u8>,
}

/// the state of layer.s (import, flatten question, probes)
pub(crate) struct Lay {
    /// g_imp_doc: image the import goes to
    imp_doc: Option<DocId>,
    /// g_imp_list: IL records
    imp_list: Vec<ImpRec>,
    /// g_imp_w / g_imp_h: largest imported image
    imp_w: i32,
    imp_h: i32,
    imp_fd_open: Shared<i32>,
    imp_ask_open: Shared<i32>,
    imp_ask_pending: bool,
    imp_ask_rect: Shared<WinRect>,
    flat_doc: Option<DocId>,
    flat_open: Shared<i32>,
    flat_pending: bool,
    flat_rect: Shared<WinRect>,
    imp_path: String,
    imp_dir: String,
    /// g_imp_exts: the entries of the import dialog's type list
    imp_exts: Vec<String>,
    /// g_a4_probe: text of the last probe (cmd 370 / 372)
    a4_probe: String,
}

impl Default for Lay {
    fn default() -> Lay {
        Lay {
            imp_doc: None,
            imp_list: Vec::new(),
            imp_w: 0,
            imp_h: 0,
            imp_fd_open: Shared::new(0),
            imp_ask_open: Shared::new(0),
            imp_ask_pending: false,
            imp_ask_rect: Shared::new(WinRect::new(0, 0, 480, 0, 0)),
            flat_doc: None,
            flat_open: Shared::new(0),
            flat_pending: false,
            flat_rect: Shared::new(WinRect::new(0, 0, 400, 0, 0)),
            imp_path: String::new(),
            imp_dir: String::new(),
            imp_exts: Vec::new(),
            a4_probe: String::new(),
        }
    }
}

// ------------------------------------------------------------------ records

// lay_insert_rec(doc, index) -> the clamped index of a new zeroed record (uid, visible)
fn lay_insert_rec(d: &mut Doc, index: i32) -> usize {
    let n = d.layers.len() as i32;
    let i = index.max(0).min(n);
    let uid = d.uid_next;
    d.uid_next = d.uid_next.wrapping_add(1);
    let l = Layer { uid, flags: LYF_VISIBLE as u32, ..Layer::default() };
    d.layers.insert(i as usize, l);
    if n != 0 && i <= d.cur {
        d.cur += 1;
    }
    i as usize
}

// fix_cur_after_remove(doc, i) -- DOC_CUR after record i was removed: the layers above move
// down; a removed current layer selects the one below (or 0)
fn fix_cur_after_remove(d: &mut Doc, i: i32) {
    let mut c = d.cur;
    if c >= i && c != 0 {
        c -= 1;
    }
    let top = d.layers.len() as i32 - 1;
    if c > top {
        c = top;
    }
    if c < 0 {
        c = 0;
    }
    d.cur = c;
}

/// lay_free_rec(layer) -- pixels, name, thumbnail, dirty bitset, metadata
pub(crate) fn lay_free_rec(rec: Layer) {
    drop(rec);
}

// default_name(doc, name) -> name, or "Ebene n" (n = layer count)
fn default_name(d: &Doc, name: Option<&str>) -> String {
    match name {
        Some(s) => String::from(s),
        None => fmt_cap(alloc::format!("Ebene {}", d.layers.len() as i32), LNAME_CAP as usize),
    }
}

/// lay_new(doc, index, name) -> index of the new layer or None
pub(crate) fn lay_new(p: &mut Pn, doc: DocId, index: i32, name: Option<&str>) -> Option<i32> {
    let d = p.doc.get_mut(doc)?;
    let r = lay_new_d(d, index, name);
    if r.is_some() {
        p.invalidate();
    }
    r
}

/// lay_new of a document record (doc_mark_layers included: the UI must be invalidated)
pub(crate) fn lay_new_d(d: &mut Doc, index: i32, name: Option<&str>) -> Option<i32> {
    let i = lay_insert_rec(d, index);
    // opacity 255, Normal
    let Some(sf) = Surface::try_new(d.w, d.h, FMT_ARGB) else {
        d.layers.remove(i);
        fix_cur_after_remove(d, i as i32);
        return None;
    };
    let nm = default_name(d, name);
    let l = &mut d.layers[i];
    l.sf = sf;
    l.name = nm;
    doc_mark_layers_d(d);
    Some(i as i32)
}

/// lay_add_sf(doc, index, sf, name) -> index (sf is given back on failure)
pub(crate) fn lay_add_sf(p: &mut Pn, doc: DocId, index: i32, sf: Surface, name: Option<&str>) -> Result<i32, Surface> {
    let Some(d) = p.doc.get_mut(doc) else { return Err(sf) };
    // a surface that does not fit its own geometry would make every pixel loop read out of range
    let rows = (sf.h.max(0) as usize).saturating_mul(sf.stride.max(0) as usize);
    if sf.fmt != FMT_ARGB || sf.w <= 0 || sf.h <= 0 || sf.stride < sf.w.saturating_mul(4) || sf.px.len() < rows {
        return Err(sf);
    }
    let i = lay_insert_rec(d, index);
    let nm = default_name(d, name);
    let l = &mut d.layers[i];
    l.name = nm;
    l.sf = sf;
    l.sf.opac = 255;
    l.sf.blend = BM_NORMAL as u8;
    doc_mark_layers_d(d);
    p.invalidate();
    Ok(i as i32)
}

/// lay_delete(doc, i) -> 0 / -1 (the last layer is never deleted).  The record leaves the
/// array first; hist_release takes what the history still references.
pub(crate) fn lay_delete(p: &mut Pn, doc: DocId, i: i32) -> i32 {
    let Some(d) = p.doc.get_mut(doc) else { return -1 };
    let n = d.layers.len() as i32;
    if n < 2 || i as u32 >= n as u32 {
        return -1;
    }
    let rec = d.layers.remove(i as usize);
    hist_release(p, doc, rec);
    let d = p.doc.dm(doc);
    fix_cur_after_remove(d, i);
    doc_mark_layers_d(d);
    p.invalidate();
    0
}

/// lay_move(doc, from, to) -- both clamped to the layer range
pub(crate) fn lay_move(p: &mut Pn, doc: DocId, from: i32, to: i32) {
    let Some(d) = p.doc.get_mut(doc) else { return };
    let top = d.layers.len() as i32 - 1;
    if top < 0 {
        return;
    }
    let from = from.max(0).min(top);
    let to = to.max(0).min(top);
    if from == to {
        return;
    }
    let rec = d.layers.remove(from as usize);
    d.layers.insert(to as usize, rec);
    // the current layer follows its record
    let mut c = d.cur;
    if c == from {
        c = to;
    } else if from > to {
        // from > to: [to, from) moved up
        if c >= to && c < from {
            c += 1;
        }
    } else if c > from && c <= to {
        // from < to: (from, to] moved down
        c -= 1;
    }
    d.cur = c;
    doc_mark_layers_d(d);
    p.invalidate();
}

/// lay_rec(doc, i) -> LAYER when 0 <= i < n, else None (no clamping)
pub(crate) fn lay_rec(d: &Doc, i: i32) -> Option<&Layer> {
    d.layers.get(i as u32 as usize)
}

/// lay_rec, mutable
pub(crate) fn lay_rec_mut(d: &mut Doc, i: i32) -> Option<&mut Layer> {
    d.layers.get_mut(i as u32 as usize)
}

/// lay_set_name(doc, i, str) -> 0 / -1 (the old name goes through hist_release)
pub(crate) fn lay_set_name(p: &mut Pn, doc: DocId, i: i32, s: &str) -> i32 {
    let Some(d) = p.doc.get_mut(doc) else { return -1 };
    let Some(l) = lay_rec_mut(d, i) else { return -1 };
    let old = core::mem::replace(&mut l.name, String::from(s));
    let tmp = Layer { name: old, ..Layer::default() };
    hist_release(p, doc, tmp);
    let d = p.doc.dm(doc);
    d.ver_lay = d.ver_lay.wrapping_add(1);
    d.flags |= DF_MODIFIED as u32;
    p.repass();
    0
}

/// lay_set_opacity(doc, i, v) -- v clamped to 0..255
pub(crate) fn lay_set_opacity(p: &mut Pn, doc: DocId, i: i32, v: i32) {
    let Some(d) = p.doc.get_mut(doc) else { return };
    let Some(l) = lay_rec_mut(d, i) else { return };
    l.sf.opac = v.clamp(0, 255) as u8;
    doc_mark_layers_d(d);
    p.invalidate();
}

/// lay_set_blend(doc, i, mode) -- BM_NORMAL .. BM_XOR (others -> Normal)
pub(crate) fn lay_set_blend(p: &mut Pn, doc: DocId, i: i32, mode: u32) {
    let Some(d) = p.doc.get_mut(doc) else { return };
    let Some(l) = lay_rec_mut(d, i) else { return };
    l.sf.blend = if mode <= BM_XOR { mode as u8 } else { 0 };
    doc_mark_layers_d(d);
    p.invalidate();
}

/// lay_set_visible(doc, i, on)
pub(crate) fn lay_set_visible(p: &mut Pn, doc: DocId, i: i32, on: bool) {
    let Some(d) = p.doc.get_mut(doc) else { return };
    let Some(l) = lay_rec_mut(d, i) else { return };
    l.flags &= !(LYF_VISIBLE as u32);
    if on {
        l.flags |= LYF_VISIBLE as u32;
    }
    doc_mark_layers_d(d);
    p.invalidate();
}

/// lay_dup(doc, i) -> index of the copy (above i, same pixels / opacity / mode / visibility)
pub(crate) fn lay_dup(p: &mut Pn, doc: DocId, i: i32) -> Option<i32> {
    let d = p.doc.get_mut(doc)?;
    let src = lay_rec(d, i)?;
    let name = fmt_cap(alloc::format!("{} Kopie", src.name), LNAME_CAP as usize);
    let j = lay_new_d(d, i + 1, Some(&name))? as usize;
    let i = i as usize; // the source stays below the copy
    let (lo, hi) = d.layers.split_at_mut(j);
    let s = &lo[i];
    let c = &mut hi[0];
    let nb = (s.sf.stride as usize * s.sf.h as usize).min(c.sf.px.len()).min(s.sf.px.len());
    c.sf.px[..nb].copy_from_slice(&s.sf.px[..nb]);
    c.sf.opac = s.sf.opac;
    c.sf.blend = s.sf.blend;
    c.flags = s.flags & LYF_VISIBLE as u32;
    c.ver = c.ver.wrapping_add(1);
    d.ver_px = d.ver_px.wrapping_add(1);
    doc_mark_layers_d(d);
    p.invalidate();
    Some(j as i32)
}

/// lay_merge_down(doc, i) -> 0 / -1 -- the lower layer gets a new surface (its old one may
/// belong to a history entry: hist_release) = old lower blended with layer i
pub(crate) fn lay_merge_down(p: &mut Pn, doc: DocId, i: i32) -> i32 {
    let Some(d) = p.doc.get_mut(doc) else { return -1 };
    if i <= 0 || i >= d.layers.len() as i32 {
        return -1;
    }
    let iu = i as usize;
    let (lo, hi) = d.layers.split_at_mut(iu);
    let lower = &mut lo[iu - 1];
    let upper = &hi[0];
    if lower.sf.px.is_empty() || upper.sf.px.is_empty() {
        return -1;
    }
    // the new lower surface: a copy of the old one
    let mut nsf = Surface::new(lower.sf.w, lower.sf.h, FMT_ARGB);
    if nsf.px.is_empty() || nsf.stride != lower.sf.stride {
        return -1; // (same size and format: same stride)
    }
    let nb = lower.sf.stride as usize * lower.sf.h as usize;
    nsf.px[..nb].copy_from_slice(&lower.sf.px[..nb]);
    let w = lower.sf.w.min(upper.sf.w);
    let h = lower.sf.h.min(upper.sf.h);
    let mode = (upper.sf.opac as u32) << 8 | upper.sf.blend as u32;
    {
        let ls = nsf.stride as usize;
        let us = upper.sf.stride as usize;
        let up = &upper.sf.px;
        let bands: Vec<SpinLock<&mut [u8]>> = nsf.px.chunks_mut(TILE as usize * ls).map(SpinLock::new).collect();
        let job = |band: usize, _: &plat::JobCtl| {
            if let Some(b) = bands.get(band) {
                let mut g = b.lock();
                // rows band * 64 .. +63 of the lower layer
                let y0 = (band as i32) << TILE_SH;
                let y1 = (y0 + TILE).min(h);
                for y in y0..y1 {
                    let o = (y - y0) as usize * ls;
                    let u = y as usize * us;
                    bm_row_inplace(&mut g[o..o + w as usize * 4], &up[u..u + w as usize * 4], mode);
                }
            }
        };
        let _ = plat::parallel_for(((h + TILE - 1) as u32 >> TILE_SH) as usize, &job);
    }
    // install the new surface (opacity and mode of the lower layer stay)
    nsf.opac = lower.sf.opac;
    nsf.blend = lower.sf.blend;
    let old = core::mem::replace(&mut lower.sf, nsf);
    lower.ver = lower.ver.wrapping_add(1);
    d.ver_px = d.ver_px.wrapping_add(1);
    hist_release(p, doc, Layer { sf: old, ..Layer::default() });
    lay_delete(p, doc, i);
    p.doc.dm(doc).cur = i - 1;
    0
}

// ------------------------------------------------------------------ commands

/// the commands lay_states handles (disabled without a document)
static LAY_MENU_CMDS: [u32; 13] = [
    CMD_LAYER_ADD,
    CMD_LAYER_DEL,
    CMD_LAYER_DUP,
    CMD_LAYER_MERGE,
    CMD_LAYER_GO_TOP,
    CMD_LAYER_GO_UP,
    CMD_LAYER_GO_DOWN,
    CMD_LAYER_GO_BOTTOM,
    CMD_LAYER_TO_TOP,
    CMD_LAYER_UP,
    CMD_LAYER_DOWN,
    CMD_LAYER_TO_BOTTOM,
    CMD_LAYER_TOGGLE_VIS,
];
static LAY_A4_CMDS: [u32; 6] =
    [CMD_LAYER_IMPORT, CMD_LAYER_FLIP_H, CMD_LAYER_FLIP_V, CMD_LAYER_ROT_180, CMD_LAYER_ROTZOOM, CMD_LAYER_PROPS];

/// lay_states() -- Ebenen menu (the M3 items keep their disabled state from cmds.rs)
pub(crate) fn lay_states(p: &mut Pn, ui: &mut Ui) {
    let Some(d) = p.doc.active() else {
        for &c in LAY_MENU_CMDS.iter().chain(LAY_A4_CMDS.iter()) {
            pn_cmd_set(ui, c, CF_DISABLED);
        }
        return;
    };
    let cur = d.cur;
    let top = d.layers.len() as i32 - 1;
    pn_cmd_set(ui, CMD_LAYER_ADD, 0);
    pn_cmd_set(ui, CMD_LAYER_DUP, 0);
    pn_cmd_set(ui, CMD_LAYER_TOGGLE_VIS, 0);
    // delete: more than one layer
    pn_cmd_set(ui, CMD_LAYER_DEL, if top > 0 { 0 } else { CF_DISABLED });
    // below the current layer: merge, go down / bottom, move down / bottom
    let f = if cur > 0 { 0 } else { CF_DISABLED };
    for c in [CMD_LAYER_MERGE, CMD_LAYER_GO_DOWN, CMD_LAYER_GO_BOTTOM, CMD_LAYER_DOWN, CMD_LAYER_TO_BOTTOM] {
        pn_cmd_set(ui, c, f);
    }
    // above: go up / top, move up / top
    let f = if cur < top { 0 } else { CF_DISABLED };
    for c in [CMD_LAYER_GO_UP, CMD_LAYER_GO_TOP, CMD_LAYER_UP, CMD_LAYER_TO_TOP] {
        pn_cmd_set(ui, c, f);
    }
    // A4: import, flips, Rotationszoom, Ebeneneigenschaften (any image)
    for &c in LAY_A4_CMDS.iter() {
        pn_cmd_set(ui, c, 0);
    }
}

/// lay_cmd(id, arg) -> handled
pub(crate) fn lay_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    // A4: the layers window's thumbnail timer
    if id == UIC_TIMER {
        if arg != TMR_WLAY {
            return false;
        }
        p.invalidate();
        return true;
    }
    if id.wrapping_sub(CMD_LAYER_FIRST) > CMD_LAYER_LAST - CMD_LAYER_FIRST {
        return false;
    }
    // A4: internal commands (dialog results, test hooks) keep their own document
    if id >= CMD_LAY_TEST0 {
        a4_test(p, ui, id, arg);
        return true;
    }
    match id {
        CMD_LAY_PROPS_DONE => {
            crate::workspace::dialogs::layer_properties::lprops_done(p, ui, arg != 0);
            return true;
        }
        CMD_LAY_RZ_DONE => {
            crate::workspace::dialogs::rotate_zoom::rz_done(p, ui, arg != 0);
            return true;
        }
        CMD_LAY_RZ_UPDATE => {
            crate::workspace::dialogs::rotate_zoom::rz_update(p, ui);
            return true;
        }
        CMD_LAY_IMP_FILE => {
            imp_chosen(p);
            return true;
        }
        CMD_LAY_IMP_ANSWER => {
            imp_answer(p, arg);
            return true;
        }
        CMD_LAY_FLAT_ANSWER => {
            flat_answer(p, arg);
            return true;
        }
        _ => {}
    }
    let Some(doc) = doc_active(&p.doc) else { return true };
    let d = p.doc.d(doc);
    let cur = d.cur;
    let top = d.layers.len() as i32 - 1;
    let set_cur = |p: &mut Pn, v: i32| {
        let v = v.max(0).min(top);
        let d = p.doc.dm(doc);
        if v != d.cur {
            d.cur = v;
            p.repass();
        }
    };
    let mv = |p: &mut Pn, name: &'static str, t: i32| {
        if t < 0 || t > top || t == cur {
            return;
        }
        let icon = if t > cur { IC_UP } else { IC_DOWN };
        if hist_push_layers(p, doc, name, icon) {
            lay_move(p, doc, cur, t);
        }
    };
    match id {
        CMD_LAYER_IMPORT => imp_start(p, doc),
        CMD_LAYER_FLIP_H => {
            lay_flip(p, doc, cur, 0);
        }
        CMD_LAYER_FLIP_V => {
            lay_flip(p, doc, cur, 1);
        }
        CMD_LAYER_ROT_180 => {
            lay_flip(p, doc, cur, 2);
        }
        CMD_LAYER_ROTZOOM => crate::workspace::dialogs::rotate_zoom::rz_open(p, ui, doc),
        CMD_LAYER_PROPS => crate::workspace::dialogs::layer_properties::lprops_open(p, ui, doc, cur),
        CMD_LAYER_ADD => {
            if hist_push_layers(p, doc, "Neue Ebene", IC_ADD) && lay_new(p, doc, cur + 1, None).is_some() {
                p.doc.dm(doc).cur = cur + 1;
            }
        }
        CMD_LAYER_DEL => {
            if top > 0 && hist_push_layers(p, doc, "Ebene löschen", IC_DELETE) {
                lay_delete(p, doc, cur);
            }
        }
        CMD_LAYER_DUP => {
            if hist_push_layers(p, doc, "Ebene verdoppeln", IC_COPY) && lay_dup(p, doc, cur).is_some() {
                p.doc.dm(doc).cur = cur + 1;
            }
        }
        CMD_LAYER_MERGE => {
            if cur > 0 && hist_push_layers(p, doc, "Nach unten vereinen", IC_DOWNLOAD) {
                lay_merge_down(p, doc, cur);
            }
        }
        CMD_LAYER_GO_TOP => set_cur(p, top),
        CMD_LAYER_GO_UP => set_cur(p, cur + 1),
        CMD_LAYER_GO_DOWN => set_cur(p, cur - 1),
        CMD_LAYER_GO_BOTTOM => set_cur(p, 0),
        CMD_LAYER_SELECT => set_cur(p, arg as i32),
        CMD_LAYER_TO_TOP => mv(p, "Ebene ganz nach oben", top),
        CMD_LAYER_UP => mv(p, "Ebene eine Stufe höher", cur + 1),
        CMD_LAYER_DOWN => mv(p, "Ebene eine Stufe tiefer", cur - 1),
        CMD_LAYER_TO_BOTTOM => mv(p, "Ebene ganz nach unten", 0),
        CMD_LAYER_REORDER => {
            let from = (arg & 0xFFFF) as i32;
            let to = (arg >> 16) as i32;
            if from <= top && to <= top && to != from && hist_push_layers(p, doc, "Ebene verschieben", IC_UP) {
                lay_move(p, doc, from, to);
            }
        }
        CMD_LAYER_TOGGLE_VIS | CMD_LAYER_SET_VIS => {
            let idx = if id == CMD_LAYER_SET_VIS { arg as i32 } else { cur };
            if id == CMD_LAYER_SET_VIS && arg > top as u32 {
                return true;
            }
            if hist_push_layer_props(p, doc, idx, "Ebene Sichtbarkeit", IC_VIEW) {
                if let Some(l) = lay_rec(p.doc.d(doc), idx) {
                    let on = !l.flags & LYF_VISIBLE as u32 != 0;
                    lay_set_visible(p, doc, idx, on);
                }
            }
        }
        _ => return false,
    }
    true
}

// ================================================================== A4 (layers package)

/// lay_init() -- import file types, start folder (A4)
pub(crate) fn lay_init(p: &mut Pn, _ui: &mut Ui) {
    imp_build_exts(&mut p.lay);
    p.lay.imp_dir = cap_path(lay_slashes(&plat::cwd()));
}

/// lay_frame() -- dialogs of the layers package (main window context) and the probes
pub(crate) fn lay_frame(p: &mut Pn, ui: &mut Ui) {
    imp_filedlg(p, ui);
    imp_ask_window(p, ui);
    flat_ask_window(p, ui);
    crate::workspace::dialogs::layer_properties::lprops_frame(p, ui);
    crate::workspace::dialogs::rotate_zoom::rz_frame(p, ui);
    a4_probes(p, ui);
}

// lay_slashes(path) -- '\' -> '/'
fn lay_slashes(s: &str) -> String {
    s.replace('\\', "/")
}

// a path in a LAY_PATH_CAP buffer (str_copy: cut to cap - 1 bytes)
fn cap_path(s: String) -> String {
    fmt_cap(s, LAY_PATH_CAP as usize)
}

// lay_file_part(path) -> the file name inside path
fn lay_file_part(path: &str) -> &str {
    match path.rfind(['/', '\\']) {
        Some(i) => &path[i + 1..],
        None => path,
    }
}

/// lay_find_uid(doc, uid) -> index | None
pub(crate) fn lay_find_uid(d: &Doc, uid: u64) -> Option<i32> {
    d.layers.iter().position(|l| l.uid == uid).map(|i| i as i32)
}

/// lay_lines(style, str) -- one ui.text(style) per '\n'-separated line (ui.text does not wrap)
pub(crate) fn lay_lines(ui: &mut Ui, style: u32, s: &str) {
    for line in s.split('\n') {
        let mut n = line.len().min(LINE_CAP - 1);
        while !line.is_char_boundary(n) {
            n -= 1;
        }
        ui.text(style, &line[..n]);
    }
}

// ------------------------------------------------------------------ surfaces

// sf_copy_tl(dst, src) -- the overlapping top-left part of src into dst (32-bit pixels)
fn sf_copy_tl(dst: &mut Surface, src: &Surface) {
    let nb = dst.w.min(src.w) as usize * 4;
    let h = dst.h.min(src.h);
    if src.px.is_empty() || dst.px.is_empty() {
        return;
    }
    let (ds, ss) = (dst.stride as usize, src.stride as usize);
    for y in 0..h.max(0) as usize {
        dst.px[y * ds..y * ds + nb].copy_from_slice(&src.px[y * ss..y * ss + nb]);
    }
}

// sf_fill(sf, argb) -- every pixel = argb
fn sf_fill(sf: &mut Surface, argb: u32) {
    let c = argb.to_le_bytes();
    let (w, s) = (sf.w.max(0) as usize, sf.stride as usize);
    for y in 0..sf.h.max(0) as usize {
        for px in sf.px[y * s..y * s + w * 4].chunks_exact_mut(4) {
            px.copy_from_slice(&c);
        }
    }
}

// ------------------------------------------------------------------ flip / rotate 180

/// lay_flip(doc, i, kind) -> 1 = history step -- the whole layer i in place: kind 0
/// "Horizontal kippen", 1 "Vertikal kippen", 2 "Um 180° rotieren" (every tile saved first)
pub(crate) fn lay_flip(p: &mut Pn, doc: DocId, i: i32, kind: i32) -> bool {
    let Some(d) = p.doc.get(doc) else { return false };
    let Some(l) = lay_rec(d, i) else { return false };
    if l.sf.px.is_empty() {
        return false;
    }
    if hist_px_begin(p, doc, i) != 0 {
        return false;
    }
    let (w, h) = {
        let l = lay_rec(p.doc.d(doc), i).expect("layer");
        (l.sf.w, l.sf.h)
    };
    if hist_px_touch(p, doc, 0, 0, w, h) != 0 {
        hist_px_cancel(p, doc);
        return false;
    }
    let l = lay_rec_mut(p.doc.dm(doc), i).expect("layer");
    let name = match kind as u32 {
        0 => {
            img_core::ops::flip_h(&mut l.sf);
            "Horizontal kippen"
        }
        1 => {
            img_core::ops::flip_v(&mut l.sf);
            "Vertikal kippen"
        }
        _ => {
            img_core::ops::rotate180(&mut l.sf);
            "Um 180° rotieren"
        }
    };
    doc_mark_px(p, doc, i, 0, 0, w, h);
    hist_px_end(p, doc, name, if kind == 2 { IC_ROTATE } else { IC_SWITCH })
}

// ------------------------------------------------------------------ Zusammenfassen

/// lay_flat_states() -- CMD_FLATTEN: an image with more than one layer (iop_states calls it)
pub(crate) fn lay_flat_states(p: &mut Pn, ui: &mut Ui) {
    let on = p.doc.active().is_some_and(|d| d.layers.len() >= 2);
    pn_cmd_set(ui, CMD_FLATTEN, if on { 0 } else { CF_DISABLED });
}

/// lay_flatten_cmd() -- Bild > Zusammenfassen (iop_cmd forwards CMD_FLATTEN): with a hidden
/// layer paint.net asks "Ausgeblendete Ebenen löschen?" first
pub(crate) fn lay_flatten_cmd(p: &mut Pn, _ui: &mut Ui) {
    let Some(doc) = doc_active(&p.doc) else { return };
    let d = p.doc.d(doc);
    if d.layers.len() < 2 {
        return;
    }
    if d.layers.iter().any(|l| l.flags & LYF_VISIBLE as u32 == 0) {
        p.lay.flat_doc = Some(doc);
        p.lay.flat_open.set(1);
        p.lay.flat_pending = true;
        p.repass();
        return;
    }
    lay_flatten(p, doc);
}

// flat_answer(ok) -- CMD_LAY_FLAT_ANSWER
fn flat_answer(p: &mut Pn, ok: u32) {
    p.lay.flat_pending = false;
    if ok != 1 {
        return;
    }
    if let Some(doc) = p.lay.flat_doc {
        if doc_index(&p.doc, doc) >= 0 {
            lay_flatten(p, doc);
        }
    }
}

/// lay_flatten(doc) -> 0 / -1 -- the composite of the visible layers becomes the bottom layer
/// (Normal, 255, visible; its name and background flag stay), every other layer is deleted:
/// one history step "Zusammenfassen"
pub(crate) fn lay_flatten(p: &mut Pn, doc: DocId) -> i32 {
    let Some(d) = p.doc.get(doc) else { return -1 };
    if d.layers.len() < 2 {
        return -1;
    }
    let (w, h) = (d.w, d.h);
    let mut nsf = {
        let c = crate::document::composite::comp_surface(p, doc);
        if c.px.is_empty() {
            return -1;
        }
        let mut s = Surface::new(w, h, FMT_ARGB);
        if s.px.is_empty() {
            return -1;
        }
        sf_copy_tl(&mut s, c);
        s
    };
    if !hist_push_layers(p, doc, "Zusammenfassen", IC_LAYERS) {
        return -1;
    }
    // every layer above the bottom one goes (lay_delete: record out, hist_release)
    loop {
        let top = p.doc.d(doc).layers.len() as i32 - 1;
        if top <= 0 || lay_delete(p, doc, top) != 0 {
            break;
        }
    }
    let d = p.doc.dm(doc);
    let l = &mut d.layers[0];
    nsf.opac = 255;
    nsf.blend = BM_NORMAL as u8;
    let old = core::mem::replace(&mut l.sf, nsf);
    l.flags |= LYF_VISIBLE as u32;
    l.ver = l.ver.wrapping_add(1);
    d.ver_px = d.ver_px.wrapping_add(1);
    d.cur = 0;
    hist_release(p, doc, Layer { sf: old, ..Layer::default() });
    doc_mark_layers_d(p.doc.dm(doc));
    p.invalidate();
    0
}

// flat_ask_window() -- "Zusammenfassen: Ausgeblendete Ebenen löschen?" (modal)
fn flat_ask_window(p: &mut Pn, ui: &mut Ui) {
    let l = &mut p.lay;
    if l.flat_pending && l.flat_open.get() == 0 {
        l.flat_pending = false; // Esc / close button: Abbrechen
    }
    let open = l.flat_open.clone();
    let rect = l.flat_rect.clone();
    if ui.window_begin(WINID_FLATASK, crate::preferences::localization::tr("Zusammenfassen"), Some(&open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&rect)) {
        ui.text(TS_BODY, crate::preferences::localization::tr("Ausgeblendete Ebenen löschen?"));
        let r = ui.dialog_buttons(Some(crate::preferences::localization::tr("OK")), None, Some(crate::preferences::localization::tr("Abbrechen")));
        if r != DLG_NONE {
            p.lay.flat_pending = false;
            if r == DLG_PRIMARY {
                ui.cmd_run(CMD_LAY_FLAT_ANSWER, 1);
            }
        }
    }
    ui.window_end();
}

// ------------------------------------------------------------------ Als Ebene importieren

// imp_build_exts() -- the type list for ui.filedlg: "ext;ext;...=Alle Bildtypen", then the
// entry of every type with a decoder (exts)
fn imp_build_exts(l: &mut Lay) {
    let mut all = String::new();
    for f in 1..IMGF_COUNT {
        let Some(fi) = img_core::io::format_info(f) else { continue };
        if fi.caps & IFC_DECODE == 0 || fi.exts.is_empty() {
            continue;
        }
        if !all.is_empty() {
            if all.len() >= IMPX_CAP / 2 {
                continue;
            }
            all.push(';');
        }
        for c in fi.exts.bytes() {
            if c == 0 || c == b'=' || all.len() >= IMPX_CAP / 2 {
                break;
            }
            all.push(c as char);
        }
    }
    // ibe_put(str): append str with its NUL at g_ibe_pos (skipped when full); the first one
    // ("=Alle Bildtypen") continues the extensions written so far
    let mut pos = all.len();
    let tail = "=Alle Bildtypen";
    if pos + tail.len() + 2 < IMPX_CAP {
        all.push_str(tail);
        pos += tail.len() + 1;
    }
    let mut list = alloc::vec![all];
    for f in 1..IMGF_COUNT {
        let Some(fi) = img_core::io::format_info(f) else { continue };
        if fi.caps & IFC_DECODE == 0 || fi.exts.is_empty() {
            continue;
        }
        if pos + fi.exts.len() + 2 < IMPX_CAP {
            pos += fi.exts.len() + 1;
            list.push(String::from(fi.exts));
        }
    }
    l.imp_exts = list;
}

// imp_start(doc) -- Als Ebene importieren…: the open dialog in the folder used last
fn imp_start(p: &mut Pn, doc: DocId) {
    let l = &mut p.lay;
    l.imp_doc = Some(doc);
    l.imp_path = l.imp_dir.clone();
    if !l.imp_path.is_empty() {
        l.imp_path.push('/');
        l.imp_path = cap_path(core::mem::take(&mut l.imp_path));
    }
    l.imp_fd_open.set(1);
    p.repass();
}

// imp_filedlg() -- the import file dialog (every frame, like any dialog)
fn imp_filedlg(p: &mut Pn, ui: &mut Ui) {
    let l = &mut p.lay;
    let exts: Vec<&str> = l.imp_exts.iter().map(|s| s.as_str()).collect();
    let open = l.imp_fd_open.clone();
    if ui.filedlg(ID_FD_IMPORT, FD_OPEN, &open, &mut l.imp_path, LAY_PATH_CAP as u32, &exts) & EV_ACT != 0 {
        ui.cmd_run(CMD_LAY_IMP_FILE, 0);
    }
}

// imp_chosen() -- CMD_LAY_IMP_FILE: load g_imp_path; its folder is the next start folder
fn imp_chosen(p: &mut Pn) {
    let tmp = lay_slashes(&p.lay.imp_path);
    if let Some(i) = tmp.rfind('/') {
        p.lay.imp_dir = String::from(&tmp[..i]);
    }
    imp_clear(&mut p.lay);
    let path = p.lay.imp_path.clone();
    imp_load(p, &path);
    imp_decide(p);
}

/// lay_import_paths(doc, paths) -> records loaded -- every file of the list as layers above the
/// current layer of doc in one history step; images larger than the canvas ask first (drag and
/// drop / tests)
pub(crate) fn lay_import_paths(p: &mut Pn, _ui: &mut Ui, doc: DocId, paths: &[String]) -> i32 {
    p.lay.imp_doc = Some(doc);
    imp_clear(&mut p.lay);
    for path in paths {
        if path.is_empty() {
            break;
        }
        imp_load(p, path);
    }
    let n = p.lay.imp_list.len() as i32;
    imp_decide(p);
    n
}

// imp_size(sf) -- the largest imported size so far (g_imp_w / g_imp_h)
fn imp_size(l: &mut Lay, w: i32, h: i32) {
    l.imp_w = l.imp_w.max(w);
    l.imp_h = l.imp_h.max(h);
}

// imp_clear() -- free every import record
fn imp_clear(l: &mut Lay) {
    l.imp_list.clear();
    l.imp_w = 0;
    l.imp_h = 0;
}

// imp_error(err, path) -- the German message for a file that cannot be imported
fn imp_error(p: &mut Pn, err: i32, path: &str) {
    let msg = match err {
        ERR_IO => {
            if plat::file_exists(path) != 0 {
                "Die Datei ließ sich nicht vom Datenträger lesen."
            } else {
                "Datei nicht gefunden."
            }
        }
        ERR_UNSUPPORTED => "Dieses Bildformat wird nicht erkannt; die Datei lässt sich nicht öffnen.",
        ERR_NOMEM | ERR_LIMIT => "Zu wenig Arbeitsspeicher, um das Bild zu laden.",
        ERR_DIMS => "Das Bild ist zu groß.",
        _ => "Die Datei ist defekt oder passt nicht zu ihrem Format.",
    };
    crate::integrations::file_io::fio_error(p, Some(lay_file_part(path)), msg);
}

// imp_take_pdn(path, pdndoc) -- one import record per layer of a decoded .pdn ("file.pdn:
// layer name", opacity, blend mode, visibility, metadata)
fn imp_take_pdn(l: &mut Lay, path: &str, pd: img_pdn::PdnDoc) {
    let file = lay_file_part(path);
    for pl in pd.layers {
        let lname = String::from_utf8_lossy(&pl.name);
        let name = fmt_cap(alloc::format!("{}: {}", file, lname), LNAME_CAP as usize);
        let (w, h) = (pl.sf.w, pl.sf.h);
        l.imp_list.push(ImpRec {
            sf: pl.sf,
            name: Some(name),
            opac: pl.opacity,
            blend: if pl.blend as u32 <= BM_XOR { pl.blend } else { 0 },
            vis: pl.visible,
            meta: pl.meta,
        });
        imp_size(l, w, h);
    }
}

// imp_load(path) -> 0 / ERR_* -- decode an image (one record named after the file) or a .pdn
// (every layer); errors go to the error bar
fn imp_load(p: &mut Pn, path: &str) -> i32 {
    let Some(data) = plat::read_file(path) else {
        imp_error(p, ERR_IO, path);
        return ERR_IO;
    };
    let hint = img_core::io::format_from_path(path);
    let mut err = ERR_UNSUPPORTED;
    let dec = ImgDec { flags: IMGD_STRAIGHT, hint, maxmem: crate::integrations::file_io::img_mem_budget(), ..ImgDec::default() };
    if img_core::io::detect(&data, hint) == IMGF_PDN {
        match img_pdn::decode(&data, Some(&dec)) {
            Ok(pd) => {
                imp_take_pdn(&mut p.lay, path, pd);
                err = 0;
            }
            Err(e) => err = e,
        }
    }
    if err == ERR_UNSUPPORTED {
        match img_core::io::decode_ex(&data, Some(&dec), None) {
            Ok(sf) => {
                let (w, h) = (sf.w, sf.h);
                let name = String::from(lay_file_part(path));
                p.lay.imp_list.push(ImpRec { sf, name: Some(name), opac: 255, blend: 0, vis: 1, meta: Vec::new() });
                imp_size(&mut p.lay, w, h);
                err = 0;
            }
            Err(e) => err = e,
        }
    }
    if err != 0 {
        imp_error(p, err, path);
    }
    err
}

// imp_decide() -- the loaded records: ask when an image is larger than the canvas, else
// import them at once
fn imp_decide(p: &mut Pn) {
    if p.lay.imp_list.is_empty() {
        return;
    }
    let Some(d) = p.lay.imp_doc.and_then(|doc| p.doc.get(doc)) else {
        imp_clear(&mut p.lay);
        return;
    };
    if p.lay.imp_w > d.w || p.lay.imp_h > d.h {
        p.lay.imp_ask_open.set(1);
        p.lay.imp_ask_pending = true;
        p.repass();
        return;
    }
    imp_commit(p, 2);
}

// imp_answer(a) -- CMD_LAY_IMP_ANSWER: 1 enlarge the canvas, 2 keep it, else cancel
fn imp_answer(p: &mut Pn, a: u32) {
    p.lay.imp_ask_pending = false;
    p.lay.imp_ask_open.set(0);
    if a.wrapping_sub(1) <= 1 {
        imp_commit(p, a);
    } else {
        imp_clear(&mut p.lay);
    }
}

// imp_fit(rec, w, h) -> 0 / -1 -- the record's surface becomes w x h (image at the top left,
// the rest transparent; a larger image is cut)
fn imp_fit(r: &mut ImpRec, w: i32, h: i32) -> i32 {
    if r.sf.w == w && r.sf.h == h {
        return 0;
    }
    let mut s = Surface::new(w, h, FMT_ARGB);
    if s.px.is_empty() {
        return -1;
    }
    sf_copy_tl(&mut s, &r.sf);
    r.sf = s;
    0
}

// grow_prepare(doc, w, h) -> every layer on a w x h canvas (pixels at the top left; the new
// area of the background layer in the secondary colour, of other layers transparent), None =
// no memory (nothing changed)
fn grow_prepare(p: &Pn, d: &Doc, w: i32, h: i32) -> Option<Vec<Surface>> {
    let fill = crate::workspace::palette::pal_secondary(&p.pal);
    let mut v = Vec::with_capacity(d.layers.len());
    for l in d.layers.iter() {
        let mut s = Surface::new(w, h, FMT_ARGB);
        if s.px.is_empty() {
            return None;
        }
        if l.flags & LYF_BG as u32 != 0 {
            sf_fill(&mut s, fill);
        }
        sf_copy_tl(&mut s, &l.sf);
        v.push(s);
    }
    Some(v)
}

// grow_install(doc, array, w, h) -- the prepared surfaces replace the layers' surfaces (the old
// ones through hist_release), the image becomes w x h
fn grow_install(p: &mut Pn, doc: DocId, sfs: Vec<Surface>, w: i32, h: i32) {
    for (i, mut s) in sfs.into_iter().enumerate() {
        let d = p.doc.dm(doc);
        let Some(l) = d.layers.get_mut(i) else { break };
        s.opac = l.sf.opac;
        s.blend = l.sf.blend;
        let old = core::mem::replace(&mut l.sf, s);
        l.ver = l.ver.wrapping_add(1);
        hist_release(p, doc, Layer { sf: old, ..Layer::default() });
    }
    let d = p.doc.dm(doc);
    d.w = w;
    d.h = h;
    doc_resized(p, doc);
    // the view places the image again (like imgops' canvas size; the selection mask follows
    // in sel_states)
    p.doc.dm(doc).view.zoom = 0;
    crate::workspace::view::view_invalidate(p);
}

// imp_commit(mode) -- mode 1: the canvas grows to hold every imported image, 2: it keeps its
// size (images at the top left, cut).  The import records become layers above the current
// layer in one history step "Als Ebene importieren" (hist_push_doc when the size changes,
// else hist_push_layers); the topmost new layer becomes the current one
fn imp_commit(p: &mut Pn, mode: u32) {
    let doc = match p.lay.imp_doc {
        Some(doc) if doc_index(&p.doc, doc) >= 0 && !p.lay.imp_list.is_empty() => doc,
        _ => {
            imp_clear(&mut p.lay);
            return;
        }
    };
    let d = p.doc.d(doc);
    let (mut w, mut h) = (d.w, d.h);
    if mode == 1 {
        w = w.max(p.lay.imp_w);
        h = h.max(p.lay.imp_h);
    }
    let mut grown = None;
    let mut err = None;
    if w != d.w || h != d.h {
        grown = grow_prepare(p, d, w, h);
        if grown.is_none() {
            err = Some("Zu wenig Arbeitsspeicher für die neue Bildgröße.");
        }
    }
    if err.is_none() {
        for r in p.lay.imp_list.iter_mut() {
            if imp_fit(r, w, h) != 0 {
                err = Some("Zu wenig Arbeitsspeicher für die neue Ebenengröße.");
                break;
            }
        }
    }
    if let Some(msg) = err {
        crate::integrations::file_io::fio_error(p, Some("Als Ebene importieren"), msg);
        imp_clear(&mut p.lay);
        return;
    }
    let ok = if grown.is_some() {
        hist_push_doc(p, doc, "Als Ebene importieren", IC_OPEN_FILE)
    } else {
        hist_push_layers(p, doc, "Als Ebene importieren", IC_OPEN_FILE)
    };
    if !ok {
        imp_clear(&mut p.lay); // 0 = out of memory: no change
        return;
    }
    if let Some(g) = grown {
        grow_install(p, doc, g, w, h);
    }
    let mut pos = p.doc.d(doc).cur + 1; // insert position
    let mut added = 0;
    let list = core::mem::take(&mut p.lay.imp_list);
    for r in list {
        let ImpRec { sf, name, opac, blend, vis, meta } = r;
        if let Ok(i) = lay_add_sf(p, doc, pos, sf, name.as_deref()) {
            let l = &mut p.doc.dm(doc).layers[i as usize];
            l.sf.opac = opac;
            l.sf.blend = blend;
            l.flags = if vis != 0 { LYF_VISIBLE as u32 } else { 0 };
            l.meta = meta;
            l.ver = l.ver.wrapping_add(1);
            pos += 1;
            added += 1;
        }
    }
    let d = p.doc.dm(doc);
    if added != 0 {
        d.cur = pos - 1;
    }
    d.ver_px = d.ver_px.wrapping_add(1);
    doc_mark_layers_d(d);
    p.invalidate();
    imp_clear(&mut p.lay);
}

// imp_ask_window() -- "Das Bild ragt über die Leinwand hinaus." (modal):
// Leinwand vergrößern / Leinwand nicht ändern / Abbrechen
fn imp_ask_window(p: &mut Pn, ui: &mut Ui) {
    let l = &mut p.lay;
    if l.imp_ask_pending && l.imp_ask_open.get() == 0 {
        l.imp_ask_pending = false; // Esc / close button: Abbrechen
        ui.cmd_run(CMD_LAY_IMP_ANSWER, 0);
    }
    let open = l.imp_ask_open.clone();
    let rect = l.imp_ask_rect.clone();
    if ui.window_begin(WINID_IMPASK, crate::preferences::localization::tr("Als Ebene importieren"), Some(&open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&rect)) {
        let mut answer = 0;
        lay_lines(ui, TS_BODY, "Das Bild ragt über die Leinwand hinaus.\nWie soll es weitergehen?");
        // Leinwand vergrößern
        ui.box_begin(0, BOX_V, 4);
        if ui.accent_button(ID_IMP_ENLARGE, "Leinwand vergrößern") & EV_ACT != 0 {
            answer = 1;
        }
        lay_lines(
            ui,
            TS_CAPTION | TX_SECONDARY,
            "Die Leinwand wächst mit, bis das neue Bild\nvollständig darauf passt.",
        );
        ui.box_end();
        // Leinwand nicht ändern
        ui.box_begin(0, BOX_V, 4);
        if ui.button(ID_IMP_KEEP, crate::preferences::localization::tr("Leinwand nicht ändern")) & EV_ACT != 0 {
            answer = 2;
        }
        lay_lines(
            ui,
            TS_CAPTION | TX_SECONDARY,
            "Die Leinwand bleibt, wie sie ist. Was über den Rand ragt,\nwird abgeschnitten; schieben Sie das Bild so, dass der\ngewünschte Teil auf der Leinwand liegt.",
        );
        ui.box_end();
        if ui.dialog_buttons(None, None, Some("Abbrechen")) != DLG_NONE {
            answer = 3; // Abbrechen
        }
        if answer != 0 {
            p.lay.imp_ask_open.set(0);
            p.lay.imp_ask_pending = false;
            ui.cmd_run(CMD_LAY_IMP_ANSWER, if answer == 3 { 0 } else { answer });
        }
    }
    ui.window_end();
}

// ------------------------------------------------------------------ test hooks and probes

// a4_sem(id, label, value) -- a hidden K_TEXT record (empty rectangle)
fn a4_sem(ui: &mut Ui, id: u32, label: &str, value: i32) {
    ui.last_reset();
    ui.sem(K_TEXT, id, Some(label), value);
}

// a4_probes() -- hidden records (main window, every frame):
//   4390 text last probe (cmd 370 / 372)     4391 value history position, text current entry
//   4392 value history entries               4393 value layers-window thumbnail renders
//   4394 value Rotationszoom previews computed, text "angle|zoom|panx|pany" (x 100)
//   4395 value open A4 dialogs: 1 Ebeneneigenschaften, 2 Rotationszoom, 4 canvas question,
//        8 hidden-layers question, 16 import file dialog
fn a4_probes(p: &mut Pn, ui: &mut Ui) {
    a4_sem(ui, ID_A4_PROBE, &p.lay.a4_probe, 0);
    let (pos, n, name) = match p.doc.active() {
        Some(d) => {
            let pos = hist_pos(d);
            let n = hist_count(d);
            let name = if pos > 0 { String::from(hist_entry_name(d, pos - 1)) } else { String::new() };
            (pos, n, name)
        }
        None => (0, 0, String::new()),
    };
    a4_sem(ui, ID_A4_PROBE + 1, &name, pos);
    a4_sem(ui, ID_A4_PROBE + 2, "", n);
    a4_sem(ui, ID_A4_PROBE + 3, "", crate::workspace::panels::layers::wlay_thumb_count(&p.wlay));
    let (rt, rn) = crate::workspace::dialogs::rotate_zoom::rz_probe(&p.rz);
    a4_sem(ui, ID_A4_PROBE + 4, &rt, rn as i32);
    let mut bits = crate::workspace::dialogs::layer_properties::lprops_is_open(&p.lprops) as i32;
    bits |= (crate::workspace::dialogs::rotate_zoom::rz_is_open(&p.rz) as i32) << 1;
    if p.lay.imp_ask_open.get() != 0 {
        bits |= 4;
    }
    if p.lay.flat_open.get() != 0 {
        bits |= 8;
    }
    if p.lay.imp_fd_open.get() != 0 {
        bits |= 16;
    }
    a4_sem(ui, ID_A4_PROBE + 5, "", bits);
}

/// test fixtures (paths relative to RustLib/, the working directory of the tests)
static A4_FIXTURES: [&str; FIXT_N] = [
    "apps/PaintCraft/tests/fixtures/layers/l4_small.png",
    "apps/PaintCraft/tests/fixtures/layers/l4_wide.png",
    "apps/PaintCraft/tests/fixtures/layers/l4_tall.png",
    "apps/PaintCraft/tests/fixtures/pdn/c_layers.pdn",
    "build/a4_layers.pdn",
    "apps/PaintCraft/tests/fixtures/layers/missing.png",
    "apps/PaintCraft/tests/fixtures/layers/l4_lo.png",
    "apps/PaintCraft/tests/fixtures/layers/l4_hi.png",
];

// a4_test(id, arg) -- test hooks (cmd ID ARG in scripts, active image):
//   370 i                   layer i -> #4390 text "name|visible|opacity|blend" ("-": none)
//   371 mask                import the fixtures of the bits of mask at once
//   372 x | y<<12 | i<<24   pixel (x, y) of layer i -> #4390 text "AARRGGBB"
//   373 op<<16 | v          Rotationszoom parameter (rz_test; v signed 16 bit)
//   374 kind                lay_flip of the current layer (0, 1, 2) without the menu
fn a4_test(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) {
    if id == CMD_LAY_TEST0 + 3 {
        crate::workspace::dialogs::rotate_zoom::rz_test(p, ui, arg >> 16, arg as u16 as i16 as i32);
        return;
    }
    let Some(doc) = doc_active(&p.doc) else { return };
    match id - CMD_LAY_TEST0 {
        0 => {
            let t = match lay_rec(p.doc.d(doc), arg as i32) {
                Some(l) => alloc::format!(
                    "{}|{}|{}|{}",
                    l.name,
                    (l.flags & LYF_VISIBLE as u32) as i32,
                    l.sf.opac as i32,
                    l.sf.blend as i32
                ),
                None => String::from("-"),
            };
            p.lay.a4_probe = fmt_cap(t, A4_PROBE_CAP);
            p.repass();
        }
        1 => {
            // the fixture paths of the mask as a list
            let list: Vec<String> =
                (0..FIXT_N).filter(|&k| arg >> k & 1 != 0).map(|k| String::from(A4_FIXTURES[k])).collect();
            lay_import_paths(p, ui, doc, &list);
        }
        2 => {
            let t = match lay_rec(p.doc.d(doc), (arg >> 24) as i32) {
                Some(l) => {
                    let x = arg & 0xFFF;
                    let y = (arg >> 12) & 0xFFF;
                    let mut v = 0u32;
                    if x < l.sf.w as u32 && y < l.sf.h as u32 {
                        let o = y as usize * l.sf.stride as usize + x as usize * 4;
                        v = u32::from_le_bytes([l.sf.px[o], l.sf.px[o + 1], l.sf.px[o + 2], l.sf.px[o + 3]]);
                    }
                    alloc::format!("{:08X}", v)
                }
                None => String::from("-"),
            };
            p.lay.a4_probe = t;
            p.repass();
        }
        4 => {
            let c = p.doc.d(doc).cur;
            lay_flip(p, doc, c, arg as i32);
        }
        _ => {}
    }
}

const BM_NORMAL: u32 = 0;
