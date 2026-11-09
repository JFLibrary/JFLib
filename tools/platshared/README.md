# platshared -- the plat-shared acceptance tools (agent plat-shared, R4)

The xdg-desktop-portal client and the monitor profile of `lib/plat` against the assembly
reference, and fake buses for the other Linux services (Rust only).  Nothing here opens a real
dialog or touches the user's clipboard.  (The print, clipboard and plugin-loader comparisons went
with PaintCraft to its own repository, JFLibrary/JFPainter, in 2026-11.)

The tools need a copy of the reference build inside the checkout they run in (WSL paths are
repo-relative):

    mkdir -p build/rwref/win build/rwref/linux
    copy <ref>/win/{gfxtest,codectest}.exe  -> build/rwref/win/
    copy <ref>/linux/{gfxtest,codectest}    -> build/rwref/linux/
    python build_rs.py app plattest --target both
    python tools/platshared/accept.py

| Tool | What |
|---|---|
| `accept.py` | `portal_bus.py` and `icc_x11.py` + the monitor profile (plattest --test icc against codectest op 325); the print, clipboard and plugin-loader comparisons imported PaintCraft's check scripts and went with the app (JFLibrary/JFPainter, 2026-11) |
| `icc_x11.py` | (inside WSL) a raw X11 client sets / restores the root window's _ICC_PROFILE around plattest --test iccx11 (valid, invalid, above 1 MB, none) |
| `portal_bus.py` | (inside WSL) fake session bus: OpenDirectory / OpenURI (gfxtest --open) byte for byte; FileChooser and OpenURI of plattest --test portal checked |
| `tray_bus.py` | (inside WSL, Rust only) fake session bus playing StatusNotifierWatcher, a panel and org.freedesktop.Notifications against plattest --test tray-bus: item properties, dbusmenu layout and clicks, Notify, ActionInvoked |
| `settings_bus.py` | (inside WSL, Rust only) fake settings portal (ReadAll: appearance, GNOME interface, a11y, mouse, wm.preferences) against plattest --test settings-bus: theme, accent (ddd), contrast, motion, text scale (d), blink and double-click times, font smoothing / hinting / subpixel order, the title bar's button-layout and click actions; a second run without appearance keys against --test settings-bus-files: GTK's settings.ini and KDE's kdeglobals in a scratch XDG_CONFIG_HOME fill in |
| `life_bus.py` | (inside WSL, Rust only) fake system bus (logind PrepareForSleep) and session bus (settings portal SettingChanged, a connection per client) against plattest --test life-bus: E_POWER, E_SETTINGS (SET_THEME, SET_FONTS, SET_OTHER for button-layout), a kdeglobals renamed into place in a scratch XDG_CONFIG_HOME (inotify: one E_SETTINGS), then SIGTERM: E_SESSION_END |
| `atspi_bus.py` | (inside WSL, Rust only, a display: WSLg) fake session and accessibility bus playing org.a11y.Bus, the AT-SPI2 registry and a screen reader against plattest --test a11y-bus: the tree (roles, names, states, interfaces, parents, extents, Value, Text), DoAction / CurrentValue / SetTextContents / GrabFocus and their events, the caret as screen magnifiers follow it (TextCaretMoved on focus, GetCharacterExtents, SetCaretOffset / SetSelection); `--real` repeats it with at-spi2-core (dbus-run-session, at-spi-bus-launcher, registryd) through libatspi |
| `docs_bus.py` | (inside WSL, Rust only) the documents portal against plattest --test docs-bus: a fake (GetMountPoint, Add with an O_PATH descriptor through SCM_RIGHTS, reuse_existing / persistent, one bus connection only for document paths, a refusal); `--real` in a private dbus-run-session with scratch XDG_RUNTIME_DIR / XDG_DATA_HOME: xdg-document-portal's own answers, a lasting and a session document across a restart of the portal, and plattest in a bwrap sandbox (a /.flatpak-info) choosing a file through xdg-desktop-portal's file chooser (a fake chooser behind it), read with fs.user only and read / written again after the restart; the portals and the FUSE mount are removed |
| `secret_bus.py` | (inside WSL, Rust only, needs python3-cryptography) fake session bus playing org.freedesktop.secrets against plattest --test secret-bus: DH sessions (dh-ietf1024-sha256-aes128-cbc-pkcs7) and plain, CreateItem / SearchItems / GetSecret / Delete, another app's prefix, 64 KiB values, a locked collection with Unlock prompts, dismissed prompts, no default collection (CreateCollection), an invalid public key, no Secret Service |
| `desktop_bus.py` | (inside WSL, Rust only) fake session bus for the desktop integration: plattest --test power-bus (the Inhibit portal with flags 4 / 12, a backend that refuses suspend and accepts idle alone, ScreenSaver / PowerManagement cookies without a portal, nothing; Request.Close / UnInhibit, the connection closed with the last guard), --test taskbar-bus (com.canonical.Unity.LauncherEntry Update signals: progress (d), count (x), visibility; Query answered), --test autostart-bus (Background.RequestBackground with autostart / commandline, Response granted / refused, no portal) |
| `dnd_x11.py` | (inside WSL, Rust only, a display: WSLg) a raw X11 client plays another program against plattest --test dnd-x11: XDND drops into a window (XdndTypeList, XdndStatus per point, URI list, UTF-8 text, HTML, a 300 KB image through INCR, leave) and plattest's own drags onto the client's window (XdndEnter / Position / Drop, its content through ConvertSelection, XdndFinished, Esc cancels); `--app` repeats it with the real dndtest window for a few seconds (another program's text onto a drop target, a button dragged out, a fruit out and back with the app's own kind) |
| `ibus_bus.py` | (inside WSL, Rust only) fake session bus playing org.freedesktop.portal.IBus and an input method against plattest --test ibus-bus / ibus-none / ibus-x11: input context, focus, caret, ProcessKeyEvent, preedit, commit, forwarded keys, no answer, no input method; the X11 glue with synthetic X events (WSLg) |

Work files go to `build/plat-shared/`.
