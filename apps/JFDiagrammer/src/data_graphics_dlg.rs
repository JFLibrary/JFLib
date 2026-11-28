// data_graphics_dlg -- (JD19, DA-07, RD-05, RD-10) the dialogs of data graphics
//
//   open_edit(app, ui, m)      Datengrafik bearbeiten (m) / Neue Datengrafik (None): the items
//                              (Neues Element ..., Element bearbeiten ..., Löschen, Nach oben,
//                              Nach unten), the default position (horizontal, vertical), Shape-Text
//                              ausblenden, Rahmen um Elemente an der Standardposition; OK and
//                              Übernehmen write it (a new graphic goes onto the selected shapes),
//                              every shape that shows it follows
//   open_item / open_item_from_ribbon   Element bearbeiten / Neues Element: Datenfeld (Weitere
//                              Felder: an expression of {Feld} names), Anzeigen als, Stil,
//                              Position (Standardposition verwenden, horizontal, vertical) and the
//                              details of the kind -- text: label, its position, value format,
//                              show value, border, fill; data bar: minimum, maximum, show value,
//                              value format; icon set: one rule per icon (test, value, second
//                              value of "zwischen"); colour by value: each value or ranges, the
//                              rows of value (to), fill and text colour
//   open_rename(app, ui)       Umbenennen
//   dialogs(app, ui)           every pass
//
// One dialog is shown at a time: Element bearbeiten opened from Datengrafik bearbeiten hides it
// and brings it back when it closes.  Konfigurieren (the ribbon) opens Element bearbeiten alone;
// its OK changes the graphic of the shapes at once (data_graphics::change).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::datagraphic::{self as dg, DataGraphic, Item, Kind, Op, Opt, Pos, Rule, Shade};
use diagram_core::MasterRef;
use ui::i18n::{t, tf};
use ui::*;

use crate::data_graphics::{self as dgs, change, kind_name, PALETTE};
use crate::{edit_doc, tc, Diagrammer};

const ID_EDIT: u32 = 0x7D20_0000;
const ID_ITEM: u32 = 0x7D30_0000;
const ID_RENAME: u32 = 0x7D40_0000;

// i18n
const H_NAMES: [&str; 7] = ["Weit links|Position", "Linker Rand|Position", "Links|Position", "Zentriert|Position", "Rechts|Position", "Rechter Rand|Position", "Weit rechts|Position"];
// i18n
const V_NAMES: [&str; 7] = ["Unterhalb|Position", "Unterer Rand|Position", "Unten|Position", "Mittig|Position", "Oben|Position", "Oberer Rand|Position", "Oberhalb|Position"];
const KINDS: [Kind; 4] = [Kind::Text, Kind::DataBar, Kind::IconSet, Kind::ColorByValue];
// i18n
const FORMATS: [(&str, &str); 8] = [
    ("Allgemein", ""),
    ("Zahl", "0"),
    ("Zahl mit zwei Dezimalstellen", "0.00"),
    ("Zahl mit Tausendertrennzeichen", "#,##0"),
    ("Währung", "#,##0.00 €"),
    ("Prozent", "0\" %\""),
    ("Datum", "{{dd.MM.yyyy}}"),
    ("Text", "@"),
];
const LABEL_VALUES: [&str; 5] = ["Not Shown", "Left of Value", "Right of Value", "Above Value", "Below Value"];
// i18n
const LABEL_NAMES: [&str; 5] = ["Nicht angezeigt", "Links vom Wert", "Rechts vom Wert", "Über dem Wert", "Unter dem Wert"];
const BORDER_VALUES: [&str; 3] = ["None", "Bottom", "Outline"];
// i18n
const BORDER_NAMES: [&str; 3] = ["Kein Rahmen", "Linie unten", "Umriss"];
const FILL_VALUES: [&str; 2] = ["None", "Filled"];
// i18n
const FILL_NAMES: [&str; 2] = ["Keine Füllung|Datengrafik", "Gefüllt"];
// i18n
const OP_NAMES: [&str; 10] = ["[nicht verwendet]", "ist gleich", "ist ungleich", "ist größer als", "ist größer als oder gleich", "ist kleiner als", "ist kleiner als oder gleich", "liegt zwischen", "enthält", "enthält nicht"];
// i18n
const TEXT_COLORS: [(&str, Option<u32>); 5] = [("Automatisch", None), ("Schwarz", Some(0x000000)), ("Weiß", Some(0xFFFFFF)), ("Dunkelblau", Some(0x1F3864)), ("Dunkelrot", Some(0x7F0000))];

/// a colour row of the dialog
#[derive(Clone, Default)]
struct ShadeRow {
    value: String,
    to: String,
    fill: i32,
    /// a fill not in the palette (kept)
    own_fill: Option<u32>,
    text: i32,
    own_text: Option<u32>,
}

/// the form of Element bearbeiten
#[derive(Clone, Default)]
struct Form {
    id: u32,
    fields: Vec<String>,
    field: i32,
    expr: String,
    kind: i32,
    style: i32,
    use_default: i32,
    h: i32,
    v: i32,
    label: String,
    label_pos: i32,
    format: i32,
    own_format: String,
    show_value: i32,
    border: i32,
    fill: i32,
    min: String,
    max: String,
    rules: Vec<(i32, String, String)>,
    ranges: i32,
    shades: Vec<ShadeRow>,
    icon_formula: Option<String>,
    /// options of the item the form does not show (kept)
    other: Vec<(String, Opt)>,
}

/// the dialogs' state
#[derive(Default)]
pub(crate) struct Dialogs {
    edit: Shared<i32>,
    target: Option<MasterRef>,
    g: Option<DataGraphic>,
    pick: i32,
    item: Shared<i32>,
    index: Option<usize>,
    from_ribbon: bool,
    form: Form,
    rename: Shared<i32>,
    name: String,
}

// ---------------------------------------------------------------- opening

/// Datengrafik bearbeiten (Some) / Neue Datengrafik erstellen (None)
pub(crate) fn open_edit(app: &mut Diagrammer, ui: &mut Ui, m: Option<MasterRef>) {
    let Some(o) = app.opened.as_ref() else { return };
    let g = match m.and_then(|m| dg::read(&o.doc, m)) {
        Some(g) => g,
        None => DataGraphic::new(&dg::unique_name(&o.doc, t("Datengrafik"))),
    };
    let d = &mut app.dg.dlg;
    d.target = m.filter(|&m| dg::is_graphic(&o.doc, m));
    d.g = Some(g);
    d.pick = 0;
    d.edit.set(1);
    ui.repass();
}

/// DG_NEW
pub(crate) fn new_graphic(app: &mut Diagrammer, ui: &mut Ui) {
    open_edit(app, ui, None);
}

/// DG_EDIT_GRAPHIC: the graphic of a tile's menu, else the selection's, else a new one
pub(crate) fn edit_graphic(app: &mut Diagrammer, ui: &mut Ui) {
    let m = app.dg.current.take().or_else(|| dgs::graphic_of(app));
    open_edit(app, ui, m);
}

/// Konfigurieren: Element bearbeiten for an item of the shapes' graphic
pub(crate) fn open_item_from_ribbon(app: &mut Diagrammer, ui: &mut Ui, it: Item) {
    // the default position of the shapes' graphic
    app.dg.dlg.g = app.opened.as_ref().and_then(|o| dgs::graphic_of(app).and_then(|m| dg::read(&o.doc, m)));
    app.dg.dlg.target = None;
    app.dg.dlg.from_ribbon = true;
    app.dg.dlg.index = None;
    load_form(app, &it);
    app.dg.dlg.item.set(1);
    ui.repass();
}

// Element bearbeiten / Neues Element from Datengrafik bearbeiten
fn open_item(app: &mut Diagrammer, ui: &mut Ui, index: Option<usize>) {
    let it = match index.and_then(|i| app.dg.dlg.g.as_ref().and_then(|g| g.items.get(i).cloned())) {
        Some(it) => it,
        None => {
            let field = dgs::fields(app).first().map(|f| f.label.clone()).unwrap_or_default();
            dgs::new_item(app, &field, Kind::Text, dgs::default_callout(Kind::Text))
        }
    };
    app.dg.dlg.from_ribbon = false;
    app.dg.dlg.index = index;
    load_form(app, &it);
    app.dg.dlg.edit.set(0);
    app.dg.dlg.item.set(1);
    ui.repass();
}

/// Umbenennen
pub(crate) fn open_rename(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(m) = dgs::current(app) else { return };
    let Some(o) = app.opened.as_ref() else { return };
    app.dg.current = Some(m);
    app.dg.dlg.name = dg::master_name(&o.doc, m);
    app.dg.dlg.rename.set(1);
    ui.repass();
}

fn opt_text(it: &Item, name: &str) -> Option<String> {
    it.opt(name).map(|o| o.text())
}

fn opt_flag(it: &Item, name: &str, default: bool) -> bool {
    it.opt(name).and_then(|o| o.num()).map_or(default, |x| x != 0.0)
}

// the form from an item
fn load_form(app: &mut Diagrammer, it: &Item) {
    let mut fields: Vec<String> = dgs::fields(app).into_iter().map(|f| f.label).collect();
    let label = it.label().map(String::from);
    if let Some(l) = &label {
        if !fields.contains(l) {
            fields.push(l.clone());
        }
    }
    let field = match &label {
        Some(l) => fields.iter().position(|f| f == l).map_or(0, |k| k as i32),
        // an expression: Weitere Felder (the entry after the fields)
        None => fields.len() as i32,
    };
    let kind = KINDS.iter().position(|&k| k == it.kind || (it.kind == Kind::Heading && k == Kind::Text)).unwrap_or(0) as i32;
    let styles = dgs::styles(app, KINDS[kind as usize]);
    let style = styles.iter().position(|(n, _)| n.eq_ignore_ascii_case(&it.callout)).unwrap_or(0) as i32;
    let def = app.dg.dlg.g.as_ref().map_or(Pos::DEFAULT, |g| g.default_pos);
    let p = it.pos.unwrap_or(def);
    let fmt = opt_text(it, "msvCalloutPropValueFormat").unwrap_or_default();
    let format = FORMATS.iter().position(|(_, f)| *f == fmt).map_or(FORMATS.len() as i32, |k| k as i32);
    let pick = |vals: &[&str], name: &str, default: usize| opt_text(it, name).and_then(|v| vals.iter().position(|x| x.eq_ignore_ascii_case(&v))).unwrap_or(default) as i32;
    let shown = ["msvCalloutPropLabel", "msvCalloutPropLabelPosition", "msvCalloutPropValueFormat", "msvCalloutPropShowValue", "msvCalloutPropBorder", "msvCalloutPropFill", "msvCalloutPropMin", "msvCalloutPropMax"];
    let other: Vec<(String, Opt)> = it.options.iter().filter(|(n, _)| !shown.iter().any(|s| s.eq_ignore_ascii_case(n))).cloned().collect();
    let shades = it
        .shades
        .iter()
        .map(|s| {
            let fill = PALETTE.iter().position(|(_, c)| *c == s.fill);
            let text = TEXT_COLORS.iter().position(|(_, c)| *c == s.text);
            ShadeRow { value: s.value.clone(), to: s.to.clone(), fill: fill.map_or(PALETTE.len() as i32, |k| k as i32), own_fill: fill.is_none().then_some(s.fill), text: text.map_or(TEXT_COLORS.len() as i32, |k| k as i32), own_text: if text.is_none() { s.text } else { None } }
        })
        .collect();
    let lo = it.opt("msvCalloutPropMin").map(|o| o.text()).unwrap_or_else(|| String::from("0"));
    let hi = it.opt("msvCalloutPropMax").map(|o| o.text()).unwrap_or_else(|| String::from("100"));
    app.dg.dlg.form = Form {
        id: it.id,
        fields,
        field,
        expr: if label.is_none() { it.field.clone() } else { String::new() },
        kind,
        style,
        use_default: it.pos.is_none() as i32,
        h: p.h as i32,
        v: p.v as i32,
        label: opt_text(it, "msvCalloutPropLabel").unwrap_or_else(|| String::from("[Default]")),
        label_pos: pick(&LABEL_VALUES, "msvCalloutPropLabelPosition", 1),
        format,
        own_format: fmt,
        show_value: opt_flag(it, "msvCalloutPropShowValue", it.kind != Kind::DataBar || it.callout == "DG Progress bar") as i32,
        border: pick(&BORDER_VALUES, "msvCalloutPropBorder", 0),
        fill: pick(&FILL_VALUES, "msvCalloutPropFill", 0),
        min: lo,
        max: hi,
        rules: it.rules.iter().map(|r| (Op::ALL.iter().position(|&o| o == r.op).unwrap_or(0) as i32, r.a.clone(), r.b.clone())).collect(),
        ranges: it.ranges as i32,
        shades,
        icon_formula: it.icon_formula.clone(),
        other,
    };
}

// the item from the form
fn form_item(app: &Diagrammer, f: &Form) -> Option<Item> {
    let kind = KINDS[f.kind.clamp(0, 3) as usize];
    let field = match f.fields.get(f.field.max(0) as usize) {
        Some(l) => dg::field_expr(l),
        None => String::from(f.expr.trim()),
    };
    if field.is_empty() || !field.contains('{') {
        return None;
    }
    let styles = dgs::styles(app, kind);
    let callout = if kind == Kind::ColorByValue { String::new() } else { styles.get(f.style.max(0) as usize).map(|s| s.0.clone()).unwrap_or_else(|| String::from(dgs::default_callout(kind))) };
    let mut it = Item::new(kind, &field, &callout);
    it.id = f.id;
    it.pos = (f.use_default == 0).then_some(Pos { h: f.h.clamp(0, 6) as u8, v: f.v.clamp(0, 6) as u8 });
    it.options = f.other.clone();
    let format = FORMATS.get(f.format.max(0) as usize).map_or(f.own_format.clone(), |x| String::from(x.1));
    match kind {
        Kind::Text | Kind::Heading => {
            it.set_opt("msvCalloutPropLabel", Opt::Text(if f.label.trim().is_empty() { String::from("[Default]") } else { f.label.clone() }));
            it.set_opt("msvCalloutPropLabelPosition", Opt::Text(String::from(LABEL_VALUES[f.label_pos.clamp(0, 4) as usize])));
            it.set_opt("msvCalloutPropValueFormat", Opt::Text(format));
            it.set_opt("msvCalloutPropShowValue", Opt::Bool(f.show_value != 0));
            it.set_opt("msvCalloutPropBorder", Opt::Text(String::from(BORDER_VALUES[f.border.clamp(0, 2) as usize])));
            it.set_opt("msvCalloutPropFill", Opt::Text(String::from(FILL_VALUES[f.fill.clamp(0, 1) as usize])));
        }
        Kind::DataBar => {
            it.set_opt("msvCalloutPropMin", Opt::Num(dg::parse_num(&f.min).unwrap_or(0.0)));
            it.set_opt("msvCalloutPropMax", Opt::Num(dg::parse_num(&f.max).unwrap_or(100.0)));
            it.set_opt("msvCalloutPropShowValue", Opt::Bool(f.show_value != 0));
            it.set_opt("msvCalloutPropValueFormat", Opt::Text(format));
        }
        Kind::IconSet => {
            let n = dgs::icon_count(app, &callout);
            it.rules = (0..n)
                .map(|k| match f.rules.get(k) {
                    Some((op, a, b)) => Rule::new(Op::ALL[(*op).clamp(0, 9) as usize], a.trim(), b.trim()),
                    None => Rule::new(Op::NotUsed, "", ""),
                })
                .collect();
            if it.rules.iter().all(|r| r.op == Op::NotUsed) {
                it.icon_formula = f.icon_formula.clone();
                it.rules.clear();
            }
        }
        Kind::ColorByValue => {
            it.ranges = f.ranges != 0;
            it.shades = f
                .shades
                .iter()
                .filter(|s| !s.value.trim().is_empty())
                .map(|s| Shade {
                    value: String::from(s.value.trim()),
                    to: if f.ranges != 0 { String::from(s.to.trim()) } else { String::new() },
                    fill: PALETTE.get(s.fill.max(0) as usize).map_or(s.own_fill.unwrap_or(0xFFFFFF), |p| p.1),
                    text: TEXT_COLORS.get(s.text.max(0) as usize).map_or(s.own_text, |p| p.1),
                })
                .collect();
        }
    }
    Some(it)
}

// ---------------------------------------------------------------- every pass

/// the dialogs
pub(crate) fn dialogs(app: &mut Diagrammer, ui: &mut Ui) {
    edit_dialog(app, ui);
    item_dialog(app, ui);
    rename_dialog(app, ui);
}

// texts of a list in the current language (a context after '|' left out)
fn pos_names(names: &[&'static str]) -> Vec<&'static str> {
    names.iter().map(|n| tc(n)).collect()
}

fn edit_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.dg.dlg.edit.clone();
    if open.get() == 0 {
        return;
    }
    if app.opened.is_none() || app.dg.dlg.g.is_none() {
        open.set(0);
        return;
    }
    let title = if app.dg.dlg.target.is_some() { t("Datengrafik bearbeiten") } else { t("Neue Datengrafik") };
    if !ui.dialog_begin(ID_EDIT, title, &open) {
        return;
    }
    let kinds = [kind_name(Kind::Text), kind_name(Kind::DataBar), kind_name(Kind::IconSet), kind_name(Kind::ColorByValue)];
    let mut action: Option<u32> = None;
    {
        let d = &mut app.dg.dlg;
        let g = d.g.as_mut().expect("graphic");
        ui.next_size(500, 0);
        ui.box_begin(ID_EDIT + 1, BOX_V, 6);
        ui.text(TS_BODY_STRONG, &g.name);
        ui.text(TS_CAPTION | TX_SECONDARY, t("Elemente"));
        let rows: Vec<String> = g.items.iter().map(|it| dg::describe(it, &kinds)).collect();
        ui.next_size(-1, 150);
        let (a, b) = ui.list_begin(ID_EDIT + 0x10, &mut d.pick, rows.len() as i32, 28);
        for i in a..b {
            if ui.list_item(i, &rows[i as usize]) & EV_DOUBLE != 0 {
                action = Some(2);
            }
        }
        ui.list_end(&mut d.pick);
        let has = d.pick >= 0 && (d.pick as usize) < g.items.len();
        ui.box_begin(0, BOX_H, 6);
        for (k, label, on) in [(1u32, t("Neues Element ..."), true), (2, t("Element bearbeiten ..."), has), (3, tc("Löschen|Element"), has), (4, t("Nach oben"), has && d.pick > 0), (5, t("Nach unten"), has && (d.pick as usize) + 1 < g.items.len())] {
            ui.next_flags(if on { 0 } else { ST_DISABLED });
            if ui.button(ID_EDIT + 0x20 + k, label) & EV_ACT != 0 {
                action = Some(k);
            }
        }
        ui.box_end();
        ui.spacer(4);
        ui.text(TS_CAPTION | TX_SECONDARY, t("Standardposition"));
        ui.box_begin(0, BOX_H, 8);
        let (mut h, mut v) = (g.default_pos.h as i32, g.default_pos.v as i32);
        ui.next_size(140, 0);
        ui.text(TS_BODY, tc("Horizontal|Position"));
        ui.next_size(160, 0);
        ui.combobox(ID_EDIT + 0x30, &mut h, &pos_names(&H_NAMES));
        ui.box_end();
        ui.box_begin(0, BOX_H, 8);
        ui.next_size(140, 0);
        ui.text(TS_BODY, tc("Vertikal|Position"));
        ui.next_size(160, 0);
        ui.combobox(ID_EDIT + 0x31, &mut v, &pos_names(&V_NAMES));
        ui.box_end();
        g.default_pos = Pos { h: h.clamp(0, 6) as u8, v: v.clamp(0, 6) as u8 };
        ui.spacer(4);
        ui.text(TS_CAPTION | TX_SECONDARY, t("Anzeigeoptionen"));
        let mut hide = g.hide_text as i32;
        ui.checkbox(ID_EDIT + 0x32, t("Shape-Text ausblenden"), &mut hide);
        g.hide_text = hide != 0;
        let mut border = g.show_border as i32;
        ui.checkbox(ID_EDIT + 0x33, t("Rahmen um Elemente an der Standardposition"), &mut border);
        g.show_border = border != 0;
        ui.box_end();
    }
    let r = ui.dialog_buttons(Some(t("OK")), Some(t("Übernehmen")), Some(t("Abbrechen")));
    ui.dialog_end();
    match action {
        Some(1) => open_item(app, ui, None),
        Some(2) => {
            let k = app.dg.dlg.pick.max(0) as usize;
            open_item(app, ui, Some(k));
        }
        Some(3) => {
            let d = &mut app.dg.dlg;
            if let Some(g) = d.g.as_mut() {
                if (d.pick as usize) < g.items.len() {
                    g.items.remove(d.pick as usize);
                    d.pick = d.pick.min(g.items.len() as i32 - 1);
                }
            }
            ui.repass();
        }
        Some(4) | Some(5) => {
            let d = &mut app.dg.dlg;
            if let Some(g) = d.g.as_mut() {
                let k = d.pick as usize;
                let to = if action == Some(4) { k - 1 } else { k + 1 };
                if to < g.items.len() {
                    g.items.swap(k, to);
                    d.pick = to as i32;
                }
            }
            ui.repass();
        }
        _ => {}
    }
    match r {
        DLG_PRIMARY => {
            write_graphic(app, ui);
        }
        DLG_SECONDARY => {
            write_graphic(app, ui);
            app.dg.dlg.edit.set(1);
        }
        _ => {}
    }
}

// OK / Übernehmen: the graphic written; a new one goes onto the selected shapes
fn write_graphic(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(g) = app.dg.dlg.g.clone() else { return };
    let target = app.dg.dlg.target;
    let sel: Vec<_> = if target.is_none() { dgs::targets(app).into_iter().filter(|_| !crate::edit_canvas::selected(app).is_empty()).collect() } else { Vec::new() };
    let src = dgs::source(app).cloned();
    let Some(o) = app.opened.as_mut() else { return };
    let layer = dgs::layer_name();
    let label = if target.is_some() { t("Datengrafik bearbeiten") } else { t("Neue Datengrafik") };
    let r = edit_doc::edit(o, label, true, |tx, eng| match target {
        Some(m) => {
            dg::update(tx, eng, m, &g, src.as_ref(), layer)?;
            Ok(m)
        }
        None => {
            let m = dg::create(tx, eng, &g, src.as_ref())?;
            if !sel.is_empty() {
                dg::apply(tx, eng, &sel, m, layer)?;
            }
            Ok(m)
        }
    });
    if let Ok(m) = r {
        app.dg.dlg.target = Some(m);
        if let Some(o) = app.opened.as_ref() {
            app.dg.dlg.g = dg::read(&o.doc, m);
        }
    }
    dgs::report(ui, r);
}

fn item_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.dg.dlg.item.clone();
    if open.get() == 0 {
        return;
    }
    if app.opened.is_none() {
        open.set(0);
        return;
    }
    let title = if app.dg.dlg.index.is_none() && !app.dg.dlg.from_ribbon { t("Neues Element") } else { t("Element bearbeiten") };
    if !ui.dialog_begin(ID_ITEM, title, &open) {
        return;
    }
    let kind_names = [kind_name(Kind::Text), kind_name(Kind::DataBar), kind_name(Kind::IconSet), kind_name(Kind::ColorByValue)];
    let kind = KINDS[app.dg.dlg.form.kind.clamp(0, 3) as usize];
    let styles = dgs::styles(app, kind);
    let callout = styles.get(app.dg.dlg.form.style.max(0) as usize).map(|s| s.0.clone()).unwrap_or_default();
    let icons = dgs::icon_count(app, &callout);
    let values = app.dg.dlg.form.fields.get(app.dg.dlg.form.field.max(0) as usize).map(|f| dgs::values_of(app, f)).unwrap_or_default();
    {
        let f = &mut app.dg.dlg.form;
        ui.next_size(520, 0);
        ui.box_begin(ID_ITEM + 1, BOX_V, 6);
        // Datenfeld, Anzeigen als, Stil
        let mut names: Vec<&str> = f.fields.iter().map(|s| s.as_str()).collect();
        let more = t("Weitere Felder");
        names.push(more);
        row(ui, t("Datenfeld"), |ui| {
            ui.combobox(ID_ITEM + 0x10, &mut f.field, &names);
        });
        if f.field as usize >= f.fields.len() {
            row(ui, t("Ausdruck"), |ui| {
                ui.textbox(ID_ITEM + 0x11, Some("{Feld}*2"), &mut f.expr, 256);
            });
            ui.tooltip(t("Feldnamen in geschweiften Klammern, verbunden mit + - * /"));
        }
        let before = f.kind;
        row(ui, t("Anzeigen als"), |ui| {
            ui.combobox(ID_ITEM + 0x12, &mut f.kind, &kind_names);
        });
        if f.kind != before {
            f.style = 0;
            ui.repass();
        }
        if kind != Kind::ColorByValue {
            let names: Vec<&str> = styles.iter().map(|s| s.1.as_str()).collect();
            row(ui, t("Stil"), |ui| {
                ui.combobox(ID_ITEM + 0x13, &mut f.style, &names);
            });
            // Position
            ui.spacer(2);
            ui.text(TS_CAPTION | TX_SECONDARY, t("Position"));
            ui.checkbox(ID_ITEM + 0x14, t("Standardposition verwenden"), &mut f.use_default);
            let off = if f.use_default != 0 { ST_DISABLED } else { 0 };
            row(ui, tc("Horizontal|Position"), |ui| {
                ui.next_flags(off);
                ui.combobox(ID_ITEM + 0x15, &mut f.h, &pos_names(&H_NAMES));
            });
            row(ui, tc("Vertikal|Position"), |ui| {
                ui.next_flags(off);
                ui.combobox(ID_ITEM + 0x16, &mut f.v, &pos_names(&V_NAMES));
            });
        }
        ui.spacer(2);
        ui.text(TS_CAPTION | TX_SECONDARY, t("Details"));
        let mut formats: Vec<String> = FORMATS.iter().map(|(n, _)| String::from(t(n))).collect();
        if f.format as usize >= FORMATS.len() {
            formats.push(tf("Eigenes: {}", &[&f.own_format]));
        }
        let formats: Vec<&str> = formats.iter().map(|s| s.as_str()).collect();
        match kind {
            Kind::Text | Kind::Heading => {
                row(ui, t("Bezeichnung"), |ui| {
                    ui.textbox(ID_ITEM + 0x20, None, &mut f.label, 128);
                });
                ui.tooltip(t("[Default]: die Bezeichnung des Datenfelds"));
                row(ui, t("Position der Bezeichnung"), |ui| {
                    ui.combobox(ID_ITEM + 0x21, &mut f.label_pos, &pos_names(&LABEL_NAMES));
                });
                row(ui, t("Wertformat"), |ui| {
                    ui.combobox(ID_ITEM + 0x22, &mut f.format, &formats);
                });
                ui.checkbox(ID_ITEM + 0x23, t("Wert anzeigen"), &mut f.show_value);
                row(ui, tc("Rahmen|Text"), |ui| {
                    ui.combobox(ID_ITEM + 0x24, &mut f.border, &pos_names(&BORDER_NAMES));
                });
                row(ui, tc("Füllung|Text"), |ui| {
                    ui.combobox(ID_ITEM + 0x25, &mut f.fill, &pos_names(&FILL_NAMES));
                });
            }
            Kind::DataBar => {
                row(ui, t("Minimalwert"), |ui| {
                    ui.textbox(ID_ITEM + 0x30, None, &mut f.min, 32);
                });
                row(ui, t("Maximalwert"), |ui| {
                    ui.textbox(ID_ITEM + 0x31, None, &mut f.max, 32);
                });
                ui.checkbox(ID_ITEM + 0x32, t("Wert anzeigen"), &mut f.show_value);
                row(ui, t("Wertformat"), |ui| {
                    ui.combobox(ID_ITEM + 0x33, &mut f.format, &formats);
                });
            }
            Kind::IconSet => {
                ui.text(TS_BODY, t("Regeln für die Symbole"));
                if icons == 0 {
                    ui.text(TS_CAPTION | TX_SECONDARY, t("Der Stil hat keine Symbole."));
                }
                while f.rules.len() < icons {
                    f.rules.push((0, String::new(), String::new()));
                }
                let ops: Vec<&str> = pos_names(&OP_NAMES);
                for k in 0..icons {
                    let (op, a, b) = &mut f.rules[k];
                    ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
                    ui.next_size(70, 0);
                    ui.text(TS_BODY, &tf("Symbol {}", &[&format!("{}", k + 1)]));
                    ui.next_size(190, 0);
                    ui.combobox(ID_ITEM + 0x40 + k as u32 * 4, op, &ops);
                    ui.next_size(110, 0);
                    ui.textbox(ID_ITEM + 0x41 + k as u32 * 4, None, a, 64);
                    if *op == 7 {
                        ui.next_size(90, 0);
                        ui.textbox(ID_ITEM + 0x42 + k as u32 * 4, None, b, 64);
                    }
                    ui.box_end();
                }
                if !values.is_empty() {
                    ui.next_size(-1, 0);
                    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &tf("Werte des Felds: {}", &[&values.join(", ")]));
                }
            }
            Kind::ColorByValue => {
                let methods = [t("Jeder Wert eine Farbe"), t("Jeder Bereich eine Farbe")];
                row(ui, t("Färbemethode"), |ui| {
                    ui.combobox(ID_ITEM + 0x50, &mut f.ranges, &methods);
                });
                let fills: Vec<&str> = PALETTE.iter().map(|(n, _)| t(n)).collect();
                let texts: Vec<&str> = TEXT_COLORS.iter().map(|(n, _)| t(n)).collect();
                let mut remove = None;
                for (k, s) in f.shades.iter_mut().enumerate() {
                    let id = ID_ITEM + 0x100 + k as u32 * 8;
                    ui.box_begin(0, BOX_H | BOX_XCENTER, 6);
                    ui.next_size(if f.ranges != 0 { 70 } else { 140 }, 0);
                    ui.textbox(id, Some(t("Wert")), &mut s.value, 64);
                    if f.ranges != 0 {
                        ui.next_size(70, 0);
                        ui.textbox(id + 1, Some(t("bis")), &mut s.to, 64);
                    }
                    let mut fl = fills.clone();
                    let own = s.own_fill.map(|c| tf("Eigene: {}", &[&dg::hex(c)]));
                    if let Some(o) = &own {
                        fl.push(o.as_str());
                    }
                    ui.next_size(120, 0);
                    ui.combobox(id + 2, &mut s.fill, &fl);
                    let mut tl = texts.clone();
                    let own_t = s.own_text.map(|c| tf("Eigene: {}", &[&dg::hex(c)]));
                    if let Some(o) = &own_t {
                        tl.push(o.as_str());
                    }
                    ui.next_size(110, 0);
                    ui.combobox(id + 3, &mut s.text, &tl);
                    if ui.button(id + 4, tc("Löschen|Farbe")) & EV_ACT != 0 {
                        remove = Some(k);
                    }
                    ui.box_end();
                }
                if let Some(k) = remove {
                    f.shades.remove(k);
                    ui.repass();
                }
                ui.box_begin(0, BOX_H, 6);
                if ui.button(ID_ITEM + 0x51, t("Farbe hinzufügen")) & EV_ACT != 0 {
                    let k = f.shades.len();
                    f.shades.push(ShadeRow { fill: (k % PALETTE.len()) as i32, ..ShadeRow::default() });
                    ui.repass();
                }
                ui.next_flags(if values.is_empty() { ST_DISABLED } else { 0 });
                if ui.button(ID_ITEM + 0x52, t("Werte der Shapes einsetzen")) & EV_ACT != 0 {
                    f.ranges = 0;
                    f.shades = values.iter().enumerate().map(|(k, v)| ShadeRow { value: v.clone(), fill: (k % PALETTE.len()) as i32, ..ShadeRow::default() }).collect();
                    ui.repass();
                }
                ui.box_end();
            }
        }
        ui.box_end();
    }
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    let from_ribbon = app.dg.dlg.from_ribbon;
    match r {
        DLG_PRIMARY => {
            let form = app.dg.dlg.form.clone();
            match form_item(app, &form) {
                Some(it) if from_ribbon => {
                    let id = it.id;
                    change(app, ui, t("Element bearbeiten"), |g| match g.items.iter().position(|x| x.id == id) {
                        Some(k) => g.items[k] = it,
                        None => {
                            g.push(it);
                        }
                    });
                }
                Some(it) => {
                    let d = &mut app.dg.dlg;
                    if let Some(g) = d.g.as_mut() {
                        match d.index.filter(|&k| k < g.items.len()) {
                            Some(k) => g.items[k] = Item { id: g.items[k].id, ..it },
                            None => {
                                d.pick = g.push(it) as i32;
                            }
                        }
                    }
                }
                None => {
                    ui.announce(t("Das Element braucht ein Datenfeld."), LIVE_POLITE);
                }
            }
            if !from_ribbon {
                app.dg.dlg.edit.set(1);
            }
        }
        DLG_CLOSE if !from_ribbon => app.dg.dlg.edit.set(1),
        _ => {}
    }
    if r != DLG_NONE {
        ui.repass();
    }
}

// a labelled row of the dialogs: the label, then the control
fn row(ui: &mut Ui, label: &str, f: impl FnOnce(&mut Ui)) {
    ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
    ui.next_size(170, 0);
    ui.text(TS_BODY, label);
    ui.next_size(260, 0);
    f(ui);
    ui.box_end();
}

fn rename_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.dg.dlg.rename.clone();
    if open.get() == 0 {
        return;
    }
    if !ui.dialog_begin(ID_RENAME, t("Datengrafik umbenennen"), &open) {
        return;
    }
    ui.next_size(320, 0);
    ui.box_begin(ID_RENAME + 1, BOX_V, 6);
    ui.text(TS_BODY, t("Name"));
    ui.next_size(-1, 0);
    ui.textbox(ID_RENAME + 2, None, &mut app.dg.dlg.name, 128);
    ui.box_end();
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        let name = String::from(app.dg.dlg.name.trim());
        let Some(m) = app.dg.current.take() else { return };
        let Some(o) = app.opened.as_mut() else { return };
        let r = edit_doc::edit(o, t("Datengrafik umbenennen"), true, |tx, _| dg::rename(tx, m, &name));
        if r == Err(diagram_core::EditError::Exists) {
            ui.announce(&tf("Es gibt schon eine Datengrafik „{}“.", &[&name]), LIVE_POLITE);
            app.dg.current = Some(m);
            app.dg.dlg.rename.set(1);
        } else {
            dgs::report(ui, r);
        }
    }
}
