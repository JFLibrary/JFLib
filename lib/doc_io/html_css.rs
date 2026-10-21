//! Supported clipboard CSS values. Everything is parsed as bounded numbers, never executed.
use super::*;
use doc_core::{BorderStyle,CellBorder,CellBorders};
fn number(s:&str)->Result<(i64,&str),Error>{
    let s=s.trim();let end=s.find(|c:char|!c.is_ascii_digit()&&!matches!(c,'.'|'+'|'-')).unwrap_or(s.len());let n=&s[..end];if !n.bytes().any(|b|b.is_ascii_digit()){return Err(Error::Invalid);}
    let negative=n.starts_with('-');let n=n.trim_start_matches(['+','-']);let (whole,decimal)=n.split_once('.').unwrap_or((n,""));
    if decimal.len()>6 || !decimal.bytes().all(|c|c.is_ascii_digit()){return Err(Error::Invalid);}
    let whole=if whole.is_empty(){0}else{whole.parse::<i64>().map_err(|_|Error::Invalid)?};
    let fraction=if decimal.is_empty(){0}else{decimal.parse::<i64>().map_err(|_|Error::Invalid)?*10i64.pow(6-decimal.len() as u32)};
    let v=whole.checked_mul(1_000_000).and_then(|v|v.checked_add(fraction)).filter(|v|*v<=1_000_000_000_000).ok_or(Error::Limit)?;
    Ok((if negative{-v}else{v},s[end..].trim()))
}
pub(super) fn twips(s:&str)->Result<i32,Error>{let(v,unit)=number(s)?;let (n,d)=match unit.to_ascii_lowercase().as_str(){""|"px"=>(15,1),"pt"=>(20,1),"in"=>(1440,1),"cm"=>(144000,254),"mm"=>(14400,254),_=>return Err(failure("CSS length unit"))};
    let result=v*n/(d*1_000_000);i32::try_from(result).map_err(|_|Error::Limit)
}
pub(super) fn length(s:&str)->Result<u32,Error>{let v=twips(s)?;if !(0..=31680).contains(&v){return Err(Error::Invalid);}Ok(v as u32)}
pub(super) fn uniform(s:&str)->Result<u32,Error>{let mut value=None;for token in s.split_whitespace(){let n=length(token)?;if value.is_some_and(|v|v!=n){return Err(failure("nonuniform padding"));}value=Some(n);}value.ok_or(Error::Invalid)}
pub(super) fn color(s:&str)->Result<Option<u32>,Error>{let s=s.trim().to_ascii_lowercase();let v=s.as_str();
    if matches!(v,"transparent"|"none"){return Ok(None);}
    if let Some(hex)=v.strip_prefix('#'){return match hex.len(){3=>{let n=u32::from_str_radix(hex,16).map_err(|_|Error::Invalid)?;Ok(Some(((n>>8)*17<<16)|(((n>>4)&15)*17<<8)|((n&15)*17)))},6=>Ok(Some(u32::from_str_radix(hex,16).map_err(|_|Error::Invalid)?)),_=>Err(Error::Invalid)};}
    if let Some(rgb)=v.strip_prefix("rgb(").and_then(|v|v.strip_suffix(')')){let mut out=0;let channels:Vec<_>=rgb.split(',').collect();if channels.len()!=3{return Err(Error::Invalid);}for c in channels{let n=c.trim().parse::<u8>().map_err(|_|Error::Invalid)?;out=out<<8|u32::from(n);}return Ok(Some(out));}
    Ok(Some(match v {"black"|"windowtext"|"currentcolor"=>0,"white"|"window"=>0xffffff,"red"=>0xff0000,"green"=>0x008000,"blue"=>0x0000ff,"yellow"=>0xffff00,"gray"|"grey"=>0x808080,"silver"=>0xc0c0c0,"navy"=>0x000080,"teal"=>0x008080,"purple"=>0x800080,"maroon"=>0x800000,"lime"=>0x00ff00,"aqua"=>0x00ffff,"fuchsia"=>0xff00ff,"olive"=>0x808000,_=>return Err(failure("CSS color"))}))
}
fn family(s:&str)->Result<FontFamily,Error>{let first=s.split(',').next().unwrap_or("").trim().trim_matches(['\'','"']);let mut out=String::new();let mut chars=first.chars().peekable();
    while let Some(c)=chars.next(){if c!='\\'{out.push(c);continue;}let mut hex=String::new();while hex.len()<6 && chars.peek().is_some_and(|c|c.is_ascii_hexdigit()){hex.push(chars.next().unwrap());}
        if hex.is_empty(){out.push(chars.next().ok_or(Error::Invalid)?);}else{out.push(u32::from_str_radix(&hex,16).ok().and_then(char::from_u32).ok_or(Error::Invalid)?);if chars.peek().is_some_and(|c|c.is_ascii_whitespace()){chars.next();}}
    }Ok(FontFamily::new(&out)?)
}
pub(super) fn context(n:&Node,mut c:Context)->Result<Context,Error>{
    match n.tag.as_str(){"b"|"strong"|"th"=>c.text.bold=Some(true),"i"|"em"=>c.text.italic=Some(true),"u"=>c.text.underline=Some(true),"s"|"strike"|"del"=>c.text.strike=Some(true),"sup"=>c.text.positioning=Some(TextPositioning::Superscript),"sub"=>c.text.positioning=Some(TextPositioning::Subscript),"pre"=>c.pre=true,_=>()}
    if let Some(v)=n.style("white-space"){c.pre=match v{"pre"|"pre-wrap"|"break-spaces"=>true,"normal"|"nowrap"=>false,_=>return Err(failure("white-space"))};}
    if let Some(v)=n.style("font-family").or_else(||n.attr("face")){c.text.font_family=Some(family(v)?);}
    if let Some(v)=n.style("font-size"){c.text.size_half_points=Some(u16::try_from(length(v)?/10).map_err(|_|Error::Invalid)?);}
    if let Some(v)=n.style("font-weight"){c.text.bold=Some(match v{"bold"|"bolder"|"600"|"700"|"800"|"900"=>true,"normal"|"lighter"|"100"|"200"|"300"|"400"|"500"=>false,_=>return Err(failure("font weight"))});}
    if let Some(v)=n.style("font-style"){c.text.italic=Some(matches!(v,"italic"|"oblique"));}
    if let Some(v)=n.style("text-decoration"){c.text.underline=Some(v.split_whitespace().any(|v|v=="underline"));c.text.strike=Some(v.split_whitespace().any(|v|v=="line-through"));}
    if let Some(v)=n.style("color").or_else(||n.attr("color")){c.text.color=Some(color(v)?.map_or(TextColor::Auto,TextColor::Rgb));}
    if matches!(n.tag.as_str(),"span"|"mark"){if let Some(v)=n.style("background-color").or_else(||n.style("background")){c.text.highlight=Some(match color(v)?{None=>Highlight::None,Some(rgb)=>Highlight::from_rgb(rgb).ok_or_else(||failure("text highlight outside supported palette"))?});}}
    if let Some(v)=n.style("vertical-align").filter(|_|n.tag=="span"){c.text.positioning=Some(match v{"baseline"=>TextPositioning::Baseline,"super"=>TextPositioning::Superscript,"sub"=>TextPositioning::Subscript,_=>{let shift=twips(v)?;if shift>0{TextPositioning::Superscript}else if shift<0{TextPositioning::Subscript}else{TextPositioning::Baseline}}});}
    // Legacy TABLE align positions the table, while text-align still styles its cell text.
    if let Some(v)=n.style("text-align").or_else(||n.attr("align").filter(|_|n.tag!="table")) {c.paragraph.alignment=match v{"left"|"start"=>Alignment::Left,"center"=>Alignment::Center,"right"|"end"=>Alignment::Right,"justify"=>Alignment::Justify,_=>return Err(failure("paragraph alignment"))};}
    // A table's margins position its box; they are not inherited paragraph indents.
    if n.tag!="table" {
        if let Some(v)=n.style("margin"){let a:Vec<u32>=v.split_whitespace().map(length).collect::<Result<_,_>>()?;let(top,right,bottom,left)=match a.as_slice(){[a]=>(*a,*a,*a,*a),[a,b]=>(*a,*b,*a,*b),[a,b,c]=>(*a,*b,*c,*b),[a,b,c,d]=>(*a,*b,*c,*d),_=>return Err(Error::Invalid)};c.paragraph.before=top as u16;c.paragraph.after=bottom as u16;c.paragraph.left=left as u16;c.paragraph.right=right as u16;}
        for (key,target) in [("margin-top",&mut c.paragraph.before),("margin-bottom",&mut c.paragraph.after),("margin-left",&mut c.paragraph.left),("margin-right",&mut c.paragraph.right)]{if let Some(v)=n.style(key){*target=length(v)? as u16;}}
    }
    if let Some(v)=n.style("text-indent"){c.paragraph.first_line=twips(v)?;}
    if let Some(v)=n.style("line-height"){if v!="normal"{let(a,unit)=number(v)?;if !unit.is_empty() && unit!="%"{return Err(failure("absolute CSS line height"));}c.paragraph.line=u16::try_from(a*240/if unit=="%"{100_000_000}else{1_000_000}).map_err(|_|Error::Invalid)?;}}
    if let Some(v)=n.style("page-break-before"){c.paragraph.page_break_before=v=="always";}
    c.text.validate()?;Ok(c)
}
fn border_width(s:&str)->Result<u32,Error>{let(v,unit)=number(s)?;if v<0{return Err(Error::Invalid);}let(n,d)=match unit.to_ascii_lowercase().as_str(){""|"px"=>(6,1),"pt"=>(8,1),"in"=>(576,1),"cm"=>(57600,254),"mm"=>(5760,254),_=>return Err(failure("border width unit"))};u32::try_from((v*n+d*500000)/(d*1000000)).map_err(|_|Error::Limit)}
fn border(s:&str)->Result<CellBorder,Error>{let mut style=BorderStyle::Single;let mut width=8;let mut rgb=0;
    for part in s.split_whitespace(){match part {"none"=>style=BorderStyle::None,"hidden"=>style=BorderStyle::Nil,"solid"=>style=BorderStyle::Single,"double"=>style=BorderStyle::Double,"dotted"=>style=BorderStyle::Dotted,"dashed"=>style=BorderStyle::Dashed,"thin"=>width=6,"medium"=>width=18,"thick"=>width=30,_ if part.starts_with('#') || part.chars().all(|c|c.is_ascii_alphabetic())=>rgb=color(part)?.unwrap_or(0),_=>width=border_width(part)?}}
    if matches!(style,BorderStyle::Nil|BorderStyle::None)||width==0{return Ok(CellBorder{style:if style==BorderStyle::Nil{style}else{BorderStyle::None},width:0,color:0});}
    let width=(if style==BorderStyle::Double{(width+1)/3}else{width}).max(2);let b=CellBorder{style,width:u16::try_from(width).map_err(|_|Error::Invalid)?,color:rgb};b.validate()?;Ok(b)
}
pub(super) fn borders(n:&Node)->Result<CellBorders,Error>{let mut out=[None;4];if let Some(v)=n.style("border"){out=[Some(border(v)?);4];}
    for side in doc_core::BorderSide::ALL {if let Some(v)=n.style(&format!("border-{}",side.name())){out[side as usize]=Some(border(v)?);}}Ok(out)
}
