# Crates-Plan: JFLib für crates.io

Stand 2026-11-05. CR0 ist umgesetzt, CR1 bis CR6 noch nicht. Die Ergebnisse unter „Was schon geprüft ist“ stammen
aus einer Kopie der Quellen außerhalb des Repos, die Größen der Stufen sind geschätzt.

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
| Version | eine für alle Crates, sie steht in der `Cargo.toml` im Wurzelordner |
| Offene Worktrees | bleiben lokal und passen sich später an |

## Zielbild

```text
Cargo.toml                          Workspace: Version, Profile, Lints
Cargo.lock
lib/<crate>/Cargo.toml              48 Crates wie heute
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
hallo/
  Cargo.toml                 jflib, jflib-build, die Profile unten
  build.rs                   jflib_build::app();
  PERMISSIONS, PACKAGE       optional
  assets/                    optional
  src/main.rs                use jflib::{style, ui};  ...  ui::main!(...)
```

Diese Profile braucht jedes Projekt, denn Cargo übernimmt sie nicht von einer Abhängigkeit:

```toml
[profile.dev]
panic = "abort"
opt-level = 1
debug-assertions = false
overflow-checks = false

[profile.release]
panic = "abort"
opt-level = 2
codegen-units = 1
lto = "fat"
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

Mittel. `build_rs.py` kompiliert weiter selbst, liest aber nur noch die Manifeste.

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
   fdlibm zu `rt`, OFL und Fluent-Lizenz zu `style_assets`. Die Dateien müssen im jeweiligen Paket liegen.
7. **Eine Version im Build.** `links = "jflib_rt"` an `rt` (braucht eine leere `build.rs`), damit zwei Versionen
   im selben Programm als klare Meldung enden und nicht als E0152. Noch nicht getestet.

**Fertig, wenn:** `cargo check -p jflib` für Windows, Linux, Linux mit `jfgui_glibc`, Web und Android durchläuft,
`jxlenc` mit `--cfg jflib_module` baut, `cargo package --workspace` mit Verifikation durchläuft und
`build_rs.py test --quick` grün bleibt.

### CR2: Fremde Projekte

Mittel. **Damit ist das Ziel „lokal kompatibel“ erreicht.**

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

**Fertig, wenn:** `crates_accept.py` grün ist und beide offenen Fälle gelöst oder dokumentiert sind.

### CR3: `build_rs.py` kompiliert über Cargo

Groß. Erst danach gibt es nur noch ein Bausystem.

1. **App-Manifeste.** Für die 58 Apps einmal erzeugen, aus `STYLE`, `TARGETS`, `BINS` und `source_crates`. Neue Apps
   bekommen ihres von `build_rs.py new`. Eine einzige Datei pro App, die Agenten sofort verstehen.
2. **Umstellen in dieser Reihenfolge.** Windows, Linux statisch, Linux mit glibc, Web scalar und SIMD, Web mit
   Threads, Android, Module. Die Artefakte kommen aus `--message-format=json`.
3. **Prüfungen behalten.** `lint()`, `check_intrinsics`, `plugin_verify`, `perm_lint`. `-D warnings` wird zu
   `[workspace.lints]`, der Toolchain-Test zu `rust-version`.
4. **Varianten.** `--style` über `JFLIB_STYLE` mit eigenem Zielordner, `--profile` über `[profile.size]`, Web und
   Android über `cargo rustc --lib --crate-type cdylib|staticlib`. Das braucht ein `[lib]` neben `[[bin]]` und gibt
   eine Warnung wegen derselben Datei. Die Alternative ist ein vom Werkzeug erzeugtes zweites Paket.
5. **Aufräumen.** Der direkte `rustc`-Pfad und `ide` entfallen, rust-analyzer liest Cargo selbst.

**Fertig, wenn:** die volle Testsuite grün ist, die Programmgrößen höchstens 1 % abweichen und die echten
Testgeräte (Android, Linux, Web) durchgelaufen sind.

### CR4: `cargo jflib`

Groß, erst kurz vor dem Crate-Release sinnvoll. Eine Binary `cargo-jflib` in Rust wird automatisch zu `cargo jflib`.

- **Reihenfolge:** `new`, `web`, `android`, `test`, `package`, `watch`, `plugin`.
- **Umfang:** rund 2.350 Zeilen Python aus `tools/package.py`, `pkg_linux.py`, `pkg_msix.py`, `pkgmeta.py`,
  `codesign.py` und `android.py`, dazu der Testlauf aus `build_rs.py`.
- **Mitzuliefern:** `web/*`, `android/*` sowie die Starter `apps/installer` und `apps/appimagert`.
- **Bleibt nötig wie heute:** MakeAppx, signtool, `dpkg-deb`, `flatpak-builder` und das Android SDK.

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
| Pfade im Programm | Ein Cargo-Release enthält `C:\Users\…` aus den Panic-Orten. `--remap-path-prefix` entfernt sie im Test vollständig, `trim-paths` ist nicht stabil | `cargo jflib` setzt den Schalter, in der Doku nennen |
| Module | gebaut und mit denselben Exporten, aber nicht geladen; die Kopien in `modules/modrt/shared` können auseinanderlaufen | in CR3 mit PaintCraft testen, Prüfung aus CR0 |
| `img_jxl_enc` für Wasm | braucht allein 1468 s bei `opt-level` 1, bei 2 und im Release über 900 s (dort abgebrochen), bei 0 samt Abhängigkeiten 397 s. Ohne ihn dauert der erste Web-Build 88 s | auf Web aus der Fassade nehmen; die Ursache ist nicht geklärt |
| `panic = "abort"` fehlt | ein nacktes `cargo new` baut nicht, nur mit eigener Meldung gelöst | `cargo jflib new` schreibt es, `README.md` nennt es zuerst |
| Offene Worktrees | Konflikte vor allem bei `lib/script`, den Assets und `build_rs.py` | CR0 und CR1 getrennt committen, damit sich Worktrees schrittweise anpassen können |
