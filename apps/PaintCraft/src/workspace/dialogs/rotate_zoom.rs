// dlg_rotzoom -- PaintCraft: "Rotationszoom" (Ebenen > Rotationszoom…, Strg+Umschalt+Z) as a
// modal dialog window like paint.net 5's RotateZoomEffect:
//   Kippen / Rotieren   Winkel: dial + number box (degrees, counter-clockwise, -180..180)
//   Verschieben         pan control over a thumbnail of the layer: where the centre of the
//                       layer goes (-1..1 = the canvas edges; Home centres)
//   Zoom                0,06..16 (1 = original size; logarithmic slider + number box)
//   Kachelmodus:        Keine / Wiederholen / Spiegeln (outside the source: transparent, tiled,
//                       mirrored)
//   Original als Hintergrund   the result drawn over the untransformed layer
//   Qualität            Probenahme: Nächster Nachbar / Bilineare / Bikubisch /
//                       Bilinear mit Multisampling (img_core::xform filters)
// The current layer shows the result live: a preview surface of the layer's size is computed
// with Affine::rows on the worker pool (plus the background / selection pass) whenever a
// parameter changed (one CMD_LAY_RZ_UPDATE per frame) and replaces the layer's rows in the
// composite through the compose tool hook (comp_set_tool).  With a selection only the
// selected part changes (antialiased edges: lerp by the coverage).  OK writes the preview
// into the layer as ONE history step "Rotationszoom" (hist_px); Abbrechen / Esc remove the
// hook.  Transform (image coordinates, C = centre, P = pan * C, Z = zoom, a = angle):
//   dst = C + P + R_ccw(a) Z (src - C);  the resampler gets the inverse
//   T(C) S(1/Z) R(a) T(-(C + P))   (xform::rotate: positive = clockwise on screen)
// Sub-module of layer.rs (agent A4):
//   rz_open(doc)            show the dialog for the current layer (defaults)
//   rz_frame()              the window (lay_frame calls it every frame)
//   rz_update()             CMD_LAY_RZ_UPDATE: recompute the preview
//   rz_done(ok)             CMD_LAY_RZ_DONE: commit (ok) or cancel
//   rz_is_open()            rz_probe() -> ("angle|zoom|panx|pany" (x 100), previews)
//   rz_test(op, v)          test hook (a4_test 373)
//   rz_hook(p)              the compose hook (HookKey::RotZoom)
// Window WINID_ROTZOOM, widgets ID_RZ_* (pn_lay.inc).
//                                              [apps/PaintCraft/src/dlg_rotzoom.s]

use alloc::boxed::Box;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::blendmode::bm_row;
use gfx::straight::lerp_mask_into_b;
use gfx::{Surface, FMT_ARGB, FMT_PARGB};
use img_core::ops::Band;
use img_core::resample::{RS_BICUBIC, RS_BILINEAR, RS_MULTISAMPLE, RS_NEAREST};
use img_core::xform::{rotate, scale, translate, Affine, Matrix, XF_EDGE_SHIFT, XF_REFLECT, XF_TRANSPARENT, XF_WRAP};
use rt::fx::{cvtd, maxsd};
use rt::sync::SpinLock;
use ui::*;

use crate::document::composite::{comp_mark, comp_mark_all, comp_set_tool, CompHook, HookKey};
use crate::app::constants::*;
use crate::document::model::{doc_cur_layer, doc_index, Doc, DocId};
use crate::document::layers::{lay_find_uid, lay_rec};
use crate::document::selection::{sel_active, sel_clip_row};
use crate::app::formatting::fmt_cap;
use crate::app::Pn;

const LBL_W: i32 = 104; // "Kachelmodus:" column (DIP)
const THUMB_DIP: i32 = 160; // pan control thumbnail
const PROBE_CAP: usize = 96;
const ZMIN: f64 = 0.06;
const DEG2RAD: f64 = 0.017453292519943295;
const RF_ANGLE: RangeF = RangeF::new(-180.0, 180.0, 0.0, 2, 0);
const RF_TILT: RangeF = RangeF::new(-80.0, 80.0, 0.0, 2, 0);
const RF_ZOOM: RangeF = RangeF::new(0.06, 16.0, 0.0, 2, 0);
const RF_ZLOG: RangeF = RangeF::new(-4.06, 4.0, 0.0, 3, 0);
static RZ_FILTERS: [u32; 4] = [RS_NEAREST, RS_BILINEAR, RS_BICUBIC, RS_MULTISAMPLE];
static RZ_EDGES: [u32; 3] = [XF_TRANSPARENT << XF_EDGE_SHIFT, XF_WRAP << XF_EDGE_SHIFT, XF_REFLECT << XF_EDGE_SHIFT];
static TILE_LIST: [&str; 3] = ["Keine", "Wiederholen", "Spiegeln"];
static SAMPLE_LIST: [&str; 4] = ["Nächster Nachbar", "Bilineare", "Bikubisch", "Bilinear mit Multisampling"];

/// the parameter block g_rz_par (any change recomputes the preview)
#[derive(Clone, Copy, Default)]
struct Par {
    /// RZP_ANGLE: degrees, counter-clockwise
    angle: f64,
    tilt_x: f64,
    tilt_y: f64,
    zoom: f64,
    /// RZP_PANX / RZP_PANY: -1..1
    panx: f64,
    pany: f64,
    /// RZP_TILE: 0 Keine, 1 Wiederholen, 2 Spiegeln
    tile: i32,
    /// RZP_KEEP: Original als Hintergrund
    keep: i32,
    /// RZP_SAMPLE: 0 nearest, 1 bilinear, 2 bicubic, 3 multisample
    sample: i32,
}

impl Par {
    // the 48 bytes of the asm's block (what its hash covers)
    fn bytes(&self) -> [u8; 64] {
        let mut b = [0u8; 64];
        b[0..8].copy_from_slice(&self.angle.to_le_bytes());
        b[8..16].copy_from_slice(&self.zoom.to_le_bytes());
        b[16..24].copy_from_slice(&self.panx.to_le_bytes());
        b[24..32].copy_from_slice(&self.pany.to_le_bytes());
        b[32..36].copy_from_slice(&self.tile.to_le_bytes());
        b[36..40].copy_from_slice(&self.keep.to_le_bytes());
        b[40..44].copy_from_slice(&self.sample.to_le_bytes());
        b[48..56].copy_from_slice(&self.tilt_x.to_le_bytes());
        b[56..64].copy_from_slice(&self.tilt_y.to_le_bytes());
        b
    }
}

/// the state of dlg_rotzoom.s (its g_* globals)
pub(crate) struct Rz {
    par: Par,
    /// g_rz_prev: preview (FMT_ARGB, the layer's size)
    prev: Surface,
    /// g_rz_thumb: pan control thumbnail (FMT_PARGB) and its version
    thumb: Option<Rc<Surface>>,
    thumb_ver: u32,
    doc: Option<DocId>,
    uid: u64,
    /// g_rz_hdone: the parameters of the current preview (None = none)
    hdone: Option<[u8; 64]>,
    /// g_rz_zlog: zoom slider (log2 of the factor); g_rz_zseen: the factor it belongs to
    zlog: f64,
    zseen: f64,
    open: Shared<i32>,
    pending: bool,
    queued: bool,
    count: u32,
    rect: Shared<WinRect>,
}

impl Default for Rz {
    fn default() -> Rz {
        Rz {
            par: Par::default(),
            prev: Surface::default(),
            thumb: None,
            thumb_ver: 0,
            doc: None,
            uid: 0,
            hdone: None,
            zlog: 0.0,
            zseen: 0.0,
            open: Shared::new(0),
            pending: false,
            queued: false,
            count: 0,
            rect: Shared::new(WinRect::new(0, 0, 400, 0, 0)),
        }
    }
}

/// rz_is_open()
pub(crate) fn rz_is_open(r: &Rz) -> bool {
    r.pending
}

// rz_layer() -> (doc, index) of the dialog's layer | None
fn rz_layer(p: &Pn) -> Option<(DocId, i32)> {
    let doc = p.rz.doc?;
    if doc_index(&p.doc, doc) < 0 {
        return None;
    }
    let i = lay_find_uid(p.doc.d(doc), p.rz.uid)?;
    Some((doc, i))
}

// rz_defaults() -- angle 0, zoom 1, centred, no tiling, Bikubisch
fn rz_defaults(r: &mut Rz) {
    r.par = Par { angle: 0.0, tilt_x: 0.0, tilt_y: 0.0, zoom: 1.0, panx: 0.0, pany: 0.0, tile: 0, keep: 0, sample: 2 };
    r.zseen = 0.0; // the zoom slider follows (rz_zsync)
}

/// rz_open(doc) -- the dialog for the current layer: preview surface, pan thumbnail, hook
pub(crate) fn rz_open(p: &mut Pn, ui: &mut Ui, doc: DocId) {
    if p.rz.pending {
        return;
    }
    let Some(l) = p.doc.get(doc).and_then(doc_cur_layer) else { return };
    if l.sf.px.is_empty() {
        return;
    }
    let (lw, lh, uid) = (l.sf.w, l.sf.h, l.uid);
    p.rz.doc = Some(doc);
    p.rz.uid = uid;
    rz_defaults(&mut p.rz);
    p.rz.prev = Surface::new(lw, lh, FMT_ARGB);
    if p.rz.prev.px.is_empty() {
        crate::integrations::file_io::fio_error(p, Some("Rotationszoom"), "Der Arbeitsspeicher reicht für diesen Schritt nicht aus.");
        return;
    }
    // the pan control's thumbnail: the layer fitted into THUMB_DIP
    let side = ui.dp(THUMB_DIP);
    let (mut w, mut h) = (side, side);
    if lw >= lh {
        h = h.wrapping_mul(lh) / lw; // wide: h = side * h / w
    } else {
        w = w.wrapping_mul(lw) / lh; // tall: w = side * w / h
    }
    let mut th = Surface::new(w.max(1), h.max(1), FMT_PARGB);
    p.rz.thumb = None;
    if !th.px.is_empty() {
        let cell = ui.dp(4);
        let l = doc_cur_layer(p.doc.d(doc)).expect("layer");
        crate::document::composite::thumb_render(&mut th, &l.sf, cell);
        p.rz.thumb_ver = p.rz.thumb_ver.wrapping_add(1);
        p.rz.thumb = Some(Rc::new(th));
    }
    p.rz.pending = true;
    p.rz.open.set(1);
    p.rz.hdone = None; // (differs: compute now)
    rz_update(p, ui);
    ui.repass();
}

// rz_matrix(w, h) -> the inverse transform of the parameters (see the header)
fn rz_matrix(par: &Par, w: i32, h: i32) -> Matrix {
    let cx = w as f64 * 0.5;
    let cy = h as f64 * 0.5;
    let mut m = Matrix::IDENTITY;
    translate(&mut m, cx, cy);
    let s = 1.0 / maxsd(par.zoom, ZMIN);
    scale(&mut m, s, s);
    rotate(&mut m, par.angle * DEG2RAD, 0.0, 0.0);
    translate(&mut m, -(par.panx * cx + cx), -(par.pany * cy + cy));
    m
}

// Rotate the source plane about its centre, then project from a camera two image
// widths away. Zoom and pan operate in screen space. Invert the forward homography.
fn rz_projective(p: &Par, w: i32, h: i32) -> [f64; 9] {
    let (sx, cx) = gfx::xform::sincos(p.tilt_x.clamp(-80.0, 80.0) * DEG2RAD);
    let (sy, cy) = gfx::xform::sincos(p.tilt_y.clamp(-80.0, 80.0) * DEG2RAD);
    let (sr, cr) = gfx::xform::sincos(p.angle * DEG2RAD);
    let (x, y) = (w as f64 * 0.5, h as f64 * 0.5);
    let (dx, dy) = (x * (1.0 + p.panx), y * (1.0 + p.pany));
    let f = w.max(h) as f64 * 2.0;
    let (g, h) = (-sy / f, cy * sx / f);
    let z = p.zoom.max(ZMIN);
    let (a, b) = (z * cr * cy + dx * g, z * (cr * sy * sx + sr * cx) + dx * h);
    let (d, e) = (-z * sr * cy + dy * g, z * (-sr * sy * sx + cr * cx) + dy * h);
    let (c, f, i) = (dx - a * x - b * y, dy - d * x - e * y, 1.0 - g * x - h * y);
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    [ (e*i-f*h)/det, (c*h-b*i)/det, (b*f-c*e)/det,
      (f*g-d*i)/det, (a*i-c*g)/det, (c*d-a*f)/det,
      (d*h-e*g)/det, (b*g-a*h)/det, (a*e-b*d)/det ]
}

// the worker context of rz_band (g_rz_ctx)
struct Ctx<'a> {
    xa: Affine<'a>,
    src: &'a Surface,
    doc: &'a Doc,
    keep: bool,
    sel: bool,
    w: i32,
    h: i32,
    stride: usize,
}

// rz_band(band) -- worker: preview rows band * 64 .. + 63: Affine::rows, then "Hintergrund
// beibehalten" (the result over the layer, Normal) and the selection (outside it the layer
// stays; partly selected pixels are a lerp by the coverage)
fn rz_band(c: &Ctx, i: usize, px: &mut [u8]) {
    let y0 = (i as i32) << 6;
    let y1 = (y0 + 64).min(c.h);
    let mut band = Band { px: &mut px[..], w: c.w, h: c.h, fmt: FMT_ARGB, stride: c.stride, y0, y1 };
    c.xa.rows(&mut band, y0, y1);
    if !c.keep && !c.sel {
        return;
    }
    let mut cov = [0u8; 64];
    let mut tmp = [0u8; 256];
    for y in y0..y1 {
        let so = y as usize * c.src.stride as usize;
        let orow = &c.src.px[so..so + c.w as usize * 4];
        let po = (y - y0) as usize * c.stride;
        let prow = &mut px[po..po + c.w as usize * 4];
        let mut x = 0;
        while x < c.w {
            let n = (c.w - x).min(64);
            let a = x as usize * 4;
            let e = a + n as usize * 4;
            let o = &orow[a..e];
            let pv = &mut prow[a..e];
            if c.keep {
                bm_row(&mut tmp[..n as usize * 4], o, pv, 255 << 8); // BM_NORMAL | 255 << 8
                pv.copy_from_slice(&tmp[..n as usize * 4]);
            }
            if c.sel {
                match sel_clip_row(c.doc, x, y, n, &mut cov) {
                    SCR_ALL_U => {}
                    SCR_PART_U => lerp_mask_into_b(o, pv, &cov[..n as usize]),
                    _ => pv.copy_from_slice(o), // not selected: the layer stays
                }
            }
            x += n;
        }
    }
}

const SCR_ALL_U: u32 = SCR_ALL as u32;
const SCR_PART_U: u32 = SCR_PART as u32;

/// the compose hook of the preview (HookKey::RotZoom): the preview's pixels x .. x+n-1 of
/// row y (worker threads)
struct RzHook<'a> {
    prev: &'a Surface,
}

impl CompHook for RzHook<'_> {
    fn row(&self, _doc: &Doc, dst: &mut [u8], x: i32, y: i32, n: i32, _layer_row: &[u8]) -> bool {
        let s = self.prev;
        if s.px.is_empty() || y as u32 >= s.h as u32 || (x.wrapping_add(n)) as u32 > s.w as u32 {
            return false;
        }
        let o = y as usize * s.stride as usize + x as usize * 4;
        let k = n as usize * 4;
        dst[..k].copy_from_slice(&s.px[o..o + k]);
        true
    }
}

/// the compose hook of the preview (HookKey::RotZoom)
pub(crate) fn rz_hook(p: &Pn) -> Option<Box<dyn CompHook + '_>> {
    Some(Box::new(RzHook { prev: &p.rz.prev }))
}

/// rz_update() -- CMD_LAY_RZ_UPDATE: the preview for the current parameters (worker pool), the
/// hook, the composite marked
pub(crate) fn rz_update(p: &mut Pn, ui: &mut Ui) {
    p.rz.queued = false;
    if !p.rz.pending || p.rz.prev.px.is_empty() {
        return;
    }
    let Some((doc, li)) = rz_layer(p) else { return };
    let par = p.rz.par;
    let hash = par.bytes();
    let (w, h) = {
        let d = p.doc.d(doc);
        let l = lay_rec(d, li).expect("layer");
        let m = rz_matrix(&par, l.sf.w, l.sf.h);
        // filter and edge mode (clamped indexes)
        let filter = RZ_FILTERS[if (par.sample as u32) <= 3 { par.sample as usize } else { 2 }];
        let edge = RZ_EDGES[if (par.tile as u32) <= 2 { par.tile as usize } else { 0 }];
        let prev = &mut p.rz.prev;
        let prepared = if par.tilt_x == 0.0 && par.tilt_y == 0.0 {
            Affine::prepare(&l.sf, prev, &m, filter, edge, None)
        } else {
            Affine::prepare_projective(&l.sf, prev, &rz_projective(&par, l.sf.w, l.sf.h), filter, edge, None)
        };
        let Ok(xa) = prepared else { return };
        let c = Ctx {
            xa,
            src: &l.sf,
            doc: d,
            keep: par.keep & 1 != 0,
            sel: sel_active(d),
            w: l.sf.w,
            h: l.sf.h,
            stride: prev.stride as usize,
        };
        let stride = c.stride;
        let nb = ((c.h + 63) as u32 >> 6) as usize;
        let bands: Vec<SpinLock<&mut [u8]>> = prev.px.chunks_mut(stride * 64).take(nb).map(SpinLock::new).collect();
        let item = |i: usize, _: &plat::JobCtl| {
            let mut b = bands[i].lock();
            rz_band(&c, i, &mut b);
        };
        let _ = plat::parallel_for(nb, &item);
        (l.sf.w, l.sf.h)
    };
    comp_set_tool(p, doc, Some(HookKey::RotZoom));
    comp_mark(p, doc, 0, 0, w, h);
    p.rz.hdone = Some(hash);
    p.rz.count = p.rz.count.wrapping_add(1);
    ui.repass();
}

/// rz_done(ok) -- OK: the preview becomes the layer's pixels (one history step
/// "Rotationszoom"); cancel: the hook goes, the composite shows the layer again
pub(crate) fn rz_done(p: &mut Pn, ui: &mut Ui, ok: bool) {
    p.rz.pending = false;
    p.rz.open.set(0);
    if let Some((doc, li)) = rz_layer(p) {
        if rz_commit(p, ui, ok, doc, li).is_none() {
            // cancel
            comp_set_tool(p, doc, None);
            comp_mark_all(p, doc);
        }
    }
    p.rz.prev = Surface::default();
    p.rz.thumb = None;
    ui.repass();
}

// rz_commit(ok, doc, i) -> Some(()) when the preview was written (else: cancel)
fn rz_commit(p: &mut Pn, ui: &mut Ui, ok: bool, doc: DocId, li: i32) -> Option<()> {
    if !ok || p.rz.prev.px.is_empty() {
        return None;
    }
    // the preview for the final parameters
    p.rz.pending = true;
    if p.rz.hdone != Some(p.rz.par.bytes()) {
        rz_update(p, ui);
    }
    p.rz.pending = false;
    if crate::document::history::hist_px_begin(p, doc, li) != 0 {
        return None;
    }
    let (lw, lh) = {
        let l = lay_rec(p.doc.d(doc), li)?;
        (l.sf.w, l.sf.h)
    };
    if crate::document::history::hist_px_touch(p, doc, 0, 0, lw, lh) != 0 {
        crate::document::history::hist_px_cancel(p, doc);
        return None;
    }
    comp_set_tool(p, doc, None);
    // preview rows -> layer rows
    {
        let prev = &p.rz.prev;
        let d = p.doc.dm(doc);
        let l = &mut d.layers[li as usize];
        let n = l.sf.w.min(prev.w) as usize * 4;
        let rows = l.sf.h.min(prev.h);
        let (ls, ps) = (l.sf.stride as usize, prev.stride as usize);
        for y in 0..rows as usize {
            l.sf.px[y * ls..y * ls + n].copy_from_slice(&prev.px[y * ps..y * ps + n]);
        }
    }
    crate::document::model::doc_mark_px(p, doc, li, 0, 0, lw, lh);
    crate::document::history::hist_px_end(p, doc, "Rotationszoom", IC_ROTATE);
    Some(())
}

/// rz_frame() -- the dialog window
pub(crate) fn rz_frame(p: &mut Pn, ui: &mut Ui) {
    if p.rz.pending && p.rz.open.get() == 0 {
        p.rz.pending = false; // Esc / close button: Abbrechen
        ui.cmd_run(CMD_LAY_RZ_DONE, 0);
    }
    let open = p.rz.open.clone();
    let rect = p.rz.rect.clone();
    if ui.window_begin(WINID_ROTZOOM, crate::preferences::localization::tr("Rotationszoom"), Some(&open), WF_DIALOG | WF_MODAL | WF_AUTOHEIGHT, Some(&rect)) {
        let r = &mut p.rz;
        // Kippen / Rotieren
        ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Kippen / Rotieren"));
        ui.box_begin(0, BOX_H | BOX_XCENTER, 16);
        ui.angle(ID_RZ_ANGLE, &mut r.par.angle, 0);
        ui.box_begin(0, BOX_V | BOX_XSTART, 4);
        ui.text(TS_BODY, crate::preferences::localization::tr("Winkel:"));
        ui.next_size(120, 0);
        ui.number_box_f(ID_RZ_ANGLE_NUM, &mut r.par.angle, &RF_ANGLE);
        ui.box_end();
        ui.box_end();
        rz_row(ui, "Kippen X:");
        ui.number_box_f(ID_RZ_ZOOM_NUM + 1, &mut r.par.tilt_x, &RF_TILT);
        ui.box_end();
        rz_row(ui, "Kippen Y:");
        ui.number_box_f(ID_RZ_ZOOM_NUM + 2, &mut r.par.tilt_y, &RF_TILT);
        ui.box_end();
        // Verschieben
        ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Verschieben"));
        let thumb = r.thumb.clone().filter(|t| !t.px.is_empty());
        ui.pan(ID_RZ_PAN, &mut r.par.panx, &mut r.par.pany, thumb.as_ref(), r.thumb_ver, 0);
        // Zoom: a logarithmic slider (1/16 .. 16) and the factor in a number box
        rz_zsync(r, ui);
        ui.text(TS_BODY, crate::preferences::localization::tr("Zoom"));
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
        ui.next_size(-1, 0);
        if ui.slider_f(ID_RZ_ZOOM, &mut r.zlog, &RF_ZLOG) & EV_ACT != 0 {
            let z = rt::x87::rz_exp2(r.zlog);
            r.par.zoom = z;
            r.zseen = z;
        }
        ui.next_size(88, 0);
        ui.number_box_f(ID_RZ_ZOOM_NUM, &mut r.par.zoom, &RF_ZOOM);
        ui.box_end();
        // Kachelmodus
        rz_row(ui, "Kachelmodus:");
        ui.combobox(ID_RZ_TILE, &mut r.par.tile, &TILE_LIST);
        ui.box_end();
        ui.checkbox(ID_RZ_KEEP, crate::preferences::localization::tr("Original als Hintergrund"), &mut r.par.keep);
        // Qualität
        ui.text(TS_BODY_STRONG, crate::preferences::localization::tr("Qualität"));
        rz_row(ui, "Probenahme:");
        ui.combobox(ID_RZ_SAMPLE, &mut r.par.sample, &SAMPLE_LIST);
        ui.box_end();
        // a changed parameter: one preview update after this frame
        if r.hdone != Some(r.par.bytes()) && !r.queued {
            r.queued = true;
            ui.cmd_run(CMD_LAY_RZ_UPDATE, 0);
        }
        let b = ui.dialog_buttons(Some(crate::preferences::localization::tr("OK")), None, Some(crate::preferences::localization::tr("Abbrechen")));
        if b != DLG_NONE {
            p.rz.pending = false;
            ui.cmd_run(CMD_LAY_RZ_DONE, (b == DLG_PRIMARY) as u32);
        }
    }
    ui.window_end();
}

// rz_zsync() -- the slider position follows a zoom factor set elsewhere (number box, test)
fn rz_zsync(r: &mut Rz, ui: &mut Ui) {
    let z = r.par.zoom;
    // ucomisd + je: equal or unordered
    if z == r.zseen || z.is_nan() || r.zseen.is_nan() {
        return;
    }
    r.zseen = z;
    r.zlog = rt::x87::rz_log2(maxsd(z, ZMIN));
    ui.repass();
}

// rz_row(label) -- a BOX_H row "label" + a filling control (the caller closes the box)
fn rz_row(ui: &mut Ui, label: &str) {
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
    ui.next_size(LBL_W, 0);
    ui.text(TS_BODY, label);
    ui.next_size(-1, 0);
}

/// rz_probe() -> ("angle|zoom|panx|pany" (x 100, rounded), previews computed)
pub(crate) fn rz_probe(r: &Rz) -> (String, u32) {
    let q = &r.par;
    let t = format!(
        "{}|{}|{}|{}",
        cvtd(q.angle * 100.0),
        cvtd(q.zoom * 100.0),
        cvtd(q.panx * 100.0),
        cvtd(q.pany * 100.0)
    );
    (fmt_cap(t, PROBE_CAP), r.count)
}

/// rz_test(op, v) -- set a parameter (test hook): 0 angle (degrees), 1 zoom (%), 2 pan x (%),
/// 3 pan y (%), 4 Kachelmodus, 5 Original als Hintergrund, 6 Probenahme
pub(crate) fn rz_test(p: &mut Pn, ui: &mut Ui, op: u32, v: i32) {
    let q = &mut p.rz.par;
    let f = v as f64;
    match op {
        0 => q.angle = f,
        1 => q.zoom = f / 100.0,
        2 => q.panx = f / 100.0,
        3 => q.pany = f / 100.0,
        4 => q.tile = v,
        5 => q.keep = v,
        6 => q.sample = v,
        7 => q.tilt_x = f.clamp(-80.0, 80.0),
        8 => q.tilt_y = f.clamp(-80.0, 80.0),
        _ => {}
    }
    ui.repass();
}
