// assess -- an assessment in the Aufgabe panel (A1): the grade of the open project (points, a
// percentage, the items as the feedback mode allows), the time limit and handing in
//
//   Run::new(&assess, &project, clock)    the items prepared (jfn_sim::assess::Grader); graded at
//                                         the next tick; a time limit runs from `clock` (µs of
//                                         the task clock)
//   run.due(rev, ver, clock)              a grade is due: "Prüfen", else while the score shows a
//                                         change of the project (`rev`, at most every EDIT_US) or
//                                         of the simulator (`ver`, a new snapshot: what the
//                                         consoles did, the clock moving with events; at most
//                                         every EVERY_US); never once it ended
//   run.wake(rev, ver, clock) -> Option<ms>   when the window must look again without anything
//                                         else to draw: the countdown each second, a grade that
//                                         waits for its interval (TIMER_ASSESS)
//   run.grade(&project, &engine, out, rev, ver, clock)   the items now (Score)
//   run.remaining(clock) -> Option<u64>   what is left of the time limit (None: there is none)
//   run.time_up(clock)                    the limit is over and it has not ended yet
//   run.finish(&project, &engine, out, clock, timeout)   handed in or out of time: graded a last
//                                         time, the final score stays
//   run.shows_score() / shows_items()     what the feedback mode shows now: Items both, Score the
//                                         score, End nothing; once it ended, both
//   loaded(n) / tick(n, ui) / check(n) / timer(n, ui)   the window's (task.rs calls them)
//   section(n, ui)                        the score card and the items (in task.rs's panel)
//   dialog(n, ui)                         "Jetzt abgeben?" before handing in
//   points_text(k) / clock_text(us)       "20 Punkte", "14:05"
//
// The time limit is the task clock's (plat::task::now_us: a script's `wait`), not the simulation's:
// pausing or speeding up the simulator does not change it.  It starts when the assessment opens
// and is not saved with the project.  Screen readers: the score is a progress bar, each item a
// check box record named by its text (checked: it holds; it cannot be toggled), the findings and
// the time text records.

use alloc::format;
use alloc::string::{String, ToString};

use jfn_model::lab::{Assess, Feedback};
use jfn_model::Project;
use jfn_sim::assess::{item_text, Grader, Score};
use jfn_sim::{Engine, Ns};
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{self, t, tf};
use crate::app::Net;

/// a change of the project grades again after this long at the earliest (µs of the task clock)
pub(crate) const EDIT_US: u64 = 500_000;
/// while the score shows, it is graded again this often (µs of the task clock)
pub(crate) const EVERY_US: u64 = 2_000_000;

/// how an assessment ended
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct End {
    /// the task clock and the simulation's time then
    pub at_us: u64,
    pub virt: Ns,
    /// the time was up (else it was handed in)
    pub timeout: bool,
}

/// an assessment over a session of the window
pub(crate) struct Run {
    pub grader: Grader,
    pub feedback: Feedback,
    /// the time limit (µs, 0: none) and when it began (task clock)
    pub limit_us: u64,
    pub started_us: u64,
    /// the last grade, its virtual time, the task clock, the project's revision and the
    /// simulator's snapshot then
    pub score: Option<Score>,
    pub at: Option<Ns>,
    clock_at: u64,
    rev: u32,
    ver: u32,
    /// grade at the next tick
    pub force: bool,
    /// "Prüfen" was used: the items that do not hold say what they found
    pub explain: bool,
    /// handed in or out of time (the score is final then)
    pub end: Option<End>,
    /// the question before handing in is open
    pub ask: Shared<i32>,
}

impl Run {
    pub(crate) fn new(a: &Assess, p: &Project, clock: u64) -> Run {
        Run {
            grader: Grader::new(a, p),
            feedback: a.feedback,
            limit_us: a.time_limit_s as u64 * 1_000_000,
            started_us: clock,
            score: None,
            at: None,
            clock_at: 0,
            rev: 0,
            ver: 0,
            force: true,
            explain: false,
            end: None,
            ask: Shared::new(0),
        }
    }

    /// the feedback mode shows the score now
    pub(crate) fn shows_score(&self) -> bool {
        self.end.is_some() || self.feedback != Feedback::End
    }

    /// the feedback mode shows each item now
    pub(crate) fn shows_items(&self) -> bool {
        self.end.is_some() || self.feedback == Feedback::Items
    }

    /// what is left of the time limit (µs; None: there is none)
    pub(crate) fn remaining(&self, clock: u64) -> Option<u64> {
        if self.limit_us == 0 {
            return None;
        }
        let till = match &self.end {
            Some(e) => e.at_us,
            None => clock,
        };
        Some((self.started_us + self.limit_us).saturating_sub(till))
    }

    /// the time limit is over and the assessment has not ended yet
    pub(crate) fn time_up(&self, clock: u64) -> bool {
        self.end.is_none() && self.limit_us > 0 && clock >= self.started_us + self.limit_us
    }

    // the task clock when a change (project `rev`, simulator `ver`) is graded next, None:
    // nothing changed since the last grade (or it ended, or nothing shows)
    fn next_grade(&self, rev: u32, ver: u32) -> Option<u64> {
        if self.end.is_some() || !self.shows_score() {
            return None;
        }
        if rev != self.rev {
            Some(self.clock_at + EDIT_US)
        } else if ver != self.ver {
            Some(self.clock_at + EVERY_US)
        } else {
            None
        }
    }

    /// a grade is due now (project revision `rev`, simulator snapshot `ver`, task clock `clock`)
    pub(crate) fn due(&self, rev: u32, ver: u32, clock: u64) -> bool {
        if self.end.is_some() {
            return false;
        }
        self.force || self.next_grade(rev, ver).is_some_and(|at| clock >= at)
    }

    /// when the window must look again (ms), without anything else to draw: the countdown,
    /// a change that waits for its grade
    pub(crate) fn wake(&self, rev: u32, ver: u32, clock: u64) -> Option<u32> {
        if self.end.is_some() {
            return None;
        }
        let tick = if self.limit_us > 0 { Some(1000) } else { None };
        let grade = self.next_grade(rev, ver).map(|at| (at.saturating_sub(clock) / 1000).max(16) as u32);
        match (tick, grade) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// the items against the project and its engine now
    pub(crate) fn grade(&mut self, p: &Project, e: &Engine, out: &dyn Fn(u32) -> String, rev: u32, ver: u32, clock: u64) {
        self.score = Some(self.grader.grade(p, e, out));
        self.at = Some(e.now());
        self.clock_at = clock;
        self.rev = rev;
        self.ver = ver;
        self.force = false;
    }

    /// handed in (or out of time): graded a last time, then the score stays
    pub(crate) fn finish(&mut self, p: &Project, e: &Engine, out: &dyn Fn(u32) -> String, rev: u32, ver: u32, clock: u64, timeout: bool) {
        if self.end.is_some() {
            return;
        }
        // out of time: it ends at the limit, whenever the window noticed it
        let at_us = if timeout { self.started_us + self.limit_us } else { clock };
        self.grade(p, e, out, rev, ver, clock);
        self.end = Some(End { at_us, virt: e.now(), timeout });
        self.explain = true;
    }
}

/// "1 Punkt", "20 Punkte"
pub(crate) fn points_text(k: u32) -> String {
    if k == 1 { String::from(t("1 Punkt")) } else { tf("{} Punkte", &[&k.to_string()]) }
}

/// a span of the task clock as a clock shows it: "4:05", "1:02:03"
pub(crate) fn clock_text(us: u64) -> String {
    let s = us.div_ceil(1_000_000);
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

// ---------------------------------------------------------------- the window

/// a new or loaded project (task.rs's loaded): its assessment starts over
pub(crate) fn loaded(n: &mut Net) {
    let clock = plat::task::now_us();
    n.task.assess = match n.doc.proj.lab.as_ref().and_then(|l| l.assess.as_ref()) {
        Some(a) => Some(Run::new(a, &n.doc.proj, clock)),
        None => None,
    };
}

/// every frame (task.rs's tick): graded when due, ended when the time is up; the timer when
/// the window must look again
pub(crate) fn tick(n: &mut Net, ui: &mut Ui) {
    let Some(run) = n.task.assess.as_mut() else { return };
    let clock = plat::task::now_us();
    let (rev, ver) = (n.doc.rev, n.doc.sim.ver);
    let cons = &n.cons;
    let out = |dev: u32| cons.by_dev.get(&dev).map(|c| c.term.text()).unwrap_or_default();
    let eng = n.doc.sim.engine();
    // a fresh engine (Simulation zurücksetzen): graded again at once
    if run.at.is_some_and(|a| eng.now() < a) {
        run.force = true;
    }
    if run.time_up(clock) {
        run.finish(&n.doc.proj, eng, &out, rev, ver, clock, true);
        ui.repass();
        return;
    }
    if run.due(rev, ver, clock) {
        run.grade(&n.doc.proj, eng, &out, rev, ver, clock);
        ui.repass();
    }
    if !n.task.timer_armed
        && let Some(ms) = run.wake(rev, ver, clock)
    {
        n.task.timer_armed = true;
        ui.timer(TIMER_ASSESS, ms);
    }
}

/// TIMER_ASSESS: a frame (its tick grades what waits, moves the countdown and sets the timer
/// again when it is needed)
pub(crate) fn timer(n: &mut Net, ui: &mut Ui) {
    n.task.timer_armed = false;
    ui.repass();
}

/// "Prüfen": graded now; the items that do not hold say what they found
pub(crate) fn check(n: &mut Net) {
    if let Some(run) = n.task.assess.as_mut() {
        run.force = run.end.is_none();
        run.explain = true;
    }
}

/// "Abgeben": graded a last time, the final score shows
pub(crate) fn submit(n: &mut Net) {
    let Some(run) = n.task.assess.as_mut() else { return };
    let clock = plat::task::now_us();
    let cons = &n.cons;
    let out = |dev: u32| cons.by_dev.get(&dev).map(|c| c.term.text()).unwrap_or_default();
    run.finish(&n.doc.proj, n.doc.sim.engine(), &out, n.doc.rev, n.doc.sim.ver, clock, false);
}

// an icon in a colour of the theme, at the top of its row
fn mark(ui: &mut Ui, cp: u32, tok: u32, size: i32) {
    let s = ui.dp(size);
    let r = ui.place(s, s);
    let c = ui.color(tok);
    ui.draw_color(c);
    ui.draw_icon(cp, r[0], r[1] + ui.dp(1), s << 8);
}

/// the score card: the points, "Prüfen" (`check_button`: the lab has no goals, whose card has
/// it) or "Abgeben", the time
pub(crate) fn card(n: &mut Net, ui: &mut Ui, check_button: bool) {
    let Some(run) = n.task.assess.as_ref() else { return };
    let clock = plat::task::now_us();
    let total = run.grader.total();
    let score = run.score.clone();
    let shows_score = run.shows_score();
    let shows_items = run.shows_items();
    let end = run.end.clone();
    let left = run.remaining(clock);
    let feedback = run.feedback;
    let at = run.at;
    let mut act_check = false;
    let mut act_submit = false;
    // ---- the card: the score, "Prüfen" / "Abgeben", the time
    ui.next_size(-1, 0);
    ui.box_begin(ID_AS_CARD, BOX_V | BOX_CARD | pad(10), 6);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(-1, 0);
    match (&score, shows_score) {
        (Some(s), true) => ui.text(TS_BODY_STRONG, &tf("{} von {} Punkten · {} %", &[&s.points.to_string(), &total.to_string(), &s.percent().to_string()])),
        (None, true) => ui.text(TS_BODY_STRONG, &tf("? von {} Punkten", &[&total.to_string()])),
        (_, false) => ui.text(TS_BODY_STRONG | TX_WRAP, t("Die Bewertung zeigt sich nach dem Abgeben.")),
    }
    if end.is_none() {
        if feedback == Feedback::End {
            ui.next_icon(IC_UPLOAD);
            if ui.accent_button(ID_TASK_CHECK, t("Abgeben")) & EV_ACT != 0 {
                act_submit = true;
            }
            ui.tooltip(t("Die Arbeit abgeben: danach zählt die Bewertung, wie sie jetzt ist"));
        } else if check_button {
            ui.next_icon(IC_ACCEPT);
            if ui.accent_button(ID_TASK_CHECK, t("Prüfen")) & EV_ACT != 0 {
                act_check = true;
            }
            ui.tooltip(t("Die Kriterien jetzt prüfen und bei den falschen zeigen, was fehlt (F9)"));
        }
    }
    ui.box_end();
    if shows_score {
        let points = score.as_ref().map_or(0, |s| s.points);
        ui.next_size(-1, 0);
        ui.progress(ID_AS_PROGRESS, points as i32, total.max(1) as i32);
    }
    match (&end, left) {
        (Some(e), _) => {
            ui.next_size(-1, 0);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
            mark(ui, if e.timeout { IC_RECENT } else { IC_FLAG }, T_ACCENT_TEXT, 16);
            ui.next_size(-1, 0);
            let s = if e.timeout { t("Die Zeit ist um: die Bewertung ist endgültig.") } else { t("Abgegeben: die Bewertung ist endgültig.") };
            ui.text(TS_BODY | TX_WRAP, s);
            ui.box_end();
            ui.text(TS_CAPTION | TX_TERTIARY, &tf("bewertet bei t = {}", &[&tr::msg(&jfn_model::msg::dur(e.virt.min(i64::MAX as u64) as i64))]));
        }
        (None, Some(us)) => {
            ui.next_size(-1, 0);
            ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
            mark(ui, IC_RECENT, T_TEXT2, 16);
            ui.next_size(-1, 0);
            ui.text(TS_BODY, &tf("Noch {}", &[&clock_text(us)]));
            if feedback != Feedback::End {
                ui.next_flags(ST_SUBTLE | ST_COMPACT);
                ui.next_icon(IC_UPLOAD);
                if ui.button(ID_AS_SUBMIT, t("Abgeben")) & EV_ACT != 0 {
                    act_submit = true;
                }
            }
            ui.box_end();
        }
        (None, None) => {
            if let (Some(at), true) = (at, shows_score) {
                ui.text(TS_CAPTION | TX_TERTIARY, &tf("geprüft bei t = {}", &[&tr::msg(&jfn_model::msg::dur(at.min(i64::MAX as u64) as i64))]));
            }
        }
    }
    if !shows_items && feedback == Feedback::Score && end.is_none() {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Diese Aufgabe zeigt nur die Punktzahl, nicht die einzelnen Kriterien."));
    }
    ui.box_end();
    if act_check {
        crate::workspace::task::check(n);
        ui.repass();
    }
    if act_submit && let Some(run) = n.task.assess.as_ref() {
        run.ask.set(1);
        ui.repass();
    }
}

/// the items as the feedback mode allows: a mark, the text, the points and (after "Prüfen", or
/// once it ended) what a wrong one found
pub(crate) fn items(n: &mut Net, ui: &mut Ui) {
    let Some(run) = n.task.assess.as_ref() else { return };
    if !run.shows_items() {
        return;
    }
    let en = tr::en();
    let score = run.score.clone();
    let explain = run.explain || run.end.is_some();
    {
        let items = run.grader.items().to_vec();
        ui.text(TS_BODY_STRONG, t("Kriterien"));
        for (i, it) in items.iter().enumerate().take(ASSESS_ITEMS) {
            let g = score.as_ref().and_then(|s| s.items.get(i));
            let held = g.is_some_and(|g| g.held);
            let text = if it.text.is_empty() { tr::msg(&item_text(it)) } else { String::from(it.text.get(en)) };
            ui.next_size(-1, 0);
            ui.box_begin(0, BOX_H, 8);
            let r = ui.box_rect();
            ui.last_set(r, if held { SS_CHECKED as u32 } else { 0 });
            ui.sem(K_CHECKBOX, ID_AS_ITEM0 + i as u32, Some(&text), held as i32);
            match g {
                Some(g) if g.held => mark(ui, IC_CHECK_CIRCLE_FILL, T_OK, 18),
                Some(_) => mark(ui, IC_CANCEL_CIRCLE, T_CRIT, 18),
                None => mark(ui, IC_CIRCLE_RING, T_TEXT3, 18),
            }
            ui.next_size(-1, 0);
            ui.box_begin(0, BOX_V, 2);
            ui.next_size(-1, 0);
            ui.box_begin(0, BOX_H, 8);
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_WRAP, &text);
            let pts = format!("{}/{}", g.map_or(0, |g| g.points), it.points);
            ui.text(TS_CAPTION | TX_SECONDARY, &pts);
            ui.box_end();
            if let Some(g) = g
                && !g.held
                && explain
            {
                let why = tr::msg(&g.found);
                if !why.is_empty() {
                    ui.next_size(-1, 0);
                    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &why);
                }
            }
            ui.box_end();
            ui.box_end();
        }
    }
}

/// "Jetzt abgeben?" (at the end of the frame)
pub(crate) fn dialog(n: &mut Net, ui: &mut Ui) {
    let Some(ask) = n.task.assess.as_ref().map(|r| r.ask.clone()) else { return };
    if ui.dialog_begin(ID_AS_ASK, t("Jetzt abgeben?"), &ask) {
        ui.next_size(380, 0);
        ui.text(TS_BODY | TX_WRAP, t("Danach zählt die Bewertung, wie sie jetzt ist, und die Kriterien zeigen, was fehlt. Weiterarbeiten geht, die Punkte bleiben."));
        if ui.dialog_buttons(Some(t("Abgeben")), None, Some(t("Abbrechen"))) == DLG_PRIMARY {
            ui.cmd_run(CMD_ASSESS_SUBMIT, 0);
        }
    }
    ui.dialog_end();
}

/// the assessment's commands: Abgeben (after its question)
pub(crate) fn command(n: &mut Net, ui: &mut Ui, id: u32) -> bool {
    match id {
        CMD_ASSESS_SUBMIT => {
            n.shell.dock.show(P_TASK);
            submit(n);
        }
        _ => return false,
    }
    ui.repass();
    true
}

/// enabled of the commands (every frame)
pub(crate) fn states(n: &Net, ui: &mut Ui) {
    let open = n.task.assess.as_ref().is_some_and(|r| r.end.is_none());
    ui.cmd_state(CMD_ASSESS_SUBMIT, if open { 0 } else { CF_DISABLED });
}
