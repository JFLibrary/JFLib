// Document-independent source-library workflow. Transactions and persistence live in the libraries.
use crate::{LIBRARY_EDIT_DRAFT, Word, source_form::SourceForm};
use alloc::{format, string::String, vec, vec::Vec};
use doc_core::{Edit, Id, LibraryEdit, Source, SourceField, SourceLibraryEditor};
use doc_io::{LibraryRecovery, LibraryStore, LibraryStoreError};
use ui::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Browse,
    Compare,
    Remove,
    Recovery,
}

struct Comparison {
    document: Source,
    library: Source,
    document_revision: u64,
    library_revision: u64,
    to_library: i32,
    incoming: Vec<SourceField>,
    field: i32,
    preview: bool,
}
impl Comparison {
    fn target(&self) -> &Source {
        if self.to_library != 0 {
            &self.library
        } else {
            &self.document
        }
    }
    fn incoming(&self) -> &Source {
        if self.to_library != 0 {
            &self.document
        } else {
            &self.library
        }
    }
    fn defaults(&mut self) {
        self.incoming = SourceField::ALL
            .into_iter()
            .filter(|field| {
                self.target().field_empty(*field) && !self.incoming().field_empty(*field)
            })
            .collect();
    }
    fn merged(&self) -> Result<Source, doc_core::Error> {
        self.target().merge_fields(self.incoming(), &self.incoming)
    }
}

pub(crate) struct LibraryUi {
    path: Option<String>,
    store: Option<LibraryStore>,
    editor: Option<SourceLibraryEditor>,
    open: Shared<i32>,
    page: Page,
    query: String,
    selected: Id,
    message: String,
    comparison: Option<Comparison>,
    remove: Option<Source>,
    recovery: Option<LibraryRecovery>,
    recovery_source: i32,
    pending_draft: Option<Source>,
}
impl Default for LibraryUi {
    fn default() -> Self {
        Self {
            path: None,
            store: None,
            editor: None,
            open: Shared::new(0),
            page: Page::Browse,
            query: String::new(),
            selected: 0,
            message: String::new(),
            comparison: None,
            remove: None,
            recovery: None,
            recovery_source: 0,
            pending_draft: None,
        }
    }
}
fn storage_message(error: &LibraryStoreError) -> String {
    String::from(match error {
        LibraryStoreError::Busy => {
            "Die Bibliothek ist gesperrt oder die Sperrdatei ist nicht zugänglich. Es wurde nichts übernommen. Bitte erneut versuchen."
        }
        LibraryStoreError::Conflict => {
            "Die Bibliotheksdatei wurde außerhalb dieses Fensters geändert. Es wurde nichts überschrieben. Bitte neu laden und die Änderung erneut prüfen."
        }
        LibraryStoreError::Io => {
            "Die Bibliothek oder ihre Sicherung konnte nicht gelesen oder gespeichert werden. Bitte Pfad und Zugriffsrechte prüfen."
        }
        LibraryStoreError::Codec(_) => {
            "Die Bibliothek oder die Änderung enthält ungültige oder nicht unterstützte Daten. Bitte eine geprüfte Sicherung verwenden."
        }
    })
}
fn source_label(source: &Source) -> String {
    format!(
        "{} · {} · {} · ID {}",
        source.title,
        source.citation_author(),
        source.year,
        source.id
    )
}
fn button(ui: &mut Ui, id: u32, title: &str, enabled: bool) -> bool {
    if !enabled {
        ui.next_flags(ST_DISABLED);
    }
    ui.button(id, title) & EV_ACT != 0 && enabled
}
fn details(ui: &mut Ui, source: &Source) {
    for field in SourceField::ALL {
        ui.text(
            TS_CAPTION | TX_SECONDARY,
            crate::source_tools::field_label(field),
        );
        ui.text(
            TS_BODY | TX_WRAP,
            &crate::source_tools::field_text(source, field),
        );
    }
}
fn body_width(ui: &Ui) -> i32 {
    let padding = (ui.style.metric(SM_DLG_CONTENT_BOX) as u32 >> PAD_SHIFT) & 255;
    ui.style
        .metric(SM_DLG_MAX)
        .min(ui.client_size().0 - ui.style.metric(SM_DLG_MARGIN))
        .saturating_sub(2 * padding as i32)
        .max(1)
}
impl LibraryUi {
    pub(crate) fn configure(&mut self, path: Option<String>) {
        *self = Self {
            path,
            ..Self::default()
        };
    }
    pub(crate) fn modal(&self) -> bool {
        self.open.get() != 0
    }
    pub(crate) fn cancel(&mut self) {
        self.open.set(0);
        self.pending_draft = None;
        self.comparison = None;
        self.recovery = None;
        self.remove = None;
        self.page = Page::Browse;
    }
    fn selected_source(&self) -> Option<Source> {
        self.editor
            .as_ref()?
            .library()
            .source(self.selected)
            .cloned()
    }
    fn load(&mut self) {
        let Some(path) = &self.path else {
            self.message = String::from(
                "Keine Bibliothek konfiguriert. Für automatisierte Läufe mit --library-dir einen eigenen Ordner angeben.",
            );
            return;
        };
        match LibraryStore::open(path) {
            Ok(store) => {
                self.editor = Some(SourceLibraryEditor::new(store.library().clone()).unwrap());
                self.store = Some(store);
                self.message = String::from(
                    "Bibliothek geladen. Änderungen werden sofort gespeichert; Rückgängig gilt für diese Sitzung.",
                );
            }
            Err(error) => self.message = storage_message(&error),
        }
    }
    fn commit(&mut self, next: SourceLibraryEditor) -> bool {
        let Some(store) = &mut self.store else {
            return false;
        };
        match store.save(next.library()) {
            Ok(()) => {
                self.editor = Some(next);
                true
            }
            Err(error) => {
                self.message = storage_message(&error);
                false
            }
        }
    }
    fn apply(&mut self, edit: LibraryEdit) -> Option<Id> {
        let mut next = self.editor.as_ref()?.clone();
        match next.apply(edit) {
            Ok(id) => {
                if self.commit(next) {
                    Some(id)
                } else {
                    None
                }
            }
            Err(error) => {
                self.message = format!("Bibliotheksänderung nicht übernommen: {error:?}");
                None
            }
        }
    }
    fn history(&mut self, redo: bool) {
        let Some(mut next) = self.editor.clone() else {
            return;
        };
        let changed = if redo { next.redo() } else { next.undo() };
        if changed && self.commit(next) {
            self.message = String::from(if redo {
                "Bibliotheksänderung wiederholt und gespeichert."
            } else {
                "Bibliotheksänderung rückgängig gemacht und gespeichert."
            });
        }
    }
    fn prepare_recovery(&mut self) {
        let Some(path) = &self.path else {
            return;
        };
        match LibraryStore::prepare_recovery(path) {
            Ok(preview) => {
                self.recovery = Some(preview);
                self.recovery_source = 0;
                self.page = Page::Recovery;
                self.message.clear();
            }
            Err(error) => self.message = storage_message(&error),
        }
    }
}

impl Word {
    pub(crate) fn open_library(&mut self) {
        if self.library.editor.is_none() {
            self.library.load();
        }
        self.library.page = Page::Browse;
        self.library.comparison = None;
        self.library.recovery = None;
        self.library.remove = None;
        self.library.open.set(1);
    }
    pub(crate) fn open_library_draft(&mut self) {
        if let Some(source) = self.library.pending_draft.take() {
            self.draft = SourceForm::new(source);
            self.source_is_library = true;
            self.source_open.set(1);
        }
    }
    pub(crate) fn finish_library_source(&mut self, source: Source) -> bool {
        if let Some(id) = self.library.apply(LibraryEdit::Put(source)) {
            self.library.selected = id;
            self.library.query.clear();
            self.library.message =
                String::from("Bibliotheksquelle gespeichert. Dokumentquellen bleiben unverändert.");
            self.message = self.library.message.clone();
            self.source_is_library = false;
            true
        } else {
            self.draft.set_error(&self.library.message);
            self.source_open.set(1);
            false
        }
    }
    pub(crate) fn library_dialogs(&mut self, ui: &mut Ui) {
        let was_open = self.library.open.get() != 0;
        if ui.dialog_begin(1100, "Quellenbibliothek", &self.library.open) {
            match self.library.page {
                Page::Browse => self.library_browse(ui),
                Page::Compare => self.library_compare(ui),
                Page::Remove => self.library_remove(ui),
                Page::Recovery => self.library_recovery(ui),
            }
        }
        ui.dialog_end();
        // Closing the parent dialog first restores its previous focus; queue the new modal later.
        if !was_open && self.library.pending_draft.is_some() {
            ui.cmd_run(LIBRARY_EDIT_DRAFT, 0);
        }
    }
    fn library_browse(&mut self, ui: &mut Ui) {
        let height = ui.client_size().1;
        let compact = height < 620;
        // Give the extra body container a known natural width on its first pass.
        // Otherwise its wrapped text reaches the dialog through three generations
        // of previous-pass measurements before the inner preview can settle.
        let width = body_width(ui);
        if compact {
            ui.next_size(width, (height - 150).max(180));
            ui.scroll_begin(1114, SC_V | BOX_V);
        } else {
            ui.next_size(width, 0);
            ui.box_begin(1114, BOX_V, 4);
        }
        let state = &mut self.library;
        ui.next_size(-1, 32);
        ui.text(TS_CAPTION | TX_WRAP, "Persönliche Quellen für alle Dokumente. Übernahme erstellt eine unabhängige Kopie. Ein Abgleich aktualisiert nur die ausdrücklich gewählte Quelle.");
        ui.next_size(-1, 0);
        ui.textbox(
            1101,
            Some("Bibliothek durchsuchen …"),
            &mut state.query,
            257,
        );
        let sources: Vec<&Source> = state
            .editor
            .as_ref()
            .map(|editor| {
                editor
                    .library()
                    .sources
                    .iter()
                    .filter(|source| source.matches_query(&state.query))
                    .collect()
            })
            .unwrap_or_default();
        let names: Vec<String> = sources.iter().map(|source| source_label(source)).collect();
        let mut selected = sources
            .iter()
            .position(|source| source.id == state.selected)
            .unwrap_or(0) as i32;
        ui.next_size(-1, 0);
        if sources.is_empty() {
            ui.next_flags(ST_DISABLED);
            ui.combobox(1102, &mut selected, &["Keine Quellen in dieser Auswahl"]);
            state.selected = 0;
        } else {
            ui.combobox(
                1102,
                &mut selected,
                &names.iter().map(String::as_str).collect::<Vec<_>>(),
            );
            state.selected = sources[selected as usize].id;
        }
        let total = state
            .editor
            .as_ref()
            .map(|editor| editor.library().sources.len())
            .unwrap_or(0);
        ui.text(
            TS_CAPTION,
            &format!("{} von {total} Bibliotheksquellen", sources.len()),
        );
        ui.next_size(-1, (height - 590).clamp(36, 90));
        ui.scroll_begin(1103, SC_V | BOX_V | BOX_CARD | pad(6));
        ui.text(
            TS_CAPTION | TX_SECONDARY | TX_WRAP,
            state.path.as_deref().unwrap_or("Bibliothek deaktiviert"),
        );
        if let Some(source) = sources.get(selected as usize) {
            details(ui, source);
        } else {
            ui.text(
                TS_BODY | TX_WRAP,
                "Eine Dokumentquelle übernehmen oder eine neue Bibliotheksquelle anlegen.",
            );
        }
        ui.scroll_end();
        let ready = state.editor.is_some();
        let selected = state.selected_source();
        let document = self.active_source();
        ui.box_begin(0, BOX_H, 6);
        if button(ui, 1104, "Neue Quelle", ready) {
            self.library.pending_draft = Some(Source::default());
            self.library.open.set(0);
        }
        if button(ui, 1105, "Bearbeiten", selected.is_some()) {
            self.library.pending_draft = selected.clone();
            self.library.open.set(0);
        }
        if button(ui, 1106, "Entfernen …", selected.is_some()) {
            self.library.remove = selected.clone();
            self.library.page = Page::Remove;
            self.library.message.clear();
        }
        ui.box_end();
        ui.box_begin(0, BOX_H, 6);
        if button(
            ui,
            1107,
            "Dokument → Bibliothek",
            ready && document.is_some(),
        ) {
            if let Some(id) = self
                .library
                .apply(LibraryEdit::Import(vec![document.clone().unwrap()]))
            {
                self.library.selected = id;
                self.library.query.clear();
                self.library.message = String::from(
                    "Dokumentquelle in der Bibliothek verfügbar. Identische Angaben werden nur einmal importiert.",
                );
            }
        }
        if button(ui, 1108, "Bibliothek → Dokument", selected.is_some()) {
            let incoming = selected.clone().unwrap();
            if self
                .apply(Edit::ImportSources(vec![incoming.clone()]))
                .is_some()
            {
                // Import returns zero when every source was already present. Identities in
                // the two containers are unrelated; select the actual metadata match.
                if let Some(index) = self
                    .editor
                    .document()
                    .sources
                    .iter()
                    .position(|source| source.same_content(&incoming))
                {
                    self.source_sel = index as i32;
                }
                self.source_query.clear();
                self.library.message = String::from(
                    "Bibliotheksquelle im Dokument verfügbar. Identische Angaben werden nur einmal importiert.",
                );
            } else {
                self.library.message = self.message.clone();
            }
        }
        ui.box_end();
        ui.box_begin(0, BOX_H, 6);
        if button(
            ui,
            1109,
            "Abgleichen …",
            selected.is_some() && document.is_some(),
        ) {
            let mut comparison = Comparison {
                document: document.clone().unwrap(),
                library: selected.unwrap(),
                document_revision: self.editor.revision(),
                library_revision: self.library.editor.as_ref().unwrap().revision(),
                to_library: 0,
                incoming: Vec::new(),
                field: 0,
                preview: false,
            };
            comparison.defaults();
            self.library.comparison = Some(comparison);
            self.library.page = Page::Compare;
            self.library.message.clear();
        }
        if button(ui, 1113, "Sicherung prüfen …", self.library.path.is_some()) {
            self.library.prepare_recovery();
        }
        ui.box_end();
        ui.next_size(-1, 28);
        ui.text(
            TS_CAPTION | TX_WRAP,
            &document
                .map(|source| format!("Ausgewählte Dokumentquelle: {}", source.title))
                .unwrap_or_else(|| String::from("Im Dokument ist keine Quelle ausgewählt.")),
        );
        ui.box_begin(0, BOX_H, 6);
        let undo = self
            .library
            .editor
            .as_ref()
            .is_some_and(SourceLibraryEditor::can_undo);
        let redo = self
            .library
            .editor
            .as_ref()
            .is_some_and(SourceLibraryEditor::can_redo);
        if button(ui, 1110, "Rückgängig", undo) {
            self.library.history(false);
        }
        if button(ui, 1111, "Wiederholen", redo) {
            self.library.history(true);
        }
        if button(ui, 1112, "Neu laden", self.library.path.is_some()) {
            self.library.load();
        }
        ui.box_end();
        ui.next_size(-1, 46);
        ui.text(TS_CAPTION | TX_WRAP, &self.library.message);
        if compact {
            ui.scroll_end();
        } else {
            ui.box_end();
        }
        ui.dialog_buttons(None, None, Some("Schließen"));
    }
    fn library_compare(&mut self, ui: &mut Ui) {
        let state = &mut self.library;
        let Some(comparison) = &mut state.comparison else {
            state.page = Page::Browse;
            return;
        };
        ui.text(TS_BODY_STRONG, "Quellen gezielt abgleichen");
        ui.next_size(-1, 44);
        ui.text(
            TS_CAPTION | TX_WRAP,
            &format!(
                "Dokument: {}\nBibliothek: {}",
                comparison.document.title, comparison.library.title
            ),
        );
        let prior = comparison.to_library;
        ui.next_size(-1, 0);
        ui.combobox(
            1120,
            &mut comparison.to_library,
            &[
                "Bibliothek → Dokumentquelle",
                "Dokument → Bibliotheksquelle",
            ],
        );
        if prior != comparison.to_library {
            comparison.defaults();
        }
        ui.box_begin(0, BOX_H, 6);
        if ui.button(1121, "Feldvergleich") & EV_ACT != 0 {
            comparison.preview = false;
        }
        if ui.button(1122, "Ergebnis") & EV_ACT != 0 {
            comparison.preview = true;
        }
        ui.box_end();
        ui.next_size(-1, 290);
        ui.scroll_begin(if comparison.preview { 1124 } else { 1123 }, SC_V | BOX_V);
        if comparison.preview {
            match comparison.merged() {
                Ok(source) => details(ui, &source),
                Err(_) => ui.text(
                    TS_BODY | TX_WRAP,
                    "Diese Auswahl ergibt keine gültige Quelle.",
                ),
            }
        } else {
            ui.text(
                TS_CAPTION,
                &format!(
                    "{} gewählte Felder aus der anderen Quelle",
                    comparison.incoming.len()
                ),
            );
            ui.next_size(-1, 0);
            let fields: Vec<&str> = SourceField::ALL
                .iter()
                .map(|field| crate::source_tools::field_label(*field))
                .collect();
            ui.combobox(1125, &mut comparison.field, &fields);
            let field = SourceField::ALL[comparison.field as usize];
            ui.box_begin(0, BOX_H, 8);
            for (id, label, source) in [
                (1126, "Bisheriger Wert im Ziel", comparison.target()),
                (1127, "Wert aus der anderen Quelle", comparison.incoming()),
            ] {
                ui.next_size(-1, 136);
                ui.scroll_begin(id, SC_V | BOX_V | BOX_CARD | pad(6));
                ui.text(TS_CAPTION | TX_SECONDARY, label);
                ui.text(
                    TS_BODY | TX_WRAP,
                    &crate::source_tools::field_text(source, field),
                );
                ui.scroll_end();
            }
            ui.box_end();
            let mut incoming = i32::from(comparison.incoming.contains(&field));
            if ui.checkbox(
                1128,
                "Wert aus der anderen Quelle übernehmen",
                &mut incoming,
            ) & EV_ACT
                != 0
            {
                comparison.incoming.retain(|existing| *existing != field);
                if incoming != 0 {
                    comparison.incoming.push(field);
                }
            }
            ui.text(TS_CAPTION | TX_WRAP, "Vorauswahl ergänzt nur leere Zielfelder. Belegte Angaben ändern sich ausschließlich durch Ihre Auswahl.");
        }
        ui.scroll_end();
        ui.next_size(-1, 60);
        ui.text(TS_CAPTION | TX_WRAP, if state.message.is_empty() { "Die Zielquelle behält ihre ID und alle Zitatverweise. Andere Dokumente bleiben unverändert. Rückgängig erfolgt im jeweiligen Ziel." } else { &state.message });
        let action = ui.dialog_buttons(
            Some(if comparison.to_library != 0 {
                "Bibliotheksquelle aktualisieren"
            } else {
                "Dokumentquelle aktualisieren"
            }),
            None,
            Some("Zurück"),
        );
        if action == DLG_CLOSE {
            state.open.set(1);
            state.page = Page::Browse;
            state.comparison = None;
        } else if action == DLG_PRIMARY {
            state.open.set(1);
            if comparison.document_revision != self.editor.revision()
                || state
                    .editor
                    .as_ref()
                    .is_none_or(|editor| comparison.library_revision != editor.revision())
            {
                state.message = String::from(
                    "Eine Quelle wurde inzwischen geändert. Bitte den Vergleich neu öffnen.",
                );
                return;
            }
            let to_library = comparison.to_library != 0;
            match comparison.merged() {
                Ok(source) => {
                    let success = if to_library {
                        self.library.apply(LibraryEdit::Put(source)).is_some()
                    } else {
                        let success = self.apply(Edit::PutSource(source)).is_some();
                        if !success {
                            self.library.message = self.message.clone();
                        }
                        success
                    };
                    if success {
                        self.library.message = String::from(if to_library {
                            "Bibliotheksquelle abgeglichen und gespeichert. Dokument unverändert."
                        } else {
                            "Dokumentquelle abgeglichen. Ihre Zitatverweise bleiben erhalten."
                        });
                        self.library.page = Page::Browse;
                        self.library.comparison = None;
                    }
                }
                Err(_) => {
                    self.library.message = String::from(
                        "Die Feldauswahl ergibt keine gültige Quelle; es wurde nichts geändert.",
                    )
                }
            }
        }
    }
    fn library_remove(&mut self, ui: &mut Ui) {
        let state = &mut self.library;
        let Some(source) = &state.remove else {
            state.page = Page::Browse;
            return;
        };
        let id = source.id;
        ui.text(TS_BODY_STRONG, "Bibliotheksquelle entfernen");
        ui.next_size(-1, 310);
        ui.scroll_begin(1140, SC_V | BOX_V);
        details(ui, source);
        ui.scroll_end();
        ui.next_size(-1, 70);
        ui.text(TS_CAPTION | TX_WRAP, if state.message.is_empty() { "Diese Quelle wird aus der persönlichen Bibliothek entfernt. Kopien und Zitate in Dokumenten bleiben erhalten. Sie können die Entfernung in dieser Sitzung rückgängig machen." } else { &state.message });
        match ui.dialog_buttons(Some("Aus Bibliothek entfernen"), None, Some("Zurück")) {
            DLG_PRIMARY => {
                state.open.set(1);
                if state.apply(LibraryEdit::Remove(id)).is_some() {
                    state.message = String::from(
                        "Bibliotheksquelle entfernt und gespeichert. Dokumente unverändert.",
                    );
                    state.page = Page::Browse;
                    state.remove = None;
                }
            }
            DLG_CLOSE => {
                state.open.set(1);
                state.page = Page::Browse;
                state.remove = None;
            }
            _ => {}
        }
    }
    fn library_recovery(&mut self, ui: &mut Ui) {
        let height = ui.client_size().1;
        let state = &mut self.library;
        let Some(recovery) = &state.recovery else {
            state.page = Page::Browse;
            return;
        };
        ui.next_size(body_width(ui), 0);
        ui.box_begin(1162, BOX_V, 4);
        let sources = &recovery.library().sources;
        ui.text(TS_BODY_STRONG, "Sicherung der Bibliothek prüfen");
        ui.text(
            TS_CAPTION,
            &format!(
                "{} {} in der letzten Sicherung",
                sources.len(),
                if sources.len() == 1 {
                    "Quelle"
                } else {
                    "Quellen"
                }
            ),
        );
        ui.next_size(-1, 0);
        if sources.is_empty() {
            ui.next_flags(ST_DISABLED);
            ui.combobox(1160, &mut state.recovery_source, &["Leere Bibliothek"]);
        } else {
            let names: Vec<String> = sources.iter().map(source_label).collect();
            ui.combobox(
                1160,
                &mut state.recovery_source,
                &names.iter().map(String::as_str).collect::<Vec<_>>(),
            );
        }
        ui.next_size(-1, (height - 390).clamp(40, 270));
        ui.scroll_begin(1161, SC_V | BOX_V);
        if let Some(source) = sources.get(state.recovery_source as usize) {
            details(ui, source);
        }
        ui.scroll_end();
        ui.next_size(-1, 100);
        ui.text(TS_CAPTION | TX_WRAP, if state.message.is_empty() { "Wiederherstellen ersetzt die gesamte aktuelle Bibliothek durch diese Sicherung. Beim Ersetzen wird eine zuvor vorhandene Datei unter .damaged gesichert. Die Bibliothekshistorie dieser Sitzung wird zurückgesetzt; Dokumente bleiben unverändert." } else { &state.message });
        ui.box_end();
        match ui.dialog_buttons(Some("Sicherung wiederherstellen"), None, Some("Zurück")) {
            DLG_PRIMARY => {
                state.open.set(1);
                match recovery.restore() {
                    Ok(store) => {
                        state.editor =
                            Some(SourceLibraryEditor::new(store.library().clone()).unwrap());
                        state.store = Some(store);
                        state.recovery = None;
                        state.page = Page::Browse;
                        state.query.clear();
                        state.message = String::from(
                            "Geprüfte Sicherung wiederhergestellt. Dokumente bleiben unverändert.",
                        );
                    }
                    Err(error) => state.message = storage_message(&error),
                }
            }
            DLG_CLOSE => {
                state.open.set(1);
                state.page = Page::Browse;
                state.recovery = None;
            }
            _ => {}
        }
    }
}
