# jfgui Pocket Gallery für Android

Kleine native Test-App mit den vorhandenen jfguiLib-Widgets: Buttons mit Klickzähler,
Checkbox, Schalter, Slider, Fortschrittsanzeige, Akzentfarben, hellem/dunklem Design,
Unicode-Textfeld und einer Karte „Dateien & Zwischenablage“ (Öffnen, Speichern unter, Ordner
über den System-Picker, eine Notiz, die einen Neustart überlebt, ein Startzähler, Kopieren und
Einfügen über die System-Zwischenablage; [`files.rs`](files.rs)). Vertikal wischen scrollt die
Seite; horizontal ziehen bewegt den Slider. Die App benötigt keine Berechtigungen und keinen
Netzwerkzugang: Dokumente anderer Apps erreicht sie nur über die Freigabe im Picker.

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
- `android/bridge.c` überträgt die BGRA-Pixel in ein Android-Bitmap. Die schmale Java-Hülle
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
- **Zwischenablage**: Text und HTML über den `ClipboardManager`; Bilder als PNG über den
  Provider `GalleryFiles` (im Manifest, Authority `org.jflib.gallery.jfgui.files`).

## Grenzen dieses Prototyps

Der Adapter bietet eine einzelne Android-Activity und darin die Library-Oberflächen.
Drucken, native Plugins, Hintergrundthreads, Papierkorb und `plat::fs` sind nicht
angeschlossen; nicht verfügbare Host-Anfragen enden mit `Unsupported`. `content://` schreibt
nicht atomar (SAF kann kein Dokument durch Umbenennen ersetzen), und die Zwischenablage lässt
sich nur lesen, solange die App den Fensterfokus hat (direkt nach dem Picker kurz nicht).

Barrierefreiheit (seit 19.10.2026): `android/GalleryAccessibility.java` stellt jedes Widget
als virtuelle View eines `AccessibilityNodeProvider` dar (Klasse, Text, Zustände, Werte,
Auswahl, Rechteck), nimmt die Aktionen von TalkBack entgegen (Klick, Fokus, Wert, Text,
Auswahl, Explore-by-Touch) und sendet die Ereignisse; eingeschaltet wird der Spiegel mit dem
Accessibility-Schalter des Systems (`docs/GUIDE.md`, Abschnitt 19). Im Browser derselben App
liegt ein ARIA-Schattenbaum über dem Canvas (`web/a11y.js`).
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
liest Klassen, Texte, Zustände, Wertebereiche, Rechtecke und Auswahl der virtuellen Views,
klickt, setzt Fortschritt, Text und Auswahl, verschiebt den Accessibility-Fokus und prüft die
Ereignisse sowie Explore-by-Touch. Am 19.10.2026 auf einem x86_64-Emulator mit Android 15 /
API 35: 30 Prüfungen bestanden; `GalleryInstrumentation` (78) weiter grün. TalkBack selbst
war auf dem Image nicht installiert und wurde nicht ausprobiert.

Browser (Playwright über `PLAYWRIGHT_MODULE` oder die Node-Modulsuche; Chromium aus
`CHROME_PATH`, sonst das installierte Chrome, sonst Playwrights Chromium, siehe
`tools/tests/playwright.mjs`; `--channel=msedge` nimmt stattdessen Edge):

```powershell
python RustLib/build_rs.py app androidGallery --target web
node RustLib/apps/androidGallery/tests/a11y_web.mjs RustLib/build
```

33 Prüfungen: Chromiums eigener Accessibility-Baum (DevTools-Protokoll), Klicks wie von
einem Screenreader, Slider, Tab-Fokus, Tippen, Auswahl, Ersetzung, Zeiger, `?a11y=0`.

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
