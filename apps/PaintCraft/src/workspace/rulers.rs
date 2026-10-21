// rulers -- PaintCraft: the horizontal and vertical ruler (paint.net: Ansicht > Lineale) in
// Pixel, Zoll or Zentimeter from the image resolution.  Called by view.rs with an RP record
// holding the band, the transform and the unit.
//                                                                 [apps/PaintCraft/src/rulers.s]
//
//   ruler_draw(rp)      band background, the image extent, ticks with numbers at the major
//                       ticks, the pointer marker
//
// Major steps are 1, 2, 5, 10, 20, 50 ... units at least 48 px apart; each is divided into
// the largest of 10 / 5 / 2 (Pixel, Zentimeter) or 16 / 8 / 4 / 2 (Zoll) parts that stay at
// least 5 px apart.  Tick length: major = band, half step = 1/2, others = 1/4.

use alloc::format;

use rt::fx::{cvtd, cvttd};
use ui::*;

use crate::app::constants::*;
use crate::app::formatting::fmt_cap;

const LABEL_MIN_PX: f64 = 48.0;
const TICK_MIN_PX: f64 = 5.0;
const NUMBUF_CAP: usize = 24;
static R_STEPS: [i32; 16] = [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000, 50000, 0];
static R_PARTS_PX: [u32; 4] = [10, 5, 2, 1];
static R_PARTS_IN: [u32; 5] = [16, 8, 4, 2, 1];
static R_PARTS_CM: [u32; 4] = [10, 5, 2, 1];

/// ruler parameters (RP record of pn_doc.inc)
#[derive(Clone, Copy, Default)]
pub(crate) struct RulerParams {
    /// RP_X / RP_Y / RP_LEN: band rectangle (px): x, y, length along the ruler
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) len: i32,
    /// RP_THICK: band thickness (px)
    pub(crate) thick: i32,
    /// RP_VERT: vertical ruler
    pub(crate) vert: bool,
    /// RP_ORIGIN: screen position of image coordinate 0
    pub(crate) origin: i32,
    /// RP_ZOOM: zoom in 1/100 %
    pub(crate) zoom: u32,
    /// RP_UNITS: UNIT_*
    pub(crate) units: u32,
    /// RP_IMGLEN: image length on screen (px)
    pub(crate) imglen: i32,
    /// RP_CUR: pointer position (px) or -100000
    pub(crate) cur: i32,
    /// RP_PPU: image pixels per unit
    pub(crate) ppu: f64,
}

// band_rect(rp, x0, x1) -- draw the band part x0..x1 (along the ruler) with the draw colour
fn band_rect(ui: &mut Ui, rp: &RulerParams, a: i32, b: i32) {
    if a >= b {
        return;
    }
    if rp.vert {
        ui.draw_rect(rp.x, a, rp.thick, b - a);
    } else {
        ui.draw_rect(a, rp.y, b - a, rp.thick);
    }
}

// rflr(x) -> floor
fn rflr(x: f64) -> i32 {
    let t = cvttd(x);
    if (t as f64) > x { t.wrapping_sub(1) } else { t }
}

/// ruler_draw(rp)
pub(crate) fn ruler_draw(ui: &mut Ui, rp: &RulerParams) {
    let start = if rp.vert { rp.y } else { rp.x };
    let end = start.wrapping_add(rp.len);
    // background, the image extent a little lighter
    let c = ui.color(T_BG);
    ui.draw_color(c);
    band_rect(ui, rp, start, end);
    let c = ui.color(T_CARD);
    ui.draw_color(c);
    let a = rp.origin.max(start);
    let b = rp.origin.wrapping_add(rp.imglen).min(end);
    band_rect(ui, rp, a, b);
    // hairline along the inner edge
    let hair = ui.hair();
    let c = ui.color(T_DIVIDER);
    ui.draw_color(c);
    if rp.vert {
        ui.draw_rect(rp.x + rp.thick - hair, rp.y, hair, rp.len);
        ui.clip_push(rp.x, rp.y, rp.thick, rp.len);
    } else {
        ui.draw_rect(rp.x, rp.y + rp.thick - hair, rp.len, hair);
        ui.clip_push(rp.x, rp.y, rp.len, rp.thick);
    }
    // screen px per unit
    let sppu = rp.zoom as i32 as f64 / 10000.0 * rp.ppu;
    if sppu > 0.0 {
        ticks(ui, rp, sppu, start, end, hair);
    }
    // the pointer position
    if rp.cur >= start && rp.cur < end {
        let c = ui.color(T_ACCENT);
        ui.draw_color(c);
        if rp.vert {
            ui.draw_rect(rp.x, rp.cur, rp.thick, hair);
        } else {
            ui.draw_rect(rp.cur, rp.y, hair, rp.thick);
        }
    }
    ui.clip_pop();
}

// the ticks and labels of ruler_draw
fn ticks(ui: &mut Ui, rp: &RulerParams, sppu: f64, start: i32, end: i32, hair: i32) {
    // major step: the first of 1, 2, 5, 10 ... with step * sppu >= LABEL_MIN_PX
    let mut si = 0;
    let step = loop {
        let s = R_STEPS[si];
        if R_STEPS[si + 1] == 0 || s as f64 * sppu >= LABEL_MIN_PX {
            break s;
        }
        si += 1;
    };
    // parts: the first of the unit's list that keeps minor ticks TICK_MIN_PX apart (and
    // divides the step for Pixel)
    let list: &[u32] = if rp.units == UNIT_IN as u32 {
        &R_PARTS_IN
    } else if rp.units == UNIT_CM as u32 {
        &R_PARTS_CM
    } else {
        &R_PARTS_PX
    };
    let mut parts = 1u32;
    for &c in list {
        parts = c;
        if c <= 1 {
            break;
        }
        if rp.units == UNIT_PX as u32 && step as u32 % c != 0 {
            continue;
        }
        if !(step as f64 * sppu / c as f64 >= TICK_MIN_PX) {
            continue;
        }
        break;
    }
    // minor step in px
    let minor = step as f64 * sppu / parts as f64;
    // k from floor((start - origin) / minor) to (end - origin) / minor + 1
    let mut k = rflr(start.wrapping_sub(rp.origin) as f64 / minor);
    let mut kend = rflr(end.wrapping_sub(rp.origin) as f64 / minor).wrapping_add(2);
    if kend > k + 4000 {
        kend = k + 4000; // never more than 4000 ticks
    }
    let font = ((ui.dp(9) as u32) << 16) | FONT_REG;
    let toff = ui.dp(3);
    let base = ui.dp(10);
    while k <= kend {
        // pos = origin + round(k * minor)
        let pos = cvtd(k as f64 * minor).wrapping_add(rp.origin);
        // j = k mod parts (>= 0)
        let pi = parts as i32;
        let mut q = k / pi;
        let mut j = k % pi;
        if j < 0 {
            j += pi;
            q -= 1; // floor division for the label
        }
        let mut len = rp.thick;
        if j != 0 {
            len = (rp.thick as u32 >> 2) as i32;
            if parts & 1 == 0 && j == (parts >> 1) as i32 {
                len = (rp.thick as u32 >> 1) as i32;
            }
        }
        let c = ui.color(T_TEXT3);
        ui.draw_color(c);
        if rp.vert {
            ui.draw_rect(rp.x + rp.thick - len, pos, len, hair);
        } else {
            ui.draw_rect(pos, rp.y + rp.thick - len, hair, len);
        }
        if len == rp.thick {
            // label: major index * step
            let s = fmt_cap(format!("{}", q.wrapping_mul(step)), NUMBUF_CAP);
            let c = ui.color(T_TEXT2);
            ui.draw_color(c);
            if rp.vert {
                // vertical ruler: the characters stacked below the tick (one per line)
                let mut by = pos + base;
                for ch in s.chars() {
                    let mut b = [0u8; 4];
                    ui.draw_text(rp.x + toff, by, ch.encode_utf8(&mut b), font);
                    by += ui.dp(9);
                }
            } else {
                ui.draw_text(pos + toff, rp.y + base, &s, font);
            }
        }
        k += 1;
    }
}
