// t_window -- (plat-win) plattest --test window: a real-window smoke test of the Windows
// platform layer (win_api.rs and its modules).  A short-lived window of our own (created
// hidden, shown, destroyed at the end) gets everything through the plat API; messages the user
// would cause are sent to our own windows only (plat::win_test_send / win_test) -- nothing is
// injected into the system.  Covered: create (hidden, size in DIP at the window's dpi), show,
// present (whole frame, dirty rectangles, WM_PAINT from the kept copy), resize (own: not
// reported; the user's: E_WINPOS / E_RESIZE), the minimum size, DPI change (E_DPI before
// E_RESIZE), the hit hook (WM_NCHITTEST), mouse / wheel / key / text translation, custom
// cursors (WM_SETCURSOR), popup windows, an owned tool window (default place, no activation,
// docked to the owner when it moves), enable, the clipboard round trip (text, PNG, image +
// DIB), the IME caret, the system file dialog (closed by a timer: Cancel, then a name + OK),
// a WM_DROPFILES of jfgui_TEST_DROPFILES, wake() from a thread, close and destroy.
// The clipboard's text is saved and put back at the end.  Windows only (Linux: linuxtest).

#[cfg(not(windows))]
pub fn t_window(t: &mut crate::T) {
    t.info("  (window: the Windows smoke test; X11 / Wayland have linuxtest)\n");
}

#[cfg(windows)]
pub use win::t_window;

#[cfg(windows)]
mod win {
    use alloc::boxed::Box;
    use alloc::format;
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;
    use core::sync::atomic::{AtomicU32, Ordering::SeqCst};

    use plat::*;

    use crate::T;

    const WM_ACTIVATEAPP: u32 = 0x001C;
    const WM_ENTERSIZEMOVE: u32 = 0x0231;
    const WM_EXITSIZEMOVE: u32 = 0x0232;
    const WM_SETCURSOR: u32 = 0x0020;

    static REFRESHED: AtomicU32 = AtomicU32::new(0);

    fn refresh() {
        REFRESHED.fetch_add(1, SeqCst);
    }
    const WM_MOUSEACTIVATE: u32 = 0x0021;
    const WM_NCHITTEST: u32 = 0x0084;
    const WM_KEYDOWN: u32 = 0x0100;
    const WM_CHAR: u32 = 0x0102;
    const WM_MOUSEMOVE: u32 = 0x0200;
    const WM_LBUTTONDOWN: u32 = 0x0201;
    const WM_LBUTTONUP: u32 = 0x0202;
    const WM_MOUSEWHEEL: u32 = 0x020A;
    const HTCLIENT: isize = 1;

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

    fn dip(v: i32, dpi: i32) -> i32 {
        (v * dpi + 48) / 96
    }

    fn lp(x: i32, y: i32) -> isize {
        ((x as u32 & 0xFFFF) | (y as u32) << 16) as i32 as isize
    }

    fn hit_test(w: i32, _x: i32, y: i32) -> u32 {
        if w == 1 {
            return HB_TEXT;
        }
        if y < 30 {
            HB_DRAG
        } else if y < 40 {
            HB_CLOSE
        } else {
            0
        }
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

    /// text, PNG, image + DIB through the system clipboard
    fn clip_round_trip(t: &mut T) {
        let saved = clip_get(CLIP_TEXT);
        let text = "Grüße\nline 2 \u{1F600}";
        let ok = clip_set(CLIP_TEXT, text.as_bytes());
        t.check(ok && clip_has(CLIP_TEXT) && clip_get(CLIP_TEXT).as_deref() == Some(text.as_bytes()),
                "clip: text round trip (UTF-8, \\n, a pair)");
        let png: Vec<u8> = b"\x89PNG\r\n\x1a\nnot really a png".to_vec();
        let ok = clip_set(CLIP_PNG, &png);
        t.check(ok && clip_has(CLIP_PNG) && clip_has(CLIP_IMAGE) && !clip_has(CLIP_TEXT), "clip: CLIP_PNG replaces the text");
        t.check(clip_get(CLIP_PNG).as_deref() == Some(&png[..]), "clip: PNG bytes round trip");
        let bmp = bmp_v5();
        let ok = clip_set(CLIP_IMAGE, &clipimg_pack(&png, &bmp));
        t.check(ok && clip_get(CLIP_IMAGE).as_deref() == Some(&png[..]), "clip: CLIP_IMAGE gives the PNG first");
        let ok = clip_set(CLIP_IMAGE, &clipimg_pack(&[], &bmp));
        let want = clip_bmp_fix(&bmp);
        t.check(ok && !clip_has(CLIP_PNG) && clip_get(CLIP_IMAGE) == want && want.is_some(),
                "clip: CLIP_IMAGE without PNG comes back from CF_DIBV5 as the BMP");
        clip_set(CLIP_TEXT, saved.as_deref().unwrap_or(b""));
    }

    pub fn t_window(t: &mut T) {
        evq_clear();
        let drops = "C:/plattest/a.png|C:/plattest/Bild ä.bmp";
        setenv("jfgui_TEST_DROPFILES", Some(drops));
        let w = win_create("plattest window", 320, 200, PWF_RESIZABLE, PW_NOOWNER, PW_DEFAULT, PW_DEFAULT);
        setenv("jfgui_TEST_DROPFILES", None);
        t.check(w == 0, "window: win_create (main, library frame) -> index 0");
        if w != 0 {
            return;
        }
        let dpi = win_dpi(0);
        t.info(&format!("  (dpi {dpi})\n"));
        let (cw, ch) = win_size(0);
        t.check((cw, ch) == (dip(320, dpi), dip(200, dpi)), "window: client size = 320 x 200 DIP at the window's dpi");
        t.check(win_test(WT_VISIBLE, 0, [0; 4]) == 0, "window: created hidden");
        win_show(0, PWS_SHOW);
        let ev = pump(300);
        t.check(win_test(WT_VISIBLE, 0, [0; 4]) == 1, "window: win_show shows it");
        t.check(ev.iter().any(|e| e.typ == E_PAINT && e.win == 0), "window: WM_PAINT -> E_PAINT");
        let dr = ev.iter().find(|e| e.typ == E_DROP);
        t.check(dr.is_some_and(|e| (e.x, e.y, e.data) == (100, 100, 2)), "window: jfgui_TEST_DROPFILES -> E_DROP 100, 100, 2 files");
        let paths = drop_take();
        t.check(paths == ["C:/plattest/a.png", "C:/plattest/Bild ä.bmp"], "window: drop_take gives the dropped paths");

        // ---- present
        let (ca, cb, cc) = (0x0033_6699u32, 0x00CC_2211, 0x0011_EE44);
        let mut fb = Fb::new(cw, ch, ca);
        win_present(0, &fb.frame(), None);
        t.check(win_test(WT_PIXEL, 0, [5, 5, 0, 0]) == ca as i64, "present: the whole frame");
        fb.fill(0, 0, cw, ch, cb);
        win_present(0, &fb.frame(), Some(&[[10, 10, 20, 20], [-5, 100, 10, 10]]));
        let (p_in, p_out, p_clip) = (win_test(WT_PIXEL, 0, [15, 15, 0, 0]), win_test(WT_PIXEL, 0, [5, 5, 0, 0]),
                                     win_test(WT_PIXEL, 0, [2, 102, 0, 0]));
        t.check(p_in == cb as i64 && p_out == ca as i64 && p_clip == cb as i64,
                "present: only the dirty rectangles (clipped to the frame)");
        fb.fill(0, 0, cw, ch, cc); // not presented
        evq_clear();
        win_test(WT_REDRAW, 0, [0; 4]);
        let ev = pump(50);
        let (p_in, p_out) = (win_test(WT_PIXEL, 0, [15, 15, 0, 0]), win_test(WT_PIXEL, 0, [5, 5, 0, 0]));
        t.check(p_in == cb as i64 && p_out == ca as i64, "present: WM_PAINT repaints the kept copy of the presented pixels");
        t.check(ev.iter().any(|e| e.typ == E_PAINT), "present: ... and pushes E_PAINT");

        // ---- size, position, dpi
        let r0 = win_get_rect(0).unwrap_or_default();
        win_set_rect(0, PW_KEEP, PW_KEEP, 400, 300);
        let ev = pump(50);
        t.check(win_size(0) == (400, 300), "size: win_set_rect sets the client size");
        t.check(!ev.iter().any(|e| e.typ == E_RESIZE || e.typ == E_WINPOS), "size: ... not reported back (own change)");
        let r1 = win_get_rect(0).unwrap_or_default();
        t.check(r1 == [r0[0], r0[1], 400, 300], "size: win_get_rect = client origin + size");
        win_test(WT_MOVE, 0, [r1[0] + 20, r1[1] + 10, 360, 240]);
        let ev = pump(50);
        t.check(ev.iter().any(|e| e.typ == E_WINPOS && (e.x, e.y) == (r1[0] + 20, r1[1] + 10)),
                "size: a move by the user -> E_WINPOS (client origin)");
        t.check(ev.iter().any(|e| e.typ == E_RESIZE && (e.x, e.y) == (360, 240)), "size: a resize by the user -> E_RESIZE");
        win_set_min_size(0, 300, 200);
        let r2 = win_get_rect(0).unwrap_or_default();
        win_test(WT_MOVE, 0, [r2[0], r2[1], 50, 50]);
        pump(30);
        let (mw, mh) = win_size(0);
        t.check(mw >= dip(300, dpi) && mh >= dip(200, dpi), "size: the minimum client size holds (WM_GETMINMAXINFO)");
        t.info(&format!("  (after a 50 x 50 request: {mw} x {mh})\n"));
        win_set_min_size(0, 0, 0);
        win_set_rect(0, PW_KEEP, PW_KEEP, 360, 240);
        pump(30);
        let nd = if dpi == 144 { 120 } else { 144 };
        win_test(WT_DPI, 0, [nd, 0, 0, 0]);
        let ev = pump(50);
        let id = ev.iter().position(|e| e.typ == E_DPI && e.data == nd as u32);
        let ir = ev.iter().position(|e| e.typ == E_RESIZE);
        t.check(id.is_some() && ir.is_some() && id < ir, "dpi: WM_DPICHANGED -> E_DPI, then E_RESIZE of the new rect");
        t.check(win_dpi(0) == nd, "dpi: win_dpi follows");
        win_test(WT_DPI, 0, [dpi, 0, 0, 0]);
        pump(30);
        win_set_rect(0, PW_KEEP, PW_KEEP, 360, 240);
        pump(30);
        // the modal size / move loop: the refresh callback redraws, E_WINPOS waits for its end
        set_refresh(Some(refresh));
        win_test_send(0, WM_ENTERSIZEMOVE, 0, 0);
        let o = win_get_rect(0).unwrap_or_default();
        win_test(WT_MOVE, 0, [o[0] + 5, o[1] + 5, 340, 230]);
        let ev = pump(30);
        t.check(REFRESHED.load(SeqCst) == 1 && ev.iter().any(|e| e.typ == E_RESIZE) && !ev.iter().any(|e| e.typ == E_WINPOS),
                "sizemove: E_RESIZE + the refresh callback inside the loop, no E_WINPOS yet");
        win_test_send(0, WM_EXITSIZEMOVE, 0, 0);
        let ev = pump(30);
        t.check(ev.iter().any(|e| e.typ == E_WINPOS && (e.x, e.y) == (o[0] + 5, o[1] + 5)), "sizemove: E_WINPOS at its end");
        set_refresh(None);
        win_set_rect(0, PW_KEEP, PW_KEEP, 360, 240);
        pump(30);
        // the app's activation (every window of it) -> WST_ACTIVE
        win_test_send(0, WM_ACTIVATEAPP, 1, 0);
        let on = win_state(0) & WST_ACTIVE;
        win_test_send(0, WM_ACTIVATEAPP, 0, 0);
        let off = win_state(0) & WST_ACTIVE;
        t.check(on != 0 && off == 0, "state: WM_ACTIVATEAPP -> WST_ACTIVE");

        // ---- the hit hook (library frame), input translation
        win_hit_hook(Some(hit_test));
        let o = win_get_rect(0).unwrap_or_default();
        let ht = |x: i32, y: i32| win_test_send(0, WM_NCHITTEST, 0, lp(o[0] + x, o[1] + y));
        let (h_drag, h_close, h_client) = (ht(100, 20), ht(100, 35), ht(100, 100));
        t.info(&format!("  (hit tests: {h_drag} {h_close} {h_client})\n"));
        t.check(h_drag == 2 && h_close == 20 && h_client == HTCLIENT, "hit: HB_DRAG -> HTCAPTION, HB_CLOSE -> HTCLOSE, else client");
        evq_clear();
        win_test_send(0, WM_MOUSEMOVE, 0, lp(10, 20));
        win_test_send(0, WM_LBUTTONDOWN, 1, lp(11, 21));
        win_test_send(0, WM_LBUTTONUP, 0, lp(12, 22));
        win_test_send(0, WM_MOUSEWHEEL, 240 << 16, lp(o[0] + 30, o[1] + 40));
        win_test_send(0, WM_KEYDOWN, 0x41, 1);
        win_test_send(0, WM_CHAR, 0xD83D, 1);
        win_test_send(0, WM_CHAR, 0xDE00, 1);
        let mut got = Vec::new();
        while let Some(e) = evq_pop() {
            got.push((e.typ, e.btn, e.win, e.x, e.y, e.data));
        }
        let want = [(E_MOVE, 0, 0, 10, 20, 0), (E_DOWN, BTN_LEFT, 0, 11, 21, 0), (E_UP, BTN_LEFT, 0, 12, 22, 0),
                    (E_WHEEL, 0, 0, 30, 40, 240), (E_KEY_DOWN, 0, 0, 0, 0, 0x41), (E_TEXT, 0, 0, 0, 0, 0x1F600)];
        t.check(got == want, "input: move, button, wheel (screen -> client), key, a surrogate pair -> events");
        if got != want {
            t.info(&format!("  (got {got:?})\n"));
        }

        // ---- cursors
        let mut cur = vec![0u8; 32 * 32 * 4];
        for (k, p) in cur.chunks_exact_mut(4).enumerate() {
            let (x, y) = ((k % 32) as i32 - 16, (k / 32) as i32 - 16);
            if x * x + y * y < 144 {
                p.copy_from_slice(&0xFFFF_0000u32.to_le_bytes());
            }
        }
        let id = cursor_create(&cur, 32, 32, 16, 16, dpi);
        t.check(id >= CUR_USER0 as i32, "cursor: a custom cursor");
        win_set_cursor(0, id as u32);
        let r = win_test_send(0, WM_SETCURSOR, 0, HTCLIENT | (WM_MOUSEMOVE as isize) << 16);
        t.check(r == 1 && win_test(WT_CURSOR_IS, 0, [id, 0, 0, 0]) == 1, "cursor: WM_SETCURSOR shows the window's custom cursor");
        win_set_cursor(0, CUR_IBEAM);
        win_test_send(0, WM_SETCURSOR, 0, HTCLIENT | (WM_MOUSEMOVE as isize) << 16);
        t.check(win_test(WT_CURSOR_IS, 0, [CUR_IBEAM as i32, 0, 0, 0]) == 1, "cursor: ... and a system shape");
        cursor_destroy(id);
        win_set_cursor(0, CUR_ARROW);

        // ---- popup
        let pd = 0x0055_AA77u32;
        let pf = Fb::new(50, 40, pd);
        popup(0, 0, Some([10, 10, 50, 40]), Some(&pf.frame()));
        let pr = win_test(WT_POPUP_RECT, 0, [0; 4]);
        let o = win_get_rect(0).unwrap_or_default();
        let want = (o[0] + 10) as u16 as i64 | ((o[1] + 10) as u16 as i64) << 16 | 50 << 32 | 40 << 48;
        t.check(pr == want, "popup: shown at the owner's client point, its size");
        t.check(win_test(WT_POPUP_PIXEL, 0, [25, 20, 0, 0]) == pd as i64, "popup: its pixels");
        evq_clear();
        win_test(WT_POPUP_SEND, 0, [WM_MOUSEMOVE as i32, 0, 5, 6]);
        win_test(WT_POPUP_SEND, 0, [WM_LBUTTONDOWN as i32, 1, 5, 6]);
        win_test(WT_POPUP_SEND, 0, [WM_LBUTTONUP as i32, 0, 5, 6]);
        let mut got = Vec::new();
        while let Some(e) = evq_pop() {
            got.push((e.typ, e.btn, e.win, e.x, e.y));
        }
        t.check(got == [(E_MOVE, 0, 0, 15, 16), (E_DOWN, BTN_LEFT, 0, 15, 16), (E_UP, BTN_LEFT, 0, 15, 16)],
                "popup: its input comes as events of the owner in owner coordinates");
        popup(0, 0, None, None);
        t.check(win_test(WT_POPUP_RECT, 0, [0; 4]) == -1, "popup: hidden");

        // ---- an owned tool window
        let w1 = win_create("plattest tool", 100, 80, PWF_TOOL | PWF_SYSFRAME, 0, PW_DEFAULT, PW_DEFAULT);
        t.check(w1 == 1, "tool: win_create (owned) -> index 1");
        let o = win_get_rect(0).unwrap_or_default();
        let t1 = win_get_rect(1).unwrap_or_default();
        let d = dip(32, win_dpi(1));
        t.check(t1[0] == o[0] + d && t1[1] == o[1] + d, "tool: default place = owner client origin + 32 DIP");
        win_show(1, PWS_SHOWNA);
        pump(50);
        t.check(win_test(WT_ACTIVE, 1, [0; 4]) == 0, "tool: PWS_SHOWNA does not activate it");
        let ma = win_test_send(1, WM_MOUSEACTIVATE, 0, HTCLIENT);
        t.check(ma == 1, "tool: a click into a text box (HB_TEXT) activates it (MA_ACTIVATE)");
        win_hit_hook(None);
        let ma = win_test_send(1, WM_MOUSEACTIVATE, 0, HTCLIENT);
        t.check(ma == 3, "tool: other clicks do not (MA_NOACTIVATE)");
        evq_clear();
        win_test(WT_MOVE, 0, [o[0] + 30, o[1], o[2], o[3]]);
        let ev = pump(50);
        let t2 = win_get_rect(1).unwrap_or_default();
        t.check(t2[0] == t1[0] + 30 && t2[1] == t1[1], "tool: docked to the owner, it follows a move");
        t.check(ev.iter().any(|e| e.typ == E_WINPOS && e.win == 1), "tool: ... reported as E_WINPOS of window 1");
        win_enable(0, false);
        let off = win_test(WT_ENABLED, 0, [0; 4]);
        win_enable(0, true);
        t.check(off == 0 && win_test(WT_ENABLED, 0, [0; 4]) == 1, "tool: win_enable disables / enables the owner");
        win_destroy(1);
        t.check(win_get_rect(1).is_none(), "tool: destroyed");

        // ---- clipboard (the user's text is put back at the end)
        match win_test(WT_CLIP_ACCESS, 0, [0; 4]) {
            0 => clip_round_trip(t),
            e => t.info(&format!("  (clip: the clipboard cannot be opened here (error {e}) -- round trip skipped)\n")),
        }

        // ---- IME caret
        ime_pos(30, 40, 16);
        let c = win_test(WT_CARET, 0, [0; 4]);
        ime_pos(0, 0, 0);
        t.check(c == 30 | 40 << 32, "ime: the caret at the text position");

        // ---- the system file dialog
        let (f0, _) = file_dialog_filter(0, &["png=PNG-Bild", "jpg;jpeg=JPEG-Bild", "bmp"]);
        let want = "Alle unterstützten Dateien (*.png;*.jpg;*.jpeg;*.bmp)\0*.png;*.jpg;*.jpeg;*.bmp\0PNG-Bild (*.png)\0\
                    *.png\0JPEG-Bild (*.jpg;*.jpeg)\0*.jpg;*.jpeg\0BMP-Datei (*.bmp)\0*.bmp\0Alle Dateien (*.*)\0*.*\0\0\0";
        t.check(f0.iter().copied().eq(want.encode_utf16()), "filedlg: the open filter list");
        let (fs, de) = file_dialog_filter(FD_SAVE | 1 << FD_EXT_SHIFT, &["png=PNG-Bild", "jpg;jpeg=JPEG-Bild"]);
        let want = "PNG-Bild (*.png)\0*.png\0JPEG-Bild (*.jpg;*.jpeg)\0*.jpg;*.jpeg\0\0\0";
        t.check(fs.iter().copied().eq(want.encode_utf16()) && de == [b'j' as u16, b'p' as u16, b'g' as u16, 0],
                "filedlg: the save filter list, the default extension of the selected entry");
        let mut path = String::from("build/plattest_fd.png");
        file_dialog_autoclose(300, None);
        let r = file_dialog(FD_SAVE, &mut path, &["png=PNG-Bild", "jpg;jpeg=JPEG-Bild"]);
        t.check(r.0 == 0, "filedlg: the system Save dialog, cancelled");
        let full = format!("{}\\build\\plattest_fd2.jpg", cwd());
        file_dialog_autoclose(300, Some(&full));
        let r = file_dialog(FD_SAVE | 1 << FD_EXT_SHIFT, &mut path, &["png=PNG-Bild", "jpg;jpeg=JPEG-Bild"]);
        t.info(&format!("  (file dialog: {} {} {path})\n", r.0, r.1));
        t.check(r == (1, 1) && path == full.replace('\\', "/"), "filedlg: a name + OK -> the path ('/'), the type");

        // ---- process level: theme, home, drives, pens, the shell (dry run)
        let th = theme();
        let home = home().unwrap_or_default();
        let dr = drives();
        t.info(&format!("  (theme {th}, home {home}, drives {}, pens {})\n", dr.join(" "), pen_devices()));
        t.check(th == 0 || th == 1, "system: theme() reads the setting (0 light / 1 dark)");
        t.check(!home.is_empty() && file_exists(&home) == 2, "system: home() is a folder");
        t.check(dr.iter().any(|d| d == "C:/"), "system: drives() lists C:/");
        setenv("jfgui_OPEN_DRYRUN", Some("1"));
        let u = open_url("https://example.com/");
        let f = open_folder(&cwd());
        let g = open_folder("C:/no such folder/x.png");
        setenv("jfgui_OPEN_DRYRUN", None);
        t.check(u == 0 && f == 0 && g == -1, "shell: open_url / open_folder (dry run), a missing path -> -1");

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
}
