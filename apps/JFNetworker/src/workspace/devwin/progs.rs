// progs -- a small program of a host's desktop (desktop.rs): IP-Konfiguration
//
//   IpState::new()                     its fields with nothing typed
//   ipconfig(n, ui, k, id, dev)        IP-Konfiguration: a static address (address, mask,
//                                      gateway) or DHCP per interface; it types shell commands
//                                      on the console as the forms do (DHCP: the DHCP client
//                                      service, doc/config.rs), one undo step; below that the
//                                      802.1X panel: wired 802.1X and enterprise WLAN (method,
//                                      identity, password), written as the supplicant's file by
//                                      Action::Supplicant, and what the supplicant says now
//
// Ids (id: the device window's desktop block + 0x10, as before the desktop had windows): + 1 the
// interface, + 2 / + 3 Statisch / DHCP, + 4 .. + 6 address, mask, gateway, + 7 Übernehmen.  The
// 802.1X panel: ID_WPA0 + 0x40 k: + 1 802.1X am Kabel, + 2 .. + 4 method, identity, password, + 5
// Unternehmens-WLAN, + 6 .. + 9 SSID, method, identity, password, + 10 Anmeldung übernehmen.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use eapm::Method;
use jfn_sim::supp::{self, KeyMgmt, NetBlock, SuppState, SuppView};
use jfn_sim::{Action, Cmd};
use netpkt::{Ipv4, Prefix4};
use ui::*;

use crate::app::constants::ID_WPA0;
use crate::app::tr::{self, t, tf};
use crate::app::Net;
use crate::doc::{config, console};

/// the fields of IP-Konfiguration and what they started from
pub(crate) struct IpState {
    iface: i32,
    mode: i32,
    addr: String,
    mask: String,
    gw: String,
    src: (String, String, String, String, i32),
    err: String,
    // the 802.1X panel and what it started from
    wired: i32,
    w_eap: i32,
    w_id: String,
    w_pw: String,
    wl: i32,
    wl_ssid: String,
    wl_eap: i32,
    wl_id: String,
    wl_pw: String,
    conf_src: Option<Vec<NetBlock>>,
    conf_err: String,
}

impl IpState {
    pub(crate) fn new() -> IpState {
        IpState {
            iface: 0,
            mode: 0,
            addr: String::new(),
            mask: String::new(),
            gw: String::new(),
            src: (String::new(), String::new(), String::new(), String::new(), -1),
            err: String::new(),
            wired: 0,
            w_eap: 0,
            w_id: String::new(),
            w_pw: String::new(),
            wl: 0,
            wl_ssid: String::new(),
            wl_eap: 0,
            wl_id: String::new(),
            wl_pw: String::new(),
            conf_src: None,
            conf_err: String::new(),
        }
    }
    /// the panel from the blocks of the file
    fn fill(&mut self, blocks: &[NetBlock]) {
        let w = blocks.iter().find(|b| b.key_mgmt == KeyMgmt::Ieee8021x);
        self.wired = w.is_some() as i32;
        self.w_eap = (w.and_then(|b| b.eap) == Some(Method::Pwd)) as i32;
        self.w_id = w.map(|b| b.identity.clone()).unwrap_or_default();
        self.w_pw = w.map(|b| b.password.clone()).unwrap_or_default();
        let e = blocks.iter().find(|b| b.key_mgmt == KeyMgmt::WpaEap);
        self.wl = e.is_some() as i32;
        self.wl_ssid = e.map(|b| b.ssid.clone()).unwrap_or_default();
        self.wl_eap = (e.and_then(|b| b.eap) == Some(Method::Md5)) as i32;
        self.wl_id = e.map(|b| b.identity.clone()).unwrap_or_default();
        self.wl_pw = e.map(|b| b.password.clone()).unwrap_or_default();
        self.conf_src = Some(blocks.to_vec());
    }
}

// a "label  field" row (the field follows, then box_end)
fn field(ui: &mut Ui, label: &'static str) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(150, 0);
    ui.text(TS_BODY | TX_SECONDARY, t(label));
}

/// IP-Konfiguration of device window k
pub(crate) fn ipconfig(n: &mut Net, ui: &mut Ui, k: usize, id: u32, dev: u32) {
    let Some(d) = n.doc.proj.device(dev).cloned() else { return };
    let ifs: Vec<(String, Option<Prefix4>)> = n.doc.sim.state().ifaces(dev).into_iter().filter(|f| f.name != "lo").map(|f| (f.name.clone(), f.addrs.first().copied())).collect();
    let gw_now = config::default_gateways(n.doc.sim.state(), dev).first().map(|g| g.to_string()).unwrap_or_default();
    let Some(w) = n.devwin.get(k) else { return };
    let s = &mut w.desk.ip;
    if ifs.is_empty() {
        ui.text(TS_BODY | TX_SECONDARY, t("Dieses Gerät hat keine IPv4-Schnittstelle."));
        return;
    }
    s.iface = s.iface.clamp(0, ifs.len() as i32 - 1);
    let (name, now) = ifs[s.iface as usize].clone();
    let dhcp = config::dhcp_service(&d, &name).is_some();
    let src = (name.clone(), now.map(|p| p.addr.to_string()).unwrap_or_default(), now.map(|p| p.mask().to_string()).unwrap_or_default(), gw_now.clone(), dhcp as i32);
    if src != s.src {
        s.addr = src.1.clone();
        s.mask = src.2.clone();
        s.gw = src.3.clone();
        s.mode = src.4;
        s.src = src;
    }
    let mut apply = false;
    ui.next_size(-1, -1);
    ui.scroll_begin(id, SC_V);
    ui.box_begin(0, BOX_V | pad(4), 8);
    if ifs.len() > 1 {
        let names: Vec<&str> = ifs.iter().map(|f| f.0.as_str()).collect();
        field(ui, "Schnittstelle");
        ui.next_size(200, 0);
        if ui.combobox(id + 1, &mut s.iface, &names) & EV_ACT != 0 {
            s.err.clear();
            ui.repass();
        }
        ui.box_end();
    } else {
        field(ui, "Schnittstelle");
        ui.text(TS_BODY, &name);
        ui.box_end();
    }
    field(ui, "Adresse");
    ui.radio(id + 2, t("Statisch"), &mut s.mode, 0);
    ui.radio(id + 3, t("DHCP"), &mut s.mode, 1);
    ui.box_end();
    let st = if s.mode == 1 { ST_DISABLED } else { 0 };
    field(ui, "IPv4-Adresse");
    ui.next_size(200, 0);
    ui.next_flags(st);
    ui.textbox(id + 4, Some(t("z. B. 192.168.1.10")), &mut s.addr, 20);
    ui.box_end();
    field(ui, "Subnetzmaske");
    ui.next_size(200, 0);
    ui.next_flags(st);
    ui.textbox(id + 5, Some(t("z. B. 255.255.255.0")), &mut s.mask, 20);
    ui.box_end();
    field(ui, "Standard-Gateway");
    ui.next_size(200, 0);
    ui.next_flags(st);
    ui.textbox(id + 6, Some(t("z. B. 192.168.1.1")), &mut s.gw, 20);
    ui.box_end();
    if dhcp {
        let lease = now.map_or_else(|| String::from(t("noch keine Adresse")), |p| format!("{}{}", p, if gw_now.is_empty() { String::new() } else { tf(", Gateway {}", &[&gw_now]) }));
        field(ui, "Per DHCP");
        ui.text(TS_BODY, &lease);
        ui.box_end();
    }
    ui.box_begin(0, BOX_H | BOX_END, 8);
    let changed = s.mode != s.src.4 || (s.mode == 0 && (s.addr.trim() != s.src.1 || s.mask.trim() != s.src.2 || s.gw.trim() != s.src.3));
    ui.next_flags(if changed { 0 } else { ST_DISABLED });
    if ui.accent_button(id + 7, t("Übernehmen")) & EV_ACT != 0 {
        apply = true;
    }
    ui.box_end();
    if !s.err.is_empty() {
        let e = s.err.clone();
        crate::workspace::error_text(ui, &e, 0);
    } else {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Statische Einstellungen laufen als Befehle im Terminal und werden Teil der Startkonfiguration; DHCP holt Adresse und Gateway vom Server."));
    }
    let (mode, addr, mask, gw) = (s.mode, String::from(s.addr.trim()), String::from(s.mask.trim()), String::from(s.gw.trim()));
    wpa_panel(n, ui, k, dev);
    ui.box_end();
    ui.scroll_end();
    if !apply {
        return;
    }
    let r = set_ip(n, dev, &name, mode == 1, dhcp, &addr, &mask, &gw, &gw_now);
    if let Some(w) = n.devwin.get(k) {
        w.desk.ip.err = r.err().unwrap_or_default();
    }
    ui.repass();
}

/// what the supplicant says of a port
fn supp_line(v: &SuppView) -> (String, u32) {
    let (text, badge) = match v.state {
        SuppState::Off => (String::from(t("aus")), BADGE_NEUTRAL),
        SuppState::Connecting => (String::from(t("sucht einen Authenticator")), BADGE_INFO),
        SuppState::NoPeer => (String::from(t("niemand antwortet, läuft ohne 802.1X")), BADGE_NEUTRAL),
        SuppState::Authenticating => (tf("meldet {} an", &[&v.user]), BADGE_INFO),
        SuppState::Authenticated => (tf("angemeldet als {} ({})", &[&v.user, &v.method]), BADGE_OK),
        SuppState::Held => (tf("{} abgelehnt, wartet", &[&v.user]), BADGE_CRIT),
        SuppState::NoCreds => (String::from(t("keine Zugangsdaten")), BADGE_CRIT),
    };
    (format!("{}: {}", v.name, text), badge)
}

/// the 802.1X panel of IP-Konfiguration: wired 802.1X and enterprise WLAN as the supplicant's
/// file (Action::Supplicant writes it and reads it again)
fn wpa_panel(n: &mut Net, ui: &mut Ui, k: usize, dev: u32) {
    let id = ID_WPA0 + 0x40 * k as u32;
    let eng = n.doc.sim.state();
    let blocks = eng.supp_conf(dev);
    let views = eng.supp_views(dev);
    let (wired_port, radio) = n.doc.proj.model_of(dev).map_or((false, false), |m| (m.ports().iter().any(|p| p.media.is_ethernet() && !p.media.is_radio()), jfn_model::wlan::role(m) == jfn_model::wlan::Role::Station));
    let Some(w) = n.devwin.get(k) else { return };
    let s = &mut w.desk.ip;
    if s.conf_src.as_ref() != Some(&blocks) {
        s.fill(&blocks);
    }
    ui.text(TS_BODY_STRONG, t("Anmeldung am Netz (802.1X)"));
    if wired_port {
        field(ui, "Kabel");
        ui.checkbox(id + 1, t("802.1X an der Netzwerkbuchse"), &mut s.wired);
        ui.box_end();
        if s.wired != 0 {
            field(ui, "EAP-Methode");
            ui.next_size(200, 0);
            ui.combobox(id + 2, &mut s.w_eap, &["EAP-MD5", "EAP-PWD"]);
            ui.box_end();
            field(ui, "Identität");
            ui.next_size(200, 0);
            ui.textbox(id + 3, Some(t("der Benutzer beim RADIUS-Server")), &mut s.w_id, 64);
            ui.box_end();
            field(ui, "Passwort");
            ui.next_size(200, 0);
            ui.textbox(id + 4, None, &mut s.w_pw, 64 | TB_PASSWORD);
            ui.box_end();
        }
    }
    if radio {
        field(ui, "WLAN");
        ui.checkbox(id + 5, t("Unternehmens-WLAN (WPA-Enterprise)"), &mut s.wl);
        ui.box_end();
        if s.wl != 0 {
            field(ui, "SSID");
            ui.next_size(200, 0);
            ui.textbox(id + 6, Some(t("Name des Netzes")), &mut s.wl_ssid, 32);
            ui.box_end();
            field(ui, "EAP-Methode");
            ui.next_size(200, 0);
            ui.combobox(id + 7, &mut s.wl_eap, &["EAP-PWD", "EAP-MD5"]);
            ui.box_end();
            field(ui, "Identität");
            ui.next_size(200, 0);
            ui.textbox(id + 8, Some(t("der Benutzer beim RADIUS-Server")), &mut s.wl_id, 64);
            ui.box_end();
            field(ui, "Passwort");
            ui.next_size(200, 0);
            ui.textbox(id + 9, None, &mut s.wl_pw, 64 | TB_PASSWORD);
            ui.box_end();
        }
    }
    // the new file: the blocks of the panel and the others as they were
    let mut new: Vec<NetBlock> = Vec::new();
    if s.wired != 0 {
        new.push(NetBlock { ssid: String::new(), key_mgmt: KeyMgmt::Ieee8021x, eap: Some(if s.w_eap == 1 { Method::Pwd } else { Method::Md5 }), identity: String::from(s.w_id.trim()), password: s.w_pw.clone() });
    }
    if s.wl != 0 {
        new.push(NetBlock { ssid: String::from(s.wl_ssid.trim()), key_mgmt: KeyMgmt::WpaEap, eap: Some(if s.wl_eap == 1 { Method::Md5 } else { Method::Pwd }), identity: String::from(s.wl_id.trim()), password: s.wl_pw.clone() });
    }
    new.extend(blocks.iter().filter(|b| matches!(b.key_mgmt, KeyMgmt::Other(_))).cloned());
    ui.box_begin(0, BOX_H | BOX_END, 8);
    ui.next_flags(if new != blocks { 0 } else { ST_DISABLED });
    let mut apply = false;
    if ui.accent_button(id + 10, t("Anmeldung übernehmen")) & EV_ACT != 0 {
        apply = true;
    }
    ui.box_end();
    for v in &views {
        let (line, badge) = supp_line(v);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        ui.status_dot(badge);
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &line);
        ui.box_end();
    }
    if !s.conf_err.is_empty() {
        let e = s.conf_err.clone();
        crate::workspace::error_text(ui, &e, 0);
    }
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, &tf("Die Angaben liegen in {} auf dem Gerät; wpa_cli im Terminal zeigt den Zustand.", &[supp::CONF]));
    if apply {
        if new.iter().any(|b| b.identity.is_empty() || (b.key_mgmt == KeyMgmt::WpaEap && b.ssid.is_empty())) {
            s.conf_err = String::from(t("Eine Anmeldung braucht eine Identität, im WLAN auch die SSID."));
        } else {
            s.conf_err.clear();
            let r = n.doc.sim.send(Cmd::Apply(Action::Supplicant { dev, conf: supp::conf_text(&new) }));
            if let (Err(e), Some(w)) = (r, n.devwin.get(k)) {
                w.desk.ip.conf_err = tr::msg(&e.msg());
            }
        }
        ui.repass();
    }
}

// the IP configuration of one interface: DHCP, or a static address and gateway (one undo
// step; switching DHCP off first)
#[allow(clippy::too_many_arguments)]
fn set_ip(n: &mut Net, dev: u32, iface: &str, to_dhcp: bool, dhcp: bool, addr: &str, mask: &str, gw: &str, gw_now: &str) -> Result<(), String> {
    if to_dhcp {
        return if dhcp { Ok(()) } else { config::dhcp(&mut n.doc, dev, iface, true) };
    }
    let prefix = match (addr.is_empty(), Ipv4::parse(addr), Ipv4::parse(mask)) {
        (true, ..) => None,
        (false, Some(a), Some(m)) => Some(Prefix4::from_mask(a, m).ok_or_else(|| tf("„{}“ ist keine Subnetzmaske.", &[mask]))?),
        (false, None, _) => return Err(tf("„{}“ ist keine IPv4-Adresse.", &[addr])),
        (false, _, None) => return Err(tf("„{}“ ist keine Subnetzmaske.", &[mask])),
    };
    let gw_new = if gw.is_empty() { None } else { Some(Ipv4::parse(gw).ok_or_else(|| tf("„{}“ ist keine IPv4-Adresse.", &[gw]))?) };
    let mut lines = alloc::vec![format!("ip addr flush dev {}", iface)];
    if let Some(p) = prefix {
        lines.push(format!("ip addr add {} dev {}", p, iface));
    }
    if gw != gw_now || dhcp {
        if !gw_now.is_empty() {
            lines.push(String::from("ip route del default"));
        }
        if let Some(g) = gw_new {
            lines.push(format!("ip route add default via {}", g));
        }
    }
    let name = n.doc.proj.device(dev).map(|d| d.name.clone()).unwrap_or_default();
    let label = format!("IP-Konfiguration von {} ändern", name);
    n.doc.hist.group_begin(&label);
    let mut r = Ok(());
    if dhcp {
        r = config::dhcp(&mut n.doc, dev, iface, false);
    }
    if r.is_ok() {
        r = console::run_lines(n, dev, &label, &lines);
    }
    n.doc.hist.group_end();
    r
}
