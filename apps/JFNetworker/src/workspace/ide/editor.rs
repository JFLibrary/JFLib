// editor -- the IDE's drafts and its editor area: the tabs of open files, the code editor with
// SK6's language services, the outline, the status line
//
//   Draft                        a file or inline script being edited: a CodeDoc with its own
//                                undo, the saved text it started from, the check's messages
//   follow(n, ui, k, d, st)      every draft of the device follows the project while it is
//                                unchanged (an undo of a save shows the text before); tabs of
//                                what is gone close unless their draft has changes
//   open(n, ui, k, cx, key, at) / close(n, k, i)   a tab (and its draft) / the tab goes, its
//                                draft stays (closing the window keeps them too)
//   save(n, k, cx, all)          the shown tab, or every changed draft of the device: files as
//                                one undo step of the project (doc/ide.rs save_many), inline
//                                scripts as theirs (doc/scripts.rs apply, which refuses a source
//                                that does not compile)
//   area(n, ui, k, id, cx, acts) the tabs, the editor, the status line (the rename and the
//                                conditional breakpoint flyouts hang there)
//   outline(n, ui, k, id, cx, acts)   the side bar's Gliederung of the shown tab
//   check_due / check_soon       the check while typing (script::check, at most every CHECK_MS)
//   format / definition / rename_open   the language services' commands for the shown tab
//
// A tab shows "●" after its title while its draft differs from the saved text, "(gelöscht)"
// when the file went (saving makes it again).  Diagnostics: the check's errors and warnings and
// the runtime errors of the service the IDE runs (its current run, ScriptService::run_errors)
// in this file, their lines mapped from the text the program runs onto the edited one.  The
// status line has the caret, the counts, the message at the caret, a refusal, or -- when the
// service runs an older text of the file than the saved one -- that a restart takes the saved
// one (with a button).  The language services (devwin/scriptls.rs) read the files a script
// imports from the drafts and the device: completion of a module's names, go to definition
// into the other file (it opens there).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::files::Store;
use jfn_model::Device;
use jfn_sim::{ScriptErr, ScriptService};
use script::ls::{self, CompleteCx};
use ui::*;

use super::{focus_editor, resolve, Act, Cx, Key, LOW_PROBLEMS};
use crate::app::constants::*;
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::{ide as dide, scripts};
use crate::workspace::devwin::scriptls;
use crate::workspace::devwin::scripts::{map_line, word_at};

/// the check while typing: at most this often (ms)
pub(crate) const CHECK_MS: u32 = 250;

/// a file or an inline script being edited
pub(crate) struct Draft {
    pub(crate) doc: CodeDoc,
    /// the saved text it started from or was brought to (None: the file is gone), how often it
    /// was set, changed() for a revision and that count
    pub(crate) base: Option<String>,
    base_gen: u64,
    dirty: core::cell::Cell<(u64, u64, bool)>,
    /// the revision the check's diagnostics belong to, and the diagnostics
    pub(crate) checked: Option<u64>,
    pub(crate) check: Vec<Diag>,
    /// the runtime errors marked: the run, how many, the revision they were mapped onto
    marked: (u64, usize, u64),
    /// the revision the folding ranges and the outline belong to, the outline
    ls_rev: Option<u64>,
    pub(crate) outline: Vec<scriptls::Entry>,
    /// a jfs program (checked, language services)
    pub(crate) jfs: bool,
}

impl Draft {
    pub(crate) fn new(text: &str, jfs: bool) -> Draft {
        let mut doc = CodeDoc::from_text(text);
        doc.tab_width = 2;
        Draft {
            doc,
            base: Some(String::from(text)),
            base_gen: 0,
            dirty: core::cell::Cell::new((u64::MAX, u64::MAX, false)),
            checked: None,
            check: Vec::new(),
            marked: (u64::MAX, 0, 0),
            ls_rev: None,
            outline: Vec::new(),
            jfs,
        }
    }
    /// the text differs from the saved one
    pub(crate) fn changed(&self) -> bool {
        let key = (self.doc.revision(), self.base_gen);
        let c = self.dirty.get();
        if (c.0, c.1) == key {
            return c.2;
        }
        let v = self.base.as_deref().is_none_or(|b| self.doc.text() != b);
        self.dirty.set((key.0, key.1, v));
        v
    }
    /// the saved text it is compared with
    pub(crate) fn set_base(&mut self, text: Option<String>) {
        self.base = text;
        self.base_gen += 1;
    }
}

/// the saved text of what a key names (None: it is not there, or no text)
pub(crate) fn saved_text(d: &Device, st: &Store, key: &Key) -> Option<String> {
    match key {
        Key::File(p) => dide::text_of(st, p),
        Key::Inline(name) => scripts::find(d, name).filter(|&j| dide::file_of(&d.services[j]).is_none()).map(|j| scripts::source_of(&d.services[j])),
    }
}

/// the folding ranges and the outline follow the text -> they changed
fn refresh(dr: &mut Draft) -> bool {
    if !dr.jfs || dr.ls_rev == Some(dr.doc.revision()) {
        return false;
    }
    scriptls::folds(&mut dr.doc);
    let o = scriptls::outline(&dr.doc.text());
    dr.ls_rev = Some(dr.doc.revision());
    let changed = o != dr.outline;
    dr.outline = o;
    changed
}

/// the text of a draft becomes `text` as one step of its own undo; the caret stays on its line
fn set_text(doc: &mut CodeDoc, text: &str) {
    let caret = doc.selection().caret;
    let end = doc.end();
    let _ = doc.replace(TextPos::default(), end, text);
    let p = doc.clamp(caret);
    doc.set_selection(TextSel::at(p));
}

/// the caret to line / column (from 1, characters)
pub(crate) fn goto(doc: &mut CodeDoc, line: usize, col: usize) {
    let l = line.clamp(1, doc.line_count()) - 1;
    let text = doc.line(l);
    let byte = text.char_indices().nth(col.saturating_sub(1)).map_or(text.len(), |(b, _)| b);
    doc.set_selection(TextSel::at(TextPos::new(l, byte)));
}

/// drafts follow the project; tabs of what is gone go (see the module comment)
pub(crate) fn follow(n: &mut Net, ui: &mut Ui, k: usize, d: &Device, st: &Store) {
    let dev = d.id;
    let mut recheck = false;
    for ((dv, key), dr) in n.devwin.ide.drafts.iter_mut() {
        if *dv != dev {
            continue;
        }
        let now = saved_text(d, st, key);
        if dr.base != now {
            if !dr.changed()
                && let Some(t_) = &now
            {
                set_text(&mut dr.doc, t_);
                dr.checked = None;
                recheck = true;
            }
            dr.set_base(now);
        }
    }
    // tabs without a draft (a rename moved it), or of what is gone and unchanged
    let live: Vec<Key> = n.devwin.ide.drafts.iter().filter(|((dv, _), dr)| *dv == dev && (dr.base.is_some() || dr.changed())).map(|((_, key), _)| key.clone()).collect();
    if let Some(w) = n.devwin.get(k) {
        let p = &mut w.ide;
        let before = p.tabs.len();
        let cur = p.cur_key().cloned();
        p.tabs.retain(|key| live.contains(key));
        if p.tabs.len() != before {
            p.cur = cur.and_then(|c| p.tabs.iter().position(|x| *x == c)).unwrap_or(p.cur.min(p.tabs.len().saturating_sub(1)));
        }
        p.cur = p.cur.min(p.tabs.len().saturating_sub(1));
    }
    // drafts of what is gone that nobody changed go too
    n.devwin.ide.drafts.retain(|(dv, _), dr| *dv != dev || dr.base.is_some() || dr.changed());
    if recheck {
        ui.repass();
    }
}

/// opens a tab of `key` (its draft from the saved text) and shows it; at: the caret's line and
/// column
pub(crate) fn open(n: &mut Net, ui: &Ui, k: usize, cx: &Cx, key: Key, at: Option<(usize, usize)>) -> Result<(), String> {
    let dk = (cx.dev, key.clone());
    if !n.devwin.ide.drafts.contains_key(&dk) {
        let Some(text) = saved_text(&cx.d, &cx.st, &key) else {
            return Err(match &key {
                Key::File(p) if cx.st.exists(p) && !cx.st.is_dir(p) => tf("„{}“ ist keine Textdatei: die IDE zeigt sie nicht.", &[p]),
                Key::File(p) => tf("„{}“ gibt es nicht.", &[p]),
                Key::Inline(name) => tf("Das Skript „{}“ gibt es nicht.", &[name]),
            });
        };
        let jfs = key.jfs();
        n.devwin.ide.drafts.insert(dk, Draft::new(&text, jfs));
    }
    if let Some(w) = n.devwin.get(k) {
        let p = &mut w.ide;
        match p.tabs.iter().position(|x| *x == key) {
            Some(i) => p.cur = i,
            None => {
                // a new tab after the shown one
                let at_ = if p.tabs.is_empty() { 0 } else { (p.cur + 1).min(p.tabs.len()) };
                p.tabs.insert(at_, key);
                p.cur = at_;
            }
        }
        p.msg = None;
    }
    focus_editor(n, ui, k, at);
    Ok(())
}

/// the tab goes (usize::MAX: the shown one); its draft stays
pub(crate) fn close(n: &mut Net, k: usize, i: usize) -> Result<(), String> {
    if let Some(w) = n.devwin.get(k) {
        let p = &mut w.ide;
        let i = if i == usize::MAX { p.cur } else { i };
        if i < p.tabs.len() {
            p.tabs.remove(i);
            if p.cur > i || p.cur >= p.tabs.len() {
                p.cur = p.cur.saturating_sub(1);
            }
        }
    }
    Ok(())
}

/// saves the shown tab, or every changed draft of the device
pub(crate) fn save(n: &mut Net, k: usize, cx: &Cx, all: bool) -> Result<(), String> {
    let keys: Vec<Key> = if all {
        n.devwin.ide.drafts.iter().filter(|((dv, _), dr)| *dv == cx.dev && dr.changed()).map(|((_, key), _)| key.clone()).collect()
    } else {
        cx.cur.iter().cloned().collect()
    };
    let mut files: Vec<(String, String)> = Vec::new();
    let mut inline: Vec<(String, String)> = Vec::new();
    for key in &keys {
        let Some(dr) = n.devwin.ide.drafts.get(&(cx.dev, key.clone())) else { continue };
        if !dr.changed() {
            continue;
        }
        match key {
            Key::File(p) => files.push((p.clone(), dr.doc.text())),
            Key::Inline(name) => inline.push((name.clone(), dr.doc.text())),
        }
    }
    if !files.is_empty() {
        dide::save_many(&mut n.doc, cx.dev, &files, "")?;
        for (p, text) in files {
            if let Some(dr) = n.devwin.ide.drafts.get_mut(&(cx.dev, Key::File(p))) {
                dr.set_base(Some(text));
            }
        }
    }
    for (name, text) in inline {
        scripts::apply(&mut n.doc, cx.dev, &name, &text)?;
        if let Some(dr) = n.devwin.ide.drafts.get_mut(&(cx.dev, Key::Inline(name))) {
            dr.set_base(Some(text));
        }
    }
    if let Some(w) = n.devwin.get(k) {
        w.ide.msg = None;
    }
    Ok(())
}

/// the selected text of the shown tab, else the word at its caret (find's start)
pub(crate) fn selection_word(n: &Net, cx: &Cx) -> String {
    let Some(key) = &cx.cur else { return String::new() };
    let Some(dr) = n.devwin.ide.drafts.get(&(cx.dev, key.clone())) else { return String::new() };
    let s = dr.doc.selected_text();
    if !s.is_empty() && !s.contains('\n') {
        return s;
    }
    scriptls::name_at(&dr.doc, dr.doc.selection().caret)
}

// ---------------------------------------------------------------- diagnostics

/// the check's diagnostics of a text
fn check_of(text: &str) -> Vec<Diag> {
    script::check(text).iter().map(|x| Diag::new(x.line as usize, x.col as usize, x.end_col as usize, if x.error { DIAG_ERROR } else { DIAG_WARNING }, &x.msg)).collect()
}

/// "3,000 s" (English "3.000 s") of a virtual time, to the millisecond
pub(crate) fn when(t_: jfn_sim::Ns) -> String {
    let dot = if crate::app::tr::en() { '.' } else { ',' };
    format!("{}{}{:03} s", t_ / jfn_sim::SEC, dot, t_ % jfn_sim::SEC / 1_000_000)
}

/// the message of a runtime error for its mark
pub(crate) fn err_text(e: &ScriptErr) -> String {
    tf("Fehler im Lauf ({}, bei {}): {}", &[e.during, &when(e.at), &e.msg])
}

/// the runtime errors of the service the IDE runs that belong to `key`, with the text that ran
/// there (the program's source for its own file, else the saved file)
pub(crate) fn runtime_errors(n: &Net, cx: &Cx, key: &Key) -> (Vec<ScriptErr>, String, u64) {
    let Some(t_) = &cx.target else { return (Vec::new(), String::new(), 0) };
    let e = n.doc.sim.engine();
    let Some(s) = e.service::<ScriptService>(cx.dev, t_) else { return (Vec::new(), String::new(), 0) };
    let own = match (key, s.file()) {
        (Key::Inline(name), None) => name == t_,
        (Key::File(p), Some(f)) => cx.st.norm(f).ok().as_deref() == Some(p.as_str()),
        _ => false,
    };
    let file = key.debug_file();
    // (a service from a file names its own file in its errors; an inline one "")
    let errs: Vec<ScriptErr> = s.run_errors().into_iter().filter(|x| x.line > 0 && (x.file == file || (own && x.file.is_empty()))).collect();
    let ran = if own { String::from(s.source()) } else { saved_text(&cx.d, &cx.st, key).unwrap_or_default() };
    (errs, ran, s.view(e.now()).runs)
}

/// the check's and the runtime errors' diagnostics into the document
fn mark(dr: &mut Draft, errs: &[ScriptErr], ran: &str, runs: u64) {
    let mut d = dr.check.clone();
    let run: Vec<&str> = ran.lines().collect();
    let now = dr.doc.lines();
    for e in errs {
        if let Some(line) = map_line(&run, now, e.line as usize) {
            let (a, b) = word_at(&now[line - 1], e.col as usize);
            d.push(Diag::new(line, a, b, DIAG_ERROR, &err_text(e)));
        }
    }
    dr.doc.set_diagnostics(&d);
    dr.marked = (runs, errs.len(), dr.doc.revision());
}

/// TIMER_IDE_CHECK: every draft whose text changed since its check
pub(crate) fn check_due(n: &mut Net, ui: &mut Ui) {
    n.devwin.ide.pending = false;
    for dr in n.devwin.ide.drafts.values_mut() {
        if dr.jfs && dr.checked != Some(dr.doc.revision()) {
            dr.check = check_of(&dr.doc.text());
            dr.checked = Some(dr.doc.revision());
            // (the runtime errors join at the next frame: marked differs)
            dr.marked = (u64::MAX, 0, 0);
        }
    }
    ui.repass();
}

/// a check soon (the timer, when it is not set yet)
pub(crate) fn check_soon(n: &mut Net, ui: &mut Ui) {
    let due = n.devwin.ide.drafts.values().any(|d| d.jfs && d.checked != Some(d.doc.revision()));
    if due && !n.devwin.ide.pending {
        n.devwin.ide.pending = true;
        ui.timer(TIMER_SCRIPT_CHECK, CHECK_MS);
    }
}

// ---------------------------------------------------------------- language services

/// the files' texts as the language services read imports: changed drafts first, then the
/// device's files
struct Files {
    st: Store,
    over: Vec<(String, String)>,
    dir: String,
}

impl Files {
    fn of(n: &Net, cx: &Cx, key: &Key) -> Files {
        let over = n.devwin.ide.drafts.iter().filter(|((dv, k_), dr)| *dv == cx.dev && matches!(k_, Key::File(_)) && dr.changed()).map(|((_, k_), dr)| (String::from(k_.debug_file()), dr.doc.text())).collect();
        Files { st: cx.st.clone(), over, dir: key.dir() }
    }
    fn read(&self, path: &str) -> Option<String> {
        let abs = resolve(&self.dir, path);
        if let Some((_, t_)) = self.over.iter().find(|(p, _)| *p == abs) {
            return Some(t_.clone());
        }
        dide::text_of(&self.st, &abs)
    }
}

/// the go to definition of the word at pos: in the file (the caret goes there) or in a module
/// (Act::OpenAt) -> found one
fn definition_at(dr: &mut Draft, key: &Key, files: &Files, pos: TextPos, acts: &mut Vec<Act>) -> bool {
    let load = |p: &str| files.read(p);
    let host = scriptls::host_items();
    let cx = CompleteCx { host: &host, load: Some(&load) };
    let Some(l) = ls::definition(&dr.doc.text(), dr.doc.flat(pos) as u32, &cx) else { return false };
    match l.path {
        None => {
            let at = dr.doc.unflat(l.span.start as usize);
            dr.doc.set_selection(TextSel::at(at));
        }
        Some(p) => acts.push(Act::OpenAt(Key::File(resolve(&key.dir(), &p)), l.span.line as usize, l.span.col as usize)),
    }
    true
}

/// F12 from the menu: the definition of the word at the caret of the shown tab
pub(crate) fn definition(n: &mut Net, ui: &mut Ui, k: usize, cx: &Cx, pos: Option<TextPos>) -> Result<(), String> {
    let Some(key) = cx.cur.clone() else { return Ok(()) };
    let files = Files::of(n, cx, &key);
    let mut acts = Vec::new();
    let found = match n.devwin.ide.drafts.get_mut(&(cx.dev, key.clone())) {
        Some(dr) => {
            let p = pos.unwrap_or(dr.doc.selection().caret);
            definition_at(dr, &key, &files, p, &mut acts)
        }
        None => false,
    };
    for a in acts {
        if let Act::OpenAt(key, line, col) = a {
            open(n, ui, k, cx, key, Some((line, col)))?;
        }
    }
    if found {
        focus_editor(n, ui, k, None);
        Ok(())
    } else {
        Err(String::from(t("Hier steht kein Name, dessen Deklaration die IDE findet.")))
    }
}

/// Formatieren of the shown tab (one undo step of its draft)
pub(crate) fn format(n: &mut Net, ui: &Ui, k: usize, cx: &Cx) -> Result<(), String> {
    let Some(key) = &cx.cur else { return Ok(()) };
    let Some(dr) = n.devwin.ide.drafts.get_mut(&(cx.dev, key.clone())) else { return Ok(()) };
    if !dr.jfs {
        return Ok(());
    }
    let unit = dr.doc.indent_unit();
    let r = scriptls::format(&mut dr.doc, &unit);
    focus_editor(n, ui, k, None);
    r.map(|_| ()).map_err(|m| tf("Formatieren geht erst ohne Syntaxfehler ({})", &[&m]))
}

/// opens the rename flyout for the name at pos (None: the caret)
pub(crate) fn rename_open(n: &mut Net, ui: &mut Ui, k: usize, id: u32, cx: &Cx, pos: Option<TextPos>) {
    let Some(key) = &cx.cur else { return };
    let Some(dr) = n.devwin.ide.drafts.get(&(cx.dev, key.clone())) else { return };
    if !dr.jfs {
        return;
    }
    let p = pos.unwrap_or(dr.doc.selection().caret);
    let name = scriptls::name_at(&dr.doc, p);
    if let Some(w) = n.devwin.get(k) {
        w.ide.ren = Some(name);
        w.ide.ren_at = p;
        w.ide.ren_msg = None;
    }
    ui.popup_open(id + 0xD0);
    ui.focus(id + 0xD1);
}

// ---------------------------------------------------------------- the area

/// the tabs' titles: the file's name ("●" while changed), with its folder when two are alike
fn titles(n: &Net, dev: u32, tabs: &[Key]) -> Vec<String> {
    let base: Vec<String> = tabs.iter().map(|x| x.title()).collect();
    tabs.iter()
        .enumerate()
        .map(|(i, key)| {
            let mut s = base[i].clone();
            if base.iter().filter(|b| **b == s).count() > 1 {
                s = format!("{} – {}", s, key.dir());
            }
            match n.devwin.ide.drafts.get(&(dev, key.clone())) {
                Some(dr) if dr.base.is_none() => format!("{} {}", s, t("(gelöscht)")),
                Some(dr) if dr.changed() => format!("{} ●", s),
                _ => s,
            }
        })
        .collect()
}

/// the tabs, the editor of the shown one, its status line
pub(crate) fn area(n: &mut Net, ui: &mut Ui, k: usize, id: u32, cx: &Cx, acts: &mut Vec<Act>) {
    let Some(p) = n.devwin.get(k).map(|w| &w.ide) else { return };
    let tabs = p.tabs.clone();
    let mut sel = p.cur as i32;
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 4);
    if tabs.is_empty() {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER | pad(12), 0);
        ui.empty_state(IC_DOCUMENT, t("Keine Datei offen"), t("Ein Doppelklick im Explorer öffnet eine Datei; „Neue Datei …“ legt eine an. Ein Skript, das noch im Projekt steht, öffnet sich unter „Eingebettete Skripte“."));
        ui.box_end();
        ui.box_end();
        return;
    }
    let names = titles(n, cx.dev, &tabs);
    let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
    ui.box_begin(0, BOX_H | BOX_XCENTER, 4);
    ui.next_size(-1, 0);
    if ui.tabs_scrolling(id + 0x150, &mut sel, &refs) & EV_ACT != 0 {
        if let Some(w) = n.devwin.get(k) {
            w.ide.cur = (sel.max(0) as usize).min(tabs.len() - 1);
            w.ide.msg = None;
        }
        focus_editor(n, ui, k, None);
    }
    ui.next_icon(IC_CANCEL);
    ui.next_flags(ST_COMPACT | ST_SUBTLE | ST_ICON_ONLY);
    if ui.button(id + 0x180, t("Tab schließen")) & EV_ACT != 0 {
        acts.push(Act::Close(usize::MAX));
    }
    ui.tooltip(&format!("{} ({})", t("Tab schließen"), ui.key_text(b'W' as u32, MOD_CTRL)));
    ui.box_end();
    let Some(key) = n.devwin.get(k).and_then(|w| w.ide.cur_key().cloned()) else {
        ui.box_end();
        return;
    };
    code(n, ui, k, id, cx, &key, acts);
    ui.box_end();
}

// the code editor of the shown tab and its status line
fn code(n: &mut Net, ui: &mut Ui, k: usize, id: u32, cx: &Cx, key: &Key, acts: &mut Vec<Act>) {
    let cid = id + 0x200;
    let dk = (cx.dev, key.clone());
    // the keyboard and the caret where it was asked for (see scripts.rs: a device window is a
    // tool window, a click in a list does not give the editor the keyboard)
    let until = n.devwin.get(k).map_or(0, |w| w.ide.focus_until);
    if until != 0 {
        let over = ui.now() >= until;
        let win = ID_DEVWIN0 + 0x200 * k as u32;
        if !over && ui.window_state(win) & UWS_ACTIVE == 0 {
            ui.window_activate(win);
        }
        if !over && ui.focused() != cid {
            ui.focus(cid);
        }
        let go = n.devwin.get(k).and_then(|w| {
            if over {
                w.ide.focus_until = 0;
            }
            w.ide.goto.take()
        });
        if let Some((line, col)) = go
            && let Some(dr) = n.devwin.ide.drafts.get_mut(&dk)
        {
            goto(&mut dr.doc, line, col);
        }
    }
    let (errs, ran, runs) = runtime_errors(n, cx, key);
    let files = Files::of(n, cx, key);
    let bps: Vec<(usize, u8)> = n.devwin.ide.bps.get(&dk).map(|v| v.iter().map(|(l, c)| (*l as usize - 1, if c.is_some() { BP_COND } else { BP_ON })).collect()).unwrap_or_default();
    let Some(dr) = n.devwin.ide.drafts.get_mut(&dk) else { return };
    if dr.jfs && dr.checked.is_none() {
        dr.check = check_of(&dr.doc.text());
        dr.checked = Some(dr.doc.revision());
        dr.marked = (u64::MAX, 0, 0);
    }
    if dr.checked == Some(dr.doc.revision()) && dr.marked != (runs, errs.len(), dr.doc.revision()) {
        mark(dr, &errs, &ran, runs);
    }
    if refresh(dr) {
        // (the side bar's outline was drawn before)
        ui.repass();
    }
    if dr.doc.breakpoints() != bps.as_slice() {
        dr.doc.set_breakpoints(&bps);
    }
    let jfs = dr.jfs;
    let hl = if jfs { Highlighter::jfs() } else { Highlighter::for_name(key.debug_file()) };
    ui.next_size(-1, -1);
    let ev = if jfs {
        let load = |p: &str| files.read(p);
        let mut assist = scriptls::Assist::with_loader(&load);
        ui.code_editor_with(cid, &mut dr.doc, &hl, Some(&mut assist), CE_MARKS | CE_BREAKPOINTS | CE_FOLDS)
    } else {
        ui.code_editor(cid, &mut dr.doc, &hl, 0)
    };
    let events = ui.code_events(cid);
    let mut ren: Option<TextPos> = None;
    let mut msg_now: Option<String> = None;
    let mut edited = false;
    for e in events {
        match e {
            CodeEv::Definition(p) => {
                if !definition_at(dr, key, &files, p, acts) {
                    msg_now = Some(String::from(t("Hier steht kein Name, dessen Deklaration die IDE findet.")));
                }
            }
            CodeEv::Rename(p) => ren = Some(p),
            CodeEv::Format => {
                let unit = dr.doc.indent_unit();
                match scriptls::format(&mut dr.doc, &unit) {
                    Ok(did) => edited |= did,
                    Err(m) => msg_now = Some(tf("Formatieren geht erst ohne Syntaxfehler ({})", &[&m])),
                }
            }
            CodeEv::Breakpoint(line) => acts.push(Act::Bp(line + 1, None)),
            _ => {}
        }
    }
    let changed = dr.changed();
    let base = dr.base.clone();
    let st_ = ui.code_status(cid);
    let at = ui.code_diag(cid);
    if ev & EV_ACT != 0 || edited {
        check_soon(n, ui);
        if let Some(w) = n.devwin.get(k) {
            w.ide.msg = None;
        }
        ui.repass();
    }
    if let Some(m) = msg_now
        && let Some(w) = n.devwin.get(k)
    {
        w.ide.msg = Some(m);
    }
    if let Some(p) = ren {
        rename_open(n, ui, k, id, cx, Some(p));
    }
    // a running service of this file with an older text than the saved one
    let stale = match (&cx.target, key, &base) {
        (Some(t_), Key::File(p), Some(b)) => n.doc.sim.engine().service::<ScriptService>(cx.dev, t_).filter(|s| s.state() == jfn_sim::ScriptState::Running && s.file().and_then(|f| cx.st.norm(f).ok()).as_deref() == Some(p.as_str()) && s.source() != b).map(|_| t_.clone()),
        _ => None,
    };
    status(n, ui, k, id, cx, key, &st_, at.as_ref(), changed, stale, acts);
}

// the status line: caret, counts, the message; Neu starten when the program is older than
// the file; the language services' buttons with the rename and condition flyouts
#[allow(clippy::too_many_arguments)]
fn status(n: &mut Net, ui: &mut Ui, k: usize, id: u32, cx: &Cx, key: &Key, st_: &CodeStatus, at: Option<&Diag>, changed: bool, stale: Option<String>, acts: &mut Vec<Act>) {
    let msg = n.devwin.get(k).and_then(|w| w.ide.msg.clone());
    ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
    ui.text(TS_CAPTION | TX_SECONDARY, &tf("Zeile {}, Spalte {}", &[&st_.line.to_string(), &st_.col.to_string()]));
    let (e, w) = (st_.diags[0], st_.diags[1]);
    let counts = match (e, w) {
        (0, 0) => String::from(t("keine Meldungen")),
        _ => {
            let es = if e == 1 { String::from(t("1 Fehler")) } else { tf("{} Fehler", &[&e.to_string()]) };
            let ws = if w == 1 { String::from(t("1 Warnung")) } else { tf("{} Warnungen", &[&w.to_string()]) };
            format!("{}, {}", es, ws)
        }
    };
    ui.text(TS_CAPTION | if e > 0 { 0 } else { TX_SECONDARY }, &counts);
    if e + w > 0 {
        ui.next_flags(ST_COMPACT | ST_SUBTLE);
        if ui.button(id + 0x182, t("Probleme")) & EV_ACT != 0 {
            acts.push(Act::Lower(LOW_PROBLEMS));
        }
    }
    ui.next_size(-1, 0);
    match (&msg, &stale, at) {
        (Some(m), _, _) => crate::workspace::error_text(ui, m, 0),
        (None, Some(s), _) => ui.text(TS_CAPTION | TX_ELLIPSIS, &tf("Der Dienst {} läuft noch mit der Fassung vor dem Speichern – Neu starten übernimmt die Datei.", &[s])),
        (None, None, Some(x)) => ui.text(TS_CAPTION | TX_ELLIPSIS, &format!("{}:{}: {}", x.line, x.col, x.msg)),
        (None, None, None) => {
            let hint = match key {
                _ if changed => t("geändert – Strg+S speichert"),
                Key::File(_) => t("gespeichert"),
                Key::Inline(_) => t("wie im Projekt"),
            };
            ui.text(TS_CAPTION | TX_TERTIARY | TX_ELLIPSIS, hint)
        }
    }
    if stale.is_some() {
        ui.next_flags(ST_COMPACT);
        if ui.button(id + 0x181, t("Neu starten")) & EV_ACT != 0 {
            acts.push(Act::Restart);
        }
    }
    let jfs = key.jfs();
    if jfs {
        ui.next_icon(IC_ALIGN_LEFT);
        ui.next_flags(ST_COMPACT | ST_SUBTLE | ST_ICON_ONLY);
        if ui.button(id + 0x183, t("Formatieren")) & EV_ACT != 0 {
            acts.push(Act::Format);
        }
        ui.tooltip(t("Formatieren (Umschalt+Alt+F): Einrückung, Leerzeichen, eine Anweisung je Zeile – ein Rückgängig-Schritt"));
        ui.next_icon(IC_EDIT);
        ui.next_flags(ST_COMPACT | ST_SUBTLE | ST_ICON_ONLY);
        let rev = ui.dropdown_button(id + 0xD0, t("Name umbenennen"));
        if rev & EV_OPEN != 0 && n.devwin.get(k).is_some_and(|w| w.ide.ren.is_none()) {
            rename_open(n, ui, k, id, cx, None);
        }
        rename_flyout(n, ui, k, id, cx);
        ui.next_icon(IC_CIRCLE_FILL);
        ui.next_flags(ST_COMPACT | ST_SUBTLE | ST_ICON_ONLY);
        let cev = ui.dropdown_button(id + 0xC8, t("Bedingter Haltepunkt"));
        if cev & EV_OPEN != 0 && n.devwin.get(k).is_some_and(|w| w.ide.dbg.cond.is_none()) {
            super::debug::cond_open(n, ui, k, id, cx);
        }
        super::debug::cond_flyout(n, ui, k, id, cx, acts);
    }
    ui.box_end();
}

/// the flyout of Name umbenennen: the new name, a refusal, Umbenennen, Abbrechen
pub(crate) fn rename_flyout(n: &mut Net, ui: &mut Ui, k: usize, id: u32, cx: &Cx) {
    let fid = id + 0xD0;
    if !ui.popup_is_open(fid)
        && let Some(w) = n.devwin.get(k)
    {
        w.ide.ren = None;
        w.ide.fly[1] = 0;
    }
    let Some(w) = n.devwin.get(k) else { return };
    let mut done = 0; // 1 Umbenennen, 2 Abbrechen
    let refusal = w.ide.ren_msg.clone();
    if ui.flyout_begin(fid, FL_ALIGN_END)
        && let Some(text) = w.ide.ren.as_mut()
    {
        ui.box_begin(0, BOX_V | pad(8), 6);
        ui.text(TS_BODY_STRONG, t("Neuer Name (jede Stelle in der Datei)"));
        ui.next_size(240, 0);
        let tev = ui.textbox(id + 0xD1, None, text, 64);
        w.ide.fly[1] += 1;
        if tev & EV_COMMIT != 0 && ui.focused() == id + 0xD1 && w.ide.fly[1] > 1 {
            done = 1;
        }
        if let Some(m) = &refusal {
            crate::workspace::error_text(ui, m, 240);
        }
        ui.box_begin(0, BOX_H | BOX_END, 8);
        if ui.accent_button(id + 0xD2, t("Umbenennen")) & EV_ACT != 0 {
            done = 1;
        }
        if ui.button(id + 0xD3, t("Abbrechen")) & EV_ACT != 0 {
            done = 2;
        }
        ui.box_end();
        ui.box_end();
    }
    ui.flyout_end();
    match done {
        1 => {
            let (to, at) = (w.ide.ren.clone().unwrap_or_default(), w.ide.ren_at);
            let r = cx.cur.as_ref().and_then(|key| n.devwin.ide.drafts.get_mut(&(cx.dev, key.clone()))).map(|dr| scriptls::rename(&mut dr.doc, at, &to));
            match r {
                Some(Err(m)) => {
                    if let Some(w) = n.devwin.get(k) {
                        w.ide.ren_msg = Some(m);
                    }
                }
                _ => {
                    if let Some(w) = n.devwin.get(k) {
                        w.ide.ren = None;
                    }
                    ui.popup_close();
                    focus_editor(n, ui, k, None);
                    check_soon(n, ui);
                }
            }
            ui.repass();
        }
        2 => {
            w.ide.ren = None;
            ui.popup_close();
        }
        _ => {}
    }
}

/// the side bar's Gliederung: the fns and top-level lets of the shown tab, a click puts the
/// caret there
pub(crate) fn outline(n: &mut Net, ui: &mut Ui, k: usize, id: u32, cx: &Cx, acts: &mut Vec<Act>) {
    let entries = cx.cur.as_ref().and_then(|key| n.devwin.ide.drafts.get(&(cx.dev, key.clone()))).filter(|dr| dr.jfs).map(|dr| (dr.outline.clone(), dr.doc.selection().caret.line + 1));
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 4);
    match entries {
        Some((list, cl)) if !list.is_empty() => {
            let mut sel = list.iter().rposition(|e| e.line <= cl).map_or(-1, |i| i as i32);
            ui.next_size(-1, -1);
            let (first, end) = ui.list_begin(id + 0x240, &mut sel, list.len().min(190) as i32, 24);
            for i in first..end {
                let e = &list[i as usize];
                ui.list_item(i, &format!("{}{}", "\u{2003}".repeat(e.level), e.text));
            }
            let (lev, s) = ui.list_end(&mut sel);
            if lev & EV_ACT != 0
                && let (Some(e), Some(key)) = (list.get(s.max(0) as usize), cx.cur.clone())
            {
                acts.push(Act::OpenAt(key, e.line, e.col));
            }
        }
        _ => {
            ui.next_size(-1, 0);
            ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, t("Noch keine Funktionen und globalen Variablen."));
            ui.spacer(-1);
        }
    }
    ui.box_end();
    let _ = k;
}
