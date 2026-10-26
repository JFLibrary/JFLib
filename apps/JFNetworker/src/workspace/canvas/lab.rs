// lab -- the lab on the canvas: the services of the devices as pills, mini sparklines under load
// balancers, KV nodes, caches and queues, and devices a fault switched off
//
//   Lab                       what the simulator runs: the pills of each device, the devices it
//                             has off while the project has them on (a power fault), read again
//                             when its snapshot changes (Sim::ver); key() is part of the canvas'
//                             version, so the backing store repaints when they change
//   update(n)                 (every frame, before ui.diagram) the above
//   PILLS / pill_of(kind)     the pill of a kind of service: its short text and colour
//   paint_pills(g, f, ..)     (render time, into the backing store) a device's pills along the
//                             top edge of its box from the right corner; returns the box they
//                             cover (culling)
//   pills_from(list) / sparks_from(list)   the pills and the sparklines of the services
//                             (Engine::service_list)
//   pill_records(n, ui)       (after ui.diagram) the pills of the devices in view as text
//                             records: "SRV1: DNS, DHCP", "KV2: KV (gestoppt)"
//   sparklines(n, ui)         (after ui.diagram) the sparklines: from 75 % on, under the
//                             devices in view that run lb, kv, cache or mq, at most SPARK_MAX,
//                             the last SPARK_POINTS samples of their key series (requests per
//                             second, a queue's depth); each has a text record "KV2: Anfragen/s
//                             11,6" (screen readers, scripts)
//
// Pills (the plan's "Dienst-Pillen"): LB, KV, Cache, Q (a message queue), Worker, HTTP (a web
// server), DNS, DHCP (a server), Skript -- one per kind, in that order, at most three (then
// "+n"); outlined in the colour of the kind's category (distributed systems olive, servers
// slate violet, scripts steel blue) with the text in the theme's text colour, so they read in
// both themes; a stopped service's pill is faded.  They belong to the static scene: the painter
// draws them with the devices (from 50 %, as the names), and the backing store repaints only
// when they change.  The sparklines change with every sample, so they are ordinary commands
// over the canvas instead (clipped to it): at most 24 drawing objects that the dirty logic
// compares by their values, read again only when series_gen changes; devices outside the view
// get none (the scene's boxes against the viewport), so 500 devices cost what 24 do.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
use alloc::vec::Vec;

use ui::*;

use super::paint::DEV_W;
use crate::app::constants::ID_SPARK0;
use crate::app::tr::t;
use crate::app::Net;
use crate::workspace::icons::TINTS;

/// sparklines on the canvas at most, their points, the least zoom
pub(crate) const SPARK_MAX: usize = 24;
pub(crate) const SPARK_POINTS: usize = 20;
pub(crate) const SPARK_ZOOM: f32 = 0.75;
/// pills on a device at most (the last one says "+n" for the rest)
const PILLS_MAX: usize = 3;
/// a sparkline's height (world units) and its gap below the name
const SPARK_H: f32 = 14.0;
const SPARK_GAP: f32 = 3.0;

/// the pill of a kind of service: its kind, short text, colour (an index into TINTS)
pub(crate) struct Pill {
    pub kind: &'static str,
    pub text: &'static str,
    pub tint: usize,
}

/// the pills in their order on a device
// i18n: text
pub(crate) static PILLS: [Pill; 9] = [
    Pill { kind: "lb", text: "LB", tint: 5 },
    Pill { kind: "kv", text: "KV", tint: 5 },
    Pill { kind: "cache", text: "Cache", tint: 5 },
    Pill { kind: "mq", text: "Q", tint: 5 },
    Pill { kind: "worker", text: "Worker", tint: 5 },
    Pill { kind: "http-server", text: "HTTP", tint: 4 },
    Pill { kind: "dns-server", text: "DNS", tint: 4 },
    Pill { kind: "dhcp-server", text: "DHCP", tint: 4 },
    Pill { kind: "script", text: "Skript", tint: 1 },
];

/// the index of a kind's pill (None: the kind has none, as clients and the console's tools)
pub(crate) fn pill_of(kind: &str) -> Option<usize> {
    PILLS.iter().position(|p| p.kind == kind)
}

/// the kinds of service with a sparkline and their key series
const SPARK_KINDS: [(&str, &str); 4] = [("lb", "requests"), ("kv", "requests"), ("cache", "requests"), ("mq", "depth")];

/// the pills of the services (Engine::service_list): device -> (index into PILLS, a service
/// of the kind runs), one per kind in PILLS' order
pub(crate) fn pills_from(list: &[jfn_sim::lab::SvcBrief]) -> BTreeMap<u32, Vec<(u8, bool)>> {
    let mut pills: BTreeMap<u32, Vec<(u8, bool)>> = BTreeMap::new();
    for s in list {
        if let Some(i) = pill_of(s.kind) {
            let v = pills.entry(s.dev).or_default();
            match v.iter_mut().find(|x| x.0 == i as u8) {
                Some(x) => x.1 |= s.running,
                None => v.push((i as u8, s.running)),
            }
        }
    }
    for v in pills.values_mut() {
        v.sort();
    }
    pills
}

/// the sparklines of the services: device -> (its first service of lb, kv, cache or mq, the
/// series it shows)
pub(crate) fn sparks_from(list: &[jfn_sim::lab::SvcBrief]) -> BTreeMap<u32, (String, &'static str)> {
    let mut out: BTreeMap<u32, (String, &'static str)> = BTreeMap::new();
    for s in list {
        if let Some(&(_, series)) = SPARK_KINDS.iter().find(|k| k.0 == s.kind) {
            out.entry(s.dev).or_insert((s.name.clone(), series));
        }
    }
    out
}

#[derive(Default)]
pub(crate) struct Lab {
    /// the snapshot it was read from (Sim::ver and the view's address; 0: never)
    ver: (u32, usize),
    /// device -> its pills: (index into PILLS, a service of the kind runs)
    pub pills: BTreeMap<u32, Vec<(u8, bool)>>,
    /// devices the simulator has off while the project has them on
    pub dark: BTreeSet<u32>,
    key: u32,
    /// device -> the service and series of its sparkline
    spark_of: BTreeMap<u32, (String, &'static str)>,
    /// the values of the sparklines (device -> values) and the series_gen they were read at
    sparks: BTreeMap<u32, Vec<f32>>,
    spark_gen: u64,
}

impl Lab {
    /// changes with the pills and the dark devices (the canvas' version)
    pub(crate) fn key(&self) -> u32 {
        self.key
    }
}

/// what the simulator runs, read again when its snapshot changed
pub(crate) fn update(n: &mut Net) {
    let lab = &mut n.canvas.lab;
    let seen = (n.doc.sim.ver, alloc::rc::Rc::as_ptr(&n.doc.sim.view) as usize);
    if lab.ver == seen {
        return;
    }
    lab.ver = seen;
    let list = n.doc.sim.state().service_list();
    let (pills, spark_of) = (pills_from(&list), sparks_from(&list));
    // a device off in the simulator, on in the project: a power fault
    let p = &n.doc.proj;
    let dark: BTreeSet<u32> = n.doc.sim.view.devices.iter().filter(|d| !d.power && p.device(d.dev).is_some_and(|x| !x.off)).map(|d| d.dev).collect();
    let mut h: u32 = 0x811C_9DC5;
    for (d, v) in &pills {
        for x in core::iter::once(*d).chain(v.iter().map(|x| x.0 as u32 | (x.1 as u32) << 8)) {
            h = (h ^ x).wrapping_mul(0x0100_0193);
        }
    }
    for d in &dark {
        h = (h ^ d ^ 0xA5A5_0000).wrapping_mul(0x0100_0193);
    }
    if spark_of != lab.spark_of {
        lab.spark_gen = u64::MAX;
    }
    (lab.pills, lab.dark, lab.key, lab.spark_of) = (pills, dark, h, spark_of);
}

/// the pills of a device into the backing store: along the top edge of its box q (px) from the
/// right corner, half above it; `off`: the device has no power.  Returns the px box x0, y0,
/// x1, y1 they cover (none: q's top left corner twice)
pub(crate) fn paint_pills(g: &mut Gfx, f: &DgFrame, lab: &Lab, dev: u32, q: [i32; 4], off: bool, measure: bool) -> [i32; 4] {
    let none = [q[0], q[1], q[0], q[1]];
    let Some(v) = lab.pills.get(&dev) else { return none };
    if v.is_empty() {
        return none;
    }
    let s = f.vp.scale() as f32;
    let (font, size) = f.font(TS_CAPTION);
    if size < 4 << 8 {
        return none;
    }
    g.text.set_font(font, size);
    let (asc, desc) = (g.text.ascent(), g.text.descent());
    let (px, py, gap) = (((4.0 * s) as i32).max(2), ((1.0 * s) as i32).max(1), ((2.0 * s) as i32).max(1));
    let h = asc + desc + py * 2;
    let more = v.len().saturating_sub(PILLS_MAX - 1);
    let texts: Vec<(String, usize, bool)> = if v.len() > PILLS_MAX {
        let mut t2: Vec<(String, usize, bool)> = v.iter().take(PILLS_MAX - 1).map(|x| (String::from(t(PILLS[x.0 as usize].text)), PILLS[x.0 as usize].tint, x.1)).collect();
        t2.push((alloc::format!("+{}", more), 0, true));
        t2
    } else {
        v.iter().map(|x| (String::from(t(PILLS[x.0 as usize].text)), PILLS[x.0 as usize].tint, x.1)).collect()
    };
    let ws: Vec<i32> = texts.iter().map(|x| g.text.width(&x.0) + px * 2).collect();
    let total: i32 = ws.iter().sum::<i32>() + gap * (ws.len() as i32 - 1);
    let right = q[0] + q[2];
    let top = q[1] - h / 2;
    let area = [right - total, top, right, top + h];
    if measure {
        return area;
    }
    let bg = f.bg | 0xFF00_0000;
    let text_c = f.color(T_TEXT);
    let hair = ((f.vp.dpi + 24) / 96).max(1);
    let mut x = right - total;
    for (k, (text, tint, running)) in texts.iter().enumerate() {
        let w = ws[k];
        let base = TINTS[*tint] | 0xFF00_0000;
        let base = if f.dark { color_mix(base, 0xFFFF_FFFF, 72) } else { base };
        let faded = off || !*running;
        let stroke = if faded { color_mix(base, bg, 150) } else { base };
        let fill = color_mix(bg, base, if f.dark { 70 } else { 44 });
        let tc = if faded { color_mix(text_c | 0xFF00_0000, bg, 130) } else { text_c };
        {
            let mut sh = g.shapes();
            sh.dc.set_color(fill);
            sh.fill_rrect(x, top, w, h, h / 2);
            sh.dc.set_color(stroke);
            sh.stroke_rrect(x, top, w, h, h / 2, hair);
        }
        g.text.set_font(font, size);
        g.dc.set_color(tc);
        g.text.draw(g.dc, g.sf, x + px, top + py + asc, text.as_bytes());
        x += w + gap;
    }
    area
}

/// the pills of the devices in view as text records (screen readers, scripts): "SRV1: DNS,
/// DHCP", a service that does not run "KV (gestoppt)"; from 50 % on, as the pills show
pub(crate) fn pill_records(n: &Net, ui: &mut Ui) {
    let vp = n.canvas.st.vp;
    let lab = &n.canvas.lab;
    if vp.zoom < 0.5 - 1e-3 || lab.pills.is_empty() {
        return;
    }
    let r = vp.rect;
    for d in &n.canvas.scene.devs {
        let Some(v) = lab.pills.get(&d.id) else { continue };
        let q = vp.rect_to_screen(d.rect);
        if q[0] + q[2] < r[0] as f32 || q[0] > (r[0] + r[2]) as f32 || q[1] + q[3] < r[1] as f32 || q[1] > (r[1] + r[3]) as f32 {
            continue;
        }
        let parts: Vec<String> = v
            .iter()
            .map(|&(i, run)| {
                let p = t(PILLS[i as usize].text);
                if run { String::from(p) } else { crate::app::tr::tf("{} ({})", &[p, t("gestoppt")]) }
            })
            .collect();
        ui.last_set([q[0] as i32, q[1] as i32, q[2] as i32, (q[3] * 0.3) as i32], 0);
        ui.sem(K_TEXT, 0, Some(&crate::app::tr::tf("{}: {}", &[&d.name, &parts.join(", ")])), 0);
    }
}

/// the sparklines under the lab's devices in view (after ui.diagram)
pub(crate) fn sparklines(n: &mut Net, ui: &mut Ui) {
    let st = &n.canvas.st;
    let vp = st.vp;
    if vp.zoom < SPARK_ZOOM - 1e-3 || n.canvas.lab.spark_of.is_empty() {
        return;
    }
    let r = vp.rect;
    // the devices in view that have one, in the project's order
    let mut shown: Vec<(u32, [f32; 4])> = Vec::new();
    for d in n.canvas.scene.devs.iter() {
        if !n.canvas.lab.spark_of.contains_key(&d.id) {
            continue;
        }
        let q = vp.rect_to_screen(d.rect);
        let below = q[3] * 0.9;
        if q[0] + q[2] < r[0] as f32 || q[0] > (r[0] + r[2]) as f32 || q[1] + q[3] + below < r[1] as f32 || q[1] > (r[1] + r[3]) as f32 {
            continue;
        }
        shown.push((d.id, q));
        if shown.len() == SPARK_MAX {
            break;
        }
    }
    if shown.is_empty() {
        return;
    }
    // their values, read again with new samples
    let eng = n.doc.sim.state();
    let sgen = eng.series_gen();
    let lab = &mut n.canvas.lab;
    if lab.spark_gen != sgen {
        lab.spark_gen = sgen;
        lab.sparks.clear();
    }
    for (d, _) in &shown {
        if lab.sparks.contains_key(d) {
            continue;
        }
        let Some((svc, series)) = lab.spark_of.get(d) else { continue };
        let pts = eng.series_one(*d, svc, series).map(|v| v.points).unwrap_or_default();
        let vals: Vec<f32> = pts.iter().skip(pts.len().saturating_sub(SPARK_POINTS)).map(|p| p.1 as f32).collect();
        lab.sparks.insert(*d, vals);
    }
    // under the name below each box
    let zoom = vp.zoom;
    let s = vp.scale() as f32;
    ui.ts_font(TS_BODY);
    let name_h = (ui.text.ascent() + ui.text.descent()) as f32 * zoom;
    let plate = (ui.color(T_BG) & 0x00FF_FFFF) | 0xD800_0000;
    let line = ui.color(T_ACCENT);
    let sep = ui.decimal_sep();
    ui.clip_push(r[0], r[1], r[2], r[3]);
    for (k, (d, q)) in shown.iter().enumerate() {
        let Some(vals) = n.canvas.lab.sparks.get(d) else { continue };
        if vals.len() < 2 {
            continue;
        }
        let w = DEV_W * s * 0.8;
        let h = SPARK_H * s;
        let x = q[0] + (q[2] - w) * 0.5;
        let y = q[1] + q[3] + 3.0 * s + name_h + SPARK_GAP * s;
        let (xi, yi, wi, hi) = (x as i32, y as i32, (w as i32).max(8), (h as i32).max(4));
        let pad = (2.0 * s) as i32;
        ui.cmd_rrect(xi - pad, yi - pad, wi + pad * 2, hi + pad * 2, (2.0 * s) as i32, 0, plate, 0, 0);
        ui.next_rect_px(xi, yi, wi, hi);
        ui.sparkline(ID_SPARK0 + k as u32, vals, line);
        // for screen readers and scripts: whose it is, what it shows, the newest value
        let name = n.doc.proj.device(*d).map_or("", |x| x.name.as_str());
        let what = n.canvas.lab.spark_of.get(d).map_or_else(String::new, |x| crate::workspace::metrics::series_label(x.1));
        let last = vals.last().map_or_else(String::new, |v| LcUnit::Num("").text(*v as f64, sep));
        ui.sem(K_TEXT, 0, Some(&crate::app::tr::tf("{}: {} {}", &[name, &what, &last])), 0);
    }
    ui.clip_pop();
}
