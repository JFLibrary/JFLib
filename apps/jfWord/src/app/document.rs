// jfWord editing workflows, shared editor transactions and selection.
use crate::*;
impl Word {
    pub(crate) fn repair_story_owner(&mut self) {
        let d=self.editor.document();
        if self.active_note.is_some_and(|id|d.note(id).is_none()) || self.active_cell.is_some_and(|id|d.cell(id).is_none()) {
            self.active_note=None;self.active_cell=None;self.selected=0;
            self.formatting.selection=None;
        }
    }
    pub(crate) fn text_id(&mut self, paragraph: u64, inline: usize) -> u32 {
        // Navigation controls (usize::MAX) belong to individual note paragraphs.
        // Only the editing surface shares an ID across a continuous story.
        let surface = !self.structured_edit && inline != usize::MAX;
        let inline = if surface { 0 } else { inline };
        let paragraph = if !surface {
            paragraph
        } else {
            self.editor.document().paragraph_group(paragraph).unwrap_or(&[])
                .first()
                .map_or(paragraph, |p| p.id)
        };
        if let Some((_, _, id)) = self
            .widget_ids
            .iter()
            .find(|(p, i, _)| *p == paragraph && *i == inline)
        {
            return *id;
        }
        if (0x100000..0x104000).contains(&self.next_widget) {
            self.next_widget = 0x104000;
        }
        if (0x200000..0x200100).contains(&self.next_widget) {
            self.next_widget = 0x200100;
        }
        if (0x300000..0x300800).contains(&self.next_widget) {
            self.next_widget = 0x300800;
        }
        let id = self.next_widget;
        self.next_widget += 8;
        self.widget_ids.push((paragraph, inline, id));
        id
    }
    pub(crate) fn apply(&mut self, edit: Edit) -> Option<u64> {
        match self.editor.apply(edit) {
            Ok(id) => {
                self.repair_story_owner();
                self.message = String::from("Änderung übernommen");
                Some(id)
            }
            Err(e) => {
                self.message = match e {
                    doc_core::Error::InUse => {
                        String::from("Quelle wird zitiert. Zitate zuerst entfernen.")
                    }
                    _ => format!("Änderung nicht übernommen: {e:?}"),
                };
                None
            }
        }
    }
    pub(crate) fn history(&mut self, ui: &mut Ui, redo: bool) {
        let before = self.editor.state_id();
        let rich_caret = self.rich_caret_snapshot();
        let source = self.active_source().map(|s| s.id);
        let note_anchor = self
            .active_note
            .and_then(|id| self.editor.document().note_location(id));
        let focused = self
            .widget_ids
            .iter()
            .find(|(_, _, id)| *id == ui.focused())
            .copied();
        let changed = if redo {
            self.editor.redo()
        } else {
            self.editor.undo()
        };
        if !changed {
            return;
        }
        if self.active_cell.is_some_and(|id|self.editor.document().cell(id).is_none()) { self.active_cell=None;self.selected=0; }
        if let Some(index) = source.and_then(|id| {
            self.editor
                .document()
                .sources
                .iter()
                .position(|s| s.id == id)
        }) {
            self.source_sel = index as i32;
        }
        self.widget_ids.clear();
        if self
            .active_note
            .is_some_and(|id| self.editor.document().note(id).is_none())
        {
            if let Some(at) = note_anchor {
                if self.select_paragraph(at.paragraph) {
                    let paragraph = self.editor.document().paragraph(at.paragraph).unwrap();
                    if let Some(inline) = paragraph
                        .content
                        .iter()
                        .position(|node| node.text().is_some())
                    {
                        let widget = self.text_id(at.paragraph, inline);
                        ui.focus(widget);
                    }
                }
            }
        }
        if let Some((paragraph, inline, _)) = focused {
            if self
                .editor
                .document()
                .paragraph(paragraph)
                .is_some_and(|p| {
                    p.content.get(inline).is_some_and(|i| i.text().is_some())
                        || (inline == 0 && p.content.is_empty())
                })
            {
                self.select_paragraph(paragraph);
                let id = self.text_id(paragraph, inline);
                ui.focus(id);
            }
        }
        self.message = String::from(if redo { "Wiederholt" } else { "Rückgängig" });
        self.formatting.selection = None;
        self.formatting.pending_selection = None;
        self.restore_edit_focus(ui, before, self.editor.state_id());
        if !self.structured_edit {
            self.rich_restore(ui, before, self.editor.state_id(), rich_caret);
        }
        self.repair_story_owner();
    }
    pub(crate) fn selected_id(&self) -> u64 {
        self.current_paragraphs()[self.selected.min(self.current_paragraphs().len() - 1)].id
    }
    pub(crate) fn active_source(&self) -> Option<Source> {
        self.editor
            .document()
            .sources
            .get(usize::try_from(self.source_sel).ok()?)
            .filter(|s| s.matches_query(&self.source_query))
            .cloned()
    }
    pub(crate) fn choose_file(&mut self, save: bool) {
        #[cfg(target_arch = "wasm32")]
        if !save {
            if plat::web::pick_file(".docx,.rtf,.html,.htm").is_none() { self.message = String::from("Dateiauswahl ist momentan nicht verfügbar."); }
            return;
        }
        self.dialog_path = if self.path.is_empty() {
            format!("{}/Dokument.docx", plat::cwd())
        } else {
            self.path.clone()
        };
        self.save_dialog = save;
        self.source_file = false;
        self.file_open.set(1);
    }
    pub(crate) fn choose_sources(&mut self, save: bool) {
        let extension = ["xml", "ris", "bib"][self.source_format.clamp(0, 2) as usize];
        self.dialog_path = format!("{}/Quellen.{extension}", plat::cwd());
        self.save_dialog = save;
        self.source_file = true;
        self.file_open.set(1);
    }
    pub(crate) fn write_selected_file(&mut self, ui: &mut Ui) {
        if !self.source_file {
            self.save(ui, self.dialog_path.clone());
            return;
        }
        self.message =
            match doc_io::save_bibliography(&self.dialog_path, &self.editor.document().sources) {
                Ok(()) => format!(
                    "{} Quellen exportiert. Das Dokument wurde dadurch nicht gespeichert.",
                    self.editor.document().sources.len()
                ),
                Err(e) => format!("Quellenexport fehlgeschlagen: {e:?}"),
            };
    }
    pub(crate) fn perform(&mut self, ui: &mut Ui, cmd: u32) {
        match cmd {
            NEW | NEW_REPORT | NEW_RESEARCH | NEW_LETTER | NEW_RESUME => {
                self.document_epoch = self.document_epoch.wrapping_add(1);
                self.editor = Editor::new(shell_ui::template(cmd)).unwrap();
                self.document_file = None;
                self.file_ui = Default::default();
                self.shell.home = false;
                self.shell.has_document = true;
                self.preview.reset();
                self.preview_mode = false;
                self.tab = 1;
                self.jump_to_selected = false;
                self.reset_paper_view = true;
                self.formatting = Default::default();
                self.editing = Default::default();
                self.rich = Default::default();
                self.rich_request_focus();
                self.source_import = Default::default();
                self.citations = Default::default();
                self.source_tools = Default::default();
                self.source_query.clear();
                self.source_sel = 0;
                self.path.clear();
                self.selected = 0;
                self.active_note = None;
                self.active_cell = None;
                self.notes_ui = Default::default();
                self.widget_ids.clear();
                self.message = String::from("Neues Dokument");
                let field = self.text_id(self.selected_id(), 0);
                ui.focus(field);
                ui.scroll_to(110, 0, 0);
                ui.scroll_to(129, 0, 0);
            }
            OPEN => self.choose_file(false),
            OPEN_RECENT => {
                let path = core::mem::take(&mut self.shell.open_target);
                if let Err(error) = self.open_document(ui, &path) {
                    self.message = format!(
                        "Öffnen fehlgeschlagen: {error:?}. Das aktuelle Dokument bleibt erhalten."
                    );
                }
            }
            UIC_CLOSE => ui.quit(),
            _ => (),
        }
    }
    pub(crate) fn save(&mut self, ui: &mut Ui, path: String) -> bool {
        #[cfg(target_arch = "wasm32")]
        if self.browser_save.is_some() { self.message = String::from("Speicherung läuft noch."); return false; }
        if !path.to_ascii_lowercase().ends_with(".docx") {
            self.message = String::from("Bitte als .docx speichern.");
            self.after_save = false;
            return false;
        }
        let preserved = self.preserved();
        let result = if let Some(file) = &self.document_file {
            file.save(
                &path,
                if preserved {
                    None
                } else {
                    Some(self.editor.document())
                },
            )
        } else {
            doc_io::save_docx(&path, self.editor.document())
        };
        match result {
            Ok(()) => {
                #[cfg(target_arch = "wasm32")]
                { self.start_browser_save(ui, path) }
                #[cfg(not(target_arch = "wasm32"))]
                { self.finish_save(ui, path, preserved); true }
            }
            Err(e) => {
                self.after_save = false;
                self.message = format!("Speichern fehlgeschlagen: {e:?}");
                false
            }
        }
    }
    pub(crate) fn finish_save(&mut self, ui: &mut Ui, path: String, preserved: bool) {
        if self.path != path || self.document_dirty() {
            self.document_epoch = self.document_epoch.wrapping_add(1);
        }
        self.path = path;
        self.shell.has_document = true;
        self.editor.mark_saved();
        if let Some(file) = &mut self.document_file { file.mark_saved(); }
        self.shell.remember(&self.path);
        self.message = String::from(if preserved {
            "Gespeichert · Dokumentpaket mit allen enthaltenen Teilen"
        } else { "Gespeichert · Text, Quellen und Zitatverweise" });
        if self.after_save {
            self.after_save = false;
            let next = core::mem::take(&mut self.pending);
            self.perform(ui, next);
        }
    }
    pub(crate) fn page(&mut self, ui: &mut Ui) {
        if self.preview_mode {
            if let Some(id) = self
                .preview
                .show(ui, self.editor.document(), self.editor.revision())
            {
                self.select_paragraph(id);
            }
            return;
        }
        ui.next_size(-1, -1);
        ui.scroll_begin(110, SC_V | BOX_V | BOX_CARD | pad(24));
        let viewport = ui.last_rect();
        self.note_editor_header(ui);
        let mut paragraphs = self.current_paragraphs().to_vec();
        // An imported empty paragraph has no runs. Expose a virtual text slot without
        // modifying the document until the user types (SetText then creates the run).
        for paragraph in &mut paragraphs {
            if paragraph.content.is_empty() {
                paragraph.content.push(Inline::Text(String::new()));
            }
        }
        for (pi, p) in paragraphs.iter().enumerate() {
            let ts = match p.style {
                ParagraphStyle::Title => TS_TITLE,
                ParagraphStyle::Heading1 => TS_SUBTITLE,
                ParagraphStyle::Heading2 => TS_BODY_STRONG,
                _ => TS_BODY,
            };
            if self.selected == pi {
                ui.text(
                    TS_CAPTION | TX_SECONDARY,
                    &format!("Absatz {} · {}", pi + 1, style_label(p.style)),
                );
                if self.jump_to_selected {
                    let y = ui.last_rect()[1];
                    let current = ui.scroll_pos(110).1;
                    ui.scroll_to(110, -1, (current + ui.px2dip(y - viewport[1]) - 32).max(0));
                    self.jump_to_selected = false;
                }
                if p.format.page_break_before {
                    ui.text(TS_CAPTION | TX_ACCENT, "Seitenwechsel vor diesem Absatz");
                }
                for (ii, inline) in p.content.iter().enumerate() {
                    let widget = self.text_id(p.id, ii);
                    match inline {
                        Inline::Text(t) | Inline::StyledText { text: t, .. } => {
                            let mut t = t.clone();
                            if let Inline::StyledText { format, .. } = inline {
                                let f = self.editor.document().resolve_paragraph_text(&p, *format);
                                ui.text(
                                    TS_CAPTION | TX_SECONDARY,
                                    &format!(
                                        "Textsegment · {} pt{}{}{}",
                                        f.size_half_points as f32 / 2.0,
                                        if f.bold { " · Fett" } else { "" },
                                        if f.italic { " · Kursiv" } else { "" },
                                        if f.underline { " · Unterstrichen" } else { "" }
                                    ),
                                );
                            }
                            ui.next_size(-1, if p.content.len() == 1 { 160 } else { 70 });
                            if ui.textbox(
                                widget,
                                Some("Text schreiben …"),
                                &mut t,
                                (2 << 20) | TB_MULTI | TB_WRAP,
                            ) & EV_ACT
                                != 0
                            {
                                self.apply(Edit::SetText {
                                    paragraph: p.id,
                                    inline: ii,
                                    text: t.clone(),
                                });
                            }
                            if let Some((id, at, start, end)) = self.formatting.pending_selection {
                                if id == p.id
                                    && at == ii
                                    && ui.textbox_select(widget, &t, start, end)
                                {
                                    self.formatting.pending_selection = None;
                                }
                            }
                            if let Some((start, end)) = ui.textbox_selection(widget) {
                                self.formatting.selection = Some((p.id, ii, start, end));
                            }
                        }
                        Inline::Citation { .. } | Inline::CitationCluster(_) => {
                            self.citation_card(ui, p.id, ii, widget, inline)
                        }
                        Inline::Bibliography => {
                            ui.text(TS_SUBTITLE, "Literaturverzeichnis");
                            ui.text(
                                TS_CAPTION | TX_SECONDARY,
                                "Automatisch aus den zitierten Quellen",
                            );
                            for row in self.editor.document().bibliography() {
                                ui.text(TS_BODY | TX_WRAP, &row);
                            }
                        }
                        Inline::NoteReference(id) => self.note_card(ui, *id, widget),
                        Inline::Formula(f)=>{
                            ui.text(TS_BODY,&self.editor.document().evaluate_formula(p.id,f).text);
                            if ui.button(widget+4,"Formel bearbeiten")&EV_ACT!=0{self.open_formula(Some((p.id,ii)));}
                        },
                    }
                }
            } else {
                ui.text(ts | TX_WRAP, &self.editor.document().paragraph_text(p));
            }
            ui.text(TS_CAPTION, "");
        }
        ui.scroll_end();
    }
    pub(crate) fn sources_pane(&mut self, ui: &mut Ui) {
        ui.next_size(286, -1);
        ui.scroll_begin(120, SC_V | BOX_V | BOX_CARD | pad(16));
        ui.text(TS_SUBTITLE, "Quellen");
        ui.text(TS_CAPTION | TX_SECONDARY, "Im Dokument gespeichert");
        ui.next_size(-1, 0);
        ui.textbox(407, Some("Quellen suchen …"), &mut self.source_query, 257);
        let titles: Vec<(usize, String)> = self
            .editor
            .document()
            .sources
            .iter()
            .enumerate()
            .filter(|(_, s)| s.matches_query(&self.source_query))
            .map(|(i, s)| (i, s.title.clone()))
            .collect();
        if titles.is_empty() {
            ui.text(
                TS_BODY | TX_WRAP,
                if self.editor.document().sources.is_empty() {
                    "Bücher, Artikel und Webseiten erfassen oder aus Sources.xml, RIS und BibTeX importieren."
                } else { "Keine Quellen zum Suchtext gefunden." },
            );
        } else {
            let mut selected = titles
                .iter()
                .position(|(i, _)| *i as i32 == self.source_sel)
                .unwrap_or(0) as i32;
            ui.next_size(-1, 0);
            ui.combobox(
                400,
                &mut selected,
                &titles.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(),
            );
            self.source_sel = titles[selected as usize].0 as i32;
            if let Some(s) = self.active_source() {
                ui.text(
                    TS_BODY | TX_WRAP,
                    &format!("{} · {}", s.author_text(), s.year),
                );
                ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &s.publisher);
            }
        }
        if ui.button(401, "Quelle anlegen") & EV_ACT != 0 {
            ui.cmd_run(SOURCE_NEW, 0);
        }
        ui.box_begin(0, BOX_H, 6);
        if ui.button(404, "Importieren") & EV_ACT != 0 {
            ui.cmd_run(SOURCE_IMPORT, 0);
        }
        if self.editor.document().sources.is_empty() {
            ui.next_flags(ST_DISABLED);
        }
        if ui.button(405, "Exportieren") & EV_ACT != 0 {
            ui.cmd_run(SOURCE_EXPORT, 0);
        }
        ui.box_end();
        ui.text(TS_CAPTION, "Exportformat");
        ui.next_size(-1, 0);
        ui.combobox(406, &mut self.source_format, &["Sources.xml", "RIS", "BibTeX"]);
        if !titles.is_empty() {
            ui.text(TS_CAPTION, "Seite / Fundstelle");
            ui.next_size(-1, 0);
            ui.textbox(402, Some("z. B. 42–44"), &mut self.locator, 129);
            if ui.accent_button(403, "Im Absatz zitieren") & EV_ACT != 0 {
                ui.cmd_run(CITE, 0);
            }
            if ui.button(408, "Verwendungen anzeigen") & EV_ACT != 0 {
                self.open_source_uses();
            }
        }
        ui.box_begin(0, BOX_H, 6);
        if self.editor.document().sources.len() < 2 || self.active_source().is_none() {
            ui.next_flags(ST_DISABLED);
        }
        if ui.button(409, "Zusammenführen") & EV_ACT != 0 {
            ui.cmd_run(SOURCE_MERGE, 0);
        }
        if ui.button(410, "Prüfen") & EV_ACT != 0 {
            ui.cmd_run(SOURCE_AUDIT, 0);
        }
        ui.box_end();
        ui.next_size(-1, 0);
        if ui.button(411, "Quellenbibliothek") & EV_ACT != 0 {
            ui.cmd_run(SOURCE_LIBRARY, 0);
        }
        ui.text(TS_BODY_STRONG, "Zitierte Literatur");
        ui.text(
            TS_CAPTION | TX_SECONDARY | TX_WRAP,
            "Live-Vorschau · Autor/Jahr-Basisstil",
        );
        let rows = self.editor.document().bibliography();
        if rows.is_empty() {
            ui.text(TS_BODY | TX_SECONDARY, "Noch keine Zitate.");
        }
        for row in rows {
            ui.text(TS_BODY | TX_WRAP, &row);
        }
        ui.scroll_end();
    }
    pub(crate) fn dialogs(&mut self, ui: &mut Ui) {
        let was_open = self.file_open.get() != 0;
        if ui.filedlg(
            0x100000,
            if self.save_dialog { FD_SAVE } else { FD_OPEN },
            &self.file_open,
            &mut self.dialog_path,
            4096,
            if self.source_file {
                &[
                    "xml=Quellenliste (Sources.xml)",
                    "ris=RIS (.ris)",
                    "bib;bibtex=BibTeX (.bib)",
                ]
            } else if self.save_dialog {
                &["docx=DOCX-Dokument (.docx)"]
            } else {
                &["docx;rtf;html;htm=Dokumente (.docx, .rtf, .html)","docx=DOCX-Dokument (.docx)","rtf=Rich Text (.rtf)","html;htm=HTML (.html)"]
            },
        ) & EV_ACT
            != 0
        {
            ui.cmd_run(if self.save_dialog { DO_SAVE } else { DO_OPEN }, 0);
        } else if was_open && self.file_open.get() == 0 {
            self.after_save = false;
            self.pending = 0;
        }
        let source_title = if self.source_is_library {
            "Bibliotheksquelle bearbeiten"
        } else {
            "Quelle bearbeiten"
        };
        if let Some(source) = self.draft.show(ui, &self.source_open, source_title) {
            if self.source_is_library {
                if self.finish_library_source(source) {
                    self.source_is_library = false;
                } else {
                    self.source_open.set(1);
                }
            } else if let Some(id) = self.apply(Edit::PutSource(source)) {
                self.source_sel = self
                    .editor
                    .document()
                    .sources
                    .iter()
                    .position(|s| s.id == id)
                    .unwrap_or(0) as i32;
            } else {
                self.source_open.set(1);
            }
        }
        self.formatting_dialogs(ui);
        self.tab_dialog(ui);
        self.table_dialog(ui);
        self.formula_dialog(ui);
        self.style_dialog(ui);
        self.source_import_dialog(ui);
        self.citation_dialogs(ui);
        self.source_tools_dialogs(ui);
        self.library_dialogs(ui);
        self.note_dialogs(ui);
        self.file_dialogs(ui);
        if ui.dialog_begin(900, "Datei ersetzen", &self.overwrite_open) {
            ui.text(
                TS_BODY | TX_WRAP,
                "Diese Datei existiert bereits. Durch den aktuellen Export ersetzen?",
            );
            ui.text(TS_CAPTION | TX_WRAP, &self.dialog_path);
            match ui.dialog_buttons(Some("Ersetzen"), None, Some("Abbrechen")) {
                DLG_PRIMARY => {
                    ui.cmd_run(OVERWRITE, 0);
                }
                DLG_CLOSE => {
                    self.after_save = false;
                    self.pending = 0;
                }
                _ => (),
            }
        }
        ui.dialog_end();
        if ui.dialog_begin(800, "Ungespeicherte Änderungen", &self.guard_open) {
            ui.text(
                TS_BODY | TX_WRAP,
                "Das Dokument wurde geändert. Vor dem Fortfahren speichern?",
            );
            match ui.dialog_buttons(Some("Speichern"), Some("Verwerfen"), Some("Abbrechen")) {
                DLG_PRIMARY => {
                    self.after_save = true;
                    ui.cmd_run(SAVE, 0);
                }
                DLG_SECONDARY => {
                    let next = core::mem::take(&mut self.pending);
                    self.perform(ui, next);
                }
                DLG_CLOSE => {
                    self.pending = 0;
                }
                _ => (),
            }
        }
        ui.dialog_end();
    }
}
pub(crate) fn style_label(s: ParagraphStyle) -> &'static str {
    match s {
        ParagraphStyle::Normal => "Text",
        ParagraphStyle::Title => "Titel",
        ParagraphStyle::Heading1 => "Überschrift 1",
        ParagraphStyle::Heading2 => "Überschrift 2",
        ParagraphStyle::Named(_) => "Benutzerdefiniert",
    }
}
