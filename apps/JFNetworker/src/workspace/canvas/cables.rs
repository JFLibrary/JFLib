// cables -- what the canvas does with cables (and the power command of the device menu):
// route, kind, disconnecting, waypoints, the × at the hovered cable, the tooltips of cables and
// port dots
//
//   states(n, ui)                    enabled / checked of the cable and power commands
//   set_route(n, route)              Gerade / Rechtwinklig / Kurve for the selected cables
//   set_kind(n, kind)                the kind of the selected cables (None: "Automatisch", the
//                                    right one for the ports); kinds that do not plug in are off
//   disconnect_selection(n)          Trennen: the selected cables go
//   toggle_power(n) / set_power(n, devs, on)   the selected devices on (when one is off) or
//                                    off; given devices on or off
//   waypoint(n, ..) / waypoint_delete(n, ..)   DEv::Waypoint / WaypointDelete: the drag moves
//                                    the project's waypoint live and is one undo step when it
//                                    ends; a double click on a waypoint removes it
//   hover_x(n, ui)                   the × beside the middle of the cable under the pointer
//                                    (selection tool): a click disconnects it
//   leds_on_top(n, ui) / plus_handles(n, ui)   after ui.diagram: the LEDs of the selected and
//                                    hovered cables over the widget's highlight, a "+" in the
//                                    handles at the middles of a selected cable's stretches
//   tooltip(canvas, doc, ui)         ports, kind, speed, state and packets per second of the
//                                    cable under the pointer; name, kind and state of a port dot
//   state_text / ends_text           "aktiv", "ausgesteckt", ...; "PC1 eth0 – SW1 Gi1/0/1"
//
// Several cables or devices change in one undo step.  The simulator follows every step.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::project::{Point, Route};
use jfn_model::{cable, CableKind, Project};
use jfn_sim::Led;
use ui::*;

use super::paint::key_port;
use crate::app::constants::*;
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::edits;

/// the selected cables (project ids)
pub(crate) fn sel_links(n: &Net) -> Vec<u32> {
    n.canvas.selected().into_iter().filter(|&id| n.doc.proj.link(id).is_some()).collect()
}

/// the selected devices
pub(crate) fn sel_devices(n: &Net) -> Vec<u32> {
    n.canvas.selected().into_iter().filter(|&id| n.doc.proj.device(id).is_some()).collect()
}

fn flag(on: bool, f: u32) -> u32 {
    if on { f } else { 0 }
}

/// does `kind` plug into the ports of link `id`?
fn fits(p: &Project, id: u32, kind: CableKind) -> bool {
    let Some(l) = p.link(id) else { return false };
    match (p.resolve_port(&l.a), p.resolve_port(&l.b)) {
        (Ok(a), Ok(b)) => cable::cable_fits(kind, &a, &b).is_ok(),
        _ => false,
    }
}

/// enabled / checked of the cable and power commands (every frame)
pub(crate) fn states(n: &Net, ui: &mut Ui) {
    let p = &n.doc.proj;
    let links = sel_links(n);
    let none = links.is_empty();
    let all = |f: &dyn Fn(&jfn_model::Link) -> bool| !none && links.iter().all(|&id| p.link(id).is_some_and(f));
    for (c, r) in [(CMD_ROUTE_STRAIGHT, Route::Straight), (CMD_ROUTE_ORTHO, Route::Ortho), (CMD_ROUTE_CURVE, Route::Curve)] {
        ui.cmd_state(c, flag(none, CF_DISABLED) | flag(all(&|l| l.route == r), CF_CHECKED));
    }
    ui.cmd_state(CMD_DISCONNECT, flag(none, CF_DISABLED));
    ui.cmd_state(CMD_CABLE0, flag(none, CF_DISABLED) | flag(all(&|l| !l.cable_fixed), CF_CHECKED));
    for (k, &kind) in CableKind::ALL.iter().enumerate() {
        let ok = !none && links.iter().all(|&id| fits(p, id, kind));
        ui.cmd_state(CMD_CABLE0 + 1 + k as u32, flag(!ok, CF_DISABLED) | flag(all(&|l| l.cable_fixed && l.cable == kind), CF_CHECKED));
    }
    let devs = sel_devices(n);
    let on = !devs.is_empty() && devs.iter().all(|&d| p.device(d).is_some_and(|d| !d.off));
    ui.cmd_state(CMD_POWER, flag(devs.is_empty(), CF_DISABLED) | flag(on, CF_CHECKED));
}

// several edits as one undo step
fn grouped(n: &mut Net, label: &str, edits: Vec<alloc::boxed::Box<dyn ui::Edit<Project>>>) {
    if edits.is_empty() {
        return;
    }
    n.doc.hist.group_begin(label);
    for e in edits {
        n.doc.apply(e);
    }
    n.doc.hist.group_end();
}

/// the route of the selected cables
pub(crate) fn set_route(n: &mut Net, route: Route) {
    let label = match route {
        Route::Straight => "Kabel gerade führen",
        Route::Ortho => "Kabel rechtwinklig führen",
        Route::Curve => "Kabel als Kurve führen",
    };
    let v: Vec<_> = sel_links(n).into_iter().filter_map(|id| edits::link_edit(&n.doc.proj, id, label, |l| l.route = route)).collect();
    grouped(n, label, v);
}

/// the kind of the selected cables (None: the right one for their ports)
pub(crate) fn set_kind(n: &mut Net, kind: Option<CableKind>) {
    let p = &n.doc.proj;
    let label = "Kabeltyp ändern";
    let v: Vec<_> = sel_links(n)
        .into_iter()
        .filter_map(|id| {
            let l = p.link(id)?;
            let (a, b) = (p.resolve_port(&l.a).ok()?, p.resolve_port(&l.b).ok()?);
            let (k, fixed) = match kind {
                Some(k) => {
                    cable::cable_fits(k, &a, &b).ok()?;
                    (k, true)
                }
                None => (cable::cable_kind(&a, &b).ok()?, false),
            };
            edits::link_edit(p, id, label, |l| {
                l.cable = k;
                l.cable_fixed = fixed;
            })
        })
        .collect();
    grouped(n, label, v);
}

/// Trennen: the selected cables go (devices stay)
pub(crate) fn disconnect_selection(n: &mut Net) {
    let ids = sel_links(n);
    if let Some(e) = edits::delete(&n.doc.proj, &ids) {
        n.doc.apply(e);
    }
    n.canvas.st.sel.retain(|k| !ids.contains(&(*k as u32)));
}

/// the selected devices on (when one of them is off), else off
pub(crate) fn toggle_power(n: &mut Net) {
    let devs = sel_devices(n);
    let p = &n.doc.proj;
    let on = devs.iter().any(|&d| p.device(d).is_some_and(|d| d.off));
    set_power(n, &devs, on);
}

/// switch devices on or off (one undo step)
pub(crate) fn set_power(n: &mut Net, devs: &[u32], on: bool) {
    let p = &n.doc.proj;
    let label = if devs.len() == 1 {
        let name = p.device(devs[0]).map_or("", |d| d.name.as_str());
        format!("{} {}", name, if on { "einschalten" } else { "ausschalten" })
    } else {
        String::from(if on { "Geräte einschalten" } else { "Geräte ausschalten" })
    };
    let v: Vec<_> = devs.iter().filter_map(|&d| edits::device_edit(p, d, &label, |x| x.off = !on)).collect();
    grouped(n, &label, v);
}

// round to whole world units
fn rnd(v: f32) -> i32 {
    let f = v + 0.5;
    let t = f as i32;
    if (t as f32) > f { t - 1 } else { t }
}

/// DEv::Waypoint: waypoint idx of a cable goes to `at` (inserted first); one undo step when
/// the drag ends
pub(crate) fn waypoint(n: &mut Net, edge: u64, idx: u32, at: [f32; 2], insert: bool, done: bool) {
    let id = edge as u32;
    let Some(l) = n.doc.proj.link(id) else { return };
    if n.canvas.wp_drag.as_ref().is_none_or(|b| b.id != id) {
        n.canvas.wp_drag = Some(l.clone());
    }
    let pt = Point { x: rnd(at[0]), y: rnd(at[1]) };
    if let Some(l) = n.doc.proj.link_mut(id) {
        let i = idx as usize;
        if insert {
            l.waypoints.insert(i.min(l.waypoints.len()), pt);
        } else if let Some(w) = l.waypoints.get_mut(i) {
            *w = pt;
        }
    }
    n.doc.touch();
    if done {
        if let Some(before) = n.canvas.wp_drag.take() {
            let more = n.doc.proj.link(id).is_some_and(|l| l.waypoints.len() > before.waypoints.len());
            let label = if more { "Wegpunkt einfügen" } else { "Wegpunkt verschieben" };
            if let Some(e) = edits::link_since(&n.doc.proj, &before, label) {
                n.doc.record(e);
            }
        }
    }
}

/// DEv::WaypointDelete: a double click on a waypoint removes it (one undo step); Esc during
/// the drag that inserted it takes the drag back
pub(crate) fn waypoint_delete(n: &mut Net, edge: u64, idx: u32) {
    let id = edge as u32;
    if let Some(before) = n.canvas.wp_drag.take() {
        if before.id == id {
            if let Some(l) = n.doc.proj.link_mut(id) {
                *l = before;
            }
            n.doc.touch();
            return;
        }
    }
    if let Some(e) = edits::link_edit(&n.doc.proj, id, "Wegpunkt entfernen", |l| {
        if (idx as usize) < l.waypoints.len() {
            l.waypoints.remove(idx as usize);
        }
    }) {
        n.doc.apply(e);
    }
}

/// the × at the middle of the cable under the pointer (selection tool, cable not selected): a
/// click disconnects it; it stays while the pointer is on it
pub(crate) fn hover_x(n: &mut Net, ui: &mut Ui) {
    if n.canvas.tool != super::tools::Tool::Select || ui.popup_count() != 0 || n.canvas.st.busy() {
        n.canvas.x_link = None;
        return;
    }
    let st = &n.canvas.st;
    // (a line of several cables of a cluster has no ×, G1)
    let under = match st.hover {
        DHit::Edge { key, .. } if !st.sel.contains(&key) && !super::groups::is_agg(key) => Some(key as u32),
        _ => None,
    };
    // the × of the last frame stays while the pointer is near it (on its way from the cable to
    // the × the canvas sees neither)
    let (mx, my) = ui.mouse();
    let keep = ui.dp(26);
    let on_x = |id: u32| {
        x_rect(n, ui, id).is_some_and(|r| {
            let (dx, dy) = (mx - (r[0] + r[2] / 2), my - (r[1] + r[3] / 2));
            dx * dx + dy * dy <= keep * keep
        })
    };
    let kept = n.canvas.x_link.filter(|&id| n.doc.proj.link(id).is_some() && !n.canvas.st.sel.contains(&(id as u64)) && on_x(id));
    let link = kept.or(under);
    n.canvas.x_link = link;
    let Some(id) = link else { return };
    let Some(r) = x_rect(n, ui, id) else { return };
    ui.next_rect_px(r[0], r[1], r[2], r[3]);
    ui.next_flags(ST_ICON_ONLY | ST_NOFOCUS);
    if ui.icon_button(ID_CABLE_X, IC_CANCEL, t("Kabel trennen")) & EV_ACT != 0 {
        if let Some(e) = edits::delete(&n.doc.proj, &[id]) {
            n.doc.apply(e);
        }
        n.canvas.x_link = None;
        ui.repass();
    }
}

// the × of a cable: beside the middle of its route, inside the canvas (px x, y, w, h)
fn x_rect(n: &Net, ui: &Ui, id: u32) -> Option<[i32; 4]> {
    let rt = n.canvas.st.route(id as u64)?;
    if rt.pts.len() < 2 {
        return None;
    }
    let vp = n.canvas.st.vp;
    // beside the middle of the cable (above it, or right of an upright one): the cable itself
    // stays clickable there
    let len = dg_length(&rt.pts);
    let (m, u) = super::paint::at_len(&rt.pts, len * 0.5);
    let mut nrm = [-u[1], u[0]];
    if nrm[1] > 0.0 || (nrm[1] == 0.0 && nrm[0] < 0.0) {
        nrm = [-nrm[0], -nrm[1]];
    }
    let c = vp.to_screen(m);
    // far enough that its square never reaches the cable's line (18 > 9 * sqrt 2 + 4)
    let off = ui.dp(18) as f32;
    let c = [c[0] + nrm[0] * off, c[1] + nrm[1] * off];
    let r = vp.rect;
    let half = ui.dp(9);
    let (x, y) = (c[0] as i32 - half, c[1] as i32 - half);
    if x < r[0] || y < r[1] || x + half * 2 > r[0] + r[2] || y + half * 2 > r[1] + r[3] {
        return None;
    }
    Some([x, y, half * 2, half * 2])
}

/// the LEDs of the selected and the hovered cables once more, over the widget's highlight
/// (after ui.diagram)
pub(crate) fn leds_on_top(n: &Net, ui: &mut Ui) {
    let st = &n.canvas.st;
    if st.vp.zoom < 0.5 {
        return;
    }
    let mut keys: Vec<u64> = st.sel.iter().copied().filter(|&k| (k >> 32 == 0 && n.doc.proj.link(k as u32).is_some()) || super::groups::is_agg(k)).collect();
    if let DHit::Edge { key, .. } = st.hover {
        keys.push(key);
    }
    if keys.is_empty() {
        return;
    }
    let ri = super::paint::led_radius(st.vp.scale() as f32);
    let hair = ui.hair();
    let bg = ui.color(T_BG) | 0xFF00_0000;
    let ring = super::paint::led_ring(ui.color(T_TEXT), bg, ui.theme_dark());
    let mut cols = [0u32; 5];
    for (i, led) in [Led::Up, Led::Down, Led::Blocking, Led::Learning, Led::NoPower].into_iter().enumerate() {
        cols[i] = super::paint::led_color(led, &|t| ui.color(t), bg);
    }
    let col = |l: Led| cols[match l {
        Led::Up => 0,
        Led::Down => 1,
        Led::Blocking => 2,
        Led::Learning => 3,
        Led::NoPower => 4,
    }];
    let r = st.vp.rect;
    ui.clip_push(r[0], r[1], r[2], r[3]);
    for k in keys {
        let Some(rt) = st.route(k) else { continue };
        if rt.pts.len() < 2 {
            continue;
        }
        let len = dg_length(&rt.pts);
        // (a line of a cluster: the worst of its cables', G1)
        let leds = n.canvas.scene.line_leds(k, &n.doc.sim.leds).unwrap_or([Led::NoPower; 2]);
        let rev: Vec<[f32; 2]> = rt.pts.iter().rev().copied().collect();
        for (i, pts) in [rt.pts.as_slice(), rev.as_slice()].into_iter().enumerate() {
            let (c, _) = super::paint::led_at(pts, len);
            let p = st.vp.to_screen(c);
            let (x, y) = ((p[0] + 0.5) as i32 - ri, (p[1] + 0.5) as i32 - ri);
            ui.cmd_rrect(x, y, ri * 2, ri * 2, ri, hair, col(leds[i]), ring, ring);
        }
    }
    ui.clip_pop();
}

/// a "+" in the handles at the middles of a selected cable's stretches (the widget draws the
/// handles; a drag there inserts a waypoint)
pub(crate) fn plus_handles(n: &Net, ui: &mut Ui) {
    let st = &n.canvas.st;
    if n.canvas.tool != super::tools::Tool::Select {
        return;
    }
    let arm = ui.dp(2) as f32 + 0.5;
    let w = (ui.hair() * 256) as i32;
    let accent = ui.color(T_ACCENT);
    let r = st.vp.rect;
    ui.clip_push(r[0], r[1], r[2], r[3]);
    ui.draw_color(accent);
    for &k in &st.sel {
        if k >> 32 != 0 || n.doc.proj.link(k as u32).is_none() {
            continue;
        }
        let Some(rt) = st.route(k) else { continue };
        for j in 0..rt.marks.len().saturating_sub(1) {
            let (a, b) = (rt.marks[j] as usize, rt.marks[j + 1] as usize);
            if b <= a || b >= rt.pts.len() {
                continue;
            }
            let p = st.vp.to_screen(dg_point_at(&rt.pts[a..=b], 0.5));
            let (x, y) = ((p[0] as i32) as f32 + 0.5, (p[1] as i32) as f32 + 0.5);
            let fx = |v: f32| (v * 256.0) as i32;
            ui.draw_line(fx(x - arm), fx(y), fx(x + arm), fx(y), w);
            ui.draw_line(fx(x), fx(y - arm), fx(x), fx(y + arm), w);
        }
    }
    ui.clip_pop();
}

/// the state of a cable in words: "aktiv", "ausgesteckt", "ohne Strom", "unten" (in the
/// current language)
pub(crate) fn state_text(down: bool, leds: Option<[Led; 2]>) -> &'static str {
    t(match leds {
        _ if down => "ausgesteckt",
        Some([Led::Up, Led::Up]) => "aktiv",
        Some(l) if l.contains(&Led::NoPower) => "ohne Strom",
        Some(l) if l.iter().any(|x| matches!(x, Led::Blocking | Led::Learning)) => "blockiert",
        _ => "unten",
    })
}

/// a cable for screen readers and scripts: "Kabel PC1 eth0 – SW1 Gi1/0/1, Kupfer, gerade, aktiv"
pub(crate) fn sem_label(doc: &crate::doc::Doc, key: u64) -> String {
    match doc.proj.link(key as u32) {
        Some(l) => tf("Kabel {}, {}, {}", &[&ends_text(&doc.proj, l), t(l.cable.title()), state_text(l.down, doc.sim.led(l.id))]),
        None => String::from("–"),
    }
}

/// "PC1 eth0 – SW1 Gi1/0/1"
pub(crate) fn ends_text(p: &Project, l: &jfn_model::Link) -> String {
    let name = |d: u32| p.device(d).map_or("?", |x| x.name.as_str());
    format!("{} {} – {} {}", name(l.a.dev), jfn_model::ifname::short(&l.a.port), name(l.b.dev), jfn_model::ifname::short(&l.b.port))
}

/// the tooltip of the cable or port dot under the pointer (right after ui.diagram)
pub(crate) fn tooltip(c: &super::Canvas, doc: &crate::doc::Doc, ui: &mut Ui) {
    let p = &doc.proj;
    match c.st.hover {
        // a line of several cables of a cluster: their ends (G1)
        DHit::Edge { key, .. } if super::groups::is_agg(key) => match super::groups::line_tooltip(doc, &c.scene, key) {
            Some(text) => ui.tooltip_part(&text, key),
            None => ui.tooltip_part("", 0),
        },
        DHit::Edge { key, .. } => {
            let Some(l) = p.link(key as u32) else { return ui.tooltip_part("", 0) };
            let sim = &doc.sim;
            let state = state_text(l.down, sim.led(l.id));
            let pps = sim.rate(l.id);
            let text = tf("{} · {} · {} · {} · {} Pakete/s", &[&ends_text(p, l), t(l.cable.title()), &crate::app::tr::speed(l.kbps), state, &pps.to_string()]);
            ui.tooltip_part(&text, key);
        }
        DHit::Port(key) => {
            let Some((d, i)) = key_port(key) else { return ui.tooltip_part("", 0) };
            let spec = p.model_of(d).and_then(|m| m.ports().into_iter().find(|s| s.index == i));
            let Some(s) = spec else { return ui.tooltip_part("", 0) };
            let text = tf("{} · {} {} · frei", &[&s.name, &crate::app::tr::speed(s.kbps), t(s.media.name())]);
            ui.tooltip_part(&text, key);
        }
        _ => ui.tooltip_part("", 0),
    }
}
