use super::ribbon_ui::*;
use crate::*;

impl Word {
    pub(super) fn references_ribbon(&mut self, ui: &mut Ui) {
        let contents = 80 + ui.ribbon_button_width("Text hinzufügen", IC_OFFICE_ADD_TEXT, ST_MENU);
        let notes = 68
            + [
                ("Endnote einfügen", IC_OFFICE_ENDNOTE, 0),
                ("Nächste Fußnote", IC_OFFICE_NEXT_FOOTNOTE, ST_MENU),
                ("Noten anzeigen", IC_OFFICE_SHOW_NOTES, 0),
            ]
            .iter()
            .map(|(label, icon, flags)| ui.ribbon_button_width(label, *icon, *flags))
            .max()
            .unwrap_or(0);
        let captions =
            90 + ui.ribbon_button_width("Abbildungsverzeichnis", IC_OFFICE_TABLE_OF_FIGURES, 0);
        let groups = [
            RibbonGroup::new(1300, "Inhaltsverzeichnis", contents, contents),
            RibbonGroup::new(1310, "Fußnoten", notes, notes),
            RibbonGroup::new(1320, "Recherche", 78, 78),
            RibbonGroup::new(1330, "Zitate und Literaturverzeichnis", 234, 234),
            RibbonGroup::new(1340, "Beschriftungen", captions, captions),
            RibbonGroup::new(1350, "Index", 168, 168),
            RibbonGroup::new(1360, "Rechtsgrundlagenverzeichnis", 116, 116),
        ];
        let layout = ui.ribbon_begin(101, &groups);
        for (index, group) in groups.iter().enumerate() {
            let Some(mode) = ui.ribbon_group_begin(&layout, index, group) else {
                continue;
            };
            row(ui);
            match index {
                0 => {
                    ui.next_size(78, 66);
                    unavailable(ui, 3000, "Inhalts-\nverzeichnis", IC_OFFICE_TOC, true, true);
                    stack(ui, -1);
                    unavailable(ui, 3001, "Text hinzufügen", IC_OFFICE_ADD_TEXT, false, true);
                    unavailable(
                        ui,
                        3002,
                        "Aktualisieren",
                        IC_OFFICE_UPDATE_TABLE,
                        false,
                        false,
                    );
                    ui.box_end();
                }
                1 => {
                    ui.next_size(66, 66);
                    if large(
                        ui,
                        320,
                        "Fußnote\neinfügen",
                        IC_OFFICE_FOOTNOTE,
                        if self.active_note.is_none() {
                            0
                        } else {
                            ST_DISABLED
                        },
                    ) {
                        ui.cmd_run(FOOTNOTE, 0);
                    }
                    stack(ui, -1);
                    action(
                        ui,
                        321,
                        ENDNOTE,
                        "Endnote einfügen",
                        IC_OFFICE_ENDNOTE,
                        self.active_note.is_none(),
                    );
                    unavailable(
                        ui,
                        3003,
                        "Nächste Fußnote",
                        IC_OFFICE_NEXT_FOOTNOTE,
                        false,
                        true,
                    );
                    action(
                        ui,
                        322,
                        NOTE_RETURN,
                        "Noten anzeigen",
                        IC_OFFICE_SHOW_NOTES,
                        self.active_note.is_some(),
                    );
                    ui.box_end();
                }
                2 => {
                    unavailable(ui, 3004, "Recherche", IC_OFFICE_RESEARCHER, true, false);
                }
                3 => {
                    ui.next_size(64, 66);
                    if large(
                        ui,
                        312,
                        "Zitat\neinfügen",
                        IC_OFFICE_CITATION,
                        if self.active_source().is_some() {
                            0
                        } else {
                            ST_DISABLED
                        },
                    ) {
                        ui.cmd_run(CITE, 0);
                    }
                    stack(ui, -1);
                    ui.next_size(-1, 22);
                    ui.ribbon_menu_button(
                        323,
                        "Quellen verwalten",
                        IC_OFFICE_SOURCES,
                        ST_ALIGN_START,
                    );
                    self.source_ribbon_menu(ui);
                    ui.next_size(-1, 22);
                    ui.ribbon_button(
                        3005,
                        "Stil: Autor/Jahr",
                        IC_OFFICE_FONTS,
                        ST_ALIGN_START | ST_DISABLED | ST_MENU,
                    );
                    action(
                        ui,
                        313,
                        BIBLIOGRAPHY,
                        "Literaturverzeichnis",
                        IC_OFFICE_BIBLIOGRAPHY,
                        self.active_note.is_none(),
                    );
                    ui.box_end();
                }
                4 => {
                    ui.next_size(88, 66);
                    unavailable(
                        ui,
                        3006,
                        "Beschriftung\neinfügen",
                        IC_OFFICE_CAPTION,
                        true,
                        false,
                    );
                    stack(ui, -1);
                    unavailable(
                        ui,
                        3007,
                        "Abbildungsverzeichnis",
                        IC_OFFICE_TABLE_OF_FIGURES,
                        false,
                        false,
                    );
                    unavailable(
                        ui,
                        3008,
                        "Aktualisieren",
                        IC_OFFICE_UPDATE_TABLE,
                        false,
                        false,
                    );
                    unavailable(ui, 3009, "Querverweis", IC_OFFICE_CROSS_REF, false, false);
                    ui.box_end();
                }
                5 => {
                    ui.next_size(64, 66);
                    unavailable(
                        ui,
                        3010,
                        "Eintrag\nfestlegen",
                        IC_OFFICE_MARK_ENTRY,
                        true,
                        false,
                    );
                    stack(ui, -1);
                    unavailable(ui, 3011, "Index einfügen", IC_OFFICE_INDEX, false, false);
                    unavailable(
                        ui,
                        3012,
                        "Aktualisieren",
                        IC_OFFICE_UPDATE_TABLE,
                        false,
                        false,
                    );
                    ui.box_end();
                }
                _ => {
                    ui.next_size(74, 66);
                    unavailable(
                        ui,
                        3013,
                        "Zitat\nmarkieren",
                        IC_OFFICE_MARK_CITATION,
                        true,
                        false,
                    );
                    stack(ui, -1);
                    icon(
                        ui,
                        3014,
                        "Rechtsgrundlagenverzeichnis",
                        IC_OFFICE_TABLE_OF_AUTHORITIES,
                        ST_DISABLED,
                    );
                    ui.box_end();
                }
            }
            ui.box_end();
            ui.ribbon_group_end(mode, group.label);
        }
        ui.ribbon_end();
    }

    fn source_ribbon_menu(&mut self, ui: &mut Ui) {
        if ui.flyout_begin(323, 0) {
            action(ui, 3015, SOURCES, "Quellenbereich", IC_OFFICE_SOURCES, true);
            action(ui, 309, SOURCE_NEW, "Neue Quelle", IC_ADD, true);
            action(
                ui,
                310,
                SOURCE_EDIT,
                "Quelle bearbeiten",
                IC_EDIT,
                self.active_source().is_some(),
            );
            action(
                ui,
                311,
                SOURCE_REMOVE,
                "Quelle entfernen",
                IC_DELETE,
                self.active_source().is_some(),
            );
            ui.menu_sep();
            action(
                ui,
                318,
                SOURCE_LIBRARY,
                "Quellenbibliothek",
                IC_FOLDER,
                true,
            );
            action(
                ui,
                316,
                SOURCE_MERGE,
                "Quellen zusammenführen",
                IC_LINK,
                self.active_source().is_some() && self.editor.document().sources.len() >= 2,
            );
            action(ui, 317, SOURCE_AUDIT, "Quellen prüfen", IC_CHECK_MARK, true);
            action(
                ui,
                327,
                SOURCE_IMPORT,
                "Quellen importieren",
                IC_OPEN_FILE,
                true,
            );
            action(
                ui,
                328,
                SOURCE_EXPORT,
                "Quellen exportieren",
                IC_SAVE_AS,
                !self.editor.document().sources.is_empty(),
            );
            ui.menu_sep();
            for (id, name) in [
                (1410, "Feld bearbeiten"),
                (1411, "Nächstes Feld"),
                (1412, "Vorheriges Feld"),
            ] {
                ui.next_size(-1, 22);
                let disabled = self.structured_edit
                    || self.preview_mode
                    || (id == 1410 && self.rich_field_target().is_none());
                if ui.ribbon_button(
                    id,
                    name,
                    IC_OFFICE_QUICK_PARTS,
                    ST_ALIGN_START | if disabled { ST_DISABLED } else { 0 },
                ) & EV_ACT
                    != 0
                {
                    ui.popup_close();
                    if id == 1410 {
                        self.field_edit(ui);
                    } else {
                        self.rich_field_next(ui, id == 1411);
                    }
                }
            }
        }
        ui.flyout_end();
    }

    pub(super) fn page_ribbon(&mut self, ui: &mut Ui) {
        let groups = [
            RibbonGroup::new(1300, "Seite einrichten", 382, 382),
            RibbonGroup::new(1310, "Absatz", 190, 190),
            RibbonGroup::new(1320, "Anordnen", 320, 320),
        ];
        let layout = ui.ribbon_begin(101, &groups);
        for (index, group) in groups.iter().enumerate() {
            let Some(mode) = ui.ribbon_group_begin(&layout, index, group) else {
                continue;
            };
            row(ui);
            match index {
                0 => {
                    for (id, name, glyph) in [
                        (36, "Seiten-\nränder", IC_OFFICE_MARGINS),
                        (3020, "Aus-\nrichtung", IC_OFFICE_ORIENTATION),
                        (3021, "Größe", IC_OFFICE_SIZE),
                    ] {
                        ui.next_size(64, 66);
                        if large(ui, id, name, glyph, ST_MENU) {
                            self.open_page_format();
                        }
                    }
                    ui.next_size(58, 66);
                    unavailable(ui, 3022, "Spalten", IC_OFFICE_COLUMNS, true, true);
                    stack(ui, -1);
                    ui.next_size(-1, 22);
                    ui.ribbon_menu_button(3023, "Umbrüche", IC_OFFICE_BREAKS, ST_ALIGN_START);
                    if ui.flyout_begin(3023, 0) {
                        let active = self.editor.document().resolve_paragraph(self.formatting_paragraph()).page_break_before;
                        ui.next_size(220, 24);
                        if ui.ribbon_button(
                            44,
                            "Seitenwechsel vor Absatz",
                            IC_OFFICE_PAGE_BREAK,
                            if active { ST_CHECKED } else { 0 },
                        ) & EV_ACT
                            != 0
                        {
                            self.paragraph_change(
                                ui,
                                doc_core::ParagraphChange::PageBreakBefore(!active),
                            );
                            ui.popup_close();
                        }
                    }
                    ui.flyout_end();
                    unavailable(
                        ui,
                        3024,
                        "Zeilennummern",
                        IC_OFFICE_LINE_NUMBERS,
                        false,
                        true,
                    );
                    unavailable(
                        ui,
                        3025,
                        "Silbentrennung",
                        IC_OFFICE_HYPHENATION,
                        false,
                        true,
                    );
                    ui.box_end();
                }
                1 => {
                    let mut format = self.editor.document().resolve_paragraph(self.formatting_paragraph());
                    let mut changed = 0;
                    for column in 0..2 {
                        stack(ui, 94);
                        ui.next_size(-1, 18);
                        ui.text(TS_CAPTION, if column == 0 { "Einzug" } else { "Abstand" });
                        for line in 0..2 {
                            ui.next_size(-1, 24);
                            ui.box_begin(0, BOX_H, 2);
                            ui.next_size(39, 22);
                            ui.text(
                                TS_CAPTION,
                                match (column, line) {
                                    (0, 0) => "Links:",
                                    (0, _) => "Rechts:",
                                    (_, 0) => "Vor:",
                                    _ => "Nach:",
                                },
                            );
                            let value = match (column, line) {
                                (0, 0) => &mut format.left,
                                (0, _) => &mut format.right,
                                (_, 0) => &mut format.before,
                                _ => &mut format.after,
                            };
                            let mut points = f64::from(*value) / 20.0;
                            ui.next_size(-1, 22);
                            if ui.number_box_f(
                                3030 + column * 2 + line,
                                &mut points,
                                &RangeF::new(0.0, 144.0, 1.0, 1, 0),
                            ) & EV_ACT
                                != 0
                            {
                                *value = (points * 20.0) as u16;
                                changed |= match (column, line) { (0, 0) => 16, (0, _) => 32, (_, 0) => 2, _ => 4 };
                            }
                            ui.tooltip("Wert in Punkt");
                            ui.box_end();
                        }
                        ui.box_end();
                    }
                    if changed != 0 {
                        self.paragraph_change(ui, doc_core::ParagraphChange::Properties(format.select(changed)));
                    }
                }
                _ => {
                    ui.next_size(62, 66);
                    unavailable(ui, 3040, "Position", IC_OFFICE_POSITION, true, true);
                    ui.next_size(72, 66);
                    unavailable(ui, 3041, "Text-\numbruch", IC_OFFICE_WRAP_TEXT, true, true);
                    stack(ui, 145);
                    unavailable(
                        ui,
                        3042,
                        "Eine Ebene vor",
                        IC_OFFICE_BRING_FORWARD,
                        false,
                        false,
                    );
                    unavailable(
                        ui,
                        3043,
                        "Eine Ebene zurück",
                        IC_OFFICE_SEND_BACKWARD,
                        false,
                        false,
                    );
                    unavailable(
                        ui,
                        3044,
                        "Auswahlbereich",
                        IC_OFFICE_SELECTION_PANE,
                        false,
                        false,
                    );
                    ui.box_end();
                    stack(ui, -1);
                    for (id, name, glyph) in [
                        (3045, "Ausrichten", IC_OFFICE_ALIGN),
                        (3046, "Gruppieren", IC_OFFICE_GROUP),
                        (3047, "Drehen", IC_OFFICE_ROTATE),
                    ] {
                        icon(ui, id, name, glyph, ST_DISABLED);
                    }
                    ui.box_end();
                }
            }
            ui.box_end();
            if index < 2 {
                if ui.ribbon_group_end_launcher(
                    mode,
                    group.label,
                    3050 + index as u32,
                    if index == 0 {
                        "Seite einrichten"
                    } else {
                        "Absatz"
                    },
                    0,
                ) & EV_ACT
                    != 0
                {
                    if index == 0 {
                        self.open_page_format();
                    } else {
                        self.open_paragraph_format();
                    }
                }
            } else {
                ui.ribbon_group_end(mode, group.label);
            }
        }
        ui.ribbon_end();
    }

    pub(super) fn view_ribbon(&mut self, ui: &mut Ui) {
        let groups = [
            RibbonGroup::new(1300, "Ansichten", 265, 265),
            RibbonGroup::new(1310, "Lesen", 146, 146),
            RibbonGroup::new(1320, "Seitenbewegung", 133, 133),
            RibbonGroup::new(1330, "Anzeigen", 128, 128),
            RibbonGroup::new(1340, "Zoom", 270, 270),
            RibbonGroup::new(1350, "Farbschema", 118, 118),
            RibbonGroup::new(1360, "Fenster", 184, 184),
        ];
        let layout = ui.ribbon_begin(101, &groups);
        for (index, group) in groups.iter().enumerate() {
            let Some(mode) = ui.ribbon_group_begin(&layout, index, group) else {
                continue;
            };
            row(ui);
            match index {
                0 => {
                    for (id, label, glyph, preview) in [
                        (45, "Lese-\nmodus", IC_OFFICE_READ_MODE, true),
                        (46, "Druck-\nlayout", IC_OFFICE_PRINT_LAYOUT, false),
                    ] {
                        if large(
                            ui,
                            id,
                            label,
                            glyph,
                            if self.preview_mode == preview {
                                ST_CHECKED
                            } else {
                                0
                            },
                        ) {
                            self.preview_mode = preview;
                        }
                    }
                    unavailable(ui, 3060, "Web-\nlayout", IC_OFFICE_WEB_LAYOUT, true, false);
                    stack(ui, -1);
                    ui.next_size(-1, 22);
                    if ui.ribbon_button(
                        54,
                        "Gliederung",
                        IC_OFFICE_OUTLINE,
                        ST_ALIGN_START | if self.structured_edit { ST_CHECKED } else { 0 },
                    ) & EV_ACT
                        != 0
                    {
                        self.structured_edit = !self.structured_edit;
                        self.widget_ids.clear();
                        self.rich = Default::default();
                        self.formatting.selection = None;
                        self.formatting.pending_selection = None;
                        self.preview_mode = false;
                        let field = self.text_id(self.selected_id(), 0);
                        ui.focus(field);
                    }
                    unavailable(ui, 3061, "Entwurf", IC_OFFICE_DRAFT, false, false);
                    ui.box_end();
                }
                1 => {
                    if large(
                        ui,
                        2830,
                        "Fokus",
                        IC_OFFICE_FOCUS,
                        if self.focus_mode { ST_CHECKED } else { 0 },
                    ) {
                        ui.cmd_run(VIEW_FOCUS, 0);
                    }
                    unavailable(
                        ui,
                        3062,
                        "Lese-\nhilfe",
                        IC_OFFICE_IMMERSIVE,
                        true,
                        false,
                    );
                }
                2 => {
                    unavailable(ui, 3063, "Vertikal", IC_OFFICE_VERTICAL, true, false);
                    unavailable(
                        ui,
                        3064,
                        "Seite an\nSeite",
                        IC_OFFICE_SIDE_TO_SIDE,
                        true,
                        false,
                    );
                }
                3 => {
                    stack(ui, -1);
                    ui.next_size(-1, 22);
                    ui.next_flags(ST_DISABLED);
                    let mut ruler = 1;
                    ui.checkbox(3065, "Lineal", &mut ruler);
                    ui.next_size(-1, 22);
                    ui.next_flags(ST_DISABLED);
                    let mut grid = 0;
                    ui.checkbox(3066, "Gitternetzlinien", &mut grid);
                    ui.next_size(-1, 22);
                    let mut navigation = i32::from(self.show_navigation);
                    if ui.checkbox(49, "Navigation", &mut navigation) & EV_ACT != 0 {
                        self.show_navigation = navigation != 0;
                    }
                    ui.box_end();
                }
                4 => {
                    stack(ui, 56);
                    ui.next_size(-1, 30);
                    ui.ribbon_button(3067, "Zoom", IC_OFFICE_ZOOM, ST_ICON_ONLY | ST_DISABLED);
                    ui.next_size(-1, 24);
                    let mut percent = self.preview.zoom_percent();
                    if ui.number_box(2834, &mut percent, &Range::new(10, 500, 1, 0)) & EV_ACT != 0 {
                        self.preview.set_zoom(percent);
                    }
                    ui.box_end();
                    if large(ui, 2833, "100 %", IC_OFFICE_ZOOM100, 0) {
                        self.preview.set_zoom(100);
                    }
                    stack(ui, -1);
                    unavailable(ui, 3068, "Eine Seite", IC_OFFICE_ONE_PAGE, false, false);
                    unavailable(
                        ui,
                        3069,
                        "Mehrere Seiten",
                        IC_OFFICE_MULTIPLE_PAGES,
                        false,
                        false,
                    );
                    ui.next_size(-1, 22);
                    if ui.ribbon_button(
                        2832,
                        "Seitenbreite",
                        IC_OFFICE_PAGE_WIDTH,
                        ST_ALIGN_START
                            | if self.preview.is_fit_width() {
                                ST_CHECKED
                            } else {
                                0
                            },
                    ) & EV_ACT
                        != 0
                    {
                        self.preview.set_fit_width();
                    }
                    ui.box_end();
                }
                5 => {
                    if large(ui, 3070, "Modus\nwechseln", IC_OFFICE_DARK_MODE, 0) {
                        ui.theme_set(u32::from(!ui.theme_dark()));
                    }
                    unavailable(ui, 3071, "Design", IC_OFFICE_INTERFACE_THEME, true, true);
                }
                _ => {
                    unavailable(
                        ui,
                        3072,
                        "Neues\nFenster",
                        IC_OFFICE_NEW_WINDOW,
                        true,
                        false,
                    );
                    unavailable(
                        ui,
                        3073,
                        "Alle\nanordnen",
                        IC_OFFICE_ARRANGE_ALL,
                        true,
                        false,
                    );
                    unavailable(ui, 3074, "Teilen", IC_OFFICE_SPLIT, true, false);
                }
            }
            ui.box_end();
            ui.ribbon_group_end(mode, group.label);
        }
        ui.ribbon_end();
    }
}
