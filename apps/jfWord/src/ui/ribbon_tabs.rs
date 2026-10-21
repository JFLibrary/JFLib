// Reference-shaped command composition; unsupported actions retain disabled semantics.
use super::ribbon_ui::*;
use crate::*;

enum Column<'a> {
    Big(u32, &'a str, u32, i32, bool),
    Small(i32, &'a [(u32, &'a str, u32, bool)]),
}
struct Group<'a>(&'a str, i32, &'a [Column<'a>]);
use Column::{Big as B, Small as S};

impl Column<'_> {
    fn width(&self, ui: &mut Ui) -> i32 {
        match self {
            B(_, label, glyph, minimum, menu) => (*minimum).max(ui.ribbon_button_width(
                label,
                *glyph,
                ST_LARGE | if *menu { ST_MENU } else { 0 },
            )),
            S(minimum, items) => items
                .iter()
                .fold(*minimum, |width, (_, label, glyph, menu)| {
                    width.max(ui.ribbon_button_width(
                        label,
                        *glyph,
                        if *menu { ST_MENU } else { 0 },
                    ))
                }),
        }
    }
}

impl Word {
    fn reference_groups(&mut self, ui: &mut Ui, groups: &[Group<'_>]) {
        let descriptors: Vec<_> = groups
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let measured = g.2.iter().map(|column| column.width(ui)).sum::<i32>()
                    + 2 * g.2.len().saturating_sub(1) as i32;
                let width = measured.max(g.1);
                RibbonGroup::new(1500 + i as u32 * 10, g.0, width, width)
            })
            .collect();
        let layout = ui.ribbon_begin(101, &descriptors);
        for (index, descriptor) in descriptors.iter().enumerate() {
            let Some(mode) = ui.ribbon_group_begin(&layout, index, descriptor) else {
                continue;
            };
            row(ui);
            for column in groups[index].2 {
                let width = column.width(ui);
                match column {
                    B(id, label, glyph, _, menu) => {
                        ui.next_size(width, 66);
                        if *id==1514 {if ui.ribbon_button(*id,label,*glyph,ST_LARGE|ST_MENU)&EV_ACT!=0 {self.open_table_insert();}}
                        else {unavailable(ui, *id, label, *glyph, true, *menu);}
                    }
                    S(_, items) => {
                        stack(ui, width);
                        for (id, label, glyph, menu) in *items {
                            if *id == 1506 {
                                ui.next_size(-1, 22);
                                if ui.ribbon_button(*id, label, *glyph, ST_ALIGN_START) & EV_ACT
                                    != 0
                                {
                                    let value =
                                        self.formatting_paragraph().format.page_break_before;
                                    self.paragraph_change(
                                        ui,
                                        doc_core::ParagraphChange::PageBreakBefore(!value),
                                    );
                                }
                            } else {
                                unavailable(ui, *id, label, *glyph, false, *menu);
                            }
                        }
                        ui.box_end();
                    }
                }
            }
            ui.box_end();
            ui.ribbon_group_end(mode, descriptor.label);
        }
        ui.ribbon_end();
    }

    pub(super) fn insert_ribbon(&mut self, ui: &mut Ui) {
        self.reference_groups(
            ui,
            &[
                Group(
                    "Seiten",
                    110,
                    &[S(
                        110,
                        &[
                            (1504, "Deckblatt", IC_OFFICE_COVER_PAGE, true),
                            (1505, "Leere Seite", IC_OFFICE_BLANK_PAGE, false),
                            (1506, "Seitenumbruch", IC_OFFICE_PAGE_BREAK, false),
                        ],
                    )],
                ),
                Group(
                    "Tabellen",
                    53,
                    &[B(1514, "Tabelle", IC_OFFICE_TABLE, 53, true)],
                ),
                Group(
                    "Illustrationen",
                    202,
                    &[
                        B(1515, "Bilder", IC_OFFICE_PICTURE, 53, false),
                        B(1516, "Formen", IC_OFFICE_SHAPES, 59, true),
                        S(
                            86,
                            &[
                                (1517, "Piktogramme", IC_OFFICE_ICONS, false),
                                (1518, "Strukturdiagramm", IC_OFFICE_SMART_ART, false),
                                (1519, "Diagramm", IC_OFFICE_CHART, false),
                            ],
                        ),
                    ],
                ),
                Group(
                    "Links",
                    128,
                    &[S(
                        128,
                        &[
                            (1524, "Link", IC_OFFICE_LINK, false),
                            (1525, "Textmarke", IC_OFFICE_BOOKMARK, false),
                            (1526, "Querverweis", IC_OFFICE_CROSS_REF, false),
                        ],
                    )],
                ),
                Group(
                    "Kommentare",
                    78,
                    &[B(3100, "Kommentar", IC_OFFICE_COMMENT, 78, false)],
                ),
                Group(
                    "Kopf- und Fußzeile",
                    192,
                    &[
                        B(1534, "Kopfzeile", IC_OFFICE_HEADER, 61, true),
                        B(1535, "Fußzeile", IC_OFFICE_FOOTER, 61, true),
                        B(1536, "Seiten-\nzahl", IC_OFFICE_PAGE_NUMBER, 66, true),
                    ],
                ),
                Group(
                    "Text",
                    176,
                    &[
                        B(1544, "Text-\nfeld", IC_OFFICE_TEXT_BOX, 44, false),
                        S(
                            106,
                            &[
                                (3101, "Schnellbausteine", IC_OFFICE_QUICK_PARTS, false),
                                (3102, "Texteffekt", IC_OFFICE_WORD_ART, false),
                                (3103, "Initiale", IC_OFFICE_DROP_CAP, false),
                            ],
                        ),
                        S(
                            22,
                            &[
                                (3104, "", IC_OFFICE_SIGNATURE, false),
                                (3105, "", IC_OFFICE_DATE_TIME, false),
                                (3106, "", IC_OFFICE_OBJECT, false),
                            ],
                        ),
                    ],
                ),
                Group(
                    "Symbole",
                    124,
                    &[
                        B(1545, "Formel", IC_OFFICE_EQUATION, 62, false),
                        B(1546, "Symbol", IC_OFFICE_SYMBOL, 60, true),
                    ],
                ),
            ],
        );
    }

    pub(super) fn draw_ribbon(&mut self, ui: &mut Ui) {
        self.reference_groups(
            ui,
            &[
                Group(
                    "Zeichenwerkzeuge",
                    410,
                    &[
                        B(3110, "Auswählen", IC_OFFICE_SELECT, 70, false),
                        B(3111, "Lasso", IC_OFFICE_LASSO, 49, false),
                        B(3112, "Radierer", IC_OFFICE_ERASER, 60, false),
                        B(3113, "Stift", IC_OFFICE_PEN, 44, false),
                        B(3114, "Bleistift", IC_OFFICE_PENCIL, 57, false),
                        B(3115, "Textmarker", IC_OFFICE_HIGHLIGHT, 72, false),
                        S(
                            44,
                            &[
                                (3116, "Farbe", IC_OFFICE_FONTCOLOR, true),
                                (3117, "Stärke", IC_OFFICE_THICKNESS, true),
                            ],
                        ),
                    ],
                ),
                Group(
                    "Konvertieren",
                    170,
                    &[
                        B(3118, "Freihand\nin Form", IC_OFFICE_INK_TO_SHAPE, 83, false),
                        B(
                            3119,
                            "Freihand\nin Formel",
                            IC_OFFICE_INK_TO_MATH,
                            85,
                            false,
                        ),
                    ],
                ),
                Group(
                    "Einfügen",
                    100,
                    &[B(3120, "Zeichen-\nbereich", IC_OFFICE_CANVAS, 100, false)],
                ),
                Group(
                    "Wiedergabe",
                    90,
                    &[B(
                        3121,
                        "Freihand-\nwiedergabe",
                        IC_OFFICE_REPLAY,
                        90,
                        false,
                    )],
                ),
            ],
        );
    }

    pub(super) fn design_ribbon(&mut self, ui: &mut Ui) {
        let font_width = ui.ribbon_button_width("Schriften", IC_OFFICE_FONTS, ST_MENU);
        let setting_width =
            ui.ribbon_button_width("Als Standard festlegen", IC_OFFICE_SET_DEFAULT, 0);
        let formatting_width = 7 * 56 + font_width + setting_width + 16;
        let groups = [
            RibbonGroup::new(1500, "Designs", 65, 65),
            RibbonGroup::new(
                1510,
                "Dokumentformatierung",
                formatting_width,
                formatting_width,
            ),
            RibbonGroup::new(1520, "Seitenhintergrund", 216, 216),
        ];
        let layout = ui.ribbon_begin(101, &groups);
        for (index, group) in groups.iter().enumerate() {
            let Some(mode) = ui.ribbon_group_begin(&layout, index, group) else {
                continue;
            };
            row(ui);
            match index {
                0 => unavailable(ui, 3130, "Designs", IC_OFFICE_THEMES, true, true),
                1 => {
                    for i in 0..7 {
                        ui.next_size(56, 66);
                        ui.next_flags(ST_DISABLED);
                        ui.ribbon_theme_tile(3131 + i, "Dokumentdesign", 0xff1a6080, i);
                    }
                    stack(ui, font_width);
                    unavailable(ui, 3140, "Farben", IC_OFFICE_COLORS, false, true);
                    unavailable(ui, 3141, "Schriften", IC_OFFICE_FONTS, false, true);
                    ui.box_end();
                    stack(ui, setting_width);
                    unavailable(
                        ui,
                        3142,
                        "Absatzabstand",
                        IC_OFFICE_PARA_SPACING,
                        false,
                        true,
                    );
                    unavailable(ui, 3143, "Effekte", IC_OFFICE_EFFECTS_DESIGN, false, true);
                    unavailable(
                        ui,
                        3144,
                        "Als Standard festlegen",
                        IC_OFFICE_SET_DEFAULT,
                        false,
                        false,
                    );
                    ui.box_end();
                }
                _ => {
                    unavailable(ui, 3145, "Wasserzeichen", IC_OFFICE_WATERMARK, true, true);
                    unavailable(ui, 3146, "Seiten-\nfarbe", IC_OFFICE_PAGE_COLOR, true, true);
                    unavailable(
                        ui,
                        3147,
                        "Seiten-\nränder",
                        IC_OFFICE_PAGE_BORDERS,
                        true,
                        true,
                    );
                }
            }
            ui.box_end();
            ui.ribbon_group_end(mode, group.label);
        }
        ui.ribbon_end();
    }

    pub(super) fn mailings_ribbon(&mut self, ui: &mut Ui) {
        self.reference_groups(
            ui,
            &[
                Group(
                    "Erstellen",
                    136,
                    &[
                        B(3150, "Umschläge", IC_OFFICE_ENVELOPE, 73, false),
                        B(3151, "Etiketten", IC_OFFICE_LABELS, 61, false),
                    ],
                ),
                Group(
                    "Seriendruck starten",
                    241,
                    &[
                        B(3152, "Seriendruck\nstarten", IC_OFFICE_MAIL_MERGE, 77, true),
                        B(3153, "Empfänger\nauswählen", IC_OFFICE_RECIPIENTS, 79, true),
                        B(
                            3154,
                            "Empfängerliste\nbearbeiten",
                            IC_OFFICE_EDIT_RECIPIENTS,
                            81,
                            false,
                        ),
                    ],
                ),
                Group(
                    "Felder schreiben und einfügen",
                    450,
                    &[
                        B(
                            3155,
                            "Seriendruckfelder\nhervorheben",
                            IC_OFFICE_HIGHLIGHT_FIELDS,
                            104,
                            false,
                        ),
                        B(3156, "Adress-\nblock", IC_OFFICE_ADDRESS_BLOCK, 66, false),
                        B(3157, "Gruß-\nzeile", IC_OFFICE_GREETING_LINE, 56, false),
                        B(
                            3158,
                            "Seriendruckfeld\neinfügen",
                            IC_OFFICE_MERGE_FIELD,
                            94,
                            true,
                        ),
                        S(
                            122,
                            &[
                                (3159, "Regeln", IC_OFFICE_RULES, true),
                                (3160, "Felder zuordnen", IC_OFFICE_MATCH_FIELDS, false),
                            ],
                        ),
                    ],
                ),
                Group(
                    "Vorschau Ergebnisse",
                    230,
                    &[
                        B(3161, "Vorschau\nErgebnisse", IC_OFFICE_PREVIEW, 84, false),
                        S(
                            144,
                            &[
                                (3162, "Nächster Empfänger", IC_OFFICE_NEXT, false),
                                (3163, "Empfänger suchen", IC_OFFICE_FIND, false),
                                (3164, "Auf Fehler prüfen", IC_OFFICE_CHECK_ERRORS, false),
                            ],
                        ),
                    ],
                ),
                Group(
                    "Fertigstellen",
                    90,
                    &[B(
                        3165,
                        "Fertig stellen\nund zusammenführen",
                        IC_OFFICE_FINISH,
                        90,
                        true,
                    )],
                ),
            ],
        );
    }

    pub(super) fn review_ribbon(&mut self, ui: &mut Ui) {
        self.reference_groups(
            ui,
            &[
                Group(
                    "Dokumentprüfung",
                    177,
                    &[
                        B(
                            3170,
                            "Rechtschreibung\nund Grammatik",
                            IC_OFFICE_SPELLING,
                            82,
                            false,
                        ),
                        S(
                            93,
                            &[
                                (3171, "Thesaurus", IC_OFFICE_THESAURUS, false),
                                (3172, "Wörter zählen", IC_OFFICE_WORD_COUNT, false),
                            ],
                        ),
                    ],
                ),
                Group(
                    "Sprache",
                    67,
                    &[B(3173, "Laut\nvorlesen", IC_OFFICE_READ_ALOUD, 67, false)],
                ),
                Group(
                    "Barrierefreiheit",
                    85,
                    &[B(
                        3174,
                        "Barrierefreiheit\nprüfen",
                        IC_OFFICE_ACCESSIBILITY,
                        85,
                        false,
                    )],
                ),
                Group(
                    "Sprache",
                    130,
                    &[
                        B(3175, "Übersetzen", IC_OFFICE_TRANSLATE, 72, false),
                        B(3176, "Sprache", IC_OFFICE_LANGUAGE, 56, true),
                    ],
                ),
                Group(
                    "Kommentare",
                    240,
                    &[
                        B(3177, "Neuer\nKommentar", IC_OFFICE_NEW_COMMENT, 74, false),
                        S(
                            85,
                            &[
                                (3178, "Löschen", IC_OFFICE_DELETE_COMMENT, false),
                                (3179, "Vorheriger", IC_OFFICE_PREV_COMMENT, false),
                                (3180, "Nächster", IC_OFFICE_NEXT_COMMENT, false),
                            ],
                        ),
                        B(
                            3181,
                            "Kommentare\nanzeigen",
                            IC_OFFICE_SHOW_COMMENTS,
                            77,
                            false,
                        ),
                    ],
                ),
                Group(
                    "Nachverfolgung",
                    208,
                    &[
                        B(
                            3182,
                            "Änderungen\nnachverfolgen",
                            IC_OFFICE_TRACK_CHANGES,
                            89,
                            true,
                        ),
                        S(
                            117,
                            &[
                                (3183, "Alle Markups", IC_OFFICE_MARKUP, true),
                                (
                                    3184,
                                    "Überarbeitungsbereich",
                                    IC_OFFICE_REVIEWING_PANE,
                                    false,
                                ),
                            ],
                        ),
                    ],
                ),
                Group(
                    "Änderungen",
                    215,
                    &[
                        B(3185, "Annehmen", IC_OFFICE_ACCEPT, 70, true),
                        B(3186, "Ablehnen", IC_OFFICE_REJECT, 67, true),
                        S(
                            74,
                            &[
                                (3187, "Vorherige", IC_OFFICE_PREV_CHANGE, false),
                                (3188, "Nächste", IC_OFFICE_NEXT_CHANGE, false),
                            ],
                        ),
                    ],
                ),
                Group(
                    "Vergleichen",
                    78,
                    &[B(3189, "Vergleichen", IC_OFFICE_COMPARE, 78, true)],
                ),
                Group(
                    "Schützen",
                    163,
                    &[
                        B(
                            3190,
                            "Autoren\nblockieren",
                            IC_OFFICE_BLOCK_AUTHORS,
                            74,
                            false,
                        ),
                        B(
                            3191,
                            "Bearbeitung\neinschränken",
                            IC_OFFICE_RESTRICT,
                            87,
                            false,
                        ),
                    ],
                ),
            ],
        );
    }

    pub(super) fn help_ribbon(&mut self, ui: &mut Ui) {
        let groups = [
            RibbonGroup::new(1500, "Hilfe", 165, 165),
            RibbonGroup::new(1510, "Automatisierung", 90, 90),
        ];
        let layout = ui.ribbon_begin(101, &groups);
        for (index, group) in groups.iter().enumerate() {
            let Some(mode) = ui.ribbon_group_begin(&layout, index, group) else {
                continue;
            };
            row(ui);
            if index == 0 {
                if large(ui, 3200, "Information", IC_OFFICE_INFO, 0) {
                    ui.cmd_run(COMPAT_INFO, 0);
                }
                unavailable(ui, 3201, "Hilfe", IC_OFFICE_HELP, true, false);
            } else {
                unavailable(ui, 3202, "Befehle", IC_OFFICE_MACROS, true, false);
            }
            ui.box_end();
            ui.ribbon_group_end(mode, group.label);
        }
        ui.ribbon_end();
    }
}
