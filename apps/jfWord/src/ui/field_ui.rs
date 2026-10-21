// Contextual field actions in the paper editor; structured data stays in doc_core.
use crate::Word;
use alloc::{format, string::String};
use doc_core::Inline;
use ui::*;

impl Word {
    pub(crate) fn field_edit(&mut self, ui: &mut Ui) {
        let Some((paragraph, inline)) = self.rich_field_target() else {
            return;
        };
        let node = self.editor.document().paragraph(paragraph).unwrap().content[inline].clone();
        match node {
            Inline::Citation { .. } | Inline::CitationCluster(_) => {
                self.edit_citation(paragraph, inline)
            }
            Inline::NoteReference(id) => self.open_note(ui, id),
            Inline::Formula(_)=>self.open_formula(Some((paragraph,inline))),
            Inline::Bibliography => {
                self.show_sources = true;
                self.message = String::from(
                    "Das Literaturverzeichnis aktualisiert sich aus den zitierten Quellen. Angaben über Quelle bearbeiten ändern.",
                );
            }
            _ => (),
        }
    }

    pub(crate) fn field_toolbar(&mut self, ui: &mut Ui) {
        if self.structured_edit || self.preview_mode {
            return;
        }
        let Some((paragraph, inline)) = self.rich_field_target() else {
            return;
        };
        let node = self.editor.document().paragraph(paragraph).unwrap().content[inline].clone();
        let (label, action) = match &node {
            Inline::Citation { .. } | Inline::CitationCluster(_) => {
                (String::from("Zitatfeld"), "Zitat bearbeiten")
            }
            Inline::NoteReference(id) => (
                format!("Anmerkung {}", self.editor.document().note_marker(*id)),
                "Anmerkung bearbeiten",
            ),
            Inline::Formula(f)=>(format!("Formel {}",f.expression),"Formel bearbeiten"),
            Inline::Bibliography => (
                String::from("Literaturverzeichnis · automatisch aktuell"),
                "Quellen bearbeiten",
            ),
            _ => return,
        };
        ui.next_size(-1, 36);
        ui.box_begin(1400, BOX_H | pad(4), 6);
        ui.next_size(-1, 28);
        ui.text(TS_CAPTION | TX_ACCENT | TX_ELLIPSIS, &label);
        if ui.button(1401, action) & EV_ACT != 0 {
            self.field_edit(ui);
        }
        if let Some(items) = node.citation_items() {
            if ui.button(1402, "Quelle zeigen") & EV_ACT != 0 {
                self.source_query.clear();
                self.source_sel = self
                    .editor
                    .document()
                    .sources
                    .iter()
                    .position(|s| s.id == items[0].source)
                    .unwrap_or(0) as i32;
                self.show_sources = true;
            }
        }
        if ui.button(1403, "Feld entfernen") & EV_ACT != 0 {
            // Note deletion uses the existing explicit body-and-anchor confirmation.
            if let Inline::NoteReference(id) = node {
                self.confirm_remove_note(id);
            } else {
                self.rich_remove_field(ui, paragraph, inline);
            }
        }
        if ui.button(1404, "Vorheriges Feld") & EV_ACT != 0 {
            self.rich_field_next(ui, false);
        }
        if ui.button(1405, "Nächstes Feld") & EV_ACT != 0 {
            self.rich_field_next(ui, true);
        }
        ui.box_end();
    }
}
