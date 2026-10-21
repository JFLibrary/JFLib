// tz -- time zones from POSIX TZ rules ("CET-1CEST,M3.5.0,M10.5.0/3")
//
//   Zone::utc() / Zone::by_name("Europe/Berlin") / Zone::from_windows("W. Europe Standard Time")
//   Zone::from_posix(name, rule) / Zone::fixed(name, offset_secs)
//   Zone::names() -> every IANA name of the table (sorted)
//   zone.offset_at(secs) -> UTC offset (seconds east), zone.abbrev_at(secs), zone.is_dst(secs)
//   zone.to_local(ts) -> Civil;  zone.from_local(&civil) -> LocalResult:
//        Single(ts) | Ambiguous(first, second) (the repeated hour) | Gap { after } (the skipped
//        hour: `after` is the instant the clocks jumped, the first valid time after the gap)
//
// A rule describes the zone from its last change on (the TZif footer of tools/gen_tz.py), so
// times before that change use today's rule.

use alloc::string::String;

use crate::zones::{WINDOWS, ZONES};
use crate::{days_in_month, weekday, Civil, Timestamp};

/// a DST transition rule: Mm.w.d (month, week 1..5 (5 = last), weekday 0 = Sunday), Jn (day
/// 1..365 without Feb 29) or n (day 0..365 with it); time in seconds after local midnight
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rule {
    M { month: u8, week: u8, wday: u8, time: i32 },
    J { day: u16, time: i32 },
    D { day: u16, time: i32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Dst {
    abbr: String,
    off: i32,
    start: Rule,
    end: Rule,
}

/// a time zone: its name, standard offset and an optional daylight saving rule
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Zone {
    name: String,
    abbr: String,
    off: i32,
    dst: Option<Dst>,
}

/// the instants a local wall time stands for
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalResult {
    Single(Timestamp),
    /// the wall time occurs twice (the clocks went back): earlier and later instant
    Ambiguous(Timestamp, Timestamp),
    /// the wall time does not exist (the clocks jumped forward): the jump's instant
    Gap { after: Timestamp },
}

impl LocalResult {
    /// the earliest instant that stands for the wall time (a gap: the jump)
    pub fn earliest(&self) -> Timestamp {
        match *self {
            LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => t,
            LocalResult::Gap { after } => after,
        }
    }
}

// ------------------------------------------------------------------ POSIX TZ parsing

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn name(&mut self) -> Option<String> {
        if self.peek() == Some(b'<') {
            let start = self.i + 1;
            while self.peek()? != b'>' {
                self.i += 1;
            }
            let n = core::str::from_utf8(&self.s[start..self.i]).ok()?;
            self.i += 1;
            return Some(String::from(n));
        }
        let start = self.i;
        while matches!(self.peek(), Some(c) if c.is_ascii_alphabetic()) {
            self.i += 1;
        }
        if self.i - start < 3 {
            return None;
        }
        Some(String::from(core::str::from_utf8(&self.s[start..self.i]).ok()?))
    }
    fn num(&mut self) -> Option<i32> {
        let start = self.i;
        let mut v = 0i32;
        while let Some(c) = self.peek() {
            if !c.is_ascii_digit() {
                break;
            }
            v = v.checked_mul(10)?.checked_add((c - b'0') as i32)?;
            self.i += 1;
        }
        if self.i == start { None } else { Some(v) }
    }
    // [+-]hh[:mm[:ss]] -> seconds
    fn time(&mut self) -> Option<i32> {
        let neg = match self.peek() {
            Some(b'-') => {
                self.i += 1;
                true
            }
            Some(b'+') => {
                self.i += 1;
                false
            }
            _ => false,
        };
        let mut t = self.num()? * 3600;
        if self.peek() == Some(b':') {
            self.i += 1;
            t += self.num()? * 60;
            if self.peek() == Some(b':') {
                self.i += 1;
                t += self.num()?;
            }
        }
        Some(if neg { -t } else { t })
    }
    fn rule(&mut self) -> Option<Rule> {
        let r = match self.peek()? {
            b'M' => {
                self.i += 1;
                let month = self.num()? as u8;
                if self.peek()? != b'.' {
                    return None;
                }
                self.i += 1;
                let week = self.num()? as u8;
                if self.peek()? != b'.' {
                    return None;
                }
                self.i += 1;
                let wday = self.num()? as u8;
                if !(1..=12).contains(&month) || !(1..=5).contains(&week) || wday > 6 {
                    return None;
                }
                Rule::M { month, week, wday, time: 7200 }
            }
            b'J' => {
                self.i += 1;
                Rule::J { day: self.num()? as u16, time: 7200 }
            }
            _ => Rule::D { day: self.num()? as u16, time: 7200 },
        };
        let time = if self.peek() == Some(b'/') {
            self.i += 1;
            self.time()?
        } else {
            7200
        };
        Some(match r {
            Rule::M { month, week, wday, .. } => Rule::M { month, week, wday, time },
            Rule::J { day, .. } => Rule::J { day, time },
            Rule::D { day, .. } => Rule::D { day, time },
        })
    }
}

fn parse_posix(rule: &str) -> Option<(String, i32, Option<Dst>)> {
    let mut p = P { s: rule.as_bytes(), i: 0 };
    let abbr = p.name()?;
    // POSIX offsets are west of UTC: "CET-1" = UTC+1
    let off = -p.time()?;
    if p.peek().is_none() {
        return Some((abbr, off, None));
    }
    let dabbr = p.name()?;
    let doff = match p.peek() {
        Some(b',') | None => off + 3600,
        _ => -p.time()?,
    };
    let (start, end) = if p.peek() == Some(b',') {
        p.i += 1;
        let s = p.rule()?;
        if p.peek()? != b',' {
            return None;
        }
        p.i += 1;
        let e = p.rule()?;
        (s, e)
    } else {
        // the US default of POSIX
        (Rule::M { month: 3, week: 2, wday: 0, time: 7200 }, Rule::M { month: 11, week: 1, wday: 0, time: 7200 })
    };
    if p.peek().is_some() {
        return None;
    }
    Some((abbr, off, Some(Dst { abbr: dabbr, off: doff, start, end })))
}

// the local seconds (since the epoch, read as UTC) of a rule in year y, at its time
fn rule_local(y: i32, r: Rule) -> i64 {
    let (date, time) = match r {
        Rule::M { month, week, wday, time } => {
            // weekday of the 1st: crate weekday is 0 = Monday; the rule's 0 = Sunday
            let first = (weekday(y, month, 1) + 1) % 7;
            let mut day = 1 + (wday as i32 - first as i32 + 7) % 7 + 7 * (week as i32 - 1);
            let dim = days_in_month(y, month) as i32;
            while day > dim {
                day -= 7;
            }
            (crate::days_from_civil(y, month, day as u8), time)
        }
        Rule::J { day, time } => {
            // 1..365, February 29 never counted
            let mut d = day as i64 - 1;
            if crate::is_leap(y) && day >= 60 {
                d += 1;
            }
            (crate::days_from_civil(y, 1, 1) + d, time)
        }
        Rule::D { day, time } => (crate::days_from_civil(y, 1, 1) + day as i64, time),
    };
    date * 86_400 + time as i64
}

impl Zone {
    /// UTC
    pub fn utc() -> Zone {
        Zone { name: String::from("UTC"), abbr: String::from("UTC"), off: 0, dst: None }
    }

    /// a fixed offset (seconds east of UTC)
    pub fn fixed(name: &str, offset_secs: i32) -> Zone {
        Zone { name: String::from(name), abbr: String::from(name), off: offset_secs, dst: None }
    }

    /// a zone from a POSIX TZ rule
    pub fn from_posix(name: &str, rule: &str) -> Option<Zone> {
        let (abbr, off, dst) = parse_posix(rule)?;
        Some(Zone { name: String::from(name), abbr, off, dst })
    }

    /// an IANA zone of the table ("Europe/Berlin"; "UTC", "Etc/UTC", "Z" also work)
    pub fn by_name(name: &str) -> Option<Zone> {
        if matches!(name, "UTC" | "Z" | "Etc/UTC" | "Etc/Universal" | "Universal" | "Zulu" | "GMT" | "Etc/GMT") {
            return Some(Zone { name: String::from(if name == "Z" { "UTC" } else { name }), ..Zone::utc() });
        }
        let i = ZONES.binary_search_by(|(n, _)| (*n).cmp(name)).ok()?;
        Zone::from_posix(ZONES[i].0, ZONES[i].1)
    }

    /// the IANA zone of a Windows time zone key ("W. Europe Standard Time")
    pub fn from_windows(key: &str) -> Option<Zone> {
        let i = WINDOWS.binary_search_by(|(k, _)| (*k).cmp(key)).ok()?;
        Zone::by_name(WINDOWS[i].1)
    }

    /// the IANA name of a Windows time zone key
    pub fn windows_to_iana(key: &str) -> Option<&'static str> {
        WINDOWS.binary_search_by(|(k, _)| (*k).cmp(key)).ok().map(|i| WINDOWS[i].1)
    }

    /// every IANA zone name of the table, sorted
    pub fn names() -> impl Iterator<Item = &'static str> {
        ZONES.iter().map(|(n, _)| *n)
    }

    /// a zone from a TZif file (its footer rule; /etc/localtime), named `name`
    pub fn from_tzif(name: &str, data: &[u8]) -> Option<Zone> {
        if !data.starts_with(b"TZif") || data.last() != Some(&b'\n') {
            return None;
        }
        let body = &data[..data.len() - 1];
        let i = body.iter().rposition(|&c| c == b'\n')?;
        let rule = core::str::from_utf8(&body[i + 1..]).ok()?;
        if rule.is_empty() {
            return None;
        }
        Zone::from_posix(name, rule)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// the standard offset (seconds east of UTC)
    pub fn std_offset(&self) -> i32 {
        self.off
    }

    // (start, end) of DST in the UTC seconds of year y (by the standard-time year)
    fn dst_window(&self, y: i32) -> Option<(i64, i64, i32)> {
        let d = self.dst.as_ref()?;
        // the start time is given in standard time, the end time in daylight time
        let start = rule_local(y, d.start) - self.off as i64;
        let end = rule_local(y, d.end) - d.off as i64;
        Some((start, end, d.off))
    }

    /// whether daylight saving time is in effect at an instant
    pub fn is_dst(&self, secs: i64) -> bool {
        let Some(_) = self.dst else { return false };
        let y = Civil::from_unix(secs + self.off as i64).year;
        let Some((s, e, _)) = self.dst_window(y) else { return false };
        if s < e { secs >= s && secs < e } else { secs < e || secs >= s }
    }

    /// the UTC offset (seconds east) in effect at an instant
    pub fn offset_at(&self, secs: i64) -> i32 {
        match &self.dst {
            Some(d) if self.is_dst(secs) => d.off,
            _ => self.off,
        }
    }

    /// the abbreviation in effect at an instant ("CET" / "CEST")
    pub fn abbrev_at(&self, secs: i64) -> &str {
        match &self.dst {
            Some(d) if self.is_dst(secs) => &d.abbr,
            _ => &self.abbr,
        }
    }

    /// the wall time of an instant
    pub fn to_local(&self, ts: Timestamp) -> Civil {
        let mut c = Civil::from_unix(ts.secs + self.offset_at(ts.secs) as i64);
        c.nanos = ts.nanos;
        c
    }

    /// the instants of a wall time (seconds; nanos kept)
    pub fn from_local(&self, c: &Civil) -> LocalResult {
        let l = c.to_unix();
        let ns = c.nanos;
        let Some(d) = &self.dst else {
            return LocalResult::Single(Timestamp { secs: l - self.off as i64, nanos: ns });
        };
        let t1 = l - self.off as i64; // read as standard time
        let t2 = l - d.off as i64; // read as daylight time
        let ok1 = self.offset_at(t1) == self.off;
        let ok2 = self.offset_at(t2) == d.off;
        let (a, b) = (t1.min(t2), t1.max(t2));
        match (ok1, ok2) {
            (true, true) if t1 != t2 => LocalResult::Ambiguous(Timestamp { secs: a, nanos: ns }, Timestamp { secs: b, nanos: ns }),
            (true, _) => LocalResult::Single(Timestamp { secs: t1, nanos: ns }),
            (_, true) => LocalResult::Single(Timestamp { secs: t2, nanos: ns }),
            _ => {
                // in the gap: the transition between a and b
                let y = c.year;
                let mut jump = b;
                for yy in [y - 1, y, y + 1] {
                    if let Some((s, e, _)) = self.dst_window(yy) {
                        for t in [s, e] {
                            if t >= a && t <= b {
                                jump = t;
                            }
                        }
                    }
                }
                LocalResult::Gap { after: Timestamp { secs: jump, nanos: 0 } }
            }
        }
    }
}
