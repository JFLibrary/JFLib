// groups -- the groups of the logical canvas (G1): their frames and clusters in the scene and on
// the canvas, the lines a collapsed group gathers its cables into, the commands and events
//
//   layout(p, gs, syms) -> Layout     (Scene::build) the frames of the expanded groups in view
//                                     (outer ones first; each around what it holds with room
//                                     for names, pills and its title bar) and the clusters of
//                                     the collapsed ones (a device's box at the middle of their
//                                     devices and notes), what each collapsed group hides
//   agg_key(link) / is_agg(key)       the key of a line that stands for several cables
//   role_stroke(colour, dark)         a role's frame colour (3:1 against the canvas, selftest
//                                     group groups)
//   paint_frames(..) / paint_clusters(..)   (render time) frames under the notes: tinted
//                                     fill, frame, title bar with the name, the count and the
//                                     button; clusters with the devices: two cards behind the
//                                     front one, the symbol of their most common kind of device,
//                                     the count in the corner, the badge of their members'
//                                     findings, their services as pills, the name below
//   Grp (Canvas::grp), update(n)      (every frame) the findings of the members of each
//                                     cluster (its badge); key() is part of the canvas' version
//   label(sc, key) / line_label(..)   "Gruppe „Serverraum“, 5 Geräte", "…, zugeklappt", "3 Kabel
//                                     PC1 – Serverraum, Kupfer, gerade, aktiv" (screen readers,
//                                     scripts)
//   group_selection / ungroup_selection / set_collapsed / toggle / activate   the commands and
//                                     events (one undo step each, doc/groups.rs)
//   reveal(n, id) -> bool             the collapsed groups around a device open (its console,
//                                     its device window, a finding's double click)
//   move_set(n, keys)                 the devices and notes a drag of these keys moves
//   states(n, ui) / command(n, ui, id)   Bearbeiten › Gruppieren (Ctrl+G), Gruppierung
//                                     aufheben (Ctrl+Shift+G), Gruppe zuklappen / aufklappen
//
// Keys: a group is its project id (a frame while expanded, a cluster while collapsed); a line
// of several cables of a cluster is agg_key(its lowest cable) -- one per pair of ends and kind
// of cable, its count on the line -- while a single cable keeps its own id (its inspector, its
// waypoints).  Cables inside a cluster are hidden.  A click on a member selects it, on a
// frame's title bar or a cluster the group; the inside of a frame is the background (marquees
// start there).  A double click (Enter) on a cluster expands it in place -- decided over
// opening it as a view of its own: the cables stay where they are and nothing to go back to --,
// on a frame's title it renames the group.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::group::{self, GroupColor, Groups};
use jfn_model::{catalog, Project};
use jfn_sim::Led;
use ui::*;

use super::paint::{fl, px_rect, touches, Scene, DEV_H, DEV_W};
use crate::app::constants::*;
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::groups as dgroups;
use crate::workspace::icons::{tint_of, Symbols, TINTS};

/// a frame's padding around what it holds and its title bar (world units)
pub(crate) const FRAME_PAD: f32 = 10.0;
pub(crate) const FRAME_BAR: f32 = 22.0;
// what a device takes with its pills above and its name below (world units), a character of
// a name about (frames leave room for names)
const PILL_H: f32 = 8.0;
const NAME_H: f32 = 22.0;
const CHAR_W: f32 = 7.2;

/// the key of a line that stands for several cables of a cluster (its lowest cable's id)
pub(crate) fn agg_key(link: u32) -> u64 {
    2 << 48 | link as u64
}

/// a line of several cables
pub(crate) fn is_agg(key: u64) -> bool {
    key >> 48 == 2
}

/// the frame colour of a role (0xFFRRGGBB): muted, 3:1 against the canvas in both themes
pub(crate) fn role_stroke(c: GroupColor, dark: bool) -> u32 {
    let (l, d) = match c {
        GroupColor::Grey => (0x6B737D, 0x9AA2AC),
        GroupColor::Blue => (0x46698C, 0x7FA3C8),
        GroupColor::Green => (0x4A7A52, 0x82B08A),
        GroupColor::Ochre => (0x8A6A2A, 0xC4A15E),
        GroupColor::Brick => (0x9A4F40, 0xD08A7C),
        GroupColor::Violet => (0x665A8C, 0xA79CD0),
    };
    0xFF00_0000 | if dark { d } else { l }
}

/// a frame or a cluster to paint
pub(crate) struct SGroup {
    pub id: u32,
    pub rect: [f32; 4],
    pub name: String,
    pub color: GroupColor,
    /// the devices below it (all levels) and the number of notes
    pub devs: Vec<u32>,
    pub notes: usize,
    /// a cluster: the symbol of its most common kind of device and that category's tint
    pub sym: Option<usize>,
    pub tint: usize,
}

/// the groups in the scene
#[derive(Default)]
pub(crate) struct Layout {
    pub frames: Vec<SGroup>,
    pub clusters: Vec<SGroup>,
    /// device, note or group id -> the cluster that hides it
    pub hidden: BTreeMap<u32, u32>,
}

fn union(a: Option<[f32; 4]>, b: [f32; 4]) -> Option<[f32; 4]> {
    Some(match a {
        None => b,
        Some(a) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
    })
}

// a box x, y, w, h with its pills above and its name below as x0, y0, x1, y1
fn with_name(r: [f32; 4], name: &str) -> [f32; 4] {
    let w = r[2].max(name.chars().count() as f32 * CHAR_W + 8.0);
    let cx = r[0] + r[2] * 0.5;
    [cx - w * 0.5, r[1] - PILL_H, cx + w * 0.5, r[1] + r[3] + NAME_H]
}

fn note_rect(n: &jfn_model::Note) -> [f32; 4] {
    [n.x as f32, n.y as f32, n.w.max(40) as f32, n.h.max(24) as f32]
}

// the symbol of the most common category among devices (the first device of it), its tint
fn rep_symbol(p: &Project, devs: &[u32], syms: &Symbols) -> (Option<usize>, usize) {
    let mut count = [0usize; TINTS.len()];
    let mut first: [Option<&'static catalog::Model>; TINTS.len()] = [None; TINTS.len()];
    for &d in devs {
        if let Some(m) = p.model_of(d) {
            let k = tint_of(m.category);
            count[k] += 1;
            first[k].get_or_insert(m);
        }
    }
    let best = (0..TINTS.len()).max_by_key(|&k| (count[k], usize::MAX - k)).unwrap_or(0);
    match first[best] {
        Some(m) if count[best] > 0 => (syms.index(m.symbol), best),
        _ => (syms.index("note"), 0),
    }
}

fn round(v: f32) -> f32 {
    fl(v + 0.5) as f32
}

/// the frames and clusters of the project's groups in view, what the collapsed ones hide
pub(crate) fn layout(p: &Project, gs: &Groups, syms: &Symbols) -> Layout {
    let mut l = Layout::default();
    if gs.is_empty() {
        return l;
    }
    for &g in gs.order() {
        if let Some(c) = gs.hidden_by(p, g) {
            l.hidden.insert(g, c);
        }
    }
    for id in p.devices.iter().map(|d| d.id).chain(p.notes.iter().map(|n| n.id)) {
        if let Some(c) = gs.hidden_by(p, id) {
            l.hidden.insert(id, c);
        }
    }
    // the innermost groups first: an outer frame holds the boxes of the inner ones
    let mut ext: BTreeMap<u32, [f32; 4]> = BTreeMap::new();
    let mut frames: Vec<(usize, SGroup)> = Vec::new();
    for (k, &g) in gs.order().iter().enumerate().rev() {
        if l.hidden.contains_key(&g) {
            continue;
        }
        let Some(rec) = gs.get(p, g) else { continue };
        let leaves = gs.leaves(g);
        let devs: Vec<u32> = leaves.iter().copied().filter(|&d| p.device(d).is_some()).collect();
        let notes = leaves.len() - devs.len();
        let (sym, tint) = rep_symbol(p, &devs, syms);
        let mut s = SGroup { id: g, rect: [0.0; 4], name: rec.name.clone(), color: rec.color, devs, notes, sym, tint };
        if rec.collapsed {
            // a device's box at the middle of its devices' and notes' boxes
            let mut b = None;
            for &id in &leaves {
                let r = match (p.device(id), p.note(id)) {
                    (Some(d), _) => [d.x as f32, d.y as f32, d.x as f32 + DEV_W, d.y as f32 + DEV_H],
                    (None, Some(n)) => {
                        let r = note_rect(n);
                        [r[0], r[1], r[0] + r[2], r[1] + r[3]]
                    }
                    _ => continue,
                };
                b = union(b, r);
            }
            let Some(b) = b else { continue };
            let (cx, cy) = (round((b[0] + b[2]) * 0.5), round((b[1] + b[3]) * 0.5));
            s.rect = [cx - DEV_W * 0.5, cy - DEV_H * 0.5, DEV_W, DEV_H];
            ext.insert(g, with_name(s.rect, &s.name));
            l.clusters.push(s);
            continue;
        }
        let mut b = None;
        for &m in gs.members(g) {
            let r = if gs.has(m) {
                ext.get(&m).copied()
            } else if let Some(d) = p.device(m) {
                Some(with_name([d.x as f32, d.y as f32, DEV_W, DEV_H], &d.name))
            } else {
                p.note(m).map(|n| {
                    let r = note_rect(n);
                    [r[0], r[1], r[0] + r[2], r[1] + r[3]]
                })
            };
            if let Some(r) = r {
                b = union(b, r);
            }
        }
        let Some(b) = b else { continue };
        let mut r = [b[0] - FRAME_PAD, b[1] - FRAME_PAD - FRAME_BAR, b[2] + FRAME_PAD, b[3] + FRAME_PAD];
        // room for the title and the button
        let need = s.name.chars().count() as f32 * CHAR_W + 72.0 + FRAME_BAR;
        if r[2] - r[0] < need {
            r[2] = r[0] + need;
        }
        ext.insert(g, r);
        s.rect = [r[0], r[1], r[2] - r[0], r[3] - r[1]];
        frames.push((k, s));
    }
    frames.sort_by_key(|x| x.0);
    l.frames = frames.into_iter().map(|x| x.1).collect();
    l.clusters.reverse();
    l
}

// ---------------------------------------------------------------- texts

/// "1 Gerät", "5 Geräte" (and the notes)
pub(crate) fn count_text(devs: usize, notes: usize) -> String {
    let d = if devs == 1 { String::from(t("1 Gerät")) } else { tf("{} Geräte", &[&devs.to_string()]) };
    match notes {
        0 => d,
        1 => tf("{}, 1 Notiz", &[&d]),
        k => tf("{}, {} Notizen", &[&d, &k.to_string()]),
    }
}

/// the name of a group node for screen readers and scripts: "Gruppe „Serverraum“, 5 Geräte"
/// (a frame), "Gruppe „Serverraum“, 5 Geräte, zugeklappt" (a cluster)
pub(crate) fn label(sc: &Scene, key: u64) -> Option<String> {
    if let Some(g) = sc.frames.iter().find(|g| g.id as u64 == key) {
        return Some(tf("Gruppe „{}“, {}", &[&g.name, &count_text(g.devs.len(), g.notes)]));
    }
    sc.clusters.iter().find(|g| g.id as u64 == key).map(|g| tf("Gruppe „{}“, {}, zugeklappt", &[&g.name, &count_text(g.devs.len(), g.notes)]))
}

/// the name of a node that is a device, a note or a group (a cluster shows its group's)
fn node_name(p: &Project, sc: &Scene, id: u32) -> String {
    if let Some(d) = p.device(id) {
        return d.name.clone();
    }
    sc.clusters.iter().chain(sc.frames.iter()).find(|g| g.id == id).map_or_else(|| String::from("?"), |g| g.name.clone())
}

/// a line of several cables for screen readers and scripts: "3 Kabel PC1 – Serverraum,
/// Kupfer, gerade, aktiv"
pub(crate) fn line_label(doc: &crate::doc::Doc, sc: &Scene, key: u64) -> Option<String> {
    let l = sc.line(key)?;
    let (a, b) = sc.line_ends(key)?;
    let leds = sc.line_leds(key, &doc.sim.leds);
    let down = l.agg.iter().all(|&(id, _)| doc.proj.link(id).is_some_and(|x| x.down));
    let state = super::cables::state_text(down, leds);
    Some(tf("{} Kabel {} – {}, {}, {}", &[&l.agg.len().to_string(), &node_name(&doc.proj, sc, a), &node_name(&doc.proj, sc, b), t(l.cable.title()), state]))
}

/// "PC1 eth0 – SW1 Gi1/0/1" for each cable of a line, one per row (its tooltip)
pub(crate) fn line_tooltip(doc: &crate::doc::Doc, sc: &Scene, key: u64) -> Option<String> {
    let l = sc.line(key)?;
    let mut rows: Vec<String> = Vec::new();
    for &(id, _) in &l.agg {
        if let Some(x) = doc.proj.link(id) {
            rows.push(super::cables::ends_text(&doc.proj, x));
        }
    }
    Some(format!("{}\n{}", line_label(doc, sc, key)?, rows.join("\n")))
}

// ---------------------------------------------------------------- painting

/// the frames of the expanded groups in view (render time, under the notes)
pub(crate) fn paint_frames(g: &mut Gfx, f: &DgFrame, sc: &Scene, nodes: &[u32], lod: bool) {
    if sc.frames.is_empty() {
        return;
    }
    let s = f.vp.scale() as f32;
    let hair = ((f.vp.dpi + 24) / 96).max(1);
    let (font, size) = f.font(TS_BODY);
    let bg = f.bg | 0xFF00_0000;
    let text_c = f.color(T_TEXT);
    let text2 = f.color(T_TEXT2);
    for fr in nodes.iter().filter_map(|&i| (i as usize).checked_sub(sc.f0).and_then(|k| sc.frames.get(k))) {
        if !f.visible(fr.rect) {
            continue;
        }
        let q = px_rect(f, fr.rect);
        let stroke = role_stroke(fr.color, f.dark);
        // tinted, not opaque: the grid and the background picture show through (nested frames
        // add up)
        let fill = (stroke & 0x00FF_FFFF) | if f.dark { 0x2400_0000 } else { 0x1600_0000 };
        let bar_fill = (stroke & 0x00FF_FFFF) | if f.dark { 0x5800_0000 } else { 0x4000_0000 };
        let rad = ((5.0 * s) as i32).max(1);
        let bar = ((FRAME_BAR * s) as i32).clamp(1, q[3]);
        // the title bar and the inside, each its part of one rounded box (no tint twice)
        for (y, h, c) in [(q[1], bar, bar_fill), (q[1] + bar, q[3] - bar, fill)] {
            g.dc.clip_push(q[0], y, q[2], h);
            let mut sh = g.shapes();
            sh.dc.set_color(c);
            sh.fill_rrect(q[0], q[1], q[2], q[3], rad);
            drop(sh);
            g.dc.clip_pop();
        }
        // the line under the bar, the frame
        g.dc.set_color((stroke & 0x00FF_FFFF) | 0x8000_0000);
        g.dc.fill(g.sf, q[0] + hair, q[1] + bar, q[2] - hair * 2, hair);
        {
            let mut sh = g.shapes();
            sh.dc.set_color(stroke);
            sh.stroke_rrect(q[0], q[1], q[2], q[3], rad, hair);
        }
        // the button: a minus in a square (collapse)
        let b = px_rect(f, dg_toggle_rect(&DNode { key: 0, rect: fr.rect, ports: 0..0, flags: DN_FRAME | DN_TOGGLE }, FRAME_BAR));
        let m = (b[2] / 4).max(2);
        let (cx, cy) = (b[0] + b[2] / 2, b[1] + b[3] / 2);
        if b[2] >= 8 {
            let mut sh = g.shapes();
            sh.dc.set_color(color_mix(bg, text2 | 0xFF00_0000, 150));
            sh.stroke_rrect(cx - m - hair, cy - m - hair, (m + hair) * 2, (m + hair) * 2, (2.0 * s) as i32, hair);
            sh.dc.set_color(text2);
            sh.line_fx((cx - m / 2) << 8, cy << 8 | 128, (cx + m / 2) << 8, cy << 8 | 128, hair << 8);
        }
        if lod || size < 5 << 8 {
            continue;
        }
        // the title: the name, then the count
        g.text.set_font(font, size);
        let (asc, desc) = (g.text.ascent(), g.text.descent());
        let pad = (8.0 * s) as i32;
        let base = q[1] + (bar + asc - desc) / 2;
        let room = (b[0] - q[0] - pad * 2).max(0);
        g.dc.clip_push(q[0] + pad, q[1], room, bar);
        g.dc.set_color(text_c);
        g.text.draw(g.dc, g.sf, q[0] + pad, base, fr.name.as_bytes());
        let tw = g.text.width(&fr.name);
        let count = count_text(fr.devs.len(), fr.notes);
        g.dc.set_color(text2);
        g.text.draw(g.dc, g.sf, q[0] + pad + tw + pad, base, count.as_bytes());
        g.dc.clip_pop();
    }
}

/// what a cluster shows of its members' findings: errors, warnings
#[derive(Default)]
pub(crate) struct Grp {
    /// cluster -> (errors, warnings) among its devices
    pub probs: BTreeMap<u32, (u32, u32)>,
    seen: (u32, usize, u32),
    key: u32,
    /// the inspector's name field, what it came from, why a name was refused
    pub name: String,
    pub name_src: (u32, String),
    pub err: String,
}

impl Grp {
    /// changes with the badges (the canvas' version)
    pub(crate) fn key(&self) -> u32 {
        self.key
    }
}

/// (every frame, before ui.diagram) the findings of the simulator about the members of each
/// cluster, read again when they or the scene change
pub(crate) fn update(n: &mut Net) {
    let seen = (n.doc.sim.ver, alloc::rc::Rc::as_ptr(&n.doc.sim.view) as usize, n.canvas.scene.ver);
    if n.canvas.grp.seen == seen {
        return;
    }
    n.canvas.grp.seen = seen;
    let mut probs: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    if !n.canvas.scene.clusters.is_empty() {
        let list = n.doc.sim.problems();
        let hidden = &n.canvas.scene.hidden;
        for x in list.iter() {
            let mut hit: Vec<u32> = core::iter::once(x.dev).chain(x.others.iter().copied()).filter_map(|d| hidden.get(&d).copied()).collect();
            hit.sort_unstable();
            hit.dedup();
            for c in hit {
                let e = probs.entry(c).or_default();
                if x.severity == jfn_sim::Severity::Error {
                    e.0 += 1;
                } else {
                    e.1 += 1;
                }
            }
        }
    }
    let mut h: u32 = 0x811C_9DC5;
    for (c, (a, b)) in &probs {
        for v in [*c, *a, *b] {
            h = (h ^ v).wrapping_mul(0x0100_0193);
        }
    }
    n.canvas.grp.probs = probs;
    n.canvas.grp.key = h;
}

/// the clusters of the collapsed groups in view (render time, with the devices)
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_clusters(g: &mut Gfx, f: &DgFrame, sc: &Scene, syms: &Symbols, lab: &super::lab::Lab, grp: &Grp, nodes: &[u32], lod: bool) {
    if sc.clusters.is_empty() {
        return;
    }
    let zoom = f.vp.zoom;
    let s = f.vp.scale() as f32;
    let hair = ((f.vp.dpi + 24) / 96).max(1);
    let (font, size) = f.font(TS_BODY);
    let (cfont, csize) = f.font(TS_CAPTION);
    let named = zoom >= 0.5 && !lod;
    let bg = f.bg | 0xFF00_0000;
    let card = f.color(T_CARD);
    let card = color_mix(bg, card | 0xFF00_0000, ((card >> 24) * 256 / 255) as i32);
    let text_c = f.color(T_TEXT);
    let plate = (f.bg & 0x00FF_FFFF) | 0xD800_0000;
    for c in nodes.iter().filter_map(|&i| (i as usize).checked_sub(sc.c0).and_then(|k| sc.clusters.get(k))) {
        let q = px_rect(f, c.rect);
        let o = ((4.0 * s) as i32).max(2);
        g.text.set_font(font, size);
        let tw = if named { g.text.width(&c.name) } else { 0 };
        let ext = [q[0].min(q[0] + (q[2] - tw) / 2 - o), q[1] - o * 3, (q[0] + q[2] + o * 3).max(q[0] + (q[2] + tw) / 2 + o), q[1] + q[3] + (24.0 * s) as i32];
        if !touches(g, ext) {
            continue;
        }
        let stroke = role_stroke(c.color, f.dark);
        let rad = ((5.0 * s) as i32).max(1);
        {
            // two cards behind the front one, up and to the right
            let mut sh = g.shapes();
            for k in [2, 1] {
                sh.dc.set_color(color_mix(card, stroke, if k == 2 { 40 } else { 24 }));
                sh.fill_rrect(q[0] + o * k, q[1] - o * k, q[2], q[3], rad);
                sh.dc.set_color(stroke);
                sh.stroke_rrect(q[0] + o * k, q[1] - o * k, q[2], q[3], rad, hair);
            }
            sh.dc.set_color(card);
            sh.fill_rrect(q[0], q[1], q[2], q[3], rad);
            sh.dc.set_color(stroke);
            sh.stroke_rrect(q[0], q[1], q[2], q[3], rad, hair.max((1.5 * s) as i32));
        }
        // the symbol of its most common kind of device (far away: a square of its colour)
        if zoom < 0.3 || c.sym.is_none() {
            let side = (q[3] / 2).max(2);
            let tint = TINTS[c.tint] | 0xFF00_0000;
            let mut sh = g.shapes();
            sh.dc.set_color(if f.dark { color_mix(tint, 0xFFFF_FFFF, 56) } else { tint });
            sh.fill_rrect(q[0] + (q[2] - side) / 2, q[1] + (q[3] - side) / 2, side, side, (side / 6).max(1));
        } else if let Some(i) = c.sym {
            let w = (q[2] as f32 * 0.66) as i32;
            let h = syms.height(i, w);
            syms.draw(g, i, q[0] + (q[2] - w) / 2, q[1] + (q[3] - h) / 2, w, c.tint, false);
        }
        if !named || size < 4 << 8 {
            continue;
        }
        // the count in the top right corner (its colour's pill, the canvas' colour in it)
        g.text.set_font(cfont, csize);
        let (casc, cdesc) = (g.text.ascent(), g.text.descent());
        let ch = casc + cdesc + (2.0 * s) as i32;
        let n = c.devs.len().to_string();
        let cw = (g.text.width(&n) + (8.0 * s) as i32).max(ch);
        let (bx, by) = (q[0] + q[2] - cw / 2 + o, q[1] - ch / 2 - o);
        {
            let mut sh = g.shapes();
            sh.dc.set_color(stroke);
            sh.fill_rrect(bx - cw / 2, by, cw, ch, ch / 2);
        }
        g.dc.set_color(bg);
        let nw = g.text.width(&n);
        g.text.draw(g.dc, g.sf, bx - nw / 2, by + (ch + casc - cdesc) / 2, n.as_bytes());
        // the members' findings in the top left corner: "!" and their number
        if let Some(&(errs, warns)) = grp.probs.get(&c.id) {
            let col = if errs > 0 { f.color(T_CRIT) } else { f.color(T_WARN) } | 0xFF00_0000;
            let label = format!("! {}", errs + warns);
            let lw = g.text.width(&label) + (8.0 * s) as i32;
            let (lx, ly) = (q[0] - o, q[1] - ch / 2);
            {
                let mut sh = g.shapes();
                sh.dc.set_color(col);
                sh.fill_rrect(lx, ly, lw, ch, ch / 2);
            }
            g.dc.set_color(0xFFFF_FFFF);
            g.text.draw(g.dc, g.sf, lx + (4.0 * s) as i32, ly + (ch + casc - cdesc) / 2, label.as_bytes());
        }
        // the services of its devices as pills along the top edge (canvas/lab.rs)
        let mut pills: Vec<(u8, bool)> = Vec::new();
        for d in &c.devs {
            for &(k, run) in lab.pills.get(d).map_or(&[][..], |v| v.as_slice()) {
                match pills.iter_mut().find(|x| x.0 == k) {
                    Some(x) => x.1 |= run,
                    None => pills.push((k, run)),
                }
            }
        }
        if !pills.is_empty() {
            pills.sort_unstable();
            super::lab::paint_pill_list(g, f, &pills, [q[0] - o * 2, q[1], q[2], q[3]], false, false);
        }
        // the name below
        g.text.set_font(font, size);
        let (asc, desc) = (g.text.ascent(), g.text.descent());
        let (lx, ly) = (q[0] + (q[2] - tw) / 2, q[1] + q[3] + (3.0 * s) as i32);
        let pad = (3.0 * s) as i32;
        {
            let mut sh = g.shapes();
            sh.dc.set_color(plate);
            sh.fill_rrect(lx - pad, ly, tw + pad * 2, asc + desc, (2.0 * s) as i32);
        }
        g.dc.set_color(text_c);
        g.text.draw(g.dc, g.sf, lx, ly + asc, c.name.as_bytes());
    }
}

/// the LED state of a line's end from its cables' (worst first: down, blocking, without
/// power, up)
pub(crate) fn worst(leds: &[Led]) -> Led {
    let rank = |l: &Led| match l {
        Led::Down => 0,
        Led::Blocking | Led::Learning => 1,
        Led::NoPower => 2,
        Led::Up => 3,
    };
    leds.iter().copied().min_by_key(rank).unwrap_or(Led::NoPower)
}

// ---------------------------------------------------------------- commands and events

/// the groups among the selected keys
pub(crate) fn selected_groups(n: &Net) -> Vec<u32> {
    let gs = &n.canvas.scene.gs;
    n.canvas.st.sel.iter().filter(|&&k| k >> 32 == 0 && gs.has(k as u32)).map(|&k| k as u32).collect()
}

/// key is a group of the scene
pub(crate) fn is_group(n: &Net, key: u64) -> bool {
    key >> 32 == 0 && n.canvas.scene.gs.has(key as u32)
}

fn refused(n: &mut Net, title: &'static str, e: &jfn_model::GroupErr) {
    n.file.msg = Some((IB_INFO, String::from(t(title)), crate::app::tr::msg(&e.msg())));
}

/// Gruppieren (Ctrl+G): the selected devices, notes and groups into a new group (one step);
/// the group is selected
pub(crate) fn group_selection(n: &mut Net, ui: &mut Ui) {
    let ids: Vec<u32> = n.canvas.st.sel.iter().filter(|&&k| k >> 32 == 0).map(|&k| k as u32).collect();
    match dgroups::step(&mut n.doc, "Gruppieren", |p| group::group(p, &ids, "", dgroups::names())) {
        Ok(Some(id)) => n.canvas.select_only(id),
        Ok(None) => {}
        Err(e) => refused(n, "Nicht gruppiert", &e),
    }
    ui.repass();
}

/// Gruppierung aufheben (Ctrl+Shift+G): the selected groups go, their members stay (and are
/// selected)
pub(crate) fn ungroup_selection(n: &mut Net, ui: &mut Ui) {
    let gids = selected_groups(n);
    if gids.is_empty() {
        return;
    }
    let mut members: Vec<u32> = Vec::new();
    for &g in &gids {
        members.extend(n.canvas.scene.gs.members(g).iter().copied().filter(|m| !gids.contains(m)));
    }
    let label = if gids.len() == 1 { "Gruppierung aufheben" } else { "Gruppierungen aufheben" };
    let _ = dgroups::step(&mut n.doc, label, |p| {
        for &g in &gids {
            group::ungroup(p, g)?;
        }
        Ok(())
    });
    n.canvas.st.sel = members.into_iter().map(|k| k as u64).collect();
    ui.repass();
}

/// collapse (on) or expand groups (one step); they stay selected
pub(crate) fn set_collapsed(n: &mut Net, ui: &mut Ui, gids: &[u32], on: bool) {
    let label = if on { "Gruppe zuklappen" } else { "Gruppe aufklappen" };
    let _ = dgroups::step(&mut n.doc, label, |p| {
        for &g in gids {
            group::set_collapsed(p, g, on)?;
        }
        Ok(())
    });
    ui.repass();
}

/// a frame's button: collapse it (a cluster's: expand)
pub(crate) fn toggle(n: &mut Net, ui: &mut Ui, gid: u32) {
    let on = !n.doc.proj.groups.iter().any(|g| g.id == gid && g.collapsed);
    set_collapsed(n, ui, &[gid], on);
    n.canvas.select_only(gid);
}

/// a double click or Enter on a group: a cluster expands, a frame's name opens to rename it
pub(crate) fn activate(n: &mut Net, ui: &mut Ui, gid: u32) {
    if n.doc.proj.groups.iter().any(|g| g.id == gid && g.collapsed) {
        set_collapsed(n, ui, &[gid], false);
        n.canvas.select_only(gid);
    } else {
        n.canvas.open_rename(&n.doc.proj, gid);
        ui.repass();
    }
}

/// the collapsed groups around a device, note or group open (one step) -> something opened
pub(crate) fn reveal(n: &mut Net, id: u32) -> bool {
    let p = &n.doc.proj;
    let gs = Groups::of(p);
    let hiding: Vec<u32> = gs.chain(id).into_iter().filter(|&g| gs.get(p, g).is_some_and(|x| x.collapsed)).collect();
    if hiding.is_empty() {
        return false;
    }
    let _ = dgroups::step(&mut n.doc, "Gruppe aufklappen", |p| {
        for &g in &hiding {
            group::set_collapsed(p, g, false)?;
        }
        Ok(())
    });
    true
}

/// the middle of a device, note or group (its devices' and notes' boxes) in world units
pub(crate) fn centre_of(p: &Project, gs: &Groups, id: u32) -> Option<[f32; 2]> {
    let leaves = if gs.has(id) { gs.leaves(id) } else { alloc::vec![id] };
    let mut b = None;
    for x in leaves {
        let r = match (p.device(x), p.note(x)) {
            (Some(d), _) => [d.x as f32, d.y as f32, d.x as f32 + DEV_W, d.y as f32 + DEV_H],
            (None, Some(o)) => {
                let r = note_rect(o);
                [r[0], r[1], r[0] + r[2], r[1] + r[3]]
            }
            _ => continue,
        };
        b = union(b, r);
    }
    b.map(|b| [(b[0] + b[2]) * 0.5, (b[1] + b[3]) * 0.5])
}

/// a member chosen in the inspector (or a finding): its collapsed groups open (one step), it
/// is selected alone and the view centres on it
pub(crate) fn show(n: &mut Net, id: u32) {
    reveal(n, id);
    let gs = Groups::of(&n.doc.proj);
    if let Some(c) = centre_of(&n.doc.proj, &gs, id) {
        n.canvas.st.vp.cx = c[0] as f64;
        n.canvas.st.vp.cy = c[1] as f64;
    }
    n.canvas.select_only(id);
}

/// the devices and notes a drag of these keys moves: themselves, and everything below a
/// group (a frame or a cluster), once each
pub(crate) fn move_set(n: &Net, keys: &[u64]) -> Vec<u32> {
    let p = &n.doc.proj;
    let gs = &n.canvas.scene.gs;
    let mut v: Vec<u32> = Vec::new();
    for &k in keys {
        if k >> 32 != 0 {
            continue;
        }
        let id = k as u32;
        let more = if gs.has(id) { gs.leaves(id) } else { alloc::vec![id] };
        for x in more {
            if (p.device(x).is_some() || p.note(x).is_some()) && !v.contains(&x) {
                v.push(x);
            }
        }
    }
    v
}

/// enabled / checked of the group commands (every frame)
pub(crate) fn states(n: &Net, ui: &mut Ui) {
    let flag = |on: bool, f: u32| if on { f } else { 0 };
    let p = &n.doc.proj;
    let gs = &n.canvas.scene.gs;
    let any = n.canvas.st.sel.iter().any(|&k| k >> 32 == 0 && (p.device(k as u32).is_some() || p.note(k as u32).is_some() || gs.has(k as u32)));
    let groups = selected_groups(n);
    let collapsed = |g: &u32| p.groups.iter().any(|x| x.id == *g && x.collapsed);
    ui.cmd_state(CMD_GROUP, flag(!any, CF_DISABLED));
    ui.cmd_state(CMD_UNGROUP, flag(groups.is_empty(), CF_DISABLED));
    ui.cmd_state(CMD_GROUP_COLLAPSE, flag(!groups.iter().any(|g| !collapsed(g)), CF_DISABLED));
    ui.cmd_state(CMD_GROUP_EXPAND, flag(!groups.iter().any(collapsed), CF_DISABLED));
}

/// the group commands -> handled
pub(crate) fn command(n: &mut Net, ui: &mut Ui, id: u32) -> bool {
    match id {
        CMD_GROUP => group_selection(n, ui),
        CMD_UNGROUP => ungroup_selection(n, ui),
        CMD_GROUP_COLLAPSE | CMD_GROUP_EXPAND => {
            let on = id == CMD_GROUP_COLLAPSE;
            let gids: Vec<u32> = selected_groups(n).into_iter().filter(|g| n.doc.proj.groups.iter().any(|x| x.id == *g && x.collapsed != on)).collect();
            if !gids.is_empty() {
                set_collapsed(n, ui, &gids, on);
            }
        }
        _ => return false,
    }
    ui.repass();
    true
}
