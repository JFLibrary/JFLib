//! Clip measured floating tables at row boundaries or between lines of an oversized row.
use super::*;

pub(super) fn slice(d:&Document,cursor:&Cursor,y:i32,page_bottom:i32)->Result<Option<Floating>,Error>{
    let s=&cursor.source;let top=cursor.offset;
    let mut repeated:Vec<_>=s.regions.iter().filter(|r|top>0&&r.top<=top&&top<r.bottom).collect();
    repeated.sort_by_key(|r|d.table_depth(r.header.table).unwrap_or(0));
    let prefix:i32=repeated.iter().map(|r|r.header.height).sum();
    let capacity=page_bottom-y-i32::from(s.position.distance[2])-prefix;
    if capacity<=0{return Ok(None);}
    let mut bottom=(top+capacity).min(s.height);
    // Prefer complete row bands. A row taller than the remaining page can split at a line.
    if bottom<s.height {if let Some(edge)=s.bands.iter().copied().filter(|b|*b>top&&*b<=bottom).last(){
        // The first header band alone is not a usable table fragment.
        let header_only=top==0&&s.regions.iter().any(|r|r.header.height==edge&&r.top==edge);
        if !header_only{bottom=edge;}
    }}
    let first_header=if top==0{s.regions.iter().filter(|r|r.top==r.header.height).map(|r|r.header.height).max().unwrap_or(0)}else{0};
    let fresh_capacity=d.page.content_height() as i32-i32::from(s.position.distance[2])-prefix-first_header;
    bottom=row_breaks::cut(&s.keeps,&s.children,top,bottom,fresh_capacity,
        if y==d.page.top as i32{Some(top+first_header)}else{None});
    loop {if let Some(at)=s.children.iter().filter(|l|l.top<bottom&&l.top+l.height>bottom).map(|l|l.top).min(){bottom=at;}else{break;}}
    if bottom<=top{return Ok(None);}
    // A repeated header must stay whole, both at the beginning and inside an outer row.
    if s.regions.iter().any(|r|{let a=r.top-r.header.height;a>=top&&a<bottom&&bottom<=r.top}){return Ok(None);}
    let mut children:Vec<_>=s.children.iter().filter(|l|l.top>=top&&l.top<bottom).map(|l|{let mut l=l.clone();let at=l.top-top+prefix+y;position(&mut l,at);l}).collect();
    let mut cells:Vec<_>=s.cells.iter().filter(|c|c.rect[1]<bottom&&c.rect[1]+c.rect[3]>top).map(|c|{
        let mut c=c.clone();let start=c.rect[1].max(top);let finish=(c.rect[1]+c.rect[3]).min(bottom);
        crate::table_borders::clip(&mut c,start,finish);c.row_end &= finish==c.rect[1]+c.rect[3];c.rect[1]=start-top+prefix+y;c.rect[3]=finish-start;
        // Ancestor walls enclose repeated child headers, but never the table's own header.
        let extra:i32=repeated.iter().filter(|r|{let mut parent=d.table_parent(r.header.table);while let Some((id,_,_))=parent{if id==c.table{return true;}parent=d.table_parent(id);}false}).map(|r|r.header.height).sum();
        if start==top&&extra>0 {c.rect[1]-=extra;c.rect[3]+=extra;for edge in &mut c.edges{match edge.side{doc_core::BorderSide::Left|doc_core::BorderSide::Right=>{if edge.offset==0{edge.length+=extra;}else{edge.offset+=extra;}},_=>()}}}
        c
    }).collect();
    let mut at=y;
    for r in repeated {for l in &r.header.children{let mut l=l.clone();let top=l.top+at;position(&mut l,top);children.push(l);}
        for c in &r.header.cells{let mut c=c.clone();c.rect[1]+=at;c.repeated=true;cells.push(c);}at+=r.header.height;
    }
    children.sort_by_key(|l|l.top);
    let rect=[s.x,y,s.width,bottom-top+prefix];let [up,right,down,left]=s.position.distance.map(i32::from);
    let exclusion=[s.x-left,y-up,s.width+left+right,rect[3]+up+down];
    Ok(Some(Floating{cursor:Cursor{source:s.clone(),offset:top},end:bottom,locked_end:top,rect,exclusion,children,cells}))
}
