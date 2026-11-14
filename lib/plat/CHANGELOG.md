# Changelog of jflib-plat

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

## [0.2.0] - 2026-11-14

### Added

#### macOS and iOS

- macOS and iOS (`aarch64-apple-darwin`, `aarch64-apple-ios`, `aarch64-apple-ios-sim`): plat runs
  on libSystem. The start, arguments and environment, time, files, threads, semaphores and the
  wake-up; processes through `posix_spawn`, `plat::sync`, sockets, `plat::fs`, random numbers,
  the system's trusted certificates (the Security framework), local control sockets and the
  single instance, the keychain as the secret store, crash reports through signals with the call
  chain, the system font folders; `system_languages` and `system_locale` ask CoreFoundation
  (`LANGUAGE` and `LC_*` set by hand still win, `LANG` alone does not). With the feature `std`,
  `plat::std_start` starts a program there too.
- macOS: folder watching through FSEvents, services through launchd (`plat::service`;
  `plat::service::plist_text` gives the property list an install writes).
- macOS: windows through AppKit. The library's own event loop, windows with a layer-backed view of
  their own (Retina scale, the library's frame or the system's title bar), mouse, keys (Command
  acts as Ctrl), text and input methods, the trackpad (fine scrolling, the pinch as Ctrl + wheel),
  custom cursors, popup and owned windows, `win_enable`, the pasteboard (text, HTML, RTF, PNG and
  images; a TIFF arrives as PNG), the Open and Save panels (`FD_FOLDER` chooses a folder), drag
  and drop both ways, the monitors, and the appearance, accent colour and accessibility settings
  with `E_SETTINGS` when they change.
- macOS: the app in the system. The app and Window menus (Cmd+Q asks the main window through
  `E_CLOSE`), the Dock icon brings the main window back, `E_POWER` and `E_SESSION_END`, tool
  windows that take the keyboard only for a text field, `inhibit` through IOKit power assertions,
  the Finder's trash, the recent documents, the system's spell checker, and the print dialog (the
  job printed as a PDF through PDFKit; without a window of the app, as in headless runs,
  `ERR_UNSUPPORTED`).
- macOS: the tray as a status item in the menu bar with its menu, notifications through
  UNUserNotificationCenter (in an app bundle), the Dock tile (badge, progress, the tasks as the
  Dock menu), files and URLs the system opens with the app (they reach it as a later start's
  arguments through `plat::instance`, `UIC_OPEN` in the UI), global shortcuts, and autostart as a
  LaunchAgent.
- macOS: the app's menus in the system's menu bar: `plat::sysmenu` (`sysmenu_supported`,
  `sysmenu_set`, `sysmenu_take`, `SysMenuItem`).
- macOS: `plat::pkg::app_bundle_follows(old, new)`, whether an app bundle's signature satisfies
  the installed bundle's designated requirement, and `plat::pkg::app_bundle_swap(a, b)`, which
  exchanges two folders in one step, for `jflib-update`'s updates of app bundles.
- macOS: VoiceOver. Every node of the accessibility tree is an NSAccessibility element under its
  window (roles and subroles, names, values, states, frames, the focus, the hit test), with press,
  increment and decrement, values, focus and selection, the text attributes of text boxes
  (characters, lines, the insertion point, the selection, the rectangle of a range) and
  notifications of the tree's changes; dialog windows are AXDialog. The first question of an
  assistive technology turns the tree on.
- iOS through UIKit, driven by a host in Rust: one scene with a view inside the safe area and above
  the keyboard, fingers with their coalesced samples, the Apple Pencil as a pen, an iPad's pointer
  as a mouse, the on-screen keyboard (with a bar with Done above it) and a hardware keyboard,
  rotation, a display link that rests while the app is idle, the app's states as `E_FOCUS_*`,
  `E_POWER` and `E_SESSION_END`, and the appearance, reduced motion and darker colours as system
  settings. The clipboard, Open, Save as and folders through the document picker (answering later,
  as on Android), opening URLs, spell checking, notifications (asked the first time; refused:
  `NoteClosed`), the motion sensors and the location (Core Motion, Core Location).
- iOS: VoiceOver. The nodes are UIAccessibility elements of the host's view; lists, tab strips,
  dialogs and menus are containers, with traits, values, double tap, swipes on sliders and the
  Escape gesture. Text boxes and the view are UITextInput, so VoiceOver moves through the text and
  the keyboard sees the text around the caret. The tree follows VoiceOver's and Switch Control's
  switches.
- macOS and iOS: a window's GPU layer (a CAMetalLayer above the CPU's picture) for the Metal
  backend of `jflib-gpu` and the UI's GPU output (`win_native`, `win_gpu_child`,
  `win_gpu_child_show`).

#### ARM64

- Linux on ARM64 (`aarch64-unknown-linux-gnu`): plat as on x86-64. Static programs start and run,
  programs against glibc too; crash reports with x0 .. x15, pc, the kind of access and the call
  chain from the CFI; the page size read at run time (kernels with 4, 16 and 64 KB pages) and
  64 KB guard pages for threads; the seccomp filter of the `sandbox` line; `cpu_brand`.
- Linux on ARM64: plugins and loadable modules, with the crash guard, the ELF loader for AArch64
  and the module host.
- Windows on ARM64 (`aarch64-pc-windows-msvc`): the crash guard, crash reports with x0 .. x15, pc
  and the call chain from the unwind data, `cpu_brand`; the CoreCLR host refuses an x64 runtime
  with a message of its own.
- `plat::LIBE_ARCH`: `lib_open` refuses a plugin or module built for the other processor (x86-64
  on ARM64, ARM64 on x86-64) with this code instead of `LIBE_FORMAT`, on Windows too.

#### Everywhere

- A web page's accessibility tree in `plat::a11y`: `Sub`, a finer role (`Node::role` stays the
  nearest basic role, so a bridge that does not know `Sub` falls back to it); the node fields
  `desc`, `url`, `set`, `item_level`, `cell`, `grid` and `live`; the states `S_REQUIRED` ..
  `S_SELECTABLE`; the actions `A_EXPAND`, `A_STEP`, `A_SCROLL` and `A_SELECT` with `Op`,
  `OpRequest`, `request_op` and `take_ops`; `announce` (`Change::Announce`) and live regions
  (`Change::Live`); `SUB_TEXTS`, `sub_name`, `Node::role_text`; on `WinTree` `grid_size`,
  `cell_at`, `headers`, `headers_of`, `live_root` and `subtree_end`; `check_due()` and `epoch()`.
  The bridges pass it on: UI Automation with control types, landmarks, ARIA roles and properties,
  descriptions, form states, the place in a set and the level, live settings, the Grid, GridItem,
  Table, TableItem, ScrollItem and Selection patterns and notifications; AT-SPI2 with roles,
  attributes, states, Table, TableCell, Hyperlink, Selection, the expand, collapse, increment and
  decrement actions, ScrollTo and announcements; the browser with ARIA roles and attributes and
  live regions (`a11y_wire::VERSION2`, `Mirror::set_version`).
- Web browsers (`browser=1` in the app's `PACKAGE`): `plat::default_browser_status()`
  (`BrowserStatus`: `Default`, `NotDefault`, `Unknown`) and `plat::default_browser_request()`.
  Windows reads the user's choice of `http` and `https` and opens Settings at the app's page of
  the default apps; Linux reads the defaults of `mimeapps.list`, else KDE's `kdeglobals`, and
  writes the app as the default as `xdg-settings` does; macOS asks Launch Services (and macOS asks
  the user). Registering without a setup (`plat::assoc::assoc_register`, which `--register`
  runs) writes, on Windows, the ProgID `<app>URL`, `Software\Clients\StartMenuInternet\<app>`
  with its capabilities and `RegisteredApplications` in `HKCU` instead of keys of `http` and
  `https`, and unregistering removes them; on Linux the `.desktop` file names
  `Network;WebBrowser;`. `plat::assoc`: `package_entries`, `browser_status`, `browser_request`,
  `AssocError::NotRegistered`.
- Print jobs of several pages: `plat::PrintDoc::begin(&job, pages)` shows the system's print
  dialog with a page range, `page()`, `copies()` and `wants(n)` tell what was chosen,
  `add_image(&src)` adds a page, `add_pdf(bytes)` prints the app's own PDF as it is where
  `pdf_native()`, then `end()` or `abort()`. Windows prints page by page through GDI, Linux sends
  one PDF through the portal; macOS, iOS, Android and the browser answer `ERR_UNSUPPORTED`.
- `plat::file_dialog_paths()`: every file a system file dialog with `FD_MULTI` chose. Windows'
  dialog and Linux's portal choose several, the other systems' pickers one.
- More mouse cursors, `CUR_X0` .. `CUR_X_END`: `CUR_NOT_ALLOWED`, `CUR_NO_DROP`, `CUR_GRAB`,
  `CUR_GRABBING`, `CUR_HELP`, `CUR_PROGRESS`, `CUR_ZOOM_IN`, `CUR_ZOOM_OUT`, `CUR_CELL`,
  `CUR_VTEXT`, `CUR_ALIAS`, `CUR_COPY`, `CUR_CONTEXT_MENU`, `CUR_ALL_SCROLL`, `CUR_COL_RESIZE`,
  `CUR_ROW_RESIZE` and the eight `CUR_SIZE_*` edges, on Windows, X11, Wayland, macOS and in the
  browser; `plat::cursor_fallback` gives the nearest shape elsewhere. Shapes a system lacks are
  drawn by plat: on Windows at the system's cursor size for the window's dpi, on X11 at the
  desktop's cursor size (made again when its dpi changes), the same images on Wayland without
  cursor-shape, zoom and cell on macOS; `plat::cursor_probe` for tests.
- Raw key events: `E_KEY_DOWN` and `E_KEY_UP` carry the scancode and the key's location
  (`Event::x`, `Event::y`: `KL_STANDARD`, `KL_LEFT`, `KL_RIGHT`, `KL_NUMPAD`) on Windows, X11 and
  Wayland; `plat::evdev_location`.
- An input method for fields an app draws itself: `plat::ime_purpose` with `IP_TEXT`, `IP_URL`,
  `IP_EMAIL`, `IP_NUMBER`, `IP_PHONE`, `IP_PASSWORD`, `IP_SEARCH` (Windows: no input context for
  passwords, applied again when a window gets the keyboard; Wayland: the content purpose, sent
  again when it changes; X11: the keys of a password field stay away from IBus, as in GTK), and
  `plat::ime_inline`: Windows hides the input method's composition window while the app draws the
  composition itself.
- Drop effects per target: `plat::drag_query_all` (the effect and every effect taken) and
  `plat::drag_decide`. The platform answers a drag over the window with an effect the target
  takes, so the system's pointer shows copy, move, link or no (Windows' `DragOver` and `Drop`, X11's
  `XdndStatus`, Wayland's `wl_data_offer.set_actions`, macOS through `drag_query`).
- `plat::sandbox::denied_syscalls` (Linux): the system calls the seccomp filter refuses with
  `EPERM`.
- The feature `unwind` (with `std`, for a program built with `panic = "unwind"`): a panic that
  leaves plat's threads (the pool, `spawn_blocking`) or the start of the program uncaught still
  ends it with JFLib's crash report and exit code 3, while panics the app catches leave it
  running.
- `vfs::Provider::create_new` (a new file where nothing is, the check and the creation in one step; the default uses the OS exclusive create of `os_path`) and `vfs::Stream::deferred` (bytes reach the provider only by sync or drop)
- `fill_random`, `random_bytes` and `random_u64` on Android (bionic's `arc4random_buf`); the browser has none, which the docs now say (a web page takes `crypto.getRandomValues`)
- `drop_hold(on)`, `drop_waiting()` and `drop_finish(serial, effect)`: a drop's answer to its
  source waits after `E_DROP` until the program gives it, where the platform allows a later
  answer: X11 (`XdndFinished`), Wayland (`wl_data_offer.finish`) and Windows for a source whose
  data object offers `IDataObjectAsyncCapability` in async mode (`EndOperation`).
- Windows: the data object of the app's drags offers `IDataObjectAsyncCapability` (async mode
  off); `dnd_test_drop_async` and `dnd_test_async_end` test the late answer without a user.
- `drop_allowed()` and `drop_set_allowed(effects)`: the effects the source of a drop allows, as
  the platform gives them with the drop (Windows `IDropTarget::Drop`, X11 the `XdndActionList`
  with the last `XdndPosition`'s action, Wayland `source_actions`, macOS the operation mask); 0
  in the browser and on Android.

### Changed

- `plat::dl_sym` and `plat::dl_close` (Linux with glibc, Android) are `unsafe fn`: the dynamic
  linker reads a wrong handle's structures before it could tell.
- `plat::inst::reg_delete_tree` deletes only keys below `Software\Classes` (a ProgID, a URL
  scheme) and below `Software\Clients\StartMenuInternet` (a web browser's key) and returns `false`
  for any other name. It took every name, and an empty one meant all of `HKEY_CURRENT_USER`.
- The calling convention of loadable modules (`plat::modabi`, `ItemFn`) is `rt::plugin_abi!`:
  `extern "win64"` on x86-64 as before, `extern "C"` on ARM64.
- `win_create` with `PW_NOOWNER` after the main window gives a second top-level window on X11 and
  Wayland too, as on Windows (the UI's independent app windows): no `WM_TRANSIENT_FOR`, of type
  normal and not hidden with the main window, or an `xdg_toplevel` without a parent. Before, it
  was refused there and the UI fell back to an owned window.
- Windows: `plat::a11y::active()` turns false again 10 s after the last question of a UI
  Automation client unless a screen reader (Narrator, NVDA, JAWS) or Magnifier runs and listens,
  and is true again with the next question. Before, any client kept it on for the whole run, and
  the tools that always listen on Windows 11 (the touch keyboard's host, vendors' tools) kept the
  tree on in every app.
- Android: the accessibility tree reaches TalkBack in the rich format: tables and trees as
  collections with their cells, headers and places, the place in a set, descriptions, a link's
  address, invalid fields, meters, live regions, announcements, and the further actions (show on
  screen, expand and collapse, increment and decrement, select).
- Presenting a frame copies large dirty rectangles in row bands on the worker pool (from 1 MB on)
  and from 12 MB on with streaming stores, into Windows' DIB section, X11's shared-memory images
  and Wayland's `wl_shm` buffers: a 4K frame takes 2.0 to 2.5 ms instead of 3.6 to 4.0 ms.
  Copies below 1 MB stay on the UI thread.
- Wayland: `wl_data_offer.set_actions` names the effects the drop target under the drag takes
  instead of every effect the source allows.
- Windows: the CoreCLR host's AppDomain is called `JFLib.ClrHost` (was `PaintCraft.PdnHost`).
- `File::create_new` on a mount whose provider has neither `Provider::create_new` nor `os_path` returns None (it was exclusive only against other `create_new` calls of the process; `write_file` or `File::open` of another writer in between was emptied). `log_file` returns false for a file of such a provider (its lines reached the provider only when the log stopped, a crash line never)
- X11: the effects another program's drag allows come from the source's `XdndActionList` and the
  action it asks for (`E_DRAG`, the `XdndStatus` answer); without a list every effect, as before.
- `net::UnixListener::bind` removes only a socket file nobody listens on any more; a file, a folder or a socket that is listened on stays and `bind` fails (AddrInUse; a folder on Windows: Other). It removed whatever was at the path first: a file with its bytes, an empty folder, another listener's socket
- `base::Ini::load_checked` gives None when it cannot tell that the name is free (refused, an I/O error, a folder there) or cannot learn the file's size; only a missing name gives an empty Ini. `file_exists_checked` on a folder mount answers for its OS file (it gave the mount's kind: 0 for an error too)

### Fixed

- The UI thread's state of plat (windows, the clipboard, the file and print dialogs, pen and
  touch, Linux's X11 connection and portals, the objects of AppKit and UIKit) belongs to the first
  thread that uses it: a call on another thread now panics before it touches anything. Before, it
  raced on unsynchronized cells, undefined behaviour from safe code.
- Deadlocks around values dropped under a lock: a channel's `Receiver` drops the queued values
  after releasing the channel's lock (a queued value holding a `Sender` of the same channel hung);
  the `cancelled()` future of a `CancelToken`, `Sleep`, spawned tasks and `spawn_blocking` drop a
  replaced or removed waker outside their lock. `Executor::ui().block_on` takes the wake-ups of
  other threads itself, so a future woken from a worker thread completes instead of waiting
  forever.
- `Job::start` called on two threads at once starts the job once.
- `thr_shutdown` called on several threads at once: one takes the pool down and the others wait
  for it (they could join the same workers and drop semaphores a worker still waited on).
- `plat::sync::RwLock`: a reader count that would wrap (2^32 guards passed to `mem::forget`)
  panics instead of letting a writer in beside a reader; the guard of `plat::sync::Mutex` is
  `Sync` only when the value is.
- Linux: the folder watcher watches the whole tree again after the kernel's event queue
  overflowed, then reports the overflow (new subfolders stayed unwatched).
- Linux with glibc: a thread that joins itself keeps its stack (`Thread::join` unmapped it under
  the running thread).
- Android: host calls on several threads at once no longer share one answer buffer (a call on
  another thread, the audio thread too, could free it while the bridge copied into it).
- X11: a finger that drags a window no longer ends the program with "nested reply wait".
- X11: a drop takes the effect of the last `XdndStatus`; it took copy whatever the source asked
  for.
- UI Automation: the ExpandCollapse pattern had a wrong interface id, so no client got it
  (drop-downs and menus too). AT-SPI2: a read-only text box was reported as editable.
- `plat::a11y::set_active` wakes the UI thread when the flag goes off too: an idle app never
  noticed that a client left.
- Android: the status bar and the navigation bar take the colours at the app's edges with
  matching icons and no system scrim, instead of a fixed light grey around a dark app.
- `plat::audio::ring_check`: when the reader's time ran out, the writer thread waited at the full
  ring and `join()` never returned; the writer stops now and the check fails
  (`ring_check_within` takes a limit of its own).
- The crash pre-hook (the last log lines, the crash line in the log file) neither allocates nor
  waits, so the app's crash hook and exit code 3 follow. It no longer rotates a log file that the
  crash line takes over its size (this hung when the crash happened while the mount table was
  held, and with `keep` 0 it deleted the crash line), it writes the crash line to files of the
  operating system only, and on Linux it gives up a non-blocking stderr that takes nothing for half
  a second instead of waiting for it.
- Ogg Vorbis floor 1 keeps its values in 15 bits as libvorbis does. A codebook entry up to 2^24 overflowed i32 in the next point's interpolation; valid streams decode as before.
- Overflows that only a build with overflow checks saw (a debug build of an app without JFLib's profile aborted): X11's work area with huge edges (Linux), a file time from 2^63 on (Windows).
- `File::create_new` and `AtomicFile`'s sibling on mounts check and create in one step (`Provider::create_new`: `MemFs`, `Overlay`, `Sub`, content://; folder mounts the OS exclusive create): another writer's file made between the check and the open was emptied by `create_new` and taken over by `AtomicFile`, which then removed it on drop or renamed it at commit (also on `DirFs` mounts)
- content:// (`ContentFs`): a folder move to another folder is refused before the file in its way is deleted (it was deleted, then the move refused); an open for writing or a rename the provider refuses keeps the pending journal copy for `recover` (it was dropped before asking); writes of one document with a journal run one after the other (also `recover` and the removal of a copy), so a write that succeeded no longer removed the copy of a later one that failed
- Linux trash on a file system without `RENAME_NOREPLACE`: the fallback looked whether the name in `Trash/files` was free and then renamed, replacing an entry another program made in between; now a file goes by a hard link and the removal of its old name, a folder replaces an empty folder made for it (steps that fail on a taken name); a file system without hard links refuses files
- Windows secrets: two writes of one large entry at once (or a small and a large one) could leave the credential pointing at a DPAPI file the other write had swept; set, get and delete of an entry hold `data://secrets/<hash>.lock` (an exclusive handle, across processes) from the file to the sweep
- Windows logout and shutdown: `WM_QUERYENDSESSION` ended the app already (`UIC_SESSION_END`, then `UIC_EXIT`), also when another app cancelled the logout; and the app saved only after `WM_ENDSESSION` had returned, when Windows may end the process. Now only a confirmed `WM_ENDSESSION` sends it, and the UI takes it while the message is handled (when it comes during the wait for events; elsewhere it waits in the queue)
- An OS path with a NUL character reached the system cut at the NUL: `delete`, `File::open` for writing, `rename`, `fs::write_file_private` and the rest changed the file named by the part before it. `policy::fs_allowed` refuses such a path, also without a manifest, so every file call fails; `fs::symlink` refuses such a target
- Windows: `File::write_at` at offset u64::MAX appended to the file (the OVERLAPPED value for "at the end") and reported success; an offset whose end passes i64::MAX is refused before anything is written, for `read_at` too
- The log file counted a short write as the whole line: the rest was lost, the next line started past the end (a gap of NUL bytes), the rotation came early. Only the bytes written count now, and the rest of the line follows
- content:// (`ContentFs`): a new document the provider named `name (1)` (the name taken meanwhile) took the bytes of `write_file` or a writing `File::open`, which reported success; now that document goes again and the one under the name asked for is written (`mkdir` takes the folder made meanwhile instead of failing and leaving `name (1)`). The chosen folder could be deleted by its document form (`tree/ID/document/ID`)
- `vfs::Sub`: opening its root for writing emptied or created a file of the folder's name in the inner provider and was refused only then; refused first now (`create_new` too)
- `aio::Writer`: a write whose future was dropped while its job ran could overwrite a later write at the same offset, and the commit succeeded. Such a writer is spoilt now: later writes give `Err(Write)`, the commit `Err(Commit)`, nothing is put in place

### Security

- `plat::open_url` and `plat::policy::url_allowed` take only absolute URLs (a scheme of two or more
  characters), with or without a manifest. A path (`C:\x.exe`, `\\server\share\x`, `x.exe`)
  went to ShellExecuteW or Launch Services, which start it as a program.
- In the browser and on Android the `PERMISSIONS` policy treats `\` as a folder separator, as
  their file stores do: a path with `\..\` could pass the rule of one file and open another.
- Ogg Vorbis: `Sound::ogg` refuses streams that decode to more samples than 10 minutes of 48 kHz
  stereo, whatever rate and channels the header names (10 minutes at 384 kHz and 8 channels were
  7 GB, from 110 KB of silent packets), and reserves the samples fallibly (`AudioError::Format`
  instead of an abort). Silent channels skip their residue and a residue's classification buffer
  is only as long as its partitions, so a costly setup no longer makes silence decode 30 times
  slower.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
