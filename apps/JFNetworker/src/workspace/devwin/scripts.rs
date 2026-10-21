// scripts -- the Skripte tab of a host's device window: the device's scripts (.jfs programs of
// lib/script that run as services, jfn_sim/scriptsvc.rs) in a list, the chosen one in a code
// editor with diagnostics while typing, its output and errors, run / stop / restart, apply
//
//   Drafts::new()                 no script edited (a new project: DevWins::new)
//   Pane::new(dev)                the tab's state in one window (the chosen script, the
//                                 splits, the lower part)
//   tab(n, ui, k, id, dev)        the tab of device window k (ids id .. id + 0x3FF)
//   states(n, ui) / command(n, ui, id) -> handled   CMD_SCRIPTS: the selected host's window on
//                                 this tab (the context menu of a device)
//   check_due(n, ui)              TIMER_SCRIPT_CHECK: the check of every changed draft
//
// The list: the device's scripts in the project's order with their state (läuft, rechnet,
// gestoppt, Fehler, "übersetzt nicht" when the simulator could not take the definition), CPU
// and errors.  "Neu" makes one from a template, "Umbenennen" and "Löschen" change the project;
// each is one undo step that keeps the running device (doc/scripts.rs).
// The editor: ui.code_editor with Highlighter::jfs() and CE_MARKS.  Each script has a draft (a
// CodeDoc with its own undo) that outlives switching scripts and closing the window, until it
// is applied or the project is replaced; an unchanged draft follows the project (an undo of
// "Übernehmen" shows the source before), a changed one keeps the user's text.  Diagnostics:
// script::check at most every CHECK_MS while typing (errors and warnings) and the errors of the
// running program -- the current run's (ScriptService::run_errors), so a restart clears them --
// with their lines mapped from the running source onto the edited text (lines the edits
// replaced lose their mark).  F8 / Shift+F8 in the editor go from one to the next; the status
// line has the caret, the counts and the message at the caret.
// Übernehmen (Ctrl+S in the editor): a source that compiles goes into the project as one undo
// step and into the running script (Action::ScriptSource: the script restarts with it and keeps
// its disk, numbers and errors); a source with errors is refused with the first one.
// Ausführen / Stoppen / Neu starten act in the simulator only (Action::ServiceRun), as the
// shell's script start|stop|restart.  Below the editor: Ausgabe (what the current run printed,
// following live in a console without input) and Fehler (the current run's errors with time,
// line and column; a click puts the caret there); above it the key numbers (state, CPU over the
// last seconds, queue, callbacks, errors, memory, what it has open).
//
// Ids from id: + 1 Neu (its menu + 2 + k), + 8 Umbenennen (its flyout: + 0xB the name, + 0xC OK,
// + 0xD Abbrechen), + 9 Löschen, + 0x10 Ausführen, + 0x11 Stoppen, + 0x12 Neu starten, + 0x13
// Verwerfen, + 0x14 Übernehmen, + 0x16 the outer split, + 0x18 the split of the editor, + 0x1A
// Ausgabe | Fehler, + 0x20 the editor (.. + 0x2F), + 0x30 the output (.. + 0x37), + 0x40 the
// errors (rows + 1 + i), + 0x100 the list (rows + 1 + i).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::catalog::{CliStyle, F_HOST};
use jfn_model::{Device, Project};
use jfn_sim::{Engine, Ns, ScriptErr, ScriptService, ScriptState, ScriptView, SEC};
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{t, tc, tf};
use crate::app::Net;
use crate::doc::scripts;
use crate::workspace::devwin::{self, Tab};

/// the check while typing: at most this often (ms)
pub(crate) const CHECK_MS: u32 = 250;
/// lines the output of a script keeps
const OUT_LINES: usize = 2000;
/// CPU over the last whole seconds, this many at most
const CPU_SECS: u64 = 5;

/// a script being edited
pub(crate) struct Draft {
    pub doc: CodeDoc,
    /// the project's source the draft started from (or was brought to since), how often it
    /// was set, changed() for a revision and that count (the text is joined only then)
    base: String,
    base_gen: u64,
    dirty: core::cell::Cell<(u64, u64, bool)>,
    /// the revision the check's diagnostics belong to, and the diagnostics
    checked: Option<u64>,
    check: Vec<Diag>,
    /// the runtime errors marked: the run, how many, the revision they were mapped onto
    marked: (u64, usize, u64),
    /// the output of the current run, the run it shows, the end of what it shows, whether it
    /// shows anything
    out: TermState,
    out_run: u64,
    out_at: u64,
    out_any: bool,
}

impl Draft {
    fn new(src: &str) -> Draft {
        let mut doc = CodeDoc::from_text(src);
        doc.tab_width = 2;
        Draft {
            doc,
            base: String::from(src),
            base_gen: 0,
            dirty: core::cell::Cell::new((u64::MAX, u64::MAX, false)),
            checked: None,
            check: Vec::new(),
            marked: (u64::MAX, 0, 0),
            out: TermState::new(OUT_LINES),
            out_run: u64::MAX,
            out_at: 0,
            out_any: false,
        }
    }
    /// the text differs from the project's source
    fn changed(&self) -> bool {
        let key = (self.doc.revision(), self.base_gen);
        let c = self.dirty.get();
        if (c.0, c.1) == key {
            return c.2;
        }
        let v = self.doc.text() != self.base;
        self.dirty.set((key.0, key.1, v));
        v
    }
    /// the project's source the draft is compared with
    fn set_base(&mut self, src: String) {
        self.base = src;
        self.base_gen += 1;
    }
}

/// the drafts of every device window, by device and script name
pub(crate) struct Drafts {
    map: BTreeMap<(u32, String), Draft>,
    /// the check timer is set
    pending: bool,
}

impl Drafts {
    pub(crate) fn new() -> Drafts {
        Drafts { map: BTreeMap::new(), pending: false }
    }
}

/// the tab's state in one device window
pub(crate) struct Pane {
    dev: u32,
    /// the chosen script
    sel: Option<String>,
    /// the list's width, the lower part's height (DIP)
    split: i32,
    lower_h: i32,
    /// 0 Ausgabe, 1 Fehler
    lower: i32,
    errs: i32,
    /// the rename flyout's text (Some while it is open)
    rename: Option<String>,
    /// the last refusal (apply, rename, run)
    msg: Option<String>,
    /// the editor gets the keyboard until then (ms of ui.now(): the click that asked for it
    /// may still be under way in a list, which takes the keyboard on a press), and then the
    /// caret goes to line / column (the editor scrolls to its caret while it is focused)
    focus_until: u32,
    goto: Option<(usize, usize)>,
}

impl Pane {
    pub(crate) fn new(dev: u32) -> Pane {
        Pane { dev, sel: None, split: 230, lower_h: 150, lower: 0, errs: -1, rename: None, msg: None, focus_until: 0, goto: None }
    }
}

/// device `dev` gets the tab: a host with a Linux shell (the devices whose OS runs scripts and
/// that have a desktop)
pub(crate) fn has_tab(p: &Project, dev: u32) -> bool {
    p.model_of(dev).is_some_and(|m| m.has(F_HOST) && m.cli == CliStyle::Linux)
}

// ---------------------------------------------------------------- what the simulator says

/// a script as the simulator runs it, for the window
struct Live {
    view: Option<ScriptView>,
    /// the current run's errors and the source it runs (only for the chosen script)
    errors: Vec<ScriptErr>,
    source: String,
    /// CPU in per mille over the last seconds
    cpu_pm: u64,
    /// why the simulator does not run the definition
    refused: Option<String>,
}

/// CPU of a script in per mille over the last whole seconds (at most CPU_SECS)
fn cpu_pm(s: &ScriptService, now: Ns) -> u64 {
    let secs = (now / SEC).min(CPU_SECS);
    if secs == 0 {
        return if now > 0 { (s.load(now, 1) as u128 * 1000 / now as u128) as u64 } else { 0 };
    }
    (s.load(now - SEC, secs) as u128 * 1000 / (secs as u128 * SEC as u128)) as u64
}

/// what the simulator says of script `name` of device `d` (full: its errors and source too)
fn live(e: &Engine, d: &Device, name: &str, full: bool) -> Live {
    let now = e.now();
    match e.service::<ScriptService>(d.id, name) {
        Some(s) => Live {
            view: Some(s.view(now)),
            errors: if full { s.run_errors() } else { Vec::new() },
            source: if full { String::from(s.source()) } else { String::new() },
            cpu_pm: cpu_pm(s, now),
            refused: None,
        },
        None => {
            let refused = scripts::find(d, name).and_then(|j| jfn_sim::scriptsvc::from_def_msg(&d.services[j].config).err()).map(|m| crate::app::tr::msg(&m));
            Live { view: None, errors: Vec::new(), source: String::new(), cpu_pm: 0, refused }
        }
    }
}

/// a state as a word and a badge
fn state_of(l: &Live, off: bool) -> (&'static str, u32) {
    match &l.view {
        None if l.refused.is_some() => (t("übersetzt nicht"), BADGE_CRIT),
        None => (t("nicht im Simulator"), BADGE_NEUTRAL),
        Some(_) if off => (t("Gerät aus"), BADGE_NEUTRAL),
        Some(v) => match v.state {
            ScriptState::Running if v.busy => (t("rechnet"), BADGE_OK),
            ScriptState::Running => (t("läuft"), BADGE_OK),
            ScriptState::Stopped => (t("gestoppt"), BADGE_NEUTRAL),
            ScriptState::Failed => (t("mit Fehler beendet"), BADGE_CRIT),
        },
    }
}

/// "12 %", "< 1 %" of per mille
fn percent(pm: u64) -> String {
    match pm {
        0 => String::from("0 %"),
        1..=9 => String::from("< 1 %"),
        p => format!("{} %", (p + 5) / 10),
    }
}

/// "34 KiB" of bytes
fn size(b: u64) -> String {
    if b >= 1 << 20 { format!("{} MiB", b >> 20) } else if b >= 1 << 10 { format!("{} KiB", b >> 10) } else { format!("{} B", b) }
}

/// the key numbers of a running script
fn numbers(l: &Live) -> String {
    let Some(v) = &l.view else {
        return l.refused.clone().map_or_else(|| String::from(t("Der Simulator führt dieses Skript nicht aus.")), |r| tf("Der Simulator führt es nicht aus: {}", &[&r]));
    };
    let ports: Vec<String> = v.ports.iter().map(|(k, p)| format!("{} {}", k, p)).collect();
    let mut s = tf("CPU {} · Warteschlange {} (höchstens {}) · Rückrufe {} · Fehler {} · Speicher {}", &[&percent(l.cpu_pm), &v.queued.to_string(), &v.max_queued.to_string(), &v.calls.to_string(), &v.errors.to_string(), &size(v.heap)]);
    if !ports.is_empty() {
        s.push_str(" · ");
        s.push_str(&tf("offen: {}", &[&ports.join(", ")]));
    }
    s
}

// ---------------------------------------------------------------- diagnostics

/// the check's diagnostics of a text (errors and warnings of script::check)
fn check_of(text: &str) -> Vec<Diag> {
    script::check(text).iter().map(|x| Diag::new(x.line as usize, x.col as usize, x.end_col as usize, if x.error { DIAG_ERROR } else { DIAG_WARNING }, &x.msg)).collect()
}

/// where line `line` (from 1) of the running source is in the edited text: lines before the
/// first difference stay, those after the last one move with it, a changed one is gone
pub(crate) fn map_line(run: &[&str], now: &[String], line: usize) -> Option<usize> {
    let pre = run.iter().zip(now).take_while(|(a, b)| **a == b.as_str()).count();
    let post = run[pre..].iter().rev().zip(now[pre..].iter().rev()).take_while(|(a, b)| **a == b.as_str()).count();
    let i = line.checked_sub(1)?;
    if i < pre {
        Some(line)
    } else if i >= run.len() - post && i < run.len() {
        Some((i as isize + now.len() as isize - run.len() as isize) as usize + 1)
    } else {
        None
    }
}

/// the columns (from 1, characters; the end exclusive) of the word that starts at column `col`
/// of `text` (one character when it is none)
pub(crate) fn word_at(text: &str, col: usize) -> (usize, usize) {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let n = text.chars().skip(col.saturating_sub(1)).take_while(|&c| word(c)).count();
    (col.max(1), col.max(1) + n.max(1))
}

/// the message of a runtime error for its mark and the list
fn err_text(e: &ScriptErr) -> String {
    tf("Fehler im Lauf ({}, bei {}): {}", &[e.during, &when(e.at), &e.msg])
}

/// "3,000 s" (English "3.000 s") of a virtual time, to the millisecond
fn when(t: Ns) -> String {
    let dot = if crate::app::tr::en() { '.' } else { ',' };
    format!("{}{}{:03} s", t / SEC, dot, t % SEC / 1_000_000)
}

/// the runtime errors as marks on the edited text
fn runtime_marks(run_src: &str, doc: &CodeDoc, errs: &[ScriptErr]) -> Vec<Diag> {
    let run: Vec<&str> = run_src.lines().collect();
    let now = doc.lines();
    errs.iter()
        .filter_map(|e| {
            let line = map_line(&run, now, e.line as usize)?;
            let (a, b) = word_at(&now[line - 1], e.col as usize);
            Some(Diag::new(line, a, b, DIAG_ERROR, &err_text(e)))
        })
        .collect()
}

/// the check's and the runtime errors' diagnostics into the document
fn mark(dr: &mut Draft, l: &Live) {
    let mut d = dr.check.clone();
    if l.view.is_some() {
        d.extend(runtime_marks(&l.source, &dr.doc, &l.errors));
    }
    dr.doc.set_diagnostics(&d);
    dr.marked = (l.view.as_ref().map_or(0, |v| v.runs), l.errors.len(), dr.doc.revision());
}

/// checks the draft's text now and marks it
fn check_now(dr: &mut Draft, l: &Live) {
    dr.check = check_of(&dr.doc.text());
    dr.checked = Some(dr.doc.revision());
    mark(dr, l);
}

/// TIMER_SCRIPT_CHECK: every draft whose text changed since its check
pub(crate) fn check_due(n: &mut Net, ui: &mut Ui) {
    n.devwin.drafts.pending = false;
    let keys: Vec<(u32, String)> = n.devwin.drafts.map.iter().filter(|(_, d)| d.checked != Some(d.doc.revision())).map(|(k, _)| k.clone()).collect();
    for (dev, name) in keys {
        let Some(d) = n.doc.proj.device(dev).cloned() else { continue };
        let l = live(n.doc.sim.engine(), &d, &name, true);
        if let Some(dr) = n.devwin.drafts.map.get_mut(&(dev, name)) {
            check_now(dr, &l);
        }
    }
    ui.repass();
}

/// a check soon (the timer, when it is not set yet)
fn check_soon(n: &mut Net, ui: &mut Ui) {
    if !n.devwin.drafts.pending {
        n.devwin.drafts.pending = true;
        ui.timer(TIMER_SCRIPT_CHECK, CHECK_MS);
    }
}

/// the text of a draft becomes `text` as one step of its own undo; the caret stays on its line
fn set_text(doc: &mut CodeDoc, text: &str) {
    let caret = doc.selection().caret;
    let end = doc.end();
    let _ = doc.replace(TextPos::default(), end, text);
    let p = doc.clamp(caret);
    doc.set_selection(TextSel::at(p));
}

/// the caret to line / column (from 1, characters) of the draft
fn goto(doc: &mut CodeDoc, line: usize, col: usize) {
    let l = line.clamp(1, doc.line_count()) - 1;
    let text = doc.line(l);
    let byte = text.char_indices().nth(col.saturating_sub(1)).map_or(text.len(), |(b, _)| b);
    doc.set_selection(TextSel::at(TextPos::new(l, byte)));
}

// ---------------------------------------------------------------- the tab

/// what the tab's buttons asked for (done after the widgets)
enum Act {
    New(usize),
    Rename(String),
    Delete,
    Run(bool),
    Restart,
    Apply,
    Discard,
    Goto(usize, usize),
}

/// the Skripte tab of device window k (ids id .. id + 0x3FF)
pub(crate) fn tab(n: &mut Net, ui: &mut Ui, k: usize, id: u32, dev: u32) {
    let Some(d) = n.doc.proj.device(dev).cloned() else { return };
    let names: Vec<String> = scripts::of(&d).iter().map(|s| s.name.clone()).collect();
    let Some(w) = n.devwin.get(k) else { return };
    if w.scr.dev != dev {
        w.scr = Pane::new(dev);
    }
    if w.scr.sel.as_ref().is_none_or(|s| !names.contains(s)) {
        w.scr.sel = names.first().cloned();
    }
    let sel = w.scr.sel.clone();
    let lives: Vec<Live> = names.iter().map(|nm| live(n.doc.sim.engine(), &d, nm, sel.as_ref() == Some(nm))).collect();
    let mut acts: Vec<Act> = Vec::new();
    let mut pick: Option<String> = None;
    let mut split = n.devwin.get(k).map_or(230, |w| w.scr.split);
    ui.next_size(-1, -1);
    ui.split_begin(id + 0x16, split, false);
    list(n, ui, k, id, &d, &names, &lives, sel.as_deref(), &mut acts, &mut pick);
    ui.split_next();
    match sel.as_ref().and_then(|s| names.iter().position(|x| x == s)) {
        Some(i) => editor(n, ui, k, id, &d, &names[i], &lives[i], &mut acts),
        None => {
            ui.next_size(-1, -1);
            ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER | pad(12), 0);
            ui.empty_state(IC_DOCUMENT, t("Noch keine Skripte"), t("Ein Skript ist ein eigenes Programm in der Sprache .jfs, das auf diesem Gerät als Dienst läuft: UDP, TCP, HTTP, Timer, die Disk. „Neu“ legt eines aus einer Vorlage an."));
            ui.box_end();
        }
    }
    if ui.split_end(&mut split)
        && let Some(w) = n.devwin.get(k)
    {
        w.scr.split = split;
    }
    if let Some(p) = pick
        && let Some(w) = n.devwin.get(k)
    {
        w.scr.sel = Some(p);
        w.scr.msg = None;
        w.scr.errs = -1;
        ui.repass();
    }
    if !acts.is_empty() {
        act(n, ui, k, dev, sel.as_deref(), acts);
        ui.repass();
    }
}

// the list of scripts with Neu, Umbenennen, Löschen
#[allow(clippy::too_many_arguments)]
fn list(n: &mut Net, ui: &mut Ui, k: usize, id: u32, d: &Device, names: &[String], lives: &[Live], sel: Option<&str>, acts: &mut Vec<Act>, pick: &mut Option<String>) {
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | pad(2), 6);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
    ui.next_icon(IC_ADD);
    ui.next_flags(ST_COMPACT);
    ui.dropdown_button(id + 1, t("Neu"));
    ui.tooltip(t("Ein neues Skript aus einer Vorlage"));
    if ui.flyout_begin(id + 1, FL_ALIGN_END) {
        for (i, tpl) in scripts::TEMPLATES.iter().enumerate() {
            if ui.menu_item(id + 2 + i as u32, t(tpl.title)) & EV_ACT != 0 {
                acts.push(Act::New(i));
                ui.popup_close();
            }
        }
    }
    ui.flyout_end();
    ui.spacer(-1);
    // Umbenennen (a flyout with the name), Löschen
    let none = if sel.is_none() { ST_DISABLED } else { 0 };
    ui.next_icon(IC_EDIT);
    ui.next_flags(ST_COMPACT | ST_SUBTLE | ST_ICON_ONLY | none);
    let ev = ui.dropdown_button(id + 8, t("Umbenennen"));
    rename_flyout(n, ui, k, id, sel, ev, acts);
    ui.next_icon(IC_DELETE);
    ui.next_flags(ST_COMPACT | ST_SUBTLE | ST_ICON_ONLY | none);
    if ui.button(id + 9, t("Löschen")) & EV_ACT != 0 {
        acts.push(Act::Delete);
    }
    ui.tooltip(t("Löscht das Skript und was es auf der Disk abgelegt hat (Rückgängig bringt das Skript zurück, die Disk beginnt leer)"));
    ui.box_end();
    if names.is_empty() {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &tf("{} hat noch kein Skript.", &[&d.name]));
        ui.spacer(-1);
    } else {
        let mut i_sel = sel.and_then(|s| names.iter().position(|x| x == s)).map_or(-1, |i| i as i32);
        ui.next_size(-1, -1);
        let (first, end) = ui.list_begin(id + 0x100, &mut i_sel, names.len() as i32, 40);
        for i in first..end {
            let (nm, l) = (&names[i as usize], &lives[i as usize]);
            let (word, badge) = state_of(l, d.off);
            let changed = n.devwin.drafts.map.get(&(d.id, nm.clone())).is_some_and(|x| x.changed());
            let mut detail = String::from(word);
            if let Some(v) = &l.view {
                detail.push_str(&format!(" · {}", percent(l.cpu_pm)));
                if v.errors > 0 {
                    detail.push_str(" · ");
                    detail.push_str(&if v.errors == 1 { String::from(t("1 Fehler")) } else { tf("{} Fehler", &[&v.errors.to_string()]) });
                }
            }
            if changed {
                detail.push_str(" · ");
                detail.push_str(t("geändert"));
            }
            ui.list_item_begin(i);
            ui.status_dot(badge);
            ui.box_begin(0, BOX_V, 0);
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_ELLIPSIS, nm);
            ui.next_size(-1, 0);
            ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &detail);
            ui.box_end();
            ui.list_item_end();
        }
        let (ev, s) = ui.list_end(&mut i_sel);
        if ev & EV_ACT != 0
            && let Some(nm) = names.get(s.max(0) as usize)
        {
            *pick = Some(nm.clone());
        }
    }
    ui.box_end();
}

// the flyout of Umbenennen: the new name, OK, Abbrechen
fn rename_flyout(n: &mut Net, ui: &mut Ui, k: usize, id: u32, sel: Option<&str>, ev: u32, acts: &mut Vec<Act>) {
    let Some(w) = n.devwin.get(k) else { return };
    let open = ev & EV_OPEN != 0;
    if open && w.scr.rename.is_none() {
        w.scr.rename = Some(String::from(sel.unwrap_or("")));
        ui.focus(id + 0xB);
    } else if !open {
        w.scr.rename = None;
    }
    let mut done = 0; // 1 OK, 2 Abbrechen
    if ui.flyout_begin(id + 8, FL_BELOW) {
        if let Some(text) = w.scr.rename.as_mut() {
            ui.box_begin(0, BOX_V | pad(8), 6);
            ui.text(TS_BODY_STRONG, t("Neuer Name des Skripts"));
            ui.next_size(220, 0);
            let tev = ui.textbox(id + 0xB, None, text, 32);
            if tev & EV_COMMIT != 0 && ui.focused() == id + 0xB {
                done = 1;
            }
            ui.box_begin(0, BOX_H | BOX_END, 8);
            if ui.accent_button(id + 0xC, t("Umbenennen")) & EV_ACT != 0 {
                done = 1;
            }
            if ui.button(id + 0xD, t("Abbrechen")) & EV_ACT != 0 {
                done = 2;
            }
            ui.box_end();
            ui.box_end();
        }
    }
    ui.flyout_end();
    if done != 0 {
        if let Some(to) = w.scr.rename.take().filter(|_| done == 1) {
            acts.push(Act::Rename(to));
        }
        ui.popup_close();
    }
}

// the chosen script: its head with the buttons, the key numbers, the editor and its status, the
// output and the errors
#[allow(clippy::too_many_arguments)]
fn editor(n: &mut Net, ui: &mut Ui, k: usize, id: u32, d: &Device, name: &str, l: &Live, acts: &mut Vec<Act>) {
    let src = scripts::find(d, name).map(|j| scripts::source_of(&d.services[j])).unwrap_or_default();
    let key = (d.id, String::from(name));
    // the draft: new, or following the project while it is unchanged
    let dr = n.devwin.drafts.map.entry(key.clone()).or_insert_with(|| Draft::new(&src));
    if dr.base != src {
        if !dr.changed() {
            set_text(&mut dr.doc, &src);
            dr.checked = None;
        }
        dr.set_base(src.clone());
    }
    if dr.checked.is_none() {
        check_now(dr, l);
    }
    let runs = l.view.as_ref().map_or(0, |v| v.runs);
    if dr.checked == Some(dr.doc.revision()) && dr.marked != (runs, l.errors.len(), dr.doc.revision()) {
        mark(dr, l);
    }
    // the output of the current run, what is new of it
    if let Some(s) = n.doc.sim.engine().service::<ScriptService>(d.id, name) {
        if runs != dr.out_run {
            dr.out.clear();
            dr.out_run = runs;
            dr.out_at = 0;
            dr.out_any = false;
        }
        let (text, end) = s.run_output(dr.out_at);
        if !text.is_empty() {
            dr.out.write_str(text);
            dr.out_any = true;
        }
        dr.out_at = end;
    }
    let changed = dr.changed();
    // (a check that is still due: the refusal of apply says it)
    let has_errors = dr.checked == Some(dr.doc.revision()) && dr.check.iter().any(|x| x.severity == DIAG_ERROR);
    let (word, badge) = state_of(l, d.off);
    let running = l.view.as_ref().is_some_and(|v| v.state == ScriptState::Running);
    let (has, off) = (l.view.is_some(), d.off);
    // the head: name, state; run, stop, restart; discard, apply
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | pad(2), 6);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
    ui.status_dot(badge);
    ui.text(TS_BODY_STRONG, name);
    ui.text(TS_BODY | TX_SECONDARY, word);
    ui.spacer(-1);
    let flag = |on: bool| if on { 0 } else { ST_DISABLED };
    // (icons: their labels are the tooltips and the names for screen readers)
    ui.next_icon(IC_PLAY);
    ui.next_flags(ST_COMPACT | ST_ICON_ONLY | flag(has && !running && !off));
    if ui.button(id + 0x10, t("Ausführen")) & EV_ACT != 0 {
        acts.push(Act::Run(true));
    }
    ui.next_icon(IC_STOP);
    ui.next_flags(ST_COMPACT | ST_ICON_ONLY | flag(running));
    if ui.button(id + 0x11, t("Stoppen")) & EV_ACT != 0 {
        acts.push(Act::Run(false));
    }
    ui.next_icon(IC_REFRESH);
    ui.next_flags(ST_COMPACT | ST_ICON_ONLY | flag(has && !off));
    if ui.button(id + 0x12, t("Neu starten")) & EV_ACT != 0 {
        acts.push(Act::Restart);
    }
    ui.tooltip(t("Neu starten: Speicher und Timer beginnen leer, die Disk und die Zahlen bleiben"));
    ui.next_size(1, 20);
    ui.separator();
    ui.next_flags(ST_COMPACT | flag(changed));
    if ui.button(id + 0x13, t("Verwerfen")) & EV_ACT != 0 {
        acts.push(Act::Discard);
    }
    ui.tooltip(t("Holt die Fassung des Projekts zurück (Strg+Z im Editor nimmt das zurück)"));
    ui.next_icon(IC_SAVE);
    ui.next_flags(ST_COMPACT | flag(changed && !has_errors));
    if ui.accent_button(id + 0x14, t("Übernehmen")) & EV_ACT != 0 {
        acts.push(Act::Apply);
    }
    ui.tooltip(t("Ins Projekt (ein Rückgängig-Schritt) und in das laufende Skript, das damit neu startet und seine Disk und Zahlen behält (Strg+S)"));
    ui.box_end();
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &numbers(l));
    // the editor above, output and errors below (that part keeps its height)
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_FIXED, 0);
    let total = ui.px2dip(ui.box_rect()[3]);
    let lower_h = n.devwin.get(k).map_or(150, |w| w.scr.lower_h);
    let mut pos = if total > 0 { (total - lower_h - 6).max(80) } else { 260 };
    ui.next_size(-1, -1);
    ui.split_begin(id + 0x18, pos, true);
    code(n, ui, k, id, &key, acts);
    ui.split_next();
    lower(n, ui, k, id, &key, l, acts);
    if ui.split_end(&mut pos)
        && total > 0
        && let Some(w) = n.devwin.get(k)
    {
        w.scr.lower_h = (total - pos - 6).max(60);
    }
    ui.box_end();
    ui.box_end();
}

// the code editor and its status line
fn code(n: &mut Net, ui: &mut Ui, k: usize, id: u32, key: &(u32, String), acts: &mut Vec<Act>) {
    let cid = id + 0x20;
    let until = n.devwin.get(k).map_or(0, |w| w.scr.focus_until);
    if until != 0 {
        let over = ui.now() >= until;
        // (a device window is a tool window: a click on a list does not give it the
        // keyboard, the editor needs it)
        let win = ID_DEVWIN0 + 0x200 * k as u32;
        if !over && ui.window_state(win) & UWS_ACTIVE == 0 {
            ui.window_activate(win);
        }
        if !over && ui.focused() != cid {
            ui.focus(cid);
        }
        // the caret where it was asked for, once the editor has the keyboard (or the time is
        // over): the editor shows it
        let go = n.devwin.get(k).and_then(|w| {
            if over {
                w.scr.focus_until = 0;
            }
            w.scr.goto.take()
        });
        if let Some((line, col)) = go
            && let Some(dr) = n.devwin.drafts.map.get_mut(key)
        {
            goto(&mut dr.doc, line, col);
        }
    }
    // Ctrl+S in the editor applies (instead of saving the project)
    if ui.focused() == cid && ui.capture_shortcut(b'S' as u32, MOD_CTRL) {
        acts.push(Act::Apply);
    }
    let Some(dr) = n.devwin.drafts.map.get_mut(key) else { return };
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 4);
    ui.next_size(-1, -1);
    let ev = ui.code_editor(cid, &mut dr.doc, &Highlighter::jfs(), CE_MARKS);
    let changed = dr.changed();
    let st = ui.code_status(cid);
    let at = ui.code_diag(cid);
    if ev & EV_ACT != 0 {
        // (the editor drew the marks as they were; they follow the edit until the check)
        check_soon(n, ui);
        // a refusal was about the text before
        if let Some(w) = n.devwin.get(k) {
            w.scr.msg = None;
        }
    }
    let msg = n.devwin.get(k).and_then(|w| w.scr.msg.clone());
    ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
    ui.text(TS_CAPTION | TX_SECONDARY, &tf("Zeile {}, Spalte {}", &[&st.line.to_string(), &st.col.to_string()]));
    let (e, w) = (st.diags[0], st.diags[1]);
    let counts = match (e, w) {
        (0, 0) => String::from(t("keine Meldungen")),
        _ => {
            let es = if e == 1 { String::from(t("1 Fehler")) } else { tf("{} Fehler", &[&e.to_string()]) };
            let ws = if w == 1 { String::from(t("1 Warnung")) } else { tf("{} Warnungen", &[&w.to_string()]) };
            format!("{}, {}", es, ws)
        }
    };
    ui.text(TS_CAPTION | if e > 0 { 0 } else { TX_SECONDARY }, &counts);
    ui.next_size(-1, 0);
    match (&msg, &at) {
        (Some(m), _) => crate::workspace::error_text(ui, m, 0),
        (None, Some(x)) => ui.text(TS_CAPTION | TX_ELLIPSIS, &format!("{}:{}: {}", x.line, x.col, x.msg)),
        (None, None) => ui.text(TS_CAPTION | TX_TERTIARY | TX_ELLIPSIS, if changed { t("geändert – Strg+S übernimmt, F8 springt zur nächsten Meldung") } else { t("wie im Projekt") }),
    }
    ui.box_end();
    ui.box_end();
}

// below the editor: Ausgabe (a console without input) or Fehler (the current run's errors)
fn lower(n: &mut Net, ui: &mut Ui, k: usize, id: u32, key: &(u32, String), l: &Live, acts: &mut Vec<Act>) {
    let Some((mut sel, errs)) = n.devwin.get(k).map(|w| (w.scr.lower, w.scr.errs)) else { return };
    let errs_title = if l.errors.is_empty() { String::from(tc("Fehler|Liste")) } else { tf("Fehler ({})", &[&l.errors.len().to_string()]) };
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 4);
    if ui.tabs(id + 0x1A, &mut sel, &[t("Ausgabe"), &errs_title]) & EV_ACT != 0 {
        if let Some(w) = n.devwin.get(k) {
            w.scr.lower = sel;
        }
        ui.repass();
    }
    if sel == 0 {
        if let Some(dr) = n.devwin.drafts.map.get_mut(key) {
            ui.next_size(-1, -1);
            if !dr.out_any {
                ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER, 0);
                ui.text(TS_CAPTION | TX_TERTIARY, t("Noch keine Ausgabe in diesem Lauf: print(…) schreibt hierher."));
                ui.box_end();
            } else {
                let _ = ui.terminal(id + 0x30, &mut dr.out, TF_NO_INPUT);
            }
        }
    } else if l.errors.is_empty() {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER, 0);
        let earlier = l.view.as_ref().map_or(0, |v| v.errors);
        ui.text(TS_CAPTION | TX_TERTIARY, &if earlier > 0 { tf("Keine Fehler in diesem Lauf (davor {}; die Konsole zeigt sie mit „script errors“).", &[&earlier.to_string()]) } else { String::from(t("Keine Fehler in diesem Lauf.")) });
        ui.box_end();
    } else {
        let mut esel = errs.min(l.errors.len() as i32 - 1);
        ui.next_size(-1, -1);
        let (first, end) = ui.list_begin(id + 0x40, &mut esel, l.errors.len() as i32, 24);
        for i in first..end {
            let e = &l.errors[i as usize];
            let text = if e.line > 0 { tf("{} · Zeile {}, Spalte {} ({}): {}", &[&when(e.at), &e.line.to_string(), &e.col.to_string(), e.during, &e.msg]) } else { tf("{} · ({}): {}", &[&when(e.at), e.during, &e.msg]) };
            ui.list_item(i, &text);
        }
        let (ev, s) = ui.list_end(&mut esel);
        if let Some(w) = n.devwin.get(k) {
            w.scr.errs = s;
        }
        if ev & (EV_ACT | EV_DOUBLE) != 0
            && let Some(e) = l.errors.get(s.max(0) as usize)
            && e.line > 0
        {
            // where it is in the edited text
            let run: Vec<&str> = l.source.lines().collect();
            if let Some(line) = n.devwin.drafts.map.get(key).and_then(|dr| map_line(&run, dr.doc.lines(), e.line as usize)) {
                acts.push(Act::Goto(line, e.col as usize));
            }
        }
    }
    ui.box_end();
}

// what the buttons asked for
fn act(n: &mut Net, ui: &mut Ui, k: usize, dev: u32, sel: Option<&str>, acts: Vec<Act>) {
    // the editor gets the keyboard for the next 100 ms (a few frames)
    let focus_until = ui.now().wrapping_add(100).max(1);
    for a in acts {
        let r: Result<(), String> = match (a, sel) {
            (Act::New(tpl), _) => scripts::add(&mut n.doc, dev, tpl).map(|name| {
                if let Some(w) = n.devwin.get(k) {
                    w.scr.sel = Some(name);
                    w.scr.focus_until = focus_until;
                }
            }),
            (Act::Rename(to), Some(name)) => scripts::rename(&mut n.doc, dev, name, &to).map(|to| {
                // the draft goes with the script
                if let Some(dr) = n.devwin.drafts.map.remove(&(dev, String::from(name))) {
                    n.devwin.drafts.map.insert((dev, to.clone()), dr);
                }
                if let Some(w) = n.devwin.get(k) {
                    w.scr.sel = Some(to);
                }
            }),
            (Act::Delete, Some(name)) => {
                let r = scripts::remove(&mut n.doc, dev, name);
                if r.is_ok() {
                    n.devwin.drafts.map.remove(&(dev, String::from(name)));
                }
                r
            }
            (Act::Run(on), Some(name)) => scripts::run(&mut n.doc, dev, name, on),
            (Act::Restart, Some(name)) => scripts::restart(&mut n.doc, dev, name),
            (Act::Apply, Some(name)) => {
                let text = n.devwin.drafts.map.get(&(dev, String::from(name))).map(|d| d.doc.text());
                match text {
                    Some(text) => scripts::apply(&mut n.doc, dev, name, &text).map(|_| {
                        if let Some(dr) = n.devwin.drafts.map.get_mut(&(dev, String::from(name))) {
                            dr.set_base(text);
                        }
                    }),
                    None => Ok(()),
                }
            }
            (Act::Discard, Some(name)) => {
                if let Some(dr) = n.devwin.drafts.map.get_mut(&(dev, String::from(name))) {
                    let base = dr.base.clone();
                    set_text(&mut dr.doc, &base);
                    dr.checked = None;
                }
                Ok(())
            }
            (Act::Goto(line, col), Some(_)) => {
                if let Some(w) = n.devwin.get(k) {
                    w.scr.focus_until = focus_until;
                    w.scr.goto = Some((line, col));
                }
                Ok(())
            }
            _ => Ok(()),
        };
        if let Some(w) = n.devwin.get(k) {
            w.scr.msg = r.err();
        }
    }
    ui.repass();
}

// ---------------------------------------------------------------- the command

// the selected device when it is one host
fn selected_host(n: &Net) -> Option<u32> {
    match crate::workspace::canvas::single_item(n)? {
        crate::doc::edits::Item::Device(d) if has_tab(&n.doc.proj, d) => Some(d),
        _ => None,
    }
}

/// CMD_SCRIPTS: hidden unless one host is selected
pub(crate) fn states(n: &Net, ui: &mut Ui) {
    ui.cmd_state(CMD_SCRIPTS, if selected_host(n).is_some() { 0 } else { CF_DISABLED | CF_HIDDEN });
}

/// CMD_SCRIPTS -> handled
pub(crate) fn command(n: &mut Net, ui: &mut Ui, id: u32) -> bool {
    if id != CMD_SCRIPTS {
        return false;
    }
    if let Some(d) = selected_host(n) {
        devwin::open_on(n, ui, d, Tab::Scripts);
    }
    ui.repass();
    true
}
