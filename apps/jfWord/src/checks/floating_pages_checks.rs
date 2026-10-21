//! Multi-page floats, editable continuation geometry and static HTML position interchange.
use super::*;
use alloc::vec::Vec;
fn change(e:&mut Editor,t:Id,change:TableChange){e.apply(Edit::ChangeTable{table:t,change}).unwrap();}
fn setup(rows:usize)->(Editor,Id){
    let mut d=Document::default();d.page=PageSettings{width:8640,height:5760,top:720,right:720,bottom:720,left:720};
    let mut e=Editor::new(d).unwrap();let t=e.apply(Edit::InsertTable{paragraph:1,offset:0,rows,columns:2}).unwrap();
    change(&mut e,t,TableChange::Columns(vec![1440,1440]));
    for row in 0..rows {change(&mut e,t,TableChange::RowHeight{row,height:900});for column in 0..2 {let id=e.document().table(t).unwrap().cell(row,column).unwrap().paragraphs[0].id;
        e.apply(Edit::SetText{paragraph:id,inline:0,text:format!("Zelle {row}:{column}")}).unwrap();}}
    change(&mut e,t,TableChange::HeaderRows(1));change(&mut e,t,TableChange::Position(Some(TablePosition::default())));(e,t)
}
fn intersects(a:[i32;4],b:[i32;4])->bool{a[0]<b[0]+b[2]&&b[0]<a[0]+a[2]&&a[1]<b[1]+b[3]&&b[1]<a[1]+a[3]}
pub(super) fn run()->(usize,usize){
    let(mut passed,mut failed)=(0,0);
    macro_rules! check{($v:expr,$name:expr)=>{if $v{passed+=1;}else{failed+=1;plat::write_stdout(format!("FAIL floating pages/HTML: {}\n",$name).as_bytes());}};}
    let (mut e,t)=setup(12);let d=e.document().clone();let layout=doc_layout::paginate(&d,&mut Fixed).unwrap();
    check!(layout.pages.len()==4,"short anchor text still drains all table continuation pages");
    check!(layout.pages.iter().skip(1).all(|p|p.cells.iter().filter(|c|c.table==t&&c.repeated).count()==2),"same header cells repeat on every continuation");
    check!(layout.pages.iter().flat_map(|p|&p.cells).filter(|c|!c.repeated).count()==24,"every original cell appears once when rows fit");
    check!(layout.pages.iter().all(|p|p.cells.iter().all(|c|c.rect[1]>=720&&c.rect[1]+c.rect[3]<=4920)),"fragments honor page margins and text distances");
    let body="Absatz neben mehrseitiger Tabelle. ".repeat(220);e.apply(Edit::SetText{paragraph:1,inline:0,text:body.clone()}).unwrap();
    let layout=doc_layout::paginate(e.document(),&mut Fixed).unwrap();
    check!(layout.pages.iter().take(4).all(|p|p.lines.iter().any(|l|l.paragraph==1&&l.x>=3720)),"body wraps around each continuation, not only the first page");
    check!(layout.pages.iter().all(|p|p.lines.iter().filter(|l|l.paragraph==1).all(|l|p.cells.iter().all(|c|!intersects([l.x,l.top,l.fragments.iter().map(|f|f.x+f.width-l.x).max().unwrap_or(1),l.height],c.rect)))),"body text never overlaps floating cells");
    check!(layout.pages.iter().all(|p|p.lines.iter().filter(|l|l.paragraph==1).all(|l|p.cells.iter().all(|c|!intersects(doc_view::text_line_bounds(e.document(),p,l),c.rect)))),"main-body hit regions leave floating cells clickable on every page");
    let lines:Vec<_>=layout.pages.iter().flat_map(|p|&p.lines).filter(|l|l.paragraph==1).collect();
    check!(lines.first().unwrap().start==0&&lines.last().unwrap().end==body.len()&&lines.windows(2).all(|v|v[0].end==v[1].start),"body source ranges are continuous across all replanned pages");
    check!(layout.pages.iter().skip(4).all(|p|p.lines.iter().filter(|l|l.paragraph==1).all(|l|l.x==720)),"full body width returns after the final table fragment");
    e.apply(Edit::InsertNote{paragraph:1,inline:0,offset:12,kind:NoteKind::Footnote,text:"Lange wissenschaftliche Fußnote. ".repeat(170)}).unwrap();
    let notes=doc_layout::paginate(e.document(),&mut Fixed).unwrap();
    check!(notes.pages[0].separators.len()==1,"first footnote begins on its anchor page");
    check!(notes.pages.iter().all(|p|p.separators.first().is_none_or(|s|p.cells.iter().all(|c|c.rect[1]+c.rect[3]<s.y))),"continued floats and footnotes reserve disjoint areas");
    check!(notes.pages.iter().flat_map(|p|&p.cells).filter(|c|!c.repeated&&c.row_end).count()==24,"footnote reflow preserves all row ends");
    let (mut tall,tall_id)=setup(3);change(&mut tall,tall_id,TableChange::RowHeight{row:1,height:14000});
    let geometry=doc_layout::paginate(tall.document(),&mut Fixed).unwrap();
    let (page,edge)=geometry.pages.iter().enumerate().find_map(|(i,p)|p.cells.iter().find(|c|c.table==tall_id&&c.row==1&&c.column==0&&c.row_end).map(|c|(i,c))).unwrap();
    let mut controller=doc_view::TableController::default();
    let mut input=ui::UiInput{rect:[0,0,tall.document().page.width as i32,tall.document().page.height as i32],x:edge.rect[0]+100,y:edge.rect[1]+edge.rect[3],pressed:1<<ui::BTN_LEFT,mods:ui::MOD_ALT,..Default::default()};
    controller.update(tall.document(),&geometry.pages[page],page,&input,tall.revision());
    input.pressed=0;input.released=1<<ui::BTN_LEFT;input.y+=180;
    check!(matches!(controller.update(tall.document(),&geometry.pages[page],page,&input,tall.revision()),Some((id,TableChange::RowHeight{row:1,height})) if id==tall_id&&height==edge.last_row_height+180),"oversized row remains resizable at its actual continuation edge");
    let p=tall.document().table(tall_id).unwrap().cell(1,0).unwrap().paragraphs[0].id;
    let text="Fortgesetzter Zelltext ".repeat(150);tall.apply(Edit::SetText{paragraph:p,inline:0,text:text.clone()}).unwrap();
    let layout=doc_layout::paginate(tall.document(),&mut Fixed).unwrap();
    let cell=tall.document().table(tall_id).unwrap().cell(1,0).unwrap().id;
    let fragments:Vec<_>=layout.pages.iter().flat_map(|p|&p.cells).filter(|c|c.cell==cell).collect();
    check!(fragments.len()>3&&fragments.iter().filter(|c|c.row_end).count()==1,"oversized row has one real bottom edge across fragments");
    check!(fragments.iter().skip(1).all(|c|c.edges.iter().all(|e|e.side!=BorderSide::Top)),"continuations do not invent horizontal borders inside a row");
    let lines:Vec<_>=layout.pages.iter().flat_map(|p|&p.lines).filter(|l|l.paragraph==p).collect();
    check!(lines.first().unwrap().start==0&&lines.last().unwrap().end==text.len()&&lines.windows(2).all(|v|v[0].end==v[1].start),"cell text appears exactly once with editable source ranges");
    check!(layout.pages.iter().all(|p|p.lines.iter().filter(|l|l.paragraph==lines[0].paragraph).all(|l|p.cells.iter().any(|c|c.cell==cell&&l.top>=c.rect[1]&&l.top+l.height<=c.rect[1]+c.rect[3]))),"no text line is cut by a table page boundary");
    // Full-width floats consume complete pages before their anchor text can follow.
    let (mut wide,wt)=setup(10);change(&mut wide,wt,TableChange::Columns(vec![3600,3600]));
    let layout=doc_layout::paginate(wide.document(),&mut Fixed).unwrap();
    check!(layout.pages.len()>=3&&layout.pages.iter().flat_map(|p|&p.cells).filter(|c|!c.repeated).count()==20,"full-width multi-page float progresses without a repeated anchor");
    let second=wide.apply(Edit::InsertTable{paragraph:1,offset:0,rows:2,columns:1}).unwrap();change(&mut wide,second,TableChange::Columns(vec![7200]));change(&mut wide,second,TableChange::Position(Some(TablePosition::default())));
    let layout=doc_layout::paginate(wide.document(),&mut Fixed).unwrap();
    check!(layout.pages.iter().flat_map(|p|&p.cells).filter(|c|c.table==second&&c.row_end).count()==2,"queued float survives a full preceding continuation page");
    check!(layout.pages.iter().all(|p|p.cells.iter().filter(|c|c.table==wt).all(|a|p.cells.iter().filter(|c|c.table==second).all(|b|!intersects(a.rect,b.rect)))),"overlap disabled is honored across continuation queues");
    let (mut nested,nt)=setup(2);let cp=nested.document().table(nt).unwrap().cell(1,0).unwrap().paragraphs[0].id;
    let child=nested.apply(Edit::InsertTable{paragraph:cp,offset:0,rows:14,columns:1}).unwrap();change(&mut nested,child,TableChange::HeaderRows(1));
    for row in 0..14{change(&mut nested,child,TableChange::RowHeight{row,height:700});}
    let layout=doc_layout::paginate(nested.document(),&mut Fixed).unwrap();
    check!(layout.pages.len()>2&&layout.pages.iter().skip(1).all(|p|p.cells.iter().any(|c|c.table==child&&c.repeated)),"nested table headers repeat inside floating continuation cells");
    check!(layout.pages.iter().all(|p|p.cells.iter().filter(|c|c.table==child).all(|c|p.cells.iter().any(|outer|outer.table==nt&&outer.row==1&&c.rect[1]>=outer.rect[1]&&c.rect[1]+c.rect[3]<=outer.rect[1]+outer.rect[3]))),"nested headers remain inside outer cell walls");
    let (mut bad,bt)=setup(2);let before=bad.document().clone();let revision=bad.revision();
    let r=bad.apply_checked(Edit::ChangeTable{table:bt,change:TableChange::RowHeight{row:0,height:4300}},|d|doc_layout::paginate(d,&mut Fixed).map(|_|()).map_err(|_|Error::Limit));
    check!(r.is_err()&&bad.revision()==revision&&bad.document()==&before,"unusable repeated header rejects atomically");
    // Exact HTML properties across every anchor pair, negative offsets and custom page geometry.
    for horizontal in [TableHorizontal::Page,TableHorizontal::Margin,TableHorizontal::Column] {for vertical in [TableVertical::Page,TableVertical::Margin,TableVertical::Paragraph] {for overlap in [false,true]{
        let mut d=d.clone();let pos=TablePosition{horizontal,vertical,x:if horizontal==TableHorizontal::Page{121}else{-121},y:333,distance:[21,42,63,84],overlap};d.tables[0].position=Some(pos);
        let html=doc_io::clipboard_html_encode(&d).unwrap();let decoded=doc_io::clipboard_html_decode(&html).unwrap();
        check!(decoded.page==d.page&&decoded.tables[0].position==Some(pos)&&decoded.tables[0].columns==d.tables[0].columns,"HTML exact positions and page setup for all anchor pairs");
        check!(doc_layout::paginate(&decoded,&mut Fixed).is_ok(),"HTML positions remain renderable across pages");
        check!(doc_io::decode_docx(&doc_io::encode_docx(&decoded).unwrap())==Ok(decoded),"HTML import can be saved and reopened as native DOCX");
    }}}
    for (css,x,y,h,v,dist) in [
        ("float:left;margin:3pt 6pt 9pt 12pt",240,60,TableHorizontal::Margin,TableVertical::Paragraph,[60,120,180,240]),
        ("float:right;margin-right:12pt",6432,0,TableHorizontal::Margin,TableVertical::Paragraph,[0,240,0,0]),
        ("position:absolute;left:24pt;top:36pt",480,720,TableHorizontal::Page,TableVertical::Page,[0;4]),
        ("position:absolute;right:24pt;top:36pt",7609,720,TableHorizontal::Page,TableVertical::Page,[0;4]),
    ] {
        let html=format!("<table style=\"width:120pt;{css}\"><tr><td>Import</td></tr></table><p>Anker</p>");
        let d=doc_io::clipboard_html_decode(html.as_bytes()).unwrap();let p=d.tables[0].position.unwrap();
        // Right offsets depend on the exact A4 twip width, not a rounded CSS pixel width.
        let expected=if css.contains("right:"){if h==TableHorizontal::Page{d.page.width as i32-2400-480}else{d.page.content_width() as i32-2400-240}}else{x};
        check!(p.x==expected&&p.y==y&&p.horizontal==h&&p.vertical==v&&p.distance==dist,"external CSS float and absolute positions resolve after table width");
        check!(d.tables[0].cell(0,0).unwrap().paragraphs[0].format.left==0,"table margins are not inherited as cell paragraph indents");
    }
    for attrs in ["data-jf-table-position=\"2 margin paragraph 0 0 0 0 0 0 0\"","data-jf-table-position=\"1 margin paragraph -99999 0 0 0 0 0 0\"","data-jf-table-position=\"1 margin paragraph 0 0 0 0 0 0 2\"","style=\"position:fixed\"","style=\"float:right;margin:calc(1px)\"","style=\"position:absolute;left:2pt;right:2pt\""]{
        let h=format!("<table {attrs}><tr><td>x</td></tr></table>");check!(doc_io::clipboard_html_decode(h.as_bytes()).is_err(),"ambiguous or invalid HTML positions fail explicitly");
    }
    for align in ["right","RIGHT"] {
        let html=format!("<table align=\"{align}\" style=\"width:120pt\"><tr><td>Text</td></tr></table>");
        let d=doc_io::clipboard_html_decode(html.as_bytes()).unwrap();
        check!(d.tables[0].position.unwrap().x==d.page.content_width() as i32-2400&&d.tables[0].cell(0,0).unwrap().paragraphs[0].format.alignment==Alignment::Left,"legacy table alignment positions the box without changing cell alignment");
    }
    let d=doc_io::clipboard_html_decode(b"<table style=\"position:absolute;left:0;top:36pt;width:50%\"><tr><td>Text</td></tr></table>").unwrap();
    check!(d.tables[0].width()==d.page.width/2,"page-positioned percentage width uses the physical page reference");
    (passed,failed)
}
