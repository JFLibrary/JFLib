// data_graphics_gallery -- (JD19, RD-04, RD-10) the galleries of data graphics with rendered tiles
//
//   PRESETS                    the styles the gallery offers, by kind: text (three callouts),
//                              data bars (progress bar, stars, thermometer, speedometer), icon
//                              sets (traffic light, arrows, flags, rating bars), colour by value
//   gallery(app, ui, height)   Daten › Datengrafiken: Keine and the first style of each kind as
//                              tiles, ▾ the flyout with every style by kind and Weitere
//                              Datengrafiken (Konfigurieren); a click gives the field chosen in
//                              the pane that style (Keine: takes it away), for the shapes the
//                              commands work on (data_graphics::change)
//   flyout(app, ui, pid)       Erweiterte Datengrafiken: the document's data graphics as tiles (a
//                              click puts one on the shapes, a right click opens its menu), then
//                              Keine Datengrafik, Neue Datengrafik erstellen ..., Bearbeiten,
//                              Umbenennen, Duplizieren, Löschen, Shapes mit dieser Grafik
//                              auswählen; and the gallery's flyout (ID_ALL)
//
// The tiles are small drawings built and drawn like the page (as design_gallery.rs): a box with
// a shape data field "Wert", the style applied as a data graphic through
// diagram_core::datagraphic, drawn by diagram_view; a document's graphic gets a box with one
// field per item (a value in the middle of a bar, the first rule's value of an icon set, the
// first colour's value).  They are kept per style, graphic and size.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::datagraphic::{self as dg, DataGraphic, Item, Kind};
use diagram_core::formula::Engine;
use diagram_core::ops::{self, Basic};
use diagram_core::render::Options;
use diagram_core::{DocKind, Document, MasterRef, Owner, SheetRef, Value};
use diagram_io::vsdx::{self, Measure};
use diagram_view::PageView;
use gfx::Surface;
use ui::i18n::t;
use ui::*;

use crate::commands::*;
use crate::data_graphics::{self as dgs, change, chosen, kind_name, new_item};
use crate::Diagrammer;

const ID_TILES: u32 = 0x7D00_0000;
const ID_MORE: u32 = 0x7D00_0100;
/// the gallery's flyout with every style (DATA_GRAPHICS opens it, ribbon::menu_place)
pub(crate) const ID_ALL: u32 = 0x7D01_0000;
const ID_ADV_TILES: u32 = 0x7D03_0000;
const ID_TILE_MENU: u32 = 0x7D04_0000;

/// the styles the gallery offers: kind, callout master (NameU)
pub(crate) const PRESETS: [(Kind, &str); 12] = [
    (Kind::Text, "DG Text"),
    (Kind::Text, "DG Text box"),
    (Kind::Text, "DG Text underlined"),
    (Kind::DataBar, "DG Progress bar"),
    (Kind::DataBar, "DG Star rating"),
    (Kind::DataBar, "DG Thermometer"),
    (Kind::DataBar, "DG Speedometer"),
    (Kind::IconSet, "DG Traffic lights"),
    (Kind::IconSet, "DG Arrows"),
    (Kind::IconSet, "DG Flags"),
    (Kind::IconSet, "DG Rating"),
    (Kind::ColorByValue, ""),
];

/// the tiles shown in the ribbon: Keine, then text, data bar, icon set, colour by value
const SHOWN: [usize; 4] = [0, 3, 7, 11];

fn hash(text: &str, extra: &[u32]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes().chain(extra.iter().flat_map(|v| v.to_le_bytes())) {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

// a sample value: a number or a text
enum Sv {
    Num(f64),
    Text(String),
}

// the sample drawing: a box with the fields of `values`, the graphic `g` on it (a graphic of the
// document: `own`, copied over)
fn sample(g: &DataGraphic, values: &[(String, Sv)], src: &Document, own: Option<(&Document, MasterRef)>) -> Option<Document> {
    let mut doc = vsdx::new_document(DocKind::Drawing, Measure::Us);
    let page = doc.pages().next()?;
    let mut eng = Engine::new();
    let mut tx = doc.transaction("");
    let ps = SheetRef::Page(page);
    for (c, v) in [("PageWidth", 2.4), ("PageHeight", 1.3)] {
        let a = tx.single(c);
        tx.force_constant(ps, &a, Value::Num(v), None).ok()?;
    }
    let b = ops::new_shape(&mut tx, &mut eng, Owner::Page(page), Basic::Rectangle, [0.15, 0.4], [0.95, 0.9]).ok()?;
    if g.items.is_empty() && own.is_none() {
        let _ = tx.commit();
        return Some(doc);
    }
    for (label, v) in values {
        let v = match v {
            Sv::Num(x) => Value::Num(*x),
            Sv::Text(t) => Value::Text(tx.intern(t)),
        };
        tx.set_shape_data(SheetRef::Shape(b), label, label, v).ok()?;
    }
    let m = match own {
        Some((d, m)) => diagram_core::instance::import_master(&mut tx, d, m).ok()?,
        None => dg::create(&mut tx, &mut eng, g, Some(src)).ok()?,
    };
    dg::apply(&mut tx, &mut eng, &[b], m, "").ok()?;
    let _ = tx.commit();
    Some(doc)
}

// a thumbnail (kept)
fn thumb(app: &mut Diagrammer, key: u64, build: impl FnOnce(&Diagrammer) -> Option<Document>, w: i32, h: i32) -> Option<Rc<Surface>> {
    if let Some((_, s)) = app.dg.thumbs.iter().find(|(k, _)| *k == key) {
        return Some(s.clone());
    }
    let doc = build(app)?;
    let page = doc.pages().next()?;
    let mut fonts = app.fonts.clone();
    let mut v = PageView::new(&doc, page, &mut fonts, &Options::default());
    let sf = Rc::new(v.render(w.max(8) as u32, h.max(8) as u32));
    if app.dg.thumbs.len() > 64 {
        app.dg.thumbs.remove(0);
    }
    app.dg.thumbs.push((key, sf.clone()));
    Some(sf)
}

// the tile of preset k (None: Keine)
fn preset_thumb(app: &mut Diagrammer, k: Option<usize>, w: i32, h: i32) -> Option<Rc<Surface>> {
    let key = hash("preset", &[k.map_or(99, |k| k as u32), w as u32, h as u32]);
    thumb(
        app,
        key,
        |app| {
            let src = dgs::source(app)?;
            let mut g = DataGraphic::new("Vorschau");
            let mut values: Vec<(String, Sv)> = Vec::new();
            if let Some(k) = k {
                let (kind, callout) = PRESETS[k];
                let mut it = Item::new(kind, "{Wert}", callout);
                match kind {
                    Kind::DataBar => values.push((String::from("Wert"), Sv::Num(72.0))),
                    Kind::IconSet => {
                        it.rules = alloc::vec![dg::Rule::new(dg::Op::Greater, "50", "")];
                        values.push((String::from("Wert"), Sv::Num(72.0)));
                    }
                    Kind::ColorByValue => {
                        it.shades = alloc::vec![dg::Shade { value: String::from("A"), to: String::new(), fill: dgs::PALETTE[0].1, text: None }];
                        values.push((String::from("Wert"), Sv::Text(String::from("A"))));
                    }
                    _ => values.push((String::from("Wert"), Sv::Num(123.0))),
                }
                g.push(it);
            }
            sample(&g, &values, src, None)
        },
        w,
        h,
    )
}

// the tile of a document's graphic
fn graphic_thumb(app: &mut Diagrammer, m: MasterRef, w: i32, h: i32) -> Option<Rc<Surface>> {
    let g = dg::read(&app.opened.as_ref()?.doc, m)?;
    let key = hash(&format!("{g:?}"), &[w as u32, h as u32]);
    thumb(
        app,
        key,
        |app| {
            let o = app.opened.as_ref()?;
            let src = dgs::source(app)?;
            let mut values: Vec<(String, Sv)> = Vec::new();
            for it in &g.items {
                let Some(label) = it.label() else { continue };
                let v = match it.kind {
                    Kind::DataBar => {
                        let lo = it.opt("msvCalloutPropMin").and_then(|o| o.num()).unwrap_or(0.0);
                        let hi = it.opt("msvCalloutPropMax").and_then(|o| o.num()).unwrap_or(100.0);
                        Sv::Num(lo + (hi - lo) * 0.7)
                    }
                    Kind::IconSet => match it.rules.iter().find(|r| r.op != dg::Op::NotUsed) {
                        Some(r) => match dg::parse_num(&r.a) {
                            Some(x) if r.op == dg::Op::Greater => Sv::Num(x + 1.0),
                            Some(x) if r.op == dg::Op::Less => Sv::Num(x - 1.0),
                            Some(x) => Sv::Num(x),
                            None => Sv::Text(r.a.clone()),
                        },
                        None => Sv::Num(0.0),
                    },
                    Kind::ColorByValue => match it.shades.first() {
                        Some(s) => match dg::parse_num(&s.value) {
                            Some(x) => Sv::Num(x),
                            None => Sv::Text(s.value.clone()),
                        },
                        None => Sv::Num(0.0),
                    },
                    _ => Sv::Num(123.0),
                };
                values.push((String::from(label), v));
            }
            sample(&g, &values, src, Some((&o.doc, m)))
        },
        w,
        h,
    )
}

fn tile_size(ui: &Ui) -> (i32, i32) {
    (ui.dp(48), ui.dp(44))
}

/// the label of preset k (None: Keine)
pub(crate) fn preset_label(app: &Diagrammer, k: Option<usize>) -> String {
    let Some(k) = k else { return String::from(crate::tc("Keine|Datengrafik")) };
    let (kind, callout) = PRESETS[k];
    if kind == Kind::ColorByValue {
        return String::from(kind_name(kind));
    }
    // the name in the current language from the built-in stencil (every pass: not the drawing's masters)
    let shown = dgs::source(app).and_then(|doc| {
        let m = dg::find_master(doc, callout)?;
        let n = diagram_core::instance::MasterTexts::of(doc, m);
        Some(String::from(n.name(crate::stencils::lang())))
    });
    shown.filter(|n| !n.is_empty()).unwrap_or_else(|| String::from(callout))
}

/// a click on preset k: the chosen field gets this style (None: Keine, the field's item goes)
pub(crate) fn apply_preset(app: &mut Diagrammer, ui: &mut Ui, k: Option<usize>) {
    let Some(f) = chosen(app) else { return };
    match k {
        None => {
            change(app, ui, t("Keine Datengrafik"), |g| {
                if let Some(i) = g.item_of(&dg::field_expr(&f)) {
                    g.items.remove(i);
                }
            });
        }
        Some(k) => {
            let (kind, callout) = PRESETS[k];
            let fresh = new_item(app, &f, kind, callout);
            change(app, ui, t("Datengrafik anwenden"), |g| match g.item_of(&dg::field_expr(&f)) {
                Some(i) => {
                    let it = &mut g.items[i];
                    // another kind starts with its own options, rules and colours
                    if it.kind != kind {
                        let (id, pos) = (it.id, it.pos);
                        *it = Item { id, pos, ..fresh };
                    } else {
                        it.callout = String::from(callout);
                    }
                }
                None => {
                    g.push(fresh);
                }
            });
        }
    }
}

// whether preset k is the chosen field's style
fn preset_on(app: &Diagrammer, k: Option<usize>) -> bool {
    let it = dgs::chosen_item(app);
    match (k, it) {
        (None, None) => chosen(app).is_some(),
        (Some(k), Some(it)) => {
            let (kind, callout) = PRESETS[k];
            it.kind == kind && (kind == Kind::ColorByValue || it.callout.eq_ignore_ascii_case(callout))
        }
        _ => false,
    }
}

/// Daten › Datengrafiken: the tiles in the ribbon, ▾ every style
pub(crate) fn gallery(app: &mut Diagrammer, ui: &mut Ui, height: i32) {
    let (w, h) = tile_size(ui);
    let off = chosen(app).is_none();
    ui.box_begin(0, BOX_H | BOX_FIXED, 2);
    for (n, k) in core::iter::once(None).chain(SHOWN.iter().map(|&k| Some(k))).enumerate() {
        let label = preset_label(app, k);
        let Some(sf) = preset_thumb(app, k, w, h) else { continue };
        ui.next_size(56, height);
        ui.next_flags(if preset_on(app, k) { ST_CHECKED } else { 0 } | if off { ST_DISABLED } else { 0 });
        if ui.ribbon_image_tile(ID_TILES + n as u32, &label, &sf, n as u32) & EV_ACT != 0 {
            apply_preset(app, ui, k);
        }
        if off {
            ui.tooltip(&format!("{label} · {}", t("Wählen Sie ein Feld im Bereich Datengrafikfelder.")));
        }
    }
    ui.next_size(16, height);
    ui.next_flags(if app.opened.is_some() { 0 } else { ST_DISABLED });
    if ui.icon_button(ID_MORE, IC_CHEV_DOWN, t("Alle Datengrafiken")) & EV_ACT != 0 {
        ui.popup_open(ID_ALL);
    }
    ui.box_end();
    all_presets(app, ui);
}

// the gallery's flyout: every style by kind, Weitere Datengrafiken
fn all_presets(app: &mut Diagrammer, ui: &mut Ui) {
    if !ui.flyout_begin(ID_ALL, 0) {
        ui.flyout_end();
        return;
    }
    let (w, h) = tile_size(ui);
    let off = chosen(app).is_none();
    if off {
        ui.next_size(300, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Wählen Sie ein Feld im Bereich Datengrafikfelder."));
    }
    let groups: [(Option<Kind>, &str); 5] =
        [(None, crate::tc("Keine|Datengrafik")), (Some(Kind::Text), kind_name(Kind::Text)), (Some(Kind::DataBar), kind_name(Kind::DataBar)), (Some(Kind::IconSet), kind_name(Kind::IconSet)), (Some(Kind::ColorByValue), kind_name(Kind::ColorByValue))];
    for (kind, caption) in groups {
        ui.text(TS_CAPTION | TX_SECONDARY, caption);
        ui.box_begin(0, BOX_H, 2);
        let ks: Vec<Option<usize>> = match kind {
            None => alloc::vec![None],
            Some(kd) => (0..PRESETS.len()).filter(|&k| PRESETS[k].0 == kd).map(Some).collect(),
        };
        for k in ks {
            let label = preset_label(app, k);
            let Some(sf) = preset_thumb(app, k, w, h) else { continue };
            ui.next_size(56, 66);
            ui.next_flags(if preset_on(app, k) { ST_CHECKED } else { 0 } | if off { ST_DISABLED } else { 0 });
            let tile = ID_ALL + 1 + k.map_or(99, |k| k as u32);
            if ui.ribbon_image_tile(tile, &label, &sf, tile) & EV_ACT != 0 {
                apply_preset(app, ui, k);
                ui.popup_close();
            }
        }
        ui.box_end();
    }
    ui.menu_sep();
    ui.next_flags(if crate::commands::usable(app, DG_CONFIGURE) { 0 } else { ST_DISABLED });
    if ui.menu_item(ID_ALL + 0x100, t("Weitere Datengrafiken ...")) & EV_ACT != 0 {
        ui.popup_close();
        dgs::configure(app, ui);
    }
    ui.flyout_end();
}

// the advanced menu's commands
const ADV_ROWS: [u32; 8] = [DG_NONE, 0, DG_NEW, DG_EDIT_GRAPHIC, DG_RENAME, DG_DUPLICATE, DG_DELETE, DG_SELECT_SHAPES];

/// the flyouts of this file: Erweiterte Datengrafiken (the button's menu) -> drawn here
pub(crate) fn flyout(app: &mut Diagrammer, ui: &mut Ui, pid: u32) -> bool {
    if pid != DATA_ADVANCED_DG {
        return false;
    }
    if ui.flyout_begin(pid, 0) {
        let (w, h) = tile_size(ui);
        let list: Vec<MasterRef> = app.opened.as_ref().map(|o| dg::graphics(&o.doc)).unwrap_or_default();
        let applied = dgs::graphic_of(app);
        ui.text(TS_CAPTION | TX_SECONDARY, t("In dieser Zeichnung"));
        if list.is_empty() {
            ui.next_size(260, 0);
            ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Die Zeichnung hat noch keine Datengrafik."));
        }
        ui.box_begin(0, BOX_H, 2);
        let no_targets = dgs::targets(app).is_empty();
        for (k, &m) in list.iter().enumerate() {
            let name = app.opened.as_ref().map(|o| dg::master_name(&o.doc, m)).unwrap_or_default();
            let Some(sf) = graphic_thumb(app, m, w, h) else { continue };
            ui.next_size(56, 66);
            ui.next_flags(if applied == Some(m) { ST_CHECKED } else { 0 });
            let ev = ui.ribbon_image_tile(ID_ADV_TILES + k as u32, &name, &sf, k as u32);
            if ev & EV_ACT != 0 && !no_targets {
                dgs::apply_graphic(app, ui, m);
                ui.popup_close();
            }
            if ev & EV_RCLICK != 0 {
                app.dg.current = Some(m);
                let (mx, my) = ui.mouse();
                ui.popup_open_at(ID_TILE_MENU, mx, my);
            }
        }
        ui.box_end();
        ui.menu_sep();
        rows(app, ui, &ADV_ROWS, None);
    }
    ui.flyout_end();
    // a tile's menu: its rows have ids of their own (the flyout's rows are the commands')
    if ui.flyout_begin(ID_TILE_MENU, 0) {
        rows(app, ui, &[DG_EDIT_GRAPHIC, DG_RENAME, DG_DUPLICATE, DG_DELETE, 0, DG_SELECT_SHAPES], Some(ID_TILE_MENU + 1));
    }
    ui.flyout_end();
    true
}

// menu rows of commands: their ids (or base + k), the command runs on a click
fn rows(app: &mut Diagrammer, ui: &mut Ui, cmds: &[u32], base: Option<u32>) {
    for (k, &id) in cmds.iter().enumerate() {
        if id == 0 {
            ui.menu_sep();
            continue;
        }
        ui.next_flags(if crate::commands::usable(app, id) { 0 } else { ST_DISABLED });
        if ui.menu_item(base.map_or(id, |b| b + k as u32), crate::commands::label(id)) & EV_ACT != 0 {
            ui.popup_close();
            ui.cmd_run(id, 0);
        }
    }
}
