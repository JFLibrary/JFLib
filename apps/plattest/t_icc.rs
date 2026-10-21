// t_icc -- (plat-shared) plattest --test icc: the monitor colour profile without a window
// (Windows: the primary monitor's profile, if it has one, is valid; window 0 without a
// window -> none; Linux headless -> none, key 0).  "  icc: ..." prints its size and a hash
// (tools/platshared compares them with the assembly's codectest op 325).
//                          [ASMLIB/src/plat/win/win_icc.s, linux/lx_icc.s]

use alloc::format;

use plat::{ICC_MON_PRIMARY, monitor_icc, monitor_key};

use crate::T;

fn fnv(b: &[u8]) -> u64 {
    b.iter().fold(0xCBF2_9CE4_8422_2325u64, |h, &c| (h ^ c as u64).wrapping_mul(0x100_0000_01B3))
}

pub fn t_icc(t: &mut T) {
    let k = monitor_key(ICC_MON_PRIMARY);
    let p = monitor_icc(ICC_MON_PRIMARY);
    match &p {
        Some(d) => {
            t.info(&format!("  icc: primary monitor: {} bytes, fnv {:016x}, key {:x}\n", d.len(), fnv(d), k));
            let head: alloc::string::String = d.iter().take(128).map(|b| format!("{b:02x}")).collect();
            t.info(&format!("  icc: head {head}\n"));
        }
        None => t.info(&format!("  icc: primary monitor: none, key {k:x}\n")),
    }
    if cfg!(windows) {
        t.check(k != 0, "icc: plat_monitor_key(-1) = the primary monitor");
        t.check(p.as_ref().is_none_or(|d| d.len() >= 132 && &d[36..40] == b"acsp"), "icc: a profile of the primary monitor is valid");
    } else {
        t.check(k == 0 && p.is_none(), "icc: Linux without a display: no profile, key 0");
    }
    t.check(monitor_key(0) == 0 && monitor_icc(0).is_none(), "icc: window 0 without a window: none");
}

/// (Linux, X11) plattest --test iccx11: an unmapped window of our own makes the X11 backend
/// active; then the root window's _ICC_PROFILE (tools/platshared/icc_x11.py sets it on
/// the X server for the run) -- "  icc: window 0: N bytes, fnv X" or "none"; the window is
/// destroyed right away (never shown)
#[cfg(windows)]
pub fn t_iccx11(_: &mut T) {
    plat::write_stderr(b"plattest: --test iccx11 is Linux only\n");
    plat::exit(2);
}

#[cfg(target_os = "linux")]
pub fn t_iccx11(t: &mut T) {
    let w = plat::win_create("plattest icc", 160, 120, 0, plat::PW_NOOWNER, plat::PW_DEFAULT, plat::PW_DEFAULT);
    t.check(w == 0 && plat::disp_backend() == plat::BK_X11, "iccx11: an X11 window (never shown)");
    let a = monitor_icc(0);
    let b = monitor_icc(0); // the atom is cached now
    match &a {
        Some(d) => t.info(&format!("  icc: window 0: {} bytes, fnv {:016x}\n", d.len(), fnv(d))),
        None => t.info("  icc: window 0: none\n"),
    }
    t.check(a == b, "iccx11: the same answer twice");
    t.check(monitor_key(0) == 0, "iccx11: plat_monitor_key = 0 on Linux");
    plat::win_destroy(w);
}
