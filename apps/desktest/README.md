# Fenster-Test

The test app of the library's windows inside an area of a window (`lib/ui/desk.rs`), its
taskbar and full screen (`ui.window_fullscreen`; [GUIDE.md, section 35](../../docs/GUIDE.md)):
four windows -- Editor (a text box and a button), Rechner (a fixed size), Notizen (a minimum
size), Protokoll (the last events) -- in an area that keeps a strip for the taskbar (Start, a
button per window, a clock), a tool row (Neues Fenster, Speichern, Laden, Zurücksetzen,
Vollbild, a tool window and its full screen, Baum: the screen readers' tree of the windows) and
status lines with the z-order, the active window's place, the events, the full-screen state
and the saved state. Ctrl+N opens a new window, F11 switches full screen.

## Build, run, test

```sh
python build_rs.py run desktest                   # Windows (Linux: --target linux)
python build_rs.py test desktest                  # tests/*.uis on Windows and Linux (WSL)
build/win/desktest.exe --check-fullscreen         # full screen with real windows (exit code 1 on a failure)
jfgui_BACKEND=wayland build/linux/desktest --check-fullscreen   # the same on Linux: x11 or wayland
python build_rs.py app desktest --target web      # then:
node apps/desktest/tests/fullscreen_web.mjs build --engines=chromium,firefox
```

The scripts run headless with `--verify-dirty`: `move.uis` (dragging, click-to-front, edges
that stick, the title bar kept reachable, Esc), `resize.uis` (every edge and corner, minimum
sizes, a fixed size), `maxmin.uis` (buttons, double click, Alt+F10 / Alt+F9 / Alt+-, the window
menu, closing), `keys.uis` (Alt+F6 / Alt+Shift+F6, the focus, Tab), `snap.uis` (halves and the
top edge, the preview), `taskbar.uis` (activating, minimising, the menu, the keyboard, the
overflow), `shrink.uis` (a smaller area), `save.uis` (the state saved and loaded), `fullscreen.uis`
(the recorded state of the main and the tool window, `system fullscreen`), `rtl.uis` (a
mirrored window), `a11y.uis` (the tree of the screen readers with overlapping windows) and
`shots.uis` (light and dark at 100 and 150 %, WinUI and NetLab, high contrast). Headless runs
only record full screen; `--check-fullscreen` puts the main window and the tool window into
full screen and back with the real platform (Windows; X11 and Wayland under WSLg -- a Wayland
tool window drawn inside its owner has none of its own, `jfgui_WL_TOOLWIN=toplevel` checks a
separate one), `tests/fullscreen_web.mjs` the browser's Fullscreen API (from a click, left by
the browser, refused without a gesture).
