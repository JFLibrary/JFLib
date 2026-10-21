// Navigation over existing document identities and layout. No document-format changes.
use crate::Word;
use alloc::{format, string::String, vec::Vec};
use doc_core::{SearchOptions, TextMatch};
use ui::*;

#[derive(Default)]
pub(crate) struct Navigation {
    tab: i32,
    query: String,
    sensitive: i32,
    whole: i32,
    cached: Option<(u64, String, i32, i32)>,
    matches: Vec<TextMatch>,
    error: bool,
    heading_start: usize,
    page_start: usize,
    result_start: usize,
}

fn excerpt(text: &str, count: usize) -> String {
    let mut label: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some((end, _)) = label.char_indices().nth(count) {
        label.truncate(end);
        label.push('…');
    }
    label
}

fn pages(ui: &mut Ui, id: u32, start: &mut usize, count: usize, step: usize) {
    if count <= step {
        *start = 0;
        return;
    }
    *start = (*start / step * step).min((count - 1) / step * step);
    ui.box_begin(0, BOX_H, 4);
    ui.next_size(28, 24);
    ui.next_flags(ST_SUBTLE | if *start == 0 { ST_DISABLED } else { 0 });
    if ui.icon_button(id, IC_BACK, "Vorherige Einträge") & EV_ACT != 0 {
        *start = start.saturating_sub(step);
    }
    ui.next_size(-1, 24);
    ui.text(
        TS_CAPTION | TX_CENTER,
        &format!("{}–{} von {count}", *start + 1, (*start + step).min(count)),
    );
    ui.next_size(28, 24);
    ui.next_flags(
        ST_SUBTLE
            | if *start + step >= count {
                ST_DISABLED
            } else {
                0
            },
    );
    if ui.icon_button(id + 1, IC_FORWARD, "Weitere Einträge") & EV_ACT != 0 {
        *start = (*start + step).min((count - 1) / step * step);
    }
    ui.box_end();
}

impl Word {
    pub(crate) fn navigation_view(&mut self, ui: &mut Ui) {
        self.preview
            .update(ui, self.editor.document(), self.editor.revision());
        ui.next_size(260, -1);
        ui.box_begin(2490, BOX_V, 0);
        let panel = ui.box_rect();
        ui.cmd_rrect(panel[0], panel[1], panel[2], panel[3], 0, ui.dp(1),
            ui.color(T_CARD), ui.color(T_CARD_STROKE), ui.color(T_CARD_STROKE));
        ui.draw_color(ui.color(T_CARD_STROKE));
        ui.draw_rect(
            panel[0] + panel[2] - ui.dp(2),
            panel[1],
            ui.dp(2),
            panel[3],
        );
        ui.next_size(259, -1);
        ui.box_begin(2493, BOX_V | pad(11), 0);
        ui.box_begin(2491, BOX_H | BOX_XCENTER, 4);
        ui.next_size(-1, 22);
        ui.text(TS_SUBTITLE, "Navigation");
        ui.next_size(29, 18);
        ui.next_flags(ST_ICON_ONLY);
        ui.next_icon(IC_CHROME_CLOSE);
        if ui.button(2492, "Navigation schließen") & EV_ACT != 0 {
            self.show_navigation = false;
        }
        ui.box_end();
        ui.spacer(8);
        ui.next_size(-1, 19);
        if ui.textbox(
            2501,
            Some("Dokument durchsuchen"),
            &mut self.navigation.query,
            4097,
        ) & EV_ACT
            != 0
        {
            self.navigation.tab = 2;
        }
        ui.spacer(7);
        ui.next_size(-1, 22);
        ui.next_flags(ST_COMPACT);
        ui.tabs(
            2502,
            &mut self.navigation.tab,
            &["Überschriften", "Seiten", "Ergebnisse"],
        );
        ui.spacer(4);
        ui.separator();
        if self.navigation.tab == 2 {
            ui.next_size(-1, 22);
            ui.checkbox(
                2520,
                "Groß-/Kleinschreibung",
                &mut self.navigation.sensitive,
            );
            ui.next_size(-1, 22);
            ui.checkbox(2521, "Ganze Wörter", &mut self.navigation.whole);
        }
        ui.next_size(-1, -1);
        ui.scroll_begin(104, SC_V | BOX_V);
        match self.navigation.tab {
            1 => self.navigation_pages(ui),
            2 => self.navigation_results(ui),
            _ => self.navigation_headings(ui),
        }
        ui.scroll_end();
        ui.box_end();
        ui.box_end();
    }

    fn navigation_headings(&mut self, ui: &mut Ui) {
        let document = self.editor.document();
        let has_headings = document
            .paragraphs
            .iter()
            .any(|p| document.outline_level(p.style).is_some());
        let items: Vec<_> = document
            .paragraphs
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                !has_headings
                    || document.outline_level(p.style).is_some()
            })
            .map(|(index, p)| (index, p.id, p.style))
            .collect();
        if !has_headings {
            ui.next_size(-1, 40);
            ui.text(
                TS_CAPTION | TX_SECONDARY | TX_WRAP,
                "Noch keine Überschriften. Absätze im Dokument:",
            );
        }
        pages(
            ui,
            2530,
            &mut self.navigation.heading_start,
            items.len(),
            64,
        );
        let mut target = None;
        for &(index, id, style) in items.iter().skip(self.navigation.heading_start).take(64) {
            let label = if has_headings {
                self.editor
                    .document()
                    .text_projection(id)
                    .map(|p| excerpt(&p.text, 80))
                    .unwrap_or_default()
            } else {
                format!("{} · {}", index + 1, crate::style_label(style))
            };
            let label = if label.is_empty() {
                String::from("Leere Überschrift")
            } else {
                label
            };
            ui.box_begin(0, BOX_H, 0);
            if has_headings && document.outline_level(style).is_some_and(|n| n > 0) {
                ui.spacer(14);
            }
            ui.next_size(-1, 26);
            ui.next_flags(
                ST_SUBTLE
                    | ST_ALIGN_START
                    | if id == self.selected_id() {
                        ST_CHECKED
                    } else {
                        0
                    },
            );
            // Preserve small-document IDs; large outlines must not overlap text IDs.
            let widget = if index < 4000 {
                5000 + index as u32
            } else {
                0x540000 + index as u32
            };
            let typography = if has_headings && document.outline_level(style) == Some(0) {
                TS_BODY_STRONG
            } else { TS_BODY };
            if ui.label_button(widget, &label, typography, 8) & EV_ACT != 0 {
                target = Some(id);
            }
            ui.tooltip(&label);
            ui.box_end();
        }
        self.notes_navigation(ui);
        if let Some(id) = target {
            self.navigation_target(ui, id, 0);
        }
    }

    fn navigation_pages(&mut self, ui: &mut Ui) {
        let Some(layout) = self.preview.layout() else {
            ui.text(TS_CAPTION | TX_WRAP, self.preview.error());
            return;
        };
        pages(
            ui,
            2532,
            &mut self.navigation.page_start,
            layout.pages.len(),
            12,
        );
        let selected = self.navigation_current_page();
        let mut target = None;
        for index in
            self.navigation.page_start..(self.navigation.page_start + 12).min(layout.pages.len())
        {
            let width = 124;
            let height =
                doc_view::page_height(layout.settings.width, layout.settings.height, width);
            ui.next_size(-1, height);
            ui.box_begin(0, BOX_H, 0);
            ui.spacer(-1);
            ui.next_size(width, height);
            let input = ui.canvas(
                0x500000 + index as u32,
                Some(self.preview.draw(index)),
                0,
                self.editor.revision() as u32,
            );
            if input.ev & EV_ACT != 0 {
                target = Some(index);
            }
            ui.spacer(-1);
            ui.box_end();
            ui.next_size(-1, 26);
            ui.next_flags(ST_SUBTLE | if index == selected { ST_CHECKED } else { 0 });
            if ui.button(0x510000 + index as u32, &format!("Seite {}", index + 1)) & EV_ACT != 0 {
                target = Some(index);
            }
            ui.spacer(6);
        }
        if let Some(index) = target {
            if self.preview_mode {
                self.preview.set_page(index);
            } else if let Some(line) = layout.pages[index]
                .lines
                .iter()
                .find(|line| line.paragraph != 0 && !layout.pages[index].cells.iter().any(|c|Some(c.cell)==line.cell && c.repeated))
            {
                self.navigation_target(ui, line.paragraph, line.start);
            }
        }
    }

    fn navigation_results(&mut self, ui: &mut Ui) {
        let key = (
            self.document_revision(),
            self.navigation.query.clone(),
            self.navigation.sensitive,
            self.navigation.whole,
        );
        if self.navigation.cached.as_ref() != Some(&key) {
            self.navigation.matches.clear();
            self.navigation.error = false;
            self.navigation.result_start = 0;
            if !self.navigation.query.is_empty() {
                match self.editor.document().find_text(
                    &self.navigation.query,
                    SearchOptions {
                        case_sensitive: self.navigation.sensitive != 0,
                        whole_word: self.navigation.whole != 0,
                    },
                ) {
                    Ok(found) => self.navigation.matches = found,
                    Err(_) => self.navigation.error = true,
                }
            }
            self.navigation.cached = Some(key);
        }
        ui.next_size(-1, 0);
        ui.text(
            TS_CAPTION | TX_SECONDARY | TX_WRAP,
            if self.navigation.query.is_empty() {
                "Suchbegriff eingeben."
            } else if self.navigation.error {
                "Suche überschreitet die Dokumentgrenzen."
            } else if self.navigation.matches.is_empty() {
                "Keine Treffer"
            } else {
                "Text und Anmerkungen, ohne erzeugte Felder"
            },
        );
        if !self.navigation.matches.is_empty() {
            ui.text(
                TS_BODY_STRONG,
                &format!("{} Treffer", self.navigation.matches.len()),
            );
        }
        pages(
            ui,
            2534,
            &mut self.navigation.result_start,
            self.navigation.matches.len(),
            32,
        );
        let mut target = None;
        for (index, found) in self
            .navigation
            .matches
            .iter()
            .enumerate()
            .skip(self.navigation.result_start)
            .take(32)
        {
            let label = self
                .editor
                .document()
                .text_projection(found.paragraph)
                .map(|p| {
                    let offset = p.offset(found.start_inline, found.start).unwrap_or(0);
                    let start = p.text[..offset]
                        .char_indices()
                        .rev()
                        .nth(24)
                        .map_or(0, |(at, _)| at);
                    format!("{} · {}", index + 1, excerpt(&p.text[start..], 82))
                })
                .unwrap_or_else(|_| format!("Treffer {}", index + 1));
            ui.next_size(-1, 30);
            ui.next_flags(ST_SUBTLE | ST_ALIGN_START);
            if ui.button(0x530000 + index as u32, &label) & EV_ACT != 0 {
                target = Some((index, found.clone()));
            }
            ui.tooltip(&label);
            ui.next_size(-1, 18);
            ui.text(
                TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS,
                &self.paragraph_label(found.paragraph),
            );
        }
        if let Some((index, found)) = target {
            self.select_paragraph(found.paragraph);
            if self.structured_edit {
                let text = self
                    .editor
                    .document()
                    .paragraph(found.paragraph)
                    .and_then(|p| p.content.get(found.start_inline))
                    .and_then(doc_core::Inline::text)
                    .unwrap_or("");
                let end = if found.start_inline == found.end_inline {
                    found.end
                } else {
                    text.len()
                };
                self.formatting.pending_selection =
                    Some((found.paragraph, found.start_inline, found.start, end));
                let widget = self.text_id(found.paragraph, found.start_inline);
                ui.focus(widget);
            } else {
                self.rich_find(ui, &found);
            }
            self.jump_to_selected = true;
            self.message = format!(
                "Treffer {} von {}",
                index + 1,
                self.navigation.matches.len()
            );
        }
    }

    fn navigation_target(&mut self, ui: &mut Ui, paragraph: u64, offset: usize) {
        if self.preview_mode {
            if let Some(layout) = self.preview.layout() {
                if let Some(index) = layout.pages.iter().position(|p| {
                    p.lines.iter().any(|line| {
                        line.paragraph == paragraph && line.start <= offset && line.end >= offset
                        && !p.cells.iter().any(|c|Some(c.cell)==line.cell && c.repeated)
                    })
                }) {
                    self.preview.set_page(index);
                }
            }
            return;
        }
        if self.structured_edit {
            if !self.select_paragraph(paragraph) {
                return;
            }
            if let Ok(projection) = self.editor.document().text_projection(paragraph) {
                if let Ok(position) = projection.position(offset) {
                    self.formatting.pending_selection =
                        Some((paragraph, position.inline, position.offset, position.offset));
                    let widget = self.text_id(paragraph, position.inline);
                    ui.focus(widget);
                }
            }
        } else if let Ok(story) = self.editor.document().story_projection(paragraph) {
            if let Some(at) = story.offset(paragraph, offset) {
                let _ = self.rich_select(ui, paragraph, at, at);
            }
        }
        self.jump_to_selected = true;
    }

    pub(crate) fn navigation_current_page(&self) -> usize {
        if self.preview_mode {
            return self.preview.page_index();
        }
        let Some(layout) = self.preview.layout() else {
            return 0;
        };
        if let Some((_,page))=self.rich_header_instance(){return page;}
        let paragraph = self.selected_id();
        let offset = self
            .formatting
            .selection
            .filter(|s| s.0 == paragraph)
            .and_then(|(_, inline, _, caret)| {
                self.editor
                    .document()
                    .text_projection(paragraph)
                    .ok()?
                    .offset(inline, caret)
            })
            .unwrap_or(0);
        layout
            .pages
            .iter()
            .enumerate()
            .rev()
            .find(|(_, page)| {
                page.lines.iter().any(|line| {
                    line.paragraph == paragraph && line.start <= offset && line.end >= offset
                        && !page.cells.iter().any(|c|Some(c.cell)==line.cell && c.repeated)
                })
            })
            .map_or(0, |(index, _)| index)
    }
}
