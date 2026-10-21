# Scaffold-Test

A JFLib tool: a window and a command line program on one shared crate. Made with `python build_rs.py new scaffoldtest --kind tool --title "Scaffold-Test"` on 2026-10-20.

## Build, run, test

```sh
python build_rs.py app scaffoldtest                 # build/win/scaffoldtest.exe and build/linux/scaffoldtest
python build_rs.py run scaffoldtest                 # the window (Linux: --target linux)
python build_rs.py run scaffoldtest --bin scaffoldtest-cli -- README.md   # the command line program
python build_rs.py test scaffoldtest                # build it and run tests/*.uis (Windows and Linux)
```

`python build_rs.py test --quick` runs these tests together with every other app's.
docs/GUIDE.md describes the app model, layout, widgets, files and the test scripts.

## Files

| File | What it is |
|---|---|
| `src/main.rs` | the window |
| `cli/main.rs` | the command line program `scaffoldtest-cli` |
| `crates/scaffoldtest_core/lib.rs` | the code both share (an app-local crate) |
| `BINS` | the further programs of the app (here `scaffoldtest-cli`) |
| `STYLE` | the style the window is built with (docs/STYLES.md) |
| `PERMISSIONS` | what the program may do; the commented lines are rules it might need ([PERMISSIONS.md](../../docs/PERMISSIONS.md)) |
| `PACKAGE` | name, version, app id and package settings ([PACKAGING.md](../../docs/PACKAGING.md)) |
| `TARGETS` | the targets it builds for |
| `tests/smoke.uis` | the test `build_rs.py test` runs |

## Versions

`version=` in `PACKAGE` is this app's own version (MAJOR.MINOR.PATCH). The program reads it as
`rt::app_version!()`; the packages and `lib/update` use it. The library has a version of its own
(`lib/VERSION`, `ui::VERSION`); [VERSIONING.md](../../docs/VERSIONING.md) explains both and
[CHANGELOG.md](../../CHANGELOG.md) lists the library's changes.

## A fixture

This app is the output of `python build_rs.py new scaffoldtest --kind tool --title "Scaffold-Test"`
and stays that way: `tools/tests/test_devkit.py` checks that the generator still writes exactly
these sources (all but this README and the `PACKAGE` fields taken from git), and
`build_rs.py test` builds and runs it with every other app. After a change of the templates in
`tools/scaffold.py`, delete the folder and run the command again; after a library change that
breaks this app, fix the templates too.
