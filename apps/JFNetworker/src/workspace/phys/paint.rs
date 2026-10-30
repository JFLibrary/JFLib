// paint -- the physical view at render time (a DgPaint reading the scene through a Weak to Net)
//
// The outline of the place shown (a site's area, a building's floor plan, a room's walls, a
// rack's frame with its units numbered from 1 at the bottom) and the tray beside it come first,
// then the places in it (buildings and rooms as plans, floors as bands with their rooms drawn
// small, racks dark with how full they are, desks), the cables as straight lines in their
// kind's colour and pattern with their length at the middle, then the devices (their symbol,
// the name below; in a rack a box over its units with name and model) and the ways out to other
// places.  A cable over its limit or with optics that do not fit is not marked by colour alone:
// it is dashed, carries a warning triangle with "!" at its middle and its label says why; a
// place or device it starts from gets the triangle too.

use alloc::rc::Weak;
use alloc::string::String;
use core::cell::RefCell;

use gfx::dash::{DASH_DASH, DASH_DOT, LCAP_FLAT};
use jfn_model::PlaceKind;
use ui::*;

use super::scene::{Level, SItem, Scene, What, RACK_PAD, RACK_W, RACK_X, RU};
use crate::app::Net;
use crate::workspace::canvas::paint::{cable_color, cable_style};
use crate::workspace::icons::Symbols;

pub(crate) struct Painter {
    pub net: Weak<RefCell<Net>>,
}

fn fl(v: f32) -> i32 {
    let t = v as i32;
    if (t as f32) > v { t - 1 } else { t }
}

// px x, y, w, h of a world rectangle
fn px_rect(f: &DgFrame, r: [f32; 4]) -> [i32; 4] {
    let q = f.vp.rect_to_screen(r);
    let (x0, y0) = (fl(q[0] + 0.5), fl(q[1] + 0.5));
    let (x1, y1) = (fl(q[0] + q[2] + 0.5), fl(q[1] + q[3] + 0.5));
    [x0, y0, (x1 - x0).max(1), (y1 - y0).max(1)]
}

fn opaque(c: u32) -> u32 {
    c | 0xFF00_0000
}

// the px rectangle x0, y0, x1, y1 touches the clip (2 px for anti-aliasing); every dirty
// rectangle must paint the same pixels, so labels outside their node count too
fn touches(g: &Gfx, r: [i32; 4]) -> bool {
    let c = g.dc.clip;
    r[2] + 2 > c[0] && r[0] - 2 < c[2] && r[3] + 2 > c[1] && r[1] - 2 < c[3]
}

// text at px x, y (baseline) in a style, clipped to a width (no clip: w 0)
fn label(g: &mut Gfx, f: &DgFrame, ts: u32, s: &str, x: i32, y: i32, color: u32) {
    let (font, size) = f.font(ts);
    if size < 5 << 8 {
        return;
    }
    g.text.set_font(font, size);
    g.dc.set_color(color);
    g.text.draw(g.dc, g.sf, x, y, s.as_bytes());
}

fn text_w(g: &mut Gfx, f: &DgFrame, ts: u32, s: &str) -> i32 {
    let (font, size) = f.font(ts);
    g.text.set_font(font, size);
    g.text.width(s)
}

// a warning triangle with "!" whose box is x, y, s, s (px)
fn warn_mark(g: &mut Gfx, f: &DgFrame, x: i32, y: i32, s: i32) {
    let fill = opaque(f.color(T_WARN));
    {
        let sh = g.shapes();
        sh.dc.set_color(fill);
        sh.path.begin(sh.dc);
        sh.path.move_f(x as f32 + s as f32 * 0.5, y as f32);
        sh.path.line_f((x + s) as f32, (y + s) as f32);
        sh.path.line_f(x as f32, (y + s) as f32);
        sh.path.close();
        sh.path.fill(sh.ras, sh.dc, sh.sf, 0);
    }
    let (font, _) = f.font(TS_CAPTION);
    g.text.set_font(font, (s * 3 / 4).max(6) << 8);
    let tw = g.text.width("!");
    g.dc.set_color(0xFF1D_2228);
    g.text.draw(g.dc, g.sf, x + (s - tw) / 2, y + s - s / 8, b"!");
}

impl DgPaint for Painter {
    fn paint(&self, g: &mut Gfx, f: &DgFrame) {
        let Some(net) = self.net.upgrade() else { return };
        let Ok(n) = net.try_borrow() else { return };
        let sc = &n.phys.scene;
        let c = g.dc.clip;
        g.dc.set_color(f.bg);
        g.dc.fill(g.sf, c[0], c[1], c[2] - c[0], c[3] - c[1]);
        frame(g, f, sc);
        // places first (the order of the scene: lower ones first)
        for it in sc.items.iter().filter(|it| matches!(it.what, What::Place(_))) {
            place(g, f, sc, it);
        }
        cables(g, f, sc);
        for it in sc.items.iter().filter(|it| !matches!(it.what, What::Place(_))) {
            match it.what {
                What::Device => device(g, f, sc, &n.syms, it),
                What::Exit(_) => exit(g, f, it),
                What::Place(_) => {}
            }
        }
    }
}

// the outline of the place shown, the tray, a rack's frame and units
fn frame(g: &mut Gfx, f: &DgFrame, sc: &Scene) {
    let s = f.vp.scale() as f32;
    let hair = ((f.vp.dpi + 24) / 96).max(1);
    let text2 = f.color(T_TEXT2);
    if let Some(r) = sc.frame {
        let q = px_rect(f, r);
        match sc.level {
            Level::Rack => {
                let mut sh = g.shapes();
                sh.dc.set_color(opaque(color_mix(f.color(T_TEXT), f.bg, if f.dark { 200 } else { 60 })));
                sh.fill_rrect(q[0], q[1], q[2], q[3], (3.0 * s) as i32);
            }
            _ => {
                let fill = color_mix(opaque(f.color(T_CARD)), f.bg, 96);
                let mut sh = g.shapes();
                sh.dc.set_color(fill);
                sh.fill_rrect(q[0], q[1], q[2], q[3], 0);
                sh.dc.set_color(opaque(f.color(T_TEXT2)));
                let wall = if sc.level == Level::Room { ((4.0 * s) as i32).max(2) } else { hair * 2 };
                sh.stroke_rrect(q[0], q[1], q[2], q[3], 0, wall);
            }
        }
        if sc.level != Level::Rack {
            let (_, size) = f.font(TS_CAPTION);
            label(g, f, TS_CAPTION, &sc.frame_name, q[0], q[1] - (size >> 8) / 2, text2);
        }
    }
    if let (Some(geo), Some(r)) = (sc.rack, sc.frame) {
        // the units: a line between two, the numbers on the left rail
        let line = opaque(color_mix(f.color(T_TEXT), f.bg, if f.dark { 150 } else { 110 }));
        let num = opaque(color_mix(f.color(T_TEXT), f.bg, if f.dark { 60 } else { 200 }));
        for u in 0..=geo.units {
            let y = r[1] + RACK_PAD + u as f32 * RU;
            let a = f.vp.to_screen([r[0] + RACK_X, y]);
            let b = f.vp.to_screen([r[0] + RACK_X + RACK_W, y]);
            g.dc.set_color(line);
            g.dc.fill(g.sf, fl(a[0]), fl(a[1]), (fl(b[0]) - fl(a[0])).max(1), hair);
            if u < geo.units {
                let unit = geo.units - u;
                let t = alloc::format!("{}", unit);
                let tw = text_w(g, f, TS_CAPTION, &t);
                let p = f.vp.to_screen([r[0] + RACK_X - 4.0, y + RU * 0.78]);
                label(g, f, TS_CAPTION, &t, fl(p[0]) - tw, fl(p[1]), num);
            }
        }
        let p = f.vp.to_screen([r[0], r[1] - 6.0]);
        label(g, f, TS_BODY_STRONG, &sc.frame_name, fl(p[0]), fl(p[1]), f.color(T_TEXT));
    }
    if let Some(r) = sc.tray {
        let q = px_rect(f, r);
        let mut sh = g.shapes();
        sh.dc.set_color(opaque(color_mix(f.color(T_CARD), f.bg, 160)));
        sh.fill_rrect(q[0], q[1], q[2], q[3], (4.0 * s) as i32);
        sh.dc.set_color(opaque(f.color(T_CTRL_STROKE)));
        sh.stroke_rrect(q[0], q[1], q[2], q[3], (4.0 * s) as i32, hair);
        drop(sh);
        let (_, size) = f.font(TS_CAPTION);
        label(g, f, TS_CAPTION, &sc.tray_name, q[0] + (6.0 * s) as i32, q[1] + (size >> 8) + (4.0 * s) as i32, text2);
    }
}

// a place: plan, band, rack, desk
fn place(g: &mut Gfx, f: &DgFrame, sc: &Scene, it: &SItem) {
    let What::Place(kind) = it.what else { return };
    let s = f.vp.scale() as f32;
    let hair = ((f.vp.dpi + 24) / 96).max(1);
    let q = px_rect(f, it.rect);
    // the box and, for a narrow one, the name below it
    let below = if matches!(kind, PlaceKind::Rack | PlaceKind::Desk) { text_w(g, f, TS_BODY_STRONG, &it.name).max(text_w(g, f, TS_CAPTION, &it.sub)) + 8 } else { 0 };
    let (_, bs) = f.font(TS_BODY_STRONG);
    if !touches(g, [q[0].min(q[0] + (q[2] - below) / 2), q[1] - 2, (q[0] + q[2]).max(q[0] + (q[2] + below) / 2), q[1] + q[3] + if below > 0 { (bs >> 8) * 3 + 12 } else { 2 }]) {
        return;
    }
    let text = f.color(T_TEXT);
    let text2 = f.color(T_TEXT2);
    let card = opaque(f.color(T_CARD));
    let rad = ((3.0 * s) as i32).max(1);
    let (fill, stroke) = match kind {
        PlaceKind::Site => (color_mix(card, 0xFF7F_A36B, if f.dark { 40 } else { 50 }), opaque(f.color(T_CTRL_STROKE))),
        PlaceKind::Building => (color_mix(card, 0xFF8C_96A3, if f.dark { 50 } else { 60 }), opaque(f.color(T_TEXT2))),
        PlaceKind::Floor => (card, opaque(f.color(T_TEXT2))),
        PlaceKind::Room => (card, opaque(f.color(T_TEXT2))),
        PlaceKind::Rack => (opaque(color_mix(f.color(T_TEXT), f.bg, if f.dark { 200 } else { 70 })), opaque(f.color(T_TEXT))),
        PlaceKind::Desk => (color_mix(card, 0xFFC8_A46E, if f.dark { 60 } else { 90 }), opaque(color_mix(0xFF8A_6A3A, f.bg, 40))),
    };
    {
        let mut sh = g.shapes();
        sh.dc.set_color(fill);
        sh.fill_rrect(q[0], q[1], q[2], q[3], rad);
        sh.dc.set_color(stroke);
        let t = if kind == PlaceKind::Room && sc.level == Level::Floor { hair * 2 } else { hair };
        sh.stroke_rrect(q[0], q[1], q[2], q[3], rad, t);
    }
    // a rack: how full it is, as a bar along its bottom
    if let Some((used, units)) = it.fill
        && units > 0
    {
        let bar = ((3.0 * s) as i32).max(2);
        let w = (q[2] - 2 * hair) * used as i32 / units as i32;
        g.dc.set_color(opaque(f.color(T_ACCENT)));
        g.dc.fill(g.sf, q[0] + hair, q[1] + q[3] - bar - hair, w.max(0), bar);
    }
    // the rooms inside a floor band
    for (r, name) in &it.inner {
        let rq = px_rect(f, *r);
        {
            let mut sh = g.shapes();
            sh.dc.set_color(color_mix(card, f.bg, 60));
            sh.fill_rrect(rq[0], rq[1], rq[2], rq[3], 0);
            sh.dc.set_color(opaque(f.color(T_CTRL_STROKE)));
            sh.stroke_rrect(rq[0], rq[1], rq[2], rq[3], 0, hair);
        }
        let (_, size) = f.font(TS_CAPTION);
        if rq[2] > (24.0 * s) as i32 {
            label(g, f, TS_CAPTION, name, rq[0] + (3.0 * s) as i32, rq[1] + (size >> 8) + (2.0 * s) as i32, text2);
        }
    }
    // the name inside (racks: white on dark), the second line below it; a place too narrow for
    // its name (a rack, a desk) has both below it
    let on_dark = kind == PlaceKind::Rack;
    let (c1, c2) = if on_dark { (0xFFF0_F2F4, 0xFFC8_CDD3) } else { (text, text2) };
    let (_, size) = f.font(TS_BODY_STRONG);
    let lh = (size >> 8) + (3.0 * s) as i32;
    let pad = (5.0 * s) as i32;
    let nw = text_w(g, f, TS_BODY_STRONG, &it.name);
    if matches!(kind, PlaceKind::Rack | PlaceKind::Desk) && nw + pad * 2 > q[2] {
        let sw = text_w(g, f, TS_CAPTION, &it.sub);
        let w = nw.max(sw);
        let x = q[0] + (q[2] - w) / 2;
        let y = q[1] + q[3] + (2.0 * s) as i32;
        {
            let mut sh = g.shapes();
            sh.dc.set_color((f.bg & 0x00FF_FFFF) | 0xD800_0000);
            sh.fill_rrect(x - 3, y, w + 6, lh * 2, 3);
        }
        label(g, f, TS_BODY_STRONG, &it.name, x + (w - nw) / 2, y + (size >> 8), text);
        label(g, f, TS_CAPTION, &it.sub, x + (w - sw) / 2, y + lh + (size >> 8) - 2, text2);
    } else if q[3] > lh && q[2] > (20.0 * s) as i32 {
        g.dc.clip_push(q[0], q[1], q[2], q[3]);
        let mut y = q[1] + pad + (size >> 8);
        label(g, f, TS_BODY_STRONG, &it.name, q[0] + pad, y, c1);
        y += lh;
        if q[3] > lh * 2 {
            label(g, f, TS_CAPTION, &it.sub, q[0] + pad, y, c2);
        }
        g.dc.clip_pop();
    }
    if it.warn {
        let ws = ((12.0 * s) as i32).max(8);
        warn_mark(g, f, q[0] + q[2] - ws - pad / 2, q[1] + pad / 2, ws);
    }
}

// the cables: lines in their kind's style with the length at the middle; the bad ones dashed
// with a warning triangle
fn cables(g: &mut Gfx, f: &DgFrame, sc: &Scene) {
    let s = f.vp.scale() as f32;
    let plate = (f.bg & 0x00FF_FFFF) | 0xE000_0000;
    for (i, c) in sc.cables.iter().enumerate() {
        let Some(rt) = f.geo.routes.get(i) else { continue };
        if rt.pts.len() < 2 {
            continue;
        }
        let (_, w, dash) = cable_style(c.cable);
        let w = if c.link.is_none() { w + 1.5 } else { w };
        let argb = if c.bad { opaque(f.color(T_CRIT)) } else { cable_color(c.cable, f.dark, c.down, f.bg) };
        let dash = if c.bad { DASH_DASH } else if c.cable == jfn_model::CableKind::Console { DASH_DOT } else { dash };
        if f.route_visible(rt, w + 2.0) {
            dg_stroke_edge(g, &f.vp, &rt.pts, w, argb, LCAP_FLAT, LCAP_FLAT, dash);
        }
        // the label at the middle
        let len = dg_length(&rt.pts);
        let mid = dg_point_at(&rt.pts, c.at);
        let p = f.vp.to_screen(mid);
        if len * s < 30.0 {
            continue;
        }
        let tw = text_w(g, f, TS_CAPTION, &c.label);
        let (_, size) = f.font(TS_CAPTION);
        let th = (size >> 8) + 4;
        let ws = if c.bad { th } else { 0 };
        let x = fl(p[0]) - (tw + ws) / 2;
        let y = fl(p[1]) - th / 2;
        if !touches(g, [x - 3, y, x + tw + ws + 3, y + th]) {
            continue;
        }
        {
            let mut sh = g.shapes();
            sh.dc.set_color(plate);
            sh.fill_rrect(x - 3, y, tw + ws + 6, th, 3);
        }
        if c.bad {
            warn_mark(g, f, x, y + 1, th - 2);
        }
        label(g, f, TS_CAPTION, &c.label, x + ws, y + th - 4, if c.bad { opaque(f.color(T_CRIT)) } else { f.color(T_TEXT2) });
    }
}

// a device: its symbol and its name below (in a rack: a box over its units)
fn device(g: &mut Gfx, f: &DgFrame, sc: &Scene, syms: &Symbols, it: &SItem) {
    let s = f.vp.scale() as f32;
    let q = px_rect(f, it.rect);
    let nw = text_w(g, f, TS_CAPTION, &it.name);
    let (_, cs) = f.font(TS_CAPTION);
    let ext = [q[0].min(q[0] + (q[2] - nw) / 2 - 3), q[1] - (12.0 * s) as i32, (q[0] + q[2]).max(q[0] + (q[2] + nw) / 2 + 3) + (12.0 * s) as i32, q[1] + q[3] + (cs >> 8) + 8];
    if !touches(g, ext) {
        return;
    }
    let text = f.color(T_TEXT);
    let hair = ((f.vp.dpi + 24) / 96).max(1);
    if sc.level == Level::Rack {
        // a box over its units: front panel, name, model
        let panel = opaque(color_mix(f.color(T_CARD), 0xFF5A_6169, if f.dark { 90 } else { 40 }));
        {
            let mut sh = g.shapes();
            sh.dc.set_color(if it.off { color_mix(panel, f.bg, 120) } else { panel });
            sh.fill_rrect(q[0], q[1] + hair, q[2], q[3] - 2 * hair, (2.0 * s) as i32);
            sh.dc.set_color(opaque(f.color(T_TEXT2)));
            sh.stroke_rrect(q[0], q[1] + hair, q[2], q[3] - 2 * hair, (2.0 * s) as i32, hair);
        }
        let side = q[3] - 4 * hair;
        if let Some(i) = it.sym
            && side > 6
        {
            let w = side * 4 / 3;
            syms.draw(g, i, q[0] + (4.0 * s) as i32, q[1] + 2 * hair, w, it.tint, it.off);
        }
        let (_, size) = f.font(TS_CAPTION);
        let x = q[0] + (q[3] * 4 / 3) + (10.0 * s) as i32;
        let y = q[1] + q[3] / 2 + (size >> 8) / 2 - hair;
        g.dc.clip_push(q[0], q[1], q[2], q[3]);
        label(g, f, TS_BODY_STRONG, &it.name, x, y, text);
        let nw = text_w(g, f, TS_BODY_STRONG, &it.name);
        label(g, f, TS_CAPTION, &it.sub, x + nw + (10.0 * s) as i32, y, f.color(T_TEXT2));
        g.dc.clip_pop();
        if it.warn {
            let ws = ((11.0 * s) as i32).max(8);
            warn_mark(g, f, q[0] + q[2] - ws - 4, q[1] + (q[3] - ws) / 2, ws);
        }
        return;
    }
    match it.sym {
        Some(i) => {
            let h = syms.height(i, q[2]);
            syms.draw(g, i, q[0], q[1] + (q[3] - h) / 2, q[2], it.tint, it.off);
        }
        None => {
            let mut sh = g.shapes();
            sh.dc.set_color(opaque(f.color(T_TEXT3)));
            sh.stroke_rrect(q[0], q[1], q[2], q[3], 2, hair);
        }
    }
    let (_, size) = f.font(TS_CAPTION);
    if size >= 5 << 8 {
        let tw = text_w(g, f, TS_CAPTION, &it.name);
        let x = q[0] + (q[2] - tw) / 2;
        let y = q[1] + q[3] + (size >> 8) + 1;
        {
            let mut sh = g.shapes();
            sh.dc.set_color((f.bg & 0x00FF_FFFF) | 0xD000_0000);
            sh.fill_rrect(x - 2, y - (size >> 8), tw + 4, (size >> 8) + 4, 2);
        }
        label(g, f, TS_CAPTION, &it.name, x, y, text);
    }
    if it.warn {
        let ws = ((10.0 * s) as i32).max(7);
        warn_mark(g, f, q[0] + q[2] - ws / 2, q[1] - ws / 2, ws);
    }
}

// a way out to another place: a card with an arrow and the place's name
fn exit(g: &mut Gfx, f: &DgFrame, it: &SItem) {
    if !f.visible(it.rect) {
        return;
    }
    let s = f.vp.scale() as f32;
    let hair = ((f.vp.dpi + 24) / 96).max(1);
    let q = px_rect(f, it.rect);
    {
        let mut sh = g.shapes();
        sh.dc.set_color(opaque(f.color(T_CARD)));
        sh.fill_rrect(q[0], q[1], q[2], q[3], (4.0 * s) as i32);
        sh.dc.set_color(opaque(f.color(T_ACCENT)));
        sh.stroke_rrect(q[0], q[1], q[2], q[3], (4.0 * s) as i32, hair);
    }
    let (_, size) = f.font(TS_BODY_STRONG);
    let pad = (6.0 * s) as i32;
    g.dc.clip_push(q[0], q[1], q[2], q[3]);
    let arrow: String = String::from("\u{2192} ");
    let aw = text_w(g, f, TS_BODY_STRONG, &arrow);
    label(g, f, TS_BODY_STRONG, &arrow, q[0] + pad, q[1] + pad + (size >> 8), f.color(T_ACCENT_TEXT));
    label(g, f, TS_BODY_STRONG, &it.name, q[0] + pad + aw, q[1] + pad + (size >> 8), f.color(T_TEXT));
    label(g, f, TS_CAPTION, &it.sub, q[0] + pad, q[1] + pad + (size >> 8) * 2 + (2.0 * s) as i32, f.color(T_TEXT2));
    g.dc.clip_pop();
}
