// jfOffice: Value sliders, angle controls and image panning.
// Uses jfguiLib drawing primitives; local implementations derived from the
// existing native widget contract. Reference geometry is implemented locally.

use ui::consts::*;
use ui::{Sp, StyleValue, Ui, cvtd, deg_sincos, grad_at};

use crate::Office;

const THUMB_DIP: i32 = 20; // slider thumb diameter
const TRACK_DIP: i32 = 6; // standard track thickness
const GTRACK_DIP: i32 = 12; // gradient track thickness
const ANG_NAT: i32 = 64; // natural size of the angle dial (DIP)
const PAN_NAT_W: i32 = 160; // natural size of the pan control without a thumbnail
const PAN_NAT_H: i32 = 120;

impl StyleValue for Office {
    fn value_slider_size(&self, ui: &mut Ui, sp: &mut Sp) -> (i32, i32) {
        // 200 x 24, 24 x 200 with RG_VERT
        let (w, h) = (ui.dp(200), ui.dp(24));
        if sp.flags & RG_VERT != 0 {
            (h, w)
        } else {
            (w, h)
        }
    }
    fn value_slider(&self, ui: &mut Ui, sp: &Sp) {
        value_slider(ui, sp)
    }
    fn value_slider_thumb(&self, ui: &mut Ui) -> i32 {
        // thumb diameter (px): the travel is the length minus it
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
        (
            r[0].wrapping_add(half).wrapping_add(off),
            (((r[3] as u32) >> 1) as i32).wrapping_add(r[1]),
        )
    } else {
        (
            (((r[2] as u32) >> 1) as i32).wrapping_add(r[0]),
            r[1].wrapping_add(r[3]).wrapping_sub(half).wrapping_sub(off),
        )
    };
    match sp.track {
        None => {
            // ---- standard track: grey fill over the travel, accent up to the thumb
            let th = ui.dp(TRACK_DIP);
            let mut p = pill(r, vert, half, travel, cx, cy, th);
            let c = ui.color(T_TRACK);
            ui.draw_color(c);
            let rad = ((th as u32) >> 1) as i32;
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
            // ---- gradient track (12 DIP pill around the travel) with a hairline frame
            let th = ui.dp(GTRACK_DIP);
            let mut p = pill(r, vert, half, travel, cx, cy, th);
            let e = ((th as u32) >> 1) as i32; // the pill reaches half its thickness past the travel ends
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
            let hair = ui.dp(1);
            ui.draw_frame(p[0], p[1], p[2], p[3], e, hair);
        }
    }
    // knob: outline (highlight while hovered / held), knob fill
    let c = ui.color(if state == 1 || state == 2 {
        T_ACCENT
    } else {
        T_STRONG_STROKE
    });
    ui.draw_color(c);
    let (fx, fy) = ((cx << 8).wrapping_add(128), (cy << 8).wrapping_add(128));
    let k = ui.dp_fx(10 * 256);
    ui.draw_circle(fx, fy, k, 0);
    let c = ui.color(if state == 2 { T_CTRL_PRESSED } else { T_KNOB }) | 0xFF00_0000;
    ui.draw_color(c);
    let k = ui.dp_fx(9 * 256);
    ui.draw_circle(fx, fy, k, 0);
    // gradient tracks: a dot of the colour under the thumb
    let Some(tk) = sp.track else { return };
    let mut c = ui.color((state << 2).wrapping_add(T_ACCENT));
    if tk.flags & TK_CUSTOM == 0 {
        c = grad_at(tk, t);
    }
    ui.draw_color(c);
    let dr = ui.dp_fx(5 * 256);
    ui.draw_circle(fx, fy, dr, 0);
    // gradient: a hairline ring keeps light colours visible on the knob
    let c = ui.color(T_CTRL_STROKE2);
    ui.draw_color(c);
    ui.draw_circle(fx, fy, dr.wrapping_sub(128), 256);
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
    let t = ui.dp(1) << 8;
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
        let w = ui.dp(1) << 8;
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
    // focus ring: a circle 3 DIP outside the dial
    if sp.s[0] != 0 {
        // (a round frame: rendered like every focus visual, identical in partial redraws)
        let c = (ui.color(T_ACCENT) & 0x00FF_FFFF) | 0x8C00_0000;
        ui.draw_color(c);
        let d = ui.dp(3); // distance outside the dial
        let side = r[2].min(r[3]);
        let x = (r[2].wrapping_sub(side) >> 1)
            .wrapping_add(r[0])
            .wrapping_sub(d);
        let y = (r[3].wrapping_sub(side) >> 1)
            .wrapping_add(r[1])
            .wrapping_sub(d);
        let s = side.wrapping_add(d.wrapping_mul(2));
        ui.draw_frame(x, y, s, s, ((s as u32) >> 1) as i32, d); // band width = distance outside the dial
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
    // frame: control fill, hairline border, 4 DIP corners
    let c = ui.color((sp.state << 2).wrapping_add(T_CTRL));
    ui.draw_color(c);
    let rad = ui.dp(4);
    ui.draw_rrect(r[0], r[1], r[2], r[3], rad);
    let c = ui.color(T_CTRL_STROKE2);
    ui.draw_color(c);
    let rad = ui.dp(4);
    let hair = ui.dp(1);
    ui.draw_frame(r[0], r[1], r[2], r[3], rad, hair);
    // thumbnail inside the border
    let h = ui.dp(1);
    let (ix, iy) = (r[0].wrapping_add(h), r[1].wrapping_add(h));
    let (iw, ih) = (
        r[2].wrapping_sub(h).wrapping_sub(h),
        r[3].wrapping_sub(h).wrapping_sub(h),
    );
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
