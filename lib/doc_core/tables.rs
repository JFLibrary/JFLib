//! Rectangular document tables. Cells own ordinary rich paragraphs and their own story.
use crate::{Document, Error, Id, Inline, Paragraph, ParagraphFormat, ParagraphStyle};
use alloc::{collections::BTreeSet, string::String, vec, vec::Vec};
#[path = "table_grid.rs"]
mod grid;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CellAlignment {
    #[default]
    Top,
    Center,
    Bottom,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableCell {
    pub id: Id,
    /// Grid extent of an anchor; both are zero for a covered grid slot.
    pub row_span: u16,
    pub column_span: u16,
    pub paragraphs: Vec<Paragraph>,
    pub fill: Option<u32>,
    pub alignment: CellAlignment,
    pub borders: crate::CellBorders,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableRow {
    pub cells: Vec<TableCell>,
    /// Requested row height in twips. Zero means automatic for `AtLeast`.
    pub min_height: u32,
    pub height_rule: RowHeightRule,
    /// Keep the row on one page when possible. Oversized rows still flow without losing text.
    pub cant_split: bool,
    /// Omitted leading/trailing grid intervals. These are space, not empty cells.
    pub grid_before: u16,
    pub grid_after: u16,
}
/// Whether cell content may increase the requested row height.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RowHeightRule {
    #[default]
    AtLeast,
    /// A positive fixed height; overflow stays in the model and is clipped in the view.
    Exact,
}
/// A body or cell block immediately before `before`. Several tables may share an anchor, in storage order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub id: Id,
    pub before: Id,
    pub position: Option<crate::TablePosition>,
    /// Contiguous leading rows repeated at automatic page breaks.
    pub header_rows: u16,
    pub style: Option<String>,
    pub look: crate::TableLook,
    /// False inherits the border from the named table style.
    pub border_override: bool,
    pub columns: Vec<u32>,
    pub rows: Vec<TableRow>,
    /// Uniform cell padding in twips (1/20 point).
    pub padding: u16,
    /// Uniform border width in eighth-points, matching WordprocessingML `w:sz`.
    pub border_width: u16,
    /// Opaque RGB color; application themes do not change it.
    pub border_color: u32,
}
/// Structural operations are one validated undo transaction; indices are zero based.
#[derive(Clone, Debug)]
pub enum TableChange {
    Position(Option<crate::TablePosition>),
    Borders { row:usize, column:usize, borders:crate::CellBorders },
    HeaderRows(u16),
    Style { style: Option<String>, look: crate::TableLook },
    Look(crate::TableLook),
    Merge {
        row: usize,
        column: usize,
        rows: usize,
        columns: usize,
    },
    Split {
        row: usize,
        column: usize,
    },
    Properties {
        row: usize,
        column: usize,
        width: u32,
        height: u32,
        height_rule: RowHeightRule,
        cant_split: bool,
        fill: Option<u32>,
        alignment: CellAlignment,
    },
    InsertRow(usize),
    RemoveRow(usize),
    InsertColumn(usize),
    RemoveColumn(usize),
    Columns(Vec<u32>),
    RowHeight {
        row: usize,
        height: u32,
    },
    /// Set height and rule together. `RowHeight` alone preserves the current rule.
    RowSizing { row: usize, height: u32, rule: RowHeightRule },
    RowBreak { row: usize, allow_split: bool },
    Cell {
        row: usize,
        column: usize,
        fill: Option<u32>,
        alignment: CellAlignment,
    },
    Appearance {
        padding: u16,
        border_width: u16,
        border_color: u32,
    },
    Delete,
}
impl Table {
    pub fn width(&self) -> u32 {
        self.columns.iter().fold(0u32, |sum,w|sum.saturating_add(*w))
    }
    pub fn cell(&self, row: usize, column: usize) -> Option<&TableCell> {
        let (r, c) = self.origin(row, column)?;
        self.rows.get(r)?.cells.get(c)
    }
    pub fn cells(&self) -> impl Iterator<Item = &TableCell> {
        self.rows.iter().flat_map(|r| &r.cells)
    }
    pub(crate) fn cells_mut(&mut self) -> impl Iterator<Item = &mut TableCell> {
        self.rows.iter_mut().flat_map(|r| &mut r.cells)
    }
}
impl Document {
    pub fn table(&self, id: Id) -> Option<&Table> {
        self.tables.iter().find(|t| t.id == id)
    }
    pub fn cell(&self, id: Id) -> Option<&TableCell> {
        self.tables
            .iter()
            .flat_map(Table::cells)
            .find(|c| c.id == id && !c.is_covered())
    }
    /// Table, row and column owning a paragraph; cells never share a text-selection story.
    pub fn table_location(&self, paragraph: Id) -> Option<(Id, usize, usize)> {
        for t in &self.tables {
            for (r, row) in t.rows.iter().enumerate() {
                for (c, cell) in row.cells.iter().enumerate() {
                    if cell.paragraphs.iter().any(|p| p.id == paragraph) {
                        return Some((t.id, r, c));
                    }
                }
            }
        }
        None
    }
    pub fn cell_for_paragraph(&self, paragraph: Id) -> Option<&TableCell> {
        let (t, r, c) = self.table_location(paragraph)?;
        self.table(t)?.cell(r, c)
    }
    /// Physical horizontal text bounds in twips. Cell padding is part of its text origin.
    pub fn paragraph_bounds(&self, paragraph: Id) -> (u32, u32) {
        if let Some((id, row, column)) = self.table_location(paragraph) {
            let t = self.table(id).unwrap();
            let origin=self.table_x(t).unwrap_or(self.page.left);
            let left=origin.saturating_add(t.columns.get(..column).unwrap_or(&[]).iter().fold(0u32,|a,w|a.saturating_add(*w)));
            (left.saturating_add(u32::from(t.padding)),left.saturating_add(t.cell_width(row,column).unwrap_or(0)).saturating_sub(u32::from(t.padding)))
        } else {
            (self.page.left, self.page.width - self.page.right)
        }
    }
    /// Reading order, excluding note bodies. Tables precede their main-body anchor.
    pub fn body_paragraphs(&self) -> impl Iterator<Item = &Paragraph> {
        self.reading_paragraphs().into_iter()
    }
    /// One cell, one note, or a contiguous main-body segment bounded by tables.
    pub fn paragraph_group(&self, id: Id) -> Option<&[Paragraph]> {
        if let Some(n) = self.note_for_paragraph(id) { return Some(&n.paragraphs); }
        let ps = self.cell_for_paragraph(id).map_or(self.paragraphs.as_slice(), |c| c.paragraphs.as_slice());
        let at = ps.iter().position(|p| p.id == id)?;
        let boundary = |p: &Paragraph| self.tables.iter().any(|t| t.before == p.id);
        let start = ps[..=at].iter().rposition(boundary).unwrap_or(0);
        let end = ps[at + 1..].iter().position(boundary).map_or(ps.len(), |i| at + 1 + i);
        Some(&ps[start..end])
    }

    pub(crate) fn validate_tables(&self, ids: &mut BTreeSet<Id>) -> Result<(), Error> {
        if self.tables.len() > 128 {
            return Err(Error::Limit);
        }
        for t in &self.tables {
            self.table_depth(t.id)?;
            self.validate_table_position(t)?;
            let (left, right) = self.table_bounds(t.id)?;
            if t.id == 0
                || t.id >= 0x7fff_ffff
                || !ids.insert(t.id)
                || right <= left
                || t.columns.is_empty()
                || t.columns.len() > 32
                || t.rows.is_empty()
                || t.rows.len() > 256
                || t.padding > 720
                || t.border_width > 80
                || t.border_color > 0xffffff
                || t.columns
                    .iter()
                    .any(|w| *w == 0 || *w > 31680)
                || (t.position.is_none() && t.width() > right - left)
                || t.anchors().any(|(r,c,_)|t.cell_width(r,c).unwrap_or(0)<u32::from(t.padding)*2+240)
            {
                return Err(Error::Invalid);
            }
            t.validate_grid()?;
            if usize::from(t.header_rows) > t.rows.len() || t.anchors().any(|(r, _, c)| r < usize::from(t.header_rows) && r + usize::from(c.row_span) > usize::from(t.header_rows)) { return Err(Error::Invalid); }
            if t.style.as_deref().is_some_and(|id| self.table_style(id).is_none()) { return Err(Error::Missing); }
            if !t.border_override && (t.style.is_none() || t.border_width != 4 || t.border_color != 0x808080) { return Err(Error::Invalid); }
            for row in &t.rows {
                if row.cells.len() != t.columns.len() || row.min_height > 31680
                    || (row.height_rule == RowHeightRule::Exact && row.min_height == 0)
                {
                    return Err(Error::Invalid);
                }
                for (i, c) in row.cells.iter().enumerate() {
                    for border in c.borders.iter().flatten(){border.validate()?;}
                    if c.id == 0
                        || c.id >= 0x7fff_ffff
                        || !ids.insert(c.id)
                        || (c.row_span > 0 && c.paragraphs.is_empty())
                        || c.fill.is_some_and(|v| v > 0xffffff)
                    {
                        return Err(Error::Invalid);
                    }
                    for p in &c.paragraphs {
                        if p.content
                            .iter()
                            .any(|n| matches!(n, Inline::Bibliography))
                        {
                            return Err(Error::Invalid);
                        }
                        let f = self.resolve_paragraph(p);
                        let first = i64::from(f.left) + i64::from(f.first_line);
                        if first < 0
                            || first + i64::from(f.right) + i64::from(t.padding) * 2 + 120
                                >= i64::from(
                                    t.columns[i..i + usize::from(c.column_span)]
                                        .iter()
                                        .sum::<u32>(),
                                )
                        {
                            return Err(Error::Invalid);
                        }
                        if u32::from(f.left) + u32::from(f.right) + u32::from(t.padding) * 2 + 120
                            >= t.columns[i..i + usize::from(c.column_span)]
                                .iter()
                                .sum::<u32>()
                        {
                            return Err(Error::Invalid);
                        }
                    }
                }
            }
        }
        Ok(())
    }
    pub(crate) fn insert_table(
        &mut self,
        paragraph: Id,
        offset: usize,
        rows: usize,
        columns: usize,
        allocate: &mut impl FnMut() -> Result<Id, Error>,
    ) -> Result<Id, Error> {
        if !(1..=256).contains(&rows)
            || !(1..=32).contains(&columns)
            || self.note_for_paragraph(paragraph).is_some()
            || self.paragraph(paragraph).is_none()
        {
            return Err(Error::Invalid);
        }
        let before = if offset == 0 {
            paragraph
        } else {
            let pos = self.text_projection(paragraph)?.position(offset)?;
            let tail = allocate()?;
            self.split_paragraph(paragraph, pos.inline, pos.offset, pos.offset, tail)?;
            tail
        };
        let id = allocate()?;
        let (left, right) = self.paragraph_bounds(paragraph);
        let width = (right - left) / columns as u32;
        let mut t = Table {
            id,
            before,
            position: None,
            header_rows: 0, style: None, look: crate::TableLook::default(), border_override: true,
            columns: vec![width; columns],
            rows: Vec::new(),
            padding: 90,
            border_width: 4,
            border_color: 0x808080,
        };
        for _ in 0..rows {
            t.rows.push(new_row(columns, allocate)?);
        }
        self.tables.push(t);
        self.sort_tables();
        Ok(id)
    }
    pub(crate) fn change_table(
        &mut self,
        id: Id,
        change: TableChange,
        allocate: &mut impl FnMut() -> Result<Id, Error>,
    ) -> Result<(), Error> {
        let i = self
            .tables
            .iter()
            .position(|t| t.id == id)
            .ok_or(Error::Missing)?;
        if matches!(change, TableChange::Delete) {
            self.tables.remove(i);
            self.prune_tables();
            return Ok(());
        }
        let t = &mut self.tables[i];
        match change {
            TableChange::Position(position)=>t.position=position,
            TableChange::Borders{row,column,borders}=>{let(r,c)=t.origin(row,column).ok_or(Error::Missing)?;t.rows[r].cells[c].borders=borders;},
            TableChange::HeaderRows(rows) => t.header_rows = rows,
            TableChange::Look(look) => t.look = look,
            TableChange::Style { style, look } => {
                t.border_override = style.is_none(); t.border_width = 4; t.border_color = 0x808080; t.style = style; t.look = look;
            },
            TableChange::Merge {
                row,
                column,
                rows,
                columns,
            } => t.merge(row, column, rows, columns)?,
            TableChange::Split { row, column } => t.split(row, column, allocate)?,
            TableChange::Properties {
                row,
                column,
                width,
                height,
                height_rule,
                cant_split,
                fill,
                alignment,
            } => {
                let (origin_r, origin_c) = t.origin(row, column).ok_or(Error::Missing)?;
                let span = usize::from(t.rows[origin_r].cells[origin_c].column_span);
                let old = t.cell_width(row, column).ok_or(Error::Missing)?;
                let mut used = 0;
                for c in origin_c..origin_c + span {
                    let w = (u64::from(width) * u64::from(t.columns[c]) / u64::from(old)) as u32;
                    t.columns[c] = w;
                    used += w;
                }
                t.columns[origin_c + span - 1] += width - used;
                let (r, c) = t.origin(row, column).ok_or(Error::Missing)?;
                t.rows.get_mut(row).ok_or(Error::Missing)?.min_height = height;
                t.rows[row].height_rule = height_rule;
                t.rows[row].cant_split = cant_split;
                let cell = &mut t.rows[r].cells[c];
                cell.fill = fill;
                cell.alignment = alignment;
            }
            TableChange::InsertRow(at) => {
                if at > t.rows.len() || t.rows.len() == 256 {
                    return Err(Error::Limit);
                }
                t.reshape(true, true, at, allocate)?;
                if at < usize::from(t.header_rows) { t.header_rows += 1; }
            }
            TableChange::RemoveRow(at) => {
                if at >= t.rows.len() || t.rows.len() == 1 {
                    return Err(Error::Invalid);
                }
                t.reshape(true, false, at, allocate)?;
                if at < usize::from(t.header_rows) { t.header_rows -= 1; }
            }
            TableChange::InsertColumn(at) => {
                if at > t.columns.len() || t.columns.len() == 32 {
                    return Err(Error::Limit);
                }
                t.reshape(false, true, at, allocate)?;
            }
            TableChange::RemoveColumn(at) => {
                if at >= t.columns.len() || t.columns.len() == 1 {
                    return Err(Error::Invalid);
                }
                t.reshape(false, false, at, allocate)?;
            }
            TableChange::Columns(widths) => {
                if widths.len() != t.columns.len() {
                    return Err(Error::Invalid);
                }
                t.columns = widths;
            }
            TableChange::RowHeight { row, height } => {
                t.rows.get_mut(row).ok_or(Error::Missing)?.min_height = height
            }
            TableChange::RowSizing { row, height, rule } => {
                let row = t.rows.get_mut(row).ok_or(Error::Missing)?;
                row.min_height = height;
                row.height_rule = rule;
            }
            TableChange::RowBreak { row, allow_split } => {
                t.rows.get_mut(row).ok_or(Error::Missing)?.cant_split = !allow_split;
            }
            TableChange::Cell {
                row,
                column,
                fill,
                alignment,
            } => {
                let (r, c) = t.origin(row, column).ok_or(Error::Missing)?;
                let c = &mut t.rows[r].cells[c];
                c.fill = fill;
                c.alignment = alignment;
            }
            TableChange::Appearance {
                padding,
                border_width,
                border_color,
            } => {
                t.border_override = true;
                t.padding = padding;
                t.border_width = border_width;
                t.border_color = border_color;
            }
            TableChange::Delete => (),
        }
        self.prune_tables();
        self.sort_tables();
        Ok(())
    }
}
fn new_cell(allocate: &mut impl FnMut() -> Result<Id, Error>) -> Result<TableCell, Error> {
    Ok(TableCell {
        id: allocate()?,
        row_span: 1,
        column_span: 1,
        paragraphs: vec![Paragraph {
            id: allocate()?,
            style: ParagraphStyle::Normal,
            format: ParagraphFormat::default(),
            content: vec![Inline::Text(String::new())],
        }],
        fill: None,
        alignment: CellAlignment::Top,
        borders: [None;4],
    })
}
fn new_row(
    columns: usize,
    allocate: &mut impl FnMut() -> Result<Id, Error>,
) -> Result<TableRow, Error> {
    let mut cells = Vec::new();
    for _ in 0..columns {
        cells.push(new_cell(allocate)?);
    }
    Ok(TableRow {
        cells,
        min_height: 0,
        height_rule: RowHeightRule::AtLeast,
        cant_split: false,
        grid_before:0,grid_after:0,
    })
}
