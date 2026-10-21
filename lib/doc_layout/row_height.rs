//! Fixed row overflow is a view operation; document stories and their IDs stay intact.
use super::*;

pub(super) fn clip(bottom:i32,lines:&mut Vec<Line>,cells:&mut Vec<CellBox>,regions:&mut Vec<RepeatRegion>,keeps:&mut Vec<Keep>){
    lines.retain_mut(|line|{
        if line.top>=bottom{return false;}
        // The baseline is left intact. A partially visible last line keeps its font
        // metrics, but selection, pagination and hit testing use only visible height.
        line.height=line.height.min(bottom-line.top);
        line.height>0
    });
    cells.retain_mut(|cell|{
        let top=cell.rect[1];let end=(top+cell.rect[3]).min(bottom);
        if end<=top{return false;}
        crate::table_borders::clip(cell,top,end);
        cell.row_end &= end==top+cell.rect[3];
        cell.rect[3]=end-top;
        true
    });
    regions.retain_mut(|r|{r.bottom=r.bottom.min(bottom);r.top<r.bottom});
    keeps.retain_mut(|r|{r.bottom=r.bottom.min(bottom);r.top<r.bottom});
}
