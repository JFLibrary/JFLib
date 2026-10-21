//! Independent expected decimal results, table ownership and field/codec contracts.
use alloc::{format,string::String,vec};
use doc_core::*;
fn cell(e:&Editor,t:Id,r:usize,c:usize)->Id{e.document().table(t).unwrap().cell(r,c).unwrap().paragraphs[0].id}
fn text(e:&mut Editor,p:Id,s:&str){e.apply(Edit::SetText{paragraph:p,inline:0,text:s.into()}).unwrap();}
fn add(e:&mut Editor,p:Id,s:&str,picture:&str){e.apply(Edit::InsertFormula{paragraph:p,inline:0,offset:0,formula:Formula::new(s,picture).unwrap()}).unwrap();}
fn field(e:&Editor,p:Id)->String{e.document().formula_result(p,1).unwrap().text}
pub(crate) fn run()->(usize,usize){
    let(mut passed,mut failed)=(0,0);
    macro_rules! check{($ok:expr,$name:expr)=>{if $ok{passed+=1;}else{failed+=1;plat::write_stdout(format!("FAIL formula: {}\n",$name).as_bytes());}};}
    let d=Document::default();
    for (expression,picture,want) in [
        ("=0.1+0.2","","0.3"),("=2+3*4","","14"),("=(2+3)*4","","20"),
        ("=2^3^2","","512"),("=-2^2","","-4"),("=2^-2","","0.25"),
        ("=ABS(-22)","","22"),("=INT(-1.2)","","-2"),("=SIGN(-11)","","-1"),
        ("=ROUND(123.456,2)","","123.46"),("=ROUND(-2.5,0)","","-3"),("=ROUND(123.456,-2)","","100"),
        ("=MOD(4,2)","","0"),("=MOD(4,-3)","","-2"),("=MOD(-4,3)","","2"),
        ("=IF(1<2,7,1/0)","","7"),("=IF(1>2,1/0,8)","","8"),
        ("=AND(1=1,3>=2)","","1"),("=OR(1=5,2<>3)","","1"),("=NOT(TRUE())","","0"),
        ("=PRODUCT(2,3,4)","","24"),("=AVERAGE(1;2;3)","","2"),("=COUNT(1,2,3)","","3"),
        ("=MIN(3,-4,6)","","-4"),("=MAX(3,-4,6)","","6"),("=FALSE","","0"),
        ("=1/3","0.00","0.33"),("=1/4","0.00%","25.00%"),("=12345.678","#,##0.00","12,345.68"),
        ("=-0.000000001","","-0.000000001"),("=1/0","","#DIV/0!"),("=ROUND(1,10)","","#NUM!"),
        ("=AVERAGE()","","#DIV/0!"),("=SUM(A1)","","#REF!"),("=1.5^2.5","","#NUM!"),
        ("=2^32*2^32*2^32","","#NUM!"),("=ABS(1,2)","","#VALUE!"),
    ]{let f=Formula::new(expression,picture).unwrap();check!(d.evaluate_formula(1,&f).text==want,expression);}
    for expression in ["=1+","=SUM(","=1..2","=SUM(1,,2)","=UNKNOWN(1)","=bookmark","=SUM(A1:B)","=1e3","=∞"]{check!(Formula::new(expression,"").is_err(),"unsupported/malformed syntax rejected");}
    for picture in ["0.##","€0.00","0.0000000000","0;0","garbage"]{check!(Formula::new("=1",picture).is_err(),"unsupported number picture rejected");}
    check!(Formula::new(&format!("={}1{}","(".repeat(40),")".repeat(40)),"")==Err(FormulaError::Limit),"parser nesting bound");
    let long=format!("=1{}","+1".repeat(150));check!(d.evaluate_formula(1,&Formula::new(&long,"").unwrap()).error==Some(FormulaError::Limit),"evaluation depth bound");
    let mut e=Editor::new(Document::default()).unwrap();let t=e.apply(Edit::InsertTable{paragraph:1,offset:0,rows:4,columns:3}).unwrap();
    let a1=cell(&e,t,0,0);let a2=cell(&e,t,1,0);let a3=cell(&e,t,2,0);let a4=cell(&e,t,3,0);let b4=cell(&e,t,3,1);let c4=cell(&e,t,3,2);
    text(&mut e,a1,"999");text(&mut e,a2,"12,5");text(&mut e,a3,"7.5");
    e.apply(Edit::ChangeTable{table:t,change:TableChange::HeaderRows(1)}).unwrap();
    add(&mut e,a4,"=SUM(ABOVE)","0.00");add(&mut e,b4,"=A4*2","0.00");add(&mut e,c4,"=SUM(LEFT)","");
    check!(field(&e,a4)=="20.00"&&field(&e,b4)=="40.00"&&field(&e,c4)=="60","headers excluded and dependencies use raw numeric values");
    let before=e.document().clone();text(&mut e,a2,"22.5");
    check!(field(&e,a4)=="30.00"&&field(&e,c4)=="90","dependent results update after a source edit");
    check!(e.undo()&&e.document()==&before&&field(&e,c4)=="60"&&e.redo(),"derived results follow undo and redo");e.undo();
    for (s,want) in [("=SUM(A1:A3)","1019"),("=SUM(R2C1:R3C1)","20"),("=SUM(A1:C4)","#CYCLE!"),("=A4","#CYCLE!"),("=Z999","#REF!"),("=SUM(A3:A2)","#REF!")]{check!(e.document().evaluate_formula(a4,&Formula::new(s,"").unwrap()).text==want,s);}
    e.apply(Edit::SetFormula{paragraph:a4,inline:1,formula:Formula::new("=B4","").unwrap()}).unwrap();
    check!(field(&e,a4)=="#CYCLE!"&&field(&e,b4)=="#CYCLE!","indirect dependency cycle is visible");e.undo();
    let before=e.document().clone();let revision=e.revision();
    check!(e.apply(Edit::SetFormula{paragraph:a2,inline:0,formula:Formula::new("=1","").unwrap()}).is_err()&&e.revision()==revision&&e.document()==&before,"updating plain text rejects atomically");
    let projection=e.document().text_projection(a4).unwrap();check!(!projection.spans[1].editable,"generated calculation result is protected");
    check!(e.apply(Edit::ReplaceParagraphText{paragraph:a4,start:0,end:1,text:"x".into()}).is_err()&&e.document()==&before,"typing through a formula is rejected");
    let copy=e.document().copy_table(t).unwrap();let bytes=doc_io::encode_document_fragment(&copy).unwrap();let copy=doc_io::decode_document_fragment(&bytes).unwrap();
    e.replace_story_fragment(1,0,0,copy).unwrap();let pasted=e.document().tables.iter().find(|v|v.id!=t).unwrap();let pasted_a4=pasted.cell(3,0).unwrap().paragraphs[0].id;
    check!(e.document().formula_result(pasted_a4,1).unwrap().text=="20.00","whole-table clipboard keeps live local references");
    e.undo();let fragment=e.document().copy_story_range(a4,0,5).unwrap();let b2=cell(&e,t,1,1);
    e.replace_story_fragment(b2,0,0,fragment).unwrap();check!(e.document().paragraph(b2).unwrap().content.iter().any(|n|matches!(n,Inline::Formula(_))),"selected formula survives native rich paste");e.undo();
    let docx=doc_io::encode_docx(e.document()).unwrap();check!(doc_io::decode_docx(&docx)==Ok(e.document().clone()),"DOCX formula instruction and picture roundtrip");
    let note=e.apply(Edit::InsertNote{paragraph:a2,inline:0,offset:4,kind:NoteKind::Footnote,text:"Measured value".into()}).unwrap();
    check!(field(&e,a4)=="20.00"&&e.document().note(note).is_some(),"scientific note markers do not corrupt numeric cells");e.undo();
    let html=String::from_utf8(doc_io::clipboard_html_encode(e.document()).unwrap()).unwrap();let rtf=String::from_utf8(doc_io::clipboard_rtf_encode(e.document()).unwrap()).unwrap();
    check!(html.contains("20.00")&&rtf.contains("20.00"),"HTML/RTF materialize current calculation results");
    let origin=e.document().clone();e.apply(Edit::ChangeTable{table:t,change:TableChange::InsertRow(2)}).unwrap();check!(field(&e,a4)=="20.00","directional formula uses changed current grid");
    e.undo();check!(e.document()==&origin,"structural undo restores formula coordinates");
    e.apply(Edit::ChangeTable{table:t,change:TableChange::RemoveRow(2)}).unwrap();check!(field(&e,a4)=="12.50","deleted source row recalculates sum");e.undo();
    let inner=e.apply(Edit::InsertTable{paragraph:a2,offset:0,rows:2,columns:1}).unwrap();let ip=cell(&e,inner,0,0);let iq=cell(&e,inner,1,0);text(&mut e,ip,"3");add(&mut e,iq,"=SUM(ABOVE)","");
    check!(field(&e,iq)=="3"&&field(&e,a4)=="20.00","nested table formula stays in inner grid; outer ignores child content");
    check!(doc_io::decode_docx(&doc_io::encode_docx(e.document()).unwrap())==Ok(e.document().clone()),"nested field roundtrip");
    let form=e.document().formula_result(a4,1).unwrap();check!(form.error.is_none(),"structured successful result");
    e.apply(Edit::RemoveField{paragraph:a4,inline:1}).unwrap();check!(e.document().formula_result(a4,1).is_none()&&e.undo(),"generic field removal/undo retains calculations");
    let mut merged=Editor::new(Document::default()).unwrap();let mt=merged.apply(Edit::InsertTable{paragraph:1,offset:0,rows:2,columns:3}).unwrap();let mp=cell(&merged,mt,0,0);text(&mut merged,mp,"5");
    merged.apply(Edit::ChangeTable{table:mt,change:TableChange::Merge{row:0,column:0,rows:1,columns:2}}).unwrap();let sum=cell(&merged,mt,1,2);add(&mut merged,sum,"=SUM(A1:B1)","");check!(field(&merged,sum)=="5","merged range counts the physical anchor once");
    let many=vec![Inline::Formula(Formula::new("=1","").unwrap());129];let mut bad=Document::default();bad.paragraphs[0].content=many;check!(bad.validate()==Err(Error::Limit),"formula count budget");
    (passed,failed)
}
