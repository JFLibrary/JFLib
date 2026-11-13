# jflib-base

The portable basics of [JFLib](https://jflib.org) that sit above its runtime and platform layer:
settings files and typed settings, numbers as text, the command line, text encodings, a bounded
XML reader, and a handful of tools for programs that simulate, measure or talk to people at a
console. It is plain Rust, without the standard library and without a line of `unsafe` code.

## Settings

`base::Ini` reads and writes the familiar `[section]` / `key=value` files and touches only the
lines you change. Comments, blank lines, keys it does not know and even malformed lines come back
exactly as they were, with their spacing. Values are stored as written, without quoting, so they
may contain `;` and `#`. Saving writes the new file next to the old one and then puts it in place,
creating missing folders on the way.

On top of that, `base::settings` gives an app typed settings. Each key is declared once with its
type, default, limits and label, a `Schema` lists them with a format version, and the library
validates, stores and migrates them:

```rust
use jflib::base::{Key, Schema, Settings};

const DARK: Key<bool> = Key::bool("view.dark", false).label("Dark mode");
const ZOOM: Key<i64> = Key::int("view.zoom", 100, 10, 800).label("Zoom (%)").step(10);
static SCHEMA: Schema = Schema::new(1, &[DARK.d, ZOOM.d]).sections(&[("view", "View")]);

let mut s = Settings::open("config://settings.ini", &SCHEMA); // a missing file: the defaults
let zoom: i64 = s.get(&ZOOM);
s.set(&ZOOM, zoom + 10);                                      // kept within 10..=800
s.save();
```

The file keeps only what differs from the default, so a new default reaches every user who never
changed the value. A value that does not fit reads as the default, numbers are clamped and texts
cut to their maximum. An older file runs the app's migration once before its values are read
(`Schema::migrate`, with `settings::rename` for keys that moved). In an app with a window,
`ui.settings_page` of [`jflib-ui`](https://crates.io/crates/jflib-ui) draws the whole settings page
from the same schema.

## Numbers, text and the command line

`f64_format` writes the shortest text that reads back as the same double, always with a `.` as
the decimal separator, and `f64_parse` reads it back; the text is the same on x86-64 and ARM64.
`parse_i64` takes decimal and hexadecimal numbers and the words `true`, `yes`, `on` and their
opposites. `utf8_decode` steps through bytes that may not be valid UTF-8, and `Args` answers the
usual questions about the command line (`value("--out")`, `flag("--quiet")`).

`base::encoding` decodes and encodes text strictly: UTF-8, with or without a byte order mark, and
UTF-16 in both byte orders, which it recognises by the mark. It never guesses a legacy code page
and never puts in replacement characters; invalid input is an error.

`base::xml` reads a deliberately small, namespace-aware subset of XML for data exchange. It has no
DTDs, no entities and no way to fetch anything, refuses what it cannot represent without loss,
and stops at limits on size, element count and nesting that the caller can set
(`xml::parse_with` and `xml::Limits`).

## For simulators, servers and consoles

- `rng`: deterministic pseudo-random numbers, PCG32 and SplitMix64. The same seed gives the same
  sequence on every target, and `Pcg32::derive` gives each of many objects a generator of its own,
  so adding one does not shift the others. Not for keys or nonces.
- `metrics`: counters, gauges, log-linear histograms and sliding windows that answer quantiles
  without keeping the values, in integer arithmetic only.
- `cmdtree`: command trees for router-style command lines, with abbreviations, `?` help, Tab
  completion, modes, the `no` form, typed parameters and the familiar error messages.
- `hashring`: consistent hashing with virtual nodes; adding or removing a member moves only the
  keys of its own tokens.
- `lru`: a map that forgets the least recently used entries, limited by count and by weight.
- `pattern`: shell wildcards (`glob`) and POSIX basic and extended regular expressions without
  back-references. Matching runs a Pike VM, so its time grows with the text, never exponentially.

## How to get it

Like all of JFLib, the crate comes through the one dependency
[`jflib`](https://crates.io/crates/jflib), as `jflib::base`. All of JFLib's crates share one
version and are released together. It runs on every platform JFLib supports.

## What's new in 0.2.0

The XML reader takes limits of its own: `xml::parse_with` with an `xml::Limits` of bytes,
elements and nesting depth, for formats whose documents legitimately grow beyond the defaults of
`xml::parse`. Everything is in the
[changelog](https://docs.rs/crate/jflib-base/latest/source/CHANGELOG.md).

## Documentation

- [Settings in JFLib's guide](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  and [command trees](https://docs.rs/jflib/latest/jflib/guide/index.html#26-command-trees-basecmdtree)
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#base): every public
  item of `base`, also on [docs.rs](https://docs.rs/jflib-base)

## License

MIT or Apache-2.0, at your choice.
