// events -- the Ereignisse panel: the protocol chips and "Nur Auswahl", the timeline with one
// lane per category and the playhead, the event list (#, Zeit, Von, Bei, Typ, Info)
//
//   Events              the grid's and the timeline's state; the recorded decisions in a
//                       compact form, read on as the engine adds them; the shown ones and the
//                       timeline's ticks, rebuilt when the filters change
//   frame(n, ui)        the panel
//   follow(n, t)        the view moved to t: the last shown event at or before t is chosen and
//                       scrolled to; the playhead stands at t
//   reveal(seq)         scroll a line into view (next frame)
//   shown()             the number of shown events (status bar)
//
// Both ways in step: a chosen line moves the view (in the simulation mode) and the playhead to
// its moment, and the packet tab shows it; moving the playhead goes to the simulation mode,
// shows that moment and chooses the last line at or before it.  In real time the playhead
// follows the live end and the list stays at its end while it is there.  The lines carry the
// engine's numbers; times are seconds with six decimals; "Von" is the device the frame came
// from, "Bei" the one that decided; "Typ" the protocol the decision is about (the question a
// router asks before it forwards a ping is ARP), with its category's colour; what was dropped
// says so.  The lab's faults (Engine::faults) have a lane of their own on the timeline: a tick
// in the error colour where each one started and where it was healed.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_sim::Ns;
use ui::*;

use super::{cat_color, cat_of, hidden, CATS, CAT_OTHER};
use crate::app::constants::*;
use crate::app::tr::{t, tf};
use crate::app::Net;

/// a recorded decision as the list needs it
#[derive(Clone, Copy, Debug)]
struct Row {
    seq: u64,
    t: Ns,
    /// the device the frame came from (0: none, the device acted on its own)
    from: u32,
    dev: u32,
    /// the protocols the decision is about (its own, else its frame's)
    own: u32,
}

// the columns in the current language
fn cols() -> [GridCol<'static>; 6] {
    [
        GridCol::new("#", 56).flags(GC_RIGHT | GC_NOSORT),
        GridCol::new(t("Zeit (s)"), 128).flags(GC_RIGHT | GC_NOSORT),
        GridCol::new(t("Von"), 84).flags(GC_NOSORT),
        GridCol::new(t("Bei"), 84).flags(GC_NOSORT),
        GridCol::new(t("Typ"), 92).flags(GC_NOSORT),
        GridCol::new(t("Info"), 240).flags(GC_FILL | GC_NOSORT),
    ]
}

pub(crate) struct Events {
    pub grid: GridState,
    pub tl: TimelineState,
    rows: Vec<Row>,
    /// the last decision read
    last: u64,
    /// the shown decisions (numbers) and their rows
    keys: Vec<u64>,
    at: Vec<u32>,
    /// what the shown ones were chosen by: hidden categories, devices; rows looked at
    made_for: Option<(u16, Vec<u32>)>,
    seen: usize,
    /// the timeline's ticks per category (s, colour)
    ticks: Vec<Vec<(f64, u32)>>,
    ticks_dark: bool,
    /// the timeline's height (DIP; 0: as high as its lanes need)
    tl_h: i32,
    /// the timeline's view as the simulation mode set it last (the user's own when it differs)
    auto: Option<(f64, f64)>,
    /// a line to scroll to (and passes left to try); the list stays at its end (real time)
    reveal: Option<(u64, u8)>,
    at_end: bool,
    /// the measured height of a line and of the list (DIP)
    row_h: i32,
    view_h: i32,
}

impl Events {
    pub(crate) fn new() -> Events {
        let mut grid = GridState::default();
        grid.striped = true;
        let mut tl = TimelineState::default();
        tl.follow = true;
        Events {
            grid,
            tl,
            rows: Vec::new(),
            last: 0,
            keys: Vec::new(),
            at: Vec::new(),
            made_for: None,
            seen: 0,
            ticks: (0..CATS.len()).map(|_| Vec::new()).collect(),
            ticks_dark: false,
            tl_h: 0,
            auto: None,
            reveal: None,
            at_end: true,
            row_h: 22,
            view_h: 0,
        }
    }

    /// another engine: nothing read yet
    pub(crate) fn reset(&mut self) {
        let tl_h = self.tl_h;
        *self = Events::new();
        self.tl_h = tl_h;
    }

    /// scroll the line of event `seq` into view
    pub(crate) fn reveal(&mut self, seq: u64) {
        self.reveal = Some((seq, 3));
    }

    /// the number of shown events
    pub(crate) fn shown(&self) -> usize {
        self.keys.len()
    }

    // the shown line of event seq
    fn line(&self, seq: u64) -> Option<usize> {
        self.keys.binary_search(&seq).ok()
    }

    // the view of the simulation mode at playhead t: the shown ticks (and t while it lies
    // among them or shortly after), a twentieth more at both ends; inside the span 0 .. end
    // as the timeline keeps it (so it stays the same pass after pass)
    fn fit(&self, t: f64, end: f64) -> (f64, f64) {
        let lo = self.ticks.iter().filter_map(|v| v.first()).map(|x| x.0).fold(f64::MAX, f64::min);
        let hi = self.ticks.iter().filter_map(|v| v.last()).map(|x| x.0).fold(0.0, f64::max);
        let (a, b) = if lo > hi {
            (0.0, end)
        } else {
            let lo = lo.min(t);
            let w = (hi - lo).max(1e-8);
            let hi = if t <= hi + w { hi.max(t) } else { hi };
            let w = (hi - lo).max(1e-8);
            ((lo - w * 0.05).max(0.0), hi + w * 0.05)
        };
        // the timeline's own clamping: at most the span, at least 10 ns, inside the span
        let w = (b - a).min(end).max(1e-8f64.min(end));
        let a = if a + w > end { end - w } else { a };
        let a = if a < 0.0 { 0.0 } else { a };
        (a, a + w)
    }

    // the last shown event at or before t
    fn last_at(&self, t: Ns) -> Option<u64> {
        let i = self.at.partition_point(|&r| self.rows[r as usize].t <= t);
        i.checked_sub(1).map(|i| self.keys[i])
    }
}

fn secs_f(t: Ns) -> f64 {
    t as f64 / 1e9
}

/// the new decisions of the engine, the shown ones and the ticks (every frame)
pub(crate) fn refresh(n: &mut Net, dark: bool) {
    let h = hidden(n);
    let devs = super::devices(n).to_vec();
    let eng = n.doc.sim.engine();
    let ev = &mut n.sim.ev;
    // the ring of the recording dropped its oldest ones
    let first = eng.trace.events().next().map_or(u64::MAX, |e| e.seq);
    let mut rebuild = ev.made_for.as_ref() != Some(&(h, devs.clone())) || ev.ticks_dark != dark;
    if ev.rows.first().is_some_and(|r| r.seq < first) {
        ev.rows.retain(|r| r.seq >= first);
        rebuild = true;
    }
    for e in eng.trace.events_after(ev.last) {
        let hop = e.hop.and_then(|id| eng.trace.hop(id));
        let hm = hop.map_or(0, |x| x.mask);
        // IPv6 in place of IPv4 for decisions about IPv6 traffic (DevEvent::mask)
        let nm = e.mask(hm);
        let own = if nm & !hm != 0 { nm & !hm } else { hm | nm };
        ev.rows.push(Row { seq: e.seq, t: e.t, from: hop.map_or(0, |x| x.from.0), dev: e.dev, own });
    }
    ev.last = eng.trace.next_event_seq().saturating_sub(1);
    if rebuild {
        ev.keys.clear();
        ev.at.clear();
        for t in ev.ticks.iter_mut() {
            t.clear();
        }
        ev.seen = 0;
        ev.made_for = Some((h, devs.clone()));
        ev.ticks_dark = dark;
    }
    for (i, r) in ev.rows.iter().enumerate().skip(ev.seen) {
        let k = cat_of(r.own);
        if h & 1 << k != 0 || !(devs.is_empty() || devs.contains(&r.dev) || (r.from != 0 && devs.contains(&r.from))) {
            continue;
        }
        ev.keys.push(r.seq);
        ev.at.push(i as u32);
        ev.ticks[k].push((secs_f(r.t), cat_color(k, dark)));
    }
    ev.seen = ev.rows.len();
}

/// the view moved to t: the playhead, the last shown event at or before it chosen and shown
pub(crate) fn follow(n: &mut Net, t: Ns) {
    // the decisions of the step first
    let dark = n.sim.ev.ticks_dark;
    refresh(n, dark);
    let past = n.doc.sim.view_at().is_some();
    let ev = &mut n.sim.ev;
    ev.tl.t = secs_f(t);
    ev.tl.follow = !past;
    let seq = ev.last_at(t);
    super::select_event(n, seq);
    if let Some(s) = seq {
        n.sim.ev.reveal(s);
    }
}

/// the Ereignisse panel
pub(crate) fn frame(n: &mut Net, ui: &mut Ui) {
    let dark = ui.theme_dark();
    refresh(n, dark);
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 0);
    chips(n, ui);
    // the timeline as high as its lanes (14 DIP each below the axis), unless dragged
    let lanes = (n.sim.ev.ticks.iter().filter(|t| !t.is_empty()).count() + !n.doc.sim.engine().faults().is_empty() as usize).max(1) as i32;
    let mut tl_h = if n.sim.ev.tl_h > 0 { n.sim.ev.tl_h } else { (28 + 16 * lanes).clamp(70, 160) };
    ui.next_size(-1, -1);
    ui.split_begin(ID_EV_TL_SPLIT, tl_h, true);
    timeline(n, ui);
    ui.split_next();
    grid(n, ui);
    if ui.split_end(&mut tl_h) {
        n.sim.ev.tl_h = tl_h;
    }
    ui.box_end();
}

// the chips of the categories, "Nur Auswahl", the count
fn chips(n: &mut Net, ui: &mut Ui) {
    let h = hidden(n);
    let dark = ui.theme_dark();
    ui.box_begin(0, BOX_H | BOX_XCENTER | pad(4), 4);
    ui.text(TS_CAPTION | TX_SECONDARY, t("Protokolle"));
    for (k, c) in CATS.iter().enumerate() {
        let mut on = (h & 1 << k == 0) as i32;
        ui.next_flags(ST_COMPACT);
        if ui.toggle_button(ID_EV_CHIP0 + k as u32, t(c.name), &mut on) & EV_ACT != 0 {
            ui.cmd_run(CMD_SIM_CAT0 + k as u32, 0);
        }
        // the category's colour as a bar along the chip's bottom edge
        let r = ui.last_rect();
        let col = if on != 0 { cat_color(k, dark) } else { color_mix(cat_color(k, dark), ui.color(T_BG) | 0xFF00_0000, 170) };
        let bh = ui.dp(3);
        ui.cmd_rrect(r[0] + ui.dp(6), r[1] + r[3] - bh - ui.dp(3), r[2] - ui.dp(12), bh, bh / 2, 0, col, 0, 0);
        ui.tooltip(t(if k == CAT_OTHER { "Ethernet ohne bekanntes Protokoll, LLDP, IPv6, IGMP" } else { "Ein- oder ausblenden: Schritte, Liste, Zeitleiste und Leinwand" }));
    }
    ui.next_size(1, 20);
    ui.separator();
    let mut only = n.sim.only_sel as i32;
    ui.next_flags(ST_COMPACT);
    if ui.toggle_button(ID_EV_ONLY_SEL, t("Nur Auswahl"), &mut only) & EV_ACT != 0 {
        ui.cmd_run(CMD_SIM_ONLY_SEL, 0);
    }
    let names: Vec<String> = n.sim.sel_devs.iter().filter_map(|&d| n.doc.proj.device(d).map(|x| x.name.clone())).collect();
    ui.tooltip(&if n.sim.only_sel && !names.is_empty() { tf("Nur: {}", &[&names.join(", ")]) } else { String::from(t("Nur die auf der Leinwand gewählten Geräte")) });
    ui.spacer(-1);
    let all = n.sim.ev.rows.len();
    let k = n.sim.ev.keys.len();
    ui.text(TS_CAPTION | TX_SECONDARY, &tf("{} von {}", &[&k.to_string(), &all.to_string()]));
    ui.tooltip(t("Gezeigte Ereignisse von allen aufgezeichneten"));
    ui.spacer(4);
    ui.box_end();
}

// the timeline: a lane per category with events, the playhead at the moment shown
fn timeline(n: &mut Net, ui: &mut Ui) {
    let now = n.doc.sim.engine().now();
    let shown = n.doc.sim.view_time();
    let sim = n.sim.sim_mode;
    let ev = &mut n.sim.ev;
    // real time: the playhead follows the live end over the whole span.  The simulation mode:
    // the playhead stands at the moment shown, and the view fits the recorded ticks (the
    // playhead with them while it is among them) until the user zooms or pans
    let auto = if sim {
        ev.tl.follow = false;
        ev.tl.t = secs_f(shown);
        let fit = ev.fit(secs_f(shown), secs_f(now).max(1e-6));
        let mine = ev.auto.is_none_or(|a| a == ev.tl.view) || ev.tl.view == (0.0, 0.0);
        if mine && ev.tl.view != fit {
            ev.tl.view = fit;
        }
        mine
    } else {
        ev.tl.follow = true;
        if ev.auto.take().is_some_and(|a| a == ev.tl.view) {
            ev.tl.view = (0.0, 0.0);
        }
        false
    };
    // the lab's faults (U6a): a lane with their starts and ends in the error colour
    let crit = ui.color(T_CRIT);
    let mut faults: Vec<(f64, u32)> = n.doc.sim.engine().faults().iter().flat_map(|m| core::iter::once(m.at).chain(m.healed)).map(|x| (secs_f(x), crit)).collect();
    faults.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut tracks: Vec<TlTrack> = Vec::new();
    for (k, c) in CATS.iter().enumerate() {
        if !ev.ticks[k].is_empty() {
            tracks.push(TlTrack { label: t(c.name), ticks: &ev.ticks[k] });
        }
    }
    if !faults.is_empty() {
        tracks.push(TlTrack { label: crate::app::tr::tc("Fehler|Zeitleiste"), ticks: &faults });
    }
    if tracks.is_empty() {
        tracks.push(TlTrack { label: t("Ereignisse"), ticks: &[] });
    }
    ui.next_size(-1, -1);
    let e = ui.timeline(ID_EV_TIMELINE, &mut ev.tl, (0.0, secs_f(now).max(1e-6)), &tracks);
    if auto {
        // as the widget keeps it (clamped to the span)
        ev.auto = Some(ev.tl.view);
    }
    if e & EV_ACT != 0 {
        let t = (n.sim.ev.tl.t.max(0.0) * 1e9) as Ns;
        super::seek(n, ui, t.min(now));
    }
}

// the event list
fn grid(n: &mut Net, ui: &mut Ui) {
    // a line to show: scroll it into view (the grid's rows scroll in the view ID_EV_GRID); the
    // scroll view keeps to the last pass' height, so a new last line takes another pass
    let ev = &mut n.sim.ev;
    if let Some((seq, tries)) = ev.reveal {
        ev.reveal = None;
        if let Some(i) = ev.line(seq) {
            let (_, y) = ui.scroll_pos(ID_EV_GRID);
            let (top, rh) = (i as i32 * ev.row_h, ev.row_h);
            let vh = ev.view_h.max(rh);
            let want = if top < y {
                Some(top)
            } else if top + rh > y + vh {
                Some(top + rh - vh)
            } else {
                None
            };
            if let Some(w) = want {
                ui.scroll_to(ID_EV_GRID, -1, w);
                if tries > 0 {
                    ev.reveal = Some((seq, tries - 1));
                }
            }
        }
    } else if !n.sim.sim_mode && ev.at_end && !ev.keys.is_empty() {
        // real time: the list stays at its end
        let end = ev.keys.len() as i32 * ev.row_h;
        let (_, y) = ui.scroll_pos(ID_EV_GRID);
        if y + ev.view_h.max(ev.row_h) < end {
            ui.scroll_to(ID_EV_GRID, -1, (end - ev.view_h).max(0));
        }
    }
    if n.sim.ev.keys.is_empty() {
        let text = t(if n.sim.ev.rows.is_empty() {
            "Hier stehen die Entscheidungen der Geräte Schritt für Schritt: wer einen Rahmen bekam, was er damit tat. Eine PDU (Werkzeug P) oder ein ping auf einer Konsole füllt die Liste."
        } else {
            "Die gewählten Protokolle und Geräte haben keine Ereignisse."
        });
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER | pad(12), 0);
        ui.empty_state(IC_BULLETED_LIST, t("Noch keine Ereignisse"), text);
        ui.box_end();
        return;
    }
    let dark = ui.theme_dark();
    let keys = n.sim.ev.keys.clone();
    let mut st = core::mem::take(&mut n.sim.ev.grid);
    ui.next_size(-1, -1);
    let cols = cols();
    let (first, end) = ui.grid_begin(ID_EV_GRID, &mut st, &cols, &keys);
    let eng = n.doc.sim.engine();
    let p = &n.doc.proj;
    let name = |d: u32| p.device(d).map_or_else(|| eng.node(d).map_or_else(|| tf("Gerät {}", &[&d.to_string()]), |x| x.name.clone()), |x| x.name.clone());
    let (_, scroll) = ui.scroll_pos(ID_EV_GRID);
    let mut row_h = 0;
    // the top of the lines' view (px): a line's top less its place in the list
    let mut view_top = 0;
    for i in first..end {
        let r = n.sim.ev.rows[n.sim.ev.at[i as usize] as usize];
        ui.grid_row(&mut st, i);
        let lr = ui.last_rect();
        if row_h == 0 {
            row_h = lr[3];
            view_top = lr[1] - i * lr[3] + ui.dp(scroll);
        }
        ui.grid_text(&st, 0, &format!("{}", r.seq));
        ui.grid_text(&st, 1, &super::secs(r.t));
        ui.grid_text_ts(&st, 2, TS_BODY | if r.from == 0 { TX_TERTIARY } else { 0 }, &if r.from == 0 { String::from("–") } else { name(r.from) });
        ui.grid_text(&st, 3, &name(r.dev));
        // the line of the engine (Engine::event_row): the type in its category's colour, the
        // decision in words
        let er = eng.trace.event(r.seq).map(|e| eng.event_row(e));
        ui.grid_cell(&st, 4);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        let d = ui.dp(8);
        let q = ui.place(d, d);
        ui.cmd_rrect(q[0], q[1], d, d, ui.dp(2), 0, cat_color(cat_of(r.own), dark), 0, 0);
        let proto = er.as_ref().map_or("", |x| x.proto);
        ui.text(TS_BODY | TX_ELLIPSIS, if proto.is_empty() { t("Leitung") } else { proto });
        ui.box_end();
        let text = match er {
            Some(x) if x.drop => tf("× verworfen: {}", &[&crate::app::tr::msg(&x.msg)]),
            Some(x) => crate::app::tr::msg(&x.msg),
            None => String::from(t("(nicht mehr aufgezeichnet)")),
        };
        ui.grid_text(&st, 5, &text);
    }
    let ge = ui.grid_end(&mut st);
    let outer = ui.last_rect();
    let ev = &mut n.sim.ev;
    if row_h > 0 {
        ev.row_h = ui.px2dip(row_h).max(1);
        ev.view_h = ui.px2dip(outer[1] + outer[3] - view_top).max(ev.row_h);
    }
    let (_, y) = ui.scroll_pos(ID_EV_GRID);
    ev.at_end = y + ev.view_h + ev.row_h >= ev.keys.len() as i32 * ev.row_h;
    n.sim.ev.grid = st;
    if ge.ev & EV_ACT != 0 {
        if let Some(seq) = n.sim.ev.grid.cur {
            choose(n, ui, seq);
        }
    }
    if ge.ev & EV_DOUBLE != 0 {
        n.shell.dock.focus(P_PACKET);
        ui.repass();
    }
}

// a line was chosen: its event; in the simulation mode the view goes to its moment
fn choose(n: &mut Net, ui: &mut Ui, seq: u64) {
    let Some(i) = n.sim.ev.line(seq) else { return };
    let t = n.sim.ev.rows[n.sim.ev.at[i] as usize].t;
    n.sim.sel_ev = Some(seq);
    n.sim.sel_hop = None;
    if n.sim.sim_mode {
        let _ = n.doc.sim.send(jfn_sim::Cmd::Seek(t));
        n.sim.tween = None;
        super::anim::drops_at(n, t, plat::task::now_us());
        n.sim.ev.tl.t = secs_f(t);
    }
    ui.repass();
}
