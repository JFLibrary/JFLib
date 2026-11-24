// view -- a page of a diagram as a render2d scene plus what the scene cannot hold
//
//   PageView::new(&doc, page, &mut fonts, &opts)   the display list (diagram_core::render,
//                                    texts measured with the fonts drawn), the scene made of
//                                    it, pictures decoded
//   view.update(&doc, &mut fonts, &shapes) -> Option<[f64; 4]>   (JD7) the named shapes (their
//                                    top-level shapes; JD21: and the connectors whose line jumps
//                                    they change, render::affected) made anew in the list and
//                                    the scene; the
//                                    box of what changed, old and new place (paper inches;
//                                    a zero box: nothing drawn either way); None: shapes came,
//                                    went or changed their order -- the page was built anew
//   view.draw(dc, sf, r, &cam)       the page on the CPU into the rectangle r of a surface
//                                    (render2d's CPU path; gradients and pictures in between)
//   view.render(w, h) -> Surface     the page fitted into w x h pixels on its colour (XRGB)
//   view.fit(size, margin_px) -> Camera   the camera showing the whole page
//   view.drawing() / substitutions() / refused() / scene()
//
// The scene: render2d layers in painter's order.  A run of items render2d can draw (solid and
// hatched fills, lines with dashes, caps and joins, texts) lies on layers of at most
// LAYER_ITEMS items each; a gradient fill or a picture ends the run and is drawn by gfx between
// two layers (direct.rs).  On the CPU the layers between two such items are drawn in one go
// (render2d keeps the painter's order across layers) while the others are hidden, so the order
// holds; without such items every layer stays shown.  On a GPU (Scene::attach, the app's
// choice) the same scene shows everything but those items.
//
// An update (JD7): render::rebuild replaces the items of the named top-level shapes, then the
// scene follows, the cheapest way that keeps the order: (1) each item keeps its kind and its
// number of entities -> every entity updated in place (render2d keeps an entity's place);
// (2) the shape's items lie on one layer and none is drawn by gfx -> its entities updated in
// order, the surplus removed, more added at the layer's end when the shape is the layer's last;
// (3) else the layers holding the shape's items are made anew from their items, with orders
// between the neighbours' (Layer::order, GAP apart, renumbered when the room runs out).  The
// fonts, the measured widths, the styles and the scene's glyph outlines stay across updates and
// across a page built anew.  Measured (diagramiotest group editview): 1000 shapes, one moved:
// update well under a millisecond, the drawing of the page dominates.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::ops::Range;

use diagram_core::geometry::{Seg, Sub};
use diagram_core::render::{self, Drawing, ImageSource, Item, ItemKind, Options, Paint, Stroke};
use diagram_core::{Document, PageRef, ShapeRef};
use gfx::raster::Raster;
use gfx::{Dc, Surface, FMT_XRGB};
use render2d::{Camera, Cap, EntityId, HatchLine, Join, Layer, LayerId, Scene, Shape, Style, StyleId, Text, TextStyle, TextStyleId, Unit};

use crate::direct::{self, Target};
use crate::fonts::FontSet;
use crate::image::{self, Decoded};
use crate::measure::{FontCache, Measurer};

/// how far a synthetic italic leans (radians)
pub const SYNTHETIC_ITALIC: f64 = 0.2;

/// the most items a layer gets when the scene is made (an update remakes at most the layers
/// its shape lies on)
const LAYER_ITEMS: usize = 64;
/// the room between the orders of two layers
const GAP: i64 = 1 << 12;

/// a piece of the painter's order: items on a render2d layer (its id and order), or one item
/// gfx draws
struct Part {
    layer: Option<(LayerId, i32)>,
    start: usize,
    len: usize,
}

/// what an item of the drawing became
#[derive(Default)]
struct Slot {
    /// drawn by gfx (a gradient fill, a picture)
    direct: bool,
    ents: Vec<(EntityId, StyleId)>,
    /// render2d refused one of its entities
    refused: bool,
    /// a picture: its source and what it decoded to
    pic: Option<(ImageSource, Decoded)>,
}

/// an entity to be: what render2d gets
enum Spec {
    /// rings: their points and whether they are straight
    Fill(Vec<(Vec<[f64; 2]>, bool)>),
    /// points, straight, closed
    Line(Vec<[f64; 2]>, bool, bool),
    Text { text: String, style: TextStyleId, at: [f64; 2], height: f64, rotation: f64, width: f64, oblique: f64 },
}

/// the scene and what makes entities for it (apart from the drawing, so that both can be
/// borrowed at once)
struct Side {
    scene: Scene,
    cache: FontCache,
    styles: BTreeMap<StyleKey, StyleId>,
    text_styles: BTreeMap<u32, TextStyleId>,
    ras: Raster,
}

/// a page ready to draw
pub struct PageView {
    drawing: Drawing,
    opts: Options,
    side: Side,
    parts: Vec<Part>,
    /// per item of the drawing
    slots: Vec<Slot>,
    /// items render2d refused (invalid geometry)
    refused: u32,
    /// layers hidden but while they are drawn (there are items gfx draws)
    hidden: bool,
}

impl PageView {
    /// the page as a scene without a device
    pub fn new(doc: &Document, page: PageRef, fonts: &mut FontSet, opts: &Options) -> PageView {
        let side = Side { scene: Scene::new_cpu(), cache: FontCache::default(), styles: BTreeMap::new(), text_styles: BTreeMap::new(), ras: Raster::default() };
        let mut v = PageView { drawing: Drawing::default(), opts: *opts, side, parts: Vec::new(), slots: Vec::new(), refused: 0, hidden: false };
        v.build(doc, page, fonts);
        v
    }

    pub fn drawing(&self) -> &Drawing {
        &self.drawing
    }

    pub fn scene(&mut self) -> &mut Scene {
        &mut self.side.scene
    }

    /// the font families drawn with a substitute (TX-09: shown to the user)
    pub fn substitutions(&self) -> &[String] {
        &self.side.cache.substituted
    }

    /// items render2d did not take (geometry it calls invalid)
    pub fn refused(&self) -> u32 {
        self.refused
    }

    /// the list and the scene made anew (the scene's fonts and glyphs kept)
    fn build(&mut self, doc: &Document, page: PageRef, fonts: &FontSet) {
        for p in core::mem::take(&mut self.parts) {
            if let Some((l, _)) = p.layer {
                let _ = self.side.scene.remove_layer(l);
            }
        }
        self.slots.clear();
        self.refused = 0;
        let side = &mut self.side;
        let mut m = Measurer { fonts: side.scene.fonts(), set: fonts, cache: &mut side.cache };
        self.drawing = render::build(doc, page, &mut m, &self.opts);
        self.hidden = self.drawing.items.iter().any(is_direct);
        let n = self.drawing.items.len();
        let (parts, slots) = self.make(doc, fonts, 0..n, None, None);
        self.refused = slots.iter().filter(|s| s.refused).count() as u32;
        self.parts = parts;
        self.slots = slots;
    }

    /// the named shapes made anew (their top-level shapes; subshapes name their group, shapes
    /// of other pages are passed over): the box of what changed (old and new, paper inches,
    /// line widths and shadows included; a zero box: they draw nothing either way); None:
    /// shapes came, went or changed their order, or the page's size changed -- the page was
    /// built anew.  Changes the shapes do not show (the page's layers, styles, masters,
    /// themes) need PageView::new
    pub fn update(&mut self, doc: &Document, fonts: &mut FontSet, shapes: &[ShapeRef]) -> Option<[f64; 4]> {
        let page = self.drawing.page?;
        // the top-level shapes named and their items now; (JD21) with the connectors whose line
        // jumps they change (render::rebuild makes those anew too)
        let mut olds: Vec<(ShapeRef, Range<usize>)> = Vec::new();
        for top in render::affected(doc, &self.drawing, &self.opts, shapes) {
            if olds.iter().any(|o| o.0 == top) {
                continue;
            }
            if let Some(r) = self.drawing.range_of(top) {
                olds.push((top, r));
            }
        }
        let mut area: Option<[f64; 4]> = None;
        for (_, r) in &olds {
            for it in &self.drawing.items[r.clone()] {
                area = Some(area.map_or(it.bbox, |a| union(a, it.bbox)));
            }
        }
        let ok = {
            let side = &mut self.side;
            let mut m = Measurer { fonts: side.scene.fonts(), set: fonts, cache: &mut side.cache };
            render::rebuild(doc, &mut self.drawing, &mut m, &self.opts, shapes)
        };
        if !ok {
            self.build(doc, page, fonts);
            return None;
        }
        // (old start, old count, new count), ascending
        olds.sort_by_key(|o| o.1.start);
        let mut changes: Vec<(usize, usize, usize)> = Vec::with_capacity(olds.len());
        for (top, old) in olds {
            let Some(new) = self.drawing.range_of(top) else { continue };
            for it in &self.drawing.items[new.clone()] {
                area = Some(area.map_or(it.bbox, |a| union(a, it.bbox)));
            }
            changes.push((old.start, old.len(), new.len()));
        }
        // each change in the scene's numbering of the moment: the changes before it applied
        let (mut j, mut shift) = (0usize, 0isize);
        while j < changes.len() {
            let done = self.replace(doc, fonts, &changes[j..], shift).max(1);
            for c in &changes[j..(j + done).min(changes.len())] {
                shift += c.2 as isize - c.1 as isize;
            }
            j += done;
        }
        let hidden = self.drawing.items.iter().any(is_direct);
        if hidden != self.hidden {
            self.hidden = hidden;
            for p in &self.parts {
                if let Some((l, _)) = p.layer {
                    set_visible(&mut self.side.scene, l, !hidden);
                }
            }
        }
        Some(area.filter(|a| a.iter().all(|v| v.is_finite())).unwrap_or([0.0; 4]))
    }

    /// the first of `cs` (old start, old count, new count; the rest follow it) into the scene:
    /// its old items lie at old start + shift in the scene's numbering now, the drawing's items
    /// there are its new ones -> how many of cs were done (a remade layer takes the changes on
    /// it along)
    fn replace(&mut self, doc: &Document, fonts: &FontSet, cs: &[(usize, usize, usize)], shift: isize) -> usize {
        let (a, n_old, n_new) = ((cs[0].0 as isize + shift) as usize, cs[0].1, cs[0].2);
        let mut specs: Vec<Option<Vec<(StyleId, Spec)>>> = Vec::with_capacity(n_new);
        for i in a..a + n_new {
            let it = &self.drawing.items[i];
            specs.push(if is_direct(it) { None } else { Some(self.side.specs(it, fonts)) });
        }
        // (1) the same kinds and numbers of entities: everything in place
        let same = n_old == n_new
            && (0..n_new).all(|k| {
                let s = &self.slots[a + k];
                match &specs[k] {
                    None => s.direct,
                    Some(v) => !s.direct && v.len() == s.ents.len(),
                }
            });
        if same {
            for (k, spec) in specs.into_iter().enumerate() {
                let i = a + k;
                match spec {
                    None => {
                        let src = match &self.drawing.items[i].kind {
                            ItemKind::Image(img) => Some(&img.source),
                            _ => None,
                        };
                        let keep = matches!((&self.slots[i].pic, src), (Some((old, _)), Some(new)) if old == new);
                        if !keep {
                            self.slots[i].pic = picture(doc, &self.drawing.items[i]);
                        }
                    }
                    Some(v) => {
                        let old = core::mem::take(&mut self.slots[i].ents);
                        let mut slot = Slot::default();
                        for ((id, osid), (sid, spec)) in old.into_iter().zip(v) {
                            self.side.renew(id, osid, sid, &spec, &mut slot);
                        }
                        self.set_slot(i, slot);
                    }
                }
            }
            return 1;
        }
        // (2) on one layer, nothing for gfx: entities in order, the surplus away, more at the end
        if specs.iter().all(|s| s.is_some()) {
            if let Some((pi, layer)) = self.layer_part(a, n_old) {
                let last = a + n_old == self.parts[pi].start + self.parts[pi].len;
                let old_n: usize = self.slots[a..a + n_old].iter().map(|s| s.ents.len()).sum();
                let new_n: usize = specs.iter().map(|s| s.as_ref().map_or(0, |v| v.len())).sum();
                if new_n <= old_n || last {
                    let mut olds = Vec::with_capacity(old_n);
                    for s in &mut self.slots[a..a + n_old] {
                        olds.append(&mut s.ents);
                    }
                    let mut olds = olds.into_iter();
                    let mut slots = Vec::with_capacity(n_new);
                    for v in specs.into_iter().flatten() {
                        let mut slot = Slot::default();
                        for (sid, spec) in v {
                            match olds.next() {
                                Some((id, osid)) => self.side.renew(id, osid, sid, &spec, &mut slot),
                                None => self.side.insert(layer, sid, &spec, &mut slot),
                            }
                        }
                        slots.push(slot);
                    }
                    for (id, _) in olds {
                        let _ = self.side.scene.remove(id);
                    }
                    self.splice_slots(a, n_old, slots);
                    self.parts[pi].len = self.parts[pi].len + n_new - n_old;
                    for p in &mut self.parts[pi + 1..] {
                        p.start = p.start + n_new - n_old;
                    }
                    return 1;
                }
            }
        }
        // (3) the parts the items lie on made anew
        self.remake(doc, fonts, cs, shift)
    }

    /// the layer part holding items a .. a + n (n = 0: the part the item before a ends), and
    /// its layer
    fn layer_part(&self, a: usize, n: usize) -> Option<(usize, LayerId)> {
        let k = if n == 0 { self.parts.iter().position(|p| p.start + p.len == a && p.len > 0)? } else { self.parts.iter().position(|p| p.start <= a && a + n <= p.start + p.len)? };
        self.parts[k].layer.map(|l| (k, l.0))
    }

    /// the parts over items lo .. hi (an empty range inside a part: that part)
    fn parts_over(&self, lo: usize, hi: usize) -> (usize, usize) {
        let n = self.parts.len();
        if hi > lo {
            let p0 = self.parts.iter().position(|p| lo < p.start + p.len).unwrap_or(n);
            let p1 = self.parts.iter().position(|p| p.start >= hi).unwrap_or(n);
            return (p0, p1.max(p0));
        }
        match self.parts.iter().position(|p| p.start < lo && lo < p.start + p.len) {
            Some(k) => (k, k + 1),
            None => {
                let k = self.parts.iter().position(|p| p.start >= lo).unwrap_or(n);
                (k, k)
            }
        }
    }

    /// the parts under the first of `cs` made anew from the drawing's items, with the later
    /// changes that lie on them -> how many changes that took
    fn remake(&mut self, doc: &Document, fonts: &FontSet, cs: &[(usize, usize, usize)], shift: isize) -> usize {
        let cur = |c: &(usize, usize, usize)| (c.0 as isize + shift) as usize;
        // the window: whole parts, and every change that starts in it
        let (mut w0, mut w1) = (cur(&cs[0]), cur(&cs[0]) + cs[0].1);
        let mut m = 1;
        let (mut p0, mut p1);
        loop {
            (p0, p1) = self.parts_over(w0, w1);
            if p0 < p1 {
                w0 = w0.min(self.parts[p0].start);
                w1 = w1.max(self.parts[p1 - 1].start + self.parts[p1 - 1].len);
            }
            let before = m;
            while m < cs.len() && cur(&cs[m]) < w1 {
                w1 = w1.max(cur(&cs[m]) + cs[m].1);
                m += 1;
            }
            if m == before {
                break;
            }
        }
        let grow: isize = cs[..m].iter().map(|c| c.2 as isize - c.1 as isize).sum();
        for p in &self.parts[p0..p1] {
            if let Some((l, _)) = p.layer {
                let _ = self.side.scene.remove_layer(l);
            }
        }
        let lo = self.parts[..p0].iter().rev().find_map(|p| p.layer.map(|l| l.1 as i64));
        let hi = self.parts[p1..].iter().find_map(|p| p.layer.map(|l| l.1 as i64));
        let w1_new = (w1 as isize + grow) as usize;
        let range = w0..w1_new;
        let layers = runs_of(&self.drawing.items, range.clone()).iter().filter(|r| !r.0).count();
        let (lo, hi) = if between(lo, hi, layers).is_some() {
            (lo, hi)
        } else {
            // no room: every other layer gets new orders, the room left where these go
            let mut o = 0i64;
            for k in 0..self.parts.len() {
                if k == p0 {
                    o += (layers as i64 + 1) * GAP;
                }
                if (p0..p1).contains(&k) {
                    continue;
                }
                if let Some((l, ord)) = &mut self.parts[k].layer {
                    *ord = o as i32;
                    if let Ok(mut x) = self.side.scene.layer(*l) {
                        x.order = o as i32;
                        let _ = self.side.scene.set_layer(*l, &x);
                    }
                    o += GAP;
                }
            }
            let lo = self.parts[..p0].iter().rev().find_map(|p| p.layer.map(|l| l.1 as i64));
            let hi = self.parts[p1..].iter().find_map(|p| p.layer.map(|l| l.1 as i64));
            (lo, hi)
        };
        let (parts, slots) = self.make(doc, fonts, range, lo, hi);
        // the old slots of the window go (their entities went with their layers)
        self.splice_slots(w0, w1 - w0, slots);
        let added = parts.len();
        self.parts.splice(p0..p1, parts);
        for p in &mut self.parts[p0 + added..] {
            p.start = (p.start as isize + grow) as usize;
        }
        m
    }

    /// parts and slots for the drawing's items in `range`, the layers' orders between lo and hi
    fn make(&mut self, doc: &Document, fonts: &FontSet, range: Range<usize>, lo: Option<i64>, hi: Option<i64>) -> (Vec<Part>, Vec<Slot>) {
        let runs = runs_of(&self.drawing.items, range.clone());
        let n_layers = runs.iter().filter(|r| !r.0).count();
        let orders = between(lo, hi, n_layers).unwrap_or_else(|| (0..n_layers).map(|k| (k as i64 * GAP) as i32).collect());
        let mut orders = orders.into_iter();
        let mut parts = Vec::with_capacity(runs.len());
        let mut slots = Vec::with_capacity(range.len());
        for (direct, start, len) in runs {
            if direct {
                let it = &self.drawing.items[start];
                slots.push(Slot { direct: true, ents: Vec::new(), refused: false, pic: picture(doc, it) });
                parts.push(Part { layer: None, start, len });
                continue;
            }
            let order = orders.next().unwrap_or(0);
            let mut l = Layer::new(order);
            l.visible = !self.hidden;
            let layer = self.side.scene.add_layer(&l).ok();
            for i in start..start + len {
                let it = &self.drawing.items[i];
                let mut slot = Slot::default();
                match layer {
                    Some(lid) => {
                        for (sid, spec) in self.side.specs(it, fonts) {
                            self.side.insert(lid, sid, &spec, &mut slot);
                        }
                    }
                    None => slot.refused = true,
                }
                slots.push(slot);
            }
            parts.push(Part { layer: layer.map(|l| (l, order)), start, len });
        }
        (parts, slots)
    }

    /// slot i replaced (the refused count follows)
    fn set_slot(&mut self, i: usize, slot: Slot) {
        self.refused = self.refused - self.slots[i].refused as u32 + slot.refused as u32;
        self.slots[i] = slot;
    }

    fn splice_slots(&mut self, a: usize, n_old: usize, slots: Vec<Slot>) {
        let added = slots.iter().filter(|s| s.refused).count() as u32;
        let gone: u32 = self.slots.splice(a..a + n_old, slots).filter(|s| s.refused).count() as u32;
        self.refused = self.refused - gone + added;
    }

    /// the camera that shows the whole page in a target of `size`, `margin` pixels around it
    pub fn fit(&self, size: (u32, u32), margin: f64) -> Camera {
        let (w, h) = (self.drawing.width.max(1e-6), self.drawing.height.max(1e-6));
        let sx = w / (size.0 as f64 - 2.0 * margin).max(1.0);
        let sy = h / (size.1 as f64 - 2.0 * margin).max(1.0);
        Camera::new([w * 0.5, h * 0.5], sx.max(sy))
    }

    /// the page on the CPU into the rectangle r (x, y, w, h relative to dc's origin) of sf,
    /// over what is there (the app paints the page's paper first).  Only the part of r that
    /// dc's clip keeps is drawn, with a camera on that part alone (render2d culls by its view):
    /// a changed box drawn again (update's area) costs what lies there
    pub fn draw(&mut self, dc: &mut Dc, sf: &mut Surface, r: [i32; 4], cam: &Camera) {
        if r[2] <= 0 || r[3] <= 0 {
            return;
        }
        let view = [r[0] + dc.ox, r[1] + dc.oy, r[0] + dc.ox + r[2], r[1] + dc.oy + r[3]];
        let c = dc.clip;
        let k = [view[0].max(c[0]), view[1].max(c[1]), view[2].min(c[2]), view[3].min(c[3])];
        if k[2] <= k[0] || k[3] <= k[1] {
            return;
        }
        let (r, cam) = if k == view {
            (r, *cam)
        } else {
            // the part's middle in pixels of the view, and the point of the page there
            let q = [((k[0] + k[2]) as f64 - 2.0 * view[0] as f64) * 0.5, ((k[1] + k[3]) as f64 - 2.0 * view[1] as f64) * 0.5];
            let mut part = *cam;
            part.center = cam.to_drawing(q, (r[2] as u32, r[3] as u32));
            ([k[0] - dc.ox, k[1] - dc.oy, k[2] - k[0], k[3] - k[1]], part)
        };
        let cam = &cam;
        if self.side.scene.set_camera(cam).is_err() {
            return;
        }
        let size = (r[2] as u32, r[3] as u32);
        let mut k = 0;
        while k < self.parts.len() {
            if self.parts[k].layer.is_some() {
                // the layers up to the next item gfx draws, in one go
                let mut j = k;
                while j < self.parts.len() && self.parts[j].layer.is_some() {
                    j += 1;
                }
                if self.hidden {
                    for p in &self.parts[k..j] {
                        if let Some((l, _)) = p.layer {
                            set_visible(&mut self.side.scene, l, true);
                        }
                    }
                }
                self.side.scene.draw_cpu(dc, sf, r);
                if self.hidden {
                    for p in &self.parts[k..j] {
                        if let Some((l, _)) = p.layer {
                            set_visible(&mut self.side.scene, l, false);
                        }
                    }
                }
                k = j;
                continue;
            }
            let i = self.parts[k].start;
            k += 1;
            dc.clip_push(r[0], r[1], r[2], r[3]);
            let at = [(r[0] + dc.ox) as f64, (r[1] + dc.oy) as f64];
            let mut t = Target { dc, sf: &mut *sf, at, size, cam: *cam };
            if t.dc.clip[2] > t.dc.clip[0] && t.dc.clip[3] > t.dc.clip[1] {
                match &self.drawing.items[i].kind {
                    ItemKind::Fill { path, paint: Paint::Gradient(g) } => direct::fill_gradient(&mut t, &mut self.side.ras, path, g),
                    ItemKind::Image(img) => match self.slots.get(i).and_then(|s| s.pic.as_ref()) {
                        Some((_, Decoded::Picture(p))) => direct::picture(&mut t, &mut self.side.ras, p, &img.xform, &img.clip),
                        _ => direct::placeholder(&mut t, &mut self.side.ras, &img.xform, &img.clip),
                    },
                    _ => {}
                }
            }
            dc.clip_pop();
        }
    }

    /// the page fitted into w x h pixels (a margin of 2 % around it) on the canvas grey, the
    /// page in its colour: an XRGB surface
    pub fn render(&mut self, w: u32, h: u32) -> Surface {
        let mut sf = Surface::new(w as i32, h as i32, FMT_XRGB);
        let mut dc = Dc::new(w as i32, h as i32);
        dc.begin(&sf);
        dc.clear(&mut sf, 0xFFEC_EEF1);
        let margin = (w.min(h) as f64 * 0.02).max(2.0);
        let cam = self.fit((w, h), margin);
        // the paper
        let (a, b) = (cam.to_screen([0.0, self.drawing.height], (w, h)), cam.to_screen([self.drawing.width, 0.0], (w, h)));
        let px = |v: f64| crate::floor(v + 0.5) as i32;
        let (x0, y0, x1, y1) = (px(a[0]), px(a[1]), px(b[0]), px(b[1]));
        dc.set_color(0xFF00_0000 | self.drawing.color);
        dc.fill(&mut sf, x0, y0, (x1 - x0).max(0), (y1 - y0).max(0));
        self.draw(&mut dc, &mut sf, [0, 0, w as i32, h as i32], &cam);
        dc.end();
        sf
    }
}

impl Side {
    /// the entities an item becomes (none for what gfx draws)
    fn specs(&mut self, it: &Item, fonts: &FontSet) -> Vec<(StyleId, Spec)> {
        let mut out = Vec::new();
        match &it.kind {
            ItemKind::Fill { path, paint } => {
                let key = match paint {
                    Paint::Solid(c) => StyleKey::fill(*c),
                    Paint::Hatch { color, lines } => StyleKey::hatch(*color, lines),
                    Paint::Gradient(_) => return out,
                };
                let Some(sid) = self.style_id(key) else { return out };
                let rings: Vec<(Vec<[f64; 2]>, bool)> = path.subs.iter().filter(|s| !s.segs.is_empty()).map(points).collect();
                if !rings.is_empty() {
                    out.push((sid, Spec::Fill(rings)));
                }
            }
            ItemKind::Stroke { path, stroke } => {
                let Some(sid) = self.style_id(StyleKey::stroke(stroke)) else { return out };
                for sub in path.subs.iter().filter(|s| !s.segs.is_empty()) {
                    let (p, straight) = points(sub);
                    out.push((sid, Spec::Line(p, straight, sub.closed)));
                }
            }
            ItemKind::Text(t) => {
                let ang = t.xform.angle();
                for run in &t.runs {
                    let r = match self.cache.known(&run.family, run.bold, run.italic) {
                        Some(r) => r,
                        None => {
                            let mut m = Measurer { fonts: self.scene.fonts(), set: fonts, cache: &mut self.cache };
                            m.resolve(&render::Face { family: &run.family, bold: run.bold, italic: run.italic })
                        }
                    };
                    let Some(r) = r else { continue };
                    let Some(fm) = self.scene.fonts().metrics(r.font) else { continue };
                    let Some(sid) = self.style_id(StyleKey::fill(run.color)) else { continue };
                    let style = match self.text_styles.get(&r.font.0) {
                        Some(s) => *s,
                        None => {
                            let Ok(s) = self.scene.add_text_style(&TextStyle::new(r.font)) else { continue };
                            self.text_styles.insert(r.font.0, s);
                            s
                        }
                    };
                    out.push((sid, Spec::Text { text: run.text.clone(), style, at: t.xform.apply(run.at), height: run.size * fm.cap_height, rotation: ang, width: run.scale_x * r.width, oblique: r.oblique }));
                }
            }
            ItemKind::Image(_) => {}
        }
        out
    }

    fn style_id(&mut self, key: StyleKey) -> Option<StyleId> {
        if let Some(id) = self.styles.get(&key) {
            return Some(*id);
        }
        let id = self.scene.add_style(&key.style()).ok()?;
        self.styles.insert(key, id);
        Some(id)
    }

    /// a new entity at the end of a layer
    fn insert(&mut self, layer: LayerId, sid: StyleId, spec: &Spec, slot: &mut Slot) {
        let scene = &mut self.scene;
        match with_shape(spec, &mut |s| scene.insert(layer, sid, s)) {
            Ok(id) => slot.ents.push((id, sid)),
            Err(_) => slot.refused = true,
        }
    }

    /// an entity with a new look and geometry in its place (refused: it goes)
    fn renew(&mut self, id: EntityId, old: StyleId, sid: StyleId, spec: &Spec, slot: &mut Slot) {
        if sid != old && self.scene.set_entity_style(id, sid).is_err() {
            let _ = self.scene.remove(id);
            slot.refused = true;
            return;
        }
        let scene = &mut self.scene;
        match with_shape(spec, &mut |s| scene.update(id, s)) {
            Ok(()) => slot.ents.push((id, sid)),
            Err(_) => {
                let _ = self.scene.remove(id);
                slot.refused = true;
            }
        }
    }
}

/// render2d's shape of a spec, handed to f
fn with_shape<R>(spec: &Spec, f: &mut dyn FnMut(&Shape) -> R) -> R {
    match spec {
        Spec::Fill(rings) => {
            let shapes: Vec<Shape> = rings.iter().map(|(p, straight)| if *straight { Shape::Polyline { points: p, closed: true } } else { Shape::Cubic { points: p, closed: true } }).collect();
            let rings: Vec<&[Shape]> = shapes.iter().map(core::slice::from_ref).collect();
            f(&Shape::Fill { rings: &rings })
        }
        Spec::Line(p, straight, closed) => {
            if *straight {
                f(&Shape::Polyline { points: p, closed: *closed })
            } else {
                f(&Shape::Cubic { points: p, closed: *closed })
            }
        }
        Spec::Text { text, style, at, height, rotation, width, oblique } => {
            let mut tx = Text::new(text, *style, *at);
            tx.height = Some(*height);
            tx.rotation = *rotation;
            tx.width = Some(*width);
            tx.oblique = Some(*oblique);
            f(&Shape::Text(tx))
        }
    }
}

/// whether gfx draws the item (render2d's scene has no gradients and pictures)
fn is_direct(it: &Item) -> bool {
    matches!(it.kind, ItemKind::Fill { paint: Paint::Gradient(_), .. } | ItemKind::Image(_))
}

/// a picture item's source and decoded pixels
fn picture(doc: &Document, it: &Item) -> Option<(ImageSource, Decoded)> {
    match &it.kind {
        ItemKind::Image(img) => {
            let pic = image::bytes(doc, &img.source).map_or(Decoded::Placeholder("?"), |b| image::decode(&b));
            Some((img.source.clone(), pic))
        }
        _ => None,
    }
}

/// the items of a range in parts: (drawn by gfx, start, count); gfx's items alone, the others
/// in runs of at most LAYER_ITEMS
fn runs_of(items: &[Item], range: Range<usize>) -> Vec<(bool, usize, usize)> {
    let mut out: Vec<(bool, usize, usize)> = Vec::new();
    for i in range {
        let d = is_direct(&items[i]);
        match out.last_mut() {
            Some(last) if !d && !last.0 && last.2 < LAYER_ITEMS => last.2 += 1,
            _ => out.push((d, i, 1)),
        }
    }
    out
}

/// n orders strictly between lo and hi (None: open), GAP apart where there is room; None:
/// no room left
fn between(lo: Option<i64>, hi: Option<i64>, n: usize) -> Option<Vec<i32>> {
    let n64 = n as i64;
    let v: Vec<i64> = match (lo, hi) {
        (None, None) => (0..n64).map(|k| k * GAP).collect(),
        (Some(a), None) => (1..=n64).map(|k| a + k * GAP).collect(),
        (None, Some(b)) => (0..n64).map(|k| b - (n64 - k) * GAP).collect(),
        (Some(a), Some(b)) => {
            let step = ((b - a) / (n64 + 1)).min(GAP);
            if n > 0 && step < 1 {
                return None;
            }
            (1..=n64).map(|k| a + k * step).collect()
        }
    };
    v.iter().all(|o| (i32::MIN as i64 / 2..=i32::MAX as i64 / 2).contains(o)).then(|| v.into_iter().map(|o| o as i32).collect())
}

fn union(a: [f64; 4], b: [f64; 4]) -> [f64; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

fn set_visible(s: &mut Scene, l: LayerId, v: bool) {
    if let Ok(mut x) = s.layer(l) {
        if x.visible != v {
            x.visible = v;
            let _ = s.set_layer(l, &x);
        }
    }
}

/// what a render2d style is made of (deduplicated; numbers by their bits, so that the key orders)
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct StyleKey {
    color: u32,
    width: u64,
    cap: u8,
    join: u8,
    dash: Vec<u64>,
    hatch: Vec<(u64, u64)>,
}

impl StyleKey {
    fn fill(color: u32) -> StyleKey {
        StyleKey { color, width: 0, cap: 0, join: 0, dash: Vec::new(), hatch: Vec::new() }
    }

    fn hatch(color: u32, lines: &[(f64, f64)]) -> StyleKey {
        StyleKey { hatch: lines.iter().take(render2d::HATCH_MAX).map(|&(a, s)| (a.to_bits(), s.to_bits())).collect(), ..StyleKey::fill(color) }
    }

    fn stroke(s: &Stroke) -> StyleKey {
        let cap = match s.cap {
            render::Cap::Round => 0,
            render::Cap::Butt => 1,
            render::Cap::Square => 2,
        };
        let join = match s.join {
            render::Join::Round => 0,
            render::Join::Miter => 1,
            render::Join::Bevel => 2,
        };
        let dash = s.dash.iter().take(render2d::DASH_MAX).map(|v| v.to_bits()).collect();
        StyleKey { color: s.color, width: s.width.to_bits(), cap, join, dash, hatch: Vec::new() }
    }

    fn style(&self) -> Style {
        let mut s = Style::new(self.color);
        let width = f64::from_bits(self.width);
        if width > 0.0 {
            s.width = width as f32;
            s.unit = Unit::Drawing;
        }
        s.cap = match self.cap {
            0 => Cap::Round,
            1 => Cap::Butt,
            _ => Cap::Square,
        };
        s.join = match self.join {
            0 => Join::Round,
            1 => Join::Miter,
            _ => Join::Bevel,
        };
        let dash: Vec<f64> = self.dash.iter().map(|v| f64::from_bits(*v)).collect();
        if !dash.is_empty() && dash.iter().sum::<f64>() > 0.0 {
            let pat: Vec<f32> = dash.iter().map(|v| *v as f32).collect();
            s = s.dashed(&pat, Unit::Drawing, 0.0);
        }
        if !self.hatch.is_empty() {
            let fams: Vec<HatchLine> = self.hatch.iter().map(|&(a, sp)| HatchLine::new(f64::from_bits(a), f64::from_bits(sp))).collect();
            s = s.hatched(&fams, 1.0);
        }
        s
    }
}

/// a sub-path as render2d's points: a polyline when it is all straight, else cubic pieces (a
/// straight piece as a cubic along it); closed drops a last point equal to the first
fn points(sub: &Sub) -> (Vec<[f64; 2]>, bool) {
    let straight = sub.segs.iter().all(|s| matches!(s, Seg::Line(_)));
    let mut pts = alloc::vec![sub.start];
    let mut prev = sub.start;
    for s in &sub.segs {
        match *s {
            Seg::Line(p) => {
                if straight {
                    pts.push(p);
                } else {
                    pts.push(lerp(prev, p, 1.0 / 3.0));
                    pts.push(lerp(prev, p, 2.0 / 3.0));
                    pts.push(p);
                }
            }
            Seg::Cubic(a, b, p) => {
                pts.push(a);
                pts.push(b);
                pts.push(p);
            }
        }
        prev = s.end();
    }
    if straight && sub.closed && pts.len() > 2 && pts.first() == pts.last() {
        pts.pop();
    }
    (pts, straight)
}

fn lerp(a: [f64; 2], b: [f64; 2], t: f64) -> [f64; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}
