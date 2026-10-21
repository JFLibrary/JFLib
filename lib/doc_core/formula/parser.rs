//! Small recursive descent grammar with independent syntax and evaluation budgets.
use super::{FormulaError as E,Number};
use alloc::{boxed::Box,string::String,vec::Vec};
#[derive(Clone,Debug)]
pub(super) enum Expr {Number(Number),Reference(String,Option<String>),Unary(Box<Expr>),Binary(u8,Box<Expr>,Box<Expr>),Call(String,Vec<Expr>)}
pub(super) fn parse(text:&str)->Result<Expr,E>{
    if text.len()>2048{return Err(E::Limit);}let text=text.strip_prefix('=').unwrap_or(text);
    if !text.is_ascii(){return Err(E::Syntax);}
    let mut p=Parser{bytes:text.as_bytes(),at:0,depth:0,nodes:0};let out=p.expression(0)?;p.space();if p.at!=p.bytes.len(){return Err(E::Syntax);}Ok(out)
}
struct Parser<'a>{bytes:&'a[u8],at:usize,depth:usize,nodes:usize}
impl Parser<'_>{
    fn space(&mut self){while self.bytes.get(self.at).is_some_and(u8::is_ascii_whitespace){self.at+=1;}}
    fn peek(&mut self)->u8{self.space();self.bytes.get(self.at).copied().unwrap_or(0)}
    fn consume(&mut self,b:u8)->bool{if self.peek()==b{self.at+=1;true}else{false}}
    fn expression(&mut self,min:u8)->Result<Expr,E>{
        self.depth+=1;self.nodes+=1;if self.depth>32||self.nodes>512{return Err(E::Limit);}
        let mut left=self.atom()?;
        loop {
            let (op,precedence,length)=match self.peek(){
                b'='=>(b'=',1,1),b'<'=>if self.bytes.get(self.at+1)==Some(&b'='){(b'l',1,2)}else if self.bytes.get(self.at+1)==Some(&b'>'){(b'n',1,2)}else{(b'<',1,1)},
                b'>'=>if self.bytes.get(self.at+1)==Some(&b'='){(b'g',1,2)}else{(b'>',1,1)},
                b'+'|b'-'=>(self.bytes[self.at],2,1),b'*'|b'/'|b'%'=>(self.bytes[self.at],3,1),b'^'=>(b'^',4,1),_=>break,
            };
            if precedence<min{break;}self.at+=length;let right=self.expression(if op==b'^'{precedence}else{precedence+1})?;left=Expr::Binary(op,Box::new(left),Box::new(right));
        }
        self.depth-=1;Ok(left)
    }
    fn name(&mut self)->Result<String,E>{
        self.space();let start=self.at;while self.bytes.get(self.at).is_some_and(u8::is_ascii_alphanumeric){self.at+=1;}
        if start==self.at{return Err(E::Syntax);}Ok(core::str::from_utf8(&self.bytes[start..self.at]).unwrap().to_ascii_uppercase())
    }
    fn atom(&mut self)->Result<Expr,E>{
        if self.consume(b'-'){return Ok(Expr::Unary(Box::new(self.expression(4)?)));}
        if self.consume(b'+'){return self.expression(4);}
        if self.consume(b'('){let out=self.expression(0)?;if !self.consume(b')'){return Err(E::Syntax);}return Ok(out);}
        let b=self.peek();
        if b.is_ascii_digit()||b==b'.'{let start=self.at;while self.bytes.get(self.at).is_some_and(|b|b.is_ascii_digit()||*b==b'.'){self.at+=1;}return Ok(Expr::Number(super::number::parse(core::str::from_utf8(&self.bytes[start..self.at]).unwrap())?));}
        if !b.is_ascii_alphabetic(){return Err(E::Syntax);}let name=self.name()?;
        if self.consume(b'('){
            if !matches!(name.as_str(),"ABS"|"AND"|"AVERAGE"|"COUNT"|"DEFINED"|"FALSE"|"IF"|"INT"|"MAX"|"MIN"|"MOD"|"NOT"|"OR"|"PRODUCT"|"ROUND"|"SIGN"|"SUM"|"TRUE"){return Err(E::Unsupported);}
            let mut args=Vec::new();if !self.consume(b')'){loop{if args.len()>=64{return Err(E::Limit);}args.push(self.expression(0)?);if self.consume(b')'){break;}if !self.consume(b',')&&!self.consume(b';'){return Err(E::Syntax);}}}
            return Ok(Expr::Call(name,args));
        }
        if name=="TRUE"||name=="FALSE"{return Ok(Expr::Call(name,Vec::new()));}
        if !reference_name(&name){return Err(E::Unsupported);}let last=if self.consume(b':'){let end=self.name()?;if !reference_name(&end){return Err(E::Syntax);}Some(end)}else{None};
        Ok(Expr::Reference(name,last))
    }
}
pub(super) fn reference_name(name:&str)->bool{
    if matches!(name,"ABOVE"|"BELOW"|"LEFT"|"RIGHT"|"R"|"C"){return true;}
    let mut chars=name.bytes().peekable();let mut letters=0;while chars.peek().is_some_and(u8::is_ascii_alphabetic){letters+=1;chars.next();}
    let mut digits=0;while chars.peek().is_some_and(u8::is_ascii_digit){digits+=1;chars.next();}
    if letters>0&&digits>0&&chars.peek().is_none(){return true;}
    if name.starts_with('R'){let rest=&name[1..];if let Some((r,c))=rest.split_once('C'){return !r.is_empty()&&!c.is_empty()&&r.bytes().all(|b|b.is_ascii_digit())&&c.bytes().all(|b|b.is_ascii_digit());}}
    false
}
