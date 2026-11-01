// labelstest -- checks of the slim labels on the canvas (group labels of --selftest, H3): a
// note's style in the file (a round trip, an old file and an unknown value mean the sticker,
// unknown members skipped, the fixtures and labs stay stickers), the steps of doc/edits.rs (a new
// label with its text is one step, a changed text brings its box along, undo and redo of delete
// and move, the sticker's own steps as before) and the canvas' scene (the box as stored, the hit
// flag, the names for screen readers, a sticker's size as before).

use alloc::format;
use alloc::string::String;

use jfn_model::{Note, NoteStyle, Project};
use ui::DN_HITMIN;

use crate::doc::edits;
use crate::doc::Doc;
use crate::selftest::T;
use crate::workspace::canvas::groups::note_rect;
use crate::workspace::canvas::paint::Scene;
use crate::workspace::icons::Symbols;

const FIXTURES: &[(&str, &str)] = &[
    ("office", include_str!("../tests/fixtures/office.jfnet")),
    ("campus", include_str!("../tests/fixtures/campus.jfnet")),
    ("lab 01", include_str!("../assets/labs/01-campus-vlan.jfnet")),
];

pub(crate) fn run(t: &mut T) {
    file(t);
    steps(t);
    scene(t);
}

fn note(p: &Project, id: u32) -> Note {
    p.note(id).cloned().unwrap_or_default()
}

fn file(t: &mut T) {
    let mut p = Project::new("Beschriftungen", 7);
    let s = p.add_note(10, 20, "Karte");
    let l = p.add_label(-30, 40, 75, 20, "Erster Stock");
    let m = p.add_label(0, 0, 60, 38, "Zeile 1\nZeile 2");
    t.eq("a sticker by default", note(&p, s).style, NoteStyle::Sticker);
    t.eq("a sticker's size as before", (note(&p, s).w, note(&p, s).h), (200, 80));
    t.eq("a label's box as given", (note(&p, l).w, note(&p, l).h, note(&p, l).style), (75, 20, NoteStyle::Label));
    t.check("ids from the one counter", s != l && l != m && p.next_id > m);
    t.check("a label is no problem for check()", p.check().is_empty());
    let text = p.to_text();
    t.check("in the file: both styles", text.contains("\"style\": \"sticker\"") && text.contains("\"style\": \"label\""));
    match Project::from_text(&text) {
        Ok(q) => {
            t.eq("read back: the whole project", q.clone(), p.clone());
            t.eq("read back: the lines of a label", note(&q, m).text, String::from("Zeile 1\nZeile 2"));
        }
        Err(e) => t.check(&format!("read back: {}", e.text), false),
    }
    // an old file: no style, the size it had
    let old = "{\"format\": \"jfn\", \"version\": 1, \"next_id\": 4, \"notes\": [{\"id\": 3, \"x\": 5, \"y\": 6, \"w\": 240, \"h\": 110, \"text\": \"Alt\"}]}";
    match Project::from_text(old) {
        Ok(q) => t.eq("an old file: a sticker as it was", note(&q, 3), Note { id: 3, x: 5, y: 6, w: 240, h: 110, text: String::from("Alt"), style: NoteStyle::Sticker }),
        Err(e) => t.check(&format!("an old file: {}", e.text), false),
    }
    // what a newer program may write
    let new = "{\"format\": \"jfn\", \"version\": 1, \"next_id\": 4, \"notes\": [{\"id\": 3, \"text\": \"x\", \"style\": \"wolke\", \"farbe\": \"blau\"}]}";
    t.check("an unknown style: a sticker", Project::from_text(new).is_ok_and(|q| note(&q, 3).style == NoteStyle::Sticker));
    let bad = "{\"format\": \"jfn\", \"version\": 1, \"next_id\": 4, \"notes\": [{\"id\": 3, \"text\": \"x\", \"style\": 5}]}";
    t.check("a style of another type: a sticker", Project::from_text(bad).is_ok_and(|q| note(&q, 3).style == NoteStyle::Sticker));
    for (name, text) in FIXTURES {
        let q = Project::from_text(text);
        t.check(&format!("{}: its notes stay stickers", name), q.is_ok_and(|q| q.notes.iter().all(|n| n.style == NoteStyle::Sticker)));
    }
    // the box counts for what the project covers
    let mut b = Project::new("", 1);
    b.add_label(100, 50, 75, 20, "x");
    t.eq("bounds: the label's box", b.bounds(), Some([100, 50, 175, 70]));
}

fn steps(t: &mut T) {
    let mut doc = Doc::new();
    let n0 = doc.hist.len();
    // a new label and its first text are one step
    let (id, e) = edits::add_label(&mut doc.proj, 8, 16, 75, 20, "Erster Stock");
    doc.record(e);
    t.eq("a new label: one step", doc.hist.len(), n0 + 1);
    t.eq("its name", doc.hist.undo_label(), Some("Beschriftung hinzufügen"));
    t.check("unsaved", doc.dirty());
    t.check("undo: gone with its text", doc.undo() && doc.proj.notes.is_empty());
    t.check("redo: the same label", doc.redo() && note(&doc.proj, id) == Note { id, x: 8, y: 16, w: 75, h: 20, text: String::from("Erster Stock"), style: NoteStyle::Label });
    // a new text brings its box along
    t.check("the same text and box: no step", edits::label_edit(&doc.proj, id, "Erster Stock", 75, 20).is_none());
    let e = edits::label_edit(&doc.proj, id, "Stock", 40, 20);
    t.check("another text: a step", e.is_some());
    if let Some(e) = e {
        doc.apply(e);
    }
    t.eq("text and box", (note(&doc.proj, id).text, note(&doc.proj, id).w), (String::from("Stock"), 40));
    t.eq("its name", doc.hist.undo_label(), Some("Beschriftung bearbeiten"));
    t.check("undo: text and box", doc.undo() && note(&doc.proj, id).text == "Erster Stock" && note(&doc.proj, id).w == 75);
    t.check("redo: text and box", doc.redo() && note(&doc.proj, id).text == "Stock" && note(&doc.proj, id).w == 40);
    t.check("still a label", note(&doc.proj, id).style == NoteStyle::Label);
    // moves like a note
    let from = [(id, [8, 16])];
    edits::set_pos(&mut doc.proj, id, [28, 36]);
    if let Some(e) = edits::moves(&doc.proj, &from) {
        doc.record(e);
    }
    t.eq("a move's name", doc.hist.undo_label(), Some("Beschriftung verschieben"));
    t.check("undo of the move", doc.undo() && edits::pos(&doc.proj, id) == Some([8, 16]));
    t.check("redo of the move", doc.redo() && edits::pos(&doc.proj, id) == Some([28, 36]));
    // deletes like a note, back in its place among the notes
    let (s, e) = edits::add_note(&mut doc.proj, 0, 0);
    doc.record(e);
    t.eq("a sticker's step as before", doc.hist.undo_label(), Some("Notiz hinzufügen"));
    t.eq("two notes: the label first", doc.proj.notes[0].id, id);
    if let Some(e) = edits::delete(&doc.proj, &[id]) {
        doc.apply(e);
    }
    t.eq("a delete's name", doc.hist.undo_label(), Some("Beschriftung löschen"));
    t.check("deleted, the sticker stays", doc.proj.note(id).is_none() && doc.proj.note(s).is_some());
    t.check("undo: back in its place", doc.undo() && doc.proj.notes[0].id == id && doc.proj.notes[0].style == NoteStyle::Label);
    if let Some(e) = edits::delete(&doc.proj, &[s]) {
        doc.apply(e);
    }
    t.eq("a sticker's delete as before", doc.hist.undo_label(), Some("Notiz löschen"));
    t.check("the kinds of notes", edits::is_label(&doc.proj, id) && !edits::is_label(&doc.proj, s));
}

fn scene(t: &mut T) {
    let mut p = Project::new("", 1);
    let s = p.add_note(0, 0, "Karte");
    let e = p.add_note(300, 0, "");
    let l = p.add_label(-30, 40, 75, 20, "Erste Zeile\nZweite Zeile");
    let tiny = p.add_label(500, 500, 2, 1, "x");
    let syms = Symbols::new();
    let mut sc = Scene::default();
    sc.build(&p, &syms, 1);
    t.eq("a label's box as stored", sc.rect(l as u64), Some([-30.0, 40.0, 75.0, 20.0]));
    t.eq("a sticker's box", sc.rect(s as u64), Some([0.0, 0.0, 200.0, 80.0]));
    t.eq("a box is never nothing", sc.rect(tiny as u64), Some([500.0, 500.0, 4.0, 4.0]));
    let flags = |k: u32| sc.nodes.iter().find(|n| n.key == k as u64).map_or(0, |n| n.flags);
    t.eq("a label stays selectable at low zoom", flags(l) & DN_HITMIN, DN_HITMIN);
    t.eq("a sticker has no such flag", flags(s), 0);
    t.eq("a label's name: its first line", sc.label(l as u64), String::from("Beschriftung: Erste Zeile"));
    t.eq("a sticker's name as before", sc.label(s as u64), String::from("Notiz: Karte"));
    t.eq("an empty sticker's name as before", sc.label(e as u64), String::from("Notiz"));
    // a frame of a group hugs a small label: no sticker's minimum
    t.eq("the box a group counts", note_rect(&note(&p, tiny)), [500.0, 500.0, 4.0, 4.0]);
    t.eq("a sticker's minimum stays", note_rect(&Note { w: 10, h: 10, ..Note::default() }), [0.0, 0.0, 40.0, 24.0]);
}
