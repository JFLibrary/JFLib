// Thin jfWord adapters for the shared ruler gestures and exact tab-stop editor.
use crate::Word;
use doc_core::{ParagraphChange, TabAlignment, TabStops};
use ui::*;

pub(crate) struct Rulers {
    pub(crate) horizontal: doc_view::RulerController,
    pub(crate) vertical: doc_view::RulerController,
    pub(crate) alignment: i32,
    pub(crate) open: Shared<i32>,
    draft: TabStops,
    editor: doc_view::TabEditor,
    error: bool,
}
impl Default for Rulers {
    fn default() -> Self { Self { horizontal: Default::default(), vertical: Default::default(), alignment: 0,
        open: Shared::new(0), draft: Default::default(), editor: doc_view::TabEditor::new(), error: false } }
}
impl Rulers {
    pub(crate) fn alignment(&self) -> TabAlignment { [TabAlignment::Left, TabAlignment::Center, TabAlignment::Right, TabAlignment::Decimal, TabAlignment::Bar][self.alignment as usize] }
}
impl Word {
    pub(crate) fn open_tabs(&mut self) {
        self.rulers.draft = self.editor.document().resolve_paragraph(self.formatting_paragraph()).tabs;
        self.rulers.editor.reset(); self.rulers.error = false; self.rulers.open.set(1);
    }
    pub(crate) fn tab_dialog(&mut self, ui: &mut Ui) {
        if ui.dialog_begin(3800, "Tabstopps", &self.rulers.open) {
            self.rulers.editor.show(ui, 3801, &mut self.rulers.draft, false);
            ui.next_size(-1, 36);
            ui.text(TS_CAPTION | TX_WRAP, if self.rulers.error { "Tabstopps konnten nicht übernommen werden. Höchstens 64 Deklarationen je Absatz." }
                else { "Gilt für die ausgewählten Absätze. Alle löschen entfernt auch deren geerbte Tabstopps." });
            if ui.dialog_buttons(Some("Übernehmen"), None, Some("Abbrechen")) == DLG_PRIMARY {
                if self.paragraph_change(ui, ParagraphChange::Tabs(self.rulers.draft)).is_none() { self.rulers.error = true; self.rulers.open.set(1); }
            }
        }
        ui.dialog_end();
    }
}
