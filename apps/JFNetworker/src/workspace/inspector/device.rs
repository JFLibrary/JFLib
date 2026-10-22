// device -- the inspector of a device: symbol, name, power, its console and device window, model
// and ports, the interfaces from the simulator with the quick settings of the chosen one, the
// cables
//
//   frame(n, ui, id)
//
// The interfaces (a grid, wider than the panel: it scrolls sideways) are the device's ports and
// its other IPv4 interfaces (VLAN interfaces, subinterfaces, loopbacks) as the simulator runs
// them: status with the colour of the port's LED (aktiv, unten, abgeschaltet, frei, ohne Strom),
// IPv4 address (more: "+1"), the far end, speed, VLAN (access VLAN, "Trunk (1)" with the native
// one), MAC.  The chosen interface gets quick fields: address with prefix, DHCP, up; a host
// also its default gateway.  They apply at once and land in the startup configuration
// (doc/config.rs), one undo step each; a refusal shows below them.  Last the lab's faults of
// the device (faults.rs: power off, a service stopped, a slower disk, for a while).
//
// A device with IPv6 gets a column IPv6 after IP (the first global address, else the
// link-local one; "+1" for more) and the chosen interface lists all its IPv6 addresses with
// how they came and their state; without IPv6 nothing changes.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::catalog::{self, CliStyle, F_HOST};
use jfn_model::ifname;
use jfn_sim::{Addr6View, AddrState, IfKind, IfView, Led, Origin6, PortMode, PortView};
use ui::*;

use super::{row, rows_begin, Inspector};
use crate::app::constants::*;
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::{config, edits};
use crate::workspace::canvas::cables;
use crate::workspace::icons::port_icon;

/// the columns of the interface grid (DIP), in the current language; its width
/// (the most important first: the panel shows the start, the rest scrolls in)
fn cols() -> [GridCol<'static>; 7] {
    [
        GridCol::new(t("Port"), 80).flags(GC_NOSORT),
        GridCol::new(t("Status"), 92).flags(GC_NOSORT),
        GridCol::new(t("IP"), 116).flags(GC_NOSORT),
        GridCol::new(t("Gegenstelle"), 128).flags(GC_NOSORT),
        GridCol::new(t("Speed"), 56).flags(GC_NOSORT),
        GridCol::new(t("VLAN"), 72).flags(GC_NOSORT),
        GridCol::new(t("MAC"), 124).flags(GC_NOSORT),
    ]
}
const GRID_W: i32 = 668;
/// the column of IPv6 addresses (only devices with IPv6 get it)
const IP6_W: i32 = 200;

fn col6() -> GridCol<'static> {
    GridCol::new(t("IPv6"), IP6_W).flags(GC_NOSORT)
}

/// the columns with IPv6 after IP when `v6`
fn cols_for(v6: bool) -> Vec<GridCol<'static>> {
    let mut v: Vec<GridCol<'static>> = cols().into_iter().collect();
    if v6 {
        v.insert(3, col6());
    }
    v
}
/// interface rows shown at most (the grid scrolls beyond)
const GRID_ROWS: i32 = 10;

/// a row of the interface grid (also the device window's, workspace/devwin)
pub(crate) struct IfRow {
    pub key: u64,
    /// the full name ("GigabitEthernet1/0/1", "Vlan10")
    pub name: String,
    pub media: Option<jfn_model::Media>,
    pub status: &'static str,
    pub badge: u32,
    pub speed: String,
    pub ip: String,
    pub mac: String,
    pub vlan: String,
    pub peer: String,
    /// its IPv4 interface
    pub iface: Option<IfView>,
    /// the first IPv6 address ("–" without one)
    pub ip6: String,
    /// not shut down
    pub admin: bool,
}

/// the key of a port's row, of another interface's row
fn port_key(i: u16) -> u64 {
    i as u64 + 1
}
fn iface_key(i: u16) -> u64 {
    0x1_0000 + i as u64
}

// the status of a port in words (the current language) and as a badge kind
fn port_status(power: bool, p: &PortView) -> (&'static str, u32) {
    let (s, b) = if !power {
        ("ohne Strom", BADGE_NEUTRAL)
    } else if !p.admin_up {
        ("abgeschaltet", BADGE_NEUTRAL)
    } else {
        match (p.link, p.led) {
            (None, _) => ("frei", BADGE_NEUTRAL),
            (Some(_), Led::Up) => ("aktiv", BADGE_OK),
            (Some(_), Led::Blocking | Led::Learning) => ("blockiert", BADGE_WARN),
            (Some(_), Led::NoPower) => ("ohne Strom", BADGE_NEUTRAL),
            (Some(_), Led::Down) => ("unten", BADGE_CRIT),
        }
    };
    (t(s), b)
}

fn addrs(f: &IfView) -> String {
    match f.addrs.len() {
        0 => String::from("–"),
        1 => f.addrs[0].to_string(),
        k => format!("{} +{}", f.addrs[0], k - 1),
    }
}

/// the first global IPv6 address (else the link-local one), "+N" for the others
fn addrs6(f: Option<&IfView>) -> String {
    let Some(f) = f.filter(|f| !f.addrs6.is_empty()) else {
        return String::from("–");
    };
    let first = f.addrs6.iter().find(|a| !a.addr.is_link_local()).or(f.addrs6.first()).map(|a| format!("{}/{}", a.addr, a.len)).unwrap_or_default();
    match f.addrs6.len() {
        1 => first,
        k => format!("{} +{}", first, k - 1),
    }
}

/// how an IPv6 address came and where it stands, in words
fn addr6_line(a: &Addr6View) -> String {
    // i18n
    let how = match a.origin {
        Origin6::LinkLocal => "Link-Local",
        Origin6::LinkLocalManual => "Link-Local, manuell",
        Origin6::Manual => "manuell",
        Origin6::Eui64 => "EUI-64",
        Origin6::Slaac => "SLAAC",
        Origin6::Loopback => "Loopback",
    };
    let how = t(how);
    let state = match a.state {
        AddrState::Tentative => Some(t("wird geprüft (DAD)")),
        AddrState::Duplicate => Some(t("doppelt (DAD)")),
        AddrState::Deprecated => Some(t("veraltet")),
        AddrState::Preferred => None,
    };
    match state {
        Some(s) => format!("{}/{} · {}, {}", a.addr, a.len, how, s),
        None => format!("{}/{} · {}", a.addr, a.len, how),
    }
}

/// the rows of a device's interfaces: its ports, then its other IPv4 interfaces
pub(crate) fn if_rows(n: &Net, id: u32) -> Vec<IfRow> {
    let p = &n.doc.proj;
    let view = &n.doc.sim.view;
    let Some(dv) = view.devices.iter().find(|d| d.dev == id) else { return Vec::new() };
    let bridge = n.doc.sim.engine().bridge(id);
    let ports = p.model_of(id).map(|m| m.ports()).unwrap_or_default();
    let mut out = Vec::new();
    for pv in &dv.ports {
        let iface = dv.ifaces.iter().find(|f| f.kind == IfKind::Port(pv.index)).cloned();
        let (status, badge) = port_status(dv.power, pv);
        let media = ports.get(pv.index as usize).map(|s| s.media);
        let vlan = match bridge.and_then(|b| b.ports.get(pv.index as usize)) {
            Some(bp) if bp.member && iface.is_none() => match &bp.mode {
                PortMode::Access(v) => format!("{}", v),
                PortMode::Trunk { native, .. } => format!("Trunk ({})", native),
            },
            _ => String::from("–"),
        };
        let peer = p
            .link_at(id, &pv.name)
            .map(|l| {
                let (od, op) = if l.a.dev == id && ifname::same(&l.a.port, &pv.name) { (l.b.dev, &l.b.port) } else { (l.a.dev, &l.a.port) };
                format!("{} {}", p.device(od).map_or("?", |d| d.name.as_str()), ifname::short(op))
            })
            .unwrap_or_else(|| String::from("–"));
        let console = media.is_some_and(|m| matches!(m, jfn_model::Media::Console | jfn_model::Media::Usb));
        out.push(IfRow {
            key: port_key(pv.index),
            name: pv.name.clone(),
            media,
            status,
            badge,
            speed: if console { String::from("–") } else { crate::app::tr::speed(pv.kbps) },
            ip: iface.as_ref().map_or_else(|| String::from("–"), addrs),
            mac: if console { String::from("–") } else { pv.mac.to_string() },
            vlan,
            peer,
            ip6: addrs6(iface.as_ref()),
            iface,
            admin: pv.admin_up,
        });
    }
    // VLAN interfaces, subinterfaces and loopbacks
    for f in dv.ifaces.iter().filter(|f| !matches!(f.kind, IfKind::Port(_))) {
        let (status, badge) = if !dv.power {
            (t("ohne Strom"), BADGE_NEUTRAL)
        } else if !f.admin_up {
            (t("abgeschaltet"), BADGE_NEUTRAL)
        } else if f.up {
            (t("aktiv"), BADGE_OK)
        } else {
            (t("unten"), BADGE_CRIT)
        };
        let vlan = match f.kind {
            IfKind::Sub { vlan, .. } | IfKind::Svi(vlan) => format!("{}", vlan),
            _ => String::from("–"),
        };
        out.push(IfRow {
            key: iface_key(f.index),
            name: f.name.clone(),
            media: None,
            status,
            badge,
            speed: String::from("–"),
            ip: addrs(f),
            mac: if f.mac.is_zero() { String::from("–") } else { f.mac.to_string() },
            vlan,
            peer: String::from("–"),
            ip6: addrs6(Some(f)),
            iface: Some(f.clone()),
            admin: f.admin_up,
        });
    }
    out
}

pub(crate) fn frame(n: &mut Net, ui: &mut Ui, id: u32) {
    let p = &n.doc.proj;
    let Some(d) = p.device(id).cloned() else { return };
    let m = catalog::model(&d.model);
    // the head: symbol, name, model
    ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
    if let Some(draw) = m.and_then(|m| catalog::catalog().iter().position(|c| c.id == m.id)).and_then(|i| n.palette.tile_draw(i)) {
        let r = ui.place(ui.dp(48), ui.dp(36));
        let ver = n.syms.key() ^ 0x5A5A_0001;
        ui.cmd_custom(r[0], r[1], r[2], r[3], Some(draw), 0, ver);
    }
    ui.box_begin(0, BOX_V, 2);
    ui.text(TS_SUBTITLE, &d.name);
    ui.text(TS_CAPTION | TX_SECONDARY, m.map_or(t("Unbekanntes Modell"), |m| t(m.name)));
    ui.box_end();
    // its console (Terminal panel) and its device window
    ui.spacer(-1);
    ui.cmd_button(CMD_CONSOLE, ST_ICON_ONLY | ST_SUBTLE);
    ui.cmd_button(CMD_DEVWIN, ST_ICON_ONLY | ST_SUBTLE);
    ui.box_end();
    name(n, ui, id, &d.name);
    // power: one undo step, the simulator switches it
    let mut on = (!d.off) as i32;
    if ui.switch(ID_INS_POWER, t("Eingeschaltet"), &mut on) & EV_ACT != 0 {
        cables::set_power(n, &[id], on != 0);
        ui.repass();
    }
    // the model and its ports
    let p = &n.doc.proj;
    let ports = p.ports(id);
    let used = ports.iter().filter(|(_, l)| l.is_some()).count();
    rows_begin(ui);
    match m {
        Some(m) => {
            row(ui, t("Modell"), t(m.name));
            row(ui, t("Beschreibung"), t(m.title));
            row(ui, t("Ports"), &if m.groups.is_empty() { String::from(t("keine")) } else { crate::app::tr::msg(&m.summary_msg()) });
            row(ui, t("Belegt"), &tf("{} von {}", &[&used.to_string(), &ports.len().to_string()]));
            row(ui, t("Befehlszeile"), t(match m.cli {
                CliStyle::Ios => "IOS-artig (Router, Switches)",
                CliStyle::Linux => "Linux-Shell",
                CliStyle::None => "keine",
            }));
        }
        None => row(ui, t("Modell"), &tf("„{}“ (unbekannt)", &[&d.model])),
    }
    row(ui, t("Position"), &format!("{}, {}", d.x, d.y));
    if let Some(e) = n.doc.sim.errors.get(&id) {
        row(ui, t("Simulator"), &tf("läuft nicht: {}", &[&crate::app::tr::msg(e)]));
    }
    ui.box_end();
    interfaces(n, ui, id);
    // its cables
    let p = &n.doc.proj;
    let nb = p.neighbors(id);
    if !nb.is_empty() {
        ui.text(TS_BODY_STRONG, &tf("Kabel ({})", &[&nb.len().to_string()]));
        for (own, other, port, lid) in nb.iter().take(24) {
            let peer = p.device(*other).map_or("?", |d| d.name.as_str());
            let kind = p.link(*lid).map_or("", |l| t(l.cable.title()));
            ui.next_size(-1, 0);
            ui.text(TS_CAPTION | TX_ELLIPSIS, &format!("{} → {} {} · {}", ifname::short(own), peer, ifname::short(port), kind));
        }
        if nb.len() > 24 {
            ui.text(TS_CAPTION | TX_SECONDARY, &tf("… und {} weitere", &[&(nb.len() - 24).to_string()]));
        }
    }
    // faults of the lab: power, a service, the disk (faults.rs)
    super::faults::device(n, ui, id);
}

// the name: Enter or the button applies, Esc puts it back
fn name(n: &mut Net, ui: &mut Ui, id: u32, now: &str) {
    let ins = &mut n.inspector;
    if ins.name_for != id || ins.name_src != now {
        if ins.name_for != id {
            ins.name_err.clear();
        }
        ins.name = String::from(now);
        ins.name_src = String::from(now);
        ins.name_for = id;
    }
    ui.text(TS_BODY_STRONG, t("Name"));
    ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
    if ui.focused() == ID_INS_NAME && ui.capture_shortcut(VK_ESCAPE, 0) {
        ins.name = String::from(now);
        ins.name_err.clear();
    }
    ui.next_size(-1, 0);
    let ev = ui.textbox(ID_INS_NAME, Some(t("Gerätename")), &mut ins.name, 64);
    let mut commit = ev & EV_COMMIT != 0 && ui.focused() == ID_INS_NAME && ins.name != now;
    if ui.button(ID_INS_RENAME, t("Umbenennen")) & EV_ACT != 0 {
        commit = true;
    }
    ui.tooltip(t("Den Namen übernehmen (Eingabetaste); F2 benennt auf der Zeichenfläche um"));
    ui.box_end();
    if ev & EV_ACT != 0 {
        ins.name_err.clear();
    }
    if commit {
        let name = ins.name.clone();
        match edits::rename(&mut n.doc.proj, id, &name) {
            Ok(Some(e)) => {
                n.doc.record(e);
                n.inspector.name_err.clear();
            }
            Ok(None) => n.inspector.name_err.clear(),
            Err(e) => n.inspector.name_err = crate::app::tr::msg(&e.msg()),
        }
        ui.repass();
    }
    if !n.inspector.name_err.is_empty() {
        let err = n.inspector.name_err.clone();
        crate::workspace::error_text(ui, &err, 0);
    }
}

// the interface grid and the fields of the chosen interface
fn interfaces(n: &mut Net, ui: &mut Ui, id: u32) {
    let rows = if_rows(n, id);
    if rows.is_empty() {
        return;
    }
    ui.text(TS_BODY_STRONG, &tf("Schnittstellen ({})", &[&rows.len().to_string()]));
    let ins: &mut Inspector = &mut n.inspector;
    if ins.ifs_for != id {
        ins.ifs_for = id;
        ins.ifs = GridState::default();
        ins.cfg_err.clear();
        // the first port with a cable, else the first one
        let first = rows.iter().find(|r| r.peer != "–").or(rows.first()).map(|r| r.key);
        ins.ifs.select_only(first);
    }
    let keys: Vec<u64> = rows.iter().map(|r| r.key).collect();
    ins.ifs.retain(&keys);
    let (hh, rh, _) = ui.style.data().grid_metrics(ui);
    let h = hh + rh * (rows.len() as i32).min(GRID_ROWS) + 2;
    // a column for IPv6 only where the device has IPv6
    let v6 = rows.iter().any(|r| r.ip6 != "–");
    let k = v6 as usize;
    ui.next_size(-1, h + 8);
    ui.scroll_begin(ID_INS_IF_SCROLL, SC_H);
    ui.next_size(GRID_W + if v6 { IP6_W } else { 0 }, h);
    let cols = cols_for(v6);
    let (first, end) = ui.grid_begin(ID_INS_IFACES, &mut ins.ifs, &cols, &keys);
    for i in first..end {
        let r = &rows[i as usize];
        ui.grid_row(&mut ins.ifs, i);
        ui.grid_cell(&ins.ifs, 0);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 4);
        if let Some(md) = r.media {
            ui.icon(port_icon(md), 14);
        }
        ui.text(TS_BODY | TX_ELLIPSIS, &ifname::short(&r.name));
        ui.box_end();
        ui.grid_cell(&ins.ifs, 1);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        ui.status_dot(r.badge);
        ui.text(TS_BODY | TX_ELLIPSIS, r.status);
        ui.box_end();
        ui.grid_text(&ins.ifs, 2, &r.ip);
        if v6 {
            ui.grid_text(&ins.ifs, 3, &r.ip6);
        }
        ui.grid_text(&ins.ifs, 3 + k, &r.peer);
        ui.grid_text(&ins.ifs, 4 + k, &r.speed);
        ui.grid_text(&ins.ifs, 5 + k, &r.vlan);
        ui.grid_text_ts(&ins.ifs, 6 + k, TS_CAPTION, &r.mac);
    }
    ui.grid_end(&mut ins.ifs);
    ui.scroll_end();
    let chosen = ins.ifs.cur.or_else(|| ins.ifs.sel.iter().next().copied());
    if let Some(r) = chosen.and_then(|k| rows.iter().find(|r| r.key == k)) {
        quick(n, ui, id, r);
    }
}

// the quick settings of the chosen interface
fn quick(n: &mut Net, ui: &mut Ui, id: u32, r: &IfRow) {
    let Some(d) = n.doc.proj.device(id).cloned() else { return };
    let host = n.doc.proj.model_of(id).is_some_and(|m| m.has(F_HOST));
    let console = r.media.is_some_and(|m| matches!(m, jfn_model::Media::Console | jfn_model::Media::Usb));
    if console {
        return;
    }
    ui.box_begin(0, BOX_V | BOX_CARD | pad(10), 8);
    ui.text(TS_BODY_STRONG, &ifname::short(&r.name));
    let mut result: Option<Result<(), String>> = None;
    // up / shut down
    let mut up = r.admin as i32;
    if ui.switch(ID_INS_IF_UP, t("Aktiv (nicht abgeschaltet)"), &mut up) & EV_ACT != 0 {
        result = Some(config::iface_up(&mut n.doc, id, &r.name, r.iface.is_some(), up != 0));
    }
    if let Some(f) = &r.iface {
        let dhcp = config::dhcp_service(&d, &f.name).is_some();
        // the address: what the interface has now (with DHCP: the lease, read only)
        let now = f.addrs.first().map(|a| a.to_string()).unwrap_or_default();
        let ins = &mut n.inspector;
        if ins.ip_src != (id, f.name.clone(), now.clone()) {
            ins.ip = now.clone();
            ins.ip_src = (id, f.name.clone(), now.clone());
        }
        ui.text(TS_BODY | TX_SECONDARY, t("IPv4-Adresse mit Präfix"));
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        if ui.focused() == ID_INS_IP && ui.capture_shortcut(VK_ESCAPE, 0) {
            ins.ip = now.clone();
        }
        ui.next_size(-1, 0);
        ui.next_flags(if dhcp { ST_DISABLED } else { 0 });
        let ev = ui.textbox(ID_INS_IP, Some(t("z. B. 192.168.1.10/24")), &mut ins.ip, 40);
        let mut set = ev & EV_COMMIT != 0 && ui.focused() == ID_INS_IP && ins.ip != now;
        ui.next_flags(if dhcp || ins.ip == now { ST_DISABLED } else { 0 });
        if ui.button(ID_INS_IP_APPLY, t("Setzen")) & EV_ACT != 0 {
            set = true;
        }
        ui.box_end();
        if set && !dhcp {
            let text = n.inspector.ip.clone();
            result = Some(config::address(&mut n.doc, id, &f.name, &text));
        }
        let mut on = dhcp as i32;
        if ui.checkbox(ID_INS_DHCP, t("Adresse per DHCP beziehen"), &mut on) & EV_ACT != 0 {
            result = Some(config::dhcp(&mut n.doc, id, &f.name, on != 0));
        }
        // its IPv6 addresses (read only: the consoles set them)
        if !f.addrs6.is_empty() {
            ui.text(TS_BODY | TX_SECONDARY, t("IPv6-Adressen"));
            for a in &f.addrs6 {
                ui.next_size(-1, 0);
                ui.text(TS_CAPTION | TX_ELLIPSIS, &addr6_line(a));
            }
        }
    }
    // the default gateway of a host
    if host && r.iface.is_some() {
        let now = config::default_gateways(n.doc.sim.engine(), id).first().map(|g| g.to_string()).unwrap_or_default();
        let ins = &mut n.inspector;
        if ins.gw_src != (id, now.clone()) {
            ins.gw = now.clone();
            ins.gw_src = (id, now.clone());
        }
        ui.text(TS_BODY | TX_SECONDARY, t("Standard-Gateway"));
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        if ui.focused() == ID_INS_GW && ui.capture_shortcut(VK_ESCAPE, 0) {
            ins.gw = now.clone();
        }
        ui.next_size(-1, 0);
        let ev = ui.textbox(ID_INS_GW, Some(t("z. B. 192.168.1.1")), &mut ins.gw, 40);
        let mut set = ev & EV_COMMIT != 0 && ui.focused() == ID_INS_GW && ins.gw != now;
        ui.next_flags(if ins.gw == now { ST_DISABLED } else { 0 });
        if ui.button(ID_INS_GW_APPLY, t("Setzen")) & EV_ACT != 0 {
            set = true;
        }
        ui.box_end();
        if set {
            let text = n.inspector.gw.clone();
            result = Some(config::gateway(&mut n.doc, id, &text));
        }
    }
    match result {
        Some(Ok(())) => {
            n.inspector.cfg_err.clear();
            ui.repass();
        }
        Some(Err(e)) => {
            n.inspector.cfg_err = e;
            ui.repass();
        }
        None => {}
    }
    if !n.inspector.cfg_err.is_empty() {
        let err = n.inspector.cfg_err.clone();
        crate::workspace::error_text(ui, &err, 0);
    } else {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Gilt sofort und wird Teil der Startkonfiguration des Geräts."));
    }
    ui.box_end();
}
