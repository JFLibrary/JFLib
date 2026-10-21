// colormgmt -- PaintCraft: colour management (agent I1, plan M8, spec 2.12 Farbmanagement)
//
// Bild > Farbprofil... (CMD_COLOR_PROFILE): modal dialog "Farbprofil" like paint.net 5.1:
// Aktuell: (the document's profile) / Neu: (built-in sRGB, A98 RGB, Display P3,
// ROMM RGB, Graustufen; the monitor profile; an imported file) with Import... / Export...,
// Priorität (rendering intent) + Tiefenkompensierung, Konvertieren / Zuweisen / Abbrechen.
//   Zuweisen     only DOC_ICC changes: one history step "Farbprofil zuweisen" (hist_push_sel
//                as a carrier -- the selection stays as it is)
//   Konvertieren every layer converted on the worker pool (icc_xform_row), installed like
//                imgops.s does (hist_push_doc + hist_release): one step "Farbprofil
//                konvertieren"; then DOC_ICC = the new profile
// Undo / redo of DOC_ICC: the history keeps no DOC_ICC, so every step made here is recorded
// {document, token (DOC_CSPACE), entry HE_SEQ, old profile, new profile} and cm_sync sets
// DOC_ICC to the state of the current history position every frame (records whose entries
// are gone are dropped; DOC_CSPACE = this module's per-document token).
// Display: view.s passes every canvas row through cm_view_row: document profile (none =
// sRGB) -> monitor profile (plat_monitor_icc of the main window; queried again when
// plat_monitor_key changes, i.e. the window moved to another monitor; none / Wayland = sRGB)
// while colour management is on (default on, like paint.net 5.1).  Transforms are cached per
// (document profile, monitor profile) by CRC-32; the composite itself is never changed.
//
// Module contract (imgops.s forwards, block "I1"):  cm_states()  cm_frame()  cm_cmd(id, arg)
// view.s (block "I1"): cm_view_row(dst, n) -- worker threads, XRGB row in place
// Settings (S1): cm_enabled() -> eax 1 on    cm_set_enabled(on)
// Test hooks (cmd ID ARG):
//   290 id          monitor profile = built-in id 0..4; 255 = none (sRGB); 65535 = real query
//   291 on          cm_set_enabled(on)
//   292 i | b << 8  dialog intent i (ICC_*), b = Tiefenkompensierung
//   293 x | y << 16 display probe of composite pixel (x, y) -> #0x82301 text "AARRGGBB"
//   294 id          built-in id as the dialog's "imported" profile
//   280 / 281 row   Zuweisen / Konvertieren with list row (0..4 built-in, then the monitor
//                   profile when there is one, then the imported one)
// Probes (hidden K_TEXT): #0x82300 text = document profile description ("" = none), value =
//   DOC_ICC_LEN · #0x82301 text = display probe, value = colour management on · #0x82302
//   text = monitor profile description, value = display transform kind (-1 none / identity,
//   1 matrix, 2 3D LUT) · #0x82303 text = dialog error ("" none), value = history records
// Details: apps/PaintCraft/docs/COLORPROFILE.md
//                                              [apps/PaintCraft/src/colormgmt.s]
//
// Rust model: Cm is read by the canvas renderer on worker threads (cm_view_row), so it holds
// only Sync data; the dialog's Shared flags / placement (kept by the window records of the
// UI) are in CmDlg (Pn::cmdlg).  The display transform is a slot of the transform cache
// (cm_cache); its handle is the slot's IccXform.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::{Surface, FMT_ARGB};
use img_core::icc_xform::{icc_builtin, icc_describe, IccXform, ICC_ABSOLUTE, ICC_BI_COUNT, ICC_BI_SRGB, ICC_PERCEPTUAL, ICF_BPC, ICX_IDENTITY};
use rt::sync::SpinLock;
use ui::*;

use crate::app::constants::*;
use crate::document::model::{doc_active, doc_index, doc_mark_layers, doc_mark_px, doc_set_modified, DocId};
use crate::document::history::{hist_count, hist_entry, hist_pos, hist_push_doc, hist_push_sel, hist_release};
use crate::document::layers::Layer;
use crate::app::formatting::fmt_cap;
use crate::app::commands::pn_cmd_set;
use crate::app::Pn;

const ROW_MON: u32 = 16; // row kinds (rows): 0..4 built-in id
const ROW_IMP: u32 = 17;
const DESC_CAP: usize = 128;
const PATH_CAP: usize = 1024;
const LIST_ROOM: i32 = 60; // DIP below the widgets for the combo lists
const BAND_ROWS: i32 = 64; // rows per convert work item
const CR_MAX: usize = 256; // profile history records
const CC_N: usize = 4; // cached display transforms
const FORCE_REAL: u32 = 0xFFFF;
const ID_CM_SEL: u32 = ID_DLG_PROFILE;
const ID_CM_INTENT: u32 = ID_DLG_PROFILE + 1;
const ID_CM_BPC: u32 = ID_DLG_PROFILE + 2;
const ID_CM_IMPORT: u32 = ID_DLG_PROFILE + 3;
const ID_CM_EXPORT: u32 = ID_DLG_PROFILE + 4;
const ID_CM_PROBE: u32 = ID_ICC_FIRST;

const S_TITLE: &str = "Farbprofil";
const S_CUR_NONE: &str = "sRGB (kein Profil eingebettet)";
const S_CUR_NONAME: &str = "Unbenanntes Profil";
const S_IMP_NONAME: &str = "Importiertes Profil";
const S_H_ASSIGN: &str = "Farbprofil zuweisen";
const S_H_CONVERT: &str = "In Farbprofil umrechnen";
const S_ERR_CONV: &str = "In dieses Farbprofil lässt sich nicht umrechnen.";
const S_ERR_FILE: &str = "Die Datei enthält kein brauchbares Farbprofil.";
const S_ERR_WRITE: &str = "Das Farbprofil ließ sich nicht speichern.";
const S_EXP_NAME: &str = "Farbprofil.icc";
static S_EXTS: [&str; 1] = ["icc;icm=Farbprofil"];
static BI_NAMES: [&str; 5] = ["sRGB", "A98 RGB", "Display P3", "ROMM RGB", "Graustufen (Gamma 2,2)"];
static INTENT_LIST: [&str; 4] = ["Perzeptiv", "Relativ farbmetrisch", "Sättigung", "Absolut farbmetrisch"];

/// a profile history record (CR_*)
struct CrRec {
    doc: DocId,
    token: u32,
    /// 0 new / 1 seen / 2 dead (cm_sync pass)
    flag: u32,
    seq: u64,
    old: Vec<u8>,
    new: Vec<u8>,
}

/// a cached display transform (CC_*)
#[derive(Default)]
struct CcEntry {
    scrc: u32,
    slen: u32,
    dcrc: u32,
    dlen: u32,
    h: Option<Result<IccXform, i32>>,
    age: u32,
    used: bool,
    /// (Rust) build serial: identifies the handle (the asm compares handle pointers)
    serial: u32,
}

/// the state of colormgmt.s that the canvas renderer reads (Sync; see the header)
pub(crate) struct Cm {
    enabled: bool,
    force: u32,
    force_done: u32,
    bpc: i32,
    /// g_cm_view: the display transform = cache slot (slot, serial)
    view: Option<(usize, u32)>,
    /// g_vk: its key: doc, profile (address, length), gen, monitor gen, enabled
    vk: (Option<DocId>, usize, u32, u32, u32, bool),
    /// g_cm_mon: the monitor profile (empty = sRGB)
    mon: Vec<u8>,
    mon_key: u64,
    mon_valid: bool,
    mon_gen: u32,
    mon_desc: String,
    /// g_cm_imp: the imported profile (empty = none)
    imp: Vec<u8>,
    imp_desc: String,
    /// the dialog's document, error message
    doc: Option<DocId>,
    err: Option<&'static str>,
    icc_gen: u32,
    token: u32,
    pending: bool,
    sel: i32,
    intent: i32,
    /// g_cm_rows / g_cm_list (row kinds, labels)
    rows: Vec<u32>,
    list: Vec<String>,
    fd_mode: u32,
    fd_path: String,
    cc: [CcEntry; CC_N],
    cc_tick: u32,
    cc_serial: u32,
    cr: Vec<CrRec>,
    probe: String,
}

impl Default for Cm {
    fn default() -> Cm {
        Cm {
            enabled: true, // paint.net 5.1: the canvas is colour managed
            force: FORCE_REAL,
            force_done: u32::MAX,
            bpc: 1,
            view: None,
            vk: (None, 0, 0, 0, 0, false),
            mon: Vec::new(),
            mon_key: 0,
            mon_valid: false,
            mon_gen: 0,
            mon_desc: String::new(),
            imp: Vec::new(),
            imp_desc: String::new(),
            doc: None,
            err: None,
            icc_gen: 0,
            token: 0,
            pending: false,
            sel: 0,
            intent: 0,
            rows: Vec::new(),
            list: Vec::new(),
            fd_mode: 0,
            fd_path: String::new(),
            cc: Default::default(),
            cc_tick: 0,
            cc_serial: 0,
            cr: Vec::new(),
            probe: String::new(),
        }
    }
}

/// the dialog state the UI keeps references to (g_cm_open, g_cm_rect, g_cm_fd_open)
pub(crate) struct CmDlg {
    open: Shared<i32>,
    rect: Shared<WinRect>,
    fd_open: Shared<i32>,
}

impl Default for CmDlg {
    fn default() -> CmDlg {
        CmDlg { open: Shared::new(0), rect: Shared::new(WinRect::new(0, 0, 440, 0, 0)), fd_open: Shared::new(0) }
    }
}

// ================================================================== small helpers

/// cm_enabled() -> eax 1 when colour management of the canvas is on
pub(crate) fn cm_enabled(cm: &Cm) -> bool {
    cm.enabled
}

/// cm_set_enabled(on) -- switch colour management of the canvas (redrawn with the next frame)
pub(crate) fn cm_set_enabled(p: &mut Pn, on: bool) {
    p.cm.enabled = on;
    p.repass();
}

/// cm_view_row(dst, n) -- view.rs: n XRGB pixels of a canvas row, document -> monitor profile
/// in place (nothing when off / identity).  Worker threads (render time).
pub(crate) fn cm_view_row(cm: &Cm, dst: &mut [u8], n: i32) {
    if let Some(x) = cm_view_xf(cm) {
        let n = (n.max(0) as usize * 4).min(dst.len());
        x.row_in_place(&mut dst[..n]);
    }
}

// the display transform (g_cm_view)
fn cm_view_xf(cm: &Cm) -> Option<&IccXform> {
    let (i, _) = cm.view?;
    match &cm.cc[i].h {
        Some(Ok(x)) => Some(x),
        _ => None,
    }
}

// opt(p) -- a profile slice as icc_xform_build's argument (empty = the built-in sRGB)
fn opt(p: &[u8]) -> Option<&[u8]> {
    if p.is_empty() { None } else { Some(p) }
}

// cm_set_doc_icc(doc, p) -- DOC_ICC = a copy of p (empty = none)
fn cm_set_doc_icc(p: &mut Pn, doc: DocId, icc: &[u8]) {
    p.doc.dm(doc).icc = Vec::from(icc);
    p.cm.icc_gen = p.cm.icc_gen.wrapping_add(1);
}

// cm_sem(id, label, value) -- a hidden K_TEXT record (empty rectangle)
fn cm_sem(ui: &mut Ui, id: u32, label: &str, value: i32) {
    ui.last_reset();
    ui.sem(K_TEXT, id, Some(label), value);
}

// describe(p) -> the profile's description ("" = none); icc_describe into DESC_CAP bytes
fn describe(icc: &[u8]) -> String {
    let mut buf = [0u8; DESC_CAP];
    let n = icc_describe(icc, &mut buf);
    String::from(core::str::from_utf8(&buf[..n.min(DESC_CAP)]).unwrap_or(""))
}

// ================================================================== profile list

// cm_rows() -- rows / list: built-ins, monitor profile, imported
fn cm_rows(cm: &mut Cm) {
    cm.rows.clear();
    cm.list.clear();
    for i in 0..ICC_BI_COUNT {
        cm.rows.push(i);
        cm.list.push(String::from(BI_NAMES[i as usize]));
    }
    if !cm.mon.is_empty() {
        cm.rows.push(ROW_MON);
        cm.list.push(fmt_cap(format!("Monitor: {}", cm.mon_desc), DESC_CAP + 16));
    }
    if !cm.imp.is_empty() {
        cm.rows.push(ROW_IMP);
        cm.list.push(fmt_cap(format!("Importiert: {}", cm.imp_desc), DESC_CAP + 16));
    }
}

// cm_row_profile(row) -> the list row's profile bytes | None
fn cm_row_profile(cm: &mut Cm, row: u32) -> Option<Vec<u8>> {
    cm_rows(cm);
    let k = *cm.rows.get(row as usize)?;
    match k {
        ROW_MON => opt(&cm.mon).map(Vec::from),
        ROW_IMP => opt(&cm.imp).map(Vec::from),
        _ => icc_builtin(k).map(Vec::from),
    }
}

// ================================================================== history of DOC_ICC

// cm_record(doc, old, new) -- remember the step just pushed (entry pos - 1)
fn cm_record(p: &mut Pn, doc: DocId, old: &[u8], new: &[u8]) {
    if p.cm.cr.len() >= CR_MAX {
        p.cm.cr.remove(0); // full: drop the oldest record
    }
    let d = p.doc.dm(doc);
    if d.cspace == 0 {
        p.cm.token = p.cm.token.wrapping_add(1);
        d.cspace = p.cm.token;
    }
    let pos = hist_pos(d);
    if pos == 0 {
        return;
    }
    let Some(e) = hist_entry(d, pos - 1) else { return };
    let rec = CrRec { doc, token: d.cspace, flag: 0, seq: e.seq, old: Vec::from(old), new: Vec::from(new) };
    p.cm.cr.push(rec);
}

// cm_sync_doc(doc, token) -- DOC_ICC = the state of the history position (records of doc)
fn cm_sync_doc(p: &mut Pn, doc: DocId, token: u32) {
    let live = doc_index(&p.doc, doc) >= 0 && p.doc.d(doc).cspace == token;
    if !live {
        // the document is gone (or another token): all its records die
        for r in p.cm.cr.iter_mut().filter(|r| r.doc == doc) {
            r.flag = 2;
        }
        return;
    }
    let d = p.doc.d(doc);
    let n = hist_count(d);
    let pos = hist_pos(d);
    let mut last: Option<usize> = None; // last applied record
    let mut first: Option<usize> = None; // first unapplied record
    for (i, r) in p.cm.cr.iter_mut().enumerate() {
        if r.doc != doc {
            continue;
        }
        r.flag = 1;
        if r.token != token {
            r.flag = 2;
            continue;
        }
        let found = (0..n).find(|&k| hist_entry(d, k).is_some_and(|e| e.seq == r.seq));
        match found {
            None => r.flag = 2,
            Some(k) if k < pos => last = Some(i),
            Some(_) => {
                if first.is_none() {
                    first = Some(i);
                }
            }
        }
    }
    let want = match (last, first) {
        (Some(i), _) => p.cm.cr[i].new.clone(),
        (None, Some(i)) => p.cm.cr[i].old.clone(),
        _ => return,
    };
    if p.doc.d(doc).icc == want {
        return;
    }
    cm_set_doc_icc(p, doc, &want);
    p.repass();
}

// cm_sync() -- every frame: every document with records follows its history position;
// records of closed documents / dropped entries are freed
fn cm_sync(p: &mut Pn) {
    if p.cm.cr.is_empty() {
        return;
    }
    for r in p.cm.cr.iter_mut() {
        r.flag = 0;
    }
    let mut i = 0;
    while i < p.cm.cr.len() {
        if p.cm.cr[i].flag == 0 {
            let (doc, token) = (p.cm.cr[i].doc, p.cm.cr[i].token);
            cm_sync_doc(p, doc, token);
        }
        i += 1;
    }
    p.cm.cr.retain(|r| r.flag != 2);
}

// ================================================================== assign / convert

// cm_assign(doc, icc) -> true -- "Farbprofil zuweisen": DOC_ICC only, one history step
fn cm_assign(p: &mut Pn, doc: DocId, icc: &[u8]) -> bool {
    if !hist_push_sel(p, doc, S_H_ASSIGN, IC_COLOR) {
        return false;
    }
    let old = p.doc.d(doc).icc.clone();
    cm_record(p, doc, &old, icc);
    cm_set_doc_icc(p, doc, icc);
    doc_set_modified(p, doc, true);
    true
}

// cm_convert(doc, icc) -> true / false -- "In Farbprofil umrechnen": every layer from DOC_ICC
// to icc (dialog intent / BPC) on the worker pool, one history step, then DOC_ICC = icc
fn cm_convert(p: &mut Pn, doc: DocId, icc: &[u8]) -> bool {
    let flags = if p.cm.bpc != 0 { ICF_BPC } else { 0 };
    let d = p.doc.d(doc);
    let Ok(xf) = IccXform::build(opt(&d.icc), opt(icc), p.cm.intent as u32, flags) else { return false };
    // new surfaces (size, opacity, blend mode of the layers)
    let (w, h) = (d.w, d.h);
    let mut nsf: Vec<Surface> = Vec::with_capacity(d.layers.len());
    for l in &d.layers {
        let Some(mut sf) = crate::document::selection::surf_try(w, h) else { return false };
        sf.fmt = FMT_ARGB;
        sf.opac = l.sf.opac;
        sf.blend = l.sf.blend;
        nsf.push(sf);
    }
    // the bands: (layer, y0, y1, rows y0..y1 of the new surface)
    {
        let mut bands: Vec<SpinLock<(usize, i32, i32, &mut [u8])>> = Vec::new();
        let mut strides: Vec<usize> = Vec::new();
        for (li, sf) in nsf.iter_mut().enumerate() {
            let stride = sf.stride as usize;
            strides.push(stride);
            let mut rest: &mut [u8] = &mut sf.px[..];
            let mut y = 0;
            while y < h {
                let y1 = (y + BAND_ROWS).min(h);
                let take = ((y1 - y) as usize * stride).min(rest.len());
                let (a, r) = core::mem::take(&mut rest).split_at_mut(take);
                bands.push(SpinLock::new((li, y, y1, a)));
                rest = r;
                y = y1;
            }
        }
        let xf = &xf;
        let item = |i: usize, _: &plat::JobCtl| {
            let mut g = bands[i].lock();
            let (li, y0, y1, ref mut rows) = *g;
            let src = &d.layers[li].sf;
            let (ss, ds) = (src.stride as usize, strides[li]);
            let nb = w as usize * 4;
            for y in y0..y1 {
                let so = y as usize * ss;
                let o = (y - y0) as usize * ds;
                if so + nb > src.px.len() || o + nb > rows.len() {
                    break;
                }
                xf.row(&src.px[so..so + nb], &mut rows[o..o + nb]);
            }
        };
        let _ = plat::parallel_for(bands.len(), &item);
    }
    drop(xf);
    // one history step, then the surfaces are installed (old ones to the history)
    if !hist_push_doc(p, doc, S_H_CONVERT, IC_COLOR) {
        return false;
    }
    let old = p.doc.d(doc).icc.clone();
    cm_record(p, doc, &old, icc);
    for (i, sf) in nsf.into_iter().enumerate() {
        let l = &mut p.doc.dm(doc).layers[i];
        let old = core::mem::replace(&mut l.sf, sf);
        l.ver = l.ver.wrapping_add(1);
        hist_release(p, doc, Layer { sf: old, ..Layer::default() });
    }
    cm_set_doc_icc(p, doc, icc);
    doc_mark_layers(p, doc);
    doc_mark_px(p, doc, -1, 0, 0, w, h);
    p.repass();
    true
}

// cm_apply(doc, row, convert) -- Zuweisen / Konvertieren with a list row; failure: the
// dialog comes back with the paint.net message
fn cm_apply(p: &mut Pn, doc: Option<DocId>, row: u32, convert: bool) {
    let Some(doc) = doc else { return };
    p.cm.err = None;
    let ok = match cm_row_profile(&mut p.cm, row) {
        None => false,
        Some(icc) if convert => cm_convert(p, doc, &icc),
        Some(icc) => {
            // the profile must be usable (like paint.net: ColorContext from it)
            IccXform::build(Some(&icc), None, 0, 0).is_ok() && cm_assign(p, doc, &icc)
        }
    };
    if !ok {
        p.cm.err = Some(S_ERR_CONV);
        p.cm.doc = Some(doc);
        p.cmdlg.open.set(1);
        p.cm.pending = true;
        p.repass();
    }
}

// ================================================================== display transform

// cm_mon_update() -- the monitor profile (test override / plat_monitor_icc of window 0)
fn cm_mon_update(cm: &mut Cm) {
    if cm.force != FORCE_REAL {
        if cm.force == cm.force_done {
            return;
        }
        cm.force_done = cm.force;
        cm.mon_valid = false; // query again after the override
        cm.mon.clear();
        if cm.force < ICC_BI_COUNT {
            cm.mon = icc_builtin(cm.force).map(Vec::from).unwrap_or_default();
        }
    } else {
        cm.force_done = FORCE_REAL;
        let key = plat::monitor_key(0);
        if cm.mon_valid && key == cm.mon_key {
            return;
        }
        cm.mon_key = key;
        cm.mon_valid = true;
        cm.mon = plat::monitor_icc(0).unwrap_or_default();
    }
    cm.mon_desc = if cm.mon.is_empty() { String::new() } else { describe(&cm.mon) };
    cm.mon_gen = cm.mon_gen.wrapping_add(1);
}

// cm_crc(p) -> CRC-32 (0 for no profile)
fn cm_crc(icc: &[u8]) -> u32 {
    if icc.is_empty() { 0 } else { img_core::checksum::crc32(0, icc) }
}

// cm_cache(src, dst) -> the cache slot of the transform (its handle may have failed) --
// cached per profile pair
fn cm_cache(cm: &mut Cm, src: &[u8], dst: &[u8]) -> usize {
    let (scrc, slen) = (cm_crc(src), src.len() as u32);
    let (dcrc, dlen) = (cm_crc(dst), dst.len() as u32);
    cm.cc_tick = cm.cc_tick.wrapping_add(1);
    let mut oldest = 0;
    for i in 0..CC_N {
        let e = &cm.cc[i];
        if !e.used {
            oldest = i; // an empty slot wins
            break;
        }
        if e.scrc == scrc && e.slen == slen && e.dcrc == dcrc && e.dlen == dlen {
            cm.cc[i].age = cm.cc_tick;
            return i;
        }
        if cm.cc[oldest].used && e.age < cm.cc[oldest].age {
            oldest = i;
        }
    }
    cm.cc_serial = cm.cc_serial.wrapping_add(1);
    let e = &mut cm.cc[oldest];
    e.h = Some(IccXform::build(opt(src), opt(dst), ICC_PERCEPTUAL, 0));
    e.scrc = scrc;
    e.slen = slen;
    e.dcrc = dcrc;
    e.dlen = dlen;
    e.age = cm.cc_tick;
    e.used = true;
    e.serial = cm.cc_serial;
    oldest
}

// cm_view_update() -- the display transform for the active document (redraw when it changes)
fn cm_view_update(p: &mut Pn) {
    cm_mon_update(&mut p.cm);
    let doc = doc_active(&p.doc);
    let icc: &[u8] = match doc {
        Some(d) => &p.doc.d(d).icc,
        None => &[],
    };
    let cm = &mut p.cm;
    let key = (doc, if icc.is_empty() { 0 } else { icc.as_ptr() as usize }, icc.len() as u32, cm.icc_gen, cm.mon_gen, cm.enabled);
    if cm.vk == key {
        return;
    }
    cm.vk = key;
    let mut new = None;
    if doc.is_some() && cm.enabled {
        let mon = core::mem::take(&mut cm.mon);
        let slot = cm_cache(cm, icc, &mon);
        cm.mon = mon;
        if let Some(Ok(x)) = &cm.cc[slot].h {
            if x.kind() != ICX_IDENTITY {
                new = Some((slot, cm.cc[slot].serial));
            }
        }
    }
    if new != cm.view {
        cm.view = new;
        crate::workspace::view::view_invalidate(p);
    }
}

// ================================================================== dialog

// cm_open(doc) -- show the Farbprofil dialog
fn cm_open(p: &mut Pn, doc: Option<DocId>) {
    let Some(doc) = doc else { return };
    p.cm.doc = Some(doc);
    p.cm.err = None;
    p.cmdlg.open.set(1);
    p.cm.pending = true;
    p.repass();
}

// cm_dlg_frame() -- the dialog window
fn cm_dlg_frame(p: &mut Pn, ui: &mut Ui) {
    if p.cm.pending && p.cmdlg.open.get() == 0 {
        p.cm.pending = false; // Esc / close button
    }
    if p.cmdlg.open.get() == 0 {
        return;
    }
    let Some(doc) = p.cm.doc.filter(|&d| doc_index(&p.doc, d) >= 0) else {
        p.cmdlg.open.set(0); // the image was closed
        p.cm.pending = false;
        return;
    };
    cm_rows(&mut p.cm);
    if p.cm.sel as u32 >= p.cm.rows.len() as u32 {
        p.cm.sel = 0;
    }
    let open = p.cmdlg.open.clone();
    let rect = p.cmdlg.rect.clone();
    if ui.window_begin(WINID_DLG_PROFILE, S_TITLE, Some(&open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&rect)) {
        // Aktuell:
        ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Aktuell:"));
        let icc = &p.doc.d(doc).icc;
        let cur = if icc.is_empty() {
            String::from(S_CUR_NONE)
        } else {
            let s = describe(icc);
            if s.is_empty() { String::from(S_CUR_NONAME) } else { s }
        };
        ui.text(TS_BODY, &cur);
        ui.spacer(8);
        // Neu: list + Import / Export
        ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Neu:"));
        let cm = &mut p.cm;
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_H, 8);
        ui.next_size(-1, 0);
        let list: Vec<&str> = cm.list.iter().map(|s| s.as_str()).collect();
        ui.combobox(ID_CM_SEL, &mut cm.sel, &list);
        if ui.button(ID_CM_IMPORT, crate::preferences::localization::tr("Import…")) & EV_ACT != 0 {
            ui.cmd_run(CMD_CM_IMPORT, 0);
        }
        if ui.button(ID_CM_EXPORT, crate::preferences::localization::tr("Export…")) & EV_ACT != 0 {
            ui.cmd_run(CMD_CM_EXPORT, 0);
        }
        ui.box_end();
        ui.spacer(8);
        // intent + black point compensation
        ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Priorität:"));
        ui.next_size(-1, 0);
        ui.combobox(ID_CM_INTENT, &mut cm.intent, &INTENT_LIST);
        ui.checkbox(ID_CM_BPC, crate::preferences::localization::tr("Tiefenkompensierung verwenden"), &mut cm.bpc);
        if let Some(e) = cm.err {
            ui.spacer(8);
            ui.text(TS_BODY | TX_SECONDARY, e);
        }
        ui.spacer(LIST_ROOM);
        let r = ui.dialog_buttons(Some(crate::preferences::localization::tr("Konvertieren")), Some(crate::preferences::localization::tr("Zuweisen")), Some(crate::preferences::localization::tr("Abbrechen")));
        if r != DLG_NONE {
            cm.pending = false;
            if r == DLG_PRIMARY {
                ui.cmd_run(CMD_PROFILE_CONVERT, cm.sel as u32);
            } else if r == DLG_SECONDARY {
                ui.cmd_run(CMD_PROFILE_ASSIGN, cm.sel as u32);
            }
        }
    }
    ui.window_end();
}

// cm_fd_frame() -- the Import / Export file dialog
fn cm_fd_frame(p: &mut Pn, ui: &mut Ui) {
    let flags = if p.cm.fd_mode == 0 { FD_OPEN } else { FD_SAVE };
    let open = p.cmdlg.fd_open.clone();
    if ui.filedlg(ID_FD_ICC, flags, &open, &mut p.cm.fd_path, PATH_CAP as u32, &S_EXTS) & EV_ACT == 0 {
        return;
    }
    p.cm.err = None;
    if p.cm.fd_mode == 0 {
        let ok = match plat::read_file(&p.cm.fd_path) {
            Some(data) => cm_import(&mut p.cm, data),
            None => false,
        };
        if !ok {
            p.cm.err = Some(S_ERR_FILE);
        }
    } else if let Some(doc) = p.cm.doc.filter(|&d| doc_index(&p.doc, d) >= 0) {
        let icc = &p.doc.d(doc).icc;
        let data: &[u8] = if icc.is_empty() { icc_builtin(ICC_BI_SRGB).unwrap_or(&[]) } else { icc };
        if !plat::write_file(&p.cm.fd_path, data) {
            p.cm.err = Some(S_ERR_WRITE);
        }
    }
    ui.repass();
}

// cm_import(icc) -> true when icc is usable (then owned as the imported profile and
// selected), false otherwise
fn cm_import(cm: &mut Cm, icc: Vec<u8>) -> bool {
    if icc.is_empty() || IccXform::build(Some(&icc), None, 0, 0).is_err() {
        return false;
    }
    let d = describe(&icc);
    cm.imp = icc;
    cm.imp_desc = if d.is_empty() { String::from(S_IMP_NONAME) } else { d };
    cm_rows(cm);
    cm.sel = cm.rows.len() as i32 - 1;
    true
}

// ================================================================== module

/// cm_states() -- Bild > Farbprofil... with an image
pub(crate) fn cm_states(p: &mut Pn, ui: &mut Ui) {
    let f = if doc_active(&p.doc).is_some() { 0 } else { CF_DISABLED };
    pn_cmd_set(ui, CMD_COLOR_PROFILE, f);
}

/// cm_frame() -- profile history sync, display transform, dialogs, probes
pub(crate) fn cm_frame(p: &mut Pn, ui: &mut Ui) {
    cm_sync(p);
    cm_view_update(p);
    cm_dlg_frame(p, ui);
    cm_fd_frame(p, ui);
    cm_probes(p, ui);
}

/// cm_cmd(id, arg) -> eax 1 handled
pub(crate) fn cm_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    match id {
        CMD_COLOR_PROFILE => {
            let d = doc_active(&p.doc);
            cm_open(p, d);
            return true;
        }
        CMD_PROFILE_ASSIGN | CMD_PROFILE_CONVERT => {
            let d = doc_active(&p.doc);
            cm_apply(p, d, arg, id == CMD_PROFILE_CONVERT);
            return true;
        }
        CMD_CM_IMPORT => {
            p.cm.fd_mode = 0;
            p.cmdlg.fd_open.set(1);
            return true;
        }
        CMD_CM_EXPORT => {
            p.cm.fd_path = String::from(S_EXP_NAME);
            p.cm.fd_mode = 1;
            p.cmdlg.fd_open.set(1);
            return true;
        }
        _ => {}
    }
    let t = id.wrapping_sub(CMD_CM_TEST0);
    if t > CMD_CM_TEST_LAST - CMD_CM_TEST0 {
        return false;
    }
    match t {
        0 => p.cm.force = arg, // 290 monitor profile
        1 => cm_set_enabled(p, arg != 0), // 291 on / off
        2 => {
            // 292 intent | bpc << 8
            let i = arg & 0xFF;
            p.cm.intent = if i <= ICC_ABSOLUTE { i as i32 } else { 0 };
            p.cm.bpc = ((arg >> 8) & 1) as i32;
        }
        3 => {
            // 293 display probe
            if let Some(doc) = doc_active(&p.doc) {
                let v = crate::document::composite::comp_pixel(p, doc, (arg & 0xFFFF) as i32, (arg >> 16) as i32);
                let mut px = v.to_le_bytes();
                if let Some(x) = cm_view_xf(&p.cm) {
                    x.row_in_place(&mut px);
                }
                p.cm.probe = format!("{:08X}", u32::from_le_bytes(px));
            }
        }
        4 => {
            // 294 built-in as imported profile
            if let Some(b) = icc_builtin(arg) {
                cm_import(&mut p.cm, Vec::from(b));
            }
        }
        _ => {}
    }
    ui.repass();
    true
}

// cm_probes() -- hidden records #0x82300..#0x82303
fn cm_probes(p: &mut Pn, ui: &mut Ui) {
    let (desc, len) = match doc_active(&p.doc) {
        Some(d) if !p.doc.d(d).icc.is_empty() => {
            let icc = &p.doc.d(d).icc;
            (describe(icc), icc.len() as i32)
        }
        _ => (String::new(), 0),
    };
    cm_sem(ui, ID_CM_PROBE, &desc, len);
    let cm = &p.cm;
    cm_sem(ui, ID_CM_PROBE + 1, &cm.probe, cm.enabled as i32);
    let kind = cm_view_xf(cm).map_or(-1, |x| x.kind() as i32);
    cm_sem(ui, ID_CM_PROBE + 2, &cm.mon_desc, kind);
    cm_sem(ui, ID_CM_PROBE + 3, cm.err.unwrap_or(""), cm.cr.len() as i32);
}
