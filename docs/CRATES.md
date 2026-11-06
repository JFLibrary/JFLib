# Crates-Plan: JFLib für crates.io

Stand 2026-11-05. CR0 bis CR2 sind umgesetzt, das Ziel „lokal kompatibel“ ist erreicht. CR3 ist umgesetzt:
`build_rs.py` baut nur noch über Cargo. Von CR4 sind `new`, `test`, `web`, `android` und `package` mit Setup
und `.deb` umgesetzt, offen sind die übrigen Paketformate, das Signieren, `watch`, `plugin` und CR5, CR6. Die Ergebnisse unter „Was schon geprüft ist“
stammen aus einer Kopie der Quellen außerhalb des Repos, die Größen der Stufen sind geschätzt.

## Ziel

Ein fremdes Projekt trägt eine einzige Abhängigkeit ein, baut eine JFLib-App und startet sie, ohne dieses Repo
und ohne Python:

```toml
[dependencies]
jflib = "0.1"
```

Der erste Schritt ist **lokal kompatibel**, noch ohne Upload. Dieser Schritt ist mit [CR2](#cr2-fremde-projekte)
erreicht. Das Hochladen ist eine eigene Stufe ([CR6](#cr6-auszug-und-veröffentlichung)) und braucht eine eigene
Zusage, weil es sich nicht zurücknehmen lässt.

Nicht Teil dieses Plans: ein Umbau der eigenen Runtime, ein Betrieb mit `std`, der Stil `jfoffice`, macOS und iOS.

## Entscheidungen

| Thema | Entscheidung |
| --- | --- |
| Kompilieren | Cargo |
| Alles andere | ein Werkzeug über Cargo: zuerst `build_rs.py`, für Crate-Nutzer später `cargo jflib` in Rust |
| Runtime | bleibt die eigene (`rt`, `plat`), Cargo ändert daran nichts |
| Einstieg | nur `jflib`, keine Features; alles wird immer mitgeladen, soweit es auf dem Ziel kompiliert |
| Namen | Präfix `jflib-`, im Code weiter `rt::`, `ui::` über `[lib] name` |
| Ordner | kein `crates/`, `lib/` und `styles/` bleiben |
| Inhalt | alles aus `lib/` außer `script` und `plugin`; `modrt` und `modplat` als eigene Crates |
| Stile | `winui`, `classic`, `kde`, `macos`; `jfoffice`, `backupcraft` und `netlab` bleiben draußen |
| JFScript | `apps/jfs/crates/script` mit `publish = false`, eigenes Repo erst zum Crate-Release |
| Apps | bleiben bis zum Crate-Release im Repo, immer mit `publish = false` |
| Version | eine für alle Crates, sie steht in der `Cargo.toml` im Wurzelordner; untereinander exakt (`=0.1.0`), weil nur der Weg über `jflib` versprochen ist |
| Offene Worktrees | bleiben lokal und passen sich später an |

## Zielbild

```text
Cargo.toml                          Workspace: Version, Profile, Lints
Cargo.lock
lib/<crate>/Cargo.toml              49 Crates wie heute
lib/style_assets/                   neu: Fonts und Icons der Stile
lib/jflib/                          neu: die Fassade, die einzige Abhängigkeit eines Nutzers
lib/jflib_build/                    neu: Helfer für die build.rs einer App
styles/{winui,classic,kde,macos}/Cargo.toml
modules/{modrt,modplat}/Cargo.toml  veröffentlicht, aber nicht in der Fassade
modules/jxlenc/Cargo.toml           publish = false
apps/*/Cargo.toml                   publish = false
apps/jfs/crates/script/Cargo.toml   publish = false
```

Das ergibt 57 veröffentlichte Crates. Ein fremdes Projekt sieht davon nur diese Struktur:

```text
hallo/                       die Vorlage lib/jflib/template, im Paket von jflib
  Cargo.toml                 jflib, jflib-build, die Profile unten
  .cargo/config.toml         rust-lld; für Linux das Relocation-Modell und glibc
  build.rs                   jflib_build::app();
  PERMISSIONS, PACKAGE       optional
  assets/, res/app.ico       optional
  src/lib.rs                 die App: use jflib::{style, ui};  ...  ui::main!(...)
  src/main.rs                bindet die App nur ein, so baut derselbe Code auch für den Browser
```

Diese Profile braucht jedes Projekt, denn Cargo übernimmt sie nicht von einer Abhängigkeit:

```toml
[profile.dev]
panic = "abort"
opt-level = 1
debug = 0
debug-assertions = false
overflow-checks = false

[profile.release]
panic = "abort"
opt-level = 2
codegen-units = 1
lto = "fat"
debug = 0
strip = "symbols"
```

Die Aufgaben verteilen sich so:

| Schicht | Aufgaben |
| --- | --- |
| Cargo | `build`, `run`, `check`, `doc`, `--release`, `--target` |
| `cargo jflib` (für Crate-Nutzer) | `new`, Pakete (`setup.exe`, `.deb`, AppImage, RPM, Flatpak, MSIX), APK, Web-Bundle, `test`, `watch`, `plugin` |
| `build_rs.py` und `tools/` (nur im Repo) | `release`, `changelog`, `api-diff`, `site`, `i18n` |

## Was schon geprüft ist

Rust 1.98.1 unter Windows, Vergleiche jeweils aus einem einzigen Lauf.

| Prüfung | Ergebnis |
| --- | --- |
| `cargo package --workspace` mit Verifikation | alle 57 Crates bauen aus ihrem Paket; zusammen 6,2 MB, das größte 1,5 MB (`doc_view`) |
| Fremdes Projekt gegen die entpackten Pakete, Dev und Release | baut, `smoke.uis` läuft durch, `app://` liefert die Assets, `PERMDENY` greift, `rt::app_version!()` zeigt die Version |
| Alles in der Fassade gegen nur das Nötige (`template`) | Programm byte-gleich groß; erster Build Dev 199 s statt 121 s, Release 221 s statt 172 s |
| Stil ohne Feature (`JFLIB_STYLE`, sonst `winui`) | Screenshot zeigt den gewählten Stil |
| Windows-GUI ohne `build.rs` und Konfiguration | läuft mit `#![windows_subsystem = "windows"]`, nur System-DLLs |
| Linux, unter Windows gebaut | statisch und gegen glibc lauffähig in WSL |
| Web scalar und Web mit Threads | beide im Browser gestartet; Threads nur ohne Laufzeitprüfungen, siehe [Risiken](#risiken) |
| Android | `.so` mit denselben JNI-Exporten und Importen wie `build_rs.py`, 2,5 % größer; nicht auf einem Gerät gestartet |
| `jxlenc` über `modrt` und `modplat` | Export `jxlenc_module`, keine Importe; nicht in PaintCraft geladen |
| Inspector | Dev-Build kann bearbeiten, Release-Build verweigert es |
| `cargo doc` für Linux | läuft, 561 Warnungen; docs.rs selbst nicht getestet |

Zwei Dinge haben diese Versuche ausgeschlossen:

- **`rt` je nach Ziel aus einer anderen Quelle:** Cargo lehnt das ab („different source paths depending on the
  build target“). Deshalb die eigenen Namen in CR0.
- **`trim-paths`:** In Cargo 1.98 noch nicht stabil. Wie die Pfade verschwinden, steht unter [Risiken](#risiken).

## Stufen

### CR0: Quellen vorbereiten

**Erledigt am 2026-11-05.** `build_rs.py` baut weiter wie vorher. Die Release-Programme `template`, `jfs`, `PaintCraft`
und `jxlenc` sind unter Windows byte-gleich groß wie vor CR0, unter Linux wächst nur `jfs` um 144 Byte (längere Pfade
in den Panic-Orten). Noch wirkungslos sind Punkt 4 und die Datei-Form aus Punkt 5, sie greifen erst mit den Manifesten.

1. **JFScript umziehen.** `git mv lib/script apps/jfs/crates/script`, den Eintrag aus `LIBS` und aus `CRATES` in
   `tools/gendoc.py` streichen. `local_crates()` sieht die Crate dann von allein, auch für JFNetworker, `coretest`
   und `editortest`. Die Pfade in den Kommentaren dieser Apps mitziehen.
2. **`lib/style_assets` anlegen.** `font_reg.bin`, `font_semi.bin`, `font_bold.bin` und `icons.bin` aus
   `styles/winui/assets` dorthin, die vier Stile nutzen die `pub static`s. Sie liegt in `lib/`, weil `STYLES` jeden
   Ordner unter `styles/` als Stil zählt. Die Lizenzdateien liegen als Kopie daneben, jeder Stil behält seine eigenen.
3. **`modrt` paketfähig machen.** Seine acht Dateien aus `lib/rt`, `lib/plat` und `lib/plugin` liegen außerhalb des
   Pakets. Sie kommen als Kopie nach `modules/modrt/shared/`, `lint()` hält sie byte-gleich.
4. **Module ohne Tausch der Abhängigkeit.** In `simd`, `gfx`, `img_core`, `img_jpeg`, `img_jxl` und `img_jxl_enc`
   kommt an die Wurzel `#[cfg(jflib_module)] extern crate modrt as rt;`, wo nötig auch für `plat`. `lint()` muss
   das erlauben.
5. **`ui::main!` und `rt::main!` erweitern.** Sie lesen `PERMISSIONS` und `PACKAGE` zusätzlich aus einer Datei
   (`JFGUI_PERMISSIONS_FILE`, `JFGUI_PACKAGE_FILE`), denn `cargo::rustc-env` kann nur eine Zeile. Dazu
   `#[allow(unexpected_cfgs)]` und ein `compile_error!` bei `cfg(panic = "unwind")`, das die fehlende Zeile nennt.
   Das `compile_error!` gehört ins Makro, in einer Bibliothek würde es `cargo package` und `cargo doc` stoppen.

**Geprüft:** `build_rs.py test --quick` lief mit 1321 von 1331 Skripten durch. Die zehn anderen: `fonttest/selftest` auf
beiden Zielen (nur CRLF, mit LF-Kopie grün) und `jfWord/core.uis` mit acht Folgefehlern unter Windows (allein 53 von 53
grün). Web und Android bauen.

### CR1: Manifeste

**Erledigt am 2026-11-05.** `build_rs.py` kompiliert weiter selbst, liest aber nur noch die Manifeste
(`tools/manifests.py`). `lib/plugin` hat kein Manifest, es gehört nicht in die Crates. Das Manifest von JFScript kommt
mit denen der Apps in CR3. `build_rs.py check` prüft zusätzlich, dass die Fassade jede Crate nennt, das Manifest eines
Stils alle Crates seiner Quellen und die Lizenzkopien gleich sind. Die Werkzeuge brauchen dafür Python 3.11 (`tomllib`).

1. **Wurzel und Crates.** `Cargo.toml` und `Cargo.lock` im Wurzelordner, `/target` in `.gitignore`, ein Manifest pro
   Crate. Einmal per Skript erzeugen, danach von Hand pflegen.
2. **Eine Wahrheit.** `LIBS` und `TARGET_DEPS` werden aus den Manifesten gelesen (`tomllib`).
3. **`text_shape`.** Wird `jflib-text-shape` und Mitglied des Workspace, ihre eigene `Cargo.lock` und ihr
   `[profile.release]` entfallen. `build_text_shape` muss das Profil danach selbst setzen.
4. **Version.** `lib/VERSION` entfällt, `rt::VERSION` kommt aus `CARGO_PKG_VERSION`. `build_rs.py` setzt diese
   Variable für `rustc`. Anzupassen sind `release`, `api-diff`, `tools/apidiff.py` und `docs/VERSIONING.md`.
5. **Fassade.** `lib/jflib` mit allen Crates. `netio`, `datastore`, `cas` und `update` hängen an
   `cfg(not(any(target_arch = "wasm32", target_os = "android")))`, weil `plat` dort kein `fs` und `net` hat.
   `img_jxl_enc` fehlt auf Web, weil er für Wasm viel zu lange kompiliert ([Risiken](#risiken)); wer ihn dort
   braucht, trägt ihn direkt ein. Der Stil kommt aus `JFLIB_STYLE`, ohne Angabe `winui`.
6. **Pflichtangaben.** `description`, `license`, `repository`. Lizenzen pro Crate aus `THIRD_PARTY_NOTICES.md`
   zuordnen, zum Beispiel libaom zu `img_av1`, jxrlib zu `img_jxr`, DirectXTex zu `img_dds`, DejaVu zu `doc_view`,
   fdlibm zu `fx`, OFL und Fluent-Lizenz zu `style_assets`. Die Dateien liegen im jeweiligen Paket: `LICENSE-MIT` und
   `LICENSE-APACHE` in jeder Crate, dazu ihr Teil von `licenses/`, jeweils als Kopie.
7. **Eine Version im Build.** `links = "jflib_rt"` an `rt` (braucht eine leere `build.rs`), damit zwei Versionen
   im selben Programm als klare Meldung enden und nicht als E0152. Mit zwei kleinen Paketen geprüft.
8. **Ältere Worktrees.** `exclude = [".claude"]` in der Wurzel: Ohne die Zeile halten Worktrees unter `.claude`, die
   noch keine eigene Wurzel-`Cargo.toml` haben, ihre `text_shape` für ein Mitglied dieses Workspace, und ihr Build
   bricht ab.

**Geprüft:** `cargo check -p jflib` läuft für Windows, Linux, Linux mit `jfgui_glibc`, Web und Android ohne Warnung
durch. `jxlenc` baut mit `--cfg jflib_module` (Export `jxlenc_module`, keine Importe, 512 Byte größer als aus
`build_rs.py`, nicht in PaintCraft geladen). `cargo package --workspace --exclude jflib-module-jxlenc` verifiziert
alle 56 Crates in 347 s, zusammen 6,3 MB. Die Release-Programme `template`, `jfs`, `PaintCraft` und `jxlenc` sind unter
Windows byte-gleich groß wie vor CR1, unter Linux ist `jfs` 16 und `template` 160 Byte kleiner. `build_rs.py test --quick` lief mit 1329 von 1331 Skripten durch, es fehlt nur `fonttest/selftest` auf beiden Zielen (CRLF, wie in CR0).

### CR2: Fremde Projekte

**Erledigt am 2026-11-05. Damit ist das Ziel „lokal kompatibel“ erreicht.** Ein Projekt außerhalb des Repos baut aus
den gepackten Crates allein, offline, für Windows, Linux und den Browser, und sein Smoke-Test läuft.

- **`jflib-build`** (`lib/jflib_build`, kein Teil von `LIBS`, mit `std`): `app()` und `console()`. `PERMISSIONS` und
  `PACKAGE` prüft es mit denselben Meldungen wie `build_rs.py`, die Fälle von `PERM_CASES` sind seine Tests. Das
  Asset-Bundle ist byte-gleich mit dem von `tools/assetpack.py`, das Icon (`res/app.ico`) byte-gleich mit dem, was
  `llvm-rc` daraus macht, ohne Ressourcen-Compiler. Die Version ist die aus Cargo, ein `version=` in `PACKAGE` muss
  ihr gleichen. Für Linux mit `jfgui_glibc` baut es die Stub-Bibliotheken selbst, mit `rustc` und dem `rust-lld` aus
  rustup. Es beobachtet nur Dateien, die es gibt: Eine fehlende Datei würde Cargo das Programm bei jedem Build neu
  übersetzen lassen. Nach dem ersten Anlegen von `PERMISSIONS` & Co. braucht es deshalb `cargo clean -p NAME`.
- **Vorlage** `lib/jflib/template` im Paket von `jflib`. Die App liegt in `src/lib.rs`, `src/main.rs` bindet sie nur
  ein. So baut `cargo rustc --lib --crate-type cdylib` dieselbe App für den Browser, ohne Cargos Warnung wegen
  derselben Datei in zwei Zielen. `src/lib.rs`, die Übersetzungen und `tests/smoke.uis` kommen aus `apps/template`,
  `build_rs.py check` hält sie gleich (`python tools/crates_accept.py --template`).
- **Doku:** `lib/jflib/README.md` mit Schnellstart, Profilen, Zielen und den Fallstricken unten. `docs/GUIDE.md` und
  `docs/API.md` kopiert `crates_accept.py` vor dem Packen nach `lib/jflib/docs` (nicht in Git), `include` der
  Fassade nimmt sie mit.

1. **`jflib-build`.** Liefert Name und Version aus Cargo, `PERMISSIONS` und `PACKAGE` mit Syntaxprüfung samt
   Zeilenangabe (heute `app_permissions` und `perm_split`), `assets/` als JFA1 mit LZ4 über `compress`, die
   Link-Argumente pro Ziel. Der Prototyp speichert die Assets noch unkomprimiert. Konsolenprogramme brauchen
   `/ENTRY` und `/SUBSYSTEM`.
2. **Vorlage.** `apps/template` als Vorbild für ein Cargo-Projekt, samt `.cargo/config.toml` für Linux.
3. **Abnahmetest.** `tools/crates_accept.py`: packen, als lokale Quelle einbinden, neues Projekt anlegen, für
   Windows, Linux und Web bauen, `.uis` laufen lassen. Das läuft in der vollen Testsuite, nicht in `--quick`.
4. **Doku für Agenten.** `README.md` der Fassade mit Schnellstart. `docs/GUIDE.md` und `docs/API.md` kommen beim
   Packen in die Fassade, damit ein Agent sie unter `~/.cargo/registry/src` findet. Zusammen 2,7 MB, das passt.
5. **Offene Fälle klären.** Build auf einem echten Linux-Rechner (`cc`, `_start`, `jfgui_glibc` ohne Konfiguration)
   und auf einem Rechner ohne Visual Studio (`rustc` übergibt `kernel32.lib`).

**Geprüft:** `tools/crates_accept.py` läuft in der vollen Suite und ist grün: 57 Pakete mit 7,4 MB; Windows Dev
(wie ohne Windows SDK) und Release mit Smoke-Test; Linux gegen glibc mit Smoke-Test in WSL; ein Konsolenprogramm
mit `PERMISSIONS`, `PACKAGE` und `assets/` unter Windows und als statische Linux-Datei; im Browser zählt ein Klick
(Chromium, Playwright). Von null an etwa 14 Minuten. `build_rs.py test --quick` lief mit 1329 von 1331 Skripten durch (nur `fonttest/selftest`, CRLF), die Release-Programme unter Windows sind trotz `raw-dylib` byte-gleich groß.

Die beiden offenen Fälle:

- **Ein echter Linux-Rechner** (Fedora 44, gcc 16): Die Vorlage baut mit `rust-lld` und mit `cc`, das Programm mit
  Fenster braucht nur `GLIBC_2.2.5`, der Smoke-Test läuft. Zwei Dinge hat erst dieser Rechner gezeigt. Erstens gilt
  `linker` aus `.cargo/config.toml` auch für die Build-Skripte, wenn Ziel und Host gleich sind, auch mit
  `--target`, und `rust-lld` findet die C-Bibliotheken nicht. Deshalb steht der Linker in den `rustflags`, die mit
  `--target` nur das Programm erreichen. Zweitens scheitern ohne `--target` die Proc-Macros am Relocation-Modell.
  Unter Linux heißt es also immer `--target x86_64-unknown-linux-gnu`, das README sagt es.
- **Ohne Visual Studio:** Drei Stellen in `plat` linkten `kernel32` noch über eine Importbibliothek, sie nutzen jetzt
  `raw-dylib`. Damit braucht das Programm selbst keine Importbibliothek mehr. Nachgestellt mit
  `/lldignoreenv /winsysroot:<leerer Ordner>`, denn `/lldignoreenv` allein hält `rust-lld` nicht davon ab, Visual
  Studio selbst zu finden. Mit einem gewöhnlichen `#[link]` scheitert derselbe Build an `kernel32.lib`. Die
  Build-Skripte und Proc-Macros brauchen mit Rusts msvc-Toolchain aber weiter die Visual Studio Build Tools, wie
  jedes Rust-Projekt mit Build-Skripten. Eine GNU-Toolchain als Host ist nicht getestet.

Nicht in CR2: Android über Cargo, die Seite um das Wasm-Modul (`web/` des Repos), Web mit Threads (braucht das
eigene Sysroot) und je Programm eine eigene `PERMISSIONS.<bin>`. Das sind Aufgaben für `cargo jflib` in CR4.

### CR3: `build_rs.py` kompiliert über Cargo

Groß. Erst danach gibt es nur noch ein Bausystem. In Teilschritten, nach jedem bleibt die Suite grün.

- **CR3a: App-Manifeste. Erledigt am 2026-11-05.** Die 57 Apps, ihre 8 weiteren Programme (`BINS`), die 11
  App-eigenen Crates und die Stile `jfoffice`, `backupcraft` und `netlab` sind Pakete des Workspace mit
  `publish = false`, 79 neue Manifeste. Ein Programm aus `BINS` ist ein Paket in seinem Ordner, so hat es sein eigenes
  Subsystem und, wo es eine gibt, seine eigene `PERMISSIONS.<name>`. Jedes Programm hat ein `build.rs` mit
  `jflib-build`: die App `jflib_build::app()`, ein Programm aus `BINS` zum Beispiel
  `Build::console().part_of("BackupCraft", "..")`. Die Version kommt aus `PACKAGE` (ohne sie `0.0.0`), `style` aus
  `STYLE`. `cargo check` aller Apps läuft für Windows, Linux statisch, Linux mit glibc, Web und Android ohne Warnung.
  `build_rs.py` baut weiter mit `rustc`; `check` vergleicht die Manifeste mit den Quellen, `new` schreibt sie
  (`tools/app_manifests.py`). `themedemo` startet jetzt mit `style_winui::STYLE`: Cargo erlaubt kein Paket unter zwei
  Namen in einer Crate, dort wären es `style` und `style_winui` gewesen.
- **CR3b: Windows über Cargo. Erledigt am 2026-11-06.** Ein `cargo build` pro Ziel und Lauf für alle Apps samt
  `BINS` nach `<out>/cargo`; `build_rs.py` kopiert die Programme aus den JSON-Meldungen an ihre gewohnten Stellen,
  `check_intrinsics` prüft die neu übersetzten rlibs. Das Dev-Profil der Wurzel ist das von `build_rs.py`
  (16 Codegen-Units, `opt-level = 2` für die Rechen-Crates und die von crates.io, `strip`). `-D warnings` und
  `--remap-path-prefix` kommen über die `rustflags` des Werkzeugs. `--rustc` baut wie vorher, zum Vergleichen.
- **CR3c: Linux über Cargo. Erledigt am 2026-11-06.** Statisch nach `<out>/cargo`, gegen glibc nach
  `<out>/cargo-glibc` (die Flags unterscheiden sich); ein `cli`-Programm aus `BINS` bleibt statisch. Die
  Linker-Map schreibt jetzt `jflib-build` als `program.map` in sein `OUT_DIR`.
- **CR3d: Web, Android, Module über Cargo. Erledigt am 2026-11-06.** Web und Android laden eine App als Bibliothek:
  `build_rs.py` erzeugt dafür einen eigenen Workspace `<out>/cargo-<ziel>-ws` mit einem Paket pro App (`[lib]` auf
  ihrer `main.rs`, ihre Abhängigkeiten, ein `build.rs` mit `part_of`), die Apps bleiben Programme. Dort sind die
  Crates der Bibliothek keine Mitglieder, deshalb nennt er die Profile der Wurzel einzeln. Web baut scalar, SIMD und
  mit Threads (eigenes Sysroot), Android ein `staticlib`, das `tools/android.py` wie bisher mit der C-Brücke linkt.
  `jxlenc` baut mit `--cfg jflib_module` und Fat-LTO wie bisher und lädt in PaintCraft.

  Größen gegen `rustc`: Release unter Windows `template` +0,03 %, `jfs` +0,06 %, `PaintCraft` +0,01 %, `jxlenc`
  gleich; unter Linux `template` +0,01 %, `jfs` +0,11 %. Dev: Web +0,16 %, Windows +0,21 %, Linux −1,06 %, die
  Android-Bibliothek +0,28 %. `test --quick` lief nach jedem Schritt durch bis auf `fonttest/selftest` (CRLF) und
  einzelne Bildschirmproben von `uigputest` unter Windows, deren Farben knapp danebenliegen (allein grün).
- **CR3e: Varianten und Aufräumen. Erledigt am 2026-11-06.** Der direkte `rustc`-Weg ist weg (`build_rs.py` von etwa
  2700 auf 2200 Zeilen), `ide` auch, rust-analyzer liest den Workspace selbst.
  - `--style` baut ein Paket je Variante im Workspace des Werkzeugs, wie Web und Android. Das Programm darin heißt nach
    dem Paket, denn Cargo legt alle Programme eines Laufs in einen Ordner, zwei gleichnamige überschreiben sich dort.
  - `--profile` und die Varianten von `tools/measure.py` sind dasselbe Profil-Format (opt-level für Bibliothek,
    Rechen-Crates und Apps, LTO, Codegen-Units, weitere `rustflags`), das als `--config` auf das Release-Profil geht.
  - Die Rust-Plugins von PaintCraft sind Pakete (`lib/plugin` als `jflib-plugin`, `plugins/NAME` als
    `jflib-plugin-NAME`), `PluginBuilder` baut sie über Cargo mit ihren eigenen Einstellungen.
  - `tools/memory_safety_check.py` holt seine rlibs aus Cargo (`Builder.cargo_rlibs`), die Proben selbst bleiben
    `rustc`-Aufrufe; `tools/android.py` linkt nur noch das Archiv aus Cargo.
  - Die Prüfung auf einen Inkrementell-Cache, den ein beim ThinLTO abgebrochener `rustc` vergiftet hat, gilt jetzt
    Cargos Ordner: Cargo hat dasselbe Problem und prüft es nicht.
  - `STYLE`, `TARGETS` und `BINS` bleiben vorerst eigene Dateien.

  **Geprüft:** die volle Suite mit 1329 von 1331 Skripten (nur `fonttest/selftest`, CRLF) samt den Rust-Plugins und
  `crates_accept.py`; die Proben von `memory_safety_check.py`; ein Variantenprofil wie aus `measure.py` (dieses
  selbst braucht `legacy/` neben dem Repo und lief nicht). Auf den Zielen selbst: alle sieben Web-Tests von `androidGallery` in
  Chromium (`system_web` auch mit Threads), `text_web` auch in Firefox; drei Android-Tests auf dem Emulator (API 35,
  x86_64; kein Telefon angeschlossen); `build_rs.py` selbst auf dem Fedora-Rechner: `check`, `template` und `jfs`
  grün, `gallery` 39 von 42 (die drei anderen brauchen ein Display, es war niemand angemeldet). Erst dort fiel auf,
  dass die Kopie aus Cargos Ordner das Ausführungsrecht verlor.

Bleiben müssen `lint()`, `check_intrinsics`, `plugin_verify` und `perm_lint`.

**Erledigt am 2026-11-06.** Die volle Suite ist grün bis auf `fonttest/selftest` (CRLF), die Release-Größen weichen
höchstens 0,11 % ab, Web, Android (Emulator) und ein echter Linux-Rechner sind durchgelaufen. Offen: das Telefon,
und die GPU-Skripte von `gallery` auf Fedora mit angemeldetem Nutzer.

### CR4: `cargo jflib`

Groß. Eine Binary `cargo-jflib` in Rust wird automatisch zu `cargo jflib`. Sie braucht nur `std` und Cargo; den
kleinen JSON-Leser für Cargos Meldungen hat sie selbst. Vorlage und Web-Host trägt sie als Kopien in ihrem Paket,
`build_rs.py check` hält sie gleich.

- **CR4a: `new`, `test`, `web`. Erledigt am 2026-11-06.** `new NAME` schreibt die Vorlage mit dem Namen der App.
  `test` baut und lässt jedes `tests/*.uis` mit `--script` laufen. `web` baut die Bibliothek der App für scalar,
  SIMD und mit Threads (das Sysroot mit Atomics baut es selbst aus `rust-src`, ohne fällt der Modus mit Hinweis weg),
  legt den Host aus `web/` und die Lizenzen aller Pakete daneben; `--serve` liefert das mit den Headern aus, die
  Threads brauchen. **Geprüft** in `crates_accept.py`: aus dem gepackten Paket installiert (`cargo install --path`, denn `cargo install` aus einer Verzeichnisquelle findet das Paket und holt es dann nicht), `new`, `test` grün, `web` mit allen drei Modulen und einem Klick im Browser.
- **CR4b: `android`. Erledigt am 2026-11-06.** Die Bibliothek der App als `staticlib` über `cargo rustc`, die
  C-Brücke aus `android/` mit dem `clang` des NDK dazu gelinkt (16-KiB-Seiten, RELRO, geprüft wie in
  `tools/android.py`), Java mit `javac --release 8` und `d8`, `aapt2`, die Dateien ohne Zip-Bibliothek an die APK
  gehängt, `zipalign -P 16`, `apksigner` mit einem Debug-Schlüssel, den es selbst anlegt, oder dem aus
  `jfgui_ANDROID_KEYSTORE`; `--install` über `adb`. Das Manifest kommt aus `PACKAGE` und `PERMISSIONS`, wenn die
  App kein eigenes `android/AndroidManifest.xml` hat. `android/*` liegt als Kopie im Paket, `build_rs.py check`
  hält sie gleich. **Geprüft:** `hallo2` aus `cargo jflib new` auf dem Emulator (API 35, x86_64) installiert,
  gestartet, ein Klick zählt; `crates_accept.py` baut die APK aus den gepackten Crates und prüft Code, Bibliothek
  (unkomprimiert an einer 16-KiB-Grenze) und Lizenzen, ohne SDK mit Hinweis übersprungen. Nicht geprüft:
  `arm64-v8a` auf dem Telefon und `--release`. Dabei aufgefallen: `cargo package` setzt die Zeit jeder Datei auf
  2006, und Cargo erkennt ein Crate aus einer Verzeichnisquelle nur an der Version. Ein zweiter Lauf von
  `crates_accept.py` behielt deshalb, was der erste gebaut hatte (hier ein `cargo-jflib` ohne `android`); jetzt
  löscht er die Fingerprints der Pakete, die sich seit dem letzten Lauf geändert haben.
- **CR4c: `package`**, in drei Teilen, weil `tools/package.py` mit seinen Helfern rund 2.000 Zeilen hat:
  - **CR4c1: Setup und `.deb`, die Standardformate von `package.py`. Erledigt am 2026-11-06.** `cargo jflib
    package [--styles winui,kde] [--formats setup,deb]` baut die App je Stil mit `--release` und `JFLIB_STYLE` für
    Windows und Linux, macht Vorschaubilder mit `--shot` (Windows-Programm unter Windows, Linux-Programm unter
    Linux) und schreibt `target/jflib-package/NAME_Setup.exe` und `DEB_VERSION_amd64.deb`. Der Setup-Stub ist jetzt
    die Crate `jflib-installer` (`lib/installer`); `apps/installer` bindet sie nur noch ein. Das Werkzeug baut ihn in
    einem eigenen kleinen Projekt unter `target/` mit den Profilen und der `Cargo.lock` der App, sodass die schon
    gebauten Crates wieder genommen werden. Das `.deb` schreibt es selbst (ar, ustar, gzip), ohne `dpkg-deb` und
    ohne WSL; dafür hat `cargo-jflib` ein eigenes Deflate (0,16 % größer als zlib -9 an einem echten Programm), PNG
    lesen und schreiben und das Verkleinern von `pngutil.py`. Aus `PACKAGE` liest es die Schlüssel von
    `package.py` (ohne `cross_bins`, `unit_*` und Module). Dabei aufgefallen und behoben: `web` und `android`
    lieferten die Schrift- und Symbol-Lizenzen der Stile (`assets/OFL-*.txt`, `DejaVu-LICENSE.txt`,
    `LICENSE-FluentSystemIcons.txt`) nicht mit. **Geprüft** in `crates_accept.py`: `hallo2` mit `--styles
    winui,kde`; das Setup hat beide Vorschauen und die Schriftlizenz, installiert per Skript den kde-Stil (genau
    dessen Programm) und `uninstall.exe` entfernt ihn; `dpkg-deb` liest das `.deb` (Steuerdateien, Skripte
    ausführbar, `root/root`, Programme 0755), das ausgepackte kde-Programm besteht `tests/smoke.uis`. **Nicht
    geprüft:** ein echtes `dpkg -i` (braucht root; WSL hat keins) und die debconf-Frage. Im zweiten Lauf
    aufgefallen: Das Stub-Projekt behielt seine `Cargo.lock`, und als sich `jflib-installer` bei gleicher Version
    änderte, brach Cargo mit einer anderen Prüfsumme ab; jetzt übernimmt der Stub bei jedem Lauf die der App.
  - **CR4c2: AppImage, rpm, Flatpak, MSIX** (`pkg_linux.py`, `pkg_msix.py`); `apps/appimagert` muss dafür wie der
    Setup-Stub als Crate mitkommen.
  - **CR4c3: Signieren, apt-Repository, Update-Kanal** (`codesign.py`, `update_manifest.py`).
- **CR4d: `watch`, `plugin`.**
- **Reihenfolge danach:** CR4c2, CR4c3, `watch`, `plugin`.
- **Umfang:** rund 2.350 Zeilen Python aus `tools/package.py`, `pkg_linux.py`, `pkg_msix.py`, `pkgmeta.py`,
  `codesign.py` und `android.py`, dazu der Testlauf aus `build_rs.py`.
- **Mitzuliefern:** `web/*`, `android/*` (erledigt) sowie die Starter `apps/installer` (erledigt: `jflib-installer`)
  und `apps/appimagert`.
- **Bleibt nötig wie heute:** MakeAppx, signtool, `flatpak-builder` und das Android SDK; `dpkg-deb` nicht mehr.

### CR5: API-Vertrag und Doku

Klein bis mittel, vor dem Crate-Release.

- `docs/VERSIONING.md`: Was gilt als öffentliche API, wenn 57 Crates sichtbar sind? Vorschlag: nur der Weg über
  `jflib` ist versprochen.
- `api-diff` auf die Fassade ausrichten, `tools/gendoc.py` um die neuen Crates ergänzen.
- `docs/GUIDE.md` und `README.md` auf Cargo umstellen. Darin auch festhalten, dass eine fremde Crate mit eigenen
  Systemaufrufen `plat` umgeht und nur die `sandbox`-Zeile dagegen hilft.

### CR6: Auszug und Veröffentlichung

Braucht eine eigene Zusage. Namen und Versionen lassen sich danach nur noch zurückziehen (yank), nicht löschen.

1. Die echten Apps und JFScript in eigene Repos, die Test-Apps bleiben hier.
2. `cargo publish --workspace --dry-run`, dann in Abhängigkeitsreihenfolge hochladen. Vorher die aktuellen Limits von
   crates.io für neue Crates prüfen, bei 57 Stück kann das dauern.
3. Prüfen, ob die Dokumentation auf docs.rs baut und eine Einstiegsseite zeigt.

Die Namen `jflib`, `jflib-rt` und `jflib-ui` waren am 2026-11-04 noch frei. Reserviert ist damit nichts.

## Reihenfolge

```text
CR0 -> CR1 -> CR2   lokal kompatibel, das Ziel für jetzt
              CR3   nur noch ein Bausystem
              CR4, CR5   vor dem Crate-Release, unabhängig voneinander
              CR6   mit eigener Zusage
```

Die Punkte von CR0 lassen sich parallel bearbeiten. CR1 und CR3 berühren `build_rs.py` an vielen Stellen und sollten
nicht neben anderer Arbeit an dieser Datei laufen.

## Risiken

| Risiko | Stand | Umgang |
| --- | --- | --- |
| Laufzeitprüfungen ändern das Verhalten | Cargos Dev-Profil schaltet `overflow-checks` und `debug-assertions` ein, `build_rs.py` nicht. Mit ihnen bleibt `template` in Web mit Threads leer, die Android-`.so` wird 23 % größer | beide in den Profilen abschalten; **zusätzlich klären, welche Prüfung auslöst, das kann ein echter Fehler in der Bibliothek sein** |
| Zwei Bausysteme von CR1 bis CR3 | Tests laufen über `rustc`, Nutzer bauen über Cargo | `crates_accept.py`, CR3 nicht lange aufschieben |
| Pfade im Programm | Ein Cargo-Release enthält `C:\Users\…` aus den Panic-Orten, die Vorlage 238-mal. `--remap-path-prefix` entfernt sie im Test vollständig, `trim-paths` ist nicht stabil | `cargo jflib` setzt den Schalter; das README der Fassade nennt ihn |
| Module | gebaut und mit denselben Exporten, aber nicht geladen; die Kopien in `modules/modrt/shared` können auseinanderlaufen | in CR3 mit PaintCraft testen, Prüfung aus CR0 |
| `img_jxl_enc` für Wasm | braucht allein 1468 s bei `opt-level` 1, bei 2 und im Release über 900 s (dort abgebrochen), bei 0 samt Abhängigkeiten 397 s. Ohne ihn dauert der erste Web-Build 88 s | auf Web aus der Fassade nehmen; die Ursache ist nicht geklärt |
| `panic = "abort"` fehlt | ein nacktes `cargo new` baut nicht, nur mit eigener Meldung gelöst | `cargo jflib new` schreibt es, `README.md` nennt es zuerst |
| Offene Worktrees | Konflikte vor allem bei `lib/script`, den Assets und `build_rs.py` | CR0 und CR1 getrennt committen, damit sich Worktrees schrittweise anpassen können |
