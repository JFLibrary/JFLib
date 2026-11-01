// desktest -- windows inside an area of a window (ui/desk.rs), its taskbar and full screen
// (ui.window_fullscreen), for tests/*.uis and by hand
//
//   build/win/desktest.exe [--state FILE]          by hand: move, resize, snap, minimise,
//                                                  maximise the windows; F11 full screen
//                                                  (--state: loaded at the start, Speichern
//                                                  writes it)
//   build/win/desktest.exe --script apps/desktest/tests/move.uis
//   build/win/desktest.exe --check-fullscreen      with a real window: the main window, the
//                                                  tool window and the maximised main window
//                                                  into full screen and back, checked against
//                                                  the platform (a line per check on stdout,
//                                                  exit code 1 on a failure)
//
// The tool row: Neues Fenster (a window "Fenster N" with a button), Speichern (the state into
// memory, with --state also into the file), Laden, Zurücksetzen, Vollbild (the main window,
// also F11), Werkzeug (a tool window, id 601, shows / hides), Werkzeug-Vollbild, Baum (the
// screen readers' tree of the windows into a status line), Nur Symbole (the taskbar shows icons
// only: DTB_ICONS).  The area keeps a 40 DIP strip at
// its bottom for the taskbar (Start, a button per window, a clock).  Its windows: Editor (a
// text box and Übernehmen), Rechner (fixed size, no maximising: digits and a display), Notizen
// (at least 240 x 120: a check box), Protokoll (the last events); closing one takes it out of
// the list (and the state).  Ctrl+N opens a new window, Ctrl+Shift+N one that starts
// maximised (DWF_MAXIMIZED).  The status lines:
//   Z: Editor, Rechner=x, [Protokoll]     the z-order from the back, [the active one]; =x
//                                         maximised, =m minimised, =l / =r a half
//   Aktiv: Protokoll 40,30 420x300        the active window's normal place (DIP)
//   Ereignisse: ...                       how often desk_end reported each kind
//   Vollbild: Haupt aus | Werkzeug aus | Meldungen 0   window_state's UWS_FULLSCREEN, UIC_FULLSCREEN
//   Baum: Editor(…) …                     after Baum: each window's node with the number of
//                                         nodes inside it, where three content widgets hang
//   Gespeichert: desk 1 | win … | active …   after Speichern: the saved state's lines
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use plat::a11y::{Role, NONE};
use ui::*;

const ID_DESK: u32 = 500;
const ID_TOOL: u32 = 601; // the tool window
const ID_TWO: u32 = 602; // (tests/touch.uis) a widget that takes two fingers
const ID_NEW: u32 = 101;
const ID_SAVE: u32 = 102;
const ID_LOAD: u32 = 103;
const ID_RESET: u32 = 104;
const ID_FULL: u32 = 105;
const ID_TOOLWIN: u32 = 106;
const ID_TOOLFULL: u32 = 107;
const ID_TREE: u32 = 108;
const ID_ICONS: u32 = 109;
const ID_START: u32 = 110;
const ID_EDIT: u32 = 201;
const ID_APPLY: u32 = 202;
const ID_DIGIT: u32 = 210; // .. 219
const ID_IMPORTANT: u32 = 230;
const ID_EXTRA: u32 = 240; // .. : the buttons of the new windows
const ID_TOOLTEXT: u32 = 610;

const K_EDITOR: u32 = 1;
const K_CALC: u32 = 2;
const K_NOTES: u32 = 3;
const K_LOG: u32 = 4;
const K_EXTRA: u32 = 10; // the new windows: 10, 11, ..

const CMD_FULL: u32 = 1;
const CMD_NEW: u32 = 2;
const CMD_TREE: u32 = 3;
const CMD_NEW_MAX: u32 = 4;
static CMDS: [Command; 4] = [
    Command::new(CMD_FULL, "Vollbild", 0, 0, VK_F11, 0),
    Command::new(CMD_NEW, "Neues Fenster", 0, MOD_CTRL, b'N' as u32, 0),
    Command::new(CMD_TREE, "Baum", 0, 0, 0, 0),
    Command::new(CMD_NEW_MAX, "Neues maximiertes Fenster", 0, MOD_CTRL | MOD_SHIFT, b'N' as u32, 0),
];

const DESC: AppDesc = AppDesc { title: "jfgui Fenster", flags: AF_RESIZABLE, w: 960, h: 640, cmds: &CMDS, ..AppDesc::NONE };

struct Desk {
    desk: DeskState,
    /// the open windows in the taskbar's order
    open: Vec<u32>,
    next_extra: u32,
    /// the new windows that start maximised (Ctrl+Shift+N: DWF_MAXIMIZED)
    maxed: Vec<u32>,
    /// the taskbar shows icons only (DTB_ICONS)
    icons: i32,
    saved: String,
    file: Option<String>,
    text: String,
    display: String,
    important: i32,
    clicks: u32,
    log: Vec<String>,
    /// how often desk_end reported Close, Activate, Move, Resize, State
    evs: [u32; 5],
    two: String,
    tool_open: Shared<i32>,
    tool_rect: Shared<WinRect>,
    fs_events: u32,
    tree: String,
    /// --check-fullscreen: the step of the self-check (0 off), the size before, the failures
    check: u32,
    check_size: (i32, i32),
    check_fails: u32,
    check_wait: u32,
}

impl Default for Desk {
    fn default() -> Desk {
        let mut desk = DeskState::new();
        desk.set_reserve([0, 0, 0, 40]);
        Desk {
            desk,
            open: alloc::vec![K_EDITOR, K_CALC, K_NOTES, K_LOG],
            next_extra: K_EXTRA,
            maxed: Vec::new(),
            icons: 0,
            saved: String::new(),
            file: None,
            text: String::from("Hallo"),
            display: String::from("0"),
            important: 0,
            clicks: 0,
            log: Vec::new(),
            evs: [0; 5],
            two: String::new(),
            tool_open: Shared::new(0),
            tool_rect: Shared::new(WinRect::new(40, 60, 260, 120, 0)),
            fs_events: 0,
            tree: String::new(),
            check: 0,
            check_size: (0, 0),
            check_fails: 0,
            check_wait: 0,
        }
    }
}

const CHECK_TIMER: u32 = 900;
const CHECK_MS: u32 = 1200;

// the monitor of the main window (its client origin; the primary one where the platform does
// not tell): its size in px
fn monitor_size() -> (i32, i32) {
    let m = match plat::win_get_rect(0) {
        Some(r) => plat::screen::monitor_at(r[0] + r[2] / 2, r[1] + r[3] / 2),
        None => plat::screen::monitors().into_iter().next(),
    };
    m.map_or((0, 0), |m| (m.rect[2], m.rect[3]))
}

fn title(k: u32) -> String {
    match k {
        K_EDITOR => String::from("Editor"),
        K_CALC => String::from("Rechner"),
        K_NOTES => String::from("Notizen"),
        K_LOG => String::from("Protokoll"),
        _ => format!("Fenster {}", k - K_EXTRA + 1),
    }
}

impl Desk {
    // "Z: Editor, Rechner=x, [Protokoll]"
    fn describe(&self) -> String {
        let d = &self.desk;
        let v: Vec<String> = d
            .order()
            .iter()
            .filter(|k| self.open.contains(k))
            .map(|&k| {
                let s = d.state(k);
                let mut t = title(k);
                if s & DW_MIN != 0 {
                    t.push_str("=m");
                } else if s & DW_MAX != 0 {
                    t.push_str("=x");
                } else if s & DW_LEFT != 0 {
                    t.push_str("=l");
                } else if s & DW_RIGHT != 0 {
                    t.push_str("=r");
                }
                if d.active() == k { format!("[{}]", t) } else { t }
            })
            .collect();
        format!("Z: {}", if v.is_empty() { String::from("-") } else { v.join(", ") })
    }

    fn log(&mut self, s: String) {
        self.log.push(s);
        if self.log.len() > 6 {
            self.log.remove(0);
        }
    }

    // (tests/touch.uis) a widget that takes one finger or two: what ui.touch_two says, as text
    fn two_fingers(&mut self, ui: &mut Ui) {
        ui.next_size(SIZE_FILL, 40);
        ui.box_begin(ID_TWO, BOX_V | BOX_FIXED, 0);
        let ev = ui.behave(ID_TWO, HB_GRAB_H | HB_GRAB_V);
        ui.sem(K_TEXT, ID_TWO, Some("zwei"), 0);
        ui.touch_take(TF_BOTH | TF_SLOP | TF_TWO);
        if let Some(t) = ui.touch_two() {
            self.two = format!("zwei: {},{} von {},{} mal {} {}{}", t.x, t.y, t.start_x, t.start_y, (t.scale * 100.0) as i32, t.began as i32, t.taken_back as i32);
        }
        if ev & EV_ACT != 0 {
            self.two = String::from("ein Klick");
        } else if ev & EV_DRAG != 0 {
            self.two = String::from("ein Finger zieht");
        }
        let t = self.two.clone();
        ui.text(TS_BODY, &t);
        ui.box_end();
    }

    fn content(&mut self, ui: &mut Ui, k: u32) {
        ui.box_begin(0, BOX_V | PAD_12, 8);
        match k {
            K_EDITOR => {
                ui.textbox(ID_EDIT, Some("Text"), &mut self.text, 200);
                if ui.button(ID_APPLY, "Übernehmen") & EV_ACT != 0 {
                    let t = self.text.clone();
                    self.log(format!("Übernommen: {}", t));
                }
            }
            K_CALC => {
                ui.text(TS_BODY_STRONG, &format!("Anzeige: {}", self.display));
                ui.gridbox_begin(0, &[-1, -1, -1], 0, 4);
                for d in 1..10u32 {
                    if ui.button(ID_DIGIT + d, &format!("{}", d)) & EV_ACT != 0 {
                        if self.display == "0" {
                            self.display.clear();
                        }
                        self.display.push_str(&format!("{}", d));
                    }
                }
                ui.gridbox_end();
            }
            K_NOTES => {
                ui.text(TS_BODY | TX_WRAP, "Notizen zum Labor");
                ui.checkbox(ID_IMPORTANT, "Wichtig", &mut self.important);
            }
            K_LOG => {
                if self.log.is_empty() {
                    ui.text(TS_CAPTION | TX_SECONDARY, "Noch nichts");
                }
                for l in self.log.clone() {
                    ui.text(TS_CAPTION, &l);
                }
            }
            _ => {
                let n = k - K_EXTRA;
                ui.text(TS_BODY, &title(k));
                if ui.button(ID_EXTRA + n, &format!("Klick {}", n + 1)) & EV_ACT != 0 {
                    self.clicks += 1;
                }
            }
        }
        ui.box_end();
    }

    // the screen readers' tree of the main window: each window's node, the number of nodes in
    // it, and which window three content widgets hang in
    fn tree(&self, ui: &Ui) -> String {
        let snap = ui.a11y_tree();
        let Some(w) = snap.wins.iter().find(|w| w.win == 0) else { return String::from("Baum: -") };
        let inside = |mut i: u32, top: u32| {
            while i != NONE {
                if i == top {
                    return true;
                }
                i = w.nodes[i as usize].parent;
            }
            false
        };
        let mut parts: Vec<String> = Vec::new();
        for (d, n) in w.nodes.iter().enumerate() {
            if n.role != Role::Dialog {
                continue;
            }
            let count = (0..w.nodes.len() as u32).filter(|&i| i != d as u32 && inside(i, d as u32)).count();
            let sel = if n.state & plat::a11y::S_SELECTED != 0 { "*" } else { "" };
            parts.push(format!("{}{}({})", n.name, sel, count));
        }
        let owner = |name: &str| {
            let Some(i) = w.nodes.iter().position(|n| n.name == name) else { return String::from("?") };
            let mut p = w.nodes[i].parent;
            while p != NONE {
                if w.nodes[p as usize].role == Role::Dialog {
                    return w.nodes[p as usize].name.clone();
                }
                p = w.nodes[p as usize].parent;
            }
            String::from("-")
        };
        format!("Baum: {} | Übernehmen→{} 5→{} Wichtig→{}", parts.join(" "), owner("Übernehmen"), owner("5"), owner("Wichtig"))
    }
}

impl App for Desk {
    fn init(&mut self, ui: &mut Ui) -> bool {
        ui.styles_add(&[&style_netlab::STYLE]);
        let args = plat::args();
        if let Some(i) = args.iter().position(|a| a == "--state") {
            if let Some(f) = args.get(i + 1) {
                if let Some(d) = plat::read_file(f).and_then(|b| DeskState::load(&String::from_utf8_lossy(&b))) {
                    let r = self.desk.reserve();
                    self.desk = d;
                    self.desk.set_reserve(r);
                }
                self.file = Some(f.clone());
            }
        }
        if args.iter().any(|a| a == "--check-fullscreen") {
            self.check = 1;
            ui.timer(CHECK_TIMER, CHECK_MS);
        }
        true
    }

    fn frame(&mut self, ui: &mut Ui) {
        ui.next_size(SIZE_FILL, SIZE_FILL);
        ui.box_begin(0, BOX_V, 0);
        if plat::env("jfgui_DESKTEST_HOLD").is_some() {
            self.two_fingers(ui);
        }
        ui.box_begin(0, BOX_H | BOX_XCENTER | PAD_8 | BOX_WRAP, 8);
        if ui.button(ID_NEW, "Neues Fenster") & EV_ACT != 0 {
            ui.cmd_run(CMD_NEW, 0);
        }
        if ui.button(ID_SAVE, "Speichern") & EV_ACT != 0 {
            self.saved = self.desk.save();
            if let Some(f) = &self.file {
                plat::write_file(f, self.saved.as_bytes());
            }
        }
        if ui.button(ID_LOAD, "Laden") & EV_ACT != 0 {
            if let Some(d) = DeskState::load(&self.saved) {
                let r = self.desk.reserve();
                self.desk = d;
                self.desk.set_reserve(r);
            }
        }
        if ui.button(ID_RESET, "Zurücksetzen") & EV_ACT != 0 {
            self.desk.reset();
        }
        let mut full = (ui.window_state(0) & UWS_FULLSCREEN != 0) as i32;
        if ui.toggle_button(ID_FULL, "Vollbild", &mut full) & EV_ACT != 0 {
            ui.cmd_run(CMD_FULL, 0);
        }
        let mut tool = self.tool_open.get();
        if ui.toggle_button(ID_TOOLWIN, "Werkzeug", &mut tool) & EV_ACT != 0 {
            self.tool_open.set(tool);
        }
        let mut toolfull = (ui.window_state(ID_TOOL) & UWS_FULLSCREEN != 0) as i32;
        if ui.toggle_button(ID_TOOLFULL, "Werkzeug-Vollbild", &mut toolfull) & EV_ACT != 0 {
            ui.window_fullscreen(ID_TOOL, toolfull != 0);
        }
        if ui.button(ID_TREE, "Baum") & EV_ACT != 0 {
            ui.cmd_run(CMD_TREE, 0);
        }
        ui.toggle_button(ID_ICONS, "Nur Symbole", &mut self.icons);
        ui.box_end();
        // the window area with its taskbar
        let titles: Vec<(u32, String)> = self.open.iter().map(|&k| (k, title(k))).collect();
        let wins: Vec<DeskWin> = titles
            .iter()
            .map(|(k, t)| match *k {
                K_EDITOR => DeskWin::new(*k, t, IC_EDIT).size(380, 180),
                K_CALC => DeskWin::new(*k, t, IC_CALCULATOR).flags(DWF_FIXED).size(220, 210),
                K_NOTES => DeskWin::new(*k, t, IC_DOCUMENT).min(240, 120).size(300, 160),
                K_LOG => DeskWin::new(*k, t, IC_HISTORY).size(320, 200),
                _ if self.maxed.contains(k) => DeskWin::new(*k, t, IC_GRID).flags(DWF_MAXIMIZED).size(260, 140),
                _ => DeskWin::new(*k, t, IC_GRID).size(260, 140),
            })
            .collect();
        ui.next_size(SIZE_FILL, SIZE_FILL);
        ui.desk_begin(ID_DESK, &mut self.desk, &wins);
        while let Some(k) = ui.desk_next(&mut self.desk) {
            self.content(ui, k);
        }
        ui.desk_taskbar_begin(ID_DESK);
        ui.next_icon(IC_HOME);
        if ui.button(ID_START, "Start") & EV_ACT != 0 {
            self.log(String::from("Start"));
        }
        ui.desk_taskbar(ID_DESK, &mut self.desk, if self.icons != 0 { DTB_ICONS } else { 0 });
        ui.text(TS_CAPTION, "12:00");
        ui.desk_taskbar_end();
        for ev in ui.desk_end(&mut self.desk) {
            let (n, k, what) = match ev {
                DeskEv::Close(k) => (0, k, "geschlossen"),
                DeskEv::Activate(k) => (1, k, "aktiv"),
                DeskEv::Move(k) => (2, k, "bewegt"),
                DeskEv::Resize(k) => (3, k, "Größe"),
                DeskEv::State(k) => (4, k, "Zustand"),
            };
            self.evs[n] += 1;
            if let DeskEv::Close(k) = ev {
                self.open.retain(|&o| o != k);
                self.desk.forget(k);
            }
            self.log(format!("{} {}", title(k), what));
        }
        // the status
        ui.box_begin(0, BOX_V | pad(8), 2);
        ui.text(TS_CAPTION, &self.describe());
        let a = self.desk.active();
        let act = match self.desk.rect(a) {
            Some(r) if a != 0 => format!("Aktiv: {} {},{} {}x{}", title(a), r[0], r[1], r[2], r[3]),
            _ => String::from("Aktiv: -"),
        };
        ui.text(TS_CAPTION, &act);
        let e = self.evs;
        ui.text(TS_CAPTION, &format!("Ereignisse: Schließen {} | Aktiv {} | Bewegt {} | Größe {} | Zustand {} | Klicks {}", e[0], e[1], e[2], e[3], e[4], self.clicks));
        let on = |b: bool| if b { "an" } else { "aus" };
        let (fm, ft) = (ui.window_state(0) & UWS_FULLSCREEN != 0, ui.window_state(ID_TOOL) & UWS_FULLSCREEN != 0);
        ui.text(TS_CAPTION, &format!("Vollbild: Haupt {} | Werkzeug {} | Meldungen {}", on(fm), on(ft), self.fs_events));
        if !self.tree.is_empty() {
            ui.text(TS_CAPTION, &self.tree.clone());
        }
        if !self.saved.is_empty() {
            let lines: Vec<&str> = self.saved.lines().collect();
            ui.text(TS_CAPTION, &format!("Gespeichert: {}", lines.join(" | ")));
        }
        ui.box_end();
        ui.box_end();
        // the tool window
        let (wo, wr) = (self.tool_open.clone(), self.tool_rect.clone());
        // (tests/touch.uis: a window a finger moves only after resting on its title bar)
        let hold = if plat::env("jfgui_DESKTEST_HOLD").is_some() { WF_TOUCH_HOLD } else { 0 };
        if ui.window_begin(ID_TOOL, "Werkzeug", Some(&wo), WF_TOOL | WF_RESIZABLE | hold, Some(&wr)) {
            ui.box_begin(0, BOX_V | PAD_12, 8);
            ui.text(TS_BODY, "Ein Werkzeugfenster");
            if ui.button(ID_TOOLTEXT, "Vollbild umschalten") & EV_ACT != 0 {
                let on = ui.window_state(ID_TOOL) & UWS_FULLSCREEN == 0;
                ui.window_fullscreen(ID_TOOL, on);
            }
            ui.box_end();
        }
        ui.window_end();
    }

    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        match id {
            CMD_FULL => {
                let on = ui.window_state(0) & UWS_FULLSCREEN == 0;
                ui.window_fullscreen(0, on);
            }
            CMD_NEW | CMD_NEW_MAX => {
                let k = self.next_extra;
                self.next_extra += 1;
                if id == CMD_NEW_MAX {
                    self.maxed.push(k);
                }
                self.open.push(k);
            }
            CMD_TREE => self.tree = self.tree(ui),
            UIC_FULLSCREEN => {
                self.fs_events += 1;
                let t = if arg == ID_TOOL { "Werkzeug" } else { "Haupt" };
                self.log(format!("Vollbild {}: {}", t, if ui.window_state(arg) & UWS_FULLSCREEN != 0 { "an" } else { "aus" }));
            }
            UIC_TIMER if arg == CHECK_TIMER && self.check != 0 => self.check_step(ui),
            _ => return false,
        }
        true
    }
}

impl Desk {
    // --check-fullscreen with a real window: the main window and the tool window into full
    // screen and back; the platform must report it and cover the monitor, then give the old
    // size back (stdout: a line per check, exit code 1 when one failed)
    fn check_step(&mut self, ui: &mut Ui) {
        let say = |s: &str| plat::write_stdout(format!("fullscreen check: {}\n", s).as_bytes());
        let mut ok = |c: bool, what: &str| {
            say(&format!("{} {}", if c { "ok  " } else { "FAIL" }, what));
            if !c {
                self.check_fails += 1;
            }
        };
        let near = |a: (i32, i32), b: (i32, i32)| (a.0 - b.0).abs() <= 2 && (a.1 - b.1).abs() <= 2;
        let plat_fs = |w: i32| plat::win_state(w) & plat::WST_FULLSCREEN != 0;
        let mon = monitor_size();
        match self.check {
            1 => {
                self.check_size = plat::win_size(0);
                say(&format!("window {:?}, monitor {:?}", self.check_size, mon));
                ui.window_fullscreen(0, true);
            }
            2 => {
                ok(ui.window_state(0) & UWS_FULLSCREEN != 0, "main: window_state UWS_FULLSCREEN");
                ok(plat_fs(0), "main: plat WST_FULLSCREEN");
                let s = plat::win_size(0);
                ok(near(s, mon), &format!("main: covers the monitor ({:?} / {:?})", s, mon));
                ui.window_fullscreen(0, false);
            }
            3 => {
                ok(ui.window_state(0) & UWS_FULLSCREEN == 0 && !plat_fs(0), "main: windowed again");
                let s = plat::win_size(0);
                ok(near(s, self.check_size), &format!("main: its size back ({:?} / {:?})", s, self.check_size));
                ok(self.fs_events == 0, "main: no UIC_FULLSCREEN for the app's own change");
                self.tool_open.set(1);
            }
            4 => {
                let r = self.tool_rect.get();
                self.check_size = (r.w, r.h);
                ui.window_fullscreen(ID_TOOL, true);
                if ui.window_state(ID_TOOL) & UWS_FULLSCREEN == 0 {
                    // refused at once: a tool window drawn inside its owner (Wayland)
                    say("skip tool: no full screen of its own here (a child window)");
                    self.check = 6;
                    self.tool_open.set(0);
                    plat::win_cmd(0, plat::WCMD_MAXIMIZE);
                }
            }
            5 => {
                ok(ui.window_state(ID_TOOL) & UWS_FULLSCREEN != 0, "tool: window_state UWS_FULLSCREEN");
                let r = ui.window_rect(ID_TOOL).unwrap_or_default();
                let d = (ui.dp(r.w), ui.dp(r.h));
                ok(near(d, mon), &format!("tool: covers the monitor ({:?} px / {:?})", d, mon));
                ui.window_fullscreen(ID_TOOL, false);
            }
            6 => {
                let r = ui.window_rect(ID_TOOL).unwrap_or_default();
                ok(ui.window_state(ID_TOOL) & UWS_FULLSCREEN == 0, "tool: windowed again");
                ok(near((r.w, r.h), self.check_size), &format!("tool: its size back ({},{} / {:?} DIP)", r.w, r.h, self.check_size));
                // a maximised main window: full screen and back to maximised
                self.tool_open.set(0);
                plat::win_cmd(0, plat::WCMD_MAXIMIZE);
            }
            7 => {
                self.check_size = plat::win_size(0);
                let max = plat::win_state(0) & plat::WST_MAXIMIZED != 0;
                if (!max && self.check_wait < 5) || self.check_wait == 0 {
                    // (a compositor may take its time: WSLg's host maximises after Weston said
                    // so, and a request for full screen before that is lost)
                    self.check_wait += 1;
                    ui.timer(CHECK_TIMER, CHECK_MS);
                    return;
                }
                say(&format!("maximised: {} {:?}", max, self.check_size));
                ui.window_fullscreen(0, true);
            }
            8 => {
                ok(plat_fs(0), "maximised main: plat WST_FULLSCREEN");
                let s = plat::win_size(0);
                ok(near(s, mon), &format!("maximised main: covers the monitor ({:?} / {:?})", s, mon));
                ui.window_fullscreen(0, false);
            }
            _ => {
                let st = plat::win_state(0);
                ok(st & plat::WST_FULLSCREEN == 0, "maximised main: windowed again");
                ok(st & plat::WST_MAXIMIZED != 0, "maximised main: maximised again");
                let s = plat::win_size(0);
                ok(near(s, self.check_size), &format!("maximised main: its size back ({:?} / {:?})", s, self.check_size));
                ok(self.fs_events == 0, "no UIC_FULLSCREEN for the app's own changes");
                say(if self.check_fails == 0 { "passed" } else { "failed" });
                plat::exit((self.check_fails != 0) as i32);
            }
        }
        self.check += 1;
        ui.timer(CHECK_TIMER, CHECK_MS);
    }
}

ui::main!(ui::launch(&DESC, Desk::default(), &style::STYLE));
