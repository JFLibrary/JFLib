// imgops -- PaintCraft: the Bild menu (agent A5).  Auf Auswahl zuschneiden (sel_crop, T2),
// Größe ändern… (Strg+R), Leinwandgröße… (Strg+Umschalt+R), Horizontal / Vertikal spiegeln,
// 90° nach rechts / links drehen (Strg+H / Strg+G), Um 180° drehen (Strg+J),
// Zusammenfassen (Strg+Umschalt+F: forwarded to A4's lay_flatten_cmd / lay_flat_states).
// Every operation works on all layers: the new surfaces are computed on the worker pool
// (plat::parallel_for over layer x row bands; Größe ändern with Resize::prepare / rows), then
// installed as ONE history step (hist_push_doc, in a compound with hist_push_sel when the
// image has a selection; the old surfaces go to the history through hist_release as
// pn_hist.inc demands).  The selection follows the image (flip / rotate / scale / offset
// through sel_set_xform + sel_bake); a size change places the view again (first placement:
// 100 % or Ganzes Bild).  Dialogs: dlg_image.rs.  Details: apps/PaintCraft/docs/IMAGEOPS.md.
//                                                       [apps/PaintCraft/src/imgops.s]
//
// Module contract (app/lifecycle.rs): iop_init()  iop_states()  iop_frame()  iop_cmd(id, arg)
// Other modules / tests:
//   iop_xform(doc, kind) -> step      XK_* (flips and quarter turns)
//   iop_resize(doc, par) -> step      par: w, h, filter (RS_*), flags (RS_GAMMA), resset /
//                                     unit / dpu
//   iop_canvas(doc, par) -> step      w, h, anchor (AN_*), fill (ARGB for background layers;
//                                     others transparent) + resolution
// Test hooks (cmd ID ARG, the active image):
//   310 w | h << 16        target size of 311 / 312
//   311 filter | gamma << 8   Größe ändern to that size (RS_* filter, gamma 1 = RS_GAMMA)
//   312 anchor | fill << 8    Leinwandgröße to that size (AN_*, FILLC_* of the dialog)
//   313 x | y << 16        pixel of layer L (314) -> #5190 text "AARRGGBB"
//   314 L                  layer index for 313
//   315 kind               iop_xform(kind) (same as the menu commands)
// Hidden records: #5190 text probe · #5191 value 1 = every layer surface has the image size
//   · #5192 text "w,h" of the image, value layers

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use gfx::blend::fill_solid;
use gfx::xform::Matrix;
use gfx::Surface;
use img_core::ops::Band;
use img_core::resample::{Resize, RS_GAMMA};
use rt::sync::SpinLock;
use ui::*;

use crate::document::color::{cm_cmd, cm_frame, cm_states};
use crate::app::constants::*;
use crate::workspace::dialogs::image::{dimg_canvas_open, dimg_fill_argb, dimg_frame, dimg_resize_open, IopPar};
use crate::document::model::{doc_active, doc_resized, Doc, DocId};
use crate::integrations::file_io::fio_error;
use crate::document::history::{hist_begin_compound, hist_end_compound, hist_push_doc, hist_push_sel, hist_release};
use crate::document::layers::{lay_flat_states, lay_flatten_cmd, Layer};
use crate::document::selection::{sel_active, sel_bake, sel_crop, sel_crop_enabled, sel_set_xform, surf_try};
use crate::workspace::view::view_invalidate;
use crate::app::commands::pn_cmd_set;
use crate::app::Pn;

const CHUNK: i32 = 64; // map: columns per block (cache friendly rotations)
const MIN_BAND: i32 = 16;
const IIF_SWAPDPU: u32 = 1; // iop_install: exchange DOC_DPU_X / DOC_DPU_Y

const S_RESIZE: &str = "Größe ändern";
const S_CANVAS: &str = "Leinwandgröße";
const S_OOM: &str = "Der Arbeitsspeicher reicht für diesen Schritt nicht aus.";
const S_OOM_RESIZE: &str = "Zu wenig Arbeitsspeicher, um die Bildgröße zu ändern.";

/// the commands enabled while an image is open
static IOP_CMD_IDS: [u32; 7] = [CMD_RESIZE, CMD_CANVAS_SIZE, CMD_FLIP_H, CMD_FLIP_V, CMD_ROT_CW, CMD_ROT_CCW, CMD_ROT_180];

/// one row of xk_tab: the pixel map dst -> src (c* selects 0, W - 1 or H - 1) and the forward
/// selection matrix (m*; c / f select 0, W or H)
struct Xt {
    swap: bool,
    ax: i32,
    bx: i32,
    cxs: i32,
    ay: i32,
    by: i32,
    cys: i32,
    ma: i32,
    mb: i32,
    mcs: i32,
    md: i32,
    me: i32,
    mfs: i32,
    icon: u32,
    // history names (the same wording as the Bild menu)
    name: &'static str,
}

static XK_TAB: [Xt; XK_COUNT as usize] = [
    Xt { swap: false, ax: -1, bx: 0, cxs: 1, ay: 0, by: 1, cys: 0, ma: -1, mb: 0, mcs: 1, md: 0, me: 1, mfs: 0, icon: 0, name: "Bild horizontal spiegeln" },
    Xt { swap: false, ax: 1, bx: 0, cxs: 0, ay: 0, by: -1, cys: 2, ma: 1, mb: 0, mcs: 0, md: 0, me: -1, mfs: 2, icon: 0, name: "Bild vertikal spiegeln" },
    Xt { swap: true, ax: 0, bx: 1, cxs: 0, ay: -1, by: 0, cys: 2, ma: 0, mb: -1, mcs: 2, md: 1, me: 0, mfs: 0, icon: IC_ROTATE_CW, name: "90° nach rechts drehen" },
    Xt { swap: true, ax: 0, bx: -1, cxs: 1, ay: 1, by: 0, cys: 0, ma: 0, mb: 1, mcs: 0, md: -1, me: 0, mfs: 1, icon: IC_ROTATE_CCW, name: "90° nach links drehen" },
    Xt { swap: false, ax: -1, bx: 0, cxs: 1, ay: 0, by: -1, cys: 2, ma: -1, mb: 0, mcs: 1, md: 0, me: -1, mfs: 2, icon: IC_ROTATE, name: "180° (alle) Drehen" },
];

/// what a job of iop_run computes
enum Job<'a> {
    /// dst(x', y') <- src(ax x' + bx y' + cx, ay x' + by y' + cy)
    Map { ax: i32, bx: i32, cx: i32, ay: i32, by: i32, cy: i32 },
    /// the old image at (ox, oy), the new area filled per layer
    Canvas { ox: i32, oy: i32, fill: Vec<u32> },
    /// a prepared resize per layer (None: no pixels)
    Resize(Vec<Option<Resize<'a>>>),
}

/// the state of imgops.s (its g_* globals)
#[derive(Default)]
pub(crate) struct Iop {
    /// g_tp: parameters of the test hooks
    tp: IopPar,
    /// g_tlayer: layer of hook 313
    tlayer: u32,
    /// g_probe: probe text of #5190
    probe: String,
}

/// iop_init() -- nothing (the dialogs initialise their settings on first use)
pub(crate) fn iop_init(_p: &mut Pn, _ui: &mut Ui) {}

/// iop_states() -- Bild commands: crop (T2 rule), flatten (A4), the rest with an image
pub(crate) fn iop_states(p: &mut Pn, ui: &mut Ui) {
    let c = sel_crop_enabled(p);
    pn_cmd_set(ui, CMD_CROP, if c { 0 } else { CF_DISABLED });
    lay_flat_states(p, ui); // A4: CMD_FLATTEN (Zusammenfassen)
    cm_states(p, ui); // I1: CMD_COLOR_PROFILE (colormgmt.rs)
    let f = if doc_active(&p.doc).is_some() { 0 } else { CF_DISABLED };
    for &id in IOP_CMD_IDS.iter() {
        pn_cmd_set(ui, id, f);
    }
}

/// iop_frame() -- the dialogs, then the hidden probe records
pub(crate) fn iop_frame(p: &mut Pn, ui: &mut Ui) {
    dimg_frame(p, ui);
    cm_frame(p, ui); // I1: Farbprofil dialog, display transform
    iop_probes(p, ui);
}

/// iop_cmd(id, arg) -> handled
pub(crate) fn iop_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    if !(CMD_IMG_FIRST..=CMD_IMG_LAST).contains(&id) {
        return false;
    }
    // I1 (colormgmt.rs): Farbprofil..., commands 280-283, test hooks 290-299
    if id == CMD_COLOR_PROFILE || (CMD_PROFILE_ASSIGN..=CMD_CM_EXPORT).contains(&id) || (CMD_CM_TEST0..=CMD_CM_TEST_LAST).contains(&id) {
        cm_cmd(p, ui, id, arg);
        return true;
    }
    let Some(doc) = doc_active(&p.doc) else { return true };
    match id {
        CMD_CROP => {
            sel_crop(p, doc); // T2: Auf Auswahl zuschneiden
        }
        CMD_RESIZE => dimg_resize_open(p, ui, doc),
        CMD_CANVAS_SIZE => dimg_canvas_open(p, ui, doc),
        _ if (CMD_FLIP_H..CMD_FLIP_H + XK_COUNT as u32).contains(&id) => {
            iop_xform(p, doc, (id - CMD_FLIP_H) as i32);
        }
        CMD_FLATTEN => lay_flatten_cmd(p, ui), // A4 (layer.rs): question + flatten
        CMD_IOP_RESIZE_DO => {
            let par = p.dimg.par;
            iop_resize(p, doc, &par);
        }
        CMD_IOP_CANVAS_DO => {
            let par = p.dimg.par;
            iop_canvas(p, doc, &par);
        }
        _ if (CMD_IOP_TEST0..=CMD_IOP_TEST_LAST).contains(&id) => iop_test(p, ui, id - CMD_IOP_TEST0, arg, doc),
        _ => {}
    }
    true
}

// ================================================================== operations

/// iop_xform(doc, kind) -> step -- flip / rotate the whole image (every layer, the selection
/// and, for quarter turns, the resolution axes)
pub(crate) fn iop_xform(p: &mut Pn, doc: DocId, kind: i32) -> bool {
    if kind as u32 >= XK_COUNT as u32 {
        return false;
    }
    let t = &XK_TAB[kind as usize];
    let (w, h) = (p.doc.d(doc).w, p.doc.d(doc).h);
    let (nw, nh, flags) = if t.swap { (h, w, IIF_SWAPDPU) } else { (w, h, 0) };
    let Some(mut nsf) = iop_alloc(p.doc.d(doc), nw, nh) else {
        fio_error(p, Some(t.name), S_OOM);
        return false;
    };
    // pixel map dst -> src: c* selects 0, W - 1 or H - 1 of the old image
    let sel_px = |s: i32| match s {
        1 => w - 1,
        2 => h - 1,
        _ => 0,
    };
    let job = Job::Map { ax: t.ax, bx: t.bx, cx: sel_px(t.cxs), ay: t.ay, by: t.by, cy: sel_px(t.cys) };
    iop_run(p.doc.d(doc), &mut nsf, &job);
    // the forward matrix for the selection
    let sel_m = |s: i32| match s {
        1 => w,
        2 => h,
        _ => 0,
    };
    let m = Matrix { a: t.ma as f64, b: t.mb as f64, c: sel_m(t.mcs) as f64, d: t.md as f64, e: t.me as f64, f: sel_m(t.mfs) as f64 };
    iop_install(p, doc, Some(nsf), t.name, t.icon, Some(&m), None, flags)
}

/// iop_resize(doc, par) -> step -- "Größe ändern": every layer resampled to w x h with the
/// filter / flags (rows on the worker pool); the selection is scaled; the resolution changes
/// when par.resset
pub(crate) fn iop_resize(p: &mut Pn, doc: DocId, par: &IopPar) -> bool {
    let (w, h) = (par.w, par.h);
    if (w - 1) as u32 >= 65535 || (h - 1) as u32 >= 65535 {
        return false;
    }
    let (ow, oh) = (p.doc.d(doc).w, p.doc.d(doc).h);
    if w == ow && h == oh {
        // same size: only a new resolution makes a step
        if par.resset == 0 {
            return false;
        }
        return iop_install(p, doc, None, S_RESIZE, IC_RESIZE, None, Some(par), 0);
    }
    let ok = 'r: {
        let d = p.doc.d(doc);
        let Some(mut nsf) = iop_alloc(d, w, h) else { break 'r None };
        let mut ctx: Vec<Option<Resize>> = Vec::with_capacity(d.layers.len());
        for (i, l) in d.layers.iter().enumerate() {
            if l.sf.px.is_empty() {
                ctx.push(None); // no pixels: stays transparent
                continue;
            }
            match Resize::prepare(&l.sf, &nsf[i], par.filter, par.flags) {
                Ok(c) => ctx.push(Some(c)),
                Err(_) => break 'r None,
            }
        }
        let job = Job::Resize(ctx);
        if iop_run(d, &mut nsf, &job) {
            break 'r None; // a band ran out of memory
        }
        Some(nsf)
    };
    let Some(nsf) = ok else {
        fio_error(p, Some(S_RESIZE), S_OOM_RESIZE);
        return false;
    };
    // the selection scales with the image
    let m = Matrix { a: w as f64 / ow as f64, b: 0.0, c: 0.0, d: 0.0, e: h as f64 / oh as f64, f: 0.0 };
    iop_install(p, doc, Some(nsf), S_RESIZE, IC_RESIZE, Some(&m), Some(par), 0)
}

/// anchor_off(k, d) -> offset of the old image along one axis: k 0 = start (0), 1 = centre
/// (d / 2 truncated toward zero, like paint.net's C# code), 2 = end (d)
fn anchor_off(k: i32, d: i32) -> i32 {
    match k {
        0 => 0,
        2 => d,
        _ => d / 2,
    }
}

/// iop_canvas(doc, par) -> step -- "Leinwandgröße": every layer gets a w x h surface with the
/// old pixels at the anchor position; the new area of background layers (LYF_BG) is par.fill,
/// of every other layer transparent; the selection moves along
pub(crate) fn iop_canvas(p: &mut Pn, doc: DocId, par: &IopPar) -> bool {
    let (w, h) = (par.w, par.h);
    if (w - 1) as u32 >= 65535 || (h - 1) as u32 >= 65535 {
        return false;
    }
    let (ow, oh) = (p.doc.d(doc).w, p.doc.d(doc).h);
    if w == ow && h == oh {
        if par.resset == 0 {
            return false;
        }
        return iop_install(p, doc, None, S_CANVAS, IC_RESIZE, None, Some(par), 0);
    }
    // offsets of the old image (C# integer division: truncation toward zero)
    let a = if par.anchor > AN_BR as u32 { AN_M } else { par.anchor as i32 };
    let ox = anchor_off(a % 3, w - ow);
    let oy = anchor_off(a / 3, h - oh);
    let d = p.doc.d(doc);
    let Some(mut nsf) = iop_alloc(d, w, h) else {
        fio_error(p, Some(S_CANVAS), S_OOM_RESIZE);
        return false;
    };
    // fill colour per layer
    let fill = d.layers.iter().map(|l| if l.flags & LYF_BG as u32 != 0 { par.fill } else { 0 }).collect();
    iop_run(d, &mut nsf, &Job::Canvas { ox, oy, fill });
    // the selection moves with the old pixels
    let m = Matrix { a: 1.0, b: 0.0, c: ox as f64, d: 0.0, e: 1.0, f: oy as f64 };
    iop_install(p, doc, Some(nsf), S_CANVAS, IC_RESIZE, Some(&m), Some(par), 0)
}

// ================================================================== install (history)

/// iop_install(doc, nsf, name, icon, m, par, flags) -> step.  One history step: the new
/// surfaces nsf (one per layer, taken over; None = surfaces unchanged) replace the layers'
/// surfaces, DOC_W / DOC_H = their size, the resolution (par.resset; IIF_SWAPDPU exchanges the
/// axes), the selection moved by the forward matrix m (None = unchanged).  A compound of
/// hist_push_sel + hist_push_doc; the old surfaces go to the history (hist_release).  On
/// failure nsf is freed.
fn iop_install(p: &mut Pn, doc: DocId, nsf: Option<Vec<Surface>>, name: &'static str, icon: u32, m: Option<&Matrix>, par: Option<&IopPar>, flags: u32) -> bool {
    let (ow, oh) = (p.doc.d(doc).w, p.doc.d(doc).h);
    let (nw, nh) = match nsf.as_ref().and_then(|v| v.first()) {
        Some(s) => (s.w, s.h),
        None => (ow, oh),
    };
    hist_begin_compound(p, doc);
    let mut sel = false;
    if m.is_some() {
        sel = sel_active(p.doc.d(doc));
        if sel && !hist_push_sel(p, doc, name, icon) {
            hist_end_compound(p, doc, name, icon);
            return false;
        }
    }
    if !hist_push_doc(p, doc, name, icon) {
        hist_end_compound(p, doc, name, icon);
        return false;
    }
    if let Some(nsf) = nsf {
        for (i, sf) in nsf.into_iter().enumerate() {
            let l = &mut p.doc.dm(doc).layers[i];
            let old = core::mem::replace(&mut l.sf, sf);
            l.ver = l.ver.wrapping_add(1);
            hist_release(p, doc, Layer { sf: old, ..Layer::default() });
        }
    }
    let d = p.doc.dm(doc);
    d.w = nw;
    d.h = nh;
    if flags & IIF_SWAPDPU != 0 {
        core::mem::swap(&mut d.dpu_x, &mut d.dpu_y);
    }
    if let Some(par) = par {
        if par.resset != 0 {
            d.dpu_unit = par.unit as u8;
            d.dpu_x = par.dpu;
            d.dpu_y = par.dpu;
        }
    }
    if sel {
        sel_set_xform(p, doc, m);
        sel_bake(p, doc);
    }
    doc_resized(p, doc);
    hist_end_compound(p, doc, name, icon);
    // a new size: the view places the image again (100 % or Ganzes Bild)
    if nw != ow || nh != oh {
        p.doc.dm(doc).view.zoom = 0;
        view_invalidate(p);
    }
    p.repass();
    true
}

/// iop_alloc(doc, w, h) -> one new zeroed FMT_ARGB surface per layer (opacity and blend mode
/// of the layer) | None when out of memory
fn iop_alloc(d: &Doc, w: i32, h: i32) -> Option<Vec<Surface>> {
    let mut v = Vec::with_capacity(d.layers.len());
    for l in &d.layers {
        let mut sf = surf_try(w, h)?;
        sf.opac = l.sf.opac;
        sf.blend = l.sf.blend;
        v.push(sf);
    }
    Some(v)
}

// ================================================================== worker pool

/// iop_run(doc, nsf, job) -> true when a band ran out of memory -- the job over layers x row
/// bands of the new surfaces on the pool (about four bands per thread, at least 16 rows)
fn iop_run(d: &Doc, nsf: &mut [Surface], job: &Job) -> bool {
    let Some(first) = nsf.first() else { return false };
    let h = first.h;
    let k = plat::thr_count() as i32 * 4 + 4;
    let bh = ((h + k - 1) / k).max(MIN_BAND);
    let err = AtomicBool::new(false);
    // the bands: (layer, y0, y1, rows y0..y1 of the new surface)
    let mut bands: Vec<SpinLock<(usize, i32, i32, &mut [u8])>> = Vec::new();
    let mut meta: Vec<(i32, i32, usize, u8)> = Vec::new(); // per layer: w, h, stride, fmt
    for (li, sf) in nsf.iter_mut().enumerate() {
        let stride = sf.stride as usize;
        meta.push((sf.w, sf.h, stride, sf.fmt));
        let mut rest: &mut [u8] = &mut sf.px[..];
        let mut y = 0;
        while y < sf.h {
            let y1 = (y + bh).min(sf.h);
            let take = ((y1 - y) as usize * stride).min(rest.len());
            let (a, r) = core::mem::take(&mut rest).split_at_mut(take);
            bands.push(SpinLock::new((li, y, y1, a)));
            rest = r;
            y = y1;
        }
    }
    let item = |i: usize, _: &plat::JobCtl| {
        let mut g = bands[i].lock();
        let (li, y0, y1, ref mut rows) = *g;
        let (dw, dh, ds, dfmt) = meta[li];
        let src = &d.layers[li].sf;
        match job {
            Job::Map { ax, bx, cx, ay, by, cy } => {
                if src.px.is_empty() {
                    return;
                }
                let ss = src.stride as isize;
                // source step per destination pixel (bytes)
                let step = *ax as isize * 4 + *ay as isize * ss;
                let chunk = if step == 4 { dw } else { CHUNK };
                let mut x0 = 0;
                while x0 < dw {
                    let n = (dw - x0).min(chunk) as usize;
                    for y in y0..y1 {
                        let sx = ax.wrapping_mul(x0).wrapping_add(bx.wrapping_mul(y)).wrapping_add(*cx);
                        let sy = ay.wrapping_mul(x0).wrapping_add(by.wrapping_mul(y)).wrapping_add(*cy);
                        let mut so = sy as isize * ss + sx as isize * 4;
                        let o = (y - y0) as usize * ds + x0 as usize * 4;
                        let dst = &mut rows[o..o + n * 4];
                        if step == 4 {
                            let so = so as usize;
                            dst.copy_from_slice(&src.px[so..so + n * 4]);
                        } else {
                            for px in dst.chunks_exact_mut(4) {
                                let s = so as usize;
                                px.copy_from_slice(&src.px[s..s + 4]);
                                so += step;
                            }
                        }
                    }
                    x0 += chunk;
                }
            }
            Job::Canvas { ox, oy, fill } => {
                let c = fill[li];
                // destination columns that receive old pixels: [xa, xb)
                let xa = (*ox).max(0);
                let xb = ox.wrapping_add(src.w).min(dw).max(xa);
                for y in y0..y1 {
                    let row = &mut rows[(y - y0) as usize * ds..];
                    let sy = y.wrapping_sub(*oy);
                    if src.px.is_empty() || sy as u32 >= src.h as u32 || xb <= xa {
                        fill_px(&mut row[..dw as usize * 4], c);
                        continue;
                    }
                    let srow = &src.px[sy as usize * src.stride as usize..];
                    fill_px(&mut row[..xa as usize * 4], c);
                    let sx = (xa - ox) as usize;
                    row[xa as usize * 4..xb as usize * 4].copy_from_slice(&srow[sx * 4..(sx + (xb - xa) as usize) * 4]);
                    fill_px(&mut row[xb as usize * 4..dw as usize * 4], c);
                }
            }
            Job::Resize(ctx) => {
                let Some(c) = ctx[li].as_ref() else { return };
                let mut band = Band { px: &mut rows[..], w: dw, h: dh, fmt: dfmt, stride: ds, y0, y1 };
                if c.rows(&mut band, y0, y1) != 0 {
                    err.store(true, Ordering::Relaxed);
                }
            }
        }
    };
    let _ = plat::parallel_for(bands.len(), &item);
    err.load(Ordering::Relaxed)
}

/// fill_px(dst, colour) -- the pixels of dst (nothing for colour 0: the new surfaces are
/// zeroed already)
fn fill_px(dst: &mut [u8], c: u32) {
    if !dst.is_empty() && c != 0 {
        fill_solid(dst, c);
    }
}

// ================================================================== test hooks

/// iop_test(k, arg, doc) -- hooks 310 + k (see the file header)
fn iop_test(p: &mut Pn, ui: &mut Ui, k: u32, arg: u32, doc: DocId) {
    match k {
        0 => {
            p.iop.tp.w = (arg & 0xFFFF) as i32;
            p.iop.tp.h = (arg >> 16) as i32;
        }
        1 => {
            p.iop.tp.filter = arg & 0xFF;
            p.iop.tp.flags = (arg >> 8) & RS_GAMMA;
            p.iop.tp.resset = 0;
            let par = p.iop.tp;
            iop_resize(p, doc, &par);
        }
        2 => {
            p.iop.tp.anchor = arg & 0xFF;
            p.iop.tp.fill = dimg_fill_argb(p, (arg & 0xFFFF) >> 8);
            p.iop.tp.resset = 0;
            let par = p.iop.tp;
            iop_canvas(p, doc, &par);
        }
        3 => {
            let d = p.doc.d(doc);
            let li = p.iop.tlayer as usize;
            let (x, y) = (arg & 0xFFFF, arg >> 16);
            p.iop.probe = match d.layers.get(li) {
                Some(l) if x < l.sf.w as u32 && y < l.sf.h as u32 && !l.sf.px.is_empty() => {
                    let o = y as usize * l.sf.stride as usize + x as usize * 4;
                    let v = u32::from_le_bytes([l.sf.px[o], l.sf.px[o + 1], l.sf.px[o + 2], l.sf.px[o + 3]]);
                    format!("{v:08X}")
                }
                _ => String::from("none"),
            };
            ui.repass();
        }
        4 => p.iop.tlayer = arg,
        5 => {
            iop_xform(p, doc, arg as i32);
        }
        _ => {}
    }
}

// iop_sem(id, label, value) -- a hidden K_TEXT record (empty rectangle)
fn iop_sem(ui: &mut Ui, id: u32, label: &str, value: i32) {
    ui.last_reset();
    ui.sem(K_TEXT, id, Some(label), value);
}

/// iop_probes() -- hidden records #5190..#5192
fn iop_probes(p: &mut Pn, ui: &mut Ui) {
    iop_sem(ui, ID_IOP_PROBE, &p.iop.probe, 0);
    let Some(doc) = doc_active(&p.doc) else { return };
    let d = p.doc.d(doc);
    // every layer surface has the image size
    let ok = d.layers.iter().all(|l| l.sf.w == d.w && l.sf.h == d.h);
    iop_sem(ui, ID_IOP_PROBE + 1, "", ok as i32);
    iop_sem(ui, ID_IOP_PROBE + 2, &format!("{},{}", d.w, d.h), d.layers.len() as i32);
}
