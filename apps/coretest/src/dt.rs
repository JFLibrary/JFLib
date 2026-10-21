// dt -- checks of lib/datetime: civil calendar, RFC 3339, zones and DST, cron

use datetime::cron::{self, Cron};
use datetime::{civil_from_days, days_from_civil, parse_rfc3339, rfc3339, rfc3339_offset, weekday, Civil, LocalResult,
               Timestamp, Zone};

use crate::T;

fn ts(s: &str) -> Timestamp {
    parse_rfc3339(s).unwrap_or_default()
}

pub fn run(t: &mut T) {
    // civil calendar
    t.eq("epoch", days_from_civil(1970, 1, 1), 0);
    t.eq("2000-03-01", days_from_civil(2000, 3, 1), 11_017);
    t.eq("1969-12-31", days_from_civil(1969, 12, 31), -1);
    let mut round = true;
    for d in (-800_000i64..800_000).step_by(997) {
        let (y, m, dd) = civil_from_days(d);
        round &= days_from_civil(y, m, dd) == d;
    }
    t.check("civil round trip", round);
    t.eq("weekday 2026-10-12 (Monday)", weekday(2026, 10, 12), 0);
    t.eq("weekday 2000-01-01 (Saturday)", weekday(2000, 1, 1), 5);
    t.eq("leap 2024-02-29", Civil::date(2024, 2, 29).is_valid(), true);
    t.eq("no 2026-02-29", Civil::date(2026, 2, 29).is_valid(), false);

    // RFC 3339
    let a = ts("2026-10-12T19:58:07Z");
    t.eq("parse Z", a.secs, 1_791_835_087);
    t.eq("format Z", rfc3339(a).as_str(), "2026-10-12T19:58:07Z");
    t.eq("parse offset", ts("2026-10-12T21:58:07+02:00").secs, a.secs);
    t.eq("parse fraction", ts("2026-10-12T19:58:07.250Z").nanos, 250_000_000);
    t.eq("format offset", rfc3339_offset(a, 7200).as_str(), "2026-10-12T21:58:07+02:00");
    t.check("reject garbage", parse_rfc3339("2026-13-01T00:00:00Z").is_none());
    t.check("reject trailing", parse_rfc3339("2026-10-12T19:58:07Zx").is_none());

    // zones
    let berlin = Zone::by_name("Europe/Berlin");
    t.check("Europe/Berlin known", berlin.is_some());
    let berlin = berlin.unwrap_or_else(Zone::utc);
    t.eq("Berlin winter", berlin.offset_at(ts("2026-01-15T12:00:00Z").secs), 3600);
    t.eq("Berlin summer", berlin.offset_at(ts("2026-07-15T12:00:00Z").secs), 7200);
    // 2026: DST from March 29 01:00 UTC to October 25 01:00 UTC
    t.eq("Berlin before switch", berlin.offset_at(ts("2026-03-29T00:59:59Z").secs), 3600);
    t.eq("Berlin at switch", berlin.offset_at(ts("2026-03-29T01:00:00Z").secs), 7200);
    t.eq("Berlin before switch back", berlin.offset_at(ts("2026-10-25T00:59:59Z").secs), 7200);
    t.eq("Berlin at switch back", berlin.offset_at(ts("2026-10-25T01:00:00Z").secs), 3600);
    t.eq("Berlin abbrev", berlin.abbrev_at(ts("2026-07-15T12:00:00Z").secs), "CEST");
    t.eq("gap 02:30", berlin.from_local(&Civil::new(2026, 3, 29, 2, 30, 0)), LocalResult::Gap { after: ts("2026-03-29T01:00:00Z") });
    t.eq("repeated 02:30", berlin.from_local(&Civil::new(2026, 10, 25, 2, 30, 0)),
         LocalResult::Ambiguous(ts("2026-10-25T00:30:00Z"), ts("2026-10-25T01:30:00Z")));
    t.eq("plain 12:00", berlin.from_local(&Civil::new(2026, 6, 1, 12, 0, 0)), LocalResult::Single(ts("2026-06-01T10:00:00Z")));
    let sydney = Zone::by_name("Australia/Sydney").unwrap_or_else(Zone::utc);
    t.eq("Sydney January", sydney.offset_at(ts("2026-01-15T00:00:00Z").secs), 11 * 3600);
    t.eq("Sydney July", sydney.offset_at(ts("2026-07-15T00:00:00Z").secs), 10 * 3600);
    let kolkata = Zone::by_name("Asia/Kolkata").unwrap_or_else(Zone::utc);
    t.eq("Kolkata", kolkata.offset_at(0), 5 * 3600 + 1800);
    let ny = Zone::from_windows("Eastern Standard Time").unwrap_or_else(Zone::utc);
    t.eq("Windows key", ny.name(), "America/New_York");
    t.eq("New York summer", ny.offset_at(ts("2026-07-01T12:00:00Z").secs), -4 * 3600);
    t.check("zone list", Zone::names().count() > 400);
    t.eq("local Berlin", berlin.to_local(ts("2026-10-12T19:58:00Z")), Civil::new(2026, 10, 12, 21, 58, 0));

    // cron
    let c = Cron::parse("30 2 * * *").unwrap();
    t.eq("daily over the gap", c.next_after(ts("2026-03-28T12:00:00Z"), &berlin), Some(ts("2026-03-29T01:00:00Z")));
    t.eq("daily after the gap", c.next_after(ts("2026-03-29T01:00:00Z"), &berlin), Some(ts("2026-03-30T00:30:00Z")));
    t.eq("daily repeated hour", c.next_after(ts("2026-10-24T12:00:00Z"), &berlin), Some(ts("2026-10-25T00:30:00Z")));
    t.eq("daily runs once", c.next_after(ts("2026-10-25T00:30:00Z"), &berlin), Some(ts("2026-10-26T01:30:00Z")));
    t.eq("describe daily", c.describe(false).as_str(), "Täglich um 02:30");
    t.eq("describe daily en", c.describe(true).as_str(), "Daily at 02:30");
    let w = Cron::parse("0 9 * * 1-5").unwrap();
    t.eq("describe weekdays", w.describe(false).as_str(), "Werktags um 09:00");
    // Saturday 2026-10-17 09:00 local -> Monday 19th
    t.eq("weekdays skip weekend", w.next_after(ts("2026-10-16T08:00:00Z"), &berlin), Some(ts("2026-10-19T07:00:00Z")));
    let fri13 = Cron::parse("0 0 13 * fri").unwrap();
    // day of month OR day of week: Friday 2026-10-16 comes first
    t.eq("dom or dow", fri13.next_after(ts("2026-10-12T00:00:00Z"), &Zone::utc()), Some(ts("2026-10-13T00:00:00Z")));
    let q = Cron::parse("*/15 * * * *").unwrap();
    t.eq("describe every 15", q.describe(false).as_str(), "Alle 15 Minuten");
    t.eq("next 3", q.next_n(ts("2026-10-12T10:07:00Z"), &Zone::utc(), 3).len(), 3);
    t.eq("every 15 next", q.next_after(ts("2026-10-12T10:07:00Z"), &Zone::utc()), Some(ts("2026-10-12T10:15:00Z")));
    let m = Cron::parse("0 3 31 * *").unwrap();
    t.eq("monthly 31 skips short months", m.next_after(ts("2026-10-31T04:00:00Z"), &Zone::utc()), Some(ts("2026-12-31T03:00:00Z")));
    t.eq("@weekly", Cron::parse("@weekly").map(|c| c.describe(true)).unwrap_or_default().as_str(), "Weekly on Sun at 00:00");
    t.check("error: four fields", Cron::parse("1 2 3 4").is_err());
    t.check("error: minute 60", Cron::parse("60 * * * *").is_err());
    t.check("error: backwards", Cron::parse("0 5-2 * * *").is_err());
    t.eq("helper weekly", cron::weekly(0b0010101, 2, 0).as_str(), "0 2 * * 1,3,5");
    t.eq("classify weekly", cron::classify(&Cron::parse("0 2 * * 1,3,5").unwrap()), (3, 2, 0, 0b0010101, 0));
    t.eq("classify monthly", cron::classify(&Cron::parse("15 4 1 * *").unwrap()), (4, 4, 15, 0, 1));
}
