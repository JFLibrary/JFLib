//! Explicit, undoable column sizing with the same font metrics as pagination.
use super::*;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableFit {
    Contents,
    Page,
    Equal,
}
fn require(widths: &mut [u32], start: usize, count: usize, total: u32) {
    let current: u32 = widths[start..start + count].iter().sum();
    if total > current {
        let missing = total - current;
        for (i, w) in widths[start..start + count].iter_mut().enumerate() {
            *w += missing / count as u32 + u32::from(i < (missing % count as u32) as usize);
        }
    }
}
/// Compute fixed widths; applying `TableChange::Columns` records the fit as one undo step.
/// Contents measures explicit lines and tabs. It wraps text when natural widths exceed the page.
pub fn table_fit(
    d: &Document,
    id: Id,
    mode: TableFit,
    measure: &mut impl Measure,
) -> Result<Vec<u32>, Error> {
    d.validate().map_err(Error::Model)?;
    let table = d.table(id).ok_or(Error::Model(doc_core::Error::Missing))?;
    let nc = table.columns.len();
    let (left,right)=d.table_bounds(id).map_err(Error::Model)?;
    let page = right-left;
    let pad = u32::from(table.padding) * 2;
    if mode == TableFit::Equal {
        let total = table.width();
        return Ok((0..nc)
            .map(|i| total / nc as u32 + u32::from(i < (total % nc as u32) as usize))
            .collect());
    }
    let mut minimum = vec![1; nc];
    let mut natural = minimum.clone();
    let mut expanded = 0;
    let bibliography = d.bibliography().join("\n");
    // Narrow cells establish their constraints before wider merged cells.
    for span in 1..=nc {
        for (_, column, cell) in table
            .anchors()
            .filter(|(_, _, c)| usize::from(c.column_span) == span)
        {
            require(&mut minimum,column,span,pad+240);
            for p in &cell.paragraphs {
                for child in d.tables.iter().filter(|t|t.before==p.id){
                    require(&mut minimum,column,span,pad+child.width());
                    require(&mut natural,column,span,pad+child.width());
                }
                let pf = d.resolve_paragraph(p);
                let margins = u32::from(pf.left) + u32::from(pf.right);
                require(
                    &mut minimum,
                    column,
                    span,
                    pad + margins + pf.first_line.max(0) as u32 + 121,
                );
                let mut line = 0i64;
                let mut indent = i32::from(pf.first_line);
                let mut widest = 0i64;
                let atoms = paragraph_atoms(d, p, &bibliography, measure, &mut expanded)?;
                for (index, a) in atoms.iter().enumerate() {
                    if a.newline {
                        widest = widest.max(line + i64::from(indent));
                        line = 0;
                        indent = 0;
                    } else if a.tab {
                        let pending = Pending {
                            left:0,gap:0,page_break:false,
                            indent,
                            available: page as i32,
                            width: line as i32,
                            start: 0,
                            end: 0,
                            next: 0,
                            atoms: Vec::new(),
                            hard: false,
                        };
                        line +=
                            i64::from(tabs::advance(&pending, &atoms[index + 1..], pf, measure)?.0);
                    } else {
                        line += i64::from(a.width);
                    }
                    line = line.min(i64::from(page) * 32);
                }
                widest = widest.max(line + i64::from(indent));
                require(
                    &mut natural,
                    column,
                    span,
                    (widest + i64::from(pad + margins)).min(i64::from(page) * 32) as u32,
                );
            }
        }
    }
    for (w, m) in natural.iter_mut().zip(&minimum) {
        *w = (*w).max(*m);
    }
    let low: u32 = minimum.iter().sum();
    if low > page {
        return Err(Error::Model(doc_core::Error::Invalid));
    }
    let wanted: u32 = natural.iter().sum();
    let target = if mode == TableFit::Page {
        page
    } else {
        wanted.min(page)
    };
    let extra = target - low;
    let weight = wanted - low;
    let mut used = 0;
    for i in 0..nc {
        let add = if weight == 0 {
            extra / nc as u32
        } else {
            (u64::from(extra) * u64::from(natural[i] - minimum[i]) / u64::from(weight)) as u32
        };
        minimum[i] += add;
        used += add;
    }
    for i in 0..(extra - used) as usize {
        minimum[i % nc] += 1;
    }
    Ok(minimum)
}
