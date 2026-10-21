//! Shared native/browser table positioning controls. Document lengths are shown in points.
use doc_core::{Table,TableChange,TablePosition,TableHorizontal as H,TableVertical as V};
use ui::*;
pub struct TablePositionEditor {floating:i32,horizontal:i32,vertical:i32,x:f64,y:f64,distance:[f64;4],overlap:i32}
impl TablePositionEditor {
    pub fn new(t:&Table)->Self {
        let p=t.position.unwrap_or_default();
        Self{floating:i32::from(t.position.is_some()),horizontal:match p.horizontal{H::Page=>0,H::Margin=>1,H::Column=>2},vertical:match p.vertical{V::Page=>0,V::Margin=>1,V::Paragraph=>2},x:p.x as f64/20.0,y:p.y as f64/20.0,distance:p.distance.map(|v|v as f64/20.0),overlap:i32::from(p.overlap)}
    }
    /// Reserves ten controls. Position is relative to the selected page, margin or anchor.
    pub fn show(&mut self,ui:&mut Ui,id:u32){
        ui.next_size(-1,0);ui.combobox(id,&mut self.floating,&["Im Textfluss","Schwebend · Text umfließt die Tabelle"]);
        if self.floating==0{return;}
        ui.text(TS_CAPTION,"Horizontaler Bezug");ui.next_size(-1,0);
        ui.combobox(id+1,&mut self.horizontal,&["Seitenkante","Seitenrand","Textspalte"]);
        ui.text(TS_CAPTION,"Position von links (pt)");ui.next_size(-1,0);
        ui.number_box_f(id+2,&mut self.x,&RangeF::new(-1584.0,1584.0,0.05,2,0));
        ui.text(TS_CAPTION,"Vertikaler Bezug");ui.next_size(-1,0);
        ui.combobox(id+3,&mut self.vertical,&["Seitenkante","Seitenrand","Ankerabsatz"]);
        ui.text(TS_CAPTION,"Position von oben (pt)");ui.next_size(-1,0);
        ui.number_box_f(id+4,&mut self.y,&RangeF::new(0.0,1584.0,0.05,2,0));
        for (i,label) in ["Textabstand oben (pt)","Textabstand rechts (pt)","Textabstand unten (pt)","Textabstand links (pt)"].iter().enumerate(){
            ui.text(TS_CAPTION,label);ui.next_size(-1,0);ui.number_box_f(id+5+i as u32,&mut self.distance[i],&RangeF::new(0.0,1584.0,0.05,2,0));
        }
        ui.checkbox(id+9,"Überlappung mit anderen Tabellen zulassen",&mut self.overlap);
    }
    pub fn change(&self)->TableChange{
        fn twips(v:f64)->i32{let n=v*20.0;(n+if n<0.0{-0.5}else{0.5}) as i32}
        TableChange::Position((self.floating!=0).then(||TablePosition{horizontal:match self.horizontal{0=>H::Page,2=>H::Column,_=>H::Margin},vertical:match self.vertical{0=>V::Page,1=>V::Margin,_=>V::Paragraph},x:twips(self.x),y:twips(self.y),distance:self.distance.map(|v|twips(v) as u16),overlap:self.overlap!=0}))
    }
}
