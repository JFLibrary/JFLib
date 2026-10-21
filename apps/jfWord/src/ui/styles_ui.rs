// Style editing drafts; inheritance, validation and undo belong to doc_core.
use crate::Word;
use alloc::{format, string::String, vec::Vec};
use doc_core::{Alignment, Edit, FontFamily, StyleDefinition, TextColor};
use ui::*;

pub(crate) struct StylesUi {
    pub(crate) open: Shared<i32>,
    pub(crate) gallery_page: usize,
    families: Vec<String>,
    draft: Option<StyleDefinition>,
    family: String,
    color: String,
    error: String,
    tab: i32,
    tabs: doc_view::TabEditor,
}
impl Default for StylesUi {
    fn default() -> Self { Self { open: Shared::new(0), gallery_page: 0, families: Vec::new(), draft: None, family: String::new(), color: String::new(), error: String::new(), tab: 0, tabs: doc_view::TabEditor::new() } }
}
fn toggle(ui: &mut Ui, id: u32, label: &str, value: &mut Option<bool>) {
    ui.text(TS_CAPTION, label);
    let mut i = match value { None => 0, Some(true) => 1, Some(false) => 2 };
    ui.next_size(-1, 0);
    if ui.combobox(id, &mut i, &["Geerbt", "Ein", "Aus"]) & EV_ACT != 0 { *value = [None, Some(true), Some(false)][i as usize]; }
}
fn distance(ui: &mut Ui, id: u32, label: &str, value: &mut u16, mask: &mut u16, bit: u16, default: u16, max: i32) {
    ui.text(TS_CAPTION, label);
    let unit = if bit == 8 { 240.0 } else { 20.0 };
    let mut n = if *mask & bit != 0 || *value != default { f64::from(*value) / unit } else { -1.0 };
    ui.next_size(-1, 0);
    if ui.number_box_f(id, &mut n, &RangeF::new(-1.0, f64::from(max) / unit, if bit == 8 { 0.05 } else { 0.5 }, 2, 0)) & EV_ACT != 0 {
        if n < 0.0 { *value = default; *mask &= !bit; } else { *value = (n * unit + 0.5) as u16; *mask |= bit; }
    }
}
impl Word {
    pub(crate) fn document_families(&mut self, ui: &mut Ui) -> Vec<String> {
        if self.styles_ui.families.is_empty() {
            if let Ok(fonts) = self.preview.prepare_fonts(&mut ui.text) { self.styles_ui.families = fonts.families(); }
        }
        self.styles_ui.families.clone()
    }
    fn style_draft(&mut self, draft: StyleDefinition) {
        self.styles_ui.tabs.reset();
        self.styles_ui.family = draft.text.font_family.map_or_else(String::new, |f| String::from(f.as_str()));
        self.styles_ui.color = match draft.text.color { None => String::new(), Some(TextColor::Auto) => String::from("auto"), Some(TextColor::Rgb(rgb)) => format!("{rgb:06X}") };
        self.styles_ui.draft = Some(draft);
        self.styles_ui.error.clear();
    }
    pub(crate) fn open_styles(&mut self) {
        let style = self.formatting_paragraph().style;
        if let Some(draft) = self.editor.document().style_definition(style) {
            self.style_draft(draft);
            self.styles_ui.open.set(1);
        }
    }
    pub(crate) fn style_dialog(&mut self, ui: &mut Ui) {
        if ui.dialog_begin(3500, "Formatvorlagen", &self.styles_ui.open) {
            ui.next_size(-1, (ui.client_size().1 - 180).clamp(160, 470));
            ui.scroll_begin(3522, SC_V | BOX_V);
            let mut draft = self.styles_ui.draft.clone().unwrap();
            let ids = self.editor.document().style_ids();
            let mut names: Vec<_> = ids.iter().map(|id| self.editor.document().style_definition(*id).unwrap().name).collect();
            let mut selected = ids.iter().position(|id| *id == draft.id).unwrap_or(ids.len()) as i32;
            if selected as usize == ids.len() { names.push(String::from("Neue Formatvorlage")); }
            ui.box_begin(0, BOX_H, 6);
            ui.next_size(-1, 0);
            if ui.combobox(3501, &mut selected, &names.iter().map(String::as_str).collect::<Vec<_>>()) & EV_ACT != 0 && (selected as usize) < ids.len() {
                draft = self.editor.document().style_definition(ids[selected as usize]).unwrap();
                self.style_draft(draft.clone());
            }
            ui.next_size(70, 0);
            if ui.button(3502, "Neu") & EV_ACT != 0 {
                draft = StyleDefinition::new(self.editor.document().next_style_id(), "Neue Formatvorlage");
                draft.based_on = Some(self.formatting_paragraph().style);
                self.style_draft(draft.clone());
            }
            ui.box_end();
            ui.text(TS_CAPTION, "Name");
            ui.next_size(-1, 0);
            ui.textbox(3503, Some("Name der Formatvorlage"), &mut draft.name, 129);
            ui.text(TS_CAPTION, "Basiert auf");
            let mut bases = alloc::vec![None];
            bases.extend(ids.iter().filter(|id| **id != draft.id).map(|id| Some(*id)));
            let base_labels: Vec<_> = bases.iter().map(|id| id.map_or_else(|| String::from("Keine · Dokumentstandard"), |id| self.editor.document().style_definition(id).unwrap().name)).collect();
            let mut base = bases.iter().position(|id| *id == draft.based_on).unwrap_or(0) as i32;
            ui.next_size(-1, 0);
            if ui.combobox(3504, &mut base, &base_labels.iter().map(String::as_str).collect::<Vec<_>>()) & EV_ACT != 0 { draft.based_on = bases[base as usize]; }
            ui.tabs(3550, &mut self.styles_ui.tab, &["Zeichen", "Absatz", "Tabstopps"]);
            if self.styles_ui.tab == 0 {
                let families = self.document_families(ui);
                ui.text(TS_CAPTION, "Schriftfamilie · leer bedeutet geerbt");
                ui.next_size(-1, 0);
                ui.textbox(3505, Some("Geerbt"), &mut self.styles_ui.family, 97);
                let mut font = families.iter().position(|f| *f == self.styles_ui.family).map_or(0, |i| i + 1) as i32;
                let mut labels = alloc::vec!["Geerbt"];
                labels.extend(families.iter().map(String::as_str));
                ui.next_size(-1, 0);
                if ui.combobox(3506, &mut font, &labels) & EV_ACT != 0 { self.styles_ui.family = if font == 0 { String::new() } else { String::from(labels[font as usize]) }; }
                ui.text(TS_CAPTION, "Schriftgröße in Punkten · 0 bedeutet geerbt");
                let mut size = f64::from(draft.text.size_half_points.unwrap_or(0)) / 2.0;
                ui.next_size(-1, 0);
                if ui.number_box_f(3508, &mut size, &RangeF::new(0.0, 144.0, 0.5, 1, 0)) & EV_ACT != 0 { draft.text.size_half_points = if size == 0.0 { None } else { Some((size * 2.0) as u16) }; }
                ui.box_begin(0, BOX_H, 8);
                for (id, name, value) in [(3509, "Fett", &mut draft.text.bold), (3510, "Kursiv", &mut draft.text.italic), (3511, "Unterstrichen", &mut draft.text.underline), (3526, "Durchgestrichen", &mut draft.text.strike)] {
                    ui.next_size(-1, 0); ui.box_begin(0, BOX_V, 2); toggle(ui, id, name, value); ui.box_end();
                }
                ui.box_end();
                ui.text(TS_CAPTION, "Schriftfarbe · RGB, auto oder leer für geerbt");
                ui.next_size(-1, 0);
                ui.textbox(3512, Some("Geerbt"), &mut self.styles_ui.color, 16);
                ui.text(TS_CAPTION, "Texthervorhebung");
                let mut highlights = alloc::vec!["Geerbt"];
                highlights.extend(crate::editor_ui::HIGHLIGHT_LABELS);
                let mut selected = draft.text.highlight.map_or(0, |c| doc_core::Highlight::ALL.iter().position(|v| *v == c).unwrap() as i32 + 1);
                ui.next_size(-1, 0);
                if ui.combobox(3527, &mut selected, &highlights) & EV_ACT != 0 {
                    draft.text.highlight = if selected == 0 { None } else { Some(doc_core::Highlight::ALL[selected as usize - 1]) };
                }
                ui.text(TS_CAPTION, "Zeichenposition");
                let positions = [None, Some(doc_core::TextPositioning::Baseline), Some(doc_core::TextPositioning::Superscript), Some(doc_core::TextPositioning::Subscript)];
                let mut selected = positions.iter().position(|p| *p == draft.text.positioning).unwrap() as i32;
                ui.next_size(-1, 0);
                if ui.combobox(3528, &mut selected, &["Geerbt", "Normal", "Hochgestellt", "Tiefgestellt"]) & EV_ACT != 0 {
                    draft.text.positioning = positions[selected as usize];
                }
            } else if self.styles_ui.tab == 2 {
                ui.text(TS_CAPTION | TX_WRAP, "Tabstopps ergänzen die Basisvorlage. ‚Geerbten Stopp löschen‘ unterdrückt eine bestimmte Position.");
                self.styles_ui.tabs.show(ui, 3570, &mut draft.paragraph.tabs, true);
            } else {
                ui.text(TS_CAPTION, "Ausrichtung");
                let f = &mut draft.paragraph;
                let mut align = if f.mask() & 1 == 0 { 0 } else { match f.alignment { Alignment::Left => 1, Alignment::Center => 2, Alignment::Right => 3, Alignment::Justify => 4 } };
                ui.next_size(-1, 0);
                if ui.combobox(3513, &mut align, &["Geerbt", "Links", "Zentriert", "Rechts", "Blocksatz"]) & EV_ACT != 0 {
                    f.alignment = [Alignment::Left, Alignment::Left, Alignment::Center, Alignment::Right, Alignment::Justify][align as usize];
                    f.explicit = if align == 0 { f.explicit & !1 } else { f.explicit | 1 };
                }
                ui.text(TS_CAPTION, "−1 bedeutet geerbt · Abstände und Einzüge in Punkt");
                distance(ui, 3514, "Abstand davor", &mut f.before, &mut f.explicit, 2, 0, 2880);
                distance(ui, 3515, "Abstand danach", &mut f.after, &mut f.explicit, 4, 160, 2880);
                distance(ui, 3516, "Zeilenabstand (1 = einfach)", &mut f.line, &mut f.explicit, 8, 276, 720);
                ui.box_begin(0, BOX_H, 8);
                ui.next_size(-1, 0); ui.box_begin(0, BOX_V, 2);
                distance(ui, 3517, "Einzug links", &mut f.left, &mut f.explicit, 16, 0, 2880); ui.box_end();
                ui.next_size(-1, 0); ui.box_begin(0, BOX_V, 2);
                distance(ui, 3518, "Einzug rechts", &mut f.right, &mut f.explicit, 32, 0, 2880); ui.box_end();
                ui.box_end();
                let mut first = i32::from(f.mask() & 64 != 0);
                if ui.checkbox(3523, "Erstzeileneinzug festlegen (sonst geerbt)", &mut first) & EV_ACT != 0 {
                    if first != 0 { f.explicit |= 64; } else { f.explicit &= !64; f.first_line = 0; }
                }
                if first != 0 {
                    ui.text(TS_CAPTION, "Erste Zeile in Punkt · negativ bedeutet hängend");
                    let mut n = f64::from(f.first_line) / 20.0;
                    ui.next_size(-1, 0);
                    if ui.number_box_f(3524, &mut n, &RangeF::new(-1584.0, 1584.0, 0.5, 2, 0)) & EV_ACT != 0 { f.first_line = (n * 20.0) as i32; }
                }
                let mut page_break = (f.mask() & 128 != 0).then_some(f.page_break_before);
                toggle(ui, 3525, "Seitenumbruch davor", &mut page_break);
                f.page_break_before = page_break.unwrap_or(false);
                if page_break.is_some() { f.explicit |= 128; } else { f.explicit &= !128; }
                ui.text(TS_CAPTION, "Gliederungsebene");
                let mut outline = draft.outline_level.map_or(0, |n| i32::from(n) + 1);
                ui.next_size(-1, 0);
                if ui.combobox(3519, &mut outline, &["Geerbt", "1", "2", "3", "4", "5", "6", "7", "8", "9", "Textkörper"]) & EV_ACT != 0 { draft.outline_level = if outline == 0 { None } else { Some((outline - 1) as u8) }; }
            }
            let family = self.styles_ui.family.trim();
            let family_valid = family.is_empty() || FontFamily::new(family).is_ok();
            if family_valid { draft.text.font_family = if family.is_empty() { None } else { FontFamily::new(family).ok() }; }
            let color = self.styles_ui.color.trim();
            let color_valid = color.is_empty() || color == "auto" || color.len() == 6 && color.bytes().all(|c| c.is_ascii_hexdigit());
            if color_valid { draft.text.color = if color.is_empty() { None } else if color == "auto" { Some(TextColor::Auto) } else { u32::from_str_radix(color, 16).ok().map(TextColor::Rgb) }; }
            let mut preview = doc_core::Document::default();
            preview.page = self.editor.document().page;
            preview.styles = self.editor.document().styles.clone();
            preview.styles.retain(|s| s.id != draft.id);
            preview.styles.push(draft.clone());
            preview.paragraphs[0].style = draft.id;
            if preview.validate().is_ok() && color_valid && family_valid {
                let f = preview.resolve_text(Default::default(), draft.id);
                if let Ok(fonts) = self.preview.prepare_fonts(&mut ui.text) {
                    fonts.ensure(&mut ui.text, f.font_family);
                    ui.next_size(-1, 62);
                    ui.next_flags(ST_DISABLED);
                    ui.ribbon_sample_tile(3520, f.font_family.as_str(), "AaBbCcDd · Wissenschaft", TS_BODY, false,
                        Some(doc_view::ribbon_sample(f, &fonts)));
                }
            }
            ui.next_size(-1, 32);
            ui.text(TS_CAPTION | TX_WRAP, if self.styles_ui.error.is_empty() { "Änderungen gelten für alle abhängigen Absätze. Direkte Textformatierung bleibt erhalten." } else { &self.styles_ui.error });
            self.styles_ui.draft = Some(draft.clone());
            ui.scroll_end();
            let button = ui.dialog_buttons(Some("Speichern"), Some("Anwenden"), Some("Schließen"));
            if button == DLG_PRIMARY || button == DLG_SECONDARY {
                if !color_valid || !family_valid || self.apply_page_edit(ui,Edit::SetStyleDefinition(draft.clone())).is_none() {
                    self.styles_ui.open.set(1);
                    self.styles_ui.error = String::from("Vorlage ungültig: Name, Schrift, Farbe, Abstände oder zyklische Vererbung prüfen.");
                } else if button == DLG_SECONDARY {
                    self.paragraph_change(ui, doc_core::ParagraphChange::Style(draft.id));
                }
            }
        }
        ui.dialog_end();
    }
}
