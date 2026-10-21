//! Shared formula editor: native and browser controls create the same model transaction.
use alloc::{format,string::String};
use doc_core::{Document,Edit,Formula,FormulaError,Id,Inline};
use ui::*;
/// Calculation draft with an explicit revision and stable field/caret target.
pub struct FormulaEditor {
    pub revision:u64,
    paragraph:Id,
    inline:usize,
    offset:usize,
    replace:bool,
    expression:String,
    picture:String,
}
const PICTURES:&[&str]=&["","0","0.00","#,##0","#,##0.00","0%","0.00%"];
impl FormulaEditor {
    /// Start at a text caret or edit a calculation field. No source data is changed.
    pub fn new(d:&Document,revision:u64,paragraph:Id,inline:usize,offset:usize)->Option<Self>{
        let node=d.paragraph(paragraph)?.content.get(inline)?;
        let (expression,picture,replace)=if let Inline::Formula(f)=node{
            (f.expression.clone(),f.number_format.clone(),true)
        }else{
            let text=node.text()?;if !text.is_char_boundary(offset){return None;}
            let above=d.table_location(paragraph).is_some_and(|(id,r,_)|r>usize::from(d.table(id).unwrap().header_rows));
            (String::from(if above{"=SUM(ABOVE)"}else{"=SUM(LEFT)"}),String::new(),false)
        };
        Some(Self{revision,paragraph,inline,offset,replace,expression,picture})
    }
    /// Five widget IDs. Preview always uses the current containing table and shared arithmetic.
    pub fn show(&mut self,ui:&mut Ui,id:u32,d:&Document){
        ui.text(TS_CAPTION,"Formel");ui.next_size(-1,0);
        ui.textbox(id,Some("=SUM(ABOVE)"),&mut self.expression,2049);
        ui.text(TS_CAPTION,"Zahlenformat");ui.next_size(-1,0);
        ui.textbox(id+1,Some("Automatisch"),&mut self.picture,33);
        let mut preset=0;ui.next_size(-1,0);
        if ui.combobox(id+4,&mut preset,&["Zahlenformat wählen …","0","0.00","#,##0","#,##0.00","0%","0.00%"])&EV_ACT!=0&&preset>0{self.picture=String::from(PICTURES[preset as usize]);}
        let preview=self.formula().map(|f|d.evaluate_formula(self.paragraph,&f));
        let shown=match preview{Ok(v)=>v.text,Err(e)=>String::from(e.code())};
        ui.text(TS_BODY_STRONG,&format!("Ergebnis: {shown}"));
        ui.text(TS_CAPTION|TX_WRAP,"Zellbezüge: A1 oder R1C1; Bereiche: A1:B3. Funktionen: SUM, AVERAGE, COUNT, MIN, MAX, PRODUCT, ROUND, IF. Richtungen: ABOVE, BELOW, LEFT, RIGHT.");
        if ui.button(id+2,"Summe darüber")&EV_ACT!=0{self.expression=String::from("=SUM(ABOVE)");}
        if ui.button(id+3,"Summe links")&EV_ACT!=0{self.expression=String::from("=SUM(LEFT)");}
        ui.text(TS_CAPTION|TX_SECONDARY|TX_WRAP,"Berechnungen aktualisieren sich mit dem Dokument. Zellbezüge verwenden die aktuellen Zeilen und Spalten dieser Tabelle.");
    }
    fn formula(&self)->Result<Formula,FormulaError>{Formula::new(&self.expression,&self.picture)}
    /// A complete insert/update edit. The caller checks `revision` before applying it.
    pub fn edit(&self)->Result<Edit,FormulaError>{
        let formula=self.formula()?;
        Ok(if self.replace{Edit::SetFormula{paragraph:self.paragraph,inline:self.inline,formula}}else{Edit::InsertFormula{paragraph:self.paragraph,inline:self.inline,offset:self.offset,formula}})
    }
    /// Anchor paragraph for focus restoration after committing.
    pub fn paragraph(&self)->Id{self.paragraph}
}
