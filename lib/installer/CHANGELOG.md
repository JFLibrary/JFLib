# Changelog of jflib-installer

What changed in this crate. All of JFLib's crates share one version and are released together;
the [library's changelog](https://github.com/JFLibrary/JFLib/blob/main/CHANGELOG.md) has the
whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),
versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's
[VERSIONING.md](https://github.com/JFLibrary/JFLib/blob/main/docs/VERSIONING.md) describes.

## [Unreleased]

## [0.2.0] - 2026-11-13

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

[Unreleased]: https://github.com/JFLibrary/JFLib/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/JFLibrary/JFLib/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/JFLibrary/JFLib/releases/tag/v0.1.0
