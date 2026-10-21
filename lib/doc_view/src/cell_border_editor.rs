//! Independent edge editor shared by native and browser hosts.
use alloc::{format,string::String};
use doc_core::{BorderSide,BorderStyle,CellBorder,CellBorders,Error,TableChange};
use ui::*;
struct EdgeDraft {style:i32,width:f64,color:String}
pub struct CellBorderEditor {row:usize,column:usize,edges:[EdgeDraft;4]}
impl CellBorderEditor {
    pub fn new(row:usize,column:usize,borders:CellBorders)->Self {
        Self{row,column,edges:borders.map(|b|EdgeDraft{style:b.map_or(0,|b|match b.style{BorderStyle::Nil=>1,BorderStyle::None=>2,BorderStyle::Single=>3,BorderStyle::Double=>4,BorderStyle::Dotted=>5,BorderStyle::Dashed=>6}),width:b.filter(|b|b.visible()).map_or(0.5,|b|f64::from(b.width)/8.0),color:format!("{:06X}",b.map_or(0x808080,|b|b.color))})}
    }
    /// Reserves 16 controls: four rows with style, eighth-point width and RGB color.
    pub fn show(&mut self,ui:&mut Ui,id:u32){
        ui.text(TS_CAPTION|TX_WRAP,"Jede Seite kann die Tabelle erben oder einen eigenen Rahmen erhalten. Ausblenden unterdrückt auch den angrenzenden Rahmen.");
        ui.text(TS_CAPTION,"Linienart · Breite in Punkt · Farbe (RGB)");
        for side in BorderSide::ALL {
            let i=side as usize;let e=&mut self.edges[i];
            ui.text(TS_CAPTION,["Oben","Rechts","Unten","Links"][i]);
            ui.next_size(-1,30);ui.box_begin(id+i as u32*4,BOX_H,5);
            ui.next_size(185,0);ui.combobox(id+i as u32*4+1,&mut e.style,&["Aus Tabelle erben","Ausblenden","Tabellenstandard","Durchgezogen","Doppelt","Gepunktet","Gestrichelt"]);
            if e.style<3{ui.next_flags(HB_DISABLED);}ui.next_size(85,0);ui.number_box_f(id+i as u32*4+2,&mut e.width,&RangeF::new(0.25,12.0,0.125,3,0));ui.tooltip("Linienbreite in Punkt");
            if e.style<3{ui.next_flags(HB_DISABLED);}ui.next_size(100,0);ui.textbox(id+i as u32*4+3,None,&mut e.color,7);ui.tooltip("Sechsstelliger RGB-Farbwert");
            ui.box_end();
        }
    }
    pub fn change(&self)->Result<TableChange,Error>{
        let mut borders=[None;4];
        for (i,e) in self.edges.iter().enumerate(){
            if e.style==0{continue;}
            let style=match e.style{1=>BorderStyle::Nil,2=>BorderStyle::None,3=>BorderStyle::Single,4=>BorderStyle::Double,5=>BorderStyle::Dotted,6=>BorderStyle::Dashed,_=>return Err(Error::Invalid)};
            let visible=e.style>=3;
            let color=if visible{if e.color.len()!=6{return Err(Error::Invalid);}u32::from_str_radix(&e.color,16).map_err(|_|Error::Invalid)?}else{0};
            let border=CellBorder{style,width:if visible{(e.width*8.0+0.5) as u16}else{0},color};border.validate()?;borders[i]=Some(border);
        }
        Ok(TableChange::Borders{row:self.row,column:self.column,borders})
    }
}
