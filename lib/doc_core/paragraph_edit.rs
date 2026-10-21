// Atomic paragraph structure edits with stable field/note references. New native model code.
use crate::{Document, Error, Id, Inline, Paragraph, ParagraphStyle};
use alloc::{string::String, vec};

/// UTF-8 caret inside one editable text run, including virtual run zero of an empty paragraph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextPosition {
    /// Stable paragraph identity, in the main document or a note body.
    pub paragraph: Id,
    /// Zero-based inline text node; zero also addresses a virtual empty paragraph run.
    pub inline: usize,
    /// UTF-8 byte offset on a scalar boundary, not a character or display-column count.
    pub offset: usize,
}

fn pair(document: &Document, first: Id, second: Id) -> Result<(&Paragraph, &Paragraph), Error> {
    let group = document.paragraph_group(first).ok_or(Error::Missing)?;
    let index = group
        .iter()
        .position(|p| p.id == first)
        .ok_or(Error::Missing)?;
    let next = group
        .get(index + 1)
        .filter(|p| p.id == second)
        .ok_or(Error::Invalid)?;
    Ok((&group[index], next))
}

impl Document {
    /// Caret at the join seam after joining adjacent paragraphs in the same story.
    /// The first paragraph keeps its identity/style/paragraph properties; explicit character
    /// overrides remain unchanged. Fields retain their structure and are never textified.
    pub fn join_boundary(&self, first: Id, second: Id) -> Result<TextPosition, Error> {
        let (left, _) = pair(self, first, second)?;
        let (inline, offset) = left
            .content
            .last()
            .and_then(Inline::text)
            .map(|text| (left.content.len() - 1, text.len()))
            .unwrap_or((left.content.len(), 0));
        Ok(TextPosition {
            paragraph: first,
            inline,
            offset,
        })
    }

    pub(crate) fn split_paragraph(
        &mut self,
        paragraph: Id,
        inline: usize,
        start: usize,
        end: usize,
        created: Id,
    ) -> Result<(), Error> {
        let group = self.paragraph_group_mut(paragraph).ok_or(Error::Missing)?;
        let index = group
            .iter()
            .position(|p| p.id == paragraph)
            .ok_or(Error::Missing)?;
        let original = &group[index];
        let empty = Inline::Text(String::new());
        let node = if original.content.is_empty() && inline == 0 {
            &empty
        } else {
            original.content.get(inline).ok_or(Error::Missing)?
        };
        let text = node.text().ok_or(Error::Invalid)?;
        text.get(start..end).ok_or(Error::Invalid)?;
        let mut left = original.content[..inline].to_vec();
        left.push(Inline::with_format(
            String::from(&text[..start]),
            node.format(),
        ));
        let mut right = vec![Inline::with_format(
            String::from(&text[end..]),
            node.format(),
        )];
        right.extend_from_slice(original.content.get(inline + 1..).unwrap_or(&[]));
        let at_end = right
            .iter()
            .all(|node| node.text().is_some_and(str::is_empty));
        let style = if at_end {
            ParagraphStyle::Normal
        } else {
            original.style
        };
        let mut format = original.format;
        // A page break belongs before the existing paragraph, not before both halves.
        format.page_break_before = false;
        group[index].content = left;
        group.insert(
            index + 1,
            Paragraph {
                id: created,
                style,
                format,
                content: right,
            },
        );
        Ok(())
    }

    pub(crate) fn join_paragraphs(&mut self, first: Id, second: Id) -> Result<(), Error> {
        pair(self, first, second)?;
        let group = self.paragraph_group_mut(first).ok_or(Error::Missing)?;
        let index = group
            .iter()
            .position(|p| p.id == first)
            .ok_or(Error::Missing)?;
        let mut right = group.remove(index + 1).content;
        let left = &mut group[index].content;
        let seam_text = left.last().and_then(Inline::text).is_some();
        if !seam_text && right.first().and_then(Inline::text).is_none() {
            left.push(Inline::Text(String::new()));
        }
        if let (Some(last), Some(next)) = (left.last_mut(), right.first()) {
            if let (Some(a), Some(b)) = (last.text(), next.text()) {
                if last.format() == next.format() {
                    let mut text = String::from(a);
                    text.push_str(b);
                    *last = Inline::with_format(text, last.format());
                    right.remove(0);
                }
            }
        }
        left.extend(right);
        Ok(())
    }
}
