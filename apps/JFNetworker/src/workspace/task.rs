// task -- the Aufgabe panel: the lab of the open project (jfn_model::lab) with its title,
// introduction, goals as a checklist, hints that open one by one and the progress; and the
// list of the app's labs (Datei › Labor öffnen …)
//
//   Task::new()                  no lab
//   loaded(n)                    a new or loaded project: its lab's goals start over, a lab
//                                brings the panel forward (a tab beside Eigenschaften; the
//                                right side at least 340 DIP wide)
//   tick(n)                      every frame: the goals are judged (jfn_sim::goal::LabRun) when
//                                a second of virtual time has passed since the last time, and
//                                after a change of the project (at most every 250 ms); a fresh
//                                engine (Simulation zurücksetzen) starts them over
//   check(n)                     "Prüfen" (F9): judged now, and from then on every open goal
//                                shows what each of its checks found
//   hint(n)                      the next hint: of the first open goal that has one left, else
//                                of the whole task
//   panel(n, ui)                 the panel (P_TASK); without a lab an empty state that leads to
//                                the list
//   dialog(n, ui)                the list of labs: topic, title, summary and the number of
//                                goals of every app://labs/*.jfnet (by file name); Öffnen, a
//                                double click or Enter open one (asking about unsaved changes
//                                first): a project without a file (doc/file.rs open_lab)
//   states(n, ui) / command(n, ui, id, arg)   CMD_LAB_OPEN .. CMD_LAB_CHOSEN
//   labs() -> Vec<LabInfo>       the labs of the app
//   assessments() -> Vec<LabInfo>   the app's assessments (app://assess/*.jfnet, A1), listed after
//                                the labs with their points; an assessment's score card, items,
//                                time limit and handing in are workspace/assess.rs's
//
// A goal is reached when all its checks hold at one moment and stays reached; an ordered lab
// looks at a goal once the goals before it are reached.  "output contains" reads the console
// of the device (doc/console.rs).  The lab's texts come from its file in German and English
// (Text::get), the panel's own through t().  Screen readers: every goal is a check box record
// named by its text (checked: reached; it cannot be toggled), the progress a progress bar, the
// hints and the findings text records.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::lab::{Lab, Text};
use jfn_sim::goal::{check_text, LabRun};
use jfn_sim::SEC;
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{self, t, tc, tf};
use crate::app::Net;
use crate::doc::file::{self, After};

/// the folder of the labs in the app's assets (apps/JFNetworker/assets/labs)
pub(crate) const LABS: &str = "app://labs";
/// a change of the project judges the goals again after this long at the earliest (µs of the
/// task clock)
const EDIT_US: u64 = 250_000;
/// the right side of the dock with a lab at least (DIP)
const TASK_W: i32 = 340;

/// a lab of the app for the list
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LabInfo {
    pub path: String,
    pub title: Text,
    pub summary: Text,
    pub topic: String,
    pub goals: usize,
    /// an assessment: the points it has (0: a lab without one)
    pub points: u32,
}

/// the lab of the open project over the run, the hints shown, the list of labs
pub(crate) struct Task {
    /// the lab of the project (None: a free project)
    pub lab: Option<Lab>,
    pub run: Option<LabRun>,
    /// "Prüfen" was used: the open goals show what their checks found
    pub explain: bool,
    /// hints shown per goal, and of the whole task
    pub goal_hints: Vec<usize>,
    pub hints: usize,
    /// the introduction is open (it starts open)
    pub intro: i32,
    /// the project's revision and the task clock (µs) at the last judgement; judge at the next
    /// tick
    rev: u32,
    at_us: u64,
    force: bool,
    /// the list of labs: open, its rows, the chosen one
    pub dlg: Shared<i32>,
    pub labs: Vec<LabInfo>,
    pub lab_sel: i32,
    /// the assessment of the lab (A1, workspace/assess.rs), its countdown's timer is set
    pub assess: Option<crate::workspace::assess::Run>,
    pub timer_armed: bool,
    /// the project's revision when its lab was last compared with this one's
    pub lab_rev: u32,
}

impl Task {
    pub(crate) fn new() -> Task {
        Task { lab: None, run: None, explain: false, goal_hints: Vec::new(), hints: 0, intro: 1, rev: 0, at_us: 0, force: false, dlg: Shared::new(0), labs: Vec::new(), lab_sel: 0, assess: None, timer_armed: false, lab_rev: 0 }
    }

    // another lab (or none): everything starts over
    fn reset(&mut self, lab: Option<Lab>) {
        self.run = lab.as_ref().map(LabRun::new);
        self.goal_hints = alloc::vec![0; lab.as_ref().map_or(0, |l| l.goals.len())];
        self.lab = lab;
        self.explain = false;
        self.hints = 0;
        self.intro = 1;
        self.force = true;
    }
}

/// the labs of the app (app://labs/*.jfnet with a lab), by file name
pub(crate) fn labs() -> Vec<LabInfo> {
    let mut names: Vec<String> = plat::fs::read_dir(LABS).unwrap_or_default().into_iter().filter(|e| e.kind == plat::fs::FileKind::File && e.name.ends_with(".jfnet")).map(|e| e.name).collect();
    names.sort();
    names
        .into_iter()
        .filter_map(|name| {
            let path = format!("{}/{}", LABS, name);
            let lab = file::read(&path).ok()?.lab?;
            let points = lab.assess.as_ref().map_or(0, |a| a.total());
            Some(LabInfo { path, title: lab.title, summary: lab.summary, topic: lab.topic, goals: lab.goals.len(), points })
        })
        .collect()
}

/// the folder of the assessments in the app's assets (apps/JFNetworker/assets/assess)
pub(crate) const ASSESS: &str = "app://assess";

/// the assessments of the app (app://assess/*.jfnet with an assessment), by file name
pub(crate) fn assessments() -> Vec<LabInfo> {
    let mut names: Vec<String> = plat::fs::read_dir(ASSESS).unwrap_or_default().into_iter().filter(|e| e.kind == plat::fs::FileKind::File && e.name.ends_with(".jfnet")).map(|e| e.name).collect();
    names.sort();
    names
        .into_iter()
        .filter_map(|name| {
            let path = format!("{}/{}", ASSESS, name);
            let lab = file::read(&path).ok()?.lab?;
            let points = lab.assess.as_ref()?.total();
            Some(LabInfo { path, title: lab.title, summary: lab.summary, topic: lab.topic, goals: lab.goals.len(), points })
        })
        .collect()
}

/// a new or loaded project: its lab's goals start over; a lab brings the panel forward (the
/// right side at least TASK_W wide, so its text has room to read)
pub(crate) fn loaded(n: &mut Net) {
    let lab = n.doc.proj.lab.clone();
    let has = lab.is_some();
    n.task.reset(lab);
    crate::workspace::assess::loaded(n);
    if has {
        n.shell.dock.show(P_TASK);
        if n.shell.dock.size(DOCK_RIGHT) < TASK_W {
            n.shell.dock.set_size(DOCK_RIGHT, TASK_W);
        }
    }
}

// the project's lab changed through an edit (the author's panel saved an assessment, an undo):
// its task starts over where the panel is
fn follow(n: &mut Net) {
    if n.doc.rev == n.task.lab_rev {
        return;
    }
    n.task.lab_rev = n.doc.rev;
    if n.doc.proj.lab != n.task.lab {
        let lab = n.doc.proj.lab.clone();
        n.task.reset(lab);
        crate::workspace::assess::loaded(n);
    }
}

/// every frame: the goals follow the simulation
pub(crate) fn tick(n: &mut Net) {
    follow(n);
    let task = &mut n.task;
    let (Some(run), Some(lab)) = (task.run.as_mut(), task.lab.as_ref()) else { return };
    let eng = n.doc.sim.engine();
    let now = eng.now();
    if run.at.is_some_and(|a| now < a) {
        // a fresh engine (Simulation zurücksetzen): the goals start over
        *run = LabRun::new(lab);
    }
    let wall = plat::task::now_us();
    let edited = n.doc.rev != task.rev && wall >= task.at_us + EDIT_US;
    let due = task.force || edited || run.at.is_none_or(|a| now >= a + SEC);
    if !due {
        return;
    }
    task.force = false;
    task.rev = n.doc.rev;
    task.at_us = wall;
    let cons = &n.cons;
    let out = |dev: u32| cons.by_dev.get(&dev).map(|c| c.term.text()).unwrap_or_default();
    run.eval(&n.doc.proj, eng, &out);
}

/// "Prüfen": judged now; the open goals say why they are not reached (an assessment's items too)
pub(crate) fn check(n: &mut Net) {
    crate::workspace::assess::check(n);
    if n.task.run.is_none() {
        return;
    }
    n.task.explain = true;
    n.task.force = true;
    tick(n);
}

// the goal whose hint comes next: the first open one with a hint left
fn next_goal_hint(n: &Net) -> Option<usize> {
    let (lab, run) = (n.task.lab.as_ref()?, n.task.run.as_ref()?);
    (0..lab.goals.len()).find(|&i| run.open(i) && n.task.goal_hints.get(i).is_some_and(|&k| k < lab.goals[i].hints.len()))
}

/// a hint is left to show
fn hint_left(n: &Net) -> bool {
    next_goal_hint(n).is_some() || n.task.lab.as_ref().is_some_and(|l| n.task.hints < l.hints.len())
}

/// the next hint: of the first open goal that has one left, else of the whole task
pub(crate) fn hint(n: &mut Net) {
    if let Some(i) = next_goal_hint(n) {
        n.task.goal_hints[i] += 1;
    } else if n.task.lab.as_ref().is_some_and(|l| n.task.hints < l.hints.len()) {
        n.task.hints += 1;
    }
}

/// enabled / checked of the commands (every frame)
pub(crate) fn states(n: &Net, ui: &mut Ui) {
    let lab = n.task.lab.is_some();
    ui.cmd_state(CMD_TASK_CHECK, if lab { 0 } else { CF_DISABLED });
    ui.cmd_state(CMD_TASK_HINT, if hint_left(n) { 0 } else { CF_DISABLED });
    ui.cmd_state(CMD_TASK_PANEL, if n.shell.dock.visible(P_TASK) { CF_CHECKED } else { 0 });
}

/// the commands of the labs and the panel
pub(crate) fn command(n: &mut Net, ui: &mut Ui, id: u32, arg: u32) -> bool {
    match id {
        CMD_LAB_OPEN => {
            n.task.labs = labs();
            n.task.labs.extend(assessments());
            n.task.lab_sel = n.task.lab_sel.clamp(0, (n.task.labs.len() as i32 - 1).max(0));
            n.task.dlg.set(1);
        }
        CMD_LAB_CHOSEN => {
            let Some(l) = n.task.labs.get(arg as usize).cloned() else { return true };
            file::request(n, ui, After::Lab(l.path));
        }
        CMD_TASK_PANEL => {
            n.shell.dock.show(P_TASK);
            if n.task.lab.is_some() {
                ui.focus(ID_TASK_CHECK);
            } else {
                n.shell.dock.focus(P_TASK);
            }
        }
        CMD_TASK_CHECK => {
            n.shell.dock.show(P_TASK);
            check(n);
        }
        CMD_TASK_HINT => {
            n.shell.dock.show(P_TASK);
            hint(n);
        }
        _ => return false,
    }
    ui.repass();
    true
}

// ---------------------------------------------------------------- the panel

/// the name of a lab's topic in the current language
fn topic_name(topic: &str) -> &str {
    match topic {
        "net" => t("Netzwerk"),
        "ds" => t("Verteilte Systeme"),
        other => other,
    }
}

/// "1 Ziel", "4 Ziele"; an assessment's points ("20 Punkte"), with its goals when it has some
fn size_text(goals: usize, points: u32) -> String {
    match (goals, points) {
        (_, 0) => goals_text(goals),
        (0, p) => crate::workspace::assess::points_text(p),
        (g, p) => format!("{} · {}", goals_text(g), crate::workspace::assess::points_text(p)),
    }
}

/// "1 Ziel", "4 Ziele"
fn goals_text(k: usize) -> String {
    if k == 1 { String::from(t("1 Ziel")) } else { tf("{} Ziele", &[&k.to_string()]) }
}

/// a moment of the simulation ("12,000 s")
fn time_text(ns: u64) -> String {
    tr::msg(&jfn_model::msg::dur(ns.min(i64::MAX as u64) as i64))
}

// an icon in a colour of the theme, at the top of its row (beside the first line of a text)
fn mark(ui: &mut Ui, cp: u32, tok: u32, size: i32) {
    let s = ui.dp(size);
    let r = ui.place(s, s);
    let c = ui.color(tok);
    ui.draw_color(c);
    ui.draw_icon(cp, r[0], r[1] + ui.dp(1), s << 8);
}

// a hint: a light bulb and the text
fn hint_row(ui: &mut Ui, text: &str) {
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H, 6);
    mark(ui, IC_LIGHTBULB, T_WARN, 14);
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_WRAP, text);
    ui.box_end();
}

/// the Aufgabe panel
pub(crate) fn panel(n: &mut Net, ui: &mut Ui) {
    let (Some(lab), Some(run)) = (n.task.lab.clone(), n.task.run.clone()) else {
        ui.next_size(-1, -1);
        ui.box_begin(0, BOX_V | BOX_CENTER | BOX_XCENTER | pad(12), 8);
        ui.empty_state(IC_FLAG, t("Keine Aufgabe"), t("Dieses Projekt hat keine Aufgabe. Ein Labor bringt eine mit: ein Netz, Ziele, die der Simulator prüft, und Hinweise."));
        ui.next_icon(IC_FLAG);
        if ui.button(ID_TASK_OPEN, t("Labor öffnen …")) & EV_ACT != 0 {
            ui.cmd_run(CMD_LAB_OPEN, 0);
        }
        ui.box_end();
        return;
    };
    let en = tr::en();
    let total = lab.goals.len();
    let reached = run.reached();
    let mut act_hint: Option<usize> = None;
    let mut act_general = false;
    let mut act_check = false;
    ui.next_size(-1, -1);
    ui.scroll_begin(ID_TASK, SC_V | pad(12));
    ui.box_begin(0, BOX_V, 10);
    // the title, its topic and size
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    mark(ui, IC_FLAG, T_ACCENT_TEXT, 20);
    ui.next_size(-1, 0);
    ui.text(TS_SUBTITLE | TX_WRAP, lab.title.get(en));
    ui.box_end();
    let points = n.task.assess.as_ref().map(|r| r.grader.total());
    ui.text(TS_CAPTION | TX_SECONDARY, &format!("{} · {}", topic_name(&lab.topic), size_text(total, points.unwrap_or(0))));
    // an assessment: its score card ("Prüfen" there when there are no goals)
    if points.is_some() {
        crate::workspace::assess::card(n, ui, total == 0);
    }
    // the progress and "Prüfen"
    if points.is_none() || total > 0 {
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V | BOX_CARD | pad(10), 6);
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.next_size(-1, 0);
        ui.text(TS_BODY_STRONG, &tf("{} von {} Zielen erreicht", &[&reached.to_string(), &total.to_string()]));
        ui.next_icon(IC_ACCEPT);
        if ui.accent_button(ID_TASK_CHECK, t("Prüfen")) & EV_ACT != 0 {
            act_check = true;
        }
        ui.tooltip(t("Die Ziele jetzt prüfen und bei den offenen zeigen, was fehlt (F9)"));
        ui.box_end();
        ui.next_size(-1, 0);
        ui.progress(ID_TASK_PROGRESS, reached as i32, total.max(1) as i32);
        if run.done() {
            let at = run.goals.iter().filter_map(|g| g.reached).max().unwrap_or(0);
            ui.next_size(-1, 0);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
            mark(ui, IC_CHECK_CIRCLE_FILL, T_OK, 16);
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_WRAP, t("Geschafft: alle Ziele erreicht."));
            ui.box_end();
            ui.text(TS_CAPTION | TX_TERTIARY, &tf("das letzte bei t = {}", &[&time_text(at)]));
        } else if let Some(at) = run.at {
            ui.text(TS_CAPTION | TX_TERTIARY, &tf("geprüft bei t = {}", &[&time_text(at)]));
        }
        ui.box_end();
    }
    // the introduction (it may be folded away)
    if !lab.intro.is_empty() {
        ui.next_size(-1, 0);
        if ui.expander_begin(ID_TASK_INTRO, t("Worum es geht"), &mut n.task.intro) != 0 {
            for p in &lab.intro {
                ui.next_size(-1, 0);
                ui.text(TS_BODY | TX_WRAP, p.get(en));
            }
        }
        ui.expander_end();
    }
    // an assessment's items, as its feedback allows
    if points.is_some() {
        crate::workspace::assess::items(n, ui);
    }
    // the goals
    if total > 0 || points.is_none() {
        ui.text(TS_BODY_STRONG, t("Ziele"));
    }
    for (i, g) in lab.goals.iter().enumerate().take(TASK_GOALS) {
        let gr = &run.goals[i];
        let done = gr.reached.is_some();
        let label = format!("{}. {}", i + 1, g.text.get(en));
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_H, 8);
        // the record of the goal: a check box over the row, checked when it is reached
        let r = ui.box_rect();
        ui.last_set(r, if done { SS_CHECKED as u32 } else { 0 });
        ui.sem(K_CHECKBOX, ID_TASK_GOAL0 + i as u32, Some(&label), done as i32);
        if done {
            mark(ui, IC_CHECK_CIRCLE_FILL, T_OK, 18);
        } else {
            mark(ui, IC_CIRCLE_RING, T_TEXT3, 18);
        }
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V, 4);
        ui.next_size(-1, 0);
        ui.text(if done { TS_BODY | TX_WRAP | TX_SECONDARY } else { TS_BODY | TX_WRAP }, &label);
        match gr.reached {
            Some(at) => ui.text(TS_CAPTION | TX_TERTIARY, &tf("erreicht bei t = {}", &[&time_text(at)])),
            None if !run.open(i) => {
                let before = (0..i).find(|&k| run.goals[k].reached.is_none()).unwrap_or(0);
                ui.text(TS_CAPTION | TX_TERTIARY, &tf("wartet auf Ziel {}", &[&(before + 1).to_string()]));
            }
            None => {
                if let Err((k, why)) = &gr.checks {
                    ui.next_size(-1, 0);
                    crate::workspace::error_text(ui, &tf("Prüfung {} ist fehlerhaft: {}", &[&(k + 1).to_string(), why]), 0);
                } else if n.task.explain {
                    for v in gr.last.iter().filter(|v| !v.held) {
                        let why = tr::msg(&v.reason());
                        let line = if why.is_empty() { check_text(&v.x) } else { format!("{}: {}", check_text(&v.x), why) };
                        ui.next_size(-1, 0);
                        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &line);
                    }
                }
            }
        }
        // the hints opened so far, and the button for the next one
        let shown = n.task.goal_hints.get(i).copied().unwrap_or(0).min(g.hints.len());
        for h in &g.hints[..shown] {
            hint_row(ui, h.get(en));
        }
        if !done && shown < g.hints.len() && run.open(i) {
            ui.box_begin(0, BOX_H, 0);
            ui.next_flags(ST_SUBTLE | ST_COMPACT);
            ui.next_icon(IC_LIGHTBULB);
            let label = if shown == 0 { tc("Hinweis|Aufgabe") } else { t("Noch ein Hinweis") };
            if ui.button(ID_TASK_GOAL_HINT0 + i as u32, label) & EV_ACT != 0 {
                act_hint = Some(i);
            }
            ui.box_end();
        }
        ui.box_end();
        ui.box_end();
    }
    // the hints of the whole task
    if !lab.hints.is_empty() {
        ui.text(TS_BODY_STRONG, t("Hinweise"));
        let shown = n.task.hints.min(lab.hints.len());
        for h in &lab.hints[..shown] {
            hint_row(ui, h.get(en));
        }
        if shown < lab.hints.len() {
            ui.box_begin(0, BOX_H, 0);
            ui.next_flags(ST_COMPACT);
            ui.next_icon(IC_LIGHTBULB);
            let label = tf("Hinweis zeigen ({} von {})", &[&(shown + 1).to_string(), &lab.hints.len().to_string()]);
            if ui.button(ID_TASK_HINT, &label) & EV_ACT != 0 {
                act_general = true;
            }
            ui.box_end();
        }
    }
    ui.box_end();
    ui.scroll_end();
    // what the buttons asked for, after the widgets
    if act_check {
        check(n);
        ui.repass();
    }
    if let Some(i) = act_hint {
        if let Some(k) = n.task.goal_hints.get_mut(i) {
            *k += 1;
        }
        ui.repass();
    }
    if act_general {
        n.task.hints += 1;
        ui.repass();
    }
}

// ---------------------------------------------------------------- the list of labs

/// Datei › Labor öffnen …: the labs of the app
pub(crate) fn dialog(n: &mut Net, ui: &mut Ui) {
    let open = n.task.dlg.clone();
    if ui.dialog_begin(ID_LABS_DLG, t("Labor öffnen"), &open) {
        let en = tr::en();
        let count = n.task.labs.len() as i32;
        let mut chosen: Option<i32> = None;
        ui.box_begin(0, BOX_V, 8);
        ui.next_size(520, 0);
        ui.text(
            TS_BODY | TX_SECONDARY | TX_WRAP,
            t("Ein Labor ist ein Projekt mit einer Aufgabe: das Netz zum Start, Ziele, die der Simulator prüft, und Hinweise. Es öffnet sich ohne Datei; Speichern legt eine Kopie an."),
        );
        if count == 0 {
            ui.text(TS_BODY, t("Keine Labore gefunden."));
        } else {
            let mut sel = n.task.lab_sel.clamp(0, count - 1);
            ui.next_size(520, (count * 56 + 4).min(340));
            let (first, end) = ui.list_begin(ID_LABS_LIST, &mut sel, count, 56);
            for i in first..end {
                let l = &n.task.labs[i as usize];
                ui.list_item_begin(i);
                ui.next_size(-1, 0);
                ui.box_begin(0, BOX_V, 2);
                ui.next_size(-1, 0);
                ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
                ui.badge(if l.topic == "net" { BADGE_INFO } else { BADGE_ACCENT }, topic_name(&l.topic));
                if l.points > 0 {
                    ui.next_icon(IC_ACCEPT);
                    ui.badge(BADGE_OK, t("Mit Punkten"));
                }
                ui.next_size(-1, 0);
                ui.text(TS_BODY_STRONG | TX_ELLIPSIS, l.title.get(en));
                ui.box_end();
                let line = format!("{} · {}", size_text(l.goals, l.points), l.summary.get(en));
                ui.next_size(-1, 0);
                ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &line);
                ui.tooltip(&line);
                ui.box_end();
                ui.list_item_end();
            }
            let (ev, s) = ui.list_end(&mut sel);
            n.task.lab_sel = s;
            if ev & EV_DOUBLE != 0 && s >= 0 {
                chosen = Some(s);
                open.set(0);
            }
        }
        ui.box_end();
        let primary = if count == 0 { None } else { Some(t("Öffnen")) };
        if ui.dialog_buttons(primary, None, Some(t("Abbrechen"))) == DLG_PRIMARY {
            chosen = Some(n.task.lab_sel);
        }
        if let Some(i) = chosen.filter(|&i| i >= 0) {
            ui.cmd_run(CMD_LAB_CHOSEN, i as u32);
        }
    }
    ui.dialog_end();
}
