// sysmon -- the Systemmonitor of a host's desktop (desktop.rs): the device's processes, its
// services, its CPU load and memory
//
//   Mon::new()                      the page Prozesse, nothing chosen
//   program(n, ui, k, id, dev)      the window's content: the pages Prozesse, Dienste, Leistung
//   cpu_percent(ns_per_sec)         a second's CPU time as a share of the device's CPUs (‰)
//
// Prozesse: what ps shows (Engine::processes, jfn_sim/procs.rs) -- PID, Benutzer, TTY,
// CPU-Zeit, Befehl; Beenden types "kill PID" on the device's console, as the desktop's other
// programs type their commands (a recording replays it), and only for the background jobs the
// console's kill ends (services have their own switches: the Dienste tab).  Dienste: the device's services
// (Engine::services) with their kind, state and, while they run, their CPU time.  Leistung:
// the CPU load of the last minute (Engine::cpu_secs: the busy time of scripts, jfs processes and
// web servers per second, out of the CPUs top counts) and the memory of the scripts' heaps (the
// series heap_kb, series.rs: one sample a second) as small charts with the value of now.  In
// the past (K1) all of it shows the moment the view stands at.
//
// Ids (id = the desktop block + 0x400): + 1 the pages (+ 2 .. + 4 their segments), + 5 Beenden,
// + 0x10 / + 0x11 the charts; + 0x100 the processes (rows + 0x101 + i), + 0x180 the services
// (rows + 0x181 + i).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_sim::{ProcKind, ProcRow, SEC};
use ui::*;

use crate::app::tr::{self, t, tf};
use crate::app::Net;
use crate::doc::console;

/// the seconds the charts cover
const SPAN: u64 = 60;

/// the Systemmonitor's state in a device's desktop
pub(crate) struct Mon {
    /// 0 Prozesse, 1 Dienste, 2 Leistung
    pub page: i32,
    pub procs: GridState,
    pub svcs: GridState,
}

impl Mon {
    pub(crate) fn new() -> Mon {
        Mon { page: 0, procs: GridState::default(), svcs: GridState::default() }
    }
}

/// "m:ss.mmm" of a CPU time
fn cpu_text(ns: u64) -> String {
    let ms = ns / 1_000_000;
    format!("{}:{:02}.{:03}", ms / 60_000, (ms / 1000) % 60, ms % 1000)
}

/// a second's CPU time (ns) as thousandths of the device's CPUs (top's %Cpu(s))
pub(crate) fn cpu_percent(ns: u64) -> u64 {
    cpu_share(ns, jfn_sim::cli::tools::CPUS)
}

// the same on `cpus` CPUs (a device with hardware has its cores)
fn cpu_share(ns: u64, cpus: u64) -> u64 {
    (ns as u128 * 1000 / (SEC as u128 * cpus.max(1) as u128)) as u64
}

/// "12,5 %" of thousandths
fn permille(v: u64) -> String {
    format!("{},{} %", v / 10, v % 10)
}

// the memory of the device's scripts per sample of the last SPAN seconds (KiB), oldest first
fn heap_kb(n: &Net, dev: u32) -> Vec<(u64, i64)> {
    let e = n.doc.sim.state();
    let now = e.now();
    let from = now.saturating_sub(SPAN * SEC);
    let mut sum: BTreeMap<u64, i64> = BTreeMap::new();
    for s in e.series_services().iter().filter(|s| s.dev == dev) {
        if let Some(v) = e.series_one(dev, &s.svc, "heap_kb") {
            for (t, x) in v.points.into_iter().filter(|p| p.0 >= from) {
                *sum.entry(t).or_default() += x;
            }
        }
    }
    sum.into_iter().collect()
}

// i18n
const PAGES: [&str; 3] = ["Prozesse", "Dienste", "Leistung"];

/// the Systemmonitor of device window k
pub(crate) fn program(n: &mut Net, ui: &mut Ui, k: usize, id: u32, dev: u32) {
    let procs: Vec<ProcRow> = n.doc.sim.state().processes(dev);
    let svcs = n.doc.sim.state().services(dev);
    let on = n.doc.proj.device(dev).is_some_and(|d| !d.off);
    let Some(w) = n.devwin.get(k) else { return };
    let m = &mut w.desk.mon;
    let names: Vec<&str> = PAGES.iter().map(|p| t(p)).collect();
    if ui.segmented(id + 1, &mut m.page, &names) & EV_ACT != 0 {
        ui.repass();
    }
    if !on {
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Das Gerät ist ausgeschaltet: es läuft nichts."));
    }
    let mut kill: Option<u32> = None;
    match m.page {
        0 => {
            let keys: Vec<u64> = procs.iter().map(|p| p.pid as u64).collect();
            m.procs.retain(&keys);
            let cur = m.procs.cur.and_then(|c| procs.iter().find(|p| p.pid as u64 == c)).cloned();
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            ui.next_size(-1, 0);
            ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &tf("Prozesse: {}", &[&procs.len().to_string()]));
            let can = cur.as_ref().is_some_and(|p| p.killable);
            ui.next_icon(IC_STOP);
            ui.next_flags(if can { 0 } else { ST_DISABLED });
            if ui.button(id + 5, t("Beenden")) & EV_ACT != 0
                && let Some(p) = &cur
            {
                kill = Some(p.pid);
            }
            ui.tooltip(t("Beendet den gewählten Hintergrund-Job mit kill im Terminal; Dienste schaltet der Reiter Dienste."));
            ui.box_end();
            let cols = [
                GridCol::new(t("PID"), 56).flags(GC_NOSORT | GC_RIGHT),
                GridCol::new(t("Benutzer"), 70).flags(GC_NOSORT),
                GridCol::new(t("TTY"), 64).flags(GC_NOSORT),
                GridCol::new(t("CPU-Zeit"), 86).flags(GC_NOSORT | GC_RIGHT),
                GridCol::new(t("Befehl"), 200).flags(GC_NOSORT | GC_FILL),
            ];
            ui.next_size(-1, -1);
            let (first, end) = ui.grid_begin(id + 0x100, &mut m.procs, &cols, &keys);
            for i in first..end {
                let p = &procs[i as usize];
                ui.grid_row(&mut m.procs, i);
                ui.grid_text(&m.procs, 0, &p.pid.to_string());
                ui.grid_text(&m.procs, 1, &p.user);
                ui.grid_text(&m.procs, 2, &p.tty);
                ui.grid_text(&m.procs, 3, &cpu_text(p.cpu_ns));
                let what = match p.kind {
                    ProcKind::Service if !p.killable => format!("{} ({})", p.cmd, t("Dienst")),
                    _ => p.cmd.clone(),
                };
                ui.grid_text(&m.procs, 4, &what);
            }
            ui.grid_end(&mut m.procs);
        }
        1 => {
            let keys: Vec<u64> = (0..svcs.len() as u64).collect();
            m.svcs.retain(&keys);
            if svcs.is_empty() {
                ui.next_size(-1, 120);
                ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER, 0);
                ui.empty_state(IC_INFO, t("Keine Dienste"), t("Auf diesem Gerät läuft kein Dienst."));
                ui.box_end();
            } else {
                let cols = [
                    GridCol::new(t("Dienst"), 120).flags(GC_NOSORT | GC_FILL),
                    GridCol::new(t("Art"), 130).flags(GC_NOSORT),
                    GridCol::new(t("Zustand"), 90).flags(GC_NOSORT),
                    GridCol::new(t("CPU-Zeit"), 86).flags(GC_NOSORT | GC_RIGHT),
                ];
                ui.next_size(-1, -1);
                let (first, end) = ui.grid_begin(id + 0x180, &mut m.svcs, &cols, &keys);
                for i in first..end {
                    let s = &svcs[i as usize];
                    ui.grid_row(&mut m.svcs, i);
                    ui.grid_text(&m.svcs, 0, &s.name);
                    ui.grid_text(&m.svcs, 1, super::services::kind_name(s.kind));
                    ui.grid_cell(&m.svcs, 2);
                    ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
                    let (badge, state) = match (s.enabled, s.running) {
                        (_, true) => (BADGE_OK, t("läuft")),
                        (true, false) => (BADGE_WARN, t("gestoppt")),
                        (false, false) => (BADGE_NEUTRAL, t("aus")),
                    };
                    ui.status_dot(badge);
                    ui.text(TS_BODY | TX_ELLIPSIS, state);
                    ui.box_end();
                    let cpu = procs.iter().find(|p| p.kind == ProcKind::Service && p.svc.as_deref() == Some(s.name.as_str())).map_or_else(|| String::from("–"), |p| cpu_text(p.cpu_ns));
                    ui.grid_text(&m.svcs, 3, &cpu);
                }
                ui.grid_end(&mut m.svcs);
                if let Some(s) = m.svcs.cur.and_then(|c| svcs.get(c as usize)) {
                    ui.next_size(-1, 0);
                    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &format!("{}: {}", s.name, tr::msg(&s.status_msg)));
                }
            }
        }
        _ => {
            let cpu = n.doc.sim.state().cpu_secs(dev, SPAN);
            let heap = heap_kb(n, dev);
            let cpus = jfn_sim::cli::tools::cpus_of(n.doc.sim.state(), dev);
            let cv: Vec<f32> = cpu.iter().map(|&x| cpu_share(x, cpus) as f32 / 10.0).collect();
            let hv: Vec<f32> = heap.iter().map(|x| x.1 as f32).collect();
            let now_cpu = cpu.last().map_or(0, |&x| cpu_share(x, cpus));
            let now_heap = heap.last().map_or(0, |x| x.1);
            let card = |ui: &mut Ui, title: &str, value: &str, note: &str, cid: u32, vals: &[f32]| {
                ui.next_size(-1, 0);
                ui.box_begin(0, BOX_V | BOX_CARD | pad(10), 4);
                ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
                ui.text(TS_BODY_STRONG, title);
                ui.spacer(-1);
                ui.text(TS_BODY, value);
                ui.box_end();
                ui.next_size(-1, 48);
                if vals.is_empty() {
                    ui.box_begin(0, BOX_V | BOX_CENTER, 0);
                    ui.text(TS_CAPTION | TX_TERTIARY, t("Noch keine Messwerte"));
                    ui.box_end();
                } else {
                    ui.sparkline(cid, vals, 0);
                }
                ui.next_size(-1, 0);
                ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, note);
                ui.box_end();
            };
            card(ui, t("CPU-Last"), &permille(now_cpu), &tf("Die letzte Minute; {} CPUs wie bei top.", &[&cpus.to_string()]), id + 0x10, &cv);
            card(ui, t("Arbeitsspeicher der Skripte"), &tf("{} KiB", &[&now_heap.to_string()]), t("Die Heaps der Skript-Dienste, eine Messung je Sekunde."), id + 0x11, &hv);
        }
    }
    if let Some(pid) = kill {
        console::typed(n, dev, &format!("kill {}", pid));
        ui.repass();
    }
}
