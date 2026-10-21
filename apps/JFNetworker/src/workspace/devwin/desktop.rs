// desktop -- the Desktop tab of a host's device window: small programs as on a PC
//
//   Desk::new()                 the desktop with no program open
//   tab(n, ui, k, id, dev)      the program tiles, or the open program with the way back:
//     IP-Konfiguration      a static address (address, mask, gateway) or DHCP per interface;
//                           it types shell commands on the console as the forms do (DHCP:
//                           the DHCP client service, doc/config.rs)
//     Eingabeaufforderung   the device's console (the same one as its Terminal tab)
//     Texteditor            notizen.txt: the device's notes, saved with the project (the
//                           simulator has no file system yet)
//     Webbrowser            an address line and the page as text; until the simulator speaks
//                           HTTP it says what it would load
//
// Ids: id + 1 .. + 4 the tiles, + 5 back, + 0x10 the IP configuration, + 0x20 the editor,
// + 0x30 the browser.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use netpkt::{Ipv4, Prefix4};
use ui::*;

use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::{config, console, edits};
use crate::workspace::icons::JN_TERMINAL;

/// the programs of the desktop
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum App {
    Home,
    IpConfig,
    Prompt,
    Editor,
    Browser,
}

// i18n
const APPS: [(App, &str, &str); 4] = [
    (App::IpConfig, "IP-Konfiguration", "Adresse, Maske und Gateway oder DHCP"),
    (App::Prompt, "Eingabeaufforderung", "Die Shell des Geräts"),
    (App::Editor, "Texteditor", "Notizen zum Gerät"),
    (App::Browser, "Webbrowser", "Seiten als Text"),
];

/// the desktop's state
pub(crate) struct Desk {
    pub app: App,
    /// the device the fields belong to
    dev: u32,
    // IP-Konfiguration: the interface, static (0) or DHCP (1), the fields and what they
    // started from
    iface: i32,
    mode: i32,
    addr: String,
    mask: String,
    gw: String,
    src: (String, String, String, String, i32),
    err: String,
    // Texteditor
    text: String,
    text_src: Option<String>,
    // Webbrowser
    url: String,
    page: Vec<String>,
}

impl Desk {
    pub(crate) fn new() -> Desk {
        Desk {
            app: App::Home,
            dev: 0,
            iface: 0,
            mode: 0,
            addr: String::new(),
            mask: String::new(),
            gw: String::new(),
            src: (String::new(), String::new(), String::new(), String::new(), -1),
            err: String::new(),
            text: String::new(),
            text_src: None,
            url: String::from("http://"),
            page: Vec::new(),
        }
    }
}

/// the Desktop tab of device window k (ids id .. id + 0x7F)
pub(crate) fn tab(n: &mut Net, ui: &mut Ui, k: usize, id: u32, dev: u32) {
    let Some(w) = n.devwin.get(k) else { return };
    if w.desk.dev != dev {
        w.desk = Desk::new();
        w.desk.dev = dev;
    }
    let app = w.desk.app;
    if app == App::Home {
        home(n, ui, k, id);
        return;
    }
    // the program's head: back to the desktop, its name
    let title = APPS.iter().find(|a| a.0 == app).map_or("", |a| t(a.1));
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_icon(IC_BACK);
    ui.next_flags(ST_SUBTLE | ST_COMPACT);
    if ui.button(id + 5, t("Desktop")) & EV_ACT != 0 {
        if let Some(w) = n.devwin.get(k) {
            w.desk.app = App::Home;
        }
        ui.repass();
    }
    ui.text(TS_BODY_STRONG, title);
    ui.box_end();
    match app {
        App::IpConfig => ipconfig(n, ui, k, id + 0x10, dev),
        App::Prompt => {
            ui.next_size(-1, -1);
            ui.box_begin(0, BOX_V, 0);
            console::view(n, ui, dev);
            ui.box_end();
        }
        App::Editor => editor(n, ui, k, id + 0x20, dev),
        App::Browser => browser(n, ui, k, id + 0x30, dev),
        App::Home => {}
    }
}

// the tiles
fn home(n: &mut Net, ui: &mut Ui, k: usize, id: u32) {
    let mut chosen = None;
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_H | BOX_WRAP | pad(8), 12);
    for (i, (app, label, tip)) in APPS.iter().enumerate() {
        let icon = match app {
            App::IpConfig => IC_SETTINGS,
            App::Prompt => JN_TERMINAL,
            App::Editor => IC_DOCUMENT,
            _ => IC_GLOBE,
        };
        ui.next_size(150, 96);
        ui.next_icon(icon);
        ui.next_flags(ST_LARGE);
        if ui.button(id + 1 + i as u32, t(label)) & EV_ACT != 0 {
            chosen = Some(*app);
        }
        ui.tooltip(t(tip));
    }
    ui.box_end();
    if let Some(a) = chosen
        && let Some(w) = n.devwin.get(k)
    {
        w.desk.app = a;
        if a == App::Prompt {
            n.cons.focus = Some(w.dev);
        }
        ui.repass();
    }
}

// a "label  field" row (the field follows, then box_end)
fn field(ui: &mut Ui, label: &'static str) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(150, 0);
    ui.text(TS_BODY | TX_SECONDARY, t(label));
}

// IP-Konfiguration
fn ipconfig(n: &mut Net, ui: &mut Ui, k: usize, id: u32, dev: u32) {
    let Some(d) = n.doc.proj.device(dev).cloned() else { return };
    let ifs: Vec<(String, Option<Prefix4>)> = n.doc.sim.engine().ifaces(dev).into_iter().filter(|f| f.name != "lo").map(|f| (f.name.clone(), f.addrs.first().copied())).collect();
    let gw_now = config::default_gateways(n.doc.sim.engine(), dev).first().map(|g| g.to_string()).unwrap_or_default();
    let Some(w) = n.devwin.get(k) else { return };
    let s = &mut w.desk;
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
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Statische Einstellungen laufen als Befehle in der Eingabeaufforderung und werden Teil der Startkonfiguration; DHCP holt Adresse und Gateway vom Server."));
    }
    ui.box_end();
    ui.scroll_end();
    if !apply {
        return;
    }
    let (mode, addr, mask, gw) = (s.mode, String::from(s.addr.trim()), String::from(s.mask.trim()), String::from(s.gw.trim()));
    let r = set_ip(n, dev, &name, mode == 1, dhcp, &addr, &mask, &gw, &gw_now);
    if let Some(w) = n.devwin.get(k) {
        w.desk.err = r.err().unwrap_or_default();
    }
    ui.repass();
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

// Texteditor: the device's notes (notizen.txt), saved with the project
fn editor(n: &mut Net, ui: &mut Ui, k: usize, id: u32, dev: u32) {
    let Some(now) = n.doc.proj.device(dev).map(|d| d.comment.clone()) else { return };
    let Some(w) = n.devwin.get(k) else { return };
    let s = &mut w.desk;
    if s.text_src.as_deref() != Some(now.as_str()) {
        s.text = now.clone();
        s.text_src = Some(now.clone());
    }
    let mut save = false;
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.icon(IC_DOCUMENT, 16);
    ui.text(TS_BODY, "notizen.txt");
    ui.text(TS_CAPTION | TX_SECONDARY, t(if s.text != now { "geändert" } else { "gesichert" }));
    ui.spacer(-1);
    ui.next_flags(if s.text == now { ST_DISABLED } else { 0 });
    if ui.button(id + 2, t("Verwerfen")) & EV_ACT != 0 {
        s.text = now.clone();
        ui.repass();
    }
    ui.next_flags(if s.text == now { ST_DISABLED } else { ST_ACCENT });
    ui.next_icon(IC_SAVE);
    if ui.button(id + 1, t("Speichern")) & EV_ACT != 0 {
        save = true;
    }
    ui.box_end();
    ui.next_size(-1, -1);
    ui.textbox(id, Some(t("Notizen zu diesem Gerät: Aufgaben, Zugangsdaten des Labors, was noch fehlt …")), &mut s.text, 64_000 | TB_MULTI | TB_WRAP);
    if save {
        let text = s.text.clone();
        let name = n.doc.proj.device(dev).map(|d| d.name.clone()).unwrap_or_default();
        if let Some(e) = edits::device_edit(&n.doc.proj, dev, &format!("Notizen von {} speichern", name), |x| x.comment = text) {
            n.doc.apply(e);
        }
        ui.repass();
    }
}

// Webbrowser: an address and where it leads (the page itself: curl in the Eingabeaufforderung,
// the simulator speaks HTTP there)
fn browser(n: &mut Net, ui: &mut Ui, k: usize, id: u32, dev: u32) {
    let hosts = n.doc.sim.engine().hosts(dev);
    let Some(w) = n.devwin.get(k) else { return };
    let s = &mut w.desk;
    let mut go = false;
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(-1, 0);
    let ev = ui.textbox(id, Some("http://10.0.0.10/"), &mut s.url, 400);
    if ev & EV_COMMIT != 0 && ui.focused() == id {
        go = true;
    }
    ui.next_icon(IC_REFRESH);
    if ui.button(id + 1, t("Öffnen")) & EV_ACT != 0 {
        go = true;
    }
    ui.box_end();
    if go {
        s.page = page(&s.url, &hosts);
        ui.repass();
    }
    ui.next_size(-1, -1);
    ui.box_begin(id + 2, BOX_V | BOX_CARD | pad(12), 4);
    if s.page.is_empty() {
        ui.text(TS_BODY | TX_SECONDARY, t("Eine Adresse eingeben und „Öffnen“."));
    }
    for l in &s.page {
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_WRAP, l);
    }
    ui.box_end();
}

// the page for an address: where it would go, and how to fetch it (curl)
fn page(url: &str, hosts: &[(String, Ipv4)]) -> Vec<String> {
    let u = url.trim();
    let rest = u.strip_prefix("http://").unwrap_or(u);
    if u.starts_with("https://") {
        return alloc::vec![String::from(t("Verschlüsselte Seiten (https) kann dieser Browser nicht öffnen."))];
    }
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) => match p.parse::<u16>() {
            Ok(p) => (h, p),
            Err(_) => return alloc::vec![tf("„{}“ ist keine Portnummer.", &[p])],
        },
        None => (hostport, 80),
    };
    if host.is_empty() {
        return alloc::vec![String::from(t("Die Adresse nennt keinen Rechner."))];
    }
    let ip = Ipv4::parse(host).or_else(|| hosts.iter().find(|h| h.0.eq_ignore_ascii_case(host)).map(|h| h.1));
    let target = match ip {
        Some(a) if a.to_string() == host => tf("{}, Port {}, Pfad {}", &[&a.to_string(), &port.to_string(), path]),
        Some(a) => tf("{} ({}), Port {}, Pfad {}", &[host, &a.to_string(), &port.to_string(), path]),
        None => tf("{} (der Name ist auf diesem Gerät nicht eingetragen), Port {}, Pfad {}", &[host, &port.to_string(), path]),
    };
    alloc::vec![
        String::from(t("Dieser Browser zeigt nur, wohin die Adresse führt.")),
        tf("Ziel: {}", &[&target]),
        tf("Die Seite selbst holt „curl {}“ in der Eingabeaufforderung über den Simulator.", &[u]),
    ]
}
