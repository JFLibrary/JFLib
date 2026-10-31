// t_life -- lifecycle events of the platform (lib/plat/linux_life.rs)
//
//   plattest --test life-bus   (Linux; tools/platshared/life_bus.py starts it between a fake
//                              system bus and a fake session bus) logind's PrepareForSleep(true /
//                              false) -> E_POWER 1 / 0; the settings portal's SettingChanged ->
//                              E_SETTINGS SET_THEME, SET_FONTS and SET_OTHER (button-layout), an
//                              unrelated key -> nothing; the settings files (no portal answer):
//                              the script renames a new kdeglobals over the old one -> one
//                              E_SETTINGS SET_THEME | SET_OTHER, an unrelated file -> nothing;
//                              then the script sends SIGTERM -> E_SESSION_END (the process
//                              lives on and ends normally)

use crate::T;

#[cfg(windows)]
pub fn t_life_bus(_: &mut T) {
    plat::write_stderr(b"plattest: --test life-bus is Linux only\n");
    plat::exit(2);
}

// the events of the next ms milliseconds (or until `done` holds)
#[cfg(target_os = "linux")]
fn collect(ms: u64, done: impl Fn(&[plat::Event]) -> bool) -> alloc::vec::Vec<plat::Event> {
    let mut v = alloc::vec::Vec::new();
    let end = plat::time_us() + ms * 1000;
    while plat::time_us() < end && !done(&v) {
        plat::wait(end.min(plat::time_us() + 50_000));
        while let Some(e) = plat::evq_pop() {
            v.push(e);
        }
    }
    v
}

#[cfg(target_os = "linux")]
pub fn t_life_bus(t: &mut T) {
    use plat::{E_POWER, E_SESSION_END, E_SETTINGS, SET_FONTS, SET_OTHER, SET_THEME};

    plat::evq_clear();
    // the portal does not answer ReadAll: the settings come from the files (kdeglobals)
    let s = plat::sys_settings();
    t.check(s.dark == Some(false), "life-bus: no portal answer: kdeglobals BreezeLight -> light");
    plat::life_start();
    let ev = collect(5000, |v| v.iter().filter(|e| e.typ == E_SETTINGS).count() >= 3 && v.iter().filter(|e| e.typ == E_POWER).count() >= 2);
    let power: alloc::vec::Vec<u32> = ev.iter().filter(|e| e.typ == E_POWER).map(|e| e.data).collect();
    let settings: alloc::vec::Vec<u32> = ev.iter().filter(|e| e.typ == E_SETTINGS).map(|e| e.data).collect();
    t.check(power == [1, 0], "life-bus: PrepareForSleep -> E_POWER 1, then 0");
    t.check(settings == [SET_THEME, SET_FONTS, SET_OTHER], "life-bus: SettingChanged -> SET_THEME, SET_FONTS, SET_OTHER (the unrelated key gives nothing)");
    crate::out("  life-bus: waiting for a file\n");
    let ev = collect(5000, |v| v.iter().any(|e| e.typ == E_SETTINGS));
    let ev2 = collect(500, |_| false);
    let files: alloc::vec::Vec<u32> = ev.iter().chain(ev2.iter()).filter(|e| e.typ == E_SETTINGS).map(|e| e.data).collect();
    t.check(files == [SET_THEME | SET_OTHER], "life-bus: kdeglobals renamed into place -> one E_SETTINGS SET_THEME | SET_OTHER");
    t.check(plat::sys_settings().dark == Some(true), "life-bus: the new kdeglobals (BreezeDark) -> dark");
    crate::out("  life-bus: waiting for SIGTERM\n");
    let ev = collect(5000, |v| v.iter().any(|e| e.typ == E_SESSION_END));
    t.check(ev.iter().any(|e| e.typ == E_SESSION_END), "life-bus: SIGTERM -> E_SESSION_END, the process lives on");
    crate::out("life-bus: done\n");
}
