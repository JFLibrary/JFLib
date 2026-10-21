// Editor controls and draft dialogs. Document mutations remain atomic doc_core operations.
use crate::Word;
use alloc::{format, string::String, vec};
use doc_core::{Edit, PageSettings, ParagraphFormat, SearchOptions, TextMatch};
use ui::*;

pub(crate) const HIGHLIGHT_LABELS: [&str; 17] = ["Keine Farbe", "Gelb", "Grün", "Türkis", "Magenta",
    "Blau", "Rot", "Dunkelblau", "Dunkeltürkis", "Dunkelgrün", "Dunkelmagenta", "Dunkelrot",
    "Dunkelgelb", "Dunkelgrau", "Hellgrau", "Schwarz", "Weiß"];

/// One property patch for both continuous and structured editing. Mixed toggles turn on
/// for the whole selection; a second invocation explicitly clears inherited formatting.
pub(crate) fn character_patch(command: u32, common: doc_core::TextFormat) -> doc_core::TextFormat {
    use doc_core::{Highlight, TextFormat, TextPositioning};
    let mut f = TextFormat::default();
    match command {
        30 => f.bold = Some(common.bold != Some(true)),
        31 => f.italic = Some(common.italic != Some(true)),
        32 => f.underline = Some(common.underline != Some(true)),
        55 => f.strike = Some(common.strike != Some(true)),
        100..=104 => f.color = Some(text_color(command)),
        crate::FORMAT_SUPERSCRIPT | crate::FORMAT_SUBSCRIPT => {
            let position = if command == crate::FORMAT_SUPERSCRIPT { TextPositioning::Superscript } else { TextPositioning::Subscript };
            f.positioning = Some(if common.positioning == Some(position) { TextPositioning::Baseline } else { position });
        }
        3700..=3716 => f.highlight = Some(Highlight::ALL[(command - 3700) as usize]),
        _ => (),
    }
    f
}

pub(crate) struct Formatting {
    paragraph_open: Shared<i32>,
    page_open: Shared<i32>,
    paragraph: ParagraphFormat,
    page: PageSettings,
    pub(crate) search_open: bool,
    query: String,
    replacement: String,
    sensitive: i32,
    whole: i32,
    found: Option<TextMatch>,
    error: String,
    focus_paragraph: Option<u64>,
    pub(crate) selection: Option<(u64, usize, usize, usize)>,
    pub(crate) pending_selection: Option<(u64, usize, usize, usize)>,
}
impl Default for Formatting {
    fn default() -> Self {
        Self {
            paragraph_open: Shared::new(0),
            page_open: Shared::new(0),
            paragraph: Default::default(),
            page: Default::default(),
            search_open: false,
            query: String::new(),
            replacement: String::new(),
            sensitive: 1,
            whole: 0,
            found: None,
            error: String::new(),
            focus_paragraph: None,
            selection: None,
            pending_selection: None,
        }
    }
}
impl Formatting {
    pub(crate) fn modal(&self) -> bool {
        self.paragraph_open.get() != 0 || self.page_open.get() != 0
    }
    pub(crate) fn cancel(&self) {
        self.paragraph_open.set(0);
        self.page_open.set(0);
    }
}
impl Word {
    pub(crate) fn character_format(&mut self, ui: &mut Ui, command: u32, size: Option<u16>) {
        self.character_properties(ui, command, size, None);
    }
    pub(crate) fn character_family(&mut self, ui: &mut Ui, family: doc_core::FontFamily) {
        self.character_properties(ui, 0, None, Some(family));
    }
    fn character_properties(&mut self, ui: &mut Ui, command: u32, size: Option<u16>, family: Option<doc_core::FontFamily>) {
        if !self.structured_edit {
            self.rich_character_properties(ui, command, size, family);
            return;
        }
        let paragraph = self.selected_id();
        let p = self.editor.document().paragraph(paragraph).unwrap();
        let target = self.formatting.selection.filter(|(id, inline, _, _)| {
            *id == paragraph && p.content.get(*inline).is_some_and(|i| i.text().is_some())
        });
        let (inline, start, end) = if let Some((_, i, a, b)) = target {
            (i, a.min(b), a.max(b))
        } else if let Some((i, node)) = p
            .content
            .iter()
            .enumerate()
            .find(|(_, n)| n.text().is_some())
        {
            (i, 0, node.text().unwrap().len())
        } else {
            self.message = String::from("Bitte ein Textsegment auswählen.");
            return;
        };
        let node = &p.content[inline];
        let len = node.text().unwrap().len();
        let (start, end) = if start == end { (0, len) } else { (start, end) };
        let mut f = node.format();
        let resolved = self.editor.document().resolve_paragraph_text(p, f);
        f = f.overlay(character_patch(command, resolved.explicit()));
        if let Some(size) = size {
            f.size_half_points = Some(size);
        }
        f.font_family = family.or(f.font_family);
        let revision = self.editor.revision();
        if self
            .apply_page_edit(ui,Edit::FormatText {
                paragraph,
                inline,
                start,
                end,
                format: f,
            })
            .is_some()
        {
            if self.editor.revision() == revision {
                self.formatting.pending_selection = Some((paragraph, inline, start, end));
                let id = self.text_id(paragraph, inline);
                ui.focus(id);
                return;
            }
            self.widget_ids.clear();
            let target = inline + usize::from(start > 0);
            self.formatting.selection = Some((paragraph, target, 0, end - start));
            self.formatting.pending_selection = self.formatting.selection;
            let id = self.text_id(paragraph, target);
            ui.focus(id);
        }
    }
    pub(crate) fn open_paragraph_format(&mut self) {
        self.formatting.error.clear();
        self.formatting.paragraph = self.editor.document().resolve_paragraph(self.formatting_paragraph());
        self.formatting.paragraph.explicit = 255;
        self.formatting.paragraph_open.set(1);
    }
    pub(crate) fn open_page_format(&mut self) {
        self.formatting.error.clear();
        self.formatting.page = self.editor.document().page;
        self.formatting.page_open.set(1);
    }
    pub(crate) fn formatting_dialogs(&mut self, ui: &mut Ui) {
        let paragraph_was_open = self.formatting.paragraph_open.get() != 0;
        if ui.dialog_begin(850, "Absatz formatieren", &self.formatting.paragraph_open) {
            let f = &mut self.formatting.paragraph;
            ui.text(TS_CAPTION, "Abstände und Einzüge in Punkten");
            for (id, label, value) in [
                (851, "Abstand davor", &mut f.before),
                (852, "Abstand danach", &mut f.after),
                (853, "Einzug links", &mut f.left),
                (854, "Einzug rechts", &mut f.right),
            ] {
                ui.text(TS_CAPTION, label);
                let mut n = *value as i32 / 20;
                ui.next_size(-1, 0);
                if ui.number_box(id, &mut n, &Range::new(0, 144, 1, 0)) & EV_ACT != 0 {
                    *value = (n * 20) as u16;
                }
            }
            ui.text(TS_CAPTION, "Sondereinzug");
            let mut special = match f.first_line.cmp(&0) {
                core::cmp::Ordering::Equal => 0,
                core::cmp::Ordering::Greater => 1,
                core::cmp::Ordering::Less => 2,
            };
            ui.next_size(-1, 0);
            if ui.combobox(856, &mut special, &["Keiner", "Erste Zeile", "Hängend"]) & EV_ACT != 0
            {
                let amount = f.first_line.abs().max(360);
                f.first_line = match special {
                    1 => amount,
                    2 => -amount,
                    _ => 0,
                };
            }
            if special != 0 {
                ui.text(TS_CAPTION, "Sondereinzug um (Punkte)");
                let mut amount = f.first_line.abs() / 20;
                ui.next_size(-1, 0);
                if ui.number_box(857, &mut amount, &Range::new(0, 144, 1, 0)) & EV_ACT != 0 {
                    f.first_line = amount * 20 * if special == 2 { -1 } else { 1 };
                }
            }
            ui.text(TS_CAPTION, "Zeilenabstand");
            let mut line = match f.line {
                240 => 0,
                276 => 1,
                360 => 2,
                480 => 3,
                _ => 4,
            };
            let custom = format!(
                "Benutzerdefiniert ({} %)",
                (u32::from(f.line) * 100 + 120) / 240
            );
            let mut labels = vec!["Einfach", "1,15-fach", "1,5-fach", "Doppelt"];
            if line == 4 {
                labels.push(&custom);
            }
            ui.next_size(-1, 0);
            if ui.combobox(855, &mut line, &labels) & EV_ACT != 0 {
                f.line = [240, 276, 360, 480, f.line][line as usize];
            }
            // Reserve the message area so failed validation does not resize the modal.
            if ui.button(858, "Tabstopps …") & EV_ACT != 0 {
                self.formatting.paragraph_open.set(0);
                self.open_tabs();
            }
            ui.next_size(-1, 48);
            ui.text(
                TS_CAPTION | TX_WRAP,
                if self.formatting.error.is_empty() {
                    "Einzüge und Abstände gelten für die ausgewählten Absätze."
                } else {
                    &self.formatting.error
                },
            );
            if ui.dialog_buttons(Some("Übernehmen"), None, Some("Abbrechen")) == DLG_PRIMARY {
                if self
                    .paragraph_change(
                        ui,
                        doc_core::ParagraphChange::Properties(self.formatting.paragraph.select(255)),
                    )
                    .is_none()
                {
                    self.formatting.paragraph_open.set(1);
                    self.formatting.error =
                        String::from("Einzüge oder Abstände lassen nicht genug Platz für Text.");
                } else {
                    self.formatting.focus_paragraph = Some(self.selected_id());
                }
            }
        }
        ui.dialog_end();
        if !paragraph_was_open {
            if let Some(paragraph) = self.formatting.focus_paragraph.take() {
                if !self.structured_edit {
                    let widget = self.text_id(paragraph, 0);
                    ui.focus(widget);
                }
            }
        }
        if ui.dialog_begin(860, "Seite einrichten", &self.formatting.page_open) {
            let p = &mut self.formatting.page;
            let mut orientation = i32::from(p.width > p.height);
            ui.text(TS_CAPTION, "Ausrichtung");
            ui.next_size(-1, 0);
            if ui.combobox(861, &mut orientation, &["Hochformat", "Querformat"]) & EV_ACT != 0 {
                if (p.width > p.height) != (orientation == 1) {
                    core::mem::swap(&mut p.width, &mut p.height);
                }
            }
            ui.text(TS_CAPTION, "Papierformat");
            let mut paper = match (p.width.min(p.height), p.width.max(p.height)) {
                (11906, 16838) => 0,
                (12240, 15840) => 1,
                _ => 2,
            };
            let labels = if paper == 2 {
                &["A4", "US Letter", "Benutzerdefiniert"][..]
            } else {
                &["A4", "US Letter"][..]
            };
            ui.next_size(-1, 0);
            if ui.combobox(862, &mut paper, labels) & EV_ACT != 0 && paper != 2 {
                let (w, h) = if paper == 1 {
                    (12240, 15840)
                } else {
                    (11906, 16838)
                };
                (p.width, p.height) = if orientation == 1 { (h, w) } else { (w, h) };
            }
            ui.text(TS_CAPTION, "Seitenränder in Millimetern");
            ui.box_begin(0, BOX_H, 8);
            for (id, label, value) in [
                (863, "Oben", &mut p.top),
                (864, "Unten", &mut p.bottom),
                (865, "Links", &mut p.left),
                (866, "Rechts", &mut p.right),
            ] {
                ui.box_begin(0, BOX_V, 4);
                ui.text(TS_CAPTION, label);
                let mut n = ((*value as u64 * 254 + 7200) / 14400) as i32;
                ui.next_size(86, 0);
                if ui.number_box(id, &mut n, &Range::new(0, 100, 1, 0)) & EV_ACT != 0 {
                    *value = ((n as u64 * 14400 + 127) / 254) as u32;
                }
                ui.box_end();
            }
            ui.box_end();
            ui.next_size(-1, 48);
            ui.text(
                TS_CAPTION | TX_WRAP,
                if self.formatting.error.is_empty() {
                    "Die Einstellungen gelten für das gesamte Dokument."
                } else {
                    &self.formatting.error
                },
            );
            if ui.dialog_buttons(Some("Übernehmen"), None, Some("Abbrechen")) == DLG_PRIMARY {
                if self.apply_page_edit(ui,Edit::SetPage(self.formatting.page)).is_none() {
                    self.formatting.page_open.set(1);
                    self.formatting.error = String::from(
                        "Bitte kleinere Ränder wählen; Textbereich oder Absatzeinzüge passen nicht.",
                    );
                }
            }
        }
        ui.dialog_end();
    }
    pub(crate) fn search_panel(&mut self, ui: &mut Ui) {
        if !self.formatting.search_open {
            return;
        }
        ui.box_begin(810, BOX_V | BOX_CARD | pad(8), 6);
        ui.box_begin(0, BOX_H, 6);
        ui.next_size(200, 0);
        ui.textbox(820, Some("Suchen"), &mut self.formatting.query, 4097);
        ui.next_size(200, 0);
        ui.textbox(
            821,
            Some("Ersetzen durch"),
            &mut self.formatting.replacement,
            4097,
        );
        if ui.button(822, "Nächster Treffer") & EV_ACT != 0 {
            self.find_next(ui);
        }
        if ui.button(823, "Alle ersetzen") & EV_ACT != 0 {
            let options = SearchOptions {
                case_sensitive: self.formatting.sensitive != 0,
                whole_word: self.formatting.whole != 0,
            };
            if let Some(n) = self.apply_page_edit(ui,Edit::ReplaceAll {
                query: self.formatting.query.clone(),
                replacement: self.formatting.replacement.clone(),
                options,
            }) {
                self.widget_ids.clear();
                self.formatting.selection = None;
                self.formatting.found = None;
                self.message = format!("{n} Treffer ersetzt · ein Rückgängig-Schritt");
            }
        }
        if ui.button(824, "Schließen") & EV_ACT != 0 {
            self.formatting.search_open = false;
        }
        ui.box_end();
        ui.box_begin(0, BOX_H, 12);
        ui.checkbox(825, "Groß-/Kleinschreibung", &mut self.formatting.sensitive);
        ui.checkbox(826, "Ganze Wörter", &mut self.formatting.whole);
        ui.text(
            TS_CAPTION | TX_SECONDARY,
            "Text und Anmerkungen, ohne erzeugte Felder",
        );
        ui.box_end();
        ui.box_end();
    }
    fn find_next(&mut self, ui: &mut Ui) {
        let options = SearchOptions {
            case_sensitive: self.formatting.sensitive != 0,
            whole_word: self.formatting.whole != 0,
        };
        match self
            .editor
            .document()
            .find_text(&self.formatting.query, options)
        {
            Ok(matches) if !matches.is_empty() => {
                let at = self
                    .formatting
                    .found
                    .as_ref()
                    .and_then(|f| matches.iter().position(|m| m == f))
                    .map(|n| (n + 1) % matches.len())
                    .unwrap_or(0);
                let found = matches[at].clone();
                self.select_paragraph(found.paragraph);
                let end = if found.start_inline == found.end_inline {
                    found.end
                } else {
                    self.current_paragraphs()[self.selected].content[found.start_inline]
                        .text()
                        .unwrap()
                        .len()
                };
                self.formatting.pending_selection =
                    Some((found.paragraph, found.start_inline, found.start, end));
                let id = self.text_id(found.paragraph, found.start_inline);
                ui.focus(id);
                if !self.structured_edit {
                    self.rich_find(ui, &found);
                }
                self.formatting.found = Some(found);
                self.message = format!("Treffer {} von {}", at + 1, matches.len());
                self.jump_to_selected = true;
            }
            Ok(_) => {
                self.message = String::from("Keine Treffer");
                self.formatting.found = None;
            }
            Err(_) => self.message = String::from("Bitte einen gültigen Suchtext eingeben."),
        }
    }
}

pub(crate) fn text_color(command: u32) -> doc_core::TextColor {
    match command {
        101 => doc_core::TextColor::Rgb(0x000000),
        102 => doc_core::TextColor::Rgb(0xc62828),
        103 => doc_core::TextColor::Rgb(0x2459a6),
        104 => doc_core::TextColor::Rgb(0x176b70),
        _ => doc_core::TextColor::Auto,
    }
}
