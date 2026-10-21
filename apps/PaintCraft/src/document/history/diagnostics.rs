// hist_diag -- PaintCraft: history test hooks (script `cmd ID ARG`, ids CMD_HIST_TEST0 + k) and
// probe texts; diag.rs routes CMD_HIST_TEST0..CMD_HIST_TEST1 here
//                                                         [apps/PaintCraft/src/hist_diag.s]
//
//   hist_diag_cmd(id, arg) -> true      the hooks below (active document)
//   hist_diag_frame()                   test mode: tool window "Verlauf-Test" (WINID_HTEST)
//                                       with probe texts "t.name=value" (expect text:"t.n=3")
//   hist_state_hash(doc) -> u64         FNV-1a of size, current layer, every layer's
//                                       properties, name and pixels
// Hooks (k = id - CMD_HIST_TEST0):
//    0 on        probes on / off              12 slot    compare with slot -> t.cmp=ok|diff
//    1 x0|y0<<16 rectangle start             13 kb      budget in KB (0 = default)
//    2 x1|y1<<16 rectangle end (exclusive)   14 n       the n-th next allocation fails
//    3 argb      colour                      15 on      spill file on / off
//    4 layer     fill rect (-1 = current)    16         compound: two pattern fills
//    5 seed      pattern fill (current)      17         selection step (hist_push_sel)
//    6 layer     fill, then cancel           18 w|h<<16 new document (white)
//    7 index     add layer (lay_new)         19 i       activate document i
//    8 index     delete layer (contract way) 20         hist_enforce
//    9 opacity   opacity of current layer    21         hist_mark_clean (as if saved)
//   10 w|h<<16   canvas size (top left)      22 n       rename current layer "Ebene n"
//   11 slot      remember the state hash     23 n       n pattern fills (small, moving)
//                                            24         clear the history message
//   25 / 26 / 27  timed undo / redo / hist_enforce -> probe t.us=microseconds
//   28 kb      budget in KB without enforcing it now
//   29         compound of two structure steps: add a layer on top, add another one

use alloc::format;
use alloc::string::String;

use gfx::{Surface, FMT_ARGB};
use ui::*;

use crate::app::constants::*;
use crate::document::model::{doc_active, doc_at, doc_count, doc_cur_layer, doc_mark_layers, doc_mark_px, doc_new, doc_resized, doc_set_active, doc_set_modified, Doc, DocId};
use crate::document::history::{hist_begin_compound, hist_can_redo, hist_can_undo, hist_clean, hist_count, hist_end_compound, hist_enforce, hist_entry, hist_mark_clean, hist_message, hist_message_clear, hist_pos, hist_push_doc, hist_push_layer_props, hist_push_layers, hist_push_sel, hist_px_begin, hist_px_cancel, hist_px_end, hist_px_touch, hist_redo, hist_release, hist_undo, HistEntry, HK_COMP, HK_PX};
use crate::document::history::memory::{hist_budget_set, hist_fail_after, hist_mem_disk, hist_spill_enable, TST_DISK, TST_RAW, TST_SOLID, TST_Z};
use crate::document::layers::{lay_new, lay_set_opacity, Layer};
use crate::app::Pn;

const TH_N: u32 = 30; // hooks
const TH_SLOTS: usize = 8;
const WINID_HTEST: u32 = 0x60FE; // probe window (dialog window id range, A2)

/// the state of hist_diag.s (its g_* globals)
#[derive(Default)]
pub(crate) struct HistDiag {
    slots: [u64; TH_SLOTS],
    rect: Shared<WinRect>,
    open: Shared<i32>,
    /// g_th_r: the clipped rectangle x0, y0, x1, y1
    r: [i32; 4],
    on: u32,
    p0: u32,
    p1: u32,
    col: u32,
    cmp: u32,
    us: u32,
}

/// hist_diag_cmd(id, arg) -> true
pub(crate) fn hist_diag_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    let doc = doc_active(&p.doc);
    let k = id.wrapping_sub(CMD_HIST_TEST0);
    if k >= TH_N {
        return true;
    }
    match k {
        0 => {
            p.hdiag.on = arg;
            ui.repass();
        }
        1 => p.hdiag.p0 = arg,
        2 => p.hdiag.p1 = arg,
        3 => p.hdiag.col = arg,
        4 => th_fill(p, doc, arg as i32, 0, 0),
        5 => th_fill(p, doc, -1, 1, arg as i32),
        6 => th_fill(p, doc, arg as i32, 2, 0),
        7 => th_add(p, doc, arg as i32),
        8 => th_del(p, doc, arg as i32),
        9 => th_opac(p, doc, arg as i32),
        10 => th_canvas(p, doc, arg),
        11 => {
            if let Some(doc) = doc {
                let h = hist_state_hash(p.doc.d(doc));
                p.hdiag.slots[arg as usize & (TH_SLOTS - 1)] = h;
            }
        }
        12 => {
            if let Some(doc) = doc {
                let h = hist_state_hash(p.doc.d(doc));
                p.hdiag.cmp = if p.hdiag.slots[arg as usize & (TH_SLOTS - 1)] == h { 2 } else { 1 }; // 1 diff, 2 ok
                ui.repass();
            }
        }
        13 => {
            hist_budget_set(p, (arg as u64) << 10);
            hist_enforce(p);
        }
        14 => hist_fail_after(&mut p.hm, arg),
        15 => hist_spill_enable(&mut p.hm, arg != 0),
        16 => {
            if let Some(doc) = doc {
                hist_begin_compound(p, doc);
                th_fill(p, Some(doc), -1, 1, 7);
                p.hdiag.p0 = p.hdiag.p0.wrapping_add(40); // second fill 40 px to the right
                p.hdiag.p1 = p.hdiag.p1.wrapping_add(40);
                th_fill(p, Some(doc), -1, 1, 9);
                p.hdiag.p0 = p.hdiag.p0.wrapping_sub(40);
                p.hdiag.p1 = p.hdiag.p1.wrapping_sub(40);
                hist_end_compound(p, doc, "Verbund", IC_SHAPES);
            }
        }
        17 => {
            if let Some(doc) = doc {
                hist_push_sel(p, doc, "Alles auswählen", IC_SELECT_ALL);
            }
        }
        18 => {
            doc_new(p, (arg & 0xFFFF) as i32, (arg >> 16) as i32, 0xFFFF_FFFF, 0);
        }
        19 => {
            if let Some(d) = doc_at(&p.doc, arg as i32) {
                doc_set_active(p, d);
            }
        }
        20 => hist_enforce(p),
        21 => {
            if let Some(doc) = doc {
                hist_mark_clean(p, doc);
                doc_set_modified(p, doc, false);
            }
        }
        22 => th_rename(p, doc, arg as i32),
        23 => th_many(p, doc, arg as i32),
        24 => {
            hist_message_clear(&mut p.hist);
            ui.repass();
        }
        29 => {
            if let Some(doc) = doc {
                hist_begin_compound(p, doc);
                let n = p.doc.d(doc).layers.len() as i32;
                th_add(p, Some(doc), n);
                let n = p.doc.d(doc).layers.len() as i32;
                th_add(p, Some(doc), n);
                hist_end_compound(p, doc, "Zwei Ebenen", IC_LAYERS);
            }
        }
        28 => hist_budget_set(p, (arg as u64) << 10),
        _ => {
            // 25 / 26 / 27: timed undo / redo / hist_enforce
            let t0 = plat::time_us();
            match (k, doc) {
                (25, Some(doc)) => {
                    hist_undo(p, doc);
                }
                (26, Some(doc)) => {
                    hist_redo(p, doc);
                }
                (25 | 26, None) => {}
                _ => hist_enforce(p),
            }
            p.hdiag.us = plat::time_us().wrapping_sub(t0) as u32;
            ui.repass();
        }
    }
    true
}

/// th_rect(doc, layer) -> true when the parameter rectangle clipped to the layer is not empty
/// (g_th_r = x0, y0, x1, y1)
fn th_rect(p: &mut Pn, doc: DocId, layer: i32) -> bool {
    let l = &p.doc.d(doc).layers[layer as usize];
    let t = &mut p.hdiag;
    let (x0, y0) = ((t.p0 & 0xFFFF) as i32, (t.p0 >> 16) as i32);
    let (x1, y1) = (((t.p1 & 0xFFFF) as i32).min(l.sf.w), ((t.p1 >> 16) as i32).min(l.sf.h));
    t.r = [x0, y0, x1, y1];
    x0 < x1 && y0 < y1
}

/// th_layer(doc, index) -> clamped layer index (-1 = current), -1 without layers
fn th_layer(d: &Doc, index: i32) -> i32 {
    let n = d.layers.len() as i32;
    if n <= 0 {
        return -1;
    }
    let mut i = if index < 0 { d.cur } else { index };
    if i >= n {
        i = n - 1;
    }
    i.max(0)
}

/// th_fill(doc, layer, mode, seed) -- fill the parameter rectangle of a layer through the pixel
/// history: mode 0 colour ("Farbeimer"), 1 pattern ("Farbverlauf"), 2 colour + cancel
fn th_fill(p: &mut Pn, doc: Option<DocId>, layer: i32, mode: i32, seed: i32) {
    let Some(doc) = doc else { return };
    let li = th_layer(p.doc.d(doc), layer);
    if li < 0 || !th_rect(p, doc, li) {
        return;
    }
    if hist_px_begin(p, doc, li) != 0 {
        return;
    }
    let r = p.hdiag.r;
    hist_px_touch(p, doc, r[0], r[1], r[2], r[3]);
    // write the pixels
    let col = p.hdiag.col;
    let l = &mut p.doc.dm(doc).layers[li as usize];
    let stride = l.sf.stride as usize;
    for y in r[1]..r[3] {
        let row = &mut l.sf.px[y as usize * stride..];
        for x in r[0]..r[2] {
            let mut c = col;
            if mode == 1 {
                // pattern: B = 3x + 17s, G = 5y + 29s, R = (x ^ y) + s, A = 255
                let b = (x.wrapping_mul(3).wrapping_add(seed.wrapping_mul(17))) as u32 & 255;
                let g = (y.wrapping_mul(5).wrapping_add(seed.wrapping_mul(29))) as u32 & 255;
                let rr = ((x ^ y).wrapping_add(seed)) as u32 & 255;
                c = b | g << 8 | rr << 16 | 0xFF00_0000;
            }
            row[x as usize * 4..x as usize * 4 + 4].copy_from_slice(&c.to_le_bytes());
        }
    }
    doc_mark_px(p, doc, li, r[0], r[1], r[2], r[3]);
    if mode == 2 {
        hist_px_cancel(p, doc);
    } else if mode == 1 {
        hist_px_end(p, doc, "Farbverlauf", IC_CONTRAST);
    } else {
        hist_px_end(p, doc, "Farbeimer", IC_DROP);
    }
}

/// th_many(doc, n) -- n pattern fills of 16 x 16 pixels, 16 per row (many entries)
fn th_many(p: &mut Pn, doc: Option<DocId>, n: i32) {
    if doc.is_none() {
        return;
    }
    let (s0, s1) = (p.hdiag.p0, p.hdiag.p1);
    for i in 0..n.max(0) {
        let x = ((i & 15) << 4) as u32;
        let y = ((i as u32) >> 4) << 4;
        p.hdiag.p0 = y << 16 | x;
        p.hdiag.p1 = p.hdiag.p0.wrapping_add(16 | 16 << 16);
        th_fill(p, doc, -1, 1, i);
    }
    p.hdiag.p0 = s0;
    p.hdiag.p1 = s1;
}

/// th_add(doc, index) -- "Neue Ebene" at index (lay_new, then it becomes current)
fn th_add(p: &mut Pn, doc: Option<DocId>, index: i32) {
    let Some(doc) = doc else { return };
    if !hist_push_layers(p, doc, "Neue Ebene", IC_ADD) {
        return;
    }
    if lay_new(p, doc, index, None).is_none() {
        return;
    }
    let d = p.doc.dm(doc);
    let n = d.layers.len() as i32 - 1;
    d.cur = if index > n { n } else { index };
    doc_mark_layers(p, doc);
}

/// th_del(doc, index) -- "Ebene löschen" the way pn_hist.inc asks for: remove the record from
/// the array, hist_release, lay_free_rec
fn th_del(p: &mut Pn, doc: Option<DocId>, index: i32) {
    let Some(doc) = doc else { return };
    let n = p.doc.d(doc).layers.len() as i32;
    if n < 2 || index as u32 >= n as u32 {
        return;
    }
    if !hist_push_layers(p, doc, "Ebene löschen", IC_DELETE) {
        return;
    }
    let d = p.doc.dm(doc);
    let rec = d.layers.remove(index as usize);
    d.cur = (index - 1).max(0);
    hist_release(p, doc, rec);
    doc_mark_layers(p, doc);
}

/// th_opac(doc, v) -- opacity of the current layer (layer properties step)
fn th_opac(p: &mut Pn, doc: Option<DocId>, v: i32) {
    let Some(doc) = doc else { return };
    let cur = p.doc.d(doc).cur;
    if !hist_push_layer_props(p, doc, cur, "Ebeneneigenschaften", IC_EDIT) {
        return;
    }
    let cur = p.doc.d(doc).cur;
    lay_set_opacity(p, doc, cur, v);
    doc_mark_layers(p, doc);
}

/// th_rename(doc, n) -- rename the current layer to "Ebene n" (contract way: the old name goes
/// through hist_release)
fn th_rename(p: &mut Pn, doc: Option<DocId>, n: i32) {
    let Some(doc) = doc else { return };
    if p.doc.d(doc).layers.is_empty() {
        return;
    }
    let cur = p.doc.d(doc).cur;
    if !hist_push_layer_props(p, doc, cur, "Ebeneneigenschaften", IC_EDIT) {
        return;
    }
    let d = p.doc.dm(doc);
    let cur = d.cur as usize;
    let old = core::mem::replace(&mut d.layers[cur].name, format!("Ebene {n}"));
    hist_release(p, doc, Layer { name: old, ..Layer::default() });
    doc_mark_layers(p, doc);
}

/// th_canvas(doc, w | h << 16) -- "Leinwandgröße": every layer gets a new surface, the old
/// pixels stay at the top left (hist_push_doc keeps the old surfaces)
fn th_canvas(p: &mut Pn, doc: Option<DocId>, arg: u32) {
    let Some(doc) = doc else { return };
    let (w, h) = ((arg & 0xFFFF) as i32, (arg >> 16) as i32);
    if w == 0 || h == 0 {
        return;
    }
    if !hist_push_doc(p, doc, "Leinwandgröße", IC_RESIZE) {
        return;
    }
    let n = p.doc.d(doc).layers.len();
    for i in 0..n {
        let d = p.doc.dm(doc);
        let l = &mut d.layers[i];
        let mut sf = Surface::new(w, h, FMT_ARGB);
        if sf.px.is_empty() {
            break;
        }
        sf.opac = l.sf.opac;
        sf.blend = l.sf.blend;
        // copy the overlap
        let cw = l.sf.w.min(w) as usize * 4;
        let ch = l.sf.h.min(h);
        let (ds, ss) = (sf.stride as usize, l.sf.stride as usize);
        for y in 0..ch.max(0) as usize {
            sf.px[y * ds..y * ds + cw].copy_from_slice(&l.sf.px[y * ss..y * ss + cw]);
        }
        let old = core::mem::replace(&mut l.sf, sf);
        hist_release(p, doc, Layer { sf: old, ..Layer::default() });
    }
    let d = p.doc.dm(doc);
    d.w = w;
    d.h = h;
    doc_resized(p, doc);
}

/// hash_fnv64(data, seed) -- FNV-1a 64 (seed 0 = the offset basis)
fn fnv(d: &[u8], seed: u64) -> u64 {
    let mut h = if seed == 0 { 0xCBF2_9CE4_8422_2325 } else { seed };
    for &b in d {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01B3);
    }
    h
}

/// hist_state_hash(doc) -> FNV-1a of W, H, NLAYERS, CUR and per layer: opacity, blend,
/// visibility, uid, name, the pixel rows
pub(crate) fn hist_state_hash(d: &Doc) -> u64 {
    let mut hd = [0u8; 16];
    hd[0..4].copy_from_slice(&d.w.to_le_bytes());
    hd[4..8].copy_from_slice(&d.h.to_le_bytes());
    hd[8..12].copy_from_slice(&(d.layers.len() as i32).to_le_bytes());
    hd[12..16].copy_from_slice(&d.cur.to_le_bytes());
    let mut h = fnv(&hd, 0);
    for l in &d.layers {
        let mut r = [0u8; 8];
        r[0] = l.sf.opac;
        r[1] = l.sf.blend;
        r[2..4].copy_from_slice(&((l.flags & LYF_VISIBLE as u32) as u16).to_le_bytes());
        r[4..8].copy_from_slice(&(l.uid as u32).to_le_bytes());
        h = fnv(&r, h);
        if !l.name.is_empty() {
            h = fnv(l.name.as_bytes(), h);
        }
        let (n, s) = (l.sf.w.max(0) as usize * 4, l.sf.stride as usize);
        for y in 0..l.sf.h.max(0) as usize {
            h = fnv(&l.sf.px[y * s..y * s + n], h);
        }
    }
    h
}

// ------------------------------------------------------------------ probes

/// th_count(e) -- add the tile states of e (and its children) to cnt[TST_*]
fn th_count(e: &HistEntry, cnt: &mut [u32; 4]) {
    match e.kind {
        HK_COMP => {
            for k in &e.kids {
                th_count(k, cnt);
            }
        }
        HK_PX => {
            for t in &e.tiles {
                cnt[(t.st & 3) as usize] += 1;
            }
        }
        _ => {}
    }
}

/// hist_diag_frame() -- test mode: the probe window (from hist_frame)
pub(crate) fn hist_diag_frame(p: &mut Pn, ui: &mut Ui) {
    if p.hdiag.on == 0 {
        return;
    }
    let t = &mut p.hdiag;
    t.open.set(1);
    let mut r = t.rect.get();
    if r.flags & WRF_POS == 0 {
        r.x = 300;
        r.y = 160;
        r.w = 220;
        r.flags |= WRF_POS;
        t.rect.set(r);
    }
    let (open, rect) = (t.open.clone(), t.rect.clone());
    if ui.window_begin(WINID_HTEST, crate::preferences::localization::tr("Verlauf-Test"), Some(&open), WF_TOOL | WF_AUTOHEIGHT | WF_NOACTIVATE | WF_NOCLOSE, Some(&rect)) {
        ui.box_begin(0, BOX_V | PAD_8, 0);
        let probe = |ui: &mut Ui, s: String| ui.text(TS_CAPTION, &s);
        if let Some(doc) = doc_active(&p.doc) {
            let d = p.doc.d(doc);
            probe(ui, format!("t.n={}", hist_count(d)));
            probe(ui, format!("t.pos={}", hist_pos(d)));
            probe(ui, format!("t.clean={}", hist_clean(d)));
            probe(ui, format!("t.layers={}", d.layers.len()));
            probe(ui, format!("t.cur={}", d.cur));
            probe(ui, format!("t.size={}x{}", d.w, d.h));
            if let Some(l) = doc_cur_layer(d) {
                probe(ui, format!("t.opac={}", l.sf.opac));
                probe(ui, format!("t.name={}", l.name));
            }
            probe(ui, format!("t.mod={}", d.flags & DF_MODIFIED as u32));
            probe(ui, format!("t.undo={} redo={}", hist_can_undo(d) as i32, hist_can_redo(d) as i32));
            // tile states of every entry of this document
            let mut cnt = [0u32; 4];
            for i in 0..hist_count(d) {
                if let Some(e) = hist_entry(d, i) {
                    th_count(e, &mut cnt);
                }
            }
            probe(ui, format!("t.raw={}", cnt[TST_RAW as usize]));
            probe(ui, format!("t.solid={}", cnt[TST_SOLID as usize]));
            probe(ui, format!("t.z={}", cnt[TST_Z as usize]));
            probe(ui, format!("t.disk={}", cnt[TST_DISK as usize]));
        }
        probe(ui, format!("t.docs={}", doc_count(&p.doc)));
        let c = match p.hdiag.cmp {
            1 => "diff",
            2 => "ok",
            _ => "none",
        };
        probe(ui, format!("t.cmp={c}"));
        probe(ui, format!("t.msg={}", hist_message(&p.hist).is_some() as i32));
        probe(ui, format!("t.file={}", (hist_mem_disk(&p.hm) != 0) as i32));
        probe(ui, format!("t.us={}", p.hdiag.us as i32));
        ui.box_end();
    }
    ui.window_end();
}
