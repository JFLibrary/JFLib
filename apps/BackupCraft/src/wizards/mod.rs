// wizards -- full-page assistants: the step list at the left, the step's content, a footer
// with Back / Next / Finish / Cancel; job (create, edit), restore, agent pairing, agent
// installation over SSH
//
//   open_named(app, ui, "job" | "restore" | "pair" | "ssh")
//   open_job_edit(app, ui, &job) / open_restore(app, ui, &job, point)

pub mod job;
pub mod pair;
pub mod restore;
pub mod ssh;

use alloc::string::String;

use bc_proto::j::Value;
use bc_proto::model::Job;
use ui::*;

use crate::app::{Console, ID_WIZ};
use crate::tr::t;

pub enum Wizard {
    Job(job::JobWiz),
    Restore(restore::RestoreWiz),
    Pair(pair::PairWiz),
    Ssh(ssh::SshWiz),
}

impl Wizard {
    /// whether an action label belongs to this wizard
    pub fn owns(&self, label: &str) -> bool {
        match self {
            Wizard::Job(_) => label == "job.save",
            Wizard::Restore(_) => label == "restore.start",
            Wizard::Pair(_) => label == "pair.code",
            Wizard::Ssh(_) => label.starts_with("ssh."),
        }
    }

    /// the job to start after a save ("start right after saving")
    pub fn run_after(&self, body: &Value) -> Option<String> {
        match self {
            Wizard::Job(w) => w.run_after(body),
            _ => None,
        }
    }

    /// an action of this wizard succeeded -> close the wizard
    pub fn done(&mut self, label: &str, body: &Value) -> bool {
        match self {
            Wizard::Job(w) => w.done(body),
            Wizard::Restore(_) => true,
            Wizard::Pair(w) => w.done(body),
            Wizard::Ssh(w) => w.done(label, body),
        }
    }

    /// an action of this wizard failed (the message is shown at its footer)
    pub fn failed(&mut self, code: &str, message: &str) {
        let (busy, error) = match self {
            Wizard::Job(w) => (&mut w.busy, &mut w.error),
            Wizard::Restore(w) => (&mut w.busy, &mut w.error),
            Wizard::Pair(w) => (&mut w.busy, &mut w.error),
            Wizard::Ssh(w) => (&mut w.busy, &mut w.error),
        };
        *busy = false;
        *error = String::from(message);
        let _ = code;
    }

    /// draw the wizard -> keep it open
    pub fn frame(&mut self, app: &mut Console, ui: &mut Ui) -> bool {
        match self {
            Wizard::Job(w) => w.frame(app, ui),
            Wizard::Restore(w) => w.frame(app, ui),
            Wizard::Pair(w) => w.frame(app, ui),
            Wizard::Ssh(w) => w.frame(app, ui),
        }
    }
}

pub fn open_named(app: &mut Console, ui: &mut Ui, name: &str) {
    match name {
        "job" => app.wizard = Some(Wizard::Job(job::JobWiz::new(app, None))),
        "restore" => {
            let j = app.net.data.jobs.iter().find(|j| j.points > 0).cloned();
            if let Some(j) = j {
                open_restore(app, ui, &j, None);
                return;
            }
        }
        "pair" => app.wizard = Some(Wizard::Pair(pair::PairWiz::default())),
        "ssh" => app.wizard = Some(Wizard::Ssh(ssh::SshWiz::default())),
        _ => {}
    }
    ui.repass();
}

pub fn open_job_edit(app: &mut Console, ui: &mut Ui, job: &Job) {
    app.wizard = Some(Wizard::Job(job::JobWiz::new(app, Some(job))));
    ui.repass();
}

pub fn open_restore(app: &mut Console, ui: &mut Ui, job: &Job, point: Option<String>) {
    app.wizard = Some(Wizard::Restore(restore::RestoreWiz::new(job, point)));
    ui.repass();
}

// ------------------------------------------------------------------ the page frame

/// the wizard page: title row (with a close button), steps at the left; the caller emits the
/// step content and then calls page_end -> (step clicked, close clicked)
pub fn page_begin(ui: &mut Ui, id: u32, title: &str, subtitle: &str, steps: &[&str], cur: usize, states: &[i32]) -> (Option<usize>, bool) {
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | pad(24), 16);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
    ui.next_size(-1, 0);
    ui.page_header(title, subtitle);
    let close = ui.icon_button(id + 1, IC_CANCEL, t("Schließen")) & EV_ACT != 0;
    ui.box_end();
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_H, 16);
    ui.next_size(250, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | pad(10), 0);
    let clicked = ui.wizard_steps(id + 10, steps, cur, states);
    ui.box_end();
    ui.next_size(-1, -1);
    ui.box_begin(0, BOX_V | BOX_CARD | BOX_CLIP | BOX_FIXED, 0);
    ui.next_size(-1, -1);
    ui.scroll_begin(id + 40, SC_V | BOX_FIXED);
    // one id per step: the steps' anonymous boxes keep their own measurements
    ui.box_begin(id + 32 + cur.min(7) as u32, BOX_V | pad(24), 14);
    (clicked, close)
}

/// end of the step content; the footer -> 1 back, 2 next / finish, 3 cancel, 0 nothing
#[allow(clippy::too_many_arguments)]
pub fn page_end(ui: &mut Ui, id: u32, first: bool, last: bool, can_next: bool, busy: bool, error: &str, finish: &str) -> u32 {
    ui.box_end();
    ui.scroll_end();
    // footer
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_LAYER | pad(14), 10);
    if !error.is_empty() {
        ui.icon(IC_ERROR, 16);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_ELLIPSIS, error);
    } else if busy {
        ui.spinner(id + 60, 16);
        ui.next_size(-1, 0);
        ui.text(TS_BODY | TX_SECONDARY, t("Wird gespeichert …"));
    } else {
        ui.spacer(-1);
    }
    let mut r = 0;
    if ui.button(id + 61, t("Abbrechen")) & EV_ACT != 0 {
        r = 3;
    }
    ui.next_flags(if first || busy { ST_DISABLED } else { 0 });
    ui.next_icon(IC_CHEV_LEFT);
    if ui.button(id + 62, t("Zurück")) & EV_ACT != 0 {
        r = 1;
    }
    ui.next_flags(if can_next && !busy { 0 } else { ST_DISABLED });
    if ui.accent_button(id + 63, if last { finish } else { t("Weiter") }) & EV_ACT != 0 {
        r = 2;
    }
    ui.box_end();
    ui.box_end();
    ui.box_end();
    ui.box_end();
    r
}

/// a choice tile (icon, title, text), selected look; disabled for staged kinds -> clicked
pub fn tile(ui: &mut Ui, id: u32, icon: u32, title: &str, text: &str, selected: bool, enabled: bool, badge: Option<&str>) -> bool {
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_LAYER | pad(14), 14);
    let r = ui.box_rect();
    let p = ui.dp(14);
    let ev = ui.hit_rect(id, r[0] - p, r[1] - p, r[2] + p * 2, r[3] + p * 2, if enabled { HB_FOCUS } else { HB_DISABLED });
    if selected || ev & EV_HOVER != 0 {
        let c = ui.color(if selected { T_ACCENT } else { T_CTRL_STROKE });
        ui.cmd_rrect(r[0] - p, r[1] - p, r[2] + p * 2, r[3] + p * 2, ui.dp(8), if selected { ui.dp(2) } else { ui.hair() }, 0, c, c);
    }
    if !enabled {
        ui.disabled_push();
    }
    ui.icon(icon, 26);
    ui.next_size(-1, 0);
    ui.box_begin(0, BOX_V, 2);
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.text(TS_BODY_STRONG, title);
    if let Some(b) = badge {
        ui.badge(BADGE_NEUTRAL, b);
    }
    ui.box_end();
    ui.next_size(-1, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, text);
    ui.box_end();
    if selected {
        ui.icon(IC_CHECK_CIRCLE_FILL, 18);
    }
    if !enabled {
        ui.disabled_pop();
    }
    ui.box_end();
    ev & EV_ACT != 0 && enabled
}

pub const ID_WJOB: u32 = ID_WIZ;
pub const ID_WRESTORE: u32 = ID_WIZ + 0x2_0000;
pub const ID_WPAIR: u32 = ID_WIZ + 0x4_0000;
pub const ID_WSSH: u32 = ID_WIZ + 0x6_0000;
