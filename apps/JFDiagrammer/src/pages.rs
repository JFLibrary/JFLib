// pages -- (JD8 rest) the pages of the drawing: the page tabs kept in step with the document,
// Einfügen › Neue Seite (Leere Seite, Hintergrundseite, Diese Seite duplizieren), the page tab's
// menu (Seite einfügen, Seite löschen, Seite umbenennen, Diese Seite duplizieren, Seiten neu
// anordnen) and its double click, the dialogs Seite umbenennen and Seiten neu anordnen
// (FEATURES RE-01, PG-01 .. PG-03, ZF-13, ZF-14)
//
//   tabs_of(doc)              the tabs of a document: the foreground pages, then the background
//                             pages, each in document order (edit_doc::install, sync)
//   sync(app)                 every pass and after each page command: the tabs again when the
//                             document's pages or names changed (also after undo and redo); the
//                             tab shown stays on its page, a page that appears (inserted,
//                             duplicated, back by undo) is shown, a page that went gives way to
//                             the one before it; (page_size.rs) the view follows a page that
//                             grew to the left or up
//   available(app, id)        Seite löschen needs another foreground page (or a background page
//                             shown); the rest an open drawing
//   new_page / background_page / duplicate / delete / open_rename / open_reorder   the handlers
//   dialogs(app, ui)          Seite umbenennen, Seiten neu anordnen (every pass)
//
// Every command is one undo step (edit_doc::edit).  What the app does, and why:
// * A new foreground page (Neue Seite, Leere Seite, the plus button, Seite einfügen) comes after
//   the page shown (after the last foreground page when a background page is shown) and takes
//   its settings: size, scale, units, print setup, layout and routing, background page, design
//   (diagram_core Transaction::add_page_like; Visio 2016 Step by Step, chapter 3: "Visio uses
//   the properties of the current, active page").  Deviation, decided for JFDiagrammer: Visio
//   2016 puts the page of the plus button and of Neue Seite after the last foreground page and
//   only the tab menu's Einfügen after the current one (same chapter); here all of them follow
//   the page shown.
// * Names: "Seite-n" in the language of the UI, NameU "Page-n"; a background page
//   "Hintergrund-n" / "Background-n" (Step by Step: manual background pages are named
//   "Background-1"); n is the smallest number no page has as Name or NameU (Annahme: Visio's
//   own numbering after deleted pages is not documented).
// * Hintergrundseite: a new background page at the end, shown, assigned to no page (as Visio;
//   Seite einrichten › Seiteneigenschaften assigns it).
// * Diese Seite duplizieren: the page with its content and settings right after it
//   (Transaction::duplicate_page; Visio's Page.Duplicate: "inserted immediately after the
//   original page.  If the original page is numbered automatically, the duplicated page is
//   numbered in the same way").  A page with an own name gets "<Name> (2)", "(3)" ... --
//   Annahme, Visio's naming of such a copy is not documented.
// * Seite löschen: the page shown; pages that use a deleted background page lose it (the
//   library does that); the last foreground page cannot go.
// * Seite umbenennen: Name and NameU both take the new name and are marked as own names
//   (IsCustomName, IsCustomNameU, as the corpus shows for renamed pages; Annahme that Visio's
//   rename sets NameU too, as Seite einrichten does since JD12); a name another page has
//   (Name or NameU, any case) is refused: formulas address pages by name.
// * Seiten neu anordnen: the foreground pages only (Visio's dialog), moved up and down or
//   dragged in the list; background pages keep their places.  The tab strip
//   (ui.tabs_scrolling) offers no dragging, so the dialog is the way.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use diagram_core::{Document, Owner, PageRef, SheetRef};
use ui::i18n::{t, tf};
use ui::*;

use crate::commands::{self, *};
use crate::viewer::PageTab;
use crate::Diagrammer;

const ID_RENAME: u32 = 0x5F40_0000;
const ID_REORDER: u32 = 0x5F50_0000;

/// the dialogs' state
pub(crate) struct PagesState {
    /// Seite umbenennen: open, the page, the name being typed
    pub rename: Shared<i32>,
    pub rename_page: Option<PageRef>,
    pub rename_text: String,
    /// the name box: 2 takes the keyboard next pass, 1 then selects its text
    rename_focus: u8,
    /// Seiten neu anordnen: open, the foreground pages in their new order, the row selected
    pub reorder: Shared<i32>,
    pub order: Vec<PageRef>,
    pub order_sel: i32,
}

impl Default for PagesState {
    fn default() -> PagesState {
        PagesState { rename: Shared::new(0), rename_page: None, rename_text: String::new(), rename_focus: 0, reorder: Shared::new(0), order: Vec::new(), order_sel: 0 }
    }
}

/// the name a tab shows: Name, else NameU, else "Seite-<k>"
pub(crate) fn page_name(doc: &Document, p: PageRef, k: usize) -> String {
    doc.page(p).and_then(|pg| pg.header.name.or(pg.header.name_u)).map(|s| String::from(doc.str(s))).unwrap_or_else(|| tf("Seite-{}", &[&format!("{}", k + 1)]))
}

/// the tabs of a document: the foreground pages, then the background pages (document order)
pub(crate) fn tabs_of(doc: &Document) -> Vec<PageTab> {
    let mut tabs: Vec<PageTab> = Vec::new();
    for bg in [false, true] {
        for p in doc.pages() {
            if doc.page(p).is_some_and(|pg| pg.header.background == bg) {
                let name = page_name(doc, p, tabs.len());
                tabs.push(PageTab { page: p, name });
            }
        }
    }
    tabs
}

/// the page shown
pub(crate) fn shown(app: &Diagrammer) -> Option<PageRef> {
    let o = app.opened.as_ref()?;
    o.tabs.get(app.page_index()).map(|t| t.page)
}

fn is_background(doc: &Document, p: PageRef) -> bool {
    doc.page(p).is_some_and(|pg| pg.header.background)
}

/// the tabs again when the document's pages changed (see the file header)
pub(crate) fn sync(app: &mut Diagrammer) {
    let k = app.page_index();
    let Some(o) = app.opened.as_mut() else { return };
    // (page_size.rs) a page that grew to the left or up: the view keeps the drawing in place
    let grown = core::mem::take(&mut o.ed.grown);
    let shown_now = o.tabs.get(k).map(|t| t.page);
    for (p, d) in grown {
        if Some(p) == shown_now {
            app.canvas.vp.cx += d[0] as f64;
            app.canvas.vp.cy += d[1] as f64;
        }
    }
    let want = tabs_of(&o.doc);
    if want.len() == o.tabs.len() && want.iter().zip(&o.tabs).all(|(a, b)| a.page == b.page && a.name == b.name) {
        return;
    }
    let old: Vec<PageRef> = o.tabs.iter().map(|t| t.page).collect();
    let fresh: Vec<PageRef> = want.iter().map(|t| t.page).filter(|p| !old.contains(p)).collect();
    o.set_tabs(want);
    let pages: Vec<PageRef> = o.tabs.iter().map(|t| t.page).collect();
    let Some(&first) = pages.first() else {
        app.page_tab = 0;
        return;
    };
    let target = if fresh.len() == 1 {
        fresh[0]
    } else if let Some(s) = shown_now.filter(|s| pages.contains(s)) {
        s
    } else {
        // the page before the one that went, else the one after it
        let i = shown_now.and_then(|s| old.iter().position(|p| *p == s)).unwrap_or(0);
        old[..i].iter().rev().chain(old[i..].iter()).find(|p| pages.contains(p)).copied().unwrap_or(first)
    };
    app.page_tab = pages.iter().position(|p| *p == target).unwrap_or(0) as i32;
    if shown_now != Some(target) {
        app.se.select(&[]);
        app.canvas.request_fit();
        app.fit_again = true;
    }
}

/// whether a page command can run now
pub(crate) fn available(app: &Diagrammer, id: u32) -> bool {
    let Some(o) = app.opened.as_ref() else {
        return !matches!(id, NEW_PAGE | PAGE_BLANK | PAGE_INSERT | PAGE_BACKGROUND | PAGE_DUPLICATE | PAGE_DELETE | PAGE_RENAME | PAGE_REORDER);
    };
    match id {
        PAGE_DELETE => match shown(app) {
            Some(p) if is_background(&o.doc, p) => true,
            Some(_) => o.doc.foreground_pages().count() > 1,
            None => false,
        },
        NEW_PAGE | PAGE_BLANK | PAGE_INSERT | PAGE_BACKGROUND | PAGE_DUPLICATE | PAGE_RENAME | PAGE_REORDER => shown(app).is_some(),
        _ => true,
    }
}

/// "<prefix>-n" in the UI's language and "<universal>-n" with the smallest n no page has as
/// Name or NameU (prefix: the German source, "Seite" or "Hintergrund")
pub(crate) fn free_names(doc: &Document, background: bool) -> (String, String) {
    let used = |s: &str| doc.pages().any(|p| doc.page(p).is_some_and(|pg| [pg.header.name, pg.header.name_u].iter().flatten().any(|n| doc.str(*n).eq_ignore_ascii_case(s))));
    for n in 1..100_000u32 {
        let num = format!("{n}");
        let (local, uni) = if background { (tf("Hintergrund-{}", &[&num]), format!("Background-{n}")) } else { (tf("Seite-{}", &[&num]), format!("Page-{n}")) };
        if !used(&local) && !used(&uni) {
            return (local, uni);
        }
    }
    (String::from("?"), String::from("?"))
}

// whether a page's name is one of the automatic ones ("Seite-3", NameU "Page-3", a background
// page's "Background-2") -> Some(background) for such a page
fn auto_named(doc: &Document, p: PageRef) -> Option<bool> {
    let h = &doc.page(p)?.header;
    if h.is_custom_name == Some(true) || h.is_custom_name_u == Some(true) {
        return None;
    }
    let numbered = |s: &str, prefix: &str| s.strip_prefix(prefix).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    let name = h.name.map(|n| doc.str(n)).unwrap_or("");
    let nu = h.name_u.map(|n| doc.str(n)).unwrap_or("");
    let seite = tf("Seite-{}", &[""]);
    let hinter = tf("Hintergrund-{}", &[""]);
    if numbered(nu, "Page-") || numbered(name, &seite) {
        Some(false)
    } else if numbered(nu, "Background-") || numbered(name, &hinter) {
        Some(true)
    } else {
        None
    }
}

/// Neue Seite, Leere Seite, Seite einfügen (the plus button, the tab's menu): a foreground page
/// after the page shown with its settings, then shown
pub(crate) fn new_page(app: &mut Diagrammer, ui: &mut Ui, id: u32) {
    let Some(cur) = shown(app) else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let doc = &o.doc;
    let pos = if is_background(doc, cur) {
        doc.pages().enumerate().filter(|(_, p)| !is_background(doc, *p)).map(|(i, _)| i + 1).last().unwrap_or(0)
    } else {
        doc.page_position(cur).map_or(doc.page_count(), |i| i + 1)
    };
    let (name, name_u) = free_names(doc, false);
    let _ = crate::edit_doc::edit(o, commands::label(id), true, |tx, eng| {
        let p = tx.add_page_like(cur, &name, &name_u, false, Some(pos))?;
        eng.recalc(tx, &[SheetRef::Page(p)])?;
        Ok(p)
    });
    sync(app);
    ui.repass();
}

/// Hintergrundseite: a background page at the end with the settings of the page shown, shown
pub(crate) fn background_page(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(cur) = shown(app) else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let (name, name_u) = free_names(&o.doc, true);
    let _ = crate::edit_doc::edit(o, commands::label(PAGE_BACKGROUND), true, |tx, eng| {
        let p = tx.add_page_like(cur, &name, &name_u, true, None)?;
        eng.recalc(tx, &[SheetRef::Page(p)])?;
        Ok(p)
    });
    sync(app);
    ui.repass();
}

/// Diese Seite duplizieren: the page shown with content and settings right after it, shown
pub(crate) fn duplicate(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(cur) = shown(app) else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let doc = &o.doc;
    let (name, name_u, custom) = match auto_named(doc, cur) {
        Some(bg) => {
            let (a, b) = free_names(doc, bg);
            (a, b, false)
        }
        None => {
            let base = page_name(doc, cur, 0);
            let base_u = doc.page(cur).and_then(|p| p.header.name_u).map(|n| String::from(doc.str(n))).unwrap_or_else(|| base.clone());
            let taken = |s: &str| doc.pages().any(|p| doc.page(p).is_some_and(|pg| [pg.header.name, pg.header.name_u].iter().flatten().any(|n| doc.str(*n).eq_ignore_ascii_case(s))));
            let k = (2..100_000u32).find(|k| !taken(&format!("{base} ({k})")) && !taken(&format!("{base_u} ({k})"))).unwrap_or(2);
            (format!("{base} ({k})"), format!("{base_u} ({k})"), true)
        }
    };
    let _ = crate::edit_doc::edit(o, commands::label(PAGE_DUPLICATE), true, |tx, eng| {
        let p = tx.duplicate_page(cur, &name, &name_u, None)?;
        if custom {
            let mut h = tx.doc().page(p).map(|pg| pg.header.clone()).ok_or(diagram_core::EditError::NoPage)?;
            h.is_custom_name = Some(true);
            h.is_custom_name_u = Some(true);
            tx.set_page_header(p, h)?;
        }
        // the copy's cells in the engine's graph (a page number, the page's name may differ)
        let mut sheets = alloc::vec![SheetRef::Page(p)];
        sheets.extend(tx.doc().all_shapes(Owner::Page(p)).map(SheetRef::Shape));
        eng.recalc(tx, &sheets)?;
        Ok(p)
    });
    sync(app);
    ui.repass();
}

/// Seite löschen: the page shown (not the last foreground page)
pub(crate) fn delete(app: &mut Diagrammer, ui: &mut Ui) {
    if !available(app, PAGE_DELETE) {
        return;
    }
    let Some(cur) = shown(app) else { return };
    let Some(o) = app.opened.as_mut() else { return };
    let _ = crate::edit_doc::edit(o, commands::label(PAGE_DELETE), true, |tx, _| tx.delete_page(cur));
    sync(app);
    ui.repass();
}

/// Seite umbenennen (and a double click on a tab): the dialog for the page shown
pub(crate) fn open_rename(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(cur) = shown(app) else { return };
    let Some(o) = app.opened.as_ref() else { return };
    app.pages.rename_text = page_name(&o.doc, cur, app.page_index());
    app.pages.rename_page = Some(cur);
    app.pages.rename_focus = 2;
    app.pages.rename.set(1);
    ui.repass();
}

/// Seiten neu anordnen: the dialog with the foreground pages
pub(crate) fn open_reorder(app: &mut Diagrammer, ui: &mut Ui) {
    let Some(o) = app.opened.as_ref() else { return };
    app.pages.order = o.doc.foreground_pages().collect();
    let cur = shown(app);
    app.pages.order_sel = app.pages.order.iter().position(|p| Some(*p) == cur).unwrap_or(0) as i32;
    app.pages.reorder.set(1);
    ui.repass();
}

/// the dialogs (every pass)
pub(crate) fn dialogs(app: &mut Diagrammer, ui: &mut Ui) {
    rename_dialog(app, ui);
    reorder_dialog(app, ui);
}

// why a name cannot be taken (None: it can)
fn name_problem(doc: &Document, page: PageRef, name: &str) -> Option<&'static str> {
    let n = name.trim();
    if n.is_empty() {
        return Some(t("Der Name darf nicht leer sein."));
    }
    let taken = doc.pages().filter(|p| *p != page).any(|p| doc.page(p).is_some_and(|pg| [pg.header.name, pg.header.name_u].iter().flatten().any(|s| doc.str(*s).to_lowercase() == n.to_lowercase())));
    taken.then(|| t("Eine andere Seite hat diesen Namen schon."))
}

fn rename_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.pages.rename.clone();
    if !ui.dialog_begin(ID_RENAME, commands::label(PAGE_RENAME), &open) {
        return;
    }
    let mut name = core::mem::take(&mut app.pages.rename_text);
    ui.next_size(320, 0);
    ui.text(TS_BODY, t("Name der Seite"));
    ui.next_size(320, 0);
    ui.textbox(ID_RENAME + 1, None, &mut name, 128);
    // the name ready to be typed over
    match app.pages.rename_focus {
        2 => {
            ui.focus(ID_RENAME + 1);
            app.pages.rename_focus = 1;
        }
        1 if ui.focused() == ID_RENAME + 1 => {
            ui.textbox_select(ID_RENAME + 1, &name, 0, name.len());
            app.pages.rename_focus = 0;
        }
        _ => {}
    }
    let problem = match (app.opened.as_ref(), app.pages.rename_page) {
        (Some(o), Some(p)) => name_problem(&o.doc, p, &name),
        _ => Some(t("Die Seite gibt es nicht mehr.")),
    };
    if let Some(msg) = problem {
        ui.next_size(320, 0);
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, msg);
    }
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    app.pages.rename_text = name;
    if r == DLG_PRIMARY && problem.is_none() {
        rename(app);
    }
    if r != DLG_NONE {
        // OK with a name that cannot be taken keeps the dialog open
        app.pages.rename.set((r == DLG_PRIMARY && problem.is_some()) as i32);
        ui.repass();
    }
}

// the name typed into the dialog as the page's Name and NameU
fn rename(app: &mut Diagrammer) {
    let Some(page) = app.pages.rename_page else { return };
    let name = String::from(app.pages.rename_text.trim());
    let Some(o) = app.opened.as_mut() else { return };
    let _ = crate::edit_doc::edit(o, commands::label(PAGE_RENAME), true, |tx, _| set_name(tx, page, &name));
    sync(app);
}

/// a page's Name and NameU set to `name`, marked as own names (also Seite einrichten's)
pub(crate) fn set_name(tx: &mut diagram_core::Transaction, page: PageRef, name: &str) -> Result<(), diagram_core::EditError> {
    let mut h = tx.doc().page(page).ok_or(diagram_core::EditError::NoPage)?.header.clone();
    let n = tx.intern(name);
    if h.name == Some(n) && h.name_u == Some(n) {
        return Ok(());
    }
    h.name = Some(n);
    h.name_u = Some(n);
    h.is_custom_name = Some(true);
    h.is_custom_name_u = Some(true);
    tx.set_page_header(page, h)
}

fn reorder_dialog(app: &mut Diagrammer, ui: &mut Ui) {
    let open = app.pages.reorder.clone();
    if !ui.dialog_begin(ID_REORDER, commands::label(PAGE_REORDER), &open) {
        return;
    }
    let names: Vec<String> = match app.opened.as_ref() {
        Some(o) => app.pages.order.iter().map(|&p| page_name(&o.doc, p, 0)).collect(),
        None => Vec::new(),
    };
    let n = names.len() as i32;
    ui.next_size(320, 0);
    ui.text(TS_BODY, t("Seiten"));
    ui.box_begin(0, BOX_H, 12);
    ui.next_size(240, 200);
    let mut sel = app.pages.order_sel;
    let (first, end) = ui.list_begin(ID_REORDER + 1, &mut sel, n, 28 | LF_REORDER);
    for row in first..end {
        ui.list_item(row, &names[row as usize]);
    }
    let r = ui.list_end_full(&mut sel);
    if let Some((from, to)) = r.drop {
        if from < app.pages.order.len() && to < app.pages.order.len() {
            let p = app.pages.order.remove(from);
            app.pages.order.insert(to, p);
            sel = to as i32;
        }
    }
    ui.box_begin(0, BOX_V, 8);
    let (up, down) = (sel > 0 && sel < n, sel >= 0 && sel + 1 < n);
    ui.next_size(110, 0);
    ui.next_flags(if up { 0 } else { ST_DISABLED });
    if ui.button(ID_REORDER + 0x100, t("Nach oben")) & EV_ACT != 0 && up {
        app.pages.order.swap(sel as usize, sel as usize - 1);
        sel -= 1;
        ui.repass();
    }
    ui.next_size(110, 0);
    ui.next_flags(if down { 0 } else { ST_DISABLED });
    if ui.button(ID_REORDER + 0x101, t("Nach unten")) & EV_ACT != 0 && down {
        app.pages.order.swap(sel as usize, sel as usize + 1);
        sel += 1;
        ui.repass();
    }
    ui.box_end();
    ui.box_end();
    app.pages.order_sel = sel;
    let r = ui.dialog_buttons(Some(t("OK")), None, Some(t("Abbrechen")));
    ui.dialog_end();
    if r == DLG_PRIMARY {
        reorder(app);
    }
    if r != DLG_NONE {
        app.pages.reorder.set(0);
        ui.repass();
    }
}

// the foreground pages into the dialog's order; background pages keep their places in
// document order
fn reorder(app: &mut Diagrammer) {
    let order = app.pages.order.clone();
    let Some(o) = app.opened.as_mut() else { return };
    let now: Vec<PageRef> = o.doc.pages().collect();
    let slots: Vec<usize> = now.iter().enumerate().filter(|(_, p)| !is_background(&o.doc, **p)).map(|(i, _)| i).collect();
    if slots.len() != order.len() || order.iter().any(|p| !now.contains(p)) {
        return; // the pages changed meanwhile
    }
    let mut want = now.clone();
    for (k, &i) in slots.iter().enumerate() {
        want[i] = order[k];
    }
    if want == now {
        return;
    }
    let _ = crate::edit_doc::edit(o, commands::label(PAGE_REORDER), true, |tx, _| {
        for (j, &p) in want.iter().enumerate() {
            if tx.doc().page_position(p) != Some(j) {
                tx.move_page(p, j)?;
            }
        }
        Ok(())
    });
    sync(app);
}
