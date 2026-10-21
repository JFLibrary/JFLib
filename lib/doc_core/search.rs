// Literal Unicode search over adjacent text runs, without searching generated citation fields.
use crate::{Document, Error, Id, Inline, Paragraph};
use alloc::{string::String, vec::Vec};

/// Literal search behavior. Case-insensitive mode compares Unicode lowercase characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
}
impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            case_sensitive: true,
            whole_word: false,
        }
    }
}
/// UTF-8 byte positions in one paragraph, possibly crossing adjacent formatting runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextMatch {
    pub paragraph: Id,
    pub start_inline: usize,
    pub start: usize,
    pub end_inline: usize,
    pub end: usize,
}
fn word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}
fn positions(text: &str, query: &str, options: SearchOptions) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut next = 0;
    for (start, _) in text.char_indices() {
        if start < next {
            continue;
        }
        let mut tail = text[start..].char_indices();
        let mut end = start;
        let matches = query.chars().all(|needle| {
            if let Some((at, c)) = tail.next() {
                end = start + at + c.len_utf8();
                if options.case_sensitive {
                    c == needle
                } else {
                    c.to_lowercase().eq(needle.to_lowercase())
                }
            } else {
                false
            }
        });
        if matches
            && (!options.whole_word
                || (!text[..start].chars().next_back().is_some_and(word)
                    && !text[end..].chars().next().is_some_and(word)))
        {
            out.push((start, end));
            next = end;
            if out.len() > 10000 {
                break;
            }
        }
    }
    out
}
impl Document {
    /// Find non-overlapping literal matches. Formatting boundaries are transparent; fields and
    /// paragraph boundaries are not. Each main paragraph is followed by its anchored note bodies
    /// in anchor order, independent of note storage order. At most 10,000 matches and 4,096 query
    /// bytes are accepted.
    pub fn find_text(&self, query: &str, options: SearchOptions) -> Result<Vec<TextMatch>, Error> {
        if query.is_empty() || query.len() > 4096 {
            return Err(Error::Invalid);
        }
        let mut out = Vec::new();
        let paragraphs = self.body_paragraphs().flat_map(|paragraph| {
            core::iter::once(paragraph).chain(
                paragraph
                    .content
                    .iter()
                    .filter_map(|node| match node {
                        Inline::NoteReference(id) => self.note(*id),
                        _ => None,
                    })
                    .flat_map(|note| &note.paragraphs),
            )
        });
        for p in paragraphs {
            let mut i = 0;
            while i < p.content.len() {
                if p.content[i].text().is_none() {
                    i += 1;
                    continue;
                }
                let mut joined = String::new();
                let mut spans = Vec::new();
                while i < p.content.len() {
                    let Some(text) = p.content[i].text() else {
                        break;
                    };
                    spans.push((i, joined.len(), text.len()));
                    joined.push_str(text);
                    i += 1;
                }
                for (start, end) in positions(&joined, query, options) {
                    let &(si, so, _) = spans
                        .iter()
                        .find(|(_, at, len)| start < at + len)
                        .ok_or(Error::Invalid)?;
                    let &(ei, eo, _) = spans
                        .iter()
                        .find(|(_, at, len)| end <= at + len && end > *at)
                        .ok_or(Error::Invalid)?;
                    out.push(TextMatch {
                        paragraph: p.id,
                        start_inline: si,
                        start: start - so,
                        end_inline: ei,
                        end: end - eo,
                    });
                    if out.len() > 10000 {
                        return Err(Error::Limit);
                    }
                }
            }
        }
        Ok(out)
    }
}
pub(crate) fn replace(
    p: &mut Paragraph,
    m: &TextMatch,
    expected: Option<&str>,
    replacement: &str,
) -> Result<(), Error> {
    if p.id != m.paragraph || m.start_inline > m.end_inline || m.end_inline >= p.content.len() {
        return Err(Error::Invalid);
    }
    let mut selected = String::new();
    for i in m.start_inline..=m.end_inline {
        let text = p.content[i].text().ok_or(Error::Invalid)?;
        let start = if i == m.start_inline { m.start } else { 0 };
        let end = if i == m.end_inline { m.end } else { text.len() };
        selected.push_str(text.get(start..end).ok_or(Error::Invalid)?);
    }
    if expected.is_some_and(|s| s != selected) {
        return Err(Error::Invalid);
    }
    if selected == replacement {
        return Ok(());
    }
    let first = &p.content[m.start_inline];
    let last = &p.content[m.end_inline];
    let prefix = &first.text().ok_or(Error::Invalid)?[..m.start];
    let suffix = &last.text().ok_or(Error::Invalid)?[m.end..];
    let mut updated = Vec::new();
    if !prefix.is_empty() {
        updated.push(Inline::with_format(String::from(prefix), first.format()));
    }
    if !replacement.is_empty() {
        updated.push(Inline::with_format(
            String::from(replacement),
            first.format(),
        ));
    }
    if !suffix.is_empty() {
        updated.push(Inline::with_format(String::from(suffix), last.format()));
    }
    if updated.is_empty() {
        updated.push(Inline::with_format(String::new(), first.format()));
    }
    p.content.splice(m.start_inline..=m.end_inline, updated);
    Ok(())
}
