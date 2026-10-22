// t_burst -- bursts of synthetic keys beyond the 256-event ring (X11 and Wayland)
//
//   linuxtest --test burst [--keys N]      (tools/keyburst.py: a proxy between linuxtest and
//                                          WSLg's X server / compositor plays the keys)
//
// The title "linuxtest burst" arms the proxy; at the next round trip (X11 GetInputFocus,
// Wayland wl_display.sync) it writes N key presses and releases (the digit keys 1 .. 0 in
// turn) before passing the request on, so they arrive in one read while the backend waits
// for the reply and must dispatch through them.  "linuxtest burst 2" makes it write them at
// once while linuxtest only waits for events.  Every key must come, in order: E_KEY_DOWN,
// E_TEXT, E_KEY_UP per key (3 N events, far more than the ring holds), none dropped.

use alloc::format;
use alloc::vec::Vec;

use plat::{E_KEY_DOWN, E_KEY_UP, E_TEXT, Event};

use crate::{Lt, out};

const DIGITS: &[u8; 10] = b"1234567890";

pub fn test_burst(t: &mut Lt) {
    if t.headless {
        t.check(false, "burst test needs a window (run without --shot)");
        return;
    }
    let n = t.arg_int("--keys", 1000).max(1) as usize;
    let wayland = plat::disp_backend() == 1;
    out(&format!("backend {}, {} keys per burst\n", if wayland { "wayland" } else { "x11" }, n));
    t.redraw();
    t.pump(1500, 0); // mapped, configured, the first frame shown
    let dropped = plat::evq_dropped();
    let mut keys = Vec::new();
    // 1: the burst arrives while the backend waits for a round trip's answer
    plat::set_title("linuxtest burst");
    if wayland {
        t.check(plat::wl_test_roundtrip() == 0, "burst 1: the round trip is answered");
    } else {
        plat::x11t::sync();
    }
    let queued = plat::evq_count() as usize;
    out(&format!("burst 1: {queued} events queued when the round trip returned\n"));
    take_keys(&mut keys);
    t.check(queued >= 3 * n, "burst 1: every key was queued while waiting for the reply (more than the ring holds)");
    check_keys(t, "burst 1", &keys, n);
    // 2: the same while linuxtest only waits for events
    plat::set_title("linuxtest burst 2");
    keys.clear();
    let (deadline, mut most) = (plat::time_us() + 5_000_000, 0);
    while keys.len() < 3 * n && plat::time_us() < deadline {
        most = most.max(plat::wait(plat::time_us() + 100_000));
        take_keys(&mut keys);
    }
    out(&format!("burst 2: at most {most} events queued at once\n"));
    check_keys(t, "burst 2", &keys, n);
    t.check(plat::evq_dropped() == dropped, "no event dropped");
}

// take_keys(keys): the key events queued now (others go)
fn take_keys(keys: &mut Vec<Event>) {
    while let Some(e) = plat::evq_pop() {
        if matches!(e.typ, E_KEY_DOWN | E_TEXT | E_KEY_UP) {
            keys.push(e);
        }
    }
}

// check_keys: key k (the digit key DIGITS[k % 10]) as E_KEY_DOWN, E_TEXT, E_KEY_UP, n keys
fn check_keys(t: &mut Lt, name: &str, keys: &[Event], n: usize) {
    let mut bad = None;
    for k in 0..n {
        let c = DIGITS[k % 10] as u32;
        let want = [(E_KEY_DOWN, c), (E_TEXT, c), (E_KEY_UP, c)];
        for (j, &(typ, data)) in want.iter().enumerate() {
            let i = 3 * k + j;
            if keys.get(i).is_none_or(|e| e.typ != typ || e.data != data) {
                bad = bad.or(Some(i));
            }
        }
    }
    let ok = bad.is_none() && keys.len() == 3 * n;
    t.check(ok, &format!("{name}: {n} keys -> {} events, all in order", 3 * n));
    if !ok {
        let at = bad.unwrap_or(keys.len().min(3 * n));
        let e = keys.get(at).copied().unwrap_or_default();
        out(&format!("     got {} key events; at {at}: type {} data {:x}\n", keys.len(), e.typ, e.data));
    }
}
