# Ansichtstest

A test app of the library: a canvas that acts like a web view, for the parts of `ui` an app needs
when an engine of its own draws its content (docs/GUIDE.md, section 5 "A canvas that takes the
keyboard"). Made with `python build_rs.py new viewtest --title "Ansichtstest"` on 2026-11-09.

| Script | What it tests |
|---|---|
| `tests/smoke.uis` | the window comes up with its page |
| `tests/keys.uis` | `HB_KEYS`: Ctrl + letter, Tab and Ctrl+Z reach the page (`ui.key_take`), no accelerator runs, the focus stays; F5 goes back to the command table (`ui.accel_run`); without it the defaults; `HB_CURSOR_SET`: the arrow on the page, the hand over a link, the crosshair on a canvas without flags |
| `tests/menus.uis` | a context menu opened a pass after the right click, where it was (`ui.menu_popup_at`), keys going to the open menu, not to the page; a flyout of the app's own at a point of the page (`ui.popup_open_at`) |
| `tests/files.uis` | the file dialog with `FD_MULTI` (E8), the library's own one as in every headless run: Ctrl+click and Shift+click mark several rows (`"a.txt" "c.txt"` in the name box), quoted names typed there, a missing one refused, one file without a modifier; `ui.filedlg_paths()` |
| `tests/folder.uis` | a folder dialog (`FD_FOLDER`) after a file dialog: `ui.filedlg_paths()` is the folder, not the file before |
| `tests/keyraw.uis` | (E2) "Rohe Tasten": the page takes the keys as they go down and up (`ui.key_event_take`): a held Shift (left / right), letters with their scancode, an auto-repeat, the number block, Ctrl + letter, no accelerator; the script commands `keydown` / `keyup` |
| `tests/ime.uis` | (E4) "Eingabefeld": the page's own editable field with the input method (`ui.ime_enable`, `ui.ime_pos`), the composition from `ui.ime_preedit` (script: `preedit`), the commit through `text_take` |
| `tests/appwin.uis` | (E7) "Neues Fenster": an independent app window (`WF_APP`) without the library's title bar, its own one from `titlebar_begin` / `titlebar_end` with a tab in it and the caption buttons acting on that window |
| `tests/cursors.uis` | (E3) `ui.cursor_next` on the page: not-allowed, grab, zoom-in, the north-east corner (shapes beyond the 4 flag bits), a custom cursor through `CUR_CUSTOM`; `ui.cursor_at`; `ui.cursor_replace` redraws it (24 x 24), `ui.cursor_free` frees the handle and `cursor_new` hands it out again |
| `tests/links.uis` | links the page draws itself, published with `ui.sem_rect` (no hit box of their own): their kind and place, a click by label reaching the page at the link, the hand over one, the links in the accessibility tree; (E14) the page's own record named by `ui.canvas_label` |

## Build, run, test

```sh
python build_rs.py app viewtest                 # build/win/viewtest.exe and build/linux/viewtest
python build_rs.py test viewtest                # build it and run tests/*.uis (Windows and Linux)
```

`python build_rs.py test --quick` runs these tests together with every other app's.
