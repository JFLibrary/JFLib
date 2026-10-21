// linuxtest -- platform-layer self tests (written with the Linux layer)
//                                                                [apps/linuxtest/src/main.s]
//
//   linuxtest --shot x.png --test files          file read/write/exists/dir_list, time
//   linuxtest --shot x.png --test crash          null dereference -> crash handler
//   linuxtest --shot x.png --test xkb --keymap F  XKB parser + key translation (Linux)
//   linuxtest --test window                      maximize / restore / title / close
//   linuxtest --test show [--hold MS]            test pattern (for window captures)
//   linuxtest --test anim [--hold MS] [--save F] [--minimize] [--after MS]  continuous
//                                                partial presents, then keep the window
//   linuxtest --shot x.png --test cursors [--out F]  fallback cursor images as a PNG
//   linuxtest --test protoerr                    provoke wl_display.error (expect exit 1)
//   linuxtest --shot x.png --test synth          synthetic pointer/keyboard events -> E_*
//   linuxtest --test clip [--hold MS] [--big] [--text S]  clipboard read (e.g. from
//                                                Windows), set, read back; --hold keeps
//                                                offering it; --big N offers "0123456789abcdef"
//                                                x N + "END" (INCR on X11), --text S offers S
//   linuxtest --test grab [--hold MS]            left press: left half begin_move, right
//                                                half begin_resize(right|bottom)
//   linuxtest --shot x.png --test x11synth       X11 keysyms, German core keymap and
//                                                synthetic X11 events -> E_* (no server)
//   linuxtest --test x11err                      bad X request: error recorded, no exit;
//                                                every cursor shape without an X error
//
// Every window test runs on the backend linux_disp picks (jfgui_BACKEND=x11|wayland).
// Every test prints PASS/FAIL lines and ends the process with exit(failures).
// Builds for both targets; dir_list, clipboard, xkb, cursors, protoerr, synth and the X11
// tests are Linux-only.
//
// The assembly app runs inside the UI runtime (APP_DESC, the tests in its init callback);
// this one calls plat directly (R4: the runtime has not switched to the window-indexed
// API yet): the same window (title, 640 x 400 DIP, minimum 320 x 240, resizable) is made
// before the test unless --shot / --script make the run headless, and the framebuffer is
// its own surface.  Modules: t_files.rs (files, crash), t_x11.rs (x11synth, x11err),
// t_wl.rs (cursors, protoerr, synth) and t_xkb.rs (xkb) -- the last two belong to the
// plat-wayland agent.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
#[cfg(target_os = "linux")]
use alloc::vec::Vec;

use base::Args;
use gfx::{Dc, FMT_XRGB, Surface};
#[cfg(target_os = "linux")]
use plat::{CLIP_PNG, CLIP_TEXT, E_FOCUS_IN};
use plat::{
    BTN_LEFT, E_CLOSE, E_DOWN, E_MOVE, E_RESIZE, EDGE_BOTTOM, EDGE_RIGHT, Event, Frame, WCMD_CLOSE, WCMD_MAXIMIZE,
    WCMD_MINIMIZE, WCMD_RESTORE, WST_MAXIMIZED, WST_MINIMIZED,
};

mod t_files;
#[cfg(target_os = "linux")]
mod t_wl;
#[cfg(target_os = "linux")]
mod t_x11;
#[cfg(target_os = "linux")]
mod t_xkb;

ui::main!(linuxtest());

const TITLE: &str = "jfgui Linux test";
const AF_RESIZABLE: u32 = 2;
const INIT_W: i32 = 640;
const INIT_H: i32 = 400;
const MIN_W: i32 = 320;
const MIN_H: i32 = 240;
const RGB_BOXES: [u32; 3] = [0xFFFF0000, 0xFF00FF00, 0xFF0000FF];
#[cfg(target_os = "linux")]
const CLIPTEXT: &str = "jfgui Wayland clipboard: Grüße € ✓";
#[cfg(target_os = "linux")]
const PATTERN: &[u8; 16] = b"0123456789abcdef";

/// the test state: failures, the framebuffer, the command line
pub struct Lt {
    pub fails: u32,
    pub headless: bool,
    pub fb: Surface,
    pub args: Args,
}

pub fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
}

/// parse_int of core/str.s: blanks, a sign, decimal digits (wrapping), stops at the first
/// other character
pub fn parse_int(s: &str) -> i32 {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() && b[i] == b' ' {
        i += 1;
    }
    let mut neg = false;
    if i < b.len() && b[i] == b'-' {
        neg = true;
        i += 1;
    } else if i < b.len() && b[i] == b'+' {
        i += 1;
    }
    let mut v: i32 = 0;
    while i < b.len() && b[i].is_ascii_digit() {
        v = v.wrapping_mul(10).wrapping_add((b[i] - b'0') as i32);
        i += 1;
    }
    if neg { v.wrapping_neg() } else { v }
}

impl Lt {
    /// print PASS / FAIL, count failures
    pub fn check(&mut self, ok: bool, name: &str) {
        if ok {
            out(&format!("PASS {name}\n"));
        } else {
            self.fails += 1;
            out(&format!("FAIL {name}\n"));
        }
    }

    /// arg_int: the value after `name` (parse_int), else def
    pub fn arg_int(&self, name: &str, def: i32) -> i32 {
        self.args.value(name).map_or(def, parse_int)
    }

    /// fill the framebuffer with a test pattern and present it
    pub fn redraw(&mut self) {
        let fb = &mut self.fb;
        let mut dc = Dc::new(fb.w, fb.h);
        dc.begin(fb);
        dc.clear(fb, 0xFF203040);
        dc.set_color(0xFF0078D4);
        dc.fill(fb, 20, 20, fb.w - 40, 60);
        // red, green, blue boxes (channel order check)
        for (i, &c) in RGB_BOXES.iter().enumerate() {
            dc.set_color(c);
            dc.fill(fb, i as i32 * 120 + 20, 100, 100, 100);
        }
        dc.set_color(0xFFFFFFFF);
        dc.frame(fb, 0, 0, fb.w, fb.h);
        dc.end();
        self.present(None);
    }

    pub fn present(&self, rects: Option<&[[i32; 4]]>) {
        let f = Frame { px: &self.fb.px, w: self.fb.w, h: self.fb.h, stride: self.fb.stride };
        plat::present(&f, rects);
    }

    /// pump(ms, want_type) -> true if an event of want_type arrived (returns right after
    /// it); E_RESIZE resizes the framebuffer and redraws
    pub fn pump(&mut self, ms: u32, want: u8) -> bool {
        let deadline = plat::time_us() + ms as u64 * 1000;
        loop {
            plat::wait(deadline);
            while let Some(e) = plat::evq_pop() {
                if e.typ != E_MOVE {
                    log_event(&e);
                }
                if e.typ == E_RESIZE {
                    self.fb.resize(e.x, e.y);
                    self.redraw();
                }
                if e.typ == want {
                    return true;
                }
            }
            if plat::time_us() >= deadline {
                return false;
            }
        }
    }

    /// pop one event and compare type | btn << 8, x, y (x = 0x7FFFFFFF: position not
    /// checked) and data
    pub fn expect(&mut self, tb: u32, x: i32, y: i32, data: u32, name: &str) {
        let e = plat::evq_pop();
        let ok = match e {
            Some(e) => {
                (e.typ as u32 | (e.btn as u32) << 8) == tb && (x == 0x7FFFFFFF || (e.x == x && e.y == y)) && e.data == data
            }
            None => false,
        };
        self.check(ok, name);
        if !ok {
            let e = e.unwrap_or_default();
            out(&format!(
                "     got type {} btn {} mods {:x} x {} y {} data {:x}\n",
                e.typ,
                e.btn,
                e.mods as u32 | (e.win as u32) << 8,
                e.x,
                e.y,
                e.data
            ));
        }
    }
}

/// "  event ..." line of pump / grab
pub fn log_event(e: &Event) {
    out(&format!("  event {} btn {} mods {:x} x {} y {} data {:x}\n", e.typ, e.btn, e.mods, e.x, e.y, e.data));
}

/// FNV-1a 64 (hash_fnv64, seed 0)
pub fn fnv64(d: &[u8]) -> u64 {
    let mut h: u64 = 0xCBF29CE484222325;
    for &b in d {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001B3);
    }
    h
}

fn dp(v: i32, dpi: i32) -> i32 {
    let p = v.wrapping_mul(dpi);
    if p >= 0 { (p + 48) / 96 } else { (p - 48) / 96 }
}

fn linuxtest() -> i32 {
    let args = Args(plat::args());
    let (mut w, mut h) = (INIT_W, INIT_H);
    if let Some(v) = args.value("--size") {
        if let Some((a, b)) = v.split_once(['x', 'X']) {
            let (a, b) = (parse_int(a), parse_int(b));
            if a > 0 && b > 0 {
                w = a;
                h = b;
            }
        }
    }
    let shot = args.value("--shot").map(String::from);
    let headless = shot.is_some() || args.value("--script").is_some();
    let fb = if headless {
        let dpi = (parse_int(args.value("--scale").unwrap_or("100")).clamp(50, 400) * 96 + 50) / 100;
        Surface::new(dp(w, dpi), dp(h, dpi), FMT_XRGB)
    } else {
        plat::set_min_size(MIN_W, MIN_H);
        if !plat::window_create(TITLE, w, h, AF_RESIZABLE) {
            plat::write_stderr(b"error: no window could be created (use --shot FILE.png for a headless run)\n");
            return 2;
        }
        let (pw, ph) = plat::window_size();
        Surface::new(pw, ph, FMT_XRGB)
    };
    let mut lt = Lt { fails: 0, headless, fb, args };
    let test: Option<fn(&mut Lt)> = match lt.args.value("--test") {
        Some("files") => Some(t_files::test_files),
        Some("crash") => Some(t_files::test_crash),
        #[cfg(target_os = "linux")]
        Some("xkb") => Some(t_xkb::test_xkb),
        #[cfg(target_os = "linux")]
        Some("cursors") => Some(t_wl::test_cursors),
        #[cfg(target_os = "linux")]
        Some("protoerr") => Some(t_wl::test_protoerr),
        #[cfg(target_os = "linux")]
        Some("synth") => Some(t_wl::test_synth),
        #[cfg(target_os = "linux")]
        Some("wlwin") => Some(t_wl::test_wlwin), // plat-wayland: no assembly counterpart
        #[cfg(target_os = "linux")]
        Some("wlclip") => Some(t_wl::test_wlclip), // plat-wayland: no assembly counterpart
        Some("window") => Some(test_window),
        #[cfg(target_os = "linux")]
        Some("clip") => Some(test_clip),
        Some("show") => Some(test_show),
        Some("anim") => Some(test_anim),
        Some("grab") => Some(test_grab),
        #[cfg(target_os = "linux")]
        Some("x11synth") => Some(t_x11::test_x11synth),
        #[cfg(target_os = "linux")]
        Some("x11err") => Some(t_x11::test_x11err),
        _ => None,
    };
    if let Some(t) = test {
        t(&mut lt);
        out(&format!("linuxtest: {} failure(s)\n", lt.fails));
        plat::exit(lt.fails as i32);
    }
    // no test: the plain app (lt_frame clears to a dark grey)
    let mut dc = Dc::new(lt.fb.w, lt.fb.h);
    dc.begin(&lt.fb);
    dc.clear(&mut lt.fb, 0xFF202020);
    dc.end();
    if let Some(s) = shot {
        let png = img_png::encode(&lt.fb.px, lt.fb.w as usize, lt.fb.h as usize, lt.fb.stride as usize, false);
        if !plat::write_file(&s, &png) {
            plat::write_stderr(format!("error: could not write {s}\n").as_bytes());
            return 1;
        }
        out(&format!("wrote {s}\n"));
        return 0;
    }
    if headless {
        return 0;
    }
    lt.present(None);
    let auto = lt.arg_int("--auto-close", 0);
    let ms = if auto > 0 { auto as u32 } else { u32::MAX / 1000 };
    if lt.args.flag("--log-events") {
        log_events(&mut lt, ms); // plat-wayland: pencheck.py / the protocol-proxy checks
        return 0;
    }
    lt.pump(ms, E_CLOSE);
    0
}

/// plat-wayland: the plain app's loop with --log-events as the UI runtime prints it (every
/// event "event type=.. btn=.. mods=.. x=.. y=.. data=.. win=..", E_RESIZE redraws the plain
/// frame, E_CLOSE or --auto-close ends it)  [ui_main.s log_event]
fn log_events(t: &mut Lt, ms: u32) {
    let deadline = plat::time_us() + ms as u64 * 1000;
    loop {
        plat::wait(deadline);
        while let Some(e) = plat::evq_pop() {
            out(&format!("event type={} btn={} mods={:x} x={} y={} data={:x} win={}\n", e.typ, e.btn, e.mods, e.x, e.y,
                         e.data, e.win));
            if e.typ == E_CLOSE {
                return;
            }
            if e.typ == E_RESIZE {
                t.fb.resize(e.x, e.y);
                let mut dc = Dc::new(t.fb.w, t.fb.h);
                dc.begin(&t.fb);
                dc.clear(&mut t.fb, 0xFF202020);
                dc.end();
                t.present(None);
            }
        }
        if plat::time_us() >= deadline {
            return;
        }
    }
}

// ---------------------------------------------------------------- window

/// window state commands and events
fn test_window(t: &mut Lt) {
    if t.headless {
        t.check(false, "window test needs a window (run without --shot)");
        return;
    }
    let (w0, h0) = plat::window_size();
    out(&format!("window {} x {} px, dpi {}\n", w0, h0, plat::dpi()));
    t.redraw();
    t.pump(1000, 0);
    out(&format!("state after 1 s: {:x}\n", plat::window_state()));
    plat::window_cmd(WCMD_MAXIMIZE);
    let ok = t.pump(3000, E_RESIZE);
    t.check(ok, "maximize -> E_RESIZE");
    t.pump(500, 0);
    let st = plat::window_state();
    let (w, h) = plat::window_size();
    out(&format!("maximized: {} x {} px, state {:x}\n", w, h, st));
    t.check(st & WST_MAXIMIZED != 0, "plat_window_state has WST_MAXIMIZED");
    plat::window_cmd(WCMD_RESTORE);
    let ok = t.pump(3000, E_RESIZE);
    t.check(ok, "restore -> E_RESIZE");
    t.pump(500, 0);
    let (w, h) = plat::window_size();
    out(&format!("restored: {} x {} px\n", w, h));
    t.check(w == w0 && h == h0, "restore gives the original size back");
    t.check(plat::window_state() & WST_MAXIMIZED == 0, "state no longer maximized");
    plat::begin_move(); // no button held: must be harmless
    plat::begin_resize(EDGE_RIGHT | EDGE_BOTTOM);
    plat::set_title("linuxtest: title changed");
    plat::set_cursor(plat::CUR_HAND);
    t.pump(300, 0);
    plat::window_cmd(WCMD_MINIMIZE);
    t.pump(500, 0);
    t.redraw(); // presenting while minimized must not stall
    t.redraw();
    t.pump(1500, 0);
    let st = plat::window_state();
    out(&format!("state after minimize: {:x}\n", st));
    t.check(st & WST_MINIMIZED != 0, "minimize -> WST_MINIMIZED");
    plat::window_cmd(WCMD_CLOSE);
    let ok = t.pump(1000, E_CLOSE);
    t.check(ok, "WCMD_CLOSE -> E_CLOSE");
}

/// draw the test pattern and keep the window for --hold MS (default 3000)
fn test_show(t: &mut Lt) {
    let (w, h) = plat::window_size();
    out(&format!("window {} x {} px, dpi {}\n", w, h, plat::dpi()));
    t.redraw();
    let hold = t.arg_int("--hold", 3000);
    t.pump(hold as u32, E_CLOSE);
}

/// move a box as fast as possible for --hold MS (default 2000), presenting only the damaged
/// rectangles; then save the framebuffer to --save FILE (default build/anim_fb.png) and keep
/// the window --after MS (2 s) for a capture
fn test_anim(t: &mut Lt) {
    t.redraw();
    if t.args.flag("--minimize") {
        t.pump(300, 0);
        plat::window_cmd(WCMD_MINIMIZE);
    }
    let end = plat::time_us() + (t.arg_int("--hold", 2000) as i64 * 1000) as u64;
    let mut frames: u32 = 0;
    let mut bx: i32 = 20;
    let mut dc = Dc::new(t.fb.w, t.fb.h);
    while plat::time_us() < end {
        let fb = &mut t.fb;
        dc.begin(fb);
        dc.set_color(0xFF203040); // erase the old box
        dc.fill(fb, bx, 220, 60, 60);
        let old = [bx, 220, 60, 60];
        bx += 3;
        if bx >= fb.w - 80 {
            bx = 20;
        }
        dc.set_color(frames.wrapping_mul(0x010307) | 0xFF808080); // colour changes every frame
        dc.fill(fb, bx, 220, 60, 60);
        dc.end();
        t.present(Some(&[old, [bx, 220, 60, 60]]));
        frames = frames.wrapping_add(1);
        plat::wait(0); // poll: never block
        while plat::evq_pop().is_some() {}
    }
    out(&format!("anim: {frames} frames drawn\n"));
    let save = t.args.value("--save").unwrap_or("build/anim_fb.png");
    let png = img_png::encode(&t.fb.px, t.fb.w as usize, t.fb.h as usize, t.fb.stride as usize, false);
    plat::write_file(save, &png);
    let after = t.arg_int("--after", 2000); // keep the window (for captures)
    t.pump(after as u32, E_CLOSE);
}

// ---------------------------------------------------------------- clipboard

/// read what is there, set our text, read it back, optionally hold it
#[cfg(target_os = "linux")]
fn test_clip(t: &mut Lt) {
    if t.headless {
        t.check(false, "clipboard test needs a window");
        return;
    }
    t.redraw();
    let ok = t.pump(3000, E_FOCUS_IN);
    t.check(ok, "keyboard focus (E_FOCUS_IN)");
    t.pump(300, 0);
    match plat::clip_get(CLIP_TEXT) {
        Some(d) => {
            let n = d.iter().position(|&c| c == 0).unwrap_or(d.len()).min(200);
            let mut line: Vec<u8> = b"clipboard before: [".to_vec();
            line.extend_from_slice(&d[..n]);
            line.extend_from_slice(format!("] ({} bytes)\n", d.len()).as_bytes());
            plat::write_stdout(&line);
            out(&format!("clipboard hash {:x}\n", fnv64(&d)));
        }
        None => out("clipboard before: [(none)] (0 bytes)\n"),
    }
    // the text to offer: CLIPTEXT, --text S, or with --big N "0123456789abcdef" x N + "END"
    let mut text: Vec<u8> = t.args.value("--text").unwrap_or(CLIPTEXT).as_bytes().to_vec();
    let big = t.arg_int("--big", 0);
    if big > 0 {
        let n = (big as u32).wrapping_shl(4) as usize;
        let mut v = Vec::with_capacity(n + 4);
        while v.len() < n {
            v.extend_from_slice(PATTERN);
        }
        v.extend_from_slice(b"END");
        text = v;
    }
    let ok = plat::clip_set(CLIP_TEXT, &text);
    t.check(ok, "plat_clip_set text");
    t.pump(300, 0);
    let back = plat::clip_get(CLIP_TEXT);
    t.check(back.as_deref() == Some(&text[..]), "plat_clip_get returns the text we set");
    t.check(plat::clip_get(CLIP_PNG).is_none(), "plat_clip_get(CLIP_PNG) = 0 while we own text");
    let hold = t.arg_int("--hold", 0);
    if hold != 0 {
        out(&format!("holding the clipboard for {} ms\n", hold as u32));
        t.pump(hold as u32, E_CLOSE);
    }
}

// ---------------------------------------------------------------- window manager grabs

/// a left press in the left half starts begin_move, in the right half begin_resize(right |
/// bottom); events are logged for --hold MS (8000)
fn test_grab(t: &mut Lt) {
    t.redraw();
    let deadline = plat::time_us() + (t.arg_int("--hold", 8000) as i64 * 1000) as u64;
    loop {
        plat::wait(deadline);
        while let Some(e) = plat::evq_pop() {
            if e.typ == E_MOVE {
                continue;
            }
            log_event(&e);
            if e.typ == E_CLOSE {
                return;
            }
            if e.typ == E_RESIZE {
                t.fb.resize(e.x, e.y);
                t.redraw();
                continue;
            }
            if e.typ == E_DOWN && e.btn == BTN_LEFT {
                if e.x < (t.fb.w as u32 >> 1) as i32 {
                    out("begin_move\n");
                    plat::begin_move();
                } else {
                    out("begin_resize\n");
                    plat::begin_resize(EDGE_RIGHT | EDGE_BOTTOM);
                }
            }
        }
        if plat::time_us() >= deadline {
            return;
        }
    }
}
