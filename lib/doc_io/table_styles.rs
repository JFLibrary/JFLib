// Standard WordprocessingML table style definitions and conditional formatting.
use crate::{container, esc, format, parse, unsupported, Error, Node, package::W};
use alloc::{collections::BTreeSet, string::String, vec::Vec};
use doc_core::{Document, TableLook, TableRegion, TableRegionKind, TableStyle};

const REGIONS:[(&str,TableRegionKind);12]=[
    ("band1Horz",TableRegionKind::Band1),("band2Horz",TableRegionKind::Band2),
    ("firstCol",TableRegionKind::FirstColumn),("lastCol",TableRegionKind::LastColumn),
    ("lastRow",TableRegionKind::LastRow),("firstRow",TableRegionKind::FirstRow),
    ("band1Vert",TableRegionKind::Band1Vertical),("band2Vert",TableRegionKind::Band2Vertical),
    ("nwCell",TableRegionKind::TopLeft),("neCell",TableRegionKind::TopRight),
    ("swCell",TableRegionKind::BottomLeft),("seCell",TableRegionKind::BottomRight),
];

pub(super) fn border(width: u16, color: u32) -> String {
    let mut out=String::from("<w:tblBorders>");
    for side in ["top","left","bottom","right","insideH","insideV"] {
        out.push_str(&format!("<w:{side} w:val=\"{}\" w:sz=\"{width}\" w:color=\"{color:06X}\"/>",if width==0 {"nil"} else {"single"}));
    }
    out.push_str("</w:tblBorders>");out
}
pub(super) fn read_border(n: &Node) -> Result<(u16,u32), Error> {
    container(n,W,"tblBorders")?;
    let mut names=BTreeSet::new();let mut value=None;
    for c in &n.children {
        if !["top","left","bottom","right","insideH","insideV"].contains(&c.name.local.as_str()) || !names.insert(&c.name.local) {return Err(Error::Invalid);}
        attrs(c,&["val","sz","color"])?;
        let width=c.attr(W,"sz").ok_or(Error::Invalid)?.parse::<u16>().map_err(|_|Error::Invalid)?;
        if c.attr(W,"val")!=Some("single") && !(width==0 && c.attr(W,"val")==Some("nil")) {return Err(unsupported("Tabellenrahmen"));}
        let pair=(width,rgb(c,"color")?);
        if value.is_some_and(|v|v!=pair) {return Err(unsupported("unterschiedliche Rahmen"));}value=Some(pair);
    }
    if names.len()!=6 {return Err(Error::Invalid);}value.ok_or(Error::Invalid)
}
fn attrs(n:&Node,names:&[&str])->Result<(),Error>{
    if n.name.ns!=W || !n.children.is_empty() || !n.text.is_empty() || n.attrs.len()!=names.len()
        || n.attrs.iter().any(|(k,_)|k.ns!=W || !names.contains(&k.local.as_str())) {return Err(unsupported("Tabellenformatvorlage"));}Ok(())
}
fn val(n:&Node)->Result<&str,Error>{attrs(n,&["val"])?;n.attr(W,"val").ok_or(Error::Invalid)}
fn rgb(n:&Node,key:&str)->Result<u32,Error>{let s=n.attr(W,key).ok_or(Error::Invalid)?;if s.len()!=6{return Err(Error::Invalid);}u32::from_str_radix(s,16).map_err(|_|Error::Invalid)}
fn region(r:TableRegion)->String{
    let mut s=format::write_run(r.text);
    if let Some(fill)=r.fill{s.push_str(&format!("<w:tcPr><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{fill:06X}\"/></w:tcPr>"));}s
}
fn read_region(nodes:&[Node])->Result<TableRegion,Error>{
    let mut r=TableRegion::default();let mut seen=BTreeSet::new();
    for n in nodes{
        if n.name.ns!=W || !seen.insert(&n.name.local){return Err(Error::Invalid);}
        match n.name.local.as_str(){
            "rPr"=>r.text=format::read_run(n)?,
            "tcPr"=>{container(n,W,"tcPr")?;if n.children.len()!=1 || !n.children[0].is(W,"shd"){return Err(unsupported("Tabellenvorlagen-Zelleigenschaft"));}
                let shd=&n.children[0];attrs(shd,&["val","color","fill"])?;
                if shd.attr(W,"val")!=Some("clear") || shd.attr(W,"color")!=Some("auto"){return Err(unsupported("Tabellenvorlagen-Schattierung"));}r.fill=Some(rgb(shd,"fill")?);},
            _=>return Err(unsupported(&n.name.local)),
        }
    }Ok(r)
}
pub(super) fn write(d:&Document)->Result<String,Error>{
    let mut out=String::new();
    let mut needed:BTreeSet<String>=d.table_styles.iter().map(|s|s.id.clone()).chain(d.tables.iter().filter_map(|t|t.style.clone())).collect();
    // Include built-in ancestors too, even if no table directly assigns them.
    for _ in 0..32 {
        let bases:Vec<_>=needed.iter().filter_map(|id|d.table_style(id).and_then(|s|s.based_on)).collect();
        let before=needed.len();needed.extend(bases);if before==needed.len(){break;}
    }
    for id in d.table_style_ids(){
        if !needed.contains(&id){continue;}
        let s=d.table_style(&id).ok_or(Error::Invalid)?;
        out.push_str(&format!("<w:style w:type=\"table\" w:styleId=\"{}\"><w:name w:val=\"{}\"/>",esc(&s.id)?,esc(&s.name)?));
        if let Some(base)=&s.based_on {out.push_str(&format!("<w:basedOn w:val=\"{}\"/>",esc(base)?));}
        // CT_Style orders rPr before tblPr and tcPr.
        out.push_str(&format::write_run(s.body.text));
        if !s.border_inherit || s.row_band_size.is_some() || s.column_band_size.is_some() {
            out.push_str("<w:tblPr>");
            if let Some(n)=s.row_band_size{out.push_str(&format!("<w:tblStyleRowBandSize w:val=\"{n}\"/>"));}
            if let Some(n)=s.column_band_size{out.push_str(&format!("<w:tblStyleColBandSize w:val=\"{n}\"/>"));}
            if !s.border_inherit {out.push_str(&border(s.border_width,s.border_color));}out.push_str("</w:tblPr>");
        }
        out.push_str(&region(TableRegion{fill:s.body.fill,text:Default::default()}));
        for (name,kind) in REGIONS {let r=s.region_format(kind);
            if r==TableRegion::default(){continue;}out.push_str(&format!("<w:tblStylePr w:type=\"{name}\">"));out.push_str(&region(r));out.push_str("</w:tblStylePr>");
        }
        out.push_str("</w:style>");
    }Ok(out)
}
pub(super) fn read(bytes:&[u8])->Result<Vec<TableStyle>,Error>{
    let root=parse(bytes)?;let mut out=Vec::new();let mut ids=BTreeSet::new();let defaults=Document::default();
    for node in root.children.iter().filter(|n|n.is(W,"style")){
        let id=node.attr(W,"styleId").ok_or(Error::Invalid)?;
        if !ids.insert(id){return Err(Error::Invalid);}
        if node.attr(W,"type")!=Some("table"){continue;}
        if node.attrs.len()!=2 || !node.text.is_empty(){return Err(unsupported("Tabellenvorlagenattribute"));}
        let mut s=TableStyle::new(id,"");s.border_inherit=true;let mut seen=BTreeSet::new();let mut regions=BTreeSet::new();let mut body=Vec::new();
        for n in &node.children{
            if n.name.ns!=W || (n.name.local!="tblStylePr" && !seen.insert(&n.name.local)){return Err(Error::Invalid);}
            match n.name.local.as_str(){
                "name"=>s.name=String::from(val(n)?),
                "basedOn"=>s.based_on=Some(String::from(val(n)?)),
                "tblPr"=>{container(n,W,"tblPr")?;let mut props=BTreeSet::new();for p in &n.children {
                    if p.name.ns!=W || !props.insert(&p.name.local){return Err(Error::Invalid);}
                    match p.name.local.as_str(){
                        "tblBorders"=>{(s.border_width,s.border_color)=read_border(p)?;s.border_inherit=false;},
                        "tblStyleRowBandSize"=>s.row_band_size=Some(val(p)?.parse().map_err(|_|Error::Invalid)?),
                        "tblStyleColBandSize"=>s.column_band_size=Some(val(p)?.parse().map_err(|_|Error::Invalid)?),
                        _=>return Err(unsupported("Tabellenvorlagen-Eigenschaften")),
                    }
                }},
                "rPr"|"tcPr"=>body.push(n.clone()),
                "tblStylePr"=>{
                    if n.attrs.len()!=1 || !n.text.is_empty(){return Err(Error::Invalid);}
                    let key=n.attr(W,"type").ok_or(Error::Invalid)?;if !regions.insert(key){return Err(Error::Invalid);}
                    let r=read_region(&n.children)?;
                    let kind=REGIONS.iter().find(|(name,_)|*name==key).map(|(_,kind)|*kind).ok_or_else(||unsupported("Tabellenvorlagen-Bereich"))?;
                    *s.region_format_mut(kind)=r;
                },
                _=>return Err(unsupported(&n.name.local)),
            }
        }
        s.body=read_region(&body)?;s.validate()?;
        if defaults.table_style(id).as_ref()!=Some(&s){out.push(s);}
    }Ok(out)
}
pub(super) fn look(l:TableLook)->String{
    format!("<w:tblLook w:firstRow=\"{}\" w:lastRow=\"{}\" w:firstColumn=\"{}\" w:lastColumn=\"{}\" w:noHBand=\"{}\" w:noVBand=\"{}\"/>",u8::from(l.first_row),u8::from(l.last_row),u8::from(l.first_column),u8::from(l.last_column),u8::from(!l.banded_rows),u8::from(!l.banded_columns))
}
pub(super) fn read_look(n:&Node)->Result<TableLook,Error>{
    let keys=["firstRow","lastRow","firstColumn","lastColumn","noHBand","noVBand"];
    if !n.is(W,"tblLook") || !n.children.is_empty() || !n.text.is_empty()
        || n.attrs.iter().any(|(k,_)|k.ns!=W || k.local!="val" && !keys.contains(&k.local.as_str())) {return Err(unsupported("Tabellenformatoptionen"));}
    // Word ignores the legacy bitmask when any of the six explicit attributes is present.
    let bits=if keys.iter().any(|k|n.attr(W,k).is_some()){None}else{
        let value=n.attr(W,"val").unwrap_or("0000");if value.len()!=4 {return Err(Error::Invalid);}Some(u16::from_str_radix(value,16).map_err(|_|Error::Invalid)?)
    };
    let bit=|key,mask|->Result<bool,Error>{if let Some(bits)=bits{return Ok(bits&mask!=0);}match n.attr(W,key){None|Some("0"|"false"|"off")=>Ok(false),Some("1"|"true"|"on")=>Ok(true),_=>Err(Error::Invalid)}};
    Ok(TableLook{first_row:bit("firstRow",0x20)?,last_row:bit("lastRow",0x40)?,first_column:bit("firstColumn",0x80)?,last_column:bit("lastColumn",0x100)?,banded_rows:!bit("noHBand",0x200)?,banded_columns:!bit("noVBand",0x400)?})
}
