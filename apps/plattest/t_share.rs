// t_share -- the system's share sheet (lib/plat/share.rs)
//
//   plattest --test share      the contract: nothing to share, an empty or missing file -> SHARE_NONE,
//                              no answer without a sheet, nothing open; CAP_SHARE where the system
//                              has a sheet (Windows, macOS; iOS only under UIKit's host; Linux none)
//   plattest --test share-ui   (by hand, Windows and macOS: a real window that takes the front) the
//                              sheet opened from the app's frame with a title, a text, a link and a
//                              file -> SHARE_PENDING, a second one refused while it is open; after
//                              jfgui_SHARE_WAIT_MS (4000: Windows asks for the data about 1.5 s after
//                              the sheet opens) closed again as the user would (Windows: Esc to the
//                              sheet when it is in front, the app's window in front again -- the sheet
//                              closes when it loses the focus, which another app in front can keep
//                              from happening; macOS: the picker closed) -> SHARE_CANCELLED, then a
//                              new one opens and closes the same way.  Elsewhere it says so.

use alloc::format;

use plat::{CAP_SHARE, SHARE_NONE, Share, capabilities, share, share_open, share_poll};

use crate::T;

pub fn t_share(t: &mut T) {
    t.check(share(&Share::default()) == SHARE_NONE, "share: nothing to share -> SHARE_NONE");
    t.check(share(&Share { files: alloc::vec![alloc::string::String::new()], ..Share::default() }) == SHARE_NONE, "share: an empty path -> SHARE_NONE");
    let missing = format!("{}/plattest-share-missing-{}.txt", plat::temp_dir().unwrap_or_default().trim_end_matches('/'), plat::time_us());
    t.check(share(&Share::files(&[&missing])) == SHARE_NONE, "share: a missing file -> SHARE_NONE");
    t.check(share_poll().is_none() && !share_open(), "share: no answer, nothing open");
    let cap = capabilities().supports(CAP_SHARE);
    let want = cfg!(any(windows, target_os = "macos"));
    if cfg!(any(windows, target_os = "macos", target_os = "linux")) {
        t.check(cap == want, &format!("share: CAP_SHARE {} here", if want { "set" } else { "not set" }));
    } else {
        t.info(&format!("  share: CAP_SHARE {cap} (iOS: only under UIKit's host; browser and Android: the host's)\n"));
    }
    if cfg!(target_os = "linux") {
        t.check(share(&Share::text("Hallo")) == SHARE_NONE, "share: Linux has no sheet -> SHARE_NONE");
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub fn t_share_ui(t: &mut T) {
    t.info("  (share-ui: Windows and macOS in a real window; iOS: plattest --test ios under UIKit)\n");
}

#[cfg(any(windows, target_os = "macos"))]
pub use ui_test::t_share_ui;

#[cfg(any(windows, target_os = "macos"))]
mod ui_test {
    use alloc::boxed::Box;
    use alloc::format;
    use alloc::string::String;
    use core::sync::atomic::{AtomicI32, AtomicU32, Ordering::SeqCst};

    use plat::{SHARE_CANCELLED, SHARE_NONE, SHARE_PENDING, Share, share, share_open, share_poll};
    use ui::{App, AppDesc, Ui};

    use super::*;

    const DESC: AppDesc = AppDesc { title: "Teilen", flags: ui::AF_RESIZABLE, ..AppDesc::NONE };

    static FRAMES: AtomicU32 = AtomicU32::new(0);
    // the client asks the frame for a sheet (1: open one, 2: open a second one while one is open)
    static ASK: AtomicU32 = AtomicU32::new(0);
    // what share answered there, what share_poll gave (i32::MIN: nothing yet)
    static OPENED: AtomicI32 = AtomicI32::new(i32::MIN);
    static SECOND: AtomicI32 = AtomicI32::new(i32::MIN);
    static ANSWER: AtomicI32 = AtomicI32::new(i32::MIN);
    static QUIT: AtomicU32 = AtomicU32::new(0);
    static CHECKS: AtomicI32 = AtomicI32::new(0);
    static FAILS: AtomicI32 = AtomicI32::new(0);

    struct Sharer {
        file: String,
    }

    impl App for Sharer {
        fn init(&mut self, _ui: &mut Ui) -> bool {
            if plat::thread_spawn(0, Box::new(client)).is_none() {
                record(false, "share-ui: the client thread starts");
            }
            true
        }

        fn frame(&mut self, ui: &mut Ui) {
            if QUIT.load(SeqCst) != 0 {
                ui.quit();
            }
            ui.text(ui::TS_BODY, "Teilen");
            match ASK.swap(0, SeqCst) {
                1 => {
                    let s = Share {
                        title: String::from("plattest: Teilen"),
                        text: String::from("Ein Text aus plattest"),
                        url: String::from("https://jflib.org/"),
                        files: alloc::vec![self.file.clone()],
                        anchor: ui.last_rect(),
                    };
                    OPENED.store(share(&s), SeqCst);
                }
                2 => SECOND.store(share(&Share::text("noch einer")), SeqCst),
                _ => {}
            }
            if let Some(r) = share_poll() {
                ANSWER.store(r, SeqCst);
            }
            FRAMES.fetch_add(1, SeqCst);
        }
    }

    pub fn t_share_ui(t: &mut T) {
        let file = format!("{}/plattest-share.txt", plat::temp_dir().unwrap_or_default().trim_end_matches('/'));
        t.check(plat::write_file(&file, "Eine Datei aus plattest\n".as_bytes()), "share-ui: the file to share");
        let code = ui::launch(&DESC, Sharer { file: file.clone() }, &style::STYLE);
        t.expect_i(code as i64, 0, "share-ui: the window ran");
        let _ = plat::delete(&file);
        t.expect_i(FAILS.load(SeqCst) as i64, 0, &format!("share-ui: {} checks of the client passed", CHECKS.load(SeqCst)));
    }

    fn record(ok: bool, name: &str) {
        CHECKS.fetch_add(1, SeqCst);
        if !ok {
            FAILS.fetch_add(1, SeqCst);
        }
        plat::write_stdout(format!("{} {name}\n", if ok { "PASS" } else { "FAIL" }).as_bytes());
    }

    fn wait(ms: u32, mut cond: impl FnMut() -> bool) -> bool {
        for _ in 0..ms / 20 {
            if cond() {
                return true;
            }
            plat::sleep_ms(20);
        }
        cond()
    }

    /// close the open sheet as the user would
    fn dismiss() -> bool {
        #[cfg(windows)]
        return plat::win_share_dismiss();
        #[cfg(target_os = "macos")]
        return plat::mac_share_close();
    }

    fn round(k: u32) {
        OPENED.store(i32::MIN, SeqCst);
        ANSWER.store(i32::MIN, SeqCst);
        ASK.store(1, SeqCst);
        plat::wake();
        wait(3000, || OPENED.load(SeqCst) != i32::MIN);
        record(OPENED.load(SeqCst) == SHARE_PENDING && share_open(), &format!("share-ui: round {k}: the sheet opens (SHARE_PENDING)"));
        if OPENED.load(SeqCst) != SHARE_PENDING {
            return;
        }
        if k == 1 {
            SECOND.store(i32::MIN, SeqCst);
            ASK.store(2, SeqCst);
            plat::wake();
            wait(3000, || SECOND.load(SeqCst) != i32::MIN);
            record(SECOND.load(SeqCst) == SHARE_NONE, "share-ui: a second sheet while one is open -> SHARE_NONE");
        }
        // the sheet on screen a moment (Windows asks DataRequested meanwhile)
        plat::sleep_ms(plat::env("jfgui_SHARE_WAIT_MS").and_then(|v| v.parse().ok()).unwrap_or(4000));
        record(ANSWER.load(SeqCst) == i32::MIN, &format!("share-ui: round {k}: no answer while the sheet is open"));
        dismiss();
        let got = wait(8000, || ANSWER.load(SeqCst) != i32::MIN);
        let a = ANSWER.load(SeqCst);
        record(got && a == SHARE_CANCELLED, &format!("share-ui: round {k}: closed without a target -> SHARE_CANCELLED ({a})"));
        record(!share_open(), &format!("share-ui: round {k}: nothing open after the answer"));
    }

    fn client() {
        plat::wake();
        if !wait(10_000, || FRAMES.load(SeqCst) > 0) {
            record(false, "share-ui: the window shows within 10 s");
        } else {
            round(1);
            round(2);
        }
        QUIT.store(1, SeqCst);
        plat::wake();
    }
}
