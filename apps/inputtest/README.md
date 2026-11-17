# Eingabetest

A desktop window app built with JFLib. Made with `python build_rs.py new inputtest --title "Eingabetest"` on 2026-11-17.

## Build, run, test

```sh
python build_rs.py app inputtest                 # build/win/inputtest.exe and build/linux/inputtest
python build_rs.py run inputtest                 # build and start (Linux: --target linux)
python build_rs.py test inputtest                # build it and run tests/*.uis (Windows and Linux)
```

`python build_rs.py test --quick` runs these tests together with every other app's.
docs/GUIDE.md describes the app model, layout, widgets, files and the test scripts.

## Files

| File | What it is |
|---|---|
| `src/main.rs` | the app: its window, menus, commands and state |
| `STYLE` | the style it is built with (docs/STYLES.md) |
| `PERMISSIONS` | what the program may do; the commented lines are rules it might need ([PERMISSIONS.md](../../docs/PERMISSIONS.md)) |
| `PACKAGE` | name, version, app id and package settings ([PACKAGING.md](../../docs/PACKAGING.md)) |
| `TARGETS` | the targets it builds for |
| `tests/smoke.uis` | the test `build_rs.py test` runs |

## Versions

`version=` in `PACKAGE` is this app's own version (MAJOR.MINOR.PATCH). The program reads it as
`rt::app_version!()`; the packages and `lib/update` use it. The library has a version of its own
(the root `Cargo.toml`, `ui::VERSION`); [VERSIONING.md](../../docs/VERSIONING.md) explains both and
[CHANGELOG.md](../../CHANGELOG.md) lists the library's changes.
