use super::ribbon_ui::*;
use crate::*;
use doc_core::Alignment;

impl Word {
    pub(super) fn start_ribbon(&mut self, ui: &mut Ui) {
        let clipboard = 56 + ui.ribbon_button_width("Ausschneiden", IC_OFFICE_CUT, 0);
        let editing = ui.ribbon_button_width("Markieren", IC_OFFICE_SELECT, ST_MENU);
        let groups = [
            RibbonGroup::new(1350, "Zwischenablage", clipboard, clipboard),
            RibbonGroup::new(1300, "Schriftart", 314, 314),
            RibbonGroup::new(1310, "Absatz", 220, 220),
            RibbonGroup::new(1320, "Formatvorlagen", 474, 474),
            RibbonGroup::new(1340, "Bearbeiten", editing, editing),
            RibbonGroup::new(1360, "Sprache", 52, 52),
            RibbonGroup::new(1370, "Korrektur", 62, 62),
        ];
        let layout = ui.ribbon_begin(101, &groups);
        for (index, group) in groups.iter().enumerate() {
            let Some(mode) = ui.ribbon_group_begin(&layout, index, group) else {
                continue;
            };
            match index {
                0 => self.clipboard_ribbon(ui),
                1 => self.ribbon_font(ui),
                2 => self.ribbon_paragraph(ui),
                3 => self.ribbon_styles(ui),
                4 => {
                    action(ui, 35, SEARCH, "Suchen", IC_OFFICE_FIND, true);
                    action(ui, 2890, SEARCH, "Ersetzen", IC_OFFICE_REPLACE, true);
                    ui.next_size(-1, 22);
                    ui.ribbon_menu_button(2891, "Markieren", IC_OFFICE_SELECT, ST_ALIGN_START);
                    if ui.flyout_begin(2891, 0) {
                        action(ui, ADD, ADD, "Absatz hinzufügen", IC_ADD, true);
                        action(
                            ui,
                            REMOVE,
                            REMOVE,
                            "Absatz entfernen",
                            IC_DELETE,
                            self.current_paragraphs().len() > 1,
                        );
                    }
                    ui.flyout_end();
                }
                5 => unavailable(ui, 1364, "Diktieren", IC_OFFICE_DICTATE, true, true),
                _ => unavailable(ui, 1374, "Prüfen", IC_OFFICE_EDITOR, true, false),
            }
            if index <= 3 {
                if ui.ribbon_group_end_launcher(
                    mode,
                    group.label,
                    [2878, 2879, 2874, 2881][index],
                    [
                        "Zwischenablage öffnen",
                        "Schriftartdialog öffnen",
                        "Absatzdialog öffnen",
                        "Formatvorlagen öffnen",
                    ][index],
                    if index == 2 || index == 3 { 0 } else { ST_DISABLED },
                ) & EV_ACT
                    != 0
                {
                    if index == 3 { self.open_styles(); } else { self.open_paragraph_format(); }
                }
            } else {
                ui.ribbon_group_end(mode, group.label);
            }
        }
        ui.ribbon_end();
    }

    fn clipboard_ribbon(&mut self, ui: &mut Ui) {
        let flags = if self.structured_edit || self.preview_mode {
            ST_DISABLED
        } else {
            0
        };
        row(ui);
        ui.next_size(54, 66);
        if ui.ribbon_stacked_split_button(
            72,
            2870,
            "Einfügen",
            "Einfügeoptionen",
            IC_OFFICE_PASTE,
            flags,
        ) & EV_ACT
            != 0
        {
            self.rich_clipboard(ui, 72);
        }
        if ui.flyout_begin(2870, 0) {
            for (id, command, label) in [
                (2871, 72, "Mit Formatierung einfügen"),
                (2872, 73, "Nur Text einfügen"),
            ] {
                ui.next_flags(flags);
                ui.next_icon(IC_OFFICE_PASTE);
                if ui.menu_item(id, label) & EV_ACT != 0 {
                    self.rich_clipboard(ui, command);
                }
            }
            ui.menu_sep();
            for (id, command, label) in [
                (2875, FORMAT_COPY, "Format aufnehmen"),
                (2876, FORMAT_PASTE, "Format anwenden"),
                (2877, FORMAT_RESET, "Formatierung löschen"),
            ] {
                ui.next_flags(if self.rich_format_available(command) {
                    0
                } else {
                    ST_DISABLED
                });
                ui.next_icon(IC_OFFICE_PAINTER);
                if ui.menu_item(id, label) & EV_ACT != 0 {
                    ui.cmd_run(command, 0);
                }
            }
        }
        ui.flyout_end();
        stack(ui, -1);
        for (id, label, glyph) in [
            (71, "Ausschneiden", IC_OFFICE_CUT),
            (70, "Kopieren", IC_OFFICE_COPY),
        ] {
            ui.next_size(-1, 22);
            if ui.ribbon_button(id, label, glyph, flags | ST_ALIGN_START) & EV_ACT != 0 {
                self.rich_clipboard(ui, id);
            }
        }
        let command = if self.rich_format_available(FORMAT_PASTE) {
            FORMAT_PASTE
        } else {
            FORMAT_COPY
        };
        action(
            ui,
            2882,
            command,
            "Format",
            IC_OFFICE_PAINTER,
            self.rich_format_available(command),
        );
        ui.box_end();
        ui.box_end();
    }

    fn ribbon_font(&mut self, ui: &mut Ui) {
        let current = self.current_character_format();
        let common = self.current_character_common();
        ui.next_size(-1, 22);
        ui.box_begin(0, BOX_H, 2);
        ui.next_size(134, 22);
        let mut families = self.document_families(ui);
        if !families.iter().any(|f| f == current.font_family.as_str()) { families.push(String::from(current.font_family.as_str())); }
        let labels: Vec<_> = families.iter().map(String::as_str).collect();
        let mut family = labels.iter().position(|f| *f == current.font_family.as_str()).unwrap_or(0) as i32;
        if ui.combobox(2883, &mut family, &labels) & EV_ACT != 0 {
            if let Ok(name) = doc_core::FontFamily::new(labels[family as usize]) { self.character_family(ui, name); }
        }
        let substituted = self.preview.fonts().is_some_and(|f| f.substituted(current.font_family));
        ui.tooltip(if substituted { "Diese Schrift ist hier nicht verfügbar. Ersatzschrift wird angezeigt; der Name bleibt gespeichert." } else { "Schriftfamilie für die Textauswahl" });
        ui.next_size(56, 22);
        ui.box_begin(0, BOX_H | BOX_FIXED, 0);
        ui.next_size(42, 22);
        let mut points = f64::from(current.size_half_points) / 2.0;
        if ui.number_box_f(2880, &mut points, &RangeF::new(4.0, 144.0, 0.5, 1, 0)) & EV_ACT != 0 {
            self.character_format(ui, 0, Some((points * 2.0) as u16));
        }
        ui.tooltip("Schriftgröße in Punkt");
        let size_label = if current.size_half_points % 2 == 0 {
            format!("{} pt", current.size_half_points / 2)
        } else {
            format!("{},5 pt", current.size_half_points / 2)
        };
        ui.next_size(14, 22);
        ui.ribbon_menu_button(33, &size_label, IC_CHEV_DOWN, ST_ICON_ONLY);
        if ui.flyout_begin(33, 0) {
            for (index, size) in [8, 9, 10, 11, 12, 14, 16, 18, 24, 36, 48, 72]
                .into_iter()
                .enumerate()
            {
                if ui.menu_item(2930 + index as u32, &format!("{size} pt")) & EV_ACT != 0 {
                    self.character_format(ui, 0, Some(size * 2));
                }
            }
        }
        ui.flyout_end();
        ui.box_end();
        if icon(ui, 2884, "Schrift vergrößern", IC_OFFICE_GROW, 0) {
            self.character_format(ui, 0, Some((current.size_half_points + 2).min(288)));
        }
        if icon(ui, 2885, "Schrift verkleinern", IC_OFFICE_SHRINK, 0) {
            self.character_format(
                ui,
                0,
                Some(current.size_half_points.saturating_sub(2).max(8)),
            );
        }
        icon(
            ui,
            2886,
            "Groß-/Kleinschreibung",
            IC_OFFICE_CASE,
            ST_DISABLED | ST_MENU,
        );
        if icon(
            ui,
            2887,
            "Formatierung löschen",
            IC_OFFICE_CLEAR,
            if self.rich_format_available(FORMAT_RESET) {
                0
            } else {
                ST_DISABLED
            },
        ) {
            ui.cmd_run(FORMAT_RESET, 0);
        }
        ui.box_end();
        ui.spacer(3);
        ui.next_size(-1, 22);
        ui.box_begin(0, BOX_H, 1);
        for (id, label, glyph, checked) in [
            (30, "Fett", IC_OFFICE_BOLD, common.bold == Some(true)),
            (31, "Kursiv", IC_OFFICE_ITALIC, common.italic == Some(true)),
            (32, "Unterstrichen", IC_OFFICE_UNDERLINE, common.underline == Some(true)),
            (55, "Durchgestrichen", IC_OFFICE_STRIKE, common.strike == Some(true)),
        ] {
            if icon(ui, id, label, glyph, if checked { ST_CHECKED } else { 0 }) {
                self.character_format(ui, id, None);
            }
            if id == 32 {
                ui.next_size(10, 22);
                ui.ribbon_button(
                    2892,
                    "Unterstreichungsarten",
                    IC_CHEV_DOWN,
                    ST_ICON_ONLY | ST_DISABLED,
                );
            }
        }
        for (id, label, glyph, command, position) in [
            (2888, "Tiefgestellt", IC_OFFICE_SUBSCRIPT, FORMAT_SUBSCRIPT, doc_core::TextPositioning::Subscript),
            (2889, "Hochgestellt", IC_OFFICE_SUPERSCRIPT, FORMAT_SUPERSCRIPT, doc_core::TextPositioning::Superscript),
        ] {
            if icon(ui, id, label, glyph, if common.positioning == Some(position) { ST_CHECKED } else { 0 }) {
                self.character_format(ui, command, None);
            }
        }
        ui.spacer(4);
        icon(
            ui,
            2893,
            "Texteffekte",
            IC_OFFICE_EFFECTS,
            ST_MENU | ST_DISABLED,
        );
        ui.next_size(34, 22);
        ui.ribbon_menu_button(2894, "Texthervorhebung", IC_OFFICE_HIGHLIGHT, ST_ICON_ONLY);
        if ui.flyout_begin(2894, 0) {
            for (index, color) in doc_core::Highlight::ALL.into_iter().enumerate() {
                ui.next_flags(if common.highlight == Some(color) { ST_CHECKED } else { 0 });
                if ui.menu_item(3700 + index as u32, crate::editor_ui::HIGHLIGHT_LABELS[index]) & EV_ACT != 0 {
                    self.character_format(ui, 3700 + index as u32, None);
                }
            }
        }
        ui.flyout_end();
        ui.next_size(34, 22);
        ui.ribbon_menu_button(56, "Schriftfarbe", IC_OFFICE_FONTCOLOR, ST_ICON_ONLY);
        if ui.flyout_begin(56, 0) {
            for (index, name) in ["Automatisch", "Schwarz", "Rot", "Blau", "Petrol"]
                .iter()
                .enumerate()
            {
                if ui.menu_item(2920 + index as u32, name) & EV_ACT != 0 {
                    self.character_format(ui, 100 + index as u32, None);
                }
            }
        }
        ui.flyout_end();
        ui.box_end();
    }

    fn ribbon_paragraph(&mut self, ui: &mut Ui) {
        let format = self.editor.document().resolve_paragraph(self.formatting_paragraph());
        ui.next_size(-1, 22);
        ui.box_begin(0, BOX_H, 1);
        for (id, label, glyph) in [
            (2900, "Aufzählung", IC_OFFICE_BULLETS),
            (2901, "Nummerierung", IC_OFFICE_NUMBERING),
            (2902, "Liste mit mehreren Ebenen", IC_OFFICE_MULTILEVEL),
        ] {
            icon(ui, id, label, glyph, ST_DISABLED | ST_MENU);
        }
        ui.spacer(3);
        for (id, label, glyph, increase) in [
            (2903, "Einzug verkleinern", IC_OFFICE_OUTDENT, false),
            (2904, "Einzug vergrößern", IC_OFFICE_INDENT, true),
        ] {
            if icon(ui, id, label, glyph, 0) {
                let mut updated = format;
                updated.left = if increase {
                    updated.left.saturating_add(360)
                } else {
                    updated.left.saturating_sub(360)
                };
                self.paragraph_change(ui, doc_core::ParagraphChange::Properties(updated.select(16)));
            }
        }
        icon(ui, 2905, "Sortieren", IC_OFFICE_SORT, ST_DISABLED);
        icon(
            ui,
            2906,
            "Formatierungszeichen",
            IC_OFFICE_PILCROW,
            ST_DISABLED,
        );
        ui.box_end();
        ui.spacer(3);
        ui.next_size(-1, 22);
        ui.box_begin(0, BOX_H, 1);
        for (id, label, glyph, alignment) in [
            (40, "Linksbündig", IC_OFFICE_ALIGN_LEFT, Alignment::Left),
            (41, "Zentriert", IC_OFFICE_ALIGN_CENTER, Alignment::Center),
            (42, "Rechtsbündig", IC_OFFICE_ALIGN_RIGHT, Alignment::Right),
            (43, "Blocksatz", IC_OFFICE_JUSTIFY, Alignment::Justify),
        ] {
            if icon(
                ui,
                id,
                label,
                glyph,
                if format.alignment == alignment {
                    ST_CHECKED
                } else {
                    0
                },
            ) {
                self.paragraph_change(ui, doc_core::ParagraphChange::Alignment(alignment));
            }
        }
        ui.next_size(36, 22);
        ui.ribbon_menu_button(
            34,
            "Zeilen- und Absatzabstand",
            IC_OFFICE_LINE_SPACING,
            ST_ICON_ONLY,
        );
        if ui.flyout_begin(34, 0) {
            for (i, (label, line)) in [("1,0", 240), ("1,15", 276), ("1,5", 360), ("2,0", 480)]
                .into_iter()
                .enumerate()
            {
                if ui.menu_item(2910 + i as u32, label) & EV_ACT != 0 {
                    let mut updated = format;
                    updated.line = line;
                    self.paragraph_change(ui, doc_core::ParagraphChange::Properties(updated.select(8)));
                }
            }
            ui.menu_sep();
            action(ui, 2914, PARAGRAPH_SETUP, "Absatz …", IC_SLIDERS, true);
        }
        ui.flyout_end();
        icon(
            ui,
            2907,
            "Schattierung",
            IC_OFFICE_SHADING,
            ST_DISABLED | ST_MENU,
        );
        icon(ui, 2908, "Rahmen", IC_OFFICE_BORDERS, ST_DISABLED | ST_MENU);
        ui.box_end();
    }

    fn ribbon_styles(&mut self, ui: &mut Ui) {
        let selected = self.formatting_paragraph().style;
        ui.next_size(-1, 66);
        ui.ribbon_gallery_begin(0);
        let mut tiles = alloc::vec![
            (200, "Standard", TS_BODY, Some(ParagraphStyle::Normal)),
            (2899, "Kein Leerraum", TS_BODY, None),
            (
                202,
                "Überschrift 1",
                TS_SUBTITLE,
                Some(ParagraphStyle::Heading1),
            ),
            (201, "Titel", TS_TITLE, Some(ParagraphStyle::Title)),
            (
                203,
                "Überschrift 2",
                TS_SUBTITLE,
                Some(ParagraphStyle::Heading2),
            ),
            (2915, "Untertitel", TS_BODY, None),
        ];
        for (index, custom) in self.editor.document().styles.iter().filter(|s| matches!(s.id, ParagraphStyle::Named(_))).enumerate() {
            tiles.push((3600 + index as u32, custom.name.as_str(), TS_BODY, Some(custom.id)));
        }
        let tiles: Vec<_> = tiles.into_iter().map(|(id, label, typography, style)| (id, String::from(label), typography, style)).collect();
        let max_page = tiles.len().saturating_sub(1) / 6;
        self.styles_ui.gallery_page = self.styles_ui.gallery_page.min(max_page);
        let fonts = self.preview.prepare_fonts(&mut ui.text).ok();
        for (id, label, typography, style) in tiles.iter().skip(self.styles_ui.gallery_page * 6).take(6) {
            let (id, typography, style) = (*id, *typography, *style);

            ui.next_size(74, 60);
            if style.is_none() {
                ui.next_flags(ST_DISABLED);
            }
            let face = style.and_then(|id| fonts.as_ref().map(|fonts| {
                let f = self.editor.document().resolve_text(Default::default(), id);
                fonts.ensure(&mut ui.text, f.font_family);
                doc_view::ribbon_sample(f, fonts)
            }));
            let name = style.and_then(|id| self.editor.document().style_definition(id)).map_or_else(|| label.clone(), |s| s.name);
            if ui.ribbon_sample_tile(id, &name, "AaBbCcDd", typography, style == Some(selected), face) & EV_ACT
                != 0
            {
                if let Some(style) = style {
                    self.paragraph_change(ui, doc_core::ParagraphChange::Style(style));
                }
            }
        }
        ui.next_size(14, 60);
        ui.box_begin(0, BOX_V, 0);
        for (id, icon, label) in [
            (2917, IC_CHEV_UP, "Vorherige Formatvorlagen"),
            (2918, IC_CHEV_DOWN, "Nächste Formatvorlagen"),
            (2916, IC_MORE, "Weitere Formatvorlagen"),
        ] {
            ui.next_size(14, 20);
            let enabled = id == 2916 || id == 2917 && self.styles_ui.gallery_page > 0 || id == 2918 && self.styles_ui.gallery_page < max_page;
            if ui.ribbon_button(id, label, icon, ST_ICON_ONLY | if enabled { 0 } else { ST_DISABLED }) & EV_ACT != 0 {
                if id == 2916 { self.open_styles(); } else if id == 2917 { self.styles_ui.gallery_page -= 1; } else { self.styles_ui.gallery_page += 1; }
            }
        }
        ui.box_end();
        ui.box_end();
    }
}
