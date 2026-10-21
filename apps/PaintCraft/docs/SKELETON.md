# A0 – PaintCraft app skeleton (M0, Strang A)

This is the historical assembly-era skeleton. The current Rust module layout is
documented in [ARCHITECTURE.md](ARCHITECTURE.md).

The skeleton of `apps/PaintCraft` (paint.net 5.1 clone, German UI): complete main-window chrome,
all menus with German labels and paint.net's shortcuts, command routing to per-module stubs,
a temporary canvas, tests.  Builds on both targets, `skeleton.uis` passes on Windows and Linux
(also with `--verify-dirty`), asmcheck: 0 errors in `apps/PaintCraft` (9 BOUNDS warnings: sentinel
terminated table walks).

```
python build.py app PaintCraft --out build/a0
build/a0/win/PaintCraft.exe --shot s.png --size 1280x800 [--theme dark] [--scale 150]
build/a0/win/PaintCraft.exe --script apps/PaintCraft/tests/skeleton.uis [--verify-dirty]
wsl.exe -d Ubuntu-24.04 --cd <repo> -e ./build/a0/linux/PaintCraft --script apps/PaintCraft/tests/skeleton.uis
```

## 1. Files

| file | owner after M0 | content |
|---|---|---|
| `inc/pn.inc` | integrator | the one include (pn_ids.inc, pn_tool.inc, WND_*, ZOOM_PRESETS_N, API notes) |
| `inc/pn_ids.inc` | integrator | every command id, widget id block, timer id, window id |
| `inc/pn_tool.inc` | integrator | TOOL_* (19 tools), UNIT_*, ZOOM_MIN/MAX/100, DOC_MAX, DEF_W/H |
| `src/main.s` | integrator | APP_DESC, module tables, routing, `pn_cmd_set/flags/label/ptr` |
| `src/cmds.s` | integrator | command table `pn_cmds` (.data), menu bar `pn_menubar` (.data) |
| `src/chrome.s` | integrator (A0) | title row, tool bar rows 1/2, status bar, window toggles, Hilfe flyout, zoom presets |
| `src/tabs.s` | M1 fileio/tabs agent | PLACEHOLDER image list (one white thumbnail per document) |
| `src/canvas_stub.s` | M1 view agent → **delete**, write `view.s` (+`rulers.s`) | workspace, centred white 800×600 image with shadow, zoom, grid/ruler state, units, cursor, stub rulers |
| `src/doc.s` | M1 doc agent | STUB: one document "Unbenannt" 800×600 |
| `src/fileio.s` | M1 fileio agent | STUB (only Beenden works) |
| `src/hist.s` | M1 hist agent | STUB |
| `src/clip.s` | M4 clip agent | STUB |
| `src/sel.s` | M2 selection agent | STUB (+ `sel_size`) |
| `src/palette.s` | M2 colours agent | STUB |
| `src/imgops.s` | M4 imgops agent | STUB |
| `src/layer.s` | M1/M3 layers agent | STUB |
| `src/fx.s` | M6 fx agent | STUB (Korrekturen + Effekte) |
| `src/tool.s` | tool framework agent | STUB with real behaviour: current tool, tool keys, brush width, options row, hint text |
| `src/wnd_tools.s`, `wnd_history.s`, `wnd_layers.s`, `wnd_colors.s` | window agents | STUBS (`*_init`, `*_frame`) |
| `src/dlg_settings.s` | M7 settings agent | STUB |
| `src/dlg_misc.s` | integrator / diag agent | STUB (Hilfe items, Info) |
| `src/diag.s` | all (test hooks) | STUB (commands 900–999) |
| `tests/skeleton.uis` | integrator | 450-line test of chrome, menus, states, accelerators, tooltips, combos, theme |

Every STUB file says so in its header.  **Replace a stub file as a whole** (same file name, same
exported names).  `canvas_stub.s` is the only one with a different successor name: delete it when
`view.s` exists (else duplicate symbols).

## 2. Command ids (pn_ids.inc) and routing

Command ids are also the widget ids of `ui_cmd_button` / `ui_menu_cmd`, so widget ids start at 1000.

| range | menu / area | handler (file) |
|---|---|---|
| 100–149 | Datei (CMD_RECENT0 111..120 = recent rows) | `fio_cmd` (fileio.s) |
| 150–159 | Bearbeiten: Verlauf (Undo/Redo, HIST_JUMP arg) | `hist_cmd` (hist.s) |
| 160–179 | Bearbeiten: Zwischenablage, Auswahl kopieren/einfügen | `clip_cmd` (clip.s) |
| 180–199 | Bearbeiten: Auswahl | `sel_cmd` (sel.s) |
| 200–219 | Palette | `pal_cmd` (palette.s) |
| 220–269 | Ansicht (+ CMD_ZOOM_SET arg = zoom in 1/100 %, 0 = fit; CMD_UNIT_SET arg) | `view_cmd` (canvas_stub.s → view.s) |
| 270–319 | Bild | `iop_cmd` (imgops.s) |
| 320–379 | Ebenen (+ LAYER_SELECT / SET_VIS / REORDER with args) | `lay_cmd` (layer.s) |
| 380–439 | Korrekturen (400–439 plugin rows, hidden) | `fx_cmd` (fx.s) |
| 440–699 | Effekte: 440 repeat, 441 plugin errors, categories in blocks of 20 (450 Künstlerisch, 470 Unschärfe, 490 Farbe, 510 Verzerren, 530 Rauschen, 550 Objekt, 570 Foto, 590 Rendern, 610 Stilisieren), 630–699 plugin rows (hidden) | `fx_cmd` (fx.s) |
| 700–705, 711–719 | window toggles F5–F8, reset, translucent; Hilfe flyout (Alt+H) | `chrome_cmd` (chrome.s) |
| 706–709 | Nächstes/Vorheriges Bild, Bilderliste | `tabs_cmd` (tabs.s) |
| 710, 850–899 | Einstellungen (Alt+X), settings dialog | `set_cmd` (dlg_settings.s) |
| 720–749 | Hilfe items (Dokumentation F1 … Info) | `misc_cmd` (dlg_misc.s) |
| 750–799 | 750+TOOL_* select tool; 770–783 tool keys S M Z H F G B E P K L R T O (plain letters); 784 X swap colours; 785 reset colours; 786–789 `[` `]` `Strg+[` `Strg+]`; 790 Fertigstellen; 791 cancel | `tool_cmd` (tool.s) |
| 800–849 | tabs: SELECT/CLOSE (arg = document), copy path, open folder, MOVE | `tabs_cmd` (tabs.s) |
| 900–999 | test hooks for scripts (`cmd ID ARG`) | `diag_cmd` (diag.s) |

Widget id blocks: chrome 1000–1199, tabs 1200–1399 (tab i = 1200+i, close = 1300+i), view/rulers
1400–1599 (canvas 1400), fileio 1600–1799, shared tool options 2000–2199, per-tool options
2200 + TOOL_* × 50, tool windows 4000–4999 (tools 4000, history 4100, layers 4200, colours 4400,
palette 4700), dialogs 5000–8999 (new 5000, resize 5100, canvas size 5200, rotate/zoom 5300, layer
props 5400, settings 5500–5999, save options 6000, print 6200, colour profile 6300, misc 6400,
fxui 7000–7999, curves 8000, levels 8200, other adjustments 8400), test widgets 9900–9999; large
blocks: file dialogs 0x10000 (open), 0x14000 (save), 0x18000 (import), 0x1C000 (palette),
0x20000 (misc) — each uses id .. id+0x3FFF; lists 0x30000 history, 0x40000 layers, 0x50000 fonts,
0x60000 shapes, 0x70000 fx; 0x80000+ unassigned.  Timers TMR_* 0x5000–0x50FF (16 per module),
window ids WINID_TOOLS/HISTORY/LAYERS/COLORS 0x6001–0x6004, dialog windows 0x6010–0x60FF
(named WINID_* to avoid the library's WIN_* names).

## 3. Module contract (main.s)

Every top-level module `MOD` exports four functions; `main.s` calls them through tables
(`mod_init`, `mod_states`, `mod_frame`, `mod_cmd`, `route_tab`):

* `MOD_init()` — once from `pn_init` (order: doc, fio, hist, clip, sel, pal, view, iop, lay, fx,
  tool, tabs, chrome, set, misc, diag, wtools, whist, wlay, wcol).
* `MOD_states()` — start of every frame, **before** anything is drawn: set the state of your own
  commands with `pn_cmd_set(id, CF_DISABLED | CF_CHECKED | CF_HIDDEN)` (never `ui_cmd_state`:
  the library keeps only 128 states; `pn_cmd_set` writes the CF byte of the command table, which
  lives in `.data`).  Commands not implemented yet start with CF_DISABLED from `cmds.s`, so a new
  module must clear it (`pn_cmd_set(id, 0)`) for everything it implements.
* `MOD_frame()` — after the main layout: dialogs, tool windows (`ui_window_begin` once the
  multi-window API is used), overlays that are not part of the canvas.
* `MOD_cmd(id, arg) -> eax 1 handled` — the ids of its range, plus the broadcast built-ins:
  `UIC_CLOSE` goes to `fio_cmd` only (return 1 = keep the window, e.g. "Änderungen speichern?");
  every other `UIC_*` is broadcast in the order tool, view, sel, hist, clip, fio, iop, lay, fx,
  pal, tabs, chrome, set, misc, diag and stops at the first module that returns 1, except
  `UIC_THEME` / `UIC_DPI` / `UIC_WAKE` which reach everybody.  `UIC_TIMER` (arg = timer id):
  handle only your own `TMR_*` block (library timers UIT_* are ignored by all).  `UIC_ESCAPE`
  reaches the tool first.  `UIC_WINCLOSE` for WINID_TOOLS..COLORS is handled by chrome.

Layout calls (not in the tables): `chrome_titlebar` (calls `tabs_frame` after
`ui_next_size(-1000, 48)` – tabs_frame must open exactly one outer box), `chrome_toolbar`,
`chrome_toolrow` (calls `tool_options` inside a BOX_H | BOX_XCENTER after "Werkzeug:"),
`view_frame` (fills the canvas area), `chrome_statusbar` (calls `tool_hint`, `sel_size`,
`doc_cur`/`doc_size`, `view_cursor`, `view_units`, `view_zoom`).

Adding a new **top-level** module = integrator adds it to the tables in `main.s`.  Sub-modules
(dlg_*.s, t_*.s, adj_*.s, efx_*.s, fxui.s, rulers.s, fmt_pdn.s …) are called by their parent
module and need no table entry.

Helpers in main.s: `pn_cmd_set`, `pn_cmd_flags`, `pn_cmd_label(id, str)` (runtime label, e.g.
recent files — escape '&' as "&&"; "<Effekt> wiederholen"), `pn_cmd_ptr(id)` (record: change
CMD_ICON / CMD_KEY / CMD_MODS, e.g. plugin rows).  Menus are in `.data` too: a module may build
menu rows at run time and point a MENU / MI_SUB record (MI_MENU @+16) to them (plugin effects).

Cross-module functions used by the chrome (keep them when replacing the stubs):

| function | module | meaning |
|---|---|---|
| `doc_count() -> eax`, `doc_cur() -> eax` (-1 = none), `doc_size(i) -> eax w, edx h`, `doc_name(i) -> rax`, `doc_dirty(i)`, `doc_version(i)` | doc.s | document list |
| `view_zoom() -> eax` (1/100 %), `view_units() -> eax` (UNIT_*), `view_cursor() -> eax x, edx y, r8d valid` | view | status bar |
| `tool_cur() -> eax`, `tool_options()`, `tool_hint() -> rax` | tool.s | tool row, status bar |
| `sel_size() -> eax w, edx h` (0,0 = none) | sel.s | status bar |
| `chrome_wnd_open(WND_*) -> rax &u32` | chrome.s | visibility flag the tool windows pass to `ui_window_begin` (toggled by F5–F8 / title buttons) |
| `pn_zoom_presets` (28 × u32, descending) | chrome.s | zoom steps (combo box, slider, view zoom in/out) |

Zoom semantics: 100 % = one image pixel per physical screen pixel (like paint.net), so at 150 %
UI scale a 100 % image is smaller relative to the chrome.

## 4. What the skeleton does

* Title row (48 DIP, `ui_titlebar_begin/end`, AF_CUSTOM_TITLEBAR): app icon (accent square with a
  brush), menu bar (7 menus), image list (tab "Unbenannt": thumbnail, selected look, round close
  button with red hover, tooltip), window toggles Werkzeuge/Verlauf/Ebenen/Farben (checked = shown,
  F5–F8), Einstellungen (disabled), Hilfe (flyout, Alt+H), caption buttons.  Empty parts drag the
  window; minimum size 1000 × 600 DIP (the title row cannot collapse yet).
* Tool bar row 1: Neu Öffnen Speichern | Drucken | Ausschneiden Kopieren Einfügen Zuschneiden
  Auswahl aufheben | Rückgängig Wiederholen | Pixel-Raster Lineale (toggles).  Tooltips
  "Label (Kürzel)" from the command table.
* Row 2: "Werkzeug:" + drop-down with the current tool icon (menu of all 19 tools, active one
  checked), divider, tool options (stub: "Pinselbreite:" number box for Pinsel, Radierer, Klonen,
  Neu färben, Linie/Kurve, Formen).  Hairline under the tool bars.
* Canvas stub: workspace (#DADADA light / #181818 dark), centred white image with a 4-step soft
  shadow, 100 % if it fits else "Ganzes Bild"; zoom in/out through the presets, Ganzes Bild (≤ 100 %),
  Originalgröße; rulers (ticks + numbers in image pixels) when Lineale is on; pixel grid state only.
* Status bar: tool icon + "Name: Hilfetext" (own German help texts), selection size (only with
  a selection), image size, cursor position (in the chosen unit: "8,33 × 6,25" Zoll, "21,17 × 15,88"
  cm at 96 dpi), units combo (Pixel/Zoll/Zentimeter), zoom combo ("100%", current value added
  when it is no preset), Ganzes Bild button, zoom slider over the presets.

## 5. Labels

The menu structure, the commands and their shortcuts follow paint.net 5.1; the German labels,
help texts, tooltips and messages are PaintCraft's own wording.  No text is taken over from
paint.net's localisation.  Only short everyday words ("Datei", "Rückgängig", "Ebene löschen")
and established names of the trade ("Gaußscher Weichzeichner", "Helligkeit / Kontrast",
"Seitenverhältnis beibehalten", the DDS format names) coincide.  A comparison against an
installed paint.net (its German resource file, read in memory only and never copied into the
repository) found no sentence of 20 or more characters in common; keep it that way when adding
text.

Notable label choices: Datei ▸ "Zuletzt geöffnet", "Übernehmen ▸ Von Scanner/Kamera…",
"Alles speichern"; Bearbeiten ▸ "Sichtbares kopieren", "Einfügen als neue Ebene / neues Bild",
Auswahl einfügen ▸ Ersetzen / Hinzufügen (Union) / Ausschließen / Schneiden / Umkehren ("xor");
Ansicht ▸ "Ganzes Bild" (Strg+B), "Originalgröße", "Pixel-Raster", units inline (no submenu);
Bild ▸ "Auf Auswahl zuschneiden", "Leinwandgröße…", "90° nach rechts / links drehen",
"Farbprofil…" (one dialog, no submenu), "Zusammenfassen"; Ebenen ▸ "Neue Ebene", "Ebene verdoppeln", "Nach unten
vereinen", "Als Ebene importieren…", "Rotationszoom…", "Oberste / Unterste Ebene wählen", "Ebene
ganz nach oben / eine Stufe höher / eine Stufe tiefer / ganz nach unten", "Ebene ein-/ausblenden"
(Strg+,), "Ebeneneigenschaften…"; adjustments and effects e.g. "Automatisch" = Auto-Level,
"Manuelle Anpassung…" = Levels, "Tontrennung…" = Posterize, "Tiefen / Lichter…",
"Farbtemperatur / Tönung…", submenu "Weichzeichnen", "Verwackelung…" = Fragment,
"Zoom-Weichzeichner…" = Zoom Blur, "In Pixel auflösen…", "Spirale…" = Twist, "Wölbung…" = Bulge,
"Einprägen…" = Emboss, "Gravur…" = Relief, "Quantize…"; tools "Farbpinsel", "Zeichenstift",
"Farbauswahl", "Klonen (Stempel)", "Neu färben", "Zoomen", "Ansicht verschieben", "Pixel
verschieben".

**Open points** (not checkable without UI Automation):
* "Letzten Effekt wiederholen" (paint.net shows "<Effekt> wiederholen"; fx.s should set it with
  `pn_cmd_label`).
* Bearbeiten ▸ "Palette ▸" and "Verlauf löschen": not in the paint.net 5.1 menus (palette items
  live in the Colors window's palette button; clearing the history has no menu item) — added per
  plan / HIST-017.
* Menu order and separators (from "Practical Paint.NET" / paint.net 4 knowledge), Korrekturen and
  Effekte sorted by German name (assumed paint.net sorts by the localised name), "Medianunschärfe"
  placed in Unschärfe, "Median" in Rauschen, "…" after every adjustment/effect with a dialog
  (Sepia assumed to have one: `SepiaEffect.Intensity`), Hilfe flyout order.
* Shortcuts without a source: Beenden Strg+Q, Helligkeit/Kontrast Strg+Umschalt+T, Originalgröße
  also Strg+Umschalt+A, Oberste Ebene wählen Strg+Alt+Bild↑, Gehe zu Ebene oben/unten
  Alt+Bild↑/↓ (book: only Strg+Alt+Bild↓), Rotationszoom Strg+Umschalt+Z, Nächstes/Vorheriges Bild
  Strg+Tab / Strg+Umschalt+Tab, Dokumentation F1.  Verified from resource tooltips: Strg+Z/Y, layer
  keys Strg+Umschalt+N/Entf/D, Strg+M, F4, window toggles F5–F8, Alt+H, Alt+X, X, [ ], Strg+,.
* Tool order (paint.net Tools window, row by row: Rechteckige Auswahl, Pixel verschieben, Lasso, Auswahl verschieben, Ellipse, Zoomen, Zauberstab, Ansicht verschieben,
  Farbeimer, Farbverlauf, Farbpinsel, Radierer, Zeichenstift, Farbauswahl, Klonen, Neu färben, Text,
  Linie/Kurve, Formen) and the default tool (Farbpinsel).
* Window title "Unbenannt - PaintCraft" (paint.net: "{0} - paint.net 5.1.12"; kept distinct on purpose).
* Look: no reference screenshot (paint.net was running); layout/colours from paint.net 5
  knowledge.  Workspace colours, row heights (title 48, rows 40, status 32 DIP) are guesses.

## 6. Icons

Appended to `tools/icons.txt` (section "PaintCraft (agent A0)") and regenerated with
`python tools/ttf2bin.py --all` — additive only (fonts unchanged, icons.bin 162 → 169 icons):
`IC_CLEAR_SELECTION` E8E6, `IC_SCAN` E8FE, `IC_RULER` ED5E, `IC_DEV_TOOLS` EC7A, `IC_SAVE_COPY`
EA35, `IC_MAGIC_SELECT` EB1D, `IC_TOUCH_POINTER` F271.

Placeholders (no Segoe Fluent glyph; a later agent draws vector icons): Zauberstab
IC_MAGIC_SELECT, Ansicht verschieben IC_TOUCH_POINTER (pointing hand; paint.net: open hand),
Farbeimer IC_DROP, Farbverlauf IC_CONTRAST, Klonen IC_PICTURES, Neu färben IC_CRAYON, Auswahl
verschieben IC_POINTER, Pixel verschieben IC_MOVE, Elliptische Auswahl IC_CIRCLE_RING,
Linie/Kurve IC_CURVE, Formen IC_SHAPES, app icon = IC_BRUSH on an accent square, Werkzeuge window
IC_DEV_TOOLS, Ebeneneigenschaften IC_EDIT, merge down IC_DOWNLOAD.  All icons are monochrome
Fluent glyphs; paint.net uses coloured icons.

## 7. Library gaps found (worked around in the app)

1. `ui_cmd_state` keeps only CST_MAX = 128 states and silently drops more → PaintCraft has ~320
   command rows; workaround: command table in `.data` + `pn_cmd_set` writes CMD_FLAGS (the library
   falls back to the table flags).  Suggest: CST_MAX 512 or a documented "table flags are live" rule.
2. `MI_LIST` (DESIGN §5.3) is not implemented → recent files use 10 reserved hidden CMD rows with
   runtime labels; plugin effects need run-time menus (menus kept in `.data`).
3. `TX_ELLIPSIS` is not implemented in `ui_text` (the status bar hint is clipped without "…").
4. Script key names lack OEM / numpad keys (`[ ] , . Num+ Num-`) → the `[` `]` / `Strg+,` / `Strg+Num+`
   accelerators cannot be tested by key; the test uses `cmd`.  `focus S` (DESIGN §5.8) is missing.
5. A focused drop-down / combo box takes F4 itself (WinUI), which collides with paint.net's F4
   (Ebeneneigenschaften) while the tool chooser has the focus.
6. Accelerator text order is "Strg+Umschalt+Alt+C" (paint.net: "Strg+Alt+Umschalt+C"); a checked
   menu row replaces its icon by the check mark (tool menu); `ui_dropdown_button` has no flat
   (ST_SUBTLE) look; the menu bar font is fixed at 14 px (paint.net: 9 pt = 12 px).  Cosmetic.
7. Title bar heights are 32 or 48 DIP only; no overflow for a narrow title row (app min width 1000).

## 8. Tests

`apps/PaintCraft/tests/skeleton.uis` (run by `build.py test`): all 7 menus open (mouse and Alt+K /
Alt+I), every item of every menu and submenu exists with its disabled / checked state (including
all 9 effect categories), Ansicht toggles through the menu and the tool bar, units radio items,
window toggles by click and F5–F8, Hilfe flyout by click and Alt+H, zoom accelerators Strg++ /
Strg+- / Strg+0 / Strg+Umschalt+A / Strg+B, zoom combo box, Ganzes Bild button, zoom slider,
units combo box (size text in Zoll / cm), tool menu, all 14 tool keys incl. S/M/O cycling, brush
width commands, tool keys swallowed by a focused text box, disabled accelerators, tooltips
("Pixel-Raster", "Werkzeuge (F5)"), cursor position with and without rulers, theme switch.
Results: Windows and Linux exit 0, both also with `--verify-dirty`.

Screenshots: `build/a0/shot_{light,dark}_{100,150}.png`, `build/a0/shot_linux_dark_100.png`,
real windows `build/a0/cap_win.png` (Win32) and `build/a0/cap_linux.png` (WSLg), menus
`build/a0/m_*.png`.
