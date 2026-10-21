//! RTF row definitions and nested table destinations, shared by native and WASM.
use super::*;
use crate::import_blocks::{Block,InputCell,InputTable};
use doc_core::{BorderSide,BorderStyle,CellAlignment,CellBorder,CellBorders};
#[derive(Clone,Default)]
struct CellDef { edge:u32, horizontal:u8, vertical:u8, fill:Option<u32>, alignment:CellAlignment, borders:CellBorders }
#[derive(Default)]
struct Row { defs:Vec<CellDef>, cells:Vec<Vec<Block>>, height:u32, height_rule:doc_core::RowHeightRule, header:bool, cant_split:bool, padding:u16, left:i32, position:Option<doc_core::TablePosition> }
struct Frame { rows:Vec<Row>, row:Row, cell:Vec<Block>, def:CellDef, side:Option<usize> }
impl Default for Frame {fn default()->Self{Self{rows:Vec::new(),row:Row{padding:90,..Default::default()},cell:Vec::new(),def:CellDef::default(),side:None}}}
#[derive(Default)]
pub(super) struct Tables { frames:Vec<Frame>, blocks:Vec<Block>, paragraphs:usize }
fn has_text(blocks:&[Block])->bool {blocks.iter().any(|b|match b{Block::Table(_)=>true,Block::Paragraph(p)=>p.content.iter().any(|i|i.text().is_none_or(|s|!s.is_empty()))})}
impl Tables {
    fn level(&mut self,depth:usize)->Result<(),Error>{
        if depth>doc_core::MAX_TABLE_DEPTH{return Err(Error::Limit);}
        while self.frames.len()>depth {let f=self.frames.pop().unwrap();if !f.row.cells.is_empty() || !f.cell.is_empty() || !f.row.defs.is_empty(){return Err(Error::Invalid);}
            let t=convert(f.rows)?;let block=Block::Table(t);if let Some(parent)=self.frames.last_mut(){parent.cell.push(block);}else{self.blocks.push(block);}
        }
        while self.frames.len()<depth {self.frames.push(Frame::default());}
        Ok(())
    }
    pub(super) fn paragraph(&mut self,depth:usize,p:Paragraph)->Result<(),Error>{self.paragraphs+=1;if self.paragraphs>2048{return Err(Error::Limit);}self.level(depth)?;
        if let Some(f)=self.frames.last_mut(){f.cell.push(Block::Paragraph(p));}else{self.blocks.push(Block::Paragraph(p));}Ok(())
    }
    pub(super) fn cell_end(&mut self,depth:usize)->Result<(),Error>{if depth==0{return Err(Error::Invalid);}self.level(depth)?;let f=self.frames.last_mut().unwrap();if f.row.cells.len()>=32{return Err(Error::Limit);}
        f.row.cells.push(core::mem::take(&mut f.cell));Ok(())
    }
    fn row_end(&mut self,depth:usize)->Result<(),Error>{self.level(depth)?;let f=self.frames.last_mut().ok_or(Error::Invalid)?;
        if !f.cell.is_empty() || f.row.cells.is_empty() || f.row.cells.len()!=f.row.defs.len(){return Err(Error::Invalid);}
        if f.rows.len()==256{return Err(Error::Limit);}f.rows.push(core::mem::take(&mut f.row));f.row.padding=90;f.def=CellDef::default();f.side=None;Ok(())
    }
    pub(super) fn finish(mut self)->Result<Document,Error>{self.level(0)?;crate::import_blocks::document(self.blocks)}
    pub(super) fn control(&mut self,depth:usize,name:&str,value:Option<i32>,colors:&[TextColor])->Result<bool,Error>{
        let known=matches!(name,"trowd"|"trgaph"|"trleft"|"trrh"|"trkeep"|"trhdr"|"trql"|"cellx"|"clmgf"|"clmrg"|"clvmgf"|"clvmrg"|"clcbpat"|"clvertalt"|"clvertalc"|"clvertalb"|"clbrdrt"|"clbrdrr"|"clbrdrb"|"clbrdrl"|"brdrs"|"brdrdb"|"brdrdot"|"brdrdash"|"brdrnil"|"brdrnone"|"brdrw"|"brdrcf"|"row"|"nestrow"|"cltxlrtb"|"clftsWidth"|"clwWidth"|"trftsWidth"|"trwWidth"|"trautofit"|"trpaddl"|"trpaddr"|"trpaddt"|"trpaddb"|"trpaddfl"|"trpaddfr"|"trpaddft"|"trpaddfb");
        let floating=matches!(name,"tphpg"|"tphmrg"|"tphcol"|"tpvpg"|"tpvmrg"|"tpvpara"|"tposx"|"tposy"|"tposnegx"|"tposnegy"|"tdfrmtxtTop"|"tdfrmtxtRight"|"tdfrmtxtBottom"|"tdfrmtxtLeft"|"tabsnoovrlp");
        if !known&&!floating{return Ok(false);}let depth=if name=="row"{1}else{depth.max(1)};
        if name=="nestrow"&&depth<2{return Err(Error::Invalid);}if matches!(name,"row"|"nestrow"){self.row_end(depth)?;return Ok(true);}
        self.level(depth)?;let f=self.frames.last_mut().unwrap();let n=value.unwrap_or(1);
        let required=||value.ok_or(Error::Invalid);let rgb=|index:i32|->Result<Option<u32>,Error>{if index==0{return Ok(None);}match colors.get(usize::try_from(index).map_err(|_|Error::Invalid)?).ok_or(Error::Invalid)?{TextColor::Auto=>Ok(None),TextColor::Rgb(v)=>Ok(Some(*v))}};
        if floating {
            use doc_core::{TablePosition,TableHorizontal as H,TableVertical as V};
            let p=f.row.position.get_or_insert(TablePosition{distance:[0;4],horizontal:H::Column,overlap:true,..Default::default()});
            match name {
                "tphpg"=>p.horizontal=H::Page,"tphmrg"=>p.horizontal=H::Margin,"tphcol"=>p.horizontal=H::Column,
                "tpvpg"=>p.vertical=V::Page,"tpvmrg"=>p.vertical=V::Margin,"tpvpara"=>p.vertical=V::Paragraph,
                "tposx"|"tposnegx"=>p.x=required()?,"tposy"|"tposnegy"=>p.y=required()?,
                "tabsnoovrlp"=>p.overlap=!toggle(n)?,
                _=>{let side=match name{"tdfrmtxtTop"=>0,"tdfrmtxtRight"=>1,"tdfrmtxtBottom"=>2,_=>3};p.distance[side]=u16::try_from(required()?).map_err(|_|Error::Invalid)?;},
            }return Ok(true);
        }
        match name {
            "trowd"=>{f.row.defs.clear();f.row.height=0;f.row.height_rule=doc_core::RowHeightRule::AtLeast;f.row.header=false;f.row.cant_split=false;f.row.padding=90;f.row.left=0;f.row.position=None;f.def=CellDef::default();f.side=None;},
            "trgaph"=>f.row.padding=u16::try_from(required()?).map_err(|_|Error::Invalid)?,
            "trleft"=>f.row.left=required()?,"trrh"=>{let h=required()?;f.row.height=h.unsigned_abs();f.row.height_rule=if h<0{doc_core::RowHeightRule::Exact}else{doc_core::RowHeightRule::AtLeast};},
            "trhdr"=>f.row.header=toggle(n)?,"trql"=>(),
            "cellx"=>{let edge=required()?.checked_sub(f.row.left).filter(|n|*n>0&&*n<=31680).ok_or(Error::Invalid)? as u32;if f.row.defs.len()==32 || f.row.defs.last().is_some_and(|c|c.edge>=edge){return Err(Error::Invalid);}f.def.edge=edge;f.row.defs.push(core::mem::take(&mut f.def));f.side=None;},
            "trkeep"=>{if !matches!(value,None|Some(0)|Some(1)){return Err(Error::Invalid);}f.row.cant_split=value!=Some(0);},
            "clmgf"=>f.def.horizontal=1,"clmrg"=>f.def.horizontal=2,"clvmgf"=>f.def.vertical=1,"clvmrg"=>f.def.vertical=2,
            "clcbpat"=>f.def.fill=rgb(required()?)?,
            "clvertalt"=>f.def.alignment=CellAlignment::Top,"clvertalc"=>f.def.alignment=CellAlignment::Center,"clvertalb"=>f.def.alignment=CellAlignment::Bottom,
            "clbrdrt"|"clbrdrr"|"clbrdrb"|"clbrdrl"=>{let side=match name{"clbrdrt"=>BorderSide::Top,"clbrdrr"=>BorderSide::Right,"clbrdrb"=>BorderSide::Bottom,_=>BorderSide::Left} as usize;f.side=Some(side);f.def.borders[side]=Some(CellBorder{style:BorderStyle::Single,width:4,color:0});},
            "brdrs"|"brdrdb"|"brdrdot"|"brdrdash"|"brdrnil"|"brdrnone"|"brdrw"|"brdrcf"=>{
                let b=f.def.borders[f.side.ok_or_else(||unsupported("non-cell border"))?].as_mut().ok_or(Error::Invalid)?;
                match name{"brdrw"=>{let w=required()?;if w<0||w>240{return Err(Error::Invalid);}b.width=((w as u32*2+2)/5) as u16;},"brdrcf"=>b.color=rgb(required()?)?.unwrap_or(0),_=>{b.style=match name{"brdrdb"=>BorderStyle::Double,"brdrdot"=>BorderStyle::Dotted,"brdrdash"=>BorderStyle::Dashed,"brdrnil"=>BorderStyle::Nil,"brdrnone"=>BorderStyle::None,_=>BorderStyle::Single};if !b.visible(){b.width=0;b.color=0;}}}
            },
            "trpaddl"|"trpaddr"|"trpaddt"|"trpaddb"=>{let pad=u16::try_from(required()?).map_err(|_|Error::Invalid)?;if pad!=f.row.padding{return Err(unsupported("nonuniform table padding"));}},
            "trpaddfl"|"trpaddfr"|"trpaddft"|"trpaddfb" if n!=3=>return Err(unsupported("relative cell padding")),
            "trautofit" if n!=0=>return Err(unsupported("RTF autofit")),
            _=>(),
        }Ok(true)
    }
}
fn convert(rows:Vec<Row>)->Result<InputTable,Error>{
    let first=rows.first().ok_or(Error::Invalid)?;
    let left=rows.iter().map(|r|r.left).min().ok_or(Error::Invalid)?;
    let edge=|row:&Row,n:u32|->Result<u32,Error>{u32::try_from(i64::from(row.left)-i64::from(left)+i64::from(n)).ok().filter(|v|*v<=31680).ok_or(Error::Limit)};
    let mut t=InputTable::default();t.padding=first.padding;
    t.position=first.position;
    // Physical boundaries form one common grid. Visible cells span its intervals.
    let mut edges=alloc::collections::BTreeSet::new();edges.insert(0);
    for row in &rows {
        if row.position!=t.position{return Err(unsupported("different floating positions in one RTF table"));}
        if row.padding!=t.padding{return Err(unsupported("different RTF row padding"));}
        edges.insert(edge(row,0)?);
        for def in &row.defs{edges.insert(edge(row,def.edge)?);}
        if edges.len()>33{return Err(Error::Limit);}
    }
    let edges:Vec<u32>=edges.into_iter().collect();let nc=edges.len()-1;
    t.columns=edges.windows(2).map(|w|w[1]-w[0]).collect();
    let mut vertical:Vec<Option<(usize,usize,usize)>>=alloc::vec![None;nc];
    for (r,row) in rows.into_iter().enumerate(){
        let origin=edge(&row,0)?;let last=edge(&row,row.defs.last().ok_or(Error::Invalid)?.edge)?;
        t.gaps.push((edges.binary_search(&origin).map_err(|_|Error::Invalid)?,nc-edges.binary_search(&last).map_err(|_|Error::Invalid)?));
        if row.header{if t.headers!=r{return Err(unsupported("nonleading RTF header"));}t.headers+=1;}
        let mut out:Vec<InputCell>=Vec::new();let mut cells=row.cells.into_iter();let mut col=0;let mut next=alloc::vec![None;nc];
        while col<row.defs.len() {
            let def=&row.defs[col];if def.horizontal==2{return Err(Error::Invalid);}
            let mut count=1;if def.horizontal==1{while col+count<row.defs.len() && row.defs[col+count].horizontal==2{count+=1;}}
            let start=origin+if col==0{0}else{row.defs[col-1].edge};let end=origin+row.defs[col+count-1].edge;
            let gc=edges.binary_search(&start).map_err(|_|Error::Invalid)?;
            let span=edges.binary_search(&end).map_err(|_|Error::Invalid)?-gc;
            let blocks=cells.next().ok_or(Error::Invalid)?;
            for i in 1..count{if has_text(&cells.next().ok_or(Error::Invalid)?){return Err(unsupported("content in merged continuation"));}
                if row.defs[col+i].vertical!=def.vertical{return Err(unsupported("inconsistent merged cell definition"));}}
            if def.vertical==2 {let (ar,ac,expected)=vertical[gc].ok_or(Error::Invalid)?;if expected!=span||has_text(&blocks){return Err(Error::Invalid);}t.rows[ar][ac].rows+=1;next[gc]=Some((ar,ac,span));}
            else {let index=out.len();out.push(InputCell{blocks,rows:1,columns:span,fill:def.fill,alignment:def.alignment,borders:def.borders});if def.vertical==1{next[gc]=Some((r,index,span));}}
            col+=count;
        }
        t.rows.push(out);t.heights.push(row.height);t.height_rules.push(row.height_rule);t.cant_split.push(row.cant_split);vertical=next;
    }Ok(t)
}
