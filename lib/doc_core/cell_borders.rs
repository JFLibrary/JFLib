//! Physical cell borders and deterministic shared-edge resolution, independent of pixels.
use crate::{Document,Error,Table};

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum BorderStyle { Nil, None, Single, Double, Dotted, Dashed }
impl BorderStyle {
    pub fn name(self)->&'static str {match self {Self::Nil=>"nil",Self::None=>"none",Self::Single=>"single",Self::Double=>"double",Self::Dotted=>"dotted",Self::Dashed=>"dashed"}}
    pub fn parse(s:&str)->Option<Self>{Some(match s {"nil"=>Self::Nil,"none"=>Self::None,"single"=>Self::Single,"double"=>Self::Double,"dotted"=>Self::Dotted,"dashed"=>Self::Dashed,_=>return Option::None})}
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum BorderSide { Top, Right, Bottom, Left }
impl BorderSide {
    pub const ALL:[Self;4]=[Self::Top,Self::Right,Self::Bottom,Self::Left];
    pub fn name(self)->&'static str{match self{Self::Top=>"top",Self::Right=>"right",Self::Bottom=>"bottom",Self::Left=>"left"}}
    pub fn opposite(self)->Self{match self{Self::Top=>Self::Bottom,Self::Right=>Self::Left,Self::Bottom=>Self::Top,Self::Left=>Self::Right}}
}
/// Width is 2..96 eighth-points for visible lines, color is opaque RGB. Nil suppresses a shared edge;
/// None follows the table default. An absent cell property inherits without a local declaration.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub struct CellBorder {pub style:BorderStyle,pub width:u16,pub color:u32}
pub type CellBorders=[Option<CellBorder>;4];
impl CellBorder {
    pub const NIL:Self=Self{style:BorderStyle::Nil,width:0,color:0};
    pub fn validate(self)->Result<(),Error>{
        if self.color>0xffffff || match self.style {BorderStyle::Nil|BorderStyle::None=>self.width!=0 || self.color!=0,_=>self.width<2 || self.width>96}{Err(Error::Invalid)}else{Ok(())}
    }
    pub fn visible(self)->bool{!matches!(self.style,BorderStyle::Nil|BorderStyle::None) && self.width>0}
    /// Word's zero-spacing conflict rules for the supported line styles.
    pub fn competing(self,other:Self)->Self{
        if self.style==BorderStyle::Nil || other.style==BorderStyle::Nil{return Self::NIL;}
        if !self.visible(){return other;}if !other.visible(){return self;}
        let weight=|b:Self|match b.style{BorderStyle::Dotted|BorderStyle::Dashed=>1,BorderStyle::Double=>b.width*3,_=>b.width};
        if weight(self)!=weight(other){return if weight(self)>weight(other){self}else{other};}
        let priority=|b:Self|match b.style{BorderStyle::Single=>0,BorderStyle::Double=>1,BorderStyle::Dotted=>2,_=>3};
        if priority(self)!=priority(other){return if priority(self)<priority(other){self}else{other};}
        let brightness=|rgb:u32|{let(r,g,b)=(rgb>>16,(rgb>>8)&255,rgb&255);(r+b+2*g,b+2*g,g)};
        if brightness(self.color)<=brightness(other.color){self}else{other}
    }
}
impl Document {
    pub fn cell_border(&self,t:&Table,row:usize,column:usize,side:BorderSide)->CellBorder{
        if let Some(b)=t.cell(row,column).and_then(|c|c.borders[side as usize]).filter(|b|b.style!=BorderStyle::None){return b;}
        let(width,color)=self.table_border(t);
        if width==0{CellBorder::NIL}else{CellBorder{style:BorderStyle::Single,width,color}}
    }
    /// One grid segment on a possibly merged cell edge. `unit` counts columns for
    /// top/bottom and rows for left/right. Direct cell properties precede table borders.
    pub fn shared_cell_border(&self,t:&Table,row:usize,column:usize,side:BorderSide,unit:usize)->CellBorder{
        let Some((r,c))=t.origin(row,column)else{return CellBorder::NIL;};let cell=&t.rows[r].cells[c];
        let units=if matches!(side,BorderSide::Top|BorderSide::Bottom){cell.column_span}else{cell.row_span};
        if unit>=usize::from(units){return CellBorder::NIL;}
        let own=self.cell_border(t,r,c,side);
        let neighbor=match side {
            BorderSide::Top=>r.checked_sub(1).map(|r|(r,c+unit)),
            BorderSide::Bottom=>Some((r+usize::from(cell.row_span),c+unit)),
            BorderSide::Left=>c.checked_sub(1).map(|c|(r+unit,c)),
            BorderSide::Right=>Some((r+unit,c+usize::from(cell.column_span))),
        };
        let Some((nr,nc))=neighbor.filter(|(r,c)|*r<t.rows.len() && *c<t.columns.len())else{return own;};
        let Some(other_cell)=t.cell(nr,nc)else{return own;};let opposite=side.opposite();let other=self.cell_border(t,nr,nc,opposite);
        let direct=cell.borders[side as usize].is_some_and(|b|b.style!=BorderStyle::None);
        let other_direct=other_cell.borders[opposite as usize].is_some_and(|b|b.style!=BorderStyle::None);
        if direct!=other_direct{return if direct{own}else{other};}
        if matches!(side,BorderSide::Top|BorderSide::Left){other.competing(own)}else{own.competing(other)}
    }
}
