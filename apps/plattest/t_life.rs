// t_life -- lifecycle events of the platform (lib/plat/linux_life.rs)
//
//   plattest --test life-bus   (Linux; tools/platshared/life_bus.py starts it between a fake
//                              system bus and a fake session bus) logind's PrepareForSleep(true /
//                              false) -> E_POWER 1 / 0; the settings portal's SettingChanged ->
//                              E_SETTINGS SET_THEME and SET_FONTS, an unrelated key -> nothing;
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
    use plat::{E_POWER, E_SESSION_END, E_SETTINGS, SET_FONTS, SET_THEME};

    plat::evq_clear();
    plat::life_start();
    let ev = collect(5000, |v| v.iter().filter(|e| e.typ == E_SETTINGS).count() >= 2 && v.iter().filter(|e| e.typ == E_POWER).count() >= 2);
    let power: alloc::vec::Vec<u32> = ev.iter().filter(|e| e.typ == E_POWER).map(|e| e.data).collect();
    let settings: alloc::vec::Vec<u32> = ev.iter().filter(|e| e.typ == E_SETTINGS).map(|e| e.data).collect();
    t.check(power == [1, 0], "life-bus: PrepareForSleep -> E_POWER 1, then 0");
    t.check(settings == [SET_THEME, SET_FONTS], "life-bus: SettingChanged -> SET_THEME, SET_FONTS (the unrelated key gives nothing)");
    crate::out("  life-bus: waiting for SIGTERM\n");
    let ev = collect(5000, |v| v.iter().any(|e| e.typ == E_SESSION_END));
    t.check(ev.iter().any(|e| e.typ == E_SESSION_END), "life-bus: SIGTERM -> E_SESSION_END, the process lives on");
    crate::out("life-bus: done\n");
}
