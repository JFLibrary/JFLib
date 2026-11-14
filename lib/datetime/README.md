# jflib-datetime

Calendar time for [JFLib](https://jflib.org), without the standard library and without a line of
`unsafe` code: timestamps, calendar dates, ISO weeks, RFC 3339 text, time zones with daylight
saving time, and cron schedules. It has no clock of its own and only computes, so it gives the
same answers on every machine.

## What it does

A `Timestamp` is a point in time, seconds and nanoseconds since 1970 in UTC. A `Civil` is a
calendar date and wall time without a zone (`Civil::from_unix` and `to_unix` convert), and free
functions answer the calendar questions: leap years, the days of a month, the weekday, the ISO 8601
week of a date and the Monday of a week. `rfc3339` and `parse_rfc3339` write and read the timestamps of logs and
protocols, `format_date_time`, `format_duration` and `format_relative` the texts people read (in
German or English).

`Zone` knows the zones of the IANA time zone database by name, each with its current rule for
daylight saving time, maps Windows' time zone names to them, and also takes a POSIX TZ string or a
TZif file such as `/etc/localtime`. It turns a timestamp into local time and back. Going back is
the hard direction, and `from_local` says so honestly: a wall time is `Single`, `Ambiguous` in the
hour the clocks repeat, or falls into the `Gap` when they jump forward.

`Cron` reads five-field cron expressions, with month and weekday names and shortcuts such as
`@daily`, and finds the next runs in a zone. A run that falls into a skipped hour happens right
after the jump, one in a repeated hour happens once. `describe` puts a schedule into words.

```rust
use jflib::{datetime, plat};
use datetime::{Cron, Timestamp, Zone};

let now = Timestamp::from_nanos(plat::unix_time_ns());
let berlin = Zone::by_name("Europe/Berlin").unwrap_or_else(Zone::utc);
let stamp = datetime::rfc3339(now);                          // "2026-11-13T09:30:00.123Z"
let shown = datetime::format_date_time(now, &berlin, true);  // "2026-11-13 10:30"
if let Ok(cron) = Cron::parse("30 2 * * 1-5") {
    let next = cron.next_after(now, &berlin);                // the next weekday at 02:30 in Berlin
    let words = cron.describe(true);                         // "Weekdays at 02:30"
}
```

A zone describes its offsets from its last rule change on: times before that change are computed
with today's rule. That suits schedules, logs and displays of the present, not historical
records.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::datetime`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.0

ISO 8601 weeks: `iso_week` gives the week of a date with its week-numbering year,
`iso_weeks_in_year` says whether a year has 52 or 53 weeks, and `iso_week_monday` gives the Monday
of a week. The week calendar of `ui.calendar` in `jflib-ui` is built on them. Everything is in the
[changelog](https://docs.rs/crate/jflib-datetime/latest/source/CHANGELOG.md).

## Documentation

- [JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html): how an app is written
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#datetime): every public
  item of `datetime`, also on [docs.rs](https://docs.rs/jflib-datetime)

## License

MIT or Apache-2.0, at your choice. The table that maps Windows' time zone names to IANA zones
comes from the Unicode CLDR and is also under the
[Unicode License v3](https://github.com/JFLibrary/JFLib/blob/main/lib/datetime/licenses/Unicode-3.0.txt).
