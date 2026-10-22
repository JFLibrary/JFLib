// devwin -- the device windows (a double click with Shift, the context menu, the inspector):
// a tool window per device with the tabs Übersicht, Schnittstellen, Konfiguration, Dienste,
// CLI and, for hosts, Desktop
//
//   DevWins::new()          none open
//   open(n, ui, dev)        the device's window (one that is open comes to the front; at most
//                           DEVWINS, the oldest gives its place up)
//   frame(n, ui)            the windows (after the main window's content)
//   shows(n, dev)           the window that draws this device's console now (its CLI tab, the
//                           desktop's prompt): the Terminal panel shows a note instead
//   open_on(n, ui, dev, tab)   the device's window on one of its tabs (CMD_SCRIPTS: Skripte)
//
//   config.rs    Konfiguration: forms that type command lines (doc/console.rs run_lines)
//   services.rs  Dienste: what runs on the device, switched on and off
//   svcedit.rs   the settings of a project's service as JSON (Dienste › Einstellungen …)
//   desktop.rs   Desktop of hosts: IP-Konfiguration, Eingabeaufforderung, Texteditor, Browser
//   scripts.rs   Skripte of hosts: the device's scripts, a code editor with diagnostics, run,
//                stop, apply, their output and errors (ids ID_SCRIPTS0 + 0x400 k ..)
//
// Übersicht shows model, ports, power and state; Schnittstellen every interface as the
// simulator runs it (the inspector's rows, with frame counters); CLI the device's console --
// the same TermState as its tab in the Terminal panel, drawn here while the tab is shown.
// Window k has the ids ID_DEVWIN0 + 0x200 k (the window) .. + 0x1FF: + 1 its tabs, + 0x10
// the overview, + 0x20 the configuration, + 0x60 the services, + 0x80 the desktop, + 0x100
// the interface table (rows + 1 + i).

pub(crate) mod config;
pub(crate) mod desktop;
pub(crate) mod services;
pub(crate) mod svcedit;
pub(crate) mod scripts;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::catalog::{self, CliStyle, F_HOST};
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
    Services,
    Cli,
    Desktop,
    Scripts,
}

impl Tab {
    // its title in the current language
    fn title(self) -> &'static str {
        t(match self {
            Tab::Overview => "Übersicht",
            Tab::Ifaces => "Schnittstellen",
            Tab::Config => "Konfiguration",
            Tab::Services => "Dienste",
            Tab::Cli => "CLI",
            Tab::Desktop => "Desktop",
            Tab::Scripts => "Skripte",
        })
    }
}

/// the tabs device `dev` has (CLI with a command line, Desktop for hosts)
fn tabs_of(n: &Net, dev: u32) -> Vec<Tab> {
    let m = n.doc.proj.model_of(dev);
    let mut v = alloc::vec![Tab::Overview, Tab::Ifaces];
    let cli = m.is_some_and(|m| m.cli != CliStyle::None);
    if cli {
        v.push(Tab::Config);
    }
    v.push(Tab::Services);
    if cli {
        v.push(Tab::Cli);
    }
    if m.is_some_and(|m| m.has(F_HOST) && m.cli == CliStyle::Linux) {
        v.push(Tab::Desktop);
    }
    if scripts::has_tab(&n.doc.proj, dev) {
        v.push(Tab::Scripts);
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
    pub desk: desktop::Desk,
    /// opened at (the oldest gives its place up)
    age: u32,
    /// the Skripte tab
    pub scr: scripts::Pane,
}

pub(crate) struct DevWins {
    wins: Vec<Option<DevWin>>,
    clock: u32,
    /// the settings editor of a service (svcedit.rs)
    pub svc: svcedit::SvcEdit,
    /// the scripts being edited, of every window (they outlive a closed window)
    pub drafts: scripts::Drafts,
}

impl DevWins {
    pub(crate) fn new() -> DevWins {
        DevWins { wins: (0..DEVWINS).map(|_| None).collect(), clock: 0, svc: svcedit::SvcEdit::new(), drafts: scripts::Drafts::new() }
    }

    /// device window k (for the tabs' modules)
    pub(crate) fn get(&mut self, k: usize) -> Option<&mut DevWin> {
        self.wins.get_mut(k).and_then(|w| w.as_mut())
    }
}

/// the id of device window k
fn base(k: usize) -> u32 {
    ID_DEVWIN0 + 0x200 * k as u32
}

/// the device's window: an open one to the front, else a new one (the oldest one gives its
/// place up when all are open)
pub(crate) fn open(n: &mut Net, ui: &mut Ui, dev: u32) {
    if n.doc.proj.device(dev).is_none() {
        return;
    }
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
    // the old window's place and size stay
    let rect = w.wins[k].as_ref().map_or_else(|| Shared::new(WinRect::new(0, 0, 820, 560, 0)), |x| x.rect.clone());
    let open = Shared::new(1);
    w.wins[k] = Some(DevWin {
        dev,
        tab: Tab::Overview,
        open,
        rect,
        ifs: GridState::default(),
        cfg: config::Form::new(),
        desk: desktop::Desk::new(),
        age: w.clock,
        scr: scripts::Pane::new(dev),
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
        let console = x.tab == Tab::Cli || (x.tab == Tab::Desktop && x.desk.app == desktop::App::Prompt);
        (x.dev == dev && x.open.get() != 0 && console).then(|| base(k))
    })
}

/// every device window (at the end of the frame)
pub(crate) fn frame(n: &mut Net, ui: &mut Ui) {
    for k in 0..DEVWINS {
        let Some(w) = n.devwin.wins[k].as_ref() else { continue };
        let (dev, open, rect) = (w.dev, w.open.clone(), w.rect.clone());
        // a deleted device closes its window
        let Some(d) = n.doc.proj.device(dev) else {
            open.set(0);
            n.devwin.wins[k] = None;
            continue;
        };
        let title = format!("{} – {}", d.name, catalog::model(&d.model).map_or(t("Gerät"), |m| t(m.name)));
        let id = base(k);
        if ui.window_begin(id, &title, Some(&open), WF_TOOL | WF_RESIZABLE, Some(&rect)) {
            window(n, ui, k, dev);
        }
        ui.window_end();
        if open.get() == 0 {
            n.devwin.wins[k] = None;
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
        Tab::Services => services::tab(n, ui, k, id + 0x60, dev),
        Tab::Cli => {
            ui.next_size(-1, -1);
            ui.box_begin(0, BOX_V, 0);
            console::view(n, ui, dev);
            ui.box_end();
        }
        Tab::Desktop => desktop::tab(n, ui, k, id + 0x80, dev),
        Tab::Scripts => scripts::tab(n, ui, k, ID_SCRIPTS0 + 0x400 * k as u32, dev),
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
    let svcs = n.doc.sim.engine().services(dev);
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
