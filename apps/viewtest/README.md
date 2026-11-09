# Ansichtstest

A test app of the library: a canvas that acts like a web view, for the parts of `ui` an app needs
when an engine of its own draws its content (docs/GUIDE.md, section 5 "A canvas that takes the
keyboard"). Made with `python build_rs.py new viewtest --title "Ansichtstest"` on 2026-11-09.

| Script | What it tests |
|---|---|
| `tests/smoke.uis` | the window comes up with its page |
| `tests/keys.uis` | `HB_KEYS`: Ctrl + letter, Tab and Ctrl+Z reach the page (`ui.key_take`), no accelerator runs, the focus stays; F5 goes back to the command table (`ui.accel_run`); without it the defaults; `HB_CURSOR_SET`: the arrow on the page, the hand over a link, the crosshair on a canvas without flags |
| `tests/menus.uis` | a context menu opened a pass after the right click, where it was (`ui.menu_popup_at`), keys going to the open menu, not to the page; a flyout of the app's own at a point of the page (`ui.popup_open_at`) |

## Build, run, test

```sh
python build_rs.py app viewtest                 # build/win/viewtest.exe and build/linux/viewtest
python build_rs.py test viewtest                # build it and run tests/*.uis (Windows and Linux)
```

`python build_rs.py test --quick` runs these tests together with every other app's.
