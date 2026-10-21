// Per-edge semantics and complete structural clipboard transfer, shared by every host.
use super::*;
fn change(e:&mut Editor,id:Id,c:TableChange){e.apply(Edit::ChangeTable{table:id,change:c}).unwrap();}
fn border(style:BorderStyle,width:u16,color:u32)->CellBorder{CellBorder{style,width,color}}
pub(super) fn run()->(usize,usize){
    let(mut passed,mut failed)=(0,0);
    macro_rules! check{($v:expr,$name:expr)=>{if $v{passed+=1;}else{failed+=1;plat::write_stdout(format!("FAIL cell borders/clipboard: {}\n",$name).as_bytes());}};}
    let single=border(BorderStyle::Single,8,0x123456);let double=border(BorderStyle::Double,8,0x286080);
    for b in [single,double,border(BorderStyle::Dotted,96,0),border(BorderStyle::Dashed,2,0xffffff),CellBorder::NIL,border(BorderStyle::None,0,0)]{check!(b.validate().is_ok(),"accepted border declaration");}
    for b in [border(BorderStyle::Single,0,0),border(BorderStyle::Single,1,0),border(BorderStyle::Single,97,0),border(BorderStyle::Nil,8,0),border(BorderStyle::None,0,1),border(BorderStyle::Dotted,8,0x1000000)]{check!(b.validate().is_err(),"invalid border declaration");}
    check!(single.competing(CellBorder::NIL)==CellBorder::NIL,"nil suppresses opposing border");
    check!(single.competing(double)==double,"double weight");
    check!(single.competing(border(BorderStyle::Dotted,80,0))==single,"dotted conflict weight independent of width");
    check!(border(BorderStyle::Single,24,0).competing(double).style==BorderStyle::Single,"equal-weight style precedence");
    check!(border(BorderStyle::Single,8,0xffffff).competing(single)==single,"brightness tie break");
    let mut e=Editor::new(Document::default()).unwrap();let id=e.apply(Edit::InsertTable{paragraph:1,offset:0,rows:5,columns:3}).unwrap();
    let edges=[Some(double),Some(single),Some(border(BorderStyle::Dashed,12,0xbc3456)),Some(border(BorderStyle::Dotted,6,0x237833))];
    change(&mut e,id,TableChange::Borders{row:0,column:0,borders:edges});
    let d=e.document();let t=d.table(id).unwrap();
    check!(t.cell(0,0).unwrap().borders==edges,"all independent edges committed");
    check!(d.shared_cell_border(t,0,1,BorderSide::Left,0)==single,"direct opposite overrides table default");
    check!(d.shared_cell_border(t,0,0,BorderSide::Top,usize::MAX)==CellBorder::NIL,"segment bounds checked before arithmetic");
    let saved=d.clone();e.undo();check!(e.document().table(id).unwrap().cell(0,0).unwrap().borders==[None;4],"border undo");e.redo();check!(e.document()==&saved,"border redo");
    let mut opposing=[None;4];opposing[3]=Some(CellBorder::NIL);change(&mut e,id,TableChange::Borders{row:0,column:1,borders:opposing});
    check!(e.document().shared_cell_border(e.document().table(id).unwrap(),0,0,BorderSide::Right,0)==CellBorder::NIL,"nil hides a shared edge from either side");
    opposing[3]=Some(border(BorderStyle::None,0,0));change(&mut e,id,TableChange::Borders{row:0,column:1,borders:opposing});
    check!(e.document().shared_cell_border(e.document().table(id).unwrap(),0,1,BorderSide::Left,0)==single,"none falls back to table and loses to direct neighbour");
    let mut right=[None;4];right[1]=edges[1];right[2]=edges[2];change(&mut e,id,TableChange::Borders{row:1,column:1,borders:right});
    change(&mut e,id,TableChange::Merge{row:0,column:0,rows:2,columns:2});
    let merged=e.document().table(id).unwrap().cell(0,0).unwrap();
    check!(merged.borders[0]==edges[0] && merged.borders[3]==edges[3],"merge preserves leading top/left");
    change(&mut e,id,TableChange::Borders{row:1,column:1,borders:edges});
    check!(e.document().table(id).unwrap().cell(0,0).unwrap().borders==edges,"covered coordinate edits the merged anchor");
    for (r,b) in [(0,single),(1,double)]{let mut bds=[None;4];bds[3]=Some(b);change(&mut e,id,TableChange::Borders{row:r,column:2,borders:bds});}
    let layout=doc_layout::paginate(e.document(),&mut Fixed).unwrap();
    let boxed=layout.pages.iter().flat_map(|p|&p.cells).find(|c|c.row==0&&c.column==0).unwrap();
    let segments:alloc::vec::Vec<_>=boxed.edges.iter().filter(|e|e.side==BorderSide::Right).collect();
    check!(segments.len()==2 && segments[0].border==single && segments[1].border==double && segments[1].offset==segments[0].length,"merged edge resolves distinct neighboring row segments");
    let merged_doc=e.document().clone();change(&mut e,id,TableChange::Split{row:0,column:0});
    let t=e.document().table(id).unwrap();
    check!(t.cell(0,0).unwrap().borders==[edges[0],None,None,edges[3]] && t.cell(1,1).unwrap().borders==[None,edges[1],edges[2],None],"split distributes only original perimeter");
    e.undo();check!(e.document()==&merged_doc,"merged borders restored by undo");
    let p=e.document().table(id).unwrap().cell(0,0).unwrap().paragraphs[0].id;
    e.apply(Edit::SetText{paragraph:p,inline:0,text:String::from("Messung äöü")}).unwrap();
    let source=Source{kind:SourceKind::Book,title:String::from("Studie"),year:String::from("2026"),authors:vec![Person{given:String::from("Ada"),family:String::from("Lovelace"),..Default::default()}],..Default::default()};
    let sid=e.apply(Edit::PutSource(source.clone())).unwrap();
    e.apply(Edit::InsertCitation{paragraph:p,inline:0,offset:14,items:vec![CitationItem{source:sid,..Default::default()}]}).unwrap();
    let base=TableStyle::derived("ClipboardBase","Clipboard Base","jfTableBlue");let child=TableStyle::derived("ClipboardChild","Clipboard Child","ClipboardBase");
    e.apply(Edit::SetTableStyle{style:base.clone(),table:None}).unwrap();e.apply(Edit::SetTableStyle{style:child,table:Some(id)}).unwrap();
    change(&mut e,id,TableChange::HeaderRows(2));
    let snapshot=e.document().clone();let revision=e.revision();let mut bad=edges;bad[0]=Some(border(BorderStyle::Single,0,0));
    let mut flowing=snapshot.clone();flowing.tables[0].rows[0].min_height=3000;flowing.tables[0].rows[1].min_height=3000;flowing.tables[0].rows[2].min_height=12000;
    let flow=doc_layout::paginate(&flowing,&mut Fixed).unwrap();
    let pieces:alloc::vec::Vec<_>=flow.pages.iter().flat_map(|p|&p.cells).filter(|c|c.row==2&&c.column==0).collect();
    check!(pieces.len()>1 && pieces.iter().map(|c|c.rect[3]).sum::<i32>()==12000,"tall cell remains complete through page continuations");
    check!(pieces.iter().filter(|c|c.edges.iter().any(|e|e.side==BorderSide::Top)).count()==1 && pieces.iter().filter(|c|c.edges.iter().any(|e|e.side==BorderSide::Bottom)).count()==1,"page continuation does not invent horizontal cell borders");
    check!(flow.pages.iter().skip(1).any(|p|p.cells.iter().any(|c|c.repeated&&c.row==0&&c.column==0&&c.edges.iter().any(|e|e.side==BorderSide::Top&&e.border==double))),"repeated merged headers retain edge geometry");
    check!(e.apply(Edit::ChangeTable{table:id,change:TableChange::Borders{row:0,column:0,borders:bad}}).is_err() && e.revision()==revision && e.document()==&snapshot,"bad edges fail atomically");
    let bytes=doc_io::encode_docx(&snapshot).unwrap();check!(doc_io::decode_docx(&bytes)==Ok(snapshot.clone()),"DOCX retains physical edges including vertical continuations");
    let copy=snapshot.copy_table(id).unwrap();let clip=doc_io::encode_document_fragment(&copy).unwrap();
    check!(doc_io::decode_document_fragment(&clip)==Ok(copy.clone()),"native whole-table clipboard roundtrip");
    check!(copy.document.sources.len()==1 && copy.document.tables[0].header_rows==2 && copy.document.tables[0].cell(0,0).unwrap().borders==edges,"whole-table copy retains headers sources spans borders");
    let display=doc_io::clipboard_display_fragment(&copy,&snapshot).unwrap();check!(display.sources.is_empty() && display.plain_text().contains("Lovelace"),"external table display materializes citation fields");
    let rtf=String::from_utf8(doc_io::clipboard_rtf_encode(&display).unwrap()).unwrap();let html=String::from_utf8(doc_io::clipboard_html_encode(&display).unwrap()).unwrap();
    check!(rtf.contains("\\brdrdb") && rtf.contains("\\brdrdash") && rtf.contains("\\brdrdot") && rtf.contains("\\trhdr"),"RTF includes distinct edges and repeated headers");
    check!(html.contains("border-top:3.000pt double #286080") && html.contains("rowspan=\"2\"") && html.contains("<thead>"),"HTML preserves merged grid and total double-border thickness");
    let mut dest=Document::default();dest.paragraphs[0].content=vec![Inline::Text(String::from("VorNach"))];
    let mut conflicting=base;conflicting.body.fill=Some(0xff0000);dest.table_styles.push(conflicting);
    let mut normal=dest.style_definition(ParagraphStyle::Normal).unwrap();normal.text.italic=Some(true);dest.styles.push(normal);
    dest.sources.push(Source{id:sid,title:String::from("Andere Quelle"),..source});
    let mut into=Editor::new(dest.clone()).unwrap();let (tail,offset)=into.replace_story_fragment(1,3,3,copy.clone()).unwrap();let pasted=into.document().clone();let t=&pasted.tables[0];
    check!(offset==0 && pasted.paragraphs.len()==2 && pasted.paragraph_text(&pasted.paragraphs[0])=="Vor" && pasted.paragraph_text(&pasted.paragraphs[1])=="Nach" && t.before==tail,"insert between prefix and suffix returns body caret");
    check!(t.id!=id && t.cell(0,0).unwrap().id!=snapshot.table(id).unwrap().cell(0,0).unwrap().id && pasted.validate().is_ok(),"all incoming identities remapped");
    check!(pasted.sources.len()==2 && pasted.sources.iter().any(|s|s.id!=sid && s.title=="Studie"),"conflicting source id remapped without replacing destination");
    let ps=pasted.table_style(t.style.as_ref().unwrap()).unwrap();let parent=pasted.table_style(ps.based_on.as_ref().unwrap()).unwrap();
    check!(parent.id!="ClipboardBase" && parent.based_on.as_deref()==Some("jfTableBlue") && pasted.table_style("ClipboardBase").unwrap().body.fill==Some(0xff0000),"live style graph imported without overwriting conflicting destination");
    let cp=&t.cell(0,0).unwrap().paragraphs[0];check!(!pasted.resolve_paragraph_text(cp,cp.content[0].format()).italic,"destination paragraph defaults do not change pasted table text");
    into.undo();check!(into.document()==&dest,"whole table paste is one undo step");into.redo();check!(into.document()==&pasted,"whole table paste redo");
    let (tail,_) = into.replace_story_fragment(tail,0,0,copy.clone()).unwrap();
    check!(into.document().tables.len()==2 && into.document().sources.len()==2 && into.document().table_styles.len()==pasted.table_styles.len(),"repeated paste reuses source and identical remapped style graph");
    let nested=into.document().tables[0].cell(0,0).unwrap().paragraphs[0].id;let before=into.document().clone();let rev=into.revision();
    check!(into.replace_story_fragment(nested,0,0,copy.clone()).is_ok() && into.revision()>rev && into.document().tables.iter().any(|t|into.document().table_parent(t.id).is_some()),"nested paste fits the containing cell");
    into.undo();check!(into.document()==&before,"nested paste undo restores the enclosing table");
    let mut narrow=Document::default();narrow.page.left+=100;let mut narrow=Editor::new(narrow).unwrap();let before=narrow.document().clone();
    check!(narrow.replace_story_fragment(1,0,0,copy.clone()).is_ok() && narrow.document().tables[0].width()==narrow.document().page.content_width(),"wide clipboard table fitted to destination");
    narrow.undo();check!(narrow.document()==&before,"fitting is part of one paste transaction");
    into.replace_story_fragment(tail,0,4,copy).unwrap();
    check!(into.document().paragraphs.last().unwrap().content.iter().all(|i|i.text()==Some("")),"paste replaces selected suffix without extra prefix paragraph");
    let bytes=doc_io::encode_docx(into.document()).unwrap();check!(doc_io::decode_docx(&bytes)==Ok(into.document().clone()),"pasted graph and remapped citations survive saved DOCX");
    let mut overridden=Document::default();let mut blue=overridden.table_style("jfTableBlue").unwrap();blue.first_row.fill=Some(0xff0000);overridden.table_styles.push(blue);
    let mut overridden=Editor::new(overridden).unwrap();overridden.replace_story_fragment(1,0,0,snapshot.copy_table(id).unwrap()).unwrap();
    check!(overridden.document().table_region(&overridden.document().tables[0],0,0).fill==Some(0x196180) && overridden.document().table_style("jfTableBlue").unwrap().first_row.fill==Some(0xff0000),"implicit built-in base remains visually stable when destination overrides it");
    (passed,failed)
}
