# Packaging -- setup.exe, .deb, AppImage, RPM, Flatpak and MSIX

An app that only uses library widgets can be shipped in every style ([STYLES.md](STYLES.md)).
`tools/package.py` builds one variant per style and lets the user pick the look at install
time; the installed program contains only that style.  The setup and the .deb are the default;
`--formats` adds AppImage, RPM, Flatpak and MSIX (below, "Other formats").

```
python tools/package.py PaintCraft                         # build/dist/PaintCraft_Setup.exe
                                                         # build/dist/PaintCraft_0.1.0_amd64.deb
python tools/package.py Kritzel --styles winui,classic   # a subset of the styles
python tools/package.py Kritzel --no-deb                 # Windows only (--no-win: .deb only)
python tools/package.py PaintCraft --out build/pkg         # another output folder
python tools/package.py PaintCraft --impl asm              # legacy: the assembly reference, build/dist_asm
python tools/package.py PaintCraft --sign                  # every signature required (below)
python tools/package.py PaintCraft --repo build/repo       # also a signed apt repository
python tools/package.py PaintCraft --update build/channel  # also a signed update channel (UPDATES.md)
python tools/package.py template --formats all             # every format (--formats appimage,flatpak, ...)
```

What it does:

1. **Variants**: one `python build_rs.py app NAME --style S1,S2,.. --profile size
   --target both --out <out>/rs_size` for every style and target (the libraries are built
   once, both targets side by side, the style variants linked in parallel into
   `<out>/rs_size/<target>/<style>/`); each exe is copied to `<out>/<App>/<style>/<target>/`.
   A package of PaintCraft with four styles and both targets takes about 7 minutes from scratch
   on 12 logical CPUs (one style after the other: 19.5), less when the libraries are built
   already; the eight LTO links then run at once and set the time. For a quick try,
   `--styles winui --no-deb` builds one variant. `--profile size` (the
   default) builds the UI and platform crates with opt-level z and the compute crates and the
   app with 2, fat LTO: 14-21 % smaller than the opt-level-3 release of R13, 4-15 % smaller
   than `--release` with opt-level 2 (PaintCraft 4-5 %, gallery / Kritzel 14-15 %); codecs, effects
   and rendering as fast,
   UI scripts about 4 % slower ([rewrite/REPORT.md](../../rewrite/REPORT.md) section 2).
   `package.py --profile o2|release` picks another variant.
2. **Windows**: builds the setup stub `apps/installer` (same profile), renders a preview of every
   variant (`--shot` at 1100 x 700 DIP, light theme, scaled to 480 x 305) and writes
   `<out>/<App>_Setup.exe` = stub + payload + trailer.
3. **Linux**: assembles the package tree and runs `dpkg-deb` in WSL (Ubuntu-24.04).
4. **Modules** (jxl-module): the loadable modules of the app (`build_rs.py` `APP_MODULES`:
   PaintCraft's JPEG XL writer `jxlenc.dll` / `jxlenc.so`, the same file for every style) are
   built with the variants and installed next to the exe: one extra-file entry (kind 5) of
   the setup -- installed into the program folder, listed in `install.ini`, removed by
   `uninstall.exe` -- and `/usr/lib/<deb>/` in the .deb.

`--impl asm` does the same with `legacy/build.py` and the assembly setup stub (the frozen
reference; it needs the assembly toolchain). Both modes produce the same payload format and
the same .deb layout.

## Inputs

| File | Content |
|---|---|
| `apps/NAME/PACKAGE` | `key=value` lines: `display` (shown name), `version` (the app's own `MAJOR.MINOR.PATCH`: the build checks it and every program of the app reads it as `rt::app_version!()`, [VERSIONING.md](VERSIONING.md)), `publisher`, `summary` (one line, .deb / .desktop comment), `description` (one line), `generic` (.desktop GenericName), `categories` (.desktop), `deb_name`, `default_win` / `default_linux` (preselected style), `styles` (the variants, in order); optional `filetypes` and `schemes` (the file types and URL schemes the app opens, below); for `--update`: `update_app` (the manifest's app id, default the folder name), `update_notes`, `update_expires_days`; for the other formats (below): `app_id` (reverse DNS, default `org.jflib.<App>`: the library's own demo, test and template apps keep it; the author's apps set `org.jfreund.<App>`, e.g. PaintCraft and BackupCraft; an app of someone else sets a domain of its own), `homepage` (AppStream; Flathub wants it), `license` (SPDX, default `LicenseRef-proprietary`), `appimage_update`, `flatpak_runtime`, `flatpak_args`, `msix_name`, `msix_publisher`, `language` |
| `apps/NAME/PERMISSIONS` | the app's permissions ([PERMISSIONS.md](PERMISSIONS.md)): the Flatpak finish-args, the MSIX capabilities and StartupTask follow them |
| `apps/NAME/res/app.ico` | icon resource 1 of the Windows exe (`build_rs.py` compiles it with `llvm-rc` and links it: window, taskbar, Explorer, shortcuts) |
| `apps/NAME/res/app_{48,128,256}.png` | Linux icons (hicolor theme); the MSIX logos are scaled from them. Missing sizes are scaled down from the largest; an app without any gets a plain placeholder (and a note) |
| `styles/S/INFO` | line 1: the style's name in the setup / debconf, line 2: a one-line description (`--impl asm`: `legacy/ASMLIB/styles/S/INFO`) |

Texts are German (the setup and the .deb questions are). Example, `apps/PaintCraft/PACKAGE`:

```
display=PaintCraft
version=0.1.0
publisher=JFLib
summary=Bildbearbeitung im Stil von paint.net
description=Bildbearbeitung mit Ebenen, Effekten und vielen Bildformaten, geschrieben in Rust mit JFLib.
generic=Bildbearbeitung
categories=Graphics;2DGraphics;RasterGraphics;
deb_name=PaintCraft
default_win=winui
default_linux=kde
styles=winui classic macos kde
```

**Icons**: `python tools/mkicon.py NAME --glyph E771 --color 1E78E6` renders a glyph of the icon
font on a gradient square in every icon size (16..256 px), cuts anti-aliased rounded corners
(`tools/pngutil.py`) and writes `app.ico` and the PNGs. The glyph comes from
`styles/winui/assets/icons.bin` (Fluent UI System Icons, MIT; the styles that use it ship
`LICENSE-FluentSystemIcons.txt`) and is drawn by `tools/refrender.py`, so no build is needed. The
committed icons are installer `--glyph E896 --color 2E9E5B`, PaintCraft `E771 1E78E6` and paint
`E790 D9822B`.

## Windows: setup.exe

`apps/installer` (Windows only: `apps/installer/TARGETS` = `win`) is a normal library app
in the winui style. The packaged setup is that exe with a payload and a trailer appended.

* **Pages**: style (list with the INFO names and descriptions, preview of the selection) ->
  options (folder, default `%LOCALAPPDATA%\Programs\<display>` -- per user, no administrator
  rights; desktop shortcut, start menu entry, "installed apps" entry, start after installing)
  -> progress -> done (an error page on failures).
* **Installs** `<app>.exe` (the chosen variant), `uninstall.exe` (the setup without its
  payload), `install.ini` (what was created), `.lnk` files through COM `IShellLinkW`, and
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\<app>` (DisplayName, DisplayIcon,
  UninstallString, ...). ole32 / shell32 / advapi32 are loaded with `LoadLibraryA` inside
  `plat::inst` (the only place of the shell calls).
* `uninstall.exe --uninstall` removes everything listed in `install.ini`; a hidden `cmd`
  deletes `uninstall.exe` and the folder after the process ended.
* Running the setup again installs another style over the old one (the app's settings stay).
* Options: `--dir PATH`, `--style ID`.
* **File types and URL schemes** (`PACKAGE` `filetypes=` / `schemes=`, carried in the
  manifest; `apps/installer/assoc.rs`): for each type a ProgID `<app>.<ext>` under
  `HKCU\Software\Classes` (description, `DefaultIcon`, `shell\open\command` = `"<exe>" "%1"`) and
  the app in `.<ext>\OpenWithProgids` ("Open with"). An `own` type also becomes the default of its
  extensions (`.<ext>` default value and `Content Type`). A scheme gets `HKCU\Software\Classes\<scheme>`
  (`URL:<display>`, `URL Protocol`, `DefaultIcon`, the open command). `SHChangeNotify` tells
  Explorer. `install.ini` `assoc=` lists what was added (`T:` key trees, `V:` values), and
  `uninstall.exe` removes exactly that. In test mode the values go to
  `<ASMINST_SHELL_DIR>\assoc.txt` (`subkey|name=value` lines).
* **Test mode**: with `ASMINST_SHELL_DIR=DIR` the desktop and the start menu are `DIR\Desktop`
  and `DIR\Programs`, and the registry values go to `DIR\registry.txt` -- a test never touches
  the user's shell or registry. Always run the setup with `--script` (and `--dir` into a test
  folder):

  ```
  set ASMINST_SHELL_DIR=%CD%\build\dist\test\shell
  build\dist\Kritzel_Setup.exe --script build\dist\inst.uis --dir %CD%\build\dist\test\inst
  build\dist\test\inst\uninstall.exe --uninstall --script build\dist\un.uis
  ```

  with, for example,

  ```
  # inst.uis: the preselected style, default options (start after installing is off)
  click "Weiter"
  click "Installieren"
  wait 500
  expect "Fertigstellen" exists
  ```

  ```
  # un.uis
  click "Deinstallieren"
  wait 500
  expect "Schließen" exists
  ```

  `apps/installer/tests/nopayload.uis` checks the bare stub (no payload: the error page); it
  runs in `build_rs.py test` and in parity.

**Payload layout** (written by `tools/package.py`, read by `apps/installer/payload.rs`;
little endian, offsets relative to the payload start):

| Part | Layout |
|---|---|
| header | `"AIPK"`, u32 entry count (at most 256) |
| entry (128 bytes each) | u32 kind, u32 stored size, u32 unpacked size, u32 flags (1 = zlib), u64 offset, u64 0, name[48], label[48] (UTF-8, NUL-padded) |
| kinds | 1 manifest (`key=value` lines: `app`, `display`, `version`, `publisher`, `default`, `desc_<style>`), 2 a style variant (the exe; name = style id, label = INFO name), 3 its preview PNG, 5 an extra file (name = its path in the program folder; e.g. PaintCraft's `jxlenc.dll`) |
| trailer | `"ASMINST1"`, u64 stub size, u64 payload size (the last 24 bytes of the file; in a signed setup the last 24 bytes before the Authenticode certificate table and its padding to 8 bytes) |

A signed setup (below) keeps this layout: signtool appends the certificate table after the
trailer and points the PE security directory at it. The installer looks for the trailer before
that table, and writes `uninstall.exe` (the stub) with the security directory cleared, since the
setup's signature covers the payload too.

## Linux: .deb

Built in WSL (`dpkg-deb --root-owner-group -Zxz`, inside the Linux file system for the
permissions). Every variant goes to `/usr/lib/<deb>/<App>-<style>`, the app's modules next to
them (`/usr/lib/PaintCraft/jxlenc.so`: the program looks next to its real path); debconf asks for the look
(`<deb>/style`, a select question), `postinst` registers every variant with
`update-alternatives` and points `/usr/bin/<deb>` at the chosen one. Change it later with
`sudo dpkg-reconfigure <deb>`. Also installed: `/usr/share/applications/<deb>.desktop` and the
hicolor icons. Text files are written with LF line ends (maintainer scripts).
With `filetypes=` / `schemes=` the `.desktop` file lists the MIME types and
`x-scheme-handler/<scheme>` in `MimeType=`, and `Exec` passes `%U` (schemes) or `%F`. `own` types
get a shared-mime-info package `/usr/share/mime/packages/<deb>.xml` (glob patterns,
description), and `postinst` runs `update-mime-database`.

```
# apps/PaintCraft/PACKAGE
filetypes=png|image/png|PNG-Bild; jpg,jpeg|image/jpeg|JPEG-Bild; pdn|image/x-paint.net|Paint.NET-Bild
# an app's own format and its links (myapp://...):
filetypes=myd|application/x-mydoc|MyApp-Dokument|own
schemes=myapp
```

The program receives a file or a URL as a command-line argument (for `%U` Linux file managers
pass local files as `file://` URIs: `Request::paths()` and the `fs.user` grant turn them into
paths, `plat::assoc::file_uri_path` does it for an app's own parsing). With
`AF_SINGLE_INSTANCE` ([GUIDE.md](GUIDE.md) section 7), a running instance receives it as
`UIC_OPEN` instead.

**Without a package** (a developer build, a portable copy): every app built with `ui::main!`
registers itself with `--register` (`--register-replace` also takes defaults of other
programs, `--unregister` removes exactly that again; [GUIDE.md](GUIDE.md) section 21).
`build_rs.py` builds `display`, `app_id`, `categories`, `filetypes` and `schemes` of
`PACKAGE` into the program (`JFGUI_PACKAGE`, checked like here). Windows writes the setup's
keys below `HKCU\Software\Classes`; Linux `~/.local/share/applications/<app_id>.desktop`, the
own types in `~/.local/share/mime/packages/<app_id>.xml` and the defaults in
`~/.config/mimeapps.list` (no `xdg-mime`; `update-mime-database` when installed). An
installed `.deb` and a registration live side by side (`<deb>.desktop` and `<app_id>.desktop`);
on Windows a setup's ProgID with another command counts as another program's (asked, saved
and put back).

```
sudo apt install ./build/dist/PaintCraft_0.1.0_amd64.deb
sudo dpkg-reconfigure PaintCraft          # pick another style
```

Tests never install the package on the real WSL system: check it with
`dpkg-deb --info / --contents` and extract it with `dpkg-deb -x` into a test folder.

## Signing

`tools/package.py` signs what it can and reports the rest: every run ends with
`[sign] signiert: ...` and one `[sign] nicht signiert: <what is missing>` line per gap.
`--sign` turns a gap into an error (a release build). The keys come from the environment or
from files outside the repository; `*.pfx`, `*.p12`, `*.jks`, `*.keystore` and `*.updkey` are
ignored by git. The code is in [`tools/codesign.py`](../tools/codesign.py).

| What | How | Configuration | Without it |
| --- | --- | --- | --- |
| Windows (Authenticode) | `signtool sign /fd SHA256` for every program variant, its modules and further programs (before they go into the payload: the installed files are signed), then the finished `<App>_Setup.exe` | `jfgui_SIGN_PFX` + `jfgui_SIGN_PFX_PASS` (a .pfx), or `jfgui_SIGN_THUMBPRINT` (a certificate in the user's store, also tokens / HSM providers); `jfgui_SIGN_TIMESTAMP` (an RFC 3161 server, recommended); `jfgui_SIGNTOOL` (default: PATH, else the newest Windows Kit) | "signtool.exe nicht gefunden" / "kein Zertifikat (...)" |
| Linux (apt repository) | `--repo DIR`: a flat repository (`pool/`, `Packages(.gz)`, `Release` by `apt-ftparchive`), `InRelease` and `Release.gpg` by gpg, the public key as `repo.gpg` | `jfgui_GPG_KEY` (key id or fingerprint), `jfgui_GPG_HOME` (GnuPG home in Linux / WSL), `jfgui_GPG_PASSPHRASE` (batch signing) | the repository is written unsigned, "jfgui_GPG_KEY setzen" |
| Updates ([UPDATES.md](UPDATES.md)) | `--update DIR`: `update.json` + `update.json.sig` (Ed25519) per style | `--update-key FILE`, `jfgui_UPDATE_KEY_FILE` or `jfgui_UPDATE_KEY` (`python tools/update_manifest.py keygen FILE`) | no channel, "kein Signierschlüssel" |
| MSIX | `signtool sign /fd SHA256` of the finished `.msix` (`AppxSignature.p7x`); the certificate's subject must be exactly the manifest's `Publisher` (signtool error 0x8007000B otherwise; `package.py` says so) | as Authenticode; the Publisher from `jfgui_MSIX_PUBLISHER` or PACKAGE `msix_publisher` | an unsigned package: fine for the Store (it signs), not installable with `Add-AppxPackage` |
| Android | `build_rs.py app NAME --target android --release` signs with the release key (apksigner reads the passwords from the environment) | `jfgui_ANDROID_KEYSTORE`, `jfgui_ANDROID_KEYSTORE_PASS`, `jfgui_ANDROID_KEY_ALIAS`, optional `jfgui_ANDROID_KEY_PASS` | the build fails and names the variables; `jfgui_ANDROID_DEBUG_SIGN=1` signs a release build with the debug key (tests only). Debug builds keep the local debug key |

A user adds the signed repository with:

```
sudo curl -o /usr/share/keyrings/paintcraft.gpg https://example.org/repo/repo.gpg
echo "deb [signed-by=/usr/share/keyrings/paintcraft.gpg] https://example.org/repo ./" | sudo tee /etc/apt/sources.list.d/paintcraft.list
```

apt checks the repository's signature (`InRelease`), and through it the SHA-256 of every
package. A signature inside the `.deb` itself (debsigs) is not made: apt does not check it.

What could not be tested here, and how it was covered instead (`python tools/sign_check.py`):

* There is no real code-signing certificate. `sign_check.py` makes a self-signed one for the
  run with openssl in WSL (never installed into a store), signs a setup with
  `codesign.Signer` (real `signtool`), installs and uninstalls it, and checks that
  `signtool verify` reads the signature (and only refuses the untrusted test root). A setup with
  the certificate-table layout but without signtool is installed as well.
* The apt repository is signed with a throwaway GnuPG key in `/tmp`; `gpgv` accepts
  `InRelease` and `Release.gpg` with `repo.gpg` and refuses a changed `Release`. An
  `apt-get update` against it was not run (no root in the test system).
* Android: a keystore made with keytool for the run; without the variables `--release` fails
  with their names. `sign_check.py --android` builds androidGallery with `--release` and checks
  with `apksigner verify --print-certs` that the APK carries the test certificate. Uploading to
  Google Play (an AAB, Play App Signing) is not covered ("Android App Bundle" below).
* Time stamps (`jfgui_SIGN_TIMESTAMP`) need a time-stamp server on the internet and were not
  tested.

## Other formats: AppImage, RPM, Flatpak, MSIX

`--formats setup,deb,appimage,rpm,flatpak,msix` (or `all`; default `setup,deb`) picks the
packages; `--no-win` / `--no-deb` drop every Windows / Linux format.  The code is in
[`tools/pkg_linux.py`](../tools/pkg_linux.py), [`tools/pkg_msix.py`](../tools/pkg_msix.py) and
[`tools/pkgmeta.py`](../tools/pkgmeta.py) (PACKAGE, PERMISSIONS, `.desktop`, AppStream, icons);
the Linux tools run in Linux or WSL ([`tools/linux_task.py`](../tools/linux_task.py)).  A missing
tool never stops the run: that package is skipped, and the run ends with `[formats] gebaut: ...`,
one `[formats] nicht gebaut: <format>: <what is missing, what was written instead>` line per gap
and `[formats] Hinweis ...` lines.  Nothing is downloaded or installed.

| Format | Output (in `--out`, default `build/dist`) | Styles | Tool (here) |
| --- | --- | --- | --- |
| AppImage | `<App>-<ver>-<style>-x86_64.AppImage` | one file per style | `mksquashfs` (squashfs-tools; present), or `appimagetool` (missing) |
| RPM | `rpm/SPECS/<deb>.spec`, `rpm/SOURCES/<deb>-<ver>-root.tar.gz`, `<deb>-<ver>-1.x86_64.rpm` | all, `update-alternatives` | `rpmbuild` (missing: only the .spec and its tarball) |
| Flatpak | `flatpak/<app id>.json` + `flatpak/files/`, `<app id>.flatpak` (a bundle) | `default_linux` | `flatpak-builder`, the runtime and SDK (present: Platform / Sdk 26.08) |
| MSIX | `<App>-<ver>-x64.msix` (folder: `<App>/msix/<style>/`) | `default_win` | `MakeAppx.exe` of the Windows SDK (present), signtool |

What each format can and cannot do, compared with the setup and the .deb:

| | setup / .deb | AppImage | RPM | Flatpak | MSIX |
| --- | --- | --- | --- | --- | --- |
| choice of style | at install time | one file per style | `update-alternatives --config` | `default_linux` only | `default_win` only |
| file types, URL schemes | registered | in the `.desktop` file; registered by an integration tool (appimaged, AppImageLauncher) or the app's `--register` (the `.desktop` file then starts `$APPIMAGE`) | registered (`.desktop`, shared-mime-info; the system's file triggers) | exported `.desktop` and MIME package; files arrive through the document portal as `file://` URIs (`--file-forwarding`), which the app takes as paths | `windows.fileTypeAssociation`, `windows.protocol`; an `own` type is offered, never made the default (Windows asks the user); each extension once |
| autostart (`plat::autostart_set`) | Run key / XDG autostart | XDG autostart with `$APPIMAGE` | XDG autostart | Background portal | `Unavailable` (HKCU is virtualized); the manifest's StartupTask, switched on in Settings -> Apps -> Startup |
| `lib/update` ([UPDATES.md](UPDATES.md)) | exe, bundle, package | no (it would replace the extracted copy; the update information for AppImageUpdate is supported) | the channel lists only the .deb | no: updates come from the Flatpak repository | no (the install folder is read-only): Store or App Installer |
| further programs (BINS) | next to the program, linked into `/usr/bin` | inside, AppRun starts the main program | `/usr/bin` links | in `/app/lib/<deb>/`, not in PATH | next to the program |
| signing | Authenticode, apt repository | not signed (no `.sha256_sig`) | not signed (rpmsign) | the bundle is not signed (a repository's GPG key) | signtool (below) |

### AppImage

The AppDir: `AppRun` (a link to `usr/lib/<deb>/<deb>`, the program of the style), the
`<app id>.desktop` file and `<app id>.png` at the top, `.DirIcon`, `usr/bin/<deb>`, the program's
modules, further programs and notices in `usr/lib/<deb>/` and `usr/share/doc/<deb>/`, the hicolor
icons, `usr/share/metainfo/<app id>.appdata.xml` (AppStream) and the MIME package of `own` types.
Built in the Linux file system (modes: 0755 for programs and folders, 0644 else), packed by
`mksquashfs -comp zstd -all-root` (reproducible with `SOURCE_DATE_EPOCH`) and put behind the
runtime.  With `appimagetool` in PATH that tool makes the image instead, with the same runtime
(`--runtime-file`); that path is not tested here.

**The runtime** is [`apps/appimagert`](../apps/appimagert/src/main.rs), a static Linux program
of JFLib without libc (about 200 KB, built with `--profile size`); `package.py` makes it an
AppImage type 2 runtime: the magic `AI\x02` at byte 8 of the ELF header and a section
`.upd_info` of 1024 bytes (PACKAGE `appimage_update=`, e.g. `zsync|https://example.org/App-latest-x86_64.AppImage.zsync`),
added to the file by `pkg_linux.runtime_image`.  The SquashFS image starts where the ELF data ends.
The upstream runtime would mount the image with FUSE, and is downloaded by appimagetool; this one
reads the image itself ([`sqfs.rs`](../apps/appimagert/src/sqfs.rs): SquashFS 4.0, zstd or lz4,
fragments, sparse blocks, extended inodes) and needs no FUSE:

* The first start extracts the image into `$XDG_CACHE_HOME/appimage-run/<name>-<xxh64 of the
  image>` (a part folder, renamed into place: parallel starts and broken runs never leave a half
  folder in use); later starts check the hash (a quick read of the file) and reuse the folder.
  Folders of other versions of the same file name that were not used for 7 days are removed.
  The first start of a large app therefore takes as long as unpacking it.
* Then `AppRun` replaces the runtime (`plat::process::exec`, execve) with `APPIMAGE`, `APPDIR`,
  `ARGV0` and `OWD` set.  A relative `AppRun` link is followed first, so the program's `argv[0]`
  is its own name: JFLib uses it as the window class and the Wayland app id, and the `.desktop`
  files carry it as `StartupWMClass` (the desktop matches the window to the entry).
* Options: `--appimage-extract [PATTERN]` (into `./squashfs-root`), `--appimage-extract-and-run`,
  `--appimage-offset`, `--appimage-updateinformation`, `--appimage-portable-home` /
  `-config` (`NAME.AppImage.home` / `.config`: `HOME` / `XDG_CONFIG_HOME` of the app),
  `--appimage-version`, `--appimage-help`; `--appimage-mount` is refused.  `TARGET_APPIMAGE=FILE`
  uses another file's image.
* `--appimage-runtime FILE` or `jfgui_APPIMAGE_RUNTIME` packs another runtime (for example the
  upstream `runtime-x86_64`, which needs FUSE on the user's machine).

### RPM

A binary package from the prebuilt files: the `.spec` takes the tree as `Source0`
(`<deb>-<ver>-root.tar.gz`, written by Python with root as owner and the modes of the .deb) and
lists every file in `%files`; the package owns `/usr/lib/<deb>` and `/usr/share/doc/<deb>`.  The
tree is the .deb's, except that systemd units go to `/usr/lib/systemd/system` (`/lib` is a link on
RPM systems).  There is no install-time question: every style is an alternative of
`/usr/bin/<deb>` (`default_linux` with priority 50), `sudo update-alternatives --config <deb>`
switches; `%preun` removes them (and the BINS links, the units) on removal, not on an upgrade.
Desktop database, icon cache and MIME database are left to the file triggers of Fedora / openSUSE.
`AutoReqProv: no` (the programs are static), no debuginfo, no strip, `ExclusiveArch: x86_64`.
The Version takes `~` for `-` (`1.2.3-beta` -> `1.2.3~beta`).

Not tested here: `rpmbuild` is not installed in the test system (Ubuntu, no root), so only the
`.spec` and its tarball are written; `tools/pkg_check.py` compares `%files` with the tarball and
runs `sh -n` on the scriptlets.  With rpmbuild (`dnf install rpm-build`):
`rpmbuild -bb --define "_topdir $PWD" SPECS/<deb>.spec` in `build/dist/rpm`, or simply run
`package.py --formats rpm` there.

### Flatpak

The manifest `flatpak/<app id>.json` (flatpak-builder, JSON) builds one module from the
prebuilt files in `flatpak/files/` (a `dir` source; the programs installed with `install -Dm755`,
`/app/bin/<deb>` a link to `/app/lib/<deb>/<deb>`, `share/` copied): the program of
`default_linux`, its modules, further programs and notices, `<app id>.desktop` (with
`StartupWMClass`), the icons, `<app id>.metainfo.xml` and the MIME package.  Runtime
`org.freedesktop.Platform//26.08` (PACKAGE `flatpak_runtime=`), SDK to match; the static program
uses nothing of it.  flatpak-builder exports the build into a temporary repository and
`flatpak build-bundle` writes `<app id>.flatpak` (`flatpak install --user <file>` installs it).
Flathub builds from sources and would need a manifest that builds JFLib itself; this one is for
own repositories and bundles.

**finish-args from PERMISSIONS** (`pkg_linux.finish_args`): always `--share=ipc`
`--socket=wayland` `--socket=fallback-x11`; then

| PERMISSIONS | finish-args |
| --- | --- |
| `fs.read P` / `fs.write P` | `--filesystem=LOCATION:ro` / `--filesystem=LOCATION`: `$HOME/x/**` -> `~/x`, `$DOCUMENTS` -> `xdg-documents`, `$PICTURES`, `$DESKTOP`, `$DOWNLOADS`, `$MUSIC`, `$VIDEOS` likewise, `/abs/dir/*.x` -> `/abs/dir` (the path up to the first pattern), `**` -> `host`; `$CONFIG`, `$APPCONFIG`, `$APPDATA`, `$APPCACHE`, `$APPLOG`, `$TEMP`, `$EXE`, `$FONTS` need none (inside the sandbox); Windows paths are left out |
| `plugins P` | as `fs.read` |
| `net.connect`, `net.listen` | `--share=network` |
| `secrets` | `--talk-name=org.freedesktop.secrets` (the Secret Service; plat does not use the Secret portal) |
| `fs.user`, `url`, `print`, `autostart` | none: the file dialog, OpenURI, Print and Background portals; files of "Open with" come through the document portal |
| `process` | none: programs outside the sandbox are not reachable (`flatpak-spawn --host` would need `--talk-name=org.freedesktop.Flatpak`, a full escape: not granted; noted) |
| no PERMISSIONS | `--share=network --filesystem=host --talk-name=org.freedesktop.secrets` (the app may do everything; noted) |

`flatpak_args=` adds more, for example `--talk-name=org.kde.StatusNotifierWatcher` for the tray
(`plat::tray`), `--talk-name=org.freedesktop.Notifications` or
`--filesystem=xdg-data/recently-used.xbel` (the host's list of recent files for
`plat::recent_add`; without it the list stays in the sandbox's own data folder).  The exported
`.desktop` file's `Exec` gets flatpak's file forwarding (`@@u %U @@`): opened files come as
`file:///run/user/<uid>/doc/...` URIs that `plat` turns into paths and grants (`fs.user`).
With `fs.user` every document of the portal's mount stays readable and writable without a
new grant, so a file the user chose in the portal's file dialog (a lasting document) opens
again from `ui.recent_add`'s list after a restart; files of "Open with" are documents for the
session only (flatpak forwards them so) and are gone after it ([VFS.md](VFS.md), "Flatpak").
`--register` is `Unavailable` inside the sandbox (the export registers).  Not covered: files dropped from
other programs arrive as paths the sandbox may not see (no portal file transfer), and the
library's own file dialog (used without a portal) sees only the sandbox.

### MSIX

`AppxManifest.xml` (`pkg_msix.appx_manifest`) for a full-trust desktop app
(`Windows.FullTrustApplication`, `runFullTrust`): the identity (`msix_name`, default the app id
with `-` for `_`; `Publisher` = the signing certificate's subject: `jfgui_MSIX_PUBLISHER`, PACKAGE
`msix_publisher`, default `CN=<publisher>`; the version as `a.b.c.0`), display name, description,
the logos `Assets/{StoreLogo,Square44x44Logo,Square150x150Logo,Wide310x150Logo}.png` (scaled from
the app's icons), `Resource Language` (PACKAGE `language`, default `de-DE`); from PACKAGE the
`windows.fileTypeAssociation` per file type and a `windows.protocol` per URL scheme (Windows'
reserved schemes and an extension in two file types are refused; reserved types like `.exe`,
`.bat`, `.lnk` and their content types MakeAppx names itself); a full-trust app gets the file
or URL as its command-line argument; a `desktop:StartupTask` (`Enabled="false"`) when PERMISSIONS
allows `autostart` (or there is no PERMISSIONS); capabilities: `internetClient` for `net.connect`,
`internetClientServer` and `privateNetworkClientServer` for `net.listen` (a full-trust app is not
confined by them; they are shown in the Store, PERMISSIONS stays what plat enforces).  The
package folder holds `<App>.exe` (signed like the setup's, when a certificate is set), modules,
further programs and notices; `MakeAppx pack` checks the manifest against the schema.

In an MSIX package Windows virtualizes writes below HKCU and AppData: the app's `config://` and
`data://` live in the package's private folder (removed with it), and `plat::autostart_set` is
`Unavailable` (`win_autostart.rs` asks `GetCurrentPackageFullName`), and so is `--register`
(`plat::assoc`): the manifest declares the types.  Switching the StartupTask
from the app (WinRT `StartupTask.RequestEnableAsync`) is not implemented.  No `resources.pri`:
the logos have no scale, `targetsize` or `altform-unplated` variants (Windows draws the small logo on a plate).
Not tested here: installing the package (`Add-AppxPackage` needs a trusted certificate or
developer mode; nothing is installed by the checks) and a Store submission (the Store takes the
identity reserved in Partner Center: set `msix_name` and `msix_publisher` to it).

### Android App Bundle (not built)

Google Play takes Android App Bundles (`.aab`), not APKs.  `tools/android.py` builds a signed APK
(`aapt2 link`, d8, zipalign, apksigner).  An AAB would need, in `build_apk`'s place:

1. `aapt2 link --proto-format` (instead of the binary resource table; aapt2 of build-tools 36 has it),
2. the base module as a ZIP: `manifest/AndroidManifest.xml`, `resources.pb`, `res/`, `dex/classes.dex`,
   `lib/<abi>/libjfgui_android.so` (both ABIs in one bundle: arm64-v8a and x86_64), `assets/`,
3. `java -jar bundletool.jar build-bundle --modules=base.zip --output=<App>.aab`,
4. signing with `jarsigner` (apksigner does not sign bundles) and the upload key; Play App Signing
   then signs the APKs Play makes.

bundletool is not part of the Android SDK; it is a JAR on GitHub (google/bundletool), and it is
not on this machine, so the AAB is not built (no download without the user's consent).
`tools/pkg_check.py` reports what is there (SDK, build-tools, aapt2, Java, bundletool).

## Checks

[`tools/pkg_check.py`](../tools/pkg_check.py): `--quick` runs in `build_rs.py test` (a few
seconds, nothing built): the fixture app's metadata in every format (`.desktop`, AppStream, the
finish-args of every PERMISSIONS key, the `.spec` and its tarball, the AppxManifest with file
types, protocol, StartupTask and capabilities, escaping), every format with its tools taken away
(skipped with what is missing, the AppDir / `.spec` / manifest / package folder still written),
the `.desktop` entry, metainfo and AppxManifest of every `apps/*/PACKAGE` (validators, MakeAppx
pack with a stand-in program; desktop-file-validate's hints are shown, not counted),
the shipped notices (`THIRD_PARTY_NOTICES.md` names every file of `licenses/`, each style
carries the notices of the styles whose fonts or icons it embeds), `runtime_image` on a small
ELF file,
desktop-file-validate, `appstreamcli validate --no-net`, `sh -n` on the scriptlets, MakeAppx pack
of the fixture package, and the runtime against SquashFS images made by mksquashfs (zstd with
4 KiB blocks: fragments, several blocks, sparse and stored blocks; lz4 without fragments; links,
a hard link, 300 entries in one folder): `--appimage-extract` gives the same tree, a start runs
AppRun with the variables and reuses the cache, a cut image fails with a message.

Without `--quick` (by hand, a few minutes) it runs `package.py template --formats all` into
`build/pkg_check/dist` and checks each package: the AppImage's header, `.upd_info` and image
offset, its AppDir (validators, `unsquashfs` at `--appimage-offset`) and the template's
`tests/*.uis` run through the AppImage; the `.spec` against its tarball; the Flatpak manifest,
its files and validators, a second flatpak-builder run whose `metadata` holds the finish-args,
the exported `.desktop`, and the scripts run in the build's sandbox (`flatpak build`); the MSIX
package's files, its manifest read back, `MakeAppx unpack`, a signature with a certificate made
for the run (the host's openssl when there is one: the WSL clock may run ahead of Windows' and
make a fresh certificate "not yet valid"; never installed), `signtool verify` (the untrusted test
root is expected), and a certificate of another subject refused with 0x8007000B.

## Third-party notices

[`THIRD_PARTY_NOTICES.md`](../THIRD_PARTY_NOTICES.md) lists the third-party code, data,
fonts, and artwork in JFLib; the license texts are in `licenses/`. `build_rs.app_notices`
collects what an app ships: that file and `licenses/` in the repository's layout,
`lib/text_shape/NOTICE.md` with its crate licenses as `licenses/text_shape/`, and for each
style the license files in `styles/<style>/` and `styles/<style>/assets/` (names with
`license`, `OFL`, `NOTICE`, or `COPYING`) as `licenses/<style>/`, together with those of the
styles whose fonts or icons it embeds (`include_bytes!("../<other>/...")`). Loose builds
carry them next to the executable (style notices in `licenses/<app>/<style>/`), the setup,
MSIX, `.deb`, `.rpm`, AppImage, and Flatpak packages in the installation folder or
`/usr/share/doc/<package>/`, and APKs below `assets/`. In the setup's payload, notices whose
path is longer than 47 bytes are combined into one `ALL-LICENSES.txt` per library folder.
Preserve these files when distributing an app.

The Rust build uses pinned Cargo dependencies for OpenType/Unicode shaping. The
first build needs Cargo and a populated cache or registry access. G'MIC, Paint.NET and
third-party plugins are separate installations and are not bundled.
