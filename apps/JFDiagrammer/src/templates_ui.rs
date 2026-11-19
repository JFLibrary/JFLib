// templates_ui -- (JD13) Datei › Neu, the template catalogue (VL-01, UI-04)
//
//   page(app, ui)               the categories at the side (every template, the categories that
//                               have templates, Meine Vorlagen), the templates of the one chosen
//                               as tiles with a picture of their first page (rendered from the
//                               template, templates::thumb); a click opens the preview, a double
//                               click creates the drawing at once
//   preview(app, ui, k)         the template's picture, its forms (Leer and the starter
//                               diagrams, each with its picture), name, description, metric or
//                               US units (not for own templates: they have theirs), Erstellen
//
// The tiles are buttons named after the template (scripts click button:"Standardflussdiagramm");
// the forms are buttons too, checked while chosen.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use ui::i18n::{t, tf};
use ui::*;

use crate::stencils;
use crate::templates::{self, MINE};
use crate::Diagrammer;

const ID_PAGE: u32 = 0x6100;
const ID_CAT: u32 = 0x6180;
const ID_TILE: u32 = 0x6200;
const ID_FORM: u32 = 0x6300;

// the names of the categories, in the order of stencils::CATEGORIES (the names of "Weitere
// Shapes")
// i18n
static CATEGORY_NAMES: [&str; 9] = ["Allgemein", "Flussdiagramm", "Geschäft", "Konstruktion", "Netzwerk", "Pläne und Grundrisse", "Software und Datenbank", "Terminplan", "Extras"];

fn category_name(key: &str) -> &'static str {
    if key == MINE {
        return t("Meine Vorlagen");
    }
    stencils::CATEGORIES.iter().position(|c| *c == key).and_then(|i| CATEGORY_NAMES.get(i)).map_or("", |n| t(n))
}

fn title(ui: &mut Ui, s: &str) {
    ui.text(TS_TITLE, s);
    ui.spacer(12);
}

/// the page Neu
pub(crate) fn page(app: &mut Diagrammer, ui: &mut Ui) {
    templates::catalog(app, ui);
    if let Some(k) = app.template {
        if k < app.catalog.len() {
            preview(app, ui, k);
            return;
        }
        app.template = None;
    }
    title(ui, t("Neu"));
    ui.box_begin(ID_PAGE, BOX_H, 24);
    categories(app, ui);
    ui.box_begin(0, BOX_V, 8);
    let shown = app.catalog.category.clone();
    ui.text(TS_BODY_STRONG, shown.as_deref().map_or(t("Vorlagen"), category_name));
    let list: Vec<usize> = (0..app.catalog.len()).filter(|&k| app.catalog.get(k).is_some_and(|e| shown.as_deref().is_none_or(|c| e.category == c))).collect();
    if list.is_empty() && shown.as_deref() == Some(MINE) {
        let dir = templates::own_dir(app, ui).unwrap_or_default();
        ui.next_size(560, 0);
        ui.text(
            TS_BODY | TX_WRAP | TX_SECONDARY,
            &tf("Hier erscheinen Ihre eigenen Vorlagen. Speichern Sie eine Zeichnung unter Datei › Speichern unter › Als Vorlage, oder legen Sie Vorlagen (.vstx) in diesen Ordner: {}", &[&dir]),
        );
    }
    let l = stencils::lang();
    ui.box_begin(ID_PAGE + 1, BOX_H | BOX_WRAP, 16);
    for &k in &list {
        let Some(e) = app.catalog.get(k) else { continue };
        let name = e.title(l);
        let form = e.default;
        let pic = showcase(e);
        ui.next_size(152, 0);
        ui.box_begin(0, BOX_V, 4);
        let ev = tile(app, ui, ID_TILE + k as u32, k, pic, &name, [152, 112], false);
        if ev & EV_DOUBLE != 0 {
            templates::create(app, ui, k, form);
        } else if ev & EV_ACT != 0 {
            app.template = Some(k);
            app.catalog.form = form;
            ui.repass();
        }
        ui.next_size(152, 0);
        ui.text(TS_BODY | TX_WRAP, &name);
        ui.box_end();
    }
    ui.box_end();
    ui.box_end();
    ui.box_end();
}

// the form a tile shows: the default one unless it is the blank page and a starter diagram can
// show what the template is for
fn showcase(e: &templates::Entry) -> usize {
    let blank = e.forms.get(e.default).is_none_or(|f| f.starter.is_none());
    if blank { e.forms.iter().position(|f| f.starter.is_some()).unwrap_or(e.default) } else { e.default }
}

// the categories at the side: every template, the categories that have some, the own ones
fn categories(app: &mut Diagrammer, ui: &mut Ui) {
    let mut keys: Vec<Option<String>> = alloc::vec![None];
    for key in stencils::CATEGORIES.iter() {
        if app.catalog.builtin.iter().any(|e| e.category == *key) {
            keys.push(Some(String::from(*key)));
        }
    }
    keys.push(Some(String::from(MINE)));
    ui.next_size(190, 0);
    ui.box_begin(0, BOX_V, 2);
    for (k, key) in keys.iter().enumerate() {
        let label = key.as_deref().map_or(t("Alle Vorlagen"), category_name);
        ui.next_size(-1, 30);
        ui.next_flags(ST_SUBTLE | ST_ALIGN_START | if app.catalog.category == *key { ST_CHECKED } else { 0 });
        if ui.button(ID_CAT + k as u32, label) & EV_ACT != 0 {
            app.catalog.category = key.clone();
            ui.repass();
        }
    }
    ui.box_end();
}

// a tile: the picture of form `f` of entry `k` on a card, `size` in dp; checked: chosen
#[allow(clippy::too_many_arguments)]
fn tile(app: &mut Diagrammer, ui: &mut Ui, id: u32, k: usize, f: usize, name: &str, size: [i32; 2], checked: bool) -> u32 {
    let r = ui.place(ui.dp(size[0]), ui.dp(size[1]));
    let ev = ui.behave(id, HB_FOCUS);
    let st = ui.style;
    let mut sp = Sp { flags: ST_SUBTLE | if checked { ST_CHECKED } else { 0 }, ..Sp::default() };
    sp.place(r);
    sp.ev = ev;
    sp.state = if ev & EV_HELD != 0 { 2 } else if ev & EV_HOVER != 0 { 1 } else { 0 };
    st.button(ui, &sp);
    // the page fitted into the card, its aspect kept
    let m = ui.dp(8);
    let (iw, ih) = ((r[2] - 2 * m).max(1), (r[3] - 2 * m).max(1));
    let a = templates::aspect(app, k, f).clamp(0.2, 5.0);
    let (w, h) = if iw as f64 / ih as f64 > a { (((ih as f64) * a) as i32, ih) } else { (iw, ((iw as f64) / a) as i32) };
    let (x, y) = (r[0] + (r[2] - w) / 2, r[1] + (r[3] - h) / 2);
    if let Some(sf) = templates::thumb(app, k, f, w, h) {
        ui.draw_image(&sf, (k as u32) << 8 | f as u32, x, y, w, h);
    }
    let frame = ui.color(if checked { T_ACCENT } else { T_CTRL_STROKE });
    let bw = if checked { ui.dp(2) } else { ui.hair() };
    ui.cmd_rrect(x, y, w, h, 0, bw, 0x00FF_FFFF, frame, frame);
    ui.focus_ring_draw(ev);
    let sem = ui.sem(K_BUTTON, id, Some(name), 0);
    if checked {
        sem.state |= SS_CHECKED;
    }
    ui.tooltip(name);
    ev
}

// a template's preview (UI-04): picture, forms, name, description, units, Erstellen
fn preview(app: &mut Diagrammer, ui: &mut Ui, k: usize) {
    let l = stencils::lang();
    let Some(e) = app.catalog.get(k) else { return };
    let name = e.title(l);
    let own = e.own();
    let forms: Vec<String> = e.forms.iter().map(|f| f.starter.as_ref().map_or_else(|| String::from(t("Leer")), |s| s[l].clone())).collect();
    let f = app.catalog.form.min(forms.len().saturating_sub(1));
    let about = e.about(f, l);
    ui.next_flags(ST_SUBTLE);
    ui.next_icon(IC_BACK);
    if ui.button(ID_PAGE + 1, t("Zurück zu den Vorlagen")) & EV_ACT != 0 {
        app.template = None;
        ui.repass();
    }
    ui.spacer(8);
    ui.box_begin(ID_PAGE + 2, BOX_H, 24);
    ui.box_begin(0, BOX_V, 12);
    ui.next_size(320, 236);
    ui.box_begin(0, BOX_V | BOX_CARD | pad(8), 0);
    let big = format!("{name} · {}", forms.get(f).map_or("", |s| s.as_str()));
    let ev = tile(app, ui, ID_PAGE + 3, k, f, &big, [304, 220], false);
    if ev & EV_DOUBLE != 0 {
        templates::create(app, ui, k, f);
        ui.box_end();
        ui.box_end();
        ui.box_end();
        return;
    }
    ui.box_end();
    if forms.len() > 1 {
        ui.box_begin(ID_PAGE + 7, BOX_H | BOX_WRAP, 8);
        for (i, label) in forms.iter().enumerate() {
            ui.next_size(100, 0);
            ui.box_begin(0, BOX_V, 2);
            let ev = tile(app, ui, ID_FORM + i as u32, k, i, label, [100, 74], i == f);
            if ev & EV_DOUBLE != 0 {
                app.catalog.form = i;
                templates::create(app, ui, k, i);
            } else if ev & EV_ACT != 0 {
                app.catalog.form = i;
                ui.repass();
            }
            ui.next_size(100, 0);
            ui.text(TS_CAPTION | TX_WRAP, label);
            ui.box_end();
        }
        ui.box_end();
    }
    ui.box_end();
    ui.next_size(340, 0);
    ui.box_begin(0, BOX_V, 10);
    ui.text(TS_SUBTITLE, &name);
    if forms.len() > 1 {
        ui.text(TS_BODY_STRONG, forms.get(f).map_or("", |s| s.as_str()));
    }
    if !about.is_empty() {
        ui.next_size(340, 0);
        ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, &about);
    }
    ui.spacer(4);
    if !own {
        ui.radio(ID_PAGE + 4, t("Metrische Einheiten"), &mut app.units_us, 0);
        ui.radio(ID_PAGE + 5, t("US-Einheiten"), &mut app.units_us, 1);
        ui.spacer(4);
    }
    ui.next_flags(ST_ACCENT);
    if ui.button(ID_PAGE + 6, t("Erstellen")) & EV_ACT != 0 {
        templates::create(app, ui, k, f);
    }
    ui.box_end();
    ui.box_end();
}
