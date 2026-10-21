# Versions, the public API and the changelog

JFLib has one version for the library and lets every app keep a version of its own. The
library follows [Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html): the version
says whether an app that built against the last release still builds against the new one.
`python build_rs.py api-diff` checks that for the Rust API, `python build_rs.py release` cuts a
release, and [CHANGELOG.md](../CHANGELOG.md) lists what changed.

| What | Where |
| --- | --- |
| The library's version | [`lib/VERSION`](../lib/VERSION); in code `rt::VERSION` (also `plat::VERSION`, `ui::VERSION`) and `VERSION_MAJOR`, `VERSION_MINOR`, `VERSION_PATCH` |
| An app's version | `version=` in `apps/NAME/PACKAGE`; in code `rt::app_version!()` (also `ui::app_version!()`) |
| API snapshots of the releases | `docs/api/X.Y.Z.txt`, written by `build_rs.py release` / `api-diff --snapshot` ([`tools/apidiff.py`](../tools/apidiff.py)) |
| The changelog | [`CHANGELOG.md`](../CHANGELOG.md) ([Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/)), [`tools/changelog.py`](../tools/changelog.py) |
| Tests | [`tools/tests/test_devkit.py`](../tools/tests/test_devkit.py) (the suite runs it; the suite also prints the API changes since the last release) |

## Two kinds of versions

**The library** is versioned as a whole: every crate under `lib/`, the styles, and the contract
between an app and the build (the app folder: `TARGETS`, `STYLE`, `PERMISSIONS`, `PACKAGE`,
`BINS`, `assets/`, `tests/*.uis`; the `ui::main!` / `rt::main!` entry points; the command line
of every app; the script language). `rt::VERSION` is checked at compile time to be
`MAJOR.MINOR.PATCH`.

**An app** keeps its own version in `apps/NAME/PACKAGE` (`version=0.1.0` for a new app). The
build hands it to every program of the app (`JFGUI_VERSION`), which reads it with
`rt::app_version!()` -- an `&'static str`, `"0.0.0"` without a `version=`. The packages
([PACKAGING.md](PACKAGING.md)) and the updater ([UPDATES.md](UPDATES.md):
`update::Config::new("paint", rt::app_version!(), ..)`) use the same value. The build refuses a
`version=` that is not `MAJOR.MINOR.PATCH` (with an optional `-pre` / `+build` part). Raise it
for every release of the app; the library's version does not change it.

```rust
const APP_VERSION: &str = rt::app_version!();
ui.text(TS_CAPTION, &format!("Version {} · JFLib {}", APP_VERSION, ui::VERSION));
```

## What is public

The public API is what [API.md](API.md) documents:

- every item of the crates in the `CRATES` table of [`tools/gendoc.py`](../tools/gendoc.py) that
  another crate can name: `pub` items of the crate root and of `pub mod` modules, items
  re-exported with `pub use` (also through private modules and globs), the `pub` methods and
  fields of public types, every member of a public trait, `#[macro_export]` macros, the derives
  and `impl Trait for Type` of public types;
- of the codec crates (`img_png` ... `img_pdn`) only the crate root, its re-exports and the items
  `CODEC_EXTRA` names: their other modules are building blocks for `apps/codectest`;
- not: items marked `#[doc(hidden)]` (internal hand-overs such as `plat::boot::permissions`),
  `pub(crate)` and private items, `rt::os` (the OS calls `plat` is built on), `simd`, the style
  crates' own items (the `ui::Style` traits are public), anything in `apps/`.

Beyond the Rust items, these are part of the contract and are reviewed by hand at a release:
the `PERMISSIONS` syntax ([PERMISSIONS.md](PERMISSIONS.md)), the `PACKAGE` keys, the script
language and the command line switches of `ui::main!` ([GUIDE.md](GUIDE.md) section 12), the
`app://` bundle and the app folder layout, the commands of `build_rs.py`.

Not covered: how a widget looks (styles may change pixels in any release), the exact text of
messages, timings, the layout of private data, and behaviour that is documented as unspecified.

## What each part of the version promises

| Change | 1.0 and later | while 0.x |
| --- | --- | --- |
| a public item removed or renamed; a signature changed (parameter or return types, generics, `unsafe`, `const`); a field's type changed | MAJOR | MINOR |
| a new variant of an enum without `#[non_exhaustive]`; a new required member of a public trait; a provided trait member that became required; `#[non_exhaustive]` added | MAJOR | MINOR |
| a derive or trait impl of a public type removed; an item that existed on every target became target-specific | MAJOR | MINOR |
| a documented behaviour or the syntax of `PERMISSIONS`, `PACKAGE` or the script language changed incompatibly | MAJOR | MINOR |
| new items, new provided trait members, new variants of `#[non_exhaustive]` enums, new derives | MINOR | PATCH |
| a new public field of a struct whose other fields are public too (see below) | MINOR | PATCH |
| a deprecation | MINOR | PATCH |
| fixes, speed, documentation, internal changes, styles drawing differently | PATCH | PATCH |

While the library is 0.x, the MINOR number counts incompatible changes and PATCH everything
else, as Cargo treats 0.x versions: an app that built against 0.3.1 builds against every 0.3.y.

**Struct fields.** Rust counts a new public field as a breaking change when a program builds the
struct as a literal. The library's descriptor structs come with an empty value
(`AppDesc::NONE`, `Default`), and the guide asks for `AppDesc { title: "..", ..AppDesc::NONE }`,
so a new field is a compatible addition here; `api-diff` lists it under "Notes". Code that lists
every field without `..X::NONE` may have to be extended.

**Parameter names** are no part of a signature: renaming `_id` to `id` changes nothing for a
caller, and `api-diff` ignores it, as it ignores line breaks and trailing commas.

## Deprecations

1. Mark the item with `#[deprecated(since = "X.Y.Z", note = "use ... instead")]` (X.Y.Z: the
   release that brings the deprecation) and say what replaces it in its doc comment.
2. The build turns warnings into errors (`-D warnings`): the library's own crates and apps stop
   using the item in the same change. Where a use must stay (a test of the old item), wrap it in
   `#[allow(deprecated)]` with a comment why.
3. Add a changelog entry under "Deprecated" (`build_rs.py changelog "..." --type deprecated`).
4. Keep the item for at least one release; remove it only in a release that may break (the next
   MAJOR, while 0.x the next MINOR), with an entry under "Removed".

`api-diff` lists new deprecations, and a removed item that was deprecated says so.

## `build_rs.py api-diff`

```
python build_rs.py api-diff                    # the sources against the newest docs/api snapshot
python build_rs.py api-diff --against 0.1.0    # ... against that release
python build_rs.py api-diff --all              # every line (default: 200 per group)
python build_rs.py api-diff --snapshot         # write docs/api/<lib/VERSION>.txt
```

It reads the sources with gendoc's parser (no compiler), walks every crate's modules to find
each item's public paths and keeps the shortest, so moving an item between modules that are
re-exported changes nothing. It prints four groups: **Breaking** (removed, changed, a new variant
of an exhaustive enum, a new required trait member, ...), **Deprecated**, **Added** and
**Notes**, then the version the next release needs. The exit code is 1 when there are breaking
changes and `lib/VERSION` is lower than that version, so it can guard a release or a CI job;
between releases (`lib/VERSION` still the last release) it flags every breaking change.

`build_rs.py test` prints a one-line summary (`API since 0.1.0: 2 breaking, 0 deprecated, 14 added`)
and does not fail on it: breaking changes between releases are allowed, the release decides the
version.

A snapshot has one line per item, sorted: `PATH`, `KIND` (`fn`, `method`, `struct`, `field`,
`enum`, `variant`, `trait`, `trait-fn`, `const`, `static`, `type`, `macro`, `impl`, `use` ...),
the signature (constants without their value) and flags (`windows` / `linux` for items of one
target, `deprecated`, `provided`, `all-pub`, `non-exhaustive`). [`tools/apidiff.py`](../tools/apidiff.py)
describes the format and the rules in detail.

## Releasing the library

1. `python build_rs.py api-diff`: read the breaking changes and decide the version.
2. `python build_rs.py changelog` shows `[Unreleased]`; add what is missing.
3. `python build_rs.py release X.Y.Z` renames `[Unreleased]` to `[X.Y.Z] - DATE` (a new empty
   `[Unreleased]` above it), writes `lib/VERSION` and the snapshot `docs/api/X.Y.Z.txt`. It
   refuses a version that is not newer than the last release or too small for the API changes
   (`--force` cuts it anyway, with a warning; `--date` sets the date).
4. `python build_rs.py test --quick` (both targets) and `python tools/gendoc.py --check`, then
   commit and tag: `git tag vX.Y.Z`. The tools never commit, tag or push.

## The changelog

[CHANGELOG.md](../CHANGELOG.md) follows Keep a Changelog: newest release first, an
`[Unreleased]` section on top, and in every section the groups Added, Changed, Deprecated,
Removed, Fixed and Security. Entries are written for app authors (what they can use now, what
they must change), grouped by area, not one line per commit.

```
python build_rs.py changelog                                       # show [Unreleased]
python build_rs.py changelog "Tables sort by column" --area ui     # Added (default)
python build_rs.py changelog "Wrong DPI on the second monitor" --type fixed --area plat
```

A branch adds its entries under `[Unreleased]` before it is merged. Apps note their own changes
in their own files (for example `apps/NAME/README.md`), not here.
