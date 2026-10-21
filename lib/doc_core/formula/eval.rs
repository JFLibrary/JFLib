//! Table-local references use stable cell ownership and the current logical grid.
use super::{Formula,FormulaError as E,Number,number::{self,SCALE},parser::{self,Expr}};
use crate::{Document,Id,Inline,Table,TableCell};
use alloc::{collections::{BTreeMap,BTreeSet},string::String,vec,vec::Vec};

pub(super) fn evaluate(d:&Document,paragraph:Id,formula:&Formula)->Result<Number,E>{
    let tree=parser::parse(&formula.expression)?;
    let mut e=Eval{d,root_cell:d.cell_for_paragraph(paragraph).map(|c|c.id),stack:Vec::new(),memo:BTreeMap::new(),steps:0,depth:0};
    e.scalar(paragraph,&tree)
}
struct Eval<'a>{d:&'a Document,root_cell:Option<Id>,stack:Vec<(Id,usize)>,memo:BTreeMap<(Id,usize),Result<Number,E>>,steps:usize,depth:usize}
impl Eval<'_>{
    fn step(&mut self)->Result<(),E>{self.steps+=1;if self.steps>16384{Err(E::Limit)}else{Ok(())}}
    fn scalar(&mut self,p:Id,tree:&Expr)->Result<Number,E>{let v=self.values(p,tree)?;if v.len()==1{Ok(v[0])}else{Err(E::Value)}}
    fn values(&mut self,p:Id,tree:&Expr)->Result<Vec<Number>,E>{
        self.step()?;
        if self.depth>=64{return Err(E::Limit);}self.depth+=1;
        let result=self.compute(p,tree);self.depth-=1;result
    }
    fn compute(&mut self,p:Id,tree:&Expr)->Result<Vec<Number>,E>{
        match tree {
            Expr::Number(n)=>Ok(vec![*n]),
            Expr::Reference(start,end)=>self.reference(p,start,end.as_deref()),
            Expr::Unary(a)=>Number::checked(-self.scalar(p,a)?.0).map(|n|vec![n]),
            Expr::Binary(op,a,b)=>{
                let a=self.scalar(p,a)?;let b=self.scalar(p,b)?;
                let n=match op {b'+'=>a.add(b)?,b'-'=>a.sub(b)?,b'*'=>a.mul(b)?,b'/'=>a.div(b)?,b'%'=>{if b.0==0{return Err(E::DivisionByZero);}Number::checked(a.0%b.0)?},
                    b'^'=>{if b.0%SCALE!=0||b.0.abs()/SCALE>32{return Err(E::Number);}let mut v=Number(SCALE);for _ in 0..b.0.abs()/SCALE{v=v.mul(a)?;}if b.0<0{Number(SCALE).div(v)?}else{v}},
                    b'='|b'<'|b'>'|b'l'|b'g'|b'n'=>Number::integer(i128::from(match op {b'='=>a.0==b.0,b'<'=>a.0<b.0,b'>'=>a.0>b.0,b'l'=>a.0<=b.0,b'g'=>a.0>=b.0,_=>a.0!=b.0}))?,_=>return Err(E::Syntax),};
                Ok(vec![n])
            },
            Expr::Call(name,args)=>self.call(p,name,args).map(|n|vec![n]),
        }
    }
    fn call(&mut self,p:Id,name:&str,args:&[Expr])->Result<Number,E>{
        if name=="IF"{if args.len()!=3{return Err(E::Value);}let condition=self.scalar(p,&args[0])?;return self.scalar(p,&args[if condition.truth(){1}else{2}]);}
        if name=="DEFINED"{if args.len()!=1{return Err(E::Value);}return Number::integer(i128::from(self.scalar(p,&args[0]).is_ok()));}
        if name=="TRUE"||name=="FALSE"{if !args.is_empty(){return Err(E::Value);}return Number::integer(i128::from(name=="TRUE"));}
        let mut values=Vec::new();for arg in args{values.extend(self.values(p,arg)?);}let v=&values;
        match name {
            "SUM"=>v.iter().try_fold(Number(0),|a,b|a.add(*b)),
            "COUNT"=>Number::integer(v.len() as i128),
            "PRODUCT"=>{if v.is_empty(){return Ok(Number(0));}v.iter().try_fold(Number(SCALE),|a,b|a.mul(*b))},
            "AVERAGE"=>{if v.is_empty(){return Err(E::DivisionByZero);}let sum=v.iter().try_fold(Number(0),|a,b|a.add(*b))?;sum.div(Number::integer(v.len() as i128)?)},
            "MIN"|"MAX"=>{if v.is_empty(){return Err(E::Value);}Ok(Number(if name=="MIN"{v.iter().map(|n|n.0).min().unwrap()}else{v.iter().map(|n|n.0).max().unwrap()}))},
            "AND"|"OR"=>{if v.is_empty(){return Err(E::Value);}Number::integer(i128::from(if name=="AND"{v.iter().all(|n|n.truth())}else{v.iter().any(|n|n.truth())}))},
            "ABS"|"INT"|"SIGN"|"NOT"=>{
                if v.len()!=1{return Err(E::Value);}let n=v[0];match name {
                    "ABS"=>Number::checked(n.0.abs()),"INT"=>Number::integer(n.0.div_euclid(SCALE)),
                    "SIGN"=>Number::integer(n.0.signum()),_=>Number::integer(i128::from(!n.truth())),
                }
            },
            "ROUND"=>{if v.len()!=2||v[1].0%SCALE!=0{return Err(E::Value);}v[0].round(v[1].0/SCALE)},
            "MOD"=>{if v.len()!=2{return Err(E::Value);}if v[1].0==0{return Err(E::DivisionByZero);}let r=v[0].0.rem_euclid(v[1].0.abs());Number::checked(if v[1].0<0&&r!=0{r-v[1].0.abs()}else{r})},
            _=>Err(E::Unsupported),
        }
    }
    fn field(&mut self,p:Id,index:usize,f:&Formula)->Result<Number,E>{
        let key=(p,index);if let Some(result)=self.memo.get(&key){return *result;}
        if self.stack.contains(&key){return Err(E::Cycle);}if self.stack.len()>=32{return Err(E::Limit);}
        self.stack.push(key);let result=parser::parse(&f.expression).and_then(|tree|self.scalar(p,&tree));self.stack.pop();self.memo.insert(key,result);result
    }
    fn cell(&mut self,cell:&TableCell)->Result<Option<Number>,E>{
        self.step()?;
        // Multiple paragraphs/runs form one numeric value. Notes and child tables do not
        // accidentally enter the arithmetic. Multiple calculation fields are ambiguous.
        let mut text=String::new();let mut field=None;
        for (j,p) in cell.paragraphs.iter().enumerate(){
            if j>0{text.push(' ');}for (i,node) in p.content.iter().enumerate(){match node {
                Inline::Text(s)|Inline::StyledText{text:s,..}=>text.push_str(s),
                Inline::Formula(f)=>{if field.is_some(){return Err(E::Value);}field=Some(self.field(p.id,i,f)?);},
                Inline::NoteReference(_)=>(),_=>return Ok(None),
            }}
        }
        if let Some(n)=field{return if text.trim().is_empty(){Ok(Some(n))}else{Err(E::Value)};}
        match number::parse(&text){Ok(n)=>Ok(Some(n)),Err(E::Value)=>Ok(None),Err(e)=>Err(e)}
    }
    fn reference(&mut self,p:Id,start:&str,end:Option<&str>)->Result<Vec<Number>,E>{
        let (id,row,col)=self.d.table_location(p).ok_or(E::Reference)?;
        let table=self.d.table(id).ok_or(E::Reference)?;let own=table.cell(row,col).ok_or(E::Reference)?.id;
        let (r1,c1,r2,c2,positional)=match (start,end){
            ("ABOVE",None)=>(0,col,row.saturating_sub(1),col,true),
            ("BELOW",None)=>(row+1,col,table.rows.len()-1,col,true),
            ("LEFT",None)=>(row,0,row,col.saturating_sub(1),true),
            ("RIGHT",None)=>(row,col+1,row,table.columns.len()-1,true),
            ("R",None)=>(row,0,row,table.columns.len()-1,false),
            ("C",None)=>(0,col,table.rows.len()-1,col,false),
            _=>{let (r,c)=address(start,table)?;let (r2,c2)=if let Some(end)=end{address(end,table)?}else{(r,c)};if r>r2||c>c2{return Err(E::Reference);}(r,c,r2,c2,false)},
        };
        if positional&&((start=="ABOVE"&&row==0)||(start=="LEFT"&&col==0)||r1>r2||c1>c2){return Ok(Vec::new());}
        let many=positional||end.is_some()||start=="R"||start=="C";
        let mut seen=BTreeSet::new();let mut out=Vec::new();
        for r in r1..=r2 {for c in c1..=c2 {
            self.step()?;if positional&&r<usize::from(table.header_rows){continue;}
            let Some(cell)=table.cell(r,c)else{continue;};if !seen.insert(cell.id){continue;}
            if cell.id==own{if many{continue;}else{return Err(E::Cycle);}}
            if self.root_cell==Some(cell.id){return Err(E::Cycle);}
            if let Some(n)=self.cell(cell)?{out.push(n);}
        }}
        Ok(out)
    }
}
fn address(name:&str,t:&Table)->Result<(usize,usize),E>{
    let pair=if let Some((r,c))=name.strip_prefix('R').and_then(|s|s.split_once('C')){
        (r.parse::<usize>().map_err(|_|E::Reference)?,c.parse::<usize>().map_err(|_|E::Reference)?)
    }else{
        let at=name.bytes().position(|b|b.is_ascii_digit()).ok_or(E::Reference)?;
        let column=name[..at].bytes().try_fold(0usize,|n,b|n.checked_mul(26).and_then(|n|n.checked_add(usize::from(b-b'A'+1))).ok_or(E::Reference))?;
        (name[at..].parse::<usize>().map_err(|_|E::Reference)?,column)
    };
    let row=pair.0.checked_sub(1).ok_or(E::Reference)?;let column=pair.1.checked_sub(1).ok_or(E::Reference)?;
    if row>=t.rows.len()||column>=t.columns.len()||t.cell(row,column).is_none(){return Err(E::Reference);}Ok((row,column))
}
