// commands -- the command table, the menus and what the commands do
//
//   CMDS / MENUBAR / M_CANVAS     the table (labels, icons, accelerators) and the menus
//   states(n, ui)                 enabled / checked per command from the state (every frame)
//   command(n, ui, id, arg)       App::command: menus, buttons, accelerators, the dialogs'
//                                 answers, closing the window
//
// Datei: Neu, Öffnen, Zuletzt geöffnet, Speichern (unter), Exportieren (the recording as
// pcapng), Beenden; Bearbeiten: Rückgängig, Wiederholen, Löschen, Alles auswählen, Umbenennen
// (F2); Ansicht: Zoom, 100 %, Alles zeigen, Raster, Einrasten, Minimap, Bereiche (Ctrl+1 .. 8);
// Simulation: Echtzeit / Simulationsmodus, Abspielen (F6), Schritt vor (F7) und zurück
// (Shift+F7), Zurück zu live, Tempo, PDUs, Nur Auswahl, Zurücksetzen (workspace/sim);
// Werkzeuge: the tools with their single keys (V, H, C, N, P) and Einstellungen (Ctrl+,);
// Hilfe: Tastenkürzel (F1), Info.  Simulation (F5) and Echtzeit (Shift+F5) switch the mode,
// Ctrl+9 gives the topology the keyboard as Ctrl+1 .. 8 do for the panels.  The context menus of the canvas:
// a cable (route, kind, Trennen), a device (Konsole öffnen, Gerätefenster, Umbenennen,
// Eingeschaltet, Löschen), the background.  TIMER_SIM moves the simulator's clock, TIMER_ANIM
// the simulation mode's animation.  The consoles' commands (CMD_CONSOLE .. CMD_LOG_CLEAR: tabs,
// windows, the log) are workspace/terminal.rs's.

use jfn_model::CableKind;
use ui::*;

use crate::app::constants::*;
use crate::app::Net;
use crate::doc::file::{self, After};
use crate::workspace::canvas::{self, cables, tools, tools::Tool};
use crate::workspace::icons::*;
use crate::workspace::terminal;

pub(crate) static CMDS: [Command; 75] = [
    Command::new(CMD_NEW, "&Neu", IC_DOCUMENT, MOD_CTRL, b'N' as u32, 0),
    Command::new(CMD_OPEN, "Ö&ffnen …", IC_OPEN_FILE, MOD_CTRL, b'O' as u32, 0),
    Command::new(CMD_SAVE, "&Speichern", IC_SAVE, MOD_CTRL, b'S' as u32, 0),
    Command::new(CMD_SAVE_AS, "Speichern &unter …", IC_SAVE_AS, MOD_CTRL | MOD_SHIFT, b'S' as u32, 0),
    Command::new(CMD_QUIT, "&Beenden", 0, MOD_CTRL, b'Q' as u32, 0),
    Command::new(CMD_UNDO, "&Rückgängig", IC_UNDO, MOD_CTRL, b'Z' as u32, CF_DISABLED),
    Command::new(CMD_REDO, "&Wiederholen", IC_REDO, MOD_CTRL, b'Y' as u32, CF_DISABLED),
    Command::new(CMD_DELETE, "&Löschen", IC_DELETE, 0, VK_DELETE, CF_DISABLED),
    Command::new(CMD_SELECT_ALL, "&Alles auswählen", IC_SELECT_ALL, MOD_CTRL, b'A' as u32, 0),
    Command::new(CMD_RENAME, "&Umbenennen …", IC_EDIT, 0, VK_F2, CF_DISABLED),
    Command::new(CMD_ZOOM_IN, "Ver&größern", IC_ZOOM_IN, MOD_CTRL, VK_OEM_PLUS, 0),
    Command::new(CMD_ZOOM_OUT, "Ver&kleinern", IC_ZOOM_OUT, MOD_CTRL, VK_OEM_MINUS, 0),
    Command::new(CMD_ZOOM_100, "&100 %", 0, 0, 0, 0),
    Command::new(CMD_FIT, "Alles &zeigen", JN_FIT, MOD_CTRL, b'0' as u32, 0),
    Command::new(CMD_GRID, "&Raster", JN_GRID, 0, 0, 0),
    Command::new(CMD_SNAP, "&Einrasten", JN_SNAP, 0, 0, 0),
    Command::new(CMD_MINIMAP, "&Minimap", JN_MINIMAP, 0, 0, 0),
    Command::new(CMD_LAYOUT_RESET, "Anordnung &zurücksetzen", 0, 0, 0, 0),
    Command::new(CMD_PANEL0, "&Geräte", JN_DEVICES, MOD_CTRL, b'1' as u32, 0),
    Command::new(CMD_PANEL0 + 1, "&Eigenschaften", IC_SETTINGS, MOD_CTRL, b'2' as u32, 0),
    Command::new(CMD_PANEL0 + 2, "&Terminal", JN_TERMINAL, MOD_CTRL, b'3' as u32, 0),
    Command::new(CMD_PANEL0 + 3, "E&reignisse", IC_BULLETED_LIST, MOD_CTRL, b'4' as u32, 0),
    Command::new(CMD_PANEL0 + 4, "&Paket", JN_PACKET, MOD_CTRL, b'5' as u32, 0),
    Command::new(CMD_PANEL0 + 5, "Pr&otokoll", IC_HISTORY, MOD_CTRL, b'6' as u32, 0),
    Command::new(CMD_PANEL0 + 6, "Pro&bleme", IC_WARNING, MOD_CTRL, b'7' as u32, 0),
    Command::new(CMD_TOOL_SELECT, "Auswahl", JN_SELECT, 0, b'V' as u32, 0),
    Command::new(CMD_TOOL_HAND, "Hand", JN_HAND, 0, b'H' as u32, 0),
    Command::new(CMD_TOOL_CONNECT, "Verbinden", JN_CONNECT, 0, b'C' as u32, 0),
    Command::new(CMD_TOOL_NOTE, "Notiz einfügen", JN_NOTE, 0, b'N' as u32, 0),
    Command::new(CMD_SIM_REALTIME, "&Echtzeit", 0, MOD_SHIFT, VK_F5, CF_DISABLED | CF_CHECKED),
    Command::new(CMD_SIM_MODE, "&Simulationsmodus", 0, 0, VK_F5, CF_DISABLED),
    Command::new(CMD_SIM_PLAY, "&Abspielen / Anhalten", IC_PLAY, 0, VK_F6, CF_DISABLED),
    Command::new(CMD_SIM_STEP, "&Schritt vor", IC_NEXT, 0, VK_F7, CF_DISABLED),
    Command::new(CMD_SIM_BACK, "Schritt &zurück", IC_PREVIOUS, MOD_SHIFT, VK_F7, CF_DISABLED),
    Command::new(CMD_SIM_RESET, "Simulation zu&rücksetzen", 0, 0, 0, CF_DISABLED),
    Command::new(CMD_ABOUT, "&Info …", IC_INFO, 0, 0, 0),
    Command::new(CMD_ROUTE_STRAIGHT, "&Gerade", 0, 0, 0, CF_DISABLED),
    Command::new(CMD_ROUTE_ORTHO, "&Rechtwinklig", 0, 0, 0, CF_DISABLED),
    Command::new(CMD_ROUTE_CURVE, "&Kurve", 0, 0, 0, CF_DISABLED),
    Command::new(CMD_DISCONNECT, "&Trennen", 0, 0, 0, CF_DISABLED),
    Command::new(CMD_POWER, "&Eingeschaltet", 0, 0, 0, CF_DISABLED),
    Command::new(CMD_CABLE0, "&Automatisch", 0, 0, 0, CF_DISABLED),
    Command::new(CMD_CABLE0 + 1, "Kupfer, &gerade", 0, 0, 0, CF_DISABLED),
    Command::new(CMD_CABLE0 + 2, "Kupfer, ge&kreuzt", 0, 0, 0, CF_DISABLED),
    Command::new(CMD_CABLE0 + 3, "Glasfaser, &Multimode", 0, 0, 0, CF_DISABLED),
    Command::new(CMD_CABLE0 + 4, "Glasfaser, &Singlemode", 0, 0, 0, CF_DISABLED),
    Command::new(CMD_CABLE0 + 5, "Se&riell (WAN)", 0, 0, 0, CF_DISABLED),
    Command::new(CMD_CABLE0 + 6, "K&onsolenkabel", 0, 0, 0, CF_DISABLED),
    Command::new(CMD_CABLE0 + 7, "&Funk (WLAN)", 0, 0, 0, CF_DISABLED),
    Command::new(CMD_CABLE0 + 8, "&WAN-Leitung", 0, 0, 0, CF_DISABLED),
    Command::new(CMD_CONSOLE, "&Konsole öffnen", JN_TERMINAL, 0, 0, CF_DISABLED),
    Command::new(CMD_DEVWIN, "&Gerätefenster …", IC_OPEN_IN_NEW_WINDOW, 0, 0, CF_DISABLED),
    Command::new(CMD_TERM_CLOSE, "Konsole &schließen", IC_CHROME_CLOSE, 0, 0, CF_DISABLED),
    Command::new(CMD_TERM_RENAME, "Tab &umbenennen …", IC_EDIT, 0, 0, CF_DISABLED),
    Command::new(CMD_TERM_FLOAT, "In eigenem &Fenster", IC_OPEN_IN_NEW_WINDOW, 0, 0, CF_DISABLED),
    Command::new(CMD_TERM_TILE, "Konsolen &nebeneinander", IC_VIEW_ALL, 0, 0, CF_DISABLED),
    Command::new(CMD_TERM_CLEAR, "Konsole &leeren", IC_DELETE, 0, 0, CF_DISABLED),
    Command::new(CMD_LOG_CLEAR, "Protokoll &leeren", IC_DELETE, 0, 0, 0),
    Command::new(CMD_SIM_TEMPO0, "0,25×", 0, 0, 0, 0),
    Command::new(CMD_SIM_TEMPO0 + 1, "0,5×", 0, 0, 0, 0),
    Command::new(CMD_SIM_TEMPO0 + 2, "1×", 0, 0, 0, 0),
    Command::new(CMD_SIM_TEMPO0 + 3, "2×", 0, 0, 0, 0),
    Command::new(CMD_SIM_TEMPO0 + 4, "4×", 0, 0, 0, 0),
    Command::new(CMD_SIM_TEMPO0 + 5, "8×", 0, 0, 0, 0),
    Command::new(CMD_SIM_LIVE, "Zurück zu &live", IC_HISTORY, 0, 0, CF_DISABLED),
    Command::new(CMD_SIM_ONLY_SEL, "&Nur Auswahl", 0, 0, 0, 0),
    Command::new(CMD_TOOL_PDU, "Einfache &PDU", IC_MAIL, 0, b'P' as u32, 0),
    Command::new(CMD_PDU_COMPLEX, "&Komplexe PDU …", IC_ADD, 0, 0, 0),
    Command::new(CMD_EXPORT_PCAP, "Aufzeichnung als &pcapng …", 0, 0, 0, CF_DISABLED),
    Command::new(CMD_PANEL0 + 7, "P&DUs", IC_MAIL, MOD_CTRL, b'8' as u32, 0),
    Command::new(CMD_FOCUS_CANVAS, "Topo&logie", JN_CONNECT, MOD_CTRL, b'9' as u32, 0),
    Command::new(CMD_SETTINGS, "&Einstellungen …", IC_SETTINGS, MOD_CTRL, VK_OEM_COMMA, 0),
    Command::new(CMD_SHORTCUTS, "&Tastenkürzel …", IC_HELP, 0, VK_F1, 0),
    Command::new(CMD_METRICS, "Me&triken", JN_METRICS, MOD_CTRL | MOD_SHIFT, b'M' as u32, 0),
    Command::new(CMD_HEAL_ALL, "Alle &Fehler beheben", IC_REPAIR, 0, 0, CF_DISABLED),
];

pub(crate) static MENUBAR: [Mi; 6] = [
    Mi::Menu("&Datei", &M_FILE),
    Mi::Menu("&Bearbeiten", &M_EDIT),
    Mi::Menu("&Ansicht", &M_VIEW),
    Mi::Menu("&Simulation", &M_SIM),
    Mi::Menu("&Werkzeuge", &M_TOOLS),
    Mi::Menu("&Hilfe", &M_HELP),
];
static M_FILE: [Mi; 10] = [
    Mi::Cmd(CMD_NEW),
    Mi::Cmd(CMD_OPEN),
    Mi::Sub("&Zuletzt geöffnet", &M_RECENT),
    Mi::Sep,
    Mi::Cmd(CMD_SAVE),
    Mi::Cmd(CMD_SAVE_AS),
    Mi::Sep,
    Mi::Sub("E&xportieren", &M_EXPORT),
    Mi::Sep,
    Mi::Cmd(CMD_QUIT),
];
static M_EXPORT: [Mi; 1] = [Mi::Cmd(CMD_EXPORT_PCAP)];
static M_RECENT: [Mi; 1] = [Mi::List(CMD_RECENT0, RECENT_MAX as u16, "Keine Dateien")];
static M_EDIT: [Mi; 7] = [
    Mi::Cmd(CMD_UNDO),
    Mi::Cmd(CMD_REDO),
    Mi::Sep,
    Mi::Cmd(CMD_DELETE),
    Mi::Cmd(CMD_SELECT_ALL),
    Mi::Sep,
    Mi::Cmd(CMD_RENAME),
];
static M_VIEW: [Mi; 10] = [
    Mi::Cmd(CMD_ZOOM_IN),
    Mi::Cmd(CMD_ZOOM_OUT),
    Mi::Cmd(CMD_ZOOM_100),
    Mi::Cmd(CMD_FIT),
    Mi::Sep,
    Mi::Cmd(CMD_GRID),
    Mi::Cmd(CMD_SNAP),
    Mi::Cmd(CMD_MINIMAP),
    Mi::Sep,
    Mi::Sub("&Bereiche", &M_PANELS),
];
static M_PANELS: [Mi; 12] = [
    Mi::Cmd(CMD_FOCUS_CANVAS),
    Mi::Cmd(CMD_PANEL0),
    Mi::Cmd(CMD_PANEL0 + 1),
    Mi::Cmd(CMD_PANEL0 + 2),
    Mi::Cmd(CMD_PANEL0 + 3),
    Mi::Cmd(CMD_PANEL0 + 4),
    Mi::Cmd(CMD_PANEL0 + 5),
    Mi::Cmd(CMD_PANEL0 + 6),
    Mi::Cmd(CMD_PANEL0 + 7),
    Mi::Cmd(CMD_METRICS),
    Mi::Sep,
    Mi::Cmd(CMD_LAYOUT_RESET),
];
static M_SIM: [Mi; 17] = [
    Mi::Cmd(CMD_SIM_REALTIME),
    Mi::Cmd(CMD_SIM_MODE),
    Mi::Sep,
    Mi::Cmd(CMD_SIM_PLAY),
    Mi::Cmd(CMD_SIM_STEP),
    Mi::Cmd(CMD_SIM_BACK),
    Mi::Cmd(CMD_SIM_LIVE),
    Mi::Sub("&Tempo", &M_TEMPO),
    Mi::Sep,
    Mi::Cmd(CMD_TOOL_PDU),
    Mi::Cmd(CMD_PDU_COMPLEX),
    Mi::Sep,
    Mi::Cmd(CMD_SIM_ONLY_SEL),
    Mi::Sep,
    Mi::Cmd(CMD_SIM_RESET),
    Mi::Sep,
    Mi::Cmd(CMD_HEAL_ALL),
];
static M_TEMPO: [Mi; 6] = [
    Mi::Cmd(CMD_SIM_TEMPO0),
    Mi::Cmd(CMD_SIM_TEMPO0 + 1),
    Mi::Cmd(CMD_SIM_TEMPO0 + 2),
    Mi::Cmd(CMD_SIM_TEMPO0 + 3),
    Mi::Cmd(CMD_SIM_TEMPO0 + 4),
    Mi::Cmd(CMD_SIM_TEMPO0 + 5),
];
/// the tools (their single keys) and the settings
static M_TOOLS: [Mi; 7] = [
    Mi::Cmd(CMD_TOOL_SELECT),
    Mi::Cmd(CMD_TOOL_HAND),
    Mi::Cmd(CMD_TOOL_CONNECT),
    Mi::Cmd(CMD_TOOL_NOTE),
    Mi::Cmd(CMD_TOOL_PDU),
    Mi::Sep,
    Mi::Cmd(CMD_SETTINGS),
];
static M_HELP: [Mi; 3] = [Mi::Cmd(CMD_SHORTCUTS), Mi::Sep, Mi::Cmd(CMD_ABOUT)];
/// the canvas' context menu (the background, notes)
pub(crate) static M_CANVAS: [Mi; 5] = [Mi::Cmd(CMD_RENAME), Mi::Cmd(CMD_DELETE), Mi::Sep, Mi::Cmd(CMD_SELECT_ALL), Mi::Cmd(CMD_FIT)];
/// the context menu of a device
pub(crate) static M_DEVICE: [Mi; 10] = [
    Mi::Cmd(CMD_CONSOLE),
    Mi::Cmd(CMD_DEVWIN),
    Mi::Sep,
    Mi::Cmd(CMD_RENAME),
    Mi::Cmd(CMD_POWER),
    Mi::Sep,
    Mi::Cmd(CMD_DELETE),
    Mi::Sep,
    Mi::Cmd(CMD_SELECT_ALL),
    Mi::Cmd(CMD_FIT),
];
/// the context menu of a cable
pub(crate) static M_CABLE: [Mi; 7] = [
    Mi::Cmd(CMD_ROUTE_STRAIGHT),
    Mi::Cmd(CMD_ROUTE_ORTHO),
    Mi::Cmd(CMD_ROUTE_CURVE),
    Mi::Sep,
    Mi::Sub("Kabel&typ", &M_CABLE_KIND),
    Mi::Sep,
    Mi::Cmd(CMD_DISCONNECT),
];
static M_CABLE_KIND: [Mi; 10] = [
    Mi::Cmd(CMD_CABLE0),
    Mi::Sep,
    Mi::Cmd(CMD_CABLE0 + 1),
    Mi::Cmd(CMD_CABLE0 + 2),
    Mi::Cmd(CMD_CABLE0 + 3),
    Mi::Cmd(CMD_CABLE0 + 4),
    Mi::Cmd(CMD_CABLE0 + 5),
    Mi::Cmd(CMD_CABLE0 + 6),
    Mi::Cmd(CMD_CABLE0 + 7),
    Mi::Cmd(CMD_CABLE0 + 8),
];

fn flag(on: bool, f: u32) -> u32 {
    if on { f } else { 0 }
}

/// enabled / checked per command (every frame, before the widgets)
pub(crate) fn states(n: &Net, ui: &mut Ui) {
    let h = &n.doc.hist;
    let sel = !n.canvas.st.sel.is_empty();
    let any = !n.doc.proj.devices.is_empty() || !n.doc.proj.notes.is_empty();
    ui.cmd_state(CMD_UNDO, flag(!h.can_undo(), CF_DISABLED));
    ui.cmd_state(CMD_REDO, flag(!h.can_redo(), CF_DISABLED));
    ui.cmd_state(CMD_DELETE, flag(!sel, CF_DISABLED));
    ui.cmd_state(CMD_SELECT_ALL, flag(!any, CF_DISABLED));
    ui.cmd_state(CMD_RENAME, flag(canvas::single(n).is_none(), CF_DISABLED));
    ui.cmd_state(CMD_GRID, flag(n.prefs.grid, CF_CHECKED));
    ui.cmd_state(CMD_SNAP, flag(n.prefs.snap, CF_CHECKED));
    ui.cmd_state(CMD_MINIMAP, flag(n.prefs.minimap, CF_CHECKED));
    for (k, &p) in PANELS.iter().enumerate() {
        ui.cmd_state(CMD_PANEL0 + k as u32, flag(n.shell.dock.visible(p), CF_CHECKED));
    }
    let t = n.canvas.tool;
    ui.cmd_state(CMD_TOOL_SELECT, flag(t == Tool::Select, CF_CHECKED));
    ui.cmd_state(CMD_TOOL_HAND, flag(t == Tool::Hand, CF_CHECKED));
    ui.cmd_state(CMD_TOOL_CONNECT, flag(t == Tool::Connect, CF_CHECKED));
    ui.cmd_state(CMD_TOOL_NOTE, flag(t == Tool::Note, CF_CHECKED));
    crate::workspace::sim::states(n, ui);
    cables::states(n, ui);
    terminal::states(n, ui);
    crate::workspace::metrics::states(n, ui);
}

// Ctrl+1 .. 8: the panel shows; the keyboard goes into it -- the device search, the shown
// console -- or to its tab strip (Tab goes on into it)
fn focus_panel(n: &mut Net, ui: &mut Ui, p: u32) {
    let console = n.cons.cur.filter(|d| n.cons.docked().contains(d));
    match p {
        P_DEVICES => {
            n.shell.dock.show(p);
            n.shell.dock.activate(p);
            ui.focus(ID_SEARCH);
        }
        P_TERMINAL if console.is_some() => {
            n.shell.dock.show(p);
            n.shell.dock.activate(p);
            n.cons.focus = console;
        }
        _ => n.shell.dock.focus(p),
    }
}

/// App::command
pub(crate) fn command(n: &mut Net, ui: &mut Ui, id: u32, arg: u32) -> bool {
    match id {
        UIC_TIMER if arg == TIMER_SIM => {
            // the simulator's clock: a new snapshot needs a new pass
            if n.doc.sim.tick() {
                ui.repass();
            }
            return true;
        }
        UIC_TIMER if arg == TIMER_ANIM => {
            crate::workspace::sim::tick(n, ui);
            return true;
        }
        UIC_TIMER if arg == TIMER_AUTOSAVE => {
            crate::doc::autosave::tick(n, ui);
            return true;
        }
        // the system goes to sleep, the session ends: a copy of unsaved work now
        UIC_SUSPEND if arg != 0 => {
            crate::doc::autosave::flush(n);
            return false;
        }
        UIC_SESSION_END => {
            crate::doc::autosave::flush(n);
            return false;
        }
        UIC_CLOSE => {
            if !n.doc.dirty() {
                return false; // the window closes
            }
            file::request(n, ui, After::Quit);
        }
        UIC_EXIT => {
            crate::doc::autosave::exit(n);
            n.save_prefs(ui);
            return false;
        }
        UIC_ESCAPE => {
            if !matches!(n.canvas.tool, Tool::Place(_) | Tool::Note | Tool::Connect | Tool::Pdu) {
                return false;
            }
            n.canvas.tool = Tool::Select;
            n.canvas.kb_from = None;
            n.sim.pdu.src = None;
        }
        CMD_NEW => file::request(n, ui, After::New),
        CMD_OPEN => file::request(n, ui, After::Open),
        CMD_SAVE => {
            file::save(n, ui);
        }
        CMD_SAVE_AS => file::save_as(n, ui),
        CMD_QUIT => file::request(n, ui, After::Quit),
        CMD_FILE_DONE => file::chosen(n, ui, arg != 0),
        CMD_ASK_ANSWER => file::answer(n, ui, arg != 0),
        _ if (CMD_RECENT0..CMD_RECENT0 + RECENT_MAX as u32).contains(&id) => {
            let Some(p) = n.file.recent.get((id - CMD_RECENT0) as usize).cloned() else { return true };
            file::request(n, ui, After::OpenPath(p));
        }
        CMD_UNDO | CMD_REDO => {
            // what comes back (an undone delete, a redone add) is selected
            let before = canvas::ids(&n.doc.proj);
            if id == CMD_UNDO { n.doc.undo() } else { n.doc.redo() };
            n.canvas.rename = None;
            let back: alloc::vec::Vec<u32> = canvas::ids(&n.doc.proj).difference(&before).copied().collect();
            if !back.is_empty() {
                n.canvas.st.sel = back.iter().map(|&k| k as u64).collect();
            }
        }
        CMD_DELETE => canvas::delete_selection(n, ui),
        CMD_SELECT_ALL => canvas::select_all(n, ui),
        CMD_RENAME => canvas::rename_selection(n, ui),
        CMD_ZOOM_IN => canvas::zoom(n, ui, 1),
        CMD_ZOOM_OUT => canvas::zoom(n, ui, -1),
        CMD_ZOOM_100 => canvas::zoom(n, ui, 0),
        CMD_FIT => canvas::zoom(n, ui, 2),
        CMD_GRID => {
            n.prefs.grid = !n.prefs.grid;
            n.prefs.push();
        }
        CMD_SNAP => {
            n.prefs.snap = !n.prefs.snap;
            n.prefs.push();
        }
        CMD_MINIMAP => {
            n.prefs.minimap = !n.prefs.minimap;
            n.prefs.push();
        }
        CMD_LAYOUT_RESET => n.shell.dock.reset(),
        _ if (CMD_PANEL0..CMD_PANEL0 + PANELS.len() as u32).contains(&id) => focus_panel(n, ui, PANELS[(id - CMD_PANEL0) as usize]),
        CMD_TOOL_SELECT => tools::set(n, Tool::Select),
        CMD_TOOL_HAND => tools::set(n, Tool::Hand),
        CMD_TOOL_CONNECT => tools::set(n, Tool::Connect),
        CMD_TOOL_NOTE => tools::set(n, Tool::Note),
        CMD_ABOUT => n.shell.about.set(1),
        CMD_FOCUS_CANVAS => ui.focus(ID_CANVAS),
        CMD_SETTINGS => n.settings_open.set(1),
        CMD_SHORTCUTS => n.keys_open.set(1),
        CMD_RECOVER => crate::doc::autosave::recover(n, ui),
        CMD_RECOVER_DROP => crate::doc::autosave::drop_offer(n, ui),
        CMD_ROUTE_STRAIGHT => cables::set_route(n, jfn_model::project::Route::Straight),
        CMD_ROUTE_ORTHO => cables::set_route(n, jfn_model::project::Route::Ortho),
        CMD_ROUTE_CURVE => cables::set_route(n, jfn_model::project::Route::Curve),
        CMD_DISCONNECT => cables::disconnect_selection(n),
        CMD_POWER => cables::toggle_power(n),
        _ if (CMD_CABLE0..=CMD_CABLE0 + CableKind::ALL.len() as u32).contains(&id) => {
            let k = (id - CMD_CABLE0) as usize;
            cables::set_kind(n, if k == 0 { None } else { Some(CableKind::ALL[k - 1]) });
        }
        CMD_CONSOLE..=CMD_LOG_CLEAR => return terminal::command(n, ui, id),
        // the simulation mode (workspace/sim)
        CMD_SIM_REALTIME | CMD_SIM_MODE | CMD_SIM_PLAY | CMD_SIM_STEP | CMD_SIM_BACK | CMD_SIM_RESET => return crate::workspace::sim::command(n, ui, id, arg),
        _ if (CMD_SIM_TEMPO0..200).contains(&id) => return crate::workspace::sim::command(n, ui, id, arg),
        // the lab (U6a: the Metriken panel, faults)
        CMD_METRICS | CMD_HEAL_ALL => return crate::workspace::metrics::command(n, ui, id, arg),
        _ => return false,
    }
    ui.repass();
    true
}
