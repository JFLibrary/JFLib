// groups_cmds -- (JD21) the group commands: Start › Anordnen › Gruppieren › Zu Gruppe hinzufügen,
// Aus Gruppe entfernen, Gruppe öffnen (RS-27, SP-15), Gruppierung aufheben with Visio's warning
// (SP-06), the dialog Verhalten for groups and shapes (SS-17)
//
//   GroupsState                 the open group (groups_view.rs), the warning, the dialog
//   available(app, id)          whether a command of this module can run now
//   add(app, ui) / remove(app, ui) / open(app, ui) / close(app, ui) / behavior(app, ui)
//   ungroup(app, ui)            Gruppierung aufheben: asks first when ungrouping loses something
//                               (diagram_core::groups::ungroup_warning: an instance's link to its
//                               master, formulas that name the group), then takes the groups apart
//                               (an instance is made plain first: instance::detach)
//   dialogs(app, ui)            every pass: the warning, Verhalten
//
// Zu Gruppe hinzufügen takes the group that is the primary selection, else the only group of the
// selection (Visio's AddToGroup), and the other selected shapes; afterwards the group is
// selected.  Aus Gruppe entfernen takes the sub-selected members out; they stay selected.
// Verhalten shows Visio's group part of the Behavior dialog (Auswahl, Gruppendaten, An
// Elementshapes ausrichten, Gruppentext bearbeiten, Abgelegte Shapes übernehmen), DontMoveChildren
// (Elemente nicht mit der Maus verschieben, a cell the Behavior dialog lacks), the double click
// (Gruppe öffnen), and for every shape Visio's Verschiedenes › Shape bei Ablage zu Gruppen
// hinzufügen (IsDropSource).

use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::groups::{self, Props, SelectMode, Warning};
use diagram_core::{instance, ops, EditError, PageRef, ShapeRef};
use ui::i18n::{t, tf};
use ui::*;

use crate::commands::{self, *};
use crate::edit_canvas::{self, key_of, selected};
use crate::{edit_doc, Diagrammer};

const ID_WARN: u32 = 0x7A11_0000;
const ID_BEHAVIOR: u32 = 0x7A12_0000;

/// the state of the group commands
pub(crate) struct GroupsState {
    /// the group open on the canvas: drawing serial, page, group (groups_view.rs)
    pub view: Option<(u32, PageRef, ShapeRef)>,
    /// Gruppierung aufheben's warning: open, the text, the groups waiting for it
    pub warn: Shared<i32>,
    warn_text: String,
    pending: Vec<ShapeRef>,
    /// Verhalten: open, the shapes, whether a group is among them, the values shown
    pub behavior: Shared<i32>,
    targets: Vec<ShapeRef>,
    group: Option<ShapeRef>,
    select_mode: i32,
    display: i32,
    snap: i32,
    text_edit: i32,
    drop_target: i32,
    dont_move: i32,
    double_open: i32,
    drop_source: i32,
    /// Shape ändern: the stencil whose masters the gallery shows unfolded, the gallery to open
    /// again after a stencil row was clicked (change_shape.rs)
    pub change_open: Option<u32>,
    pub change_reopen: bool,
}

impl Default for GroupsState {
    fn default() -> GroupsState {
        GroupsState {
            view: None,
            warn: Shared::new(0),
            warn_text: String::new(),
            pending: Vec::new(),
            behavior: Shared::new(0),
            targets: Vec::new(),
            group: None,
            select_mode: 1,
            display: 2,
            snap: 1,
            text_edit: 1,
            drop_target: 0,
            dont_move: 0,
            double_open: 0,
            drop_source: 0,
            change_open: None,
            change_reopen: false,
        }
    }
}

// the group a selection adds to: the primary selection when it is a group, else the only group
fn add_target(app: &Diagrammer) -> Option<(ShapeRef, Vec<ShapeRef>)> {
    let o = app.opened.as_ref()?;
    let page = app.model.page?;
    let sel = selected(app);
    if sel.len() < 2 {
        return None;
    }
    let primary = edit_canvas::shape_of(page, app.se.primary);
    let g = if sel.contains(&primary) && groups::is_group(&o.doc, primary) {
        primary
    } else {
        let gs: Vec<ShapeRef> = sel.iter().copied().filter(|&s| groups::is_group(&o.doc, s)).collect();
        match gs.as_slice() {
            [g] => *g,
            _ => return None,
        }
    };
    let rest: Vec<ShapeRef> = sel.into_iter().filter(|&s| s != g).collect();
    Some((g, rest))
}

// the selected shapes that can leave their group
fn removable(app: &Diagrammer) -> Vec<ShapeRef> {
    let Some(o) = app.opened.as_ref() else { return Vec::new() };
    selected(app)
        .into_iter()
        .filter(|&s| o.doc.shape(s).is_some_and(|x| x.parent().is_some() && !(x.header().master_shape.is_some() && x.header().master.is_none())))
        .collect()
}

fn single_group(app: &Diagrammer) -> Option<ShapeRef> {
    let o = app.opened.as_ref()?;
    match selected(app).as_slice() {
        [g] if groups::is_group(&o.doc, *g) => Some(*g),
        _ => None,
    }
}

/// whether command `id` can run now (true for commands of other modules)
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    match id {
        ADD_TO_GROUP => add_target(app).is_some(),
        REMOVE_FROM_GROUP => !removable(app).is_empty() && removable(app).len() == app.se.sel.len(),
        OPEN_GROUP => single_group(app).is_some(),
        CLOSE_GROUP => crate::groups_view::scope(app).is_some(),
        GROUP_BEHAVIOR => app.opened.is_some() && !app.se.sel.is_empty(),
        CHANGE_SHAPE => crate::change_shape::available(app),
        _ => true,
    }
}

/// Zu Gruppe hinzufügen
pub(crate) fn add(app: &mut Diagrammer, ui: &mut Ui) {
    let Some((g, rest)) = add_target(app) else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let owner = g.owner;
    let r = edit_doc::edit(o, commands::label(ADD_TO_GROUP), true, |tx, eng| {
        let added = groups::add_to_group(tx, eng, g, &rest)?;
        if added.is_empty() {
            return Err(EditError::Invalid);
        }
        edit_canvas::follow(tx, eng, owner, &[g])
    });
    if r.is_ok() {
        app.se.select(&[key_of(g)]);
    }
    ui.repass();
}

/// Aus Gruppe entfernen
pub(crate) fn remove(app: &mut Diagrammer, ui: &mut Ui) {
    let shapes = removable(app);
    let Some(o) = app.opened.as_mut() else { return };
    if shapes.is_empty() {
        return;
    }
    let owner = shapes[0].owner;
    let r = edit_doc::edit(o, commands::label(REMOVE_FROM_GROUP), true, |tx, eng| {
        let moved = groups::remove_from_group(tx, eng, &shapes)?;
        edit_canvas::follow(tx, eng, owner, &moved)?;
        Ok(moved)
    });
    if let Ok(moved) = r {
        let keys: Vec<u64> = moved.iter().map(|&s| key_of(s)).collect();
        app.se.select(&keys);
    }
    ui.repass();
}

/// Gruppe öffnen
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui) {
    if let Some(g) = single_group(app) {
        crate::groups_view::open(app, ui, g);
    }
}

/// Gruppe schließen
pub(crate) fn close(app: &mut Diagrammer, ui: &mut Ui) {
    crate::groups_view::close(app, ui);
}

/// Gruppierung aufheben: the warning first when something would be lost
pub(crate) fn ungroup(app: &mut Diagrammer, ui: &mut Ui) {
    let shapes = selected(app);
    let Some(o) = app.opened.as_ref() else { return };
    let mut master = false;
    let mut formulas = 0;
    for &s in &shapes {
        match groups::ungroup_warning(&o.doc, s) {
            Some(Warning::Master) => master = true,
            Some(Warning::Formulas(n)) => formulas += n,
            None => {}
        }
    }
    if !master && formulas == 0 {
        run_ungroup(app, ui, shapes);
        return;
    }
    let mut text = String::new();
    if master {
        text.push_str(t("Durch diese Aktion wird die Verknüpfung des Objekts mit dem Master getrennt. Dadurch kann sich das Objekt anders verhalten."));
    }
    if formulas > 0 {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&tf("{} Formeln der Elemente beziehen sich auf die Gruppe; sie behalten nur ihre Werte.", &[&alloc::format!("{formulas}")]));
    }
    let g = &mut app.groups;
    g.warn_text = text;
    g.pending = shapes;
    g.warn.set(1);
    ui.repass();
}

fn run_ungroup(app: &mut Diagrammer, ui: &mut Ui, shapes: Vec<ShapeRef>) {
    let Some(o) = app.opened.as_mut() else { return };
    let r = edit_doc::edit(o, commands::label(UNGROUP), true, |tx, eng| {
        let mut all = Vec::new();
        for &s in &shapes {
            if groups::is_group(tx.doc(), s) {
                if tx.doc().shape(s).is_some_and(|x| x.header().master.is_some()) {
                    instance::detach(tx, s)?;
                }
                all.extend(ops::ungroup(tx, eng, s)?);
            } else {
                all.push(s);
            }
        }
        Ok(all)
    });
    if let Ok(kids) = r {
        let keys: Vec<u64> = kids.iter().map(|&s| key_of(s)).collect();
        app.se.select(&keys);
    }
    ui.repass();
}

/// Verhalten: the dialog for the selection (its first group, IsDropSource of every shape)
pub(crate) fn behavior(app: &mut Diagrammer, ui: &mut Ui) {
    let shapes = selected(app);
    let Some(o) = app.opened.as_ref() else { return };
    if shapes.is_empty() {
        return;
    }
    let doc = &o.doc;
    let group = shapes.iter().copied().find(|&s| groups::is_group(doc, s));
    let p = group.map_or(Props::default(), |g| groups::props(doc, g));
    let dbl = group.is_some_and(|g| groups::double_click_opens(doc, g));
    let src = shapes.iter().all(|&s| groups::is_drop_source(doc, s));
    let b = &mut app.groups;
    b.targets = shapes;
    b.group = group;
    b.select_mode = p.select_mode.code() as i32;
    b.display = p.display_mode as i32;
    b.snap = p.snap_target as i32;
    b.text_edit = p.text_edit_target as i32;
    b.drop_target = p.drop_target as i32;
    b.dont_move = p.dont_move_children as i32;
    b.double_open = dbl as i32;
    b.drop_source = src as i32;
    b.behavior.set(1);
    ui.repass();
}

/// every pass: the warning, Verhalten
pub(crate) fn dialogs(app: &mut Diagrammer, ui: &mut Ui) {
    if core::mem::take(&mut app.groups.change_reopen) {
        ui.popup_open(CHANGE_SHAPE);
    }
    warning(app, ui);
    behavior_dialog(app, ui);
}

fn warning(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.groups.warn.clone();
    if !ui.dialog_begin(ID_WARN, commands::label(UNGROUP), &open) {
        return;
    }
    ui.next_size(380, 0);
    ui.text(TS_BODY | TX_WRAP, &app.groups.warn_text);
    ui.next_size(380, 0);
    ui.text(TS_BODY, t("Möchten Sie den Vorgang fortsetzen?"));
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r != DLG_NONE {
        app.groups.warn.set(0);
        let shapes = core::mem::take(&mut app.groups.pending);
        if r == DLG_PRIMARY {
            run_ungroup(app, ui, shapes);
        }
        ui.repass();
    }
}

// i18n
const SELECT_MODES: [&str; 3] = ["Nur Gruppe", "Gruppe zuerst", "Elemente zuerst"];
// i18n
const DISPLAY_MODES: [&str; 3] = ["Ausblenden", "Hinter Elementshapes", "Vor Elementshapes"];

fn behavior_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.groups.behavior.clone();
    if !ui.dialog_begin(ID_BEHAVIOR, commands::label(GROUP_BEHAVIOR), &open) {
        return;
    }
    let b = &mut app.groups;
    let w = 360;
    if b.group.is_some() {
        ui.next_size(w, 0);
        ui.text(TS_SUBTITLE, t("Gruppenverhalten"));
        ui.next_size(w, 0);
        ui.text(TS_BODY, t("Auswahl"));
        let modes: Vec<&str> = SELECT_MODES.iter().map(|m| t(m)).collect();
        ui.next_size(w, 0);
        ui.combobox(ID_BEHAVIOR + 1, &mut b.select_mode, &modes);
        ui.next_size(w, 0);
        ui.text(TS_BODY, t("Gruppendaten"));
        let modes: Vec<&str> = DISPLAY_MODES.iter().map(|m| t(m)).collect();
        ui.next_size(w, 0);
        ui.combobox(ID_BEHAVIOR + 2, &mut b.display, &modes);
        ui.next_size(w, 0);
        ui.checkbox(ID_BEHAVIOR + 3, t("An Elementshapes ausrichten"), &mut b.snap);
        ui.next_size(w, 0);
        ui.checkbox(ID_BEHAVIOR + 4, t("Gruppentext bearbeiten"), &mut b.text_edit);
        ui.next_size(w, 0);
        ui.checkbox(ID_BEHAVIOR + 5, t("Abgelegte Shapes übernehmen"), &mut b.drop_target);
        ui.next_size(w, 0);
        ui.checkbox(ID_BEHAVIOR + 6, t("Elemente nicht mit der Maus verschieben"), &mut b.dont_move);
        ui.next_size(w, 0);
        ui.checkbox(ID_BEHAVIOR + 7, t("Doppelklick öffnet die Gruppe"), &mut b.double_open);
    }
    ui.next_size(w, 0);
    ui.text(TS_SUBTITLE, t("Verschiedenes"));
    ui.next_size(w, 0);
    ui.checkbox(ID_BEHAVIOR + 8, t("Shape bei Ablage zu Gruppen hinzufügen"), &mut b.drop_source);
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r != DLG_NONE {
        app.groups.behavior.set(0);
        if r == DLG_PRIMARY {
            apply_behavior(app);
        }
        ui.repass();
    }
}

fn apply_behavior(app: &mut Diagrammer) {
    let b = &app.groups;
    let (targets, group) = (b.targets.clone(), b.group);
    let p = Props {
        select_mode: SelectMode::of(b.select_mode as i64),
        display_mode: b.display.clamp(0, 2) as u8,
        drop_target: b.drop_target != 0,
        snap_target: b.snap != 0,
        text_edit_target: b.text_edit != 0,
        dont_move_children: b.dont_move != 0,
    };
    let (dbl, src) = (b.double_open != 0, b.drop_source != 0);
    let Some(o) = app.opened.as_mut() else { return };
    let _ = edit_doc::edit(o, commands::label(GROUP_BEHAVIOR), true, |tx, eng| {
        if let Some(g) = group.filter(|&g| tx.doc().shape(g).is_some()) {
            groups::set_props(tx, eng, g, &p)?;
            groups::set_double_click_opens(tx, eng, g, dbl)?;
        }
        for &s in &targets {
            if tx.doc().shape(s).is_some() {
                groups::set_drop_source(tx, eng, s, src)?;
            }
        }
        Ok(())
    });
    // the edit layer lists members by the new modes
    app.model.geo = None;
}
