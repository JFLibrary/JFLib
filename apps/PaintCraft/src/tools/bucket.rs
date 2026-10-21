// t_bucket -- PaintCraft: Farbeimer (paint bucket, agent T1).  Sub-module of tool.rs.
//                                              [apps/PaintCraft/src/t_bucket.s]
//
// A click fills the region around the pixel (flood_fill_mask with paint.net's tolerance;
// Füllungsmodus zusammenhängend / global, Shift = global) sampled from the layer or the whole
// image (a private snapshot taken when the fill starts), with the colour of the button (left
// primary, right secondary) or a hatch pattern, the blend mode, antialiasing (the region's
// edge is softened with a 3 x 3 tent filter).  Like paint.net 5 the fill stays LIVE: a nub
// at the origin can be dragged (the fill follows), changing tolerance / mode / sampling /
// antialiasing re-runs it, changing colours / pattern / blend mode re-renders it -- until
// Fertigstellen / Enter / another tool / another image / a document command (one history
// step "Farbeimer"); Esc or Strg+Z drop it (no step).  Images of 4 Mpx and more fill on the
// worker pool; a newer request cancels the running job and restarts when it has finished.
//
//   t_bucket_init()               register TOOL_BUCKET
//   bk_poll()                     (tool.rs, UIC_WAKE) a background fill finished?
//   bk_pending() -> bool          a background fill runs (script `sync`; tool::SYNC_BK)
//   bk_state() -> (text, bool)    test record 9939: "x,y" of the live origin, a job running
//   bk_test_async(on)             test hook: every fill runs on the worker pool
//
// Rust model: the fill request and its result (the snapshot, the back masks) live in a
// `BkCtx` shared with the background job (Arc + SpinLock: the job holds the lock while it
// computes; the UI thread locks it only while no job runs).  The shown (front) masks are
// p.tbucket.mf; the tool layer's mask is TlMask::Bucket(i) = mf[i].

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use core::sync::atomic::Ordering;

use gfx::flood::{flood_fill_mask, FF_GLOBAL};
use gfx::tmask::{Span, TMask};
use gfx::{Surface, ERR_NOMEM, FMT_ARGB};
use rt::sync::SpinLock;
use ui::*;

use crate::document::composite::comp_surface;
use crate::app::constants::*;
use crate::document::model::{doc_cur_layer, DocId};
use crate::workspace::nubs::{nub_set, nubs_damage, nubs_draw, nubs_hit, nubs_hover, Nub};
use crate::workspace::palette::pal_version;
use crate::tools::*;
use crate::workspace::view::view_cursor;
use crate::app::Pn;

const BK_ASYNC_PX: u64 = 4_000_000; // pixels from which the fill runs in the background

/// the fill request and its result (g_bk_ctx + the snapshot + the back masks)
#[derive(Default)]
pub(crate) struct BkCtx {
    /// BKC_SRC: the snapshot of the sampled pixels (straight)
    src: Surface,
    /// BKC_X / BKC_Y: seed
    x: i32,
    y: i32,
    /// BKC_TOL: tolerance %
    tol: u32,
    /// BKC_FLAGS: FF_GLOBAL
    flags: u32,
    /// BKC_AA: soften the edge
    aa: bool,
    /// BKC_RES: 0 ok / ERR_*
    res: i32,
    /// BKC_FLOOD / BKC_FILL: the back masks (region, softened region)
    flood: Option<TMask>,
    fill: Option<TMask>,
    /// BKC_BOX: box of the result
    bx: [i32; 4],
}

/// the state of t_bucket.s (its g_* globals)
pub(crate) struct TBucket {
    job: plat::Job,
    ctx: Arc<SpinLock<BkCtx>>,
    /// origin handle
    nub: Nub,
    /// g_bk_mf: the shown masks (region, softened region)
    pub(crate) mf: [Option<TMask>; 2],
    doc: Option<DocId>,
    live: bool,
    /// origin (image px)
    x: i32,
    y: i32,
    btn: u32,
    /// Shift at the click (global)
    shift: u32,
    drag: bool,
    dragbtn: u32,
    /// document state of the snapshot
    vpx: u32,
    vlay: u32,
    vcur: i32,
    vsmp: u32,
    vsel: u32,
    /// requested fill options
    rtol: u32,
    rflags: u32,
    raa: u32,
    /// TC_VER / pal_version of the paint
    pver: u32,
    palv: u32,
    jrun: bool,
    jpend: bool,
    /// the running job was cancelled (JOB_CANCEL)
    jcancel: bool,
    force: bool,
    /// compute now (commit)
    sync: bool,
}

impl Default for TBucket {
    fn default() -> TBucket {
        TBucket {
            job: plat::Job::new(),
            ctx: Arc::new(SpinLock::new(BkCtx::default())),
            nub: Nub::default(),
            mf: [None, None],
            doc: None,
            live: false,
            x: 0,
            y: 0,
            btn: 0,
            shift: 0,
            drag: false,
            dragbtn: 0,
            vpx: 0,
            vlay: 0,
            vcur: 0,
            vsmp: 0,
            vsel: 0,
            rtol: 0,
            rflags: 0,
            raa: 0,
            pver: 0,
            palv: 0,
            jrun: false,
            jpend: false,
            jcancel: false,
            force: false,
            sync: false,
        }
    }
}

static DESC_BUCKET: ToolDesc = ToolDesc {
    flags: (TF_LIVE | TF_CLIP_SEL | TF_AA | TF_BLEND | TF_FILL | TF_TOL | TF_SAMPLE) as u32,
    input: Some(bk_input),
    options: Some(bk_options),
    commit: Some(bk_commit),
    cancel: Some(bk_cancel),
    cursor: Some(bk_cursor),
    overlay: Some(bk_overlay),
    tick: Some(bk_tick),
    ..ToolDesc::NONE
};

/// t_bucket_init()
pub(crate) fn t_bucket_init(p: &mut Pn, _ui: &mut Ui) {
    tool_register(&mut p.tool, TOOL_BUCKET, Some(&DESC_BUCKET));
}

// the `sync` flag of tool.rs follows jrun / jpend
fn bk_sync_flag(b: &TBucket) {
    SYNC_BK.store(b.jrun || b.jpend, Ordering::SeqCst);
}

/// bk_pending() -> 1 while a background fill runs
pub(crate) fn bk_pending(p: &mut Pn) -> bool {
    p.tbucket.jrun || p.tbucket.jpend
}

/// bk_state() -> ("x,y" (live origin) or "", 1 while a background fill runs)
pub(crate) fn bk_state(b: &TBucket) -> (String, bool) {
    let s = if b.live { format!("{},{}", b.x, b.y) } else { String::new() };
    (s, b.jrun)
}

/// bk_test_async(on)
pub(crate) fn bk_test_async(b: &mut TBucket, on: bool) {
    b.force = on;
}

// ------------------------------------------------------------------ computing a fill

// bk_compute(ctx): flood fill + optional edge softening + box (UI thread or worker)
fn bk_compute(c: &mut BkCtx, cancel: &dyn Fn() -> bool) {
    c.bx = [0; 4];
    let (Some(flood), Some(fill)) = (c.flood.as_mut(), c.fill.as_mut()) else {
        c.res = ERR_NOMEM;
        return;
    };
    c.res = flood_fill_mask(&c.src, c.x, c.y, c.tol as i32, c.flags, flood);
    if c.res != 0 {
        return;
    }
    let m: &TMask = if c.aa {
        c.res = bk_smooth(flood, fill, cancel);
        if c.res != 0 {
            return;
        }
        fill
    } else {
        flood
    };
    c.bx = m.bbox().unwrap_or([0; 4]);
}

// slot class of a tile: 0 empty, 1 full, 2 tile
fn slot_kind(m: &TMask, tx: u32, ty: u32) -> u32 {
    match m.span((tx << 6) as i32, (ty << 6) as i32, 1).0 {
        Span::Empty => 0,
        Span::Full => 1,
        Span::Bytes(_) => 2,
    }
}

// bk_smooth(src, dst, cancel) -> 0 / ERR_NOMEM / 1 cancelled: dst = 3 x 3 tent filter of src
// ([1 2 1] x [1 2 1] / 16, pixels outside the image repeat the edge); uniform neighbourhoods
// stay sentinels
fn bk_smooth(src: &TMask, dst: &mut TMask, cancel: &dyn Fn() -> bool) -> i32 {
    dst.clear(0);
    let (w, h) = (src.w as i32, src.h as i32);
    let (tw, th) = (src.tiles_x(), src.tiles_y());
    let mut win = vec![0u8; 66 * 66];
    let mut hs = vec![0u16; 66 * 64];
    let mut buf = [0u8; 64];
    for ty in 0..th {
        for tx in 0..tw {
            if cancel() {
                return 1;
            }
            // neighbourhood: bit 0 = some tile is not empty, bit 1 = some tile is not full
            let mut k = 0u32;
            for dty in -1i32..=1 {
                for dtx in -1i32..=1 {
                    let (nx, ny) = (tx as i32 + dtx, ty as i32 + dty);
                    if nx < 0 || ny < 0 || nx as u32 >= tw || ny as u32 >= th {
                        continue;
                    }
                    k |= match slot_kind(src, nx as u32, ny as u32) {
                        0 => 2,
                        1 => 1,
                        _ => 3,
                    };
                }
            }
            if k & 1 == 0 {
                continue; // all empty
            }
            if k & 2 == 0 {
                dst.tile_set(tx, ty, true); // all full
                continue;
            }
            let x0 = (tx << 6) as i32;
            let y0 = (ty << 6) as i32;
            let n = (w - x0).min(64) as usize;
            // the 66 x 66 window (edges repeated)
            for r in 0..66usize {
                let iy = (y0 + r as i32 - 1).clamp(0, h - 1);
                let wr = &mut win[r * 66..r * 66 + 66];
                let (row, _) = src.row(x0, iy, n as i32, &mut buf);
                wr[1..1 + n].copy_from_slice(&row[..n]);
                // right part: the next pixel or the repeated edge
                let last = wr[n];
                for b in &mut wr[n + 1..66] {
                    *b = last;
                }
                if x0 + 64 < w {
                    wr[65] = src.get(x0 + 64, iy);
                }
                wr[0] = if x0 != 0 { src.get(x0 - 1, iy) } else { wr[1] };
            }
            // h[r][c] = w[r][c] + 2 w[r][c + 1] + w[r][c + 2]   (66 rows x 64 u16)
            for r in 0..66usize {
                let wr = &win[r * 66..r * 66 + 66];
                let hr = &mut hs[r * 64..r * 64 + 64];
                for c in 0..64 {
                    hr[c] = wr[c] as u16 + 2 * wr[c + 1] as u16 + wr[c + 2] as u16;
                }
            }
            let Some(t) = dst.tile_w(tx, ty) else { return ERR_NOMEM };
            for r in 0..64usize {
                for c in 0..64 {
                    let v = hs[r * 64 + c] as u32 + 2 * hs[(r + 1) * 64 + c] as u32 + hs[(r + 2) * 64 + c] as u32;
                    t[r * 64 + c] = ((v + 8) >> 4) as u8;
                }
            }
        }
    }
    dst.optimize();
    0
}

// bk_masks(w, h) -> true: the back masks exist with that size
fn bk_masks(c: &mut BkCtx, w: i32, h: i32) -> bool {
    for m in [&mut c.flood, &mut c.fill] {
        if !matches!(m, Some(t) if t.w as i32 == w && t.h as i32 == h) {
            *m = None;
            *m = TMask::new(w as u32, h as u32);
            if m.is_none() {
                return false;
            }
        }
    }
    true
}

// bk_install(): the finished back result becomes the shown one
fn bk_install(p: &mut Pn) {
    let (aa, res, bx) = {
        let mut c = p.tbucket.ctx.lock();
        core::mem::swap(&mut p.tbucket.mf[0], &mut c.flood);
        core::mem::swap(&mut p.tbucket.mf[1], &mut c.fill);
        if c.res != 0 {
            c.bx = [0; 4];
        }
        (c.aa, c.res, c.bx)
    };
    p.tool.tls[TL_BUCKET].mask = if res != 0 { TlMask::None } else { TlMask::Bucket(aa as usize) };
    tl_set_box(p, TL_BUCKET, bx[0], bx[1], bx[2], bx[3]);
    tl_show(p, TL_BUCKET);
    p.repass();
}

// bk_request(): (re)compute the fill with the current origin and options
fn bk_request(p: &mut Pn) {
    if !p.tbucket.live {
        return;
    }
    let c = p.tool.cfg;
    let b = &mut p.tbucket;
    b.rtol = c.tol;
    b.rflags = (c.flood | b.shift) & FF_GLOBAL;
    b.raa = c.aa;
    if b.jrun {
        b.jpend = true; // restart when the running job ended
        b.jcancel = true;
        b.job.cancel();
        bk_sync_flag(b);
        return;
    }
    let Some(doc) = b.doc else { return };
    let Some(d) = p.doc.get(doc) else { return };
    let (w, h) = (d.w, d.h);
    let b = &mut p.tbucket;
    {
        let mut x = b.ctx.lock();
        if !bk_masks(&mut x, w, h) {
            return;
        }
        x.x = b.x;
        x.y = b.y;
        x.tol = b.rtol;
        x.flags = b.rflags;
        x.aa = b.raa != 0;
    }
    // small images at once, large ones on the worker pool
    let px = w as u32 as u64 * h as u32 as u64;
    if b.sync || (!b.force && px < BK_ASYNC_PX) {
        bk_compute(&mut b.ctx.lock(), &|| false);
        bk_install(p);
        return;
    }
    let ctx = b.ctx.clone();
    b.job.set(1, move |_, job| bk_compute(&mut ctx.lock(), &|| job.cancelled()));
    b.jcancel = false;
    if b.job.start() {
        b.jrun = true;
    }
    bk_sync_flag(b);
}

/// bk_poll() -- a finished background fill is shown (or discarded when superseded)
pub(crate) fn bk_poll(p: &mut Pn, ui: &mut Ui) {
    let b = &mut p.tbucket;
    if !b.jrun || !b.job.done() {
        return;
    }
    b.jrun = false;
    if b.jpend {
        b.jpend = false;
        bk_sync_flag(b);
        bk_request(p);
    } else {
        bk_sync_flag(b);
        if !b.jcancel && b.live {
            bk_install(p);
        }
    }
    ui.repass();
}

// bk_wait(): no background fill running any more (cancelled and waited for)
fn bk_wait(b: &mut TBucket) {
    b.jpend = false;
    if b.jrun {
        b.jcancel = true;
        b.job.cancel();
        let _ = b.job.wait();
        b.jrun = false;
    }
    bk_sync_flag(b);
}

// bk_snapshot(doc): private copy of the sampled pixels (layer or composite without the
// fill's own preview); remembers the document versions it was taken from
fn bk_snapshot(p: &mut Pn, doc: DocId) {
    bk_wait(&mut p.tbucket);
    let smp = p.tool.cfg.sample;
    p.tbucket.vsmp = smp;
    let mut shown = false;
    let snap = if smp == SMP_IMAGE as u32 {
        shown = p.tool.tls[TL_BUCKET].shown;
        tl_hide(p, TL_BUCKET);
        Some(sf_copy_argb(comp_surface(p, doc)))
    } else {
        doc_cur_layer(p.doc.d(doc)).map(|l| sf_copy_argb(&l.sf))
    };
    p.tbucket.ctx.lock().src = snap.unwrap_or_default();
    if shown {
        tl_show(p, TL_BUCKET);
    }
    let d = p.doc.d(doc);
    let b = &mut p.tbucket;
    b.vpx = d.ver_px;
    b.vlay = d.ver_lay;
    b.vcur = d.cur;
    b.vsel = d.ver_sel;
}

// a straight copy of a surface (an empty surface stays empty)
fn sf_copy_argb(s: &Surface) -> Surface {
    if s.px.is_empty() {
        return Surface::default();
    }
    let mut c = Surface::new(s.w, s.h, FMT_ARGB);
    if c.px.is_empty() {
        return c;
    }
    let n = s.w as usize * 4;
    for y in 0..s.h as usize {
        let a = y * s.stride as usize;
        let b = y * c.stride as usize;
        c.px[b..b + n].copy_from_slice(&s.px[a..a + n]);
    }
    c
}

// bk_end(): the live fill is over: nub hidden, snapshot and masks freed
fn bk_end(p: &mut Pn, ui: &mut Ui) {
    bk_wait(&mut p.tbucket);
    let b = &mut p.tbucket;
    b.live = false;
    b.drag = false;
    b.nub.flags = (NBF_HIDDEN | NBF_DRAWN) as u32;
    nubs_damage(&p.view, ui, core::slice::from_mut(&mut b.nub));
    {
        let mut c = b.ctx.lock();
        c.src = Surface::default();
        c.flood = None;
        c.fill = None;
    }
    b.mf = [None, None];
    p.tool.tls[TL_BUCKET].mask = TlMask::None;
}

// ------------------------------------------------------------------ TOOLDESC callbacks

// bk_input(doc, ui, x_fx, y_fx) -> changed
fn bk_input(p: &mut Pn, ui: &mut Ui, doc: DocId, inp: &UiInput, x_fx: i32, y_fx: i32) -> bool {
    let sx = inp.x + inp.rect[0];
    let sy = inp.y + inp.rect[1];
    if p.tbucket.drag {
        // dragging the origin nub
        let b = &mut p.tbucket;
        if inp.btn & (1 << b.dragbtn) == 0 {
            b.drag = false;
            b.nub.flags &= !(NBF_DRAG as u32);
            nubs_damage(&p.view, ui, core::slice::from_mut(&mut b.nub));
            return true;
        }
        if Some(doc) != b.doc {
            return false;
        }
        let d = p.doc.d(doc);
        let x = (x_fx >> 8).max(0).min(d.w - 1);
        let y = (y_fx >> 8).max(0).min(d.h - 1);
        let b = &mut p.tbucket;
        if x == b.x && y == b.y {
            return false;
        }
        b.x = x;
        b.y = y;
        nub_set(&p.view, ui, &mut b.nub, (x << 8) + 128, (y << 8) + 128);
        bk_request(p);
        return true;
    }
    let btn = if inp.pressed & (1 << BTN_LEFT) != 0 {
        BTN_LEFT
    } else if inp.pressed & (1 << BTN_RIGHT) != 0 {
        BTN_RIGHT
    } else {
        // hover: the nub lights up
        let b = &mut p.tbucket;
        if !b.live || Some(doc) != b.doc {
            return false;
        }
        return nubs_hover(&p.view, ui, core::slice::from_mut(&mut b.nub), sx, sy);
    };
    let b = &mut p.tbucket;
    if b.live && Some(doc) == b.doc && nubs_hit(&p.view, ui, core::slice::from_ref(&b.nub), sx, sy) >= 0 {
        b.drag = true;
        b.dragbtn = btn;
        b.nub.flags |= NBF_DRAG as u32;
        nubs_damage(&p.view, ui, core::slice::from_mut(&mut b.nub));
        return true;
    }
    // a click inside the image: (commit the fill before and) start a new one
    let x = x_fx >> 8;
    let y = y_fx >> 8;
    let d = p.doc.d(doc);
    if x as u32 >= d.w as u32 || y as u32 >= d.h as u32 {
        return false;
    }
    if p.tbucket.live {
        tool_commit_live(p, ui);
    }
    let b = &mut p.tbucket;
    b.live = true;
    b.doc = Some(doc);
    b.x = x;
    b.y = y;
    b.btn = btn;
    b.shift = (inp.mods & MOD_SHIFT != 0) as u32; // Shift: global mode (FF_GLOBAL = 1)
    tl_init(p, TL_BUCKET, doc);
    tl_set_paint(p, TL_BUCKET, btn);
    p.tbucket.pver = p.tool.cfg.ver;
    p.tbucket.palv = pal_version(&p.pal);
    let b = &mut p.tbucket;
    b.nub.x = (x << 8) + 128;
    b.nub.y = (y << 8) + 128;
    b.nub.kind = NK_CIRCLE as u32;
    b.nub.flags = 0;
    nubs_damage(&p.view, ui, core::slice::from_mut(&mut b.nub));
    bk_snapshot(p, doc);
    bk_request(p);
    tool_live_begin(p, doc);
    true
}

// bk_tick(doc): background results, document / option / colour changes of a live fill
fn bk_tick(p: &mut Pn, ui: &mut Ui, doc: Option<DocId>) {
    bk_poll(p, ui);
    let Some(doc) = doc else { return };
    if !p.tbucket.live || Some(doc) != p.tbucket.doc {
        return;
    }
    // the pointer left the canvas: the nub is no longer hovered
    let b = &mut p.tbucket;
    if b.nub.flags & NBF_HOVER as u32 != 0 && !view_cursor(&p.view).2 {
        b.nub.flags &= !(NBF_HOVER as u32);
        nubs_damage(&p.view, ui, core::slice::from_mut(&mut b.nub));
    }
    let c = p.tool.cfg;
    let d = p.doc.d(doc);
    let (vpx, vlay, vcur) = (d.ver_px, d.ver_lay, d.cur);
    let b = &p.tbucket;
    // the sampled pixels changed (undo, another layer, sampling): new snapshot + fill
    let resnap = vpx != b.vpx || vlay != b.vlay || vcur != b.vcur || c.sample != b.vsmp;
    // fill options
    let rerun = c.tol != b.rtol || (c.flood | b.shift) & FF_GLOBAL != b.rflags || c.aa != b.raa;
    if resnap {
        bk_snapshot(p, doc);
    }
    if resnap || rerun {
        bk_request(p);
    }
    // colours, pattern, blend mode: re-render
    let palv = pal_version(&p.pal);
    let b = &mut p.tbucket;
    if c.ver != b.pver || palv != b.palv {
        b.pver = c.ver;
        b.palv = palv;
        let btn = b.btn;
        tl_set_paint(p, TL_BUCKET, btn);
    } else {
        let vsel = p.doc.d(doc).ver_sel;
        if vsel == b.vsel {
            return;
        }
        b.vsel = vsel;
    }
    let bx = p.tool.tls[TL_BUCKET].bx;
    tl_set_box(p, TL_BUCKET, bx[0], bx[1], bx[2], bx[3]);
}

// bk_commit(doc) -> step: the fill into the layer ("Farbeimer")
fn bk_commit(p: &mut Pn, ui: &mut Ui, _doc: DocId) -> bool {
    if !p.tbucket.live {
        return false;
    }
    if p.tbucket.jrun || p.tbucket.jpend {
        // a fill is still being computed: finish it here with the latest request
        bk_wait(&mut p.tbucket);
        p.tbucket.sync = true;
        bk_request(p);
        p.tbucket.sync = false;
    }
    let r = tl_commit(p, TL_BUCKET, "Farbeimer", IC_DROP);
    bk_end(p, ui);
    r
}

// bk_cancel(doc): drop the live fill (no history; doc None = the image is gone)
fn bk_cancel(p: &mut Pn, ui: &mut Ui, doc: Option<DocId>) {
    if !p.tbucket.live {
        return;
    }
    bk_wait(&mut p.tbucket);
    if doc.is_some() {
        tl_hide(p, TL_BUCKET);
    }
    p.tool.tls[TL_BUCKET].shown = false;
    bk_end(p, ui);
}

// bk_cursor(x_fx, y_fx) -> CUR_MOVE over / while dragging the nub, else the crosshair
fn bk_cursor(p: &mut Pn, _x: i32, _y: i32) -> u32 {
    if p.tbucket.live && p.tbucket.nub.flags & (NBF_HOVER | NBF_DRAG) as u32 != 0 {
        return CUR_MOVE;
    }
    CUR_CROSS
}

// bk_overlay(doc, &rect): the origin nub of the live fill
fn bk_overlay(p: &mut Pn, g: &mut Gfx, doc: DocId, _rect: [i32; 4]) {
    if p.tbucket.live && Some(doc) == p.tbucket.doc {
        nubs_draw(&p.view, g, core::slice::from_mut(&mut p.tbucket.nub));
    }
}

// bk_options(): Füllungsmodus, Toleranz, Ebene / Bild, Fülleffekt, Füllmethode, Antialiasing
// (the framework adds Fertigstellen)
fn bk_options(p: &mut Pn, ui: &mut Ui) {
    opt_flood(p, ui);
    opt_sep(ui);
    opt_tolerance(p, ui);
    opt_sep(ui);
    opt_sampling(p, ui);
    opt_sep(ui);
    opt_fill(p, ui);
    opt_sep(ui);
    opt_blend(p, ui);
    opt_aa(p, ui);
}
