// t_view -- PaintCraft: Zoomen and Ansicht verschieben (agent T1).  Sub-module of tool.rs.
//   Zoomen                  left click = next zoom preset around the point, right click =
//                           previous one; a left drag draws a rectangle, the release zooms so
//                           that it fills the canvas (centred); middle drag pans (view.rs)
//   Ansicht verschieben  a drag moves the view (view_pan_begin / move / end per frame)
//                                              [apps/PaintCraft/src/t_view.s]
//
//   t_view_init()           register TOOL_ZOOM and TOOL_PAN

use ui::*;

use crate::app::constants::*;
use crate::document::model::DocId;
use crate::tools::*;
use crate::workspace::view::{view_canvas_rect, view_pan_begin, view_pan_end, view_pan_move, view_screen_to_doc_fx, view_zoom_at, view_zoom_step};
use crate::app::Pn;

const DRAG_DIP: i32 = 4; // movement that turns a click into a rectangle

/// the state of t_view.s (its g_* globals)
#[derive(Default)]
pub(crate) struct TView {
    zm_on: bool,
    zm_btn: u32,
    /// press point (screen px)
    zm_sx0: i32,
    zm_sy0: i32,
    /// pointer
    zm_sx1: i32,
    zm_sy1: i32,
    /// rectangle as drawn: visible, x0, y0, x1, y1
    zr_vis: bool,
    zr: [i32; 4],
    /// the accent colour of the rectangle (ui_color at render time in the asm: taken when
    /// the rectangle changes)
    zr_col: u32,
    pn_on: bool,
    pn_btn: u32,
    /// last pointer (screen px)
    pn_lx: i32,
    pn_ly: i32,
}

static DESC_ZOOM: ToolDesc = ToolDesc { deactivate: Some(zm_deactivate), input: Some(zm_input), overlay: Some(zm_overlay), ..ToolDesc::NONE };
static DESC_PAN: ToolDesc = ToolDesc { deactivate: Some(pan_deactivate), input: Some(pan_input), cursor: Some(pan_cursor), ..ToolDesc::NONE };

/// t_view_init() -- register the tool(s)
pub(crate) fn t_view_init(p: &mut Pn, _ui: &mut Ui) {
    tool_register(&mut p.tool, TOOL_ZOOM, Some(&DESC_ZOOM));
    tool_register(&mut p.tool, TOOL_PAN, Some(&DESC_PAN));
}

// zr_damage(): the screen area of the rectangle as drawn
fn zr_damage(p: &Pn, ui: &mut Ui) {
    let t = &p.tview;
    if t.zr_vis {
        tool_damage(&p.view, ui, t.zr[0] - 2, t.zr[1] - 2, t.zr[2] + 3, t.zr[3] + 3);
    }
}

// zr_set(vis): the rectangle from the press point to the pointer (normalised)
fn zr_set(p: &mut Pn, ui: &mut Ui, vis: bool) {
    zr_damage(p, ui);
    let t = &mut p.tview;
    t.zr_col = ui.color(T_ACCENT);
    t.zr_vis = vis;
    t.zr = [t.zm_sx0.min(t.zm_sx1), t.zm_sy0.min(t.zm_sy1), t.zm_sx0.max(t.zm_sx1), t.zm_sy0.max(t.zm_sy1)];
    zr_damage(p, ui);
}

// zm_rect_zoom(): zoom so that the rectangle fills the canvas, centred on it
fn zm_rect_zoom(p: &mut Pn) {
    let zr = p.tview.zr;
    // image size of the rectangle
    let (ax, ay) = view_screen_to_doc_fx(&p.view, zr[0] << 8, zr[1] << 8);
    let (bx, by) = view_screen_to_doc_fx(&p.view, zr[2] << 8, zr[3] << 8);
    let w = bx.wrapping_sub(ax).max(256) as u32 as u64; // width 24.8
    let h = by.wrapping_sub(ay).max(256) as u32 as u64;
    let cr = view_canvas_rect(&p.view);
    // zoom = min(canvas w / w, canvas h / h) * 10000
    let zw = (cr[2] as u32 as u64 * (ZOOM_100 as u64 * 256)) / w;
    let zh = (cr[3] as u32 as u64 * (ZOOM_100 as u64 * 256)) / h;
    let z = zw.min(zh).clamp(ZOOM_MIN as u64, ZOOM_MAX as u64);
    // zoom around the rectangle's centre, then move that point to the canvas centre
    let cx = (zr[0] + zr[2]) >> 1;
    let cy = (zr[1] + zr[3]) >> 1;
    view_zoom_at(p, z as u32, cx, cy);
    view_pan_begin(p, cx, cy);
    let cr = view_canvas_rect(&p.view);
    view_pan_move(p, (cr[2] >> 1) + cr[0], (cr[3] >> 1) + cr[1]);
    view_pan_end(p);
}

// zm_input(doc, ui, x_fx, y_fx) -> changed
fn zm_input(p: &mut Pn, ui: &mut Ui, _doc: DocId, inp: &UiInput, _x: i32, _y: i32) -> bool {
    let sx = inp.x + inp.rect[0];
    let sy = inp.y + inp.rect[1];
    if !p.tview.zm_on {
        let btn = if inp.pressed & (1 << BTN_LEFT) != 0 {
            BTN_LEFT
        } else if inp.pressed & (1 << BTN_RIGHT) != 0 {
            BTN_RIGHT
        } else {
            return false;
        };
        let t = &mut p.tview;
        t.zm_btn = btn;
        t.zm_on = true;
        t.zm_sx0 = inp.press_x + inp.rect[0];
        t.zm_sx1 = t.zm_sx0;
        t.zm_sy0 = inp.press_y + inp.rect[1];
        t.zm_sy1 = t.zm_sy0;
        return true;
    }
    let t = &mut p.tview;
    let moved = sx != t.zm_sx1 || sy != t.zm_sy1;
    t.zm_sx1 = sx;
    t.zm_sy1 = sy;
    let m = 1 << t.zm_btn;
    if inp.btn & m == 0 || inp.released & m != 0 {
        // released
        t.zm_on = false;
        if t.zr_vis {
            zr_set(p, ui, false);
            zm_rect_zoom(p);
        } else {
            let dir = if t.zm_btn == BTN_LEFT { 1 } else { -1 };
            let (x, y) = (t.zm_sx0, t.zm_sy0);
            view_zoom_step(p, dir, x, y);
        }
        return true;
    }
    // a left drag shows the rectangle once it moved a little
    if !moved {
        return false; // nothing moved (another pass of the frame)
    }
    if t.zm_btn != BTN_LEFT {
        return true;
    }
    if !t.zr_vis {
        let (x0, y0) = (t.zm_sx0, t.zm_sy0);
        let d = ui.dp(DRAG_DIP);
        if (sx - x0).abs() <= d && (sy - y0).abs() <= d {
            return true;
        }
    }
    zr_set(p, ui, true);
    true
}

// zm_deactivate(doc): a rectangle in progress disappears
fn zm_deactivate(p: &mut Pn, ui: &mut Ui, _doc: Option<DocId>) {
    p.tview.zm_on = false;
    zr_set(p, ui, false);
}

// zm_overlay(doc, &rect): the zoom rectangle (accent frame over a light accent fill)
fn zm_overlay(p: &mut Pn, g: &mut Gfx, _doc: DocId, _rect: [i32; 4]) {
    let t = &p.tview;
    if !t.zr_vis {
        return;
    }
    let c = t.zr_col;
    let (x, y, w, h) = (t.zr[0], t.zr[1], t.zr[2] - t.zr[0] + 1, t.zr[3] - t.zr[1] + 1);
    g.dc.set_color((c & 0x00FF_FFFF) | 0x3300_0000);
    g.dc.fill(g.sf, x, y, w, h);
    g.dc.set_color(c);
    g.dc.frame(g.sf, x, y, w, h);
}

// pan_input(doc, ui, x_fx, y_fx) -> changed: the view follows the pointer
fn pan_input(p: &mut Pn, _ui: &mut Ui, _doc: DocId, inp: &UiInput, _x: i32, _y: i32) -> bool {
    let sx = inp.x + inp.rect[0];
    let sy = inp.y + inp.rect[1];
    if !p.tview.pn_on {
        let btn = if inp.pressed & (1 << BTN_LEFT) != 0 {
            BTN_LEFT
        } else if inp.pressed & (1 << BTN_RIGHT) != 0 {
            BTN_RIGHT
        } else {
            return false;
        };
        let t = &mut p.tview;
        t.pn_btn = btn;
        t.pn_on = true;
        t.pn_lx = inp.press_x + inp.rect[0];
        t.pn_ly = inp.press_y + inp.rect[1];
    }
    let t = &mut p.tview;
    if inp.btn & (1 << t.pn_btn) == 0 {
        t.pn_on = false;
        return true;
    }
    if sx == t.pn_lx && sy == t.pn_ly {
        // the press itself / held without moving
        return inp.pressed & ((1 << BTN_LEFT) | (1 << BTN_RIGHT)) != 0;
    }
    let (lx, ly) = (t.pn_lx, t.pn_ly);
    view_pan_begin(p, lx, ly);
    view_pan_move(p, sx, sy);
    view_pan_end(p);
    p.tview.pn_lx = sx;
    p.tview.pn_ly = sy;
    true
}

// pan_cursor(x_fx, y_fx) -> CUR_HAND
fn pan_cursor(_p: &mut Pn, _x: i32, _y: i32) -> u32 {
    CUR_HAND
}

// pan_deactivate(doc)
fn pan_deactivate(p: &mut Pn, _ui: &mut Ui, _doc: Option<DocId>) {
    p.tview.pn_on = false;
}
