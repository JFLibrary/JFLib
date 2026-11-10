# Apple-Plan: macOS und iOS nativ

Nachtrag 2026-11-10: AP5 ist auch von Hand auf dem iPhone geprüft, AP8 (Pakete, Updates, CI, Developer ID und
Notarisierung, Dokumentation und Website) ist fertig; offen sind AP1 (Safari) und AP6 (Metal, im Plan der GPU-Arbeit).

Stand 2026-11-09, auf Basis von `main` ba9e6d03. Umgesetzt sind AP0 (Mac und iPhone bereit), AP2 (Bau, `rt`
und der Kern von `plat`), AP3 (`plat` ohne Fenster), AP4 (Fenster auf macOS), AP7 (Desktop-Dienste) und AP5 (iOS) bis
auf die Prüfung von Hand auf dem iPhone. Die Aufwände sind geschätzt.

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

**Umgesetzt (2026-11-09):** Mac mini mit M2 (16 GB, macOS 27.2, Xcode 27.2), iPhone 14 mit iOS 27.2. Auf dem Mac
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

**AP1 auf dem Mac umgesetzt (2026-11-10):** `tools/tests/web_apps.mjs --engine=safari` fährt dieselben Prüfungen wie
in Chromium und Firefox in einem echten Safari (27.2) über `safaridriver`. Playwright steuert kein Safari (sein
„webkit“ ist ein eigener WebKit-Bau); [`tools/tests/safari.mjs`](../tools/tests/safari.mjs) bildet deshalb den Teil
von Playwrights API nach, den der Test braucht, über W3C WebDriver. [`tools/tests/safari_tunnel.py`](../tools/tests/safari_tunnel.py)
startet `safaridriver` auf dem Mac und trägt beide Richtungen über eine SSH-Verbindung: Der Test läuft auf dem PC
(auf dem Mac ist kein Node nötig), Safari lädt die Seiten unter derselben Adresse. `template`, `gallery`, `jfWord`
und `Kritzel` bestehen, auch die Pixelgleichheit mit Rusts PNG-Renderer, das Speichern in IndexedDB mit den
Fehlerfällen, Tastenkürzel, IME, das Einfügen-Ereignis und die Dateiauswahl. Nicht nachstellbar: die Sprache
(Safari nimmt die des Macs, hier Deutsch, wie der Test erwartet) und die Pixeldichten 1,5 und 2 (der Mac mini hat
1×). Einmalig auf dem Mac: `safaridriver --enable`, in Safari Entwickler → „Entfernte Automatisierung erlauben“,
danach Safari neu starten (sonst läuft die Sitzung „while connecting to a Safari instance“ in ein Timeout).
**Fallen:** Das versteckte Texteingabefeld des Hosts nimmt keine Elementeingabe von WebDriver an (der Adapter tippt
über Tastatur-Aktionen); Init-Skripte laufen erst nach dem Laden, deshalb erkennt der Test „bereit“ auch am
`data-compute` des Body; ein beendeter Tunnel ließ `safaridriver` auf dem Mac weiterlaufen (ohne Terminal kein
Hangup), das Skript beendet ihn jetzt über die geschlossene Eingabe.

### AP2: Bau und `rt` (M)

- `build_rs.py` und `cargo-jflib`: `--target mac` und `--target ios` (mit `--sim`), `TRIPLE`, `TARGETS` der Apps,
  Ausgabe nach `build/mac/` und `build/ios/`.
- `tools/apple.py` nach dem Muster von `tools/android.py`: `.app` mit `Info.plist` packen, signieren (ad hoc für
  den Mac, Entwicklerzertifikat für das iPhone).
- `rt/apple.rs`: Speicher, Ausgabe, Ende, Threads über libSystem; Auswahl in `rt/lib.rs` wie bei Android.

**Fertig, wenn:** `coretest` und `portabletest` auf dem Mac und im Simulator grün sind und die Ergebnisse
von `portabletest` denselben Digest haben wie auf dem A56.

**Umgesetzt (2026-11-09), mit zwei Änderungen am Plan:** `plat` kannte nur die Backends Windows, Linux und
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

**Umgesetzt (2026-11-09):** `coretest` auf dem Mac 4726 von 4726, `plattest --test all` 212 von 212, dazu die einzelnen
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

**Umgesetzt (2026-11-09), mit einer Änderung am Prüfkriterium:** `build_rs.py test gallery|uitest|dndtest --target mac`
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

**Ansatz (2026-11-09), nach Entscheidung 4:** iOS nimmt den Host-Weg von Android. `ui::launch` legt die
Laufzeit an (`HostRuntime`, `plat::web::install`) und kehrt zurück, `main` übergibt dann den Hauptthread an
`UIApplicationMain`, und der Host in Rust treibt die Oberfläche über dieselben Einstiege wie `android/bridge.c`
(`jfgui_tick`, `jfgui_input`, `jfgui_finger`, `jfgui_event`, `jfgui_text`; `jfgui_host_present`, `jfgui_host_request`).
Von `web.rs` gilt auf iOS nur der Host-Teil (Fenster in einer Fläche, Eingabe, Anfragen, Systemwerte); Zeit,
Argumente, Umgebung, Threads, Dateien, Zwischenablage und Dialoge sind die nativen von `apple*.rs`. Programme ohne
Fenster (Tests mit `simctl spawn`, `--script`, `--shot`) starten wie bisher ohne UIKit. Die Schritte:

- **AP5a, Fenster und Eingabe:** ein Vollbild-`UIWindow` mit eigener View (Bild als Inhalt des `CALayer` wie auf dem
  Mac), Skala und Drehen, sichere Bereiche, Berührung über den Finger-Weg der UI, Apple Pencil, die
  Bildschirmtastatur (`UIKeyInput`, Texteingabe), Tasten einer Hardware-Tastatur, Hintergrund und Beenden.
- **AP5b, Systemdienste:** Zwischenablage (`UIPasteboard`), Dateien über `UIDocumentPickerViewController`, URLs
  öffnen, Teilen, Hell/Dunkel, Rechtschreibung (`UITextChecker`).
- **AP5c, die Gallery:** `androidGallery` (die mobile Gallery) für `ios`, ihre Skripte im Simulator, ein Lauf im
  Simulator mit Bildschirmfoto und die Prüfung von Hand auf dem iPhone.

**AP5a umgesetzt (2026-11-09):** [`apple_ios.rs`](../lib/plat/apple_ios.rs) registriert `JFAppDelegate`,
`JFSceneDelegate`, `JFViewController` und `JFView` über die Laufzeit; `Info.plist` nennt die Szene
(`UIApplicationSceneManifest`, `tools/apple.py`). Die View liegt im sicheren Bereich über der Tastatur
(`keyboardLayoutGuide`) und zeigt das ganze Bild als `CGImage` im Layer. Pixel pro Punkt: die Skala des Bildschirms,
höchstens 2 und höchstens 2048 Pixel an der langen Seite wie bei Android, dpi 96 mal das. Ein `CADisplayLink` tickt,
solange die App animiert, sonst ruht er bis zur Frist von `jfgui_frame_delay` (ein Timer von libdispatch); Eingaben und
`wake()` wecken ihn (im Leerlauf 0,1 % CPU im Simulator). Finger gehen mit allen zusammengefassten Proben und ihrem
Alter an `jfgui_finger`, der Apple Pencil als Stift (Druck, Neigung aus Höhe und Azimut), der Zeiger eines Trackpads
als Maus. Die Bildschirmtastatur ist `UIKeyInput` der View: sie wird First Responder, wenn eine Geste ein Widget
erreicht hat und die App Text will (`REQUEST_IME` mit Höhe), und gibt ab, wenn die App keinen mehr will. Tasten einer
Hardware-Tastatur nimmt der View-Controller (`pressesBegan`); was das Textsystem tippt, bleibt ihm. Aktiv und inaktiv
werden `E_FOCUS_IN` / `OUT`, Hintergrund und Vordergrund `E_POWER` 2 / 3, Beenden `E_SESSION_END`. Das Hauptfenster
ist maximiert (keine Titelleiste). Die Leisten um den sicheren Bereich und der Stil der Statusleiste folgen der Farbe
oben links im Bild, nicht dem System (eine App darf ihr eigenes Design behalten).

**AP5b umgesetzt (2026-11-09):** Zwischenablage über `UIPasteboard` (Text, HTML, RTF, PNG; Bilder anderer Formate
als PNG über `UIImage`), [`apple_ios_services.rs`](../lib/plat/apple_ios_services.rs). Öffnen, Speichern unter und
Ordner über `UIDocumentPickerViewController`, wie auf Android später beantwortet (`FD_PENDING`, `file_dialog_poll`):
Öffnen und Ordner an Ort und Stelle (der Zugriff auf die gewählte URL gilt für die Sitzung), Speichern unter
exportiert eine leere Datei des vorgeschlagenen Namens dorthin, wo der Nutzer sie ablegt, und die App schreibt sie
dann. URLs über `UIApplication openURL`. Hell/Dunkel, weniger Bewegung und dunklere Farben gehen über `jfgui_system`
(`web_system.rs`). Rechtschreibung über `UITextChecker` (`apple_spell.rs`; `plattest --test spell` im Simulator 60 von
60). Der Schlüsselbund war schon da (`apple_secret.rs`). `plattest --test clip` (neu, nicht in `all`: überschreibt
die Zwischenablage) als App im Simulator 11 von 11.

**AP5c (2026-11-09):** `androidGallery` hat `ios` in `TARGETS`. `build_rs.py test androidGallery --target ios --sim`
führt ihre zehn Skripte im Simulator aus, alle grün; dafür startet `tools/apple.py sim` Programme im Ordner des
Repositorys (`jfgui_CWD`, nur iOS). Ein Lauf als App im Simulator zeigt die Gallery mit Statusleiste, ohne Titelleiste,
hell und bei dunklem System. Die Prüfung von Hand auf dem iPhone steht aus.

**Nach der Prüfung am iPhone (2026-11-09):** Alles lief bis auf drei Punkte, die nachgezogen sind. Die
Bildschirmtastatur ließ sich nur schließen, wenn ein Tipp den Fokus woandershin brachte (das iPhone hat keine Taste
dafür, ein Tipp auf leere Fläche ist für die UI ein Klick und lässt den Fokus im Textfeld): Über der Tastatur steht
jetzt eine Leiste mit „Fertig“ (`inputAccessoryView`), das Textfeld behält den Fokus, ein Tipp hinein bringt die
Tastatur zurück. Mitteilungen gehen über `UNUserNotificationCenter` (iOS fragt beim ersten Mal, abgelehnt
verschwindet die Mitteilung mit `NoteClosed`, ein Tipp darauf ist `NoteClicked`); die Blöcke dafür teilen sich macOS
und iOS ([`apple_block.rs`](../lib/plat/apple_block.rs)). Die Sensoren und der Standort kommen über den Host-Weg von
Android (`web_sensors.rs`) aus Core Motion und Core Location
([`apple_ios_motion.rs`](../lib/plat/apple_ios_motion.rs)): die Bewegung des Geräts mit 30 Hz, Bezug magnetisch
Nord (mit Magnetometer, sonst beliebig), der Standort „beim Verwenden“ (`NSLocationWhenInUseUsageDescription` setzt
`tools/apple.py` für Apps, deren PERMISSIONS `location` nennen oder die keine haben).

- **Noch offen:** Teilen (`UIActivityViewController`; die Bibliothek hat noch keine Schnittstelle dafür), markierter
  Text der Tastatur (`UITextInput` statt `UIKeyInput`: Diktat, Vorschläge, Tastaturen mit Zusammensetzung),
  Mausbewegung ohne Taste auf dem iPad (`UIHoverGestureRecognizer`), dauerhafte Freigaben gewählter Dateien über einen
  Neustart (Lesezeichen), Bildschirm anlassen, Textgröße des Systems, Accessibility. Das Bild geht bei jedem Frame
  ganz als Kopie an den Layer; mit Metal (AP6) entfällt das.

### AP6: Metal (L)

- `lib/gpu` mit einem Metal-Backend neben Vulkan und WebGPU, `CAMetalLayer` zum Anzeigen.
- `lib/wgsl` erzeugt MSL.

**Fertig, wenn:** `gputest` und `uigputest` auf dem Mac und dem iPhone grün sind.

### AP7: Desktop-Dienste auf macOS (M)

- Menüleiste oben, Dock, Statusmenü (Tray), Mitteilungen, Dateizuordnungen (`Info.plist`), Autostart
  (`SMAppService`), Ruhezustand verhindern, Drucken, App-Sandbox und die Rechte aus [PERMISSIONS.md](PERMISSIONS.md).

**Fertig, wenn:** die Desktop-Apps (`jfWord`, `BackupCraft`, JFPainter) auf dem Mac ohne fehlende Grundfunktionen
laufen.

**Aufteilung (2026-11-09).** Die Bestandsaufnahme nach AP4 ergab mehr als die Liste oben: Papierkorb, zuletzt
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

**AP7a umgesetzt (2026-11-09):** [`apple_mac_app.rs`](../lib/plat/apple_mac_app.rs) baut beim ersten Fenster das
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

**AP7b umgesetzt (2026-11-09):** [`apple_mac_services.rs`](../lib/plat/apple_mac_services.rs): das Statusmenü als
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

**AP7c umgesetzt (2026-11-09), damit ist AP7 fertig:** `build_rs.py` packt jedes Programm für `mac` zusätzlich als
`NAME.app` ([`tools/apple.py`](../tools/apple.py) `mac_bundle`). `Info.plist` kommt aus `PACKAGE`: Name, Version,
`app_id`, `filetypes=` als Dokumenttypen (eigene Typen auch als exportierte UTIs) und `schemes=` als URL-Typen. Das
Symbol entsteht als `NAME.icns` aus `res/app_*.png`, das Bündel wird ad hoc signiert. Mit `sandbox` in PERMISSIONS
schaltet es die App Sandbox ein, mit den Entitlements, die die Regeln brauchen: Netz, vom Nutzer gewählte Dateien,
Drucken, Standort, die Ordner Downloads, Bilder, Musik und Filme sowie befristete Ausnahmen für andere Pfade.
`cargo jflib mac [--run]` macht dasselbe für Apps außerhalb des Repos. `tools/apple.py gui` startet die Kopie im
Bündel, die damit eine Bundle-ID hat und Mitteilungen senden darf.

Geprüft auf dem Mac: `--test window` (77) aus dem Bündel, mit einer gesendeten Mitteilung und dem Klick darauf.
`jfWord` 51 von 53 Skripten (die zwei `rich_field`-Skripte scheitern im Worktree auch unter Windows und Linux),
`BackupCraft` 10 von 10, beide auch im echten Fenster. JFPainter mit `cargo jflib mac` gegen diesen JFLib-Stand (ein
lokaler `[patch.crates-io]` in der Mac-Kopie): 72 von 72 Skripten, die vier Plugin-Skripte übersprungen, dazu ein Lauf im
echten Fenster. Dabei behoben:

- **Schriften:** Die Ordner von macOS (`/System/Library/Fonts`, `/Library/Fonts`, `~/Library/Fonts`) fehlten,
  die Schriftliste war leer.
- **Drucken:** Ohne Fenster der App (headless) geht kein Druckdialog auf, das Ergebnis ist `ERR_UNSUPPORTED` wie
  unter Linux ohne Display.
- **`cargo jflib test`:** Eine Zeile `#!only windows linux` lässt ein Skript nur auf diesen Systemen laufen.
- **JFPainter** (eigenes Repo, Branch `claude/mac-ap7c`): Native Plugins nutzen die x86-64-Konvention `win64`.
  Der Plugin-Host läuft deshalb nur auf x86-64 unter Windows und Linux, anderswo gibt es nur die eingebauten Effekte.
  Die Systemdienste (Update-Prüfung, Scanner über SANE) laufen jetzt auch auf macOS.

Offen für AP8: `.dmg`, Developer-ID-Signatur und Notarisierung (dann mit Hardened Runtime), `SMAppService` für
Autostart in der Sandbox, eine Bildaufnahme über ImageCaptureCore statt SANE.

### AP8: Pakete, Updates, CI, Dokumentation (M)

- `.app` als `.dmg`, Signatur mit Developer ID und Notarisierung, Update-Kanal `macos-aarch64`, iOS über TestFlight,
  falls gewünscht.
- `.github/workflows/apple.yml` auf den macOS-Runnern von GitHub (kostenlos für öffentliche Repositories), mit
  Simulator, Start nur per Hand, bis der Lauf stabil ist.
- Erst jetzt README, [PACKAGING.md](PACKAGING.md), GUIDE und Website.

**Fertig, wenn:** ein signiertes und notarisiertes Programm auf einem fremden Mac ohne Warnung startet und sich
aktualisiert.

**Aufteilung (2026-11-09).** Der Nutzer tritt dem Apple Developer Program bei (Frage 3 unten); bis Zertifikat und
Zugang da sind, entsteht alles, was ohne sie geht. Dabei fiel auf: `update::target()` meldet auf dem Mac
`linux-aarch64`. AP8 läuft in vier Schritten:

- **AP8a, Pakete und Updates auf dem Mac:** `.dmg` (`hdiutil`, mit einer Verknüpfung auf `/Applications`) aus
  `tools/package.py --formats dmg` und `cargo jflib package --formats dmg`. `update::target()` meldet
  `macos-aarch64` und `ios-aarch64`. Eine neue Update-Art `app` ersetzt das ganze Bündel (die Signatur deckt das
  Bündel, nicht nur das Programm): neben das alte entpackt, die Signatur des neuen gegen die Anforderung des laufenden
  geprüft, getauscht, neu gestartet; Probe und Rückfall wie bei `exe`. Ein Programm an einem Ort ohne Schreibrecht
  oder von macOS verschoben (App Translocation) bekommt `ReadOnly`. `tools/package.py --update` schreibt den Kanal mit.
- **AP8b, CI:** `.github/workflows/apple.yml` auf den arm64-Runnern von GitHub, nur per Hand gestartet: Bau für
  `mac` und den Simulator, die Skripte der Gallery und von `androidGallery`, `coretest`, `plattest` ohne Fenster, ein
  `.dmg` als Artefakt.
- **AP8c, Developer ID und Notarisierung:** Signatur mit „Developer ID Application“, gehärteter Laufzeit und
  Zeitstempel, `notarytool` über ein Profil im Schlüsselbund, `stapler`. Abnahme: ein `.dmg` mit dem
  Quarantäne-Attribut eines Downloads startet ohne Warnung (`spctl --assess`) und aktualisiert sich. TestFlight für
  iOS danach, falls gewünscht.
- **AP8d, Dokumentation:** README, [PACKAGING.md](PACKAGING.md), GUIDE und die Website (deren Inhalt vorher mit dem
  Nutzer abgestimmt).

**AP8a umgesetzt (2026-11-09):** `tools/package.py --formats dmg` und `cargo jflib package --formats dmg` bauen auf dem
Mac je Stil ein `.dmg` (BackupCraft, JFPainter mit vier Stilen); `jfgui_MAC_IDENTITY` schaltet die Signatur von ad hoc
auf ein Zertifikat mit gehärteter Laufzeit und Zeitstempel um. Die Update-Art `app` ([UPDATES.md](UPDATES.md) „macOS
app bundles“) tauscht das Bündel als Ganzes; `updatetest --test app` prüft Einspielen, Signatur, Rückfall und die
Ablehnungen, `--test update` und `--test apply` laufen jetzt auch auf dem Mac. Dabei gefunden: Ein `exe`-Update verlor
auf dem Mac das Ausführrecht, und `cargo jflib keygen` baute sein Signierprogramm dort für Linux.

**AP8b umgesetzt (2026-11-09):** [`.github/workflows/apple.yml`](../.github/workflows/apple.yml), nur per Hand: `coretest`,
die Skripte von `gallery` und `updatetest`, `plattest` ohne Fenster (`all`, `cliprich`, `spell`), `androidGallery` im
Simulator und ein `.dmg` von BackupCraft als Artefakt. Auf dem Mac zu Hause liefen alle Schritte grün; der erste Lauf
auf GitHub folgt nach dem Push. `build_rs.py` startet Programme mit `ui` auf dem Mac über ihr Bündel (`tools/apple.py
gui`); updatetest nimmt dann eine Programmdatei außerhalb davon, sonst bekäme es nur `app`-Einträge. Der erste Lauf
auf GitHub (2026-11-10) lief in 14 Minuten grün durch.

**AP8c umgesetzt (2026-11-10):** Mit `jfgui_MAC_IDENTITY` („Developer ID Application: …“) und `jfgui_MAC_NOTARY`
(ein Profil von `xcrun notarytool store-credentials`) notarisieren `tools/package.py --formats dmg` und `cargo jflib
package --formats dmg` erst das Bündel, dann das Image, und heften beiden ihr Ticket an (`stapler`): Eine aus dem
Image gezogene Kopie startet auch offline. Über SSH ist der Schlüsselbund gesperrt; `tools/apple.py` lässt codesign
mit Zertifikat und notarytool dann in der angemeldeten Sitzung laufen (`keychain_run` über `gui_run`, ohne Fenster).
Die Einrichtung (Zertifikat, app-spezifisches Passwort, Profil) steht in [PACKAGING.md](PACKAGING.md) „macOS: .dmg“.
Abnahme mit BackupCraft auf dem Mac mini: Die `.dmg` mit dem Quarantäne-Attribut eines Downloads, die App darauf
und eine herauskopierte Kopie nimmt Gatekeeper an (`spctl --assess`: „accepted, source=Notarized Developer ID“),
`stapler validate` und `syspolicy_check distribution` sind zufrieden; die ad hoc signierte `.dmg` und App lehnt er
ab. `updatetest --test app` mit `jfgui_MAC_IDENTITY` aktualisiert Bündel mit der Developer ID und lehnt dieselbe
App ad hoc signiert ab. Die erste Notarisierung des neuen Kontos brauchte rund zwei Stunden, danach dauerten beide
Einreichungen zusammen gut eine Minute. Dabei gefunden: Das Programm eines Bündels mit Developer ID beendet macOS
sofort, wenn es außerhalb des Bündels läuft („invalid Info.plist“); updatetest nimmt für seine `exe`-Tests deshalb
das einfache Programm neben dem Bündel. Die Hand-Probe des Nutzers (die `.dmg` aus „Downloads“ geöffnet, die App
gestartet) fand einen Fehler, den `spctl` nicht sieht: `tools/package.py` kopierte das Programm ohne Ausführrecht,
die App öffnete nicht; behoben, `mac_dmg` lehnt ein solches Bündel jetzt ab, danach startete sie mit der üblichen
Rückfrage. Offen: Signieren in der CI (Zertifikat als Geheimnis) und TestFlight für iOS (falls gewünscht).

**AP8d umgesetzt (2026-11-10), damit ist AP8 fertig:** README, [PACKAGING.md](PACKAGING.md), [UPDATES.md](UPDATES.md)
und GUIDE nennen macOS und iOS (mit AP8a–AP8c). Die Website jflib.org (Repository jflibWebsite, Inhalt vorher mit dem
Nutzer abgestimmt) führt beide als native Ziele auf der Startseite, unter Features, im Vergleich und in der Roadmap
(dort „Fertig“; als Nächstes Metal und VoiceOver), in allen fünf Sprachen; Docs und Galerie kommen aus `8c94d080`.

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
- **UIKit ohne Szene geht nicht mehr:** Mit dem SDK von iOS 27 braucht eine App `UIApplicationSceneManifest` im
  `Info.plist` und einen Szenen-Delegate, der das Fenster mit `initWithWindowScene:` anlegt.
- **`simctl spawn` startet im Datenordner des Simulators** (`…/Devices/UDID/data`) und hat keine Shell, also auch
  kein `cd`. `tools/apple.py sim` gibt den Ordner als `jfgui_CWD` mit (`SIMCTL_CHILD_`), `main` wechselt dorthin.
- **Die Zwischenablage liest nur eine App:** Ein mit `simctl spawn` gestarteter Prozess kann `UIPasteboard` setzen,
  liest aber nichts zurück. Als App (`simctl launch`) geht beides; Inhalte anderer Apps fragt iOS beim Lesen nach.
- **Core Motion zählt anders:** Beschleunigung und Schwerkraft in g und mit umgekehrtem Vorzeichen (Bildschirm nach
  oben: z = -1), die Bibliothek will m/s² mit +9,81. Ob die Rotationsmatrix der Lage vom Bezug zum Gerät abbildet
  oder umgekehrt, entscheidet `apple_ios_motion.rs` an Schwerkraft und Magnetfeld derselben Probe.
- **`wake()` unter dem Host:** Das Wecken ist über ein Flag zusammengefasst, das nur die Warteschleife eines
  Programms zurücksetzt (`wake_consume`). Unter UIKit wartet niemand darauf: Das Flag blieb nach dem ersten Wecken
  stehen, jedes weitere ging verloren (die Sensoren-Seiten froren beim ersten Messwert ein). Mit dem Host geht
  `wake()` deshalb direkt an ihn (`apple_ios.rs`).
- **Ohne Nutzungstext fragt iOS nie:** `requestWhenInUseAuthorization` tut ohne
  `NSLocationWhenInUseUsageDescription` im `Info.plist` nichts.
- **Zwei Bildschirme:** Der Simulator „iPhone Duo“ hat zwei Displays, `simctl io screenshot` will dann `--display`.
  `tools/apple.py` nimmt ihn nicht von selbst.

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
3. **Apple Developer Program** (99 € im Jahr) oder die kostenlose Apple-ID? Bis AP7 die kostenlose (AP0): Apps laufen
   auf dem iPhone nur sieben Tage, ohne TestFlight und ohne Notarisierung. Für AP8 tritt der Nutzer bei (2026-11-09);
   Mitglied seit 2026-11-10 (Team 6RBCBNA6DJ), Developer ID und Notarisierung laufen (AP8c).
4. **Welche Apps zuerst?** Vorschlag: `gallery`, dann JFPainter und `jfWord`; auf dem iPhone die Gallery, dann
   `Kritzel`.
5. **App Store** als späteres Ziel für iOS? Das ändert Sandbox, Plugins und die Prüfregeln.
