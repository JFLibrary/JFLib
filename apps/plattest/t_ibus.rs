// t_ibus -- input methods on X11: the IBus portal client (lib/plat/linux_ibus.rs)
//
//   plattest --test ibus-bus    (Linux; tools/platshared/ibus_bus.py starts it on a fake session
//                               bus playing org.freedesktop.portal.IBus and an input method)
//                               focus, caret, keys the input method takes or not, preedit with
//                               its cursor, commit, forwarded keys, a key without an answer
//                               (300 ms, the next key still works), focus out, jfgui_IME=0
//   plattest --test ibus-none   (Linux; ibus_bus.py runs it with a bus without the portal and
//                               without a reachable bus) no input method: keys are not taken and
//                               not delayed
//   plattest --test ibus-x11    (Linux, X11; ibus_bus.py sends synthetic X events to its unmapped
//                               window) the X11 glue: FocusIn + ime_pos -> IBus focus, KeyPress
//                               'a' taken (preedit, no E_TEXT), 's' not taken (E_TEXT), the
//                               window's FocusOut ends the input method's focus

use crate::T;

#[cfg(windows)]
pub fn t_ibus_bus(_: &mut T) {
    plat::write_stderr(b"plattest: --test ibus-bus is Linux only\n");
    plat::exit(2);
}

#[cfg(windows)]
pub fn t_ibus_none(_: &mut T) {
    plat::write_stderr(b"plattest: --test ibus-none is Linux only\n");
    plat::exit(2);
}

#[cfg(windows)]
pub fn t_ibus_x11(_: &mut T) {
    plat::write_stderr(b"plattest: --test ibus-x11 is Linux only\n");
    plat::exit(2);
}

// wait up to 3 s until f() holds
#[cfg(target_os = "linux")]
fn wait(f: impl Fn() -> bool) -> bool {
    for _ in 0..300 {
        if f() {
            return true;
        }
        plat::sleep_ms(10);
    }
    f()
}

#[cfg(target_os = "linux")]
pub fn t_ibus_bus(t: &mut T) {
    use alloc::string::String;
    use alloc::vec::Vec;
    use plat::{IBUS_RELEASE, ibus_available, ibus_focus, ibus_key, ibus_ready, ime_preedit};

    plat::evq_clear();
    t.check(ibus_available(), "ibus-bus: a session bus is announced");
    t.check(!ibus_ready(), "ibus-bus: no input context before the first focus");
    ibus_focus(0, true, 100, 200, 18);
    ibus_focus(0, true, 104, 200, 18); // the caret moved: one FocusIn, two locations
    t.check(wait(ibus_ready), "ibus-bus: the input context was made");

    // 'a' starts a composition "かな" with the cursor after the first character
    t.check(ibus_key(0x61, 30, 0), "ibus-bus: the input method takes 'a'");
    let p = ime_preedit(0);
    t.check(p.text == "かな" && p.cursor == 3, "ibus-bus: preedit with its cursor in bytes");
    t.check(!ibus_key(0x61, 30, IBUS_RELEASE), "ibus-bus: the release is not taken");
    t.check(ibus_key(0xFF0D, 28, 0), "ibus-bus: Return commits");
    t.check(ime_preedit(0).text.is_empty(), "ibus-bus: the commit ends the preedit");
    t.check(!ibus_key(0x7A, 44, 0), "ibus-bus: 'z' is not taken");
    t.check(ibus_key(0xFF08, 14, 0), "ibus-bus: BackSpace is taken and forwarded");
    t.check(ibus_key(0x62, 48, 0) && ime_preedit(0).text == "b", "ibus-bus: a new preedit");
    t.check(ibus_key(0xFF1B, 1, 0) && ime_preedit(0).text.is_empty(), "ibus-bus: Escape hides the preedit");

    // no answer: false after 300 ms, then the next key works again
    let t0 = plat::time_us();
    let took = ibus_key(0x71, 16, 0);
    let ms = (plat::time_us() - t0) / 1000;
    t.check(!took && (250..1500).contains(&ms), "ibus-bus: a key without an answer gives up after 300 ms");
    t.check(ibus_key(0x62, 48, 0) && ime_preedit(0).text == "b", "ibus-bus: the next key is answered");
    ibus_focus(0, false, 0, 0, 0);
    t.check(wait(|| ime_preedit(0).text.is_empty()), "ibus-bus: focus out ends the preedit");

    // what reached the UI thread
    let mut commits: Vec<String> = Vec::new();
    let (mut left, mut text_c, mut preedits) = (false, false, 0);
    while let Some(e) = plat::evq_pop() {
        match e.typ {
            plat::E_IME_COMMIT => commits.extend(plat::ime_commit_take(e.data)),
            plat::E_KEY_DOWN => left |= e.data == 37,
            plat::E_TEXT => text_c |= e.data == 0x63,
            plat::E_PREEDIT => preedits += 1,
            _ => {}
        }
    }
    t.check(commits.len() == 1 && commits[0] == "かな", "ibus-bus: one E_IME_COMMIT with the text");
    t.check(left && text_c, "ibus-bus: forwarded keys arrive as E_KEY_DOWN (Left) and E_TEXT ('c')");
    t.check(preedits >= 5, "ibus-bus: E_PREEDIT on every change");

    plat::setenv("jfgui_IME", Some("0"));
    t.check(!ibus_available() && !ibus_key(0x61, 30, 0), "ibus-bus: jfgui_IME=0 turns it off");
    plat::setenv("jfgui_IME", None);
    plat::sleep_ms(300); // FocusOut reaches the bus
    crate::out("ibus-bus: done\n");
}

#[cfg(target_os = "linux")]
pub fn t_ibus_none(t: &mut T) {
    use plat::{ibus_available, ibus_focus, ibus_key, ibus_ready};

    ibus_focus(0, true, 10, 10, 16);
    t.check(wait(|| !ibus_available()), "ibus-none: no input method is noticed");
    t.check(!ibus_ready(), "ibus-none: no input context");
    let t0 = plat::time_us();
    let took = ibus_key(0x61, 30, 0);
    t.check(!took && plat::time_us() - t0 < 50_000, "ibus-none: keys are not taken and not delayed");
    ibus_focus(0, false, 0, 0, 0);
    t.check(plat::ime_preedit(0).text.is_empty(), "ibus-none: no preedit");
}

// pump the OS until f(event) holds (at most 10 s)
#[cfg(target_os = "linux")]
fn pump_until(mut f: impl FnMut(&plat::Event) -> bool) -> bool {
    let dl = plat::time_us() + 10_000_000;
    loop {
        plat::wait(dl.min(plat::time_us() + 50_000));
        while let Some(e) = plat::evq_pop() {
            if f(&e) {
                return true;
            }
        }
        if plat::time_us() >= dl {
            return false;
        }
    }
}

#[cfg(target_os = "linux")]
pub fn t_ibus_x11(t: &mut T) {
    use plat::{E_FOCUS_IN, E_FOCUS_OUT, E_TEXT, ibus_ready, ime_pos, ime_preedit};

    let w = plat::win_create("plattest ibus", 160, 120, 0, plat::PW_NOOWNER, plat::PW_DEFAULT, plat::PW_DEFAULT);
    t.check(w == 0 && plat::disp_backend() == plat::BK_X11, "ibus-x11: an X11 window (never shown)");
    crate::out("  ibus-x11: window\n");
    t.check(pump_until(|e| e.typ == E_FOCUS_IN), "ibus-x11: the window has the keyboard (FocusIn)");
    ime_pos(5, 6, 16);
    ime_pos(5, 6, 16); // every frame: nothing new for the bus
    t.check(wait(ibus_ready), "ibus-x11: a text field has the focus: the input context is there");
    crate::out("  ibus-x11: ime on\n");
    let mut text: alloc::vec::Vec<u32> = alloc::vec::Vec::new();
    pump_until(|e| {
        if e.typ == E_TEXT {
            text.push(e.data);
        }
        text.contains(&0x73) && ime_preedit(0).text == "かな"
    });
    t.check(ime_preedit(0).text == "かな" && !text.contains(&0x61), "ibus-x11: 'a' went to the input method (preedit, no E_TEXT)");
    t.check(text.contains(&0x73), "ibus-x11: 's' was not taken (E_TEXT)");
    crate::out("  ibus-x11: keys seen\n");
    t.check(pump_until(|e| e.typ == E_FOCUS_OUT), "ibus-x11: the window lost the keyboard (FocusOut)");
    t.check(wait(|| ime_preedit(0).text.is_empty()), "ibus-x11: the window's FocusOut ends the preedit");
    ime_pos(5, 6, 16); // the next frame: no window has the keyboard, the input method stays off
    plat::win_destroy(0);
    plat::sleep_ms(300); // FocusOut reaches the bus
    crate::out("ibus-x11: done\n");
}
