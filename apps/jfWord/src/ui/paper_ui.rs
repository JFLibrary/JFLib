// Direct paper editing over the shared document pagination. New jfWord functionality.
// Simple paragraphs use native text selection/IME on their actual page; structured fields
// retain explicit controls in a compact editor so generated citations are never plain text.
use alloc::{format, rc::Rc, string::String};
use doc_core::{Edit, Inline, Paragraph, TextFormat};
use doc_layout::Layout;
use ui::*;

use crate::{Word, preview::Fonts};

/// Reserve the two rulers once; document pages retain their existing shared scroll view.
pub(crate) fn paper_viewport_begin(ui: &mut Ui) -> [i32; 4] {
    ui.next_size(-1, -1);
    ui.box_begin(2850, BOX_V, 0);
    let frame = ui.box_rect();
    ui.draw_color(ui.color(T_BG_ALT));
    ui.draw_rect(frame[0], frame[1], frame[2], frame[3]);
    ui.spacer(doc_view::RULER_SIZE);
    ui.next_size(-1, -1);
    ui.box_begin(2851, BOX_H, 0);
    ui.spacer(doc_view::RULER_SIZE);
    ui.next_size(-1, -1);
    ui.scroll_begin(110, SC_V | SC_H | BOX_V | pad(doc_view::PAGE_GAP as u32));
    frame
}

/// Paint rulers at the exact scrolled page position without changing its document geometry.
impl Word {
pub(crate) fn paper_viewport_end(
    &mut self,
    ui: &mut Ui,
    frame: [i32; 4],
    page_rect: [i32; 4],
    page: doc_core::PageSettings,
) {
    let paragraph = self.editor.document().resolve_paragraph(self.formatting_paragraph());
    let context = self.ruler_context();
    ui.scroll_end();
    ui.spacer(doc_view::SCROLL_GUTTER);
    ui.box_end();
    let extent = ui.dp(doc_view::RULER_SIZE);
    let old = ui.text.get_font();
    ui.ts_font(TS_BODY);
    let font = ui.text.get_font().0;
    ui.text.set_font(old.0, old.1);
    let palette = if ui.theme_dark() {
        doc_view::RulerPalette::dark(ui.color(T_BG_ALT))
    } else {
        doc_view::RulerPalette::light(ui.color(T_BG_ALT))
    };
    ui.next_rect_px(frame[0], frame[1], extent, extent);
    doc_view::tab_selector(ui, 2854, &mut self.rulers.alignment);
    let alignment = self.rulers.alignment();
    let mut edit = None;
    for (id, axis, rect, page_start) in [
        (2852, doc_view::RulerAxis::Horizontal,
         [frame[0] + extent, frame[1], frame[2] - extent, extent], page_rect[0] - frame[0] - extent),
        (2853, doc_view::RulerAxis::Vertical,
         [frame[0], frame[1] + extent, extent, frame[3] - extent], page_rect[1] - frame[1] - extent),
    ] {
        let mut draw = doc_view::RulerDraw {
            paragraph_only: false,
            page, paragraph, axis, page_start, page_width: page_rect[2],
            dpi: ui.dp(96) as u32, font, palette,
        };
        if id == 2852 && self.active_cell.is_some() {
            let (left, right) = self.editor.document().paragraph_bounds(self.selected_id());
            draw.page.left = left; draw.page.right = page.width - right; draw.paragraph_only = true;
        }
        ui.next_rect_px(rect[0], rect[1], rect[2], rect[3]);
        ui.next_flags(CUR_ARROW << HB_CURSOR_SHIFT);
        let mut input = ui.canvas(id, None, 0, 0).clone();
        if ui.focused() != id { input.key = 0; }
        let controller = if id == 2852 { &mut self.rulers.horizontal } else { &mut self.rulers.vertical };
        if let Some(change) = controller.update(&draw, &input, context, alignment) { edit = Some(change); }
        controller.preview(&mut draw, alignment);
        if ui.focused() == id && input.key & 0xffff == VK_ESCAPE { ui.capture_shortcut(VK_ESCAPE, 0); }
        let version = draw.version();
        ui.cmd_custom(rect[0], rect[1], rect[2], rect[3], Some(Rc::new(draw)), 0, version);
        ui.sem(K_CANVAS, id, Some(if id == 2852 { "Horizontales Lineal" } else { "Vertikales Lineal" }), 0);
        ui.tooltip("Ziehen: Einzüge, Ränder und Tabstopps · Alt: ohne Raster · Esc: abbrechen · Tab aus dem Lineal ziehen: löschen");
    }
    ui.box_end();
    if let Some(edit) = edit {
        match edit {
            doc_view::RulerEdit::Paragraph(p) => { self.paragraph_change(ui, doc_core::ParagraphChange::Properties(p)); },
            doc_view::RulerEdit::Page(p) => { self.apply(Edit::SetPage(p)); },
        }
        ui.repass();
    }
}
}

struct ParagraphArea {
    page: usize,
    top: i32,
    height: i32,
    line_height: i32,
    baseline: i32,
}

fn paragraph_area(layout: &Layout, paragraph: u64) -> Option<ParagraphArea> {
    let mut area: Option<ParagraphArea> = None;
    for (page, sheet) in layout.pages.iter().enumerate() {
        for line in sheet
            .lines
            .iter()
            .filter(|line| line.paragraph == paragraph)
        {
            match &mut area {
                Some(area) if area.page != page => return None,
                Some(area) => area.height = line.top + line.height - area.top,
                None => {
                    area = Some(ParagraphArea {
                        page,
                        top: line.top,
                        height: line.height,
                        line_height: line.height,
                        baseline: line.fragments.first().map_or(0, |f| f.baseline - line.top),
                    });
                }
            }
        }
    }
    area
}

fn clicked_caret(
    ui: &mut Ui,
    layout: &Layout,
    paragraph: &Paragraph,
    page: usize,
    x: i32,
    y: i32,
    fonts: &Fonts,
) -> Option<usize> {
    if paragraph.content.len() != 1 {
        return None;
    }
    let text = paragraph.content[0].text()?;
    let target = layout.pages[page]
        .lines
        .iter()
        .filter(|line| line.paragraph == paragraph.id)
        .min_by_key(|line| {
            if y < line.top {
                line.top - y
            } else if y >= line.top + line.height {
                y - line.top - line.height + 1
            } else {
                0
            }
        })?;
    let old = ui.text.get_font();
    let mut offset = 0;
    let mut caret = None;
    for (sheet_index, sheet) in layout.pages.iter().enumerate() {
        for line in sheet
            .lines
            .iter()
            .filter(|line| line.paragraph == paragraph.id)
        {
            let on_line = sheet_index == page && core::ptr::eq(line, target);
            for fragment in &line.fragments {
                let Some(relative) = text[offset..].find(&fragment.text) else {
                    continue;
                };
                let start = offset + relative;
                offset = start + fragment.text.len();
                if on_line && caret.is_none() && x <= fragment.x + fragment.width {
                    ui.text.set_font(
                        fonts.get(fragment.format),
                        fragment.format.display_size_half_points() as i32 * 10 * 256,
                    );
                    caret = Some(
                        start
                            + ui.text
                                .hit(fragment.text.as_bytes(), None, (x - fragment.x).max(0)),
                    );
                }
            }
            if on_line {
                caret = caret.or(Some(offset));
            }
        }
    }
    ui.text.set_font(old.0, old.1);
    caret
}

fn reveal_on_paper(ui: &mut Ui, viewport: [i32; 4], y: i32, height: i32) {
    let margin = ui.dp(24);
    let delta = if y < viewport[1] + margin {
        y - viewport[1] - margin
    } else if y + height > viewport[1] + viewport[3] - margin {
        y + height - viewport[1] - viewport[3] + margin
    } else {
        return;
    };
    let current = ui.scroll_pos(110).1;
    ui.scroll_to(110, -1, (current + ui.px2dip(delta)).max(0));
}

impl Word {
    pub(crate) fn paper_status(&mut self, ui: &mut Ui) {
        ui.next_size(0, 22);
        ui.box_begin(0x550126, BOX_H | BOX_XCENTER, 3);
        ui.next_size(if self.focus_mode { 112 } else { 66 }, 22);
        ui.next_flags(ST_SUBTLE | ST_COMPACT | if self.focus_mode { ST_CHECKED } else { 0 });
        ui.next_icon(IC_OFFICE_FOCUS);
        if ui.button(
            2840,
            if self.focus_mode {
                "Fokus beenden"
            } else {
                "Fokus"
            },
        ) & EV_ACT
            != 0
        {
            ui.cmd_run(crate::VIEW_FOCUS, 0);
        }
        ui.tooltip("Fokusmodus · Strg+Umschalt+F11 · Esc beendet den Fokusmodus");
        ui.next_size(24, 22);
        ui.next_flags(ST_SUBTLE | ST_COMPACT | ST_DISABLED);
        ui.icon_button(0x550124, IC_OFFICE_READ_MODE, "Lesemodus");
        ui.tooltip("Der Lesemodus ist noch nicht verfügbar.");
        ui.next_size(24, 22);
        ui.next_flags(ST_SUBTLE | ST_COMPACT | ST_CHECKED);
        if ui.icon_button(
            48,
            IC_OFFICE_PRINT_LAYOUT,
            if self.preview_mode {
                "Bearbeiten"
            } else {
                "Seitenvorschau"
            },
        ) & EV_ACT
            != 0
        {
            self.preview_mode = !self.preview_mode;
        }
        ui.next_size(24, 22);
        ui.next_flags(ST_SUBTLE | ST_COMPACT | ST_DISABLED);
        ui.icon_button(0x550125, IC_OFFICE_WEB_LAYOUT, "Weblayout");
        ui.tooltip("Die Weblayout-Ansicht ist noch nicht verfügbar.");
        self.preview.zoom_control(ui);
        ui.box_end();
    }
    pub(crate) fn paper(&mut self, ui: &mut Ui) {
        self.preview
            .update(ui, self.editor.document(), self.editor.revision());
        let Some(layout) = self.preview.layout() else {
            ui.text(TS_BODY | TX_WRAP, self.preview.error());
            return;
        };
        let selected = self.current_paragraphs()[self.selected].clone();
        let area = paragraph_area(&layout, selected.id);
        // Edits that span pages, have generated fields or mixed runs use the safe structured
        // strip. Their complete formatted result remains visible in the paper below it.
        let simple = selected.content.is_empty()
            || (selected.content.len() == 1
                && selected.content[0]
                    .text()
                    .is_some_and(|text| !text.contains('\t')));
        let direct = simple
            && !selected
                .content
                .iter()
                .any(|n| {
                    let f = self.editor.document().resolve_paragraph_text(&selected, n.format());
                    f.strike || f.highlight != doc_core::Highlight::None || f.positioning != doc_core::TextPositioning::Baseline
                })
            && self.active_note.is_none()
            && self.editor.document().resolve_paragraph(&selected).alignment == doc_core::Alignment::Left
            && area
                .as_ref()
                .is_some_and(|a| a.height < layout.settings.height as i32 / 3);

        ui.next_size(-1, -1);
        ui.box_begin(135, BOX_V | BOX_FIXED, 0);
        if self.active_note.is_some() {
            ui.next_size(-1, 100);
            ui.box_begin(136, BOX_V | pad(10), 3);
            self.note_editor_header(ui);
            ui.box_end();
        }
        if !direct {
            self.paper_structure(ui, &selected);
        }
        let paper_frame = paper_viewport_begin(ui);
        let viewport = ui.last_rect();
        let available = ui.px2dip(viewport[2]).saturating_sub(doc_view::PAGE_GAP * 2).max(280);
        let width = self.preview.page_width(layout.settings.width, available);
        let height = doc_view::page_height(layout.settings.width, layout.settings.height, width);
        let Some(fonts) = self.preview.fonts() else {
            return;
        };
        let revision = self.editor.revision();
        let mut clicked = None;
        let mut ruler_page = [0; 4];
        let ruler_page_index = self.navigation_current_page();
        for index in 0..layout.pages.len() {
            ui.next_size(width.max(available), height);
            ui.box_begin(0, BOX_H, 0);
            ui.spacer(-1);
            let space = ui.last_rect();
            let row = ui.box_rect();
            doc_view::page_backdrop(ui, [space[0] + space[2], row[1], ui.dp(width), ui.dp(height)]);
            ui.next_size(width, height);
            ui.next_flags(CUR_IBEAM << HB_CURSOR_SHIFT);
            let input = ui.canvas(
                0x300000 + index as u32,
                Some(self.preview.draw(index)),
                0,
                revision as u32,
            );
            let rect = input.rect;
            if index == ruler_page_index || ruler_page[2] == 0 { ruler_page = rect; }
            if input.ev & EV_ACT != 0 && rect[2] > 0 {
                let x = (i64::from(input.x) * i64::from(layout.settings.width) / i64::from(rect[2]))
                    as i32;
                let y = (i64::from(input.y) * i64::from(layout.settings.width) / i64::from(rect[2]))
                    as i32;
                let paragraph = layout.paragraph_at(index, y).or_else(|| {
                    layout.pages[index]
                        .lines
                        .iter()
                        .min_by_key(|line| (line.top - y).abs())
                        .map(|line| line.paragraph)
                });
                if let Some(id) = paragraph {
                    let caret = self.editor.document().paragraph(id).and_then(|p| {
                        if self.editor.document().note_for_paragraph(id).is_none() {
                            clicked_caret(ui, &layout, p, index, x, y, &fonts)
                        } else {
                            None
                        }
                    });
                    clicked = Some((id, caret));
                }
            }
            ui.spacer(-1);
            if let Some(area) = area.as_ref().filter(|a| direct && a.page == index) {
                let scale = |twips: i32| -> i32 {
                    (i64::from(twips) * i64::from(rect[2]) / i64::from(layout.settings.width))
                        as i32
                };
                let left = scale(layout.settings.left as i32 + self.editor.document().resolve_paragraph(&selected).left as i32);
                let text_width = scale(
                    layout.settings.content_width() as i32
                        - self.editor.document().resolve_paragraph(&selected).left as i32
                        - self.editor.document().resolve_paragraph(&selected).right as i32,
                );
                let y = rect[1] + scale(area.top);
                // Keep one extra pixel for the caret and rounding. The page renderer remains
                // the layout authority; this editor never changes the stored page settings.
                let text_height = scale(area.height).max(1) + 2;
                if self.jump_to_selected {
                    reveal_on_paper(ui, viewport, y, text_height);
                    self.jump_to_selected = false;
                }
                let inline = selected.content.first();
                let format = self.editor.document().resolve_paragraph_text(&selected, inline
                    .map_or_else(TextFormat::default, Inline::format));
                let size = (i64::from(format.size_half_points) * 10 * 256 * i64::from(rect[2])
                    / i64::from(layout.settings.width)) as i32;
                let old = ui.text.get_font();
                ui.text.set_font(fonts.get(format), size);
                let baseline = if area.baseline == 0 {
                    ui.text.ascent()
                } else {
                    scale(area.baseline)
                };
                ui.text.set_font(old.0, old.1);
                ui.next_rect_px(rect[0] + left, y, text_width, text_height);
                let mut value = inline.and_then(Inline::text).unwrap_or("").into();
                let widget = self.text_id(selected.id, 0);
                let changed = ui.textbox_presented(
                    widget,
                    None,
                    &mut value,
                    (2 << 20) | TB_WRAP,
                    TextboxPresentation {
                        font: fonts.get(format),
                        size,
                        line_height: scale(area.line_height).max(1),
                        baseline,
                        color: format.color,
                        background: 0xffffffff,
                        underline: format.underline,
                    },
                );
                self.paper_text_result(ui, selected.id, 0, widget, value, changed);
            } else if self.jump_to_selected {
                if let Some(line) = layout.pages[index]
                    .lines
                    .iter()
                    .find(|line| line.paragraph == selected.id)
                {
                    let y = rect[1]
                        + (i64::from(line.top) * i64::from(rect[2])
                            / i64::from(layout.settings.width)) as i32;
                    let line_height = (i64::from(line.height) * i64::from(rect[2])
                        / i64::from(layout.settings.width))
                        as i32;
                    reveal_on_paper(ui, viewport, y, line_height);
                    self.jump_to_selected = false;
                }
            }
            ui.box_end();
            ui.spacer(doc_view::PAGE_GAP);
        }
        self.paper_viewport_end(ui, paper_frame, ruler_page, layout.settings);
        ui.box_end();
        // First layout can have a provisional viewport when an initial focus is followed.
        // Reset after the scroll view has observed that focus, preserving the page margin.
        if self.reset_paper_view {
            ui.scroll_to(110, 0, 0);
            self.reset_paper_view = false;
        }
        if let Some((id, caret)) = clicked {
            if self.select_paragraph(id) {
                if let Some(inline) = self.editor.document().paragraph(id).and_then(|p| {
                    p.content
                        .iter()
                        .position(|inline| inline.text().is_some())
                        .or(Some(0))
                }) {
                    let widget = self.text_id(id, inline);
                    ui.focus(widget);
                    let caret = caret.unwrap_or_else(|| {
                        self.editor
                            .document()
                            .paragraph(id)
                            .and_then(|p| p.content.get(inline))
                            .and_then(Inline::text)
                            .map_or(0, str::len)
                    });
                    self.formatting.pending_selection = Some((id, inline, caret, caret));
                }
            }
        }
    }

    fn paper_text_result(
        &mut self,
        ui: &mut Ui,
        paragraph: u64,
        inline: usize,
        widget: u32,
        text: String,
        events: u32,
    ) {
        if events & EV_ACT != 0 {
            self.apply(Edit::SetText {
                paragraph,
                inline,
                text: text.clone(),
            });
        }
        if let Some((id, at, start, end)) = self.formatting.pending_selection {
            if id == paragraph && at == inline && ui.textbox_select(widget, &text, start, end) {
                self.formatting.pending_selection = None;
            }
        }
        if let Some((start, end)) = ui.textbox_selection(widget) {
            self.formatting.selection = Some((paragraph, inline, start, end));
        }
    }

    fn paper_structure(&mut self, ui: &mut Ui, paragraph: &Paragraph) {
        ui.next_size(-1, 190);
        ui.scroll_begin(137, SC_V | BOX_V | BOX_CARD | pad(12));
        ui.text(
            TS_CAPTION | TX_SECONDARY,
            "Absatz bearbeiten · Formatierte Ansicht auf der Seite",
        );
        let mut content = paragraph.content.clone();
        if content.is_empty() {
            content.push(Inline::Text(String::new()));
        }
        for (inline_index, inline) in content.iter().enumerate() {
            let widget = self.text_id(paragraph.id, inline_index);
            match inline {
                Inline::Text(text) | Inline::StyledText { text, .. } => {
                    if let Inline::StyledText { format, .. } = inline {
                        let f = self.editor.document().resolve_paragraph_text(paragraph, *format);
                        ui.text(
                            TS_CAPTION | TX_SECONDARY,
                            &format!(
                                "Textsegment · {} pt{}{}{}",
                                f.size_half_points as f32 / 2.0,
                                if f.bold { " · Fett" } else { "" },
                                if f.italic { " · Kursiv" } else { "" },
                                if f.underline { " · Unterstrichen" } else { "" }
                            ),
                        );
                    }
                    let mut text = text.clone();
                    ui.next_size(
                        -1,
                        if paragraph.content.len() <= 1 {
                            110
                        } else {
                            42
                        },
                    );
                    let events = ui.textbox(
                        widget,
                        Some("Text schreiben …"),
                        &mut text,
                        (2 << 20) | TB_MULTI | TB_WRAP,
                    );
                    self.paper_text_result(ui, paragraph.id, inline_index, widget, text, events);
                }
                Inline::Citation { .. } | Inline::CitationCluster(_) => {
                    self.citation_card(ui, paragraph.id, inline_index, widget, inline)
                }
                Inline::NoteReference(id) => self.note_card(ui, *id, widget),
                Inline::Formula(f)=>{
                    ui.text(TS_BODY,&self.editor.document().evaluate_formula(paragraph.id,f).text);
                    if ui.button(widget+4,"Formel bearbeiten")&EV_ACT!=0{self.open_formula(Some((paragraph.id,inline_index)));}
                },
                Inline::Bibliography => {
                    ui.text(TS_BODY_STRONG, "Literaturverzeichnis");
                    ui.text(
                        TS_CAPTION | TX_SECONDARY,
                        "Automatisch aus den zitierten Quellen",
                    );
                }
            }
        }
        ui.scroll_end();
    }
}
