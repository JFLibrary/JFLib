// quiz -- the Akademie's page of a quiz or a test (jfn_model::quiz): the questions as the library's own
// widgets, never as a form in a document, so that no solution stands in anything the learner can read
//
//   open(n, i)          the step at place i of the current course on the page: what this session has of it,
//                       else its file (a bad file: its messages on the page, nothing else is hurt)
//   frame(n, ui, i)     the page (pages.rs's frame)
//   tick(n, ui)         every frame: a test whose time is up is handed in, also while another page shows; its
//                       countdown's timer (TIMER_QUIZ)
//   timer(n, ui)        TIMER_QUIZ: a frame
//   command(n, id)      CMD_QZ_START, CMD_QZ_HAND_IN, CMD_QZ_AGAIN
//   dialog(n, ui)       "Jetzt abgeben?", "Programm wirklich beenden?"
//   drop_course(n, c)   Fortschritt zurücksetzen: what this session has of a course's quizzes goes
//   running(a)          an attempt at a test is under way (install.rs waits for it)
//   ask_quit(n, ui)     leaving the program with an attempt under way: the question first
//
// Two modes by the step's kind.  A quiz (practice) shows its questions at once: "Prüfen" per question says
// right or wrong and gives the explanation at once, a retry is free, a question keeps the best it reached;
// the result (the sum of those) goes to Progress.best, and the step is done as soon as it passes.  A test
// (exam) starts with what it asks (questions, points, time, attempts, what passes) and "Test beginnen"; then
// all its questions with the time on top; "Abgeben" asks once and grades once; the result has the points per
// question and every explanation; a passed test marks the step done, the best result stays in the progress,
// and "Neuer Versuch" starts the next attempt while there are attempts left.  An attempt in progress lives
// in this session only and says so: its clock is the app's (plat::task::now_us: a script's `wait`), it runs
// on while another page shows, and only attempts handed in count.
//
// The order of an attempt comes from a seed chosen for it; a run without a window takes a fixed one, so that
// its scripts are the same every time.  Ids: ID_QZ_Q + q * QZ_SLOTS + a slot per question by its place in the
// file: the stem, the input, Prüfen, the explanation, the answers.  Screen readers: a stem is a document
// named "Frage N", an answer a radio or a check box named by its text, an order's entry a text with its
// buttons named after it, the result text records.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::academy::{quiz_issues, Problem, StepKind};
use jfn_model::lab::Text;
use jfn_model::quiz::{Attempt, Marked, QKind, Question, Quiz, Reply, MILLI};
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{self, t, tf};
use crate::app::Net;
use crate::workspace::academy::{pages, store, View};
use crate::workspace::assess::clock_text;
use crate::workspace::task::mark;

/// the slots of a question's ids
const SLOT_STEM: u32 = 0;
const SLOT_INPUT: u32 = 1;
const SLOT_CHECK: u32 = 2;
const SLOT_EXPLAIN: u32 = 3;
const SLOT_ANSWER: u32 = 4;
/// the fixed seed of a run without a window
const SEED_FIXED: u64 = 0x4A46_4E41_4B32;
/// the longest text a reply takes (bytes)
const REPLY_MAX: u32 = 200;

/// a quiz or test of this session
pub(crate) struct Page {
    pub course: usize,
    pub step: usize,
    pub exam: bool,
    /// None: its file cannot be used, `problems` say why
    pub quiz: Option<Quiz>,
    pub problems: Vec<Problem>,
    /// the attempt (practice: always one)
    pub at: Option<Attempt>,
    /// per question by its place in the file: the views of its stem and its explanation
    pub stems: Vec<MdViewState>,
    pub notes: Vec<MdViewState>,
    /// practice: what its last Prüfen found, the most it reached (thousandths)
    pub checked: Vec<Option<Marked>>,
    pub best: Vec<u32>,
    /// the question before handing in
    pub ask: Shared<i32>,
    /// the attempts of this session (a part of the seed)
    pub runs: u64,
}

fn stem_id(q: usize, slot: u32) -> u32 {
    ID_QZ_Q + q as u32 * QZ_SLOTS + slot
}

// a view of a stem or an explanation: pictures from the course's folder only
fn view(dir: &str) -> MdViewState {
    let mut md = MdViewState::new();
    md.base = format!("{}/", dir);
    md.images = MI_APP;
    md
}

// the seed of the next attempt
fn seed(n: &Net, runs: u64) -> u64 {
    let base = if n.academy.headless { SEED_FIXED } else { plat::unix_time_ns() as u64 };
    base.wrapping_add(runs.wrapping_mul(0x9E37_79B9))
}

// a new attempt on page k
fn begin(n: &mut Net, k: usize) {
    let s = seed(n, n.academy.quizzes[k].runs);
    let p = &mut n.academy.quizzes[k];
    let Some(q) = &p.quiz else { return };
    p.at = Some(Attempt::new(q, s, plat::task::now_us()));
    p.runs += 1;
    p.checked = alloc::vec![None; q.questions.len()];
    p.best = alloc::vec![0; q.questions.len()];
}

// the page of the step at place i of the current course (read now the first time)
fn page_of(n: &mut Net, i: usize) -> Option<usize> {
    let a = &mut n.academy;
    let cur = a.cur;
    if let Some(k) = a.quizzes.iter().position(|p| p.course == cur && p.step == i) {
        return Some(k);
    }
    let e = a.entries.get(cur)?;
    let s = e.usable()?.step_at(i)?;
    let exam = s.kind() == Some(StepKind::Exam);
    let text = store::read_quiz(&e.dir, &s.file);
    let problems: Vec<Problem> = match &text {
        Some(t) => quiz_issues(&s.id, if exam { StepKind::Exam } else { StepKind::Quiz }, t),
        None => alloc::vec![Problem { fatal: true, msg: jfn_model::msg!("Schritt {}: die Datei {} fehlt.", s.id, s.file) }],
    };
    let good = !problems.iter().any(|p| p.fatal);
    let quiz = text.as_deref().filter(|_| good).and_then(|t| Quiz::from_text(t).ok());
    let count = quiz.as_ref().map_or(0, |q| q.questions.len());
    let (stems, notes) = ((0..count).map(|_| view(&e.dir)).collect(), (0..count).map(|_| view(&e.dir)).collect());
    a.quizzes.push(Page { course: cur, step: i, exam, quiz, problems, at: None, stems, notes, checked: Vec::new(), best: Vec::new(), ask: Shared::new(0), runs: 0 });
    let k = a.quizzes.len() - 1;
    if !exam {
        begin(n, k);
    }
    Some(k)
}

/// the step at place i of the current course on the page
pub(crate) fn open(n: &mut Net, i: usize) {
    let Some(k) = page_of(n, i) else { return };
    let a = &mut n.academy;
    let p = &a.quizzes[k];
    let first = p.at.as_ref().and_then(|at| at.plan.questions.first().copied());
    a.focus = match (&p.quiz, p.exam, &p.at) {
        (None, ..) => ID_AK_BACK,
        (_, true, None) => ID_QZ_START,
        (_, true, Some(x)) if x.end.is_some() => ID_AK_NEXT,
        _ => first.map_or(ID_AK_BACK, |q| stem_id(q, SLOT_STEM)),
    };
    a.focus_quiet = true;
    a.view = View::Quiz(i);
    a.on = true;
}

/// Fortschritt zurücksetzen: what this session has of the course's quizzes goes
pub(crate) fn drop_course(n: &mut Net, course: usize) {
    n.academy.quizzes.retain(|p| p.course != course);
}

/// leaving the program: with an attempt at a test under way the question first (its answers live in this session
/// only) -> the question is up, nothing else happens now
pub(crate) fn ask_quit(n: &mut Net, ui: &mut Ui) -> bool {
    let a = &mut n.academy;
    if core::mem::take(&mut a.quit_ok) || !running(a) {
        return false;
    }
    a.ask_quit.set(1);
    ui.repass();
    true
}

/// a page without a quiz (selftest: Academy::relist clears them)
pub(crate) fn blank_page() -> Page {
    Page { course: 0, step: 0, exam: false, quiz: None, problems: Vec::new(), at: None, stems: Vec::new(), notes: Vec::new(), checked: Vec::new(), best: Vec::new(), ask: Shared::new(0), runs: 0 }
}

/// an attempt at a test is under way, on this page or another (practice has nothing to lose: it counts per question)
pub(crate) fn running(a: &crate::workspace::academy::Academy) -> bool {
    a.quizzes.iter().any(|p| p.exam && p.at.as_ref().is_some_and(|x| x.end.is_none()))
}

// ---------------------------------------------------------------- what counts

// an attempt ended on page k: its result into the progress
fn handed_in(n: &mut Net, k: usize) {
    let a = &mut n.academy;
    let p = &a.quizzes[k];
    let Some(e) = p.at.as_ref().and_then(|x| x.end.as_ref()) else { return };
    let (percent, passed, course, step) = (e.score.percent(), e.passed, p.course, p.step);
    let ok = a.result(course, step, percent, passed);
    super::saved(n, ok);
}

// practice on page k: the sum of the best of each question into the progress, done when it passes
fn practised(n: &mut Net, k: usize) {
    let a = &mut n.academy;
    let p = &a.quizzes[k];
    let Some(q) = &p.quiz else { return };
    let got: u64 = p.best.iter().map(|&b| b as u64).sum();
    let percent = (got * 100 / q.total().max(1)) as u32;
    let passed = percent >= q.pass_percent();
    let (course, step) = (p.course, p.step);
    let ok = a.note(course, step, percent, passed);
    super::saved(n, ok);
}

/// every frame: a test whose time is up is handed in; the timer of a countdown that shows
pub(crate) fn tick(n: &mut Net, ui: &mut Ui) {
    let clock = plat::task::now_us();
    let mut running = false;
    for k in 0..n.academy.quizzes.len() {
        let p = &mut n.academy.quizzes[k];
        let Some(q) = p.quiz.as_ref() else { continue };
        let Some(at) = p.at.as_mut() else { continue };
        if at.time_up(clock) {
            at.finish(q, clock, true);
            p.ask.set(0);
            handed_in(n, k);
            ui.repass();
        } else if at.end.is_none() && at.limit_us > 0 {
            running = true;
        }
    }
    if running && !n.academy.timer_armed {
        n.academy.timer_armed = true;
        ui.timer(TIMER_QUIZ, 1000);
    }
}

/// TIMER_QUIZ: a frame (its tick moves the countdown and sets the timer again)
pub(crate) fn timer(n: &mut Net, ui: &mut Ui) {
    n.academy.timer_armed = false;
    ui.repass();
}

// the page the view shows
fn shown(n: &Net) -> Option<usize> {
    let View::Quiz(i) = n.academy.view else { return None };
    n.academy.quizzes.iter().position(|p| p.course == n.academy.cur && p.step == i)
}

/// the commands of a quiz's page
pub(crate) fn command(n: &mut Net, id: u32) {
    let Some(k) = shown(n) else { return };
    match id {
        CMD_QZ_START => {
            let a = &n.academy;
            let p = &a.quizzes[k];
            let used = a.tries_at(p.course, p.step);
            if p.quiz.as_ref().is_some_and(|q| q.can_start(used)) {
                begin(n, k);
                let a = &mut n.academy;
                let first = a.quizzes[k].at.as_ref().and_then(|x| x.plan.questions.first().copied());
                a.focus = first.map_or(ID_AK_BACK, |q| stem_id(q, SLOT_STEM));
                a.focus_quiet = true;
            }
        }
        CMD_QZ_HAND_IN => {
            let p = &mut n.academy.quizzes[k];
            let (Some(q), Some(at)) = (p.quiz.as_ref(), p.at.as_mut()) else { return };
            if at.end.is_some() {
                return;
            }
            at.finish(q, plat::task::now_us(), false);
            handed_in(n, k);
            n.academy.focus = ID_AK_NEXT;
        }
        CMD_QZ_AGAIN => {
            begin(n, k);
            let a = &mut n.academy;
            let first = a.quizzes[k].at.as_ref().and_then(|x| x.plan.questions.first().copied());
            a.focus = first.map_or(ID_AK_BACK, |q| stem_id(q, SLOT_STEM));
            a.focus_quiet = true;
        }
        _ => {}
    }
}

// ---------------------------------------------------------------- texts

/// thousandths of a point as a person reads them: "2", "1,5"
pub(crate) fn milli_text(m: u64) -> String {
    let whole = m / MILLI as u64;
    let frac = m % MILLI as u64;
    if frac == 0 {
        return whole.to_string();
    }
    let f = format!("{:03}", frac);
    format!("{}{}{}", whole, if tr::en() { '.' } else { ',' }, f.trim_end_matches('0'))
}

// "1 Punkt", "2 Punkte", "1,5 Punkte"
fn points_of(m: u64) -> String {
    if m == MILLI as u64 { String::from(t("1 Punkt")) } else { tf("{} Punkte", &[&milli_text(m)]) }
}

// what a question's kind asks for, under its stem
fn hint_of(k: QKind) -> &'static str {
    match k {
        QKind::Single => t("Eine Antwort ist richtig."),
        QKind::Multi => t("Mehrere Antworten können richtig sein."),
        QKind::Text => t("Tragen Sie Ihre Antwort ein."),
        QKind::Number => t("Tragen Sie eine Zahl ein."),
        QKind::Order => t("Bringen Sie die Einträge in die richtige Reihenfolge."),
    }
}

// ---------------------------------------------------------------- the page

/// the page of the quiz or test at place i
pub(crate) fn frame(n: &mut Net, ui: &mut Ui, i: usize) {
    if shown(n).is_none() && page_of(n, i).is_none() {
        return;
    }
    let Some(k) = shown(n) else { return };
    let en = tr::en();
    let clock = plat::task::now_us();
    let a = &n.academy;
    let Some(c) = a.course() else { return };
    let Some(s) = c.step_at(i) else { return };
    let total = c.steps().count();
    let title = String::from(s.title.get(en));
    let names = format!("{} › {}", c.title.get(en), title);
    let left = a.quizzes[k].at.as_ref().filter(|x| x.end.is_none()).and_then(|x| x.remaining(clock));
    // the head: back, where this is, the time
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_LAYER | pad(6), 8);
    pages::back_button(n, ui);
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_SECONDARY | TX_ELLIPSIS, &names);
    if let Some(us) = left {
        mark(ui, IC_RECENT, T_TEXT2, 16);
        ui.text(TS_BODY_STRONG, &tf("Noch {}", &[&clock_text(us)]));
    }
    ui.text(TS_CAPTION | TX_SECONDARY, &tf("Schritt {} von {}", &[&(i + 1).to_string(), &total.to_string()]));
    ui.spacer(4);
    ui.box_end();
    // the questions in their column
    pages::column_begin(ui, ID_QZ_SCROLL);
    ui.next_size(-1, 0);
    ui.text(TS_TITLE | TX_WRAP, &title);
    let mut link = None;
    if n.academy.quizzes[k].quiz.is_none() {
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Dieser Schritt lässt sich nicht öffnen. Das stimmt mit seiner Datei nicht:"));
        for p in n.academy.quizzes[k].problems.iter().filter(|p| p.fatal) {
            ui.next_size(-1, 0);
            crate::workspace::error_text(ui, &tr::msg(&p.msg), 0);
        }
    } else if n.academy.quizzes[k].exam {
        match &n.academy.quizzes[k].at {
            None => ready(n, ui, k),
            Some(x) if x.end.is_none() => link = questions(n, ui, k, en),
            Some(_) => link = result(n, ui, k, en),
        }
    } else {
        link = practice(n, ui, k, en);
    }
    pages::column_end(ui);
    foot(n, ui, k, i);
    if let Some(l) = link {
        super::follow_link(n, ui, &l);
    }
}

// the foot: back and on; while a test runs, Abgeben
fn foot(n: &mut Net, ui: &mut Ui, k: usize, i: usize) {
    let en = tr::en();
    let a = &n.academy;
    let Some(c) = a.course() else { return };
    let (prev, next) = (c.prev_ready(i), a.next_after(i));
    let prev_title = prev.and_then(|x| c.step_at(x)).map(|x| String::from(x.title.get(en)));
    let next_title = next.and_then(|x| c.step_at(x)).map(|x| String::from(x.title.get(en)));
    let p = &a.quizzes[k];
    let running = p.exam && p.at.as_ref().is_some_and(|x| x.end.is_none());
    let unanswered = match (&p.quiz, &p.at) {
        (Some(q), Some(x)) => x.unanswered(q),
        _ => 0,
    };
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_LAYER | pad(8), 8);
    if running {
        ui.next_size(-1, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, t("Ein begonnener Versuch wird nicht gespeichert: Nach einem Neustart des Programms beginnt er von vorn."));
        ui.next_icon(IC_UPLOAD);
        if ui.accent_button(ID_QZ_HAND, t("Abgeben")) & EV_ACT != 0 {
            n.academy.quizzes[k].ask.set(1);
            ui.repass();
        }
        if unanswered > 0 {
            ui.tooltip(&tf("Noch ohne Antwort: {}", &[&unanswered.to_string()]));
        }
        ui.box_end();
        return;
    }
    if let Some(title) = &prev_title {
        ui.next_icon(IC_BACK);
        if ui.button(ID_AK_PREV, t("Vorheriger Schritt")) & EV_ACT != 0 {
            ui.cmd_run(CMD_AK_PREV, 0);
        }
        ui.tooltip(title);
    }
    ui.spacer(-1);
    ui.next_icon(IC_FORWARD);
    if ui.accent_button(ID_AK_NEXT, if next_title.is_some() { t("Weiter") } else { t("Zur Kursübersicht") }) & EV_ACT != 0 {
        ui.cmd_run(CMD_AK_NEXT, 0);
    }
    if let Some(title) = &next_title {
        ui.tooltip(title);
    }
    ui.box_end();
}

// the card of the figures: a line, a bar
fn score_card(ui: &mut Ui, line: &str, got: u64, total: u64) {
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V | BOX_CARD | pad(12), 8);
    ui.next_size(-1, 0);
    ui.text(TS_BODY_STRONG | TX_WRAP, line);
    ui.next_size(-1, 0);
    ui.progress(ID_QZ_PROGRESS, (got / 100).min(i32::MAX as u64) as i32, (total / 100).clamp(1, i32::MAX as u64) as i32);
    ui.box_end();
}

// a line with a mark in front
fn marked_line(ui: &mut Ui, cp: u32, tok: u32, ts: u32, text: &str) {
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H, 6);
    mark(ui, cp, tok, 18);
    ui.next_size(-1, 0);
    ui.text(ts | TX_WRAP, text);
    ui.box_end();
}

// a short document at the full width of its column; the link it activated; its pictures from the course's
// folder only
fn doc(n: &mut Net, ui: &mut Ui, k: usize, q: usize, stem: bool, id: u32, text: &str) -> Option<String> {
    let w = ui.box_rect()[2];
    let a = &mut n.academy;
    let dir = a.entries.get(a.quizzes[k].course).map_or(String::new(), |e| e.dir.clone());
    let p = &mut a.quizzes[k];
    let md = if stem { &mut p.stems[q] } else { &mut p.notes[q] };
    // (the window's language may have changed)
    md.label = tf(if stem { "Frage {}" } else { "Erklärung zu Frage {}" }, &[&(q + 1).to_string()]);
    let h = ui.markdown_fit(md, text, w, MV_NO_FRAME);
    ui.next_size(-1, h);
    let ev = ui.markdown_view(id, md, text, MV_NO_FRAME);
    let asked = md.image_requests();
    if !asked.is_empty() {
        for addr in &asked {
            let data = store::image(&dir, addr);
            md.set_image(addr, data.as_deref());
        }
        ui.repass();
    }
    ev.link
}

// the head of a question: its number, what it is worth, its stem
fn head(n: &mut Net, ui: &mut Ui, k: usize, q: usize, place: usize, x: &Question, en: bool) -> Option<String> {
    let count = n.academy.quizzes[k].stems.len();
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY, &tf("Frage {} von {}", &[&(place + 1).to_string(), &count.to_string()]));
    ui.text(TS_CAPTION | TX_SECONDARY, &points_of(x.worth() as u64 * MILLI as u64));
    ui.box_end();
    doc(n, ui, k, q, true, stem_id(q, SLOT_STEM), x.stem.get(en))
}

// the widgets of a question: radios, check boxes, a text box, the entries with up and down
fn inputs(n: &mut Net, ui: &mut Ui, k: usize, q: usize, x: &Question, en: bool) {
    let Some(kind) = x.kind_of() else { return };
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, hint_of(kind));
    let p = &mut n.academy.quizzes[k];
    let Some(at) = p.at.as_mut() else { return };
    let shown: Vec<usize> = at.plan.answers.get(q).cloned().unwrap_or_default();
    let r = &mut at.replies[q];
    let before = r.clone();
    match kind {
        QKind::Single => {
            for &a in &shown {
                ui.next_size(-1, 0);
                ui.radio_wrap(stem_id(q, SLOT_ANSWER + a as u32), x.answers[a].text.get(en), &mut r.choice, a as i32);
            }
        }
        QKind::Multi => {
            for &a in &shown {
                ui.next_size(-1, 0);
                ui.checkbox_wrap(stem_id(q, SLOT_ANSWER + a as u32), x.answers[a].text.get(en), &mut r.marks[a]);
            }
        }
        QKind::Text | QKind::Number => {
            ui.box_begin(0, BOX_H, 0);
            ui.next_size(280, 0);
            let hint = if kind == QKind::Number { t("Zahl") } else { t("Ihre Antwort") };
            ui.textbox(stem_id(q, SLOT_INPUT), Some(hint), &mut r.text, REPLY_MAX);
            ui.box_end();
        }
        QKind::Order => {
            let mut moved = None;
            let rows = r.order.len();
            for (row, &e) in r.order.iter().enumerate() {
                let text = x.order.get(e).map_or("", |o| o.get(en));
                ui.next_size(-1, 0);
                ui.box_begin(0, BOX_H | BOX_XCENTER, 4);
                ui.text(TS_BODY | TX_SECONDARY, &format!("{}.", row + 1));
                ui.next_size(-1, 0);
                ui.text(TS_BODY | TX_WRAP, text);
                ui.next_flags(if row == 0 { ST_DISABLED } else { 0 });
                if ui.icon_button(stem_id(q, SLOT_ANSWER + 2 * row as u32), IC_UP, &tf("„{}“ nach oben", &[text])) & EV_ACT != 0 {
                    moved = Some((row, row - 1, row == 1));
                }
                ui.next_flags(if row + 1 == rows { ST_DISABLED } else { 0 });
                if ui.icon_button(stem_id(q, SLOT_ANSWER + 2 * row as u32 + 1), IC_DOWN, &tf("„{}“ nach unten", &[text])) & EV_ACT != 0 {
                    moved = Some((row, row + 1, row + 2 == rows));
                }
                ui.box_end();
            }
            if let Some((from, to, edge)) = moved {
                r.order.swap(from, to);
                // the keyboard stays with the entry; at the end of the list on the button that still works
                let up = (to < from) != edge;
                n.academy.focus = stem_id(q, SLOT_ANSWER + 2 * to as u32 + if up { 0 } else { 1 });
                ui.repass();
                let p = &mut n.academy.quizzes[k];
                if let Some(c) = p.checked.get_mut(q) {
                    *c = None;
                }
                return;
            }
        }
    }
    if *r != before
        && let Some(c) = p.checked.get_mut(q)
    {
        // what it said no longer goes for this reply
        *c = None;
    }
}

// ---------------------------------------------------------------- practice

fn practice(n: &mut Net, ui: &mut Ui, k: usize, en: bool) -> Option<String> {
    let p = &n.academy.quizzes[k];
    let (q, at) = (p.quiz.clone()?, p.at.as_ref()?);
    let order = at.plan.questions.clone();
    let got: u64 = p.best.iter().map(|&b| b as u64).sum();
    let total = q.total();
    let percent = (got * 100 / total.max(1)) as u32;
    let line = if percent >= q.pass_percent() {
        tf("{} von {} Punkten · {} % · bestanden", &[&milli_text(got), &milli_text(total), &percent.to_string()])
    } else {
        tf("{} von {} Punkten · {} %, bestanden ab {} %", &[&milli_text(got), &milli_text(total), &percent.to_string(), &q.pass_percent().to_string()])
    };
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Üben: Prüfen Sie jede Frage, so oft Sie wollen. Die Erklärung kommt gleich dazu."));
    score_card(ui, &line, got, total);
    let mut link = None;
    let mut act = None;
    for (place, &i) in order.iter().enumerate() {
        let x = &q.questions[i];
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V | BOX_CARD | pad(12), 8);
        if let Some(l) = head(n, ui, k, i, place, x, en) {
            link = Some(l);
        }
        inputs(n, ui, k, i, x, en);
        let p = &n.academy.quizzes[k];
        let (answered, seen) = match (&p.at, p.checked.get(i)) {
            (Some(at), Some(c)) => (x.answered(&at.replies[i]), *c),
            _ => (false, None),
        };
        ui.box_begin(0, BOX_H, 0);
        ui.next_icon(IC_ACCEPT);
        ui.next_flags(if answered { 0 } else { ST_DISABLED });
        if ui.button(stem_id(i, SLOT_CHECK), t("Prüfen")) & EV_ACT != 0 {
            act = Some(i);
        }
        ui.box_end();
        if let Some(m) = seen {
            if let Some(l) = verdict(n, ui, k, i, x, &m, en) {
                link = Some(l);
            }
        }
        ui.box_end();
    }
    ui.box_begin(0, BOX_H, 0);
    ui.next_flags(ST_SUBTLE);
    ui.next_icon(IC_REFRESH);
    if ui.button(ID_QZ_AGAIN, t("Alle Fragen von vorn")) & EV_ACT != 0 {
        ui.cmd_run(CMD_QZ_AGAIN, 0);
    }
    ui.box_end();
    if let Some(i) = act {
        let p = &mut n.academy.quizzes[k];
        let m = p.at.as_ref().map(|at| q.questions[i].grade(&at.replies[i]));
        if let Some(m) = m {
            p.checked[i] = Some(m);
            p.best[i] = p.best[i].max(m.got);
            practised(n, k);
        }
        ui.repass();
    }
    link
}

// what Prüfen found: right or not, the points, the explanations of the answers chosen, the question's
fn verdict(n: &mut Net, ui: &mut Ui, k: usize, i: usize, x: &Question, m: &Marked, en: bool) -> Option<String> {
    let (cp, tok, text) = if m.right {
        (IC_CHECK_CIRCLE_FILL, T_OK, String::from(t("Richtig.")))
    } else if m.got > 0 {
        (IC_WARNING, T_WARN, tf("Teilweise richtig: {} von {} Punkten.", &[&milli_text(m.got as u64), &milli_text(m.max as u64)]))
    } else {
        (IC_CANCEL_CIRCLE, T_CRIT, String::from(t("Noch nicht richtig.")))
    };
    marked_line(ui, cp, tok, TS_BODY_STRONG, &text);
    let reply = n.academy.quizzes[k].at.as_ref().map(|at| at.replies[i].clone()).unwrap_or_default();
    for (a, y) in x.answers.iter().enumerate().filter(|(_, y)| !y.explain.is_empty()) {
        if reply.choice == a as i32 || reply.marks.get(a).is_some_and(|&v| v != 0) {
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, &format!("{} → {}", y.text.get(en), y.explain.get(en)));
        }
    }
    explanation(n, ui, k, i, x, en)
}

// the question's explanation
fn explanation(n: &mut Net, ui: &mut Ui, k: usize, i: usize, x: &Question, en: bool) -> Option<String> {
    if x.explain.is_empty() {
        return None;
    }
    doc(n, ui, k, i, false, stem_id(i, SLOT_EXPLAIN), x.explain.get(en))
}

// ---------------------------------------------------------------- a test

// before an attempt: what it asks, how it went so far, Test beginnen
fn ready(n: &mut Net, ui: &mut Ui, k: usize) {
    let a = &n.academy;
    let p = &a.quizzes[k];
    let Some(q) = &p.quiz else { return };
    let used = a.tries_at(p.course, p.step);
    let best = a.best_at(p.course, p.step);
    let can = q.can_start(used);
    let time = if q.minutes > 0 { tf("{} Min.", &[&q.minutes.to_string()]) } else { String::from(t("keine Zeitgrenze")) };
    let tries = match q.attempts_left(used) {
        Some(l) => tf("noch {} von {} Versuchen", &[&l.to_string(), &q.attempts.to_string()]),
        None => String::from(t("beliebig viele Versuche")),
    };
    let facts = format!("{} · {} · {} · {}", tf("{} Fragen", &[&q.questions.len().to_string()]), points_of(q.total()), time, tries);
    let pass = tf("Bestanden ab {} %.", &[&q.pass_percent().to_string()]);
    let best_line = best.map(|b| tf("Bestes Ergebnis bisher: {} %.", &[&b.to_string()]));
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V | BOX_CARD | pad(12), 8);
    ui.next_size(-1, 0);
    ui.text(TS_BODY_STRONG | TX_WRAP, &facts);
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_WRAP, &pass);
    if let Some(l) = &best_line {
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_WRAP, l);
    }
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Sie geben einmal ab; danach zeigt der Test Punkte und Erklärungen. Die Zeit läuft, sobald Sie beginnen, auch auf anderen Seiten."));
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Ein begonnener Versuch wird nicht gespeichert: Nach einem Neustart des Programms beginnt er von vorn."));
    ui.box_begin(0, BOX_H, 0);
    ui.next_icon(IC_FORWARD);
    ui.next_flags(if can { 0 } else { ST_DISABLED });
    if ui.accent_button(ID_QZ_START, t("Test beginnen")) & EV_ACT != 0 {
        ui.cmd_run(CMD_QZ_START, 0);
    }
    ui.box_end();
    if !can {
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_WRAP, t("Alle Versuche sind verbraucht. Fortschritt zurücksetzen in der Kursübersicht gibt sie zurück."));
    }
    ui.box_end();
}

// the questions of an attempt
fn questions(n: &mut Net, ui: &mut Ui, k: usize, en: bool) -> Option<String> {
    let p = &n.academy.quizzes[k];
    let (q, at) = (p.quiz.clone()?, p.at.as_ref()?);
    let order = at.plan.questions.clone();
    let mut link = None;
    for (place, &i) in order.iter().enumerate() {
        let x = &q.questions[i];
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V | BOX_CARD | pad(12), 8);
        if let Some(l) = head(n, ui, k, i, place, x, en) {
            link = Some(l);
        }
        inputs(n, ui, k, i, x, en);
        ui.box_end();
    }
    link
}

// the result of an attempt: pass or not, the points per question, what was right, every explanation
fn result(n: &mut Net, ui: &mut Ui, k: usize, en: bool) -> Option<String> {
    let a = &n.academy;
    let p = &a.quizzes[k];
    let (q, at) = (p.quiz.clone()?, p.at.clone()?);
    let e = at.end.clone()?;
    let used = a.tries_at(p.course, p.step);
    let best = a.best_at(p.course, p.step);
    let line = tf("{} von {} Punkten · {} %", &[&milli_text(e.score.got), &milli_text(e.score.total), &e.score.percent().to_string()]);
    if e.timeout {
        marked_line(ui, IC_RECENT, T_TEXT2, TS_BODY_STRONG, t("Die Zeit ist um: abgegeben wurde, was bis dahin da stand."));
    }
    let pass = tf("Bestanden ab {} %.", &[&q.pass_percent().to_string()]);
    if e.passed {
        marked_line(ui, IC_CHECK_CIRCLE_FILL, T_OK, TS_SUBTITLE, t("Bestanden"));
    } else {
        marked_line(ui, IC_CANCEL_CIRCLE, T_CRIT, TS_SUBTITLE, t("Nicht bestanden"));
    }
    score_card(ui, &line, e.score.got, e.score.total);
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, &pass);
    if let Some(b) = best {
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, &tf("Bestes Ergebnis bisher: {} %.", &[&b.to_string()]));
    }
    if q.can_start(used) {
        ui.box_begin(0, BOX_H, 0);
        ui.next_icon(IC_REFRESH);
        if ui.button(ID_QZ_START, t("Neuer Versuch")) & EV_ACT != 0 {
            ui.cmd_run(CMD_QZ_START, 0);
        }
        ui.box_end();
    } else {
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_WRAP, t("Alle Versuche sind verbraucht. Fortschritt zurücksetzen in der Kursübersicht gibt sie zurück."));
    }
    let mut link = None;
    for (place, &i) in at.plan.questions.iter().enumerate() {
        let x = &q.questions[i];
        let m = e.score.marks.get(i).copied().unwrap_or_default();
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V | BOX_CARD | pad(12), 8);
        if let Some(l) = head(n, ui, k, i, place, x, en) {
            link = Some(l);
        }
        let (cp, tok) = if m.right {
            (IC_CHECK_CIRCLE_FILL, T_OK)
        } else if m.got > 0 {
            (IC_WARNING, T_WARN)
        } else {
            (IC_CANCEL_CIRCLE, T_CRIT)
        };
        marked_line(ui, cp, tok, TS_BODY_STRONG, &tf("{} von {} Punkten", &[&milli_text(m.got as u64), &milli_text(m.max as u64)]));
        solution(ui, x, &at.replies[i], &at.plan.answers[i], en);
        if let Some(l) = explanation(n, ui, k, i, x, en) {
            link = Some(l);
        }
        ui.box_end();
    }
    link
}

// what was given and what was right, now that it is handed in
fn solution(ui: &mut Ui, x: &Question, r: &Reply, shown: &[usize], en: bool) {
    match x.kind_of() {
        Some(QKind::Single | QKind::Multi) => {
            for &a in shown {
                let y = &x.answers[a];
                let chosen = r.choice == a as i32 || r.marks.get(a).is_some_and(|&v| v != 0);
                let (cp, tok) = match (y.right, chosen) {
                    (true, _) => (IC_CHECK_CIRCLE_FILL, T_OK),
                    (false, true) => (IC_CANCEL_CIRCLE, T_CRIT),
                    (false, false) => (IC_CIRCLE_RING, T_TEXT3),
                };
                let text = if chosen { tf("{} (Ihre Wahl)", &[y.text.get(en)]) } else { String::from(y.text.get(en)) };
                marked_line(ui, cp, tok, TS_BODY, &text);
                if chosen && !y.explain.is_empty() {
                    ui.next_size(-1, 0);
                    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, y.explain.get(en));
                }
            }
        }
        Some(QKind::Text) => {
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_WRAP, &if r.text.trim().is_empty() { String::from(t("Keine Antwort.")) } else { tf("Ihre Antwort: {}", &[&r.text]) });
        }
        Some(QKind::Number) => {
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_WRAP, &if r.text.trim().is_empty() { String::from(t("Keine Antwort.")) } else { tf("Ihre Antwort: {}", &[&r.text]) });
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_WRAP, &tf("Richtig ist: {}", &[&x.value_text()]));
        }
        Some(QKind::Order) => {
            let line = |v: &[usize]| v.iter().filter_map(|&e| x.order.get(e)).map(Text::clone).map(|o| String::from(o.get(en))).collect::<Vec<_>>().join(" → ");
            ui.next_size(-1, 0);
            ui.text(TS_BODY | TX_WRAP, &tf("Ihre Reihenfolge: {}", &[&line(&r.order)]));
            ui.next_size(-1, 0);
            let right: Vec<usize> = (0..x.order.len()).collect();
            ui.text(TS_BODY | TX_WRAP, &tf("Richtig ist: {}", &[&line(&right)]));
        }
        None => {}
    }
}

/// "Jetzt abgeben?"
pub(crate) fn dialog(n: &mut Net, ui: &mut Ui) {
    let open = n.academy.ask_quit.clone();
    if ui.dialog_begin(ID_AI_QUIT_DLG, t("Programm wirklich beenden?"), &open) {
        ui.next_size(400, 0);
        ui.text(TS_BODY | TX_WRAP, t("Ein Test läuft noch. Seine Antworten gehen beim Beenden verloren, und der Versuch zählt nicht."));
        if ui.dialog_buttons(Some(t("Beenden")), None, Some(t("Weiterarbeiten"))) == DLG_PRIMARY {
            n.academy.quit_ok = true;
            ui.cmd_run(CMD_QUIT, 0);
        }
    }
    ui.dialog_end();
    let Some(k) = shown(n) else { return };
    let ask = n.academy.quizzes[k].ask.clone();
    if ui.dialog_begin(ID_QZ_ASK, t("Test abgeben?"), &ask) {
        let p = &n.academy.quizzes[k];
        let open = match (&p.quiz, &p.at) {
            (Some(q), Some(x)) => x.unanswered(q),
            _ => 0,
        };
        ui.next_size(380, 0);
        ui.text(TS_BODY | TX_WRAP, t("Dieser Versuch wird dann bewertet und lässt sich nicht mehr ändern."));
        if open > 0 {
            ui.next_size(380, 0);
            ui.text(TS_BODY_STRONG | TX_WRAP, &tf("Noch ohne Antwort: {}", &[&open.to_string()]));
        }
        if ui.dialog_buttons(Some(t("Abgeben")), None, Some(t("Abbrechen"))) == DLG_PRIMARY {
            ui.cmd_run(CMD_QZ_HAND_IN, 0);
        }
    }
    ui.dialog_end();
}
