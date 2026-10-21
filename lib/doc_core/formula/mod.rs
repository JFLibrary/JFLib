//! Bounded decimal formula fields, independent of the platform and table renderer.
use alloc::string::String;
use crate::{Document, Error, Id, Inline};
mod number;
mod parser;
mod eval;
use number::Number;

/// A live calculation field. Expressions use Word's invariant English function names.
/// Results are derived from the current document; no stale cache is authoritative.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Formula {
    pub expression: String,
    pub number_format: String,
}
/// Formula failures are displayable data, not failed document edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormulaError { Syntax, Unsupported, Reference, Value, DivisionByZero, Cycle, Number, Limit }
impl FormulaError {
    /// Stable code for page rendering and automation; independent of UI language.
    pub fn code(self)->&'static str { match self {
        Self::Syntax=>"#SYNTAX!",Self::Unsupported=>"#UNSUPPORTED!",Self::Reference=>"#REF!",
        Self::Value=>"#VALUE!",Self::DivisionByZero=>"#DIV/0!",Self::Cycle=>"#CYCLE!",
        Self::Number=>"#NUM!",Self::Limit=>"#LIMIT!",
    }}
}
/// Display result with a separately inspectable error code. Numeric output is decimal text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormulaResult { pub text:String, pub error:Option<FormulaError> }
impl Formula {
    /// Parse a bounded supported expression and number picture before accepting an edit.
    pub fn new(expression:&str,number_format:&str)->Result<Self,FormulaError>{
        let expression=expression.trim();
        if expression.bytes().any(|c|c.is_ascii_control()){return Err(FormulaError::Syntax);}
        if !expression.starts_with('=')&&expression.len()>=2048{return Err(FormulaError::Limit);}
        parser::parse(expression)?;number::picture(number_format)?;
        let expression=if expression.starts_with('='){String::from(expression)}else{alloc::format!("={expression}")};
        Ok(Self{expression,number_format:String::from(number_format)})
    }
    /// Validate public codec data without evaluating cell references.
    pub fn validate(&self)->Result<(),FormulaError>{
        let canonical=Self::new(&self.expression,&self.number_format)?;
        if canonical==*self{Ok(())}else{Err(FormulaError::Syntax)}
    }
}
impl Document {
    /// Evaluate a proposed formula at a paragraph's current table position.
    /// Arithmetic also works outside tables; cell references then return #REF!.
    pub fn evaluate_formula(&self,paragraph:Id,formula:&Formula)->FormulaResult {
        let value=eval::evaluate(self,paragraph,formula).and_then(|n|number::display(n,&formula.number_format));
        match value {Ok(text)=>FormulaResult{text,error:None},Err(error)=>FormulaResult{text:String::from(error.code()),error:Some(error)}}
    }
    /// Inspect one calculation field at its stable paragraph and inline position.
    pub fn formula_result(&self,paragraph:Id,inline:usize)->Option<FormulaResult>{
        match self.paragraph(paragraph)?.content.get(inline)?{Inline::Formula(f)=>Some(self.evaluate_formula(paragraph,f)),_=>None}
    }
    pub(crate) fn insert_formula(&mut self,paragraph:Id,inline:usize,offset:usize,formula:Formula)->Result<(),Error>{
        formula.validate().map_err(|_|Error::Invalid)?;
        let p=self.paragraph_mut(paragraph).ok_or(Error::Missing)?;
        let node=p.content.get(inline).ok_or(Error::Missing)?;
        let text=node.text().ok_or(Error::Invalid)?;
        let prefix=text.get(..offset).ok_or(Error::Invalid)?;let suffix=text.get(offset..).ok_or(Error::Invalid)?;
        let format=node.format();
        let parts=alloc::vec![Inline::with_format(String::from(prefix),format),Inline::Formula(formula),Inline::with_format(String::from(suffix),format)];
        p.content.splice(inline..=inline,parts);Ok(())
    }
}
