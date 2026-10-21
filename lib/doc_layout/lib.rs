// Bounded paragraph flow and pagination in twips. New native functionality; no legacy counterpart.
// Font measurement is supplied by the caller, so pagination is independent of zoom and GUI DPI.
#![no_std]
#![forbid(unsafe_code)]
extern crate alloc;
use alloc::{string::String, vec, vec::Vec};
use doc_core::{
    Alignment, Document, Id, Inline, PageSettings, Paragraph, ParagraphStyle, ResolvedFormat,
    TextFormat,
};
mod notes;
mod tabs;
mod table_fit;
mod table_borders;
pub use table_borders::CellEdge;
pub use table_fit::{TableFit,table_fit};

/// Font metrics in twips (1/20 point). Positive ascent/height and nonnegative descent.
#[derive(Clone, Copy, Debug)]
pub struct Metrics {
    pub ascent: i32,
    pub descent: i32,
    pub height: i32,
}
/// A reusable measurement interface. Widths must be nonnegative, metrics positive and bounded.
pub trait Measure {
    /// Measure a fragment at `format.display_size_half_points()`, without GUI zoom.
    fn width(&mut self, text: &str, format: ResolvedFormat) -> i32;
    /// Return glyph metrics at `format.display_size_half_points()`, in twips.
    /// The layout itself accounts for the script baseline displacement.
    fn metrics(&mut self, format: ResolvedFormat) -> Metrics;
}
/// A text fragment positioned on a page in twips, ready for the caller's renderer.
#[derive(Clone, Debug)]
pub struct Fragment {
    /// Visual fill for a tab; `text` remains empty and its source is the original tab.
    pub leader: doc_core::TabLeader,
    /// UTF-8 range in Document::text_projection; None for generated note-number prefixes.
    pub source: Option<(usize, usize)>,
    pub text: String,
    pub format: ResolvedFormat,
    pub x: i32,
    pub baseline: i32,
    pub width: i32,
}
/// A line with a stable paragraph target for navigation and hit testing.
#[derive(Clone, Debug)]
pub struct Line {
    /// Owning table cell, or `None` for body and note text.
    pub cell: Option<Id>,
    /// Absolute page x positions of bar tab rules, in twips.
    pub bar_tabs: Vec<i32>,
    /// Start/end in the paragraph's displayed UTF-8 text, excluding an explicit line break.
    pub start: usize,
    pub end: usize,
    /// Document paragraph ID, or zero for a generated non-editable heading.
    pub paragraph: Id,
    /// Aligned start in twips, including an empty line's insertion point.
    pub x: i32,
    pub top: i32,
    pub height: i32,
    pub fragments: Vec<Fragment>,
}
/// One page; an empty document still produces one page.
#[derive(Clone, Debug, Default)]
pub struct Page {
    pub cells: Vec<CellBox>,
    pub lines: Vec<Line>,
    /// Page-bottom rules above footnotes; empty on pages without footnotes.
    pub separators: Vec<Separator>,
}
/// A table cell rectangle in page twips, shared by paint, hit testing and resizing.
#[derive(Clone, Debug)]
pub struct CellBox {
    /// Shared edges resolved per adjacent grid segment; offsets are relative to this box.
    pub edges: Vec<CellEdge>,
    /// Display instance of an existing header cell; never a second document cell.
    pub repeated: bool,
    pub table: Id, pub cell: Id, pub row: usize, pub column: usize, pub paragraph: Id,
    pub row_span: usize, pub column_span: usize, pub last_row_height: u32,
    /// False for a cell fragment ending at a page break instead of its grid row boundary.
    pub row_end: bool,
    pub rect: [i32;4], pub fill: Option<u32>, pub border_width: u16, pub border_color: u32,
}
/// A horizontal rule separating footnote content from the main page text, in twips.
#[derive(Clone, Copy, Debug)]
pub struct Separator {
    pub x: i32,
    pub y: i32,
    pub width: i32,
}
/// Laid-out document; page coordinates do not change when the view zoom changes.
#[derive(Clone, Debug)]
pub struct Layout {
    pub settings: PageSettings,
    pub pages: Vec<Page>,
}
/// Invalid model/measurements or exceeded expansion, fragment or page budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Model(doc_core::Error),
    Metrics,
    Limit,
}
impl Layout {
    /// Hit-test the vertical extent of a line on a page, in page-relative twips.
    pub fn paragraph_at(&self, page: usize, y: i32) -> Option<Id> {
        self.pages
            .get(page)?
            .lines
            .iter()
            .find(|line| line.paragraph != 0 && y >= line.top && y < line.top + line.height)
            .map(|line| line.paragraph)
    }
}
#[derive(Clone)]
struct Atom {
    leader: doc_core::TabLeader,
    source: Option<(usize, usize)>,
    text: String,
    format: ResolvedFormat,
    width: i32,
    metrics: Metrics,
    space: bool,
    newline: bool,
    tab: bool,
    note: Option<Id>,
}
fn metrics(measure: &mut impl Measure, f: ResolvedFormat) -> Result<Metrics, Error> {
    let m = measure.metrics(f);
    if !(1..=10000).contains(&m.ascent)
        || !(0..=10000).contains(&m.descent)
        || !(1..=10000).contains(&m.height)
        || m.height < m.ascent + m.descent
    {
        Err(Error::Metrics)
    } else {
        let shift = f.baseline_shift();
        let ascent = (m.ascent - shift).max(1);
        let descent = (m.descent + shift).max(0);
        Ok(Metrics { ascent, descent, height: m.height.max(ascent + descent) })
    }
}
fn atoms(
    text: &str,
    f: ResolvedFormat,
    measure: &mut impl Measure,
    out: &mut Vec<Atom>,
) -> Result<(), Error> {
    let m = metrics(measure, f)?;
    // Break opportunities: ordinary spaces, tabs and explicit newlines. NBSP stays within words.
    let mut start = 0;
    let push = |s: &str,
                space: bool,
                newline: bool,
                tab: bool,
                out: &mut Vec<Atom>,
                measure: &mut dyn Measure|
     -> Result<(), Error> {
        let width = if newline || tab {
            0
        } else {
            measure.width(s, f)
        };
        if width < 0 || width > 100_000_000 {
            return Err(Error::Metrics);
        }
        out.push(Atom {
            leader: doc_core::TabLeader::None,
            source: None,
            text: String::from(s),
            format: f,
            width,
            metrics: m,
            space,
            newline,
            tab,
            note: None,
        });
        if out.len() > 100000 {
            return Err(Error::Limit);
        }
        Ok(())
    };
    for (at, c) in text.char_indices() {
        if matches!(c, ' ' | '\t' | '\n' | '\r') {
            if start < at {
                push(&text[start..at], false, false, false, out, measure)?;
            }
            if c != '\r' || text.as_bytes().get(at + 1) != Some(&b'\n') {
                push(
                    &text[at..at + c.len_utf8()],
                    c == ' ',
                    matches!(c, '\r' | '\n'),
                    c == '\t',
                    out,
                    measure,
                )?;
            }
            start = at + c.len_utf8();
        }
    }
    if start < text.len() {
        push(&text[start..], false, false, false, out, measure)?;
    }
    Ok(())
}
fn paragraph_atoms(
    d: &Document,
    p: &Paragraph,
    bibliography: &str,
    measure: &mut impl Measure,
    expanded: &mut usize,
) -> Result<Vec<Atom>, Error> {
    let mut out = Vec::new();
    let mut offset = 0;
    for inline in &p.content {
        let mut f = d.resolve_paragraph_text(p, inline.format());
        let generated;
        let text = match inline {
            Inline::Text(t) | Inline::StyledText { text: t, .. } => t.as_str(),
            Inline::Citation { source, locator } => {
                generated = d.citation_text(*source, locator);
                &generated
            }
            Inline::CitationCluster(items) => {
                generated = d.citation_cluster_text(items);
                &generated
            }
            Inline::Bibliography => bibliography,
            Inline::Formula(f) => { generated=d.evaluate_formula(p.id,f).text;generated.as_str() },
            Inline::NoteReference(id) => {
                generated = d.note_marker(*id);
                f.size_half_points = (f.size_half_points * 7 / 10).max(8);
                &generated
            }
        };
        *expanded = expanded.checked_add(text.len()).ok_or(Error::Limit)?;
        if *expanded > 4 << 20 {
            return Err(Error::Limit);
        }
        let start = out.len();
        atoms(text, f, measure, &mut out)?;
        let mut cursor = 0;
        for atom in &mut out[start..] {
            let at = cursor + text[cursor..].find(&atom.text).ok_or(Error::Metrics)?;
            cursor = at + atom.text.len();
            atom.source = Some((offset + at, offset + cursor));
        }
        offset += text.len();
        if let Inline::NoteReference(id) = inline {
            for atom in &mut out[start..] {
                atom.note = Some(*id);
            }
        }
    }
    Ok(out)
}
struct Pending {
    left: i32,
    gap: i32,
    page_break: bool,
    indent: i32,
    available: i32,
    start: usize,
    end: usize,
    next: usize,
    atoms: Vec<Atom>,
    width: i32,
    hard: bool,
}
fn flush(lines: &mut Vec<Pending>, current: &mut Pending) -> Result<(), Error> {
    let next = current.next;
    let available = current.available + current.indent;
    lines.push(core::mem::replace(
        current,
        Pending {
            left: 0, gap: 0, page_break: false,
            indent: 0,
            available,
            start: next,
            end: next,
            next,
            atoms: Vec::new(),
            width: 0,
            hard: false,
        },
    ));
    if lines.len() > 100000 {
        Err(Error::Limit)
    } else {
        Ok(())
    }
}
fn append(current: &mut Pending, atom: Atom) {
    if let Some((_, end)) = atom.source {
        current.end = end;
        current.next = end;
    }
    current.width += atom.width;
    current.atoms.push(atom);
}
fn wrap(
    input: Vec<Atom>,
    width: i32,
    pf: doc_core::ParagraphFormat,
    measure: &mut impl Measure,
) -> Result<Vec<Pending>, Error> {
    wrap_with(input,width,pf,measure,&mut |_,_|Ok(()))
}
fn wrap_with(
    input: Vec<Atom>, width:i32, pf:doc_core::ParagraphFormat, measure:&mut impl Measure,
    prepare:&mut impl FnMut(&mut Pending,Option<&Pending>)->Result<(),Error>,
) -> Result<Vec<Pending>,Error> {
    let mut lines = Vec::new();
    let mut current = Pending {
        left: 0, gap: 0, page_break: false,
        indent: pf.first_line,
        available: width - pf.first_line,
        start: 0,
        end: 0,
        next: 0,
        atoms: Vec::new(),
        width: 0,
        hard: false,
    };
    prepare(&mut current,None)?;
    let flush = |lines:&mut Vec<Pending>,current:&mut Pending,prepare:&mut dyn FnMut(&mut Pending,Option<&Pending>)->Result<(),Error>| {
        self::flush(lines,current)?;
        current.available=width;
        prepare(current,lines.last())
    };
    let mut i = 0;
    while i < input.len() {
        let mut atom = input[i].clone();
        if atom.newline {
            if let Some((start, end)) = atom.source {
                current.end = start;
                current.next = end;
            }
            current.hard = true;
            flush(&mut lines, &mut current,prepare)?;
            i += 1;
            continue;
        }
        if atom.space || atom.tab {
            if atom.tab {
                (atom.width, atom.leader) = tabs::advance(&current, &input[i + 1..], pf, measure)?;
                atom.text.clear();
            }
            if current.width + atom.width > current.available {
                if let Some((_, end)) = atom.source {
                    current.end = end;
                    current.next = end;
                }
                flush(&mut lines, &mut current,prepare)?;
            } else {
                append(&mut current, atom);
            }
            i += 1;
            continue;
        }
        // Adjacent formatting runs do not introduce word-break opportunities.
        let start = i;
        let mut word_width = 0i64;
        while i < input.len() && !input[i].space && !input[i].tab && !input[i].newline {
            word_width += input[i].width as i64;
            i += 1;
        }
        if current.width as i64 + word_width > current.available as i64 && !current.atoms.is_empty()
        {
            flush(&mut lines, &mut current,prepare)?;
        }
        for atom in &input[start..i] {
            if atom.note.is_some() {
                if current.width + atom.width > current.available && !current.atoms.is_empty() {
                    flush(&mut lines, &mut current,prepare)?;
                }
                if atom.width > current.available {
                    return Err(Error::Limit);
                }
                append(&mut current, atom.clone());
                continue;
            }
            if current.width + atom.width <= current.available && atom.text.len() <= 4096 {
                append(&mut current, atom.clone());
                continue;
            }
            // Split a long token only at UTF-8 character boundaries, with bounded candidates.
            let mut part = String::new();
            let mut part_width = 0;
            let mut consumed = 0;
            for ch in atom.text.chars() {
                let mut candidate = part.clone();
                candidate.push(ch);
                if candidate.len() > 4096 {
                    return Err(Error::Limit);
                }
                let w = measure.width(&candidate, atom.format);
                if !(0..=100_000_000).contains(&w) {
                    return Err(Error::Metrics);
                }
                if current.width + w > current.available {
                    if !part.is_empty() {
                        let mut piece = atom.clone();
                        piece.text = core::mem::take(&mut part);
                        piece.source = atom.source.map(|(start, _)| {
                            (start + consumed, start + consumed + piece.text.len())
                        });
                        consumed += piece.text.len();
                        piece.width = part_width;
                        append(&mut current, piece);
                    }
                    if !current.atoms.is_empty() {
                        flush(&mut lines, &mut current,prepare)?;
                    }
                    part.push(ch);
                    part_width = measure.width(&part, atom.format);
                    if part_width < 0 || part_width > current.available {
                        return Err(Error::Limit);
                    }
                } else {
                    part = candidate;
                    part_width = w;
                }
            }
            if !part.is_empty() {
                let mut piece = atom.clone();
                piece.text = part;
                piece.source = atom
                    .source
                    .map(|(start, _)| (start + consumed, start + consumed + piece.text.len()));
                piece.width = part_width;
                append(&mut current, piece);
            }
        }
    }
    lines.push(current);
    Ok(lines)
}
fn line_height(line: &Pending, default: Metrics, spacing: u16) -> (i32, i32) {
    let ascent = line
        .atoms
        .iter()
        .map(|a| a.metrics.ascent)
        .max()
        .unwrap_or(default.ascent);
    let descent = line
        .atoms
        .iter()
        .map(|a| a.metrics.descent)
        .max()
        .unwrap_or(default.descent);
    let natural = line
        .atoms
        .iter()
        .map(|a| a.metrics.height)
        .max()
        .unwrap_or(default.height)
        .max(ascent + descent);
    let height = natural * spacing as i32 / 240;
    (height, ascent + (height - natural) / 2)
}
/// Paginate a validated document. Handles formatting runs, whitespace, tabs, line/page breaks,
/// paragraph spacing/indents, alignment and heading keep-with-next when it fits on one page.
/// Footnotes reserve page-bottom space with continuations; endnotes follow the main text.
/// An anchor line and the first line of every note it references must fit on the same page.
/// A same-line group that cannot meet that requirement without reordering note bodies fails
/// with `Error::Limit`; notes are never silently omitted.
/// Rectangular tables flow by rows. No shaping, bidirectional algorithm or Word-identical pagination is claimed.
pub fn paginate(d: &Document, measure: &mut impl Measure) -> Result<Layout, Error> {
    d.validate().map_err(Error::Model)?;
    if !d.notes.is_empty() || !d.tables.is_empty() {
        return notes::paginate(d, measure);
    }
    let mut layout = Layout {
        settings: d.page,
        pages: vec![Page::default()],
    };
    let top = d.page.top as i32;
    let bottom = (d.page.height - d.page.bottom) as i32;
    let mut y = top;
    let mut expanded = 0;
    let mut fragments = 0;
    let bibliography = d.bibliography().join("\n");
    let mut previous_after = 0;
    for (paragraph_index, p) in d.paragraphs.iter().enumerate() {
        let pf = d.resolve_paragraph(p);
        let width = d.page.content_width() as i32 - pf.left as i32 - pf.right as i32;
        let input = paragraph_atoms(d, p, &bibliography, measure, &mut expanded)?;
        let mut lines = wrap(input, width, pf, measure)?;
        let default = metrics(measure, d.resolve_paragraph_text(p, TextFormat::default()))?;
        let advance = previous_after.max(pf.before as i32);
        if pf.page_break_before && !layout.pages.last().unwrap().lines.is_empty() {
            layout.pages.push(Page::default());
            y = top;
        }
        if y > top {
            y += advance;
        } else {
            y += pf.before as i32;
        }
        let total: i64 = lines
            .iter()
            .map(|l| i64::from(line_height(l, default, pf.line).0))
            .sum();
        let mut reserve = 0;
        if d.outline_level(p.style).is_some() {
            if let Some(next) = d
                .paragraphs
                .get(paragraph_index + 1)
                .filter(|n| !d.resolve_paragraph(n).page_break_before)
            {
                let next_pf = d.resolve_paragraph(next);
                let next_width = d.page.content_width() as i32
                    - next_pf.left as i32
                    - next_pf.right as i32;
                let input = paragraph_atoms(d, next, &bibliography, measure, &mut 0)?;
                let next_lines = wrap(input, next_width, next_pf, measure)?;
                let next_metrics = metrics(measure, d.resolve_paragraph_text(next, TextFormat::default()))?;
                reserve = line_height(&next_lines[0], next_metrics, next_pf.line).0
                    + pf.after.max(next_pf.before) as i32;
            }
        }
        if reserve > 0
            && total + i64::from(reserve) <= i64::from(bottom - top)
            && i64::from(y) + total + i64::from(reserve) > i64::from(bottom)
            && y > top
        {
            layout.pages.push(Page::default());
            y = top;
        }
        let count = lines.len();
        for (index, line) in lines.iter_mut().enumerate() {
            while line.atoms.last().is_some_and(|a| a.space) {
                line.width -= line.atoms.pop().unwrap().width;
            }
            let (height, ascent) = line_height(line, default, pf.line);
            if height > bottom - top {
                return Err(Error::Limit);
            }
            if y + height > bottom {
                layout.pages.push(Page::default());
                y = top;
            }
            if layout.pages.len() > 2048 {
                return Err(Error::Limit);
            }
            let free = (line.available - line.width).max(0);
            let mut x = (d.page.left + pf.left as u32) as i32
                + line.indent
                + match if line.atoms.iter().any(|a| a.tab) { Alignment::Left } else { pf.alignment } {
                    Alignment::Center => free / 2,
                    Alignment::Right => free,
                    _ => 0,
                };
            let spaces = line.atoms.iter().filter(|a| a.space).count() as i32;
            let justify = pf.alignment == Alignment::Justify
                && !line.atoms.iter().any(|a| a.tab)
                && index + 1 < count
                && !line.hard
                && spaces > 0;
            let mut used_space = 0;
            let mut positioned = Line {
                cell: None,
                bar_tabs: tabs::bars(pf, d.page),
                start: line.start,
                end: line.end,
                paragraph: p.id,
                x,
                top: y,
                height,
                fragments: Vec::new(),
            };
            for atom in &line.atoms {
                let mut advance = atom.width;
                if justify && atom.space {
                    advance += free / spaces + i32::from(used_space < free % spaces);
                    used_space += 1;
                }
                if !atom.text.is_empty() || atom.source.is_some() {
                    positioned.fragments.push(Fragment {
                        leader: atom.leader,
                        source: atom.source,
                        text: atom.text.clone(),
                        format: atom.format,
                        x,
                        baseline: y + ascent + atom.format.baseline_shift(),
                        width: advance,
                    });
                    fragments += 1;
                    if fragments > 100000 {
                        return Err(Error::Limit);
                    }
                }
                x += advance;
            }
            layout.pages.last_mut().unwrap().lines.push(positioned);
            y += height;
        }
        previous_after = pf.after as i32;
    }
    Ok(layout)
}
