// t_input -- (G7a) where keys sit and the pointer lock without real input: the translation
// tables of the platforms' key codes to USB HID usages, the decoding of raw motion (Windows'
// RAWINPUT records, XInput 2 raw events, Wayland's fixed point), the shared state of the lock
// (events, motion that adds up, Esc) and, on Linux, the Wayland and X11 paths with synthetic
// events (no compositor, no server)
//
//   plattest --test input

use alloc::format;
use alloc::vec::Vec;

use plat::*;

use crate::T;

pub fn t_input(t: &mut T) {
    tables(t);
    decoding(t);
    state(t);
    #[cfg(target_os = "linux")]
    linux(t);
}

// ------------------------------------------------------------------ the key tables

fn tables(t: &mut T) {
    let rows = key_code_rows_test();
    t.check(rows.len() >= 120, &format!("input: {} rows in the key table", rows.len()));
    let (mut bad, mut dup) = (Vec::new(), Vec::new());
    for (i, &(u, win, ev, mac, name)) in rows.iter().enumerate() {
        if win != 0 && key_code_set1(win) != u {
            bad.push(format!("set1 {win:#x}"));
        }
        if ev != 0 && key_code_evdev(ev) != u {
            bad.push(format!("evdev {ev}"));
        }
        if mac != 0xFF && key_code_mac(mac) != u {
            bad.push(format!("mac {mac:#x}"));
        }
        if key_code_from_name(name) != u || key_code_name(u) != name {
            bad.push(format!("name {name}"));
        }
        // each platform's code at most once (else a key would come out as two usages)
        for &(u2, win2, ev2, mac2, name2) in &rows[i + 1..] {
            if u2 == u || name2 == name || (win != 0 && win2 == win) || (ev != 0 && ev2 == ev) || (mac != 0xFF && mac2 == mac) {
                dup.push(format!("{name}/{name2}"));
            }
        }
        // this platform's scan code goes back to the usage
        let sc = key_scancode(u);
        if sc != 0 && key_code(sc) != u {
            bad.push(format!("scancode of {name}"));
        }
    }
    t.check(bad.is_empty(), &format!("input: every row's codes give its usage {bad:?}"));
    t.check(dup.is_empty(), &format!("input: no usage, name or platform code twice {dup:?}"));
    // the usages of the USB HID tables (keyboard page) and the places that matter for games
    let fixed = [(KC_A, 0x04), (KC_Z, 0x1D), (KC_1, 0x1E), (KC_0, 0x27), (KC_ENTER, 0x28), (KC_ESCAPE, 0x29), (KC_SPACE, 0x2C),
                 (KC_F1, 0x3A), (KC_F12, 0x45), (KC_RIGHT, 0x4F), (KC_UP, 0x52), (KC_NUMPAD0, 0x62), (KC_INTL_BACKSLASH, 0x64),
                 (KC_CONTROL_LEFT, 0xE0), (KC_SHIFT_LEFT, 0xE1), (KC_META_RIGHT, 0xE7)];
    t.check(fixed.iter().all(|&(k, v)| k == v), "input: KC_* are the HID usages");
    // Windows: set 1 with 0xE0.. for the extended keys, its two swapped ones, Alt+Print, Ctrl+Pause
    let win = [(0x11, KC_W), (0x1E, KC_A), (0x2A, KC_SHIFT_LEFT), (0x36, KC_SHIFT_RIGHT), (0x1D, KC_CONTROL_LEFT),
               (0xE01D, KC_CONTROL_RIGHT), (0xE038, KC_ALT_RIGHT), (0xE048, KC_UP), (0x48, KC_NUMPAD8), (0xE01C, KC_NUMPAD_ENTER),
               (0x1C, KC_ENTER), (0x45, KC_PAUSE), (0xE045, KC_NUM_LOCK), (0x54, KC_PRINT_SCREEN), (0xE037, KC_PRINT_SCREEN),
               (0xE046, KC_PAUSE), (0x56, KC_INTL_BACKSLASH), (0xE05B, KC_META_LEFT), (0x57, KC_F11), (0, 0), (0xE0FF, 0)];
    let w_bad: Vec<_> = win.iter().filter(|&&(sc, k)| key_code_set1(sc) != k).collect();
    t.check(w_bad.is_empty(), &format!("input: Windows scan codes {w_bad:x?}"));
    // Linux: evdev (the X keycode minus 8)
    let lx = [(17, KC_W), (30, KC_A), (1, KC_ESCAPE), (28, KC_ENTER), (96, KC_NUMPAD_ENTER), (97, KC_CONTROL_RIGHT),
              (100, KC_ALT_RIGHT), (103, KC_UP), (125, KC_META_LEFT), (127, KC_CONTEXT_MENU), (86, KC_INTL_BACKSLASH),
              (119, KC_PAUSE), (183, KC_F13), (0, 0), (255, 0)];
    let l_bad: Vec<_> = lx.iter().filter(|&&(c, k)| key_code_evdev(c) != k).collect();
    t.check(l_bad.is_empty(), &format!("input: evdev codes {l_bad:?}"));
    // macOS: kVK_* (kVK_ANSI_A is 0)
    let mac = [(0x00, KC_A), (0x0D, KC_W), (0x01, KC_S), (0x02, KC_D), (0x0C, KC_Q), (0x06, KC_Z), (0x32, KC_BACKQUOTE),
               (0x0A, KC_INTL_BACKSLASH), (0x24, KC_ENTER), (0x4C, KC_NUMPAD_ENTER), (0x35, KC_ESCAPE), (0x37, KC_META_LEFT),
               (0x36, KC_META_RIGHT), (0x3A, KC_ALT_LEFT), (0x3D, KC_ALT_RIGHT), (0x7E, KC_UP), (0x72, KC_INSERT), (0x7A, KC_F1),
               (0x3F, 0), (0xFF, 0), (0x1000, 0)];
    let m_bad: Vec<_> = mac.iter().filter(|&&(c, k)| key_code_mac(c) != k).collect();
    t.check(m_bad.is_empty(), &format!("input: macOS key codes {m_bad:x?}"));
    // the browser: KeyboardEvent.code names, Firefox's older ones
    let web = [("KeyW", KC_W), ("ShiftLeft", KC_SHIFT_LEFT), ("Space", KC_SPACE), ("ArrowUp", KC_UP), ("NumpadEnter", KC_NUMPAD_ENTER),
               ("IntlBackslash", KC_INTL_BACKSLASH), ("MetaLeft", KC_META_LEFT), ("OSLeft", KC_META_LEFT), ("OSRight", KC_META_RIGHT),
               ("VolumeUp", KC_VOLUME_UP), ("AudioVolumeUp", KC_VOLUME_UP), ("", 0), ("Fn", 0), ("keyw", 0)];
    let b_bad: Vec<_> = web.iter().filter(|&&(n, k)| key_code_from_name(n) != k).collect();
    t.check(b_bad.is_empty(), &format!("input: browser code names {b_bad:?}"));
    t.check(key_code_name(KC_W) == "KeyW" && key_code_name(0) == "" && key_code_name(0x32) == "" && key_code_name(0x1_0000) == "",
            "input: the browser's name of a usage; none for 0, 0x32 (no row) and beyond");
    t.check(key_label(0).is_none() && key_label(0x32).is_none(), "input: key_label of no key: none");
    // what this platform's events carry
    let w = key_scancode(KC_W);
    #[cfg(windows)]
    t.check(w == 0x11 && key_code(0xE048) == KC_UP, "input: Windows' Event::x is set 1");
    #[cfg(any(target_os = "linux", target_os = "android"))]
    t.check(w == 17 && key_code(103) == KC_UP, "input: Linux's Event::x is evdev");
    #[cfg(not(any(windows, target_os = "linux", target_os = "android")))]
    t.check(w == KC_W && key_code(KC_UP) == KC_UP && key_code(0x32) == 0, "input: Event::x is the usage itself");
    t.info(&format!("  (key_label: W {:?}, A {:?}, Esc {:?})\n", key_label(KC_W), key_label(KC_A), key_label(KC_ESCAPE)));
}

// ------------------------------------------------------------------ raw motion decoding

/// a RAWINPUT record of 64-bit Windows: header (dwType, dwSize, hDevice, wParam), RAWMOUSE
fn rawinput(typ: u32, flags: u16, x: i32, y: i32) -> Vec<u8> {
    let mut b = alloc::vec![0u8; 48];
    b[0..4].copy_from_slice(&typ.to_le_bytes());
    b[4..8].copy_from_slice(&48u32.to_le_bytes());
    b[8..16].copy_from_slice(&0x1234u64.to_le_bytes()); // hDevice
    b[24..26].copy_from_slice(&flags.to_le_bytes());
    b[28..30].copy_from_slice(&1u16.to_le_bytes()); // usButtonFlags: left down
    b[36..40].copy_from_slice(&x.to_le_bytes());
    b[40..44].copy_from_slice(&y.to_le_bytes());
    b
}

/// an XI_RawMotion: device, source, time, the mask words, then transformed and raw values (FP3232)
fn xi_raw(op: u8, dev: u16, src: u16, time: u32, mask: &[u32], acc: &[f64], raw: &[f64]) -> Vec<u8> {
    let mut b = alloc::vec![0u8; 32];
    b[0] = 35;
    b[1] = op;
    b[8..10].copy_from_slice(&17u16.to_le_bytes());
    b[10..12].copy_from_slice(&dev.to_le_bytes());
    b[12..16].copy_from_slice(&time.to_le_bytes());
    b[20..22].copy_from_slice(&src.to_le_bytes());
    b[22..24].copy_from_slice(&(mask.len() as u16).to_le_bytes());
    for m in mask {
        b.extend_from_slice(&m.to_le_bytes());
    }
    for v in acc.iter().chain(raw) {
        let int = libm_floor(*v);
        let frac = ((*v - int) * 4_294_967_296.0) as u64 as u32;
        b.extend_from_slice(&(int as i32).to_le_bytes());
        b.extend_from_slice(&frac.to_le_bytes());
    }
    let extra = ((b.len() - 32) / 4) as u32;
    b[4..8].copy_from_slice(&extra.to_le_bytes());
    b
}

fn libm_floor(v: f64) -> f64 {
    let i = v as i64 as f64;
    if i > v { i - 1.0 } else { i }
}

fn decoding(t: &mut T) {
    t.check(raw_mouse_decode(&rawinput(0, 0, -7, 12)) == Some((0, -7, 12)), "input: RAWMOUSE relative motion -7, 12");
    t.check(raw_mouse_decode(&rawinput(0, 1, 32768, 16384)) == Some((1, 32768, 16384)), "input: RAWMOUSE absolute (MOUSE_MOVE_ABSOLUTE)");
    t.check(raw_mouse_decode(&rawinput(1, 0, 5, 5)).is_none(), "input: RAWINPUT of a keyboard: none");
    t.check(raw_mouse_decode(&rawinput(0, 0, 5, 5)[..40]).is_none(), "input: a short RAWINPUT: none");
    let mut small = rawinput(0, 0, 5, 5);
    small[4..8].copy_from_slice(&40u32.to_le_bytes());
    t.check(raw_mouse_decode(&small).is_none(), "input: RAWINPUT whose dwSize is too small: none");
    let (dx, dy) = raw_abs_delta(Some([0, 0]), [32768, 16384], [1920, 1080]);
    t.check((dx - 960.0146).abs() < 0.01 && (dy - 270.0041).abs() < 0.01, &format!("input: absolute motion in pixels ({dx}, {dy})"));
    t.check(raw_abs_delta(None, [32768, 16384], [1920, 1080]) == (0.0, 0.0), "input: the first absolute position moves nothing");
    // XInput 2: raw values (after the transformed ones), valuators 0 and 1
    let e = xi_raw(131, 2, 9, 1000, &[0b11], &[20.0, -10.0], &[5.5, -2.25]);
    t.check(xi_raw_decode(&e) == Some((2, 9, 1000, 5.5, -2.25, 3)), &format!("input: XI_RawMotion x, y raw {:?}", xi_raw_decode(&e)));
    let e = xi_raw(131, 2, 9, 1001, &[0b1010], &[1.0, 2.0], &[-3.0, 7.0]);
    t.check(xi_raw_decode(&e) == Some((2, 9, 1001, 0.0, -3.0, 2)), "input: XI_RawMotion with valuators 1 and 3: y alone");
    let e = xi_raw(131, 2, 9, 1002, &[0, 1], &[4.0], &[8.0]);
    t.check(xi_raw_decode(&e) == Some((2, 9, 1002, 0.0, 0.0, 0)), "input: XI_RawMotion of valuator 32 only: neither x nor y");
    let mut e = xi_raw(131, 2, 9, 1003, &[0b11], &[1.0, 1.0], &[1.0, 1.0]);
    e.truncate(e.len() - 8);
    t.check(xi_raw_decode(&e).is_none() && xi_raw_decode(&e[..20]).is_none(), "input: a short XI_RawMotion: none");
    t.check(wl_fixed(3 * 256 + 128) == 3.5 && wl_fixed(-64) == -0.25, "input: Wayland's 24.8 fixed point");
}

// ------------------------------------------------------------------ the shared state

fn pop_typ(typ: u8) -> Option<Event> {
    let mut found = None;
    while let Some(e) = evq_pop() {
        if e.typ == typ && found.is_none() {
            found = Some(e);
        }
    }
    found
}

fn state(t: &mut T) {
    evq_clear();
    // no such window: refused, nothing changes
    t.check(win_pointer_lock(7, true) == PointerLock::Refused || cfg!(any(target_os = "android", target_os = "ios")),
            "input: the lock of a window that does not exist: refused");
    t.check(win_pointer_lock(7, false) != PointerLock::Locked, "input: letting a missing window's lock go");
    evq_clear();
    pointer_lock_test(0, PointerLock::Locked);
    let e = pop_typ(E_POINTER_LOCK);
    t.check(e.is_some_and(|e| e.win == 0 && e.data == 1), "input: locked -> E_POINTER_LOCK data 1");
    t.check(win_pointer_lock_state(0) == PointerLock::Locked && win_pointer_lock_state(1) == PointerLock::Off,
            "input: the state of the window that holds it, Off for another");
    raw_motion_test(0, 3.0, -2.0);
    raw_motion_test(0, 1.5, 0.5);
    raw_motion_test(1, 100.0, 100.0); // (another window)
    raw_motion_test(0, f64::NAN, 1.0);
    t.check(evq_count() == 1 && evq_pop().is_some_and(|e| e.typ == E_RAW_MOTION && e.win == 0),
            "input: one E_RAW_MOTION for the motion until it is taken");
    t.check(win_raw_motion_take(0) == (4.5, -1.5) && win_raw_motion_take(0) == (0.0, 0.0), "input: the motion adds up, taken once");
    t.check(win_raw_motion_take(1) == (0.0, 0.0), "input: nothing for a window without the lock");
    raw_motion_test(0, 1.0, 1.0);
    t.check(pop_typ(E_RAW_MOTION).is_some(), "input: after the take the next motion has an event of its own");
    // Esc: down and its repeat and up belong to the lock; other keys do not
    t.check(!pointer_lock_key_test(0x41, true), "input: another key is the app's");
    t.check(pointer_lock_key_test(0x1B, true), "input: Esc going down while locked is the lock's");
    t.check(pointer_lock_key_test(0x1B, true), "input: so is its auto-repeat");
    t.check(pointer_lock_key_test(0x1B, false) && !pointer_lock_key_test(0x1B, false), "input: its up too, once");
    // let go: the motion that waited goes, motion is dropped
    pointer_lock_test(0, PointerLock::Off);
    t.check(pop_typ(E_POINTER_LOCK).is_some_and(|e| e.data == 0), "input: let go -> E_POINTER_LOCK data 0");
    t.check(win_raw_motion_take(0) == (0.0, 0.0), "input: the motion that waited went with the lock");
    raw_motion_test(0, 1.0, 1.0);
    t.check(evq_count() == 0, "input: no motion without the lock");
    t.check(!pointer_lock_key_test(0x1B, true) && !pointer_lock_key_test(0x1B, false), "input: Esc without the lock is the app's");
    // another window asks: the first one's change comes first
    pointer_lock_test(0, PointerLock::Locked);
    evq_clear();
    pointer_lock_test(1, PointerLock::Pending);
    let (a, b) = (evq_pop(), evq_pop());
    t.check(a.is_some_and(|e| e.typ == E_POINTER_LOCK && e.win == 0 && e.data == 0) && b.is_some_and(|e| e.typ == E_POINTER_LOCK && e.win == 1),
            "input: another window takes it: window 0 hears first");
    t.check(win_pointer_lock_state(0) == PointerLock::Off && win_pointer_lock_state(1) == PointerLock::Pending, "input: window 1 waits");
    t.check(pointer_lock_key_test(0x1B, true) && pointer_lock_key_test(0x1B, false), "input: Esc while it waits is the lock's (it cancels)");
    pointer_lock_test(1, PointerLock::Off);
    evq_clear();
}

// ------------------------------------------------------------------ Linux: Wayland and X11 synthetic

#[cfg(target_os = "linux")]
fn linux(t: &mut T) {
    let u = |v: i32| v as u32 as u64;
    // ---- Wayland: zwp_locked_pointer_v1, zwp_relative_pointer_v1, Esc on wl_keyboard
    evq_clear();
    wl_test_lock(0);
    t.check(win_pointer_lock_state(0) == PointerLock::Pending, "input wl: lock_pointer sent: Pending");
    t.check(wl_test_event("zwp_relative_pointer_v1", "relative_motion", &[0, 0, u(9 * 256), 0, u(640), 0]), "input wl: relative_motion handled");
    t.check(win_raw_motion_take(0) == (0.0, 0.0), "input wl: no motion before the compositor says locked");
    evq_clear();
    wl_test_event("zwp_locked_pointer_v1", "locked", &[]);
    t.check(win_pointer_lock_state(0) == PointerLock::Locked && pop_typ(E_POINTER_LOCK).is_some_and(|e| e.data == 1), "input wl: locked -> Locked");
    wl_test_event("zwp_relative_pointer_v1", "relative_motion", &[0, 0, u(9 * 256), u(9 * 256), u(640), u(-256)]);
    wl_test_event("zwp_relative_pointer_v1", "relative_motion", &[0, 0, u(1), u(1), u(128), u(64)]);
    t.check(pop_typ(E_RAW_MOTION).is_some(), "input wl: E_RAW_MOTION");
    t.check(win_raw_motion_take(0) == (3.0, -0.75), "input wl: the unaccelerated motion adds up (2.5 + 0.5, -1 + 0.25)");
    evq_clear();
    wl_test_event("wl_keyboard", "key", &[1, 0, 1, 1]); // Esc (evdev 1) down
    wl_test_event("wl_keyboard", "key", &[2, 0, 1, 0]); // and up
    let mut keys = 0;
    let mut off = false;
    while let Some(e) = evq_pop() {
        keys += matches!(e.typ, E_KEY_DOWN | E_KEY_UP) as u32;
        off |= e.typ == E_POINTER_LOCK && e.data == 0;
    }
    t.check(off && keys == 0 && win_pointer_lock_state(0) == PointerLock::Off, "input wl: Esc lets it go and is no key");
    wl_test_event("wl_keyboard", "key", &[3, 0, 1, 1]);
    wl_test_event("wl_keyboard", "key", &[4, 0, 1, 0]);
    t.check(pop_typ(E_KEY_DOWN).is_some(), "input wl: Esc without the lock is a key");
    wl_test_lock(0);
    wl_test_event("zwp_locked_pointer_v1", "locked", &[]);
    evq_clear();
    wl_test_event("zwp_locked_pointer_v1", "unlocked", &[]);
    t.check(win_pointer_lock_state(0) == PointerLock::Off && pop_typ(E_POINTER_LOCK).is_some(), "input wl: unlocked (the compositor let go) -> Off");
    evq_clear();

    // ---- X11: XI_RawMotion, repeats, absolute devices, Esc on the core keyboard
    x11t::xi_setup(131, [500, 501, 502]);
    x11t::lock_setup(0, true, &[(12, [0, 65535], [0, 65535])]);
    evq_clear();
    x11t::event_bytes(&xi_raw(131, 2, 9, 2000, &[0b11], &[20.0, -10.0], &[5.5, -2.25]));
    x11t::event_bytes(&xi_raw(131, 9, 9, 2000, &[0b11], &[20.0, -10.0], &[5.5, -2.25])); // (the same of the slave)
    t.check(pop_typ(E_RAW_MOTION).is_some(), "input x11: XI_RawMotion -> E_RAW_MOTION");
    t.check(win_raw_motion_take(0) == (5.5, -2.25), "input x11: the raw values, a repeat once");
    // an absolute device: the first position starts, the next moves (scaled to the screen; 1 px wide
    // without a server)
    x11t::event_bytes(&xi_raw(131, 2, 12, 2001, &[0b11], &[0.0, 0.0], &[0.0, 0.0]));
    x11t::event_bytes(&xi_raw(131, 2, 12, 2002, &[0b11], &[0.0, 0.0], &[32768.0, 65535.0]));
    let (dx, dy) = win_raw_motion_take(0);
    t.check((dx - 0.5).abs() < 0.001 && (dy - 1.0).abs() < 0.001, &format!("input x11: an absolute device's positions -> motion ({dx}, {dy})"));
    // a core keymap with Esc (keycode 9) as the X server would give it
    x11t::kb_set_keysyms(&[0xFF1B, 0, 0, 0, 0, 0, 0, 0], 9, 1, 8);
    x11t::kb_build(0);
    evq_clear();
    let mut p = [0u8; 32];
    p[0] = 2; // KeyPress
    p[1] = 9; // Esc (evdev 1 + 8)
    p[4] = 1;
    x11t::event(&p);
    p[0] = 3;
    p[4] = 2;
    x11t::event(&p);
    let mut keys = 0;
    let mut off = false;
    while let Some(e) = evq_pop() {
        keys += matches!(e.typ, E_KEY_DOWN | E_KEY_UP) as u32;
        off |= e.typ == E_POINTER_LOCK && e.data == 0;
    }
    t.check(off && keys == 0 && win_pointer_lock_state(0) == PointerLock::Off, "input x11: Esc lets it go and is no key");
    x11t::event_bytes(&xi_raw(131, 2, 9, 2003, &[0b11], &[1.0, 1.0], &[1.0, 1.0]));
    t.check(win_raw_motion_take(0) == (0.0, 0.0) && evq_count() == 0, "input x11: no motion after the release");
    x11t::lock_setup(0, false, &[]);
    x11t::xi_setup(0, [0; 3]);
    evq_clear();
}
