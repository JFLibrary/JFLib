// doc -- PaintCraft: the open documents (up to DOC_MAX images = tabs), the active one, names
// ("Unbenannt", "Unbenannt 2", ...), paths, change marking.  Records: pn_doc.inc.
//                                                                    [apps/PaintCraft/src/doc.s]
//
//   doc_init()                        module init (no document yet; fileio creates the first)
//   doc_count() -> n                  open documents
//   doc_cur() -> i                    index of the active document (-1 = none)
//   doc_at(i) / doc_active() -> DocId | None
//   doc_index(doc) -> i               tab index (-1 = not open)
//   doc_set_active(doc)               make doc the active tab
//   doc_size(i) -> (w, h)             ((0, 0) when there is no such document)
//   doc_name(i) -> &str               display name ("" when there is no such document)
//   doc_dirty(i) -> bool              unsaved changes
//   doc_version(i) -> u32             grows with every pixel / layer change (thumbnails)
//   doc_new(w, h, bg_argb, flags) -> DocId | None     DNF_EMPTY / DNF_NOACTIVATE
//   doc_close(doc)                    free the document (fileio asks about changes first)
//   doc_layer(doc, i) -> &Layer       (i clamped; None without layers)
//   doc_cur_layer(doc) -> &Layer
//   doc_mark_px(doc, i, x0, y0, x1, y1)   pixels of layer i (-1 = any) changed
//   doc_mark_layers(doc)              layer structure / properties changed
//   doc_set_modified(doc, on)
//   doc_resized(doc) -> 0 / ERR_NOMEM DOC_W / DOC_H changed (layers already resized)
//   doc_set_path(doc, path) -> 0 / -1 file path + display name (the file name)
//   doc_find_path(path) -> DocId | None   an open document with that file (case and separator
//                                     insensitive)
//   doc_move(from, to)                reorder the tabs (the active document stays active)
//   pn_strdup(s) -> String            (a copy; mem_alloc in the asm)
//
// A document is a `Doc` in `Docs::docs` (tab order); other modules keep a `DocId` (the asm's
// DOC* pointer): stable while the document is open, never reused.  `p.doc.d(id)` /
// `p.doc.dm(id)` give the record (they panic for a closed document: a use-after-free in the
// asm), `get` / `get_mut` return None instead.  The `*_d` variants work on a record without
// Pn (they return true where the asm calls ui_invalidate; the Pn wrappers do it).
//
// Doc must stay Send + Sync: compose, the effects and the tool layers read it from worker
// threads (plat::parallel_for).  Nothing inside may be an Rc / RefCell / Cell; shared
// immutable data is an Arc.  The sub-blocks belong to their modules: sel (DocSel, pn-hist),
// view (DocView), hist (DocHist, pn-hist), comp (compose::Comp).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::Surface;
use img_core::{ImgMeta, ImgOpt, IMG_MAXDIM};
use ui::*;

use crate::document::composite::{comp_init_d, comp_mark_all_d, comp_mark_d, Comp};
use crate::app::constants::*;
use crate::document::history::DocHist;
use crate::document::layers::Layer;
use crate::document::selection::DocSel;
use crate::workspace::view::DocView;
use crate::app::Pn;

/// a document's identity (the asm's DOC*): unique for the run of the program
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct DocId(pub(crate) u32);

/// DOC (pn_doc.inc)
pub(crate) struct Doc {
    /// identity (the asm's record address)
    pub(crate) id: DocId,
    /// DOC_W / DOC_H: size in pixels
    pub(crate) w: i32,
    pub(crate) h: i32,
    /// DOC_LAYERS (DOC_NLAYERS = len): index 0 = bottom
    pub(crate) layers: Vec<Layer>,
    /// DOC_CUR: active layer index
    pub(crate) cur: i32,
    /// DOC_FLAGS: DF_*
    pub(crate) flags: u32,
    /// DOC_DPU_UNIT: resolution unit 0 none, 1 inch, 2 centimetre; DOC_DPU_X / _Y dots per unit
    pub(crate) dpu_unit: u8,
    pub(crate) dpu_x: f64,
    pub(crate) dpu_y: f64,
    /// DOC_PATH: file path ('/' separators), None = never saved
    pub(crate) path: Option<String>,
    /// DOC_NAME: display name ("Unbenannt", "Foto.jpg")
    pub(crate) name: String,
    /// DOC_FMT: IMGF_* of the file (0 = none)
    pub(crate) fmt: u32,
    /// DOC_SAVEOPT: last save options of this document
    pub(crate) saveopt: ImgOpt<'static>,
    /// DOC_ICC: ICC profile bytes (empty = none)
    pub(crate) icc: Vec<u8>,
    /// DOC_CSPACE: colour space id (colour management module)
    pub(crate) cspace: u32,
    /// DOC_META: EXIF / XMP blobs of the file (no ICC: that is `icc`) kept for round trips
    pub(crate) meta: Option<Box<ImgMeta>>,
    /// DOC_VER_PX / _LAY / _SEL / _HIST: bumped on pixel / layer / selection / history changes
    pub(crate) ver_px: u32,
    pub(crate) ver_lay: u32,
    pub(crate) ver_sel: u32,
    pub(crate) ver_hist: u32,
    /// DOC_UID_NEXT: next layer uid
    pub(crate) uid_next: u64,
    /// DOC_SEL block (sel.rs)
    pub(crate) sel: DocSel,
    /// DOC_VIEW block (view.rs)
    pub(crate) view: DocView,
    /// DOC_HIST block (hist.rs)
    pub(crate) hist: DocHist,
    /// DOC_COMP block (compose.rs)
    pub(crate) comp: Comp,
    /// DOC_THUMB: tab thumbnail (FMT_PARGB, opaque); DOC_THUMB_VER: doc_version it shows;
    /// thumb_aux: its SF_AUX (bumped on every render: the renderer's image version)
    pub(crate) thumb: Surface,
    pub(crate) thumb_ver: u32,
    pub(crate) thumb_aux: u32,
    /// DOC_SERIAL: n of "Unbenannt n" (1 = "Unbenannt"), 0 = named from a file
    pub(crate) serial: u32,
}

/// the open documents (g_docs, g_ndocs, g_curdoc)
pub(crate) struct Docs {
    /// g_docs: tab order
    pub(crate) docs: Vec<Box<Doc>>,
    /// g_curdoc: index of the active document, -1 = none
    pub(crate) cur: i32,
    /// the next DocId
    pub(crate) next_id: u32,
}

impl Default for Docs {
    fn default() -> Docs {
        Docs { docs: Vec::new(), cur: -1, next_id: 1 }
    }
}

impl Docs {
    /// the open document `id`
    pub(crate) fn get(&self, id: DocId) -> Option<&Doc> {
        self.docs.iter().find(|d| d.id == id).map(|d| &**d)
    }
    pub(crate) fn get_mut(&mut self, id: DocId) -> Option<&mut Doc> {
        self.docs.iter_mut().find(|d| d.id == id).map(|d| &mut **d)
    }
    /// the open document `id` (panics when it was closed)
    pub(crate) fn d(&self, id: DocId) -> &Doc {
        self.get(id).expect("closed document")
    }
    pub(crate) fn dm(&mut self, id: DocId) -> &mut Doc {
        self.get_mut(id).expect("closed document")
    }
    /// the active document's record
    pub(crate) fn active(&self) -> Option<&Doc> {
        self.docs.get(self.cur as u32 as usize).map(|d| &**d)
    }
    pub(crate) fn active_mut(&mut self) -> Option<&mut Doc> {
        self.docs.get_mut(self.cur as u32 as usize).map(|d| &mut **d)
    }
}

#[allow(dead_code)]
fn _assert_doc_send_sync() {
    fn s<T: Send + Sync>() {}
    s::<Doc>();
}

/// doc_init() -- no document yet
pub(crate) fn doc_init(p: &mut Pn, _ui: &mut Ui) {
    p.doc.docs.clear();
    p.doc.cur = -1;
}

/// doc_count()
pub(crate) fn doc_count(d: &Docs) -> i32 {
    d.docs.len() as i32
}

/// doc_cur() -> active index or -1
pub(crate) fn doc_cur(d: &Docs) -> i32 {
    d.cur
}

/// doc_at(i) -> DOC | None
pub(crate) fn doc_at(d: &Docs, i: i32) -> Option<DocId> {
    d.docs.get(i as u32 as usize).map(|x| x.id)
}

/// doc_active() -> DOC | None
pub(crate) fn doc_active(d: &Docs) -> Option<DocId> {
    doc_at(d, d.cur)
}

/// doc_index(doc) -> index or -1
pub(crate) fn doc_index(d: &Docs, doc: DocId) -> i32 {
    d.docs.iter().position(|x| x.id == doc).map_or(-1, |i| i as i32)
}

/// doc_set_active(doc) -- (unknown documents are ignored)
pub(crate) fn doc_set_active(p: &mut Pn, doc: DocId) {
    let i = doc_index(&p.doc, doc);
    if i < 0 || i == p.doc.cur {
        return;
    }
    p.doc.cur = i;
    p.repass();
}

/// doc_size(i) -> (w, h)
pub(crate) fn doc_size(d: &Docs, i: i32) -> (i32, i32) {
    d.docs.get(i as u32 as usize).map_or((0, 0), |x| (x.w, x.h))
}

/// doc_name(i) -> name ("" when there is no document)
pub(crate) fn doc_name(d: &Docs, i: i32) -> &str {
    d.docs.get(i as u32 as usize).map_or("", |x| x.name.as_str())
}

/// doc_dirty(i) -> unsaved changes
pub(crate) fn doc_dirty(d: &Docs, i: i32) -> bool {
    d.docs.get(i as u32 as usize).is_some_and(|x| x.flags & DF_MODIFIED as u32 != 0)
}

/// doc_version(i) -> pixel version + layer version (only grows)
pub(crate) fn doc_version(d: &Docs, i: i32) -> u32 {
    d.docs.get(i as u32 as usize).map_or(0, |x| x.ver_px.wrapping_add(x.ver_lay))
}

/// pn_strdup(s) -> copy
pub(crate) fn pn_strdup(s: &str) -> String {
    String::from(s)
}

// free_serial() -> the smallest n >= 1 no open "Unbenannt n" document uses
fn free_serial(d: &Docs) -> u32 {
    let mut n = 1;
    while d.docs.iter().any(|x| x.serial == n) {
        n += 1;
    }
    n
}

// untitled_name(serial) -> "Unbenannt" / "Unbenannt n"
fn untitled_name(serial: u32) -> String {
    if serial <= 1 { String::from("Unbenannt") } else { alloc::format!("Unbenannt {}", serial as i32) }
}

/// a new document record (doc_new without the list): no layers
fn doc_record(id: DocId, w: i32, h: i32, serial: u32) -> Doc {
    Doc {
        id,
        w,
        h,
        layers: Vec::with_capacity(8),
        cur: 0,
        flags: DF_UNTITLED as u32,
        dpu_unit: 1,
        dpu_x: 96.0,
        dpu_y: 96.0,
        path: None,
        name: untitled_name(serial),
        fmt: 0,
        saveopt: ImgOpt::default(),
        icc: Vec::new(),
        cspace: 0,
        meta: None,
        ver_px: 0,
        ver_lay: 0,
        ver_sel: 0,
        ver_hist: 0,
        uid_next: 1,
        sel: DocSel::default(),
        view: DocView::default(),
        hist: DocHist::default(),
        comp: Comp::default(),
        thumb: Surface::default(),
        thumb_ver: 0,
        thumb_aux: 0,
        serial,
    }
}

/// doc_new(w, h, bg_argb, flags) -> DOC or None (bad size, too many documents, no memory)
pub(crate) fn doc_new(p: &mut Pn, w: i32, h: i32, bg_argb: u32, flags: u32) -> Option<DocId> {
    if (w as u32).wrapping_sub(1) >= IMG_MAXDIM || (h as u32).wrapping_sub(1) >= IMG_MAXDIM {
        return None;
    }
    if p.doc.docs.len() >= DOC_MAX as usize {
        return None;
    }
    let id = DocId(p.doc.next_id);
    p.doc.next_id = p.doc.next_id.wrapping_add(1);
    let mut d = Box::new(doc_record(id, w, h, free_serial(&p.doc)));
    if comp_init_d(&mut d) != 0 {
        return None;
    }
    if flags & DNF_EMPTY as u32 == 0 {
        let i = crate::document::layers::lay_new_d(&mut d, 0, Some("Hintergrund"))?;
        let l = &mut d.layers[i as usize];
        l.flags |= LYF_BG as u32;
        if bg_argb != 0 {
            // fill with the background colour (straight ARGB)
            let row = w as usize * 4;
            let c = bg_argb.to_le_bytes();
            let stride = l.sf.stride as usize;
            for r in l.sf.px.chunks_mut(stride).take(h as usize) {
                for px in r[..row].chunks_exact_mut(4) {
                    px.copy_from_slice(&c);
                }
            }
        }
    }
    // the new document is not modified (a fresh image)
    d.flags &= !(DF_MODIFIED as u32);
    p.doc.docs.push(d);
    if flags & DNF_NOACTIVATE as u32 == 0 {
        p.doc.cur = p.doc.docs.len() as i32 - 1;
    }
    p.invalidate(); // comp_init / lay_new (comp_mark_all)
    p.repass();
    Some(id)
}

/// doc_close(doc) -- remove from the list and free; the tab to the right (else left) becomes
/// active when doc was the active one
pub(crate) fn doc_close(p: &mut Pn, doc: DocId) {
    let i = doc_index(&p.doc, doc);
    if i < 0 {
        return;
    }
    // (the history goes before the record leaves the list: hist_free finds it by id)
    crate::document::history::hist_free(p, doc);
    let docs = &mut p.doc;
    let d = docs.docs.remove(i as usize);
    let n = docs.docs.len() as i32;
    let c = docs.cur;
    if c > i {
        docs.cur = c - 1;
    } else if c == i && c >= n {
        // the active one was closed: keep the index (the tab to its right), else the last
        docs.cur = n - 1;
    }
    drop(d);
    p.repass();
}

/// doc_layer(doc, i) -> LAYER (i clamped to 0..n-1; None without layers)
pub(crate) fn doc_layer(d: &Doc, i: i32) -> Option<&Layer> {
    let n = d.layers.len() as i32;
    if n <= 0 {
        return None;
    }
    d.layers.get(i.clamp(0, n - 1) as usize)
}

/// doc_layer, mutable
pub(crate) fn doc_layer_mut(d: &mut Doc, i: i32) -> Option<&mut Layer> {
    let n = d.layers.len() as i32;
    if n <= 0 {
        return None;
    }
    d.layers.get_mut(i.clamp(0, n - 1) as usize)
}

/// doc_cur_layer(doc) -> LAYER
pub(crate) fn doc_cur_layer(d: &Doc) -> Option<&Layer> {
    doc_layer(d, d.cur)
}

/// doc_cur_layer, mutable
pub(crate) fn doc_cur_layer_mut(d: &mut Doc) -> Option<&mut Layer> {
    let c = d.cur;
    doc_layer_mut(d, c)
}

/// doc_mark_px(doc, i, x0, y0, x1, y1) -- x1 / y1 exclusive
pub(crate) fn doc_mark_px(p: &mut Pn, doc: DocId, i: i32, x0: i32, y0: i32, x1: i32, y1: i32) {
    if let Some(d) = p.doc.get_mut(doc) {
        if doc_mark_px_d(d, i, x0, y0, x1, y1) {
            p.invalidate();
        }
    }
}

/// doc_mark_px of a document record -> true when the UI must be invalidated
pub(crate) fn doc_mark_px_d(d: &mut Doc, i: i32, x0: i32, y0: i32, x1: i32, y1: i32) -> bool {
    if let Some(l) = d.layers.get_mut(i as u32 as usize) {
        l.ver = l.ver.wrapping_add(1);
    }
    d.ver_px = d.ver_px.wrapping_add(1);
    d.flags |= DF_MODIFIED as u32;
    comp_mark_d(d, x0, y0, x1, y1)
}

/// doc_mark_layers(doc) -- the layer list or a layer property changed
pub(crate) fn doc_mark_layers(p: &mut Pn, doc: DocId) {
    if let Some(d) = p.doc.get_mut(doc) {
        doc_mark_layers_d(d);
        p.invalidate();
    }
}

/// doc_mark_layers of a document record (the UI must be invalidated)
pub(crate) fn doc_mark_layers_d(d: &mut Doc) {
    d.ver_lay = d.ver_lay.wrapping_add(1);
    d.flags |= DF_MODIFIED as u32;
    comp_mark_all_d(d);
}

/// doc_set_modified(doc, on)
pub(crate) fn doc_set_modified(p: &mut Pn, doc: DocId, on: bool) {
    if let Some(d) = p.doc.get_mut(doc) {
        d.flags &= !(DF_MODIFIED as u32);
        if on {
            d.flags |= DF_MODIFIED as u32;
        }
    }
    p.repass();
}

/// doc_resized(doc) -> 0 / ERR_NOMEM -- the image size changed
pub(crate) fn doc_resized(p: &mut Pn, doc: DocId) -> i32 {
    let Some(d) = p.doc.get_mut(doc) else { return 0 };
    d.ver_lay = d.ver_lay.wrapping_add(1);
    d.ver_px = d.ver_px.wrapping_add(1);
    d.flags |= DF_MODIFIED as u32;
    let r = comp_init_d(d);
    p.invalidate();
    r
}

/// doc_set_path(doc, path) -> 0 / -1 -- DOC_PATH = copy of path, DOC_NAME = its file name
pub(crate) fn doc_set_path(p: &mut Pn, doc: DocId, path: &str) -> i32 {
    let Some(d) = p.doc.get_mut(doc) else { return -1 };
    // the file name: after the last '/' or '\'
    let name = match path.rfind(['/', '\\']) {
        Some(i) => &path[i + 1..],
        None => path,
    };
    d.name = String::from(name);
    d.path = Some(String::from(path));
    d.serial = 0;
    d.flags &= !(DF_UNTITLED as u32);
    p.repass();
    0
}

// path_eq(a, b) -> exact on Linux/web; Windows ignores ASCII case and '/' vs '\'
fn path_eq(a: &str, b: &str) -> bool {
    if !cfg!(windows) {
        return a == b;
    }
    let norm = |c: u8| -> u8 { if c == b'\\' { b'/' } else { c.to_ascii_lowercase() } };
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(&x, &y)| norm(x) == norm(y))
}

/// doc_find_path(path) -> the open document with this file or None
pub(crate) fn doc_find_path(d: &Docs, path: &str) -> Option<DocId> {
    d.docs.iter().find(|x| x.path.as_deref().is_some_and(|q| path_eq(q, path))).map(|x| x.id)
}

/// doc_move(from, to) -- move tab `from` to position `to` (both clamped)
pub(crate) fn doc_move(p: &mut Pn, from: i32, to: i32) {
    let n = p.doc.docs.len() as i32;
    if from as u32 >= n as u32 {
        return;
    }
    let to = to.max(0).min(n - 1);
    if from == to {
        return;
    }
    let act = doc_active(&p.doc);
    let d = p.doc.docs.remove(from as usize);
    p.doc.docs.insert(to as usize, d);
    p.doc.cur = match act {
        Some(a) => doc_index(&p.doc, a),
        None => -1,
    };
    p.repass();
}
