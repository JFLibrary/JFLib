# Writing an app with jfguiLib (Rust)

This guide is for developers and AI models writing desktop apps on the Rust version of
jfguiLib. Start from `apps/template`, look up functions in [API.md](API.md) (generated from
the sources), and look at `apps/gallery` for every control in use. How widgets look is the
business of the linked style ([STYLES.md](STYLES.md)); packaging is in
[PACKAGING.md](PACKAGING.md). The library itself is described in `README.md`.

The examples below were compiled and run against the library (R13); identifiers such as
`ID_*`, `CMD_*` and the fields of `self` are the example app's own.

## 1. Rules of the code base

* **`#![no_std]`, no crates, no Cargo.** Only `core` and `alloc` may be `extern crate`d;
  `build_rs.py` calls `rustc` per crate and refuses `std` or a foreign dependency
  (`python build_rs.py check` runs the lint alone). Edition 2024, so `gen` is a keyword.
* **`unsafe` only in `rt`, `simd`, `plat` and `plugin`.** Every other library and style crate
  has `#![forbid(unsafe_code)]`; apps have `#![deny(unsafe_code)]`, whose only exception is
  the `ui::main!` entry point. The safe interfaces of those four crates are all an app needs.
* **No system libraries** beyond kernel32 / user32 / gdi32 on Windows (other DLLs only through
  `GetProcAddress`, inside `plat`), nothing but syscalls on Linux. Fonts, icons and codecs are
  the library's own.
* **Warnings fail the build** (`-D warnings`).
* **Float code keeps a fixed operation order** where results must be reproducible: no FMA,
  `rt::fx` for the SSE conversions (`cvtt`, `cvtd`, `minsd`, ...), `rt::x87` for the x87 steps.
  This matters for library code that must stay bit-identical to the assembly reference; app
  code only needs it when it promises reproducible output.
* Strings are UTF-8 everywhere; the apps' UI texts are German.
* Colours in the API are `0xAARRGGBB`, straight alpha. Images and layers are premultiplied
  (`gfx::FMT_PARGB`); effects work on straight `FMT_ARGB`.

## 2. Skeleton

**Start a new app with `build_rs.py new`.** It writes a complete app folder that builds and
passes its test as it is:

```
python build_rs.py new notes --title "Notizen"           # a window app (the default kind)
python build_rs.py new mytool --kind tool --style kde    # window + command line program + shared crate
python build_rs.py new report --kind console             # a console program (rt::main!)
python build_rs.py new syncd --kind service              # a background service (plat::service)
python build_rs.py test notes                            # build it, run tests/*.uis (Windows and Linux)
python build_rs.py run notes
```

| Kind | What it writes |
|---|---|
| `window` | `src/main.rs`: `ui::main!`, a command table with menu bar and shortcuts, a counter, an info dialog, a status bar with the app's and the library's version; `TARGETS` win linux web |
| `console` | `src/main.rs`: `rt::main!`, arguments, `--help`, `--version`, `--check` (the self-check its test runs) |
| `tool` | `src/main.rs` (a window that counts lines, words and characters), `cli/main.rs` (the same for files, `NAME-cli` in `BINS`), `crates/<name>_core/lib.rs` (the shared code, section 16) |
| `service` | `src/main.rs`: a work loop that a stop request ends, `service install / uninstall / start / stop / status` (`plat::service`), `--once` for the test |

Every kind also gets `PERMISSIONS` (the rules the skeleton needs; commented lines list the
others), `PACKAGE` (`version=0.1.0`, an `app_id` and the `homepage` from the git remote),
`STYLE` (window and tool, `--style`), `TARGETS`, `tests/smoke.uis` and a `README.md`. The name is
a letter followed by letters, digits, `_` or `-`; existing apps (in any case), library crates and
Rust keywords are refused. `--title` sets the window title and the package's display name.
`python build_rs.py new --lib NAME --deps rt,plat` creates a library crate `lib/NAME/src/lib.rs`
instead, enters it in `LIBS` of `build_rs.py` and in `tools/gendoc.py` and regenerates
[API.md](API.md). `apps/scaffoldtest` is the generator's `tool` output; the suite builds and
tests it, so the skeletons keep building.

The app's own version is `version=` in `PACKAGE`; the program reads it with
`rt::app_version!()`, the library's version is `ui::VERSION` ([VERSIONING.md](VERSIONING.md)).

`apps/template/main.rs`, the smallest complete app:

```rust
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use ui::*;

const ID_HELLO: u32 = 100;
const ID_RESET: u32 = 101;
const CMD_RESET: u32 = 1;

const DESC: AppDesc = AppDesc {
    title: "jfgui Template",
    flags: AF_RESIZABLE,
    w: 480,
    h: 320,
    min_w: 320,
    min_h: 240,
    ..AppDesc::NONE
};

#[derive(Default)]
struct Template {
    count: i32,
}

impl App for Template {
    // one vertical stack: title, text, two buttons
    fn frame(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_V | pad(24), 12);
        ui.text(TS_TITLE, "Hallo, Assembly!");
        ui.text(TS_BODY | TX_SECONDARY, &alloc::format!("Klicks: {}", self.count));
        ui.box_begin(0, BOX_H | BOX_XCENTER, 8);
        if ui.accent_button(ID_HELLO, "Klick mich") & EV_ACT != 0 {
            self.count += 1;
        }
        if ui.button(ID_RESET, "Zurücksetzen") & EV_ACT != 0 {
            ui.cmd_run(CMD_RESET, 0); // commands run after the frame
        }
        ui.box_end();
        ui.box_end();
    }

    fn command(&mut self, _ui: &mut Ui, id: u32, _arg: u32) -> bool {
        if id != CMD_RESET {
            return false;
        }
        self.count = 0;
        true
    }
}

ui::main!(ui::launch(&DESC, Template::default(), &style::STYLE));
```

* `AppDesc` holds the title, `AF_*` flags (`AF_RESIZABLE`, `AF_CUSTOM_TITLEBAR`, `AF_DARK`,
  `AF_LIGHT`), the initial and minimum client size in DIP, an accent colour (`0xRRGGBB`, 0 =
  the style's), the command table `cmds` and the menu bar `menubar` (section 6). Write
  `..AppDesc::NONE` so new fields keep their defaults.
* `ui::launch(desc, app, style)` owns the app. Native targets parse the command line and run
  headless (`--shot`, `--script`) or open a window; web targets retain it in the shared worker
  runtime. `ui::run` remains available for existing borrowed native apps. `style::STYLE` is
  whatever style crate the build linked as `style` (section 11).
* `ui::main!` expands to the `jfgui_main` symbol the platform entry point calls.

**Where an app's files live:**

| Path | Content |
|---|---|
| `apps/NAME/src/main.rs` | new modular crate root; keep app, UI and workflows in separate modules; existing flat `main.rs` projects stay supported |
| `apps/NAME/tests/*.uis` | test scripts (`apps/NAME/tests/*.uis` for scripts that only the Rust build has) |
| `apps/NAME/STYLE` | one word: the app's style (default `winui`) |
| `apps/NAME/TARGETS` | e.g. `win linux web`: explicit supported targets; owned GUI apps default to native plus web, native CLI probes to Windows/Linux |
| `apps/NAME/res/app.ico`, `app_{48,128,256}.png` | exe icon, Linux icons (`tools/mkicon.py`) |
| `apps/NAME/PACKAGE` | packaging metadata (PACKAGING.md); `version=` is the app's own version (`rt::app_version!()`, VERSIONING.md) |
| `apps/NAME/PERMISSIONS`, `BINS`, `assets/` | what it may do (PERMISSIONS.md), further programs (section 16), files built in as `app://` (VFS.md) |
| `apps/NAME/parity.txt` | parity settings, only for apps that also exist in `legacy/` |

The same source entrypoint is used by build/dependency discovery, hashes, lint and IDE/API
tools. Two competing roots (`main.rs` plus `src/main.rs`, or their library equivalents) are
an error. Reference material under `context` or `books` is never discovered as app source.
New reusable crates use `src/lib.rs`; existing app/library/style trees need no migration.

The common [agent and browser guide](AGENT_WEB.md) explains opt-in `--agent`, headless
JSON/MCP, `App::actions` and capability checks. `--target web` builds the same GUI into
`build/web/NAME/`. Use `plat::capabilities()` and asynchronous web requests for services that
depend on the host; never report a VFS write as durable before its completion arrives.

**Build and run:**

```
python build_rs.py app NAME                    # build/win/NAME.exe and build/linux/NAME
python build_rs.py app NAME --target win       # one target
python build_rs.py run NAME -- --shot out.png  # build and start (Linux: --target linux, via WSL)
python build_rs.py app NAME --style kde --out build/kde
python build_rs.py app NAME --release          # opt-level 2 + LTO
python build_rs.py app NAME --profile size     # smaller release variant (packages)
```

Every app crate gets every library crate (`ui`, `gfx`, `text`, `plat`, `base`, `rt`, `img_core`,
`img_*`, `img_all`, `fx`, ...) as an `--extern`, plus its style as `style`. Development builds
are incremental (about 1.5 s after an edit in `ui`). Use `--out build/<name>` when several
agents build at the same time.

**Platforms need no app code.** The same source builds a Windows exe and a static Linux ELF.
On Linux the library picks the display backend at start-up: Wayland when `WAYLAND_DISPLAY` is
set, else X11 via `DISPLAY` (`jfgui_BACKEND=x11|wayland` forces one). Apps call only `ui`,
`gfx`, `img_core`, `text`, `fx` and the neutral `plat` helpers (files, clock, threads). The only
platform-related choice is `AF_CUSTOM_TITLEBAR` (section 7); without it the system frame is used,
and on Wayland compositors without server-side decorations the library draws a title bar itself
(`jfgui_CSD=1` forces that).

## 3. The model: immediate mode

* `App::frame` runs for every pass and describes the whole UI by calling widget methods on
  `ui`. Nothing is retained between frames except your own data and small per-id state inside
  the library (animations, scroll positions, measurements).
* Every widget returns **`EV_*` flags**: `ui.button(id, "OK") & EV_ACT != 0` = clicked /
  changed / selected. Others: `EV_COMMIT` (value committed: drag released, Enter, focus lost),
  `EV_HOVER`, `EV_HELD`, `EV_FOCUS`, `EV_DOUBLE`, `EV_RCLICK`, `EV_OPEN`, `EV_DRAG`.
* Value widgets edit your variables through `&mut`: `ui.checkbox(id, "Label", &mut self.flag)`.
  Booleans of the API are `i32` (0 / 1, checkboxes also 2 = indeterminate), as in the assembly.
* After an action the library builds the frame once more, so everything shows the new state.
  Keep `frame` free of side effects other than reacting to `EV_*` flags.
* Heavy or structural work (file I/O, opening dialogs, big edits) goes through
  `ui.cmd_run(cmd, arg)`; it reaches `App::command(ui, id, arg)` between frames, in the order
  queued (the queue grows as needed; only beyond 65536 waiting commands does it drop new ones,
  with a `UIWARN`). Return `true` when you handled it. The library also delivers:

  | Id | When |
  |---|---|
  | `UIC_CLOSE` | the user closes the main window; return `true` to keep it (ask "save changes?" first) |
  | `UIC_TIMER` | `arg` = timer id of `ui.timer(id, ms)` (one-shot; library timers arrive too: ignore unknown ids) |
  | `UIC_ESCAPE` | Esc that no widget consumed |
  | `UIC_DPI`, `UIC_THEME` | the dpi / the theme changed |
  | `UIC_WINCLOSE` | `arg` = window id: the user closes a secondary window; `true` keeps it |
  | `UIC_DROP` | `arg` = window id: files (or text, see `drop_accept`) were dropped outside every drop target (`ui.drop_list()`, `ui.drop_data()`, `ui.drop_pos()`) |
  | `UIC_DRAG_END` | `arg` = `DE_*` effect (0 = cancelled / refused): a drag of the app ended (section 7, drag and drop) |
  | `UIC_WAKE` | `plat::wake()` from another thread (a background job finished) |
  | `UIC_EXIT` | the window loop ends (only after a real window ran: save settings here) |
  | `UIC_ACTIVE` | `arg` 1: one of the app's windows got the keyboard, 0: none has it (pause animations, poll less) |
  | `UIC_SUSPEND` | `arg` 1: the system goes to sleep (save, close connections), 0: it woke up; to keep it awake see `plat::inhibit` (section 21) |
  | `UIC_SESSION_END` | logout, shutdown or SIGTERM: save now; `UIC_EXIT` and the end follow |
  | `UIC_SETTINGS` | `arg` = `plat::SET_THEME \| SET_LOCALE \| SET_CONTRAST \| SET_FONTS \| SET_MONITORS`: system settings changed; a system theme and the format (`i18n::locale()`) are already updated; `SET_MONITORS`: a monitor came, went, moved or changed its scale or work area (`ui.monitors()`, section 21) |
  | `UIC_HOTKEY`, `UIC_HOTKEY_LOST` | `arg` = id: a global shortcut was pressed anywhere / the desktop dropped it (`ui.hotkey_error(id)`; section 21) |
  | `UIC_PICK` | `arg` 1: the user picked a colour on the screen (`ui.picked_color()`), 0: cancelled or failed (`ui.screen_error()`) |
  | `UIC_SCREENSHOT` | `arg` 1: the picture of `ui.screenshot(..)` is there (`ui.screenshot_take()`), 0: failed |

* `App::init(ui)` runs before the first frame (register codecs, read arguments, load
  settings); `false` ends the program with exit code 2.
* **Ids** are plain `u32` constants, unique among the widgets of a frame (duplicate ids break
  hover / press tracking). Repeated items: `BASE + i`. Bits 28..31 are reserved for the
  library (`part_id`, split-button chevrons). A box id may be 0 (automatic). Some widgets also
  take the ids after their own: a segmented control, tabs, wizard steps and breadcrumbs one
  per item (`id + 1 + i`), an infobar `id + 1` (its close button), list and grid rows
  `id + 1 + i` -- leave that room before the next widget's id.
* **Ids from names** spare the bookkeeping: `ui.id("save")` hashes a name, `ui.id_n(i)` a
  number, both inside the current scope; `ui.id_scope(i, |ui| ...)` (or `id_push(i)` /
  `id_push_str(name)` ... `id_pop()`) gives every row of a loop its own scope, so the same
  names can repeat. `ui::wid("save")` and `ui::wid_path(&["layers", "add"])` are the same
  values as `const`s (window ids, `#id` selectors in scripts). Hashed ids start at
  `ID_HASH_BASE` (0x0800_0000): keep hand-written ids below it and both kinds mix. Scripts
  and `jfgui_DEBUG=1` warn when two names hash alike, when one id is used twice (naming it)
  and when a scope is still open at the end of the frame.

  ```rust
  for (i, item) in self.items.iter().enumerate() {
      let del = ui.id_scope(i as u32, |ui| {
          ui.text(TS_BODY, &item.name);
          ui.button(ui.id("del"), "Löschen") & EV_ACT != 0
      });
      if del { /* remove item i after the loop */ }
  }
  ```

## 4. Layout

```rust
ui.text(TS_TITLE, "Hallo");
ui.box_begin(0, BOX_H | BOX_XCENTER, 8); // a row, children centred vertically, 8 DIP gap
ui.text(TS_BODY, "Name:");
ui.next_size(-1, 0); // the text box takes the remaining width
ui.textbox(ID_NAME, Some("Name eingeben"), &mut self.name, 64);
ui.next_size(120, 0); // 120 DIP wide, natural height
ui.button(ID_OK, "OK");
ui.box_end();
// a card with a 16 DIP padding
ui.box_begin(0, BOX_V | BOX_CARD | PAD_16, 4);
ui.text(TS_BODY_STRONG, "Karte");
ui.text(TS_CAPTION | TX_SECONDARY | TX_WRAP, "Karten gruppieren Inhalte.");
ui.box_end();
```

* `box_begin(id, flags, gap)` / `box_end()`: children flow along the main axis (`BOX_H` left to
  right, `BOX_V` top to bottom). Padding: `| PAD_8`, `| pad(24)` (DIP).
* `next_size(w, h)` sizes the next widget or box in DIP: `> 0` fixed, `0` natural, `< 0` fill
  weight (`-1` = take the remaining space). The cross axis stretches by default;
  `BOX_XCENTER` / `BOX_XSTART` / `BOX_XEND` give children their natural cross size.
* Main axis: `BOX_CENTER`, `BOX_END`; `spacer(dip)` (`-1` = flexible).
* `BOX_WRAP` (horizontal boxes) for button rows, legends, tile palettes: a child that does not
  fit after the others of its row starts a new row (a row's first child stays, however wide).
  Each row is as tall as its tallest child; the cross alignment (`BOX_XCENTER` ..., stretched
  children take the row's height) and `BOX_CENTER` / `BOX_END` apply per row, and the gap also
  separates the rows. The natural height is the rows plus their gaps, the natural width one row
  with every child: the box wraps where its parent gives it less (stretched across a column, a
  fill share, a fixed width, or at most the room left in a row). Fill children share the rest
  only while everything fits on one row, else they keep their natural size.

  ```rust
  ui.box_begin(0, BOX_H | BOX_XCENTER | BOX_WRAP, 8);
  for (i, tag) in self.tags.iter_mut().enumerate() {
      ui.toggle_button(ID_TAG + i as u32, &tag.name, &mut tag.on); // on: i32
  }
  ui.box_end();
  ```
* Absolute placement: `next_rect(x, y, w, h)` in DIP relative to the current box (overlays,
  floating panels over a canvas); `next_rect_px` in window pixels.
* Decoration and behaviour: `BOX_CARD`, `BOX_LAYER` (command bars), `BOX_BG`, `BOX_CLIP`,
  `BOX_BLOCK` (swallows clicks), `BOX_DISABLED` (everything inside disabled). `BOX_FIXED` for
  fill boxes whose size never depends on their content (fewer layout passes when deeply nested).
* Sizes are measured over passes: a new layout settles within 2-3 internal passes; you never
  see intermediate states.
* `last_rect()` is the physical-pixel rectangle `[x, y, w, h]` of the last widget,
  `box_rect()` the content rectangle of the current box.
* Units: everything you pass to layout and widgets is DIP (96 dpi = 100 %); drawing calls and
  rectangles the library returns are physical pixels. `ui.dp(dip)` converts, `ui.hair()` is a
  one-line width for the current dpi. Test with `--scale 150`.
* **Grid box** (`lib/ui/grid.rs`) for forms and tables of controls: one entry per column, DIP
  `> 0` fixed, `0` its widest child, `< 0` a share of the width left; rows grow with their
  tallest child (`gridbox_rows(&[..])` sets them like columns). Children fill the cells row by
  row; `next_span(cols, rows)`, `next_cell(col, row)` and `next_align(AL_START | AL_CENTER |
  AL_END | AL_STRETCH, ..)` steer one child, `GRID_H_*` / `GRID_V_*` in the flags all of them
  (default: stretched across, centred vertically). The flags also take the box decorations.
  A `TX_WRAP` text wraps at its cell's width and its row grows with the lines (labels and
  values: `gridbox_begin(id, &[92, -1], 0, 6)`, then `text` / `text(TS_BODY | TX_WRAP, ..)`
  pairs).

  ```rust
  ui.gridbox_begin(ID_FORM, &[0, -1, 120], BOX_CARD | pad(16), 8);
  ui.text(TS_BODY, "Name");
  ui.textbox(ID_NAME, None, &mut self.name, 64);
  ui.button(ID_PICK, "Wählen …");
  ui.text(TS_BODY, "Beschreibung");
  ui.next_span(2, 1); // the field takes the last two columns
  ui.textbox(ID_DESC, None, &mut self.desc, 256);
  ui.gridbox_end();
  ```
* **Animations**: `ui.anim(id, target, ms)` moves a value (0..256) towards `target` with an
  ease-out curve; `ui.anim_ex(id, target, ms, EASE_*)` takes another curve (`EASE_LINEAR`,
  `EASE_IN`, `EASE_IN_OUT`, `EASE_OUT_CUBIC`, `EASE_OUT_BACK` overshoots a little), and
  `ui.anim_color(id, argb, ms, curve)` fades a colour (hover fills, state colours). `ease(curve,
  t)` is the bare curve. With "reduce motion" (`ui.anim_off()`) every value jumps to its target.

## 5. Widgets

All of them are `Ui` methods (API.md, sections Widgets, Text boxes, Lists, Values). Labels are
copied, so they may come from temporary `String`s.

**Text and buttons**

```rust
ui.box_begin(0, BOX_H | BOX_XCENTER, 12);
if ui.accent_button(ID_APPLY, "Übernehmen") & EV_ACT != 0 {
    ui.cmd_run(CMD_RESET, 0); // the work runs in App::command after the frame
}
ui.next_flags(ST_DISABLED);
ui.button(ID_APPLY + 1, "Deaktiviert");
ui.next_icon(IC_ADD);
ui.button(ID_APPLY + 2, "Mit Icon");
ui.icon_button(ID_APPLY + 3, IC_SAVE, "Speichern"); // the label becomes the tooltip
ui.cmd_button(CMD_SAVE, ST_ICON_ONLY | ST_SUBTLE);   // a button for a command (section 6)
ui.tooltip("Speichert das Bild");                     // tooltip of the last widget
ui.box_end();
```

* `text(TS_* | TX_*, s)`: text styles `TS_CAPTION`, `TS_BODY`, `TS_BODY_STRONG`, `TS_SUBTITLE`,
  `TS_TITLE`, `TS_TITLE_LARGE`, `TS_DISPLAY`; flags `TX_SECONDARY`, `TX_TERTIARY`, `TX_ACCENT`,
  `TX_WRAP` (wrap at the box width), `TX_HYPHENATE` (with `TX_WRAP`: hyphenate words that do
  not fit, section 11), `TX_ELLIPSIS` (cut with "…"; with `TX_WRAP` at the height of
  `next_size`), `TX_CENTER`, `TX_RIGHT`. `heading(level, TS_* | TX_*, s)` draws the same text
  as a section title that screen readers know as a heading of that level (1 the top; their
  heading navigation finds it, section 19).
* Buttons: `button`, `accent_button`, `icon_button(id, IC_*, label)`, `toggle_button(id, label,
  &mut v)`, `tool_button(id, IC_*, label, &mut sel, mine)` (tool palettes), `cmd_button(cmd, ST_*)`.
* Modifiers for the next widget: `next_flags(ST_DISABLED | ST_SUBTLE | ST_ACCENT | ST_CHECKED |
  ST_LARGE | ST_COMPACT | ST_NOFOCUS | ST_ICON_ONLY)`, `next_icon(IC_*)`. Post-modifiers for the
  last widget: `tooltip(s)`, `context_menu(menu)`; for one part of it (an item on a canvas)
  `tooltip_part(s, part)`: the delay starts again when the part changes.
* `icon(IC_*, size_dip)`, `separator()`, `progress(id, v, max)`.

**Selection and ranges**

```rust
ui.checkbox(ID_SHOW, "Anzeigen", &mut self.show);
for (k, label) in ["Rot", "Grün", "Blau"].iter().enumerate() {
    ui.radio(ID_MODE + k as u32, label, &mut self.mode, k as i32);
}
ui.switch(ID_SHOW + 1, "WLAN", &mut self.wlan);
ui.next_size(200, 0);
if ui.slider(ID_SIZE, &mut self.size, &RG_SIZE) & EV_COMMIT != 0 {
    // released: a good moment for expensive work
}
ui.number_box(ID_SIZE + 1, &mut self.size, &RG_SIZE);
```

with `const RG_SIZE: Range = Range::new(1, 100, 1, 0);` (min, max, step, `RG_*` flags:
`RG_VERT`, `RG_TICKS`, `RG_TIP`). A vertical slider needs a height: `next_size(60, 100)`.

**Text input**

* `textbox(id, Some("placeholder"), &mut string, cap | TB_*)` -> `EV_ACT` = changed,
  `EV_COMMIT` = Enter / focus lost. `cap` is the byte capacity; flags `TB_PASSWORD`,
  `TB_READONLY`, `TB_DIGITS` (+ `TB_SIGNED`, `TB_DECIMAL`), `TB_MULTI` (+ `TB_WRAP`; give it a
  height with `next_size`; + `TB_HYPHENATE`: hyphenation, section 11), `TB_SPELL` (spell
  checking, below). The capacity takes bits 0..22 (at most 8 MB).
* Formatted text (bold, lists, links ...) and source code with highlighting: the editors
  `ui.richtext` and `ui.code_editor` (section 28).
* `number_box(id, &mut i32, &Range)`, `number_box_f(id, &mut f64, &RangeF)` with
  `RangeF::new(min, max, step, decimals, flags)`; `ui.fmt_f64(v, decimals)` /
  `ui.parse_f64(s)` use the UI's decimal separator (default `,`; `set_decimal_sep`).
* `clip_set` / `clip_get` / `clip_set_image` / `clip_get_image` reach the clipboard (headless
  runs use a private one). On Android that is the system's `ClipboardManager`: text and HTML
  go to other apps, an image as a PNG through the app's `GalleryFiles` provider (declared in
  the manifest; without it images stay in the app), the library's RTF / DOCX forms stay in the
  app and count while the system's clip is the one it set. Android lets only the focused app
  read the clipboard, so a paste right after the system's picker may find it empty.
* Text boxes follow Unicode's segmentation (`text::segment`, UAX #29 / #14, Unicode 15.0): the
  arrows and Delete move over whole characters as the user sees them (an accented letter in two
  code points, an emoji family, a flag); Backspace removes an emoji sequence whole and of other
  characters the last code point; Ctrl+arrows and a double click work on words (`can't` and
  `3.14` are one word, runs of punctuation one unit); wrapped boxes and `TX_WRAP` texts break
  lines after spaces and hyphens, between ideographs and never inside a number or before a
  closing bracket; after a soft hyphen (U+00AD, invisible inside a line) they draw a hyphen.
  Your own text code can use `text::segment::{graphemes, word_bounds, word_at, line_breaks,
  wrap_end, wrap_end_hyph}` (`wrap_end_hyph`: soft hyphens and a hyphenation hook such as
  `spell::Hyphenator::hyphenate`).

**Spell checking.** A text box with `TB_SPELL` (one line or `TB_MULTI`) underlines misspelled
words with a red wavy line; a right click on one -- or the menu key / Shift+F10 with the caret
in it -- offers up to five suggestions, "Zum Wörterbuch hinzufügen" and "Ignorieren". The word
being typed is marked only once the caret leaves it; password, read-only and digit boxes are
never checked; the app's own `context_menu()` still opens on unmarked words.

```rust
ui.textbox(ID_BODY, None, &mut self.body, 4096 | TB_MULTI | TB_WRAP | TB_SPELL);
ui.spell_enable(self.settings.spelling);      // the switch for the whole UI (default on)
ui.spell_language("en-GB");                   // default "": the user's languages
let bad = ui.spell_errors(ID_BODY);           // misspelled byte ranges, as last checked
if let Some(sp) = ui.speller() { sp.add("JFLib"); } // spell::Speller: check, suggest, add, ignore
```

* The checker (crate `spell`): Windows' own (`ISpellChecker`) when it knows the language, else
  Hunspell dictionaries the library reads itself -- Linux `/usr/share/hunspell`, `myspell`,
  `~/.local/share/hunspell`, `DICPATH`; then `config://hunspell` and the app's own in
  `apps/NAME/assets/hunspell` (`de_DE.aff` + `de_DE.dic`; Web and Android only these).
  `spell_sources(SRC_APP)` limits it, `spell_set(Speller::with_dictionary(..))` sets one.
* "Zum Wörterbuch hinzufügen" writes `config://spell/<language>.dic` (one word per line);
  "Ignorieren" lasts for the session. Headless runs read only `app://hunspell` and keep both
  in memory, so scripts give the same result everywhere.
* Suggestions from a Hunspell dictionary are made on a worker (a long German compound takes up
  to a few tenths of a second): the menu opens at once with "Vorschläge werden gesucht …" and
  fills when they are there; Windows' checker answers at once. Headless runs make them at once
  too; `ui.spell_suggest_async(true)` lets a script see the waiting menu (`sync` waits for
  the worker, `apps/spelltest/tests/async.uis`).
* Checking is cached per text and per word: typing re-checks only new words; a long text is
  checked in slices of at most 6 ms per frame (`spell_state()` is `SPELL_LOADING` while a
  dictionary is read in the background).
* The Hunspell reader follows Hunspell 1.7 (affixes, compounds, `COMPOUNDRULE`, case rules);
  it was compared with real de_DE / en_US / en_GB and twelve more dictionaries
  (docs/ROADMAP.md 4).
* Tests: `apps/spelltest` (`--test engine` for the Hunspell rules, scripts for marks, menu,
  word list, typing, language; `--compare AFF DIC WORDS [--sug N]` checks a word list with any
  dictionary, e.g. to compare it with Hunspell), `plattest --test spell` (the real
  `ISpellChecker`).

**Values and colours**

```rust
const RF_ZOOM: RangeF = RangeF::new(10.0, 800.0, 0.0, 1, 0);

ui.next_size(200, 0);
ui.slider_f(ID_ZOOM, &mut self.zoom, &RF_ZOOM);
ui.number_box_f(ID_ZOOM + 1, &mut self.zoom, &RF_ZOOM);
ui.angle(ID_ANGLE, &mut self.angle, 0);
ui.swatch(ID_COLOR, self.color, false);
ui.color_picker(ID_COLOR + 1, &mut self.color);
```

Also `prop_slider(id, label, &mut v, &range, default)` / `prop_slider_f` (paint.net-style
property rows with a reset button), `track_slider(id, &mut v, &range, Some(&track))` (a slider
over a gradient: `Track::new(TK_*, &stops)`), `pan(id, &mut x, &mut y, thumb, ver, flags)`,
`color_picker_ex(id, &mut argb, CP_ALPHA | CP_FIELDS)`, `color_wheel(id, &mut hsv_or_argb,
CW_HSV)`; `hsv_to_rgb` / `rgb_to_hsv` convert (hue in tenths of a degree, S / V 0..1000).
`CP_PIPETTE` adds a pipette button beside the preview where the user can pick a colour anywhere
on the screen (Windows, X11, the Wayland / Flatpak screenshot portal, Chromium's EyeDropper; not
shown elsewhere): the picked colour (alpha kept) comes back as `EV_ACT | EV_COMMIT` of the
picker's next pass. The pick itself is `ui.pick_color()` (section 21).

**Containers**

```rust
ui.tabs(ID_TABS, &mut self.tab, &["Allgemein", "Erweitert"]);
if self.tab == 0 {
    ui.next_size(0, 200);
    let (first, end) = ui.list_begin(ID_LIST, &mut self.sel, 1000, 0); // 1000 rows, default height
    for i in first..end {                                              // only the visible rows
        if ui.list_item(i, &format!("Eintrag {}", i + 1)) & EV_DOUBLE != 0 {
            ui.cmd_run(CMD_OPEN, i as u32);
        }
    }
    let (_ev, _v) = ui.list_end(&mut self.sel);
} else {
    if ui.expander_begin(ID_EXP, "Erweiterte Einstellungen", &mut self.exp) != 0 {
        ui.checkbox(ID_AUTO, "Automatisch speichern", &mut self.auto);
    }
    ui.expander_end();
}
```

* `scroll_begin(id, SC_V | SC_H | SC_NOBAR | BOX_* flags)` / `scroll_end()`: a scroll view (give it
  a size, e.g. `next_size(0, -1)`); `scroll_to`, `scroll_pos`, `scroll_extent`.
* `list_begin(id, &mut sel, count, row_h | LF_REORDER)` -> `(first, end)`: emit rows
  `first..end` with `list_item(i, label)` or `list_item_begin(i)` ... any widgets ...
  `list_item_end()`; `list_end(&mut sel)` -> `(EV_*, value)`: `EV_ACT` selection changed,
  `EV_DOUBLE`; with `LF_REORDER`, `EV_DRAG` and value = `from | to << 16` (the gallery's
  "Listen" section moves the item).
* `tabs(id, &mut sel, names)`, `expander_begin(id, header, &mut open)` (non-zero: emit the
  content) / `expander_end()`, `scrollbar(id, &mut pos, content, view, vertical)` for custom
  canvases.
* Only what changed is drawn (dirty regions: every command is hashed into 32 × 32 px cells and
  compared with the last frame). When a scroll view's content only moved, the pixels it still
  shows are **copied** in the framebuffer and just the band that came into view (and what
  really changed) is drawn -- byte for byte what a full redraw gives. Scroll views, lists,
  data views, `vlist`, the document and code views and multi-line text boxes do this. A
  widget of your own that scrolls its content by drawing it at `origin - offset` can join in:
  call `ui.scroll_copy_begin(id, ox, oy)` right after its `clip_push` (`ox`, `oy`: window px
  of the content's origin, i.e. its place minus the scroll offset; `i64` for long contents)
  and `ui.scroll_copy_end()` before the `clip_pop`. Content that does not move with the
  origin (a sticky header) is simply drawn again; text, icons and images are copied, rounded
  corners, circles, lines and `Draw` objects are drawn again (the rasterizer works in absolute
  coordinates). Whether a frame copies is measured per view: copying costs memory bandwidth, so
  flat content (cards, backgrounds) is cheaper to fill again.

**Images, custom drawing, canvases**

* `image(id, &Rc<Surface>, ver)` shows a surface at its natural size (or `next_size`); bump
  `ver` when its pixels change (the dirty-region renderer compares it).
* Drawing between widgets, in physical pixels on the current layer: `draw_color(argb)`,
  `draw_rect`, `draw_rrect`, `draw_frame`, `draw_text(x, y, s, FONT_* | size_fx << 8)`,
  `draw_icon(IC_*, x, y, size_fx)`, `draw_circle` / `draw_line` (24.8 fixed point = px × 256),
  `draw_image(&rc, ver, x, y, w, h)`, `draw_clip` / `draw_unclip`. Colours of the theme come
  from `ui.color(T_*)`.

  ```rust
  let r = ui.last_rect();
  let c = ui.color(T_ACCENT);
  ui.draw_color(c);
  ui.draw_rect(r[0], r[1] + r[3] - ui.hair(), r[2], ui.hair());
  ```

* `canvas(id, Some(draw), user, version)` is a custom area with full pointer input. The draw
  object implements `ui::Draw` and runs at render time with the clip set to the widget; it gets
  a `Gfx` (the draw context `dc`, the target surface `sf`, the text engine; `g.shapes()` for
  anti-aliased rounded rectangles, circles and lines). It always runs on the UI thread (large
  redraws draw the other commands on several threads, section 9), and the clip may be only a
  part of the widget (a dirty rectangle, a row band): draw the same pixels whatever the clip.
  Share the data it draws through `Rc` / `RefCell` / `Cell`:

  ```rust
  struct Board {
      img: RefCell<gfx::Surface>,
  }

  impl Draw for Board {
      fn draw(&self, g: &mut Gfx, r: [i32; 4]) {
          g.dc.set_color(0xFFFF_FFFF);
          g.dc.fill(g.sf, r[0], r[1], r[2], r[3]);
          g.dc.blit(g.sf, &self.img.borrow(), r[0], r[1]);
          g.dc.set_color(0xFF00_78D4);
          g.shapes().stroke_rrect(r[0] + 4, r[1] + 4, r[2] - 8, r[3] - 8, 6, 2);
      }
  }

  // in frame (board: Rc<Board>, board_ver: u32):
  ui.next_size(320, 200);
  let d: DrawRef = self.board.clone();
  let inp = ui.canvas(ID_CANVAS, Some(d), 0, self.board_ver).clone();
  if inp.ev & EV_HELD != 0 && inp.btn & (1 << BTN_LEFT) != 0 {
      let mut img = self.board.img.borrow_mut();
      let mut dc = gfx::Dc::new(img.w, img.h);
      dc.set_color(self.color);
      for p in &inp.pts {                       // every move point since the last frame
          dc.fill(&mut img, p[0] - 2, p[1] - 2, 5, 5);
      }
      self.board_ver += 1; // the content changed: the canvas must be redrawn
      ui.repass();         // ... in this frame (its command carries the old version)
  }
  ```

  `UiInput` has the position relative to the widget (`x`, `y`, `press_x`, `press_y`), buttons
  (`btn`, `pressed`, `released`, bit `1 << BTN_*`), `wheel_x` / `wheel_y` (120 per notch),
  `mods`, all move points `pts`, the first unconsumed key, and pen data (`pressure`, `ptype`,
  `ppress`, tilt; `legacy/ASMLIB/docs/modules/PEN_INPUT.md`). The cursor shape comes from
  `next_flags(CUR_* << HB_CURSOR_SHIFT)`. `ui.canvas` returns a borrow of `ui`: clone the
  input before calling more `ui` methods. `canvas_damage(id, x, y, w, h)` redraws a part
  without a version change.
* Direct pixel work (paint programs): `gfx::Surface::new(w, h, FMT_PARGB)`, a `gfx::Dc`
  (`fill`, `blit`, `blit_scaled`, clip and origin), `gfx::path` / `raster` / `stroke` for
  anti-aliased vector paths, `gfx::blendmode`, `gfx::tmask` (selection masks), `gfx::flood`,
  `gfx::gradient`; the text engine in the field `ui.text` (`text::Text`) draws text into any
  surface (`set_font`, `draw`, `measure`).
  `img_core::ops` rotates, flips, scales and crops, `img_core::resample` resizes with filters,
  `img_core::xform` applies affine transforms.

## 6. Commands, menus, accelerators, popups

Put repeated actions into a **command table** and reference them from menus, buttons and
accelerators; the library then does shortcuts, disabled / checked states and tooltips:

```rust
const CMD_OPEN: u32 = 1;
const CMD_SAVE: u32 = 2;
const CMD_UNDO: u32 = 3;
const CMD_GRID: u32 = 4;
const CMD_RESET: u32 = 5;
const CMD_RECENT0: u32 = 20; // .. 27

static CMDS: [Command; 4] = [
    //           id         label           icon          mods      key            CF_* state
    Command::new(CMD_OPEN, "Ö&ffnen…", IC_OPEN_FILE, MOD_CTRL, b'O' as u32, 0),
    Command::new(CMD_SAVE, "&Speichern", IC_SAVE, MOD_CTRL, b'S' as u32, 0),
    Command::new(CMD_UNDO, "&Rückgängig", IC_UNDO, MOD_CTRL, b'Z' as u32, CF_DISABLED),
    Command::new(CMD_GRID, "&Raster", IC_GRID, MOD_CTRL, b'G' as u32, 0),
];
static MENUBAR: [Mi; 2] = [Mi::Menu("&Datei", &M_FILE), Mi::Menu("&Ansicht", &M_VIEW)];
static M_FILE: [Mi; 5] = [
    Mi::Cmd(CMD_OPEN),
    Mi::Cmd(CMD_SAVE),
    Mi::Sep,
    Mi::Sub("&Zuletzt geöffnet", &M_RECENT),           // a submenu
    Mi::Item(CMD_RESET, "&Zurücksetzen", 0),           // a plain row: App::command(id, 0)
];
static M_RECENT: [Mi; 1] = [Mi::List(CMD_RECENT0, 8, "Keine Dateien")];
static M_VIEW: [Mi; 1] = [Mi::Cmd(CMD_GRID)];

const DESC: AppDesc = AppDesc { title: "Mein Werkzeug", flags: AF_RESIZABLE, w: 900, h: 640,
    cmds: &CMDS, menubar: &MENUBAR, ..AppDesc::NONE };

// in frame, before the widgets:
ui.cmd_state(CMD_GRID, if self.grid { CF_CHECKED } else { 0 });
ui.menu_list(CMD_RECENT0, &self.recent); // the rows of Mi::List (Vec<String>)
ui.menubar(None);                        // the menu bar of the AppDesc
```

* A chosen menu row, a `cmd_button` and an accelerator all queue `App::command(cmd, 0)`.
  `&` marks the mnemonic (`&&` = a literal `&`). Keys are `VK_*` (Win32 numbering on every OS,
  `ui::keys`), modifiers `MOD_CTRL | MOD_SHIFT | MOD_ALT`.
* `cmd_state(cmd, CF_DISABLED | CF_CHECKED | CF_HIDDEN)` is persistent; set it every frame from
  your model. Accelerators fire only when no widget consumed the key first (`ui.key(VK_*)`).
  `accel_text(cmd)` gives the shortcut as shown in menus ("Strg+S").
  An editor that owns the whole document history can call `capture_shortcut(vk, mods)`
  before its widgets to reserve an exact chord, including one normally owned by a textbox.
  Guard modal draft/file dialogs so their text-editing shortcuts retain priority. jfWord
  uses this opt-in API for document undo/redo and inserting a paragraph.
* `textbox_selection(id)` returns the focused textbox's UTF-8 byte offsets `(anchor, caret)`.
  Read it after emitting that textbox. `textbox_select(id, text, anchor, caret)` restores a
  selection on an already focused textbox; pass its exact current text. Invalid byte
  boundaries and inactive fields return `false`. Document editors can use these opt-in APIs
  to format a selection or highlight a search result without changing ordinary textbox undo.
* `textbox_surface(id, &mut text, cap_flags, &lines)` adds input over externally rendered
  rich text. Each `TextSurfaceLine` supplies UTF-8 start/end offsets, an absolute pixel
  rectangle and one caret stop per scalar boundary. Lines can span multiple pages and
  have different alignment and font metrics. The caller draws text and owns scrolling;
  the widget draws selection/caret and provides keyboard, clipboard and IME handling.
  `textbox_surface_change()` returns the original replacement range and inserted text,
  including the actual insertion position when neighboring characters repeat. Apply
  this to the document model atomically, reject protected fields there, and rebuild layout
  after `EV_ACT`. `apps/jfWord/rich_ui.rs` shows integration with `doc_layout` source ranges.
  The buffer may span an entire story, with paragraph offsets from `doc_core::StoryText`.
  Surface paste normalises CRLF and lone CR to LF; the model decides which inserted breaks
  create paragraphs. Surface replacements exceeding capacity are rejected before deletion,
  not truncated. Set capacity for the displayed projection, including expanded fields and
  the terminator. These opt-in rules leave ordinary textbox behaviour unchanged.
* `clip_set_many(&[(format, bytes), ...])` publishes up to four representations of one
  selection: `CLIP_TEXT`, `CLIP_HTML`, `CLIP_RTF` and `CLIP_DOCX`. It returns `0` on success
  and `-1` on failure; only remove a cut selection after success. Text and HTML must be
  NUL-free UTF-8; RTF and the private document payload are binary. Combined data is bounded
  to 64 MiB. The platform handles CF_HTML framing on Windows and MIME offers on Linux;
  callers supply raw HTML fragments. `clip_has` probes and `clip_get` retrieves each format.
  `CLIP_DOCX` carries `doc_io`'s versioned document fragment, not an arbitrary DOCX file.
  `doc_core::Document::copy_story_range` and `Editor::replace_story_fragment` preserve
  formatting, sources and note bodies with atomic document history. `doc_io` also provides
  the bounded RTF reader/writer and HTML writer. See `apps/jfWord/rich_ui.rs` for shortcut
  interception and format preference. Ordinary textboxes continue to paste plain text;
  plain-text/image writes clear older rich representations. Scripts use an isolated
  in-process clipboard and never change the user's system clipboard.
* `textbox_presented(id, placeholder, &mut text, cap_flags, presentation)` uses the same
  native editing, selection, clipboard and IME path with an explicit `TextboxPresentation`.
  It draws a borderless field with the supplied font, physical-pixel font size (24.8), line
  height, baseline, foreground/background and underline. The app owns page geometry and
  document history; ordinary `textbox` calls retain their existing style and rendering.
* `RibbonGroup::new(id, caption, preferred_width, compact_width)` describes a command group
  in DIP; reserve `id..=id+3`. Start with `ribbon_begin(id, &groups)`, then call
  `ribbon_group_begin(&layout, index, &group)` for each group. Emit controls only when it
  returns `Some(mode)`, followed by `ribbon_group_end(mode, caption)`. Finish with
  `ribbon_end()`. Groups first shrink, then collapse into labeled flyouts from the right.
  `ribbon_button` provides compact icon actions; `ribbon_tile` adds typography previews.
  The app still provides real command availability and actions. See `apps/jfWord/ribbon_ui.rs`.
* Flyouts of your own: a button's `EV_ACT` -> `popup_open(id)` (drop-down buttons do this
  themselves); then `flyout_begin(id, FL_*)` -> `true`: emit `menu_item(id, label)`,
  `menu_cmd(cmd)`, `menu_sep()` or any widgets; always `flyout_end()`:

  ```rust
  ui.combobox(ID_SHAPE, &mut self.shape, &["Kreis", "Quadrat", "Dreieck"]);
  ui.dropdown_button(ID_MORE, "Mehr");
  if ui.flyout_begin(ID_MORE, FL_BELOW) {
      if self.more != 0 {
          ui.next_flags(ST_CHECKED);
      }
      if ui.menu_item(ID_MORE_A, "Option A") & EV_ACT != 0 {
          self.more ^= 1;
      }
      ui.menu_sep();
      ui.menu_cmd(CMD_SAVE);
  }
  ui.flyout_end(); // always, also when flyout_begin returned false
  ui.button(ID_RCLICK, "Rechtsklick hier");
  ui.context_menu(&M_CTX); // right click / menu key / Shift+F10 on the last widget
  ```

  A widget without `context_menu` gets the menu key as a right click (`EV_RCLICK`); a
  `menu_popup` in answer to it opens at the widget (§27).

* Drop-downs: `dropdown_button`, `split_button` (main part `EV_ACT`, chevron opens popup `id`),
  `combobox(id, &mut sel, list)`; `menu_popup(menu)` opens a menu at the pointer.
* Popups (flyouts, menus, combo lists, the calendar of `date_picker`, context menus, tooltips)
  are not limited to their window: on Windows and X11 they may use the work area of the
  monitor -- below / to the right of their anchor when there is room, above it / to the left
  (submenus, context menus) when not -- and what lies outside the window becomes a borderless
  popup window of the platform (owned, never activated, input comes back as the window's
  events, so nothing changes for the app). Wayland does not tell where a window is: a popup
  that fits stays inside the window, a larger one becomes an `xdg_popup` that the compositor
  keeps on the screen; the UI lays it out where the compositor put it. Popups close on a press
  elsewhere in the app, with Esc, and when the app loses the keyboard. Headless runs keep the
  main window's popups inside it unless a script sets a virtual screen (`screen X Y W H`, §12).
* Timers: `ui.timer(id, ms)` -> `App::command(UIC_TIMER, id)` once; start it again for a
  periodic timer (the same id replaces the waiting one). Library timers (caret, tooltips) arrive
  as well: ignore ids you did not start. Any number may wait (up to 4096 ids); due timers fire
  by their deadlines, equal deadlines in the order they were set.
* Limits: the queues and stacks of a pass grow as needed (keys of one pass, commands, timers,
  pointer positions, hit boxes, widget records, clips, tabs, canvases); their ceilings only
  stop runaway memory. Where a ceiling or a fixed nesting depth (windows, popups, menus inside
  each other) holds something back, a `UIWARN` names it once (scripts: `expect_warn`). The
  platform's event ring holds 256 events; a larger burst (synthetic keys, a flooding client)
  waits behind it in order, up to 65536 events (`plat::evq_dropped()` counts what is lost
  beyond).

## 7. Dialogs, windows, title bar, files

**Content dialogs** are drawn on the window (smoke layer, modal). The library keeps the `open`
flag (`Shared<i32>`: closing by Esc or a button clears it):

```rust
// state: dlg_open: Shared<i32> (Shared::new(0)); open it with self.dlg_open.set(1)
let dlg = self.dlg_open.clone();
if ui.dialog_begin(ID_DLG, "Änderungen speichern?", &dlg) {
    ui.text(TS_BODY | TX_WRAP, "Möchten Sie die Änderungen speichern?");
    match ui.dialog_buttons(Some("Speichern"), Some("Nicht speichern"), Some("Abbrechen")) {
        DLG_PRIMARY => ui.cmd_run(CMD_SAVE, 0), // do the work in App::command
        DLG_SECONDARY => ui.quit(),
        _ => {}                                 // DLG_NONE, DLG_CLOSE
    }
}
ui.dialog_end(); // always
```

* Emit dialogs and windows at the end of `frame` (on top of everything else).
  `ui.modal_active()` tells whether a dialog is open (for shortcuts of your own).
* **Undo / redo** of the app's own state: `ui::History<S>` records `Edit`s (apply / revert /
  label, optionally `merge` so a drag is one step); `Snapshot::new(label, before, after)` is the
  edit for small `Clone` states, `.merging(key)` joins steps with the same key. `h.apply(&mut
  state, Box::new(edit))`, `h.undo(&mut state)`, `h.redo(..)`, `undo_label()` for the menu
  ("Rückgängig: …"), `undo_value()` / `redo_value()` (the step as data, `Edit::value`: an app
  can see what kind of edit it is before it runs), `group_begin` / `group_end` for compound
  steps, `mark_saved()` / `dirty()` for "save changes?" (`mark_unsaved()`: a recovered copy counts as unsaved). `ui.undo_keys()` gives `Some(true)` for Ctrl+Z and `Some(false)` for
  Ctrl+Y / Ctrl+Shift+Z unless a text field (which has its own undo) has the keyboard.
* **Unsaved changes**: in `App::command(UIC_CLOSE)` return `true` to keep the window, open your
  "save?" dialog, and call `ui.quit()` when done:

  ```rust
  UIC_CLOSE => {
      if !self.modified {
          return false; // not handled: the window closes
      }
      self.dlg_open.set(1); // ask first; true keeps the window open
  }
  ```

* Info bars: `infobar(id, IB_INFO | IB_SUCCESS | IB_WARNING | IB_ERROR, title, msg)` ->
  `EV_ACT` when its close button was clicked. Status bar: `statusbar_begin()` ... texts,
  `separator()` ... `statusbar_end()` at the end of the main box.
* **Secondary windows** (tool windows, dialog windows) are real OS windows, built by the same
  `frame`, each with its own UI state (`legacy/ASMLIB/docs/modules/WINDOWS.md` describes the
  model; the API is the same):

  ```rust
  // state: win_open: Shared<i32>, win_rect: Shared<WinRect> (WinRect::new(0, 0, 260, 0, 0))
  let (wo, wr) = (self.win_open.clone(), self.win_rect.clone());
  if ui.window_begin(ID_WIN, "Werkzeuge", Some(&wo), WF_TOOL | WF_AUTOHEIGHT, Some(&wr)) {
      ui.box_begin(0, BOX_V | PAD_12, 8);
      ui.slider(ID_WIN_SIZE, &mut self.size, &RG_SIZE);
      ui.box_end();
  }
  ui.window_end(); // always
  ```

  Flags `WF_TOOL`, `WF_DIALOG` (+ `WF_MODAL`; `dialog_buttons` then make the footer),
  `WF_RESIZABLE`, `WF_NOCLOSE`, `WF_AUTOHEIGHT`, ...; the library writes the user's placement
  back into the `WinRect`. `window_place`, `window_state`, `window_activate`, `window_destroy`.
* **One instance**: with `AF_SINGLE_INSTANCE` in the `AppDesc`, a later start of the app hands its
  working folder and command line to the running window and ends (`plat::instance`: a pipe /
  an abstract socket per user and app; only for real windows, not for `--shot` / `--script`).
  The window comes forward, the files are the user's ([PERMISSIONS.md](PERMISSIONS.md),
  `fs.user`), and `App::command(UIC_OPEN, count)` arrives:

  ```rust
  UIC_OPEN => {
      for r in ui.open_take() {          // plat::instance::Request { cwd, args }
          for path in r.paths() {        // the arguments that are no options, absolute
              self.open(ui, &path);
          }
      }
  }
  ```

  The first start reads its own `plat::args()` as before. File types and URL schemes that open
  the app come from `PACKAGE` ([PACKAGING.md](PACKAGING.md)).
* **Notification area and notifications**: `ui.tray_set(icon, tooltip, &MENU)` shows a symbol
  (`None`: the program's own icon) whose menu is `Mi` rows of the command table; a chosen row
  arrives as `App::command(cmd, 0)`, a click on the symbol as `UIC_TRAY` (`TRAY_ACTIVATE` /
  `TRAY_SECONDARY`). `ui.notify(title, body, icon)` sends a system notification and returns its
  id; a click on it sends `UIC_NOTE` (arg = id), and `UIC_NOTE_GONE` arrives when it closes unclicked.
  Call `tray_set` again after `cmd_state` changes. `ui.bring_to_front()` restores the window. Windows uses
  `Shell_NotifyIconW` (the notification is the symbol's balloon, which Windows 10 / 11 show as a toast);
  Linux uses StatusNotifierItem + dbusmenu and `org.freedesktop.Notifications`. Headless runs only
  print what they would show. `apps/traydemo` shows all of it.
* **Title bar**: `AF_CUSTOM_TITLEBAR` in the `AppDesc`, then as the first row of the main box
  (caption buttons, dragging, snap layouts on Windows, resize grips on Linux):

  ```rust
  let t = String::from(ui.title()); // title() borrows ui
  ui.titlebar_begin(&t, IC_BRUSH);
  // optional widgets in the title bar (search box, buttons)
  ui.titlebar_end();
  ```

  `ui.set_title(s)` changes the window title.
* **File dialog**: `filedlg(id, FD_OPEN | FD_SAVE | FD_FOLDER | i << FD_EXT_SHIFT, &open, &mut
  path, cap, exts)` -> `EV_ACT` when a file (`FD_FOLDER`: a folder) was chosen; `path` then
  holds an absolute path with `/` separators (Windows accepts them). It shows the system dialog
  (Explorer on Windows, the desktop's file chooser through xdg-desktop-portal on Linux) and
  falls back to the library's own dialog when there is none, in headless runs, or with
  `jfgui_FILEDLG=builtin`; `FD_FOLDER` uses the own dialog on Windows (folders only, the
  selected or shown folder is the answer). On Android it is the system's picker (Storage
  Access Framework): `path` becomes a `content://` name ([VFS.md](VFS.md), "Android"),
  readable and writable with the ordinary file functions and kept across restarts; show the
  user `plat::display_name(&path)`, not the name itself. In a browser with the File System
  Access API (Chromium) `FD_OPEN` / `FD_SAVE` show the system's pickers too: `path` becomes
  `/handles/ID/NAME`, the user's own file (an opened one already read in); `write_file` on it
  writes the file back (`plat::web::persist` confirms it), `plat::web::reopen` reads it again
  after a reload ([VFS.md](VFS.md), "The browser"). Firefox and Safari keep the library's own
  dialog over the browser's virtual files. These pickers answer a few frames later (nothing is
  drawn meanwhile), so keep calling `filedlg` every frame as usual. Keep chosen files in
  `ui.recent_add` (a Flatpak's documents of the portal stay reachable after a restart, Android's
  grants and the browser's handles are kept). Ids `id .. id + 0x3FFF` belong to it. `i`
  preselects a file type of `exts`:

  ```rust
  static EXTS: [&str; 2] = ["png=PNG-Bild", "jpg;jpeg=JPEG-Bild"];
  const ID_FILEDLG: u32 = 0x0010_0000; // .. + 0x3FFF

  // in frame:
  let open = self.fd_open.clone();
  if ui.filedlg(ID_FILEDLG, self.fd_mode, &open, &mut self.path, 1040, &EXTS) & EV_ACT != 0 {
      ui.cmd_run(CMD_FILE_DONE, self.fd_mode); // load / save in App::command
  }
  // CMD_OPEN in App::command: self.fd_mode = FD_OPEN; self.fd_open.set(1);
  ```

* **Drag and drop** ([`ui/dnd.rs`](../lib/ui/dnd.rs), [`plat/dnd.rs`](../lib/plat/dnd.rs)):
  what a drag carries is a `DragData` (`files`, `text`, `urls`, `html`, `image` = a PNG file,
  `bmp`, the app's own `app` kind `DT_USER0 << k` with `bytes`, a `label`, `effects`); its kinds
  are `DT_FILES`, `DT_TEXT`, `DT_URL`, `DT_HTML`, `DT_IMAGE` and the app's own bits, the effects
  `DE_COPY`, `DE_MOVE`, `DE_LINK`.

  ```rust
  const DT_ROW: u32 = DT_USER0; // the app's own kind
  // a source: after the widget, with the widget's EV_*
  let ev = ui.button(ID_ITEM + i, &name);
  match ui.drag_source(ID_ITEM + i, ev) {
      DS_START => {
          let mut d = DragData::of_app(DT_ROW, &[i as u8]);
          d.text = Some(name.clone()); // other programs (and text targets) take the text
          d.effects = DE_MOVE | DE_COPY;
          ui.drag_set(d);              // in the same pass, else nothing is dragged
      }
      DS_END if ui.drag_effect() == DE_MOVE => { /* it was moved: remove it here */ }
      _ => {}
  }
  // a target: right after the widget or box_begin (it takes last_rect)
  ui.box_begin(ID_BASKET, BOX_V | BOX_CARD | PAD_8, 6);
  if ui.drop_target(ID_BASKET, DT_ROW | DT_TEXT) == DS_DROP {
      let d = ui.drop_data();   // and ui.drop_effect(), ui.drop_pos()
      self.pending = Some(d.clone()); // apply after the widgets were drawn, then ui.repass()
  }
  ```

  * Inside the app a widget's drag follows the pointer with a label (the style's tooltip), the
    drop target under it is highlighted (`StyleBasic::drop_highlight`, by default an accent
    frame), Esc cancels, the release drops onto the innermost matching target of the topmost
    layer (a plain drag moves when the source allows it, Ctrl copies, Shift moves). A target
    reports `DS_HOVER` while a matching drag is over it and `DS_DROP` once, in the first pass
    after the drop; the source reports `DS_ACTIVE` and `DS_END` once. A drop changes what this
    pass already drew: apply it after the widgets (end of `frame`) and call `ui.repass()`.
  * When the pointer leaves the window with content other programs know, the platform takes
    the drag over: Windows `DoDragDrop` with our `IDataObject` (CF_UNICODETEXT, CF_HDROP, HTML
    Format, UniformResourceLocator, PNG, CF_DIBV5), X11 as an XDND source, Wayland
    `wl_data_device.start_drag`. Dropped back onto the app it arrives with the app's own kind
    and bytes again. `ui.drag_out(data)` starts such a drag at once (the left button held). Paths
    should be absolute files (a file written for the drag: `plat::temp_dir()`).
  * Other programs' drags: every window is a drop target (Windows `IDropTarget`, X11 XDND 5,
    Wayland `wl_data_device`); the platform asks the targets of the last pass synchronously
    for its answer to the source. While such a drag is over a window the matching targets get
    `DS_HOVER` and `ui.drag_over()` returns its kinds and position. A drop goes to the target
    at the point; outside every target `ui.drop_accept(types)` decides (default `DT_FILES`,
    as before) and it arrives as `UIC_DROP` with `drop_list()` / `drop_data()`. Files are never
    moved (their source would delete them): Shift gives a copy. Dropped files may be read and
    written (`fs.user`).
  * Mirrored windows (RTL): targets and positions stay logical, as every pointer event.
  * Tests: `drag S1 S2` and `down` / `move` / `key esc` / `up` for widget drags; `dragover`,
    `dragleave` and `drop X Y text "s" html "s" url U image F files F...` for other programs;
    `dragout expect ...` / `dragout end move` for a drag that left the window (headless, the
    platform is not involved); see `apps/dndtest`. `plattest --test dnd` checks the shared part
    and Windows' OLE objects without a user, `tools/platshared/dnd_x11.py` XDND both ways under
    WSLg (`--app`: also with the real dndtest window). By hand only: dragging with a real mouse to and from Explorer, a browser, a file
    manager, and everything on Wayland (a compositor gives the drag to the client of a real
    button press).

## 8. Files, settings, images

* **Files** (`plat`): `read_file(path) -> Option<Vec<u8>>`, `write_file(path, data) -> bool`
  (atomic: a temporary file + rename), `file_exists` (0 / 1 file / 2 directory), `mkdir`
  (with parents), `delete`, `rename`, `dir_list(path, &mut |name, is_dir, size| ...)`,
  `plat::File` for random access; `config_dir()`, `temp_dir()`, `cwd()`, `args()`,
  `arg_value("--open")`, `env(name)`, `time_us()`, `date()`.
  `plat::user_dir(plat::DIR_DOCUMENTS)` (also `DIR_DESKTOP`, `DIR_DOWNLOADS`,
  `DIR_PICTURES`, `DIR_MUSIC`, `DIR_VIDEOS`) finds the user's folders where they really are:
  the Known Folders on Windows (also when moved to OneDrive), the XDG user dirs on Linux
  (`~/Dokumente` on a German system). `plat::log_dir()` is the base folder of logs.
  `plat::trash(path)` moves a file or folder into the recycle bin (Windows) or the
  freedesktop trash (Linux) instead of deleting it; on `false` the item is still in its
  place, so offer `delete` only after asking the user.
  `plat::display_name(path)` is what to show the user for a file (`content://` documents:
  their display name). On Android the app's folders are real files of its internal storage
  and survive restarts ([VFS.md](VFS.md), "Android").
  `file_exists_checked` distinguishes a missing path (`Some(0)`) from access errors
  (`None`); Linux also rejects special files. `FileLock::acquire(path)` returns a
  nonblocking exclusive sidecar guard or `None` for busy/error. Keep the guard until
  the read/compare/write transaction completes and leave the sidecar on disk: deleting
  it can let another process lock a different file at the same pathname.
* **Document sources**: `doc_core::SourceLibrary` stores an independent bibliography and
  its ID watermark; `SourceLibraryEditor` adds atomic edits and bounded undo/redo.
  `doc_io::LibraryStore` owns the strict native XML codec, bounded reads, lock/conflict
  checks and atomic backup/write sequence. `prepare_recovery` captures a reviewed
  main/backup snapshot; its token restores only if both files remain unchanged.
  Profile paths, confirmation dialogs and document-to-library selection belong in the app.
* **Document notes**: `doc_core::Note` owns rich paragraphs; an `Inline::NoteReference`
  anchors it exactly once in the main body. Use `Edit::InsertNote`, `SetNoteKind` and
  `RemoveNote` for atomic history. Paragraph edits, search and citation/source operations
  also accept note-body paragraph IDs. `note_number` derives separate decimal sequences
  from anchor order; never store that display number as the note's identity.
  `doc_io` owns the bounded DOCX note parts and their package relationships, while
  `doc_layout` reserves footnotes at page bottoms, continues long notes and appends
  endnotes. Render each `Page.separators` item alongside its lines. Note bodies have
  normal paragraph hit-test IDs, so apps can navigate from the preview to their editor.
* **Virtual files**: every file function also takes `scheme://path`. `app://` holds the files
  of `apps/NAME/assets`, which are built into the program. `config://`, `data://`,
  `cache://` and `logs://` are the app's own folders, and `mem://` lives in memory.
  `desktop://`, `documents://`, `downloads://`, `pictures://`, `music://` and `videos://` are
  the user's folders (with a manifest only what its rules allow). Your own mounts are
  folders, overlays, sub-folders and ZIP archives. [VFS.md](VFS.md) has the details.

  ```rust
  let logo = plat::read_file("app://images/logo.png");
  let ini = base::Ini::load("config://settings.ini");
  plat::write_file("documents://MeinWerkzeug/Bericht.txt", text.as_bytes());
  if !plat::trash("documents://MeinWerkzeug/alt.txt") { /* still there: ask before delete */ }
  ```

* **Permissions**: an `apps/NAME/PERMISSIONS` file limits what the app may touch: files
  below patterns, the user's own files, programs, URL schemes, plugins, printing, the secret
  store, network hosts and ports. The build
  checks it, `ui::main!` installs it, and `plat` refuses everything else: a refused path
  behaves like a missing one, and stderr shows `PERMDENY ...`. Without the file everything
  stays allowed. [PERMISSIONS.md](PERMISSIONS.md) has the syntax and the limits.

  ```text
  fs.write $CONFIG/MeinWerkzeug/**
  fs.write $DOCUMENTS/MeinWerkzeug/**
  fs.user
  url https
  net.connect api.example.com:443
  ask fs.read $PICTURES/** "Bilder einfügen"   # asked at the first access
  agent.confirm document.save                  # an agent needs the user's OK
  ```

  `ask` before a rule asks the user at the first access, in a dialog of the library ("Einmal
  erlauben", "Immer erlauben", "Ablehnen", "Nie erlauben"); one answer covers the whole line.
  The UI thread's access is refused while the question is open and the app gets
  `UIC_PERMISSION` (arg 1: allowed) to try again; a worker thread waits for the answer (60 s,
  then refused). Headless runs never ask (scripts answer with `perm answer ...`). Put
  `ui.permissions_page(ui.id("perms"))` into the settings: it lists the answers, each with a
  reset. Native plugins bring their own `<name>.permissions`; what `plat` does for them needs
  their rights and the app's (`plat::policy::on_behalf`). `agent.confirm` / `agent.deny`
  (or `ActionSpec::confirm()` / `local_only()`) gate agent actions; the agent gets
  `confirmation_required` or `forbidden` back.

  A `sandbox` line (or `sandbox strict`) has the operating system enforce the manifest too, as
  defence in depth against a bug or native code that bypasses `plat`: Landlock plus seccomp on
  Linux, process mitigation policies on Windows, applied at start-up before the app's own code.
  Apps without the line are unchanged. See [PERMISSIONS.md](PERMISSIONS.md) "OS sandbox" for
  what each platform enforces and the limits (a `sandbox` app without a `process` rule cannot
  `spawn_self`, and files the user picks at run time need a Flatpak's documents portal or an
  `fs.*` rule on the folder).

  ```rust
  fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
      if id == UIC_PERMISSION && arg == 1 && self.pending_import {
          self.import(ui); // the user allowed it: the same access again
      }
      true
  }
  ```

* **Passwords, tokens, keys** never go into settings files: `plat::secret` keeps them in the
  system's store (section 20).

* **Settings**: `base::Ini` reads and writes `[section]` / `key=value` files and keeps comments
  and unknown lines:

  ```rust
  let path = format!("{}/MeinWerkzeug/settings.ini", plat::config_dir().unwrap_or_default());
  let mut ini = base::Ini::load(&path);       // a missing file is empty
  let size = ini.get_int("view", "size", 10);
  ini.set_int("view", "size", size + 1);
  ini.save(&path);                            // atomic, creates the folder
  ```

  Save settings in `App::command(UIC_EXIT)` (sent only after a real window ran) and never in
  test runs.
* **Typed settings** (`base::settings`): declare each key once with its type, default,
  limits and label, list them in a `Schema` with a format version, and let the library
  validate, store and show them. The file keeps only what differs from the default;
  comments and unknown lines survive. An older file runs your `migrate(from, &mut ini)` once
  (`base::settings::rename(ini, "old.key", "new.key")` helps); a value that does not fit
  reads as the default (a WARN in the log), numbers are clamped, texts cut.

  ```rust
  use base::{Key, Schema, Settings};
  const DARK: Key<bool> = Key::bool("view.dark", false).label("Dunkles Design");
  const ZOOM: Key<i64> = Key::int("view.zoom", 100, 10, 800).label("Zoom (%)").step(10);
  const UNIT: Key<usize> = Key::choice("view.unit", 0, &[("px", "Pixel"), ("cm", "Zentimeter")]).label("Einheit");
  const NAME: Key<String> = Key::text("user.name", "", 64).label("Name").help("Steht in neuen Dokumenten.");
  const LAST: Key<String> = Key::text("state.last", "", 260).hidden();   // not on the page
  static SCHEMA: Schema = Schema::new(2, &[DARK.d, ZOOM.d, UNIT.d, NAME.d, LAST.d])
      .sections(&[("view", "Ansicht"), ("user", "Benutzer")])
      .migrate(migrate);
  fn migrate(from: u32, ini: &mut base::Ini) {
      if from < 2 {
          base::settings::rename(ini, "view.size", "view.zoom");   // version 1 called it so
      }
  }

  let mut s = Settings::open("config://settings.ini", &SCHEMA);   // in App::init
  let zoom: i64 = s.get(&ZOOM);                                    // typed
  s.set(&ZOOM, 120);                                               // clamped; true = changed
  ui.settings_page(ui::wid("settings"), &mut s);                   // the whole page
  s.save();                                                        // in UIC_EXIT
  ```

  Types: `Key<bool>`, `Key<i64>` (`int`), `Key<f64>` (`num`, with decimals), `Key<String>`
  (`text`, at most N bytes), `Key<usize>` (`choice`: the file keeps the word), `Key<u32>`
  (`color` 0xRRGGBB, the file `#RRGGBB`); `.restart()` marks keys that take effect after a
  restart. `settings_page` shows a card per section with a switch, number box, text box,
  combo box or colour button per key and a reset button; labels go through `i18n::t`.
  `s.revision()` counts changes, `s.ini_mut()` holds free-form data (recent files).
* **Window placement**: `AF_SAVE_STATE` in the app flags keeps the main window's position,
  normal size and maximized state and the tool windows' (palettes, `WF_TOOL`) open flag and
  placement in `config://windows.ini`: the next start opens them where they were (a position
  off every monitor is centred; `--size` wins; Wayland places windows itself).
  `jfgui_STATE=FILE` uses another file, `jfgui_STATE=0` none; test runs never touch it.
  `ui.state_save(&mut ini)` / `ui.state_load(&ini)` do the same with any `Ini` (several
  layouts, tests). Give tool windows stable ids (`ui::wid("palette.layers")`).
* **Images** (`img_core::io`): the runtime registers PNG, JPEG, BMP, GIF and SVG (`.svg`,
  `.svgz`: drawn at their intrinsic size, section 31) at start-up and connects the file
  functions. For every other format call `img_all::use_all_codecs()` once in `App::init`
  (TIFF, TGA, WebP, AVIF, HEIC, JPEG XL, JPEG XR, DDS, PDN).

  ```rust
  match img_core::io::load_ex(path, None, None) {   // any registered format -> FMT_PARGB
      Ok(sf) => {
          self.photo = Some(Rc::new(sf));
          self.photo_ver += 1;
      }
      Err(e) => self.error = Some(format!("Die Datei konnte nicht geöffnet werden ({}).", e)),
  }

  let mut opt = img_core::io::opt_init(img_core::io::format_from_path(path)); // paint.net's defaults
  opt.quality = 90;
  if img_core::io::save_opt(path, sf, Some(&opt)) != 0 { /* ERR_* */ }
  ```

  * Classic calls: `load(path)`, `save(path, &sf)`, `decode(data)`, `save_ex(path, &sf, quality)`
    (PNG / JPEG / BMP / GIF by extension).
  * Extended: `detect(data, hint)`, `probe(data, Some(&mut meta))` (header only),
    `decode_ex(data, Some(&dec), Some(&mut meta))`, `encode(&sf, Some(&opt))`, `save_opt`.
    `ImgDec` chooses straight output (`IMGD_STRAIGHT`), the frame / page, a pixel budget and
    which metadata blobs to keep; `ImgMeta` receives size, format details, frames, ICC / EXIF /
    XMP; `ImgOpt` carries quality, effort, flags (`IOF_*`), bit depth, chroma, DDS / TIFF
    choices and the metadata to embed. Each codec's options are documented in its section of
    API.md (e.g. `img_jxl`: effort, progressive, photon noise).
  * `format_info(fmt)`, `format_from_path(path)`, `register_dyn(...)` (runtime formats of
    plugins). Errors are the `ERR_*` codes of `img_core`.
  * Metadata and colour: `img_core::meta` (conversions `convert` / `surf_convert` between
    `FMT_PARGB` and `FMT_ARGB`, `all_opaque`), `img_core::exif`, `img_core::icc` /
    `icc_xform` (colour management, `legacy/ASMLIB/docs/modules/COLOR_MANAGEMENT.md`).

**Logging** (`plat::log`): `plat::log!(plat::LOG_INFO, "net", "{} bytes from {}", n, host)` formats
only when a sink wants the level; `plat::log_error / log_warn / log_info / log_debug(target, msg)`
are the short forms. Every line goes to a 64 KiB ring (up to DEBUG), to stderr (WARN and
above) and, with `AF_LOG_FILE` in the app flags, to `logs://APP.log` (INFO and above; at
1 MiB it moves to `.1`, three older files are kept). `logs://` is the app's log folder:
`%LOCALAPPDATA%/APP/Logs` on Windows, `$XDG_STATE_HOME/APP` (`~/.local/state/APP`) on Linux. `jfgui_LOG=error|warn|info|debug|trace`
sets the levels, `jfgui_LOG_FILE=PATH|1|0` the file. A crash or panic prints the ring's last
lines and marks the file before the app's crash hook runs; `plat::log_recent(max)` gives the
newest lines for an own report. `ui.warn` lines are in the log too. The runtime logs start,
exit, sleep, session end and settings changes.

## 9. Threads

`plat` has a worker pool (`legacy/ASMLIB/docs/modules/THREADS.md` describes the design; the
Rust pool is the same). Results must not depend on the number of threads: give every item a
fixed piece of work.

```rust
/// brighten every pixel of a straight-ARGB surface on all cores
fn brighten(sf: &mut gfx::Surface, add: u8) {
    let stride = sf.stride as usize;
    let w = sf.w as usize;
    // one item per band of 64 rows; each item locks only its own band
    let bands: Vec<SpinLock<&mut [u8]>> = sf.px.chunks_mut(stride * 64).map(SpinLock::new).collect();
    let item = |i: usize, job: &plat::JobCtl| {
        let mut band = bands[i].lock();
        for row in band.chunks_mut(stride) {
            if job.cancelled() {
                return;
            }
            for px in row[..w * 4].chunks_exact_mut(4) {
                for c in &mut px[..3] {
                    *c = c.saturating_add(add);
                }
            }
        }
    };
    let _ = plat::parallel_for(bands.len(), &item); // returns when every item is done
}
```

(`SpinLock` is `rt::sync::SpinLock`.)

* `parallel_for(count, &f)` runs `f(index, job)` on the workers and the calling thread; nested
  loops are fine; `job.cancel()` skips the remaining items -> `Err(Cancelled)`.
* The renderer uses the pool too: a redraw of about 3 million pixels or more (1.5 times
  1920 x 1080), in a framebuffer of more than 16 MB (4K, 8K) of 750 000 pixels or more, is
  cut into row bands that the workers draw into buffers of their own (`ui/render_par.rs`),
  with the bytes one thread draws. Texts and font icons are placed on the UI thread first
  (`Text::record_n`, the glyph masks go into the cache), the workers blend them read-only
  (`text::GlyphView`). Canvases (`Draw`), icons an icon hook or the style paints and glyphs
  too large for the cache are drawn on the UI thread: the workers draw their bands up to the
  first of them, the UI thread the rest. Without workers (the browser without shared memory,
  Android) everything stays on one thread; `jfgui_RENDER_PAR=0` turns it off (section 12).
* Background work: `plat::Job` -- `set(count, f)` with a `'static + Send + Sync` closure (share
  data through `alloc::sync::Arc`), `start()`, `done()`, `wait()`, `cancel()`,
  `items_done()`. When it finishes the pool calls `plat::wake()`, which arrives as
  `App::command(UIC_WAKE, 0)`:

  ```rust
  struct Loader {
      job: plat::Job,
      result: Arc<SpinLock<Option<Result<gfx::Surface, i32>>>>,
  }

  impl Loader {
      fn start(&mut self, path: String) {
          let slot = self.result.clone();
          self.job.set(1, move |_, _| {
              let r = img_core::io::load_ex(&path, None, None);
              *slot.lock() = Some(r);
          });
          self.job.start(); // UIC_WAKE when it is done
      }
      // in App::command(UIC_WAKE)
      fn poll(&mut self) -> Option<Result<gfx::Surface, i32>> {
          if !self.job.done() {
              return None;
          }
          self.result.lock().take()
      }
  }
  ```

* `thr_count()` workers, `thr_self()` a per-thread slot index; `--threads N` on the command line
  sets the pool size. Test scripts wait for jobs with `sync` once the app installs
  `ui.sync_hook_set(Some(f))` (`f() -> true` while work is pending).

### Async tasks: `ui.spawn`, `plat::task`, `plat::aio`

Work that waits (files, network, timers) is simpler as an `async` block handed to `ui.spawn`.
It runs on the UI thread between passes, its blocking parts go to a pool of their own, and
`App::frame` looks at the handle: no `UIC_WAKE` handler, no shared result slots.

```rust
use plat::aio::{self, Ctl, IoError};
use plat::task::Task;

struct Copier {
    task: Option<Task<Result<u64, IoError>>>,
    ctl: Ctl, // progress (bytes) and cancel token, shared with the copy
    status: String,
}

impl App for Copier {
    fn frame(&mut self, ui: &mut Ui) {
        if let Some(t) = self.task.as_ref().filter(|t| t.is_done()) {
            self.status = match t.take() {
                Some(Ok(n)) => format!("{n} Bytes kopiert"),
                Some(Err(IoError::Cancelled)) | None => String::from("abgebrochen"),
                Some(Err(e)) => format!("Fehler: {e}"),
            };
            self.task = None;
        }
        ui.progress(1, self.ctl.progress.permille() as i32, 1000);
        ui.text(TS_BODY, &self.status);
        if ui.button(2, "Kopieren") & EV_ACT != 0 && self.task.is_none() {
            self.ctl = Ctl::new();
            self.task = Some(ui.spawn(aio::copy_file("documents://a.bin", "documents://b.bin", &self.ctl)));
        }
        if ui.button(3, "Abbrechen") & EV_ACT != 0 {
            self.ctl.cancel.cancel(); // stops between two chunks; b.bin stays as it was
        }
    }
}
```

* `ui.spawn(fut) -> Task<T>`: the future need not be `Send` (an `Rc<Cell<_>>` shared with the
  app works). `task.take()` gives the result once; `is_done()`, `is_cancelled()`; `cancel()`
  drops the future at the next run, with everything it holds; dropping the handle cancels too,
  `detach()` lets it run on. Awaiting a `Task` inside another task gives `Result<T, Cancelled>`;
  `ui.spawner()` spawns from inside tasks.
* A task that ran gets the app another pass, so the frame sees what it changed. A
  `plat::task::Progress` report wakes the event loop (at most every 30 ms, always at the end),
  so progress bars move while a job runs on another thread.
* `plat::task`: `spawn_blocking(f)` for any blocking call (files, sockets, `parallel_for`) on a
  pool of up to 8 threads apart from the compute workers; `spawn_cancellable(|token| ..)` (the
  token says nobody waits any more; a job not started yet is skipped); `sleep(ms)`;
  `CancelToken` (`cancel`, `is_cancelled`, `check()?`, `cancelled().await`, `run(fut)`);
  `join` / `join3` / `join_all` / `select`; `yield_now`. `plat::task::block_on(fut)` runs the
  same code in console programs and tests.
* `plat::aio`: `read_file`, `write_file`, `copy_file` with a `Ctl` (progress in bytes, the
  cancel token, the chunk size); `Reader::open` / `read(max)` and `Writer::create` / `write` /
  `commit` chunk by chunk. Writes are atomic like `plat::write_file` (`plat::AtomicFile`, see
  [VFS.md](VFS.md)): cancelled, failed or dropped, the sibling is removed and the old file
  stays. Names, mounts and permissions are those of the blocking calls.
  `netio::Client::request_async` and `download(url, headers, name, &ctl)` do the same for HTTP.
* Test scripts stay deterministic: `sync` waits until no task work is in flight (pool jobs and
  the wake-ups they send), `wait MS` moves the task clock (`sleep` follows the virtual clock in
  headless runs); wake-ups from other threads reach headless tasks only at `sync`.
  [`apps/asynctest`](../apps/asynctest/main.rs) shows a copy with a progress bar and a cancel
  button and the scripts that step through it.
* A panic in a task or a blocking job ends the process like any panic (exit code 3). In the
  browser without shared memory and on Android there are no threads: blocking jobs run between
  frames on the UI thread, one per run.

## 10. Effects

The `fx` crate holds paint.net's effects and adjustments behind one contract (its crate header
in API.md; `legacy/ASMLIB/docs/modules/EFFECTS_*.md` describe the algorithms):

```rust
fx::use_all(); // once (App::init): register every built-in effect

let Ok(src) = img_core::meta::surf_convert(sf, gfx::FMT_ARGB) else { return }; // straight ARGB
let Some(desc) = fx::find("gaussian_blur") else { return };
let mut params = fx::Params::default();
fx::defaults(desc, &mut params);
params.set_i(0, 8); // "radius": property 0 lives in slot 0
let roi = [0, 0, src.w, src.h];
let args = fx::FxArgs::new(&src, roi, &params);
let aux = match desc.prepare {
    Some(prepare) => match prepare(&args) {
        Ok(a) => a,
        Err(_) => return,
    },
    None => None,
};
let args = fx::FxArgs { aux: aux.as_deref(), ..args };
let mut out = gfx::Surface::new(src.w, src.h, gfx::FMT_ARGB);
let mut dst = fx::Dst::of_surface(&mut out, roi);
if (desc.render)(&args, &mut dst) == 0 {
    img_core::meta::convert(&mut out, gfx::FMT_PARGB);
    // show / store `out`
}
```

* `FxDesc` describes an effect: `name` (ASCII key), `flags` (`FXF_*`), `props` (`FxProp`: type
  `FP_*`, slot, range, default, choices), `prepare` (statistics, LUTs; once) and `render`.
* `render` may be called for many disjoint rectangles from several threads at once
  (`fx::tiles` splits one surface into `Dst`s; `plat::JobCtl` is a `fx::Cancel` for
  `FxArgs::job`); `FXF_BANDS` wants full-width bands, `FXF_WHOLE` one call. The output never
  depends on the split. Pass a selection mask as `FxArgs::sel` and blend the result back through
  it with `gfx::straight::lerp_mask_row`.
* `fx::register(&GROUP)` adds effects of your own; PaintCraft plugins add effects through the
  plugin ABI (section 14).

## 11. Styles, theme, fonts

* The look comes from one style crate linked at build time: `winui` (default), `classic`,
  `macos`, `kde`. `apps/NAME/STYLE` sets an app's style, `build_rs.py ... --style S` overrides
  it. Only the linked style is in the executable. [STYLES.md](STYLES.md) has the model and how
  to write one. Your own colours, corners, spacing, control heights and type without a style
  crate: an app theme; several styles in one app, switched at run time: §34.
* Colours of your own drawing come from the theme tokens: `ui.color(T_TEXT)`, `T_ACCENT`,
  `T_CTRL`, `T_BG`, ... (`ui::consts`), so they work in every style, light and dark.
  `ui.theme_set(0 | 1 | 2)` (light, dark, system), `ui.theme_dark()`, `ui.accent_set(rgb)`;
  command line `--theme light|dark`, `--accent RRGGBB`.
* Fonts: `FONT_REG`, `FONT_SEMI`, `FONT_BOLD`, `FONT_ICON` of the style; icons are `IC_*` code
  points (`ui::keys`), mapped by every style's icon font. `ui.ts_font(TS_*)` selects a text
  style in the text engine `ui.text` (for measuring with `ui.text.width(s)`).
* System settings (`plat::sys_settings()`): in window mode the UI follows the system's light /
  dark choice (with `theme_set(2)`), its accent colour (unless the `AppDesc` or `--accent` set
  one), high contrast, "reduce motion" (`ui.anim_off()`), the text scale, the double-click and
  caret blink times -- at start and live when they change (`UIC_SETTINGS`; Windows registry and
  system calls, Linux the settings portal). Headless runs take only the `jfgui_THEME`,
  `jfgui_ACCENT`, `jfgui_CONTRAST`, `jfgui_REDUCE_MOTION`, `jfgui_TEXT_SCALE`,
  `jfgui_DOUBLE_CLICK_MS`, `jfgui_CARET_BLINK_MS`, `jfgui_FONT_AA`, `jfgui_FONT_HINTING`,
  `jfgui_FONT_RGBA` overrides, so shots stay the same. The system's font smoothing off draws
  black-and-white glyphs, its subpixel order (ClearType's orientation, the portal's or
  fontconfig's `rgba`) subpixel smoothed ones (below); `ui.font_prefs()` gives smoothing,
  hinting and subpixel order.
* Subpixel smoothing (ClearType-like, `ui.text.set_subpixel(text::SUBPIXEL_RGB | _BGR | _VRGB |
  _VBGR | _NONE)`): glyphs are rasterized at three times the resolution across the subpixels,
  FreeType's 5-tap LCD filter keeps colour fringes small, each colour channel is blended with
  its own coverage in linear light (gamma 1.6; the coverage is adjusted by the text colour's
  luminance, so black and white text keep the weight of greyscale text). The UI applies the
  system's order in window mode; headless runs stay greyscale unless `jfgui_FONT_RGBA=rgb`
  (`bgr`, `vrgb`, `vbgr`) forces it, so shots are the same everywhere. It is used only where it
  is safe: on the window's opaque framebuffer (`FMT_XRGB`); layers and images (`FMT_PARGB`,
  `FMT_A8`), colour glyphs (COLR, CBDT), the icon font, glyphs too large for the glyph cache
  and black-and-white text stay greyscale; the text colour's alpha scales the coverage.
  Mirrored (right-to-left) windows keep the physical order: their commands are mirrored, not
  their pixels. Drawing text yourself into an XRGB surface that is scaled or saved later?
  Switch it off around that (`set_subpixel(SUBPIXEL_NONE)`, then back to `subpixel()`).
* Hyphenation: `TX_HYPHENATE` (with `TX_WRAP`) and `TB_HYPHENATE` (with `TB_MULTI | TB_WRAP`)
  break a word that does not fit at its last hyphenation point that fits and draw a hyphen
  there (in a text box only on screen: the text stays as typed). Soft hyphens (U+00AD) work in
  every wrapped text: invisible inside a line, a hyphen where the line breaks after one; a word
  with soft hyphens is not hyphenated otherwise. The patterns are `spell::Hyphenator`s:
  Liang's algorithm on libhyphen's `hyph_xx_YY.dic` (every character set of the first line,
  `LEFTHYPHENMIN` / `RIGHTHYPHENMIN` / `COMPOUNDLEFTHYPHENMIN` / `COMPOUNDRIGHTHYPHENMIN`,
  `NOHYPHEN`, two levels with `NEXTLEVEL`; results as libhyphen's) or on TeX's `\patterns{}` /
  `\hyphenation{}`. The language is the UI language's (`font_language()`), `ui.hyph_language(tag)`
  changes it, `ui.next_hyph_language(tag)` for the next wrapped text or box only. Dictionaries
  are found like Hunspell files (`spell::find_hyphen`): LibreOffice's dictionary extensions,
  `/usr/share/hyphen` and the other data folders, DICPATH, `config://hyphen`, `app://hyphen`
  -- headless runs only `app://`. Your own: `ui.hyph_set(tag, hyphenator)`.

  ```rust
  ui.next_size(240, 0);
  ui.text(TS_BODY | TX_WRAP | TX_HYPHENATE, "Donaudampfschifffahrtsgesellschaft");
  ui.textbox(ID_NOTE, None, &mut self.note, 4096 | TB_MULTI | TB_WRAP | TB_HYPHENATE);
  // patterns of your own (else the dictionary of the language is looked for in the sources)
  if let Some(h) = spell::Hyphenator::from_tex(include_str!("../assets/hyph-la.tex")) {
      ui.hyph_set("la", h); // h.hyphenated(word, "-") shows its breaks
  }
  ```
* High contrast (`ui.high_contrast()`, `ui.set_contrast(Some(on) | None)`, `--contrast high`):
  every token comes from the scheme's colours -- the system's on Windows, else white on black
  or black on white -- opaque fills, borders in the text colour, accents in the highlight
  colour. Draw your own widgets with the tokens (`T_TEXT`, `T_CTRL_STROKE`, `T_ACCENT`, ...),
  not fixed colours, and they follow. `ui.set_text_scale(percent)` / `--text-scale` scale text
  and line heights; controls keep their sizes.
* Reduced motion as an app setting: `ui.set_reduced_motion(Some(on) | None)` forces it on or
  off (like `--anim`) or follows the system again (None). Colours of your own (charts,
  canvases): `ui::contrast_ratio(a, b)` gives the WCAG 2 contrast of two colours (`luminance`
  the relative luminance) -- 4.5 for text, 3 for graphical objects; a selftest can check a
  palette against `style::STYLE.tokens(dark)` (JFNetworker's `uitest.rs`).
* Missing characters (emoji, CJK, Arabic, symbols) are drawn with another font: first the
  app's chain `ui.text.set_fallback_fonts(&[id, ...])`, then system fonts
  (`ui.text.set_system_fallback(on)`, on in window mode; off in headless runs so shots match
  on every machine). The system list (Segoe UI Symbol / Emoji, YaHei, ... on Windows; DejaVu,
  Noto, WenQuanYi, ... on Linux) is found by scanning the font folders once (cached in
  `cache://fontdb.fdb`). Environment: `jfgui_FONT_FALLBACK=0` turns system fonts off (`=1`
  turns them on headless), `jfgui_FALLBACK_FONTS="Family;Family"` puts families first. The
  fonts of the UI language come before the list, so Han characters take the forms of ja,
  zh-Hans or zh-Hant.
* System fonts for your own text: a `FontSel` names a family (also a generic one:
  `sans-serif`, `serif`, `monospace`, `cursive`, `system-ui`, `emoji`, `math`), a weight, italic
  and a size in points. `ui.font_load(&sel)` resolves it like fontconfig -- generic names,
  substitutes (Arial ↔ Liberation Sans, Helvetica → Arial), the language's fonts; on Linux
  from `/etc/fonts`, elsewhere from built-in tables (`text::fontconf::FontConf`) -- and gives
  a font id; `ui.font_resolved(&sel)` names the family it found; `ui.text_in_font(id, pt, s)`
  draws a line in it. `ui.font_picker(id, &mut sel)` is a compact family drop-down with search
  plus a size box; `ui.font_dialog(id, &open, &mut sel)` the full dialog (search, families,
  the family's styles, sizes, preview; OK writes `sel`). `jfgui_FONTDIRS` / `jfgui_FONTCONF`
  replace the system's folders and rules (tests).

  ```rust
  if ui.font_picker(ui::wid("font"), &mut self.font) & EV_ACT != 0 { self.redraw = true; }
  if let Some(f) = ui.font_load(&self.font) {
      ui.text_in_font(f, self.font.size, "Beispieltext");
  }
  ```
* Input methods (composition of CJK and other scripts) reach the text box as preedit and
  commit (`E_PREEDIT` / `E_IME_COMMIT`): Windows IMM, Wayland text-input-v3, X11 IBus (and
  fcitx5's IBus frontend) through the session bus portal `org.freedesktop.portal.IBus`.
  The text box reports its caret, so candidate windows follow it. `jfgui_IME=0` turns IBus
  off; without an input method keys are not delayed.

## 12. Testing -- always verify

* **Command line of every app**: `--shot FILE.png` (headless render, no window system needed),
  `--script FILE.uis` (headless UI test, exit code 0 = every `expect` met), `--dump` (widget tree
  as JSON lines), `--verify-dirty` (compare every incremental redraw with a full redraw),
  `--size WxH` (DIP), `--scale PERCENT`, `--theme light|dark`, `--theme-file PATH` (an app
  theme, §34), `--accent RRGGBB`, `--anim off`,
  `--log-events`, `--auto-close MS`, `--threads N`, `--simd sse2` (no AVX2 paths), `--inspect`
  (the live inspector, §30). Environment: `jfgui_DEBUG=1` (layout / render trace), `jfgui_CSD=1`,
  `jfgui_FILEDLG=builtin`, `jfgui_THEME`, `jfgui_SCALE` (Linux), `jfgui_BACKEND`,
  `jfgui_INSPECT=1` (the inspector's hot keys Ctrl+Shift+I / F12 work), `jfgui_RENDER_PAR=0`
  (every render on one thread), `=1` (row bands for every render, whatever its size), `=2`
  (every render both ways, a `UIWARN` when a byte differs: run the scripts with it after a
  change to the renderer or a `Draw`).
* **Never start an app without `--shot`, `--script` or `--auto-close`**: a plain start opens a
  real window on the user's desktop. PaintCraft saves its settings when a real window closes:
  every real-window run needs `--config build/<name>/cfg` (the capture tools set
  `PaintCraft_CONFIG_DIR`).
* Linux binaries run through WSL:
  `wsl.exe -d Ubuntu-24.04 --cd <repo> -e ./build/linux/NAME --script apps/NAME/tests/x.uis`
  (`jfgui_BACKEND=x11` through `-e env jfgui_BACKEND=x11 ./build/linux/NAME ...`).
* Android runs on an emulator or a device, not in the suite: the debug APK of
  `apps/androidGallery` carries instrumentations for input, accessibility and files
  (`python apps/androidGallery/tests/android_files.py --serial S --apk APK`: internal storage
  across a restart, the system's picker answered through UiAutomation, `content://`, the
  clipboard); see [apps/androidGallery/README.md](../apps/androidGallery/README.md).
* Look at screenshots (`shot`) with an image viewer or the Read tool; test dark and light and
  150 %. Real windows: `python tools/wincap.py build/win/NAME.exe --out cap.png`,
  `python tools/wslcap.py --title T --out cap.png -- build/linux/NAME`.

**The script language** (`ui/auto.rs`; one command per line, `#` starts a comment, coordinates
in DIP):

| Command | Effect |
|---|---|
| `click S [l\|r]`, `rclick S`, `dblclick S`, `move S`, `down S [l\|r\|m]`, `up [S] [l\|r\|m]` | pointer |
| `drag S1 S2 [steps] [shift] [ctrl] [alt] [right\|middle]` | drag with modifiers / other buttons |
| `drag ... pen P0 P1`, `drag ... eraser P0 P1` | a pen stroke, pressure P0 -> P1 (%) |
| `pointer mouse \| pen [P [TX TY]] \| eraser [P ...] \| touch` | pen data of the following pointer events |
| `wheel S DY [shift] [ctrl] [alt]` | raw wheel units (120 = one notch); the modifiers are held during the wheel event (Ctrl + wheel zoom) |
| `key CHORD...` | `key ctrl+s`, `key tab shift+tab enter esc left f4`, also `[ ] , . - + /`, `num0`..`num9`, `numadd` ... (every key a pass of its own, as the platform delivers them) |
| `keys [N] CHORD...` | the chords N times over, queued for ONE pass of the window with the keyboard (more keys than frames, as a tool window hands them on) |
| `type "text"` | typed text (`"\n"` presses Enter) |
| `commit "text" [N]` | an input method commits the text (N times over: one long commit, as the platform queues it) |
| `focus S` | keyboard focus to a widget (focus ring shown) |
| `cmd ID [ARG]` | run a command as if a menu row was chosen |
| `wait MS` | advance the virtual clock (16 ms frames) |
| `size W H`, `theme dark\|light\|contrast`, `scale P [window ID]` | window size, theme, dpi |
| `theme file PATH`, `theme none`, `style NAME` | an app theme from a file (an error is printed and remembered, the theme stays), none, another linked style (§34) |
| `expect theme "NAME"\|none`, `expect theme error "TEXT"\|none`, `expect style NAME` | the app theme, the error of the last `theme file` (contains TEXT), the style in use |
| `expect token NAME #RRGGBB\|#AARRGGBB`, `expect pixel X Y #RRGGBB`, `expect same FILE.png` | a colour token now (`bg`, `accent_text`, ...), the rendered colour at a point of the main window (DIP), the main window rendered now has the pixels of an earlier `shot` |
| `expect S exists\|missing\|hover\|focused\|checked\|unchecked\|disabled\|enabled\|selected\|open\|pressed` | widget states |
| `expect S value N`, `expect S max N`, `expect S text "s"`, `expect S rect X Y W H` | values, texts, rectangles (DIP; `*` takes any value: `rect * * * 36`) |
| `shot FILE.png [window ID\|desktop] [X Y W H]`, `dump [FILE]`, `print TEXT`, `bench N`, `quit` | output |
| `bench N anim [MAX_US]`, `bench N drag S1 S2 [middle\|right\|shift\|ctrl\|alt]... [MAX_US]`, `bench N wheel S DY [shift\|ctrl\|alt]... [MAX_US]` | frame times while things move: N frames 16 ms apart on the virtual clock, the N moves of a drag (every other run drags back), N wheel events (DY and -DY by turns); each frame's passes and the render of its dirty cells are timed; prints mean, render, median and max; with `MAX_US` a run passes when its median is at most `MAX_US`, up to three runs, else FAIL |
| `window ID\|main\|any` | selectors only look into that window |
| `expect window ID open\|closed\|visible\|active\|modal\|keys`, `expect window ID rect X Y W H` | secondary windows (`keys`: the window the keys go to now -- a tool window borrows them after F6 / Ctrl+F6, §27) |
| `close window ID`, `movewin ID X Y`, `resizewin ID W H`, `activate ID` | the user moves / closes windows |
| `sync [MS]` | wait until no background job is pending (`sync_hook_set`) |
| `expect_warn`, `expect_nowarn` | a `UIWARN` was / was not printed since the last check |
| `expect scrollcopy [N]\|none` | at least N (default 1) / no rendered frames moved pixels by a scroll copy since the last check (§5; frames checked by `--verify-dirty` copy whenever they can, `jfgui_DEBUG=1` prints each copy) |
| `expect sound KIND\|none` | the last sound `ui.beep` / `ui.dialog_sound` asked for since the last check (`default`, `information`, `warning`, `error`, `question`, `notify`; headless runs play nothing, §33) |
| `drop X Y FILE...` | files dropped at (X, Y) of the scoped window (onto a drop target, else `UIC_DROP`) |
| `drop X Y [files F...] [text "s"] [html "s"] [url U] [image F] [copy\|move\|link]`, `drop X Y self` | other content dropped there; `self`: the app's own drag that left the window comes back |
| `dragover X Y [files] [text] [url] [html] [image]`, `dragleave` | another program's drag hovers there / leaves |
| `dragout expect none\|text "s"\|html "s"\|url "s"\|files N\|image\|kinds N`, `dragout end copy\|move\|link\|none` | what the app's drag handed to the platform when it left the window, the platform's answer |
| `system focus in\|out`, `system suspend\|resume`, `system session-end`, `system settings theme\|locale\|contrast\|fonts\|monitors\|N` | the platform's lifecycle events (`UIC_ACTIVE`, `UIC_SUSPEND`, ...) |
| `system hotkey ID`, `system pick RRGGBB\|cancel` | a recorded global shortcut pressed (`UIC_HOTKEY`); the user's answer to a running colour pick (`UIC_PICK`, a `CP_PIPETTE` picker; section 21) |
| `clip text "s"`, `clip file F`, `clip dib F`, `clip clear`, `clip save F [bmp]`, `clip expect none\|text\|image\|png` | the headless clipboard (never the real one) |
| `clip utf8 F` | exact UTF-8 text clipboard bytes from a file (up to 4 MiB), including CR/LF and long paste payloads; opt-in Rust extension |
| `clip data FORMAT F [FORMAT F ...]` | publish one to four representations together; formats: `text`, `html`, `rtf`, `docx`; opt-in Rust extension |
| `clip save F text\|html\|rtf\|docx`, `clip expect html\|rtf\|docx\|text-only` | save exact bytes or check rich-format availability; `text-only` also checks absence of rich representations |
| `inspect on\|off\|arm\|pick\|select S\|...`, `inspect expect open\|idle\|same\|selected S\|selector "s"\|...` | the live inspector headless: open it, pick, select, check the overlay, the selector and that a closed inspector changes nothing (§30) |

**Selectors** `S`: `#123` (widget id), `"Label"`, `kind:"Label"` (`button:"OK"`, `dialog:"Öffnen"`,
`text:"Klicks: 2"`), `X,Y` (a point in DIP). A label must match exactly one widget (plain text is
ignored when a control with the same label exists; with matches in several windows the topmost
window wins, then the topmost layer, so an ambiguous label resolves to the dialog or popup).
Failed expectations print `FAIL line N: <command> -> <detail>` and the script goes on.

```
# apps/template/tests/smoke.uis
click "Klick mich"
click "Klick mich"
expect text:"Klicks: 2" exists
click "Zurücksetzen"
expect text:"Klicks: 0" exists
shot build/template.png
```

* A first line `#!args --open file.png` adds program arguments when `build_rs.py test` runs
  the script (pass them yourself when you run it by hand).
* `--dump` / `dump` print the widget tree (kind, id, label, rect in DIP, states, value): use it
  to find ids and positions without looking at pixels. Prefer `#id` and labels over coordinates.
* Popups may leave their window (§6): their dump rectangles can be negative or beyond the
  window; `shot F desktop` shows them. Secondary windows' popups use the main window's client
  area as the screen; `screen X Y W H` (DIP of the main window's client area) gives a virtual
  monitor work area to every window -- then the main window's popups leave it as well and flip
  at the screen's edges (`apps/gallery/tests/popup_edges.uis`); `screen off` goes back.
  Real windows: `tools/wincheck.py popups|edges`, `tools/x11check.py popups|edges --backend
  x11|wayland`, `tools/wlproxy/popflip.py` (a compositor that moves popups). `UIC_EXIT` is never
  sent in `--script` / `--shot` runs.
* Input bursts on Linux: `python tools/keyburst.py [--keys N]` puts a proxy between
  `linuxtest --test burst` and WSLg's X server / compositor that writes N synthetic key presses
  in one go (while the backend waits for a reply, and while it waits for events); every key
  must arrive, in order.

**Test commands**

```
python build_rs.py test --quick     # lint, build every app (both targets), every .uis script
python build_rs.py test             # ... plus parity with the assembly reference
python build_rs.py test NAME        # one app: build it, run its tests/*.uis
python build_rs.py check            # the rules lint alone
python tools/gendoc.py --check      # docs/API.md matches the doc comments
python build_rs.py api-diff         # the public API since the last release (VERSIONING.md)
python build_rs.py i18n [APP]       # the catalogues: missing, untranslated, unused texts ("Languages")
```

* Run new scripts with `--verify-dirty` too (`build_rs.py test` does not add it): it reports
  missed pixels with their bounding box. Checked frames also take every scroll copy they can
  (§5), so a script that scrolls tests those; it renders after each input event (a view copies
  from the second frame that moves it) and `bench N anim` renders the frames of smooth scrolling.
* **Frame times**: the moving forms of `bench` measure with the real clock; `--verify-dirty`
  compares each frame after the timed part, untimed. The median and up to three runs ride out
  a machine that builds other things meanwhile; give a frame budget as the limit (33000 for 30
  fps) rather than today's times. Example: `apps/JFNetworker/tests/perf.uis` (500 devices, 800
  cables, 300 frames moving, pan, zoom and a drag at 1920 x 1080).
* **Presenting** (`plat::win_present(w, frame, rects)`, `bench` leaves it out): the frame's
  dirty rectangles are copied once toward the screen, straight from the framebuffer. Windows:
  into the window's kept copy, a DIB section, then BitBlt (batched, `GdiFlush`); WM_PAINT
  repaints its area from that copy. X11: into one of the window's two MIT-SHM segments, each a
  whole frame that knows where it is stale (the segment of the last put first), then
  ShmPutImage of the rectangles; Expose reads the segment of the last put. Wayland: into a free
  wl_shm buffer that waits for its commit (the frame callback). Only with every segment / buffer
  busy does the damage wait in a staging image. Browser and Android: the B, G, R, x of the
  frame becomes the window's R, G, B, A copy while it is copied in; one window over the whole
  canvas without popups is the canvas itself (no composition), and the host copies rows into
  one buffer per frame (the page's ImageData, Android's Bitmap). Pixels outside the rectangles
  must be what the last present of that window showed (the UI's dirty rectangles cover every
  change; `--verify-dirty` checks it). `plat::present_time()` sums the time spent there;
  `linuxtest --test pbench [--size WxH] [--gap MS]` prints it per frame (the whole frame, two
  boxes, a quarter, 40 rectangles) on Windows, X11 and Wayland; `linuxtest --test pcheck` reads
  the presented pixels back (the window's DC, GetImage, the committed wl_shm buffer) after
  partial presents, frames back to back, hide + show, minimize, WM_PAINT and a new size (Linux:
  `jfgui_BACKEND=x11|wayland`, `jfgui_X11SHM=fd|sysv|off`). `node tools/tests/web_present.mjs
  BUILD [--base OLD]` measures the browser worker and compares canvas and framebuffer in
  Chromium and Firefox; `GalleryPresentInstrumentation` (`apps/androidGallery`) measures
  Android's frames and compares partial frames with a full one.
* **Parity** (apps that also exist in `legacy/`): `python tools/parity.py app NAME` runs the
  shot matrix (light / dark × 100 / 150 %, with dumps) and every script on the assembly
  reference and the Rust build and compares pixels, dumps, stdout and exit codes;
  `apps/NAME/parity.txt` can mark pending scripts or masked areas. The reference binaries live
  in `build/rwref` (`jfgui_REF=<main checkout>/build/rwref` from a worktree);
  `parity.py ... --style S` compares another style (STYLES.md).
* Library check tools: `tools/*_check.py` run against the Rust build with `--out build`
  (codec, kernel and effect checks: `codec_check`, `png_check`, `jxl_check`, `blend_check`,
  `fx_check`, ...); `python tools/diffcheck.py TOOL` additionally compares every result byte
  for byte with the assembly build.

## 13. Pitfalls

* **Ids must be unique** in a frame. Two widgets with the same id share hover / press state:
  clicks seem to get lost, or a click on one acts on the other (a text box below a segmented
  control whose segments take `id + 1 ..`). Scripts (and `jfgui_DEBUG=1`) print a `UIWARN`
  for an id two widgets of one pass share, with both positions; `expect_nowarn` fails on it.
* `ui.canvas(...)` records the version *before* you react to its input. When handling the
  returned `UiInput` changes the content, bump the version and call `ui.repass()`; otherwise
  the dirty-region renderer shows the change one frame late (`--verify-dirty` reports it).
* `ui.canvas`, `ui.title()` and a few others return borrows of `ui`: clone / copy the value
  before the next `ui` call.
* A surface shown with `image` / `draw_image` / a `Draw` object is compared by its version:
  bump it on every pixel change.
* `flyout_begin` / `flyout_end`, `dialog_begin` / `dialog_end`, `window_begin` /
  `window_end`, `box_begin` / `box_end`, `list_begin` / `list_end` always come in pairs, also
  when the begin call returned "closed".
* Do structural work in `App::command`, not in `frame`: `frame` may run several times per
  input event (settle passes).
* Overlays over a canvas: wrap them in `ui.clip_push(rect)` / `ui.clip_pop()` so they never
  spill onto neighbouring panels; boxes placed with `next_rect` are fine next to normal children.
* Panics end the program with exit code 3 and a crash line on stderr; decoders return `ERR_*`
  for bad input instead of panicking.
* `shot` / `dump` in a script do not create folders: `build_rs.py test` creates them for the
  scripts it runs; create them yourself for runs by hand.
* Windows paths: `/` works everywhere; the file dialog returns `/` separators.
* Python helper scripts that patch sources: `Path.write_text` writes CRLF on Windows (use
  `write_bytes`), shell heredocs turn `\\n` into newlines.

## 14. Plugins

PaintCraft loads effects, file types and shapes from plugins (DLL / .so). Plugins written in Rust
use the crate `lib/plugin` (safe `Effect` / `FileType` traits over the PNAPI ABI) and are
built with `python build_rs.py plugin NAME`; see `plugins/README.md` and
`apps/PaintCraft/docs/PLUGINS.md`.

## 15. Data views and app shells

Business apps (BackupCraft's console is the example in the repository) are made of a few
recurring parts. They live in `lib/ui/data.rs`, `shell.rs`, `dock.rs`, `chart.rs`, `notify.rs`
and `datepick.rs`, and every style draws them: the default painters use the theme tokens, a style may
bring its own (`StyleData`, `StyleShell` in [STYLES.md](STYLES.md)).

**Grids** are virtualized tables with sorting, resizable columns and multiple selection by key.
The app keeps a `GridState`, passes the row keys in display order and emits only the rows the
grid asks for:

```rust
let cols = [GridCol::new("Name", 240).flags(GC_FILL), GridCol::new("Größe", 110)];
let keys: Vec<u64> = self.rows.iter().map(|r| r.key).collect();
let (first, end) = ui.grid_begin(ID_GRID, &mut self.grid, &cols, &keys);
for i in first..end {
    let r = &self.rows[i as usize];
    ui.grid_row(&mut self.grid, i);
    ui.grid_text(&self.grid, 0, &r.name);
    ui.grid_text(&self.grid, 1, &r.size);
}
let ev = ui.grid_end(&mut self.grid);   // ev.ev: EV_ACT selection, EV_DOUBLE / EV_RCLICK on ev.row
if ev.sort_changed { /* sort self.rows by self.grid.sort */ }
```

`grid_cell(st, c)` opens a cell for any content (an icon and a text, a badge). The selection
survives reloads because it is kept by key (`self.grid.sel`, `selected_in_order`).

**Trees**: the app flattens its model into `TreeRow`s (key, depth, children, optional tri-state
check box, icon, label, detail) using `TreeState::expanded`; `ui.tree(id, &mut st, &rows,
row_h)` reports opened and closed nodes (load children lazily) and toggled checks.

**Status**: `badge(kind, label)`, `status_dot(kind)`, `meter(id, value, max, kind)` with the
`BADGE_*` kinds (neutral, info, ok, warn, crit, accent) in the token colours (`T_OK`, `T_WARN`,
`T_CRIT`, `T_INFO` and their `_BG`).

**Shell parts**:

| Function | Part |
|---|---|
| `nav_begin(id, &mut NavState)`, `nav_section`, `nav_item(id, icon, label, selected, badge)`, `nav_footer`, `nav_end` | navigation pane, wide or compact (icons only) |
| `command_bar(id, &[CmdItem]) -> Option<u32>` | primary and secondary commands; what does not fit goes into an overflow menu |
| `split_begin(id, pos, vertical)`, `split_next`, `split_end(&mut pos)` | two panes with a draggable splitter (master / detail) |
| `page_header`, `breadcrumb`, `key_value`, `empty_state`, `link` | page titles, paths, label / value rows, empty pages with a call to action, text links |
| `segmented(id, &mut sel, items)` | segmented control (filters, schedule kinds) |
| `wizard_steps(id, steps, current, states)` | the step list of a full-page wizard |
| `toast(kind, title, msg, ms)`, `toast_host()` | notifications; call `toast_host` once per frame, last |
| `spinner(id, size)`, `progress_indeterminate(id)` | waiting |
| `date_picker`, `time_box`, `weekday_toggles` | dates, times, weekday masks (schedules) |

**Docking** ([`ui/dock.rs`](../lib/ui/dock.rs)): panels at the left, right and bottom of a
document area, in tab groups with splitters; the user drags tabs onto drop markers, floats
panels in tool windows and docks them again, hides them and collapses a side to a rail of
icons. The app keeps the layout in a `DockState` and describes its panels every frame; the
area yields the places to fill one by one:

```rust
// state: dock: DockState (DockState::load(&text) at the start, dock.save() to keep it)
let panels = [
    DockPanel::new(P_FILES, "Dateien", IC_FOLDER).at(DOCK_LEFT, 0),
    DockPanel::new(P_OUTLINE, "Gliederung", IC_BULLETED_LIST).at(DOCK_LEFT, 0), // a tab beside it
    DockPanel::new(P_PROPS, "Eigenschaften", IC_SETTINGS).at(DOCK_RIGHT, 0),
    DockPanel::new(P_OUTPUT, "Ausgabe", IC_DOCUMENT).at(DOCK_BOTTOM, 0).flags(DP_NOCLOSE),
    DockPanel::new(P_HISTORY, "Verlauf", IC_HISTORY).at(DOCK_RIGHT, 1).flags(DP_HIDDEN),
];
ui.dock_begin(ID_DOCK, &mut self.dock, &panels); // fills the rest of the box (or next_size)
while let Some(p) = ui.dock_next(&mut self.dock) {
    match p {
        DOCK_CENTER => self.document(ui), // the middle, always first
        P_FILES => self.files(ui),        // each in a vertical box that clips
        _ => {}
    }
}
if ui.dock_end(&mut self.dock) {
    self.settings_dirty = true; // the user changed the layout: store self.dock.save()
}
// a command of the app (menu "Ansicht", Ctrl+3): show the panel and give it the keyboard
self.dock.focus(P_PROPS);
```

* Places: a side holds tab groups (left / right one above the other, bottom side by side);
  splitters between the sides and the document area and between the groups keep their sizes
  in the layout. `DockPanel::at(side, group)` is only the first place: panels with the same
  group number start as tabs of one group, the groups in number order; `DP_HIDDEN` starts
  hidden, `DP_NOCLOSE` has no hide button, `DP_NOFLOAT` never floats. Panel ids must not be 0
  and should never change (the saved layout names them); panels the app no longer describes
  are kept in the layout but not shown.
* Each group has a tab strip with a menu (float / dock to a side / collapse / hide), a
  collapse and a hide button. Dragging a tab, or the empty part of a strip (the whole group),
  shows drop markers at the three edges of the area and on the group under the pointer (as a
  tab, before, after); a release over a strip inserts the tab there, anywhere else -- also
  outside the window -- the panel floats in a tool window (`WF_TOOL`, window ids `ID + 1 ..
  ID + DOCK_FLOAT_MAX`, six at most). A floating window's tabs drag back onto the markers, its
  pin button or a double click docks it where it came from, its close button hides its
  panels. Esc cancels a drag. Painters: `StyleShell::dock_*` (stock ones from the tokens).
* `DockState`: `save()` / `load(text)` (a few lines, `dock 1` first; unknown lines and bad
  values are skipped), `reset`, `show` / `hide` / `toggle` / `focus` / `activate`, `float` /
  `dock_to`, `collapse`, `size` / `set_size`, and for menus and status lines `groups(side)`,
  `floats()`, `hidden_panels()`, `side(id)`, `visible`, `front`, `shown`.
* Keyboard: a group's tab strip is one focus stop (Left / Right / Home / End switch tabs, the
  menu key or Shift+F10 open the group's menu); Ctrl+PgDn / Ctrl+PgUp switch tabs while the
  focus is anywhere in the group. Mirrored windows (RTL) show the left side at the right with
  its markers; menus and markers name the sides as they show.
* Tests: `apps/docktest` (`tests/layout.uis`, `drag.uis`, `float.uis`, `keys.uis`,
  `save.uis`, `load.uis`, `rtl.uis`, `shots.uis`); a floating window is `window 501`, and
  `window any` + `drag tab:"…" X,Y` drags its tab onto a marker of the main window.
  PaintCraft keeps its own placement of floating palettes inside the main window
  (`apps/PaintCraft/src/workspace/panels/dock.rs`), a different model.

**Charts**: `sparkline(id, values, color)`, `bar_chart(id, labels, &[BarSeries], stacked)`,
`donut(id, &[DonutSlice], center, sub)` and `legend(items)`; colours 0 take the style's chart
palette.

**Line charts** of time series (metrics, measurements): the app keeps each series in a
`SeriesBuf` (a ring of `(t, value)` points with a capacity; a full one drops its oldest point)
and describes the chart every frame:

```rust
// state: p50, p95: SeriesBuf (SeriesBuf::new(1200)), faults: Vec<(f64, &'static str)>
self.p50.push(t, 0.012); // t in s, never decreasing; the value in s; f64::NAN is a gap
let series = [
    LcSeries::new("p50", &self.p50).unit(LcUnit::Secs),
    LcSeries::new("p95", &self.p95).pal(3).unit(LcUnit::Secs),
];
let crit = ui.color(T_CRIT);
let marks: Vec<LcMarker> = self.faults.iter().map(|f| LcMarker::line(f.0, f.1).color(crit)).collect();
let slo = [LcThreshold::new(0.25, "SLO 250 ms")];
let chart = LineChart::new(&series).name("Latenz").follow(60.0).y_range(0.0, f64::NAN).markers(&marks).thresholds(&slo);
ui.next_size(-1, 240); // the plot with its axes and the legend below it
if let Some((i, t)) = ui.line_chart(ID_LATENCY, &chart) {
    // the pointer is over series i at its point t
}
```

* The x axis: `LX_SECS` (the default) and `LX_NANOS` (a simulation clock) label time as `0:05`,
  `1:00`, `1:30` (`1:00:00` from an hour on, decimals of the second below one), `LX_NUM` plain
  numbers. `follow(span)` shows the newest `span` (from the first point on while the data are
  shorter), `x_range(a, b)` a fixed view: to pause, keep `chart.view()` and pass it.
* The y axis has nice ticks (1 / 2 / 2.5 / 5 × 10^k) in the first series' unit:
  `LcUnit::Secs` (ns / µs / ms / s), `LcUnit::Si("B")` (kB, MB, GB; `"bit/s"`),
  `LcUnit::Num("req/s")` (1,5k req/s; `"%"`; `""` plain numbers). A NaN end of `y_range` is
  automatic (the visible values and the thresholds, out to the next tick).
  `unit.text(v, ui.decimal_sep())` writes a value as the chart does (KPI tiles beside it).
* Series are lines, `.area()` adds a translucent fill; colour 0 takes the palette colour
  `.pal(k)`, else the series' index. Markers are lines (`LcMarker::line`) or shaded ranges
  (`LcMarker::range`) with a tag at the top, thresholds dashed lines with a tag at their right
  end. Tags never overlap: a marker's tag takes the first of up to three rows with room, else it
  is left out (the hover card lists the markers under the pointer with their time).
* Hovering shows a crosshair at the nearest point, a dot on every series and a card with every
  series' value there. The chart returns the series nearest to the pointer and that x, and
  passes once more when they change, so a readout above the chart is current. The keyboard:
  the plot is a Tab stop; Left / Right step a cursor over the points of its series, Up / Down
  change the series, Home / End, Esc ends it (a click puts it on the point under the pointer);
  crosshair and card follow the cursor, and the chart's record starts with its point
  ("p95, 1:45: 88,1 ms. Latenz – ..."), so a screen reader reads the values one by one.
* Many points: up to two per pixel column the points themselves are drawn, more as one band per
  column (`SeriesBuf::columns`: min / max per pixel column), so 100 000 points draw as fast as
  500. The statistics and the drawing object are kept per id until a series' `ver()` or the
  view changes; a change reads the visible points once (about a millisecond for 100 000).
* No points at all: the `empty` text (default "Keine Daten" in the UI language). High contrast:
  the scheme's colours with a dash pattern per series (also in the legend), no fills. The plot
  keeps its left-to-right order in mirrored windows; the legend is mirrored. Screen readers get
  a `K_CANVAS` record whose label sums the chart up ("Latenz – p50: aktuell 12 ms, min. 9,8 ms,
  max. 41 ms; ..."); axis labels, tags and the card's rows are `K_TEXT` records.
* Testing: the page "Metriken" of `apps/datagallery` (`tests/linechart.uis`,
  `linechart_hc.uis`); `uitest --only chart` checks the ring and the axis maths.

**Languages**: `ui::i18n::Catalog::from_tsv(include_str!("locales/ui.tsv"))` and
`i18n::set_catalog` give an app a German-source catalogue of its own (BackupCraft:
`src/tr.rs`); `i18n::t(s)` reads it. The column follows the user's languages: at start
`ui::main!` reads `--lang` or `jfgui_LANG` (`fr-CA` or a list `fr-CA:en`), else in window mode
the system's languages (Windows UI languages, `LANGUAGE` / `LC_*` / `LANG`, the browser's, the
phone's); headless runs stay German so shots match everywhere (a script picks one with
`#!args --lang ar`). A language the catalogue lacks falls back to English, then to the source.
`i18n::set_language(LANG_EN)` or `set_language_code("fr")` choose one explicitly (settings
page). The library's own texts (file dialog, shortcut names, date picker, tree, tabs, command
bar) follow the catalogue's language: German, English, French, Spanish, Italian, Dutch,
Polish, Arabic and Hebrew (`i18n::library_languages()`), every other language gets English.
An app without a catalogue stays German; one whose texts are language-neutral can install a
catalogue that only names languages (`Catalog::from_tsv("de\ten\tfr\tar\n")`, as
`datagallery` does) to let the library follow the user.

**Checking the catalogues**: `python build_rs.py i18n` ([`tools/i18n_check.py`](../tools/i18n_check.py))
extracts the translatable texts of every app with a catalogue (`apps/NAME/**/locales/*.tsv`)
and of the library, and reports per app:

| Report | Means |
| --- | --- |
| `missing` | a text of the sources without a row (the app shows German in every language) |
| `untranslated` | a row with an empty or missing cell for a language |
| `unused` | a row whose text is no string literal of the app's sources any more (counted; `--unused` or `i18n APP` lists them) |
| `error` | a translation whose placeholders differ (`{}` / `{0}` / `{name}`, the kinds of `{n, number}` ...), a `plural` without `other` or without the categories of its language (`pl` needs `one few many`, `en` never uses `few`), a `tf` text whose translation uses more than `"{}"` (`i18n::fill` fills only those), a message template with formats or names, a text translated differently in two rows |

The texts are the string literals of `t("…")`, `tc("…")`, `tr("…")` (not the library's
`i18n::tr`), `tf("…")`, `msg!` / `Msg::new` / `phrase` templates, `list(&TABLE)`, tables
marked `// i18n` (`// i18n: name title` for some fields of a table, `… // i18n` for one
line), and the labels of `Command::new` and `Mi::*` when the app translates its command
table and menus. A text the library translates (`LIB_TEXTS`) needs no row in an app that
installs its catalogue with `i18n::set_catalog` (`t` falls back to the library). `python build_rs.py i18n APP` lists one app's gaps (`lib`: the library's `tr("…")`
against `LIB_TEXTS`), `--update` adds the missing texts as rows with empty cells to the app's
catalogue (the one whose texts come from the same folder, `src/` or `crates/`), at its end in
source order; existing rows are never changed, and the app keeps showing German there until
the cells are filled. A catalogue the app reads itself without `Catalog::from_tsv` names its
languages in a comment (`# languages: de en fr`, PaintCraft's). The suite runs
`i18n --check`: it fails on errors and on gaps in the catalogues that are complete
(BackupCraft, JFNetworker, the library) and only counts the others (PaintCraft's).

**Sorting and case** follow the user's format locale, as Windows' `CompareStringEx` does:
`i18n::collate(a, b)` orders base letters, then accents, then case ("a" < "A" < "ä" < "b"),
with the rules of the language (Swedish å ä ö after z, Spanish ñ after n, Czech ch after h,
Turkish ı before i, ...). `i18n::collator()` (or `Collator::new("de-DE")`,
`Locale::collate`) takes options: `.numeric(true)` sorts "Datei 2" before "Datei 10",
`.strength(COLL_SECONDARY)` ignores case, `.phonebook(true)` is the German phone book
(ä = ae); `.key(s)` is a sort key and `.sort(&mut rows, |r| &r.name)` sorts once per row. Sort
grid rows with it when `sort_changed` comes (`datagallery`: `coll.compare(x.name, y.name)`);
the file dialog does already. `i18n::to_upper(s)`, `to_lower(s)`, `fold(s)` and
`eq_ignore_case(a, b)` cover all of Unicode (ß -> SS, final sigma) plus the Turkish i; the
`_in(lang, s)` forms take a language. `str::to_lowercase` is right except for Turkish and
Azeri; `to_ascii_lowercase` is right only for ASCII.

**Messages with placeholders** go through `i18n::msg`, a subset of ICU MessageFormat, so a
translation can reorder and inflect:

```rust
let s = i18n::msg(i18n::t("{n, plural, =0 {Keine Dateien} one {# Datei} other {# Dateien}} in {dir}"),
                  &[("n", count.into()), ("dir", name.into())]);
```

`{name}`, `{0}`, `{}`; `{n, number}` (grouped), `{n, number, percent | integer | size |
currency/EUR}`; `{d, date}`, `{d, date, long}`, `{d, time}`, `{d, datetime}` for a
`datetime::Civil`; `{n, plural, ...}` with the CLDR categories of the UI language (`zero one
two few many other`, `=N`, `#`, `offset:N`) and `{g, select, ...}`. `'{'` quotes a brace.

**Regional formats** come from the user's format locale (`i18n::locale()`; Windows' regional
setting, `LC_TIME` / `LANG`, else the first language; `jfgui_LANG` overrides, headless runs use
`de-DE`): `i18n::fmt_int`, `fmt_num(v, decimals)`, `fmt_percent`, `fmt_currency(v, "USD")`,
`fmt_money` (the region's currency), `fmt_size(bytes)`, `fmt_date(&civil, DATE_SHORT |
DATE_LONG)`, `fmt_time`, `fmt_date_time`; `locale().first_weekday()`, `.rtl()`, `.decimal()`.
`Locale::get("en-GB")` formats for a fixed region. Number boxes and the date picker use the
user's format.

**Right to left**: when the UI language is written right to left (Arabic, Hebrew, Persian,
Urdu, ...: `i18n::rtl_language()`), every window is mirrored as a whole, as Windows does with
`WS_EX_LAYOUTRTL`. Write the UI as usual: layout, `last_rect` and hit tests stay logical
(a box's first child is its start, now at the right), and before rendering the recorded
commands are mirrored -- boxes, scroll bars, menus and submenus, check boxes and sliders
flip; text, images and canvases keep their direction; chevrons, back / forward, undo / redo
and indent icons swap. The pointer is mirrored on input and plain Left / Right swap, so keys
follow what is on the screen. `ui.set_rtl(Some(true))` / `--rtl` force it (shots, tests),
`--ltr` / `Some(false)` turn it off. Regions whose content must not flip -- the library's
text fields, canvas, colour pads, pan and angle controls do it already -- go between
`ui.ltr_begin(x, w)` and `ui.ltr_end()`, and read the pointer with `ui.ltr_x(mx)`; an app
that lays out text itself for `textbox_surface` wraps it that way. Rectangles that leave the
UI (scripts' `expect rect`, the agent, popup windows, the platform's hit tests) are physical.

## 16. Several programs per app, app-local crates

An app folder builds one GUI program from `src/main.rs`. `apps/NAME/BINS` adds further
programs, one line each -- name, entry file, `cli` (console program without a style) or `gui`:

```
bc-server   server/main.rs   cli
bc-agent    agent/main.rs    cli
bcctl       ctl/main.rs      cli
```

They are built with the app (`build/<target>/bc-server[.exe]`), started with
`python build_rs.py run BackupCraft --bin bc-server -- --data D`, and packaged next to the GUI
program (`tools/package.py`). Code that several programs share goes into app-local crates,
`apps/NAME/crates/<crate>/lib.rs`: they are built, linted (`#![no_std]`,
`#![forbid(unsafe_code)]`) and linked like the library crates, and their dependencies are the
crates their sources name.

Every program of the app gets the app's name for its folders (`config://` ...), its assets
(`app://`) and its manifest (`apps/NAME/PERMISSIONS`; a program may have its own
`apps/NAME/PERMISSIONS.<bin>`, docs/PERMISSIONS.md "Every program of the app").

Console programs use `rt::main!(run())` instead of `ui::run` (it installs the manifest before
`run`), write with `plat::write_stdout` / `write_stderr` and read arguments with
`plat::args()` / `plat::arg_value`. For background
services, `plat::service::run_service(name, run)` runs under the Windows service control
manager (and as a plain process elsewhere), `install` / `start` / `stop` / `uninstall` / `state`
manage the Windows service or the systemd unit, and `stop_requested()` tells the program to
finish.

## 17. Network programs

The building blocks of BackupCraft's server, agent and command-line client are library crates
that any program can use (all `no_std`, no `unsafe`, see [API.md](API.md)):

* `netio::Server::start(&addr, ServerConfig, handler)` -- an HTTP/1.1 server with a worker pool,
  keep-alive and limits; `start_with(.., Some(wrap))` puts TLS (or anything else) under it and
  passes the identity it proved (a client certificate) to the handler.
* `netio::Client` / `client::Session` -- requests with timeouts and size limits, kept-alive
  sessions; `with_connector(netio::trust::connector(..))` for https to a server with its own CA
  (a CA file, a pinned fingerprint, or trust on first use).
* `netio::tls` -- TLS 1.3 (ChaCha20-Poly1305, X25519, Ed25519 certificates, mutual
  authentication); `netio::x509` builds and checks the certificates of a private CA.
  `Trust::System(extra)` connects to public servers: TLS 1.3 (AES-GCM or ChaCha20, X25519 or
  P-256) or TLS 1.2 with ECDHE, RSA / ECDSA / Ed25519 certificates, the chain built and checked
  against the system's authorities (Windows CryptoAPI, Linux CA bundle) and extra CAs;
  `netio::trust::system_connector(extra)` gives the connector for `Client`.
* `netio::smtp::send(&SmtpConfig, &Mail)` -- e-mail (STARTTLS, TLS, or plain on loopback;
  AUTH PLAIN / LOGIN only over TLS; text and HTML parts, RFC 2047 subjects).
* `netio::ssh::Client` -- run commands on a machine over SSH (curve25519, ssh-ed25519,
  chacha20-poly1305, strict key exchange), host key pinned by fingerprint.
* `netio::dns::resolve` -- names to addresses (Linux: hosts file, resolv.conf, UDP and TCP).
* `datastore::Store` -- JSON collections in a crash-safe journal database with transactions;
  `Journal` for append-only logs.
* `cas::Repo` -- content-addressed, deduplicated, encrypted repositories with signed snapshots.
* `crypto` -- hashes (SHA-2 incl. SHA-384, BLAKE2b, BLAKE3), MACs, KDFs, Argon2id, AEAD
  (ChaCha20-Poly1305, XChaCha, AES-GCM), X25519, P-256 key exchange, Ed25519, RSA (PKCS#1 v1.5,
  PSS) and ECDSA P-256 / P-384 verification (randomness comes from the caller:
  `plat::random_bytes`). For protocols that are defined with them (WPA2 and WPA3): SHA-1
  (`sha1`, `Sha1`, `hmac_sha1`; broken for collisions, new designs take SHA-256), PBKDF2 over
  any hash (`pbkdf2::<H>`, `pbkdf2_sha1`), the AES key wrap of RFC 3394 (`aes_wrap` /
  `aes_unwrap`, which checks the integrity value) and AES-CMAC of RFC 4493 (`cmac_aes`).
* `compress` -- LZ4 and zstd (`compress::zstd::compress(data, level)` / `decompress`; levels
  1 to 19, single-segment frames with checksums); `Codec` picks one by name.
* `plat::watch::Watcher::new(&roots)` / `take(ms)` -- the changed paths below folders
  (ReadDirectoryChangesW / inotify, recursive; `overflow` when events were lost).
* `plat::net::UnixListener::bind(path)` / `connect_unix(path, ms)` -- Unix domain sockets (Linux,
  Windows 10 1803 and later) with the stream calls of `TcpStream`.
* `plat::process::Command { program, args, cwd, env }.spawn()` -- a child process with pipes
  (no shell); dropping the `Child` ends its process tree.
* `netpkt` -- packets byte for byte, without I/O (written for JFNetworker's simulator): `Mac`,
  `Ipv4`, `Ipv6` (RFC 5952 text), `Prefix4` / `Prefix6`; the Internet checksum
  (`checksum::sum` / `finish` / `verify`, pseudo headers, `update16` for incremental changes)
  and `crc32`; views that check lengths once (`EthView`, `Ipv4View`, `TcpView`, ...) and records
  that write headers in front of a payload (`Pkt::push`, `Ipv4Hdr::push_onto`,
  `TcpHdr::push_v4`); 802.1Q tags, ARP, ICMP errors with their quote, IPv4 fragmentation and
  `Reassembly`; `decode::layers(frame)` (fields with byte ranges for an inspector),
  `decode::proto_mask` (filters; `decode::ip_mask` and `decode::stack_ip` for packets without
  a link header, as on serial lines), `decode::info` (one-line summaries); `pcapng::Writer` /
  `pcapng::read` for Wireshark files; `http::Parser` for HTTP/1.1 in pieces of any size
  (`push` adds bytes and gives the next event -- `More`, `Head`, `Data`, `End` --, `poll` the
  ones after it, pipelined messages one after the other, `finish` at the end of the stream)
  and `http::encode_request` / `encode_response` / `encode_chunked` by the wire rules of
  `netio::http`. For address translation, `nat::rewrite(pkt, Side::Src | Side::Dst, addr,
  Some(port))` changes a packet in place with every checksum it touches updated incrementally
  (IPv4 header, TCP and UDP with their pseudo header -- a UDP checksum of 0 stays 0 --, the
  identifier of an ICMP query; a later fragment changes its address only),
  `nat::rewrite_quoted` does the same to the packet an ICMP error quotes together with the
  error's own checksum (RFC 5508), `nat::Flow::of` / `Flow::quoted` give what translation and
  filters look at (addresses, ports or the ICMP identifier, type and code, TCP flags, fragment
  position), and `checksum::update(ck, old, new)` follows any changed run of bytes (RFC 1624).

  ```rust
  let mut p = Pkt::new(netpkt::pkt::HEADROOM, b"payload");
  UdpHdr::new(40000, 53).push_v4(&mut p, src, dst);
  Ipv4Hdr::new(src, dst, ipv4::PROTO_UDP, 64).push_onto(&mut p);
  EthHdr { dst: gw_mac, src: my_mac, ethertype: eth::ET_IPV4 }.push_onto(&mut p);
  let frame = p.into_vec();
  ```

  Application protocols: `dhcp` (RFC 2131 / 2132: `DhcpView`, `Dhcp` with typed options in
  their order, `Opt::RelayInfo` of RFC 3046, long options split and joined per RFC 3396,
  `to_bytes(dhcp::BOOTP_MIN)` pads to the usual 300 bytes), `dns` (RFC 1035: `Message` with
  header, questions and A, AAAA, NS, CNAME, PTR, MX, SOA and TXT records; names with
  compression pointers both ways, loops refused; `to_bytes_max(512)` leaves out whole records
  and sets TC; `reverse_v4`, `is_subdomain`), `ntp` (the 48-byte header, `Ts` timestamps that
  keep whole nanoseconds, `offset_delay`) and `syslog` (`pri`, the BSD and IOS message forms,
  `parse`). `decode::layers` and `decode::info` dissect all four ("DHCP Discover - Transaction
  ID 0x3d1d", "Standard query response 0x1234 A www.lab CNAME web.lab A 10.0.2.20").

  ```rust
  let mut m = Dhcp::new(dhcp::OP_REQUEST, xid, mac);
  m.set(Opt::MsgType(MsgType::Discover));
  m.set(Opt::Params(vec![1, 3, 6, 15]));
  let payload = m.to_bytes(dhcp::BOOTP_MIN);
  let q = dns::Message::query(id, "www.lab", dns::TYPE_A, true).to_bytes();
  let answer = dns::Message::parse(&reply)?;          // answers, authority, additional
  ```

  Spanning tree: `stp` has the BPDUs of STP and RSTP -- Configuration and TCN BPDUs of
  802.1D-1998, RST BPDUs of 802.1D-2004 with their flags and port role (`Role`, `F_PROPOSAL`,
  `F_AGREEMENT`, ...), `BridgeId` (priority with the system ID extension, then the MAC; lower
  is better) and `PortId` (priority in steps of 16, port number), `path_cost(kbps)` after
  802.1D-2004 Table 17-3 (1G 20 000, 10G 2 000, 100G 200). `Bpdu::parse` validates as 9.3.4
  says (protocol identifier, type, length; an MST BPDU reads as an RST BPDU), `Bpdu::frame(src)`
  writes the 802.3 frame with LLC 42 42 03 to 01:80:c2:00:00:00, `stp::of_frame` reads one
  back. `decode::layers` shows every field and flag, `decode::info` sums a BPDU up as capture
  tools do ("RST. Root = 4096/0/02:4a:00:00:01:00  Cost = 0  Port = 0x8001").

  ```rust
  let t = Times { msg_age: 0, max_age: 20, hello: 2, fwd_delay: 15 };     // seconds
  let me = BridgeId::new(4096, mac);
  let b = Bpdu::rst(stp::F_PROPOSAL, me, 0, me, PortId::new(128, 1), t).with_role(Role::Designated);
  let frame = b.frame(port_mac);                      // 60 bytes, padded
  let back = stp::of_frame(&frame);                   // Some(b)
  ```

  Routing: `rip` (RFC 2453, version 1 after RFC 1058) -- `RipView` reads a message (command,
  version, the complete 20-byte entries, `routes()` without the authentication entry, `auth()`
  for that entry of RFC 2453 4.1, `trailing()` bytes), `Rip` writes one (`response`,
  `request_all` for the whole table, `to_bytes`; the sender splits at `rip::MAX_ENTRIES`),
  `Rte::v2(prefix, next_hop, metric, tag)` and `Rte::v1(addr, metric)` build entries,
  `Rte::prefix()` gives a version 2 destination (version 1 masks are the receiver's business),
  `Auth::password` / `text()` the plain-text password (keyed MD5 is recognised, not checked).
  `decode::layers` shows one field per entry with its bytes and the problems a receiver refuses
  (a must-be-zero field of version 1, metrics outside 1 to 16, masks with gaps, bytes after the
  last entry); `decode::info` sums it up ("RIPv2 Response, 3 routes", "RIPv1 Request for the
  whole table").

  ```rust
  let m = rip::Rip::response(2, vec![rip::Rte::v2(net, Ipv4::UNSPECIFIED, 2, 0)]);
  let mut p = Pkt::new(netpkt::pkt::HEADROOM, &m.to_bytes());
  UdpHdr::new(udp::PORT_RIP, udp::PORT_RIP).push_v4(&mut p, src, rip::GROUP);
  let heard = rip::RipView::parse(payload).map(|v| rip::Rip::read(&v));
  ```

  IPv6: `ipv6` reads a packet with its extension headers (`Ipv6View`, `ext_headers`,
  `upper_layer` for the header that carries the payload), writes the fixed header
  (`Ipv6Hdr::packet` / `push_onto`), fragments as a sender does (`fragment(packet, mtu, id)`:
  fragment headers of RFC 8200 4.5, pieces in multiples of 8 bytes; `frag` reads one back) and
  reassembles (`Reassembly6::add`: offsets and lengths checked, overlapping fragments refused
  as RFC 5722 says). `icmpv6` has the messages of RFC 4443 and RFC 4861: `Icmp6` (echo and the
  errors; `error_quote` fills the 1280-byte minimum MTU) with the checksum over the pseudo
  header, `Nd` for Neighbor Discovery (`ns`, `na`, `rs`, router advertisements with their flags
  and lifetimes, redirects; the options source and target link-layer address, prefix
  information, MTU and redirected header) with `Nd::parse` and `check_ip`, the validation of
  RFC 4861 6.1 and 7.1 (hop limit 255, code 0, option lengths, link-local sources of router
  advertisements, DAD only to a solicited-node group) as an `NdErr`. `Ipv6::solicited_node`,
  `Ipv6::link_local_of(mac)` / `eui64_in(prefix, mac)` (the interface identifier of RFC 4291
  appendix A) and `Mac::of_ipv6_multicast` give the addresses Neighbor Discovery needs;
  `decode::layers` shows every field and option, `decode::info` sums a message up ("Neighbor
  Advertisement 2001:db8::5 (sol, ovr) is at 02:4a:00:00:01:01").

  ```rust
  let group = target.solicited_node();
  let msg = Nd::ns(target, Some(my_mac)).encode(src, group);      // with the checksum
  let pkt = Ipv6Hdr::new(src, group, ipv6::NH_ICMPV6, 255).packet(&msg);
  let nd = Nd::parse(&Icmp6View::parse(body)?).ok()?;            // then nd.check_ip(src, dst, hops)
  let parts = ipv6::fragment(&big, 1280, id)?;                    // 1280 bytes, then the rest
  ```

  And `ospf` (OSPF version 2, RFC 2328 appendix A) -- `OspfView` reads a packet (the 24-byte
  header: version, type, length, router ID, area, checksum, authentication type and field;
  `checksum_ok()` is the IP checksum without the authentication field, `lsu_spans()` the LSAs of
  an update), `Packet::read` / `Packet::to_bytes` turn it into a record and back (`Header` and
  `Body::Hello`, `Dd`, `Lsr`, `Lsu` with the LSAs byte for byte, `Ack`; `to_bytes` fills in the
  length and the checksum). `Lsa::parse` / `Lsa::new(..).to_bytes()` read and write an LSA
  (`LsaHeader` and `LsaBody::Router`, `Network`, `Summary` for types 3 and 4, `External`,
  `Other`) with the Fletcher checksum of 12.1.7 (`lsa_checksum`, `lsa_checksum_ok`; `set_age`
  changes the age outside it), `compare` tells which of two instances is more recent (13.1) and
  `LsaKey` names an LSA. The constants of appendix B (`MAX_AGE`, `LS_REFRESH_TIME`,
  `MIN_LS_INTERVAL`, `MIN_LS_ARRIVAL` ...), the option bits, DD flags, LSA and link types come
  with it. `decode::layers` shows every field and every LSA with its bytes and the problems (a
  checksum of the packet or of an LSA that does not fit, another version, lengths that do not
  add up); `decode::info` sums it up ("Hello Packet, DR 10.0.0.2, BDR 10.0.0.1, 3 neighbors",
  "LS Update, 2 LSAs").

  ```rust
  let hdr = ospf::Header { kind: ospf::T_HELLO, router: rid, area: 0, autype: ospf::AU_NULL, auth: [0; 8] };
  let bytes = ospf::Packet { hdr, body: ospf::Body::Hello(hello) }.to_bytes();
  let back = ospf::OspfView::parse(&bytes).filter(|v| v.checksum_ok()).map(|v| ospf::Packet::read(&v));
  let lsa = ospf::Lsa::new(ospf::LSA_ROUTER, rid, rid, ospf::INITIAL_SEQ, ospf::OPT_E, body).to_bytes();
  assert!(ospf::lsa_checksum_ok(&lsa));
  ```

  Address selection: `addrsel` orders the addresses a name resolves to as RFC 6724 6 says,
  given the source address each one would leave with (`Dest::new(addr, Some(src))`, `None`
  for one that cannot be reached; `src_len` is the source's prefix length, `src_deprecated` its
  preferred lifetime ended): unusable destinations last, matching scope, no deprecated source,
  matching label, higher precedence, smaller scope, the longest matching prefix between IPv6
  destinations (capped at the source's prefix, as glibc does), else the given order (`sort` is
  stable). `scope`, `precedence` and `label` give the values of 3.1 / 3.2 and of the default
  policy table of 2.1 (an IPv4 address as ::ffff:a.b.c.d), so a host with a global IPv6 address
  tries IPv6 first and one with only a link-local address IPv4. Rules 4 and 7 (home addresses,
  native transport) need Mobile IPv6 and tunnels and are left out.

  ```rust
  let mut d = vec![addrsel::Dest::new(v4, Some(my_v4)), addrsel::Dest::new(v6, Some(my_v6))];
  addrsel::sort(&mut d);                              // v6 first: precedence 40 against 35
  ```

  WLAN security: `eapol` has the EAPOL frames of IEEE 802.1X-2004 (ethertype `eth::ET_EAPOL`,
  0x888E) with the EAPOL-Key body of the 4-way handshake (IEEE 802.11-2020 12.7.2) --
  `EapolView` reads the header, `Key` reads and writes the RSN key descriptor (Key Information
  bits `KI_*` with the descriptor version, replay counter, nonce, IV, RSC, MIC, key data) and
  tells which message of the handshake it is (`message()`: 1 to 4, None for group key messages
  and requests); `mic_input` gives what the MIC covers (the PDU with its MIC field zeroed) and
  `set_mic` writes one, so the caller computes it with its key (HMAC-SHA1-128 or AES-CMAC from
  `crypto`). `Rsne` writes and reads the RSN element of a personal network (CCMP, AKM PSK, PSK-
  SHA256 or SAE, management frame protection bits), `kde_gtk` / `kde_igtk` the key data
  encapsulations of message 3, `kdes` and `elements` read key data, `pad_key_data` pads it for
  the key wrap. `decode::layers` shows every field ("802.1X Authentication, EAPOL-Key (Message
  3 of 4)", the bits of Key Information by name, the RSN element and the KDEs of plain key
  data), `decode::info` sums it up ("Key (Message 2 of 4)").

  ```rust
  let m1 = eapol::Key { info: eapol::KDV_SHA1 | eapol::KI_PAIRWISE | eapol::KI_ACK, key_len: 16, replay: 1, nonce: anonce, ..Default::default() };
  let frame = EthHdr { dst: sta, src: bssid, ethertype: eth::ET_EAPOL }.frame(&m1.pdu(eapol::VERSION_2004));
  let mut pdu = m2.pdu(1);                       // message 2 of the station: KI_MIC set
  let mic = crypto::hmac_sha1(&kck, &eapol::mic_input(&pdu).unwrap_or_default());
  eapol::set_mic(&mut pdu, mic[..16].try_into().unwrap_or(&[0; 16]));
  ```
* `base::rng::Pcg32::new(seed, stream)` / `Pcg32::derive(seed, id)` -- deterministic random
  numbers (the reference PCG32): the same seed gives the same run on every target. For keys and
  nonces use `plat::random_bytes`.
* `base::metrics` -- measuring without keeping the values: `Counter`, `Gauge` (with its
  maximum), `Hist` (log-linear: exact below 32, then 32 steps per power of two, so a quantile is
  at most 1.6 % off; `quantile(per_mille)`, `p50` / `p95` / `p99`, `merge`) and `Window` (n
  slots of a time span, e.g. `Window::new(300, 1_000_000_000)` for five minutes of seconds in
  nanoseconds: `record(t, v)`, quantiles over what is still in the window, `per_slot` for
  charts). Integer arithmetic only, so every target computes the same numbers.
  `Window::hist_between(from, to)` merges the slots of a time span into one `Hist`: a time
  series takes a point per interval from it and the quantiles of any span (JFNetworker's
  `jfn_sim::series`, which samples every service on virtual time).
* `base::hashring` -- consistent hashing: `Ring::add(member, key, vnodes)` puts a member's
  tokens on the ring (made from its key, so every device that knows the members builds the same
  ring), `owner(hash64(key))` is the member of the first token at or after the hash,
  `preference(h, n)` the next n distinct members (the replicas of a key), `preference_by` skips
  members that are down or draining, `shares()` gives each member's part of the ring. Adding
  or removing a member moves only the keys of its own tokens.
* `base::lru::Lru` -- a map that forgets the least recently used entries: limits on the number
  of entries and on their weight (bytes), `insert` returns what had to go, `get` counts as a
  use and `peek` does not. Ordered maps only, so it is deterministic.
* `agent_core::jmap` -- typed JSON for protocols and files: `record! { pub struct P { pub a:
  String, pub n: u32 } }` declares the struct with `to_json` / `from_json`, `wire_enum!` an
  enum carried as a string; missing or mistyped members read as their defaults, unknown ones
  are ignored (old and new programs read each other's data). `json::stringify_pretty(&v,
  limits, 2)` writes indented JSON for files people read and diff (JFNetworker's `.jfnet`).

## 18. Updates

`lib/update` ([UPDATES.md](UPDATES.md)) brings signed updates with a rollback to any app. The
app builds an `update::Config` (its id, version -- `rt::app_version!()`, the `version=` of its
`PACKAGE` -- the address of `update.json` and the Ed25519
public keys of its publisher, as hex constants in the code), calls `update::startup(&cfg)` first
in `ui::main!` and `update::healthy(&cfg)` once it works:

```rust
ui::main!({
    if let update::Startup::RolledBack(v) = update::startup(&update_config()) {
        // version v did not start and was rolled back: tell the user
    }
    ui::launch(&DESC, App::default(), &style::STYLE)
});
```

* `update::Updater::new(cfg)` -- `start_check()` / `start_download()` run on a thread and
  wake the UI (`UIC_WAKE`); `status()` has the phase (`Available`, `Downloading` with
  `done` / `total`, `Ready`, `Failed` with the error) for a dialog or a status line;
  `cancel()`; `apply()` puts the update in place now and restarts the program.
* A downloaded update is put in place by the next `startup()`, which restarts into it. The new
  version runs on trial until `healthy()`; a start that finds the trial still open restores the
  old files and restarts the old version, which learns of it through `Startup::RolledBack`.
* Everything is checked against the signature (`tools/update_manifest.py` writes and signs the
  channel, `tools/package.py --update DIR` does it per style); the downloads go through the
  app's `net.connect` rules. Putting files in place needs a writable program folder
  (`fs.write $EXE/**` in `PERMISSIONS`); programs in system folders use `package` updates.

## 19. Accessibility (screen readers)

Every semantic record of a frame (`sem()`, section 12's `dump`) becomes a node of the
accessibility tree that screen readers read: AT-SPI2 on Linux (Orca), UI Automation on Windows
(Narrator, NVDA), an ARIA shadow tree in the DOM in the browser (NVDA / JAWS / VoiceOver with the
browser), an `AccessibilityNodeProvider` on Android (TalkBack). Nothing has to be switched on:
the first window frame starts the bridge. Linux connects to the accessibility bus only when
assistive technology is enabled (`org.a11y.Status`; `jfgui_A11Y=1` forces it), Windows answers
`WM_GETOBJECT`, Android follows the system's accessibility switch, the browser cannot tell and
always keeps the shadow tree (`?a11y=0` in the page address leaves it out); everywhere the UI
builds the tree only while a client listens. `jfgui_A11Y=0` (or `NO_AT_BRIDGE=1`) turns it
off. Headless runs have no bridge.

What the built-in widgets report (role on AT-SPI / UI Automation, the name, state and value,
the actions a screen reader can take):

| Widget | Role | Name | State, value | Actions |
|---|---|---|---|---|
| `text` | label / Text | the text | | |
| `heading`, the headings of `markdown_view` / `html_view` (`K_HEADING`) | heading / Text with HeadingLevel | the text | the level | |
| `button`, `icon_button`, `tool_button`, `toggle_button`, `cmd_button` | push button / Button | the label (icon buttons: their tooltip label) | pressed when checked | click / Invoke |
| `checkbox` | check box / CheckBox | the label | checked, mixed (value 2) | toggle |
| `radio` | radio button / RadioButton | the label | checked (selected) | select |
| `switch` | toggle button / Button + Toggle | the label | checked | toggle |
| `slider`, `track_slider`, `slider_f`, `prop_slider` | slider / Slider | the property label, else the text before it | value, min, max | set the value |
| `progress` | progress bar / ProgressBar | the text before it | value of max | |
| `textbox`, `number_box` | entry / Edit | the text before it | the text, the caret and its rectangle, the selection | set the text, the caret / selection |
| `combobox` | combo box / ComboBox | the text before it | the choice, open | open / close |
| `tabs`, segmented strips | page tab list + page tabs / Tab + TabItem | the tab labels | selected | select |
| `list_begin` .. `list_item` | list box + list items / List + ListItem | the row labels | selected | select |
| `menubar`, menus, popups | menu bar, menu, menu item | the labels | open, checked | click / expand |
| `dialog_begin` | dialog / Pane | the title | | |
| `link` | link / Hyperlink | the label | | click |
| colour swatches | push button / Button | `#RRGGBB` | | click |
| titled boxes (notifications, info bars) | panel / Group | the title | | |

**Headings** (`heading(level, ..)`, the headings of the document view; records `K_HEADING`
with the level as value, `Role::Heading` with `Node::level()`) are headings on every platform,
so heading navigation finds them (H / 1 .. 6 in the browse modes of NVDA, JAWS and Orca,
Narrator's headings, TalkBack's heading navigation): UI Automation a Text element with
`HeadingLevel1` .. `9` (plus AriaRole `heading` and AriaProperties `level=N`; a new level raises
the property change), AT-SPI2 the role `heading` with the object attribute `level:N`
(`GetAttributes`; a new level: `object:attributes-changed`), the browser `role="heading"` with
`aria-level`, Android `setHeading(true)` (API 28 and later; API 26 / 27 the heading flag of a
`CollectionItemInfo`, which `isHeading()` and TalkBack read there) -- Android has no heading
level. A text box, drop-down, slider, progress bar or list without a label right after a heading
takes its name from it, as from a text.

Every node has its rectangle; disabled widgets (`ST_DISABLED`, `disabled_push`) report it and
take no actions, and nothing behind a modal dialog can be acted on. The focus is the focused
widget's (a focused list or tab strip passes it on to its selected row / tab). Changes go out as
events: focus, checked / selected / open / enabled, names (a label whose text changes), values,
texts, added and removed nodes. A screen reader's click, toggle, select or open acts like Space
on the focused widget (`EV_ACT` from `behave`); a new slider value or text goes through the
widget like typed input (`EV_ACT`, plus `EV_COMMIT` for texts and the value sliders).

**Widgets of your own** need one thing: after `behave` (or `hit_rect`), a `sem()` record with
the same id, the kind that says what it is, and a label that names it:

```rust
let r = ui.place(ui.dp(120), ui.dp(28));
let ev = ui.hit_rect(ID_GRID, r[0], r[1], r[2], r[3], HB_FOCUS);
if ev & EV_ACT != 0 {
    self.grid = !self.grid; // a click, Space, or a screen reader's "toggle"
}
// ... draw it ...
let rec = ui.sem(K_CHECKBOX, ID_GRID, Some("Raster"), self.grid as i32);
if self.grid {
    rec.state |= SS_CHECKED;
}
```

* Use the id you gave `behave`: then a screen reader's click arrives as `EV_ACT`. A record with
  another id gets a pointer click in its middle instead.
* Name every control: icon-only buttons through their label, drawings through the label of
  their `K_CANVAS` record. Text boxes, sliders, drop-downs, progress bars and lists without a
  label take the text (or heading) record directly before them -- put `text(TS_BODY, "Name")` to
  the left in the same row, or directly above with the same left edge.
* Section titles: `heading(2, TS_SUBTITLE, "Konto")` instead of `text` -- or a `K_HEADING` record
  with the level as value for a title a widget of your own draws.
* States the kind does not imply go into the returned record (`state |= SS_CHECKED`,
  `SS_SELECTED`, `SS_OPEN`); a slider-like widget sets `rec.min` / `rec.max` and applies
  `ui.a11y_value(id)` (a text widget `ui.a11y_text(id)`) when a screen reader sets a value.
* Records without area (hidden probe records) are left out.

**Screen magnifiers** follow the focus and the text caret. Text boxes report their caret
(`ime_pos`, which a text widget of your own calls with its caret's rectangle, like the IME): the
node of a focused text box carries the caret offset, the selection and the caret's rectangle
(`Node::caret_rect`, `char_rect(k)`: exact at the caret, estimated elsewhere). Windows: a
hidden system caret sits in the window with the keyboard (Magnifier, ZoomText:
`GetGUIThreadInfo`, the caret's location events), and the Text / Text2 patterns serve the text,
its ranges, `GetCaretRange` and the selection (NVDA, Narrator, Magnifier); a moved caret raises
`TextSelectionChanged`. Linux: `CaretOffset`, `GetCharacterExtents` (screen coordinates),
`object:text-caret-moved` -- also when a text box gets the focus -- for Orca and GNOME's zoom;
`SetCaretOffset` / `SetSelection` move the caret.

**The browser and Android** keep a mirror of the tree: after every frame the library sends the
differences to the tree the host has (`plat::a11y::wire`: new and changed nodes, the order when
nodes came or went, the events) -- in the browser from the WASM worker to the page, on Android
to the Java host after the frame. The page (`web/a11y.js`) lays one transparent element per node
over the canvas at the node's rectangle (no pointer events, `opacity: 0`, still in the browser's
accessibility tree); Android (`android/GalleryAccessibility.java`) serves virtual views with
their bounds on the screen:

| Node | Browser (ARIA / element) | Android (class) |
|---|---|---|
| label | text | `TextView` |
| button (pressed: toggle) | `button` (`aria-pressed`) | `Button` / `ToggleButton` |
| check box, radio button, switch | `checkbox`, `radio`, `switch` (`aria-checked`, mixed) | `CheckBox`, `RadioButton`, `Switch` |
| slider | `<input type=range>` | `SeekBar` (range info) |
| progress bar | `progressbar` (`aria-valuenow`) | `ProgressBar` (range info) |
| text box | `<input>` / `<textarea>` (password: `type=password`) with the text and the selection | `EditText` (text, hint = name, selection, multi-line, password) |
| list, row | `listbox`, `option` (`aria-selected`) | `ListView`, `TextView` (selected) |
| menu bar, menu, entry | `menubar`, `menu` / `menuitem` (`aria-haspopup`, `aria-expanded`) | `Button` / `ListView`, `TextView` |
| tab strip, tab | `tablist`, `tab` | `TabWidget`, `TextView` (selected) |
| drop-down | `combobox` (the choice, `aria-expanded`; its open list: `aria-controls`) | `Spinner` |
| image, drawing | `img` | `ImageView` |
| dialog, window | `dialog` (`aria-modal`), the main window a `region` | `ViewGroup` with its pane title |

In the browser the DOM focus follows the app's focus as soon as the keyboard (Tab, arrows) or a
screen reader drives it; a pointer on the canvas gives the keyboard back to the hidden text area
that serves typing and the IME. Keys, typed text, the IME and paste reach the app the same way
from either; a screen reader's click, a focus it moves, a range input it sets, a text it
replaces and a selection it moves in a text field become the actions above. On Android TalkBack's
accessibility focus, explore by touch (hover), click, focus, set progress / scroll a slider, set
text and set selection work on the virtual views, and the app sends the events TalkBack speaks
(focus, text changed, selection changed, state and value changes, a dialog appearing).

Testing: `ui.a11y_tree()` returns the tree as the bridges serve it (headless, e.g. through
`ui::Runtime`); `plattest --test a11y` checks the mapping, the actions, the caret and the
browser's / Android's update format, `tools/platshared/atspi_bus.py` the AT-SPI2 bridge in WSL
(a fake bus, and with `--real` at-spi2-core through libatspi; both follow the caret),
`plattest --test uia` the UI Automation providers through IUIAutomation and `plattest --test
caret` the caret alone: system caret and TextPattern2 (Windows, a window for a few seconds).
`node apps/androidGallery/tests/a11y_web.mjs build` checks the shadow tree of the androidGallery
web build in Chromium (Chromium's own accessibility tree through the DevTools protocol, clicks,
values, Tab, typing, selections; Playwright through `tools/tests/playwright.mjs`), and the
debug APK's `GalleryA11yInstrumentation` the Android provider through UiAutomation, the client
side TalkBack uses (`adb shell am instrument -w -r org.jflib.gallery/.GalleryA11yInstrumentation`).
Each of them also checks a heading and its level (the demo's `heading(2, ..)`, the gallery's
section titles).

## 20. Secrets (passwords, tokens, keys)

`plat::secret` ([SECRETS.md](SECRETS.md)) stores small secrets in the operating system's store
instead of a file: the Credential Manager on Windows, the Secret Service (gnome-keyring,
KWallet, KeePassXC) on Linux, the Android Keystore on Android; in the browser the web host
encrypts them with a WebCrypto key that cannot be exported (IndexedDB). The app needs
`secrets` in its `PERMISSIONS`.

```rust
use plat::secret::{secret_available, secret_delete, secret_get, secret_set};

if secret_available() && self.remember {
    if let Err(e) = secret_set("imap.example.org", &self.user, self.password.as_bytes()) {
        self.error = Some(format!("Das Passwort wurde nicht gespeichert ({e})."));
    }
}
let saved: Option<Vec<u8>> = secret_get("imap.example.org", &self.user);
let _ = secret_delete("imap.example.org", &self.user); // when the user signs out
```

* An entry is the app's (`vfs::app_id()`) plus a service and an account (`""` for one value per
  service); values are bytes up to 64 KiB. `secret_set` replaces, `secret_get` gives `None` for
  a missing entry and on errors, `secret_get_checked` tells them apart.
* Without a store (WSL, a minimal Linux, a page without a secure context) every call fails
  with `SecretError::Unavailable` -- nothing is ever written to a plain file. Offer "remember"
  only when `secret_available()` says so.
* In the browser every script of the page's origin can use the key: the store keeps values
  from other origins and from readers of the profile's storage, not from the page itself.
* A locked keyring asks the user to unlock it, and the call waits for the answer (at most five
  minutes): call it from a `plat::Job` when the window must keep drawing. A dismissed dialog is
  `SecretError::Dismissed`.
* Other programs of the same user can read the store too; it protects against other users and
  stolen disks, not against malware running as the user.

Testing: `plattest --test secret` (the real store; its entries are removed again),
`tools/platshared/secret_bus.py` (a fake Secret Service in WSL).

## 21. Desktop integration: keeping awake, autostart, taskbar, file types, recent files, monitors, global shortcuts, the screen

**Keeping the computer awake.** `plat::inhibit(reason, what)` asks the system not to sleep
while the returned guard lives; dropping it lets the system sleep again. `INHIBIT_SUSPEND`
keeps the system running (downloads, backups, long exports; the screen may turn off),
`INHIBIT_IDLE` also keeps the screen on and the session active (presentations, videos). The
reason appears in the system's list (`powercfg /requests`, the desktop's inhibitors).

```rust
// in the app: busy: Option<plat::Inhibit>
self.busy = plat::inhibit("Export läuft", plat::INHIBIT_SUSPEND); // None: not possible here
// ... when the export is done:
self.busy = None;
```

* Windows uses a power request, Linux the Inhibit portal (also in Flatpak), else
  `org.freedesktop.ScreenSaver` / `PowerManagement`. Some Linux desktops only inhibit idle:
  then `INHIBIT_SUSPEND` gives `None` and `INHIBIT_IDLE` still works.
* The other direction already arrives as commands: `UIC_SUSPEND` (sleep / wake up) and
  `UIC_SESSION_END` (logout, shutdown) -- see the table in section 3.
* `plat::inhibited()` returns the bits the live guards hold.

**Starting at login.** `plat::autostart_set(true, &["--tray"])` makes the system start the
running program with these arguments when the user logs in; `autostart_set(false, &[])`
removes the entry, `autostart_get()` says whether it is on (on Windows the Task Manager's
switch counts) and `autostart_entry()` returns the command the entry starts. The app needs
`autostart` in its `PERMISSIONS`; without it `autostart_set` is `AutostartError::Denied`.

* Windows writes `HKCU\...\Run\<app id>`, Linux `~/.config/autostart/<desktop id>.desktop`.
  In Flatpak the Background portal asks the user, and the call waits for the answer (up to
  two minutes): call it from a `plat::Job` there. A refusal is `AutostartError::Dismissed`.
* The entry points at the executable that set it: after the program moved (a portable copy,
  an update into another folder), compare `autostart_entry()` and set it again.

**The taskbar button.**

```rust
ui.taskbar_progress(TaskbarProgress::Normal(done as f32 / total as f32));
ui.taskbar_progress(TaskbarProgress::Paused(0.4)); // Error(..), Indeterminate, None
ui.taskbar_badge(self.unread);                     // 0 removes the badge
ui.taskbar_tasks(&[JumpTask {
    title: String::from("Neues Dokument"),
    args: vec![String::from("--new")],
    desc: String::from("Startet mit einem leeren Dokument"),
}]);
```

* Windows shows the progress on the button (green, yellow when paused, red on error, a moving
  bar when indeterminate), the badge as a red dot with the number over the icon, and the
  tasks in the button's jump list. A task starts the program with its arguments; with
  `AF_SINGLE_INSTANCE` they reach the running app as `UIC_OPEN` (`ui.open_take()`).
* Linux sends the progress and the count to the dock (KDE Plasma, Ubuntu's dock / Dash to
  Dock, Plank) through the Unity launcher API; it needs the app's `.desktop` file to be
  installed under the app's id (`jfgui_DESKTOP_ID` overrides it). Paused / error /
  indeterminate have no own look there, and there is no jump list at run time (`false`).
* State set before the window is shown is applied when its button appears. Call these
  functions from the UI thread (`frame`, `command`). Headless runs only print what they
  would show (`taskbar: ...`).

**File types and URL schemes without a setup.** The setup, the `.deb`, MSIX and Flatpak
register the `filetypes=` / `schemes=` of `apps/NAME/PACKAGE` themselves
([PACKAGING.md](PACKAGING.md)). A developer build or a portable copy registers itself: every
app built with `ui::main!` understands

```
PaintCraft.exe --register            # "Open with", own types and schemes for this program
PaintCraft.exe --register-replace    # also take defaults that belong to other programs
PaintCraft.exe --unregister          # remove exactly that again
```

They print what they did (a Windows GUI program on the console it was started from) and end
the program (exit code 0; 1 failed; 2 not possible here or nothing declared). The app can do
the same from its settings page:

```rust
let pkg = plat::assoc::package();                       // PACKAGE, built in by build_rs.py
let asks = plat::assoc::assoc_conflicts(&pkg)?;         // defaults of other programs
let done = plat::assoc::assoc_register(&pkg, &mut |c| self.user_said_yes(&c.what, &c.holder))?;
// done.kept: the defaults that stayed; plat::assoc::assoc_unregister()? undoes it
```

* Windows writes the setup's keys below `HKCU\Software\Classes` (ProgID `<app>.<ext>`,
  `.<ext>\OpenWithProgids`, for `own` types the extension's default, the scheme's key) and
  tells Explorer (`SHChangeNotify`). Linux writes `~/.local/share/applications/<app_id>.desktop`
  (`Exec=... %U` with schemes, else `%F`; `MimeType=`), the own types as
  `~/.local/share/mime/packages/<app_id>.xml` and the defaults of own types and schemes into
  `~/.config/mimeapps.list` -- directly, without `xdg-mime`. `update-mime-database` (and
  `update-desktop-database` when that folder has a `mimeinfo.cache`) runs when it is
  installed; without it the own types are known after the next update of the MIME database.
* A default that belongs to another program (an extension's default ProgID, a key of the same
  name with another command, a `mimeapps.list` entry, another `.desktop` file with the type) is
  only replaced when `take` says yes (`--register-replace`). The user's own pick in Windows'
  dialog (`UserChoice`) is never touched. On Linux another program's `.desktop` file of the
  same name stops the registration (`AssocError::Declined`) unless it may be moved aside.
* Everything added and replaced is recorded in `config://assoc.list`; `--unregister` removes a
  value only while it still holds what was written and puts back what was replaced (another
  program's keys, files and `mimeapps.list` lines byte for byte). Registering again first
  undoes the old registration (a build that moved).
* Not possible in an MSIX package or a Flatpak (`AssocError::Unavailable`): the package
  declares the types. Registering is no permission of `PERMISSIONS`: only what `PACKAGE`
  declares is written, in the user's own part of the system.
* Linux file managers hand files over as `file://` URIs for `%U`: `Request::paths()` (`ui.open_take()`),
  the `fs.user` grant of the command line and `plat::assoc::file_uri_path(arg)` turn local
  ones into paths. An app that reads `plat::args()` itself uses `file_uri_path` too.

**Recent files.** `ui.recent_add(path)` after opening or saving a file keeps the app's own
list (`ui.recent_files()`, newest first, at most `ui::RECENT_KEEP`, in `config://recent.list`;
`recent_remove`, `recent_clear`) and tells the system:

* Windows: `SHAddToRecentDocs` -- Explorer's recent files (Home / Quick access, the Recent
  folder, Start's recommendations) and, when Windows counts the app as a handler of the type
  (setup, `--register`, MSIX), the "Recent" category of its jump list (`ui.taskbar_tasks`
  keeps that category). `plat::recent_list(max)` reads the jump list back
  (`IApplicationDocumentLists`); which app's jump list gets a file is Windows' decision, so the
  app's own list is the one to show in a menu.
* Linux: a bookmark in `~/.local/share/recently-used.xbel` in the format GTK and KDE use
  (MIME type, the app's entry with its command and a count), changed under a lock and written
  atomically; other programs' bookmarks stay as they are. In a Flatpak the host's list needs
  `flatpak_args=--filesystem=xdg-data/recently-used.xbel` in `PACKAGE`.
* `plat::recent_add` only takes files the app may read under its `PERMISSIONS` (else
  `PERMDENY fs.read`). Headless runs (`--script`, `--shot`, agents) leave the system's list and
  `config://recent.list` alone; `jfgui_RECENT=/path/x.xbel` redirects the system's list to a
  file (every OS), `jfgui_RECENT=off` switches it off.

Testing: `plattest --test power` (real power requests on Windows), `--test autostart` (the
real Run value, removed again; XDG autostart in a scratch folder), `--test taskbar` (the dry
run), `--test taskbar-win` (by hand: a window with every state), and
`tools/platshared/desktop_bus.py` in WSL (the Inhibit and Background portals and the
LauncherEntry signals on a fake bus). `apps/traydemo` has buttons for all of it.
`plattest --test assoc` registers a test package (Windows: test names in the real HKCU,
removed again; Linux: scratch XDG folders, `desktop-file-validate`), `--test recent` the xbel
list through `jfgui_RECENT`, `--test recent-win` (by hand) the real shell.

**Monitors.** `ui.monitors()` (or `plat::monitors()`) lists every monitor, the primary one
first: `name`, `rect` and `work` (x, y, w, h without taskbars and panels), `dpi` (96 = 100 %),
`refresh_mhz` (60000 = 60 Hz, 0 unknown), `primary` and an `id` that stays while the monitor is
connected. `plat::screen::monitor_at(x, y)` finds the monitor of a point. A change (a monitor
plugged in, another resolution or scale, a moved taskbar) arrives as `UIC_SETTINGS` with
`plat::SET_MONITORS` in `arg`: read the list again.

```rust
for m in ui.monitors() {
    let hz = m.refresh_mhz as f32 / 1000.0;
    plat::log_info("app", &format!("{} {}x{} at {},{}, {} dpi, {hz} Hz", m.name, m.rect[2], m.rect[3], m.rect[0], m.rect[1], m.dpi));
}
```

* The units are the desktop's: physical pixels on Windows (the process is per-monitor DPI
  aware), X11 and Android; the compositor's logical units on Wayland (xdg-output; `dpi` from
  the physical and logical width, so fractional scales show); CSS pixels in the browser.
* Windows: `EnumDisplayMonitors`, `GetMonitorInfoW` (device name `\\.\DISPLAY1`, work area),
  `GetDpiForMonitor`, `EnumDisplaySettingsW` (refresh); `WM_DISPLAYCHANGE` and a work area
  change send `SET_MONITORS`. X11: RandR 1.5 (`GetMonitors`, the refresh rate of the CRTC's
  mode, names such as `HDMI-1`), the work area of `_NET_WORKAREA`, RandR's change events; one
  dpi for all monitors (the X server has one). Wayland: `wl_output` with `xdg-output`, no work
  area (= the monitor) and no primary (the first output). Browser: `window.screen`, or every
  screen where the page already may ask (`getScreenDetails`, the "window-management"
  permission granted; the page never asks itself), no refresh rate. Android: the
  `DisplayManager`'s displays. Linux knows the monitors only once the app has its first window.
* Headless runs list one monitor: the window, or the virtual screen of the script command
  `screen X Y W H`; `system settings monitors` sends the change.

**Global shortcuts.** A chord registered with `ui.hotkey_register(id, "Ctrl+Alt+K",
description)` reaches the app while another program has the keyboard: `App::command(UIC_HOTKEY,
id)`. `ui.hotkey_unregister(id)` removes it (the program's end does too). The app needs
`input.global` in its `PERMISSIONS` (else `HotkeyError::Denied` and `PERMDENY input.global`).

```rust
match ui.hotkey_register(HK_SHOW, "Ctrl+Alt+K", "Fenster zeigen") {
    Ok(()) => {}
    Err(HotkeyError::Conflict) => self.note = String::from("Strg+Alt+K gehört schon einem anderen Programm."),
    Err(e) => self.note = format!("Kein globales Kürzel: {e}"),
}
// in command(): UIC_HOTKEY if arg == HK_SHOW => ui.bring_to_front(),
```

* Chords: modifiers `Ctrl`, `Alt`, `Shift`, `Super` (`Win`, `Meta`, `Logo`), one key: a letter,
  a digit, `F1`..`F24`, `Space`, `Enter`, `Tab`, `Esc`, `Backspace`, `Delete`, `Insert`, `Home`,
  `End`, `PageUp`, `PageDown`, the arrows, `Print`, `Pause`, `Num0`..`Num9`, `NumAdd` ...
  (`plat::Chord::parse`; case does not matter). A held chord reports once. Ctrl+Alt+F1..F12
  switch the console on Linux: never use them.
* Windows: `RegisterHotKey` on a message-only window of the UI thread; a chord another
  program holds is `HotkeyError::Conflict`. X11: `XGrabKey` on the root window for every key
  that gives the key and every combination of Caps Lock and Num Lock; another client's grab is
  `Conflict`. Wayland and Flatpak: the GlobalShortcuts portal -- the desktop shows the
  shortcuts with their descriptions and the user binds them (maybe to another chord, maybe
  not). There `hotkey_register` answers `Ok` at once and a shortcut the desktop did not bind
  comes later as `UIC_HOTKEY_LOST` (`ui.hotkey_error(id)`: `Dismissed`). Browser and Android:
  `Unavailable` (`ui.hotkeys_available()` is false).
* Headless runs only record the registration; the script command `system hotkey ID` presses
  it.

**A colour from anywhere on the screen.** `ui.pick_color()` lets the user click any pixel of
the screen (Esc or a right click cancels); then `App::command(UIC_PICK, 1)` and
`ui.picked_color()` is `Some(0xRRGGBB)` (`UIC_PICK` with 0: cancelled or failed,
`ui.screen_error()` says why). Colour pickers with `CP_PIPETTE` (section 5) do this for the
user. `ui.pick_available()` says whether it works here. Picking needs no permission: the user
chooses the one pixel the app gets.

* Windows: a frozen picture of every monitor in a topmost window with a magnifier and the hex
  value beside the pointer; the arrows move the pointer by a pixel (Shift: ten), Enter / Space
  or a click takes it. X11: a pointer grab with a crosshair, the pixel read from the root
  window (a rootless server such as WSLg's Xwayland: from the window under the pointer).
  Wayland and Flatpak: the screenshot portal's `PickColor` (the desktop's own picker). Browser:
  the `EyeDropper` (Chromium; call it from a click), elsewhere and on Android `Unavailable`.
* Headless runs wait for the script's answer: `system pick RRGGBB` or `system pick cancel`.

**Pictures of the screen.** `ui.screenshot(None)` takes the whole desktop (`Some([x, y, w, h])`
a rectangle); then `App::command(UIC_SCREENSHOT, 1)` and `ui.screenshot_take()` is the picture
as a `gfx::Surface` (save it with `img_png::enc`, show it with `ui.image`). The app needs
`screen.capture` in its `PERMISSIONS` (else `ScreenError::Denied`, `PERMDENY screen.capture`).
Windows: `BitBlt` from the screen DC. X11: `GetImage` of the root window (a rootless server
refuses it: `Failed`). Wayland and Flatpak: the screenshot portal (the desktop may ask; the file
it writes stays where it put it, the rectangle is ignored). Browser and Android: `Unavailable`;
headless runs too. `plat::screen::capture_start` is the same without the UI (the picture as
B, G, R, x rows or, from the portal, the PNG file's bytes).

Testing: `plattest --test screen` (Windows: the monitors against `GetSystemMetrics`, a pattern
window captured and picked, `RegisterHotKey` pressed through `SendInput`, a chord another
process holds; Linux: the same on WSLg's X server with RandR, GetImage, XTEST and `XGrabKey`),
`--test screen-wl` (the Wayland outputs), `--test screen-policy`, and
`tools/platshared/screen_bus.py` in WSL (the Screenshot and GlobalShortcuts portals on a fake
bus). `apps/screentest` shows all of it; `apps/screentest/tests/screen_web.mjs` checks the
browser (the monitors of `window.screen`, the EyeDropper's pipette in Chromium only,
`Unavailable` for shortcuts and pictures), `apps/screentest/tests/android_screen.py` an Android
emulator (the display against `wm size`, `wm density` and `dumpsys display`, a change through
`wm size`).

## 22. Diagrams: nodes, ports and edges

`ui.diagram` is a canvas for node / edge editors (network topologies, flow charts). The app
owns the model and paints it; the widget keeps the view, the selection and the gestures in a
`DiagramState` and returns what the user did as `DEv` events (`lib/ui/diagram.rs`, the test
app `apps/diagramtest`).

```rust
// in frame (self.st: DiagramState, self.scene: Rc<RefCell<Scene>>, self.painter: Rc<dyn DgPaint>)
ui.next_size(-1, -1);
let sc = self.scene.clone();
let sc = sc.borrow();
let m = DModel { nodes: &sc.nodes, ports: &sc.ports, edges: &sc.edges, wps: &sc.wps, ver: sc.ver };
let evs = ui.diagram(ID_DIAGRAM, &mut self.st, &m, DG_SELECT, Some(self.painter.clone()), 0);
ui.diagram_minimap(ID_MINI, &mut self.st, &m);
ui.diagram_sems(ID_DIAGRAM, &mut self.st, &m, &|k| sc.label(k));
drop(sc);
for ev in evs {
    match ev {
        DEv::Move { keys, d, .. } => self.move_nodes(&keys, d), // and bump the model's ver
        DEv::Connect { from, to, .. } => self.connect(from, to),
        DEv::Delete => self.delete_selection(),
        _ => {}
    }
}
```

* The model is borrowed per call: `DNode { key, rect, ports, flags }` (world x, y, w, h and
  a range of `ports`), `DPort { key, at, flags }` (relative to its node), `DEdge { key, a, b,
  route, wps }` (a / b are port or node keys; `DR_STRAIGHT`, `DR_BEZIER`, `DR_ORTHO`; a range
  of `wps`). Keys are non-zero and unique over all three; bump `ver` with every geometry
  change (the routes and the spatial index are rebuilt then). A world unit is a DIP at 100 %.
* Tools: `DG_SELECT` (click, Shift adds, Ctrl toggles, marquee; a drag moves the selection
  snapped to `st.grid`, Alt moves freely; a drag out of a port connects; the handles of a
  selected edge move and insert waypoints, a double click removes one), `DG_PAN`,
  `DG_CONNECT` (drag, or two clicks, from node / port to node / port), `DG_PLACE`
  (`DEv::Click` at a snapped point). Middle drag, Space + drag and the wheel pan, Ctrl + wheel
  zooms at the pointer; arrows nudge, Del, Ctrl+A, Esc, Tab, Enter, Ctrl + plus / minus / 0
  work while the canvas has the focus.
* Painting: the `DgPaint` runs at render time with a `DgFrame` (`vp`: world <-> px, `clip`: the
  world part to paint, `geo.routes`: the polyline of every edge, the theme's colours and fonts,
  `moving`). Cull with `f.visible(rect)` / `f.route_visible(route,
  width)`, stroke with `dg_stroke_edge` (gfx::dash caps for arrows, dash patterns), place
  moving things with `dg_point_at(&route.pts, f)`. Selection, hover, ports, handles, marquee
  and rubber band are drawn by the widget as ordinary commands on top. Pass a `ver` that
  changes with anything else the paint shows (states, labels).
* Screen readers and keyboard: `ui.diagram_edge_sems(id, &mut st, &m, &label)` adds one
  `listitem` per visible edge (a small box around the middle of its route; a screen reader's
  click selects it), `st.tab_edges = true` lets Tab walk the edges after the nodes (a focused
  edge gets the focus ring along its route, Enter activates it, the menu key opens its context
  menu), `st.name` names the canvas.
* Scripts: every visible node is a `listitem` with the label the app gives (`click
  listitem:"R1"`, `expect listitem:"R1" selected`). `down` / `up` take a point
  (`down 470,164`): a `listitem:` selector there reads as the button letter. A second press
  on the same thing within the double-click time activates it, so `wait 600` between a click
  and a drag of the same node. Ctrl + wheel: `wheel S DY ctrl`. Scripts cannot hold Space:
  the app sets `DiagramState::test_keys` (1 Space, 2 Ctrl) from a command (`cmd 900 1` in
  diagramtest).
* **Large diagrams**: the widget paints the whole canvas into a backing store (an XRGB surface
  of the `DiagramState`, from the window's corner to the canvas' far corner) whenever the
  paint's version changes (the model, `ver`, the view, the theme, `DgFrame::moving`); dirty
  rectangles then only copy their part, so hover, selection and what is drawn on top (moving
  packets) never repaint the scene. In the paint, iterate `f.find(margin_px, edges)` (the
  spatial buckets near the clip, ascending indices) instead of every node and edge, and cull
  with `visible` / `route_visible` before computing anything per item. While a gesture moves
  the view or nodes (a pan, wheel turns until 160 ms after the last one, a drag of nodes or a
  waypoint) `f.moving` is set: with many things in view, leave out small labels and LEDs and
  stroke edges with `dg_hairline` (1 px anti-aliased lines, a fraction of `dg_stroke_edge`'s
  cost); the canvas is painted in full detail once the gesture ends. JFNetworker's
  `tests/perf.uis` holds 500 devices and 800 cables this way.

## 23. Consoles: `ui.terminal`

`lib/ui/term.rs` is a console for command lines (an IOS-like CLI, a Linux-like shell, a log):
a scrollback ring with an ANSI subset, the input line on `textbox_surface` (IME, clipboard,
selection), history, completion, a `--More--` pager, busy mode with type-ahead and raw keys.
The app keeps a `TermState` per console and answers its events; `apps/termtest` is the
example (an IOS-like router console and a fake shell).

```rust
static MONO: &[u8] = include_bytes!("../../lib/doc_view/assets/DejaVuSansMono.ttf");

fn init(&mut self, ui: &mut Ui) -> bool {
    let f = ui.text.load_mem(MONO, 0);   // embed a monospace font (licence next to it)
    if f >= 0 { ui.mono_font_set(f as u32); }
    self.con.set_prompt("Router#");
    true
}

// in frame: the console fills its place; ids ID_CON .. ID_CON + 7
ui.next_size(-1, -1);
for ev in ui.terminal(ID_CON, &mut self.con, TF_QHELP | TF_CTRL_SHIFT6) {
    match ev {
        TermEv::Submit(line) => self.run(ui, &line),        // echoed and in the history
        TermEv::Help(line) => self.con.write_str(&help(&line)),
        TermEv::Complete { line, caret } => self.con.complete_reply(&complete(&line, caret), &[] as &[&str]),
        TermEv::Interrupt => self.stop_job(),
        _ => {}
    }
}
```

* Output: `write(&[u8])` / `write_str` take UTF-8 with SGR colours and attributes, `\r`, `\t`,
  backspace, `ESC[K`, `ESC[2J` (the screen scrolls clear), `ESC[nC/D/G`, BEL (a flash). The
  16 colours come from the theme tokens, so they suit light, dark and high contrast.
* Modes (`set_mode`): `TM_LINE` the input line; `TM_BUSY` while a command runs (no input line;
  what the user types waits and comes back as one `Submit` per line afterwards; Ctrl+C still
  interrupts); `TM_RAW` every key as `TermEv::Raw`; `TM_SECRET` a password without echo.
* Long output: `pager(true)`, write it all, `pager(false)` -- the console shows a screen at a
  time with `pager_text` (`" --More-- "`): Space, Enter, q / Esc (`TermEv::PagerQuit`).
  `page_len` fixes the lines of a page instead (IOS' `terminal length`: JFNetworker uses 23).
* While busy or paging (no input line) a focused console still holds the keyboard like a text
  box: plain keys are no accelerators, and Ctrl+Z / Ctrl+Y stay with it, so the app's undo
  (`ui.undo_keys`, an accelerator) does not run.
* Flags: `TF_QHELP` (a typed `?` gives `TermEv::Help` instead of a character), `TF_CTRL_SHIFT6`
  (Ctrl+Shift+6 interrupts), `TF_NO_INPUT` (output only), `TF_NO_FRAME`.
* The user selects with the mouse (copy: Ctrl+C with a selection, Ctrl+Shift+C), searches with
  Ctrl+F, scrolls with the wheel / PgUp / PgDn; new output while scrolled up shows a
  "Neue Ausgabe" button. Pasting several lines submits each.
* Tests: every visible line is a `K_TEXT` record (`expect text:"Router#" exists`), the input
  line the text box `ID_CON + 1` (termtest: `expect #1001 text "show "`). Pasted lines and type-ahead
  follow one per 16 ms: `wait 50` before checking the later ones.

## 24. Multi-colour vector symbols

`ui::Symbol` ([`lib/ui/symbol.rs`](../lib/ui/symbol.rs)) draws pictures with several colours
-- JFNetworker's device symbols, diagram nodes, palette tiles -- from SVG path layers. The
colours are palette *roles*, so light, dark and high contrast only swap the `SymPalette`.

```rust
// once: parse the art (tools/vsym.py writes it from SVG files)
let sym = Symbol::parse(art, [64.0, 48.0]);
// per frame: the theme's palette for the symbol's category colour (0xRRGGBB, light value)
let pal = SymPalette::from_theme(ui, 0x4F7396);
// at render time (a canvas Draw): the raster from the cache, blitted ...
self.cache.borrow_mut().draw(g, id, &sym, x, y, 64, &pal, 255);  // 64 px wide, opaque
// ... or the vectors at any scale and fractional position (a zoomed diagram)
sym.paint(g, x as f32, y as f32, 2.0, &pal);                     // 2 px per grid unit
```

* Roles: 0 ink (outlines), 1 body, 2 body shade, 3 screen, 4 category tint, 5 ports / dark
  parts, 6 ok, 7 warn, 8 crit (status LEDs), 9 highlight (`SYM_INK` .. `SYM_HI`). An app
  shows a device's state by changing a role, e.g. `pal.c[SYM_OK as usize] = ...`;
  `pal.powered_off()` turns LEDs and screens off (draw it at opacity 115 = 45 %).
* `SymbolCache` keeps up to 256 rasters per (id, width, palette); wider than 256 px it draws
  the vectors. Keep it in a `RefCell` next to the parsed symbols (a `Draw` gets `&self`).
* The layer syntax extends that of `ui::paint_svg_icon` (one-colour icons, `tools/vicon.py`):
  `F` / `E` fill, `#n` role, `S<w>` stroke width in grid units (default 1.5), so icon art is
  valid symbol art (role 0, an `A` layer role 4).

**Drawing symbols in SVG.** `python tools/vsym.py apps/NAME` reads `apps/NAME/icons/*.svg`
(viewBox `0 0 64 48`, 2-unit margin) and writes `src/workspace/icons/symbols_data.rs` with
`SYMBOLS: &[(&str, &str)]` (name, art). Each element's `class` names its roles (`"body"`,
`"tint"`, or `"body port"` = fill body, stroke port); it is filled unless `fill="none"`
and stroked when it has a `stroke`; the colour values are ignored (they only make the file
look right in a viewer). `path`, `rect`, `circle`, `ellipse`, `line`, `polyline`,
`polygon` and `g` with any `transform` are supported, other elements are errors. Layers
paint in document order: fill a body, then its details, then the outline.
`apps/symboltest` shows every symbol at 32 / 64 / 128 DIP and without power, light and dark
(`tests/sheet.uis`).

## 25. Event timeline and hex view

Two data widgets for the simulation mode of a network simulator (and any recorded events):
[`ui/timeline.rs`](../lib/ui/timeline.rs) and [`ui/hexview.rs`](../lib/ui/hexview.rs), both on
the page "Simulation" of `apps/datagallery`.

**Timeline**: a time axis, one lane of event ticks per track and a playhead. The app keeps a
`TimelineState` and passes the recorded span and its tracks every frame:

```rust
// state: tl: TimelineState; tcp, arp: Vec<(f64, u32)> -- (time in s, 0xAARRGGBB), sorted by time
let tracks = [TlTrack { label: "TCP", ticks: &self.tcp }, TlTrack { label: "ARP", ticks: &self.arp }];
ui.next_size(-1, 0); // natural height: the axis + 22 DIP per track
let ev = ui.timeline(ID_TL, &mut self.tl, (0.0, self.sim_end), &tracks);
if ev & EV_ACT != 0 {
    self.show_state_at(self.tl.t); // the user moved the playhead
}
```

* `tl.t` is the playhead (s), `tl.view` the visible range (empty: the whole span; zoomed out
  fully it grows with the span), `tl.sel` a range selected with Shift+drag; `tl.follow` keeps
  the playhead at the live end `span.1` (moving it by hand ends that, End starts it again);
  `tl.fit()` shows everything again. `EV_COMMIT` comes when a drag of the playhead ends.
* Every pixel column of a lane draws its ticks as one bar: taller for more, the colours
  stacked by share (colour 0: the chart palette colour of the track). Tens of thousands of
  ticks stay cheap; the bars are one cached draw object.
* Mouse: a press in the lanes seeks and drags the playhead, so does its time on the axis;
  dragging the axis pans, a click there seeks, a double click shows the whole span;
  Ctrl+wheel zooms around the pointer, Shift+wheel and the horizontal wheel pan (the plain
  wheel stays with the scroll views around it). Keys: Left / Right the previous / next tick
  of any track, Home / End, PgUp / PgDn, + / -, Esc clears the range. Pointer seeks are
  rounded to the decimal resolution of a pixel. The axis goes down to nanoseconds (labels with
  up to nine decimals, the narrowest view 10 ns), so the frames of a simulated LAN, which are
  microseconds apart, get lanes of their own.
* Scripts cannot hold Ctrl during `wheel`: `tl.test_mods = MOD_CTRL` treats it as held
  (`datagallery` maps `cmd 9100 2` to it).

**Hex view**: bytes as hex and ASCII side by side, with marked ranges (the fields of a
decoded packet) and a selection that the app and the user share:

```rust
// state: hex: HexState; fields: (offset, length, id) of the decoded layers
let marks: Vec<HexMark> = fields.iter().map(|f| HexMark { start: f.0, len: f.1, color: 0, key: f.2 }).collect();
ui.next_size(-1, 240); // without next_size it fills its parent
let ev = ui.hex_view(ID_HEX, &mut self.hex, &self.pdu, &marks);
if let Some(key) = ev.clicked_key {
    self.select_field(key); // a byte was clicked: the smallest mark holding it
}
// the other way round: a field chosen elsewhere selects its bytes, the view scrolls to them
self.hex.sel = Some((off, len));
```

* Rows `0010  45 00 00 54 ...  |E..T....|` in the system's `monospace` font (else the UI font
  cell by cell), virtualized in a scroll view; `hex.cols` bytes per row (0: 16 when they fit,
  else 8). Marks are translucent fills behind both columns (colour 0: the chart palette; an
  opaque colour gets a highlight's alpha), smaller ranges over larger ones.
* A click selects a byte, a drag a range (the view scrolls at its edges), Shift+click
  extends, a double click selects the mark, a click on an offset its row; arrows, PgUp / PgDn
  and Home / End move the caret (Shift extends, a move reports the caret's mark like a
  click), Ctrl+A selects all, Ctrl+C copies the selection as hex (`45 00 00 54`).
  `ev.hover` is the byte under the pointer (its offset is also a tooltip); `ev.ev` has
  `EV_ACT` (the selection changed), `EV_DOUBLE` and `EV_RCLICK` (open a context menu).
* Records: each visible row has `K_TEXT` records for its offset (`text:"0010"`), its hex
  and its characters; the view is a `K_LIST` (value: the caret, max: the number of bytes).

Both keep their left-to-right order in mirrored windows. Testing:
`apps/datagallery/tests/timeline.uis`, `hexview.uis`, `simulation.uis` (light, dark, 150 %).
## 26. Command trees: `base::cmdtree`

[`lib/base/cmdtree.rs`](../lib/base/cmdtree.rs) parses the lines of IOS-like command lines:
any unique prefix of a keyword works (`sh ip int br`), `?` lists what may follow with its
help text, Tab completes, modes have their prompts, `no` negates, parameters are typed, and an
error knows its position for the `^` marker. The tree holds no actions: the app matches on
its own command type `C` (an enum) and does the work; JFNetworker's IOS-like console
(`apps/JFNetworker/crates/jfn_sim/cli/ios.rs`) is the large example, `apps/coretest`'s group
`cmdtree` the small one.

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum C { Enable, ShowRoute, Hostname, Interface, IpAddr, Shutdown }

let mut t: Tree<C> = Tree::new();
let exec = t.mode("#", "Commands at the privileged level:", None, None);
let conf = t.mode("(config)#", "Commands for the whole device:", Some(exec), Some("Undo a setting or restore its default"));
let ifm = t.mode("(config-if)#", "Commands for this interface:", Some(conf), Some("Undo a setting or restore its default"));
t.add(exec, "enable", "Switch to the privileged level", Some(C::Enable), 0);
t.add(exec, "show", "Display the state of the device", None, 0);
t.add(exec, "show ip", "IPv4 state", None, 0);
t.add(exec, "show ip route", "Routes this device knows", Some(C::ShowRoute), 0);
t.add(conf, "hostname", "Give the device a name", None, F_NO);      // "no hostname" too
t.add(conf, "hostname WORD=name", "The new name", Some(C::Hostname), 0);
t.add_no(conf, "hostname", C::Hostname);                            // the no-form ends early
t.add(conf, "interface NAME=if", "", Some(C::Interface), 0);        // names from the app
t.add(ifm, "ip address A.B.C.D=addr MASK=mask", "The network mask", Some(C::IpAddr), 0);
t.add(ifm, "shutdown", "Switch the interface off", Some(C::Shutdown), F_NO);

match t.parse(conf, line, &names) {
    Ok(p) if p.cmd == C::Hostname => set_name(if p.no { "Router" } else { p.text("name").unwrap_or("") }),
    Ok(p) => run(p),
    Err(e) => out(&e.render(prompt.len(), line)),   // "% Invalid input detected at '^' marker."
}
out(&t.help(ifm, "ip address 10.0.0.1 ", &names).render(prompt.len(), line));   // "?" after a line
let c = t.complete(conf, "int gi", &names);   // c.line == Some("int GigabitEthernet")
```

* Paths are words: a keyword stands for itself (`trunk=mode` stores it under the key `mode`);
  parameters are `<lo-hi>`, `A.B.C.D`, `MASK` (its length), `A.B.C.D/nn`, `H.H.H` (MAC),
  `WORD`, `LINE` (the rest of the line) and `NAME` .. `NAME9` (lists the app gives through the
  `Names` trait, e.g. its interfaces; `*` in a name is a number: `"Loopback*"`). Shared
  prefixes merge; `add` on an existing path only sets its help, command and flags. A later
  value of a key hides an earlier one, so `name WORD=name` reads as the word.
* `also(mode, path, target)`: the words after `target` may follow `path` too, for options in
  any order (`ping X repeat 5 size 100 df-bit`).
* Matching: case is ignored, a whole keyword wins over a longer one it prefixes (`ip` /
  `ipv6`), WORD and LINE take only what no keyword takes, a name's type may be abbreviated and
  stand apart from its number (`Gi 0/0/0`). Ambiguity is an error (`% Ambiguous command:  "s"`).
* `Error::Invalid(byte)` is the first character no candidate accepts (`show ip routee` marks
  the second `e`); `Error::render(col, line)` writes the console text with the marker under it.
  `Incomplete` and `Ambiguous` render as the usual short `%` lines; `knows_first` tells an unknown first word
  (an IOS-like EXEC looks those up as host names) from a later error.
* Help (`Help::render`) lists labels in ASCII order (parameters and capitalised interface types
  before lowercase keywords), two spaces after the longest, `<cr>` when the line may end, the
  mode's title for an empty line; a partly typed word lists what it may become
  (`show  shutdown  `). `F_HIDDEN` keeps a command out of help and Tab.
* Modes carry their prompt suffix and the mode they fall back to (`mode_info(m).parent`): an
  IOS-like console tries a line that fails in a configuration submode again in the global
  configuration; the app decides when to do that. `find_mode(prompt)` finds a mode by its
  prompt, so a part of an app that adds its own modes later (JFNetworker's access-list modes)
  needs no fixed mode number.
* Write the help texts, mode titles and status lines in your own words; only the command
  words, their syntax, configuration lines and short `%` errors should follow the original
  (JFNetworker's rule, see its README).
* IPv6 parameters: `X:X:X:X::X` (an address in the text of RFC 4291 2.2; `Parsed::ip6` gives
  its 16 bytes) and `X:X:X:X::X/<0-128>` (with a prefix length; `Parsed::prefix6`);
  `cmdtree::parse_ipv6` reads such an address alone.

## 27. Keyboard operation

Every app built with the library can be used without a mouse. The library does most of it
([`ui/kbnav.rs`](../lib/ui/kbnav.rs), the widgets); an app adds a few marks where its own
layout means something. `python tools/kbwalk.py build/win/APP.exe [--keys f6] [--pre "cmd N"]`
presses a key again and again (headless) and lists where the focus goes: unreachable widgets,
a lost focus, a focus outside its window, widgets without a semantic record.

**What the library gives.**

| Keys | Effect |
|---|---|
| Tab / Shift+Tab | next / previous Tab stop of the window (the top layer: an open dialog or flyout keeps it); Ctrl+Tab is left to the app. The stops of one pane stay together (a dock group: its tab strip, its buttons, its panel). When the focused widget went away, Tab goes on from its place. |
| arrows, Home / End | inside a **focus group** (one Tab stop): radios of one variable (the selection follows), tool buttons of one variable, the swatches of one box, `focus_group_begin` groups (tool bars, segment rows, button grids); one row or column wraps, a grid moves to the nearest member |
| Space / Enter | act on the focused widget; check boxes, radios and switches only take Space (Enter is the dialog's default button) |
| Esc | the widget's (a text box leaves the focus), then `UIC_ESCAPE` / the dialog's close; a tool window that got the keys by F6 gives them back |
| F6 / Shift+F6 | next / previous **pane** of the window (dock groups, the document area, both sides of a split view, `focus_pane_begin`), past the last one the next window: the tool windows borrow the keys from the main window until Esc, F6 or a click elsewhere |
| Ctrl+F6 / Ctrl+Shift+F6 | next / previous window directly (main window, tool windows, dialog windows that are not modal) |
| F8 / Shift+F8 | the window's splitters (split views, dock sides and groups): arrows move the focused one by 8 DIP, Shift+arrows by 1, Home / End to the ends; Enter, Esc or F8 past the last go back |
| menu key / Shift+F10 | `context_menu` of the focused widget (a focused list: its selected row) at the widget's lower left corner with the first row focused; a widget without one gets a right click (`EV_RCLICK`) and `menu_popup` opens at the widget |
| Alt, F10, Alt+letter | the menu bar (§6); accelerators of the command table everywhere |
| lists, grids, trees | Up / Down / Home / End / PgUp / PgDn select, Enter = double click (`EV_DOUBLE`), Space = the selected row's first check box (lists) or its check (trees); `LF_REORDER` lists: Ctrl+Up / Ctrl+Down / Ctrl+Home / Ctrl+End move the selected row (`EV_DRAG` with from / to, as a drop); the widgets of a custom row are Tab stops in the selected row only |
| sliders, number boxes, the colour picker | arrows / PgUp / PgDn / Home / End change the value; the picker's spectrum, hue and alpha bars are Tab stops (Shift = 10 steps) |

A dialog window starts with the focus on its first control (no ring until a key is used;
Esc closes it at once even when that control is a text box); a content dialog moves the
focus into itself and gives it back when it closes, popups and menus give it back to the
widget that had it, and a closed tool window or dialog window gives the keys back to its
owner. The focus ring is the style's `focus_ring` (every style draws one; high contrast: the
text colour) and appears only after a key was used.

**What an app does.**

* Use the library's widgets; a custom widget built on `behave` / `hit_rect` passes
  `HB_FOCUS` (it is a Tab stop and takes the focus on a press), reacts to `EV_ACT` (Space /
  Enter) and calls `ui.focus_ring_draw(ev)` after its painting. `HB_NOTAB`: focusable but no
  Tab stop; `HB_CURRENT`: the member a group is entered at; `HB_SPACE`: Space acts, Enter
  does not; `HB_OWNKEYS`: Space and Enter are the widget's own; `HB_SPLITTER`: a splitter
  F8 reaches -- `ui.splitter_keys(ev, along_x)` says how far (DIP) the arrows move it. Give
  it a `sem` record.
* Mark tool bars and button grids as one Tab stop: `ui.focus_group_begin(id)` ..
  `ui.focus_group_end()` (id 0: the current box). A selection that follows the focus (tabs)
  asks `ui.group_arrived(id)`.
* Mark the parts of a window the user switches between with F6: `ui.focus_pane_begin(id)` ..
  `ui.focus_pane_end()` (dock areas and split views do it themselves).
* A decoration that looks like a control is no Tab stop: `ui.next_flags(ST_NOFOCUS)` before a
  button or swatch.
* Do not take F6, F8, Tab or the menu key as accelerators unless the app needs them (an
  accelerator wins: PaintCraft keeps paint.net's F5 .. F8 for its windows, Ctrl+F6 still
  switches them).
* Test it: `key tab`, `key ctrl+f6`, `expect S focused`, `expect window ID keys` (§12), and
  `tools/kbwalk.py` for the whole ring.

```rust
// a tool bar: one Tab stop, the arrows move between the buttons
ui.box_begin(0, BOX_H | BOX_XCENTER, 4);
ui.focus_group_begin(0);
for &(id, icon, tip) in TOOLS.iter() {
    if ui.icon_button(id, icon, tip) & EV_ACT != 0 {
        ui.cmd_run(id, 0);
    }
}
ui.focus_group_end();
ui.box_end();
// a custom swatch grid: focusable cells, a ring, Space = click, menu key = right click
ui.focus_group_begin(ID_GRID);
for i in 0..n {
    let ev = ui.hit_rect(ID_CELL0 + i, x(i), y(i), s, s, HB_FOCUS);
    paint_cell(ui, i);
    ui.focus_ring_draw(ev);
    ui.sem(K_SWATCH, ID_CELL0 + i, Some(&label(i)), 0);
}
ui.focus_group_end();
```

## 28. Rich text and code editors: `ui.richtext`, `ui.code_editor`

Two ready-made editors keep their document in a model of their own (not one `String` as the
text boxes do): [`RichDoc`](../lib/ui/rich_doc.rs) -- paragraphs of formatted runs -- and
[`CodeDoc`](../lib/ui/code_doc.rs) -- lines. Both models carry their selection and an undo
history (`ui::History`), so the app only keeps the document and draws the widget every
frame. A shared core ([`ed_core.rs`](../lib/ui/ed_core.rs)) gives them the keys of the text
boxes (grapheme clusters, words, Home / End, PgUp / PgDn, Ctrl+A / C / X / V / Z / Y), IME and
magnifier caret (`ime_pos`; the composition of IBus / Wayland is drawn at the caret) and a
`K_TEXTBOX` record for screen readers and scripts.
`apps/editortest` shows both (`tests/*.uis`, `--test model` for the models).

**Rich text.**

```rust
// once (init): fonts of the app's own become families of documents and of the toolbar
let f = ui.text.load_mem(SERIF, 0);
if f >= 0 { ui.font_register("DejaVu Serif", 400, false, f as u32); }
// the document: build it, or load it
self.doc = RichDoc::from_paras(vec![
    Para::new().with("Title", Fmt::default()).attrs(RA_LEFT, LS_NONE, 0, PS_H1),
    Para::new().with("Some ", Fmt::default()).with("bold", Fmt::bold()).with(" text.", Fmt::default()),
    Para::plain("an item").attrs(RA_LEFT, LS_BULLET, 0, PS_NORMAL),
]);
let doc = RichDoc::from_json(&saved).unwrap_or_default();

// in frame: an optional toolbar, then the editor filling its place (ids ID_ED .. ID_ED + 5)
ui.richtext_toolbar(ID_TOOLS, ID_ED, &mut self.doc, RTB_ALL);   // ids ID_TOOLS .. + 149
ui.next_size(-1, -1);
if ui.richtext(ID_ED, &mut self.doc, 0) & EV_ACT != 0 { self.dirty = true; }
if let Some(url) = ui.richtext_link(ID_ED) { ui.open_url(&url); } // Ctrl+click on a link

// saving: JSON keeps everything, HTML and plain text for others
let json = self.doc.to_json();
let html = self.doc.to_html();
let text = self.doc.to_plain();          // with list markers ("• ", "1. ")
```

* Characters (`Fmt`): bold, italic, underline, strike (`RF_*`), colour, highlight, size (tenths
  of a point), font family (`font_register` families, `"monospace"`, else `ui.font_load`), a
  link. Paragraphs (`Para`): alignment `RA_LEFT / CENTER / RIGHT / JUSTIFY`, lists
  `LS_BULLET / LS_NUMBER` with a level 0..8 (•, ◦, ▪ and 1., a., i. by level), styles
  `PS_H1..H3`, `PS_QUOTE`. Without a selection a format applies to what is typed next.
* Editing from code goes through the document (each call one undo step): `insert_text`,
  `insert_paras`, `delete`, `split`, `toggle(RF_BOLD)`, `set_color`, `set_size`, `set_font`,
  `set_link` (on the link at the caret, `""` removes it), `set_align`, `toggle_list`, `indent`,
  `set_style`; `undo` / `redo` / `is_dirty` / `mark_saved`. The attribute fields of a `Para`
  may be set directly while building a document (not undoable).
* Keys: Ctrl+B / I / U, Ctrl+Shift+X (strike), Ctrl+Space (clear), Ctrl+K (link flyout),
  Ctrl+L / E / R / J (alignment), Ctrl+Shift+8 / 7 (lists), Tab / Shift+Tab (list level),
  Ctrl+1 / 2 / 3 / 0 (headings, body), Ctrl+] / [ (size), Ctrl+Shift+V (paste text only);
  Enter in an empty list item ends the list, Backspace at an item's start makes it body text.
* Clipboard: text (list markers when several paragraphs are copied) and HTML; the HTML holds
  the exact fragment as JSON in a comment, so copy and paste between richtexts keep every
  attribute; HTML from browsers and office programs is read (`RichDoc::from_html`).
* Flags: `RT_READONLY` (select, copy; a click opens a link), `RT_NO_FRAME`, `RT_OPEN_LINKS`
  (the editor calls `open_url` itself), `RT_TAB` (Tab inserts a tab outside lists),
  `RT_PLAIN_PASTE`. High contrast replaces the document's colours by the scheme's.

**Code.**

```rust
// once: the font (as for consoles) and the document
let f = ui.text.load_mem(MONO, 0);
if f >= 0 { ui.mono_font_set(f as u32); }
self.code = CodeDoc::from_text(&text);
self.code.tab_width = 4;               // use_tabs = true indents with tabs
self.hl = Highlighter::for_name("main.rs");   // rust(), json(), ini(), toml(), markdown(), plain()

// in frame: the editor fills its place (ids ID_CODE .. ID_CODE + 15)
ui.next_size(-1, -1);
if ui.code_editor(ID_CODE, &mut self.code, &self.hl, 0) & EV_ACT != 0 { self.dirty = true; }
let st = ui.code_status(ID_CODE);      // line / column of the caret, find matches
ui.text(TS_CAPTION, &format!("Zeile {}, Spalte {}", st.line, st.col));
// saving
let text = self.code.text();
```

* The editor shows line numbers, the current line, the bracket pair at the caret (outside
  strings and comments), a find / replace bar (Ctrl+F / Ctrl+H, incremental, case, whole word,
  F3 / Shift+F3, Enter / Shift+Enter, "Alle ersetzen" as one undo step); `ui.code_find(id,
  query, replace)` opens it from a menu. Enter keeps the indentation (one level more after an
  opener, the closer of `{}` on a line of its own), a typed closer on a blank line goes out a
  level, Tab / Shift+Tab indent the selected lines, Ctrl+/ toggles line comments, Alt+Up /
  Down move lines, Home toggles first non-blank / column 0, Ctrl+C / X without a selection
  take the line.
* Large files: only the visible lines are laid out and highlighted; the editor keeps the
  state each line starts in and follows `CodeDoc::changes_since`, so an edit re-highlights
  only until the old states agree again. 100 000 lines scroll and edit at the speed of a
  short file (`apps/editortest/tests/code_bench.uis`).
* Long lines (minified JSON / JS, logs): of a line of 4 KB and more only the part in view is
  drawn; the editor keeps checkpoints of its columns and its tokens, and after an edit works
  them again only near it (tokens from 4 KB before the edit to where the old ones agree
  again). Typing, moving the caret, selecting, scrolling sideways and find stay well under a
  millisecond or two per pass on a line of 100 000 characters and on a 1 MB single-line file
  (`apps/editortest/tests/code_longline.uis`). A construct that decides by looking more than
  4 KB ahead (Markdown emphasis waiting for its closer) catches up when the whole line is
  highlighted again (another start state or grammar). The editor has no soft wrap; very long
  lines scroll sideways.
* Highlighting: a `Grammar` is a list of `Rule`s tried at each position (line comments,
  blocks with nesting, strings with escapes and multi-line, word lists, numbers, punctuation,
  `Custom` functions) plus a few switches (identifiers before `(` are functions, capitalised
  ones types). An app's own language is a `static Grammar` and `Highlighter::new(&MY_GRAMMAR)`.
  `h.line(text, state, &mut tokens)` is the engine itself (tests, other views).
* Colours: `ui.code_theme()` follows the UI (light / dark syntax colours on the text box
  colours, the high-contrast scheme's few colours with keywords bold);
  `ui.code_theme_set(Some(CodeTheme::dark()))` fixes them, `ui.code_font_size(pt)` sets the size.
* Flags: `CE_READONLY`, `CE_NO_GUTTER`, `CE_NO_FRAME`, `CE_NO_FIND`, `CE_NO_LINE`.
* Tests: the record of the editor is a text box (`expect #3000 text "fn main() {\n    "`),
  beyond 64 KB it holds the visible lines. Mouse tests need `wait 1000` between clicks on the
  same editor, else they count as a double click.

**Diagnostics** (a checker's or compiler's messages) belong to the document:

```rust
// after a change (or a pause in typing): lines and columns from 1 as compilers print them
if ui.code_editor(ID_CODE, &mut self.code, &self.hl, CE_MARKS) & EV_ACT != 0 {
    let d: Vec<Diag> = script::check(&self.code.text()).iter()
        .map(|x| Diag::new(x.line as usize, x.col as usize, x.end_col as usize,
                           if x.error { DIAG_ERROR } else { DIAG_WARNING }, &x.msg))
        .collect();
    self.code.set_diagnostics(&d);
    ui.repass();                       // the editor drew the old ones in this pass
}
let st = ui.code_status(ID_CODE);      // st.diags: errors / warnings / infos, st.diag: at the caret
if let Some(d) = ui.code_diag(ID_CODE) { self.status = format!("{}:{}: {}", d.line, d.col, d.msg); }
```

* `Diag::new(line, col, end_col, severity, msg)` marks a part of a line (`Diag::span` one over
  lines); columns count characters (a tab is one), the end is exclusive, an end at or before
  the start marks one character, a column past the line's end its end. Severities
  `DIAG_ERROR`, `DIAG_WARNING`, `DIAG_INFO`. The positions are those of the text as it is
  when they are set (a check on another thread: drop its result when `doc.revision()` changed
  meanwhile).
* The editor draws wavy underlines in the severity's colour (`CodeTheme::diag`; `code_theme()`
  takes the UI's critical / warning / info colours) and, on the line where one starts, a
  marker in a gutter column of their own: a filled circle with a cross, a triangle, an "i" --
  shapes, so high contrast and colour-blind readers tell them apart. The column is there while
  the document has diagnostics; `CE_MARKS` keeps it, so the text does not move when they come
  and go. The pointer over an underline or a marker shows the messages as a tooltip
  ("Fehler: …", most severe first); F8 / Shift+F8 move the caret to the next / previous one
  (round the end; without diagnostics F8 stays the window's key for the splitters); the ones at
  the caret are also a `K_TEXT` record at its line for screen readers.
* They follow the edits (typing, Enter, deleting, block indent, comments, moved lines, undo /
  redo) until the app sets new ones: text before a change keeps its place, text after it
  shifts, a diagnostic whose text was replaced shrinks to what is left of it and is dropped
  when nothing is, typed text at its edges stays outside. So a check may come later (after a
  pause in typing) without squiggles sliding off their words. Where they are now:
  `diagnostics()` / `diagnostic(i)` (a problems list), `diagnostics_at(pos)`,
  `next_diagnostic(pos, back)`, `diagnostic_counts()`.
* Only the visible lines' diagnostics are drawn, an underline is one command however long it
  is: 100 000 lines with 13 000 diagnostics scroll and edit at the speed of a short file
  (`apps/editortest/tests/code_diag.uis`: hover, F8, edits, checking while typing, shots).
* `Highlighter::jfs()` (`for_name("x.jfs")`) highlights lib/script's language: keywords,
  `true` / `false` / `nil`, numbers (`1_000`, `0xFF`, `0b101`, `1.5e-3`), `//` and `/* */`,
  strings with escapes and `${…}` whose code is highlighted too (the delimiters are `SY_META`,
  so their braces match as a bracket pair). A grammar of one's own gets such strings with
  `Rule::Interp { open, close, start }`. `editortest --test model` compares it token by token
  with `script::tokens`.

## 29. Scripts: `script` (compiler and VM)

[`lib/script`](../lib/script/lib.rs) runs JFNetworker's `.jfs` scripts: `compile` turns the
source into bytecode, a `Vm` runs it with fuel (virtual CPU time), a heap of its own with a
mark-and-sweep collector and hard limits. Everything is deterministic: no clock, no randomness,
float arithmetic in software (`script::num`), so a script gives the same bits on every target,
and a run split into slices of any size gives the same result as one uninterrupted run. The
syntax and the editor helpers (`check`, `tokens`, `outline`) are in the crate comment.

```rust
use script::{Cx, Fail, Host, Limits, Status, Value, Vm};

let prog = match script::compile_named("server.jfs", src) {
    Ok(p) => p,
    Err(diags) => return show(&diags),                 // "3:5: Fehler: …", warnings left out
};
let mut vm = Vm::new(&prog, Limits::default());       // the top-level code is pending
let mut host = MyHost::default();
loop {
    match vm.run(&mut host, 10_000) {                 // at most 10 000 fuel per slice
        Status::Done(_) => break,
        Status::Yielded => continue,                  // a simulator would let time pass here
        Status::Error(e) => { log(&e.to_string()); break }   // message, position, stack trace
    }
}
let req = vm.new_str("GET /")?;
vm.call("on_request", &[req])?;                       // a global function, then run() again
let v = vm.global("hits");                            // read and set globals
```

A host gives scripts their functions by name. The standard library (below) answers first, so
names such as `str.split` or `list.map` never reach the host:

```rust
#[derive(Default)]
struct MyHost { handlers: Vec<script::Pin> }

impl Host for MyHost {
    fn call(&mut self, cx: &mut Cx, name: &str, args: &[Value]) -> Result<Value, Fail> {
        match name {
            "log" => { emit(&cx.display(args[0])); Ok(Value::Nil) }      // log(x)
            "http.serve" => { self.handlers.push(cx.pin(args[1])); Ok(Value::Nil) }
            "map.json" => {                                             // res.json(200, v)
                let body = cx.json_encode(args[2], 0)?;                 // like json.encode
                send(&body);
                Ok(Value::Nil)
            }
            "list.each" => {                                            // [1, 2].each(f)
                let items = cx.list(args[0]).map(|l| l.to_vec()).unwrap_or_default();
                for x in &items {
                    cx.keep(*x);                                        // f may empty the list
                }
                for x in items {
                    cx.spend(1)?;                                       // fuel by the work
                    cx.call(self, args[1], &[x])?;                      // f runs to its end
                }
                Ok(Value::Nil)
            }
            "fail" => Err(cx.error("kaputt")),                          // a catchable exception
            _ => Err(Fail::Unknown),                                    // "Unbekannte Funktion …"
        }
    }
    fn knows(&self, name: &str) -> bool { name == "log" }              // `let f = log` works
}
```

* Names: a name the script does not declare comes from the host. `print`, `len`, `type`,
  `str`, `int`, `float` are built in (`print` writes to `vm.output()` / `take_output()`,
  capped by `Limits::output`), the standard library comes next. `f(..)` and `ns.f(..)` call
  `Host::call` with `"f"` / `"ns.f"`; `obj.m(..)` calls a function stored in a map under `m`,
  then the built-ins and the library, then `Host::call("list.m", [obj, ..])` (the type name
  first, the object as the first argument). `vm.set_global("config", map)` gives a host name a
  value, which then wins: `config.port`, or `math.sqrt(2.0)` when `math` is a map holding
  `vm.native("math.sqrt")`.
* Natives: `cx.spend(n)?` adds fuel for the work done and ends the invocation once
  `Limits::fuel` is used up (call it as a long job goes; `cx.charge(n)` only adds),
  `cx.yield_now()` ends the run after the call (`Status::Yielded`, the next `run` continues),
  `cx.call(self, f, &args)` runs a callback to its end (no yield in between, nested at most 20
  deep; an exception that leaves it keeps its position and stack trace). Objects a native
  allocates stay alive until it returns; values it only holds while calling back need
  `cx.keep(v)`. `cx.json_encode(v, indent)` / `cx.json_decode(text)` (and
  `vm.json_encode` / `vm.json_decode` between runs) work like `json.encode` / `json.decode`.
  `cx.fuel()` is the fuel of the invocation so far, over all its runs and this call included:
  a host that turns fuel into time knows the moment a native is called at (JFNetworker's
  scripts send a packet then: `apps/JFNetworker/crates/jfn_sim/scriptsvc.rs`).
* Values (`Value`, 16 bytes) refer to the VM's heap. A value the host keeps beyond the next
  `run` must be pinned (`pin` / `pinned` / `unpin`), e.g. a request handler; `valid(v)` tells
  a stale one, and the API returns `None` or an error instead of crashing on it.
* Fuel: one per instruction, more for copying (string and list concatenation, literals, map
  iteration). `run(host, fuel)` stops before the next instruction once `fuel` is used (it may
  use a little more: `last_fuel()`). `Limits::fuel` caps one invocation over all its runs, so
  an endless loop ends with `ErrorKind::Fuel` ("Rechenzeit aufgebraucht …"). JFNetworker turns
  fuel into simulated CPU time.
* Limits (`Limits`): heap bytes (accounted the same way on every target), stack values, call
  depth, fuel per invocation, print output. Crossing one ends the invocation at once; scripts
  cannot catch it. Runtime errors (`1 / 0`, a bad index, wrong types, unknown functions) throw
  a `str` with a German message that `try`/`catch` can catch; uncaught, they end the
  invocation with an `Error` (`kind`, `msg`, `line`, `col`, `trace`). The VM stays usable:
  globals keep their values, `call` works again; `cancel()` drops a pending invocation.
* `Vm: Clone + Send`: a clone is a full copy (heap included) that runs on independently, also
  from the middle of a yielded invocation; a `Program` is shared (`Arc`) between VMs.
* Semantics for script writers: `nil` and `false` are falsy, everything else is truthy; `int`
  is i64 with wrapping `+ - *`, `/` and `%` truncate and throw on 0; an `int` meeting a `float`
  becomes a `float`; `==` compares numbers exactly across types, `str` and `bytes` by content,
  lists, maps and functions by identity; maps keep insertion order, keys are `str`, `int` or
  `bool`, a missing key reads `nil`; `for k, v in map` visits the keys present at its start;
  missing arguments are `nil`, extra ones are dropped; closures capture variables (a loop
  variable afresh each round).
* Testing: `coretest --only script` (parser), `coretest --only scriptvm` (compiler, VM,
  collector, fuel, yield at every instruction against an uninterrupted run, soft float against
  the hardware) and `coretest --only scriptlib` (the standard library); `program.disasm()`
  prints the bytecode.

**The standard library** ([`stdlib.rs`](../lib/script/stdlib.rs) and its neighbours) is part of
every VM, whatever the host. Methods take the value first, so `s.split(",")` and
`str.split(s, ",")` are the same call; a map's own field of the same name wins over a map
method (`map.keys(m)` always reaches the library), and a script may shadow a module with a
variable (`let list = []`). `script::library()` lists every name for editors.

| Module | Functions and constants |
|---|---|
| built in | `print(..)`, `len(x)`, `type(x)` (`"nil" "bool" "int" "float" "str" "bytes" "list" "map" "fn"`), `str(x)`, `int(x)` (truncates floats, parses `"42"`, `"0x1F"`, `"-0b11"`, throws otherwise), `float(x)` |
| `str` | `s.len()` `s.slice(start, end)` `s.split(sep, n)` `sep.join(list)` `s.trim(chars)` `s.trim_start(chars)` `s.trim_end(chars)` `s.find(sub, from)` `s.find_last(sub)` `s.contains(sub)` `s.starts_with(p)` `s.ends_with(p)` `s.count(sub)` `s.replace(from, to, n)` `s.upper()` `s.lower()` `s.repeat(n)` `s.chars()` `s.bytes()` `s.code(i)` `str.from_code(n, ..)` `s.format(args..)` `s.hash()` |
| `list` | `l.len()` `l.push(x, ..)` `l.pop()` `l.insert(i, x)` `l.remove(i)` `l.slice(start, end)` `l.reverse()` `l.sort(cmp)` `l.map(f)` `l.filter(f)` `l.reduce(f, init)` `l.contains(x)` `l.index(x)` `l.join(sep)` |
| `map` | `m.len()` `m.keys()` `m.values()` `m.entries()` `m.has(k)` `m.get(k, default)` `m.delete(k)` `m.merge(other, ..)` |
| `bytes` | `bytes()` `bytes("text")` (UTF-8) `bytes([1, 2])` `bytes(1, 2)` `b.len()` `b.slice(start, end)` `b.hex()` `bytes.from_hex(hex)` `b.text(lossy)` `b.hash()`; also `b[i]`, `b + b`, `for x in b` |
| `math` | `abs` `min` `max` `clamp` `floor` `ceil` `round` `trunc` `sqrt` `pow` `exp` `log` `log2` `log10` `sin` `cos` `is_nan` `is_finite`; `math.pi` `math.e` `math.tau` `math.inf` `math.nan` |
| `json` | `json.encode(v, indent)` `json.decode(text)` |
| `int`, `float` | `int.parse(s, base)` `float.parse(s)` (`nil` when `s` is no number), `int.max` `int.min` |

```js
// a request path and a header
let parts = "/users/42/posts".split("/").filter(fn(p) { return len(p) > 0 })
let id = int.parse(parts[1]) ?? 0                    // 42; nil for "abc", so ?? gives 0
let kv = "Content-Type:  text/html".split(":", 2)    // ["Content-Type", "  text/html"]
print(kv[0].lower(), kv[1].trim())                   // content-type text/html

// records sorted stably by a comparison (a number, or a bool: true puts a first)
let jobs = [{n: "b", prio: 2}, {n: "a", prio: 1}, {n: "c", prio: 2}]
jobs.sort(fn(x, y) { return x.prio - y.prio })       // a, b, c
print(jobs.map(fn(j) { return j.n }).join(", "))     // a, b, c

// JSON keeps the order of maps; ints stay ints, floats floats
let t = json.encode({id: 7, tags: ["x"], load: 0.5}) // {"id":7,"tags":["x"],"load":0.5}
let v = json.decode(t)
print("{:>4}|{:.2}|{:#x}".format(v.id, v.load, 255)) //    7|0.50|0xff

// math in software: the same bits on every machine
let backoff = math.min(30.0, 0.1 * math.pow(2, 5))   // 3.2
let load = 100 + 50 * math.sin(2 * math.pi * 3 / 24) // a daily curve
let shard = "user:42".hash() % 8                     // FNV-1a, stable everywhere
```

* Strings: lengths and indexes count characters (`"Größe".len()` is 5, `.find("ö")` 2);
  `slice` takes negative indexes from the end and clamps the others; `find` answers `nil`
  when there is no match; `split()` without a separator splits at runs of white space,
  `split("")` into characters, `n` caps the parts (the last keeps the rest); `upper` / `lower`
  map Unicode fully (`"straße".upper()` is `"STRASSE"`); `hash()` is FNV-1a (32 bits) of the
  UTF-8 bytes, for hash rings and shards.
* `format`: `{}` the next argument, `{0}` one by position, `{name}` a field of the first
  argument (a map), `{{` and `}}` for braces; after a colon
  `[[fill]align][+][#][0][width][.precision][type]` with align `<` `>` `^`, type `x X b o d`
  (ints), `f e` (numbers), `s`. A precision rounds the exact value (ties to even:
  `"{:.1}".format(0.25)` is `0.2`); width at most 10 000, precision at most 100.
* Lists: `insert`, `remove`, `reverse` and `sort` change the list (`reverse` / `sort` return
  it), the others make a new one; `insert` takes `0..=len`, `remove` `0..len`. `map` /
  `filter` call `f(item)` with the item alone (so `l.map(str.trim)` works) on the items the
  list had at the call; `reduce` without `init` starts with the first item. `sort` is a stable
  merge sort of its own (the same comparisons on every target): without `cmp` it orders
  numbers by value (ints against floats exactly, `nan` last), strings and bytes by their
  bytes, and throws for other or mixed types; an error or an exception in `cmp` leaves the
  list as it was.
* Maps: `keys` / `values` / `entries` in insertion order; `delete` answers whether the key was
  there; `merge` makes a new map (later maps win, a key keeps its first position);
  `get(k, d)` answers `d` for a missing key.
* math: `abs`, `min`, `max`, `clamp`, `floor`, `ceil`, `round` and `trunc` keep ints as ints
  (`math.floor(7)` is `7`, `math.floor(2.5)` is `2.0`); `round` rounds halves away from zero;
  `min` / `max` take arguments or one list and answer `nan` when one is `nan`. The float
  functions ([`math.rs`](../lib/script/math.rs)) are fdlibm in its FreeBSD / musl form on
  `script::num`: errors below 1 ulp, exact where the result is (`math.log2(8)` is `3.0`,
  `math.pow(2, 10)` is `1024.0`), `exp` and `pow` bit for bit like Java's `StrictMath`
  except that `pow` follows C99 where Java answers NaN (`pow(1, nan)`, `pow(-1, inf)` and
  `pow(1, inf)` are 1; `pow(x, 0)` is 1 even for `nan`); `sin` / `cos` reduce their argument
  exactly, so `sin(1e300)` is right too. `tools/script_math_check.py [--target both]` checks
  them against Python's `decimal` (and Windows against Linux).
* json: `encode` writes compact JSON (or `indent` 1 to 8 spaces per level); int and bool keys
  become their text; `nan` / `inf`, bytes, functions, colliding keys (`1` and `"1"`), data
  that contains itself and more than 64 levels throw. `decode` takes a str or bytes and makes
  maps in member order, numbers without a point or exponent ints (floats when they do not
  fit), the others floats; errors throw with the position:
  `json.decode: Hier fehlt ein Doppelpunkt (Zeile 1, Spalte 6)`.
* Errors in arguments throw a catchable `str` in German, named after the function and the
  parameter (`str.split: sep muss str sein, nicht int`,
  `list.insert: Index 5 liegt außerhalb der Liste (Länge 3)`).
* Fuel follows the work, on top of the 1 of the call: 1 per new object, 1 per 64 bytes of text
  read or written, 1 per 8 list values and 1 per 4 map entries copied or visited, 1 per
  comparison of a sort, 2 for `sqrt`, 4 for `exp log log2 log10 sin cos`, 8 for `pow`, plus
  what callbacks cost. Long operations pay as they go, so `Limits::fuel` stops a huge sort,
  split or JSON job in the middle (`ErrorKind::Fuel`, not catchable), and a list being
  sorted stays as it was.
* Heap: results are allocated in the VM, so `Limits::heap` holds for them (`ErrorKind::Heap`,
  not catchable), and a text whose size is known before it is built (`repeat`, `replace`,
  `join`, `format`) is checked first: `"x".repeat(1000000000000)` fails at once instead of
  allocating. JSON needs up to about three times the heap limit as temporary memory for its
  syntax tree.

## 30. Live inspector

A running app can be looked into without changing it: the inspector shows which widget is
where, with which id, label, value and state, how the layout boxes nest, the Tab order, what
the last frames repainted, and the frame's cost. It is part of `ui` (`lib/ui/inspect.rs`)
and off by default; closed it costs nothing (the runtime tests one flag; nothing is recorded or
drawn, the frames are byte for byte those of an app without it).

* **Turning it on**: `--inspect` opens it at the start; `jfgui_INSPECT=1` only arms it (its
  hot keys work), `jfgui_INSPECT=open` opens it too. In code: `ui.inspect_enable(true)` (arm),
  `ui.inspect_show(true)` (open; arms it), `ui.inspect_open()`. The hot keys **Ctrl+Shift+I**
  and **F12** open and close it while it is armed (they are then the inspector's, not the
  app's); `ui.inspect_keys(&[VK_F11 | MOD_CTRL << 16])` sets others, `&[]` none.
* **The overlay** in every window of the app: the widget under the pointer outlined (orange)
  with a tag -- kind, id (`#100`, hashed ids in hex with their name from `ui.id("...")` when the
  frame used it), label, size in DIP, state flags; the selected widget (blue). Options in the
  tool window: **Umrisse** (every layout box), **Tab-Reihenfolge** (the Tab stops numbered as
  Tab visits them), **Neu gezeichnet** (the rectangles each frame repainted, fading out: a
  widget that repaints too much shows at once). The overlay is drawn into the framebuffer
  after the app's commands, outside their lists: the app's dirty regions stay its own, and
  `--verify-dirty` checks the overlay too.
* **The tool window "Inspektor"** (a tool window: it never takes the keyboard by itself; its
  Esc, close button and drops never reach the app): **Widgets** -- the tree of the current
  frame (the semantic records of every app window inside the layout boxes they were emitted
  in; boxes without widgets are left out; collapsible; selecting a row highlights the widget
  in the app) and the selected widget's properties (kind, id and name, label, rectangle in DIP
  and px, value / min / max, state, layer, window) with a **script selector** (`kind:"Label"`
  when that names it alone, else `#id`, else its centre `X,Y`), copied with **Kopieren** or
  Ctrl+C in the inspector; **Bildaufbau** -- passes of the last frame, the time of `App::frame`
  and of the render, the repainted area, draw commands (and the bytes of the command lists),
  the number of widgets, the glyph cache's use; **Farben** -- every theme token with its colour.
  **Auswählen** starts the pick: the next click into the app selects the widget there and
  does not reach the app; Esc ends the pick.
* The inspector's own window never appears in the tree, the overlay or the pick. It takes one
  of the eight window slots (`WIN_MAX`); F6 / Ctrl+F6 reach it like any tool window.
* A widget appears in the tree when it records itself (`ui.sem`, as every library widget does);
  custom widgets built from `hit_rect` and drawing calls should call `ui.sem(K_*, id, label,
  value)` after their `behave` -- the dump, scripts, screen readers and the inspector see the
  same records.

```
# apps/gallery/tests/inspect.uis (excerpt)
inspect snap                         # the main window's frame
inspect arm
key f12                              # the hot key opens it
inspect expect open
move "Standard"
inspect expect hover button:"Standard"
inspect pick
click "Standard"                     # selects; the app does not see the click
inspect expect selected button:"Standard"
inspect expect selector "button:\"Standard\""
click button:"Kopieren"
inspect expect copied "button:\"Standard\""
key f12                              # closed again
inspect expect same                  # renders byte for byte as at the snap
inspect expect idle                  # and did no work since it closed
```

The script commands: `inspect on|off|toggle|arm|disarm`, `inspect pick`, `inspect select
S|none`, `inspect option outlines|order|dirty on|off`, `inspect tab widgets|frame|colors`,
`inspect copy`, `inspect snap`, `inspect expect open|closed|pick on|off|flash on|off|idle|same|
excluded|selected S|none|hover S|none|selector "s"|copied "s"` (`excluded`: no record of the
inspector's window in the tree). The inspector's window has the id 2147422208 (`expect window
2147422208 open`, `shot F window 2147422208`).

## 31. SVG images and the image viewer

[`lib/img_svg`](../lib/img_svg/lib.rs) reads SVG files (a practical SVG 1.1 / SVG Tiny subset)
and draws them through gfx's path rasterizer: sharp at every size, no libc. A document is
parsed once into an immutable `Svg` and drawn as often as needed. The UI shows it with
`ui.svg` (pictures and icons at their layout size) or `ui.image_view` (zoom and pan); the
runtime also registers it as the image format `IMGF_SVG`, so `img_core::io::load` /
`load_ex` / `decode_ex` return `.svg` and `.svgz` files as bitmaps of their intrinsic size.

```rust
// once: parse (UTF-8 XML; gzip data of .svgz is inflated); Err(img_core::ERR_*)
let Some(data) = plat::read_file("app://art/logo.svg") else { return };
let Ok(doc) = ui::Svg::parse(&data) else { return };
let doc = Rc::new(doc);
let (w, h) = doc.size();                     // intrinsic px: width / height, else the viewBox
// per frame: a picture at 64 DIP wide (height from the aspect ratio), drawn at render time
// at the device size; currentColor = the theme's text colour; K_IMAGE with its <title>
ui.svg(ID_LOGO, &doc, 64, 0);
// a bitmap of any size, outside the UI (no text: Opts::default() has no font engine)
let sf = doc.rasterize(256, 256, &mut img_svg::Opts::default())?;
```

* Drawing yourself (a canvas `Draw`): `doc.render_rect(g.sf, x, y, w, h, Some(g.dc.clip),
  &mut opts)` maps the viewBox into the rectangle (preserveAspectRatio);
  `doc.render(sf, &matrix, clip, &mut opts)` draws the intrinsic size through any matrix
  (zoomed, rotated). `Opts { color, text }`: the colour of `currentColor`, and the font
  engine for `<text>`: `ui::SvgText { text: g.text, fonts: &fonts }` (fonts =
  `ui.svg_fonts()`, taken at frame time into the `Draw` object) sets text with the UI's fonts (families registered with `ui.font_register`, `monospace` =
  `ui.mono_font_set`, everything else the style's fonts; italic is slanted). Without a font
  engine text is skipped.
* Supported: `svg` (nested viewports), `g`, `defs`, `use` + `symbol`, `switch` (its first
  child), `a`; `path` (every command, arcs), `rect` (rx / ry), `circle`, `ellipse`, `line`,
  `polyline`, `polygon`; transforms; fill / stroke with colours (hex, `rgb()`, `hsl()`, the
  148 names, `currentColor`), opacities, fill-rule, width / caps / joins / miter limit /
  dashes; linear and radial gradients (stops, units, transform, spread, focal point, `href`
  inheritance); `opacity` of groups (layers); `clipPath`; `display`, `visibility`; `<style>`
  sheets (type, class, id and attribute selectors, descendant and child combinators), the
  `style` attribute, presentation attributes, `inherit`; lengths in px / pt / pc / mm / cm /
  in / em / ex / %; `text` / `tspan` (one line per chunk, x / y / dx / dy, text-anchor, font
  size / weight / style / family); `image` with `data:` URIs of the registered raster
  formats. Not supported (drawn without): `pattern` (its fallback colour is used), `mask`,
  `marker`, `filter`, text on paths, per-glyph positions, external files, CSS pseudo-classes
  and `@media`, animation, scripts.
* Robustness: malformed files never panic; the reader is forgiving (unclosed tags, unquoted
  attributes, a broken path draws up to the error, `svg.truncated()` tells). Limits: 32 MB
  per file (also inflated), 250 000 elements, 256 levels, entity references expand once (no
  exponential growth), `use` cycles are cut, 1 000 000 element visits and 64 levels of
  nesting per render, a step budget for CSS selectors.

**The image viewer** shows a bitmap or an SVG in a zoomable, scrollable view; the app keeps an
`ImageViewState`:

```rust
// state: self.view = ImageViewState::default();      (fit mode at first)
let ev = ui.image_view(ID_VIEW, &mut self.view, ViewImage::Bitmap(&self.photo, self.photo_ver), 0);
// or ViewImage::Svg(&self.doc): drawn again at every zoom, only the visible part
if ev & EV_ACT != 0 {
    self.status = format!("{} %", self.view.zoom_pct());   // zoom or position changed
}
if ui.button(ID_FIT, "Einpassen") & EV_ACT != 0 { self.view.set_fit(); }
if ui.button(ID_ONE, "100 %") & EV_ACT != 0 { self.view.set_actual(); }
```

* `zoom` 1.0 = 100 %: a bitmap pixel on one screen pixel, an SVG at its intrinsic size in
  DIP; `cx`, `cy` = the image point at the view's centre; `fit` = the zoom follows the view.
  `zoom_to([x, y, w, h])` shows an image rectangle, `zoom_by(f)` multiplies, `label` names the
  view for screen readers (default: the SVG's title or "Bild"; the record's value is the zoom
  in percent). Another image (another `Rc`) starts in fit mode again.
* Mouse: the wheel zooms at the pointer (the image point under it stays), Shift + wheel
  scrolls sideways; a left or middle drag pans; Ctrl + drag zooms into a rectangle; a double
  click switches between 100 % and fit; the scroll bars at the edges move the view. Keys:
  arrows, PgUp / PgDn, Home / End pan; + / - zoom in steps (1 % .. 6400 %); Ctrl+0 fits;
  Ctrl+1 shows 100 %. Flags: `IV_WHEEL_SCROLL` (the wheel scrolls, Ctrl + wheel zooms; for
  views inside scrolled pages), `IV_SMOOTH` (no visible pixels), `IV_NO_GRID`, `IV_NO_BARS`,
  `IV_NO_CHECKER`, `IV_TEXT_COLOR` (an SVG's currentColor follows the theme).
* Drawing: bitmaps below 50 % through mip levels (2 x 2 box), bilinear up to 400 %, nearest
  neighbour above, a pixel grid from 800 %; transparency over a checkerboard (plain background
  in high contrast).
* Tests: `apps/svgtest` (`--test unit`: 321 checks of parser, path data, transforms,
  gradients, text layout (with a font engine of boxes), clipping, CSS, the codec and malformed
  files; `gallery.uis`: every fixture of `apps/svgtest/fixtures` light / dark / high contrast /
  200 %; `viewer.uis`, `viewer_modes.uis`: zoom and pan by script, all with `--verify-dirty`).

## 32. Markdown / HTML viewer and lists with rows of different heights

**The document view** shows Markdown or HTML read-only: help pages, README-like content,
release notes, chat output. [`MdDoc`](../lib/ui/md_doc.rs) is its model -- blocks of formatted
text -- read from Markdown (CommonMark with GitHub's tables, task lists, strikethrough and bare
links) or from HTML (the reader of the rich-text editor, extended by rules, images,
preformatted code and tables). [`ui.markdown_view` / `ui.html_view`](../lib/ui/md_view.rs)
draw it; the app keeps an `MdViewState` (layouts, images, scroll position, selection).

```rust
// state: self.help = MdViewState::new(); self.help.base = String::from("app://help/");
// per frame: the text is parsed again only when it changes (the state keeps a hash of it)
ui.next_size(-1, -1);
let ev = ui.markdown_view(ID_HELP, &mut self.help, &self.page, 0);
if let Some(url) = ev.link {                 // a click on a link, Enter on the focused one
    if url.starts_with("#") { self.go_to(&url) } else { ui.open_url(&url); }
}
self.status = ev.hover.unwrap_or_default();   // the link under the pointer
// HTML instead: ui.html_view(ID_NOTES, &mut self.notes, &html, MV_NO_FRAME)
// a document built once: st.set_markdown(s) / set_html(s) / set_doc(MdDoc::markdown(s)),
// then ui.doc_view(id, &mut st, flags)
```

* Markdown: ATX and setext headings, paragraphs (lazy continuation lines), block quotes,
  bullet and ordered lists (start numbers, tight / loose spacing, `[ ]` / `[x]` task items),
  fenced and indented code, thematic breaks, tables with alignment (`\|` in cells), HTML blocks
  (through the HTML reader), link reference definitions; emphasis by CommonMark's delimiter
  rules, `~~strike~~`, code spans, inline / reference / auto links and bare `https://` and
  `www.` addresses, images, hard and soft breaks, entities, backslash escapes, a little inline
  HTML (`br`, `b`, `i`, `s`, `u`, `code`, `kbd`, `a href`, `img`). HTML: what
  `RichDoc::from_html` reads (headings h1 .. h6, paragraphs, lists, quotes, formats, colours,
  links) and `hr`, `img` (width / height), `pre` / `code class="language-x"`, tables (`th`,
  `thead`, alignment).
* Layout: the blocks in a column; quotes indented with a bar, list markers in front (bullets by
  depth, numbers, task boxes), headings 2 / 1.5 / 1.25 / 1 / 0.875 / 0.85 times the body text
  (h1 / h2 underlined), code blocks in the monospace font with the code editor's highlighting
  (`Highlighter::for_name` of the fence's language, `ui.code_theme()` colours), tables with
  column widths from their cells (wrapped to fit), images at their size in DIP (scaled down to
  the column; SVGs through `lib/img_svg`, sharp at every size). Text wraps at
  `text::segment`'s line breaks. A paragraph's direction comes from its first strong
  character: Hebrew and Arabic run right to left, shaped and reordered by `text_shape` (with
  the fallback font that has the script); the document's direction (indents, markers) is that
  of its first strong character unless `MV_RTL` / `MV_LTR` says otherwise -- a German page
  stays left to right in a mirrored window, a Hebrew one runs right to left in any window.
* Long documents: only the blocks in view are laid out (cached per width, dpi, text scale and
  fonts) and drawn; scrolling is anchored at the first visible block, so blocks laid out for
  the first time never move what is shown. 50 000 blocks scroll in about a millisecond per
  frame. `scroll_to_end()` (and scrolling to the end of a long document) keeps the end in view
  while the document grows -- chat output.
* Mouse: drag selects (the view scrolls at its edges), Shift + press extends, a double click
  selects a word, a click on a link activates it. Keys: Up / Down scroll by a line, PgUp / PgDn
  and Space / Shift+Space by a view, Home / End, Tab / Shift+Tab through the links (the focused
  one is framed and scrolled into view; past the last one the focus moves on), Enter activates
  the focused link, Esc drops it, Ctrl+A, Ctrl+C / Ctrl+Insert copy text (list markers, cells by
  tabs, images as their alt texts) and HTML. `MdViewState` also gives `selection()`,
  `selected_text()`, `selected_html()`, `select_all()`, `scroll_to_block(i)`.
* Links are reported, not opened: `MdEv::link` carries the target; with `MV_OPEN_LINKS` the view
  also hands it to `open_url` (whose scheme rules and the app's `url` permission apply) -- only
  after the user's click or Enter, never by itself.
* Images: names of the VFS (`app://`, `data://` ...) and paths -- relative ones after `st.base`
  -- are read when first shown and kept in the state; `http(s)` and `data:` addresses are never
  fetched (the alt text shows in a frame instead).
* Accessibility: the view is a `K_BOX` (label `st.label`, default "Dokument"; value: the scroll
  position in DIP); per visible block a `K_HEADING` record (new kind; value: the level),
  `K_TEXT` (paragraphs, code, table cells), `K_LISTITEM` (value: the item's number),
  `K_CHECKBOX` (a task's box, checked), `K_LINK` per link, `K_IMAGE` (label: the alt text).
  Scripts select them as `heading:"Title"`, `link:"Docs"` ... Screen readers get the headings
  as headings with their level on every platform (section 19): heading navigation jumps
  through the document's visible blocks.
* Robustness: malformed input never panics and takes linear time -- container nesting is capped
  at 32 (the HTML reader's at 64 open elements), the inline parser has a scan budget for link
  destinations and tags, code spans find their closing run through an index, nested images
  collapse as they form; 16 MB of input at most.

**Lists with rows of different heights** (`ui.vlist_begin`, [`vlist.rs`](../lib/ui/vlist.rs))
virtualize like `list_begin`, but every row has a height of its own: the app emits the rows the
list asks for, each in a box whose natural height is measured.

```rust
// state: self.vl = VListState::new();
ui.next_size(-1, -1);
let (first, end) = ui.vlist_begin(ID_LOG, &mut self.vl, self.msgs.len());
for i in first..end {
    ui.vlist_item_begin(i);                      // the row's box (BOX_V, the list's width)
    ui.text(TS_BODY_STRONG, &self.msgs[i].from);
    ui.text(TS_BODY | TX_WRAP, &self.msgs[i].text);
    ui.vlist_item_end();                         // measured: the rows below follow it
}
let ev = ui.vlist_end(&mut self.vl);            // EV_ACT: selection, EV_DOUBLE: Enter / double click
// older messages arrived at the top: tell the list, the view stays where it is
self.msgs.splice(0..0, older);
self.vl.insert(0, n);
```

* Heights: measured as the rows are built (a changed row is measured again the next time it is
  built; `invalidate(i)` / `invalidate_all()` force it), or given by the app with
  `set_height(i, dip)`. Rows not measured yet count with `estimate` (DIP; by default the mean
  of the measured rows). The heights sit in Fenwick trees: positions and the row at a position
  cost O(log n), 100 000 rows scroll in about a millisecond per frame.
* Scrolling is anchored at the first visible row: rows above it may change their height, be
  inserted (`insert(at, n)`) or removed (`remove(at, n)`) without moving what is shown; the
  selection stays on its row. `reveal(i)` scrolls a row fully into view (exactly, once the
  rows on the way are measured), `scroll_to(i)` shows it at the top, `first_visible()`,
  `scroll_pos()` / `set_scroll_pos(px)`, `total_height()`.
* Keys: Up / Down, PgUp / PgDn (a view of rows), Home / End move the selection and reveal it;
  Enter reports `EV_DOUBLE`. The wheel scrolls smoothly, the scroll bar drags. Records:
  `K_LIST` (value: the selection, max: the count), `K_LISTITEM` per row (`id + 1 + i`).
* The document view scrolls its blocks with the same height cache (`Rows`).

Tests: `apps/mdviewtest` (`--test unit`: the parser -- blocks, inline parts, HTML, text and HTML
of ranges -- adversarial input with time limits, the list's height cache and the headings in
the accessibility tree;
`doc_shots.uis` light / dark / high contrast / HTML / Hebrew, `doc_rtl.uis` (mirrored window),
`doc_scale.uis` (150 % text), `doc_links.uis`, `doc_select.uis`, `doc_keys.uis` (also 50 000
blocks), `vlist.uis` (100 000 rows); all with `--verify-dirty`).

## 33. Sound: streams, short sounds, the system's sounds

[`plat::audio`](../lib/plat/audio.rs) plays sound on all four targets: **streams** the app fills
(a synthesizer, a decoder, a tone generator), **short sounds** from memory or files mixed
together (`play`, `play_file`), and **the system's own sounds** for events (`sound`). The UI
side is `ui.beep(kind)` and `ui.dialog_sound(kind)` ([`ui/sound.rs`](../lib/ui/sound.rs)).

```rust
use plat::audio::{self, Config, Stream};

// a stream that calls back for its samples (f32, interleaved, -1 ..= 1): a 440 Hz sine
let cfg = Config { rate: 48_000, channels: 2, latency_ms: 30, name: "Ton" };
let mut phase = 0.0f64;
let tone = Stream::open_fill(&cfg, move |buf: &mut [f32]| {
    for frame in buf.chunks_exact_mut(2) {
        let s = (0.2 * rt::x87::sin(phase)) as f32;  // (a table is cheaper)
        frame[0] = s;
        frame[1] = s;
        phase += core::f64::consts::TAU * 440.0 / 48_000.0;
    }
    true                                          // false: this was the last buffer
})?;
tone.set_volume(0.5);                             // ramped over one buffer, no click
tone.pause(); tone.start();                       // the queued frames stay while paused
drop(tone);                                       // closes the device stream

// a stream the app writes into (from a decoder, a timer): write takes what fits
let mut s = Stream::open(&Config::new(44_100, 2))?;   // 40 ms, the app's name
let n = s.write(&samples);                        // whole frames taken; s.writable() before
s.write_i16(&pcm16);                              // 16-bit samples, converted
s.finish();                                       // State::Ended once the ring is played

// short sounds: mixed into one shared stream, the file decoded once per call
audio::play_file("app://klick.wav")?;             // WAV or Ogg Vorbis through the VFS
let chime = audio::Sound::load("app://glocke.oga")?;   // keep it, play it often
let v = audio::play(&chime)?;                     // v.stop(), v.playing(), v.set_volume(0.3)

// the system's sound for an event (the user's sound scheme / theme)
ui.beep(audio::SOUND_WARNING);
if ui.dialog_begin(ID_ASK, "Datei überschreiben?", &open) {
    ui.dialog_sound(audio::SOUND_QUESTION);       // once, as it opens -- where dialogs do that
    // ...
}
```

* **One model: the ring.** A stream has a ring the app writes into and the platform's audio
  thread drains; `open_fill` is the same stream without a ring, its `fill` called on that
  thread. This works on all four targets alike: the browser's sound runs in an AudioWorklet
  that cannot call into the app's WASM instance, so it is fed from a buffer anyway, and the
  ring keeps the app's code (allocation, locks, the UI's pace) off the real-time thread.
  `fill` runs on Windows / Linux / Android on the audio thread and in the browser in the
  worker's pump (every 10 ms): it must not block. `write` never blocks either -- it takes what
  fits (`writable()`); a stream that runs dry plays silence and counts an underrun
  (`underruns()`), not when the app said `finish()`.
* **Latency**: `Config::latency_ms` (default 40, 10 ..= 1000) is split between the device's
  buffer (half, 10 ..= 100 ms) and the ring (the rest); a fill stream gives all of it to the
  device. `latency_ms()` is what is queued now plus the device's own latency (WASAPI's stream
  latency, PulseAudio's sink latency, the AudioContext's base and output latency). An app that
  writes from UI timers (16 ms frames) wants 50 ms or more; a fill stream reaches 20 ms.
  `played()` counts the frames heard so far.
* **States**: `Starting` (the browser host has not answered yet), `Playing`, `Paused`,
  `Stopped` (`stop()`: the ring and the device's buffer emptied), `Ended` (after `finish()` or
  a fill that returned false; the device stream stops, `start()` or another `write` plays
  again), `Blocked` (the browser waits for a user gesture), `Lost` (the device went away and no
  other is there; the stream plays again when one appears). No device or sound service at all:
  `open` gives `AudioError::Unavailable`; `available()` asks beforehand.
* **Formats**: the device side is always f32 at the stream's rate and channels (WAVE order:
  1 mono, 2 left / right, 6 = 5.1 as FL FR FC LFE BL BR); the system converts rate and
  channels where needed, the browser's context rate is converted here (windowed sinc).
  `Sound::wav` reads RIFF WAVE with PCM 8 / 16 / 24 / 32 bits and float 32 / 64 (also
  extensible), `Sound::ogg` Ogg Vorbis (floor 1, residues 0 / 1 / 2, coupling; matches
  libvorbis to 5e-5), `Sound::decode` tells them apart, `Sound::tone(freq, ms)` makes a sine,
  `to_wav()` writes 16-bit PCM. A `Sound` holds everything decoded (10 minutes at most):
  music belongs in a stream.
* **`play`** mixes into one stream of 48 kHz stereo that opens with the first sound and ends
  after 2 s of silence (an idle open stream would keep the computer from sleeping); each sound
  is converted to 48 kHz while it plays. Dropping the `Voice` lets the sound play on.

| | Streams | Default device | System sounds |
|---|---|---|---|
| Windows | WASAPI shared mode, event-driven, a thread per stream ("Audio" MMCSS class), f32 with `AUTOCONVERTPCM` | `IMMNotificationClient`: a new default device or an unplugged one reopens the stream there (`device_changes()`); `device()` is its friendly name | `PlaySound` aliases of the sound scheme: `SystemAsterisk`, `SystemExclamation`, `SystemHand`, `SystemQuestion`, `.Default`, `Notification.Default` |
| Linux | the PulseAudio native protocol over its Unix socket (`$PULSE_SERVER`, `$XDG_RUNTIME_DIR/pulse/native`): pipewire-pulse on current desktops, PulseAudio, WSLg's server; a connection and thread per stream, the cookie for auth | the server moves the stream to a new default sink itself (`device()`: the sink's name, `device_changes()` counts the moves); a lost server: `Lost`, reconnected every second | the freedesktop sound theme (`dialog-information`, `-warning`, `-error`, `-question`, `bell`, `message-new-instant`; `.oga` / `.ogg` / `.wav`, `Inherits=`, `.disabled`), the theme and the switch of `org.gnome.desktop.sound` (settings portal); no file: a generated tone |
| Browser | WebAudio: an AudioWorklet per stream fed from a SharedArrayBuffer ring (cross-origin isolated pages) or posted buffers ([`web/audio.js`](../web/audio.js)) | the browser follows the system's output itself | generated tones |
| Android | AAudio (shared, low latency) through the host ([`android/audio.c`](../android/audio.c)), its callback renders | a disconnected device reopens the stream on the new default | the notification sound (`SOUND_NOTIFY`), `ToneGenerator` beeps on the system stream |

* **The browser** keeps an AudioContext suspended until the page had a user gesture: a stream
  opened before shows `State::Blocked`; the first pointer or key event resumes it (the host
  listens for them). Background tabs throttle the worker's timers less than the page's, but a
  long busy frame of the app delays the pump: give such apps a larger latency.
  `window.jfguiAudio` (read-only) gives tests the context and the nodes.
* **Sounds off**: `ui.set_sounds(false)` (an app setting) or `jfgui_SOUNDS=0` silence
  `ui.beep` and `ui.dialog_sound`; the systems' own switches apply too (Windows' "No sounds"
  scheme; Linux's `event-sounds`, also for `plat::audio::sound`; Android's system volume).
  `dialog_sound` plays only where message dialogs play one (`audio::dialog_sounds()`: Windows,
  Linux desktops; not Android, not the browser). Overrides: `jfgui_SOUND_THEME`,
  `jfgui_EVENT_SOUNDS=0|1`; `jfgui_AUDIO_DRYRUN=1` (or `audio::set_dry_run`) prints
  `sound: NAME` instead of playing.
* **Scripts**: headless runs play nothing; `expect sound KIND|none` checks what `ui.beep` /
  `ui.dialog_sound` asked for since the last check (section 12).
* **Permissions**: an app with a `PERMISSIONS` manifest needs the line `audio.output` for any
  sound ([PERMISSIONS.md](PERMISSIONS.md)). Without it `Stream::open`, `open_fill`, `play` and
  `play_file` give `AudioError::Denied`, `sound()` and `ui.beep` return `false` (a script sees
  `expect sound none`), `available()` is `false`, and stderr shows `PERMDENY audio.output` once.
  `ui.dialog_sound` stays silent without a question or a record, also under
  `ask audio.output` until that line is allowed. Only apps that play sound list the right: a
  Flatpak package (`tools/package.py`) reaches the sound server through
  `--socket=pulseaudio`, which also allows recording, and gets it only with `audio.output`;
  MSIX, the setup, .deb, .rpm, AppImage and Android need nothing for playing sound.
* Not there yet: output without a sound server on Linux (raw ALSA), choosing an output device,
  the stream's name in Windows' volume mixer (it names the program), capture (microphone),
  video.

Tests: `apps/audiotest` -- the demo (tone generator, the chime as WAV and Ogg Vorbis, the
system sounds, a dialog with its sound) and `--test play` (push and fill streams, pause / stop,
volume, a device change, the mixer; Windows: WASAPI loopback must hear the tones at their
frequency), `--test sounds`, `--test pulse` (Linux: the server lists the stream, corks it,
reports its latency; WSLg's server), `tests/audio_web.mjs` (Chromium and Firefox: an
AnalyserNode hears the tone through the ring and through posted buffers, `Blocked` until a
click), `tests/android_audio.py` (an emulator: audioserver plays the app's AAudio track, the
self-test passes). `plattest --test audio`: WAV formats and damaged files, the ring, the
resampler, Ogg Vorbis files against their sines, the PulseAudio wire format, the sound theme.

## 34. App themes and the style at run time

An app's own colours, corners, spacing, control heights and type do not need a style crate: an
**app theme** ([`ui/theme_app.rs`](../lib/ui/theme_app.rs)) lays them over the active style,
from a small INI file or built in code, and takes effect on the next frame. An app that links
several styles switches between them at run time. [STYLES.md](STYLES.md) has the whole format
and how the styles read it.

```rust
use ui::theme::{Radius, Theme, THEME_DARK, THEME_LIGHT};

// a theme file of the app (apps/NAME/assets/themes/...) or of the user (config://)
match ui.load_theme("app://themes/ocean.theme") {
    Ok(()) => {}
    Err(e) => self.error = e.to_string(),   // "Zeile 5: „#12345“ ist keine Farbe (#RRGGBB oder #AARRGGBB)"
}
// an editor: start from the active theme, change a token, apply it live, save it
let mut th = ui.app_theme().cloned().unwrap_or_default();
th.name = String::from("Eigenes");
th.accent[THEME_LIGHT] = Some(0x6B4FBB);                 // the style derives its accent ramp
th.colors[THEME_DARK][(T_BG / 4) as usize] = Some(0xFF17_2029);
th.radius = Some(Radius::Dip(8));
th.spacing = 110;                                        // percent: box padding and gaps
th.control_height = 36;                                  // DIP: buttons, text / combo boxes
th.text[TS_BODY as usize].size_fx = Some(15 << 8);
ui.set_theme(&th)?;                                      // a font that cannot be loaded: Err, theme stays
th.save("config://themes/eigenes.theme");                // the text of a theme file (to_ini)
ui.clear_theme();                                        // the style's own look again

// several styles: build_rs.py links every style_NAME the sources name
ui.styles_add(&[&style_winui::STYLE, &style_classic::STYLE, &style_kde::STYLE]);
let names = ui.style_names();                            // the app's own style first
ui.set_style("kde");                                     // tokens, fonts, metrics, painters
```

```ini
# app://themes/ocean.theme (apps/themedemo/assets/themes)
[theme]
name = Ozean
[light]
accent = #0F7B8A
bg = #EAF3F5
text = #0B2A33
[dark]
accent = #4FC3D1
bg = #0E1E24
[metrics]
radius = 8
spacing = 110%
control_height = 36
[text]
body.size = 15
subtitle.weight = bold
```

* **What a theme sets**: every `T_*` colour token for light, dark (`[colors]` both) and high
  contrast, the accent (through the style's ramp; it wins over the app's and the system's),
  rounded corners that are not pills (`radius = 8` DIP or `150%`), box padding and gaps
  (`spacing`), the height of standard buttons, text boxes, combo boxes, drop-down and split
  buttons (`control_height`; it grows with a text scale above 100 %), size, line height and
  weight of each text style and a font family (a font file next to the theme, a VFS path, or an
  installed family). Light / dark / system and high contrast stay the app's and the user's
  choice: the theme has colours for each.
* **Errors** name the line and come in the UI language (`ThemeError { line, msg }`); a broken
  file or a missing font leaves the previous theme as it was. Without a theme the rendering is
  the style's to the pixel.
* **Limits**: a style's own fixed values stay (classic's grey 3D face and bevels come from its
  palette, not from tokens; title bar fonts; paddings inside a widget). The style switch
  replaces the embedded fonts (`FONT_REG` .. `FONT_ICON`) for the whole process; fonts the app
  loaded itself keep their ids. `build_rs.py` passes the extra style crates on every target;
  `apps/themedemo` is built and tested on Windows and Linux.
* **Command line and scripts**: `--theme-file PATH` (after `App::init`); scripts `theme file
  PATH`, `theme none`, `style NAME`, `expect theme "NAME"|none`, `expect theme error "TEXT"`,
  `expect style NAME`, `expect token NAME #RRGGBB`, `expect pixel X Y #RRGGBB`, `expect same
  FILE.png` (the same pixels as an earlier shot), `rect * * * 36` (§12).

Tests: `apps/themedemo` -- the page "Darstellung" (style, light / dark / system, high contrast,
the two sample themes, a live editor for accent, background, corners, spacing, control height,
text size and a system font, saving to `config://themes/eigenes.theme` and loading a theme
file by its path) with a preview of the common widgets; `tests/themes.uis` (tokens, pixels and
rectangles of both themes in light, dark and high contrast, back to the same pixels, all four
linked styles with and without a theme, broken files, the editor, save and load),
`tests/text150.uis` (at 150 % text). `apps/gallery/tests/themes.uis` and `themes150.uis` (the
sample themes and a font family from files over the gallery; they hold in the builds of every
style), `uitest --only themes` (the format, the errors with their lines, the text form,
`Ini::items`).
