// search -- find and replace in files (the side bar's Suchen): a text over the device's .jfs
// files -- what the drafts show where a file has unsaved changes --, with case and whole words,
// the matches as a list (a click goes there), replace all as one undo step of the project
//
//   Search                         the query, the replacement, the options, the matches of the
//                                  last look
//   start(n, ui, k, id, q, replace)   the side bar shows Suchen with the query (the selection or
//                                  the word at the caret), its box gets the keyboard
//   view(n, ui, k, id, cx, acts)   the fields, the summary, the matches
//   replace_all(n, k, cx)          every match in every file as one step (doc/ide.rs save_many);
//                                  refused while a file with matches has unsaved changes
//   find(text, q, case, word) -> Vec<(line, col, end)>   the matches of a text (lines and
//                                  columns from 1, characters; the end exclusive)
//   replace(text, q, rep, case, word) -> (text, count)

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use ui::*;

use super::{Act, Cx, Key, SIDE_SEARCH};
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::ide as dide;

/// one match: the file, line and columns (from 1), the line's text
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Hit {
    pub path: String,
    pub line: usize,
    pub col: usize,
    pub text: String,
}

/// the side bar's search in one window
pub(crate) struct Search {
    pub(crate) query: String,
    pub(crate) rep: String,
    pub(crate) case: i32,
    pub(crate) word: i32,
    pub(crate) replace: bool,
    pub(crate) sel: i32,
    hits: Vec<Hit>,
    /// what the hits belong to: query, options, the project's revision, the drafts' revisions
    of: Option<(String, i32, i32, u32, u64)>,
}

impl Search {
    pub(crate) fn new() -> Search {
        Search { query: String::new(), rep: String::new(), case: 0, word: 0, replace: false, sel: -1, hits: Vec::new(), of: None }
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// the matches of `q` in `text` (see the module comment)
pub(crate) fn find(text: &str, q: &str, case: bool, word: bool) -> Vec<(usize, usize, usize)> {
    let low = |c: char| if case { c } else { c.to_lowercase().next().unwrap_or(c) };
    let qc: Vec<char> = q.chars().map(low).collect();
    let mut out = Vec::new();
    if qc.is_empty() {
        return out;
    }
    for (li, line) in text.split('\n').enumerate() {
        let lc: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i + qc.len() <= lc.len() {
            let hit = lc[i..i + qc.len()].iter().zip(&qc).all(|(a, b)| low(*a) == *b);
            let edges = !word || ((i == 0 || !is_word(lc[i - 1])) && (i + qc.len() == lc.len() || !is_word(lc[i + qc.len()])));
            if hit && edges {
                out.push((li + 1, i + 1, i + 1 + qc.len()));
                i += qc.len();
            } else {
                i += 1;
            }
        }
    }
    out
}

/// `text` with every match of `q` replaced by `rep` -> (text, how many)
pub(crate) fn replace(text: &str, q: &str, rep: &str, case: bool, word: bool) -> (String, usize) {
    let hits = find(text, q, case, word);
    if hits.is_empty() {
        return (String::from(text), 0);
    }
    let mut out = String::with_capacity(text.len());
    let mut h = hits.iter().peekable();
    for (li, line) in text.split('\n').enumerate() {
        if li > 0 {
            out.push('\n');
        }
        let lc: Vec<char> = line.chars().collect();
        let mut i = 0;
        while let Some(&&(l, a, b)) = h.peek() {
            if l != li + 1 {
                break;
            }
            out.extend(&lc[i..a - 1]);
            out.push_str(rep);
            i = b - 1;
            h.next();
        }
        out.extend(&lc[i..]);
    }
    (out, hits.len())
}

/// the .jfs files of the device in path order with the text the IDE shows of each
fn texts(n: &Net, cx: &Cx) -> Vec<(String, String, bool)> {
    let mut v = Vec::new();
    let mut todo = alloc::vec![String::from("/")];
    let mut files = Vec::new();
    while let Some(d) = todo.pop() {
        for (name, s) in cx.st.list(&d).unwrap_or_default() {
            let p = if d == "/" { format!("/{}", name) } else { format!("{}/{}", d, name) };
            if s.dir {
                todo.push(p);
            } else if p.ends_with(".jfs") {
                files.push(p);
            }
        }
    }
    files.sort();
    for p in files {
        match n.devwin.ide.drafts.get(&(cx.dev, Key::File(p.clone()))) {
            Some(dr) if dr.changed() => v.push((p, dr.doc.text(), true)),
            _ => {
                if let Some(t_) = dide::text_of(&cx.st, &p) {
                    v.push((p, t_, false));
                }
            }
        }
    }
    v
}

/// the side bar shows Suchen with `q`
pub(crate) fn start(n: &mut Net, ui: &mut Ui, k: usize, id: u32, q: &str, replace: bool) {
    if let Some(w) = n.devwin.get(k) {
        w.ide.side_on = true;
        w.ide.side = SIDE_SEARCH;
        if !q.is_empty() {
            w.ide.search.query = String::from(q);
        }
        w.ide.search.replace |= replace;
    }
    ui.focus(id + if replace && !q.is_empty() { 0x121 } else { 0x120 });
}

/// Alle ersetzen (see the module comment)
pub(crate) fn replace_all(n: &mut Net, k: usize, cx: &Cx) -> Result<(), String> {
    let Some(s) = n.devwin.get(k).map(|w| (w.ide.search.query.clone(), w.ide.search.rep.clone(), w.ide.search.case != 0, w.ide.search.word != 0)) else { return Ok(()) };
    let (q, rep, case, word) = s;
    if q.is_empty() {
        return Ok(());
    }
    let mut out = Vec::new();
    let mut total = 0;
    for (p, text, changed) in texts(n, cx) {
        let (new, c) = replace(&text, &q, &rep, case, word);
        if c == 0 {
            continue;
        }
        if changed {
            return Err(tf("„{}“ hat ungespeicherte Änderungen: erst speichern, dann in den Dateien ersetzen.", &[&p]));
        }
        total += c;
        out.push((p, new));
    }
    if out.is_empty() {
        return Err(tf("„{}“ kommt in keiner Datei vor.", &[&q]));
    }
    let label = format!("„{}“ in {} Dateien auf {} durch „{}“ ersetzen", q, out.len(), cx.d.name, rep);
    dide::save_many(&mut n.doc, cx.dev, &out, &label)?;
    if let Some(w) = n.devwin.get(k) {
        w.ide.msg = Some(tf("{} Stellen in {} Dateien ersetzt (ein Rückgängig-Schritt).", &[&total.to_string(), &out.len().to_string()]));
        w.ide.search.of = None;
    }
    Ok(())
}

/// the search in the side bar (see the module comment)
pub(crate) fn view(n: &mut Net, ui: &mut Ui, k: usize, id: u32, cx: &Cx, acts: &mut Vec<Act>) {
    // the matches anew when the query, the options, the project or a draft changed
    let revs: u64 = n.devwin.ide.drafts.iter().filter(|((dv, _), _)| *dv == cx.dev).map(|(_, d)| d.doc.revision()).fold(0u64, |a, b| a.wrapping_mul(31).wrapping_add(b));
    let rev = n.doc.rev;
    let stale = n.devwin.get(k).is_some_and(|w| w.ide.search.of != Some((w.ide.search.query.clone(), w.ide.search.case, w.ide.search.word, rev, revs)));
    if stale {
        let (q, case, word) = n.devwin.get(k).map(|w| (w.ide.search.query.clone(), w.ide.search.case != 0, w.ide.search.word != 0)).unwrap_or_default();
        let mut hits = Vec::new();
        if !q.is_empty() {
            for (p, text, _) in texts(n, cx) {
                let lines: Vec<&str> = text.split('\n').collect();
                for (l, c, _) in find(&text, &q, case, word) {
                    if hits.len() < 1000 {
                        hits.push(Hit { path: p.clone(), line: l, col: c, text: String::from(lines[l - 1].trim()) });
                    }
                }
            }
        }
        if let Some(w) = n.devwin.get(k) {
            let s = &mut w.ide.search;
            s.hits = hits;
            s.of = Some((s.query.clone(), s.case, s.word, rev, revs));
            s.sel = s.sel.min(s.hits.len() as i32 - 1);
        }
    }
    let Some(w) = n.devwin.get(k) else { return };
    let s = &mut w.ide.search;
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 4);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 4);
    ui.next_size(-1, 0);
    let qev = ui.textbox(id + 0x120, Some(t("In Dateien suchen")), &mut s.query, 200);
    ui.next_icon(IC_SWITCH);
    ui.next_flags(ST_COMPACT | ST_SUBTLE | ST_ICON_ONLY | if s.replace { ST_CHECKED } else { 0 });
    if ui.button(id + 0x126, t("Ersetzen ein / aus")) & EV_ACT != 0 {
        s.replace = !s.replace;
        ui.repass();
    }
    ui.tooltip(t("Ersetzen ein / aus (Strg+Umschalt+H)"));
    ui.box_end();
    if s.replace {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 4);
        ui.next_size(-1, 0);
        let _ = ui.textbox(id + 0x121, Some(t("Ersetzen durch")), &mut s.rep, 200);
        ui.next_flags(ST_COMPACT | if s.hits.is_empty() { ST_DISABLED } else { 0 });
        if ui.button(id + 0x125, t("Alle ersetzen")) & EV_ACT != 0 {
            acts.push(Act::ReplaceAll);
        }
        ui.tooltip(t("Jede Stelle in jeder Datei – ein Rückgängig-Schritt des Projekts"));
        ui.box_end();
    }
    ui.box_begin(0, BOX_V, 2);
    if ui.checkbox(id + 0x122, t("Groß-/Kleinschreibung"), &mut s.case) & EV_ACT != 0 {
        ui.repass();
    }
    if ui.checkbox(id + 0x123, t("Ganzes Wort"), &mut s.word) & EV_ACT != 0 {
        ui.repass();
    }
    ui.box_end();
    if qev & EV_ACT != 0 {
        ui.repass();
    }
    let files = {
        let mut f: Vec<&str> = s.hits.iter().map(|h| h.path.as_str()).collect();
        f.dedup();
        f.len()
    };
    let summary = match (s.query.is_empty(), s.hits.len()) {
        (true, _) => String::from(t("Sucht in allen .jfs-Dateien des Geräts.")),
        (false, 0) => String::from(t("Keine Treffer.")),
        (false, h) => tf("{} Treffer in {} Dateien", &[&h.to_string(), &files.to_string()]),
    };
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &summary);
    if !s.hits.is_empty() {
        let mut sel = s.sel;
        ui.next_size(-1, -1);
        let (first, end) = ui.list_begin(id + 0x400, &mut sel, s.hits.len().min(250) as i32, 24);
        for i in first..end {
            let h = &s.hits[i as usize];
            let name = h.path.rsplit('/').next().unwrap_or(&h.path);
            ui.list_item(i, &format!("{}:{}: {}", name, h.line, h.text));
        }
        let (ev, si) = ui.list_end(&mut sel);
        s.sel = si;
        if ev & (EV_ACT | EV_DOUBLE) != 0
            && let Some(h) = s.hits.get(si.max(0) as usize)
        {
            acts.push(Act::OpenAt(Key::File(h.path.clone()), h.line, h.col));
        }
    } else {
        ui.spacer(-1);
    }
    ui.box_end();
}
