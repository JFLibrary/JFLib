//! Shared, bounded block builder for clipboard import. No host or browser dependency.
use super::*;
use doc_core::{CellAlignment, CellBorders, Table, TableCell, TableRow};
pub(super) enum Block { Paragraph(Paragraph), Table(InputTable) }
pub(super) struct InputCell {
    pub blocks: Vec<Block>, pub rows: usize, pub columns: usize,
    pub fill: Option<u32>, pub alignment: CellAlignment, pub borders: CellBorders,
}
impl Default for InputCell {
    fn default() -> Self { Self {blocks:Vec::new(),rows:1,columns:1,fill:None,alignment:CellAlignment::Top,borders:[None;4]} }
}
pub(super) struct InputTable {
    pub position: Option<doc_core::TablePosition>,
    pub right:Option<i32>,
    pub gaps:Vec<(usize,usize)>,
    pub columns: Vec<u32>, pub width: Option<u32>, pub percent: Option<u32>, pub rows: Vec<Vec<InputCell>>,
    pub heights: Vec<u32>, pub height_rules: Vec<doc_core::RowHeightRule>, pub cant_split: Vec<bool>, pub headers: usize, pub padding: u16,
    pub border_width: u16, pub border_color: u32,
}
impl Default for InputTable {
    fn default() -> Self { Self {position:None,right:None,gaps:Vec::new(),columns:Vec::new(),width:None,percent:None,rows:Vec::new(),heights:Vec::new(),height_rules:Vec::new(),cant_split:Vec::new(),headers:0,padding:90,border_width:0,border_color:0} }
}
pub(super) fn empty() -> Paragraph { Paragraph {id:0,style:ParagraphStyle::Normal,format:ParagraphFormat::default(),content:vec![Inline::Text(String::new())]} }
struct Builder { document: Document, next: Id, paragraphs: usize, tables: usize }
impl Builder {
    fn id(&mut self)->Id {let id=self.next;self.next+=1;id}
    fn story(&mut self, blocks:Vec<Block>, width:u32, depth:usize)->Result<Vec<Paragraph>,Error>{
        let mut paragraphs=Vec::new();let mut pending:Vec<usize>=Vec::new();
        for block in blocks.into_iter().chain(core::iter::once(Block::Paragraph(empty()))) {
            match block {
                Block::Paragraph(mut p)=>{
                    // A terminal sentinel is retained only when a table needs an anchor or the story is empty.
                    self.paragraphs+=1;if self.paragraphs>8192{return Err(Error::Limit);}
                    p.id=self.id();for index in pending.drain(..){self.document.tables[index].before=p.id;}
                    paragraphs.push(p);
                }
                Block::Table(t)=>{pending.push(self.table(t,width,depth+1)?);}
            }
        }
        // Remove the extra sentinel when the previous paragraph already anchors the final block.
        if paragraphs.len()>1 {let last=paragraphs.last().unwrap().id;if !self.document.tables.iter().any(|t|t.before==last){paragraphs.pop();}}
        Ok(paragraphs)
    }
    fn table(&mut self, input:InputTable, space:u32, depth:usize)->Result<usize,Error>{
        self.tables+=1;
        if depth>doc_core::MAX_TABLE_DEPTH || self.tables>128 || input.rows.is_empty() || input.rows.len()>256 || input.padding>720 {return Err(Error::Limit);}
        let count=input.rows.len();let mut occupied=vec![[false;32];count];let mut placements=Vec::new();let mut nc=0;
        for (r,(before,after)) in input.gaps.iter().copied().enumerate(){
            if r>=count || input.columns.len()>32 || before+after>=input.columns.len(){return Err(Error::Invalid);}
            for c in 0..input.columns.len(){if c<before||c>=input.columns.len()-after{occupied[r][c]=true;}}
        }
        for (r,row) in input.rows.into_iter().enumerate(){let mut col=0;
            for c in row {while col<32 && occupied[r][col]{col+=1;}
                if c.rows==0 || c.columns==0 || c.rows>count-r || c.columns>32-col {return Err(Error::Invalid);}
                for line in &mut occupied[r..r+c.rows]{for slot in &mut line[col..col+c.columns]{if *slot{return Err(Error::Invalid);}*slot=true;}}
                nc=nc.max(col+c.columns);let span=c.columns;placements.push((r,col,c));col+=span;
            }
        }
        nc=nc.max(input.columns.len());if nc==0 || nc>32{return Err(Error::Limit);}
        let minimum=u32::from(input.padding)*2+240;
        let reference=if input.position.is_some_and(|p|p.horizontal==doc_core::TableHorizontal::Page){self.document.page.width}else{space};
        let requested=input.percent.map(|v|reference*v/100).or(input.width).unwrap_or_else(||if input.columns.is_empty(){space}else{input.columns.iter().sum()});
        let total=if input.position.is_some(){requested}else{requested.min(space)};
        let mut position=input.position;
        if let Some(right)=input.right {let p=position.as_mut().ok_or(Error::Invalid)?;let width=if p.horizontal==doc_core::TableHorizontal::Page{self.document.page.width}else{space};p.x=i32::try_from(width).map_err(|_|Error::Limit)?-i32::try_from(total).map_err(|_|Error::Limit)?-right;}
        if total<nc as u32{return Err(unsupported("table does not fit its enclosing cell"));}
        let mut columns=if input.columns.len()==nc {input.columns}else{vec![1;nc]};
        let sum:u32=columns.iter().sum();if sum==0 || columns.contains(&0){return Err(Error::Invalid);}
        if sum!=total {
            // Scale cumulative boundaries, not individual cells: virtual grid segments
            // may be narrower than padding, while every visible cell still must fit.
            let mut edge=0u64;let mut previous=0;
            for w in &mut columns {edge+=u64::from(*w);let next=(edge*u64::from(total)/u64::from(sum)) as u32;*w=next-previous;previous=next;}
            if columns.contains(&0){return Err(unsupported("table grid too fine for enclosing cell"));}
        }
        if placements.iter().any(|(_,col,c)|columns[*col..*col+c.columns].iter().sum::<u32>()<minimum)
            || occupied.iter().any(|row|(0..nc).any(|c|!row[c]&&columns[c]<minimum)){return Err(unsupported("table cell does not fit its enclosing cell"));}
        // Preserve exact declared grids when they already fit, instead of redistributing padding.
        // The proportional path above is only used for automatic or oversized imported grids.
        let id=self.id();let index=self.document.tables.len();
        self.document.tables.push(Table{id,before:0,position,header_rows:input.headers as u16,style:None,look:doc_core::TableLook::default(),border_override:true,columns:columns.clone(),rows:Vec::new(),padding:input.padding,border_width:input.border_width,border_color:input.border_color});
        let mut rows=Vec::new();for r in 0..count {let mut cells=Vec::new();for _ in 0..nc {cells.push(TableCell{id:self.id(),row_span:0,column_span:0,paragraphs:Vec::new(),fill:None,alignment:CellAlignment::Top,borders:[None;4]});}let (before,after)=input.gaps.get(r).copied().unwrap_or((0,0));rows.push(TableRow{cells,cant_split:input.cant_split.get(r).copied().unwrap_or(false),min_height:input.heights.get(r).copied().unwrap_or(0),height_rule:input.height_rules.get(r).copied().unwrap_or_default(),grid_before:before as u16,grid_after:after as u16});}
        for (r,col,c) in placements {let width=columns[col..col+c.columns].iter().sum::<u32>()-u32::from(input.padding)*2;
            let paragraphs=self.story(c.blocks,width,depth)?;let cell=&mut rows[r].cells[col];cell.row_span=c.rows as u16;cell.column_span=c.columns as u16;cell.paragraphs=paragraphs;cell.fill=c.fill;cell.alignment=c.alignment;cell.borders=c.borders;
        }
        for (r,line) in occupied.iter().enumerate(){for c in 0..nc {if !line[c]{rows[r].cells[c].row_span=1;rows[r].cells[c].column_span=1;rows[r].cells[c].paragraphs=self.story(Vec::new(),columns[c]-u32::from(input.padding)*2,depth)?;}}}
        self.document.tables[index].rows=rows;Ok(index)
    }
}
pub(super) fn document(blocks:Vec<Block>)->Result<Document,Error>{
    document_on_page(blocks,PageSettings::default())
}
pub(super) fn document_on_page(blocks:Vec<Block>,page:PageSettings)->Result<Document,Error>{
    page.validate()?;
    let mut b=Builder{document:Document::default(),next:1,paragraphs:0,tables:0};
    b.document.page=page;
    b.document.paragraphs=b.story(blocks,b.document.page.content_width(),0)?;
    b.document.validate()?;Ok(b.document)
}
