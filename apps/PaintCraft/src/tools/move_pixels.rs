// t_move -- PaintCraft: Pixel verschieben, Auswahl verschieben (agent T2).
// Sub-module of sel.s (tmove_init is called from sel_init).
//                                              [apps/PaintCraft/src/t_move.s]
//
// Both tools show a frame around the selection (its bounds; Pixel verschieben
// without a selection: the whole layer) with 8 handles and the rotation centre (pivot).
// A press starts a LIVE session (tool_live_begin, Fertigstellen): the session's transform M
// (image -> image, f64 affine) grows with every drag until Fertigstellen / Enter / another
// tool / another image / a document command (Auswahl aufheben ...) commits it as ONE history
// step; Esc / Strg+Z drop it (no step, HIST-019/020).
//   drag inside the frame      move (whole pixels)
//   drag a handle              scale (Umschalt at a corner: keep the aspect ratio, Alt: about
//                              the centre); the opposite side stays
//   drag outside / right drag  rotate about the pivot (Umschalt: 15 degree steps)
//   drag the pivot             move the rotation centre
//   arrow keys                 1 px, Strg + arrow 10 px (TRANS-015/016)
//   Horizontal / Vertikal spiegeln (option buttons): mirror inside the frame
// Auswahl verschieben: only the selection geometry moves (pending transform of sel.rs, baked
// into the polygons at the commit: lossless).  History "Auswahl verschieben" / "Auswahl-
// größe ändern" / "Auswahl drehen".
// Pixel verschieben: the selected pixels of the current layer are lifted into a
// floating straight ARGB surface F (alpha * coverage); unless Strg was held at the first
// press (copy) the hole becomes transparent (alpha * (1 - coverage) of the original
// coverage).  The preview goes through the compose hook (mv_row: hole, then the transformed
// pixels P = rs_affine(F, M, Probenahme) blended with Normal); the commit runs the same row
// function on the layer rows (history pixels + selection: one compound step "Ausgewählte
// Pixel verschieben").
//
//   tmove_init()                 register the two tools
//   tmove_active(doc) -> bool    true while a session of doc is pending
//   tmove_timer()                (sel_cmd, TMR_TMOVE) nothing yet
//   tmove_diag() -> (text, value)   test record 9984: "tx,ty,sx%,sy%,deg" of M,
//                                value live | frame << 1 | drag << 2
//   tmove_paste(doc, sf, x, y) -> 0 / -1   (K1, clip.rs) Einfügen: a session of Ausgewählte
//                                Pixel verschieben whose floating pixels are sf (FMT_ARGB,
//                                taken over) at (x, y); nothing is lifted, the selection shows
//                                the pasted rectangle; the commit is ONE step "Einfügen"
//                                (selection before = the old one), Esc / Strg+Z drop it and
//                                bring the old selection back
//   tmove_hook()                 the compose hook (HookKey::Move): mv_row
//
// Rust model: the asm's MC_* row context is `MvCtx` (the hole coverage, its rectangle, the
// shown pixels P and their rectangle); the compose hook borrows it on the worker threads, the
// commit runs mv_row in place on the layer's rows (the layer surface and the context are
// different fields of Pn).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::blendmode::{bm_cov_row_inplace, BM_NORMAL, BM_OPAC_SHIFT};
use gfx::straight::erase_row;
use gfx::tmask::TMask;
use gfx::{Surface, FMT_ARGB};
use img_core::ops::Band;
use img_core::resample::{RS_BICUBIC, RS_BILINEAR, RS_MULTISAMPLE, RS_NEAREST};
use img_core::xform::{invert, mul, rotate, scale, translate, Affine, Matrix, XF_EDGE_SHIFT, XF_TRANSPARENT};
use rt::fx::{cvtd, cvttd, maxsd, minsd, sqrtd};
use rt::sync::SpinLock;
use rt::x87::atan2;
use ui::*;

use crate::document::composite::{comp_mark, comp_set_tool, CompHook, HookKey};
use crate::app::constants::*;
use crate::document::model::{doc_active, doc_mark_px, Doc, DocId};
use crate::document::history::{hist_begin_compound, hist_end_compound, hist_push_sel, hist_px_begin, hist_px_cancel, hist_px_end, hist_px_touch};
use crate::workspace::nubs::{nub_set, nubs_damage, nubs_draw, nubs_hit, nubs_hover, Nub};
use crate::document::selection::{sel_bake, sel_clear, sel_clip_row, sel_hrestore, sel_hsnap, sel_line, sel_set_rect, sel_set_xform, SelBlob};
use crate::tools::*;
use crate::workspace::view::{view_damage_doc, view_doc_to_screen_f64};
use crate::app::Pn;

const ID_MV_PX: u32 = (TOOLOPT_BASE + TOOL_MOVE_PIXELS * TOOLOPT_STRIDE) as u32;
const ID_MV_SEL: u32 = (TOOLOPT_BASE + TOOL_MOVE_SEL * TOOLOPT_STRIDE) as u32;
const MVO_FILTER: u32 = 0; // Probenahme drop-down (rows + 1 .. + 4)
const MVO_FLIPH: u32 = 10;
const MVO_FLIPV: u32 = 11;
const NUB_N: usize = 9; // 8 frame handles + the pivot
const NUB_PIVOT: usize = 8;
const FILTER_N: u32 = 4;
const BAND: i32 = 32; // rows per rs_affine work item
const DR_NONE: u32 = 0;
const DR_MOVE: u32 = 1;
const DR_SCALE: u32 = 2;
const DR_ROTATE: u32 = 3;
const DR_PIVOT: u32 = 4;
const DID_MOVE: u32 = 1;
const DID_SCALE: u32 = 2;
const DID_ROTATE: u32 = 4;

const C_INV256: f64 = 0.00390625;
const C_EPS: f64 = 1.0e-6;
const C_MINSCALE: f64 = 1.0e-4;
const C_BIG: f64 = 1073741824.0;
const C_NBIG: f64 = -1073741824.0;
const C_STEP15: f64 = 0.2617993877991494; // 15 degrees
const C_RAD2DEG: f64 = 57.29577951308232;
const SIGN: u64 = 0x8000_0000_0000_0000;
const M_ZERO: Matrix = Matrix { a: 0.0, b: 0.0, c: 0.0, d: 0.0, e: 0.0, f: 0.0 };

/// frame handles in NF_* order: fractions of the width / height
static NUB_FX: [f64; 8] = [0.0, 0.5, 1.0, 1.0, 1.0, 0.5, 0.0, 0.0];
static NUB_FY: [f64; 8] = [0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 0.5];
/// edges moved by each frame handle: 1 left, 2 top, 4 right, 8 bottom
static NUB_EDGES: [u8; 8] = [3, 2, 6, 4, 12, 8, 9, 1];
static NUB_CURSORS: [u32; 8] = [CUR_SIZE_NWSE, CUR_SIZE_NS, CUR_SIZE_NESW, CUR_SIZE_WE, CUR_SIZE_NWSE, CUR_SIZE_NS, CUR_SIZE_NESW, CUR_SIZE_WE];
/// corners TL, TR, BR, BL as indexes into B (x0 0, y0 1, x1 2, y1 3)
static CORNER_X: [usize; 4] = [0, 2, 2, 0];
static CORNER_Y: [usize; 4] = [1, 1, 3, 3];
/// Probenahme rows -> RS_*
static FILTER_RS: [u32; 4] = [RS_NEAREST, RS_BILINEAR, RS_BICUBIC, RS_MULTISAMPLE];
static FILTER_LABELS: [&str; 4] = ["Nächster Nachbar", "Bilineare", "Bikubisch", "Bilinear mit Multisampling"];
const S_MOVEPX: &str = "Pixel verschieben";

/// the row context of mv_row (MC_*, g_mv_ctx): what the compose hook and the commit read
#[derive(Default)]
pub(crate) struct MvCtx {
    /// MC_OMASK: original coverage (the hole), None = copy / none
    omask: Option<TMask>,
    /// MC_BX0 .. MC_BY1: hole rectangle (the lifted bounds)
    bx: [i32; 4],
    /// MC_PPX (g_mv_p while shown): the transformed pixels P, None = nothing shown
    p: Option<Surface>,
    /// MC_PX0 .. MC_PY1: P rectangle (image px)
    pr: [i32; 4],
}

/// the state of t_move.s (its g_* globals)
pub(crate) struct TMove {
    /// frame B: f64 x0, y0, x1, y1 (local)
    b: [f64; 4],
    /// transform M, M^-1, M at the drag start and its inverse
    m: Matrix,
    mi: Matrix,
    m0: Matrix,
    mi0: Matrix,
    /// pivot (local), pivot (image) at the drag start, pointer at the drag start / last step
    pl: [f64; 2],
    p0: [f64; 2],
    st: [f64; 2],
    lp: [f64; 2],
    /// rotation of M (radians), at the drag start; start angle of a rotation drag
    rot: f64,
    rot0: f64,
    a0: f64,
    doc: Option<DocId>,
    /// floating pixels (straight ARGB)
    f: Surface,
    ctx: MvCtx,
    nubs: [Nub; NUB_N],
    /// frame box (image px) for damage
    fb: [i32; 4],
    live: bool,
    tool: i32,
    /// the frame / handles are valid
    frame: bool,
    /// DOC_VER_SEL / size of the frame
    fver: u32,
    fw: i32,
    fh: i32,
    /// M invertible
    inv: bool,
    /// DR_*
    drag: u32,
    nub: i32,
    btn: u32,
    /// keys of the last drag step
    lmods: u32,
    copy: bool,
    /// the session made a select-all
    nosel: bool,
    /// DID_*
    did: u32,
    /// Probenahme row; the one P was rendered with
    filter: u32,
    rfilter: u32,
    /// K1: selection before the paste, pasted rectangle, the session is a paste
    psel: Option<Box<SelBlob>>,
    prect: [i32; 4],
    paste: bool,
}

impl Default for TMove {
    fn default() -> TMove {
        TMove {
            b: [0.0; 4],
            m: M_ZERO, // (.bss: tmove_diag reads it before the first frame)
            mi: M_ZERO,
            m0: M_ZERO,
            mi0: M_ZERO,
            pl: [0.0; 2],
            p0: [0.0; 2],
            st: [0.0; 2],
            lp: [0.0; 2],
            rot: 0.0,
            rot0: 0.0,
            a0: 0.0,
            doc: None,
            f: Surface::default(),
            ctx: MvCtx::default(),
            nubs: [Nub::default(); NUB_N],
            fb: [0; 4],
            live: false,
            tool: 0,
            frame: false,
            fver: 0,
            fw: 0,
            fh: 0,
            inv: false,
            drag: DR_NONE,
            nub: 0,
            btn: 0,
            lmods: 0,
            copy: false,
            nosel: false,
            did: 0,
            filter: 0,
            rfilter: 0,
            psel: None,
            prect: [0; 4],
            paste: false,
        }
    }
}

static DESC_MOVEPX: ToolDesc = ToolDesc {
    flags: (TF_LIVE | TF_CLIP_SEL) as u32,
    activate: Some(mv_activate),
    deactivate: Some(mv_deactivate),
    input: Some(mv_input),
    options: Some(mv_options),
    key: Some(mv_key),
    commit: Some(mv_commit),
    cancel: Some(mv_cancel),
    cursor: Some(mv_cursor),
    docswitch: Some(mv_docswitch),
    overlay: Some(mv_overlay),
    tick: Some(mv_tick),
    ..ToolDesc::NONE
};

static DESC_MOVESEL: ToolDesc = ToolDesc {
    flags: TF_LIVE as u32,
    activate: Some(mv_activate),
    deactivate: Some(mv_deactivate),
    input: Some(mv_input),
    options: Some(mv_options),
    key: Some(mv_key),
    commit: Some(mv_commit),
    cancel: Some(mv_cancel),
    cursor: Some(mv_cursor),
    docswitch: Some(mv_docswitch),
    overlay: Some(mv_overlay),
    tick: Some(mv_tick),
    ..ToolDesc::NONE
};

/// tmove_init()
pub(crate) fn tmove_init(p: &mut Pn, _ui: &mut Ui) {
    p.tmove.filter = 1; // Bilinear
    tool_register(&mut p.tool, TOOL_MOVE_PIXELS, Some(&DESC_MOVEPX));
    tool_register(&mut p.tool, TOOL_MOVE_SEL, Some(&DESC_MOVESEL));
}

/// tmove_active(doc) -> 1 while a move session of doc is pending
pub(crate) fn tmove_active(p: &Pn, doc: DocId) -> bool {
    p.tmove.live && p.tmove.doc == Some(doc)
}

/// tmove_timer() -- (no timer used yet)
pub(crate) fn tmove_timer(_p: &mut Pn, _ui: &mut Ui) {}

/// tmove_diag() -> ("tx,ty,sx%,sy%,deg", live | frame << 1 | drag << 2)
pub(crate) fn tmove_diag(t: &TMove) -> (String, u32) {
    let m = &t.m;
    let tx = mv_round(m.c);
    let ty = mv_round(m.f);
    let sx = mv_round(sqrtd(m.a * m.a + m.d * m.d) * 100.0);
    let sy = mv_round(sqrtd(m.b * m.b + m.e * m.e) * 100.0);
    let deg = mv_round(t.rot * C_RAD2DEG);
    let s = crate::app::formatting::fmt_cap(format!("{},{},{},{},{}", tx, ty, sx, sy, deg), 96);
    (s, (t.drag << 2) | (t.frame as u32) << 1 | t.live as u32)
}

// ------------------------------------------------------------------ small helpers

// mv_round(x) -> nearest integer, clamped to +-2^30
fn mv_round(x: f64) -> i32 {
    cvtd(minsd(maxsd(x, C_NBIG), C_BIG))
}

// mv_floor(x) -> floor, clamped
fn mv_floor(x: f64) -> i32 {
    let x = minsd(maxsd(x, C_NBIG), C_BIG);
    let t = cvttd(x);
    if (t as f64) > x { t - 1 } else { t }
}

// ucomisd + jne: ordered and not equal
#[inline]
fn ne(a: f64, b: f64) -> bool {
    a < b || a > b
}

#[inline]
fn fabs(v: f64) -> f64 {
    f64::from_bits(v.to_bits() & !SIGN)
}

// (v & sign) | s: s with the sign of v
#[inline]
fn with_sign(s: f64, v: f64) -> f64 {
    f64::from_bits((v.to_bits() & SIGN) | s.to_bits())
}

// xf_invert(m, dst) -> 1 ok; dst is unchanged when m is singular
fn inv_into(m: &Matrix, dst: &mut Matrix) -> bool {
    match invert(m) {
        Some(i) => {
            *dst = i;
            true
        }
        None => false,
    }
}

// mv_is_px() -> 1 when the session / frame belongs to Pixel verschieben
fn mv_is_px(t: &TMove) -> bool {
    t.tool == TOOL_MOVE_PIXELS
}

// ------------------------------------------------------------------ frame

// mv_nubs_update() -- the 9 handles and the frame box from B, M, the pivot
fn mv_nubs_update(p: &mut Pn, ui: &mut Ui) {
    let t = &mut p.tmove;
    let mut bx = [C_BIG, C_BIG, C_NBIG, C_NBIG];
    let b = t.b;
    for i in 0..NF_COUNT as usize {
        let x = (b[2] - b[0]) * NUB_FX[i] + b[0];
        let y = (b[3] - b[1]) * NUB_FY[i] + b[1];
        let (x, y) = t.m.apply(x, y);
        bx[0] = minsd(x, bx[0]);
        bx[1] = minsd(y, bx[1]);
        bx[2] = maxsd(x, bx[2]);
        bx[3] = maxsd(y, bx[3]);
        t.nubs[i].kind = NK_SQUARE as u32;
        mv_nub_at(&p.view, ui, &mut t.nubs[i], x, y);
    }
    let (x, y) = t.m.apply(t.pl[0], t.pl[1]);
    t.nubs[NUB_PIVOT].kind = NK_CIRCLE as u32;
    mv_nub_at(&p.view, ui, &mut t.nubs[NUB_PIVOT], x, y);
    // frame box (image px, margin 1)
    t.fb = [mv_floor(bx[0]) - 1, mv_floor(bx[1]) - 1, mv_floor(bx[2]) + 2, mv_floor(bx[3]) + 2];
}

// mv_nub_at(nub, x, y) -- nub_set to the image point
fn mv_nub_at(v: &crate::workspace::view::View, ui: &mut Ui, nub: &mut Nub, x: f64, y: f64) {
    let a = mv_round(x * 256.0);
    let b = mv_round(y * 256.0);
    nub_set(v, ui, nub, a, b);
}

// mv_damage_frame() -- redraw the frame box and the handles
fn mv_damage_frame(p: &mut Pn, ui: &mut Ui) {
    let t = &mut p.tmove;
    if !t.frame {
        return;
    }
    if let Some(d) = t.doc {
        view_damage_doc(&p.view, ui, d, t.fb[0], t.fb[1], t.fb[2], t.fb[3]);
    }
    nubs_damage(&p.view, ui, &mut t.nubs);
}

// mv_frame_set() -- M = identity, the pivot in the centre of B, the frame of the document
// (the common part of mv_frame_reset and tmove_paste)
fn mv_frame_set(p: &mut Pn, ui: &mut Ui, doc: DocId) {
    let (ver, w, h) = {
        let d = p.doc.d(doc);
        (d.ver_sel, d.w, d.h)
    };
    let t = &mut p.tmove;
    t.m = Matrix::IDENTITY;
    t.mi = Matrix::IDENTITY;
    t.inv = true;
    t.pl = [(t.b[0] + t.b[2]) * 0.5, (t.b[1] + t.b[3]) * 0.5];
    t.rot = 0.0;
    t.fver = ver;
    t.fw = w;
    t.fh = h;
    t.nubs = [Nub::default(); NUB_N];
    t.frame = true;
    mv_nubs_update(p, ui);
    mv_damage_frame(p, ui);
}

// mv_frame_reset(doc) -- a new frame (not live): the selection bounds, the whole image for
// Pixel verschieben without a selection, else none
fn mv_frame_reset(p: &mut Pn, ui: &mut Ui, doc: Option<DocId>) {
    mv_damage_frame(p, ui);
    p.tmove.frame = false;
    p.tmove.doc = doc;
    let Some(doc) = doc else { return };
    p.tmove.tool = tool_cur(&p.tool);
    let d = p.doc.d(doc);
    if d.sel.nops != 0 {
        let bb = d.sel.bbox;
        if bb[2] <= bb[0] {
            return;
        }
        p.tmove.b = [bb[0] as f64, bb[1] as f64, bb[2] as f64, bb[3] as f64];
    } else {
        if p.tmove.tool != TOOL_MOVE_PIXELS || d.layers.is_empty() {
            return;
        }
        p.tmove.b = [0.0, 0.0, d.w as f64, d.h as f64];
    }
    mv_frame_set(p, ui, doc);
}

// mv_frame_ok(doc) -> 1 when the frame belongs to doc and is current
fn mv_frame_ok(p: &Pn, doc: Option<DocId>) -> bool {
    let t = &p.tmove;
    let Some(doc) = doc else { return false };
    if !t.frame || t.doc != Some(doc) {
        return false;
    }
    if t.live {
        return true;
    }
    let d = p.doc.d(doc);
    d.ver_sel == t.fver && d.w == t.fw && d.h == t.fh && tool_cur(&p.tool) == t.tool
}

// mv_inside(x, y) -> 1 when the image point lies inside the (transformed) frame
fn mv_inside(t: &TMove, x: f64, y: f64) -> bool {
    let (x, y) = t.mi.apply(x, y);
    x >= t.b[0] && t.b[2] > x && y >= t.b[1] && t.b[3] > y
}

// ------------------------------------------------------------------ pixels

// mv_lift(doc) -> 0 / -1 -- F = the selected pixels of the current layer (alpha * coverage),
// the hole coverage, the compose hook
fn mv_lift(p: &mut Pn, doc: DocId) -> i32 {
    {
        let d = p.doc.d(doc);
        let Some(ly) = crate::document::model::doc_cur_layer(d) else { return -1 };
        if ly.sf.px.is_empty() {
            return -1;
        }
        let t = &mut p.tmove;
        // B as integers (the selection bounds or the whole image)
        t.ctx.bx = [mv_round(t.b[0]), mv_round(t.b[1]), mv_round(t.b[2]), mv_round(t.b[3])];
        let (w, h) = (t.ctx.bx[2].wrapping_sub(t.ctx.bx[0]), t.ctx.bx[3].wrapping_sub(t.ctx.bx[1]));
        if w as u32 > 65536 || h as u32 > 65536 {
            return -1;
        }
        t.f = Surface::new(w, h, FMT_ARGB);
        let (bx0, by0) = (t.ctx.bx[0], t.ctx.bx[1]);
        let fs = t.f.stride as usize;
        let ls = ly.sf.stride as usize;
        let wb = w as usize * 4;
        let mut cov = [0u8; 64];
        for row in 0..h {
            let fo = row as usize * fs;
            let lo = (by0 + row) as usize * ls + bx0 as usize * 4;
            t.f.px[fo..fo + wb].copy_from_slice(&ly.sf.px[lo..lo + wb]);
            // alpha *= coverage
            let mut col = 0;
            while col < w {
                let n = (w - col).min(64);
                let nu = n as usize;
                match sel_clip_row(d, bx0 + col, by0 + row, n, &mut cov) as i32 {
                    SCR_ALL => {
                        col += n;
                        continue;
                    }
                    SCR_PART => {
                        for c in cov[..nu].iter_mut() {
                            *c = !*c;
                        }
                    }
                    _ => cov[..nu].fill(255),
                }
                let o = fo + col as usize * 4;
                erase_row(&mut t.f.px[o..o + nu * 4], &cov[..nu], 255);
                col += n;
            }
        }
        // the hole: a copy of the coverage (the selection moves on)
        t.ctx.omask = None;
        if !t.copy {
            if let Some(m) = d.sel.mask.as_deref() {
                t.ctx.omask = Some(m.clone());
            }
        }
    }
    comp_set_tool(p, doc, Some(HookKey::Move));
    0
}

// mv_render() -- P = F through M (Probenahme), its rectangle, composite marked
fn mv_render(p: &mut Pn) {
    let Some(doc) = p.tmove.doc else { return };
    let (dw, dh) = (p.doc.d(doc).w, p.doc.d(doc).h);
    let t = &mut p.tmove;
    let old = if t.ctx.p.is_some() { t.ctx.pr } else { [0; 4] };
    t.ctx.p = None;
    t.rfilter = t.filter;
    // bounds of M(B)
    let mut bx = [C_BIG, C_BIG, C_NBIG, C_NBIG];
    for i in 0..4 {
        let (x, y) = t.m.apply(t.b[CORNER_X[i]], t.b[CORNER_Y[i]]);
        bx[0] = minsd(x, bx[0]);
        bx[1] = minsd(y, bx[1]);
        bx[2] = maxsd(x, bx[2]);
        bx[3] = maxsd(y, bx[3]);
    }
    // floor / ceil, clipped to the image (rounding noise of 1e-9 does not add a column)
    let r = [
        mv_floor(bx[0] + C_EPS).max(0),
        mv_floor(bx[1] + C_EPS).max(0),
        (mv_floor(bx[2] - C_EPS) + 1).min(dw),
        (mv_floor(bx[3] - C_EPS) + 1).min(dh),
    ];
    let mut newr = [0; 4];
    if r[2] > r[0] && r[3] > r[1] && t.inv && (r[2] - r[0]) as u32 <= 65536 && (r[3] - r[1]) as u32 <= 65536 {
        let mut ps = Surface::new(r[2] - r[0], r[3] - r[1], FMT_ARGB);
        // m = T(-B0) * M^-1 * T(p0): destination pixel -> F coordinates
        let mut m = Matrix::IDENTITY;
        translate(&mut m, -t.b[0], -t.b[1]);
        m = mul(&m, &t.mi);
        translate(&mut m, r[0] as f64, r[1] as f64);
        let filter = FILTER_RS[(t.filter & (FILTER_N - 1)) as usize];
        let ok = match Affine::prepare(&t.f, &ps, &m, filter, XF_TRANSPARENT << XF_EDGE_SHIFT, None) {
            Ok(xa) => {
                affine_bands(&xa, &mut ps);
                true
            }
            Err(_) => false,
        };
        if ok {
            t.ctx.p = Some(ps);
            t.ctx.pr = r;
            newr = r;
        }
    }
    if newr == [0; 4] {
        t.ctx.pr = [0; 4];
    }
    // old and new rectangle recomposited
    mv_mark_rect(p, doc, old);
    mv_mark_rect(p, doc, newr);
}

// the rs_affine rows of P in bands of BAND rows on the worker pool (mv_rs_band)
fn affine_bands(xa: &Affine, ps: &mut Surface) {
    let (w, h, fmt, stride) = (ps.w, ps.h, ps.fmt, ps.stride as usize);
    let cells: Vec<SpinLock<Option<Band>>> = ps
        .px
        .chunks_mut(BAND as usize * stride)
        .enumerate()
        .map(|(i, c)| {
            let y0 = i as i32 * BAND;
            SpinLock::new(Some(Band { px: c, w, h, fmt, stride, y0, y1: (y0 + BAND).min(h) }))
        })
        .collect();
    let n = ((h + BAND - 1) / BAND) as usize;
    let _ = plat::parallel_for(n.min(cells.len()), &|i, _| {
        let b = cells[i].lock().take();
        if let Some(mut b) = b {
            let (y0, y1) = (b.y0, b.y1);
            xa.rows(&mut b, y0, y1);
        }
    });
}

// mv_mark_rect(&rect) -- comp_mark(g_mv_doc, rect) when not empty
fn mv_mark_rect(p: &mut Pn, doc: DocId, r: [i32; 4]) {
    if r[2] <= r[0] || r[3] <= r[1] {
        return;
    }
    comp_mark(p, doc, r[0], r[1], r[2], r[3]);
}

// mv_row(ctx, dst, x, y, n, layer_row) -> 1 written -- compose hook and commit row function
// (worker threads): the hole (alpha * (1 - original coverage)), then the moved pixels P
// blended with Normal where P is not transparent.  layer None: dst holds the layer row (the
// commit, in place).
fn mv_row(c: &MvCtx, dst: &mut [u8], x: i32, y: i32, n: i32, layer: Option<&[u8]>) -> bool {
    let mut touched = false;
    let nu = n as usize;
    // MV_TOUCH: dst = the layer row once
    let touch = |dst: &mut [u8], touched: &mut bool| {
        if !*touched {
            *touched = true;
            if let Some(l) = layer {
                dst[..nu * 4].copy_from_slice(&l[..nu * 4]);
            }
        }
    };
    // ---- the hole
    if let Some(om) = c.omask.as_ref() {
        if y >= c.bx[1] && y < c.bx[3] {
            let a = x.max(c.bx[0]);
            let b = x.wrapping_add(n).min(c.bx[2]);
            if a < b {
                touch(dst, &mut touched);
                let mut buf = [0u8; 64];
                let (cov, _) = om.row(a, y, b - a, &mut buf);
                let o = (a - x) as usize * 4;
                erase_row(&mut dst[o..o + (b - a) as usize * 4], cov, 255);
            }
        }
    }
    // ---- the moved pixels
    if let Some(ps) = c.p.as_ref() {
        if y >= c.pr[1] && y < c.pr[3] {
            let a = x.max(c.pr[0]);
            let b = x.wrapping_add(n).min(c.pr[2]);
            if a < b {
                touch(dst, &mut touched);
                let k = (b - a) as usize;
                let po = (y - c.pr[1]) as usize * ps.stride as usize + (a - c.pr[0]) as usize * 4;
                let prow = &ps.px[po..po + k * 4];
                // coverage: 255 where P has alpha
                let mut cov = [0u8; 64];
                for (i, cv) in cov[..k].iter_mut().enumerate() {
                    *cv = if prow[i * 4 + 3] != 0 { 255 } else { 0 };
                }
                let o = (a - x) as usize * 4;
                bm_cov_row_inplace(&mut dst[o..o + k * 4], prow, Some(&cov[..k]), k, BM_NORMAL | 255 << BM_OPAC_SHIFT);
            }
        }
    }
    touched
}

// the compose hook of the move tools
struct MvHook<'a>(&'a MvCtx);

impl CompHook for MvHook<'_> {
    fn row(&self, _doc: &Doc, dst: &mut [u8], x: i32, y: i32, n: i32, layer_row: &[u8]) -> bool {
        mv_row(self.0, dst, x, y, n, Some(layer_row))
    }
}

/// the compose hook of the move tools (HookKey::Move)
pub(crate) fn tmove_hook(p: &Pn) -> Option<Box<dyn CompHook + '_>> {
    Some(Box::new(MvHook(&p.tmove.ctx)))
}

// mv_write(doc) -> 1 pixel step -- the preview into the layer (history pixels)
fn mv_write(p: &mut Pn, doc: DocId) -> bool {
    // region = hole rectangle U P rectangle
    let c = &p.tmove.ctx;
    let mut r = if c.omask.is_some() { c.bx } else { [0x7FFF_FFFF, 0x7FFF_FFFF, 0, 0] };
    if c.p.is_some() {
        r[0] = r[0].min(c.pr[0]);
        r[1] = r[1].min(c.pr[1]);
        r[2] = r[2].max(c.pr[2]);
        r[3] = r[3].max(c.pr[3]);
    }
    if r[2] <= r[0] || r[3] <= r[1] {
        return false;
    }
    let cur = {
        let d = p.doc.d(doc);
        match crate::document::model::doc_cur_layer(d) {
            Some(l) if !l.sf.px.is_empty() => d.cur,
            _ => return false,
        }
    };
    if hist_px_begin(p, doc, -1) != 0 {
        return false;
    }
    if hist_px_touch(p, doc, r[0], r[1], r[2], r[3]) != 0 {
        hist_px_cancel(p, doc);
        return false;
    }
    // mv_row in place on the layer rows: one work item per 64-row band of the region
    {
        let Pn { doc: docs, tmove, .. } = p;
        let ctx = &tmove.ctx;
        let sf = &mut docs.dm(doc).layers[cur as usize].sf;
        let stride = sf.stride as usize;
        let rows: Vec<SpinLock<&mut [u8]>> = sf.px.chunks_mut(stride).skip(r[1] as usize).take((r[3] - r[1]) as usize).map(SpinLock::new).collect();
        let nb = ((r[3] - r[1] + TILE - 1) >> TILE_SH) as usize;
        let item = |i: usize, _: &plat::JobCtl| {
            let y0 = r[1] + ((i as i32) << TILE_SH);
            let y1 = (y0 + TILE).min(r[3]);
            for y in y0..y1 {
                let mut row = rows[(y - r[1]) as usize].lock();
                let mut x = r[0];
                while x < r[2] {
                    let n = (r[2] - x).min(64);
                    let o = x as usize * 4;
                    mv_row(ctx, &mut row[o..o + n as usize * 4], x, y, n, None);
                    x += n;
                }
            }
        };
        let _ = plat::parallel_for(nb, &item);
    }
    comp_set_tool(p, doc, None);
    doc_mark_px(p, doc, cur, r[0], r[1], r[2], r[3]);
    hist_px_end(p, doc, S_MOVEPX, IC_MOVE)
}

// ------------------------------------------------------------------ session

// mv_begin(doc, copy) -> 0 / -1 -- start a live session on the current frame
fn mv_begin(p: &mut Pn, ui: &mut Ui, doc: DocId, copy: bool) -> i32 {
    let t = &mut p.tmove;
    t.copy = copy;
    t.nosel = false;
    t.did = 0;
    t.tool = tool_cur(&p.tool);
    if t.tool == TOOL_MOVE_PIXELS {
        // no selection: the whole layer (a select-all that is undone at the end)
        if p.doc.d(doc).sel.nops == 0 {
            p.tmove.nosel = true;
            let (w, h) = (p.doc.d(doc).w, p.doc.d(doc).h);
            sel_set_rect(p, doc, 0, 0, w, h);
        }
        if mv_lift(p, doc) != 0 {
            // failed: back to the old state
            if p.tmove.nosel {
                sel_clear(p, doc);
            }
            p.tmove.f = Surface::default();
            return -1;
        }
    }
    p.tmove.live = true;
    p.tmove.doc = Some(doc);
    tool_live_begin(p, doc);
    if mv_is_px(&p.tmove) {
        mv_render(p);
    }
    let _ = ui;
    0
}

// mv_free() -- F, P, the hole coverage
fn mv_free(t: &mut TMove) {
    t.f = Surface::default();
    t.ctx = MvCtx::default();
}

// mv_end() -- the session is over (frame rebuilt with the next tick)
fn mv_end(p: &mut Pn, ui: &mut Ui) {
    let t = &mut p.tmove;
    mv_free(t);
    t.psel = None; // K1: the paste's saved selection
    t.paste = false;
    t.live = false;
    t.drag = DR_NONE;
    mv_damage_frame(p, ui);
    p.tmove.frame = false;
}

// mv_commit(doc) -> 1 step (TD_COMMIT)
fn mv_commit(p: &mut Pn, ui: &mut Ui, doc: DocId) -> bool {
    if !p.tmove.live {
        return false;
    }
    if p.tmove.did == 0 {
        // nothing moved: no step
        mv_cancel(p, ui, Some(doc));
        return false;
    }
    let t = &p.tmove;
    let (name, icon) = if t.paste {
        ("Einfügen", IC_PASTE) // K1
    } else if mv_is_px(t) {
        (S_MOVEPX, IC_MOVE)
    } else if t.did & DID_ROTATE != 0 {
        ("Auswahl drehen", IC_ROTATE)
    } else if t.did & DID_SCALE != 0 {
        ("Auswahlgröße ändern", IC_RESIZE)
    } else {
        ("Auswahl verschieben", IC_POINTER)
    };
    hist_begin_compound(p, doc);
    // the selection: the state before (pending transform off), then the moved one
    sel_set_xform(p, doc, None);
    if p.tmove.paste {
        // K1: the selection before the paste
        let psel = p.tmove.psel.take();
        sel_hrestore(p, doc, psel.as_deref());
        p.tmove.psel = psel;
    }
    if p.tmove.nosel {
        sel_clear(p, doc);
    }
    hist_push_sel(p, doc, name, icon);
    if p.tmove.paste {
        // K1: then the pasted rectangle
        let r = p.tmove.prect;
        sel_set_rect(p, doc, r[0], r[1], r[2], r[3]);
    } else if p.tmove.nosel {
        let (w, h) = (p.doc.d(doc).w, p.doc.d(doc).h);
        sel_set_rect(p, doc, 0, 0, w, h);
    }
    let m = p.tmove.m;
    sel_set_xform(p, doc, Some(&m));
    sel_bake(p, doc);
    // the pixels
    if mv_is_px(&p.tmove) {
        mv_write(p, doc);
    }
    comp_set_tool(p, doc, None);
    if mv_is_px(&p.tmove) {
        let bx = p.tmove.ctx.bx;
        mv_mark_rect(p, doc, bx);
        if p.tmove.ctx.p.is_some() {
            let pr = p.tmove.ctx.pr;
            mv_mark_rect(p, doc, pr);
        }
    }
    let r = hist_end_compound(p, doc, name, icon);
    mv_end(p, ui);
    r
}

// mv_cancel(doc) -- drop the session (TD_CANCEL; doc None = the image is gone)
fn mv_cancel(p: &mut Pn, ui: &mut Ui, doc: Option<DocId>) {
    if !p.tmove.live {
        return;
    }
    if let Some(doc) = doc {
        comp_set_tool(p, doc, None);
        // recomposite the hole and the preview
        let bx = p.tmove.ctx.bx;
        mv_mark_rect(p, doc, bx);
        if p.tmove.ctx.p.is_some() {
            let pr = p.tmove.ctx.pr;
            mv_mark_rect(p, doc, pr);
        }
        sel_set_xform(p, doc, None);
        if p.tmove.paste {
            // K1: no paste: the old selection
            let psel = p.tmove.psel.take();
            sel_hrestore(p, doc, psel.as_deref());
            p.tmove.psel = psel;
        }
        if p.tmove.nosel {
            sel_clear(p, doc);
        }
    }
    mv_end(p, ui);
}

// mv_changed() -- M changed: inverse, handles, selection transform, preview
fn mv_changed(p: &mut Pn, ui: &mut Ui) {
    let t = &mut p.tmove;
    let m = t.m;
    t.inv = inv_into(&m, &mut t.mi);
    mv_damage_frame(p, ui);
    mv_nubs_update(p, ui);
    mv_damage_frame(p, ui);
    if let Some(doc) = p.tmove.doc {
        sel_set_xform(p, doc, Some(&m));
    }
    if mv_is_px(&p.tmove) {
        mv_render(p);
    }
}

// mv_translate(dx, dy) -- M = T(d) * M (live session required)
fn mv_translate(p: &mut Pn, ui: &mut Ui, dx: f64, dy: f64) {
    let t = &mut p.tmove;
    t.m.c = dx + t.m.c;
    t.m.f = dy + t.m.f;
    t.did |= DID_MOVE;
    mv_changed(p, ui);
}

// mv_session(doc, copy) -> 1 when a session of doc is live (started if needed)
fn mv_session(p: &mut Pn, ui: &mut Ui, doc: DocId, copy: bool) -> bool {
    if p.tmove.live {
        return true;
    }
    mv_begin(p, ui, doc, copy) == 0
}

// mv_flip(vertical) -- mirror inside the frame (M = M * F about the centre of B)
fn mv_flip(p: &mut Pn, ui: &mut Ui, vertical: bool) {
    let doc = doc_active(&p.doc);
    if !mv_frame_ok(p, doc) {
        mv_frame_reset(p, ui, doc);
        if !mv_frame_ok(p, doc) {
            return;
        }
    }
    let Some(doc) = doc else { return };
    if !mv_session(p, ui, doc, false) {
        return;
    }
    let t = &mut p.tmove;
    let mut f = Matrix::IDENTITY;
    // x' = (x0 + x1) - x  or  y' = (y0 + y1) - y
    let cx = t.b[0] + t.b[2];
    let cy = t.b[1] + t.b[3];
    if vertical {
        f.e = -1.0;
        f.f = cy;
    } else {
        f.a = -1.0;
        f.c = cx;
    }
    t.m = mul(&t.m, &f);
    t.did |= DID_SCALE;
    mv_changed(p, ui);
}

// ------------------------------------------------------------------ dragging

// mv_drag_to(x, y, mods) -- the drag in progress follows the pointer (image px)
fn mv_drag_to(p: &mut Pn, ui: &mut Ui, x: f64, y: f64, mods: u32) {
    let t = &mut p.tmove;
    match t.drag {
        DR_MOVE => {
            // whole pixels: M = T(round(p - start)) * M0
            t.m = t.m0;
            let dx = mv_round(x - t.st[0]) as f64;
            let dy = mv_round(y - t.st[1]) as f64;
            mv_translate(p, ui, dx, dy);
        }
        DR_SCALE => {
            // the pointer in the frame's own coordinates (whole image pixels without rotation)
            let (mut x, mut y) = (x, y);
            if !ne(t.m0.b, 0.0) && !ne(t.m0.d, 0.0) {
                x = mv_round(x) as f64;
                y = mv_round(y) as f64;
            }
            let (px, py) = t.mi0.apply(x, y);
            let b = t.b;
            let mut r = b;
            let e = NUB_EDGES[(t.nub & (NF_COUNT - 1)) as usize]; // 1 left, 2 top, 4 right, 8 bottom
            if e & 1 != 0 {
                r[0] = px;
            }
            if e & 2 != 0 {
                r[1] = py;
            }
            if e & 4 != 0 {
                r[2] = px;
            }
            if e & 8 != 0 {
                r[3] = py;
            }
            // Alt: the opposite side mirrors the moved one about the centre
            if mods & MOD_ALT != 0 {
                let cx2 = b[0] + b[2];
                let cy2 = b[1] + b[3];
                if e & 1 != 0 {
                    r[2] = cx2 - r[0];
                }
                if e & 4 != 0 {
                    r[0] = cx2 - r[2];
                }
                if e & 2 != 0 {
                    r[3] = cy2 - r[1];
                }
                if e & 8 != 0 {
                    r[1] = cy2 - r[3];
                }
            }
            // factors sx = w' / w, sy = h' / h (never 0)
            let mut sx = (r[2] - r[0]) / (b[2] - b[0]);
            let mut sy = (r[3] - r[1]) / (b[3] - b[1]);
            // Umschalt at a corner: keep the aspect ratio (the larger factor, signs kept)
            if mods & MOD_SHIFT != 0 && matches!(e, 3 | 6 | 12 | 9) {
                let s = maxsd(fabs(sx), fabs(sy));
                sx = with_sign(s, sx);
                sy = with_sign(s, sy);
                // the moved corner follows: x' = fixed + s * w (fixed side = the other one)
                let w2 = (b[2] - b[0]) * sx;
                let h2 = (b[3] - b[1]) * sy;
                if e & 1 != 0 {
                    r[0] = r[2] - w2;
                } else {
                    r[2] = r[0] + w2;
                }
                if e & 2 != 0 {
                    r[1] = r[3] - h2;
                } else {
                    r[3] = r[1] + h2;
                }
            }
            // tiny factors (a side dragged onto the other) would make M singular
            if !(fabs(sx) >= C_MINSCALE) || !(fabs(sy) >= C_MINSCALE) {
                return;
            }
            // S = T(N0) * Scale(sx, sy) * T(-B0);  M = M0 * S
            let mut s = Matrix::IDENTITY;
            translate(&mut s, r[0], r[1]);
            scale(&mut s, sx, sy);
            translate(&mut s, -b[0], -b[1]);
            t.m = mul(&t.m0, &s);
            t.did |= DID_SCALE;
            mv_changed(p, ui);
        }
        DR_ROTATE => {
            // angle about the pivot (at the drag start); Umschalt: 15 degree steps of the total
            let mut a = atan2(y - t.p0[1], x - t.p0[0]) - t.a0 + t.rot0;
            if mods & MOD_SHIFT != 0 {
                a = mv_round(a / C_STEP15) as f64 * C_STEP15;
            }
            t.rot = a;
            let mut r = Matrix::IDENTITY;
            rotate(&mut r, t.rot - t.rot0, t.p0[0], t.p0[1]);
            t.m = mul(&r, &t.m0);
            t.did |= DID_ROTATE;
            mv_changed(p, ui);
        }
        DR_PIVOT => {
            let (px, py) = t.mi.apply(x, y);
            t.pl = [px, py];
            mv_damage_frame(p, ui);
            mv_nubs_update(p, ui);
            mv_damage_frame(p, ui);
        }
        _ => {}
    }
}

// ------------------------------------------------------------------ tool callbacks

// mv_input(doc, ui, x_fx, y_fx) -> 1 changed
fn mv_input(p: &mut Pn, ui: &mut Ui, doc: DocId, inp: &UiInput, x_fx: i32, y_fx: i32) -> bool {
    let sx = inp.x + inp.rect[0];
    let sy = inp.y + inp.rect[1];
    let x = x_fx as f64 * C_INV256;
    let y = y_fx as f64 * C_INV256;
    if p.tmove.drag != DR_NONE {
        // a drag in progress
        let t = &mut p.tmove;
        if Some(doc) == t.doc && inp.btn & (1 << t.btn) != 0 {
            // only a new pointer position or other keys change anything
            if x == t.lp[0] && y == t.lp[1] && inp.mods == t.lmods {
                return false;
            }
            t.lp = [x, y];
            t.lmods = inp.mods;
            mv_drag_to(p, ui, x, y, inp.mods);
            return true;
        }
        t.drag = DR_NONE;
        for n in t.nubs.iter_mut() {
            n.flags &= !(NBF_DRAG as u32);
        }
        nubs_damage(&p.view, ui, &mut t.nubs);
        return true;
    }
    if !mv_frame_ok(p, Some(doc)) {
        mv_frame_reset(p, ui, Some(doc));
        if !mv_frame_ok(p, Some(doc)) {
            return false;
        }
    }
    let btn = if inp.pressed & (1 << BTN_LEFT) != 0 {
        BTN_LEFT
    } else if inp.pressed & (1 << BTN_RIGHT) != 0 {
        BTN_RIGHT
    } else {
        return nubs_hover(&p.view, ui, &mut p.tmove.nubs, sx, sy);
    };
    p.tmove.btn = btn;
    // what is dragged: right = rotate; a handle; inside = move; outside = rotate
    let mut what = DR_ROTATE;
    let mut k = -1;
    if btn != BTN_RIGHT {
        k = nubs_hit(&p.view, ui, &p.tmove.nubs, sx, sy);
        if k >= 0 {
            what = if k as usize == NUB_PIVOT { DR_PIVOT } else { DR_SCALE };
        } else {
            what = if mv_inside(&p.tmove, x, y) { DR_MOVE } else { DR_ROTATE };
        }
    }
    // the session (Strg at the first press of Pixel verschieben: copy)
    let copy = !p.tmove.live && what == DR_MOVE && inp.mods & MOD_CTRL != 0;
    if !mv_session(p, ui, doc, copy) {
        return false;
    }
    let t = &mut p.tmove;
    t.drag = what;
    t.nub = k;
    t.m0 = t.m;
    let m0 = t.m0;
    inv_into(&m0, &mut t.mi0);
    t.st = [x, y];
    t.lp = [x, y];
    t.lmods = inp.mods;
    t.rot0 = t.rot;
    // the pivot in image space and the start angle
    let (px, py) = t.m.apply(t.pl[0], t.pl[1]);
    t.p0 = [px, py];
    t.a0 = atan2(y - py, x - px);
    if k >= 0 {
        t.nubs[k as usize].flags |= NBF_DRAG as u32;
        nubs_damage(&p.view, ui, &mut t.nubs);
    }
    true
}

// mv_key(vk, mods) -> 1 handled -- arrow keys move by 1 px (Strg: 10 px)
fn mv_key(p: &mut Pn, ui: &mut Ui, vk: u32, mods: u32) -> bool {
    let step = if mods & MOD_CTRL != 0 { 10 } else { 1 };
    let (dx, dy) = match vk {
        VK_LEFT => (-step, 0),
        VK_RIGHT => (step, 0),
        VK_UP => (0, -step),
        VK_DOWN => (0, step),
        _ => return false,
    };
    if p.tmove.drag != DR_NONE {
        return true;
    }
    let doc = doc_active(&p.doc);
    if doc.is_none() {
        return false;
    }
    if !mv_frame_ok(p, doc) {
        mv_frame_reset(p, ui, doc);
        if !mv_frame_ok(p, doc) {
            return false;
        }
    }
    if mv_session(p, ui, doc.unwrap(), false) {
        mv_translate(p, ui, dx as f64, dy as f64);
    }
    true
}

// mv_activate(doc) / mv_docswitch(old, new) -- a frame for the (new) image
fn mv_activate(p: &mut Pn, ui: &mut Ui, doc: Option<DocId>) {
    mv_frame_reset(p, ui, doc);
}

fn mv_docswitch(p: &mut Pn, ui: &mut Ui, _old: Option<DocId>, new: Option<DocId>) {
    mv_frame_reset(p, ui, new);
}

// mv_deactivate(doc) -- the frame disappears (a session was committed before)
fn mv_deactivate(p: &mut Pn, ui: &mut Ui, _doc: Option<DocId>) {
    mv_damage_frame(p, ui);
    p.tmove.frame = false;
}

// mv_tick(doc) -- keep the frame on the current selection; option changes re-render
fn mv_tick(p: &mut Pn, ui: &mut Ui, doc: Option<DocId>) {
    let t = &p.tmove;
    if t.live {
        if doc == t.doc && mv_is_px(t) && t.filter != t.rfilter {
            mv_render(p);
        }
        return;
    }
    if t.drag == DR_NONE && !mv_frame_ok(p, doc) {
        mv_frame_reset(p, ui, doc);
    }
}

// mv_overlay(doc, &rect) -- the frame (thin dark line through the 4 corners, aliased so every
// dirty-rectangle render is exact) + the handles
fn mv_overlay(p: &mut Pn, g: &mut Gfx, doc: DocId, _rect: [i32; 4]) {
    let t = &mut p.tmove;
    if !t.frame || t.doc != Some(doc) {
        return;
    }
    let mut c = [[0i32; 2]; 4];
    for (i, ci) in c.iter_mut().enumerate() {
        let (x, y) = t.m.apply(t.b[CORNER_X[i]], t.b[CORNER_Y[i]]);
        let mut pt = [x, y];
        view_doc_to_screen_f64(&p.view, &mut pt);
        *ci = [mv_floor(pt[0]), mv_floor(pt[1])];
    }
    for i in 0..4 {
        let j = (i + 1) & 3;
        sel_line(g, c[i][0], c[i][1], c[j][0], c[j][1], 0xFF50_5050);
    }
    nubs_draw(&p.view, g, &mut t.nubs);
}

// mv_cursor(x_fx, y_fx) -> CUR_* -- handles resize, pivot / inside move, outside rotate
fn mv_cursor(p: &mut Pn, x: i32, y: i32) -> u32 {
    let t = &p.tmove;
    if !t.frame {
        return CUR_ARROW;
    }
    for (i, n) in t.nubs.iter().enumerate() {
        if n.flags & (NBF_HOVER | NBF_DRAG) as u32 != 0 {
            return if i == NUB_PIVOT { CUR_MOVE } else { NUB_CURSORS[i] };
        }
    }
    if t.drag == DR_MOVE {
        return CUR_MOVE;
    }
    if mv_inside(t, x as f64 * C_INV256, y as f64 * C_INV256) {
        return CUR_MOVE;
    }
    (PNC_BASE + PNC_ROTATE) as u32 // outside: rotate (cursors.rs)
}

// mv_options() -- Probenahme (pixels), Horizontal / Vertikal spiegeln
fn mv_options(p: &mut Pn, ui: &mut Ui) {
    let mut id = ID_MV_SEL;
    if tool_cur(&p.tool) == TOOL_MOVE_PIXELS {
        id = ID_MV_PX;
        opt_label(ui, "Probenahme:");
        let mut v = p.tmove.filter;
        opt_menu(p, ui, id + MVO_FILTER, &mut v, &FILTER_LABELS, &[], None, OMF_NOICON as u32);
        p.tmove.filter = v;
        opt_sep(ui);
    }
    if ui.icon_button(id + MVO_FLIPH, IC_SWITCH, "Horizontal spiegeln") & EV_ACT != 0 {
        mv_flip(p, ui, false);
    }
    if ui.icon_button(id + MVO_FLIPV, IC_CHEV_UP_DOWN, "Vertikal spiegeln") & EV_ACT != 0 {
        mv_flip(p, ui, true);
    }
}

// ------------------------------------------------------------------ K1: paste (clip.rs)

/// tmove_paste(doc, sf, x, y) -> 0 / -1 -- a paste session (see the file header)
pub(crate) fn tmove_paste(p: &mut Pn, ui: &mut Ui, doc: DocId, sf: Surface, x: i32, y: i32) -> i32 {
    tool_commit_live(p, ui);
    if tool_cur(&p.tool) != TOOL_MOVE_PIXELS {
        tool_set(p, ui, TOOL_MOVE_PIXELS);
    }
    if tool_cur(&p.tool) != TOOL_MOVE_PIXELS || p.tmove.live {
        return -1;
    }
    let x1 = x.wrapping_add(sf.w);
    let y1 = y.wrapping_add(sf.h);
    // the old selection (restored by Esc, recorded by the commit)
    p.tmove.psel = sel_hsnap(p.doc.d(doc));
    p.tmove.prect = [x, y, x1, y1];
    sel_set_rect(p, doc, x, y, x1, y1);
    // the frame: the whole pasted rectangle (it may reach past the canvas)
    mv_damage_frame(p, ui);
    p.tmove.doc = Some(doc);
    p.tmove.tool = TOOL_MOVE_PIXELS;
    p.tmove.b = [x as f64, y as f64, x1 as f64, y1 as f64];
    mv_frame_set(p, ui, doc);
    // F = the pasted pixels; no hole
    let t = &mut p.tmove;
    mv_free(t);
    t.f = sf;
    t.copy = true;
    t.nosel = false;
    t.did = DID_MOVE;
    t.drag = DR_NONE;
    t.paste = true;
    comp_set_tool(p, doc, Some(HookKey::Move));
    p.tmove.live = true;
    tool_live_begin(p, doc);
    mv_render(p);
    0
}
