// lower -- the IDE's lower area: Ausgabe (what the service the IDE runs printed in its current
// run, following live), Terminal (the device's console), Probleme (the check's messages of the
// open jfs files and the runtime errors of the service; a click goes there), Debugger
// (debug.rs) and Tests (the device's last `jfs --test`, jfn_sim cli/jfstest.rs; a click on a
// test goes to its place); Bereitstellen and Lasttest (A2: deploy.rs, load.rs)
//
//   names() -> [&str; 7]         the panes' titles (the Ansicht menu)
//   area(n, ui, k, id, cx, acts) the tabs and the shown pane
//   problems(n, cx) -> Vec<Problem>   what Probleme lists, errors first

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_sim::ScriptService;
use ui::*;

use super::editor::{err_text, when};
use super::{Act, Cx, Key, LOW_DEBUG, LOW_DEPLOY, LOW_LOAD, LOW_OUTPUT, LOW_PROBLEMS, LOW_TERMINAL, LOW_TESTS};
use crate::app::tr::{t, tf};
use crate::app::Net;

/// the panes' titles
pub(crate) fn names() -> [&'static str; 7] {
    [t("Ausgabe"), t("Terminal"), t("Probleme"), t("Debugger"), t("Tests"), t("Bereitstellen"), t("Lasttest")]
}

/// a row of Probleme
pub(crate) struct Problem {
    pub key: Option<Key>,
    pub line: usize,
    pub col: usize,
    pub error: bool,
    pub text: String,
}

/// what Probleme lists: the check's messages of every open jfs draft of the device, then the
/// runtime errors of the service the IDE runs (errors first, then by file and place)
pub(crate) fn problems(n: &Net, cx: &Cx) -> Vec<Problem> {
    let mut v = Vec::new();
    for ((dv, key), dr) in n.devwin.ide.drafts.iter() {
        if *dv != cx.dev || !dr.jfs {
            continue;
        }
        for d in &dr.check {
            v.push(Problem { key: Some(key.clone()), line: d.line, col: d.col, error: d.severity == DIAG_ERROR, text: d.msg.clone() });
        }
    }
    if let Some(t_) = &cx.target
        && let Some(s) = n.doc.sim.engine().service::<ScriptService>(cx.dev, t_)
    {
        let own = match s.file() {
            Some(f) => Key::File(cx.st.norm(f).unwrap_or_else(|_| String::from(f))),
            None => Key::Inline(t_.clone()),
        };
        for e in s.run_errors() {
            let key = if e.file.is_empty() { own.clone() } else { Key::File(e.file.clone()) };
            let text = if e.line > 0 { err_text(&e) } else { tf("Dienst {} ({}, bei {}): {}", &[t_, e.during, &when(e.at), &e.msg]) };
            v.push(Problem { key: (e.line > 0).then_some(key), line: e.line as usize, col: e.col as usize, error: true, text });
        }
    }
    v.sort_by(|a, b| (!a.error, &a.key, a.line, a.col).cmp(&(!b.error, &b.key, b.line, b.col)));
    v
}

/// the lower area: its tabs and the shown pane
pub(crate) fn area(n: &mut Net, ui: &mut Ui, k: usize, id: u32, cx: &Cx, acts: &mut Vec<Act>) {
    let Some(mut sel) = n.devwin.get(k).map(|w| w.ide.lower) else { return };
    let probs = problems(n, cx);
    let errs = probs.iter().filter(|p| p.error).count();
    let ptitle = if probs.is_empty() { String::from(t("Probleme")) } else { tf("Probleme ({})", &[&probs.len().to_string()]) };
    let tests = n.doc.sim.engine().jfs_tests(cx.dev).cloned();
    let ttitle = match &tests {
        Some(r) if r.counts().1 > 0 => tf("Tests ({} fehlgeschlagen)", &[&r.counts().1.to_string()]),
        _ => String::from(t("Tests")),
    };
    let dtitle = if cx.paused.is_some() { String::from(t("Debugger (angehalten)")) } else { String::from(t("Debugger")) };
    let nm = names();
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 4);
    if ui.tabs(id + 0x108, &mut sel, &[nm[0], nm[1], &ptitle, &dtitle, &ttitle, nm[5], nm[6]]) & EV_ACT != 0 {
        acts.push(Act::Lower(sel));
    }
    let _ = errs;
    match sel {
        LOW_TERMINAL => super::terminal(n, ui, k, cx.dev),
        LOW_PROBLEMS => problems_pane(n, ui, k, id, &probs, acts),
        LOW_DEBUG => super::debug::view(n, ui, k, id, cx, acts),
        LOW_TESTS => tests_pane(n, ui, k, id, cx, tests.as_ref(), acts),
        LOW_DEPLOY => super::deploy::view(n, ui, k, cx, acts),
        LOW_LOAD => super::load::view(n, ui, k, cx, acts),
        LOW_OUTPUT | _ => output(n, ui, k, id, cx),
    }
    ui.box_end();
}

// Ausgabe: the current run's output of the service the IDE runs, following live
fn output(n: &mut Net, ui: &mut Ui, k: usize, id: u32, cx: &Cx) {
    let e = n.doc.sim.engine();
    let svc = cx.target.as_ref().and_then(|t_| e.service::<ScriptService>(cx.dev, t_).map(|s| (t_.clone(), s.view(e.now()).runs, s)));
    let Some(w) = n.devwin.get(k) else { return };
    let p = &mut w.ide;
    match svc {
        Some((name, runs, s)) => {
            if p.out_of.as_ref() != Some(&(name.clone(), runs)) {
                p.out.clear();
                p.out_of = Some((name, runs));
                p.out_at = 0;
                p.out_any = false;
            }
            let (text, end) = s.run_output(p.out_at);
            if !text.is_empty() {
                p.out.write_str(text);
                p.out_any = true;
            }
            p.out_at = end;
        }
        None => {
            p.out_of = None;
            p.out_any = false;
        }
    }
    ui.next_size(-1, -1);
    if !p.out_any {
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER, 0);
        let note = match &cx.target {
            Some(t_) => tf("Noch keine Ausgabe von {} in diesem Lauf: print(…) schreibt hierher.", &[t_]),
            None => String::from(t("Hier steht, was ein Dienst ausgibt, den die IDE ausführt (Als Dienst ausführen, F5). Ein Prozess schreibt ins Terminal.")),
        };
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP | TX_CENTER, &note);
        ui.box_end();
    } else {
        let _ = ui.terminal(id + 0x220, &mut p.out, TF_NO_INPUT);
    }
}

// Probleme: a row per message, a click goes there
fn problems_pane(n: &mut Net, ui: &mut Ui, k: usize, id: u32, probs: &[Problem], acts: &mut Vec<Act>) {
    ui.next_size(-1, -1);
    if probs.is_empty() {
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER, 0);
        ui.text(TS_CAPTION | TX_TERTIARY, t("Keine Probleme in den offenen Dateien."));
        ui.box_end();
        return;
    }
    let Some(mut sel) = n.devwin.get(k).map(|w| w.ide.probs) else { return };
    sel = sel.min(probs.len() as i32 - 1);
    let count = probs.len().min(250) as i32;
    let (first, end) = ui.list_begin(id + 0x300, &mut sel, count, 24);
    for i in first..end {
        let p = &probs[i as usize];
        let place = match &p.key {
            Some(key) => format!("{}:{}:{}: ", key.title(), p.line, p.col),
            None => String::new(),
        };
        ui.list_item_begin(i);
        ui.icon(if p.error { IC_ERROR } else { IC_WARNING }, 14);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, &format!("{}{}", place, p.text));
        ui.list_item_end();
    }
    let (ev, s) = ui.list_end(&mut sel);
    if let Some(w) = n.devwin.get(k) {
        w.ide.probs = s;
    }
    if ev & (EV_ACT | EV_DOUBLE) != 0
        && let Some(p) = probs.get(s.max(0) as usize)
        && let Some(key) = &p.key
    {
        acts.push(Act::OpenAt(key.clone(), p.line, p.col));
    }
}

// Tests: the device's last run of `jfs --test`, a click goes to the test or to its failure
fn tests_pane(n: &mut Net, ui: &mut Ui, k: usize, id: u32, cx: &Cx, run: Option<&jfn_sim::cli::jfstest::TestRun>, acts: &mut Vec<Act>) {
    let file = matches!(&cx.cur, Some(Key::File(p)) if p.ends_with(".jfs"));
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 4);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_icon(IC_ACCEPT);
    ui.next_flags(ST_COMPACT | if file && !cx.d.off { 0 } else { ST_DISABLED });
    if ui.button(id + 0x140, t("Tests ausführen")) & EV_ACT != 0 {
        acts.push(Act::Tests);
    }
    ui.tooltip(t("Die Funktionen test_… der Datei laufen im Terminal mit jfs --test, jede in einem frischen Programm (Strg+Umschalt+T)"));
    let summary = match run {
        None => String::from(t("Noch kein Testlauf auf diesem Gerät.")),
        Some(r) => {
            let (p, f) = r.counts();
            let all = (p + f).to_string();
            if r.done {
                tf("{} Tests: {} bestanden, {} fehlgeschlagen", &[&all, &p.to_string(), &f.to_string()])
            } else {
                tf("läuft … {} von {} Tests: {} bestanden, {} fehlgeschlagen", &[&all, &r.tests.len().to_string(), &p.to_string(), &f.to_string()])
            }
        }
    };
    ui.next_size(-1, 0);
    let bad = run.is_some_and(|r| r.counts().1 > 0);
    ui.text(TS_BODY | if bad { 0 } else { TX_SECONDARY } | TX_ELLIPSIS, &summary);
    ui.box_end();
    if let Some(r) = run.filter(|r| !r.results.is_empty()) {
        let Some(mut sel) = n.devwin.get(k).map(|w| w.ide.tests) else {
            ui.box_end();
            return;
        };
        sel = sel.min(r.results.len() as i32 - 1);
        ui.next_size(-1, -1);
        let count = r.results.len().min(250) as i32;
        let (first, end) = ui.list_begin(id + 0x700, &mut sel, count, 24);
        for i in first..end {
            let x = &r.results[i as usize];
            let name = x.file.rsplit('/').next().unwrap_or(&x.file);
            let text = match (&x.at, x.ok) {
                (_, true) => tf("bestanden: {} ({})", &[&x.name, name]),
                // (a file that does not load or compile: no test of it ran)
                (Some((f, l, c)), false) if *l > 0 && x.name.is_empty() => tf("lässt sich nicht übersetzen: {}:{}:{}: {}", &[f.rsplit('/').next().unwrap_or(f), &l.to_string(), &c.to_string(), &x.msg]),
                _ if x.name.is_empty() => tf("lässt sich nicht laden: {}: {}", &[name, &x.msg]),
                (Some((f, l, c)), false) if *l > 0 => tf("fehlgeschlagen: {} ({}:{}:{}): {}", &[&x.name, f.rsplit('/').next().unwrap_or(f), &l.to_string(), &c.to_string(), &x.msg]),
                _ => tf("fehlgeschlagen: {} ({}): {}", &[&x.name, name, &x.msg]),
            };
            ui.list_item_begin(i);
            ui.icon(if x.ok { IC_CHECK_MARK } else { IC_CANCEL_CIRCLE }, 14);
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_ELLIPSIS, &text);
            ui.list_item_end();
        }
        let (ev, s) = ui.list_end(&mut sel);
        if let Some(w) = n.devwin.get(k) {
            w.ide.tests = s;
        }
        if ev & (EV_ACT | EV_DOUBLE) != 0
            && let Some(x) = r.results.get(s.max(0) as usize)
        {
            match &x.at {
                Some((f, l, c)) if *l > 0 && !x.ok => acts.push(Act::OpenAt(Key::File(f.clone()), *l as usize, *c as usize)),
                _ => acts.push(Act::OpenAt(Key::File(x.file.clone()), (x.line as usize).max(1), 1)),
            }
        }
    }
    ui.box_end();
}
