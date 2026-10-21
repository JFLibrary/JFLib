// Source comparison, deliberate merges and non-mutating completeness reports.
// New jfWord functionality; all source decisions and mutations live in doc_core.
use crate::{SOURCE_EDIT, Word};
use alloc::{collections::BTreeSet, format, string::String, vec::Vec};
use doc_core::{Edit, Id, Source, SourceField, SourceIssue, SourceIssueKind, SourceMerge};
use ui::*;

pub(crate) struct SourceTools {
    merge_open: Shared<i32>,
    keep: Id,
    remove: Id,
    similar: Vec<Id>,
    only_similar: i32,
    plan: Option<SourceMerge>,
    preview: Option<(SourceMerge, Result<Source, doc_core::Error>)>,
    field: i32,
    page: i32,
    revision: u64,
    error: String,
    audit_open: Shared<i32>,
    issues: Vec<SourceIssue>,
    issue_sources: BTreeSet<Id>,
    cited_sources: BTreeSet<Id>,
    audit_source: Id,
    only_cited: i32,
    pending_edit: Option<Id>,
}
impl Default for SourceTools {
    fn default() -> Self {
        Self {
            merge_open: Shared::new(0),
            keep: 0,
            remove: 0,
            similar: Vec::new(),
            only_similar: 0,
            plan: None,
            preview: None,
            field: 0,
            page: 0,
            revision: 0,
            error: String::new(),
            audit_open: Shared::new(0),
            issues: Vec::new(),
            issue_sources: BTreeSet::new(),
            cited_sources: BTreeSet::new(),
            audit_source: 0,
            only_cited: 0,
            pending_edit: None,
        }
    }
}
impl SourceTools {
    pub(crate) fn modal(&self) -> bool {
        self.merge_open.get() != 0 || self.audit_open.get() != 0
    }
    pub(crate) fn cancel(&mut self) {
        self.merge_open.set(0);
        self.audit_open.set(0);
        self.pending_edit = None;
    }
}

pub(crate) fn field_label(field: SourceField) -> &'static str {
    match field {
        SourceField::Kind => "Quellentyp",
        SourceField::Creators => "Autoren / Körperschaft",
        SourceField::Title => "Titel",
        SourceField::Year => "Jahr",
        SourceField::Publisher => "Verlag",
        SourceField::City => "Verlagsort",
        SourceField::Edition => "Auflage",
        SourceField::Journal => "Zeitschrift",
        SourceField::Volume => "Band",
        SourceField::Issue => "Heft",
        SourceField::Pages => "Seiten",
        SourceField::Website => "Website",
        SourceField::Url => "URL / DOI-Link",
        SourceField::StandardNumber => "ISBN / ISSN",
        SourceField::YearAccessed => "Zugriff: Jahr",
        SourceField::MonthAccessed => "Zugriff: Monat",
        SourceField::DayAccessed => "Zugriff: Tag",
        SourceField::ShortTitle => "Kurztitel",
        SourceField::Comments => "Notizen",
        SourceField::ExternalId => "Externe Kennung",
        SourceField::Language => "Sprachkennung",
    }
}
pub(crate) fn field_text(source: &Source, field: SourceField) -> String {
    if field == SourceField::Creators {
        if !source.corporate.is_empty() {
            return format!("Körperschaft:\n{}", source.corporate);
        }
        if !source.authors.is_empty() {
            return source
                .authors
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    format!(
                        "Person {}\nVorname: {}\nWeitere Vornamen: {}\nNachname: {}",
                        i + 1,
                        p.given,
                        p.middle,
                        p.family
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n");
        }
    }
    if field == SourceField::Kind {
        return String::from(match source.kind {
            doc_core::SourceKind::Book => "Buch",
            doc_core::SourceKind::JournalArticle => "Zeitschriftenartikel",
            doc_core::SourceKind::Website => "Webseite",
        });
    }
    let text = source.field_text(field);
    if text.is_empty() {
        String::from("(leer)")
    } else {
        text
    }
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
fn issue_label(kind: SourceIssueKind) -> &'static str {
    match kind {
        SourceIssueKind::MissingCreators => "Autoren oder Körperschaft fehlen.",
        SourceIssueKind::MissingYear => "Erscheinungsjahr fehlt.",
        SourceIssueKind::MissingPublisher => "Verlag fehlt beim Buch.",
        SourceIssueKind::MissingJournal => "Zeitschrift fehlt beim Artikel.",
        SourceIssueKind::MissingUrl => "URL fehlt bei der Webseite.",
        SourceIssueKind::IncompleteAccessDate => {
            "Zugriffsdatum: Tag oder Monat ohne übergeordnete Datumsangabe."
        }
    }
}

impl Word {
    pub(crate) fn open_source_merge(&mut self) {
        let Some(source) = self.active_source() else {
            return;
        };
        if self.editor.document().sources.len() < 2 {
            return;
        }
        let similar = self
            .editor
            .document()
            .source_candidates(source.id)
            .unwrap_or_default();
        self.source_tools.keep = source.id;
        self.source_tools.similar = similar.iter().map(|s| s.source).collect();
        self.source_tools.only_similar = i32::from(!similar.is_empty());
        self.source_tools.remove = similar.first().map(|s| s.source).unwrap_or_else(|| {
            self.editor
                .document()
                .sources
                .iter()
                .find(|s| s.id != source.id)
                .unwrap()
                .id
        });
        self.source_tools.plan = self
            .editor
            .document()
            .plan_source_merge(source.id, self.source_tools.remove)
            .ok();
        self.source_tools.field = 0;
        self.source_tools.preview = None;
        self.source_tools.page = 0;
        self.source_tools.revision = self.editor.revision();
        self.source_tools.error.clear();
        self.source_tools.merge_open.set(1);
    }
    pub(crate) fn open_source_audit(&mut self) {
        self.source_tools.issues = self.editor.document().audit_sources();
        // The modal report freezes its document snapshot. Do not rescan every citation
        // per source on every layout, scroll or filter pass.
        self.source_tools.issue_sources =
            self.source_tools.issues.iter().map(|i| i.source).collect();
        self.source_tools.cited_sources = self
            .editor
            .document()
            .cited_source_ids()
            .into_iter()
            .collect();
        self.source_tools.audit_source = self.active_source().map(|s| s.id).unwrap_or(0);
        self.source_tools.only_cited = 0;
        self.source_tools.audit_open.set(1);
    }
    pub(crate) fn source_tools_dialogs(&mut self, ui: &mut Ui) {
        if ui.dialog_begin(
            1000,
            "Quellen zusammenführen",
            &self.source_tools.merge_open,
        ) {
            let doc = self.editor.document();
            let state = &mut self.source_tools;
            let kept = doc.source(state.keep).unwrap();
            ui.text(TS_CAPTION, "Verbleibende Quelle (ihre ID bleibt erhalten)");
            ui.next_size(-1, 42);
            ui.scroll_begin(1001, SC_V | BOX_V);
            ui.text(TS_BODY_STRONG | TX_WRAP, &source_label(kept));
            ui.scroll_end();
            ui.checkbox(
                1002,
                "Nur mögliche Dubletten anzeigen",
                &mut state.only_similar,
            );
            let sources: Vec<&Source> = doc
                .sources
                .iter()
                .filter(|s| {
                    s.id != state.keep && (state.only_similar == 0 || state.similar.contains(&s.id))
                })
                .collect();
            ui.text(
                TS_CAPTION,
                "Zusätzliche Quelle (wird nach dem Zusammenführen entfernt)",
            );
            let names: Vec<String> = sources.iter().map(|s| source_label(s)).collect();
            let mut selected = sources
                .iter()
                .position(|s| s.id == state.remove)
                .unwrap_or(0) as i32;
            ui.next_size(-1, 0);
            if sources.is_empty() {
                ui.next_flags(ST_DISABLED);
                ui.combobox(1003, &mut selected, &["Keine möglichen Dubletten gefunden"]);
                state.plan = None;
            } else {
                ui.combobox(
                    1003,
                    &mut selected,
                    &names.iter().map(String::as_str).collect::<Vec<_>>(),
                );
                let remove = sources[selected as usize].id;
                if state.remove != remove || state.plan.is_none() {
                    state.remove = remove;
                    state.plan = doc.plan_source_merge(state.keep, remove).ok();
                    state.error.clear();
                }
            }
            ui.box_begin(0, BOX_H, 6);
            for (i, label) in ["Feldvergleich", "Ergebnis"].iter().enumerate() {
                let event = if state.page == i as i32 {
                    ui.accent_button(1004 + i as u32, label)
                } else {
                    ui.button(1004 + i as u32, label)
                };
                if event & EV_ACT != 0 {
                    state.page = i as i32;
                }
            }
            ui.box_end();
            ui.next_size(-1, 268);
            ui.scroll_begin(1006 + state.page as u32, SC_V | BOX_V);
            if let Some(plan) = &mut state.plan {
                let removed = doc.source(plan.remove).unwrap();
                let differences = SourceField::ALL
                    .iter()
                    .filter(|f| !kept.field_equals(removed, **f))
                    .count();
                ui.text(
                    TS_CAPTION,
                    &format!(
                        "{differences} abweichende Felder · {} aus zusätzlicher Quelle",
                        plan.incoming.len()
                    ),
                );
                if state.page == 0 {
                    let fields: Vec<&str> =
                        SourceField::ALL.iter().map(|f| field_label(*f)).collect();
                    ui.next_size(-1, 0);
                    ui.combobox(1010, &mut state.field, &fields);
                    let field = SourceField::ALL[state.field as usize];
                    ui.box_begin(0, BOX_H, 8);
                    for (id, heading, source) in [
                        (1011, "Verbleibende Quelle", kept),
                        (1012, "Zusätzliche Quelle", removed),
                    ] {
                        ui.next_size(-1, 126);
                        ui.scroll_begin(id, SC_V | BOX_V | BOX_CARD | pad(6));
                        ui.text(TS_CAPTION | TX_SECONDARY, heading);
                        ui.text(TS_BODY | TX_WRAP, &field_text(source, field));
                        ui.scroll_end();
                    }
                    ui.box_end();
                    let mut incoming = i32::from(plan.incoming.contains(&field));
                    if ui.checkbox(
                        1013,
                        "Wert der zusätzlichen Quelle verwenden",
                        &mut incoming,
                    ) & EV_ACT
                        != 0
                    {
                        plan.incoming.retain(|f| *f != field);
                        if incoming != 0 {
                            plan.incoming.push(field);
                        }
                    }
                    ui.text(TS_CAPTION | TX_WRAP, if kept.field_equals(removed, field) {
                        "Die Angaben dieses Feldes sind identisch."
                    } else { "Leere Felder werden im Entwurf ergänzt. Abweichende belegte Angaben bleiben zunächst erhalten." });
                } else {
                    if !state
                        .preview
                        .as_ref()
                        .is_some_and(|(cached, _)| cached == plan)
                    {
                        state.preview = Some((plan.clone(), doc.preview_source_merge(plan)));
                    }
                    match &state.preview.as_ref().unwrap().1 {
                        Ok(merged) => {
                            for field in SourceField::ALL {
                                ui.text(TS_CAPTION | TX_SECONDARY, field_label(field));
                                ui.text(TS_BODY | TX_WRAP, &field_text(merged, field));
                            }
                        }
                        Err(_) => ui.text(
                            TS_BODY | TX_WRAP,
                            "Diese Auswahl ist ungültig oder überschreitet die Dokumentgrenzen.",
                        ),
                    }
                }
            } else {
                ui.text(
                    TS_BODY | TX_WRAP,
                    "Für einen manuellen Vergleich den Dublettenfilter ausschalten.",
                );
            }
            ui.scroll_end();
            ui.next_size(-1, 62);
            ui.text(TS_CAPTION | TX_WRAP, if state.error.is_empty() {
                "Die zusätzliche Quelle wird entfernt; alle ihre Zitate werden umgehängt. Nur die gewählten Angaben bleiben erhalten. Rückgängig stellt beide Quellen wieder her."
            } else { &state.error });
            let action = ui.dialog_buttons(
                if state.plan.is_some() {
                    Some("Zusammenführen")
                } else {
                    None
                },
                None,
                Some("Abbrechen"),
            );
            if action == DLG_PRIMARY {
                let plan = state.plan.clone().unwrap();
                if state.revision != self.editor.revision() {
                    state.error = String::from(
                        "Das Dokument wurde geändert. Bitte den Vergleich neu öffnen.",
                    );
                    state.merge_open.set(1);
                } else {
                    let uses = self.editor.document().source_uses(plan.remove).len();
                    let keep = plan.keep;
                    if self.apply(Edit::MergeSources(plan)).is_some() {
                        self.source_sel = self
                            .editor
                            .document()
                            .sources
                            .iter()
                            .position(|s| s.id == keep)
                            .unwrap() as i32;
                        self.source_query.clear();
                        self.message = format!(
                            "Quellen zusammengeführt · {uses} Zitatfelder umgehängt · ein Rückgängig-Schritt"
                        );
                    } else {
                        self.source_tools.error = String::from(
                            "Die Auswahl ist ungültig oder überschreitet Dokumentgrenzen. Beide Quellen bleiben unverändert.",
                        );
                        self.source_tools.merge_open.set(1);
                    }
                }
            } else if action == DLG_CLOSE {
                self.message = String::from("Quellenvergleich abgebrochen · Dokument unverändert");
            }
        }
        ui.dialog_end();
        self.source_audit_dialog(ui);
    }
    fn source_audit_dialog(&mut self, ui: &mut Ui) {
        let was_open = self.source_tools.audit_open.get() != 0;
        if ui.dialog_begin(1050, "Quellen prüfen", &self.source_tools.audit_open) {
            let doc = self.editor.document();
            let state = &mut self.source_tools;
            ui.next_size(-1, 52);
            ui.text(TS_CAPTION | TX_WRAP, "Hinweise auf fehlende Autoren, Jahres- und Publikationsangaben. Prüft außerdem, ob Teile des Zugriffsdatums fehlen. Quellen werden dabei nicht geändert.");
            ui.checkbox(1051, "Nur zitierte Quellen prüfen", &mut state.only_cited);
            let sources: Vec<&Source> = doc
                .sources
                .iter()
                .filter(|s| {
                    state.issue_sources.contains(&s.id)
                        && (state.only_cited == 0 || state.cited_sources.contains(&s.id))
                })
                .collect();
            let mut selected = sources
                .iter()
                .position(|s| s.id == state.audit_source)
                .unwrap_or(0) as i32;
            let names: Vec<String> = sources.iter().map(|s| source_label(s)).collect();
            ui.next_size(-1, 0);
            if sources.is_empty() {
                ui.next_flags(ST_DISABLED);
                ui.combobox(1052, &mut selected, &["Keine Quellen mit Hinweisen"]);
            } else {
                ui.combobox(
                    1052,
                    &mut selected,
                    &names.iter().map(String::as_str).collect::<Vec<_>>(),
                );
                state.audit_source = sources[selected as usize].id;
            }
            ui.text(
                TS_CAPTION,
                &format!(
                    "{} {} mit Hinweisen in dieser Auswahl",
                    sources.len(),
                    if sources.len() == 1 {
                        "Quelle"
                    } else {
                        "Quellen"
                    }
                ),
            );
            ui.next_size(-1, 270);
            ui.scroll_begin(1053, SC_V | BOX_V | pad(6));
            if let Some(source) = sources.get(selected as usize) {
                ui.text(TS_BODY_STRONG | TX_WRAP, &source.title);
                for issue in state.issues.iter().filter(|i| i.source == source.id) {
                    ui.text(TS_BODY | TX_WRAP, &format!("• {}", issue_label(issue.kind)));
                }
            } else {
                ui.text(
                    TS_BODY | TX_WRAP,
                    "Keine Hinweise für diese Auswahl. Die Quellenangaben wurden nicht verändert.",
                );
            }
            ui.scroll_end();
            if ui.dialog_buttons(
                if sources.is_empty() {
                    None
                } else {
                    Some("Quelle bearbeiten")
                },
                None,
                Some("Schließen"),
            ) == DLG_PRIMARY
            {
                state.pending_edit = Some(state.audit_source);
            }
        }
        ui.dialog_end();
        // Queue the editor only after the closing report has restored its focus.
        if !was_open {
            if let Some(id) = self.source_tools.pending_edit.take() {
                if let Some(index) = self
                    .editor
                    .document()
                    .sources
                    .iter()
                    .position(|s| s.id == id)
                {
                    self.source_sel = index as i32;
                    self.source_query.clear();
                    ui.cmd_run(SOURCE_EDIT, 0);
                }
            }
        }
    }
}
