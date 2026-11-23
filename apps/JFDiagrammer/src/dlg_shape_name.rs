// dlg_shape_name -- (JD15) the dialog Shape-Name (SP-01): the selected shape's Name and Name
// universell (the file's Name and NameU), from the shape's context menu
//
//   NameDialog                   open, the shape, the two names being typed
//   open(app, ui)                the dialog for the first selected shape, its names as shown
//   dialog(app, ui)              every pass; OK writes both names in one undo step
//                                (Transaction::set_shape_names)
//
// An empty field, or a derived name left as it was, keeps the name derived ("Sheet.7",
// "<Master>.7"); the derived names stay "Sheet.n" in every language, since formulas, scripts and
// screen readers name shapes so.  A name another shape of the page has is refused (formulas
// name shapes by it), as is a name with '!' (a formula's sheet reference ends there).

use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{Document, Owner, ShapeRef};
use ui::i18n::t;
use ui::*;

use crate::commands::{self, SHAPE_NAME};
use crate::{edit_canvas, edit_doc, Diagrammer};

const ID: u32 = 0x7910_0000;

/// the dialog's state
pub(crate) struct NameDialog {
    pub open: Shared<i32>,
    shape: Option<ShapeRef>,
    name: String,
    name_u: String,
    /// the names shown when it opened (left as they were: nothing changes)
    was: (String, String),
    /// 2: the name box takes the keyboard next pass, 1: then its text is selected
    focus: u8,
}

impl Default for NameDialog {
    fn default() -> NameDialog {
        NameDialog { open: Shared::new(0), shape: None, name: String::new(), name_u: String::new(), was: (String::new(), String::new()), focus: 0 }
    }
}

/// Shape-Name: the dialog for the first selected shape
pub(crate) fn open(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(&s) = edit_canvas::selected(app).first() else { return };
    let Some(o) = app.opened.as_ref() else { return };
    let Some(sh) = o.doc.shape(s) else { return };
    let d = &mut app.shape_name;
    d.shape = Some(s);
    d.name = sh.name();
    d.name_u = sh.name_u();
    d.was = (d.name.clone(), d.name_u.clone());
    d.focus = 2;
    d.open.set(1);
    ui.repass();
}

// every shape of a page (groups' members too)
fn shapes_of(doc: &Document, owner: Owner) -> Vec<ShapeRef> {
    doc.all_shapes(owner).collect()
}

// why a name cannot be taken (None: it can)
fn problem(doc: &Document, s: ShapeRef, name: &str, universal: bool) -> Option<&'static str> {
    let n = name.trim();
    if n.contains('!') {
        return Some(t("Ein Shape-Name darf kein „!“ enthalten."));
    }
    if n.is_empty() {
        return None;
    }
    let taken = shapes_of(doc, s.owner).into_iter().filter(|&x| x != s).any(|x| {
        doc.shape(x).is_some_and(|sh| {
            let other = if universal { sh.name_u() } else { sh.name() };
            other.to_lowercase() == n.to_lowercase()
        })
    });
    taken.then(|| t("Ein anderes Shape der Seite hat diesen Namen schon."))
}

/// the dialog (every pass)
pub(crate) fn dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.shape_name.open.clone();
    if !ui.dialog_begin(ID, commands::label(SHAPE_NAME), &open) {
        return;
    }
    let d = &mut app.shape_name;
    let mut name = core::mem::take(&mut d.name);
    let mut name_u = core::mem::take(&mut d.name_u);
    ui.next_size(340, 0);
    ui.text(TS_BODY, t("Name"));
    ui.next_size(340, 0);
    ui.textbox(ID + 1, None, &mut name, 128);
    match d.focus {
        2 => {
            ui.focus(ID + 1);
            d.focus = 1;
        }
        1 if ui.focused() == ID + 1 => {
            ui.textbox_select(ID + 1, &name, 0, name.len());
            d.focus = 0;
        }
        _ => {}
    }
    ui.next_size(340, 0);
    ui.text(TS_BODY, t("Name universell"));
    ui.next_size(340, 0);
    ui.textbox(ID + 2, None, &mut name_u, 128);
    ui.next_size(340, 0);
    ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Leer: der Name wird abgeleitet (Sheet.n oder der Name des Master-Shapes mit der Nummer)."));
    let issue = match (app.opened.as_ref(), d.shape) {
        (Some(o), Some(s)) if o.doc.shape(s).is_some() => problem(&o.doc, s, &name, false).or_else(|| problem(&o.doc, s, &name_u, true)),
        _ => Some(t("Das Shape gibt es nicht mehr.")),
    };
    if let Some(msg) = issue {
        ui.next_size(340, 0);
        ui.text(TS_CAPTION | TX_WRAP, msg);
    }
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    let d = &mut app.shape_name;
    d.name = name;
    d.name_u = name_u;
    if r == DLG_PRIMARY && issue.is_none() {
        apply(app);
    }
    if r != DLG_NONE {
        // OK with a name that cannot be taken keeps the dialog open
        app.shape_name.open.set((r == DLG_PRIMARY && issue.is_some()) as i32);
        ui.repass();
    }
}

// the typed names into the shape (one undo step); unchanged derived names stay derived
fn apply(app: &mut Diagrammer) {
    let d = &app.shape_name;
    let Some(s) = d.shape else { return };
    let (name, name_u) = (String::from(d.name.trim()), String::from(d.name_u.trim()));
    let was = d.was.clone();
    let Some(o) = app.opened.as_mut() else { return };
    let Some(h) = o.doc.shape(s).map(|x| x.header().clone()) else { return };
    // a field left as it opened keeps what the shape had (an own name, or none: derived)
    let pick = |typed: &str, shown: &str, own: Option<diagram_core::Sym>| -> Option<String> {
        if typed.is_empty() {
            None
        } else if typed == shown {
            own.map(|_| String::from(typed))
        } else {
            Some(String::from(typed))
        }
    };
    let new_name = pick(&name, &was.0, h.name);
    let new_name_u = pick(&name_u, &was.1, h.name_u);
    if new_name.is_none() == h.name.is_none() && new_name_u.is_none() == h.name_u.is_none() && name == was.0 && name_u == was.1 {
        return;
    }
    let _ = edit_doc::edit(o, commands::label(SHAPE_NAME), true, |tx, _| tx.set_shape_names(s, new_name.as_deref(), new_name_u.as_deref()));
}
