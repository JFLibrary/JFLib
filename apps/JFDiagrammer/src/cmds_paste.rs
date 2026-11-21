// cmds_paste -- (JD8 rest) Start › Zwischenablage › Inhalte einfügen and Als Hyperlink einfügen
// (RS-01), and what Einfügen (Strg+V) takes from the system's clipboard
//
//   PasteState                   the dialog Inhalte einfügen, what the system's clipboard offers
//                                (asked every pass in scripts, every 250 ms in a window: the
//                                commands' states need it), where the app's own clipboard came
//                                from (Als Hyperlink einfügen links to it)
//   poll(app, ui)                every pass before the commands' states
//   available(app, id)           PASTE_SPECIAL: something to paste; PASTE_LINK: text on the
//                                clipboard or shapes copied from this drawing
//   system_has(app)              Einfügen may take the system's text or picture
//   copied(app, shapes)          edit_cmds::copy: the source of the app's clipboard
//   paste_system(app, ui)        Einfügen with an empty app clipboard: the system's text as a
//                                text box, else its picture as a picture shape
//   special(app, ui)             PASTE_SPECIAL: the dialog with the formats there are now:
//                                Shapes (JFDiagrammer), unformatted text (a text box), a picture
//   link(app, ui)                PASTE_LINK: a URL or path on the clipboard -> a text box with
//                                that hyperlink (Address); shapes copied from this drawing -> a
//                                text box that links to the shape (SubAddress "Seite/Shape", one
//                                shape; its text "Server (Seite-1)") or to its page (several)
//
// Einfügen keeps JD7's rule: the app's own clipboard first (shapes copied inside the app).  Only
// when it is empty, Einfügen takes the system's text or picture, through the same code as
// Inhalte einfügen: that is a few lines, and it is what a user expects of Strg+V after copying
// text or a picture in another program (Visio pastes them too).  Text first: programs that put
// text on the clipboard often add a picture of it, and a text box is what the text meant.  The
// app's shapes are not on the system's clipboard (no own clipboard format yet, JD12 notes), so a
// copy in another program does not replace them; Inhalte einfügen offers every format there is.
//
// Visio's cells: a hyperlink is a row of the Hyperlink section named Row_1 with Address (URL or
// file), SubAddress (a page "Seite-1" or a shape "Seite-1/Sheet.5") and Description
// (Transaction::set_hyperlink).

use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{ops, Owner, PageRef, ShapeRef, SheetRef, Text};
use ui::i18n::t;
use ui::*;

use crate::edit_canvas::{key_of, shape_label};
use crate::edit_doc;
use crate::Diagrammer;

const ID_DLG: u32 = 0x8200;
/// the formats of Inhalte einfügen
const F_SHAPES: i32 = 0;
const F_TEXT: i32 = 1;
const F_IMAGE: i32 = 2;
/// the longest text pasted (bytes; more is cut at a character)
const TEXT_MAX: usize = 64 * 1024;

/// the paste commands' state
pub(crate) struct PasteState {
    pub dlg: Shared<i32>,
    /// the format chosen in the dialog
    pub choice: i32,
    /// the system's clipboard as last asked: text, a picture
    pub sys_text: bool,
    pub sys_image: bool,
    polled: Option<u32>,
    /// the app's clipboard came from: the drawing (serial), its page, the shapes
    pub src: Option<(u32, PageRef, Vec<ShapeRef>)>,
}

impl Default for PasteState {
    fn default() -> PasteState {
        PasteState { dlg: Shared::new(0), choice: 0, sys_text: false, sys_image: false, polled: None, src: None }
    }
}

/// every pass: what the system's clipboard offers (cheap; throttled in a window)
pub(crate) fn poll(app: &mut Diagrammer, ui: &mut Ui) {
    let now = ui.now();
    let st = &mut app.paste;
    if !ui.headless() && st.polled.is_some_and(|p| now.wrapping_sub(p) < 250) {
        return;
    }
    st.polled = Some(now);
    st.sys_text = ui.clip_has(CLIP_TEXT);
    st.sys_image = ui.clip_has(CLIP_IMAGE) || ui.clip_has(CLIP_PNG);
}

/// Einfügen may take the system's clipboard (the app's own is empty)
pub(crate) fn system_has(app: &Diagrammer) -> bool {
    app.paste.sys_text || app.paste.sys_image
}

// shapes of this drawing on the app's clipboard that still exist
fn linkable(app: &Diagrammer) -> Option<(PageRef, Vec<ShapeRef>)> {
    let o = app.opened.as_ref()?;
    let (serial, page, shapes) = app.paste.src.as_ref()?;
    if *serial != o.serial || app.clip.is_none() {
        return None;
    }
    let live: Vec<ShapeRef> = shapes.iter().copied().filter(|&s| o.doc.shape(s).is_some_and(|x| !x.is_deleted())).collect();
    (!live.is_empty()).then_some((*page, live))
}

/// whether a command of this module can run now
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    let open = app.opened.is_some();
    match id {
        crate::commands::PASTE_SPECIAL => open && (app.clip.is_some() || system_has(app)),
        crate::commands::PASTE_LINK => open && (app.paste.sys_text || linkable(app).is_some()),
        _ => true,
    }
}

/// edit_cmds::copy: where the app's clipboard came from
pub(crate) fn copied(app: &mut Diagrammer, shapes: &[ShapeRef]) {
    let serial = app.opened.as_ref().map(|o| o.serial);
    app.paste.src = match (serial, app.model.page) {
        (Some(s), Some(p)) if !shapes.is_empty() => Some((s, p, shapes.to_vec())),
        _ => None,
    };
}

// the system's text, cut to TEXT_MAX at a character, line ends as \n (None: none, or empty)
fn system_text(ui: &mut Ui) -> Option<String> {
    let b = ui.clip_get(CLIP_TEXT)?;
    let mut s = String::from_utf8_lossy(&b).into_owned();
    if s.len() > TEXT_MAX {
        let mut k = TEXT_MAX;
        while !s.is_char_boundary(k) {
            k -= 1;
        }
        s.truncate(k);
    }
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    let s = String::from(s.trim_end_matches(['\n', '\0']));
    (!s.trim().is_empty()).then_some(s)
}

// a text box with `text` (and a hyperlink: address, sub-address) in the middle of the view; one
// undo step, selected -> made
fn text_box(app: &mut Diagrammer, ui: &mut Ui, label: &str, text: &str, link: Option<(&str, &str)>) -> bool {
    let (Some(geo), Some(page)) = (app.model.geo, app.model.page) else { return false };
    let vp = app.canvas.vp;
    let r = vp.rect;
    let mid = geo.page(vp.to_world(r[0] as f32 + r[2] as f32 * 0.5, r[1] as f32 + r[3] as f32 * 0.5));
    // as wide as its longest line (about 0.08 in a character at the style's size), 1 to 6 in
    let longest = text.lines().map(|l| l.chars().count()).max().unwrap_or(1) as f64;
    let w = (longest * 0.08 + 0.2).clamp(1.0, 6.0) / geo.scale;
    let h = 0.25 / geo.scale;
    let (a, b) = ([mid[0] - w * 0.5, mid[1] - h * 0.5], [mid[0] + w * 0.5, mid[1] + h * 0.5]);
    let owner = Owner::Page(page);
    let text = String::from(text);
    let link = link.map(|(x, y)| (String::from(x), String::from(y)));
    let Some(o) = app.opened.as_mut() else { return false };
    let r = edit_doc::edit(o, label, true, |tx, eng| {
        let s = ops::new_shape(tx, eng, owner, ops::Basic::TextBox, a, b)?;
        ops::set_text(tx, eng, s, Text::plain(&text))?;
        if let Some((address, sub)) = &link {
            tx.set_hyperlink(SheetRef::Shape(s), "Row_1", address, sub, &text)?;
        }
        crate::structures::joined(tx, eng, owner, &[s])?;
        Ok(s)
    });
    match r {
        Ok(s) => {
            app.se.select(&[key_of(s)]);
            ui.repass();
            true
        }
        Err(_) => false,
    }
}

// the system's picture as a picture shape
fn paste_image(app: &mut Diagrammer, ui: &mut Ui) -> bool {
    // a picture file: PNG, or the system's bitmap as BMP (stored as PNG)
    let Some(bytes) = ui.clip_get(CLIP_IMAGE).or_else(|| ui.clip_get(CLIP_PNG)) else { return false };
    crate::cmds_picture::insert_bytes(app, ui, &bytes, None)
}

/// Einfügen with an empty app clipboard: the system's text, else its picture
pub(crate) fn paste_system(app: &mut Diagrammer, ui: &mut Ui) {
    if let Some(text) = system_text(ui) {
        text_box(app, ui, t("Einfügen"), &text, None);
    } else if app.paste.sys_image {
        paste_image(app, ui);
    }
}

// the formats there are now
fn offers(app: &Diagrammer) -> Vec<i32> {
    let mut v = Vec::new();
    if app.clip.is_some() {
        v.push(F_SHAPES);
    }
    if app.paste.sys_text {
        v.push(F_TEXT);
    }
    if app.paste.sys_image {
        v.push(F_IMAGE);
    }
    v
}

/// Inhalte einfügen: the dialog
pub(crate) fn special(app: &mut Diagrammer, ui: &mut Ui) {
    let v = offers(app);
    if !v.contains(&app.paste.choice) {
        app.paste.choice = v.first().copied().unwrap_or(F_SHAPES);
    }
    app.paste.dlg.set(1);
    ui.repass();
}

/// every pass: the dialog Inhalte einfügen while it is open
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let dlg = app.paste.dlg.clone();
    if !ui.dialog_begin(ID_DLG, t("Inhalte einfügen"), &dlg) {
        return;
    }
    let v = offers(app);
    ui.next_size(380, 0);
    ui.box_begin(ID_DLG + 1, BOX_V, 8);
    ui.text(TS_BODY, t("Als:"));
    for (k, f) in [(F_SHAPES, t("Shapes (JFDiagrammer)")), (F_TEXT, t("Unformatierter Text (als Textfeld)")), (F_IMAGE, t("Bild (PNG oder BMP)"))] {
        if v.contains(&k) {
            ui.radio(ID_DLG + 2 + k as u32, f, &mut app.paste.choice, k);
        }
    }
    if v.is_empty() {
        ui.text(TS_BODY | TX_SECONDARY, t("Die Zwischenablage ist leer."));
    }
    ui.box_end();
    let ok = !v.is_empty();
    let r = ui.dialog_buttons(if ok { Some(t("OK")) } else { None }, None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY && ok {
        match app.paste.choice {
            F_SHAPES => crate::edit_cmds::paste(app, ui),
            F_TEXT => {
                if let Some(text) = system_text(ui) {
                    text_box(app, ui, t("Inhalte einfügen"), &text, None);
                }
            }
            _ => {
                paste_image(app, ui);
            }
        }
    }
    if r != DLG_NONE {
        app.paste.dlg.set(0);
        ui.repass();
    }
}

/// a text that is an address: a URL (scheme://, mailto:, www.) or a path (C:\, \\server,
/// /folder); one line -> the address it links to (None: no address)
pub(crate) fn address_of(text: &str) -> Option<String> {
    let s = text.trim();
    if s.is_empty() || s.contains('\n') || s.len() > 2048 {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    let scheme = lower.find("://").is_some_and(|i| i > 0 && lower[..i].chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')));
    if scheme || lower.starts_with("mailto:") || lower.starts_with("file:") {
        return (!s.contains(' ')).then(|| String::from(s));
    }
    if lower.starts_with("www.") && !s.contains(' ') {
        return Some(alloc::format!("http://{s}"));
    }
    let b = s.as_bytes();
    let drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/');
    if drive || s.starts_with("\\\\") || s.starts_with('/') {
        return Some(String::from(s));
    }
    None
}

/// Als Hyperlink einfügen
pub(crate) fn link(app: &mut Diagrammer, ui: &mut Ui) {
    // the app's shapes of this drawing first (they were copied last), then the system's text
    if let Some((page, shapes)) = linkable(app) {
        let Some(o) = app.opened.as_ref() else { return };
        let doc = &o.doc;
        let page_name = diagram_view::export::page_name(doc, page);
        let (text, sub) = if shapes.len() == 1 {
            let s = shapes[0];
            let name = doc.shape(s).map(|x| x.name()).unwrap_or_default();
            let label = shape_label(doc, s);
            let label = if label.is_empty() { name.clone() } else { label };
            // the text says where it leads: "Server (Seite-1)"
            (alloc::format!("{label} ({page_name})"), alloc::format!("{page_name}/{name}"))
        } else {
            (page_name.clone(), page_name)
        };
        text_box(app, ui, t("Als Link einfügen"), &text, Some(("", &sub)));
        return;
    }
    let Some(text) = system_text(ui) else { return };
    match address_of(&text) {
        Some(addr) => {
            text_box(app, ui, t("Als Link einfügen"), text.trim(), Some((&addr, "")));
        }
        None => {
            app.error = Some((String::from(t("Kein Hyperlink in der Zwischenablage")), String::from(t("Als Hyperlink lässt sich eine Adresse (https://…) oder ein Pfad einfügen, oder Shapes aus dieser Zeichnung."))));
            ui.repass();
        }
    }
}
