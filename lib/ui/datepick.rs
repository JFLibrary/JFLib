// datepick -- date and time input
//
//   time_box(id, &mut hour, &mut minute) -> EV    two number boxes "HH : MM" (EV_ACT changed)
//   weekday_toggles(id, &mut mask) -> EV          seven toggle buttons, bit 0 = Monday ..
//                                                 bit 6 = Sunday (EV_ACT changed); they wrap
//                                                 where the row is narrower (BOX_WRAP)
//   date_picker(id, &mut y, &mut m, &mut d) -> EV a button with the date (the user's format,
//                                                 i18n::fmt_date) that opens a month calendar
//                                                 starting on the locale's first weekday
//                                                 (EV_ACT when a day was picked; it closes the
//                                                 calendar only, a flyout around it stays)
//   calendar(id, &mut y, &mut m, &mut d, &DateRange) -> EV
//                                                 the calendar in place, for any panel or flyout:
//                                                 a month of days (CAL_DAY), of ISO weeks
//                                                 (CAL_WEEK: a row is a week, the date its
//                                                 Monday) or a year of months (CAL_MONTH: the 1st);
//                                                 only dates between min and max can be picked
//                                                 (EV_ACT when the user picked)
//   The civil helpers (days in a month, weekday, ISO weeks) are jflib::datetime's.
//
// Ids: parts (id << 3 | k) | 0x80000000; calendar cells ((id ^ 0x3C3C) << 6 | cell) | 0x80000000
// (a day 0 .. 41, a week row 0 .. 5, a month 0 .. 11).

use alloc::format;

use crate::consts::*;
use crate::i18n;
use crate::keys::*;
use crate::{Sp, Ui};

/// what a calendar picks (DateRange::mode): a day
pub const CAL_DAY: u32 = 0;
/// an ISO 8601 week (Monday to Sunday): the date is its Monday, the rows start on Monday and show
/// the week's number (datetime::iso_week)
pub const CAL_WEEK: u32 = 1;
/// a month: the date is its 1st, the calendar shows the twelve months of a year
pub const CAL_MONTH: u32 = 2;

/// what a calendar picks and between which dates (calendar): `mode` CAL_DAY, CAL_WEEK or
/// CAL_MONTH; min and max (year, month, day) inclusive -- in CAL_WEEK the weeks that hold them, in
/// CAL_MONTH their months
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateRange {
    pub mode: u32,
    pub min: (i32, u32, u32),
    pub max: (i32, u32, u32),
}

impl DateRange {
    /// days of the years 1 to 9999
    pub const DAYS: DateRange = DateRange::new(CAL_DAY, (1, 1, 1), (9999, 12, 31));

    pub const fn new(mode: u32, min: (i32, u32, u32), max: (i32, u32, u32)) -> DateRange {
        DateRange { mode, min, max }
    }

    // a date in the mode's unit: its day, the Monday of its week, its month
    fn key(&self, (y, m, d): (i32, u32, u32)) -> i64 {
        match self.mode {
            CAL_WEEK => days(y, m, d) - wday(y, m, d) as i64,
            CAL_MONTH => y as i64 * 12 + m as i64 - 1,
            _ => days(y, m, d),
        }
    }

    // the date can be picked
    fn has(&self, date: (i32, u32, u32)) -> bool {
        let k = self.key(date);
        self.key(self.min) <= k && k <= self.key(self.max)
    }
}

fn part(id: u32, k: u32) -> u32 {
    id.wrapping_mul(8).wrapping_add(k) | 0x8000_0000
}

fn cell_id(id: u32, cell: u32) -> u32 {
    ((id ^ 0x3C3C).wrapping_mul(64).wrapping_add(cell)) | 0x8000_0000
}

// the civil helpers of datetime with the widgets' types (month and day u32)
fn dim(y: i32, m: u32) -> u32 {
    datetime::days_in_month(y, m.clamp(1, 12) as u8) as u32
}

fn days(y: i32, m: u32, d: u32) -> i64 {
    datetime::days_from_civil(y, m as u8, d.min(31) as u8)
}

fn wday(y: i32, m: u32, d: u32) -> u32 {
    datetime::weekday(y, m as u8, d.min(31) as u8) as u32
}

fn civil(z: i64) -> (i32, u32, u32) {
    let (y, m, d) = datetime::civil_from_days(z);
    (y, m as u32, d as u32)
}

impl Ui {
    /// ui_time_box(id, &mut hour, &mut minute) -> EV_* (EV_ACT changed)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn time_box(&mut self, id: u32, hour: &mut i32, minute: &mut i32) -> u32 {
        let _src = self.src_scope();
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

    /// ui_weekday_toggles(id, &mut mask) -> EV_* (EV_ACT changed); bit 0 = Monday.  The
    /// buttons wrap where the row has less room than the seven of them (a narrow card)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn weekday_toggles(&mut self, id: u32, mask: &mut u32) -> u32 {
        let _src = self.src_scope();
        let old = *mask;
        self.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 4);
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

    /// ui_date_picker(id, &mut y, &mut m, &mut d) -> EV_* (EV_ACT when a day was picked).  A
    /// pick closes the calendar's popup and the popups inside it, not a flyout the picker sits
    /// in (popup_close_id)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn date_picker(&mut self, id: u32, y: &mut i32, m: &mut u32, d: &mut u32) -> u32 {
        let _src = self.src_scope();
        // the date in the user's format
        let label = i18n::fmt_date(&datetime::Civil { year: *y, month: *m as u8, day: *d as u8, ..Default::default() }, i18n::DATE_SHORT);
        self.next_icon(IC_CALENDAR);
        let ev = self.dropdown_button(id, &label);
        let mut out = ev & !EV_ACT;
        // closed: the calendar shows the date's month when it opens again
        if ev & EV_OPEN == 0 {
            self.slot(part(id, 7)).aux = 0;
        }
        if self.flyout_begin(id, 0) {
            if self.calendar(id, y, m, d, &DateRange::DAYS) & EV_ACT != 0 {
                out |= EV_ACT;
                self.popup_close_id(id);
            }
            // today
            self.next_flags(ST_SUBTLE | ST_COMPACT);
            if self.button(part(id, 3), i18n::tr("Heute")) & EV_ACT != 0 {
                let today = plat::date().0;
                *y = today.year as i32;
                *m = today.month as u32;
                *d = today.day as u32;
                out |= EV_ACT;
                self.popup_close_id(id);
                self.repass();
            }
        }
        self.flyout_end();
        out
    }

    /// ui_calendar(id, &mut y, &mut m, &mut d, &range) -> EV_* (EV_ACT when the user picked):
    /// a month calendar in place, no popup -- a panel's, or the content of a flyout of the app's
    /// own.  range.mode picks a day (CAL_DAY: the weeks start on the locale's first weekday), an
    /// ISO week (CAL_WEEK: rows from Monday with the week's number; y, m, d become its Monday) or
    /// a month (CAL_MONTH: a year of months; y, m, d become its 1st); what lies outside
    /// range.min .. range.max is drawn disabled.  The arrows show the month (the year) before and
    /// after; the calendar shows the month of y, m, d again when the app changes the date.
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn calendar(&mut self, id: u32, y: &mut i32, m: &mut u32, d: &mut u32, range: &DateRange) -> u32 {
        let _src = self.src_scope();
        let months = range.mode == CAL_MONTH;
        // the view (slot scrollx: year * 12 + month - 1, or the year in CAL_MONTH) follows the
        // date whenever it or the mode changes (scrolly: the date's day number; aux: the mode + 1,
        // 0 not set)
        let vid = part(id, 7);
        let now = days(*y, *m, *d) as i32;
        let tag = range.mode as i32 + 1;
        {
            let s = self.slot(vid);
            if s.aux != tag || s.scrolly != now {
                s.scrollx = if months { *y } else { *y * 12 + *m as i32 - 1 };
                s.scrolly = now;
                s.aux = tag;
            }
        }
        let mut view = self.slot(vid).scrollx;
        let (vy, vm) = if months { (view, 1) } else { (view.div_euclid(12), (view.rem_euclid(12) + 1) as u32) };
        // header: previous, the month (the year), next; an arrow is off where nothing could be
        // picked beyond it
        let (before, after) = if months {
            (range.key((vy - 1, 12, 31)) < range.key(range.min), range.key((vy + 1, 1, 1)) > range.key(range.max))
        } else {
            let (py, pm) = if vm == 1 { (vy - 1, 12) } else { (vy, vm - 1) };
            let (ny, nm) = if vm == 12 { (vy + 1, 1) } else { (vy, vm + 1) };
            (range.key((py, pm, dim(py, pm))) < range.key(range.min), range.key((ny, nm, 1)) > range.key(range.max))
        };
        let prev = if months { i18n::tr("Vorheriges Jahr") } else { i18n::tr("Vorheriger Monat") };
        let next = if months { i18n::tr("Nächstes Jahr") } else { i18n::tr("Nächster Monat") };
        self.box_begin(0, BOX_H | BOX_XCENTER, 4);
        self.next_icon(IC_CHEV_LEFT);
        self.next_flags(ST_SUBTLE | ST_ICON_ONLY | if before { ST_DISABLED } else { 0 });
        if self.button(part(id, 1), prev) & EV_ACT != 0 {
            view -= 1;
        }
        self.next_size(-1, 0);
        let title = if months { format!("{vy}") } else { format!("{} {}", i18n::month_name(vm), vy) };
        self.text(TS_BODY_STRONG | TX_CENTER, &title);
        self.next_icon(IC_CHEV_RIGHT);
        self.next_flags(ST_SUBTLE | ST_ICON_ONLY | if after { ST_DISABLED } else { 0 });
        if self.button(part(id, 2), next) & EV_ACT != 0 {
            view += 1;
        }
        self.box_end();
        if view != self.slot(vid).scrollx {
            self.slot(vid).scrollx = view;
            self.repass();
        }
        let picked = if months { self.cal_months(id, vy, (*y, *m, *d), range) } else { self.cal_days(id, vy, vm, (*y, *m, *d), range) };
        match picked {
            Some(date) => {
                (*y, *m, *d) = date;
                self.repass();
                EV_ACT
            }
            None => 0,
        }
    }

    // the weekday header and the 6 x 7 days of month vm of year vy (CAL_DAY, CAL_WEEK) -> the
    // date picked
    fn cal_days(&mut self, id: u32, vy: i32, vm: u32, value: (i32, u32, u32), range: &DateRange) -> Option<(i32, u32, u32)> {
        let weeks = range.mode == CAL_WEEK;
        // ISO weeks start on Monday; days on the locale's first weekday
        let fw = if weeks { 0 } else { i18n::locale().first_weekday() };
        let cw = self.dp(36);
        let ch = self.dp(32);
        // CAL_WEEK: a column of week numbers before the days
        let x0 = if weeks { cw } else { 0 };
        let grid = self.place(x0 + cw * 7, ch * 7);
        let t3 = self.color(T_TEXT3);
        for c in 0..7u32 {
            let s = i18n::weekday_short((c + fw) % 7);
            self.ts_font(TS_CAPTION);
            let tw = self.text.width(s);
            let x = grid[0] + x0 + cw * c as i32;
            crate::stock::text_at(self, TS_CAPTION, x + ((cw - tw) >> 1), [x, grid[1], cw, ch], s, t3, 0);
        }
        // the first cell: the day on or before the 1st that starts a row
        let start = days(vy, vm, 1) - ((wday(vy, vm, 1) + 7 - fw) % 7) as i64;
        let today = plat::date().0;
        let today = (today.year as i32, today.month as u32, today.day as u32);
        let sel_week = range.key(value);
        let sh = self.style.shell();
        let mut picked = None;
        for row in 0..6i32 {
            let ry = grid[1] + ch * (1 + row);
            let monday = civil(start + row as i64 * 7);
            // CAL_WEEK: the row is one target: its hover and press, the week marked
            let (wev, wsel) = if weeks {
                let rid = cell_id(id, row as u32);
                let on = range.has(monday);
                let ev = self.hit_rect(rid, grid[0], ry, x0 + cw * 7, ch, if on { 0 } else { HB_DISABLED });
                let (wy, wn) = datetime::iso_week(monday.0, monday.1 as u8, monday.2 as u8);
                let sel = range.key(monday) == sel_week;
                if sel {
                    self.wp.ctx.last_state |= SS_SELECTED as u32;
                }
                let name = format!("{wy}-W{wn:02}");
                self.sem(K_BUTTON, rid, Some(&name), wn as i32);
                // the row's band: the accent when chosen, the hover's colour under the pointer
                let band = if sel { self.color(T_ACCENT) } else if ev & EV_HOVER != 0 { self.color(T_SUBTLE_HOVER) } else { 0 };
                if band != 0 {
                    self.cmd_rrect(grid[0], ry + self.dp(2), x0 + cw * 7, ch - self.dp(4), (ch - self.dp(4)) >> 1, 0, band, 0, 0);
                }
                let num = format!("{wn}");
                self.ts_font(TS_CAPTION);
                let tw = self.text.width(&num);
                let c = if sel { self.color(T_ON_ACCENT) } else { t3 };
                crate::stock::text_at(self, TS_CAPTION, grid[0] + ((cw - tw) >> 1), [grid[0], ry, cw, ch], &num, c, 0);
                if ev & EV_ACT != 0 && on {
                    picked = Some(monday);
                }
                (ev & !EV_ACT, sel)
            } else {
                (0, false)
            };
            for col in 0..7i32 {
                let cell = (row * 7 + col) as u32;
                let date = civil(start + cell as i64);
                let other = (date.0, date.1) != (vy, vm);
                let on = range.has(date);
                let r = [grid[0] + x0 + cw * col, ry, cw, ch];
                let s = format!("{}", date.2);
                let (dev, selected) = if weeks {
                    (wev, wsel)
                } else {
                    let did = cell_id(id, cell);
                    let dev = self.hit_rect(did, r[0], r[1], r[2], r[3], if on { 0 } else { HB_DISABLED });
                    let selected = date == value;
                    if selected {
                        self.wp.ctx.last_state |= SS_SELECTED as u32;
                    }
                    self.sem(K_BUTTON, did, Some(&s), date.2 as i32);
                    if dev & EV_ACT != 0 && on {
                        picked = Some(date);
                    }
                    (dev, selected)
                };
                let mut sp = Sp { ev: dev, value: selected as i32, value2: (date == today) as i32, flags: other as u32, label: Some(&s), ..Sp::default() };
                sp.state = if on { 0 } else { 3 };
                sp.place(r);
                sh.calendar_day(self, &sp);
            }
        }
        picked
    }

    // the twelve months of year vy, three in a row (CAL_MONTH) -> the date picked (the 1st)
    fn cal_months(&mut self, id: u32, vy: i32, value: (i32, u32, u32), range: &DateRange) -> Option<(i32, u32, u32)> {
        let (cw, ch) = (self.dp(36) * 7 / 3, self.dp(48));
        let grid = self.place(cw * 3, ch * 4);
        let today = plat::date().0;
        let sh = self.style.shell();
        let mut picked = None;
        for k in 0..12u32 {
            let date = (vy, k + 1, 1);
            let on = range.has(date);
            let r = [grid[0] + cw * (k % 3) as i32, grid[1] + ch * (k / 3) as i32, cw, ch];
            let mid = cell_id(id, k);
            let ev = self.hit_rect(mid, r[0], r[1], r[2], r[3], if on { 0 } else { HB_DISABLED });
            let selected = (value.0, value.1) == (vy, k + 1);
            if selected {
                self.wp.ctx.last_state |= SS_SELECTED as u32;
            }
            let name = format!("{} {vy}", i18n::month_name(k + 1));
            self.sem(K_BUTTON, mid, Some(&name), (k + 1) as i32);
            let is_now = (today.year as i32, today.month as u32) == (vy, k + 1);
            let mut sp = Sp { ev, value: selected as i32, value2: is_now as i32, label: Some(i18n::month_short(k + 1)), ..Sp::default() };
            sp.state = if on { 0 } else { 3 };
            sp.place(r);
            sh.calendar_day(self, &sp);
            if ev & EV_ACT != 0 && on {
                picked = Some(date);
            }
        }
        picked
    }
}
