# A3 – colours: colour model, palette, "Farben" window (PaintCraft, M2)

COLOR-001..008, 016..020 (the picker tool items 009..015 belong to the tool agents; they use the
API below).  Both targets, also with `--verify-dirty`:

```
python build.py app PaintCraft --out build/a3
build/a3/win/PaintCraft.exe --script apps/PaintCraft/tests/colors.uis [--verify-dirty]
wsl.exe -d Ubuntu-24.04 --cd <repo> -e ./build/a3/linux/PaintCraft --script apps/PaintCraft/tests/colors.uis
```

## 1. Files

| file | content |
|---|---|
| `src/palette.s` | colour model (primary / secondary), palette (96), palette files, palette commands 200–219, the palette grid + palette buttons of the Colors window (ids 4700–4999), file dialog 0x1C000, error dialog, test hooks |
| `src/wnd_colors.s` | the "Farben" tool window (ids 4400–4699), WINID_COLORS |
| `tests/colors.uis` | 450-line test (both targets, `--verify-dirty`) |
| `tests/fixtures/palettes/a3_*.txt` | `a3_mixed` (BOM, CRLF, comments, 6/8 digits, '#', tabs, 6 malformed lines → 7 colours), `a3_big` (100 colours → 96, 4 ignored), `a3_empty` (no colour → error), `a3_two` |
| `src/diag.s` | block "A3": commands 985–994 → `pal_diag_cmd`, `diag_frame` also calls `pal_diag_frame` |

## 2. Colour API (for tools: brush, bucket, picker, text, shapes, gradient …)

Colours are **straight 0xAARRGGBB** (convert to the layer format yourself).

```
pal_primary() -> eax            primary colour (default FF000000)
pal_secondary() -> eax          secondary colour (default FFFFFFFF)
pal_color(which) -> eax         0 = primary, 1 = secondary (left / right mouse button)
pal_set_primary(argb)           e.g. the colour picker (left click)
pal_set_secondary(argb)         (right click)
pal_set_color(which, argb)
pal_swap()                      X          -- tool.s (T1) routes CMD_SWAP_COLORS (784) here
pal_reset()                     black/white -- tool.s (T1) routes CMD_RESET_COLORS (785) here
pal_version() -> eax            bumped by every change of the colours or the palette
```
A setter that changes a colour bumps the version and calls `ui_repass()`: every window is
rebuilt, the Colors window shows the new colour in the next pass (also when it is set from
`on_command` or from a tool's canvas input).  Setting the same colour does nothing.
`pal_swap` / `pal_reset` are cheap and can be called from any command handler.
**T1:** please enable 784 / 785 (`pn_cmd_set(id, 0)` in `tool_states`; `cmds.s` starts them
with XD) and route them to `pal_swap` / `pal_reset`; palette.s does not touch them.

## 3. Palette

```
pal_count() -> eax              colours (0..96)
pal_entry(i) -> eax             colour i (0 when i >= count)
pal_set_entry(i, argb) -> eax   1: i < count replaces, i == count appends
pal_add(argb) -> eax            index, -1 when full
pal_add_primary()               "Farbe hinzufügen" (see 4.)
pal_default()                   default palette
pal_load_file(path) -> eax 0/-1, edx = ignored lines   (palette unchanged on -1)
pal_save_file(path) -> eax 0/-1                         (atomic: plat_file_write)
pal_folder(buf) -> eax 1 exists  (buf PAL_PATH_CAP = 1040 bytes)
pal_checker(x, y, w, h)         checkerboard (px) under transparent colours
```
**File format** (paint.net `*.txt`, human readable, COLOR-020): one colour per line,
`AARRGGBB` (8 hex digits) or `RRGGBB` (6 digits = opaque), optional leading `#`, upper or lower
case; `;` starts a comment (a whole line or the rest of a line); blanks, tabs, CR ignored; a
UTF-8 BOM is skipped; at most 96 colours are taken; malformed lines and colours beyond 96 are
ignored and counted (`edx`, probe 9988).  A file without any colour is refused ("Die Palette
ließ sich nicht laden." dialog).  At most 256 KB of a file are parsed.  Saved with a
German `;` header (own wording, same content as paint.net's), one `AARRGGBB` line per colour,
CRLF on Windows, LF on Linux.  paint.net pads palettes with fewer than 96 colours with white:
our files load there unchanged; here missing slots are **free slots** (empty wells).

**Default palette** (reconstructed from paint.net; verified by the hue structure): four rows of
16 = two greys + 14 hues 360/14° apart (FF0000 FF6A00 FFD800 B6FF00 4CFF00 00FF21 00FF90
00FFFF 0094FF 0026FF 4800FF B200FF FF00DC FF006E): full colours (greys 000000, 404040), half
value (FFFFFF, 808080), pastel (A0A0A0, 303030), dim (C0C0C0, 606060).  paint.net adds 32 white
slots (its format rule); here those 32 are free slots, so "Farbe hinzufügen" can fill them.

**Palette folder**: Windows `<home>/Documents/paint.net App Files/Palettes` when it exists
(shared with an installed paint.net), else `<config>/PaintCraft/Palettes`
(`plat_config_dir`; Linux `$XDG_CONFIG_HOME` / `~/.config`).  The app never creates it and
never writes anywhere but the path chosen in the Speichern-unter dialog.

**Commands** (`pal_cmd`, Bearbeiten ▸ Palette and the Colors window's palette menu):
`CMD_PAL_FOLDER` 200 "Ordner mit Paletten öffnen…" (Öffnen dialog in the palette folder – there is
no "open in Explorer" platform API, see 8.), `CMD_PAL_OPEN` 204 "Palette öffnen…" (starts at
the last palette file), `CMD_PAL_SAVE` 201 "Palette speichern als…" (last file, else
`<folder>/Palette.txt`), `CMD_PAL_RESET` 202 "Standardpalette laden",
`CMD_PAL_LOAD` 203 arg = row of the palette list, `CMD_PAL_ADD` 205 "Farbe hinzufügen";
`UIC_ESCAPE` ends the replace mode (consumed only then).  File dialog: `ui_filedlg`
(`ID_FD_PALETTE` 0x1C000, `"txt=Palette"`; Windows system dialog, headless /
`jfgui_FILEDLG=builtin` the library's; name box = 114700).  Errors: content dialog "Palette"
in the main window (id 4900).

## 4. The "Farben" window (wnd_colors.s)

`ui_window_begin(WINID_COLORS, "Farben", chrome_wnd_open(WND_COLORS), WF_TOOL | WF_AUTOHEIGHT,
&g_wc_rect)`; F8 / the title-bar toggle / the close button (chrome.s) show and hide it.
Default place: bottom left of the main window above the status bar; right of the Werkzeuge
window when they would overlap (1280 × 800: x 130, y 410; compact 272 × 320 DIP client,
expanded 496 × 372).  "Mehr >>" keeps the larger window inside the main window (moves it up /
left).  `wcol_reset_pos()` puts it back (for CMD_WND_RESET, integrator).

```
[Primärfarbe v] [Mehr >>]          4400 combo box (which colour the controls edit), 4401
[P/S] [ wheel ] [V]                4402 primary square (top left) over 4403 secondary; click =
[+][pal v]                           edit that one (accent frame = edited); 4404 swap arrow
[palette 16 x 6]                     (tooltip "Farben vertauschen (X)"), 4405 black / white
                                     ("Standardfarben"); 4406 HSV wheel (hue, saturation; drawn
                                     at full value like paint.net; picking on it while the
                                     colour is black gives full value), 4407 value track
                                     (black .. full value); 4796 "Farbe hinzufügen", 4797
                                     palette menu; 4700 + i swatches
"Mehr >>" adds on the right:       RGB  R: G: B: 4410/4411, 4412/4413, 4414/4415 (track +
                                     number box 0..255, gradient with the other channels)
                                   Hex.: 4424 (RRGGBB; 6 digits keep the alpha, 8 set it, '#'
                                     ok, junk restores the text; taken on Enter / focus loss)
                                   HSV  H: 4416/4417 (0..360, rainbow), S: 4418/4419, V:
                                     4420/4421 (0..100)
                                   Transparenz - Alpha: 4422/4423 (checkerboard track, 0..255)
```
Palette grid: click = primary, right click = secondary (COLOR-019), tooltip `#RRGGBB`
(`#AARRGGBB` when transparent), transparent colours over a checkerboard, free slots as empty
wells, semantic record K_SWATCH label `AARRGGBB` (value 1 = filled).  "Farbe hinzufügen"
appends the primary colour; on a full palette (96) it switches to the **replace mode**
(paint.net's + button): accent frame round the grid, the next click on a swatch replaces it
with the primary colour (right click: secondary; a free slot appends); a second click on + or
Esc ends it.  Palette menu: the four palette commands, then up to 3 `*.txt` files of the
palette folder (sorted, without ".txt"; read when the menu opens) and "Weitere Paletten…"
(Öffnen dialog in the folder) when there are more.

Consistency rules: all controls edit one colour live (every change calls `pal_set_color`);
the window keeps an HSV record so a grey keeps its hue and black keeps hue and saturation;
H/S/V fields are rounded integers (paint.net: S 87 % → `hsv(…, 870, …)`).  When the colour
changes elsewhere (palette, tools, X, another window) the fields follow in the next pass.
**Stale text guard**: `ui_number_box` keeps the text it had when it got the focus and commits
it when the focus leaves (library behaviour); if the colour changes elsewhere meanwhile the
window takes the focus from that box and undoes its next commit (`wc_guard`, `wc_blur`,
`wc_text_ev`; tested in colors.uis).

## 5. Test hooks (diag.s block A3, `pal_diag_*` in palette.s)

`cmd 985 k` palette folder: 0 real, 1 `build`, 2 `apps/PaintCraft/tests/fixtures/palettes`
(relative to the working directory); `cmd 986 argb` / `cmd 987 argb` primary / secondary
(decimal, e.g. 4294901760 = FFFF0000); `cmd 988 n` palette of n generated colours
(FF000000 + i·010203).  Records: 9985 text primary `AARRGGBB` (value = pal_version), 9986
secondary, 9987 value colour count / text name of the last palette file, 9988 value ignored
lines / text last error, 9989 value replace mode.

## 6. Tests

`tests/colors.uis`: defaults (window rect, colours, 64-colour default palette, free slots),
F8 / close button / chrome toggle, Mehr / Weniger (width and WF_AUTOHEIGHT height via `expect
window 24580 rect`), palette left / right click, tooltips, R G B boxes → hex + HSV, H S V boxes
→ RGB, grey keeps the hue, alpha, stale-text guard, hex 6 / 8 digits / junk, track drags to
both ends (R, A, H = 360), wheel (edge = hue 0 / 100 %, top = 90°, centre = grey), value track
(black keeps hue / saturation), black + wheel = full value, secondary editing (square and combo
box value), swap / reset icons, colours set from outside, Farbe hinzufügen, save to
`build/a3_colors_test.txt` + reset + reload (Öffnen starts with the last file), fixtures (mixed
lines, 96 limit), replace mode (left / right click, Esc, second click, free slot), empty file
error dialog, palette menu (commands, every palette file, loading from the menu),
Bearbeiten ▸ Palette enabled, screenshots.  Results: Windows and Linux exit 0, both with
`--verify-dirty`; all other PaintCraft scripts unchanged and passing.  Not in the test yet: the X
key (needs T1's routing of 784 — add `key x` / `expect #9985 …` there).

Screenshots: `build/a3/colors_{less,more}_{light,dark}_100.png`,
`build/a3/colors_more_{dark,light}_150.png`, `build/a3/colors_menu.png`,
`build/a3/colors_desktop.png`.

## 7. Changes in files of others (all additive, marked "A3")

* `inc/pn_ids.inc`: `CMD_PAL_OPEN = 204`, `CMD_PAL_ADD = 205`.
* `src/cmds.s`: row `CMD CMD_PAL_OPEN, "&Palette öffnen…", IC_FOLDER_OPEN …` and
  `MI_CMD CMD_PAL_OPEN` in `m_palette`.
* `src/diag.s`: A3 block in `diag_cmd`; `diag_frame` calls `a1_diag_frame` then `pal_diag_frame`.
* `tests/skeleton.uis`: Bearbeiten ▸ Palette items are now `enabled` (+ "Palette öffnen…").
* Integrator: `wcol_reset_pos` exists for "Fensteranordnung zurücksetzen"; `g_wc_rect` /
  `g_wc_more` could be persisted by the settings module (no API yet); paint.net has no colour
  indicator in the tool bar, so none was added to chrome.s.

## 8. Open issues / library gaps

1. `TX_WRAP` (ui.inc) is not implemented in `ui_text` (the error dialog uses two lines;
   fileio.s uses TX_WRAP in its dialogs and is cut off too).
2. `ui_number_box` does not show a value changed by the app while it has the focus and commits
   its old text later → worked around in the window (guard); a library fix (reformat when
   `*v` changed and nothing was typed since) would help every app.
3. (done, U3) Popups may leave their window: the palette menu lists every palette file of the
   folder (up to 32; "Weitere Paletten…" only beyond that), a long menu scrolls.
4. No platform call to open a folder in the file manager (paint.net opens
   Explorer there) → it opens the Öffnen dialog in that folder.
5. (done, U3) After a press into the Colors window its focused track / wheel / combo box gets
   the keys it uses; other keys still reach the main window (library WINDOWS.md §7.10).
6. Not verified against a running paint.net 5.1 (not started / automated): layout sizes, the
   wheel orientation (library: red right, counter-clockwise), the default window place, the
   "Farbe hinzufügen" semantics (append + replace mode).  The labels are short usual words
   ("Mehr"/"Weniger", "Hex.:", "RGB", "HSV", "R:" …, "Primärfarbe"/"Sekundärfarbe").  Palette settings are not
   persisted (settings agent).
