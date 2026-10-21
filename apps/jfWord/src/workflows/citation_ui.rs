// Citation drafts and source-use navigation. New jfWord UI, no legacy counterpart.
use crate::Word;
use alloc::{format, string::String, vec, vec::Vec};
use doc_core::{CitationItem, Edit, Inline};
use ui::*;

pub(crate) struct Citations {
    open: Shared<i32>,
    uses_open: Shared<i32>,
    items: Vec<CitationItem>,
    active: i32,
    target: (u64, usize),
    revision: u64,
    error: String,
    use_source: u64,
    use_page: usize,
    focus_target: Option<(u64, usize)>,
}
impl Default for Citations {
    fn default() -> Self {
        Self {
            open: Shared::new(0),
            uses_open: Shared::new(0),
            items: Vec::new(),
            active: 0,
            target: (0, 0),
            revision: 0,
            error: String::new(),
            use_source: 0,
            use_page: 0,
            focus_target: None,
        }
    }
}
impl Citations {
    pub(crate) fn modal(&self) -> bool {
        self.open.get() != 0 || self.uses_open.get() != 0
    }
    pub(crate) fn cancel(&self) {
        self.open.set(0);
        self.uses_open.set(0);
    }
}
impl Word {
    pub(crate) fn edit_citation(&mut self, paragraph: u64, inline: usize) {
        let Some(items) = self
            .editor
            .document()
            .paragraph(paragraph)
            .and_then(|p| p.content.get(inline))
            .and_then(Inline::citation_items)
        else {
            return;
        };
        self.citations.items = items;
        self.citations.active = 0;
        self.citations.target = (paragraph, inline);
        self.citations.revision = self.editor.revision();
        self.citations.error.clear();
        self.citations.open.set(1);
    }
    pub(crate) fn insert_citation(&mut self, ui: &mut Ui) {
        let Some(source) = self.active_source() else {
            return;
        };
        let paragraph = self.selected_id();
        let target = self.formatting.selection.filter(|(id, i, _, caret)| {
            *id == paragraph
                && self.current_paragraphs()[self.selected]
                    .content
                    .get(*i)
                    .and_then(Inline::text)
                    .is_some_and(|t| *caret <= t.len() && t.is_char_boundary(*caret))
        });
        let edit = if let Some((_, inline, _, offset)) = target {
            Edit::InsertCitation {
                paragraph,
                inline,
                offset,
                items: vec![CitationItem {
                    source: source.id,
                    locator: self.locator.clone(),
                    ..Default::default()
                }],
            }
        } else {
            Edit::Cite {
                paragraph,
                source: source.id,
                locator: self.locator.clone(),
            }
        };
        if self.apply(edit).is_some() {
            self.widget_ids.clear();
            let suffix = if let Some((_, inline, _, offset)) = target {
                inline + usize::from(offset > 0) + 1
            } else {
                self.current_paragraphs()[self.selected].content.len() - 1
            };
            self.formatting.selection = Some((paragraph, suffix, 0, 0));
            self.formatting.pending_selection = self.formatting.selection;
            let widget = self.text_id(paragraph, suffix);
            ui.focus(widget);
            self.preview_mode = false;
        }
    }
    pub(crate) fn citation_card(
        &mut self,
        ui: &mut Ui,
        paragraph: u64,
        inline: usize,
        widget: u32,
        node: &Inline,
    ) {
        let items = node.citation_items().unwrap();
        ui.text(
            TS_BODY | TX_ACCENT | TX_WRAP,
            &self.editor.document().citation_cluster_text(&items),
        );
        ui.box_begin(0, BOX_H, 6);
        if ui.button(widget + 2, "Zitat bearbeiten") & EV_ACT != 0 {
            self.edit_citation(paragraph, inline);
        }
        if ui.button(widget + 1, "Zitat entfernen") & EV_ACT != 0 {
            if self
                .apply(Edit::RemoveCitation { paragraph, inline })
                .is_some()
            {
                self.widget_ids.clear();
                self.formatting.selection = None;
                self.formatting.pending_selection = None;
            }
        }
        if ui.button(widget + 3, "Quelle zeigen") & EV_ACT != 0 {
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
        ui.box_end();
    }
    pub(crate) fn open_source_uses(&mut self) {
        if let Some(source) = self.active_source() {
            self.citations.use_source = source.id;
            self.citations.use_page = 0;
            self.citations.uses_open.set(1);
        }
    }
    pub(crate) fn citation_dialogs(&mut self, ui: &mut Ui) {
        let edit_was_open = self.citations.open.get() != 0;
        if ui.dialog_begin(950, "Zitat bearbeiten", &self.citations.open) {
            let draft = &mut self.citations;
            let labels: Vec<String> = draft
                .items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let name = self
                        .editor
                        .document()
                        .source(item.source)
                        .map(|s| s.title.as_str())
                        .unwrap_or("Quelle fehlt");
                    format!("{}. {name}", i + 1)
                })
                .collect();
            ui.text(TS_CAPTION, "Quellen im Zitat · Reihenfolge");
            ui.next_size(-1, 0);
            ui.combobox(
                951,
                &mut draft.active,
                &labels.iter().map(String::as_str).collect::<Vec<_>>(),
            );
            let active = draft.active.clamp(0, draft.items.len() as i32 - 1) as usize;
            ui.box_begin(0, BOX_H, 6);
            if draft.items.len() >= 32 {
                ui.next_flags(ST_DISABLED);
            }
            let add = ui.button(952, "Hinzufügen") & EV_ACT != 0;
            if draft.items.len() == 1 {
                ui.next_flags(ST_DISABLED);
            }
            let remove = ui.button(953, "Entfernen") & EV_ACT != 0;
            if active == 0 {
                ui.next_flags(ST_DISABLED);
            }
            let up = ui.button(954, "Aufwärts") & EV_ACT != 0;
            if active + 1 == draft.items.len() {
                ui.next_flags(ST_DISABLED);
            }
            let down = ui.button(955, "Abwärts") & EV_ACT != 0;
            ui.box_end();
            ui.next_size(-1, 300);
            ui.scroll_begin(964, SC_V | BOX_V | pad(4));
            let item = &mut draft.items[active];
            let sources = &self.editor.document().sources;
            let names: Vec<String> = sources
                .iter()
                .map(|s| format!("{} · {} · {}", s.title, s.citation_author(), s.year))
                .collect();
            let mut chosen = sources
                .iter()
                .position(|s| s.id == item.source)
                .unwrap_or(0) as i32;
            ui.text(TS_CAPTION, "Quelle");
            ui.next_size(-1, 0);
            if ui.combobox(
                956,
                &mut chosen,
                &names.iter().map(String::as_str).collect::<Vec<_>>(),
            ) & EV_ACT
                != 0
            {
                item.source = sources[chosen as usize].id;
            }
            for (id, label, value) in [
                (957, "Seite / Fundstelle", &mut item.locator),
                (958, "Präfix (Abstände mit eingeben)", &mut item.prefix),
                (959, "Suffix (Abstände mit eingeben)", &mut item.suffix),
            ] {
                ui.text(TS_CAPTION, label);
                ui.next_size(-1, 0);
                ui.textbox(id, None, value, 129);
            }
            for (id, label, value) in [
                (960, "Autor ausblenden", &mut item.suppress_author),
                (961, "Jahr ausblenden", &mut item.suppress_year),
                (
                    962,
                    "Titel bei anonymen Quellen ausblenden",
                    &mut item.suppress_title,
                ),
            ] {
                let mut checked = i32::from(*value);
                ui.checkbox(id, label, &mut checked);
                *value = checked != 0;
            }
            ui.scroll_end();
            if add && draft.items.len() < 32 {
                draft.items.push(CitationItem {
                    source: draft.items[active].source,
                    ..Default::default()
                });
                draft.active = draft.items.len() as i32 - 1;
            } else if remove && draft.items.len() > 1 {
                draft.items.remove(active);
                draft.active = active.min(draft.items.len() - 1) as i32;
            } else if up && active > 0 {
                draft.items.swap(active, active - 1);
                draft.active -= 1;
            } else if down && active + 1 < draft.items.len() {
                draft.items.swap(active, active + 1);
                draft.active += 1;
            }
            ui.text(TS_CAPTION, "Vorschau · Autor/Jahr-Basisstil");
            ui.next_size(-1, 60);
            ui.scroll_begin(963, SC_V | BOX_V);
            ui.text(
                TS_BODY | TX_WRAP,
                &self.editor.document().citation_cluster_text(&draft.items),
            );
            ui.scroll_end();
            ui.next_size(-1, 36);
            ui.text(TS_CAPTION | TX_WRAP, if draft.error.is_empty() { "Alle Einträge werden gemeinsam übernommen. Abbrechen lässt das Zitat unverändert." } else { &draft.error });
            match ui.dialog_buttons(Some("Zitat übernehmen"), None, Some("Abbrechen")) {
                DLG_PRIMARY => {
                    let (paragraph, inline) = draft.target;
                    if draft.revision != self.editor.revision() {
                        draft.error = String::from(
                            "Das Dokument wurde verändert. Bitte das Zitat erneut öffnen.",
                        );
                        draft.open.set(1);
                    } else {
                        let items = draft.items.clone();
                        if self
                            .apply(Edit::UpdateCitation {
                                paragraph,
                                inline,
                                items,
                            })
                            .is_some()
                        {
                            self.formatting.selection = None;
                            self.formatting.pending_selection = None;
                            self.citations.focus_target = Some((paragraph, inline));
                        } else {
                            self.citations.error = String::from(
                                "Ungültige Angaben: maximal 128 UTF-8-Bytes je Feld; keine Anführungszeichen, Backslashes oder Zeilenumbrüche.",
                            );
                            self.citations.open.set(1);
                        }
                    }
                }
                DLG_CLOSE => {
                    self.citations.focus_target = Some(self.citations.target);
                    self.message =
                        String::from("Zitatbearbeitung abgebrochen · Dokument unverändert")
                }
                _ => (),
            }
        }
        ui.dialog_end();
        let uses_was_open = self.citations.uses_open.get() != 0;
        if ui.dialog_begin(980, "Verwendungen der Quelle", &self.citations.uses_open) {
            let doc = self.editor.document();
            let uses = doc.source_uses(self.citations.use_source);
            ui.next_size(-1, 60);
            ui.scroll_begin(984, SC_V | BOX_V);
            if let Some(s) = doc.source(self.citations.use_source) {
                ui.text(TS_BODY_STRONG | TX_WRAP, &s.title);
            }
            ui.scroll_end();
            let pages = uses.len().div_ceil(8).max(1);
            self.citations.use_page = self.citations.use_page.min(pages - 1);
            ui.text(
                TS_CAPTION,
                &format!(
                    "{} Zitatfelder · Seite {} von {pages}",
                    uses.len(),
                    self.citations.use_page + 1
                ),
            );
            ui.next_size(-1, 280);
            ui.scroll_begin(983, SC_V | BOX_V);
            if uses.is_empty() {
                ui.text(TS_BODY, "Diese Quelle wird noch nicht zitiert.");
            }
            let mut jump = None;
            for (i, at) in uses
                .iter()
                .skip(self.citations.use_page * 8)
                .take(8)
                .enumerate()
            {
                let label = self.paragraph_label(at.paragraph);
                ui.next_size(-1, 0);
                if ui.button(
                    1900 + i as u32,
                    &format!("{label} · Zitat an Position {}", at.inline + 1),
                ) & EV_ACT
                    != 0
                {
                    jump = Some((at.paragraph, at.inline));
                }
            }
            ui.scroll_end();
            ui.box_begin(0, BOX_H, 6);
            if self.citations.use_page == 0 {
                ui.next_flags(ST_DISABLED);
            }
            if ui.button(981, "Zurück") & EV_ACT != 0 {
                self.citations.use_page = self.citations.use_page.saturating_sub(1);
            }
            if self.citations.use_page + 1 == pages {
                ui.next_flags(ST_DISABLED);
            }
            if ui.button(982, "Weiter") & EV_ACT != 0 && self.citations.use_page + 1 < pages {
                self.citations.use_page += 1;
            }
            ui.box_end();
            ui.dialog_buttons(None, None, Some("Schließen"));
            if let Some((paragraph, inline)) = jump {
                self.select_paragraph(paragraph);
                self.citations.uses_open.set(0);
                self.citations.focus_target = Some((paragraph, inline));
                self.message = format!(
                    "Verwendung in {} ausgewählt",
                    self.paragraph_label(paragraph)
                );
            }
        }
        ui.dialog_end();
        // Let the closed dialog restore its old focus before requesting the new target.
        if !uses_was_open && !edit_was_open {
            if let Some((paragraph, inline)) = self.citations.focus_target.take() {
                if self.structured_edit {
                    let widget = self.text_id(paragraph, inline);
                    ui.focus(widget + 2);
                } else {
                    self.rich_anchor(ui, paragraph, inline);
                }
            }
        }
    }
}
