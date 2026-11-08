// devwin -- the device windows (a double click with Shift, the context menu, the inspector):
// a tool window per device with the tabs Übersicht, Schnittstellen, Konfiguration, Firewall
// (IOS-like devices that route), Dienste, CLI and, for hosts, Desktop; Dateien for devices with
// files
//
//   DevWins::new()          none open
//   open(n, ui, dev)        the device's window (one that is open comes to the front; at most
//                           DEVWINS, the oldest gives its place up); a host's opens on its
//                           Desktop, the others on Übersicht
//   frame(n, ui)            the windows (after the main window's content)
//   shows(n, dev)           the window that draws this device's console now (its CLI tab, the
//                           desktop's Terminal or MIB-Browser window): the Terminal panel shows
//                           a note instead
//   open_on(n, ui, dev, tab)   the device's window on one of its tabs
//   get(k) / peek(k)       device window k (for the tabs' modules and the IDE)
//   fullscreen(n, ui)       F11 (CMD_DESK_FULL): the active device window into full screen and
//                           back (ui.window_fullscreen), without one the main window
//   win_id(k)               the window id of device window k
//
// A device's desktop (desktop.rs) lives in its window while it is open and in DevWins::kept
// while it is closed: the session's state, so the desktop comes back as it was (its windows,
// programs and what they show); a new or loaded project starts without any.
//
//   config.rs    Konfiguration: forms that type command lines (doc/console.rs run_lines)
//   firewall.rs  Firewall: access lists, what protects which interface, NAT, which rule a packet
//                meets, as forms that type command lines (ids ID_FW0 + 0x800 k ..)
//   services.rs  Dienste: what runs on the device, switched on and off
//   svcedit.rs   the settings of a project's service as JSON (Dienste › Einstellungen …)
//   svcform.rs   the same as a form for the lab's kinds, and the dialog "Dienst hinzufügen"
//   desktop.rs   Desktop of hosts: the programs in windows of their own, icons, a taskbar with
//                Start, full screen, the clock, Einstellungen (ids ID_DESK0 + 0x1000 k ..)
//   progs.rs     the desktop's IP-Konfiguration and Webbrowser
//   editor.rs    the desktop's Texteditor: the notes (notizen.txt) and the device's files
//   filemgr.rs   the desktop's Dateimanager: folders and files, copy, move, import, export
//   sysmon.rs    the desktop's Systemmonitor: processes (ps), services, CPU and memory
//   scripts.rs   CMD_SCRIPTS ("IDE …": the desktop's IDE window, desktop::open_ide), the IDE's
//                check while typing, helpers of its runtime marks (the IDE: workspace/ide, in
//                the desktop's IDE window; ids ID_IDE0 + 0x1000 k ..)
//   files.rs     Dateien: the device's files (flash:, nvram:, the Linux tree) with their text,
//                delete, import, export (ids ID_FILES0 + 0x180 k ..)
//   email.rs     the E-Mail program of the Desktop: account, inbox, sent, write, answer, send,
//                fetch (ids ID_MAIL0 + 0x400 k ..)
//   mib.rs       the MIB-Browser of the Desktop: the MIB's objects, Get, GetNext, Walk and Set
//                as commands on the console (ids ID_MIB0 + 0x400 k ..)
//   scriptls.rs  the language services of the IDE's editor (apps/jfs/crates/script's script::ls with the
//                simulator's host names and the device's files for imports): completion,
//                hover, signature help, folding, the outline, go to definition, rename, format
//
// Übersicht shows model, ports, power and state; Schnittstellen every interface as the
// simulator runs it (the inspector's rows, with frame counters); CLI the device's console --
// the same TermState as its tab in the Terminal panel, drawn here while the tab is shown.
// Window k has the ids ID_DEVWIN0 + 0x200 k (the window) .. + 0x1FF: + 1 its tabs, + 0x10
// the overview, + 0x20 the configuration, + 0x60 the services, + 0x80 the desktop's
// IP-Konfiguration and Webbrowser (progs.rs; the desktop's own block is ID_DESK0 + 0x1000 k),
// + 0x100 the interface table (rows + 1 + i).  F11 or the desktop's Vollbild put the window
// into full screen (every tab can be shown that way).

pub(crate) mod config;
pub(crate) mod firewall;
pub(crate) mod fwzones;
pub(crate) mod desktop;
pub(crate) mod services;
pub(crate) mod svcedit;
pub(crate) mod svcform;
pub(crate) mod scripts;
pub(crate) mod files;
pub(crate) mod email;
pub(crate) mod mib;
pub(crate) mod scriptls;
pub(crate) mod progs;
pub(crate) mod editor;
pub(crate) mod filemgr;
pub(crate) mod sysmon;
pub(crate) mod browser;
// page scripts of the Webbrowser (B3): the VM's host on a document, the console
pub(crate) mod pagescript;
// the Webbrowser's developer view (B4)
pub(crate) mod devtools;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::catalog::{self, CliStyle, F_HOST, F_ROUTER};
use jfn_model::ifname;
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::console;
use crate::workspace::canvas::cables;
use crate::workspace::icons::{port_icon, JN_TERMINAL};
use crate::workspace::inspector::device::if_rows;

/// the tabs of a device window
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
    Overview,
    Ifaces,
    Config,
    Firewall,
    Services,
    Cli,
    Desktop,
    Files,
}

impl Tab {
    // its title in the current language
    fn title(self) -> &'static str {
        t(match self {
            Tab::Overview => "Übersicht",
            Tab::Ifaces => "Schnittstellen",
            Tab::Config => "Konfiguration",
            Tab::Firewall => "Firewall",
            Tab::Services => "Dienste",
            Tab::Cli => "CLI",
            Tab::Desktop => "Desktop",
            Tab::Files => "Dateien",
        })
    }
}

/// the tabs device `dev` has (CLI with a command line, Firewall on an IOS-like device that
/// routes, Desktop for hosts)
pub(crate) fn tabs_of(n: &Net, dev: u32) -> Vec<Tab> {
    let m = n.doc.proj.model_of(dev);
    let mut v = alloc::vec![Tab::Overview, Tab::Ifaces];
    let cli = m.is_some_and(|m| m.cli != CliStyle::None);
    if cli {
        v.push(Tab::Config);
    }
    if m.is_some_and(|m| m.has(F_ROUTER) && m.cli == CliStyle::Ios) {
        v.push(Tab::Firewall);
    }
    v.push(Tab::Services);
    if cli {
        v.push(Tab::Cli);
    }
    if m.is_some_and(|m| m.has(F_HOST) && m.cli == CliStyle::Linux) {
        v.push(Tab::Desktop);
    }
    if files::has_tab(&n.doc.proj, dev) {
        v.push(Tab::Files);
    }
    v
}

/// one device window
pub(crate) struct DevWin {
    pub dev: u32,
    pub tab: Tab,
    open: Shared<i32>,
    rect: Shared<WinRect>,
    ifs: GridState,
    pub cfg: config::Form,
    /// the Firewall tab's state
    pub fw: firewall::Form,
    pub desk: desktop::Desk,
    /// opened at (the oldest gives its place up)
    age: u32,
    /// the IDE's state in this window (workspace/ide; drawn in the desktop's IDE window)
    pub ide: crate::workspace::ide::Pane,
    /// the Dateien tab's grid
    pub files: GridState,
}

pub(crate) struct DevWins {
    wins: Vec<Option<DevWin>>,
    clock: u32,
    /// the settings editor of a service (svcedit.rs)
    pub svc: svcedit::SvcEdit,
    /// the IDE's drafts and breakpoints, of every window (they outlive a closed window)
    pub ide: crate::workspace::ide::Ides,
    /// the Dateien tab's file dialog
    pub files: files::FilesUi,
    /// the desktops of devices whose window is closed (the session's: back when it opens)
    kept: BTreeMap<u32, desktop::Desk>,
}

impl DevWins {
    pub(crate) fn new() -> DevWins {
        DevWins { wins: (0..DEVWINS).map(|_| None).collect(), clock: 0, svc: svcedit::SvcEdit::new(), ide: crate::workspace::ide::Ides::new(), files: files::FilesUi::new(), kept: BTreeMap::new() }
    }

    // a window goes: its desktop is kept for the device
    fn close(&mut self, k: usize) {
        if let Some(w) = self.wins.get_mut(k).and_then(|w| w.take()) {
            self.kept.insert(w.dev, w.desk);
        }
    }

    /// device window k (for the tabs' modules)
    pub(crate) fn get(&mut self, k: usize) -> Option<&mut DevWin> {
        self.wins.get_mut(k).and_then(|w| w.as_mut())
    }

    /// the desktop of a device, whether its window is open or not (Report::Web finds its
    /// browser here)
    pub(crate) fn desk_of(&mut self, dev: u32) -> Option<&mut desktop::Desk> {
        // (an open window first: a closed one may still hold an older desktop of the device)
        let open = self.wins.iter().position(|w| w.as_ref().is_some_and(|w| w.dev == dev && w.open.get() != 0));
        match open.or_else(|| self.wins.iter().position(|w| w.as_ref().is_some_and(|w| w.dev == dev))) {
            Some(k) => self.wins[k].as_mut().map(|w| &mut w.desk),
            None => self.kept.get_mut(&dev),
        }
    }

    /// device window k to look at
    pub(crate) fn peek(&self, k: usize) -> Option<&DevWin> {
        self.wins.get(k).and_then(|w| w.as_ref())
    }
}

/// the id of device window k
fn base(k: usize) -> u32 {
    ID_DEVWIN0 + 0x200 * k as u32
}

/// the window id of device window k (full screen, activation)
pub(crate) fn win_id(k: usize) -> u32 {
    base(k)
}

/// F11: the active device window into full screen and back; without one the main window
pub(crate) fn fullscreen(n: &mut Net, ui: &mut Ui) {
    let active = (0..DEVWINS).find(|&k| n.devwin.wins[k].is_some() && ui.window_state(base(k)) & UWS_ACTIVE != 0);
    let id = active.map_or(0, base);
    let on = ui.window_state(id) & UWS_FULLSCREEN == 0;
    ui.window_fullscreen(id, on);
}

/// the device's window: an open one to the front, else a new one (the oldest one gives its
/// place up when all are open)
pub(crate) fn open(n: &mut Net, ui: &mut Ui, dev: u32) {
    if n.doc.proj.device(dev).is_none() {
        return;
    }
    // a member of a collapsed group: its groups open first (G1)
    crate::workspace::canvas::groups::reveal(n, dev);
    let w = &mut n.devwin;
    w.clock += 1;
    if let Some(k) = w.wins.iter().position(|x| x.as_ref().is_some_and(|x| x.dev == dev && x.open.get() != 0)) {
        ui.window_activate(base(k));
        return;
    }
    let k = w
        .wins
        .iter()
        .position(|x| x.as_ref().is_none_or(|x| x.open.get() == 0))
        .unwrap_or_else(|| w.wins.iter().enumerate().min_by_key(|(_, x)| x.as_ref().map_or(0, |x| x.age)).map_or(0, |(k, _)| k));
    // the old window's place and size stay; its desktop is kept for its device
    let rect = w.wins[k].as_ref().map_or_else(|| Shared::new(WinRect::new(0, 0, 820, 560, 0)), |x| x.rect.clone());
    w.close(k);
    let open = Shared::new(1);
    // a host's desktop as the session left it
    let desk = w.kept.remove(&dev).unwrap_or_else(|| desktop::Desk::new(dev));
    let tab = if tabs_of(n, dev).contains(&Tab::Desktop) { Tab::Desktop } else { Tab::Overview };
    let w = &mut n.devwin;
    w.wins[k] = Some(DevWin {
        dev,
        tab,
        open,
        rect,
        ifs: GridState::default(),
        cfg: config::Form::new(),
        fw: firewall::Form::new(),
        desk,
        age: w.clock,
        ide: crate::workspace::ide::Pane::new(dev),
        files: GridState::default(),
    });
    ui.window_activate(base(k));
    ui.repass();
}

/// the device's window (open() as it is) on one of its tabs
pub(crate) fn open_on(n: &mut Net, ui: &mut Ui, dev: u32, tab: Tab) {
    open(n, ui, dev);
    let tabs = tabs_of(n, dev);
    if let Some(w) = n.devwin.wins.iter_mut().flatten().find(|w| w.dev == dev && w.open.get() != 0)
        && tabs.contains(&tab)
    {
        w.tab = tab;
    }
}

/// the window that draws device `dev`'s console now
pub(crate) fn shows(n: &Net, dev: u32) -> Option<u32> {
    n.devwin.wins.iter().enumerate().find_map(|(k, x)| {
        let x = x.as_ref()?;
        // the desktop's Terminal and MIB-Browser (mib.rs) windows draw the console too (the
        // IDE's Terminal pane is a session of its own)
        let console = x.tab == Tab::Cli || (x.tab == Tab::Desktop && x.desk.console_at.is_some());
        (x.dev == dev && x.open.get() != 0 && console).then(|| base(k))
    })
}

/// every device window (at the end of the frame)
pub(crate) fn frame(n: &mut Net, ui: &mut Ui) {
    for k in 0..DEVWINS {
        let Some(w) = n.devwin.wins[k].as_ref() else { continue };
        let (dev, open, rect) = (w.dev, w.open.clone(), w.rect.clone());
        // a deleted device closes its window (its desktop goes with it)
        let Some(d) = n.doc.proj.device(dev) else {
            open.set(0);
            n.devwin.wins[k] = None;
            n.devwin.kept.remove(&dev);
            continue;
        };
        let title = format!("{} – {}", d.name, catalog::model(&d.model).map_or(t("Gerät"), |m| t(m.name)));
        let id = base(k);
        if ui.window_begin(id, &title, Some(&open), WF_TOOL | WF_RESIZABLE, Some(&rect)) {
            window(n, ui, k, dev);
        }
        ui.window_end();
        if open.get() == 0 {
            // a closed window leaves full screen (the next one in its place starts windowed)
            if ui.window_state(id) & UWS_FULLSCREEN != 0 {
                ui.window_fullscreen(id, false);
            }
            n.devwin.close(k);
        }
    }
}

// the content of window k: the tabs, the chosen one
fn window(n: &mut Net, ui: &mut Ui, k: usize, dev: u32) {
    let id = base(k);
    let tabs = tabs_of(n, dev);
    let Some(w) = n.devwin.wins[k].as_mut() else { return };
    if !tabs.contains(&w.tab) {
        w.tab = Tab::Overview;
    }
    let mut sel = tabs.iter().position(|&t| t == w.tab).unwrap_or(0) as i32;
    let names: Vec<&str> = tabs.iter().map(|x| x.title()).collect();
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | pad(10), 8);
    if ui.tabs(id + 1, &mut sel, &names) & EV_ACT != 0 {
        w.tab = tabs[sel.clamp(0, tabs.len() as i32 - 1) as usize];
        if w.tab == Tab::Cli {
            n.cons.focus = Some(dev);
        }
        ui.repass();
    }
    let tab = w.tab;
    match tab {
        Tab::Overview => overview(n, ui, id, dev),
        Tab::Ifaces => ifaces(n, ui, k, dev),
        Tab::Config => config::tab(n, ui, k, id + 0x20, dev),
        Tab::Firewall => firewall::tab(n, ui, k, dev),
        Tab::Services => services::tab(n, ui, k, id + 0x60, dev),
        Tab::Cli => {
            ui.next_size(-1, -1);
            ui.box_begin(0, BOX_V, 0);
            console::view(n, ui, dev);
            ui.box_end();
        }
        Tab::Desktop => {
            ui.next_size(-1, -1);
            ui.box_begin(0, BOX_V, 0);
            desktop::tab(n, ui, k, id + 0x80, dev);
            ui.box_end();
        }
        Tab::Files => files::tab(n, ui, k, dev),
    }
    ui.box_end();
}

// "key  value" rows
fn row(ui: &mut Ui, key: &str, value: &str) {
    ui.box_begin(0, BOX_H, 8);
    ui.next_size(150, 0);
    ui.text(TS_BODY | TX_SECONDARY, key);
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_WRAP, value);
    ui.box_end();
}

// Übersicht: the symbol, name and model, power, the state of the ports and interfaces, the
// command line, the services
fn overview(n: &mut Net, ui: &mut Ui, id: u32, dev: u32) {
    let Some(d) = n.doc.proj.device(dev).cloned() else { return };
    let m = catalog::model(&d.model);
    ui.next_size(-1, -1);
    ui.scroll_begin(id + 0x10, SC_V);
    ui.box_begin(0, BOX_V | pad(4), 10);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
    if let Some(draw) = m.and_then(|m| catalog::catalog().iter().position(|c| c.id == m.id)).and_then(|i| n.palette.tile_draw(i)) {
        let r = ui.place(ui.dp(64), ui.dp(48));
        let ver = n.syms.key() ^ 0x5A5A_0002;
        ui.cmd_custom(r[0], r[1], r[2], r[3], Some(draw), 0, ver);
    }
    ui.box_begin(0, BOX_V, 2);
    ui.text(TS_SUBTITLE, &d.name);
    ui.text(TS_BODY | TX_SECONDARY, m.map_or(t("Unbekanntes Modell"), |m| t(m.name)));
    ui.box_end();
    ui.box_end();
    let mut on = (!d.off) as i32;
    if ui.switch(id + 0x11, t("Eingeschaltet"), &mut on) & EV_ACT != 0 {
        cables::set_power(n, &[dev], on != 0);
        ui.repass();
    }
    let rows = if_rows(n, dev);
    let ports = rows.iter().filter(|r| r.key < 0x1_0000).count();
    let up = rows.iter().filter(|r| r.badge == BADGE_OK).count();
    let used = n.doc.proj.ports(dev).iter().filter(|(_, l)| l.is_some()).count();
    let svcs = n.doc.sim.state().services(dev);
    let running = svcs.iter().filter(|s| s.running).count();
    ui.box_begin(0, BOX_V | BOX_CARD | pad(12), 6);
    if let Some(m) = m {
        row(ui, t("Modell"), t(m.name));
        row(ui, t("Beschreibung"), t(m.title));
        row(ui, t("Ports"), &if m.groups.is_empty() { String::from(t("keine")) } else { crate::app::tr::msg(&m.summary_msg()) });
    }
    row(ui, t("Belegt"), &tf("{} von {} Ports", &[&used.to_string(), &ports.to_string()]));
    row(ui, t("Status"), &if d.off { String::from(t("ausgeschaltet")) } else { tf("eingeschaltet, {} von {} Schnittstellen aktiv", &[&up.to_string(), &rows.len().to_string()]) });
    row(ui, t("Befehlszeile"), t(match m.map(|m| m.cli) {
        Some(CliStyle::Ios) => "IOS-artig (Router, Switches)",
        Some(CliStyle::Linux) => "Linux-Shell",
        _ => "keine",
    }));
    row(ui, t("Dienste"), &if svcs.is_empty() { String::from(t("keine")) } else { tf("{} von {} laufen", &[&running.to_string(), &svcs.len().to_string()]) });
    if let Some(e) = n.doc.sim.errors.get(&dev) {
        row(ui, t("Simulator"), &tf("läuft nicht: {}", &[&crate::app::tr::msg(e)]));
    }
    ui.box_end();
    // the first addresses at a glance (IPv6 after IPv4)
    let mut addrs: Vec<String> = rows.iter().filter(|r| r.ip != "–").map(|r| format!("{} {}", ifname::short(&r.name), r.ip)).take(6).collect();
    addrs.extend(rows.iter().filter(|r| r.ip6 != "–").map(|r| format!("{} {}", ifname::short(&r.name), r.ip6)).take(6));
    if !addrs.is_empty() {
        ui.text(TS_BODY_STRONG, t("Adressen"));
        for a in &addrs {
            ui.text(TS_BODY, a);
        }
    }
    if console::has_cli(&n.doc.proj, dev) {
        ui.box_begin(0, BOX_H, 8);
        ui.next_icon(JN_TERMINAL);
        if ui.button(id + 0x12, t("Konsole im Terminal öffnen")) & EV_ACT != 0 {
            let _ = console::open(n, ui, dev);
        }
        ui.box_end();
    }
    ui.box_end();
    ui.scroll_end();
}

// Schnittstellen: every interface with its state, addresses, far end, speed, VLAN, MAC and
// frame counters
fn if_cols(v6: bool) -> Vec<GridCol<'static>> {
    let mut v = alloc::vec![
        GridCol::new(t("Port"), 76).flags(GC_NOSORT),
        GridCol::new(t("Status"), 112).flags(GC_NOSORT),
        GridCol::new(t("IP"), 108).flags(GC_NOSORT),
        GridCol::new(t("Gegenstelle"), 108).flags(GC_NOSORT | GC_FILL),
        GridCol::new(t("Speed"), 72).flags(GC_NOSORT),
        GridCol::new(t("VLAN"), 50).flags(GC_NOSORT),
        GridCol::new(t("MAC"), 116).flags(GC_NOSORT),
        GridCol::new(t("Empfangen"), 86).flags(GC_NOSORT | GC_RIGHT),
        GridCol::new(t("Gesendet"), 86).flags(GC_NOSORT | GC_RIGHT),
    ];
    // the IPv6 addresses after IP, only on a device with IPv6
    if v6 {
        v.insert(3, GridCol::new(t("IPv6"), 180).flags(GC_NOSORT));
    }
    v
}

fn ifaces(n: &mut Net, ui: &mut Ui, k: usize, dev: u32) {
    let rows = if_rows(n, dev);
    let counters: Vec<(u64, u64)> = {
        let v = &n.doc.sim.view;
        let dv = v.devices.iter().find(|d| d.dev == dev);
        rows.iter()
            .map(|r| match dv.and_then(|d| d.ports.iter().find(|p| p.index as u64 + 1 == r.key)) {
                Some(p) => (p.counters.rx_frames, p.counters.tx_frames),
                None => (u64::MAX, u64::MAX),
            })
            .collect()
    };
    let id = base(k) + 0x100;
    let Some(w) = n.devwin.wins[k].as_mut() else { return };
    if rows.is_empty() {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER, 0);
        ui.empty_state(IC_INFO, t("Keine Schnittstellen"), t("Dieses Gerät hat keine Ports oder läuft nicht im Simulator."));
        ui.box_end();
        return;
    }
    let keys: Vec<u64> = rows.iter().map(|r| r.key).collect();
    w.ifs.retain(&keys);
    ui.next_size(-1, -1);
    let v6 = rows.iter().any(|r| r.ip6 != "–");
    let k = v6 as usize;
    let cols = if_cols(v6);
    let (first, end) = ui.grid_begin(id, &mut w.ifs, &cols, &keys);
    let num = |v: u64| if v == u64::MAX { String::from("–") } else { format!("{}", v) };
    for i in first..end {
        let r = &rows[i as usize];
        ui.grid_row(&mut w.ifs, i);
        ui.grid_cell(&w.ifs, 0);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 4);
        if let Some(md) = r.media {
            ui.icon(port_icon(md), 14);
        }
        ui.text(TS_BODY | TX_ELLIPSIS, &ifname::short(&r.name));
        ui.box_end();
        ui.grid_cell(&w.ifs, 1);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
        ui.status_dot(r.badge);
        ui.text(TS_BODY | TX_ELLIPSIS, r.status);
        ui.box_end();
        ui.grid_text(&w.ifs, 2, &r.ip);
        if v6 {
            ui.grid_text(&w.ifs, 3, &r.ip6);
        }
        ui.grid_text(&w.ifs, 3 + k, &r.peer);
        ui.grid_text(&w.ifs, 4 + k, &r.speed);
        ui.grid_text(&w.ifs, 5 + k, &r.vlan);
        ui.grid_text_ts(&w.ifs, 6 + k, TS_CAPTION, &r.mac);
        ui.grid_text(&w.ifs, 7 + k, &num(counters[i as usize].0));
        ui.grid_text(&w.ifs, 8 + k, &num(counters[i as usize].1));
    }
    ui.grid_end(&mut w.ifs);
}
