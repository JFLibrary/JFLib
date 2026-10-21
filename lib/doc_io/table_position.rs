//! WordprocessingML and RTF floating-table properties (twips).
use super::*;
use doc_core::{TablePosition,TableHorizontal as H,TableVertical as V};
pub(super) fn write(p:TablePosition)->String {
    let h=match p.horizontal{H::Page=>"page",H::Margin=>"margin",H::Column=>"text"};
    let v=match p.vertical{V::Page=>"page",V::Margin=>"margin",V::Paragraph=>"text"};
    let [top,right,bottom,left]=p.distance;let overlap=if p.overlap{"overlap"}else{"never"};
    format!("<w:tblpPr w:horzAnchor=\"{h}\" w:vertAnchor=\"{v}\" w:tblpX=\"{}\" w:tblpY=\"{}\" w:topFromText=\"{top}\" w:rightFromText=\"{right}\" w:bottomFromText=\"{bottom}\" w:leftFromText=\"{left}\"/><w:tblOverlap w:val=\"{overlap}\"/>",p.x,p.y)
}
pub(super) fn read(n:&Node)->Result<TablePosition,Error>{
    if !n.children.is_empty() || !n.text.is_empty() || n.attrs.iter().any(|(k,_)|k.ns!=W || !["horzAnchor","vertAnchor","tblpX","tblpY","topFromText","rightFromText","bottomFromText","leftFromText"].contains(&k.local.as_str())){return Err(unsupported("relative floating table alignment"));}
    let mut p=TablePosition{distance:[0;4],overlap:true,..Default::default()};
    p.horizontal=match n.attr(W,"horzAnchor").unwrap_or("text"){"page"=>H::Page,"margin"=>H::Margin,"text"=>H::Column,_=>return Err(Error::Invalid)};
    p.vertical=match n.attr(W,"vertAnchor").unwrap_or("text"){"page"=>V::Page,"margin"=>V::Margin,"text"=>V::Paragraph,_=>return Err(Error::Invalid)};
    p.x=n.attr(W,"tblpX").unwrap_or("0").parse().map_err(|_|Error::Invalid)?;
    p.y=n.attr(W,"tblpY").unwrap_or("0").parse().map_err(|_|Error::Invalid)?;
    for (i,name) in ["topFromText","rightFromText","bottomFromText","leftFromText"].iter().enumerate(){p.distance[i]=n.attr(W,name).unwrap_or("0").parse().map_err(|_|Error::Invalid)?;}
    Ok(p)
}
pub(super) fn rtf(p:TablePosition)->String {
    let h=match p.horizontal{H::Page=>"tphpg",H::Margin=>"tphmrg",H::Column=>"tphcol"};
    let v=match p.vertical{V::Page=>"tpvpg",V::Margin=>"tpvmrg",V::Paragraph=>"tpvpara"};
    let [top,right,bottom,left]=p.distance;let overlap=if p.overlap{""}else{"\\tabsnoovrlp"};
    let x=if p.x<0{"tposnegx"}else{"tposx"};
    format!("\\{h}\\{v}\\{x}{}\\tposy{}\\tdfrmtxtTop{top}\\tdfrmtxtRight{right}\\tdfrmtxtBottom{bottom}\\tdfrmtxtLeft{left}{overlap} ",p.x,p.y)
}
