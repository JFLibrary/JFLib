// config -- the Konfiguration tab of a device window: forms for the common settings that type
// command lines on the device's console (doc/console.rs run_lines: they show there, and land
// in the startup configuration as one undo step)
//
//   Form::new()                  empty fields
//   tab(n, ui, k, id, dev)       Allgemein (host name), Schnittstelle (address, description,
//                                up, DHCP, the VLAN of a switch port), Routing (default
//                                gateway, static routes), Sichern (the running configuration
//                                becomes the startup one); below, the lines that ran last
//
// IOS-like devices get configuration commands ("interface Gi0/0/0", "ip address A M",
// "no shutdown", "ip route 0.0.0.0 0.0.0.0 G"), Linux shells iproute2 commands ("ip addr
// flush dev eth0", "ip addr add A/P dev eth0", "ip route add default via G").  A field the
// user did not change makes no line; a refusal (the device's own words) shows below the
// form.  DHCP has no command line in the simulator yet: it is a service (doc/config.rs, as in
// the inspector).  The fields start over when their values change elsewhere.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::catalog::{CliStyle, Media};
use jfn_model::ifname;
use jfn_sim::PortMode;
use netpkt::{Ipv4, Prefix4};
use ui::*;

use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::{config, console};
use crate::workspace::inspector::device::{if_rows, IfRow};

/// the fields of the Konfiguration tab
pub(crate) struct Form {
    /// the device the fields belong to
    dev: u32,
    host: String,
    host_src: String,
    /// the chosen interface (an index into the rows), the values the fields started from
    iface: i32,
    addr: String,
    descr: String,
    up: i32,
    vlan: String,
    src: (String, String, String, i32, String),
    gw: String,
    gw_src: String,
    net: String,
    via: String,
    /// why the last form was refused
    err: String,
    /// the lines the last form typed
    ran: Vec<String>,
}

impl Form {
    pub(crate) fn new() -> Form {
        Form {
            dev: 0,
            host: String::new(),
            host_src: String::new(),
            iface: 0,
            addr: String::new(),
            descr: String::new(),
            up: 1,
            vlan: String::new(),
            src: (String::new(), String::new(), String::new(), -1, String::new()),
            gw: String::new(),
            gw_src: String::new(),
            net: String::new(),
            via: String::new(),
            err: String::new(),
            ran: Vec::new(),
        }
    }
}

/// what an interface row offers: an address (IPv4), a VLAN (a switch port)
struct IfInfo {
    name: String,
    ip: bool,
    addr: String,
    descr: String,
    up: bool,
    /// the access VLAN of a switch port (None: no switch port, or a trunk)
    vlan: Option<u16>,
    trunk: bool,
    dhcp: bool,
}

// the interfaces the form can set (no console or USB ports)
fn interfaces(n: &Net, dev: u32) -> Vec<IfInfo> {
    let e = n.doc.sim.engine();
    let descr = e.node(dev).map(|x| x.cli.cfg.descr.clone()).unwrap_or_default();
    let bridge = e.bridge(dev);
    let d = n.doc.proj.device(dev);
    if_rows(n, dev)
        .into_iter()
        .filter(|r: &IfRow| !r.media.is_some_and(|m| matches!(m, Media::Console | Media::Usb)))
        .map(|r| {
            let port = if r.key < 0x1_0000 { Some((r.key - 1) as usize) } else { None };
            let mode = port.and_then(|p| bridge.and_then(|b| b.ports.get(p))).filter(|bp| bp.member && r.iface.is_none()).map(|bp| bp.mode.clone());
            IfInfo {
                addr: r.iface.as_ref().and_then(|f| f.addrs.first()).map(|a| a.to_string()).unwrap_or_default(),
                ip: r.iface.is_some(),
                descr: descr.get(&r.name).cloned().unwrap_or_default(),
                up: r.admin,
                vlan: match &mode {
                    Some(PortMode::Access(v)) => Some(*v),
                    _ => None,
                },
                trunk: matches!(mode, Some(PortMode::Trunk { .. })),
                dhcp: d.is_some_and(|d| config::dhcp_service(d, &r.name).is_some()),
                name: r.name,
            }
        })
        .collect()
}

// a section's heading
fn heading(ui: &mut Ui, s: &'static str) {
    ui.text(TS_BODY_STRONG, t(s));
}

// a "label  field" row (the field follows, then box_end)
fn field(ui: &mut Ui, label: &'static str) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(170, 0);
    ui.text(TS_BODY | TX_SECONDARY, if label.is_empty() { label } else { t(label) });
}

/// the Konfiguration tab of device window k (ids id .. id + 0x3F)
pub(crate) fn tab(n: &mut Net, ui: &mut Ui, k: usize, id: u32, dev: u32) {
    let Some(d) = n.doc.proj.device(dev).cloned() else { return };
    let ios = n.doc.proj.model_of(dev).is_some_and(|m| m.cli == CliStyle::Ios);
    let ifs = interfaces(n, dev);
    let gws: Vec<Ipv4> = config::default_gateways(n.doc.sim.engine(), dev);
    let statics: Vec<(Prefix4, Option<Ipv4>)> = n.doc.sim.engine().node(dev).and_then(|x| x.ip.as_ref()).map(|st| st.statics.iter().filter(|r| r.prefix.len > 0).map(|r| (r.prefix, r.via)).collect()).unwrap_or_default();
    let has_ip = n.doc.sim.engine().node(dev).is_some_and(|x| x.ip.is_some());
    let Some(w) = n.devwin.get(k) else { return };
    let f = &mut w.cfg;
    if f.dev != dev {
        *f = Form::new();
        f.dev = dev;
    }
    // fields start over when their values change elsewhere
    if f.host_src != d.name {
        f.host = d.name.clone();
        f.host_src = d.name.clone();
    }
    f.iface = f.iface.clamp(0, (ifs.len() as i32 - 1).max(0));
    if let Some(x) = ifs.get(f.iface as usize) {
        let src = (x.name.clone(), x.addr.clone(), x.descr.clone(), x.up as i32, x.vlan.map(|v| v.to_string()).unwrap_or_default());
        if src != f.src {
            f.addr = src.1.clone();
            f.descr = src.2.clone();
            f.up = src.3;
            f.vlan = src.4.clone();
            f.src = src;
        }
    }
    let gw_now = gws.first().map(|g| g.to_string()).unwrap_or_default();
    if f.gw_src != gw_now {
        f.gw = gw_now.clone();
        f.gw_src = gw_now.clone();
    }
    // what the buttons ask for
    let mut run: Option<(String, Vec<String>)> = None;
    let mut dhcp: Option<(String, bool)> = None;
    let mut save = false;
    ui.next_size(-1, -1);
    ui.scroll_begin(id, SC_V);
    ui.box_begin(0, BOX_V | pad(4), 10);
    // Allgemein: the host name
    heading(ui, "Allgemein");
    field(ui, "Gerätename");
    ui.next_size(220, 0);
    let ev = ui.textbox(id + 1, Some(t("Name")), &mut f.host, 64);
    let mut set = ev & EV_COMMIT != 0 && ui.focused() == id + 1 && f.host != d.name;
    ui.next_flags(if f.host.trim() == d.name { ST_DISABLED } else { 0 });
    if ui.button(id + 2, t("Übernehmen")) & EV_ACT != 0 {
        set = true;
    }
    ui.box_end();
    if set {
        let name = String::from(f.host.trim());
        if !jfn_model::project::valid_name(&name) {
            f.err = tf("„{}“ ist kein gültiger Gerätename: Buchstaben, Ziffern und Bindestriche, vorn ein Buchstabe.", &[&name]);
        } else if n_name_taken(&n.doc.proj, dev, &name) {
            f.err = tf("Der Name „{}“ ist schon vergeben.", &[&name]);
        } else {
            run = Some((format!("{} umbenennen", d.name), alloc::vec![format!("hostname {}", name)]));
        }
    }
    // Schnittstelle
    if !ifs.is_empty() {
        heading(ui, "Schnittstelle");
        let names: Vec<String> = ifs.iter().map(|x| ifname::short(&x.name)).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        field(ui, "Schnittstelle");
        ui.next_size(220, 0);
        if ui.combobox(id + 3, &mut f.iface, &refs) & EV_ACT != 0 {
            f.err.clear();
            ui.repass();
        }
        ui.box_end();
        let x = &ifs[f.iface.clamp(0, ifs.len() as i32 - 1) as usize];
        let mut lines: Vec<String> = Vec::new();
        if x.ip {
            field(ui, "IPv4-Adresse mit Präfix");
            ui.next_size(220, 0);
            ui.next_flags(if x.dhcp { ST_DISABLED } else { 0 });
            ui.textbox(id + 4, Some(t("z. B. 192.168.1.1/24")), &mut f.addr, 40);
            let mut on = x.dhcp as i32;
            if ui.checkbox(id + 8, t("per DHCP"), &mut on) & EV_ACT != 0 {
                dhcp = Some((x.name.clone(), on != 0));
            }
            ui.box_end();
        }
        if x.vlan.is_some() {
            field(ui, "VLAN (Access-Port)");
            ui.next_size(100, 0);
            ui.textbox(id + 7, Some("1"), &mut f.vlan, 6);
            ui.box_end();
        } else if x.trunk {
            field(ui, "VLAN");
            ui.text(TS_BODY, t("Trunk (in der Konsole einstellen)"));
            ui.box_end();
        }
        if ios {
            field(ui, "Beschreibung");
            ui.next_size(-1, 0);
            ui.textbox(id + 5, Some(t("z. B. zum Kern-Switch")), &mut f.descr, 240);
            ui.box_end();
        }
        field(ui, "");
        ui.switch(id + 6, t("Aktiv (nicht abgeschaltet)"), &mut f.up);
        ui.box_end();
        // the lines the changed fields make
        let addr_changed = x.ip && !x.dhcp && f.addr.trim() != x.addr;
        let mut bad = String::new();
        let new_addr = if addr_changed && !f.addr.trim().is_empty() {
            match config::parse_addr(&f.addr) {
                Some(p) => Some(p),
                None => {
                    bad = tf("„{}“ ist keine Adresse mit Präfix (10.0.0.1/24 oder 10.0.0.1 255.255.255.0).", &[f.addr.trim()]);
                    None
                }
            }
        } else {
            None
        };
        let vlan_changed = x.vlan.is_some() && f.vlan.trim() != x.vlan.map(|v| v.to_string()).unwrap_or_default();
        let new_vlan = if vlan_changed {
            match f.vlan.trim().parse::<u16>() {
                Ok(v) if (1..=4094).contains(&v) => Some(v),
                _ => {
                    bad = tf("„{}“ ist keine VLAN-Nummer (1 bis 4094).", &[f.vlan.trim()]);
                    None
                }
            }
        } else {
            None
        };
        if ios {
            if f.descr.trim() != x.descr {
                lines.push(if f.descr.trim().is_empty() { String::from("no description") } else { format!("description {}", f.descr.trim()) });
            }
            if addr_changed {
                lines.push(match new_addr {
                    Some(p) => format!("ip address {} {}", p.addr, p.mask()),
                    None => String::from("no ip address"),
                });
            }
            if let Some(v) = new_vlan {
                lines.push(String::from("switchport mode access"));
                lines.push(format!("switchport access vlan {}", v));
            }
            if (f.up != 0) != x.up {
                lines.push(String::from(if f.up != 0 { "no shutdown" } else { "shutdown" }));
            }
            if !lines.is_empty() {
                lines.insert(0, format!("interface {}", x.name));
            }
        } else {
            if addr_changed {
                lines.push(format!("ip addr flush dev {}", x.name));
                if let Some(p) = new_addr {
                    lines.push(format!("ip addr add {} dev {}", p, x.name));
                }
            }
            if (f.up != 0) != x.up {
                lines.push(format!("ip link set dev {} {}", x.name, if f.up != 0 { "up" } else { "down" }));
            }
        }
        ui.box_begin(0, BOX_H | BOX_END, 8);
        ui.next_flags(if lines.is_empty() && bad.is_empty() { ST_DISABLED } else { 0 });
        if ui.accent_button(id + 9, t("Übernehmen")) & EV_ACT != 0 {
            if !bad.is_empty() {
                f.err = bad;
            } else {
                run = Some((format!("{} {} einstellen", d.name, ifname::short(&x.name)), lines));
            }
        }
        ui.box_end();
    }
    // Routing
    if has_ip {
        heading(ui, "Routing");
        field(ui, if ios { "Standardroute über" } else { "Standard-Gateway" });
        ui.next_size(220, 0);
        let ev = ui.textbox(id + 0xA, Some(t("z. B. 192.168.1.254")), &mut f.gw, 40);
        let mut set = ev & EV_COMMIT != 0 && ui.focused() == id + 0xA && f.gw.trim() != gw_now;
        ui.next_flags(if f.gw.trim() == gw_now { ST_DISABLED } else { 0 });
        if ui.button(id + 0xB, t("Setzen")) & EV_ACT != 0 {
            set = true;
        }
        ui.box_end();
        if set {
            let gw_text = String::from(f.gw.trim());
            match if gw_text.is_empty() { Some(None) } else { Ipv4::parse(&gw_text).map(Some) } {
                None => f.err = tf("„{}“ ist keine IPv4-Adresse.", &[&gw_text]),
                Some(new) => {
                    let mut lines = Vec::new();
                    if ios {
                        for g in &gws {
                            lines.push(format!("no ip route 0.0.0.0 0.0.0.0 {}", g));
                        }
                        if let Some(g) = new {
                            lines.push(format!("ip route 0.0.0.0 0.0.0.0 {}", g));
                        }
                    } else {
                        if !gws.is_empty() {
                            lines.push(String::from("ip route del default"));
                        }
                        if let Some(g) = new {
                            lines.push(format!("ip route add default via {}", g));
                        }
                    }
                    run = Some((format!("Gateway von {} ändern", d.name), lines));
                }
            }
        }
        // static routes: the configured ones, one more
        for (i, (p, via)) in statics.iter().enumerate().take(16) {
            ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
            ui.next_size(170, 0);
            ui.text(TS_BODY | TX_SECONDARY, if i == 0 { t("Statische Routen") } else { "" });
            let via_s = via.map_or_else(|| String::from(t("direkt")), |v| tf("über {}", &[&v.to_string()]));
            ui.next_size(-1, 0);
            ui.text(TS_BODY, &format!("{} {}", p.trunc(), via_s));
            ui.next_flags(ST_SUBTLE | ST_COMPACT | ST_ICON_ONLY);
            ui.next_icon(IC_DELETE);
            if ui.button(id + 0x10 + i as u32, &tf("Route {} entfernen", &[&p.trunc().to_string()])) & EV_ACT != 0 {
                let line = if ios {
                    let v = via.map_or_else(String::new, |v| format!(" {}", v));
                    format!("no ip route {} {}{}", p.network(), p.mask(), v)
                } else {
                    let v = via.map_or_else(String::new, |v| format!(" via {}", v));
                    format!("ip route del {}{}", p.trunc(), v)
                };
                run = Some((format!("Route von {} entfernen", d.name), alloc::vec![line]));
            }
            ui.box_end();
        }
        field(ui, "Neue Route");
        ui.next_size(150, 0);
        ui.textbox(id + 0xC, Some(t("Netz, z. B. 10.2.0.0/16")), &mut f.net, 40);
        ui.text(TS_BODY | TX_SECONDARY, t("über"));
        ui.next_size(140, 0);
        ui.textbox(id + 0xD, Some(t("Next Hop")), &mut f.via, 40);
        ui.next_flags(if f.net.trim().is_empty() || f.via.trim().is_empty() { ST_DISABLED } else { 0 });
        if ui.button(id + 0xE, t("Hinzufügen")) & EV_ACT != 0 {
            match (Prefix4::parse(f.net.trim()).filter(|p| p.len > 0), Ipv4::parse(f.via.trim())) {
                (Some(p), Some(v)) => {
                    let line = if ios { format!("ip route {} {} {}", p.network(), p.mask(), v) } else { format!("ip route add {} via {}", p.trunc(), v) };
                    run = Some((format!("Route von {} hinzufügen", d.name), alloc::vec![line]));
                }
                (None, _) => f.err = tf("„{}“ ist kein Netz mit Präfix (z. B. 10.2.0.0/16).", &[f.net.trim()]),
                (_, None) => f.err = tf("„{}“ ist keine IPv4-Adresse.", &[f.via.trim()]),
            }
        }
        ui.box_end();
    }
    // Sichern
    heading(ui, "Startkonfiguration");
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_icon(IC_SAVE);
    if ui.button(id + 0xF, t("Laufende Konfiguration sichern")) & EV_ACT != 0 {
        save = true;
    }
    ui.next_size(-1, 0);
    ui.text(
        TS_CAPTION | TX_SECONDARY | TX_WRAP,
        t(if ios { "Wie „copy running-config startup-config“ in der Konsole: was dort von Hand eingestellt wurde, bleibt beim nächsten Start." } else { "Die laufende Konfiguration (Adressen, Routen, Name) wird zur Startkonfiguration; eine Shell hat dafür keinen Befehl." }),
    );
    ui.box_end();
    if !f.err.is_empty() {
        let err = f.err.clone();
        crate::workspace::error_text(ui, &err, 0);
    } else {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Die Formulare tippen ihre Befehle in die Konsole des Geräts; sie gelten sofort und werden Teil der Startkonfiguration."));
    }
    if !f.ran.is_empty() {
        ui.box_begin(0, BOX_V | BOX_CARD | pad(10), 2);
        ui.text(TS_CAPTION | TX_SECONDARY, t("Zuletzt in der Konsole:"));
        for l in &f.ran {
            ui.text(TS_CAPTION, l);
        }
        ui.box_end();
    }
    ui.box_end();
    ui.scroll_end();
    // the work, after the widgets
    let mut result: Option<Result<Vec<String>, String>> = None;
    if let Some((label, lines)) = run {
        result = Some(console::run_lines(n, dev, &label, &lines).map(|_| lines));
    }
    if let Some((iface, on)) = dhcp {
        result = Some(config::dhcp(&mut n.doc, dev, &iface, on).map(|_| Vec::new()));
    }
    if save {
        result = Some(console::save_running(n, dev).map(|_| Vec::new()));
    }
    if let Some(r) = result {
        if let Some(w) = n.devwin.get(k) {
            match r {
                Ok(lines) => {
                    w.cfg.err.clear();
                    if !lines.is_empty() {
                        w.cfg.ran = lines;
                    }
                }
                Err(e) => w.cfg.err = e,
            }
        }
        ui.repass();
    }
}

// another device has this name already
fn n_name_taken(p: &jfn_model::Project, dev: u32, name: &str) -> bool {
    p.devices.iter().any(|x| x.id != dev && x.name.eq_ignore_ascii_case(name))
}
