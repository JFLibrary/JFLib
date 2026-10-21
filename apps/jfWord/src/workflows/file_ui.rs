// Loss-aware DOCX sessions. Foreign files retain their original package, never a flattened model.
use crate::Word;
use alloc::{format, string::String, vec::Vec};
use doc_core::{Document, Editor};
use doc_io::{DocxFile, DocxMode, DocxStory};
use ui::*;

pub(crate) struct FileUi {
    pub(crate) info_open: Shared<i32>,
    text_open: Shared<i32>,
    run: usize,
    before: String,
    replacement: String,
    error: String,
    query: String,
    page: usize,
    story: i32,
}
impl Default for FileUi {
    fn default() -> Self {
        Self {
            info_open: Shared::new(0),
            text_open: Shared::new(0),
            run: 0,
            before: String::new(),
            replacement: String::new(),
            error: String::new(),
            query: String::new(),
            page: 0,
            story: 0,
        }
    }
}
fn story_label(story: DocxStory) -> &'static str {
    match story {
        DocxStory::Main => "Haupttext",
        DocxStory::Header => "Kopfzeile",
        DocxStory::Footer => "Fußzeile",
        DocxStory::Footnote => "Fußnote",
        DocxStory::Endnote => "Endnote",
    }
}
impl FileUi {
    pub(crate) fn modal(&self) -> bool {
        self.info_open.get() != 0 || self.text_open.get() != 0
    }
    pub(crate) fn cancel(&self) {
        self.info_open.set(0);
        self.text_open.set(0);
    }
}
impl Word {
    pub(crate) fn preserved(&self) -> bool {
        self.document_file
            .as_ref()
            .is_some_and(|file| file.report().mode == DocxMode::Preserved)
    }
    pub(crate) fn document_dirty(&self) -> bool {
        if self.preserved() {
            self.document_file.as_ref().is_some_and(DocxFile::dirty)
        } else {
            self.editor.dirty()
        }
    }
    pub(crate) fn open_document(&mut self, ui: &mut Ui, path: &str) -> Result<(), doc_io::Error> {
        let lower=path.to_ascii_lowercase();
        let imported=lower.ends_with(".rtf")||lower.ends_with(".html")||lower.ends_with(".htm");
        let (file,editor)=if imported {
            let bytes=doc_io::read_file(path,16<<20)?;
            let document=if lower.ends_with(".rtf"){doc_io::clipboard_rtf_decode(&bytes)?}else{doc_io::clipboard_html_decode(&bytes)?};
            let mut editor=Editor::new(Document{page:document.page,..Document::default()})?;
            editor.replace_story_fragment(1,0,0,doc_core::DocumentFragment{document})?;(None,editor)
        }else{let file=DocxFile::load(path)?;let editor=Editor::new(file.document().cloned().unwrap_or_else(Document::default))?;(Some(file),editor)};
        if editor.document().tables.iter().any(|t|t.position.is_some()) {
            let fonts=self.preview.prepare_fonts(&mut ui.text).map_err(|_|doc_io::Error::Limit)?;
            doc_view::paginate(editor.document(),&mut ui.text,&fonts).map_err(|_|doc_io::Error::Limit)?;
        }
        self.document_epoch = self.document_epoch.wrapping_add(1);
        self.editor = editor;
        self.document_file = file;
        self.preview.reset();
        self.formatting = Default::default();
        self.editing = Default::default();
        self.rich = Default::default();
        self.rich_request_focus();
        self.source_import = Default::default();
        self.citations = Default::default();
        self.source_tools = Default::default();
        self.notes_ui = Default::default();
        self.file_ui = Default::default();
        self.source_query.clear();
        self.source_sel = 0;
        self.path = if imported {String::new()} else {String::from(path)};
        self.dialog_path = self.path.clone();
        self.selected = 0;
        self.active_note = None;
                self.active_cell = None;
        self.widget_ids.clear();
        self.jump_to_selected = false;
        self.reset_paper_view = true;
        for scroll in [104, 110, 129, 2402, 2403] {
            ui.scroll_to(scroll, 0, 0);
        }
        self.shell.home = false;
        self.shell.has_document = true;
        self.shell.remember(path);
        self.message = String::from(if imported {"RTF/HTML importiert · als DOCX speichern"} else if self.preserved() {
            "DOCX geöffnet · Originalteile erhalten · eingeschränkte Textbearbeitung"
        } else {
            "Dokument geöffnet · Quellen und Zitatverweise geladen"
        });
        Ok(())
    }
    pub(crate) fn preserved_view(&mut self, ui: &mut Ui) {
        let Some(file) = &self.document_file else {
            return;
        };
        ui.next_size(-1, 82);
        ui.box_begin(2400, BOX_V | BOX_CARD | pad(12), 4);
        ui.text(TS_BODY_STRONG, "DOCX-Dokument · Originalinhalt erhalten");
        ui.text(TS_BODY | TX_WRAP,"Textauszug: Seitenlayout und Objekte sind hier nicht vollständig dargestellt. Unterstützte Textbereiche können einzeln geändert werden; alle übrigen Inhalte bleiben erhalten.");
        ui.box_end();
        ui.next_size(-1, -1);
        ui.box_begin(2401, BOX_H | pad(14), 16);
        ui.next_size(-1, -1);
        ui.scroll_begin(2402, SC_V | BOX_V | BOX_CARD | pad(24));
        ui.text(TS_SUBTITLE, "Textauszug");
        if file.preview().is_empty() {
            ui.text(TS_BODY | TX_WRAP,"Kein darstellbarer Haupttext. Die enthaltenen Objekte bleiben in der Datei gespeichert.");
        }
        for paragraph in file.preview() {
            ui.text(TS_BODY | TX_WRAP, paragraph);
            ui.spacer(8);
        }
        if file.report().preview_truncated {
            ui.text(TS_CAPTION | TX_WRAP,"Der Textauszug ist gekürzt. Beim Speichern bleibt das vollständige Paket erhalten.");
        }
        ui.scroll_end();
        ui.next_size(330, -1);
        ui.scroll_begin(2403, SC_V | BOX_V | BOX_CARD | pad(14));
        ui.text(TS_SUBTITLE, "Textbereiche");
        ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP,"Formatierung und Struktur bleiben erhalten. Neue Absätze, Feldänderungen und Layoutbearbeitung sind in diesem Modus noch nicht verfügbar.");
        ui.next_size(-1, 28);
        ui.textbox(
            2404,
            Some("Textbereich suchen …"),
            &mut self.file_ui.query,
            256,
        );
        ui.next_size(-1, 28);
        if ui.combobox(
            2405,
            &mut self.file_ui.story,
            &[
                "Alle Dokumentbereiche",
                "Haupttext",
                "Kopfzeilen",
                "Fußzeilen",
                "Fußnoten",
                "Endnoten",
            ],
        ) & EV_ACT
            != 0
        {
            self.file_ui.page = 0;
        }
        let scope = match self.file_ui.story {
            1 => Some(DocxStory::Main),
            2 => Some(DocxStory::Header),
            3 => Some(DocxStory::Footer),
            4 => Some(DocxStory::Footnote),
            5 => Some(DocxStory::Endnote),
            _ => None,
        };
        let query = self.file_ui.query.to_lowercase();
        let runs: Vec<_> = file
            .text_runs()
            .iter()
            .filter(|run| {
                scope.is_none_or(|story| story == run.story)
                    && run.text.to_lowercase().contains(&query)
            })
            .collect();
        let pages = runs.len().div_ceil(20).max(1);
        self.file_ui.page = self.file_ui.page.min(pages - 1);
        let mut edit = None;
        ui.text(
            TS_CAPTION,
            &format!(
                "{} Bereiche · Seite {} von {}",
                runs.len(),
                self.file_ui.page + 1,
                pages
            ),
        );
        for (row, run) in runs
            .iter()
            .skip(self.file_ui.page * 20)
            .take(20)
            .enumerate()
        {
            ui.text(
                TS_CAPTION | TX_SECONDARY,
                &format!("{} · Absatz {}", story_label(run.story), run.paragraph),
            );
            let label: String = run.text.chars().take(42).collect();
            ui.next_size(-1, 30);
            if ui.button(
                2420 + row as u32,
                if label.is_empty() {
                    "Leerer Textbereich"
                } else {
                    &label
                },
            ) & EV_ACT
                != 0
            {
                edit = Some((run.index, run.text.clone()));
            }
        }
        if runs.is_empty() {
            ui.text(
                TS_BODY | TX_WRAP,
                "Keine passenden sicher bearbeitbaren Textbereiche.",
            );
        }
        ui.box_begin(2442, BOX_H, 6);
        if self.file_ui.page == 0 {
            ui.next_flags(ST_DISABLED);
        }
        if ui.button(2443, "Zurück") & EV_ACT != 0 {
            self.file_ui.page = self.file_ui.page.saturating_sub(1);
        }
        if self.file_ui.page + 1 == pages {
            ui.next_flags(ST_DISABLED);
        }
        if ui.button(2444, "Weiter") & EV_ACT != 0 {
            self.file_ui.page += 1;
        }
        ui.box_end();
        ui.next_size(-1, 30);
        if ui.button(2445, "Dateiinformationen") & EV_ACT != 0 {
            self.file_ui.info_open.set(1);
        }
        ui.scroll_end();
        ui.box_end();
        if let Some((run, text)) = edit {
            self.file_ui.run = run;
            self.file_ui.before = text.clone();
            self.file_ui.replacement = text;
            self.file_ui.error.clear();
            self.file_ui.text_open.set(1);
        }
    }
    pub(crate) fn file_dialogs(&mut self, ui: &mut Ui) {
        if ui.dialog_begin(2450, "Dateiinformationen", &self.file_ui.info_open) {
            ui.next_size(440, 0);
            ui.box_begin(0, BOX_V, 8);
            ui.text(
                TS_BODY_STRONG | TX_WRAP,
                if self.path.is_empty() {
                    "Neues Dokument"
                } else {
                    &self.path
                },
            );
            if let Some(file) = &self.document_file {
                ui.text(
                    TS_BODY | TX_WRAP,
                    if self.preserved() {
                        "Erhaltungsmodus mit begrenzter Textbearbeitung"
                    } else {
                        "Vollständig im aktuellen jfWord-Dokumentmodell bearbeitbar"
                    },
                );
                ui.text(
                    TS_CAPTION,
                    &format!("{} Paketteile", file.report().part_count),
                );
                ui.next_size(-1, 240);
                ui.scroll_begin(2451, SC_V | BOX_V);
                for issue in &file.report().diagnostics {
                    ui.text(TS_BODY | TX_WRAP, issue);
                    ui.spacer(6);
                }
                ui.scroll_end();
            } else {
                ui.text(TS_BODY | TX_WRAP,"Eigenes DOCX-Dokument mit Text, Formatierung, Quellen, Zitaten und Anmerkungen.");
            }
            ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP,"Die vollständige Bearbeitungs- und Darstellungskompatibilität mit allen DOCX-Dateien ist noch nicht erreicht.");
            ui.box_end();
            ui.dialog_buttons(None, None, Some("Schließen"));
        }
        ui.dialog_end();
        if ui.dialog_begin(2460, "Textbereich bearbeiten", &self.file_ui.text_open) {
            ui.next_size(440, 0);
            ui.box_begin(0, BOX_V, 8);
            if let Some(run) = self
                .document_file
                .as_ref()
                .and_then(|file| file.text_runs().get(self.file_ui.run))
            {
                ui.text(
                    TS_BODY_STRONG,
                    &format!("{} · Absatz {}", story_label(run.story), run.paragraph),
                );
            }
            ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP,"Nur dieser Text wird geändert. Die vorhandene Formatierung, Objekte und alle anderen Paketteile bleiben erhalten.");
            ui.next_size(-1, 170);
            ui.textbox(
                2461,
                None,
                &mut self.file_ui.replacement,
                (64 << 10) | TB_MULTI | TB_WRAP,
            );
            if !self.file_ui.error.is_empty() {
                ui.text(TS_BODY | TX_WRAP, &self.file_ui.error);
            }
            ui.box_end();
            if ui.dialog_buttons(Some("Text übernehmen"), None, Some("Abbrechen")) == DLG_PRIMARY {
                if let Some(file) = &mut self.document_file {
                    match file.replace_text(
                        self.file_ui.run,
                        &self.file_ui.before,
                        &self.file_ui.replacement,
                    ) {
                        Ok(()) => {
                            self.message = String::from(
                                "Textbereich geändert · vorhandene Dokumentteile bleiben erhalten",
                            )
                        }
                        Err(error) => {
                            self.file_ui.error = format!("Änderung nicht übernommen: {error:?}");
                            self.file_ui.text_open.set(1);
                        }
                    }
                }
            }
        }
        ui.dialog_end();
    }
}
