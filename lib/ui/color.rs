// color -- colour swatches, the colour picker and the HSV colour wheel
//                                                     [src/ui/ui_color.s, src/ui/ui_wheel.s]
//
//   swatch(id, argb, selected) -> EV        round colour well (22 DIP or next_size); alpha 0
//        draws an empty slot; EV_ACT on click, EV_RCLICK on right click
//   color_picker(id, &mut argb) -> EV       spectrum (saturation x value), hue slider, preview
//        and hex text; EV_ACT while the colour changes, EV_COMMIT on release.  Uses the ids
//        id + 0x20000000 (spectrum) and id + 0x10000000 (hue) for its parts.
//   color_picker_ex(id, &mut argb, flags) -> EV   (U1) the same with CP_ALPHA: an alpha bar
//        (id + 0x30000000, checkerboard) under the hue bar; CP_FIELDS: number boxes R G B (A)
//        and H S V (part_id(id, 3..9)) and a hex text box (part_id(id, 10); RRGGBB,
//        AARRGGBB with CP_ALPHA, taken on Enter / focus loss) instead of the hex text;
//        CP_PIPETTE: a pipette button beside the preview (part_id(id, 11); only where the
//        user can pick a colour anywhere on the screen, screen.rs): the picked colour (alpha
//        kept) comes with EV_ACT | EV_COMMIT on the picker's next pass.
//   color_wheel(id, &mut color, flags) -> EV  (U1) hue around the circle (0 = red at the
//        right, counter-clockwise), saturation from the centre to the rim; the value is not
//        part of the wheel (a separate track slider) but the wheel is drawn at that value.
//        color: [0xAARRGGBB] (alpha kept; the HSV state lives in a state slot so greys keep
//        their hue), or with CW_HSV an HSV record [h 0..3599, s 0..1000, v 0..1000].  Drag
//        sets hue and saturation; keys: Left / Right hue -/+ 1 degree, Up / Down saturation
//        +/- 1 % (Shift: x10).  EV_ACT while it changes, EV_COMMIT on release / keys.
//        Natural size 160 DIP.  Semantic record: K_SLIDER, value = hue, max = saturation.
//   hsv_to_rgb(h, s, v) -> 0xFFRRGGBB       h 0..3599 (tenths of a degree), s / v 0..1000
//   rgb_to_hsv(rgb) -> (h, s, v)            inverse (same ranges)
//
// The wheel image is cached per widget (WC_MAX entries, shared by all windows): a
// premultiplied image at full value for the current size (antialiased rim), and the image
// scaled to the current value; only a change of size or value re-renders it.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;

use super::cmd::{Draw, Gfx};
use super::consts::*;
use super::keys::*;
use super::value::{deg_atan2, f_round, part_id, px_mix, Track};
use super::{Sp, Ui};
use gfx::{Surface, FMT_PARGB};
use rt::fx::{cvtd, minsd, sqrtd};

const PART_SPEC: u32 = 0x2000_0000;
const PART_HUE: u32 = 0x1000_0000;
const PART_ALPHA: u32 = 0x3000_0000;
const WHEEL_NAT: i32 = 160; // natural size of the wheel (DIP)
const WC_MAX: usize = 4; // cached wheels

const RG_BYTE: Range = Range::new(0, 255, 1, 0);
const RG_HUE: Range = Range::new(0, 359, 1, 0);
const RG_PCT: Range = Range::new(0, 100, 1, 0);
const FLD_LEFT: [&str; 4] = ["R", "G", "B", "A"];
const FLD_RIGHT: [&str; 4] = ["H", "S", "V", "Hex"];

// the field store of a picker with CP_FIELDS (the asm keeps it in the slot of
// part_id(id, 15)): R G B A, H S V, the hex text
#[derive(Default)]
struct Fields {
    rgba: [i32; 4],
    hsv: [i32; 3],
    hex: String,
}

// a cached wheel  [WC_*]
#[derive(Default)]
struct Wc {
    id: u32,
    /// side in px of the cached images (0 = none)
    size: i32,
    /// value of the display image (-1 = stale)
    v: i32,
    seen: u32,
    ver: u32,
    base: Surface,
    disp: Rc<Surface>,
}

/// state shared by all windows
#[derive(Default)]
pub(crate) struct S {
    fields: BTreeMap<u32, Fields>,
    wc: [Wc; WC_MAX],
    wc_ver: u32,
    spec: Option<Rc<SpectrumDraw>>,
    hue: Option<Rc<HueDraw>>,
}

// ------------------------------------------------------------------ HSV

/// hsv_to_rgb(h, s, v) -> 0xFFRRGGBB   (h 0..3599, s / v 0..1000)
pub fn hsv_to_rgb(h: u32, s: u32, v: u32) -> u32 {
    // V = v * 255 / 1000
    let vv = v.wrapping_mul(255).wrapping_add(500) / 1000;
    if s == 0 {
        return vv.wrapping_mul(0x01_0101) | 0xFF00_0000;
    }
    // sector and fraction
    let mut sec = h / 600;
    let f = h % 600;
    if sec > 5 {
        sec = 0;
    }
    // p = V (1000 - s) / 1000, q = V (600000 - s f) / 600000, t = V (600000 - s (600 - f)) / 600000
    let p = 1000u32.wrapping_sub(s).wrapping_mul(vv).wrapping_add(500) / 1000;
    let q = 600_000u32.wrapping_sub(s.wrapping_mul(f)).wrapping_mul(vv).wrapping_add(300_000) / 600_000;
    let t = 600_000u32.wrapping_sub(600u32.wrapping_sub(f).wrapping_mul(s)).wrapping_mul(vv).wrapping_add(300_000) / 600_000;
    let (r, g, b) = match sec {
        0 => (vv, t, p),
        1 => (q, vv, p),
        2 => (p, vv, t),
        3 => (p, q, vv),
        4 => (t, p, vv),
        _ => (vv, p, q),
    };
    r << 16 | g << 8 | b | 0xFF00_0000
}

/// rgb_to_hsv(rgb) -> (h 0..3599, s 0..1000, v 0..1000)
pub fn rgb_to_hsv(rgb: u32) -> (u32, u32, u32) {
    let r = (rgb >> 16) & 255;
    let g = (rgb >> 8) & 255;
    let b = rgb & 255;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    // v = max * 1000 / 255
    let v = (max * 1000 + 127) / 255;
    let delta = max - min;
    if max == 0 {
        return (0, 0, v); // black: h = s = 0
    }
    let s = delta * 1000 / max;
    let mut h: i32 = 0;
    if delta != 0 {
        let d = delta as i32;
        let (r, g, b) = (r as i32, g as i32, b as i32);
        if max as i32 == r {
            h = (g - b) * 600 / d;
            if h < 0 {
                h += 3600;
            }
        } else if max as i32 == g {
            h = (b - r) * 600 / d + 1200;
        } else {
            h = (r - g) * 600 / d + 2400;
        }
    }
    let mut h = h as u32;
    if h >= 3600 {
        h -= 3600;
    }
    (h, s, v)
}

// clamp1000(v) -> v in 0..1000
fn clamp1000(v: i32) -> i32 {
    v.clamp(0, 1000)
}

// row of the target at y (the custom drawings write the target pixels directly)
fn put(g: &mut Gfx, x: i32, y: i32, c: u32) {
    let o = y as usize * g.sf.stride as usize + x as usize * 4;
    g.sf.px[o..o + 4].copy_from_slice(&c.to_le_bytes());
}

/// spectrum_draw: the saturation / value square of a hue (render callback)
pub(crate) struct SpectrumDraw {
    hue: u32,
}

impl Draw for SpectrumDraw {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let k = g.dc.clip;
        // rows y in [max(ry, cy0), min(ry + rh, cy1))
        let mut y = r[1].max(k[1]);
        while y < r[1].wrapping_add(r[3]).min(k[3]) {
            // v = 1000 - (y - ry) * 1000 / rh
            let v = 1000 - y.wrapping_sub(r[1]).wrapping_mul(1000) / r[3];
            let mut x = r[0].max(k[0]);
            while x < r[0].wrapping_add(r[2]).min(k[2]) {
                let s = x.wrapping_sub(r[0]).wrapping_mul(1000) / r[2];
                put(g, x, y, hsv_to_rgb(self.hue, s as u32, v as u32));
                x += 1;
            }
            y += 1;
        }
    }
}

/// hue_draw: the rainbow track (render callback)
pub(crate) struct HueDraw;

impl Draw for HueDraw {
    fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
        let k = g.dc.clip;
        let mut y = r[1].max(k[1]);
        while y < r[1].wrapping_add(r[3]).min(k[3]) {
            let mut x = r[0].max(k[0]);
            while x < r[0].wrapping_add(r[2]).min(k[2]) {
                let h = x.wrapping_sub(r[0]).wrapping_mul(3599) / r[2];
                put(g, x, y, hsv_to_rgb(h as u32, 1000, 1000));
                x += 1;
            }
            y += 1;
        }
    }
}

// wc_render_base(base, side): the wheel at full value (premultiplied)
fn wc_render_base(sf: &mut Surface, side: i32) {
    let half = side as f64 * 0.5; // centre
    let rr = half - 1.0; // radius: 1 px inside the square
    let stride = sf.stride as usize;
    for y in 0..side {
        let row = y as usize * stride;
        // fy = half - (y + 0.5)
        let fy = half - (y as f64 + 0.5);
        for x in 0..side {
            let fx = (x as f64 + 0.5) - half;
            let r = sqrtd(fx * fx + fy * fy);
            // coverage = clamp(R + 0.5 - r, 0, 1)
            let c = (rr + 0.5) - r;
            let o = row + x as usize * 4;
            let px = if !(c > 0.0) {
                0
            } else {
                let a = cvtd(minsd(c, 1.0) * 255.0); // alpha 0..255
                // saturation = min(r / R, 1) * 1000
                let sat = cvtd(minsd(r / rr, 1.0) * 1000.0);
                // hue = atan2(fy, fx) in tenths of a degree, 0 .. 3599
                let mut h = cvtd(deg_atan2(fy, fx) * 10.0);
                if h < 0 {
                    h += 3600;
                }
                if h as u32 >= 3600 {
                    h -= 3600;
                }
                let c = hsv_to_rgb(h as u32, sat as u32, 1000);
                // premultiply by the coverage
                if (a as u32) < 255 {
                    px_mix(0, c, a as u32) | (a as u32) << 24
                } else {
                    c | 0xFF00_0000
                }
            };
            sf.px[o..o + 4].copy_from_slice(&px.to_le_bytes());
        }
    }
}

// wc_render_disp(base, side, v) -> the base with the colour channels * v / 1000
fn wc_render_disp(base: &Surface, side: i32, v: u32) -> Surface {
    let mut d = Surface::new(side, side, FMT_PARGB);
    let (bs, ds) = (base.stride as usize, d.stride as usize);
    for y in 0..side as usize {
        for x in 0..side as usize {
            let o = y * bs + x * 4;
            let p = u32::from_le_bytes([base.px[o], base.px[o + 1], base.px[o + 2], base.px[o + 3]]);
            let mut q = p & 0xFF00_0000; // alpha stays
            for sh in [0, 8, 16] {
                let c = ((p >> sh) & 255).wrapping_mul(v).wrapping_add(500) / 1000;
                q |= c << sh;
            }
            let o = y * ds + x * 4;
            d.px[o..o + 4].copy_from_slice(&q.to_le_bytes());
        }
    }
    d
}

impl Ui {
    // ------------------------------------------------------------------ swatch

    /// ui_swatch(id, argb, selected) -> EV_*: the swatches of one box are one Tab stop (the
    /// selected one; arrows move inside, Space / Enter act, the menu key is a right click);
    /// next_flags(ST_NOFOCUS): a picture only, no Tab stop
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn swatch(&mut self, id: u32, argb: u32, selected: bool) -> u32 {
        let _src = self.src_scope();
        let f = self.take_flags();
        let st = self.style;
        let mut sp = Sp::default();
        let (w, h) = st.swatch_size(self, &mut sp);
        self.place(w, h);
        let hb = if f & ST_NOFOCUS != 0 {
            0
        } else {
            self.kb_box_group(0x5357);
            HB_FOCUS | if selected { HB_CURRENT } else { 0 }
        };
        let ev = self.behave(id, hb);
        // paint (style)
        sp.place(self.wp.ctx.last_rect);
        sp.ev = ev;
        sp.value = argb as i32;
        sp.value2 = selected as i32;
        st.swatch(self, &sp);
        self.focus_ring_draw(ev);
        if selected {
            self.wp.ctx.last_state |= SS_SELECTED as u32;
        }
        self.sem(K_SWATCH, id, None, argb as i32);
        ev
    }

    // ------------------------------------------------------------------ colour picker

    /// ui_color_picker(id, &mut argb) -> EV_*
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn color_picker(&mut self, id: u32, argb: &mut u32) -> u32 {
        let _src = self.src_scope();
        self.color_picker_ex(id, argb, 0)
    }

    // the picker's HSV state: slot(id) measw = the colour it belongs to, scrollx / scrolly /
    // aux = h / s / v
    fn cp_hsv(&mut self, id: u32) -> (u32, u32, u32) {
        let s = self.slot(id);
        (s.scrollx as u32, s.scrolly as u32, s.aux as u32)
    }

    // .cp_apply: *argb = the HSV state (alpha kept, 0 -> opaque), remembered
    fn cp_apply(&mut self, id: u32, argb: &mut u32) {
        let (h, s, v) = self.cp_hsv(id);
        let rgb = hsv_to_rgb(h, s, v) & 0xFF_FFFF;
        let mut a = *argb & 0xFF00_0000;
        if a == 0 {
            a = 0xFF00_0000;
        }
        *argb = rgb | a;
        self.slot(id).measw = *argb as i32;
        self.repass();
    }

    // .cp_marker: the style's ring marker at (x, y) 24.8 px
    fn cp_marker(&mut self, x: i32, y: i32) {
        let st = self.style;
        st.color_marker(self, x, y);
    }

    /// ui_color_picker_ex(id, &mut argb, flags) -> EV_* (CP_ALPHA, CP_FIELDS, CP_PIPETTE)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn color_picker_ex(&mut self, id: u32, argb: &mut u32, flags: u32) -> u32 {
        let _src = self.src_scope();
        let mut ev = 0;
        let old = *argb;
        // a colour the pipette picked for this picker (screen.rs)
        if let Some((owner, c)) = self.m.screen.pending {
            if owner == id {
                self.m.screen.pending = None;
                let a = if *argb >> 24 == 0 { 0xFF00_0000 } else { *argb & 0xFF00_0000 };
                *argb = a | (c & 0xFF_FFFF);
                ev |= EV_COMMIT;
            }
        }
        // resync the HSV state when the colour was changed from outside
        if *argb as i32 != self.slot(id).measw {
            let c = *argb;
            let (h, s, v) = rgb_to_hsv(c);
            let sl = self.slot(id);
            sl.measw = c as i32;
            if s != 0 {
                sl.scrollx = h as i32; // grey: keep the previous hue
            }
            sl.scrolly = s as i32;
            sl.aux = v as i32;
        }
        self.box_begin(0, BOX_V, 12);
        let rect = self.wp.ctx.last_rect; // (U1) the picker's rectangle for its record

        // ---- spectrum: x = saturation, y = value
        let (w, h) = (self.dp(256), self.dp(180));
        let r = self.place(w, h);
        self.ltr_begin(r[0], r[2]); // right-to-left: colours keep their direction
        let e = self.behave(id.wrapping_add(PART_SPEC), HB_FOCUS | HB_MOVES | CUR_CROSS << HB_CURSOR_SHIFT);
        ev |= e;
        // keyboard: Left / Right saturation, Up / Down brightness (1 %, Shift 10 %)
        let (dx, dy) = self.cp_keys(e, 10, true);
        if dx | dy != 0 {
            let sl = self.slot(id);
            sl.scrolly = (sl.scrolly + dx).clamp(0, 1000);
            sl.aux = (sl.aux - dy).clamp(0, 1000);
            self.cp_apply(id, argb);
        }
        if e & EV_HELD != 0 {
            let r = self.wp.ctx.last_rect;
            let s = clamp1000(self.ltr_x(self.wp.ctx.mx).wrapping_sub(r[0]).wrapping_mul(1000) / r[2]);
            let v = 1000 - clamp1000(self.wp.ctx.my.wrapping_sub(r[1]).wrapping_mul(1000) / r[3]);
            let sl = *self.slot(id);
            // (U1) only a change repaints: a held pointer must not repass for ever
            if s != sl.scrolly || v != sl.aux {
                let sl = self.slot(id);
                sl.scrolly = s;
                sl.aux = v;
                self.cp_apply(id, argb);
            }
        }
        let r = self.wp.ctx.last_rect;
        let (hue, sat, val) = self.cp_hsv(id);
        let d = self.spectrum_drawing(hue);
        self.cmd_custom(r[0], r[1], r[2], r[3], Some(d), hue as usize, hue);
        // marker ring at (s, v)
        let mx = ((sat.wrapping_mul(r[2] as u32) / 1000) as i32).wrapping_add(r[0]) << 8;
        let my = (((1000u32.wrapping_sub(val)).wrapping_mul(r[3] as u32) / 1000) as i32).wrapping_add(r[1]) << 8;
        self.cp_marker(mx, my);
        self.cp_part_end(e, id.wrapping_add(PART_SPEC), super::i18n::tr("Sättigung und Helligkeit"), sat as i32, 1000);
        self.ltr_end();

        // ---- hue slider
        let (w, h) = (self.dp(256), self.dp(16));
        let r = self.place(w, h);
        self.ltr_begin(r[0], r[2]);
        let e = self.behave(id.wrapping_add(PART_HUE), HB_FOCUS | CUR_HAND << HB_CURSOR_SHIFT);
        ev |= e;
        // keyboard: arrows 1 degree (Shift 10), Home / End the ends
        let (dx, _) = self.cp_keys(e, 10, false);
        if dx != 0 {
            let sl = self.slot(id);
            sl.scrollx = (sl.scrollx + dx).clamp(0, 3599);
            self.cp_apply(id, argb);
        }
        if e & EV_HELD != 0 {
            let r = self.wp.ctx.last_rect;
            let hh = (self.ltr_x(self.wp.ctx.mx).wrapping_sub(r[0]).wrapping_mul(3599) / r[2]).clamp(0, 3599);
            if hh != self.slot(id).scrollx {
                // (U1) only a change repaints
                self.slot(id).scrollx = hh;
                self.cp_apply(id, argb);
            }
        }
        let r = self.wp.ctx.last_rect;
        let d = self.hue_drawing();
        self.cmd_custom(r[0], r[1], r[2], r[3], Some(d), 0, 0);
        let hue = self.slot(id).scrollx as u32;
        let mx = ((hue.wrapping_mul(r[2] as u32) / 3599) as i32).wrapping_add(r[0]) << 8;
        let my = (r[3] << 7).wrapping_add(r[1] << 8);
        self.cp_marker(mx, my);
        self.cp_part_end(e, id.wrapping_add(PART_HUE), super::i18n::tr("Farbton"), hue as i32, 3599);
        self.ltr_end();

        // ---- (U1) alpha bar: transparent .. opaque over a checkerboard
        if flags & CP_ALPHA != 0 {
            let (w, h) = (self.dp(256), self.dp(16));
            let r = self.place(w, h);
            self.ltr_begin(r[0], r[2]);
            let e = self.behave(id.wrapping_add(PART_ALPHA), HB_FOCUS | CUR_HAND << HB_CURSOR_SHIFT);
            ev |= e;
            // keyboard: arrows 1 of 255 (Shift 16), Home / End the ends
            let (dx, _) = self.cp_keys(e, 1, false);
            if dx != 0 {
                let d = if dx.abs() == 10 { dx / 10 * 16 } else { dx };
                let a = ((*argb >> 24) as i32 + d).clamp(0, 255) as u32;
                let na = a << 24 | (*argb & 0xFF_FFFF);
                if na != *argb {
                    *argb = na;
                    self.slot(id).measw = na as i32; // rgb unchanged: the HSV state stays
                    self.repass();
                }
            }
            if e & EV_HELD != 0 {
                let r = self.wp.ctx.last_rect;
                let half = ((r[2] as u32) >> 1) as i32;
                let a = (self.ltr_x(self.wp.ctx.mx).wrapping_sub(r[0]).wrapping_mul(255).wrapping_add(half) / r[2]).clamp(0, 255);
                let na = (a as u32) << 24 | (*argb & 0xFF_FFFF);
                if na != *argb {
                    *argb = na;
                    self.slot(id).measw = na as i32; // rgb unchanged: the HSV state stays
                    self.repass();
                }
            }
            let tk = Track::new(TK_RECT | TK_CHECKER, &[*argb & 0xFF_FFFF, *argb | 0xFF00_0000]);
            let r = self.wp.ctx.last_rect;
            self.cmd_track(r[0], r[1], r[2], r[3], &tk);
            let mx = (((*argb >> 24).wrapping_mul(r[2] as u32) / 255) as i32).wrapping_add(r[0]) << 8;
            let my = (r[3] << 7).wrapping_add(r[1] << 8);
            self.cp_marker(mx, my);
            self.cp_part_end(e, id.wrapping_add(PART_ALPHA), super::i18n::tr("Deckkraft"), (*argb >> 24) as i32, 255);
            self.ltr_end();
        }

        // ---- preview + hex
        self.box_begin(0, BOX_H | BOX_XCENTER, 12);
        let (w, h) = (self.dp(48), self.dp(28)); // the preview well (style paints it)
        self.place(w, h);
        let st = self.style;
        let r = self.wp.ctx.last_rect;
        st.color_preview(self, *argb, r);
        let hex = format!("#{:06X}", *argb & 0xFF_FFFF);
        if flags & CP_FIELDS == 0 {
            self.text(TS_BODY, &hex);
        }
        // the pipette: a colour from anywhere on the screen (only where that is possible)
        if flags & CP_PIPETTE != 0 && self.pick_available() {
            let e = self.icon_button(part_id(id, 11), super::keys::IC_EYEDROPPER, super::i18n::tr("Farbe vom Bildschirm aufnehmen"));
            if e & EV_ACT != 0 {
                self.pick_start_for(id);
            }
        }
        self.box_end();
        if flags & CP_FIELDS != 0 {
            ev |= self.cp_fields(id, argb, flags);
        }
        self.box_end();
        self.wp.ctx.last_rect = rect;
        self.wp.ctx.last_state = 0;
        self.sem(K_SWATCH, id, None, *argb as i32);
        let mut ret = ev & !EV_ACT;
        if *argb != old {
            ret |= EV_ACT;
        }
        ret
    }

    // cp_keys(ev, step, two_d) -> (dx, dy) the arrows ask for while the picker part has the
    // focus (Shift: 10 steps; Home / End: the ends of the x axis); one axis: Up / Right add
    fn cp_keys(&mut self, ev: u32, step: i32, two_d: bool) -> (i32, i32) {
        if ev & EV_FOCUS == 0 {
            return (0, 0);
        }
        let s = if self.mods & MOD_SHIFT != 0 { step * 10 } else { step };
        let mut h = (self.key(VK_RIGHT) as i32 - self.key(VK_LEFT) as i32) * s;
        if self.rtl {
            h = -h; // the parts keep their colours left to right: arrows as drawn
        }
        let up = (self.key(VK_UP) as i32 - self.key(VK_DOWN) as i32) * s;
        let (mut dx, dy) = if two_d { (h, -up) } else { (h + up, 0) };
        if self.key(VK_HOME) {
            dx = -100_000;
        }
        if self.key(VK_END) {
            dx = 100_000;
        }
        if dx | dy != 0 {
            self.wp.ctx.focus_visible = true;
        }
        (dx, dy)
    }

    // cp_part_end(ev, id, label, value, max): the focus ring and the record of a picker part
    fn cp_part_end(&mut self, ev: u32, id: u32, label: &str, value: i32, max: i32) {
        self.focus_ring_draw(ev);
        let r = self.sem(K_SLIDER, id, Some(label), value);
        r.max = max;
    }

    // the spectrum drawing of a hue (one shared object while the hue stays)
    fn spectrum_drawing(&mut self, hue: u32) -> Rc<dyn Draw> {
        let s = &mut self.m.color;
        match &s.spec {
            Some(d) if d.hue == hue => d.clone(),
            _ => {
                let d = Rc::new(SpectrumDraw { hue });
                s.spec = Some(d.clone());
                d
            }
        }
    }

    fn hue_drawing(&mut self) -> Rc<dyn Draw> {
        self.m.color.hue.get_or_insert_with(|| Rc::new(HueDraw)).clone()
    }

    // cp_fields(id, &argb, flags) -> EV: number boxes R G B (A) / H S V and a hex text box
    // under the picker; their values live in the field store of the picker
    fn cp_fields(&mut self, id: u32, argb: &mut u32, flags: u32) -> u32 {
        let mut ev = 0;
        // this frame's values
        let (h, s, v) = self.cp_hsv(id);
        {
            let f = self.m.color.fields.entry(id).or_default();
            let c = *argb;
            f.rgba = [((c >> 16) & 255) as i32, ((c >> 8) & 255) as i32, (c & 255) as i32, (c >> 24) as i32];
            let mut hh = h.wrapping_add(5) / 10;
            if hh >= 360 {
                hh = 0;
            }
            f.hsv = [hh as i32, (s.wrapping_add(5) / 10) as i32, (v.wrapping_add(5) / 10) as i32];
        }
        // hex text, unless it is being edited
        if self.wp.ctx.focus != part_id(id, 10) {
            self.cf_hex_text(id, *argb, flags);
        }
        self.box_begin(0, BOX_V, 4);
        for row in 0..4usize {
            self.box_begin(0, BOX_H | BOX_XCENTER, 8);
            // left: R, G, B, A
            if row == 3 && flags & CP_ALPHA == 0 {
                self.spacer(24 + 8 + 72);
            } else {
                self.next_size(24, 0);
                self.text(TS_BODY, FLD_LEFT[row]);
                self.next_size(72, 0);
                let mut val = self.m.color.fields.get(&id).map_or(0, |f| f.rgba[row]);
                let e = self.number_box(part_id(id, row as u32 + 3), &mut val, &RG_BYTE);
                if let Some(f) = self.m.color.fields.get_mut(&id) {
                    f.rgba[row] = val;
                }
                ev |= e;
                if e & EV_ACT != 0 {
                    self.cf_apply_rgba(id, argb, flags);
                }
            }
            self.spacer(8);
            self.next_size(32, 0);
            self.text(TS_BODY, FLD_RIGHT[row]);
            if row == 3 {
                self.next_size(88, 0);
                let mut t = self.m.color.fields.get(&id).map_or(String::new(), |f| f.hex.clone());
                let e = self.textbox(part_id(id, 10), None, &mut t, 12);
                if let Some(f) = self.m.color.fields.get_mut(&id) {
                    f.hex = t;
                }
                ev |= e & !EV_ACT;
                if e & EV_COMMIT != 0 {
                    self.cf_apply_hex(id, argb, flags);
                    self.cf_hex_text(id, *argb, flags); // the committed text shows the colour
                    self.repass();
                }
            } else {
                self.next_size(72, 0);
                let mut val = self.m.color.fields.get(&id).map_or(0, |f| f.hsv[row]);
                let rg = if row == 0 { &RG_HUE } else { &RG_PCT };
                let e = self.number_box(part_id(id, row as u32 + 7), &mut val, rg);
                if let Some(f) = self.m.color.fields.get_mut(&id) {
                    f.hsv[row] = val;
                }
                ev |= e;
                if e & EV_ACT != 0 {
                    self.cf_apply_hsv(id, argb);
                }
            }
            self.box_end();
        }
        self.box_end();
        ev
    }

    // .cf_hex_text: the colour as RRGGBB (AARRGGBB with CP_ALPHA) into the hex field
    fn cf_hex_text(&mut self, id: u32, argb: u32, flags: u32) {
        let t = if flags & CP_ALPHA != 0 { format!("{:08X}", argb) } else { format!("{:06X}", argb & 0xFF_FFFF) };
        self.m.color.fields.entry(id).or_default().hex = t;
    }

    // .cf_apply_rgba: R G B A fields -> *argb, HSV follows
    fn cf_apply_rgba(&mut self, id: u32, argb: &mut u32, flags: u32) {
        let f = self.m.color.fields.entry(id).or_default().rgba;
        let mut c = (f[3] as u32) << 24 | (f[0] as u32) << 16 | (f[1] as u32) << 8 | f[2] as u32;
        if flags & CP_ALPHA == 0 {
            c = (c & 0xFF_FFFF) | (*argb & 0xFF00_0000);
        }
        self.cf_set_argb(id, argb, c);
    }

    // .cf_apply_hsv: H S V fields -> slot HSV -> *argb (alpha kept)
    fn cf_apply_hsv(&mut self, id: u32, argb: &mut u32) {
        let f = self.m.color.fields.entry(id).or_default().hsv;
        let sl = self.slot(id);
        sl.scrollx = f[0].wrapping_mul(10);
        sl.scrolly = f[1].wrapping_mul(10);
        sl.aux = f[2].wrapping_mul(10);
        let (h, s, v) = (sl.scrollx as u32, sl.scrolly as u32, sl.aux as u32);
        let c = (hsv_to_rgb(h, s, v) & 0xFF_FFFF) | (*argb & 0xFF00_0000);
        *argb = c;
        self.slot(id).measw = c as i32;
        self.repass();
    }

    // .cf_apply_hex: the hex text -> *argb (6 digits keep the alpha)
    fn cf_apply_hex(&mut self, id: u32, argb: &mut u32, _flags: u32) {
        let t = self.m.color.fields.entry(id).or_default().hex.clone();
        let b = t.as_bytes();
        // hex digits (the asm tests c | 32 against '0'..'9' and 'a'..'f')
        let n = b.iter().take_while(|&&c| c != 0).filter(|&&c| matches!(c | 32, b'0'..=b'9' | b'a'..=b'f')).count();
        if n == 0 {
            return;
        }
        let mut i = 0;
        while i < b.len() && b[i] == b' ' {
            i += 1;
        }
        let (mut c, _) = super::runtime::parse_hex(&b[i..]);
        if n <= 6 {
            c = (c & 0xFF_FFFF) | (*argb & 0xFF00_0000);
        }
        self.cf_set_argb(id, argb, c);
    }

    // .cf_set_argb(c): *argb = c, the slot's HSV follows (greys keep their hue)
    fn cf_set_argb(&mut self, id: u32, argb: &mut u32, c: u32) {
        if c == *argb {
            return;
        }
        *argb = c;
        let (h, s, v) = rgb_to_hsv(c);
        let sl = self.slot(id);
        sl.measw = c as i32;
        if s != 0 {
            sl.scrollx = h as i32;
        }
        sl.scrolly = s as i32;
        sl.aux = v as i32;
        self.repass();
    }

    // ------------------------------------------------------------------ colour wheel

    // wc_get(id, side, v) -> the display image of wheel id at this size and value
    fn wc_get(&mut self, id: u32, side: i32, v: i32) -> Rc<Surface> {
        let frame = self.frame;
        let s = &mut self.m.color;
        // the entry of id, else a free or the least recently used one
        let mut cand = 0;
        let mut age = u32::MAX;
        let mut found = None;
        for (i, e) in s.wc.iter().enumerate() {
            if e.id == id {
                found = Some(i);
                break;
            }
            let a = if e.id == 0 { 0 } else { e.seen };
            if a < age {
                age = a;
                cand = i;
            }
        }
        let i = match found {
            Some(i) => i,
            None => {
                let e = &mut s.wc[cand];
                *e = Wc { id, ..Wc::default() };
                cand
            }
        };
        let e = &mut s.wc[i];
        e.seen = frame;
        if e.size != side {
            // new size: both images, the base rendered at full value
            e.base = Surface::new(side, side, FMT_PARGB);
            e.size = side;
            e.v = -1;
            wc_render_base(&mut e.base, side);
        }
        if e.v != v {
            e.v = v;
            e.disp = Rc::new(wc_render_disp(&e.base, side, v as u32));
            s.wc_ver = s.wc_ver.wrapping_add(1);
            e.ver = s.wc_ver;
        }
        s.wc[i].disp.clone()
    }

    /// ui_color_wheel(id, &mut color, flags) -> EV_*: color = [argb], or [h, s, v] with
    /// CW_HSV
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn color_wheel(&mut self, id: u32, color: &mut [u32], flags: u32) -> u32 {
        let _src = self.src_scope();
        let hsv = flags & CW_HSV != 0;
        let mut keych = false;
        // the colour as hue / saturation / value
        let (mut h, mut s, v);
        if hsv {
            h = color[0] as i32;
            s = color[1] as i32;
            v = color[2] as i32;
        } else {
            let sl = *self.slot(id);
            let valid = sl.afrom == 1 && sl.ato == 0; // state valid
            if color[0] as i32 == sl.measw && valid {
                h = sl.scrollx;
                s = sl.scrolly;
                v = sl.aux;
            } else {
                let (hh, ss, vv) = rgb_to_hsv(color[0]);
                h = hh as i32;
                if ss == 0 && valid {
                    h = sl.scrollx; // grey: keep the previous hue
                }
                s = ss as i32;
                v = vv as i32;
            }
        }
        let (oh, os) = (h, s);
        let stflags = self.take_flags();
        let n = self.dp(WHEEL_NAT);
        self.place(n, n);
        let mut hb = HB_FOCUS | CUR_CROSS << HB_CURSOR_SHIFT;
        if stflags & ST_DISABLED != 0 {
            hb |= HB_DISABLED;
        }
        let mut ev = self.behave(id, hb);
        // geometry: the largest centred square
        let r = self.wp.ctx.last_rect;
        self.ltr_begin(r[0], r[2]); // right-to-left: hues keep their places
        let side = r[2].min(r[3]).max(8);
        let wx = (r[2].wrapping_sub(side) >> 1).wrapping_add(r[0]);
        let wy = (r[3].wrapping_sub(side) >> 1).wrapping_add(r[1]);
        let rad = ((side as u32 >> 1) as i32).wrapping_sub(1);
        if ev & EV_HELD != 0 {
            // pointer: hue from the angle, saturation from the distance
            let half = (side as u32 >> 1) as i32;
            let dx = self.ltr_x(self.wp.ctx.mx).wrapping_sub(wx).wrapping_sub(half); // px from the centre
            let dy = wy.wrapping_add(half).wrapping_sub(self.wp.ctx.my); // up
            let fy = dy as f64;
            let fx = dx as f64;
            if dx | dy != 0 {
                // the centre: hue unchanged
                let mut hh = f_round(deg_atan2(fy, fx) * 10.0) as i32;
                if hh < 0 {
                    hh += 3600;
                }
                if hh as u32 >= 3600 {
                    hh -= 3600;
                }
                h = hh;
            }
            let d = sqrtd(fy * fy + fx * fx) / rad as f64;
            s = f_round(minsd(d, 1.0) * 1000.0) as i32;
        } else if ev & EV_FOCUS != 0 {
            let st = if self.mods & MOD_SHIFT != 0 { 100 } else { 10 }; // 1 degree / 1 %
            let mut dh = self.key(VK_RIGHT) as i32;
            dh -= self.key(VK_LEFT) as i32;
            let mut ds = self.key(VK_UP) as i32;
            ds -= self.key(VK_DOWN) as i32;
            if dh | ds != 0 {
                keych = true;
                self.wp.ctx.focus_visible = true;
                let mut rem = (h.wrapping_add(dh.wrapping_mul(st)).wrapping_add(3600)) % 3600; // wrap 0 .. 3599
                if rem < 0 {
                    rem += 3600;
                }
                h = rem;
                s = s.wrapping_add(ds.wrapping_mul(st)).clamp(0, 1000);
            }
        }
        if h != oh || s != os {
            ev |= EV_ACT;
            if keych {
                ev |= EV_COMMIT;
            }
            self.repass();
            if hsv {
                color[0] = h as u32;
                color[1] = s as u32;
            } else {
                let c = hsv_to_rgb(h as u32, s as u32, v as u32) & 0xFF_FFFF;
                color[0] = c | (color[0] & 0xFF00_0000);
            }
        }
        // remember the HSV state of an ARGB colour
        if !hsv {
            let c = color[0];
            let sl = self.slot(id);
            sl.measw = c as i32;
            sl.afrom = 1;
            sl.ato = 0;
            sl.scrollx = h;
            sl.scrolly = s;
            sl.aux = v;
        }
        // the cached image at the current value
        let img = self.wc_get(id, side, v.clamp(0, 1000));
        let ver = self.m.color.wc.iter().find(|e| e.id == id).map_or(0, |e| e.ver);
        self.draw_image(&img, ver, wx, wy, 0, 0);
        // paint (style): rim, marker, focus visual
        let st = self.style;
        let mut sp = Sp { x: wx, y: wy, w: side, h: side, value: h, value2: s, ..Sp::default() };
        sp.s[0] = rad;
        sp.s[1] = self.focus_ring(ev) as i32;
        st.color_wheel(self, &sp);
        self.ltr_end();
        self.sem(K_SLIDER, id, None, h).max = s;
        ev
    }
}
