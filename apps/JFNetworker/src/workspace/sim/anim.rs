// anim -- the frames on the canvas in the simulation mode: envelopes in their category's
// colour along the cables (dots below 50 %), red crosses where something was thrown away, the
// animation of a step, the frames' hit areas, the PDU tool's source
//
//   Tween::new(n, from, to, now)   a step's animation: every shown frame on a cable at either
//                                  moment moves from where it was to where it is; one that
//                                  arrived runs into its receiver and fades, one that left
//                                  fades in at its sender
//   Drops / drops_at(n, t, now)    the crosses of a moment: what was dropped since the previous
//                                  moment a frame left (Engine::drops_between); they show
//                                  half way through the step and fade out after it
//   overlay(n, ui)                 after the canvas: frames, crosses, the selected frame's
//                                  ring, the PDU tool's source; a press on a frame selects it
//                                  (sim::select_frame, the packet tab opens); a tooltip names
//                                  it
//
// Positions come from InFlight::fraction along the cable's route (ui::dg_point_at) from the
// sender's end (dir 0: from the link's end a, where the route starts).  Everything is drawn
// after ui.diagram with plain commands inside the canvas' clip: the canvas' own painting
// (grid, cables, devices) stays cached, the dirty cells are only those the frames touch.
// Reduced motion draws no tweens and lets the crosses stand.  A frame's label (hit area,
// tooltip) is made once (Labels: decoding costs, hundreds of frames move at once); frames whose
// numbers meet in ID_PKT0 + number % 1024 share no id: the first one shown keeps it.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};

use jfn_sim::{DropMark, Filter, HopId, Ns};
use ui::*;

use super::{cat_color, cat_of, shows, tween_us, FRAMES_MAX};
use crate::app::constants::*;
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::workspace::canvas::paint::{DEV_H, DEV_W};
use crate::workspace::canvas::tools::Tool;

/// crosses: shown this long after they appear (µs), then fading this long
const DROP_SHOW: u64 = 1_200_000;
const DROP_FADE: u64 = 600_000;

/// a frame of a step's animation: where it was and is (0 .. 1 along its cable) and whether it
/// was and is on the cable
#[derive(Clone, Copy, Debug)]
struct TFrame {
    hop: HopId,
    link: u32,
    dir: u8,
    mask: u32,
    p0: f32,
    p1: f32,
    vis0: bool,
    vis1: bool,
}

/// the animation of a step from one moment of the recording to another
pub(crate) struct Tween {
    start: u64,
    dur: u64,
    frames: Vec<TFrame>,
}

impl Tween {
    pub(crate) fn new(n: &Net, from: Ns, to: Ns, now: u64) -> Tween {
        let eng = n.doc.sim.engine();
        let f = Filter::all();
        let (a, b) = (eng.frames_at(from, &f, FRAMES_MAX), eng.frames_at(to, &f, FRAMES_MAX));
        let mut frames: Vec<TFrame> = Vec::new();
        for x in a.iter().chain(b.iter()) {
            if frames.iter().any(|t| t.hop == x.hop) || !shows(n, x.mask, x.from.0, Some(x.to.0)) {
                continue;
            }
            let pos = |t: Ns| {
                if t <= x.t_tx {
                    0.0
                } else if t >= x.t_rx {
                    1.0
                } else {
                    ((t - x.t_tx) as f64 / (x.t_rx - x.t_tx) as f64) as f32
                }
            };
            let vis = |t: Ns| x.t_tx <= t && t < x.t_rx;
            frames.push(TFrame { hop: x.hop, link: x.link, dir: x.dir, mask: x.mask, p0: pos(from), p1: pos(to), vis0: vis(from), vis1: vis(to) });
        }
        Tween { start: now, dur: tween_us(n).max(1), frames }
    }

    /// the animation is over
    pub(crate) fn done(&self, now: u64) -> bool {
        now >= self.start + self.dur
    }

    // how far it got, eased (0 .. 1)
    fn k(&self, now: u64) -> f32 {
        let k = (now.saturating_sub(self.start) as f32 / self.dur as f32).clamp(0.0, 1.0);
        k * k * (3.0 - 2.0 * k)
    }

    // the frames at this point of the animation
    fn items(&self, now: u64) -> Vec<Item> {
        let k = self.k(now);
        let mut v = Vec::new();
        for f in &self.frames {
            let alpha = match (f.vis0, f.vis1) {
                // it leaves its sender: it appears in the last part of the step
                (false, true) => ((k - 0.65) / 0.35).clamp(0.0, 1.0),
                // it reaches its receiver: it fades on arrival
                (true, false) => ((1.0 - k) / 0.25).clamp(0.0, 1.0),
                _ => 1.0,
            };
            if alpha <= 0.0 {
                continue;
            }
            v.push(Item { hop: f.hop, link: f.link, dir: f.dir, mask: f.mask, pos: f.p0 + (f.p1 - f.p0) * k, alpha: (alpha * 255.0) as u32 });
        }
        v
    }
}

/// the red crosses of the moment shown
#[derive(Default)]
pub(crate) struct Drops {
    pub marks: Vec<DropMark>,
    /// when they appear (task clock, µs)
    at: u64,
}

impl Drops {
    /// they still move (appear or fade)
    pub(crate) fn fading(&self, now: u64, anim_off: bool) -> bool {
        !anim_off && !self.marks.is_empty() && now < self.at + DROP_SHOW + DROP_FADE
    }

    // their opacity now (0 .. 255)
    fn alpha(&self, now: u64, anim_off: bool) -> u32 {
        if anim_off {
            return 255;
        }
        if now < self.at {
            return 0;
        }
        let d = now - self.at;
        if d <= DROP_SHOW {
            255
        } else if d >= DROP_SHOW + DROP_FADE {
            0
        } else {
            (255 * (DROP_SHOW + DROP_FADE - d) / DROP_FADE) as u32
        }
    }
}

/// the crosses of moment t: what was dropped since the previous moment a frame left, shown
/// half way through the step's animation
pub(crate) fn drops_at(n: &mut Net, t: Ns, now: u64) {
    let eng = n.doc.sim.engine();
    let prev = eng.trace.prev_send(t, &Filter::all()).unwrap_or(0);
    let marks: Vec<DropMark> = eng
        .drops_between(prev, t, &Filter::all())
        .into_iter()
        .filter(|d| {
            let mask = match (d.hop.and_then(|h| eng.trace.hop(h)), d.seq.and_then(|s| eng.trace.event(s))) {
                (Some(h), _) => h.mask,
                (None, Some(e)) => e.mask(0),
                _ => 0,
            };
            shows(n, mask, d.dev, None)
        })
        .collect();
    let half = if n.sim.tween.is_some() { tween_us(n) / 2 } else { 0 };
    n.sim.drops = Drops { marks, at: now + half };
}

/// a frame to draw
#[derive(Clone, Copy, Debug)]
struct Item {
    hop: HopId,
    link: u32,
    dir: u8,
    mask: u32,
    pos: f32,
    alpha: u32,
}

// the frames on the cables at the moment shown
fn still(n: &Net) -> Vec<Item> {
    let eng = n.doc.sim.engine();
    let t = n.doc.sim.view_time();
    eng.frames_at(t, &Filter::all(), FRAMES_MAX)
        .into_iter()
        .filter(|x| shows(n, x.mask, x.from.0, Some(x.to.0)))
        .map(|x| Item { hop: x.hop, link: x.link, dir: x.dir, mask: x.mask, pos: x.fraction(), alpha: 255 })
        .collect()
}

fn with_alpha(argb: u32, a: u32) -> u32 {
    (argb & 0x00FF_FFFF) | ((argb >> 24) * a.min(255) / 255) << 24
}

/// 24.8 fixed point for draw_line
fn fx(v: f32) -> i32 {
    (v * 256.0) as i32
}

/// what a press on a frame chose, its tooltip
struct Hit {
    hop: HopId,
    rect: [i32; 4],
    label: Rc<str>,
}

/// labels kept at most (then they are made again)
const LABELS_MAX: usize = 4096;

/// the frames' labels "Rahmen N: A → B · Info", made once per frame (decoding it for the info
/// costs) and again when the project changes (device names); a frame is known by its number,
/// moment and cable (a fresh engine numbers from 1 again)
#[derive(Default)]
pub(crate) struct Labels {
    rev: Cell<u32>,
    /// the labels are English (they are made again after a change of the language)
    en: Cell<bool>,
    map: RefCell<BTreeMap<HopId, (Ns, u32, Rc<str>)>>,
}

impl Labels {
    fn get(&self, n: &Net, id: HopId) -> Rc<str> {
        let en = crate::app::tr::en();
        if self.rev.get() != n.doc.rev || self.en.get() != en {
            self.map.borrow_mut().clear();
            self.rev.set(n.doc.rev);
            self.en.set(en);
        }
        let Some(hop) = n.doc.sim.engine().trace.hop(id) else {
            return Rc::from(tf("Rahmen {}", &[&format!("{}", id)]).as_str());
        };
        if let Some((t, l, s)) = self.map.borrow().get(&id) {
            if (*t, *l) == (hop.t_tx, hop.link) {
                return s.clone();
            }
        }
        let name = |d: u32| n.doc.proj.device(d).map_or_else(|| tf("Gerät {}", &[&format!("{}", d)]), |x| x.name.clone());
        let (_, info) = jfn_sim::view::hop_info(hop);
        let s: Rc<str> = Rc::from(tf("Rahmen {}: {} → {} · {}", &[&format!("{}", id), &name(hop.from.0), &name(hop.to.0), &info]).as_str());
        let mut m = self.map.borrow_mut();
        if m.len() >= LABELS_MAX {
            m.clear();
        }
        m.insert(id, (hop.t_tx, hop.link, s.clone()));
        s
    }
}

/// the frames, crosses and rings over the canvas (after ui.diagram)
pub(crate) fn overlay(n: &mut Net, ui: &mut Ui) {
    let vp = n.canvas.st.vp;
    let r = vp.rect;
    if r[2] <= 0 || r[3] <= 0 {
        return;
    }
    ui.clip_push(r[0], r[1], r[2], r[3]);
    pdu_source(n, ui);
    if n.sim.sim_mode {
        let (hits, shown) = frames(n, ui);
        // how many frames are on the cables at the moment shown (screen readers;
        // tests/perf.uis)
        ui.last_set(r, 0);
        ui.sem(K_CANVAS, ID_SIM_FRAMES, Some(t("Rahmen auf den Kabeln")), shown as i32);
        crosses(n, ui);
        let mut held = None;
        // the ids given out in this pass (ID_PKT0 + number % 1024)
        let mut used = [0u64; 16];
        for h in &hits {
            let k = (h.hop % 1024) as usize;
            if used[k / 64] & 1 << (k % 64) != 0 {
                continue;
            }
            used[k / 64] |= 1 << (k % 64);
            let ev = ui.hit_rect(ID_PKT0 + (h.hop % 1024) as u32, h.rect[0], h.rect[1], h.rect[2], h.rect[3], 0);
            ui.sem(K_BUTTON, ID_PKT0 + (h.hop % 1024) as u32, Some(&*h.label), 0);
            if ev & EV_HOVER != 0 {
                ui.tooltip(&h.label);
            }
            if ev & EV_HELD != 0 {
                held = Some(h.hop);
                if n.sim.held != Some(h.hop) {
                    super::select_frame(n, h.hop);
                    ui.repass();
                }
            }
        }
        n.sim.held = held;
    } else {
        n.sim.held = None;
    }
    ui.clip_pop();
}

// the envelopes (dots below 50 %) -> their hit areas, the frames shown (also outside the view)
fn frames(n: &Net, ui: &mut Ui) -> (Vec<Hit>, usize) {
    let now = plat::task::now_us();
    let items = match &n.sim.tween {
        Some(t) if !t.done(now) => t.items(now),
        _ => still(n),
    };
    let st = &n.canvas.st;
    let vp = st.vp;
    let dark = ui.theme_dark();
    let hair = ui.hair();
    let accent = ui.color(T_ACCENT) | 0xFF00_0000;
    let small = vp.zoom < 0.5;
    let z = vp.zoom.clamp(1.0, 1.5);
    let (w, h) = ((ui.dp(12) as f32 * z) as i32, (ui.dp(8) as f32 * z) as i32);
    let dot = ui.dp(3);
    let mut hits = Vec::new();
    let shown = items.iter().filter(|it| it.alpha >= 128).count();
    // the canvas with room for an envelope and its ring
    let r = vp.rect;
    let g = (w + ui.dp(8)) as f32;
    let (x0, y0, x1, y1) = (r[0] as f32 - g, r[1] as f32 - g, (r[0] + r[2]) as f32 + g, (r[1] + r[3]) as f32 + g);
    for it in items {
        let Some(rt) = st.route(it.link as u64) else { continue };
        if rt.pts.len() < 2 {
            continue;
        }
        let f = if it.dir == 0 { it.pos } else { 1.0 - it.pos };
        let p = vp.to_screen(dg_point_at(&rt.pts, f));
        if p[0] < x0 || p[0] > x1 || p[1] < y0 || p[1] > y1 {
            continue;
        }
        let c = cat_color(cat_of(it.mask), dark);
        let fill = with_alpha(c, it.alpha);
        let edge = with_alpha(color_mix(c, 0xFF00_0000, 70), it.alpha);
        let selected = n.sim.sel_hop == Some(it.hop);
        let rect = if small {
            let (x, y) = (p[0] as i32 - dot, p[1] as i32 - dot);
            ui.cmd_rrect(x, y, dot * 2, dot * 2, dot, hair, fill, edge, edge);
            [x - 2, y - 2, dot * 2 + 4, dot * 2 + 4]
        } else {
            let (x, y) = (p[0] as i32 - w / 2, p[1] as i32 - h / 2);
            ui.cmd_rrect(x, y, w, h, ui.dp(2), hair, fill, edge, edge);
            // the flap
            let flap = with_alpha(color_mix(c, 0xFFFF_FFFF, 150), it.alpha);
            ui.draw_color(flap);
            let (x0, y0, x1) = (x as f32 + 1.5, y as f32 + 1.5, (x + w) as f32 - 1.5);
            let (cx, cy) = (x as f32 + w as f32 * 0.5, y as f32 + h as f32 * 0.58);
            let lw = (hair * 256) as i32;
            ui.draw_line(fx(x0), fx(y0), fx(cx), fx(cy), lw);
            ui.draw_line(fx(cx), fx(cy), fx(x1), fx(y0), lw);
            [x - 2, y - 2, w + 4, h + 4]
        };
        if selected {
            let g = ui.dp(2);
            ui.cmd_rrect(rect[0] - g + 2, rect[1] - g + 2, rect[2] + 2 * g - 4, rect[3] + 2 * g - 4, ui.dp(3), hair * 2, 0, accent, accent);
        }
        if it.alpha >= 128 {
            hits.push(Hit { hop: it.hop, rect, label: n.sim.labels.get(n, it.hop) });
        }
    }
    (hits, shown)
}

// the red crosses: at the device's top right corner, or near the receiver on the cable that
// lost the frame
fn crosses(n: &Net, ui: &mut Ui) {
    let now = plat::task::now_us();
    let a = n.sim.drops.alpha(now, ui.anim_off());
    if a == 0 || n.sim.drops.marks.is_empty() {
        return;
    }
    let st = &n.canvas.st;
    let vp = st.vp;
    let eng = n.doc.sim.engine();
    let red = with_alpha(ui.color(T_CRIT) | 0xFF00_0000, a);
    let halo = with_alpha(ui.color(T_BG) | 0xFF00_0000, a * 3 / 4);
    let s = ui.dp(5) as f32;
    let lw = ui.dp(2) * 256;
    for d in &n.sim.drops.marks {
        let at = match (d.lost, d.hop.and_then(|h| eng.trace.hop(h))) {
            (Some(_), Some(h)) => st.route(h.link as u64).filter(|rt| rt.pts.len() >= 2).map(|rt| {
                let forward = n.doc.proj.link(h.link).is_none_or(|l| l.a.dev == h.from.0);
                dg_point_at(&rt.pts, if forward { 0.85 } else { 0.15 })
            }),
            _ => n.doc.proj.device(d.dev).map(|x| [x.x as f32 + DEV_W - 6.0, x.y as f32 + 6.0]),
        };
        let Some(w) = at else { continue };
        let p = vp.to_screen(w);
        let r = (s + 3.0) as i32;
        ui.cmd_rrect(p[0] as i32 - r, p[1] as i32 - r, r * 2, r * 2, r, 0, halo, 0, 0);
        ui.draw_color(red);
        ui.draw_line(fx(p[0] - s), fx(p[1] - s), fx(p[0] + s), fx(p[1] + s), lw);
        ui.draw_line(fx(p[0] - s), fx(p[1] + s), fx(p[0] + s), fx(p[1] - s), lw);
    }
}

// the PDU tool: a ring around the chosen source and a line to the pointer
fn pdu_source(n: &Net, ui: &mut Ui) {
    if n.canvas.tool != Tool::Pdu {
        return;
    }
    let Some(d) = n.sim.pdu.src.and_then(|d| n.doc.proj.device(d)) else { return };
    let vp = n.canvas.st.vp;
    let q = vp.rect_to_screen([d.x as f32, d.y as f32, DEV_W, DEV_H]);
    let g = ui.dp(4) as f32;
    let accent = ui.color(T_ACCENT) | 0xFF00_0000;
    let hair = ui.hair();
    ui.cmd_rrect((q[0] - g) as i32, (q[1] - g) as i32, (q[2] + 2.0 * g) as i32, (q[3] + 2.0 * g) as i32, ui.dp(4), hair * 2, 0, accent, accent);
    let (mx, my) = ui.mouse();
    let (cx, cy) = (q[0] + q[2] * 0.5, q[1] + q[3] * 0.5);
    ui.draw_color(with_alpha(accent, 160));
    ui.draw_line(fx(cx), fx(cy), fx(mx as f32 + 0.5), fx(my as f32 + 0.5), hair * 256);
}
