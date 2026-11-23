// t_ios -- (iOS under UIKit: tools/apple.py simapp, an iPhone or an iPad simulator) the host's parts of
// the iOS list (docs/APPLE.md AP5): a small app; a thread of the test drives the host through
// plat::ios_* and reads what the app saw:
//
//   screen on   plat::inhibit(INHIBIT_IDLE) turns UIApplication's idle timer off while a guard
//               lives, on again after the last one
//   dark        dark and light switched while the app runs (the window's style override): the
//               settings follow, E_SETTINGS reaches the app (UIC_SETTINGS), its theme follows
//   text size   the system's text size (Dynamic Type; set as the window's trait override, as the
//               user's setting arrives): the settings' text_scale follows UIFontMetrics ("Large"
//               100 %, XXL above it, an accessibility size beyond 200 %), E_SETTINGS reaches the
//               app and its text scale follows; the override gone: the start again
//   share       plat::share from the app's frame (a title, a text, a link, a file): the sheet
//               (UIActivityViewController) shows, SHARE_PENDING; closed again -> SHARE_CANCELLED;
//               CAP_SHARE under the host
//   turning     landscape and back (requestGeometryUpdate, as the user turning the phone): the
//               canvas, the app's size and the picture follow the view; with the GPU's output
//               (jfgui_GPU=1) the layer stays shown, at the view's frame, its drawable at the
//               canvas's size
//
// Elsewhere it says so and passes.  The client prints its own checks (the host runs the app after
// main returned) and ends the app.

use crate::T;

#[cfg(not(target_os = "ios"))]
pub fn t_ios(t: &mut T) {
    t.info("  (ios: the iOS host under UIKit; tools/apple.py simapp runs it in a simulator)\n");
}

#[cfg(target_os = "ios")]
pub use ios::t_ios;

#[cfg(target_os = "ios")]
mod ios {
    use alloc::boxed::Box;
    use alloc::format;
    use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering::SeqCst};

    use plat::{CAP_SHARE, INHIBIT_IDLE, IosPadView, SHARE_CANCELLED, SHARE_PENDING, Share, capabilities, inhibit, ios_dark, ios_idle_timer_off,
               ios_pad_rotate, ios_pad_view, ios_share_dismiss, ios_share_shown, ios_text_size, share, share_open, share_poll, sys_settings};
    use ui::{App, AppDesc, Ui};

    use super::*;

    const DESC: AppDesc = AppDesc { title: "iOS", flags: ui::AF_RESIZABLE, ..AppDesc::NONE };

    // what the app saw: frames, its client size, its text scale, E_SETTINGS
    static FRAMES: AtomicU32 = AtomicU32::new(0);
    static CW: AtomicI32 = AtomicI32::new(0);
    static CH: AtomicI32 = AtomicI32::new(0);
    static SCALE: AtomicU32 = AtomicU32::new(0);
    static DARK: AtomicBool = AtomicBool::new(false);
    // the client asks the frame for a share sheet; what share answered, what share_poll gave
    static ASK_SHARE: AtomicBool = AtomicBool::new(false);
    static SHARED: AtomicI32 = AtomicI32::new(i32::MIN);
    static SHARE_ANSWER: AtomicI32 = AtomicI32::new(i32::MIN);
    // UIC_SETTINGS: how many, their bits
    static SETTINGS: AtomicU32 = AtomicU32::new(0);
    static SETTINGS_BITS: AtomicU32 = AtomicU32::new(0);
    static QUIT: AtomicBool = AtomicBool::new(false);

    static CHECKS: AtomicI32 = AtomicI32::new(0);
    static FAILS: AtomicI32 = AtomicI32::new(0);

    struct Phone;

    impl App for Phone {
        fn init(&mut self, _ui: &mut Ui) -> bool {
            if plat::thread_spawn(0, Box::new(client)).is_none() {
                record(false, "ios: the client thread starts");
            }
            true
        }

        fn frame(&mut self, ui: &mut Ui) {
            if QUIT.load(SeqCst) {
                ui.quit();
            }
            ui.box_begin(0, ui::BOX_V | ui::PAD_16, 8);
            for k in 0..12 {
                ui.text(ui::TS_BODY, &format!("Zeile {k}"));
            }
            ui.box_end();
            let (w, h) = ui.client_size();
            CW.store(w, SeqCst);
            CH.store(h, SeqCst);
            SCALE.store(ui.text_scale(), SeqCst);
            DARK.store(ui.theme_dark(), SeqCst);
            if ASK_SHARE.swap(false, SeqCst) {
                let file = alloc::format!("{}/plattest-share.txt", plat::temp_dir().unwrap_or_default().trim_end_matches('/'));
                let _ = plat::write_file(&file, b"plattest");
                let s = Share {
                    title: alloc::string::String::from("plattest"),
                    text: alloc::string::String::from("Ein Text"),
                    url: alloc::string::String::from("https://jflib.org/"),
                    files: alloc::vec![file],
                    anchor: ui.last_rect(),
                };
                SHARED.store(share(&s), SeqCst);
            }
            if let Some(r) = share_poll() {
                SHARE_ANSWER.store(r, SeqCst);
            }
            FRAMES.fetch_add(1, SeqCst);
        }

        fn command(&mut self, _ui: &mut Ui, id: u32, arg: u32) -> bool {
            if id == ui::UIC_SETTINGS {
                SETTINGS.fetch_add(1, SeqCst);
                SETTINGS_BITS.fetch_or(arg, SeqCst);
            }
            false
        }
    }

    pub fn t_ios(t: &mut T) {
        let code = ui::launch(&DESC, Phone, &style::STYLE);
        t.expect_i(code as i64, 0, "ios: UIKit's host takes the app");
    }

    fn record(ok: bool, name: &str) {
        CHECKS.fetch_add(1, SeqCst);
        if !ok {
            FAILS.fetch_add(1, SeqCst);
        }
        plat::write_stdout(format!("{} {name}\n", if ok { "PASS" } else { "FAIL" }).as_bytes());
    }

    fn info(s: &str) {
        plat::write_stdout(format!("  {s}\n").as_bytes());
    }

    /// cond within ms (frames run on the main thread meanwhile)
    fn wait(ms: u32, mut cond: impl FnMut() -> bool) -> bool {
        for _ in 0..ms / 20 {
            if cond() {
                return true;
            }
            plat::sleep_ms(20);
        }
        cond()
    }

    /// a few frames after now (a change has been laid out and drawn)
    fn settle() {
        let f = FRAMES.load(SeqCst);
        plat::wake();
        wait(1000, || FRAMES.load(SeqCst) >= f + 2);
    }

    fn view() -> IosPadView {
        ios_pad_view().unwrap_or_default()
    }

    fn rnd(v: f64) -> i32 {
        if v >= 0.0 { (v + 0.5) as i32 } else { (v - 0.5) as i32 }
    }

    fn client() {
        if !wait(10_000, || FRAMES.load(SeqCst) > 2 && ios_pad_view().is_some_and(|v| v.canvas.0 > 0)) {
            record(false, "ios: the app shows within 10 s");
        } else {
            screen_on();
            dark();
            text_size();
            share_sheet();
            turning();
        }
        let (n, f) = (CHECKS.load(SeqCst), FAILS.load(SeqCst));
        plat::write_stdout(format!("ios: {n} checks, {f} failed{}\n", if f != 0 { " (check(s) failed)" } else { "" }).as_bytes());
        QUIT.store(true, SeqCst);
        plat::wake();
    }

    fn screen_on() {
        record(!ios_idle_timer_off(), "ios: screen on: the idle timer runs at the start");
        let a = inhibit("plattest", INHIBIT_IDLE);
        record(a.is_some() && ios_idle_timer_off(), "ios: screen on: a guard turns the idle timer off");
        let b = inhibit("", INHIBIT_IDLE);
        drop(a);
        record(ios_idle_timer_off(), "ios: screen on: still off while a second guard lives");
        drop(b);
        record(!ios_idle_timer_off(), "ios: screen on: the last guard gone, the timer runs again");
    }

    fn share_sheet() {
        record(capabilities().supports(CAP_SHARE), "ios: share: CAP_SHARE under UIKit's host");
        ASK_SHARE.store(true, SeqCst);
        plat::wake();
        wait(3000, || SHARED.load(SeqCst) != i32::MIN);
        record(SHARED.load(SeqCst) == SHARE_PENDING && share_open(), "ios: share: the sheet opens (SHARE_PENDING)");
        let shown = wait(5000, || ios_share_shown().is_some_and(|c| c.contains("ActivityViewController")));
        record(shown, &format!("ios: share: the sheet on screen ({:?})", ios_share_shown()));
        plat::sleep_ms(1000);
        ios_share_dismiss();
        let got = wait(8000, || SHARE_ANSWER.load(SeqCst) != i32::MIN);
        let a = SHARE_ANSWER.load(SeqCst);
        record(got && a == SHARE_CANCELLED, &format!("ios: share: closed without a target -> SHARE_CANCELLED ({a})"));
        record(!share_open() && wait(3000, || ios_share_shown().is_none()), "ios: share: nothing open after the answer");
    }

    /// dark and light as the user switches them while the app runs: the settings follow, E_SETTINGS
    /// reaches the app (UIC_SETTINGS) and its theme follows
    fn dark() {
        let n0 = SETTINGS.load(SeqCst);
        for d in [true, false] {
            ios_dark(Some(d));
            let ok = wait(3000, || sys_settings().dark == Some(d) && DARK.load(SeqCst) == d);
            record(ok, &format!("ios: {}: the settings and the app's theme follow", if d { "dark" } else { "light" }));
        }
        record(SETTINGS.load(SeqCst) > n0, "ios: the changes reached the app as UIC_SETTINGS");
        ios_dark(None);
        settle();
    }

    /// the settings' text scale passes `ok`, and the app's text scale followed it
    fn scale_is(label: &str, ok: impl Fn(u32) -> bool) -> u32 {
        let got = wait(3000, || ok(sys_settings().text_scale) && SCALE.load(SeqCst) == sys_settings().text_scale.clamp(50, 300));
        let (s, a) = (sys_settings().text_scale, SCALE.load(SeqCst));
        record(got, &format!("ios: text size {label}: the settings say {s} %, the app's text scale {a} % (UIC_SETTINGS {} times, bits {:#x})",
                             SETTINGS.load(SeqCst), SETTINGS_BITS.load(SeqCst)));
        s
    }

    fn text_size() {
        let s0 = sys_settings().text_scale;
        info(&format!("text size at the start: {s0} % (the simulator's setting)"));
        if !ios_text_size(Some("XXL")) {
            info("(text size: no trait overrides before iOS 17: not tried)");
            return;
        }
        scale_is("XXL", |s| s > 100 && s < 200);
        ios_text_size(Some("AccessibilityXXXL"));
        scale_is("accessibility XXXL", |s| s >= 200);
        ios_text_size(Some("L"));
        scale_is("Large", |s| s == 100);
        ios_text_size(None);
        scale_is("the system's again", |s| s == s0);
    }

    /// the canvas follows the view and the app draws that size; with the GPU's layer: it shows the
    /// window at the view's frame, its drawable the canvas
    fn consistent(label: &str, gpu: bool) -> IosPadView {
        settle();
        let v = view();
        let (pw, ph) = (rnd(v.view.0 * v.scale), rnd(v.view.1 * v.scale));
        let dip = |px: i32, d: i32| (px - rnd(d as f64 * v.scale)).abs() <= 1;
        let layer_ok = |v: &IosPadView| {
            v.gpu.is_some_and(|g| {
                g.shown && rnd(g.drawable.0) == pw && rnd(g.drawable.1) == ph && (g.frame.0 - v.view.0).abs() < 1.0 && (g.frame.1 - v.view.1).abs() < 1.0
            })
        };
        let ok = wait(3000, || {
            let v = view();
            dip(pw, CW.load(SeqCst)) && dip(ph, CH.load(SeqCst)) && v.canvas == (pw, ph) && (!gpu || layer_ok(&v))
        });
        let v = view();
        let g = v.gpu.unwrap_or_default();
        info(&format!("{label}: view {:.0}x{:.0} pt, canvas {}x{} px at {:.3}, shown {}x{}, app {}x{}, orientation {}, gpu layer {} {:.0}x{:.0} px at {:.0}x{:.0} pt",
                      v.view.0, v.view.1, v.canvas.0, v.canvas.1, v.scale, v.shown.0, v.shown.1, CW.load(SeqCst), CH.load(SeqCst), v.orientation,
                      if v.gpu.is_none() { "none" } else if g.shown { "shown" } else { "hidden" }, g.drawable.0, g.drawable.1, g.frame.0, g.frame.1));
        record(ok, &format!("ios: {label}: the canvas and the app's size follow the view{}", if gpu { ", the GPU's layer too" } else { "" }));
        v
    }

    fn turning() {
        // the GPU's output: jfgui_GPU=1 puts the window on the layer after a few frames
        let gpu = plat::env("jfgui_GPU").is_some_and(|v| v == "1");
        if gpu && !wait(5000, || view().gpu.is_some_and(|g| g.shown)) {
            record(false, "ios: jfgui_GPU=1: the window goes to the GPU's layer within 5 s");
            return;
        }
        let v0 = consistent("upright", gpu);
        let o0 = v0.orientation;
        ios_pad_rotate(o0 <= 2);
        if !wait(4000, || view().orientation != o0) {
            info("(turning: the system kept the orientation)");
            return;
        }
        let v = consistent("turned", gpu);
        record((v.view.0 > v.view.1) == (o0 <= 2), "ios: turned: the view's longer side follows");
        ios_pad_rotate(o0 > 2);
        wait(4000, || view().orientation == o0);
        let v = consistent("turned back", gpu);
        record(v.canvas == v0.canvas, "ios: turned back: the first canvas");
    }
}
