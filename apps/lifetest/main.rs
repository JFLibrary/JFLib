// lifetest -- the app lifecycle of the runtime (UIC_ACTIVE, UIC_SUSPEND, UIC_SESSION_END,
// UIC_SETTINGS, UIC_THEME), the log and the settings it follows (high contrast, text scale,
// animations), for tests/life.uis and tests/settings.uis; the runtime's queues (keys of one
// pass, commands, timers) for tests/queues.uis
//
//   build/win/lifetest.exe --script apps/lifetest/tests/life.uis
//
// It shows what the commands told it and the newest log line (without its time); the
// script sends the platform's events with `system ...`.  Test hooks (`cmd ID ARG`):
//   9101 N   queue N commands (9104, 0 .. N-1) in the next frame: they must come in order
//   9102 N   N timers with different intervals, set out of order: they fire by deadline
//   9103 N   N timers due within one frame, ten intervals: equal deadlines in the order set
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;

use ui::*;

const DESC: AppDesc = AppDesc { title: "jfgui Lifecycle", flags: AF_RESIZABLE, w: 640, h: 360, ..AppDesc::NONE };

const CMD_QUEUE: u32 = 9101; // test hook: queue N commands in the next frame
const CMD_TIMERS: u32 = 9102; // test hook: N timers, different intervals
const CMD_TIMERS_SAME: u32 = 9103; // test hook: N timers, ten intervals within one frame
const CMD_ITEM: u32 = 9104; // a queued command (arg = its number)
const TIMER_BASE: u32 = 0x7100_0000; // timer ids: TIMER_BASE + k

#[derive(Default)]
struct Life {
    active: bool,
    sleeping: bool,
    session_end: u32,
    settings: u32,
    themes: u32,
    /// keys taken in all passes, the most in one pass, their letters in order
    keys: usize,
    keys_pass: usize,
    key_seq: String,
    /// commands to queue in the next frame; queued ones received, all in order
    to_queue: u32,
    cmds: u32,
    cmds_disorder: bool,
    /// timers: the test (CMD_TIMERS / CMD_TIMERS_SAME), how many were set and fired, the last
    /// one's order key, all in order
    tmode: u32,
    tset: u32,
    tfired: u32,
    tlast: Option<(u32, u32)>,
    tdisorder: bool,
}

// the newest log line without its time: "INFO  app: ..."
fn last_log() -> String {
    let r = plat::log_recent(4096);
    let line = r.lines().last().unwrap_or("");
    let rest = line.split_once(' ').map_or("", |x| x.1);
    String::from(rest.split_once(' ').map_or("", |x| x.1))
}

impl Life {
    // timer k of the current test: its interval (ms)
    fn tms(&self, k: u32) -> u32 {
        if self.tmode == CMD_TIMERS {
            20 + (k * 37) % self.tset.max(1) // a permutation of 0 .. N-1 (N = 100)
        } else {
            5 + k % 10 // ten deadlines inside the first frame of `wait`
        }
    }
}

impl App for Life {
    fn frame(&mut self, ui: &mut Ui) {
        // every key of this pass, in order (`keys` in a script queues many for one pass)
        let mut n = 0;
        loop {
            let k = ui.key_first();
            if k == 0 {
                break;
            }
            ui.key(k & 0xFFFF);
            let c = (k & 0xFFFF) as u8;
            self.key_seq.push(if c.is_ascii_alphanumeric() { c as char } else { '?' });
            n += 1;
        }
        self.keys += n;
        self.keys_pass = self.keys_pass.max(n);
        // the commands of the test hook, all queued in this pass
        for k in 0..core::mem::take(&mut self.to_queue) {
            ui.cmd_run(CMD_ITEM, k);
        }
        ui.box_begin(0, BOX_V | pad(16), 8);
        ui.text(TS_SUBTITLE, "Lebenszyklus");
        let yn = |b: bool| if b { "ja" } else { "nein" };
        ui.text(TS_BODY, &format!("Aktiv: {}  Schlaf: {}  Ende: {}", yn(self.active), yn(self.sleeping), self.session_end));
        ui.text(TS_BODY, &format!("Einstellungen: {}  Theme: {}", self.settings, self.themes));
        ui.text(TS_BODY, &format!("Kontrast: {}  Text: {} %  Animationen: {}", yn(ui.high_contrast()), ui.text_scale(),
                                  if ui.anim_off() { "aus" } else { "an" }));
        ui.text(TS_BODY, &format!("Log: {}", last_log()));
        ui.text(TS_BODY, &format!("Tasten: {}, höchstens {} in einem Durchgang", self.keys, self.keys_pass));
        ui.text(TS_BODY, &format!("Folge: {}", self.key_seq));
        ui.text(TS_BODY, &format!("Befehle: {}, Reihenfolge: {}", self.cmds, yn(!self.cmds_disorder)));
        ui.text(TS_BODY, &format!("Timer: {} von {}, Reihenfolge: {}", self.tfired, self.tset, yn(!self.tdisorder)));
        ui.box_end();
    }

    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        match id {
            UIC_ACTIVE => self.active = arg != 0,
            UIC_SUSPEND => self.sleeping = arg != 0,
            UIC_SESSION_END => {
                self.session_end += 1;
                plat::log_info("lifetest", "saved before the end");
            }
            UIC_SETTINGS => self.settings = arg,
            UIC_THEME => self.themes += 1,
            CMD_QUEUE => {
                self.to_queue = arg;
                (self.cmds, self.cmds_disorder) = (0, false);
                ui.invalidate();
            }
            CMD_ITEM => {
                self.cmds_disorder |= arg != self.cmds;
                self.cmds += 1;
            }
            CMD_TIMERS | CMD_TIMERS_SAME => {
                (self.tmode, self.tset, self.tfired, self.tlast, self.tdisorder) = (id, arg, 0, None, false);
                for k in 0..arg {
                    ui.timer(TIMER_BASE + k, self.tms(k));
                }
            }
            UIC_TIMER if arg.wrapping_sub(TIMER_BASE) < self.tset => {
                // by deadline; of equal deadlines in the order set
                let k = arg - TIMER_BASE;
                let key = (self.tms(k), k);
                self.tdisorder |= self.tlast.is_some_and(|l| l >= key);
                self.tlast = Some(key);
                self.tfired += 1;
            }
            _ => return false,
        }
        true
    }
}

ui::main!(ui::launch(&DESC, Life::default(), &style::STYLE));
