//! Resolved grid segments in document units, including clipping through page continuations.
use alloc::vec::Vec;
use doc_core::{BorderSide,CellBorder,Document,Table};
use crate::CellBox;
#[derive(Clone,Debug,PartialEq,Eq)]
pub struct CellEdge {pub side:BorderSide,pub offset:i32,pub length:i32,pub border:CellBorder}
pub(super) fn edges(d:&Document,t:&Table,row:usize,column:usize,heights:&[i32])->Vec<CellEdge>{
    let cell=t.cell(row,column).unwrap();let mut out:Vec<CellEdge>=Vec::new();
    for side in BorderSide::ALL {
        let horizontal=matches!(side,BorderSide::Top|BorderSide::Bottom);
        let units=usize::from(if horizontal{cell.column_span}else{cell.row_span});let mut offset=0;
        for unit in 0..units {
            let length=if horizontal{t.columns[column+unit] as i32}else{heights[row+unit]};
            let border=d.shared_cell_border(t,row,column,side,unit);
            if let Some(last)=out.last_mut().filter(|e|e.side==side && e.border==border){last.length+=length;}
            else{out.push(CellEdge{side,offset,length,border});}
            offset+=length;
        }
    }out
}
pub(super) fn clip(cell:&mut CellBox,start:i32,finish:i32){
    let top=cell.rect[1];let height=cell.rect[3];
    cell.edges.retain_mut(|edge|match edge.side {
        BorderSide::Top=>start==top,
        BorderSide::Bottom=>finish==top+height,
        _=>{let a=(top+edge.offset).max(start);let b=(top+edge.offset+edge.length).min(finish);edge.offset=a-start;edge.length=b-a;b>a},
    });
}
