// academy -- the Akademie: courses from files (jfn_model::academy), their outline, lessons to read, labs
// in the simulator, quizzes and tests (quiz.rs), fault hunts, what the learner has done
//
//   Academy::new() / setup(headless, dir)    where the files are (store.rs, --academy-dir PATH)
//   academy.ensure()                         the courses and the progress, read once
//   academy.cards() -> Vec<Card>             the courses for the start page and the list of courses
//   academy.loaded()                         a new or loaded project (Net::after_load): the page goes, the
//                                            project below it is no course's lab any more
//   command(n, ui, id, arg)                  CMD_ACADEMY .. CMD_AK_RESET
//   academy.can_install()                    there is a folder to install into (a run without a window has
//                                            none unless --academy-dir names one)
//   academy.relist(&changed) -> bool         the courses again after these changed (install.rs); true: the page
//                                            moved away from one of them
//   back(n, ui)                              Esc and Alt+Left while the page shows: lesson, course, the
//                                            list of courses, then what lay below the page
//   open_lab(n, ui, course, step)            After::CourseLab: a lab step's file as a project without a
//                                            file (doc/file.rs open_lab), named as its file until it is saved
//   follow_link(n, ui, target)               a link of a lesson: step:ID and lab:ID; the web after a question
//                                            that shows the address; else a message
//   tick(n, ui)                              every frame: a course's lab with every goal reached is done; a
//                                            fault hunt handed in has its result; a test out of time ends
//   frame(n, ui) / bar(n, ui) / dialog(n, ui)   pages.rs: the page, the bar over the canvas, the question
//                                            before resetting
//
// The page lies above the start page and the project and takes the place of toolbar, dock and status bar
// (shell.rs); dialogs and timers run on.  Below it nothing changes: a lab stays as it is while the learner
// reads, "Zurück zum Kurs" and opening the same step again only move the page, and only another lab asks
// about unsaved changes (After::CourseLab) and replaces the project.  A lesson is done when the learner
// goes on from it with Weiter; a lab when all its goals are reached, and going on without that leaves it
// open.  Progress is written at once to its own file (store.rs), never into the project and without an
// undo step; it goes by ids, so a new version of a course keeps it.
//
// A quiz or a test is a page of its own (quiz.rs); what this session knows of it stays while other pages show.
// A fault hunt is a lab whose project is broken (Lab::start_faults, Lab::lock) and that grades: handing it
// in (assess.rs) gives its result, it is done when it reaches the lab's `pass`, and Neu beginnen on the bar
// loads it from its file again.  Results go to Progress.best.  A step with `requires` stays locked until those
// are done, Weiter goes past it.  How courses come and go is install.rs.
//
// A course can change while it is open, because a worker runs on while the learner reads.  Files are read whole and
// nothing stays open, so renaming a course's folder takes nothing away from under a page.  Then Academy::relist
// goes by ids, because everything here holds places in the list: a lesson of a course that changed goes back to its
// outline with a note (the text it showed may no longer exist), of a course that went to the list of courses; a
// page of another course stays where it is.  A lab of a course that changed stays the project it is, in memory; it
// keeps its bar while the step still exists, else it is a plain project without one.  Quizzes and tests are never
// taken away: while a quiz's page shows or a test runs the list waits (install.rs), and only then
// Academy::quizzes is cleared, since its pages hold numbers of the old list.

pub(crate) mod install;
pub(crate) mod pages;
pub(crate) mod quiz;
pub(crate) mod store;

pub(crate) use pages::{bar, dialog, frame};

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::academy::{link_to, Course, Level, LinkTo, Progress, StepKind};
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{self, t, tf};
use crate::app::Net;
use crate::doc::file::{self, After};
use store::{Entry, Place};

/// what the page shows
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum View {
    /// the list of courses
    Home,
    /// the outline of the current course
    Course,
    /// the lesson at this place of the current course
    Lesson(usize),
    /// the quiz or test at this place of the current course
    Quiz(usize),
}

/// the project below the page is this lab step of a course
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LabStep {
    pub course: usize,
    pub step: usize,
    /// its goals were all reached once since it was opened
    pub marked: bool,
}

/// a course as a card
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Card {
    pub title: String,
    pub text: String,
    pub badge: Option<(u32, String)>,
}

/// the state of the Akademie
pub(crate) struct Academy {
    /// the page shows
    pub on: bool,
    pub view: View,
    /// back from the outline goes to the list of courses
    pub via_home: bool,
    place: Place,
    read: bool,
    pub entries: Vec<Entry>,
    pub progress: Progress,
    /// the current course in `entries`
    pub cur: usize,
    /// the lesson: its view, its text (None: no language has it), the language it is in, the window's
    /// language it was read for
    pub md: MdViewState,
    pub text: Option<String>,
    pub text_lang: &'static str,
    pub text_en: bool,
    pub lab: Option<LabStep>,
    /// the question before resetting
    pub ask_reset: Shared<i32>,
    /// the widget that takes the keyboard in the next frame (0: none); without its ring (the document, the
    /// lab's panel: a ring only follows keyboard navigation there)
    pub focus: u32,
    pub focus_quiet: bool,
    /// the note about the other language was closed
    pub note_closed: bool,
    /// the quizzes and tests this session opened; a run without a window (their fixed order); a test's
    /// countdown has its timer
    pub quizzes: Vec<quiz::Page>,
    pub headless: bool,
    pub timer_armed: bool,
    /// the lists of what can be installed, a worker, the questions (install.rs)
    pub inst: install::Inst,
    /// the question before leaving with a test under way, its answer (quiz.rs)
    pub ask_quit: Shared<i32>,
    pub quit_ok: bool,
}

impl Academy {
    pub(crate) fn new() -> Academy {
        Academy {
            on: false,
            view: View::Home,
            via_home: false,
            place: store::place(true, None),
            read: false,
            entries: Vec::new(),
            progress: Progress::default(),
            cur: 0,
            md: MdViewState::new(),
            text: None,
            text_lang: "de",
            text_en: false,
            lab: None,
            ask_reset: Shared::new(0),
            focus: 0,
            focus_quiet: false,
            note_closed: false,
            quizzes: Vec::new(),
            headless: true,
            timer_armed: false,
            inst: install::Inst::new(),
            ask_quit: Shared::new(0),
            quit_ok: false,
        }
    }

    /// Net::init: where the files are
    pub(crate) fn setup(&mut self, headless: bool, dir: Option<String>) {
        self.place = store::place(headless, dir);
        self.headless = headless;
        self.inst.setup(headless);
    }

    /// the courses and the progress, once
    pub(crate) fn ensure(&mut self) {
        if core::mem::replace(&mut self.read, true) {
            return;
        }
        install::read_last(self);
        self.entries = self.place.courses.as_deref().map(store::list).unwrap_or_default();
        self.progress = store::read_progress(&self.place.progress);
    }

    /// there is a folder to install into
    pub(crate) fn can_install(&self) -> bool {
        self.place.courses.is_some()
    }

    /// the courses again after these changed: everything that holds a place in the list follows by id (the top
    /// of this file) -> the page moved away from a course that changed
    pub(crate) fn relist(&mut self, changed: &[String]) -> bool {
        let cur = self.entries.get(self.cur).map(|e| e.id.clone());
        let lab = self.lab.as_ref().and_then(|l| self.ids(l.course, l.step).map(|ids| (ids, l.marked)));
        self.entries = self.place.courses.as_deref().map(store::list).unwrap_or_default();
        self.quizzes.clear();
        let place_of = |id: &str| self.entries.iter().position(|e| e.id == id);
        let found = cur.as_deref().and_then(place_of);
        let touched = cur.as_deref().is_some_and(|c| changed.iter().any(|x| x == c));
        self.lab = lab.and_then(|((c, s), marked)| {
            let course = place_of(&c)?;
            let step = self.entries[course].usable()?.index_of(&s)?;
            Some(LabStep { course, step, marked })
        });
        self.cur = found.unwrap_or(0);
        let mut moved = false;
        match self.view {
            View::Lesson(_) | View::Quiz(_) if touched || found.is_none() => {
                moved = self.on;
                self.view = if found.is_some() { View::Course } else { View::Home };
            }
            View::Course if found.is_none() => {
                moved = self.on;
                self.view = View::Home;
            }
            View::Home | View::Course | View::Lesson(_) | View::Quiz(_) => {}
        }
        if self.entries.is_empty() {
            self.cur = 0;
        }
        moved
    }

    /// a new or loaded project
    pub(crate) fn loaded(&mut self) {
        self.on = false;
        self.lab = None;
    }

    /// the current course, if it can be offered
    pub(crate) fn course(&self) -> Option<&Course> {
        self.entries.get(self.cur).and_then(Entry::usable)
    }

    /// the id of the current course
    pub(crate) fn course_id(&self) -> &str {
        self.entries.get(self.cur).map_or("", |e| e.id.as_str())
    }

    /// the courses as cards: title, what it is, how far the learner is
    pub(crate) fn cards(&self) -> Vec<Card> {
        let en = tr::en();
        self.entries
            .iter()
            .map(|e| {
                let Some(c) = e.usable() else {
                    return Card { title: String::from(e.title(en)), text: String::from(t("Dieser Kurs lässt sich nicht öffnen.")), badge: Some((BADGE_WARN, String::from(t("Fehlerhaft")))) };
                };
                let (done, all) = (self.progress.done_count(c, &e.id), c.ready_count());
                let badge = if done == 0 {
                    None
                } else if done >= all {
                    Some((BADGE_OK, String::from(t("Abgeschlossen"))))
                } else {
                    Some((BADGE_INFO, tf("{} von {}", &[&done.to_string(), &all.to_string()])))
                };
                Card { title: String::from(c.title.get(en)), text: format!("{} · {}", meta_text(c), c.summary.get(en)), badge }
            })
            .collect()
    }

    // the next step after place i of the current course that can be opened now
    pub(crate) fn next_after(&self, i: usize) -> Option<usize> {
        let (c, id) = (self.course()?, self.course_id());
        c.steps().enumerate().skip(i + 1).find(|(_, s)| s.ready() && !self.progress.locked(c, id, s)).map(|(k, _)| k)
    }

    // the ids of a step
    fn ids(&self, course: usize, step: usize) -> Option<(String, String)> {
        let e = self.entries.get(course)?;
        let s = e.course.as_ref()?.step_at(step)?;
        Some((e.id.clone(), s.id.clone()))
    }

    /// the attempts handed in of a step
    pub(crate) fn tries_at(&self, course: usize, step: usize) -> u32 {
        self.ids(course, step).map_or(0, |(c, s)| self.progress.tries_of(&c, &s))
    }

    /// the best result of a step
    pub(crate) fn best_at(&self, course: usize, step: usize) -> Option<u32> {
        self.ids(course, step).and_then(|(c, s)| self.progress.best_of(&c, &s))
    }

    /// an attempt was handed in: its result, done when it passed; at once in its file (false: it could not
    /// be written)
    pub(crate) fn result(&mut self, course: usize, step: usize, percent: u32, passed: bool) -> bool {
        let Some((c, s)) = self.ids(course, step) else { return true };
        let when = plat::unix_time_ns() / 1_000_000_000;
        self.progress.record(&c, &s, percent, when);
        if passed {
            self.progress.mark_done(&c, &s, when);
        }
        store::write_progress(&self.place.progress, &self.progress)
    }

    /// practice reached this: kept if it is the most so far, done when it passes
    pub(crate) fn note(&mut self, course: usize, step: usize, percent: u32, passed: bool) -> bool {
        let Some((c, s)) = self.ids(course, step) else { return true };
        let mut changed = self.progress.note_best(&c, &s, percent);
        if passed {
            changed |= self.progress.mark_done(&c, &s, plat::unix_time_ns() / 1_000_000_000);
        }
        !changed || store::write_progress(&self.place.progress, &self.progress)
    }

    // a step is done: at once in its file (false: it could not be written)
    fn mark_done(&mut self, course: usize, step: usize) -> bool {
        let Some(e) = self.entries.get(course) else { return true };
        let Some(s) = e.course.as_ref().and_then(|c| c.step_at(step)) else { return true };
        let when = plat::unix_time_ns() / 1_000_000_000;
        if !self.progress.mark_done(&e.id, &s.id, when) {
            return true;
        }
        store::write_progress(&self.place.progress, &self.progress)
    }

    // the lesson at place i of the current course in the window's language; the view starts over
    fn read_lesson(&mut self, i: usize) {
        let en = tr::en();
        let Some(e) = self.entries.get(self.cur) else { return };
        let Some(s) = e.usable().and_then(|c| c.step_at(i)) else { return };
        let found = store::lesson(&e.dir, &s.file, if en { "en" } else { "de" });
        let mut md = MdViewState::new();
        md.base = format!("{}/", e.dir);
        md.images = MI_APP;
        md.label = String::from(s.title.get(en));
        self.md = md;
        self.text_en = en;
        self.note_closed = false;
        (self.text, self.text_lang) = match found {
            Some((t, l)) => (Some(t), l),
            None => (None, "de"),
        };
    }
}

/// "Einsteiger", "Fortgeschrittene", "Erfahrene"
fn level_name(l: Level) -> &'static str {
    match l {
        Level::Beginner => t("Einsteiger"),
        Level::Intermediate => t("Fortgeschrittene"),
        Level::Advanced => t("Erfahrene"),
    }
}

/// "Einsteiger · 15 Min."
pub(crate) fn meta_text(c: &Course) -> String {
    level_text(c.level, c.minutes)
}

/// "Einsteiger · 15 Min."
pub(crate) fn level_text(l: Level, minutes: u32) -> String {
    format!("{} · {}", level_name(l), tf("{} Min.", &[&minutes.to_string()]))
}

// the list of courses
fn home(n: &mut Net) {
    n.academy.ensure();
    let a = &mut n.academy;
    a.on = true;
    a.view = View::Home;
    a.via_home = true;
    a.focus = match (a.entries.is_empty(), a.can_install()) {
        (false, _) => ID_AK_CARD,
        (true, true) => ID_AI_ALL,
        (true, false) => ID_AK_BACK,
    };
}

// a course's outline (a bad one: its messages)
fn open_course(n: &mut Net, i: usize, via_home: bool) {
    n.academy.ensure();
    let a = &mut n.academy;
    if i >= a.entries.len() {
        return;
    }
    a.on = true;
    a.cur = i;
    a.view = View::Course;
    a.via_home = via_home;
    a.focus = if a.course().is_some_and(|c| a.progress.next_open(c, &a.entries[i].id).is_some()) { ID_AK_CONTINUE } else { ID_AK_BACK };
}

// the step at place i of the current course: a lesson, a quiz or a test on the page; a lab or a fault hunt as
// the project, unless it is the project already (the page then only moves away); what the program cannot open
// yet or what is still locked stays where it is
fn open_step(n: &mut Net, ui: &mut Ui, i: usize) {
    let a = &n.academy;
    let Some(c) = a.course() else { return };
    let Some(s) = c.step_at(i) else { return };
    let (kind, step_id, ready, locked) = (s.kind(), s.id.clone(), s.ready(), a.progress.locked(c, a.course_id(), s));
    if !ready {
        return;
    }
    if locked {
        n.file.msg = Some((IB_INFO, String::from(t("Dieser Schritt ist noch gesperrt")), String::from(t("Er öffnet sich, wenn die Schritte davor erledigt sind, die er verlangt."))));
        ui.repass();
        return;
    }
    match kind {
        Some(StepKind::Lesson) => {
            n.file.msg = None;
            let a = &mut n.academy;
            a.read_lesson(i);
            a.view = View::Lesson(i);
            a.on = true;
            a.focus = ID_AK_LESSON;
            a.focus_quiet = true;
        }
        Some(StepKind::Quiz | StepKind::Exam) => {
            n.file.msg = None;
            quiz::open(n, i);
        }
        Some(StepKind::Lab | StepKind::Fault) => {
            n.file.msg = None;
            let a = &mut n.academy;
            if a.lab.as_ref().is_some_and(|l| l.course == a.cur && l.step == i) {
                a.on = false;
                a.focus = ID_TASK_CHECK;
                a.focus_quiet = true;
                n.shell.dock.show(P_TASK);
            } else {
                let course = String::from(a.course_id());
                file::request(n, ui, After::CourseLab(course, step_id));
            }
        }
        None => {}
    }
}

/// After::CourseLab: the lab step's file as a project; it failing leaves the page and the info bar's message
pub(crate) fn open_lab(n: &mut Net, ui: &mut Ui, course: &str, step: &str) {
    let Some(c) = n.academy.entries.iter().position(|e| e.id == course) else { return };
    let e = &n.academy.entries[c];
    let Some(x) = e.usable() else { return };
    let Some(i) = x.index_of(step) else { return };
    let Some(s) = x.step_at(i) else { return };
    let path = format!("{}/{}", e.dir, s.file);
    if file::open_lab(n, ui, &path) {
        let a = &mut n.academy;
        a.cur = c;
        a.lab = Some(LabStep { course: c, step: i, marked: false });
        a.focus = ID_TASK_CHECK;
        a.focus_quiet = true;
        // a fault hunt is handed in, with or without a time limit
        if fault_hunt(n)
            && let Some(run) = n.task.assess.as_mut()
        {
            run.hand_in = true;
        }
    }
}

/// Esc, Alt+Left, the page's back button: one level up, at last what lay below the page
pub(crate) fn back(n: &mut Net, ui: &mut Ui) {
    n.file.msg = None;
    let a = &mut n.academy;
    match a.view {
        View::Lesson(i) | View::Quiz(i) => {
            a.view = View::Course;
            a.focus = ID_AK_STEP + i as u32;
        }
        View::Course if a.via_home => {
            a.view = View::Home;
            a.focus = ID_AK_CARD + a.cur as u32;
        }
        View::Course => {
            a.on = false;
            let card = ID_SP_COURSE + a.cur as u32;
            if n.shell.start {
                ui.focus(card);
            }
        }
        View::Home => {
            a.on = false;
            if n.shell.start {
                crate::workspace::startpage::show(n);
            }
        }
    }
    ui.repass();
}

/// a link of a lesson: step:ID and lab:ID stay in the course; the web opens after a question that shows the
/// address; anything else is refused
pub(crate) fn follow_link(n: &mut Net, ui: &mut Ui, target: &str) {
    if link_to(target) == LinkTo::Web {
        install::ask_link(n, ui, target);
        return;
    }
    let found = n.academy.course().and_then(|c| match link_to(target) {
        LinkTo::Step(id) => c.index_of(id),
        LinkTo::Lab(id) => c.index_of(id).filter(|&i| c.step_at(i).and_then(|s| s.kind()) == Some(StepKind::Lab)),
        _ => None,
    });
    match found {
        Some(i) => ui.cmd_run(CMD_AK_STEP, i as u32),
        None => {
            n.file.msg = Some((IB_WARNING, tf("Der Link „{}“ geht nicht auf", &[target]), String::from(t("Er führt zu keinem Schritt dieses Kurses."))));
            ui.repass();
        }
    }
}

// the learner's file could not be written: said once per attempt, what was done still counts in memory
pub(crate) fn saved(n: &mut Net, ok: bool) {
    if !ok {
        n.file.msg = Some((IB_WARNING, String::from(t("Der Fortschritt wurde nicht gespeichert")), String::from(t("Die Datei ließ sich nicht schreiben."))));
    }
}

// the project below the page is a fault hunt of a course
pub(crate) fn fault_hunt(n: &Net) -> bool {
    let a = &n.academy;
    a.lab.as_ref().and_then(|l| a.entries.get(l.course)?.course.as_ref()?.step_at(l.step)?.kind()) == Some(StepKind::Fault)
}

/// every frame: a course's lab with every goal reached is done; a fault hunt handed in has its result and
/// is done if it reached the lab's `pass`; a test out of time ends
pub(crate) fn tick(n: &mut Net, ui: &mut Ui) {
    quiz::tick(n, ui);
    install::tick(n, ui);
    let hunt = fault_hunt(n);
    let Some(lab) = n.academy.lab.as_mut() else { return };
    if lab.marked {
        return;
    }
    let (c, s) = (lab.course, lab.step);
    if hunt {
        let Some(percent) = n.task.assess.as_ref().filter(|r| r.end.is_some()).and_then(|r| r.score.as_ref()).map(|x| x.percent()) else { return };
        lab.marked = true;
        let pass = n.doc.proj.lab.as_ref().map_or(100, |l| l.pass_percent());
        let ok = n.academy.result(c, s, percent, percent >= pass);
        saved(n, ok);
        return;
    }
    if !n.task.run.as_ref().is_some_and(|r| r.done()) {
        return;
    }
    lab.marked = true;
    let ok = n.academy.mark_done(c, s);
    saved(n, ok);
}

/// the commands of the Akademie
pub(crate) fn command(n: &mut Net, ui: &mut Ui, id: u32, arg: u32) -> bool {
    match id {
        CMD_ACADEMY => home(n),
        CMD_SP_COURSE => open_course(n, arg as usize, false),
        CMD_AK_COURSE => open_course(n, arg as usize, true),
        CMD_AK_BACK => back(n, ui),
        CMD_AK_STEP => open_step(n, ui, arg as usize),
        CMD_AK_CONTINUE => {
            let next = n.academy.course().and_then(|c| n.academy.progress.next_open(c, n.academy.course_id()));
            if let Some(i) = next {
                open_step(n, ui, i);
            }
        }
        CMD_AK_PREV => {
            let (View::Lesson(i) | View::Quiz(i)) = n.academy.view else { return true };
            if let Some(k) = n.academy.course().and_then(|c| c.prev_ready(i)) {
                open_step(n, ui, k);
            }
        }
        // Weiter: this lesson is done (a quiz or a test by its result); after the last step the outline
        // shows what is left
        CMD_AK_NEXT => {
            let (i, lesson) = match n.academy.view {
                View::Lesson(i) => (i, true),
                View::Quiz(i) => (i, false),
                _ => return true,
            };
            let cur = n.academy.cur;
            let ok = !lesson || n.academy.mark_done(cur, i);
            let next = n.academy.next_after(i);
            match next {
                Some(k) => open_step(n, ui, k),
                None => {
                    n.academy.view = View::Course;
                    n.academy.focus = ID_AK_STEP + i as u32;
                }
            }
            saved(n, ok);
        }
        CMD_AK_BAR_BACK => {
            let Some(l) = n.academy.lab.clone() else { return true };
            let a = &mut n.academy;
            a.cur = l.course;
            a.view = View::Course;
            a.via_home = false;
            a.on = true;
            a.focus = ID_AK_STEP + l.step as u32;
        }
        // going on from a lab leaves it as it is: done when its goals are, else open
        CMD_AK_BAR_NEXT => {
            let Some(l) = n.academy.lab.clone() else { return true };
            n.academy.cur = l.course;
            n.academy.via_home = false;
            let next = n.academy.next_after(l.step);
            match next {
                Some(k) => open_step(n, ui, k),
                None => {
                    n.academy.view = View::Course;
                    n.academy.on = true;
                    n.academy.focus = ID_AK_STEP + l.step as u32;
                }
            }
        }
        CMD_AK_RESET => {
            let a = &mut n.academy;
            let course = String::from(a.course_id());
            if a.progress.clear(&course) {
                let ok = store::write_progress(&a.place.progress, &a.progress);
                saved(n, ok);
            }
            let cur = n.academy.cur;
            quiz::drop_course(n, cur);
            n.academy.focus = ID_AK_CONTINUE;
        }
        CMD_QZ_START | CMD_QZ_HAND_IN | CMD_QZ_AGAIN => {
            quiz::command(n, id);
        }
        // a fault hunt once more: from its file, after the question about what changed in it
        CMD_AK_RETRY => {
            let Some(l) = n.academy.lab.clone() else { return true };
            let Some((c, s)) = n.academy.ids(l.course, l.step) else { return true };
            file::request(n, ui, After::CourseLab(c, s));
        }
        _ => return false,
    }
    ui.repass();
    true
}
