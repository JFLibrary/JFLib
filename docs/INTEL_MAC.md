# Intel-Mac-Plan: JFLib auf Macs mit Intel-Prozessor

Stand 2026-11-14, auf Basis von `main` 85cd8d06. Die fünf Entscheidungen unten hat der Nutzer am 2026-11-14 alle wie
empfohlen getroffen. IM0 bis IM3 sind umgesetzt und unter Rosetta 2 geprüft, IM5 ist am Code geprüft, die Doku aus
IM6 ist nachgezogen (je ein Abschnitt „Umgesetzt“ unten). Offen: der Lauf des Jobs `intel` (IM4, braucht den Branch
auf GitHub) und die Website (IM6, Inhalt erst abstimmen).

## Ziel

Die Programme der Bibliothek laufen auch auf Macs mit Intel-Prozessor (`x86_64-apple-darwin`), aus derselben `.app` und
derselben `.dmg` wie auf Apple Silicon (ein Universal-Programm, Entscheidung 1a). Gebaut wird weiter auf dem Mac mini
(M2), der das Intel-Programm mitbaut. Geprüft wird unter Rosetta 2 auf dem Mac mini und auf GitHubs Intel-Runner.
Eine macOS-VM ist kein Weg: macOS darf laut Apples Lizenz nur auf Apple-Hardware laufen, also nicht auf dem PC, und
der Mac mini virtualisiert nur ARM-Versionen von macOS.

Nicht Teil dieses Plans: iOS (jedes iPhone und iPad ist arm64, auch der Simulator auf dem Mac mini), macOS älter als
13, native Plugins auf dem Mac (wie auf Apple Silicon: keine, `web_lib.rs`).

## Warum jetzt und wie lange

Apple hat mit macOS 26 die letzte Version für Intel-Macs gebracht; sie bekommen noch einige Jahre
Sicherheitsupdates, und viele sind noch im Einsatz. Die Prüfmöglichkeiten laufen aber aus: GitHub hat das Image
`macos-15-intel` als letztes x86_64-Image für macOS angekündigt, mit Ende etwa im Herbst 2027, und Apple will Rosetta
nach macOS 27 einschränken. Rust baut `x86_64-apple-darwin` seit 1.90 nur noch als Tier 2. Der Plan ist deshalb
schlank: kein neuer Code, wo der x86-64-Code von Windows und Linux schon passt, und eine klare Regel für die Zeit
danach (Entscheidung 4a).

## Ausgangslage

Gefunden in `main` 85cd8d06; der Apple-Code nimmt an vielen Stellen arm64 an:

- **Bau:** `build_rs.py` kennt für `mac` nur `aarch64-apple-darwin` (`TRIPLE`); `--arch` gilt nur für `win` und
  `linux` (`ARM64_TRIPLE`, Ordner `win-arm64`/`linux-arm64`). cargo-jflib baut fest für `aarch64-apple-darwin`
  (`lib/cargo_jflib/src/mac.rs`, `TRIPLE`), `tools/apple.py` arbeitet mit `build/mac/`.
- **rt:** Auf Apple nimmt `rt` die portable CPU-Erkennung (`portable/cpu.rs`: nie AVX2) und die portable Mathe
  (`portable/math.rs`) statt `x87.rs`, unabhängig von der Architektur; Speicher kommt vom System (`apple.rs`).
  `fx.rs` (SSE-Intrinsics) würde auf x86-64 schon passen.
- **plat:**
  - `apple.rs` deklariert libSystem mit den Records von arm64. Auf x86-64 heißen die Funktionen mit 64-Bit-Inodes
    anders (`stat$INODE64`, `fstat$INODE64`, `lstat$INODE64`, `fstatat$INODE64`, `readdir$INODE64`,
    `opendir$INODE64` …); ohne diese Namen kommen still die alten Layouts zurück.
  - `cpu.rs` legt für Apple „kein AVX2, Stapel immer ausgerichtet“ fest; auf x86-64 kollidiert das mit der x86-Fassung
    von `stack_aligned` (zwei Definitionen).
  - `apple_crash.rs` liest den Signal-Kontext von arm64 (x0 … x28, fp, lr, sp, pc). `crash_test` für x86-64 gibt es
    schon (`plat/lib.rs`, für Windows und Linux).
  - `apple_objc.rs`: `msg!` ruft immer `objc_msgSend`. Auf x86-64 kommen Strukturen über 16 Byte über
    `objc_msgSend_stret` zurück (und `objc_msgSendSuper_stret`): `NSRect` (32 Byte) an 33 Stellen, `MTLSize`
    (24 Byte) an einer. `NSPoint`, `NSSize`, `NSRange` (16 Byte) gehen in Registern zurück wie bisher.
- **SIMD:** `lib/simd` hat für x86-64 die Wege von Windows und Linux (SSE2, AVX2 nach Erkennung); auf Apple meldet die
  Erkennung heute nie AVX2.
- **Metal:** `mtl_dev.rs` kennt die Familie Mac2 und liest `hasUnifiedMemory`, `isLowPower`, `isRemovable`; Stufe Voll
  ist an Metal 3 mit Apple7 oder Mac2 gebunden. Puffer: Device privat, Upload/Readback geteilt, Texturen über Blit.
  Das passt auch für eine eigene AMD-Grafikkarte (geteilter Speicher liegt dann im Hauptspeicher, über PCIe).
- **Updates:** `update::target()` meldet auf x86-64 schon `macos-x86_64`; `tools/package.py` und cargo-jflib
  (`update.rs`) schreiben nur `macos-aarch64` ins Manifest.
- **CI:** `.github/workflows/apple.yml` läuft von Hand auf `macos-15` (arm64).
- **Mac mini (geprüft am 2026-11-14):** macOS 27.2, kein Rosetta, Rust nur mit den arm64-Zielen.

## Stufen

### IM0: Vorbereitung (S)

- Rosetta 2 auf dem Mac mini (`softwareupdate --install-rosetta`: ein Download von Apple mit Lizenzzustimmung, also
  mit dem Ja des Nutzers oder von ihm selbst) und `rustup target add x86_64-apple-darwin` (Download, ebenso).
- Probe: ein Hallo für x86-64 läuft unter Rosetta; was Rosetta per CPUID meldet (SSE4.1, AVX2, FMA, BMI2) und ob x87
  unter Rosetta exakt rechnet (die Testvektoren von `rt::x87` gegen Windows). Das entscheidet, wie viel die Rosetta-Läufe
  in IM1 bis IM3 über echtes Intel aussagen.

**Umgesetzt (2026-11-14):** Rosetta 2 und das Rust-Ziel `x86_64-apple-darwin` sind auf dem Mac mini installiert (mit dem
Ja des Nutzers). Ein Rust-Programm für x86-64 läuft dort unter Rosetta (`sysctl.proc_translated` = 1).

- **CPUID unter Rosetta (macOS 27.2):** Hersteller `GenuineIntel`, Name `VirtualApple @ 2.50GHz`. Gemeldet werden SSE
  bis SSE4.2, POPCNT, AES und PCLMULQDQ. AVX, AVX2, FMA, BMI1/2, F16C, LZCNT und OSXSAVE fehlen. Mit der
  Umgebungsvariablen `ROSETTA_ADVERTISE_AVX=1` meldet Rosetta zusätzlich AVX, AVX2, FMA, BMI1, BMI2, F16C, LZCNT und
  XCR0 = 7, und die Befehle laufen. Damit lassen sich unter Rosetta beide SIMD-Wege prüfen: SSE ohne die Variable,
  AVX2 mit ihr. `plat::cpu_brand` liest `machdep.cpu.brand_string` und meldet auch unter Rosetta „Apple M2“.
- **x87 unter Rosetta:** Die Mantisse hat 64 Bit. Das Kontrollwort steht beim Start auf 0x037F, unter Windows auf
  0x027F; `rt::x87` setzt es in jedem Aufruf selbst. Eine Probe mit 20 000 Eingaben je Befehl gegen einen Ryzen 5 3600
  unter Windows:
  - **bitgleich:** `fadd`, `fmul`, `fdiv`, `fsqrt`, `fscale`, `frndint`, `fprem`;
  - **abweichend:** `fsin`, `fcos`, `fpatan`, `fyl2x`, `f2xm1`, `fptan` und `fyl2xp1` in 1 bis 17 % der Fälle um eine
    Einheit der letzten Stelle (`fptan` um zwei). Auf `double` gerundet bleiben davon 3 von 140 000.
- **Die Testvektoren der Bibliothek:** `portabletest --math-dump` liefert 360 000 Ergebnisse, verglichen gegen Windows
  auf demselben Ryzen mit dem neuen [`x87_diff.py`](../apps/portabletest/tests/x87_diff.py).
  - **bitgleich:** die Dezimal-Umwandlungen (`scale`, `log10_round`, `scale_round`), `exp2`, `log2`, `sin` und
    `atan2`;
  - **abweichend:** 18 Ergebnisse um 1 ulp, in `pow` (5), `pow_gamma` (2), `cos` und `sincos_cos` (je 3),
    `sin_wide` und `cos_wide` (je 1) und `rz_exp2` (3).
  - **Digest:** Der von `portabletest` ist unter Windows x86-64, unter Rosetta (SSE, AVX2, `--simd sse2`) und auf
    Apple Silicon derselbe, `af6940ef15c56d1f`.
- **Folgerung:** Für die Logik, die SIMD-Wege und die Dezimal-Umwandlungen sagen die Läufe unter Rosetta dasselbe
  wie echtes x86-64. Bei den transzendenten x87-Befehlen weichen schon AMD und Intel voneinander ab; hier bleibt der
  Intel-Runner maßgeblich (IM4).

### IM1: Bau, rt und plat ohne Fenster (M)

- `build_rs.py --target mac --arch x86_64` baut nach `build/mac-x86_64/` (wie `win-arm64`); ohne `--arch` bleibt `mac`
  arm64. `tools/apple.py` und cargo-jflib nehmen das zweite Ziel an.
- `rt` wie unter Windows und Linux (Entscheidung 2a): die x86-Erkennung (`rt/cpu.rs`, CPUID) und `x87.rs` auch auf
  Intel-Macs. `plat/cpu.rs` bekommt die passenden `cfg`s.
- `plat/apple.rs`: die `$INODE64`-Namen per `link_name` auf x86-64, die Records (`Stat`, `Dirent`, `Pollfd`,
  `PthreadAttr`, dazu `SigAction` in `apple_crash.rs`) gegen die SDK-Header von x86-64 geprüft.
- `msg!` wählt `objc_msgSend_stret`, wenn der Rückgabetyp auf x86-64 größer als 16 Byte ist; dasselbe für die
  Super-Aufrufe. Eine Stelle statt 34.
- `apple_crash.rs`: der Kontext von x86-64 (`__rip`, `__rsp`, `__rbp`, die Kette über `rbp`; Apples x86-64-ABI behält
  den Rahmenzeiger auch).
- Prüfung unter Rosetta: `coretest`, `plattest` ohne Fenster (`all`, `cliprich`, `spell`), der Crash-Test.

**Umgesetzt (2026-11-14):**

- **Bau:** `build_rs.py ... --target mac --arch x86_64` baut nach `build/mac-x86_64/`, der Cargo-Ordner ist
  `cargo-mac-x86_64`. `--arch` hat jetzt je Ziel eine eigene Vorgabe: `mac` und `ios` ARM64, die übrigen x86-64. Ohne
  `--arch` bleibt also alles wie bisher. `--arch x86_64` mit `ios` lehnt das Skript ab. `run` und `test` starten den
  Intel-Teil auf einem Mac mit Apple Silicon unter Rosetta.
- **`rt`:** `cpu.rs` (CPUID) und `x87.rs` gelten jetzt auch für `x86_64-apple-darwin` (Entscheidung 2a); `fx.rs` galt
  schon. Rust baut dieses Ziel für den Prozessor `penryn`, also mit SSE4.1 als Grundlage.
- **`plat`:**
  - `cpu.rs`: kein AVX2 und „Stapel immer ausgerichtet“ gelten nur noch für Apple mit ARM64.
  - `apple.rs` ruft auf x86-64 `fstat`, `stat`, `lstat`, `opendir` und `readdir` mit der Endung `$INODE64` auf.
    Außerhalb von `apple.rs` fanden sich zwei weitere Stellen: `statfs$INODE64` in `fs.rs` und
    `realpath$DARWIN_EXTSN` in `watch.rs` (nur diese Fassung legt die Antwort selbst an).
  - Die Records sind auf x86-64 dieselben wie auf arm64: `stat` mit 144 Byte, `dirent`, `pollfd`, `pthread_attr_t`
    mit 64 Byte, `sigaction` und `uc_mcontext` bei 48.
- **`msg!`:** `msg_entry` wählt auf x86-64 `objc_msgSend_stret`, wenn das Ergebnis größer als 16 Byte ist. Das neue
  `msg_super!` macht dasselbe mit `objc_msgSendSuper_stret`; der einzige Super-Aufruf (`sendEvent:` des Panels) nutzt
  es. Ein BOOL wird als Byte gelesen, weil er auf x86-64 ein `signed char` mit beliebigem Wert ist. `gpu/mtl_objc.rs`
  hat dieselbe Auswahl (`MTLSize`).
- **`apple_crash.rs`:** liest den Kontext von x86-64: Fehlercode, `faultvaddr`, rax bis r15, rip. Als erste
  Rücksprungadresse dient das Wort an rsp, wenn es hinter einem `call` ins Programm zeigt (ein Blatt ohne eigenen
  Rahmen, wie `crash_test`). Danach folgt die Kette über rbp.
- **Geprüft unter Rosetta (Mac mini):**
  - `coretest` (`all.uis`) und `plattest --test all` (220 von 220);
  - `cpu`, ohne und mit `ROSETTA_ADVERTISE_AVX=1`, und `crash` (7 von 7: Schreiben auf 0x10, rip, rax, Code-Bereich,
    Rücksprung in den Aufrufer);
  - `cliprich` 27, `spell` 57, `panic`, `spawn`, `launch`, `vfs` 132, `dirs` 33, `instance`, `log` 31, `policy` 129,
    `print`, `trash` 19, `power` 10;
  - `recent` 43 von 43, wenn `plattest` in seinem Bündel läuft. Ohne Bündel schwankt die Liste von
    `NSDocumentController` auch im arm64-Build (ein bis zwei Prüfungen).
  - Der arm64-Build mit denselben Änderungen ist ebenso grün: `plattest all` 220, `crash` 7, `coretest`,
    `portabletest` mit demselben Digest.

### IM2: Fenster und Dienste (S–M)

- Unter Rosetta über `tools/apple.py gui`: `plattest --test window` (mit Menüleiste), `--test a11y-mac`, die Skripte
  von `gallery`. Höchstens drei Runden pro Variante, keine echte Eingabe.
- Erwartet sind vor allem Fehler der Art aus IM1 (Rückgaben von `NSRect`, Records); AppKit selbst ist dasselbe.

**Umgesetzt (2026-11-14):** Unter Rosetta in echten Fenstern, über `tools/apple.py gui`, je eine Runde ohne echte
Eingabe:

- `plattest --test window` 86 von 86 (mit der Menüleiste), `--test a11y-mac` 59 von 59, `--test dnd` 65 von 65;
- `gallery --auto-close 3000` startet und schließt;
- die 43 Skripte von `gallery` sind grün.

Der arm64-Build besteht dieselben Prüfungen, Zeile für Zeile. Der Mac war gesperrt: Drei Fokus-Prüfungen von
`a11y-mac` sind deshalb in beiden Teilen übersprungen. Nach dem Wechsel auf `objc_msgSend_stret` (IM1) blieb kein
Fehler übrig.

### IM3: Universal-Paket und Updates (S–M)

- `mac_bundle` und cargo-jflib bauen beide Ziele und fügen sie mit `lipo` zu einem Programm zusammen; Signatur,
  Notarisierung und `.dmg` wie bisher (sie decken beide Teile ab).
- Die Manifeste nennen dieselbe `.app`/`.dmg` für `macos-aarch64` und `macos-x86_64` (`tools/package.py`,
  `lib/cargo_jflib/src/update.rs`). `updatetest` (auch `t_app`) läuft für beide Teile (`arch -x86_64`).

**Umgesetzt (2026-11-14):**

- **`tools/apple.py`:** `mac_universal` fügt die beiden Programme mit `lipo -create` zu einem zusammen.
  `python tools/apple.py universal build/mac/NAME build/mac-x86_64/NAME` legt es mit seinem `NAME.app` nach
  `build/mac-universal/`.
- **`tools/package.py --formats dmg`:** baut beide Teile (der zweite Lauf von `build_rs.py` mit `--arch x86_64`)
  und fügt das Programm und seine weiteren Programme (`BINS`) je Stil zusammen. Die Datei heißt jetzt
  `<App>-<ver>[-<stil>]-universal.dmg` statt `-arm64.dmg`. Der Update-Kanal nennt dasselbe Bündel und dieselbe `.dmg`
  für `macos-aarch64` und `macos-x86_64`.
- **cargo-jflib:**
  - `cargo jflib mac` baut beide Ziele und fügt sie zusammen; `--arch aarch64|x86_64` baut nur eines.
  - `package --formats dmg` und `update.rs` machen es wie `tools/package.py`.
  - Fehlt das Rust-Ziel, nennt die Fehlermeldung `rustup target add x86_64-apple-darwin`.
  - Die Helfer, die cargo-jflib auf dem Mac selbst startet (der Signierer des Update-Kanals), baut es für dessen
    eigenen Prozessor. Vorher war das immer arm64, was auf einem Intel-Mac nicht startet.
- **Geprüft:**
  - `updatetest` in beiden Teilen 3 von 3 (`app`, `apply`, `update`; der x86-64-Teil unter Rosetta).
  - `tools/package.py BackupCraft --formats dmg --update`: `BackupCraft-0.1.0-universal.dmg` (7,4 MB). Ihr Programm
    ist `x86_64 arm64`, die Signatur ist gültig, der Kanal nennt beide Ziele.
  - Ein Universal-Programm der Gallery besteht ihre Skripte mit `arch -arm64` und mit `arch -x86_64`.
  - `cargo jflib mac -p portabletest` baut ein Bündel mit `x86_64 arm64`. Beide Teile geben den Digest
    `af6940ef15c56d1f` aus. Mit `--arch x86_64` entsteht nur der Intel-Teil.
- **Nicht geprüft:** Signatur mit Developer ID und Notarisierung einer Universal-`.dmg`. Sie laufen wie bisher und
  decken laut Apple beide Teile ab. Den Start einer Universal-`.dmg` auf einem echten Intel-Mac kann nur der Nutzer
  prüfen, wenn er einen hat.

### IM4: CI auf echtem Intel (S)

- Ein zweiter Job in `apple.yml` auf `macos-15-intel`, von Hand gestartet: `coretest`, die Skripte von `gallery` und
  `updatetest`, `plattest` ohne Fenster. Das ist der einzige Lauf auf einem echten Intel-Prozessor (echtes x87, echtes
  AVX2), ohne GPU. Er bleibt, bis GitHub das Image abschaltet (angekündigt für etwa Herbst 2027).

**Umgesetzt (2026-11-14), noch nicht gelaufen:** Der Job `intel` in [`apple.yml`](../.github/workflows/apple.yml) läuft
auf `macos-15-intel` und wird von Hand gestartet, zusammen mit dem Job für Apple Silicon. Er zeigt zuerst Prozessor und
Merkmale (`sysctl`) und führt dann diese Schritte aus, alle mit `--target mac --arch x86_64`:

- `coretest`, die Skripte von `gallery` und `updatetest`;
- `plattest --test all`, `cliprich`, `spell` und `cpu`;
- der Crash-Test, der den Exitcode 3 und keine fehlgeschlagene Prüfung verlangt;
- `portabletest` mit und ohne `--simd sse2`.

Den x87-Dump (`--math-dump`) lädt der Job als Artefakt hoch. Dagegen vergleicht man einen Dump von Windows x86-64 mit
`python apps/portabletest/tests/x87_diff.py A.txt B.txt`; das Skript endet mit Exitcode 1, wenn sich eine
Dezimal-Umwandlung unterscheidet. Damit der Job läuft, muss der Branch auf GitHub liegen; das braucht das Ja des
Nutzers.

### IM5: Metal auf Intel-GPUs (S, ohne Gerät nur am Code)

- Kein neuer Code erwartet. Zu prüfen: die Wahl der GPU auf MacBooks mit zweien (Intel und AMD, `isLowPower`, Wechsel
  im Betrieb), eine eGPU (`isRemovable`, Abziehen), geteilte Puffer bei eigener Grafikkarte, Stufe Voll auf Metal 3 mit
  Mac2 (laut Apple: AMD Vega, Radeon 5000 und 6000, Intel UHD 630, Iris Plus), das MSL unseres Compilers auf den
  Treibern von AMD und Intel.
- Rosetta führt nur den x86-Code aus, die GPU dahinter bleibt die des M2; der Intel-Runner hat keine nutzbare GPU.
  Ohne echten Intel-Mac (Entscheidung 3a) gilt Metal auf Intel-GPUs in der Doku als „gebaut, nicht geprüft“.

**Am Code geprüft (2026-11-14), Metal auf Intel- und AMD-GPUs: gebaut, nicht geprüft.**

- **Wahl der GPU:** Zum Rechnen kommt die eigene GPU zuerst (`adapters`), für Fenster die sparsame
  (`adapters_low_power`, wie unter Windows und Linux). Fenster erzwingen auf MacBooks mit Intel und AMD also keinen
  Wechsel auf die AMD-GPU. Wechselt macOS die GPU, die die Displays ansteuert, bleibt ein Gerät gültig; die
  `CAMetalLayer` kopiert dann über das System.
- **eGPU:** Sie gilt als eigene GPU (`isRemovable`). Wird sie abgezogen, scheitert der nächste Command Buffer, und das
  Gerät meldet `DeviceLost`, wie nach einem Treiber-Reset. Nicht gebaut: das Beobachten von
  `MTLDeviceRemovalRequestedNotification` (`MTLCopyAllDevicesWithObserver`). Ohne das kann macOS beim sicheren
  Trennen melden, dass die App die eGPU noch benutzt.
- **Speicher:** Puffer des Geräts sind privat, Upload und Readback liegen im geteilten Speicher. Bei einer eigenen GPU
  liegt dieser im Hauptspeicher und wird über PCIe gelesen: langsamer, aber korrekt, weil geteilter Speicher nach dem
  Ende eines Command Buffers für die CPU sichtbar ist. Texturen sind immer privat; geteilte Texturen gibt es auf
  Intel- und AMD-GPUs nicht. Speicher ohne Inhalt (`memoryless`) gibt es nur auf Apples GPUs und wird nicht benutzt.
- **Rückgaben:** `maxThreadsPerThreadgroup` (`MTLSize`, 24 Byte) kommt jetzt über `objc_msgSend_stret` (IM1).
- **Subgroups:** Die Limits meldeten fest 32 Lanes. Auf Intel wählt der Treiber pro Pipeline 8 bis 32, auf AMD 32
  oder 64. Auf GPUs ohne Apple-Familie meldet das Backend jetzt 8 bis 64, und `create_compute_pipeline_sized` nimmt
  dort nur 0 an. `simd_ballot` wird über 64 Bit gelesen und passt für 64 Lanes.
- **Stufe Voll:** Sie braucht Metal 3 mit Mac2. Das haben AMD Vega, Radeon 5000 und 6000, Intel UHD 630 und Iris Plus
  (Ice Lake). Ältere Intel-GPUs, etwa die Iris Plus 655 von 2018, bleiben auf Stufe Basis. `primitive_index` ist nur
  mit Apple7 an, auf Mac2 also aus.
- **Der x86-64-Code des Backends:** Unter Rosetta läuft er gegen die GPU des M2. `gputest` besteht so 20 von 21
  Skripten, genau wie der arm64-Build. `validate.uis` scheitert in beiden: Seine Prüfung, dass die Validierung eine
  fehlende Barriere meldet, kann auf Metal nicht greifen: Metal verfolgt solche Konflikte selbst, und
  `validation_messages()` meldet dort immer 0. Diese Arbeit ändert weder `gputest` noch die Validierung; der Fehler
  ist ein offener Punkt außerhalb dieses Plans.
- **Nicht ohne Gerät prüfbar:** das MSL unseres Compilers auf den Treibern von AMD und Intel, SIMD-Funktionen in
  Fragment-Shadern auf Mac2, das Zeitverhalten.

### IM6: Doku und Website (S)

- `APPLE.md` (Ziel), `PACKAGING.md` (Universal-Programm), `ROADMAP.md`, CHANGELOG, README.
- Website (Roadmap, Vergleich, Features: „Macs mit Apple Silicon und Intel“): erst den Inhalt abstimmen, dann bauen.

## Reihenfolge

```text
IM0 -> IM1 -> IM2 -> IM3 -> IM6
              IM4 nach IM1, parallel zu IM2/IM3
              IM5 nach IM2: ohne Gerät (3a) nur am Code und der Vermerk in IM6
```

Aufwand zusammen M. Die Arbeit liegt in `build_rs.py`, `tools/apple.py`, cargo-jflib, `lib/rt` (Auswahl der Module),
`lib/plat` (`apple.rs`, `apple_objc.rs`, `apple_crash.rs`, `cpu.rs`) und `apple.yml`.

## Prüfung

- **Rosetta 2 auf dem Mac mini:** die Suite des Macs für den x86-64-Teil. Die Ausgaben werden mit denen von
  Windows x86-64 verglichen (Entscheidung 2a), soweit IM0 zeigt, dass Rosetta x87 exakt rechnet.
- **GitHubs Intel-Runner:** die Probe auf echter Intel-Hardware, ohne Fenster und GPU.
- **Von Hand:** Der Nutzer startet die Universal-`.dmg` auf dem Mac mini (arm64-Teil) und, falls vorhanden, auf einem
  Intel-Mac.
- **Grenzen:** Rosetta übersetzt nur; es zeigt weder Intels Zeitverhalten noch eine Intel- oder AMD-GPU.

## Fallen und Risiken

- **Falscher Rückgabeweg:** Ein `NSRect` über `objc_msgSend` statt `_stret` stürzt auf x86-64 ab oder liefert Unsinn,
  auf arm64 fällt es nie auf. Ein Test vergleicht Rückgaben von `NSRect` (Fensterrahmen, Bildschirme) in beiden Teilen.
- **`$INODE64`:** Ohne die richtigen Namen stimmen Dateigrößen und Zeiten still nicht.
- **Rosetta ist kein Intel:** Die Emulation unter Windows on ARM hält x87 nur in doppelter Genauigkeit; ob Rosetta
  exakt rechnet und AVX2 meldet, klärt IM0. Maßgeblich bleibt der Intel-Runner.
- **Lebensdauer:** Mit macOS 28 auf dem Mac mini kann Rosetta wegfallen, mit Herbst 2027 der Intel-Runner. Danach ist
  der Intel-Teil nur noch gebaut, nicht mehr geprüft (Entscheidung 4a).
- **Größe:** Das Programm in `.app` und `.dmg` wird etwa doppelt so groß; Schriften und Bilder bleiben einmal drin.

## Entscheidungen

Entschieden (Nutzer, 2026-11-14): alle wie empfohlen, also 1a, 2a, 3a, 4a und 5a.

1. **Paket:** (a, empfohlen) ein Universal-Programm in derselben `.app`/`.dmg` für alle Macs, das Manifest nennt
   dieselbe Datei für beide Ziele; (b) eigene Intel-Pakete (kleiner, aber doppelte Downloads und Kanäle).
2. **Rechnen auf Intel-Macs:** (a, empfohlen) wie Windows und Linux auf x86-64: CPUID, AVX2 und x87 (`rt/cpu.rs`,
   `rt/x87.rs`), Ergebnisse bitgleich mit Windows, getesteter Code, AVX2-Tempo; (b) wie Apple Silicon portabel: eine
   Mathe für alle Macs, aber nur SSE2-Breite und langsamer.
3. **Echte Intel-Hardware:** (a, empfohlen) keine anschaffen: Rosetta und Intel-Runner, Metal auf Intel-GPUs als
   „nicht geprüft“ vermerkt, bis ein Intel-Mac zum Prüfen da ist; (b) ein gebrauchter Intel-Mac (z. B. ein MacBook
   Pro 2019 mit AMD-GPU) als drittes Testgerät für Fenster und Metal.
4. **Nach dem Ende der Prüfwege:** (a, empfohlen) Intel bleibt im Universal-Programm, solange Rust und Xcode das Ziel
   bauen; geprüft wird, solange Rosetta oder der Intel-Runner da ist, danach steht „nur gebaut“ in der Doku; (b) Intel
   mit dem Ende des Intel-Runners wieder herausnehmen.
5. **Mindestversion:** (a, empfohlen) macOS 13 wie auf Apple Silicon (Intel-Macs ab 2017); (b) älter (macOS 11 oder
   12): mehr alte Geräte, aber ohne Testgerät und ohne Runner.
