// schedule -- the schedule editor: manual / hourly / daily / weekly / monthly / cron, time,
// weekdays, day of month, the time zone (searchable), and a preview: the schedule in words
// and the next five runs (the scheduler's own rules: a time in a DST gap runs right after
// it, a repeated time once)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::model::Schedule;
use datetime::cron::{self, Cron};
use datetime::{Timestamp, Zone};
use ui::*;

use crate::tr::{en, t};

/// the editor's state
#[derive(Clone)]
pub struct Sched {
    /// 0 manual, 1 hourly, 2 daily, 3 weekly, 4 monthly, 5 cron
    pub kind: i32,
    pub hour: i32,
    pub minute: i32,
    /// bit 0 = Monday
    pub days: u32,
    pub dom: i32,
    pub cron: String,
    pub tz: String,
    pub tz_filter: String,
    pub enabled: bool,
}

impl Default for Sched {
    fn default() -> Sched {
        Sched { kind: 2, hour: 2, minute: 0, days: 0b0011111, dom: 1, cron: String::from("0 2 * * *"), tz: String::from("Europe/Berlin"),
                tz_filter: String::new(), enabled: true }
    }
}

impl Sched {
    /// the editor state of a stored schedule (an expression the choices cannot show: cron)
    pub fn from_schedule(s: &Schedule) -> Sched {
        let mut e = Sched { tz: if s.tz.is_empty() { String::from("Europe/Berlin") } else { s.tz.clone() }, enabled: s.enabled, ..Sched::default() };
        if s.cron.is_empty() {
            e.kind = 0;
            return e;
        }
        e.cron = s.cron.clone();
        match Cron::parse(&s.cron) {
            Ok(c) => {
                let (kind, h, m, mask, dom) = cron::classify(&c);
                e.kind = match kind {
                    1 => 1,
                    2 => 2,
                    3 => 3,
                    4 => 4,
                    _ => 5,
                };
                e.hour = h as i32;
                e.minute = m as i32;
                if kind == 3 {
                    e.days = mask as u32;
                }
                if kind == 4 {
                    e.dom = dom as i32;
                }
            }
            Err(_) => e.kind = 5,
        }
        e
    }

    /// the cron expression of the choices ("" = manual)
    pub fn expr(&self) -> String {
        let (h, m) = (self.hour.clamp(0, 23) as u8, self.minute.clamp(0, 59) as u8);
        match self.kind {
            0 => String::new(),
            1 => cron::hourly(m),
            2 => cron::daily(h, m),
            3 => cron::weekly(self.days as u8, h, m),
            4 => cron::monthly(self.dom.clamp(1, 31) as u8, h, m),
            _ => String::from(self.cron.trim()),
        }
    }

    pub fn to_schedule(&self) -> Schedule {
        Schedule { cron: self.expr(), tz: self.tz.clone(), enabled: self.enabled && self.kind != 0 }
    }

    /// the schedule in words
    pub fn describe(&self) -> String {
        let e = self.expr();
        if e.is_empty() {
            return String::from(t("Nur manuell"));
        }
        match Cron::parse(&e) {
            Ok(c) => c.describe(en()),
            Err(_) => String::from(t("Ungültiger Ausdruck")),
        }
    }

    /// an error of the expression ("" = fine)
    pub fn error(&self) -> String {
        let e = self.expr();
        if e.is_empty() {
            return String::new();
        }
        match Cron::parse(&e) {
            Ok(_) => {
                if Zone::by_name(&self.tz).is_none() {
                    String::from(t("Unbekannte Zeitzone"))
                } else {
                    String::new()
                }
            }
            Err(err) => {
                let field = ["Minute", "Stunde", "Tag des Monats", "Monat", "Wochentag", "Ausdruck"][err.field.min(5) as usize];
                format!("{}: {}", t(field), t(match err.msg {
                    "five fields expected" => "Fünf Felder erwartet",
                    "value out of range" => "Wert außerhalb des Bereichs",
                    "range runs backwards" => "Bereich ist rückwärts",
                    "bad step" => "Ungültige Schrittweite",
                    _ => "Zahl erwartet",
                }))
            }
        }
    }
}

/// the editor; now = seconds since the epoch (for the preview)
pub fn editor(ui: &mut Ui, id: u32, s: &mut Sched, now: i64) {
    ui.box_begin(0, BOX_V, 14);
    let kinds = [t("Manuell"), t("Stündlich"), t("Täglich"), t("Wöchentlich"), t("Monatlich"), t("Cron")];
    crate::widgets::inline_begin(ui);
    ui.segmented(id + 1, &mut s.kind, &kinds); // the segments: id + 2 .. id + 7
    crate::widgets::inline_end(ui);
    match s.kind {
        0 => {
            ui.text(TS_BODY | TX_SECONDARY | TX_WRAP, t("Der Job läuft nur, wenn er von Hand oder über bcctl gestartet wird."));
        }
        1 => {
            ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
            ui.text(TS_BODY, t("Jede Stunde zur Minute"));
            ui.next_size(80, 0);
            ui.number_box(id + 20, &mut s.minute, &Range::new(0, 59, 1, 0));
            ui.box_end();
        }
        2..=4 => {
            ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
            ui.text(TS_BODY, t("Uhrzeit"));
            ui.time_box(id + 21, &mut s.hour, &mut s.minute);
            if s.kind == 4 {
                ui.spacer(12);
                ui.text(TS_BODY, t("am Tag"));
                ui.next_size(80, 0);
                ui.number_box(id + 22, &mut s.dom, &Range::new(1, 31, 1, 0));
            }
            ui.box_end();
            if s.kind == 3 {
                ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
                ui.text(TS_BODY, t("An diesen Tagen"));
                ui.weekday_toggles(id + 23, &mut s.days);
                ui.box_end();
            }
            if s.kind == 4 && s.dom > 28 {
                ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Monate ohne diesen Tag werden übersprungen."));
            }
        }
        _ => {
            ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
            ui.text(TS_BODY, t("Ausdruck"));
            ui.next_size(260, 0);
            ui.textbox(id + 24, Some("30 2 * * 1-5"), &mut s.cron, 120);
            ui.box_end();
            ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, t("Minute Stunde Tag Monat Wochentag; Namen (mon, jan), Bereiche (1-5), Schritte (*/15) und Listen (1,15) sind erlaubt."));
        }
    }
    // time zone
    ui.box_begin(0, BOX_H | BOX_XCENTER, 10);
    ui.text(TS_BODY, t("Zeitzone"));
    zone_picker(ui, id + 10, s);
    ui.box_end();
    // preview
    ui.box_begin(0, BOX_V | BOX_LAYER | pad(14), 6);
    let err = s.error();
    if !err.is_empty() {
        crate::widgets::error(ui, &err);
    } else {
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        ui.icon(crate::icons::BC_CLOCK, 16);
        ui.text(TS_BODY_STRONG, &s.describe());
        ui.box_end();
        let e = s.expr();
        if !e.is_empty() {
            let zone = Zone::by_name(&s.tz).unwrap_or_else(Zone::utc);
            if let Ok(c) = Cron::parse(&e) {
                ui.text(TS_CAPTION | TX_SECONDARY, t("Nächste Läufe"));
                let runs = c.next_n(Timestamp::from_secs(now), &zone, 5);
                for r in runs {
                    let local = zone.to_local(r);
                    let ab = zone.abbrev_at(r.secs);
                    let wd = ui::i18n::weekday_short(local.weekday() as u32);
                    let line = format!("{} {}  {}", wd, datetime::format_date_time(r, &zone, en()), ab);
                    ui.text(TS_BODY | TX_SECONDARY, &line);
                }
                ui.text(TS_CAPTION | TX_TERTIARY | TX_WRAP, t("Zur Zeitumstellung: Eine ausgefallene Uhrzeit läuft direkt danach, eine doppelte Uhrzeit nur einmal."));
            }
        }
    }
    ui.box_end();
    ui.box_end();
}

/// a drop-down with the zone name that opens a searchable zone list
fn zone_picker(ui: &mut Ui, id: u32, s: &mut Sched) {
    ui.next_size(260, 0);
    ui.next_icon(IC_GLOBE);
    let label = s.tz.clone();
    ui.dropdown_button(id, &label);
    if ui.flyout_begin(id, FL_MINW) {
        ui.next_size(300, 0);
        ui.textbox_icon(id + 1, IC_SEARCH, Some(t("Zeitzone suchen")), &mut s.tz_filter, 64);
        let f = s.tz_filter.to_lowercase();
        let names: Vec<&str> = Zone::names().filter(|n| f.is_empty() || n.to_lowercase().contains(f.as_str())).take(400).collect();
        let mut sel = names.iter().position(|n| *n == s.tz).map_or(-1, |p| p as i32);
        let old = sel;
        ui.next_size(300, 260);
        let (a, b) = ui.list_begin(id + 100, &mut sel, names.len() as i32, 30);
        for i in a..b {
            ui.list_item(i, names[i as usize]);
        }
        let (ev, v) = ui.list_end(&mut sel);
        if (ev & EV_ACT != 0 || v != old) && v >= 0 && (v as usize) < names.len() {
            s.tz = String::from(names[v as usize]);
            ui.popup_close();
        }
    }
    ui.flyout_end();
}
