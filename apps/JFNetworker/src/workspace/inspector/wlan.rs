// wlan -- WLAN in the inspector: an access point's networks (SSID, band and channel, security
// with a masked passphrase, VLAN, guest network, client isolation; add and remove) with its
// ranges and stations; a station's network (SSID and passphrase; join, leave) with its state in
// the simulator (the access point it is on, signal, rate, the networks in reach); the project's
// scale
//
//   Bufs                       the edit buffers: SSID, passphrase and VLAN of each network, the
//                              station's fields, the scale; they start over when the value
//                              changes elsewhere (undo, the console)
//   device(n, ui, id)          after the interfaces of an access point or a station
//   project(n, ui)             the scale on the project's page
//
// Lists (band, channel, security) and switches apply at once, text fields with Enter or their
// button (Esc puts them back); every change is one undo step (doc/wlan.rs) the simulator
// follows without a restart.  A network that cannot be offered (jfn_model::wlan::check_net)
// says why below it.  The VLAN of a network is a setting of plain access points; the home router
// keeps its stations in its LAN (a guest network reaches only the router).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::catalog::{Model, F_ROUTER};
use jfn_model::wlan::{self, Band, Role, Security, WlanJoin, WlanNet};
use jfn_model::ifname;
use jfn_sim::StaState;
use ui::*;

use super::{field_begin, row, rows_begin};
use crate::app::constants::*;
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::workspace::canvas::wlan::{bars, mbit};

/// networks an access point shows at most
const NETS_MAX: usize = 16;

/// the edit buffers
#[derive(Default)]
pub(crate) struct Bufs {
    /// the device and the networks the buffers belong to
    src: (u32, Vec<WlanNet>),
    ssid: Vec<String>,
    pass: Vec<String>,
    vlan: Vec<String>,
    show: Vec<bool>,
    /// why the last change of network k was refused
    err: (usize, String),
    /// the station and what it joins
    join_src: (u32, Option<WlanJoin>),
    j_ssid: String,
    j_pass: String,
    j_show: bool,
    j_err: String,
    scale: String,
    scale_src: u32,
    scale_err: String,
}

/// the WLAN part of a device's page
pub(crate) fn device(n: &mut Net, ui: &mut Ui, id: u32) {
    let Some(m) = n.doc.proj.model_of(id) else { return };
    match wlan::role(m) {
        Role::Ap => ap(n, ui, id, m),
        Role::Station => station(n, ui, id),
        Role::None => {}
    }
}

/// the name of a device ("?" when gone)
fn name_of(n: &Net, dev: u32) -> String {
    n.doc.proj.device(dev).map_or_else(|| String::from("?"), |d| d.name.clone())
}

fn ap(n: &mut Net, ui: &mut Ui, id: u32, m: &'static Model) {
    let Some(d) = n.doc.proj.device(id).cloned() else { return };
    let nets: Vec<WlanNet> = d.wlan.as_ref().map(|w| w.nets.clone()).unwrap_or_default();
    let b = &mut n.inspector.wl;
    if b.src != (id, nets.clone()) {
        if b.src.0 != id {
            b.show.clear();
            b.err = (0, String::new());
        }
        b.ssid = nets.iter().map(|x| x.ssid.clone()).collect();
        b.pass = nets.iter().map(|x| x.passphrase.clone()).collect();
        b.vlan = nets.iter().map(|x| if x.vlan == 0 { String::new() } else { x.vlan.to_string() }).collect();
        b.show.resize(nets.len(), false);
        b.src = (id, nets.clone());
    }
    ui.text(TS_BODY_STRONG, &tf("WLAN ({})", &[&nets.len().to_string()]));
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Die Netze dieses Access Points. Eine Station nimmt in ihrer Reichweite das beste Netz ihrer SSID."));
    let bands: Vec<Band> = Band::ALL.iter().copied().filter(|x| wlan::ap_bands(m) & x.bit() != 0).collect();
    let plain = !m.has(F_ROUTER);
    let stations = n.doc.sim.engine().wlan_stations_of(id);
    let mut change: Option<(usize, WlanNet, &'static str)> = None;
    let mut remove: Option<usize> = None;
    for (k, net) in nets.iter().enumerate().take(NETS_MAX) {
        let base = ID_WL_NET0 + 0x10 * k as u32;
        ui.box_begin(0, BOX_V | BOX_CARD | pad(10), 6);
        ui.text(TS_BODY_STRONG, &if net.ssid.is_empty() { String::from(t("Netz ohne SSID")) } else { net.ssid.clone() });
        // the SSID
        let b = &mut n.inspector.wl;
        let mut apply = false;
        field_begin(ui, t("SSID"));
        if ui.focused() == base + WL_SSID && ui.capture_shortcut(VK_ESCAPE, 0) {
            b.ssid[k] = net.ssid.clone();
        }
        ui.next_size(-1, 0);
        if ui.textbox(base + WL_SSID, Some(t("Name des Netzes")), &mut b.ssid[k], 32) & EV_COMMIT != 0 && ui.focused() == base + WL_SSID {
            apply = true;
        }
        ui.box_end();
        // band and channel
        let mut bsel = bands.iter().position(|&x| x == net.band).map_or(0, |i| i as i32);
        let bnames: Vec<&str> = bands.iter().map(|x| t(x.title())).collect();
        field_begin(ui, t("Band"));
        ui.next_size(-1, 0);
        if ui.combobox(base + WL_BAND, &mut bsel, &bnames) & EV_ACT != 0
            && let Some(&nb) = bands.get(bsel as usize)
        {
            let mut x = net.clone();
            x.band = nb;
            x.channel = 0;
            // 6 GHz takes WPA3 only
            if nb == Band::G6 && x.security != Security::Wpa3 {
                x.security = Security::Wpa3;
            }
            change = Some((k, x, "Band"));
        }
        ui.box_end();
        let chans = wlan::channels(net.band);
        let cnames: Vec<String> = chans.iter().map(|c| format!("{} ({} MHz)", c, wlan::freq_mhz(net.band, *c))).collect();
        let crefs: Vec<&str> = cnames.iter().map(|s| s.as_str()).collect();
        let mut csel = chans.iter().position(|&c| c == wlan::channel_of(net)).map_or(0, |i| i as i32);
        field_begin(ui, t("Kanal"));
        ui.next_size(-1, 0);
        if ui.combobox(base + WL_CHANNEL, &mut csel, &crefs) & EV_ACT != 0
            && let Some(&c) = chans.get(csel as usize)
        {
            let mut x = net.clone();
            x.channel = c;
            change = Some((k, x, "Kanal"));
        }
        ui.box_end();
        // security and passphrase
        let secs: &[Security] = if net.band == Band::G6 { &[Security::Wpa3] } else { Security::ALL };
        let snames: Vec<&str> = secs.iter().map(|s| t(s.title())).collect();
        let mut ssel = secs.iter().position(|&s| s == net.security).map_or(0, |i| i as i32);
        field_begin(ui, t("Sicherheit"));
        ui.next_size(-1, 0);
        if ui.combobox(base + WL_SECURITY, &mut ssel, &snames) & EV_ACT != 0
            && let Some(&s) = secs.get(ssel as usize)
        {
            let mut x = net.clone();
            x.security = s;
            change = Some((k, x, "Sicherheit"));
        }
        ui.box_end();
        let b = &mut n.inspector.wl;
        if net.security.secured() {
            field_begin(ui, t("Passphrase"));
            if ui.focused() == base + WL_PASS && ui.capture_shortcut(VK_ESCAPE, 0) {
                b.pass[k] = net.passphrase.clone();
            }
            ui.next_size(-1, 0);
            let mask = if b.show[k] { 0 } else { TB_PASSWORD };
            if ui.textbox(base + WL_PASS, Some(t("8 bis 63 Zeichen")), &mut b.pass[k], 63 | mask) & EV_COMMIT != 0 && ui.focused() == base + WL_PASS {
                apply = true;
            }
            ui.box_end();
            let mut sh = b.show[k] as i32;
            if ui.checkbox(base + WL_SHOW, t("Passphrase zeigen"), &mut sh) & EV_ACT != 0 {
                b.show[k] = sh != 0;
                ui.repass();
            }
        }
        // VLAN (plain access points)
        if plain {
            field_begin(ui, t("VLAN"));
            if ui.focused() == base + WL_VLAN && ui.capture_shortcut(VK_ESCAPE, 0) {
                b.vlan[k] = if net.vlan == 0 { String::new() } else { net.vlan.to_string() };
            }
            ui.next_size(-1, 0);
            if ui.textbox(base + WL_VLAN, Some(t("keins")), &mut b.vlan[k], 4 | TB_DIGITS) & EV_COMMIT != 0 && ui.focused() == base + WL_VLAN {
                apply = true;
            }
            ui.box_end();
        }
        let mut guest = net.guest as i32;
        if ui.checkbox(base + WL_GUEST, t("Gastnetz"), &mut guest) & EV_ACT != 0 {
            let mut x = net.clone();
            x.guest = guest != 0;
            change = Some((k, x, "Gastnetz"));
        }
        let mut iso = net.isolate as i32;
        if ui.checkbox(base + WL_ISOLATE, t("Client-Isolation"), &mut iso) & EV_ACT != 0 {
            let mut x = net.clone();
            x.isolate = iso != 0;
            change = Some((k, x, "Client-Isolation"));
        }
        ui.box_begin(0, BOX_H | BOX_END, 8);
        let b = &n.inspector.wl;
        let typed = b.ssid[k] != net.ssid || b.pass[k] != net.passphrase || b.vlan[k] != if net.vlan == 0 { String::new() } else { net.vlan.to_string() };
        ui.next_flags(if typed { 0 } else { ST_DISABLED });
        if ui.button(base + WL_APPLY, t("Übernehmen")) & EV_ACT != 0 {
            apply = true;
        }
        if ui.button(base + WL_REMOVE, t("Entfernen")) & EV_ACT != 0 {
            remove = Some(k);
        }
        ui.box_end();
        if apply {
            let b = &n.inspector.wl;
            let vlan = b.vlan[k].trim().parse::<u16>().unwrap_or(0);
            let mut x = net.clone();
            x.ssid = String::from(b.ssid[k].trim());
            x.passphrase = b.pass[k].clone();
            x.vlan = vlan;
            change = Some((k, x, "Netz"));
        }
        // what is wrong with it, its stations
        let b = &n.inspector.wl;
        if b.err.0 == k && !b.err.1.is_empty() {
            let e = b.err.1.clone();
            crate::workspace::error_text(ui, &e, 0);
        } else if let Some(why) = wlan::check_net(net) {
            crate::workspace::error_text(ui, &tf("Wird nicht angeboten: {}", &[&crate::app::tr::msg(&why)]), 0);
        }
        let mine: Vec<String> = stations.iter().filter(|a| a.ssid == net.ssid && a.band == net.band).map(|a| tf("{} · {} dBm · {} Mbit/s", &[&name_of(n, a.sta), &a.dbm.to_string(), &mbit(a.kbps)])).collect();
        if !mine.is_empty() {
            rows_begin(ui);
            row(ui, t("Stationen"), &mine.join("\n"));
            ui.box_end();
        }
        ui.box_end();
    }
    ui.box_begin(0, BOX_H, 8);
    ui.next_flags(if nets.len() >= NETS_MAX { ST_DISABLED } else { 0 });
    if ui.button(ID_WL_ADD, t("Netz hinzufügen")) & EV_ACT != 0 {
        let x = wlan::new_net(m, &nets);
        let label = format!("WLAN-Netz von {} hinzufügen", d.name);
        crate::doc::wlan::set(&mut n.doc, id, &label, |w| w.nets.push(x));
        ui.repass();
    }
    ui.box_end();
    // the ranges of the bands, in metres and units of the canvas
    let scale = wlan::dip_per_m(&n.doc.proj);
    let lines: Vec<String> = bands.iter().map(|&band| {
        let r = wlan::range_m(m, band);
        tf("{}: {} m ({} Einheiten)", &[t(band.title()), &r.to_string(), &(r * scale).to_string()])
    }).collect();
    rows_begin(ui);
    row(ui, t("Reichweite"), &lines.join("\n"));
    ui.box_end();
    if let Some((k, x, what)) = change {
        if let Some(why) = wlan::check_net(&x).filter(|_| what == "Netz") {
            n.inspector.wl.err = (k, crate::app::tr::msg(&why));
        } else {
            n.inspector.wl.err = (0, String::new());
            let label = format!("WLAN-Netz {} von {} ändern", x.ssid, d.name);
            crate::doc::wlan::set(&mut n.doc, id, &label, |w| {
                if let Some(slot) = w.nets.get_mut(k) {
                    *slot = x;
                }
            });
        }
        ui.repass();
    }
    if let Some(k) = remove {
        let label = format!("WLAN-Netz von {} entfernen", d.name);
        crate::doc::wlan::set(&mut n.doc, id, &label, |w| {
            if k < w.nets.len() {
                w.nets.remove(k);
            }
        });
        n.inspector.wl.err = (0, String::new());
        ui.repass();
    }
}

/// the state of a station in words
fn state_text(v: &jfn_sim::StaView, now: jfn_sim::Ns) -> String {
    let ssid = v.join.as_ref().map_or("", |j| j.ssid.as_str());
    match v.state {
        StaState::Idle if v.join.is_none() => String::from(t("kein Netz eingestellt")),
        StaState::Idle => String::from(t("aus (Gerät oder Port)")),
        StaState::Fixed => String::from(t("fest verbunden (Funkkabel des Projekts)")),
        StaState::Scanning => tf("sucht „{}“", &[ssid]),
        StaState::Joining => tf("meldet sich bei „{}“ an", &[ssid]),
        StaState::Handshake => String::from(t("4-Wege-Handshake läuft")),
        StaState::Connected => String::from(t("verbunden")),
        StaState::Roaming => String::from(t("wechselt den Access Point")),
        StaState::Backoff => tf("{}. Fehlschlag, neuer Versuch in {} s",&[&v.failures.to_string(), &v.retry_at.map_or(0, |at| at.saturating_sub(now).div_ceil(jfn_sim::SEC)).to_string()]),
        StaState::NoNetwork => tf("„{}“ ist nicht in Reichweite", &[ssid]),
        StaState::NoPassphrase => tf("„{}“ braucht eine Passphrase", &[ssid]),
        StaState::GaveUp => tf("aufgegeben nach {} Fehlschlägen: Passphrase prüfen", &[&v.failures.to_string()]),
    }
}

fn station(n: &mut Net, ui: &mut Ui, id: u32) {
    let Some(d) = n.doc.proj.device(id).cloned() else { return };
    let join = d.wlan.as_ref().and_then(|w| w.join.clone());
    let b = &mut n.inspector.wl;
    if b.join_src != (id, join.clone()) {
        if b.join_src.0 != id {
            b.j_show = false;
            b.j_err.clear();
        }
        b.j_ssid = join.as_ref().map(|j| j.ssid.clone()).unwrap_or_default();
        b.j_pass = join.as_ref().map(|j| j.passphrase.clone()).unwrap_or_default();
        b.join_src = (id, join.clone());
    }
    ui.text(TS_BODY_STRONG, t("WLAN"));
    // what the simulator does with it
    let eng = n.doc.sim.engine();
    let v = eng.wlan_station(id);
    rows_begin(ui);
    if let Some(v) = &v {
        row(ui, t("Zustand"), &state_text(v, eng.now()));
        if let Some(a) = &v.assoc {
            let port = eng.node(a.ap).map(|x| ifname::short(x.port_name(a.ap_port))).unwrap_or_default();
            row(ui, t("Access Point"), &format!("{} {}", name_of(n, a.ap), port));
            row(ui, t("Netz"), &format!("{} · {} · {} {} · {}", a.ssid, t(a.band.title()), t("Kanal"), a.channel, t(a.security.title())));
            if a.lost {
                row(ui, t("Signal"), t("außer Reichweite"));
            } else {
                row(ui, t("Signal"), &tf("{} dBm ({} von 4 Balken)", &[&a.dbm.to_string(), &bars(a.dbm).to_string()]));
                row(ui, t("Rate"), &tf("{} Mbit/s", &[&mbit(a.kbps)]));
            }
        }
    }
    ui.box_end();
    if v.as_ref().is_some_and(|v| v.state == StaState::Fixed) {
        return;
    }
    // the network to join
    let b = &mut n.inspector.wl;
    let mut apply = false;
    field_begin(ui, t("SSID"));
    if ui.focused() == ID_WL_JOIN_SSID && ui.capture_shortcut(VK_ESCAPE, 0) {
        b.j_ssid = join.as_ref().map(|j| j.ssid.clone()).unwrap_or_default();
    }
    ui.next_size(-1, 0);
    if ui.textbox(ID_WL_JOIN_SSID, Some(t("Name des Netzes")), &mut b.j_ssid, 32) & EV_COMMIT != 0 && ui.focused() == ID_WL_JOIN_SSID {
        apply = true;
    }
    ui.box_end();
    field_begin(ui, t("Passphrase"));
    if ui.focused() == ID_WL_JOIN_PASS && ui.capture_shortcut(VK_ESCAPE, 0) {
        b.j_pass = join.as_ref().map(|j| j.passphrase.clone()).unwrap_or_default();
    }
    ui.next_size(-1, 0);
    let mask = if b.j_show { 0 } else { TB_PASSWORD };
    if ui.textbox(ID_WL_JOIN_PASS, Some(t("für WPA2 / WPA3")), &mut b.j_pass, 63 | mask) & EV_COMMIT != 0 && ui.focused() == ID_WL_JOIN_PASS {
        apply = true;
    }
    ui.box_end();
    let mut sh = b.j_show as i32;
    if ui.checkbox(ID_WL_JOIN_SHOW, t("Passphrase zeigen"), &mut sh) & EV_ACT != 0 {
        b.j_show = sh != 0;
        ui.repass();
    }
    ui.box_begin(0, BOX_H | BOX_END, 8);
    if ui.accent_button(ID_WL_JOIN_APPLY, t("Anmelden")) & EV_ACT != 0 {
        apply = true;
    }
    ui.next_flags(if join.is_none() { ST_DISABLED } else { 0 });
    if ui.button(ID_WL_JOIN_LEAVE, t("Abmelden")) & EV_ACT != 0 {
        let label = format!("WLAN von {} trennen", d.name);
        crate::doc::wlan::set(&mut n.doc, id, &label, |w| w.join = None);
        n.inspector.wl.j_err.clear();
        ui.repass();
    }
    ui.box_end();
    if apply {
        let b = &n.inspector.wl;
        let j = WlanJoin { ssid: String::from(b.j_ssid.trim()), passphrase: b.j_pass.clone() };
        match wlan::check_join(&j) {
            Some(why) => n.inspector.wl.j_err = crate::app::tr::msg(&why),
            None => {
                n.inspector.wl.j_err.clear();
                let label = format!("WLAN von {} einstellen", d.name);
                crate::doc::wlan::set(&mut n.doc, id, &label, |w| w.join = Some(j));
            }
        }
        ui.repass();
    }
    if !n.inspector.wl.j_err.is_empty() {
        let e = n.inspector.wl.j_err.clone();
        crate::workspace::error_text(ui, &e, 0);
    }
    // what a scan finds now
    let found = n.doc.sim.engine().wlan_survey(id, None);
    if !found.is_empty() {
        ui.text(TS_BODY | TX_SECONDARY, t("In Reichweite"));
        for x in found.iter().take(8) {
            ui.next_size(-1, 0);
            let sec = if x.security == Security::Open { String::from(t("offen")) } else { String::from(t(x.security.title())) };
            ui.text(TS_CAPTION | TX_ELLIPSIS, &format!("{} · {} · {} · {} dBm · {}", x.ssid, name_of(n, x.ap), t(x.band.title()), x.dbm(), sec));
        }
    }
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Die Station sucht ihr Netz, meldet sich beim besten Access Point an und wechselt, wenn ein anderer deutlich besser ist."));
}

/// the scale on the project's page
pub(crate) fn project(n: &mut Net, ui: &mut Ui) {
    let now = wlan::dip_per_m(&n.doc.proj);
    let b = &mut n.inspector.wl;
    if b.scale_src != now || b.scale.is_empty() && ui.focused() != ID_WL_SCALE {
        b.scale = now.to_string();
        b.scale_src = now;
    }
    ui.text(TS_BODY_STRONG, t("Maßstab"));
    let mut set = false;
    field_begin(ui, t("Je Meter"));
    if ui.focused() == ID_WL_SCALE && ui.capture_shortcut(VK_ESCAPE, 0) {
        b.scale = now.to_string();
        b.scale_err.clear();
    }
    ui.next_size(-1, 0);
    if ui.textbox(ID_WL_SCALE, Some("10"), &mut b.scale, 4 | TB_DIGITS) & EV_COMMIT != 0 && ui.focused() == ID_WL_SCALE {
        set = true;
    }
    if ui.button(ID_WL_SCALE_APPLY, t("Setzen")) & EV_ACT != 0 {
        set = true;
    }
    ui.box_end();
    if set {
        match n.inspector.wl.scale.trim().parse::<u32>() {
            Ok(v) if (1..=1000).contains(&v) => {
                n.inspector.wl.scale_err.clear();
                crate::doc::wlan::scale(&mut n.doc, v);
            }
            _ => n.inspector.wl.scale_err = String::from(t("Eine ganze Zahl von 1 bis 1000.")),
        }
        ui.repass();
    }
    if !n.inspector.wl.scale_err.is_empty() {
        let e = n.inspector.wl.scale_err.clone();
        crate::workspace::error_text(ui, &e, 0);
    }
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &tf("Ein Meter sind {} Einheiten der Zeichenfläche (bei 100 % Pixel); danach richten sich die Reichweiten im WLAN.", &[&now.to_string()]));
}
