// academy -- the Akademie: courses from files (jfn_model::academy), their outline, lessons to read, labs
// in the simulator, what the learner has done
//
//   Academy::new() / setup(headless, dir)    where the files are (store.rs, --academy-dir PATH)
//   academy.ensure()                         the courses and the progress, read once
//   academy.cards() -> Vec<Card>             the courses for the start page and the list of courses
//   academy.loaded()                         a new or loaded project (Net::after_load): the page goes, the
//                                            project below it is no course's lab any more
//   command(n, ui, id, arg)                  CMD_ACADEMY .. CMD_AK_RESET
//   back(n, ui)                              Esc and Alt+Left while the page shows: lesson, course, the
//                                            list of courses, then what lay below the page
//   open_lab(n, ui, course, step)            After::CourseLab: a lab step's file as a project without a
//                                            file (doc/file.rs open_lab), named as its file until it is saved
//   follow_link(n, ui, target)               a link of a lesson: step:ID and lab:ID; else a message
//   tick(n)                                  every frame: a course's lab with every goal reached is done
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
// For AK2: quizzes, fault hunts and exams are steps the format knows (StepKind) and the outline lists as
// not available; open_step gets a page for each, Progress's `best` takes their results.  For AK3: installed
// courses appear in the folder store.rs lists: list it again (Academy::read back to false), and the list of
// courses gets its "Jetzt installieren"; a lesson's web links come to follow_link (they are refused here).

pub(crate) mod pages;
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
        }
    }

    /// Net::init: where the files are
    pub(crate) fn setup(&mut self, headless: bool, dir: Option<String>) {
        self.place = store::place(headless, dir);
    }

    /// the courses and the progress, once
    pub(crate) fn ensure(&mut self) {
        if core::mem::replace(&mut self.read, true) {
            return;
        }
        self.entries = self.place.courses.as_deref().map(store::list).unwrap_or_default();
        self.progress = store::read_progress(&self.place.progress);
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
    format!("{} · {}", level_name(c.level), tf("{} Min.", &[&c.minutes.to_string()]))
}

// the list of courses
fn home(n: &mut Net) {
    n.academy.ensure();
    let a = &mut n.academy;
    a.on = true;
    a.view = View::Home;
    a.via_home = true;
    a.focus = if a.entries.is_empty() { ID_AK_BACK } else { ID_AK_CARD };
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

// the step at place i of the current course: a lesson on the page; a lab as the project, unless it is the
// project already (the page then only moves away); what the program cannot open yet stays where it is
fn open_step(n: &mut Net, ui: &mut Ui, i: usize) {
    let Some(s) = n.academy.course().and_then(|c| c.step_at(i)) else { return };
    let (kind, step_id) = (s.kind(), s.id.clone());
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
        Some(StepKind::Lab) => {
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
        _ => {}
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
    }
}

/// Esc, Alt+Left, the page's back button: one level up, at last what lay below the page
pub(crate) fn back(n: &mut Net, ui: &mut Ui) {
    n.file.msg = None;
    let a = &mut n.academy;
    match a.view {
        View::Lesson(i) => {
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

/// a link of a lesson: step:ID and lab:ID stay in the course; anything else is refused for now (AK3 asks
/// about the web)
pub(crate) fn follow_link(n: &mut Net, ui: &mut Ui, target: &str) {
    let found = n.academy.course().and_then(|c| match link_to(target) {
        LinkTo::Step(id) => c.index_of(id),
        LinkTo::Lab(id) => c.index_of(id).filter(|&i| c.step_at(i).and_then(|s| s.kind()) == Some(StepKind::Lab)),
        _ => None,
    });
    match found {
        Some(i) => ui.cmd_run(CMD_AK_STEP, i as u32),
        None => {
            n.file.msg = Some((IB_WARNING, tf("Der Link „{}“ geht nicht auf", &[target]), String::from(t("Er führt zu keinem Schritt dieses Kurses. Links ins Web gehen hier noch nicht auf."))));
            ui.repass();
        }
    }
}

// the learner's file could not be written: said once per attempt, what was done still counts in memory
fn saved(n: &mut Net, ok: bool) {
    if !ok {
        n.file.msg = Some((IB_WARNING, String::from(t("Der Fortschritt wurde nicht gespeichert")), String::from(t("Die Datei ließ sich nicht schreiben."))));
    }
}

/// every frame: a course's lab with every goal reached is done
pub(crate) fn tick(n: &mut Net) {
    let Some(lab) = n.academy.lab.as_mut() else { return };
    if lab.marked || !n.task.run.as_ref().is_some_and(|r| r.done()) {
        return;
    }
    lab.marked = true;
    let (c, s) = (lab.course, lab.step);
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
            let View::Lesson(i) = n.academy.view else { return true };
            if let Some(k) = n.academy.course().and_then(|c| c.prev_ready(i)) {
                open_step(n, ui, k);
            }
        }
        // Weiter: this lesson is done; after the last step the outline shows what is left
        CMD_AK_NEXT => {
            let View::Lesson(i) = n.academy.view else { return true };
            let cur = n.academy.cur;
            let ok = n.academy.mark_done(cur, i);
            let next = n.academy.course().and_then(|c| c.next_ready(i));
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
            let next = n.academy.course().and_then(|c| c.next_ready(l.step));
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
            n.academy.focus = ID_AK_CONTINUE;
        }
        _ => return false,
    }
    ui.repass();
    true
}
