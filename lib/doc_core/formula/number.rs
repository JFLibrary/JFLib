//! Nine fractional decimal digits. Checked i128 arithmetic gives identical native/WASM results.
use super::FormulaError as E;
use alloc::{format,string::{String,ToString}};
pub(super) const SCALE:i128=1_000_000_000;
const MAX:i128=1_000_000_000_000_000_000_000_000_000;
#[derive(Clone,Copy,Debug)]
pub(super) struct Number(pub i128);
impl Number {
    pub fn checked(v:i128)->Result<Self,E>{if v.abs()>MAX{Err(E::Number)}else{Ok(Self(v))}}
    pub fn integer(n:i128)->Result<Self,E>{Self::checked(n.checked_mul(SCALE).ok_or(E::Number)?)}
    pub fn add(self,b:Self)->Result<Self,E>{Self::checked(self.0.checked_add(b.0).ok_or(E::Number)?)}
    pub fn sub(self,b:Self)->Result<Self,E>{Self::checked(self.0.checked_sub(b.0).ok_or(E::Number)?)}
    pub fn mul(self,b:Self)->Result<Self,E>{Self::checked(self.0.checked_mul(b.0).ok_or(E::Number)?/SCALE)}
    pub fn div(self,b:Self)->Result<Self,E>{if b.0==0{return Err(E::DivisionByZero);}Self::checked(self.0.checked_mul(SCALE).ok_or(E::Number)?/b.0)}
    pub fn truth(self)->bool{self.0!=0}
    pub fn round(self,digits:i128)->Result<Self,E>{
        if !(-9..=9).contains(&digits){return Err(E::Number);}
        let factor=10i128.pow((9-digits) as u32);
        Self::checked(((self.0.abs()+factor/2)/factor)*factor*if self.0<0{-1}else{1})
    }
}
pub(super) fn parse(s:&str)->Result<Number,E>{
    let s=s.trim();if s.is_empty()||s.len()>64{return Err(E::Value);}
    let (s,negative)=if let Some(s)=s.strip_prefix('-'){(s,true)}else{(s.strip_prefix('+').unwrap_or(s),false)};
    let (s,percent)=s.strip_suffix('%').map_or((s,false),|s|(s,true));
    if s.bytes().filter(|c|*c==b'.'||*c==b',').count()>1{return Err(E::Value);}
    let mut value=0i128;let mut fractional=None;let mut count=0;
    for b in s.bytes(){
        if b==b'.'||b==b','{if fractional.is_some(){return Err(E::Value);}fractional=Some(0);continue;}
        if !b.is_ascii_digit(){return Err(E::Value);}count+=1;
        value=value.checked_mul(10).and_then(|n|n.checked_add(i128::from(b-b'0'))).ok_or(E::Number)?;
        if let Some(n)=&mut fractional{*n+=1;if *n>9{return Err(E::Number);}}
    }
    if count==0{return Err(E::Value);}
    value=value.checked_mul(10i128.pow(9-fractional.unwrap_or(0))).ok_or(E::Number)?;
    if negative{value=-value;}if percent{value/=100;}
    Number::checked(value)
}
pub(super) fn picture(s:&str)->Result<Option<(usize,bool,bool)>,E>{
    if s.is_empty(){return Ok(None);}if s.len()>32{return Err(E::Unsupported);}
    let (s,percent)=s.strip_suffix('%').map_or((s,false),|s|(s,true));
    let (s,group)=s.strip_prefix("#,##").map_or((s,false),|s|(s,true));
    let Some(tail)=s.strip_prefix('0')else{return Err(E::Unsupported);};
    let digits=if tail.is_empty(){0}else{let tail=tail.strip_prefix('.').ok_or(E::Unsupported)?;if tail.is_empty()||tail.len()>9||!tail.bytes().all(|c|c==b'0'){return Err(E::Unsupported);}tail.len()};
    Ok(Some((digits,group,percent)))
}
pub(super) fn display(mut n:Number,s:&str)->Result<String,E>{
    let picture=picture(s)?;
    if picture.is_some_and(|(_,_,percent)|percent){n=n.mul(Number::integer(100)?)?;}
    let digits=picture.map_or(9,|(digits,_,_)|digits);n=n.round(digits as i128)?;
    let whole=n.0.abs()/SCALE;let mut integer=format!("{whole}");
    if picture.is_some_and(|(_,group,_)|group){let mut grouped=String::new();for (i,c) in integer.chars().enumerate(){if i>0&&(integer.len()-i)%3==0{grouped.push(',');}grouped.push(c);}integer=grouped;}
    let mut out=if n.0<0{format!("-{integer}")}else{integer};
    if digits>0{let fraction=n.0.abs()%SCALE;let mut tail=format!("{fraction:09}")[..digits].to_string();if picture.is_none(){while tail.ends_with('0'){tail.pop();}}if !tail.is_empty(){out.push('.');out.push_str(&tail);}}
    if picture.is_some_and(|(_,_,percent)|percent){out.push('%');}Ok(out)
}
