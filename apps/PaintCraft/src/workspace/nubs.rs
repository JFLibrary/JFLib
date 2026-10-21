// nubs -- PaintCraft: handles ("nubs") of live tools, reusable by the move / shape / text tools
// (contract pn_tooli.inc).  A NUB lives in image coordinates (24.8); it is drawn in screen
// space at a fixed size (paint.net 5: white disc, dark ring; NK_SQUARE resize handles as white
// squares with a dark outline; the hovered / dragged one in the accent colour).  Every change
// damages the screen area where the nub was drawn and where it is now (tool_damage), so
// incremental rendering stays exact.
//                                                                   [apps/PaintCraft/src/nubs.s]
//
//   nub_set(nub, x_fx, y_fx)                  move (damaged)
//   nubs_draw(nubs)                           overlay time: the visible nubs
//   nubs_hit(nubs, sx, sy) -> index | -1      screen point (absolute px)
//   nubs_hover(nubs, sx, sy) -> true when a hover state changed (damaged)
//   nubs_damage(nubs)                         redraw them (old and new place)
//   nubs_frame(nubs, x0_fx, y0_fx, x1_fx, y1_fx)   8 frame nubs (NF_* order)
//   nubs_frame_draw(nubs)                     the frame rectangle + its 8 nubs
//
// The functions take the view (the transforms, the dpi and accent of the frame) and the UI
// (damage) or, at render time, the gfx target; the nubs are a slice of the tool's state.

use ui::*;

use crate::app::constants::*;
use crate::tools::tool_damage;
use crate::workspace::view::{view_doc_to_screen_fx, View};

const NUB_DIP: i32 = 6; // radius (DIP)
const NUB_SLACK: i32 = 3; // extra hit radius (DIP)
const NUB_PAD: i32 = 3; // damage margin (px)
static NF_POS: [[u8; 2]; 8] = [[0, 0], [1, 0], [2, 0], [2, 1], [2, 2], [1, 2], [0, 2], [0, 1]];

/// NUB (pn_tooli.inc)
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Nub {
    /// NB_X / NB_Y: image position, 24.8
    pub(crate) x: i32,
    pub(crate) y: i32,
    /// NB_KIND: NK_*
    pub(crate) kind: u32,
    /// NB_FLAGS: NBF_*
    pub(crate) flags: u32,
    /// NB_SX / NB_SY: screen position where it was drawn last (internal)
    pub(crate) sx: i32,
    pub(crate) sy: i32,
}

// nub_screen(nub) -> (sx, sy): screen position (px) of the nub's centre
fn nub_screen(v: &View, n: &Nub) -> (i32, i32) {
    let (x, y) = view_doc_to_screen_fx(v, n.x, n.y);
    (x >> 8, y >> 8)
}

// nub_rad() -> radius in px
fn nub_rad(v: &View) -> i32 {
    dp_at(NUB_DIP, v.dpi)
}

// nub_damage_at(sx, sy) -- the screen area of a nub centred there
fn nub_damage_at(v: &View, ui: &mut Ui, sx: i32, sy: i32) {
    let r = nub_rad(v) + NUB_PAD;
    tool_damage(v, ui, sx - r, sy - r, sx + r + 1, sy + r + 1);
}

// nub_damage(nub) -- where it was drawn and where it is now
fn nub_damage(v: &View, ui: &mut Ui, n: &Nub) {
    if n.flags & NBF_DRAWN as u32 != 0 {
        nub_damage_at(v, ui, n.sx, n.sy);
    }
    let (sx, sy) = nub_screen(v, n);
    nub_damage_at(v, ui, sx, sy);
}

/// nubs_damage(nubs)
pub(crate) fn nubs_damage(v: &View, ui: &mut Ui, nubs: &mut [Nub]) {
    for n in nubs.iter() {
        nub_damage(v, ui, n);
    }
}

/// nub_set(nub, x_fx, y_fx) -- move a nub (damaged before and after)
pub(crate) fn nub_set(v: &View, ui: &mut Ui, nub: &mut Nub, x_fx: i32, y_fx: i32) {
    if x_fx == nub.x && y_fx == nub.y {
        return;
    }
    nub_damage(v, ui, nub);
    nub.x = x_fx;
    nub.y = y_fx;
    nub_damage(v, ui, nub);
}

/// nubs_hit(nubs, sx, sy) -> index of the (last drawn = topmost) nub under the point, -1 = none
pub(crate) fn nubs_hit(v: &View, _ui: &Ui, nubs: &[Nub], sx: i32, sy: i32) -> i32 {
    let r = nub_rad(v) + NUB_SLACK;
    let r2 = r.wrapping_mul(r);
    for (i, n) in nubs.iter().enumerate().rev() {
        if n.flags & NBF_HIDDEN as u32 != 0 {
            continue;
        }
        let (x, y) = nub_screen(v, n);
        let dx = x.wrapping_sub(sx);
        let dy = y.wrapping_sub(sy);
        if dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy)) <= r2 {
            return i as i32;
        }
    }
    -1
}

/// nubs_hover(nubs, sx, sy) -> true when the hovered nub changed
pub(crate) fn nubs_hover(v: &View, ui: &mut Ui, nubs: &mut [Nub], sx: i32, sy: i32) -> bool {
    let h = nubs_hit(v, ui, nubs, sx, sy);
    let mut changed = false;
    for (i, n) in nubs.iter_mut().enumerate() {
        let mut f = n.flags & !(NBF_HOVER as u32);
        if i as i32 == h {
            f |= NBF_HOVER as u32;
        }
        if f != n.flags {
            n.flags = f;
            nub_damage(v, ui, n);
            changed = true;
        }
    }
    changed
}

/// nubs_draw(nubs) -- overlay time: every visible nub; remembers where it was drawn
pub(crate) fn nubs_draw(v: &View, g: &mut Gfx, nubs: &mut [Nub]) {
    let r = nub_rad(v) << 8; // radius 24.8
    for n in nubs.iter_mut() {
        if n.flags & NBF_HIDDEN as u32 != 0 {
            continue;
        }
        let (sx, sy) = nub_screen(v, n);
        n.sx = sx;
        n.sy = sy;
        n.flags |= NBF_DRAWN as u32;
        let cx = (sx << 8) + 128;
        let cy = (sy << 8) + 128;
        let lit = n.flags & (NBF_HOVER | NBF_DRAG) as u32 != 0;
        if n.kind == NK_SQUARE as u32 {
            // square (resize) handle: shadow, fill, 1 px outline (U3)
            nub_square(v, g, sx, sy, r >> 8, lit);
            continue;
        }
        // shadow ring, fill, outline
        g.dc.set_color(0x4000_0000);
        g.shapes().fill_circle_fx(cx, cy, r + 256);
        g.dc.set_color(if lit { v.accent } else { 0xFFFF_FFFF });
        g.shapes().fill_circle_fx(cx, cy, r);
        g.dc.set_color(0xFF30_3030);
        g.shapes().stroke_circle_fx(cx, cy, r - 64, 192);
    }
}

// nub_square(sx, sy, r, nub) -- a square handle centred on pixel (sx, sy), half side r - 1 (as
// large as the disc of radius r looks): 1 px shadow, white / accent fill, dark outline
fn nub_square(v: &View, g: &mut Gfx, sx: i32, sy: i32, r: i32, lit: bool) {
    let h = r - 1; // half side
    g.dc.set_color(0x4000_0000);
    g.dc.frame(g.sf, sx - h - 1, sy - h - 1, h * 2 + 3, h * 2 + 3);
    g.dc.set_color(if lit { v.accent } else { 0xFFFF_FFFF });
    g.dc.fill(g.sf, sx - h, sy - h, h * 2 + 1, h * 2 + 1);
    g.dc.set_color(0xFF30_3030);
    g.dc.frame(g.sf, sx - h, sy - h, h * 2 + 1, h * 2 + 1);
}

/// nubs_frame(nubs, x0_fx, y0_fx, x1_fx, y1_fx) -- 8 square nubs on a rectangle (NF_* order)
pub(crate) fn nubs_frame(v: &View, ui: &mut Ui, nubs: &mut [Nub], x0: i32, y0: i32, x1: i32, y1: i32) {
    // nf_coord(which, a, b) -> a / (a + b) / 2 / b
    let co = |w: u8, a: i32, b: i32| -> i32 {
        match w {
            0 => a,
            2 => b,
            _ => a.wrapping_add(b) >> 1,
        }
    };
    for (k, n) in nubs.iter_mut().enumerate().take(NF_COUNT as usize) {
        let x = co(NF_POS[k][0], x0, x1);
        let y = co(NF_POS[k][1], y0, y1);
        n.kind = NK_SQUARE as u32;
        nub_set(v, ui, n, x, y);
    }
}

/// nubs_frame_draw(nubs) -- the rectangle through the corner nubs, then the 8 nubs
pub(crate) fn nubs_frame_draw(v: &View, g: &mut Gfx, nubs: &mut [Nub]) {
    let tl = nubs[NF_TL as usize];
    let br = nubs[NF_BR as usize];
    let (x, y) = view_doc_to_screen_fx(v, tl.x, tl.y);
    let (x, y) = (x >> 8, y >> 8);
    let (x2, y2) = view_doc_to_screen_fx(v, br.x, br.y);
    let w = (x2 >> 8) - x;
    let h = (y2 >> 8) - y;
    g.dc.set_color(0xC0FF_FFFF);
    g.dc.frame(g.sf, x - 1, y - 1, w + 2, h + 2);
    g.dc.set_color(0xFF30_3030);
    g.dc.frame(g.sf, x, y, w, h);
    nubs_draw(v, g, &mut nubs[..NF_COUNT as usize]);
}
