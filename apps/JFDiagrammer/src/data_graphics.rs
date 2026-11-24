// data_graphics -- (JD19) data graphics in the app: the register Daten › Datengrafiken, Daten
// anzeigen › Legende einfügen, Einblenden/Ausblenden › Datengrafikfelder, Erweiterte
// Datenverknüpfung › Erweiterte Datengrafiken (DA-07, DA-08, ST-08, RD-04, RD-05, RD-07, RD-08,
// RD-10, AB-12); the model is diagram_core::datagraphic, the callout masters come from the
// hidden stencil stencils/datagraphics
//
//   DgState                   the field chosen in the pane, the graphic the menus act on, the
//                             thumbnails, the dialogs
//   available(app, id)        when the commands can run (commands::usable asks)
//   targets(app)              the shapes an action works on: the selected shapes with data (a
//                             callout stands for its shape), else every shape of the page with
//                             linked data
//   fields(app)               the fields of the pane: the columns of the data linked to the
//                             targets (else their shape data), with the graphic's items marked
//   change(app, ui, label, f) the graphic of the targets changed by f (an item added, its kind,
//                             style or position set ...) and written back: the graphic the targets
//                             show is updated (every shape that uses it follows), else a new one
//                             "Daten – <Quelle>" is made and put on the targets; one undo step
//   commands                  Position (horizontal / vertical, PNT of the chosen field's item),
//                             Konfigurieren (the item's dialog), Legende einfügen (horizontal /
//                             vertical), Keine Datengrafik, the advanced menu's Neu, Bearbeiten,
//                             Umbenennen, Duplizieren, Löschen, Shapes mit dieser Grafik auswählen
//
// The other files: data_graphics_gallery.rs (the ribbon gallery and the advanced gallery with
// rendered tiles), data_graphics_pane.rs (the pane Datengrafikfelder, P_DG_FIELDS),
// data_graphics_dlg.rs (the dialogs Datengrafik bearbeiten, Element bearbeiten, Umbenennen).
// As in Visio a shape shows one data graphic; the gallery and the pane change the graphic the
// shapes show for the field chosen in the pane, so every shape that shows it follows.

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::datagraphic::{self as dg, legend, DataGraphic, Item, Kind, Pos};
use diagram_core::{Document, EditError, MasterRef, Owner, PageRef, ShapeRef, SheetRef};
use gfx::Surface;
use ui::i18n::{t, tf};
use ui::*;

use crate::commands::*;
use crate::{edit_canvas, edit_doc, Diagrammer};

/// the hidden stencil with the callout masters
pub(crate) const STENCIL: &str = "datagraphics";

/// the app's state of data graphics
#[derive(Default)]
pub(crate) struct DgState {
    /// the field chosen in the pane Datengrafikfelder (its label)
    pub field: Option<String>,
    /// the graphic the advanced menu's commands act on (a tile's menu)
    pub current: Option<MasterRef>,
    /// rendered tiles (key, picture)
    pub thumbs: Vec<(u64, Rc<Surface>)>,
    /// the dialogs (data_graphics_dlg.rs)
    pub dlg: crate::data_graphics_dlg::Dialogs,
}

// ---------------------------------------------------------------- where things are

/// the callout masters (the hidden built-in stencil)
pub(crate) fn source(app: &Diagrammer) -> Option<&Document> {
    app.stencils
        .loaded
        .iter()
        .find(|l| matches!(l.origin, crate::stencils::Origin::Builtin(k) if crate::stencils::BUILTIN.get(k).is_some_and(|b| b.file == STENCIL)))
        .map(|l| &l.doc)
}

/// the page shown
pub(crate) fn page(app: &Diagrammer) -> Option<PageRef> {
    app.opened.as_ref()?;
    app.model.page
}

// whether a shape shows data: it is linked to a row or has shape data
fn has_data(doc: &Document, s: ShapeRef) -> bool {
    !doc.data_links(s).is_empty()
}

// a callout of a data graphic stands for its shape
fn owner_of(doc: &Document, s: ShapeRef) -> ShapeRef {
    match doc.shape(s).and_then(|x| x.parent()) {
        Some(p) if dg::callouts(doc, p).contains(&s) => p,
        _ => s,
    }
}

/// the shapes an action works on: the selection (with data or a graphic), else every shape of
/// the page with linked data
pub(crate) fn targets(app: &Diagrammer) -> Vec<ShapeRef> {
    let (Some(o), Some(p)) = (app.opened.as_ref(), page(app)) else { return Vec::new() };
    let doc = &o.doc;
    let sel: Vec<ShapeRef> = edit_canvas::selected(app).into_iter().map(|s| owner_of(doc, s)).collect();
    let mut out: Vec<ShapeRef> = Vec::new();
    if !sel.is_empty() {
        for s in sel {
            if !out.contains(&s) && doc.shape(s).is_some() && (has_data(doc, s) || dg::applied(doc, s).is_some() || !doc.shape_data(SheetRef::Shape(s)).is_empty()) {
                out.push(s);
            }
        }
        // a selection without data (a legend, a connector): the page's shapes as without one
        if !out.is_empty() {
            return out;
        }
    }
    doc.all_shapes(Owner::Page(p)).filter(|&s| has_data(doc, s)).collect()
}

/// the graphic the targets show (the first one that shows one)
pub(crate) fn graphic_of(app: &Diagrammer) -> Option<MasterRef> {
    let doc = &app.opened.as_ref()?.doc;
    targets(app).into_iter().find_map(|s| dg::applied(doc, s))
}

/// a field of the pane: its label, whether the graphic has an item for it
pub(crate) struct PaneField {
    pub label: String,
    pub on: bool,
    /// a number column (a data bar fits)
    pub numeric: bool,
}

/// the pane's fields: the columns of the data linked to the targets in their order, else the
/// shape data the targets have
pub(crate) fn fields(app: &Diagrammer) -> Vec<PaneField> {
    let Some(o) = app.opened.as_ref() else { return Vec::new() };
    let doc = &o.doc;
    let ts = targets(app);
    let g = graphic_of(app).and_then(|m| dg::read(doc, m));
    let on = |label: &str| g.as_ref().is_some_and(|g| g.item_of(&dg::field_expr(label)).is_some());
    let mut out: Vec<PaneField> = Vec::new();
    let mut sets: Vec<u32> = Vec::new();
    for &s in &ts {
        for (rs, _) in doc.data_links(s) {
            if !sets.contains(&rs) {
                sets.push(rs);
            }
        }
    }
    for rs in sets {
        let Some(r) = doc.recordset(rs) else { continue };
        let mut cols: Vec<&diagram_core::data::Column> = r.columns.iter().collect();
        cols.sort_by_key(|c| c.display_order.unwrap_or(u32::MAX));
        for c in cols {
            if out.iter().any(|f| f.label == c.label) {
                continue;
            }
            let numeric = matches!(c.data_type, diagram_core::data::kind::NUMBER | diagram_core::data::kind::CURRENCY);
            out.push(PaneField { label: c.label.clone(), on: on(&c.label), numeric });
        }
    }
    if out.is_empty() {
        for f in dg::fields(doc, &ts) {
            out.push(PaneField { on: on(&f.label), label: f.label, numeric: false });
        }
    }
    out
}

/// the chosen field when the pane lists it
pub(crate) fn chosen(app: &Diagrammer) -> Option<String> {
    let f = app.dg.field.clone()?;
    fields(app).iter().any(|x| x.label == f).then_some(f)
}

/// the item of the chosen field in the targets' graphic
pub(crate) fn chosen_item(app: &Diagrammer) -> Option<Item> {
    let f = chosen(app)?;
    let m = graphic_of(app)?;
    let g = dg::read(&app.opened.as_ref()?.doc, m)?;
    let k = g.item_of(&dg::field_expr(&f))?;
    Some(g.items[k].clone())
}

/// the name of the data the targets show ("Daten – Geräte")
fn new_name(app: &Diagrammer) -> String {
    let doc = app.opened.as_ref().map(|o| &o.doc);
    let rs = doc.and_then(|d| targets(app).into_iter().find_map(|s| d.data_links(s).first().map(|x| x.0)).and_then(|id| d.recordset(id)).map(|r| r.name.clone()));
    match rs {
        Some(n) => tf("Daten – {}", &[&n]),
        None => String::from(t("Datengrafik")),
    }
}

/// the layer's name in the current language
pub(crate) fn layer_name() -> &'static str {
    t("Datengrafik")
}

// ---------------------------------------------------------------- changing

/// change the targets' graphic with `f` (see the module comment) -> it worked.  Shapes without
/// a graphic take the document's graphic of the data's name when there is one (the pane's
/// graphic, its last item switched off before), else a new one
pub(crate) fn change(app: &mut Diagrammer, ui: &mut Ui, label: &str, f: impl FnOnce(&mut DataGraphic)) -> bool {
    let ts = targets(app);
    if ts.is_empty() {
        return false;
    }
    let name = new_name(app);
    let m = graphic_of(app).or_else(|| app.opened.as_ref().and_then(|o| dg::find_master(&o.doc, &name).filter(|&m| dg::is_graphic(&o.doc, m))));
    let mut g = match (m, app.opened.as_ref()) {
        (Some(m), Some(o)) => dg::read(&o.doc, m).unwrap_or_else(|| DataGraphic::new(&name)),
        _ => DataGraphic::new(&name),
    };
    f(&mut g);
    let src = source(app).cloned();
    let Some(o) = app.opened.as_mut() else { return false };
    let layer = layer_name();
    let r = edit_doc::edit(o, label, true, |tx, eng| {
        let m = match m {
            // (the last item gone: the graphic goes off the shapes, the master stays)
            Some(m) => {
                dg::update(tx, eng, m, &g, src.as_ref(), layer)?;
                m
            }
            None if g.items.is_empty() => return Ok(()),
            None => dg::create(tx, eng, &g, src.as_ref())?,
        };
        let missing: Vec<ShapeRef> = ts.iter().copied().filter(|&s| dg::applied(tx.doc(), s) != Some(m)).collect();
        if !missing.is_empty() && !g.items.is_empty() {
            dg::apply(tx, eng, &missing, m, layer)?;
        }
        Ok(())
    });
    report(ui, r)
}

/// a refused edit is said in the status bar's announcement
pub(crate) fn report<T>(ui: &mut Ui, r: Result<T, EditError>) -> bool {
    ui.repass();
    match r {
        Ok(_) => true,
        Err(e) => {
            ui.announce(&tf("Die Datengrafik lässt sich nicht ändern: {}", &[&format!("{e:?}")]), LIVE_POLITE);
            false
        }
    }
}

/// the callout master of a kind the app offers first
pub(crate) fn default_callout(kind: Kind) -> &'static str {
    match kind {
        Kind::Text | Kind::Heading => "DG Text",
        Kind::DataBar => "DG Progress bar",
        Kind::IconSet => "DG Traffic lights",
        Kind::ColorByValue => "",
    }
}

/// an item for a field shown as `kind` with callout master `callout`: rules and colours made
/// from the shapes' values (the first values in the icons' order, each value a colour)
pub(crate) fn new_item(app: &Diagrammer, field: &str, kind: Kind, callout: &str) -> Item {
    let mut it = Item::new(kind, &dg::field_expr(field), if kind == Kind::ColorByValue { "" } else { callout });
    let values = values_of(app, field);
    match kind {
        Kind::IconSet => {
            let n = icon_count(app, callout).max(1);
            let nums: Vec<f64> = values.iter().filter_map(|v| dg::parse_num(v)).collect();
            if !values.is_empty() && nums.len() == values.len() {
                // numbers: even steps between the smallest and the largest
                let (lo, hi) = nums.iter().fold((f64::MAX, f64::MIN), |a, &x| (a.0.min(x), a.1.max(x)));
                let step = (hi - lo) / n as f64;
                for k in 0..n {
                    let a = hi - step * (k + 1) as f64;
                    it.rules.push(if k + 1 == n { dg::Rule::new(dg::Op::Less, &num_text(hi - step * k as f64), "") } else { dg::Rule::new(dg::Op::GreaterEq, &num_text(a), "") });
                }
            } else {
                for k in 0..n {
                    match values.get(k) {
                        Some(v) => it.rules.push(dg::Rule::new(dg::Op::Equal, v, "")),
                        None => it.rules.push(dg::Rule::new(dg::Op::NotUsed, "", "")),
                    }
                }
            }
        }
        Kind::ColorByValue => {
            for (k, v) in values.iter().take(12).enumerate() {
                it.shades.push(dg::Shade { value: v.clone(), to: String::new(), fill: PALETTE[k % PALETTE.len()].1, text: None });
            }
        }
        Kind::DataBar => {
            let nums: Vec<f64> = values.iter().filter_map(|v| dg::parse_num(v)).collect();
            let hi = nums.iter().fold(0.0f64, |a, &x| a.max(x));
            it.set_opt("msvCalloutPropMin", dg::Opt::Num(0.0));
            it.set_opt("msvCalloutPropMax", dg::Opt::Num(if hi <= 100.0 { 100.0 } else { nice(hi) }));
        }
        _ => {}
    }
    it
}

// a round number at or above x (1, 2, 5 times a power of ten)
fn nice(x: f64) -> f64 {
    let mut p = 1.0;
    while p * 10.0 <= x {
        p *= 10.0;
    }
    [1.0, 2.0, 5.0, 10.0].iter().map(|k| k * p).find(|&v| v >= x).unwrap_or(x)
}

pub(crate) fn num_text(x: f64) -> String {
    if x == (x as i64) as f64 { format!("{}", x as i64) } else { format!("{x}") }
}

/// the named colours the dialogs offer (fill colours of colour by value)
// i18n
pub(crate) const PALETTE: [(&str, u32); 12] = [
    ("Hellgrün", 0xC6E0B4),
    ("Hellblau", 0xBDD7EE),
    ("Orange", 0xF8CBAD),
    ("Gelb", 0xFFE699),
    ("Lila", 0xD9C3E9),
    ("Grau", 0xD9D9D9),
    ("Grün", 0xA9D18E),
    ("Blau", 0x9DC3E6),
    ("Rot", 0xF4B183),
    ("Dunkelgelb", 0xFFD966),
    ("Türkis", 0xB4E0DC),
    ("Weiß", 0xFFFFFF),
];

/// the distinct values of a field on the targets, in the order met
pub(crate) fn values_of(app: &Diagrammer, field: &str) -> Vec<String> {
    let Some(o) = app.opened.as_ref() else { return Vec::new() };
    let doc = &o.doc;
    let mut out: Vec<String> = Vec::new();
    for s in targets(app) {
        let Some(r) = doc.shape_data(SheetRef::Shape(s)).into_iter().find(|r| r.text("Label").is_some_and(|l| l == field) || r.name() == Some(field)) else { continue };
        let v = crate::pane_data::value_text(doc, r.get("Value").value());
        let v = String::from(v.trim());
        if !v.is_empty() && !out.contains(&v) {
            out.push(v);
        }
    }
    out
}

/// how many icons an icon set master has (User.msvCalloutIconCount)
pub(crate) fn icon_count(app: &Diagrammer, callout: &str) -> usize {
    let n = |doc: &Document| {
        let m = dg::find_master(doc, callout)?;
        let slot = doc.master(m)?.master_shape()?;
        let sh = SheetRef::Shape(ShapeRef { owner: Owner::Master(m), slot });
        dg::user_text(doc, sh, "msvCalloutIconCount").and_then(|t| dg::parse_num(&t))
    };
    let v = app.opened.as_ref().and_then(|o| n(&o.doc)).or_else(|| source(app).and_then(n));
    v.map_or(0, |x| x.clamp(0.0, 5.0) as usize)
}

/// the callout masters of a kind: (NameU, name in the current language)
pub(crate) fn styles(app: &Diagrammer, kind: Kind) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let lang = crate::stencils::lang();
    let mut add = |doc: &Document| {
        for m in doc.masters() {
            if dg::is_graphic(doc, m) {
                continue;
            }
            let Some(slot) = doc.master(m).and_then(|x| x.master_shape()) else { continue };
            let sh = SheetRef::Shape(ShapeRef { owner: Owner::Master(m), slot });
            let Some(k) = dg::user_text(doc, sh, "msvCalloutType").and_then(|t| Kind::parse(&t)) else { continue };
            let same = k == kind || (kind == Kind::Text && k == Kind::Heading);
            let Some(nu) = doc.master(m).and_then(|x| x.header.name_u).map(|s| String::from(doc.str(s))) else { continue };
            if !same || out.iter().any(|(n, _)| n.eq_ignore_ascii_case(&nu)) {
                continue;
            }
            let texts = diagram_core::instance::MasterTexts::of(doc, m);
            let shown = String::from(texts.name(lang));
            out.push((nu.clone(), if shown.is_empty() { nu } else { shown }));
        }
    };
    if let Some(s) = source(app) {
        add(s);
    }
    if let Some(o) = app.opened.as_ref() {
        add(&o.doc);
    }
    out
}

/// the name of a kind in the current language
pub(crate) fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Text | Kind::Heading => crate::tc("Text|Datengrafik"),
        Kind::DataBar => t("Datenleiste"),
        Kind::IconSet => t("Symbolsatz"),
        Kind::ColorByValue => t("Farbe nach Wert"),
    }
}

// ---------------------------------------------------------------- the commands

/// whether command `id` of this file can run now (other commands: true)
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    let doc = app.opened.as_ref().map(|o| &o.doc);
    let has_dg = doc.is_some_and(|d| !dg::graphics(d).is_empty());
    match id {
        DATA_GRAPHICS => doc.is_some(),
        DG_POSITION | DG_CONFIGURE => chosen_item(app).is_some(),
        _ if (DG_POS_H0..=DG_POS_V6).contains(&id) => chosen_item(app).is_some(),
        DATA_LEGEND | DG_LEGEND_H | DG_LEGEND_V => page_has_graphic(app),
        PANE_DG_FIELDS | DATA_ADVANCED_DG | DG_NEW => doc.is_some(),
        DG_EDIT_GRAPHIC => doc.is_some() && (has_dg || !targets(app).is_empty()),
        DG_RENAME | DG_DUPLICATE | DG_DELETE | DG_SELECT_SHAPES => current(app).is_some(),
        DG_NONE => !targets(app).is_empty() && targets(app).iter().any(|&s| doc.is_some_and(|d| dg::applied(d, s).is_some())),
        _ => true,
    }
}

fn page_has_graphic(app: &Diagrammer) -> bool {
    let (Some(o), Some(p)) = (app.opened.as_ref(), page(app)) else { return false };
    o.doc.all_shapes(Owner::Page(p)).any(|s| dg::applied(&o.doc, s).is_some())
}

/// the graphic the advanced menu's commands act on: a tile's (its menu; the command clears it),
/// else the selection's, else the document's first
pub(crate) fn current(app: &Diagrammer) -> Option<MasterRef> {
    let doc = &app.opened.as_ref()?.doc;
    let ok = |m: MasterRef| dg::is_graphic(doc, m);
    app.dg.current.filter(|&m| ok(m)).or_else(|| graphic_of(app)).or_else(|| dg::graphics(doc).into_iter().next())
}

/// Position › Horizontal / Vertikal: the chosen field's item (from the default position)
pub(crate) fn position(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
    let Some(f) = chosen(app) else { return };
    let (horizontal, k) = if id <= DG_POS_H6 { (true, (id - DG_POS_H0) as u8) } else { (false, (id - DG_POS_V0) as u8) };
    change(app, ui, t("Position der Datengrafik"), |g| {
        let def = g.default_pos;
        if let Some(i) = g.item_of(&dg::field_expr(&f)) {
            let p = g.items[i].pos.unwrap_or(def);
            g.items[i].pos = Some(if horizontal { Pos { h: k, ..p } } else { Pos { v: k, ..p } });
        }
    });
}

/// whether a position command is the chosen item's place
pub(crate) fn position_on(app: &Diagrammer, id: u32) -> bool {
    let Some(it) = chosen_item(app) else { return false };
    let def = app.opened.as_ref().and_then(|o| graphic_of(app).and_then(|m| dg::read(&o.doc, m))).map_or(Pos::DEFAULT, |g| g.default_pos);
    let p = it.pos.unwrap_or(def);
    if (DG_POS_H0..=DG_POS_H6).contains(&id) { p.h as u32 == id - DG_POS_H0 } else { p.v as u32 == id - DG_POS_V0 }
}

/// Konfigurieren: the dialog of the chosen field's item
pub(crate) fn configure(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(it) = chosen_item(app) else { return };
    crate::data_graphics_dlg::open_item_from_ribbon(app, ui, it);
}

/// Legende einfügen › Horizontal / Vertikal
pub(crate) fn insert_legend(app: &mut Diagrammer, ui: &mut Ui, horizontal: bool) {
    let Some(p) = page(app) else { return };
    let texts = legend_texts();
    let Some(o) = app.opened.as_mut() else { return };
    let r = edit_doc::edit(o, t("Legende einfügen"), true, |tx, eng| legend::insert(tx, eng, p, horizontal, &texts, layer_name()));
    if let Ok(l) = r {
        app.se.select(&[edit_canvas::key_of(l)]);
    }
    report(ui, r);
}

/// the legend's words in the current language
pub(crate) fn legend_texts() -> legend::Texts {
    let ops = [
        String::new(),
        String::from("="),
        String::from("≠"),
        String::from(">"),
        String::from("≥"),
        String::from("<"),
        String::from("≤"),
        String::from(t("zwischen")),
        String::from(t("enthält")),
        String::from(t("enthält nicht")),
    ];
    legend::Texts { title: String::from(t("Legende")), text: String::from(t("Wert als Text")), ops, icon: String::from(t("Symbol {}")) }
}

/// Keine Datengrafik: off the targets
pub(crate) fn none(app: &mut Diagrammer, ui: &mut Ui) {
    let ts = targets(app);
    let Some(o) = app.opened.as_mut() else { return };
    let r = edit_doc::edit(o, t("Keine Datengrafik"), true, |tx, eng| dg::remove(tx, eng, &ts));
    report(ui, r);
}

/// a graphic of the document onto the targets
pub(crate) fn apply_graphic(app: &mut Diagrammer, ui: &mut Ui, m: MasterRef) {
    let ts = targets(app);
    if ts.is_empty() {
        return;
    }
    let Some(o) = app.opened.as_mut() else { return };
    let r = edit_doc::edit(o, t("Datengrafik anwenden"), true, |tx, eng| dg::apply(tx, eng, &ts, m, layer_name()));
    report(ui, r);
}

/// Shapes mit dieser Grafik auswählen (on the page shown)
pub(crate) fn select_shapes(app: &mut Diagrammer, ui: &mut Ui) {
    let (Some(m), Some(p)) = (current(app), page(app)) else { return };
    app.dg.current = None;
    let Some(o) = app.opened.as_ref() else { return };
    let keys: Vec<u64> = dg::shapes_with(&o.doc, m).into_iter().filter(|s| s.owner == Owner::Page(p)).map(edit_canvas::key_of).collect();
    app.se.select(&keys);
    ui.repass();
}

/// Duplizieren
pub(crate) fn duplicate(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(m) = current(app) else { return };
    app.dg.current = None;
    let Some(o) = app.opened.as_mut() else { return };
    let name = dg::master_name(&o.doc, m);
    let r = edit_doc::edit(o, t("Datengrafik duplizieren"), true, |tx, eng| dg::duplicate(tx, eng, m, &name));
    report(ui, r);
}

/// Löschen: off every shape, then gone
pub(crate) fn delete(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(m) = current(app) else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let r = edit_doc::edit(o, t("Datengrafik löschen"), true, |tx, eng| dg::delete(tx, eng, m));
    app.dg.current = None;
    report(ui, r);
}
