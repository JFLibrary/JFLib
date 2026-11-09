// t_clip -- the system's clipboard for real: what is set comes back (not in "all": it replaces what
// the user copied, so it is for test devices -- the iOS simulator, a test Mac).
//
//   plattest --test clip      text (UTF-8 with umlauts and an emoji), text with HTML as one selection,
//                             a PNG (CLIP_PNG; CLIP_IMAGE sees it), clip_has for each; without a
//                             clipboard (a headless Linux, the browser's worker): only that every
//                             set answers false.

use alloc::format;

use plat::*;

use crate::T;

// a 1 x 1 PNG (one white pixel)
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
    0xD7, 0x63, 0xF8, 0xFF, 0xFF, 0x3F, 0x00, 0x05, 0xFE, 0x02, 0xFE, 0xDC, 0xCC, 0x59, 0xE7, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

pub fn t_clip(t: &mut T) {
    let text = "Grüße aus dem Test 😀";
    if !clip_set(CLIP_TEXT, text.as_bytes()) {
        t.info("  no clipboard here: nothing more to check\n");
        t.check(!clip_set_many(&[(CLIP_TEXT, b"x")]), "clip: no clipboard, set_many answers false too");
        return;
    }
    t.check(clip_has(CLIP_TEXT), "clip: text is offered");
    let got = clip_get(CLIP_TEXT);
    t.check(got.as_deref() == Some(text.as_bytes()), &format!("clip: the text comes back ({:?})", got.as_deref().map(alloc::string::String::from_utf8_lossy)));

    let html = "<p>Grüße <b>fett</b></p>";
    t.check(clip_set_many(&[(CLIP_TEXT, b"Gruesse fett"), (CLIP_HTML, html.as_bytes())]), "clip: text and HTML as one selection");
    t.check(clip_has(CLIP_TEXT) && clip_has(CLIP_HTML), "clip: both are offered");
    t.check(clip_get(CLIP_TEXT).as_deref() == Some(&b"Gruesse fett"[..]), "clip: its text comes back");
    t.check(clip_get(CLIP_HTML).as_deref() == Some(html.as_bytes()), "clip: its HTML comes back");

    t.check(clip_set(CLIP_PNG, PNG), "clip: a PNG");
    t.check(clip_has(CLIP_IMAGE), "clip: an image is offered");
    let png = clip_get(CLIP_PNG);
    t.check(png.as_deref() == Some(PNG), &format!("clip: the PNG comes back ({} bytes)", png.map_or(0, |p| p.len())));
    let img = clip_get(CLIP_IMAGE);
    t.check(img.as_deref().is_some_and(|i| i.starts_with(b"\x89PNG")), "clip: CLIP_IMAGE is a PNG");
    t.check(!clip_has(CLIP_TEXT) || clip_get(CLIP_TEXT).is_none_or(|v| v.is_empty()), "clip: the text went with the new selection");
}
