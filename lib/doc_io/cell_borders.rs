//! Physical cell border declarations. Unsupported drawing features remain preserved DOCX content.
use super::*;
use doc_core::{BorderSide,BorderStyle,CellBorder,CellBorders};
pub(super) fn write(borders:&CellBorders)->String {
    if borders.iter().all(Option::is_none){return String::new();}
    let mut s=String::from("<w:tcBorders>");
    // WordprocessingML schema order differs from our clockwise indexing.
    for side in [BorderSide::Top,BorderSide::Left,BorderSide::Bottom,BorderSide::Right] {
        if let Some(b)=borders[side as usize] {
            s.push_str(&format!("<w:{} w:val=\"{}\"",side.name(),b.style.name()));
            if b.visible(){s.push_str(&format!(" w:sz=\"{}\" w:color=\"{:06X}\"",b.width,b.color));}
            s.push_str("/>");
        }
    }
    s.push_str("</w:tcBorders>");s
}
pub(super) fn read(n:&Node)->Result<CellBorders,Error>{
    container(n,W,"tcBorders")?;let mut out=[None;4];
    for edge in &n.children {
        let side=BorderSide::ALL.into_iter().find(|s|edge.is(W,s.name())).ok_or_else(||unsupported("Zellrahmenrichtung"))?;
        if out[side as usize].is_some() || !edge.children.is_empty() || !edge.text.is_empty(){return Err(Error::Invalid);}
        for (key,value) in &edge.attrs {
            if key.ns!=W || !["val","sz","color","space","shadow","frame"].contains(&key.local.as_str()) {return Err(unsupported("Zellrahmenattribut"));}
            if key.local=="space" && value!="0" || ["shadow","frame"].contains(&key.local.as_str()) && !["0","false","off"].contains(&value.as_str()){return Err(unsupported("Zellrahmenabstand oder Effekt"));}
        }
        let style=BorderStyle::parse(edge.attr(W,"val").ok_or(Error::Invalid)?).ok_or_else(||unsupported("Zellrahmenstil"))?;
        let mut b=CellBorder{style,width:0,color:0};
        if !matches!(style,BorderStyle::Nil|BorderStyle::None){
            b.width=edge.attr(W,"sz").unwrap_or("4").parse().map_err(|_|Error::Invalid)?;
            let rgb=edge.attr(W,"color").unwrap_or("auto");
            b.color=if rgb=="auto" {0} else {if rgb.len()!=6{return Err(Error::Invalid);}u32::from_str_radix(rgb,16).map_err(|_|Error::Invalid)?};
        } else if edge.attr(W,"sz").is_some_and(|s|s.parse::<u16>().is_err()) {return Err(Error::Invalid);}
        b.validate().map_err(|_|Error::Invalid)?;out[side as usize]=Some(b);
    }
    Ok(out)
}
