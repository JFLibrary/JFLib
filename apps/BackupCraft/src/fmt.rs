// fmt -- how the console shows values: sizes, times in the display zone, durations, ratios,
// status names and their badge kinds

use alloc::format;
use alloc::string::String;

use bc_proto::model::{HostStatus, OpStatus, RepoStatus, SessionStatus, Severity};
use datetime::{Timestamp, Zone};
use ui::consts::*;

use crate::tr::{en, t};

/// "1,2 GB" (de) / "1.2 GB" (en); 0 -> "–"
pub fn size(b: u64) -> String {
    if b == 0 {
        return String::from("–");
    }
    let units = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1000.0 && u + 1 < units.len() {
        v /= 1000.0;
        u += 1;
    }
    let s = if u == 0 {
        format!("{} B", b)
    } else if v >= 100.0 {
        format!("{:.0} {}", v, units[u])
    } else if v >= 10.0 {
        format!("{:.1} {}", v, units[u])
    } else {
        format!("{:.2} {}", v, units[u])
    };
    if en() { s } else { s.replace('.', ",") }
}

/// "58.064" (de) / "58,064" (en)
pub fn count(n: u64) -> String {
    let digits = format!("{}", n);
    let sep = if en() { ',' } else { '.' };
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

/// "12.10.2026 21:58" in the display zone; 0 -> "–"
pub fn when(secs: i64, zone: &Zone) -> String {
    if secs <= 0 {
        return String::from("–");
    }
    datetime::format_date_time(Timestamp::from_secs(secs), zone, en())
}

/// "Heute 21:58", "Gestern 03:10", "Mo 09:00" (within a week), else the date
pub fn when_short(secs: i64, now: i64, zone: &Zone) -> String {
    if secs <= 0 {
        return String::from(t("Nie"));
    }
    let a = zone.to_local(Timestamp::from_secs(secs));
    let n = zone.to_local(Timestamp::from_secs(now));
    let da = datetime::days_from_civil(a.year, a.month, a.day);
    let dn = datetime::days_from_civil(n.year, n.month, n.day);
    let hm = format!("{:02}:{:02}", a.hour, a.minute);
    match dn - da {
        0 => format!("{} {}", t("Heute"), hm),
        1 => format!("{} {}", t("Gestern"), hm),
        -1 => format!("{} {}", t("Morgen"), hm),
        d if (-6..=6).contains(&d) => format!("{} {}", ui::i18n::weekday_abbr(a.weekday() as u32), hm),
        _ => datetime::format_date(Timestamp::from_secs(secs), zone, en()),
    }
}

/// "vor 5 min" / "in 2 h"
pub fn relative(secs: i64, now: i64) -> String {
    if secs <= 0 {
        return String::from("–");
    }
    datetime::format_relative(now, secs, en())
}

/// "3 min 05 s"
pub fn duration(secs: i64) -> String {
    datetime::format_duration(secs, en())
}

/// "2,5 : 1" of a ratio x 100
pub fn ratio(x100: u32) -> String {
    if x100 == 0 {
        return String::from("–");
    }
    let s = format!("{}.{} : 1", x100 / 100, (x100 % 100) / 10);
    if en() { s } else { s.replace('.', ",") }
}

/// "71 %"
pub fn percent(part: u64, whole: u64) -> String {
    if whole == 0 {
        return String::from("–");
    }
    format!("{} %", (part as u128 * 100 / whole as u128) as u64)
}

/// the label and badge kind of a session status
pub fn session_status(s: SessionStatus) -> (&'static str, u32) {
    match s {
        SessionStatus::Never => (t("Nie gelaufen"), BADGE_NEUTRAL),
        SessionStatus::Running => (t("Läuft"), BADGE_INFO),
        SessionStatus::Success => (t("Erfolgreich"), BADGE_OK),
        SessionStatus::Warning => (t("Warnung"), BADGE_WARN),
        SessionStatus::Failed => (t("Fehlgeschlagen"), BADGE_CRIT),
        SessionStatus::Cancelled => (t("Abgebrochen"), BADGE_NEUTRAL),
    }
}

pub fn op_status(s: OpStatus) -> (&'static str, u32) {
    match s {
        OpStatus::Queued => (t("Wartet"), BADGE_NEUTRAL),
        OpStatus::Running => (t("Läuft"), BADGE_INFO),
        OpStatus::Cancelling => (t("Wird abgebrochen"), BADGE_WARN),
        OpStatus::Succeeded => (t("Erfolgreich"), BADGE_OK),
        OpStatus::Failed => (t("Fehlgeschlagen"), BADGE_CRIT),
        OpStatus::Cancelled => (t("Abgebrochen"), BADGE_NEUTRAL),
        OpStatus::Interrupted => (t("Unterbrochen"), BADGE_WARN),
    }
}

pub fn host_status(s: HostStatus) -> (&'static str, u32) {
    match s {
        HostStatus::Online => (t("Online"), BADGE_OK),
        HostStatus::Offline => (t("Offline"), BADGE_CRIT),
        HostStatus::Pairing => (t("Wird gekoppelt"), BADGE_INFO),
        HostStatus::Error => (t("Fehler"), BADGE_CRIT),
        HostStatus::Unknown => (t("Unbekannt"), BADGE_NEUTRAL),
    }
}

pub fn repo_status(s: RepoStatus) -> (&'static str, u32) {
    match s {
        RepoStatus::Ok => (t("Bereit"), BADGE_OK),
        RepoStatus::Warning => (t("Warnung"), BADGE_WARN),
        RepoStatus::Offline => (t("Nicht erreichbar"), BADGE_CRIT),
        RepoStatus::Locked => (t("Gesperrt"), BADGE_WARN),
        RepoStatus::Error => (t("Fehler"), BADGE_CRIT),
    }
}

pub fn severity(s: Severity) -> u32 {
    match s {
        Severity::Info => BADGE_INFO,
        Severity::Warning => BADGE_WARN,
        Severity::Critical => BADGE_CRIT,
    }
}

/// the kind of a storage fill level (warn from 80 %, critical from 92 %)
pub fn fill_kind(used: u64, cap: u64) -> u32 {
    if cap == 0 {
        return BADGE_NEUTRAL;
    }
    let p = used as u128 * 100 / cap as u128;
    if p >= 92 {
        BADGE_CRIT
    } else if p >= 80 {
        BADGE_WARN
    } else {
        BADGE_ACCENT
    }
}
