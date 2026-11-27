# Changelog of jflib-installer

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

### Added

- The setup speaks the user's language: German, English, French, Spanish, Italian, Dutch, Polish, Arabic and Hebrew (lib/installer/setup.tsv, an i18n::Catalog), otherwise English; --lang CODE chooses. The styles' names and descriptions follow when the manifest has them (label_<style>.<lang>, desc_<style>.<lang>, from styles/S/INFO). Arabic and Hebrew mirror the window, but their text is drawn without bidi reordering, backwards, as in all of the library's controls (ROADMAP section 13, U7)
- Installing for all users: the options page asks who the app is for. All users get C:\Program Files\<app>, the common desktop and Start menu and HKLM for the uninstall entry, the file types and a browser's registration (install.ini scope=machine); a setup without administrator rights starts itself again with them (the UAC prompt, quiet, waited for) and carries on, a refused prompt leads back to the options. Its uninstall.exe does the same. When the app is installed in Program Files and not for the user, the setup preselects all users
- Installing and removing without questions: --quiet (no window) with --all-users / --current-user, --dir, --style, --no-desktop, --no-start-menu, --no-uninstall-entry, --launch and --log FILE; exit codes 0 done, 1 failed, 2 a wrong command line (an unknown option or style), 740 for all users without administrator rights. The uninstall entry carries uninstall.exe --uninstall --quiet as its QuietUninstallString. Without --quiet the options preset the pages

### Changed

- A payload with one style (PACKAGE styles= with one name, the default of cargo jflib package) has no style page: the setup starts on the options page, which has no "Zurück" then. "Start the app" and a browser's "Make it the default browser" moved to the last page, where Fertigstellen does them; scripts that click "Weiter" on a setup of one style click "Installieren" at once

### Fixed

- The uninstaller deletes uninstall.exe and its folder once it has ended, trying once a second for up to five minutes: it tried once after two seconds, and Windows deletes no running program, so both stayed whenever its last page was open longer (not checked on Windows: Wine deletes running programs). The removal page said the files were being installed
- A key of the app's own that exists with another program's command (a scheme such as mailto) is neither written nor removed; a setup for all users overwrote the computer's handler and its uninstaller removed the key, for every user (setup.log: kept)

### Security

- An installation for all users puts its folder (and the payload's folders) under the administrators with a DACL of its own (SYSTEM and administrators full, users read and run) and takes only a new, empty or its own folder: in a folder chosen outside Program Files (D:\Apps) users could swap the program or the uninstaller that runs with administrator rights, or put a DLL beside them
- The uninstaller removes only shortcuts the setup made (a .lnk in the desktop or start menu folder of the scope); it deleted any path install.ini named, with administrator rights for an installation for all users

## [0.2.1] - 2026-11-16

### Security

- Windows: URL schemes are registered as `"<exe>" -- "%1"` and plat::args keeps the URL after -- as one argument, so a quote in a link cannot add options; ui stops reading options at --. Apps registered before keep the old command until they are registered again

## [0.2.0] - 2026-11-14

### Added

- Web browsers (`browser=1` in the app's `PACKAGE`): the setup registers the app as a browser in
  `HKCU` -- the ProgID `<app>URL`, `Software\Clients\StartMenuInternet\<app>` with its
  capabilities (the URL and file associations) and the app in `RegisteredApplications` -- instead
  of keys of `http` and `https` of its own, and `uninstall.exe` removes them with the rest.
- The options page of a web browser's setup offers "Als Standardbrowser festlegen" ("Make <App>
  my default browser" in other languages than German), checked by default. Finishing the setup
  then opens Windows' Settings at the app's page of the default apps, after starting the app when
  that is chosen too. Test mode and script runs only print the page's address.

### Changed

- A payload holds up to 65536 entries (was 256): an app with std and the licences of many crates
  did not fit.

### Fixed

- `install.ini` keeps the whole list of the installed files: its `files=` line was cut at 4096
  bytes (and the file at 8192), so `uninstall.exe` left the rest behind. Installing over an older
  installation writes the whole list.
- `uninstall.exe` removes every listed file and then the folders below the program folder that
  held them, deepest first, when they are empty (it removed only each file's own folder), and
  skips names outside the program folder (absolute ones, `..`).
- A setup that cannot write `install.ini` fails instead of ending without it; the uninstaller
  found nothing to remove then.
- The uninstaller deletes no registry key outside the app's own: an `assoc=` entry of
  `install.ini` that names another key (a list cut by an older setup, such as an entry `Software`
  of `Software\Classes\App.ext`) is refused instead of deleted with everything below it.

## [0.1.0] - 2026-11-08

First release on crates.io.

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.1...HEAD
[0.2.1]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
