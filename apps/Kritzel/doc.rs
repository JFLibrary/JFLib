// doc -- the Kritzel document: layers, composite cache, dirty tracking  [apps/paint/src/doc.s]
//
//   doc_new(p, w, h) -> 0 / -1        fresh white image (one layer)
//   doc_free(d)
//   doc_w(d)   doc_h(d)   doc_version(d)
//   doc_layer(d, i) -> &mut Surface   layer surface (PARGB)
//   doc_cur(d) -> &mut Surface        current layer surface
//   doc_mark(d, x0, y0, x1, y1)       pixels changed (doc coordinates, exclusive end)
//   doc_mark_all(d)
//   doc_damage_take(d) -> rect        the pixels marked since the last call (the canvas's
//                                     screen damage, view.rs view_damage)
//   doc_composite(d)                  brings the composite surface (d.comp) up to date
//   doc_pick(d, x, y) -> u32          straight ARGB of the composite at x, y (0 outside)
//   doc_opaque(d) -> bool             the composite has no transparency (no checker needed)
//   doc_set_modified(d, v)   doc_modified(d) -> bool
//
// State for other modules (the asm's globals): layers (MAX_LAYERS surfaces), nlayers,
// cur_layer, layer_vis, layer_opac, path, comp, bg_opaque.  The composite is shared with
// the canvas draw object (view.rs), which blits it at render time.

use alloc::rc::Rc;
use alloc::string::String;
use core::cell::RefCell;

use gfx::straight::unpremul_px;
use gfx::{blend, Surface, FMT_PARGB};

use crate::consts::*;
use crate::tools::tool_abort;
use crate::Kritzel;

/// the document (the .bss of doc.s)
pub(crate) struct Doc {
    pub(crate) layers: [Surface; MAX_LAYERS],
    /// the composite of the visible layers (shared with the canvas draw object)
    pub(crate) comp: Rc<RefCell<Surface>>,
    pub(crate) dirty: [i32; 4],
    pub(crate) layer_vis: [u8; MAX_LAYERS],
    pub(crate) layer_opac: [u8; MAX_LAYERS],
    /// the document's file path ("" = none; g_doc_path, PATH_CAP 1040)
    pub(crate) path: String,
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) nlayers: i32,
    pub(crate) cur_layer: i32,
    version: u32,
    pub(crate) dirty_set: bool,
    /// marked since the last doc_damage_take (x0, y0, x1, y1): redrawn on the screen
    dmg: [i32; 4],
    dmg_set: bool,
    modified: bool,
    /// 1 = the background layer may contain transparency
    pub(crate) bg_opaque: i32,
}

impl Default for Doc {
    fn default() -> Doc {
        Doc {
            layers: core::array::from_fn(|_| Surface::default()),
            comp: Rc::new(RefCell::new(Surface::default())),
            dirty: [0; 4],
            layer_vis: [0; MAX_LAYERS],
            layer_opac: [0; MAX_LAYERS],
            path: String::new(),
            w: 0,
            h: 0,
            nlayers: 0,
            cur_layer: 0,
            version: 0,
            dirty_set: false,
            dmg: [0; 4],
            dmg_set: false,
            modified: false,
            bg_opaque: 0,
        }
    }
}

/// doc_free() -- release every layer and the composite
pub(crate) fn doc_free(d: &mut Doc) {
    for l in d.layers.iter_mut() {
        *l = Surface::default();
    }
    *d.comp.borrow_mut() = Surface::default();
    d.nlayers = 0;
}

/// clear_sf(sf, argb) -- every pixel = argb (opaque; gfx_begin / gfx_clear / gfx_end)
pub(crate) fn clear_sf(sf: &mut Surface, argb: u32) {
    let (w, stride) = (sf.w as usize, sf.stride as usize);
    for y in 0..sf.h as usize {
        blend::fill_solid(&mut sf.px[y * stride..y * stride + w * 4], argb);
    }
}

/// doc_new(w, h) -> 0 ok / -1
pub(crate) fn doc_new(p: &mut Kritzel, w: i32, h: i32) -> i32 {
    tool_abort(p);
    let d = &mut p.doc;
    doc_free(d);
    d.w = w;
    d.h = h;
    d.layers[0] = Surface::new(w, h, FMT_PARGB);
    if d.layers[0].w != w || d.layers[0].h != h {
        return -1;
    }
    *d.comp.borrow_mut() = Surface::new(w, h, FMT_PARGB);
    // white background
    clear_sf(&mut d.layers[0], 0xFFFF_FFFF);
    d.nlayers = 1;
    d.cur_layer = 0;
    d.layer_vis[0] = 1;
    d.layer_opac[0] = 255;
    d.modified = false;
    d.bg_opaque = 0;
    d.path.clear();
    doc_mark_all(d);
    d.modified = false;
    0
}

/// doc_set_size(w, h) -- adopt a new canvas size (layers already have it); reallocates the
/// composite when needed
pub(crate) fn doc_set_size(p: &mut Kritzel, w: i32, h: i32) {
    tool_abort(p); // stroke buffers were sized for the old size
    let d = &mut p.doc;
    d.w = w;
    d.h = h;
    {
        let mut c = d.comp.borrow_mut();
        if c.w != w || c.h != h {
            c.resize(w, h);
        }
    }
    d.dirty_set = false; // old rectangles may exceed the new size
    doc_mark_all(d);
}

pub(crate) fn doc_w(d: &Doc) -> i32 {
    d.w
}

pub(crate) fn doc_h(d: &Doc) -> i32 {
    d.h
}

pub(crate) fn doc_version(d: &Doc) -> u32 {
    d.version
}

pub(crate) fn doc_modified(d: &Doc) -> bool {
    d.modified
}

pub(crate) fn doc_set_modified(d: &mut Doc, v: bool) {
    d.modified = v;
}

/// doc_layer(i) -> layer surface
pub(crate) fn doc_layer(d: &mut Doc, i: i32) -> &mut Surface {
    &mut d.layers[i as usize]
}

/// doc_cur() -> current layer surface
pub(crate) fn doc_cur(d: &mut Doc) -> &mut Surface {
    let i = d.cur_layer;
    doc_layer(d, i)
}

/// doc_mark_all()
pub(crate) fn doc_mark_all(d: &mut Doc) {
    let (w, h) = (d.w, d.h);
    doc_mark(d, 0, 0, w, h);
}

/// doc_mark(x0, y0, x1, y1) -- union into the dirty rectangle (clamped)
pub(crate) fn doc_mark(d: &mut Doc, x0: i32, y0: i32, x1: i32, y1: i32) {
    let x0 = x0.max(0);
    let y0 = y0.max(0);
    let x1 = x1.min(d.w);
    let y1 = y1.min(d.h);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    d.version = d.version.wrapping_add(1);
    d.modified = true;
    if d.dmg_set {
        let r = &mut d.dmg;
        r[0] = r[0].min(x0);
        r[1] = r[1].min(y0);
        r[2] = r[2].max(x1);
        r[3] = r[3].max(y1);
    } else {
        d.dmg_set = true;
        d.dmg = [x0, y0, x1, y1];
    }
    if !d.dirty_set {
        d.dirty_set = true;
        d.dirty = [x0, y0, x1, y1];
        return;
    }
    let r = &mut d.dirty;
    r[0] = r[0].min(x0);
    r[1] = r[1].min(y0);
    r[2] = r[2].max(x1);
    r[3] = r[3].max(y1);
}

/// doc_damage_take() -- the union of the pixels marked since the last call (None: nothing)
pub(crate) fn doc_damage_take(d: &mut Doc) -> Option<[i32; 4]> {
    if !d.dmg_set {
        return None;
    }
    d.dmg_set = false;
    Some(d.dmg)
}

/// doc_composite() -- recomposites the dirty rectangle into d.comp
pub(crate) fn doc_composite(d: &mut Doc) {
    let mut comp = d.comp.borrow_mut();
    composite_into(&mut comp, &d.layers, d.nlayers, &d.layer_vis, &d.layer_opac, &mut d.dirty, &mut d.dirty_set, d.w, d.h);
}

/// doc_composite's work on the parts of a document (the canvas draw object runs it at render
/// time on the layers lent to it: view.rs)
#[allow(clippy::too_many_arguments)]
pub(crate) fn composite_into(
    comp: &mut Surface,
    layers: &[Surface; MAX_LAYERS],
    nlayers: i32,
    vis: &[u8; MAX_LAYERS],
    opac: &[u8; MAX_LAYERS],
    dirty: &mut [i32; 4],
    dirty_set: &mut bool,
    w: i32,
    h: i32,
) {
    if !*dirty_set {
        return;
    }
    *dirty_set = false;
    // never trust the rectangle beyond the current size
    dirty[2] = dirty[2].min(w);
    dirty[3] = dirty[3].min(h);
    let [x0, y0, x1, y1] = *dirty;
    if x1 <= x0 {
        return;
    }
    let cs = comp.stride as usize;
    let (a, b) = (x0 as usize * 4, x1 as usize * 4);
    for y in y0.max(0)..y1 {
        let o = y as usize * cs;
        // clear the composite span
        let dst = &mut comp.px[o + a..o + b];
        blend::fill_solid(dst, 0);
        // layers bottom to top
        for i in 0..nlayers as usize {
            if vis[i] == 0 {
                continue;
            }
            let l = &layers[i];
            let lo = y as usize * l.stride as usize;
            blend::blit_over(dst, &l.px[lo + a..lo + b], opac[i] as u32);
        }
    }
}

/// doc_pick(x, y) -> straight ARGB at x, y of the composite (0 outside)
pub(crate) fn doc_pick(d: &mut Doc, x: i32, y: i32) -> u32 {
    if x as u32 >= d.w as u32 || y as u32 >= d.h as u32 {
        return 0;
    }
    doc_composite(d);
    let c = d.comp.borrow();
    let o = y as usize * c.stride as usize + x as usize * 4;
    unpremul_px(u32::from_le_bytes([c.px[o], c.px[o + 1], c.px[o + 2], c.px[o + 3]]))
}

/// doc_opaque() -> true if the bottom layer is visible, opaque and fully opaque pixels are
/// guaranteed (the white background layer); used to skip the checkerboard
pub(crate) fn doc_opaque(d: &Doc) -> bool {
    d.layer_vis[0] != 0 && d.layer_opac[0] == 255 && d.bg_opaque == 0
}
