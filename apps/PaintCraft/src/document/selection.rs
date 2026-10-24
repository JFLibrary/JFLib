// sel -- PaintCraft: Auswahl (agent T2): the selection model of every image, the Bearbeiten
// selection commands, "Auf Auswahl zuschneiden", marching ants, selection history.
//                                                [apps/PaintCraft/src/sel.s, inc/pn_sel.inc]
//
// Module contract (app/lifecycle.rs): sel_init()  sel_states()  sel_frame()  sel_cmd(id, arg)
//   sel_size() -> (w, h)  sel_offset() -> (x, y)    bounds of the active image's selection
// Public API (docs/SELECTION.md): sel_clip_row, sel_active, sel_mask, sel_bbox, sel_version,
//   sel_poly_new, sel_poly_rect, sel_poly_ellipse, sel_poly_trace, sel_poly_xform,
//   sel_poly_bbox, sel_poly_ref, sel_poly_unref, sel_combine, sel_apply, sel_set_rect,
//   sel_set_ellipse, sel_edit_last, sel_set_preview, sel_clear, sel_set_xform, sel_bake,
//   sel_translate, sel_erase, sel_fill, sel_crop, sel_crop_enabled, sel_geom_text,
//   sel_geom_parse, sel_line, sel_hsnap, sel_hrestore, sel_hfree, sel_hsize, sel_diag_cmd,
//   sel_diag_frame
//
// Model: per document a list of ops {POLY, SM_* mode, SQ_* quality} (the state record lives in
// DocSel) replayed into the coverage mask DocSel::mask (TMask of the image size) from the last
// SM_REPLACE op on: antialiased ops through raster_path (even-odd), pixelated ops thresholded
// at 128.  A preview op (tool drag) and a pending transform (move tools) take part in
// rendering only.  The outline for the marching ants is the transformed polygon when a single
// antialiased op is shown, else the traced mask.  History (hist.rs): a blob holds the op list
// with shared (Arc) polygons, so a step costs 16 bytes per op.
//
// Screen damage of the ants (view_damage_doc needs the Ui) is collected while the selection
// changes and handed to the view by sel_states / sel_frame / sel_cmd (before the render).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use gfx::blendmode::{bm_solid_row_inplace, BM_NORMAL, BM_OPAC_SHIFT};
use gfx::path::Path;
use gfx::raster::Raster;
use gfx::straight::erase_row;
use gfx::tmask::{Span, TMask};
use gfx::xform::{sincos, Matrix};
use gfx::{Surface, FMT_ARGB, RULE_EVENODD};
use img_core::ERR_NOMEM;
use rt::fx::{cvt, cvtt, cvttd, maxsd, maxss, minsd, minss, sqrtd};
use rt::sync::SpinLock;
use ui::*;

use crate::app::constants::*;
use crate::document::model::{doc_active, doc_cur_layer, doc_mark_px, doc_resized, Doc, DocId};
use crate::document::history::{hist_begin_compound, hist_end_compound, hist_push_doc, hist_push_sel, hist_px_begin, hist_px_cancel, hist_px_end, hist_px_touch, hist_release, hist_sel_hooks};
use crate::document::layers::Layer;
use crate::workspace::palette::pal_primary;
use crate::workspace::view::{view_damage_doc, view_doc_to_screen_f64, view_overlay_add};
use crate::app::Pn;

const ANT_MS: u32 = 100; // marching ants step
const ANT_DASH: i32 = 4; // dash length (DIP): black, white, black ...
const ANT_EDGES: usize = 256; // outlines with fewer points damage edge by edge
const ELL_MIN: i32 = 16; // ellipse polygon points
const ELL_MAX: i32 = 8192;
const PL_CNT: u64 = 16; // header bytes of a POLY block (PL_REF, PL_NSUB, PL_NPTS, PL_BYTES)
const GEOM_HDR: &str = "PaintCraft Auswahl 1";
const SF_XF: u32 = SELF_XF as u32;
const SF_PREVIEW: u32 = SELF_PREVIEW as u32;
const M_REPLACE: u32 = SM_REPLACE as u32;
const M_EXCLUDE: u32 = SM_EXCLUDE as u32;
const M_INTERSECT: u32 = SM_INTERSECT as u32;
const M_XOR: u32 = SM_XOR as u32;
const Q_AA: u32 = SQ_AA as u32;

/// a refcounted polygon set (POLY: f32 x, y image px; contours closed implicitly, even-odd);
/// Arc so that a Doc stays Sync
pub(crate) type PolyRef = Arc<Poly>;

/// POLY block
#[derive(Default)]
pub(crate) struct Poly {
    /// PL_CNT: points per contour
    pub(crate) cnt: Vec<u32>,
    /// every contour's points one after the other
    pub(crate) pts: Vec<[f32; 2]>,
}

impl Poly {
    /// PL_BYTES: the size of the asm's POLY block (history accounting)
    pub(crate) fn bytes(&self) -> u64 {
        PL_CNT + ((self.cnt.len() as u64 * 4 + 7) & !7) + self.pts.len() as u64 * 8
    }
}

// poly_alloc's size check: blocks above 2 GB fail
fn poly_ok(nsub: usize, npts: usize) -> bool {
    PL_CNT + ((nsub as u64 * 4 + 7) & !7) + npts as u64 * 8 <= 0x7FFF_FFFF
}

/// one op of the list (SOP_*)
#[derive(Clone)]
pub(crate) struct Op {
    pub(crate) poly: PolyRef,
    pub(crate) mode: u32,
    pub(crate) qual: u32,
    /// A positive radius filters the accumulated mask at this point in the op list.
    feather: u32,
}

/// a selection snapshot of the history (BL_*): the op list with shared polygons
pub(crate) struct SelBlob {
    flags: u32,
    xf: Matrix,
    ops: Vec<Op>,
}

/// the per-document selection (DOC_SEL block + the state record).  Other modules read the
/// public fields.  Part of Doc: Send + Sync (no Rc / RefCell inside).
pub(crate) struct DocSel {
    /// SEL_MASK: rendered coverage, None = no selection
    pub(crate) mask: Option<Box<TMask>>,
    /// SEL_X0 .. SEL_Y1: bounding box of the coverage (exclusive), all 0 when empty
    pub(crate) bbox: [i32; 4],
    /// SEL_NOPS: ops (0 = no selection)
    pub(crate) nops: u32,
    /// SEL_FLAGS: SELF_*
    pub(crate) flags: u32,
    /// SEL_OX0 .. SEL_OY1: bounding box of the marching ants outline (x1 <= x0 = none)
    pub(crate) obox: [i32; 4],
    // ---- the state record (SS_*); `state` = it exists (ss_find)
    state: bool,
    ops: Vec<Op>,
    /// SS_MASK while it is not shown (SEL_MASK = 0 but the record keeps it)
    keep: Option<Box<TMask>>,
    outl: Option<PolyRef>,
    tmp: Option<Box<TMask>>,
    pv: Option<Op>,
    xf: Matrix,
}

impl Default for DocSel {
    fn default() -> DocSel {
        DocSel {
            mask: None,
            bbox: [0; 4],
            nops: 0,
            flags: 0,
            obox: [0; 4],
            state: false,
            ops: Vec::new(),
            keep: None,
            outl: None,
            tmp: None,
            pv: None,
            xf: Matrix::IDENTITY,
        }
    }
}

/// the state of sel.s (its g_* globals)
#[derive(Default)]
pub(crate) struct Sel {
    feather_open: Shared<i32>,
    feather_doc: Option<DocId>,
    feather_radius: i32,
    /// the path and rasterizer of sel_render (the asm's current path)
    path: Path,
    ras: Raster,
    /// ants damage for the view: (document, x0, y0, x1, y1)
    dmg: Vec<(DocId, [i32; 4])>,
    /// g_ant_on: the ants timer is pending
    ant_on: bool,
    ant_phase: u32,
    /// ui_dp(ANT_DASH) of the last frame (the overlay draws without a Ui)
    dash: i32,
    dg_p0: u32,
    dg_p1: u32,
    dg_mode: u32,
    dg_cov: u32,
    dg_res: &'static str,
}

// ================================================================== module

/// sel_init() -- history hooks, marching ants overlay, the selection / move tools
pub(crate) fn sel_init(p: &mut Pn, ui: &mut Ui) {
    hist_sel_hooks(&mut p.hist, true);
    view_overlay_add(p, sel_overlay, 0);
    crate::tools::select::tsel_init(p, ui);
    crate::tools::wand::twand_init(p, ui);
    crate::tools::move_pixels::tmove_init(p, ui);
}

/// sel_frame() -- the ants damage collected since the last pass
pub(crate) fn sel_frame(p: &mut Pn, ui: &mut Ui) {
    let open=p.sel.feather_open.clone();
    if ui.window_begin(0x60E8,crate::preferences::localization::tr("Weiche Auswahlkante"),Some(&open),WF_DIALOG|WF_MODAL|WF_AUTOHEIGHT,None) {
        ui.text(TS_BODY,crate::preferences::localization::tr("Radius in Bildpixeln:"));
        ui.number_box(0x93000,&mut p.sel.feather_radius,&Range::new(1,128,1,0));
        if ui.button(0x93001,crate::preferences::localization::tr("Anwenden"))&EV_ACT!=0 {
            if let Some(doc)=p.sel.feather_doc.filter(|d|p.doc.get(*d).is_some()) {
                if hist_push_sel(p,doc,"Weiche Auswahlkante",IC_SELECT_ALL) {
                    sel_bake(p,doc);
                    p.doc.dm(doc).sel.ops.push(Op{poly:Arc::new(Poly::default()),mode:SM_UNION as u32,qual:Q_AA,feather:p.sel.feather_radius.clamp(1,128) as u32});
                    sel_render(p,doc);
                }
            }
            open.set(0);ui.repass();
        }
        if ui.button(0x93002,crate::preferences::localization::tr("Abbrechen"))&EV_ACT!=0 {open.set(0);ui.repass();}
    }
    ui.window_end();
    sel_flush(p, ui);
}

// the collected ants damage to the view
fn sel_flush(p: &mut Pn, ui: &mut Ui) {
    p.sel.dash = ui.dp(ANT_DASH);
    if p.sel.dmg.is_empty() {
        return;
    }
    let dmg = core::mem::take(&mut p.sel.dmg);
    for (doc, r) in &dmg {
        view_damage_doc(&p.view, ui, *doc, r[0], r[1], r[2], r[3]);
    }
}

// --anim off (the library's g_anim_off)
fn anim_off() -> bool {
    plat::arg_value("--anim").as_deref() == Some("off")
}

/// sel_states() -- command states, size changes, ants timer
pub(crate) fn sel_states(p: &mut Pn, ui: &mut Ui) {
    let doc = doc_active(&p.doc);
    pn_cmd(ui, CMD_SELECT_ALL, doc.is_some());
    let have = doc.is_some_and(|d| p.doc.d(d).sel.nops != 0);
    for id in [CMD_DESELECT, CMD_ERASE_SEL, CMD_FILL_SEL, CMD_INVERT_SEL, CMD_FEATHER_SEL] {
        pn_cmd(ui, id, have);
    }
    if let (true, Some(doc)) = (have, doc) {
        // the image size changed (canvas size, resize, undo of a crop): render again
        let d = p.doc.d(doc);
        let m = if d.sel.mask.is_some() { d.sel.mask.as_deref() } else { d.sel.keep.as_deref() };
        if d.sel.state && m.is_some_and(|m| m.w != d.w as u32 || m.h != d.h as u32) {
            sel_render(p, doc);
        }
        // marching ants: keep the timer running while the active image shows an outline
        let s = &p.doc.d(doc).sel;
        if !p.sel.ant_on && !anim_off() && s.obox[2] > s.obox[0] {
            p.sel.ant_on = true;
            ui.timer(TMR_ANTS, ANT_MS);
        }
    }
    sel_flush(p, ui);
}

// pn_cmd_set(id, ok ? 0 : CF_DISABLED)
fn pn_cmd(ui: &mut Ui, id: u32, ok: bool) {
    crate::app::commands::pn_cmd_set(ui, id, if ok { 0 } else { CF_DISABLED });
}

/// sel_cmd(id, arg) -> handled: 180..199 and the ants timer
pub(crate) fn sel_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    if id == UIC_TIMER {
        if arg == TMR_ANTS {
            sel_ant_tick(p, ui);
            sel_flush(p, ui);
            return true;
        }
        if arg == TMR_TMOVE {
            crate::tools::move_pixels::tmove_timer(p, ui);
            sel_flush(p, ui);
            return true;
        }
        return false;
    }
    if !(CMD_SEL_FIRST..=CMD_SEL_LAST).contains(&id) {
        return false;
    }
    let Some(doc) = doc_active(&p.doc) else { return true };
    if id == CMD_SELECT_ALL {
        if hist_push_sel(p, doc, "Alles auswählen", IC_SELECT_ALL) {
            let (w, h) = (p.doc.d(doc).w, p.doc.d(doc).h);
            sel_set_rect(p, doc, 0, 0, w, h);
        }
    } else if p.doc.d(doc).sel.nops != 0 {
        match id {
            CMD_FEATHER_SEL=>{
                p.sel.feather_doc=Some(doc);if p.sel.feather_radius<1 {p.sel.feather_radius=5;}
                p.sel.feather_open.set(1);ui.repass();
            }
            CMD_DESELECT => {
                if hist_push_sel(p, doc, "Auswahl aufheben", IC_CLEAR_SELECTION) {
                    sel_clear(p, doc);
                }
            }
            CMD_INVERT_SEL => {
                if hist_push_sel(p, doc, "Auswahl umkehren", IC_SELECT_ALL) {
                    let (w, h) = (p.doc.d(doc).w, p.doc.d(doc).h);
                    if let Some(r) = sel_poly_rect(0, 0, w, h) {
                        sel_combine(p, doc, &r, M_XOR, Q_AA);
                    }
                }
            }
            CMD_ERASE_SEL => {
                sel_erase(p, doc);
            }
            CMD_FILL_SEL => {
                let c = pal_primary(&p.pal);
                sel_fill(p, doc, c);
            }
            _ => {}
        }
    }
    sel_flush(p, ui);
    true
}

/// sel_size() -> (w, h) of the active image's selection bounds ((0, 0) = none)
pub(crate) fn sel_size(p: &Pn) -> (i32, i32) {
    match doc_active(&p.doc).map(|d| &p.doc.d(d).sel) {
        Some(s) if s.nops != 0 => (s.bbox[2] - s.bbox[0], s.bbox[3] - s.bbox[1]),
        _ => (0, 0),
    }
}

/// sel_offset() -> (x, y): top left corner of the selection bounds ((0, 0) = none)
pub(crate) fn sel_offset(p: &Pn) -> (i32, i32) {
    match doc_active(&p.doc).map(|d| &p.doc.d(d).sel) {
        Some(s) if s.nops != 0 => (s.bbox[0], s.bbox[1]),
        _ => (0, 0),
    }
}

// ================================================================== reading

/// sel_active(doc) -> true when doc has a selection
pub(crate) fn sel_active(d: &Doc) -> bool {
    d.sel.nops != 0
}

/// sel_mask(doc) -> the coverage TMASK | None
pub(crate) fn sel_mask(d: &Doc) -> Option<&TMask> {
    d.sel.mask.as_deref()
}

/// sel_version(doc) -> DOC_VER_SEL
pub(crate) fn sel_version(d: &Doc) -> u32 {
    d.ver_sel
}

/// sel_bbox(doc) -> x0, y0, x1, y1 of the coverage (None when empty or no selection)
pub(crate) fn sel_bbox(d: &Doc) -> Option<[i32; 4]> {
    let s = &d.sel;
    if s.nops == 0 || s.bbox[2] <= s.bbox[0] {
        return None;
    }
    Some(s.bbox)
}

/// sel_clip_row(doc, x, y, n, out) -> SCR_NONE / SCR_PART / SCR_ALL -- selection coverage of
/// the pixels (x .. x+n-1, y), 1 <= n <= 64; pure (worker threads).  No selection = SCR_ALL;
/// pixels outside the image are unselected.  out[0..n-1] is written only for SCR_PART.
pub(crate) fn sel_clip_row(d: &Doc, x: i32, y: i32, n: i32, out: &mut [u8]) -> u32 {
    clip_row(&d.sel, x, y, n, out) as u32
}

// sel_clip_row on the selection block
fn clip_row(s: &DocSel, x: i32, y: i32, n: i32, out: &mut [u8]) -> i32 {
    let Some(m) = s.mask.as_deref() else { return SCR_ALL };
    if y as u32 >= m.h {
        return SCR_NONE; // y < 0 or y >= h
    }
    let w = m.w as i32;
    if x >= w {
        return SCR_NONE;
    }
    let end = x.wrapping_add(n);
    if end <= 0 {
        return SCR_NONE;
    }
    // the common case: the run lies inside the mask and is one span
    if x >= 0 && end <= w {
        let (sp, k) = m.span(x, y, n);
        if k == n {
            match sp {
                Span::Empty => return SCR_NONE,
                Span::Full => return SCR_ALL,
                Span::Bytes(b) => {
                    out[..n as usize].copy_from_slice(&b[..n as usize]);
                    return SCR_PART;
                }
            }
        }
    }
    // the part inside the mask: [max(x, 0), min(x + n, w)); anything selected?
    let (x0, x1) = (x.max(0), end.min(w));
    let mut pos = x0;
    loop {
        if pos >= x1 {
            return SCR_NONE;
        }
        let (sp, k) = m.span(pos, y, x1 - pos);
        if k == 0 {
            return SCR_NONE;
        }
        if !matches!(sp, Span::Empty) {
            break;
        }
        pos += k;
    }
    out[..n as usize].fill(0);
    let mut pos = x0;
    while pos < x1 {
        let (sp, k) = m.span(pos, y, x1 - pos);
        if k == 0 {
            break;
        }
        let o = (pos - x) as usize;
        match sp {
            Span::Empty => {}
            Span::Full => out[o..o + k as usize].fill(255),
            Span::Bytes(b) => out[o..o + k as usize].copy_from_slice(&b[..k as usize]),
        }
        pos += k;
    }
    SCR_PART
}

// ================================================================== polygons

/// sel_poly_ref(poly) -- one more reference (Arc clone)
pub(crate) fn sel_poly_ref(poly: &PolyRef) -> PolyRef {
    poly.clone()
}

/// sel_poly_unref(poly) -- one reference less (drop)
pub(crate) fn sel_poly_unref(_poly: Option<PolyRef>) {}

/// sel_poly_new(pts) -> POLY of one contour (f32 x, y points) | None
pub(crate) fn sel_poly_new(pts: &[[f32; 2]]) -> Option<PolyRef> {
    if pts.is_empty() || !poly_ok(1, pts.len()) {
        return None;
    }
    Some(Arc::new(Poly { cnt: vec![pts.len() as u32], pts: pts.to_vec() }))
}

/// sel_poly_rect(x0, y0, x1, y1) -> POLY (clockwise rectangle, corners ordered) | None
pub(crate) fn sel_poly_rect(x0: i32, y0: i32, x1: i32, y1: i32) -> Option<PolyRef> {
    let (x0, x1) = if x0 > x1 { (x1, x0) } else { (x0, x1) };
    let (y0, y1) = if y0 > y1 { (y1, y0) } else { (y0, y1) };
    let (a, b, c, d) = (x0 as f32, y0 as f32, x1 as f32, y1 as f32);
    sel_poly_new(&[[a, b], [c, b], [c, d], [a, d]])
}

/// sel_poly_ellipse(x0, y0, x1, y1) -> POLY: the ellipse inscribed in the rectangle,
/// n = 5 pi sqrt(r) points (max. error ~0.02 px), a multiple of 4, clockwise from 3 o'clock
pub(crate) fn sel_poly_ellipse(x0: i32, y0: i32, x1: i32, y1: i32) -> Option<PolyRef> {
    let (x0, x1) = if x0 > x1 { (x1, x0) } else { (x0, x1) };
    let (y0, y1) = if y0 > y1 { (y1, y0) } else { (y0, y1) };
    if x0 == x1 || y0 == y1 {
        return sel_poly_rect(x0, y0, x1, y1);
    }
    let (fx0, fx1) = (x0 as f64, x1 as f64);
    let rx = (fx1 - fx0) * 0.5;
    let cx = (fx0 + fx1) * 0.5;
    let (fy0, fy1) = (y0 as f64, y1 as f64);
    let ry = (fy1 - fy0) * 0.5;
    let cy = (fy0 + fy1) * 0.5;
    let r = sqrtd(maxsd(rx, ry) * 25.0) * core::f64::consts::PI;
    let n = (cvttd(r).wrapping_add(4) & -4).clamp(ELL_MIN, ELL_MAX);
    let (s, c) = sincos(core::f64::consts::TAU / n as f64);
    let mut pts = Vec::with_capacity(n as usize);
    let (mut ux, mut uy) = (1.0f64, 0.0f64);
    for _ in 0..n {
        pts.push([(ux * rx + cx) as f32, (uy * ry + cy) as f32]);
        let nx = ux * c - uy * s;
        let ny = ux * s + uy * c;
        ux = nx;
        uy = ny;
    }
    Some(Arc::new(Poly { cnt: vec![n as u32], pts }))
}

// one point through m: (a x + b y + c, x d + y e + f), back to f32
#[inline]
fn xf_pt(m: &Matrix, x: f32, y: f32) -> [f32; 2] {
    let (x, y) = (x as f64, y as f64);
    let nx = m.a * x + m.b * y + m.c;
    let ny = x * m.d + y * m.e + m.f;
    [nx as f32, ny as f32]
}

/// sel_poly_xform(poly, m) -> new POLY with every point mapped by m | None
pub(crate) fn sel_poly_xform(poly: &Poly, m: &Matrix) -> Option<PolyRef> {
    if !poly_ok(poly.cnt.len(), poly.pts.len()) {
        return None;
    }
    let pts = poly.pts.iter().map(|q| xf_pt(m, q[0], q[1])).collect();
    Some(Arc::new(Poly { cnt: poly.cnt.clone(), pts }))
}

/// sel_poly_bbox(poly) -> x0, y0, x1, y1 of the points (all 0 without points)
pub(crate) fn sel_poly_bbox(poly: Option<&Poly>) -> [f64; 4] {
    let Some(p) = poly else { return [0.0; 4] };
    let Some(f) = p.pts.first() else { return [0.0; 4] };
    let (mut x0, mut y0, mut x1, mut y1) = (f[0], f[1], f[0], f[1]);
    for q in &p.pts {
        x0 = minss(x0, q[0]);
        x1 = maxss(x1, q[0]);
        y0 = minss(y0, q[1]);
        y1 = maxss(y1, q[1]);
    }
    [x0 as f64, y0 as f64, x1 as f64, y1 as f64]
}

/// the polygon builder (pb_*): contours collected for one POLY
#[derive(Default)]
struct Builder {
    cnt: Vec<u32>,
    pts: Vec<[f32; 2]>,
}

impl Builder {
    // pb_contour(pts, n, 0): a contour of i32 points
    fn contour_i(&mut self, pts: &[(i32, i32)]) {
        if !pts.is_empty() {
            self.cnt.push(pts.len() as u32);
            self.pts.extend(pts.iter().map(|&(x, y)| [x as f32, y as f32]));
        }
    }
    // pb_contour(pts, n, 1): a contour of f32 points
    fn contour_f(&mut self, pts: &[[f32; 2]]) {
        if !pts.is_empty() {
            self.cnt.push(pts.len() as u32);
            self.pts.extend_from_slice(pts);
        }
    }
    // pb_finish() -> POLY of the collected contours | None
    fn finish(self) -> Option<PolyRef> {
        if !poly_ok(self.cnt.len(), self.pts.len()) {
            return None;
        }
        Some(Arc::new(Poly { cnt: self.cnt, pts: self.pts }))
    }
}

/// sel_poly_trace(tm) -> POLY of the pixel-edge contours of tm (coverage >= 128) | None
pub(crate) fn sel_poly_trace(tm: &TMask) -> Option<PolyRef> {
    let mut b = Builder::default();
    let r = tm.trace(&mut |pts| {
        b.contour_i(pts);
        0
    });
    if r != 0 {
        return None;
    }
    b.finish()
}

// path_poly(poly, m | None) -- the path = the contours of poly, mapped by m
fn path_poly(path: &mut Path, poly: &Poly, m: Option<&Matrix>) {
    path.reset();
    let mut k = 0usize;
    for &n in &poly.cnt {
        for j in 0..n as usize {
            let mut q = poly.pts[k + j];
            if let Some(m) = m {
                q = xf_pt(m, q[0], q[1]);
            }
            if j == 0 {
                path.move_f(q[0], q[1]);
            } else {
                path.line_f(q[0], q[1]);
            }
        }
        k += n as usize;
    }
}

// floor_i(x) -> floor, clamped to +-2^30
fn floor_i(x: f64) -> i32 {
    let x = minsd(maxsd(x, -1073741824.0), 1073741824.0);
    let v = cvttd(x);
    if (v as f64) > x { v - 1 } else { v }
}

// ================================================================== rendering

// render_op(state, poly, mode, qual) -> 0 / ERR_* -- combine one op into the mask
fn render_op(path: &mut Path, ras: &mut Raster, s: &mut DocSel, poly: &Poly, mode: u32, qual: u32) -> i32 {
    let m = if s.flags & SF_XF != 0 { Some(s.xf) } else { None };
    path_poly(path, poly, m.as_ref());
    let mask = s.keep.as_mut().unwrap();
    if qual == Q_AA {
        return mask.raster_path(path, ras, RULE_EVENODD, mode);
    }
    if mode == M_REPLACE {
        let r = mask.raster_path(path, ras, RULE_EVENODD, M_REPLACE);
        mask.threshold(128);
        return r;
    }
    // pixelated op combined with the others: rendered alone into the scratch mask
    let (w, h) = (mask.w, mask.h);
    if s.tmp.as_ref().is_some_and(|t| t.w != w || t.h != h) {
        s.tmp = None;
    }
    if s.tmp.is_none() {
        match TMask::new(w, h) {
            Some(t) => s.tmp = Some(Box::new(t)),
            None => return ERR_NOMEM,
        }
    }
    let t = s.tmp.as_mut().unwrap();
    t.raster_path(path, ras, RULE_EVENODD, M_REPLACE);
    t.threshold(128);
    s.keep.as_mut().unwrap().combine(s.tmp.as_ref().unwrap(), mode)
}

// sel_damage_outline(doc) -- redraw the marching ants (edge by edge for small outlines)
fn sel_damage_outline(p: &mut Pn, doc: DocId) {
    let s = &p.doc.d(doc).sel;
    if s.obox[2] <= s.obox[0] {
        return;
    }
    let dmg = &mut p.sel.dmg;
    match s.outl.as_ref().filter(|_| s.state) {
        Some(o) if o.pts.len() <= ANT_EDGES => {
            let mut k = 0usize;
            for &n in &o.cnt {
                let n = n as usize;
                for i in 0..n {
                    let j = if i + 1 < n { i + 1 } else { 0 };
                    let (a, b) = (o.pts[k + i], o.pts[k + j]);
                    let (x0, x1) = (minss(a[0], b[0]), maxss(a[0], b[0]));
                    let (y0, y1) = (minss(a[1], b[1]), maxss(a[1], b[1]));
                    dmg.push((doc, [cvtt(x0) - 1, cvtt(y0) - 1, cvtt(x1) + 2, cvtt(y1) + 2]));
                }
                k += n;
            }
        }
        _ => dmg.push((doc, [s.obox[0] - 1, s.obox[1] - 1, s.obox[2] + 1, s.obox[3] + 1])),
    }
}

/// sel_render(doc) -- replay the ops (+ preview, pending transform) into the mask, bounds,
/// outline, damage of the old and the new outline, DOC_VER_SEL++
fn sel_render(p: &mut Pn, doc: DocId) {
    sel_damage_outline(p, doc);
    {
        let Pn { sel, doc: docs, .. } = p;
        let (path, ras) = (&mut sel.path, &mut sel.ras);
        let d = docs.dm(doc);
        let (dw, dh) = (d.w, d.h);
        let s = &mut d.sel;
        s.obox = [0; 4];
        if !s.state {
            nosel(s);
        } else {
            s.outl = None;
            s.nops = s.ops.len() as u32;
            s.flags &= SF_XF;
            let show = if s.pv.is_some() {
                s.flags |= SF_PREVIEW;
                true
            } else {
                s.nops != 0
            };
            if !show {
                // nothing to show: no mask
                s.keep = None;
                s.mask = None;
                s.tmp = None;
                nosel(s);
            } else {
                // a mask of the image size
                if let Some(m) = s.mask.take() {
                    s.keep = Some(m);
                }
                if s.keep.as_ref().is_some_and(|m| m.w != dw as u32 || m.h != dh as u32) {
                    s.keep = None;
                }
                if s.keep.is_none() {
                    s.keep = TMask::new(dw as u32, dh as u32).map(Box::new);
                }
                if s.keep.is_none() {
                    s.bbox = [0; 4]; // out of memory
                } else {
                    render_ops(path, ras, s);
                }
            }
        }
    }
    sel_damage_outline(p, doc);
    let d = p.doc.dm(doc);
    d.ver_sel = d.ver_sel.wrapping_add(1);
    p.repass();
}

// no selection: no mask, no bounds
fn nosel(s: &mut DocSel) {
    s.mask = None;
    s.bbox = [0; 4];
    s.nops = 0;
    s.flags = 0;
}

// the replay of sel_render into s.keep, then mask, bounds and outline
fn render_ops(path: &mut Path, ras: &mut Raster, s: &mut DocSel) {
    // replay from the last replacing op
    let mut start = s.ops.iter().rposition(|o| o.mode == M_REPLACE).unwrap_or(0);
    if s.pv.as_ref().is_some_and(|v| v.mode == M_REPLACE) {
        start = s.ops.len();
    }
    let mut rendered = 0;
    let mut lastq = Q_AA;
    let mut lastp: Option<PolyRef> = None;
    let mut i = start;
    loop {
        let op = if i < s.ops.len() {
            s.ops[i].clone()
        } else if i == s.ops.len() && s.pv.is_some() {
            s.pv.clone().unwrap()
        } else {
            break;
        };
        i += 1;
        if op.feather!=0 {
            // (no memory: the selection keeps its hard edge)
            if !feather_mask(s.keep.as_mut().unwrap(),op.feather as i32) {
                plat::log_warn("selection", "feather: out of memory, edge not softened");
            }
            rendered+=1;lastq=0;continue;
        }
        let mut mode = op.mode;
        if rendered == 0 {
            // the first op: combining with nothing
            if mode == M_EXCLUDE || mode == M_INTERSECT {
                s.keep.as_mut().unwrap().clear(0);
                rendered = 2; // (a trace is needed)
                continue;
            }
            mode = M_REPLACE;
        }
        render_op(path, ras, s, &op.poly, mode, op.qual);
        rendered += 1;
        lastq = op.qual;
        lastp = Some(op.poly);
    }
    let m = s.keep.take().unwrap();
    let mut m = m;
    if rendered == 0 {
        m.clear(0);
    }
    s.bbox = m.bbox().unwrap_or([0; 4]);
    // outline: the polygon of a single antialiased op, else the traced mask
    let o = if rendered == 1 && lastq == Q_AA {
        let lp = lastp.unwrap();
        if s.flags & SF_XF != 0 { sel_poly_xform(&lp, &s.xf) } else { Some(lp) }
    } else {
        sel_poly_trace(&m)
    };
    s.mask = Some(m);
    if let Some(o) = &o {
        if !o.pts.is_empty() {
            let bb = sel_poly_bbox(Some(o));
            s.obox = [floor_i(bb[0]), floor_i(bb[1]), floor_i(bb[2]) + 1, floor_i(bb[3]) + 1];
        }
    }
    s.outl = o;
}

// ================================================================== changing the selection

// Separable box filter of A8 coverage, with zero coverage outside the canvas.
// Rolling sums keep work linear in pixel count even for a large feather radius.
// -> false (mask unchanged) when the work buffers do not fit into memory.
fn feather_mask(mask:&mut TMask,r:i32) -> bool {
    let (w,h)=(mask.w as usize,mask.h as usize);
    let n=(2*r+1) as u32;
    let Some(mut rows)=w.checked_mul(h).and_then(gfx::try_zeroed) else {return false};
    for y in 0..h {
        let mut sum=0u32;
        for x in -r..=r {sum+=mask.get(x,y as i32) as u32;}
        for x in 0..w {
            rows[y*w+x]=((sum+n/2)/n) as u8;
            sum-=mask.get(x as i32-r,y as i32) as u32;
            sum+=mask.get(x as i32+r+1,y as i32) as u32;
        }
    }
    let mut sums=vec![0u32;w];
    for y in 0..=(r as usize).min(h-1) {for x in 0..w {sums[x]+=rows[y*w+x] as u32;}}
    mask.clear(0);
    for y in 0..h {
        let mut x=0;
        while x<w {
            let len=(64-x%64).min(w-x);
            if sums[x..x+len].iter().any(|v|*v>=n/2) {
                if let Some(dst)=mask.row_w(x as i32,y as i32) {for (i,v) in dst.iter_mut().take(len).enumerate(){*v=((sums[x+i]+n/2)/n) as u8;}}
            }
            x+=len;
        }
        for x in 0..w {
            if y>=r as usize {sums[x]-=rows[(y-r as usize)*w+x] as u32;}
            if y+r as usize+1<h {sums[x]+=rows[(y+r as usize+1)*w+x] as u32;}
        }
    }
    mask.optimize();
    true
}

/// sel_combine(doc, poly, mode, qual) -> true -- append an op (SM_REPLACE drops the others);
/// an empty result = no selection.  The caller keeps its reference to poly.
pub(crate) fn sel_combine(p: &mut Pn, doc: DocId, poly: &PolyRef, mode: u32, qual: u32) -> bool {
    ss_get(p, doc);
    sel_bake(p, doc);
    let s = &mut p.doc.dm(doc).sel;
    if mode == M_REPLACE {
        ss_drop_ops(s);
    } else if s.ops.is_empty() && (mode == M_EXCLUDE || mode == M_INTERSECT) {
        // no selection yet: Subtrahieren / Schneiden give nothing
        sel_render(p, doc);
        return true;
    }
    p.doc.dm(doc).sel.ops.push(Op { poly: poly.clone(), mode, qual, feather: 0 });
    sel_render(p, doc);
    let s = &p.doc.d(doc).sel;
    if s.bbox[2] <= s.bbox[0] {
        sel_clear(p, doc);
    }
    true
}

// ss_get(doc): the state record (created on first use)
fn ss_get(p: &mut Pn, doc: DocId) {
    let s = &mut p.doc.dm(doc).sel;
    if !s.state {
        s.state = true;
        s.ops.clear();
        s.xf = Matrix::IDENTITY;
    }
}

/// sel_apply(doc, pts, mode, qual) -> true -- sel_combine of one contour
pub(crate) fn sel_apply(p: &mut Pn, doc: DocId, pts: &[[f32; 2]], mode: u32, qual: u32) -> bool {
    match sel_poly_new(pts) {
        Some(poly) => sel_combine(p, doc, &poly, mode, qual),
        None => false,
    }
}

/// sel_set_rect(doc, x0, y0, x1, y1) -> true -- replace by a rectangle
pub(crate) fn sel_set_rect(p: &mut Pn, doc: DocId, x0: i32, y0: i32, x1: i32, y1: i32) -> bool {
    match sel_poly_rect(x0, y0, x1, y1) {
        Some(poly) => sel_combine(p, doc, &poly, M_REPLACE, Q_AA),
        None => false,
    }
}

/// sel_set_ellipse(doc, x0, y0, x1, y1) -> true -- replace by an ellipse
pub(crate) fn sel_set_ellipse(p: &mut Pn, doc: DocId, x0: i32, y0: i32, x1: i32, y1: i32) -> bool {
    match sel_poly_ellipse(x0, y0, x1, y1) {
        Some(poly) => sel_combine(p, doc, &poly, M_REPLACE, Q_AA),
        None => false,
    }
}

/// sel_edit_last(doc, poly) -> true -- the last op gets the geometry poly
pub(crate) fn sel_edit_last(p: &mut Pn, doc: DocId, poly: &PolyRef) -> bool {
    let s = &mut p.doc.dm(doc).sel;
    if !s.state {
        return false;
    }
    let Some(op) = s.ops.last_mut() else { return false };
    op.poly = poly.clone();
    sel_render(p, doc);
    true
}

/// sel_set_preview(doc, poly, mode, qual) -- shown / clipped op of a drag in progress
/// (None = none)
pub(crate) fn sel_set_preview(p: &mut Pn, doc: DocId, poly: Option<&PolyRef>, mode: u32, qual: u32) {
    match poly {
        None => {
            let s = &mut p.doc.dm(doc).sel;
            if !s.state || s.pv.is_none() {
                return;
            }
            s.pv = None;
        }
        Some(pl) => {
            ss_get(p, doc);
            p.doc.dm(doc).sel.pv = Some(Op { poly: pl.clone(), mode, qual, feather: 0 });
        }
    }
    sel_render(p, doc);
}

// ss_drop_ops(state) -- release every op and the preview
fn ss_drop_ops(s: &mut DocSel) {
    s.ops.clear();
    s.pv = None;
}

/// sel_clear(doc) -- no selection
pub(crate) fn sel_clear(p: &mut Pn, doc: DocId) {
    let s = &mut p.doc.dm(doc).sel;
    if !s.state {
        return;
    }
    ss_drop_ops(s);
    s.flags &= !SF_XF;
    s.xf = Matrix::IDENTITY;
    sel_render(p, doc);
}

/// sel_set_xform(doc, m | None) -- pending transform of the whole selection (move tools)
pub(crate) fn sel_set_xform(p: &mut Pn, doc: DocId, m: Option<&Matrix>) {
    let s = &mut p.doc.dm(doc).sel;
    if !s.state {
        return;
    }
    match m {
        Some(m) => {
            s.xf = *m;
            s.flags |= SF_XF;
        }
        None => {
            s.flags &= !SF_XF;
            s.xf = Matrix::IDENTITY;
        }
    }
    sel_render(p, doc);
}

/// sel_bake(doc) -- the pending transform becomes part of the ops (new polygons; the old ones
/// may still be used by history steps)
pub(crate) fn sel_bake(p: &mut Pn, doc: DocId) {
    let s = &mut p.doc.dm(doc).sel;
    if s.flags & SF_XF == 0 || !s.state {
        return;
    }
    let m = s.xf;
    for op in s.ops.iter_mut() {
        if let Some(np) = sel_poly_xform(&op.poly, &m) {
            op.poly = np; // out of memory: keep the old geometry
        }
    }
    if let Some(v) = s.pv.as_mut() {
        if let Some(np) = sel_poly_xform(&v.poly, &m) {
            v.poly = np;
        }
    }
    s.flags &= !SF_XF;
    s.xf = Matrix::IDENTITY;
}

/// sel_translate(doc, dx, dy) -- move every op by whole pixels (baked)
pub(crate) fn sel_translate(p: &mut Pn, doc: DocId, dx: i32, dy: i32) {
    if p.doc.d(doc).sel.nops == 0 {
        return;
    }
    let m = Matrix { c: dx as f64, f: dy as f64, ..Matrix::IDENTITY };
    sel_set_xform(p, doc, Some(&m));
    sel_bake(p, doc);
}

// ================================================================== history hooks

/// sel_hsnap(doc) -> blob | None (no selection)
pub(crate) fn sel_hsnap(d: &Doc) -> Option<Box<SelBlob>> {
    let s = &d.sel;
    if !s.state || s.ops.is_empty() {
        return None;
    }
    Some(Box::new(SelBlob { flags: s.flags & SF_XF, xf: s.xf, ops: s.ops.clone() }))
}

/// sel_hrestore(doc, blob) -- blob (None = no selection) becomes the selection; blob stays
pub(crate) fn sel_hrestore(p: &mut Pn, doc: DocId, blob: Option<&SelBlob>) {
    if blob.is_some() {
        ss_get(p, doc);
    }
    let s = &mut p.doc.dm(doc).sel;
    if !s.state {
        return;
    }
    ss_drop_ops(s);
    s.flags &= !SF_XF;
    s.xf = Matrix::IDENTITY;
    if let Some(b) = blob {
        s.ops = b.ops.clone();
        if b.flags & SF_XF != 0 {
            s.xf = b.xf;
            s.flags |= SF_XF;
        }
    }
    sel_render(p, doc);
}

/// sel_hfree(blob)
pub(crate) fn sel_hfree(_blob: Box<SelBlob>) {}

/// sel_hsize(blob) -> bytes (blob + its polygons, shared ones counted fully)
pub(crate) fn sel_hsize(b: &SelBlob) -> u64 {
    64 + b.ops.len() as u64 * core::mem::size_of::<Op>() as u64 + b.ops.iter().map(|o| o.poly.bytes()).sum::<u64>()
}

// ================================================================== pixels

/// sel_erase(doc) -> step -- "Auswahlinhalt löschen": alpha *= 1 - coverage
pub(crate) fn sel_erase(p: &mut Pn, doc: DocId) -> bool {
    sel_px_op(p, doc, 0, 0, "Auswahlinhalt löschen", IC_DELETE)
}

/// sel_fill(doc, argb) -> step -- "Auswahl füllen": argb (Normal) through the coverage
pub(crate) fn sel_fill(p: &mut Pn, doc: DocId, argb: u32) -> bool {
    sel_px_op(p, doc, 1, argb, "Auswahl füllen", IC_DROP)
}

// sel_px_op(doc, kind, argb, name, icon) -> step -- kind 0 erase, 1 fill; the tiles with
// coverage are saved (hist_px_touch), the rows are written on the worker pool
fn sel_px_op(p: &mut Pn, doc: DocId, kind: u32, argb: u32, name: &'static str, icon: u32) -> bool {
    let b = {
        let d = p.doc.d(doc);
        if d.sel.mask.is_none() {
            return false;
        }
        let Some(l) = doc_cur_layer(d) else { return false };
        if l.sf.px.is_empty() {
            return false;
        }
        // the box: selection bounds clipped to the layer
        let s = &d.sel;
        [s.bbox[0], s.bbox[1], s.bbox[2].min(l.sf.w), s.bbox[3].min(l.sf.h)]
    };
    if b[2] <= b[0] || b[3] <= b[1] {
        return false;
    }
    if hist_px_begin(p, doc, -1) != 0 {
        return false;
    }
    // save the tiles the selection covers
    let ty0 = (b[1] as u32 >> TILE_SH) as i32;
    let mut ty = ty0;
    while ty << TILE_SH < b[3] {
        let mut tx = (b[0] as u32 >> TILE_SH) as i32;
        while tx << TILE_SH < b[2] {
            let empty = {
                let m = p.doc.d(doc).sel.mask.as_deref().unwrap();
                matches!(m.span(tx << TILE_SH, ty << TILE_SH, 1).0, Span::Empty)
            };
            if !empty {
                let r = hist_px_touch(p, doc, (tx << TILE_SH).max(b[0]), (ty << TILE_SH).max(b[1]), ((tx + 1) << TILE_SH).min(b[2]), ((ty + 1) << TILE_SH).min(b[3]));
                if r != 0 {
                    hist_px_cancel(p, doc);
                    return false;
                }
            }
            tx += 1;
        }
        ty += 1;
    }
    // one work item per tile row
    let cur = {
        let d = p.doc.dm(doc);
        let cur = d.cur.clamp(0, d.layers.len() as i32 - 1);
        let Doc { sel, layers, .. } = d;
        let l = &mut layers[cur as usize];
        let stride = l.sf.stride as usize;
        let ty1 = ((b[3] - 1) as u32 >> TILE_SH) as i32;
        let mut bands: Vec<SpinLock<(i32, i32, &mut [u8])>> = Vec::new();
        let mut rest: &mut [u8] = &mut l.sf.px[b[1] as usize * stride..];
        for t in ty0..=ty1 {
            let y0 = (t << TILE_SH).max(b[1]);
            let y1 = ((t + 1) << TILE_SH).min(b[3]);
            let take = ((y1 - y0) as usize * stride).min(rest.len());
            let (a, r) = core::mem::take(&mut rest).split_at_mut(take);
            bands.push(SpinLock::new((y0, y1, a)));
            rest = r;
        }
        let sel: &DocSel = sel;
        let item = |i: usize, _: &plat::JobCtl| {
            let mut g = bands[i].lock();
            let (y0, y1, ref mut rows) = *g;
            let mut cov = [0u8; 64];
            for y in y0..y1 {
                let row = &mut rows[(y - y0) as usize * stride..];
                let mut x = b[0];
                while x < b[2] {
                    let n = (b[2] - x).min(64);
                    let c = clip_row(sel, x, y, n, &mut cov);
                    if c != SCR_NONE {
                        let px = &mut row[x as usize * 4..x as usize * 4 + n as usize * 4];
                        if kind == 0 {
                            if c == SCR_ALL {
                                cov[..n as usize].fill(255);
                            }
                            erase_row(px, &cov[..n as usize], 255);
                        } else {
                            let cv = if c == SCR_ALL { None } else { Some(&cov[..n as usize]) };
                            bm_solid_row_inplace(px, argb, cv, n as usize, BM_NORMAL | 255 << BM_OPAC_SHIFT);
                        }
                    }
                    x += n;
                }
            }
        };
        let _ = plat::parallel_for(bands.len(), &item);
        cur
    };
    doc_mark_px(p, doc, cur, b[0], b[1], b[2], b[3]);
    hist_px_end(p, doc, name, icon)
}

/// sel_crop_enabled() -> true when "Auf Auswahl zuschneiden" applies to the active image
pub(crate) fn sel_crop_enabled(p: &Pn) -> bool {
    match doc_active(&p.doc).map(|d| &p.doc.d(d).sel) {
        Some(s) => s.nops != 0 && s.bbox[2] > s.bbox[0],
        None => false,
    }
}

// sel_is_rect(doc) -> true when every pixel of the selection bounds is fully selected
fn sel_is_rect(s: &DocSel) -> bool {
    let m = s.mask.as_deref().unwrap();
    for y in s.bbox[1]..s.bbox[3] {
        let mut x = s.bbox[0];
        while s.bbox[2] - x > 0 {
            let (sp, k) = m.span(x, y, s.bbox[2] - x);
            if k == 0 {
                return false;
            }
            match sp {
                Span::Full => {}
                Span::Empty => return false,
                Span::Bytes(b) => {
                    if b[..k as usize].iter().any(|&v| v != 255) {
                        return false;
                    }
                }
            }
            x += k;
        }
    }
    true
}

/// sel_crop(doc) -> step -- "Auf Auswahl zuschneiden": the image becomes the selection
/// bounds; pixels outside a non-rectangular selection keep coverage * alpha; the selection
/// moves along.  One compound step (selection + document).
pub(crate) fn sel_crop(p: &mut Pn, doc: DocId) -> bool {
    let (w, h, rect, b) = {
        let d = p.doc.d(doc);
        let s = &d.sel;
        if s.nops == 0 || s.mask.is_none() {
            return false;
        }
        let (w, h) = (s.bbox[2] - s.bbox[0], s.bbox[3] - s.bbox[1]);
        if w <= 0 || h <= 0 {
            return false;
        }
        let rect = sel_is_rect(s);
        if rect && w == d.w && h == d.h {
            return false; // the whole image: nothing to do
        }
        (w, h, rect, s.bbox)
    };
    // new surfaces with the pixels of the bounds
    let mut nsf: Vec<Surface> = Vec::new();
    {
        let d = p.doc.d(doc);
        for l in &d.layers {
            let Some(mut sf) = surf_try(w, h) else { return false };
            sf.opac = l.sf.opac;
            sf.blend = l.sf.blend;
            if !l.sf.px.is_empty() {
                let (ds, ss) = (sf.stride as usize, l.sf.stride as usize);
                let mut cov = [0u8; 64];
                for r in 0..h as usize {
                    let so = (b[1] as usize + r) * ss + b[0] as usize * 4;
                    let drow = &mut sf.px[r * ds..r * ds + w as usize * 4];
                    drow.copy_from_slice(&l.sf.px[so..so + w as usize * 4]);
                    if rect {
                        continue;
                    }
                    // not rectangular: alpha *= coverage
                    let mut c = 0;
                    while c < w {
                        let n = (w - c).min(64);
                        match clip_row(&d.sel, b[0] + c, b[1] + r as i32, n, &mut cov) {
                            SCR_ALL => {
                                c += n;
                                continue;
                            }
                            SCR_PART => {
                                for v in cov[..n as usize].iter_mut() {
                                    *v = !*v;
                                }
                            }
                            _ => cov[..n as usize].fill(255), // none: erase completely
                        }
                        erase_row(&mut drow[c as usize * 4..(c + n) as usize * 4], &cov[..n as usize], 255);
                        c += n;
                    }
                }
            }
            nsf.push(sf);
        }
    }
    hist_begin_compound(p, doc);
    if !hist_push_sel(p, doc, "Auf Auswahl zuschneiden", IC_CROP) || !hist_push_doc(p, doc, "Auf Auswahl zuschneiden", IC_CROP) {
        hist_end_compound(p, doc, "Auf Auswahl zuschneiden", IC_CROP);
        return false;
    }
    // install the new surfaces (the old ones: history or freed)
    for (i, sf) in nsf.into_iter().enumerate() {
        let l = &mut p.doc.dm(doc).layers[i];
        let old = core::mem::replace(&mut l.sf, sf);
        l.ver = l.ver.wrapping_add(1);
        hist_release(p, doc, Layer { sf: old, ..Layer::default() });
    }
    let d = p.doc.dm(doc);
    d.w = w;
    d.h = h;
    sel_translate(p, doc, -b[0], -b[1]);
    doc_resized(p, doc);
    hist_end_compound(p, doc, "Auf Auswahl zuschneiden", IC_CROP);
    true
}

/// a zeroed FMT_ARGB surface (surf_alloc) | None when out of memory
pub(crate) fn surf_try(w: i32, h: i32) -> Option<Surface> {
    Surface::try_new(w, h, FMT_ARGB)
}

// ================================================================== marching ants

// sel_ant_tick() -- next dash phase, redraw the outline, restart the timer
fn sel_ant_tick(p: &mut Pn, ui: &mut Ui) {
    p.sel.ant_on = false;
    let Some(doc) = doc_active(&p.doc) else { return };
    let s = &p.doc.d(doc).sel;
    if s.obox[2] <= s.obox[0] {
        return;
    }
    p.sel.ant_phase = p.sel.ant_phase.wrapping_add(1);
    sel_damage_outline(p, doc);
    if !anim_off() {
        p.sel.ant_on = true;
        ui.timer(TMR_ANTS, ANT_MS);
    }
}

/// the drawing state of sel_overlay / sel_line (the asm's L_* locals block)
struct Seg<'a> {
    fb: &'a mut [u8],
    stride: usize,
    /// gfx clip x0, y0, x1, y1
    clip: [i32; 4],
    dash: u32,
    period: u32,
    phase: u32,
    /// pixels of the contour done so far (the dash continues along the contour)
    done: u32,
    c0: u32,
    c1: u32,
}

// a / b rounded toward minus infinity (the asm's idiv + correction of negative remainders)
#[inline]
fn fdiv(a: i64, b: i64) -> i64 {
    let q = a / b;
    if a % b < 0 { q - 1 } else { q }
}

// ov_segment -- one edge (X0, Y0) -> (X1, Y1): pixels k = 0 .. steps-1 inside the clip, the
// dash phase continues along the contour
fn ov_segment(s: &mut Seg, x0: i32, y0: i32, x1: i32, y1: i32) {
    let dx = x1.wrapping_sub(x0);
    let dy = y1.wrapping_sub(y0);
    let steps = dx.wrapping_abs().max(dy.wrapping_abs());
    if steps == 0 {
        return;
    }
    let (bx0, bx1) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
    let (by0, by1) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
    if !(bx1 < s.clip[0] || bx0 >= s.clip[2] || by1 < s.clip[1] || by0 >= s.clip[3]) {
        // k range [k0, k1) from the clip: axis-aligned edges exactly, others by scanning
        let (mut k0, mut k1) = (0i32, steps);
        if dy == 0 {
            let (mut a, mut b) = (s.clip[0].wrapping_sub(x0), s.clip[2].wrapping_sub(x0));
            if dx < 0 {
                (a, b) = (b.wrapping_neg().wrapping_add(1), a.wrapping_neg().wrapping_add(1));
            }
            k0 = k0.max(a);
            k1 = k1.min(b);
        } else if dx == 0 {
            let (mut a, mut b) = (s.clip[1].wrapping_sub(y0), s.clip[3].wrapping_sub(y0));
            if dy < 0 {
                (a, b) = (b.wrapping_neg().wrapping_add(1), a.wrapping_neg().wrapping_add(1));
            }
            k0 = k0.max(a);
            k1 = k1.min(b);
        }
        // pixel k: x = X0 + round(dx k / steps), y likewise; colour by the dash position
        let st2 = steps as i64 * 2;
        for k in k0..k1 {
            let x = (fdiv(k as i64 * dx as i64 * 2 + steps as i64, st2) as i32).wrapping_add(x0);
            let y = (fdiv(k as i64 * dy as i64 * 2 + steps as i64, st2) as i32).wrapping_add(y0);
            if x < s.clip[0] || x >= s.clip[2] || y < s.clip[1] || y >= s.clip[3] {
                continue;
            }
            let o = y as usize * s.stride + x as usize * 4;
            let pos = s.phase.wrapping_add(s.done).wrapping_add(k as u32) % s.period;
            let c = if pos < s.dash { s.c0 } else { s.c1 };
            s.fb[o..o + 4].copy_from_slice(&c.to_le_bytes());
        }
    }
    s.done = s.done.wrapping_add(steps as u32);
}

// ov_screen(point) -> floor(o + p * z) per axis
fn ov_screen(q: [f32; 2], ox: f64, oy: f64, z: f64) -> (i32, i32) {
    let f = |v: f64| {
        let v = minsd(maxsd(v, -1073741824.0), 1073741824.0);
        let i = cvttd(v);
        if (i as f64) > v { i - 1 } else { i }
    };
    (f(q[0] as f64 * z + ox), f(q[1] as f64 * z + oy))
}

/// sel_overlay(ctx, doc, &canvas_rect) -- marching ants (render time): every outline edge as
/// 1 px black / white dashes directly in the framebuffer, clipped to the gfx clip
fn sel_overlay(p: &mut Pn, g: &mut Gfx, _ctx: usize, doc: DocId, _cr: [i32; 4]) {
    let Some(d) = p.doc.get(doc) else { return };
    if !d.sel.state {
        return;
    }
    let Some(o) = d.sel.outl.as_ref() else { return };
    if o.pts.is_empty() || g.sf.px.is_empty() {
        return;
    }
    let clip = g.dc.clip;
    if clip[2] <= clip[0] || clip[3] <= clip[1] {
        return;
    }
    // transform: screen = o + p * z
    let mut a = [0.0f64, 0.0];
    view_doc_to_screen_f64(&p.view, &mut a);
    let mut b = [1024.0f64, 1024.0];
    view_doc_to_screen_f64(&p.view, &mut b);
    let z = (b[0] - a[0]) / 1024.0;
    let dash = if p.sel.dash > 0 { p.sel.dash as u32 } else { 1 };
    let period = dash * 2;
    let phase = period - p.sel.ant_phase % period; // marching: the pattern moves forward
    let stride = g.sf.stride as usize;
    let mut s = Seg { fb: &mut g.sf.px[..], stride, clip, dash, period, phase, done: 0, c0: 0xFF00_0000, c1: 0xFFFF_FFFF };
    let mut k = 0usize;
    for &n in &o.cnt {
        let n = n as usize;
        s.done = 0;
        if n != 0 {
            // start: the last point of the contour
            let (mut x0, mut y0) = ov_screen(o.pts[k + n - 1], a[0], a[1], z);
            for i in 0..n {
                let (x1, y1) = ov_screen(o.pts[k + i], a[0], a[1], z);
                ov_segment(&mut s, x0, y0, x1, y1);
                (x0, y0) = (x1, y1);
            }
        }
        k += n;
    }
}

/// sel_line(x0, y0, x1, y1, argb) -- render time: an aliased 1 px line between the screen
/// pixels (x0, y0) and (x1, y1) (the last one excluded), argb written directly into the target
/// (opaque), clipped to the gfx clip
pub(crate) fn sel_line(g: &mut Gfx, x0: i32, y0: i32, x1: i32, y1: i32, argb: u32) {
    if g.sf.px.is_empty() {
        return;
    }
    let stride = g.sf.stride as usize;
    let clip = g.dc.clip;
    let mut s = Seg { fb: &mut g.sf.px[..], stride, clip, dash: 1, period: 2, phase: 0, done: 0, c0: argb, c1: argb };
    ov_segment(&mut s, x0, y0, x1, y1);
}

// ================================================================== selection geometry as text
// (Auswahl kopieren / Auswahl einfügen of the clipboard module)
// Format (UTF-8 lines): "PaintCraft Auswahl 1", then one line per contour "p N x0 y0 x1 y1 ..."
// (N points in 1/256 image px, closed implicitly, even-odd).  The geometry is the single
// antialiased op itself (lossless), else the traced coverage (pixel edges).

// gt_itoa(dst, value) -- the decimal digits of a signed value
fn gt_itoa(out: &mut String, v: i32) {
    let mut u = v as u32;
    if v < 0 {
        out.push('-');
        u = (v as u32).wrapping_neg();
    }
    let mut d = [0u8; 10];
    let mut n = 0;
    loop {
        d[n] = b'0' + (u % 10) as u8;
        n += 1;
        u /= 10;
        if u == 0 || n >= 10 {
            break;
        }
    }
    for i in (0..n).rev() {
        out.push(d[i] as char);
    }
}

/// sel_geom_text(doc) -> text | None
pub(crate) fn sel_geom_text(d: &Doc) -> Option<String> {
    let s = &d.sel;
    if !s.state || s.ops.is_empty() {
        return None;
    }
    let poly = if s.ops.len() == 1 && s.ops[0].qual == Q_AA {
        if s.flags & SF_XF != 0 { sel_poly_xform(&s.ops[0].poly, &s.xf)? } else { s.ops[0].poly.clone() }
    } else {
        sel_poly_trace(s.mask.as_deref()?)?
    };
    let mut t = String::with_capacity(poly.pts.len() * 26 + poly.cnt.len() * 16 + 64);
    t.push_str(GEOM_HDR);
    t.push('\n');
    let mut k = 0usize;
    for &n in &poly.cnt {
        t.push_str("p ");
        gt_itoa(&mut t, n as i32);
        for j in 0..n as usize {
            let q = poly.pts[k + j];
            t.push(' ');
            gt_itoa(&mut t, cvt(q[0] * 256.0));
            t.push(' ');
            gt_itoa(&mut t, cvt(q[1] * 256.0));
        }
        t.push('\n');
        k += n as usize;
    }
    Some(t)
}

// parse_int(s, i) -> (value, position past the digits): leading blanks, optional sign, i32
// wrapping like the asm's parse_int
fn parse_int(s: &[u8], mut i: usize) -> (i32, usize) {
    let at = |k: usize| s.get(k).copied().unwrap_or(0);
    while at(i) == b' ' {
        i += 1;
    }
    let mut neg = false;
    if at(i) == b'-' {
        neg = true;
        i += 1;
    } else if at(i) == b'+' {
        i += 1;
    }
    let mut v = 0i32;
    while at(i).wrapping_sub(b'0') <= 9 {
        v = v.wrapping_mul(10).wrapping_add((at(i) - b'0') as i32);
        i += 1;
    }
    (if neg { v.wrapping_neg() } else { v }, i)
}

const GEOM_POINTS_MAX: usize = 4_000_000;

/// sel_geom_parse(text) -> POLY of the text of sel_geom_text | None (not that format)
pub(crate) fn sel_geom_parse(text: &[u8]) -> Option<PolyRef> {
    let t = &text[..text.iter().position(|&c| c == 0).unwrap_or(text.len())]; // a NUL ends it
    if !t.starts_with(GEOM_HDR.as_bytes()) {
        return None;
    }
    let at = |k: usize| t.get(k).copied().unwrap_or(0);
    let mut b = Builder::default();
    let mut buf: Vec<[f32; 2]> = Vec::new();
    let mut total = 0usize;
    let mut i = GEOM_HDR.len();
    loop {
        while matches!(at(i), b' ' | b'\t' | b'\r' | b'\n') {
            i += 1;
        }
        match at(i) {
            0 => break,
            b'p' => i += 1,
            _ => return None,
        }
        let (n, j) = parse_int(t, i);
        if j == i || n <= 0 || n > 0x100_0000 {
            return None;
        }
        total += n as usize;
        if total > GEOM_POINTS_MAX {
            return None; // pasted text must not build an unbounded outline
        }
        i = j;
        buf.clear();
        for _ in 0..n {
            let (x, j) = parse_int(t, i);
            if j == i {
                return None;
            }
            let (y, j2) = parse_int(t, j);
            if j2 == j {
                return None;
            }
            i = j2;
            buf.push([x as f32 * 0.00390625, y as f32 * 0.00390625]);
        }
        b.contour_f(&buf);
    }
    b.finish()
}

// ================================================================== test hooks (diag.rs block T2)
// cmd 980 x | y << 16    coverage probe (sel_clip_row of one pixel) -> #9980 value 0..255,
//                        text "none" (0) / "part" (1..254) / "all" (255)
// cmd 981 op             0 rectangle, 1 ellipse (982 / 983 corners, 984 mode | qual << 8),
//                        each with history "Test"; 2 deselect without history; 3 fill the
//                        selection with the primary colour (history); 4 erase;
//                        5 crop; 6 ants phase + 1 (redraw); 7 invert (history);
//                        8 count the partially covered pixels -> #9980 value, text "count";
//                        9 the geometry through sel_geom_text / sel_geom_parse (Ersetzen)
// cmd 982 x0 | y0 << 16  first corner            cmd 983 x1 | y1 << 16  second corner
// cmd 984 mode | qual << 8
// Records: #9980 probe  #9981 text "x0,y0,x1,y1" (bounds), value ops  #9982 text outline
// "c,p" (contours, points), value SEL_FLAGS  #9983 text tool state (rectangle / ellipse
// "x0,y0,x1,y1" while its handles apply, magic wand "x,y" origin), value DOC_VER_SEL  #9984
// value tmove state (t_move.rs), text its transform "tx,ty,sx,sy,deg" (rounded)

/// sel_diag_cmd(id, arg) -> true
pub(crate) fn sel_diag_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    match id {
        x if x == CMD_T2_TEST0 + 2 => p.sel.dg_p0 = arg,
        x if x == CMD_T2_TEST0 + 3 => p.sel.dg_p1 = arg,
        x if x == CMD_T2_TEST0 + 4 => p.sel.dg_mode = arg,
        _ => {
            let Some(doc) = doc_active(&p.doc) else { return true };
            if id == CMD_T2_TEST0 {
                let mut buf = [0u8; 64];
                let c = clip_row(&p.doc.d(doc).sel, (arg & 0xFFFF) as i32, (arg >> 16) as i32, 1, &mut buf);
                let v = match c {
                    SCR_NONE => 0,
                    SCR_ALL => 255,
                    _ => buf[0] as u32,
                };
                p.sel.dg_cov = v;
                p.sel.dg_res = match v {
                    0 => "none",
                    255 => "all",
                    _ => "part",
                };
                ui.repass();
            } else if id == CMD_T2_TEST0 + 1 {
                sel_diag_op(p, ui, doc, arg);
            }
        }
    }
    sel_flush(p, ui);
    true
}

// sel_diag_cmd 981 op
fn sel_diag_op(p: &mut Pn, ui: &mut Ui, doc: DocId, op: u32) {
    match op {
        0 | 1 => {
            if !hist_push_sel(p, doc, "Test", IC_SELECT_RECT) {
                return;
            }
            let (a, b) = (p.sel.dg_p0, p.sel.dg_p1);
            let (x0, y0, x1, y1) = ((a & 0xFFFF) as i32, (a >> 16) as i32, (b & 0xFFFF) as i32, (b >> 16) as i32);
            let poly = if op == 0 { sel_poly_rect(x0, y0, x1, y1) } else { sel_poly_ellipse(x0, y0, x1, y1) };
            if let Some(poly) = poly {
                let m = p.sel.dg_mode;
                sel_combine(p, doc, &poly, m & 255, (m >> 8) & 255);
            }
        }
        2 => sel_clear(p, doc),
        3 => {
            let c = pal_primary(&p.pal);
            sel_fill(p, doc, c);
        }
        4 => {
            sel_erase(p, doc);
        }
        5 => {
            sel_crop(p, doc);
        }
        6 => sel_ant_tick(p, ui),
        7 => {
            sel_cmd(p, ui, CMD_INVERT_SEL, 0);
        }
        8 => {
            // pixels with a partial coverage (1..254) -> #9980 value, text "count"
            let mut n = 0u32;
            if let Some(m) = p.doc.d(doc).sel.mask.as_deref() {
                for y in 0..m.h as i32 {
                    let mut x = 0;
                    while (m.w as i32) - x > 0 {
                        let (sp, k) = m.span(x, y, m.w as i32 - x);
                        if k == 0 {
                            break;
                        }
                        x += k;
                        if let Span::Bytes(b) = sp {
                            n += b[..k as usize].iter().filter(|&&v| v.wrapping_sub(1) <= 253).count() as u32;
                        }
                    }
                }
            }
            p.sel.dg_cov = n;
            p.sel.dg_res = "count";
            ui.repass();
        }
        9 => {
            // the geometry through its clipboard text and back (Ersetzen, no history)
            let Some(t) = sel_geom_text(p.doc.d(doc)) else { return };
            if let Some(poly) = sel_geom_parse(t.as_bytes()) {
                sel_combine(p, doc, &poly, M_REPLACE, Q_AA);
            }
        }
        _ => {}
    }
}

// dg_rec(id, text, value) -- hidden K_TEXT record (empty rectangle)
fn dg_rec(ui: &mut Ui, id: u32, text: &str, value: i32) {
    ui.last_reset();
    ui.sem(K_TEXT, id, Some(text), value);
}

/// sel_diag_frame() -- the hidden records #9980..#9984
pub(crate) fn sel_diag_frame(p: &mut Pn, ui: &mut Ui) {
    dg_rec(ui, ID_T2_TEST0, p.sel.dg_res, p.sel.dg_cov as i32);
    let Some(doc) = doc_active(&p.doc) else { return };
    let d = p.doc.d(doc);
    let s = &d.sel;
    let t = format!("{},{},{},{}", s.bbox[0], s.bbox[1], s.bbox[2], s.bbox[3]);
    dg_rec(ui, ID_T2_TEST0 + 1, &t, s.nops as i32);
    let (c, n) = match s.outl.as_ref().filter(|_| s.state) {
        Some(o) => (o.cnt.len(), o.pts.len()),
        None => (0, 0),
    };
    dg_rec(ui, ID_T2_TEST0 + 2, &format!("{c},{n}"), s.flags as i32);
    let st = if crate::tools::tool_cur(&p.tool) == TOOL_MAGIC_WAND { crate::tools::wand::twand_state(p) } else { crate::tools::select::tsel_state(p) }; // pn-tools1
    dg_rec(ui, ID_T2_TEST0 + 3, &st, d.ver_sel as i32);
    let (txt, v) = crate::tools::move_pixels::tmove_diag(&p.tmove);
    dg_rec(ui, ID_T2_TEST0 + 4, &txt, v as i32);
}
