// permprompt -- questions at run time: the ask lines of apps/permprompt/PERMISSIONS, the
// library's dialog, the answers (this run, stored, revoked) and the settings section
// ui.permissions_page; agent actions that need the user's confirmation or are refused
// (docs/PERMISSIONS.md "Asking the user", "Agent control"); for tests/*.uis
//
//   build/win/permprompt.exe --script apps/permprompt/tests/dialog.uis
//   build/win/permprompt.exe          (a window: the buttons ask for real)
//
//   "Datei schreiben"            $TEMP/permprompt-ask/a.txt on the UI thread: refused while its
//                                question is open, written again after the answer
//                                (UIC_PERMISSION)
//   "Im Hintergrund schreiben"   $TEMP/permprompt-bg/b.txt from a pool thread, which waits for
//                                the answer
//   "Mail prüfen" / "Drucken prüfen"   policy::url_allowed("mailto:..") / print_allowed (nothing
//                                is opened or printed)
//   "Antworten überschreiben"    the app writes the file of the stored answers (refused)
// Agent actions: doc.count (open), doc.save (agent.confirm), doc.wipe (agent.deny), doc.peek
// (ActionSpec::local_only).
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;

use agent_core::{ActionContext, ActionError, ActionResult, ActionSpec, ErrorCode, Value};
use plat::policy;
use plat::task::{spawn_blocking, Task};
use ui::*;

const DESC: AppDesc = AppDesc { title: "permprompt", flags: AF_RESIZABLE, w: 720, h: 1000, min_w: 400, min_h: 300, ..AppDesc::NONE };

static ACTIONS: [ActionSpec; 4] = [
    ActionSpec::read("doc.count", "Count the saved documents"),
    ActionSpec { revision_required: false, ..ActionSpec::write("doc.save", "Save the document") },
    ActionSpec { revision_required: false, ..ActionSpec::write("doc.wipe", "Delete every document") },
    ActionSpec::read("doc.peek", "Show the private notes").local_only(),
];

struct P {
    ask: String,
    bg_path: String,
    write: &'static str,
    retry: bool,
    bg: Option<Task<bool>>,
    bg_state: &'static str,
    mail: &'static str,
    print: &'static str,
    store: &'static str,
    saves: u64,
}

impl P {
    fn new() -> P {
        let temp = plat::temp_dir().unwrap_or_default();
        P {
            ask: format!("{temp}/permprompt-ask/a.txt"),
            bg_path: format!("{temp}/permprompt-bg/b.txt"),
            write: "-",
            retry: false,
            bg: None,
            bg_state: "-",
            mail: "-",
            print: "-",
            store: "-",
            saves: 0,
        }
    }

    // on the UI thread: refused while the question is open, done again after the answer
    fn write_file(&mut self) {
        if put(&self.ask, b"Brief") {
            self.write = "geschrieben";
        } else if !policy::questions().is_empty() {
            self.write = "wartet auf Antwort";
            self.retry = true;
        } else {
            self.write = "abgelehnt";
        }
    }
}

// write path, its folder made first (both under the same ask line)
fn put(path: &str, data: &[u8]) -> bool {
    let dir = &path[..path.rfind('/').unwrap_or(0)];
    plat::mkdir(dir) && plat::write_file(path, data)
}

fn yes(ok: bool) -> &'static str {
    if ok { "erlaubt" } else { "abgelehnt" }
}

impl App for P {
    fn frame(&mut self, ui: &mut Ui) {
        if let Some(t) = &self.bg {
            if let Some(ok) = t.take() {
                self.bg_state = if ok { "geschrieben" } else { "abgelehnt" };
                self.bg = None;
            }
        }
        ui.box_begin(0, BOX_V | pad(16), 8);
        ui.text(TS_SUBTITLE, "Rückfragen");
        ui.box_begin(0, BOX_H | BOX_WRAP, 8);
        if ui.button(ui.id("write"), "Datei schreiben") & EV_ACT != 0 {
            self.write_file();
        }
        if ui.button(ui.id("bg"), "Im Hintergrund schreiben") & EV_ACT != 0 {
            let path = self.bg_path.clone();
            self.bg = Some(ui.spawn(async move { spawn_blocking(move || put(&path, b"Sicherung")).await }));
            self.bg_state = "läuft";
        }
        if ui.button(ui.id("mail"), "Mail prüfen") & EV_ACT != 0 {
            self.mail = yes(policy::url_allowed("mailto:support@example.org"));
        }
        if ui.button(ui.id("print"), "Drucken prüfen") & EV_ACT != 0 {
            self.print = yes(policy::print_allowed());
        }
        if ui.button(ui.id("store"), "Antworten überschreiben") & EV_ACT != 0 {
            let ok = plat::write_file("config://permission-decisions.txt", b"always print\n");
            self.store = if ok { "geschrieben" } else { "abgelehnt" };
        }
        ui.box_end();
        ui.text(TS_BODY, &format!("Datei: {}", self.write));
        ui.text(TS_BODY, &format!("Hintergrund: {}", self.bg_state));
        ui.text(TS_BODY, &format!("Mail: {}", self.mail));
        ui.text(TS_BODY, &format!("Drucken: {}", self.print));
        ui.text(TS_BODY, &format!("Antworten: {}", self.store));
        ui.text(TS_BODY, &format!("Agent: {} gespeichert", self.saves));
        ui.text(TS_BODY_STRONG, "Berechtigungen");
        ui.permissions_page(ui.id("perms"));
        ui.box_end();
    }

    fn command(&mut self, _ui: &mut Ui, id: u32, arg: u32) -> bool {
        if id == UIC_PERMISSION && self.retry {
            self.retry = false;
            if arg == 1 {
                self.write_file();
            } else {
                self.write = "abgelehnt";
            }
        }
        true
    }

    fn actions(&self) -> &'static [ActionSpec] {
        &ACTIONS
    }

    fn action(&mut self, _ui: &mut Ui, spec: &ActionSpec, _input: &Value, _context: &ActionContext) -> Result<ActionResult, ActionError> {
        match spec.name {
            "doc.count" | "doc.peek" => {}
            "doc.save" => self.saves += 1,
            "doc.wipe" => self.saves = 0,
            _ => return Err(ActionError::new(ErrorCode::NotFound, "Unknown action")),
        }
        Ok(ActionResult::completed(Value::object([("saved", Value::u64(self.saves))]), 0))
    }
}

ui::main!(ui::launch(&DESC, P::new(), &style::STYLE));
