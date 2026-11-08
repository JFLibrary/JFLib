// console -- the consoles of the devices: a ui::TermState per device that printed something or
// was opened, the simulator's reports distributed to them, what is typed sent to the engine,
// and the console tabs of the Terminal panel (workspace/terminal.rs draws them)
//
//   Consoles::new()                  no console (a new project)
//   pump(n)                          the simulator's reports: console output, prompts, saved
//                                    configurations (an undo step that keeps the running
//                                    device), new host names (a rename), log lines (the
//                                    Protokoll panel), power going off and on, changed files
//                                    (an undo step the engine has already: Report::Files)
//   open(n, ui, dev) -> Result       the device's console as the shown tab of the Terminal
//                                    panel (or its window to the front), with the keyboard
//   close_tab / float_tab / dock_tab / tile   a tab (device, session) goes (a further
//                                    session ends with it), into a window of its own, back,
//                                    every console in a window side by side
//   view(n, ui, dev)                 the console of a device here (the panel, its window, a
//                                    device window): it fills the place, its keys go to the
//                                    engine
//   typed(n, dev, line)              a line as if typed: its echo after the prompt, the
//                                    engine runs it, its output follows
//   run_lines(n, dev, label, lines)  a form's command lines: typed on the console and into the
//                                    startup configuration, one undo step
//   run_lines_running(n, dev, label, lines)   the same for forms that show what the device runs
//                                    (the Firewall tab): the startup configuration becomes the
//                                    running one with the lines, what was typed and not saved
//                                    included, so a rule typed on the console can be moved
//   save_running(n, dev)             the running configuration becomes the startup one (IOS:
//                                    "copy running-config startup-config" on the console)
//   resumed(n, t)                    "Von hier fortsetzen" at moment t: every console says so
//
//   open_session(n, dev) -> Option<u32>   a further session of the device (jfn_sim cli/ses.rs:
//                                    a terminal of its own, IOS "tty N", Linux "pts/N"); None
//                                    without a command line or power
//   view_session(n, ui, dev, s)      session s here as view does it for the console (s = 0 is
//                                    view); after the session ended its console says so and
//                                    takes no input
//   close_session(n, dev, s)         the session ends and its console goes
//   typed_session(n, dev, s, line)   a line as if typed in session s, as typed does it for the
//                                    console (the IDE runs files in its terminal's session)
//   new_tab(n, ui, dev)              "Neue Sitzung": a further session as a tab of the Terminal
//                                    panel, titled "PC1 (2)"; closing the tab ends it
//
// The console of a device (session 0) is the one of the tabs, the device window's CLI tab and
// the desktop; a further session has a console of its own (Consoles::sess), drawn where the
// caller puts it (a tab of the Terminal panel, a desktop's terminal window, the IDE's terminal
// pane).  The reports of further sessions (Report::SesOut / SesPrompt / SesClosed) reach their
// consoles; their lines, `?`, Tab and Ctrl+C go to the engine's session forms (cli_ses_*).
// Further sessions always talk to the present (a line returns the view to it).  While a
// console's program reads typed lines (telnet, nc: Engine::cli_reads) its input line stays,
// without a prompt of its own and without `?` help, and hides what is typed when the far end
// asks for a password (Engine::cli_secret).
//
// The engine prints without prompt and echo (ui::term adds them); the prompt and the busy
// state come from the engine whenever a console is drawn.  IOS-like consoles take `?`
// (Engine::cli_help) and Ctrl+Shift+6 and page what a command prints with --More-- (24 lines
// a screen as "terminal length 24"; "terminal length N" changes it, 0 turns it off); Tab completes from the engine's command trees, a Linux shell lists
// several choices and completes paths.  A Linux shell's less, more and man page what they
// print the same way (Engine::cli_pager).  Ctrl+C reaches the engine while a command runs or a question waits (else
// the terminal ended the line itself); Ctrl+Z leaves the configuration modes ("end"), Ctrl+L
// clears a shell.  A console is drawn in one place at a time: a device window that shows it
// first, then its own window, then its tab (workspace/terminal.rs).
//
// While the view stands in the past (K1) a console answers from the devices' state at that
// moment: its prompt, `?` and Tab come from it, and a line runs on a copy of it that goes
// afterwards (Sim::past_line; a command that ran then is given up on the copy first) -- show
// commands give the tables of then, nothing changes (a configuration command or a ping has no
// effect); a dim line says so once per moment.  Ctrl+C and Ctrl+Z do nothing there.  "Von hier
// fortsetzen" makes the moment the present again.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::catalog::CliStyle;
use jfn_model::{Device, Project};
use jfn_sim::cli::Mode;
use jfn_sim::{Action, Cmd, Engine, Report};
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::edits;
use crate::workspace::canvas::cables;

/// lines a console keeps
const SCROLLBACK: usize = 5000;
/// IOS' "terminal length" at the start: 24 lines, the last one --More--
const TERM_LENGTH: u16 = 24;

/// the console of one device
pub(crate) struct Console {
    pub term: TermState,
    /// IOS-like: `?`, Ctrl+Shift+6, --More--; else a Linux shell
    pub ios: bool,
    /// its widget slot (ID_TERM0 + 8 slot), given when it is drawn the first time
    slot: Option<u32>,
    /// "terminal length 0": no --More--
    pub nopage: bool,
    /// the device had power when last looked at
    power: bool,
    /// the next output's leading "\n" ends a line the terminal ended already (Ctrl+C while
    /// a question waited)
    skip_nl: bool,
    /// the moment in the past it last said it answers from (K1)
    past_note: Option<jfn_sim::Ns>,
    /// a further session's console: the session ended (cli/ses.rs)
    pub closed: bool,
}

/// a console in the Terminal panel
pub(crate) struct Tab {
    pub dev: u32,
    /// the session it shows (0: the console; jfn_sim cli/ses.rs)
    pub ses: u32,
    /// the title the user gave it (None: the device's name)
    pub title: Option<String>,
    /// in a window of its own: ID_TERM_WIN0 + 0x10 k
    pub win: Option<usize>,
}

/// the rename flyout of a tab
pub(crate) struct Rename {
    pub dev: u32,
    pub ses: u32,
    pub text: String,
    pub opened: bool,
}

pub(crate) struct Consoles {
    pub by_dev: BTreeMap<u32, Console>,
    /// the open consoles in tab order
    pub tabs: Vec<Tab>,
    /// the device of the shown tab
    pub cur: Option<u32>,
    /// the keyboard goes to this device's console where it is drawn next
    pub focus: Option<u32>,
    /// widget slot -> device
    slots: BTreeMap<u32, u32>,
    /// the windows of consoles: open flag and place
    pub wins: Vec<(Shared<i32>, Shared<WinRect>)>,
    pub rename: Option<Rename>,
    /// the consoles of further sessions by (device, session) (jfn_sim cli/ses.rs)
    pub sess: BTreeMap<(u32, u32), Console>,
    /// the session of the shown tab (0: the console)
    pub cur_ses: u32,
    /// the keyboard goes to this session's console where it is drawn next
    pub focus_ses: Option<(u32, u32)>,
    /// session widget slot -> (device, session)
    ses_slots: BTreeMap<u32, (u32, u32)>,
}

/// the device has a command line
pub(crate) fn has_cli(p: &Project, dev: u32) -> bool {
    p.model_of(dev).is_some_and(|m| m.cli != CliStyle::None)
}

fn ios_of(p: &Project, dev: u32) -> bool {
    p.model_of(dev).is_some_and(|m| m.cli == CliStyle::Ios)
}

/// the longest start all words share
fn common_prefix(words: &[String]) -> String {
    let Some(first) = words.first() else { return String::new() };
    let mut n = first.len();
    for w in &words[1..] {
        n = n.min(first.bytes().zip(w.bytes()).take_while(|(a, b)| a == b).count());
    }
    while !first.is_char_boundary(n) {
        n -= 1;
    }
    String::from(&first[..n])
}

/// "terminal length N" (IOS abbreviations allowed) -> N
fn term_length(line: &str) -> Option<u32> {
    let w: Vec<&str> = line.split_whitespace().collect();
    let is = |word: &str, full: &str, min: usize| word.len() >= min && full.starts_with(&word.to_ascii_lowercase());
    match w.as_slice() {
        [a, b, n] if is(a, "terminal", 3) && is(b, "length", 1) => n.parse().ok(),
        _ => None,
    }
}

impl Consoles {
    pub(crate) fn new() -> Consoles {
        let wins = (0..TERM_WINS).map(|_| (Shared::new(0), Shared::new(WinRect::new(0, 0, 640, 360, 0)))).collect();
        Consoles { by_dev: BTreeMap::new(), tabs: Vec::new(), cur: None, focus: None, slots: BTreeMap::new(), wins, rename: None, sess: BTreeMap::new(), cur_ses: 0, focus_ses: None, ses_slots: BTreeMap::new() }
    }

    // a new console of a device
    fn make(p: &Project, dev: u32) -> Console {
        let mut term = TermState::new(SCROLLBACK);
        term.pager_text = String::from(" --More-- ");
        term.page_len = TERM_LENGTH - 1;
        let power = p.device(dev).is_some_and(|d| !d.off);
        Console { term, ios: ios_of(p, dev), slot: None, nopage: false, power, skip_nl: false, past_note: None, closed: false }
    }

    /// a device's console (made when it is first needed)
    pub(crate) fn get(&mut self, p: &Project, dev: u32) -> &mut Console {
        self.by_dev.entry(dev).or_insert_with(|| Consoles::make(p, dev))
    }

    /// the console of session `s` of a device (0: get; made when it is first needed)
    pub(crate) fn get_ses(&mut self, p: &Project, dev: u32, s: u32) -> &mut Console {
        if s == 0 {
            return self.get(p, dev);
        }
        self.sess.entry((dev, s)).or_insert_with(|| Consoles::make(p, dev))
    }

    /// the widget id of a further session's console (ID_SES0 + 8 slot; a free slot the first
    /// time, else the one of a session that ended or has no tab)
    pub(crate) fn ses_id(&mut self, p: &Project, dev: u32, s: u32) -> u32 {
        if s == 0 {
            return self.id(p, dev);
        }
        let have = self.get_ses(p, dev, s).slot;
        let slot = match have {
            Some(x) => x,
            None => {
                let free = (0..SES_SLOTS).find(|k| !self.ses_slots.contains_key(k));
                let k = free.unwrap_or_else(|| {
                    let tabs: Vec<(u32, u32)> = self.tabs.iter().map(|t| (t.dev, t.ses)).collect();
                    let (&k, &old) = self.ses_slots.iter().find(|(_, key)| !tabs.contains(key) || self.sess.get(key).is_some_and(|c| c.closed)).unwrap_or((&0, &(0, 0)));
                    if let Some(c) = self.sess.get_mut(&old) {
                        c.slot = None;
                    }
                    k
                });
                self.ses_slots.insert(k, (dev, s));
                if let Some(c) = self.sess.get_mut(&(dev, s)) {
                    c.slot = Some(k);
                }
                k
            }
        };
        ID_SES0 + 8 * slot
    }

    /// the tab of a session (0: the console's)
    pub(crate) fn tab_of(&self, dev: u32, s: u32) -> Option<usize> {
        self.tabs.iter().position(|t| t.dev == dev && t.ses == s)
    }

    /// the title of a session's tab: the user's, else "PC1" for the console and "PC1 (2)" for
    /// a further session
    pub(crate) fn title_of(&self, p: &Project, dev: u32, s: u32) -> String {
        if s == 0 {
            return self.title(p, dev);
        }
        match self.tab_of(dev, s).and_then(|i| self.tabs[i].title.clone()) {
            Some(t) => t,
            None => format!("{} ({})", p.device(dev).map_or_else(|| tf("Gerät {}", &[&dev.to_string()]), |d| d.name.clone()), s + 1),
        }
    }

    /// the docked tabs as (device, session), in order
    pub(crate) fn docked_keys(&self) -> Vec<(u32, u32)> {
        self.tabs.iter().filter(|t| t.win.is_none()).map(|t| (t.dev, t.ses)).collect()
    }

    /// the (device, session) whose console window k shows
    pub(crate) fn in_win_key(&self, k: usize) -> Option<(u32, u32)> {
        self.tabs.iter().find(|t| t.win == Some(k)).map(|t| (t.dev, t.ses))
    }

    /// the widget id of a device's console (ID_TERM0 + 8 slot; a free slot the first time,
    /// else the one of a console without a tab)
    pub(crate) fn id(&mut self, p: &Project, dev: u32) -> u32 {
        let have = self.get(p, dev).slot;
        let slot = match have {
            Some(s) => s,
            None => {
                let free = (0..TERM_SLOTS).find(|s| !self.slots.contains_key(s));
                let s = free.unwrap_or_else(|| {
                    // every slot taken: the first console without a tab gives its slot up
                    let tabs: Vec<u32> = self.tabs.iter().map(|t| t.dev).collect();
                    let (&s, &old) = self.slots.iter().find(|(_, d)| !tabs.contains(d)).unwrap_or((&0, &0));
                    if let Some(c) = self.by_dev.get_mut(&old) {
                        c.slot = None;
                    }
                    s
                });
                self.slots.insert(s, dev);
                if let Some(c) = self.by_dev.get_mut(&dev) {
                    c.slot = Some(s);
                }
                s
            }
        };
        ID_TERM0 + 8 * slot
    }

    /// the tab of a device's console
    pub(crate) fn tab(&self, dev: u32) -> Option<usize> {
        self.tab_of(dev, 0)
    }

    /// the title of a tab: the user's, else the device's name
    pub(crate) fn title(&self, p: &Project, dev: u32) -> String {
        match self.tab(dev).and_then(|i| self.tabs[i].title.clone()) {
            Some(t) => t,
            None => p.device(dev).map_or_else(|| tf("Gerät {}", &[&dev.to_string()]), |d| d.name.clone()),
        }
    }

    /// the docked tabs (not in a window of their own), in order
    pub(crate) fn docked(&self) -> Vec<u32> {
        self.tabs.iter().filter(|t| t.win.is_none()).map(|t| t.dev).collect()
    }

    /// the device whose console window k shows
    pub(crate) fn in_win(&self, k: usize) -> Option<u32> {
        self.tabs.iter().find(|t| t.win == Some(k)).map(|t| t.dev)
    }
}

// ---------------------------------------------------------------- reports

/// the simulator's reports since the last call (every frame, and around what is typed)
pub(crate) fn pump(n: &mut Net) {
    pump_paged(n, None);
}

// ... with the output of console `paged` (device, session) paged (IOS: what a command prints)
// -> that output
fn pump_paged(n: &mut Net, paged: Option<(u32, u32)>) -> String {
    let mut text = String::new();
    let mut paging = false;
    for r in n.doc.sim.take_reports() {
        let (dev, ses, bytes) = match r {
            Report::CliOut { dev, bytes, .. } => (dev, 0, bytes),
            // a further session's output: only to a console that exists (cli/ses.rs)
            Report::SesOut { dev, ses, bytes, .. } if n.cons.sess.contains_key(&(dev, ses)) => (dev, ses, bytes),
            Report::SesOut { .. } => continue,
            Report::CliState { dev, prompt, busy } => {
                if let Some(c) = n.cons.by_dev.get_mut(&dev) {
                    c.term.set_prompt(&prompt);
                    c.term.set_mode(if busy { TM_BUSY } else { TM_LINE });
                }
                n.task.console |= !busy;
                continue;
            }
            Report::SesPrompt { dev, ses, prompt, busy } => {
                if let Some(c) = n.cons.sess.get_mut(&(dev, ses)) {
                    c.term.set_prompt(&prompt);
                    c.term.set_mode(if busy { TM_BUSY } else { TM_LINE });
                }
                continue;
            }
            Report::SesClosed { dev, ses } => {
                if let Some(c) = n.cons.sess.get_mut(&(dev, ses))
                    && !c.closed
                {
                    c.closed = true;
                    c.term.write_str(&format!("\x1b[2m--- {} ---\x1b[0m\n", t("Sitzung beendet")));
                }
                continue;
            }
            Report::Startup { dev, text } => {
                saved(n, dev, text);
                continue;
            }
            Report::Hostname { dev, name } => {
                renamed(n, dev, &name);
                continue;
            }
            Report::Log { dev, at, sev, tag, text } => {
                n.log.push(dev, at, sev, tag, text);
                continue;
            }
            Report::Files { dev, path, files, .. } => {
                files_changed(n, dev, &path, files);
                continue;
            }
            Report::Web { dev, id, result, .. } => {
                crate::workspace::devwin::browser::report(n, dev, id, result);
                continue;
            }
        };
        let c = n.cons.get_ses(&n.doc.proj, dev, ses);
        let mut b: &[u8] = &bytes;
        if c.skip_nl {
            c.skip_nl = false;
            if b.first() == Some(&b'\n') {
                b = &b[1..];
            }
        }
        if paged == Some((dev, ses)) {
            if !paging {
                c.term.pager(true);
                paging = true;
            }
            text.push_str(&String::from_utf8_lossy(b));
        }
        c.term.write(b);
    }
    if paging && let Some((d, s)) = paged {
        let c = if s == 0 { n.cons.by_dev.get_mut(&d) } else { n.cons.sess.get_mut(&(d, s)) };
        if let Some(c) = c {
            c.term.pager(false);
        }
    }
    // the tabs of devices that are gone close (an undo brings the device, not its tab, back)
    let gone: Vec<(u32, u32)> = n.cons.tabs.iter().map(|t| (t.dev, t.ses)).filter(|&(d, _)| n.doc.proj.device(d).is_none()).collect();
    for (d, s) in gone {
        close_tab(n, d, s);
    }
    n.cons.sess.retain(|(d, _), _| n.doc.proj.device(*d).is_some());
    // power: a console says when its device went off or came back
    let p = &n.doc.proj;
    for (&dev, c) in n.cons.by_dev.iter_mut() {
        let Some(d) = p.device(dev) else { continue };
        if c.power == d.off {
            c.power = !d.off;
            let what = if d.off { "ausgeschaltet" } else { "eingeschaltet" };
            c.term.write_str(&format!("\x1b[2m--- {} {} ---\x1b[0m\n", d.name, what));
        }
    }
    text
}

// "copy running-config startup-config": the project keeps the configuration (one undo step;
// the running device keeps running, also through undo and redo)
fn saved(n: &mut Net, dev: u32, text: String) {
    let Some(name) = n.doc.proj.device(dev).map(|d| d.name.clone()) else { return };
    let label = format!("Konfiguration von {} sichern", name);
    if let Some(e) = edits::device_edit(&n.doc.proj, dev, &label, |x| x.startup_config = text) {
        n.doc.apply_kept(e, dev);
    }
}

// a transfer or a command line changed the device's files: the project keeps them (one undo
// step the engine has already; undo and redo hand the other records back: Sim::keep)
fn files_changed(n: &mut Net, dev: u32, path: &str, files: Vec<jfn_model::files::File>) {
    let Some(name) = n.doc.proj.device(dev).map(|d| d.name.clone()) else { return };
    let label = format!("Datei {} auf {}", path, name);
    if let Some(e) = edits::device_edit(&n.doc.proj, dev, &label, |x| x.files = files) {
        n.doc.apply_kept(e, dev);
    }
}

// "hostname": the project renames the device (one undo step); a name it refuses goes back
fn renamed(n: &mut Net, dev: u32, name: &str) {
    let Some(old) = n.doc.proj.device(dev).map(|d| d.name.clone()) else { return };
    if old == name {
        return;
    }
    match edits::rename(&mut n.doc.proj, dev, name) {
        Ok(Some(e)) => n.doc.record(e),
        Ok(None) => {}
        Err(why) => {
            let _ = n.doc.sim.send(Cmd::Apply(Action::Hostname { dev, name: old.clone() }));
            let c = n.cons.get(&n.doc.proj, dev);
            c.term.write_str(&format!("% {}\n", tf("{} Das Gerät heißt weiter {}.", &[&why.to_string(), &old])));
        }
    }
}

// ---------------------------------------------------------------- tabs and windows

/// the console of device `dev` as the shown tab of the Terminal panel (its own window comes
/// to the front), with the keyboard -> Err: why there is none
pub(crate) fn open(n: &mut Net, ui: &mut Ui, dev: u32) -> Result<(), String> {
    // a member of a collapsed group: its groups open first (G1)
    if n.doc.proj.device(dev).is_some() && has_cli(&n.doc.proj, dev) {
        crate::workspace::canvas::groups::reveal(n, dev);
    }
    let Some(d) = n.doc.proj.device(dev) else { return Err(tf("Das Gerät {} gibt es nicht.", &[&dev.to_string()])) };
    if !has_cli(&n.doc.proj, dev) {
        return Err(tf("{} hat keine Befehlszeile.", &[&d.name]));
    }
    let cons = &mut n.cons;
    if cons.tab(dev).is_none() {
        cons.tabs.push(Tab { dev, ses: 0, title: None, win: None });
    }
    cons.focus = Some(dev);
    cons.focus_ses = None;
    match cons.tab(dev).and_then(|i| cons.tabs[i].win) {
        Some(k) => ui.window_activate(ID_TERM_WIN0 + 0x10 * k as u32),
        None => {
            cons.cur = Some(dev);
            cons.cur_ses = 0;
            n.shell.dock.show(P_TERMINAL);
        }
    }
    ui.repass();
    Ok(())
}

/// the tab of a session goes (the console's scrollback stays; a further session ends)
pub(crate) fn close_tab(n: &mut Net, dev: u32, s: u32) {
    let cons = &mut n.cons;
    let Some(i) = cons.tab_of(dev, s) else { return };
    let t = cons.tabs.remove(i);
    if let Some(k) = t.win {
        cons.wins[k].0.set(0);
    }
    if cons.cur == Some(dev) && cons.cur_ses == s {
        // the neighbour becomes the shown tab
        let docked: Vec<(usize, (u32, u32))> = cons.tabs.iter().enumerate().filter(|(_, t)| t.win.is_none()).map(|(j, t)| (j, (t.dev, t.ses))).collect();
        let next = docked.iter().find(|(j, _)| *j >= i).or(docked.last()).map(|(_, k)| *k);
        cons.cur = next.map(|k| k.0);
        cons.cur_ses = next.map_or(0, |k| k.1);
    }
    if cons.rename.as_ref().is_some_and(|r| r.dev == dev && r.ses == s) {
        cons.rename = None;
    }
    if s != 0 {
        close_session(n, dev, s);
    }
}

/// a session's tab into a window of its own -> false when every console window is in use
pub(crate) fn float_tab(n: &mut Net, ui: &mut Ui, dev: u32, s: u32) -> bool {
    let cons = &mut n.cons;
    let Some(i) = cons.tab_of(dev, s) else { return false };
    if cons.tabs[i].win.is_some() {
        return true;
    }
    let Some(k) = (0..TERM_WINS).find(|&k| cons.in_win(k).is_none()) else { return false };
    cons.tabs[i].win = Some(k);
    cons.wins[k].0.set(1);
    if cons.cur == Some(dev) && cons.cur_ses == s {
        let next = cons.tabs.iter().find(|t| t.win.is_none()).map(|t| (t.dev, t.ses));
        cons.cur = next.map(|k| k.0);
        cons.cur_ses = next.map_or(0, |k| k.1);
    }
    focus_key(cons, dev, s);
    ui.repass();
    true
}

/// a session's tab from its window back into the panel, as the shown tab
pub(crate) fn dock_tab(n: &mut Net, dev: u32, s: u32) {
    let cons = &mut n.cons;
    let Some(i) = cons.tab_of(dev, s) else { return };
    if let Some(k) = cons.tabs[i].win.take() {
        cons.wins[k].0.set(0);
    }
    cons.cur = Some(dev);
    cons.cur_ses = s;
    n.shell.dock.show(P_TERMINAL);
}

// the keyboard goes to session s's console of `dev` where it is drawn next
fn focus_key(cons: &mut Consoles, dev: u32, s: u32) {
    if s == 0 {
        cons.focus = Some(dev);
        cons.focus_ses = None;
    } else {
        cons.focus = None;
        cons.focus_ses = Some((dev, s));
    }
}

/// every open console in a window of its own (as many as there are windows), side by side
/// over the lower half of the main window
pub(crate) fn tile(n: &mut Net, ui: &mut Ui) {
    for (dev, s) in n.cons.docked_keys() {
        if !float_tab(n, ui, dev, s) {
            break;
        }
    }
    let shown: Vec<usize> = (0..TERM_WINS).filter(|&k| n.cons.in_win(k).is_some()).collect();
    if shown.is_empty() {
        return;
    }
    let (w, h) = ui.client_size();
    let k = shown.len() as i32;
    let ww = ((w - 16) / k).max(240);
    let wh = (h / 2).max(200);
    for (i, &slot) in shown.iter().enumerate() {
        let r = WinRect::new(8 + i as i32 * ww, h - wh - 8, ww - 8, wh - 40, WRF_POS);
        n.cons.wins[slot].1.set(r);
        ui.window_place(ID_TERM_WIN0 + 0x10 * slot as u32, &r);
    }
    ui.repass();
}

// ---------------------------------------------------------------- drawing and keys

/// the console of device `dev`, filling the place it gets: prompt and busy state from the
/// engine, its keys to the engine; without power a line says why it does not answer
pub(crate) fn view(n: &mut Net, ui: &mut Ui, dev: u32) {
    let Some(d) = n.doc.proj.device(dev) else { return };
    let (on, name) = (!d.off, d.name.clone());
    let (prompt, busy, secret, reads) = {
        let e = n.doc.sim.state();
        let live = n.doc.sim.past().is_none();
        // a program that reads typed lines (telnet, nc): the input line stays, without a prompt
        let reads = on && live && e.cli_reads(dev);
        // in the past a line always runs (on a copy, Sim::past_line)
        let busy = e.cli_busy(dev) && live && !reads;
        // a login gate (or a telnet session's far end) that waits for a password: the input
        // line hides what is typed
        let secret = on && e.cli_secret(dev) && live;
        let prompt = if reads { String::new() } else { e.cli_prompt(dev) };
        if on { (prompt, busy, secret, reads) } else { (String::new(), false, false, false) }
    };
    let id = n.cons.id(&n.doc.proj, dev);
    if !on {
        ui.box_begin(0, BOX_H | BOX_XCENTER | pad(4), 8);
        ui.icon(IC_POWER, 16);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_SECONDARY | TX_ELLIPSIS, &tf("{} ist ausgeschaltet: die Konsole antwortet wieder, sobald es Strom hat.", &[&name]));
        if ui.button(ID_TERM_POWER, t("Einschalten")) & EV_ACT != 0 {
            cables::set_power(n, &[dev], true);
            ui.repass();
        }
        ui.box_end();
    }
    if n.cons.focus == Some(dev) {
        n.cons.focus = None;
        ui.focus(id);
    }
    let c = n.cons.get(&n.doc.proj, dev);
    c.term.set_prompt(&prompt);
    c.term.set_mode(if secret { TM_SECRET } else if busy { TM_BUSY } else { TM_LINE });
    let mut flags = if c.ios { TF_QHELP | TF_CTRL_SHIFT6 } else { 0 };
    if reads {
        // `?` belongs to the far end then
        flags &= !TF_QHELP;
    }
    if !on {
        flags |= TF_NO_INPUT;
    }
    ui.next_size(-1, -1);
    let evs = ui.terminal(id, &mut c.term, flags);
    if !evs.is_empty() {
        for ev in evs {
            event(n, dev, ev);
        }
        ui.repass();
    }
}

fn event(n: &mut Net, dev: u32, ev: TermEv) {
    match ev {
        TermEv::Submit(line) => submit(n, dev, &line),
        TermEv::Help(line) => help(n, dev, &line),
        TermEv::Complete { line, caret } => complete(n, dev, &line, caret),
        TermEv::Interrupt => interrupt(n, dev),
        TermEv::Chord(k) => chord(n, dev, k),
        _ => {}
    }
}

// Enter: the engine runs the line; on IOS its output pages
fn submit(n: &mut Net, dev: u32, line: &str) {
    pump(n);
    if n.doc.sim.past().is_some() {
        past_submit(n, dev, line);
        return;
    }
    let r = n.doc.sim.send(Cmd::Cli { dev, line: String::from(line) });
    if let Err(e) = r {
        n.cons.get(&n.doc.proj, dev).term.write_str(&format!("% {}\n", e));
        return;
    }
    let page = n.cons.by_dev.get(&dev).is_some_and(|c| c.ios && !c.nopage) || n.doc.sim.engine().cli_pager(dev);
    let out = pump_paged(n, if page { Some((dev, 0)) } else { None });
    // a line that needs no time sends no CliState, and a quiet network wakes nothing: the goals look now
    n.task.console |= !n.doc.sim.engine().cli_busy(dev);
    // "terminal length N" (the device took it): N lines a page (the last one --More--), 0 none
    if let Some(len) = term_length(line)
        && out.is_empty()
        && let Some(c) = n.cons.by_dev.get_mut(&dev)
        && c.ios
    {
        c.nopage = len == 0;
        c.term.page_len = (len.clamp(2, 512) - 1) as u16;
    }
}

// in the past (K1): the line runs on a copy of the state shown, its output follows (paged on
// IOS); a dim line says once per moment where the answers come from
fn past_submit(n: &mut Net, dev: u32, line: &str) {
    let Some(at) = n.doc.sim.past() else { return };
    let out = n.doc.sim.past_line(dev, line);
    let c = n.cons.get(&n.doc.proj, dev);
    if c.past_note != Some(at) {
        c.past_note = Some(at);
        let secs = crate::workspace::sim::secs(at);
        c.term.write_str(&format!("\x1b[2m--- {} ---\x1b[0m\n", tf("Zustand bei t = {} s: nur Anzeige, nichts wird geändert", &[&secs])));
    }
    match out {
        Some(Ok(text)) => {
            let page = c.ios && !c.nopage;
            if page {
                c.term.pager(true);
            }
            c.term.write_str(&text);
            if page {
                c.term.pager(false);
            }
        }
        Some(Err(e)) => c.term.write_str(&format!("% {}\n", e)),
        None => {}
    }
}

// `?` (IOS): the help of the engine's command tree; the line stays
fn help(n: &mut Net, dev: u32, line: &str) {
    pump(n);
    let text = n.doc.sim.state().cli_help(dev, line);
    let c = n.cons.get(&n.doc.proj, dev);
    let page = c.ios && !c.nopage;
    if page {
        c.term.pager(true);
    }
    c.term.write_str(&text);
    if page {
        c.term.pager(false);
    }
}

// Tab: the engine completes the word before the caret; a shell lists several choices (IOS
// keeps the line as it is)
fn complete(n: &mut Net, dev: u32, line: &str, caret: usize) {
    let mut caret = caret.min(line.len());
    while !line.is_char_boundary(caret) {
        caret -= 1;
    }
    let (head, tail) = line.split_at(caret);
    let (new, choices) = n.doc.sim.state().cli_complete(dev, head);
    let c = n.cons.get(&n.doc.proj, dev);
    match new {
        Some(s) => c.term.complete_reply(&format!("{}{}", s, tail), &[] as &[&str]),
        None if !c.ios && choices.len() > 1 => {
            let start = head.rfind(char::is_whitespace).map_or(0, |i| i + 1);
            let common = common_prefix(&choices);
            let word = if common.len() > head.len() - start { common.as_str() } else { &head[start..] };
            c.term.complete_reply(&format!("{}{}{}", &head[..start], word, tail), &choices);
        }
        None => c.term.complete_reply(line, &[] as &[&str]),
    }
}

// Ctrl+C / Ctrl+Shift+6: a running command stops, a waiting question is given up (not in the
// past)
fn interrupt(n: &mut Net, dev: u32) {
    if n.doc.sim.past().is_some() {
        return;
    }
    let (busy, asked) = {
        let e = n.doc.sim.engine();
        (e.cli_busy(dev), e.node(dev).is_some_and(|x| x.cli.ask.is_some()))
    };
    if !busy && !asked {
        return; // the terminal ended the line already
    }
    pump(n);
    if !busy && let Some(c) = n.cons.by_dev.get_mut(&dev) {
        c.skip_nl = true;
    }
    let _ = n.doc.sim.send(Cmd::Interrupt { dev });
    pump(n);
}

// Ctrl+Z (IOS): out of the configuration modes, as "end"; Ctrl+L (shell): a clear screen
fn chord(n: &mut Net, dev: u32, k: u32) {
    let ios = n.cons.by_dev.get(&dev).is_some_and(|c| c.ios);
    match k & 0xFFFF {
        0x5A if ios && n.doc.sim.past().is_none() => {
            let (config, busy, prompt) = {
                let e = n.doc.sim.engine();
                let config = e.node(dev).is_some_and(|x| !matches!(x.cli.mode, Mode::User | Mode::Priv | Mode::Out));
                (config, e.cli_busy(dev), e.cli_prompt(dev))
            };
            if config && !busy {
                pump(n);
                n.cons.get(&n.doc.proj, dev).term.write_str(&format!("{}^Z\n", prompt));
                let _ = n.doc.sim.send(Cmd::Cli { dev, line: String::from("end") });
                pump(n);
            }
        }
        0x4C if !ios => n.cons.get(&n.doc.proj, dev).term.write_str("\x1b[H\x1b[2J"),
        _ => {}
    }
}

/// a line as if typed at the console: its echo after the prompt, the engine runs it, its
/// output follows (not paged)
pub(crate) fn typed(n: &mut Net, dev: u32, line: &str) {
    pump(n);
    let prompt = n.doc.sim.engine().cli_prompt(dev);
    n.cons.get(&n.doc.proj, dev).term.write_str(&format!("{}{}\n", prompt, line));
    if let Err(e) = n.doc.sim.send(Cmd::Cli { dev, line: String::from(line) }) {
        n.cons.get(&n.doc.proj, dev).term.write_str(&format!("% {}\n", e));
    }
    pump(n);
}

// ---------------------------------------------------------------- further sessions (cli/ses.rs)

/// a further session of device `dev` with a console of its own -> its id (None: the device has
/// no command line or no power); the caller draws it (view_session) and ends it (close_session)
pub(crate) fn open_session(n: &mut Net, dev: u32) -> Option<u32> {
    if !has_cli(&n.doc.proj, dev) || n.doc.proj.device(dev).is_none_or(|d| d.off) {
        return None;
    }
    pump(n);
    let s = n.doc.sim.open_session(dev).ok()?;
    n.cons.get_ses(&n.doc.proj, dev, s);
    pump(n);
    Some(s)
}

/// session `s` of device `dev` ends and its console goes (0: nothing, the console stays)
pub(crate) fn close_session(n: &mut Net, dev: u32, s: u32) {
    if s == 0 {
        return;
    }
    let alive = !n.cons.sess.get(&(dev, s)).is_some_and(|c| c.closed);
    if alive {
        let _ = n.doc.sim.send(Cmd::SesClose { dev, ses: s });
    }
    pump(n);
    n.cons.sess.remove(&(dev, s));
    n.cons.ses_slots.retain(|_, k| *k != (dev, s));
    if n.cons.focus_ses == Some((dev, s)) {
        n.cons.focus_ses = None;
    }
}

/// a line as if typed in session `s` of `dev` (0: typed): its echo after the prompt, the engine
/// runs it, its output follows
pub(crate) fn typed_session(n: &mut Net, dev: u32, s: u32, line: &str) {
    if s == 0 {
        typed(n, dev, line);
        return;
    }
    pump(n);
    let prompt = n.doc.sim.engine().cli_ses_prompt(dev, s);
    n.cons.get_ses(&n.doc.proj, dev, s).term.write_str(&format!("{}{}\n", prompt, line));
    ses_submit(n, dev, s, line);
}

/// "Neue Sitzung": a further session of `dev` as a tab of the Terminal panel ("PC1 (2)"), shown
/// and with the keyboard -> Err: why there is none
pub(crate) fn new_tab(n: &mut Net, ui: &mut Ui, dev: u32) -> Result<u32, String> {
    let Some(name) = n.doc.proj.device(dev).map(|d| d.name.clone()) else { return Err(tf("Das Gerät {} gibt es nicht.", &[&dev.to_string()])) };
    let Some(s) = open_session(n, dev) else {
        return Err(tf("{} nimmt keine weitere Sitzung an: es hat keine Befehlszeile oder keinen Strom.", &[&name]));
    };
    let cons = &mut n.cons;
    cons.tabs.push(Tab { dev, ses: s, title: None, win: None });
    cons.cur = Some(dev);
    cons.cur_ses = s;
    focus_key(cons, dev, s);
    n.shell.dock.show(P_TERMINAL);
    ui.repass();
    Ok(s)
}

/// session `s` of device `dev` here, as view draws the console (s = 0 is view): prompt and busy
/// state from the engine, its keys to the engine's session forms; a session that ended says so
/// and takes no input
pub(crate) fn view_session(n: &mut Net, ui: &mut Ui, dev: u32, s: u32) {
    if s == 0 {
        view(n, ui, dev);
        return;
    }
    if n.doc.proj.device(dev).is_none() {
        return;
    }
    let (prompt, busy, secret, reads, alive) = {
        let e = n.doc.sim.engine();
        let alive = e.cli_sessions(dev).iter().any(|x| x.id == s) && !n.cons.sess.get(&(dev, s)).is_some_and(|c| c.closed);
        let reads = alive && e.cli_ses_reads(dev, s);
        let busy = alive && e.cli_ses_busy(dev, s) && !reads;
        let secret = alive && e.cli_ses_secret(dev, s);
        let prompt = if alive && !reads { e.cli_ses_prompt(dev, s) } else { String::new() };
        (prompt, busy, secret, reads, alive)
    };
    if !alive && let Some(c) = n.cons.sess.get_mut(&(dev, s))
        && !c.closed
    {
        // it went while nobody pumped (the device was reloaded)
        c.closed = true;
        c.term.write_str(&format!("\x1b[2m--- {} ---\x1b[0m\n", t("Sitzung beendet")));
    }
    let id = n.cons.ses_id(&n.doc.proj, dev, s);
    if n.cons.focus_ses == Some((dev, s)) {
        n.cons.focus_ses = None;
        ui.focus(id);
    }
    let c = n.cons.get_ses(&n.doc.proj, dev, s);
    c.term.set_prompt(&prompt);
    c.term.set_mode(if secret { TM_SECRET } else if busy { TM_BUSY } else { TM_LINE });
    let mut flags = if c.ios && !reads { TF_QHELP | TF_CTRL_SHIFT6 } else if c.ios { TF_CTRL_SHIFT6 } else { 0 };
    if !alive {
        flags |= TF_NO_INPUT;
    }
    ui.next_size(-1, -1);
    let evs = ui.terminal(id, &mut c.term, flags);
    if !evs.is_empty() {
        for ev in evs {
            ses_event(n, dev, s, ev);
        }
        ui.repass();
    }
}

fn ses_event(n: &mut Net, dev: u32, s: u32, ev: TermEv) {
    match ev {
        TermEv::Submit(line) => ses_submit(n, dev, s, &line),
        TermEv::Help(line) => {
            pump(n);
            let text = n.doc.sim.live_mut().cli_ses_help(dev, s, &line);
            let c = n.cons.get_ses(&n.doc.proj, dev, s);
            let page = c.ios && !c.nopage;
            if page {
                c.term.pager(true);
            }
            c.term.write_str(&text);
            if page {
                c.term.pager(false);
            }
        }
        TermEv::Complete { line, caret } => {
            let mut caret = caret.min(line.len());
            while !line.is_char_boundary(caret) {
                caret -= 1;
            }
            let (head, tail) = line.split_at(caret);
            let (new, choices) = n.doc.sim.live_mut().cli_ses_complete(dev, s, head);
            let c = n.cons.get_ses(&n.doc.proj, dev, s);
            reply_complete(c, &line, head, tail, new, &choices);
        }
        TermEv::Interrupt => {
            let busy = n.doc.sim.engine().cli_ses_busy(dev, s);
            if busy {
                pump(n);
                let _ = n.doc.sim.send(Cmd::SesInterrupt { dev, ses: s });
                pump(n);
            }
        }
        TermEv::Chord(k) => {
            let ios = n.cons.sess.get(&(dev, s)).is_some_and(|c| c.ios);
            match k & 0xFFFF {
                0x5A if ios => {
                    let (config, busy, prompt) = {
                        let e = n.doc.sim.engine();
                        (e.cli_ses_mode(dev, s).is_some_and(|m| !matches!(m, Mode::User | Mode::Priv | Mode::Out)), e.cli_ses_busy(dev, s), e.cli_ses_prompt(dev, s))
                    };
                    if config && !busy {
                        pump(n);
                        n.cons.get_ses(&n.doc.proj, dev, s).term.write_str(&format!("{}^Z\n", prompt));
                        let _ = n.doc.sim.send(Cmd::SesLine { dev, ses: s, line: String::from("end") });
                        pump(n);
                    }
                }
                0x4C if !ios => n.cons.get_ses(&n.doc.proj, dev, s).term.write_str("\x1b[H\x1b[2J"),
                _ => {}
            }
        }
        _ => {}
    }
}

// Enter in a further session: the engine runs the line; on IOS its output pages
fn ses_submit(n: &mut Net, dev: u32, s: u32, line: &str) {
    pump(n);
    if let Err(e) = n.doc.sim.send(Cmd::SesLine { dev, ses: s, line: String::from(line) }) {
        n.cons.get_ses(&n.doc.proj, dev, s).term.write_str(&format!("% {}\n", e));
        return;
    }
    let page = n.cons.sess.get(&(dev, s)).is_some_and(|c| c.ios && !c.nopage) || n.doc.sim.engine().cli_ses_pager(dev, s);
    let out = pump_paged(n, if page { Some((dev, s)) } else { None });
    if let Some(len) = term_length(line)
        && out.is_empty()
        && let Some(c) = n.cons.sess.get_mut(&(dev, s))
        && c.ios
    {
        c.nopage = len == 0;
        c.term.page_len = (len.clamp(2, 512) - 1) as u16;
    }
}

// the answer to Tab: the completed line, or (a shell) the choices below the line
fn reply_complete(c: &mut Console, line: &str, head: &str, tail: &str, new: Option<String>, choices: &[String]) {
    match new {
        Some(x) => c.term.complete_reply(&format!("{}{}", x, tail), &[] as &[&str]),
        None if !c.ios && choices.len() > 1 => {
            let start = head.rfind(char::is_whitespace).map_or(0, |i| i + 1);
            let common = common_prefix(choices);
            let word = if common.len() > head.len() - start { common.as_str() } else { &head[start..] };
            c.term.complete_reply(&format!("{}{}{}", &head[..start], word, tail), choices);
        }
        None => c.term.complete_reply(line, &[] as &[&str]),
    }
}

// ---------------------------------------------------------------- forms

// what the device said to a line is a complaint: IOS "% ..." lines (not the time-stamped
// console messages), anything on a shell (its commands print nothing when they work)
pub(crate) fn refused(ios: bool, out: &str) -> Option<String> {
    let out = out.trim_end_matches('\n');
    if ios {
        out.lines().find(|l| l.starts_with('%')).map(String::from)
    } else if out.trim().is_empty() {
        None
    } else {
        out.lines().find(|l| !l.trim().is_empty()).map(String::from)
    }
}

// the output of the scratch engine's console since the last look
pub(crate) fn scratch_out(e: &mut Engine, dev: u32) -> String {
    let mut s = String::new();
    for r in e.take_reports() {
        if let Report::CliOut { dev: d, bytes, .. } = r
            && d == dev
        {
            s.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    s
}

/// the startup configuration device `d` has with `lines` (IOS: configuration commands; Linux:
/// shell commands): a scratch copy boots from the project's configuration and takes them ->
/// Err: the line the device refused and why
fn with_lines(seed: u64, d: &Device, ios: bool, lines: &[String]) -> Result<String, String> {
    let dev = d.id;
    let mut e = Engine::new(seed);
    e.apply(Action::AddDevice(Device { services: Vec::new(), off: false, ..d.clone() })).map_err(|x| x.to_string())?;
    let _ = e.apply(Action::Hostname { dev, name: d.name.clone() });
    if ios {
        for l in ["enable", "configure terminal"] {
            e.cli_line(dev, l).map_err(|x| x.to_string())?;
        }
    }
    let _ = scratch_out(&mut e, dev);
    for l in lines {
        e.cli_line(dev, l).map_err(|x| x.to_string())?;
        let out = scratch_out(&mut e, dev);
        if let Some(why) = refused(ios, &out) {
            return Err(tf("{} lehnt „{}“ ab: {}", &[&d.name, l, why.trim()]));
        }
    }
    if ios {
        let _ = e.cli_line(dev, "end");
    }
    Ok(e.running_config(dev))
}

/// a form's command lines for device `dev` (IOS: configuration commands, they run inside
/// "configure terminal"; Linux: shell commands): they are typed on its console as if by hand
/// and land in its startup configuration -- one undo step (`label`), which restarts the
/// device with the old configuration when it is undone.  A device without power only gets the
/// new startup configuration.  Err: why nothing changed (a line the device refuses, a command
/// still running on the console)
pub(crate) fn run_lines(n: &mut Net, dev: u32, label: &str, lines: &[String]) -> Result<(), String> {
    run_from(n, dev, label, lines, false)
}

/// run_lines for a form that shows what the device runs: the lines are checked on, and the new
/// startup configuration made from, the running configuration of a device with power -- a list
/// typed on the console and not saved yet is there to change, and the step saves it too
pub(crate) fn run_lines_running(n: &mut Net, dev: u32, label: &str, lines: &[String]) -> Result<(), String> {
    run_from(n, dev, label, lines, true)
}

fn run_from(n: &mut Net, dev: u32, label: &str, lines: &[String], running: bool) -> Result<(), String> {
    let d = n.doc.proj.device(dev).cloned().ok_or_else(|| tf("Das Gerät {} gibt es nicht.", &[&dev.to_string()]))?;
    if !has_cli(&n.doc.proj, dev) {
        return Err(tf("{} hat keine Befehlszeile.", &[&d.name]));
    }
    let ios = ios_of(&n.doc.proj, dev);
    let base = if running && !d.off { Device { startup_config: n.doc.sim.engine().running_config(dev), ..d.clone() } } else { d.clone() };
    let text = with_lines(n.doc.proj.seed, &base, ios, lines)?;
    // lines that only run a command ("do clear ...") leave the configuration as it is written
    let text = if lines.iter().all(|l| l.starts_with("do ")) { d.startup_config.clone() } else { text };
    if d.off {
        if let Some(e) = edits::device_edit(&n.doc.proj, dev, label, |x| x.startup_config = text) {
            n.doc.apply(e);
        }
        return Ok(());
    }
    let (busy, asked, mode) = {
        let e = n.doc.sim.engine();
        (e.cli_busy(dev), e.node(dev).is_some_and(|x| x.cli.ask.is_some()), e.node(dev).map(|x| x.cli.mode))
    };
    if busy {
        return Err(tf("Auf {} läuft noch ein Befehl; Strg+C in der Konsole bricht ihn ab.", &[&d.name]));
    }
    if asked {
        interrupt(n, dev);
    }
    // the lines as typed: into the configuration mode from where the console is, back out
    let mut seq: Vec<String> = Vec::new();
    if ios {
        match mode {
            Some(Mode::Out) => seq.extend([String::new(), String::from("enable")]),
            Some(Mode::User) | None => seq.push(String::from("enable")),
            Some(Mode::Priv) => {}
            Some(_) => seq.push(String::from("end")),
        }
        seq.push(String::from("configure terminal"));
    }
    seq.extend(lines.iter().cloned());
    if ios {
        seq.push(String::from("end"));
        if matches!(mode, Some(Mode::User) | Some(Mode::Out) | None) {
            seq.push(String::from("disable"));
        }
    }
    n.doc.hist.group_begin(label);
    for l in &seq {
        typed(n, dev, l);
    }
    if let Some(e) = edits::device_edit(&n.doc.proj, dev, label, |x| x.startup_config = text) {
        n.doc.apply_adopted(e, dev);
    }
    n.doc.hist.group_end();
    Ok(())
}

/// the running configuration of device `dev` becomes its startup configuration (IOS: "copy
/// running-config startup-config" on its console; a shell has no such command)
pub(crate) fn save_running(n: &mut Net, dev: u32) -> Result<(), String> {
    let d = n.doc.proj.device(dev).cloned().ok_or_else(|| tf("Das Gerät {} gibt es nicht.", &[&dev.to_string()]))?;
    if d.off {
        return Err(tf("{} ist ausgeschaltet.", &[&d.name]));
    }
    if n.doc.sim.engine().cli_busy(dev) {
        return Err(tf("Auf {} läuft noch ein Befehl; Strg+C in der Konsole bricht ihn ab.", &[&d.name]));
    }
    if ios_of(&n.doc.proj, dev) {
        if n.doc.sim.engine().node(dev).is_some_and(|x| x.cli.ask.is_some()) {
            interrupt(n, dev);
        }
        let mode = n.doc.sim.engine().node(dev).map(|x| x.cli.mode);
        // "copy" is a privileged command: from the user level and back
        let user = matches!(mode, Some(Mode::User) | Some(Mode::Out));
        if user {
            typed(n, dev, "enable");
        } else if !matches!(mode, Some(Mode::Priv)) {
            typed(n, dev, "end");
        }
        typed(n, dev, "copy running-config startup-config");
        typed(n, dev, "");
        if user {
            typed(n, dev, "disable");
        }
        return Ok(());
    }
    let text = n.doc.sim.engine().running_config(dev);
    saved(n, dev, text);
    Ok(())
}

/// "Von hier fortsetzen" at moment `t` (K1): every console says where the run goes on (what
/// they show above it happened after that moment and is gone)
pub(crate) fn resumed(n: &mut Net, t: jfn_sim::Ns) {
    let secs = crate::workspace::sim::secs(t);
    let line = format!("\x1b[2m--- {} ---\x1b[0m\n", tf("fortgesetzt bei t = {} s: was danach kam, ist verworfen", &[&secs]));
    for c in n.cons.by_dev.values_mut() {
        c.past_note = None;
        c.term.write_str(&line);
    }
}
