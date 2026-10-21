//! Shared document table-style editor; applies through the ordinary Editor transaction.
use alloc::{format, string::String, vec::Vec};
use doc_core::{Document, Error, TableRegion, TableRegionKind, TableStyle, TextColor};
use ui::*;

struct RegionDraft { original:TableRegion, fill:String, ink:String, bold:i32 }
fn hex(c:Option<u32>)->String{c.map(|v|format!("{v:06X}")).unwrap_or_default()}
fn rgb(s:&str)->Result<Option<u32>,Error>{
    if s.is_empty(){return Ok(None);}
    if s.len()!=6{return Err(Error::Invalid);}Ok(Some(u32::from_str_radix(s,16).map_err(|_|Error::Invalid)?))
}
impl RegionDraft {
    fn new(r:TableRegion)->Self{Self{original:r,fill:hex(r.fill),ink:hex(match r.text.color{Some(TextColor::Rgb(v))=>Some(v),_=>None}),bold:match r.text.bold{None=>0,Some(false)=>1,Some(true)=>2}}}
    fn value(&self)->Result<TableRegion,Error>{
        let mut r=self.original;r.fill=rgb(&self.fill)?;
        r.text.color=if self.ink.is_empty(){self.original.text.color.filter(|c|matches!(c,TextColor::Auto))}else{rgb(&self.ink)?.map(TextColor::Rgb)};
        r.text.bold=match self.bold{1=>Some(false),2=>Some(true),_=>None};Ok(r)
    }
}
/// Draft preserves every unexposed text property and distinguishes inheritance from explicit values.
pub struct TableStyleEditor {
    original:TableStyle, name:String, regions:[RegionDraft;13], border:String, width:i32,
    inherit_border:i32, row_size:i32, column_size:i32, parents:Vec<(String,String,(u16,u32))>,
    base:i32, tab:i32, area:i32,
}
impl TableStyleEditor {
    pub fn new(style:TableStyle,document:&Document)->Self{
        let parents:Vec<_>=document.table_style_ids().into_iter().filter(|id|!document.table_style_depends_on(id,&style.id))
            .map(|id|{let s=document.resolved_table_style(&id).unwrap();(id,s.name,(s.border_width,s.border_color))}).collect();
        let base=style.based_on.as_ref().and_then(|id|parents.iter().position(|(key,_,_)|key==id)).map_or(0,|i|i as i32+1);
        let effective=document.resolved_table_style(&style.id).ok().or_else(||style.based_on.as_ref().and_then(|id|document.resolved_table_style(id).ok())).unwrap_or_else(||style.clone());
        Self {name:style.name.clone(),regions:core::array::from_fn(|i|RegionDraft::new(style.region_format(TableRegionKind::ALL[i]))),
            border:hex(Some(if style.border_inherit{effective.border_color}else{style.border_color})),
            width:i32::from(if style.border_inherit{effective.border_width}else{style.border_width}),
            inherit_border:i32::from(style.border_inherit),row_size:i32::from(style.row_band_size.unwrap_or(0)),
            column_size:i32::from(style.column_band_size.unwrap_or(0)),parents,base,tab:0,area:0,original:style}
    }
    fn refresh_inherited_border(&mut self){
        let (width,color)=if self.base==0{(4,0x808080)}else{self.parents[(self.base-1) as usize].2};
        self.width=i32::from(width);self.border=hex(Some(color));
    }
    /// Reserves IDs id..id+23. First eight IDs retain their earlier meanings.
    pub fn show(&mut self,ui:&mut Ui,id:u32){
        ui.tabs(id+20,&mut self.tab,&["Grundlagen","Vererbung","Bereiche"]);
        if self.tab==0 {
            ui.text(TS_CAPTION,"Name");ui.next_size(-1,0);ui.textbox(id,None,&mut self.name,129);
            for (offset,label,index) in [(1,"Tabellenfläche · RGB",0),(2,"Erste Zeile · RGB",1),(3,"Zeilenband · RGB",5)] {
                ui.text(TS_CAPTION,label);ui.next_size(-1,0);ui.textbox(id+offset,None,&mut self.regions[index].fill,7);
            }
            ui.text(TS_CAPTION,"Schrift der ersten Zeile · RGB");ui.next_size(-1,0);ui.textbox(id+4,None,&mut self.regions[1].ink,7);
            if self.inherit_border!=0 {ui.next_flags(HB_DISABLED);}
            ui.text(TS_CAPTION,"Rahmen · RGB");ui.next_size(-1,0);if self.inherit_border!=0 {ui.next_flags(HB_DISABLED);}
            ui.textbox(id+5,None,&mut self.border,7);
            ui.text(TS_CAPTION,"Rahmenstärke · Achtelpunkt (0 = ohne)");ui.next_size(-1,0);if self.inherit_border!=0 {ui.next_flags(HB_DISABLED);}
            ui.number_box(id+6,&mut self.width,&Range::new(0,80,1,0));
            ui.text(TS_CAPTION,"Erste Zeile · Schriftgewicht");ui.next_size(-1,0);ui.combobox(id+7,&mut self.regions[1].bold,&["Geerbt","Normal","Fett"]);
        } else if self.tab==1 {
            let previous_base=self.base;let previous_inherit=self.inherit_border;
            ui.text(TS_CAPTION,"Basiert auf");let mut names=alloc::vec!["Keine Basisvorlage"];names.extend(self.parents.iter().map(|(_,name,_)|name.as_str()));
            ui.next_size(-1,0);ui.combobox(id+9,&mut self.base,&names);
            ui.checkbox(id+10,"Rahmen aus Basisvorlage übernehmen",&mut self.inherit_border);
            if self.inherit_border!=0 && (self.base!=previous_base || previous_inherit==0){self.refresh_inherited_border();}
            for (offset,label,value,max) in [(11,"Zeilen pro Band · 0 = geerbt",&mut self.row_size,256),(12,"Spalten pro Band · 0 = geerbt",&mut self.column_size,32)] {
                ui.text(TS_CAPTION,label);ui.next_size(-1,0);ui.number_box(id+offset,value,&Range::new(0,max,1,0));
            }
            if ui.button(id+13,"Alle Formatüberschreibungen zurücksetzen")&EV_ACT!=0 {
                self.regions=core::array::from_fn(|_|RegionDraft::new(TableRegion::default()));self.inherit_border=1;self.row_size=0;self.column_size=0;self.refresh_inherited_border();
            }
            ui.text(TS_CAPTION|TX_WRAP,"Änderungen an der Basis wirken auf diese Vorlage und ihre Tabellen. Eigene Werte haben Vorrang. Zurücksetzen erhält Name und Basisbeziehung.");
        } else {
            ui.text(TS_CAPTION,"Tabellenbereich");ui.next_size(-1,0);
            ui.combobox(id+14,&mut self.area,&["Gesamte Tabelle","Erste Zeile","Ergebniszeile","Erste Spalte","Letzte Spalte","Zeilenband 1","Zeilenband 2","Spaltenband 1","Spaltenband 2","Oben links","Oben rechts","Unten links","Unten rechts"]);
            let r=&mut self.regions[self.area as usize];
            for (offset,label,value) in [(15,"Fläche · RGB",&mut r.fill),(16,"Schrift · RGB",&mut r.ink)] {
                ui.text(TS_CAPTION,label);ui.next_size(-1,0);ui.textbox(id+offset,None,value,7);
            }
            ui.text(TS_CAPTION,"Schriftgewicht");ui.next_size(-1,0);ui.combobox(id+17,&mut r.bold,&["Geerbt","Normal","Fett"]);
            if ui.button(id+18,"Bereich zurücksetzen")&EV_ACT!=0 {*r=RegionDraft::new(TableRegion::default());}
            ui.text(TS_CAPTION|TX_WRAP,"Leere Werte erben. FFFFFF setzt ausdrücklich Weiß. Eckzellen gelten bei aktivierter erster/letzter Zeile und erster/letzter Spalte; Spaltenbänder überlagern Zeilenbänder.");
        }
    }
    pub fn definition(&self)->Result<TableStyle,Error>{
        let mut s=self.original.clone();s.name=self.name.clone();
        s.based_on=if self.base==0{None}else{Some(self.parents[(self.base-1) as usize].0.clone())};
        s.border_inherit=self.inherit_border!=0;
        s.border_color=if s.border_inherit{0x808080}else{rgb(&self.border)?.ok_or(Error::Invalid)?};
        s.border_width=if s.border_inherit{4}else{self.width as u16};
        s.row_band_size=if self.row_size==0{None}else{Some(self.row_size as u16)};
        s.column_band_size=if self.column_size==0{None}else{Some(self.column_size as u16)};
        for (i,kind) in TableRegionKind::ALL.iter().enumerate(){*s.region_format_mut(*kind)=self.regions[i].value()?;}
        s.validate()?;Ok(s)
    }
}
