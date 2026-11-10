// t_window_mac -- plattest --test window on macOS: a real-window smoke test of apple_mac.rs.  A
// short-lived window of our own (created hidden, shown, destroyed at the end) gets everything through
// the plat API; what the user would do is made as events for our own windows only (plat::win_test:
// WT_SEND, WT_MOVE, WT_POPUP_SEND in apple_mac_test.rs) -- nothing is injected into the system.
// Covered: create (hidden, the size in DIP at the window's scale), show (E_PAINT), present (the
// layer's image), set / get rect (own changes not reported), the user's move / resize (E_WINPOS /
// E_RESIZE), the minimum size, mouse / key / text / wheel translation, the hit hook, cursors (a system
// shape, a custom one, CUR_NONE), a popup (place, pixels, its input as events of the owner, a child
// window of the owner, hidden), an owned tool window (default place, PWS_SHOWNA, a child window that
// follows the owner), enable, the clipboard round trip (text, PNG, HTML), the input method's caret,
// the Save panel (closed by a timer: Cancel, then a name + OK), monitors, the system settings, wake()
// from a thread, close and destroy.  The clipboard's text is put back at the end.
// It needs the logged-in session: from SSH `python3 tools/apple.py gui build/mac/plattest --test window`.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use plat::taskbar::{JumpTask, TaskbarProgress, taskbar_badge, taskbar_progress, taskbar_supported, taskbar_tasks};
use plat::tray::{Note, TI_CHECKED, TI_DISABLED, TI_SEPARATOR, Tray, TrayEvent, TrayIcon, TrayItem, notify, notify_close, tray_remove, tray_set,
                 tray_supported, tray_take};
use plat::*;

use crate::T;

/// a frame of w x h pixels in one colour (0xRRGGBB)
struct Fb {
    px: Vec<u8>,
    w: i32,
    h: i32,
}

impl Fb {
    fn new(w: i32, h: i32, c: u32) -> Fb {
        let mut f = Fb { px: vec![0; (w * h * 4) as usize], w, h };
        f.fill(0, 0, w, h, c);
        f
    }

    fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, c: u32) {
        for yy in y..y + h {
            for xx in x..x + w {
                let o = ((yy * self.w + xx) * 4) as usize;
                self.px[o..o + 4].copy_from_slice(&c.to_le_bytes());
            }
        }
    }

    fn frame(&self) -> Frame<'_> {
        Frame { px: &self.px, w: self.w, h: self.h, stride: self.w * 4 }
    }
}

/// pump the OS for ms milliseconds -> the events that arrived
fn pump(ms: u64) -> Vec<Event> {
    let dl = time_us() + ms * 1000;
    let mut v = Vec::new();
    loop {
        wait(dl);
        while let Some(e) = evq_pop() {
            v.push(e);
        }
        if time_us() >= dl {
            return v;
        }
    }
}

/// pump until an event of type typ arrived (at most ms) -> it
fn pump_for(typ: u8, ms: u64) -> Option<Event> {
    let dl = time_us() + ms * 1000;
    loop {
        wait(dl);
        while let Some(e) = evq_pop() {
            if e.typ == typ {
                return Some(e);
            }
        }
        if time_us() >= dl {
            return None;
        }
    }
}

/// the events queued now: (type, button, window, x, y, data)
fn drain() -> Vec<(u8, u8, u8, i32, i32, u32)> {
    let mut got = Vec::new();
    while let Some(e) = evq_pop() {
        got.push((e.typ, e.btn, e.win, e.x, e.y, e.data));
    }
    got
}

fn dip(v: i32, dpi: i32) -> i32 {
    (v * dpi + 48) / 96
}

/// the caption strip (never clicked by the test: HB_DRAG starts the system's window drag); window 1
/// (the tool window): a text field right of x = 60
fn hit_test(w: i32, x: i32, y: i32) -> u32 {
    if w == 1 {
        return if x >= 60 { HB_TEXT } else { 0 };
    }
    if y < 30 { HB_DRAG } else { 0 }
}

fn pack(x: i32, y: i32, w: i32, h: i32) -> i64 {
    x as u16 as i64 | (y as u16 as i64) << 16 | (w as i64) << 32 | (h as i64) << 48
}

/// a 2 x 2 BMP file: BITMAPV5HEADER, 32 bit, BI_BITFIELDS, straight alpha
fn bmp_v5() -> Vec<u8> {
    let mut f = vec![0u8; 14 + 124];
    f[0] = b'B';
    f[1] = b'M';
    f[2..6].copy_from_slice(&(138u32 + 16).to_le_bytes());
    f[10..14].copy_from_slice(&138u32.to_le_bytes());
    let v = &mut f[14..];
    v[0..4].copy_from_slice(&124u32.to_le_bytes());
    v[4..8].copy_from_slice(&2u32.to_le_bytes());
    v[8..12].copy_from_slice(&2u32.to_le_bytes());
    v[12..14].copy_from_slice(&1u16.to_le_bytes());
    v[14..16].copy_from_slice(&32u16.to_le_bytes());
    v[16..20].copy_from_slice(&3u32.to_le_bytes());
    v[20..24].copy_from_slice(&16u32.to_le_bytes());
    for (k, m) in [0x00FF_0000u32, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000].iter().enumerate() {
        v[40 + k * 4..44 + k * 4].copy_from_slice(&m.to_le_bytes());
    }
    v[56..60].copy_from_slice(&0x7352_4742u32.to_le_bytes());
    for p in [0x80FF_0000u32, 0xFF00_FF00, 0x4000_00FF, 0x0000_0000] {
        f.extend_from_slice(&p.to_le_bytes());
    }
    f
}

/// text, PNG, images and HTML through the general pasteboard
fn clip_round_trip(t: &mut T) {
    let saved = clip_get(CLIP_TEXT);
    let text = "Grüße\nline 2 \u{1F600}";
    let ok = clip_set(CLIP_TEXT, text.as_bytes());
    t.check(ok && clip_has(CLIP_TEXT) && clip_get(CLIP_TEXT).as_deref() == Some(text.as_bytes()), "clip: text round trip (UTF-8, \\n, a pair)");
    // a real PNG of 1 x 1 pixel (the pasteboard may look into images)
    let png: Vec<u8> = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0x64, 0x60, 0xF8, 0x5F, 0x0F, 0x00, 0x02, 0x87, 0x01, 0x80, 0xEB, 0x47, 0xBA, 0x92, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82];
    let ok = clip_set(CLIP_PNG, &png);
    let (has_png, has_text) = (clip_has(CLIP_PNG), clip_has(CLIP_TEXT));
    t.check(ok && has_png && !has_text, "clip: CLIP_PNG replaces the text");
    if !(ok && has_png && !has_text) {
        t.info(&format!("  (set {ok}, PNG there {has_png}, text there {has_text})\n"));
    }
    let back = clip_get(CLIP_PNG);
    t.check(back.as_deref() == Some(&png[..]), "clip: PNG bytes round trip");
    if back.as_deref() != Some(&png[..]) {
        t.info(&format!("  (got {:?} bytes)\n", back.map(|b| b.len())));
    }
    let ok = clip_set(CLIP_IMAGE, &clipimg_pack(&png, &[]));
    t.check(ok && clip_has(CLIP_IMAGE) && clip_get(CLIP_IMAGE).as_deref() == Some(&png[..]), "clip: CLIP_IMAGE gives its PNG");
    let ok = clip_set(CLIP_IMAGE, &clipimg_pack(&[], &bmp_v5()));
    let img = clip_get(CLIP_IMAGE).unwrap_or_default();
    t.check(ok && img.starts_with(b"\x89PNG\r\n\x1a\n") && clip_has(CLIP_PNG), "clip: CLIP_IMAGE of a BMP alone: a PNG made of it, for other programs too");
    let html = "<b>fett</b> ä";
    let ok = clip_set_many(&[(CLIP_TEXT, b"fett \xC3\xA4"), (CLIP_HTML, html.as_bytes())]);
    t.check(ok && clip_get(CLIP_HTML).as_deref() == Some(html.as_bytes()) && clip_get(CLIP_TEXT).as_deref() == Some("fett ä".as_bytes()),
            "clip: text + HTML of one selection");
    clip_set(CLIP_TEXT, saved.as_deref().unwrap_or(b""));
}

pub fn t_window(t: &mut T) {
    evq_clear();
    let w = win_create("plattest window", 320, 200, PWF_RESIZABLE, PW_NOOWNER, PW_DEFAULT, PW_DEFAULT);
    t.check(w == 0, "window: win_create (main, library frame) -> index 0");
    if w != 0 {
        t.info("  (no window server: run it in the logged-in session, python3 tools/apple.py gui ...)\n");
        return;
    }
    let dpi = win_dpi(0);
    t.info(&format!("  (dpi {dpi})\n"));
    let (cw, ch) = win_size(0);
    t.check((cw, ch) == (dip(320, dpi), dip(200, dpi)), "window: client size = 320 x 200 DIP at the window's scale");
    t.check(win_test(WT_VISIBLE, 0, [0; 4]) == 0, "window: created hidden");
    win_show(0, PWS_SHOW);
    let ev = pump(300);
    t.check(win_test(WT_VISIBLE, 0, [0; 4]) == 1, "window: win_show shows it");
    t.check(ev.iter().any(|e| e.typ == E_PAINT && e.win == 0), "window: shown -> E_PAINT");
    let active = win_test(WT_ACTIVE, 0, [0; 4]) == 1;
    t.info(&format!("  (key window after win_show: {}, state {:#x})\n", active as i32, win_state(0)));

    // ---- present
    let (ca, cb) = (0x0033_6699u32, 0x00CC_2211);
    let mut fb = Fb::new(cw, ch, ca);
    win_present(0, &fb.frame(), None);
    t.check(win_test(WT_PIXEL, 0, [5, 5, 0, 0]) == ca as i64, "present: the frame becomes the layer's image");
    fb.fill(10, 10, 20, 20, cb);
    win_present(0, &fb.frame(), Some(&[[10, 10, 20, 20]]));
    t.check(win_test(WT_PIXEL, 0, [15, 15, 0, 0]) == cb as i64 && win_test(WT_PIXEL, 0, [5, 5, 0, 0]) == ca as i64,
            "present: a later frame replaces it");

    // ---- size, position
    let r0 = win_get_rect(0).unwrap_or_default();
    evq_clear();
    win_set_rect(0, PW_KEEP, PW_KEEP, 400, 300);
    let ev = pump(50);
    t.check(win_size(0) == (400, 300), "size: win_set_rect sets the client size (px)");
    t.check(!ev.iter().any(|e| e.typ == E_RESIZE || e.typ == E_WINPOS), "size: ... not reported back (own change)");
    let r1 = win_get_rect(0).unwrap_or_default();
    t.check(r1 == [r0[0], r0[1], 400, 300], "size: win_get_rect = client origin + size, the top left stays");
    if r1 != [r0[0], r0[1], 400, 300] {
        t.info(&format!("  (before {r0:?}, after {r1:?})\n"));
    }
    win_test(WT_MOVE, 0, [r1[0] + 20, r1[1] + 10, 360, 240]);
    let ev = pump(50);
    t.check(ev.iter().any(|e| e.typ == E_WINPOS && (e.x, e.y) == (r1[0] + 20, r1[1] + 10)), "size: a move by the user -> E_WINPOS (client origin)");
    if !ev.iter().any(|e| e.typ == E_WINPOS && (e.x, e.y) == (r1[0] + 20, r1[1] + 10)) {
        let seen: Vec<_> = ev.iter().filter(|e| e.typ == E_WINPOS || e.typ == E_RESIZE).map(|e| (e.typ, e.win, e.x, e.y)).collect();
        t.info(&format!("  (want {:?}, got {seen:?}, rect now {:?})\n", (r1[0] + 20, r1[1] + 10), win_get_rect(0)));
    }
    t.check(ev.iter().any(|e| e.typ == E_RESIZE && (e.x, e.y) == (360, 240)), "size: a resize by the user -> E_RESIZE");
    win_set_min_size(0, 300, 200);
    t.check(win_test(WT_MIN_SIZE, 0, [0; 4]) == 300 | 200 << 32, "size: win_set_min_size -> the window's minimum content size (DIP)");
    win_set_min_size(0, 0, 0);

    // ---- input: moves, buttons, keys, text, the wheel
    win_hit_hook(Some(hit_test));
    evq_clear();
    win_test(WT_SEND, 0, [TS_MOVE, 10, 40, 0]);
    win_test(WT_SEND, 0, [TS_DOWN, 12, 42, 0]);
    win_test(WT_SEND, 0, [TS_UP, 14, 44, 0]);
    win_test(WT_SEND, 0, [TS_KEY, 0, 'a' as i32, 0]); // kVK_ANSI_A
    win_test(WT_SEND, 0, [TS_TEXT, 0x1F600, 0, 0]);
    let got = drain();
    let want = [(E_MOVE, 0, 0, 10, 40, 0), (E_DOWN, BTN_LEFT, 0, 12, 42, 0), (E_UP, BTN_LEFT, 0, 14, 44, 0), (E_KEY_DOWN, 0, 0, 0, 0, 0x41),
                (E_TEXT, 0, 0, 0, 0, 'a' as u32), (E_TEXT, 0, 0, 0, 0, 0x1F600)];
    t.check(got == want, "input: move, button, key (VK, the input system's text), a character beyond the BMP -> events");
    if got != want {
        t.info(&format!("  (got {got:?})\n"));
    }
    win_test(WT_SEND, 0, [TS_WHEEL, 2, 0, 0]);
    let got = drain();
    t.check(got.iter().any(|e| e.0 == E_WHEEL && e.5 & 0xFFFF == 240), "input: two lines up -> E_WHEEL 240");
    if !got.iter().any(|e| e.0 == E_WHEEL && e.5 & 0xFFFF == 240) {
        t.info(&format!("  (wheel: {got:?})\n"));
    }
    // the trackpad's pinch: Ctrl + wheel, 120 per 0.2 of magnification
    evq_clear();
    win_test(WT_SEND, 0, [TS_PINCH, 250, 20, 50]);
    let e = evq_pop();
    t.check(e.is_some_and(|e| e.typ == E_WHEEL && e.mods & MOD_CTRL != 0 && e.data & 0xFFFF == 150 && (e.x, e.y) == (20, 50)),
            "input: a pinch of 0.25 -> Ctrl + wheel 150 at the point");
    if !e.is_some_and(|e| e.typ == E_WHEEL && e.mods & MOD_CTRL != 0 && e.data & 0xFFFF == 150) {
        t.info(&format!("  (pinch: {:?})\n", e.map(|e| (e.typ, e.mods, e.x, e.y, e.data))));
    }
    win_hit_hook(None);

    // ---- cursors
    let mut cur = vec![0u8; 32 * 32 * 4];
    for (k, p) in cur.chunks_exact_mut(4).enumerate() {
        let (x, y) = ((k % 32) as i32 - 16, (k / 32) as i32 - 16);
        if x * x + y * y < 144 {
            p.copy_from_slice(&0xFFFF_0000u32.to_le_bytes());
        }
    }
    win_set_cursor(0, CUR_IBEAM);
    t.check(win_test(WT_CURSOR_IS, 0, [CUR_IBEAM as i32, 0, 0, 0]) == 1, "cursor: a system shape");
    let id = cursor_create(&cur, 32, 32, 16, 16, dpi);
    t.check(id >= CUR_USER0 as i32, "cursor: a custom cursor");
    win_set_cursor(0, id as u32);
    t.check(win_test(WT_CURSOR_IS, 0, [id, 0, 0, 0]) == 1, "cursor: ... shown");
    win_set_cursor(0, CUR_NONE);
    t.check(win_test(WT_CURSOR_IS, 0, [CUR_NONE as i32, 0, 0, 0]) == 1, "cursor: CUR_NONE hides it");
    cursor_destroy(id);
    win_set_cursor(0, CUR_ARROW);

    // ---- popup
    let pd = 0x0055_AA77u32;
    let pf = Fb::new(50, 40, pd);
    popup(0, 0, Some([10, 10, 50, 40]), Some(&pf.frame()));
    pump(30);
    let o = win_get_rect(0).unwrap_or_default();
    let pr = win_test(WT_POPUP_RECT, 0, [0; 4]);
    t.check(pr == pack(o[0] + 10, o[1] + 10, 50, 40), "popup: shown at the owner's client point, its size");
    if pr != pack(o[0] + 10, o[1] + 10, 50, 40) {
        t.info(&format!("  (popup rect {:?}, owner {o:?})\n", [pr as u16 as i16, (pr >> 16) as u16 as i16, (pr >> 32) as u16 as i16, (pr >> 48) as i16]));
    }
    t.check(win_test(WT_POPUP_PIXEL, 0, [25, 20, 0, 0]) == pd as i64, "popup: its pixels");
    t.check(win_test(WT_PARENT, 0, [1, 0, 0, 0]) == 0, "popup: a child window of the owner");
    evq_clear();
    win_test(WT_POPUP_SEND, 0, [TS_MOVE, 0, 5, 6]);
    win_test(WT_POPUP_SEND, 0, [TS_DOWN, 0, 5, 6]);
    win_test(WT_POPUP_SEND, 0, [TS_UP, 0, 5, 6]);
    let got: Vec<_> = drain().into_iter().map(|e| (e.0, e.1, e.2, e.3, e.4)).collect();
    t.check(got == [(E_MOVE, 0, 0, 15, 16), (E_DOWN, BTN_LEFT, 0, 15, 16), (E_UP, BTN_LEFT, 0, 15, 16)],
            "popup: its input comes as events of the owner in owner coordinates");
    if got != [(E_MOVE, 0, 0, 15, 16), (E_DOWN, BTN_LEFT, 0, 15, 16), (E_UP, BTN_LEFT, 0, 15, 16)] {
        t.info(&format!("  (got {got:?})\n"));
    }
    t.check(win_test(WT_POPUP_RECT, 0, [0; 4]) != -1, "popup: still shown after a click into it");
    popup(0, 0, None, None);
    t.check(win_test(WT_POPUP_RECT, 0, [0; 4]) == -1 && win_test(WT_PARENT, 0, [1, 0, 0, 0]) == -1, "popup: hidden, off the owner");

    // ---- an owned tool window
    let w1 = win_create("plattest tool", 100, 80, PWF_TOOL | PWF_SYSFRAME, 0, PW_DEFAULT, PW_DEFAULT);
    t.check(w1 == 1, "tool: win_create (owned) -> index 1");
    let o = win_get_rect(0).unwrap_or_default();
    let t1 = win_get_rect(1).unwrap_or_default();
    let d = dip(32, win_dpi(1));
    t.check(t1[0] == o[0] + d && t1[1] == o[1] + d, "tool: default place = owner client origin + 32 DIP");
    let key0 = win_test(WT_ACTIVE, 0, [0; 4]);
    win_show(1, PWS_SHOWNA);
    pump(50);
    t.check(win_test(WT_ACTIVE, 1, [0; 4]) == 0 && win_test(WT_ACTIVE, 0, [0; 4]) == key0, "tool: PWS_SHOWNA does not activate it");
    t.check(win_test(WT_PARENT, 1, [0; 4]) == 0, "tool: a child window of its owner");
    // a panel: a click leaves the keyboard with the owner, a click into a text field takes it
    win_hit_hook(Some(hit_test));
    win_test(WT_SEND, 1, [TS_DOWN, 20, 30, 0]);
    win_test(WT_SEND, 1, [TS_UP, 20, 30, 0]);
    let after_button = win_test(WT_ACTIVE, 1, [0; 4]);
    win_test(WT_SEND, 1, [TS_DOWN, 80, 30, 0]);
    win_test(WT_SEND, 1, [TS_UP, 80, 30, 0]);
    let after_text = win_test(WT_ACTIVE, 1, [0; 4]);
    if active {
        t.check(after_button == 0 && after_text == 1, "tool: a click keeps the keyboard with the owner, a click into a text field takes it");
        if (after_button, after_text) != (0, 1) {
            t.info(&format!("  (key after the button: {after_button}, after the text field: {after_text})\n"));
        }
    } else {
        t.info("  (tool: the app is not active -- the screen locked? -- the panel's keyboard is not tested)\n");
    }
    win_hit_hook(None);
    win_activate(0);
    pump(50);
    evq_clear();
    win_test(WT_MOVE, 0, [o[0] + 30, o[1], o[2], o[3]]);
    let ev = pump(50);
    let t2 = win_get_rect(1).unwrap_or_default();
    t.check(t2[0] == t1[0] + 30 && t2[1] == t1[1], "tool: it follows a move of the owner");
    t.check(ev.iter().any(|e| e.typ == E_WINPOS && e.win == 1), "tool: ... reported as E_WINPOS of window 1");
    win_enable(0, false);
    evq_clear();
    win_test(WT_SEND, 0, [TS_DOWN, 50, 50, 0]);
    win_test(WT_SEND, 0, [TS_UP, 50, 50, 0]);
    let off = (win_test(WT_ENABLED, 0, [0; 4]), drain().len());
    win_enable(0, true);
    t.check(off == (0, 0) && win_test(WT_ENABLED, 0, [0; 4]) == 1, "tool: win_enable: a disabled owner takes no input");
    win_destroy(1);
    t.check(win_get_rect(1).is_none(), "tool: destroyed");

    // ---- clipboard (the text there is put back at the end)
    clip_round_trip(t);

    // ---- the input method's caret
    ime_pos(30, 40, 16);
    let c = win_test(WT_CARET, 0, [0; 4]);
    ime_pos(0, 0, 0);
    t.check(c == 30 | 40 << 32, "ime: the candidate window goes to the caret");
    if c != 30 | 40 << 32 {
        t.info(&format!("  (caret at {}, {})\n", c as i32, c >> 32));
    }

    // ---- the Save panel
    let mut path = String::from("build/plattest_fd.png");
    file_dialog_autoclose(500, None);
    let r = file_dialog(FD_SAVE, &mut path, &["png=PNG-Bild", "jpg;jpeg=JPEG-Bild"]);
    t.check(r.0 == 0, "filedlg: the system Save panel, cancelled");
    let full = format!("{}/build/plattest_fd2.jpg", cwd());
    file_dialog_autoclose(500, Some(&full));
    let r = file_dialog(FD_SAVE | 1 << FD_EXT_SHIFT, &mut path, &["png=PNG-Bild", "jpg;jpeg=JPEG-Bild"]);
    t.info(&format!("  (file dialog: {} {} {path})\n", r.0, r.1));
    t.check(r == (1, 1) && path == full, "filedlg: a name + OK -> the path, the type");

    // ---- the print panel (cancelled by a timer: nothing reaches a printer)
    let px = vec![0xFFu8; 8 * 8 * 4];
    let mut job = PrintJob::new(Some(PrintSrc { px: &px, w: 8, h: 8, stride: 32, fmt: PRINT_ARGB }), "plattest");
    print_dialog_autoclose(500);
    let r = print(&mut job);
    t.check(r == ERR_CANCEL && job.copies == 0, "print: the system's print panel, cancelled -> ERR_CANCEL");
    if r != ERR_CANCEL {
        t.info(&format!("  (print answered {r})\n"));
    }

    // ---- monitors, settings, process level
    let mons = monitors();
    for m in &mons {
        t.info(&format!("  (monitor {} '{}' {:?} work {:?} dpi {} {} mHz{})\n", m.id, m.name, m.rect, m.work, m.dpi, m.refresh_mhz,
                        if m.primary { " primary" } else { "" }));
    }
    t.check(!mons.is_empty() && mons[0].primary && mons[0].dpi >= 96 && mons[0].rect[2] > 0 && mons[0].work[3] <= mons[0].rect[3],
            "monitors: the primary screen first, its size, dpi and work area");
    let ss = sys_settings();
    t.info(&format!("  (settings: dark {:?}, accent {:x?}, contrast {}, reduce motion {}, double click {} ms)\n", ss.dark, ss.accent,
                    ss.high_contrast, ss.reduce_motion, ss.double_click_ms));
    t.check(ss.dark.is_some() && ss.accent.is_some() && ss.double_click_ms > 0, "settings: appearance, accent colour, double-click time");
    let th = theme();
    let home = home().unwrap_or_default();
    t.check(th == 0 || th == 1, "system: theme() reads the appearance (0 light / 1 dark)");
    t.check(!home.is_empty() && file_exists(&home) == 2, "system: home() is a folder");

    // ---- the app in the system (docs/APPLE.md AP7a)
    let m = win_test(WT_MENU, 0, [0; 4]);
    t.check(m >= 0 && m & 0xFF == 2 && (m >> 8) & 0xFF >= 5 && (m >> 16) & 1 == 1, "app: the main menu: the app menu (Quit is Cmd+Q) and the Window menu");
    evq_clear();
    win_test(WT_TERMINATE, 0, [0; 4]);
    let e = pump_for(E_CLOSE, 500);
    t.check(e.is_some_and(|e| e.win == 0), "app: Cmd+Q / the Dock's Quit asks the main window (E_CLOSE), the app goes on");
    for (k, typ, data, what) in [(0, E_POWER, 1, "will sleep -> E_POWER 1"), (1, E_POWER, 0, "did wake -> E_POWER 0"),
                                 (2, E_SESSION_END, 0, "will power off -> E_SESSION_END")] {
        evq_clear();
        win_test(WT_WORKSPACE, 0, [k, 0, 0, 0]);
        let e = pump_for(typ, 500);
        t.check(e.is_some_and(|e| e.data == data), &format!("app: NSWorkspace {what}"));
    }
    evq_clear();
    win_test(WT_SIGTERM, 0, [0; 4]);
    let e = pump_for(E_SESSION_END, 1000);
    t.check(e.is_some(), "app: SIGTERM -> E_SESSION_END (the app saves and ends itself)");

    // ---- services (docs/APPLE.md AP7b): the status item, notifications, the Dock, opened files, shortcuts
    let menu = vec![
        TrayItem { id: 11, label: String::from("Öffnen"), ..TrayItem::default() },
        TrayItem { flags: TI_SEPARATOR, ..TrayItem::default() },
        TrayItem { id: 12, label: String::from("Pause"), flags: TI_CHECKED, ..TrayItem::default() },
        TrayItem { id: 13, label: String::from("Gesperrt"), flags: TI_DISABLED, ..TrayItem::default() },
    ];
    let icon = TrayIcon { w: 16, h: 16, px: vec![0xFF33_66CC; 256] };
    t.check(tray_supported() && tray_set(&Tray { icon: Some(icon), tooltip: String::from("plattest"), menu }), "tray: a status item with a menu");
    t.check(win_test(WT_TRAY, 0, [0; 4]) == 4, "tray: its menu has the four rows");
    tray_take();
    win_test(WT_TRAY, 0, [1, 2, 0, 0]);
    t.check(tray_take() == [TrayEvent::Menu(12)], "tray: a chosen row -> TrayEvent::Menu(its id)");
    t.check(tray_set(&Tray { icon: None, tooltip: String::from("plattest"), menu: Vec::new() }) && win_test(WT_TRAY, 0, [0; 4]) == 0,
            "tray: without a menu (the app's icon)");
    win_test(WT_TRAY, 0, [2, 0, 0, 0]);
    t.check(tray_take() == [TrayEvent::Activate], "tray: ... a click is Activate");
    tray_remove();
    t.check(win_test(WT_TRAY, 0, [0; 4]) == -1, "tray: removed");
    let note = Note { title: String::from("plattest"), body: String::from("Eine Mitteilung des Fenstertests"), icon: None };
    if win_test(WT_BUNDLE, 0, [0; 4]) == 1 {
        let id = notify(&note);
        t.check(id != 0, "notify: in the app bundle a notification is sent (macOS asks once for the permission)");
        tray_take();
        win_test(WT_NOTE_CLICK, 0, [id as i32, 0, 0, 0]);
        t.check(tray_take() == [TrayEvent::NoteClicked(id)], "notify: a click on it -> NoteClicked(its id)");
        notify_close(id);
    } else {
        t.check(notify(&note) == 0, "notify: a program outside an app bundle gets none (0, no crash)");
    }
    t.check(taskbar_supported() && taskbar_badge(3, None) && win_test(WT_DOCK, 0, [0; 4]) == 3, "dock: the badge shows 3");
    t.check(taskbar_badge(0, None) && win_test(WT_DOCK, 0, [0; 4]) == -1, "dock: ... and goes");
    t.check(taskbar_progress(TaskbarProgress::Normal(0.4)) && win_test(WT_DOCK, 0, [1, 0, 0, 0]) == 1, "dock: progress drawn into the tile");
    t.check(taskbar_progress(TaskbarProgress::None) && win_test(WT_DOCK, 0, [1, 0, 0, 0]) == 0, "dock: ... and taken away");
    let tasks = [JumpTask { title: String::from("Neues Fenster"), args: vec![String::from("--new")], desc: String::new() },
                 JumpTask { title: String::from("Datei"), args: vec![String::from("a b.txt")], desc: String::from("öffnet a b.txt") }];
    t.check(taskbar_tasks(&tasks) && win_test(WT_DOCK, 0, [2, 0, 0, 0]) == 2, "dock: the tasks as the Dock menu");
    plat::instance::take();
    win_test(WT_DOCK, 0, [3, 1, 0, 0]);
    let reqs = plat::instance::take();
    t.check(reqs.len() == 1 && reqs[0].args == ["a b.txt"] && reqs[0].paths() == [format!("{}/a b.txt", cwd())],
            "dock: a chosen task reaches the app as a later start (UIC_OPEN)");
    taskbar_tasks(&[]);
    win_test(WT_OPEN_URLS, 0, [0; 4]);
    let reqs = plat::instance::take();
    t.check(reqs.len() == 1 && reqs[0].paths() == [format!("{}/build/plattest_open.txt", cwd()), String::from("https://example.org/jflib")],
            "open: application:openURLs: -> the paths and URLs as a later start's (UIC_OPEN)");
    let chord = plat::hotkey::Chord::parse("Ctrl+Alt+Shift+F13").unwrap_or_default();
    let reg = plat::hotkey::hotkey_register(7, chord, "plattest");
    t.check(reg.is_ok(), "hotkey: Ctrl+Alt+Shift+F13 registered (Carbon)");
    if reg.is_err() {
        t.info(&format!("  (hotkey: {reg:?})\n"));
    }
    plat::hotkey::hotkey_take();
    t.check(hotkey_test_post(7) && plat::hotkey::hotkey_take() == [plat::hotkey::HotkeyEvent::Pressed(7)], "hotkey: its event -> Pressed(7)");
    t.check(plat::hotkey::hotkey_register(8, chord, "plattest") == Err(plat::hotkey::HotkeyError::Conflict), "hotkey: the same chord again -> Conflict");
    t.check(plat::hotkey::hotkey_unregister(7), "hotkey: unregistered");

    // ---- the app's menus in the menu bar (plat::sysmenu: docs/APPLE.md "Menüs in der Menüleiste")
    {
        use plat::sysmenu::{SysMenuItem, sysmenu_set, sysmenu_supported, sysmenu_take};
        let row = |id: u32, label: &str, flags: u32| SysMenuItem { id, label: String::from(label), flags, ..SysMenuItem::default() };
        let menus = vec![
            SysMenuItem { label: String::from("Datei"), sub: vec![
                SysMenuItem { key: String::from("n"), mods: MOD_CTRL as u32, ..row(21, "Neu", 0) },
                row(0, "", TI_SEPARATOR),
                row(22, "Raster", TI_CHECKED),
                row(23, "Gesperrt", TI_DISABLED),
            ], ..SysMenuItem::default() },
            SysMenuItem { label: String::from("Ansicht"), sub: vec![
                SysMenuItem { label: String::from("Zoom"), sub: vec![row(31, "100 %", 0)], ..SysMenuItem::default() },
            ], ..SysMenuItem::default() },
        ];
        t.check(sysmenu_supported(), "sysmenu: with a window the app's menus go into the menu bar");
        sysmenu_set(&menus);
        t.check(win_test(WT_SYSMENU, 0, [0; 4]) == 2, "sysmenu: two menus between the app menu and the Window menu");
        t.check(win_test(WT_SYSMENU, 0, [1, 0, 0, 0]) == 4, "sysmenu: Datei has its four rows");
        t.check(win_test(WT_SYSMENU, 0, [3, 0, 0, 0]) == 1 | 4, "sysmenu: Neu is enabled, with Command+N");
        t.check(win_test(WT_SYSMENU, 0, [3, 0, 2, 0]) == 1 | 2, "sysmenu: Raster is checked");
        t.check(win_test(WT_SYSMENU, 0, [3, 0, 3, 0]) == 0, "sysmenu: Gesperrt is disabled");
        t.check(win_test(WT_SYSMENU, 0, [1, 1, 0, 0]) == 1, "sysmenu: Ansicht holds the submenu Zoom");
        sysmenu_take();
        win_test(WT_SYSMENU, 0, [2, 0, 2, 0]);
        t.check(sysmenu_take() == [22], "sysmenu: a chosen row -> its id");
        sysmenu_set(&menus[..1]);
        t.check(win_test(WT_SYSMENU, 0, [0; 4]) == 1, "sysmenu: a new list replaces the menus");
        sysmenu_set(&[]);
        t.check(win_test(WT_SYSMENU, 0, [0; 4]) == 0 && win_test(WT_MENU, 0, [0; 4]) & 0xFF == 2,
                "sysmenu: none again, the app menu and the Window menu stay");
    }

    // ---- wake from another thread, close, destroy
    evq_clear();
    let th = thread_spawn(0, Box::new(|| {
        sleep_ms(20);
        wake();
    }));
    let e = pump_for(E_WAKE, 2000);
    if let Some(th) = th {
        th.join();
    }
    t.check(e.is_some(), "wake: wake() from a thread -> E_WAKE");
    win_cmd(0, WCMD_CLOSE);
    let e = pump_for(E_CLOSE, 500);
    t.check(e.is_some_and(|e| e.win == 0), "close: WCMD_CLOSE -> E_CLOSE");
    win_destroy(0);
    let e = pump_for(E_CLOSE, 500);
    t.check(e.is_some() && win_size(0) == (0, 0), "close: destroying the main window ends the loop (E_CLOSE)");
    evq_clear();
}
