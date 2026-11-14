# Changelog of jflib-ui

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- `ui.drop_defer()` and `ui.drop_finish(effect)`: a drop target that learns only later whether
  it took a drop (a web page's drop handler in another process) tells the drag's source then --
  the effect, or that it was not taken -- at most `DROP_DEFER_MS` (3 s) after the drop. X11 and
  Wayland carry the late answer, Windows for a source whose data object is in async mode; on
  macOS and in the browser `drop_defer` says the answer went at the drop.
- `ui.drag_allowed()`: the effects the source of the drag over a window allows; after a drop
  those of its source, from its `E_DROP` to the end of its passes and, after `drop_defer`, until
  its answer went.
- Script commands `drop expect copy|move|link|none|pending`, `drop ... sync` and
  `drop ... allows copy|move|link` (the effects a drop's source allows, given with the drop).

### Fixed

- The effects another program's drag allows are forgotten when it leaves the window, is dropped
  or the app's own drag ends, and a drop takes those the platform gives with it: a drop without
  a hover before it no longer has the effects of the drag before it.
- A drop has an effect its source allows: the first of the target's effects that the source
  allows when the platform's is not one the target takes (it was the target's first effect, so a
  copy-only drop onto a move target was delivered as a move); with none in common the drop is
  refused (no `DS_DROP`, no `UIC_DROP`, the source is answered "not taken").
- Every window keeps its own state slots (`ui.slot`): a box or view of the same id in two
  windows, such as the title bar's box in two app windows, no longer flips between their sizes
  ("layout did not settle within 4 passes").
- Script selectors and `expect S text` match a record's whole label, also a wrapped paragraph of
  more than 255 bytes; `dump` writes labels up to 64 KiB.

## [0.2.0] - 2026-11-13

### Added

- macOS on Apple silicon: apps run in windows of their own through AppKit, sharp at the Retina
  scale, with the library's title bar or the system's. Command acts as Ctrl for shortcuts, the
  trackpad scrolls finely and its pinch zooms like Ctrl + wheel, input methods compose into text
  boxes, and `ui.filedlg` shows the system's Open and Save panels. Cmd+Q closes the main window
  the way its close button does, and the Dock icon brings it back. The tray (`ui.tray_set`),
  notifications, the Dock tile's badge, progress and tasks (`ui.taskbar_*`), global shortcuts,
  recent documents, files and URLs the system opens with the app (`UIC_OPEN`) and spell checking
  of `TB_SPELL` boxes go through the same calls as on Windows and Linux. The source inspector
  opens the editor on macOS too.
- On macOS the app's menus live in the system's menu bar: `ui.menubar(None)` of the main window
  publishes the menus of the `AppDesc` there, with the commands' labels, states and accelerators
  (Command for `MOD_CTRL`), and draws no row in the window. A choice up there runs as from the
  window's bar. Headless runs keep the bar in the window, and so does `jfgui_SYSMENU=0`.
- iOS: `ui::launch` hands the app to a UIKit host. It gets one scene inside the safe area and
  above the keyboard, fingers, the Apple Pencil as a pen, an iPad's pointer as a mouse, the
  on-screen keyboard with a Done bar above it and a hardware keyboard, rotation, and a display
  link that rests while the app is idle. The system's appearance, reduced motion and darker
  colours count as system settings; the clipboard, the document picker for Open, Save as and
  folders (answering later, as on Android), notifications, the motion sensors, the location and
  spell checking work as on the other systems. Headless runs (`--shot`, `--script`) stay as on
  the desktop.
- VoiceOver on macOS and iOS reads every widget. On macOS each record is an accessibility
  element of its window, with its role, value, state and actions, and text boxes with their
  text, caret and selection; dialog windows are dialogs. On iOS lists, tab strips, dialogs and
  menus are containers, double tap, swipes on sliders and the escape gesture work, VoiceOver
  moves through the text of text boxes, and the keyboard sees the text around the caret.
- The UI can draw its windows through Metal on macOS and iOS (`jfgui_GPU=1`; off by default
  there), and views of GPU content work on both.
- Phones and tablets: `ui.form_factor()` (`FF_DESKTOP`, `FF_TABLET`, `FF_PHONE`) and
  `ui.mobile()`; `jfgui_FORM` decides instead, for tests. On a phone or a tablet only, the title
  bar has no caption buttons, a menu bar that does not fit becomes one menu button (fill widgets
  of its row keep 64 DIP), the status bar and vertical scroll views roll sideways while their
  content is too wide, and windows and content dialogs stay inside the screen and roll their
  content. Android paints its status and navigation bars in the colours at the app's edges. A
  desktop window stays as it was however narrow.
- Views of GPU content in a window: `ui.gpu_view(id, &content, ver)` shows what a `GpuContent`
  draws on the UI's own device, a 3D scene of jflib-render3d for example, between the widgets
  and under popups and tooltips. Where the window is drawn through its swapchain the output's
  render pass draws the view; elsewhere its frame is read back pixel for pixel, and without a
  device the content paints itself. A lost device detaches the content and attaches it to the
  next one. A view's frames follow what the display shows, so a view drawn as fast as the app
  can no longer starves the compositor on the same GPU. `AF_GPU_3D` asks for the fastest adapter
  at level Voll; `GPU_VIEW_FORMAT`, `Ui::gpu_view_info` with `GpuViewInfo` (`read_frame`,
  `held`), `GpuOutput::view_quads` and the test aid `Ui::gpu_test_hold` are there for measuring
  and testing. `jfgui_GPU_OUTPUT=0` keeps the windows on the CPU path, `jfgui_GPU_VIEWS=0`
  reads every frame back.
- Canvases that take the keyboard (a web view): with the hit box flag `HB_KEYS` the focused
  canvas keeps every key, Ctrl + letter, Tab, F6 and Ctrl+Z included; `ui.key_take()` takes the
  next key and `ui.accel_run(key)` hands one back to the command table. `HB_CURSOR_SET` takes a
  canvas's cursor bits as given.
- Raw key events: `ui.key_event_take()` gives each key as it went down or up as a `KeyEvent`
  (key, modifiers, auto-repeat, scan code, location `KL_STANDARD`, `KL_LEFT`, `KL_RIGHT`,
  `KL_NUMPAD`); Windows, X11 and Wayland report scan code and location.
- An input method for fields the app draws itself: `ui.ime_enable(on, purpose)` with `IP_TEXT`,
  `IP_URL`, `IP_EMAIL`, `IP_NUMBER`, `IP_PHONE`, `IP_PASSWORD` and `IP_SEARCH`, and
  `ui.ime_preedit()` for the composition to draw at the caret. On Windows the input method's own
  composition window stays hidden while the app draws it, in the code and rich-text editors too.
- More mouse cursors, the shapes of CSS: `CUR_NOT_ALLOWED`, `CUR_NO_DROP`, `CUR_GRAB`,
  `CUR_GRABBING`, `CUR_HELP`, `CUR_PROGRESS`, `CUR_ZOOM_IN`, `CUR_ZOOM_OUT`, `CUR_CELL`,
  `CUR_VTEXT`, `CUR_ALIAS`, `CUR_COPY`, `CUR_CONTEXT_MENU`, `CUR_ALL_SCROLL`, `CUR_COL_RESIZE`,
  `CUR_ROW_RESIZE` and one per edge and corner (`CUR_SIZE_N` .. `CUR_SIZE_SW`), on Windows, X11,
  Wayland, macOS and in the browser. Shapes a system lacks are drawn by the library at the
  system's cursor size, elsewhere the nearest shape stands in. `ui.cursor_next(shape)` gives the
  next hit box any shape, `ui.cursor_at(x, y)` tells the cursor at a point, `ui.cursor_replace`
  and `ui.cursor_free` manage custom cursors.
- Popups at a point the app learns later (a web page's context menu):
  `ui.popup_open_at(id, x, y)` opens a flyout of the app's own there,
  `ui.menu_popup_at(menu, x, y)` a menu. While one is open a canvas with `HB_KEYS` leaves the
  keys to it. `ui.popup_close_id(id)` closes one popup
  and those opened inside it, the flyouts around it stay open.
- Records for content an app draws inside a canvas: `ui.sem_rect(kind, id, rect, label, value)`
  publishes a link or heading of a web page without a hit box of its own. Scripts find it by its
  label, a click on it reaches the canvas at that place, and screen readers get it with its
  kind's role. `ui.canvas_label(label)` names the canvas's own record.
- A web page's accessibility tree: the record kinds `K_TABLE`, `K_ROW`, `K_CELL`,
  `K_COLHEADER`, `K_ROWHEADER`, `K_LANDMARK`, `K_TREE`, `K_TREEITEM`, `K_RADIOGROUP`,
  `K_TOOLBAR`, `K_TOOLTIP`, `K_SEPARATOR`, `K_ALERT`, `K_STATUS`, `K_LOG`, `K_METER`, `K_SPIN`,
  `K_SCROLLBAR`, `K_ARTICLE` and `K_FIGURE`; setters on the returned record for an id of its
  own, a parent, a description, a link's address, states, the place in a set, a level, table
  cells, live regions, ranges and a text selection (`Sem::ax`, `SemAx`); records out of view
  (`AS_OFFSCREEN`). `ui.a11y_request_take()` hands over what a screen reader asks of such
  records (`A11yRequest`, `A11yAction`), `ui.announce` has it say a text, and `ui.a11y_active()`
  tells whether one listens. The rich records reach UI Automation, AT-SPI, TalkBack and the
  browser's ARIA tree.
- `UIC_A11Y` tells the app when a screen reader or another accessibility client comes (arg 1)
  or leaves (arg 0), also while the app is idle. `A11yRequest::new` builds requests for an app's
  own tests.
- Several files at once in the file dialog: `FD_MULTI` (with `FD_OPEN`) lets Windows' dialog,
  Linux's portal and the library's own dialog choose several files, and `ui.filedlg_paths()`
  gives all of them. The pickers of macOS, iOS, Android and the browser choose one for now.
- `ui.calendar(id, &mut y, &mut m, &mut d, &range)`: the month calendar of `date_picker` in
  place, for a panel or a flyout. `DateRange::new(mode, min, max)` chooses days (`CAL_DAY`),
  ISO weeks with their numbers (`CAL_WEEK`) or months (`CAL_MONTH`); dates outside the range are
  disabled.
- Independent app windows: `WF_APP` gives a window of its own, with its own taskbar button and
  not minimized or hidden with the main window, on Windows, X11 and Wayland. It has no library
  title bar; the app draws one with `titlebar_begin` / `titlebar_end`. macOS gives it an owned
  window for now.
- Dock groups that move as a whole: a panel with `DP_GROUP` takes its whole group along when it
  docks to another side or floats, from the group menu or a double click on the strip;
  `DockState::dock_group_to(id, side)` and `float_group(id, rect)` do the same from the app's
  own commands.
- `ui.drop_effect_set(effects)`: the `DE_*` effects a drop target takes in this pass (0 refuses
  the drag there). The system's pointer shows copy, move, link or no accordingly, on Windows,
  X11, Wayland and macOS. `ui.drop_pos_px()` and `ui.drag_over_px()` give a drop's and a drag's
  place in exact pixels.
- `ui.code_reveal(id, line)` scrolls the code editor to a line, opening its folds, without
  moving the keyboard focus or the caret (a debugger's current line).
- Syntax highlighting of the web's languages: `Highlighter::javascript()`, `css()`, `html()`
  (also XML and SVG, with `<script>` as JavaScript and `<style>` as CSS) and `wat()`, the
  grammars `JAVASCRIPT`, `CSS`, `HTML` and `WAT`, and their extensions in
  `Highlighter::for_name`. `Rule::Nest` and `Rule::Element` let a grammar of one's own embed
  another language.
- `ui.default_browser()` and `ui.default_browser_request()` for an app that registers as the
  system's web browser (`PACKAGE` `browser=1`).
- `StyleList::scrollbar_room`: a style whose scroll bars stay visible keeps their room free
  beside the content of a view that scrolls only vertically, whether it overflows or not.
- The script language: `click` takes the modifier words `shift`, `ctrl` and `alt`; `keydown`
  and `keyup` (with `scan N`, `left`, `right`, `numpad`); `preedit`; `wait real MS` lets real
  time pass while the app's own threads work; `sync MS soft` does not fail when MS runs out;
  `system env jfgui_FONT_FALLBACK 1` (or `-`) turns the system fonts for missing characters on
  or off from that point; `a11y` (actions, expectations, a dump of the tree) and `expect S
  unfocused`, `unselected`, `closed`; `dragover expect copy|move|link|none`; `system browser
  default|other|unknown`, followed by the app becoming inactive and active again as after a
  real choice.
- The feature `unwind` (with `std`): an app built with `panic = "unwind"` keeps running after a
  panic it catches with `std::panic::catch_unwind`; `ui::main!` no longer asks for
  `panic = "abort"` then.
- `ui::test_main!()`: the stub the unit tests of a library crate with std need to link, such as
  an app's own style crate (`#[cfg(test)] ui::test_main!();` in its `lib.rs`).

### Changed

- A `TB_PASSWORD` box sets the input method's purpose itself while it has the keyboard: Windows
  takes the input method away, X11 keeps the keys from IBus, Wayland tells the compositor.
- `markdown_view` and `html_view` publish a table as `K_TABLE` with `K_ROW`, `K_CELL` and
  `K_COLHEADER` records, each cell with its row and column, instead of one `K_TEXT` per cell.
- The accessibility tree is not built or published again when a frame's records, windows and
  carets are those of the tree published last: 3,400 rich records cost 0.13 ms per frame then
  instead of 2.2 to 4.8 ms. The script command `a11y dump` prints descriptions, addresses,
  values with their range and the value texts of sliders and meters.
- On Windows the UI stops building the accessibility tree 10 s after the last question of a UI
  Automation client, unless a screen reader or Magnifier runs. Before, any client, the touch
  keyboard's host too, kept it on for the whole run.
- Large frames cost less: the UI hands the worker pool to gfx (`gfx::set_parallel`), which blits
  a canvas from 1 MB on in row bands, so a 4K frame takes about 2 ms on the UI thread instead of
  4.5 to 6 ms, and the present step copies large frames in bands as well.
- The built-in catalogue has no row "Neuer Ordner" any more; `ui::i18n::t` falls back to the
  text itself.
- `GpuOutput` is `#[non_exhaustive]`: apps read its fields but no longer build it with a struct literal.

### Fixed

- Text and number boxes with the same id in two windows no longer share their caret, selection
  and undo.
- A frame is no longer lost when the last command round of a pass changes something; an app
  that asks for a frame after every frame gets 8 more, then a warning.
- A fill child of a new box gets the room left after the children before it in its first pass
  instead of 0, so nested new boxes no longer end in "layout did not settle within 4 passes".
- A hidden dock panel comes back between the neighbours it had in its group, also after saving
  and loading the layout.
- `date_picker` inside a flyout of the app's own closes only its calendar when a day is picked.
- After the library's own folder dialog `ui.filedlg_paths()` holds the chosen folder, and the
  folder is granted to the app as the user's choice.
- A toast's message has a record of its own: screen readers read it and scripts find it.
- Code editor: undo and redo of "replace all" keep the selection inside the text, moving the
  last lines down leaves the selection on the last line, and toggling a comment no longer cuts
  a line indented with a no-break or ideographic space inside the character.
- No more panics on a finger tap near a hit box that is empty or clipped away, on HTML read as
  rich text whose style sheet ends in a non-ASCII character inside an unclosed rule, or on a
  catalogue text that ends inside a placeholder. A bar chart with an
  infinite or huge value no longer hangs.
- Screen readers on Windows get the expand / collapse pattern of drop-downs and menus (it had a
  wrong interface id), and on Linux a read-only text box no longer shows as editable.
- `ui::main!` and the new `ui::test_main!` expand without `#[allow(unsafe_code)]`, so a crate
  with `#![forbid(unsafe_code)]` can use them (`ui::main!` was error E0453 there).

### Security

- `MV_OPEN_LINKS` and `RT_OPEN_LINKS` open only http, https and mailto links by themselves.
  Other links of a document (`file:`, `ms-msdt:`, `search-ms:`, relative ones) still reach the
  app (`MdEv::link`, `ui.richtext_link`), which decides; a link in a Markdown file or in rich
  text could start a program before.
- `markdown_view` and `html_view` read an image's file only when it is a regular file of at
  most 32 MB, and never from a network or device path: `![x](//server/share/x.png)` made Windows
  connect to that server when the document was shown, and `/dev/zero` was read until memory
  ran out.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
