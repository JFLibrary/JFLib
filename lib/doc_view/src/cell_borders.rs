//! Native and WASM use the same resolved document edges and pixel phase.
use doc_core::{BorderSide,BorderStyle};
use doc_layout::CellBox;
use ui::Gfx;
use crate::PageTransform;
pub(super) fn draw(g:&mut Gfx,cell:&CellBox,transform:PageTransform,r:[i32;4]) {
    let [x,y,w,h]=cell.rect;
    for edge in &cell.edges {
        if !edge.border.visible() || edge.length<=0 {continue;}
        let horizontal=matches!(edge.side,BorderSide::Top|BorderSide::Bottom);
        let (axis,start,end)=if horizontal {
            (r[1]+transform.scale(y+if edge.side==BorderSide::Bottom{h}else{0}),r[0]+transform.scale(x+edge.offset),r[0]+transform.scale(x+edge.offset+edge.length))
        }else{
            (r[0]+transform.scale(x+if edge.side==BorderSide::Right{w}else{0}),r[1]+transform.scale(y+edge.offset),r[1]+transform.scale(y+edge.offset+edge.length))
        };
        let stroke=transform.scale(i32::from(edge.border.width)*5/2).max(1);
        g.dc.set_color(0xff000000|edge.border.color);
        let fill=|g:&mut Gfx,a:i32,b:i32,offset:i32|{if b>a {if horizontal {g.dc.fill(g.sf,a,axis+offset-stroke/2,b-a,stroke);}else{g.dc.fill(g.sf,axis+offset-stroke/2,a,stroke,b-a);}}};
        match edge.border.style {
            BorderStyle::Double=>{fill(g,start,end,-stroke);fill(g,start,end,stroke);},
            BorderStyle::Dotted|BorderStyle::Dashed=>{
                let dash=stroke*if edge.border.style==BorderStyle::Dotted{1}else{3};let period=dash+stroke*2;
                // Absolute phase makes both sides of a shared edge agree, including merged cells.
                let mut at=start-start.rem_euclid(period);
                while at<end {fill(g,at.max(start),(at+dash).min(end),0);at+=period;}
            },
            _=>fill(g,start,end,0),
        }
    }
}
