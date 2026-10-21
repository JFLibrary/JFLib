// Inheritance and conditional areas exercised through document transactions and real codecs.
use super::*;
fn set(e:&mut Editor,s:TableStyle,table:Option<Id>){e.apply(Edit::SetTableStyle{style:s,table}).unwrap();}
fn change(e:&mut Editor,id:Id,c:TableChange){e.apply(Edit::ChangeTable{table:id,change:c}).unwrap();}
fn fill(e:&Editor,id:Id,r:usize,c:usize)->Option<u32>{let d=e.document();d.table_region(d.table(id).unwrap(),r,c).fill}
pub(super) fn run()->(usize,usize){
    let(mut passed,mut failed)=(0,0);
    macro_rules! check{($v:expr,$label:expr)=>{if $v{passed+=1;}else{failed+=1;plat::write_stdout(format!("FAIL table graph: {}\n",$label).as_bytes());}};}
    let mut e=Editor::new(Document::default()).unwrap();
    let id=e.apply(Edit::InsertTable{paragraph:1,offset:0,rows:8,columns:6}).unwrap();
    let mut base=TableStyle::derived("ResearchBase","Forschung", "jfTableBlue");
    base.band1.fill=Some(0xeeeeee);base.band2.fill=Some(0xdddddd);
    base.band1_vertical.fill=Some(0xaaaaaa);base.band2_vertical.fill=Some(0xbbbbbb);
    base.row_band_size=Some(2);base.column_band_size=Some(2);
    base.top_left.fill=Some(0x110000);base.top_right.fill=Some(0x220000);
    base.bottom_left.fill=Some(0x330000);base.bottom_right.fill=Some(0x440000);
    set(&mut e,base.clone(),None);
    let mut child=TableStyle::derived("ResearchChild","Messwerte", "ResearchBase");
    child.first_row.text.bold=Some(false);set(&mut e,child.clone(),Some(id));
    check!(fill(&e,id,0,1)==Some(0x196180),"three-level header inheritance");
    let head=e.document().table(id).unwrap().cell(0,0).unwrap().paragraphs[0].id;
    e.apply(Edit::SetText{paragraph:head,inline:0,text:String::from("Kopf")}).unwrap();
    let format=|e:&Editor|{let d=e.document();d.resolve_paragraph_text(d.paragraph(head).unwrap(),TextFormat::default())};
    check!(!format(&e).bold && format(&e).color==0xffffffff,"explicit false and inherited text color coexist");
    let clip=e.document().copy_story_range(head,0,4).unwrap();
    let clip=doc_io::decode_docx(&doc_io::encode_docx(&clip.document).unwrap()).unwrap();
    let cp=&clip.paragraphs[0];let copied=clip.resolve_paragraph_text(cp,cp.content[0].format());
    check!(!copied.bold && copied.color==0xffffffff && clip.plain_text()=="Kopf","copied cell text freezes inherited character values through native clipboard DOCX");
    check!(fill(&e,id,1,1)==fill(&e,id,2,1) && fill(&e,id,2,1)!=fill(&e,id,3,1),"two-row bands after first row");
    let look=TableLook{first_row:true,last_row:true,first_column:true,last_column:true,banded_rows:true,banded_columns:true};
    change(&mut e,id,TableChange::Look(look));
    check!(fill(&e,id,1,1)==Some(0xaaaaaa) && fill(&e,id,2,2)==Some(0xaaaaaa) && fill(&e,id,3,3)==Some(0xbbbbbb),"column bands override row bands and honor width");
    for (r,c,color) in [(0,0,0x110000),(0,5,0x220000),(7,0,0x330000),(7,5,0x440000)]{
        check!(fill(&e,id,r,c)==Some(color),"corner conditional region");
    }
    let mut no_edge=look;no_edge.first_column=false;change(&mut e,id,TableChange::Look(no_edge));
    check!(fill(&e,id,0,0)==Some(0x196180),"corner requires both row and column flags");
    change(&mut e,id,TableChange::Look(look));
    child.top_left.fill=Some(0xffffff);set(&mut e,child.clone(),None);
    base.top_left.fill=Some(0x550000);base.first_row.fill=Some(0x778899);
    base.border_inherit=false;base.border_width=12;base.border_color=0x123456;set(&mut e,base.clone(),None);
    check!(fill(&e,id,0,0)==Some(0xffffff) && fill(&e,id,0,1)==Some(0x778899),"child white survives parent changes");
    check!(e.document().table_border(e.document().table(id).unwrap())==(12,0x123456),"inherited borders update live");
    let changed=e.document().clone();e.undo();check!(fill(&e,id,0,1)==Some(0x196180),"parent update undo");e.redo();check!(e.document()==&changed,"parent update redo");
    child.top_left.fill=None;child.first_row.text.bold=None;set(&mut e,child.clone(),None);
    check!(fill(&e,id,0,0)==Some(0x550000) && format(&e).bold,"clear override restores parent property");
    change(&mut e,id,TableChange::Cell{row:0,column:0,fill:Some(0xabcdef),alignment:CellAlignment::Top});
    check!(fill(&e,id,0,0)==Some(0xabcdef),"direct shading overrides inherited corner");
    change(&mut e,id,TableChange::Cell{row:0,column:0,fill:None,alignment:CellAlignment::Top});
    // Full-width merged cells intersect both left and right corner regions.
    change(&mut e,id,TableChange::Merge{row:0,column:0,rows:1,columns:6});
    check!(fill(&e,id,0,0)==Some(0x220000) && fill(&e,id,0,5)==Some(0x220000),"merged header resolves at anchor with corner priority");
    let doc=e.document().clone();let bytes=doc_io::encode_docx(&doc).unwrap();
    check!(doc_io::decode_docx(&bytes)==Ok(doc.clone()),"DOCX retains style graph, inherited borders, vertical bands and corners");
    let reopened=doc_io::decode_docx(&bytes).unwrap();
    check!(format!("{:?}",doc_layout::paginate(&doc,&mut Fixed).unwrap())==format!("{:?}",doc_layout::paginate(&reopened,&mut Fixed).unwrap()),"native layout survives style graph interchange");
    let html=String::from_utf8(doc_io::clipboard_html_encode(&doc).unwrap()).unwrap();
    let rtf=String::from_utf8(doc_io::clipboard_rtf_encode(&doc).unwrap()).unwrap();
    check!(html.contains("#AAAAAA") && html.contains("#440000"),"HTML resolves inherited conditional shading");
    check!(rtf.contains("\\red170\\green170\\blue170") && rtf.contains("\\red68\\green0\\blue0"),"RTF resolves inherited conditional shading");
    let rev=e.revision();let snapshot=e.document().clone();
    for (id,parent) in [("ResearchBase","ResearchChild"),("ResearchChild","ResearchChild"),("ResearchChild","missing")]{
        let mut bad=e.document().table_style(id).unwrap();bad.based_on=Some(String::from(parent));
        check!(e.apply(Edit::SetTableStyle{style:bad,table:None}).is_err() && e.revision()==rev && e.document()==&snapshot,"invalid graph rejected atomically");
    }
    let mut invalid=child.clone();invalid.row_band_size=Some(0);
    check!(e.apply(Edit::SetTableStyle{style:invalid,table:None}).is_err() && e.document()==&snapshot,"zero explicit band size rejected");
    let mut chain=Document::default();
    for i in 0..32 {chain.table_styles.push(if i==0{TableStyle::new("Chain0","Level0")}else{TableStyle::derived(&format!("Chain{i}"),&format!("Level{i}"),&format!("Chain{}",i-1))});}
    check!(chain.validate().is_ok(),"32-level graph supported");
    chain.table_styles.push(TableStyle::derived("TooDeep","Too deep","Chain31"));
    check!(chain.validate().is_err(),"graph depth is bounded");
    check!(snapshot.table_style_depends_on("ResearchChild","ResearchBase") && !snapshot.table_style_depends_on("ResearchBase","ResearchChild"),"editor excludes descendants as parents");
    (passed,failed)
}
