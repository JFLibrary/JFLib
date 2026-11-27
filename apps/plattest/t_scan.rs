// t_scan -- the system's scan dialog (lib/plat/scan.rs)
//
//   plattest --test scan      the contract: scan_available where the system has the dialog (macOS),
//                             elsewhere scan_start is Unavailable; no answer, nothing open
//   plattest --test scan-ui   (macOS, in the logged-in session: tools/apple.py gui) the dialog's
//                             window opens (scan_start Ok, a second one Busy, the window on screen),
//                             closed as by its close button -> Cancelled, nothing open after.  With
//                             jfgui_SCAN_HAND=1 the window stays open for the user, who scans a page
//                             within jfgui_SCAN_WAIT_S (480: a network scanner can take minutes to
//                             connect) -> the file (its name, its size, PNG by default)
//                             Elsewhere it says so.

use alloc::format;

use plat::{ScanError, scan_available, scan_open, scan_poll, scan_start};

use crate::T;

pub fn t_scan(t: &mut T) {
    let want = cfg!(target_os = "macos");
    t.check(scan_available() == want, &format!("scan: available {}", if want { "(ImageKit)" } else { "nowhere but macOS yet" }));
    if !want {
        t.check(scan_start("plattest") == Err(ScanError::Unavailable), "scan: scan_start is Unavailable here");
    }
    t.check(scan_poll().is_none() && !scan_open(), "scan: no answer, nothing open");
}

/// pump the system up to ms until the dialog answers
#[cfg(target_os = "macos")]
fn scan_wait(ms: u64) -> Option<Result<plat::Scanned, ScanError>> {
    let dl = plat::time_us() + ms * 1000;
    loop {
        plat::wait(dl.min(plat::time_us() + 50_000));
        while plat::evq_pop().is_some() {}
        if let Some(r) = scan_poll() {
            return Some(r);
        }
        if plat::time_us() >= dl {
            return None;
        }
    }
}

#[cfg(target_os = "macos")]
pub fn t_scan_ui(t: &mut T) {
    let hand = plat::env("jfgui_SCAN_HAND").is_some();
    t.check(scan_start("plattest: Scannen") == Ok(()), "scan-ui: the dialog opens");
    t.check(scan_open() && scan_start("noch einer") == Err(ScanError::Busy), "scan-ui: a second one is Busy");
    let mut shown = false;
    for _ in 0..60 {
        let _ = scan_wait(50);
        if plat::mac_scan_shown() {
            shown = true;
            break;
        }
    }
    t.check(shown, "scan-ui: the window is on screen");
    if hand {
        let secs: u64 = plat::env("jfgui_SCAN_WAIT_S").and_then(|v| v.parse().ok()).unwrap_or(480);
        t.info(&format!("  (choose the scanner on the left, then Scan: within {secs} s)\n"));
        match scan_wait(secs * 1000) {
            Some(Ok(s)) => {
                let png = s.data.starts_with(b"\x89PNG");
                t.info(&format!("  scanned {:?}, {} bytes{}\n", s.name, s.data.len(), if png { ", PNG" } else { "" }));
                t.check(!s.data.is_empty(), "scan-ui: by hand: a scanned file");
            }
            other => t.check(false, &format!("scan-ui: by hand: a scanned file ({other:?})")),
        }
    } else {
        let _ = scan_wait(1000); // the device browser looks around a moment
        t.check(plat::mac_scan_close(), "scan-ui: closed as by its close button");
        let r = scan_wait(5000);
        t.check(r == Some(Err(ScanError::Cancelled)), &format!("scan-ui: closed without a scan -> Cancelled ({r:?})"));
    }
    t.check(!scan_open() && !plat::mac_scan_shown(), "scan-ui: nothing open after the answer");
}

#[cfg(not(target_os = "macos"))]
pub fn t_scan_ui(t: &mut T) {
    t.info("  (scan-ui: macOS only so far)\n");
}
