# jflib-plat

The platform layer of [JFLib](https://jflib.org). Everything that touches the operating system
goes through this crate: windows and input, files, threads, the network, sound, the system's
services. Each operating system has its own module behind the same functions, so the crates above
it, and an app, are written once and run on Windows, Linux, macOS, iOS, Android and in the
browser.

It does this without the standard library and without bindings of other authors. On Windows it
calls the Win32 API, on Linux it makes the kernel's system calls itself and speaks the X11,
Wayland, D-Bus and PulseAudio protocols on its own, on macOS and iOS it goes through libSystem
and the Objective-C runtime, and in the browser and on Android it talks to a small host that runs
the app. plat is one of the few JFLib crates that contain `unsafe` code, so that the crates above
it need none.

## Do you need this crate?

It comes along with everything else: JFLib's UI is built on it, and an app reaches it as
`jflib::plat` for what the UI does not wrap, such as files and the app's folders, threads and
async tasks, the network, processes, secrets, sound and logging. For desktop features the UI has
its own calls on top (`ui.recent_add`, `ui.hotkey_register`, `ui.monitors`, ...); plat is the
layer underneath. The window functions of plat are what the UI runtime uses; an app works with
the windows of `ui`.

## Windows, input and the screen

plat opens windows and popups, presents the pixels the UI drew, and turns what the system
reports into one stream of events: mouse, keyboard with input methods, pen and touch, resizing,
focus, the system going to sleep or the session ending. Around that come the clipboard (text,
HTML, RTF, images), drag and drop with other programs, the system's file dialogs, mouse cursors,
the monitors with their scale, the system's settings the UI follows (dark or light, accent colour,
high contrast, reduced motion, text scale) and the user's languages. For `jflib-gpu` it provides
the surface of a window to draw into.

## Files, the app's folders and permissions

Every file function takes paths of the operating system and names with a scheme: `app://` for
the files built into the program, `config://`, `data://`, `cache://` and `logs://` for the app's
own folders on each system, `documents://` and the other user folders, `mem://` for files in
memory, and folders or archives an app mounts itself. `write_file` writes a sibling and puts it
in place in one step, so a crash never leaves half a file.

```rust
// the app's settings, in its own folder on every system
let old = plat::read_file("config://settings.ini");               // None: missing, or not allowed
let ok = plat::write_file("config://settings.ini", b"zoom=150\n"); // false: not written
```

`plat::aio` does the same without blocking: reads, atomic writes and copies in chunks, with
progress and cancellation. Files can go to the trash instead of being deleted, chosen files enter
the system's list of recent files, and `plat::watch` reports changes below folders as they happen.
The same names work in the browser and on Android, where a document the user picks in the
system's picker is a `content://` name.

An app can declare what it may touch in a `PERMISSIONS` file next to its `Cargo.toml`: which
files and folders, programs, URL schemes, network addresses, the secret store, sound, global
shortcuts, pictures of the screen, sensors and the location. plat then refuses everything else
that goes through it, whoever asks: the app's own code, JFLib's crates, plugins. A rule can also
ask the user at the first access. With the line `sandbox` the operating system enforces the
manifest as well (Landlock and seccomp on Linux, process mitigation policies on Windows), which
also confines code that does not go through plat. Apps without the file may do everything.

## Threads and async tasks

A worker pool runs `parallel_for` loops and background `Job`s, and `plat::wake()` brings the UI
thread back when work is done. `plat::task` runs async code: in the app's event loop (the UI's
`ui.spawn`), or with `block_on` in a console program, with a pool for blocking calls, timers,
cancellation and progress. `plat::sync` has a mutex, a condition variable, a reader-writer lock
and channels.

```rust
use plat::aio::{self, Ctl};

// a console program runs the same async code the UI runs with ui.spawn
let ctl = Ctl::new(); // progress in bytes and a cancel token, shared with the copy
let copied = plat::task::block_on(aio::copy_file("documents://a.bin", "documents://b.bin", &ctl));
```

The windows, the clipboard and the dialogs belong to the UI thread, the first thread that uses
them; a call from another thread panics instead of racing, so such work goes to the UI thread
(`plat::wake()` and a channel). On Android and in a browser without shared memory there are no
worker threads: there the pool's jobs run between frames.

## Network, processes and services

On Windows, Linux, macOS and iOS plat has TCP, UDP and Unix domain sockets for IPv4 and IPv6 with
timeouts, name lookups, and the system's trusted certificate authorities, on which `jflib-netio`
builds HTTP and TLS. Child processes start without a shell, with pipes, and dropping one ends its
whole process tree. A program can run as a system service (the Windows service control manager,
systemd, launchd) and keep one instance per user that later starts hand their arguments to; on
Windows and Linux it can start a helper with administrator rights after the system's prompt.

## The system around the app

An app can show a symbol in the notification area or the menu bar with its own menu, send system
notifications, show progress and a badge on its taskbar or Dock button, start at login, register
global shortcuts, keep the computer awake during a long job, register its file types and URL
schemes without a setup, and, as a web browser, ask whether it is the default browser. plat also
reaches the system's spell checker, printing, opening a URL in the right program, the colour of a
pixel anywhere on the screen and pictures of the screen. Each of these uses what the system has
for it, from the Windows taskbar and the desktop portals of Linux to the Dock of macOS.

## Accessibility

The UI hands plat a tree of everything on screen, and plat serves it to the system's screen
readers: UI Automation on Windows, AT-SPI2 on Linux, VoiceOver on macOS and iOS, TalkBack on
Android and an ARIA tree in the browser. The bridges start by themselves; where the system can
tell, the tree is built only while a client listens; and the actions of a screen reader (press,
change a value, move the focus, expand, scroll) come back to the app.

## Sound, sensors and secrets

`plat::audio` plays streams the app fills, short sounds mixed together (WAV and Ogg Vorbis) and
the system's own sounds, on Windows, Linux, Android and in the browser; macOS and iOS have no
sound yet. `plat::sensors` reads the motion sensors (accelerometer, gyroscope, magnetic field,
the device's rotation) and the location on Android, iOS and in the browser. `plat::secret` keeps
passwords, tokens and keys in the system's store (the Credential Manager, the Secret Service, the
keychain, the Android Keystore, a WebCrypto key in the browser) and never falls back to a plain
file.

## Crashes and logging

A hardware fault and a panic end the same way: a report on stderr that says what happened and
where, then the app's crash hook (`plat::crash_hook`), which gets the details with the call chain,
then exit code 3. An app that logs also gets the newest lines of its log in that report and a
crash line in its log file. The crash path neither allocates nor waits, so a crash while the
allocator or the log is held still gets its report. `plat::log!` writes lines with levels to
stderr, to a ring that keeps the newest ones for such a report or a "copy log" button, and to a
log file that rotates.

## Where it runs

- **Windows**, x86-64 and ARM64: the Win32 API; other system libraries are loaded at run time.
- **Linux**, x86-64 and ARM64: Wayland or X11, whichever the session offers, and the desktop
  portals, also inside Flatpak. A program is one static file; one that uses the GPU links against
  glibc, because the GPU drivers are glibc libraries.
- **macOS** on Apple silicon: AppKit, with the app's menus in the system's menu bar.
- **iOS**, devices and the simulator: UIKit, driven by a host in Rust.
- **Android**: the app runs in an activity that `cargo jflib android` builds around it.
- **The browser** (wasm32): a JavaScript host runs the app in a web worker and shows its windows
  in a canvas.

In the browser and on Android plat has no sockets, child processes, services, `plat::sync`,
`plat::fs`, folder watching or random numbers of its own; native plugins load on Windows and
Linux only.

## How to get it

Through the one dependency [`jflib`](https://crates.io/crates/jflib), as `jflib::plat`, on every
target. jflib's features `std` (an app that uses crates with the standard library) and `unwind`
(panics an app catches leave it running) turn on the features of the same names here. All of
JFLib's crates share one version and are released together.

## What's new in 0.2.0

plat runs natively on macOS and iOS now: windows through AppKit and UIKit, the clipboard, the
system's file dialogs, notifications, the keychain and VoiceOver on both, and on the Mac also drag
and drop, the tray, the Dock, global shortcuts and services through launchd. Linux and Windows run
on ARM64 as well, with crash reports that show the call chain and, on Linux, plugins and loadable
modules. The accessibility tree carries what a web page needs (tables, landmarks, live regions,
more actions) to UI Automation, AT-SPI2, TalkBack and the browser. New are the registration of an
app as a web browser with its default status, print jobs of several pages, several files from one
file dialog, more mouse cursors, raw key events, an input method for fields an app draws itself
and drop effects per target, and large frames reach the screen faster. Among the fixes:
`open_url` takes only absolute URLs (a path could start a program), the UI thread's state panics
on other threads instead of racing, deadlocks in channels and async tasks and races in the worker
pool are gone, and the crash report no longer allocates or waits. `dl_sym` and `dl_close` are `unsafe fn` now.
Everything is in the [changelog](https://docs.rs/crate/jflib-plat/latest/source/CHANGELOG.md).

## Documentation

- JFLib's guide on [files](https://docs.rs/jflib/latest/jflib/guide/index.html#8-files-settings-images),
  [threads and async tasks](https://docs.rs/jflib/latest/jflib/guide/index.html#9-threads),
  [network programs](https://docs.rs/jflib/latest/jflib/guide/index.html#17-network-programs),
  [accessibility](https://docs.rs/jflib/latest/jflib/guide/index.html#19-accessibility-screen-readers),
  [secrets](https://docs.rs/jflib/latest/jflib/guide/index.html#20-secrets-passwords-tokens-keys),
  [desktop integration](https://docs.rs/jflib/latest/jflib/guide/index.html#21-desktop-integration-keeping-awake-autostart-taskbar-file-types-recent-files-monitors-global-shortcuts-the-screen)
  and [sound](https://docs.rs/jflib/latest/jflib/guide/index.html#33-sound-streams-short-sounds-the-systems-sounds)
- [App permissions](https://github.com/JFLibrary/JFLib/blob/main/docs/PERMISSIONS.md) and
  [the file namespace](https://github.com/JFLibrary/JFLib/blob/main/docs/VFS.md)
- [API reference](https://github.com/JFLibrary/JFLib/blob/main/docs/API.md#plat): every public
  item of `plat`, also on [docs.rs](https://docs.rs/jflib-plat)

## License

MIT or Apache-2.0, at your choice.
