// t_ipad -- (iOS on an iPad, under UIKit: tools/apple.py simapp in an iPad simulator) the iPad's parts
// of the host (docs/APPLE.md AP9): a small app with a menu bar, a scrolling list and a text box; a
// thread of the test drives the host through plat::ios_pad_* and reads what the app saw:
//
//   sizes      the window as Split View, Slide Over and Stage Manager give it (its frame in the
//              scene): the canvas follows the view in px, the picture shown has that size, the form
//              factor is FF_PHONE below 600 points, FF_TABLET above; the whole scene again; the
//              keyboard and back; turning to landscape and back (where iPadOS lets the app turn)
//   menus      the main menu built with a stand-in for UIKit's builder (iPadOS builds its own only
//              when the menu is needed: the bar revealed, Command held, a hardware keyboard's key):
//              the app's menus after the application menu, before Window, the system's File / Edit /
//              Format / View gone; rows, separators, a submenu, Command keys, F5, a key used twice
//              only once, checked and disabled rows, a list's rows (UIKit's own UIMenu, UICommand and
//              UIKeyCommand objects); a row chosen through the responder chain reaches App::command,
//              a changed state asks iPadOS for a new build, a pass without the bar takes the menus
//              out; the window keeps its menu row (VoiceOver's elements show its "Datei")
//   pointer    hover moves the mouse without a button (the pencil's hover a pen), it leaves; two
//              fingers on a trackpad scroll the list; a pinch is Ctrl + wheel
//
// Elsewhere (and on an iPhone) it says so and passes.  The client prints its own checks (the host runs
// the app after main returned) and ends the app.

use crate::T;

#[cfg(not(target_os = "ios"))]
pub fn t_ipad(t: &mut T) {
    t.info("  (ipad: the iPad's host; tools/apple.py simapp runs it in an iPad simulator)\n");
}

#[cfg(target_os = "ios")]
pub use ios::t_ipad;

#[cfg(target_os = "ios")]
mod ios {
    use alloc::boxed::Box;
    use alloc::format;
    use alloc::string::String;
    use alloc::vec::Vec;
    use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering::SeqCst};

    use plat::{IosMenuRow, IosPadMenus, IosPadPointer as P, IosPadView, ios_pad_keyboard, ios_pad_menu_build, ios_pad_menu_choose, ios_pad_menus,
               ios_pad_pointer, ios_pad_rotate, ios_pad_view, ios_pad_window};
    use ui::{AppDesc, Command, Mi, Ui, App};

    use super::*;

    const CMD_NEW: u32 = 9001;
    const CMD_OPEN: u32 = 9002;
    const CMD_GRID: u32 = 9003;
    const CMD_LOCKED: u32 = 9004;
    const CMD_ZOOMIN: u32 = 9005;
    const CMD_ZOOMOUT: u32 = 9006;
    const CMD_RELOAD: u32 = 9007;
    const CMD_COPY: u32 = 9008;
    const CMD_NEW2: u32 = 9009;
    const CMD_RECENT0: u32 = 9040;
    const ID_LIST: u32 = 0x200;
    const ID_EDIT: u32 = 0x201;

    static CMDS: [Command; 9] = [
        Command::new(CMD_NEW, "&Neu", 0, ui::MOD_CTRL, ui::VK_N, 0),
        Command::new(CMD_OPEN, "Ö&ffnen…", 0, ui::MOD_CTRL, ui::VK_O, 0),
        Command::new(CMD_GRID, "&Raster", 0, 0, 0, 0),
        Command::new(CMD_LOCKED, "&Gesperrt", 0, 0, 0, ui::CF_DISABLED),
        Command::new(CMD_ZOOMIN, "Ver&größern", 0, ui::MOD_CTRL, ui::VK_OEM_PLUS, 0),
        Command::new(CMD_ZOOMOUT, "Ver&kleinern", 0, ui::MOD_CTRL, ui::VK_OEM_MINUS, 0),
        Command::new(CMD_RELOAD, "&Aktualisieren", 0, 0, ui::VK_F5, 0),
        Command::new(CMD_COPY, "&Kopieren", 0, ui::MOD_CTRL, ui::VK_C, 0),
        Command::new(CMD_NEW2, "Noch &neu", 0, ui::MOD_CTRL, ui::VK_N, 0),
    ];
    static MENUBAR: [Mi; 2] = [Mi::Menu("&Datei", &M_FILE), Mi::Menu("&Ansicht", &M_VIEW)];
    static M_FILE: [Mi; 10] = [
        Mi::Cmd(CMD_NEW),
        Mi::Cmd(CMD_OPEN),
        Mi::Sep,
        Mi::Cmd(CMD_GRID),
        Mi::Cmd(CMD_LOCKED),
        Mi::Sep,
        Mi::Sub("&Zoom", &M_ZOOM),
        Mi::Cmd(CMD_RELOAD),
        Mi::Cmd(CMD_COPY),
        Mi::Sep,
    ];
    static M_ZOOM: [Mi; 2] = [Mi::Cmd(CMD_ZOOMIN), Mi::Cmd(CMD_ZOOMOUT)];
    static M_VIEW: [Mi; 3] = [Mi::List(CMD_RECENT0, 4, "Keine Dateien"), Mi::Sep, Mi::Cmd(CMD_NEW2)];
    const DESC: AppDesc = AppDesc { title: "iPad", flags: ui::AF_RESIZABLE, cmds: &CMDS, menubar: &MENUBAR, ..AppDesc::NONE };

    // what the app saw, for the client: frames, form factor, client size, mouse, pen type, list
    // position, commands (count of CMD_NEW, the last id), the grid
    static FRAMES: AtomicU32 = AtomicU32::new(0);
    static FORM: AtomicU32 = AtomicU32::new(u32::MAX);
    static CW: AtomicI32 = AtomicI32::new(0);
    static CH: AtomicI32 = AtomicI32::new(0);
    static MX: AtomicI32 = AtomicI32::new(i32::MIN);
    static MY: AtomicI32 = AtomicI32::new(i32::MIN);
    static PTYPE: AtomicU32 = AtomicU32::new(u32::MAX);
    static LIST_Y: AtomicI32 = AtomicI32::new(0);
    static NEWS: AtomicU32 = AtomicU32::new(0);
    static LAST_CMD: AtomicU32 = AtomicU32::new(0);
    static GRID: AtomicBool = AtomicBool::new(false);
    // what the client asks of the app: the menu bar drawn, the text box focused, the end
    static MENUS_ON: AtomicBool = AtomicBool::new(true);
    static FOCUS: AtomicBool = AtomicBool::new(false);
    static QUIT: AtomicBool = AtomicBool::new(false);

    static CHECKS: AtomicI32 = AtomicI32::new(0);
    static FAILS: AtomicI32 = AtomicI32::new(0);

    struct Pad {
        text: String,
        recent: Vec<String>,
    }

    impl App for Pad {
        fn init(&mut self, _ui: &mut Ui) -> bool {
            if plat::thread_spawn(0, Box::new(client)).is_none() {
                record(false, "ipad: the client thread starts");
            }
            true
        }

        fn frame(&mut self, ui: &mut Ui) {
            if QUIT.load(SeqCst) {
                ui.quit();
            }
            if MENUS_ON.load(SeqCst) {
                ui.cmd_state(CMD_GRID, if GRID.load(SeqCst) { ui::CF_CHECKED } else { 0 });
                ui.menu_list(CMD_RECENT0, &self.recent);
                ui.menubar(None);
            }
            ui.next_size(ui::SIZE_FILL, 0);
            ui.textbox(ID_EDIT, Some("Text"), &mut self.text, 64);
            if FOCUS.swap(false, SeqCst) {
                ui.focus(ID_EDIT);
            }
            ui.next_size(ui::SIZE_FILL, ui::SIZE_FILL);
            ui.scroll_begin(ID_LIST, ui::SC_V);
            ui.box_begin(0, ui::BOX_V | ui::PAD_16, 8);
            for k in 0..80 {
                ui.text(ui::TS_BODY, &format!("Zeile {k}"));
            }
            ui.box_end();
            ui.scroll_end();
            let (w, h) = ui.client_size();
            CW.store(w, SeqCst);
            CH.store(h, SeqCst);
            FORM.store(ui.form_factor(), SeqCst);
            let (mx, my) = ui.mouse();
            MX.store(mx, SeqCst);
            MY.store(my, SeqCst);
            PTYPE.store(ui.pen().1, SeqCst);
            LIST_Y.store(ui.scroll_pos(ID_LIST).1, SeqCst);
            FRAMES.fetch_add(1, SeqCst);
        }

        fn command(&mut self, _ui: &mut Ui, id: u32, _arg: u32) -> bool {
            LAST_CMD.store(id, SeqCst);
            match id {
                CMD_NEW => {
                    NEWS.fetch_add(1, SeqCst);
                }
                CMD_GRID => {
                    GRID.fetch_xor(true, SeqCst);
                }
                _ => {}
            }
            true
        }
    }

    pub fn t_ipad(t: &mut T) {
        let app = Pad { text: String::new(), recent: alloc::vec![String::from("a.txt"), String::from("b.txt")] };
        let code = ui::launch(&DESC, app, &style::STYLE);
        t.expect_i(code as i64, 0, "ipad: UIKit's host takes the app");
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
            record(false, "ipad: the app shows within 10 s");
        } else if !view().pad {
            info("(ipad: not an iPad -- run it in an iPad simulator, jfgui_SIM_DEVICE)");
        } else {
            sizes();
            menus();
            pointer();
        }
        let (n, f) = (CHECKS.load(SeqCst), FAILS.load(SeqCst));
        plat::write_stdout(format!("ipad: {n} checks, {f} failed{}\n", if f != 0 { " (check(s) failed)" } else { "" }).as_bytes());
        QUIT.store(true, SeqCst);
        plat::wake();
    }

    /// the canvas follows the view and the app draws that size; the form factor fits the view
    fn consistent(label: &str) -> IosPadView {
        settle();
        let v = view();
        let (pw, ph) = (rnd(v.view.0 * v.scale), rnd(v.view.1 * v.scale));
        // (the app's client size is in DIP: px over the canvas's px per DIP, which is the host's scale)
        let dip = |px: i32, d: i32| (px - rnd(d as f64 * v.scale)).abs() <= 1;
        let ok = wait(2000, || dip(pw, CW.load(SeqCst)) && dip(ph, CH.load(SeqCst)) && view().shown == (pw, ph));
        let v = view();
        info(&format!("{label}: scene {:.0}x{:.0}, window {:.0}x{:.0}, view {:.0}x{:.0} pt, safe {:.0}/{:.0}/{:.0}/{:.0}, canvas {}x{} px at {:.3}, shown {}x{}, app {}x{}, form {}",
                      v.scene.0, v.scene.1, v.window.0, v.window.1, v.view.0, v.view.1, v.safe.0, v.safe.1, v.safe.2, v.safe.3, v.canvas.0, v.canvas.1,
                      v.scale, v.shown.0, v.shown.1, CW.load(SeqCst), CH.load(SeqCst), FORM.load(SeqCst)));
        record(ok && v.canvas == (pw, ph), &format!("ipad: {label}: the canvas, the app's size and the picture follow the view"));
        let want = if v.view.0.min(v.view.1) < 600.0 { ui::FF_PHONE } else { ui::FF_TABLET };
        record(FORM.load(SeqCst) == want, &format!("ipad: {label}: form factor {}", if want == ui::FF_PHONE { "phone" } else { "tablet" }));
        v
    }

    fn sizes() {
        let v0 = consistent("the whole scene");
        record(v0.scale == v0.screen_scale.min(2.0), "ipad: the whole screen at its own scale (no smaller canvas on an iPad)");
        record((v0.window.0 - v0.scene.0).abs() < 1.0 && (v0.window.1 - v0.scene.1).abs() < 1.0, "ipad: the window fills the scene");
        let (sw, sh) = v0.scene;
        // Slide Over, Split View's thirds and half, Stage Manager's windows: small, wide, tall
        for (label, w, h) in [("Slide Over 320 pt", 320.0, sh), ("Split View 1/3", (sw / 3.0) as i32 as f64, sh), ("Split View 1/2", (sw / 2.0) as i32 as f64, sh),
                              ("Split View 2/3", (sw * 2.0 / 3.0) as i32 as f64, sh), ("Stage Manager 700 x 500", 700.0, 500.0),
                              ("Stage Manager 500 x 700", 500.0, 700.0), ("Stage Manager 1000 x 760", 1000.0, 760.0)] {
            ios_pad_window(w, h);
            let v = consistent(label);
            record((v.window.0 - w).abs() < 1.0 && (v.window.1 - h).abs() < 1.0, &format!("ipad: {label}: the window took the size"));
        }
        ios_pad_window(0.0, 0.0);
        let v = consistent("the whole scene again");
        record(v.canvas == v0.canvas, "ipad: the whole scene again: the first canvas");
        // the keyboard: the text box focused, JFView first responder (as after a tap into it)
        FOCUS.store(true, SeqCst);
        settle();
        ios_pad_keyboard(true);
        plat::sleep_ms(1500);
        let k = consistent("keyboard shown");
        record(k.keyboard, "ipad: the keyboard is asked for");
        info(&format!("keyboard: the view {:.0} pt high (without {:.0})", k.view.1, v0.view.1));
        ios_pad_keyboard(false);
        plat::sleep_ms(1500);
        let k = consistent("keyboard gone");
        record(!k.keyboard && (k.view.1 - v0.view.1).abs() < 1.0, "ipad: the keyboard gone: the view as before");
        // turning (iPadOS may keep a window's orientation: then nothing changes, said so)
        let o0 = view().orientation;
        ios_pad_rotate(o0 <= 2);
        if wait(4000, || view().orientation != o0) {
            let v = consistent("turned");
            record((v.view.0 > v.view.1) == (o0 <= 2), "ipad: turned: the view's longer side follows");
            ios_pad_rotate(o0 > 2);
            wait(4000, || view().orientation == o0);
            let v = consistent("turned back");
            record(v.canvas == v0.canvas, "ipad: turned back: the first canvas");
        } else {
            info("(turning: iPadOS kept the orientation of the app's window)");
        }
    }

    fn row<'a>(rows: &'a [IosMenuRow], label: &str) -> Option<&'a IosMenuRow> {
        rows.iter().find(|r| r.label == label)
    }

    /// the main menu built with the stand-in builder (iPadOS builds its own only when the menu is
    /// needed) until the app's menus are in it, or not
    fn built(want: bool) -> Option<(Vec<String>, IosPadMenus)> {
        let mut out = None;
        wait(3000, || {
            let tops = ios_pad_menu_build().unwrap_or_default();
            let m = ios_pad_menus();
            let ok = m.menus.is_empty() != want;
            out = Some((tops, m));
            ok
        })
        .then_some(())?;
        out
    }

    fn menus() {
        record(view().window_row, "ipad: the window keeps its menu row");
        let r0 = ios_pad_menus().rebuilds;
        info(&format!("(builds of the main menu by iPadOS so far: {}; rebuilds asked: {r0})", ios_pad_menus().builds));
        let Some((tops, pm)) = built(true) else {
            record(false, "ipad: the main menu holds the app's menus");
            return;
        };
        let m = pm.menus.clone();
        info(&format!("top level: {tops:?}"));
        for (t, rows) in &m {
            for r in rows {
                info(&format!("  {t} | {:?} id {} key {:?} flags {:#x} {}{}{}sub {}", r.label, r.id, r.key, r.key_flags,
                              if r.disabled { "disabled " } else { "" }, if r.checked { "checked " } else { "" },
                              if r.sep_before { "sep " } else { "" }, r.sub));
            }
        }
        record(pm.system_gone, "ipad: the system's File, Edit, Format and View menus went");
        record(tops == ["com.apple.menu.application", "org.jflib.sysmenu.0", "org.jflib.sysmenu.1", "com.apple.menu.window", "com.apple.menu.help"],
               "ipad: the app's menus between the application menu and Window");
        record(m.len() == 2 && m[0].0 == "Datei" && m[1].0 == "Ansicht", "ipad: Datei and Ansicht in the main menu");
        let file = m.first().map(|x| x.1.clone()).unwrap_or_default();
        let view_rows = m.get(1).map(|x| x.1.clone()).unwrap_or_default();
        const CMD: isize = 1 << 20;
        record(file.len() == 7, "ipad: Datei has seven rows (the separator at the end gone)");
        record(row(&file, "Neu").is_some_and(|r| r.id == CMD_NEW && r.key == "n" && r.key_flags == CMD && !r.disabled), "ipad: Neu with Command+N");
        record(row(&file, "Raster").is_some_and(|r| r.sep_before && !r.checked), "ipad: Raster after a separator, not checked");
        record(row(&file, "Gesperrt").is_some_and(|r| r.disabled), "ipad: Gesperrt is disabled");
        record(row(&file, "Zoom").is_some_and(|r| r.sep_before && r.sub == 2), "ipad: the submenu Zoom with its two rows");
        record(row(&file, "Aktualisieren").is_some_and(|r| !r.key.is_empty() && r.key != "\u{F708}" && r.key_flags == 0), "ipad: Aktualisieren with F5 (UIKit's name)");
        record(row(&file, "Kopieren").is_some_and(|r| r.key == "c" && r.key_flags == CMD), "ipad: Kopieren with Command+C");
        record(row(&view_rows, "a.txt").is_some_and(|r| r.id == CMD_RECENT0) && row(&view_rows, "b.txt").is_some_and(|r| r.id == CMD_RECENT0 + 1),
               "ipad: the list's rows with their ids");
        record(row(&view_rows, "Noch neu").is_some_and(|r| r.sep_before && r.key.is_empty() && r.id == CMD_NEW2), "ipad: Command+N a second time: no key");
        // chosen through the responder chain, as the menu bar does
        let n = NEWS.load(SeqCst);
        record(ios_pad_menu_choose(0, 0), "ipad: Neu chosen: the app delegate takes the action");
        record(wait(2000, || NEWS.load(SeqCst) == n + 1), "ipad: Neu chosen: App::command(CMD_NEW)");
        record(ios_pad_menu_choose(1, 1) && wait(2000, || LAST_CMD.load(SeqCst) == CMD_RECENT0 + 1), "ipad: b.txt chosen: its id");
        let r1 = ios_pad_menus().rebuilds;
        record(ios_pad_menu_choose(0, 2) && wait(2000, || GRID.load(SeqCst)), "ipad: Raster chosen: the grid on");
        record(wait(2000, || ios_pad_menus().rebuilds > r1), "ipad: the changed state asks iPadOS to build the menu again");
        record(built(true).is_some_and(|(_, pm)| row(&pm.menus.first().map(|x| x.1.clone()).unwrap_or_default(), "Raster").is_some_and(|r| r.checked)),
               "ipad: built again: Raster checked");
        // the row in the window: VoiceOver's elements show the bar's Datei
        let mut seen = false;
        for _ in 0..60 {
            if plat::ios_ax_tree().unwrap_or_default().iter().any(|e| e.element && e.label == "Datei") {
                seen = true;
                break;
            }
            plat::sleep_ms(50);
        }
        record(seen, "ipad: the window still shows its menu row (Datei)");
        // a pass without the bar: the menus leave the main menu, the system's come back
        MENUS_ON.store(false, SeqCst);
        plat::wake();
        let gone = built(false);
        record(gone.as_ref().is_some_and(|(tops, pm)| !pm.system_gone && tops.len() == 7), "ipad: no bar drawn: the app's menus gone, the system's back");
        MENUS_ON.store(true, SeqCst);
        plat::wake();
        record(built(true).is_some_and(|(_, pm)| pm.menus.len() == 2), "ipad: the bar again: the app's menus back");
    }

    fn pointer() {
        let v = view();
        let s = v.scale;
        let (cx, cy) = (v.view.0 / 2.0, v.view.1 / 2.0);
        ios_pad_pointer(P::Hover { x: 120.0, y: cy, pen: None });
        record(wait(2000, || MX.load(SeqCst) == rnd(120.0 * s) && MY.load(SeqCst) == rnd(cy * s) && PTYPE.load(SeqCst) == ui::PTR_MOUSE),
               "ipad: hover: the mouse moves without a button");
        ios_pad_pointer(P::Hover { x: 140.0, y: cy, pen: Some((20, -10)) });
        record(wait(2000, || MX.load(SeqCst) == rnd(140.0 * s) && PTYPE.load(SeqCst) == ui::PTR_PEN), "ipad: the pencil's hover: a pen");
        ios_pad_pointer(P::Leave);
        record(wait(2000, || MX.load(SeqCst) == -1 && MY.load(SeqCst) == -1), "ipad: the pointer left the view");
        let y0 = LIST_Y.load(SeqCst);
        for _ in 0..10 {
            ios_pad_pointer(P::Scroll { dx: 0.0, dy: -30.0, x: cx, y: cy });
        }
        record(wait(2000, || LIST_Y.load(SeqCst) > y0), "ipad: two fingers up on the trackpad: the list scrolls down");
        let w = view().last_wheel;
        record(w.0 == plat::WHEEL_PIXELS && (w.2 & 0xFFFF) as u16 as i16 == rnd(-30.0 * s) as i16, "ipad: the wheel in physical pixels (down negative)");
        ios_pad_pointer(P::Pinch { m: 0.2, x: cx, y: cy });
        let w = view().last_wheel;
        record(w.1 & plat::MOD_CTRL != 0 && w.2 & 0xFFFF == 120, "ipad: a pinch: Ctrl + wheel, 120 per 0.2");
    }
}
