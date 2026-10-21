//! Row keep constraints and line-safe clipping, shared by inline and floating tables.
use super::*;

#[derive(Clone, Copy)]
pub(super) struct Keep { pub top:i32, pub bottom:i32 }

/// Oversized rows may split after reaching a fresh page, or once already continued.
/// Recheck every reduced edge against sibling cells and nested row constraints.
pub(super) fn cut(keeps:&[Keep],lines:&[Line],top:i32,mut bottom:i32,fresh_capacity:i32,fresh_start:Option<i32>)->i32 {
    let requested=bottom;
    loop {
        let old=bottom;
        for r in keeps {
            if r.top>=top && r.top<bottom && bottom<r.bottom
                && (r.bottom-r.top<=fresh_capacity || fresh_start.is_none_or(|at|r.top>at)) {
                bottom=r.top;
            }
        }
        if let Some(at)=lines.iter().filter(|l|l.top<bottom&&l.top+l.height>bottom).map(|l|l.top).min(){bottom=at;}
        if bottom<=top || (fresh_start.is_some() && !lines.iter().any(|l|l.top>=top&&l.top<bottom)) {
            // Independent nested tables in neighbouring cells can have interleaved
            // kept rows with no common legal edge. If their overlapping constraints
            // exceed a whole fresh page, relax that impossible keep group like an
            // oversized row. A merely reduced page (e.g. footnotes) still moves on.
            if let Some(fresh)=fresh_start {
                let mut ranges:Vec<_>=keeps.iter().filter(|r|r.top>=top&&r.bottom>top
                    && (r.bottom-r.top<=fresh_capacity||r.top>fresh)).copied().collect();
                ranges.sort_by_key(|r|r.top);
                let start=bottom.max(top);let mut end=start;
                for r in ranges {if r.top<=start || r.top<end {end=end.max(r.bottom);}else{break;}}
                if end-top>fresh_capacity {
                    bottom=requested;
                    loop {if let Some(at)=lines.iter().filter(|l|l.top<bottom&&l.top+l.height>bottom).map(|l|l.top).min(){bottom=at;}else{break;}}
                }
            }
            return bottom;
        }
        if bottom==old{return bottom;}
    }
}

/// Split one inline band at the remaining page space, retaining source IDs and offsets.
pub(super) fn split(d:&Document,item:&Flow,capacity:i32,fresh_capacity:i32,fresh:bool)->Option<(Flow,Flow)> {
    if item.cells.is_empty() || capacity<=0 || capacity>=item.line.height {return None;}
    let end=cut(&item.keeps,&item.children,0,capacity,fresh_capacity,if fresh{Some(0)}else{None});
    if end<=0 || item.regions.iter().any(|r|{let start=r.top-r.header.height;start>=0&&start<end&&end<=r.top}){return None;}
    let mut repeated:Vec<_>=item.regions.iter().filter(|r|r.top<=end&&end<r.bottom).collect();
    repeated.sort_by_key(|r|d.table_depth(r.header.table).unwrap_or(0));
    let prefix:i32=repeated.iter().map(|r|r.header.height).sum();
    if prefix>=fresh_capacity{return None;}
    let slice=|start:i32,finish:i32,prefix:i32|{
        let mut out=item.clone();out.line.height=finish-start+prefix;
        out.anchors=item.anchors.iter().filter(|a|a.top>=start&&a.top<finish).map(|a|Anchor{id:a.id,top:a.top-start+prefix}).collect();
        out.children=item.children.iter().filter(|l|l.top>=start&&l.top<finish).map(|l|{let mut l=l.clone();l.top+=prefix-start;l}).collect();
        out.cells=item.cells.iter().filter(|c|c.rect[1]<finish&&c.rect[1]+c.rect[3]>start).map(|c|{
            let mut c=c.clone();let a=c.rect[1].max(start);let b=(c.rect[1]+c.rect[3]).min(finish);
            crate::table_borders::clip(&mut c,a,b);c.row_end &= b==c.rect[1]+c.rect[3];c.rect[1]=a-start+prefix;c.rect[3]=b-a;
            let extra:i32=if start==0{0}else{repeated.iter().filter(|r|{let mut parent=d.table_parent(r.header.table);while let Some((id,_,_))=parent{if id==c.table{return true;}parent=d.table_parent(id);}false}).map(|r|r.header.height).sum()};
            if a==start&&extra>0{c.rect[1]-=extra;c.rect[3]+=extra;for e in &mut c.edges{match e.side{doc_core::BorderSide::Left|doc_core::BorderSide::Right=>{if e.offset==0{e.length+=extra;}else{e.offset+=extra;}},_=>()}}}c
        }).collect();
        out.keeps=item.keeps.iter().filter(|r|r.top<finish&&r.bottom>start).map(|r|Keep{top:r.top-start+prefix,bottom:r.bottom-start+prefix}).collect();
        out.regions=item.regions.iter().filter(|r|r.top<finish&&r.bottom>start).map(|r|RepeatRegion{top:r.top-start+prefix,bottom:r.bottom-start+prefix,header:r.header.clone()}).collect();out
    };
    let left=slice(0,end,0);let mut right=slice(end,item.line.height,prefix);
    if prefix>0{right.keeps.push(Keep{top:0,bottom:prefix});}
    right.page_break=true;right.before=0;right.at_top=0;right.keep_height=0;
    let mut y=0;
    for r in repeated {
        for l in &r.header.children{let mut l=l.clone();l.top+=y;right.children.push(l);}
        for c in &r.header.cells{let mut c=c.clone();c.rect[1]+=y;c.repeated=true;right.cells.push(c);}y+=r.header.height;
    }
    right.children.sort_by_key(|l|l.top);
    Some((left,right))
}
