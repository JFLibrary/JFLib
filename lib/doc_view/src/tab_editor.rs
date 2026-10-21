//! Reusable exact-position tab editor. Values stay in twips; presentation uses points.
use alloc::{format, string::String, vec::Vec};
use doc_core::{TabAlignment, TabLeader, TabStop, TabStops};
use ui::*;

#[derive(Default)]
pub struct TabEditor { selected: Option<i32>, position: f64, alignment: i32, leader: i32, error: String }
impl TabEditor {
    pub fn new() -> Self { Self { position: 36.0, ..Default::default() } }
    pub fn reset(&mut self) { *self = Self::new(); }
    /// Reserve eight consecutive widget IDs. Clear markers are available to style
    /// editors; effective paragraph lists use only the five visible tab alignments.
    pub fn show(&mut self, ui: &mut Ui, id: u32, tabs: &mut TabStops, allow_clear: bool) -> bool {
        let mut changed = false;
        let stops = tabs.as_slice();
        let mut labels: Vec<String> = stops.iter().map(|s| format!("{:.2} pt · {} · {}", f64::from(s.position) / 20.0, ALIGNMENTS[s.alignment as usize], LEADERS[s.leader as usize])).collect();
        labels.insert(0, String::from("Neuer Tabstopp"));
        let mut selected = self.selected.and_then(|pos| stops.iter().position(|s| s.position == pos)).map_or(0, |i| i as i32 + 1);
        ui.next_size(-1, 0);
        if ui.combobox(id, &mut selected, &labels.iter().map(String::as_str).collect::<Vec<_>>()) & EV_ACT != 0 {
            self.selected = if selected == 0 { None } else {
                let s = stops[selected as usize - 1];
                self.position = f64::from(s.position) / 20.0; self.alignment = s.alignment as i32; self.leader = s.leader as i32;
                Some(s.position)
            };
        }
        ui.text(TS_CAPTION, "Position in Punkt ab linkem Seitenrand (72 pt = 1 Zoll)");
        ui.next_size(-1, 0);
        ui.number_box_f(id + 1, &mut self.position, &RangeF::new(-1584.0, 1584.0, 0.05, 2, 0));
        ui.text(TS_CAPTION, "Ausrichtung");
        ui.next_size(-1, 0);
        ui.combobox(id + 2, &mut self.alignment, &ALIGNMENTS[..if allow_clear { 6 } else { 5 }]);
        ui.text(TS_CAPTION, "Füllzeichen");
        ui.next_size(-1, 0);
        if self.alignment >= 4 { self.leader = 0; ui.disabled_push(); }
        ui.combobox(id + 3, &mut self.leader, &LEADERS);
        if self.alignment >= 4 { ui.disabled_pop(); }
        ui.box_begin(0, BOX_H, 6);
        if ui.button(id + 4, "Setzen") & EV_ACT != 0 {
            let mut candidate = *tabs;
            let position = (self.position * 20.0 + if self.position < 0.0 { -0.5 } else { 0.5 }) as i32;
            if let Some(old) = self.selected { candidate.remove(old); }
            let alignment = [TabAlignment::Left, TabAlignment::Center, TabAlignment::Right, TabAlignment::Decimal, TabAlignment::Bar, TabAlignment::Clear][self.alignment as usize];
            let leader = [TabLeader::None, TabLeader::Dot, TabLeader::Hyphen, TabLeader::Underscore, TabLeader::Heavy, TabLeader::MiddleDot][self.leader as usize];
            match candidate.set(TabStop { position, alignment, leader }) {
                Ok(()) => { *tabs = candidate; self.selected = Some(position); self.error.clear(); changed = true; },
                Err(_) => self.error = String::from("Höchstens 64 gültige Tabstopps sind möglich."),
            }
        }
        if ui.button(id + 5, "Löschen") & EV_ACT != 0 {
            if let Some(pos) = self.selected.take() { tabs.remove(pos); changed = true; self.error.clear(); }
        }
        if ui.button(id + 6, "Alle löschen") & EV_ACT != 0 { *tabs = Default::default(); self.selected = None; changed = true; self.error.clear(); }
        ui.box_end();
        ui.next_size(-1, 30);
        ui.text(TS_CAPTION | TX_WRAP, if self.error.is_empty() { "Ohne individuellen Stopp gilt das Standardraster von 36 pt." } else { &self.error });
        changed
    }
}
pub const ALIGNMENTS: [&str; 6] = ["Links", "Zentriert", "Rechts", "Dezimal", "Vertikale Linie", "Geerbten Stopp löschen"];
pub const LEADERS: [&str; 6] = ["Keine", "Punkte", "Striche", "Unterstrich", "Dicke Linie", "Mittelpunkte"];

/// Font-independent selector for the five ruler tab markers, at the caller's next rect.
pub fn tab_selector(ui: &mut Ui, id: u32, alignment: &mut i32) {
    *alignment = (*alignment).clamp(0, 4);
    if ui.button(id, "") & EV_ACT != 0 { *alignment = (*alignment + 1) % 5; }
    let r = ui.last_rect();
    let x = r[0] + r[2] / 2;
    let y = r[1] + r[3] / 2 + ui.dp(3);
    let s = ui.dp(1).max(1); let half = ui.dp(4);
    ui.draw_color(ui.color(T_TEXT));
    ui.draw_rect(x, y - ui.dp(7), s, ui.dp(7));
    let (left, width) = match *alignment { 0 => (x, half), 2 => (x - half, half), 4 => (x, s), _ => (x - half, half * 2) };
    ui.draw_rect(left, y, width, s);
    if *alignment == 3 { ui.draw_rect(x + ui.dp(3), y - ui.dp(3), s * 2, s * 2); }
    ui.tooltip(ALIGNMENTS[*alignment as usize]);
}
