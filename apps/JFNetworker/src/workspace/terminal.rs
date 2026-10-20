// terminal -- the Terminal panel: a tab per open console (doc/console.rs keeps them), the shown
// console, and the consoles in windows of their own
//
//   panel(n, ui)        the tabs, the buttons of the shown tab (rename, own window, side by
//                       side, clear, close) and its console; without a console how to open one
//   windows(n, ui)      the windows of consoles (after the main window's content); closing one
//                       brings its console back into the panel
//   states(n, ui)       enabled / disabled of the console commands (every frame)
//   command(n, ui, id)  CMD_CONSOLE, CMD_DEVWIN, CMD_TERM_*, CMD_LOG_CLEAR -> handled
//   activate(n, ui, id) a double click or Enter on the canvas: a device's console (with Shift,
//                       or without a command line, its device window), a note's text
//
// A console is drawn in one place at a time: while a device window shows it (its CLI tab, the
// desktop's prompt) its tab and its window say so, with a button that brings that window to
// the front.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use ui::*;

use crate::app::constants::*;
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::console::{self, Rename};
use crate::workspace::icons::JN_TERMINAL;
use crate::workspace::{canvas, devwin};

/// the Terminal panel
pub(crate) fn panel(n: &mut Net, ui: &mut Ui) {
    let docked = n.cons.docked();
    if docked.is_empty() {
        let floating = !n.cons.tabs.is_empty();
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER | pad(12), 0);
        if floating {
            ui.empty_state(JN_TERMINAL, t("Die Konsolen sind in eigenen Fenstern"), t("„Andocken“ in einem Konsolenfenster bringt es als Tab hierher zurück."));
        } else {
            ui.empty_state(JN_TERMINAL, t("Keine Konsole geöffnet"), t("Hier erscheinen die Befehlszeilen der Geräte: IOS-artig bei Routern und Switches, eine Linux-Shell bei Hosts. Ein Doppelklick auf ein Gerät öffnet seine Konsole."));
        }
        ui.box_end();
        return;
    }
    let cur = match n.cons.cur.filter(|d| docked.contains(d)) {
        Some(d) => d,
        None => docked[0],
    };
    n.cons.cur = Some(cur);
    let titles: Vec<String> = docked.iter().map(|&d| n.cons.title(&n.doc.proj, d)).collect();
    let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
    let mut sel = docked.iter().position(|&d| d == cur).unwrap_or(0) as i32;
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER | pad(2), 2);
    ui.next_size(-1, 0);
    if ui.tabs_scrolling(ID_TERM_TABS, &mut sel, &refs) & EV_ACT != 0 {
        if let Some(&d) = docked.get(sel.max(0) as usize) {
            n.cons.cur = Some(d);
            n.cons.focus = Some(d);
        }
        ui.repass();
    }
    ui.cmd_button(CMD_TERM_RENAME, ST_ICON_ONLY | ST_SUBTLE | ST_COMPACT);
    rename_flyout(n, ui);
    for c in [CMD_TERM_FLOAT, CMD_TERM_TILE, CMD_TERM_CLEAR, CMD_TERM_CLOSE] {
        ui.cmd_button(c, ST_ICON_ONLY | ST_SUBTLE | ST_COMPACT);
    }
    ui.box_end();
    let cur = n.cons.cur.unwrap_or(cur);
    here(n, ui, cur);
    ui.box_end();
}

// the console of `dev` here, or where it is shown instead
fn here(n: &mut Net, ui: &mut Ui, dev: u32) {
    match devwin::shows(n, dev) {
        Some(win) => {
            let name = n.doc.proj.device(dev).map_or_else(String::new, |d| d.name.clone());
            ui.next_size(-1, -1);
            ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER | pad(12), 8);
            ui.empty_state(JN_TERMINAL, &tf("Die Konsole von {} ist im Gerätefenster offen", &[&name]), t("Eine Konsole erscheint an einer Stelle: solange das Gerätefenster sie zeigt, tippt man dort."));
            if ui.button(ID_TERM_SHOW, t("Gerätefenster zeigen")) & EV_ACT != 0 {
                ui.window_activate(win);
            }
            ui.box_end();
        }
        None => console::view(n, ui, dev),
    }
}

// the rename flyout of the shown tab (under its button)
fn rename_flyout(n: &mut Net, ui: &mut Ui) {
    let mut gone = false;
    if let Some(r) = &mut n.cons.rename {
        if !r.opened {
            r.opened = true;
            ui.popup_open(ID_TERM_RENAME);
            ui.focus(ID_TERM_RENAME_BOX);
        } else if !ui.popup_is_open(ID_TERM_RENAME) {
            gone = true; // Esc, a click elsewhere
        }
    } else if ui.popup_is_open(ID_TERM_RENAME) {
        ui.popup_close();
    }
    if gone {
        n.cons.rename = None;
    }
    let mut act = 0; // 1 commit, 2 cancel
    if ui.flyout_begin(ID_TERM_RENAME, FL_BELOW) {
        if let Some(r) = &mut n.cons.rename {
            ui.box_begin(0, BOX_V | pad(8), 6);
            ui.text(TS_BODY_STRONG, t("Name des Tabs"));
            if ui.focused() == ID_TERM_RENAME_BOX && ui.capture_shortcut(VK_ESCAPE, 0) {
                act = 2;
            }
            ui.next_size(240, 0);
            let ev = ui.textbox(ID_TERM_RENAME_BOX, Some(t("leer: der Name des Geräts")), &mut r.text, 64);
            if ev & EV_COMMIT != 0 && ui.focused() == ID_TERM_RENAME_BOX {
                act = 1;
            }
            ui.box_begin(0, BOX_H | BOX_END, 8);
            if ui.accent_button(ID_TERM_RENAME_OK, t("Umbenennen")) & EV_ACT != 0 {
                act = 1;
            }
            if ui.button(ID_TERM_RENAME_CANCEL, t("Abbrechen")) & EV_ACT != 0 {
                act = 2;
            }
            ui.box_end();
            ui.box_end();
        }
    }
    ui.flyout_end();
    match act {
        1 => {
            if let Some(r) = n.cons.rename.take() {
                let name = n.doc.proj.device(r.dev).map_or_else(String::new, |d| d.name.clone());
                let t = String::from(r.text.trim());
                if let Some(i) = n.cons.tab(r.dev) {
                    n.cons.tabs[i].title = if t.is_empty() || t == name { None } else { Some(t) };
                }
            }
            ui.popup_close();
            ui.repass();
        }
        2 => {
            n.cons.rename = None;
            ui.popup_close();
            ui.repass();
        }
        _ => {}
    }
}

/// the windows of consoles; closing one brings its console back into the panel
pub(crate) fn windows(n: &mut Net, ui: &mut Ui) {
    for k in 0..TERM_WINS {
        let dev = n.cons.in_win(k);
        let (open, rect) = (n.cons.wins[k].0.clone(), n.cons.wins[k].1.clone());
        if dev.is_none() {
            open.set(0);
        }
        let title = dev.map_or_else(String::new, |d| tf("Konsole – {}", &[&n.cons.title(&n.doc.proj, d)]));
        let id = ID_TERM_WIN0 + 0x10 * k as u32;
        if ui.window_begin(id, &title, Some(&open), WF_TOOL | WF_RESIZABLE, Some(&rect)) {
            if let Some(dev) = dev {
                ui.next_size(-1, -1);
                ui.box_begin(0, BOX_V | pad(6), 4);
                ui.box_begin(0, BOX_H | BOX_XCENTER, 4);
                let ios = n.cons.by_dev.get(&dev).is_some_and(|c| c.ios);
                ui.next_size(-1, 0);
                ui.text(
                    TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS,
                    t(if ios { "Strg+F sucht, Strg+Umschalt+6 bricht ab; Schließen dockt die Konsole wieder an." } else { "Strg+F sucht, Strg+C bricht ab; Schließen dockt die Konsole wieder an." }),
                );
                ui.next_icon(IC_BACK_TO_WINDOW);
                ui.next_flags(ST_SUBTLE | ST_COMPACT);
                if ui.button(id + 1, t("Andocken")) & EV_ACT != 0 {
                    console::dock(n, dev);
                    ui.repass();
                }
                ui.box_end();
                here(n, ui, dev);
                ui.box_end();
            }
        }
        ui.window_end();
        // closed by the user: back into the panel
        if let Some(d) = dev
            && open.get() == 0
            && n.cons.tab(d).is_some_and(|i| n.cons.tabs[i].win == Some(k))
        {
            console::dock(n, d);
            ui.repass();
        }
    }
}

// the selected device, when exactly one is selected
fn selected_device(n: &Net) -> Option<u32> {
    match canvas::single_item(n)? {
        crate::doc::edits::Item::Device(d) => Some(d),
        _ => None,
    }
}

fn flag(on: bool, f: u32) -> u32 {
    if on { f } else { 0 }
}

/// enabled / disabled of the console commands
pub(crate) fn states(n: &Net, ui: &mut Ui) {
    let dev = selected_device(n);
    ui.cmd_state(CMD_CONSOLE, flag(!dev.is_some_and(|d| console::has_cli(&n.doc.proj, d)), CF_DISABLED));
    ui.cmd_state(CMD_DEVWIN, flag(dev.is_none(), CF_DISABLED));
    let tab = n.cons.cur.is_some_and(|d| n.cons.tab(d).is_some_and(|i| n.cons.tabs[i].win.is_none()));
    for c in [CMD_TERM_CLOSE, CMD_TERM_RENAME, CMD_TERM_FLOAT, CMD_TERM_CLEAR] {
        ui.cmd_state(c, flag(!tab, CF_DISABLED));
    }
    ui.cmd_state(CMD_TERM_TILE, flag(n.cons.tabs.is_empty(), CF_DISABLED));
}

/// the console commands (App::command) -> handled
pub(crate) fn command(n: &mut Net, ui: &mut Ui, id: u32) -> bool {
    let cur = n.cons.cur.filter(|&d| n.cons.tab(d).is_some_and(|i| n.cons.tabs[i].win.is_none()));
    match id {
        CMD_CONSOLE => {
            if let Some(d) = selected_device(n) {
                open_console(n, ui, d);
            }
        }
        CMD_DEVWIN => {
            if let Some(d) = selected_device(n) {
                devwin::open(n, ui, d);
            }
        }
        CMD_TERM_CLOSE => {
            if let Some(d) = cur {
                console::close(n, d);
            }
        }
        CMD_TERM_RENAME => {
            if let Some(d) = cur {
                let text = n.cons.title(&n.doc.proj, d);
                n.cons.rename = Some(Rename { dev: d, text, opened: false });
            }
        }
        CMD_TERM_FLOAT => {
            if let Some(d) = cur
                && !console::float(n, ui, d)
            {
                n.file.msg = Some((IB_INFO, String::from(t("Kein Fenster frei")), tf("Höchstens {} Konsolen laufen in eigenen Fenstern; „Andocken“ gibt eines frei.", &[&TERM_WINS.to_string()])));
            }
        }
        CMD_TERM_TILE => console::tile(n, ui),
        CMD_TERM_CLEAR => {
            if let Some(c) = cur.and_then(|d| n.cons.by_dev.get_mut(&d)) {
                c.term.clear();
            }
        }
        CMD_LOG_CLEAR => n.log.clear(),
        _ => return false,
    }
    ui.repass();
    true
}

// the console of a device, or why it has none (the info bar above the canvas)
fn open_console(n: &mut Net, ui: &mut Ui, dev: u32) {
    if let Err(e) = console::open(n, ui, dev) {
        n.file.msg = Some((IB_INFO, String::from(t("Keine Konsole")), e));
    }
}

/// a double click or Enter on the canvas: a device's console (with Shift, or without a command
/// line, its device window), a note's text
pub(crate) fn activate(n: &mut Net, ui: &mut Ui, id: u32) {
    if n.doc.proj.device(id).is_none() {
        n.canvas.open_rename(&n.doc.proj, id);
        return;
    }
    if ui.mods() & MOD_SHIFT != 0 || !console::has_cli(&n.doc.proj, id) {
        devwin::open(n, ui, id);
    } else {
        open_console(n, ui, id);
    }
}
