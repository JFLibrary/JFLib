// t_wl -- linuxtest: the Wayland tests (cursors, protoerr, synth)
//                          [apps/linuxtest/src/main.s: test_cursors, test_protoerr, test_synth]
//
//   cursors   every fallback cursor image at S = 2 into a sheet PNG (--out FILE, default
//             build/cursors.png); red pixel = hot spot
//   protoerr  attach a buffer id that does not exist: the compositor answers with
//             wl_display.error, which ends the process with exit code 1
//   synth     drive the pointer / keyboard handlers with synthetic events (scale 2, seat
//             version 7, the US fallback keymap) and check the E_* events (no compositor)
// Rust only (no assembly counterpart; run behind a protocol proxy that plays the input
// devices and the other clipboard clients, see the plat-wayland report):
//   wlwin     an owned tool window, a popup over the main window, a custom cursor
//   wlclip    clipboard images: an image/png + image/bmp offer, an image/bmp-only offer
//             (clip_bmp_fix), our own CLIP_IMAGE / CLIP_PNG selection

use alloc::format;
use alloc::vec;

use gfx::{Dc, FMT_XRGB, Surface};
use plat::{
    BTN_LEFT, BTN_RIGHT, BTN_X1, E_DOWN, E_FOCUS_IN, E_FOCUS_OUT, E_KEY_DOWN, E_KEY_UP, E_LEAVE, E_MOVE, E_TEXT, E_UP,
    E_WHEEL, MOD_SHIFT,
};

use crate::{Lt, out};

const VK_SHIFT: u32 = 0x10;

/// every fallback cursor at S = 2 into a sheet PNG (--out FILE); red pixel = hot spot
pub fn test_cursors(t: &mut Lt) {
    let mut sheet = Surface::new(10 * 56 + 8, 64, FMT_XRGB);
    let mut dc = Dc::new(sheet.w, sheet.h);
    dc.begin(&sheet);
    dc.clear(&mut sheet, 0xFF808080);
    dc.end();
    let stride = sheet.stride as usize;
    let mut x0 = 8usize;
    for shape in 0..10u32 {
        let Some((w, h, hx, hy, _)) = plat::wl_cursor_image(shape) else { break };
        let (w, h) = (w as usize, h as usize);
        let mut px = vec![0u32; w * h * 4];
        plat::wl_cursor_render(&mut px, shape, 2);
        // copy opaque pixels
        for y in 0..h * 2 {
            for x in 0..w * 2 {
                let c = px[y * w * 2 + x];
                if c & 0xFF00_0000 == 0 {
                    continue;
                }
                let o = (y + 8) * stride + (x0 + x) * 4;
                if let Some(d) = sheet.px.get_mut(o..o + 4) {
                    d.copy_from_slice(&c.to_le_bytes());
                }
            }
        }
        let o = (hy as usize * 2 + 8) * stride + (x0 + hx as usize * 2) * 4;
        if let Some(d) = sheet.px.get_mut(o..o + 4) {
            d.copy_from_slice(&0xFFFF_0000u32.to_le_bytes());
        }
        x0 += w * 2 + 8;
    }
    let path = alloc::string::String::from(t.args.value("--out").unwrap_or("build/cursors.png"));
    let ok = match img_png::enc::encode_default(&sheet) {
        Some(png) => plat::write_file(&path, &png),
        None => false,
    };
    t.check(ok, "cursor sheet written");
    out(&format!("  -> {path}\n"));
}

/// an object id that does not exist: wl_display.error ends the process with exit code 1
pub fn test_protoerr(t: &mut Lt) {
    t.redraw();
    plat::wl_test_protoerr();
    t.pump(2000, 0);
    t.check(false, "protocol error should have ended the process");
}

/// a handler with decoded arguments (object id 1, udata 0)  [hcall / HC]
fn hc(iface: &str, ev: &str, a: &[i64]) {
    let mut v = [0u64; 4];
    for (k, &x) in a.iter().enumerate() {
        v[k] = x as u32 as u64; // the assembly passes 32-bit values, zero-extended
    }
    plat::wl_test_event(iface, ev, &v);
}

fn ptr(ev: &str, a: &[i64]) {
    hc("wl_pointer", ev, a);
}

fn kb(ev: &str, a: &[i64]) {
    hc("wl_keyboard", ev, a);
}

/// drive the pointer / keyboard handlers with synthetic events
pub fn test_synth(t: &mut Lt) {
    let tb = |typ: u8, btn: u8| typ as u32 | (btn as u32) << 8;
    plat::evq_clear();
    plat::wl_test_setup(240, 7); // scale 2, seat version 7
    ptr("enter", &[5, 0, 10 * 256 + 128, 20 * 256]);
    t.expect(tb(E_MOVE, 0), 21, 40, 0, "enter: 10.5,20 at scale 2 -> E_MOVE 21,40");
    ptr("motion", &[0, 100 * 256, -3 * 256]);
    t.expect(tb(E_MOVE, 0), 200, -6, 0, "motion -> E_MOVE 200,-6");
    ptr("button", &[7, 0, 0x110, 1]);
    t.expect(tb(E_DOWN, BTN_LEFT), 200, -6, 0, "left press -> E_DOWN 1");
    ptr("button", &[8, 0, 0x111, 1]);
    t.expect(tb(E_DOWN, BTN_RIGHT), 200, -6, 0, "right press -> E_DOWN 2");
    ptr("button", &[9, 0, 0x111, 0]);
    t.expect(tb(E_UP, BTN_RIGHT), 200, -6, 0, "right release -> E_UP 2");
    ptr("button", &[10, 0, 0x112, 0]);
    ptr("button", &[11, 0, 0x113, 1]);
    t.expect(tb(E_DOWN, BTN_X1), 200, -6, 0, "middle release without press ignored; side -> X1");
    ptr("axis", &[0, 0, 10 * 256]);
    ptr("axis_discrete", &[0, 1]);
    ptr("frame", &[]);
    t.expect(tb(E_WHEEL, 0), 200, -6, 0xFF88, "wheel down one notch -> dy -120");
    ptr("axis", &[0, 1, 5 * 256]);
    ptr("frame", &[]);
    t.expect(tb(E_WHEEL, 0), 200, -6, 60 << 16, "touchpad right 5 px -> dx 60");
    ptr("axis_value120", &[0, -60]);
    ptr("axis", &[0, 0, -5 * 256]);
    ptr("frame", &[]);
    t.expect(tb(E_WHEEL, 0), 200, -6, 60, "value120 up half notch -> dy 60");
    ptr("frame", &[]);
    ptr("leave", &[12, 0]);
    t.expect(tb(E_UP, BTN_LEFT), 200, -6, 0, "leave releases left");
    t.expect(tb(E_UP, BTN_X1), 200, -6, 0, "leave releases X1");
    t.expect(tb(E_LEAVE, 0), 0, 0, 0, "leave -> E_LEAVE (empty frame: no wheel)");
    // keyboard with the US fallback table
    plat::xkb_use_fallback();
    kb("enter", &[20, 0, 0]);
    t.expect(tb(E_FOCUS_IN, 0), 0, 0, 0, "keyboard enter -> E_FOCUS_IN");
    kb("key", &[21, 0, 42, 1]);
    t.expect(tb(E_KEY_DOWN, 0), 0, 0, VK_SHIFT, "Shift down");
    t.check(plat::wl_mods() & MOD_SHIFT as u32 != 0, "Shift key event already carries MOD_SHIFT (like Win32)");
    kb("modifiers", &[22, 1, 0, 0]);
    kb("key", &[23, 0, 16, 1]);
    t.expect(tb(E_KEY_DOWN, 0), 0, 0, 'Q' as u32, "Shift+q -> E_KEY_DOWN Q");
    t.expect(tb(E_TEXT, 0), 0, 0, 'Q' as u32, "Shift+q -> E_TEXT Q");
    plat::wl_input_service(plat::time_us() + 700_000);
    t.expect(tb(E_KEY_DOWN, 0), 0, 0, 0x8000_0051, "held 700 ms -> repeat E_KEY_DOWN Q | repeat");
    t.expect(tb(E_TEXT, 0), 0, 0, 'Q' as u32, "repeat -> E_TEXT Q");
    kb("key", &[24, 0, 16, 0]);
    t.expect(tb(E_KEY_UP, 0), 0, 0, 'Q' as u32, "q release -> E_KEY_UP");
    let r = plat::wl_input_service(plat::time_us() + 2_000_000);
    t.check(r == u64::MAX, "release stops the repeat");
    kb("leave", &[25, 0]);
    t.expect(tb(E_FOCUS_OUT, 0), 0, 0, 0, "keyboard leave -> E_FOCUS_OUT");
    t.check(plat::evq_count() == 0, "no stray events");
}

// the image data of the clipboard image test (wlclip; the proxy offers the same bytes)
const CLIP_PNG_DATA: &[u8] = b"\x89PNG fake image data for the clipboard test";
const CLIP_BMP_DATA: [u8; 70] = [
    66, 77, 70, 0, 0, 0, 0, 0, 0, 0, 54, 0, 0, 0, 40, 0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0, 1, 0, 32, 0, 0, 0, 0, 0, 16, 0, 0,
    0, 19, 11, 0, 0, 19, 11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 10, 20, 30, 0, 40, 50, 60, 0, 70, 80, 90, 0, 100, 110, 120, 0,
];

/// (no assembly counterpart) clipboard images: another client's image/png + image/bmp offer,
/// then an image/bmp-only offer (paint.net's alpha rule through clip_bmp_fix), then our own
/// CLIP_IMAGE / CLIP_PNG selection (read by the proxy).  Needs the wlinject.py "clipimg"
/// scenario.
pub fn test_wlclip(t: &mut Lt) {
    use plat::{CLIP_IMAGE, CLIP_PNG, CLIP_TEXT};
    if t.headless {
        t.check(false, "wlclip needs a window (run without --shot)");
        return;
    }
    t.redraw();
    let ok = t.pump(3000, plat::E_FOCUS_IN);
    t.check(ok, "keyboard focus (E_FOCUS_IN)");
    pump_log(300);
    t.check(plat::clip_has(CLIP_PNG) && plat::clip_has(CLIP_IMAGE) && !plat::clip_has(CLIP_TEXT),
            "offer of image/png + image/bmp: CLIP_PNG, CLIP_IMAGE, no CLIP_TEXT");
    t.check(plat::clip_get(CLIP_PNG).as_deref() == Some(CLIP_PNG_DATA), "clip_get(CLIP_PNG) reads image/png");
    t.check(plat::clip_get(CLIP_TEXT).is_none(), "clip_get(CLIP_TEXT) of an image offer -> None");
    t.check(plat::clip_get(CLIP_IMAGE).as_deref() == Some(CLIP_PNG_DATA), "clip_get(CLIP_IMAGE) prefers image/png");
    // the proxy now offers image/bmp only
    let end = plat::time_us() + 3_000_000;
    while plat::clip_has(CLIP_PNG) && plat::time_us() < end {
        pump_log(100);
    }
    t.check(!plat::clip_has(CLIP_PNG) && plat::clip_has(CLIP_IMAGE), "offer of image/bmp: CLIP_IMAGE only");
    let want = plat::clip_bmp_fix(&CLIP_BMP_DATA);
    let got = plat::clip_get(CLIP_IMAGE);
    t.check(want.is_some() && got == want, "clip_get(CLIP_IMAGE) reads image/bmp through clip_bmp_fix");
    t.check(plat::clip_get(CLIP_PNG).is_none(), "clip_get(CLIP_PNG) of a BMP offer -> None");
    // our own image selection
    let ok = plat::clip_set(CLIP_IMAGE, &plat::clipimg_pack(CLIP_PNG_DATA, &CLIP_BMP_DATA));
    t.check(ok && plat::clip_has(CLIP_PNG) && plat::clip_has(CLIP_IMAGE) && !plat::clip_has(CLIP_TEXT),
            "clip_set(CLIP_IMAGE): we offer CLIP_PNG and CLIP_IMAGE");
    t.check(plat::clip_get(CLIP_IMAGE).as_deref() == Some(CLIP_PNG_DATA), "our CLIP_IMAGE reads back as the PNG");
    pump_log(1500); // the proxy reads image/png and image/bmp
    t.check(!plat::clip_set(CLIP_IMAGE, &plat::clipimg_pack(&[], &CLIP_BMP_DATA)), "CLIP_IMAGE without a PNG -> false");
    t.check(!plat::clip_set(CLIP_IMAGE, &[1, 2, 3]), "a malformed CLIPIMG record -> false");
    t.check(plat::clip_set(CLIP_PNG, CLIP_PNG_DATA) && !plat::clip_has(CLIP_TEXT), "clip_set(CLIP_PNG)");
    t.check(plat::clip_get(CLIP_TEXT).is_none(), "clip_get(CLIP_TEXT) while we own a PNG -> None");
    pump_log(1500);
}

/// one event line with its window (wlwin)
fn ev_line(e: &plat::Event) {
    out(&format!("  ev {} btn {} x {} y {} data {:x} win {}\n", e.typ, e.btn, e.x, e.y, e.data, e.win));
}

/// pump events for ms, log every event
fn pump_log(ms: u32) {
    let deadline = plat::time_us() + ms as u64 * 1000;
    loop {
        plat::wait(deadline);
        while let Some(e) = plat::evq_pop() {
            ev_line(&e);
        }
        if plat::time_us() >= deadline {
            return;
        }
    }
}

/// a frame of one colour
fn solid(w: i32, h: i32, c: u32) -> alloc::vec::Vec<u8> {
    let mut v = vec![0u8; (w.max(0) * h.max(0) * 4) as usize];
    for p in v.chunks_exact_mut(4) {
        p.copy_from_slice(&c.to_le_bytes());
    }
    v
}

/// (no assembly counterpart) the window-indexed parts linuxtest's other tests do not reach:
/// an owned tool window (show, present, hide, show again, destroy), a popup over the main
/// window (show, redraw in place, move, hide), a custom cursor, a second owned window on the
/// freed index.  Run it under a protocol proxy for pointer input on the popup; every check
/// here is local.
pub fn test_wlwin(t: &mut Lt) {
    if t.headless {
        t.check(false, "wlwin needs a window (run without --shot)");
        return;
    }
    t.redraw();
    pump_log(300);
    let w = plat::win_create("linuxtest tool", 200, 120, plat::PWF_TOOL, 0, plat::PW_DEFAULT, plat::PW_DEFAULT);
    t.check(w == 1, "owned window -> index 1");
    let (pw, ph) = plat::win_size(w);
    out(&format!("tool window {pw} x {ph} px, dpi {}\n", plat::win_dpi(w)));
    t.check(pw == 200 * plat::win_dpi(0) / 96 && ph == 120 * plat::win_dpi(0) / 96, "its size at the owner's scale");
    plat::win_show(w, plat::PWS_SHOW);
    let px = solid(pw, ph, 0xFF20A040);
    plat::win_present(w, &plat::Frame { px: &px, w: pw, h: ph, stride: pw * 4 }, None);
    pump_log(800);
    // a popup over the main window, its pixel 0, 0 at owner pixel 30, 40
    let pp = solid(120, 80, 0xFFC03030);
    let f = plat::Frame { px: &pp, w: 120, h: 80, stride: 480 };
    plat::popup(0, 0, Some([30, 40, 120, 80]), Some(&f));
    pump_log(600);
    let pp2 = solid(120, 80, 0xFF3030C0);
    plat::popup(0, 0, Some([30, 40, 120, 80]), Some(&plat::Frame { px: &pp2, w: 120, h: 80, stride: 480 }));
    pump_log(300);
    plat::popup(0, 0, Some([60, 50, 120, 80]), Some(&f));
    pump_log(1500);
    plat::popup(0, 0, None, None);
    pump_log(200);
    // a custom cursor (premultiplied ARGB, 32 x 32, hot spot 4, 4)
    let cur = solid(32, 32, 0xFF000000);
    let id = plat::cursor_create(&cur, 32, 32, 4, 4, 96);
    t.check(id == plat::CUR_USER0 as i32, "cursor_create -> CUR_USER0");
    plat::win_set_cursor(0, id as u32);
    pump_log(200);
    plat::cursor_destroy(id);
    t.check(plat::cursor_create(&cur, 0, 32, 0, 0, 96) == -1, "cursor_create of width 0 -> -1");
    // hide / show the tool window again (its last frame comes back), then destroy it
    plat::win_show(w, plat::PWS_HIDE);
    pump_log(300);
    plat::win_show(w, plat::PWS_SHOW);
    pump_log(500);
    t.check(plat::win_size(w) == (pw, ph), "tool window keeps its size when shown again");
    plat::win_destroy(w);
    pump_log(200);
    t.check(plat::win_size(w) == (0, 0), "destroyed window has no size");
    let w2 = plat::win_create("linuxtest tool 2", 100, 80, plat::PWF_TOOL | plat::PWF_RESIZABLE, 0, 0, 0);
    t.check(w2 == 1, "the freed index is used again");
    plat::win_set_min_size(w2, 50, 40);
    plat::win_show(w2, plat::PWS_SHOWNA);
    let px2 = solid(100, 80, 0xFF808000);
    plat::win_present(w2, &plat::Frame { px: &px2, w: 100, h: 80, stride: 400 }, Some(&[[0, 0, 50, 40], [60, 50, 40, 30]]));
    pump_log(500);
    plat::win_set_title(w2, "linuxtest tool 2: retitled");
    plat::win_destroy(w2);
    pump_log(200);
    t.check(plat::win_create("x", 10, 10, 0, 5, 0, 0) == -1, "an owner that does not exist -> -1");
}
