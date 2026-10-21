// jfWord stable numeric command identifiers and dispatch.
use crate::*;
pub(crate) const NEW: u32 = 1;
pub(crate) const OPEN: u32 = 2;
pub(crate) const SAVE: u32 = 3;
pub(crate) const SAVE_AS: u32 = 4;
pub(crate) const UNDO: u32 = 5;
pub(crate) const REDO: u32 = 6;
pub(crate) const ADD: u32 = 7;
pub(crate) const REMOVE: u32 = 8;
pub(crate) const SOURCE_NEW: u32 = 9;
pub(crate) const SOURCE_EDIT: u32 = 10;
pub(crate) const SOURCE_REMOVE: u32 = 11;
pub(crate) const CITE: u32 = 12;
pub(crate) const BIBLIOGRAPHY: u32 = 13;
pub(crate) const SOURCE_IMPORT: u32 = 14;
pub(crate) const SOURCE_EXPORT: u32 = 15;
pub(crate) const SOURCE_MERGE: u32 = 16;
pub(crate) const SOURCE_AUDIT: u32 = 17;
pub(crate) const SOURCE_LIBRARY: u32 = 18;
pub(crate) const LIBRARY_EDIT_DRAFT: u32 = 19;
pub(crate) const DO_OPEN: u32 = 20;
pub(crate) const DO_SAVE: u32 = 21;
pub(crate) const OVERWRITE: u32 = 22;
pub(crate) const FOOTNOTE: u32 = 23;
pub(crate) const ENDNOTE: u32 = 24;
pub(crate) const NOTE_RETURN: u32 = 25;
pub(crate) const NOTE_CONVERT: u32 = 26;
pub(crate) const NOTE_REMOVE: u32 = 27;
pub(crate) const HOME: u32 = 28;
pub(crate) const SOURCES: u32 = 29;
pub(crate) const NAVIGATION: u32 = 49;
pub(crate) const SEARCH: u32 = 50;
pub(crate) const PAGE_SETUP: u32 = 51;
pub(crate) const PARAGRAPH_SETUP: u32 = 52;
pub(crate) const COMPAT_INFO: u32 = 53;
pub(crate) const NEW_REPORT: u32 = 60;
pub(crate) const NEW_RESEARCH: u32 = 61;
pub(crate) const OPEN_RECENT: u32 = 62;
pub(crate) const VIEW_FOCUS: u32 = 63;
pub(crate) const VIEW_PAGE_WIDTH: u32 = 64;
pub(crate) const VIEW_ZOOM: u32 = 65;
pub(crate) const NEW_LETTER: u32 = 66;
pub(crate) const NEW_RESUME: u32 = 67;
pub(crate) const FORMAT_COPY: u32 = 80;
pub(crate) const FORMAT_PASTE: u32 = 81;
pub(crate) const FORMAT_RESET: u32 = 82;
pub(crate) const FORMAT_SUPERSCRIPT: u32 = 83;
pub(crate) const FORMAT_SUBSCRIPT: u32 = 84;
pub(crate) const TABLE_FORMULA:u32=130;
pub(crate) const FORMULAS_REFRESH:u32=131;
pub(crate) static CMDS: [Command; 23] = [
    Command::new(NEW, "Neu", IC_DOCUMENT, MOD_CTRL, VK_N, 0),
    Command::new(OPEN, "Öffnen", IC_OPEN_FILE, MOD_CTRL, VK_O, 0),
    Command::new(SAVE, "Speichern", IC_SAVE, MOD_CTRL, VK_S, 0),
    Command::new(
        SAVE_AS,
        "Speichern unter",
        IC_SAVE_AS,
        MOD_CTRL | MOD_SHIFT,
        VK_S,
        0,
    ),
    Command::new(UNDO, "Rückgängig", IC_UNDO, MOD_CTRL, VK_Z, 0),
    Command::new(REDO, "Wiederholen", IC_REDO, MOD_CTRL, VK_Y, 0),
    Command::new(ADD, "Absatz hinzufügen", IC_ADD, MOD_CTRL, VK_RETURN, 0),
    Command::new(REMOVE, "Absatz entfernen", IC_DELETE, 0, 0, 0),
    Command::new(VIEW_FOCUS, "Fokusmodus", IC_VIEW, MOD_CTRL | MOD_SHIFT, VK_F11, 0),
    Command::new(VIEW_PAGE_WIDTH, "Seitenbreite", IC_FIT_PAGE, 0, 0, 0),
    Command::new(VIEW_ZOOM, "Zoom in Prozent", IC_ZOOM_IN, 0, 0, 0),
    Command::new(FORMAT_COPY, "Format aufnehmen", IC_COPY, MOD_CTRL | MOD_SHIFT, VK_C, 0),
    Command::new(FORMAT_PASTE, "Format anwenden", IC_PASTE, MOD_CTRL | MOD_SHIFT | MOD_ALT, VK_V, 0),
    Command::new(FORMAT_RESET, "Formatierung löschen", IC_DELETE, MOD_CTRL, VK_SPACE, 0),
    Command::new(FORMAT_SUPERSCRIPT, "Hochgestellt", IC_OFFICE_SUPERSCRIPT, MOD_CTRL | MOD_SHIFT, VK_OEM_PLUS, 0),
    Command::new(FORMAT_SUBSCRIPT, "Tiefgestellt", IC_OFFICE_SUBSCRIPT, MOD_CTRL, VK_OEM_PLUS, 0),
    Command::new(FOOTNOTE, "Fußnote einfügen", IC_OFFICE_FOOTNOTE, 0, 0, 0),
    Command::new(ENDNOTE, "Endnote einfügen", IC_OFFICE_ENDNOTE, 0, 0, 0),
    Command::new(NOTE_RETURN, "Zur Textstelle", IC_OFFICE_NEXT_FOOTNOTE, 0, 0, 0),
    Command::new(NOTE_CONVERT, "Anmerkung umwandeln", IC_OFFICE_UPDATE_TABLE, 0, 0, 0),
    Command::new(NOTE_REMOVE, "Anmerkung entfernen", IC_DELETE, 0, 0, 0),
    Command::new(TABLE_FORMULA,"Tabellenformel",IC_OFFICE_FORMULA,0,0,0),
    Command::new(FORMULAS_REFRESH,"Formeln aktualisieren",IC_OFFICE_UPDATE_TABLE,0,VK_F9,0),
];
pub(crate) const DESC: AppDesc = AppDesc {
    title: "jfWord",
    flags: AF_RESIZABLE | AF_CUSTOM_TITLEBAR,
    w: 1240,
    h: 840,
    min_w: 1000,
    min_h: 700,
    cmds: &CMDS,
    ..AppDesc::NONE
};
impl Word {
    pub(crate) fn handle_command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        if self.guard_open.get() != 0 && !(id == SAVE && self.after_save) {
            if id == UIC_CLOSE {
                self.guard_open.set(0);
                self.pending = 0;
            }
            return true;
        }
        if self.overwrite_open.get() != 0 && id != OVERWRITE {
            if id == UIC_CLOSE {
                self.overwrite_open.set(0);
                self.after_save = false;
                self.pending = 0;
            }
            return true;
        }
        if self.file_open.get() != 0 && !matches!(id, DO_OPEN | DO_SAVE) {
            if id == UIC_CLOSE {
                self.file_open.set(0);
                self.after_save = false;
                self.pending = 0;
            }
            return true;
        }
        if self.file_ui.modal() {
            if id == UIC_CLOSE {
                self.file_ui.cancel();
            }
            return true;
        }
        if self.formatting.modal() || self.styles_ui.open.get() != 0 || self.rulers.open.get() != 0 || self.tables_ui.open.get()!=0 || self.formulas_ui.open.get()!=0 {
            if id == UIC_CLOSE {
                self.formatting.cancel();
                self.styles_ui.open.set(0);
                self.rulers.open.set(0);
                self.tables_ui.open.set(0);
                self.formulas_ui.open.set(0);
            }
            return true;
        }
        if self.notes_ui.modal() {
            if id == UIC_CLOSE {
                self.notes_ui.cancel();
            }
            return true;
        }
        if self.library.modal() {
            if id == UIC_CLOSE {
                self.library.cancel();
            }
            return true;
        }
        if self.source_tools.modal() {
            if id == UIC_CLOSE {
                self.source_tools.cancel();
                self.message = String::from("Quellendialog geschlossen · Dokument unverändert");
            }
            return true;
        }
        if self.source_open.get() != 0 {
            if id == UIC_CLOSE {
                self.source_open.set(0);
            }
            return true;
        }
        if self.citations.modal() {
            if id == UIC_CLOSE {
                self.citations.cancel();
                self.message = String::from("Zitatdialog geschlossen · Dokument unverändert");
            }
            return true;
        }
        // Content dialogs still receive global accelerators. Freeze document/file commands
        // while reviewing a snapshot so Cancel really preserves the original document.
        if self.source_import.open.get() != 0 {
            if id == UIC_CLOSE {
                self.source_import.open.set(0);
                self.message = String::from("Quellenimport abgebrochen · Dokument unverändert");
            }
            return true;
        }
        if self.preserved()
            && !matches!(
                id,
                NEW | NEW_REPORT
                    | NEW_RESEARCH
                    | NEW_LETTER
                    | NEW_RESUME
                    | OPEN
                    | OPEN_RECENT
                    | SAVE
                    | SAVE_AS
                    | UNDO
                    | REDO
                    | HOME
                    | COMPAT_INFO
                    | DO_OPEN
                    | DO_SAVE
                    | OVERWRITE
                    | UIC_CLOSE
            )
        {
            self.message = String::from(
                "Diese Bearbeitung ist für das erhaltene DOCX-Dokument noch nicht verfügbar.",
            );
            return true;
        }
        if matches!(
            id,
            SOURCES
                | NAVIGATION
                | SEARCH
                | PAGE_SETUP
                | PARAGRAPH_SETUP
                | SOURCE_NEW
                | SOURCE_LIBRARY
                | CITE
                | BIBLIOGRAPHY
                | FOOTNOTE
                | ENDNOTE
                | ADD
                | VIEW_FOCUS
                | VIEW_PAGE_WIDTH
                | VIEW_ZOOM
        ) {
            self.shell.home = false;
            self.shell.has_document = true;
        }
        match id {
            HOME => self.shell.home = true,
            SOURCES => self.show_sources = !self.show_sources,
            NAVIGATION => self.show_navigation = !self.show_navigation,
            VIEW_FOCUS => {
                self.focus_mode = !self.focus_mode;
                if self.focus_mode && !self.preview_mode {
                    let paragraph = if self.structured_edit { self.selected_id() } else {
                        self.editor.document().story_projection(self.selected_id()).ok()
                            .and_then(|story| story.paragraphs.first().map(|p| p.projection.paragraph))
                            .unwrap_or(self.selected_id())
                    };
                    let widget = self.text_id(paragraph, 0);
                    ui.focus(widget);
                } else if self.focus_mode { ui.focus(0); }
            }
            VIEW_PAGE_WIDTH => self.preview.set_fit_width(),
            VIEW_ZOOM => self.preview.set_zoom(arg.min(500) as i32),
            FORMAT_COPY | FORMAT_PASTE | FORMAT_RESET => self.rich_format_transfer(ui, id),
            FORMAT_SUPERSCRIPT | FORMAT_SUBSCRIPT => self.character_format(ui, id, None),
            SEARCH => {
                self.formatting.search_open = true;
                ui.focus(820);
            }
            PAGE_SETUP => self.open_page_format(),
            PARAGRAPH_SETUP => self.open_paragraph_format(),
            COMPAT_INFO => self.file_ui.info_open.set(1),
            NEW | NEW_REPORT | NEW_RESEARCH | NEW_LETTER | NEW_RESUME | OPEN | OPEN_RECENT | UIC_CLOSE => {
                if self.document_dirty() {
                    self.pending = id;
                    self.guard_open.set(1);
                } else {
                    self.perform(ui, id);
                }
            }
            SAVE => {
                if self.path.is_empty() {
                    self.choose_file(true);
                } else {
                    self.save(ui, self.path.clone());
                }
            }
            SAVE_AS => self.choose_file(true),
            SOURCE_IMPORT => self.choose_sources(false),
            SOURCE_EXPORT => self.choose_sources(true),
            SOURCE_MERGE => self.open_source_merge(),
            SOURCE_AUDIT => self.open_source_audit(),
            SOURCE_LIBRARY => self.open_library(),
            LIBRARY_EDIT_DRAFT => self.open_library_draft(),
            FOOTNOTE => self.insert_note(ui, doc_core::NoteKind::Footnote),
            ENDNOTE => self.insert_note(ui, doc_core::NoteKind::Endnote),
            NOTE_RETURN => self.return_to_note_anchor(ui),
            NOTE_CONVERT => self.convert_note(ui),
            TABLE_FORMULA => self.open_formula(self.rich_field_target().filter(|(p,i)|matches!(self.editor.document().paragraph(*p).and_then(|p|p.content.get(*i)),Some(Inline::Formula(_))))),
            FORMULAS_REFRESH => {ui.invalidate();self.message=String::from("Tabellenformeln aktualisiert");},
            NOTE_REMOVE => {
                if let Some(id) = self.active_note {
                    self.confirm_remove_note(id);
                }
            }
            DO_SAVE => {
                if (self.source_file || self.dialog_path != self.path)
                    && plat::file_exists(&self.dialog_path) == 1
                {
                    self.overwrite_open.set(1);
                } else {
                    self.write_selected_file(ui);
                }
            }
            OVERWRITE => self.write_selected_file(ui),
            DO_OPEN if self.source_file => self.import_sources(),
            DO_OPEN => match self.open_document(ui, &self.dialog_path.clone()) {
                Ok(()) => (),
                Err(doc_io::Error::Unsupported(part)) => {
                    self.message = format!(
                        "Noch nicht unterstützt: {part}. Das aktuelle Dokument bleibt erhalten."
                    )
                }
                Err(e) => {
                    self.message = format!(
                        "Öffnen fehlgeschlagen: {e:?}. Das aktuelle Dokument bleibt erhalten."
                    )
                }
            },
            UNDO => {
                if self.preserved() {
                    self.document_file.as_mut().unwrap().undo();
                } else {
                    self.history(ui, false);
                }
            }
            REDO => {
                if self.preserved() {
                    self.document_file.as_mut().unwrap().redo();
                } else {
                    self.history(ui, true);
                }
            }
            ADD => {
                if let Some(id) = self.apply(Edit::AddParagraph {
                    after: self.selected_id(),
                }) {
                    self.select_paragraph(id);
                    self.formatting.selection = None;
                    self.formatting.pending_selection = None;
                    self.jump_to_selected = true;
                    self.preview_mode = false;
                    let field = self.text_id(id, 0);
                    ui.focus(field);
                }
            }
            BIBLIOGRAPHY if self.active_note.is_some() => {
                self.message = String::from("Das Literaturverzeichnis gehört in den Haupttext.");
            }
            BIBLIOGRAPHY => {
                if let Some(id) = self.apply(Edit::InsertBibliography {
                    after: self.selected_id(),
                }) {
                    self.selected = self
                        .editor
                        .document()
                        .paragraphs
                        .iter()
                        .position(|p| p.id == id)
                        .unwrap_or(0);
                }
            }
            REMOVE => {
                self.apply(Edit::RemoveParagraph(self.selected_id()));
                self.widget_ids.clear();
            }
            SOURCE_NEW => {
                self.source_is_library = false;
                self.source_query.clear();
                self.draft = SourceForm::default();
                self.source_open.set(1);
            }
            SOURCE_EDIT => {
                self.source_is_library = false;
                if let Some(s) = self.active_source() {
                    self.draft = SourceForm::new(s);
                    self.source_open.set(1);
                }
            }
            SOURCE_REMOVE => {
                if let Some(s) = self.active_source() {
                    self.apply(Edit::RemoveSource(s.id));
                }
            }
            CITE => self.insert_citation(ui),
            _ => return false,
        }
        true
    }
}
