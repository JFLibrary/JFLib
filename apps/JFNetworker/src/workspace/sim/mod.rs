// sim -- the simulation mode: real time or simulation, playing, steps forward and back along
// the recording, the speed, the protocols and devices that count, the hint while the view
// stands in the past, the status, the frame and event selected
//
//   SimUi                           mode, playing, speed, "Nur Auswahl", the step animation,
//                                   the red crosses, the selection, the panels' states
//   loaded(n, keep_pdus)            a new or loaded project, or a fresh engine (Simulation
//                                   zurücksetzen): real time at the chosen speed, playing
//   states(n, ui) / command(..)     the commands CMD_SIM_* (50 .. 55) and 160 .. 199
//   toolbar(n, ui)                  Echtzeit | Simulation, play / pause, step back, step, the
//                                   speed, the PDU tools
//   hint(n, ui)                     above the canvas while the view stands in the past:
//                                   "Ansicht bei t = …", "Von hier fortsetzen" and "Zurück
//                                   zu live"
//   resume_dialog(n, ui)            the question before "Von hier fortsetzen" (K1)
//   status(n, ui)                   mode, clock and events in the status bar
//   timer(n, ui) / tick(n, ui)      the animation timer (TIMER_ANIM): it runs only while
//                                   something moves or plays
//   step / back / seek / live       move the view (the toolbar, keys, the event list, the
//                                   timeline, scripts)
//   select_event / select_frame     the selection the canvas, the event list and the packet
//                                   tab share
//   filter(n) / shows(n, ..)        the engine filter of the shown protocols and devices
//   CATS / cat_of(mask) / cat_color the protocol categories and their colours
//
// A frame belongs to the category of its top protocol: ICMP green, ARP amber, DNS violet,
// TCP/HTTP blue, UDP cyan, DHCP teal, STP grey, OSPF/RIP brown, the rest slate -- muted,
// lighter in the dark theme, each at least 3:1 against the canvas in both themes (WCAG for
// graphical objects; checked by the selftest's group ui).  Hiding a category hides its frames everywhere: the canvas, the
// list and the timeline leave them out, and steps go past them (the engine's filter passes
// frames carrying one of the shown categories' bits; a step that lands on a frame of a hidden
// category goes on).  The hidden categories are the project's layout.filters, saved with it.
//
// Echtzeit: the engine follows the wall clock times the speed; pause stops it.  Simulation:
// the engine stands; a step runs it to the next moment a shown frame leaves (or walks the
// recording while the view stands in the past), play steps by itself (one step a second at
// 1×), back moves the view along the recording; the devices show their state at the moment
// shown (K1: keyframes and a replay, doc/sim.rs; the hint says while it is computed).  Steps
// tween the frames on the canvas from the old moment to the new one (anim.rs), reduced motion
// jumps.  Moving the timeline's playhead or choosing a line of the event list goes to the
// simulation mode and shows that moment.
//
// "Von hier fortsetzen" (K1) asks first: the state shown becomes the present, the recording
// after it and what came from outside after it (console lines, PDUs, faults) go; the project
// and its undo list stay as they are, so a change of the project made after that moment takes
// effect at it, and undo and redo go on working on the project (a fault step whose fault is
// gone finds nothing to heal).  PDUs started later are shown as gone, the event list and the
// packet tab start afresh, consoles and the log say where the run goes on.
//
// anim.rs draws the frames on the canvas, events.rs the Ereignisse panel (chips, timeline,
// grid, the PDU list), packet.rs the Paket panel, pdu.rs makes PDUs and exports the recording.

pub(crate) mod anim;
pub(crate) mod events;
pub(crate) mod packet;
pub(crate) mod pdu;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_sim::{Cmd, Filter, HopId, Mode, Ns};
use netpkt::decode::*;
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::workspace::canvas::tools::{self, Tool};

/// the speeds (per mille of real time) and their labels (German; shown through t())
// i18n
pub(crate) const SPEEDS: [(u32, &str); 6] = [(250, "0,25×"), (500, "0,5×"), (1000, "1×"), (2000, "2×"), (4000, "4×"), (8000, "8×")];
/// the speed after the start (1×)
const TEMPO_START: usize = 2;
/// steps over frames of hidden categories at most this often per step
const STEP_TRIES: usize = 64;
/// frames looked at per moment
pub(crate) const FRAMES_MAX: usize = 512;
/// the bottom dock gets at least this height (DIP) when the simulation mode starts
const MIN_PANEL: i32 = 300;

/// a protocol category: its chip, its colour (light theme), the P_* bits of its protocols
pub(crate) struct Cat {
    pub name: &'static str,
    pub bits: u32,
    pub argb: u32,
}

pub(crate) const CATS: [Cat; 10] = [
    Cat { name: "ICMP", bits: P_ICMP | P_ICMPV6, argb: 0xFF3F_8A52 },
    Cat { name: "ARP", bits: P_ARP, argb: 0xFFAA_7414 },
    Cat { name: "DNS", bits: P_DNS, argb: 0xFF7A_5C99 },
    Cat { name: "TCP/HTTP", bits: P_TCP | P_HTTP | P_TLS | P_SSH, argb: 0xFF35_68A8 },
    Cat { name: "UDP", bits: P_UDP | P_NTP | P_SYSLOG | P_TFTP | P_SNMP, argb: 0xFF2A_8AA6 },
    Cat { name: "DHCP", bits: P_DHCP | P_DHCPV6, argb: 0xFF2F_7D72 },
    Cat { name: "STP", bits: P_STP, argb: 0xFF7A_818A },
    Cat { name: "OSPF/RIP", bits: P_OSPF | P_RIP | P_BGP, argb: 0xFF8C_6A4F },
    Cat { name: "Andere", bits: P_ETH | P_VLAN | P_LLC | P_IPV4 | P_IPV6 | P_IGMP | P_LLDP | P_DATA, argb: 0xFF6A_737D },
    // SMTP and POP3 (X1c): a muted berry red, apart from DNS's violet and ARP's amber
    Cat { name: "Mail", bits: P_MAIL, argb: 0xFF9B_4D69 },
];
/// the category "Andere"
pub(crate) const CAT_OTHER: usize = 8;

/// the category of a frame or decision: its top protocol's (an ICMP error quoting a UDP packet
/// is ICMP)
pub(crate) fn cat_of(mask: u32) -> usize {
    const ORDER: [usize; 9] = [0, 1, 5, 2, 7, 6, 9, 3, 4];
    ORDER.iter().copied().find(|&k| mask & CATS[k].bits != 0).unwrap_or(CAT_OTHER)
}

/// the colour of a category (lighter in the dark theme), opaque
pub(crate) fn cat_color(k: usize, dark: bool) -> u32 {
    let c = CATS[k.min(CATS.len() - 1)].argb;
    if dark { color_mix(c, 0xFFFF_FFFF, 72) } else { c }
}

pub(crate) struct SimUi {
    /// the simulation mode (else real time)
    pub sim_mode: bool,
    /// the clock runs (real time) / steps follow each other (simulation)
    pub playing: bool,
    /// index into SPEEDS
    pub tempo: usize,
    /// steps, list and canvas only for the selected devices
    pub only_sel: bool,
    /// the devices "Nur Auswahl" means (the canvas' selection when it was switched on)
    pub sel_devs: Vec<u32>,
    /// the step animation (anim.rs)
    pub tween: Option<anim::Tween>,
    /// the red crosses of the moment shown (anim.rs)
    pub drops: anim::Drops,
    /// the frame and the event selected (canvas, list, packet tab)
    pub sel_hop: Option<HopId>,
    pub sel_ev: Option<u64>,
    /// a press on a frame of the canvas
    pub held: Option<HopId>,
    /// playing found nothing more to do (until something changes)
    pub stalled: bool,
    /// the last step of playing (task clock, µs)
    last_step: u64,
    /// the animation timer is set
    armed: bool,
    /// the dock's changes for the simulation mode still to come (one per frame: show the
    /// PDUs, the event list, the bottom's height)
    grow: u8,
    pub ev: events::Events,
    pub pk: packet::Packet,
    pub pdu: pdu::Pdus,
    /// the labels of the frames on the canvas (anim.rs)
    pub labels: anim::Labels,
    /// the question "Von hier fortsetzen?" is open (K1)
    pub ask: Shared<i32>,
}

impl SimUi {
    pub(crate) fn new() -> SimUi {
        SimUi {
            sim_mode: false,
            playing: true,
            tempo: TEMPO_START,
            only_sel: false,
            sel_devs: Vec::new(),
            tween: None,
            drops: anim::Drops::default(),
            sel_hop: None,
            sel_ev: None,
            held: None,
            stalled: false,
            last_step: 0,
            armed: false,
            grow: 0,
            ev: events::Events::new(),
            pk: packet::Packet::new(),
            pdu: pdu::Pdus::new(),
            labels: anim::Labels::default(),
            ask: Shared::new(0),
        }
    }
}

/// the speed in per mille of real time
pub(crate) fn speed(n: &Net) -> u32 {
    SPEEDS[n.sim.tempo.min(SPEEDS.len() - 1)].0
}

/// one step of playing (µs of the task clock): a second at 1×
fn period_us(n: &Net) -> u64 {
    1_000_000_000 / speed(n).max(1) as u64
}

/// the length of a step's animation (µs): four fifths of a step of playing, at most 1,2 s
pub(crate) fn tween_us(n: &Net) -> u64 {
    (period_us(n) * 4 / 5).min(1_200_000)
}

/// a new or loaded project, or a fresh engine: real time at the chosen speed, playing, nothing
/// selected; the PDU list goes with another project (a fresh engine keeps it, its runs ended)
pub(crate) fn loaded(n: &mut Net, keep_pdus: bool) {
    let s = &mut n.sim;
    s.sim_mode = false;
    s.playing = true;
    s.tween = None;
    s.drops = anim::Drops::default();
    s.sel_hop = None;
    s.sel_ev = None;
    s.held = None;
    s.stalled = false;
    s.sel_devs.clear();
    s.only_sel = false;
    s.ev.reset();
    s.pk.reset();
    s.pdu.engine_gone(keep_pdus);
    let speed_pm = speed(n);
    let _ = n.doc.sim.send(Cmd::Realtime { speed_pm });
}

// ---------------------------------------------------------------- filters

/// the hidden categories (bits by index in CATS), from the project's layout.filters
pub(crate) fn hidden(n: &Net) -> u16 {
    let mut h = 0u16;
    for f in &n.doc.proj.layout.filters {
        if let Some(k) = CATS.iter().position(|c| c.name.eq_ignore_ascii_case(f.trim())) {
            h |= 1 << k;
        }
    }
    h
}

/// show or hide a category (the project keeps it; no undo step, like the view)
fn toggle_cat(n: &mut Net, k: usize) {
    let name = CATS[k].name;
    let f = &mut n.doc.proj.layout.filters;
    if let Some(i) = f.iter().position(|x| x.eq_ignore_ascii_case(name)) {
        f.remove(i);
    } else {
        f.push(String::from(name));
        // in the order of the chips
        f.sort_by_key(|x| CATS.iter().position(|c| c.name.eq_ignore_ascii_case(x)).unwrap_or(CATS.len()));
    }
}

/// the devices "Nur Auswahl" keeps to (empty: every device)
pub(crate) fn devices(n: &Net) -> &[u32] {
    if n.sim.only_sel { &n.sim.sel_devs } else { &[] }
}

/// the engine's filter: the shown categories' protocols and the devices
pub(crate) fn filter(n: &Net) -> Filter {
    let h = hidden(n);
    let mask = if h == 0 {
        0
    } else {
        let m = CATS.iter().enumerate().filter(|(k, _)| h & 1 << k == 0).fold(0, |m, (_, c)| m | c.bits);
        // everything hidden: a bit no frame carries
        if m == 0 { 1 << 30 } else { m }
    };
    Filter { mask, devices: devices(n).to_vec() }
}

/// a frame (or decision) of these protocols, between these devices, is shown
pub(crate) fn shows(n: &Net, mask: u32, a: u32, b: Option<u32>) -> bool {
    if hidden(n) & 1 << cat_of(mask) != 0 {
        return false;
    }
    let d = devices(n);
    d.is_empty() || d.contains(&a) || b.is_some_and(|b| d.contains(&b))
}

// a frame that is shown left at moment t
fn sent_shown(n: &Net, t: Ns) -> bool {
    let eng = n.doc.sim.engine();
    eng.frames_at(t, &Filter::all(), FRAMES_MAX).iter().any(|x| x.t_tx == t && shows(n, x.mask, x.from.0, Some(x.to.0)))
}

// ---------------------------------------------------------------- moving the view

/// real time or the simulation mode
pub(crate) fn set_mode(n: &mut Net, sim: bool) {
    if n.sim.sim_mode == sim {
        return;
    }
    n.sim.sim_mode = sim;
    n.sim.tween = None;
    n.sim.stalled = false;
    if sim {
        n.sim.playing = false;
        let _ = n.doc.sim.send(Cmd::Simulation);
        // the event list and the PDUs come forward and the bottom grows tall enough for the
        // timeline and the lines -- in the frames after this one (the animation timer), one
        // change of the dock at a time: together with the mode they would not settle
        n.sim.grow = 3;
    } else {
        n.sim.playing = true;
        let speed_pm = speed(n);
        let _ = n.doc.sim.send(Cmd::Realtime { speed_pm });
    }
    n.sim.drops = anim::Drops::default();
}

/// play / pause
fn play(n: &mut Net, ui: &mut Ui) {
    n.sim.playing = !n.sim.playing;
    n.sim.stalled = false;
    if !n.sim.sim_mode {
        let c = if n.sim.playing { Cmd::Realtime { speed_pm: speed(n) } } else { Cmd::Pause };
        let _ = n.doc.sim.send(c);
    } else if n.sim.playing && n.sim.tween.is_none() {
        n.sim.last_step = plat::task::now_us();
        if !step(n, ui) {
            n.sim.stalled = true;
        }
    }
}

/// a step forward: to the next moment a shown frame leaves -> the view moved
pub(crate) fn step(n: &mut Net, ui: &mut Ui) -> bool {
    set_mode(n, true);
    let f = filter(n);
    let from = n.doc.sim.view_time();
    let mut moved = false;
    for _ in 0..STEP_TRIES {
        let (t0, h0) = (n.doc.sim.view_time(), n.doc.sim.engine().trace.next_hop_id());
        let _ = n.doc.sim.send(Cmd::Step(f.clone()));
        let (t1, h1) = (n.doc.sim.view_time(), n.doc.sim.engine().trace.next_hop_id());
        if t1 == t0 && h1 == h0 {
            break; // nothing happens any more
        }
        moved = true;
        // a frame that is shown, or the engine ran its budget without sending anything
        if sent_shown(n, t1) || (h1 == h0 && n.doc.sim.view_at().is_none()) {
            break;
        }
    }
    if moved {
        arrived(n, ui, from, true);
    }
    moved
}

/// a step back along the recording -> the view moved
pub(crate) fn back(n: &mut Net, ui: &mut Ui) -> bool {
    set_mode(n, true);
    let f = filter(n);
    let from = n.doc.sim.view_time();
    let mut moved = false;
    for _ in 0..STEP_TRIES {
        let t0 = n.doc.sim.view_time();
        let _ = n.doc.sim.send(Cmd::StepBack(f.clone()));
        let t1 = n.doc.sim.view_time();
        if t1 == t0 {
            break;
        }
        moved = true;
        if sent_shown(n, t1) {
            break;
        }
    }
    if moved {
        arrived(n, ui, from, true);
    }
    moved
}

/// the view at moment t of the recording (the simulation mode)
pub(crate) fn seek(n: &mut Net, ui: &mut Ui, t: Ns) {
    set_mode(n, true);
    let from = n.doc.sim.view_time();
    let _ = n.doc.sim.send(Cmd::Seek(t));
    arrived(n, ui, from, false);
}

/// the view back to the present
pub(crate) fn live(n: &mut Net, ui: &mut Ui) {
    let from = n.doc.sim.view_time();
    let _ = n.doc.sim.send(Cmd::Live);
    if n.sim.sim_mode {
        arrived(n, ui, from, false);
    } else {
        n.sim.tween = None;
    }
}

// the view moved from `from` to the view time: the animation (a step), the red crosses, the
// list and the timeline follow
fn arrived(n: &mut Net, ui: &mut Ui, from: Ns, tween: bool) {
    let to = n.doc.sim.view_time();
    let now = plat::task::now_us();
    n.sim.tween = if tween && !ui.anim_off() && from != to { Some(anim::Tween::new(n, from, to, now)) } else { None };
    anim::drops_at(n, to, now);
    events::follow(n, to);
    ui.repass();
}

// ---------------------------------------------------------------- selection

/// a line of the event list was chosen (or the view moved there): the event, its frame
pub(crate) fn select_event(n: &mut Net, seq: Option<u64>) {
    n.sim.sel_ev = seq;
    n.sim.sel_hop = None;
    n.sim.ev.grid.select_only(seq);
}

/// a frame on the canvas was clicked: it and the event about it; the packet tab opens
pub(crate) fn select_frame(n: &mut Net, hop: HopId) {
    let eng = n.doc.sim.engine();
    // the first decision about it (its arrival, or what its sender decided)
    let seq = eng.trace.events_of(hop).next().map(|e| e.seq);
    n.sim.sel_hop = Some(hop);
    n.sim.sel_ev = seq;
    n.sim.ev.grid.select_only(seq);
    if let Some(s) = seq {
        n.sim.ev.reveal(s);
    }
    n.shell.dock.focus(P_PACKET);
}

// ---------------------------------------------------------------- commands

fn flag(on: bool, f: u32) -> u32 {
    if on { f } else { 0 }
}

/// enabled / checked of the simulation commands (every frame)
pub(crate) fn states(n: &Net, ui: &mut Ui) {
    let s = &n.sim;
    let past = n.doc.sim.view_at().is_some();
    let recorded = n.doc.sim.engine().trace.hops().len() > 0;
    ui.cmd_state(CMD_SIM_REALTIME, flag(!s.sim_mode, CF_CHECKED));
    ui.cmd_state(CMD_SIM_MODE, flag(s.sim_mode, CF_CHECKED));
    ui.cmd_state(CMD_SIM_PLAY, flag(s.playing, CF_CHECKED));
    ui.cmd_state(CMD_SIM_STEP, 0);
    ui.cmd_state(CMD_SIM_BACK, flag(!recorded, CF_DISABLED));
    ui.cmd_state(CMD_SIM_LIVE, flag(!past, CF_DISABLED));
    ui.cmd_state(CMD_SIM_RESET, 0);
    for k in 0..SPEEDS.len() {
        ui.cmd_state(CMD_SIM_TEMPO0 + k as u32, flag(s.tempo == k, CF_CHECKED));
    }
    ui.cmd_state(CMD_SIM_ONLY_SEL, flag(s.only_sel, CF_CHECKED));
    ui.cmd_state(CMD_TOOL_PDU, flag(n.canvas.tool == Tool::Pdu, CF_CHECKED));
    ui.cmd_state(CMD_PDU_COMPLEX, 0);
    ui.cmd_state(CMD_EXPORT_PCAP, flag(!recorded, CF_DISABLED));
    let shown = n.doc.sim.past().is_some() && n.doc.sim.restoring().is_none();
    ui.cmd_state(CMD_SIM_RESUME, flag(!shown, CF_DISABLED));
}

/// the commands of the simulation mode -> handled
pub(crate) fn command(n: &mut Net, ui: &mut Ui, id: u32, arg: u32) -> bool {
    match id {
        CMD_SIM_REALTIME => set_mode(n, false),
        CMD_SIM_MODE => set_mode(n, true),
        CMD_SIM_PLAY => play(n, ui),
        CMD_SIM_STEP => {
            n.sim.stalled = false;
            n.sim.playing = false;
            step(n, ui);
        }
        CMD_SIM_BACK => {
            n.sim.playing = false;
            back(n, ui);
        }
        CMD_SIM_LIVE => live(n, ui),
        CMD_SIM_RESET => reset(n),
        CMD_SIM_SEEK => seek(n, ui, arg as Ns),
        CMD_SIM_ONLY_SEL => {
            n.sim.only_sel = !n.sim.only_sel;
            n.sim.sel_devs = crate::workspace::canvas::cables::sel_devices(n);
            // a selected group stands for its devices (G1)
            for g in crate::workspace::canvas::groups::selected_groups(n) {
                for d in n.canvas.scene.gs.devices(&n.doc.proj, g) {
                    if !n.sim.sel_devs.contains(&d) {
                        n.sim.sel_devs.push(d);
                    }
                }
            }
        }
        CMD_TOOL_PDU => {
            n.sim.pdu.src = None;
            let t = if n.canvas.tool == Tool::Pdu { Tool::Select } else { Tool::Pdu };
            tools::set(n, t);
        }
        CMD_PDU_COMPLEX => pdu::open_dialog(n),
        CMD_EXPORT_PCAP => pdu::export(n, ui),
        CMD_PCAP_DONE => pdu::export_done(n, ui),
        CMD_SIM_RESUME if n.doc.sim.past().is_some() => n.sim.ask.set(1),
        CMD_SIM_RESUME_GO => resume(n, ui),
        CMD_SIM_SEEK_MS => seek(n, ui, arg as Ns * 1_000_000),
        _ if (CMD_SIM_TEMPO0..CMD_SIM_TEMPO0 + SPEEDS.len() as u32).contains(&id) => {
            n.sim.tempo = (id - CMD_SIM_TEMPO0) as usize;
            if !n.sim.sim_mode && n.sim.playing {
                let speed_pm = speed(n);
                let _ = n.doc.sim.send(Cmd::Realtime { speed_pm });
            }
        }
        _ if (CMD_SIM_CAT0..CMD_SIM_CAT0 + CATS.len() as u32).contains(&id) => toggle_cat(n, (id - CMD_SIM_CAT0) as usize),
        _ => return false,
    }
    ui.repass();
    true
}

/// Simulation zurücksetzen: a fresh engine from the project (time 0, nothing learned, an empty
/// recording) in the mode it had
fn reset(n: &mut Net) {
    let sim = n.sim.sim_mode;
    n.doc.sim = crate::doc::sim::Sim::new(&n.doc.proj);
    loaded(n, true);
    if sim {
        set_mode(n, true);
    }
}

// ---------------------------------------------------------------- the animation timer

/// the animation timer, when something moves or plays (end of every frame)
pub(crate) fn timer(n: &mut Net, ui: &mut Ui) {
    if n.sim.armed {
        return;
    }
    let now = plat::task::now_us();
    let moving = n.sim.tween.is_some() || n.sim.drops.fading(now, ui.anim_off());
    let playing = n.sim.sim_mode && n.sim.playing && !n.sim.stalled;
    if moving || playing || n.sim.grow > 0 {
        ui.timer(TIMER_ANIM, 33);
        n.sim.armed = true;
    }
}

/// TIMER_ANIM: the animation moves on; playing takes its next step
pub(crate) fn tick(n: &mut Net, ui: &mut Ui) {
    n.sim.armed = false;
    let dock = &mut n.shell.dock;
    match n.sim.grow {
        3 => dock.show(P_PDUS),
        2 => dock.activate(P_EVENTS),
        1 if dock.side(P_EVENTS) == Some(DOCK_BOTTOM) && dock.size(DOCK_BOTTOM) < MIN_PANEL => dock.set_size(DOCK_BOTTOM, MIN_PANEL),
        _ => {}
    }
    n.sim.grow = n.sim.grow.saturating_sub(1);
    let now = plat::task::now_us();
    if n.sim.tween.as_ref().is_some_and(|t| t.done(now)) {
        n.sim.tween = None;
    }
    if n.sim.sim_mode && n.sim.playing && !n.sim.stalled && n.sim.tween.is_none() && now >= n.sim.last_step + period_us(n) {
        n.sim.last_step = now;
        if !step(n, ui) {
            n.sim.stalled = true;
        }
    }
    ui.repass();
}

/// something new to send while playing found nothing more: go on
pub(crate) fn kick(n: &mut Net) {
    n.sim.stalled = false;
}

// ---------------------------------------------------------------- toolbar, hint, status

/// the simulation part of the toolbar (first in the frame: the recorded decisions are read on
/// here, so the status bar counts them while the event list is hidden)
pub(crate) fn toolbar(n: &mut Net, ui: &mut Ui) {
    events::refresh(n, ui.theme_dark());
    let mut m = n.sim.sim_mode as i32;
    ui.next_flags(ST_COMPACT);
    if ui.segmented(ID_TB_MODE, &mut m, &[t("Echtzeit"), t("Simulation")]) & EV_ACT != 0 {
        ui.cmd_run(if m == 1 { CMD_SIM_MODE } else { CMD_SIM_REALTIME }, 0);
    }
    ui.tooltip(t("Echtzeit: die Uhr läuft mit · Simulation: Schritt für Schritt (F7)"));
    ui.spacer(4);
    let (icon, label) = if n.sim.playing { (IC_PAUSE, t("Anhalten (F6)")) } else { (IC_PLAY, t("Abspielen (F6)")) };
    // play, back, step: one Tab stop (the arrows move inside)
    ui.focus_group_begin(ID_TB_GRP_PLAY);
    ui.next_flags(ST_ICON_ONLY | ST_SUBTLE);
    if ui.icon_button(ID_TB_PLAY, icon, label) & EV_ACT != 0 {
        ui.cmd_run(CMD_SIM_PLAY, 0);
    }
    for c in [CMD_SIM_BACK, CMD_SIM_STEP] {
        ui.cmd_button(c, ST_ICON_ONLY | ST_SUBTLE);
    }
    ui.focus_group_end();
    let labels: Vec<&str> = SPEEDS.iter().map(|s| t(s.1)).collect();
    let mut tempo = n.sim.tempo as i32;
    ui.next_flags(ST_COMPACT);
    if ui.combobox(ID_TB_TEMPO, &mut tempo, &labels) & EV_ACT != 0 {
        ui.cmd_run(CMD_SIM_TEMPO0 + tempo.max(0) as u32, 0);
    }
    ui.tooltip(t("Tempo"));
    sep(ui);
    ui.focus_group_begin(ID_TB_GRP_PDU);
    for c in [CMD_TOOL_PDU, CMD_PDU_COMPLEX] {
        ui.cmd_button(c, ST_ICON_ONLY | ST_SUBTLE);
    }
    ui.focus_group_end();
}

fn sep(ui: &mut Ui) {
    ui.next_size(1, 20);
    ui.separator();
}

/// seconds to the nanosecond, the decimals in groups of three ("0,000 012 345"): a LAN frame
/// takes microseconds
pub(crate) fn secs(t: Ns) -> String {
    let f = t % 1_000_000_000;
    let dot = if crate::app::tr::en() { '.' } else { ',' };
    format!("{}{}{:03} {:03} {:03}", t / 1_000_000_000, dot, f / 1_000_000, f / 1000 % 1000, f % 1000)
}

/// above the canvas while the view stands in the past: the moment and "Zurück zu live"
pub(crate) fn hint(n: &mut Net, ui: &mut Ui) {
    if !n.sim.sim_mode {
        return;
    }
    let Some(at) = n.doc.sim.view_at() else { return };
    ui.box_begin(0, BOX_V | pad(8), 0);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_CARD | pad(6), 10);
    ui.icon(IC_HISTORY, 16);
    ui.text(TS_BODY_STRONG, &tf("Ansicht bei t = {} s", &[&secs(at)]));
    // the devices' state at that moment (K1): computed, shown, or older than the keyframes
    let what = match (n.doc.sim.restoring(), n.doc.sim.past_gone()) {
        (Some(pm), _) => tf("Zustand wird berechnet … {} %", &[&(pm / 10).to_string()]),
        (None, true) => String::from(t("Den Zustand der Geräte von damals gibt es nicht mehr.")),
        (None, false) => String::from(t("Zustand von damals")),
    };
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_SECONDARY | TX_ELLIPSIS, &what);
    ui.cmd_button(CMD_SIM_RESUME, 0);
    if ui.button(ID_SIM_LIVE, t("Zurück zu live")) & EV_ACT != 0 {
        ui.cmd_run(CMD_SIM_LIVE, 0);
    }
    ui.box_end();
    ui.box_end();
}

/// "Von hier fortsetzen?" (at the end of the frame)
pub(crate) fn resume_dialog(n: &mut Net, ui: &mut Ui) {
    let ask = n.sim.ask.clone();
    let at = n.doc.sim.past().unwrap_or(0);
    if ui.dialog_begin(ID_SIM_RESUME_ASK, t("Von hier fortsetzen?"), &ask) {
        ui.next_size(420, 0);
        ui.text(TS_BODY | TX_WRAP, &tf("Die Geräte machen bei t = {} s weiter. Was danach aufgezeichnet wurde und was danach von außen kam (Befehle in den Konsolen, PDUs, Fehler), geht verloren.", &[&secs(at)]));
        ui.next_size(420, 0);
        ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, t("Das Projekt und die Liste zum Rückgängigmachen bleiben, wie sie sind: spätere Änderungen am Projekt gelten ab diesem Zeitpunkt."));
        if ui.dialog_buttons(Some(t("Fortsetzen")), None, Some(t("Abbrechen"))) == DLG_PRIMARY {
            ui.cmd_run(CMD_SIM_RESUME_GO, 0);
        }
    }
    ui.dialog_end();
}

// "Von hier fortsetzen" after its question: the state shown becomes the present
// (doc/sim.rs Sim::resume); PDUs started later never ran, the event list and the packet tab
// start afresh from the recording that is left, consoles and the log say where it goes on
fn resume(n: &mut Net, ui: &mut Ui) {
    let Some(at) = n.doc.sim.past() else { return };
    if !n.doc.sim.resume(&n.doc.proj) {
        return;
    }
    n.sim.pdu.resumed(at);
    n.sim.tween = None;
    n.sim.drops = anim::Drops::default();
    n.sim.sel_hop = None;
    n.sim.sel_ev = None;
    n.sim.held = None;
    n.sim.playing = false;
    n.sim.stalled = false;
    n.sim.ev.reset();
    n.sim.pk.reset();
    crate::doc::console::resumed(n, at);
    n.log.drop_after(at);
    events::follow(n, at);
    ui.repass();
}

/// the mode in words (the status bar, the inspector's project), in the current language
pub(crate) fn mode_text(n: &Net) -> &'static str {
    t(match (n.sim.sim_mode, n.doc.sim.mode()) {
        (true, _) if n.sim.playing && !n.sim.stalled => "Simulationsmodus, spielt ab",
        (true, _) => "Simulationsmodus",
        (false, Mode::Realtime { .. }) => "läuft in Echtzeit",
        (false, _) => "Echtzeit, angehalten",
    })
}

/// the status bar: mode, clock, events
pub(crate) fn status(n: &Net, ui: &mut Ui) {
    let sim = &n.doc.sim;
    let mode = t(match (n.sim.sim_mode, sim.mode()) {
        (true, _) if n.sim.playing && !n.sim.stalled => "Simulation läuft",
        (true, _) => "Simulation",
        (false, Mode::Realtime { .. }) => "Echtzeit",
        (false, _) => "Echtzeit angehalten",
    });
    ui.status_dot(if n.sim.sim_mode { BADGE_ACCENT } else if n.sim.playing { BADGE_OK } else { BADGE_NEUTRAL });
    ui.text(TS_CAPTION, mode);
    if !n.sim.sim_mode {
        return;
    }
    sep(ui);
    let at = sim.view_time();
    let clock = if sim.view_at().is_some() { tf("t = {} s (Aufzeichnung)", &[&secs(at)]) } else { format!("t = {} s", secs(at)) };
    ui.text(TS_CAPTION | TX_SECONDARY, &clock);
    sep(ui);
    let k = n.sim.ev.shown();
    ui.text(TS_CAPTION | TX_SECONDARY, &if k == 1 { String::from(t("1 Ereignis")) } else { tf("{} Ereignisse", &[&k.to_string()]) });
}

/// the hint of the PDU tool in the status bar
pub(crate) fn pdu_hint(n: &Net, ui: &mut Ui) {
    if n.canvas.tool != Tool::Pdu {
        return;
    }
    let text = match n.sim.pdu.src.and_then(|d| n.doc.proj.device(d)) {
        Some(d) => tf("Von {}: jetzt das Ziel anklicken, Esc beendet", &[&d.name]),
        None => String::from(t("Erst die Quelle, dann das Ziel anklicken (oder mit Tab und Eingabe wählen), Esc beendet")),
    };
    ui.text(TS_CAPTION | TX_TERTIARY, &text);
}
