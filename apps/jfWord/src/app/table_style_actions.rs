// Discovery and editing of shared, document-owned table templates.
use super::*;
use doc_core::{TableLook, TableRegion, TableRegionKind, TableStyle};
static KEY:Schema=Schema::String{min:1,max:64,choices:&[]};
static RGB:Schema=Schema::Unsigned{min:0,max:0xffffff};
static REGION:Schema=Schema::Object{properties:&[
    property("fill",&Schema::String{min:0,max:6,choices:&[]},false),
    property("format",&CHARACTER,false),
],additional:false};
pub(super) static LOOK:Schema=Schema::Object{properties:&[
    property("firstRow",&BOOLEAN,false),property("lastRow",&BOOLEAN,false),
    property("firstColumn",&BOOLEAN,false),property("lastColumn",&BOOLEAN,false),property("bandedRows",&BOOLEAN,false),
    property("bandedColumns",&BOOLEAN,false),
],additional:false};
pub(super) static LIST:Schema=Schema::Object{properties:&[property("offset",&U64,false),property("limit",&Schema::Unsigned{min:1,max:32},false)],additional:false};
pub(super) static PUT:Schema=Schema::Object{properties:&[
    property("id",&KEY,true),property("name",&Schema::String{min:1,max:128,choices:&[]},true),
    property("cloneFrom",&KEY,false),property("applyTo",&U64,false),
    property("basedOn",&Schema::String{min:0,max:64,choices:&[]},false),property("inheritBorder",&BOOLEAN,false),
    property("rowBandSize",&Schema::Unsigned{min:0,max:256},false),property("columnBandSize",&Schema::Unsigned{min:0,max:32},false),
    property("borderWidth",&Schema::Unsigned{min:0,max:80},false),property("borderColor",&RGB,false),
    property("body",&REGION,false),property("firstRow",&REGION,false),property("lastRow",&REGION,false),
    property("firstColumn",&REGION,false),property("lastColumn",&REGION,false),property("band1",&REGION,false),property("band2",&REGION,false),
    property("band1Vertical",&REGION,false),property("band2Vertical",&REGION,false),
    property("topLeft",&REGION,false),property("topRight",&REGION,false),property("bottomLeft",&REGION,false),property("bottomRight",&REGION,false),
],additional:false};
pub(super) fn look(v:&Value,mut l:TableLook)->TableLook{
    for (key,b) in [("firstRow",&mut l.first_row),("lastRow",&mut l.last_row),("firstColumn",&mut l.first_column),("lastColumn",&mut l.last_column),("bandedRows",&mut l.banded_rows),("bandedColumns",&mut l.banded_columns)]{
        if let Some(value)=v.get(key).and_then(Value::as_bool){*b=value;}
    }l
}
pub(super) fn look_json(l:TableLook)->Value{
    Value::object([("firstRow",Value::Bool(l.first_row)),("lastRow",Value::Bool(l.last_row)),("firstColumn",Value::Bool(l.first_column)),("lastColumn",Value::Bool(l.last_column)),("bandedRows",Value::Bool(l.banded_rows)),("bandedColumns",Value::Bool(l.banded_columns))])
}
fn region(r:TableRegion)->Value{
    Value::object([("fill",r.fill.map(|c|Value::string(format!("{c:06X}"))).unwrap_or(Value::Null)),("format",format_json(r.text))])
}
fn definition(s:&TableStyle)->Value{
    let mut values=alloc::vec![
        ("id",Value::string(&s.id)),("name",Value::string(&s.name)),
        ("basedOn",s.based_on.as_ref().map(Value::string).unwrap_or(Value::Null)),
        ("inheritBorder",Value::Bool(s.border_inherit)),
        ("borderWidth",Value::u64(s.border_width.into())),("borderColor",Value::u64(s.border_color.into())),
        ("rowBandSize",s.row_band_size.map(|v|Value::u64(v.into())).unwrap_or(Value::Null)),
        ("columnBandSize",s.column_band_size.map(|v|Value::u64(v.into())).unwrap_or(Value::Null)),
    ];
    for kind in TableRegionKind::ALL {values.push((kind.key(),region(s.region_format(kind))));}
    Value::Object(values.into_iter().map(|(k,v)|(String::from(k),v)).collect())
}
pub(super) fn list(d:&Document,v:&Value)->Value{
    let ids=d.table_style_ids();let offset=id(v,"offset") as usize;let limit=v.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
    let styles=ids.iter().skip(offset).take(limit).map(|id|{
        let s=d.table_style(id).unwrap();let mut raw=definition(&s);
        if let Value::Object(values)=&mut raw {values.push((String::from("effective"),d.resolved_table_style(id).as_ref().map(definition).unwrap_or(Value::Null)));}
        raw
    }).collect();
    Value::object([("styles",Value::Array(styles)),("nextOffset",if offset.saturating_add(limit)<ids.len(){Value::u64((offset+limit) as u64)}else{Value::Null})])
}
pub(super) fn put(d:&Document,v:&Value)->Result<Edit,ActionError>{
    let key=text(v,"id");let name=text(v,"name");
    if v.get("cloneFrom").is_some() && v.get("basedOn").is_some(){return Err(invalid("Choose cloneFrom or basedOn"));}
    let mut s=if let Some(base)=v.get("cloneFrom").and_then(Value::as_str){d.resolved_table_style(base).map_err(|_|invalid("Missing cloneFrom table style"))?}
        else {d.table_style(&key).unwrap_or_else(||{
            if let Some(base)=v.get("basedOn").and_then(Value::as_str).filter(|b|!b.is_empty()){TableStyle::derived(&key,&name,base)}else{TableStyle::new(&key,&name)}
        })};
    s.id=key;s.name=name;
    if let Some(base)=v.get("basedOn").and_then(Value::as_str){s.based_on=if base.is_empty(){None}else{Some(String::from(base))};}
    if let Some(n)=v.get("rowBandSize").and_then(Value::as_u64){s.row_band_size=if n==0{None}else{Some(n as u16)};}
    if let Some(n)=v.get("columnBandSize").and_then(Value::as_u64){s.column_band_size=if n==0{None}else{Some(n as u16)};}
    if v.get("borderWidth").is_some() || v.get("borderColor").is_some() {
        if s.border_inherit {let resolved=d.resolved_table_style(s.based_on.as_deref().unwrap_or(&s.id)).ok();if let Some(r)=resolved{s.border_width=r.border_width;s.border_color=r.border_color;}}
        s.border_inherit=false;
    }
    if let Some(n)=v.get("borderWidth").and_then(Value::as_u64){s.border_width=n as u16;}
    if let Some(n)=v.get("borderColor").and_then(Value::as_u64){s.border_color=n as u32;}
    if let Some(inherit)=v.get("inheritBorder").and_then(Value::as_bool){
        if inherit && (v.get("borderWidth").is_some() || v.get("borderColor").is_some()){return Err(invalid("Inherited borders cannot also have explicit values"));}
        if !inherit && s.border_inherit {if let Ok(r)=d.resolved_table_style(s.based_on.as_deref().unwrap_or(&s.id)){s.border_width=r.border_width;s.border_color=r.border_color;}}
        s.border_inherit=inherit;if inherit{s.border_width=4;s.border_color=0x808080;}
    }
    for kind in TableRegionKind::ALL {
        if let Some(value)=v.get(kind.key()){let r=s.region_format_mut(kind);
            if let Some(fill)=value.get("fill").and_then(Value::as_str){r.fill=if fill.is_empty(){None}else{if fill.len()!=6{return Err(invalid("Expected six hexadecimal RGB digits"));}Some(u32::from_str_radix(fill,16).map_err(|_|invalid("Invalid RGB"))?)};}
            if let Some(format)=value.get("format"){r.text=style_actions::character(format)?;}
        }
    }
    s.validate().map_err(|e|invalid(format!("Invalid table style: {e:?}")))?;
    Ok(Edit::SetTableStyle{style:s,table:v.get("applyTo").and_then(Value::as_u64)})
}
