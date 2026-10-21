// Paragraph-wide UTF-8 coordinates for continuous editing across formatting runs.
use crate::{Document, Error, Id, Inline, TextFormat, TextMatch, TextPosition};
use alloc::{string::String, vec::Vec};

/// A run's range in a paragraph's displayed text; generated fields are read-only ranges.
#[derive(Clone, Debug)]
pub struct TextSpan {
    pub inline: usize,
    pub start: usize,
    pub end: usize,
    pub editable: bool,
}
/// A lossless editing projection. Field text is displayed but never converted into plain runs.
#[derive(Clone, Debug)]
pub struct ParagraphText {
    pub paragraph: Id,
    pub text: String,
    pub spans: Vec<TextSpan>,
}
impl ParagraphText {
    /// Resolve a caret to a text run, preferring the preceding editable run at a seam.
    pub fn position(&self, offset: usize) -> Result<TextPosition, Error> {
        if !self.text.is_char_boundary(offset) {
            return Err(Error::Invalid);
        }
        let span = self
            .spans
            .iter()
            .find(|s| s.editable && s.start == offset && s.end == offset)
            .or_else(|| {
                self.spans
                    .iter()
                    .find(|s| s.editable && s.start <= offset && offset <= s.end)
            })
            .ok_or(Error::Invalid)?;
        Ok(TextPosition {
            paragraph: self.paragraph,
            inline: span.inline,
            offset: offset - span.start,
        })
    }
    /// Map a native inline caret back to paragraph coordinates.
    pub fn offset(&self, inline: usize, offset: usize) -> Option<usize> {
        let span = self
            .spans
            .iter()
            .find(|s| s.inline == inline && s.editable)?;
        let at = span.start.checked_add(offset)?;
        (at <= span.end && self.text.is_char_boundary(at)).then_some(at)
    }
    /// Resolve a selection across adjacent text runs; field intersections are rejected.
    pub fn selection(&self, start: usize, end: usize) -> Result<TextMatch, Error> {
        self.text.get(start..end).ok_or(Error::Invalid)?;
        if self
            .spans
            .iter()
            .any(|s| !s.editable && start < s.end && end > s.start)
        {
            return Err(Error::InUse);
        }
        let a = if start == end {
            self.position(start)?
        } else {
            let s = self
                .spans
                .iter()
                .find(|s| s.editable && s.start <= start && start < s.end)
                .ok_or(Error::Invalid)?;
            TextPosition {
                paragraph: self.paragraph,
                inline: s.inline,
                offset: start - s.start,
            }
        };
        let b = self.position(end)?;
        Ok(TextMatch {
            paragraph: self.paragraph,
            start_inline: a.inline,
            start: a.offset,
            end_inline: b.inline,
            end: b.offset,
        })
    }
}
impl Document {
    /// Displayed paragraph text with UTF-8 source ranges for editable text and generated fields.
    pub fn text_projection(&self, id: Id) -> Result<ParagraphText, Error> {
        let p = self.paragraph(id).ok_or(Error::Missing)?;
        let mut out = ParagraphText {
            paragraph: id,
            text: String::new(),
            spans: Vec::new(),
        };
        for (inline, node) in p.content.iter().enumerate() {
            let start = out.text.len();
            match node {
                Inline::Text(t) | Inline::StyledText { text: t, .. } => out.text.push_str(t),
                Inline::Citation { source, locator } => {
                    out.text.push_str(&self.citation_text(*source, locator))
                }
                Inline::CitationCluster(items) => {
                    out.text.push_str(&self.citation_cluster_text(items))
                }
                Inline::NoteReference(id) => out.text.push_str(&self.note_marker(*id)),
                Inline::Bibliography => out.text.push_str(&self.bibliography().join("\n")),
                Inline::Formula(f) => out.text.push_str(&self.evaluate_formula(p.id,f).text),
            }
            if out.text.len() > 4 << 20 {
                return Err(Error::Limit);
            }
            out.spans.push(TextSpan {
                inline,
                start,
                end: out.text.len(),
                editable: node.text().is_some(),
            });
        }
        if p.content.is_empty() {
            out.spans.push(TextSpan {
                inline: 0,
                start: 0,
                end: 0,
                editable: true,
            });
        }
        Ok(out)
    }
    pub(crate) fn replace_paragraph_range(
        &mut self,
        paragraph: Id,
        start: usize,
        end: usize,
        text: &str,
    ) -> Result<(), Error> {
        let selection = self.text_projection(paragraph)?.selection(start, end)?;
        let p = self.paragraph_mut(paragraph).ok_or(Error::Missing)?;
        if p.content.is_empty() {
            p.content.push(Inline::Text(String::new()));
        }
        crate::search::replace(p, &selection, None, text)?;
        // Native typing must not manufacture a new formatting run for every keystroke.
        coalesce(&mut p.content);
        Ok(())
    }
    pub(crate) fn format_paragraph_range(
        &mut self,
        paragraph: Id,
        start: usize,
        end: usize,
        format: TextFormat,
        replace: bool,
    ) -> Result<(), Error> {
        format.validate()?;
        let selection = self.text_projection(paragraph)?.selection(start, end)?;
        let p = self.paragraph_mut(paragraph).ok_or(Error::Missing)?;
        if p.content.is_empty() {
            p.content.push(Inline::Text(String::new()));
        }
        let mut content = Vec::new();
        for (i, node) in p.content.iter().enumerate() {
            if i < selection.start_inline || i > selection.end_inline {
                content.push(node.clone());
                continue;
            }
            let text = node.text().ok_or(Error::Invalid)?;
            let a = if i == selection.start_inline {
                selection.start
            } else {
                0
            };
            let b = if i == selection.end_inline {
                selection.end
            } else {
                text.len()
            };
            if a > 0 {
                content.push(Inline::with_format(String::from(&text[..a]), node.format()));
            }
            let old = node.format();
            let applied = if replace { format } else { old.overlay(format) };
            content.push(Inline::with_format(String::from(&text[a..b]), applied));
            if b < text.len() {
                content.push(Inline::with_format(String::from(&text[b..]), node.format()));
            }
        }
        coalesce(&mut content);
        p.content = content;
        Ok(())
    }
}
pub(crate) fn coalesce(content: &mut Vec<Inline>) {
    let mut out: Vec<Inline> = Vec::new();
    for node in core::mem::take(content) {
        if let Some(last) = out.last_mut() {
            if let (Some(a), Some(b)) = (last.text(), node.text()) {
                if last.format() == node.format() {
                    let mut joined = String::from(a);
                    joined.push_str(b);
                    *last = Inline::with_format(joined, last.format());
                    continue;
                }
            }
        }
        out.push(node);
    }
    *content = out;
}
