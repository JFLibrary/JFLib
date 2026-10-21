// view -- PaintCraft: the canvas view (workspace, image with checkerboard and drop shadow,
// zoom 1 %..6400 %, Ganzes Bild, scrolling, panning, pixel grid, rulers, status bar values,
// coordinate transforms and hooks for the tools).  Per-document state: Doc::view (DV_*).
//                                                                   [apps/PaintCraft/src/view.s]
//
// Module contract (main.rs / chrome.rs):
//   view_init()  view_states()  view_cmd(id, arg) -> handled  view_frame()
//   view_zoom() -> zoom of the active image in 1/100 % (10000 = 100 %)
//   view_units() -> UNIT_PX / UNIT_IN / UNIT_CM
//   view_cursor() -> (x, y, true while the pointer is over the canvas) (image px)
// For tools and other modules (screen = absolute window px, doc = image px):
//   view_screen_to_doc(sx, sy) -> (x, y)              floor
//   view_doc_to_screen(x, y) -> (sx, sy)              top-left corner of pixel (x, y)
//   view_screen_to_doc_fx(sx, sy) / view_doc_to_screen_fx(x, y)   24.8 fixed point in and out
//   view_screen_to_doc_f64(&pt) / view_doc_to_screen_f64(&pt)    in place, pt = [x, y]
//   view_canvas_rect() -> [x, y, w, h] of the canvas widget (px)
//   view_zoom_at(zoom, sx, sy)        zoom keeping the image point under (sx, sy) in place
//   view_zoom_step(dir, sx, sy)       next (dir > 0) / previous preset around (sx, sy)
//   view_pan_begin(sx, sy)  view_pan_move(sx, sy)  view_pan_end()     (Pan tool)
//   view_damage_doc(doc, x0, y0, x1, y1)  redraw that image area with the next frame (tool
//                                     overlays; pixel changes are reported by compose.rs)
//   view_invalidate()                 redraw the whole canvas with the next frame
//   view_overlay_add(fn, ctx) -> 0 / -1   fn(p, g, ctx, doc, canvas_rect) draws over the image
//                                     at render time (on the framebuffer, clipped)
//   view_overlay_remove(fn, ctx)
//   view_test_keys(flags)             test hook: 1 = Space held (diag.rs)
//   view_dpi100() -> resolution of the active image in dots per inch * 100 (9600 without an
//                                     image or resolution; status bar units)
//   view_set_opts(units, grid, rulers)    restore the Ansicht options (settings.rs)
// Input: the canvas UIINPUT goes to tool_input(doc, uiinput, x_fx, y_fx) (24.8 image
// coordinates of the pointer) while the pointer is over the canvas or a button is held,
// except while panning (Space + drag, middle drag).  Wheel: scroll, Shift: horizontal,
// Ctrl: zoom around the pointer.
// Rendering (view_draw, at render time): zoom >= 100 % nearest samples of the straight
// composite over the checkerboard (k_view_row); below 100 % the premultiplied mip level
// 2^-L with zoom * 2^L <= 100 % is shown 1:1 or bilinearly filtered (k_view_row_pm); rows
// are rendered in bands on the worker threads.  Pixel changes only damage their area
// (ui.canvas_damage); zoom / scroll / size changes redraw the canvas.
//
// The canvas is a ui::Draw object holding a Weak of the app (Pn lives in an Rc<RefCell>); at
// render time it borrows Pn and draws as the asm's callback does.  The transforms read the
// values view_frame keeps in View (the shown document's zoom and origin, the dpi), so they
// need only &View.

use alloc::rc::{Rc, Weak};
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

use gfx::straight::{premul_row, view_row, view_row_pm, ViewCfg};
use gfx::Surface;
use rt::fx::{cvtd, cvttd, maxsd, minsd};
use rt::sync::SpinLock;
use ui::*;

use crate::workspace::chrome::PN_ZOOM_PRESETS;
use crate::document::composite::{comp_damage_take, comp_mip_level, comp_mip_sf, comp_sf, comp_update};
use crate::app::constants::*;
use crate::document::model::{doc_active, Doc, DocId};
use crate::workspace::rulers::{ruler_draw, RulerParams};
use crate::app::commands::pn_cmd_set;
use crate::app::Pn;

const RULER_DIP: i32 = 20; // ruler thickness
const FIT_DIP: i32 = 16; // free space around a "Ganzes Bild" image
const MARGIN_DIP: i32 = 32; // overscroll beyond the image edges
const WHEEL_DIP: i32 = 48; // scroll distance per wheel notch
const CHECK_DIP: i32 = 8; // checkerboard cell
const BAR_DIP: i32 = 12; // overlay scroll bars
const GRID_ZOOM: u32 = 40000; // pixel grid from 400 %
const WS_LIGHT: u32 = 0xFFDADADA; // workspace background
const WS_DARK: u32 = 0xFF181818;
const GRID_COLOR: u32 = 0x55808080;
const OVL_MAX: usize = 8;
const BAND_ROWS: i32 = 32; // rows per render work item
const BAND_MAX: i32 = 64;
static SHADOW_ALPHA: [u8; 6] = [0x04, 0x06, 0x08, 0x0B, 0x10, 0x18];
const Z100: u32 = ZOOM_100 as u32;
const ZMIN: u32 = ZOOM_MIN as u32;
const ZMAX: u32 = ZOOM_MAX as u32;

/// a render-time overlay: fn(p, g, ctx, doc, canvas_rect)
pub(crate) type Overlay = fn(&mut Pn, &mut Gfx, usize, DocId, [i32; 4]);

/// the DOC_VIEW block (DV_*).  Part of Doc: Send + Sync.
#[derive(Clone, Copy, Default)]
pub(crate) struct DocView {
    /// DV_ZOOM: zoom in 1/100 % (0 = not placed yet)
    pub(crate) zoom: u32,
    /// DV_FIT: "Ganzes Bild": the zoom follows the window
    pub(crate) fit: bool,
    /// DV_PREVZOOM: zoom before "Ganzes Bild" (Strg+B toggles back)
    pub(crate) prevzoom: u32,
    /// DV_FLAGS: reserved
    pub(crate) flags: u32,
    /// DV_CX / DV_CY: document point shown at the centre of the canvas
    pub(crate) cx: f64,
    pub(crate) cy: f64,
    /// DV_PREVCX / DV_PREVCY: centre before "Ganzes Bild"
    pub(crate) prevcx: f64,
    pub(crate) prevcy: f64,
}

/// the canvas draw callback (view_draw): borrows the app at render time
struct CanvasDraw {
    p: Weak<RefCell<Pn>>,
}

impl Draw for CanvasDraw {
    fn draw(&self, g: &mut Gfx, _r: [i32; 4]) {
        let Some(rc) = self.p.upgrade() else { return };
        let Ok(mut p) = rc.try_borrow_mut() else { return };
        view_draw(&mut p, g);
    }
}

/// the state of view.s (its g_* globals)
pub(crate) struct View {
    /// g_vdoc: the document on screen
    pub(crate) vdoc: Option<DocId>,
    /// g_ovl: render-time overlays (fn, ctx)
    ovl: [Option<(Overlay, usize)>; OVL_MAX],
    /// g_cvx .. g_cvh: canvas rectangle (px)
    pub(crate) cv: [i32; 4],
    pub(crate) units: u32,
    pub(crate) grid: bool,
    pub(crate) rulers: bool,
    /// g_vz / g_vzf: the frame's zoom (1/100 %) and factor
    pub(crate) vz: u32,
    vzf: f64,
    /// g_vox / g_voy: screen position of image pixel (0, 0); g_viw / g_vih: image size on
    /// screen (px)
    pub(crate) vox: i32,
    pub(crate) voy: i32,
    pub(crate) viw: i32,
    pub(crate) vih: i32,
    /// g_vcur_*: cursor position for the status bar
    cur_x: i32,
    cur_y: i32,
    cur_ok: bool,
    panning: bool,
    pan_btn: u32,
    pan_mx: i32,
    pan_my: i32,
    pan_cx: f64,
    pan_cy: f64,
    /// g_vtest: 1 = Space held (test hook)
    vtest: u32,
    /// g_vver: bumped by view_invalidate
    vver: u32,
    hpos: i32,
    vpos: i32,
    /// g_vptr_x / g_vptr_y: pointer (absolute px)
    ptr_x: i32,
    ptr_y: i32,
    /// the dpi of the last frame (ui_dp outside a frame and at render time)
    pub(crate) dpi: i32,
    /// ui_color(T_ACCENT) of the last frame (render-time overlays: nubs)
    pub(crate) accent: u32,
    /// the canvas draw object
    draw: Option<Rc<CanvasDraw>>,
}

impl Default for View {
    fn default() -> View {
        View {
            vdoc: None,
            ovl: [None; OVL_MAX],
            cv: [0; 4],
            units: 0,
            grid: false,
            rulers: false,
            vz: Z100,
            vzf: 0.0,
            vox: 0,
            voy: 0,
            viw: 0,
            vih: 0,
            cur_x: 0,
            cur_y: 0,
            cur_ok: false,
            panning: false,
            pan_btn: 0,
            pan_mx: 0,
            pan_my: 0,
            pan_cx: 0.0,
            pan_cy: 0.0,
            vtest: 0,
            vver: 0,
            hpos: 0,
            vpos: 0,
            ptr_x: 0,
            ptr_y: 0,
            dpi: 96,
            accent: 0,
            draw: None,
        }
    }
}

impl View {
    fn dp(&self, dip: i32) -> i32 {
        dp_at(dip, self.dpi)
    }
}

/// view_init() -- units Pixel, grid and rulers off
pub(crate) fn view_init(p: &mut Pn, _ui: &mut Ui) {
    let v = &mut p.view;
    v.units = UNIT_PX as u32;
    v.grid = false;
    v.rulers = false;
    v.vz = Z100;
}

/// view_zoom() -> zoom of the active document (100 % before it was placed / none)
pub(crate) fn view_zoom(p: &Pn) -> u32 {
    match p.doc.active() {
        Some(d) if d.view.zoom != 0 => d.view.zoom,
        _ => Z100,
    }
}

/// view_units() -> UNIT_*
pub(crate) fn view_units(v: &View) -> u32 {
    v.units
}

/// view_grid() -> the pixel grid is on
pub(crate) fn view_grid(v: &View) -> bool {
    v.grid
}

/// view_rulers() -> the rulers are on
pub(crate) fn view_rulers(v: &View) -> bool {
    v.rulers
}

/// view_cursor() -> (x, y, valid)
pub(crate) fn view_cursor(v: &View) -> (i32, i32, bool) {
    (v.cur_x, v.cur_y, v.cur_ok)
}

/// view_canvas_rect() -> [x, y, w, h]
pub(crate) fn view_canvas_rect(v: &View) -> [i32; 4] {
    v.cv
}

/// view_origin() -> (x, y): screen position of image pixel (0, 0) relative to the canvas
pub(crate) fn view_origin(v: &View) -> (i32, i32) {
    (v.vox.wrapping_sub(v.cv[0]), v.voy.wrapping_sub(v.cv[1]))
}

/// view_dpi100() -> dots per inch * 100 of the active image (9600 = 96 dpi)
pub(crate) fn view_dpi100(p: &Pn) -> u32 {
    let Some(d) = p.doc.active() else { return 9600 };
    let mut x = d.dpu_x;
    match d.dpu_unit {
        1 => {}
        2 => x *= 2.54, // dots per cm -> per inch
        _ => return 9600,
    }
    let v = cvtd(x * 100.0);
    if v < 100 { 9600 } else { v as u32 } // nonsense resolution: 96 dpi
}

/// view_test_keys(flags) -- 1 = treat Space as held (scripts cannot hold Space; Ctrl + wheel:
/// `wheel S DY ctrl`)
pub(crate) fn view_test_keys(v: &mut View, flags: u32) {
    v.vtest = flags;
}

/// view_invalidate() -- the whole canvas is drawn again
pub(crate) fn view_invalidate(p: &mut Pn) {
    p.view.vver = p.view.vver.wrapping_add(1);
    p.invalidate();
}

// ------------------------------------------------------------------ transforms

// flr(x) -> floor (cvttsd2si, one step down when it truncated upwards)
fn flr(x: f64) -> i32 {
    let t = cvttd(x);
    if (t as f64) > x { t.wrapping_sub(1) } else { t }
}

// floordiv(a, b) -> floor(a / b), b > 0
fn floordiv(a: i64, b: i64) -> i64 {
    let q = a.wrapping_div(b);
    if a.wrapping_rem(b) < 0 { q - 1 } else { q }
}

/// view_screen_to_doc(sx, sy) -> (x, y) (floor)
pub(crate) fn view_screen_to_doc(v: &View, sx: i32, sy: i32) -> (i32, i32) {
    let z = v.vz as i64;
    let x = floordiv((sx as i64 - v.vox as i64).wrapping_mul(ZOOM_100 as i64), z);
    let y = floordiv((sy as i64 - v.voy as i64).wrapping_mul(ZOOM_100 as i64), z);
    (x as i32, y as i32)
}

/// view_doc_to_screen(x, y) -> (sx, sy): ox + ceil(x * zoom / 10000)
pub(crate) fn view_doc_to_screen(v: &View, x: i32, y: i32) -> (i32, i32) {
    let z = v.vz as i64;
    let sx = floordiv((x as i64).wrapping_mul(z) + (ZOOM_100 as i64 - 1), ZOOM_100 as i64) as i32;
    let sy = floordiv((y as i64).wrapping_mul(z) + (ZOOM_100 as i64 - 1), ZOOM_100 as i64) as i32;
    (sx.wrapping_add(v.vox), sy.wrapping_add(v.voy))
}

/// view_screen_to_doc_fx(sx, sy) -> (x, y) (24.8 in and out)
pub(crate) fn view_screen_to_doc_fx(v: &View, sx: i32, sy: i32) -> (i32, i32) {
    let z = v.vz as i64;
    let x = floordiv((sx as i64 - ((v.vox as i64) << 8)).wrapping_mul(ZOOM_100 as i64), z);
    let y = floordiv((sy as i64 - ((v.voy as i64) << 8)).wrapping_mul(ZOOM_100 as i64), z);
    (x as i32, y as i32)
}

/// view_doc_to_screen_fx(x, y) -> (sx, sy) (24.8 in and out)
pub(crate) fn view_doc_to_screen_fx(v: &View, x: i32, y: i32) -> (i32, i32) {
    let z = v.vz as i64;
    let sx = floordiv((x as i64).wrapping_mul(z), ZOOM_100 as i64) as i32;
    let sy = floordiv((y as i64).wrapping_mul(z), ZOOM_100 as i64) as i32;
    ((v.vox << 8).wrapping_add(sx), (v.voy << 8).wrapping_add(sy))
}

// zf() -> zoom factor g_vz / 10000
fn zf(v: &View) -> f64 {
    v.vz as i32 as f64 / 10000.0
}

/// view_screen_to_doc_f64(&pt) -- pt = (pt - origin) / z
pub(crate) fn view_screen_to_doc_f64(v: &View, pt: &mut [f64; 2]) {
    let z = zf(v);
    pt[0] = (pt[0] - v.vox as f64) / z;
    pt[1] = (pt[1] - v.voy as f64) / z;
}

/// view_doc_to_screen_f64(&pt) -- pt = origin + pt * z
pub(crate) fn view_doc_to_screen_f64(v: &View, pt: &mut [f64; 2]) {
    let z = zf(v);
    pt[0] = pt[0] * z + v.vox as f64;
    pt[1] = pt[1] * z + v.voy as f64;
}

// ------------------------------------------------------------------ placement

// fit_zoom(doc) -> the largest zoom <= 100 % showing the whole image in the canvas
fn fit_zoom(v: &View, d: &Doc) -> u32 {
    let m = v.dp(FIT_DIP * 2);
    let w = v.cv[2].wrapping_sub(m);
    let h = v.cv[3].wrapping_sub(m);
    if w <= 0 || h <= 0 || d.w <= 0 {
        return ZMIN;
    }
    let mut z = Z100 as u64;
    z = z.min(w as u64 * Z100 as u64 / d.w as u64);
    if d.h <= 0 {
        return ZMIN;
    }
    z = z.min(h as u64 * Z100 as u64 / d.h as u64);
    (z as u32).max(ZMIN)
}

// centre_doc(doc) -- the view centre = image centre
fn centre_doc(d: &mut Doc) {
    d.view.cx = d.w as f64 * 0.5;
    d.view.cy = d.h as f64 * 0.5;
}

// place_axis(size, view, &centre, margin) -> (origin relative to the view (px), the image
// length on screen (ceil)); uses vzf.  The image is centred while it fits, otherwise the
// centre is clamped so that at most `margin` px show beyond its edges.
fn place_axis(vzf: f64, size: i32, view: i32, centre: &mut f64, margin: i32) -> (i32, i32) {
    let img = size as f64 * vzf;
    let len = flr(0.0 - img).wrapping_neg();
    let vw = view as f64;
    if !(img > vw) {
        // fits: centred
        let o = flr((vw - img) * 0.5);
        *centre = size as f64 * 0.5;
        return (o, len);
    }
    // o = view / 2 - centre * z, clamped to [view - margin - img, margin]
    let half = vw * 0.5;
    let mut o = half - *centre * vzf;
    let mg = margin as f64;
    let lo = vw - mg - img;
    o = maxsd(o, lo);
    o = minsd(o, mg);
    let r = flr(o);
    // the centre from the clamped, unrounded origin: rounding never drifts it
    *centre = (half - o) / vzf;
    (r, len)
}

// view_place(doc) -- zoom (first placement, Ganzes Bild), clamped centre, and the frame's
// transform vz, vox, voy, viw, vih for the canvas rectangle
fn view_place(p: &mut Pn, doc: Option<DocId>) {
    let Some(doc) = doc else { return };
    let v = &mut p.view;
    let Some(d) = p.doc.get_mut(doc) else { return };
    if v.cv[2] <= 0 || v.cv[3] <= 0 {
        return;
    }
    if d.view.zoom == 0 {
        // first placement: 100 % when the image fits, else Ganzes Bild (Strg+B goes to 100 %)
        centre_doc(d);
        let z = fit_zoom(v, d);
        if z >= Z100 {
            d.view.zoom = Z100;
            d.view.fit = false;
        } else {
            d.view.zoom = z;
            d.view.fit = true;
            d.view.prevzoom = Z100;
            d.view.prevcx = d.view.cx;
            d.view.prevcy = d.view.cy;
        }
    }
    if d.view.fit {
        d.view.zoom = fit_zoom(v, d);
        centre_doc(d);
    }
    let z = d.view.zoom.clamp(ZMIN, ZMAX);
    d.view.zoom = z;
    v.vz = z;
    v.vzf = zf(v);
    let m = v.dp(MARGIN_DIP);
    let (o, l) = place_axis(v.vzf, d.w, v.cv[2], &mut d.view.cx, m);
    v.vox = o.wrapping_add(v.cv[0]);
    v.viw = l;
    let (o, l) = place_axis(v.vzf, d.h, v.cv[3], &mut d.view.cy, m);
    v.voy = o.wrapping_add(v.cv[1]);
    v.vih = l;
}

// ------------------------------------------------------------------ zoom / pan

/// view_zoom_at(zoom, sx, sy) -- active document; the image point under (sx, sy) stays put
pub(crate) fn view_zoom_at(p: &mut Pn, zoom: u32, sx: i32, sy: i32) {
    let Some(doc) = doc_active(&p.doc) else { return };
    let z = (zoom as i32).max(ZOOM_MIN).min(ZOOM_MAX) as u32;
    let v = &p.view;
    let d = p.doc.dm(doc);
    d.view.fit = false;
    let old = d.view.zoom;
    d.view.zoom = z;
    if old == 0 {
        centre_doc(d); // never placed: centre the image
    } else if v.vdoc == Some(doc) && z != old && v.cv[2] > 0 {
        // image point under the anchor: d = (a - o) / z_old ; new o = a - d * z_new
        let zo = old as i32 as f64 / 10000.0;
        let zn = z as i32 as f64 / 10000.0;
        let dx = sx.wrapping_sub(v.vox) as f64 / zo * zn;
        let ox = sx.wrapping_sub(v.cv[0]) as f64 - dx;
        d.view.cx = (v.cv[2] as f64 * 0.5 - ox) / zn;
        let dy = sy.wrapping_sub(v.voy) as f64 / zo * zn;
        let oy = sy.wrapping_sub(v.cv[1]) as f64 - dy;
        d.view.cy = (v.cv[3] as f64 * 0.5 - oy) / zn;
        view_place(p, Some(doc));
    }
    p.repass();
}

// preset_step(zoom, dir) -> the next preset above (dir > 0) / below (dir <= 0), or zoom
fn preset_step(zoom: u32, dir: i32) -> u32 {
    if dir > 0 {
        PN_ZOOM_PRESETS.iter().rev().copied().find(|&z| z > zoom).unwrap_or(zoom)
    } else {
        PN_ZOOM_PRESETS.iter().copied().find(|&z| z < zoom).unwrap_or(zoom)
    }
}

/// view_zoom_step(dir, sx, sy) -- next / previous zoom preset around (sx, sy)
pub(crate) fn view_zoom_step(p: &mut Pn, dir: i32, sx: i32, sy: i32) {
    let z = preset_step(view_zoom(p), dir);
    view_zoom_at(p, z, sx, sy);
}

// zoom_centre(zoom) -- view_zoom_at around the canvas centre
fn zoom_centre(p: &mut Pn, zoom: u32) {
    let c = p.view.cv;
    view_zoom_at(p, zoom, (c[2] >> 1).wrapping_add(c[0]), (c[3] >> 1).wrapping_add(c[1]));
}

/// view_pan_begin(sx, sy)
pub(crate) fn view_pan_begin(p: &mut Pn, sx: i32, sy: i32) {
    let v = &mut p.view;
    v.pan_mx = sx;
    v.pan_my = sy;
    if let Some(d) = p.doc.active() {
        v.pan_cx = d.view.cx;
        v.pan_cy = d.view.cy;
        v.panning = true;
    }
}

/// view_pan_move(sx, sy) -- the image follows the pointer
pub(crate) fn view_pan_move(p: &mut Pn, sx: i32, sy: i32) {
    if !p.view.panning {
        return;
    }
    let Some(doc) = doc_active(&p.doc) else { return };
    let v = &p.view;
    let z = zf(v);
    let d = p.doc.dm(doc);
    d.view.cx = v.pan_cx - sx.wrapping_sub(v.pan_mx) as f64 / z;
    d.view.cy = v.pan_cy - sy.wrapping_sub(v.pan_my) as f64 / z;
    let (ox, oy) = (v.vox, v.voy);
    view_place(p, Some(doc));
    // another pass only when the image moved
    if ox != p.view.vox || oy != p.view.voy {
        p.repass();
    }
}

/// view_pan_end()
pub(crate) fn view_pan_end(p: &mut Pn) {
    p.view.panning = false;
}

/// view_damage_doc(doc, x0, y0, x1, y1) -- screen area of that image rectangle is redrawn
/// (only for the document on screen)
pub(crate) fn view_damage_doc(v: &View, ui: &mut Ui, doc: DocId, x0: i32, y0: i32, x1: i32, y1: i32) {
    if v.vdoc != Some(doc) {
        return;
    }
    let (a, b) = view_doc_to_screen(v, x0, y0);
    let (c, d) = view_doc_to_screen(v, x1, y1);
    // 2 px more on every side (bilinear footprint, rounding)
    let (a, b, c, d) = (a - 2, b - 2, c + 2, d + 2);
    ui.canvas_damage(ID_CANVAS, a - v.cv[0], b - v.cv[1], c - a, d - b);
}

/// view_overlay_add(fn, ctx) -> 0 / -1 (full list)
pub(crate) fn view_overlay_add(p: &mut Pn, f: Overlay, ctx: usize) -> i32 {
    let Some(s) = p.view.ovl.iter_mut().find(|o| o.is_none()) else { return -1 };
    *s = Some((f, ctx));
    view_invalidate(p);
    0
}

/// view_overlay_remove(fn, ctx)
pub(crate) fn view_overlay_remove(p: &mut Pn, f: Overlay, ctx: usize) {
    for s in p.view.ovl.iter_mut() {
        if let Some((g, c)) = *s {
            if core::ptr::fn_addr_eq(g, f) && c == ctx {
                *s = None;
            }
        }
    }
    view_invalidate(p);
}

// ------------------------------------------------------------------ commands

/// view_states() -- toggles, unit radio items, zoom limits
pub(crate) fn view_states(p: &mut Pn, ui: &mut Ui) {
    let v = &p.view;
    pn_cmd_set(ui, CMD_GRID, if v.grid { CF_CHECKED } else { 0 });
    pn_cmd_set(ui, CMD_RULERS, if v.rulers { CF_CHECKED } else { 0 });
    for u in 0..3u32 {
        pn_cmd_set(ui, CMD_UNIT_PX + u, if u == v.units { CF_CHECKED } else { 0 });
    }
    let f = if p.doc.active().is_some() { 0 } else { CF_DISABLED };
    let z = view_zoom(p);
    pn_cmd_set(ui, CMD_ZOOM_IN, if z < ZMAX { f } else { CF_DISABLED });
    pn_cmd_set(ui, CMD_ZOOM_OUT, if z > ZMIN { f } else { CF_DISABLED });
    pn_cmd_set(ui, CMD_ZOOM_WINDOW, f);
    pn_cmd_set(ui, CMD_ZOOM_ACTUAL, f);
}

/// view_cmd(id, arg) -> handled
pub(crate) fn view_cmd(p: &mut Pn, ui: &mut Ui, id: u32, arg: u32) -> bool {
    match id {
        CMD_ZOOM_IN => {
            let z = preset_step(view_zoom(p), 1);
            zoom_centre(p, z);
        }
        CMD_ZOOM_OUT => {
            let z = preset_step(view_zoom(p), 0);
            zoom_centre(p, z);
        }
        CMD_ZOOM_WINDOW => zoom_window_toggle(p),
        CMD_ZOOM_ACTUAL => zoom_centre(p, Z100),
        CMD_GRID => {
            p.view.grid = !p.view.grid;
            view_invalidate(p);
        }
        CMD_RULERS => p.view.rulers = !p.view.rulers,
        CMD_UNIT_PX | CMD_UNIT_IN | CMD_UNIT_CM => p.view.units = id - CMD_UNIT_PX,
        CMD_ZOOM_SET => {
            if arg == 0 {
                zoom_window_on(p);
            } else {
                zoom_centre(p, arg);
            }
        }
        CMD_UNIT_SET => {
            if arg < 3 {
                p.view.units = arg;
            }
        }
        _ => return false,
    }
    ui.repass();
    true
}

// zoom_window_on() -- Ganzes Bild for the active document (remembers zoom and centre)
fn zoom_window_on(p: &mut Pn) {
    let Some(d) = p.doc.active_mut() else { return };
    let v = &mut d.view;
    if v.fit {
        return;
    }
    v.prevzoom = v.zoom;
    v.prevcx = v.cx;
    v.prevcy = v.cy;
    v.fit = true;
}

// zoom_window_toggle() -- Strg+B: Ganzes Bild, or back to the zoom before it
fn zoom_window_toggle(p: &mut Pn) {
    let Some(d) = p.doc.active_mut() else { return };
    let v = &mut d.view;
    if !v.fit {
        zoom_window_on(p);
        return;
    }
    if v.prevzoom == 0 {
        return; // nothing to go back to: stay
    }
    v.fit = false;
    v.zoom = v.prevzoom;
    v.cx = v.prevcx;
    v.cy = v.prevcy;
}

// ------------------------------------------------------------------ frame

// view_version() -> hash of everything that changes the whole canvas
fn view_version(v: &View, ui: &Ui) -> u32 {
    let mut h = v.vz.wrapping_mul(0x0100_0193);
    for x in [v.vox, v.voy, v.viw, v.vih, v.cv[2], v.cv[3], v.vver as i32, v.grid as i32] {
        h = h.wrapping_add(x as u32).wrapping_mul(0x0100_0193);
    }
    h = h.wrapping_add(ui.theme_dark() as u32).wrapping_mul(0x0100_0193);
    h.wrapping_add(ui.dpi() as u32)
}

// box_fill(argb) -- fill the current box's rectangle
fn box_fill(ui: &mut Ui, argb: u32) {
    ui.draw_color(argb);
    let r = ui.box_rect();
    ui.draw_rect(r[0], r[1], r[2], r[3]);
}

// draw_shadow() -- soft drop shadow around the image (paint.net 5): six translucent rectangles
// 6..1 DIP larger than the image, shifted down by a third
fn draw_shadow(v: &View, ui: &mut Ui) {
    let dark = ui.theme_dark();
    for (k, &a) in SHADOW_ALPHA.iter().enumerate() {
        let g = ui.dp(6 - k as i32); // grow (px)
        let a = if dark { a as u32 * 2 } else { a as u32 }; // darker on the dark workspace
        ui.draw_color(a << 24);
        ui.draw_rect(v.vox - g, (g >> 2) + v.voy - g, v.viw + g * 2, v.vih + g * 2);
    }
}

/// view_frame() -- rulers, workspace, canvas, scroll bars (fills the canvas area of main.rs)
pub(crate) fn view_frame(p: &mut Pn, ui: &mut Ui) {
    let doc = doc_active(&p.doc);
    p.view.vdoc = doc;
    p.view.dpi = ui.dpi();
    p.view.accent = ui.color(T_ACCENT);
    let mut hr = [0i32; 3];
    let mut vr = [0i32; 3];
    let rpx = ui.dp(RULER_DIP);
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CLIP | BOX_FIXED, 0);
    crate::integrations::file_io::fio_infobar(p, ui);
    if p.view.rulers {
        // top ruler row: corner + horizontal ruler
        ui.next_size(-1, RULER_DIP);
        ui.box_begin(0, BOX_H | BOX_FIXED, 0);
        let r = ui.box_rect();
        hr = [r[0] + rpx, r[1], r[2] - rpx];
        ruler_corner(ui);
        ui.box_end();
    }
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_H | BOX_FIXED, 0);
    if p.view.rulers {
        ui.next_size(RULER_DIP, -1);
        ui.box_begin(0, BOX_V | BOX_FIXED, 0);
        let r = ui.box_rect();
        vr = [r[0], r[1], r[3]];
        ui.box_end();
    }
    // the canvas area: workspace, shadow, canvas, scroll bars
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CLIP | BOX_FIXED, 0);
    p.view.cv = ui.box_rect();
    box_fill(ui, if p.st.st.workspace != 0 {p.st.st.workspace | 0xFF000000} else if ui.theme_dark() { WS_DARK } else { WS_LIGHT });
    view_place(p, doc);
    if doc.is_some() && p.st.st.canvas_shadow != 0 {
        draw_shadow(&p.view, ui);
    }
    // cursor: the tool's (T1: tool_cursor), a hand while Space is held or while panning
    let mut cur = crate::tools::tool_cursor(p, ui);
    if p.view.panning || space_held(&p.view, ui) {
        cur = CUR_HAND;
    }
    ui.next_size(-1, -1);
    ui.next_flags(cur << HB_CURSOR_SHIFT);
    let ver = view_version(&p.view, ui) ^ p.st.st.checker0.rotate_left(7) ^ p.st.st.checker1.rotate_left(17);
    if p.view.draw.is_none() {
        p.view.draw = Some(Rc::new(CanvasDraw { p: p.me.clone() }));
    }
    let dr: Option<DrawRef> = p.view.draw.clone().map(|d| d as DrawRef);
    let user = doc.map_or(0, |d| d.0 as usize);
    let inp = ui.canvas(ID_CANVAS, dr, user, ver).clone();
    if view_input(p, ui, &inp) {
        view_place(p, doc);
        ui.repass();
    }
    // changed image pixels: redraw only their screen area
    if let Some(doc) = doc {
        if let Some(r) = comp_damage_take(p.doc.dm(doc)) {
            view_damage_doc(&p.view, ui, doc, r[0], r[1], r[2], r[3]);
        }
        scroll_bars(p, ui, doc);
    }
    ui.box_end();
    ui.box_end();
    // rulers (the transform is known now)
    if p.view.rulers {
        view_ruler(p, ui, hr[0], hr[1], hr[2], false);
        view_ruler(p, ui, vr[0], vr[1], vr[2], true);
    }
    ui.box_end();
}

// space_held() -> true while Space (or the test flag) is down
fn space_held(v: &View, ui: &Ui) -> bool {
    v.vtest & 1 != 0 || ui.key_down(VK_SPACE)
}

// view_input(ui) -> true when the transform changed -- cursor position, panning, wheel, tool
// input
fn view_input(p: &mut Pn, ui: &mut Ui, inp: &UiInput) -> bool {
    let mut changed = false;
    let doc = p.view.vdoc;
    let px = inp.x.wrapping_add(inp.rect[0]);
    let py = inp.y.wrapping_add(inp.rect[1]);
    p.view.ptr_x = px;
    p.view.ptr_y = py;
    let mods = inp.mods;
    // cursor position for the status bar
    p.view.cur_ok = false;
    let Some(doc) = doc else { return false };
    if inp.ev & EV_HOVER != 0 {
        let (x, y) = view_screen_to_doc(&p.view, px, py);
        p.view.cur_x = x;
        p.view.cur_y = y;
        p.view.cur_ok = true;
    }
    // panning: Space + left drag, middle drag
    if p.view.panning {
        if inp.btn & (1 << p.view.pan_btn) != 0 {
            view_pan_move(p, px, py);
        } else {
            view_pan_end(p);
        }
    } else if inp.ev & EV_HOVER != 0 {
        let mut go = false;
        if inp.pressed & (1 << BTN_MIDDLE) != 0 {
            p.view.pan_btn = BTN_MIDDLE;
            go = true;
        } else if inp.pressed & (1 << BTN_LEFT) != 0 && space_held(&p.view, ui) {
            p.view.pan_btn = BTN_LEFT;
            go = true;
        }
        if go {
            view_pan_begin(p, px, py);
        }
    }
    let wy = inp.wheel_y;
    let mut tool = false;
    if wy != 0 {
        if mods & MOD_CTRL != 0 {
            // Strg + wheel: one preset per notch around the pointer
            let n = (wy.unsigned_abs() + 119) / 120; // notches (at least 1)
            for _ in 0..n {
                view_zoom_step(p, if wy > 0 { 1 } else { -1 }, px, py);
            }
            changed = true;
            tool = true;
        } else {
            // wheel up (> 0) shows the upper part: the centre moves up
            let s = wy.wrapping_neg();
            if mods & MOD_SHIFT != 0 {
                scroll_by(p, s, 0); // Shift: horizontal
            } else {
                scroll_by(p, 0, s);
            }
            changed = true;
        }
    }
    if !tool && inp.wheel_x != 0 {
        scroll_by(p, inp.wheel_x, 0);
        changed = true;
    }
    // the tool gets the pointer unless the view pans
    if !p.view.panning && !space_held(&p.view, ui) && inp.ev & (EV_HOVER | EV_HELD) != 0 {
        let (fx, fy) = view_screen_to_doc_fx(&p.view, (px << 8) + 128, (py << 8) + 128);
        if crate::tools::tool_input(p, ui, doc, inp, fx, fy) {
            ui.repass();
        }
    }
    changed
}

// scroll_by(dx, dy) -- scroll the active document by wheel units (120 = one notch; a positive
// value moves the image content left / up)
fn scroll_by(p: &mut Pn, dx: i32, dy: i32) {
    let Some(doc) = doc_active(&p.doc) else { return };
    let v = &p.view;
    let k = v.dp(WHEEL_DIP) as f64 / 120.0; // px per wheel unit
    let z = zf(v);
    let d = p.doc.dm(doc);
    d.view.cx += dx as f64 * k / z;
    d.view.cy += dy as f64 * k / z;
}

// scroll_bars() -- overlay scroll bars along the right and bottom edge while the image is
// larger than the canvas
fn scroll_bars(p: &mut Pn, ui: &mut Ui, doc: DocId) {
    let bar = ui.dp(BAR_DIP);
    let m = ui.dp(MARGIN_DIP);
    let v = &mut p.view;
    let cw = v.viw + m * 2;
    let ch = v.vih + m * 2;
    let cv = v.cv;
    // horizontal
    if v.viw > cv[2] {
        v.hpos = m + cv[0] - v.vox;
        ui.next_rect_px(cv[0], cv[1] + cv[3] - bar, cv[2] - bar, bar);
        if ui.scrollbar(ID_HSCROLL, &mut v.hpos, cw, cv[2], false) & EV_ACT != 0 {
            // new origin = margin - pos
            let o = (m - v.hpos) as f64;
            let vzf = v.vzf;
            p.doc.dm(doc).view.cx = (cv[2] as f64 * 0.5 - o) / vzf;
            view_place(p, Some(doc));
            ui.repass();
        }
    }
    let v = &mut p.view;
    let cv = v.cv;
    if v.vih > cv[3] {
        v.vpos = m + cv[1] - v.voy;
        ui.next_rect_px(cv[0] + cv[2] - bar, cv[1], bar, cv[3] - bar);
        if ui.scrollbar(ID_VSCROLL, &mut v.vpos, ch, cv[3], true) & EV_ACT != 0 {
            let o = (m - v.vpos) as f64;
            let vzf = v.vzf;
            p.doc.dm(doc).view.cy = (cv[3] as f64 * 0.5 - o) / vzf;
            view_place(p, Some(doc));
            ui.repass();
        }
    }
}

// ruler_corner() -- the square where the two rulers meet
fn ruler_corner(ui: &mut Ui) {
    let c = ui.color(T_BG);
    ui.draw_color(c);
    let r = ui.box_rect();
    let s = ui.dp(RULER_DIP);
    ui.draw_rect(r[0], r[1], s, s);
}

// view_ruler(x, y, len, vertical) -- one ruler band with the current transform
fn view_ruler(p: &Pn, ui: &mut Ui, x: i32, y: i32, len: i32, vert: bool) {
    if len <= 0 {
        return;
    }
    let v = &p.view;
    let mut rp = RulerParams {
        x,
        y,
        len,
        thick: ui.dp(RULER_DIP),
        vert,
        zoom: v.vz,
        units: v.units,
        ..RulerParams::default()
    };
    let (o, l, c) = if vert { (v.voy, v.vih, v.ptr_y) } else { (v.vox, v.viw, v.ptr_x) };
    rp.origin = o;
    rp.imglen = l;
    // pointer marker (screen px) while the pointer is over the canvas
    rp.cur = if v.cur_ok { c } else { -100000 };
    // image pixels per unit from the document resolution
    rp.ppu = ruler_ppu(rp.units, v.vdoc.and_then(|d| p.doc.get(d)));
    ruler_draw(ui, &rp);
}

// ruler_ppu(rp, doc) -- image pixels per display unit (px: 1; in / cm from the document
// resolution, 96 dpi without one)
fn ruler_ppu(units: u32, d: Option<&Doc>) -> f64 {
    if units == UNIT_PX as u32 {
        return 1.0;
    }
    let mut x = 96.0; // dots per inch
    if let Some(d) = d {
        if d.dpu_unit == 1 {
            x = d.dpu_x;
        } else if d.dpu_unit == 2 {
            x = d.dpu_x * 2.54; // dots per cm -> per inch
        }
    }
    if units == UNIT_IN as u32 { x } else { x / 2.54 } // per cm
}

// ------------------------------------------------------------------ rendering

/// the render job (g_vjob) of one view_draw
struct VJob<'a> {
    x0: i32,
    x1: i32,
    y0: i32,
    y1: i32,
    ox: i32,
    oy: i32,
    zoom: u32,
    /// 0 nearest (>= 100 %), 1 level 1:1, 2 bilinear
    mode: u32,
    rows: i32,
    src: &'a Surface,
    /// bilinear: first source column used, source columns used, 1 = level 0 (straight)
    xsmin: i32,
    xspan: i32,
    straight: bool,
    /// per column: nearest: the image column; bilinear: i0, i1 (relative to xsmin), w0 | w1 << 16
    xtab: Vec<[i32; 3]>,
    den: i64,
    cfg: ViewCfg,
}

// view_draw(doc, rect) -- canvas render callback: the image (clipped), grid, overlays
fn view_draw(p: &mut Pn, g: &mut Gfx) {
    let Some(doc) = p.view.vdoc else { return };
    if p.doc.get(doc).is_none() {
        return;
    }
    let clip = g.dc.clip;
    let v = &p.view;
    // visible part of the image: clip & image rectangle
    let x0 = v.vox.max(clip[0]);
    let x1 = v.vox.wrapping_add(v.viw).min(clip[2]);
    let y0 = v.voy.max(clip[1]);
    let y1 = v.voy.wrapping_add(v.vih).min(clip[3]);
    let n = x1 - x0;
    if n > 0 && y1 > y0 {
        render_image(p, g, doc, x0, x1, y0, y1);
    }
    // overlays
    let cv = p.view.cv;
    for k in 0..OVL_MAX {
        if let Some((f, ctx)) = p.view.ovl[k] {
            f(p, g, ctx, doc, cv);
        }
    }
}

// the image part of view_draw: sources, tables, bands, grid
fn render_image(p: &mut Pn, g: &mut Gfx, doc: DocId, x0: i32, x1: i32, y0: i32, y1: i32) {
    let zoom = p.view.vz;
    // source: level L, the largest with zoom * 2^L <= 100 %
    let mut level = 0i32;
    if zoom < Z100 {
        let mut z = zoom;
        while z + z <= Z100 {
            z += z;
            level += 1;
        }
        if level != 0 {
            match comp_mip_level(p, doc, level) {
                Some(k) => level = k,
                None => level = 0,
            }
        }
    }
    if level == 0 {
        comp_update(p, doc);
    }
    let v = &p.view;
    let d = p.doc.d(doc);
    let src = if level != 0 { comp_mip_sf(d, level) } else { Some(comp_sf(d)) };
    let Some(src) = src else { return };
    if src.px.is_empty() {
        return;
    }
    let mut j = VJob {
        x0,
        x1,
        y0,
        y1,
        ox: v.vox,
        oy: v.voy,
        zoom,
        mode: 0,
        rows: 0,
        src,
        xsmin: 0,
        xspan: 0,
        straight: level == 0,
        xtab: Vec::new(),
        den: 0,
        cfg: ViewCfg { cell: v.dp(CHECK_DIP), c0: p.st.st.checker0 & 0xFFFFFF, c1: p.st.st.checker1 & 0xFFFFFF },
    };
    if zoom < Z100 {
        // den = zoom * 2^L: 10000 = 1:1, else bilinear
        j.den = (zoom as i64) << level;
        j.mode = if j.den == Z100 as i64 { 1 } else { 2 };
    }
    view_tables(&mut j, x1 - x0, y1 - y0);
    // rows in bands on the worker threads
    let fbs = g.sf.stride as usize;
    let nb = ((y1 - y0 + j.rows - 1) / j.rows) as usize;
    let cm = &p.cm;
    {
        let fb = &mut g.sf.px[y0 as usize * fbs..];
        let bands: Vec<SpinLock<&mut [u8]>> = fb.chunks_mut(j.rows as usize * fbs).take(nb).map(SpinLock::new).collect();
        let jr = &j;
        let job = |b: usize, _: &plat::JobCtl| {
            if let Some(s) = bands.get(b) {
                let mut gd = s.lock();
                view_band(jr, cm, b as i32, fbs, &mut gd);
            }
        };
        let _ = plat::parallel_for(nb, &job);
    }
    draw_grid(&p.view, g, &j, y1);
}

// view_tables(n) -- per column table and bands of the job
fn view_tables(j: &mut VJob, n: i32, rows: i32) {
    // bands: at most BAND_MAX of about BAND_ROWS rows
    let bands = ((rows + BAND_ROWS - 1) as u32 / BAND_ROWS as u32).min(BAND_MAX as u32) as i32;
    j.rows = ((rows + bands - 1) as u32 / bands as u32) as i32;
    match j.mode {
        0 => {
            // nearest: image column of every screen column
            j.xtab = (0..n)
                .map(|i| {
                    let s = (j.x0 + i - j.ox) as i64 * ZOOM_100 as i64;
                    let c = (floordiv(s, j.zoom as i64) as i32).min(j.src.w - 1).max(0);
                    [c, 0, 0]
                })
                .collect();
        }
        2 => {
            bil_span(j);
            j.xtab = (0..n)
                .map(|i| {
                    let (i0, i1, w) = bil_coord(j.den, j.x0 + i - j.ox, j.src.w);
                    [i0 - j.xsmin, i1 - j.xsmin, w as i32]
                })
                .collect();
        }
        _ => {}
    }
}

// bil_coord(s, size) -> (i0, i1, w0 | w1 << 16) -- source position of screen offset s (>= 0,
// from the image origin) at den: u = (s + 0.5) * 10000 / den - 0.5 in 1/128, clamped to
// 0 .. size - 1 (7-bit weights, w0 + w1 = 128)
fn bil_coord(den: i64, s: i32, size: i32) -> (i32, i32, u32) {
    let mut u = ((s as i64) * 2 + 1).wrapping_mul(ZOOM_100 as i64 * 128) / (den + den) - 64;
    if u < 0 {
        u = 0;
    }
    let mut w1 = (u & 127) as u32;
    let mut i0 = (u >> 7) as i32;
    let last = size - 1;
    if i0 >= last {
        i0 = last;
        w1 = 0;
    }
    let i1 = (i0 + 1).min(last);
    (i0, i1, (w1 << 16) | (128 - w1))
}

// bil_span() -- xsmin / xspan: the source columns the bilinear table reads
fn bil_span(j: &mut VJob) {
    let (a, _, _) = bil_coord(j.den, j.x0 - j.ox, j.src.w);
    let (_, b, _) = bil_coord(j.den, j.x1 - 1 - j.ox, j.src.w);
    j.xsmin = a;
    j.xspan = b - a + 1;
}

// view_band(job, band) -- worker: the rows of one band (fb = its framebuffer rows)
fn view_band(j: &VJob, cm: &crate::document::color::Cm, band: i32, fbs: usize, fb: &mut [u8]) {
    let ys = band * j.rows + j.y0;
    let ye = (ys + j.rows).min(j.y1);
    let n = (j.x1 - j.x0) as usize;
    let src = j.src;
    let ss = src.stride as usize;
    let mut exp: Vec<u8> = Vec::new();
    let mut cached = -1i32;
    let (mut h0, mut h1, mut mix, mut prem) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    if j.mode == 0 && j.zoom != Z100 {
        exp = vec![0u8; n * 4];
    }
    if j.mode == 2 {
        h0 = vec![0u16; n * 4];
        h1 = vec![0u16; n * 4];
        mix = vec![0u8; n * 4];
        if j.straight {
            prem = vec![0u8; j.xspan.max(0) as usize * 4];
        }
    }
    for y in ys..ye {
        let o = (y - ys) as usize * fbs + j.x0 as usize * 4;
        if o + n * 4 > fb.len() {
            break;
        }
        let dst = &mut fb[o..o + n * 4];
        match j.mode {
            1 => {
                // premultiplied level shown 1:1
                let r = (y - j.oy).min(src.h - 1) as usize;
                let sx = (j.x0 - j.ox) as usize;
                // the level may be one pixel narrower
                let m = n.min((src.w as usize).saturating_sub(sx));
                let s = &src.px[r * ss + sx * 4..r * ss + (sx + m) * 4];
                view_row_pm(&mut dst[..m * 4], s, j.x0 - j.ox, y - j.oy, &j.cfg);
                crate::document::color::cm_view_row(cm, &mut dst[..m * 4], m as i32);
            }
            2 => {
                let (r0, r1, w) = bil_coord(j.den, y - j.oy, src.h);
                bil_hrow(j, r0, &mut h0, &mut prem);
                bil_hrow(j, r1, &mut h1, &mut prem);
                bil_vmix(&h0, &h1, &mut mix, n, w);
                view_row_pm(dst, &mix, j.x0 - j.ox, y - j.oy, &j.cfg);
                crate::document::color::cm_view_row(cm, dst, n as i32);
            }
            _ => {
                // nearest from the straight composite
                let r = (floordiv((y - j.oy) as i64 * ZOOM_100 as i64, j.zoom as i64) as i32).min(src.h - 1);
                let row = &src.px[r as usize * ss..];
                if j.zoom == Z100 {
                    let sx = (j.x0 - j.ox) as usize * 4;
                    view_row(dst, &row[sx..sx + n * 4], j.x0 - j.ox, y - j.oy, &j.cfg);
                } else {
                    if r != cached {
                        cached = r;
                        for (i, t) in j.xtab.iter().enumerate() {
                            let c = t[0] as usize * 4;
                            exp[i * 4..i * 4 + 4].copy_from_slice(&row[c..c + 4]);
                        }
                    }
                    view_row(dst, &exp, j.x0 - j.ox, y - j.oy, &j.cfg);
                }
                crate::document::color::cm_view_row(cm, dst, n as i32);
            }
        }
    }
}

// bil_hrow(job, row, dst, prem) -- source row `row` filtered horizontally with the column
// table into dst (n x 4 words = channel * 128); a straight source is premultiplied into prem
// first
fn bil_hrow(j: &VJob, row: i32, dst: &mut [u16], prem: &mut [u8]) {
    let ss = j.src.stride as usize;
    let o = row as usize * ss + j.xsmin as usize * 4;
    let span = j.xspan.max(0) as usize * 4;
    let s: &[u8] = if j.straight {
        premul_row(&mut prem[..span], &j.src.px[o..o + span]);
        prem
    } else {
        &j.src.px[o..]
    };
    for (i, t) in j.xtab.iter().enumerate() {
        let (a, b) = (t[0] as usize * 4, t[1] as usize * 4);
        let w0 = (t[2] & 0xFFFF) as u32;
        let w1 = (t[2] as u32) >> 16;
        for c in 0..4 {
            dst[i * 4 + c] = (s[a + c] as u32 * w0 + s[b + c] as u32 * w1) as u16;
        }
    }
}

// bil_vmix(h0, h1, dst, n, weights) -- dst = premultiplied pixels (h0 w0 + h1 w1 + 8192) >> 14
// per channel (weights = w0 | w1 << 16, w0 + w1 = 128)
fn bil_vmix(h0: &[u16], h1: &[u16], dst: &mut [u8], n: usize, w: u32) {
    let w0 = w & 0xFFFF;
    let w1 = w >> 16;
    for i in 0..n * 4 {
        let v = (h0[i] as u32 * w0 + h1[i] as u32 * w1 + 8192) >> 14;
        dst[i] = v.min(255) as u8;
    }
}

// draw_grid() -- Pixel-Raster: 1 px lines between image pixels from 400 % (after the bands,
// inside the render clip)
fn draw_grid(v: &View, g: &mut Gfx, j: &VJob, y1: i32) {
    if !v.grid || j.zoom < GRID_ZOOM {
        return;
    }
    g.dc.set_color(GRID_COLOR);
    let (c0, r0) = view_screen_to_doc(v, j.x0, j.y0);
    let mut c = c0.max(1);
    loop {
        let (sx, _) = view_doc_to_screen(v, c, 0);
        if sx >= j.x1 {
            break;
        }
        if sx >= j.x0 {
            g.dc.fill(g.sf, sx, j.y0, 1, y1 - j.y0);
        }
        c += 1;
    }
    let mut r = r0.max(1);
    loop {
        let (_, sy) = view_doc_to_screen(v, 0, r);
        if sy >= y1 {
            break;
        }
        if sy >= j.y0 {
            g.dc.fill(g.sf, j.x0, sy, j.x1 - j.x0, 1);
        }
        r += 1;
    }
}

// ------------------------------------------------------------------ M7 (settings)

/// view_set_opts(units, grid, rulers) -- restore the Ansicht options (settings.rs): units
/// UNIT_* (ignored when out of range), grid / rulers
pub(crate) fn view_set_opts(p: &mut Pn, units: u32, grid: bool, rulers: bool) {
    if units < 3 {
        p.view.units = units;
    }
    p.view.grid = grid;
    p.view.rulers = rulers;
    view_invalidate(p);
    p.repass();
}
