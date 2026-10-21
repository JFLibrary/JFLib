// datetime -- calendar time: timestamps, civil dates, RFC 3339, time zones, cron schedules
//
//   Timestamp { secs, nanos }          seconds since 1970-01-01 00:00 UTC
//   Civil { year .. second, nanos }    a calendar date and wall time (no zone)
//   days_from_civil / civil_from_days / days_in_month / is_leap / weekday (0 = Monday)
//   rfc3339(ts) / rfc3339_offset(ts, offset) / parse_rfc3339(s)
//   format_date_time(ts, &zone, en) / format_date / format_time / format_duration(secs, en)
//   tz::Zone                           IANA zones with their current DST rule (zones.rs from
//                                      tools/gen_tz.py), Windows zone keys, POSIX TZ strings;
//                                      offset_at / to_local / from_local (Single, Ambiguous,
//                                      Gap)
//   cron::Cron                         5-field cron expressions: next_after in a zone (a time
//                                      in a DST gap runs right after the gap, a repeated time
//                                      runs once), next_n, describe (de / en)
//
// No clock here: plat::unix_time_ns gives "now"; datetime only computes.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod cron;
pub mod tz;
mod zones;

use alloc::format;
use alloc::string::String;

pub use cron::{Cron, CronError};
pub use tz::{LocalResult, Zone};

/// a point in time: seconds since 1970-01-01 00:00:00 UTC and nanoseconds
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default, Hash)]
pub struct Timestamp {
    pub secs: i64,
    pub nanos: u32,
}

impl Timestamp {
    pub const fn from_secs(secs: i64) -> Timestamp {
        Timestamp { secs, nanos: 0 }
    }
    /// from nanoseconds since the epoch (plat::unix_time_ns)
    pub const fn from_nanos(ns: i64) -> Timestamp {
        Timestamp { secs: ns.div_euclid(1_000_000_000), nanos: ns.rem_euclid(1_000_000_000) as u32 }
    }
    pub const fn from_millis(ms: i64) -> Timestamp {
        Timestamp { secs: ms.div_euclid(1000), nanos: (ms.rem_euclid(1000) * 1_000_000) as u32 }
    }
    pub const fn as_millis(&self) -> i64 {
        self.secs * 1000 + (self.nanos / 1_000_000) as i64
    }
    pub const fn add_secs(&self, s: i64) -> Timestamp {
        Timestamp { secs: self.secs + s, nanos: self.nanos }
    }
}

/// a calendar date and wall time without a zone
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default, Hash)]
pub struct Civil {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub nanos: u32,
}

impl Civil {
    pub const fn new(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Civil {
        Civil { year, month, day, hour, minute, second, nanos: 0 }
    }
    pub const fn date(year: i32, month: u8, day: u8) -> Civil {
        Civil::new(year, month, day, 0, 0, 0)
    }
    /// the civil time of seconds since the epoch read as UTC
    pub fn from_unix(secs: i64) -> Civil {
        let days = secs.div_euclid(86_400);
        let rem = secs.rem_euclid(86_400);
        let (y, m, d) = civil_from_days(days);
        Civil::new(y, m, d, (rem / 3600) as u8, (rem % 3600 / 60) as u8, (rem % 60) as u8)
    }
    /// seconds since the epoch of this wall time read as UTC
    pub fn to_unix(&self) -> i64 {
        days_from_civil(self.year, self.month, self.day) * 86_400
            + self.hour as i64 * 3600
            + self.minute as i64 * 60
            + self.second as i64
    }
    /// 0 = Monday .. 6 = Sunday
    pub fn weekday(&self) -> u8 {
        weekday(self.year, self.month, self.day)
    }
    pub fn is_valid(&self) -> bool {
        (1..=12).contains(&self.month)
            && self.day >= 1
            && self.day <= days_in_month(self.year, self.month)
            && self.hour < 24
            && self.minute < 60
            && self.second < 61
    }
    /// the same wall time `days` days later
    pub fn add_days(&self, days: i64) -> Civil {
        let (y, m, d) = civil_from_days(days_from_civil(self.year, self.month, self.day) + days);
        Civil { year: y, month: m, day: d, ..*self }
    }
}

pub fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

/// days of month m (1..12)
pub fn days_in_month(y: i32, m: u8) -> u8 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(y) => 29,
        2 => 28,
        _ => 0,
    }
}

/// days since 1970-01-01 of a date (proleptic Gregorian; Howard Hinnant's algorithm)
pub fn days_from_civil(y: i32, m: u8, d: u8) -> i64 {
    let y = if m <= 2 { y as i64 - 1 } else { y as i64 };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// (year, month, day) of days since 1970-01-01
pub fn civil_from_days(z: i64) -> (i32, u8, u8) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

/// weekday of a date: 0 = Monday .. 6 = Sunday
pub fn weekday(y: i32, m: u8, d: u8) -> u8 {
    (days_from_civil(y, m, d) + 3).rem_euclid(7) as u8
}

fn two(s: &[u8], i: usize) -> Option<u32> {
    let a = s.get(i)?.wrapping_sub(b'0');
    let b = s.get(i + 1)?.wrapping_sub(b'0');
    if a > 9 || b > 9 {
        return None;
    }
    Some(a as u32 * 10 + b as u32)
}

/// "2026-10-12T19:58:00Z" (with ".123" milliseconds when the timestamp has them)
pub fn rfc3339(ts: Timestamp) -> String {
    rfc3339_offset(ts, 0)
}

/// the timestamp as local time with its UTC offset: "2026-10-12T21:58:00+02:00"
pub fn rfc3339_offset(ts: Timestamp, offset_secs: i32) -> String {
    let c = Civil::from_unix(ts.secs + offset_secs as i64);
    let ms = ts.nanos / 1_000_000;
    let frac = if ms != 0 { format!(".{:03}", ms) } else { String::new() };
    let zone = if offset_secs == 0 {
        String::from("Z")
    } else {
        let a = offset_secs.unsigned_abs();
        format!("{}{:02}:{:02}", if offset_secs < 0 { '-' } else { '+' }, a / 3600, a % 3600 / 60)
    };
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}{}", c.year, c.month, c.day, c.hour, c.minute, c.second, frac, zone)
}

/// parse "YYYY-MM-DDTHH:MM:SS[.fff…](Z|±HH:MM)" (also with a space or lower-case t / z)
pub fn parse_rfc3339(s: &str) -> Option<Timestamp> {
    let b = s.trim().as_bytes();
    if b.len() < 20 {
        return None;
    }
    let y = two(b, 0)? * 100 + two(b, 2)?;
    if b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let c = Civil::new(y as i32, two(b, 5)? as u8, two(b, 8)? as u8, two(b, 11)? as u8, two(b, 14)? as u8, two(b, 17)? as u8);
    if !c.is_valid() {
        return None;
    }
    let mut i = 19;
    let mut nanos = 0u32;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let mut digits = 0;
        while let Some(&d) = b.get(i) {
            if !d.is_ascii_digit() {
                break;
            }
            if digits < 9 {
                nanos = nanos * 10 + (d - b'0') as u32;
                digits += 1;
            }
            i += 1;
        }
        while digits < 9 {
            nanos *= 10;
            digits += 1;
        }
    }
    let off = match b.get(i)? {
        b'Z' | b'z' => {
            i += 1;
            0
        }
        &sign @ (b'+' | b'-') => {
            let h = two(b, i + 1)? as i64;
            if b.get(i + 3) != Some(&b':') {
                return None;
            }
            let m = two(b, i + 4)? as i64;
            i += 6;
            let o = h * 3600 + m * 60;
            if sign == b'-' { -o } else { o }
        }
        _ => return None,
    };
    if i != b.len() {
        return None;
    }
    Some(Timestamp { secs: c.to_unix() - off, nanos })
}

/// "12.10.2026 19:58" (de) / "2026-10-12 19:58" (en) in a zone
pub fn format_date_time(ts: Timestamp, zone: &Zone, en: bool) -> String {
    let c = zone.to_local(ts);
    if en {
        format!("{:04}-{:02}-{:02} {:02}:{:02}", c.year, c.month, c.day, c.hour, c.minute)
    } else {
        format!("{:02}.{:02}.{:04} {:02}:{:02}", c.day, c.month, c.year, c.hour, c.minute)
    }
}

/// "12.10.2026" / "2026-10-12"
pub fn format_date(ts: Timestamp, zone: &Zone, en: bool) -> String {
    let c = zone.to_local(ts);
    if en {
        format!("{:04}-{:02}-{:02}", c.year, c.month, c.day)
    } else {
        format!("{:02}.{:02}.{:04}", c.day, c.month, c.year)
    }
}

/// "19:58"
pub fn format_time(ts: Timestamp, zone: &Zone) -> String {
    let c = zone.to_local(ts);
    format!("{:02}:{:02}", c.hour, c.minute)
}

/// a duration for people: "42 s", "3 min 05 s", "1 h 12 min", "2 d 3 h"
pub fn format_duration(secs: i64, en: bool) -> String {
    let s = secs.max(0);
    let d = if en { "d" } else { "T" };
    if s < 60 {
        format!("{} s", s)
    } else if s < 3600 {
        format!("{} min {:02} s", s / 60, s % 60)
    } else if s < 86_400 {
        format!("{} h {:02} min", s / 3600, s % 3600 / 60)
    } else {
        format!("{} {} {} h", s / 86_400, d, s % 86_400 / 3600)
    }
}

/// a relative time for people: "vor 5 min" / "5 min ago", "in 2 h" (now and then in seconds)
pub fn format_relative(now: i64, then: i64, en: bool) -> String {
    let d = then - now;
    let a = d.unsigned_abs() as i64;
    let v = if a < 60 {
        return String::from(if en { "just now" } else { "gerade eben" });
    } else if a < 3600 {
        format!("{} min", a / 60)
    } else if a < 86_400 {
        format!("{} h", a / 3600)
    } else {
        let n = a / 86_400;
        // German: dative after "vor" / "in" ("vor 2 Tagen", "in 1 Tag")
        format!("{} {}", n, match (en, n == 1) {
            (true, true) => "day",
            (true, false) => "days",
            (false, true) => "Tag",
            (false, false) => "Tagen",
        })
    };
    match (d < 0, en) {
        (true, true) => format!("{} ago", v),
        (true, false) => format!("vor {}", v),
        (false, true) => format!("in {}", v),
        (false, false) => format!("in {}", v),
    }
}
