// change_shape -- (JD21) Start › Bearbeiten › Shape ändern (SP-11, RS-28, SH-G05): the selected
// shapes replaced by a master of the open stencils (diagram_core::replace keeps place, size,
// text, shape data, format, glue and membership as the master's Change Shape Behavior cells say)
//
//   available(app)              a selection with a shape that can be replaced
//   flyout(app, ui, pid) -> bool   the gallery of the ribbon button (ribbon::menu asks it first):
//                               the quick shapes of the active stencil, then each open stencil as a
//                               row that unfolds its masters (Visio: the stencils as submenus)
//   run(app, ui, key, k)        the selection replaced by master k of stencil `key`, one undo step
//                               ("Shape ändern"); the new shapes selected
//
// The gallery offers what fits the selection: 1-D masters for 1-D shapes, 2-D ones for 2-D,
// containers for containers, and with User.msvReplaceClass on the selection only masters of the
// same class (Visio's).  Shapes the master cannot replace (a list, LockReplace ...) stay; when
// none can, nothing changes.

use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{instance, replace, Document, EditError, MasterRef, Owner, ShapeRef, SheetRef};
use ui::i18n::{t, tf};
use ui::*;

use crate::commands::CHANGE_SHAPE;
use crate::edit_canvas::{self, key_of, selected};
use crate::{edit_doc, Diagrammer};

// the rows of the gallery: quick shapes, stencils, masters of the unfolded stencil
const ROW_QUICK: u32 = 0x7A20_0000;
const ROW_STENCIL: u32 = 0x7A21_0000;
const ROW_MASTER: u32 = 0x7A22_0000;

// what the selection is: 1-D, a container, its class
#[derive(Clone, PartialEq)]
struct Kind {
    one_d: bool,
    container: bool,
    class: Option<String>,
}

fn structure_of(doc: &Document, s: SheetRef) -> bool {
    doc.user_cells(s).into_iter().any(|r| {
        r.name().is_some_and(|n| n.eq_ignore_ascii_case("msvStructureType"))
            && matches!(r.get("Value").value(), diagram_core::Value::Text(x) if doc.str(x).trim().eq_ignore_ascii_case("container"))
    })
}

fn kind_of(doc: &Document, s: ShapeRef) -> Kind {
    Kind { one_d: ops_one_d(doc, SheetRef::Shape(s)), container: structure_of(doc, SheetRef::Shape(s)), class: replace::class(doc, SheetRef::Shape(s)) }
}

fn ops_one_d(doc: &Document, s: SheetRef) -> bool {
    doc.sym("BeginX").is_some_and(|c| doc.resolve(s, &diagram_core::CellAddr::single(c)).exists())
}

fn master_sheet(doc: &Document, m: MasterRef) -> Option<SheetRef> {
    let slot = doc.master(m)?.master_shape()?;
    Some(SheetRef::Shape(ShapeRef { owner: Owner::Master(m), slot }))
}

// whether master `m` of stencil `doc` fits the selection's kind
fn fits(doc: &Document, m: MasterRef, want: &Kind) -> bool {
    let Some(s) = master_sheet(doc, m) else { return false };
    if ops_one_d(doc, s) != want.one_d || structure_of(doc, s) != want.container {
        return false;
    }
    match &want.class {
        Some(c) => replace::class(doc, s).is_some_and(|x| x.eq_ignore_ascii_case(c)),
        None => true,
    }
}

// the selection's kind (None: nothing replaceable selected, or a mix of 1-D and 2-D)
fn wanted(app: &Diagrammer) -> Option<Kind> {
    let o = app.opened.as_ref()?;
    let sel = selected(app);
    let doc = &o.doc;
    let mut k: Option<Kind> = None;
    for s in sel {
        let sh = doc.shape(s)?;
        if sh.header().kind == diagram_core::ShapeKind::Guide {
            continue;
        }
        let this = kind_of(doc, s);
        match &k {
            None => k = Some(this),
            Some(x) if x.one_d != this.one_d || x.container != this.container => return None,
            Some(x) if x.class != this.class => k = Some(Kind { class: None, ..x.clone() }),
            _ => {}
        }
    }
    k
}

/// a selection that a master can replace
pub(crate) fn available(app: &Diagrammer) -> bool {
    app.opened.is_some() && !app.se.sel.is_empty() && !app.stencils.open.is_empty() && wanted(app).is_some()
}

/// the gallery of Shape ändern (pid: the ribbon button's popup) -> it is this one
pub(crate) fn flyout(app: &mut Diagrammer, ui: &mut Ui, pid: u32) -> bool {
    if pid != CHANGE_SHAPE {
        return false;
    }
    if ui.flyout_begin(pid, 0) {
        let lang = crate::stencils::lang();
        let want = wanted(app);
        let mut pick: Option<(u32, usize)> = None;
        match &want {
            None => {
                ui.next_flags(ST_DISABLED);
                ui.menu_item(ROW_QUICK, t("Kein ersetzbares Shape ausgewählt"));
            }
            Some(want) => {
                // the quick shapes of the active stencil (else of the first open one)
                let st = &app.stencils;
                let active = if st.open.contains(&st.panel.active) { Some(st.panel.active) } else { st.open.first().copied() };
                if let Some(l) = active.and_then(|k| st.get(k)) {
                    ui.text(TS_CAPTION | TX_SECONDARY, &tf("Schnell-Shapes: {}", &[&l.title(lang)]));
                    let mut n = 0;
                    for (k, e) in l.entries.iter().enumerate() {
                        if !e.texts.quick || !fits(&l.doc, e.master, want) {
                            continue;
                        }
                        if ui.menu_item(ROW_QUICK + k as u32, e.texts.name(lang)) & EV_ACT != 0 {
                            pick = Some((l.key, k));
                        }
                        n += 1;
                    }
                    if n == 0 {
                        ui.next_flags(ST_DISABLED);
                        ui.menu_item(ROW_QUICK + 0xFFFF, t("Keine passenden Schnell-Shapes"));
                    }
                }
                ui.menu_sep();
                ui.text(TS_CAPTION | TX_SECONDARY, t("Schablonen"));
                let keys: Vec<u32> = app.stencils.open.clone();
                for (i, key) in keys.iter().enumerate() {
                    let Some(l) = app.stencils.get(*key) else { continue };
                    let unfolded = app.groups.change_open == Some(*key);
                    let title = alloc::format!("{} {}", if unfolded { "▾" } else { "▸" }, l.title(lang));
                    if ui.menu_item_keys(ROW_STENCIL + i as u32, &title, "") & EV_ACT != 0 {
                        app.groups.change_open = if unfolded { None } else { Some(*key) };
                        // the click closed the gallery: it opens again, unfolded (groups_cmds::dialogs)
                        app.groups.change_reopen = true;
                        ui.repass();
                    }
                    if unfolded {
                        let l = app.stencils.get(*key).expect("open");
                        for (k, e) in l.entries.iter().enumerate() {
                            if !fits(&l.doc, e.master, want) {
                                continue;
                            }
                            if ui.menu_item(ROW_MASTER + k as u32, e.texts.name(lang)) & EV_ACT != 0 {
                                pick = Some((*key, k));
                            }
                        }
                    }
                }
            }
        }
        if let Some((key, k)) = pick {
            ui.popup_close();
            run(app, ui, key, k);
        }
    }
    ui.flyout_end();
    true
}

/// the selection replaced by master k of stencil `key`
pub(crate) fn run(app: &mut Diagrammer, ui: &mut Ui, key: u32, k: usize) {
    let shapes = selected(app);
    let Some(page) = app.model.page else { return };
    let lang = crate::stencils::lang();
    let st = &app.stencils;
    let Some((l, e)) = st.entry(key, k) else { return };
    let name = String::from(e.texts.name(lang));
    let Some(o) = app.opened.as_mut() else { return };
    let owner = Owner::Page(page);
    let r = edit_doc::edit(o, crate::commands::label(CHANGE_SHAPE), true, |tx, eng| {
        let m = instance::import_master(tx, &l.doc, e.master)?;
        let new = replace::replace(tx, eng, &shapes, m, lang)?;
        if new.is_empty() {
            return Err(EditError::Invalid);
        }
        edit_canvas::follow(tx, eng, owner, &new)?;
        Ok(new)
    });
    match r {
        Ok(new) => {
            let keys: Vec<u64> = new.iter().map(|&s| key_of(s)).collect();
            app.se.select(&keys);
            ui.announce(&tf("Shape geändert in „{}“", &[&name]), LIVE_POLITE);
        }
        Err(_) => {
            app.error = Some((String::from(t("Das Shape lässt sich nicht ändern.")), tf("„{}“ kann die ausgewählten Shapes nicht ersetzen.", &[&name])));
        }
    }
    ui.repass();
}
