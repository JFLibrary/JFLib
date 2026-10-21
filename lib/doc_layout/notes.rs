// Page-bottom footnotes and terminal endnotes. New native layout; no legacy counterpart.
// Reservation happens before committing an anchor line. Continuations consume a finite queue.
use super::*;
use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use doc_core::{NoteKind, ParagraphFormat};

const SEPARATION: i32 = 160;
#[path="floating.rs"] mod floating;
#[path="row_breaks.rs"] mod row_breaks;
#[path="row_height.rs"] mod row_height;
use row_breaks::Keep;
#[path="table_notes.rs"] mod table_notes;

#[derive(Clone, Copy)]
struct Anchor { id: Id, top: i32 }

struct Header { table: Id, height: i32, children: Vec<Line>, cells: Vec<CellBox> }

#[derive(Clone)]
struct RepeatRegion {top:i32,bottom:i32,header:alloc::rc::Rc<Header>}

#[derive(Clone)]
struct Flow {
    floating_notes: bool,
    after_floats: bool,
    keeps: Vec<Keep>,
    deferred: Option<(Id,usize,i32)>,
    resume: Option<Id>,
    regions: Vec<RepeatRegion>,
    repeat: Option<alloc::rc::Rc<Header>>,
    children: Vec<Line>,
    cells: Vec<CellBox>,
    line: Line,
    before: i32,
    at_top: i32,
    page_break: bool,
    keep_height: i64,
    anchors: Vec<Anchor>,
    note_start: bool,
}

struct Budget {
    expanded: usize,
    fragments: usize,
    lines: usize,
}

fn flow_in(
    area: PageSettings,
    d: &Document,
    paragraphs: &[Paragraph],
    prefix: Option<&str>,
    bibliography: &str,
    measure: &mut impl Measure,
    budget: &mut Budget,
) -> Result<Vec<Flow>, Error> {
    flow_planned(area,d,paragraphs,prefix,bibliography,measure,budget,None,0,0)
}
fn flow_planned(
    area:PageSettings,d:&Document,paragraphs:&[Paragraph],prefix:Option<&str>,bibliography:&str,
    measure:&mut impl Measure,budget:&mut Budget,mut plan:Option<&mut floating::Wrap>,offset:usize,after:i32,
) -> Result<Vec<Flow>,Error> {
    let mut output: Vec<Flow> = Vec::new();
    let mut previous_after = after;
    let mut previous_heading: Option<(usize, i64, i32)> = None;
    for (paragraph_index, p) in paragraphs.iter().enumerate() {
        let mut pf = d.resolve_paragraph(p);
        if offset>0 {pf.first_line=0;pf.before=0;pf.page_break_before=false;}
        let width = area.content_width() as i32 - pf.left as i32 - pf.right as i32;
        let mut input = paragraph_atoms(d, p, bibliography, measure, &mut budget.expanded)?;
        if paragraph_index == 0 {
            if let Some(prefix) = prefix {
                budget.expanded = budget
                    .expanded
                    .checked_add(prefix.len())
                    .ok_or(Error::Limit)?;
                if budget.expanded > 4 << 20 {
                    return Err(Error::Limit);
                }
                let mut numbered = Vec::new();
                atoms(
                    prefix,
                    d.resolve_paragraph_text(p, TextFormat::default()),
                    measure,
                    &mut numbered,
                )?;
                numbered.append(&mut input);
                input = numbered;
            }
        }
        let default = metrics(measure, d.resolve_paragraph_text(p, TextFormat::default()))?;
        if offset>0 {floating::trim(&mut input,offset,measure)?;}
        let mut lines = if let Some(plan)=plan.as_mut(){
            plan.configure(area,pf,default,&input);
            wrap_with(input,width,pf,measure,&mut |next,previous|plan.prepare(next,previous))?
        } else {wrap(input,width,pf,measure)?};
        if offset>0 {if let Some(first)=lines.first_mut(){first.start=offset;first.end=first.end.max(offset);first.next=first.next.max(offset);}}
        let count = lines.len();
        let start = output.len();
        let mut total = 0i64;
        for (index, pending) in lines.iter_mut().enumerate() {
            while pending.atoms.last().is_some_and(|a| a.space) {
                pending.width -= pending.atoms.pop().unwrap().width;
            }
            let (height, ascent) = line_height(pending, default, pf.line);
            if height > area.content_height() as i32 {
                return Err(Error::Limit);
            }
            total += i64::from(height);
            let free = (pending.available - pending.width).max(0);
            let mut x = (area.left + pf.left as u32) as i32
                + pending.left
                + pending.indent
                + match if pending.atoms.iter().any(|a| a.tab) { Alignment::Left } else { pf.alignment } {
                    Alignment::Center => free / 2,
                    Alignment::Right => free,
                    _ => 0,
                };
            let spaces = pending.atoms.iter().filter(|a| a.space).count() as i32;
            let justify = pf.alignment == Alignment::Justify
                && !pending.atoms.iter().any(|a| a.tab)
                && index + 1 < count
                && !pending.hard
                && spaces > 0;
            let mut used_space = 0;
            let mut anchors = Vec::new();
            let mut line = Line {
                cell: None,
                bar_tabs: tabs::bars(pf, area).into_iter().filter(|v|plan.is_none() || (*v>=x && *v<=x+pending.available)).collect(),
                start: pending.start,
                end: pending.end,
                paragraph: p.id,
                x,
                top: 0,
                height,
                fragments: Vec::new(),
            };
            for atom in &pending.atoms {
                let mut advance = atom.width;
                if justify && atom.space {
                    advance += free / spaces + i32::from(used_space < free % spaces);
                    used_space += 1;
                }
                if let Some(id) = atom.note {
                    if d.note(id).is_some_and(|n| n.kind == NoteKind::Footnote) {
                        anchors.push(Anchor {id,top:0});
                    }
                }
                if !atom.text.is_empty() || atom.source.is_some() {
                    line.fragments.push(Fragment {
                        leader: atom.leader,
                        source: atom.source,
                        text: atom.text.clone(),
                        format: atom.format,
                        x,
                        baseline: ascent + atom.format.baseline_shift(),
                        width: advance,
                    });
                    budget.fragments += 1;
                    if budget.fragments > 100000 {
                        return Err(Error::Limit);
                    }
                }
                x += advance;
            }
            output.push(Flow {
                floating_notes:false,after_floats:false,
                keeps:Vec::new(),
                deferred:None,resume:plan.as_ref().map(|_|p.id),
                regions:Vec::new(), repeat: None, children: Vec::new(), cells: Vec::new(),
                line,
                before: pending.gap + if index == 0 && !pending.page_break {
                    previous_after.max(pf.before as i32)
                } else {
                    0
                },
                at_top: pending.gap + if index == 0 && !pending.page_break {
                    pf.before as i32
                } else {
                    0
                },
                page_break: pending.page_break || (plan.is_none() && index == 0 && pf.page_break_before),
                keep_height: 0,
                anchors,
                note_start: prefix.is_some() && paragraph_index == 0 && index == 0,
            });
            budget.lines += 1;
            if budget.lines > 100000 {
                return Err(Error::Limit);
            }
        }
        if let Some((heading_start, heading_height, after)) = previous_heading.take() {
            if !pf.page_break_before {
                output[heading_start].keep_height = heading_height
                    + i64::from(after.max(pf.before as i32))
                    + i64::from(output[start].line.height);
            }
        }
        if d.outline_level(p.style).is_some() {
            previous_heading = Some((start, total, pf.after as i32));
        }
        previous_after = pf.after as i32;
    }
    Ok(output)
}

fn flow(d: &Document, paragraphs: &[Paragraph], prefix: Option<&str>, bibliography: &str, measure: &mut impl Measure, budget: &mut Budget) -> Result<Vec<Flow>,Error> {
    flow_in(d.page,d,paragraphs,prefix,bibliography,measure,budget)
}
fn body_flow(d: &Document, bibliography: &str, measure: &mut impl Measure, budget: &mut Budget) -> Result<Vec<Flow>,Error> {
    if d.tables.iter().any(|t|t.position.is_some()) {return floating::body(d,bibliography,measure,budget);}
    story_flow(d.page,d,&d.paragraphs,bibliography,measure,budget)
}
fn story_flow(area:PageSettings,d:&Document,ps:&[Paragraph],bibliography:&str,measure:&mut impl Measure,budget:&mut Budget)->Result<Vec<Flow>,Error>{
    let mut output=Vec::new();let mut start=0;
    for (i,p) in ps.iter().enumerate() {
        if d.tables.iter().any(|t|t.before==p.id){
            output.extend(flow_in(area,d,&ps[start..i],None,bibliography,measure,budget)?);
            for t in d.tables.iter().filter(|t|t.before==p.id){output.extend(table_flow(d,t,bibliography,measure,budget)?);}
            start=i;
        }
    }
    output.extend(flow_in(area,d,&ps[start..],None,bibliography,measure,budget)?);Ok(output)
}
fn table_flow(d: &Document, table: &doc_core::Table, bibliography: &str, measure: &mut impl Measure, budget: &mut Budget) -> Result<Vec<Flow>,Error> {
    let pad=i32::from(table.padding);
    let (border_width,border_color)=d.table_border(table);
    let mut heights:Vec<i32>=table.rows.iter().map(|r|r.min_height as i32).collect();
    let mut measured=Vec::new();
    for (r,c,cell) in table.anchors() {
        let x=(d.table_x(table).map_err(Error::Model)?+table.columns[..c].iter().sum::<u32>()) as i32;
        let width=table.cell_width(r,c).unwrap() as i32;
        let mut area=d.page;area.left=(x+pad) as u32;area.right=area.width-(x+width-pad) as u32;
        let mut lines=Vec::new();let mut nested=Vec::new();let mut regions:Vec<RepeatRegion>=Vec::new();let mut keeps=Vec::new();let mut anchors=Vec::new();let mut y=pad;
        for mut item in story_flow(area,d,&cell.paragraphs,bibliography,measure,budget)? {
            y+=if y==pad {item.at_top} else {item.before};
            anchors.extend(item.anchors.iter().map(|a|Anchor{id:a.id,top:a.top+y}));
            keeps.extend(item.keeps.iter().map(|r|Keep{top:r.top+y,bottom:r.bottom+y}));
            if item.cells.is_empty() {item.line.cell=Some(cell.id);item.line.top=y;lines.push(item.line.clone());}
            else {
                for line in &mut item.children {line.top+=y;}lines.extend(item.children);
                for c in &mut item.cells {c.rect[1]+=y;}nested.extend(item.cells);
            }
            if let Some(header)=item.repeat {
                if let Some(region)=regions.iter_mut().find(|v|v.header.table==header.table && v.bottom==y){region.bottom+=item.line.height;}
                else{regions.push(RepeatRegion{top:y,bottom:y+item.line.height,header});}
            }
            for mut region in item.regions {region.top+=y;region.bottom+=y;regions.push(region);}
            y+=item.line.height;
        }
        if cell.row_span==1 && table.rows[r].height_rule!=doc_core::RowHeightRule::Exact {heights[r]=heights[r].max(y+pad);}
        measured.push((r,c,x,width,y+pad,lines,nested,regions,keeps,anchors,cell));
    }
    for (r,_,_,_,needed,_,_,_,_,_,cell) in &measured {
        let end=*r+usize::from(cell.row_span);
        let present=heights[*r..end].iter().sum::<i32>();
        if present<*needed {
            if let Some(grow)=(*r..end).rev().find(|r|table.rows[*r].height_rule!=doc_core::RowHeightRule::Exact){heights[grow]+=needed-present;}
        }
    }
    let mut offsets=vec![0];for h in &heights {offsets.push(offsets.last().unwrap()+h);}
    let mut children=Vec::new();let mut cells=Vec::new();let mut regions=Vec::new();let mut keeps=Vec::new();let mut anchors=Vec::new();
    for (r,c,x,width,needed,mut lines,mut nested,mut nested_regions,mut nested_keeps,cell_anchors,cell) in measured {
        let end=r+usize::from(cell.row_span);let height=offsets[end]-offsets[r];
        let free=(height-needed).max(0);
        let shift=offsets[r]+match cell.alignment {doc_core::CellAlignment::Top=>0,doc_core::CellAlignment::Center=>free/2,doc_core::CellAlignment::Bottom=>free};
        // A fixed-height cell hides overflow, never deletes it from the document.
        // Clip nested geometry too: hit tests, page breaks and paint share this extent.
        if needed>height {row_height::clip(height,&mut lines,&mut nested,&mut nested_regions,&mut nested_keeps);}
        // Fixed-height clipping hides the marker, not its scientific annotation.
        // Such a note belongs to the final visible fragment of its cell.
        anchors.extend(cell_anchors.iter().map(|a|Anchor{id:a.id,top:a.top.min(height-1)+shift}));
        for line in &mut lines {line.top+=shift;}children.extend(lines);
        cells.push(CellBox {table:table.id,cell:cell.id,row:r,column:c,paragraph:cell.paragraphs[0].id,
            row_span:usize::from(cell.row_span),column_span:usize::from(cell.column_span),last_row_height:heights[end-1] as u32,
            row_end:true,repeated:false,edges:crate::table_borders::edges(d,table,r,c,&heights),
            rect:[x,offsets[r],width,height],fill:d.table_region(table,r,c).fill,border_width,border_color});
        for c in &mut nested {c.rect[1]+=shift;}cells.extend(nested);
        for mut region in nested_regions{region.top+=shift;region.bottom+=shift;regions.push(region);}
        keeps.extend(nested_keeps.iter().map(|r|Keep{top:r.top+shift,bottom:r.bottom+shift}));
    }
    for (r,row) in table.rows.iter().enumerate(){if row.cant_split{keeps.push(Keep{top:offsets[r],bottom:offsets[r+1]});}}
    let header_count=usize::from(table.header_rows);
    let header=if header_count>0 {
        let height=offsets[header_count];
        if height>=d.page.content_height() as i32 {return Err(Error::Limit);}
        Some(alloc::rc::Rc::new(Header {table:table.id,height,
            children:children.iter().filter(|l| l.top<height).cloned().collect(),
            cells:cells.iter().filter(|c| c.rect[1]<height).cloned().collect()}))
    } else {None};
    if let Some(h)=&header{keeps.push(Keep{top:0,bottom:h.height});}
    // Vertically connected rows form a band. Fit it together, or continue without cutting text.
    let mut output=Vec::new();let available=if table.position.is_some() || d.table_parent(table.id).is_some(){offsets.last().copied().unwrap_or(0).saturating_add(d.page.content_height() as i32)}else{d.page.content_height() as i32};let mut row=0;
    while row<table.rows.len() {
        let mut end=if row==0 {header_count.max(1)} else {row+1};
        loop {
            let expanded=table.anchors().filter(|(r,_,_)|*r>=row && *r<end).map(|(r,_,c)|r+usize::from(c.row_span)).max().unwrap_or(end).max(end);
            if expanded==end {break;}end=expanded;
        }
        let mut top=offsets[row];let band_top=top;let band_bottom=offsets[end];
        while top<band_bottom {
            let mut repeated:Vec<_>=regions.iter().filter(|v|top>band_top && v.top<=top && top<v.bottom).collect();
            repeated.sort_by_key(|v|d.table_depth(v.header.table).unwrap_or(0));
            repeated.dedup_by_key(|v|v.header.table);
            let prefix:i32=repeated.iter().map(|v|v.header.height).sum();
            let capacity=available-prefix-if row>=header_count {header.as_ref().map_or(0,|h|h.height)} else {0};
            let mut bottom=(top+capacity).min(band_bottom);
            bottom=row_breaks::cut(&keeps,&children,top,bottom,capacity,Some(top));
            loop {
                let crossing=children.iter().filter(|l|l.top<bottom && l.top+l.height>bottom).map(|l|l.top).min();
                if let Some(at)=crossing {if at<=top {return Err(Error::Limit);}bottom=at;}else {break;}
            }
            if bottom<=top {return Err(Error::Limit);}
            let mut chunk:Vec<_>=children.iter().filter(|l|l.top>=top && l.top<bottom).map(|l|{let mut l=l.clone();l.top+=prefix-top;l}).collect();
            let mut boxes:Vec<_>=cells.iter().filter(|c|c.rect[1]<bottom && c.rect[1]+c.rect[3]>top).map(|c|{
                let mut c=c.clone();let start=c.rect[1].max(top);let finish=(c.rect[1]+c.rect[3]).min(bottom);
                crate::table_borders::clip(&mut c,start,finish);
                c.row_end &= finish==c.rect[1]+c.rect[3];c.rect[1]=start-top+prefix;c.rect[3]=finish-start;
                // Enclosing cell walls continue around repeated child headers.
                let contains_header=repeated.iter().any(|v|{let mut parent=d.table_parent(v.header.table);while let Some((id,_,_))=parent{if id==c.table{return true;}parent=d.table_parent(id);}false});
                if start==top && contains_header {c.rect[1]-=prefix;c.rect[3]+=prefix;for edge in &mut c.edges {match edge.side {doc_core::BorderSide::Left|doc_core::BorderSide::Right=>{if edge.offset==0{edge.length+=prefix;}else{edge.offset+=prefix;}},_=>()}}}
                c
            }).collect();
            let mut header_y=0;
            for v in &repeated {for line in &v.header.children{let mut line=line.clone();line.top+=header_y;chunk.push(line);}
                for c in &v.header.cells{let mut c=c.clone();c.rect[1]+=header_y;c.repeated=true;boxes.push(c);}header_y+=v.header.height;
            }
            chunk.sort_by_key(|l|l.top);
            let inherited=regions.iter().filter(|v|v.top<bottom&&v.bottom>top).map(|v|RepeatRegion{top:v.top-top,bottom:v.bottom-top,header:v.header.clone()}).collect();
            output.push(Flow {floating_notes:false,after_floats:false,keeps:keeps.iter().filter(|r|r.top<bottom&&r.bottom>top).map(|r|Keep{top:r.top-top+prefix,bottom:r.bottom-top+prefix}).collect(),deferred:None,resume:None,regions:inherited,repeat:if row>=header_count {header.clone()} else {None},children:chunk,cells:boxes,line:Line {cell:None,bar_tabs:Vec::new(),start:0,end:0,paragraph:0,x:d.page.left as i32,top:0,height:bottom-top+prefix,fragments:Vec::new()},before:0,at_top:0,page_break:top>band_top,keep_height:0,anchors:anchors.iter().filter(|a|a.top>=top&&a.top<bottom).map(|a|Anchor{id:a.id,top:a.top-top+prefix}).collect(),note_start:false});
            top=bottom;
        }
        row=end;
    }
    if header_count>0 && output.len()>1 {
        output[0].keep_height=i64::from(output[0].line.height)+i64::from(output[1].line.height);
    }
    Ok(output)
}

struct FooterLine {
    line: Line,
    gap: i32,
}

struct Draft {
    floating:Vec<floating::Floating>,
    pending:Vec<floating::Cursor>,
    cells: Vec<CellBox>,
    body: Vec<Line>,
    footer: Vec<FooterLine>,
    footer_height: i32,
    y: i32,
}
impl Draft {
    fn new(top: i32) -> Self {
        Self {
            floating:Vec::new(),
            pending:Vec::new(),
            cells: Vec::new(),
            body: Vec::new(),
            footer: Vec::new(),
            footer_height: 0,
            y: top,
        }
    }
    fn occupied(&self) -> bool {
        !self.body.is_empty() || !self.cells.is_empty() || !self.footer.is_empty() || !self.floating.is_empty() || !self.pending.is_empty()
    }
    fn reserved(&self) -> i32 {
        self.footer_height
            + if self.footer.is_empty() {
                0
            } else {
                SEPARATION
            }
    }
    fn append_footer(&mut self, item: Flow) {
        let gap = if self.footer.is_empty() {
            item.at_top
        } else {
            item.before
        };
        self.footer_height += gap + item.line.height;
        self.footer.push(FooterLine {
            line: item.line,
            gap,
        });
    }
    fn advance(mut self,d:&Document,layout:&mut Layout,budget:&mut Budget,bottom:i32)->Result<Self,Error>{
        let next=floating::continuations(&mut self);
        self.finish(layout,budget,bottom)?;
        let mut draft=Self::new(d.page.top as i32);floating::resume(d,next,&mut draft)?;Ok(draft)
    }
    fn advance_notes(mut self,d:&Document,layout:&mut Layout,budget:&mut Budget,bottom:i32)->Result<Self,Error>{
        let next=floating::continuations(&mut self);
        self.finish(layout,budget,bottom)?;
        let mut draft=Self::new(d.page.top as i32);draft.pending=next;Ok(draft)
    }
    fn finish(self, layout: &mut Layout, budget:&mut Budget, bottom: i32) -> Result<(), Error> {
        if layout.pages.len() >= 2048 {
            return Err(Error::Limit);
        }
        let mut page = Page {
            cells: self.cells,
            lines: self.body,
            separators: Vec::new(),
        };
        for f in self.floating {
            for l in &f.children {if f.cells.iter().any(|c|c.repeated&&Some(c.cell)==l.cell){budget.lines+=1;budget.fragments+=l.fragments.len();}}
            if budget.lines>100000||budget.fragments>100000{return Err(Error::Limit);}
            page.cells.extend(f.cells);page.lines.extend(f.children);
        }
        let mut y = bottom - self.footer_height;
        if !self.footer.is_empty() {
            page.separators.push(Separator {
                x: layout.settings.left as i32,
                y: y - SEPARATION / 2,
                width: layout.settings.content_width() as i32 / 3,
            });
        }
        for mut item in self.footer {
            y += item.gap;
            position(&mut item.line, y);
            y += item.line.height;
            page.lines.push(item.line);
        }
        layout.pages.push(page);
        Ok(())
    }
}

fn position(line: &mut Line, top: i32) {
    line.top = top;
    for fragment in &mut line.fragments {
        fragment.baseline += top;
    }
}

// Return the prefix that fits. A forced note paragraph break ends an already populated footer.
fn fitting(queue: &VecDeque<Flow>, draft: &Draft, available: i32) -> usize {
    let mut used = 0;
    let mut count = 0;
    for item in queue {
        if item.page_break && (count > 0 || !draft.footer.is_empty()) {
            break;
        }
        let gap = if count == 0 && draft.footer.is_empty() {
            item.at_top
        } else {
            item.before
        };
        used += gap + item.line.height;
        if used > available {
            break;
        }
        count += 1;
    }
    count
}

pub(super) fn paginate(d: &Document, measure: &mut impl Measure) -> Result<Layout, Error> {
    let mut budget = Budget {
        expanded: 0,
        fragments: 0,
        lines: 0,
    };
    let bibliography = d.bibliography().join("\n");
    let mut body = body_flow(d, &bibliography, measure, &mut budget)?;
    let mut notes = BTreeMap::new();
    for note in d.ordered_notes(NoteKind::Footnote) {
        let prefix = d.note_marker(note.id) + " ";
        let mut lines = flow(d, &note.paragraphs, Some(&prefix), "", measure, &mut budget)?;
        lines[0].before = lines[0].before.max(120);
        notes.insert(note.id, lines);
    }
    let endnotes = d.ordered_notes(NoteKind::Endnote);
    if !endnotes.is_empty() {
        let heading = Paragraph {
            id: 0,
            style: ParagraphStyle::Heading1,
            format: ParagraphFormat {
                before: 240,
                after: 160,
                ..ParagraphFormat::default()
            },
            content: vec![Inline::Text(String::from("Endnoten"))],
        };
        let heading_index = body.len();
        body.extend(flow(d, &[heading], None, "", measure, &mut budget)?);
        body[heading_index].after_floats=true;
        for item in &mut body[heading_index..] {
            item.line.start = 0;
            item.line.end = 0;
            for fragment in &mut item.line.fragments {
                fragment.source = None;
            }
        }
        let first_endnote = body.len();
        let heading_height: i64 = body[heading_index..]
            .iter()
            .map(|item| i64::from(item.line.height))
            .sum();
        for note in endnotes {
            let prefix = d.note_marker(note.id) + " ";
            let mut lines = flow(d, &note.paragraphs, Some(&prefix), "", measure, &mut budget)?;
            lines[0].before = lines[0].before.max(160);
            body.extend(lines);
        }
        if let Some(next) = body.get(first_endnote) {
            body[heading_index].keep_height =
                heading_height + i64::from(next.before) + i64::from(next.line.height);
        }
    }
    let mut layout = Layout {
        settings: d.page,
        pages: Vec::new(),
    };
    let top = d.page.top as i32;
    let bottom = (d.page.height - d.page.bottom) as i32;
    let mut draft = Draft::new(top);
    let mut placed=BTreeSet::new();
    let mut explicit_breaks=BTreeSet::new();
    let cell_footnotes=d.notes.iter().any(|n|n.kind==NoteKind::Footnote&&d.note_location(n.id).is_some_and(|p|d.cell_for_paragraph(p.paragraph).is_some()));
    let mut body:VecDeque<_>=body.into();
    'body: loop {
        let anchors=floating::note_anchors(&draft,&notes);
        let mut item=if !anchors.is_empty(){
            let mut item=floating::marker(0,0,0);item.deferred=None;item.floating_notes=true;item.anchors=anchors;item
        }else if let Some(item)=body.pop_front(){item}else{
            if draft.occupied(){draft=draft.advance(d,&mut layout,&mut budget,bottom)?;continue;}
            break;
        };
        if item.after_floats && (!draft.floating.is_empty()||!draft.pending.is_empty()) {
            body.push_front(item);draft=draft.advance(d,&mut layout,&mut budget,bottom)?;continue;
        }
        if let Some((id,offset,after))=item.deferred {
            let p=d.paragraph(id).ok_or(Error::Model(doc_core::Error::Missing))?;
            let pf=d.resolve_paragraph(p);
            if offset==0 && !placed.contains(&id) {
                if pf.page_break_before&&explicit_breaks.insert(id)&&draft.occupied(){
                    draft=draft.advance(d,&mut layout,&mut budget,bottom)?;
                    if !floating::note_anchors(&draft,&notes).is_empty(){body.push_front(item);continue;}
                }
                while !floating::place(d,id,&bibliography,measure,&mut budget,&mut draft)? {
                    if !draft.occupied(){return Err(Error::Limit);}
                    draft=draft.advance(d,&mut layout,&mut budget,bottom)?;
                    if !floating::note_anchors(&draft,&notes).is_empty(){body.push_front(item);continue 'body;}
                }
                placed.insert(id);
            }
            let gap=if offset>0{0}else if draft.body.is_empty()&&draft.cells.is_empty(){pf.before as i32}else{after.max(pf.before as i32)};
            let mut plan=floating::Wrap::new(d,&draft,gap);
            let mut lines=flow_planned(d.page,d,core::slice::from_ref(p),None,&bibliography,measure,&mut budget,Some(&mut plan),offset,after)?;
            if offset==0 && d.outline_level(p.style).is_some() {
                if let Some(next)=d.paragraphs.iter().position(|v|v.id==id).and_then(|i|d.paragraphs.get(i+1)).filter(|n|!d.resolve_paragraph(n).page_break_before&&!d.tables.iter().any(|t|t.before==n.id)){
                    let next_lines=flow(d,core::slice::from_ref(next),None,&bibliography,measure,&mut budget)?;
                    let total:i64=lines.iter().map(|l|i64::from(l.line.height)+i64::from(l.before)).sum();
                    if let Some(first)=lines.first_mut(){first.keep_height=total-i64::from(first.before)+i64::from(pf.after.max(d.resolve_paragraph(next).before))+i64::from(next_lines[0].line.height);}
                }
            }
            for line in lines.into_iter().rev(){body.push_front(line);}continue;
        }
        let mut queue;
        if item.page_break && draft.occupied() {
            if let Some(id)=item.resume {
                // A footnote or page boundary invalidates all remaining line widths.
                draft=draft.advance(d,&mut layout,&mut budget,bottom)?;
                while body.front().is_some_and(|v|v.resume==Some(id)){body.pop_front();}
                body.push_front(floating::marker(id,item.line.start,0));continue;
            }
            draft=draft.advance(d,&mut layout,&mut budget,bottom)?;
            if !floating::note_anchors(&draft,&notes).is_empty(){item.page_break=false;body.push_front(item);continue;}
        }
        loop {
            if item.floating_notes {item.anchors=floating::note_anchors(&draft,&notes);}
            queue = table_notes::queue(&item.anchors, &notes);
            let required = table_notes::required(&queue);
            let gap = if draft.body.is_empty() && draft.cells.is_empty() {
                item.at_top
            } else {
                item.before
            };
            let repeated=item.repeat.clone();
            let repeated=repeated.as_ref().filter(|h| !draft.cells.iter().any(|c| c.table==h.table));
            let repeat_height=repeated.map_or(0,|h|h.height);
            let mut y = draft.y + gap + repeat_height;
            // An inline table occupies its full rectangle; it must clear floats too.
            if !item.cells.is_empty(){for f in &draft.floating{if y<f.exclusion[1]+f.exclusion[3] && y+item.line.height>f.exclusion[1]{y=y.max(f.exclusion[1]+f.exclusion[3]);}}}
            let reserve = draft.reserved()
                + if !queue.is_empty() && draft.footer.is_empty() {
                    SEPARATION
                } else {
                    0
                };
            let needed=table_notes::minimum(&queue,&draft);
            if required>0 {
                if item.floating_notes {
                    if floating::reserve_notes(d,&mut draft,bottom-reserve-needed)? {continue;}
                }else{floating::reserve(d,&mut draft,bottom-reserve-needed)?;}
            }
            let float_bottom=draft.floating.iter().map(|f|f.exclusion[1]+f.exclusion[3]).max().unwrap_or(top);
            if !item.cells.is_empty() && y+item.line.height>bottom-reserve-needed && item.keep_height==0 {
                if let Some((left,right))=row_breaks::split(d,&item,bottom-reserve-needed-y,bottom-top-repeat_height,y==top+repeat_height){
                    item=left;body.push_front(right);continue;
                }
            }
            let available = bottom - (y + item.line.height).max(float_bottom) - reserve;
            let count = fitting(&queue, &draft, available);
            let keep_fits_fresh = item.keep_height > 0
                && item.keep_height + i64::from(item.at_top) <= i64::from(bottom - top);
            let move_heading = keep_fits_fresh
                && i64::from(y) + item.keep_height > i64::from(bottom - draft.reserved());
            let movable=draft.occupied()&&(draft.floating.is_empty()||!draft.body.is_empty()||!draft.cells.is_empty());
            if available < 0 || count < required || (move_heading && movable) {
                if item.floating_notes {return Err(Error::Limit);}
                if !draft.occupied() {
                    return Err(Error::Limit);
                }
                if let Some(id)=item.resume {
                    draft=draft.advance(d,&mut layout,&mut budget,bottom)?;
                    while body.front().is_some_and(|v|v.resume==Some(id)){body.pop_front();}
                    body.push_front(floating::marker(id,item.line.start,0));continue 'body;
                }
                draft=draft.advance(d,&mut layout,&mut budget,bottom)?;
                if !floating::note_anchors(&draft,&notes).is_empty(){body.push_front(item);continue 'body;}
                continue;
            }
            if let Some(header)=repeated {
                budget.lines+=header.children.len();budget.fragments+=header.children.iter().map(|l|l.fragments.len()).sum::<usize>();
                if budget.lines>100000 || budget.fragments>100000 {return Err(Error::Limit);}
                for original in &header.children {
                    let mut line=original.clone(); let top=line.top+draft.y; position(&mut line,top); draft.body.push(line);
                }
                for original in &header.cells {
                    let mut cell=original.clone();cell.rect[1]+=draft.y;cell.repeated=true;draft.cells.push(cell);
                }
            }
            if item.floating_notes {floating::lock_notes(&mut draft,&item.anchors);}
            for a in &item.anchors{notes.remove(&a.id);}
            position(&mut item.line, y);
            if !item.floating_notes {
                draft.y = y + item.line.height;
                if item.cells.is_empty() { draft.body.push(item.line); }
                else {
                    for mut line in item.children { let top = line.top + y; position(&mut line, top); draft.body.push(line); }
                    for mut cell in item.cells { cell.rect[1] += y; draft.cells.push(cell); }
                }
            }
            for _ in 0..count {
                draft.append_footer(queue.pop_front().unwrap());
            }
            break;
        }
        let continued=!queue.is_empty();
        while !queue.is_empty() {
            draft=if cell_footnotes {
                draft.advance_notes(d,&mut layout,&mut budget,bottom)?
            }else{draft.advance(d,&mut layout,&mut budget,bottom)?};
            let first=queue.front().unwrap();
            floating::reserve(d,&mut draft,bottom-SEPARATION-first.at_top-first.line.height)?;
            let edge=draft.floating.iter().map(|f|f.exclusion[1]+f.exclusion[3]).max().unwrap_or(top);
            let count = fitting(&queue, &draft, bottom - edge.max(top) - SEPARATION);
            if count == 0 {
                if draft.floating.is_empty(){return Err(Error::Limit);}continue;
            }
            for _ in 0..count {
                draft.append_footer(queue.pop_front().unwrap());
            }
        }
        if item.floating_notes {if let Some(next)=body.front().filter(|v|v.resume.is_some()){
            let id=next.resume.unwrap();let offset=next.line.start;
            while body.front().is_some_and(|v|v.resume==Some(id)){body.pop_front();}
            body.push_front(floating::marker(id,offset,0));
        }}
        if continued {if let Some(id)=item.resume {if let Some(next)=body.front().filter(|v|v.resume==Some(id)){
            let offset=next.line.start;while body.front().is_some_and(|v|v.resume==Some(id)){body.pop_front();}
            body.push_front(floating::marker(id,offset,0));
        }}}
    }
    if !notes.is_empty() {
        return Err(Error::Model(doc_core::Error::Invalid));
    }
    if layout.pages.is_empty(){draft.finish(&mut layout,&mut budget,bottom)?;}
    Ok(layout)
}
