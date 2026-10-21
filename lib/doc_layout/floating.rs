//! Float placement and line-specific text exclusion, shared by every renderer.
use super::*;
use doc_core::{TableVertical,Table};
use alloc::rc::Rc;
#[path="floating_fragment.rs"] mod fragment;

pub(super) struct Floating {
    cursor:Cursor,end:i32,locked_end:i32,
    pub rect:[i32;4], pub exclusion:[i32;4],
    pub children:Vec<Line>,pub cells:Vec<CellBox>,
}
/// Immutable measured geometry is shared by all page fragments, including nested tables.
struct Source {
    position:doc_core::TablePosition,x:i32,width:i32,height:i32,
    children:Vec<Line>,cells:Vec<CellBox>,regions:Vec<RepeatRegion>,bands:Vec<i32>,keeps:Vec<Keep>,
    anchors:Vec<Anchor>,
}
pub(super) struct Cursor {source:Rc<Source>,offset:i32}
fn intersects(a:[i32;4],b:[i32;4])->bool {
    a[0]<b[0]+b[2] && b[0]<a[0]+a[2] && a[1]<b[1]+b[3] && b[1]<a[1]+a[3]
}
pub(super) fn marker(id:Id,offset:usize,after:i32)->Flow {
    Flow {floating_notes:false,after_floats:false,keeps:Vec::new(),deferred:Some((id,offset,after)),resume:None,regions:Vec::new(),repeat:None,children:Vec::new(),cells:Vec::new(),
        line:Line{cell:None,bar_tabs:Vec::new(),start:offset,end:offset,paragraph:id,x:0,top:0,height:0,fragments:Vec::new()},
        before:0,at_top:0,page_break:false,keep_height:0,anchors:Vec::new(),note_start:false}
}
pub(super) fn body(d:&Document,bib:&str,m:&mut impl Measure,budget:&mut Budget)->Result<Vec<Flow>,Error>{
    let mut out=Vec::new();let mut after=0;
    for p in &d.paragraphs {
        for t in d.tables.iter().filter(|t|t.before==p.id&&t.position.is_none()) {out.extend(table_flow(d,t,bib,m,budget)?);after=0;}
        out.push(marker(p.id,0,after));after=d.resolve_paragraph(p).after as i32;
    }Ok(out)
}
fn measure_table(d:&Document,t:&Table,bib:&str,m:&mut impl Measure,budget:&mut Budget)->Result<Cursor,Error>{
    let mut f=Source{position:t.position.unwrap(),x:d.table_x(t).map_err(Error::Model)? as i32,width:t.width() as i32,height:0,children:Vec::new(),cells:Vec::new(),regions:Vec::new(),bands:Vec::new(),keeps:Vec::new(),anchors:Vec::new()};
    for item in table_flow(d,t,bib,m,budget)? {
        f.anchors.extend(item.anchors.iter().map(|a|Anchor{id:a.id,top:a.top+f.height}));
        f.keeps.extend(item.keeps.iter().map(|r|Keep{top:r.top+f.height,bottom:r.bottom+f.height}));
        for mut l in item.children {l.top+=f.height;f.children.push(l);}
        for mut c in item.cells {c.rect[1]+=f.height;f.cells.push(c);}
        for mut r in item.regions {r.top+=f.height;r.bottom+=f.height;f.regions.push(r);}
        if let Some(header)=item.repeat{f.regions.push(RepeatRegion{top:f.height,bottom:f.height+item.line.height,header});}
        f.height+=item.line.height;f.bands.push(f.height);
    }
    // Each row band may describe another part of the same repeated-header region.
    f.regions.sort_by_key(|r|(r.header.table,r.top));
    let mut regions:Vec<RepeatRegion>=Vec::new();
    for r in f.regions {if let Some(old)=regions.last_mut().filter(|old|old.header.table==r.header.table&&r.top<=old.bottom){old.bottom=old.bottom.max(r.bottom);}else{regions.push(r);}}
    f.regions=regions;Ok(Cursor{source:Rc::new(f),offset:0})
}
/// False requests a new page before the anchor. Nothing is committed on failure.
pub(super) fn place(d:&Document,p:Id,bib:&str,m:&mut impl Measure,budget:&mut Budget,draft:&mut Draft)->Result<bool,Error>{
    let mut added:Vec<Floating>=Vec::new();let mut pending=Vec::new();let bottom=(d.page.height-d.page.bottom) as i32-draft.reserved();
    for t in d.tables.iter().filter(|t|t.before==p&&t.position.is_some()) {
        let pos=t.position.unwrap();let cursor=measure_table(d,t,bib,m,budget)?;
        let base=match pos.vertical{TableVertical::Page=>0,TableVertical::Margin=>d.page.top as i32,TableVertical::Paragraph=>draft.y};
        let y=clear(&cursor,base+pos.y,bottom,draft.floating.iter().chain(&added));
        let Some(f)=fragment::slice(d,&cursor,y,bottom)? else {
            // A kept first row may not fit at its anchor position. Its first fragment
            // starts on the next page, while the anchored paragraph keeps its place.
            if !cursor.source.keeps.is_empty() && y>d.page.top as i32
                && fragment::slice(d,&cursor,d.page.top as i32,(d.page.height-d.page.bottom) as i32)?.is_some(){pending.push(cursor);continue;}
            if draft.floating.is_empty()&&added.is_empty(){return Ok(false);}
            pending.push(cursor);continue;
        };
        // A page-relative anchor cannot overwrite text already committed to this page.
        if draft.cells.iter().any(|c|intersects(c.rect,f.exclusion)) || draft.body.iter().any(|l|{
            let end=l.fragments.iter().map(|v|v.x+v.width).max().unwrap_or(l.x+1);
            intersects([l.x,l.top,(end-l.x).max(1),l.height],f.exclusion)
        }) {return Ok(false);}
        added.push(f);
    }
    draft.floating.extend(added);draft.pending.extend(pending);Ok(true)
}
fn clear<'a>(cursor:&Cursor,mut y:i32,bottom:i32,old:impl Iterator<Item=&'a Floating>+Clone)->i32{
    let s=&cursor.source;
    if !s.position.overlap {for _ in 0..=old.clone().count(){
        let rect=[s.x,y,s.width,(s.height-cursor.offset).min((bottom-y).max(0))];
        if let Some(next)=old.clone().filter(|f|intersects(f.exclusion,rect)).map(|f|f.exclusion[1]+f.exclusion[3]+i32::from(s.position.distance[0])).max(){y=next;}else{break;}
    }}y
}
pub(super) fn continuations(draft:&mut Draft)->Vec<Cursor>{
    let mut out:Vec<_>=draft.floating.iter().filter(|f|f.end<f.cursor.source.height).map(|f|Cursor{source:f.cursor.source.clone(),offset:f.end}).collect();
    out.append(&mut draft.pending);out
}
pub(super) fn resume(d:&Document,cursors:Vec<Cursor>,draft:&mut Draft)->Result<(),Error>{
    let bottom=(d.page.height-d.page.bottom) as i32-draft.reserved();
    for cursor in cursors {
        let y=clear(&cursor,d.page.top as i32,bottom,draft.floating.iter());
        if let Some(f)=fragment::slice(d,&cursor,y,bottom)? {draft.floating.push(f);}
        else if !draft.floating.is_empty(){draft.pending.push(cursor);}
        else{return Err(Error::Limit);}
    }Ok(())
}
/// Reserve footnotes before their anchor line is committed. Failed reflow changes nothing.
pub(super) fn reserve(d:&Document,draft:&mut Draft,bottom:i32)->Result<(),Error>{
    if draft.floating.iter().all(|f|f.exclusion[1]+f.exclusion[3]<=bottom){return Ok(());}
    let mut changed=Vec::new();
    for f in &draft.floating {let Some(mut next)=fragment::slice(d,&f.cursor,f.rect[1],bottom)? else{return Ok(());};if next.end<f.locked_end{return Ok(());}next.locked_end=f.locked_end;changed.push(next);}
    draft.floating=changed;Ok(())
}
/// Only original cell fragments contribute anchors; repeated headers do not renumber notes.
pub(super) fn note_anchors(draft:&Draft,notes:&BTreeMap<Id,Vec<Flow>>)->Vec<Anchor>{
    draft.floating.iter().flat_map(|f|f.cursor.source.anchors.iter().filter(|a|a.top>=f.cursor.offset&&a.top<f.end&&notes.contains_key(&a.id)).copied()).collect()
}
pub(super) fn lock_notes(draft:&mut Draft,anchors:&[Anchor]){
    for f in &mut draft.floating {for a in &f.cursor.source.anchors{
        if anchors.iter().any(|n|n.id==a.id){f.locked_end=f.locked_end.max(a.top+1);}
    }}
}
/// Shrink monotonically while fitting a fragment's notes. A deferred fragment is retried
/// on a fresh page; already committed note anchors can never be moved to another page.
pub(super) fn reserve_notes(d:&Document,draft:&mut Draft,bottom:i32)->Result<bool,Error>{
    let mut changed=false;let mut out=Vec::new();
    for f in core::mem::take(&mut draft.floating){
        if f.exclusion[1]+f.exclusion[3]<=bottom{out.push(f);continue;}
        let mut next=fragment::slice(d,&f.cursor,f.rect[1],bottom)?;
        if next.is_none() && f.rect[3]>1 {
            // A large note group can over-reserve for anchors in later rows. Try a
            // smaller original fragment before concluding its first row cannot fit.
            next=fragment::slice(d,&f.cursor,f.rect[1],f.rect[1]+f.rect[3]/2)?;
        }
        if let Some(mut n)=next.filter(|n|n.end>=f.locked_end&&n.end<f.end){n.locked_end=f.locked_end;out.push(n);changed=true;}
        else if f.locked_end<=f.cursor.offset && (f.rect[1]>d.page.top as i32||!draft.body.is_empty()||!draft.cells.is_empty()||!draft.footer.is_empty()){
            draft.pending.push(f.cursor);changed=true;
        }else{out.push(f);}
    }
    draft.floating=out;Ok(changed)
}
pub(super) fn trim(input:&mut Vec<Atom>,offset:usize,m:&mut impl Measure)->Result<(),Error>{
    input.retain(|a|a.source.is_none_or(|(_,end)|end>offset));
    if let Some(a)=input.first_mut(){if let Some((start,end))=a.source {if start<offset {
        let cut=offset-start;if !a.text.is_char_boundary(cut){return Err(Error::Metrics);}
        a.text=a.text[cut..].into();a.source=Some((offset,end));a.width=m.width(&a.text,a.format);
        if !(0..=100_000_000).contains(&a.width){return Err(Error::Metrics);}
    }}}Ok(())
}
pub(super) struct Wrap {
    y:i32,top:i32,bottom:i32,fresh_bottom:i32,rects:Vec<[i32;4]>,
    left:i32,right:i32,width:i32,pf:ParagraphFormat,default:Metrics,height:i32,
}
impl Wrap {
    pub fn new(d:&Document,draft:&Draft,gap:i32)->Self{
        Self{y:draft.y+gap,top:d.page.top as i32,bottom:(d.page.height-d.page.bottom) as i32-draft.reserved(),fresh_bottom:(d.page.height-d.page.bottom) as i32,
            rects:draft.floating.iter().map(|f|f.exclusion).collect(),left:0,right:0,width:0,pf:ParagraphFormat::default(),default:Metrics{ascent:1,descent:0,height:1},height:1}
    }
    pub fn configure(&mut self,area:PageSettings,pf:ParagraphFormat,default:Metrics,input:&[Atom]){
        self.left=area.left as i32+pf.left as i32;self.right=(area.width-area.right) as i32-pf.right as i32;self.width=self.right-self.left;
        self.pf=pf;self.default=default;
        let ascent=input.iter().map(|a|a.metrics.ascent).max().unwrap_or(default.ascent);
        let descent=input.iter().map(|a|a.metrics.descent).max().unwrap_or(default.descent);
        self.height=input.iter().map(|a|a.metrics.height).max().unwrap_or(default.height).max(ascent+descent)*pf.line as i32/240;
    }
    pub fn prepare(&mut self,next:&mut Pending,previous:Option<&Pending>)->Result<(),Error>{
        if let Some(p)=previous{self.y+=line_height(p,self.default,self.pf.line).0;}
        let mut initial=self.y;
        for _ in 0..=self.rects.len()+1 {
            if self.y+self.height>self.bottom {
                self.y=self.top;initial=self.top;self.bottom=self.fresh_bottom;self.rects.clear();next.page_break=true;
                if self.y+self.height>self.bottom{return Err(Error::Limit);}
            }
            let mut blocked:Vec<_>=self.rects.iter().filter(|r|r[1]<self.y+self.height && self.y<r[1]+r[3] && r[0]<self.right && r[0]+r[2]>self.left).copied().collect();
            blocked.sort_by_key(|r|r[0]);
            let mut left=self.left+next.indent;let mut best=(left,self.right);let mut width=0;
            for r in &blocked {let right=r[0].clamp(left,self.right);if right-left>width{best=(left,right);width=right-left;}left=left.max(r[0]+r[2]).min(self.right);}
            if self.right-left>width{best=(left,self.right);width=self.right-left;}
            if blocked.is_empty() || width>=720 {
                next.left=best.0-self.left-next.indent;next.available=best.1-best.0;next.gap=self.y-initial;
                return if next.available>0 {Ok(())}else{Err(Error::Limit)};
            }
            self.y=blocked.iter().map(|r|r[1]+r[3]).filter(|y|*y>self.y).min().ok_or(Error::Limit)?;
        }Err(Error::Limit)
    }
}
