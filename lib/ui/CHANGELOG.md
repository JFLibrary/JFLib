# Changelog of jflib-ui

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- (JD21) `ui.shape_edit`: the quick shapes bar at a hot connection arrow (Visio's mini toolbar of the AutoConnect arrows): `SeState::quick` (`SeQuick`: name, preview, version; the first four show), the arrow stays hot while the pointer is on the bar, `SeEv::QuickHover` (the tile under the pointer, for a live preview), `SeEv::Quick` (a click), `SeState::quick_hover()`; one `K_BUTTON` per tile ("Schnell-Shape: <name>"); nothing changes for apps that list no quick shapes

- (JD21) `ui.shape_edit`: `SEF_UP` -- a second, slow click on a selected member with this flag selects its group
  (Visio's "members first" groups)
- (JD8 rest) `ui.shape_edit`: the freeform tool `SE_FREEFORM` (the pointer's way, reported as
  `SeEv::Stroke` with the glue of both ends) and the arc tool `SE_ARC` (a quarter ellipse as the
  rubber band, bowing the way the drag started, reported as `SeEv::Arc`); both glue to connection
  points like the line tool; Shift with the rectangle and ellipse tools draws a square and a circle, Shift while moving keeps
  to one axis;
  `SEF_NOSNAP`: moves do not snap to such a shape (the dynamic grid leaves it out)

- The Shapes window of a diagram editor, modelled on Visio's but knowing nothing about diagrams:
  `ui.stencil_panel(id, &mut StencilState, &StencilPanel) -> Vec<StencilEvent>`. The app hands in
  stencils (`StencilDef`, `SD_EDITABLE` for favourites) of tiles (`StencilTile`: a name, a
  QuickInfo, a preview drawn through `ui::Draw`; `TL_QUICK`, `TL_ARMED`, `TL_DISABLED`) and the
  hits of its search (`StencilHit`); the panel shows a search box with a history, a "More shapes"
  button, title rows (tabs) for the quick collection (`SK_QUICK`), the results (`SK_SEARCH`) and
  each stencil, a splitter and the tiles of the active view in four modes (`SV_ICONS_NAMES`,
  `SV_ICONS`, `SV_NAMES`, `SV_DETAILS`), the quick shapes above a line. It reports clicks
  (`Activate`), placing (`Place`: Enter, Space, a double click), menus with their point
  (`TileMenu`, `StencilMenu`, `More`), `Search`, drags of tiles (`DragStart`, `DragEnd`,
  `stencil_drag_decode`) and drops onto editable stencils (`Dropped` with the index to insert
  before). Titles and tiles are reachable by keyboard, tiles are buttons for screen readers, and
  only the rows of tiles in view are laid out. Every text comes from the app (`StencilLabels`).
- The window parts around a ribbon: `ui.ribbon_tabs(id, &mut sel, &tabs)`, the register row
  with the File register (`RibbonTab::file`, `EV_OPEN`), registers and context sets
  (`RibbonTab::context`, a coloured edge); the ribbon's access keys (key tips: Alt or F10,
  `ui.keytip`, `ui.keytip_keys`, `ui.keytips_level`, `ui.keytips_off`; the ribbon's buttons
  register themselves); `ui.backstage_begin` / `backstage_end` with `BackstageItem` (`BS_ACTION`,
  `BS_FOOTER`, `BS_SEP`), the File register's page; `ui.settings_section`, one section of a
  settings schema. Style hooks with defaults that keep every style's look: `ribbon_box_flags`,
  `ribbon_band`, `ribbon_tabs_height`, `ribbon_tabs_strip`, `ribbon_tab`, `keytip`.
- What a notch of the wheel scrolls as the system says it: `ui.wheel_lines()` (Windows' mouse
  settings, KDE's; `plat::WHEEL_PAGE`: a page), `ui.wheel_chars()` and `ui.wheel_notch_px(line_px,
  page_px, horizontal)`, followed live (`UIC_SETTINGS`); `WHEEL_LINE_DIP` (16), the library's
  line. Scroll views, lists and documents scroll that many lines of 16 DIP a notch (48 DIP with
  three, as before), text boxes, the rich and code editors, the terminal and the code completion
  list that many of their own lines; image views and diagrams pan by it.
- Pixel-precise scrolling: `UiInput::wheel_precise`, `wheel_px_x` and `wheel_px_y` give a
  touchpad's, trackpad's or high-resolution wheel's turn in physical pixels (`plat::WHEEL_PIXELS`
  as the platform gives them, Windows' fine steps `WHEEL_FINE` as `wheel_lines()` lines of 16 DIP
  per 120), to scroll by at once; `wheel_x` / `wheel_y` say the same turn in notches, the fractions
  of both carried from turn to turn. The library's scroll views, lists, documents, text boxes,
  rich and code editors, image views and diagrams scroll by the pixels at once (no smooth
  scrolling).
- `KeyEvent::ch` and `KeyEvent::dead`: the character the keyboard layout gives the key with Shift,
  Caps Lock and AltGr but without Ctrl, Alt and the Windows / Super key -- as a browser names the
  key also when Ctrl or Alt is held: Ctrl+Z on a German keyboard is `VK_Z` with `'z'`, its Ctrl++
  `VK_OEM_PLUS` with `'+'`, AltGr+Q `'@'`; 0 for keys without a character; a dead key with its
  spacing character.
- Test scripts: `wheel S DY [DX] [fine|pixels]` (a horizontal turn, fine steps, a trackpad's
  pixels) and `keydown` / `keyup ... [char "c"|none] [dead]`; a script's keys have the US layout's
  characters.
- `GpuContent::paints()` (default false): a view's content that paints itself on the CPU (render2d's CPU path) is painted wherever there is no device, also where a lost device left its last frame read back, which would otherwise stay whatever changes
- `ui::dock_float_id(area, win, k)`: the window id of a dock area's floating window `k` -- `area + 1
  + k` in the main window as before, an id of its own hashed from area and window in any other.
  docs/GUIDE.md section 15.
- `Ui::list_end_full(&mut sel) -> ListEnd { ev, sel, drop }`: the end of a list with a dropped row (LF_REORDER) as `drop = Some((from, to))`, for any number of rows.
- (G7a) `KeyEvent::code`: where the key sits as a USB HID usage (`plat::KC_*`), whatever the layout prints on it; `ui.key_code_down(code)`; `ui.pointer_lock(on)`, `ui.pointer_lock_state()`, `ui.pointer_motion()` (the raw motion of the locked pointer for the pass) for the window of the pass, `ui::PointerLock`; a change of the lock and the motion give passes. Scripts: `motion DX DY` and `keydown CHORD code N`; a headless run plays the lock (locked at once, Esc lets it go). docs/GUIDE.md section 39
- (JD7) Editing shapes on a canvas: `DG_EDIT` makes `ui.diagram` the view only (zoom, pan, the paint) and `ui.shape_edit(id, &mut SeState, &mut DiagramState, &SeModel, tool) -> Vec<SeEv>` takes the left button, the keys, the cursor and the overlays: selection by click, marquee, keyboard, Shift and Ctrl; the eight handles and the rotation handle of turned and flipped frames (`SeBox`), locked handles grey, the rotation pin after the rotation handle was touched, yellow control points, vertices, the ends of 1D shapes; glue of ends and control points to connection points (static) or whole shapes (dynamic, `SeGlue`); moves snapped to the grid, the dynamic grid (other shapes' edges and middles, alignment lines) and guides (`SeGuide`); the connector, line, rectangle, ellipse and text tools; the magnifier (Ctrl + Shift + click, drag, right click); live gestures report totals for the app's preview and `SeEv::Cancel` on Esc; `ui.shape_edit_sems` gives one record per shape and per handle ("Größengriff unten rechts", "Drehgriff", "Anfangspunkt" ...: library texts in nine languages)
- (JD7) `ui.ruler(id, &vp, &RulerSpec) -> RulerEv`: a ruler along a canvas in the document's units for the view of a ui.diagram canvas (ticks and numbers 1, 2 or 5 times a power of ten, the page's extent lighter, the selection's edges and middle marked, the pointer); dragging out of it reports a guide (`RulerEv::Guide { at, done }`, `GuideCancel`)
- (JD7) `DgPaint::damage(from, to)`: the world rectangle an edit changed between two content versions; ui.diagram then paints only that part of its backing store again (one shape edited in a large drawing). `DiagramState::grid_lines`: false keeps the grid for snapping without drawing it (Visio's hidden grid)
- `Mi::List` rows take the state of their id like a command: `cmd_state(base + k, CF_CHECKED | CF_DISABLED)` shows row k checked or disabled (in the window's menus and in macOS' system menu), e.g. the open stencils in JFDiagrammer's "Weitere Shapes"; before, list rows were always plain
- `SE_PLACE`, a tool of `ui.shape_edit` for placing what the app holds (a shape from a stencil, click + click): a click anywhere, on a shape too, reports `SeEv::Draw { tool: SE_PLACE, a, b: a }` at the snapped point; no handles, a drag places nothing
- (JD10) Connection arrows and insertion marks in `ui.shape_edit`: with the pointer tool a top-level shape under the pointer shows four arrows around its box (`SeState::arrows`, on by default; `SEF_NOARROWS` leaves a shape out); a click on one reports `SeEv::Arrow { key, dir }`, a drag from one draws a connector from that shape (`SeEv::Connect` with `ga: SeGlue::Shape(key)`), `SeEv::ArrowHover` and `SeState::arrow_hover()` tell the arrow under the pointer; the app's insertion marks (`SeModel::marks`, `SeMark`) show near the pointer and report `SeEv::Mark { tag }`; `ui.shape_edit_sems` names both for scripts and screen readers
- (JD8) `ui.tabs` and `ui.tabs_scrolling` report `EV_DOUBLE` when a tab is double-clicked (it is the selected one then); a double click on the strip beside the tabs no longer does. A right click on a tab selects it as a click does, so a context menu on the strip belongs to the tab under the pointer
- (JD11) `ui.color_menu(id, &ColorMenu)` -> `Option<(ColorPick, rgb)>`: the swatches of an Office colour menu (design colours with five shades each, variant colours, fixed colours, last used; the current one marked) and `color_shades(rgb)`; `ui.ribbon_image_tile(id, label, &surface, ver)`: a gallery tile with a picture the app made
- (G7a part 2) `ui.gamepads()`: the game pads connected now (`ui::Gamepad`, the same as `plat::Gamepad`) with the edges of their buttons in this batch; the first call starts watching (`plat::pad_watch`), every change gives a frame, a pad that comes or goes gives `App::command(UIC_GAMEPAD, index | PADEV_* << 8)`; `ui.gamepads_watching()`; `ui::PadStyle`. Scripts: `pad connect N [xbox|playstation|nintendo|other] [NAME]`, `pad disconnect N`, `pad press N BUTTON [VALUE]`, `pad release N BUTTON`, `pad axis N lx|ly|rx|ry VALUE`. docs/GUIDE.md sections 12 and 39
- (JD15) `ui.tooltips_set(on)` / `ui.tooltips_on()`: every tooltip of the app off and on again, for an app's option "no ScreenTips" (JFDiagrammer's QuickInfos › Keine QuickInfos)
- (JD15) `ui.shape_edit`: `SeState::partial` (the marquee and the lasso also take the shapes they only touch), `SeState::snap` and `snap_guides` (moves snap to guides and to the dynamic grid at all, to guides; for a page that inhibits snapping and the targets of a snap dialog), `SPF_OUTWARD` (a connection point that points outward only: shown, no target for ends), `SeEv::MoveCopy` (a drag of the selection that starts with Ctrl + Shift moves a copy of it, live like Move), `SeState::right_taken()` (the last right release was the magnifier's or a pan's: the app opens no context menu)
- (JD15) `StencilState::no_search`: the Shapes window shows no search box (Visio's option to hide the shape search); the query stays until the app clears it

### Changed

- A notch of the wheel scrolls the system's lines in the library's views, no longer always three;
  headless runs stay at three. A trackpad on macOS and a touchpad on Wayland, which plat gives in
  pixels now, scroll the views by the fingers' way (4.8 times it on Wayland and 1.2 on macOS
  before).
- `ui::launch` on Android takes a headless run (--shot, --script, --agent-json, --agent-mcp) as on the desktop, like iOS; the rest stays the Java host's. The desktop's runner adds about 290 KB to an APK
- The scripts' `commit` hands the text to `plat::ime_commit_text` in one call, as a platform does.
- GPU canvases (`ui.canvas_gpu`, image views, diagrams): the tiles of a window take at most eight
  times its framebuffer (`jfgui_GPU_CANVAS_MB`, 128 MiB, stays the upper limit): a 1900 x 1000
  window holds 61 MB of tiles, no longer 128. A tile shows 254 x 254 pixels, its texture with the
  border around them is 256 x 256, so it takes what the budget counts (NVIDIA laid the 258 x 258
  one out as 408 KiB instead of 260).
- A run whose UI output stays on the CPU for good (no Vulkan loader, no usable adapter, a display that takes no dma-bufs, ...) says why in one line of the log at info level, `gpu: output: none in this run (REASON): the windows go through the CPU path`: in the ring for crash reports and `plat::log_recent` and in the log file, on stderr only with `jfgui_LOG=info`; the screen stays silent as before. On Linux `GpuOutput::error` names a display without dma-bufs so instead of "no child window" (docs/ARM64.md, ARM8)
- (AP9) `ui.menubar(None)` of the main window gives the menus to the iPad's menu bar too and keeps drawing its row there (the user's decision of 2026-11-19); a pass whose main window draws no `menubar(None)` takes the app's menus out of the system's bar again (macOS as well: a page without the bar leaves no stale menus up there). The iPad stays `FF_TABLET` (`FF_PHONE` below 600 points) in every window, also a large one in Stage Manager (the user's decision of 2026-11-19)
- The browser: the main window fills the page's canvas without a frame, as on Android and iOS.
  The library draws no title bar for it, a title bar of the app's own (`AF_CUSTOM_TITLEBAR`)
  shows no caption buttons, and there are no resize grips; the browser tab shows the title.
  Before, it looked like a window inside the page. Tool windows keep their title bar, also with
  `WF_SYSFRAME` (the browser draws no frame).
- (G7b) Veils: a cell of the base layer with something over a GPU view (`ui.gpu_view`; a game's HUD, a menu's panel) is composed on the GPU instead of reading the view's frame back in every frame. The CPU draws what lies over the view once over a black and once over a white view; the output's pass draws the cell as K + T x frame (`fs_veil`), the CPU's "over" to one level; a new frame of the view costs those cells nothing (before: the whole frame read back and waited for, 1.3 ms at 1200 x 700 on an RTX 2070 SUPER, and the cells drawn and uploaded again). Text in veils is greyscale; at rest the framebuffer gets the true pixels once (`gp_settle`). `jfgui_GPU_VEIL=0` and `Ui::gpu_test_veil(false)` give the way before; `GpuOutput::veil_cells`, `veil_rects`, `veil_draws`, `veil_settles`; `GPU_OUT_FS_VEIL`. uigputest `--test viewplain`: no frame read back while the view turns under something, the screen against the CPU's picture
- (JD15) `ui.shape_edit`: `SeState::show_points` shows the connection points of every visible shape (also while glue is off), not only those of the shape under the pointer; Ctrl while dragging a dynamic end swaps static and dynamic glue (over a connection point the whole shape, over a shape its nearest connection point); Ctrl + Shift + right drag pans the view (a right click with them still zooms out); Tab and Shift+Tab without a selection take the first and the last shape; Ctrl + Shift on a selected shape: a drag copies, a click zooms in as elsewhere

### Fixed

- `ui.shape_edit`: a click on a connection arrow did nothing when a handle of another selected
  shape lay under it (the shape a first arrow click had just made, its side handle under the
  arrow); a drag from it resized that shape. The arrow shown hot now takes the click and the drag;
  only the handles of the arrows' own shape, or of a selection box around it, still come first.
- GPU canvases: a tile evicted for a new one hands its texture over when no frame draws it any
  more; a zoom over many levels made a new texture for every tile while the evicted ones lived on
  until the GPU had finished the frame.

- The code editor's horizontal wheel scrolled the wrong way: tilted right it scrolled left.
- An image view whose wheel zooms panned twice as far with the horizontal wheel while the
  vertical one stood still (and not at all in a right-to-left window).
- The fine steps of a precision touchpad were cut off one by one where whole lines count: the
  terminal never scrolled with steps below 40, the code completion list scrolled three rows on
  each of them, Ctrl + each of them zoomed a diagram a whole step. They add up now.
- Dock: a floating window is found by its slot when the dock's windows are walked. The app changing the layout in the same pass made the stored index name another floating window or none.
- A suggestion menu open in a window that closes or whose state is freed (`ui.window_destroy`) goes
  with the window: it stayed an app-wide entry, and the menu key opened no menu in any other window
  until a right click replaced it.
- Esc in a text box inside the top popup closes the popup and gives the focus back to where it was
  when the popup opened, as Esc anywhere else in it does: in the font picker's search box it only
  took the focus away and left the family list open (the next click then closed it instead of
  opening it).
- A dock area of the same id in a second window floats its panels in windows of its own: both used
  the window ids `id + 1 ..`, and the second one took over the main window's floating windows
  ("window 501 begun twice in one pass").
- Wrapped labels and multi-line text boxes check the text for UTF-8 once per layout, not once per line, and do not measure the rest of the paragraph before each line (both quadratic over a long paragraph).
- Agent interface: `ui.command` of a command bound to a domain action whose input schema needs fields is `invalid_input` (the action ran with an empty input past the check every other call has).
- Code editor: accepting a completion checks again that its start is still where the word at the caret begins, else the list closes (after an edit since it opened the cut went from a stale position, on another line or past its end).
- Terminal: the local echo of a line is written apart from the peer's escape state (an open sequence of its output took the echo in, or the echo ended the peer's sequence); closing the search bar clears its hits (they stayed marked in the text).
- Window state: a tool window restored at a place where (almost) nothing of it is on a monitor is moved onto the nearest monitor's work area, as the main window (it opened off-screen after a monitor was gone).
- Text box: undo to a state longer than a smaller capacity cuts it on a character's start (it could cut a UTF-8 character); a password box's accessibility record gives caret and anchor as indices of its dots, and a screen reader's selection in it maps back to the characters (they were byte indices of the hidden text).
- `History`: `undo` / `redo` while a group is open close it first, its edits so far are the newest step (undo reverted the older step under them, redo did an undone step over newer edits); `can_undo`, `can_redo` and the labels count the open group.
- A box nested beyond the layout's limit pushes no clip or disabled level and its `box_end` pops nothing (it took the parent's frame, clip and disabled level); an expander beyond the 16 kept ones opens no content card (its box stayed open).
- `list_end` packs a drop into `from | to << 16` only while both fit in 16 bits: a move from or to a row past 65 535 is no longer reported (no EV_DRAG, the row stays selected) -- the packed value gave another item and another place (`list_end_full` reports them).
- Hebrew and Arabic in labels, buttons, check boxes, radio buttons, headings and lists read backwards ("מחיקה" as "הקיחמ", also on the Windows setup's pages): text now lays out lines with right-to-left characters in their visual order without shaping (`text::Text::set_bidi`), Arabic joined. Text fields draw, place the caret and select in that order too: a selection is drawn as its visual spans (`Text::selection_spans`), several where it crosses directions, and the end of a right-to-left line is its width. The rich-text and code editors keep such letters in logical order, cluster by cluster, as their carets are

### Security

- Permission dialog: the target, plugin, reason and rule are wrapped and whole, control characters and the characters that change the writing direction show as `<U+XXXX>` (a long target lost its end at the dialog's edge, U+202E could turn a file name around, NUL ended the text).
- Terminal: a change into or out of `TM_SECRET` drops (overwrites) the input typed so far: a hidden line showed in the next mode's input line, a visible one became part of the secret.
- `TB_MULTI | TB_PASSWORD` is a one-line password box: the multi-line box ignored `TB_PASSWORD` and showed the password in plain text.
- A text a screen reader sets for a password box is taken as the box's dots with the user's change (the kept dots stand for the hidden characters): a Backspace of TalkBack (browser, Android) set the password to the dots.

## [0.2.1] - 2026-11-16

### Added

- `FD_BUILTIN` in the flags of `ui.filedlg`: the library's own file dialog instead of the
  system's, on every platform, so the dialog looks and behaves the same everywhere and in every
  style (a browser's "Save as"). It lists what plat's file functions see. Before, only the
  environment variable `jfgui_FILEDLG=builtin` chose it, and for every dialog of a run.
- Test scripts: a line `only OS...: COMMAND` runs one command on the systems it names
  (`only windows linux: key ctrl+q`), and `!OS` on every system but that one. The names are those
  of `cargo jflib test`'s `#!only` line, which skips a whole script: windows, linux, macos, ios,
  android, wasm32. The app decides it where the script runs; an unknown name is a parse error.
- `ui.drop_defer()` and `ui.drop_finish(effect)` work for the app's own drags too. A drag of the
  app dropped onto one of its targets waits for the target's answer like another program's drop:
  its source reports `DS_ACTIVE` until then, and `DS_END` with `drag_effect()` and
  `UIC_DRAG_END` as `drop_finish` says (0: not taken). After `DROP_DEFER_MS`, or when a new drag
  of the app overtakes it, it ends as it was delivered. Without `drop_defer` the source's
  `DS_END` comes in the passes right after the drop's, no longer in the same pass as the target's
  `DS_DROP`. In the drop's passes `drag_allowed()` says the source's effects, as for other drops,
  and `dragging()` is false once the drag was dropped.

### Changed

- A drag whose `DragData::effects` is 0 allows every effect, as `drag_allowed()` and the
  effects of a drop already said for a source that does not say: inside the app a plain drag
  still moves, Ctrl+Shift links and a target that only links takes it, and dragged out it offers
  linking to other programs too. Before, 0 meant copy and move there, and in the highlight of a
  target whose `drop_effect_set` takes only linking under another program's drag that named no
  effects. A source that must not be linked names its effects (`DE_COPY | DE_MOVE`); files are
  still never moved.
- Linux: the build script sets `--cfg jfgui_glibc` (the output through the GPU) also without std, unless `-C target-feature=+crt-static` asks for one static file; so a JFLib app builds on Linux with plain `cargo run`, no flags and no `--target`.

### Fixed

- Test scripts: `type "..."` typed only the first 255 bytes of its quoted text (so did
  `bench N type`, and `commit` 1023). They take the whole string now, as long as a script line
  (16 KiB).
- Widgets of the same id in two windows (a browser's windows built by the same code) keep
  their state each, as the state slots since 0.2.0: the code and rich-text editors' views
  (scroll position, find bar, completion list), the colour pickers' fields and wheel images, the
  font pickers' lists and searches, the spell checks of `TB_SPELL` boxes and their suggestion
  menu, the charts' drawings and line-chart cursors, the dock and window areas, the file dialog,
  the canvases of `canvas_damage`, the target of the menu key and of a focus group's arrows, the
  colour picker a pipette pick is for. Before, one window's widget took over the other's state:
  a find bar showed in both, a hex colour being typed was overwritten, a font list or a file
  dialog opened again in every pass, a chart's cursor was lost, a spell menu in a second window
  closed at once. Calls that name an id address the window being built when it has such a
  widget; else (outside the passes too) the queries (`code_status`, `code_events`, `code_diag`, `code_popup_item`, `spell_errors`) read the
  first window that has the widget, `code_find`, `code_reveal` and `richtext_link` go to the
  first editor of the id that is built, and `canvas_damage` to the canvas in every window.
- The syntax highlighter nests at most 32 levels within a line (template strings in `${..}`, Nest and Element rules); deeper, the rest of the line keeps the construct's colour. A long line of "`${" ran the stack out.
- Terminal: cursor moves (`ESC[H`, `f`, `G`, `C`) reach column 1024 at most. The spaces up to a far column made 12 bytes into a scrollback line of 32000 cells.
- The GPU output: a window whose swapchain could not be made because the device was lost (lost while no window showed on it: the others on the CPU path, hidden or kept at another size) no longer tries again on the lost device every 2, 4, 8 ... seconds; every window goes to the CPU path and a new device follows, as for a failure while showing. Test scripts: `gpu on` prints a SKIP when the screen's work area cannot hold the script's window (the system keeps it smaller, on the CPU path), instead of checks that cannot hold there.
- On a phone or a tablet a vertical scroll view rolls sideways only while its content is laid
  out wider than the view, and only as far as that content reaches. Before, the natural width
  decided, where a wrapping text counts unwrapped and a view inside that rolls sideways itself
  with its whole content: nearly every page rolled sideways into nothing, and the finger after a
  sideways swipe only stopped that roll (no tap, no press and hold on a slider).
- `ui::main!` in a library crate with std (the app of `cargo jflib new --std`): `cargo test --lib`
  builds its unit tests. The panic check refused the test build (tests always unwind), and
  `ui::test_main!` beside `main!` defined `jfgui_main` twice; `main!`'s own `jfgui_main` links the
  tests now. `ui::test_main!` stays for library crates without `main!`, such as a style crate.

### Security

- The file dialog takes a chosen path longer than its `cap` as not chosen (the system's dialogs) or says it is too long (the library's own). The path was cut, reported as chosen and granted as cut.
- The permission dialog's two allowing buttons take no input for half a second after a question appears, so a double click or a held key does not answer it unread.
- Windows: URL schemes are registered as `"<exe>" -- "%1"` and plat::args keeps the URL after -- as one argument, so a quote in a link cannot add options; ui stops reading options at --. Apps registered before keep the old command until they are registered again

## [0.2.0] - 2026-11-14

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
- The syntax highlighter reads a block whose depth was lost in the 32-bit line state (below three nested constructs) as one open block, so `*/` closes it; before, the depth went below 0 and the rest of the line stayed a comment.
- Overflows that only a build with overflow checks saw (a debug build of an app without JFLib's profile aborted): the Markdown document's update of changed blocks, the terminal's resize anchor after the scrollback dropped lines.
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

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
