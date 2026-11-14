# Intel-Mac-Plan: JFLib auf Macs mit Intel-Prozessor

Stand 2026-11-14, auf Basis von `main` 85cd8d06. Noch nichts umgesetzt; die Entscheidungen unten sind offen. Die
Aufwände sind geschätzt.

## Ziel

Die Programme der Bibliothek laufen auch auf Macs mit Intel-Prozessor (`x86_64-apple-darwin`), aus derselben `.app` und
derselben `.dmg` wie auf Apple Silicon (ein Universal-Programm, Entscheidung 1). Gebaut wird weiter auf dem Mac mini
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
danach (Entscheidung 4).

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

### IM1: Bau, rt und plat ohne Fenster (M)

- `build_rs.py --target mac --arch x86_64` baut nach `build/mac-x86_64/` (wie `win-arm64`); ohne `--arch` bleibt `mac`
  arm64. `tools/apple.py` und cargo-jflib nehmen das zweite Ziel an.
- `rt` nach Entscheidung 2: die x86-Erkennung (`rt/cpu.rs`, CPUID) und `x87.rs` auch auf Intel-Macs, oder portabel wie
  auf Apple Silicon. `plat/cpu.rs` bekommt die passenden `cfg`s.
- `plat/apple.rs`: die `$INODE64`-Namen per `link_name` auf x86-64, die Records (`Stat`, `Dirent`, `Pollfd`,
  `PthreadAttr`, dazu `SigAction` in `apple_crash.rs`) gegen die SDK-Header von x86-64 geprüft.
- `msg!` wählt `objc_msgSend_stret`, wenn der Rückgabetyp auf x86-64 größer als 16 Byte ist; dasselbe für die
  Super-Aufrufe. Eine Stelle statt 34.
- `apple_crash.rs`: der Kontext von x86-64 (`__rip`, `__rsp`, `__rbp`, die Kette über `rbp`; Apples x86-64-ABI behält
  den Rahmenzeiger auch).
- Prüfung unter Rosetta: `coretest`, `plattest` ohne Fenster (`all`, `cliprich`, `spell`), der Crash-Test.

### IM2: Fenster und Dienste (S–M)

- Unter Rosetta über `tools/apple.py gui`: `plattest --test window` (mit Menüleiste), `--test a11y-mac`, die Skripte
  von `gallery`. Höchstens drei Runden pro Variante, keine echte Eingabe.
- Erwartet sind vor allem Fehler der Art aus IM1 (Rückgaben von `NSRect`, Records); AppKit selbst ist dasselbe.

### IM3: Universal-Paket und Updates (S–M)

- `mac_bundle` und cargo-jflib bauen beide Ziele und fügen sie mit `lipo` zu einem Programm zusammen; Signatur,
  Notarisierung und `.dmg` wie bisher (sie decken beide Teile ab).
- Die Manifeste nennen dieselbe `.app`/`.dmg` für `macos-aarch64` und `macos-x86_64` (`tools/package.py`,
  `lib/cargo_jflib/src/update.rs`). `updatetest` (auch `t_app`) läuft für beide Teile (`arch -x86_64`).

### IM4: CI auf echtem Intel (S)

- Ein zweiter Job in `apple.yml` auf `macos-15-intel`, von Hand gestartet: `coretest`, die Skripte von `gallery` und
  `updatetest`, `plattest` ohne Fenster. Das ist der einzige Lauf auf einem echten Intel-Prozessor (echtes x87, echtes
  AVX2), ohne GPU. Er bleibt, bis GitHub das Image abschaltet (angekündigt für etwa Herbst 2027).

### IM5: Metal auf Intel-GPUs (S, ohne Gerät nur am Code)

- Kein neuer Code erwartet. Zu prüfen: die Wahl der GPU auf MacBooks mit zweien (Intel und AMD, `isLowPower`, Wechsel
  im Betrieb), eine eGPU (`isRemovable`, Abziehen), geteilte Puffer bei eigener Grafikkarte, Stufe Voll auf Metal 3 mit
  Mac2 (laut Apple: AMD Vega, Radeon 5000 und 6000, Intel UHD 630, Iris Plus), das MSL unseres Compilers auf den
  Treibern von AMD und Intel.
- Rosetta führt nur den x86-Code aus, die GPU dahinter bleibt die des M2; der Intel-Runner hat keine nutzbare GPU.
  Ohne echten Intel-Mac (Entscheidung 3) gilt Metal auf Intel-GPUs in der Doku als „gebaut, nicht geprüft“.

### IM6: Doku und Website (S)

- `APPLE.md` (Ziel), `PACKAGING.md` (Universal-Programm), `ROADMAP.md`, CHANGELOG, README.
- Website (Roadmap, Vergleich, Features: „Macs mit Apple Silicon und Intel“): erst den Inhalt abstimmen, dann bauen.

## Reihenfolge

```text
IM0 -> IM1 -> IM2 -> IM3 -> IM6
              IM4 nach IM1, parallel zu IM2/IM3
              IM5 nach IM2 (mit Gerät), sonst nur der Vermerk in IM6
```

Aufwand zusammen M. Die Arbeit liegt in `build_rs.py`, `tools/apple.py`, cargo-jflib, `lib/rt` (Auswahl der Module),
`lib/plat` (`apple.rs`, `apple_objc.rs`, `apple_crash.rs`, `cpu.rs`) und `apple.yml`.

## Prüfung

- **Rosetta 2 auf dem Mac mini:** die Suite des Macs für den x86-64-Teil. Die Ausgaben werden mit denen des
  arm64-Teils verglichen (Entscheidung 2b) oder mit Windows x86-64 (2a).
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
  der Intel-Teil nur noch gebaut, nicht mehr geprüft (Entscheidung 4).
- **Größe:** Das Programm in `.app` und `.dmg` wird etwa doppelt so groß; Schriften und Bilder bleiben einmal drin.

## Entscheidungen

Offen; jeweils mit Empfehlung.

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
