// cmds_actions -- (JD15, SP-12) the rows of a shape's Actions section in its context menu, as
// Visio shows them: a shape switches its type or look from there (the bar's "Enden geschlossen",
// BPMN's event and task types) -- no code per shape, only its ShapeSheet
//
//   ACTION_ROWS, ACTION_ROWS_MAX   the command ids of the rows (a Mi::List at the top of the
//                                  shape's menu, panes.rs SHAPE_MENU)
//   frame(app, ui)                 every pass: the rows of the single selected shape -- Menu as
//                                  the text, Checked and Disabled as the row's state
//                                  (cmd_state); Invisible rows and rows without a text are left
//                                  out
//   command(app, ui, id) -> bool   a row clicked: its Action formula runs (Engine::run_action:
//                                  SETF writes its formula, the cells that depend on it follow),
//                                  one undo step named after the row; of the other effects
//                                  GOTOPAGE shows the page, HYPERLINK goes to a page of the
//                                  drawing or opens a web or mail address, OPENTEXTWIN edits the
//                                  text; RUNMACRO and CALLTHIS never run (FX-15)
//
// Visio's BeginGroup (a separator above the row) has no counterpart in a Mi::List row and is
// left out; the rows keep the section's order (SortKey is not read).

use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::formula::Effect;
use diagram_core::{CellAddr, RowKey, SecKey, ShapeRef, SheetRef};
use ui::i18n::t;
use ui::*;

use crate::Diagrammer;

/// the first command id of the shape menu's action rows (Mi::List, at most ACTION_ROWS_MAX)
pub(crate) const ACTION_ROWS: u32 = 0x1840;
pub(crate) const ACTION_ROWS_MAX: u16 = 16;

// a row of the menu: the shape, its Actions row, the text, checked, disabled
struct Row {
    shape: ShapeRef,
    key: RowKey,
    text: String,
    checked: bool,
    disabled: bool,
}

// the rows of the single selected shape
fn rows(app: &Diagrammer) -> Vec<Row> {
    let sel = crate::edit_canvas::selected(app);
    let (Some(o), [s]) = (app.opened.as_ref(), sel.as_slice()) else { return Vec::new() };
    let doc = &o.doc;
    let Some(sec) = doc.sym("Actions") else { return Vec::new() };
    let mut out = Vec::new();
    for r in doc.section_rows(SheetRef::Shape(*s), SecKey::one(sec)) {
        if r.flag("Invisible") {
            continue;
        }
        let text = String::from(r.text("Menu").unwrap_or("").trim());
        if text.is_empty() {
            continue;
        }
        out.push(Row { shape: *s, key: r.key(), text, checked: r.flag("Checked"), disabled: r.flag("Disabled") });
        if out.len() >= ACTION_ROWS_MAX as usize {
            break;
        }
    }
    out
}

/// every pass: the action rows of the selected shape for its menu
pub(crate) fn frame(app: &mut Diagrammer, ui: &mut Ui) {
    let rows = rows(app);
    let texts: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
    ui.menu_list(ACTION_ROWS, &texts);
    for (k, r) in rows.iter().enumerate() {
        let f = if r.checked { CF_CHECKED } else { 0 } | if r.disabled { CF_DISABLED } else { 0 };
        ui.cmd_state(ACTION_ROWS + k as u32, f);
    }
}

/// a row of the shape's menu -> handled
pub(crate) fn command(app: &mut Diagrammer, ui: &mut Ui, id: u32) -> bool {
    if !(ACTION_ROWS..ACTION_ROWS + ACTION_ROWS_MAX as u32).contains(&id) {
        return false;
    }
    let rows = rows(app);
    let Some(r) = rows.get((id - ACTION_ROWS) as usize) else { return true };
    if r.disabled {
        return true;
    }
    let (shape, key, label) = (r.shape, r.key, r.text.replace('&', ""));
    let Some(o) = app.opened.as_mut() else { return true };
    let done = crate::edit_doc::edit(o, &label, true, |tx, eng| {
        let sec = SecKey::one(tx.intern("Actions"));
        let addr = CellAddr::row(sec, key, tx.intern("Action"));
        eng.run_action(tx, SheetRef::Shape(shape), &addr)
    });
    if let Ok(effects) = done {
        for e in effects {
            effect(app, ui, shape, e);
        }
    }
    ui.repass();
    true
}

// what an action asks of the app besides its SETF
fn effect(app: &mut Diagrammer, ui: &mut Ui, shape: ShapeRef, e: Effect) {
    let page_tab = |app: &Diagrammer, name: &str| -> Option<usize> {
        let o = app.opened.as_ref()?;
        let page = diagram_view::export::page_by_name(&o.doc, name)?;
        o.tabs.iter().position(|t| t.page == page)
    };
    match e {
        Effect::GotoPage(name) => match page_tab(app, &name) {
            Some(k) => crate::cmds_text::show_shape(app, ui, k, None),
            None => app.error = Some((String::from(t("Die Aktion lässt sich nicht ausführen.")), ui::i18n::tf("Die Seite „{}“ gibt es nicht.", &[&name]))),
        },
        Effect::Hyperlink(address, sub) => {
            let address = address.trim();
            if address.is_empty() {
                if let Some(k) = page_tab(app, sub.split_once('/').map_or(sub.as_str(), |(p, _)| p)) {
                    crate::cmds_text::show_shape(app, ui, k, None);
                }
            } else {
                let lower = address.to_ascii_lowercase();
                let url = if lower.starts_with("www.") { alloc::format!("https://{address}") } else { String::from(address) };
                if !(url.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:")) || ui.open_url(&url) != 0 {
                    app.error = Some((String::from(t("Der Link lässt sich nicht öffnen.")), String::from(address)));
                }
            }
        }
        Effect::OpenTextWindow(_) => {
            app.se.select(&[crate::edit_canvas::key_of(shape)]);
            crate::edit_cmds::edit_text(app, ui);
        }
        Effect::Refused(what) => {
            app.error = Some((String::from(t("Die Aktion lässt sich nicht ausführen.")), ui::i18n::tf("„{}“ führt JFDiagrammer nicht aus.", &[&what])));
        }
        _ => {}
    }
}
