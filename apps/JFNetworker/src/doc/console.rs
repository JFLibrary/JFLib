// console -- the consoles of the devices: a ui::TermState per device that printed something or
// was opened, the simulator's reports distributed to them, what is typed sent to the engine,
// and the console tabs of the Terminal panel (workspace/terminal.rs draws them)
//
//   Consoles::new()                  no console (a new project)
//   pump(n)                          the simulator's reports: console output, prompts, saved
//                                    configurations (an undo step that keeps the running
//                                    device), new host names (a rename), log lines (the
//                                    Protokoll panel), power going off and on
//   open(n, ui, dev) -> Result       the device's console as the shown tab of the Terminal
//                                    panel (or its window to the front), with the keyboard
//   close / float / dock / tile      a tab goes, into a window of its own, back, every console
//                                    in a window side by side
//   view(n, ui, dev)                 the console of a device here (the panel, its window, a
//                                    device window): it fills the place, its keys go to the
//                                    engine
//   typed(n, dev, line)              a line as if typed: its echo after the prompt, the
//                                    engine runs it, its output follows
//   run_lines(n, dev, label, lines)  a form's command lines: typed on the console and into the
//                                    startup configuration, one undo step
//   save_running(n, dev)             the running configuration becomes the startup one (IOS:
//                                    "copy running-config startup-config" on the console)
//   resumed(n, t)                    "Von hier fortsetzen" at moment t: every console says so
//
// The engine prints without prompt and echo (ui::term adds them); the prompt and the busy
// state come from the engine whenever a console is drawn.  IOS-like consoles take `?`
// (Engine::cli_help) and Ctrl+Shift+6 and page what a command prints with --More-- (24 lines
// a screen as "terminal length 24"; "terminal length N" changes it, 0 turns it off); Tab completes from the engine's command trees, a Linux shell lists
// several choices.  Ctrl+C reaches the engine while a command runs or a question waits (else
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
}

/// a console in the Terminal panel
pub(crate) struct Tab {
    pub dev: u32,
    /// the title the user gave it (None: the device's name)
    pub title: Option<String>,
    /// in a window of its own: ID_TERM_WIN0 + 0x10 k
    pub win: Option<usize>,
}

/// the rename flyout of a tab
pub(crate) struct Rename {
    pub dev: u32,
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
        Consoles { by_dev: BTreeMap::new(), tabs: Vec::new(), cur: None, focus: None, slots: BTreeMap::new(), wins, rename: None }
    }

    /// a device's console (made when it is first needed)
    pub(crate) fn get(&mut self, p: &Project, dev: u32) -> &mut Console {
        self.by_dev.entry(dev).or_insert_with(|| {
            let mut term = TermState::new(SCROLLBACK);
            term.pager_text = String::from(" --More-- ");
            term.page_len = TERM_LENGTH - 1;
            let power = p.device(dev).is_some_and(|d| !d.off);
            Console { term, ios: ios_of(p, dev), slot: None, nopage: false, power, skip_nl: false, past_note: None }
        })
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
        self.tabs.iter().position(|t| t.dev == dev)
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

// ... with the output of device `paged` paged (IOS: what a command prints) -> that output
fn pump_paged(n: &mut Net, paged: Option<u32>) -> String {
    let mut text = String::new();
    let mut paging = false;
    for r in n.doc.sim.take_reports() {
        match r {
            Report::CliOut { dev, bytes, .. } => {
                let c = n.cons.get(&n.doc.proj, dev);
                let mut b: &[u8] = &bytes;
                if c.skip_nl {
                    c.skip_nl = false;
                    if b.first() == Some(&b'\n') {
                        b = &b[1..];
                    }
                }
                if paged == Some(dev) {
                    if !paging {
                        c.term.pager(true);
                        paging = true;
                    }
                    text.push_str(&String::from_utf8_lossy(b));
                }
                c.term.write(b);
            }
            Report::CliState { dev, prompt, busy } => {
                if let Some(c) = n.cons.by_dev.get_mut(&dev) {
                    c.term.set_prompt(&prompt);
                    c.term.set_mode(if busy { TM_BUSY } else { TM_LINE });
                }
            }
            Report::Startup { dev, text } => saved(n, dev, text),
            Report::Hostname { dev, name } => renamed(n, dev, &name),
            Report::Log { dev, at, sev, tag, text } => n.log.push(dev, at, sev, tag, text),
        }
    }
    if paging && let Some(c) = paged.and_then(|d| n.cons.by_dev.get_mut(&d)) {
        c.term.pager(false);
    }
    // the tabs of devices that are gone close (an undo brings the device, not its tab, back)
    let gone: Vec<u32> = n.cons.tabs.iter().map(|t| t.dev).filter(|&d| n.doc.proj.device(d).is_none()).collect();
    for d in gone {
        close(n, d);
    }
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
    let Some(d) = n.doc.proj.device(dev) else { return Err(tf("Das Gerät {} gibt es nicht.", &[&dev.to_string()])) };
    if !has_cli(&n.doc.proj, dev) {
        return Err(tf("{} hat keine Befehlszeile.", &[&d.name]));
    }
    let cons = &mut n.cons;
    if cons.tab(dev).is_none() {
        cons.tabs.push(Tab { dev, title: None, win: None });
    }
    cons.focus = Some(dev);
    match cons.tab(dev).and_then(|i| cons.tabs[i].win) {
        Some(k) => ui.window_activate(ID_TERM_WIN0 + 0x10 * k as u32),
        None => {
            cons.cur = Some(dev);
            n.shell.dock.show(P_TERMINAL);
        }
    }
    ui.repass();
    Ok(())
}

/// the tab of a console goes (its scrollback stays for the next time)
pub(crate) fn close(n: &mut Net, dev: u32) {
    let cons = &mut n.cons;
    let Some(i) = cons.tab(dev) else { return };
    let t = cons.tabs.remove(i);
    if let Some(k) = t.win {
        cons.wins[k].0.set(0);
    }
    if cons.cur == Some(dev) {
        // the neighbour becomes the shown tab
        let docked: Vec<(usize, u32)> = cons.tabs.iter().enumerate().filter(|(_, t)| t.win.is_none()).map(|(j, t)| (j, t.dev)).collect();
        cons.cur = docked.iter().find(|(j, _)| *j >= i).or(docked.last()).map(|(_, d)| *d);
    }
    if cons.rename.as_ref().is_some_and(|r| r.dev == dev) {
        cons.rename = None;
    }
}

/// a console into a window of its own -> false when every console window is in use
pub(crate) fn float(n: &mut Net, ui: &mut Ui, dev: u32) -> bool {
    let cons = &mut n.cons;
    let Some(i) = cons.tab(dev) else { return false };
    if cons.tabs[i].win.is_some() {
        return true;
    }
    let Some(k) = (0..TERM_WINS).find(|&k| cons.in_win(k).is_none()) else { return false };
    cons.tabs[i].win = Some(k);
    cons.wins[k].0.set(1);
    if cons.cur == Some(dev) {
        cons.cur = cons.tabs.iter().find(|t| t.win.is_none()).map(|t| t.dev);
    }
    cons.focus = Some(dev);
    ui.repass();
    true
}

/// a console from its window back into the panel, as the shown tab
pub(crate) fn dock(n: &mut Net, dev: u32) {
    let cons = &mut n.cons;
    let Some(i) = cons.tab(dev) else { return };
    if let Some(k) = cons.tabs[i].win.take() {
        cons.wins[k].0.set(0);
    }
    cons.cur = Some(dev);
    n.shell.dock.show(P_TERMINAL);
}

/// every open console in a window of its own (as many as there are windows), side by side
/// over the lower half of the main window
pub(crate) fn tile(n: &mut Net, ui: &mut Ui) {
    for dev in n.cons.docked() {
        if !float(n, ui, dev) {
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
    let (prompt, busy) = {
        let e = n.doc.sim.state();
        // in the past a line always runs (on a copy, Sim::past_line)
        let busy = e.cli_busy(dev) && n.doc.sim.past().is_none();
        if on { (e.cli_prompt(dev), busy) } else { (String::new(), false) }
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
    c.term.set_mode(if busy { TM_BUSY } else { TM_LINE });
    let mut flags = if c.ios { TF_QHELP | TF_CTRL_SHIFT6 } else { 0 };
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
    let page = n.cons.by_dev.get(&dev).is_some_and(|c| c.ios && !c.nopage);
    let out = pump_paged(n, if page { Some(dev) } else { None });
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

// ---------------------------------------------------------------- forms

// what the device said to a line is a complaint: IOS "% ..." lines (not the time-stamped
// console messages), anything on a shell (its commands print nothing when they work)
fn refused(ios: bool, out: &str) -> Option<String> {
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
fn scratch_out(e: &mut Engine, dev: u32) -> String {
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
    let d = n.doc.proj.device(dev).cloned().ok_or_else(|| tf("Das Gerät {} gibt es nicht.", &[&dev.to_string()]))?;
    if !has_cli(&n.doc.proj, dev) {
        return Err(tf("{} hat keine Befehlszeile.", &[&d.name]));
    }
    let ios = ios_of(&n.doc.proj, dev);
    let text = with_lines(n.doc.proj.seed, &d, ios, lines)?;
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
