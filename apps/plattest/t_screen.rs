// t_screen -- plattest --test screen / screen-wl / screen-bus / screen-policy / hotkey-hold:
// plat::screen (monitors, pictures of the screen, the colour pick) and plat::hotkey (global
// shortcuts)
//
//   --test screen        the shared parts (chords and their XDG notation, the dry run of the
//                        UI's headless runs, shot_pixel), then the real system:
//                        Windows: the monitors against GetSystemMetrics (count, the virtual
//                        desktop), WM_DISPLAYCHANGE / a work area change -> SET_MONITORS; a
//                        topmost window with a known pattern captured (BitBlt) and picked (a
//                        test point; the overlay with a click and with Esc through its window
//                        procedure; Busy); RegisterHotKey of an unused F13-F24 delivered through
//                        SendInput (no window of ours has the keyboard) and a posted WM_HOTKEY,
//                        the app's own second id on the chord, a chord another process holds
//                        (--test hotkey-hold) is Conflict and free again after it ended.
//                        Linux (X11 with WSLg's X server, jfgui_BACKEND=x11; SKIP without
//                        $DISPLAY): RandR monitors, the pattern window read back through
//                        GetImage (of the window; a rootless server refuses the root's -- then
//                        capture_start's Failed is checked), the pick at a test point and with
//                        XTEST (pointer to the pattern, a click; Esc), XGrabKey of Ctrl+Alt+K
//                        pressed with XTEST, the Num Lock / Caps Lock combinations through
//                        synthetic events, a chord another process grabbed is Conflict
//   --test screen-wl     (Linux, Wayland) the outputs as monitors (wl_output, xdg-output)
//   --test screen-bus --mode grant|deny|none   (Linux) the Screenshot and GlobalShortcuts
//                        portals against tools/platshared/screen_bus.py
//   --test screen-policy a manifest without input.global and screen.capture: Denied (and
//                        PERMDENY); the pick needs no right (not in "all": it stays installed)
//   --test hotkey-hold --chord C --marker F --hold MS   (the other process) holds a shortcut
//                        for MS, writing F once it has it ("ok" / the error) and F.done at the end

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use plat::hotkey::{self, Chord, HotkeyError, HotkeyEvent};
use plat::screen::{self, Monitor, ScreenError, ScreenEvent, Shot};
use plat::{MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_SUPER};

use crate::T;

// the pattern's quadrants: red, green, blue, white (0xRRGGBB)
const QUAD: [u32; 4] = [0xFF0000, 0x00FF00, 0x0000FF, 0xFFFFFF];

/// a w x h frame: four quadrants of QUAD, B G R x
fn pattern(w: i32, h: i32) -> Vec<u8> {
    let mut px = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let q = (x >= w / 2) as usize + 2 * (y >= h / 2) as usize;
            let c = QUAD[q];
            let o = ((y * w + x) * 4) as usize;
            px[o..o + 4].copy_from_slice(&[c as u8, (c >> 8) as u8, (c >> 16) as u8, 255]);
        }
    }
    px
}

/// the centre of quadrant q of a w x h rectangle at x, y
fn quad_centre(r: [i32; 4], q: usize) -> (i32, i32) {
    let (qx, qy) = ((q & 1) as i32, (q >> 1) as i32);
    (r[0] + r[2] / 4 + qx * r[2] / 2, r[1] + r[3] / 4 + qy * r[3] / 2)
}

/// pump the system up to ms until the screen answers -> the answers
fn screen_wait(ms: u64) -> Vec<ScreenEvent> {
    let dl = plat::time_us() + ms * 1000;
    loop {
        plat::wait(dl.min(plat::time_us() + 50_000));
        while plat::evq_pop().is_some() {}
        let v = screen::screen_take();
        if !v.is_empty() || plat::time_us() >= dl {
            return v;
        }
    }
}

/// pump up to ms until the hotkey events satisfy done -> every event seen
fn hotkey_wait(ms: u64, done: &dyn Fn(&[HotkeyEvent]) -> bool) -> Vec<HotkeyEvent> {
    let dl = plat::time_us() + ms * 1000;
    let mut all = Vec::new();
    let debug = plat::env("jfgui_SCREEN_DEBUG").is_some();
    loop {
        plat::wait(dl.min(plat::time_us() + 50_000));
        while let Some(e) = plat::evq_pop() {
            if debug {
                crate::out(&format!("  event {} data {:x} mods {:x}\n", e.typ, e.data, e.mods));
            }
        }
        all.extend(hotkey::hotkey_take());
        if done(&all) || plat::time_us() >= dl {
            return all;
        }
    }
}

fn color_of(v: &[ScreenEvent]) -> Option<Result<u32, ScreenError>> {
    v.iter().find_map(|e| if let ScreenEvent::Color(r) = e { Some(r.clone()) } else { None })
}

fn shot_of(v: &[ScreenEvent]) -> Option<Result<Shot, ScreenError>> {
    v.iter().find_map(|e| if let ScreenEvent::Shot(r) = e { Some(r.clone()) } else { None })
}

fn mon_line(m: &Monitor) -> String {
    format!(
        "  monitor {:?} id {:x}: {},{} {}x{}, work {},{} {}x{}, {} dpi, {} mHz{}\n",
        m.name, m.id, m.rect[0], m.rect[1], m.rect[2], m.rect[3], m.work[0], m.work[1], m.work[2], m.work[3], m.dpi, m.refresh_mhz,
        if m.primary { ", primary" } else { "" }
    )
}

/// the checks every monitor list must pass
fn monitors_sane(t: &mut T, v: &[Monitor], what: &str) {
    for m in v {
        t.info(&mon_line(m));
    }
    t.check(!v.is_empty(), &format!("{what}: at least one monitor"));
    t.check(v.iter().filter(|m| m.primary).count() == 1 && v.first().is_some_and(|m| m.primary), &format!("{what}: exactly one primary, listed first"));
    t.check(v.iter().all(|m| m.rect[2] > 0 && m.rect[3] > 0), &format!("{what}: every monitor has a size"));
    let inside = |m: &Monitor| m.work[0] >= m.rect[0] && m.work[1] >= m.rect[1] && m.work[0] + m.work[2] <= m.rect[0] + m.rect[2] && m.work[1] + m.work[3] <= m.rect[1] + m.rect[3] && m.work[2] > 0 && m.work[3] > 0;
    t.check(v.iter().all(inside), &format!("{what}: every work area inside its monitor"));
    t.check(v.iter().all(|m| (48..=960).contains(&m.dpi)), &format!("{what}: dpi 48..960"));
    let mut ids: Vec<u64> = v.iter().map(|m| m.id).collect();
    ids.sort_unstable();
    ids.dedup();
    t.check(ids.len() == v.len(), &format!("{what}: distinct ids"));
    for m in v {
        let (cx, cy) = (m.rect[0] + m.rect[2] / 2, m.rect[1] + m.rect[3] / 2);
        let at = screen::monitor_at(cx, cy);
        if at.as_ref().map(|a| a.id) != Some(m.id) {
            t.check(false, &format!("{what}: monitor_at({cx}, {cy}) finds {:?}", m.name));
        }
    }
}

// ---------------------------------------------------------------- shared parts

fn chords(t: &mut T) {
    let cases: [(&str, Option<(u32, u8)>, &str, &str); 9] = [
        ("Ctrl+Alt+K", Some((0x4B, MOD_CTRL | MOD_ALT)), "Ctrl+Alt+K", "CTRL+ALT+k"),
        ("super+f12", Some((0x7B, MOD_SUPER)), "Super+F12", "LOGO+F12"),
        ("Shift+NumAdd", Some((0x6B, MOD_SHIFT)), "Shift+NumAdd", "SHIFT+KP_Add"),
        ("ctrl + space", Some((0x20, MOD_CTRL)), "Ctrl+Space", "CTRL+space"),
        ("Alt+Shift+Ctrl+7", Some((0x37, MOD_CTRL | MOD_ALT | MOD_SHIFT)), "Ctrl+Alt+Shift+7", "CTRL+ALT+SHIFT+7"),
        ("Win+Num5", Some((0x65, MOD_SUPER)), "Super+Num5", "LOGO+KP_5"),
        ("Ctrl+PageDown", Some((0x22, MOD_CTRL)), "Ctrl+PageDown", "CTRL+Page_Down"),
        ("F24", Some((0x87, 0)), "F24", "F24"),
        ("Strg+Alt+Entf", None, "", ""),
    ];
    for (s, want, text, trig) in cases {
        let c = Chord::parse(s);
        t.check(c.map(|c| (c.vk, c.mods)) == want, &format!("chord: parse {s:?}"));
        if let Some(c) = c {
            t.expect_s(Some(&c.text()), Some(text), &format!("chord: {s:?} as text"));
            t.expect_s(c.trigger().as_deref(), Some(trig), &format!("chord: {s:?} in the XDG notation"));
        }
    }
    for bad in ["", "Ctrl+", "Ctrl+K+L", "Hyper+K", "F25", "Ctrl+Alt"] {
        t.check(Chord::parse(bad).is_none(), &format!("chord: {bad:?} is refused"));
    }
}

fn dry_run(t: &mut T) {
    hotkey::set_dry_run(true);
    let f7 = Chord::parse("Ctrl+Shift+F7").unwrap_or_default();
    t.check(hotkey::hotkey_available(), "dry run: shortcuts available");
    t.check(hotkey::hotkey_register(5, f7, "Fünf") == Ok(()), "dry run: register");
    t.check(hotkey::hotkey_list() == vec![(5, f7)], "dry run: listed");
    t.check(hotkey::hotkey_register(6, f7, "Sechs") == Err(HotkeyError::Conflict), "dry run: another id of the app on the chord is Conflict");
    t.check(hotkey::hotkey_register(0, f7, "") == Err(HotkeyError::Invalid), "dry run: id 0 is Invalid");
    t.check(hotkey::hotkey_register(7, Chord { vk: 0, mods: MOD_CTRL }, "") == Err(HotkeyError::Invalid), "dry run: no key is Invalid");
    t.check(hotkey::hotkey_register(7, f7, "a\0b") == Err(HotkeyError::Invalid), "dry run: a description with NUL is Invalid");
    t.check(hotkey::hotkey_test_press(5) && hotkey::hotkey_take() == vec![HotkeyEvent::Pressed(5)], "dry run: a test press arrives as Pressed");
    t.check(!hotkey::hotkey_test_press(6), "dry run: no press for an id that is not registered");
    let f8 = Chord::parse("Ctrl+Shift+F8").unwrap_or_default();
    t.check(hotkey::hotkey_register(5, f8, "Fünf neu") == Ok(()) && hotkey::hotkey_list() == vec![(5, f8)], "dry run: the same id again replaces its chord");
    t.check(hotkey::hotkey_unregister(5) && !hotkey::hotkey_unregister(5), "dry run: unregister once");
    t.check(hotkey::hotkey_list().is_empty(), "dry run: nothing left");
    hotkey::set_dry_run(false);
    // shot_pixel: B, G, R, x rows
    let s = Shot { rect: [0, 0, 2, 1], w: 2, h: 1, px: vec![1, 2, 3, 255, 0x10, 0x20, 0x30, 255], file: None };
    t.check(screen::shot_pixel(&s, 1, 0) == Some(0x302010) && screen::shot_pixel(&s, 2, 0).is_none(), "shot_pixel: 0xRRGGBB, None outside");
    t.check(screen::capture_start(Some([0, 0, 0, 5])).is_err(), "capture_start: an empty rectangle is refused");
}

pub fn t_screen(t: &mut T) {
    chords(t);
    dry_run(t);
    native(t);
}

// ---------------------------------------------------------------- the other process

/// --test hotkey-hold --chord C --marker F --hold MS
pub fn t_hotkey_hold(_t: &mut T) {
    let (Some(chord), Some(marker)) = (plat::arg_value("--chord"), plat::arg_value("--marker")) else { return };
    let hold: u64 = plat::arg_value("--hold").and_then(|v| v.parse().ok()).unwrap_or(3000);
    #[cfg(target_os = "linux")]
    {
        plat::setenv("jfgui_BACKEND", Some("x11"));
        plat::setenv("jfgui_HOTKEYS", Some("x11"));
        let w = plat::win_create("plattest hotkey-hold", 64, 48, 0, plat::PW_NOOWNER, plat::PW_DEFAULT, plat::PW_DEFAULT);
        if w < 0 {
            plat::write_file(&marker, b"no window");
            return;
        }
    }
    let r = match Chord::parse(&chord) {
        Some(c) => hotkey::hotkey_register(1, c, "plattest hotkey-hold"),
        None => Err(HotkeyError::Invalid),
    };
    plat::write_file(&marker, if r.is_ok() { b"ok".as_slice() } else { b"failed" });
    let dl = plat::time_us() + hold * 1000;
    while plat::time_us() < dl {
        plat::wait(dl);
        while plat::evq_pop().is_some() {}
    }
    hotkey::hotkey_clear();
    plat::write_file(&format!("{marker}.done"), b"done");
}

/// start a process holding chord; wait for its marker -> its answer ("ok" / ...)
fn hold_elsewhere(t: &mut T, chord: &str, ms: u32) -> (String, String) {
    let marker = t.subp(&format!("hold-{:x}.txt", plat::time_us() & 0xFFFF));
    let hold = format!("{ms}");
    let r = plat::spawn_self(&["--test", "hotkey-hold", "--chord", chord, "--marker", &marker, "--hold", &hold]);
    t.check(r == 0, "conflict: the other process starts");
    for _ in 0..200 {
        if plat::file_exists(&marker) == 1 {
            break;
        }
        plat::sleep_ms(50);
    }
    let got = plat::read_file(&marker).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    (got, marker)
}

fn wait_done(marker: &str) {
    let done = format!("{marker}.done");
    for _ in 0..300 {
        if plat::file_exists(&done) == 1 {
            break;
        }
        plat::sleep_ms(50);
    }
    plat::sleep_ms(200); // the process ends after writing it
}

// ---------------------------------------------------------------- Windows

#[cfg(windows)]
fn native(t: &mut T) {
    t.scratch_init();
    // ---- the monitors against GetSystemMetrics
    let v = screen::monitors();
    monitors_sane(t, &v, "monitors");
    let (count, virt) = plat::screen_test_metrics();
    t.expect_i(v.len() as i64, count as i64, "monitors: as many as SM_CMONITORS");
    let (mut l, mut tp, mut r, mut b) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for m in &v {
        l = l.min(m.rect[0]);
        tp = tp.min(m.rect[1]);
        r = r.max(m.rect[0] + m.rect[2]);
        b = b.max(m.rect[1] + m.rect[3]);
    }
    t.check([l, tp, r - l, b - tp] == virt, "monitors: together the virtual desktop (SM_*VIRTUALSCREEN)");
    t.check(v.first().is_some_and(|m| m.rect[0] == 0 && m.rect[1] == 0), "monitors: the primary one at 0, 0");
    t.check(v.iter().all(|m| m.name.starts_with("\\\\.\\DISPLAY")), "monitors: device names \\\\.\\DISPLAYn");
    t.check(v.iter().all(|m| m.refresh_mhz >= 23000), "monitors: refresh rates (EnumDisplaySettingsW)");

    // ---- a topmost window with the pattern
    let w = plat::win_create("plattest screen", 160, 120, plat::PWF_NOACTIVATE, plat::PW_NOOWNER, v[0].work[0] + 40, v[0].work[1] + 40);
    t.check(w >= 0, "window: created");
    if w < 0 {
        t.scratch_done();
        return;
    }
    plat::win_show(w, plat::PWS_SHOWNA);
    plat::screen_test_topmost(w);
    let (pw, ph) = plat::win_size(w);
    let px = pattern(pw, ph);
    plat::win_present(w, &plat::Frame { px: &px, w: pw, h: ph, stride: pw * 4 }, None);
    let _ = screen_wait(400); // DWM composes it
    let rect = plat::win_get_rect(w).unwrap_or([0, 0, 0, 0]);
    t.info(&format!("  window at {},{} {}x{}\n", rect[0], rect[1], rect[2], rect[3]));

    // ---- monitor changes reach the app
    let monitors_event = || {
        let mut found = false;
        while let Some(e) = plat::evq_pop() {
            found |= e.typ == plat::E_SETTINGS && e.data == plat::SET_MONITORS;
        }
        found
    };
    monitors_event();
    plat::win_test_send(w, 0x007E, 32, 0); // WM_DISPLAYCHANGE
    t.check(monitors_event(), "changes: WM_DISPLAYCHANGE -> E_SETTINGS SET_MONITORS");
    plat::win_test_send(w, 0x001A, 0x2F, 0); // WM_SETTINGCHANGE SPI_SETWORKAREA
    t.check(monitors_event(), "changes: a work area change -> SET_MONITORS");

    // ---- capture
    t.check(screen::capture_available(), "capture: available");
    t.check(screen::capture_start(Some(rect)) == Ok(()), "capture: started");
    match shot_of(&screen_wait(2000)) {
        Some(Ok(s)) => {
            t.check(s.w == rect[2] && s.h == rect[3] && s.px.len() == (s.w * s.h * 4) as usize, "capture: the size asked for");
            let local = [0, 0, s.w, s.h];
            let ok = (0..4).all(|q| {
                let (x, y) = quad_centre(local, q);
                screen::shot_pixel(&s, x, y) == Some(QUAD[q])
            });
            t.check(ok, "capture: the window's four colours where they are");
        }
        other => t.check(false, &format!("capture: a picture ({other:?})")),
    }
    t.check(screen::capture_start(None).is_ok(), "capture: the whole desktop");
    match shot_of(&screen_wait(2000)) {
        Some(Ok(s)) => t.check(s.rect == virt && s.w == virt[2], "capture: the virtual desktop's size"),
        other => t.check(false, &format!("capture: the desktop ({other:?})")),
    }

    // ---- the pick
    t.check(screen::pick_available(), "pick: available");
    let (rx, ry) = quad_centre(rect, 0);
    screen::pick_test_point(Some((rx, ry)));
    t.check(screen::pick_start() == Ok(()), "pick: started (test point)");
    t.check(color_of(&screen_wait(2000)) == Some(Ok(0xFF0000)), "pick: the red quadrant at the test point");
    t.check(screen::pick_start() == Ok(()) && plat::pick_test_window() != 0, "pick: the overlay is shown");
    t.check(screen::pick_start() == Err(ScreenError::Busy), "pick: a second one is Busy");
    // the magnifier beside the pointer (gap 2 zoom, a 1 px frame, 9 x 9 cells of zoom px)
    // shows the red quadrant where the frozen picture has the blue one
    t.check(plat::pick_test_move(rx, ry), "pick: the pointer over the red quadrant");
    let _ = screen_wait(300);
    let z = (12 * v[0].dpi as i32 / 96).max(8);
    let (mx, my) = (rx + 2 * z + 1 + z + z / 2, ry + 2 * z + 1 + z / 2);
    t.check(screen::capture_start(Some([mx, my, 1, 1])) == Ok(()), "pick: the magnifier captured");
    let mag = shot_of(&screen_wait(2000)).and_then(|r| r.ok()).and_then(|s| screen::shot_pixel(&s, 0, 0));
    t.check(mag == Some(0xFF0000), &format!("pick: the magnifier shows the colour under the pointer ({mag:x?})"));
    let (gx, gy) = quad_centre(rect, 1);
    t.check(plat::pick_test_click(gx, gy), "pick: a click in the overlay");
    t.check(color_of(&screen_wait(2000)) == Some(Ok(0x00FF00)), "pick: the green quadrant from the frozen picture");
    t.check(plat::pick_test_window() == 0, "pick: the overlay is gone");
    t.check(screen::pick_start() == Ok(()) && plat::pick_test_cancel(), "pick: Esc in the overlay");
    t.check(color_of(&screen_wait(2000)) == Some(Err(ScreenError::Cancelled)), "pick: Esc cancels");

    // ---- global shortcuts
    t.check(hotkey::hotkey_available(), "hotkey: available");
    // an unused one of F24 .. F13 (no modifiers: no state left behind by SendInput)
    let mut key = None;
    for vk in (0x7C..=0x87u32).rev() {
        match hotkey::hotkey_register(1, Chord { vk, mods: 0 }, "plattest") {
            Ok(()) => {
                key = Some(vk);
                break;
            }
            Err(HotkeyError::Conflict) => continue,
            Err(e) => {
                t.check(false, &format!("hotkey: register F{} ({e})", vk - 0x6F));
                break;
            }
        }
    }
    t.check(key.is_some(), "hotkey: one of F13-F24 registered");
    if let Some(vk) = key {
        t.info(&format!("  hotkey F{}\n", vk - 0x6F));
        t.check(hotkey::hotkey_register(2, Chord { vk, mods: 0 }, "zwei") == Err(HotkeyError::Conflict), "hotkey: a second id of the app on the chord is Conflict");
        t.expect_i(plat::hotkey_test_input(vk) as i64, 2, "hotkey: SendInput took the key press and release");
        let got = hotkey_wait(3000, &|v| v.contains(&HotkeyEvent::Pressed(1)));
        t.check(got == vec![HotkeyEvent::Pressed(1)], &format!("hotkey: the system's WM_HOTKEY arrives once as Pressed ({got:?})"));
        t.check(plat::hotkey_test_post(1), "hotkey: WM_HOTKEY posted to the window");
        let got = hotkey_wait(2000, &|v| !v.is_empty());
        t.check(got == vec![HotkeyEvent::Pressed(1)], "hotkey: the posted WM_HOTKEY arrives as Pressed");
        t.check(hotkey::hotkey_unregister(1), "hotkey: unregister");
        t.check(plat::hotkey_test_input(vk) == 2 && hotkey_wait(600, &|v| !v.is_empty()).is_empty(), "hotkey: no press after unregister");
    }
    // a chord another process holds
    let chord = "Ctrl+Alt+Shift+F22";
    let (got, marker) = hold_elsewhere(t, chord, 2500);
    t.expect_s(Some(&got), Some("ok"), "conflict: the other process holds Ctrl+Alt+Shift+F22");
    let c = Chord::parse(chord).unwrap_or_default();
    t.check(hotkey::hotkey_register(3, c, "drei") == Err(HotkeyError::Conflict), "conflict: RegisterHotKey refused it (Conflict)");
    wait_done(&marker);
    t.check(hotkey::hotkey_register(3, c, "drei") == Ok(()), "conflict: free again after the other process ended");
    hotkey::hotkey_clear();
    t.check(hotkey::hotkey_list().is_empty(), "hotkey: cleared");
    plat::win_destroy(w);
    t.scratch_done();
}

// ---------------------------------------------------------------- Linux (X11)

#[cfg(target_os = "linux")]
fn native(t: &mut T) {
    if plat::env("DISPLAY").is_none_or(|d| d.is_empty()) {
        t.info("  SKIP: no $DISPLAY (the X11 part)\n");
        return;
    }
    t.scratch_init();
    plat::setenv("jfgui_BACKEND", Some("x11"));
    plat::setenv("jfgui_HOTKEYS", Some("x11"));
    plat::setenv("jfgui_SCREEN", Some("x11"));
    let w = plat::win_create("plattest screen", 160, 120, 0, plat::PW_NOOWNER, plat::PW_DEFAULT, plat::PW_DEFAULT);
    t.check(w >= 0 && plat::disp_backend() == plat::BK_X11, "x11: a window on the X server");
    if w < 0 {
        t.scratch_done();
        return;
    }
    plat::win_show(w, plat::PWS_SHOW);
    let (pw, ph) = plat::win_size(w);
    let px = pattern(pw, ph);
    let mut rect = None;
    let dl = plat::time_us() + 5_000_000;
    while plat::time_us() < dl && rect.is_none() {
        plat::wait(plat::time_us() + 50_000);
        while plat::evq_pop().is_some() {}
        plat::win_present(w, &plat::Frame { px: &px, w: pw, h: ph, stride: pw * 4 }, None);
        rect = plat::win_get_rect(w);
    }
    let _ = screen_wait(500);
    plat::win_present(w, &plat::Frame { px: &px, w: pw, h: ph, stride: pw * 4 }, None);
    let _ = screen_wait(300);
    let rect = rect.unwrap_or([0, 0, pw, ph]);
    t.info(&format!("  window at {},{} {}x{}\n", rect[0], rect[1], rect[2], rect[3]));

    // ---- RandR
    let v = screen::monitors();
    monitors_sane(t, &v, "x11 monitors");
    t.check(v.iter().all(|m| !m.name.is_empty() && m.refresh_mhz > 0), "x11 monitors: RandR names and the CRTC modes' refresh rates");

    // ---- the GetImage path on the window, the root's
    match plat::x11t::image_of(w, [0, 0, pw, ph]) {
        Ok(s) => {
            let ok = (0..4).all(|q| {
                let (x, y) = quad_centre([0, 0, pw, ph], q);
                screen::shot_pixel(&s, x, y) == Some(QUAD[q])
            });
            t.check(ok && s.w == pw && s.h == ph, "x11: GetImage (in strips) gives the window's four colours");
        }
        Err(e) => t.check(false, &format!("x11: GetImage of the window ({e})")),
    }
    t.check(screen::capture_start(Some(rect)) == Ok(()), "x11 capture: started");
    match shot_of(&screen_wait(3000)) {
        Some(Ok(s)) => {
            let local = [0, 0, s.w, s.h];
            let ok = (0..4).all(|q| {
                let (x, y) = quad_centre(local, q);
                screen::shot_pixel(&s, x, y) == Some(QUAD[q])
            });
            t.check(ok, "x11 capture: the root's picture shows the window's colours");
        }
        Some(Err(ScreenError::Failed(m))) if m.contains("rootless") => {
            t.info("  (the X server shows no root picture: rootless Xwayland)\n");
            t.check(true, "x11 capture: a rootless server's refusal is Failed, not a crash");
        }
        other => t.check(false, &format!("x11 capture: {other:?}")),
    }

    // ---- the pick
    t.check(screen::pick_available(), "x11 pick: available");
    let (gx, gy) = quad_centre(rect, 1);
    screen::pick_test_point(Some((gx, gy)));
    t.check(screen::pick_start() == Ok(()), "x11 pick: started (test point)");
    t.check(color_of(&screen_wait(3000)) == Some(Ok(0x00FF00)), "x11 pick: the green quadrant at the test point");
    t.check(screen::pick_start() == Ok(()), "x11 pick: the pointer grabbed");
    // the click as the server reports it during the grab: ButtonPress on the root at the blue
    // quadrant (synthetic: XTEST does not move WSLg's pointer, its buttons do arrive)
    let (bx, by) = quad_centre(rect, 2);
    let root = plat::x11t::root();
    for typ in [4u8, 5] {
        let mut p = [0u8; 32];
        p[0] = typ;
        p[1] = 1;
        p[8..12].copy_from_slice(&root.to_le_bytes());
        p[12..16].copy_from_slice(&root.to_le_bytes());
        p[20..22].copy_from_slice(&(bx as i16).to_le_bytes());
        p[22..24].copy_from_slice(&(by as i16).to_le_bytes());
        p[30] = 1;
        plat::x11t::event(&p);
    }
    t.check(color_of(&screen_wait(5000)) == Some(Ok(0x0000FF)), "x11 pick: a click on the blue quadrant (read after the event)");
    let fake = plat::x11t::fake_input(6, 0, bx, by);
    if fake {
        let esc = plat::x11t::vk_keycodes(0x1B).first().copied().unwrap_or(9);
        t.check(screen::pick_start() == Ok(()), "x11 pick: again");
        plat::x11t::fake_input(2, esc, 0, 0);
        plat::x11t::fake_input(3, esc, 0, 0);
        t.check(color_of(&screen_wait(5000)) == Some(Err(ScreenError::Cancelled)), "x11 pick: Esc (XTEST) cancels");
    } else {
        t.info("  (no XTEST: the pick cancelled through Esc is not tried)\n");
        let _ = screen_wait(100);
    }
    let _ = screen_wait(300);
    let left = screen::screen_take();
    t.check(left.is_empty() && screen::pick_start().is_ok(), "x11 pick: no grab left behind, a new pick starts");
    plat::x11t::fake_input(4, 3, 0, 0);
    plat::x11t::fake_input(5, 3, 0, 0);
    if color_of(&screen_wait(5000)) != Some(Err(ScreenError::Cancelled)) {
        t.check(false, "x11 pick: a right click cancels");
    }

    // ---- global shortcuts
    t.check(hotkey::hotkey_available(), "x11 hotkey: available");
    let c = Chord::parse("Ctrl+Alt+K").unwrap_or_default();
    t.check(hotkey::hotkey_register(1, c, "plattest") == Ok(()), "x11 hotkey: XGrabKey Ctrl+Alt+K");
    let kc = |vks: &[u32]| vks.iter().find_map(|&v| plat::x11t::vk_keycodes(v).first().copied());
    let (ctrl, alt, key) = (kc(&[0xA2, 0x11]), kc(&[0xA4, 0x12]), kc(&[0x4B]));
    if let (Some(ctrl), Some(alt), Some(key)) = (ctrl, alt, key) {
        if fake {
            // a passive grab fires only while the focus is the root or inside it (WSLg: a
            // window of an X client, else the keys go nowhere): our window takes it for the
            // keys, the old focus afterwards.  (Ctrl+Alt+F1..F12 would switch the console: the
            // server keeps them.)
            let before = plat::x11t::focus();
            plat::x11t::set_focus(plat::x11s::main_xid(), 2);
            // Xwayland drops the first key of a new XTEST burst (measured on WSLg): Ctrl once
            // before the chord
            plat::x11t::fake_input(2, ctrl, 0, 0);
            plat::x11t::fake_input(3, ctrl, 0, 0);
            let _ = screen_wait(500);
            while plat::evq_pop().is_some() {}
            for (typ, k) in [(2, ctrl), (2, alt), (2, key), (2, key), (3, key), (3, alt), (3, ctrl)] {
                plat::x11t::fake_input(typ, k, 0, 0);
            }
            let got = hotkey_wait(5000, &|v| !v.is_empty());
            let _ = hotkey_wait(500, &|_| false);
            plat::x11t::set_focus(before.0, before.1);
            t.check(got == vec![HotkeyEvent::Pressed(1)], &format!("x11 hotkey: Ctrl+Alt+K (XTEST) arrives once as Pressed ({got:?})"));
        }
        // the lock combinations: Num Lock and Caps Lock held (synthetic events on the root)
        let (alt_mask, num_mask, _) = plat::x11t::roles();
        let root = plat::x11t::root();
        let alt_bit = if alt_mask != 0 { alt_mask & alt_mask.wrapping_neg() } else { 8 };
        for (state, what) in [(4 | alt_bit | num_mask, "Num Lock"), (4 | alt_bit | 2, "Caps Lock"), (4 | alt_bit | 2 | num_mask, "both locks")] {
            for typ in [2u8, 3] {
                let mut p = [0u8; 32];
                p[0] = typ;
                p[1] = key as u8;
                p[8..12].copy_from_slice(&root.to_le_bytes());
                p[12..16].copy_from_slice(&root.to_le_bytes());
                p[28..30].copy_from_slice(&(state as u16).to_le_bytes());
                p[30] = 1;
                plat::x11t::event(&p);
            }
            let got = hotkey::hotkey_take();
            t.check(got == vec![HotkeyEvent::Pressed(1)], &format!("x11 hotkey: pressed with {what}"));
        }
        let mut p = [0u8; 32];
        p[0] = 2;
        p[1] = key as u8;
        p[8..12].copy_from_slice(&root.to_le_bytes());
        p[12..16].copy_from_slice(&root.to_le_bytes());
        p[28..30].copy_from_slice(&(4u16 | 1).to_le_bytes()); // Ctrl + Shift: not the chord
        plat::x11t::event(&p);
        p[0] = 3;
        plat::x11t::event(&p);
        t.check(hotkey::hotkey_take().is_empty(), "x11 hotkey: another modifier set is not the chord");
    } else {
        t.check(false, "x11 hotkey: keycodes of Ctrl, Alt and K");
    }
    t.check(hotkey::hotkey_unregister(1), "x11 hotkey: ungrabbed");
    // a chord another process grabbed
    let chord = "Ctrl+Alt+J";
    let (got, marker) = hold_elsewhere(t, chord, 3000);
    t.expect_s(Some(&got), Some("ok"), "x11 conflict: the other process grabbed Ctrl+Alt+J");
    let c = Chord::parse(chord).unwrap_or_default();
    t.check(hotkey::hotkey_register(2, c, "zwei") == Err(HotkeyError::Conflict), "x11 conflict: BadAccess is Conflict");
    t.check(hotkey::hotkey_list().is_empty(), "x11 conflict: nothing recorded (the grabs made were taken back)");
    wait_done(&marker);
    t.check(hotkey::hotkey_register(2, c, "zwei") == Ok(()), "x11 conflict: free again after the other process ended");
    hotkey::hotkey_clear();
    plat::win_destroy(w);
    let _ = screen_wait(100);
    t.scratch_done();
}

#[cfg(not(any(windows, target_os = "linux")))]
fn native(_t: &mut T) {}

// ---------------------------------------------------------------- Linux: Wayland, the portals

/// --test screen-wl: the Wayland outputs as monitors
#[cfg(not(target_os = "linux"))]
pub fn t_screen_wl(t: &mut T) {
    t.info("  SKIP: Wayland is Linux only\n");
}

/// --test screen-wl: the Wayland outputs as monitors
#[cfg(target_os = "linux")]
pub fn t_screen_wl(t: &mut T) {
    if plat::env("WAYLAND_DISPLAY").is_none() {
        t.info("  SKIP: no Wayland compositor\n");
        return;
    }
    plat::setenv("jfgui_BACKEND", Some("wayland"));
    let w = plat::win_create("plattest screen-wl", 160, 120, 0, plat::PW_NOOWNER, plat::PW_DEFAULT, plat::PW_DEFAULT);
    t.check(w >= 0 && plat::disp_backend() == plat::BK_WAYLAND, "wayland: a window on the compositor");
    if w < 0 {
        return;
    }
    plat::win_show(w, plat::PWS_SHOW);
    let _ = screen_wait(800);
    let v = screen::monitors();
    monitors_sane(t, &v, "wayland monitors");
    t.check(v.iter().all(|m| m.refresh_mhz > 0), "wayland monitors: the modes' refresh rates");
    t.check(v.iter().all(|m| m.work == m.rect), "wayland monitors: the work area is the monitor (unknown)");
    plat::win_destroy(w);
}

/// --test screen-bus --mode grant|deny|none (tools/platshared/screen_bus.py)
pub fn t_screen_bus(t: &mut T) {
    let mode = plat::arg_value("--mode").unwrap_or_default();
    plat::setenv("jfgui_SCREEN", Some("portal"));
    plat::setenv("jfgui_HOTKEYS", Some("portal"));
    let k = |s: &str| Chord::parse(s).unwrap_or_default();
    match mode.as_str() {
        "grant" => {
            t.check(screen::pick_available() && screen::capture_available() && hotkey::hotkey_available(), "bus: the portals are there (versions)");
            t.check(screen::pick_start() == Ok(()), "bus: PickColor asked");
            t.check(color_of(&screen_wait(10_000)) == Some(Ok(0x336699)), "bus: the colour (0.2, 0.4, 0.6) -> #336699");
            t.check(screen::capture_start(None) == Ok(()), "bus: Screenshot asked");
            match shot_of(&screen_wait(10_000)) {
                Some(Ok(s)) => {
                    let f = s.file.clone().unwrap_or_default();
                    t.info(&format!("  file {} ({} bytes)\n", f.0, f.1.len()));
                    t.check(f.0.ends_with("/shot 1.png") && f.1.starts_with(b"\x89PNG") && s.w == 3 && s.h == 2 && s.px.is_empty(), "bus: the file of the uri (%20 decoded), its PNG size 3 x 2");
                }
                other => t.check(false, &format!("bus: the screenshot ({other:?})")),
            }
            t.check(hotkey::hotkey_register(7, k("Ctrl+Alt+K"), "Zeigen") == Ok(()), "bus: register 7");
            t.check(hotkey::hotkey_register(8, k("Ctrl+Alt+J"), "Verstecken") == Ok(()), "bus: register 8");
            t.check(hotkey::hotkey_register(9, k("Super+F12"), "Nicht gebunden") == Ok(()), "bus: register 9");
            let got = hotkey_wait(10_000, &|v| v.contains(&HotkeyEvent::Pressed(7)) && v.iter().any(|e| matches!(e, HotkeyEvent::Refused(9, _))));
            t.check(got.contains(&HotkeyEvent::Refused(9, HotkeyError::Dismissed)), &format!("bus: the shortcut the desktop did not bind is Refused ({got:?})"));
            t.check(got.contains(&HotkeyEvent::Pressed(7)), "bus: Activated -> Pressed");
            t.check(!got.iter().any(|e| matches!(e, HotkeyEvent::Pressed(9) | HotkeyEvent::Pressed(8))), "bus: no press of an unbound or other id");
            t.check(hotkey::hotkey_list().iter().map(|e| e.0).collect::<Vec<_>>() == vec![7, 8], "bus: 7 and 8 stay registered");
            t.check(hotkey::hotkey_unregister(8), "bus: unregister 8");
            let _ = hotkey_wait(1500, &|_| false);
            hotkey::hotkey_clear();
            let _ = hotkey_wait(800, &|_| false);
        }
        "deny" => {
            t.check(screen::pick_start() == Ok(()), "bus deny: PickColor asked");
            t.check(color_of(&screen_wait(10_000)) == Some(Err(ScreenError::Cancelled)), "bus deny: Response 1 is Cancelled");
            t.check(screen::capture_start(None) == Ok(()), "bus deny: Screenshot asked");
            t.check(shot_of(&screen_wait(10_000)) == Some(Err(ScreenError::Denied)), "bus deny: Response 2 is Denied");
            t.check(hotkey::hotkey_register(7, k("Ctrl+Alt+K"), "Zeigen") == Ok(()), "bus deny: register 7");
            let got = hotkey_wait(10_000, &|v| !v.is_empty());
            t.check(got == vec![HotkeyEvent::Refused(7, HotkeyError::Dismissed)], &format!("bus deny: the cancelled dialog is Refused(Dismissed) ({got:?})"));
            t.check(hotkey::hotkey_list().is_empty(), "bus deny: nothing registered");
            let _ = hotkey_wait(500, &|_| false);
        }
        _ => {
            t.check(!screen::pick_available() && !screen::capture_available() && !hotkey::hotkey_available(), "bus none: nothing available");
            t.check(screen::pick_start() == Ok(()), "bus none: the pick is tried");
            t.check(color_of(&screen_wait(10_000)) == Some(Err(ScreenError::Unavailable)), "bus none: no portal is Unavailable");
            t.check(hotkey::hotkey_register(7, k("Ctrl+Alt+K"), "Zeigen") == Err(HotkeyError::Unavailable), "bus none: register is Unavailable");
        }
    }
}

/// --test screen-policy: a manifest without input.global and screen.capture
pub fn t_screen_policy(t: &mut T) {
    t.check(plat::policy::install("fs.user\n").is_ok(), "policy: a manifest installed");
    hotkey::set_dry_run(true); // the right is checked before anything is registered
    t.check(hotkey::hotkey_register(1, Chord { vk: 0x86, mods: MOD_CTRL }, "") == Err(HotkeyError::Denied), "policy: no input.global -> Denied");
    t.check(screen::capture_start(None) == Err(ScreenError::Denied), "policy: no screen.capture -> Denied");
    let d = plat::policy::denials();
    t.check(d.iter().any(|x| x.kind == "input.global") && d.iter().any(|x| x.kind == "screen.capture"), "policy: both refusals recorded");
    hotkey::set_dry_run(false);
    t.check(plat::policy::check("input.global\nscreen.capture\nask screen.capture \"Bildschirmfoto\"\n").is_ok(), "policy: the rights parse (also asked)");
    t.check(plat::policy::check("input.global x\n").is_err(), "policy: input.global takes no arguments");
}
