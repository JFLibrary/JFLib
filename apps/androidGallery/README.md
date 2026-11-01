# jfgui Pocket Gallery für Android

Kleine native Test-App mit den vorhandenen jfguiLib-Widgets: Buttons mit Klickzähler,
Checkbox, Schalter, Slider, Fortschrittsanzeige, Akzentfarben, hellem/dunklem Design,
Unicode-Textfeld und einer Karte „Dateien & Zwischenablage“ (Öffnen, Speichern unter, Ordner
über den System-Picker, eine Notiz, die einen Neustart überlebt, ein Startzähler, Kopieren und
Einfügen über die System-Zwischenablage; [`files.rs`](files.rs)) sowie „Sicherer Speicher“
(ein Wert im Android Keystore bzw. im Browser-Speicher von `plat::secret`; [`secrets.rs`](secrets.rs))
und „System“ (Einstellungen des Systems live, eine Benachrichtigung, Bildschirm anlassen,
Lebenszyklus; [`system.rs`](system.rs)). Vertikal wischen scrollt die
Seite; horizontal ziehen bewegt den Slider; langes Drücken ist die rechte Taste.

Hinter der Startseite (die Karte „07 Weitere Seiten“ am Ende, das Menü oben rechts; die Zurück-Taste
führt zurück) liegt der technische Umfang der großen Galerie [`apps/gallery`](../gallery/main.rs),
für den Finger angeordnet (eine Spalte, 44 DIP):

| Seite | Inhalt | Datei |
| --- | --- | --- |
| Schaltflächen | Schaltflächen, Werkzeugleiste, Checkboxen, Radio, Schalter, Segmente, Regler (auch senkrecht), Karte | [`pg_basic.rs`](pg_basic.rs) |
| Menüs & Auswahl | Menüleiste mit Untermenüs und zuletzt geöffneten Dateien, Auswahlliste, Kalender, Aufklapp-, Teil- und Befehlsknöpfe, Kontextmenü, Hinweise | [`pg_menus.rs`](pg_menus.rs) |
| Texteingabe | Textfelder, Kennwort, Zahlenfelder, mehrzeilig, Formular im Raster, Umbruch und Kürzung | [`pg_text.rs`](pg_text.rs) |
| Listen & Bäume | 200 Einträge virtualisiert, Zeilen zum Ziehen mit Griff, Baum mit Checkboxen, Zeichenfläche mit `canvas_damage` | [`pg_lists.rs`](pg_lists.rs) |
| Container | Tabs, Expander, Hinweisleisten, kurze Meldungen, waagerechtes Rollen, Rollleiste | [`pg_boxes.rs`](pg_boxes.rs) |
| Werte & Farben | Dezimalregler, Eigenschaftszeilen, Winkel, Verschieben, Farbspuren, Farbrad, Farbwähler | [`pg_colors.rs`](pg_colors.rs) |
| Fenster & Dialoge | Werkzeugfenster, verschachtelte modale Dialogfenster, Seitendialog, Popups aus einem Fenster, Statusleiste | [`pg_windows.rs`](pg_windows.rs) |
| Randfälle | Bedienelemente in den Ecken: ihre Menüs, Listen und der Kalender bleiben auf dem Bildschirm | [`pg_edge.rs`](pg_edge.rs) |
| Wasserwaage, Handylage, Kompass, GPS | die Sensoren und der Standort des Geräts (`plat::sensors`), unten | [`pg_sensors.rs`](pg_sensors.rs) |

Die App braucht `POST_NOTIFICATIONS` (gefragt bei der ersten Benachrichtigung), `WAKE_LOCK`
(Bildschirm anlassen) und `ACCESS_FINE_LOCATION` / `ACCESS_COARSE_LOCATION` (gefragt, wenn der
Schalter der GPS-Seite angeht), keinen Netzwerkzugang: Dokumente anderer Apps erreicht sie nur über
die Freigabe im Picker. Eine `PERMISSIONS`-Datei hat die Galerie nicht: sie zeigt alles.

## APK bauen

Vom Repository-Verzeichnis:

```powershell
rustup target add aarch64-linux-android
python RustLib/build_rs.py app androidGallery --target android --out build/android-work
```

Ergebnis: `RustLib/build/android-work/android/androidGallery-arm64-v8a-debug.apk`.
Die APK enthält echtes AArch64-Maschinenprogramm in `lib/arm64-v8a/libjfgui_android.so`.
Sie ist für **Android 8.0 / API 26 oder neuer mit ARM64-Unterstützung** gedacht. 32-Bit-ARM
ist nicht enthalten. Der Build richtet ELF-Segmente und unkomprimierte APK-Bibliotheken
auf 16 KiB aus und prüft die APK-Signatur sowie die ZIP-Ausrichtung.

Voraussetzungen: Rust, Python, JDK 17+, Android SDK Platform 36, Build-Tools 35+ und NDK
r29. `ANDROID_SDK_ROOT`/`ANDROID_HOME` und `ANDROID_NDK_HOME` können die automatisch
erkannten SDK-/NDK-Pfade überschreiben. Mit der neuen Android CLI lassen sich die
benötigten Pakete beispielsweise mit `android sdk install ndk/29.0.14206865
platforms/android-36 build-tools/36.0.0` installieren. Es werden weder Cargo-Abhängigkeiten
noch Gradle benötigt. `--release` optimiert den Rust-Code und signiert mit dem
Release-Schlüssel aus der Umgebung (`jfgui_ANDROID_KEYSTORE`, `jfgui_ANDROID_KEYSTORE_PASS`,
`jfgui_ANDROID_KEY_ALIAS`, ggf. `jfgui_ANDROID_KEY_PASS`; Ergebnis
`androidGallery-arm64-v8a-release.apk`). Fehlen sie, bricht der Build ab und nennt sie;
`jfgui_ANDROID_DEBUG_SIGN=1` nimmt für Testbuilds den lokalen Debug-Schlüssel (siehe
`docs/PACKAGING.md`, Abschnitt „Signing“).

Installation, wenn gewünscht:

```powershell
adb -s DEVICE_SERIAL install -r RustLib/build/android-work/android/androidGallery-arm64-v8a-debug.apk
adb -s DEVICE_SERIAL shell am start -n org.jflib.gallery/.GalleryActivity
```

Für einen x86_64-Emulator:

```powershell
rustup target add x86_64-linux-android
python RustLib/build_rs.py app androidGallery --target android --android-abi x86_64 --out build/android-work
```

## Was bereits portiert ist

- `rt` verwendet unter Android Bionic für ausgerichtete Allokationen und Logcat für
  Diagnoseausgaben. Es gibt keine feste Annahme über die Speicher-Seitengröße.
- ARM64 verwendet die vorhandenen portablen Float-/Math- und skalaren SIMD-Implementierungen.
  Spezifische NEON-Kernel sind noch nicht umgesetzt.
- Der bestehende Host-Adapter in `lib/plat/web*.rs` wird für Android nativ kompiliert:
  Ereignisse, Fenster-Komposition und der besitzende UI-Laufzeitkern sind derselbe Code.
- `android/bridge.c` kopiert die geänderten Zeilen (schon R, G, B, A; bei einem einzelnen
  Fenster ohne Popup direkt aus dessen Pixeln, ohne Komposition) in ein Android-Bitmap. Die schmale Java-Hülle
  kümmert sich um Lebenszyklus, Insets, Touch, Hardwaretastatur und IME-Eingabe.
  Widgets, Layout, Schriften und Rendering kommen vollständig aus der Rust-Library.
- Die Java-Hülle zeichnet nur geänderte Bitmaps. JNI sperrt und kopiert ausschließlich
  bei gemeldeten Änderungen; im Leerlauf plant die Library den nächsten Timer statt
  ständig GPU-Uploads auszulösen. Neue Eingaben wecken den Frame-Callback sofort.
- `GalleryInputConnection` spiegelt den fokussierten Rust-Editor als echtes Android-
  `Editable`. Composing-Text erscheint bereits vor `commitText`; Textauswahl, Löschen,
  UTF-16/UTF-8-Positionen und die tatsächlich akzeptierte Textlänge bleiben synchron.
- Der vorhandene Build-Einstieg unterstützt das explizite Ziel `--target android`.
  Weitere Apps müssen Android in `TARGETS` aufnehmen und ein Manifest/Ressourcen ergänzen.

## Dateien, Dokumente und Zwischenablage (seit 20.10.2026)

- **Interner Speicher**: `config://`, `data://`, `cache://`, `logs://` und `documents://` sind
  echte Dateien unter `filesDir/fs` bzw. `cacheDir` (`lib/plat/android_files.rs`) und
  überleben Neustarts. `write_file` schreibt atomar (Geschwisterdatei, fsync, rename),
  `/tmp` wird beim Start geleert. `GalleryActivity` übergibt die Ordner vor dem Start
  (`jfgui_ANDROID_FILES`, `jfgui_ANDROID_CACHE`).
- **Dateiauswahl**: `ui.filedlg` öffnet das Storage Access Framework (`FD_OPEN`, `FD_SAVE`,
  `FD_FOLDER`); das Ergebnis ist ein `content://`-Name, den alle Dateifunktionen über den
  `ContentResolver` lesen und schreiben, auch nach einem Neustart (dauerhafte Freigabe).
  Rust fragt Java synchron über `jfgui_host_call` → `GalleryHost.call` (JNI, eigener
  Antwortpuffer), Picker-Anfragen gehen über `jfgui_host_request` → `GalleryHost.request`,
  die Antwort kommt über `GalleryHost.nativeResult` zurück ([`../../android/GalleryHost.java`](../../android/GalleryHost.java)).
  Seit 22.10.2026 schreibt `content://` über ein Journal im Datenordner der App (Kopie,
  Schreiben an Ort und Stelle, Zurücklesen; ein unterbrochener Vorgang wird beim nächsten
  Start vollendet), Namen in gewählten Ordnern werden zwischengespeichert, und ein
  `ContentObserver` meldet Änderungen des Anbieters (`GalleryHost.nativeChanged`).
- **Zuletzt geöffnet**: Öffnen und Speichern kommen in die Liste von `ui.recent_add`; „Zuletzt
  öffnen“ liest die erste Datei wieder (Android über die dauerhafte Freigabe; im Browser die
  Datei des Nutzers über ihr Handle, nach dem Neuladen per `plat::web::reopen`).
- **Browser** (Chromium): Öffnen und Speichern zeigen die Picker des Systems (File System
  Access API); gespeichert wird in die gewählte Datei des Nutzers zurück. Firefox nimmt den
  eigenen Dialog über die virtuellen Dateien. Geprüft mit `tests/files_web.mjs`.
- **Zwischenablage**: Text und HTML über den `ClipboardManager`; Bilder als PNG über den
  Provider `GalleryFiles` (im Manifest, Authority `org.jflib.gallery.jfgui.files`).

## Grenzen dieses Prototyps

Der Adapter bietet eine einzelne Android-Activity und darin die Library-Oberflächen.
Drucken, native Plugins, Hintergrundthreads, Papierkorb und `plat::fs` sind nicht
angeschlossen; nicht verfügbare Host-Anfragen enden mit `Unsupported`. `content://` schreibt
an Ort und Stelle (SAF kann kein Dokument durch Umbenennen ersetzen; das Journal vollendet
einen abgebrochenen Vorgang erst beim nächsten Start, andere Apps sehen währenddessen einen
Teil), und die Zwischenablage lässt sich nur lesen, solange die App den Fensterfokus hat
(direkt nach dem Picker kurz nicht).

Barrierefreiheit (seit 19.10.2026): `android/GalleryAccessibility.java` stellt jedes Widget
als virtuelle View eines `AccessibilityNodeProvider` dar (Klasse, Text, Zustände, Werte,
Auswahl, Rechteck), nimmt die Aktionen von TalkBack entgegen (Klick, Fokus, Wert, Text,
Auswahl, Explore-by-Touch) und sendet die Ereignisse; eingeschaltet wird der Spiegel mit dem
Accessibility-Schalter des Systems (`docs/GUIDE.md`, Abschnitt 19). Im Browser derselben App
liegt ein ARIA-Schattenbaum über dem Canvas (`web/a11y.js`). Die Abschnittstitel („01
Interaktion“ …) sind Überschriften (`ui.heading`, seit 22.10.2026): TalkBack und die
Überschriften-Navigation der Screenreader im Browser springen von Abschnitt zu Abschnitt.
Die IME-Anbindung unterstützt Composing-Text, Commit, Auswahl, umgebenden Text und
Korrekturen. Composing-Unterstreichungen, Android-Auswahlgriffe und weitergehende
Editor-/Tastaturvarianten sind noch nicht umgesetzt. Android zeichnet die Oberfläche
auf seinem UI-Thread; für große Apps sollte Rendering in einen eigenen Thread wechseln.

Diese APK belegt den ersten nativen Android-Port der Rust-Library. Sie bedeutet noch
keine vollständige Android-Unterstützung aller Desktop-Apps und Betriebssystemdienste.
Die eingefrorene x86-Assembler-Referenz in `legacy/` bleibt unverändert.

## Prüfungen

Die gleiche Galerie kann auf dem Desktop über die vorhandene Skriptsprache geprüft werden:

```powershell
python RustLib/build_rs.py app androidGallery --target win --out build/android-work
cd RustLib
build/android-work/win/androidGallery.exe --script apps/androidGallery/tests/basic.uis --size 390x1100 --theme light --anim off --verify-dirty
```

Das Skript prüft Klickzähler/Reset, Checkbox, deaktivierten Slider, Sliderwerte,
Farbauswahl, Theme-Wechsel, Texteingabe und die Sichtbarkeit des fokussierten Textfelds
bei verkleinerter Fensterhöhe. Zusätzlich muss die native APK auf Android
gestartet werden; ein erfolgreicher Desktop-Test ersetzt den Android-Laufzeittest nicht.

Der gemeinsame Host lässt sich außerdem als WASM-Modul prüfen:

```powershell
python RustLib/build_rs.py app androidGallery --target web --web-mode scalar --out build/android-work
cd RustLib
node apps/androidGallery/tests/host.mjs build/android-work
```

Am 12.10.2026 geprüft: ARM64-APK auf einem Samsung SM-A566B mit Android 16 / API 36
installiert und gestartet; Darstellung und Widget-Bedienung auf dem Gerät beobachtet.
Auf einem x86_64-Emulator mit Android 15 / API 35 wurden zusätzlich Touch, Scrollen,
Slider, Theme-Wechsel, Bildschirmtastatur, Texteingabe, Drehung und Rückkehr aus dem
Hintergrund geprüft, einschließlich einer Wischgeste direkt nach dem Kaltstart.
Desktop-Skripttest mit `--verify-dirty` und WASM-Host-Test bestanden. Die Build-Pipeline
prüft ELF-Architektur, 16-KiB-Ausrichtung und APK-Signatur. Ein Gerät mit 16-KiB-Seiten
und ältere Android-Versionen wurden nicht zur Laufzeit getestet.

## Barrierefreiheit prüfen

Android (Emulator oder Gerät, Debug-APK):

```powershell
adb -s DEVICE_SERIAL shell am instrument -w -r org.jflib.gallery/.GalleryA11yInstrumentation
```

Der Test verbindet sich über `UiAutomation` – die Client-Seite, die auch TalkBack benutzt –,
liest Klassen, Texte, Überschriften, Zustände, Wertebereiche, Rechtecke und Auswahl der
virtuellen Views, klickt, setzt Fortschritt, Text und Auswahl, verschiebt den
Accessibility-Fokus und prüft die Ereignisse sowie Explore-by-Touch. Am 19.10.2026 auf einem
x86_64-Emulator mit Android 15 / API 35: 30 Prüfungen bestanden; `GalleryInstrumentation` (78)
weiter grün. Am 22.10.2026 mit der Überschrift (`isHeading()`) 31 Prüfungen bestanden,
`GalleryInstrumentation` (78) weiter grün. TalkBack selbst war auf dem Image nicht installiert
und wurde nicht ausprobiert.

Browser (Playwright über `PLAYWRIGHT_MODULE` oder die Node-Modulsuche; Chromium aus
`CHROME_PATH`, sonst das installierte Chrome, sonst Playwrights Chromium, siehe
`tools/tests/playwright.mjs`; `--channel=msedge` nimmt stattdessen Edge):

```powershell
python RustLib/build_rs.py app androidGallery --target web
node RustLib/apps/androidGallery/tests/a11y_web.mjs RustLib/build
```

36 Prüfungen (seit 22.10.2026): Chromiums eigener Accessibility-Baum (DevTools-Protokoll,
auch die Abschnittstitel als Überschriften der Ebene 2), Klicks wie von einem Screenreader,
Slider, Tab-Fokus, Tippen, Auswahl, Ersetzung, Zeiger, `?a11y=0`.

## Dateien, Picker und Zwischenablage prüfen

Auf einem Emulator oder Gerät mit der Debug-APK (vom Repository-Verzeichnis):

```powershell
python apps/androidGallery/tests/android_files.py --serial emulator-5554 --apk build/android/androidGallery-x86_64-debug.apk
```

Das Skript legt `/sdcard/Documents/jfgui-test/hello.txt` an, löscht die App-Daten (und damit
alte Freigaben) und startet `GalleryFilesInstrumentation` zweimal mit einem `force-stop`
dazwischen. Phase 1 startet den Selbsttest der App (`jfgui_SELFTEST=phase1`): interner
Speicher (Schreiben, Ersetzen, Umbenennen, Ordner, Streams, `AtomicFile`, Sperren, `persist`),
Zwischenablage in beide Richtungen (Text, HTML, eigene RTF), dann drei Picker, die die
Instrumentation über UiAutomation wie ein Mensch bedient (Ordner `jfgui-test` wählen und
erlauben, `hello.txt` antippen, Speichern) mit Prüfungen im gewählten Ordner (Lesen,
Anlegen mit Unterordner, Ersetzen, Listen, Anzeigenamen, `AtomicFile`, Streams, Umbenennen
auch über eine vorhandene Datei, Verschieben, Löschen), zuletzt ein Bild in der
Zwischenablage, das die Instrumentation wie eine fremde App liest. Phase 2 (neuer Prozess)
prüft Dateien, Startzähler und Freigaben nach dem Neustart, gibt eine Freigabe zurück und
fügt mit Strg+V Systemtext in die Notiz ein bzw. kopiert ihn mit Strg+A, Strg+C zurück.

Am 20.10.2026 auf einem x86_64-Emulator mit Android 15 / API 35 (AVD aus dem vorhandenen
Image `system-images;android-35;default;x86_64`): Phase 1 76 Selbsttest- und 8
Instrumentation-Prüfungen, Phase 2 15 + 6, alle bestanden; `GalleryInstrumentation` (78) und
`GalleryA11yInstrumentation` (30) weiter grün. Auf dem Desktop prüft
`build/win/plattest.exe --test content` (55 Prüfungen) dieselbe Namenslogik gegen einen
SAF-Anbieter im Speicher, `tests/folder.uis` den Ordnermodus des eigenen Dialogs. Ein echtes
Gerät und Cloud-Anbieter wurden nicht geprüft.

Seit 22.10.2026 prüft Phase 1 zusätzlich das Überschreiben vorhandener Dokumente (dieselbe
Dokument-URI, keine Journal-Kopie übrig), den Namens-Cache (bekannte Namen ohne
Ordnerauflistung), eine gemeldete Änderung (die Instrumentation ruft `notifyChange` für den
gewählten Ordner, der Zähler des `ContentObserver` steigt) und einen nach sechs Bytes
unterbrochenen Schreibvorgang (Teil im Dokument, Kopie im Journal); Phase 2 findet ihn beim
ersten Zugriff auf `content://` vollendet. Das Skript sieht nach jeder Phase von außen nach
(`cat` des Dokuments, `run-as … ls` des Journal-Ordners). Am 22.10.2026 auf dem Emulator
(API 35): Phase 1 86 + 8, Phase 2 16 + 6, alle bestanden; `GalleryInstrumentation` (78) und
`GalleryA11yInstrumentation` (31) weiter grün; `plattest --test content` 79 Prüfungen (Cache,
Journal, Wiederherstellung, ein Anbieter, der Bytes verliert).

Im Browser (Chromium und Firefox, vom Repository-Verzeichnis):

```powershell
python build_rs.py app androidGallery --target web
node apps/androidGallery/tests/files_web.mjs build
```

Chromium: die Picker liefern Dateien des Origin Private File System (die echten lassen sich
nicht fernsteuern); Öffnen (Handle in IndexedDB, Pfad `/handles/ID/brief.txt`), Speichern
unter, Speichern über die geöffnete Datei (gleicher Pfad), ein fehlschlagender Schreibvorgang
(Datei unverändert), Neuladen (Datei schon gelesen; mit „prompt“: `reopen` erlaubt und
verweigert), abgebrochener Picker. Firefox: der eigene Dialog, keine Handle-Datenbank. Am
22.10.2026: 25 Prüfungen bestanden; `secrets_web`, `text_web`, `a11y_web` weiter grün.

## Sicherer Speicher prüfen (seit 21.10.2026)

Die Karte „05 Sicherer Speicher“ sichert den Inhalt ihres Feldes mit `plat::secret`
(Dienst `galerie`, Konto `demo`), leert das Feld und lädt oder löscht den Wert wieder. Auf
Android liegt er verschlüsselt mit einem Schlüssel des Android Keystore unter `no_backup`, im
Browser verschlüsselt mit einem nicht exportierbaren WebCrypto-Schlüssel in IndexedDB
(`docs/SECRETS.md`).

```powershell
python apps/androidGallery/tests/android_secrets.py --serial emulator-5554 --apk build/android/androidGallery-x86_64-debug.apk
python build_rs.py app androidGallery --target web
node apps/androidGallery/tests/secrets_web.mjs build
```

Das Android-Skript löscht die App-Daten und startet `GallerySecretsInstrumentation` zweimal
mit einem `force-stop` dazwischen. Phase 1 startet den Selbsttest der App
(`jfgui_SELFTEST=secrets1`: Setzen, Lesen, Überschreiben, Löschen, Namen, eine andere App-ID,
5000 / 40000 / 65536 Bytes) und prüft danach von außen den Schlüssel (AES, 256 Bit, GCM, nicht
exportierbar, ohne Nutzerauthentifizierung) und die drei verschlüsselten Dateien (kein Wert im
Klartext in irgendeiner Datei der App); eine Datei wird beschädigt. Der Shell-Nutzer darf den
Ordner nicht einmal auflisten. Phase 2 (neuer Prozess) prüft die Werte nach dem Neustart, die
abgewiesene beschädigte Datei und löscht alles. Der Browsertest bedient die Karte über den
ARIA-Schattenbaum in Chromium und Firefox (Sichern, Laden, Überschreiben, Neu laden, 5200
Bytes, Löschen), liest IndexedDB wie ein anderes Skript des Origins (nur Chiffretext, Schlüssel
nicht exportierbar) und prüft eine Seite ohne sicheren Kontext (kein Speicher, Knöpfe aus).

Am 21.10.2026 auf einem x86_64-Emulator mit Android 15 / API 35: Phase 1 33 Selbsttest- und
22 Instrumentation-Prüfungen, Phase 2 9 + 3, alle bestanden (Schlüssel dort nur in Software,
Sicherheitsstufe 0); `android_files.py`, `GalleryInstrumentation` (78) und
`GalleryA11yInstrumentation` (30) weiter grün. Browser: 56 Prüfungen in Chromium und Firefox,
28 in Chromium mit `--threads`; `a11y_web.mjs` (33) weiter grün.

## System prüfen (seit 01.11.2026)

Die Karte „06 System“ zeigt, was das System der App sagt (Design, Kontrast, Bewegung,
Textgröße, Akzent, die `UIC_SETTINGS` mit ihren Bits), schickt eine Benachrichtigung
(`ui.notify`; Klick: `UIC_NOTE`, weg: `UIC_NOTE_GONE`), hält mit „Bildschirm anlassen“
`plat::inhibit(INHIBIT_IDLE)` und zählt den Lebenszyklus (`UIC_ACTIVE`, `UIC_SUSPEND` 2 / 0).
„Design des Systems“ lässt das Design dem System folgen (sonst bleibt die Galerie hell).

```powershell
python apps/androidGallery/tests/android_system.py --serial SERIAL [--apk build/android/androidGallery-x86_64-debug.apk]
python build_rs.py app androidGallery --target web
node apps/androidGallery/tests/system_web.mjs build [--threads]
```

Das Android-Skript startet die Galerie mit `jfgui_SELFTEST=system` (die Karte oben, ihr Zustand
in `data://system-state.txt`, gelesen mit `run-as`), tippt ihre Knöpfe über `uiautomator dump`
an und ändert das System wie ein Nutzer: `cmd uimode night yes|no`, `font_scale 1.3`,
`animator_duration_scale 0`, `high_text_contrast_enabled 1`; Benachrichtigung abgelehnt und
erlaubt im Dialog des Systems, angetippt in der Leiste (die Galerie kommt nach vorn), weggewischt,
dauerhaft verweigert (`pm revoke`, `user-fixed`); `FLAG_KEEP_SCREEN_ON` (`dumpsys window`) und
der WakeLock (`dumpsys power`); `KEYCODE_HOME` und Neustart aus dem Launcher. Am Ende stellt es
jede geänderte Einstellung wieder her. Ohne `--apk` installiert es
`build/android/androidGallery-<ro.product.cpu.abi>-debug.apk`, falls vorhanden; es läuft so auch
auf einem Telefon (entsperrt). Der Browsertest emuliert die Media Queries in Chromium und Firefox,
klickt eine echte Notification an, prüft Erlaubnis erteilt / noch nicht gefragt / verweigert,
den Screen Wake Lock über einen versteckten Tab hinweg und `UIC_SUSPEND`.

Am 01.11.2026 auf einem x86_64-Emulator mit Android 15 / API 35: 37 Prüfungen bestanden
(das Wegwischen nimmt SystemUI im Emulator nicht immer als Geste an; dann meldet das Skript es
als nicht geprüft). Browser: 45 Prüfungen in Chromium, 41 in Firefox, mit `--threads` 86.

## Seiten, Sensoren und Standort prüfen (seit 01.11.2026)

**Wasserwaage**: flach eine runde Libelle, auf der Kante ein Rohr, grün innerhalb von 0,5°.
**Handylage**: Nicken, Rollen, Gieren und das Telefon gezeichnet, alle fünf Sensoren mit ihren
Werten. **Kompass**: die Richtung auf einer Skala, die sich dagegen dreht; ohne Magnetfeld relativ
zum Start, und das steht dann da. **GPS**: Breite, Länge, Genauigkeit, Höhe, Tempo, Richtung, Alter,
Quelle; keine Karte, kein Netzwerkzugriff der App. Eine Seite startet nur, was sie braucht, und
stoppt es beim Verlassen; im Hintergrund stoppt die Library alles (`UIC_SUSPEND`). Der Standort
startet erst mit dem Schalter, denn der darf fragen. API: [`docs/GUIDE.md`](../../docs/GUIDE.md) §36.

```powershell
# überall, ohne Hardware: Lesewerte aus dem Skript (sensor ..., location ...)
python build_rs.py test androidGallery
# Browser
python build_rs.py app androidGallery --target web
node apps/androidGallery/tests/sensors_web.mjs build --engines=chromium,firefox [--threads]
# Emulator oder Telefon
python build_rs.py app androidGallery --target android --android-abi x86_64
python apps/androidGallery/tests/android_sensors.py --serial SERIAL [--apk build/android/androidGallery-x86_64-debug.apk]
```

- `tests/pages.uis` geht mit dem Finger durch alle Seiten und ihre Widgets (mit Shots unter
  `build/shots/androidGallery/`), `tests/level.uis`, `attitude.uis`, `compass.uis`, `gps.uis` prüfen
  die Sensorseiten mit Lesewerten aus dem Skript: Rechnung, Zustände, was läuft und was nicht, den
  Hintergrund, Ablehnung, abgeschaltetes System.
- `tests/sensors_web.mjs`: `setGeolocation` und Berechtigung (auch entzogen und wieder erteilt),
  Chromiums virtuelle Sensoren über CDP, in beiden Browsern `DeviceMotionEvent` und
  `DeviceOrientationEvent` als Objekte, Safaris `requestPermission` als Stellvertreter.
- `tests/android_sensors.py` startet die Galerie mit `jfgui_SELFTEST=sensors` und `jfgui_PAGE=...`
  (ihr Zustand in `data://sensors-state.txt`), tippt wie ein Nutzer und bedient den echten Dialog
  des Systems. Auf dem Emulator setzt es Werte mit `adb emu sensor set` und `adb emu geo fix` und
  vergleicht; auf einem Telefon sagt es, was zu tun ist („Telefon flach und still auf den Tisch
  legen“), prüft auf Plausibilität (Schwerkraft etwa 9,8 m/s², Richtung 0 bis 360), wartet bis zu 60 s auf eine
  Position und meldet sonst „no fix (indoors?)“, ohne dass das ein Fehler wäre. Am Ende stellt es
  alles wieder her: beide Standortrechte samt Merkern, den Standortschalter des Systems, die
  Sensorwerte des Emulators. Das Telefon muss entsperrt sein.

Am 01.11.2026: auf einem x86_64-Emulator mit Android 15 / API 35 54 Prüfungen, Browser 104 Prüfungen
in Chromium und Firefox. Auf einem echten Telefon lief das Skript noch nicht.

Offen: Im Browser auf dem Telefon rollt der Finger die Seite noch nicht und langes Drücken ist keine
rechte Taste (der Android-Host macht beides selbst); Hinweise (Tooltips) gibt es nur mit Maus oder
Stift; die Zeilen der Menüleiste sind Desktop-hoch; iOS (das Vorzeichen seiner Beschleunigung, die
Geste für `requestPermission`) ist nicht auf einem Gerät gelaufen.

## Regressionstest für Android-Eingabe und Leerlauf

Die Debug-APK enthält einen explizit startbaren Instrumentation-Test. Er startet die
Galerie und verändert ausschließlich ihren Testtext:

```powershell
adb -s DEVICE_SERIAL shell am instrument -w -r org.jflib.gallery/.GalleryInstrumentation
```

78 Prüfungen auf dem SM-A566B bestanden: sichtbare Komposition vor Commit, Ersetzung
und Abschluss ohne doppelte Zeichen, Umlaute, Emoji, Löschen nach Codepoints, Auswahl,
Korrektur eines Wortes, Batch-Edits, Textkapazität, veraltete Eingabesitzungen und
ausbleibende Bitmap-Updates bei unverändertem Inhalt. Der Test misst zusätzlich 60
Kompositionsänderungen einschließlich JNI, Rust-Layout und nativem Rasterisieren:
Median 0,84 ms, P95 1,75 ms, Maximum 9,45 ms. Das ist keine vollständige Messung vom
Fingerkontakt bis zur Ausgabe durch das Android-Display.

Bei der Kontrolle des ruhenden Vordergrundprozesses sank die beobachtete CPU-Auslastung
von etwa 68 % auf 0–1 %. In 32 Sekunden ohne Eingabe wurden keine neuen UI-Frames
gezeichnet. Die sichtbare Eingabe wurde zusätzlich mit der Samsung-Tastatur geprüft.

## Darstellung messen (seit 22.10.2026)

```powershell
adb -s DEVICE_SERIAL shell am instrument -w -r org.jflib.gallery/.GalleryPresentInstrumentation
```

Misst 30 ganze Bilder (`nativeRender` mit neuer Fläche: alles wird neu gezeigt, nichts neu
gerechnet) und 30 Bilder mit Zeigerbewegungen, prüft, dass die Bitmap nach Teilbildern einer
vollen Darstellung gleicht, und meldet einen Hash der Bitmap (gleich in jedem Build, der
dieselben Pixel zeigt). x86_64-Emulator (Android 15, 1080 × 2400, Dichte 420, Bitmap
823 × 1635): ein ganzes Bild 4,0–4,9 ms vorher, 0,8–1,5 ms mit RGBA-Fensterpixeln ohne
Komposition, gleicher Hash. Eine native Auflösung (ohne die Skala ≤ 2/density) kostete beim
Zeigen etwa 1,7 ms je ganzem Bild; die Grenze setzt jetzt das Rechnen der Bilder.
