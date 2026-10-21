//! Standard = calculation fields. Unknown pictures/switches stay in package preservation.
use crate::{Error,unsupported};
use alloc::{format,string::String};
use doc_core::Formula;
pub(super) fn write(f:&Formula)->String{
    if f.number_format.is_empty(){format!(" {} ",f.expression)}else{format!(" {} \\# \"{}\" ",f.expression,f.number_format)}
}
pub(super) fn read(instruction:&str)->Result<Formula,Error>{
    let (expression,mut switches)=instruction.split_once('\\').map_or((instruction,""),|(a,b)|(a,b));
    let mut picture=String::new();let mut seen_picture=false;let mut merge=false;
    while !switches.trim().is_empty(){
        let rest=switches.trim_start();
        if let Some(rest)=rest.strip_prefix('#'){
            if seen_picture{return Err(unsupported("Wiederholtes Zahlenformat"));}seen_picture=true;
            let rest=rest.trim_start();let (value,tail)=if let Some(rest)=rest.strip_prefix('"'){
                rest.split_once('"').ok_or(Error::Invalid)?
            }else{rest.split_once(char::is_whitespace).unwrap_or((rest,""))};
            picture=String::from(value);switches=next(tail)?;
        }else if let Some(rest)=rest.strip_prefix('*'){
            if merge{return Err(unsupported("Wiederholter Feldschalter"));}merge=true;
            let rest=rest.trim_start();let (value,tail)=rest.split_once(char::is_whitespace).unwrap_or((rest,""));
            if !value.eq_ignore_ascii_case("MERGEFORMAT"){return Err(unsupported("Formelfeldschalter"));}
            switches=next(tail)?;
        }else{return Err(unsupported("Formelfeldschalter"));}
    }
    Formula::new(expression.trim(),&picture).map_err(|_|unsupported("Formel oder Zahlenformat außerhalb des berechenbaren Profils"))
}
fn next(tail:&str)->Result<&str,Error>{let tail=tail.trim();if tail.is_empty(){Ok("")}else{tail.strip_prefix('\\').ok_or_else(||unsupported("Unvollständiger Formelfeldschalter"))}}
