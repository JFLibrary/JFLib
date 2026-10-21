// value -- NetLab value sliders, angle dial, pan control (the slider look of basic.rs)
//
//   value_slider_size(sp), value_slider_thumb(), value_slider(sp)
//                                       slider_f / track / property sliders: 4 DIP grey track
//                                       with the accent part or a framed 10 DIP gradient bar;
//                                       a framed 10 x 18 DIP thumb (gradient: filled with the
//                                       colour under it)
//   angle_size(sp), angle(sp)           64 DIP dial
//   pan_size(sp), pan(sp)               framed thumbnail with a crosshair and a marker ring
//
// Contract: ASMLIB/inc/style.inc (lib/ui/style.rs).

use ui::consts::*;
use ui::{cvtd, deg_sincos, grad_at, Sp, StyleValue, Ui};

use crate::Netlab;

const THUMB_DIP: i32 = 10; // slider thumb length along the track
const THUMB_ACROSS: i32 = 18; // ... and across it
const TRACK_DIP: i32 = 4; // standard track thickness
const GTRACK_DIP: i32 = 10; // gradient track thickness
const ANG_NAT: i32 = 64; // natural size of the angle dial (DIP)
const PAN_NAT_W: i32 = 160; // natural size of the pan control without a thumbnail
const PAN_NAT_H: i32 = 120;

impl StyleValue for Netlab {
    fn value_slider_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        // 200 x 24, 24 x 200 with RG_VERT
        let (w, h) = (ui.dp(200), ui.dp(24));
        if sp.flags & RG_VERT != 0 { (h, w) } else { (w, h) }
    }
    fn value_slider(&self, ui: &mut Ui, sp: &Sp) {
        value_slider(ui, sp)
    }
    fn value_slider_thumb(&self, ui: &mut Ui) -> i32 {
        // thumb length (px): the travel is the length minus it
        ui.dp(THUMB_DIP)
    }
    fn angle_size(&self, ui: &mut Ui, _sp: &mut Sp) -> (i32, i32) {
        // 64 x 64
        let s = ui.dp(ANG_NAT);
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

// .svs_pill: the travel as a rectangle of thickness t centred on the track
fn pill(r: [i32; 4], vert: bool, half: i32, travel: i32, cx: i32, cy: i32, t: i32) -> [i32; 4] {
    let ht = ((t as u32) >> 1) as i32;
    if !vert {
        [r[0].wrapping_add(half), cy.wrapping_sub(ht), travel, t]
    } else {
        [cx.wrapping_sub(ht), r[1].wrapping_add(half), t, travel]
    }
}

/// sty_value_slider(sp): track (standard: grey fill + accent up to the thumb; track = a
/// gradient pill with a hairline frame) and the knob; value t 0..65536, flags RG_*
fn value_slider(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let t = sp.value;
    let state = sp.state;
    let vert = sp.flags & RG_VERT != 0;
    let thumb = ui.dp(THUMB_DIP);
    let half = ((thumb as u32) >> 1) as i32;
    let mut travel = (if vert { r[3] } else { r[2] }).wrapping_sub(thumb);
    if travel < 1 {
        travel = 1;
    }
    let off = ((t as i64).wrapping_mul(travel as i64) as u64 >> 16) as i32; // thumb offset on the travel
    // thumb centre (px)
    let (cx, cy) = if !vert {
        (r[0].wrapping_add(half).wrapping_add(off), (((r[3] as u32) >> 1) as i32).wrapping_add(r[1]))
    } else {
        ((((r[2] as u32) >> 1) as i32).wrapping_add(r[0]), r[1].wrapping_add(r[3]).wrapping_sub(half).wrapping_sub(off))
    };
    match sp.track {
        None => {
            // ---- standard track: grey fill over the travel, accent up to the thumb
            let th = ui.dp(TRACK_DIP);
            let mut p = pill(r, vert, half, travel, cx, cy, th);
            let c = ui.color(T_TRACK);
            ui.draw_color(c);
            let rad = ui.dp(1);
            ui.draw_rrect(p[0], p[1], p[2], p[3], rad);
            let c = ui.color((state << 2).wrapping_add(T_ACCENT));
            ui.draw_color(c);
            // accent part: from the start (left / bottom) to the thumb centre
            if !vert {
                p[2] = cx.wrapping_sub(p[0]);
            } else {
                p[3] = p[1].wrapping_add(p[3]).wrapping_sub(cy);
                p[1] = cy;
            }
            ui.draw_rrect(p[0], p[1], p[2], p[3], rad);
        }
        Some(tk) => {
            // ---- gradient track (a 10 DIP bar around the travel) with a hairline frame
            let th = ui.dp(GTRACK_DIP);
            let mut p = pill(r, vert, half, travel, cx, cy, th);
            let e = ((th as u32) >> 1) as i32; // the bar reaches half its thickness past the travel ends
            if !vert {
                p[0] = p[0].wrapping_sub(e);
                p[2] = p[2].wrapping_add(e.wrapping_add(e));
            } else {
                p[1] = p[1].wrapping_sub(e);
                p[3] = p[3].wrapping_add(e.wrapping_add(e));
            }
            ui.cmd_track(p[0], p[1], p[2], p[3], tk);
            let c = ui.color(T_CTRL_STROKE2);
            ui.draw_color(c);
            let hair = ui.hair();
            ui.draw_frame(p[0], p[1], p[2], p[3], ui.dp(2), hair);
        }
    }
    // thumb: a framed 10 x 18 DIP rectangle (control fill, accent frame on hover); on a
    // gradient track filled with the colour under it, a light inner line keeps it visible
    let (tw, ta) = (ui.dp(THUMB_DIP), ui.dp(THUMB_ACROSS));
    let tr = if !vert {
        [cx - (tw >> 1), cy - (ta >> 1), tw, ta]
    } else {
        [cx - (ta >> 1), cy - (tw >> 1), ta, tw]
    };
    let hair = ui.hair();
    let rad = ui.dp(2);
    let stroke = match state {
        1 | 2 => ui.color(T_ACCENT),
        3 => ui.color(T_CTRL_STROKE),
        _ => ui.color(T_STRONG_STROKE),
    };
    let Some(tk) = sp.track else {
        let fill = ui.color(match state {
            2 => T_CTRL_PRESSED,
            3 => T_CTRL_DISABLED,
            _ => T_CTRL,
        }) | 0xFF00_0000;
        ui.cmd_rrect(tr[0], tr[1], tr[2], tr[3], rad, hair, fill, stroke, stroke);
        return;
    };
    let mut c = ui.color((state << 2).wrapping_add(T_ACCENT));
    if tk.flags & TK_CUSTOM == 0 {
        c = grad_at(tk, t);
    }
    ui.cmd_rrect(tr[0], tr[1], tr[2], tr[3], rad, hair, c | 0xFF00_0000, stroke, stroke);
    ui.draw_color(0xC0FF_FFFF);
    ui.draw_frame(tr[0] + hair, tr[1] + hair, tr[2] - hair * 2, tr[3] - hair * 2, rad - hair, hair);
}

/// sty_angle(sp): dial (control fill, hairline), ticks every 90 degrees, accent radius line
/// and knob; round focus visual when s[0] = 1.  f: degrees (counter-clockwise)
fn angle(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let state = sp.state;
    // geometry: circle in the centre of the placed rectangle
    let rad = (r[2].min(r[3]) << 7).wrapping_sub(256); // radius fx = side / 2, 1 px for the edge
    let cx = (r[0] << 8).wrapping_add(r[2] << 7);
    let cy = (r[1] << 8).wrapping_add(r[3] << 7);
    // .sag_pt: the centre + dist * (cos, -sin)
    let pt = |d: i32, cs: (f64, f64)| {
        let x = cvtd(d as f64 * cs.0).wrapping_add(cx);
        let y = cy.wrapping_sub(cvtd(d as f64 * cs.1));
        (x, y)
    };
    // dial: control fill + border
    let c = ui.color((state << 2).wrapping_add(T_CTRL));
    ui.draw_color(c);
    ui.draw_circle(cx, cy, rad, 0);
    let c = ui.color(T_CTRL_STROKE2);
    ui.draw_color(c);
    let t = ui.hair() << 8;
    ui.draw_circle(cx, cy, rad.wrapping_sub(((t as u32) >> 1) as i32), t);
    // ticks every 90 degrees (tertiary text colour)
    let c = ui.color(T_TEXT3);
    ui.draw_color(c);
    let mut a = 0;
    while a < 360 {
        let cs = deg_sincos(a as f64);
        let inner = rad.wrapping_sub(ui.dp_fx(6 * 256)); // inner radius
        let outer = rad.wrapping_sub(ui.dp_fx(2 * 256)); // outer radius
        let (x0, y0) = pt(inner, cs);
        let w = ui.hair() << 8;
        let (x1, y1) = pt(outer, cs);
        ui.draw_line(x0, y0, x1, y1, w);
        a += 90;
    }
    // radius line and knob in the accent colour
    let c = ui.color((state << 2).wrapping_add(T_ACCENT));
    ui.draw_color(c);
    let cs = deg_sincos(sp.f);
    let kd = rad.wrapping_sub(ui.dp_fx(7 * 256)); // knob distance
    let (kx, ky) = pt(kd, cs);
    let w = ui.dp_fx(2 * 256);
    ui.draw_line(cx, cy, kx, ky, w);
    let r3 = ui.dp_fx(3 * 256);
    ui.draw_circle(cx, cy, r3, 0);
    let kr = ui.dp_fx(if state == 1 { 6 * 256 } else { 5 * 256 });
    ui.draw_circle(kx, ky, kr, 0);
    // focus ring: a 1 DIP circle with a 1 DIP gap outside the dial
    if sp.s[0] != 0 {
        // (a round frame: rendered like every focus visual, identical in partial redraws)
        let c = ui.color(T_FOCUS_OUT);
        ui.draw_color(c);
        let t = ui.dp(1).max(ui.hair());
        let d = ui.dp(1).wrapping_add(t); // distance outside the dial
        let side = r[2].min(r[3]);
        let x = (r[2].wrapping_sub(side) >> 1).wrapping_add(r[0]).wrapping_sub(d);
        let y = (r[3].wrapping_sub(side) >> 1).wrapping_add(r[1]).wrapping_sub(d);
        let s = side.wrapping_add(d.wrapping_mul(2));
        ui.draw_frame(x, y, s, s, ((s as u32) >> 1) as i32, t);
    }
}

/// sty_pan_size(sp) -> the thumbnail (img) fitted into 160 DIP, else 160 x 120; at least 8 x 8
fn pan_size(ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
    let mut w = ui.dp(PAN_NAT_W);
    let mut h = ui.dp(PAN_NAT_H);
    if let Some(sf) = sp.img {
        let (sw, sh) = (sf.w, sf.h);
        if sw > 0 && sh > 0 {
            if sw >= sh {
                // wide: w = 160, h = 160 * h / w
                h = w.wrapping_mul(sh) / sw;
            } else {
                // tall: h = 160, w = 160 * w / h
                let nw = sw.wrapping_mul(w) / sh;
                h = w;
                w = nw;
            }
            if w < 8 {
                w = 8;
            }
            if h < 8 {
                h = 8;
            }
        }
    }
    (w, h)
}

/// sty_pan(sp): framed thumbnail (img or None), crosshair and marker ring at the point value,
/// value2 (px)
fn pan(ui: &mut Ui, sp: &Sp) {
    let r = sp.rect();
    let (px, py) = (sp.value, sp.value2);
    // frame: control fill, hairline border, 3 DIP corners
    let c = ui.color((sp.state << 2).wrapping_add(T_CTRL));
    ui.draw_color(c);
    let rad = ui.dp(3);
    ui.draw_rrect(r[0], r[1], r[2], r[3], rad);
    let c = ui.color(T_CTRL_STROKE);
    ui.draw_color(c);
    let hair = ui.hair();
    ui.draw_frame(r[0], r[1], r[2], r[3], rad, hair);
    // thumbnail inside the border
    let h = ui.hair();
    let (ix, iy) = (r[0].wrapping_add(h), r[1].wrapping_add(h));
    let (iw, ih) = (r[2].wrapping_sub(h).wrapping_sub(h), r[3].wrapping_sub(h).wrapping_sub(h));
    ui.draw_clip(ix, iy, iw, ih);
    if let Some(sf) = sp.img {
        ui.draw_image(sf, sp.img_ver, ix, iy, iw, ih);
    }
    // crosshair: a light 3 px band with a dark 1 px line on it
    ui.draw_color(0xB0FF_FFFF);
    ui.draw_rect(px.wrapping_sub(1), r[1], 3, r[3]);
    ui.draw_rect(r[0], py.wrapping_sub(1), r[2], 3);
    ui.draw_color(0xE000_0000);
    ui.draw_rect(px, r[1], 1, r[3]);
    ui.draw_rect(r[0], py, r[2], 1);
    // marker ring: white outside, black inside
    let (fx, fy) = ((px << 8).wrapping_add(128), (py << 8).wrapping_add(128));
    ui.draw_color(0xFFFF_FFFF);
    let k = ui.dp_fx(5 * 256);
    ui.draw_circle(fx, fy, k, 2 * 256);
    ui.draw_color(0xFF00_0000);
    let k = ui.dp_fx(6 * 256);
    ui.draw_circle(fx, fy, k.wrapping_add(128), 256);
    ui.draw_unclip();
}
