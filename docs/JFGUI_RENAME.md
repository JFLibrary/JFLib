# Prüfung der Umbenennung zu jfgui

Stand: 14. Oktober 2026. Arbeitsordner CraftLIB (`GitHub/JFLib`). Die allgemeine
Textumbenennung wurde vom Benutzer ausgeführt; diese Etappe prüft ihre Schnittstellen
und korrigiert die dabei entstandenen Übergangslücken.

## Korrekturen

- Der CLI-Einstieg wurde von `tools/asmgui-agent.py` nach `tools/jfgui-agent.py`
  verschoben. Dokumentation, Tests und tatsächlicher Dateiname stimmen damit überein.
- Die Linux-X11-Konstante heißt `A_JFGUI_SEL`. Der ersetzte Name `A_jfgui_SEL` war mit
  den aktiven Rust-Warnungen als Fehler nicht übersetzbar. Alle Zugriffe sind angepasst.
- Die beiden Windows-Klassennamen wurden auch in ihren UTF-16-Zahlenarrays auf
  `JFGUI` und `JfGuiPopup` geändert. Ihre Arraygrößen enthalten jeweils den Nullabschluss.
- Alle nativen und Web-Builds wurden gegen die neuen Rust-/JavaScript-Namen erstellt.
  Alte Binärdateien mit alten MCP-Werkzeugnamen sind kein Nachweis der neuen Schnittstelle.
  Ein fertiges `buildManager.exe` liegt nicht mehr im Repository; es entsteht mit
  `python build_rs.py app buildManager` unter `build/win/`.
- Der Headless-Start über die CLI vererbt Standardinput, Protokolloutput und Diagnoseoutput
  ausdrücklich. Unter Windows gingen umgeleitete Handles zuvor beim zweiten Prozessstart
  mit `CREATE_NO_WINDOW` verloren. Der Ende-zu-Ende-Test führt eine Unicode-Bearbeitung
  über die tatsächliche CLI und anschließend die Zustandsabfrage durch, mit JSON und MCP
  auf beiden Plattformen.
- Synthetische Font-Testdateien wurden mit `tools/font_fixtures.py` neu erzeugt. Dadurch
  stimmen ihre binären internen Familiennamen wieder mit den umbenannten Erwartungen
  überein. Die Fonts der Anwendungen und ihre Lizenzhinweise werden dabei nicht verändert.
- 18 fehlende DLL-/SO-Referenzplugins wurden bytegleich aus dem alten Build in den
  CraftLIB-Testcache übernommen. Die alte Ablage bleibt unverändert. Der PaintCraft-Test
  synchronisiert seinen Hintergrundimport vor der Prüfung des geöffneten QOI-Bildes;
  die Bild-, Plugin- und Hash-Erwartungen bleiben erhalten.

Aktuelle Namen: `window.jfguiAgent`, Ereignis `jfgui-ready`, WASM-Importmodul `jfgui`,
Exports `jfgui_*`, MCP-Werkzeuge `jfgui.*` und CLI `tools/jfgui-agent.py`. Aktivierung,
Schemas, Revisionen, private IPC und die Trennung von Protokoll-/Diagnoseausgabe bleiben
die vorhandenen gemeinsamen Verträge.

## Browserdateien

Das Replace hat auch `asmgui-files-v1` in `jfgui-files-v1` geändert. Eine reine
Umbenennung würde einen leeren Browserspeicher öffnen. `web/storage.js` übernimmt
deshalb fehlende Einträge aus der alten Datenbank:

1. Aktuelle Einträge werden gelesen; vorhandene neue Schlüssel behalten Vorrang.
2. Fehlende alte Einträge werden in einer gemeinsamen Schreibtransaktion kopiert.
3. Erst deren erfolgreicher Abschluss bestätigt die Übernahme und aktiviert dauerhafte
   Schreibvorgänge. Eine Quota-/Zugriffsablehnung erzeugt keine Teilübernahme.
4. Bei Fehler bleiben vorhandene alte und neue Dateien lesbar; dauerhafte Speicherung
   bleibt deaktiviert. Beim nächsten Start wird die Übernahme erneut versucht.
5. Die alte Datenbank wird erhalten. Wiederholtes Starten erzeugt keine Duplikate.

Der alte Datenbankname bleibt als **bewusster Kompatibilitätsschlüssel** im neuen
Host enthalten. Das ist keine aktive alte Steuerungsschnittstelle. Die Migration
berücksichtigt alle App-Einträge, während eine App nur ihre eigenen Dateien lädt.
Native Konfigurationsordner und DOCX-/native Zwischenablage-Payloads verwenden weiterhin
ihre vorhandenen Datenformate; deren Inhalte werden nicht durch die Hostmigration verändert.

## Nachweise

- `build/wordcraft-table-formulas/build-rename-all.log`: kompletter aktueller Build
  aller vom Build-System für Windows, Linux und Web erfassten Apps/Ziele.
- `rename-python-tests.log`: Build-Manager und Quellauflösung; der zusätzliche
  WSL-Prozessabbruchtest wird mit `jfgui_TEST_WSL=1` ausdrücklich aktiviert.
- `rename-cli.log` / `rename-cli.json`: echter Headless-Start mit umgeleiteten Streams,
  UTF-8-Bearbeitung, Revision und den aktuellen MCP-Werkzeugnamen. Reproduzierbar mit
  `python tools/tests/agent_cli.py --target both --out build/wordcraft-table-formulas`.
- `rename-storage-browser.log` / `rename-browser/results.json`: Chromium und Firefox,
  normale Übernahme sowie simulierte Quota-Ablehnung mit erfolgreichem erneutem Start.
  Alte DOCX-Dateien öffnen bytegenau; neue Konfliktdateien werden nicht überschrieben;
  Einträge anderer Apps bleiben erhalten.
- `rename-apps-browser.log` / `browser-tests/results.json`: Template, Gallery, jfWord,
  Kritzel und PaintCraft mit aktivierbarer Steuerung, Eingabe, Größenänderung, Agenten-
  Widerruf und den vorhandenen Speicher-/Fehlerabläufen.
- `rename-extra-browser.log` / `rename-extra-browser/results.json`: DataGallery,
  OfficeIconTest und AndroidGallery als zusätzliche Browser-GUIs, jeweils Start,
  Größenänderung, helle/dunkle Ausgabe und Aktivierung/Widerruf der Steuerung.
- `rename-compute-browser.log` / `compute-tests/results.json`: tatsächlich ausgeführte
  skalare, SIMD- und Shared-Memory-Workerpfade in Chromium und Firefox. Die portable
  Rechenprobe behält den Referenzdigest `5a22a1f803872742`; parallele und verschachtelte
  Aufgaben sowie Abbruch und erneuter Start bestehen.
- Native Projektprüfung: 363/363 Skripte. Python-Prüfungen einschließlich des ausdrücklich
  aktivierten WSL-Abbruchs: 17/17, keine übersprungenen Tests. CLI- und Grafik-Agenten:
  jeweils vier erfolgreiche Fälle. Die gemeinsamen Browsernachweise umfassen 42 Fälle:
  28 GUI-, vier Übernahme- und zehn Rechenkern-/Workerfälle. Die zusätzliche
  jfWord-Featurematrix steht in `verification.json`; der Namenswechsel ist separat in
  `rename-verification.json` erfasst.

Der All-App-Build enthält 22 Windows-, 21 Linux- und neun Web-Projekte; jede Web-Ausgabe
enthält skalare, SIMD- und Threads-WASM-Binaries. `all-app-manifest.json` verzeichnet die
Artefakte und SHA-256-Hashes. Die lokale Vorschau liefert die neue Fassung; die neun
jfWord-Host-/WASM-Dateien wurden über HTTP bytegleich mit dem Build abgeglichen.

Die Browserprüfungen verwenden eigene Kontexte und lokale Testserver. Die bereits offene
Benutzerseite wird nicht neu geladen. Android-JNI-Namen wurden statisch abgeglichen;
ein Android-Gerätelauf ist kein Teil dieses Nachweises.
