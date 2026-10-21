// t_taskbar -- the taskbar / dock button (lib/plat/taskbar.rs)
//
//   plattest --test taskbar       the contract in the dry run (jfgui_TRAY_DRYRUN=1: printed):
//                                 progress states, clamping, the badge with and without icon,
//                                 repeats (printed once), tasks, taskbar_state; then for real without a window:
//                                 Windows keeps the state for the button (true), Linux without
//                                 a session bus says false
//   plattest --test taskbar-bus   (Linux; tools/platshared/desktop_bus.py starts it on a fake
//                                 session bus) the LauncherEntry Update signals of a sequence
//                                 (normal, paused, badge, error, indeterminate, none, badge 0);
//                                 the fake asks Query in between and checks the answer
//   plattest --test taskbar-win   (Windows, by hand: a window shows for about 8 s) the real
//                                 ITaskbarList3 on its button: every progress state, an overlay
//                                 badge, a jump list with two tasks (removed again)

use alloc::string::String;

use plat::taskbar::{JumpTask, TaskbarProgress, taskbar_badge, taskbar_badge_shown, taskbar_progress, taskbar_state, taskbar_supported, taskbar_tasks};
use plat::tray::TrayIcon;

use crate::T;

fn dot(side: u32) -> TrayIcon {
    let r = side as i32 / 2;
    let px = (0..side * side)
        .map(|k| {
            let (x, y) = ((k % side) as i32 - r, (k / side) as i32 - r);
            if x * x + y * y < r * r { 0xFFC4_2B1C } else { 0 }
        })
        .collect();
    TrayIcon { w: side, h: side, px }
}

fn task(title: &str, args: &[&str]) -> JumpTask {
    JumpTask { title: String::from(title), args: args.iter().map(|a| String::from(*a)).collect(), desc: String::from("plattest") }
}

pub fn t_taskbar(t: &mut T) {
    plat::setenv("jfgui_TRAY_DRYRUN", Some("1"));
    t.check(taskbar_supported(), "taskbar: supported in the dry run");
    t.check(taskbar_state() == (TaskbarProgress::None, 0), "taskbar: nothing at the start");
    t.check(taskbar_progress(TaskbarProgress::Normal(0.25)), "taskbar: normal 25 %");
    t.check(taskbar_state().0 == TaskbarProgress::Normal(0.25), "taskbar: the state keeps it");
    t.check(taskbar_progress(TaskbarProgress::Paused(1.7)) && taskbar_state().0 == TaskbarProgress::Paused(1.0), "taskbar: a value above 1 is clamped");
    t.check(taskbar_progress(TaskbarProgress::Error(f32::NAN)) && taskbar_state().0 == TaskbarProgress::Error(0.0), "taskbar: NaN counts as 0");
    t.check(taskbar_progress(TaskbarProgress::Indeterminate) && taskbar_state().0 == TaskbarProgress::Indeterminate, "taskbar: indeterminate");
    t.check(taskbar_badge(3, Some(&dot(16))) && taskbar_state().1 == 3, "taskbar: badge 3 with an icon");
    t.check(taskbar_badge(120, None) && taskbar_state().1 == 120, "taskbar: badge without an icon");
    t.check(taskbar_badge_shown(120) == false, "taskbar: a badge without icon is not 'shown' for the UI");
    t.check(taskbar_badge(4, Some(&dot(16))) && taskbar_badge_shown(4) && !taskbar_badge_shown(5), "taskbar: taskbar_badge_shown");
    t.check(taskbar_badge(4, Some(&dot(16))), "taskbar: the same badge again (not printed twice)");
    t.check(taskbar_progress(TaskbarProgress::Indeterminate), "taskbar: the same progress again (not printed twice)");
    t.check(taskbar_badge(0, Some(&dot(16))) && taskbar_state().1 == 0, "taskbar: badge removed");
    t.check(taskbar_tasks(&[task("Neues Fenster", &["--new"]), task("Zuletzt", &["--recent", "a b"])]), "taskbar: two tasks");
    t.check(taskbar_tasks(&[]), "taskbar: no tasks");
    t.check(taskbar_progress(TaskbarProgress::None) && taskbar_state() == (TaskbarProgress::None, 0), "taskbar: all off again");
    plat::setenv("jfgui_TRAY_DRYRUN", None);
    plat::tray::set_dry_run(false);
    if cfg!(windows) {
        // no window: the state waits for the button
        t.check(taskbar_supported(), "taskbar: ITaskbarList3 is there");
        t.check(taskbar_progress(TaskbarProgress::Normal(0.5)) && taskbar_badge(2, Some(&dot(16))), "taskbar: kept without a window");
        t.check(taskbar_progress(TaskbarProgress::None) && taskbar_badge(0, None), "taskbar: cleared without a window");
    } else {
        let saved = plat::env("DBUS_SESSION_BUS_ADDRESS");
        plat::setenv("DBUS_SESSION_BUS_ADDRESS", Some("unix:path=/nonexistent/plattest/bus"));
        t.check(!taskbar_progress(TaskbarProgress::Normal(0.5)) && !taskbar_badge(2, None), "taskbar: no session bus -> false");
        t.check(!taskbar_tasks(&[task("x", &[])]), "taskbar: no jump list on Linux");
        t.check(taskbar_state() == (TaskbarProgress::Normal(0.5), 2), "taskbar: the state is kept anyway");
        taskbar_progress(TaskbarProgress::None);
        taskbar_badge(0, None);
        plat::setenv("DBUS_SESSION_BUS_ADDRESS", saved.as_deref());
    }
}

#[cfg(not(target_os = "linux"))]
pub fn t_taskbar_bus(_: &mut T) {
    plat::write_stderr(b"plattest: --test taskbar-bus is Linux only\n");
    plat::exit(2);
}

#[cfg(target_os = "linux")]
pub fn t_taskbar_bus(t: &mut T) {
    plat::setenv("jfgui_DESKTOP_ID", Some("plattest-taskbar"));
    let steps: alloc::vec::Vec<(&str, bool)> = alloc::vec![
        ("normal 25 %", taskbar_progress(TaskbarProgress::Normal(0.25))),
        ("paused 50 %", taskbar_progress(TaskbarProgress::Paused(0.5))),
        ("badge 3", taskbar_badge(3, None)),
        ("error 75 %", taskbar_progress(TaskbarProgress::Error(0.75))),
    ];
    for (name, ok) in steps {
        t.check(ok, &alloc::format!("taskbar-bus: {name}"));
    }
    plat::sleep_ms(300); // the fake asks Query now; the next update answers it
    t.check(taskbar_progress(TaskbarProgress::Indeterminate), "taskbar-bus: indeterminate");
    t.check(taskbar_progress(TaskbarProgress::None), "taskbar-bus: none");
    t.check(taskbar_badge(0, None), "taskbar-bus: badge 0");
    plat::sleep_ms(300);
    plat::setenv("jfgui_DESKTOP_ID", None);
    crate::out("taskbar-bus: done\n");
}

#[cfg(not(windows))]
pub fn t_taskbar_win(_: &mut T) {
    plat::write_stderr(b"plattest: --test taskbar-win is Windows only\n");
    plat::exit(2);
}

#[cfg(windows)]
pub fn t_taskbar_win(t: &mut T) {
    use plat::{PW_DEFAULT, PW_NOOWNER, PWF_RESIZABLE, PWF_SYSFRAME, PWS_SHOW, evq_pop, time_us, wait, win_create, win_destroy, win_show};
    let pump = |ms: u64| {
        let dl = time_us() + ms * 1000;
        while time_us() < dl {
            wait(dl);
            while evq_pop().is_some() {}
        }
    };
    // set before the window exists: applied when its button appears
    t.check(taskbar_progress(TaskbarProgress::Normal(0.3)), "taskbar-win: progress before the window");
    let w = win_create("plattest taskbar", 360, 120, PWF_RESIZABLE | PWF_SYSFRAME, PW_NOOWNER, PW_DEFAULT, PW_DEFAULT);
    t.check(w == 0, "taskbar-win: the main window");
    win_show(0, PWS_SHOW);
    pump(1200);
    for (name, p) in [
        ("normal 60 %", TaskbarProgress::Normal(0.6)),
        ("paused 60 %", TaskbarProgress::Paused(0.6)),
        ("error 80 %", TaskbarProgress::Error(0.8)),
        ("indeterminate", TaskbarProgress::Indeterminate),
    ] {
        t.check(taskbar_progress(p), &alloc::format!("taskbar-win: {name}"));
        pump(1000);
    }
    t.check(taskbar_badge(7, Some(&dot(16))), "taskbar-win: overlay badge");
    pump(1500);
    t.check(taskbar_tasks(&[task("Neues Fenster", &["--new"]), task("Zuletzt geöffnet", &["--recent", "a b"])]), "taskbar-win: jump list with two tasks");
    t.info("  (right-click the button now to see the tasks)\n");
    pump(2500);
    t.check(taskbar_tasks(&[]), "taskbar-win: jump list removed");
    t.check(taskbar_progress(TaskbarProgress::None) && taskbar_badge(0, None), "taskbar-win: cleared");
    pump(500);
    win_destroy(0);
    pump(200);
}
