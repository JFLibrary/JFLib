// Thin document/table controls. Model operations, geometry and property editing are shared.
use crate::*;
use doc_core::{Id, TableChange};
#[path="table_clipboard.rs"] mod clipboard;
pub(crate) struct TablesUi {
    pub(crate) open: Shared<i32>,
    pub(crate) controller: doc_view::TableController,
    rows: i32,
    columns: i32,
    properties: Option<doc_view::TableProperties>,
    merge: Option<(Id, usize, usize)>,
    target: Id,
    error: bool,
    style_open: Shared<i32>,
    style_editor: Option<doc_view::TableStyleEditor>,
    style_apply: Option<Id>,
    border_open: Shared<i32>,
    border_editor: Option<doc_view::CellBorderEditor>,
    border_target: Id,
    position_open: Shared<i32>,
    position_editor: Option<doc_view::TablePositionEditor>,
    position_target: Id,
}
impl Default for TablesUi {
    fn default() -> Self {
        Self {
            open: Shared::new(0),
            controller: Default::default(),
            rows: 3,
            columns: 3,
            properties: None,
            merge: None,
            target: 0,
            error: false,
            style_open: Shared::new(0), style_editor: None, style_apply: None,
            border_open:Shared::new(0),border_editor:None,border_target:0,
            position_open:Shared::new(0),position_editor:None,position_target:0,
        }
    }
}
impl Word {
    pub(crate) fn apply_page_edit(&mut self,ui:&mut Ui,edit:Edit)->Option<Id>{
        let check=self.editor.document().tables.iter().any(|t|t.position.is_some()) || matches!(&edit,Edit::ChangeTable{change:TableChange::Position(Some(_)),..})
            || matches!(&edit,Edit::ReplaceStoryFragment{fragment,..} if fragment.document.tables.iter().any(|t|t.position.is_some()));
        if !check{return self.apply(edit);}
        let fonts=match self.preview.prepare_fonts(&mut ui.text){Ok(f)=>f,Err(e)=>{self.message=format!("Schriften nicht verfügbar: {e:?}");return None;}};
        match doc_view::apply_paginated(&mut self.editor,edit,&mut ui.text,&fonts){
            Ok(id)=>{self.repair_story_owner();self.message=String::from("Änderung übernommen");Some(id)},
            Err(e)=>{self.message=format!("Änderung nicht übernommen: Tabellenposition, Kopfzeilen oder Text passen nicht in den verfügbaren Seitenbereich ({e:?}).");None}
        }
    }
    pub(crate) fn open_table_insert(&mut self) {
        if self.active_note.is_some() {
            self.message = String::from("Tabellen können im Haupttext und in Tabellenzellen eingefügt werden.");
            return;
        }
        self.tables_ui.properties = None;
        self.tables_ui.merge = None;
        self.tables_ui.error = false;
        self.tables_ui.open.set(1);
    }
    pub(crate) fn table_dialog(&mut self, ui: &mut Ui) {
        if ui.dialog_begin(3991,"Tabellenposition",&self.tables_ui.position_open){
            ui.next_size((ui.client_size().0-100).clamp(240,440),(ui.client_size().1-180).clamp(160,530));
            ui.scroll_begin(3992,SC_V|BOX_V);
            if let Some(e)=&mut self.tables_ui.position_editor{e.show(ui,4000);}
            ui.scroll_end();
            if self.tables_ui.error{ui.text(TS_CAPTION|TX_WRAP,"Position und Textabstände müssen einen nutzbaren Seitenbereich lassen. Kopfzeilen müssen zusammen mit Tabelleninhalt auf eine Seite passen.");}
            if ui.dialog_buttons(Some("Übernehmen"),None,Some("Abbrechen"))==DLG_PRIMARY {
                let change=self.tables_ui.position_editor.as_ref().map(|e|e.change());
                if change.and_then(|change|self.apply_page_edit(ui,Edit::ChangeTable{table:self.tables_ui.position_target,change})).is_none(){self.tables_ui.error=true;self.tables_ui.position_open.set(1);}else{self.rich_request_focus();ui.repass();}
            }
        }
        ui.dialog_end();
        if ui.dialog_begin(
            3900,
            if self.tables_ui.merge.is_some() {
                "Zellen verbinden"
            } else if self.tables_ui.properties.is_some() {
                "Tabelleneigenschaften"
            } else {
                "Tabelle einfügen"
            },
            &self.tables_ui.open,
        ) {
            if let Some(p) = &mut self.tables_ui.properties {
                ui.next_size((ui.client_size().0-100).clamp(240,440),(ui.client_size().1-180).clamp(120,300));
                ui.scroll_begin(3904,SC_V|BOX_V);
                p.show(ui, 3905);
                ui.scroll_end();
            } else {
                if self.tables_ui.merge.is_some() {
                    ui.text(TS_CAPTION|TX_WRAP,"Bereich ab der ausgewählten Zelle nach unten und rechts. Alle Inhalte bleiben erhalten.");
                }
                ui.text(TS_CAPTION, "Zeilen");
                ui.next_size(-1, 0);
                ui.number_box(3901, &mut self.tables_ui.rows, &Range::new(1, 256, 1, 0));
                ui.text(TS_CAPTION, "Spalten");
                ui.next_size(-1, 0);
                ui.number_box(3902, &mut self.tables_ui.columns, &Range::new(1, 32, 1, 0));
            }
            if self.tables_ui.error {
                ui.text(TS_CAPTION|TX_WRAP,"Der Bereich muss ganze Zellen umfassen. Maße und Einzüge müssen auf die Seite passen. Eine genaue Zeilenhöhe muss größer als 0 sein.");
            }
            if ui.dialog_buttons(Some("Übernehmen"), None, Some("Abbrechen")) == DLG_PRIMARY {
                let selected = (self.tables_ui.properties.is_some()
                    || self.tables_ui.merge.is_some())
                .then(|| self.selected_id());
                let result = if let Some((table, row, column)) = self.tables_ui.merge {
                    self.apply_page_edit(ui,Edit::ChangeTable {
                        table,
                        change: TableChange::Merge {
                            row,
                            column,
                            rows: self.tables_ui.rows as usize,
                            columns: self.tables_ui.columns as usize,
                        },
                    })
                } else if let Some(p) = &self.tables_ui.properties {
                    self.apply_page_edit(ui,Edit::ChangeTable {
                        table: self.tables_ui.target,
                        change: p.change(),
                    })
                } else {
                    let paragraph = self.selected_id();
                    let offset = self.table_insertion_offset();
                    self.apply_page_edit(ui,Edit::InsertTable {
                        paragraph,
                        offset,
                        rows: self.tables_ui.rows as usize,
                        columns: self.tables_ui.columns as usize,
                    })
                };
                if let Some(id) = result {
                    if let Some(p) = selected.or_else(|| {
                        self.editor
                            .document()
                            .table(id)
                            .and_then(|t| t.cell(0, 0))
                            .map(|c| c.paragraphs[0].id)
                    }) {
                        self.select_paragraph(p);
                        self.rich_request_focus();
                    }
                    ui.repass();
                } else {
                    self.tables_ui.error = true;
                    self.tables_ui.open.set(1);
                }
            }
        }
        ui.dialog_end();
        if ui.dialog_begin(3930,"Tabellenformatvorlage",&self.tables_ui.style_open) {
            ui.next_size((ui.client_size().0-100).clamp(240,464),(ui.client_size().1-180).clamp(160,470));
            ui.scroll_begin(3931,SC_V|BOX_V);
            if let Some(editor)=&mut self.tables_ui.style_editor {editor.show(ui,3940);}
            ui.scroll_end();
            if self.tables_ui.error {ui.text(TS_CAPTION|TX_WRAP,"Name, RGB-Werte und Basis prüfen. Vorlagennamen müssen eindeutig sein; Vorlagen dürfen keinen Kreis bilden.");}
            if ui.dialog_buttons(Some("Übernehmen"),None,Some("Abbrechen"))==DLG_PRIMARY {
                let definition=self.tables_ui.style_editor.as_ref().and_then(|e|e.definition().ok());
                let result=definition.and_then(|style|self.apply_page_edit(ui,Edit::SetTableStyle{style,table:self.tables_ui.style_apply}));
                if result.is_none(){self.tables_ui.error=true;self.tables_ui.style_open.set(1);}else{self.rich_request_focus();ui.repass();}
            }
        }
        ui.dialog_end();
        if ui.dialog_begin(3969,"Zellrahmen",&self.tables_ui.border_open){
            if let Some(editor)=&mut self.tables_ui.border_editor{editor.show(ui,3970);}
            if self.tables_ui.error{ui.text(TS_CAPTION|TX_WRAP,"Bitte Linienbreite (0,25–12 pt) und sechsstellige RGB-Farben prüfen.");}
            if ui.dialog_buttons(Some("Übernehmen"),None,Some("Abbrechen"))==DLG_PRIMARY {
                let change=self.tables_ui.border_editor.as_ref().and_then(|e|e.change().ok());
                if change.and_then(|change|self.apply_page_edit(ui,Edit::ChangeTable{table:self.tables_ui.border_target,change})).is_none(){self.tables_ui.error=true;self.tables_ui.border_open.set(1);}else{self.rich_request_focus();ui.repass();}
            }
        }
        ui.dialog_end();
    }
    pub(crate) fn table_tools(&mut self, ui: &mut Ui) {
        let Some((id, row, col)) = self.editor.document().table_location(self.selected_id()) else {
            return;
        };
        let compact = ui.client_size().0 < 800;
        ui.next_size(-1, if compact { 194 } else { 142 });
        ui.box_begin(3910, BOX_V | BOX_FIXED | pad(3), 0);
        ui.next_size(-1, 26);
        ui.box_begin(3919, BOX_H, 4);
        ui.text(
            TS_CAPTION,
            &format!("Tabelle{} · Zeile {} · Spalte {}",if self.editor.document().table_parent(id).is_some(){format!(" (Ebene {})",self.editor.document().table_depth(id).unwrap_or(1))}else{String::new()},row+1,col+1),
        );
        let mut change = None;
        for (widget, label, op) in [
            (3911, "Zeile davor", TableChange::InsertRow(row)),
            (3912, "Zeile danach", TableChange::InsertRow(row + 1)),
            (3913, "Spalte davor", TableChange::InsertColumn(col)),
            (3914, "Spalte danach", TableChange::InsertColumn(col + 1)),
            (3915, "Zeile löschen", TableChange::RemoveRow(row)),
            (3916, "Spalte löschen", TableChange::RemoveColumn(col)),
            (3917, "Tabelle löschen", TableChange::Delete),
        ] {
            if widget == 3915 {
                ui.box_end();
                ui.next_size(-1, 26);
                ui.box_begin(3920, BOX_H, 4);
            }
            if ui.button(widget, label) & EV_ACT != 0 {
                change = Some(op);
            }
        }
        if ui.button(3918, "Eigenschaften …") & EV_ACT != 0 {
            self.tables_ui.merge = None;
            self.tables_ui.properties =
                doc_view::TableProperties::new(self.editor.document().table(id).unwrap(), row, col);
            self.tables_ui.target = id;
            self.tables_ui.error = false;
            self.tables_ui.open.set(1);
        }
        if compact {
            ui.box_end();
            ui.next_size(-1, 26);
            ui.box_begin(3924, BOX_H, 4);
        }
        if ui.button(3921, "Verbinden …") & EV_ACT != 0 {
            let t = self.editor.document().table(id).unwrap();
            self.tables_ui.rows = (t.rows.len() - row).min(2) as i32;
            self.tables_ui.columns = (t.columns.len() - col).min(2) as i32;
            self.tables_ui.properties = None;
            self.tables_ui.merge = Some((id, row, col));
            self.tables_ui.error = false;
            self.tables_ui.open.set(1);
        }
        let cell = self
            .editor
            .document()
            .table(id)
            .unwrap()
            .cell(row, col)
            .unwrap();
        if cell.row_span == 1 && cell.column_span == 1 {
            ui.next_flags(HB_DISABLED);
        }
        if ui.button(3922, "Teilen") & EV_ACT != 0 {
            change = Some(TableChange::Split { row, column: col });
        }
        let mut fit = 0;
        ui.next_size(150, 0);
        ui.combobox(
            3923,
            &mut fit,
            &[
                "Breite anpassen …",
                "An Inhalt",
                "An Seitenbreite",
                "Gleich verteilen",
            ],
        );
        if fit > 0 {
            match self.table_fit_change(
                ui,
                id,
                match fit {
                    1 => doc_layout::TableFit::Contents,
                    2 => doc_layout::TableFit::Page,
                    _ => doc_layout::TableFit::Equal,
                },
            ) {
                Ok(op) => change = Some(op),
                Err(e) => self.message = e,
            }
        }
        ui.box_end();
        ui.next_size(-1,26);ui.box_begin(3925,BOX_H,4);
        ui.text(TS_CAPTION,"Formatvorlage");
        let d=self.editor.document();let t=d.table(id).unwrap();let ids=d.table_style_ids();
        let mut names=alloc::vec![String::from("Ohne Vorlage")];names.extend(ids.iter().map(|id|d.table_style(id).unwrap().name));
        let mut selected=t.style.as_ref().and_then(|id|ids.iter().position(|v|v==id)).map_or(0,|i|i as i32+1);
        ui.next_size(if compact {180}else{220},0);
        if ui.combobox(3926,&mut selected,&names.iter().map(String::as_str).collect::<Vec<_>>())&EV_ACT!=0 {
            change=Some(TableChange::Style{style:if selected==0 {None}else{Some(ids[selected as usize-1].clone())},look:t.look});
        }
        let mut header=i32::from(t.header_rows);ui.text(TS_CAPTION,"Kopfzeilen");ui.next_size(68,0);
        if ui.number_box(3927,&mut header,&Range::new(0,t.rows.len() as i32,1,0))&EV_ACT!=0 {change=Some(TableChange::HeaderRows(header as u16));}
        ui.tooltip("Erste Zeilen auf Folgeseiten wiederholen. 0 schaltet die Wiederholung aus.");
        for (widget,label,create) in [(3928,"Neue Vorlage …",true),(3929,"Ändern …",false)] {
            if !create && t.style.is_none(){ui.next_flags(HB_DISABLED);}
            if ui.button(widget,label)&EV_ACT!=0 {
                let mut style=d.table_style(t.style.as_deref().unwrap_or("jfTableBlue")).unwrap();
                if create {
                    let mut n=1;
                    while d.table_style(&format!("jfTableCustom{n}")).is_some() || d.table_style_ids().iter().any(|id|d.table_style(id).unwrap().name==format!("Eigene Tabelle {n}")){n+=1;}
                    style=doc_core::TableStyle::derived(&format!("jfTableCustom{n}"),&format!("Eigene Tabelle {n}"),&style.id);
                }
                self.tables_ui.style_editor=Some(doc_view::TableStyleEditor::new(style,d));self.tables_ui.style_apply=create.then_some(id);
                self.tables_ui.error=false;self.tables_ui.style_open.set(1);
            }
        }
        ui.box_end();ui.next_size(-1,26);ui.box_begin(3932,BOX_H,4);
        let mut look=t.look;let mut changed=false;
        for (widget,label,v) in [(3933,"Erste Zeile",&mut look.first_row),(3934,"Ergebniszeile",&mut look.last_row),(3935,"Erste Spalte",&mut look.first_column),(3936,"Letzte Spalte",&mut look.last_column),(3937,"Zeilenbänder",&mut look.banded_rows),(3938,"Spaltenbänder",&mut look.banded_columns)] {
            if compact && widget==3936 {ui.box_end();ui.next_size(-1,26);ui.box_begin(3939,BOX_H,4);}
            let mut b=i32::from(*v);if ui.checkbox(widget,label,&mut b)&EV_ACT!=0 {*v=b!=0;changed=true;}
        }
        if changed {change=Some(TableChange::Look(look));}
        ui.box_end();
        ui.next_size(-1,26);ui.box_begin(3964,BOX_H,4);
        if d.table_parent(id).is_some(){ui.next_flags(HB_DISABLED);}
        if ui.button(3990,"Position …")&EV_ACT!=0 {
            self.tables_ui.position_editor=Some(doc_view::TablePositionEditor::new(t));self.tables_ui.position_target=id;
            self.tables_ui.error=false;self.tables_ui.position_open.set(1);
        }
        if ui.button(3965,"Zellrahmen …")&EV_ACT!=0 {
            self.tables_ui.border_editor=Some(doc_view::CellBorderEditor::new(row,col,t.cell(row,col).unwrap().borders));
            self.tables_ui.border_target=id;self.tables_ui.error=false;self.tables_ui.border_open.set(1);
        }
        let copy=ui.button(3966,"Tabelle kopieren")&EV_ACT!=0;
        let cut=ui.button(3967,"Tabelle ausschneiden")&EV_ACT!=0;
        let after=ui.button(3968,if d.table_parent(id).is_some(){"Zur äußeren Zelle"}else{"Nach der Tabelle"})&EV_ACT!=0;
        let after_id=t.before;
        let formula=ui.button(4090,"Formel …")&EV_ACT!=0;
        ui.box_end();
        ui.box_end();
        if copy || cut {self.table_clipboard(ui,id,cut);ui.repass();}
        if formula{self.open_formula(self.rich_field_target().filter(|(p,i)|matches!(self.editor.document().paragraph(*p).and_then(|p|p.content.get(*i)),Some(Inline::Formula(_)))));}
        if after {self.rich_local_focus(ui,after_id,0,0);ui.repass();}
        if let Some(change) = change {
            let before = self.editor.document().table(id).unwrap().before;
            if self
                .apply_page_edit(ui,Edit::ChangeTable { table: id, change })
                .is_some()
            {
                let target = self
                    .editor
                    .document()
                    .table(id)
                    .map(|t| {
                        t.cell(row.min(t.rows.len() - 1), col.min(t.columns.len() - 1))
                            .or_else(||t.anchors().next().map(|(_,_,c)|c)).unwrap()
                            .paragraphs[0]
                            .id
                    })
                    .unwrap_or(before);
                self.rich_local_focus(ui, target, 0, 0);
                ui.repass();
            }
        }
    }
    pub(crate) fn table_next_cell(&mut self, ui: &mut Ui, back: bool) -> bool {
        let Some((id, r, c)) = self.editor.document().table_location(self.selected_id()) else {
            return false;
        };
        let table = self.editor.document().table(id).unwrap();
        let anchors: Vec<_> = table
            .anchors()
            .map(|(r, c, cell)| (r, c, cell.paragraphs[0].id))
            .collect();
        let at = anchors
            .iter()
            .position(|(row, col, _)| *row == r && *col == c)
            .unwrap();
        if back && at == 0 {
            return true;
        }
        let target = if back {
            anchors[at - 1].2
        } else if at + 1 < anchors.len() {
            anchors[at + 1].2
        } else {
            let row = table.rows.len();
            if self
                .apply_page_edit(ui,Edit::ChangeTable {
                    table: id,
                    change: TableChange::InsertRow(row),
                })
                .is_none()
            {
                return true;
            }
            self.editor
                .document()
                .table(id)
                .unwrap()
                .cell(row, 0)
                .unwrap()
                .paragraphs[0]
                .id
        };
        self.rich_local_focus(ui, target, 0, 0);
        ui.repass();
        true
    }
    pub(crate) fn table_fit_change(
        &mut self,
        ui: &mut Ui,
        id: Id,
        mode: doc_layout::TableFit,
    ) -> Result<TableChange, String> {
        let fonts = self
            .preview
            .prepare_fonts(&mut ui.text)
            .map_err(|e| format!("Schriften nicht verfügbar: {e:?}"))?;
        let widths = doc_view::fit_table(self.editor.document(), id, mode, &mut ui.text, &fonts)
            .map_err(|e| format!("Breite konnte nicht berechnet werden: {e:?}"))?;
        Ok(TableChange::Columns(widths))
    }
}
