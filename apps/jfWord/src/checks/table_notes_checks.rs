//! Cell notes: ownership, whole-table transfer and physical page reservation.
use alloc::{format,string::String,vec,vec::Vec};
use doc_core::*;
struct Fixed;
impl doc_layout::Measure for Fixed {
    fn width(&mut self,s:&str,_:ResolvedFormat)->i32{s.chars().count() as i32*100}
    fn metrics(&mut self,_:ResolvedFormat)->doc_layout::Metrics{doc_layout::Metrics{ascent:180,descent:60,height:240}}
}
fn change(e:&mut Editor,t:Id,c:TableChange){e.apply(Edit::ChangeTable{table:t,change:c}).unwrap();}
fn note(e:&mut Editor,p:Id,kind:NoteKind,text:&str)->Id {
    e.apply(Edit::InsertNote{paragraph:p,inline:0,offset:0,kind,text:text.into()}).unwrap()
}
fn insert(e:&mut Editor,p:Id,rows:usize,columns:usize)->Id{e.apply(Edit::InsertTable{paragraph:p,offset:0,rows,columns}).unwrap()}
fn cell(e:&Editor,t:Id,r:usize,c:usize)->Id{e.document().table(t).unwrap().cell(r,c).unwrap().paragraphs[0].id}
fn text(e:&mut Editor,p:Id,s:&str){e.apply(Edit::SetText{paragraph:p,inline:0,text:s.into()}).unwrap();}
fn sample()->Editor{
    let mut d=Document::default();d.page=PageSettings{width:7200,height:6000,top:600,bottom:600,left:600,right:600};
    Editor::new(d).unwrap()
}
fn paginated(d:&Document)->doc_layout::Layout{doc_layout::paginate(d,&mut Fixed).unwrap()}
fn safe(d:&Document,l:&doc_layout::Layout)->bool{
    let bottom=(d.page.height-d.page.bottom) as i32;
    l.pages.iter().all(|p|p.cells.iter().all(|c|c.rect[1]>=d.page.top as i32&&c.rect[1]+c.rect[3]<=bottom
        && p.separators.first().is_none_or(|s|c.rect[1]+c.rect[3]<s.y))
        &&p.lines.iter().all(|line|line.top>=d.page.top as i32&&line.top+line.height<=bottom)
        &&p.separators.first().is_none_or(|s|p.lines.iter().all(|line|if d.note_for_paragraph(line.paragraph).is_some_and(|n|n.kind==NoteKind::Footnote){line.top>s.y}else{line.top+line.height<s.y})))
}
fn first_page(l:&doc_layout::Layout,p:Id)->Option<usize>{l.pages.iter().position(|page|page.lines.iter().any(|line|line.paragraph==p))}
pub(crate) fn run()->(usize,usize){
    let(mut passed,mut failed)=(0,0);
    macro_rules! check{($ok:expr,$name:expr)=>{if $ok{passed+=1;}else{failed+=1;plat::write_stdout(format!("FAIL table notes: {}\n",$name).as_bytes());}};}
    let mut e=sample();let main=note(&mut e,1,NoteKind::Footnote,"Body note");let t=insert(&mut e,1,3,2);
    let p=cell(&e,t,0,0);let q=cell(&e,t,1,1);let later=note(&mut e,q,NoteKind::Footnote,"Later cell");
    let early=note(&mut e,p,NoteKind::Footnote,"First cell");let inner=insert(&mut e,p,1,1);let ip=cell(&e,inner,0,0);
    let nested=note(&mut e,ip,NoteKind::Footnote,"Nested cell");let endp=cell(&e,t,2,0);let end=note(&mut e,endp,NoteKind::Endnote,"Cell endnote");
    check!(e.document().ordered_notes(NoteKind::Footnote).iter().map(|n|n.id).collect::<Vec<_>>()==vec![nested,early,later,main],"depth-first numbering ignores registry order");
    check!(e.document().note_number(end)==Some(1)&&e.document().note_number(main)==Some(4),"separate numbering sequences");
    check!(e.document().note_location(nested).unwrap().paragraph==ip,"nested anchor lookup");
    let nb=e.document().note(nested).unwrap().paragraphs[0].id;
    let source=e.apply(Edit::PutSource(Source{title:"Table note source".into(),..Default::default()})).unwrap();
    e.apply(Edit::Cite{paragraph:nb,source,locator:"7".into()}).unwrap();
    check!(e.document().cited_source_ids()==vec![source]&&e.document().source_uses(source)[0].paragraph==nb,"citations follow cell notes into bibliography and usage search");
    check!(e.document().plain_text().contains("Nested cell")&&e.document().plain_text().contains("Cell endnote"),"plain text retains all cell note bodies");
    check!(e.document().find_text("Nested cell",SearchOptions::default()).unwrap()[0].paragraph==nb,"search reaches nested cell note bodies");
    let before=e.document().clone();let rev=e.revision();
    check!(e.apply(Edit::InsertNote{paragraph:nb,inline:0,offset:0,kind:NoteKind::Footnote,text:String::new()}).is_err()&&e.document()==&before&&e.revision()==rev,"recursive notes reject atomically");
    for c in [TableChange::RemoveRow(0),TableChange::RemoveColumn(0),TableChange::Delete]{
        change(&mut e,t,c);check!(e.document().validate().is_ok()&&e.document().note(early).is_none()&&e.document().note(nested).is_none()&&e.document().note(main).is_some(),"structural deletion prunes only removed anchors");
        check!(e.undo()&&e.document()==&before,"structural undo restores note graph");
    }
    change(&mut e,t,TableChange::Merge{row:0,column:0,rows:2,columns:2});
    check!(e.document().notes.len()==5&&e.document().note_location(later).is_some(),"merged cells retain note ownership");e.undo();
    let whole=e.document().copy_table(t).unwrap();
    check!(whole.document.notes.len()==4&&whole.document.sources.len()==1&&whole.document.note(main).is_none(),"whole-table copy carries descendants and note-only sources");
    let bytes=doc_io::encode_document_fragment(&whole).unwrap();let whole=doc_io::decode_document_fragment(&bytes).unwrap();
    let mut target=sample();let target_note=note(&mut target,1,NoteKind::Footnote,"Keep me");let dest=insert(&mut target,1,1,1);let dp=cell(&target,dest,0,0);
    target.replace_story_fragment(dp,0,0,whole.clone()).unwrap();
    check!(target.document().validate().is_ok()&&target.document().notes.len()==5&&target.document().note(target_note).is_some()&&target.document().tables.len()==3,"table paste into cell remaps notes and preserves destination notes");
    check!(target.undo()&&target.document().notes.len()==1&&target.redo(),"table clipboard history is atomic");
    let pasted=target.document().clone();target.replace_story_fragment(1,0,0,whole).unwrap();
    check!(target.document().notes.len()==9&&target.document().sources.len()==1,"repeat paste gets fresh notes and reuses identical sources");target.undo();check!(target.document()==&pasted,"repeat paste undo");
    let len=e.document().story_projection(q).unwrap().text.len();let fragment=e.document().copy_story_range(q,0,len).unwrap();
    let target_cell=cell(&target,dest,0,0);target.replace_story_fragment(target_cell,0,0,fragment).unwrap();
    check!(target.document().notes.len()==6&&target.document().validate().is_ok(),"rich cell selection pastes live note into another cell");
    check!(doc_io::decode_docx(&doc_io::encode_docx(target.document()).unwrap())==Ok(target.document().clone()),"DOCX roundtrip for copied nested notes and note citations");
    let original=e.document().clone();let after_body=e.document().story_projection(1).unwrap().text.len();e.apply(Edit::ReplaceStoryText{paragraph:1,start:after_body,end:after_body,text:" tail".into(),paragraph_breaks:false}).unwrap();
    check!(e.document().notes.len()==5,"unrelated body edit cannot prune table notes");e.undo();check!(e.document()==&original,"body text history with cell notes");
    // Every original anchor begins its note on that physical page. Repeated heads
    // render the existing marker but never emit another note body.
    for floating in [false,true]{
        let mut e=sample();let t=insert(&mut e,1,18,2);let mut ids=Vec::new();
        for r in 0..18 {let p=cell(&e,t,r,0);text(&mut e,p,&format!("Row {r:02}"));if [0,3,9,16].contains(&r){let n=note(&mut e,p,NoteKind::Footnote,&format!("Note {r:02}"));ids.push((p,n));}}
        change(&mut e,t,TableChange::HeaderRows(1));
        if floating {change(&mut e,t,TableChange::Position(Some(TablePosition{distance:[0;4],..Default::default()})));}
        let d=e.document();let l=paginated(d);check!(l.pages.len()>=2&&safe(d,&l),"multi-page cell footnotes have reserved disjoint rectangles");
        for (p,n) in &ids {let body=d.note(*n).unwrap().paragraphs[0].id;
            check!(first_page(&l,*p)==first_page(&l,body),"note begins on original anchor page");
            check!(l.pages.iter().flat_map(|p|&p.lines).filter(|line|line.paragraph==body).count()==1,"repeated header notes emitted exactly once");}
        check!(l.pages.iter().flat_map(|p|&p.cells).filter(|c|c.row_end&&!c.repeated).count()==36,"note reservation retains every row end");
        check!(doc_io::decode_docx(&doc_io::encode_docx(d).unwrap())==Ok(d.clone()),"paginated table notes DOCX roundtrip");
        let endp=cell(&e,t,17,0);let en=note(&mut e,endp,NoteKind::Endnote,"After tables");let l=paginated(e.document());let endbody=e.document().note(en).unwrap().paragraphs[0].id;
        check!(first_page(&l,endbody)>=l.pages.iter().rposition(|p|!p.cells.is_empty()),"endnotes follow all floating continuations");
        let nbody=e.document().note(ids[0].1).unwrap().paragraphs[0].id;
        let long=(0..60).map(|i|format!("LONG{i:02}")).collect::<Vec<_>>().join("\n");text(&mut e,nbody,&long);
        let l=paginated(e.document());check!(safe(e.document(),&l)&&l.pages.iter().flat_map(|p|&p.lines).filter(|line|line.paragraph==nbody).count()==60,"long cell note continues without losing lines or floats");
    }
    for floating in [false,true]{
        let mut e=sample();let t=insert(&mut e,1,1,1);let p=cell(&e,t,0,0);text(&mut e,p,&"line\n".repeat(28));
        let n=e.apply(Edit::InsertNote{paragraph:p,inline:0,offset:70,kind:NoteKind::Footnote,text:"Split row note".into()}).unwrap();
        if floating{change(&mut e,t,TableChange::Position(Some(TablePosition{distance:[0;4],..Default::default()})));}
        let l=paginated(e.document());let nb=e.document().note(n).unwrap().paragraphs[0].id;
        check!(safe(e.document(),&l)&&l.pages.len()>1,"oversized row reserves its mid-row note while splitting");
        change(&mut e,t,TableChange::RowSizing{row:0,height:100,rule:RowHeightRule::Exact});let l=paginated(e.document());
        check!(first_page(&l,nb)==Some(0)&&safe(e.document(),&l),"clipped marker retains its footnote on the cell page");
    }
    let l=paginated(e.document());check!(safe(e.document(),&l),"nested note, source and endnote pagination");
    // Exercise changing note-reservation edges at many row/line boundaries.
    for floating in [false,true]{for lines in [1,4,12]{for row in [0,2,5]{
        let mut e=sample();let t=insert(&mut e,1,6,2);
        for r in 0..6{let p=cell(&e,t,r,0);text(&mut e,p,&"Cell\n".repeat(lines));}
        let p=cell(&e,t,row,0);let id=note(&mut e,p,NoteKind::Footnote,"Short note");
        if floating{change(&mut e,t,TableChange::Position(Some(TablePosition{distance:[0;4],..Default::default()})));}
        for header in [0,1]{change(&mut e,t,TableChange::HeaderRows(header));
            let result=doc_layout::paginate(e.document(),&mut Fixed);
            check!(result.is_ok(),"varied header/row/anchor geometry is pageable");
            if let Ok(l)=result{let body=e.document().note(id).unwrap().paragraphs[0].id;
                check!(safe(e.document(),&l)&&first_page(&l,p)==first_page(&l,body),"varied cell note stays on original anchor page");}
        }
    }}}
    let mut e=sample();let second=e.apply(Edit::AddParagraph{after:1}).unwrap();
    let a=insert(&mut e,1,20,1);let b=insert(&mut e,second,5,1);
    for t in [a,b]{change(&mut e,t,TableChange::Position(Some(TablePosition{distance:[0;4],..Default::default()})));
        let ps:Vec<_>=e.document().table(t).unwrap().anchors().map(|(_,_,c)|c.paragraphs[0].id).collect();
        for p in ps{note(&mut e,p,NoteKind::Footnote,"N");}}
    let mut f=e.document().paragraph(second).unwrap().format;f.page_break_before=true;
    e.apply(Edit::SetParagraphFormat{paragraph:second,format:f}).unwrap();
    let result=doc_layout::paginate(e.document(),&mut Fixed);check!(result.is_ok(),"explicit body break cannot skip notes in resumed floats");
    if let Ok(l)=result{check!(safe(e.document(),&l)&&e.document().notes.iter().all(|n|first_page(&l,n.paragraphs[0].id).is_some()),"all notes in competing floating continuations survive");}
    (passed,failed)
}
