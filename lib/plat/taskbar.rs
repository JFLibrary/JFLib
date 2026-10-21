// taskbar -- the app's button in the taskbar or dock: progress, a badge, the jump list's tasks
//
//   taskbar_progress(TaskbarProgress) -> bool   None, Normal(f), Paused(f), Error(f) or
//                                    Indeterminate (f: 0.0 ..= 1.0)
//   taskbar_badge(count, icon) -> bool   count 0 = no badge; icon: the overlay Windows shows on
//                                    the button (straight ARGB, 16 x 16 at 100 %; ui draws one
//                                    with the number), Linux docks show the count themselves
//   taskbar_tasks(&[JumpTask]) -> bool   the "Tasks" of the button's jump list (Windows): each
//                                    starts this program with its arguments -- with
//                                    AF_SINGLE_INSTANCE they reach the running app as
//                                    UIC_OPEN; empty = the jump list removed.  Linux: false
//   taskbar_state() -> (TaskbarProgress, u32)   what was set last (progress, count)
//   taskbar_badge_shown(count) -> bool   that badge (with its icon) is up already (ui skips
//                                    drawing it again)
//   taskbar_supported() -> bool      whether this system shows any of it
//
// The calls keep the state and return whether the system took it (Windows: also before the
// window has its button -- it is applied when the button appears).  Setting what is already
// shown does nothing (progress compared in per mille), so calling them every frame is cheap.
// The tray's dry run
// (tray::set_dry_run, jfgui_TRAY_DRYRUN=1; the UI's headless runs) prints "taskbar: ..."
// instead.
//
// Windows (win_taskbar.rs): ITaskbarList3 on the main window (SetProgressState,
// SetProgressValue, SetOverlayIcon with the count as its text), applied again on the
// registered message "TaskbarButtonCreated" (the first show, Explorer restarted);
// ICustomDestinationList with IShellLinkW tasks (title through IPropertyStore PKEY_Title).
// COM on the UI thread: call these from the UI thread.  Linux (linux_taskbar.rs): the signal
// com.canonical.Unity.LauncherEntry.Update(application://<desktop id>.desktop, {progress,
// progress-visible, count, count-visible}) on a session-bus connection kept open (docks drop
// the state when it closes; Query is answered), understood by KDE Plasma, Ubuntu's dock /
// Dash to Dock and Plank.  Linux knows no paused / error / indeterminate look (paused and
// error show the bar, indeterminate none) and no jump list at run time (that is the .desktop
// file's Actions).  Web and Android: nothing (false).

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU8, Ordering};

use rt::sync::SpinLock;

use crate::tray::TrayIcon;

/// what the taskbar button's progress bar shows
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum TaskbarProgress {
    /// no bar
    #[default]
    None,
    /// done so far (0.0 ..= 1.0)
    Normal(f32),
    /// paused at (Windows: yellow)
    Paused(f32),
    /// failed at (Windows: red)
    Error(f32),
    /// busy without a known end (Windows: a moving bar)
    Indeterminate,
}

impl TaskbarProgress {
    /// the fraction done, clamped to 0.0 ..= 1.0 (None / Indeterminate: 0.0)
    pub fn value(&self) -> f32 {
        match *self {
            TaskbarProgress::Normal(v) | TaskbarProgress::Paused(v) | TaskbarProgress::Error(v) if v.is_finite() => v.clamp(0.0, 1.0),
            _ => 0.0,
        }
    }

    fn text(&self) -> String {
        let pct = (self.value() * 100.0 + 0.5) as u32;
        match self {
            TaskbarProgress::None => String::from("none"),
            TaskbarProgress::Normal(_) => alloc::format!("normal {pct}%"),
            TaskbarProgress::Paused(_) => alloc::format!("paused {pct}%"),
            TaskbarProgress::Error(_) => alloc::format!("error {pct}%"),
            TaskbarProgress::Indeterminate => String::from("indeterminate"),
        }
    }
}

/// one task of the jump list: starts this program with args
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JumpTask {
    /// the row's text
    pub title: String,
    /// the arguments after the program
    pub args: Vec<String>,
    /// its tooltip ("" = none)
    pub desc: String,
}

/// the state the platform code applies (again)
#[derive(Clone, Default)]
pub(crate) struct State {
    pub(crate) progress: TaskbarProgress,
    pub(crate) count: u32,
    pub(crate) icon: Option<TrayIcon>,
}

static STATE: SpinLock<Option<State>> = SpinLock::new(None);

/// (platform) a copy of the state
#[allow(dead_code)]
pub(crate) fn state() -> State {
    STATE.lock().clone().unwrap_or_default()
}

// -> (the state before, after)
fn update(f: impl FnOnce(&mut State)) -> (State, State) {
    let mut g = STATE.lock();
    let s = g.get_or_insert_with(State::default);
    let old = s.clone();
    f(s);
    (old, s.clone())
}

// the answer of the last progress / badge call: 0 none yet, 1 false, 2 shown, 3 printed (dry run)
static SHOWN: [AtomicU8; 2] = [AtomicU8::new(0), AtomicU8::new(0)];
const K_PROGRESS: usize = 0;
const K_BADGE: usize = 1;

// whether the call can be skipped: nothing changed and the last answer stands
fn unchanged(k: usize, same: bool, dry: bool) -> bool {
    same && SHOWN[k].load(Ordering::Relaxed) == if dry { 3 } else { 2 }
}

fn answered(k: usize, ok: bool, dry: bool) -> bool {
    SHOWN[k].store(if !ok { 1 } else if dry { 3 } else { 2 }, Ordering::Relaxed);
    ok
}

// what the progress looks like: its kind and per mille
fn look(p: TaskbarProgress) -> (u8, u32) {
    let kind = match p {
        TaskbarProgress::None => 0,
        TaskbarProgress::Normal(_) => 1,
        TaskbarProgress::Paused(_) => 2,
        TaskbarProgress::Error(_) => 3,
        TaskbarProgress::Indeterminate => 4,
    };
    (kind, (p.value() * 1000.0 + 0.5) as u32)
}

fn dry(line: &str) {
    crate::write_stdout(alloc::format!("taskbar: {line}\n").as_bytes());
}

/// show progress on the app's taskbar button -> false: not shown here
pub fn taskbar_progress(p: TaskbarProgress) -> bool {
    let p = match p {
        TaskbarProgress::Normal(_) => TaskbarProgress::Normal(p.value()),
        TaskbarProgress::Paused(_) => TaskbarProgress::Paused(p.value()),
        TaskbarProgress::Error(_) => TaskbarProgress::Error(p.value()),
        other => other,
    };
    let (old, s) = update(|s| s.progress = p);
    let is_dry = crate::tray::dry_run();
    if unchanged(K_PROGRESS, look(old.progress) == look(p), is_dry) {
        return true;
    }
    if is_dry {
        dry(&alloc::format!("progress {}", p.text()));
        return answered(K_PROGRESS, true, true);
    }
    answered(K_PROGRESS, native::progress(&s), false)
}

/// a badge on the button: count (0 = none), icon = Windows' overlay (None: no overlay there)
pub fn taskbar_badge(count: u32, icon: Option<&TrayIcon>) -> bool {
    let (old, s) = update(|s| {
        s.count = count;
        s.icon = if count == 0 { None } else { icon.cloned() };
    });
    let is_dry = crate::tray::dry_run();
    if unchanged(K_BADGE, old.count == s.count && old.icon == s.icon, is_dry) {
        return true;
    }
    if is_dry {
        match &s.icon {
            _ if count == 0 => dry("badge none"),
            Some(i) => dry(&alloc::format!("badge {count} icon {}x{}", i.w, i.h)),
            None => dry(&alloc::format!("badge {count}")),
        }
        return answered(K_BADGE, true, true);
    }
    answered(K_BADGE, native::badge(&s), false)
}

/// the tasks of the button's jump list (empty: none) -> false: no jump list here
pub fn taskbar_tasks(tasks: &[JumpTask]) -> bool {
    if crate::tray::dry_run() {
        let rows: Vec<String> = tasks.iter().map(|t| alloc::format!("{}=[{}]", t.title, t.args.join(" "))).collect();
        dry(&alloc::format!("tasks {}", if rows.is_empty() { String::from("none") } else { rows.join(",") }));
        return true;
    }
    native::tasks(tasks)
}

/// whether a badge with count is shown already (with its icon): the UI then need not draw it
/// again
pub fn taskbar_badge_shown(count: u32) -> bool {
    let s = state();
    let want = if crate::tray::dry_run() { 3 } else { 2 };
    s.count == count && s.icon.is_some() && SHOWN[K_BADGE].load(Ordering::Relaxed) == want
}

/// what was set last: the progress and the badge's count
pub fn taskbar_state() -> (TaskbarProgress, u32) {
    let s = state();
    (s.progress, s.count)
}

/// whether this system shows the taskbar progress and badge
pub fn taskbar_supported() -> bool {
    crate::tray::dry_run() || native::supported()
}

/// (win_api.rs) a message of the main window: "TaskbarButtonCreated" applies the state again
#[cfg(windows)]
pub(crate) fn window_message(msg: u32) {
    native::window_message(msg)
}

#[cfg(windows)]
#[path = "win_taskbar.rs"]
mod native;

#[cfg(target_os = "linux")]
#[path = "linux_taskbar.rs"]
mod native;

#[cfg(not(any(windows, target_os = "linux")))]
mod native {
    use super::{JumpTask, State};
    pub fn progress(_s: &State) -> bool {
        false
    }
    pub fn badge(_s: &State) -> bool {
        false
    }
    pub fn tasks(_t: &[JumpTask]) -> bool {
        false
    }
    pub fn supported() -> bool {
        false
    }
}
