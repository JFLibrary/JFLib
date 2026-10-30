// perm -- what build_rs.py builds into an app, set up before it starts, and the app's
// permissions on the UI side (plat::policy, docs/PERMISSIONS.md, docs/VFS.md)
//
//   boot(app, manifest, assets)   (main!) the app's name for its folders (plat::vfs::
//                           set_app_id), its asset bundle as app:// (JFA1, apps/NAME/assets),
//                           then its manifest (apps/NAME/PERMISSIONS, a BINS program's own
//                           PERMISSIONS.<bin>); Err(exit code) for a bundle or manifest it
//                           refuses.  Before the manifest: the user's languages and format
//                           (i18n::boot: jfgui_LANG, else the system in window mode, German in
//                           headless runs).  The paths on the command line come from the user:
//                           with fs.user they may be read and written (folders with everything
//                           below).  The steps are plat::boot's, which console programs
//                           (rt::main!) run without the UI.  Script runs (--script) keep the
//                           answers to permission questions in memory (jfgui_PERM_STORE or
//                           `perm store` for a file).
//   grant_user(path)        the same for a path of the file dialog or a drop
//   trusted(f)              f on this thread without file checks: the library's own file
//                           dialog lists folders, scripts read and write their files
//   ask_here(on)            (runtime) the UI thread asks the questions of ask lines and agent
//                           actions (window runs; scripts after `perm answer dialog`)
//   answer(id, a) / set_answer(a) / set_wait(ms) / set_store(path)   (scripts: perm ...)
//   Ui::perm_dialog()       (run_passes, after App::frame) the first open question as a content
//                           dialog: what, the path / host / program / action, the plugin, the
//                           manifest's reason (through the app's catalogue), the ask line, and
//                           "Einmal erlauben" (until the program ends), "Immer erlauben"
//                           (stored), "Ablehnen" (until the program ends), "Nie erlauben"
//                           (stored); Esc = Ablehnen.  The answer reaches the app as
//                           UIC_PERMISSION (arg 1 allowed, 0 refused): an access the UI thread
//                           made while the question was open was refused, it may try again.
//   Ui::permissions_page(id) -> bool   the ready-made settings section: every ask line and
//                           answered agent action with what it allows, the rule, the reason
//                           and the answer; "Zurücksetzen: <rule>" revokes one (asked again
//                           on the next access), "Alle zurücksetzen" all; true when an answer
//                           was revoked.  Ids: the scope of `id`, then the rule ("reset").
//
// The key of plat::policy stays here: apps cannot widen their own permissions or answer
// their own questions.

use alloc::format;
use alloc::string::String;
use core::sync::atomic::{AtomicBool, Ordering};

use plat::policy::{Answer, Key};
use rt::sync::SpinLock;

use super::consts::*;
use super::i18n::{fill, t_text, tr};
use super::keys::IC_UNDO;
use super::win::Shared;
use super::Ui;

static KEY: SpinLock<Option<Key>> = SpinLock::new(None);
// the UI thread asks (ask_here)
static ASKING: AtomicBool = AtomicBool::new(false);

const DLG: u32 = 0xFFFF_7700;
const B_ONCE: u32 = 0xFFFF_7701;
const B_ALWAYS: u32 = 0xFFFF_7702;
const B_DENY: u32 = 0xFFFF_7703;
const B_NEVER: u32 = 0xFFFF_7704;

/// the question on screen
#[derive(Default)]
pub(crate) struct S {
    open: Shared<i32>,
    shown: u32,
}

/// (main!) set up the app: its name, its assets (app://), its permissions
#[doc(hidden)]
pub fn boot(app: Option<&str>, manifest: Option<&str>, assets: &'static [u8]) -> Result<(), i32> {
    plat::boot::name_and_assets(app, assets)?;
    // the user's languages and format (headless runs: jfgui_LANG or German)
    let headless = ["--shot", "--script"].iter().any(|o| plat::arg_value(o).is_some())
        || plat::args().iter().any(|a| a == "--agent-json" || a == "--agent-mcp");
    crate::i18n::boot(headless);
    // the app's manifest (None: the app has none, everything stays allowed; the key still
    // answers the questions of agent actions)
    let key = plat::boot::permissions(manifest)?;
    // test scripts never read or write the user's stored answers
    if plat::arg_value("--script").is_some() && plat::env("jfgui_PERM_STORE").is_none() {
        if let Some(k) = &key {
            k.set_store(None);
        }
    }
    *KEY.lock() = key;
    Ok(())
}

fn key() -> Option<Key> {
    KEY.lock().clone()
}

/// a path the user chose (file dialog, drop): read and write it (with fs.user)
pub(crate) fn grant_user(path: &str) {
    if let Some(k) = key() {
        plat::boot::grant_user(&k, path);
    }
}

/// f without the file checks of the policy on this thread
pub(crate) fn trusted<R>(f: impl FnOnce() -> R) -> R {
    match key() {
        Some(k) => k.trusted(f),
        None => f(),
    }
}

/// the paths of a drop list (each one the user's)
pub(crate) fn grant_all(list: &[String]) {
    for p in list {
        grant_user(p);
    }
}

/// (runtime, scripts) this thread shows the permission questions (on) or nobody does
pub(crate) fn ask_here(on: bool) {
    if let Some(k) = key() {
        k.ask_here(on);
        ASKING.store(on, Ordering::Release);
    }
}

/// whether this run shows the questions (perm_dialog)
pub(crate) fn asking() -> bool {
    ASKING.load(Ordering::Acquire)
}

/// (scripts) answer question id
pub(crate) fn answer(id: u32, a: Answer) -> bool {
    key().is_some_and(|k| k.answer(id, a))
}

/// (scripts) answer every question with a without asking (None: ask)
pub(crate) fn set_answer(a: Option<Answer>) {
    if let Some(k) = key() {
        k.set_answer(a);
    }
}

/// (scripts) how long other threads wait for an answer (ms)
pub(crate) fn set_wait(ms: u32) {
    if let Some(k) = key() {
        k.set_wait(ms);
    }
}

/// (scripts) where Always / Never are kept (None: memory only)
pub(crate) fn set_store(path: Option<&str>) {
    if let Some(k) = key() {
        k.set_store(path);
    }
}

// what a kind of access allows, as the dialog and the page say it
fn what(kind: &str) -> &'static str {
    match kind {
        "fs.read" => tr("Dateien lesen"),
        "fs.write" => tr("Dateien schreiben"),
        "process" => tr("Ein Programm starten"),
        "url" => tr("Eine Adresse öffnen"),
        "plugins" => tr("Ein Plugin laden"),
        "print" => tr("Drucken"),
        "secrets" => tr("Gespeicherte Kennwörter verwenden"),
        "autostart" => tr("Beim Anmelden starten"),
        "audio.output" => tr("Klänge abspielen"),
        "net.connect" => tr("Eine Netzwerkverbindung aufbauen"),
        "net.listen" => tr("Netzwerkverbindungen annehmen"),
        "agent" => tr("Eine Aktion für den Agenten ausführen"),
        _ => tr("Zugriff erlauben?"),
    }
}

// the state of an answer on the page
fn state(a: Option<Answer>) -> &'static str {
    match a {
        None => tr("Wird beim nächsten Zugriff gefragt"),
        Some(Answer::Once) => tr("Bis zum Programmende erlaubt"),
        Some(Answer::Always) => tr("Immer erlaubt"),
        Some(Answer::Deny) => tr("Bis zum Programmende abgelehnt"),
        Some(Answer::Never) => tr("Nie erlaubt"),
    }
}

impl Ui {
    /// (run_passes) the first open permission question as a content dialog (module header)
    pub(crate) fn perm_dialog(&mut self) {
        if !asking() {
            return;
        }
        let qs = plat::policy::questions();
        let Some(q) = qs.first() else {
            self.m.perm.shown = 0;
            return;
        };
        if self.m.perm.shown != q.id {
            self.m.perm.shown = q.id;
            self.m.perm.open.set(1);
        }
        let open = self.m.perm.open.clone();
        let mut picked = None;
        if self.dialog_begin(DLG, tr("Zugriff erlauben?"), &open) {
            self.text(TS_BODY_STRONG, what(q.kind));
            if !q.target.is_empty() {
                self.text(TS_BODY, &q.target);
            }
            if !q.plugin.is_empty() {
                self.text(TS_BODY, &fill(tr("Für das Plugin: {}"), &[&q.plugin]));
            }
            if !q.reason.is_empty() {
                self.text(TS_BODY, &fill(tr("Grund: {}"), &[&t_text(&q.reason)]));
            }
            self.text(TS_CAPTION, &fill(tr("Regel: {}"), &[&q.rule]));
            self.box_begin(0, BOX_H | BOX_WRAP, 8);
            if self.accent_button(B_ONCE, tr("Einmal erlauben")) & EV_ACT != 0 {
                picked = Some(Answer::Once);
            }
            if self.button(B_ALWAYS, tr("Immer erlauben")) & EV_ACT != 0 {
                picked = Some(Answer::Always);
            }
            if self.button(B_DENY, tr("Ablehnen")) & EV_ACT != 0 {
                picked = Some(Answer::Deny);
            }
            if self.button(B_NEVER, tr("Nie erlauben")) & EV_ACT != 0 {
                picked = Some(Answer::Never);
            }
            self.box_end();
        }
        self.dialog_end();
        if picked.is_none() && open.get() == 0 {
            picked = Some(Answer::Deny); // Esc
        }
        if let Some(a) = picked {
            open.set(0);
            self.m.perm.shown = 0;
            if answer(q.id, a) {
                self.cmd_run(UIC_PERMISSION, a.allows() as u32);
            }
            self.repass();
        }
    }

    /// the permissions section of a settings page: the answers to the app's permission
    /// questions, each with a reset (module header) -> an answer was revoked
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn permissions_page(&mut self, id: u32) -> bool {
        let _src = self.src_scope();
        let list = plat::policy::decisions();
        let mut changed = false;
        self.id_push(id);
        self.box_begin(0, BOX_V | BOX_CARD | pad(16), 12);
        if list.is_empty() {
            self.text(TS_BODY, tr("Keine Rückfragen"));
        }
        for d in &list {
            self.id_push_str(&d.rule);
            self.box_begin(0, BOX_H | BOX_XCENTER, 12);
            self.next_size(-1, 0);
            self.box_begin(0, BOX_V, 2);
            self.text(TS_BODY_STRONG, what(d.kind));
            let target = d.rule.split_once(' ').map_or("", |x| x.1);
            if !target.is_empty() {
                self.text(TS_BODY, target);
            }
            if !d.reason.is_empty() {
                self.text(TS_CAPTION, &t_text(&d.reason));
            }
            self.text(TS_CAPTION, state(d.answer));
            self.box_end();
            if d.answer.is_none() {
                self.next_flags(ST_DISABLED);
            }
            let reset = format!("{}: {}", tr("Zurücksetzen"), d.rule);
            if self.icon_button(self.id("reset"), IC_UNDO, &reset) & EV_ACT != 0 {
                changed |= plat::policy::forget(&d.rule);
            }
            self.box_end();
            self.id_pop();
        }
        if !list.iter().any(|d| d.answer.is_some()) {
            self.next_flags(ST_DISABLED);
        }
        if self.button(self.id("reset.all"), tr("Alle zurücksetzen")) & EV_ACT != 0 {
            plat::policy::forget_all();
            changed = true;
        }
        self.box_end();
        self.id_pop();
        if changed {
            self.repass();
        }
        changed
    }
}
