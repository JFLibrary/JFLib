// tools -- painting tools driven by the canvas input  [apps/paint/src/tools.s]
//
//   tool_input(p, ui, inp)          handle one frame of UIINPUT from the canvas
//   tool_set(t, tool)               select a tool (remembers the previous one)
//   tool_abort(p)                   end a running freehand stroke without recording it
//   flood_fill(sf, x, y, pargb) -> true if pixels changed
//   premul(argb) -> premultiplied
//
// State for other modules (the asm's globals): tool, size (px), col1 / col2 (straight ARGB
// primary / secondary), opacity (0..100), cursor_doc (last pointer position in doc px),
// brush, canvas_ev.
//
// Drawing goes through the UI's draw context, path and rasterizer (ui.dc, ui.text.path,
// ui.text.ras): the asm's global g_dc / path state.

use alloc::vec::Vec;

use gfx::{Surface, CAP_ROUND, JOIN_ROUND};
use ui::*;

use crate::brushes::{brush_abort, brush_input};
use crate::consts::*;
use crate::doc::{doc_cur, doc_h, doc_mark, doc_mark_all, doc_pick, doc_w};
use crate::history::{hist_begin, hist_cancel, hist_end};
use crate::select::select_input;
use crate::shapes::shape_input;
use crate::textool::text_input;
use crate::view::{view_scroll, view_to_doc, view_to_doc_fx, view_zoom_step};
use crate::Kritzel;

const BTN_L_MASK: u32 = 1 << BTN_LEFT;
const BTN_R_MASK: u32 = 1 << BTN_RIGHT;

/// the tool state (the .data / .bss of tools.s)
pub(crate) struct Tools {
    pub(crate) tool: i32,
    prev_tool: i32,
    pub(crate) brush: i32,
    pub(crate) size: i32,
    pub(crate) opacity: i32,
    pub(crate) col1: u32,
    pub(crate) col2: u32,
    pub(crate) cursor_doc: [i32; 2],
    pub(crate) canvas_ev: u32,
    stroking: bool,
    stroke_btn: u32,
    last_fx: i32,
    last_fy: i32,
    sbox: [i32; 4],
}

impl Default for Tools {
    fn default() -> Tools {
        Tools {
            tool: TOOL_BRUSH,
            prev_tool: TOOL_BRUSH,
            brush: B_BRUSH,
            size: 8,
            opacity: 100,
            col1: 0xFF00_0000,
            col2: 0xFFFF_FFFF,
            cursor_doc: [0; 2],
            canvas_ev: 0,
            stroking: false,
            stroke_btn: 0,
            last_fx: 0,
            last_fy: 0,
            sbox: [0; 4],
        }
    }
}

/// tool_abort() -- end a running freehand stroke (pencil / eraser / brush) without
/// recording it: the document changed under it (undo / redo, size change, new document)
pub(crate) fn tool_abort(p: &mut Kritzel) {
    if p.tools.stroking {
        p.tools.stroking = false;
        hist_cancel(p);
    }
    brush_abort(p);
}

/// tool_set(t)
pub(crate) fn tool_set(t: &mut Tools, tool: i32) {
    if t.tool == tool {
        return;
    }
    if t.tool != TOOL_PICKER {
        t.prev_tool = t.tool;
    }
    t.tool = tool;
}

// stroke_color() -> straight colour for the active button (eraser: secondary)
fn stroke_color(t: &Tools) -> u32 {
    let mut c = t.col1;
    if t.stroke_btn == BTN_RIGHT {
        c = t.col2;
    }
    if t.tool == TOOL_ERASER {
        return t.col2;
    }
    // opacity slider scales alpha (not for the eraser)
    let a = (c >> 24).wrapping_mul(t.opacity as u32) / 100;
    (a << 24) | (c & 0x00FF_FFFF)
}

// bbox_add(x0, y0, x1, y1) -- grow the stroke's dirty box
fn bbox_add(t: &mut Tools, x0: i32, y0: i32, x1: i32, y1: i32) {
    let b = &mut t.sbox;
    b[0] = b[0].min(x0);
    b[1] = b[1].min(y0);
    b[2] = b[2].max(x1);
    b[3] = b[3].max(y1);
}

// segment(x0, y0, x1, y1) -- draw one stroke segment (doc 24.8) into the current layer
fn segment(p: &mut Kritzel, ui: &mut Ui, x0: i32, y0: i32, x1: i32, y1: i32) {
    let size = p.tools.size;
    let tool = p.tools.tool;
    let col = stroke_color(&p.tools);
    let erase = tool == TOOL_ERASER && p.doc.cur_layer != 0;
    let sf = doc_cur(&mut p.doc);
    ui.dc.begin(sf);
    ui.dc.set_color(col);
    if tool != TOOL_BRUSH {
        // pencil / eraser: aliased squares of the size along the line
        square_line(ui, sf, size, erase, x0 >> 8, y0 >> 8, x1 >> 8, y1 >> 8);
    } else {
        let path = &mut ui.text.path;
        path.begin(&ui.dc);
        path.move_fx(x0, y0);
        if x1 != x0 || y1 != y0 {
            path.line_fx(x1, y1); // else a dot: single point with round caps
        }
        path.stroke(&mut ui.text.ras, &ui.dc, sf, size << 8, CAP_ROUND << 4 | JOIN_ROUND);
    }
    ui.dc.end();
    // dirty box: segment bounds +- size
    let e = size + 2;
    let bx0 = (x0.min(x1) >> 8) - e;
    let by0 = (y0.min(y1) >> 8) - e;
    let bx1 = (x0.max(x1) >> 8) + e;
    let by1 = (y0.max(y1) >> 8) + e;
    doc_mark(&mut p.doc, bx0, by0, bx1, by1);
    bbox_add(&mut p.tools, bx0, by0, bx1, by1);
}

// square_line(x0, y0, x1, y1) -- Bresenham, stamping a size x size square (current target
// and colour); the eraser on upper layers clears instead
#[allow(clippy::too_many_arguments)]
fn square_line(ui: &mut Ui, sf: &mut Surface, size: i32, erase: bool, x0: i32, y0: i32, x1: i32, y1: i32) {
    let dx = x1.wrapping_sub(x0).wrapping_abs();
    let dy = -y1.wrapping_sub(y0).wrapping_abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx.wrapping_add(dy);
    let (mut x, mut y) = (x0, y0);
    let half = (size as u32 >> 1) as i32;
    loop {
        // square centred at (x, y)
        let (px, py) = (x.wrapping_sub(half), y.wrapping_sub(half));
        if erase {
            // erase to transparent on upper layers
            ui.dc.clip_push(px, py, size, size);
            ui.dc.clear(sf, 0);
            ui.dc.clip_pop();
        } else {
            ui.dc.fill(sf, px, py, size, size);
        }
        if x == x1 && y == y1 {
            break;
        }
        let e2 = err.wrapping_add(err);
        if e2 >= dy {
            err = err.wrapping_add(dy);
            x = x.wrapping_add(sx);
        }
        if e2 <= dx {
            err = err.wrapping_add(dx);
            y = y.wrapping_add(sy);
        }
    }
}

/// tool_input(in) -- one frame of canvas input (UIINPUT)
pub(crate) fn tool_input(p: &mut Kritzel, ui: &mut Ui, inp: &UiInput) {
    let ax = inp.x.wrapping_add(inp.rect[0]);
    let ay = inp.y.wrapping_add(inp.rect[1]);
    // remember the pointer in doc px (status bar)
    let (dx, dy) = view_to_doc(&p.view, ax, ay);
    p.tools.cursor_doc = [dx, dy];
    p.tools.canvas_ev = inp.ev;
    // wheel: ctrl zooms, shift scrolls sideways, else scrolls
    let wy = inp.wheel_y;
    if wy != 0 {
        if inp.mods & MOD_CTRL != 0 {
            view_zoom_step(p, ui, if wy > 0 { 1 } else { -1 }, ax, ay);
        } else {
            let d = ui.dp(48).wrapping_mul(wy.wrapping_neg()) / 120;
            if inp.mods & MOD_SHIFT != 0 {
                view_scroll(&mut p.view, d, 0);
            } else {
                view_scroll(&mut p.view, 0, d);
            }
        }
    }
    // tool modules take their own input first (shapes, selection, text, brushes)
    match p.tools.tool {
        TOOL_SHAPE => {
            shape_input(p, ui, inp);
            return;
        }
        TOOL_SELECT => {
            select_input(p, ui, inp);
            return;
        }
        TOOL_TEXT => {
            text_input(p, ui, inp);
            return;
        }
        TOOL_BRUSH => {
            if brush_input(p, ui, inp) {
                return;
            }
        }
        _ => {}
    }
    let pressed = inp.pressed;
    let press_ok = pressed & (BTN_L_MASK | BTN_R_MASK) != 0 && inp.ev & EV_HOVER != 0;
    match p.tools.tool {
        TOOL_FILL => {
            if !press_ok {
                return;
            }
            p.tools.stroke_btn = if pressed & BTN_L_MASK != 0 { BTN_LEFT } else { BTN_RIGHT };
            let cur = p.doc.cur_layer;
            hist_begin(p, cur);
            let c = premul(stroke_color(&p.tools));
            let [x, y] = p.tools.cursor_doc;
            if flood_fill(doc_cur(&mut p.doc), x, y, c) {
                doc_mark_all(&mut p.doc);
                let (w, h) = (doc_w(&p.doc), doc_h(&p.doc));
                hist_end(p, 0, 0, w, h);
            } else {
                hist_cancel(p);
            }
        }
        TOOL_PICKER => {
            if !press_ok {
                return;
            }
            let [x, y] = p.tools.cursor_doc;
            let c = doc_pick(&mut p.doc, x, y);
            if c & 0xFF00_0000 == 0 {
                return;
            }
            if pressed & BTN_L_MASK != 0 {
                p.tools.col1 = c;
            } else {
                p.tools.col2 = c;
            }
            p.tools.tool = p.tools.prev_tool;
        }
        TOOL_ZOOM => {
            if !press_ok {
                return;
            }
            let dir = if pressed & BTN_L_MASK != 0 { 1 } else { -1 };
            view_zoom_step(p, ui, dir, ax, ay);
        }
        TOOL_PENCIL | TOOL_BRUSH | TOOL_ERASER => stroke_input(p, ui, inp),
        _ => {}
    }
}

// ---- freehand strokes
fn stroke_input(p: &mut Kritzel, ui: &mut Ui, inp: &UiInput) {
    if !p.tools.stroking {
        // press starts a stroke (left or right)
        let pressed = inp.pressed;
        if pressed & (BTN_L_MASK | BTN_R_MASK) == 0 || inp.ev & EV_HOVER == 0 {
            return;
        }
        let t = &mut p.tools;
        t.stroke_btn = if pressed & BTN_L_MASK != 0 { BTN_LEFT } else { BTN_RIGHT };
        t.stroking = true;
        t.sbox = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
        let cur = p.doc.cur_layer;
        hist_begin(p, cur);
        // dot at the press position
        let (fx, fy) =
            view_to_doc_fx(&p.view, inp.press_x.wrapping_add(inp.rect[0]), inp.press_y.wrapping_add(inp.rect[1]));
        p.tools.last_fx = fx;
        p.tools.last_fy = fy;
        segment(p, ui, fx, fy, fx, fy);
    }
    // every recorded move point continues the stroke
    for pt in inp.pts.iter() {
        let (x, y) = view_to_doc_fx(&p.view, pt[0].wrapping_add(inp.rect[0]), pt[1].wrapping_add(inp.rect[1]));
        if x == p.tools.last_fx && y == p.tools.last_fy {
            continue;
        }
        let (lx, ly) = (p.tools.last_fx, p.tools.last_fy);
        segment(p, ui, lx, ly, x, y);
        p.tools.last_fx = x;
        p.tools.last_fy = y;
    }
    // released: end the stroke
    if inp.btn & (1 << p.tools.stroke_btn) != 0 {
        return;
    }
    p.tools.stroking = false;
    let b = p.tools.sbox;
    hist_end(p, b[0], b[1], b[2], b[3]);
}

/// premul(argb) -> premultiplied
pub(crate) fn premul(argb: u32) -> u32 {
    let a = argb >> 24;
    if a == 255 {
        return argb;
    }
    let mut r = argb & 0xFF00_0000;
    for ch in 0..3 {
        let sh = ch * 8;
        let v = ((argb >> sh) & 255) * a + 127;
        r |= (v.wrapping_mul(0x8081) >> 23) << sh; // x / 255 = x * 0x8081 >> 23
    }
    r
}

#[inline(always)]
fn px4(c: &[u8]) -> u32 {
    u32::from_le_bytes([c[0], c[1], c[2], c[3]])
}

/// flood_fill(sf, x, y, pargb) -> true if anything changed
/// Scanline fill of the 4-connected region that has exactly the colour at (x, y).
pub(crate) fn flood_fill(sf: &mut Surface, x: i32, y: i32, pargb: u32) -> bool {
    if x as u32 >= sf.w as u32 || y as u32 >= sf.h as u32 {
        return false;
    }
    let stride = sf.stride as usize;
    let w = sf.w as usize;
    let h = sf.h as usize;
    let px = &mut sf.px[..];
    let target = px4(&px[y as usize * stride + x as usize * 4..]);
    if target == pargb {
        return false; // already that colour
    }
    let fill = pargb.to_le_bytes();
    // stack of (x, y) seeds
    const CAP: usize = 1 << 20;
    let mut stack: Vec<(u32, u32)> = Vec::with_capacity(4096);
    stack.push((x as u32, y as u32));
    while let Some((sx, sy)) = stack.pop() {
        let (sx, sy) = (sx as usize, sy as usize);
        let row = &mut px[sy * stride..sy * stride + w * 4];
        if px4(&row[sx * 4..sx * 4 + 4]) != target {
            continue;
        }
        // extend left and right
        let l = sx - row[..sx * 4].rchunks_exact(4).take_while(|c| px4(c) == target).count();
        let r = sx + row[(sx + 1) * 4..].chunks_exact(4).take_while(|c| px4(c) == target).count();
        // paint [l, r]
        for c in row[l * 4..(r + 1) * 4].chunks_exact_mut(4) {
            c.copy_from_slice(&fill);
        }
        // seed the rows above and below (one seed per run of target pixels)
        for ny in [sy.wrapping_sub(1), sy + 1] {
            if ny >= h {
                continue; // also catches -1 (unsigned)
            }
            let nrow = &px[ny * stride + l * 4..ny * stride + (r + 1) * 4];
            let mut inrun = false;
            for (k, c) in nrow.chunks_exact(4).enumerate() {
                if px4(c) != target {
                    inrun = false;
                } else if !inrun {
                    inrun = true;
                    // push (x, y) if there is room
                    if stack.len() < CAP {
                        stack.push(((l + k) as u32, ny as u32));
                    }
                }
            }
        }
    }
    true
}
