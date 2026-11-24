// t_power -- keeping the computer awake (lib/plat/power.rs)
//
//   plattest --test power       the contract: no bits / unknown bits refused; Windows: real power
//                               requests (system; system + display for INHIBIT_IDLE) made,
//                               nested and released, 50 in a row (no handle stays); Linux:
//                               without a session bus (DBUS_SESSION_BUS_ADDRESS points nowhere)
//                               every request is None and nothing is counted
//   plattest --test power-bus --mode portal|idleonly|fallback|none   (Linux;
//                               tools/platshared/desktop_bus.py starts it on a fake session bus)
//                               portal: two inhibitions through org.freedesktop.portal.Inhibit,
//                               released; idleonly: a backend that refuses suspend; fallback: the
//                               ScreenSaver / PowerManagement cookies; none: no service answers
//                               -> None (the fake checks the calls, flags, reasons and releases)

use plat::{INHIBIT_IDLE, INHIBIT_SUSPEND, inhibit, inhibited};

use crate::T;

fn contract(t: &mut T) {
    t.check(inhibit("plattest", 0).is_none(), "power: no bits refused");
    t.check(inhibit("plattest", 4).is_none() && inhibit("plattest", INHIBIT_IDLE | 8).is_none(), "power: unknown bits refused");
    t.expect_i(inhibited() as i64, 0, "power: nothing held at the start");
}

pub fn t_power(t: &mut T) {
    contract(t);
    if cfg!(any(windows, target_os = "macos")) {
        let a = inhibit("plattest: power test (system)", INHIBIT_SUSPEND);
        t.check(a.as_ref().is_some_and(|g| g.what() == INHIBIT_SUSPEND), "power: a system power request");
        t.expect_i(inhibited() as i64, INHIBIT_SUSPEND as i64, "power: inhibited() = suspend");
        let b = inhibit("plattest: power test (display)", INHIBIT_IDLE);
        t.check(b.as_ref().is_some_and(|g| g.what() == INHIBIT_IDLE | INHIBIT_SUSPEND), "power: a display request implies the system one");
        t.expect_i(inhibited() as i64, (INHIBIT_IDLE | INHIBIT_SUSPEND) as i64, "power: inhibited() = idle + suspend");
        drop(b);
        t.expect_i(inhibited() as i64, INHIBIT_SUSPEND as i64, "power: the display request released");
        drop(a);
        t.expect_i(inhibited() as i64, 0, "power: all released");
        let mut ok = true;
        for k in 0..50 {
            let g = inhibit("", if k % 2 == 0 { INHIBIT_SUSPEND } else { INHIBIT_IDLE });
            ok &= g.is_some();
        }
        t.check(ok && inhibited() == 0, "power: 50 requests made and released in a row (empty reason = the app id)");
    } else if cfg!(target_os = "ios") {
        ios(t);
    } else {
        let saved = plat::env("DBUS_SESSION_BUS_ADDRESS");
        plat::setenv("DBUS_SESSION_BUS_ADDRESS", Some("unix:path=/nonexistent/plattest/bus"));
        t.check(inhibit("plattest", INHIBIT_SUSPEND).is_none() && inhibit("plattest", INHIBIT_IDLE).is_none(), "power: no session bus -> None");
        t.expect_i(inhibited() as i64, 0, "power: nothing counted without a bus");
        plat::setenv("DBUS_SESSION_BUS_ADDRESS", saved.as_deref());
    }
}

// iOS: INHIBIT_IDLE turns UIApplication's idle timer off while a guard lives; INHIBIT_SUSPEND alone
// has nothing to hold (None).  The timer itself only under UIKit's host (tools/apple.py simapp)
#[cfg(target_os = "ios")]
fn ios(t: &mut T) {
    let app = plat::ios_pad_view().is_some();
    t.check(inhibit("plattest", INHIBIT_SUSPEND).is_none(), "power: iOS: suspend alone -> None (no sleep but the lock)");
    t.expect_i(inhibited() as i64, 0, "power: iOS: nothing counted for it");
    let a = inhibit("plattest: screen on", INHIBIT_IDLE);
    t.check(a.as_ref().is_some_and(|g| g.what() == INHIBIT_IDLE | INHIBIT_SUSPEND), "power: iOS: the screen kept on");
    let b = inhibit("", INHIBIT_IDLE | INHIBIT_SUSPEND);
    t.check(b.is_some(), "power: iOS: a second guard");
    if app {
        t.check(plat::ios_idle_timer_off(), "power: iOS: the idle timer is off");
    }
    drop(a);
    if app {
        t.check(plat::ios_idle_timer_off(), "power: iOS: still off while the second guard lives");
    }
    drop(b);
    t.expect_i(inhibited() as i64, 0, "power: iOS: all released");
    if app {
        t.check(!plat::ios_idle_timer_off(), "power: iOS: the idle timer is on again");
        let ok = (0..50).all(|_| inhibit("", INHIBIT_IDLE).is_some());
        t.check(ok && !plat::ios_idle_timer_off(), "power: iOS: 50 guards made and released in a row, the timer on after");
    } else {
        t.info("  (power: no UIKit host here: the idle timer itself under tools/apple.py simapp)\n");
    }
}

#[cfg(not(target_os = "ios"))]
fn ios(_: &mut T) {}

#[cfg(not(target_os = "linux"))]
pub fn t_power_bus(_: &mut T) {
    plat::write_stderr(b"plattest: --test power-bus is Linux only\n");
    plat::exit(2);
}

#[cfg(target_os = "linux")]
pub fn t_power_bus(t: &mut T) {
    let args = base::Args(plat::args());
    let mode = args.value("--mode").unwrap_or("portal");
    contract(t);
    match mode {
        "portal" | "idleonly" | "fallback" => {
            let a = inhibit("plattest: export", INHIBIT_SUSPEND);
            t.check(a.is_some(), &alloc::format!("power-bus ({mode}): suspend inhibited"));
            let b = inhibit("plattest: film", INHIBIT_IDLE);
            t.check(b.is_some(), &alloc::format!("power-bus ({mode}): idle inhibited"));
            t.expect_i(inhibited() as i64, (INHIBIT_SUSPEND | INHIBIT_IDLE) as i64, &alloc::format!("power-bus ({mode}): inhibited() = both"));
            drop(b);
            t.expect_i(inhibited() as i64, INHIBIT_SUSPEND as i64, &alloc::format!("power-bus ({mode}): idle released"));
            drop(a);
            t.expect_i(inhibited() as i64, 0, &alloc::format!("power-bus ({mode}): all released"));
        }
        _ => {
            t.check(inhibit("plattest: export", INHIBIT_SUSPEND).is_none(), "power-bus (none): suspend not possible");
            t.check(inhibit("plattest: film", INHIBIT_IDLE).is_none(), "power-bus (none): idle not possible");
            t.expect_i(inhibited() as i64, 0, "power-bus (none): nothing counted");
        }
    }
    plat::sleep_ms(200); // the releases reach the bus before the process ends
    crate::out("power-bus: done\n");
}
