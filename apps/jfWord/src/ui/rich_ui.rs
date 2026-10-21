// Continuous input over doc_layout's actual fragments. No editable run widgets or reflow.
use crate::{Word, preview::Fonts};
use alloc::{string::String, vec::Vec};
use doc_core::{Edit, TextFormat};
use ui::*;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Selection {
    paragraph: u64,
    anchor: usize,
    caret: usize,
}
struct Change {
    before: u64,
    after: u64,
    from: Selection,
    to: Selection,
}
#[derive(Default)]
pub(crate) struct RichEditing {
    selection: Option<Selection>,
    table_instance: Option<(u64, usize)>,
    pending: Option<Selection>,
    history: Vec<Change>,
    revealed: Option<Selection>,
    geometry: Option<(i32, i32, i32)>,
    active_paragraph: Option<u64>,
    focus_next_frame: bool,
    format_painter: Option<TextFormat>,
}

fn surface_line(
    ui: &mut Ui,
    line: &doc_layout::Line,
    rect: [i32; 4],
    page: doc_core::PageSettings,
    text: &str,
    fonts: &Fonts,
    cell: Option<&doc_layout::CellBox>,
    bounds:[i32;4],
) -> TextSurfaceLine {
    let scale = |v: i32| (i64::from(v) * i64::from(rect[2]) / i64::from(page.width)) as i32;
    let mut carets = Vec::new();
    let old = ui.text.get_font();
    for fragment in &line.fragments {
        let Some((a, b)) = fragment.source else {
            continue;
        };
        ui.text.set_font(
            fonts.get(fragment.format),
            (i64::from(fragment.format.display_size_half_points()) * 10 * 256 * i64::from(rect[2])
                / i64::from(page.width)) as i32,
        );
        let x = rect[0] + scale(fragment.x);
        carets.push((a, x));
        for (at, _) in fragment.text.char_indices().skip(1) {
            carets.push((a + at, x + ui.text.caret_x(fragment.text.as_bytes(), at)));
        }
        carets.push((b, x + scale(fragment.width)));
    }
    ui.text.set_font(old.0, old.1);
    let fallback = carets.first().map_or(rect[0] + scale(line.x), |(_, x)| *x);
    // Tabs, empty lines and trailing wrap spaces still have logical caret positions.
    let mut all = Vec::new();
    let mut x = fallback;
    let mut stops = carets.iter().peekable();
    let slice = text.get(line.start..line.end).unwrap_or("");
    for at in slice
        .char_indices()
        .map(|(at, _)| line.start + at)
        .chain(core::iter::once(line.end))
    {
        while stops.peek().is_some_and(|(offset, _)| *offset <= at) {
            x = stops.next().unwrap().1;
        }
        all.push((at, x));
    }
    let left = rect[0] + scale(bounds[0]);
    let right = rect[0] + scale(bounds[0]+bounds[2]);
    let (left,right)=cell.map_or((left,right),|c|(rect[0]+scale(c.rect[0])+ui.dp(4),rect[0]+scale(c.rect[0]+c.rect[2])-ui.dp(4)));
    TextSurfaceLine {
        start: line.start,
        end: line.end,
        rect: [
            left,
            rect[1] + scale(line.top),
            (right - left).max(1),
            scale(line.height).max(1),
        ],
        carets: all,
    }
}
fn hit(line: &TextSurfaceLine, x: i32) -> usize {
    line.carets
        .iter()
        .min_by_key(|(_, px)| (i64::from(*px) - i64::from(x)).abs())
        .map_or(line.start, |(at, _)| *at)
}
fn reveal(ui: &mut Ui, viewport: [i32; 4], line: &TextSurfaceLine) {
    let margin = ui.dp(24);
    let [_, y, _, h] = line.rect;
    let delta = if y < viewport[1] + margin {
        y - viewport[1] - margin
    } else if y + h > viewport[1] + viewport[3] - margin {
        y + h - viewport[1] - viewport[3] + margin
    } else {
        return;
    };
    let current = ui.scroll_pos(110).1;
    ui.scroll_to(110, -1, (current + ui.px2dip(delta)).max(0));
}
impl Word {
    pub(crate) fn current_character_common(&self) -> TextFormat {
        if !self.structured_edit {
            if let Some(s) = self.rich.selection {
                if let Ok(f) = self.editor.document().story_character_common(s.paragraph, s.anchor.min(s.caret), s.anchor.max(s.caret)) { return f; }
            }
        }
        self.current_character_format().explicit()
    }
    pub(crate) fn current_character_format(&self) -> doc_core::ResolvedFormat {
        let d = self.editor.document();
        if !self.structured_edit {
            if let Some(selection) = self.rich.selection {
                if let Ok(f) = d.story_character_format(selection.paragraph, selection.anchor.min(selection.caret), selection.anchor.max(selection.caret)) {
                    return f.resolve(doc_core::ParagraphStyle::Normal);
                }
            }
        }
        let p = self.formatting_paragraph();
        let inline = self.formatting.selection.filter(|(id, _, _, _)| *id == p.id).map_or(0, |(_, i, _, _)| i);
        d.resolve_paragraph_text(p, p.content.get(inline).map(|n| n.format()).unwrap_or_default())
    }


    pub(crate) fn rich_format_available(&self, command: u32) -> bool {
        !self.shell.home && !self.preserved() && !self.preview_mode && !self.structured_edit
            && (command != crate::FORMAT_PASTE || self.rich.format_painter.is_some())
    }

    pub(crate) fn rich_format_transfer(&mut self, ui: &mut Ui, command: u32) {
        if !self.rich_format_available(command) { return; }
        let Ok(story) = self.editor.document().story_projection(self.selected_id()) else { return; };
        let paragraph = story.paragraphs[0].projection.paragraph;
        let at = story.offset(self.selected_id(), 0).unwrap_or(0);
        let selection = self.rich.selection.filter(|s| s.paragraph == paragraph)
            .unwrap_or(Selection { paragraph, anchor: at, caret: at });
        let start = selection.anchor.min(selection.caret);
        let end = selection.anchor.max(selection.caret);
        if command == crate::FORMAT_COPY {
            match self.editor.document().story_character_format(paragraph, start, end) {
                Ok(format) => {
                    self.rich.format_painter = Some(format);
                    self.message = String::from("Zeichenformat aufgenommen · Zieltext auswählen und Format anwenden");
                }
                Err(_) => self.message = String::from("Zeichenformat kann nur aus bearbeitbarem Text aufgenommen werden."),
            }
        } else {
            let format = if command == crate::FORMAT_RESET { TextFormat::default() }
                else if let Some(format) = self.rich.format_painter { format } else { return; };
            let before = self.editor.state_id();
            if self.apply_page_edit(ui,Edit::ReplaceStoryFormat { paragraph, start, end, format }).is_some() {
                self.rich_record(before, selection, selection);
                self.message = String::from(if command == crate::FORMAT_RESET {
                    "Zeichenformatierung gelöscht · Absatzvorlage bleibt erhalten"
                } else { "Zeichenformat übertragen" });
            }
        }
        self.rich_focus(ui, selection);
        ui.invalidate();
    }

    pub(crate) fn rich_header_instance(&self) -> Option<(u64,usize)> {
        let (cell,page)=self.rich.table_instance?;
        let d=self.editor.document();let (table,row,_)=d.table_location(self.selected_id())?;
        if row>=usize::from(d.table(table)?.header_rows) || d.cell_for_paragraph(self.selected_id())?.id!=cell {return None;}
        self.preview.layout()?.pages.get(page)?.cells.iter().any(|c|c.cell==cell).then_some((cell,page))
    }
    pub(crate) fn rich_select(&mut self, ui: &mut Ui, paragraph: u64, start: usize, end: usize) -> Result<(), doc_core::Error> {
        let story = self.editor.document().story_projection(paragraph)?;
        self.rich.table_instance=None;
        story.position(start)?;
        story.position(end)?;
        let paragraph = story.paragraphs[0].projection.paragraph;
        self.shell.home = false;
        self.preview_mode = false;
        self.structured_edit = false;
        self.rich_focus(ui, Selection { paragraph, anchor: start, caret: end });
        Ok(())
    }
    pub(crate) fn rich_request_focus(&mut self) {
        self.rich.focus_next_frame = true;
    }
    pub(crate) fn rich_focus_pending(&self) -> bool {
        self.rich.focus_next_frame
    }
    pub(crate) fn rich_finish_focus(&mut self, ui: &mut Ui) {
        if self.rich.focus_next_frame {
            self.rich.focus_next_frame = false;
            if !self.preserved() && !self.preview_mode {
                let widget = self.text_id(self.selected_id(), 0);
                ui.focus(widget);
            }
        }
    }
    fn rich_insert_fragment(
        &mut self,
        ui: &mut Ui,
        from: Selection,
        fragment: doc_core::DocumentFragment,
    ) -> bool {
        let before = self.editor.state_id();
        let checked=self.editor.document().tables.iter().chain(&fragment.document.tables).any(|t|t.position.is_some());
        let result=if checked {
            match self.preview.prepare_fonts(&mut ui.text) {
                Ok(fonts)=>doc_view::replace_fragment_paginated(&mut self.editor,from.paragraph,from.anchor.min(from.caret),from.anchor.max(from.caret),fragment,&mut ui.text,&fonts),
                Err(_)=>Err(doc_layout::Error::Metrics),
            }
        }else{self.editor.replace_story_fragment(
            from.paragraph,
            from.anchor.min(from.caret),
            from.anchor.max(from.caret),
            fragment,
        ).map_err(doc_layout::Error::Model)};
        match result {
            Ok((paragraph, local)) => {
                let story = self.editor.document().story_projection(paragraph).unwrap();
                let caret = story.offset(paragraph, local).unwrap();
                let to = Selection {
                    paragraph: story.paragraphs[0].projection.paragraph,
                    anchor: caret,
                    caret,
                };
                self.rich_record(before, from, to);
                self.rich_focus(ui, to);
                self.message = String::from("Formatierte Inhalte übernommen");
                true
            }
            Err(error) => {
                self.rich_focus(ui, from);
                self.message = alloc::format!(
                    "Einfügen nicht möglich ({error:?}). Felder vollständig auswählen. Tabellen müssen in die umgebende Seite oder Zelle passen."
                );
                false
            }
        }
    }
    pub(crate) fn rich_clipboard(&mut self, ui: &mut Ui, command: u32) -> bool {
        if self.structured_edit || self.preview_mode {
            return false;
        }
        let Ok(story) = self.editor.document().story_projection(self.selected_id()) else {
            return false;
        };
        let paragraph = story.paragraphs[0].projection.paragraph;
        let Some(from) = self.rich.selection.filter(|s| s.paragraph == paragraph) else {
            return false;
        };
        let start = from.anchor.min(from.caret);
        let end = from.anchor.max(from.caret);
        let Some(text) = story.text.get(start..end) else {
            return false;
        };
        match command {
            70 | 71 => {
                if start == end {
                    return false;
                }
                let fragment = match self
                    .editor
                    .document()
                    .copy_story_range(paragraph, start, end)
                {
                    Ok(fragment) => fragment,
                    Err(doc_core::Error::InUse) if command == 70 => {
                        if ui.clip_set(CLIP_TEXT, text.as_bytes()) == 0 {
                            self.message = String::from(
                                "Teil eines Feldes als unformatierten Text kopiert. Für verknüpfte Felder das ganze Feld auswählen.",
                            );
                            return true;
                        } else {
                            self.message =
                                String::from("Die Zwischenablage ist gerade nicht verfügbar.");
                        }
                        return false;
                    }
                    Err(_) => {
                        self.message = String::from(
                            "Zum Ausschneiden Felder vollständig auswählen. Das Dokument bleibt unverändert.",
                        );
                        return false;
                    }
                };
                let encoded = doc_io::encode_document_fragment(&fragment);
                let display = doc_io::clipboard_display_fragment(&fragment, self.editor.document());
                let exchange = display.and_then(|document| {
                    Ok((
                        doc_io::clipboard_rtf_encode(&document)?,
                        doc_io::clipboard_html_encode(&document)?,
                    ))
                });
                let (Ok(encoded), Ok((rtf, html))) = (encoded, exchange) else {
                    self.message = String::from(
                        "Die Auswahl kann nicht vollständig in die Zwischenablage übertragen werden.",
                    );
                    return false;
                };
                if ui.clip_set_many(&[
                    (CLIP_TEXT, text.as_bytes()),
                    (CLIP_DOCX, &encoded),
                    (CLIP_RTF, &rtf),
                    (CLIP_HTML, &html),
                ]) != 0
                {
                    self.message = String::from(
                        "Die Zwischenablage ist gerade nicht verfügbar. Das Dokument bleibt unverändert.",
                    );
                    return false;
                }
                if command == 71 {
                    if self.rich_insert_fragment(ui, from, doc_core::DocumentFragment::default()) {
                        self.message = String::from(
                            "Formatiert ausgeschnitten · Quellen und Anmerkungen werden mitkopiert",
                        );
                    } else {
                        return false;
                    }
                } else {
                    self.message = String::from(
                        "Formatiert kopiert · RTF/HTML für andere Programme, verknüpfte Felder für jfWord",
                    );
                    self.rich_focus(ui, from);
                }
                true
            }
            72 | 73 => {
                // Explicit plain-text paste is always available; unsupported rich data is
                // never silently flattened by the normal paste command.
                let rich = if command == 73 {
                    None
                } else if ui.clip_has(CLIP_DOCX) {
                    Some(
                        ui.clip_get(CLIP_DOCX)
                            .ok_or(doc_io::Error::Io)
                            .and_then(|bytes| doc_io::decode_document_fragment(&bytes)),
                    )
                } else if ui.clip_has(CLIP_RTF) {
                    Some(
                        ui.clip_get(CLIP_RTF)
                            .ok_or(doc_io::Error::Io)
                            .and_then(|bytes| doc_io::clipboard_rtf_decode(&bytes))
                            .map(|document| doc_core::DocumentFragment { document }),
                    )
                } else if ui.clip_has(CLIP_HTML) {
                    Some(ui.clip_get(CLIP_HTML).ok_or(doc_io::Error::Io).and_then(|bytes|doc_io::clipboard_html_decode(&bytes)).map(|document|doc_core::DocumentFragment{document}))
                } else {
                    None
                };
                if let Some(result) = rich {
                    match result {
                        Ok(fragment) => {
                            self.rich_insert_fragment(ui, from, fragment)
                        }
                        Err(error) => {
                            self.message = alloc::format!(
                                "Rich-Text-Inhalt noch nicht unterstützt ({error:?}). Strg+Umschalt+V fügt nur den Text ein."
                            );
                            self.rich_focus(ui, from);
                            false
                        }
                    }
                } else if let Some(bytes) = ui.clip_get(CLIP_TEXT) {
                    let Ok(text) = String::from_utf8(bytes) else {
                        self.message = String::from("Ungültiger UTF-8-Text in der Zwischenablage.");
                        return false;
                    };
                    let text = text.replace("\r\n", "\n").replace('\r', "\n");
                    let length = text.len();
                    let before = self.editor.state_id();
                    if self.rich_apply(ui,Edit::ReplaceStoryText {
                        paragraph,
                        start,
                        end,
                        text,
                        paragraph_breaks: true,
                    }) {
                        let to = Selection {
                            paragraph,
                            anchor: start + length,
                            caret: start + length,
                        };
                        self.rich_record(before, from, to);
                        self.rich_focus(ui, to);
                        true
                    } else {
                        self.rich_focus(ui, from);
                        false
                    }
                } else {
                    false
                }
            }
            _ => false,
        }
    }
    pub(crate) fn rich_field_target(&self) -> Option<(u64, usize)> {
        let selection = self.rich.selection?;
        let story = self
            .editor
            .document()
            .story_projection(selection.paragraph)
            .ok()?;
        let start = selection.anchor.min(selection.caret);
        let end = selection.anchor.max(selection.caret);
        for p in &story.paragraphs {
            for span in &p.projection.spans {
                let a = p.start + span.start;
                let b = p.start + span.end;
                if !span.editable
                    && ((start == end && start >= a && start < b)
                        || (start < end && start >= a && end <= b)
                        || (a == b && start == a && end == b))
                {
                    return Some((p.projection.paragraph, span.inline));
                }
            }
        }
        None
    }
    pub(crate) fn rich_field_next(&mut self, ui: &mut Ui, forward: bool) {
        let Ok(story) = self.editor.document().story_projection(self.selected_id()) else {
            return;
        };
        let selection = self
            .rich
            .selection
            .filter(|s| s.paragraph == story.paragraphs[0].projection.paragraph);
        let current = self.rich_field_target().filter(|(id, inline)| {
            selection.is_some_and(|s| s.anchor != s.caret)
                || story
                    .paragraphs
                    .iter()
                    .find(|p| p.projection.paragraph == *id)
                    .and_then(|p| p.projection.spans.iter().find(|s| s.inline == *inline))
                    .is_some_and(|s| s.start == s.end)
        });
        let mut fields = Vec::new();
        for p in &story.paragraphs {
            for span in &p.projection.spans {
                if !span.editable {
                    fields.push((p.projection.paragraph, span.inline, p.start + span.start));
                }
            }
        }
        if fields.is_empty() {
            self.message = String::from("In diesem Textbereich sind noch keine Felder vorhanden.");
            return;
        }
        let caret = selection.map_or(0, |s| s.caret);
        let target = if let Some(index) = fields.iter().position(|f| Some((f.0, f.1)) == current) {
            if forward {
                (index + 1) % fields.len()
            } else {
                (index + fields.len() - 1) % fields.len()
            }
        } else if forward {
            fields.iter().position(|f| f.2 >= caret).unwrap_or(0)
        } else {
            fields
                .iter()
                .rposition(|f| f.2 < caret)
                .unwrap_or(fields.len() - 1)
        };
        self.rich_anchor(ui, fields[target].0, fields[target].1);
    }
    pub(crate) fn rich_remove_field(&mut self, ui: &mut Ui, paragraph: u64, inline: usize) {
        self.rich_anchor(ui, paragraph, inline);
        if let Some(from) = self.rich.selection {
            let before = self.editor.state_id();
            match self.editor.remove_field(paragraph, inline) {
                Ok((id, local)) => {
                    let story = self.editor.document().story_projection(id).unwrap();
                    let caret = story.offset(id, local).unwrap();
                    let to = Selection {
                        paragraph: story.paragraphs[0].projection.paragraph,
                        anchor: caret,
                        caret,
                    };
                    self.rich_record(before, from, to);
                    self.rich_focus(ui, to);
                    self.message =
                        String::from("Feld entfernt · Quellen bleiben im Dokument erhalten");
                }
                Err(_) => self.rich_focus(ui, from),
            }
        }
    }
    fn rich_apply(&mut self, ui:&mut Ui, edit: Edit) -> bool {
        if self.editor.document().tables.iter().any(|t|t.position.is_some()) {
            return self.apply_page_edit(ui,edit).is_some();
        }
        match self.editor.apply(edit) {
            Ok(_) => {
                self.message = String::from("Änderung übernommen");
                true
            }
            Err(error) => {
                self.message = String::from(match error {
                    doc_core::Error::Limit => {
                        "Die Änderung überschreitet das Dokumentlimit. Die Auswahl bleibt erhalten."
                    }
                    doc_core::Error::InUse => {
                        "Zitate, Anmerkungszeichen und Verzeichnisse bleiben geschützte Felder. Zum Ändern die Feldfunktionen verwenden."
                    }
                    _ => {
                        "Die Auswahl enthält keine gültige bearbeitbare Textposition. Die Änderung wurde nicht übernommen."
                    }
                });
                false
            }
        }
    }
    pub(crate) fn formatting_paragraph(&self) -> &doc_core::Paragraph {
        let mut id = self.selected_id();
        if !self.structured_edit {
            if let Some(selection) = self.rich.selection {
                if let Ok(story) = self.editor.document().story_projection(id) {
                    if story.paragraphs[0].projection.paragraph == selection.paragraph {
                        if let Ok((first, _)) =
                            story.position(selection.anchor.min(selection.caret))
                        {
                            id = first;
                        }
                    }
                }
            }
        }
        self.editor.document().paragraph(id).unwrap()
    }
    pub(crate) fn ruler_context(&self) -> u64 {
        let mut key = self.document_revision().wrapping_mul(0x100000001b3) ^ self.formatting_paragraph().id;
        if let Some(s) = self.rich.selection {
            key ^= (s.anchor as u64).rotate_left(21) ^ (s.caret as u64).rotate_left(42);
        }
        key
    }
    pub(crate) fn paragraph_change(
        &mut self,
        ui: &mut Ui,
        change: doc_core::ParagraphChange,
    ) -> Option<u64> {
        let selected = self.selected_id();
        let story = self.editor.document().story_projection(selected).ok()?;
        let paragraph = story.paragraphs[0].projection.paragraph;
        let at = story.offset(selected, 0)?;
        let selection = self
            .rich
            .selection
            .filter(|s| !self.structured_edit && s.paragraph == paragraph)
            .unwrap_or(Selection {
                paragraph,
                anchor: at,
                caret: at,
            });
        let before = self.editor.state_id();
        let result = self.apply_page_edit(ui,Edit::FormatStoryParagraphs {
            paragraph,
            start: selection.anchor.min(selection.caret),
            end: selection.anchor.max(selection.caret),
            change,
        });
        if result.is_some() && !self.structured_edit {
            self.rich_record(before, selection, selection);
            self.rich_focus(ui, selection);
        }
        result
    }
    pub(crate) fn rich_local_focus(&mut self, ui: &mut Ui, paragraph: u64, anchor: usize, caret: usize) {
        let Ok(story) = self.editor.document().story_projection(paragraph) else {
            return;
        };
        if let (Some(anchor), Some(caret)) = (
            story.offset(paragraph, anchor),
            story.offset(paragraph, caret),
        ) {
            self.rich_focus(
                ui,
                Selection {
                    paragraph: story.paragraphs[0].projection.paragraph,
                    anchor,
                    caret,
                },
            );
        }
    }
    pub(crate) fn rich_anchor(&mut self, ui: &mut Ui, paragraph: u64, inline: usize) {
        if let Ok(p) = self.editor.document().text_projection(paragraph) {
            if let Some(span) = p.spans.iter().find(|s| s.inline == inline) {
                self.rich_local_focus(ui, paragraph, span.start, span.end);
            }
        }
    }
    pub(crate) fn rich_find(&mut self, ui: &mut Ui, found: &doc_core::TextMatch) {
        let Ok(p) = self.editor.document().text_projection(found.paragraph) else {
            return;
        };
        if let (Some(anchor), Some(caret)) = (
            p.offset(found.start_inline, found.start),
            p.offset(found.end_inline, found.end),
        ) {
            self.rich_local_focus(ui, found.paragraph, anchor, caret);
        }
    }
    fn rich_focus(&mut self, ui: &mut Ui, selection: Selection) {
        let Ok(story) = self.editor.document().story_projection(selection.paragraph) else {
            return;
        };
        let Ok((paragraph, _)) = story.position(selection.caret) else {
            return;
        };
        if !story.text.is_char_boundary(selection.anchor) || !self.select_paragraph(paragraph) {
            return;
        }
        self.formatting.pending_selection = None;
        self.rich.active_paragraph = Some(paragraph);
        self.rich.selection = Some(selection);
        self.rich.pending = Some(selection);
        let widget = self.text_id(selection.paragraph, 0);
        ui.focus(widget);
    }
    fn rich_record(&mut self, before: u64, from: Selection, to: Selection) {
        let after = self.editor.state_id();
        if before != after {
            self.rich.history.push(Change {
                before,
                after,
                from,
                to,
            });
            if self.rich.history.len() > 64 {
                self.rich.history.remove(0);
            }
        }
    }
    pub(crate) fn rich_caret_snapshot(&self) -> Option<[(u64, usize); 2]> {
        let selection = self.rich.selection?;
        let story = self
            .editor
            .document()
            .story_projection(selection.paragraph)
            .ok()?;
        Some([
            story.position(selection.anchor).ok()?,
            story.position(selection.caret).ok()?,
        ])
    }
    pub(crate) fn rich_restore(
        &mut self,
        ui: &mut Ui,
        from: u64,
        to: u64,
        fallback: Option<[(u64, usize); 2]>,
    ) {
        let target = self.rich.history.iter().rev().find_map(|c| {
            if c.after == from && c.before == to {
                Some(c.from)
            } else if c.before == from && c.after == to {
                Some(c.to)
            } else {
                None
            }
        });
        if let Some(target) = target {
            self.rich_focus(ui, target);
        } else if let Some([a, b]) = fallback {
            if let Ok(story) = self.editor.document().story_projection(b.0) {
                let offset = |(id, at): (u64, usize)| {
                    let p = story
                        .paragraphs
                        .iter()
                        .find(|p| p.projection.paragraph == id)?;
                    let mut at = at.min(p.projection.text.len());
                    while !p.projection.text.is_char_boundary(at) {
                        at -= 1;
                    }
                    Some(p.start + at)
                };
                if let Some(caret) = offset(b) {
                    self.rich_focus(
                        ui,
                        Selection {
                            paragraph: story.paragraphs[0].projection.paragraph,
                            anchor: offset(a).unwrap_or(caret),
                            caret,
                        },
                    );
                }
            }
        }
    }
    pub(crate) fn rich_character_properties(&mut self, ui: &mut Ui, command: u32, size: Option<u16>, family: Option<doc_core::FontFamily>) {
        let Ok(story) = self.editor.document().story_projection(self.selected_id()) else {
            return;
        };
        let paragraph = story.paragraphs[0].projection.paragraph;
        let selection = self
            .rich
            .selection
            .filter(|s| s.paragraph == paragraph)
            .unwrap_or(Selection {
                paragraph,
                anchor: 0,
                caret: 0,
            });
        let start = selection.anchor.min(selection.caret);
        let end = selection.anchor.max(selection.caret);
        let common = self.editor.document().story_character_common(paragraph, start, end).unwrap_or_default();
        let mut format = crate::editor_ui::character_patch(command, common);
        format.size_half_points = size;
        format.font_family = family;
        let before = self.editor.state_id();
        if self
            .apply_page_edit(ui,Edit::FormatStoryText {
                paragraph,
                start,
                end,
                format,
            })
            .is_some()
        {
            self.rich_record(before, selection, selection);
        }
        self.rich_focus(ui, selection);
    }
    pub(crate) fn rich_keys(&mut self, ui: &mut Ui) {
        let Ok(story) = self.editor.document().story_projection(self.selected_id()) else {
            return;
        };
        let paragraph = story.paragraphs[0].projection.paragraph;
        let widget = self.text_id(paragraph, 0);
        if ui.focused() != widget {
            return;
        }
        let Some((anchor, caret)) = ui.textbox_selection(widget) else {
            return;
        };
        if !story.text.is_char_boundary(anchor) || !story.text.is_char_boundary(caret) {
            return;
        }
        self.rich.selection = Some(Selection {
            paragraph,
            anchor,
            caret,
        });
        // Capture document-format shortcuts before the textbox owns Ctrl+C/V.
        for (key, modifiers, command) in [
            (VK_C, MOD_CTRL | MOD_SHIFT, crate::FORMAT_COPY),
            (VK_V, MOD_CTRL | MOD_SHIFT | MOD_ALT, crate::FORMAT_PASTE),
            (VK_SPACE, MOD_CTRL, crate::FORMAT_RESET),
        ] {
            if ui.capture_shortcut(key, modifiers) {
                self.rich_format_transfer(ui, command);
                return;
            }
        }
        for (key, modifiers, command) in [
            (VK_C, MOD_CTRL, 70),
            (VK_X, MOD_CTRL, 71),
            (VK_V, MOD_CTRL | MOD_SHIFT, 73),
            (VK_V, MOD_CTRL, 72),
            (VK_INSERT, MOD_CTRL, 70),
            (VK_INSERT, MOD_SHIFT, 72),
            (VK_DELETE, MOD_SHIFT, 71),
        ] {
            if ui.capture_shortcut(key, modifiers) {
                self.rich_clipboard(ui, command);
                return;
            }
        }
        if ui.capture_shortcut(VK_F11, 0) {
            self.rich_field_next(ui, true);
            return;
        }
        if ui.capture_shortcut(VK_F11, MOD_SHIFT) {
            self.rich_field_next(ui, false);
            return;
        }
        if self.active_cell.is_some() {
            if ui.capture_shortcut(VK_TAB,MOD_SHIFT) {self.table_next_cell(ui,true);return;}
            if ui.capture_shortcut(VK_TAB,0) {self.table_next_cell(ui,false);return;}
        }
        let hard = ui.capture_shortcut(VK_RETURN, 0);
        let soft = !hard && ui.capture_shortcut(VK_RETURN, MOD_SHIFT);
        let tab = !hard && !soft && (ui.capture_shortcut(VK_TAB, 0) || ui.capture_shortcut(VK_TAB, MOD_CTRL));
        if !hard && !soft && !tab {
            return;
        }
        let from = Selection {
            paragraph,
            anchor,
            caret,
        };
        let before = self.editor.state_id();
        let start = anchor.min(caret);
        if self
            .apply_page_edit(ui,Edit::ReplaceStoryText {
                paragraph,
                start,
                end: anchor.max(caret),
                text: String::from(if tab { "\t" } else { "\n" }),
                paragraph_breaks: hard,
            })
            .is_some()
        {
            let to = Selection {
                paragraph,
                anchor: start + 1,
                caret: start + 1,
            };
            self.rich_record(before, from, to);
            self.rich_focus(ui, to);
        }
    }
    pub(crate) fn table_insertion_offset(&self)->usize {
        self.rich.selection.and_then(|s|self.editor.document().story_projection(s.paragraph).ok()?.position(s.caret).ok()).filter(|(id,_)|*id==self.selected_id()).map_or(0,|(_,n)|n)
    }
    pub(crate) fn rich_paper(&mut self, ui: &mut Ui) {
        ui.next_size(-1, -1);
        // Both axes fill the workspace. Child natural widths must not add a
        // redundant settle pass when contextual table tools first appear.
        ui.box_begin(135, BOX_V | BOX_FIXED, 0);
        self.table_tools(ui);
        self.rich_paper_content(ui);
        ui.box_end();
    }
    fn rich_paper_content(&mut self, ui: &mut Ui) {
        self.preview
            .update(ui, self.editor.document(), self.editor.revision());
        let Some(layout) = self.preview.layout() else {
            ui.text(TS_BODY | TX_WRAP, self.preview.error());
            return;
        };
        let selected = self.selected_id();
        let Ok(projection) = self.editor.document().story_projection(selected) else {
            return;
        };
        let paragraph = projection.paragraphs[0].projection.paragraph;
        let widget = self.text_id(paragraph, 0);
        // Native citation/note controls address inline coordinates; translate once into
        // the complete story. Navigation selects its paragraph without splitting the buffer.
        if let Some((id, inline, a, b)) = self.formatting.pending_selection.take() {
            if let Some(p) = projection
                .paragraphs
                .iter()
                .find(|p| p.projection.paragraph == id)
            {
                if let (Some(a), Some(b)) = (
                    p.projection.offset(inline, a),
                    p.projection.offset(inline, b),
                ) {
                    self.rich.pending = Some(Selection {
                        paragraph,
                        anchor: p.start + a,
                        caret: p.start + b,
                    });
                    ui.focus(widget);
                }
            }
        } else if self.rich.active_paragraph != Some(selected) {
            let at = projection.offset(selected, 0).unwrap_or(0);
            self.rich.pending = Some(Selection {
                paragraph,
                anchor: at,
                caret: at,
            });
        }
        self.rich.active_paragraph = Some(selected);
        if self.active_note.is_some() {
            ui.next_size(-1, 100);
            ui.box_begin(136, BOX_V | pad(10), 3);
            self.note_editor_header(ui);
            ui.box_end();
        }
        let paper_frame = crate::paper_ui::paper_viewport_begin(ui);
        let viewport = ui.last_rect();
        let available = ui.px2dip(viewport[2]).saturating_sub(doc_view::PAGE_GAP * 2).max(280);
        let width = self.preview.page_width(layout.settings.width, available);
        let geometry = (width, viewport[2], viewport[3]);
        if self.rich.geometry != Some(geometry) {
            self.rich.revealed = None;
            self.rich.geometry = Some(geometry);
        }
        let height = (i64::from(layout.settings.height) * i64::from(width)
            / i64::from(layout.settings.width)) as i32;
        let Some(fonts) = self.preview.fonts() else { return; };
        let mut lines = Vec::new();
        let header_instance=self.rich_header_instance();
        let mut clicked = None;
        let mut table_change=None;
        let mut ruler_page = [0; 4];
        let ruler_page_index = self.navigation_current_page();
        for (index, page) in layout.pages.iter().enumerate() {
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
                self.editor.revision() as u32,
            );
            let input=input.clone();
            let resizing=self.tables_ui.controller.active();
            let change=self.tables_ui.controller.update(self.editor.document(),page,index,&input,self.editor.revision());
            let resizing=resizing || self.tables_ui.controller.active() || change.is_some();
            if change.is_some(){table_change=change;}
            self.tables_ui.controller.draw_handles(ui,self.editor.document(),page,input.rect);
            self.tables_ui.controller.draw_guide(ui,index,input.rect,layout.settings.width);
            let (rect, ev, mx, my) = (input.rect, input.ev, input.x, input.y);
            if index == ruler_page_index || ruler_page[2] == 0 { ruler_page = rect; }
            for line in &page.lines {
                if let Some(cell)=page.cells.iter().find(|c|line.cell==Some(c.cell)) {
                    if let Some((_,chosen))=header_instance.filter(|(id,_)|*id==cell.cell) {if index!=chosen {continue;}}
                    else if cell.repeated {continue;}
                }
                if let Some(p) = projection
                    .paragraphs
                    .iter()
                    .find(|p| p.projection.paragraph == line.paragraph)
                {
                    let mut geometry =
                        surface_line(ui, line, rect, layout.settings, &p.projection.text, &fonts, page.cells.iter().find(|c|line.cell==Some(c.cell)),doc_view::text_line_bounds(self.editor.document(),page,line));
                    geometry.start += p.start;
                    geometry.end += p.start;
                    for (at, _) in &mut geometry.carets {
                        *at += p.start;
                    }
                    lines.push(geometry);
                }
            }
            if !resizing && ev & EV_ACT != 0 && rect[2] > 0 {
                let y =
                    (i64::from(my) * i64::from(layout.settings.width) / i64::from(rect[2])) as i32;
                let x=(i64::from(mx)*i64::from(layout.settings.width)/i64::from(rect[2])) as i32;
                let cell=doc_view::table_cell_at(page,x,y);
                if let Some(line) = page.lines.iter().filter(|l|l.cell==cell.map(|c|c.cell)).min_by_key(|l| {
                    if y < l.top {
                        l.top - y
                    } else {
                        (y - l.top - l.height).max(0)
                    }
                }) {
                    if let Ok(p) = self.editor.document().text_projection(line.paragraph) {
                        let line = surface_line(ui, line, rect, layout.settings, &p.text, &fonts, cell,doc_view::text_line_bounds(self.editor.document(),page,line));
                        let caret = hit(&line, rect[0] + mx);
                        clicked = Some((p.paragraph, caret,cell.filter(|c|self.editor.document().table(c.table).is_some_and(|t|c.row<usize::from(t.header_rows))).map(|c|(c.cell,index))));
                    }
                } else if let Some(cell)=cell {
                    // Even a fixed row shorter than its padding remains selectable;
                    // its properties and resize handles must reveal the hidden story.
                    clicked=Some((cell.paragraph,0,if self.editor.document().table(cell.table).is_some_and(|t|cell.row<usize::from(t.header_rows)){Some((cell.cell,index))}else{None}));
                }
            }
            ui.spacer(-1);
            ui.box_end();
            ui.spacer(doc_view::PAGE_GAP);
        }
        // One surface spans every paragraph in this story, including page boundaries.
        // Other stories on the page (notes) keep their own editable surface and history.
        if !lines.is_empty() {
            lines.sort_by_key(|l| l.start);
            let before = self.editor.state_id();
            let (a, b) = ui.textbox_selection(widget).unwrap_or((0, 0));
            let from = Selection {
                paragraph,
                anchor: a,
                caret: b,
            };
            let mut value = projection.text.clone();
            ui.textbox_surface(widget, &mut value, ((4 << 20) + 1) | TB_WRAP, &lines);
            if let Some((start, end, text)) = ui.textbox_surface_change() {
                let (anchor, caret) = ui.textbox_selection(widget).unwrap_or((start, start));
                let to = Selection {
                    paragraph,
                    anchor,
                    caret,
                };
                if self.rich_apply(ui,Edit::ReplaceStoryText {
                    paragraph,
                    start,
                    end,
                    text,
                    paragraph_breaks: true,
                }) {
                    self.rich_record(before, from, to);
                } else {
                    self.rich.pending = Some(from);
                }
            }
            let current = self.editor.document().story_projection(paragraph).unwrap();
            if let Some(pending) = self.rich.pending.filter(|s| s.paragraph == paragraph) {
                if ui.textbox_select(widget, &current.text, pending.anchor, pending.caret) {
                    self.rich.pending = None;
                }
            }
            if let Some((anchor, caret)) = ui.textbox_selection(widget) {
                let previous_format = self.current_character_common();
                let selection = Selection {
                    paragraph,
                    anchor,
                    caret,
                };
                self.rich.selection = Some(selection);
                if let Ok((id, local)) = current.position(caret) {
                    if id != self.selected_id() {
                        self.select_paragraph(id);
                    }
                    self.rich.active_paragraph = Some(id);
                    let p = current
                        .paragraphs
                        .iter()
                        .find(|p| p.projection.paragraph == id)
                        .unwrap();
                    self.formatting.selection = p.projection.position(local).ok().map(|at| {
                        let anchor = current
                            .position(anchor)
                            .ok()
                            .filter(|(other, _)| *other == id)
                            .and_then(|(_, off)| p.projection.position(off).ok())
                            .filter(|a| a.inline == at.inline)
                            .map_or(at.offset, |a| a.offset);
                        (id, at.inline, anchor, at.offset)
                    });
                }
                if self.jump_to_selected || self.rich.revealed != Some(selection) {
                    if let Some(line) = lines.iter().rev().find(|l| l.start <= caret) {
                        reveal(ui, viewport, line);
                    }
                    self.jump_to_selected = false;
                    self.rich.revealed = Some(selection);
                }
                // Only changed properties need a ribbon pass. Field/caret navigation
                // with identical typography must not consume extra layout-settle passes.
                if self.current_character_common() != previous_format { ui.repass(); }
            }
        }
        self.paper_viewport_end(ui, paper_frame, ruler_page, layout.settings);
        if self.reset_paper_view {
            ui.scroll_to(110, 0, 0);
            self.reset_paper_view = false;
        }
        if let Some((table,change))=table_change {self.apply_page_edit(ui,Edit::ChangeTable{table,change});ui.repass();}
        if let Some((paragraph, caret,instance)) = clicked {
            let changed_instance=self.rich.table_instance!=instance;
            self.rich.table_instance=instance;
            if changed_instance {self.rich.revealed=None;ui.repass();}
            if changed_instance || ui.focused() != widget
                || !projection
                    .paragraphs
                    .iter()
                    .any(|p| p.projection.paragraph == paragraph)
            {
                self.rich_local_focus(ui, paragraph, caret, caret);
            }
        }
    }
}
