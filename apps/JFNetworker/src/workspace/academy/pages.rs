// pages -- the Akademie's page: the list of courses (with install.rs's parts where there is a folder to install
// into), a course's outline, a lesson; the bar over the canvas in a course's lab; the question before resetting
// the progress
//
//   frame(n, ui)     the page in the place of toolbar, dock and status bar, once per frame; it hosts the
//                    recovery bar and the files' info bar, as the start page does
//   bar(n, ui)       over the canvas while the project is a course's lab: step and course, Zurück zum
//                    Kurs, Weiter im Kurs (accented once every goal is reached, a fault hunt once it
//                    passed); Neu beginnen on a fault hunt that was handed in
//   dialog(n, ui)    "Fortschritt zurücksetzen?"
//
// Screen readers and scripts: a step is a button named by its title and its state ("Titel, erledigt"),
// the progress a progress bar, the lesson a document named by the step.  One Tab stop per step; Enter
// opens it.  A step the program cannot open yet or that is still locked is a disabled row with its reason, no
// button that does nothing; a step with a result names it.  Plain on purpose: text, one accent, one column as
// wide as reading is comfortable.  The page of a quiz or a test is quiz.rs's.

use alloc::format;
use alloc::string::{String, ToString};

use jfn_model::academy::{Step, StepKind, StepState};
use ui::*;

use crate::app::constants::*;
use crate::app::tr::{self, t, tf};
use crate::app::Net;
use crate::workspace::academy::{meta_text, View};
use crate::workspace::task::mark;

/// the column of the list and the outline at most, of a lesson at most (DIP)
const PAGE_W: i32 = 760;
const READ_W: i32 = 820;
/// the gaps: between cards, between sections
const CARD_GAP: i32 = 8;
const SECTION_GAP: i32 = 20;
/// the bar's place from which its buttons have their labels (DIP)
const BAR_W: i32 = 440;

/// the page
pub(crate) fn frame(n: &mut Net, ui: &mut Ui) {
    n.academy.ensure();
    let want = core::mem::take(&mut n.academy.focus);
    if n.autosave.offer.is_some() && want != 0 {
        // a recovery offer comes first, it must not get lost
        ui.focus(ID_RECOVER_OK);
    } else {
        take_focus(n, ui, want);
    }
    ui.focus_pane_begin(ID_AK_PANE);
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V, 0);
    crate::doc::autosave::bar(n, ui);
    crate::workspace::canvas::infobar(n, ui);
    match n.academy.view {
        View::Home => home(n, ui),
        View::Course => course(n, ui),
        View::Lesson(i) => lesson(n, ui, i),
        View::Quiz(i) => super::quiz::frame(n, ui, i),
    }
    ui.box_end();
    ui.focus_pane_end();
}

// the keyboard to the widget the page asked for
fn take_focus(n: &mut Net, ui: &mut Ui, want: u32) {
    let quiet = core::mem::take(&mut n.academy.focus_quiet);
    if want != 0 {
        if quiet {
            ui.focus_quiet(want);
        } else {
            ui.focus(want);
        }
    }
}

// the back button: where it leads
fn back_label(n: &Net) -> &'static str {
    match n.academy.view {
        View::Lesson(_) | View::Quiz(_) => t("Kursübersicht"),
        View::Course if n.academy.via_home => t("Alle Kurse"),
        _ if n.shell.start => t("Startseite"),
        _ if n.academy.lab.is_some() => t("Zurück zum Labor"),
        _ => t("Zurück zum Projekt"),
    }
}

pub(super) fn back_button(n: &Net, ui: &mut Ui) {
    ui.next_flags(ST_SUBTLE);
    ui.next_icon(IC_BACK);
    if ui.button(ID_AK_BACK, back_label(n)) & EV_ACT != 0 {
        ui.cmd_run(CMD_AK_BACK, 0);
    }
    ui.tooltip(t("Zurück (Esc, Alt+Links)"));
}

// a scrolling column for the list and the outline: closed by column_end
pub(super) fn column_begin(ui: &mut Ui, id: u32) {
    ui.next_size(-1, -1);
    ui.scroll_begin(id, SC_V | pad(24));
    let w = ui.px2dip(ui.box_rect()[2]).clamp(1, PAGE_W);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_CENTER, 0);
    ui.next_size(w, 0);
    ui.box_begin(0, BOX_V, SECTION_GAP);
}

pub(super) fn column_end(ui: &mut Ui) {
    ui.box_end();
    ui.box_end();
    ui.scroll_end();
}

// ---------------------------------------------------------------- the list of courses

fn home(n: &mut Net, ui: &mut Ui) {
    column_begin(ui, ID_AK_HOME_SCROLL);
    ui.box_begin(0, BOX_H, 0);
    back_button(n, ui);
    ui.box_end();
    ui.box_begin(0, BOX_V, 2);
    ui.text(TS_TITLE, t("Akademie"));
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Kurse zum Lesen und Üben: Lektionen erklären, Labore im Simulator prüfen, was Sie gebaut haben."));
    ui.box_end();
    let installing = n.academy.can_install();
    if installing {
        super::install::tools(n, ui);
    }
    let cards = n.academy.cards();
    if cards.is_empty() {
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V | BOX_XCENTER | pad(24), 8);
        ui.empty_state(IC_OFFICE_READ_MODE, t("Noch keine Kurse"), t("Hier erscheinen die Kurse, die in der Akademie installiert sind."));
        if installing {
            super::install::empty(n, ui);
        }
        ui.box_end();
    } else {
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V, CARD_GAP);
        if installing {
            ui.text(TS_SUBTITLE, t("Meine Kurse"));
        }
        for (i, x) in cards.iter().enumerate() {
            let badge = x.badge.as_ref().map(|(k, s)| (*k, s.as_str()));
            ui.next_size(-1, 0);
            ui.box_begin(0, BOX_V, 2);
            ui.next_size(-1, 0);
            let d = CardDesc { title: &x.title, icon: IC_OFFICE_READ_MODE, text: &x.text, badge, ..CardDesc::default() };
            if ui.action_card(ID_AK_CARD + i as u32, &tf("Kurs: {}", &[&x.title]), &d) & EV_ACT != 0 {
                ui.cmd_run(CMD_AK_COURSE, i as u32);
            }
            if installing {
                super::install::row(n, ui, i);
            }
            ui.box_end();
        }
        ui.box_end();
    }
    if installing {
        super::install::offered(n, ui);
    }
    column_end(ui);
}

// ---------------------------------------------------------------- a course's outline

// a kind's name and icon
fn kind_of(s: &Step) -> (&'static str, u32) {
    match s.kind() {
        Some(StepKind::Lesson) => (t("Lektion"), IC_PAGE),
        Some(StepKind::Lab) => (t("Labor"), IC_FLAG),
        Some(StepKind::Quiz) => (t("Quiz"), IC_HELP),
        Some(StepKind::Fault) => (t("Fehlersuche"), IC_REPAIR),
        Some(StepKind::Exam) => (t("Test"), IC_ACCEPT),
        None => ("", 0),
    }
}

fn course(n: &mut Net, ui: &mut Ui) {
    let en = tr::en();
    let a = &n.academy;
    let Some(e) = a.entries.get(a.cur) else { return };
    column_begin(ui, ID_AK_SCROLL);
    ui.box_begin(0, BOX_H, 0);
    back_button(n, ui);
    ui.box_end();
    let Some(c) = e.usable() else {
        // a bad course: what is wrong with it, precisely, and nothing else
        ui.text(TS_TITLE, e.title(en));
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Dieser Kurs lässt sich nicht öffnen. Das stimmt mit seinen Dateien nicht:"));
        for p in e.problems.iter().filter(|p| p.fatal) {
            ui.next_size(-1, 0);
            crate::workspace::error_text(ui, &tr::msg(&p.msg), 0);
        }
        column_end(ui);
        return;
    };
    ui.box_begin(0, BOX_V, 2);
    ui.next_size(-1, 0);
    ui.text(TS_TITLE | TX_WRAP, c.title.get(en));
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_WRAP, c.summary.get(en));
    ui.text(TS_CAPTION | TX_SECONDARY, &meta_text(c));
    ui.box_end();
    // how far it is, and on
    let (done, all) = (a.progress.done_count(c, &e.id), c.ready_count());
    let next = a.progress.next_open(c, &e.id);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V | BOX_CARD | pad(12), 8);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(-1, 0);
    ui.text(TS_BODY_STRONG, &tf("{} von {} Schritten erledigt", &[&done.to_string(), &all.to_string()]));
    if next.is_some() {
        ui.next_icon(IC_FORWARD);
        if ui.accent_button(ID_AK_CONTINUE, if done == 0 { t("Beginnen") } else { t("Weiter") }) & EV_ACT != 0 {
            ui.cmd_run(CMD_AK_CONTINUE, 0);
        }
    } else {
        mark(ui, IC_CHECK_CIRCLE_FILL, T_OK, 18);
        ui.text(TS_BODY, t("Kurs abgeschlossen"));
    }
    ui.box_end();
    ui.next_size(-1, 0);
    ui.progress(ID_AK_PROGRESS, done as i32, all.max(1) as i32);
    ui.box_end();
    // the units and their steps
    let states = a.progress.states(c, &e.id);
    let best = |s: &Step| a.progress.best_of(&e.id, &s.id);
    let mut k = 0usize;
    for u in &c.units {
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V, CARD_GAP);
        ui.next_size(-1, 0);
        ui.text(TS_SUBTITLE | TX_WRAP, u.title.get(en));
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V | BOX_CARD | pad(4), 2);
        for s in &u.steps {
            step_row(ui, s, k, states.get(k).copied().unwrap_or(StepState::Open), best(s), en);
            k += 1;
        }
        ui.box_end();
        ui.box_end();
    }
    if done > 0 {
        ui.box_begin(0, BOX_H, 0);
        ui.next_flags(ST_SUBTLE);
        ui.next_icon(IC_REFRESH);
        if ui.button(ID_AK_RESET, t("Fortschritt zurücksetzen")) & EV_ACT != 0 {
            n.academy.ask_reset.set(1);
            ui.repass();
        }
        ui.box_end();
    }
    column_end(ui);
}

// a step of the outline: its state as a mark, one Tab stop that opens it
fn step_row(ui: &mut Ui, s: &Step, k: usize, state: StepState, best: Option<u32>, en: bool) {
    let title = s.title.get(en);
    let (kind, icon) = kind_of(s);
    let (cp, tok, aside, mut name) = match state {
        StepState::Done => (IC_CHECK_CIRCLE_FILL, T_OK, t("Erledigt"), tf("{}, erledigt", &[title])),
        StepState::Current => (IC_CIRCLE_RING, T_ACCENT_TEXT, t("Als Nächstes"), tf("{}, als Nächstes", &[title])),
        StepState::Open => (IC_CIRCLE_RING, T_TEXT3, "", String::from(title)),
        StepState::NotYet => (IC_CIRCLE_RING, T_TEXT_DISABLED, "", tf("{}, noch nicht verfügbar", &[title])),
        StepState::Locked => (IC_CIRCLE_RING, T_TEXT_DISABLED, "", tf("{}, gesperrt", &[title])),
    };
    // the best result beside its state
    let shown = best.map(|b| tf("{} %", &[&b.to_string()]));
    if let Some(b) = best {
        name = tf("{}, bestes Ergebnis {} %", &[&name, &b.to_string()]);
    }
    let aside = match &shown {
        Some(b) if aside.is_empty() => b.clone(),
        Some(b) => format!("{} · {}", b, aside),
        None => String::from(aside),
    };
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 4);
    ui.spacer(4);
    mark(ui, cp, tok, 18);
    let mut flags = ST_SUBTLE;
    if matches!(state, StepState::NotYet | StepState::Locked) {
        flags |= ST_DISABLED;
    }
    if state == StepState::Current {
        flags |= ST_CHECKED;
    }
    let badge = match state {
        StepState::NotYet => Some((BADGE_NEUTRAL, t("Noch nicht verfügbar"))),
        StepState::Locked => Some((BADGE_NEUTRAL, t("Noch gesperrt"))),
        _ => None,
    };
    ui.next_size(-1, 0);
    ui.next_flags(flags);
    let d = CardDesc { title, icon, text: kind, one_line: true, aside: &aside, badge, ..CardDesc::default() };
    if ui.action_card(ID_AK_STEP + k as u32, &name, &d) & EV_ACT != 0 {
        ui.cmd_run(CMD_AK_STEP, k as u32);
    }
    ui.box_end();
}

// ---------------------------------------------------------------- a lesson

fn lang_name(code: &str) -> &'static str {
    if code == "en" { "English" } else { "Deutsch" }
}

fn lesson(n: &mut Net, ui: &mut Ui, i: usize) {
    let en = tr::en();
    // another language chosen meanwhile (Einstellungen): the lesson in it, from the top
    if n.academy.text_en != en {
        n.academy.read_lesson(i);
    }
    let a = &n.academy;
    let Some(c) = a.course() else { return };
    let Some(s) = c.step_at(i) else { return };
    let (prev, next) = (c.prev_ready(i), a.next_after(i));
    let total = c.steps().count();
    let prev_title = prev.and_then(|k| c.step_at(k)).map(|x| String::from(x.title.get(en)));
    let next_title = next.and_then(|k| c.step_at(k)).map(|x| String::from(x.title.get(en)));
    // the head: back, where this is
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_LAYER | pad(6), 8);
    back_button(n, ui);
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_SECONDARY | TX_ELLIPSIS, &format!("{} › {}", c.title.get(en), s.title.get(en)));
    ui.text(TS_CAPTION | TX_SECONDARY, &tf("Schritt {} von {}", &[&(i + 1).to_string(), &total.to_string()]));
    ui.spacer(4);
    ui.box_end();
    // the other language stands in, and says so
    let want = if en { "en" } else { "de" };
    let mut close_note = false;
    if a.text.is_some() && a.text_lang != want && !a.note_closed {
        ui.next_size(-1, 0);
        ui.box_begin(0, BOX_V | pad(8), 0);
        close_note = ui.infobar(ID_AK_NOTE, IB_INFO, t("Diese Lektion gibt es noch nicht in Ihrer Sprache"), &tf("Sie sehen die Fassung auf {}.", &[lang_name(a.text_lang)])) & EV_ACT != 0;
        ui.box_end();
    }
    // the document on its sheet, as wide as reading is comfortable
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_H | BOX_CENTER | pad(12), 0);
    let w = ui.px2dip(ui.box_rect()[2]).clamp(1, READ_W);
    let a = &mut n.academy;
    if close_note {
        a.note_closed = true;
        ui.repass();
    }
    match &a.text {
        Some(text) => {
            ui.next_size(w, -1);
            let ev = ui.markdown_view(ID_AK_LESSON, &mut a.md, text, 0);
            // the pictures: asked for by the view, answered from the course's folder only
            let asked = a.md.image_requests();
            if !asked.is_empty() {
                let dir = a.entries.get(a.cur).map_or(String::new(), |e| e.dir.clone());
                for addr in &asked {
                    let data = super::store::image(&dir, addr);
                    a.md.set_image(addr, data.as_deref());
                }
                ui.repass();
            }
            ui.box_end();
            if let Some(l) = ev.link {
                super::follow_link(n, ui, &l);
            }
        }
        None => {
            ui.next_size(w, 0);
            ui.box_begin(0, BOX_V | pad(24), 0);
            crate::workspace::error_text(ui, t("Die Datei dieser Lektion fehlt oder lässt sich nicht lesen."), 0);
            ui.box_end();
            ui.box_end();
        }
    }
    // the foot: back and on
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_LAYER | pad(8), 8);
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

// ---------------------------------------------------------------- the bar and the question

/// the slim bar over the canvas while the project is a course's lab
pub(crate) fn bar(n: &mut Net, ui: &mut Ui) {
    if n.academy.lab.is_none() {
        return;
    }
    let want = core::mem::take(&mut n.academy.focus);
    take_focus(n, ui, want);
    let a = &n.academy;
    let Some(l) = &a.lab else { return };
    let en = tr::en();
    let Some(c) = a.entries.get(l.course).and_then(|e| e.course.as_ref()) else { return };
    let Some(s) = c.step_at(l.step) else { return };
    // a fault hunt counts when it is handed in
    let hunt = s.kind() == Some(StepKind::Fault);
    let handed = n.task.assess.as_ref().and_then(|r| r.end.as_ref().and(r.score.as_ref())).map(|x| x.percent());
    let pass = n.doc.proj.lab.as_ref().map_or(100, |l| l.pass_percent());
    let done = if hunt { handed.is_some_and(|p| p >= pass) } else { n.task.run.as_ref().is_some_and(|r| r.done()) };
    // beside wide panels in a small window the buttons keep their icons and their names
    let narrow = ui.px2dip(ui.box_rect()[2]) < BAR_W;
    ui.focus_pane_begin(ID_AK_BAR);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_LAYER | pad(4), 8);
    ui.spacer(4);
    ui.icon(IC_OFFICE_READ_MODE, 16);
    // the step first: the middle of the window can be narrow beside the panels
    let names = format!("{} · {}", s.title.get(en), c.title.get(en));
    ui.next_size(-1, 0);
    ui.text(TS_BODY | TX_ELLIPSIS, &names);
    ui.tooltip(&names);
    if done {
        mark(ui, IC_CHECK_CIRCLE_FILL, T_OK, 16);
        ui.tooltip(if hunt { t("Bestanden") } else { t("Alle Ziele erreicht") });
    }
    if hunt && handed.is_some() {
        let again = t("Neu beginnen");
        let ev = if narrow {
            ui.icon_button(ID_AK_BAR_RETRY, IC_REFRESH, again)
        } else {
            ui.next_flags(ST_SUBTLE | ST_COMPACT);
            ui.next_icon(IC_REFRESH);
            ui.button(ID_AK_BAR_RETRY, again)
        };
        if ev & EV_ACT != 0 {
            ui.cmd_run(CMD_AK_RETRY, 0);
        }
        ui.tooltip(t("Diese Fehlersuche noch einmal aus ihrer Datei laden"));
    }
    let back = t("Zurück zum Kurs");
    let ev = if narrow {
        ui.icon_button(ID_AK_BAR_BACK, IC_BACK, back)
    } else {
        ui.next_flags(ST_SUBTLE | ST_COMPACT);
        ui.next_icon(IC_BACK);
        ui.button(ID_AK_BAR_BACK, back)
    };
    if ev & EV_ACT != 0 {
        ui.cmd_run(CMD_AK_BAR_BACK, 0);
    }
    ui.tooltip(back);
    let label = t("Weiter im Kurs");
    let ev = if narrow {
        ui.icon_button(ID_AK_BAR_NEXT, IC_FORWARD, label)
    } else {
        ui.next_icon(IC_FORWARD);
        ui.next_flags(ST_COMPACT);
        if done { ui.accent_button(ID_AK_BAR_NEXT, label) } else { ui.button(ID_AK_BAR_NEXT, label) }
    };
    if ev & EV_ACT != 0 {
        ui.cmd_run(CMD_AK_BAR_NEXT, 0);
    }
    ui.tooltip(if done { t("Der nächste Schritt des Kurses") } else if hunt { t("Der nächste Schritt des Kurses; diese Fehlersuche bleibt offen") } else { t("Der nächste Schritt des Kurses; dieses Labor bleibt offen") });
    ui.spacer(4);
    ui.box_end();
    ui.focus_pane_end();
}

/// "Fortschritt zurücksetzen?", "Test abgeben?", those of installing
pub(crate) fn dialog(n: &mut Net, ui: &mut Ui) {
    super::quiz::dialog(n, ui);
    super::install::dialog(n, ui);
    let open = n.academy.ask_reset.clone();
    if ui.dialog_begin(ID_AK_RESET_DLG, t("Fortschritt zurücksetzen?"), &open) {
        let a = &n.academy;
        let title = a.entries.get(a.cur).map_or(String::new(), |e| String::from(e.title(tr::en())));
        ui.next_size(400, 0);
        ui.text(TS_BODY | TX_WRAP, &tf("Alles, was Sie in „{}“ erledigt haben, gilt dann wieder als offen. Das lässt sich nicht rückgängig machen.", &[&title]));
        if ui.dialog_buttons(Some(t("Zurücksetzen")), None, Some(t("Abbrechen"))) == DLG_PRIMARY {
            ui.cmd_run(CMD_AK_RESET, 0);
        }
    }
    ui.dialog_end();
}
