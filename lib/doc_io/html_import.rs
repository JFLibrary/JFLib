//! Static HTML clipboard import. A bounded tokenizer, no DOM execution or external fetches.
use super::*;
use crate::import_blocks::{Block,InputCell,InputTable};
use doc_core::{Alignment,CellAlignment,FontFamily,Highlight,TextColor,TextPositioning};
#[path="html_css.rs"] mod css;
#[path="html_position.rs"] mod position;
#[path="html_row_height.rs"] mod row_height;
#[derive(Default)]
struct Node { tag:String, attrs:Vec<(String,String)>, children:Vec<usize>, text:String }
impl Node {
    fn attr(&self,key:&str)->Option<&str>{self.attrs.iter().find(|(k,_)|k==key).map(|(_,v)|v.as_str())}
    fn style(&self,key:&str)->Option<&str>{self.attr("style").unwrap_or("").split(';').filter_map(|p|p.split_once(':')).filter(|(k,_)|k.trim().eq_ignore_ascii_case(key)).last().map(|(_,v)|v.trim())}
    fn length(&self,key:&str)->Result<Option<u32>,Error>{self.style(key).or_else(||self.attr(key)).map(css::length).transpose()}
}
fn failure(s:&str)->Error {unsupported(&format!("HTML clipboard: {s}"))}
fn entities(s:&str)->Result<String,Error>{
    let mut out=String::new();let mut rest=s;
    while let Some(at)=rest.find('&') {out.push_str(&rest[..at]);rest=&rest[at+1..];
        let Some(end)=rest.find(';').filter(|n|*n<=32) else {out.push('&');continue;};
        let key=&rest[..end];let c=match key {
            "amp"=>Some('&'),"lt"=>Some('<'),"gt"=>Some('>'),"quot"=>Some('"'),"apos"=>Some('\''),"nbsp"=>Some('\u{a0}'),
            "ndash"=>Some('–'),"mdash"=>Some('—'),"bull"=>Some('•'),"copy"=>Some('©'),
            _ if key.starts_with("#x") || key.starts_with("#X")=>u32::from_str_radix(&key[2..],16).ok().and_then(char::from_u32),
            _ if key.starts_with('#')=>key[1..].parse::<u32>().ok().and_then(char::from_u32),_=>None};
        if let Some(c)=c {if c=='\0'{return Err(Error::Invalid);}out.push(c);rest=&rest[end+1..];}else{return Err(failure("unknown character reference"));}
    }
    out.push_str(rest);Ok(out)
}
fn tokenize(s:&str)->Result<Vec<Node>,Error>{
    let mut nodes=vec![Node::default()];let mut stack=vec![0usize];let bytes=s.as_bytes();let mut at=0;let mut text=0;
    while at<bytes.len(){if nodes.len()>=32768 || stack.len()>64{return Err(Error::Limit);}
        if bytes[at]!=b'<' {let end=s[at..].find('<').map_or(s.len(),|n|at+n);let value=entities(&s[at..end])?;text+=value.len();if text>2<<20{return Err(Error::Limit);}
            let parent=*stack.last().unwrap();let index=nodes.len();nodes[parent].children.push(index);nodes.push(Node{text:value,..Default::default()});at=end;continue;}
        if s[at..].starts_with("<!--"){at+=4;at+=s[at..].find("-->").ok_or(Error::Invalid)?+3;continue;}
        if s[at..].starts_with("<!"){at+=s[at..].find('>').ok_or(Error::Invalid)?+1;continue;}
        at+=1;let close=bytes.get(at)==Some(&b'/');if close{at+=1;}let begin=at;
        while bytes.get(at).is_some_and(|b|b.is_ascii_alphanumeric()||matches!(b,b'-'|b':')){at+=1;}
        if begin==at{return Err(Error::Invalid);}let tag=s[begin..at].to_ascii_lowercase();
        if close {while bytes.get(at).is_some_and(u8::is_ascii_whitespace){at+=1;}if bytes.get(at)!=Some(&b'>'){return Err(Error::Invalid);}at+=1;
            if let Some(pos)=stack.iter().rposition(|i|nodes[*i].tag==tag){stack.truncate(pos.max(1));}continue;}
        let mut attrs=Vec::new();let mut single=false;
        loop {while bytes.get(at).is_some_and(u8::is_ascii_whitespace){at+=1;}match bytes.get(at){Some(b'>')=>{at+=1;break;},Some(b'/') if bytes.get(at+1)==Some(&b'>')=>{at+=2;single=true;break;},None=>return Err(Error::Invalid),_=>()}
            let begin=at;while bytes.get(at).is_some_and(|b|!b.is_ascii_whitespace()&&!matches!(b,b'='|b'>'|b'/'|b'\''|b'"')){at+=1;}
            if begin==at || attrs.len()>=64{return Err(Error::Limit);}let key=s[begin..at].to_ascii_lowercase();while bytes.get(at).is_some_and(u8::is_ascii_whitespace){at+=1;}
            let mut value=String::new();if bytes.get(at)==Some(&b'='){at+=1;while bytes.get(at).is_some_and(u8::is_ascii_whitespace){at+=1;}
                let quote=bytes.get(at).copied().filter(|b|matches!(b,b'\''|b'"'));if quote.is_some(){at+=1;}let begin=at;
                while bytes.get(at).is_some_and(|b|quote.map_or(!b.is_ascii_whitespace()&&*b!=b'>',|q|*b!=q)){at+=1;}
                value=entities(&s[begin..at])?;if quote.is_some(){if at==bytes.len(){return Err(Error::Invalid);}at+=1;}}
            if !attrs.iter().any(|(k,_)|k==&key){attrs.push((key,value));}
        }
        if matches!(tag.as_str(),"script"|"style"|"head") {
            // Metadata and active content are never evaluated. Limit the scan to the input budget.
            let lower=s[at..].to_ascii_lowercase();let marker=format!("</{tag}");let end=lower.find(&marker).ok_or(Error::Invalid)?;at+=end;at+=s[at..].find('>').ok_or(Error::Invalid)?+1;continue;
        }
        // HTML permits omitted p, td/th and tr end tags. Only close within the nearest table.
        if matches!(tag.as_str(),"p"|"tr"|"td"|"th"|"tbody"|"thead"|"tfoot") {
            if let Some(pos)=stack.iter().rposition(|i|{let old=nodes[*i].tag.as_str();old==tag || matches!(tag.as_str(),"td"|"th")&&matches!(old,"td"|"th")}){
                let table=stack.iter().rposition(|i|nodes[*i].tag=="table").unwrap_or(0);if pos>table{stack.truncate(pos);}
            }
        }
        let parent=*stack.last().unwrap();let index=nodes.len();nodes[parent].children.push(index);nodes.push(Node{tag:tag.clone(),attrs,..Default::default()});
        if !single && !matches!(tag.as_str(),"br"|"col"|"meta"|"link"|"hr"|"img"|"input"){stack.push(index);}
    }
    Ok(nodes)
}
#[derive(Clone,Copy)]
struct Context { text:TextFormat, paragraph:ParagraphFormat, pre:bool }
impl Default for Context {fn default()->Self{Self{text:TextFormat::default(),paragraph:ParagraphFormat::default(),pre:false}}}
struct Story { blocks:Vec<Block>, current:Vec<Inline>, format:ParagraphFormat, space:bool }
impl Story {
    fn new()->Self{Self{blocks:Vec::new(),current:Vec::new(),format:ParagraphFormat::default(),space:false}}
    fn flush(&mut self,force:bool){if !self.current.is_empty()||force {let mut p=crate::import_blocks::empty();p.format=self.format;
        if !self.current.is_empty(){p.content=core::mem::take(&mut self.current);}
        if p.content.len()==1 && p.content[0].text()==Some("\n"){p.content[0]=Inline::with_format(String::new(),p.content[0].format());}
        self.blocks.push(Block::Paragraph(p));}self.space=false;}
    fn text(&mut self,s:&str,c:Context){let mut value=String::new();for ch in s.chars(){
        if !c.pre && ch.is_ascii_whitespace(){self.space=true;continue;}
        if self.space && (!self.current.is_empty()||!value.is_empty()){value.push(' ');}self.space=false;value.push(ch);
    }if !value.is_empty(){if self.current.is_empty(){self.format=c.paragraph;}if let Some(Inline::StyledText{text,format})=self.current.last_mut(){if *format==c.text{text.push_str(&value);return;}}
        self.current.push(Inline::with_format(value,c.text));}}
}
fn walk(nodes:&[Node],ids:&[usize],ctx:Context,out:&mut Story,depth:usize)->Result<(),Error>{
    for id in ids {let n=&nodes[*id];if n.tag.is_empty(){out.text(&n.text,ctx);continue;}
        if matches!(n.tag.as_str(),"img"|"object"|"iframe"|"svg"|"math"|"canvas"|"form"|"input"|"hr"){return Err(failure("unsupported embedded content"));}
        let c=css::context(n,ctx)?;
        if n.tag=="table"{out.flush(false);out.blocks.push(Block::Table(table(nodes,n,c,depth+1)?));continue;}
        if n.tag=="br"{out.text("\n",Context{pre:true,..c});continue;}
        let block=matches!(n.tag.as_str(),"p"|"div"|"section"|"article"|"h1"|"h2"|"h3"|"blockquote"|"pre"|"li");
        if block {out.flush(false);out.format=c.paragraph;}
        walk(nodes,&n.children,c,out,depth)?;
        if block {out.flush(n.tag=="p" || n.tag.starts_with('h') && n.tag.len()==2);}
    }Ok(())
}
// These properties are not inherited into nested rows or cell paragraphs.
fn row_break(row:&Node)->Result<bool,Error>{
    let value=row.style("break-inside").or_else(||row.style("page-break-inside"));
    match value.map(str::to_ascii_lowercase).as_deref(){
        None|Some("auto"|"initial"|"unset"|"avoid-column"|"avoid-region")=>Ok(false),
        Some("avoid"|"avoid-page")=>Ok(true),
        _=>Err(failure("unsupported row break-inside")),
    }
}
fn table(nodes:&[Node],n:&Node,c:Context,depth:usize)->Result<InputTable,Error>{
    if depth>doc_core::MAX_TABLE_DEPTH{return Err(Error::Limit);}
    let mut t=InputTable::default();let mut groups:Vec<(Vec<usize>,bool)>=Vec::new();
    for id in &n.children {let child=&nodes[*id];match child.tag.as_str(){
        "tr"=>{if groups.last().is_none_or(|g|g.1){groups.push((Vec::new(),false));}groups.last_mut().unwrap().0.push(*id);},
        "thead"|"tbody"|"tfoot"=>{let mut rows=Vec::new();for id in &child.children{if nodes[*id].tag=="tr"{rows.push(*id);}else if !nodes[*id].text.trim().is_empty(){return Err(Error::Invalid);}}groups.push((rows,child.tag=="thead"));},
        "colgroup"=>{for id in &child.children{let col=&nodes[*id];if col.tag=="col"{let span=col.attr("span").unwrap_or("1").parse::<usize>().map_err(|_|Error::Invalid)?;if span==0||span>32-t.columns.len(){return Err(Error::Limit);}let width=col.length("width")?.unwrap_or(1);for _ in 0..span{t.columns.push(width);}}}},
        "caption"=>return Err(failure("table caption")),"" if child.text.trim().is_empty()=>(),_=>return Err(failure("table structure")),
    }}
    if let Some(w)=n.style("width").or_else(||n.attr("width")){if let Some(v)=w.strip_suffix('%'){let value=v.trim().parse::<u32>().map_err(|_|Error::Invalid)?;if value==0||value>100{return Err(Error::Invalid);}t.percent=Some(value);}else{t.width=Some(css::length(w)?);}}
    (t.position,t.right)=position::read(n)?;
    if let Some(p)=n.attr("cellpadding"){t.padding=u16::try_from(css::length(p)?).map_err(|_|Error::Invalid)?;}
    if let Some(v)=n.attr("border"){t.border_width=(css::length(v)?*2/5).min(80) as u16;}
    let mut padding=None;
    for (rows,header) in groups {if header && !t.rows.is_empty(){return Err(failure("nonleading header rows"));}if header{t.headers=rows.len();}
        for (ri,id) in rows.iter().enumerate(){if t.rows.len()==256{return Err(Error::Limit);}let row=&nodes[*id];let rc=css::context(row,c)?;let mut cells=Vec::new();let (height,rule)=row_height::read(row)?;t.heights.push(height);t.height_rules.push(rule);t.cant_split.push(row_break(row)?);
            let mut gaps=(0,0);
            for id in &row.children {let cell=&nodes[*id];if cell.tag.is_empty() && cell.text.trim().is_empty(){continue;}if !matches!(cell.tag.as_str(),"td"|"th"){return Err(Error::Invalid);}
                if let Some(side)=cell.attr("data-jf-grid-gap"){
                    if !cell.text.trim().is_empty() || cell.children.iter().any(|id|!nodes[*id].tag.is_empty()||!nodes[*id].text.trim().is_empty()){return Err(Error::Invalid);}
                    let n=cell.attr("colspan").ok_or(Error::Invalid)?.parse::<usize>().map_err(|_|Error::Invalid)?;
                    if n==0||n>31||t.columns.is_empty(){return Err(Error::Invalid);}
                    match side{"before" if cells.is_empty()&&gaps.0==0&&gaps.1==0=>gaps.0=n,"after" if gaps.1==0=>gaps.1=n,_=>return Err(Error::Invalid)};continue;
                }
                if gaps.1>0{return Err(Error::Invalid);}
                let mut ic=InputCell::default();ic.columns=cell.attr("colspan").unwrap_or("1").parse().map_err(|_|Error::Invalid)?;
                ic.rows=cell.attr("rowspan").unwrap_or("1").parse().map_err(|_|Error::Invalid)?;if ic.rows==0{ic.rows=rows.len()-ri;}if ic.rows>rows.len()-ri{return Err(failure("rowspan crosses row group"));}
                ic.fill=cell.style("background-color").or_else(||cell.style("background")).or_else(||cell.attr("bgcolor")).or_else(||row.style("background-color")).map(css::color).transpose()?.flatten();
                ic.alignment=match cell.style("vertical-align").or_else(||cell.attr("valign")).unwrap_or("top"){"top"|"baseline"=>CellAlignment::Top,"middle"=>CellAlignment::Center,"bottom"=>CellAlignment::Bottom,_=>return Err(failure("cell alignment"))};
                if let Some(v)=cell.style("padding"){let p=css::uniform(v)?;if padding.is_some_and(|old|old!=p){return Err(failure("nonuniform cell padding"));}padding=Some(p);t.padding=u16::try_from(p).map_err(|_|Error::Invalid)?;}
                ic.borders=css::borders(cell)?;
                let cc=css::context(cell,rc)?;let mut story=Story::new();walk(nodes,&cell.children,cc,&mut story,depth)?;story.flush(false);ic.blocks=story.blocks;cells.push(ic);
            }if gaps!=(0,0)||!t.gaps.is_empty(){t.gaps.resize(t.rows.len(),(0,0));t.gaps.push(gaps);}t.rows.push(cells);
        }
    }Ok(t)
}
/// Decode a UTF-8 HTML clipboard fragment into editable paragraphs and nested tables.
/// Handles inline text/CSS, table grids, merged cells, headers, borders and table positions.
/// JFLib's versioned data attributes retain exact twip anchors and document page settings.
/// Input is bounded to 16 MiB, 2 MiB text, 64 element levels and eight table levels.
/// Active content and metadata are ignored; external resources are never requested.
pub fn clipboard_html_decode(bytes:&[u8])->Result<Document,Error>{
    if bytes.len()>16<<20{return Err(Error::Limit);}let s=core::str::from_utf8(bytes).map_err(|_|Error::Invalid)?;
    let nodes=tokenize(s)?;let page=position::page(&nodes)?;
    let mut story=Story::new();walk(&nodes,&nodes[0].children,Context::default(),&mut story,0)?;story.flush(false);crate::import_blocks::document_on_page(story.blocks,page)
}
