// Shared header pagination, style precedence, clipboard and public-format regressions.
use super::*;
fn edit(e:&mut Editor,id:Id,change:TableChange){e.apply(Edit::ChangeTable{table:id,change}).unwrap();}
pub(super) fn run()->(usize,usize){
    let(mut passed,mut failed)=(0,0);
    macro_rules! check{($v:expr,$label:expr)=>{if $v {passed+=1;}else{failed+=1;plat::write_stdout(format!("FAIL table styles: {}\n",$label).as_bytes());}};}
    let mut d=Document::default();d.page.height=4320;d.page.top=720;d.page.bottom=720;
    let mut e=Editor::new(d).unwrap();let id=e.apply(Edit::InsertTable{paragraph:1,offset:0,rows:22,columns:3}).unwrap();
    let t=e.document().table(id).unwrap().clone();
    for(r,c,cell)in t.anchors(){e.apply(Edit::SetText{paragraph:cell.paragraphs[0].id,inline:0,text:format!("R{r}C{c}")}).unwrap();}
    let original=e.document().clone();
    edit(&mut e,id,TableChange::HeaderRows(2));
    edit(&mut e,id,TableChange::Style{style:Some(String::from("jfTableBlue")),look:TableLook::default()});
    let styled=e.document().clone();let t=styled.table(id).unwrap();let p=&t.cell(0,0).unwrap().paragraphs[0];
    check!(styled.table_region(t,0,0).fill==Some(0x196180),"first row shading");
    check!(styled.resolve_paragraph_text(p,TextFormat::default()).bold,"header text inherits bold");
    check!(styled.resolve_paragraph_text(p,TextFormat::default()).color==0xffffffff,"header text inherits white");
    check!(styled.resolve_paragraph_text(p,TextFormat{bold:Some(false),color:Some(TextColor::Rgb(0x123456)),..Default::default()}).color==0xff123456,"direct color wins");
    check!(styled.table_region(t,2,0).fill!=styled.table_region(t,3,0).fill,"alternating body banding");
    let layout=doc_layout::paginate(&styled,&mut Fixed).unwrap();
    check!(layout.pages.len()>2,"multiple continuation pages");
    let header_ids:alloc::vec::Vec<_>=t.anchors().filter(|(r,_,_)|*r<2).map(|(_,_,c)|c.id).collect();
    let all_originals:alloc::vec::Vec<_>=layout.pages.iter().flat_map(|p|&p.cells).filter(|c|!c.repeated).map(|c|c.cell).collect();
    check!(all_originals.iter().copied().collect::<alloc::collections::BTreeSet<_>>().len()==66 && layout.pages.iter().flat_map(|p|&p.cells).filter(|c|!c.repeated&&c.row_end).count()==66,"display copies and split fragments retain the original cells and one final edge");
    for(i,page)in layout.pages.iter().enumerate().filter(|(_,p)|!p.cells.is_empty()){
        let heads:alloc::vec::Vec<_>=page.cells.iter().filter(|c|c.row<2).collect();
        check!(heads.len()==6 && heads.iter().all(|c|c.repeated==(i>0)),"full header prefix repeats once");
        check!(heads.iter().map(|c|c.cell).collect::<alloc::vec::Vec<_>>()==header_ids,"header identities remain stable");
        let end=heads.iter().map(|c|c.rect[1]+c.rect[3]).max().unwrap();
        check!(page.cells.iter().filter(|c|c.row>=2).all(|c|c.rect[1]>=end),"body cannot overlap header");
        check!(page.cells.iter().all(|c|c.rect[1]>=720 && c.rect[1]+c.rect[3]<=3600),"all cells remain inside page");
        check!(page.lines.iter().all(|l|l.fragments.iter().all(|f|f.baseline>=l.top && f.baseline<=l.top+l.height)),"baseline positions follow repeated cells");
    }
    check!(styled.plain_text()==original.plain_text(),"repetition never duplicates document text");
    let bytes=doc_io::encode_docx(&styled).unwrap();
    check!(doc_io::decode_docx(&bytes)==Ok(styled.clone()),"header and preset DOCX exact model roundtrip");
    let rtf=String::from_utf8(doc_io::clipboard_rtf_encode(&styled).unwrap()).unwrap();
    check!(rtf.matches("\\trhdr ").count()==2,"RTF marks real header rows only");
    let html=String::from_utf8(doc_io::clipboard_html_encode(&styled).unwrap()).unwrap();
    check!(html.matches("<thead>").count()==1 && html.matches("<th ").count()==6 && html.contains("#196180"),"HTML semantic header and effective style");
    let clip=styled.copy_story_range(p.id,0,4).unwrap();
    let cp=&clip.document.paragraphs[0];
    check!(clip.document.resolve_paragraph_text(cp,cp.content[0].format()).bold,"copied cell text keeps effective template formatting");
    let revision=e.revision();
    check!(e.apply(Edit::ChangeTable{table:id,change:TableChange::Merge{row:1,column:0,rows:2,columns:1}}).is_err() && e.revision()==revision,"merge across header boundary rejected atomically");
    edit(&mut e,id,TableChange::Merge{row:0,column:0,rows:2,columns:2});
    let merged=e.document().clone();let ml=doc_layout::paginate(&merged,&mut Fixed).unwrap();
    check!(ml.pages.iter().filter(|p|!p.cells.is_empty()).all(|p|p.cells.iter().filter(|c|c.row<2).count()==3),"merged header repeats as merged anchors");
    check!(doc_io::decode_docx(&doc_io::encode_docx(&merged).unwrap())==Ok(merged.clone()),"merged header DOCX roundtrip");
    check!(e.apply(Edit::ChangeTable{table:id,change:TableChange::HeaderRows(1)}).is_err(),"header boundary cannot split a vertical merge");
    edit(&mut e,id,TableChange::InsertRow(1));check!(e.document().table(id).unwrap().header_rows==3,"insertion inside header extends prefix");
    edit(&mut e,id,TableChange::RemoveRow(0));check!(e.document().table(id).unwrap().header_rows==2,"deleting header shrinks prefix");
    e.undo();e.undo();check!(e.document()==&merged,"header reshape exact undo");
    edit(&mut e,id,TableChange::InsertRow(2));check!(e.document().table(id).unwrap().header_rows==2,"insertion at boundary starts body");e.undo();
    let mut style=e.document().table_style("jfTableBlue").unwrap();style.id=String::from("ResearchTable");style.name=String::from("Messreihe");
    style.first_row.fill=Some(0x304020);style.band1.fill=Some(0xeef4db);style.border_color=0x556644;
    e.apply(Edit::SetTableStyle{style:style.clone(),table:Some(id)}).unwrap();
    let custom=e.document().clone();
    check!(custom.table_region(custom.table(id).unwrap(),0,0).fill==Some(0x304020),"custom style applied in same transaction");
    check!(doc_io::decode_docx(&doc_io::encode_docx(&custom).unwrap())==Ok(custom.clone()),"named style full DOCX roundtrip");
    check!(e.undo() && e.document()==&merged && e.redo() && e.document()==&custom,"definition and assignment undo together");
    edit(&mut e,id,TableChange::Cell{row:0,column:0,fill:Some(0xffffff),alignment:CellAlignment::Top});
    style.first_row.fill=Some(0x882211);e.apply(Edit::SetTableStyle{style:style.clone(),table:None}).unwrap();
    let d=e.document();let t=d.table(id).unwrap();
    check!(d.table_region(t,0,0).fill==Some(0xffffff) && d.table_region(t,0,2).fill==Some(0x882211),"direct white survives live template changes");
    edit(&mut e,id,TableChange::Cell{row:0,column:0,fill:None,alignment:CellAlignment::Top});
    check!(e.document().table_region(e.document().table(id).unwrap(),0,0).fill==Some(0x882211),"clear cell override restores inherited color");
    edit(&mut e,id,TableChange::Appearance{padding:90,border_width:12,border_color:0x123456});
    check!(e.document().table_border(e.document().table(id).unwrap())==(12,0x123456),"direct border wins over template");
    check!(doc_io::decode_docx(&doc_io::encode_docx(e.document()).unwrap())==Ok(e.document().clone()),"direct border survives DOCX");
    let snapshot=e.document().clone();let revision=e.revision();
    style.name=String::from("Blau · Zeilenbänder");
    check!(e.apply(Edit::SetTableStyle{style,table:None}).is_err() && e.revision()==revision && e.document()==&snapshot,"duplicate name rejected without mutation");
    for count in [23,256] {check!(e.apply(Edit::ChangeTable{table:id,change:TableChange::HeaderRows(count)}).is_err() && e.document()==&snapshot,"out-of-range prefix rejected");}
    check!(e.apply(Edit::ChangeTable{table:id,change:TableChange::Style{style:Some(String::from("missing")),look:Default::default()}}).is_err() && e.document()==&snapshot,"missing template rejected");
    // A one-cell table overlaps every conditional region. OOXML gives the last row precedence.
    let mut one=Editor::new(Document::default()).unwrap();
    let tid=one.apply(Edit::InsertTable{paragraph:1,offset:0,rows:1,columns:1}).unwrap();
    let mut s=doc_core::TableStyle::new("Overlap", "Überlappende Bereiche");
    s.body.fill=Some(0xffffff);s.band1.fill=Some(0x112233);
    s.first_column.fill=Some(0x223344);s.last_column.fill=Some(0x334455);
    s.first_row.fill=Some(0x445566);s.last_row.fill=Some(0x556677);
    one.apply(Edit::SetTableStyle{style:s,table:Some(tid)}).unwrap();
    let mut look=TableLook{first_row:true,last_row:true,first_column:true,last_column:true,banded_rows:true,banded_columns:false};
    for expected in [0x556677,0x445566,0x334455,0x223344,0x112233] {
        edit(&mut one,tid,TableChange::Look(look));
        check!(one.document().table_region(one.document().table(tid).unwrap(),0,0).fill==Some(expected),"conditional region precedence for overlapping edges");
        if look.last_row {look.last_row=false;}else if look.first_row {look.first_row=false;}else if look.last_column {look.last_column=false;}else{look.first_column=false;}
    }
    edit(&mut one,tid,TableChange::Cell{row:0,column:0,fill:Some(0xffffff),alignment:CellAlignment::Top});
    check!(one.document().table_region(one.document().table(tid).unwrap(),0,0).fill==Some(0xffffff),"direct cell shading wins over every conditional region");
    // A header moves with the first body row instead of being stranded after preceding text.
    let mut d=styled.clone();d.paragraphs.insert(0,Paragraph{id:1000,style:ParagraphStyle::Normal,format:Default::default(),content:vec![Inline::Text(String::from("intro\nintro\nintro\nintro\nintro\nintro\nintro\nintro\nintro"))]});
    let layout=doc_layout::paginate(&d,&mut Fixed).unwrap();
    check!(layout.pages[0].cells.is_empty() && layout.pages[1].cells.iter().any(|c|c.row==2),"header kept with first body row");
    // Oversized connected body bands continue with a complete header and no clipped lines.
    let mut e=Editor::new(styled.clone()).unwrap();edit(&mut e,id,TableChange::Merge{row:2,column:0,rows:14,columns:1});
    let d=e.document();let l=doc_layout::paginate(d,&mut Fixed).unwrap();
    check!(l.pages.iter().filter(|p|!p.cells.is_empty()).all(|p|p.cells.iter().filter(|c|c.row<2).count()==6),"headers repeat across oversized merged bands");
    let original_lines:alloc::vec::Vec<_>=l.pages.iter().flat_map(|p|p.lines.iter().filter(|line|p.cells.iter().any(|c|Some(c.cell)==line.cell && !c.repeated))).map(|l|(l.paragraph,l.start,l.end)).collect();
    check!(original_lines.len()==66,"merged continuation keeps each original text line exactly once");
    let mut too_tall=styled.clone();too_tall.tables[0].rows[0].min_height=2880;
    check!(doc_layout::paginate(&too_tall,&mut Fixed).is_err(),"oversized header reports finite layout limit");
    let mut no_repeat=Editor::new(styled).unwrap();edit(&mut no_repeat,id,TableChange::HeaderRows(0));
    check!(doc_layout::paginate(no_repeat.document(),&mut Fixed).unwrap().pages.iter().flat_map(|p|&p.cells).all(|c|!c.repeated),"disabling repeat removes every copy");
    (passed,failed)
}
