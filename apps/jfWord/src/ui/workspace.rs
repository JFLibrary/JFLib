// jfWord frame composition: ribbon, page, navigation and task panes.
use crate::*;
impl Word {
    pub(crate) fn frame_ui(&mut self, ui: &mut Ui) {
        // Closing dialogs restore their saved focus once on the following pass.
        let document_focus_pending = self.rich_focus_pending();
        // Document shortcuts take precedence over a text segment's local undo stack.
        // Draft/source/file dialogs retain their normal text-field keyboard behavior.
        if !self.shell.home
            && !self.preserved()
            && !self.file_ui.modal()
            && self.source_open.get() == 0
            && self.file_open.get() == 0
            && self.guard_open.get() == 0
            && self.overwrite_open.get() == 0
            && self.source_import.open.get() == 0
            && !self.citations.modal()
            && !self.source_tools.modal()
            && !self.library.modal()
            && !self.notes_ui.modal()
            && !self.formatting.modal()
            && self.styles_ui.open.get() == 0
            && self.rulers.open.get() == 0
            && self.tables_ui.open.get() == 0
            && self.formulas_ui.open.get() == 0
            && !matches!(
                ui.focused(),
                407 | 820 | 821 | 2104 | 2132 | 2501 | 2834 | 2880 | 3030..=3033
            )
        {
            if self.focus_mode && ui.capture_shortcut(VK_ESCAPE, 0) {
                self.focus_mode = false;
            }
            if !self.preview_mode {
                if self.structured_edit {
                    self.paragraph_keys(ui);
                } else {
                    self.rich_keys(ui);
                }
            }
            for (key, command) in [(VK_Z, UNDO), (VK_Y, REDO), (VK_RETURN, ADD)] {
                if ui.capture_shortcut(key, MOD_CTRL) {
                    ui.cmd_run(command, 0);
                }
            }
            if ui.capture_shortcut(VK_Z, MOD_CTRL | MOD_SHIFT) {
                ui.cmd_run(REDO, 0);
            }
            for (key, command) in [(VK_B, 30), (VK_I, 31), (VK_U, 32)] {
                if ui.capture_shortcut(key, MOD_CTRL) {
                    self.character_format(ui, command, None);
                }
            }
            if ui.capture_shortcut(VK_F, MOD_CTRL) || ui.capture_shortcut(VK_H, MOD_CTRL) {
                self.formatting.search_open = true;
                ui.focus(820);
            }
        }
        if self
            .active_note
            .is_some_and(|id| self.editor.document().note(id).is_none())
        {
            self.active_note = None;
                self.active_cell = None;
        }
        self.selected = self.selected.min(self.current_paragraphs().len() - 1);
        let notes_editable=!self.shell.home&&!self.preserved()&&!self.preview_mode;
        ui.cmd_state(TABLE_FORMULA,if notes_editable&&self.active_cell.is_some(){0}else{CF_DISABLED});
        ui.cmd_state(FORMULAS_REFRESH,if notes_editable{0}else{CF_DISABLED});
        for command in [FOOTNOTE,ENDNOTE] {ui.cmd_state(command,if notes_editable&&self.active_note.is_none(){0}else{CF_DISABLED});}
        for command in [NOTE_RETURN,NOTE_CONVERT,NOTE_REMOVE] {ui.cmd_state(command,if notes_editable&&self.active_note.is_some(){0}else{CF_DISABLED});}
        for command in [FORMAT_COPY, FORMAT_PASTE, FORMAT_RESET, FORMAT_SUPERSCRIPT, FORMAT_SUBSCRIPT] {
            ui.cmd_state(
                command,
                if self.rich_format_available(command) {
                    0
                } else {
                    CF_DISABLED
                },
            );
        }
        ui.cmd_state(
            UNDO,
            if if self.preserved() {
                self.document_file
                    .as_ref()
                    .is_some_and(doc_io::DocxFile::can_undo)
            } else {
                self.editor.can_undo()
            } {
                0
            } else {
                CF_DISABLED
            },
        );
        ui.cmd_state(
            REDO,
            if if self.preserved() {
                self.document_file
                    .as_ref()
                    .is_some_and(doc_io::DocxFile::can_redo)
            } else {
                self.editor.can_redo()
            } {
                0
            } else {
                CF_DISABLED
            },
        );
        ui.cmd_state(
            REMOVE,
            if self.current_paragraphs().len() > 1 {
                0
            } else {
                CF_DISABLED
            },
        );
        let filename = self
            .path
            .rsplit(['\\', '/'])
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("Unbenannt");
        let title = format!(
            "{filename}{} · jfWord",
            if self.document_dirty() { " *" } else { "" }
        );
        if ui.title() != title {
            ui.set_title(&title);
        }
        ui.next_size(-1, -1);
        ui.box_begin(100, BOX_V, 0);
        if !self.shell.home && (!self.focus_mode || self.preserved()) {
            self.shell_header(ui);
        }
        if self.shell.home {
            self.home_view(ui);
        } else if self.preserved() {
            self.preserved_view(ui);
        } else {
            if !self.focus_mode {
                // Keep stable internal tab identifiers while exposing the complete Office order.
                const TAB_IDS: [i32; 11] = [0, 1, 5, 6, 7, 3, 2, 8, 9, 4, 10];
                let mut visible_tab =
                    TAB_IDS.iter().position(|id| *id == self.tab).unwrap_or(1) as i32;
                ui.next_size(-1, 30);
                ui.box_begin(2001, BOX_H | BOX_XCENTER, 6);
                let compact_actions = ui.px2dip(ui.box_rect()[2]) < 1150;
                // The row's six-DIP gap plus this inset matches the reference tab origin.
                ui.spacer(2);
                ui.next_size(-1, 30);
                ui.tabs_scrolling(
                    2000,
                    &mut visible_tab,
                    &[
                        "Datei",
                        "Start",
                        "Einfügen",
                        "Zeichnen",
                        "Entwurf",
                        "Layout",
                        "Referenzen",
                        "Sendungen",
                        "Überprüfen",
                        "Ansicht",
                        "Hilfe",
                    ],
                );
                for (id, label, icon, width, accent) in [
                    (0x550120, "Kommentare", IC_OFFICE_COMMENT, 102, false),
                    (0x550121, "Bearbeiten", IC_EDIT, 102, false),
                    (0x550122, "Teilen", IC_SHARE, 74, true),
                ] {
                    ui.next_size(if compact_actions { 28 } else { width }, 23);
                    ui.next_icon(icon);
                    ui.next_flags(
                        ST_DISABLED
                            | ST_COMPACT
                            | if compact_actions { ST_ICON_ONLY } else { 0 }
                            | if accent { ST_ACCENT } else { 0 },
                    );
                    ui.button(id, label);
                    ui.tooltip(if id == 0x550121 { "Dokument wird bearbeitet. Weitere Bearbeitungsmodi sind noch nicht verfügbar." }
                        else { "Diese Funktion ist noch nicht verfügbar." });
                }
                ui.spacer(8);
                ui.box_end();
                self.tab = TAB_IDS[visible_tab.clamp(0, 10) as usize];
                if self.tab == 0 {
                    self.shell.home = true;
                    self.tab = 1;
                }
                self.ribbon(ui);
                self.search_panel(ui);
                self.field_toolbar(ui);
            }
            ui.next_size(-1, -1);
            ui.box_begin(103, BOX_H, 0);
            if self.show_navigation && !self.focus_mode {
                self.navigation_view(ui);
            }
            if self.preview_mode {
                self.page(ui);
            } else {
                if self.structured_edit {
                    self.paper(ui);
                } else {
                    self.rich_paper(ui);
                }
            }
            if self.show_sources && !self.focus_mode {
                self.sources_pane(ui);
            }
            ui.box_end();
        }
        if !self.shell.home {
            ui.next_size(-1, 26);
            ui.box_begin(2090, BOX_H | BOX_XCENTER | BOX_BG, 0);
            let mut status = Sp::default();
            status.place(ui.box_rect());
            ui.style.statusbar(ui, &status);
            ui.spacer(12);
            ui.next_size(-1, 26);
            ui.box_begin(2091, BOX_H | BOX_XCENTER, 14);
            if self.shell.has_document && !self.preserved() {
                if self.focus_mode && ui.agent_enabled() {
                    ui.text(
                        TS_CAPTION | TX_ACCENT,
                        if ui.agent_connected() {
                            "Agent verbunden"
                        } else {
                            "Agent aktiv"
                        },
                    );
                }
                if let Some(layout) = self.preview.layout() {
                    ui.text(
                        TS_CAPTION | TX_SECONDARY,
                        &format!(
                            "Seite {} von {}",
                            self.navigation_current_page() + 1,
                            layout.pages.len()
                        ),
                    );
                }
                ui.text(
                    TS_CAPTION | TX_SECONDARY,
                    &format!(
                        "{} Wörter",
                        self.editor
                            .document()
                            .plain_text()
                            .split_whitespace()
                            .count()
                    ),
                );
                if !self.editor.document().sources.is_empty() {
                    ui.text(
                        TS_CAPTION | TX_SECONDARY,
                        &format!("{} Quellen", self.editor.document().sources.len()),
                    );
                }
                ui.text(TS_CAPTION | TX_SECONDARY, "Deutsch");
            }
            ui.next_size(-1, 0);
            ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &self.message);
            if !self.shell.home && !self.preserved() {
                self.paper_status(ui);
            }
            ui.box_end();
            ui.spacer(12);
            ui.box_end();
        }
        ui.box_end();
        self.dialogs(ui);
        if document_focus_pending {
            self.rich_finish_focus(ui);
        }
    }
}
