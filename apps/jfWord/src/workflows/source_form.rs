// Source dialog state and controls; validation and formatting live in doc_core.
use alloc::{format, string::String, vec};
use doc_core::{Person, Source, SourceKind};
use ui::*;

pub(crate) struct SourceForm {
    source: Source,
    page: usize,
    person: i32,
    corporate_mode: bool,
    error: String,
}
impl Default for SourceForm {
    fn default() -> Self {
        Self::new(Source::default())
    }
}
impl SourceForm {
    pub(crate) fn set_error(&mut self, error: &str) {
        self.error = String::from(error);
    }
    pub(crate) fn new(mut source: Source) -> Self {
        let corporate_mode = !source.corporate.is_empty();
        if source.authors.is_empty() {
            source.authors = vec![Person::default()];
        }
        Self {
            source,
            page: 0,
            person: 0,
            corporate_mode,
            error: String::new(),
        }
    }
    pub(crate) fn show(&mut self, ui: &mut Ui, open: &Shared<i32>, title: &str) -> Option<Source> {
        let mut result = None;
        if ui.dialog_begin(700, title, open) {
            ui.box_begin(0, BOX_H, 6);
            for (i, label) in ["Publikation", "Autoren", "Details"].iter().enumerate() {
                let event = if self.page == i {
                    ui.accent_button(706 + i as u32, label)
                } else {
                    ui.button(706 + i as u32, label)
                };
                if event & EV_ACT != 0 {
                    self.page = i;
                }
            }
            ui.box_end();
            ui.next_size(460, 300);
            ui.scroll_begin(750 + self.page as u32, SC_V | BOX_V);
            match self.page {
                0 => {
                    let mut kind = match self.source.kind {
                        SourceKind::Book => 0,
                        SourceKind::JournalArticle => 1,
                        SourceKind::Website => 2,
                    };
                    ui.text(TS_CAPTION, "Quellentyp");
                    ui.next_size(-1, 0);
                    ui.combobox(
                        709,
                        &mut kind,
                        &["Buch", "Zeitschriftenartikel", "Webseite"],
                    );
                    self.source.kind = match kind {
                        1 => SourceKind::JournalArticle,
                        2 => SourceKind::Website,
                        _ => SourceKind::Book,
                    };
                    field(ui, 712, "Titel der Quelle *", &mut self.source.title);
                    field(ui, 713, "Jahr", &mut self.source.year);
                    match self.source.kind {
                        SourceKind::Book => field(ui, 714, "Verlag", &mut self.source.publisher),
                        SourceKind::JournalArticle => {
                            field(ui, 716, "Zeitschrift", &mut self.source.journal)
                        }
                        SourceKind::Website => {
                            field(ui, 717, "Name der Website", &mut self.source.website)
                        }
                    }
                }
                1 => self.authors(ui),
                _ => {
                    ui.text(
                        TS_CAPTION | TX_SECONDARY | TX_WRAP,
                        "Angaben bleiben auch bei einem Wechsel des Quellentyps erhalten.",
                    );
                    for (id, label, value) in [
                        (760, "URL / DOI-Link", &mut self.source.url),
                        (761, "Band", &mut self.source.volume),
                        (762, "Heft", &mut self.source.issue),
                        (763, "Seiten", &mut self.source.pages),
                        (764, "Verlagsort", &mut self.source.city),
                        (765, "Auflage", &mut self.source.edition),
                        (
                            766,
                            "Standardnummer (ISBN / ISSN)",
                            &mut self.source.standard_number,
                        ),
                        (767, "Zugriff: Jahr", &mut self.source.year_accessed),
                        (768, "Zugriff: Monat", &mut self.source.month_accessed),
                        (769, "Zugriff: Tag", &mut self.source.day_accessed),
                        (770, "Kurztitel", &mut self.source.short_title),
                        (771, "Notizen", &mut self.source.comments),
                    ] {
                        field(ui, id, label, value);
                    }
                }
            }
            ui.scroll_end();
            ui.text(
                TS_CAPTION | TX_SECONDARY | TX_WRAP,
                if self.error.is_empty() {
                    "* Titel erforderlich · Autoren in Zitierreihenfolge"
                } else {
                    &self.error
                },
            );
            if ui.dialog_buttons(Some("Quelle übernehmen"), None, Some("Abbrechen")) == DLG_PRIMARY
            {
                let mut source = self.source.clone();
                if self.corporate_mode {
                    source.authors.clear();
                } else {
                    source.corporate.clear();
                    source.authors.retain(|p| !p.display().trim().is_empty());
                }
                match source.validate() {
                    Ok(()) => result = Some(source),
                    Err(_) => {
                        self.error = String::from("Bitte Titel und Quellenangaben prüfen.");
                        open.set(1);
                    }
                }
            }
        }
        ui.dialog_end();
        result
    }
    fn authors(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_H, 6);
        let people = if self.corporate_mode {
            ui.button(736, "Personen")
        } else {
            ui.accent_button(736, "Personen")
        };
        if people & EV_ACT != 0 {
            self.corporate_mode = false;
        }
        let corporate = if self.corporate_mode {
            ui.accent_button(737, "Körperschaft")
        } else {
            ui.button(737, "Körperschaft")
        };
        if corporate & EV_ACT != 0 {
            self.corporate_mode = true;
        }
        ui.box_end();
        if self.corporate_mode {
            field(
                ui,
                738,
                "Organisation / Körperschaft",
                &mut self.source.corporate,
            );
            return;
        }
        self.person = self.person.clamp(0, self.source.authors.len() as i32 - 1);
        let names = self
            .source
            .authors
            .iter()
            .enumerate()
            .map(|(i, p)| {
                format!(
                    "{}. {}",
                    i + 1,
                    if p.display().is_empty() {
                        String::from("Neue Person")
                    } else {
                        p.display()
                    }
                )
            })
            .collect::<alloc::vec::Vec<_>>();
        ui.next_size(-1, 0);
        ui.combobox(
            730,
            &mut self.person,
            &names
                .iter()
                .map(String::as_str)
                .collect::<alloc::vec::Vec<_>>(),
        );
        let selected = self.person as usize;
        let person = &mut self.source.authors[selected];
        // Distinct IDs prevent a focused textbox from retaining another person's edit state.
        let offset = selected as u32 * 4;
        field(ui, 0x200000 + offset, "Vorname", &mut person.given);
        field(
            ui,
            0x200001 + offset,
            "Weitere Vornamen",
            &mut person.middle,
        );
        field(ui, 0x200002 + offset, "Nachname", &mut person.family);
        ui.box_begin(0, BOX_H, 6);
        if self.source.authors.len() >= 64 {
            ui.next_flags(ST_DISABLED);
        }
        if ui.button(731, "+ Person") & EV_ACT != 0 {
            self.source.authors.push(Person::default());
            self.person = self.source.authors.len() as i32 - 1;
        }
        if self.source.authors.len() == 1 {
            ui.next_flags(ST_DISABLED);
        }
        if ui.button(732, "Entfernen") & EV_ACT != 0 {
            self.source.authors.remove(selected);
        }
        if selected == 0 {
            ui.next_flags(ST_DISABLED);
        }
        if ui.button(733, "Nach oben") & EV_ACT != 0 {
            self.source.authors.swap(selected, selected - 1);
            self.person -= 1;
        }
        if selected + 1 >= self.source.authors.len() {
            ui.next_flags(ST_DISABLED);
        }
        if ui.button(734, "Nach unten") & EV_ACT != 0 {
            self.source.authors.swap(selected, selected + 1);
            self.person += 1;
        }
        ui.box_end();
    }
}
fn field(ui: &mut Ui, id: u32, label: &str, value: &mut String) {
    ui.text(TS_CAPTION, label);
    ui.next_size(-1, 0);
    ui.textbox(id, Some(label), value, 4097);
}
