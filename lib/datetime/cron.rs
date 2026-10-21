// cron -- 5-field cron schedules evaluated in a time zone
//
//   Cron::parse("30 2 * * 1-5")   minute hour day-of-month month day-of-week; fields: *, n,
//                                 a-b, a-b/n, */n, lists a,b; month names jan..dec, weekday
//                                 names sun..sat (0 and 7 = Sunday); @hourly @daily @midnight
//                                 @weekly @monthly @yearly @annually.  Day of month and day of
//                                 week restricted both: either matches (Vixie cron).
//   cron.next_after(ts, &zone)    the first run after ts: a wall time in a DST gap runs at the
//                                 jump (right after the gap), a repeated wall time runs once
//                                 (its first occurrence); None when nothing matches within
//                                 5 years
//   cron.next_n(ts, &zone, n)     the next n runs
//   cron.describe(en)             "Täglich um 02:00", "Werktags um 02:00", "Alle 15 Minuten" …
//   Schedule helpers: daily(h, m), weekly(mask, h, m), monthly(day, h, m), hourly(m) build
//   the expressions of the usual choices (mask bit 0 = Monday)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::tz::{LocalResult, Zone};
use crate::{days_in_month, Civil, Timestamp};

/// a parsed cron expression
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cron {
    minutes: u64,
    hours: u32,
    /// bits 1..31
    dom: u32,
    /// bits 1..12
    months: u16,
    /// bits 0..6, 0 = Sunday
    dow: u8,
    dom_star: bool,
    dow_star: bool,
    src: String,
}

/// a parse error: the field (0 minute .. 4 day of week, 5 = the whole expression) and why
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CronError {
    pub field: u8,
    pub msg: &'static str,
}

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const DAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

fn value(s: &str, field: u8, min: u32, max: u32) -> Result<u32, CronError> {
    let l = s.to_ascii_lowercase();
    if field == 3 {
        if let Some(i) = MONTHS.iter().position(|m| *m == l) {
            return Ok(i as u32 + 1);
        }
    }
    if field == 4 {
        if let Some(i) = DAYS.iter().position(|d| *d == l) {
            return Ok(i as u32);
        }
    }
    let mut v: u32 = 0;
    if s.is_empty() || s.len() > 2 {
        return Err(CronError { field, msg: "number expected" });
    }
    for c in s.bytes() {
        if !c.is_ascii_digit() {
            return Err(CronError { field, msg: "number expected" });
        }
        v = v * 10 + (c - b'0') as u32;
    }
    // 7 = Sunday as well
    let hi = if field == 4 { 7 } else { max };
    if v < min || v > hi {
        return Err(CronError { field, msg: "value out of range" });
    }
    Ok(if field == 4 && v == 7 { 0 } else { v })
}

// one field -> (bit set, was "*")
fn field(s: &str, field: u8, min: u32, max: u32) -> Result<(u64, bool), CronError> {
    let mut bits = 0u64;
    let mut star = false;
    for part in s.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, st)) => {
                let n = value(st, 5, 1, 60).map_err(|_| CronError { field, msg: "bad step" })?;
                (r, n)
            }
            None => (part, 1),
        };
        let (lo, hi) = if range == "*" {
            if step == 1 && s == "*" {
                star = true;
            }
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            let a = value(a, field, min, max)?;
            let mut b = value(b, field, min, max)?;
            if field == 4 && b == 0 && a > 0 {
                b = 7; // mon-sun
            }
            if a > b {
                return Err(CronError { field, msg: "range runs backwards" });
            }
            (a, b)
        } else {
            let v = value(range, field, min, max)?;
            (v, if part.contains('/') { max } else { v })
        };
        let mut v = lo;
        while v <= hi {
            let b = if field == 4 && v == 7 { 0 } else { v };
            bits |= 1 << b;
            v += step;
        }
    }
    if bits == 0 {
        return Err(CronError { field, msg: "empty field" });
    }
    Ok((bits, star))
}

impl Cron {
    /// parse an expression (see the module comment)
    pub fn parse(s: &str) -> Result<Cron, CronError> {
        let t = s.trim();
        let expanded = match t {
            "@hourly" => "0 * * * *",
            "@daily" | "@midnight" => "0 0 * * *",
            "@weekly" => "0 0 * * 0",
            "@monthly" => "0 0 1 * *",
            "@yearly" | "@annually" => "0 0 1 1 *",
            _ => t,
        };
        let f: Vec<&str> = expanded.split_whitespace().collect();
        if f.len() != 5 {
            return Err(CronError { field: 5, msg: "five fields expected" });
        }
        let (minutes, _) = field(f[0], 0, 0, 59)?;
        let (hours, _) = field(f[1], 1, 0, 23)?;
        let (dom, dom_star) = field(f[2], 2, 1, 31)?;
        let (months, _) = field(f[3], 3, 1, 12)?;
        let (dow, dow_star) = field(f[4], 4, 0, 6)?;
        Ok(Cron {
            minutes,
            hours: hours as u32,
            dom: dom as u32,
            months: months as u16,
            dow: dow as u8,
            dom_star,
            dow_star,
            src: String::from(t),
        })
    }

    /// the expression as written
    pub fn as_str(&self) -> &str {
        &self.src
    }

    /// whether a date matches the month, day-of-month and day-of-week fields
    pub fn matches_date(&self, y: i32, m: u8, d: u8) -> bool {
        if self.months & (1 << m) == 0 {
            return false;
        }
        let dom_ok = self.dom & (1 << d) != 0;
        let wd = (crate::weekday(y, m, d) + 1) % 7; // 0 = Sunday
        let dow_ok = self.dow & (1 << wd) != 0;
        match (self.dom_star, self.dow_star) {
            (true, true) => true,
            (true, false) => dow_ok,
            (false, true) => dom_ok,
            (false, false) => dom_ok || dow_ok,
        }
    }

    /// the first run strictly after `after` in `zone`
    pub fn next_after(&self, after: Timestamp, zone: &Zone) -> Option<Timestamp> {
        // start at the next whole minute of the local wall time
        let mut c = zone.to_local(Timestamp::from_secs(after.secs));
        c.second = 0;
        c.nanos = 0;
        let start = Civil { ..c };
        let mut day = Civil::date(c.year, c.month, c.day);
        let mut first_day = true;
        for _ in 0..(366 * 5) {
            if self.matches_date(day.year, day.month, day.day) {
                for h in 0..24u8 {
                    if self.hours & (1 << h) == 0 || (first_day && h < start.hour) {
                        continue;
                    }
                    for mi in 0..60u8 {
                        if self.minutes & (1 << mi) == 0 || (first_day && h == start.hour && mi < start.minute) {
                            continue;
                        }
                        let cand = Civil::new(day.year, day.month, day.day, h, mi, 0);
                        let t = match zone.from_local(&cand) {
                            LocalResult::Single(t) => t,
                            LocalResult::Ambiguous(a, _) => a,
                            LocalResult::Gap { after: j } => j,
                        };
                        if t.secs > after.secs {
                            return Some(t);
                        }
                    }
                }
            }
            day = day.add_days(1);
            first_day = false;
            // skip whole months that never match
            while self.months & (1 << day.month) == 0 {
                let dim = days_in_month(day.year, day.month) as i64;
                day = day.add_days(dim - day.day as i64 + 1);
            }
        }
        None
    }

    /// the next n runs after `after`
    pub fn next_n(&self, after: Timestamp, zone: &Zone, n: usize) -> Vec<Timestamp> {
        let mut out = Vec::with_capacity(n);
        let mut t = after;
        while out.len() < n {
            match self.next_after(t, zone) {
                Some(x) => {
                    out.push(x);
                    t = x;
                }
                None => break,
            }
        }
        out
    }

    fn single(bits: u64) -> Option<u32> {
        if bits.count_ones() == 1 { Some(bits.trailing_zeros()) } else { None }
    }

    /// the schedule in words (German, or English with en)
    pub fn describe(&self, en: bool) -> String {
        let all_min = self.minutes == (1u64 << 60) - 1;
        let all_hours = self.hours == (1u32 << 24) - 1;
        let all_months = self.months == 0x1FFE;
        let at = |h: u32, m: u32| format!("{:02}:{:02}", h, m);
        let m1 = Cron::single(self.minutes);
        let h1 = Cron::single(self.hours as u64);
        if all_months && self.dom_star && self.dow_star {
            if all_min && all_hours {
                return String::from(if en { "Every minute" } else { "Jede Minute" });
            }
            if all_hours {
                if let Some(m) = m1 {
                    return if en { format!("Hourly at minute {}", m) } else { format!("Stündlich zur Minute {}", m) };
                }
                // */n
                let n = self.minutes.count_ones();
                if n > 1 && 60 % n == 0 {
                    let step = 60 / n;
                    let mut ok = true;
                    for k in 0..n {
                        if self.minutes & (1 << (k * step)) == 0 {
                            ok = false;
                        }
                    }
                    if ok {
                        return if en { format!("Every {} minutes", step) } else { format!("Alle {} Minuten", step) };
                    }
                }
            }
            if let (Some(m), Some(h)) = (m1, h1) {
                return if en { format!("Daily at {}", at(h, m)) } else { format!("Täglich um {}", at(h, m)) };
            }
            if let Some(m) = m1 {
                let n = self.hours.count_ones();
                if n > 1 && 24 % n == 0 {
                    let step = 24 / n;
                    let first = self.hours.trailing_zeros();
                    let mut ok = first < step;
                    for k in 0..n {
                        if self.hours & (1 << (first + k * step)) == 0 {
                            ok = false;
                        }
                    }
                    if ok {
                        return if en {
                            format!("Every {} hours at minute {}", step, m)
                        } else {
                            format!("Alle {} Stunden zur Minute {}", step, m)
                        };
                    }
                }
            }
        }
        if let (Some(m), Some(h), true) = (m1, h1, all_months) {
            if self.dom_star && !self.dow_star {
                if self.dow == 0b0111110 {
                    return if en { format!("Weekdays at {}", at(h, m)) } else { format!("Werktags um {}", at(h, m)) };
                }
                let names_de = ["So", "Mo", "Di", "Mi", "Do", "Fr", "Sa"];
                let names_en = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
                let mut days = Vec::new();
                for d in [1usize, 2, 3, 4, 5, 6, 0] {
                    if self.dow & (1 << d) != 0 {
                        days.push(if en { names_en[d] } else { names_de[d] });
                    }
                }
                let list = days.join(", ");
                return if en { format!("Weekly on {} at {}", list, at(h, m)) } else { format!("Wöchentlich am {} um {}", list, at(h, m)) };
            }
            if !self.dom_star && self.dow_star {
                if let Some(d) = Cron::single(self.dom as u64) {
                    return if en {
                        format!("Monthly on day {} at {}", d, at(h, m))
                    } else {
                        format!("Monatlich am {}. um {}", d, at(h, m))
                    };
                }
            }
        }
        if en { format!("Custom: {}", self.src) } else { format!("Benutzerdefiniert: {}", self.src) }
    }
}

/// "M H * * *"
pub fn daily(h: u8, m: u8) -> String {
    format!("{} {} * * *", m, h)
}

/// "M * * * *"
pub fn hourly(m: u8) -> String {
    format!("{} * * * *", m)
}

/// "M H * * days" with mask bit 0 = Monday .. bit 6 = Sunday (empty mask: every day)
pub fn weekly(mask: u8, h: u8, m: u8) -> String {
    if mask & 0x7F == 0 || mask & 0x7F == 0x7F {
        return daily(h, m);
    }
    let mut days = Vec::new();
    for d in 0..7u8 {
        if mask & (1 << d) != 0 {
            days.push(format!("{}", (d + 1) % 7));
        }
    }
    format!("{} {} * * {}", m, h, days.join(","))
}

/// "M H D * *"
pub fn monthly(day: u8, h: u8, m: u8) -> String {
    format!("{} {} {} * *", m, h, day.clamp(1, 31))
}

/// the (kind, hour, minute, weekday mask, day of month) of an expression built by the helpers
/// above: kind 1 hourly, 2 daily, 3 weekly, 4 monthly, 0 anything else (custom)
pub fn classify(c: &Cron) -> (u8, u8, u8, u8, u8) {
    let all_hours = c.hours == (1u32 << 24) - 1;
    let all_months = c.months == 0x1FFE;
    let m1 = Cron::single(c.minutes);
    let h1 = Cron::single(c.hours as u64);
    if !all_months {
        return (0, 0, 0, 0, 0);
    }
    match (m1, h1, c.dom_star, c.dow_star) {
        (Some(m), _, true, true) if all_hours => (1, 0, m as u8, 0, 0),
        (Some(m), Some(h), true, true) => (2, h as u8, m as u8, 0x7F, 0),
        (Some(m), Some(h), true, false) => {
            let mut mask = 0u8;
            for d in 0..7u8 {
                if c.dow & (1 << ((d + 1) % 7)) != 0 {
                    mask |= 1 << d;
                }
            }
            (3, h as u8, m as u8, mask, 0)
        }
        (Some(m), Some(h), false, true) => match Cron::single(c.dom as u64) {
            Some(d) => (4, h as u8, m as u8, 0, d as u8),
            None => (0, 0, 0, 0, 0),
        },
        _ => (0, 0, 0, 0, 0),
    }
}
