// cable -- the inspector of a cable: its ends, kind ("Automatisch" or a chosen one), speed,
// delay, loss, plug and route, its state and packets per second from the simulator
//
//   frame(n, ui, id)
//
// Speed in Mbit/s, delay in µs, loss in % (a decimal comma or point); Enter or "Setzen" applies
// them, Esc puts the fields back.  Every change is one undo step the simulator follows
// (SetCable, Impair, Unplug); a kind that does not plug into both ports is not offered.  Below
// them the lab's faults of the cable (faults.rs: pulled, worse, for a while), last its length
// and the limit of its medium (P1, workspace/phys/props.rs).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::project::Route;
use jfn_model::{cable, ifname, CableKind};
use ui::*;

use super::{field_begin, row, rows_begin};
use crate::app::constants::*;
use crate::app::tr::{en, t, tf};
use crate::app::Net;
use crate::doc::edits;
use crate::workspace::canvas::cables;

/// v / unit with up to three decimals and the decimal mark of the language ("2,5", "2.5")
fn decimal(v: u64, unit: u64) -> String {
    let (whole, frac) = (v / unit, v % unit * 1000 / unit);
    if frac == 0 {
        return format!("{}", whole);
    }
    let f = format!("{:03}", frac);
    format!("{}{}{}", whole, if en() { '.' } else { ',' }, f.trim_end_matches('0'))
}

/// "2,5" / "2.5" -> 2500 for unit 1000 (None: no number, negative, too big); also the fields
/// of the fault cards (faults.rs)
pub(super) fn parse_decimal(s: &str, unit: u64, max: u64) -> Option<u64> {
    let t = s.trim().replace(',', ".");
    let (a, b) = t.split_once('.').unwrap_or((&t, ""));
    if a.is_empty() && b.is_empty() || !a.bytes().all(|c| c.is_ascii_digit()) || !b.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let whole: u64 = if a.is_empty() { 0 } else { a.parse().ok()? };
    let mut frac = 0u64;
    let mut scale = unit;
    for c in b.bytes() {
        scale /= 10;
        if scale == 0 {
            break;
        }
        frac += (c - b'0') as u64 * scale;
    }
    let v = whole.checked_mul(unit)?.checked_add(frac)?;
    if v > max { None } else { Some(v) }
}

pub(crate) fn frame(n: &mut Net, ui: &mut Ui, id: u32) {
    let p = &n.doc.proj;
    let Some(l) = p.link(id).cloned() else { return };
    let (pa, pb) = (p.resolve_port(&l.a).ok(), p.resolve_port(&l.b).ok());
    ui.text(TS_SUBTITLE, t("Kabel"));
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &cables::ends_text(p, &l));
    let leds = n.doc.sim.led(id);
    let state = cables::state_text(l.down, leds);
    rows_begin(ui);
    let end = |dev: u32, port: &str, spec: &Option<jfn_model::PortSpec>| {
        let name = p.device(dev).map_or("?", |d| d.name.as_str());
        match spec {
            Some(s) if matches!(s.media, jfn_model::Media::Console | jfn_model::Media::Usb) => format!("{} {} ({})", name, ifname::short(port), t(s.media.name())),
            Some(s) => format!("{} {} ({} {})", name, ifname::short(port), crate::app::tr::speed(s.kbps), t(s.media.name())),
            None => format!("{} {}", name, ifname::short(port)),
        }
    };
    row(ui, t("Ende A"), &end(l.a.dev, &l.a.port, &pa));
    row(ui, t("Ende B"), &end(l.b.dev, &l.b.port, &pb));
    row(ui, t("Zustand"), &tf("{} · {} Pakete/s", &[state, &n.doc.sim.rate(id).to_string()]));
    if let Some(e) = n.doc.sim.errors.get(&id) {
        row(ui, t("Simulator"), &tf("läuft nicht: {}", &[&crate::app::tr::msg(e)]));
    }
    ui.box_end();
    // the kind: "Automatisch" (the right one for the ports) or one that plugs in
    let mut kinds: Vec<CableKind> = Vec::new();
    if let (Some(a), Some(b)) = (&pa, &pb) {
        kinds = CableKind::ALL.iter().copied().filter(|&k| cable::cable_fits(k, a, b).is_ok()).collect();
    }
    let right = match (&pa, &pb) {
        (Some(a), Some(b)) => cable::cable_kind(a, b).ok(),
        _ => None,
    };
    let mut items: Vec<String> = alloc::vec![match right {
        Some(k) => tf("Automatisch: {}", &[t(k.title())]),
        None => String::from(t("Automatisch")),
    }];
    items.extend(kinds.iter().map(|k| String::from(t(k.title()))));
    let refs: Vec<&str> = items.iter().map(|s| s.as_str()).collect();
    let mut sel = if l.cable_fixed { kinds.iter().position(|&k| k == l.cable).map_or(0, |i| i as i32 + 1) } else { 0 };
    ui.box_begin(0, BOX_V, 4);
    ui.text(TS_BODY | TX_SECONDARY, t("Kabeltyp"));
    ui.next_size(-1, 0);
    if ui.combobox(ID_INS_CABLE, &mut sel, &refs) & EV_ACT != 0 {
        let kind = if sel <= 0 { None } else { kinds.get(sel as usize - 1).copied() };
        cables::set_kind(n, kind);
        ui.repass();
    }
    ui.box_end();
    // speed, delay, loss (a console cable carries no frames: none)
    if l.cable != CableKind::Console {
        quality(n, ui, &l);
    }
    cable_tail(n, ui, &l);
    // faults of the lab: pulled, worse (faults.rs)
    if l.cable != CableKind::Console {
        super::faults::cable(n, ui, id);
    }
    // its length and the limit of its medium (the physical view, last: workspace/phys/props.rs)
    crate::workspace::phys::props::cable(n, ui, id);
}

fn quality(n: &mut Net, ui: &mut Ui, l: &jfn_model::Link) {
    let id = l.id;
    let ins = &mut n.inspector;
    let src = (id, l.kbps, l.delay_ns, l.loss_ppm);
    if ins.link_src != src {
        ins.kbps = decimal(l.kbps as u64, 1000);
        ins.delay = decimal(l.delay_ns, 1000);
        ins.loss = decimal(l.loss_ppm as u64, 10_000);
        if ins.link_src.0 != id {
            ins.link_err.clear();
        }
        ins.link_src = src;
    }
    let mut set = false;
    let fields = [
        (ID_INS_KBPS, t("Bandbreite"), "Mbit/s", "1000", decimal(l.kbps as u64, 1000)),
        (ID_INS_DELAY, t("Verzögerung"), "µs", if en() { "0.05" } else { "0,05" }, decimal(l.delay_ns, 1000)),
        (ID_INS_LOSS, t("Verlust"), "%", "0", decimal(l.loss_ppm as u64, 10_000)),
    ];
    for (fid, key, unit, hint, now) in fields {
        let ins = &mut n.inspector;
        let buf = match fid {
            ID_INS_KBPS => &mut ins.kbps,
            ID_INS_DELAY => &mut ins.delay,
            _ => &mut ins.loss,
        };
        field_begin(ui, key);
        // Esc puts the field back
        if ui.focused() == fid && ui.capture_shortcut(VK_ESCAPE, 0) {
            *buf = now;
            ins.link_err.clear();
        }
        ui.next_size(-1, 0);
        if ui.textbox(fid, Some(hint), buf, 24) & EV_COMMIT != 0 && ui.focused() == fid {
            set = true;
        }
        ui.next_size(48, 0);
        ui.text(TS_BODY | TX_SECONDARY, unit);
        ui.box_end();
    }
    ui.box_begin(0, BOX_H | BOX_END, 8);
    if ui.button(ID_INS_LINK_APPLY, t("Setzen")) & EV_ACT != 0 {
        set = true;
    }
    ui.box_end();
    if set {
        let ins = &n.inspector;
        let kbps = parse_decimal(&ins.kbps, 1000, u32::MAX as u64).filter(|&v| v > 0);
        let delay = parse_decimal(&ins.delay, 1000, 60_000_000_000);
        let loss = parse_decimal(&ins.loss, 10_000, 1_000_000);
        match (kbps, delay, loss) {
            (Some(k), Some(d), Some(x)) => {
                n.inspector.link_err.clear();
                if let Some(e) = edits::link_edit(&n.doc.proj, id, "Kabel einstellen", |l| {
                    l.kbps = k as u32;
                    l.delay_ns = d;
                    l.loss_ppm = x as u32;
                }) {
                    n.doc.apply(e);
                }
            }
            _ => {
                n.inspector.link_err = String::from(t("Bandbreite (mehr als 0 Mbit/s), Verzögerung (µs) und Verlust (0 bis 100 %) als Zahlen, z. B. 2,5."));
            }
        }
        ui.repass();
    }
    if !n.inspector.link_err.is_empty() {
        let err = n.inspector.link_err.clone();
        crate::workspace::error_text(ui, &err, 0);
    }
}

// plugged in, route, waypoints, Trennen
fn cable_tail(n: &mut Net, ui: &mut Ui, l: &jfn_model::Link) {
    let id = l.id;
    let mut plugged = (!l.down) as i32;
    if ui.switch(ID_INS_PLUG, t("Eingesteckt"), &mut plugged) & EV_ACT != 0 {
        let label = if plugged != 0 { "Kabel einstecken" } else { "Kabel ausstecken" };
        if let Some(e) = edits::link_edit(&n.doc.proj, id, label, |l| l.down = plugged == 0) {
            n.doc.apply(e);
        }
        ui.repass();
    }
    let mut route = match l.route {
        Route::Straight => 0,
        Route::Ortho => 1,
        Route::Curve => 2,
    };
    ui.box_begin(0, BOX_V, 4);
    ui.text(TS_BODY | TX_SECONDARY, t("Verlauf"));
    ui.next_size(-1, 0);
    ui.next_flags(ST_COMPACT);
    if ui.segmented(ID_INS_ROUTE, &mut route, &[t("Gerade"), t("Rechtwinklig"), t("Kurve")]) & EV_ACT != 0 {
        cables::set_route(n, [Route::Straight, Route::Ortho, Route::Curve][route.clamp(0, 2) as usize]);
        ui.repass();
    }
    ui.box_end();
    if !l.waypoints.is_empty() {
        let at: Vec<String> = l.waypoints.iter().map(|w| format!("{}, {}", w.x, w.y)).collect();
        row(ui, t("Wegpunkte"), &at.join("; "));
    }
    ui.box_begin(0, BOX_H | BOX_END, 8);
    if ui.button(ID_INS_DISCONNECT, t("Trennen")) & EV_ACT != 0 {
        cables::disconnect_selection(n);
        ui.repass();
    }
    ui.box_end();
    ui.next_size(-1, 0);
    ui.text(
        TS_CAPTION | TX_SECONDARY | TX_WRAP,
        t("Wegpunkte: an den „+“-Griffen des gewählten Kabels ziehen, Doppelklick entfernt einen. Rechtsklick: Verlauf, Kabeltyp, Trennen."),
    );
}
