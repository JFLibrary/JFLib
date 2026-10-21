//! Containment, import trust boundaries and native/web-independent document geometry.
use super::*;
fn insert(e:&mut Editor,p:Id,rows:usize,columns:usize)->Id{e.apply(Edit::InsertTable{paragraph:p,offset:0,rows,columns}).unwrap()}
fn paragraph(e:&Editor,t:Id,r:usize,c:usize)->Id{e.document().table(t).unwrap().cell(r,c).unwrap().paragraphs[0].id}
pub(super) fn run()->(usize,usize){
    let(mut passed,mut failed)=(0,0);
    macro_rules! check{($v:expr,$name:expr)=>{if $v{passed+=1;}else{failed+=1;plat::write_stdout(format!("FAIL nested/import: {}\n",$name).as_bytes());}};}
    let mut e=Editor::new(Document::default()).unwrap();let outer=insert(&mut e,1,3,2);let anchor=paragraph(&e,outer,0,0);let inner=insert(&mut e,anchor,2,2);let ip=paragraph(&e,inner,0,0);let leaf=insert(&mut e,ip,1,1);let lp=paragraph(&e,leaf,0,0);
    e.apply(Edit::SetText{paragraph:lp,inline:0,text:String::from("Innere Messung äβ😀")}).unwrap();
    check!(e.document().table_depth(leaf)==Ok(3) && e.document().table_parent(inner)==Some((outer,0,0)),"nested parent/depth");
    let (left,right)=e.document().paragraph_bounds(lp);check!(left==e.document().page.left+270 && right>left,"all ancestor paddings contribute to paragraph origin");
    let d=e.document().clone();let bytes=doc_io::encode_docx(&d).unwrap();check!(doc_io::decode_docx(&bytes)==Ok(d.clone()),"nested DOCX identity roundtrip");
    let native=doc_io::encode_document_fragment(&d.copy_table(outer).unwrap()).unwrap();let copied=doc_io::decode_document_fragment(&native).unwrap();check!(copied.document.tables.len()==3 && copied.document.plain_text().contains("Innere Messung"),"complete subtree copied with reading order");
    let copied_inner=d.copy_table(inner).unwrap();check!(copied_inner.document.tables.len()==2 && copied_inner.document.table_depth(inner)==Ok(1),"inner table copied as independent root");
    let layout=doc_layout::paginate(&d,&mut Fixed).unwrap();let page=&layout.pages[0];let hit=page.cells.iter().find(|c|c.table==leaf).unwrap();
    check!(doc_view::table_cell_at(page,hit.rect[0]+hit.rect[2]/2,hit.rect[1]+hit.rect[3]/2).is_some_and(|c|c.table==leaf),"hit testing chooses deepest containing cell");
    check!(page.lines.iter().any(|l|l.paragraph==lp && l.cell==Some(hit.cell) && l.x>=left as i32),"nested text uses innermost cell and origin");
    check!(page.cells.iter().filter(|c|c.table==outer).all(|c|c.rect[3]>0) && page.cells.iter().find(|c|c.table==outer&&c.row==0&&c.column==0).unwrap().rect[3]>hit.rect[3],"outer row grows around child geometry");
    e.apply(Edit::ChangeTable{table:outer,change:TableChange::RemoveRow(0)}).unwrap();check!(e.document().tables.len()==1,"row deletion removes descendants");e.undo();check!(e.document()==&d,"row subtree deletion undo");
    e.apply(Edit::ChangeTable{table:inner,change:TableChange::Delete}).unwrap();check!(e.document().tables.len()==1,"deleting inner table removes its descendants only");e.undo();check!(e.document()==&d,"subtree delete undo");
    e.apply(Edit::ChangeTable{table:outer,change:TableChange::Merge{row:0,column:0,rows:1,columns:2}}).unwrap();check!(e.document().table_depth(leaf)==Ok(3),"merge preserves nested anchors");e.undo();
    let rev=e.revision();check!(e.apply(Edit::ChangeTable{table:outer,change:TableChange::Columns(vec![420,420])}).is_err()&&e.revision()==rev&&e.document()==&d,"resize cannot clip descendant table");
    let mut cycle=d.clone();cycle.tables[0].before=lp;check!(cycle.validate().is_err(),"cycle rejected before traversal/layout");
    let mut orphan=d.clone();orphan.tables[1].before=u64::MAX;check!(orphan.validate().is_err(),"orphan anchor rejected");
    let mut deep=Editor::new(Document::default()).unwrap();let mut p=1;for _ in 0..MAX_TABLE_DEPTH {let id=insert(&mut deep,p,1,1);p=paragraph(&deep,id,0,0);}let snapshot=deep.document().clone();let rev=deep.revision();
    check!(deep.apply(Edit::InsertTable{paragraph:p,offset:0,rows:1,columns:1}).is_err()&&deep.document()==&snapshot&&deep.revision()==rev,"depth eight accepted, ninth rejected atomically");
    let mut long=Editor::new(Document::default()).unwrap();let outer_long=insert(&mut long,1,1,1);let a=paragraph(&long,outer_long,0,0);let child_long=insert(&mut long,a,12,2);
    long.apply(Edit::ChangeTable{table:child_long,change:TableChange::HeaderRows(1)}).unwrap();
    for row in 0..12 {long.apply(Edit::ChangeTable{table:child_long,change:TableChange::RowHeight{row,height:2200}}).unwrap();}
    let pages=doc_layout::paginate(long.document(),&mut Fixed).unwrap();
    check!(pages.pages.len()>=3,"tall nested table continues across pages");
    check!(pages.pages.iter().skip(1).all(|p|p.cells.iter().any(|c|c.table==child_long&&c.row==0&&c.repeated)),"nested header repeated inside continuing outer cell");
    check!(pages.pages.iter().all(|p|p.cells.iter().all(|c|c.rect[1]>=long.document().page.top as i32 && c.rect[1]+c.rect[3]<=(long.document().page.height-long.document().page.bottom) as i32)),"nested continuation geometry remains within page content");
    let html=br#"<p>Vor <b>fett</b></p><TABLE style='width:100%;border-collapse:collapse'><thead><tr><th colspan=2>Kopf</thead><tbody><tr><td rowspan=2 style='background:#ddebff;border-left:2pt dashed #123456'>A<table><tr><td><i>innen</i></td></tr></table>nach</td><td>B<tr><td>C</tbody></TABLE><p>Ende &amp; &#x3b2;</p>"#;
    let imported=doc_io::clipboard_html_decode(html);check!(imported.is_ok(),"HTML optional end tags, nested table, span and entities");
    if let Ok(h)=imported {
        check!(h.tables.len()==2&&h.tables[0].header_rows==1&&h.tables[0].cell(1,0).unwrap().row_span==2,"HTML grid and header");
        check!(h.tables[0].cell(1,0).unwrap().fill==Some(0xddebff)&&h.tables[0].cell(1,0).unwrap().borders[3].is_some_and(|b|b.style==BorderStyle::Dashed&&b.width==16),"HTML cell fill and independent border");
        check!(h.plain_text().contains("innen")&&h.plain_text().contains("Ende & β"),"HTML text survives hierarchy");
        check!(doc_io::decode_docx(&doc_io::encode_docx(&h).unwrap())==Ok(h.clone()),"HTML to native DOCX roundtrip");
        let mut target=Editor::new(Document::default()).unwrap();target.apply(Edit::SetText{paragraph:1,inline:0,text:String::from("LinksRechts")}).unwrap();let before=target.document().clone();target.replace_story_fragment(1,5,5,DocumentFragment{document:h}).unwrap();
        check!(target.document().plain_text().starts_with("LinksVor")&&target.document().plain_text().ends_with("βRechts"),"mixed HTML paragraphs/tables splice between prefix/suffix");target.undo();check!(target.document()==&before,"mixed import is one undo step");
    }
    for kind in [0,1] {
        let bytes=if kind==0{doc_io::clipboard_rtf_encode(&d)}else{doc_io::clipboard_html_encode(&d)}.unwrap();
        let back=if kind==0{doc_io::clipboard_rtf_decode(&bytes)}else{doc_io::clipboard_html_decode(&bytes)};
        if let Err(error)=&back{plat::write_stdout(format!("import codec {kind}: {error:?}\n").as_bytes());}
        check!(back.is_ok(),"nested rich interchange accepted");if let Ok(back)=back{check!(back.tables.len()==3&&back.plain_text()==d.plain_text(),"RTF/HTML preserve nested text order");check!(back.tables.iter().map(|t|&t.columns).eq(d.tables.iter().map(|t|&t.columns)),"RTF/HTML exact fitted grid widths");}
    }
    let rtf=br"{\rtf1\ansi{\colortbl;\red18\green52\blue86;}\pard Avant\par\trowd\trgaph90\trhdr\clmgf\cellx2400\clmrg\cellx4800\pard\intbl\b Header\cell\pard\intbl\cell\row\trowd\trgaph90\clvmgf\clcbpat1\cellx2400\cellx4800\pard\intbl A\cell\pard\intbl B\cell\row\trowd\trgaph90\clvmrg\cellx2400\cellx4800\pard\intbl\cell\pard\intbl C\cell\row\pard After\par}";
    let r=doc_io::clipboard_rtf_decode(rtf);check!(r.is_ok(),"foreign RTF merge controls and row definitions");if let Ok(r)=r{check!(r.tables[0].cell(0,0).unwrap().column_span==2&&r.tables[0].cell(1,0).unwrap().row_span==2&&r.tables[0].header_rows==1,"RTF spans/header represented");check!(r.tables[0].cell(1,0).unwrap().fill==Some(0x123456),"RTF color table to cell fill");}
    for bad in [b"<table><tr><td rowspan=3>x</table>".as_slice(),b"<table><tr><td colspan=999>x</table>",b"<table><tr><td style='padding:10pt 4pt'>x</table>",b"<table><tr><td><img src='file:///never'></table>",b"<p>&#0;</p>",b"<table><tr><td style='width:1pt;border:999999pt solid red'>x</table>"]{check!(doc_io::clipboard_html_decode(bad).is_err(),"malformed/unsupported HTML rejected");}
    for (css,width) in [("0.375pt solid #123456",3),("1.125pt double #123456",3),("0.875pt dashed #123456",7)] {
        let bytes=format!("<table><tr><td style='border-left:{css}'>x</td></tr></table>");let h=doc_io::clipboard_html_decode(bytes.as_bytes()).unwrap();
        check!(h.tables[0].cell(0,0).unwrap().borders[3].unwrap().width==width,"HTML fractional border widths retain eighth-point precision");
    }
    let active=doc_io::clipboard_html_decode(b"<script>throw 'never executed'</script><p>safe</p>").unwrap();check!(active.plain_text()=="safe","active content ignored without execution");
    for bad in [br"{\rtf1\trowd\cellx1000\intbl X\row}".as_slice(),br"{\rtf1\trowd\cellx1000\intbl X\cell}",br"{\rtf1\itap9 X\nestcell}",br"{\rtf1\trowd\cellx1000\cellx500\intbl X\cell\cell\row}"]{check!(doc_io::clipboard_rtf_decode(bad).is_err(),"malformed RTF table rejected");}
    (passed,failed)
}
