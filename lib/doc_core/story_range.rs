// Continuous story coordinates; paragraph structure never lives in the GUI's text buffer.
use crate::{Document, Error, Id, Inline, ParagraphChange, ParagraphText, TextFormat};
use alloc::{string::String, vec::Vec};

/// One paragraph's projection and starting UTF-8 byte offset in its story.
#[derive(Clone, Debug)]
pub struct StoryParagraph {
    pub start: usize,
    pub projection: ParagraphText,
}
/// A main body or one note, separated by single LF paragraph marks. Existing soft line
/// breaks also display as LF; paragraph provenance distinguishes them during every edit.
#[derive(Clone, Debug)]
pub struct StoryText {
    pub text: String,
    pub paragraphs: Vec<StoryParagraph>,
}
impl StoryText {
    /// Resolve a story caret to paragraph identity and paragraph-local byte offset.
    /// The caret immediately before a paragraph mark belongs to the preceding paragraph.
    pub fn position(&self, offset: usize) -> Result<(Id, usize), Error> {
        if !self.text.is_char_boundary(offset) {
            return Err(Error::Invalid);
        }
        let p = self
            .paragraphs
            .iter()
            .rev()
            .find(|p| p.start <= offset)
            .ok_or(Error::Invalid)?;
        let local = offset - p.start;
        if local > p.projection.text.len() {
            return Err(Error::Invalid);
        }
        Ok((p.projection.paragraph, local))
    }
    /// Translate an existing paragraph-local caret into the story buffer.
    pub fn offset(&self, paragraph: Id, offset: usize) -> Option<usize> {
        let p = self
            .paragraphs
            .iter()
            .find(|p| p.projection.paragraph == paragraph)?;
        p.projection
            .text
            .is_char_boundary(offset)
            .then_some(p.start + offset)
    }
    fn bounds(&self, start: usize, end: usize) -> Result<(usize, usize, usize, usize), Error> {
        self.text.get(start..end).ok_or(Error::Invalid)?;
        let (a, lo) = self.position(start)?;
        let (b, hi) = self.position(end)?;
        let first = self
            .paragraphs
            .iter()
            .position(|p| p.projection.paragraph == a)
            .unwrap();
        let last = self
            .paragraphs
            .iter()
            .position(|p| p.projection.paragraph == b)
            .unwrap();
        Ok((first, lo, last, hi))
    }
    fn range(&self, start: usize, end: usize) -> Result<(usize, usize, usize, usize), Error> {
        let (first, lo, last, hi) = self.bounds(start, end)?;
        for (index, p) in self
            .paragraphs
            .iter()
            .enumerate()
            .take(last + 1)
            .skip(first)
        {
            let from = if index == first { lo } else { 0 };
            let to = if index == last {
                hi
            } else {
                p.projection.text.len()
            };
            if from < to {
                p.projection.selection(from, to)?;
            }
            // Removing a complete intermediate paragraph also removes zero-width fields.
            if index > first && index < last && p.projection.spans.iter().any(|s| !s.editable) {
                return Err(Error::InUse);
            }
        }
        Ok((first, lo, last, hi))
    }
}
impl Document {
    /// Effective properties shared by every editable run in a selection. `None` marks
    /// mixed values. A caret samples its insertion format; fields remain protected.
    pub fn story_character_common(&self, paragraph: Id, start: usize, end: usize) -> Result<TextFormat, Error> {
        if start == end { return self.story_character_format(paragraph, start, end); }
        let story = self.story_projection(paragraph)?;
        let (first, lo, last, hi) = story.range(start, end)?;
        let mut common: Option<TextFormat> = None;
        for (i, p) in story.paragraphs.iter().enumerate().take(last + 1).skip(first) {
            let from = if i == first { lo } else { 0 };
            let to = if i == last { hi } else { p.projection.text.len() };
            let paragraph = self.paragraph(p.projection.paragraph).ok_or(Error::Missing)?;
            for span in &p.projection.spans {
                if span.editable && span.start < to && span.end > from {
                    let f = self.resolve_paragraph_text(paragraph, paragraph.content[span.inline].format()).explicit();
                    common = Some(common.map_or(f, |previous| previous.common(f)));
                }
            }
        }
        common.map_or_else(|| self.story_character_format(paragraph, start, start), Ok)
    }
    /// Sample effective character formatting at the start of a UTF-8 story selection.
    /// Empty selections use caret affinity; nonempty selections use the selected run.
    /// Protected fields and invalid scalar boundaries are rejected consistently with edits.
    pub fn story_character_format(&self, paragraph: Id, start: usize, end: usize) -> Result<TextFormat, Error> {
        let story = self.story_projection(paragraph)?;
        let (first, lo, last, hi) = story.range(start, end)?;
        let projection = &story.paragraphs[first].projection;
        let selection = projection.selection(lo, if first == last { hi } else { projection.text.len() })?;
        let paragraph = self.paragraph(projection.paragraph).ok_or(Error::Missing)?;
        Ok(self.resolve_paragraph_text(paragraph, paragraph.content.get(selection.start_inline).map_or_else(TextFormat::default, Inline::format)).explicit())
    }

    pub(crate) fn change_story_paragraphs(
        &mut self,
        paragraph: Id,
        start: usize,
        end: usize,
        change: ParagraphChange,
    ) -> Result<(), Error> {
        let story = self.story_projection(paragraph)?;
        let (first, _, last, hi) = story.bounds(start, end)?;
        let last = if last > first && hi == 0 {
            last - 1
        } else {
            last
        };
        for span in &story.paragraphs[first..=last] {
            let tabs = if let ParagraphChange::Tabs(desired) = change {
                let mut inherited = self.paragraph(span.projection.paragraph).ok_or(Error::Missing)?.clone();
                inherited.format.tabs = Default::default();
                Some(desired.relative_to(self.resolve_paragraph(&inherited).tabs)?)
            } else { None };
            let p = self
                .paragraph_mut(span.projection.paragraph)
                .ok_or(Error::Missing)?;
            match change {
                ParagraphChange::Style(style) => p.style = style,
                ParagraphChange::Format(format) => p.format = format,
                ParagraphChange::Properties(format) => p.format = p.format.overlay(format),
                ParagraphChange::Tabs(_) => { p.format.tabs = tabs.unwrap(); p.format.explicit &= !256; },
                ParagraphChange::Alignment(alignment) => { p.format.alignment = alignment; p.format.explicit |= 1; },
                ParagraphChange::PageBreakBefore(value) => { p.format.page_break_before = value; p.format.explicit |= 128; },
            }
        }
        Ok(())
    }
    /// Project the entire story containing this paragraph. Note bodies remain separate from
    /// the main body and from other notes, even when their lines share the same printed page.
    pub fn story_projection(&self, paragraph: Id) -> Result<StoryText, Error> {
        self.paragraph(paragraph).ok_or(Error::Missing)?;
        let group = self.paragraph_group(paragraph).ok_or(Error::Missing)?;
        let mut out = StoryText {
            text: String::new(),
            paragraphs: Vec::new(),
        };
        for p in group {
            if !out.paragraphs.is_empty() {
                out.text.push('\n');
            }
            let projection = self.text_projection(p.id)?;
            let start = out.text.len();
            out.text.push_str(&projection.text);
            if out.text.len() > 4 << 20 {
                return Err(Error::Limit);
            }
            out.paragraphs.push(StoryParagraph { start, projection });
        }
        Ok(out)
    }
    pub(crate) fn replace_story_range(
        &mut self,
        paragraph: Id,
        start: usize,
        end: usize,
        text: &str,
        paragraph_breaks: bool,
        allocate: &mut impl FnMut() -> Result<Id, Error>,
    ) -> Result<Id, Error> {
        crate::check_text(text)?;
        if text.len() > 2 << 20 {
            return Err(Error::Limit);
        }
        let story = self.story_projection(paragraph)?;
        let (first, lo, last, hi) = story.range(start, end)?;
        let left = story.paragraphs[first].projection.paragraph;
        let right = story.paragraphs[last].projection.paragraph;
        let normalised;
        let text = if paragraph_breaks {
            normalised = text.replace("\r\n", "\n").replace('\r', "\n");
            normalised.as_str()
        } else {
            text
        };
        if first == last && (!paragraph_breaks || !text.contains('\n')) {
            self.replace_paragraph_range(left, lo, hi, text)?;
            return Ok(left);
        }
        let projection = &story.paragraphs[first].projection;
        let selected_end = if first == last {
            hi
        } else {
            projection.text.len()
        };
        let at = projection
            .selection(lo, selected_end)
            .ok()
            .map(|r| r.start_inline)
            .or_else(|| projection.position(lo).ok().map(|p| p.inline));
        let insertion_format = at
            .and_then(|i| self.paragraph(left)?.content.get(i))
            .map_or_else(TextFormat::default, Inline::format);
        if first == last {
            self.replace_paragraph_range(left, lo, hi, "")?;
        } else {
            let length = story.paragraphs[first].projection.text.len();
            if lo < length {
                self.replace_paragraph_range(left, lo, length, "")?;
            }
            if hi > 0 {
                self.replace_paragraph_range(right, 0, hi, "")?;
            }
            let group = self.paragraph_group_mut(left).ok_or(Error::Missing)?;
            let base = group.iter().position(|p| p.id == left).ok_or(Error::Missing)?;
            group.drain(base + 1..base + last - first);
            self.join_paragraphs(left, right)?;
        }
        let mut id = left;
        let mut offset = lo;
        if paragraph_breaks {
            for (index, part) in text.split('\n').enumerate() {
                if index != 0 {
                    let at = self.text_projection(id)?.position(offset)?;
                    let next = allocate()?;
                    self.split_paragraph(id, at.inline, at.offset, at.offset, next)?;
                    id = next;
                    offset = 0;
                }
                self.insert_story_text(id, offset, part, insertion_format)?;
                offset += part.len();
            }
        } else if !text.is_empty() {
            self.insert_story_text(id, offset, text, insertion_format)?;
        }
        Ok(id)
    }
    fn insert_story_text(
        &mut self,
        paragraph: Id,
        offset: usize,
        text: &str,
        format: TextFormat,
    ) -> Result<(), Error> {
        let at = self.text_projection(paragraph)?.position(offset)?;
        let p = self.paragraph_mut(paragraph).ok_or(Error::Missing)?;
        if p.content.is_empty() {
            p.content.push(Inline::Text(String::new()));
        }
        let original = &p.content[at.inline];
        let content = original.text().ok_or(Error::Invalid)?;
        let mut nodes = Vec::new();
        if at.offset > 0 {
            nodes.push(Inline::with_format(
                String::from(&content[..at.offset]),
                original.format(),
            ));
        }
        nodes.push(Inline::with_format(String::from(text), format));
        if at.offset < content.len() {
            nodes.push(Inline::with_format(
                String::from(&content[at.offset..]),
                original.format(),
            ));
        }
        p.content.splice(at.inline..=at.inline, nodes);
        crate::text_range::coalesce(&mut p.content);
        Ok(())
    }
    pub(crate) fn format_story_range(
        &mut self,
        paragraph: Id,
        start: usize,
        end: usize,
        format: TextFormat,
        replace: bool,
    ) -> Result<(), Error> {
        format.validate()?;
        let story = self.story_projection(paragraph)?;
        let (first, lo, last, hi) = story.range(start, end)?;
        for (index, p) in story
            .paragraphs
            .iter()
            .enumerate()
            .take(last + 1)
            .skip(first)
        {
            let from = if index == first { lo } else { 0 };
            let to = if index == last {
                hi
            } else {
                p.projection.text.len()
            };
            if from < to || start == end || index < last {
                self.format_paragraph_range(p.projection.paragraph, from, to, format, replace)?;
            }
        }
        Ok(())
    }
}
