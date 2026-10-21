//! Fixed-height geometry, overflow retention, merged/nested rows and interchange.
use super::*;
use alloc::{rc::Rc,vec::Vec};
use ui::Style;
fn change(e:&mut Editor,t:Id,change:TableChange){e.apply(Edit::ChangeTable{table:t,change}).unwrap();}
fn sizing(e:&mut Editor,t:Id,row:usize,height:u32,rule:RowHeightRule){change(e,t,TableChange::RowSizing{row,height,rule});}
fn layout(e:&Editor)->doc_layout::Layout{doc_layout::paginate(e.document(),&mut Fixed).unwrap()}
fn height(l:&doc_layout::Layout,cell:Id)->i32{l.pages.iter().flat_map(|p|&p.cells).filter(|c|c.cell==cell&&!c.repeated).map(|c|c.rect[3]).sum()}
pub(super) fn run()->(usize,usize){
    let(mut passed,mut failed)=(0,0);
    macro_rules! check{($v:expr,$name:expr)=>{if $v{passed+=1;}else{failed+=1;plat::write_stdout(format!("FAIL row height: {}\n",$name).as_bytes());}};}
    let input=include_bytes!("../../fixtures/exact-row-heights.html");
    for floating in [false,true]{
        let mut d=doc_io::clipboard_html_decode(input).unwrap();if floating{d.tables[0].position=Some(TablePosition{distance:[0;4],..Default::default()});}
        let t=d.tables[0].id;let cell=d.tables[0].cell(0,0).unwrap().id;let p=d.tables[0].cell(0,0).unwrap().paragraphs[0].id;
        let original=d.clone();let mut e=Editor::new(d).unwrap();let l=layout(&e);
        check!(height(&l,cell)==480,"exact row ignores tall contents");
        check!(height(&l,e.document().table(t).unwrap().cell(1,0).unwrap().id)>480,"minimum row grows to fit text");
        let lines:Vec<_>=l.pages.iter().flat_map(|s|&s.lines).filter(|l|l.paragraph==p).collect();
        check!(lines.last().unwrap().end<e.document().paragraph_text(e.document().paragraph(p).unwrap()).len(),"overflow is absent from page line geometry");
        check!(l.pages.iter().all(|s|s.lines.iter().filter(|l|l.cell==Some(cell)).all(|l|s.cells.iter().any(|c|c.cell==cell&&l.top>=c.rect[1]&&l.top+l.height<=c.rect[1]+c.rect[3]))),"partial line hit region stays inside fixed cell");
        for align in [CellAlignment::Center,CellAlignment::Bottom]{change(&mut e,t,TableChange::Cell{row:0,column:0,fill:None,alignment:align});let a=layout(&e);
            let first=a.pages.iter().find_map(|s|s.lines.iter().find(|l|l.cell==Some(cell)).map(|l|(l,s.cells.iter().find(|c|c.cell==cell).unwrap()))).unwrap();
            check!(first.0.start==0&&first.0.top==first.1.rect[1]+90,"oversized aligned cell keeps its beginning visible");
            check!(height(&a,cell)==480,"alignment cannot grow exact row");}
        let before=e.document().clone();sizing(&mut e,t,0,480,RowHeightRule::AtLeast);let full=layout(&e);
        check!(height(&full,cell)>480,"switching to minimum reveals overflow");
        check!(e.document().plain_text()==original.plain_text(),"height and alignment never mutate text");
        check!(e.undo()&&e.document()==&before,"height rule undo");check!(e.redo()&&height(&layout(&e),cell)>480,"height rule redo");
        sizing(&mut e,t,0,350,RowHeightRule::Exact);
        let fixed=e.document().clone();let l=layout(&e);
        check!(height(&l,cell)==350,"fractional point fixed height");
        let (page,edge)=l.pages.iter().enumerate().find_map(|(i,p)|p.cells.iter().find(|c|c.cell==cell&&c.row_end).map(|c|(i,c))).unwrap();
        let mut controller=doc_view::TableController::default();
        let mut input=ui::UiInput{rect:[0,0,e.document().page.width as i32,e.document().page.height as i32],x:edge.rect[0]+150,y:edge.rect[1]+edge.rect[3],pressed:1<<ui::BTN_LEFT,mods:ui::MOD_ALT,..Default::default()};
        controller.update(e.document(),&l.pages[page],page,&input,e.revision());input.pressed=0;input.released=1<<ui::BTN_LEFT;input.y+=180;
        let dragged=controller.update(e.document(),&l.pages[page],page,&input,e.revision()).unwrap();change(&mut e,dragged.0,dragged.1);
        check!(height(&layout(&e),cell)==530&&e.document().table(t).unwrap().rows[0].height_rule==RowHeightRule::Exact,"direct row-edge dragging preserves exact rule");
        e.undo();check!(e.document()==&fixed,"fixed row dragging undo");
        check!(doc_io::decode_docx(&doc_io::encode_docx(&fixed).unwrap())==Ok(fixed.clone()),"DOCX exact model roundtrip");
        for html in [false,true]{let bytes=if html{doc_io::clipboard_html_encode(&fixed)}else{doc_io::clipboard_rtf_encode(&fixed)}.unwrap();
            let b=if html{doc_io::clipboard_html_decode(&bytes)}else{doc_io::clipboard_rtf_decode(&bytes)}.unwrap();
            check!(b.tables[0].rows[0].height_rule==RowHeightRule::Exact&&b.tables[0].rows[0].min_height==350&&b.tables[0].rows[1].height_rule==RowHeightRule::AtLeast,"HTML/RTF height and rule do not leak to next row");
            check!(b.plain_text()==fixed.plain_text(),"clipped overflow retained in public clipboard formats");}
        let mut paste=Editor::new(Document::default()).unwrap();paste.replace_story_fragment(1,0,0,fixed.copy_table(t).unwrap()).unwrap();
        check!(paste.document().tables[0].rows[0].height_rule==RowHeightRule::Exact,"private whole-table clipboard preserves fixed height");
        let rev=e.revision();for (r,h) in [(0,0),(0,31681),(99,350)]{check!(e.apply(Edit::ChangeTable{table:t,change:TableChange::RowSizing{row:r,height:h,rule:RowHeightRule::Exact}}).is_err()&&e.revision()==rev&&e.document()==&fixed,"invalid fixed height changes are atomic");}
        change(&mut e,t,TableChange::RowHeight{row:0,height:710});check!(e.document().table(t).unwrap().rows[0].height_rule==RowHeightRule::Exact,"legacy row resize preserves exact rule");
        change(&mut e,t,TableChange::InsertColumn(1));change(&mut e,t,TableChange::RemoveColumn(1));check!(e.document().table(t).unwrap().rows[0].height_rule==RowHeightRule::Exact,"column reshaping preserves row sizing");
        sizing(&mut e,t,0,15000,RowHeightRule::Exact);let pages=layout(&e);
        check!(pages.pages.len()>=3&&height(&pages,cell)==15000,"oversized fixed row retains total height across pages");
        check!(pages.pages.iter().flat_map(|p|&p.cells).filter(|c|c.cell==cell&&c.row_end).count()==1,"continued fixed row has one true resize edge");
    }
    // Only flexible rows may absorb the content of a vertically merged cell.
    for last in [RowHeightRule::Exact,RowHeightRule::AtLeast]{
        let mut e=Editor::new(doc_io::clipboard_html_decode(input).unwrap()).unwrap();let t=e.document().tables[0].id;
        change(&mut e,t,TableChange::Merge{row:0,column:0,rows:2,columns:1});sizing(&mut e,t,0,300,RowHeightRule::Exact);sizing(&mut e,t,1,400,last);
        let l=layout(&e);let a=e.document().table(t).unwrap().cell(0,0).unwrap().id;let b=e.document().table(t).unwrap().cell(0,1).unwrap().id;
        check!(height(&l,b)==300,"merged content never expands an exact row");check!(if last==RowHeightRule::Exact{height(&l,a)==700}else{height(&l,a)>700},"merged extent grows only when a flexible row is available");
        let d=e.document();check!(doc_io::decode_docx(&doc_io::encode_docx(d).unwrap())==Ok(d.clone()),"merged mixed sizing DOCX roundtrip");
    }
    for floating in [false,true]{
        let rows=(0..30).map(|n|format!("<tr style='height:45pt'><td>Zeile {n}</td></tr>")).collect::<String>();
        let h=format!("<div data-jf-page='1 8640 5760 720 720 720 720'><table><thead><tr style='height:18pt;mso-height-rule:exactly'><td>Kopf mit langem Text, der erhalten bleibt.</td></tr></thead><tbody>{rows}</tbody></table></div>");
        let mut d=doc_io::clipboard_html_decode(h.as_bytes()).unwrap();if floating{d.tables[0].position=Some(TablePosition{distance:[0;4],..Default::default()});}
        let cell=d.tables[0].cell(0,0).unwrap().id;let l=doc_layout::paginate(&d,&mut Fixed).unwrap();
        check!(l.pages.len()>2&&l.pages.iter().skip(1).filter(|p|!p.cells.is_empty()).all(|p|p.cells.iter().any(|c|c.cell==cell&&c.repeated&&c.rect[3]==360)),"repeated headers retain exact height on every continuation");
        let mut e=Editor::new(d).unwrap();let t=e.document().tables[0].id;sizing(&mut e,t,0,80,RowHeightRule::Exact);let tiny=layout(&e);
        let c=tiny.pages[0].cells.iter().find(|c|c.cell==cell).unwrap();
        check!(c.rect[3]==80&&!tiny.pages[0].lines.iter().any(|l|l.cell==Some(cell)),"row below padding height has no invisible text hit region");
        check!(doc_view::table_cell_at(&tiny.pages[0],c.rect[0]+100,c.rect[1]+40).is_some_and(|c|c.cell==cell),"fully clipped cell remains geometrically selectable");
        let mut controller=doc_view::TableController::default();let input=ui::UiInput{rect:[0,0,e.document().page.width as i32/15,e.document().page.height as i32/15],x:(c.rect[0]+300)/15,y:(c.rect[1]+45)/15,pressed:1<<ui::BTN_LEFT,..Default::default()};
        controller.update(e.document(),&tiny.pages[0],0,&input,e.revision());check!(!controller.active(),"row resize tolerance leaves short cell interior selectable");
    }
    // Parent clipping applies to nested cells, rules, edges and hit testing too.
    for floating in [false,true]{
        let h=b"<table><tr style='height:25pt;mso-height-rule:exactly'><td><table><thead><tr><td>Kopf</td></tr></thead><tbody><tr style='height:90pt;break-inside:avoid'><td>Innerer Text</td></tr></tbody></table><p>Verborgen</p></td></tr><tr><td>Danach</td></tr></table>";
        let mut d=doc_io::clipboard_html_decode(h).unwrap();if floating{d.tables[0].position=Some(TablePosition{distance:[0;4],..Default::default()});}
        let cell=d.tables[0].cell(0,0).unwrap().id;let l=doc_layout::paginate(&d,&mut Fixed).unwrap();let outer=l.pages[0].cells.iter().find(|c|c.cell==cell).unwrap();
        check!(outer.rect[3]==500,"nested table cannot enlarge exact parent");
        check!(l.pages[0].cells.iter().filter(|c|c.table==d.tables[1].id).all(|c|c.rect[1]+c.rect[3]<=outer.rect[1]+outer.rect[3]),"nested cell boxes clipped to parent");
        check!(l.pages[0].cells.iter().filter(|c|c.table==d.tables[1].id&&c.rect[1]+c.rect[3]==outer.rect[1]+outer.rect[3]).all(|c|!c.row_end&&!c.edges.iter().any(|e|e.side==BorderSide::Bottom)),"clipped nested edge is not a resize handle or bottom border");
        for html in [false,true]{let b=if html{doc_io::clipboard_html_encode(&d)}else{doc_io::clipboard_rtf_encode(&d)}.unwrap();let back=if html{doc_io::clipboard_html_decode(&b)}else{doc_io::clipboard_rtf_decode(&b)}.unwrap();check!(back.tables.len()==2&&back.plain_text()==d.plain_text(),"nested hidden content survives public interchange");}
    }
    for markup in ["height:12.35pt;mso-height-rule:exactly","height:12.35pt;mso-height-rule:EXACTLY"]{let h=format!("<table><tr style='{markup}'><td>x</td></tr></table>");let d=doc_io::clipboard_html_decode(h.as_bytes()).unwrap();check!(d.tables[0].rows[0].min_height==247&&d.tables[0].rows[0].height_rule==RowHeightRule::Exact,"MS HTML exact fractional height");}
    for attr in ["data-jf-row-height='1 exact 247' style='height:20pt'","data-jf-row-height='1 exact 247'"]{let h=format!("<table><tr {attr}><td>x</td></tr></table>");let d=doc_io::clipboard_html_decode(h.as_bytes()).unwrap();check!(d.tables[0].rows[0].min_height==247,"versioned exact metadata has precedence over rounded CSS");}
    for attr in ["data-jf-row-height='2 exact 20'","data-jf-row-height='1 exact 0'","data-jf-row-height='1 exact 31681'","data-jf-row-height='1 exact 20 extra'","data-jf-row-height='1 other 20'","style='mso-height-rule:exactly'","style='height:0;mso-height-rule:exactly'","style='height:12pt;mso-height-rule:unknown'"] {let h=format!("<table><tr {attr}><td>x</td></tr></table>");check!(doc_io::clipboard_html_decode(h.as_bytes()).is_err(),"invalid exact row metadata rejected");}
    let r=doc_io::clipboard_rtf_decode(br"{\rtf1\trowd\trrh-247\cellx2000\intbl A\cell\row\trowd\cellx2000\intbl B\cell\row\trowd\trrh500\cellx2000\intbl C\cell\row}").unwrap();
    check!(r.tables[0].rows.iter().map(|r|(r.min_height,r.height_rule)).collect::<Vec<_>>()==vec![(247,RowHeightRule::Exact),(0,RowHeightRule::AtLeast),(500,RowHeightRule::AtLeast)],"RTF negative exact, automatic reset and positive minimum");
    for v in ["-2147483648","-31681","31681"]{let r=format!("{{\\rtf1\\trowd\\trrh{v}\\cellx2000\\intbl A\\cell\\row}}");check!(doc_io::clipboard_rtf_decode(r.as_bytes()).is_err(),"RTF out-of-range signed height rejected without overflow");}
    // Real fonts and shared software renderer: hidden red glyphs must not leak into the next row.
    let h=b"<table><tr style='height:15pt;mso-height-rule:exactly'><td><p>Oben</p><p style='color:#FF0000'>Verborgen</p></td></tr><tr><td>Danach</td></tr></table>";
    let mut d=doc_io::clipboard_html_decode(h).unwrap();let mut text=text::Text::new(style::STYLE.fonts(),style::STYLE.font_names());let fonts=doc_view::Fonts::new(&mut text).unwrap();
    let page=doc_view::PageDraw::new(Rc::new(doc_view::paginate(&d,&mut text,&fonts).unwrap()),0,fonts.clone()).unwrap();let pixels=page.thumbnail(&mut text,900).unwrap();
    check!(!pixels.px.chunks_exact(4).any(|p|p[2]>200&&p[0]<100&&p[1]<100),"clipped red paragraph paints no pixels");
    d.tables[0].rows[0].height_rule=RowHeightRule::AtLeast;let page=doc_view::PageDraw::new(Rc::new(doc_view::paginate(&d,&mut text,&fonts).unwrap()),0,fonts).unwrap();let pixels=page.thumbnail(&mut text,900).unwrap();
    check!(pixels.px.chunks_exact(4).any(|p|p[2]>200&&p[0]<100&&p[1]<100),"same hidden content paints after returning to minimum height");
    (passed,failed)
}
