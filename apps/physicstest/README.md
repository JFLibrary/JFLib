# Physik-Test

A console program built with JFLib. Made with `python build_rs.py new physicstest --kind console --title "Physik-Test"` on 2026-11-17.

## Build, run, test

```sh
python build_rs.py app physicstest                 # build/win/physicstest.exe and build/linux/physicstest
python build_rs.py run physicstest -- Anna        # build and start (Linux: --target linux)
python build_rs.py test physicstest                # build it and run tests/*.uis (Windows and Linux)
```

`python build_rs.py test --quick` runs these tests together with every other app's.
docs/GUIDE.md describes the app model, layout, widgets, files and the test scripts.

## Files

| File | What it is |
|---|---|
| `src/main.rs` | the program: arguments, output, the self-check |
| `PERMISSIONS` | what the program may do; the commented lines are rules it might need ([PERMISSIONS.md](../../docs/PERMISSIONS.md)) |
| `PACKAGE` | name, version, app id and package settings ([PACKAGING.md](../../docs/PACKAGING.md)) |
| `TARGETS` | the targets it builds for |
| `tests/smoke.uis` | the test `build_rs.py test` runs |

## Versions

`version=` in `PACKAGE` is this app's own version (MAJOR.MINOR.PATCH). The program reads it as
`rt::app_version!()`; the packages and `lib/update` use it. The library has a version of its own
(the root `Cargo.toml`, `plat::VERSION`); [VERSIONING.md](../../docs/VERSIONING.md) explains both and
[CHANGELOG.md](../../CHANGELOG.md) lists the library's changes.
