// datepick -- date and time input
//
//   time_box(id, &mut hour, &mut minute) -> EV    two number boxes "HH : MM" (EV_ACT changed)
//   weekday_toggles(id, &mut mask) -> EV          seven toggle buttons, bit 0 = Monday ..
//                                                 bit 6 = Sunday (EV_ACT changed)
//   date_picker(id, &mut y, &mut m, &mut d) -> EV a button with the date (the user's format,
//                                                 i18n::fmt_date) that opens a month calendar
//                                                 starting on the locale's first weekday
//                                                 (EV_ACT when a day was picked)
//   civil helpers: days_in_month(y, m), weekday(y, m, d) (0 = Monday)
//
// Ids: parts (id << 3 | k) | 0x80000000; calendar days ((id ^ 0x3C3C) << 6 | cell) | 0x80000000.

use alloc::format;

use crate::consts::*;
use crate::i18n;
use crate::keys::*;
use crate::{Sp, Ui};

fn part(id: u32, k: u32) -> u32 {
    id.wrapping_mul(8).wrapping_add(k) | 0x8000_0000
}

/// days of month m (1..12) of year y (proleptic Gregorian)
pub fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
    }
}

/// days since 1970-01-01 of a civil date (Howard Hinnant's algorithm)
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// weekday of a civil date, 0 = Monday .. 6 = Sunday
pub fn weekday(y: i32, m: u32, d: u32) -> u32 {
    // 1970-01-01 was a Thursday (3)
    (days_from_civil(y, m, d) + 3).rem_euclid(7) as u32
}

impl Ui {
    /// ui_time_box(id, &mut hour, &mut minute) -> EV_* (EV_ACT changed)
    pub fn time_box(&mut self, id: u32, hour: &mut i32, minute: &mut i32) -> u32 {
        let (h0, m0) = (*hour, *minute);
        self.box_begin(0, BOX_H | BOX_XCENTER, 4);
        self.next_size(64, 0);
        let e1 = self.number_box(part(id, 1), hour, &Range::new(0, 23, 1, 0));
        self.text(TS_BODY_STRONG, ":");
        self.next_size(64, 0);
        let e2 = self.number_box(part(id, 2), minute, &Range::new(0, 59, 1, 0));
        self.box_end();
        let ev = (e1 | e2) & !EV_ACT;
        if *hour != h0 || *minute != m0 { ev | EV_ACT } else { ev }
    }

    /// ui_weekday_toggles(id, &mut mask) -> EV_* (EV_ACT changed); bit 0 = Monday
    pub fn weekday_toggles(&mut self, id: u32, mask: &mut u32) -> u32 {
        let old = *mask;
        self.box_begin(0, BOX_H | BOX_XCENTER, 4);
        for d in 0..7u32 {
            let on = *mask & (1 << d) != 0;
            self.next_size(40, 0);
            self.next_flags(ST_COMPACT | if on { ST_CHECKED | ST_ACCENT } else { 0 });
            if self.button(part(id, 1 + d), i18n::weekday_short(d)) & EV_ACT != 0 {
                *mask ^= 1 << d;
                self.repass();
            }
        }
        self.box_end();
        if *mask != old { EV_ACT } else { 0 }
    }

    /// ui_date_picker(id, &mut y, &mut m, &mut d) -> EV_* (EV_ACT when a day was picked)
    pub fn date_picker(&mut self, id: u32, y: &mut i32, m: &mut u32, d: &mut u32) -> u32 {
        // the date in the user's format; the week starts on the locale's first day
        let label = i18n::fmt_date(&datetime::Civil { year: *y, month: *m as u8, day: *d as u8, ..Default::default() }, i18n::DATE_SHORT);
        let fw = i18n::locale().first_weekday();
        self.next_icon(IC_CALENDAR);
        let ev = self.dropdown_button(id, &label);
        let mut out = ev & !EV_ACT;
        // the month shown: slot scrollx = year * 12 + month - 1 (set when the popup opens)
        let vid = part(id, 7);
        if ev & EV_OPEN == 0 {
            self.slot(vid).scrollx = *y * 12 + *m as i32 - 1;
        }
        if self.flyout_begin(id, 0) {
            let mut view = self.slot(vid).scrollx;
            let (vy, vm) = (view.div_euclid(12), (view.rem_euclid(12) + 1) as u32);
            // header: previous, month name, next
            self.box_begin(0, BOX_H | BOX_XCENTER, 4);
            self.next_icon(IC_CHEV_LEFT);
            self.next_flags(ST_SUBTLE | ST_ICON_ONLY);
            if self.button(part(id, 1), i18n::tr("Vorheriger Monat")) & EV_ACT != 0 {
                view -= 1;
            }
            self.next_size(-1, 0);
            let title = format!("{} {}", i18n::month_name(vm), vy);
            self.text(TS_BODY_STRONG | TX_CENTER, &title);
            self.next_icon(IC_CHEV_RIGHT);
            self.next_flags(ST_SUBTLE | ST_ICON_ONLY);
            if self.button(part(id, 2), i18n::tr("Nächster Monat")) & EV_ACT != 0 {
                view += 1;
            }
            self.box_end();
            if view != self.slot(vid).scrollx {
                self.slot(vid).scrollx = view;
                self.repass();
            }
            // weekday header and the 6 x 7 day grid
            let cw = self.dp(36);
            let ch = self.dp(32);
            let grid = self.place(cw * 7, ch * 7);
            let t3 = self.color(T_TEXT3);
            for c in 0..7u32 {
                let s = i18n::weekday_short((c + fw) % 7);
                self.ts_font(TS_CAPTION);
                let tw = self.text.width(s);
                let x = grid[0] + cw * c as i32;
                crate::stock::text_at(self, TS_CAPTION, x + ((cw - tw) >> 1), [x, grid[1], cw, ch], s, t3, 0);
            }
            let first = ((weekday(vy, vm, 1) + 7 - fw) % 7) as i32;
            let dim = days_in_month(vy, vm) as i32;
            let today = plat::date().0;
            let sh = self.style.shell();
            for cell in 0..42i32 {
                let day = cell - first + 1;
                let (cy, cm, cd, other) = if day < 1 {
                    let (py, pm) = if vm == 1 { (vy - 1, 12) } else { (vy, vm - 1) };
                    (py, pm, (days_in_month(py, pm) as i32 + day) as u32, true)
                } else if day > dim {
                    let (ny, nm) = if vm == 12 { (vy + 1, 1) } else { (vy, vm + 1) };
                    (ny, nm, (day - dim) as u32, true)
                } else {
                    (vy, vm, day as u32, false)
                };
                let r = [grid[0] + cw * (cell % 7), grid[1] + ch * (1 + cell / 7), cw, ch];
                let did = ((id ^ 0x3C3C).wrapping_mul(64).wrapping_add(cell as u32)) | 0x8000_0000;
                let dev = self.hit_rect(did, r[0], r[1], r[2], r[3], 0);
                let selected = cy == *y && cm == *m && cd == *d;
                let is_today = today.year as i32 == cy && today.month as u32 == cm && today.day as u32 == cd;
                let s = format!("{}", cd);
                let mut sp = Sp { ev: dev, value: selected as i32, value2: is_today as i32, flags: other as u32, label: Some(&s), ..Sp::default() };
                sp.place(r);
                sh.calendar_day(self, &sp);
                if selected {
                    self.wp.ctx.last_state |= SS_SELECTED as u32;
                }
                self.sem(K_BUTTON, did, Some(&s), cd as i32);
                if dev & EV_ACT != 0 {
                    *y = cy;
                    *m = cm;
                    *d = cd;
                    out |= EV_ACT;
                    self.popup_close();
                    self.repass();
                }
            }
            // today
            self.next_flags(ST_SUBTLE | ST_COMPACT);
            if self.button(part(id, 3), i18n::tr("Heute")) & EV_ACT != 0 {
                *y = today.year as i32;
                *m = today.month as u32;
                *d = today.day as u32;
                out |= EV_ACT;
                self.popup_close();
                self.repass();
            }
        }
        self.flyout_end();
        out
    }
}
