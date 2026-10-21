//! Validated grid spans and structural transformations. Covered slots own no text.
use super::*;
impl TableCell {
    pub fn is_covered(&self) -> bool {
        self.row_span == 0
    }
    fn cover(&mut self) {
        self.row_span = 0;
        self.column_span = 0;
        self.paragraphs.clear();
        self.fill = None;
        self.alignment = CellAlignment::Top;
        self.borders = [None;4];
    }
}
impl Table {
    pub fn omitted(&self,row:usize,column:usize)->bool{
        self.rows.get(row).is_some_and(|r|column<usize::from(r.grid_before) || column>=self.columns.len().saturating_sub(usize::from(r.grid_after)))
    }
    /// Visible cells, in row-major order, together with their grid origins.
    pub fn anchors(&self) -> impl Iterator<Item = (usize, usize, &TableCell)> {
        self.rows.iter().enumerate().flat_map(|(r, row)| {
            row.cells
                .iter()
                .enumerate()
                .filter(|(_, c)| !c.is_covered())
                .map(move |(c, cell)| (r, c, cell))
        })
    }
    /// Resolve any covered grid position to the cell that owns it.
    pub fn origin(&self, row: usize, column: usize) -> Option<(usize, usize)> {
        if row >= self.rows.len() || column >= self.columns.len() {
            return None;
        }
        self.anchors()
            .find(|(r, c, cell)| {
                *r <= row
                    && *c <= column
                    && row - *r < usize::from(cell.row_span)
                    && column - *c < usize::from(cell.column_span)
            })
            .map(|(r, c, _)| (r, c))
    }
    pub fn cell_width(&self, row: usize, column: usize) -> Option<u32> {
        let (r, c) = self.origin(row, column)?;
        Some(
            self.columns
                .get(c..c + usize::from(self.rows[r].cells[c].column_span))?
                .iter()
                .fold(0u32, |a,w|a.saturating_add(*w)),
        )
    }
    pub(super) fn validate_grid(&self) -> Result<(), Error> {
        let nr = self.rows.len();
        let nc = self.columns.len();
        let mut occupied = vec![false; nr * nc];
        if self.rows.iter().any(|r| r.cells.len() != nc) {
            return Err(Error::Invalid);
        }
        for (r,row) in self.rows.iter().enumerate(){
            if usize::from(row.grid_before)+usize::from(row.grid_after)>=nc{return Err(Error::Invalid);}
            for c in 0..nc{if self.omitted(r,c){if !row.cells[c].is_covered(){return Err(Error::Invalid);}occupied[r*nc+c]=true;}}
        }
        for (r, c, cell) in self.anchors() {
            let re = r + usize::from(cell.row_span);
            let ce = c + usize::from(cell.column_span);
            if re > nr || ce > nc || ce == c {
                return Err(Error::Invalid);
            }
            for y in r..re {
                for x in c..ce {
                    if occupied[y * nc + x] {
                        return Err(Error::Invalid);
                    }
                    occupied[y * nc + x] = true;
                    if (y, x) != (r, c) && !self.rows[y].cells[x].is_covered() {
                        return Err(Error::Invalid);
                    }
                }
            }
        }
        for (i, cell) in self.cells().enumerate() {
            if !occupied[i]
                || (cell.is_covered()
                    && (cell.column_span != 0
                        || !cell.paragraphs.is_empty()
                        || cell.fill.is_some()
                        || cell.borders != [None;4]
                        || cell.alignment != CellAlignment::Top))
            {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
    pub(super) fn merge(
        &mut self,
        row: usize,
        column: usize,
        rows: usize,
        columns: usize,
    ) -> Result<(), Error> {
        let re = row.checked_add(rows).ok_or(Error::Invalid)?;
        let ce = column.checked_add(columns).ok_or(Error::Invalid)?;
        if rows == 0
            || columns == 0
            || re > self.rows.len()
            || ce > self.columns.len()
            || rows * columns == 1
        {
            return Err(Error::Invalid);
        }
        if (row..re).any(|r|(column..ce).any(|c|self.omitted(r,c))){return Err(Error::Invalid);}
        for (r, c, cell) in self.anchors() {
            let end_r = r + usize::from(cell.row_span);
            let end_c = c + usize::from(cell.column_span);
            if r < re
                && end_r > row
                && c < ce
                && end_c > column
                && (r < row || c < column || end_r > re || end_c > ce)
            {
                return Err(Error::Invalid);
            }
        }
        let mut paragraphs = Vec::new();
        let fill = self.rows[row].cells[column].fill;
        let alignment = self.rows[row].cells[column].alignment;
        // A single merged edge cannot represent mixed properties along its length.
        // Keep each leading outer edge; covered interior borders cease to exist.
        let borders=[self.cell(row,column).unwrap().borders[0],self.cell(row,ce-1).unwrap().borders[1],self.cell(re-1,column).unwrap().borders[2],self.cell(row,column).unwrap().borders[3]];
        for y in row..re {
            for x in column..ce {
                let cell = &mut self.rows[y].cells[x];
                paragraphs.append(&mut cell.paragraphs);
                cell.cover();
            }
        }
        let cell = &mut self.rows[row].cells[column];
        cell.row_span = rows as u16;
        cell.column_span = columns as u16;
        cell.paragraphs = paragraphs;
        cell.fill = fill;
        cell.alignment = alignment;
        cell.borders = borders;
        Ok(())
    }
    pub(super) fn split(
        &mut self,
        row: usize,
        column: usize,
        allocate: &mut impl FnMut() -> Result<Id, Error>,
    ) -> Result<(), Error> {
        let (r, c) = self.origin(row, column).ok_or(Error::Missing)?;
        let (rs, cs) = (
            self.rows[r].cells[c].row_span,
            self.rows[r].cells[c].column_span,
        );
        if rs == 1 && cs == 1 {
            return Err(Error::Invalid);
        }
        let fill = self.rows[r].cells[c].fill;
        let alignment = self.rows[r].cells[c].alignment;
        let borders=self.rows[r].cells[c].borders;
        for y in r..r + usize::from(rs) {
            for x in c..c + usize::from(cs) {
                let cell = &mut self.rows[y].cells[x];
                cell.row_span = 1;
                cell.column_span = 1;
                cell.fill = fill;
                cell.alignment = alignment;
                cell.borders=[if y==r{borders[0]}else{None},if x+1==c+usize::from(cs){borders[1]}else{None},if y+1==r+usize::from(rs){borders[2]}else{None},if x==c{borders[3]}else{None}];
                if (y, x) != (r, c) {
                    cell.paragraphs = blank_paragraph(allocate)?;
                }
            }
        }
        Ok(())
    }
    pub(super) fn reshape(
        &mut self,
        rows: bool,
        insert: bool,
        at: usize,
        allocate: &mut impl FnMut() -> Result<Id, Error>,
    ) -> Result<(), Error> {
        let anchors: Vec<_> = self
            .anchors()
            .map(|(r, c, cell)| (r, c, cell.clone()))
            .collect();
        if rows {
            if insert {
                self.rows.insert(at, new_row(self.columns.len(), allocate)?);
            } else {
                self.rows.remove(at);
            }
        } else {
            let total = self.width();
            let old_nc=self.columns.len();
            if insert {
                self.columns.insert(at, 0);
            } else {
                self.columns.remove(at);
            }
            let w = total / self.columns.len() as u32;
            self.columns.fill(w);
            for row in &mut self.rows {
                if at<usize::from(row.grid_before){if insert{row.grid_before+=1;}else{row.grid_before-=1;}}
                else if at>=old_nc-usize::from(row.grid_after){if insert{row.grid_after+=1;}else{row.grid_after-=1;}}
                if insert {
                    row.cells.insert(at, new_cell(allocate)?);
                } else {
                    row.cells.remove(at);
                }
            }
        }
        for cell in self.cells_mut() {
            cell.cover();
        }
        let nc = self.columns.len();
        let mut occupied = vec![false; self.rows.len() * nc];
        for (r,row) in self.rows.iter().enumerate(){
            if usize::from(row.grid_before)+usize::from(row.grid_after)>=nc{return Err(Error::Invalid);}
            for c in 0..nc{if self.omitted(r,c){occupied[r*nc+c]=true;}}
        }
        for (mut r, mut c, mut cell) in anchors {
            let (pos, span) = if rows {
                (&mut r, &mut cell.row_span)
            } else {
                (&mut c, &mut cell.column_span)
            };
            if insert {
                if *pos >= at {
                    *pos += 1;
                } else if *pos + usize::from(*span) > at {
                    *span += 1;
                }
            } else if *pos > at {
                *pos -= 1;
            } else if *pos + usize::from(*span) > at {
                *span -= 1;
            }
            if *span == 0 {
                continue;
            }
            for y in r..r + usize::from(cell.row_span) {
                for x in c..c + usize::from(cell.column_span) {
                    occupied[y * nc + x] = true;
                }
            }
            self.rows[r].cells[c] = cell;
        }
        for (i, cell) in self.cells_mut().enumerate() {
            if !occupied[i] {
                cell.row_span = 1;
                cell.column_span = 1;
                cell.paragraphs = blank_paragraph(allocate)?;
            }
        }
        Ok(())
    }
}
fn blank_paragraph(
    allocate: &mut impl FnMut() -> Result<Id, Error>,
) -> Result<Vec<Paragraph>, Error> {
    Ok(vec![Paragraph {
        id: allocate()?,
        style: ParagraphStyle::Normal,
        format: ParagraphFormat::default(),
        content: vec![Inline::Text(String::new())],
    }])
}
