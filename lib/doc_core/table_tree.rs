//! Bounded table containment. Tables keep stable identities in one registry; their
//! anchor paragraph determines the enclosing body or cell story.
use crate::{Document, Error, Id, Paragraph, Table};
use alloc::{collections::BTreeSet, vec::Vec};
/// Maximum table nesting, including the outermost table.
pub const MAX_TABLE_DEPTH: usize = 8;
impl Document {
    /// Immediate enclosing table and cell coordinates, or None for the main story.
    pub fn table_parent(&self, id: Id) -> Option<(Id, usize, usize)> {
        self.table_location(self.table(id)?.before)
    }
    /// One-based containment depth. Rejects orphan anchors and cycles, even before validation.
    pub fn table_depth(&self, id: Id) -> Result<usize, Error> {
        let mut current = id;
        let mut seen = BTreeSet::new();
        for depth in 1..=MAX_TABLE_DEPTH {
            if !seen.insert(current) { return Err(Error::Invalid); }
            let t = self.table(current).ok_or(Error::Missing)?;
            if self.paragraphs.iter().any(|p| p.id == t.before) { return Ok(depth); }
            current = self.table_location(t.before).ok_or(Error::Invalid)?.0;
        }
        Err(Error::Limit)
    }
    /// Horizontal space provided by a table's enclosing story, in document twips.
    pub fn table_bounds(&self, id: Id) -> Result<(u32, u32), Error> {
        self.table_depth(id)?;
        let t = self.table(id).ok_or(Error::Missing)?;
        let mut parent=self.table_location(t.before);let mut left=self.page.left;
        let mut width=self.page.width.saturating_sub(self.page.right).saturating_sub(left);let mut first=true;
        while let Some((id,row,column))=parent {
            let outer=self.table(id).ok_or(Error::Missing)?;
            let pad=u32::from(outer.padding);
            if first {width=outer.cell_width(row,column).ok_or(Error::Invalid)?.checked_sub(pad*2).ok_or(Error::Invalid)?;first=false;}
            let columns=outer.columns.get(..column).ok_or(Error::Invalid)?;
            for w in columns{left=left.checked_add(*w).ok_or(Error::Limit)?;}
            left=left.checked_add(pad).ok_or(Error::Limit)?;
            if outer.position.is_some(){left=left.checked_sub(self.page.left).and_then(|v|v.checked_add(self.table_x(outer).ok()?)).ok_or(Error::Invalid)?;}
            parent=self.table_location(outer.before);
        }
        Ok((left,left.checked_add(width).ok_or(Error::Limit)?))
    }
    /// Depth-first block order, including tables in cells.
    pub fn ordered_tables(&self) -> Vec<&Table> {
        fn visit<'a>(d: &'a Document, ps: &[Paragraph], depth: usize, out: &mut Vec<&'a Table>) {
            if depth >= MAX_TABLE_DEPTH { return; }
            for p in ps { for t in d.tables.iter().filter(|t| t.before == p.id) {
                out.push(t);
                for c in t.cells() { visit(d, &c.paragraphs, depth + 1, out); }
            }}
        }
        let mut out = Vec::new(); visit(self, &self.paragraphs, 0, &mut out); out
    }
    pub(crate) fn reading_paragraphs(&self) -> Vec<&Paragraph> {
        fn visit<'a>(d: &'a Document, ps: &'a [Paragraph], depth: usize, out: &mut Vec<&'a Paragraph>) {
            for p in ps {
                if depth < MAX_TABLE_DEPTH { for t in d.tables.iter().filter(|t| t.before == p.id) {
                    for c in t.cells() { visit(d, &c.paragraphs, depth + 1, out); }
                }}
                out.push(p);
            }
        }
        let mut out = Vec::new(); visit(self, &self.paragraphs, 0, &mut out); out
    }
    pub(crate) fn sort_tables(&mut self) {
        let ids: Vec<_> = self.ordered_tables().iter().map(|t| t.id).collect();
        self.tables.sort_by_key(|t| ids.iter().position(|id| *id == t.id).unwrap_or(usize::MAX));
    }
    /// Fit oversized clipboard tables to their destination without widening smaller tables.
    /// Descendant widths follow their enclosing cells; impossible minimum widths fail atomically.
    pub(crate) fn fit_tables_to_width(&mut self, width:u32)->Result<(),Error>{
        let ids:Vec<_>=self.ordered_tables().iter().map(|t|t.id).collect();
        for id in ids {
            let available=if self.table_parent(id).is_some(){let (a,b)=self.table_bounds(id)?;b-a}else{width};
            let t=self.tables.iter_mut().find(|t|t.id==id).ok_or(Error::Missing)?;
            let total=t.width();if total<=available{continue;}
            let minimum=u32::from(t.padding)*2+240;let mut edge=0u64;let mut previous=0;
            for w in &mut t.columns {edge+=u64::from(*w);let next=(edge*u64::from(available)/u64::from(total)) as u32;*w=next-previous;previous=next;}
            if t.columns.contains(&0) || t.anchors().any(|(r,c,_)|t.cell_width(r,c).unwrap_or(0)<minimum){return Err(Error::Invalid);}
        }Ok(())
    }
    /// Remove descendants whose anchor was removed by a structural table edit.
    pub(crate) fn prune_tables(&mut self) {
        loop {
            let anchors: BTreeSet<_> = self.paragraphs.iter().chain(self.tables.iter().flat_map(|t| t.cells().flat_map(|c| &c.paragraphs))).map(|p| p.id).collect();
            let count = self.tables.len(); self.tables.retain(|t| anchors.contains(&t.before));
            if count == self.tables.len() { break; }
        }
        self.prune_notes();
    }
}
