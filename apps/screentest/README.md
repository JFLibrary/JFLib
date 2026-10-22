# Bildschirm-Test

The demo and test app of the library's screen services (`plat::screen`, `plat::hotkey`,
`lib/ui/screen.rs`; [GUIDE.md, section 21](../../docs/GUIDE.md)): every monitor with its name,
rectangle, work area, dpi, refresh rate and the primary flag, and how often the list changed; a
global shortcut typed into a text box (registered, its presses counted, the reason when the
desktop dropped it); a colour picker with the pipette (`CP_PIPETTE`) and a button for
`ui.pick_color`; a picture of the screen, shown scaled down. Made with `python build_rs.py new
screentest --title "Bildschirm-Test"` on 2026-10-22, then adapted.

## Build, run, test

```sh
python build_rs.py run screentest                 # Windows (Linux: --target linux)
python build_rs.py test screentest                # tests/*.uis on Windows and Linux (WSL)
python build_rs.py app screentest --target web    # then:
node apps/screentest/tests/screen_web.mjs build --engines=chromium,firefox
python build_rs.py app screentest --target android --android-abi x86_64   # then:
python apps/screentest/tests/android_screen.py --serial emulator-5560 --apk build/android/screentest-x86_64-debug.apk
```

The scripts run headless: the window is the one monitor (`screen X Y W H` replaces it,
`system settings monitors` is a change), the shortcut is only recorded and `system hotkey 1`
presses it, a pick waits for `system pick RRGGBB` or `system pick cancel`, pictures are
unavailable. The real platforms are tested by `plattest --test screen` (Windows, X11),
`--test screen-wl`, `--test screen-policy` and `tools/platshared/screen_bus.py` (the portals).
`tests/screen_web.mjs` checks the browser: the monitor of `window.screen` and a change of it
(Chromium), shortcuts and pictures unavailable, the pipette only with the EyeDropper (Chromium),
the pick's answer through a stand-in EyeDropper. `tests/android_screen.py` starts the app on a
device or an emulator with `jfgui_SELFTEST=screen` (it writes its monitors to
`data://screen-selftest.txt`) and compares the display with `wm size`, `wm density` and `dumpsys
display`, then changes it with `wm size`.

## Files

| File | What it is |
|---|---|
| `src/main.rs` | the app |
| `STYLE` | the style it is built with (docs/STYLES.md) |
| `PERMISSIONS` | `input.global` (the shortcut) and `screen.capture` (the picture); picking a colour needs nothing ([PERMISSIONS.md](../../docs/PERMISSIONS.md)) |
| `PACKAGE` | name, version, app id and package settings ([PACKAGING.md](../../docs/PACKAGING.md)) |
| `TARGETS` | Windows, Linux, the browser and Android (`android/`: the manifest of the shared Java host) |
| `tests/*.uis` | the headless scripts; `tests/screen_web.mjs` the browser test |
