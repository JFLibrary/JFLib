// ribbon -- the registers Start, Einfügen, Entwurf and Ansicht with placeholder groups
//
//   TABS                    the register names (German source texts, translated where shown)
//   draw(ui, tab)           the ribbon of one register: its groups, every button disabled
//
// Stage JD0: what the ribbon will hold, as a frame to see the NetLab look with real groups.
// Every button is disabled ("not available yet") and does nothing; JD8 builds the real
// registers (FEATURES.md 3.3-3.9) and connects them as the functions behind them are done.
// A group is a list of columns: one large button, or up to three small ones stacked.

use alloc::vec::Vec;
use ui::i18n::t;
use ui::*;

use crate::tc;

/// the registers, left to right (FEATURES.md 3.2, UI-31; Datei and the others follow in JD8)
pub(crate) static TABS: [&str; 4] = ["Start", "Einfügen", "Entwurf", "Ansicht"];

// a column of a group: one large button, or small buttons stacked (label, icon)
enum Col {
    Big(&'static str, u32),
    Small(&'static [(&'static str, u32)]),
}
use Col::{Big, Small};

// a group: its caption and its columns
struct Group(&'static str, &'static [Col]);

static START: [Group; 4] = [
    Group("Zwischenablage", &[
        Big("Einfügen|Zwischenablage", IC_PASTE),
        Small(&[("Ausschneiden", IC_CUT), ("Kopieren", IC_COPY), ("Format übertragen", IC_BRUSH)]),
    ]),
    Group("Tools", &[Small(&[("Zeiger", IC_POINTER), ("Verbinder", IC_LINE), ("Text", IC_TEXT_BOX)])]),
    Group("Anordnen", &[Small(&[("Ausrichten", IC_ALIGN_CENTER), ("Position", IC_MOVE), ("Gruppieren", IC_SHAPES)])]),
    Group("Bearbeiten", &[Small(&[("Suchen", IC_SEARCH), ("Ebenen", IC_LAYERS), ("Markieren", IC_SELECT_RECT)])]),
];

static INSERT: [Group; 4] = [
    Group("Seiten", &[Big("Leere Seite", IC_PAGE)]),
    Group("Illustrationen", &[Big("Bilder", IC_PICTURES)]),
    Group("Diagrammteile", &[Small(&[("Container", IC_SELECT_RECT), ("Beschriftung", IC_EDIT)])]),
    Group("Links", &[Big("Hyperlink", IC_LINK)]),
];

static DESIGN: [Group; 4] = [
    Group("Seite einrichten", &[Big("Ausrichtung", IC_ROTATE), Big("Größe", IC_FIT_PAGE)]),
    Group("Designs", &[Big("Design", IC_DIAMOND)]),
    Group("Varianten", &[Small(&[("Farben", IC_COLOR), ("Schriftarten", IC_FONT), ("Effekte", IC_BRIGHTNESS)])]),
    Group("Hintergründe", &[Big("Hintergrund", IC_IMAGE), Small(&[("Rahmen und Titel", IC_PAGE)])]),
];

static VIEW: [Group; 4] = [
    Group("Ansichten", &[Big("Vollbild", IC_FULL_SCREEN)]),
    Group("Anzeigen", &[Small(&[("Lineal", IC_RULER), ("Raster", IC_GRID), ("Führungslinien", IC_LINE)])]),
    Group("Zoom", &[Big("Zoom", IC_ZOOM), Small(&[("An Fenster anpassen", IC_FIT_PAGE), ("Seitenbreite", IC_PAGE)])]),
    Group("Fenster", &[Big("Aufgabenbereiche", IC_VIEW)]),
];

// ids: the ribbon 200, its groups 1000 + 10 per group (id..=id + 3 each), the buttons 2000 +
// 100 per register; the register's number goes into both, so a switch never reuses an id
const ID_RIBBON: u32 = 200;
const ID_GROUP: u32 = 1000;
const ID_BUTTON: u32 = 2000;
const CONTENT_H: i32 = 66; // RibbonMetrics::content_height, the height of a group's controls
const SMALL_H: i32 = 22; // a small button: three of them fill a column

impl Col {
    // the column's width in DIP for its labels in the current language
    fn width(&self, ui: &mut Ui) -> i32 {
        match self {
            Big(label, icon) => ui.ribbon_button_width(tc(label), *icon, ST_LARGE).max(48),
            Small(items) => items.iter().fold(0, |w, (label, icon)| w.max(ui.ribbon_button_width(tc(label), *icon, 0))),
        }
    }
}

/// the ribbon of register `tab` (an index of TABS)
pub(crate) fn draw(ui: &mut Ui, tab: i32) {
    let groups: &[Group] = match tab {
        1 => &INSERT,
        2 => &DESIGN,
        3 => &VIEW,
        _ => &START,
    };
    let base = tab.clamp(0, 9) as u32 * 100;
    // the widths first: translated labels make the groups as wide as they need
    let widths: Vec<Vec<i32>> = groups.iter().map(|g| g.1.iter().map(|c| c.width(ui)).collect()).collect();
    let desc: Vec<RibbonGroup> = groups
        .iter()
        .zip(&widths)
        .enumerate()
        .map(|(i, (g, w))| {
            let width = (w.iter().sum::<i32>() + 2 * w.len().saturating_sub(1) as i32).max(ui.ribbon_button_width(t(g.0), 0, 0));
            RibbonGroup::new(ID_GROUP + base + i as u32 * 10, t(g.0), width, width)
        })
        .collect();
    let layout = ui.ribbon_begin(ID_RIBBON, &desc);
    let mut id = ID_BUTTON + base;
    for (i, d) in desc.iter().enumerate() {
        let Some(mode) = ui.ribbon_group_begin(&layout, i, d) else {
            id += groups[i].1.iter().map(|c| if let Small(s) = c { s.len() as u32 } else { 1 }).sum::<u32>();
            continue;
        };
        ui.next_size(-1, CONTENT_H);
        ui.box_begin(0, BOX_H | BOX_FIXED, 2);
        for (col, &width) in groups[i].1.iter().zip(&widths[i]) {
            match col {
                Big(label, icon) => {
                    ui.next_size(width, CONTENT_H);
                    placeholder(ui, id, tc(label), *icon, ST_LARGE);
                    id += 1;
                }
                Small(items) => {
                    ui.next_size(width, CONTENT_H);
                    ui.box_begin(0, BOX_V | BOX_FIXED, 0);
                    for (label, icon) in *items {
                        ui.next_size(-1, SMALL_H);
                        placeholder(ui, id, tc(label), *icon, ST_ALIGN_START);
                        id += 1;
                    }
                    ui.box_end();
                }
            }
        }
        ui.box_end();
        ui.ribbon_group_end(mode, d.label);
    }
    ui.ribbon_end();
}

// a button of a function that does not exist yet: disabled, its tooltip says so
fn placeholder(ui: &mut Ui, id: u32, label: &str, icon: u32, flags: u32) {
    ui.ribbon_button(id, label, icon, flags | ST_DISABLED);
    ui.tooltip(t("Noch nicht verfügbar"));
}
