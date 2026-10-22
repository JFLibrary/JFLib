// faults -- fault injection in the inspector (the lab): for a device (power off, a service
// stopped, a slower disk), for a cable (pulled; more delay, loss, a lower rate), for a
// selection (a partition of its devices against the rest of the network), each with an
// optional duration; the active faults with "Wiederherstellen", and "Alle wiederherstellen"
//
//   Bufs                          the fields: duration (s), the disk's extra delay (ms) and
//                                 factor, the cable's delay (ms), loss (%) and rate (Mbit/s),
//                                 the service chosen, why the last injection was refused
//   device(n, ui, id)             the card of a device
//   cable(n, ui, id)              the card of a cable
//   several(n, ui, devs, links)   the card of a selection: the partition, their faults
//   project(n, ui)                every active fault (nothing selected)
//   concerns(fault, devs, links)  a fault is about these devices or cables
//
// Every injection and every heal goes through the document (Doc::inject, heal, heal_all in
// doc/faults.rs): jfn_sim's fault actions (Action::Fault, Heal, HealAll), one undo step each --
// undo heals an injected fault, redo injects it again, an undone heal brings the faults back
// with the time they had left -- and a mark on the simulation's timeline (Engine::faults: the
// Metriken panel shades it in its charts, its start and end are notes in the Ereignisse
// panel).  The project stays as it is: a fault is an experiment on the running network, not
// part of the file.  Durations are seconds with a decimal comma or point, empty: until it is
// healed; the other fields take the same numbers ("2,5"), an empty one leaves its part out.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::Msg;
use jfn_sim::{Fault, FaultMark, Ns};
use ui::*;

use super::cable::parse_decimal;
use super::field_begin;
use crate::app::constants::*;
use crate::app::tr::{en, t, tc, tf};
use crate::app::Net;

/// the longest duration (a day, as jfn_sim allows)
const DUR_MAX: u64 = jfn_sim::fault::MAX_DUR;
/// heal buttons at most (their ids)
const ROWS_MAX: usize = 64;

/// the fields of the fault cards
#[derive(Default)]
pub(crate) struct Bufs {
    dur: String,
    disk_ms: String,
    disk_x: String,
    delay: String,
    loss: String,
    rate: String,
    /// the service chosen for "Stoppen": (device, index)
    svc: (u32, i32),
    /// why the last injection was refused, and what it was for (device or cable)
    err: String,
    err_for: u32,
}

/// the fault is about one of these devices or cables (a partition: a device named on a side)
pub(crate) fn concerns(f: &Fault, devs: &[u32], links: &[u32]) -> bool {
    match f {
        Fault::Power { dev } | Fault::Stop { dev, .. } | Fault::SlowDisk { dev, .. } => devs.contains(dev),
        Fault::Unplug { link } | Fault::Impair { link, .. } => links.contains(link),
        Fault::Partition { a, b } => a.iter().chain(b.iter()).any(|d| devs.contains(d)),
    }
}

/// "4,5 s" / "4.5 s" (tenths)
fn secs1(ns: Ns) -> String {
    let ds = ns.div_ceil(100_000_000);
    format!("{}{}{} s", ds / 10, if en() { '.' } else { ',' }, ds % 10)
}

// the duration field: Some(None) none, Some(Some(ns)) one, None: not a number (the error set)
fn duration(n: &mut Net, of: u32) -> Option<Option<Ns>> {
    let b = &mut n.inspector.flt;
    let s = b.dur.trim();
    if s.is_empty() {
        return Some(None);
    }
    match parse_decimal(s, 1_000_000_000, DUR_MAX).filter(|&v| v > 0) {
        Some(v) => Some(Some(v)),
        None => {
            b.err = String::from(t("Die Dauer ist eine Zahl von Sekunden (größer als 0, höchstens ein Tag), z. B. 10 oder 2,5; leer: bis zum Wiederherstellen."));
            b.err_for = of;
            None
        }
    }
}

// the head of a card and the duration field
fn head(n: &mut Net, ui: &mut Ui, title: &str) {
    ui.text(TS_BODY_STRONG, title);
    field_begin(ui, t("Dauer"));
    let b = &mut n.inspector.flt;
    ui.next_size(-1, 0);
    ui.textbox(ID_FLT_DUR, Some(t("ohne Ende")), &mut b.dur, 16);
    ui.text(TS_BODY | TX_SECONDARY, "s");
    ui.box_end();
}

// a fault starts: one undo step; a refusal stays below the card
fn inject(n: &mut Net, ui: &mut Ui, of: u32, f: Fault) {
    let Some(dur) = duration(n, of) else {
        ui.repass();
        return;
    };
    match n.doc.inject(f, dur) {
        Ok(_) => n.inspector.flt.err.clear(),
        Err(m) => {
            n.inspector.flt.err = crate::app::tr::msg(&m);
            n.inspector.flt.err_for = of;
        }
    }
    ui.repass();
}

// the refusal of the last injection (for this object)
fn error(n: &Net, ui: &mut Ui, of: u32) {
    let b = &n.inspector.flt;
    if !b.err.is_empty() && b.err_for == of {
        let e = b.err.clone();
        crate::workspace::error_text(ui, &e, 0);
    }
}

fn refused(n: &mut Net, of: u32, text: &'static str) {
    n.inspector.flt.err = String::from(t(text));
    n.inspector.flt.err_for = of;
}

/// the card of a device: power off, a service stopped, a slower disk; its faults
pub(crate) fn device(n: &mut Net, ui: &mut Ui, id: u32) {
    let Some(d) = n.doc.proj.device(id).cloned() else { return };
    let links: Vec<u32> = n.doc.proj.links.iter().filter(|l| l.a.dev == id || l.b.dev == id).map(|l| l.id).collect();
    let eng = n.doc.sim.engine();
    // the device's services: name, label, running
    let svcs: Vec<(String, String, bool)> = eng
        .service_list()
        .into_iter()
        .filter(|s| s.dev == id)
        .map(|s| (s.name.clone(), tf("{} {}", &[crate::workspace::metrics::kind_name(s.kind), &s.name]), s.running))
        .collect();
    let disk = eng.disk_of(id).is_some();
    let powered = !d.off && n.doc.sim.view.devices.iter().find(|x| x.dev == id).is_none_or(|x| x.power);
    ui.box_begin(0, BOX_V | BOX_CARD | pad(10), 8);
    head(n, ui, t("Fehler einspielen"));
    // power
    ui.box_begin(0, BOX_H | BOX_END, 8);
    ui.next_flags(if powered { 0 } else { ST_DISABLED });
    if ui.button(ID_FLT_POWER, t("Ausschalten")) & EV_ACT != 0 {
        inject(n, ui, id, Fault::Power { dev: id });
    }
    ui.tooltip(t("Das Gerät verliert den Strom und kommt beim Wiederherstellen wieder (das Projekt bleibt eingeschaltet)."));
    ui.box_end();
    // a service
    if !svcs.is_empty() {
        let b = &mut n.inspector.flt;
        if b.svc.0 != id {
            b.svc = (id, 0);
        }
        let mut sel = b.svc.1.clamp(0, svcs.len() as i32 - 1);
        field_begin(ui, t("Dienst"));
        let names: Vec<&str> = svcs.iter().map(|s| s.1.as_str()).collect();
        ui.next_size(-1, 0);
        if ui.combobox(ID_FLT_SVC, &mut sel, &names) & EV_ACT != 0 {
            n.inspector.flt.svc = (id, sel);
            ui.repass();
        }
        ui.box_end();
        ui.box_begin(0, BOX_H | BOX_END, 8);
        let on = svcs.get(sel as usize).is_some_and(|s| s.2);
        ui.next_flags(if on && powered { 0 } else { ST_DISABLED });
        if ui.button(ID_FLT_STOP, t("Stoppen")) & EV_ACT != 0 {
            let name = svcs[sel as usize].0.clone();
            inject(n, ui, id, Fault::Stop { dev: id, svc: name });
        }
        ui.box_end();
    }
    // the disk
    if disk {
        let b = &mut n.inspector.flt;
        field_begin(ui, t("Disk +"));
        ui.next_size(-1, 0);
        ui.textbox(ID_FLT_DISK_MS, Some("10"), &mut b.disk_ms, 12);
        ui.text(TS_BODY | TX_SECONDARY, "ms");
        ui.box_end();
        field_begin(ui, t("Faktor \u{d7}"));
        ui.next_size(-1, 0);
        ui.textbox(ID_FLT_DISK_X, Some("1"), &mut b.disk_x, 12);
        ui.box_end();
        ui.box_begin(0, BOX_H | BOX_END, 8);
        let go = ui.button(ID_FLT_DISK, t("Verlangsamen")) & EV_ACT != 0;
        ui.box_end();
        if go {
            let b = &n.inspector.flt;
            let extra = if b.disk_ms.trim().is_empty() { Some(0) } else { parse_decimal(&b.disk_ms, 1_000_000, 60_000_000_000) };
            let factor = if b.disk_x.trim().is_empty() { Some(1000) } else { parse_decimal(&b.disk_x, 1000, 1_000_000_000).filter(|&v| v > 0) };
            match (extra, factor) {
                (Some(e), Some(x)) if e > 0 || x != 1000 => inject(n, ui, id, Fault::SlowDisk { dev: id, extra: e, factor_pm: x as u32 }),
                _ => {
                    refused(n, id, "Die Disk braucht mehr Zeit je Zugriff (ms) oder einen Faktor für die Übertragung (größer als 0, nicht 1), z. B. 10 und 3.");
                    ui.repass();
                }
            }
        }
    }
    error(n, ui, id);
    active(n, ui, &[id], &links);
    ui.box_end();
}

/// the card of a cable: pulled, worse (delay, loss, rate); its faults
pub(crate) fn cable(n: &mut Net, ui: &mut Ui, id: u32) {
    let Some(l) = n.doc.proj.link(id).cloned() else { return };
    ui.box_begin(0, BOX_V | BOX_CARD | pad(10), 8);
    head(n, ui, t("Fehler einspielen"));
    ui.box_begin(0, BOX_H | BOX_END, 8);
    ui.next_flags(if l.down { ST_DISABLED } else { 0 });
    if ui.button(ID_FLT_UNPLUG, t("Kabel ziehen")) & EV_ACT != 0 {
        inject(n, ui, id, Fault::Unplug { link: id });
    }
    ui.tooltip(t("Das Kabel ist getrennt, bis der Fehler behoben ist (im Projekt bleibt es eingesteckt)."));
    ui.box_end();
    let fields = [(ID_FLT_DELAY, t("Laufzeit +"), "ms", "20"), (ID_FLT_LOSS, t("Verlust +"), "%", "5"), (ID_FLT_RATE, t("Höchstens"), "Mbit/s", "10")];
    for (fid, key, unit, hint) in fields {
        let b = &mut n.inspector.flt;
        let buf = match fid {
            ID_FLT_DELAY => &mut b.delay,
            ID_FLT_LOSS => &mut b.loss,
            _ => &mut b.rate,
        };
        field_begin(ui, key);
        ui.next_size(-1, 0);
        ui.textbox(fid, Some(hint), buf, 16);
        ui.text(TS_BODY | TX_SECONDARY, unit);
        ui.box_end();
    }
    ui.box_begin(0, BOX_H | BOX_END, 8);
    let go = ui.button(ID_FLT_IMPAIR, t("Verschlechtern")) & EV_ACT != 0;
    ui.box_end();
    if go {
        let b = &n.inspector.flt;
        let opt = |s: &str, unit: u64, max: u64| if s.trim().is_empty() { Some(None) } else { parse_decimal(s, unit, max).map(Some) };
        let delay = opt(&b.delay, 1_000_000, 60_000_000_000);
        let loss = opt(&b.loss, 10_000, 1_000_000);
        let rate = opt(&b.rate, 1000, u32::MAX as u64).filter(|r| r.is_none_or(|v| v > 0));
        match (delay, loss, rate) {
            (Some(d), Some(x), Some(r)) if d.is_some_and(|v| v > 0) || x.is_some_and(|v| v > 0) || r.is_some() => {
                inject(n, ui, id, Fault::Impair { link: id, delay: d.unwrap_or(0), jitter: 0, loss_ppm: x.unwrap_or(0) as u32, kbps: r.map(|v| v as u32) })
            }
            _ => {
                refused(n, id, "Laufzeit (ms), Verlust (0 bis 100 %) oder eine Rate (Mbit/s, größer als 0) angeben, z. B. 20, 5 oder 10.");
                ui.repass();
            }
        }
    }
    error(n, ui, id);
    active(n, ui, &[], &[id]);
    ui.box_end();
}

/// the card of a selection: its devices in a partition against the rest; their faults
pub(crate) fn several(n: &mut Net, ui: &mut Ui, devs: &[u32], links: &[u32]) {
    ui.box_begin(0, BOX_V | BOX_CARD | pad(10), 8);
    if !devs.is_empty() {
        head(n, ui, t("Partition"));
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Die gewählten Geräte und der Rest des Netzes hören einander nicht mehr: jedes verwirft die IPv4-Pakete der anderen Seite."));
        ui.box_begin(0, BOX_H | BOX_END, 8);
        if ui.button(ID_FLT_PART, t("Abtrennen")) & EV_ACT != 0 {
            inject(n, ui, 0, Fault::Partition { a: devs.to_vec(), b: Vec::new() });
        }
        ui.box_end();
        error(n, ui, 0);
    }
    active(n, ui, devs, links);
    ui.box_end();
}

/// every active fault (the project's page), when there is one
pub(crate) fn project(n: &mut Net, ui: &mut Ui) {
    if n.doc.sim.engine().active_faults().next().is_none() {
        return;
    }
    ui.box_begin(0, BOX_V | BOX_CARD | pad(10), 8);
    active(n, ui, &[], &[]);
    ui.box_end();
}

// the active faults that concern the devices or cables (none given: all of them), each with
// "Wiederherstellen", then "Alle wiederherstellen" while any fault is active
fn active(n: &mut Net, ui: &mut Ui, devs: &[u32], links: &[u32]) {
    let eng = n.doc.sim.engine();
    let now = eng.now();
    let all = devs.is_empty() && links.is_empty();
    let rows: Vec<(u32, Msg, Option<Ns>)> = eng
        .active_faults()
        .filter(|m| all || concerns(&m.fault, devs, links))
        .map(|m: &FaultMark| (m.id, m.msg.clone(), m.until.map(|u| u.saturating_sub(now))))
        .collect();
    let total = eng.active_faults().count();
    if total == 0 {
        return;
    }
    let mut heal: Option<u32> = None;
    let mut heal_all = false;
    ui.text(TS_BODY_STRONG, &tf("Aktive Fehler ({})", &[&rows.len().to_string()]));
    // a fault: its line and how long it lasts, "Wiederherstellen" below them
    for (k, (id, msg, left)) in rows.iter().enumerate().take(ROWS_MAX) {
        ui.box_begin(0, BOX_H, 8);
        ui.status_dot(BADGE_CRIT);
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V, 0);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_WRAP, &tf("Fehler {}: {}", &[&id.to_string(), &crate::app::tr::msg(msg)]));
        let when = match left {
            Some(l) => tf("noch {}", &[&secs1(*l)]),
            None => String::from(t("bis zum Wiederherstellen")),
        };
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &when);
        ui.box_end();
        ui.box_end();
        ui.box_begin(0, BOX_H | BOX_END, 8);
        if ui.button(ID_FLT_HEAL0 + k as u32, tc("Wiederherstellen|Fehler")) & EV_ACT != 0 {
            heal = Some(*id);
        }
        ui.box_end();
    }
    ui.box_begin(0, BOX_H | BOX_END, 8);
    if ui.button(ID_FLT_HEAL_ALL, &tf("Alle wiederherstellen ({})", &[&total.to_string()])) & EV_ACT != 0 {
        heal_all = true;
    }
    ui.box_end();
    if let Some(id) = heal {
        let _ = n.doc.heal(&[id]);
        ui.repass();
    }
    if heal_all {
        let _ = n.doc.heal_all();
        ui.repass();
    }
}
