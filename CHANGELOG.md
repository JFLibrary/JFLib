# Changelog

All notable changes to the JFLib library -- the crates under `lib/`, the styles, the build
(`build_rs.py`) and its tools -- are recorded here. The format follows
[Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning 2.0.0](https://semver.org/spec/v2.0.0.html) as
[docs/VERSIONING.md](docs/VERSIONING.md) describes (what is public, deprecations, how
`build_rs.py api-diff` checks a release). Apps keep their own versions (`version=` in
`apps/NAME/PACKAGE`) and their own notes.

Add entries with `python build_rs.py changelog "What changed" --type added|changed|deprecated|removed|fixed|security --area AREA`;
`python build_rs.py release X.Y.Z` turns `[Unreleased]` into a release.

## [Unreleased]

### Added

- **ui:** Full keyboard operation: focus groups (the arrows inside toolbars, radio sets, swatches and button grids), F6 / Ctrl+F6 through panes, docked panels and tool windows, F8 for splitters, the menu key / Shift+F10 everywhere, Enter, Space and Ctrl+arrows in lists, grids and trees
- **ui:** A rich-text editor (`ui.richtext`, a toolbar, HTML in and out) and a code editor (`ui.code_editor`) with syntax highlighting for Rust, JSON, INI, TOML and Markdown, find and replace, and files of 100 000 lines
- **plat:** Android: the system's file picker (Storage Access Framework: `content://` names for every file function and stream, grants kept across restarts, folders), persistent app storage and the system clipboard
- **ui:** A live inspector (--inspect, Ctrl+Shift+I / F12): an overlay with the widget under the pointer, layout boxes, Tab order and repainted rectangles, and a tool window with the widget tree, properties, a copyable script selector, frame statistics and the theme colours; closed it costs nothing
- **ui:** SVG images and icons (lib/img_svg: paths, shapes, transforms, gradients, clip paths, group opacity, CSS, use / symbol, text with the UI's fonts; the image format IMGF_SVG for .svg / .svgz), ui.svg for sharp pictures at any size and the image viewer ui.image_view (zoom at the pointer, pan, fit / 100 % / zoom to a rectangle, scroll bars, keyboard, pixel grid)
- **text:** Subpixel (ClearType-like) text smoothing in the system's subpixel order on the window's framebuffer (Text::set_subpixel; headless only with jfgui_FONT_RGBA), and hyphenation: spell::Hyphenator (Liang's algorithm on TeX patterns and libhyphen's hyph_xx_YY.dic, results as libhyphen's), TX_HYPHENATE / TB_HYPHENATE, soft hyphens in every wrapped text, text::segment::wrap_end_hyph
- **tools:** A static website from the docs: python build_rs.py site writes build/site/ (a landing page from README.md, every guide, the app READMEs, changelog and roadmap as pages, the developer guide split per section, the API reference per crate with an anchor per item, search, light and dark themes, phone layout); site --check checks every link, anchor and image, the HTML and that two builds are byte-identical, and runs in every test suite
- **plat:** plat::secret on Android and in the browser: Android keeps the values encrypted with a non-exportable AES-256-GCM key of the Android Keystore (one per app, no user authentication) in the app's no_backup folder; the web host keeps them encrypted with a non-extractable WebCrypto AES-GCM key per app in IndexedDB (decrypted for the app at start, changes committed asynchronously); secret_available() is true where the store works, the API is unchanged

### Changed

- **spell:** Spell checking agrees with Hunspell 1.7 on real dictionaries (COMPOUNDRULE, compound flags, forbidden words, CHECKCOMPOUNDPATTERN ...) and checks and suggests faster; suggestions come from a worker, so the menu opens at once
- **img_core:** img_core::icc_xform: ICC_BI_ADOBE is now ICC_BI_A98 and ICC_BI_PROPHOTO is ICC_BI_ROMM (the same values; names of the standards instead of trademarks)

### Fixed

- **tests:** The cancellation, wake-up and download checks of plattest and coretest no longer race on a loaded machine
- **ui:** Text scale 150 to 200 %: bar chart column labels no longer run into each other (every k-th one, the first and the last kept, inside the chart; value steps a line apart), badges, grid headers, navigation items, segmented items, tabs, info bars, legend entries and text box placeholders that do not fit end with an ellipsis instead of a cut through a glyph, badge and navigation pills grow with the text (Ui::dp_text), donut centre texts stay inside the hole
- **ui:** The weekday toggles (ui.weekday_toggles) wrap their seven buttons (BOX_WRAP) where the row has less room than they need, instead of running past the edge of a narrow card

## [0.1.0] - 2026-10-20

The first versioned state: the Rust rewrite of jfguiLib, published as JFLib on 2026-10-14, and
the roadmap work since then ([docs/ROADMAP.md](docs/ROADMAP.md)). Its public API is recorded in
`docs/api/0.1.0.txt`.

### Added

- **Foundation:** `#![no_std]` crates without libc for Windows, Linux (X11 and Wayland), the
  browser (WebAssembly) and Android: the immediate-mode UI with interchangeable styles, a software
  renderer, text shaping, image codecs from PNG to JPEG XL, effects, documents and DOCX,
  networking (HTTP/1.1, TLS 1.3, SSH-2, DNS), cryptography, compression, an embedded database and
  backup repositories; screenshots, widget dumps and `.uis` scripts for tests, and opt-in agent
  control (JSON, MCP).
- **Permissions and security (roadmap 1):** the app manifest `apps/NAME/PERMISSIONS`, enforced by
  `plat` in every program of an app (files, programs, URL schemes, plugins, printing, secrets,
  autostart, `net.connect` / `net.listen`); questions at run time with remembered answers; rights
  for plugins and agent actions; passwords and tokens in the system's secret store
  (`plat::secret`).
- **Files and resources (roadmap 2):** one file namespace (`app://`, `config://`, `data://`,
  `cache://`, `logs://`, `mem://`, the user's folders, mounts, overlays, ZIP archives), app assets
  built into the program, the user's standard folders, the recycle bin (`plat::trash`), streamed
  file I/O with progress and cancellation (`plat::aio`).
- **Desktop integration (roadmap 3):** one instance per app, file types and URL schemes (also
  registered without a setup), the system's recent files, a notification-area symbol and
  notifications, drag and drop between widgets and programs, keeping the system awake, starting
  at login, taskbar progress, badges and jump lists.
- **Text and fonts (roadmap 4):** glyph fallback, input methods on X11, font aliases and
  languages, the system's font rendering settings, a font picker and font dialog, Unicode text
  segmentation in text boxes and wrapped text, spell checking (Hunspell dictionaries, the Windows
  spell checker, `TB_SPELL`); JFLib Sans (Selawik with the missing code points built in) as the
  text font of the winui and macos styles.
- **Internationalisation (roadmap 5):** system languages, regional formats, plural rules and
  message catalogues, mirrored windows for right-to-left languages, sorting by language, Unicode
  case mapping, the library's own texts in nine languages.
- **Widgets and layout (roadmap 6):** a grid box, an undo history, easing curves, docking panels
  in tab groups and floating tool windows, popups that leave the main window, a diagram canvas
  (nodes, ports, edges, selection, minimap), a terminal widget, multi-colour vector symbols and
  the netlab style.
- **App framework (roadmap 7):** a log framework and lifecycle hooks, typed settings, saved window
  state, widget ids from names, async tasks in the event loop (`ui.spawn`, `plat::task`).
- **Delivery and updates (roadmap 10):** signed app updates with rollback (`lib/update`), code
  signing (Authenticode, a signed apt repository, the Android release key), packages as AppImage,
  Flatpak, MSIX and RPM `.spec` next to the Windows setup and the `.deb`.
- **Accessibility (roadmap 11):** the UI for screen readers through AT-SPI2, UI Automation, an
  ARIA shadow tree in the browser and Android's accessibility nodes; the system's contrast,
  accent, motion and text size settings; the caret for screen magnifiers.
- **Network tools:** packet codecs from Ethernet to TCP with a dissector and pcapng files
  (`netpkt`), deterministic random numbers, typed JSON.
- **Developer experience (roadmap 12):** `python build_rs.py new NAME --kind window|console|tool|service`
  and `new --lib NAME` create apps and library crates that build and pass their test right away;
  `build_rs.py test NAME` tests one app; the library's version (`lib/VERSION`, `ui::VERSION`,
  `plat::VERSION`) and `rt::app_version!()` for an app's own `version=`; the SemVer policy
  ([docs/VERSIONING.md](docs/VERSIONING.md)), API snapshots and `build_rs.py api-diff`; this
  changelog with `build_rs.py changelog` and `build_rs.py release`.

### Fixed

- HTTP servers linger after an error answer, so the client can read it.
- App assets stay byte for byte the same in every checkout (no line-end conversion).
