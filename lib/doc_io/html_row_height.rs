//! CSS table height is a minimum. Exact heights need explicit, local row metadata.
use super::*;
use doc_core::RowHeightRule;

pub(super) fn read(row:&Node)->Result<(u32,RowHeightRule),Error>{
    if let Some(value)=row.attr("data-jf-row-height") {
        let mut fields=value.split_whitespace();
        if fields.next()!=Some("1") || fields.next()!=Some("exact"){return Err(failure("row-height metadata version/rule"));}
        let height=fields.next().ok_or(Error::Invalid)?.parse::<u32>().map_err(|_|Error::Invalid)?;
        if fields.next().is_some() || !(1..=31680).contains(&height){return Err(Error::Invalid);}
        return Ok((height,RowHeightRule::Exact));
    }
    let height=match row.style("height").or_else(||row.attr("height")) {
        None=>0,Some(v) if v.eq_ignore_ascii_case("auto")=>0,Some(v)=>css::length(v)?,
    };
    let rule=match row.style("mso-height-rule").map(str::to_ascii_lowercase).as_deref(){
        None|Some("at-least")=>RowHeightRule::AtLeast,
        Some("exactly")=>RowHeightRule::Exact,
        _=>return Err(failure("unsupported mso-height-rule")),
    };
    if rule==RowHeightRule::Exact && height==0{return Err(Error::Invalid);}
    Ok((height,rule))
}
