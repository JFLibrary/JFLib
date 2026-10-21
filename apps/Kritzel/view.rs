// view -- the canvas view: zoom, scrolling, coordinate transforms, drawing  [apps/paint/src/view.s]
//
//   view_layout(p, ui, x, y, w, h)   set the view rectangle (px) and place the canvas
//   view_to_doc(v, sx, sy) -> (x, y)          screen px -> doc px (floor)
//   view_to_doc_fx(v, sx, sy) -> (x, y)       screen px -> doc 24.8 (pixel centre)
//   doc_to_view(v, x, y) -> (sx, sy)          doc px -> screen px
//   view_zoom(v) -> zoom in 1/1000 (1000 = 100 %)
//   view_set_zoom(p, ui, z, ax, ay)  zoom around the screen point ax, ay
//   view_zoom_step(p, ui, dir, ax, ay)  next (+1) / previous (-1) zoom level
//   view_fit(p, ui)                  zoom so the whole image fits
//   view_scroll(v, dx, dy)           scroll by screen px
//   view_version(p) -> hash of everything that changes the canvas pixels
//   view_extent(p, ui) -> (content w, content h, view w, view h)
//   view_draw_ref(p) -> the canvas draw object (view_draw: ui.canvas's draw callback)
//   view_frame_end(p, ui)            lends the layers to the draw object (end of every pass
//                                    and command); view_reclaim(p) takes them back
//   view_canvas_rect(v) -> [x, y, w, h] of the image on screen
//
// The image is centred while it is smaller than the view; otherwise it scrolls, with a
// margin around it.
//
// view_draw runs at render time and composites the dirty rectangle there, as the asm's
// callback does (headless runs render only for shots).  The draw object cannot reach Kritzel:
// at the end of every pass and command the document lends it its layers, flags and dirty
// rectangle (view_frame_end), and the next pass or command takes them back (view_reclaim);
// it also keeps the composite (shared with Doc), the canvas rectangle, the opaque flag and
// the checker cell size.

use alloc::rc::Rc;
use core::cell::{Cell, RefCell};

use gfx::Surface;
use ui::*;

use crate::consts::*;
use crate::doc::{composite_into, doc_h, doc_opaque, doc_version, doc_w};
use crate::Kritzel;

const ZOOM_MIN: i32 = 125;
const ZOOM_MAX: i32 = 8000;
static ZOOM_LEVELS: [i32; 12] = [125, 250, 333, 500, 667, 1000, 1500, 2000, 3000, 4000, 6000, 8000];

/// the view state (the .data / .bss of view.s)
pub(crate) struct View {
    pub(crate) zoom: i32,
    fit_pending: bool,
    /// the image on screen (px): g_cx, g_cy, g_cw, g_ch
    pub(crate) cx: i32,
    pub(crate) cy: i32,
    pub(crate) cw: i32,
    pub(crate) ch: i32,
    vx: i32,
    vy: i32,
    vw: i32,
    vh: i32,
    /// scroll position (px; view_scroll_ptr)
    pub(crate) sx: i32,
    pub(crate) sy: i32,
    draw: Option<Rc<ViewDraw>>,
}

impl Default for View {
    fn default() -> View {
        View {
            zoom: 1000,
            fit_pending: false,
            cx: 0,
            cy: 0,
            cw: 0,
            ch: 0,
            vx: 0,
            vy: 0,
            vw: 0,
            vh: 0,
            sx: 0,
            sy: 0,
            draw: None,
        }
    }
}

/// view_zoom() -> zoom in 1/1000
pub(crate) fn view_zoom(v: &View) -> i32 {
    v.zoom
}

/// view_canvas_rect() -> [x, y, w, h] of the image on screen
pub(crate) fn view_canvas_rect(v: &View) -> [i32; 4] {
    [v.cx, v.cy, v.cw, v.ch]
}

/// view_layout(x, y, w, h)
pub(crate) fn view_layout(p: &mut Kritzel, ui: &mut Ui, x: i32, y: i32, w: i32, h: i32) {
    let v = &mut p.view;
    v.vx = x;
    v.vy = y;
    v.vw = w;
    v.vh = h;
    if v.zoom == 0 {
        v.zoom = 1000;
    }
    if v.fit_pending && w > 0 {
        v.fit_pending = false;
        view_fit(p, ui); // lays out again with the fitted zoom
        return;
    }
    let m = ui.dp(24); // margin
    let z = v.zoom as u32;
    v.cw = ((doc_w(&p.doc) as u32).wrapping_mul(z).wrapping_add(500) / 1000) as i32;
    v.ch = ((doc_h(&p.doc) as u32).wrapping_mul(z).wrapping_add(500) / 1000) as i32;
    let (vx, vw, cw) = (v.vx, v.vw, v.cw);
    v.cx = vl_axis(vx, vw, cw, &mut v.sx, m);
    let (vy, vh, ch) = (v.vy, v.vh, v.ch);
    v.cy = vl_axis(vy, vh, ch, &mut v.sy, m);
}

// .vl_axis: view start, view len, canvas len, &scroll, margin -> canvas start
fn vl_axis(start: i32, len: i32, clen: i32, scroll: &mut i32, m: i32) -> i32 {
    let total = clen.wrapping_add(m.wrapping_mul(2)); // canvas + margins
    if total <= len {
        *scroll = 0;
        return (len.wrapping_sub(clen) >> 1).wrapping_add(start);
    }
    let max = total.wrapping_sub(len); // max scroll
    let s = (*scroll).max(0).min(max);
    *scroll = s;
    start.wrapping_add(m).wrapping_sub(s)
}

/// view_version() -> hash of everything that changes the canvas pixels: document version,
/// zoom and the canvas position (scrolling, centring)
pub(crate) fn view_version(p: &Kritzel) -> u32 {
    let v = &p.view;
    let mut h = doc_version(&p.doc);
    for x in [v.zoom, v.cx, v.cy, v.cw, v.ch] {
        h = h.wrapping_mul(0x0100_0193).wrapping_add(x as u32);
    }
    h
}

/// view_extent() -> (content width, content height (canvas + margins, px), view w, view h)
/// -- for scroll bars
pub(crate) fn view_extent(p: &Kritzel, ui: &Ui) -> (i32, i32, i32, i32) {
    let m = ui.dp(24).wrapping_mul(2);
    let v = &p.view;
    (v.cw.wrapping_add(m), v.ch.wrapping_add(m), v.vw, v.vh)
}

/// view_to_doc(sx, sy) -> (x, y)  (floor)
pub(crate) fn view_to_doc(v: &View, sx: i32, sy: i32) -> (i32, i32) {
    let fl = |d: i32| {
        let n = d.wrapping_mul(1000);
        let q = n.wrapping_div(v.zoom);
        if n.wrapping_rem(v.zoom) < 0 { q - 1 } else { q } // floor for negatives
    };
    (fl(sx.wrapping_sub(v.cx)), fl(sy.wrapping_sub(v.cy)))
}

/// view_to_doc_fx(sx, sy) -> (x, y) in 24.8 (pixel centres)
pub(crate) fn view_to_doc_fx(v: &View, sx: i32, sy: i32) -> (i32, i32) {
    // 64-bit: (screen - canvas origin) * 256 * 1000 overflows 32 bits at high zoom
    let z = v.zoom as i64;
    let f = |s: i32, c: i32| ((((s as i64 - c as i64) << 8) + 128).wrapping_mul(1000) / z) as i32;
    (f(sx, v.cx), f(sy, v.cy))
}

/// doc_to_view(x, y) -> (sx, sy)
pub(crate) fn doc_to_view(v: &View, x: i32, y: i32) -> (i32, i32) {
    let sx = (x.wrapping_mul(v.zoom) / 1000).wrapping_add(v.cx);
    let sy = (y.wrapping_mul(v.zoom) / 1000).wrapping_add(v.cy);
    (sx, sy)
}

/// view_scroll(dx, dy)
pub(crate) fn view_scroll(v: &mut View, dx: i32, dy: i32) {
    v.sx = v.sx.wrapping_add(dx);
    v.sy = v.sy.wrapping_add(dy);
}

fn relayout(p: &mut Kritzel, ui: &mut Ui) {
    let (x, y, w, h) = (p.view.vx, p.view.vy, p.view.vw, p.view.vh);
    view_layout(p, ui, x, y, w, h);
}

/// view_set_zoom(z, ax, ay) -- keep the doc point under (ax, ay) in place
pub(crate) fn view_set_zoom(p: &mut Kritzel, ui: &mut Ui, z: i32, ax: i32, ay: i32) {
    let z = z.max(ZOOM_MIN).min(ZOOM_MAX);
    // doc point under the anchor (24.8)
    let (fx, fy) = view_to_doc_fx(&p.view, ax, ay);
    p.view.zoom = z;
    relayout(p, ui);
    // where is that point now?  scroll the difference
    let v = &mut p.view;
    let dx = (fx.wrapping_mul(z) / 256000).wrapping_add(v.cx).wrapping_sub(ax);
    v.sx = v.sx.wrapping_add(dx);
    let dy = (fy.wrapping_mul(z) / 256000).wrapping_add(v.cy).wrapping_sub(ay);
    v.sy = v.sy.wrapping_add(dy);
    relayout(p, ui);
}

/// view_zoom_step(dir, ax, ay)
pub(crate) fn view_zoom_step(p: &mut Kritzel, ui: &mut Ui, dir: i32, ax: i32, ay: i32) {
    let z = p.view.zoom;
    let n = ZOOM_LEVELS.len();
    let i = if dir >= 0 {
        ZOOM_LEVELS.iter().position(|&l| l > z).unwrap_or(n - 1)
    } else {
        ZOOM_LEVELS.iter().rposition(|&l| l < z).unwrap_or(0)
    };
    view_set_zoom(p, ui, ZOOM_LEVELS[i], ax, ay);
}

/// view_fit() -- largest zoom that shows the whole image (at most 100 %)
pub(crate) fn view_fit(p: &mut Kritzel, ui: &mut Ui) {
    let m = ui.dp(24).wrapping_mul(2);
    let v = &mut p.view;
    // before the first layout the view size is unknown: fit on the next view_layout
    if v.vw.wrapping_sub(m) <= 0 || v.vh.wrapping_sub(m) <= 0 {
        v.fit_pending = true;
        return;
    }
    let zx = v.vw.wrapping_sub(m).wrapping_mul(1000) / doc_w(&p.doc);
    let zy = v.vh.wrapping_sub(m).wrapping_mul(1000) / doc_h(&p.doc);
    let z = zy.min(zx).min(1000);
    v.zoom = z;
    v.sx = 0;
    v.sy = 0;
    relayout(p, ui);
}

// ------------------------------------------------------------------ drawing

/// the document parts lent to the draw object between the end of a pass (or command) and the
/// next one, so the composite is built at render time as in the asm -- headless runs render
/// only for shots, and only then pay for it
struct Lent {
    held: bool,
    layers: [Surface; MAX_LAYERS],
    nlayers: i32,
    vis: [u8; MAX_LAYERS],
    opac: [u8; MAX_LAYERS],
    dirty: [i32; 4],
    dirty_set: bool,
    w: i32,
    h: i32,
}

/// view_draw's view of the document: what the asm callback reads from globals at render time
pub(crate) struct ViewDraw {
    comp: Rc<RefCell<Surface>>,
    lent: RefCell<Lent>,
    /// cx, cy, cw, ch
    rect: Cell<[i32; 4]>,
    opaque: Cell<bool>,
    /// checker cell (8 DIP)
    cell: Cell<i32>,
}

/// the canvas draw object (one per app: its identity keeps the canvas command stable)
pub(crate) fn view_draw_ref(p: &mut Kritzel) -> DrawRef {
    let comp = p.doc.comp.clone();
    let d = p.view.draw.get_or_insert_with(|| {
        let lent = Lent {
            held: false,
            layers: core::array::from_fn(|_| Surface::default()),
            nlayers: 0,
            vis: [0; MAX_LAYERS],
            opac: [0; MAX_LAYERS],
            dirty: [0; 4],
            dirty_set: false,
            w: 0,
            h: 0,
        };
        Rc::new(ViewDraw { comp, lent: RefCell::new(lent), rect: Cell::new([0; 4]), opaque: Cell::new(true), cell: Cell::new(8) })
    });
    d.clone()
}

/// end of a pass or command: lend the layers to the draw object (the composite is built
/// when the canvas is rendered) and set what it shows
pub(crate) fn view_frame_end(p: &mut Kritzel, ui: &mut Ui) {
    let Some(d) = p.view.draw.clone() else { return };
    {
        let mut l = d.lent.borrow_mut();
        if !l.held {
            let doc = &mut p.doc;
            core::mem::swap(&mut l.layers, &mut doc.layers);
            l.nlayers = doc.nlayers;
            l.vis = doc.layer_vis;
            l.opac = doc.layer_opac;
            l.dirty = doc.dirty;
            l.dirty_set = doc.dirty_set;
            l.w = doc_w(doc);
            l.h = doc_h(doc);
            l.held = true;
        }
    }
    let v = &p.view;
    d.rect.set([v.cx, v.cy, v.cw, v.ch]);
    d.opaque.set(doc_opaque(&p.doc));
    d.cell.set(ui.dp(8));
}

/// start of a pass or command: take the layers back (with the dirty state a render left)
pub(crate) fn view_reclaim(p: &mut Kritzel) {
    let Some(d) = p.view.draw.clone() else { return };
    let mut l = d.lent.borrow_mut();
    if !l.held {
        return;
    }
    let doc = &mut p.doc;
    core::mem::swap(&mut l.layers, &mut doc.layers);
    doc.dirty = l.dirty;
    doc.dirty_set = l.dirty_set;
    l.held = false;
}

impl Draw for ViewDraw {
    // view_draw(user, rect) -- render the image into the view (ui.canvas callback)
    fn draw(&self, g: &mut Gfx, _r: [i32; 4]) {
        {
            let mut l = self.lent.borrow_mut();
            if l.held {
                let Lent { layers, nlayers, vis, opac, dirty, dirty_set, w, h, .. } = &mut *l;
                let mut comp = self.comp.borrow_mut();
                composite_into(&mut comp, layers, *nlayers, vis, opac, dirty, dirty_set, *w, *h);
            }
        }
        let comp = self.comp.borrow();
        if !self.opaque.get() {
            self.draw_checker(g);
        }
        let [cx, cy, cw, ch] = self.rect.get();
        g.dc.blit_scaled(g.sf, &comp, cx, cy, cw, ch);
    }
}
impl ViewDraw {
    // draw_checker() -- light/grey 8 DIP checkerboard under the image
    fn draw_checker(&self, g: &mut Gfx) {
        let c = self.cell.get();
        let [cx, cy, cw, ch] = self.rect.get();
        g.dc.clip_push(cx, cy, cw, ch);
        g.dc.set_color(0xFFFF_FFFF);
        g.dc.fill(g.sf, cx, cy, cw, ch);
        g.dc.set_color(0xFFE0_E0E0);
        // visible cells only
        let [x0, y0, x1, y1] = g.dc.clip;
        let mut y = cy;
        while y < y1 {
            if y.wrapping_add(c) > y0 {
                let mut par = (y.wrapping_sub(cy) / c) & 1; // row parity
                let mut x = cx;
                while x < x1 {
                    if x.wrapping_add(c) > x0 && par & 1 != 0 {
                        g.dc.fill(g.sf, x, y, c, c);
                    }
                    x = x.wrapping_add(c);
                    par ^= 1;
                }
            }
            y = y.wrapping_add(c);
        }
        g.dc.clip_pop();
    }
}
