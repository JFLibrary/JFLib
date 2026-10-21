//! Geometry, row-grid import and lossless floating-table interchange on all platforms.
use super::*;
use alloc::vec::Vec;
fn table(e:&mut Editor,rows:usize,columns:usize)->Id{e.apply(Edit::InsertTable{paragraph:1,offset:0,rows,columns}).unwrap()}
fn change(e:&mut Editor,t:Id,p:TablePosition){e.apply(Edit::ChangeTable{table:t,change:TableChange::Position(Some(p))}).unwrap();}
fn intersects(a:[i32;4],b:[i32;4])->bool{a[0]<b[0]+b[2]&&b[0]<a[0]+a[2]&&a[1]<b[1]+b[3]&&b[1]<a[1]+a[3]}
pub(super) fn run()->(usize,usize){
    let(mut passed,mut failed)=(0,0);
    macro_rules! check{($v:expr,$name:expr)=>{if $v{passed+=1;}else{failed+=1;plat::write_stdout(format!("FAIL floating/grid: {}\n",$name).as_bytes());}};}
    let bytes=include_bytes!("../../fixtures/varying-grid-tables.rtf");
    let decoded=doc_io::clipboard_rtf_decode(bytes);check!(decoded.is_ok(),"RTF row grids import");
    if let Ok(d)=decoded {
        let t=&d.tables[0];
        check!(t.columns==vec![1800,200,1600,1800],"union grid retains narrow geometric interval");
        check!(t.cell(0,0).unwrap().column_span==2&&t.cell_width(0,0)==Some(2000)&&t.cell_width(1,1)==Some(1800),"visible cell spans map actual boundaries");
        check!(t.anchors().count()==5&&d.plain_text().contains("Gamma"),"no invented cells or dropped text");
        check!(doc_io::decode_docx(&doc_io::encode_docx(&d).unwrap())==Ok(d.clone()),"varying grid native DOCX roundtrip");
        let r=doc_io::clipboard_rtf_decode(&doc_io::clipboard_rtf_encode(&d).unwrap()).unwrap();
        check!(r.tables[0].columns==t.columns&&r.plain_text()==d.plain_text(),"varying grid RTF export/import");
        let h=doc_io::clipboard_html_decode(&doc_io::clipboard_html_encode(&d).unwrap()).unwrap();
        check!(h.tables[0].columns==t.columns&&h.tables[0].anchors().count()==5,"varying grid HTML exchange");
        let mut paste=Editor::new(Document::default()).unwrap();paste.replace_story_fragment(1,0,0,d.copy_table(t.id).unwrap()).unwrap();
        check!(paste.document().tables[0].columns==t.columns,"clipboard preserves fine grid");
        check!(doc_layout::paginate(&d,&mut Fixed).is_ok(),"varying grid layout");
    }
    let vertical=br"{\rtf1\trowd\clvmgf\cellx1800\cellx5400\intbl A\cell B\cell\row\trowd\clvmrg\cellx1800\cellx3200\cellx5400\intbl\cell C\cell D\cell\row\pard End\par}";
    let v=doc_io::clipboard_rtf_decode(vertical);check!(v.is_ok(),"vertical merge across different row grids");
    if let Ok(v)=v{check!(v.tables[0].cell(0,0).unwrap().row_span==2&&v.tables[0].cell_width(1,1)==Some(1400),"vertical continuation uses physical interval");}
    let bad=br"{\rtf1\trowd\clvmgf\cellx1800\cellx5400\intbl A\cell B\cell\row\trowd\clvmrg\cellx1900\cellx5400\intbl\cell C\cell\row}";
    check!(doc_io::clipboard_rtf_decode(bad).is_err(),"misaligned vertical merge rejected");
    let ragged=doc_io::clipboard_rtf_decode(include_bytes!("../../fixtures/ragged-grid-tables.rtf")).unwrap();
    check!(ragged.tables[0].rows[1].grid_before==1&&ragged.tables[0].rows[1].grid_after==1,"different row starts and ends become omitted intervals");
    check!(ragged.tables[0].cell(1,0).is_none()&&ragged.tables[0].anchors().count()==4,"omissions do not create fake cells");
    check!(doc_io::decode_docx(&doc_io::encode_docx(&ragged).unwrap())==Ok(ragged.clone()),"ragged grid DOCX roundtrip");
    for html in [false,true]{
        let data=if html{doc_io::clipboard_html_encode(&ragged)}else{doc_io::clipboard_rtf_encode(&ragged)}.unwrap();
        let back=if html{doc_io::clipboard_html_decode(&data)}else{doc_io::clipboard_rtf_decode(&data)}.unwrap();
        check!(back.tables[0].columns==ragged.tables[0].columns&&back.tables[0].rows[1].grid_before==1&&back.tables[0].rows[1].grid_after==1&&back.tables[0].anchors().count()==4,"ragged RTF/HTML exchange preserves omitted grid");
    }
    let mut gaps=Editor::new(ragged.clone()).unwrap();let gt=gaps.document().tables[0].id;
    check!(gaps.apply(Edit::ChangeTable{table:gt,change:TableChange::Merge{row:1,column:0,rows:1,columns:2}}).is_err(),"merging across omitted space fails without panic");
    for op in [TableChange::InsertColumn(0),TableChange::RemoveColumn(0),TableChange::InsertRow(1),TableChange::RemoveRow(0)]{
        if gaps.apply(Edit::ChangeTable{table:gt,change:op}).is_ok(){check!(gaps.document().validate().is_ok(),"ragged structural edit stays valid");gaps.undo();check!(gaps.document()==&ragged,"ragged structural undo");}
    }
    for variant in 0..16 {
        let mut rtf=String::from("{\\rtf1 ");
        for row in 0..3 {let left=if row==0{0}else{variant*15+row*90};let a=left+1200+variant*30;let b=left+3600-variant*15;
            rtf.push_str(&format!("\\trowd\\trleft{left}\\cellx{a}\\cellx{b}\\cellx5400\\intbl A\\cell B\\cell C\\cell\\row "));}
        rtf.push_str("\\pard End\\par}");
        let d=doc_io::clipboard_rtf_decode(rtf.as_bytes()).unwrap();
        check!(doc_io::decode_docx(&doc_io::encode_docx(&d).unwrap())==Ok(d.clone()),"generated varying/offset row grids DOCX identity");
        let back=doc_io::clipboard_rtf_decode(&doc_io::clipboard_rtf_encode(&d).unwrap()).unwrap();
        check!(d.tables[0].columns==back.tables[0].columns&&d.tables[0].rows.iter().map(|r|(r.grid_before,r.grid_after)).eq(back.tables[0].rows.iter().map(|r|(r.grid_before,r.grid_after))),"generated varying grids RTF physical boundaries");
    }
    let mut e=Editor::new(Document::default()).unwrap();let t=table(&mut e,2,2);
    e.apply(Edit::ChangeTable{table:t,change:TableChange::Columns(vec![1600,1600])}).unwrap();
    e.apply(Edit::SetText{paragraph:1,inline:0,text:"Text neben der Tabelle und danach wieder über die volle Seitenbreite. ".repeat(100)}).unwrap();
    let inline=e.document().clone();let p=TablePosition::default();change(&mut e,t,p);
    let floating=e.document().clone();
    check!(doc_io::decode_docx(&doc_io::encode_docx(&floating).unwrap())==Ok(floating.clone()),"floating DOCX identities and position");
    let copy=doc_io::decode_document_fragment(&doc_io::encode_document_fragment(&floating.copy_table(t).unwrap()).unwrap()).unwrap();
    check!(copy.document.tables[0].position==Some(p),"private clipboard preserves floating position");
    let r=doc_io::clipboard_rtf_decode(&doc_io::clipboard_rtf_encode(&floating).unwrap()).unwrap();
    check!(r.tables[0].position==Some(p),"RTF floating properties roundtrip");
    let layout=doc_layout::paginate(&floating,&mut Fixed);check!(layout.is_ok(),"floating pagination succeeds");
    if let Ok(layout)=layout{
        let page=&layout.pages[0];let c=&page.cells[0];let right=c.rect[0]+3200+120;
        check!(page.lines.iter().find(|l|l.paragraph==1).unwrap().x>=right,"text wraps on right of left float");
        check!(page.lines.iter().any(|l|l.paragraph==1&&l.x==floating.page.left as i32&&l.top>c.rect[1]+c.rect[3]),"full width resumes below float");
        check!(page.lines.iter().filter(|l|l.cell.is_none()).all(|l|l.fragments.iter().all(|f|page.cells.iter().all(|c|!intersects([f.x,l.top,f.width,l.height],c.rect)))),"no text/table overlap");
        check!(layout.pages.len()>1&&layout.pages[1].lines.iter().all(|l|l.x==floating.page.left as i32),"continuation page uses full width");
        check!(page.cells.len()==4&&layout.pages.iter().skip(1).all(|p|p.cells.is_empty()),"float rendered once, no duplicated headers");
    }
    e.undo();check!(e.document()==&inline,"position undo");e.redo();check!(e.document()==&floating,"position redo");
    let right=TablePosition{x:(floating.page.content_width()-3200) as i32,..p};change(&mut e,t,right);
    let l=doc_layout::paginate(e.document(),&mut Fixed).unwrap();check!(l.pages[0].lines.iter().find(|l|l.paragraph==1).unwrap().x==floating.page.left as i32,"right float leaves left text column");
    let abs=TablePosition{horizontal:TableHorizontal::Page,vertical:TableVertical::Page,x:720,y:3600,distance:[90,180,150,60],overlap:true};change(&mut e,t,abs);
    let l=doc_layout::paginate(e.document(),&mut Fixed).unwrap();check!(l.pages[0].cells[0].rect[..2]==[720,3600],"page-relative absolute coordinates");
    let b=doc_io::encode_docx(e.document()).unwrap();check!(doc_io::decode_docx(&b)==Ok(e.document().clone()),"all position fields DOCX roundtrip");
    let r=doc_io::clipboard_rtf_decode(&doc_io::clipboard_rtf_encode(e.document()).unwrap()).unwrap();check!(r.tables[0].position==Some(abs),"all position fields RTF roundtrip");
    for bad in [TablePosition{x:i32::MIN,..p},TablePosition{x:31680,..p},TablePosition{y:31680,..p},TablePosition{distance:[u16::MAX;4],..p}]{
        let revision=e.revision();let before=e.document().clone();check!(e.apply(Edit::ChangeTable{table:t,change:TableChange::Position(Some(bad))}).is_err()&&e.revision()==revision&&e.document()==&before,"invalid position fails atomically");
    }
    change(&mut e,t,p);let cell=e.document().table(t).unwrap().cell(0,0).unwrap().paragraphs[0].id;
    let inner=e.apply(Edit::InsertTable{paragraph:cell,offset:0,rows:1,columns:1}).unwrap();
    check!(e.apply(Edit::ChangeTable{table:inner,change:TableChange::Position(Some(p))}).is_err(),"nested float rejected; inline child allowed");
    let l=doc_layout::paginate(e.document(),&mut Fixed).unwrap();check!(l.pages[0].cells.iter().any(|c|c.table==inner&&c.rect[0]==floating.page.left as i32+90),"floating parent contains editable child");
    let another=table(&mut e,1,1);e.apply(Edit::ChangeTable{table:another,change:TableChange::Columns(vec![3200])}).unwrap();change(&mut e,another,p);
    let l=doc_layout::paginate(e.document(),&mut Fixed).unwrap();let boxes:Vec<_>=l.pages[0].cells.iter().filter(|c|c.table==t||c.table==another).collect();
    check!(boxes.iter().filter(|c|c.table==t).all(|a|boxes.iter().filter(|c|c.table==another).all(|b|!intersects(a.rect,b.rect))),"non-overlap policy places subsequent float below");
    let mut note=Editor::new(floating.clone()).unwrap();note.apply(Edit::InsertNote{paragraph:1,inline:0,offset:10,kind:NoteKind::Footnote,text:"Wissenschaftliche Fußnote".into()}).unwrap();
    let l=doc_layout::paginate(note.document(),&mut Fixed).unwrap();check!(l.pages[0].separators.len()==1,"float and anchored footnote share first page");
    check!(l.pages[0].cells.iter().all(|c|c.rect[1]+c.rect[3]<l.pages[0].separators[0].y),"float clears note reservation");
    let before=note.document().clone();let revision=note.revision();
    let result=note.apply_checked(Edit::ChangeTable{table:t,change:TableChange::RowHeight{row:0,height:31680}},|d|doc_layout::paginate(d,&mut Fixed).map(|_|()).map_err(|_|Error::Limit));
    check!(result.is_ok()&&note.revision()>revision&&doc_layout::paginate(note.document(),&mut Fixed).unwrap().pages.len()>2,"oversized row continues over pages with layout preflight");
    if result.is_ok(){note.undo();}check!(note.document()==&before,"oversized row undo");
    let cell=note.document().table(t).unwrap().cell(0,0).unwrap().paragraphs[0].id;
    let mut fragment=DocumentFragment::default();fragment.document.paragraphs[0].content=vec![Inline::Text("Overflow ".repeat(3000))];
    let result=note.replace_story_fragment_checked(cell,0,0,fragment,|d|doc_layout::paginate(d,&mut Fixed).map(|_|()).map_err(|_|Error::Limit));
    check!(result.is_ok()&&doc_layout::paginate(note.document(),&mut Fixed).unwrap().pages.len()>2,"rich paste grows floating row over pages");
    note.undo();check!(note.document()==&before,"multi-page rich paste is one undo transaction");
    let mut blank=floating.clone();blank.paragraphs[0].content=vec![Inline::Text("\n".repeat(80))];
    let l=doc_layout::paginate(&blank,&mut Fixed).unwrap();
    check!(l.pages.len()>1&&l.pages.iter().flat_map(|p|&p.lines).filter(|l|l.paragraph==1).all(|l|l.start<=l.end&&l.end<=80),"empty trailing lines retain valid caret ranges across pages");
    (passed,failed)
}
