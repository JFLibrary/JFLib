//! Static HTML table positioning and the versioned, lossless JFLib exchange attributes.
use super::*;
use doc_core::{TablePosition,TableHorizontal as H,TableVertical as V};

pub(super) fn read(n:&Node)->Result<(Option<TablePosition>,Option<i32>),Error>{
    if let Some(s)=n.attr("data-jf-table-position"){
        let a:Vec<_>=s.split_whitespace().collect();if a.len()!=10||a[0]!="1"{return Err(failure("table position version or fields"));}
        let horizontal=match a[1]{"page"=>H::Page,"margin"=>H::Margin,"column"=>H::Column,_=>return Err(Error::Invalid)};
        let vertical=match a[2]{"page"=>V::Page,"margin"=>V::Margin,"paragraph"=>V::Paragraph,_=>return Err(Error::Invalid)};
        let x=a[3].parse().map_err(|_|Error::Invalid)?;let y=a[4].parse().map_err(|_|Error::Invalid)?;
        let mut distance=[0;4];for (i,v) in distance.iter_mut().enumerate(){*v=a[i+5].parse().map_err(|_|Error::Invalid)?;}
        let overlap=match a[9]{"0"=>false,"1"=>true,_=>return Err(Error::Invalid)};
        return Ok((Some(TablePosition{horizontal,vertical,x,y,distance,overlap}),None));
    }
    let position=n.style("position").unwrap_or("static").to_ascii_lowercase();
    let float=n.style("float").or_else(||n.attr("align").filter(|v|v.eq_ignore_ascii_case("left")||v.eq_ignore_ascii_case("right"))).unwrap_or("none").to_ascii_lowercase();
    if !matches!(position.as_str(),"static"|"absolute"){return Err(failure("table positioning scheme"));}
    if !matches!(float.as_str(),"none"|"left"|"right"){return Err(failure("table float direction"));}
    if float=="none"&&position=="static"{return Ok((None,None));}
    let mut distance=[0;4];
    if let Some(s)=n.style("margin"){let a:Vec<_>=s.split_whitespace().map(css::length).collect::<Result<_,_>>()?;distance=match a.as_slice(){[a]=>[*a;4],[a,b]=>[*a,*b,*a,*b],[a,b,c]=>[*a,*b,*c,*b],[a,b,c,d]=>[*a,*b,*c,*d],_=>return Err(Error::Invalid)}.map(|v|v as u16);}
    for (i,key) in ["margin-top","margin-right","margin-bottom","margin-left"].iter().enumerate(){if let Some(s)=n.style(key){distance[i]=css::length(s)? as u16;}}
    let mut p=TablePosition{distance,x:i32::from(distance[3]),y:i32::from(distance[0]),..Default::default()};
    let mut right=if float=="right"{Some(i32::from(distance[1]))}else{None};
    if position=="absolute" {
        if n.style("bottom").is_some()||n.style("left").is_some()&&n.style("right").is_some(){return Err(failure("overconstrained table position"));}
        p.horizontal=H::Page;p.vertical=V::Page;p.overlap=true;
        p.x+=n.style("left").map(css::twips).transpose()?.unwrap_or(0);
        p.y+=n.style("top").map(css::twips).transpose()?.unwrap_or(0);
        right=n.style("right").map(css::twips).transpose()?.map(|r|r+i32::from(distance[1]));
    }else if ["top","left","right","bottom"].iter().any(|k|n.style(k).is_some()) {return Err(failure("offsets on a static table"));}
    Ok((Some(p),right))
}
pub(super) fn page(nodes:&[Node])->Result<PageSettings,Error>{
    let mut page=None;
    for n in nodes {if let Some(s)=n.attr("data-jf-page"){
        if page.is_some(){return Err(failure("multiple page settings"));}
        let a:Vec<_>=s.split_whitespace().collect();if a.len()!=7||a[0]!="1"{return Err(failure("page settings version or fields"));}
        let n=|i:usize|a[i].parse::<u32>().map_err(|_|Error::Invalid);
        let value=PageSettings{width:n(1)?,height:n(2)?,top:n(3)?,right:n(4)?,bottom:n(5)?,left:n(6)?};value.validate()?;page=Some(value);
    }}Ok(page.unwrap_or_default())
}
