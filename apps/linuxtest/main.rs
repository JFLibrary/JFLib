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
//   linuxtest --test pbench [--frames N] [--gap MS] [--fast]  present cost (present_time) per
//                                                frame: whole, two boxes, a quarter, 40 rects
//   linuxtest --test pcheck                      the presented pixels (read back from the window)
//                                                equal the framebuffer after partial presents,
//                                                frames back to back, hide + show, minimize,
//                                                WM_PAINT (Windows), a new size
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
//   linuxtest --test burst [--keys N]            (tools/keyburst.py) N synthetic keys in one
//                                                read, while waiting for a reply and while
//                                                waiting for events: all arrive, in order
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
// plat-wayland agent -- and t_burst.rs (burst).
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::Args;
use gfx::{Dc, FMT_XRGB, Surface};
#[cfg(target_os = "linux")]
use plat::{CLIP_PNG, CLIP_TEXT, E_FOCUS_IN};
use plat::{
    BTN_LEFT, E_CLOSE, E_DOWN, E_MOVE, E_RESIZE, EDGE_BOTTOM, EDGE_RIGHT, Event, Frame, WCMD_CLOSE, WCMD_MAXIMIZE,
    WCMD_MINIMIZE, WCMD_RESTORE, WST_MAXIMIZED, WST_MINIMIZED,
};

#[cfg(target_os = "linux")]
mod t_burst;
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
        #[cfg(target_os = "linux")]
        Some("activate") => Some(t_wl::test_activate), // no assembly counterpart
        Some("window") => Some(test_window),
        #[cfg(target_os = "linux")]
        Some("clip") => Some(test_clip),
        Some("show") => Some(test_show),
        Some("anim") => Some(test_anim),
        Some("pbench") => Some(test_pbench),
        Some("pcheck") => Some(test_pcheck),
        Some("grab") => Some(test_grab),
        #[cfg(target_os = "linux")]
        Some("x11synth") => Some(t_x11::test_x11synth),
        #[cfg(target_os = "linux")]
        Some("x11err") => Some(t_x11::test_x11err),
        #[cfg(target_os = "linux")]
        Some("burst") => Some(t_burst::test_burst),
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

/// what presenting costs at the window's size (plat::present_time): --frames N (60) frames,
/// --gap MS (16) apart (--fast: back to back), of each kind: the whole frame, two 60 x 60 boxes (an
/// animation), a quarter of the window, 40 small rectangles
fn test_pbench(t: &mut Lt) {
    let n = t.arg_int("--frames", 60).max(1) as u32;
    let gap = if t.args.flag("--fast") { 0 } else { t.arg_int("--gap", 16).max(0) as u64 * 1000 };
    t.redraw();
    settle(400);
    let (w, h) = (t.fb.w, t.fb.h);
    out(&format!("pbench: window {w} x {h} px, dpi {}\n", plat::dpi()));
    let mut dc = Dc::new(w, h);
    for kind in ["full", "boxes", "quarter", "rects40"] {
        let (n0, us0) = plat::present_time();
        let t0 = plat::time_us();
        let mut next = t0;
        let mut rects: Vec<[i32; 4]> = Vec::new();
        for i in 0..n as i32 {
            rects.clear();
            match kind {
                "full" => rects.push([0, 0, w, h]),
                "boxes" => {
                    let x = 20 + (i * 7) % (w - 100).max(1);
                    rects.push([x, 220.min(h - 61).max(0), 60, 60]);
                    rects.push([x + 3, 220.min(h - 61).max(0), 60, 60]);
                }
                "quarter" => rects.push([w / 4, h / 4, w / 2, h / 2]),
                _ => {
                    for k in 0..40 {
                        rects.push([(k * 97 + i * 13) % (w - 40).max(1), (k * 53 + i * 7) % (h - 20).max(1), 40, 20]);
                    }
                }
            }
            let fb = &mut t.fb;
            dc.begin(fb);
            dc.set_color((i as u32).wrapping_mul(0x0001_0307) | 0xFF40_4040);
            for r in &rects {
                dc.fill(fb, r[0], r[1], r[2], r[3]);
            }
            dc.end();
            if kind == "full" {
                t.present(None);
            } else {
                t.present(Some(&rects));
            }
            next += gap;
            pump_until(next);
        }
        settle(200); // the presents waiting for the compositor
        let (n1, us1) = plat::present_time();
        let wall = plat::time_us() - t0;
        out(&format!("pbench {kind}: {w} x {h} px, {} frames, {} us per frame (wall {} ms)\n", n1 - n0,
                     (us1 - us0) / (n1 - n0).max(1), wall / 1000));
    }
    // Windows: half the window moved by 40 rows inside the window (BitBlt) against presenting
    // the same rows again
    #[cfg(windows)]
    {
        let (y, rows) = (h / 4, h / 2);
        let mut blit = 0i64;
        for i in 0..n as i32 {
            blit += plat::win_test(plat::WT_SCROLL, 0, [y, rows, if i % 2 == 0 { -40 } else { 40 }, 0]).max(0);
            pump_until(plat::time_us() + gap);
        }
        let (n0, us0) = plat::present_time();
        for i in 0..n {
            boxes(t, &[[0, y, w, rows]], 0x0030_3030 + i);
            pump_until(plat::time_us() + gap);
        }
        let (n1, us1) = plat::present_time();
        out(&format!("pbench scroll: {w} x {rows} px moved inside the window {} us, presented again {} us per frame\n",
                     blit / n as i64, (us1 - us0) / (n1 - n0).max(1)));
    }
}

/// what the main window shows as (w, h, rows of w * 4 bytes B, G, R, x): Windows the window's
/// DC, X11 GetImage of the window, Wayland the last committed buffer (None while damage waits)
fn shown() -> Option<(i32, i32, Vec<u8>)> {
    #[cfg(windows)]
    {
        plat::win_test_image(0)
    }
    #[cfg(target_os = "linux")]
    {
        if plat::disp_backend() == 1 {
            return plat::wl_test_presented(0);
        }
        let (w, h) = plat::window_size();
        plat::x11t::image_of(0, [0, 0, w, h]).ok().map(|s| (s.w, s.h, s.px))
    }
}

/// pump until the window shows the framebuffer (at most 2 s; 6 s after a new size: XWayland
/// refuses GetImage while the compositor resizes the window) -> check
fn check_shown(t: &mut Lt, name: &str) {
    let end = plat::time_us() + if name.contains("new size") { 6_000_000 } else { 2_000_000 };
    let mut last = String::from("nothing shown");
    loop {
        settle(30);
        if let Some((w, h, px)) = shown() {
            let fb = &t.fb;
            if (w, h) != (fb.w, fb.h) {
                last = format!("shows {w} x {h}, the frame is {} x {}", fb.w, fb.h);
            } else {
                let (mut n, mut at) = (0usize, (0, 0));
                for y in 0..h as usize {
                    for x in 0..w as usize {
                        let (a, b) = ((y * w as usize + x) * 4, y * fb.stride as usize + x * 4);
                        if px[a..a + 3] != fb.px[b..b + 3] {
                            if n == 0 {
                                at = (x, y);
                            }
                            n += 1;
                        }
                    }
                }
                if n == 0 {
                    t.check(true, name);
                    return;
                }
                last = format!("{n} pixels differ, the first at {}, {}", at.0, at.1);
            }
        }
        if plat::time_us() >= end {
            t.check(false, name);
            out(&format!("     ({last})\n"));
            return;
        }
    }
}

/// draw boxes of colour c into the framebuffer and present only them (x, y, w, h)
fn boxes(t: &mut Lt, rs: &[[i32; 4]], c: u32) {
    let fb = &mut t.fb;
    let mut dc = Dc::new(fb.w, fb.h);
    dc.begin(fb);
    for (k, r) in rs.iter().enumerate() {
        dc.set_color(c.wrapping_add(k as u32 * 0x0001_0203) | 0xFF00_0000);
        dc.fill(fb, r[0], r[1], r[2], r[3]);
    }
    dc.end();
    t.present(Some(rs));
}

/// the presented pixels against the framebuffer: whole frames, moving boxes, frames back to
/// back (the X11 segments / Wayland buffers busy: the staging image), many rectangles (more
/// than the pending list and a region hold), a whole frame again at the same size, hide +
/// show, minimized, a repaint by the system (Windows WM_PAINT), a new size
fn test_pcheck(t: &mut Lt) {
    if t.headless {
        t.check(false, "pcheck needs a window (run without --shot)");
        return;
    }
    t.redraw();
    check_shown(t, "pcheck: the whole frame");
    for i in 0..20 {
        let x = 20 + i * 9;
        boxes(t, &[[x, 220, 60, 60], [x + 200, 120, 30, 90]], 0x0020_4060 + i as u32 * 0x0003_0507);
        settle(16);
    }
    check_shown(t, "pcheck: moving boxes, 16 ms apart");
    for i in 0..12 {
        boxes(t, &[[(i * 37) % 500, (i * 23) % 300, 120, 70], [5, 5, 30 + i, 20]], 0x0050_1020 + i as u32 * 0x0007_0301);
    }
    check_shown(t, "pcheck: frames back to back");
    let (w, h) = (t.fb.w, t.fb.h);
    for i in 0..8 {
        let rs: Vec<[i32; 4]> =
            (0..40).map(|k| [(k * 97 + i * 13) % (w - 40).max(1), (k * 53 + i * 7) % (h - 20).max(1), 40, 20]).collect();
        boxes(t, &rs, 0x0010_7030 + i as u32 * 0x0001_0101);
        settle(5);
    }
    check_shown(t, "pcheck: 40 rectangles a frame");
    let rs: Vec<[i32; 4]> = (0..300).map(|k| [(k * 31) % (w - 8).max(1), (k * 17) % (h - 8).max(1), 7, 7]).collect();
    boxes(t, &rs, 0x0090_2010);
    boxes(t, &rs[..150], 0x0010_9020);
    check_shown(t, "pcheck: 300 rectangles in a frame");
    {
        // a whole frame again at the same size (rects None: a theme switch, a window shown again)
        let fb = &mut t.fb;
        let mut dc = Dc::new(fb.w, fb.h);
        dc.begin(fb);
        dc.clear(fb, 0xFF40_3020);
        dc.set_color(0xFF20_A040);
        dc.fill(fb, 30, 30, fb.w / 2, fb.h / 3);
        dc.end();
        t.present(None);
    }
    check_shown(t, "pcheck: a whole frame again (same size)");
    plat::win_show(0, plat::PWS_HIDE);
    settle(400);
    boxes(t, &[[50, 50, 100, 100]], 0x0030_3090); // while hidden
    plat::win_show(0, plat::PWS_SHOW);
    settle(800);
    check_shown(t, "pcheck: hidden, presented, shown again");
    #[cfg(windows)]
    {
        boxes(t, &[[0, 0, 80, 40]], 0x0060_6000);
        plat::win_test(plat::WT_REDRAW, 0, [0; 4]); // WM_PAINT from the kept copy
        check_shown(t, "pcheck: WM_PAINT repaints the presented pixels");
    }
    plat::window_cmd(plat::WCMD_MINIMIZE);
    settle(600);
    boxes(t, &[[100, 60, 200, 100]], 0x0070_2050); // while minimized
    settle(1300); // (Wayland: the frame callback does not come, the safety deadline does)
    if plat::window_state() & plat::WST_MINIMIZED != 0 || cfg!(windows) {
        plat::window_cmd(plat::WCMD_RESTORE);
        settle(800);
    }
    check_shown(t, "pcheck: presented while minimized");
    let (w0, h0) = plat::window_size();
    plat::win_set_rect(0, plat::PW_KEEP, plat::PW_KEEP, w0 + 37, h0 + 23);
    settle(500);
    let (w1, h1) = plat::window_size();
    t.fb.resize(w1, h1);
    t.redraw();
    boxes(t, &[[w1 - 50, h1 - 30, 50, 30]], 0x0040_8080);
    check_shown(t, &format!("pcheck: a new size ({w0} x {h0} -> {w1} x {h1})"));
    plat::win_set_rect(0, plat::PW_KEEP, plat::PW_KEEP, w0, h0);
    settle(300);
}

/// pump the OS (events dropped) until the deadline
fn pump_until(deadline: u64) {
    loop {
        plat::wait(deadline);
        while plat::evq_pop().is_some() {}
        if plat::time_us() >= deadline {
            return;
        }
    }
}

fn settle(ms: u64) {
    pump_until(plat::time_us() + ms * 1000);
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
