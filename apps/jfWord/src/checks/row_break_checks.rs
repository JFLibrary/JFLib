//! Row keep rules, codec exchange, nested constraints and editable continuations.
use super::*;
use alloc::vec::Vec;
fn change(e:&mut Editor,t:Id,change:TableChange){e.apply(Edit::ChangeTable{table:t,change}).unwrap();}
fn setup(floating:bool)->(Editor,Id){
    let html=format!("<div data-jf-page=\"1 8640 5760 720 720 720 720\"><p style=\"margin-top:140pt\">Vortext</p><table {} style=\"width:144pt\"><tr style=\"height:100pt\"><td>Alpha</td><td>Beta</td></tr><tr style=\"height:100pt\"><td>Gamma</td><td>Delta</td></tr></table><p>Danach</p></div>",if floating{"data-jf-table-position=\"1 margin paragraph 0 0 0 0 0 0 0\""}else{""});
    let d=doc_io::clipboard_html_decode(html.as_bytes()).unwrap();let id=d.tables[0].id;(Editor::new(d).unwrap(),id)
}
pub(super) fn run()->(usize,usize){
    let(mut passed,mut failed)=(0,0);
    macro_rules! check{($v:expr,$name:expr)=>{if $v{passed+=1;}else{failed+=1;plat::write_stdout(format!("FAIL row breaks: {}\n",$name).as_bytes());}};}
    for floating in [false,true]{
        let(mut e,t)=setup(floating);let cell=e.document().table(t).unwrap().cell(0,0).unwrap().id;
        let split=doc_layout::paginate(e.document(),&mut Fixed).unwrap();
        check!(split.pages[0].cells.iter().any(|c|c.cell==cell&&!c.row_end),"allowed row can use the remaining page space");
        let original=e.document().clone();change(&mut e,t,TableChange::RowBreak{row:0,allow_split:false});
        let d=e.document().clone();let kept=doc_layout::paginate(&d,&mut Fixed).unwrap();
        check!(!kept.pages[0].cells.iter().any(|c|c.cell==cell),"kept row starts on next page");
        check!(kept.pages.iter().flat_map(|p|&p.cells).filter(|c|c.cell==cell).count()==1,"kept row fits whole on a fresh page");
        check!(e.undo()&&e.document()==&original,"row policy undo");
        check!(e.redo()&&e.document()==&d,"row policy redo");
        check!(doc_io::decode_docx(&doc_io::encode_docx(&d).unwrap())==Ok(d.clone()),"DOCX cantSplit roundtrip");
        for html in [false,true]{let bytes=if html{doc_io::clipboard_html_encode(&d)}else{doc_io::clipboard_rtf_encode(&d)}.unwrap();
            let r=if html{doc_io::clipboard_html_decode(&bytes)}else{doc_io::clipboard_rtf_decode(&bytes)}.unwrap();
            check!(r.tables[0].rows[0].cant_split&&!r.tables[0].rows[1].cant_split,"HTML/RTF keep flag does not leak into following rows");}
        let mut paste=Editor::new(Document::default()).unwrap();paste.replace_story_fragment(1,0,0,d.copy_table(t).unwrap()).unwrap();
        check!(paste.document().tables[0].rows[0].cant_split,"whole-table clipboard retains row policy");
        let revision=e.revision();check!(e.apply(Edit::ChangeTable{table:t,change:TableChange::RowBreak{row:99,allow_split:false}}).is_err()&&e.revision()==revision&&e.document()==&d,"invalid row is atomic");
        let p=e.document().table(t).unwrap().cell(0,0).unwrap().paragraphs[0].id;
        let text="Weiterlaufender Text ".repeat(110);e.apply(Edit::SetText{paragraph:p,inline:0,text:text.clone()}).unwrap();
        change(&mut e,t,TableChange::RowHeight{row:0,height:13000});
        let l=doc_layout::paginate(e.document(),&mut Fixed).unwrap();
        check!(l.pages.len()>3&&!l.pages[0].cells.iter().any(|c|c.cell==cell),"oversized kept row starts fresh then continues");
        let lines:Vec<_>=l.pages.iter().flat_map(|p|&p.lines).filter(|l|l.paragraph==p).collect();
        check!(lines.first().unwrap().start==0&&lines.last().unwrap().end==text.len()&&lines.windows(2).all(|a|a[0].end==a[1].start),"no source text lost in oversized kept row");
        check!(l.pages.iter().flat_map(|p|&p.cells).filter(|c|c.cell==cell&&c.row_end).count()==1,"one real resize edge for oversized kept row");
    }
    for floating in [false,true] {
        let (mut e,t)=setup(floating);change(&mut e,t,TableChange::Merge{row:0,column:0,rows:2,columns:1});
        change(&mut e,t,TableChange::RowBreak{row:0,allow_split:false});
        let cell=e.document().table(t).unwrap().cell(0,0).unwrap().id;
        let l=doc_layout::paginate(e.document(),&mut Fixed).unwrap();
        check!(!l.pages[0].cells.iter().any(|c|c.cell==cell),"kept first row respects vertical merge");
        check!(l.pages.iter().flat_map(|p|&p.cells).filter(|c|c.cell==cell&&c.row_end).count()==1,"merged row has one true last edge");
        change(&mut e,t,TableChange::InsertColumn(1));change(&mut e,t,TableChange::RemoveColumn(1));
        check!(e.document().table(t).unwrap().rows[0].cant_split,"column reshape preserves row properties");
        let p=e.document().table(t).unwrap().cell(0,0).unwrap().paragraphs[0].id;
        e.apply(Edit::SetText{paragraph:p,inline:0,text:"Bearbeitung nach Umbruch".into()}).unwrap();
        check!(doc_layout::paginate(e.document(),&mut Fixed).is_ok(),"merged kept row remains editable");
    }
    // Nested kept rows constrain a much taller enclosing row; repeated headers stay intact.
    for floating in [false,true] {
        let rows=(0..12).map(|n|format!("<tr style=\"height:42pt;break-inside:avoid\"><td>Inner {n}</td></tr>")).collect::<String>();
        let h=format!("<div data-jf-page=\"1 8640 5760 720 720 720 720\"><table {}><tr><td><table><thead><tr><td>Kopf</td></tr></thead><tbody>{rows}</tbody></table></td></tr></table></div>",if floating {"data-jf-table-position=\"1 margin paragraph 0 0 0 0 0 0 0\""}else{""});
        let d=doc_io::clipboard_html_decode(h.as_bytes()).unwrap();let inner=d.tables[1].id;let l=doc_layout::paginate(&d,&mut Fixed).unwrap();
        check!(l.pages.len()>2,"nested kept rows still paginate");
        for row in &d.tables[1].rows[1..] {let cell=row.cells[0].id;check!(l.pages.iter().flat_map(|p|&p.cells).filter(|c|c.cell==cell).count()==1,"nested kept row remains whole");}
        check!(l.pages.iter().skip(1).filter(|p|p.cells.iter().any(|c|c.table==inner)).all(|p|p.cells.iter().any(|c|c.table==inner&&c.repeated)),"nested header repeats before kept rows");
        check!(l.pages.iter().all(|p|p.cells.iter().all(|c|c.rect[1]>=720&&c.rect[1]+c.rect[3]<=5040)),"nested geometry stays on page");
    }
    for (css,expected) in [("break-inside:avoid",true),("page-break-inside:avoid",true),("break-inside:avoid-page",true),("break-inside:auto;page-break-inside:avoid",false),("break-inside:AVOID",true),("break-inside:avoid-column",false)]{
        let s=format!("<table><tr style=\"{css}\"><td><table><tr><td>Inner</td></tr></table></td></tr></table>");
        let d=doc_io::clipboard_html_decode(s.as_bytes()).unwrap();
        check!(d.tables[0].rows[0].cant_split==expected&&!d.tables[1].rows[0].cant_split,"CSS row break applies locally, modern property takes precedence");
    }
    for css in ["always","inherit","calc(1)"]{let s=format!("<table><tr style=\"break-inside:{css}\"><td>x</td></tr></table>");check!(doc_io::clipboard_html_decode(s.as_bytes()).is_err(),"unsupported CSS break rule rejected");}
    let impossible=include_bytes!("../../fixtures/interleaved-row-breaks.html");
    for floating in [false,true]{
        let mut d=doc_io::clipboard_html_decode(impossible).unwrap();if floating{d.tables[0].position=Some(TablePosition{distance:[0;4],..TablePosition::default()});}
        let layout=doc_layout::paginate(&d,&mut Fixed);
        check!(layout.is_ok(),"interleaved nested keep rules relax only when no common page edge exists");
        if let Ok(layout)=layout{let text=layout.pages.iter().flat_map(|p|&p.lines).flat_map(|l|&l.fragments).map(|f|f.text.as_str()).collect::<String>();
            check!(layout.pages[0].lines.iter().any(|l|l.cell.is_some()),"impossible keep group does not create an empty padding-only page");
            check!(["Links","Links 2","Rechts","Rechts 2"].iter().all(|s|text.contains(s)),"conflicting kept rows preserve all text");}
    }
    let r=doc_io::clipboard_rtf_decode(br"{\rtf1\trowd\trkeep\trkeep0\cellx1800\intbl A\cell\row\trowd\trkeep\cellx1800\intbl B\cell\row\trowd\cellx1800\intbl C\cell\row}").unwrap();
    check!(r.tables[0].rows.iter().map(|r|r.cant_split).collect::<Vec<_>>()==vec![false,true,false],"RTF explicit off and trowd reset");
    check!(doc_io::clipboard_rtf_decode(br"{\rtf1\trowd\trkeep2\cellx1800\intbl A\cell\row}").is_err(),"invalid RTF keep switch rejected");
    (passed,failed)
}
