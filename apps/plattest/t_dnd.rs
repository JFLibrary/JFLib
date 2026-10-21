// t_dnd -- drag and drop between programs (lib/plat/dnd.rs, win_dnd.rs) without a user
//
//   plattest --test dnd     the shared part: effects (modifier keys, files never moved), the
//                           target hook, URL lists, text decoding, the pending drop; Windows:
//                           our IDataObject read as a drop target reads it (EnumFormatEtc,
//                           GetData of every format), our IDropTarget fed with it (DragEnter,
//                           DragOver, Drop -> E_DRAG / E_DROP and the content), the
//                           IDropSource's answers, a real window registered with OLE
//   plattest --test dnd-x11 (Linux, X11; tools/platshared/dnd_x11.py drives it with a raw X11
//                           client and synthetic X events) XDND both ways: a drop of a
//                           URI list, UTF-8 text, HTML and a 300 KB image (INCR) onto window
//                           0, a leave; our drag (drag_start) dropped onto the client's window,
//                           and one cancelled with Esc
//   (Wayland, DoDragDrop with a real mouse: by hand, docs/GUIDE.md)

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use crate::T;
use plat::*;

// the hook of the tests: everything is taken on the left half (x < 100), nothing on the right
fn hook_all(_pw: i32, x: i32, _y: i32, types: u32) -> (u32, u32) {
    if x < 100 { (types, DE_ALL) } else { (0, 0) }
}

// a 2 x 1 PNG (from the encoder of the image crates)
fn tiny_png() -> Vec<u8> {
    img_png::encode(&[0, 0, 255, 255, 255, 0, 0, 255], 2, 1, 8, false)
}

pub fn t_dnd(t: &mut T) {
    shared(t);
    #[cfg(windows)]
    ole(t);
    drag_hook(None);
}

fn shared(t: &mut T) {
    // effects: Ctrl copy, Shift move, both link; else copy, move, link in that order
    t.expect_i(drag_effect_pick(DE_COPY | DE_MOVE, 0) as i64, DE_COPY as i64, "dnd: plain drag copies");
    t.expect_i(drag_effect_pick(DE_COPY | DE_MOVE, MOD_SHIFT) as i64, DE_MOVE as i64, "dnd: Shift moves");
    t.expect_i(drag_effect_pick(DE_ALL, MOD_CTRL | MOD_SHIFT) as i64, DE_LINK as i64, "dnd: Ctrl+Shift links");
    t.expect_i(drag_effect_pick(DE_MOVE, MOD_CTRL) as i64, DE_MOVE as i64, "dnd: Ctrl without copy: the allowed move");
    t.expect_i(drag_effect_pick(0, 0) as i64, 0, "dnd: nothing allowed, no effect");
    // the hook: without one only files (as WM_DROPFILES / Xdnd before)
    drag_hook(None);
    t.check(drag_query(0, 10, 10, DT_FILES | DT_TEXT, DE_COPY, 0) == (DT_FILES, DE_COPY), "dnd: no hook: files are taken");
    t.check(drag_query(0, 10, 10, DT_TEXT, DE_COPY, 0) == (0, 0), "dnd: no hook: text is refused");
    drag_hook(Some(hook_all));
    t.check(drag_query(0, 10, 10, DT_TEXT | DT_HTML, 0, 0) == (DT_TEXT | DT_HTML, DE_COPY), "dnd: the hook takes text and HTML");
    t.check(drag_query(0, 200, 10, DT_TEXT, 0, 0) == (0, 0), "dnd: the hook refuses at another point");
    t.check(drag_query(0, 10, 10, DT_TEXT, DE_MOVE | DE_COPY, MOD_SHIFT) == (DT_TEXT, DE_MOVE), "dnd: text may be moved");
    t.check(drag_query(0, 10, 10, DT_FILES, DE_MOVE | DE_COPY, MOD_SHIFT) == (DT_FILES, DE_COPY), "dnd: files are never moved");
    t.check(drag_query(0, 10, 10, DT_FILES, DE_MOVE, 0) == (0, 0), "dnd: files only to move: refused");
    // URL lists: comments, local files and blanks out; duplicates once
    drop_reset();
    let n = drop_url_list(b"# comment\r\nhttps://example.org/a\r\nfile:///tmp/x\nnot a url\nmailto:a@b.c\nhttps://example.org/a\n");
    t.expect_i(n as i64, 2, "dnd: URL list: two URLs");
    let n = drop_uri_list(b"file:///tmp/x%20y\nhttps://example.org/b\n");
    t.expect_i(n as i64, 1, "dnd: uri-list: one local file");
    drop_url_list(b"https://example.org/b\n");
    drop_set_text("Grüße\0rest".as_bytes());
    drop_set_html(b"<i>x</i>");
    drop_set_image(vec![1, 2, 3]);
    drop_set_effect(DE_MOVE);
    let d = drop_data_take();
    t.check(d.urls == ["https://example.org/a", "mailto:a@b.c", "https://example.org/b"], "dnd: the pending drop's URLs");
    t.check(d.files == ["/tmp/x y"], "dnd: the pending drop's path");
    t.check(d.text.as_deref() == Some("Grüße"), "dnd: the text ends at a NUL");
    t.check(d.html.as_deref() == Some("<i>x</i>") && d.image.as_deref() == Some(&[1u8, 2, 3][..]), "dnd: HTML and image");
    t.check(d.effects == DE_MOVE && d.types() == DT_FILES | DT_TEXT | DT_URL | DT_HTML | DT_IMAGE, "dnd: effect and kinds");
    t.check(drop_data_take() == DragData::default(), "dnd: taken: the pending drop is empty again");
    drop_set_text(b"x");
    drop_reset();
    t.check(drop_data_take().text.is_none(), "dnd: drop_reset empties the text too");
    // other programs' text
    let w: Vec<u8> = "a€\u{1F600}".encode_utf16().flat_map(|u| u.to_le_bytes()).chain([0, 0, 0x41, 0]).collect();
    t.check(drop_text_from(&w, true) == "a€\u{1F600}".as_bytes(), "dnd: UTF-16 up to its NUL");
    t.check(drop_text_from(b"abc\0def", false) == b"abc", "dnd: bytes up to a NUL");
    // the content's kinds and the app's own kind
    let mut a = DragData::of_app(DT_USER0 << 2, b"7");
    t.check(a.types() == DT_USER0 << 2 && a.allowed() == DE_COPY | DE_MOVE, "dnd: the app's own kind, default effects");
    a.text = Some(String::from("t"));
    t.check(a.types() & DT_STD == DT_TEXT, "dnd: with text: one standard kind");
    t.check(!drag_start(DragData::of_app(DT_USER0, b"x")), "dnd: only the app's own kind does not leave the app");
}

#[cfg(windows)]
fn ole(t: &mut T) {
    use plat::{E_DRAG, E_DROP};
    const MK_LBUTTON: u32 = 1;
    const MK_SHIFT: u32 = 4;
    let png = tiny_png();
    let bmp = {
        // a 1 x 1 BMP: file header + BITMAPV5HEADER + one pixel
        let mut b = vec![0u8; 14 + 124 + 4];
        b[0] = b'B';
        b[1] = b'M';
        b[2..6].copy_from_slice(&(142u32).to_le_bytes());
        b[10..14].copy_from_slice(&(138u32).to_le_bytes());
        b[14..18].copy_from_slice(&124u32.to_le_bytes());
        b[18..22].copy_from_slice(&1u32.to_le_bytes());
        b[22..26].copy_from_slice(&1u32.to_le_bytes());
        b[26..28].copy_from_slice(&1u16.to_le_bytes());
        b[28..30].copy_from_slice(&32u16.to_le_bytes());
        b[30..34].copy_from_slice(&3u32.to_le_bytes()); // BI_BITFIELDS
        b[54..58].copy_from_slice(&0x00FF_0000u32.to_le_bytes());
        b[58..62].copy_from_slice(&0x0000_FF00u32.to_le_bytes());
        b[62..66].copy_from_slice(&0x0000_00FFu32.to_le_bytes());
        b[66..70].copy_from_slice(&0xFF00_0000u32.to_le_bytes());
        b[138..142].copy_from_slice(&[1, 2, 3, 255]);
        b
    };
    let data = DragData {
        files: vec![String::from("C:\\jf\\a.txt"), String::from("rel/b.txt")],
        text: Some(String::from("Hallo\nWelt")),
        urls: vec![String::from("https://example.org/")],
        html: Some(String::from("<b>fett</b>")),
        image: Some(png.clone()),
        bmp: Some(bmp.clone()),
        effects: DE_COPY | DE_MOVE,
        ..DragData::default()
    };
    // ---- our IDataObject as a drop target sees it
    let Some(f) = dnd_test_formats(&data) else {
        t.check(false, "dnd-ole: OLE is there (OleInitialize)");
        return;
    };
    let names: Vec<&str> = f.iter().map(|e| e.0.as_str()).collect();
    crate::out(&alloc::format!("  formats: {}\n", names.join(", ")));
    for want in ["CF_UNICODETEXT", "CF_HDROP", "HTML Format", "UniformResourceLocatorW", "UniformResourceLocator", "PNG", "CF_DIBV5", "CF_DIB"] {
        t.check(names.contains(&want), &alloc::format!("dnd-ole: EnumFormatEtc + GetData: {want}"));
    }
    let get = |n: &str| f.iter().find(|e| e.0 == n).map(|e| e.1.clone()).unwrap_or_default();
    let text: Vec<u8> = "Hallo\r\nWelt\0".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    t.check(get("CF_UNICODETEXT") == text, "dnd-ole: CF_UNICODETEXT is UTF-16 with CR LF and a NUL");
    let hd = get("CF_HDROP");
    let cwd: String = plat::cwd().replace('/', "\\");
    let paths = alloc::format!("C:\\jf\\a.txt\0{}\\rel\\b.txt\0\0", cwd.trim_end_matches('\\'));
    let wpaths: Vec<u8> = paths.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    t.check(hd.len() >= 20 && hd[0..4] == 20u32.to_le_bytes() && hd[16..20] == 1u32.to_le_bytes() && hd[20..] == wpaths[..],
            "dnd-ole: CF_HDROP: DROPFILES, wide absolute paths, double NUL");
    t.check(clip_html_decode(&get("HTML Format")).as_deref() == Some(&b"<b>fett</b>"[..]), "dnd-ole: HTML Format is CF_HTML");
    let url: Vec<u8> = "https://example.org/\0".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    t.check(get("UniformResourceLocatorW") == url && get("UniformResourceLocator") == b"https://example.org/\0", "dnd-ole: the URL (wide, ANSI)");
    t.check(get("PNG") == png, "dnd-ole: PNG as it is");
    t.check(get("CF_DIBV5") == bmp[14..] && get("CF_DIB").len() == 40 + 4, "dnd-ole: CF_DIBV5 / CF_DIB of the BMP");
    // ---- our IDropTarget fed with it (no window: client = screen point)
    drag_hook(Some(hook_all));
    evq_clear();
    let eff = dnd_test_drop(&data, 0, 20, 30, MK_LBUTTON);
    t.check(eff == Some(DE_COPY), "dnd-ole: Drop answers copy");
    let mut evs = Vec::new();
    while let Some(e) = evq_pop() {
        evs.push(e);
    }
    let drags = evs.iter().filter(|e| e.typ == E_DRAG).count();
    let drop = evs.iter().find(|e| e.typ == E_DROP);
    t.check(drags == 1 && evs.first().is_some_and(|e| e.typ == E_DRAG && e.x == 20 && e.y == 30
            && e.data == DT_FILES | DT_TEXT | DT_URL | DT_HTML | DT_IMAGE | (DE_COPY | DE_MOVE) << 8),
            "dnd-ole: DragEnter: E_DRAG with the kinds and effects (DragOver at the same point: none)");
    t.check(drop.is_some_and(|e| e.x == 20 && e.y == 30 && e.data == 2), "dnd-ole: Drop: E_DROP at the point, two paths");
    let got = drop_data_take();
    t.check(got.files == ["C:\\jf\\a.txt", &alloc::format!("{}\\rel\\b.txt", cwd.trim_end_matches('\\'))], "dnd-ole: the paths");
    t.check(got.text.as_deref() == Some("Hallo\nWelt"), "dnd-ole: the text (CR LF -> LF)");
    t.check(got.html.as_deref() == Some("<b>fett</b>") && got.urls == ["https://example.org/"], "dnd-ole: HTML and URL");
    t.check(got.image.as_deref() == Some(&png[..]) && got.effects == DE_COPY, "dnd-ole: the PNG, the effect");
    // Shift: text may be moved, files are not
    let txt = DragData { text: Some(String::from("weg")), effects: DE_COPY | DE_MOVE, ..DragData::default() };
    t.check(dnd_test_drop(&txt, 0, 20, 30, MK_LBUTTON | MK_SHIFT) == Some(DE_MOVE), "dnd-ole: Shift moves text");
    let files = DragData { files: vec![String::from("C:\\jf\\c.txt")], effects: DE_COPY | DE_MOVE, ..DragData::default() };
    t.check(dnd_test_drop(&files, 0, 20, 30, MK_LBUTTON | MK_SHIFT) == Some(DE_COPY), "dnd-ole: Shift does not move files");
    // a DIB without PNG arrives as a BMP; a refused point: no drop, E_DRAG leave
    let dib = DragData { bmp: Some(bmp.clone()), image: None, text: Some(String::from("i")), ..DragData::default() };
    let mut dib_only = dib.clone();
    dib_only.text = None;
    evq_clear();
    drop_data_take();
    let _ = dnd_test_drop(&dib, 0, 20, 30, MK_LBUTTON);
    let got = drop_data_take();
    t.check(got.image.as_deref().is_some_and(|b| b.starts_with(b"BM")) && got.text.as_deref() == Some("i"), "dnd-ole: CF_DIBV5 arrives as a BMP file");
    evq_clear();
    t.check(dnd_test_drop(&txt, 0, 300, 30, MK_LBUTTON) == Some(0), "dnd-ole: refused at the point: no effect");
    let mut last = None;
    while let Some(e) = evq_pop() {
        if e.typ == E_DROP {
            last = Some(E_DROP);
        } else if e.typ == E_DRAG {
            last = Some(e.data as u8);
        }
    }
    t.check(last == Some(0), "dnd-ole: refused: the last event is E_DRAG leave, no E_DROP");
    let _ = dib_only;
    // ---- the drop source
    t.expect_i(dnd_test_source(false, MK_LBUTTON) as i64, 0, "dnd-ole: QueryContinueDrag: held -> go on");
    t.expect_i(dnd_test_source(false, 0) as i64, 1, "dnd-ole: QueryContinueDrag: released -> drop");
    t.expect_i(dnd_test_source(true, MK_LBUTTON) as i64, 2, "dnd-ole: QueryContinueDrag: Esc -> cancel");
    // ---- a real (hidden) window: its client point survives the screen round trip
    let w = plat::win_create("plattest dnd", 200, 120, 0, PW_NOOWNER, PW_DEFAULT, PW_DEFAULT);
    t.check(w == 0, "dnd-ole: a window (registered with RegisterDragDrop)");
    evq_clear();
    let _ = dnd_test_drop(&txt, 0, 33, 44, MK_LBUTTON);
    let mut at = None;
    while let Some(e) = evq_pop() {
        if e.typ == E_DROP {
            at = Some((e.x, e.y, e.win));
        }
    }
    t.check(at == Some((33, 44, 0)), "dnd-ole: E_DROP in client px of window 0");
    drop_data_take();
    plat::win_destroy(0);
}

#[cfg(windows)]
pub fn t_dnd_x11(_: &mut T) {
    plat::write_stderr(b"plattest: --test dnd-x11 is Linux only\n");
    plat::exit(2);
}

// the image of the X11 drop: 300000 bytes (more than one property: INCR)
#[cfg(target_os = "linux")]
fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 7 + i / 251) as u8).collect()
}

// pump the OS until f(event) holds (at most 10 s) -> it did
#[cfg(target_os = "linux")]
fn pump_until(mut f: impl FnMut(&plat::Event) -> bool) -> bool {
    let dl = plat::time_us() + 10_000_000;
    loop {
        plat::wait(dl.min(plat::time_us() + 50_000));
        while let Some(e) = evq_pop() {
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
pub fn t_dnd_x11(t: &mut T) {
    use plat::{E_DOWN, E_DRAG, E_DRAG_END, E_DROP};
    let w = plat::win_create("plattest dnd", 200, 120, 0, PW_NOOWNER, PW_DEFAULT, PW_DEFAULT);
    t.check(w == 0 && plat::disp_backend() == plat::BK_X11, "dnd-x11: an X11 window (never shown)");
    drag_hook(Some(hook_all));
    crate::out("  dnd-x11: window\n");
    // ---- another program's drag over the window and its drop
    let mut over = Vec::new();
    let mut dropped = None;
    let got = pump_until(|e| {
        if e.typ == E_DRAG {
            over.push((e.x, e.y, e.data));
        }
        if e.typ == E_DROP {
            dropped = Some((e.x, e.y, e.data));
        }
        dropped.is_some()
    });
    t.check(got, "dnd-x11: E_DROP");
    let kinds = DT_FILES | DT_TEXT | DT_URL | DT_HTML | DT_IMAGE;
    t.check(over.first() == Some(&(20, 30, kinds | DE_ALL << 8)), "dnd-x11: E_DRAG at the first position with the kinds of XdndTypeList");
    t.check(over.iter().any(|o| o.0 == 200), "dnd-x11: E_DRAG at the refused position too");
    t.check(dropped == Some((20, 30, 1)), "dnd-x11: E_DROP at the position, one path");
    let d = drop_data_take();
    t.check(d.files == ["/tmp/jf a.txt"] && d.urls == ["https://example.org/x"], "dnd-x11: the path and the URL of the text/uri-list");
    t.check(d.text.as_deref() == Some("Grüße aus X11"), "dnd-x11: the UTF-8 text");
    t.check(d.html.as_deref() == Some("<b>fett</b>"), "dnd-x11: the HTML");
    t.check(d.image.as_deref() == Some(&pattern(300000)[..]), "dnd-x11: the 300 KB image (INCR)");
    t.check(d.effects == DE_COPY, "dnd-x11: copied");
    // a drag that leaves again
    let mut left = false;
    t.check(pump_until(|e| {
        left = e.typ == E_DRAG && e.data == 0;
        left
    }), "dnd-x11: XdndLeave -> E_DRAG with data 0");
    crate::out("  dnd-x11: ready to drag\n");
    // ---- our drag onto the client's window
    let data = DragData {
        files: vec![String::from("/tmp/jf b.txt")],
        text: Some(String::from("jfgui-Text")),
        html: Some(String::from("<i>kursiv</i>")),
        image: Some(tiny_png()),
        ..DragData::default()
    };
    t.check(pump_until(|e| e.typ == E_DOWN), "dnd-x11: the left button is pressed (synthetic)");
    t.check(drag_start(data.clone()), "dnd-x11: drag_start with the button held");
    t.check(!drag_start(data.clone()), "dnd-x11: a second drag_start while it runs: refused");
    crate::out("  dnd-x11: dragging\n");
    let mut end = None;
    t.check(pump_until(|e| {
        if e.typ == E_DRAG_END {
            end = Some(e.data);
        }
        end.is_some()
    }), "dnd-x11: E_DRAG_END");
    t.check(end == Some(DE_COPY), "dnd-x11: the target copied it (XdndFinished)");
    t.check(!drag_running(), "dnd-x11: no drag runs any more");
    // ---- one more, cancelled with Esc
    crate::out("  dnd-x11: ready to drag again\n");
    t.check(pump_until(|e| e.typ == E_DOWN), "dnd-x11: pressed again");
    t.check(drag_start(data), "dnd-x11: drag_start again");
    crate::out("  dnd-x11: dragging again\n");
    let mut end = None;
    let mut esc_seen = false;
    t.check(pump_until(|e| {
        if e.typ == plat::E_KEY_DOWN && e.data & 0xFFFF == 0x1B {
            esc_seen = true;
        }
        if e.typ == E_DRAG_END {
            end = Some(e.data);
        }
        end.is_some()
    }), "dnd-x11: E_DRAG_END after Esc");
    t.check(end == Some(0) && !esc_seen, "dnd-x11: cancelled, the Esc key did not reach the app");
    drag_hook(None);
    plat::win_destroy(0);
    plat::sleep_ms(200);
    crate::out("dnd-x11: done\n");
}
