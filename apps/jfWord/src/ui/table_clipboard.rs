//! Application adapter: every codec and structural edit lives in the shared libraries.
use crate::*;
use doc_core::Id;
impl Word {
    pub(crate) fn table_clipboard(&mut self,ui:&mut Ui,id:Id,cut:bool)->bool {
        let d=self.editor.document();
        let Ok(fragment)=d.copy_table(id)else{self.message=String::from("Tabelle nicht gefunden.");return false;};
        let before=fragment.document.tables[0].before;
        let encoded=doc_io::encode_document_fragment(&fragment);
        let exchange=doc_io::clipboard_display_fragment(&fragment,d).and_then(|display|Ok((display.plain_text(),doc_io::clipboard_rtf_encode(&display)?,doc_io::clipboard_html_encode(&display)?)));
        let(Ok(encoded),Ok((text,rtf,html)))=(encoded,exchange)else{self.message=String::from("Die Tabelle konnte nicht vollständig kopiert werden.");return false;};
        if ui.clip_set_many(&[(CLIP_TEXT,text.as_bytes()),(CLIP_DOCX,&encoded),(CLIP_RTF,&rtf),(CLIP_HTML,&html)])!=0 {
            self.message=String::from("Die Zwischenablage ist gerade nicht verfügbar. Die Tabelle bleibt unverändert.");return false;
        }
        if cut {
            if self.apply(Edit::ChangeTable{table:id,change:doc_core::TableChange::Delete}).is_none(){return false;}
            self.rich_local_focus(ui,before,0,0);
        }
        self.message=String::from(if cut{"Tabelle ausgeschnitten · im Haupttext einfügen"}else{"Tabelle vollständig kopiert · mit Rahmen, Vorlagen und Quellen"});true
    }
}
