// hotreload -- the runtime part of hot reload (`python build_rs.py watch NAME`, tools/hotreload.py):
// the session's input recorded, replayed after a restart, the state outside the app carried
// over, the watcher's requests and a failed build shown in the inspector
//
// Only in development builds of the desktop targets (build_rs.py gives `ui` --cfg jfgui_dev;
// never with --release, for the browser or Android) and only when the watcher set
// jfgui_HOTRELOAD=DIR.  Otherwise the runtime's hooks test a flag that is constant false:
// nothing is recorded, read or drawn.
//
// DIR, the session (shared with the watcher):
//   events.log   the session's input, appended by every run (records below); a start replays
//                it first, so the app's state comes back through the app's own code
//   carry.ini    written at a regular end: the style, the text scale, the main window's size
//                and dpi (taken by headless starts; a window has window.ini), the inspector
//                (armed, open, tab, options, the selected widget's script selector)
//   window.ini   the windows' placement (winstate.rs: the watcher sets jfgui_STATE to it); the
//                tool windows' open flags are dropped at the start (the replay opens them)
//   ctl          the watcher's request "quit": the app ends as with --auto-close
//   build.txt    the last build's error (empty: it built); shown at the top of the inspector
//   replay.txt   the start's report: "fresh", "replayed N events", "tainted: WHY", "diverged: WHY"
//
// events.log, one record per line (numbers decimal, T in ms of the session's clock):
//   s T W H DPI                     the main window's client size (px) and dpi from here on
//   e T TYP BTN MODS X Y DATA WIN CHK [TEXT]   a platform event (the fields of --log-events)
//                                   of the window with the app id WIN (0: the main window);
//                                   CHK the widget it reached: a press or wheel the one under
//                                   the pointer, a release the pressed one, a key or text the
//                                   focused one; "-" no check, "=" an event that is no input
//                                   (a window's close button); TEXT an input method's commit (hex)
//   b                               the end of a batch (one ui_process_events)
//   a T 0|1                         the app lost / got the keyboard (UIC_ACTIVE)
//   c FMT HEX|-                     what a clipboard read of the app returned (CLIP_*)
//   x T                             a run ended at T (the next run goes on from there)
//   ! WHY                           input that cannot be replayed came (a drop from another
//                                   program, the system's file dialog, a drag out of the app)
// The replay runs before the window loop: the clock goes from record to record (timers fire
// at their deadlines, animations jump to the time), each batch goes through
// ui_process_events as it came, a clipboard read gets the recorded answer; the system's
// clipboard, file dialogs, sounds and other programs are left alone (the headless ways).
// Every checked event must reach the same widget as when it was recorded, else the replay
// diverged: the start reports it and ends with HR_EXIT_DIVERGED (the watcher starts the app
// again without the log).  Afterwards the window's real size, dpi and keyboard state apply and
// the clock goes on from the replayed time.

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::consts::*;
use super::{App, Ui};
use crate::i18n::tr;
use plat::Event;

/// the exit code of a start whose replay diverged (the watcher starts it again afresh)
pub(crate) const HR_EXIT_DIVERGED: i32 = 3;

// development builds of the desktop targets (--cfg jfgui_dev); elsewhere the hooks fold away
const DEV: bool = cfg!(jfgui_dev);

const PH_OFF: u8 = 0;
const PH_REPLAY: u8 = 1; // from the start until the log was replayed
const PH_REC: u8 = 2;

const CLIP_MAX: usize = 1 << 20; // a recorded clipboard answer (bytes); larger: no replay
const POLL_MS: u32 = 250; // the watcher's files, while the window runs
const TIMERS_MAX: u32 = 1_000_000; // timer rounds of one step of the replay
const ERR_LINES: usize = 24; // of a failed build, in the inspector
const ID_NOTE: u32 = 0x07FE_0030; // the inspector's note (inside the inspector's id range)

/// a recorded event's check: none, the widget id, no input (it never reaches a widget)
#[derive(Clone, Copy, PartialEq)]
enum Chk {
    None,
    Id(u32),
    Outside,
}

/// a recorded event of a batch: the event (window: the app id), its check, a commit's text
struct Rec {
    ev: Event,
    win: u32,
    chk: Chk,
    text: Option<String>,
}

/// hot reload's state (ui.m.hr)
#[derive(Default)]
pub(crate) struct S {
    phase: u8,
    /// the session folder (jfgui_HOTRELOAD)
    dir: String,
    /// recording: the log, its length, the records of the batch not written yet, the events
    /// in them, the last event waits for its commit text, the main window's (w, h, dpi) of
    /// the last `s` record, the log is tainted
    log: Option<plat::File>,
    log_len: u64,
    pend: Vec<String>,
    batch: u32,
    commit: bool,
    size: (i32, i32, i32),
    tainted: bool,
    /// replaying: the log is being read, events of the batch still in the queue, the event
    /// taken is one of them, the checks of the batch's input events, the recorded clipboard
    /// answers, why it diverged
    in_log: bool,
    left: u32,
    cur: bool,
    expect: VecDeque<Chk>,
    clips: VecDeque<(u32, Option<Vec<u8>>)>,
    diverged: Option<String>,
    /// carry.ini of the last run
    carry: Option<base::Ini>,
    /// the watcher's files: the next poll (ms), build.txt as last read, the error shown, its
    /// note closed, the inspector opened for it
    poll_at: u32,
    build_raw: Vec<u8>,
    build_err: Option<String>,
    note_hidden: bool,
    auto_open: bool,
}

fn hex(b: &[u8]) -> String {
    const D: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(b.len() * 2);
    for &c in b {
        s.push(D[(c >> 4) as usize] as char);
        s.push(D[(c & 15) as usize] as char);
    }
    s
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if b.len() % 2 != 0 {
        return None;
    }
    let d = |c: u8| (c as char).to_digit(16);
    b.chunks(2).map(|p| Some((d(p[0])? << 4 | d(p[1])?) as u8)).collect()
}

// the numbers of a record: field i (None: missing or not a number)
fn num(f: &[&str], i: usize) -> Option<i64> {
    f.get(i)?.parse().ok()
}

// a short name of an event type for the reports
fn ev_name(typ: u8) -> &'static str {
    match typ {
        plat::E_DOWN => "a press",
        plat::E_UP => "a release",
        plat::E_WHEEL => "a wheel turn",
        plat::E_KEY_DOWN => "a key",
        plat::E_TEXT | plat::E_IME_COMMIT => "text",
        plat::E_CLOSE => "a close button",
        _ => "an event",
    }
}

impl Ui {
    /// hot reload runs (jfgui_HOTRELOAD in a development build)
    #[inline]
    pub(crate) fn hr_on(&self) -> bool {
        DEV && self.m.hr.phase != PH_OFF
    }

    /// the log is being replayed: the system's clipboard, file dialogs, sounds and other
    /// programs are left alone (the headless ways are taken)
    #[inline]
    pub(crate) fn hr_replaying(&self) -> bool {
        DEV && self.m.hr.phase == PH_REPLAY
    }

    fn hr_path(&self, name: &str) -> String {
        format!("{}/{}", self.m.hr.dir, name)
    }

    // ------------------------------------------------------------------ start and end

    /// (runtime: ui::run, before the window or the headless run) jfgui_HOTRELOAD=DIR: the
    /// session's carried state read, the tool windows' open flags dropped from the placement
    /// file (jfgui_STATE)
    pub(crate) fn hr_init(&mut self) {
        if !DEV {
            return;
        }
        let Some(dir) = plat::env("jfgui_HOTRELOAD").filter(|d| !d.is_empty()) else { return };
        self.m.hr.dir = String::from(dir.trim_end_matches(['/', '\\']));
        self.m.hr.phase = PH_REPLAY;
        let carry = self.hr_path("carry.ini");
        self.m.hr.carry = super::perm::trusted(|| base::Ini::load_checked(&carry));
        if let Some(p) = plat::env("jfgui_STATE").filter(|p| p.len() > 1) {
            super::perm::trusted(|| {
                let mut ini = base::Ini::load(&p);
                let secs: Vec<String> = ini.sections().iter().filter(|s| s.starts_with("window ")).map(|s| String::from(*s)).collect();
                let mut changed = false;
                for s in &secs {
                    changed |= ini.remove(s, "open");
                }
                if changed {
                    ini.save(&p);
                }
            });
        }
    }

    /// (runtime: after App::init and the first passes, window or headless) the log replayed,
    /// the carried state set again, then the input recorded -> false: the replay diverged
    /// (replay.txt says why; the caller ends with HR_EXIT_DIVERGED)
    pub(crate) fn hr_start(&mut self, app: &mut dyn App) -> bool {
        if !self.hr_on() {
            return true;
        }
        let path = self.hr_path("events.log");
        let log = super::perm::trusted(|| plat::read_file(&path)).unwrap_or_default();
        let log = String::from_utf8_lossy(&log).into_owned();
        // what the platform queued already (the window's first size, its focus) is no input
        // of the session
        self.process_events_deferred(app, false);
        let (t0, close_at) = (self.now_ms, self.close_at);
        let taint = log.lines().find_map(|l| l.strip_prefix("! "));
        let (report, keep) = if !log.lines().any(|l| l.starts_with("e ")) {
            (String::from("fresh"), false)
        } else if let Some(why) = taint {
            (format!("tainted: {}", why), false)
        } else {
            match self.hr_replay(app, &log) {
                Ok(n) => (format!("replayed {} events", n), true),
                Err(e) => {
                    let msg = format!("diverged: {}", e);
                    self.hr_report(&msg);
                    plat::write_stderr(format!("hot reload: the replay {}\n", msg).as_bytes());
                    self.m.hr.phase = PH_OFF; // no log, no carry: the next start is fresh
                    return false;
                }
            }
        };
        if !self.headless {
            // the real window again: its size and dpi, the keyboard; the clock goes on from
            // the replayed time
            let dpi = plat::dpi();
            if dpi != self.win_dpi(0) {
                self.uiw_dpi(0, dpi);
            }
            let (w, h) = plat::window_size();
            self.uiw_resize(app, 0, w, h);
            self.t0_us = plat::time_us().saturating_sub(self.now_ms as u64 * 1000);
            if close_at != 0 {
                self.close_at = close_at.wrapping_add(self.now_ms.wrapping_sub(t0));
            }
            let active = self.app_focus != 0;
            self.hr_apply_active(app, active);
        }
        self.hr_carry_apply(app);
        self.hr_open_log(keep);
        self.hr_report(&report);
        self.hr_poll_files();
        self.run_passes(app); // (the inspector opened for a failed build has its window)
        self.dirty_force_full();
        self.need = true;
        true
    }

    /// (runtime: a regular end of the run, window or headless) the log's last records and
    /// carry.ini
    pub(crate) fn hr_end(&mut self) {
        if !self.hr_on() || self.m.hr.phase != PH_REC {
            return;
        }
        self.hr_size_note();
        self.hr_flush();
        let t = self.now_ms;
        self.hr_write(&format!("x {}\n", t));
        self.m.hr.log = None;
        let mut ini = base::Ini::new();
        ini.set("ui", "style", self.style_name());
        if self.text_scale_forced {
            ini.set_int("ui", "text_scale", self.text_scale as i64);
        }
        // the main window's size and dpi (a headless start takes them; a window has its
        // placement in window.ini and the dpi of its monitor)
        let fb = self.win_fb(0);
        let (w, h, dpi) = (fb.w, fb.h, self.win_dpi(0));
        ini.set_int("ui", "w", w as i64);
        ini.set_int("ui", "h", h as i64);
        ini.set_int("ui", "dpi", dpi as i64);
        let (armed, on, tab, opts, sel) = self.ins_carry();
        ini.set_int("inspector", "armed", armed as i64);
        ini.set_int("inspector", "open", (on && !self.m.hr.auto_open) as i64);
        ini.set_int("inspector", "tab", tab as i64);
        ini.set_int("inspector", "outlines", opts[0] as i64);
        ini.set_int("inspector", "order", opts[1] as i64);
        ini.set_int("inspector", "flash", opts[2] as i64);
        if let Some(s) = sel {
            ini.set("inspector", "select", &hex(s.as_bytes()));
        }
        let p = self.hr_path("carry.ini");
        if !super::perm::trusted(|| ini.save(&p)) {
            plat::log_warn("app", &format!("hot reload: could not write {}", p));
        }
        self.m.hr.phase = PH_OFF;
    }

    // the start's report for the watcher (replay.txt)
    fn hr_report(&mut self, s: &str) {
        let p = self.hr_path("replay.txt");
        let mut b = String::from(s);
        b.push('\n');
        super::perm::trusted(|| plat::write_file(&p, b.as_bytes()));
    }

    // the carried state again: the style, the text scale, the inspector
    fn hr_carry_apply(&mut self, app: &mut dyn App) {
        let Some(ini) = self.m.hr.carry.take() else { return };
        if self.headless {
            let g = |k: &str| ini.get_int("ui", k, 0) as i32;
            self.hr_apply_size(app, g("w"), g("h"), g("dpi"));
        }
        if let Some(s) = ini.get("ui", "style").map(String::from) {
            if s != self.style_name() {
                self.set_style(&s);
            }
        }
        let ts = ini.get_int("ui", "text_scale", 0);
        if ts > 0 {
            self.set_text_scale(ts as u32);
        }
        if ini.get("inspector", "armed").is_some() {
            let g = |k: &str| ini.get_int("inspector", k, 0) as i32;
            let sel = ini.get("inspector", "select").and_then(unhex).and_then(|b| String::from_utf8(b).ok());
            self.run_passes(app);
            self.ins_restore(app, g("armed") != 0, g("open") != 0, g("tab"), [g("outlines"), g("order"), g("flash")], sel.as_deref());
        }
        self.need = false;
        self.run_passes(app);
    }

    // ------------------------------------------------------------------ recording

    fn hr_open_log(&mut self, keep: bool) {
        let path = self.hr_path("events.log");
        let mode = if keep { plat::FO_RW } else { plat::FO_WRITE };
        let Some(f) = super::perm::trusted(|| plat::File::open(&path, mode)) else {
            plat::log_warn("app", &format!("hot reload: {} cannot be written; nothing is recorded", path));
            self.m.hr.phase = PH_OFF;
            return;
        };
        self.m.hr.log_len = f.size().unwrap_or(0);
        self.m.hr.log = Some(f);
        self.m.hr.phase = PH_REC;
        self.m.hr.size = (0, 0, 0);
        if !keep {
            self.hr_write("# jfgui hot reload log 1\n");
        }
        self.hr_size_note();
        self.hr_flush();
        // the keyboard as the replay left it and the real state set it (an `a` record repeats
        // a state harmlessly)
        let t = self.now_ms;
        let on = self.app_active;
        self.hr_write(&format!("a {} {}\n", t, on as u32));
    }

    // append to the log
    fn hr_write(&mut self, s: &str) {
        let h = &mut self.m.hr;
        let Some(f) = &h.log else { return };
        let mut b = s.as_bytes();
        while !b.is_empty() {
            match f.write_at(h.log_len, b) {
                Some(n) if n > 0 => {
                    h.log_len += n as u64;
                    b = &b[n..];
                }
                _ => {
                    plat::log_warn("app", "hot reload: the log cannot be written; nothing more is recorded");
                    h.log = None;
                    return;
                }
            }
        }
    }

    // the records waiting for the batch's end, written
    fn hr_flush(&mut self) {
        let h = &mut self.m.hr;
        if h.pend.is_empty() {
            return;
        }
        let mut s = h.pend.join("\n");
        s.push('\n');
        if h.batch > 0 {
            s.push_str("b\n");
        }
        h.pend.clear();
        h.batch = 0;
        h.commit = false;
        self.hr_write(&s);
    }

    // an `s` record when the main window's size or dpi changed since the last one
    fn hr_size_note(&mut self) {
        let fb = self.win_fb(0);
        let s = (fb.w, fb.h, self.win_dpi(0));
        if s.0 > 0 && s.1 > 0 && s != self.m.hr.size {
            self.m.hr.size = s;
            let r = format!("s {} {} {} {}", self.now_ms, s.0, s.1, s.2);
            self.m.hr.pend.push(r);
        }
    }

    // input that cannot be replayed: the log is not replayed (the next start is fresh)
    fn hr_taint(&mut self, why: &str) {
        if self.m.hr.phase != PH_REC || self.m.hr.tainted {
            return;
        }
        self.m.hr.tainted = true;
        self.hr_flush();
        self.hr_write(&format!("! {}\n", why));
        plat::write_stderr(format!("hot reload: {}: the next start begins afresh\n", why).as_bytes());
    }

    /// (process_events: an event taken from the queue, before anything else) while
    /// replaying: is it one of the replayed batch; while recording: a window's close button,
    /// drops of other programs
    pub(crate) fn hr_popped(&mut self, ev: &Event) {
        let h = &mut self.m.hr;
        if h.phase == PH_REPLAY {
            h.cur = h.left > 0;
            if h.cur {
                h.left -= 1;
            }
            return;
        }
        match ev.typ {
            plat::E_DROP | plat::E_DRAG | plat::E_DRAG_END => self.hr_taint("drag and drop with another program"),
            plat::E_CLOSE => {
                let w = if self.headless { ev.win as i32 } else { self.uiw_from_pw(ev.win as i32) };
                if w < 0 {
                    return;
                }
                let id = self.uiw_rec_id(w as u32);
                if id == super::inspect::INSPECT_WIN {
                    return;
                }
                self.hr_size_note();
                let r = format!("e {} {} 0 {} 0 0 0 {} =", self.now_ms, ev.typ, ev.mods, id);
                self.m.hr.pend.push(r);
                self.m.hr.batch += 1;
            }
            _ => {}
        }
    }

    /// (process_events: an input event goes to window t, before input_event) recorded with
    /// the widget it reaches, or checked against its record
    pub(crate) fn hr_input(&mut self, ev: &Event, t: u32) {
        let chk = match ev.typ {
            plat::E_DOWN | plat::E_WHEEL => {
                let mut e = *ev;
                self.rtl_input(&mut e);
                let (x, y) = if ev.x != -1 { (e.x, e.y) } else { (self.wp.ctx.mx, self.wp.ctx.my) };
                Chk::Id(self.hot_find(x, y).0)
            }
            plat::E_UP => Chk::Id(self.wp.ctx.active),
            plat::E_KEY_DOWN | plat::E_TEXT | plat::E_IME_COMMIT => Chk::Id(self.wp.ctx.focus),
            plat::E_MOVE | plat::E_KEY_UP | plat::E_LEAVE => Chk::None,
            _ => return, // focus out, preedit: no input of the session
        };
        if self.m.hr.phase == PH_REPLAY {
            let h = &mut self.m.hr;
            if !h.cur {
                return; // the platform's own event during the replay
            }
            let bad = match h.expect.pop_front() {
                None => Some(format!("{} that was not recorded", ev_name(ev.typ))),
                Some(Chk::Id(want)) if chk != Chk::Id(want) => {
                    let got = if let Chk::Id(g) = chk { g } else { 0 };
                    Some(format!("{} reached #{} instead of #{}", ev_name(ev.typ), got, want))
                }
                _ => None,
            };
            if let Some(b) = bad {
                h.diverged.get_or_insert(b);
            }
            return;
        }
        let win = self.uiw_rec_id(ev.win as u32);
        if win == super::inspect::INSPECT_WIN || self.uiw_rec_id(t) == super::inspect::INSPECT_WIN {
            return; // the inspector's own window
        }
        self.hr_size_note();
        let c = match chk {
            Chk::Id(id) => format!("{}", id),
            _ => String::from("-"),
        };
        let r = format!("e {} {} {} {} {} {} {} {} {}", self.now_ms, ev.typ, ev.btn, ev.mods, ev.x, ev.y, ev.data, win, c);
        let h = &mut self.m.hr;
        h.pend.push(r);
        h.batch += 1;
        h.commit = ev.typ == plat::E_IME_COMMIT;
    }

    /// (ctx.rs input_event: an input method's commit taken) its text into the record
    pub(crate) fn hr_commit(&mut self, text: &str) {
        let h = &mut self.m.hr;
        if !DEV || h.phase != PH_REC || !h.commit {
            return;
        }
        h.commit = false;
        if let Some(l) = h.pend.last_mut() {
            l.push(' ');
            l.push_str(&hex(text.as_bytes()));
        }
    }

    /// (process_events: the queue is empty) the batch's records written
    pub(crate) fn hr_batch_end(&mut self) {
        if self.m.hr.phase == PH_REC {
            self.hr_flush();
        }
    }

    /// (process_events: the app got or lost the keyboard) -> apply it now; false while
    /// replaying (the replay sets it as recorded, the real state follows afterwards)
    #[inline]
    pub(crate) fn hr_active(&mut self, active: bool) -> bool {
        if !self.hr_on() {
            return true;
        }
        if self.m.hr.phase == PH_REPLAY {
            return false;
        }
        self.hr_flush();
        let t = self.now_ms;
        self.hr_write(&format!("a {} {}\n", t, active as u32));
        true
    }

    /// (filedlg, dnd: the system's dialog or another program would take part) -> take the
    /// app's own way (true while replaying); while recording the log can no longer be replayed
    pub(crate) fn hr_outside(&mut self, what: &str) -> bool {
        if !self.hr_on() {
            return false;
        }
        if self.m.hr.phase == PH_REPLAY {
            return true;
        }
        self.hr_taint(what);
        false
    }

    /// (edit.rs clip_get while hot reload runs) recording: the clipboard read and its answer
    /// noted; replaying: the answer of the same read when it was recorded
    pub(crate) fn hr_clip_get(&mut self, fmt: u32) -> Option<Vec<u8>> {
        if self.m.hr.in_log {
            return match self.m.hr.clips.pop_front() {
                Some((f, d)) if f == fmt => d,
                _ => {
                    self.m.hr.diverged.get_or_insert(String::from("a clipboard read that was not recorded"));
                    None
                }
            };
        }
        let r = self.clip_read(fmt);
        if self.m.hr.phase == PH_REC {
            if r.as_ref().is_some_and(|d| d.len() > CLIP_MAX) {
                self.hr_taint("a large clipboard content");
            } else {
                let c = format!("c {} {}", fmt, r.as_deref().map_or(String::from("-"), hex));
                self.m.hr.pend.push(c);
            }
        }
        r
    }

    // ------------------------------------------------------------------ replay

    // the log replayed -> the number of events (Err: why it diverged)
    fn hr_replay(&mut self, app: &mut dyn App, log: &str) -> Result<u32, String> {
        let armed = self.m.ins.armed;
        self.m.ins.armed = false; // its hot keys and pick would take the replayed input
        self.m.hr.in_log = true;
        let r = self.hr_replay_lines(app, log);
        self.m.ins.armed = armed;
        let h = &mut self.m.hr;
        h.in_log = false;
        h.left = 0;
        h.cur = false;
        h.expect.clear();
        h.clips.clear();
        h.diverged = None;
        r
    }

    fn hr_replay_lines(&mut self, app: &mut dyn App, log: &str) -> Result<u32, String> {
        let mut batch: Vec<Rec> = Vec::new();
        let mut n = 0u32;
        for (k, line) in log.lines().enumerate() {
            let f: Vec<&str> = line.split(' ').collect();
            let bad = || format!("record {} of the log is broken", k + 1);
            match f[0] {
                "e" => {
                    let v: Option<Vec<i64>> = (1..=8).map(|i| num(&f, i)).collect();
                    let v = v.ok_or_else(bad)?;
                    let chk = match f.get(9).copied() {
                        Some("-") => Chk::None,
                        Some("=") => Chk::Outside,
                        Some(c) => Chk::Id(c.parse().map_err(|_| bad())?),
                        None => return Err(bad()),
                    };
                    let text = match f.get(10) {
                        Some(h) => Some(unhex(h).and_then(|b| String::from_utf8(b).ok()).ok_or_else(bad)?),
                        None => None,
                    };
                    if batch.is_empty() {
                        self.hr_advance(app, v[0] as u32)?;
                    }
                    let ev = Event { typ: v[1] as u8, btn: v[2] as u8, mods: v[3] as u8, win: 0, x: v[4] as i32, y: v[5] as i32, data: v[6] as u32 };
                    batch.push(Rec { ev, win: v[7] as u32, chk, text });
                }
                "b" => n += self.hr_batch(app, &mut batch)?,
                "s" => {
                    n += self.hr_batch(app, &mut batch)?;
                    let v: Option<Vec<i64>> = (1..=4).map(|i| num(&f, i)).collect();
                    let v = v.ok_or_else(bad)?;
                    self.hr_advance(app, v[0] as u32)?;
                    self.hr_apply_size(app, v[1] as i32, v[2] as i32, v[3] as i32);
                }
                "a" => {
                    n += self.hr_batch(app, &mut batch)?;
                    let (t, on) = (num(&f, 1).ok_or_else(bad)?, num(&f, 2).ok_or_else(bad)?);
                    self.hr_advance(app, t as u32)?;
                    self.hr_apply_active(app, on != 0);
                }
                "c" => {
                    let fmt = num(&f, 1).ok_or_else(bad)? as u32;
                    let d = match f.get(2).copied() {
                        Some("-") => None,
                        Some(h) => Some(unhex(h).ok_or_else(bad)?),
                        None => return Err(bad()),
                    };
                    self.m.hr.clips.push_back((fmt, d));
                }
                "x" => {
                    n += self.hr_batch(app, &mut batch)?;
                    self.hr_advance(app, num(&f, 1).ok_or_else(bad)? as u32)?;
                }
                _ => {} // comments
            }
            if let Some(d) = self.m.hr.diverged.take() {
                return Err(d);
            }
        }
        n += self.hr_batch(app, &mut batch)?; // the last batch of a run that was killed
        Ok(n)
    }

    // the clock to t as the window loop runs it: every timer at its deadline, the passes it
    // needs; animations jump to the time
    fn hr_advance(&mut self, app: &mut dyn App, t: u32) -> Result<(), String> {
        let mut rounds = 0;
        loop {
            let (due, next) = self.timer_scan();
            let at = if due.is_some() { self.now_ms } else { next };
            if at == u32::MAX || at > t {
                break;
            }
            if at > self.now_ms {
                self.set_time(at);
            }
            self.tick_timers();
            self.need = false;
            self.run_passes(app);
            rounds += 1;
            if rounds > TIMERS_MAX {
                return Err(String::from("timers fire without end"));
            }
        }
        if t > self.now_ms {
            self.set_time(t);
        }
        if self.need || self.animating || self.tasks_ready() {
            self.need = false;
            self.run_passes(app);
        }
        Ok(())
    }

    // a batch of recorded events through ui_process_events -> the number of events
    fn hr_batch(&mut self, app: &mut dyn App, batch: &mut Vec<Rec>) -> Result<u32, String> {
        if batch.is_empty() {
            return Ok(0);
        }
        self.process_events_deferred(app, false); // whatever the platform queued goes first
        let mut expect = VecDeque::new();
        let mut left = 0;
        for r in batch.drain(..) {
            let i = self.uiw_index(r.win);
            if i < 0 || (i != 0 && !self.uiw_drawable(i as u32)) {
                return Err(format!("{} for window {}, which is not open", ev_name(r.ev.typ), r.win));
            }
            let w = if self.headless { i } else { self.uiw_pw(i as u32) };
            if w < 0 {
                return Err(format!("window {} has no window of the system", r.win));
            }
            let mut ev = r.ev;
            ev.win = w as u8;
            let queued = match &r.text {
                Some(t) if t.is_empty() => continue,
                Some(t) => plat::ime_commit_set(ev.win, t),
                None => plat::evq_push(ev),
            };
            if !queued {
                return Err(String::from("too many events at once"));
            }
            left += 1;
            if r.chk != Chk::Outside {
                expect.push_back(r.chk);
            }
        }
        self.m.hr.left = left;
        self.m.hr.expect = expect;
        self.process_events_deferred(app, false);
        let h = &mut self.m.hr;
        h.left = 0;
        h.cur = false;
        if let Some(d) = h.diverged.take() {
            return Err(d);
        }
        if !h.expect.is_empty() {
            h.expect.clear();
            return Err(String::from("an event did not reach the app"));
        }
        Ok(left)
    }

    // an `s` record: the main window's dpi and size
    fn hr_apply_size(&mut self, app: &mut dyn App, w: i32, h: i32, dpi: i32) {
        if dpi >= 24 && dpi != self.win_dpi(0) {
            self.uiw_dpi(0, dpi);
        }
        if w > 0 && h > 0 {
            self.uiw_resize(app, 0, w, h);
        }
        if self.need {
            self.need = false;
            self.run_passes(app);
        }
    }

    // the app got / lost the keyboard (an `a` record; after the replay: the real state)
    fn hr_apply_active(&mut self, app: &mut dyn App, on: bool) {
        if on == self.app_active {
            return;
        }
        self.app_active = on;
        if !on {
            // as process_events: the popups of every window close; the keys and buttons held
            // when the keyboard went are released
            self.uiw_popups_dismiss(WIN_MAX as u32);
            self.keydown = [0; 256];
            self.wp.ctx.mbtn = 0;
        }
        app.command(self, UIC_ACTIVE, on as u32);
        self.run_passes(app);
    }

    // ------------------------------------------------------------------ the watcher

    /// (the window loop) the time of the next look at the watcher's files: the loop waits no
    /// longer than that
    pub(crate) fn hr_deadline(&self) -> u32 {
        let at = self.m.hr.poll_at;
        if (at.wrapping_sub(self.now_ms) as i32) < 0 { self.now_ms } else { at }
    }

    /// (the window loop, every POLL_MS while hot reload runs) the watcher's files: "quit" in
    /// ctl ends the app as --auto-close does; build.txt: a failed build's error
    pub(crate) fn hr_poll(&mut self) {
        if (self.now_ms.wrapping_sub(self.m.hr.poll_at) as i32) < 0 {
            return;
        }
        self.m.hr.poll_at = self.now_ms.wrapping_add(POLL_MS);
        let ctl = self.hr_path("ctl");
        if let Some(d) = super::perm::trusted(|| plat::read_file(&ctl)) {
            super::perm::trusted(|| plat::delete(&ctl));
            if d.starts_with(b"quit") {
                self.quit = true;
            }
        }
        self.hr_poll_files();
    }

    // build.txt: a failed build opens the inspector with its error; a build that worked
    // closes it again when it was opened for the error
    fn hr_poll_files(&mut self) {
        let p = self.hr_path("build.txt");
        let raw = super::perm::trusted(|| plat::read_file(&p)).unwrap_or_default();
        if raw == self.m.hr.build_raw {
            return;
        }
        let text = String::from_utf8_lossy(&raw).trim().replace('\r', "");
        self.m.hr.build_raw = raw;
        self.m.hr.note_hidden = false;
        if text.is_empty() {
            self.m.hr.build_err = None;
            if self.m.hr.auto_open {
                self.m.hr.auto_open = false;
                self.inspect_show(false);
            }
        } else {
            self.m.hr.build_err = Some(text);
            if !self.inspect_open() {
                self.m.hr.auto_open = true;
                self.inspect_show(true);
            }
        }
        self.invalidate();
    }

    /// (inspect.rs: the top of the inspector's window) a failed build: its error
    pub(crate) fn hr_ins_note(&mut self) {
        if !self.hr_on() || self.m.hr.note_hidden {
            return;
        }
        let Some(err) = self.m.hr.build_err.clone() else { return };
        if self.infobar(ID_NOTE, IB_ERROR, tr("Bau fehlgeschlagen"), tr("Die laufende Fassung bleibt.")) & EV_ACT != 0 {
            self.m.hr.note_hidden = true;
            self.repass();
        }
        let lines: Vec<&str> = err.lines().take(ERR_LINES).collect();
        self.text(TS_CAPTION | TX_WRAP, &lines.join("\n"));
    }
}
