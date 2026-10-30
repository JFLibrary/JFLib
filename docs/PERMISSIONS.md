# App permissions (`apps/NAME/PERMISSIONS`)

An app can declare which files, programs, URL schemes, plugins, printers and network
addresses it uses, whether it keeps secrets in the system's store, whether it may start
itself at login, whether it plays sound, whether it may register global shortcuts and whether it
may take pictures of the screen. The
platform layer then refuses everything else, in every part of the process: the app's own
code, the library, codecs, plugins that go through the host API, scripts and agent actions.
Apps without a `PERMISSIONS` file keep the old behaviour and may do everything.

A rule can also ask the user at the first access ("permprompt wants to write files"), with
the answer kept for this run or for good and revocable in the settings ([Asking the
user](#asking-the-user)). Native plugins declare their own needs, and what `plat` does for
them needs their rights and the app's ([Plugins](#plugins)). Agent actions can need the
user's confirmation or be closed to agents ([Agent control](#agent-control)).

The model resembles Tauri's capabilities or OpenBSD's `unveil`. The app states what it
needs. The build checks the syntax. `ui::main!` and `rt::main!` install the rules before the
program's own code runs (`App::init`, the body of a console program), in every program of the
app: the GUI program, the further programs of `apps/NAME/BINS` and the Android APK.
From then on the rules can only grow by files the user hands over.

| Part | Where |
| --- | --- |
| Rules, path normalisation, checks, denials | [`lib/plat/policy.rs`](../lib/plat/policy.rs) |
| Checked file functions (`read_file`, `File::open`, ...) | [`lib/plat/guarded.rs`](../lib/plat/guarded.rs), [`lib/plat/fs.rs`](../lib/plat/fs.rs) |
| Checked sockets and name lookups | [`lib/plat/net.rs`](../lib/plat/net.rs), [`lib/netio/dns.rs`](../lib/netio/dns.rs) |
| Checked secret store | [`lib/plat/secret.rs`](../lib/plat/secret.rs) ([SECRETS.md](SECRETS.md)) |
| Checked autostart entry | [`lib/plat/autostart.rs`](../lib/plat/autostart.rs) |
| Checked sound output (streams, `play`, the system's sounds; `ui.beep`, `ui.dialog_sound`) | [`lib/plat/audio.rs`](../lib/plat/audio.rs), [`lib/ui/sound.rs`](../lib/ui/sound.rs) |
| Checked global shortcuts and pictures of the screen | [`lib/plat/hotkey.rs`](../lib/plat/hotkey.rs), [`lib/plat/screen.rs`](../lib/plat/screen.rs) |
| Install at start-up, the command line's paths | [`lib/plat/boot.rs`](../lib/plat/boot.rs) (console programs reach it through [`rt::boot`](../lib/rt/boot.rs) and `rt::main!`) |
| OS sandbox from the manifest (`sandbox` line) | [`lib/plat/sandbox.rs`](../lib/plat/sandbox.rs), [`lib/plat/linux_sandbox.rs`](../lib/plat/linux_sandbox.rs) (Landlock + seccomp), [`lib/plat/win_sandbox.rs`](../lib/plat/win_sandbox.rs) (mitigation policies) |
| Grants for the user's files, trusted library code, the question dialog, `ui.permissions_page` | [`lib/ui/perm.rs`](../lib/ui/perm.rs) |
| Questions, answers, plugin scopes, agent rules | [`lib/plat/policy.rs`](../lib/plat/policy.rs) (`questions`, `Key::answer`, `decisions`, `forget`, `plugin_rights`, `on_behalf`, `agent_rule`, `agent_ask`) |
| Agent actions: `Access`, `forbidden`, `confirmation_required` | [`lib/agent_core/src/action.rs`](../lib/agent_core/src/action.rs), the UI's host [`lib/ui/agent.rs`](../lib/ui/agent.rs) |
| Script commands `perm ...`, `agent ...` | [`lib/ui/auto_perm.rs`](../lib/ui/auto_perm.rs) |
| Syntax check, hand-over to `rustc` | `app_permissions`, `perm_split`, `Builder.boot_inputs` / `boot_args` in [`build_rs.py`](../build_rs.py), `build_apk` in [`tools/android.py`](../tools/android.py) |
| Tests | `plattest --test policy` ([`t_policy.rs`](../apps/plattest/t_policy.rs)), `--test permask` ([`t_permask.rs`](../apps/plattest/t_permask.rs): questions without the UI), `--test plugperm` ([`t_plugperm.rs`](../apps/plattest/t_plugperm.rs): plugins), `--test dirs` and `--test trash` (the user folders' variables, `trash` as a write), [`apps/permtest`](../apps/permtest/main.rs) (GUI program; sound refused without `audio.output`), [`apps/permcli`](../apps/permcli/src/main.rs) (`rt::main!` and the programs of `BINS`), [`apps/permprompt`](../apps/permprompt/main.rs) (the dialog, each answer, a worker thread, headless answers, the stored answers, the settings section, agent actions; `tests/sound.uis`: `ask audio.output`, `ui.beep` and the silent dialog sound), [`apps/audiotest`](../apps/audiotest/PERMISSIONS) (`audio.output`: plays), [`apps/sandboxtest`](../apps/sandboxtest/main.rs) (the OS sandbox: raw syscalls that the kernel blocks or allows) |

## The manifest

One rule per line; `#` starts a comment; a key may appear more than once.

```text
# apps/MyTool/PERMISSIONS
fs.read  $EXE/templates/**            # read below a folder
fs.write $CONFIG/MyTool/**            # write (and read) below a folder
fs.write $TEMP/mytool-*/**
fs.write $DOCUMENTS/MyTool/**         # documents://MyTool/...
fs.read  $PICTURES/**                 # pictures://, read only
fs.user                               # files the user chooses: read and write
process  gmic  $EXE/helpers/*.exe     # programs Command::spawn / run_elevated may start
url      https mailto                 # schemes for open_url ("*" = all)
plugins  $EXE/plugins/*.dll $EXE/plugins/*.so
print                                 # plat::print
secrets                               # plat::secret (Credential Manager, Secret Service, Keystore)
autostart                             # plat::autostart_set (start at login)
audio.output                          # plat::audio, ui.beep (sound)
input.global                          # plat::hotkey_register (global shortcuts)
screen.capture                        # plat::screen::capture_start (pictures of the screen)
net.connect api.example.com:443 *.cdn.example.com:443
net.connect 192.168.0.0/16:631 localhost:8000-8099
net.listen  127.0.0.1:0               # a local server on a free port
ask fs.read $DOCUMENTS/** "Briefe öffnen"   # asks at the first access, the reason is shown
ask net.connect *:443                 # one question for the whole line
ask url mailto
agent.confirm document.save document.export.*   # an agent needs the user's OK
agent.deny document.delete ui.render  # never for an agent
sandbox                               # let the OS enforce the manifest too (OS sandbox)
```

| Key | Arguments | Allows |
| --- | --- | --- |
| `fs.read` | patterns | reading files and listing folders below the patterns; `plat::recent_add` takes only such files (and the user's, `fs.user`) |
| `fs.write` | patterns | creating, writing, renaming, deleting and `plat::trash` below the patterns (includes reading) |
| `fs.user` | – | the files of the file dialog, of drops and of the command line: read and write |
| `process` | patterns or program names | `plat::process::Command::spawn`, `launch` and `run_elevated`; a name without `/` matches a program looked up in `PATH` (the live inspector's "Im Editor öffnen" in development builds starts the editor through `launch`: the same rule, no exception -- GUIDE.md §30) |
| `url` | schemes or `*` | `plat::open_url` (`https`, `mailto`, ...) |
| `plugins` | patterns | `plat::lib_open` (native plugin libraries) |
| `print` | – | `plat::print` |
| `secrets` | – | `plat::secret`: the app's own entries in the system's secret store ([SECRETS.md](SECRETS.md)) |
| `autostart` | – | `plat::autostart_set`: the app's own entry that starts it at login (Run key, XDG autostart, Background portal); `autostart_get` / `autostart_entry` need nothing |
| `audio.output` | – | `plat::audio`: streams (`Stream::open`, `open_fill`), `play`, `play_file` and the system's sounds (`sound`), so also `ui.beep` and `ui.dialog_sound`; without it they are refused and `available()` is `false`. Only apps that play sound list it: a Flatpak gets the sound server's socket for it, which also allows recording ([Packages](#limits)) |
| `input.global` | – | `plat::hotkey_register` / `ui.hotkey_register`: global shortcuts, chords the app hears while another program has the keyboard (RegisterHotKey, XGrabKey, the GlobalShortcuts portal); the app learns only that its own chord was pressed, never other keys |
| `screen.capture` | – | `plat::screen::capture_start` / `ui.screenshot`: pictures of the whole screen or a part of it (BitBlt, GetImage, the screenshot portal). Picking one pixel's colour (`ui.pick_color`, the colour picker's pipette) needs no right: the user chooses the pixel, as a file in the file dialog |
| `net.connect` | `HOST:PORT` | TCP connections and UDP datagrams (`TcpStream::connect`, `UdpSocket::send_to`, and everything built on them: `netio` HTTP, TLS, SMTP, SSH) |
| `net.listen` | `ADDR:PORT` | `TcpListener::bind` and UDP sockets with a fixed port |
| `ask KEY ...` | a rule of any key but `fs.user`, then an optional `"reason"` | the same access after the user agreed ([Asking the user](#asking-the-user)) |
| `agent.confirm` | action names (`*`, `?`) | external control runs these actions after the user's confirmation |
| `agent.deny` | action names (`*`, `?`) | external control may not run these actions |
| `sandbox` | nothing or `strict` | have the OS enforce the manifest as defence in depth ([OS sandbox](#os-sandbox)); `strict` refuses to start without it |
| `recent` | – | a Flatpak reaches the host's list of recent files: the package gets `--filesystem=xdg-data/recently-used.xbel` ([Flatpak](#flatpak)); that file lists the recent files of every program. Nothing changes at run time: the system's list is always allowed for files the app may read (below); it cannot be asked for |

### Patterns

- A pattern is an absolute path or starts with a variable: `$CONFIG` (`config_dir()`),
  `$TEMP` (`temp_dir()`), `$HOME`, `$EXE` (the folder of the executable), `$APPIMAGE` (the
  folder of the running AppImage file: an AppImage's program runs from the copy its runtime
  unpacked, which is `$EXE`; `lib/update` replaces the file in this folder; no value outside
  an AppImage), `$FONTS` (every system font folder), the app's own folders `$APPCONFIG`, `$APPDATA`, `$APPCACHE` and
  `$APPLOG` (`config://`, `data://`, `cache://`, `logs://`; [VFS.md](VFS.md)), or the user's
  folders `$DESKTOP`, `$DOCUMENTS`, `$DOWNLOADS`, `$PICTURES`, `$MUSIC` and `$VIDEOS`
  (`plat::user_dir`: the Known Folders on Windows, the XDG user dirs on Linux, also when the
  user renamed or moved them; `documents://` ...). A variable without a value on this
  machine drops the pattern.
- `*` and `?` match inside one path segment. A `**` segment matches any number of
  segments. `dir/**` also matches `dir` itself. `**` alone matches every path.
- `X:/...` and `//server/share/...` are Windows paths; `/...` is a Linux path. Each OS
  ignores the other's absolute patterns, so one manifest serves both.
- Windows compares without case.

### Network rules

- `HOST` is a DNS name (`api.example.com`), `*.example.com` for every name below
  `example.com` (not the name itself), an IPv4 address, an IPv6 address in brackets
  (`[2001:db8::1]`), an address block (`10.0.0.0/8`, `[fd00::]/8`), `localhost` (127.0.0.0/8 and
  `::1`) or `*` (every host).
- `PORT` is a number, a range (`8000-8099`) or `*`.
- `net.listen` takes addresses, blocks, `localhost` or `*`, not names. `0.0.0.0` and `[::]`
  mean every interface and must be listed as such. Port 0 (any free port) needs `0` or `*`.
  A UDP socket on port 0, which is a client's socket, needs no rule.
- Names are checked when they are looked up: `netio::dns::resolve` on Linux and
  `plat::net::resolve` on Windows. A name that no `net.connect` rule covers fails with
  `NetErrorKind::Denied` before any DNS query is sent. Lookups of names that are not allowed
  therefore cannot carry data out. The addresses of a permitted name are remembered with the ports
  of its rules, so the following `TcpStream::connect` to such an address is allowed.
- As soon as a rule names a host (or `*`), port 53 is open for the DNS queries of `netio`.
- Unix domain sockets (`UnixListener::bind`, `connect_unix`) are files. They need `fs.write` on
  the socket path.

`python build_rs.py app NAME` rejects unknown keys, relative paths, unknown variables, bad
URL schemes and bad `HOST:PORT` values with the file and line number, as well as `ask` before
`fs.user` or an agent line, a reason without `ask`, a reason without its closing quote or with
text after it, and action names with other characters than letters, digits, `.`, `_`, `-`,
`*` and `?`. The app is rebuilt when the manifest changes.

## Every program of the app

The build hands the manifest to every program it builds for the app, together with the app's
name for its folders (`JFGUI_APP`) and its assets (`app://`):

| Program | Started by | Manifest |
| --- | --- | --- |
| the GUI program (`src/main.rs`) | `ui::main!` (`ui::boot`) | `apps/NAME/PERMISSIONS` |
| a console program (`rt::main!`, also an app's main program like `coretest`) | `rt::main!` → `rt::boot::start` → `plat::boot::start` | `apps/NAME/PERMISSIONS` |
| a program of `apps/NAME/BINS` (`cli` or `gui`) | `rt::main!` or `ui::main!` | `apps/NAME/PERMISSIONS.<bin>` if it exists, else `apps/NAME/PERMISSIONS` |
| the Android APK (`--target android`) | `ui::main!` | `apps/NAME/PERMISSIONS` |

```text
# apps/BackupTool/PERMISSIONS.bt-agent   only this program of BINS; the others share PERMISSIONS
fs.read  $HOME/**
net.connect backup.example.org:8443
```

- `rt` knows nothing of `plat`: the process entry of `plat` registers `plat::boot::start`
  with `rt::boot::set_handler`, and `rt::main!` calls it before the program's body. A program
  with a manifest but without that entry ends with exit code 2.
- A console program drops the key after start-up (`plat::policy::Key`): its code cannot widen
  its own rules. With `fs.user` the paths on its command line are the user's, as in a GUI
  program (`--name=value` counts with its value).
- A `PERMISSIONS.<bin>` must name a program of `BINS`; the build refuses a stray one.

## What is always allowed

- Reading the system font folders (`plat::policy::font_dirs`, the folders fontdb scans; on
  Linux also the folders fontconfig's files name and, in Flatpak, the host's fonts under
  `/run/host`), fontconfig's files (`/etc/fonts`, `~/.config/fontconfig`: aliases and the
  fonts per language), the Hunspell dictionary folders (`plat::spell::hunspell_dirs`:
  `DICPATH`, on Linux `~/.local/share/hunspell` and `hunspell`, `myspell`, `myspell/dicts`
  below the XDG data folders; the spell checking of `TB_SPELL`), the hyphenation patterns
  (`plat::spell::hyphen_dirs`: on Linux `hyphen` below the XDG data folders) and the
  executable's folder (assets, modules).
- Reading and writing the app's own folders (`config://`, `data://`, `cache://`, `logs://`,
  see [VFS.md](VFS.md)), and the bundled `app://` assets and `mem://`, which touch no OS
  files. The user's folders (`documents://` ...) are not included; they need rules.
- The trash folders themselves when `plat::trash` moves a permitted item there (the
  recycle bin, `~/.local/share/Trash`, `.Trash-$uid`). `trash` checks the item as a write.
- The system's list of recent files (`plat::recent_add`: Windows' recent items,
  `~/.local/share/recently-used.xbel`) for a file the app may read; the file is checked as a
  read. A Flatpak reaches the host's list only with the line `recent`.
- Registering the app's file types and URL schemes (`--register`, `plat::assoc`): only what
  `PACKAGE` declares, in the user's part of the system (`HKCU\Software\Classes`,
  `~/.local/share/applications`, `~/.local/share/mime`, `~/.config/mimeapps.list`), and only
  when the user starts it or the app asks for it; defaults of other programs only with
  consent.
- On Linux, reading `/etc/resolv.conf` and `/etc/hosts` (the resolver of `netio`).
- Anonymous temporary files: `File::open_temp(None)`.
- `spawn_self`: a second instance of the same program, with the same manifest.
- The platform layer's own system files: the X11 cookie, ICC profiles, CA bundles, the time
  zone, `PATH` lookups, the portal's and the print dialog's results. These calls use the
  unchecked `*_raw` functions inside `plat`.

## The user's files

With `fs.user` the UI runtime grants read and write access to:

- the path the file dialog returns (system dialog or the library's own),
- every path of a drop (`UIC_DROP`),
- the command-line arguments that are not options (`--name=value` counts with its value),
  for example the file of "Open with",
- the files of later starts that a single instance receives (`AF_SINGLE_INSTANCE`, `UIC_OPEN`).

A granted folder includes everything below it. Without `fs.user` the user can still choose
files, but the app cannot open them.

Documents the system hands over one at a time are allowed with `fs.user` without a grant, so
they open again after a restart (a recent-files list): Android's `content://` names, inside a
Flatpak sandbox everything below the documents portal's mount (`$XDG_RUNTIME_DIR/doc/`: the
sandbox shows only what the user gave the app), and the browser's `/handles/` files (one
handle of the user's own file each). Outside a sandbox the portal's mount is an ordinary path.

The library's own file dialog lists folders without checks, because the user is navigating.
`--shot` and the file commands of `.uis` scripts (`shot`, `dump`, `clip file` ...) are also
unchecked: they come from whoever started the process. The key (`plat::policy::Key`) that allows this stays
inside the `ui` crate. App code cannot widen its own rules.

## Asking the user

`ask` before a rule turns it into a question at the first access: "The app wants to read
files: `C:/Users/me/Documents/letter.odt`". The rule's syntax is unchanged; a reason in
double quotes may end the line and is shown in the dialog (through the app's catalogue, so a
translated reason is a row of `i18n::set_catalog`; at most 200 bytes, `#` inside the quotes is
text):

```text
ask fs.read  $DOCUMENTS/**  "Briefe öffnen"
ask fs.write $DOCUMENTS/Exports/**
ask net.connect api.example.com:443 *.cdn.example.com:443   "Wechselkurse laden"
ask url mailto
ask print
```

Every key but `fs.user` can be asked for (`fs.user` already is the user's choice).
`ask audio.output` asks when the app plays a sound itself (`ui.beep`, a stream, `play`); the
sound of a dialog that opens (`ui.dialog_sound`) never asks: it stays silent until the line
is allowed (`policy::audio_granted`, no question, no `PERMDENY`).

- **One decision per line.** The answer decides the whole line: allowing
  `ask fs.read $DOCUMENTS/**` once allows every file below `$DOCUMENTS` for the rest of the
  run. Lines are asked separately, so `ask url mailto` and `ask url https` on two lines are
  two questions. A plain rule that covers the access wins: no question is asked.
- **The dialog.** The library shows a content dialog over the main window
  ([`lib/ui/perm.rs`](../lib/ui/perm.rs)): what the access does ("Dateien lesen",
  "Eine Netzwerkverbindung aufbauen", ...), the path, host, program, URL or action, the plugin
  it is made for, the reason and the line. Its answers:

  | Button | Answer (`plat::policy::Answer`) | Kept |
  | --- | --- | --- |
  | Einmal erlauben | `Once` | until the program ends |
  | Immer erlauben | `Always` | stored, also in later runs |
  | Ablehnen (also Esc) | `Deny` | until the program ends |
  | Nie erlauben | `Never` | stored, also in later runs |

- **Which thread waits.** The check happens in `plat`, on whatever thread makes the call.
  The UI thread cannot wait for its own dialog: an access it makes while the question is open
  is refused (`policy::Verdict::Pending` inside, the usual refusal outside, `PERMASK` on
  stderr), and the answer reaches the app as the command `UIC_PERMISSION` (arg 1 allowed,
  0 refused), so it can try again. A thread of the pool or one of its own waits for the
  answer of the dialog, at most 60 s (`jfgui_PERM_TIMEOUT` in ms, the scripts' `perm
  timeout`); after that it is refused while the question stays open (the answer still counts
  for later accesses). When the program ends, waiting threads are refused. In the browser and
  on Android there is only the UI thread.
- **Nobody asks** in headless runs (`--shot`, `--script`, `--agent-json`, `--agent-mcp`) and
  in console programs (`rt::main!`): an ask line refuses there, without a question. Scripts
  answer deterministically instead: `jfgui_PERM_ANSWER=once|always|deny|never` for every
  question, or the script commands below. Stored answers count in every run.
- **The stored answers** (`Always`, `Never`) live in `config://permission-decisions.txt`, one
  line per rule (`always fs.read $DOCUMENTS/**`, `never agent document.delete`); deleting a
  line asks again. The app's own folders are writable for the app, but not this file: the
  user's answers cannot be written by the app through `plat`. `jfgui_PERM_STORE` names
  another file (empty: memory only); test scripts keep the answers in memory unless
  `jfgui_PERM_STORE` or `perm store` names a file.
- **Revoking.** `ui.permissions_page(id)` is a ready-made settings section: every ask line
  and every answered agent action with what it allows, the rule, the reason and the answer
  ("Immer erlaubt", "Bis zum Programmende abgelehnt", "Wird beim nächsten Zugriff gefragt",
  ...), a reset button per entry ("Zurücksetzen: url mailto") and "Alle zurücksetzen". Apps
  that build their own page use `policy::decisions()` and `policy::forget(rule)` /
  `forget_all()` (revoking needs no key; answering does).
- **An app without a manifest** has no ask lines: everything stays allowed. The UI still
  holds the process's key (`policy::take_key`), for the questions of agent actions.

The pieces in `plat::policy`: `questions()` (what waits for the user), `Key::ask_here(on)`
(the UI thread asks), `Key::answer(id, Answer)`, `Key::set_answer`, `Key::set_wait`,
`Key::set_store`, `decisions()`, `forget`, `forget_all`, `reload`. Only the UI runtime holds
the key, so an app cannot answer its own questions.

Script commands ([`lib/ui/auto_perm.rs`](../lib/ui/auto_perm.rs)):

| Command | Does |
| --- | --- |
| `perm answer once\|always\|deny\|never` | answers every question so, without a dialog |
| `perm answer dialog` | shows the questions as the dialog (the script clicks its buttons) |
| `perm answer none` | nobody asks again (the default of a script) |
| `perm timeout MS` | how long other threads wait |
| `perm store FILE\|none` | where `Always` / `Never` are kept |
| `perm reload` | reads the stored answers again (what a later run sees) |
| `perm wait [MS]` | waits until a question is open (a worker's), then shows it |
| `perm reset` | forgets every answer |
| `perm expect open\|none` | a question is open / none is |
| `perm expect RULE once\|always\|deny\|never\|unanswered` | the answer of a line (`perm expect url mailto deny`) |

## Plugins

A native plugin (`plat::lib_open`) declares what it needs in `<name>.permissions` beside the
library (`Effects/qoi.dll` → `Effects/qoi.permissions`), in the syntax of the app's manifest
without `ask` and agent lines. `$PLUGIN` is the plugin's folder; reading that folder is
implied.

```text
# Effects/qoi.permissions
fs.read  $PLUGIN/qoi-data/**
fs.write $APPCACHE/qoi/**
net.connect updates.example.org:443
```

The host loads the rights with `policy::plugin_rights(lib_path)` (an `Err` names the bad
line: do not load the plugin) and runs every call into the plugin's code inside
`policy::on_behalf(&rights, || ...)`. On that thread every check of `plat` -- files, the
network and its name lookups, processes, URLs, plugins, printing, secrets, autostart, sound -- then
needs the plugin's rules **and** the app's: the plugin cannot do what the app may not, and the
app's rights do not pass to the plugin. Jobs started inside the scope keep it
(`task::spawn_blocking`, `parallel_for` items on every worker); nested scopes (a plugin
calling the host, which calls another plugin) need every one of them. A refusal names the
plugin: `PERMDENY fs.write C:/x (plugin qoi)`, `Denial::plugin`.

| App | Plugin | The plugin's calls |
| --- | --- | --- |
| no manifest | no manifest | unrestricted (as before) |
| no manifest | `<name>.permissions` | the plugin's rules |
| manifest | no manifest | nothing (fail closed) |
| manifest | `<name>.permissions` | both rule sets |

PaintCraft does this for its plugins (`integrations/plugins/native.rs`: the rights are read
before the library is opened, `gcall` runs query, render, decode, encode ... in the scope). A
question of an `ask` line that a plugin's call raises says which plugin it is for.

**What an in-process plugin can still do.** The scope is a rule of `plat`, not a sandbox.
Plugin code runs in the app's process with its memory and rights: it can read and write the
host's memory, call the operating system directly (raw system calls on Linux; on Windows it
can find the system's DLLs through the process structures, although the loader gives it no
imports), start threads outside any scope, or corrupt the host and crash it (the crash guard
catches faults, not malice). Rights for plugins therefore keep honest plugins honest and turn
the calls they make *through the host* into audited, refused requests; they do not contain a
hostile one. Untrusted code belongs in another process (and, later, in the OS sandbox:
roadmap §1). Loadable modules (`modules/`, `plat::modhost`) get no file, network or process
functions in their host table at all.

## Agent control

External control (`--agent`, `--agent-json`, `--agent-mcp`, the browser's control messages)
is switched on explicitly as before; on top of that each action has an access level
(`agent_core::Access`):

- `ActionSpec::confirm()` or `agent.confirm NAME..` in the manifest: the user confirms
  before an external call runs. The question is the dialog above ("Eine Aktion für den
  Agenten ausführen", the action's name and description); the answers are per action
  (`agent document.save`), `Once` meaning for the rest of this run (the agent's session),
  `Always` stored.
- `ActionSpec::local_only()` or `agent.deny NAME..`: only the app's own GUI may run it.
- Patterns: `*` any run, `?` one character (`document.*`, `ui.*`, `*.delete`). The manifest
  can only tighten an action (deny wins over confirm wins over open).

Refusals reach the agent as structured errors (JSON-RPC `error.data.code`, MCP tool errors):

| Code | When |
| --- | --- |
| `forbidden` | `agent.deny` / `local_only` (`details.reason: "app"`), the user refused (`"user"`), or nobody can confirm in this run (`"unattended"`: headless, console) |
| `confirmation_required` | the question is open in the app's window; call again after the answer |

An agent's `ui.command` of a domain action has that action's access. While a permission
question is open, the `ui.*` input actions are refused (`unavailable`): the agent cannot click
"Immer erlauben" itself. The `ui.*` actions
(`ui.click`, `ui.type`, `ui.key`, ...) operate the GUI like the user: an agent clicking the
"Delete" button does not run `document.delete` as an action. Apps that must keep an agent
away from such buttons deny or confirm the `ui.*` actions themselves (`agent.confirm ui.click
ui.type ui.key`). `actions/describe` and `actions/list` report each action's `access`.

## OS sandbox

The checks above run inside the process, so a bug in the library or native code that calls the
operating system directly can bypass them ([Limits](#limits), [Plugins](#plugins)). An app can
ask the operating system to enforce the same manifest as well, as defence in depth. It is
opt-in, one line in the manifest, and apps without it are unchanged:

```text
sandbox          # best effort: enforce what this OS and kernel support, carry on otherwise
sandbox strict   # refuse to start when the OS cannot enforce the manifest
```

`plat::boot` applies it right after it installs the manifest and grants the command line's
files, before the app's own code and any worker thread. A one-shot that only registers the
app's file types (`--register` ...) is not sandboxed (it writes the user's config folders and
then exits; no app code runs). `plat::sandbox::status()` reports what was applied (for a
diagnostics page or a test).

### Linux: Landlock and seccomp

- **Landlock** (`lib/plat/linux_sandbox.rs`, raw syscalls, no libc; the ABI is detected and
  older kernels get what they support) restricts the filesystem to:
  - the folders and files the manifest grants (`fs.read` read only, `fs.write` read and write,
    the plugin folders read and execute), with the command line's `fs.user` files,
  - the app's own folders (`config://`, `data://`, `cache://`, `logs://`) and the folders the
    library always reads (the system fonts, the Hunspell dictionaries, the executable's folder,
    `/etc/resolv.conf` and `/etc/hosts`),
  - the system paths the library needs: `/usr`, `/etc`, `/bin`, `/lib`, `/lib64`, `/sbin`,
    `/opt`, `/proc`, `/sys`, `/run`, `/var` (read only), and `/tmp`, `$TMPDIR`,
    `$XDG_RUNTIME_DIR` and `/dev` (read and write), the X11 cookie (`$XAUTHORITY` or
    `~/.Xauthority`).
  - Landlock grants existing paths only, and is coarser than the lexical rules: a pattern
    grants the folder above its first wildcard (`$TEMP/mytool-*/**` grants `$TEMP`; the
    in-process check still narrows it). The app's own folders and the folder of every
    `fs.write` rule are created at start-up when they are missing. A command-line file that
    does not exist yet cannot be granted (name its folder in a rule).
  - The X11 and Wayland display sockets, the D-Bus session bus and the PulseAudio socket are
    Unix sockets; connecting to a Unix socket is not a filesystem action Landlock mediates, so
    they keep working without a rule.
  - Network: on Landlock ABI 4 and newer, TCP `bind` is restricted to the ports of
    `net.listen` and TCP `connect` to the ports of `net.connect` (plus 53 for DNS when a name
    rule exists). A rule with a port range wider than 64 or `*` leaves TCP unrestricted (the
    in-process policy still checks it); with no `net.*` rule, all TCP is blocked. UDP is not
    restricted by Landlock (DNS over UDP keeps working).
- **seccomp-bpf** installs a deny list that returns `EPERM` for `ptrace`, `process_vm_readv`,
  `process_vm_writev`, `kexec_load`, `kexec_file_load`, `init_module`, `finit_module`,
  `delete_module`, `mount`, `umount2`, `pivot_root`, `chroot`, `bpf`, `perf_event_open`,
  `userfaultfd`, `keyctl`, `add_key`, `request_key`, `open_by_handle_at`, `setns` and
  `unshare`, and for `execve` / `execveat` when the manifest allows no program. It does not
  touch the syscalls the library itself uses (threads, futexes, memory, sockets, the font, IME
  and D-Bus code paths). `NO_NEW_PRIVS` is set first, so it needs no privilege. The filter is
  inherited by the threads started afterwards.
- **`fs.user` at run time.** Landlock grants are fixed when the sandbox is applied; it cannot
  add a path the user picks later in a file dialog. Inside a Flatpak the file dialog and the
  documents portal hand the file over below the portal mount (`$XDG_RUNTIME_DIR/doc/`), which
  is granted, so picked files work. Outside a sandbox the native dialog returns an ordinary
  path that Landlock did not grant: such an app should not opt in to the OS sandbox, or should
  run inside a Flatpak, or should widen its Landlock grant by naming the folder with an
  `fs.read` / `fs.write` rule. The in-process `fs.user` grant still holds either way.
- **`execve` and `spawn_self`.** A sandbox with no `process` rule blocks every `execve`,
  including `plat::spawn_self` (a crash relaunch, single-instance forwarding). An app that
  needs those must list a `process` rule (which also keeps `execve` allowed).

### Windows: process mitigation policies

A full-trust Win32 process is not confined the way an AppContainer is, but the process
mitigation policies (`lib/plat/win_sandbox.rs`, `SetProcessMitigationPolicy`) still harden it
from the manifest, as defence in depth:

- **Always** (`sandbox`): image load (no images from a remote/UNC path or with a low integrity
  label), extension points disabled, and -- unless the manifest has a `plugins` rule --
  dynamic code prohibited (no new executable memory; the plugin loader maps on-disk images,
  which stays allowed). `VirtualAlloc(PAGE_EXECUTE_READWRITE)` is then refused; `sandboxtest`
  checks that. Third-party code loaded into the process (shell extensions of the file dialog,
  IMEs, printer drivers) that needs executable memory fails under this policy.
- **`sandbox strict`** also blocks child-process creation when the app may not start other
  programs: no `process` rule and no `url` / `print` rule (`open_url`'s `ShellExecute` and the
  print dialog can start one). This stops `plat::spawn_self` too. Strict handle checks are
  deliberately not enabled: an invalid-handle reference anywhere would raise an exception and
  turn a benign race in a GUI into a crash.
- **AppContainer** (re-executing the app into an AppContainer profile with capabilities from
  the manifest) is *not* done here: it needs ACL grants on the app's real `config` / `data` /
  `cache` folders (a lasting change to the user's folders) and testing per app to be sure the
  GUI still works. For a packaged app the MSIX capabilities in `tools/pkg_msix.py` already
  follow the manifest (`internetClient` for `net.connect`, `internetClientServer` /
  `privateNetworkClientServer` for `net.listen`); an MSIX full-trust app is not confined by
  them, they are shown in the Store.

### Flatpak

`tools/pkg_linux.py` derives the Flatpak finish-args from the manifest and is already as narrow
as it allows: Wayland with X11 as fallback and IPC, the network only with a `net.*` rule, the
Secret Service only with `secrets`, `--filesystem` holes only for the folders of `fs.*` rules
(`fs.read` as `:ro`), the host's `recently-used.xbel` only with `recent`, and portals (file
dialog, `open_url`, printing, autostart, the documents portal for `fs.user`) instead of holes. Inside the Flatpak the kernel enforces those; the
in-process OS sandbox above stacks on top when the manifest also has a `sandbox` line.

## Refusals

A refused call fails the way a missing or unreadable target fails: `None`, `false`, `0`,
`-1`, `Err(LIBE_IO)`, `ERR_UNSUPPORTED` (print), `Err(5 / 13)` (access denied, process), a
`NetError` of kind `NetErrorKind::Denied` (network), `SecretError::Denied` (secrets;
`secret_available()` is `false`), `AutostartError::Denied` (autostart),
`AudioError::Denied` (sound: `sound()` and `ui.beep` give `false`, `audio::available()` is
`false`; a headless script sees `expect sound none`), `HotkeyError::Denied` (input.global)
or `ScreenError::Denied` (screen.capture).
In addition:

- stderr shows `PERMDENY <kind> <target>` once per target, for example
  `PERMDENY fs.write C:/Users/me/Desktop/out.png`, `PERMDENY net.connect evil.example` or
  `PERMDENY secrets imap.example.org` (the service of the refused secret), `PERMDENY autostart`,
  `PERMDENY audio.output` (once per run, however many sounds), `PERMDENY input.global`,
  `PERMDENY screen.capture`, `PERMDENY agent document.delete`; a call on behalf of a plugin ends with ` (plugin NAME)`;
  a question put to the user shows as `PERMASK <kind> <target> (<rule>)`;
- `plat::policy::denials()` returns the last 64 refusals (`kind`, `target`, `plugin`) for a
  log or a diagnostics page;
- `plat::capabilities()` drops `CAP_FILE_READ` / `CAP_FILE_WRITE` (no file rules and no
  `fs.user`), `CAP_PRINT` and `CAP_NATIVE_PLUGINS`. App commands and the agent schema then
  report the service as unavailable instead of failing later.

## Limits

- **Lexical checks.** A symbolic link inside an allowed folder can lead outside it. On
  Linux a path with a `..` segment is refused, because the kernel applies `..` after
  symbolic links (`allowed/link/../x`). Windows resolves `..` in the text, as the policy does.
- **Not a sandbox against native code, by default.** Native plugins and app code that call the
  OS directly bypass `plat` ([Plugins](#plugins): what an in-process plugin can still do). An
  app that adds a `sandbox` line to its manifest has the operating system enforce the manifest
  as well (Landlock + seccomp on Linux, process mitigation policies on Windows), so a bypass of
  the in-process checks is still contained ([OS sandbox](#os-sandbox)).
- **Questions.** The stored answers are a plain file in the app's config folder: `plat`
  refuses the app's writes to it, native code can still change it. Answers are per line, not
  per file or host. The UI thread's own access is refused while its question is open (the app
  retries on `UIC_PERMISSION`); the dialog cannot block it.
- **Addresses, not certificates.** A name rule allows the addresses the name resolved to in
  this process. A connection is not tied to the name: another service on the same address
  and port can also be reached. TLS (`netio`) checks the server's certificate against the name
  when it verifies against a trust store.
- **Local IPC** of the agent control (`plat::control_ipc`) uses its own private sockets. It is
  switched on explicitly and is not part of the manifest.
- **Secrets.** `secrets` allows the store as a whole. The entries are named after the app
  (`vfs::app_id()`), so the app reaches only its own through `plat::secret`; the operating
  system itself does not separate the programs of one user ([SECRETS.md](SECRETS.md)).
- **Updates.** `lib/update` ([UPDATES.md](UPDATES.md)) fetches its channel through `netio`, so
  the channel's host (and the host of absolute file addresses and redirects) needs a
  `net.connect` rule. Putting an update in place writes the program's folder: add
  `fs.write $EXE/**`, or the update stays downloaded and `startup()` reports the missing rule.
  An AppImage replaces its `.AppImage` file instead: `fs.write $APPIMAGE/**`.
- **Android.** The APK installs the manifest like a desktop program, but `plat` runs on the
  host adapter of the browser there (`web.rs`, `web_files.rs`): the files are the adapter's
  virtual files (`/config`, `/data`, `/cache`, `/tmp`, `/documents`, `/imports`), checked with
  the Linux rules. `$TEMP` is `/tmp`, `$HOME` and `$DOCUMENTS` are `/documents`, the app's
  own folders lie below `/config`, `/data` and `/cache`; `$EXE` has no value (the pattern is
  dropped), and the other user folders do not exist. `plat` has no processes, sockets or
  native plugins on Android, so `process`, `net.*` and `plugins` have nothing to allow;
  `secrets` allows the store of the Android Keystore (checked in `plat` before the Java side
  is asked), `audio.output` the AAudio streams and the system's sounds (Android itself asks
  no permission for playing sound). The rules do not reach Android's own storage (the app's private folders,
  `content://` documents): the Java side of the bridge (`android/*.java`) does not go through
  `plat`. The Android build is checked by building it;
  the manifest has not been run on a device in the test suite.
- **Packages.** `tools/package.py` derives the Flatpak sandbox (finish-args) and the MSIX
  capabilities and StartupTask from the manifest ([PACKAGING.md](PACKAGING.md) "Flatpak",
  "MSIX"); in Flatpak the kernel then enforces the file and network holes as well. Sound in a
  Flatpak needs the sound server's socket (`--socket=pulseaudio`), which also allows
  recording: only an app with `audio.output` (or without a manifest) gets it; `plat` itself
  records nothing. MSIX needs no capability for playing sound.
- **Agent mode.** An agent that drives the library's file dialog (`--agent-json`,
  `--agent-mcp`) can choose files like a user. It cannot answer a permission question: while
  one is open the `ui.*` input actions are refused (`unavailable`). Agent mode must be enabled
  explicitly.
