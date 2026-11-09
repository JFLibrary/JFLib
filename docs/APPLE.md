# Apple-Plan: macOS und iOS nativ

Stand 2026-11-10, auf Basis von `main` bdda550b. Umgesetzt sind AP0 (Mac und iPhone bereit), AP2 (Bau, `rt`
und der Kern von `plat`) und AP3 (`plat` ohne Fenster). Die Aufwände sind geschätzt.

## Ziel

Die Programme der Bibliothek laufen nativ auf einem Mac mit Apple Silicon (`aarch64-apple-darwin`) und auf dem
iPhone (`aarch64-apple-ios`, im Simulator `aarch64-apple-ios-sim`). Gebaut und getestet wird auf dem Mac, gesteuert
vom PC aus per SSH. Windows, Linux, Android und Web ändern sich dabei nicht.

Nicht Teil dieses Plans: Intel-Macs, der App Store als Ziel, iPad-eigene Oberflächen, native Plugins auf iOS.

## Ausgangslage

Heute laufen macOS und iOS nur über `--target web` (README, Tabelle der Plattformen). Den Stil `macos` gibt es schon,
er ist von der Plattform unabhängig.

Was hilft:

- **Android ist die Vorlage.** Android läuft auf ARM64 mit einer libc (Bionic) statt eigener Syscalls, und ein
  dünner Host (Java, `android/`) treibt die Oberfläche. `rt` wählt dort `portable/cpu.rs`, `portable/math.rs`,
  `aarch64/fx.rs` und `android.rs` (`lib/rt/lib.rs`), `plat` nimmt den gemeinsamen Host-Adapter `web.rs` mit den
  Android-Teilen. Genau so kann iOS laufen, und `rt` kann für beide Apple-Ziele fast dieselbe Auswahl treffen.
- **NEON** (`lib/simd/neon.rs`) und `rt/aarch64/fx.rs` gibt es seit ARM6.
- **Die Oberfläche rastert auf der CPU.** Für das erste Bild reicht es, fertige Pixel anzuzeigen.
- **GPU ist entschieden** (ROADMAP §8, [GPU.md](GPU.md)): Metal für Apple, Shader aus WGSL mit dem eigenen Compiler,
  später auch nach MSL.
- **Std-Programme** (CR6, Feature `std`) sind auf Apple ohne Zusatzarbeit möglich, std ist dort vollständig.

Was fehlte (2026-11-08): jede Zeile Apple-Code. `target_os = "macos"` kam im ganzen `lib/` einmal vor. `build_rs.py`
kannte kein Apple-Ziel, die Update-Manifeste kein `macos-aarch64`.

## Entscheidungen (Vorschläge)

1. **Gebaut wird auf dem Mac.** Das SDK von Apple darf nur auf Apple-Hardware laufen, Xcode bringt Simulator,
   Gerätewerkzeuge und `codesign`. Der PC steuert per SSH, wie beim Fedora-Rechner.
2. **libSystem statt eigener Syscalls.** macOS hat keine stabile Syscall-Schnittstelle, nur libSystem ist
   zugesichert. `rt` bekommt eine `apple.rs` nach dem Muster von `android.rs`. Damit hängt Apple **nicht** an ARM2
   und ARM3 (Linux ohne libc).
3. **Objective-C über die Laufzeit, aus Rust.** `objc_msgSend` mit einer kleinen eigenen Bindung, wie der eigene
   Wayland- und X11-Code. Keine Swift- oder Objective-C-Quellen, kein Xcode-Projekt, keine fremde Crate in der
   Bibliothek. Apps mit `std` dürfen fremde Crates nutzen wie überall.
4. **macOS als Desktop, iOS wie Android.** macOS bekommt ein eigenes Backend wie Windows und Linux (mehrere
   Fenster, Popups, Menüs). iOS nimmt den Host-Weg von Android: `UIApplicationMain` kehrt nie zurück, also treibt
   der Host die Oberfläche.
5. **Erst CPU, dann Metal.** Das erste Bild geht als Pixel über `CALayer`/`IOSurface`. Metal ist ein eigener
   Meilenstein.
6. **Erst ohne native Plugins**, wie auf Android. Auf iOS bleibt das so (Apple verbietet nachgeladenen Code), auf
   macOS ist es ein späterer Punkt mit eigener Entscheidung (signierte `.dylib` per `dlopen` oder eigener Loader
   mit `MAP_JIT`) und hängt an ARM4.
7. **Mindestversionen:** macOS 13 und iOS 16.
8. **App-IDs** wie bisher: `org.jflib.*` für Demos und Tests, `org.jfreund.*` für die eigenen Apps.

## Meilensteine

Größe: S klein, M mittel, L groß, nur relativ zueinander.

### AP0: Mac und iPhone bereit (S)

- Mac: Entfernte Anmeldung an, SSH-Schlüssel des PCs, kein Ruhezustand am Netzteil. Xcode, Rust mit den drei
  Zielen, Python 3.11, das Repository unter `~/src/JFLib`, eine `~/jf-env.sh` wie auf Fedora.
- iPhone: gekoppelt, Entwicklermodus an, ein Xcode-Beispiel einmal per Hand darauf gestartet.
- Signieren: ein Entwicklerzertifikat im Schlüsselbund. Das richtet der Nutzer ein, Passwörter gibt Claude nicht ein.

**Fertig, wenn:** ein kleines Rust-Programm per SSH im Terminal des Macs, im Simulator (`xcrun simctl`) und auf
dem iPhone (`xcrun devicectl`) läuft.

**Umgesetzt (2026-11-10):** Mac mini mit M2 (16 GB, macOS 27.2, Xcode 27.2), iPhone 14 mit iOS 27.2. Auf dem Mac
liegen Rust 1.98.1 über rustup mit `aarch64-apple-ios` und `aarch64-apple-ios-sim` und Python 3.11 über `uv`, beides
ohne sudo. Ein Rust-Programm, das Betriebssystem und Architektur ausgibt, läuft per SSH auf dem Mac (der Linker
signiert ad hoc), in einem Simulator ohne Fenster (`simctl boot`, `simctl spawn`) und auf dem iPhone
(`devicectl device install app`, `devicectl device process launch --console`):
`JFLib AP0: hello from ios / aarch64`.

- **Fürs iPhone signieren geht nicht per SSH.** Der Schlüsselbund ist in jeder SSH-Sitzung gesperrt („User
  interaction is not allowed“), `codesign` scheitert dann mit `errSecInternalComponent`. Bis AP5 signiert ein Skript
  in der angemeldeten Sitzung: `open sign.command` startet es per SSH im Terminal des Macs. Für AP5 braucht es einen
  eigenen Schlüsselbund nur zum Signieren, eingerichtet vom Nutzer.
- **Das Profil kommt nur aus Xcode.** Mit der kostenlosen Apple-ID legt Xcode es an, wenn ein Projekt auf dem Gerät
  startet (hier `org.jflib.JFHello`, Team `6RBCBNA6DJ`). Es gilt sieben Tage, danach das Projekt einmal neu starten.
- **Das Zwischenzertifikat G3** von Apple fehlte im Schlüsselbund. `security find-identity -v` zählte das
  Zertifikat deshalb als ungültig, `codesign` fand keine Kette.
- **`process launch` wartet still**, solange das iPhone gesperrt ist, ohne Meldung und ohne Ende.
- Xcode 27 hat kein Fenster „Devices and Simulators“ mehr, Geräte zeigt der „Device Hub“. Per SSH reichen
  `devicectl` und `xctrace`.

### AP1: Safari prüfen (S, sofort, parallel)

- Die Web-Ausgabe (`gallery`, `jfWord`, `Kritzel`) in Safari auf dem Mac (`safaridriver`) und auf
  dem iPhone (Web-Inspektor über den Mac). Das ist heute der Weg für Apple und noch nie getestet.

**Fertig, wenn:** die Browser-Skripte in Safari auf dem Mac laufen und die Abweichungen auf dem iPhone notiert sind.

### AP2: Bau und `rt` (M)

- `build_rs.py` und `cargo-jflib`: `--target mac` und `--target ios` (mit `--sim`), `TRIPLE`, `TARGETS` der Apps,
  Ausgabe nach `build/mac/` und `build/ios/`.
- `tools/apple.py` nach dem Muster von `tools/android.py`: `.app` mit `Info.plist` packen, signieren (ad hoc für
  den Mac, Entwicklerzertifikat für das iPhone).
- `rt/apple.rs`: Speicher, Ausgabe, Ende, Threads über libSystem; Auswahl in `rt/lib.rs` wie bei Android.

**Fertig, wenn:** `coretest` und `portabletest` auf dem Mac und im Simulator grün sind und die Ergebnisse
von `portabletest` denselben Digest haben wie auf dem A56.

**Umgesetzt (2026-11-10), mit zwei Änderungen am Plan:** `plat` kannte nur die Backends Windows, Linux und
Web/Android, ohne ein Apple-Backend baute also kein Programm. Der Kern von `plat` kam deshalb schon hier dazu.
`coretest` braucht dagegen Netz, Sperren, `fs` und Prozesse, die es bisher nur für Windows und Linux gibt, und
rückt nach AP3. `cargo-jflib` folgt, sobald Apple-Programme Fenster haben.

- **`rt`:** [`lib/rt/apple.rs`](../lib/rt/apple.rs) nimmt `malloc`, `mmap`, `write` und `_exit` aus libSystem, wie
  `android.rs` aus Bionic; dazu `portable/cpu.rs`, `portable/math.rs` und `aarch64/fx.rs`.
- **`plat`:** [`apple.rs`](../lib/plat/apple.rs) mit `main`, Argumenten und Umgebung (`_NSGetArgv`,
  `_NSGetEnviron`, gilt auch für Programme mit std) und Zeit; [`apple_sys.rs`](../lib/plat/apple_sys.rs) mit
  Dateien und Ordnern (`config://` und `data://` beide unter `~/Library/Application Support`, `cache://` unter
  `~/Library/Caches`); [`apple_thread.rs`](../lib/plat/apple_thread.rs) mit pthreads, Semaphoren von libdispatch,
  einem Weck-Pipe und Systemangaben; [`apple_disp.rs`](../lib/plat/apple_disp.rs) beantwortet die Fenster-API
  wie Linux ohne Display. Plugin-Bibliotheken, Module und ICC nehmen die Web-Fassungen, `screen.rs` kennt noch
  keine Monitore, ohne Ton und Drag and Drop gibt es keine Warnungen über ungenutzten Code.
- **Bau:** `build_rs.py app|lib|run --target mac` und `--target ios` (mit `--sim` nach `build/ios-sim/`), nur auf
  einem Mac; Mindestversionen über `MACOSX_DEPLOYMENT_TARGET` und `IPHONEOS_DEPLOYMENT_TARGET`. `jflib-build`
  kennt macOS und iOS. Für `ios` packt [`tools/apple.py`](../tools/apple.py) jedes Programm als `NAME.app`, die
  Simulator-Fassung ad hoc signiert; `tools/apple.py sim` startet im Simulator ohne Fenster, `tools/apple.py
  device APP.app` signiert, installiert und startet auf dem iPhone. Über SSH signiert es im Terminal des Macs.
- **Ergebnis:** `portabletest` gibt auf dem Mac, im Simulator, auf dem iPhone 14 und unter Windows x86-64
  denselben Digest aus, `edea7e9d9579c77a` (auch der des Browsers). Windows, Linux, wasm32 und Android bauen
  `rt` und `plat` weiter ohne Warnung.
- **Python:** `tools/gendoc.py` brauchte Python 3.12, das README verspricht 3.11; jetzt läuft `build_rs.py`
  wieder mit 3.11. Nur `apps/androidGallery/tests/android_perf.py` braucht noch 3.12.

### AP3: `plat` ohne Fenster auf macOS (M)

- Schon da (AP2): Dateien, Ordner, Zeit, Threads, Semaphoren, Weck-Pipe.
- Dazu: `fs`, `sync`, `net`, `process` (`posix_spawn`), `rand`, `watch`, Locale, Geheimnisse (Keychain),
  Absturzberichte (Signale), `spawn_self`.

**Fertig, wenn:** `coretest` und die Teile von `plattest` und `asynctest` ohne Fenster auf dem Mac grün sind.

**Umgesetzt (2026-11-10):** `coretest` auf dem Mac 4726 von 4726, `plattest --test all` 212 von 212, dazu die einzelnen
Tests ohne Fenster grün (unter anderem `crash`, `panic`, `spawn`, `launch`, `vfs`, `dirs`, `instance`, `log`, `policy`,
`print`, `spell`; `secret` in der angemeldeten Sitzung, per SSH ist der Schlüsselbund gesperrt). `asynctest` öffnet ein
Fenster und wartet auf AP4. Im iOS-Simulator `coretest` 4715 von 4717: iOS hat keine Ordnerüberwachung, und der Pfad
eines Unix-Sockets im Temp-Ordner des Simulators ist länger als die 104 Bytes von `sun_path`.

- **Module:** `process` ([`process_apple.rs`](../lib/plat/process_apple.rs): `posix_spawn` mit eigener Prozessgruppe,
  SIGPIPE zurück auf Standard, `POSIX_SPAWN_CLOEXEC_DEFAULT`; `launch` mit eigener Session), `sync` (Tabelle aus
  pthread-Mutex und -Bedingung statt Futex, denn `os_sync_wait_on_address` gibt es erst ab macOS 14.4), `net` (BSD-Sockets,
  `sockaddr` mit Längenbyte, `SO_NOSIGPIPE`, Namen über `getaddrinfo`), `fs` (Darwins `struct stat` und `statfs`,
  `utimensat`), `rand` (`getentropy`), `watch` (FSEvents mit Dateiereignissen; beim Start eine Markierungsdatei pro
  Wurzel, vorher Gemeldetes stammt von vor dem Start), `certs` (`SecTrustEvaluateWithError`, macOS dazu
  `/etc/ssl/cert.pem`), `service` (launchd: `/Library/LaunchDaemons`, `plist_text`), `control_ipc`
  ([`control_ipc_apple.rs`](../lib/plat/control_ipc_apple.rs): Unix-Sockets mit `getpeereid`; die Einzelinstanz als
  Socket-Datei in einem privaten Ordner statt eines abstrakten Namens), `secret`
  ([`apple_secret.rs`](../lib/plat/apple_secret.rs): generische Passwörter der Keychain).
- **Abstürze:** [`apple_crash.rs`](../lib/plat/apple_crash.rs): Signale auf einem eigenen Stack pro Thread, die
  Register aus dem Maschinenkontext, die Aufrufkette über die Frame-Zeiger, die Apples ARM64-ABI vorschreibt; ein
  Fehler knapp unter dem Stack ist ein Stack-Überlauf. `crash_test` gibt es für ARM64.
- **Weiter:** Ordner (`~/Documents` usw., Logs in `~/Library/Logs/<app>`, `HOME` sonst aus `getpwuid_r`), `File::set_len`,
  `create_new`, `spawn_self` / `restart_self`, `open_url` / `open_folder` über `/usr/bin/open`, `print` mit der
  Testvariante `jfgui_PRINT_TO`, `cpu_brand`, Hunspell in `~/Library/Spelling` und `/Library/Spelling`.
- **Noch nicht (AP7):** Papierkorb, Autostart, Dateizuordnungen, zuletzt benutzte Dateien, die Rechtschreibung des
  Systems, die Locale des Systems; `plattest` meldet diese Tests auf Apple als „not on Apple yet“.

### AP4: Fenster auf macOS (L)

- `NSApplication`, `NSWindow`, eigene `NSView`; die Ereignisschleife bleibt die eigene (`nextEventMatchingMask`).
- Maus, Tastatur, Trackpad (feines Scrollen, Zoom-Geste), Eingabemethoden (`NSTextInputClient`), Retina-Skala,
  Cursor, Popups, Zwischenablage, Dateidialoge, Drag and Drop, Hell/Dunkel und Akzentfarbe, mehrere Bildschirme.
- Tastenkürzel: Cmd statt Strg, wo der Stil `macos` es verlangt.

**Fertig, wenn:** `gallery`, `uitest` und `dndtest` mit ihren Skripten auf dem Mac grün sind.

**Umgesetzt (2026-11-10), mit einer Änderung am Prüfkriterium:** `build_rs.py test gallery|uitest|dndtest --target mac`
ist grün (42, 1 und 3 Skripte). Skripte laufen aber immer headless (`--script`), sie prüfen das AppKit-Backend gar
nicht. Das übernehmen deshalb zwei Tests in echten Fenstern ohne Benutzer: `plattest --test window` (50 Prüfungen) und
`plattest --test dnd` (49), dazu ein Lauf der echten Gallery mit `--auto-close`. Ereignisse gehen dabei nur an die
eigenen Fenster (`[NSWindow sendEvent:]` oder direkt an die View), nichts erreicht den Ereignisstrom des Systems.

- **Backend:** [`apple_mac.rs`](../lib/plat/apple_mac.rs) über die Objective-C-Laufzeit
  ([`apple_objc.rs`](../lib/plat/apple_objc.rs): Klassen, Selektoren, `objc_msgSend` mit genauer Signatur pro Aufruf,
  eigene Klassen mit Methoden und einer Instanzvariable). `NSApplication` mit der eigenen Schleife
  (`nextEventMatchingMask` bis zur Frist, `wake()` über eine `CFRunLoopSource`, die ein anwendungsdefiniertes Ereignis
  postet); jedes Fenster ein `NSWindow` mit eigener View `JFView` (gespiegelt, mit Layer) und eigenem Delegate. Das Bild
  wird als `CGImage` Inhalt des Layers, die Größe ist Punkte mal `backingScaleFactor`. Ohne `PWF_SYSFRAME` reicht der
  Inhalt unter eine durchsichtige Titelleiste ohne Knöpfe, `HB_DRAG` zieht das Fenster über
  `performWindowDragWithEvent:`.
- **Eingabe:** Maus, Rad (Trackpad fein: 3 pro Punkt, Mausrad 120 pro Zeile), die Pinch-Geste als Strg + Rad wie unter
  Windows, Tasten als Windows-VK (Buchstaben und Ziffern nach dem Layout), Text und Eingabemethoden über
  `NSTextInputClient` (`E_PREEDIT`, `E_IME_COMMIT`, das Kandidatenfenster am Caret). Command ist `MOD_CTRL`, Control
  `MOD_SUPER`, Option `MOD_ALT`; ein Key-up mit Command reicht die Schleife selbst ans Fenster weiter.
- **Fenster:** Popups als randlose, nicht aktivierende `NSPanel` mit runden Ecken und Schatten, Kindfenster des
  Besitzers, ihre Eingabe kommt als Ereignis des Besitzers; Fenster mit Besitzer hängen als Kindfenster an ihm (über ihm,
  ziehen mit); `win_enable`, eigene Cursor (`NSBitmapImageRep`), `CUR_NONE`; eigene Größenänderungen werden nicht
  zurückgemeldet.
- **System:** Zwischenablage (Text, HTML, RTF, PNG, Bilder: TIFF kommt als PNG an, ein BMP allein bekommt ein PNG dazu),
  Öffnen- und Sichern-Dialog (`NSOpenPanel`, `NSSavePanel`, `FD_FOLDER` wählt einen Ordner), Drag and Drop in beide
  Richtungen ([`apple_mac_dnd.rs`](../lib/plat/apple_mac_dnd.rs)), Monitore (`NSScreen`), Hell/Dunkel, Akzentfarbe, mehr
  Kontrast, weniger Bewegung, Doppelklickzeit; ein Wechsel von Erscheinungsbild, Akzentfarbe oder Bildschirmen schickt
  `E_SETTINGS`. Der Quell-Inspektor öffnet den Editor auch auf macOS.
- **Bau und Start:** `gallery`, `uitest` und `dndtest` bauen für `mac`, `build_rs.py test NAME --target mac` geht.
  Programme mit Fenstern, die per SSH starten, laufen über `tools/apple.py gui` in der angemeldeten Sitzung: `open`
  startet ein kleines Bündel, dessen Skript das Programm im Repository startet und Ausgabe und Exit-Code zurückgibt.
  `build_rs.py` nimmt diesen Weg selbst für Apps mit `ui`.
- **Testhilfen** ([`apple_mac_test.rs`](../lib/plat/apple_mac_test.rs)): `win_test` mit `WT_SEND` (Maus, Tasten, Text,
  Rad, Pinch an das eigene Fenster), `WT_MOVE` (der Benutzer verschiebt), Pixel des Layers, Popup-Lage und -Eingabe,
  Elternfenster, Caret, Mindestgröße; `file_dialog_autoclose`; `dnd_test_formats` und `dnd_test_drop` (ein
  Stellvertreter für `NSDraggingInfo`).
- **Noch offen:** Der Test-Mac hängt an einem 1080p-Bildschirm (Skala 1), Retina ist also nur im Code berücksichtigt,
  nicht geprüft. Tool-Fenster nehmen beim Klick die Tastatur (unter Windows nur ein Textfeld, auf macOS ginge das mit
  `NSPanel`). Menüleiste, Cmd+Q, `E_POWER` und `E_SESSION_END` folgen mit AP7, Farbwähler und Bildschirmfoto
  (`screen.rs`) später. Der Monitor des Test-Macs meldet keinen Namen.

### AP5: iOS (M)

- Host in Rust über die Objective-C-Laufzeit: App-Delegate, View mit `CALayer`, Berührung und Apple Pencil,
  Bildschirmtastatur (`UITextInput`), sichere Bereiche, Drehen, Hintergrund und Beenden.
- Dateien über `UIDocumentPickerViewController` (wie das Storage Access Framework auf Android), Zwischenablage,
  Keychain, Teilen.
- Erste App wie `androidGallery`: eine Gallery für das iPhone.

**Fertig, wenn:** die Gallery im Simulator mit Skripten grün ist und auf dem iPhone von Hand geprüft ist.

### AP6: Metal (L)

- `lib/gpu` mit einem Metal-Backend neben Vulkan und WebGPU, `CAMetalLayer` zum Anzeigen.
- `lib/wgsl` erzeugt MSL.

**Fertig, wenn:** `gputest` und `uigputest` auf dem Mac und dem iPhone grün sind.

### AP7: Desktop-Dienste auf macOS (M)

- Menüleiste oben, Dock, Statusmenü (Tray), Mitteilungen, Dateizuordnungen (`Info.plist`), Autostart
  (`SMAppService`), Ruhezustand verhindern, Drucken, App-Sandbox und die Rechte aus [PERMISSIONS.md](PERMISSIONS.md).

**Fertig, wenn:** die Desktop-Apps (`jfWord`, `BackupCraft`, JFPainter) auf dem Mac ohne fehlende Grundfunktionen
laufen.

**Aufteilung (2026-11-10).** Die Bestandsaufnahme nach AP4 ergab mehr als die Liste oben: Papierkorb, zuletzt
benutzte Dateien, Locale und Rechtschreibung des Systems (aus AP3), globale Tastenkürzel, `E_POWER` und
`E_SESSION_END` sowie Cmd+Q fehlen ebenfalls. Eine Menüleiste aus den Menüs der App gibt es auf keiner Plattform,
die UI zeichnet ihre Menüs selbst; oben steht deshalb zunächst nur das App-Menü des Systems. AP7 läuft in drei
Schritten, jeder für sich prüfbar und mergebar:

- **AP7a, die App im System:** App-Menü (Über, Ausblenden, Beenden mit Cmd+Q als `E_CLOSE` des Hauptfensters,
  Fenster-Menü mit Cmd+M), App-Delegate (Beenden über Dock oder Abmelden fragt die App), `E_POWER` und
  `E_SESSION_END` (`NSWorkspace`), Ruhezustand verhindern (`IOPMAssertion`), Papierkorb (`NSFileManager`), zuletzt
  benutzte Dateien (`NSDocumentController`), Sprachen und Format des Systems (`CFLocale`), die Rechtschreibung des
  Systems (`NSSpellChecker`), Drucken über den Druckdialog des Systems, Tool-Fenster, die beim Klick die Tastatur nur
  für ein Textfeld nehmen (wie unter Windows).
- **AP7b, Dienste:** Statusmenü (`NSStatusItem`) und Mitteilungen, Dock (Badge, Fortschritt), Autostart (Login-Item
  über einen LaunchAgent, mit Bündel `SMAppService`), globale Tastenkürzel, Dateien und URLs, mit denen das System die
  App öffnet (`application:openURLs:`).
- **AP7c, Bündel und Apps:** GUI-Apps als `NAME.app` mit `Info.plist` und Symbol (`.icns`) aus `build_rs.py` und
  `cargo-jflib`, Dateizuordnungen im `Info.plist`, App-Sandbox und die Rechte aus PERMISSIONS.md als Entitlements;
  dann `jfWord`, `BackupCraft` und JFPainter auf dem Mac.

**AP7a umgesetzt (2026-11-10):** [`apple_mac_app.rs`](../lib/plat/apple_mac_app.rs) baut beim ersten Fenster das
App-Menü und das Fenster-Menü (Deutsch oder Englisch nach der ersten Sprache des Systems). Cmd+Q, „Beenden“ im Dock
und das Abmelden fragen über `applicationShouldTerminate:` das Hauptfenster (`E_CLOSE`), die App entscheidet. Ein
Klick aufs Dock-Symbol holt das Hauptfenster zurück. `NSWorkspace` meldet Ruhezustand und Aufwachen (`E_POWER`) und das
Ausschalten (`E_SESSION_END`); SIGTERM kommt über eine Dispatch-Quelle der Hauptschlange ebenfalls als
`E_SESSION_END`. Tool-Fenster sind Panels einer eigenen Klasse und werden nur bei einem Klick in ein Textfeld zum
Schlüsselfenster. Dazu: Ruhezustand verhindern ([`apple_power.rs`](../lib/plat/apple_power.rs), IOKit-Assertions),
Papierkorb (`NSFileManager`), zuletzt benutzte Dateien ([`apple_recent.rs`](../lib/plat/apple_recent.rs),
`NSDocumentController`), Sprachen und Format des Systems (`CFLocale`, auch iOS; von Hand gesetzte `LANGUAGE`,
`LC_*` gehen vor, `LANG` allein nicht, weil Terminal es aus der Region setzt), die Rechtschreibung des Systems
([`apple_spell.rs`](../lib/plat/apple_spell.rs), `NSSpellChecker`, auf dem Test-Mac 44 Sprachen) und der Druckdialog
(`NSPrintPanel`, danach das PDF des Auftrags über PDFKit auf dem gewählten Papier).

Geprüft auf dem Mac: `plattest --test trash` (19), `power` (10), `spell` (57), `recent` (43), `all` (212) per SSH,
`--test window` (57) in der angemeldeten Sitzung, mit Testhilfen für Menü, Beenden, `NSWorkspace`, SIGTERM und das
abgebrochene Druckpanel. Der Panel-Fokus braucht eine aktive App; er lief erst mit AP7b, als der Test-Mac nicht mehr
auf dem Sperrbildschirm stand.

**AP7b umgesetzt (2026-11-10):** [`apple_mac_services.rs`](../lib/plat/apple_mac_services.rs): das Statusmenü als
`NSStatusItem` (Symbol 18 Punkte hoch, sonst das der App; mit Menü öffnet ein Klick das Menü wie auf dem Mac üblich,
ohne Menü meldet er `Activate`), Mitteilungen über `UNUserNotificationCenter` mit Delegate für Klicks und für das
Zeigen im Vordergrund, das Dock (Badge, der Fortschritt in die Kachel gezeichnet, die Aufgaben als Dock-Menü), Dateien
und URLs, die das System mit der App öffnet (`application:openURLs:`), und globale Tastenkürzel über Carbons
`RegisterEventHotKey`, das keine Freigabe braucht. Gewählte Dock-Aufgaben und vom System geöffnete Dateien kommen bei
der App an wie ein späterer Start (`instance::queue_push`, `UIC_OPEN`), ganz ohne zweiten Prozess. Autostart
([`apple_autostart.rs`](../lib/plat/apple_autostart.rs)) ist ein LaunchAgent in `~/Library/LaunchAgents`, der auch
ohne Bündel geht.

Geprüft auf dem Mac: `plattest --test autostart` (12) in einem Scratch-`$HOME`, `--test window` (76) mit Statusmenü
(Menü, gewählte Zeile, Klick), Dock (Badge, Fortschritt, Aufgaben), `openURLs` und einem Tastenkürzel (sein
Carbon-Ereignis, dasselbe Kürzel ein zweites Mal ergibt `Conflict`). **Fallen:** macOS verweigert Mitteilungen für
Programme ohne Bundle-ID; `currentNotificationCenter` wirft dort sogar eine Ausnahme. Ohne Bündel liefert `notify`
deshalb 0, die echte Prüfung folgt mit den Bündeln in AP7c (macOS fragt den Nutzer dann einmal um Erlaubnis).
Abschlussblöcke des Systems sind Objective-C-Blöcke: Ihre Funktion liegt im Block-Literal bei Offset 16, eigene
Blöcke sind globale Blöcke (`_NSConcreteGlobalBlock`).

### AP8: Pakete, Updates, CI, Dokumentation (M)

- `.app` als `.dmg`, Signatur mit Developer ID und Notarisierung, Update-Kanal `macos-aarch64`, iOS über TestFlight,
  falls gewünscht.
- `.github/workflows/apple.yml` auf den macOS-Runnern von GitHub (kostenlos für öffentliche Repositories), mit
  Simulator, Start nur per Hand, bis der Lauf stabil ist.
- Erst jetzt README, [PACKAGING.md](PACKAGING.md), GUIDE und Website.

**Fertig, wenn:** ein signiertes und notarisiertes Programm auf einem fremden Mac ohne Warnung startet und sich
aktualisiert.

## Reihenfolge

```text
AP0 -> AP2 -> AP3 -> AP4 -> AP7 -> AP8
AP1 (Safari): sofort, parallel
AP5 (iOS): nach AP2, parallel zu AP3/AP4
AP6 (Metal): nach AP4 oder AP5
```

Vom ARM64-Plan nutzt Apple NEON (ARM6, fertig), später ARM4 (Plugin-ABI, nur für macOS-Plugins) und ARM7: Ein Mac
mit Apple Silicon ist echte ARM-Hardware für die Prüfung der Atomics.

## Fallen

- **Keine Syscalls direkt:** Nummern und Aufrufe ändern sich mit macOS-Updates, nur libSystem bleibt.
- **Seiten haben 16 KB** auf Apple Silicon. `4096` steht an mehreren Stellen in `plat`.
- **Register `x18`** gehört auf Apple dem System, handgeschriebener Assembler darf es nicht anfassen.
- **Variadische Aufrufe** legen auf Apple ARM64 alle Argumente auf den Stack. `objc_msgSend` muss pro Aufruf auf
  die genaue Signatur umgewandelt werden, nie variadisch.
- **`c_char` ist auf Apple vorzeichenbehaftet**, anders als auf Linux ARM64.
- **Nur der Hauptthread** darf AppKit und UIKit anfassen.
- **Koordinaten:** AppKit zählt von unten links und in Punkten, nicht in Pixeln.
- **Signatur ist Pflicht:** Jedes ARM64-Programm muss signiert sein, ad hoc reicht lokal. Wer das Programm nach dem
  Signieren ändert, macht es ungültig.
- **Rechte für Bildschirmaufnahme und Eingabe** (TCC) hängen an der Signatur. Eine Ad-hoc-Signatur ändert sich mit
  jedem Bau, die Freigabe ginge jedes Mal verloren. Für solche Tests ein festes Zertifikat nehmen.
- **Schlüsselbund über SSH gesperrt:** Signieren mit dem Entwicklerzertifikat scheitert per SSH
  (`errSecInternalComponent`), solange der Schlüsselbund dort nicht entsperrt ist. Ad hoc geht immer.
- **Fenster aus SSH** erscheinen nur in der Sitzung des angemeldeten Nutzers. `launchctl asuser` braucht root, `open`
  geht: `tools/apple.py gui` startet darüber ein Bündel, dessen Skript das Programm im Repository-Ordner startet (bei
  LaunchServices ist der Arbeitsordner sonst `/`).
- **iOS:** kein nachgeladener Code, keine fremden Prozesse, Hintergrund nur eingeschränkt. Die Agenten von
  BackupCraft gibt es dort nicht.
- **Rosetta** als Übergang für x86-64 lohnt nicht: Apple hat angekündigt, es nach macOS 27 nur noch eingeschränkt
  anzubieten.
- **Semaphoren von libdispatch** beenden den Prozess, wenn sie mit einem kleineren Stand freigegeben werden, als
  sie angelegt wurden. `apple_thread.rs` legt sie deshalb bei 0 an und erhöht danach.
- **`readdir`:** Der Eintrag kann kürzer sein als `struct dirent` mit seinen 1024 Bytes Namen. Keine Referenz auf
  den ganzen Eintrag bilden, nur `d_namlen` Bytes lesen.
- **`fsync`** lässt die Daten auf Apple im Cache des Laufwerks, erst `F_FULLFSYNC` schreibt sie wirklich.
- **FSEvents liefert spät:** Änderungen kurz vor dem Start eines Watchers kommen erst danach an, auch nach
  `FSEventStreamFlushSync`. Erst eine Markierungsdatei grenzt sie sicher ab.
- **`sockaddr` hat auf Apple ein Längenbyte** vor der Familie (ein Byte), `AF_INET6` ist 30, `SOL_SOCKET` 0xffff, und
  `sun_path` fasst 104 statt 108 Bytes. Abstrakte Socket-Namen gibt es nicht.
- **`posix_spawn` statt `fork`:** In einem Prozess mit Threads ist `fork` unsicher, `posix_spawn` erledigt Umleitung,
  Prozessgruppe, Signale und das Schließen der übrigen Deskriptoren in einem Aufruf.
- **Skripte sind headless:** `--script` öffnet nie ein Fenster. Grüne Gallery-Skripte sagen nichts über AppKit, das
  prüfen `plattest --test window` und `--test dnd`.
- **Kein `windowDidMove`,** wenn eine Größenänderung den Ursprung verschiebt (Ziehen an der oberen oder linken Kante,
  `setFrame` mit neuer Lage und Größe). Das Backend vergleicht deshalb die Lage nach jeder Änderung selbst.
- **Key-up mit Command** behält `NSApplication` für sich, es kommt nie beim Fenster an.
- **Der Sichern-Dialog läuft in einem anderen Prozess:** `ok:` aus dem eigenen Code bewirkt nichts, `cancel:` schon. Die
  Testhilfe beendet ihn mit `[NSApp stopModal]`.
- **Benachrichtigungen wecken die Schleife nicht:** Ein Rückruf, der ein Ereignis einreiht, muss `wake()` auslösen, sonst
  wartet `nextEventMatchingMask` weiter auf ein Ereignis von AppKit.
- **Synthetische Mausbewegungen** schickt `[NSWindow sendEvent:]` an den First Responder, nicht an die View unter dem
  Punkt; echte kommen über die Tracking-Area. Ein Panel hat keine View als First Responder.
- **`clip_set_many`** nimmt nur Textformate (cliprich.rs), Bilder gehen über `clip_set`.

## Testen

| Was | Wo | Grenze |
|---|---|---|
| Logik, `coretest`, `portabletest` | Terminal des Macs per SSH | keine |
| Fenster macOS | angemeldete Sitzung auf dem Mac, per SSH über `tools/apple.py gui` | Mac darf nicht schlafen; Bildschirmfotos bräuchten eine Freigabe (TCC), die Tests lesen die Pixel des eigenen Layers |
| iOS | Simulator per `xcrun simctl`, iPhone per `xcrun devicectl` | Gerät gekoppelt, entsperrt, Entwicklermodus; Signatur braucht den entsperrten Schlüsselbund |
| Safari | Mac per `safaridriver`, iPhone per Web-Inspektor | `safaridriver --enable` einmal per Hand (Admin-Passwort) |
| CI | macOS-Runner von GitHub | keine echten Geräte, nur Simulator |

Die Regel gegen Fenster-Fluten gilt auch hier: höchstens drei Runden pro Variante, keine echte Eingabe unter dem
Zeiger des Nutzers.

## Offene Fragen

1. **Mac:** Mac mini M2 (AP0). Offen: Nutzt ihn noch jemand?
2. **iPhone:** iPhone 14 mit iOS 27.2 (AP0). Offen: Ist es das Alltagsgerät, also Tests nur auf Zuruf wie beim A56?
3. **Apple Developer Program** (99 € im Jahr) oder die kostenlose Apple-ID? Vorerst die kostenlose (AP0): Apps laufen
   auf dem iPhone nur sieben Tage, ohne TestFlight und ohne Notarisierung.
4. **Welche Apps zuerst?** Vorschlag: `gallery`, dann JFPainter und `jfWord`; auf dem iPhone die Gallery, dann
   `Kritzel`.
5. **App Store** als späteres Ziel für iOS? Das ändert Sandbox, Plugins und die Prüfregeln.
