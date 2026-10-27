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
- **ui:** A Markdown / HTML viewer (ui.markdown_view, ui.html_view, MdDoc: CommonMark with GFM tables, task lists, strikethrough and bare links, the HTML of the rich-text reader; headings, lists, quotes, highlighted code, tables, images through the VFS and img_svg, right-to-left paragraphs; selection and copy as text and HTML, links through the keyboard, only the visible blocks laid out) and lists whose rows have different heights (ui.vlist_begin / VListState: measured or given heights, anchored scrolling that keeps still when rows above change, 100 000 rows); a new record kind K_HEADING
- **plat:** plat::ime_commit_text: committed text of any length as input method commits of at most 4 KB (the bytes queued); the test script command commit "text" [N]
- **ui:** Headings reach screen readers as headings with their level on every platform: K_HEADING records (the Markdown / HTML viewer's headings and the new ui.heading(level, style, text) for section titles) become plat::a11y::Role::Heading with Node::level() instead of labels -- UI Automation Text elements with HeadingLevel1 .. 9 (AriaRole heading, AriaProperties level=N), AT-SPI2 role heading with the object attribute level:N, role=heading with aria-level in the browser's shadow tree, AccessibilityNodeInfo.setHeading on Android (API 26 / 27: the heading flag of a CollectionItemInfo)
- **plat:** The browser's own files of the user: in Chromium ui.filedlg / plat::file_dialog (FD_OPEN, FD_SAVE) and plat::web::pick_file show the system's pickers (File System Access API); the chosen file is /handles/ID/NAME, written back into the user's file on every change (write_file, AtomicFile, File streams; persist confirms it), its handle kept in IndexedDB across reloads, plat::web::reopen reads it again after asking for the permission from the user's gesture; Firefox and Safari keep the library's own dialog, imports and downloads
- **plat:** Android: content:// writes go through a journal in the app's data folder (the copy first, then the document written in place, read back and compared; a write a crash interrupted is finished at the next start, the document keeps its URI and grants), and names below a chosen folder are cached (each document checked once before use, forgotten on create / delete / rename, emptied when the provider reports a change through a ContentObserver)
- **plat:** Flatpak: every document of the documents portal's mount is allowed with fs.user inside the sandbox, so a file chosen in the portal's file dialog (a lasting document) opens again from ui.recent_add's list after a restart without a dialog; plat::persistent_name asks org.freedesktop.portal.Documents (Add, persistent) for the lasting name; the library's D-Bus client sends file descriptors
- **text:** text::segment::cluster_end: the end of the grapheme cluster that starts at a boundary (walks over a text without looking back)
- **ui:** Test scripts: bench N key CHORD [MAX_US] and bench N type "s" [MAX_US] time frames of key presses and typing; ui::code_long_check (tests) checks what the code editor keeps of long lines against a fresh look
- **plat:** plat::UiaTestElem::localized_type (tests): an element's LocalizedControlType
- **netpkt:** BPDUs of the spanning tree protocols (netpkt::stp): Configuration and TCN BPDUs of IEEE 802.1D-1998 and RST BPDUs of 802.1D-2004 with their flags and port roles, BridgeId and PortId, the path costs of 802.1D-2004 Table 17-3, validation as 9.3.4 says, the 802.3 / LLC frames to 01:80:c2:00:00:00; decode::layers shows every field and flag of a BPDU, decode::info sums it up
- **netpkt:** RIP messages (netpkt::rip: version 2 after RFC 2453 and version 1 after RFC 1058, requests and responses with their route entries, the authentication entry recognised, simple passwords read and written) and their dissection (decode::layers: a field per entry with its bytes and what a receiver refuses; decode::info: "RIPv2 Response, 3 routes")
- **netpkt:** Address and port translation of IPv4 packets in place (netpkt::nat: Flow::of / Flow::quoted, rewrite and rewrite_quoted with incremental checksums of the header, TCP, UDP and ICMP, ICMP errors with the packet they quote per RFC 5508) and checksum::update for a changed run of bytes
- **base:** Tree::find_mode(prompt): a mode by its prompt, so modes that parts of an app add need no fixed number
- **plat:** plat::audio: sound output on every target -- streams (Stream::open + write / write_i16 into a ring the platform drains, or open_fill with a callback on the audio thread; start / pause / stop / finish, volume, latency, played frames, underruns, the default device followed), short sounds mixed by play / play_file (WAV PCM and float, Ogg Vorbis), the system's sounds by sound(SOUND_*); Windows WASAPI, Linux the PulseAudio protocol (pipewire-pulse, WSLg), the browser an AudioWorklet (web/audio.js), Android AAudio (android/audio.c). ui.beep, ui.dialog_sound and ui.set_sounds play them from the UI; scripts check them with expect sound
- **netpkt:** ICMPv6 and Neighbor Discovery (netpkt::icmpv6: echo and the errors of RFC 4443 with their quote and the checksum over the pseudo header; router and neighbor solicitations and advertisements and redirects of RFC 4861 with the options source / target link-layer address, prefix information, MTU and redirected header; Nd::parse and check_ip with the receiver's validation as NdErr), IPv6 extension headers, the sender's fragments and reassembly (netpkt::ipv6: ext_headers, upper_layer, fragment, frag, Reassembly6 with RFC 5722's overlap rule), Ipv6::solicited_node, link_local_of / eui64_in (EUI-64 of RFC 4291) and Mac::of_ipv6_multicast; decode::layers shows every field and option, decode::info sums a message up
- **base:** IPv6 parameters in command trees: an address (X:X:X:X::X, Parsed::ip6) and an address with a prefix length (Parsed::prefix6); cmdtree::parse_ipv6 reads such an address alone
- **netpkt:** OSPF version 2 packets and LSAs (netpkt::ospf after RFC 2328 appendix A: the header with its checksum and authentication field, Hello, Database Description, Link State Request, Update and Acknowledgement, router-, network-, summary- and AS-external-LSAs with the Fletcher checksum, which of two instances is more recent) and their dissection (decode::layers: every field and every LSA with its bytes and problems; decode::info: "Hello Packet, DR 10.0.0.2, BDR 10.0.0.1, 3 neighbors")
- **plat:** The permission audio.output: an app with a PERMISSIONS manifest plays sound (plat::audio streams, play / play_file, the system's sounds, ui.beep) only with it, else AudioError::Denied, sound() false and one PERMDENY audio.output; ui.dialog_sound stays silent without a question; ask audio.output asks at the first sound; the Flatpak package gets --socket=pulseaudio only with the right
- **tools:** python build_rs.py i18n [APP|lib] [--update]: extracts the translatable texts of the apps (t / tc / tr / tf, message templates, tables, command and menu labels) and the library (tr against LIB_TEXTS) and reports missing, untranslated and unused rows and placeholders or plural forms that differ from the source; --update adds the missing texts as untranslated rows; the suite runs it with --check
- **plat:** OS sandbox from the permission manifest, opt-in per app with a sandbox / sandbox strict line: Linux Landlock (filesystem and TCP) plus seccomp syscall deny-list, Windows process mitigation policies (image load, extension points, dynamic code, child processes)
- **ui:** App themes without a style crate: ui::Theme (an INI file: colours of light, dark and high contrast, the accent through the style's ramp, corner radii, spacing, control heights, size / line / weight per text style, a font family from a file or the system) applied over any style from the next frame with ui.set_theme / load_theme / clear_theme and --theme-file, errors with the line in the UI language; several styles linked into one app (build_rs.py links every style_NAME the sources name) and switched at run time with ui.styles_add / set_style (Style::name, Text::set_embedded); base::Ini::items; script commands theme file / none, style, expect theme / style / token / pixel / same, rect with *
- **plat:** plat::screen and plat::hotkey: the monitor list (plat::monitors / ui.monitors: name, rectangle, work area, dpi, refresh rate, primary, a stable id; changes as UIC_SETTINGS with SET_MONITORS), global shortcuts (hotkey_register / ui.hotkey_register -> UIC_HOTKEY; RegisterHotKey, XGrabKey with the lock combinations, the GlobalShortcuts portal; the right input.global), a colour picked anywhere on the screen (pick_start / ui.pick_color -> UIC_PICK, the colour picker's pipette CP_PIPETTE; an overlay with a magnifier on Windows, a pointer grab on X11, Screenshot.PickColor on Wayland and in Flatpak, the EyeDropper in Chromium) and pictures of the screen (capture_start / ui.screenshot -> UIC_SCREENSHOT; BitBlt, GetImage, the Screenshot portal; the right screen.capture); the test scripts' system hotkey ID, system pick RRGGBB|cancel and system settings monitors
- **gfx:** Dc::begin_rows(sf, row0, h) and Dc::row0: a surface that holds a band of rows of a taller target, drawn with the target's coordinates (fills, blits, the rasterizer, panel_fx and glyph blits honour it); blend::copy_stream / stream_fence (simd): a row copy past the caches; Shapes::panel_fx starts its mask bands at the panel's first row whatever the clip, fill_rrect / stroke_rrect build their path only when a corner band is inside the clip
- **text:** Glyphs placed now and blended later by any thread: Text::record_n / record_icon fill a GlyphRec (layout, shaping and the masks into the glyph cache), Text::glyphs gives a read-only GlyphView whose draw blends them inside a Dc's clip; GlyphRec::valid tells whether the cache was flushed meanwhile
- **netpkt:** Destination address selection after RFC 6724 (netpkt::addrsel: scope, precedence and label of the default policy table, Dest and sort with rules 1, 2, 3, 5, 6, 8, 9 and 10, the order getaddrinfo tries the addresses of a name in)
- **crypto:** SHA-1 (crypto::sha1, Sha1, hmac_sha1; for protocols defined with it, broken for collisions), PBKDF2 over any hash (pbkdf2::<H>, pbkdf2_sha1), the AES key wrap of RFC 3394 (aes_wrap / aes_unwrap with its integrity check) and AES-CMAC of RFC 4493 (cmac_aes): what WPA2 and WPA3 need
- **netpkt:** EAPOL of IEEE 802.1X-2004 (netpkt::eapol: EapolView, the EAPOL-Key body of the 4-way handshake with its Key Information bits, which of the four messages a frame is, the MIC field and what it covers, RSN elements, GTK and IGTK KDEs, key data padding; eth::ET_EAPOL) and its dissection (decode::layers: every field, the Key Information bits by name, the RSN element and KDEs of plain key data; decode::info: "Key (Message 2 of 4)")
- **ui:** html_view: forms and images the app supplies -- <form>, <input> (text types, password, checkbox, radio, hidden, submit, image, reset, button), <textarea>, <select>, <button> and <label> become the library's own widgets in the text flow (MdDoc::forms / fields, the values in MdViewState: field_text / field_checked / field_selected and setters, form_data, reset_form; MdEv::field and MdEv::submit with an MdSubmit of action, method and the name / value pairs; Tab through links and fields, Enter sends, password fields never reach screen readers or the clipboard); MdViewState::images = MI_ASK / MI_APP asks the app for http(s) and data: images (image_requests, set_image: a placeholder with the alt text until the bytes come, then the block is laid out again)
- **script:** script::compile_with(name, src, globals) compiles a script against a host's own names (the built-ins and the standard library are known anyway); an unknown name is an error there instead of a warning. The standalone runner apps/jfs runs .jfs scripts on the command line (jfs FILE / -e CODE / -, --check): print and log on the console, config from the command line and JFS_CONFIG, timers on the real or a virtual clock, disk in memory or a JSON file, limits by flag, errors with line, column, source line and calls

### Changed

- **spell:** Spell checking agrees with Hunspell 1.7 on real dictionaries (COMPOUNDRULE, compound flags, forbidden words, CHECKCOMPOUNDPATTERN ...) and checks and suggests faster; suggestions come from a worker, so the menu opens at once
- **img_core:** img_core::icc_xform: ICC_BI_ADOBE is now ICC_BI_A98 and ICC_BI_PROPHOTO is ICC_BI_ROMM (the same values; names of the standards instead of trademarks)
- **ui:** ui::KIND_NAMES has 23 entries: the new record kind K_HEADING (level as the value, dump name heading) of the Markdown / HTML viewer; K_KIND_COUNT is 23
- **plat:** plat::a11y::Role has the new variant Heading (the level in Node::value, Node::level()); a match over every role needs an arm for it
- **ui:** The fixed limits of a pass grow as needed: keys of one pass (16 before), commands (32), timers (16), pointer positions, hit boxes, widget records, clip levels, tabs of a tab bar, known canvases and wrapped lines; their ceilings (1024 keys, 65536 commands, 4096 timers ...) only stop runaway apps, and whatever a ceiling or a fixed nesting depth (windows, popups, menus, lists) holds back is reported once with a UIWARN. Due timers fire by deadline, equal deadlines in the order set; the script command keys [N] CHORD... queues many keys for one pass
- **plat:** In Chromium plat::file_dialog answers FD_PENDING for FD_OPEN / FD_SAVE (the system's pickers) instead of -1; code that calls it directly polls plat::file_dialog_poll as on Android (ui.filedlg does)
- **plat:** Presenting copies the dirty rectangles once, straight from the framebuffer: X11 into the MIT-SHM segment that lacks them (no shadow image), Wayland into a free wl_shm buffer that waits for its commit (no kept copy), the browser and Android keep window pixels as RGBA (one swap, no composition for a single window, one pooled buffer per frame); Windows keeps its WM_PAINT copy in a DIB section (BitBlt). plat::present_time() measures it (linuxtest --test pbench / pcheck, tools/tests/web_present.mjs).
- **ui:** Scroll copies: when only a scroll view's content moved, the pixels it still shows are moved in the framebuffer and only the band that came into view and what really changed are drawn, byte-identical to a full redraw (lib/ui/scrollcopy.rs); scroll views, lists, data views, the hex view, vlist, the Markdown and code views and multi-line text boxes take part, own widgets through ui.scroll_copy_begin / scroll_copy_end; only cells with text, icons or images are copied and the measured cost per view decides between copying and drawing (code editor at 3840 x 2160: wheel frames 3.3 -> 1.9 ms); the script command expect scrollcopy [N]|none
- **ui:** Large redraws render on several threads: from about 3 million dirty pixels on (1.5 x 1920 x 1080; 750 000 in a framebuffer of more than 16 MB such as 4K), render_live cuts the dirty rectangles into row bands that the worker pool draws into buffers of their own (ui/render_par.rs), byte for byte what one thread draws; texts and font icons are placed on the UI thread first, custom Draw objects, icons an icon hook or the style paints and glyphs too large for the glyph cache are drawn on the UI thread (their bands up to the first of them on the workers; the API is unchanged); jfgui_RENDER_PAR=0 turns it off, =1 forces it, =2 renders every frame both ways and warns about a differing byte; one thread in the browser without shared memory and on Android. A render leaves the text font the passes set (before, the font of the last text or custom draw it reached stayed set, which depended on the dirty rectangles)

### Fixed

- **tests:** The cancellation, wake-up and download checks of plattest and coretest no longer race on a loaded machine
- **ui:** Text scale 150 to 200 %: bar chart column labels no longer run into each other (every k-th one, the first and the last kept, inside the chart; value steps a line apart), badges, grid headers, navigation items, segmented items, tabs, info bars, legend entries and text box placeholders that do not fit end with an ellipsis instead of a cut through a glyph, badge and navigation pills grow with the text (Ui::dp_text), donut centre texts stay inside the hole
- **ui:** The weekday toggles (ui.weekday_toggles) wrap their seven buttons (BOX_WRAP) where the row has less room than they need, instead of running past the edge of a narrow card
- **ui:** Typed text is never dropped: a long insert or input method commit (beyond 64 characters, a burst of WM_CHAR on Windows, an IBus commit over 4 KB) reaches text boxes and editors whole and in order; a pass takes up to 4096 characters, the rest comes in the next passes, also what a widget leaves with a smaller ui.text_take cap
- **web:** Browser: long text (a 5000-character insert, emoji, combining sequences) reaches the app whole as commits instead of one event per character; in Firefox, typing, inserting and IME compositions in a field of the ARIA shadow tree reach the app (the input event Firefox sends after compositionend no longer undoes the committed text)
- **plat:** X11 and Wayland no longer lose input when one socket read brings more than about 85 key presses (a burst of synthetic keys, a client flooding input, also while the backend waits for a reply): plat's 256-event ring has an overflow queue that keeps the order (up to 65536 events; plat::evq_dropped counts what is lost beyond)
- **ui:** The code editor stays fast on very long lines: a line of 100 000 characters or a 1 MB single-line file (minified JSON / JS, long log lines) types, moves the caret, selects, scrolls sideways and finds in about a millisecond per pass instead of up to a second (7000 characters) or minutes (100 000): only the part of a line in view is drawn, the columns and tokens of long lines are kept and worked again near an edit only, bracket matching tokenizes a line once; text::segment's grapheme and word functions look only near the place instead of from the line start (they made every column lookup quadratic)
- **plat:** UI Automation's LocalizedControlType of dialogs, switches and headings and AT-SPI2's GetLocalizedRoleName follow the UI language (the library's nine languages) instead of German and English strings: plat::a11y::ROLE_TEXTS / role_name / set_translator, ui hands the bridges i18n::tr

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
