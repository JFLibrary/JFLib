// value -- the classic value sliders (also gradient tracks), angle dial, pan control
//                                                                 [styles/classic/value.s]
//
//   value_slider_size(sp), value_slider_thumb(), value_slider(sp)
//                                       trackbar (11 DIP thumb); with a track the groove is a
//                                       sunken 12 DIP gradient band
//   angle_size(sp), angle(sp)           64 DIP sunken round well, radius line, square knob
//   pan_size(sp), pan(sp)               sunken frame around the thumbnail, crosshair, marker
//
// Contract: ASMLIB/inc/style.inc (lib/ui/style.rs).

use ui::consts::*;
use ui::{cvtd, deg_sincos, Sp, StyleValue, Ui};

use crate::basic::{focus_ring, trackbar};
use crate::common::*;
use crate::Classic;

const THUMB_DIP: i32 = 11; // trackbar thumb length
const GTRACK_DIP: i32 = 12; // gradient band thickness

impl StyleValue for Classic {
    fn value_slider_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        // 200 x 24 DIP, swapped with RG_VERT
        let w = ui.dp(200);
        let h = ui.dp(24);
        if sp.flags & RG_VERT != 0 { (h, w) } else { (w, h) }
    }
    fn value_slider(&self, ui: &mut Ui, sp: &Sp) {
        value_slider(ui, sp)
    }
    fn value_slider_thumb(&self, ui: &mut Ui) -> i32 {
        ui.dp(THUMB_DIP)
    }
    fn angle_size(&self, ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        // 64 x 64 DIP
        let s = ui.dp(64);
        (s, s)
    }
    fn angle(&self, ui: &mut Ui, sp: &Sp) {
        angle(ui, sp)
    }
    fn pan_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        pan_size(ui, sp)
    }
    fn pan(&self, ui: &mut Ui, sp: &Sp) {
        pan(ui, sp)
    }
}

/// sty_value_slider(sp): trackbar; with a track the groove is a sunken gradient band and
/// the thumb a raised block.  value t 0..65536 along the travel, flags RG_*
fn value_slider(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let vert = sp.flags & RG_VERT != 0;
    let thumb = ui.dp(THUMB_DIP);
    let mut len = (if vert { r[3] } else { r[2] }).wrapping_sub(thumb);
    if len < 1 {
        len = 1;
    }
    let off = ((sp.value as i64).wrapping_mul(len as i64) as u64 >> 16) as i32; // thumb offset
    let Some(tk) = sp.track else {
        trackbar(ui, r, thumb, off, sp.state, !vert);
        return;
    };
    // gradient band across the travel
    let g = ui.dp(GTRACK_DIP);
    let half = ((thumb as u32) >> 1) as i32;
    let band = if !vert {
        [half.wrapping_add(r[0]), (r[3].wrapping_sub(g) >> 1).wrapping_add(r[1]), len, g]
    } else {
        [(r[2].wrapping_sub(g) >> 1).wrapping_add(r[0]), half.wrapping_add(r[1]), g, len]
    };
    ui.cmd_track(band[0], band[1], band[2], band[3], tk);
    frame(ui, band, FR_THIN_SUNKEN);
    // the thumb alone: a raised block (.vs_thumb)
    let d = ui.dp(2);
    let th = if !vert {
        [r[0].wrapping_add(off), r[1].wrapping_add(d), thumb, r[3].wrapping_sub(d).wrapping_sub(d)]
    } else {
        let y = r[1].wrapping_add(r[3]).wrapping_sub(thumb).wrapping_sub(off);
        [r[0].wrapping_add(d), y, r[2].wrapping_sub(d).wrapping_sub(d), thumb]
    };
    let c = pal(ui, CP_FACE);
    fill(ui, th, c);
    frame(ui, th, FR_RAISED);
}

/// sty_angle(sp): sunken round well, a radius line in the text colour with a square knob;
/// dotted focus rectangle (s[0]).  f: degrees
fn angle(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let state = sp.state;
    let side = if r[3] < r[2] { r[3] } else { r[2] };
    let rad = (side << 7).wrapping_sub(256); // radius (fx)
    let cx = (r[0] << 8).wrapping_add(r[2] << 7);
    let cy = (r[1] << 8).wrapping_add(r[3] << 7);
    // well: grey ring, field inside
    let c = pal(ui, CP_SH);
    ui.draw_color(c);
    ui.draw_circle(cx, cy, rad, 0);
    let c = pal(ui, if state == 3 { CP_FACE } else { CP_FIELD });
    ui.draw_color(c);
    let d = ui.dp_fx(2 * 256);
    ui.draw_circle(cx, cy, rad.wrapping_sub(d), 0);
    // radius line and knob
    let c = ui.color(if state == 3 { T_TEXT_DISABLED } else { T_TEXT });
    ui.draw_color(c);
    let (cs, sn) = deg_sincos(sp.f);
    let kd = rad.wrapping_sub(ui.dp_fx(6 * 256)) as f64; // knob distance (fx)
    let kx = cvtd(kd * cs).wrapping_add(cx); // knob x (fx)
    let ky = cy.wrapping_sub(cvtd(kd * sn)); // knob y (fx)
    let w = ui.hair() << 8;
    ui.draw_line(cx, cy, kx, ky, w);
    let hs = ui.dp(3); // knob half size (px)
    let knob = [(kx >> 8).wrapping_sub(hs), (ky >> 8).wrapping_sub(hs), hs + hs, hs + hs];
    let c = pal(ui, CP_FACE);
    fill(ui, knob, c);
    frame(ui, knob, FR_THIN_RAISED);
    if sp.s[0] != 0 {
        focus_ring(ui, r, 4);
    }
}

/// sty_pan_size(sp) -> the thumbnail (img) fitted into 160 DIP, else 160 x 120; at least 8 x 8
fn pan_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    let mut w = ui.dp(160);
    let mut h = ui.dp(120);
    if let Some(sf) = sp.img {
        let (sw, sh) = (sf.w, sf.h);
        if sw > 0 && sh > 0 {
            if sw >= sh {
                h = w.wrapping_mul(sh) / sw;
            } else {
                let nw = sw.wrapping_mul(w) / sh;
                h = w;
                w = nw;
            }
        }
    }
    if w < 8 {
        w = 8;
    }
    if h < 8 {
        h = 8;
    }
    (w, h)
}

/// sty_pan(sp): sunken frame around the thumbnail (img), crosshair through the point value,
/// value2 (px) and a black / white square marker
fn pan(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let c = pal(ui, CP_FIELD);
    fill(ui, r, c);
    frame(ui, r, FR_SUNKEN);
    let t = ui.hair();
    let inner = inset(r, t + t);
    ui.draw_clip(inner[0], inner[1], inner[2], inner[3]);
    if let Some(sf) = sp.img {
        ui.draw_image(sf, sp.img_ver, inner[0], inner[1], inner[2], inner[3]);
    }
    rect(ui, sp.value, inner[1], 1, inner[3], 0xFF00_0000);
    rect(ui, inner[0], sp.value2, inner[2], 1, 0xFF00_0000);
    // marker: white square with a black outline
    let d = ui.dp(4);
    let s = d + d + 1;
    let m = [sp.value.wrapping_sub(d), sp.value2.wrapping_sub(d), s, s];
    fill(ui, m, 0xFFFF_FFFF);
    frame(ui, m, FR_BLACK);
    ui.draw_unclip();
}
