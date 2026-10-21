// Native Office shell: start/backstage, original templates, recents and command discovery.
use crate::{
    BIBLIOGRAPHY, CITE, COMPAT_INFO, ENDNOTE, FOOTNOTE, HOME, NAVIGATION, NEW, NEW_LETTER,
    NEW_REPORT, NEW_RESEARCH, NEW_RESUME, OPEN, OPEN_RECENT, PAGE_SETUP, PARAGRAPH_SETUP, REDO,
    SAVE, SAVE_AS, SEARCH, SOURCE_LIBRARY, SOURCE_NEW, SOURCES, UNDO, VIEW_FOCUS, VIEW_PAGE_WIDTH,
    Word,
};
use alloc::{format, rc::Rc, string::String, vec::Vec};
use doc_core::{Document, Inline, Paragraph, ParagraphFormat, ParagraphStyle};
use ui::*;

#[derive(Clone)]
struct Recent {
    path: String,
    date: String,
    pinned: bool,
}
pub(crate) struct Shell {
    pub(crate) home: bool,
    pub(crate) has_document: bool,
    pub(crate) open_target: String,
    config: Option<String>,
    recent: Vec<Recent>,
    query: String,
    filter: i32,
    pub(crate) command_query: String,
    template_draws: Vec<DrawRef>,
}
impl Default for Shell {
    fn default() -> Self {
        Self {
            home: true,
            has_document: false,
            open_target: String::new(),
            config: None,
            recent: Vec::new(),
            query: String::new(),
            filter: 0,
            command_query: String::new(),
            template_draws: Vec::new(),
        }
    }
}
impl Shell {
    pub(crate) fn configure(&mut self, path: Option<String>) {
        self.config = path;
        if let Some(path) = &self.config {
            let ini = base::Ini::load(path);
            for index in 0..24 {
                let section = format!("recent{index}");
                if let Some(path) = ini.get(&section, "path") {
                    if !path.is_empty()
                        && path.len() < 4096
                        && !path.contains(['\r', '\n', '\0'])
                        && !self.recent.iter().any(|entry| same_path(&entry.path, path))
                    {
                        self.recent.push(Recent {
                            path: String::from(path),
                            date: String::from(ini.get(&section, "date").unwrap_or("")),
                            pinned: ini.get_int(&section, "pinned", 0) != 0,
                        });
                    }
                }
            }
        }
    }
    fn persist(&self) -> bool {
        let Some(path) = &self.config else {
            return true;
        };
        let mut ini = base::Ini::new();
        for (index, entry) in self.recent.iter().enumerate() {
            let section = format!("recent{index}");
            ini.set(&section, "path", &entry.path);
            ini.set(&section, "date", &entry.date);
            ini.set_int(&section, "pinned", i64::from(entry.pinned));
        }
        ini.save(path)
    }
    pub(crate) fn remember(&mut self, path: &str) {
        let path = if path.starts_with('/')
            || path.starts_with('\\')
            || path.as_bytes().get(1) == Some(&b':')
        {
            String::from(path)
        } else {
            format!("{}/{path}", plat::cwd())
        };
        if path.len() >= 4096 || path.contains(['\r', '\n', '\0']) {
            return;
        }
        let pinned = self
            .recent
            .iter()
            .find(|entry| same_path(&entry.path, &path))
            .is_some_and(|entry| entry.pinned);
        self.recent.retain(|entry| !same_path(&entry.path, &path));
        let (date, valid) = plat::date();
        let date = if valid {
            format!("{:02}.{:02}.{:04}", date.day, date.month, date.year)
        } else {
            String::new()
        };
        self.recent.insert(0, Recent { path, pinned, date });
        if self.recent.len() > 24 {
            let index = self
                .recent
                .iter()
                .rposition(|entry| !entry.pinned)
                .unwrap_or(24);
            self.recent.remove(index);
        }
        self.persist();
    }
}
fn same_path(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.replace('\\', "/")
            .eq_ignore_ascii_case(&b.replace('\\', "/"))
    } else {
        a == b
    }
}

pub(crate) fn template(kind: u32) -> Document {
    let mut document = Document::default();
    let rows: &[(ParagraphStyle, &str)] = match kind {
        NEW_RESEARCH => &[
            (ParagraphStyle::Title, "Titel der wissenschaftlichen Arbeit"),
            (ParagraphStyle::Normal, "Verfasser · Hochschule · Datum"),
            (ParagraphStyle::Heading1, "Zusammenfassung"),
            (ParagraphStyle::Normal, ""),
            (ParagraphStyle::Heading1, "1 Einleitung"),
            (ParagraphStyle::Normal, ""),
            (ParagraphStyle::Heading1, "2 Methode"),
            (ParagraphStyle::Normal, ""),
            (ParagraphStyle::Heading1, "3 Ergebnisse"),
            (ParagraphStyle::Normal, ""),
        ],
        NEW_REPORT => &[
            (ParagraphStyle::Title, "Bericht"),
            (ParagraphStyle::Normal, "Titel · Datum · Verfasser"),
            (ParagraphStyle::Heading1, "Überblick"),
            (ParagraphStyle::Normal, ""),
            (ParagraphStyle::Heading1, "Ergebnisse und nächste Schritte"),
            (ParagraphStyle::Normal, ""),
        ],
        NEW_LETTER => &[
            (ParagraphStyle::Title, "Ihr Name"),
            (
                ParagraphStyle::Normal,
                "Straße und Hausnummer · Postleitzahl Ort",
            ),
            (ParagraphStyle::Normal, "Telefon · E-Mail"),
            (ParagraphStyle::Normal, ""),
            (ParagraphStyle::Normal, "Empfänger"),
            (ParagraphStyle::Normal, "Organisation"),
            (ParagraphStyle::Normal, "Straße und Hausnummer"),
            (ParagraphStyle::Normal, "Postleitzahl Ort"),
            (ParagraphStyle::Normal, ""),
            (ParagraphStyle::Heading1, "Betreff"),
            (ParagraphStyle::Normal, "Sehr geehrte Damen und Herren,"),
            (
                ParagraphStyle::Normal,
                "hier beginnt Ihr Brief. Ergänzen Sie den Anlass und Ihre Nachricht.",
            ),
            (ParagraphStyle::Normal, "Mit freundlichen Grüßen"),
            (ParagraphStyle::Normal, "Ihr Name"),
        ],
        NEW_RESUME => &[
            (ParagraphStyle::Title, "Ihr Name"),
            (
                ParagraphStyle::Normal,
                "Berufsbezeichnung · E-Mail · Telefon · Wohnort",
            ),
            (ParagraphStyle::Heading1, "Profil"),
            (
                ParagraphStyle::Normal,
                "Ihre Erfahrung, Schwerpunkte und beruflichen Ziele.",
            ),
            (ParagraphStyle::Heading1, "Berufserfahrung"),
            (ParagraphStyle::Heading2, "Position · Unternehmen"),
            (
                ParagraphStyle::Normal,
                "Zeitraum · Aufgaben, Verantwortung und Ergebnisse",
            ),
            (ParagraphStyle::Heading1, "Ausbildung"),
            (
                ParagraphStyle::Heading2,
                "Abschluss · Hochschule oder Einrichtung",
            ),
            (
                ParagraphStyle::Normal,
                "Zeitraum · Fachrichtung und Schwerpunkte",
            ),
            (ParagraphStyle::Heading1, "Kenntnisse"),
            (
                ParagraphStyle::Normal,
                "Fachkenntnisse · Sprachen · Werkzeuge",
            ),
        ],
        _ => return document,
    };
    document.paragraphs = rows
        .iter()
        .enumerate()
        .map(|(index, (style, text))| Paragraph {
            id: index as u64 + 1,
            style: *style,
            format: ParagraphFormat::default(),
            content: alloc::vec![Inline::Text(String::from(*text))],
        })
        .collect();
    document
}

impl Word {
    pub(crate) fn shell_header(&mut self, ui: &mut Ui) {
        ui.titlebar_begin_opt(None, 0);
        let bar = ui.box_rect();
        let compact = bar[2] < ui.dp(800);
        if !compact {
            ui.next_size(86, 24);
            ui.text(TS_CAPTION | TX_SECONDARY, "AutoSpeichern");
            let mut autosave = 0;
            ui.next_size(30, 24);
            ui.next_flags(ST_COMPACT | ST_DISABLED);
            ui.switch(2107, "", &mut autosave);
            ui.tooltip("Automatisches Speichern ist noch nicht verfügbar.");
            ui.spacer(12);
        }
        for command in [SAVE, UNDO, REDO] {
            ui.next_size(26, 24);
            ui.cmd_button(command, ST_SUBTLE | ST_ICON_ONLY | ST_COMPACT);
            ui.spacer(2);
        }
        if !compact {
            ui.next_size(26, 24);
            ui.next_flags(ST_SUBTLE | ST_DISABLED | ST_COMPACT);
            ui.icon_button(2108, IC_MORE, "Schnellzugriff anpassen");
            ui.tooltip("Der Schnellzugriff kann noch nicht angepasst werden.");
        }
        let left_edge = ui.last_rect()[0] + ui.last_rect()[2] + ui.dp(12);
        let cap_width = ui.dp(ui.style.metric(SM_CAP_W) * 3);
        let search_width = if compact {
            ui.dp(140)
        } else {
            ui.dp(260).min((bar[2] / 3).max(ui.dp(140)))
        };
        let search_x = bar[0] + bar[2] - cap_width - ui.dp(44) - search_width;
        let name = self
            .path
            .rsplit(['\\', '/'])
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("Unbenannt");
        let state = if self.document_dirty() {
            "Geändert"
        } else if self.path.is_empty() {
            "Nicht gespeichert"
        } else {
            "Gespeichert"
        };
        let center = if compact {
            (left_edge + search_x - ui.dp(12)) / 2
        } else {
            bar[0] + bar[2] / 2
        };
        let half = (center - left_edge)
            .min(search_x - ui.dp(12) - center)
            .max(0);
        if half > ui.dp(40) {
            ui.next_rect_px(center - half, bar[1], half * 2, bar[3]);
            ui.text(
                TS_BODY_STRONG | TX_CENTER | TX_ELLIPSIS,
                &format!("{name} — {state}"),
            );
        }
        ui.next_rect_px(
            search_x,
            bar[1] + (bar[3] - ui.dp(26)) / 2,
            search_width,
            ui.dp(26),
        );
        ui.textbox_icon(
            2104,
            IC_SEARCH,
            Some("Befehle und Hilfe durchsuchen"),
            &mut self.shell.command_query,
            256,
        );
        ui.next_rect_px(
            search_x + search_width + ui.dp(8),
            bar[1] + (bar[3] - ui.dp(26)) / 2,
            ui.dp(26),
            ui.dp(26),
        );
        ui.next_flags(ST_ACCENT | ST_COMPACT | ST_ROUND);
        if ui.button(2101, "jf") & EV_ACT != 0 {
            ui.cmd_run(HOME, 0);
        }
        ui.tooltip("jfWord · Datei und Startseite");
        ui.titlebar_end();
        if !self.shell.command_query.trim().is_empty() {
            let query = self.shell.command_query.to_lowercase();
            let mut chosen = None;
            ui.next_size(-1, 0);
            ui.box_begin(2106, BOX_V | BOX_CARD | pad(10), 4);
            for (index, (label, command)) in COMMANDS
                .iter()
                .enumerate()
                .filter(|(_, (label, _))| label.to_lowercase().contains(&query))
                .take(7)
            {
                ui.next_size(-1, 26);
                if ui.button(2300 + index as u32, label) & EV_ACT != 0 {
                    chosen = Some(*command);
                }
            }
            if !COMMANDS
                .iter()
                .any(|(label, _)| label.to_lowercase().contains(&query))
            {
                ui.text(TS_BODY, "Kein passender Befehl");
            }
            ui.box_end();
            if let Some(command) = chosen {
                self.shell.command_query.clear();
                ui.cmd_run(command, 0);
            }
        }
    }
    pub(crate) fn home_view(&mut self, ui: &mut Ui) {
        if self.shell.template_draws.is_empty() {
            if let Ok(fonts) = self.preview.prepare_fonts(&mut ui.text) {
                for kind in [NEW, NEW_RESEARCH, NEW_LETTER, NEW_RESUME, NEW_REPORT] {
                    if let Ok(layout) = doc_view::paginate(&template(kind), &mut ui.text, &fonts) {
                        if let Ok(draw) = doc_view::PageDraw::new(Rc::new(layout), 0, fonts.clone()) {
                            self.shell.template_draws.push(Rc::new(draw.without_frame()));
                        }
                    }
                }
                // Never bind another template's page to a missing thumbnail slot.
                if self.shell.template_draws.len() != 5 {
                    self.shell.template_draws.clear();
                }
            }
        }
        ui.next_size(-1, -1);
        ui.box_begin(2110, BOX_H, 0);
        let backstage = ui.box_rect();
        ui.draw_color(ui.color(T_LAYER));
        ui.draw_rect(backstage[0], backstage[1], backstage[2], backstage[3]);
        ui.next_size(200, -1);
        ui.box_begin(2111, BOX_V, 0);
        let sidebar = ui.box_rect();
        ui.draw_color(ui.style.navigation_surface(ui));
        ui.draw_rect(sidebar[0], sidebar[1], sidebar[2], sidebar[3]);
        ui.spacer(12);
        ui.next_size(-1, 40);
        ui.next_flags(
            ST_SUBTLE
                | ST_ON_ACCENT
                | ST_ALIGN_START
                | ST_ICON_ONLY
                | if self.shell.has_document {
                    0
                } else {
                    ST_DISABLED
                },
        );
        ui.next_icon(IC_BACK);
        if ui.button(2115, "Zum Dokument") & EV_ACT != 0 {
            self.shell.home = false;
        }
        ui.tooltip("Zum Dokument zurückkehren");
        ui.spacer(10);
        for (id, label, command, available) in [
            (2112, "Startseite", HOME, true),
            (2113, "Neu", NEW, true),
            (2114, "Öffnen", OPEN, true),
            (2118, "Informationen", COMPAT_INFO, self.shell.has_document),
            (SAVE, "Speichern", SAVE, self.shell.has_document),
            (SAVE_AS, "Speichern unter", SAVE_AS, self.shell.has_document),
            (2119, "Drucken", 0, false),
            (2129, "Exportieren", 0, false),
            (0x550100, "Optionen", 0, false),
        ] {
            ui.next_flags(
                ST_SUBTLE
                    | ST_ON_ACCENT
                    | ST_ALIGN_START
                    | if command == HOME { ST_CHECKED } else { 0 }
                    | if available { 0 } else { ST_DISABLED },
            );
            ui.next_size(-1, 38);
            if ui.button(id, label) & EV_ACT != 0 && command != 0 {
                ui.cmd_run(command, 0);
            }
            if !available {
                ui.tooltip("Diese Funktion ist derzeit nicht verfügbar.");
            }
            ui.spacer(4);
        }
        ui.spacer(-1);
        ui.next_size(-1, 32);
        ui.next_flags(ST_SUBTLE | ST_ON_ACCENT | ST_ALIGN_START);
        if ui.button(0x550101, "Über jfWord") & EV_ACT != 0 {
            ui.cmd_run(COMPAT_INFO, 0);
        }
        ui.spacer(12);
        ui.box_end();
        ui.next_size(-1, -1);
        ui.scroll_begin(2116, SC_V | SC_H | BOX_V | pad(30));
        ui.next_size(-1, -1);
        ui.box_begin(0x550102, BOX_H, 0);
        ui.spacer(10);
        ui.next_size(-1, -1);
        ui.box_begin(0x550103, BOX_V, 0);
        ui.text(TS_TITLE, "Willkommen bei jfWord");
        ui.spacer(16);
        ui.next_size(0, 212);
        ui.box_begin(2117, BOX_H, 18);
        for (index, (kind, label, base_id)) in [
            (NEW, "Leeres Dokument", 2120),
            (NEW_RESEARCH, "Wissenschaftliche Arbeit", 2123),
            (NEW_LETTER, "Brief", 0x550110),
            (NEW_RESUME, "Lebenslauf", 0x550113),
            (NEW_REPORT, "Bericht", 2126),
        ]
        .into_iter()
        .enumerate()
        {
            ui.ts_font(TS_BODY);
            let label_width = ui.text.width(label);
            let tile_width = 150.max(ui.px2dip(label_width) + 1);
            ui.next_size(tile_width, 212);
            ui.box_begin(base_id, BOX_V, 0);
            ui.next_size(150, 194);
            let ev = ui.preview_card(
                base_id + 1,
                &format!("Vorlage: {label}"),
                self.shell.template_draws.get(index).cloned(),
                0,
                kind,
            );
            if ev & EV_ACT != 0 {
                ui.cmd_run(kind, 0);
            }
            ui.next_size(-1, 18);
            ui.next_flags(ST_SUBTLE | ST_ALIGN_START);
            if ui.label_button(base_id + 2, label, TS_BODY, 0) & EV_ACT != 0 {
                ui.cmd_run(kind, 0);
            }
            ui.box_end();
        }
        ui.box_end();
        ui.spacer(24);
        if self.shell.recent.is_empty() {
            ui.next_size(-1, 28);
            ui.text(TS_SUBTITLE, "Zuletzt verwendet");
            ui.spacer(4);
        } else {
            ui.box_begin(2130, BOX_H, 16);
            ui.next_size(-1, 28);
            ui.next_flags(ST_COMPACT);
            ui.tabs(
                2250,
                &mut self.shell.filter,
                &["Zuletzt verwendet", "Angeheftet"],
            );
            ui.next_size(280, 26);
            ui.textbox(
                2132,
                Some("Nach einer Datei suchen …"),
                &mut self.shell.query,
                1024,
            );
            ui.box_end();
        }
        let query = self.shell.query.to_lowercase();
        let recent: Vec<_> = self
            .shell
            .recent
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                (self.shell.filter == 0 || entry.pinned)
                    && entry.path.to_lowercase().contains(&query)
            })
            .map(|(i, e)| (i, e.clone()))
            .collect();
        if recent.is_empty() {
            ui.text(
                TS_BODY | TX_SECONDARY,
                "Hier erscheinen Ihre geöffneten und gespeicherten Dokumente.",
            );
        }
        let mut pin = None;
        let mut open = None;
        for (index, entry) in recent {
            ui.separator();
            ui.next_size(-1, 66);
            ui.box_begin(2140 + index as u32 * 4, BOX_H | pad(4), 12);
            let name = entry.path.rsplit(['\\', '/']).next().unwrap_or(&entry.path);
            ui.next_size(-1, 0);
            ui.box_begin(0, BOX_V, 2);
            ui.next_size(-1, 28);
            ui.next_flags(ST_SUBTLE);
            if ui.button(2141 + index as u32 * 4, name) & EV_ACT != 0 {
                open = Some(entry.path.clone());
            }
            ui.text(TS_CAPTION | TX_SECONDARY | TX_ELLIPSIS, &entry.path);
            ui.box_end();
            ui.next_size(112, 30);
            ui.text(TS_CAPTION | TX_SECONDARY, &entry.date);
            ui.next_size(112, 30);
            if ui.button(
                2142 + index as u32 * 4,
                if entry.pinned {
                    "Loslösen"
                } else {
                    "Anheften"
                },
            ) & EV_ACT
                != 0
            {
                pin = Some(index);
            }
            ui.box_end();
        }
        if let Some(index) = pin {
            self.shell.recent[index].pinned = !self.shell.recent[index].pinned;
            if !self.shell.persist() {
                self.message = String::from(
                    "Die Liste zuletzt verwendeter Dateien konnte nicht gespeichert werden.",
                );
            }
        }
        if let Some(path) = open {
            self.shell.open_target = path;
            ui.cmd_run(OPEN_RECENT, 0);
        }
        ui.box_end();
        ui.spacer(10);
        ui.box_end();
        ui.scroll_end();
        ui.box_end();
        // Keep native controls and explicit agent activation visible without a second header.
        let edge = ui.dp(200);
        ui.next_rect_px(
            backstage[0] + edge,
            backstage[1],
            (backstage[2] - edge).max(0),
            ui.dp(38),
        );
        ui.titlebar_begin_overlay(None, 0);
        ui.titlebar_end();
    }
}
const COMMANDS: &[(&str, u32)] = &[
    ("Neues Dokument", NEW),
    ("Dokument öffnen", OPEN),
    ("Speichern", SAVE),
    ("Speichern unter", SAVE_AS),
    ("Quelle anlegen", SOURCE_NEW),
    ("Quellenbibliothek", SOURCE_LIBRARY),
    ("Quellenbereich", SOURCES),
    ("Zitat einfügen", CITE),
    ("Literaturverzeichnis einfügen", BIBLIOGRAPHY),
    ("Fußnote einfügen", FOOTNOTE),
    ("Endnote einfügen", ENDNOTE),
    ("Navigation", NAVIGATION),
    ("Fokusmodus", VIEW_FOCUS),
    ("Seitenbreite", VIEW_PAGE_WIDTH),
    ("Suchen / Ersetzen", SEARCH),
    ("Seite einrichten", PAGE_SETUP),
    ("Absatz formatieren", PARAGRAPH_SETUP),
    ("Startseite / Datei", HOME),
    ("Dateiinformationen", COMPAT_INFO),
];
