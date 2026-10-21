//! Thin shared calculation-dialog adapter; formulas and UI controls live in JFLib.
use crate::*;
#[derive(Default)]
pub(crate) struct FormulasUi {
    pub(crate) open:Shared<i32>,
    editor:Option<doc_view::FormulaEditor>,
    error:String,
}
impl Word {
    pub(crate) fn open_formula(&mut self,target:Option<(u64,usize)>){
        let d=self.editor.document();let paragraph=target.map_or(self.selected_id(),|v|v.0);
        let Some(p)=d.paragraph(paragraph)else{return;};
        let caret=target.map(|(_,i)|(i,0)).or_else(||self.formatting.selection.filter(|(id,_,_,_)|*id==paragraph).map(|(_,i,_,at)|(i,at)))
            .or_else(||p.content.iter().enumerate().rev().find_map(|(i,n)|n.text().map(|s|(i,s.len()))));
        if let Some((inline,offset))=caret {
            self.formulas_ui.editor=doc_view::FormulaEditor::new(d,self.editor.revision(),paragraph,inline,offset);
            if self.formulas_ui.editor.is_some(){self.formulas_ui.error.clear();self.formulas_ui.open.set(1);return;}
        }
        self.message=String::from("Bitte eine Textstelle oder ein Formelfeld auswählen.");
    }
    pub(crate) fn formula_dialog(&mut self,ui:&mut Ui){
        if ui.dialog_begin(4100,"Tabellenformel",&self.formulas_ui.open){
            ui.next_size((ui.client_size().0-100).clamp(260,480),(ui.client_size().1-160).clamp(180,390));
            ui.scroll_begin(4101,SC_V|BOX_V);
            if let Some(e)=&mut self.formulas_ui.editor{e.show(ui,4110,self.editor.document());}
            if !self.formulas_ui.error.is_empty(){ui.text(TS_CAPTION|TX_WRAP,&self.formulas_ui.error);}
            ui.scroll_end();
            if ui.dialog_buttons(Some("Übernehmen"),None,Some("Abbrechen"))==DLG_PRIMARY {
                let e=self.formulas_ui.editor.as_ref().unwrap();let p=e.paragraph();
                if e.revision!=self.editor.revision(){self.formulas_ui.error=String::from("Das Dokument wurde inzwischen geändert. Bitte den Formeldialog erneut öffnen.");self.formulas_ui.open.set(1);}
                else{match e.edit(){
                    Ok(edit)=>{if self.apply_page_edit(ui,edit).is_some(){self.widget_ids.clear();self.rich_local_focus(ui,p,0,0);ui.repass();self.message=String::from("Formel übernommen · Ergebnisse aktualisiert");}else{self.formulas_ui.error=self.message.clone();self.formulas_ui.open.set(1);}},
                    Err(error)=>{self.formulas_ui.error=format!("Formel nicht übernommen: {}. Bitte Syntax und Zahlenformat prüfen.",error.code());self.formulas_ui.open.set(1);},
                }}
            }
        }
        ui.dialog_end();
    }
}
