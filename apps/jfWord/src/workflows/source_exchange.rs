// Bibliography import review; parsing, matching and mutations live in shared libraries.
use crate::Word;
use alloc::{format, string::String, vec::Vec};
use doc_core::{Edit, Source, SourceMatch, SourceTarget};
use ui::*;

#[derive(Default)]
pub(crate) struct ImportReview {
    pub(crate) open: Shared<i32>,
    sources: Vec<Source>,
    matches: Vec<SourceMatch>,
    chosen: Vec<i32>,
    selected: i32,
    path: String,
    error: String,
}
fn target_label(target: SourceTarget) -> String {
    match target {
        SourceTarget::Document(id) => format!("Dokumentquelle {id}"),
        SourceTarget::Import(row) => format!("Importeintrag {}", row + 1),
    }
}
fn match_label(m: SourceMatch) -> String {
    match m {
        SourceMatch::New => String::from("Neu"),
        SourceMatch::Identical(target) => format!("Identisch mit {}", target_label(target)),
        SourceMatch::Similar(target) => format!("Mögliche Dublette: {}", target_label(target)),
    }
}
impl Word {
    pub(crate) fn import_sources(&mut self) {
        match doc_io::load_bibliography(&self.dialog_path).and_then(|sources| {
            let matches = self.editor.document().review_sources(&sources)?;
            Ok((sources, matches))
        }) {
            Ok((sources, matches)) => {
                let chosen = matches
                    .iter()
                    .map(|m| i32::from(!matches!(m, SourceMatch::Identical(_))))
                    .collect();
                self.source_import = ImportReview {
                    open: Shared::new(1),
                    sources,
                    matches,
                    chosen,
                    selected: 0,
                    path: self.dialog_path.clone(),
                    error: String::new(),
                };
                self.message = String::from("Quellenimport vorbereitet · Auswahl prüfen");
            }
            Err(e) => {
                self.message = format!("Quellenimport abgebrochen: {e:?}. Keine Quelle übernommen.")
            }
        }
    }
    pub(crate) fn source_import_dialog(&mut self, ui: &mut Ui) {
        if ui.dialog_begin(920, "Quellenimport prüfen", &self.source_import.open) {
            let review = &mut self.source_import;
            ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, &review.path);
            let identical = review
                .matches
                .iter()
                .filter(|m| matches!(m, SourceMatch::Identical(_)))
                .count();
            let similar = review
                .matches
                .iter()
                .filter(|m| matches!(m, SourceMatch::Similar(_)))
                .count();
            ui.text(
                TS_BODY_STRONG,
                &format!(
                    "{} Einträge · {identical} identisch · {similar} zu prüfen",
                    review.sources.len()
                ),
            );
            ui.text(TS_CAPTION | TX_WRAP,
                "Identische Angaben sind abgewählt. Ähnliche Einträge bleiben getrennte Quellen; vorhandene Quellen werden nicht überschrieben.");
            ui.box_begin(0, BOX_H, 8);
            if ui.button(921, "Alle auswählen") & EV_ACT != 0 {
                review.chosen.fill(1);
            }
            if ui.button(922, "Keine auswählen") & EV_ACT != 0 {
                review.chosen.fill(0);
            }
            ui.box_end();
            if !review.sources.is_empty() {
                let labels: Vec<String> = review
                    .sources
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        format!(
                            "{} {} · {}",
                            if review.chosen[i] != 0 { "✓" } else { "–" },
                            i + 1,
                            s.title
                        )
                    })
                    .collect();
                ui.next_size(-1, 0);
                ui.combobox(
                    923,
                    &mut review.selected,
                    &labels.iter().map(String::as_str).collect::<Vec<_>>(),
                );
                let index = review.selected.clamp(0, review.sources.len() as i32 - 1) as usize;
                ui.checkbox(924, "Diesen Eintrag übernehmen", &mut review.chosen[index]);
                ui.text(
                    TS_BODY_STRONG | TX_WRAP,
                    &match_label(review.matches[index]),
                );
                ui.next_size(-1, 156);
                ui.scroll_begin(925, SC_V | BOX_V | BOX_CARD | pad(8));
                let source = &review.sources[index];
                ui.text(TS_BODY | TX_WRAP, &source.bibliography_text());
                for (label, value) in [
                    ("Kurztitel", &source.short_title),
                    ("Notizen", &source.comments),
                    ("Sprachkennung", &source.language),
                    ("Externe Kennung", &source.external_id),
                ] {
                    if !value.is_empty() {
                        ui.text(TS_CAPTION | TX_WRAP, &format!("{label}: {value}"));
                    }
                }
                if let SourceMatch::Similar(target) = review.matches[index] {
                    let original = match target {
                        SourceTarget::Document(id) => self.editor.document().source(id),
                        SourceTarget::Import(row) => review.sources.get(row),
                    };
                    if let Some(original) = original {
                        ui.text(
                            TS_BODY_STRONG | TX_WRAP,
                            &format!("Zum Vergleich: {}", target_label(target)),
                        );
                        ui.text(TS_CAPTION | TX_WRAP, &original.bibliography_text());
                    }
                }
                ui.scroll_end();
            }
            let selected = review.chosen.iter().filter(|v| **v != 0).count();
            ui.text(
                TS_CAPTION,
                &format!(
                    "{selected} von {} ausgewählt · ein Rückgängig-Schritt",
                    review.sources.len()
                ),
            );
            ui.next_size(-1, 44);
            ui.text(TS_CAPTION | TX_WRAP, if review.error.is_empty() {
                "Externe Schlüssel werden auf lokale Quellen-IDs abgebildet. Der Import verändert das Dokument erst beim Übernehmen."
            } else { &review.error });
            match ui.dialog_buttons(Some("Auswahl importieren"), None, Some("Abbrechen")) {
                DLG_PRIMARY => {
                    let selected = review
                        .sources
                        .iter()
                        .zip(&review.chosen)
                        .filter(|(_, choice)| **choice != 0)
                        .map(|(s, _)| s.clone())
                        .collect::<Vec<_>>();
                    let count = selected.len();
                    let skipped = review.sources.len() - count;
                    let before = self.editor.document().sources.len();
                    if let Some(id) = self.apply(Edit::ImportSources(selected)) {
                        if let Some(index) = self
                            .editor
                            .document()
                            .sources
                            .iter()
                            .position(|s| s.id == id)
                        {
                            self.source_sel = index as i32;
                        }
                        self.source_query.clear();
                        let added = self.editor.document().sources.len() - before;
                        self.message = format!(
                            "{added} Quellen importiert · {} nicht übernommen",
                            skipped + count - added
                        );
                        self.source_import.sources.clear();
                    } else {
                        self.source_import.error = String::from(
                            "Die Auswahl überschreitet die Dokumentgrenzen oder enthält ungültige Angaben. Bitte weniger Einträge wählen.",
                        );
                        self.source_import.open.set(1);
                    }
                }
                DLG_CLOSE => {
                    self.message = String::from("Quellenimport abgebrochen · Dokument unverändert");
                    self.source_import.sources.clear();
                }
                _ => (),
            }
        }
        ui.dialog_end();
    }
}
