// Native note editing reuses the document's text/format/citation controls and history.
use crate::{NOTE_CONVERT, NOTE_REMOVE, NOTE_RETURN, Word};
use alloc::{format, string::String, vec::Vec};
use doc_core::{Edit, Id, Inline, NoteKind, Paragraph};
use ui::*;

pub(crate) struct NotesUi {
    remove_open: Shared<i32>,
    target: Id,
    revision: u64,
    error: String,
    focus_target: Option<(Id, usize)>,
}
impl Default for NotesUi {
    fn default() -> Self {
        Self {
            remove_open: Shared::new(0),
            target: 0,
            revision: 0,
            error: String::new(),
            focus_target: None,
        }
    }
}
impl NotesUi {
    pub(crate) fn modal(&self) -> bool {
        self.remove_open.get() != 0
    }
    pub(crate) fn cancel(&self) {
        self.remove_open.set(0);
    }
}
fn kind_label(kind: NoteKind) -> &'static str {
    match kind {
        NoteKind::Footnote => "Fußnote",
        NoteKind::Endnote => "Endnote",
    }
}
impl Word {
    pub(crate) fn current_paragraphs(&self) -> &[Paragraph] {
        if let Some(cell)=self.active_cell.and_then(|id|self.editor.document().cell(id)){return &cell.paragraphs;}
        self.active_note
            .and_then(|id| self.editor.document().note(id))
            .map(|note| note.paragraphs.as_slice())
            .unwrap_or(&self.editor.document().paragraphs)
    }
    pub(crate) fn select_paragraph(&mut self, id: Id) -> bool {
        let doc = self.editor.document();
        let note = doc.note_for_paragraph(id);
        let cell=doc.cell_for_paragraph(id);
        let paragraphs = cell.map(|c|c.paragraphs.as_slice()).or_else(||note.map(|n|n.paragraphs.as_slice())).unwrap_or(&doc.paragraphs);
        let Some(index) = paragraphs.iter().position(|p| p.id == id) else {
            return false;
        };
        self.active_note = note.map(|n| n.id);
        self.active_cell = cell.map(|c|c.id);
        self.selected = index;
        self.preview_mode = false;
        self.jump_to_selected = true;
        self.formatting.selection = None;
        self.formatting.pending_selection = None;
        true
    }
    pub(crate) fn paragraph_label(&self, id: Id) -> String {
        let doc = self.editor.document();
        if let Some(note) = doc.note_for_paragraph(id) {
            let index = note.paragraphs.iter().position(|p| p.id == id).unwrap_or(0);
            format!(
                "{} {} · Absatz {}",
                kind_label(note.kind),
                doc.note_number(note.id).unwrap_or(0),
                index + 1
            )
        } else {
            format!(
                "Absatz {}",
                doc.paragraphs.iter().position(|p| p.id == id).unwrap_or(0) + 1
            )
        }
    }
    pub(crate) fn open_note(&mut self, ui: &mut Ui, id: Id) {
        if let Some(paragraph) = self
            .editor
            .document()
            .note(id)
            .and_then(|n| n.paragraphs.first())
            .map(|p| p.id)
        {
            self.select_paragraph(paragraph);
            let widget = self.text_id(paragraph, 0);
            ui.focus(widget);
        }
    }
    pub(crate) fn insert_note(&mut self, ui: &mut Ui, kind: NoteKind) {
        if self.active_note.is_some() {
            self.message =
                String::from("Anmerkungen werden an einer Textstelle im Haupttext eingefügt.");
            return;
        }
        let paragraph = self.selected_id();
        let p = self.editor.document().paragraph(paragraph).unwrap();
        let at = self
            .formatting
            .selection
            .filter(|(id, inline, _, caret)| {
                *id == paragraph
                    && p.content
                        .get(*inline)
                        .and_then(Inline::text)
                        .is_some_and(|text| *caret <= text.len() && text.is_char_boundary(*caret))
            })
            .map(|(_, inline, _, offset)| (inline, offset))
            .or_else(|| {
                p.content
                    .iter()
                    .enumerate()
                    .rev()
                    .find_map(|(i, node)| node.text().map(|text| (i, text.len())))
            });
        let Some((inline, offset)) = at else {
            self.message = String::from("Bitte eine Textstelle für die Anmerkung auswählen.");
            return;
        };
        if let Some(id) = self.apply_page_edit(ui, Edit::InsertNote {
            paragraph,
            inline,
            offset,
            kind,
            text: String::new(),
        }) {
            self.widget_ids.clear();
            self.open_note(ui, id);
            self.message = format!(
                "{} eingefügt · Inhalt bearbeiten und über Zur Textstelle zurückkehren",
                kind_label(kind)
            );
        }
    }
    pub(crate) fn note_editor_header(&mut self, ui: &mut Ui) {
        let Some(id) = self.active_note else {
            ui.text(TS_CAPTION | TX_SECONDARY, "DOKUMENT · ABSATZBEARBEITUNG");
            return;
        };
        let doc = self.editor.document();
        let note = doc.note(id).unwrap();
        ui.text(
            TS_SUBTITLE,
            &format!(
                "{} {} bearbeiten",
                kind_label(note.kind),
                doc.note_number(id).unwrap_or(0)
            ),
        );
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, "Text, Formatierung und Zitate gehören zur Anmerkung. Änderungen werden mit dem Dokument gespeichert.");
        ui.box_begin(0, BOX_H, 6);
        if ui.button(1200, "Zur Textstelle") & EV_ACT != 0 {
            ui.cmd_run(NOTE_RETURN, 0);
        }
        if ui.button(
            1201,
            if note.kind == NoteKind::Footnote {
                "In Endnote umwandeln"
            } else {
                "In Fußnote umwandeln"
            },
        ) & EV_ACT
            != 0
        {
            ui.cmd_run(NOTE_CONVERT, 0);
        }
        if ui.button(1202, "Entfernen …") & EV_ACT != 0 {
            ui.cmd_run(NOTE_REMOVE, 0);
        }
        ui.box_end();
    }
    pub(crate) fn note_card(&mut self, ui: &mut Ui, id: Id, widget: u32) {
        let Some(note) = self.editor.document().note(id) else {
            return;
        };
        let label = format!(
            "{} {} öffnen",
            kind_label(note.kind),
            self.editor.document().note_number(id).unwrap_or(0)
        );
        if ui.button(widget + 4, &label) & EV_ACT != 0 {
            self.open_note(ui, id);
        }
    }
    pub(crate) fn notes_navigation(&mut self, ui: &mut Ui) {
        let doc = self.editor.document();
        let notes: Vec<(Id, String)> = [NoteKind::Footnote, NoteKind::Endnote]
            .into_iter()
            .flat_map(|kind| {
                doc.ordered_notes(kind).into_iter().map(move |note| {
                    (
                        note.id,
                        format!(
                            "{} {}",
                            kind_label(kind),
                            doc.note_number(note.id).unwrap_or(0)
                        ),
                    )
                })
            })
            .collect();
        if !notes.is_empty() {
            ui.text(TS_BODY_STRONG, "Anmerkungen");
        }
        for (id, label) in notes {
            let widget = self.text_id(id, usize::MAX);
            ui.next_size(-1, 0);
            if ui.button(widget + 4, &label) & EV_ACT != 0 {
                self.open_note(ui, id);
            }
            if self.active_note == Some(id) {
                let paragraphs = self.current_paragraphs().to_vec();
                for (index, p) in paragraphs.iter().enumerate() {
                    let widget = self.text_id(p.id, usize::MAX);
                    ui.next_size(-1, 0);
                    if ui.button(widget + 5, &format!("Anmerkungsabsatz {}", index + 1)) & EV_ACT
                        != 0
                    {
                        self.select_paragraph(p.id);
                    }
                }
            }
        }
    }
    pub(crate) fn return_to_note_anchor(&mut self, ui: &mut Ui) {
        let Some(at) = self
            .active_note
            .and_then(|id| self.editor.document().note_location(id))
        else {
            return;
        };
        self.select_paragraph(at.paragraph);
        let widget = self.text_id(at.paragraph, at.inline);
        ui.focus(widget + 4);
        if !self.structured_edit {
            self.rich_anchor(ui, at.paragraph, at.inline);
        }
        self.message = String::from("Textstelle der Anmerkung ausgewählt");
    }
    pub(crate) fn convert_note(&mut self, ui: &mut Ui) {
        if let Some(note) = self
            .active_note
            .and_then(|id| self.editor.document().note(id))
        {
            let kind = if note.kind == NoteKind::Footnote {
                NoteKind::Endnote
            } else {
                NoteKind::Footnote
            };
            if self
                .apply_page_edit(ui, Edit::SetNoteKind {
                    note: note.id,
                    kind,
                })
                .is_some()
            {
                self.message = format!(
                    "In {} umgewandelt · Nummerierung aktualisiert",
                    kind_label(kind)
                );
            }
        }
    }
    pub(crate) fn confirm_remove_note(&mut self, id: Id) {
        if self.editor.document().note(id).is_some() {
            self.notes_ui.target = id;
            self.notes_ui.revision = self.editor.revision();
            self.notes_ui.error.clear();
            self.notes_ui.remove_open.set(1);
        }
    }
    pub(crate) fn note_dialogs(&mut self, ui: &mut Ui) {
        let was_open = self.notes_ui.modal();
        if ui.dialog_begin(1210, "Anmerkung entfernen", &self.notes_ui.remove_open) {
            let doc = self.editor.document();
            if let Some(note) = doc.note(self.notes_ui.target) {
                ui.text(
                    TS_BODY_STRONG,
                    &format!(
                        "{} {}",
                        kind_label(note.kind),
                        doc.note_number(note.id).unwrap_or(0)
                    ),
                );
                ui.next_size(-1, 220);
                ui.scroll_begin(1211, SC_V | BOX_V);
                for paragraph in &note.paragraphs {
                    ui.text(TS_BODY | TX_WRAP, &doc.paragraph_text(paragraph));
                }
                ui.scroll_end();
            }
            ui.text(TS_BODY | TX_WRAP, "Anmerkung und Textverweis entfernen? Quellen bleiben erhalten. Rückgängig stellt den vollständigen Inhalt wieder her.");
            if !self.notes_ui.error.is_empty() {
                ui.text(TS_CAPTION | TX_WRAP, &self.notes_ui.error);
            }
            let action = ui.dialog_buttons(Some("Anmerkung entfernen"), None, Some("Abbrechen"));
            if action == DLG_CLOSE && !self.structured_edit {
                self.notes_ui.focus_target = Some((self.selected_id(), 0));
            }
            if action == DLG_PRIMARY {
                if self.editor.revision() != self.notes_ui.revision {
                    self.notes_ui.error = String::from(
                        "Das Dokument wurde inzwischen geändert. Bitte erneut öffnen.",
                    );
                    self.notes_ui.remove_open.set(1);
                } else {
                    let at = self.editor.document().note_location(self.notes_ui.target);
                    if self.apply(Edit::RemoveNote(self.notes_ui.target)).is_some() {
                        self.widget_ids.clear();
                        if let Some(at) = at {
                            self.select_paragraph(at.paragraph);
                            let paragraph = self.editor.document().paragraph(at.paragraph).unwrap();
                            if let Some(inline) = paragraph
                                .content
                                .iter()
                                .position(|node| node.text().is_some())
                            {
                                self.notes_ui.focus_target = Some((at.paragraph, inline));
                            }
                        }
                        self.message = String::from("Anmerkung und Textverweis entfernt");
                    } else {
                        self.notes_ui.error = self.message.clone();
                        self.notes_ui.remove_open.set(1);
                    }
                }
            }
        }
        ui.dialog_end();
        // Closing the dialog restores its previous focus first. Request the surviving
        // text field on the next frame, after that restoration has completed.
        if !was_open {
            if let Some((paragraph, inline)) = self.notes_ui.focus_target.take() {
                let widget = self.text_id(paragraph, inline);
                ui.focus(widget);
            }
        }
    }
}
