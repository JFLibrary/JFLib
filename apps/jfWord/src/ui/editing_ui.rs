// Paragraph-boundary editing and caret restoration. Structural operations live in doc_core.
use crate::Word;
use alloc::vec::Vec;
use doc_core::{Edit, Inline, TextPosition};
use ui::*;

struct FocusChange {
    before: u64,
    after: u64,
    from: TextPosition,
    anchor: usize,
    to: TextPosition,
}
#[derive(Default)]
pub(crate) struct Editing {
    history: Vec<FocusChange>,
}
impl Editing {
    pub(crate) fn history_focus(&self, from: u64, to: u64) -> Option<(TextPosition, usize)> {
        self.history.iter().rev().find_map(|change| {
            if from == change.after && to == change.before {
                Some((change.from, change.anchor))
            } else if from == change.before && to == change.after {
                Some((change.to, change.to.offset))
            } else {
                None
            }
        })
    }
}
impl Word {
    fn focus_text(&mut self, ui: &mut Ui, at: TextPosition, anchor: usize) {
        if !self.select_paragraph(at.paragraph) {
            return;
        }
        let text = self
            .editor
            .document()
            .paragraph(at.paragraph)
            .and_then(|p| p.content.get(at.inline))
            .and_then(Inline::text)
            .unwrap_or("");
        if !text.is_char_boundary(at.offset) || !text.is_char_boundary(anchor) {
            return;
        }
        self.widget_ids.clear();
        self.formatting.selection = Some((at.paragraph, at.inline, anchor, at.offset));
        self.formatting.pending_selection = self.formatting.selection;
        let widget = self.text_id(at.paragraph, at.inline);
        ui.focus(widget);
    }
    pub(crate) fn restore_edit_focus(&mut self, ui: &mut Ui, from: u64, to: u64) {
        if let Some((at, anchor)) = self.editing.history_focus(from, to) {
            self.focus_text(ui, at, anchor);
        }
    }
    fn record_edit_focus(
        &mut self,
        ui: &mut Ui,
        before: u64,
        from: TextPosition,
        anchor: usize,
        to: TextPosition,
    ) {
        let after = self.editor.state_id();
        if before == after {
            return;
        }
        self.editing.history.push(FocusChange {
            before,
            after,
            from,
            anchor,
            to,
        });
        if self.editing.history.len() > 64 {
            self.editing.history.remove(0);
        }
        self.focus_text(ui, to, to.offset);
    }
    pub(crate) fn paragraph_keys(&mut self, ui: &mut Ui) {
        let Some((paragraph, inline, widget)) = self
            .widget_ids
            .iter()
            .find(|(_, _, id)| *id == ui.focused())
            .copied()
        else {
            return;
        };
        let Some((anchor, caret)) = ui.textbox_selection(widget) else {
            return;
        };
        let Some(p) = self.editor.document().paragraph(paragraph) else {
            return;
        };
        let text = if p.content.is_empty() && inline == 0 {
            ""
        } else if let Some(text) = p.content.get(inline).and_then(Inline::text) {
            text
        } else {
            return;
        };
        if !text.is_char_boundary(anchor) || !text.is_char_boundary(caret) {
            return;
        }
        let from = TextPosition {
            paragraph,
            inline,
            offset: caret,
        };
        let before = self.editor.state_id();
        if ui.capture_shortcut(VK_RETURN, 0) {
            if let Some(created) = self.apply(Edit::SplitParagraph {
                paragraph,
                inline,
                start: anchor.min(caret),
                end: anchor.max(caret),
            }) {
                self.record_edit_focus(
                    ui,
                    before,
                    from,
                    anchor,
                    TextPosition {
                        paragraph: created,
                        inline: 0,
                        offset: 0,
                    },
                );
            }
            return;
        }
        if anchor != caret {
            return;
        }
        let previous = inline == 0 && caret == 0;
        let next = (p.content.is_empty() || inline + 1 == p.content.len()) && caret == text.len();
        let back = previous && ui.capture_shortcut(VK_BACK, 0);
        let forward = next && ui.capture_shortcut(VK_DELETE, 0);
        if !back && !forward {
            return;
        }
        let group = self
            .editor
            .document()
            .note_for_paragraph(paragraph)
            .map(|note| note.paragraphs.as_slice())
            .unwrap_or(&self.editor.document().paragraphs);
        let Some(index) = group.iter().position(|p| p.id == paragraph) else {
            return;
        };
        let pair = if back {
            index.checked_sub(1).map(|i| (group[i].id, paragraph))
        } else {
            group.get(index + 1).map(|p| (paragraph, p.id))
        };
        let Some((first, second)) = pair else {
            return;
        };
        let Ok(to) = self.editor.document().join_boundary(first, second) else {
            return;
        };
        if self.apply(Edit::JoinParagraphs { first, second }).is_some() {
            self.record_edit_focus(ui, before, from, anchor, to);
        }
    }
}
