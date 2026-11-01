// ide -- the IDE of a host (IDE1, plan phase 6): the device's files and its inline scripts in an
// Explorer, tabs of open files with drafts, SK6's language services in the editor, find and
// replace in files, a lower area with Ausgabe, Terminal, Probleme, Debugger and Tests, a file
// run as a process (`jfs FILE` in the terminal) or as a service, SK7's debugger and the test
// functions of a file (`jfs --test`, jfn_sim cli/jfstest.rs); deploying to a group of hosts and
// load tests with their numbers (A2)
//
//   Ides::new()                  the drafts and breakpoints of every window (DevWins::new: a
//                                new project starts without)
//   Pane::new(dev)               the IDE's state in one device window: the open tabs, the side
//                                bar, the lower area, the debugger's choices
//   view(n, ui, k, dev)          the IDE of device window k, filling the box it is given; ids
//                                ID_IDE0 + 0x1000 k .. + 0xFFF (below).  The desktop's IDE
//                                window hosts it (devwin/desktop.rs ide_window, maximised)
//   open_later(n, k, path)       a file of the device as the shown tab, at the IDE's next frame
//                                (the Dateimanager's Öffnen)
//   check_due(n, ui)             TIMER_SCRIPT_CHECK: the check of every changed draft
//   term_session(n, k, dev)      the IDE's command-line session (Desk::ide_term; opened when
//                                first wanted, a new one after the device was off)
//   terminal(n, ui, k, dev)      the Terminal pane: that session (console::view_session); a
//                                process the IDE runs and its tests run there
//   Key                          what a tab holds: a file of the device (canonical path) or the
//                                inline source of a script service
//
// explorer.rs: the Explorer; editor.rs: drafts, tabs, the code editor with its language
// services, the outline; lower.rs: Ausgabe, Terminal, Probleme, Tests; debug.rs: running a file
// or a script, the debugger (breakpoints, the stop, the call stack, variables, watch
// expressions); search.rs: find and replace in files; deploy.rs: Bereitstellen; load.rs:
// Lasttest (ids from ID_A20, Ausführen › Bereitstellen and Lasttest open them, tall enough).
//
// The bar: Datei, Bearbeiten, Ausführen, Debuggen and Ansicht are drop-down menus (Tab goes from
// one to the next; Enter, Space or Alt+Down open them, the arrows go through the rows, each row
// names its keys), then the tool bar: icons with their names as tooltips and for screen
// readers, one Tab stop with the arrows inside (focus_group).  The keys work wherever the focus is in the IDE: Ctrl+S save,
// Ctrl+Shift+S save all, Ctrl+W (Ctrl+F4) close the tab, Ctrl+PgDn / Ctrl+PgUp the next /
// previous tab, Ctrl+Shift+F find in files, Ctrl+Shift+H replace in files, Ctrl+B the side bar,
// Ctrl+J the lower area, Ctrl+Shift+E the Explorer; F5 run as a service (paused: continue),
// Ctrl+F5 run as a process, Shift+F5 stop, Ctrl+Shift+F5 restart, Ctrl+F9 pause, F9 a
// breakpoint on the caret's line, Ctrl+Shift+F9 a conditional one, F10 step over, F7 step into,
// Shift+F7 step out (F11 stays the window's), Ctrl+Shift+T run the file's tests.  While the IDE
// has the keys they win over the main window's (F5, F7 and F9 of the simulation and the lab).
// The editor's own: Ctrl+F / Ctrl+H find in the file, F12 / Ctrl+click the definition (into
// another file through its import), F2 rename, Shift+Alt+F format, F8 the next message, Ctrl+Z
// / Ctrl+Y its undo.  F6 goes from pane to pane, F8 (without messages) to the splitters.
//
// Layout: the side bar (Explorer | Suchen | Gliederung) left of the editor while the area is at
// least SIDE_MIN DIP wide, the lower area below it while it is at least LOWER_MIN DIP high (both
// can be switched off); the splits keep their sizes per window.  Narrow, the tool bar keeps the
// run and debug buttons and the menus have everything.
//
// Ids from id = ID_IDE0 + 0x1000 k: + 1 .. 5 the menus Datei, Bearbeiten, Ausführen, Debuggen,
// Ansicht (their rows + 0x40 ..), + 0x11 .. 0x1C the tool bar (Speichern, Alle speichern, Als
// Prozess / Als Dienst ausführen, Stoppen, Neu starten, Tests ausführen, Fortsetzen, Anhalten,
// Schritt über / hinein / heraus), + 0xC0 Neue Datei with the Explorer's name flyout (+ 0xC1 its
// text, + 0xC2 OK, + 0xC3 Abbrechen), + 0xC8 the flyout of a conditional breakpoint (+ 0xC9 ..
// 0xCB), + 0xD0 the rename flyout of the language services (+ 0xD1 .. 0xD3), + 0x100 the split
// of the side bar, + 0x102 the split of the lower area, + 0x104 the side bar's tabs, + 0x108 the
// lower area's tabs (+ 0x109 + i), + 0x110 .. 0x113 Neuer Ordner, Umbenennen, Löschen, In Datei
// umwandeln, + 0x120 .. 0x126 the search's fields and buttons, + 0x130 .. 0x136 the debugger's,
// + 0x140 Tests ausführen, + 0x150 the editor's tabs (+ 0x151 + i), + 0x180 Tab schließen,
// + 0x181 Neu starten in the hint, + 0x182 Probleme, + 0x183 Formatieren, + 0x200 the editor
// (.. + 0x20F), + 0x220 the output, + 0x240 the outline, + 0x300 the problems, + 0x400 the
// search results, + 0x500 the call stack, + 0x580 the watch expressions, + 0x600 the variables,
// + 0x700 the tests, + 0x800 the Explorer's tree (rows + 1 + i each).

pub(crate) mod debug;
pub(crate) mod deploy;
pub(crate) mod editor;
pub(crate) mod explorer;
pub(crate) mod load;
pub(crate) mod lower;
pub(crate) mod search;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::files::Store;
use jfn_model::Device;
use jfn_sim::{ScriptService, ScriptState, ScriptView};
use script::{Pause, Step};
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::{ide as dide, scripts};
use crate::workspace::icons::JN_TERMINAL;

/// the side bar from this width of the IDE on (DIP)
const SIDE_MIN: i32 = 520;
/// the lower area from this height on (DIP)
const LOWER_MIN: i32 = 300;

/// the side bar's views
pub(crate) const SIDE_EXPLORER: i32 = 0;
pub(crate) const SIDE_SEARCH: i32 = 1;
pub(crate) const SIDE_OUTLINE: i32 = 2;
/// the lower area's panes
pub(crate) const LOW_OUTPUT: i32 = 0;
pub(crate) const LOW_TERMINAL: i32 = 1;
pub(crate) const LOW_PROBLEMS: i32 = 2;
pub(crate) const LOW_DEBUG: i32 = 3;
pub(crate) const LOW_TESTS: i32 = 4;
pub(crate) const LOW_DEPLOY: i32 = 5;
pub(crate) const LOW_LOAD: i32 = 6;

/// what a tab holds
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Key {
    /// a file of the device (canonical path)
    File(String),
    /// the source of script service NAME, inline in its definition
    Inline(String),
}

impl Key {
    /// the tab's title: the file's name, "NAME (eingebettet)"
    pub(crate) fn title(&self) -> String {
        match self {
            Key::File(p) => String::from(p.rsplit('/').next().unwrap_or(p)),
            Key::Inline(n) => tf("{} (eingebettet)", &[n]),
        }
    }
    /// the file a breakpoint or a stop names: the path, "" for the service's own source
    pub(crate) fn debug_file(&self) -> &str {
        match self {
            Key::File(p) => p,
            Key::Inline(_) => "",
        }
    }
    /// a jfs program (the language services, the check): .jfs files and inline scripts
    pub(crate) fn jfs(&self) -> bool {
        match self {
            Key::File(p) => p.ends_with(".jfs"),
            Key::Inline(_) => true,
        }
    }
    /// the folder relative imports start in ("/root" for an inline script, as the service's
    /// host reads them)
    pub(crate) fn dir(&self) -> String {
        match self {
            Key::File(p) => match p.rfind('/') {
                Some(0) | None => String::from("/"),
                Some(i) => String::from(&p[..i]),
            },
            Key::Inline(_) => String::from("/root"),
        }
    }
}

/// a path written in an import (or a search) made absolute against folder `dir`, "." and ".."
/// resolved
pub(crate) fn resolve(dir: &str, path: &str) -> String {
    let full = if path.starts_with('/') { String::from(path) } else { format!("{}/{}", dir.trim_end_matches('/'), path) };
    let mut parts: Vec<&str> = Vec::new();
    for c in full.split(['/', '\\']) {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            c => parts.push(c),
        }
    }
    format!("/{}", parts.join("/"))
}

/// the drafts and breakpoints of every device window (they outlive a closed window)
pub(crate) struct Ides {
    pub(crate) drafts: BTreeMap<(u32, Key), editor::Draft>,
    /// the breakpoints by device and tab: line (from 1), condition
    pub(crate) bps: BTreeMap<(u32, Key), Vec<(u32, Option<String>)>>,
    /// the check timer is set
    pub(crate) pending: bool,
}

impl Ides {
    pub(crate) fn new() -> Ides {
        Ides { drafts: BTreeMap::new(), bps: BTreeMap::new(), pending: false }
    }
}

/// the Explorer's name flyout: what it makes
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AskFor {
    NewFile(String),
    NewDir(String),
    Rename(String),
}

/// the IDE's state in one device window
pub(crate) struct Pane {
    pub(crate) dev: u32,
    pub(crate) tabs: Vec<Key>,
    pub(crate) cur: usize,
    pub(crate) side_on: bool,
    pub(crate) side: i32,
    pub(crate) side_w: i32,
    pub(crate) lower_on: bool,
    pub(crate) lower: i32,
    pub(crate) lower_h: i32,
    pub(crate) tree: TreeState,
    pub(crate) tree_init: bool,
    /// the name flyout: what for, its text, a refusal
    pub(crate) ask: Option<(AskFor, String, Option<String>)>,
    /// the last refusal or note (the status line)
    pub(crate) msg: Option<String>,
    /// the editor gets the keyboard until then (ms of ui.now()) and then the caret goes to
    /// line / column of the shown tab
    pub(crate) focus_until: u32,
    pub(crate) goto: Option<(usize, usize)>,
    /// the service the IDE ran last (the debugger's when the shown tab names none)
    pub(crate) target: Option<String>,
    /// the output pane: the service and run it shows, where it is, whether it shows anything
    pub(crate) out: TermState,
    pub(crate) out_of: Option<(String, u64)>,
    pub(crate) out_at: u64,
    pub(crate) out_any: bool,
    pub(crate) search: search::Search,
    pub(crate) dbg: debug::DebugUi,
    pub(crate) probs: i32,
    pub(crate) tests: i32,
    /// the rename flyout of the language services: the new name, where, a refusal
    pub(crate) ren: Option<String>,
    pub(crate) ren_at: TextPos,
    pub(crate) ren_msg: Option<String>,
    /// the focused widget of the last frame
    pub(crate) last_focus: u32,
    /// a file to show as a tab at the next frame (open_later)
    pub(crate) open_req: Option<String>,
    /// the frames the flyouts were open (the name, the rename, the condition): a text box's
    /// commit in the first one is an old one's (its edit state went to the editor)
    pub(crate) fly: [u32; 3],
    /// the panes Bereitstellen and Lasttest (A2)
    pub(crate) deploy: deploy::DeployUi,
    pub(crate) load: load::LoadUi,
}

impl Pane {
    pub(crate) fn new(dev: u32) -> Pane {
        Pane {
            dev,
            tabs: Vec::new(),
            cur: 0,
            side_on: true,
            side: SIDE_EXPLORER,
            side_w: 260,
            lower_on: true,
            lower: LOW_OUTPUT,
            lower_h: 170,
            tree: TreeState::default(),
            tree_init: false,
            ask: None,
            msg: None,
            focus_until: 0,
            goto: None,
            target: None,
            out: TermState::new(2000),
            out_of: None,
            out_at: 0,
            out_any: false,
            search: search::Search::new(),
            dbg: debug::DebugUi::new(),
            probs: -1,
            tests: -1,
            ren: None,
            ren_at: TextPos::default(),
            ren_msg: None,
            last_focus: 0,
            open_req: None,
            fly: [0; 3],
            deploy: deploy::DeployUi::new(),
            load: load::LoadUi::new(),
        }
    }
    /// the shown tab
    pub(crate) fn cur_key(&self) -> Option<&Key> {
        self.tabs.get(self.cur)
    }
}

/// what one frame of the IDE knows of the project and the simulator
pub(crate) struct Cx {
    pub(crate) dev: u32,
    pub(crate) d: Device,
    pub(crate) st: Store,
    pub(crate) cur: Option<Key>,
    /// the service to run and debug: the shown tab's (the unit that holds its file comes first),
    /// else the one run last
    pub(crate) target: Option<String>,
    pub(crate) view: Option<ScriptView>,
    pub(crate) paused: Option<Pause>,
    /// the IDE's terminal runs a command (a process holds it)
    pub(crate) busy: bool,
    /// the IDE's terminal session (Desk::ide_term) once it was opened
    pub(crate) term: Option<u32>,
}

impl Cx {
    pub(crate) fn of(n: &mut Net, k: usize, d: &Device, st: &Store) -> Option<Cx> {
        let p = &n.devwin.get(k)?.ide;
        let cur = p.cur_key().cloned();
        let e = n.doc.sim.engine();
        let last = p.target.clone().filter(|t| scripts::find(d, t).is_some() || e.unit(d.id, t).is_some_and(|u| u.file.is_some()));
        let target = match &cur {
            Some(Key::Inline(name)) => Some(name.clone()),
            Some(Key::File(path)) => match dide::unit_of(e, d, st, path, last.as_deref()) {
                Some(u) => Some(u),
                None => {
                    let b = dide::bound(d, st, path);
                    match &last {
                        Some(l) if b.contains(l) => Some(l.clone()),
                        _ => b.into_iter().next().or(last),
                    }
                }
            },
            None => last,
        };
        let svc = target.as_ref().and_then(|t| e.service::<ScriptService>(d.id, t));
        let view = svc.map(|s| s.view(e.now()));
        let paused = svc.and_then(|s| s.paused());
        // (the IDE's own terminal: a process it runs holds it)
        let term = n.devwin.peek(k).and_then(|w| w.desk.ide_term);
        let busy = term.is_some_and(|s| e.cli_sessions(d.id).iter().any(|x| x.id == s) && e.cli_ses_busy(d.id, s));
        Some(Cx { dev: d.id, d: d.clone(), st: st.clone(), cur, target, view, paused, busy, term })
    }
    pub(crate) fn running(&self) -> bool {
        self.view.as_ref().is_some_and(|v| v.state == ScriptState::Running)
    }
}

/// what the IDE's widgets asked for (done after the frame's widgets)
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Act {
    Open(Key),
    OpenAt(Key, usize, usize),
    Close(usize),
    Tab(usize),
    Save,
    SaveAll,
    Ask(AskFor),
    Make(AskFor, String),
    Template(String, usize),
    Delete(Key),
    Convert(String),
    RunProcess,
    RunService,
    Stop,
    Restart,
    Tests,
    Dbg(debug::Op),
    DeployStart,
    DeployAbort,
    DeployUnit,
    LoadStart,
    LoadStop,
    Bp(usize, Option<String>),
    CondBp,
    ClearBps,
    Find(bool),
    FindInFiles(bool),
    ReplaceAll,
    Side(i32),
    ToggleSide,
    Lower(i32),
    ToggleLower,
    Format,
    RenameName,
    Definition,
}

/// the IDE of device window k (see the module comment)
pub(crate) fn view(n: &mut Net, ui: &mut Ui, k: usize, dev: u32) {
    let id = ID_IDE0 + 0x1000 * k as u32;
    let Some(d) = n.doc.proj.device(dev).cloned() else { return };
    let Some(st) = dide::store_of(&n.doc.proj, dev) else { return };
    match n.devwin.get(k) {
        Some(w) if w.ide.dev != dev => w.ide = Pane::new(dev),
        Some(_) => {}
        None => return,
    }
    // a device window is a tool window: a click or a flyout that gives a text box of the IDE,
    // its editor or its terminal the focus makes it the active window too, so they take what is
    // typed
    let f = ui.focused();
    let win = ID_DEVWIN0 + 0x200 * k as u32;
    let term = match n.devwin.peek(k).and_then(|w| w.desk.ide_term) {
        Some(s) => n.cons.ses_id(&n.doc.proj, dev, s),
        None => 0,
    };
    let a2 = ID_A20 + 0x800 * k as u32;
    let texty = f == term || [0xC1, 0xC9, 0xD1, 0x120, 0x121, 0x130, 0x200].iter().any(|o| f == id + o) || (a2..a2 + 0x800).contains(&f);
    if let Some(w) = n.devwin.get(k) {
        if texty && f != w.ide.last_focus && ui.window_state(win) & UWS_ACTIVE == 0 {
            ui.window_activate(win);
        }
        w.ide.last_focus = f;
    }
    editor::follow(n, ui, k, &d, &st);
    let Some(mut cx) = Cx::of(n, k, &d, &st) else { return };
    // a file the desktop asked for (the Dateimanager's Öffnen)
    if let Some(path) = n.devwin.get(k).and_then(|w| w.ide.open_req.take()) {
        if let Err(e) = editor::open(n, ui, k, &cx, Key::File(path), None)
            && let Some(w) = n.devwin.get(k)
        {
            w.ide.msg = Some(e);
        }
        let Some(c) = Cx::of(n, k, &d, &st) else { return };
        cx = c;
    }
    let mut acts: Vec<Act> = Vec::new();
    keys(n, ui, k, id, &cx, &mut acts);
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 4);
    bar(n, ui, k, id, &cx, &mut acts);
    body(n, ui, k, id, &cx, &mut acts);
    ui.box_end();
    if !acts.is_empty() {
        act(n, ui, k, &cx, acts);
        ui.repass();
    }
    debug::sync(n, ui, k, dev);
}

/// the IDE's keys (see the module comment), wherever its focus is
fn keys(n: &mut Net, ui: &mut Ui, k: usize, id: u32, cx: &Cx, acts: &mut Vec<Act>) {
    // (the keys of this window only reach its pass while it has them; Ctrl and letters stay
    // the terminal's while it has the focus, the function keys are the IDE's)
    let f = ui.focused();
    let ours = f == 0 || (id..id + 0x1000).contains(&f);
    let paused = cx.paused.is_some();
    let mut key = |vk: u32, m: u32, a: Act| {
        let letter = (b'A' as u32..=b'Z' as u32).contains(&vk);
        if (ours || !letter) && ui.capture_shortcut(vk, m) {
            acts.push(a);
        }
    };
    key(b'S' as u32, MOD_CTRL, Act::Save);
    key(b'S' as u32, MOD_CTRL | MOD_SHIFT, Act::SaveAll);
    key(b'W' as u32, MOD_CTRL, Act::Close(usize::MAX));
    key(VK_F4, MOD_CTRL, Act::Close(usize::MAX));
    key(VK_NEXT, MOD_CTRL, Act::Tab(1));
    key(VK_PRIOR, MOD_CTRL, Act::Tab(usize::MAX));
    key(b'F' as u32, MOD_CTRL | MOD_SHIFT, Act::FindInFiles(false));
    key(b'H' as u32, MOD_CTRL | MOD_SHIFT, Act::FindInFiles(true));
    key(b'B' as u32, MOD_CTRL, Act::ToggleSide);
    key(b'J' as u32, MOD_CTRL, Act::ToggleLower);
    key(b'E' as u32, MOD_CTRL | MOD_SHIFT, Act::Side(SIDE_EXPLORER));
    key(b'T' as u32, MOD_CTRL | MOD_SHIFT, Act::Tests);
    key(VK_F5, 0, if paused { Act::Dbg(debug::Op::Continue) } else { Act::RunService });
    key(VK_F5, MOD_CTRL, Act::RunProcess);
    key(VK_F5, MOD_SHIFT, Act::Stop);
    key(VK_F5, MOD_CTRL | MOD_SHIFT, Act::Restart);
    key(VK_F9, MOD_CTRL, Act::Dbg(debug::Op::Pause));
    key(VK_F9, 0, Act::Bp(usize::MAX, None));
    key(VK_F9, MOD_CTRL | MOD_SHIFT, Act::CondBp);
    key(VK_F10, 0, Act::Dbg(debug::Op::Step(Step::Over)));
    key(VK_F7, 0, Act::Dbg(debug::Op::Step(Step::Into)));
    key(VK_F7, MOD_SHIFT, Act::Dbg(debug::Op::Step(Step::Out)));
    let _ = (n, k);
}

/// a key as a menu row writes it ("" for none)
fn kt(ui: &Ui, vk: u32, m: u32) -> String {
    if vk == 0 { String::new() } else { ui.key_text(vk, m) }
}

// a row of a menu: label, keys, enabled -> the act
#[allow(clippy::too_many_arguments)]
fn row(ui: &mut Ui, id: u32, icon: u32, label: &str, keys: (u32, u32), on: bool, checked: bool, acts: &mut Vec<Act>, a: Act) {
    let keys = kt(ui, keys.0, keys.1);
    if icon != 0 {
        ui.next_icon(icon);
    }
    ui.next_flags(if on { 0 } else { ST_DISABLED } | if checked { ST_CHECKED } else { 0 });
    if ui.menu_item_keys(id, label, &keys) & EV_ACT != 0 {
        acts.push(a);
    }
}

// the menus and the tool bar
fn bar(n: &mut Net, ui: &mut Ui, k: usize, id: u32, cx: &Cx, acts: &mut Vec<Act>) {
    let Some(p) = n.devwin.get(k).map(|w| &w.ide) else { return };
    let (side_on, side, lower_on, lower) = (p.side_on, p.side, p.lower_on, p.lower);
    let has_tab = cx.cur.is_some();
    let file = matches!(cx.cur, Some(Key::File(_)));
    let jfs = cx.cur.as_ref().is_some_and(|c| c.jfs());
    let changed = cx.cur.as_ref().is_some_and(|c| n.devwin.ide.drafts.get(&(cx.dev, c.clone())).is_some_and(|d| d.changed()));
    let any_changed = n.devwin.ide.drafts.iter().any(|((dv, _), d)| *dv == cx.dev && d.changed());
    let inline = matches!(cx.cur, Some(Key::Inline(_)));
    let has_svc = cx.view.is_some();
    let running = cx.running();
    let paused = cx.paused.is_some();
    let off = cx.d.off;
    let m = id + 0x40;
    ui.box_begin(0, BOX_H | BOX_XCENTER, 2);
    // Datei
    ui.next_flags(ST_COMPACT | ST_SUBTLE);
    ui.dropdown_button(id + 1, t("Datei"));
    if ui.flyout_begin(id + 1, FL_BELOW) {
        let dir = explorer::chosen_dir(n, k, &cx.st);
        row(ui, m, IC_ADD, t("Neue Datei …"), (0, 0), true, false, acts, Act::Ask(AskFor::NewFile(dir.clone())));
        row(ui, m + 1, IC_NEW_FOLDER, t("Neuer Ordner …"), (0, 0), true, false, acts, Act::Ask(AskFor::NewDir(dir.clone())));
        // a new .jfs file from one of the templates
        for (i, tpl) in scripts::TEMPLATES.iter().enumerate() {
            let label = tf("Neues Skript: {}", &[t(tpl.title)]);
            row(ui, m + 8 + i as u32, IC_DOCUMENT, &label, (0, 0), true, false, acts, Act::Template(dir.clone(), i));
        }
        ui.menu_sep();
        row(ui, m + 2, IC_SAVE, t("Speichern"), (b'S' as u32, MOD_CTRL), changed, false, acts, Act::Save);
        row(ui, m + 3, IC_SAVE_COPY, t("Alle speichern"), (b'S' as u32, MOD_CTRL | MOD_SHIFT), any_changed, false, acts, Act::SaveAll);
        row(ui, m + 4, IC_CANCEL, t("Tab schließen"), (b'W' as u32, MOD_CTRL), has_tab, false, acts, Act::Close(usize::MAX));
        ui.menu_sep();
        if let Some(Key::Inline(name)) = &cx.cur {
            row(ui, m + 5, IC_SAVE_AS, t("In Datei umwandeln"), (0, 0), true, false, acts, Act::Convert(name.clone()));
        }
        if let Some(Key::File(path)) = &cx.cur {
            row(ui, m + 6, IC_EDIT, t("Umbenennen …"), (0, 0), true, false, acts, Act::Ask(AskFor::Rename(path.clone())));
        }
        if let Some(c) = &cx.cur {
            row(ui, m + 7, IC_DELETE, t("Löschen"), (0, 0), true, false, acts, Act::Delete(c.clone()));
        }
    }
    ui.flyout_end();
    // Bearbeiten
    ui.next_flags(ST_COMPACT | ST_SUBTLE);
    ui.dropdown_button(id + 2, t("Bearbeiten"));
    if ui.flyout_begin(id + 2, FL_BELOW) {
        row(ui, m + 0x10, IC_SEARCH, t("Suchen …"), (b'F' as u32, MOD_CTRL), has_tab, false, acts, Act::Find(false));
        row(ui, m + 0x11, 0, t("Ersetzen …"), (b'H' as u32, MOD_CTRL), has_tab, false, acts, Act::Find(true));
        row(ui, m + 0x12, 0, t("In Dateien suchen"), (b'F' as u32, MOD_CTRL | MOD_SHIFT), true, false, acts, Act::FindInFiles(false));
        row(ui, m + 0x13, 0, t("In Dateien ersetzen"), (b'H' as u32, MOD_CTRL | MOD_SHIFT), true, false, acts, Act::FindInFiles(true));
        ui.menu_sep();
        row(ui, m + 0x14, 0, t("Gehe zur Definition"), (VK_F12, 0), jfs, false, acts, Act::Definition);
        row(ui, m + 0x15, IC_EDIT, t("Name umbenennen …"), (VK_F2, 0), jfs, false, acts, Act::RenameName);
        row(ui, m + 0x16, IC_ALIGN_LEFT, t("Formatieren"), (b'F' as u32, MOD_SHIFT | MOD_ALT), jfs, false, acts, Act::Format);
    }
    ui.flyout_end();
    // Ausführen
    ui.next_flags(ST_COMPACT | ST_SUBTLE);
    ui.dropdown_button(id + 3, t("Ausführen"));
    if ui.flyout_begin(id + 3, FL_BELOW) {
        row(ui, m + 0x20, JN_TERMINAL, t("Als Prozess ausführen"), (VK_F5, MOD_CTRL), file && jfs && !off, false, acts, Act::RunProcess);
        row(ui, m + 0x21, IC_PLAY, t("Als Dienst ausführen"), (VK_F5, 0), (file && jfs || inline) && !off, false, acts, Act::RunService);
        row(ui, m + 0x22, IC_STOP, t("Stoppen"), (VK_F5, MOD_SHIFT), running, false, acts, Act::Stop);
        row(ui, m + 0x23, IC_REFRESH, t("Neu starten"), (VK_F5, MOD_CTRL | MOD_SHIFT), has_svc && !off, false, acts, Act::Restart);
        ui.menu_sep();
        row(ui, m + 0x24, IC_ACCEPT, t("Tests ausführen"), (b'T' as u32, MOD_CTRL | MOD_SHIFT), file && jfs && !off, false, acts, Act::Tests);
        ui.menu_sep();
        row(ui, m + 0x25, IC_UPLOAD, t("Bereitstellen …"), (0, 0), !off, false, acts, Act::Lower(LOW_DEPLOY));
        row(ui, m + 0x26, IC_OFFICE_CHART, t("Lasttest …"), (0, 0), !off, false, acts, Act::Lower(LOW_LOAD));
    }
    ui.flyout_end();
    // Debuggen
    ui.next_flags(ST_COMPACT | ST_SUBTLE);
    ui.dropdown_button(id + 4, t("Debuggen"));
    if ui.flyout_begin(id + 4, FL_BELOW) {
        row(ui, m + 0x30, IC_PLAY, t("Fortsetzen"), (VK_F5, 0), paused, false, acts, Act::Dbg(debug::Op::Continue));
        row(ui, m + 0x31, IC_PAUSE, t("Anhalten"), (VK_F9, MOD_CTRL), running && !paused, false, acts, Act::Dbg(debug::Op::Pause));
        row(ui, m + 0x32, IC_REDO, t("Schritt über"), (VK_F10, 0), paused, false, acts, Act::Dbg(debug::Op::Step(Step::Over)));
        row(ui, m + 0x33, IC_DOWN, t("Schritt hinein"), (VK_F7, 0), paused || running, false, acts, Act::Dbg(debug::Op::Step(Step::Into)));
        row(ui, m + 0x34, IC_UP, t("Schritt heraus"), (VK_F7, MOD_SHIFT), paused, false, acts, Act::Dbg(debug::Op::Step(Step::Out)));
        ui.menu_sep();
        row(ui, m + 0x35, IC_CIRCLE_FILL, t("Haltepunkt an / aus"), (VK_F9, 0), jfs, false, acts, Act::Bp(usize::MAX, None));
        row(ui, m + 0x36, 0, t("Bedingter Haltepunkt …"), (VK_F9, MOD_CTRL | MOD_SHIFT), jfs, false, acts, Act::CondBp);
        row(ui, m + 0x37, IC_CLEAR, t("Alle Haltepunkte entfernen"), (0, 0), true, false, acts, Act::ClearBps);
    }
    ui.flyout_end();
    // Ansicht
    ui.next_flags(ST_COMPACT | ST_SUBTLE);
    ui.dropdown_button(id + 5, t("Ansicht"));
    if ui.flyout_begin(id + 5, FL_BELOW) {
        row(ui, m + 0x40, 0, t("Seitenleiste"), (b'B' as u32, MOD_CTRL), true, side_on, acts, Act::ToggleSide);
        row(ui, m + 0x41, 0, t("Explorer"), (b'E' as u32, MOD_CTRL | MOD_SHIFT), true, side_on && side == SIDE_EXPLORER, acts, Act::Side(SIDE_EXPLORER));
        row(ui, m + 0x42, 0, t("Suchen"), (0, 0), true, side_on && side == SIDE_SEARCH, acts, Act::Side(SIDE_SEARCH));
        row(ui, m + 0x43, 0, t("Gliederung"), (0, 0), true, side_on && side == SIDE_OUTLINE, acts, Act::Side(SIDE_OUTLINE));
        ui.menu_sep();
        row(ui, m + 0x44, 0, t("Untere Leiste"), (b'J' as u32, MOD_CTRL), true, lower_on, acts, Act::ToggleLower);
        for (i, name) in lower::names().iter().enumerate() {
            row(ui, m + 0x45 + i as u32, 0, name, (0, 0), true, lower_on && lower == i as i32, acts, Act::Lower(i as i32));
        }
    }
    ui.flyout_end();
    ui.next_size(1, 20);
    ui.separator();
    // the tool bar (narrow: only running and debugging)
    let bar_w = ui.px2dip(ui.box_rect()[2]);
    let wide = bar_w >= 980;
    ui.focus_group_begin(id + 0x10);
    let mut tool = |ui: &mut Ui, tid: u32, icon: u32, label: &str, keys: (u32, u32), on: bool, a: Act| {
        let keys = kt(ui, keys.0, keys.1);
        ui.next_icon(icon);
        ui.next_flags(ST_COMPACT | ST_SUBTLE | ST_ICON_ONLY | if on { 0 } else { ST_DISABLED });
        if ui.button(tid, label) & EV_ACT != 0 {
            acts.push(a);
        }
        if keys.is_empty() {
            ui.tooltip(label);
        } else {
            ui.tooltip(&format!("{} ({})", label, keys));
        }
    };
    if wide {
        tool(ui, id + 0x11, IC_SAVE, t("Speichern"), (b'S' as u32, MOD_CTRL), changed, Act::Save);
        tool(ui, id + 0x12, IC_SAVE_COPY, t("Alle speichern"), (b'S' as u32, MOD_CTRL | MOD_SHIFT), any_changed, Act::SaveAll);
    }
    tool(ui, id + 0x13, JN_TERMINAL, t("Als Prozess ausführen"), (VK_F5, MOD_CTRL), file && jfs && !off, Act::RunProcess);
    tool(ui, id + 0x14, IC_PLAY, t("Als Dienst ausführen"), (VK_F5, 0), (file && jfs || inline) && !off && !paused, Act::RunService);
    tool(ui, id + 0x15, IC_STOP, t("Stoppen"), (VK_F5, MOD_SHIFT), running, Act::Stop);
    tool(ui, id + 0x16, IC_REFRESH, t("Neu starten"), (VK_F5, MOD_CTRL | MOD_SHIFT), has_svc && !off, Act::Restart);
    if wide {
        tool(ui, id + 0x17, IC_ACCEPT, t("Tests ausführen"), (b'T' as u32, MOD_CTRL | MOD_SHIFT), file && jfs && !off, Act::Tests);
    }
    ui.next_size(1, 20);
    ui.separator();
    tool(ui, id + 0x18, IC_NEXT, t("Fortsetzen"), (VK_F5, 0), paused, Act::Dbg(debug::Op::Continue));
    tool(ui, id + 0x19, IC_PAUSE, t("Anhalten"), (VK_F9, MOD_CTRL), running && !paused, Act::Dbg(debug::Op::Pause));
    tool(ui, id + 0x1A, IC_REDO, t("Schritt über"), (VK_F10, 0), paused, Act::Dbg(debug::Op::Step(Step::Over)));
    tool(ui, id + 0x1B, IC_DOWN, t("Schritt hinein"), (VK_F7, 0), paused, Act::Dbg(debug::Op::Step(Step::Into)));
    tool(ui, id + 0x1C, IC_UP, t("Schritt heraus"), (VK_F7, MOD_SHIFT), paused, Act::Dbg(debug::Op::Step(Step::Out)));
    ui.focus_group_end();
    // what runs: the service and its state (at the end, while there is room)
    if bar_w >= 1060 || bar_w == 0 {
        ui.spacer(-1);
        let what = match (&cx.target, &cx.view) {
            (Some(t_), Some(v)) => {
                let (word, badge) = debug::state_word(v, cx.paused.is_some(), off);
                ui.status_dot(badge);
                tf("Dienst {}: {}", &[t_, word])
            }
            (Some(t_), None) => tf("Dienst {}: nicht im Simulator", &[t_]),
            (None, _) => String::new(),
        };
        ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &what);
    }
    ui.box_end();
}

// the side bar, the editor and the lower area
fn body(n: &mut Net, ui: &mut Ui, k: usize, id: u32, cx: &Cx, acts: &mut Vec<Act>) {
    let Some(p) = n.devwin.get(k).map(|w| &w.ide) else { return };
    let (side_on, side_w, lower_on, lower_h) = (p.side_on, p.side_w, p.lower_on, p.lower_h);
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_FIXED, 0);
    let r = ui.box_rect();
    let (w, h) = (ui.px2dip(r[2]), ui.px2dip(r[3]));
    let side = side_on && (w >= SIDE_MIN || w == 0);
    let low = lower_on && (h >= LOWER_MIN || h == 0);
    let mut spos = side_w.clamp(140, (w / 2).max(140));
    if side {
        ui.next_size(-1, -1);
        ui.split_begin(id + 0x100, spos, false);
        sidebar(n, ui, k, id, cx, acts);
        ui.split_next();
    }
    // the editor above, the lower area below (it keeps its height)
    let main_h = h;
    let mut lpos = if main_h > 0 { (main_h - lower_h - 6).max(120) } else { 300 };
    if low {
        ui.next_size(-1, -1);
        ui.split_begin(id + 0x102, lpos, true);
    }
    editor::area(n, ui, k, id, cx, acts);
    if low {
        ui.split_next();
        lower::area(n, ui, k, id, cx, acts);
        if ui.split_end(&mut lpos)
            && main_h > 0
            && let Some(w) = n.devwin.get(k)
        {
            w.ide.lower_h = (main_h - lpos - 6).max(80);
        }
    }
    if side && ui.split_end(&mut spos) && let Some(w) = n.devwin.get(k) {
        w.ide.side_w = spos.max(140);
    }
    ui.box_end();
}

// the side bar: Explorer, Suchen, Gliederung
fn sidebar(n: &mut Net, ui: &mut Ui, k: usize, id: u32, cx: &Cx, acts: &mut Vec<Act>) {
    let Some(mut sel) = n.devwin.get(k).map(|w| w.ide.side) else { return };
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | pad(2), 4);
    if ui.tabs(id + 0x104, &mut sel, &[t("Explorer"), t("Suchen"), t("Gliederung")]) & EV_ACT != 0 {
        acts.push(Act::Side(sel));
    }
    match sel {
        SIDE_SEARCH => search::view(n, ui, k, id, cx, acts),
        SIDE_OUTLINE => editor::outline(n, ui, k, id, cx, acts),
        _ => explorer::view(n, ui, k, id, cx, acts),
    }
    ui.box_end();
}

/// a file of the device (its path) as the shown tab of window k's IDE at its next frame
pub(crate) fn open_later(n: &mut Net, k: usize, path: &str) {
    if let Some(w) = n.devwin.get(k) {
        w.ide.open_req = Some(String::from(path));
    }
}

/// the IDE's command-line session in window k (Desk::ide_term): the one it has while the engine
/// knows it, else a new one while the device is on (None: off, or no command line)
pub(crate) fn term_session(n: &mut Net, k: usize, dev: u32) -> Option<u32> {
    let cur = n.devwin.peek(k).and_then(|w| w.desk.ide_term);
    if let Some(s) = cur
        && n.doc.sim.engine().cli_sessions(dev).iter().any(|x| x.id == s)
    {
        return Some(s);
    }
    // ended (the device was off): its console goes once a new session can take its place
    let s = crate::doc::console::open_session(n, dev)?;
    if let Some(old) = cur {
        crate::doc::console::close_session(n, dev, old);
    }
    if let Some(w) = n.devwin.get(k) {
        w.desk.ide_term = Some(s);
    }
    Some(s)
}

/// the Terminal pane: the IDE's own command-line session (term_session; after it ended, while
/// the device is off, its console says so)
pub(crate) fn terminal(n: &mut Net, ui: &mut Ui, k: usize, dev: u32) {
    let s = term_session(n, k, dev).or_else(|| n.devwin.peek(k).and_then(|w| w.desk.ide_term));
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 0);
    match s {
        Some(s) => crate::doc::console::view_session(n, ui, dev, s),
        None => {
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Das Gerät ist ausgeschaltet: das Terminal der IDE öffnet, sobald es wieder läuft."));
        }
    }
    ui.box_end();
}

/// TIMER_SCRIPT_CHECK: the check of every changed draft
pub(crate) fn check_due(n: &mut Net, ui: &mut Ui) {
    editor::check_due(n, ui);
}

/// the editor gets the keyboard for a few frames (a click elsewhere may still be under way),
/// then the caret goes to line / column
pub(crate) fn focus_editor(n: &mut Net, ui: &Ui, k: usize, at: Option<(usize, usize)>) {
    if let Some(w) = n.devwin.get(k) {
        w.ide.focus_until = ui.now().wrapping_add(100).max(1);
        if at.is_some() {
            w.ide.goto = at;
        }
    }
}

// what the widgets and keys asked for
fn act(n: &mut Net, ui: &mut Ui, k: usize, cx: &Cx, acts: Vec<Act>) {
    for a in acts {
        let r: Result<(), String> = match a {
            Act::Open(key) => editor::open(n, ui, k, cx, key, None),
            Act::OpenAt(key, line, col) => editor::open(n, ui, k, cx, key, Some((line, col))),
            Act::Close(i) => editor::close(n, k, i),
            Act::Tab(step) => {
                if let Some(w) = n.devwin.get(k)
                    && !w.ide.tabs.is_empty()
                {
                    let l = w.ide.tabs.len();
                    w.ide.cur = if step == 1 { (w.ide.cur + 1) % l } else { (w.ide.cur + l - 1) % l };
                    focus_editor(n, ui, k, None);
                }
                Ok(())
            }
            Act::Save => editor::save(n, k, cx, false),
            Act::SaveAll => editor::save(n, k, cx, true),
            Act::Ask(what) => {
                explorer::ask(n, ui, k, what);
                Ok(())
            }
            Act::Make(what, name) => explorer::make(n, ui, k, cx, what, &name),
            Act::Template(dir, i) => explorer::from_template(n, ui, k, cx, &dir, i),
            Act::Delete(key) => explorer::delete(n, k, cx, &key),
            Act::Convert(name) => explorer::convert(n, k, cx, &name),
            Act::RunProcess => debug::run_process(n, k, cx),
            Act::RunService => debug::run_service(n, k, cx),
            Act::Stop => debug::stop(n, cx),
            Act::Restart => debug::restart(n, k, cx),
            Act::Tests => debug::tests(n, k, cx),
            Act::Dbg(op) => debug::op(n, cx, op),
            Act::Bp(line, cond) => debug::toggle(n, k, cx, line, cond),
            Act::CondBp => {
                debug::cond_open(n, ui, k, id_of(k), cx);
                Ok(())
            }
            Act::ClearBps => {
                n.devwin.ide.bps.retain(|(dv, _), _| *dv != cx.dev);
                Ok(())
            }
            Act::Find(replace) => {
                let q = editor::selection_word(n, cx);
                ui.code_find(id_of(k) + 0x200, &q, replace);
                focus_editor(n, ui, k, None);
                Ok(())
            }
            Act::FindInFiles(replace) => {
                let q = editor::selection_word(n, cx);
                search::start(n, ui, k, id_of(k), &q, replace);
                Ok(())
            }
            Act::ReplaceAll => search::replace_all(n, k, cx),
            Act::Side(s) => {
                if let Some(w) = n.devwin.get(k) {
                    w.ide.side = s.clamp(0, 2);
                    w.ide.side_on = true;
                }
                Ok(())
            }
            Act::ToggleSide => {
                if let Some(w) = n.devwin.get(k) {
                    w.ide.side_on = !w.ide.side_on;
                }
                Ok(())
            }
            // (a refusal stands in the pane's form)
            Act::DeployStart => {
                let r = deploy::start(n, k, cx);
                if let (Err(m), Some(w)) = (r, n.devwin.get(k)) {
                    w.ide.deploy.msg = Some(m);
                }
                Ok(())
            }
            Act::DeployAbort => deploy::abort(n, cx),
            Act::DeployUnit => deploy::make_unit(n, k, cx),
            Act::LoadStart => {
                let r = load::start(n, k, cx);
                if let (Err(m), Some(w)) = (r, n.devwin.get(k)) {
                    w.ide.load.msg = Some(m);
                }
                Ok(())
            }
            Act::LoadStop => load::stop(n, cx),
            Act::Lower(l) => {
                if let Some(w) = n.devwin.get(k) {
                    w.ide.lower = l.clamp(0, LOW_LOAD);
                    w.ide.lower_on = true;
                    // Bereitstellen and Lasttest need height
                    if l >= LOW_DEPLOY {
                        w.ide.lower_h = w.ide.lower_h.max(deploy::PANE_H);
                    }
                }
                if l == LOW_TERMINAL {
                    n.cons.focus = Some(cx.dev);
                }
                Ok(())
            }
            Act::ToggleLower => {
                if let Some(w) = n.devwin.get(k) {
                    w.ide.lower_on = !w.ide.lower_on;
                }
                Ok(())
            }
            Act::Format => editor::format(n, ui, k, cx),
            Act::RenameName => {
                editor::rename_open(n, ui, k, id_of(k), cx, None);
                Ok(())
            }
            Act::Definition => editor::definition(n, ui, k, cx, None),
        };
        if let Some(w) = n.devwin.get(k) {
            match r {
                Err(m) => w.ide.msg = Some(m),
                Ok(()) => {}
            }
        }
    }
    editor::check_soon(n, ui);
}

/// the ids of window k's IDE
pub(crate) fn id_of(k: usize) -> u32 {
    ID_IDE0 + 0x1000 * k as u32
}
