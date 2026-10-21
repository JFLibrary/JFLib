// compose -- PaintCraft: the composite image of a document (all visible layers blended
// bottom-up with k_bm_row), 64 x 64 tile dirty tracking, the premultiplied display mip chain
// (view below 100 %) and thumbnails.  Block DOC_COMP (Doc::comp).
//                                                                [apps/PaintCraft/src/compose.s]
//
//   comp_init(doc) -> 0 / ERR_NOMEM        (re)build for DOC_W x DOC_H; everything dirty
//   comp_free(doc)                          release composite, bitsets, mips
//   comp_mark(doc, x0, y0, x1, y1)          area changed (x1 / y1 exclusive, clipped): tiles and
//                                           mips dirty, CO_VER++, view damage (no DF_MODIFIED)
//   comp_mark_all(doc)
//   comp_update(doc)                        recomposite the dirty tiles (worker threads)
//   comp_surface(doc) -> &Surface           comp_update + the composite (empty = no memory)
//   comp_pixel(doc, x, y) -> argb           straight ARGB of the composite (0 outside)
//   comp_set_tool(doc, hook)                live tool hook (pn_doc.inc); None removes it
//   comp_damage_take(doc) -> rect           the changed area since the last call (doc px,
//                                           x0, y0, x1, y1) and clear it; None = none
//   comp_mip(doc, level) -> (&Surface, level)   premultiplied level 1..6 (clamped to the
//                                           chain; level k = 2^-k), up to date
//   thumb_render(dst, src, cell)            dst (FMT_PARGB, its size) = src (straight) scaled
//                                           with 4 x 4 supersampling over a checkerboard of
//                                           `cell` px (opaque result)
//   doc_thumb(doc, w, h, cell) -> &Surface  DOC_THUMB refreshed now (w x h px)
//   lay_thumb(doc, i, w, h, cell) -> &Surface   layer i's thumbnail, refreshed when the layer
//                                           changed; None on failure
//
// Fast path: exactly one visible layer in Normal mode at opacity 255 and no tool hook ->
// the composite is that layer's surface (no copy, Comp::alias; comp_sf resolves it).
//
// The tool hook (CO_TOOLFN / CO_TOOLCTX): a module installs a `HookKey` with comp_set_tool;
// comp_update resolves it (hook_get: the owner's `*_hook(p)` returns a CompHook borrowing its
// state) and calls CompHook::row on the worker threads with the document (its composite
// block is taken out for the time of the update).  CompHook implementations must be Sync.
// The `*_d` functions work on one document record (no Pn): doc.rs / layer.rs build documents
// with them before they are in the list; they return true where the asm calls ui_invalidate.

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

use gfx::straight::premul_row;
use gfx::{Surface, FMT_ARGB, FMT_PARGB};
use rt::sync::SpinLock;

use crate::app::constants::*;
use crate::document::model::{Doc, DocId};
use crate::app::Pn;

const MIPL_MAX: usize = 6; // 2^-6 = 1.5625 % (zoom >= 1 % samples level 6)
const THUMB_C0: u32 = 0xFFFFFF; // thumbnail checkerboard (like the canvas)
const THUMB_C1: u32 = 0xCCCCCC;
const BM_NORMAL: u8 = 0;

/// who installed the compose tool hook of a document (CO_TOOLFN + CO_TOOLCTX)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum HookKey {
    /// tool.rs tl_row of tool layer p.tool.tls[slot] (TL_* slot)
    Tl(usize),
    /// fx.rs fx_row (the running effect)
    Fx,
    /// dlg_rotzoom.rs (the Rotationszoom preview)
    RotZoom,
    /// t_move.rs (moved pixels)
    Move,
    /// diag.rs test hook 909
    Diag,
}

/// the live tool layer hook: fn(ctx, dst, x, y, n, layer_row) -> 1 = dst holds the active
/// layer's row x..x+n-1 of line y as it should be shown (false = use layer_row unchanged).
/// Worker threads (pure code only), before blending; n <= 64; dst (64 px) and layer_row
/// (n px) are straight ARGB bytes.  `doc` is the document (its comp block is empty then).
pub(crate) trait CompHook: Sync {
    fn row(&self, doc: &Doc, dst: &mut [u8], x: i32, y: i32, n: i32, layer_row: &[u8]) -> bool;
}

/// hook_get(key) -> the CompHook of a key (None = not available now: no hook)
pub(crate) fn hook_get(p: &Pn, k: HookKey) -> Option<Box<dyn CompHook + '_>> {
    match k {
        HookKey::Tl(slot) => crate::tools::tl_hook(p, slot),
        HookKey::Fx => crate::effects::fx_hook(p),
        HookKey::RotZoom => crate::workspace::dialogs::rotate_zoom::rz_hook(p),
        HookKey::Move => crate::tools::move_pixels::tmove_hook(p),
        HookKey::Diag => crate::diagnostics::diag_hook(p),
    }
}

/// the display mip chain (MC_*): premultiplied levels 1..n
#[derive(Default)]
pub(crate) struct Mip {
    /// MC_N: levels in the chain (1..MIPL_MAX)
    pub(crate) n: usize,
    /// MC_W / MC_H: base size it was built for
    pub(crate) w: i32,
    pub(crate) h: i32,
    /// level k at index k - 1
    pub(crate) sfs: Vec<Surface>,
}

/// the DOC_COMP block (CO_*).  Part of Doc: Send + Sync.
#[derive(Default)]
pub(crate) struct Comp {
    /// CO_SF: straight composite (FMT_ARGB); unused while `alias`
    pub(crate) sf: Surface,
    /// CO_ALIAS: the composite is the surface of layer `aliasl`
    pub(crate) alias: bool,
    /// CO_ALIASL: the layer index it aliases
    pub(crate) aliasl: i32,
    /// CO_TW / CO_TH: tiles across / down
    pub(crate) tw: u32,
    pub(crate) th: u32,
    /// CO_BITS: u64 words per bitset
    pub(crate) bits: u32,
    /// CO_DIRTY: tiles to recomposite (empty = not allocated)
    pub(crate) dirty: Vec<u64>,
    /// CO_MDIRTY: tiles whose display mips are stale
    pub(crate) mdirty: Vec<u64>,
    /// CO_MIP: the mip chain
    pub(crate) mip: Option<Box<Mip>>,
    /// CO_TOOLFN / CO_TOOLCTX: the live tool hook
    pub(crate) tool: Option<HookKey>,
    /// CO_VER: bumped on every composite change
    pub(crate) ver: u32,
    /// CO_NDIRTY: CO_DIRTY has set bits
    pub(crate) ndirty: bool,
    /// CO_DMG: changed area not yet shown by the view (doc px, x1 <= x0 = none)
    pub(crate) dmg: [i32; 4],
}

/// the state of compose.s (its job contexts are locals here)
#[derive(Default)]
pub(crate) struct Compose {}

// ------------------------------------------------------------------ setup

// comp_release(doc) -- free composite (when owned), bitsets and the mip chain
fn comp_release(co: &mut Comp) {
    co.sf = Surface::default();
    co.alias = false;
    co.dirty = Vec::new();
    co.mdirty = Vec::new();
    co.ndirty = false;
    co.mip = None;
}

/// comp_free(doc)
pub(crate) fn comp_free(p: &mut Pn, doc: DocId) {
    if let Some(d) = p.doc.get_mut(doc) {
        comp_release(&mut d.comp);
    }
}

/// comp_init(doc) -> 0 / ERR_NOMEM
pub(crate) fn comp_init(p: &mut Pn, doc: DocId) -> i32 {
    let Some(d) = p.doc.get_mut(doc) else { return 0 };
    let (w, h) = (d.w, d.h);
    let r = comp_init_co(&mut d.comp, w, h);
    p.invalidate();
    r
}

/// comp_init of a document record (comp_mark_all included: the UI must be invalidated)
pub(crate) fn comp_init_d(d: &mut Doc) -> i32 {
    let (w, h) = (d.w, d.h);
    comp_init_co(&mut d.comp, w, h)
}

fn comp_init_co(co: &mut Comp, w: i32, h: i32) -> i32 {
    comp_release(co);
    co.tw = ((w as u32).wrapping_add(TILE as u32 - 1)) >> TILE_SH;
    co.th = ((h as u32).wrapping_add(TILE as u32 - 1)) >> TILE_SH;
    co.bits = co.tw.wrapping_mul(co.th).wrapping_add(63) >> 6;
    co.dirty = vec![0u64; co.bits as usize];
    co.mdirty = vec![0u64; co.bits as usize];
    mark_all_co(co, w, h);
    0
}

// set_all_bits(doc) -- every tile needs compositing and new mips (no damage)
fn set_all_bits(co: &mut Comp) {
    co.dirty.fill(!0);
    co.mdirty.fill(!0);
    co.ndirty = true;
}

fn mark_all_co(co: &mut Comp, w: i32, h: i32) {
    set_all_bits(co);
    co.dmg = [0, 0, w, h];
    co.ver = co.ver.wrapping_add(1);
}

/// comp_mark_all(doc)
pub(crate) fn comp_mark_all(p: &mut Pn, doc: DocId) {
    if let Some(d) = p.doc.get_mut(doc) {
        comp_mark_all_d(d);
        p.invalidate();
    }
}

/// comp_mark_all of a document record (the UI must be invalidated)
pub(crate) fn comp_mark_all_d(d: &mut Doc) {
    let (w, h) = (d.w, d.h);
    mark_all_co(&mut d.comp, w, h);
}

/// comp_mark(doc, x0, y0, x1, y1)
pub(crate) fn comp_mark(p: &mut Pn, doc: DocId, x0: i32, y0: i32, x1: i32, y1: i32) {
    if let Some(d) = p.doc.get_mut(doc) {
        if comp_mark_d(d, x0, y0, x1, y1) {
            p.invalidate();
        }
    }
}

/// comp_mark of a document record -> true when something was marked (ui_invalidate)
pub(crate) fn comp_mark_d(d: &mut Doc, x0: i32, y0: i32, x1: i32, y1: i32) -> bool {
    let x0 = x0.max(0);
    let y0 = y0.max(0);
    let x1 = x1.min(d.w);
    let y1 = y1.min(d.h);
    if x0 >= x1 || y0 >= y1 {
        return false;
    }
    let co = &mut d.comp;
    // view damage: union with the pending rectangle
    if co.dmg[2] > co.dmg[0] {
        co.dmg[0] = co.dmg[0].min(x0);
        co.dmg[1] = co.dmg[1].min(y0);
        co.dmg[2] = co.dmg[2].max(x1);
        co.dmg[3] = co.dmg[3].max(y1);
    } else {
        co.dmg = [x0, y0, x1, y1];
    }
    if !co.dirty.is_empty() && !co.mdirty.is_empty() {
        let tx0 = (x0 as u32) >> TILE_SH;
        let ty0 = (y0 as u32) >> TILE_SH;
        let tx1 = ((x1 - 1) as u32) >> TILE_SH;
        let ty1 = ((y1 - 1) as u32) >> TILE_SH;
        let tw = co.tw;
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                let bit = ty.wrapping_mul(tw).wrapping_add(tx);
                let wd = (bit >> 6) as usize;
                if wd < co.dirty.len() {
                    co.dirty[wd] |= 1u64 << (bit & 63);
                    co.mdirty[wd] |= 1u64 << (bit & 63);
                }
            }
        }
    }
    co.ndirty = true;
    co.ver = co.ver.wrapping_add(1);
    true
}

/// comp_set_tool(doc, hook) -- the next comp_update uses the hook (the caller marks the areas
/// whose preview changes with comp_mark)
pub(crate) fn comp_set_tool(p: &mut Pn, doc: DocId, hook: Option<HookKey>) {
    if let Some(d) = p.doc.get_mut(doc) {
        d.comp.tool = hook;
    }
}

/// comp_damage_take(doc) -> x0, y0, x1, y1 (cleared), or None
pub(crate) fn comp_damage_take(d: &mut Doc) -> Option<[i32; 4]> {
    let co = &mut d.comp;
    if co.dmg[2] <= co.dmg[0] {
        return None;
    }
    let r = co.dmg;
    co.dmg = [0; 4];
    Some(r)
}

// ------------------------------------------------------------------ compositing

/// the composite as it is (no update): the aliased layer's surface or CO_SF
pub(crate) fn comp_sf(d: &Doc) -> &Surface {
    comp_sf_co(d, &d.comp)
}

fn comp_sf_co<'a>(d: &'a Doc, co: &'a Comp) -> &'a Surface {
    if co.alias {
        if let Some(l) = d.layers.get(co.aliasl as usize) {
            return &l.sf;
        }
    }
    &co.sf
}

// comp_mode(doc) -- alias the single visible Normal / 255 layer, or own a composite surface
fn comp_mode(d: &Doc, co: &mut Comp) {
    let mut count = 0;
    let mut idx = -1i32;
    for (i, l) in d.layers.iter().enumerate() {
        if l.flags & LYF_VISIBLE as u32 != 0 && !l.sf.px.is_empty() {
            count += 1;
            idx = i as i32;
        }
    }
    if count == 1 && co.tool.is_none() {
        let l = &d.layers[idx as usize];
        if l.sf.blend == BM_NORMAL && l.sf.opac == 255 {
            if !co.alias && !co.sf.px.is_empty() {
                co.sf = Surface::default();
            }
            co.alias = true;
            co.aliasl = idx;
            return;
        }
    }
    if co.alias || co.sf.px.is_empty() {
        co.alias = false;
        co.sf = Surface::new(d.w, d.h, FMT_ARGB);
        if !co.sf.px.is_empty() {
            set_all_bits(co);
        }
    }
}

/// comp_update(doc) -- bring the composite up to date
pub(crate) fn comp_update(p: &mut Pn, doc: DocId) {
    let Some(d) = p.doc.get_mut(doc) else { return };
    if d.comp.dirty.is_empty() {
        return;
    }
    let mut co = core::mem::take(&mut d.comp);
    let inval = {
        let pr: &Pn = p;
        let hook = co.tool.and_then(|k| hook_get(pr, k));
        comp_update_co(pr.doc.d(doc), &mut co, hook.as_deref())
    };
    p.doc.dm(doc).comp = co;
    if inval {
        p.invalidate();
    }
}

/// comp_update of a document whose comp block is `co` (taken out of it) -> true when the
/// composite was rebuilt (ui_invalidate)
fn comp_update_co(d: &Doc, co: &mut Comp, hook: Option<&dyn CompHook>) -> bool {
    let mut inval = false;
    // a new image size (undo of a resize ...): rebuild for it
    let tw = ((d.w as u32).wrapping_add(TILE as u32 - 1)) >> TILE_SH;
    let th = ((d.h as u32).wrapping_add(TILE as u32 - 1)) >> TILE_SH;
    let reinit = tw != co.tw
        || th != co.th
        || (!co.alias && !co.sf.px.is_empty() && (co.sf.w != d.w || co.sf.h != d.h));
    if reinit {
        comp_init_co(co, d.w, d.h);
        inval = true;
    }
    comp_mode(d, co);
    if !co.alias {
        if !co.ndirty || co.sf.px.is_empty() {
            return inval;
        }
        // the list of visible layers
        let mut list: Vec<(&Surface, u32)> = Vec::with_capacity(d.layers.len());
        let mut act = -1i32;
        for (i, l) in d.layers.iter().enumerate() {
            if l.flags & LYF_VISIBLE as u32 == 0 || l.sf.px.is_empty() {
                continue;
            }
            if i as i32 == d.cur {
                act = list.len() as i32;
            }
            list.push((&l.sf, (l.sf.opac as u32) << 8 | l.sf.blend as u32));
        }
        let (w, h, stride) = (d.w, d.h, co.sf.stride as usize);
        let tw = co.tw;
        let bits = &co.dirty;
        let band = TILE as usize * stride;
        let bands: Vec<SpinLock<&mut [u8]>> = co.sf.px.chunks_mut(band).map(SpinLock::new).collect();
        let job = |ty: usize, _: &plat::JobCtl| {
            if let Some(b) = bands.get(ty) {
                let mut g = b.lock();
                comp_job(d, &list, act, hook, bits, tw, w, h, stride, ty as u32, &mut g);
            }
        };
        let _ = plat::parallel_for(co.th as usize, &job);
    }
    co.dirty.fill(0);
    co.ndirty = false;
    inval
}

// comp_job(ctx, ty, job) -- worker: recomposite the dirty tiles of tile row ty (band = its
// rows of the composite)
#[allow(clippy::too_many_arguments)]
fn comp_job(
    d: &Doc,
    list: &[(&Surface, u32)],
    act: i32,
    hook: Option<&dyn CompHook>,
    bits: &[u64],
    tw: u32,
    w: i32,
    h: i32,
    stride: usize,
    ty: u32,
    band: &mut [u8],
) {
    let y0 = (ty << TILE_SH) as i32;
    let y1 = (y0 + TILE).min(h);
    let mut tool = [0u8; 64 * 4];
    for tx in 0..tw {
        let bit = ty * tw + tx;
        if bits.get((bit >> 6) as usize).is_none_or(|wd| wd >> (bit & 63) & 1 == 0) {
            continue;
        }
        let x0 = (tx << TILE_SH) as i32;
        let n = (w - x0).min(TILE);
        let nb = n as usize * 4;
        for y in y0..y1 {
            let o = (y - y0) as usize * stride + x0 as usize * 4;
            let dst = &mut band[o..o + nb];
            dst.fill(0);
            for (li, &(sf, mode)) in list.iter().enumerate() {
                let lo = y as usize * sf.stride as usize + x0 as usize * 4;
                let mut src = &sf.px[lo..lo + nb];
                if li as i32 == act {
                    if let Some(hk) = hook {
                        if hk.row(d, &mut tool, x0, y, n, src) {
                            src = &tool[..nb];
                        }
                    }
                }
                gfx::blendmode::bm_row_inplace(dst, src, mode);
            }
        }
    }
}

/// comp_surface(doc) -> the composite (up to date)
pub(crate) fn comp_surface(p: &mut Pn, doc: DocId) -> &Surface {
    comp_update(p, doc);
    comp_sf(p.doc.d(doc))
}

/// comp_pixel(doc, x, y) -> straight ARGB (0 outside or without pixels)
pub(crate) fn comp_pixel(p: &mut Pn, doc: DocId, x: i32, y: i32) -> u32 {
    let Some(d) = p.doc.get(doc) else { return 0 };
    if x as u32 >= d.w as u32 || y as u32 >= d.h as u32 {
        return 0;
    }
    let sf = comp_surface(p, doc);
    if sf.px.is_empty() {
        return 0;
    }
    let o = y as usize * sf.stride as usize + x as usize * 4;
    u32::from_le_bytes([sf.px[o], sf.px[o + 1], sf.px[o + 2], sf.px[o + 3]])
}

// ------------------------------------------------------------------ display mips

// mip_alloc(doc) -> the chain for the current size (new ones are all dirty) or None
fn mip_alloc(d: &Doc, co: &mut Comp) -> bool {
    if let Some(m) = &co.mip {
        if m.w == d.w && m.h == d.h {
            return true;
        }
        co.mip = None;
    }
    let mut m = Box::new(Mip { n: 0, w: d.w, h: d.h, sfs: Vec::with_capacity(MIPL_MAX) });
    let (mut w, mut h) = (d.w as u32, d.h as u32);
    while m.sfs.len() < MIPL_MAX {
        if w == 1 && h == 1 {
            break;
        }
        w = w.wrapping_add(1) >> 1;
        h = h.wrapping_add(1) >> 1;
        let s = Surface::new(w as i32, h as i32, FMT_PARGB);
        if s.px.is_empty() {
            return false;
        }
        m.sfs.push(s);
        m.n = m.sfs.len();
    }
    if m.n == 0 {
        return false;
    }
    co.mdirty.fill(!0);
    co.mip = Some(m);
    true
}

/// comp_mip(doc, level) -> (premultiplied level, the level) (clamped 1..n) or None
pub(crate) fn comp_mip(p: &mut Pn, doc: DocId, level: i32) -> Option<(&Surface, i32)> {
    let k = comp_mip_level(p, doc, level)?;
    Some((comp_mip_sf(p.doc.d(doc), k)?, k))
}

/// comp_mip without the surface: bring level `level` (clamped) up to date -> the level
pub(crate) fn comp_mip_level(p: &mut Pn, doc: DocId, level: i32) -> Option<i32> {
    p.doc.get(doc)?;
    comp_update(p, doc);
    let d = p.doc.dm(doc);
    let mut co = core::mem::take(&mut d.comp);
    let r = comp_mip_co(d, &mut co, level);
    d.comp = co;
    r
}

/// level k of the mip chain as it is (k from comp_mip_level)
pub(crate) fn comp_mip_sf(d: &Doc, k: i32) -> Option<&Surface> {
    d.comp.mip.as_ref()?.sfs.get((k - 1) as usize)
}

fn comp_mip_co(d: &Doc, co: &mut Comp, level: i32) -> Option<i32> {
    if comp_sf_co(d, co).px.is_empty() {
        return None;
    }
    if !mip_alloc(d, co) {
        return None;
    }
    let n = co.mip.as_ref().map_or(0, |m| m.n) as i32;
    let k = level.max(1).min(n);
    if co.mdirty.iter().any(|&w| w != 0) {
        mip_refresh(d, co);
    }
    Some(k)
}

// mip_refresh(doc) -- recompute every level for the stale tiles, level by level
fn mip_refresh(d: &Doc, co: &mut Comp) {
    let Some(mut m) = co.mip.take() else { return };
    let tw = co.tw;
    let th = co.th as usize;
    let bits = &co.mdirty;
    for k in 1..=m.n {
        let (lo, hi) = m.sfs.split_at_mut(k - 1);
        let src: &Surface = if k == 1 { comp_sf_co(d, co) } else { &lo[k - 2] };
        let dst = &mut hi[0];
        let (dw, dh, dstride) = (dst.w, dst.h, dst.stride as usize);
        let rows = (TILE as usize) >> k;
        let bands: Vec<SpinLock<&mut [u8]>> = dst.px.chunks_mut(rows * dstride).map(SpinLock::new).collect();
        let job = |ty: usize, _: &plat::JobCtl| {
            if let Some(b) = bands.get(ty) {
                let mut g = b.lock();
                mip_job(src, dw, dh, dstride, k as u32, bits, tw, ty as u32, &mut g);
            }
        };
        let _ = plat::parallel_for(th, &job);
    }
    co.mip = Some(m);
    co.mdirty.fill(0);
}

// mip_job(ctx, ty, job) -- worker: level k of the stale tiles in tile row ty (k <= 6: a
// level-0 tile maps to exactly (64 >> k)^2 pixels of level k, so tiles never overlap)
#[allow(clippy::too_many_arguments)]
fn mip_job(src: &Surface, dw: i32, dh: i32, dstride: usize, k: u32, bits: &[u64], tw: u32, ty: u32, band: &mut [u8]) {
    let yo0 = ((ty << TILE_SH) >> k) as i32;
    let yo1 = ((((ty + 1) << TILE_SH) >> k) as i32).min(dh);
    let sstride = src.stride as usize;
    let mut pa = [0u8; 128 * 4];
    let mut pb = [0u8; 128 * 4];
    for tx in 0..tw {
        let bit = ty * tw + tx;
        if bits.get((bit >> 6) as usize).is_none_or(|wd| wd >> (bit & 63) & 1 == 0) {
            continue;
        }
        let xo0 = ((tx << TILE_SH) >> k) as i32;
        let xo1 = ((((tx + 1) << TILE_SH) >> k) as i32).min(dw);
        let nout = xo1 - xo0;
        if nout <= 0 {
            continue;
        }
        let nsrc = (nout + nout).min(src.w - xo0 - xo0).min(128);
        if nsrc <= 0 {
            continue;
        }
        let ns = nsrc as usize * 4;
        for yo in yo0..yo1 {
            // source rows 2 yo and min(2 yo + 1, h - 1)
            let last = src.h - 1;
            let ya = (yo + yo).min(last) as usize;
            let yb = (yo + yo + 1).min(last) as usize;
            let off = xo0 as usize * 8;
            let ra = &src.px[ya * sstride + off..ya * sstride + off + ns];
            let rb = &src.px[yb * sstride + off..yb * sstride + off + ns];
            let o = (yo - yo0) as usize * dstride + xo0 as usize * 4;
            let dst = &mut band[o..o + nout as usize * 4];
            if k == 1 {
                // level 1 reads the straight composite: premultiply both spans first
                premul_row(&mut pa[..ns], ra);
                premul_row(&mut pb[..ns], rb);
                mip_row(dst, &pa[..ns], &pb[..ns], nout as usize, nsrc as usize);
            } else {
                mip_row(dst, ra, rb, nout as usize, nsrc as usize);
            }
        }
    }
}

// mip_row(dst, a, b, nout, nsrc) -- dst[i] = rounded average of a/b[2i], a/b[2i + 1]
// (premultiplied; the pixel after the last source pixel repeats it)
fn mip_row(dst: &mut [u8], a: &[u8], b: &[u8], nout: usize, nsrc: usize) {
    for i in 0..nout {
        let s0 = 2 * i;
        if s0 >= nsrc {
            break;
        }
        let s1 = if s0 + 1 < nsrc { s0 + 1 } else { s0 };
        for c in 0..4 {
            let v = a[s0 * 4 + c] as u32 + a[s1 * 4 + c] as u32 + b[s0 * 4 + c] as u32 + b[s1 * 4 + c] as u32 + 2;
            dst[i * 4 + c] = (v >> 2) as u8;
        }
    }
}

// ------------------------------------------------------------------ thumbnails

/// thumb_render(dst, src, cell) -- dst (FMT_PARGB, its own size) = src (straight ARGB) with
/// 4 x 4 samples per pixel, composited over a checkerboard of `cell` px (opaque result)
pub(crate) fn thumb_render(dst: &mut Surface, src: &Surface, cell: i32) {
    let cell = cell.max(1) as u32;
    let (dw, dh, sw, sh) = (dst.w, dst.h, src.w, src.h);
    if dw <= 0 || dh <= 0 || sw <= 0 || sh <= 0 || src.px.is_empty() || dst.px.is_empty() {
        return;
    }
    let sstride = src.stride as i64;
    let dstride = dst.stride as usize;
    for y in 0..dh as u32 {
        for x in 0..dw as u32 {
            let (mut sa, mut sr, mut sg, mut sb) = (0u32, 0u32, 0u32, 0u32);
            for j in 0..4u32 {
                // source row: sy = ((y * 8 + 2 j + 1) * sh) / (dh * 8)
                let sy = ((y.wrapping_mul(8).wrapping_add(2 * j + 1)) as u64 * sh as u64) / (dh as u64 * 8);
                let row = (sy as i64 * sstride) as usize;
                for i in 0..4u32 {
                    let sx = ((x.wrapping_mul(8).wrapping_add(2 * i + 1)) as u64 * sw as u64) / (dw as u64 * 8);
                    let o = row + sx as usize * 4;
                    let px = u32::from_le_bytes([src.px[o], src.px[o + 1], src.px[o + 2], src.px[o + 3]]);
                    let a = px >> 24;
                    sa += a;
                    sb += (px & 255) * a;
                    sg += (px >> 8 & 255) * a;
                    sr += (px >> 16 & 255) * a;
                }
            }
            let alpha = (sa + 8) >> 4;
            let ia = 255 - alpha;
            let bg = if (x / cell + y / cell) & 1 == 0 { THUMB_C0 } else { THUMB_C1 };
            let ch = |sum: u32, sh: u32| -> u32 {
                let v = (sum + 2040) / 4080;
                let b = (bg >> sh) & 255;
                let t = ((b * ia + 128) * 257) >> 16;
                (v + t).min(255) << sh
            };
            let out = ch(sb, 0) | ch(sg, 8) | ch(sr, 16) | 0xFF00_0000;
            let o = y as usize * dstride + x as usize * 4;
            dst.px[o..o + 4].copy_from_slice(&out.to_le_bytes());
        }
    }
}

/// doc_thumb(doc, w, h, cell) -> DOC_THUMB rendered now (w x h px), None on failure
pub(crate) fn doc_thumb(p: &mut Pn, doc: DocId, w: i32, h: i32, cell: i32) -> Option<&Surface> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let d = p.doc.get_mut(doc)?;
    if d.thumb.px.is_empty() || d.thumb.w != w || d.thumb.h != h {
        d.thumb = Surface::new(w, h, FMT_PARGB);
        if d.thumb.px.is_empty() {
            return None;
        }
    }
    comp_update(p, doc);
    let d = p.doc.dm(doc);
    let mut th = core::mem::take(&mut d.thumb);
    let ok = {
        let sf = comp_sf(d);
        if sf.px.is_empty() {
            false
        } else {
            thumb_render(&mut th, sf, cell);
            true
        }
    };
    d.thumb = th;
    if !ok {
        return None;
    }
    d.thumb_ver = d.ver_px.wrapping_add(d.ver_lay);
    d.thumb_aux = d.thumb_aux.wrapping_add(1);
    Some(&d.thumb)
}

/// lay_thumb(doc, i, w, h, cell) -> thumbnail of layer i (LY_THUMB), rendered when the layer's
/// pixels changed since the last call; None on failure
pub(crate) fn lay_thumb(p: &mut Pn, doc: DocId, i: i32, w: i32, h: i32, cell: i32) -> Option<&Surface> {
    let d = p.doc.get_mut(doc)?;
    let n = d.layers.len() as i32;
    if n <= 0 || w <= 0 || h <= 0 {
        return None;
    }
    let l = &mut d.layers[i.clamp(0, n - 1) as usize];
    let mut render = false;
    if l.thumb.is_none() {
        l.thumb = Some(Box::default());
        render = true;
    }
    let th = l.thumb.as_mut()?;
    if th.px.is_empty() || th.w != w || th.h != h {
        **th = Surface::new(w, h, FMT_PARGB);
        if th.px.is_empty() {
            return None;
        }
        render = true;
    }
    let v = l.ver.wrapping_add(1);
    if render || v != l.thumb_ver {
        l.thumb_ver = v;
        thumb_render(th, &l.sf, cell);
        l.thumb_aux = l.thumb_aux.wrapping_add(1);
    }
    l.thumb.as_deref()
}
