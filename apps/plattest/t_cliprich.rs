// t_cliprich -- bounded multi-format/CF_HTML framing regression checks, no OS clipboard writes.
// New opt-in clipboard_platform coverage, no assembly counterpart.
use crate::T;
use plat::*;

pub fn t_cliprich(t: &mut T) {
    for fragment in [
        "",
        "<b>Text</b>",
        "<p>Grüße αβ 😀</p>",
        "<div>\r\nA\nB</div>",
    ] {
        let encoded = clip_html_encode(fragment.as_bytes()).unwrap();
        t.check(
            clip_html_decode(&encoded).as_deref() == Some(fragment.as_bytes()),
            "CF_HTML: UTF-8 byte-offset round trip",
        );
        t.check(encoded.last() == Some(&0), "CF_HTML: trailing Windows NUL");
    }
    t.check(
        clip_html_encode(&[0xff]).is_none(),
        "CF_HTML: malformed UTF-8 rejected",
    );
    t.check(
        clip_html_encode(b"<b>\0</b>").is_none(),
        "CF_HTML: embedded NUL rejected",
    );
    for invalid in [
        b"StartFragment:0\nEndFragment:1\nx".as_slice(),
        b"StartFragment:999999999999999999999999\nEndFragment:99\nx",
        b"StartFragment:30\nEndFragment:20\nx",
        b"StartFragment:-1\nEndFragment:99\nx",
        b"StartFragment:40x\nEndFragment:99\nx",
        b"StartFragment:99\nEndFragment:999\nx",
        b"<b>Missing CF_HTML envelope</b>",
        b"StartFragment:50\nStartFragment:51\nEndFragment:52\nx",
    ] {
        t.check(
            clip_html_decode(invalid).is_none(),
            "CF_HTML: invalid offsets/header rejected",
        );
    }
    let mut encoded = clip_html_encode(b"<b>ok</b>").unwrap();
    let offset = encoded.windows(2).position(|w| w == b"ok").unwrap();
    encoded[offset] = 0xff;
    t.check(
        clip_html_decode(&encoded).is_none(),
        "CF_HTML: invalid fragment UTF-8 rejected",
    );
    encoded[offset] = 0;
    t.check(
        clip_html_decode(&encoded).is_none(),
        "CF_HTML: invalid fragment NUL rejected",
    );
    t.check(
        clip_many_valid(&[
            (CLIP_TEXT, b"text"),
            (CLIP_HTML, b"<b>text</b>"),
            (CLIP_RTF, b"{\\rtf1 text}"),
            (CLIP_DOCX, &[0, 255]),
        ]),
        "clipboard: four simultaneous representations accepted",
    );
    t.check(
        !clip_many_valid(&[]),
        "clipboard: empty format list rejected",
    );
    t.check(
        !clip_many_valid(&[(CLIP_TEXT, b"a"), (CLIP_TEXT, b"b")]),
        "clipboard: duplicate formats rejected",
    );
    t.check(
        !clip_many_valid(&[(CLIP_PNG, b"png")]),
        "clipboard: image legacy API kept separate",
    );
    t.check(
        !clip_many_valid(&[(CLIP_HTML, &[255])]),
        "clipboard: malformed HTML UTF-8 rejected",
    );
    t.check(
        !clip_many_valid(&[(CLIP_TEXT, b"a\0b")]),
        "clipboard: text NUL rejected",
    );
    t.check(
        clip_many_valid(&[(CLIP_DOCX, &[0, 255, 0])]),
        "clipboard: private binary bytes retained",
    );
}

pub fn t_owner(t: &mut T) {
    transport(t, true);
}

pub fn t_reader(t: &mut T) {
    transport(t, false);
}

// Real X11 cross-process tests require an isolated selection, never the user's CLIPBOARD.
fn transport(t: &mut T, owner: bool) {
    if !cfg!(target_os = "linux")
        || env("jfgui_BACKEND").as_deref() != Some("x11")
        || !env("jfgui_X11SELECTION").is_some_and(|s| s.starts_with("jfgui_RICH_TEST_"))
    {
        t.check(
            false,
            "transport: requires X11 and jfgui_X11SELECTION=jfgui_RICH_TEST_<id>",
        );
        return;
    }
    let win = win_create(
        "Rich clipboard private test",
        200,
        100,
        PWF_NOACTIVATE,
        PW_NOOWNER,
        PW_DEFAULT,
        PW_DEFAULT,
    );
    t.check(win >= 0, "transport: hidden test window created");
    if win < 0 {
        return;
    }
    let binary: alloc::vec::Vec<u8> = (0..180000).map(|i| ((i * 17) & 255) as u8).collect();
    let entries = [
        (CLIP_TEXT, "Grüße α 😀\nZ".as_bytes()),
        (CLIP_HTML, "<b>Grüße α 😀</b>".as_bytes()),
        (CLIP_RTF, b"{\\rtf1\\ansi a\\b b\\b0}".as_slice()),
        (CLIP_DOCX, binary.as_slice()),
    ];
    if owner {
        t.check(
            clip_set_many(&entries),
            "transport: four formats published together",
        );
    }
    for (fmt, data) in entries {
        t.check(clip_has(fmt), "transport: rich format advertised");
        t.check(
            clip_get(fmt).as_deref() == Some(data),
            "transport: exact representation bytes received",
        );
    }
    t.check(
        !clip_has(CLIP_PNG) && !clip_has(CLIP_IMAGE),
        "transport: no false image offer",
    );
    if owner {
        if let Some(path) = arg_value("--ready") {
            write_file(&path, b"ready");
        }
        let deadline = time_us() + 10_000_000;
        while time_us() < deadline {
            wait(deadline);
            while evq_pop().is_some() {}
        }
    }
    win_destroy(win);
}
