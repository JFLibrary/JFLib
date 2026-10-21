//! Shared table property editor. It prepares an atomic model change, without owning a document.
use doc_core::{CellAlignment, RowHeightRule, Table, TableChange};
use ui::*;
pub struct TableProperties {
    row: usize,
    column: usize,
    width: f64,
    height: f64,
    height_rule: i32,
    allow_split: i32,
    fill: i32,
    alignment: i32,
    original_fill: Option<u32>,
}
impl TableProperties {
    pub fn new(table: &Table, row: usize, column: usize) -> Option<Self> {
        let cell = table.cell(row, column)?;
        Some(Self {
            row,
            column,
            width: f64::from(table.cell_width(row,column)?) / 20.0,
            height: f64::from(table.rows[row].min_height) / 20.0,
            height_rule: i32::from(table.rows[row].height_rule == RowHeightRule::Exact),
            allow_split: i32::from(!table.rows[row].cant_split),
            fill: 0,
            alignment: match cell.alignment {
                CellAlignment::Top => 0,
                CellAlignment::Center => 1,
                CellAlignment::Bottom => 2,
            },
            original_fill: cell.fill,
        })
    }
    /// Reserves five consecutive IDs and `id + 0x10000` for the height rule, in points.
    pub fn show(&mut self, ui: &mut Ui, id: u32) {
        ui.text(TS_CAPTION, "Zellbreite in Punkt");
        ui.next_size(-1, 0);
        ui.number_box_f(id, &mut self.width, &RangeF::new(12.0, 1584.0, 0.05, 2, 0));
        ui.text(
            TS_CAPTION,
            "Zeilenhöhe in Punkt (0 = automatisch bei Mindesthöhe)",
        );
        ui.next_size(-1, 0);
        ui.number_box_f(
            id + 1,
            &mut self.height,
            &RangeF::new(0.0, 1584.0, 0.05, 2, 0),
        );
        ui.text(TS_CAPTION, "Höhenregel");
        ui.next_size(-1, 0);
        ui.combobox(id + 0x10000, &mut self.height_rule, &["Mindestens", "Genau"]);
        if self.height_rule == 1 {
            ui.text(TS_CAPTION | TX_WRAP, "Überstehender Inhalt bleibt erhalten und wird an der Zellgrenze abgeschnitten.");
        }
        ui.text(TS_CAPTION, "Zellhintergrund");
        ui.next_size(-1, 0);
        ui.combobox(
            id + 2,
            &mut self.fill,
            &[
                "Beibehalten",
                "Aus Tabellenformatvorlage",
                "Blau",
                "Hellblau",
                "Hellgrau",
                "Hellgelb",
                "Hellgrün",
                "Weiß",
            ],
        );
        ui.text(TS_CAPTION, "Vertikale Ausrichtung");
        ui.next_size(-1, 0);
        ui.combobox(id + 3, &mut self.alignment, &["Oben", "Zentriert", "Unten"]);
        ui.checkbox(id + 4, "Zeilenwechsel auf Seiten zulassen", &mut self.allow_split);
    }
    pub fn change(&self) -> TableChange {
        TableChange::Properties {
            row: self.row,
            column: self.column,
            width: (self.width * 20.0 + 0.5) as u32,
            height: (self.height * 20.0 + 0.5) as u32,
            height_rule: if self.height_rule == 1 { RowHeightRule::Exact } else { RowHeightRule::AtLeast },
            cant_split: self.allow_split == 0,
            fill: match self.fill {
                0 => self.original_fill,
                1 => None,
                2 => Some(0x1b607c),
                3 => Some(0xc2e4f2),
                4 => Some(0xe7e7e7),
                5 => Some(0xfff2cc),
                6 => Some(0xe2efda),
                _ => Some(0xffffff),
            },
            alignment: match self.alignment {
                1 => CellAlignment::Center,
                2 => CellAlignment::Bottom,
                _ => CellAlignment::Top,
            },
        }
    }
}
