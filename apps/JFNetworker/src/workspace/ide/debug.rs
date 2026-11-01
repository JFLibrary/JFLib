// debug -- running from the IDE and its debugger: Als Prozess ausführen (`jfs FILE` typed into
// the device's console), Als Dienst ausführen (the unit that runs the file starts anew, else the
// device's script service bound to the file, one is made when there is none; an inline script's
// own service), Stoppen, Neu starten (a unit's service through its unit), Tests
// (`jfs --test FILE` in the console), and SK7's debugger on the service the IDE runs
//
//   Op                           Continue, Pause, Step(Into | Over | Out)
//   DebugUi                      the debugger's state in a window: the chosen call, the
//                                variables' tree, the watch expressions, the condition flyout,
//                                the stop it went to last
//   run_process / run_service / stop / restart / tests (n, k, cx)   running saves every
//                                changed draft of the device first (as Alle speichern)
//   op(n, cx, op)                Action::ScriptDebug on the target (the engine's virtual time)
//   toggle(n, k, cx, line, cond) a breakpoint on / off on a line of the shown tab (usize::MAX:
//                                the caret's; a condition sets one)
//   cond_open / cond_flyout      the flyout of a conditional breakpoint (Strg+Umschalt+F9)
//   sync(n, ui, k, dev)          every frame: the breakpoints of the device's tabs go to the
//                                target as the IDE has them (ClearBreaks, then Break for each,
//                                only when they differ), a new stop opens its file at its line
//                                and shows the Debugger, the execution line in the drafts
//   view(n, ui, k, id, cx, acts) the Debugger pane: where it stopped, the call stack (a click
//                                chooses the call and goes to its line), the variables of the
//                                chosen call (Lokal, Erfasst, Global; lists and maps open), the
//                                watch expressions (read-only, evaluated in the chosen call)
//   state_word(view, paused, off) -> (word, badge)
//
// Breakpoints belong to the IDE per device and tab (Ides::bps) and to the target in the engine
// (ScriptService::breaks): a file's breakpoints hold for every service the IDE runs that loads
// it, an inline script's for its own service.  Pause, steps and continue are inputs of the
// engine (K1's recording replays them); a stop keeps the callback on the virtual CPU while time
// goes on (scriptsvc.rs).

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_sim::{Action, Cmd, DebugOp, ScriptService, ScriptState, ScriptView, UnitOp};
use script::{PauseReason, Step, Value};
use ui::*;

use super::{editor, Act, Cx, Key, LOW_DEBUG, LOW_OUTPUT, LOW_TERMINAL, LOW_TESTS};
use crate::app::tr::{t, tf};
use crate::app::Net;
use crate::doc::{console, ide as dide, scripts};

/// what the debugger's buttons and keys do
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    Continue,
    Pause,
    Step(Step),
}

/// the debugger's state in a device window
pub(crate) struct DebugUi {
    /// the chosen call (0: the innermost)
    pub(crate) frame: usize,
    pub(crate) vars: TreeState,
    vars_init: bool,
    pub(crate) watches: Vec<String>,
    pub(crate) watch_text: String,
    pub(crate) watch_sel: i32,
    pub(crate) stack_sel: i32,
    /// the condition flyout: the line (from 1), its text, a refusal
    pub(crate) cond: Option<(usize, String, Option<String>)>,
    /// the stop the IDE went to last: service, run, fuel, file, line
    seen: Option<(String, u64, u64, String, u32)>,
}

impl DebugUi {
    pub(crate) fn new() -> DebugUi {
        DebugUi { frame: 0, vars: TreeState::default(), vars_init: false, watches: Vec::new(), watch_text: String::new(), watch_sel: -1, stack_sel: 0, cond: None, seen: None }
    }
}

/// the state of the target as a word and a badge
pub(crate) fn state_word(v: &ScriptView, paused: bool, off: bool) -> (&'static str, u32) {
    match v.state {
        _ if off => (t("Gerät aus"), BADGE_NEUTRAL),
        ScriptState::Running if paused => (t("angehalten"), BADGE_WARN),
        ScriptState::Running if v.busy => (t("rechnet"), BADGE_OK),
        ScriptState::Running => (t("läuft"), BADGE_OK),
        ScriptState::Stopped => (t("gestoppt"), BADGE_NEUTRAL),
        ScriptState::Failed => (t("mit Fehler beendet"), BADGE_CRIT),
    }
}

/// a command into the simulator; the reason when it refuses
fn send(n: &mut Net, a: Action) -> Result<(), String> {
    n.doc.sim.send(Cmd::Apply(a)).map(|_| ()).map_err(|e| crate::app::tr::msg(&e.msg()))
}

/// the service `name` belongs to a unit file: it goes through the unit (its state, the journal,
/// what needs it)
fn is_unit(n: &Net, dev: u32, name: &str) -> bool {
    n.doc.sim.engine().unit(dev, name).is_some_and(|u| u.file.is_some())
}

/// a path as one word of the shell
fn word(p: &str) -> String {
    if p.chars().all(|c| c.is_alphanumeric() || "/._-+".contains(c)) { String::from(p) } else { format!("'{}'", p.replace('\'', "'\\''")) }
}

/// the IDE shows pane `low` (and the terminal gets the keyboard)
fn show(n: &mut Net, k: usize, dev: u32, low: i32) {
    if let Some(w) = n.devwin.get(k) {
        w.ide.lower = low;
        w.ide.lower_on = true;
    }
    if low == LOW_TERMINAL
        && let Some(s) = super::term_session(n, k, dev)
    {
        n.cons.focus_ses = Some((dev, s));
    }
}

/// Als Prozess ausführen: `jfs FILE` in the IDE's terminal (its own session)
pub(crate) fn run_process(n: &mut Net, k: usize, cx: &Cx) -> Result<(), String> {
    let Some(Key::File(path)) = cx.cur.clone() else {
        return Err(String::from(t("Als Prozess läuft eine Datei: ein eingebettetes Skript erst in eine Datei umwandeln.")));
    };
    editor::save(n, k, cx, true)?;
    show(n, k, cx.dev, LOW_TERMINAL);
    let s = super::term_session(n, k, cx.dev).ok_or_else(|| String::from(t("Das Gerät ist ausgeschaltet.")))?;
    console::typed_session(n, cx.dev, s, &format!("jfs {}", word(&path)));
    Ok(())
}

/// Tests ausführen: `jfs --test FILE` in the IDE's terminal, the Tests pane shows the run
pub(crate) fn tests(n: &mut Net, k: usize, cx: &Cx) -> Result<(), String> {
    let Some(Key::File(path)) = cx.cur.clone() else {
        return Err(String::from(t("Tests laufen aus einer Datei: ein eingebettetes Skript erst in eine Datei umwandeln.")));
    };
    editor::save(n, k, cx, true)?;
    let s = super::term_session(n, k, cx.dev).ok_or_else(|| String::from(t("Das Gerät ist ausgeschaltet.")))?;
    console::typed_session(n, cx.dev, s, &format!("jfs --test {}", word(&path)));
    show(n, k, cx.dev, LOW_TESTS);
    Ok(())
}

/// Als Dienst ausführen (see the module comment); a running one starts again with what was saved
pub(crate) fn run_service(n: &mut Net, k: usize, cx: &Cx) -> Result<(), String> {
    let before = n.doc.rev;
    let name = match &cx.cur {
        Some(Key::Inline(name)) => name.clone(),
        Some(Key::File(path)) if path.ends_with(".jfs") => {
            // a unit holds the file: it starts anew with what was saved
            if let Some(name) = dide::unit_of(n.doc.sim.engine(), &cx.d, &cx.st, path, cx.target.as_deref()) {
                editor::save(n, k, cx, true)?;
                send(n, Action::Unit { dev: cx.dev, name: name.clone(), op: UnitOp::Restart })?;
                finish_run(n, k, cx.dev, &name);
                return Ok(());
            }
            let b = dide::bound(&cx.d, &cx.st, path);
            match b.iter().find(|x| cx.target.as_ref() == Some(*x)).or(b.first()) {
                Some(x) => x.clone(),
                None => {
                    editor::save(n, k, cx, true)?;
                    let name = dide::bind(&mut n.doc, cx.dev, path)?;
                    finish_run(n, k, cx.dev, &name);
                    return Ok(());
                }
            }
        }
        _ => return Err(String::from(t("Als Dienst läuft eine .jfs-Datei oder ein Skript des Geräts."))),
    };
    let inline_saved = matches!(&cx.cur, Some(Key::Inline(x)) if n.devwin.ide.drafts.get(&(cx.dev, Key::Inline(x.clone()))).is_some_and(|d| d.changed()));
    editor::save(n, k, cx, true)?;
    // (a saved inline source restarted its script already)
    if !(inline_saved && n.doc.rev != before) {
        let running = n.doc.sim.engine().service::<ScriptService>(cx.dev, &name).is_some_and(|s| s.state() == ScriptState::Running);
        if running { scripts::restart(&mut n.doc, cx.dev, &name)? } else { scripts::run(&mut n.doc, cx.dev, &name, true)? }
    }
    finish_run(n, k, cx.dev, &name);
    Ok(())
}

// the service the IDE ran is its target from now on; Ausgabe shows it
fn finish_run(n: &mut Net, k: usize, dev: u32, name: &str) {
    if let Some(w) = n.devwin.get(k) {
        w.ide.target = Some(String::from(name));
        w.ide.msg = None;
        w.ide.dbg.frame = 0;
    }
    show(n, k, dev, LOW_OUTPUT);
}

/// Stoppen: the target stops (a process in the IDE's terminal gets Ctrl+C)
pub(crate) fn stop(n: &mut Net, cx: &Cx) -> Result<(), String> {
    match (&cx.target, cx.term) {
        (Some(name), _) if cx.running() && is_unit(n, cx.dev, name) => send(n, Action::Unit { dev: cx.dev, name: name.clone(), op: UnitOp::Stop }),
        (Some(name), _) if cx.running() => scripts::run(&mut n.doc, cx.dev, name, false),
        (_, Some(ses)) if cx.busy => n.doc.sim.send(Cmd::SesInterrupt { dev: cx.dev, ses }).map(|_| ()).map_err(|e| crate::app::tr::msg(&e.msg())),
        _ => Ok(()),
    }
}

/// Neu starten: the target starts again (from the saved file)
pub(crate) fn restart(n: &mut Net, k: usize, cx: &Cx) -> Result<(), String> {
    let Some(name) = cx.target.clone() else { return Ok(()) };
    if is_unit(n, cx.dev, &name) {
        send(n, Action::Unit { dev: cx.dev, name: name.clone(), op: UnitOp::Restart })?;
    } else {
        scripts::restart(&mut n.doc, cx.dev, &name)?;
    }
    finish_run(n, k, cx.dev, &name);
    Ok(())
}

/// pause, continue, a step of the target
pub(crate) fn op(n: &mut Net, cx: &Cx, op: Op) -> Result<(), String> {
    let Some(svc) = cx.target.clone() else {
        return Err(String::from(t("Kein Dienst läuft aus der IDE: Als Dienst ausführen (F5) startet die Datei.")));
    };
    let op = match op {
        Op::Continue => DebugOp::Continue,
        Op::Pause => DebugOp::Pause,
        Op::Step(s) => DebugOp::Step(s),
    };
    send(n, Action::ScriptDebug { dev: cx.dev, svc, op })
}

/// a breakpoint on / off (see the module comment)
pub(crate) fn toggle(n: &mut Net, k: usize, cx: &Cx, line: usize, cond: Option<String>) -> Result<(), String> {
    let Some(key) = cx.cur.clone().filter(|x| x.jfs()) else { return Ok(()) };
    let line = if line == usize::MAX {
        match n.devwin.ide.drafts.get(&(cx.dev, key.clone())) {
            Some(dr) => dr.doc.selection().caret.line + 1,
            None => return Ok(()),
        }
    } else {
        line
    };
    let l = line as u32;
    let list = n.devwin.ide.bps.entry((cx.dev, key)).or_default();
    match (list.iter().position(|b| b.0 == l), cond) {
        (Some(i), Some(c)) => list[i].1 = Some(c).filter(|c| !c.trim().is_empty()),
        (None, Some(c)) => list.push((l, Some(c).filter(|c| !c.trim().is_empty()))),
        (Some(i), None) => {
            list.remove(i);
        }
        (None, None) => list.push((l, None)),
    }
    list.sort_by_key(|b| b.0);
    let _ = k;
    Ok(())
}

/// opens the condition flyout for the caret's line of the shown tab
pub(crate) fn cond_open(n: &mut Net, ui: &mut Ui, k: usize, id: u32, cx: &Cx) {
    let Some(key) = cx.cur.clone().filter(|x| x.jfs()) else { return };
    let Some(line) = n.devwin.ide.drafts.get(&(cx.dev, key.clone())).map(|dr| dr.doc.selection().caret.line + 1) else { return };
    let have = n.devwin.ide.bps.get(&(cx.dev, key)).and_then(|v| v.iter().find(|b| b.0 as usize == line)).and_then(|b| b.1.clone()).unwrap_or_default();
    if let Some(w) = n.devwin.get(k) {
        w.ide.dbg.cond = Some((line, have, None));
    }
    ui.popup_open(id + 0xC8);
    ui.focus(id + 0xC9);
}

/// the flyout of a conditional breakpoint: the condition (a jfs expression), OK, Abbrechen
pub(crate) fn cond_flyout(n: &mut Net, ui: &mut Ui, k: usize, id: u32, cx: &Cx, acts: &mut Vec<Act>) {
    let fid = id + 0xC8;
    if !ui.popup_is_open(fid)
        && let Some(w) = n.devwin.get(k)
    {
        w.ide.dbg.cond = None;
        w.ide.fly[2] = 0;
    }
    let Some(w) = n.devwin.get(k) else { return };
    let mut done = 0;
    if ui.flyout_begin(fid, FL_ALIGN_END)
        && let Some((line, text, refusal)) = w.ide.dbg.cond.as_mut()
    {
        ui.box_begin(0, BOX_V | pad(8), 6);
        ui.text(TS_BODY_STRONG, &tf("Haltepunkt in Zeile {}: hält, wenn", &[&line.to_string()]));
        ui.next_size(280, 0);
        let tev = ui.textbox(id + 0xC9, Some(t("Bedingung, z. B. n > 3")), text, 256);
        w.ide.fly[2] += 1;
        if tev & EV_COMMIT != 0 && ui.focused() == id + 0xC9 && w.ide.fly[2] > 1 {
            done = 1;
        }
        ui.next_size(280, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Ein Ausdruck in jfs; leer: ein Haltepunkt ohne Bedingung."));
        if let Some(m) = refusal.as_ref() {
            crate::workspace::error_text(ui, m, 280);
        }
        ui.box_begin(0, BOX_H | BOX_END, 8);
        if ui.accent_button(id + 0xCA, t("OK")) & EV_ACT != 0 {
            done = 1;
        }
        if ui.button(id + 0xCB, t("Abbrechen")) & EV_ACT != 0 {
            done = 2;
        }
        ui.box_end();
        ui.box_end();
    }
    ui.flyout_end();
    match done {
        1 => {
            if let Some((line, text, _)) = w.ide.dbg.cond.clone() {
                let c = text.trim();
                match if c.is_empty() { Ok(()) } else { script::debug::check_expr(c) } {
                    Ok(()) => {
                        acts.push(Act::Bp(line, Some(String::from(c))));
                        w.ide.dbg.cond = None;
                        ui.popup_close();
                        super::focus_editor(n, ui, k, None);
                    }
                    Err(m) => {
                        if let Some((_, _, r)) = w.ide.dbg.cond.as_mut() {
                            *r = Some(tf("Bedingung: {}", &[&m]));
                        }
                    }
                }
            }
            ui.repass();
        }
        2 => {
            w.ide.dbg.cond = None;
            ui.popup_close();
        }
        _ => {}
    }
    let _ = cx;
}

/// the breakpoints the target should have: (file, line, condition) sorted
fn wanted(n: &Net, dev: u32, target: &str) -> Vec<(String, u32, Option<String>)> {
    let mut v = Vec::new();
    for ((dv, key), list) in n.devwin.ide.bps.iter() {
        if *dv != dev {
            continue;
        }
        let file = match key {
            Key::Inline(name) if name == target => String::new(),
            Key::Inline(_) => continue,
            Key::File(p) => p.clone(),
        };
        for (line, cond) in list {
            v.push((file.clone(), *line, cond.clone()));
        }
    }
    v.sort();
    v
}

/// every frame (see the module comment)
pub(crate) fn sync(n: &mut Net, ui: &mut Ui, k: usize, dev: u32) {
    let Some(d) = n.doc.proj.device(dev).cloned() else { return };
    let Some(st) = dide::store_of(&n.doc.proj, dev) else { return };
    let Some(cx) = Cx::of(n, k, &d, &st) else { return };
    let Some(target) = cx.target.clone() else {
        clear_exec(n, dev, None);
        return;
    };
    // the breakpoints, when they differ (only a script the simulator runs takes them)
    let want = wanted(n, dev, &target);
    let have = n.doc.sim.engine().service::<ScriptService>(dev, &target).map(|s| {
        let mut h: Vec<(String, u32, Option<String>)> = s.breaks().to_vec();
        h.sort();
        h
    });
    if let Some(h) = have
        && h != want
        && n.doc.sim.past().is_none()
    {
        let _ = send(n, Action::ScriptDebug { dev, svc: target.clone(), op: DebugOp::ClearBreaks });
        for (file, line, cond) in want {
            let _ = send(n, Action::ScriptDebug { dev, svc: target.clone(), op: DebugOp::Break { file, line, cond } });
        }
    }
    // the stop: its file at its line (once per stop), the execution line
    let Some(p) = cx.paused.clone() else {
        clear_exec(n, dev, None);
        if let Some(w) = n.devwin.get(k) {
            w.ide.dbg.seen = None;
        }
        return;
    };
    let key = if p.file.is_empty() {
        Key::Inline(target.clone())
    } else {
        Key::File(p.file.clone())
    };
    let (runs, fuel) = cx.view.as_ref().map_or((0, 0), |v| (v.runs, v.fuel));
    let stop = (target.clone(), runs, fuel, p.file.clone(), p.line);
    let new = n.devwin.get(k).is_some_and(|w| w.ide.dbg.seen.as_ref() != Some(&stop));
    if new {
        if let Some(w) = n.devwin.get(k) {
            w.ide.dbg.seen = Some(stop);
            w.ide.dbg.frame = 0;
            w.ide.dbg.stack_sel = 0;
            w.ide.lower = LOW_DEBUG;
            w.ide.lower_on = true;
        }
        let _ = editor::open(n, ui, k, &cx, key.clone(), Some((p.line as usize, p.col as usize)));
        ui.repass();
    }
    clear_exec(n, dev, Some((&key, p.line as usize)));
}

// the execution line: on the draft of the stop, off everywhere else of the device
fn clear_exec(n: &mut Net, dev: u32, at: Option<(&Key, usize)>) {
    for ((dv, key), dr) in n.devwin.ide.drafts.iter_mut() {
        if *dv != dev {
            continue;
        }
        let want = match at {
            Some((k_, line)) if k_ == key => Some(line - 1),
            _ => None,
        };
        if dr.doc.exec_line() != want {
            dr.doc.set_exec_line(want);
        }
    }
}

/// a key from a text (FNV-1a)
fn key_of(s: &str) -> u64 {
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for b in s.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0100_0000_01B3);
    }
    h
}

/// a row of the variables' tree
struct VarRow {
    key: u64,
    depth: u16,
    kids: bool,
    label: String,
}

/// the variables of call `frame` as rows (groups Lokal, Erfasst, Global; open values' children)
fn var_rows(vm: &script::Vm, frame: usize, open: &TreeState) -> Vec<VarRow> {
    fn add(vm: &script::Vm, open: &TreeState, path: &str, name: &str, v: Value, depth: u16, out: &mut Vec<VarRow>) {
        if out.len() >= 400 {
            return;
        }
        let key = key_of(path);
        let kids = vm.children(v);
        out.push(VarRow { key, depth, kids: !kids.is_empty(), label: format!("{} = {}", name, vm.preview(v, 80)) });
        if depth < 8 && open.expanded.contains(&key) {
            for (i, (cn, cv)) in kids.into_iter().enumerate().take(200) {
                add(vm, open, &format!("{}/{}", path, i), &cn, cv, depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    let groups: [(&str, &str, Vec<(String, Value)>); 3] = [("L", t("Lokal"), vm.locals(frame)), ("U", t("Erfasst"), vm.upvalues(frame)), ("G", t("Global"), vm.frame_globals(frame))];
    for (g, title, vars) in groups {
        if vars.is_empty() {
            continue;
        }
        let gk = key_of(g);
        out.push(VarRow { key: gk, depth: 0, kids: true, label: format!("{} ({})", title, vars.len()) });
        if open.expanded.contains(&gk) {
            for (name, v) in vars {
                add(vm, open, &format!("{}/{}", g, name), &name, v, 1, &mut out);
            }
        }
    }
    out
}

/// "Hauptprogramm", "Funktion ohne Namen", "greet"
fn func_name(f: &str) -> String {
    match f {
        "" => String::from(t("Hauptprogramm")),
        "fn" => String::from(t("Funktion ohne Namen")),
        f => String::from(f),
    }
}

/// the Debugger pane (see the module comment)
pub(crate) fn view(n: &mut Net, ui: &mut Ui, k: usize, id: u32, cx: &Cx, acts: &mut Vec<Act>) {
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 4);
    let head = match (&cx.target, &cx.paused, &cx.view) {
        (None, _, _) => String::from(t("Kein Dienst aus der IDE: Als Dienst ausführen (F5) startet die Datei; ein Klick links neben eine Zeilennummer (F9) setzt einen Haltepunkt.")),
        (Some(s), Some(p), _) => {
            let why = match p.reason {
                PauseReason::Breakpoint => t("Haltepunkt"),
                PauseReason::Step => t("Schritt"),
                PauseReason::Pause => t("Anhalten"),
            };
            let place = if p.file.is_empty() { tf("Zeile {}, Spalte {}", &[&p.line.to_string(), &p.col.to_string()]) } else { tf("{}, Zeile {}, Spalte {}", &[p.file.rsplit('/').next().unwrap_or(&p.file), &p.line.to_string(), &p.col.to_string()]) };
            let mut h = tf("{} hält in {} ({})", &[s, &place, why]);
            if let Some(e) = &p.error {
                h.push_str(" – ");
                h.push_str(&tf("die Bedingung scheiterte: {}", &[e]));
            }
            h
        }
        (Some(s), None, Some(v)) if v.state == ScriptState::Running => tf("{} läuft: Haltepunkte halten ihn an, Anhalten (Strg+F9) beim nächsten Schritt.", &[s]),
        (Some(s), None, _) => tf("{} läuft nicht: Als Dienst ausführen (F5) startet ihn.", &[s]),
    };
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_WRAP, &head);
    let frames = cx.paused.as_ref().and(cx.target.as_ref()).and_then(|s| n.doc.sim.engine().service::<ScriptService>(cx.dev, s)).and_then(|s| s.debug_vm()).map(|vm| vm.frames()).unwrap_or_default();
    let Some(w) = n.devwin.get(k) else {
        ui.box_end();
        return;
    };
    if w.ide.dbg.frame >= frames.len().max(1) {
        w.ide.dbg.frame = 0;
    }
    let frame = w.ide.dbg.frame;
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_FIXED, 0);
    let total = ui.px2dip(ui.box_rect()[2]);
    let third = (total / 3).max(140);
    ui.next_size(-1, -1);
    ui.split_begin(id + 0x134, third, false);
    // the call stack
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 2);
    ui.text(TS_CAPTION | TX_SECONDARY, t("Aufrufliste"));
    if frames.is_empty() {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, t("Nur während der Dienst angehalten ist."));
        ui.spacer(-1);
    } else {
        let Some(w) = n.devwin.get(k) else { return };
        let mut sel = frame as i32;
        ui.next_size(-1, -1);
        let (first, end) = ui.list_begin(id + 0x500, &mut sel, frames.len().min(120) as i32, 24);
        for i in first..end {
            let f = &frames[i as usize];
            let file = if f.file.is_empty() { cx.target.clone().unwrap_or_default() } else { String::from(f.file.rsplit('/').next().unwrap_or(&f.file)) };
            ui.list_item(i, &format!("{} ({}:{})", func_name(&f.func), file, f.line));
        }
        let (ev, s) = ui.list_end(&mut sel);
        w.ide.dbg.stack_sel = s;
        if ev & (EV_ACT | EV_DOUBLE) != 0
            && let Some(f) = frames.get(s.max(0) as usize)
        {
            w.ide.dbg.frame = s.max(0) as usize;
            let key = if f.file.is_empty() { Key::Inline(cx.target.clone().unwrap_or_default()) } else { Key::File(f.file.clone()) };
            acts.push(Act::OpenAt(key, f.line as usize, f.col as usize));
        }
    }
    ui.box_end();
    ui.split_next();
    ui.next_size(-1, -1);
    ui.split_begin(id + 0x136, third, false);
    vars(n, ui, k, id, cx, frame, !frames.is_empty());
    ui.split_next();
    watches(n, ui, k, id, cx, frame);
    let mut p2 = third;
    let _ = ui.split_end(&mut p2);
    let mut p1 = third;
    let _ = ui.split_end(&mut p1);
    ui.box_end();
    ui.box_end();
}

// the variables of the chosen call
fn vars(n: &mut Net, ui: &mut Ui, k: usize, id: u32, cx: &Cx, frame: usize, stopped: bool) {
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 2);
    ui.text(TS_CAPTION | TX_SECONDARY, t("Variablen"));
    let Some(w) = n.devwin.get(k) else {
        ui.box_end();
        return;
    };
    if !w.ide.dbg.vars_init {
        w.ide.dbg.vars_init = true;
        w.ide.dbg.vars.expanded.insert(key_of("L"));
        w.ide.dbg.vars.expanded.insert(key_of("U"));
    }
    let open = w.ide.dbg.vars.clone();
    let e = n.doc.sim.engine();
    let rows = if stopped { cx.target.as_ref().and_then(|s| e.service::<ScriptService>(cx.dev, s)).and_then(|s| s.debug_vm()).map(|vm| var_rows(vm, frame, &open)).unwrap_or_default() } else { Vec::new() };
    if rows.is_empty() {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, t("Nur während der Dienst angehalten ist."));
        ui.spacer(-1);
    } else {
        let trows: Vec<TreeRow> = rows.iter().map(|r| TreeRow { key: r.key, depth: r.depth, has_children: r.kids, check: None, icon: 0, label: &r.label, detail: None }).collect();
        let Some(w) = n.devwin.get(k) else { return };
        ui.next_size(-1, -1);
        let _ = ui.tree(id + 0x600, &mut w.ide.dbg.vars, &trows, 22);
    }
    ui.box_end();
}

// the watch expressions: evaluated in the chosen call while stopped
fn watches(n: &mut Net, ui: &mut Ui, k: usize, id: u32, cx: &Cx, frame: usize) {
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 2);
    ui.text(TS_CAPTION | TX_SECONDARY, t("Ausdrücke"));
    let e = n.doc.sim.engine();
    let svc = cx.paused.as_ref().and(cx.target.as_ref()).and_then(|s| e.service::<ScriptService>(cx.dev, s));
    let Some(w) = n.devwin.get(k) else {
        ui.box_end();
        return;
    };
    let d = &mut w.ide.dbg;
    let texts: Vec<String> = d
        .watches
        .iter()
        .map(|x| match svc {
            Some(s) => match s.debug_eval(frame, x) {
                Ok(v) => format!("{} = {}", x, v),
                Err(m) => format!("{}: {}", x, m),
            },
            None => format!("{} = –", x),
        })
        .collect();
    let mut add = false;
    let mut remove = false;
    ui.box_begin(0, BOX_H | BOX_XCENTER, 4);
    ui.next_size(-1, 0);
    let tev = ui.textbox(id + 0x130, Some(t("Ausdruck, z. B. req.path")), &mut d.watch_text, 256);
    if tev & EV_COMMIT != 0 && ui.focused() == id + 0x130 {
        add = true;
    }
    ui.next_icon(IC_ADD);
    ui.next_flags(ST_COMPACT | ST_SUBTLE | ST_ICON_ONLY);
    if ui.button(id + 0x131, t("Ausdruck hinzufügen")) & EV_ACT != 0 {
        add = true;
    }
    ui.tooltip(t("Ausdruck hinzufügen (Eingabe)"));
    ui.next_icon(IC_DELETE);
    ui.next_flags(ST_COMPACT | ST_SUBTLE | ST_ICON_ONLY | if d.watch_sel >= 0 && (d.watch_sel as usize) < d.watches.len() { 0 } else { ST_DISABLED });
    if ui.button(id + 0x132, t("Ausdruck entfernen")) & EV_ACT != 0 {
        remove = true;
    }
    ui.tooltip(t("Ausdruck entfernen"));
    ui.box_end();
    if texts.is_empty() {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, t("Ausdrücke werden im gewählten Aufruf gelesen, ohne das Programm zu ändern."));
        ui.spacer(-1);
    } else {
        let mut sel = d.watch_sel.min(texts.len() as i32 - 1);
        ui.next_size(-1, -1);
        let (first, end) = ui.list_begin(id + 0x580, &mut sel, texts.len().min(120) as i32, 22);
        for i in first..end {
            ui.list_item(i, &texts[i as usize]);
        }
        let (_, s) = ui.list_end(&mut sel);
        d.watch_sel = s;
    }
    let x = d.watch_text.trim();
    if add && !x.is_empty() && !d.watches.iter().any(|w_| w_ == x) && d.watches.len() < 50 {
        d.watches.push(String::from(x));
        d.watch_text.clear();
        ui.repass();
    }
    if remove && d.watch_sel >= 0 && (d.watch_sel as usize) < d.watches.len() {
        d.watches.remove(d.watch_sel as usize);
        d.watch_sel = -1;
        ui.repass();
    }
    ui.box_end();
}
