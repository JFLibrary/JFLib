// wlan -- WLAN on the canvas: the associations of the simulator as thin teal dotted radio
// lines between a station and its access point (no cables of the project: they come and go
// with the stations), the range of access points as circles, signal bars at the stations, and
// their text records
//
//   Wl (Canvas::wl)              the toggle of the ranges (Ansicht › WLAN-Reichweiten); without
//                                it the selected access points show theirs
//   key(n) -> u32                part of the canvas' version: the associations, which ranges
//                                show, the scale (the backing store repaints when they change)
//   paint_ranges(g, f, n)        (render time, first) per access point and band of its networks
//                                a circle of the band's range in the project's scale, its band and
//                                metres at the top
//   paint_assocs(g, f, n)        (render time, with the cables) the radio lines, border to border
//                                of the boxes; one out of range (the station will miss the
//                                beacons) faded, one whose handshake runs dashed
//   paint_bars(g, f, n)          (render time, after the devices, from 50 %) four bars beside the
//                                top right corner of a station: -60, -67, -75 dBm and above the
//                                edge; none filled while it is not connected
//   records(n, ui)               (after ui.diagram) text records of what is in view: "Funk:
//                                Laptop1 – AP1 (Buero, 5 GHz, -72 dBm, 576 Mbit/s)", "Reichweite:
//                                AP1, 5 GHz 35 m", "Funk: Laptop2 – nicht verbunden („Buero“)"
//   route(n, link)               the line of an association by its link id (the frames of the
//                                simulation mode, sim/anim.rs)
//
// The lines and bars do not rely on colour alone: dotted (radio), dashed while the handshake
// runs, faded out of range; the bars count.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use gfx::dash::{DASH_DASH, DASH_DOT, LCAP_FLAT};
use jfn_model::wlan::{self, Band, Role};
use jfn_model::{catalog, CableKind};
use jfn_sim::AssocView;
use ui::*;

use super::paint::{cable_color, cable_style, DEV_H, DEV_W};
use crate::app::tr::{t, tf};
use crate::app::Net;

/// the toggle of the ranges
#[derive(Default)]
pub(crate) struct Wl {
    /// every access point shows its range
    pub ranges: bool,
}

/// the boxes of the devices by id (a member of a collapsed group: its cluster's, G1)
fn rect_of(n: &Net, dev: u32) -> Option<[f32; 4]> {
    let d = n.doc.proj.device(dev)?;
    match n.canvas.scene.hidden.get(&dev) {
        Some(&c) => n.canvas.scene.rect(c as u64),
        None => Some([d.x as f32, d.y as f32, DEV_W, DEV_H]),
    }
}

fn centre(r: [f32; 4]) -> [f32; 2] {
    [r[0] + r[2] * 0.5, r[1] + r[3] * 0.5]
}

// where the line from the centre of r towards `to` leaves the box
fn border(r: [f32; 4], to: [f32; 2]) -> [f32; 2] {
    let c = centre(r);
    let (dx, dy) = (to[0] - c[0], to[1] - c[1]);
    let mut k = f32::MAX;
    if dx.abs() > 1e-4 {
        k = k.min(r[2] * 0.5 / dx.abs());
    }
    if dy.abs() > 1e-4 {
        k = k.min(r[3] * 0.5 / dy.abs());
    }
    if !(k < f32::MAX) {
        return c;
    }
    let k = k.min(1.0);
    [c[0] + dx * k, c[1] + dy * k]
}

/// the line of an association between the two boxes (world units)
fn line(n: &Net, a: &AssocView) -> Option<[[f32; 2]; 2]> {
    let (rs, ra) = (rect_of(n, a.sta)?, rect_of(n, a.ap)?);
    // both in one cluster: no line (G1)
    if rs == ra {
        return None;
    }
    let (cs, ca) = (centre(rs), centre(ra));
    Some([border(rs, ca), border(ra, cs)])
}

/// the route of an association's frames: from the station to the access point (dir 0)
pub(crate) fn route(n: &Net, link: u32) -> Option<Vec<[f32; 2]>> {
    let a = n.doc.sim.wlan.iter().find(|a| a.id == link)?;
    line(n, a).map(|l| l.to_vec())
}

/// the bands of an AP's valid networks, the widest range first
fn ap_bands(n: &Net, dev: u32) -> Vec<Band> {
    let Some(d) = n.doc.proj.device(dev) else { return Vec::new() };
    let Some(m) = catalog::model(&d.model) else { return Vec::new() };
    if wlan::role(m) != Role::Ap {
        return Vec::new();
    }
    let have = wlan::ap_bands(m);
    let mut v: Vec<Band> = d.wlan.as_ref().map(|w| w.nets.iter().filter(|x| have & x.band.bit() != 0 && wlan::check_net(x).is_none()).map(|x| x.band).collect()).unwrap_or_default();
    v.sort_by_key(|b| b.index());
    v.dedup();
    v
}

/// the access points whose ranges show: all of them with the toggle, else the selected ones
/// (not those a collapsed group hides, G1)
fn shown(n: &Net) -> Vec<u32> {
    let all = n.canvas.wl.ranges;
    let hidden = &n.canvas.scene.hidden;
    n.doc.proj.devices.iter().filter(|d| all || n.canvas.st.sel.contains(&(d.id as u64))).filter(|d| !hidden.contains_key(&d.id) && !ap_bands(n, d.id).is_empty()).map(|d| d.id).collect()
}

/// part of the canvas' version
pub(crate) fn key(n: &Net) -> u32 {
    let mut h: u32 = 0x811C_9DC5 ^ n.doc.sim.wlan_key;
    for x in shown(n).into_iter().chain([n.canvas.wl.ranges as u32, wlan::dip_per_m(&n.doc.proj)]) {
        h = (h ^ x).wrapping_mul(0x0100_0193);
    }
    h
}

/// the radius of a band's range in world units
fn radius(n: &Net, dev: u32, band: Band) -> f32 {
    let m = n.doc.proj.device(dev).and_then(|d| catalog::model(&d.model));
    m.map_or(0.0, |m| wlan::range_m(m, band) as f32 * wlan::dip_per_m(&n.doc.proj) as f32)
}

/// the colour of the radio (theme, faded)
fn radio(f: &DgFrame, faded: bool) -> u32 {
    cable_color(CableKind::Wireless, f.dark, faded, f.bg)
}

fn with_alpha(argb: u32, a: u32) -> u32 {
    (argb & 0x00FF_FFFF) | a.min(255) << 24
}

/// a rate in Mbit/s with one decimal in the language's decimal mark ("576,0")
pub(crate) fn mbit(kbps: u32) -> String {
    format!("{}{}{}", kbps / 1000, if crate::app::tr::en() { '.' } else { ',' }, kbps % 1000 / 100)
}

/// the circles of the ranges
pub(crate) fn paint_ranges(g: &mut Gfx, f: &DgFrame, n: &Net) {
    let s = f.vp.scale() as f32;
    let (font, size) = f.font(TS_CAPTION);
    for dev in shown(n) {
        let Some(r) = rect_of(n, dev) else { continue };
        let c = centre(r);
        for band in ap_bands(n, dev).into_iter().rev() {
            let rad = radius(n, dev, band);
            if !f.visible([c[0] - rad, c[1] - rad, rad * 2.0, rad * 2.0]) {
                continue;
            }
            let col = radio(f, false);
            let p = f.vp.to_screen(c);
            {
                let mut sh = g.shapes();
                sh.dc.set_color(with_alpha(col, if f.dark { 22 } else { 16 }));
                sh.fill_circle_fx((p[0] * 256.0) as i32, (p[1] * 256.0) as i32, (rad * s * 256.0) as i32);
            }
            // the outline dashed: 72 points, 5 degrees apart
            let (cs, sn) = (0.996_194_7f32, 0.087_155_74f32);
            let (mut x, mut y) = (1.0f32, 0.0f32);
            let mut pts: Vec<[f32; 2]> = Vec::with_capacity(73);
            for _ in 0..=72 {
                pts.push([c[0] + rad * x, c[1] + rad * y]);
                (x, y) = (x * cs - y * sn, x * sn + y * cs);
            }
            dg_stroke_edge(g, &f.vp, &pts, 1.0, with_alpha(col, 200), LCAP_FLAT, LCAP_FLAT, DASH_DASH);
            // its band and metres at the top
            if size >= 4 << 8 {
                let m = n.doc.proj.device(dev).and_then(|d| catalog::model(&d.model)).map_or(0, |m| wlan::range_m(m, band));
                let label = format!("{} · {} m", t(band.title()), m);
                g.text.set_font(font, size);
                let tw = g.text.width(&label);
                let q = f.vp.to_screen([c[0], c[1] - rad]);
                g.dc.set_color(col);
                g.text.draw(g.dc, g.sf, q[0] as i32 - tw / 2, q[1] as i32 + g.text.ascent() + 2, label.as_bytes());
            }
        }
    }
}

/// the radio lines of the associations
pub(crate) fn paint_assocs(g: &mut Gfx, f: &DgFrame, n: &Net) {
    let (_, w, _) = cable_style(CableKind::Wireless);
    for a in n.doc.sim.wlan.iter() {
        let Some(l) = line(n, a) else { continue };
        let bb = [l[0][0].min(l[1][0]), l[0][1].min(l[1][1]), (l[0][0] - l[1][0]).abs(), (l[0][1] - l[1][1]).abs()];
        if !f.visible([bb[0] - 4.0, bb[1] - 4.0, bb[2] + 8.0, bb[3] + 8.0]) {
            continue;
        }
        let dash = if a.authorized { DASH_DOT } else { DASH_DASH };
        dg_stroke_edge(g, &f.vp, &l, w.max(1.2), radio(f, a.lost), LCAP_FLAT, LCAP_FLAT, dash);
    }
}

/// the bars of a signal: 4 from -60 dBm, 3 from -67, 2 from -75, else 1
pub(crate) fn bars(dbm: i32) -> u32 {
    if dbm >= -60 {
        4
    } else if dbm >= -67 {
        3
    } else if dbm >= -75 {
        2
    } else {
        1
    }
}

/// the signal bars of the stations
pub(crate) fn paint_bars(g: &mut Gfx, f: &DgFrame, n: &Net, lod: bool) {
    if f.vp.zoom < 0.5 || lod {
        return;
    }
    let s = f.vp.scale() as f32;
    let on = radio(f, false);
    let off = color_mix(f.color(T_TEXT3) | 0xFF00_0000, f.bg | 0xFF00_0000, 120);
    let assoc_of = |dev: u32| n.doc.sim.wlan.iter().find(|a| a.sta == dev);
    for d in &n.canvas.scene.devs {
        let Some(m) = n.doc.proj.device(d.id).and_then(|x| catalog::model(&x.model)) else { continue };
        if wlan::role(m) != Role::Station {
            continue;
        }
        let a = assoc_of(d.id);
        let joins = n.doc.proj.device(d.id).and_then(|x| x.wlan.as_ref()).is_some_and(|w| w.join.is_some());
        if a.is_none() && !joins {
            continue;
        }
        let lit = match a {
            Some(a) if a.authorized && !a.lost => bars(a.dbm),
            _ => 0,
        };
        let r = d.rect;
        let at = [r[0] + r[2] + 2.0, r[1] + 2.0];
        if !f.visible([at[0], at[1], 12.0, 10.0]) {
            continue;
        }
        let mut sh = g.shapes();
        for k in 0..4u32 {
            let h = 3.0 + 2.0 * k as f32;
            let q = f.vp.rect_to_screen([at[0] + 3.0 * k as f32, at[1] + 9.0 - h, 2.0, h]);
            sh.dc.set_color(if k < lit { on } else { off });
            let (x, y) = (q[0] as i32, q[1] as i32);
            let (w, hh) = ((q[2] as i32).max(1), (q[3] as i32).max(1));
            sh.fill_rrect(x, y, w, hh, ((0.5 * s) as i32).min(1));
        }
    }
}

/// the text records of the associations and ranges in view (screen readers, scripts)
pub(crate) fn records(n: &Net, ui: &mut Ui) {
    let vp = n.canvas.st.vp;
    let r = vp.rect;
    let inside = |q: [f32; 4]| q[0] + q[2] >= r[0] as f32 && q[0] <= (r[0] + r[2]) as f32 && q[1] + q[3] >= r[1] as f32 && q[1] <= (r[1] + r[3]) as f32;
    let name = |dev: u32| n.doc.proj.device(dev).map_or_else(|| format!("{}", dev), |d| d.name.clone());
    for a in n.doc.sim.wlan.iter() {
        let Some(l) = line(n, a) else { continue };
        let m = [(l[0][0] + l[1][0]) * 0.5, (l[0][1] + l[1][1]) * 0.5];
        let q = vp.rect_to_screen([m[0] - 8.0, m[1] - 8.0, 16.0, 16.0]);
        if !inside(q) {
            continue;
        }
        let state = if a.lost {
            String::from(t("außer Reichweite"))
        } else if a.authorized {
            tf("{} dBm, {} Mbit/s", &[&format!("{}", a.dbm), &mbit(a.kbps)])
        } else {
            String::from(t("Handshake läuft"))
        };
        let text = tf("Funk: {} – {} ({}, {}, {})", &[&name(a.sta), &name(a.ap), &a.ssid, t(a.band.title()), &state]);
        ui.last_set([q[0] as i32, q[1] as i32, q[2] as i32, q[3] as i32], 0);
        ui.sem(K_TEXT, 0, Some(&text), 0);
    }
    for dev in shown(n) {
        let Some(rc) = rect_of(n, dev) else { continue };
        let q = vp.rect_to_screen(rc);
        if !inside(q) {
            continue;
        }
        let m = n.doc.proj.device(dev).and_then(|d| catalog::model(&d.model));
        let parts: Vec<String> = ap_bands(n, dev).into_iter().map(|b| format!("{} {} m", t(b.title()), m.map_or(0, |m| wlan::range_m(m, b)))).collect();
        ui.last_set([q[0] as i32, (q[1] + q[3]) as i32, q[2] as i32, 4], 0);
        ui.sem(K_TEXT, 0, Some(&tf("Reichweite: {}, {}", &[&name(dev), &parts.join(", ")])), 0);
    }
    // stations that look for their network (their bars stay empty)
    for d in &n.canvas.scene.devs {
        if n.doc.sim.wlan.iter().any(|a| a.sta == d.id) {
            continue;
        }
        let Some(j) = n.doc.proj.device(d.id).and_then(|x| x.wlan.as_ref()).and_then(|w| w.join.as_ref()) else { continue };
        let q = vp.rect_to_screen([d.rect[0] + d.rect[2] + 2.0, d.rect[1] + 2.0, 12.0, 10.0]);
        if !inside(q) {
            continue;
        }
        ui.last_set([q[0] as i32, q[1] as i32, q[2] as i32, q[3] as i32], 0);
        ui.sem(K_TEXT, 0, Some(&tf("Funk: {} – nicht verbunden („{}“)", &[&name(d.id), &j.ssid])), 0);
    }
}
