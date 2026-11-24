// select_type -- (JD8 rest) Start › Bearbeiten › Markieren › Nach Typ auswählen (RS-31): the
// top-level shapes of the page by kind, by layer or by style
//
//   SelectState                 the dialog's choices
//   open(app, ui)               SELECT_BY_TYPE: the dialog for the page shown
//   dialog(app, ui)             every pass: Auswahl nach Shapetyp (Shapes, Gruppen,
//                               Führungslinien, Bilder, Verbinder), nach Ebene (the page's layers
//                               and "Keine Ebene"), nach Formatvorlage (the document's styles);
//                               OK selects every matching top-level shape that can be selected
//   matches(doc, s, ...)        the rule per shape
//
// The kinds as Visio's Select by Type names them: groups (Type Group), pictures (Type Foreign:
// bitmaps, metafiles, OLE objects), connectors (1-D shapes: dynamic connectors and lines),
// guides (Type Guide), shapes (the rest).  A style matches when it is the shape's line, fill or
// text style (its own, else its master shape's).  Shapes on hidden or locked layers are not in
// the canvas' model as selectable ones and are left out.  (JD15) Guides are chosen by kind
// only, while Ansicht › Führungslinien shows them; selected they are drawn in the accent colour,
// move with the arrow keys and go with Del (a click on the canvas does not select them yet:
// SP-10).

use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{glue, Document, Owner, PageRef, ShapeKind, ShapeRef, StyleRefs};
use ui::i18n::t;
use ui::*;

use crate::edit_canvas::shape_of;
use crate::Diagrammer;

const ID_DLG: u32 = 0x8400;

/// the kinds of Nach Shapetyp
pub(crate) const K_SHAPES: usize = 0;
pub(crate) const K_GROUPS: usize = 1;
pub(crate) const K_GUIDES: usize = 2;
pub(crate) const K_PICTURES: usize = 3;
pub(crate) const K_CONNECTORS: usize = 4;
// i18n
const KINDS: [&str; 5] = ["Shapes", "Gruppen", "Führungslinien", "Bilder", "Verbinder"];

/// the dialog's choices
pub(crate) struct SelectState {
    pub dlg: Shared<i32>,
    /// 0 by kind, 1 by layer, 2 by style
    pub mode: i32,
    pub kinds: [i32; 5],
    /// the page's layers: index, name, chosen; then "Keine Ebene"
    layers: Vec<(u32, String, i32)>,
    none_layer: i32,
    /// the document's styles: ID, name, chosen
    styles: Vec<(u32, String, i32)>,
    page: Option<PageRef>,
}

impl Default for SelectState {
    fn default() -> SelectState {
        SelectState { dlg: Shared::new(0), mode: 0, kinds: [1, 1, 0, 1, 1], layers: Vec::new(), none_layer: 0, styles: Vec::new(), page: None }
    }
}

/// Nach Typ auswählen: the dialog
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui) {
    let (Some(o), Some(page)) = (app.opened.as_ref(), app.model.page) else { return };
    let doc = &o.doc;
    let st = &mut app.select;
    // the choices of the last time stay, by name
    let was = |list: &[(u32, String, i32)], name: &str| list.iter().find(|x| x.1 == name).map_or(0, |x| x.2);
    st.layers = doc
        .layers(page)
        .into_iter()
        .filter_map(|l| match l.key() {
            diagram_core::RowKey::Index(ix) => {
                let name = String::from(l.text("Name").or_else(|| l.text("NameUniv")).unwrap_or(""));
                let on = was(&st.layers, &name);
                Some((ix, name, on))
            }
            _ => None,
        })
        .collect();
    st.styles = doc
        .styles()
        .filter_map(|s| doc.style(s))
        .map(|s| {
            let name = String::from(s.header.name.or(s.header.name_u).map_or("", |n| doc.str(n)));
            let on = was(&st.styles, &name);
            (s.header.id, name, on)
        })
        .collect();
    st.page = Some(page);
    st.dlg.set(1);
    ui.repass();
}

// a shape's styles: its own, else its master shape's
fn styles_of(doc: &Document, s: ShapeRef) -> StyleRefs {
    let mut cur = Some(s);
    let mut out = StyleRefs::default();
    let mut depth = 0;
    while let (Some(c), true) = (cur, depth < 32) {
        let Some(sh) = doc.shape(c) else { break };
        let h = sh.header().styles;
        out.line = out.line.or(h.line);
        out.fill = out.fill.or(h.fill);
        out.text = out.text.or(h.text);
        cur = doc.master_shape_of(c);
        depth += 1;
    }
    out
}

/// the kind of a shape (K_*)
pub(crate) fn kind_of(doc: &Document, s: ShapeRef) -> usize {
    match doc.shape(s).map(|x| x.header().kind) {
        Some(ShapeKind::Group) => K_GROUPS,
        Some(ShapeKind::Foreign) => K_PICTURES,
        Some(ShapeKind::Guide) => K_GUIDES,
        _ if glue::is_one_d(doc, s) => K_CONNECTORS,
        _ => K_SHAPES,
    }
}

/// whether shape `s` matches the dialog's choice
pub(crate) fn matches(doc: &Document, s: ShapeRef, st: &SelectState) -> bool {
    match st.mode {
        0 => st.kinds[kind_of(doc, s)] != 0,
        1 => {
            let m = doc.layer_members(s);
            (m.is_empty() && st.none_layer != 0) || st.layers.iter().any(|(ix, _, on)| *on != 0 && m.contains(ix))
        }
        _ => {
            let r = styles_of(doc, s);
            st.styles.iter().any(|(id, _, on)| *on != 0 && [r.line, r.fill, r.text].contains(&Some(*id)))
        }
    }
}

/// every pass: the dialog while it is open
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let dlg = app.select.dlg.clone();
    if !ui.dialog_begin(ID_DLG, t("Nach Typ auswählen"), &dlg) {
        return;
    }
    let st = &mut app.select;
    ui.next_size(380, 0);
    ui.box_begin(ID_DLG + 1, BOX_V, 6);
    ui.text(TS_BODY_STRONG, t("Auswahl nach"));
    ui.radio(ID_DLG + 2, t("Shapetyp"), &mut st.mode, 0);
    ui.radio(ID_DLG + 3, t("Ebene"), &mut st.mode, 1);
    ui.radio(ID_DLG + 4, t("Formatvorlage"), &mut st.mode, 2);
    ui.separator();
    match st.mode {
        0 => {
            for (k, name) in KINDS.iter().enumerate() {
                ui.checkbox(ID_DLG + 8 + k as u32, t(name), &mut st.kinds[k]);
            }
        }
        1 => {
            for (k, (_, name, on)) in st.layers.iter_mut().enumerate() {
                ui.checkbox(ID_DLG + 0x20 + k as u32 % 0xE0, name, on);
            }
            ui.checkbox(ID_DLG + 16, t("Keine Ebene"), &mut st.none_layer);
        }
        _ => {
            ui.next_size(-1, 220);
            ui.scroll_begin(ID_DLG + 17, 0);
            for (k, (_, name, on)) in st.styles.iter_mut().enumerate() {
                ui.checkbox(ID_DLG + 0x100 + (k as u32 & 0xFF), name, on);
            }
            ui.scroll_end();
        }
    }
    ui.box_end();
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        select(app, ui);
    }
    if r != DLG_NONE {
        app.select.dlg.set(0);
        ui.repass();
    }
}

// select the matching top-level shapes the canvas lets select
fn select(app: &mut Diagrammer, ui: &mut Ui) {
    let (Some(o), Some(page)) = (app.opened.as_ref(), app.model.page) else { return };
    if app.select.page != Some(page) {
        return;
    }
    let doc = &o.doc;
    let mut keys: Vec<u64> = app
        .model
        .shapes
        .iter()
        .filter(|s| s.parent == 0 && s.flags & SEF_NOSELECT == 0)
        .filter(|s| {
            let r = shape_of(page, s.key);
            r.owner == Owner::Page(page) && matches(doc, r, &app.select)
        })
        .map(|s| s.key)
        .collect();
    // (JD15) the guides shown, by kind only (edit_canvas.rs keeps a selected guide selectable)
    if app.select.mode == 0 && app.select.kinds[K_GUIDES] != 0 && app.view_guides {
        let guides: Vec<u64> = app.model.guides.iter().map(|g| g.key).filter(|k| !keys.contains(k)).collect();
        keys.extend(guides);
    }
    app.se.select(&keys);
    ui.repass();
}
