# Updates -- signed, applied at the next start, rolled back when the new version fails

`lib/update` lets an app check a channel for a newer version, download it in the background,
put it in place at the next start and go back to the old version when the new one does not
start. The content is protected by an Ed25519 signature with a key built into the app, not by
the transport.

| Part | Where |
| --- | --- |
| Check, download, status, the manifest | [`lib/update/lib.rs`](../lib/update/lib.rs), [`manifest.rs`](../lib/update/manifest.rs), [`fetch.rs`](../lib/update/fetch.rs) |
| State, putting in place, trial, rollback | [`lib/update/apply.rs`](../lib/update/apply.rs) |
| AppImage files and their GPG signature | [`appimage.rs`](../lib/update/appimage.rs), [`pgp.rs`](../lib/update/pgp.rs) |
| The way the program is installed, the package systems' updates | `plat::pkg` ([`pkg.rs`](../lib/plat/pkg.rs): MSIX through WinRT, the Flatpak portal) |
| Restarting the program file at its path | `plat::restart_self`, `plat::pkg::restart_appimage` |
| Streaming downloads | `netio::Client::get_to`, `netio::http::Reader::body_to` |
| Keys, writing and signing a channel | [`tools/update_manifest.py`](../tools/update_manifest.py), `tools/package.py --update` |
| Tests | [`apps/updatetest`](../apps/updatetest/main.rs) against [`tools/update_server.py`](../tools/update_server.py) |

## In an app

```rust
// the public key of tools/update_manifest.py keygen (the private key stays with the publisher)
const UPDATE_KEYS: &[&str] = &["3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c"];

fn config() -> update::Config {
    update::Config::new("Kritzel", "1.4.0", "https://example.org/kritzel/stable/update.json", UPDATE_KEYS)
}

ui::main!({
    // before the UI: put a downloaded update in place (and restart into it), count the starts
    // of a new version, roll back one that never reported healthy
    match update::startup(&config()) {
        update::Startup::RolledBack(v) => { /* tell the user that version v failed */ }
        _ => {}
    }
    ui::launch(&DESC, Kritzel::default(), &style::STYLE)
});

// in the app, once it works (after the first frame, for example):
update::healthy(&config());

// a check and a download in the background; frame() shows the state
let u = update::Updater::new(config());
u.start_check();                 // plat::wake() when done: E_WAKE / UIC_WAKE
match u.status().phase {
    update::Phase::Available => { /* "Version {} herunterladen?" -> u.start_download() */ }
    update::Phase::Downloading => { /* progress: status().done / status().total */ }
    update::Phase::Ready => { /* "Wird beim nächsten Start installiert" or u.apply() */ }
    update::Phase::Failed => { /* status().error */ }
    _ => {}
}
```

`check()`, `download()` and `apply()` are the same steps as blocking calls (a console program,
a worker of its own). `apply()` puts the update in place at once and restarts the program; the
process ends there. `cancel()` stops a download (what arrived is kept and continued later).
`staged()` returns the file of a `package` update for the app to hand to the user.

The `Config` fields after `new`: `dir` (the updater's folder, default `data://update`), `exe`
(the program file to replace, default the running exe), `target` (`update::target()`),
`attempts` (starts of a new version without `healthy()` before the rollback, default 1),
`restart` (default true; false: `startup()` returns `Applied` / `RollingBack` and the caller
restarts), `max_size` (1 GiB), `ca_pem` (extra CAs of a self-hosted https channel),
`timeout_ms`.

## The channel

Two files at the channel's address, written by `tools/update_manifest.py` (or
`tools/package.py --update`):

* `update.json` -- the manifest (UTF-8 JSON):

  ```json
  {"format": 1, "app": "Kritzel", "version": "1.5.0", "date": 1760700000,
   "expires": 1763300000, "notes": "Neue Pinsel ...",
   "files": [
    {"target": "windows-x86_64", "kind": "exe", "url": "Kritzel-1.5.0.exe", "size": 4718592, "sha256": "..."},
    {"target": "linux-x86_64", "kind": "package", "url": "kritzel_1.5.0_amd64.deb", "size": 3145728, "sha256": "..."}]}
  ```

* `update.json.sig` -- the Ed25519 signature over the exact bytes of `update.json`, Base64.

`target` is `windows-x86_64`, `linux-x86_64`, `linux-aarch64`, ... (`update::target()`).
`kind`:

| kind | Put in place by the updater | Use |
| --- | --- | --- |
| `exe` | yes: replaces the program file | a single-file program |
| `bundle` | yes: every file of a JFA1 bundle (`tools/assetpack.py`) goes to its path relative to the program's folder | the program with its modules (`jxlenc.dll`) or further programs (`BINS`) |
| `package` | no: downloaded and checked, `Updater::staged()` names the file | an installer or a `.deb` the user or the package manager installs (programs in `/usr/lib` or `Program Files` cannot replace themselves) |
| `appimage` | yes: replaces the running AppImage's file | an AppImage of the style ("Packages" below); only a program that runs as an AppImage takes it, and it takes nothing else |

A channel may list several entries for one target: a program running as an AppImage picks the
first `appimage` entry of its target, every other program the first entry of another kind (so
one channel serves the `.deb` and the AppImage of a style).  An entry of a kind this version
does not know is left out, so a later kind does not break older programs (versions before
2026-10-30 refused the whole manifest).

`url` is relative to the address `update.json` came from, or absolute. Redirects are
followed (at most five), so a "latest" address that redirects to a versioned folder works.

## What is checked

1. The signature of `update.json` against the keys of `Config::keys`; a manifest changed
   after signing or signed with another key is refused (`Error::Signature`).
2. `format` is 1, `app` is the app's id (a manifest of another app with the same key is
   refused), `expires` (unix seconds, optional) has not passed (freeze attacks: a server that
   keeps handing out an old manifest).
3. The version: offered only when newer than the running one and not rolled back before.
   A manifest older than one a signed manifest named before is refused (`Error::Replay`):
   a mirror cannot push the app back to an older, vulnerable version.
4. The download: the size from the manifest is the limit while it arrives, the SHA-256 is
   computed on the way; a wrong file is deleted (`Error::Size` / `Error::Hash`).
5. Before the files are put in place, everything again: the staged `update.json` against the
   keys, its version against the state, the file's size and SHA-256. A damaged staged update is
   dropped.

The transport may be http: the signature covers the content. https adds privacy and
checks the server against the system's CAs (plus `ca_pem`). Connections go through `netio`
and `plat::net`, so the app's `net.connect` rules ([PERMISSIONS.md](PERMISSIONS.md)) apply to
the manifest, the signature, the file and every redirect; a refused host gives
`Error::Denied(host)` and `PERMDENY net.connect HOST` on stderr.

## Putting in place, trial and rollback

`cfg.dir` holds `state` (`stage`, `version`, `from`, `starts`, `bad`, `seen`), `lock`,
`partial/<sha256>` (a download in progress or broken off: continued with a `Range` request),
`staged/` (the manifest, its signature, the file) and `backup/` (the files the last update
replaced, and their list).

1. **Staged**: `download()` checked the file; the next `startup()` (or `apply()`) puts it in
   place.
2. **Putting in place**: every file of the update is written next to its target as
   `NAME.jfupd-new`; the old file is copied to `backup/` and renamed away (`NAME.jfupd-old-…`:
   Windows can rename a running exe, not overwrite it); the new file takes its name (Linux: an
   atomic rename over it, with the old file's permissions). If one file fails, the files
   already replaced come back. Then the program restarts itself at its path
   (`plat::restart_self`: Windows keeps the start path of a renamed exe, Linux strips
   `(deleted)` from `/proc/self/exe`) and the old process ends.
3. **Trial**: the new version's `startup()` counts its start and returns `Startup::Trial`.
   `healthy()` ends the trial (stage `ok`, the backup is kept until the next update).
4. **Rollback**: a start that finds the trial still open after `attempts` starts puts the
   backup back (new files of a bundle are removed), records the version in `bad` and restarts
   the old program, whose `startup()` returns `Startup::RolledBack(version)` once. That version
   is never offered again.

Leftover `*.jfupd-*` files are removed at a later start, once no process uses them. A second
instance of the app does nothing while another one holds the lock.

The health marker is the app's decision: after the first frame is a good default, after the
documents of the last session were opened is a stricter one. A version that crashes before
`startup()` runs cannot roll itself back; keep `startup()` first in `main`.

## Permissions

Applying writes the program's folder. In an app with a `PERMISSIONS` manifest that needs
`fs.write $EXE/**`; without it `startup()` returns `Startup::Failed(Error::Io(..))` naming the
rule, nothing is written and the update stays staged. Programs in folders of the system
(`/usr/lib/<deb>` of the `.deb`, `Program Files`) cannot be written by the user either: use
`package` updates there. The per-user Windows setup of `tools/package.py`
(`%LOCALAPPDATA%\Programs\<app>`) is writable, so `exe` and `bundle` updates work there.
An AppImage writes the folder of its `.AppImage` file instead: `fs.write $APPIMAGE/**`.

```text
net.connect updates.example.org:443   # the channel (and the host of absolute file URLs)
fs.write $EXE/**                      # putting updates in place
fs.write $APPIMAGE/**                 # ... as an AppImage
```

## Packages: AppImage, Flatpak, MSIX, Snap

The same calls (`check`, `download`, `apply`, `startup`, `healthy`) follow the way the program
is installed: `update::install(&cfg)` returns `plat::pkg::packaged()` (`Plain` when
`Config::exe` names a program file):

| `Packaged` | What an update is | Signature | Rollback |
| --- | --- | --- | --- |
| `Plain` | `exe`, `bundle`, `package` (above) | `update.json` (Ed25519) | trial, backup |
| `AppImage(file)` | the channel's `appimage` entry replaces `file` ($APPIMAGE) | `update.json` (Ed25519, SHA-256 of the file) and the AppImage's own GPG signature | trial, backup |
| `Flatpak(id)` | the app's Flatpak repository, through the Flatpak portal (`Kind::Flatpak`) | the repository's GPG key (Flatpak checks it) | none by the app (`flatpak update --commit=` by the user) |
| `Msix(name)` | `Check::Managed(Manager::AppInstaller, ..)` or `Manager::Msix` | the package's signature (Windows checks it) | none by the app |
| `Snap` | `Check::Managed(Manager::Snap, None)` | snapd | snapd |

**AppImage.**  The program runs from a copy the runtime unpacked, so an `exe` update would
change nothing that lasts; the `.AppImage` file is replaced as a whole:

1. `check()` takes the first `appimage` entry for the target (none: `Check::NoPackage`) and
   probes the file's folder: a folder the user cannot write is `Err(Error::ReadOnly(folder))` --
   reported, not worked around (copy the AppImage somewhere writable, or download it by hand).
2. `download()` stages the file as for every kind (size, SHA-256, `Range`), read and hashed in
   pieces.
3. The next `startup()` (or `apply()`) copies it as `NAME.jfupd-new` next to the AppImage
   (hashed again on the way), checks that it is an AppImage of type 2 whose signature follows
   the running one (below), keeps the old file in `backup/`, renames the new file over the old
   one (atomic: a crash leaves the old or the new file, never half of one) and starts the
   `.AppImage` again (`plat::pkg::restart_appimage`: its runtime unpacks the new image).  Trial,
   `healthy()` and the rollback are those of an `exe`.

The AppImage's signature: `tools/package.py` signs AppImages as `appimagetool --sign` does
(an ASCII-armored detached GPG signature of the file's SHA-256 -- taken with the two sections
as zeros -- in `.sha256_sig`, the armored public key in `.sig_key`; [PACKAGING.md](PACKAGING.md)
"AppImage").  The updater checks it with lib/crypto (OpenPGP v4: RSA, ECDSA P-256 / P-384,
Ed25519; `pgp.rs`): a signed AppImage only takes an update signed by a key of its own key
block; an unsigned one takes a signed or an unsigned update (the signed manifest decides); a
signature that does not check, a new file without one, another key or a file that is no
AppImage is `Startup::Failed(Error::Rejected(why))`, and the staged update is dropped.
`update::appimage_signature(path)` reads it (`Signed::No`, `Signed::By { signer, keys }`, or
why it fails), for an About dialog for example.

**Flatpak.**  The sandbox's `/app` is read-only and belongs to Flatpak; updates come from the
repository the app was installed from (a bundle of `tools/package.py` names it:
`flatpak_repo_url=`).  The updater uses the Flatpak portal's update monitor
(`org.freedesktop.portal.Flatpak.CreateUpdateMonitor`, over plat's own D-Bus client):

* `check()` makes the monitor on its first call and keeps it; the portal looks at the remote
  every few minutes (flatpak-portal `--poll-timeout`, 300 s by default) and reports
  `UpdateAvailable`, which `check()` answers as `Check::Available(Offer { kind: Kind::Flatpak,
  version: <the first 12 hex digits of the new commit>, .. })` -- the first check after the start
  usually says `UpToDate`; check again later (a timer, `start_check()`).  An update installed
  meanwhile by `flatpak update` is `staged: true`.
* `download()` calls the monitor's `Update`: Flatpak installs the new commit (checked against
  the repository's GPG key); `status().done` / `total` follow its `Progress` (0..100).  The
  portal asks the user once whether the app may update itself (the desktop's dialog; the
  answer stays in the permission store, Settings -> Privacy), and it refuses a version that
  asks for more permissions than the running one (`Error::Unsupported("Flatpak: ..")`).
* `apply()` starts the newest installed version (`Spawn` with `LATEST_VERSION`, the same
  program and arguments) and ends this process (`Config::restart = false`: it returns
  `Startup::Applied`).  `startup()` and `healthy()` do nothing: there is no trial and no
  rollback by the app.
* Without a session bus or the portal (an old Flatpak): `Check::Managed(Manager::Flatpak,
  None)` -- `flatpak update` or a software centre updates it.

**MSIX.**  The package folder is read-only; Windows installs new versions.  `check()` asks the
package through WinRT (`Package.Current.GetAppInstallerInfo`, `CheckUpdateAvailabilityAsync`;
raw COM in `plat::pkg`, no projection):

* installed from an `.appinstaller` file (`tools/package.py` writes it with PACKAGE
  `appinstaller_url=`): `Check::Managed(Manager::AppInstaller, Some(true))` when App Installer
  knows a newer version, `Some(false)` when not, `None` when it cannot tell.  App Installer
  installs it at the next start (`OnLaunch`, with a prompt) or in the background; the app has
  nothing to do but tell the user to restart.
* otherwise (the Store, `Add-AppxPackage`, a developer registration):
  `Check::Managed(Manager::Msix, None)`.

Installing a new version from inside the app (`PackageManager.AddPackageByAppInstallerFileAsync`)
is not done: App Installer does the same at the next start, with its own checks and prompt.

An update staged before the program came into a package (a `.deb` that became a Flatpak, say)
is dropped at the next start (`Startup::Failed(Error::Unsupported(..))`, once).

## Publishing

```sh
python tools/update_manifest.py keygen ~/keys/kritzel.updkey    # once; prints the public key for Config
export jfgui_UPDATE_KEY_FILE=~/keys/kritzel.updkey
python tools/update_manifest.py sign --app Kritzel --version 1.5.0 --out build/channel \
    --file windows-x86_64=build/win/Kritzel.exe,name=Kritzel-1.5.0.exe \
    --file linux-x86_64=build/dist/kritzel_1.5.0_amd64.deb,kind=package \
    --notes "Neue Pinsel" --expires-days 30
python tools/update_manifest.py verify build/channel --pub <public key>
```

`tools/package.py APP --update DIR` writes a channel per style (`DIR/<style>/`: a style is
built into the program, so each style has its own channel; the app names its style's address)
with the program of each style (`exe`, or a `bundle` with modules and further programs) and the
`.deb` (`package`). `PACKAGE` may set `update_app` (the manifest's `app`, default the app's
folder name), `update_notes` and `update_expires_days`. Upload the folder as it is; the files
are referenced relative to `update.json`.

The signing key is the key to every installation: keep it off the build machine's repository
(`*.updkey` is ignored by git), ideally offline, and pass it through `--key`,
`jfgui_UPDATE_KEY_FILE` or `jfgui_UPDATE_KEY`. To change keys, ship a version with both public
keys in `Config::keys`, then sign with the new one.

## Tests

`build_rs.py test` runs `apps/updatetest/tests/update.uis` and `apply.uis`. The test starts
`tools/update_server.py` (Python, a key generated for the run) and checks a valid update, a
redirect, a wrong signature, a manifest changed after signing, a wrong hash, a download broken
off and continued with `Range`, an older version, a replayed manifest, another app's manifest,
an expired one, no package for the system, hosts the `PERMISSIONS` refuse (manifest and file),
cancelling a background download, applying `exe` and `bundle` updates with trial, health
marker and rollback, a `package` update, a damaged staged file and a program folder without
write permission. `apply.uis` installs a copy of the test program in a scratch folder, lets it
update itself to 2.0.0 (restart, healthy), then to a 3.0.0 that fails its first start and is
rolled back to 2.0.0 by the next start -- real processes on Windows and Linux.

On Linux both also cover AppImages: the server signs fake AppImages with throwaway GPG keys
(Ed25519, RSA, ECDSA; a temporary `GNUPGHOME`, its agent ended before it serves) and
`update.uis` checks the signatures, the choice between `.deb` and AppImage, replacing the
`.AppImage` file with trial and rollback, the refused cases (another key, unsigned after
signed, a changed byte, no AppImage) and a read-only folder; `apply.uis` builds real AppImages
of the test program (the runtime `apps/appimagert`, mksquashfs) and lets the installed one
update itself through its file, then roll back.  Flatpak and Snap without a portal answer
`Managed`.  The package systems themselves: `python tools/pkg_check.py --quick --install`
installs the test program as a Flatpak (a GPG-signed repository and bundle, `flatpak --user`
in a temporary `HOME`, a private session bus with flatpak-portal) and lets it find, install
and start 2.0.0 through the portal, and registers it as an MSIX package (developer mode) to
switch its StartupTask and see `Check::Managed` ([PACKAGING.md](PACKAGING.md), "Checks").

## Limits

* Delta updates are not supported: the whole file is downloaded.
* A version that crashes before `startup()` runs, or hangs instead of crashing, is not rolled
  back automatically (no watchdog process).
* `exe` and `bundle` updates need a writable program folder; `package` updates leave the
  installation to the user (no elevation, no `apt` call).
* An AppImage is replaced as a whole (no zsync deltas, no AppImageUpdate); its signature is
  checked against the running file's key block, not against a key built into the app (the
  signed manifest is that).  OpenPGP keys other than RSA, ECDSA P-256 / P-384 and Ed25519 (and
  v5 / v6 keys) are refused.
* Flatpak: the portal reports updates minutes after the start (its poll interval), there is no
  trial or rollback by the app, and the version shown is a commit.  MSIX: the app only reports
  App Installer's state; it does not install a new version itself.
* Single-instance apps should call `apply()` (or restart) only when they are about to end:
  the new process starts right away.
* Web and Android builds update through their stores and servers; the crate is for Windows and
  Linux.
