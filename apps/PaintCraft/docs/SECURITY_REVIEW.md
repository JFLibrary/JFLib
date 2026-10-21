# PaintCraft – Code-Review mit regulären Tests

Stand: 11. Oktober 2026. Ausgangscommit: `00ac0a03cab182dc5f34ea0ffd38eec22a38eeaa` plus lokale Änderungen.

## Auftrag und Prüfgrenzen

Die erste Fortsetzung des vorherigen Reviews erfolgte nach der Nutzerpräzisierung als normaler Code-Review.
Quelltext, Aufrufer, Fehlerbehandlung, normale Builds und kleine Funktionstests bilden die
Grundlage. In diesem ersten Durchlauf wurden keine neuen Fuzzing-, gezielten Absturz-,
Angriffs- oder Ressourcen-Stresstests ausgeführt. Auf erneuten Nutzerauftrag folgte danach
die unten getrennt dokumentierte vertiefte Prüfung mit begrenzten Mutations- und Absturztests.
Der ursprüngliche Plan eines vollständigen Sicherheitsaudits ist damit **nicht als
abgeschlossen oder als Sicherheitsnachweis zu verstehen**.

Alle 59 Rust-Dateien der PaintCraft-App wurden inventarisiert und den Funktionsbereichen
zugeordnet. Vertieft verfolgt wurden die Datenflüsse für Öffnen/Speichern/Schließen,
Dokumente und Ebenen, Zwischenablage, Verlauf, Werkzeuge, Effekte, Parser, Plugins,
Diagnose und den Browserhost. Die Dateimatrix unterscheidet Durchsicht relevanter
Implementierungspfade von einer Strukturübersicht; sie behauptet keine vollständige
Zeilenprüfung jeder Datei. Gemeinsame Bibliotheken sind nur entlang dieser Aufrufpfade
einbezogen. Andere Apps, Legacy-Assembler und eingebettete Referenzprojekte sind nicht
eigenständig geprüft.

Die ursprünglichen Quellkopien, Hashes und Logs liegen unter
`build/PaintCraft-security/baseline/` und `build/PaintCraft-security/logs/`.
Die 59 App-Dateien waren bei Wiederaufnahme gegenüber dieser Baseline unverändert.
Der Erstbericht wurde **vor Änderungen am Produktivcode** gespeichert. Die ergänzenden
Abschlussabschnitte dokumentieren anschließend Korrekturen und Nachtests.

## Bestätigte Befunde des Erstberichts

### PN-01 · P1 · Browser meldet ein nur flüchtig gespeichertes Dokument als gespeichert

- Ausgangsstelle: `fileio.rs:664–679`, außerdem `save_options_done`; Bibliotheksgrenze
  `lib/plat/web_files.rs:214` und `lib/plat/web.rs:190`.
- Auslöser: regulär ein Bild importieren, bearbeiten und als PDN oder Bildformat speichern.
- `plat::write_file` schreibt im Browser absichtlich nur in das virtuelle Dateisystem.
  PaintCraft wertet dessen Erfolg unmittelbar als dauerhaften Speichererfolg und löscht
  `DF_MODIFIED`, ohne `plat::web::persist` anzufordern und dessen Ergebnis abzuwarten.
- Auswirkung: Nach Neuladen der Seite fehlt die vermeintlich gespeicherte Datei;
  auch „Speichern und schließen“ kann ungesicherte Arbeit verwerfen.
- Nachweis: bestehende normale Chromium-/Firefox-Szenarien, jeweils `dirty=0` bei
  leerem IndexedDB-Dateispeicher. Ergebnis:
  `build/PaintCraft-security/before/security-browser/results.json`.
- Korrektur: Speichern asynchron abschließen, erst nach bestätigter Persistenz das
  Dokument als sauber markieren und eine Schließen-/Speichern-alle-Kette fortsetzen.
  Währenddessen konkurrierende Bearbeitungen und Speicherketten verhindern.

### PN-02 · P1 · Dateischreiben überschreibt eine bereits vorhandene Zwischendatei

- Ausgangsstelle: `lib/plat/win_sys.rs:271`, `lib/plat/linux_sys.rs:249`.
- Auslöser: Neben dem Ziel existiert bereits eine Datei `<Ziel>.tmp~`.
- `CREATE_ALWAYS` beziehungsweise `O_CREAT|O_TRUNC` öffnet und überschreibt diesen Namen,
  bevor das Ergebnis über das Ziel umbenannt wird. Ein paralleler Speicherprozess kann
  denselben Namen verwenden. Der vorherige Review enthält zusätzlich einen Nachweis
  für Verknüpfungen; dieser Test wurde in dieser Fortsetzung nicht erneut ausgeführt.
- Auswirkung: Verlust einer anderen Zwischen-/Zieldatei und unzuverlässiges Ersetzen.
- Einfacher Nachweis: Eine gewöhnliche Datei `saved.png.tmp~` mit kurzem Text verschwindet
  nach `--shot saved.png`, obwohl das PNG erfolgreich geschrieben wird; Windows und Linux.
- Korrektur: Eigenen Namen im Zielverzeichnis verwenden und ausschließlich neu anlegen;
  bei Kollision einen neuen Namen wählen, vollständiges Schreiben und Synchronisieren
  prüfen und nur die selbst angelegte Zwischendatei bereinigen.

### PN-03 · P2 · Linux verliert restriktive Dateirechte beim Ersetzen

- Ausgangsstelle: `lib/plat/linux_sys.rs:253–262`.
- Auslöser: Reguläres Speichern über eine bestehende private Datei mit Modus `0600`.
- Die neue Datei wird mit `0666` abzüglich umask angelegt; die Rechte des bisherigen
  Ziels werden nicht übernommen. Im vorhandenen Test ergibt sich `0644`.
- Auswirkung: Andere lokale Nutzer erhalten bei entsprechend zugänglichem Verzeichnis
  Leserechte, obwohl die Zieldatei vorher privat war.
- Nachweis: Kleiner regulärer Ersetzungstest auf dem Linux-Dateisystem in WSL.
- Korrektur: Bestehende Zugriffsbits beibehalten, die Zwischendatei zunächst privat
  anlegen und einen Fehler beim Setzen der Rechte vor dem Ersetzen melden.
  ACLs, Eigentümerwechsel und absichtlich gleichzeitig veränderte Verzeichnisse sind
  kein durch diesen Review abgedeckter Nachweis.

### PN-04 · P2 · Unterschiedliche Dateinamen werden unter Linux und im Browser zusammengelegt

- Ausgangsstelle: `doc.rs:442–450` und Aufruf `fileio.rs:403`.
- `path_eq` vergleicht auf allen Plattformen ohne Beachtung der ASCII-Großschreibung.
- Auslöser: Zwei gültige Bilder `Case.png` und `case.png` nacheinander öffnen.
- Auswirkung: Statt des zweiten Bildes wird der bereits geöffnete erste Tab aktiviert.
  Die Anwendung zeigt damit nicht die ausgewählte Datei an.
- Nachweis: Kleines UI-Skript auf dem Linux-Dateisystem: Dokumentzahl bleibt 1,
  Dateiname bleibt `Case.png`, erwartet sind 2 Dokumente und `case.png`.
- Korrektur: Vergleich ohne Groß-/Kleinschreibung nur für das Windows-Ziel;
  Linux und virtuelles Browserdateisystem vergleichen exakt.

### PN-05 · P2 · Oberflächenberechnung kann auf WASM32 eine zu kleine Länge ergeben

- Ausgangsstelle: `sel.rs:1155–1163`, `lib/gfx/lib.rs:76–84`.
- Der zulässige Dimensionsbereich reicht bis 65535/65536. `stride * height` wird direkt
  in `usize` berechnet. Das kann auf WASM32 überlaufen, bevor der Pixelpuffer angelegt wird.
  Zum Beispiel ist der ausgerichtete ARGB-Zeilenabstand bei Breite 65535 gleich 262144;
  multipliziert mit Höhe 16384 ergibt das 4294967296, außerhalb von 32-Bit-`usize`.
- Aufrufer: `iop_resize`/`iop_canvas` → `iop_alloc` → `surf_try`; allgemeine Oberflächen
  werden auch über `Surface::new` angelegt. Die vorgelagerte Seitenlängenprüfung verhindert
  den Produktüberlauf nicht.
- Auswirkung: Unpassende Puffergeometrie; spätere sichere Slice-Zugriffe können abbrechen.
  Keine Aussage über Codeausführung. Keine großen Bilder oder Abstürze dafür ausgeführt.
- Nachweis: Quelltext und Größenrechnung; Laufzeitverhalten großer Eingaben bewusst
  nicht getestet. Korrektur: geprüfte Multiplikation und ein Fehler-/Leerwert vor Allokation.

### PN-06 · P2 · Optionswerte werden beim Start als Bilder geöffnet (Nachbefund)

- Stelle vor Korrektur: `fileio.rs:71–72, 209–231`; Aufrufer für `--config` und
  `--settings-save-at` in `settings.rs`, `--plugins` in `plugins.rs`, `--crash-dir` in `crash.rs`.
- Die Liste der Optionen mit separatem Wert enthält diese vier unterstützten Optionen
  nicht. Der Startparser versucht daher deren Verzeichnis- oder Zahlenwerte als Bilder
  zu öffnen. Ein normaler Start mit `--config ORDNER` zeigt eine unzutreffende Dateifehlermeldung.
- Die Fehlerleiste verschiebt auch Werkzeugfenster um 48 Pixel. Drei Koordinatenprüfungen
  in `settings.uis` erwarten bisher genau diesen unerwünschten Zustand. Ohne den expliziten
  `--config`-Parameter und mit isolierter Umgebungskonfiguration scheitern nur diese drei
  Positionsprüfungen auf beiden Plattformen; die Einstellungswerte selbst bestehen.
- Ein kleiner Starttest mit den vier Optionswerten bestätigt den Fehler auf Windows und
  Linux: `build/PaintCraft-security/after/normal-before-options.json`, jeweils `options.log`.
- Korrektur: Unterstützte Optionswerte beim Öffnen von Bildern überspringen;
  die UI-Erwartungen auf das Layout ohne Fehlerleiste setzen und das Fehlen einer
  Fehlermeldung ausdrücklich prüfen. Es werden keine Plugins geladen oder Abstürze ausgelöst.
- Dieser Nachbefund wurde vor seiner Korrektur ergänzt.

## Ausgangstests und Abdeckung

Aus dem vorherigen Chat übernommene, protokollierte Ergebnisse:

- Entwicklungs- und Release-Builds: Windows, Linux und WASM erfolgreich.
- 122 UI-Läufe nach Trennung der Konfigurationen erfolgreich. Diese historische Zahl
  umfasst auch Tests, die in dieser Fortsetzung nicht mehr ausgeführt werden.
- `pn_gradient_check` bestanden. `pn_settings_check` meldete 12 Abweichungen bei Fenster-
  geometrie und Testumgebung; diese sind kein belastbar bestätigter Produktbefund.
- Die frühere Crashprüfung wurde unterbrochen; kein Bestehensnachweis.

Neue kleine Baselineprüfung:

```text
python apps/PaintCraft/tests/file_save_check.py --out build/PaintCraft-security/before --label normal-before
```

PNG-Ausgabe funktioniert auf beiden nativen Plattformen. Die Erwartungen zur unveränderten
Zwischendatei scheitern auf beiden Plattformen; Linux-Dateirechte und Dateiunterscheidung
scheitern ebenfalls. Ergebnisse und UI-Logs:
`build/PaintCraft-security/before/normal-before.json` und `before/file-save-cases/`.

Die separate Datei `REVIEW_COVERAGE.csv` hält Verantwortung, Schwerpunkt und Prüftiefe
pro App-Datei fest: 41 Dateien mit durchgesehenen Implementierungspfaden, 18 mit
Strukturübersicht. Zeilenzahlen und protokollierte Lesebereiche beziehen sich auf den
Ausgangsstand; ergänzende Nachprüfungen sind gesondert vermerkt.
Normale UI-Regressionen erfassen zusätzlich Werkzeugzustände,
Auswahl, Bildoperationen, Effekte, Dialoge, Speichern und Verlauf.

## Offene Punkte und bewusst begrenzte Aussagen

- Keine vollständige Prüfung aller Decoder, PE-/ELF-Loader oder der kompletten Runtime.
  Plugin-Deskriptorprüfungen ersetzen keine Isolation: native Plugins laufen als Code
  mit den Rechten der Anwendung. Der Browser registriert ausschließlich eingebaute Codecs/Effekte.
- Dateiimport im Browser, Einstellungen, Paletten und ICC-Export besitzen weitere
  Unterschiede zum nativen Host; hier wird kein vollständiger Persistenznachweis für
  jede Nebenfunktion behauptet.
- Die Crashberichtsfunktion ruft trotz fester Berichtspuffer allgemeine Plattformfunktionen
  auf, die allozieren können. Ob dies im Fehlerkontext zu einer Blockade führt, wurde
  im ersten Durchlauf nicht durch absichtliche Abstürze untersucht. Die spätere Vertiefung
  bestätigt einen regulären Testabsturz je nativer Plattform. Ein bereits beschädigter
  Allocator oder beliebige andere Absturzkontexte bleiben dadurch nicht abgedeckt.
- Einige Kommando-/Labeltabellen werden absichtlich mittels `leak` veröffentlicht.
  Langzeit-Speicherwachstum ist nicht durch Lasttests vermessen worden.
- Keine neue Prüfung fremder Dienste oder Daten; sämtliche Tests verwenden eigene
  kleine Dateien, isolierte Konfigurationen und temporäre Browserkontexte.

## Korrekturen und Abschlussprüfung

### Ergänzung vor der Korrektur des INI-Aufrufers

Beim abschließenden Abgleich der Aufrufer wurde die gleiche Ursache von PN-02 und PN-03
zusätzlich in `lib/base/ini.rs:200–214` bestätigt: `Ini::save` implementiert eine eigene
Schreibfolge über `<Pfad>.tmp~`, ignoriert das Synchronisationsergebnis und verwendet
`plat::write_file` bisher nicht. PaintCraft ruft dies in `settings.rs:839` auf.
Ein regulärer Speichervorgang in einem isolierten Konfigurationsordner überschreibt die
gewöhnliche Datei `settings.ini.tmp~` auf Windows und Linux; Linux ändert `0600` in `0644`.
Die drei Erwartungen scheitern vor Änderung dieses Aufrufers, während das Speichern
selbst funktioniert. Nachweis: `build/PaintCraft-security/after/normal-before-ini.json`.
Dieser Zusatz ist vor der INI-Korrektur gespeichert; die ursprüngliche Erstfassung bleibt
unter `build/PaintCraft-security/INITIAL_NORMAL_REVIEW.md` erhalten.

### Umgesetzte Korrekturen

| Befund | Ergebnis im aktuellen Quelltext | Nachprüfung |
| --- | --- | --- |
| PN-01 | `fileio.rs:686` fordert Browserpersistenz an, `poll_save` schließt sie ab; `main.rs:545` sperrt konkurrierende Befehle. Erst bestätigtes Speichern markiert sauber und setzt eine Schließen-Kette fort. | Chromium und Firefox: Speichern, Neuladen, Speichern und Schließen, verweigertes Speichern, PNG-Optionen. |
| PN-02 | `lib/plat/win_sys.rs:280` und `linux_sys.rs:251` legen eine eigene Zwischendatei exklusiv an. Schreiben und Synchronisieren müssen erfolgreich sein. `lib/base/ini.rs:201` verwendet dieselbe Funktion. | Gewöhnliche bereits existierende `.tmp~`-Dateien bleiben bei Bild- und Einstellungsspeicherung erhalten. |
| PN-03 | Linux übernimmt vorhandene Zugriffsbits; ein Fehler beim Setzen der Rechte verhindert das Ersetzen. | Bild- und Einstellungsdateien behalten `0600`. |
| PN-04 | `doc.rs:442` vergleicht unter Linux und Web exakt; Windows behält den bisherigen Vergleich. | Zwei kleine Dateien `Case.png` und `case.png` öffnen als zwei Dokumente unter Linux und in beiden Browsern. |
| PN-05 | `sel.rs:1155` prüft die Multiplikation; `lib/gfx/lib.rs:77` prüft zusätzlich die adressierbare Allokationsgröße. | Quelltextprüfung der Fehlerpfade und Builds aller drei Ziele; keine großen Allokationen ausgelöst. |
| PN-06 | `fileio.rs:71` überspringt Werte der vier zusätzlichen Startoptionen. `settings.uis` verlangt eine leere Fehlermeldung und prüft die dazu passenden Fensterpositionen. | Kleine Startprüfung und vollständiger Einstellungstest unter Windows und Linux. |

### Durchgeführte Prüfungen

| Prüfung | Ergebnis |
| --- | --- |
| Entwicklungsbuild, Windows/Linux/WASM | 3/3 bestanden, mit INI- und Optionskorrektur. |
| Release-Build, Windows/Linux/WASM | 3/3 bestanden, mit INI- und Optionskorrektur. |
| Reguläre PaintCraft-UI-Suite | 118/118 bestanden: 59 Skripte je nativer Plattform, mit `--verify-dirty` und isolierten Einstellungen. Die beiden Skripte mit absichtlich fehlerhaften Plugins wurden ausgeschlossen. |
| Einstellungstest nach INI- und Optionskorrektur | 2/2 bestanden, Windows und Linux. |
| Kleine Datei-/Starttests nach allen Korrekturen | 13/13 bestanden: Bildspeicherung, INI-Speicherung, bestehende Zwischendateien, private Linux-Rechte, getrennte Dateinamen und Optionswerte. |
| Browserabläufe nach allen Korrekturen | 14/14 bestanden, je sieben in Chromium und Firefox. |
| Regeln, Skriptsyntax, generierte API-Dokumentation, Patch-Whitespace | Bestanden. |

Die 118 UI-Läufe erfolgten nach PN-01 bis PN-05, vor der zusätzlichen Umstellung von
`Ini::save` und der Optionskorrektur. Nach diesen Änderungen wurden die betroffenen
Einstellungstests, alle 13 kleinen Datei-/Startprüfungen sowie die 14 Browserprüfungen
erneut ausgeführt. Es wird kein zweiter vollständiger UI-Durchlauf behauptet.
`crash_report.uis` prüft lediglich die Anzeige einer vorhandenen Textdatei; dabei wird
kein Absturz verursacht. Die verweigerte Browser-Speicherung wird durch einen einfachen
Fehlerrückgabewert simuliert, ohne Speicherplatz zu füllen.

Ein anfänglicher Build scheiterte zwischenzeitlich in parallel bearbeitetem UI-Code
(`lib/ui/list.rs`, Lebensdauer eines Rückgabewerts). Nach dessen Korrektur ließen sich
die Entwicklungs- und Release-Ziele wieder bauen. Diese Änderung stammt nicht aus
diesem Review. Browser-Testfehler durch veraltete Zustandsrevisionen und die Verwechslung
von Hauptfenster-Modalität mit einem eigenen Dialogfenster wurden im Testtreiber
korrigiert; die endgültigen Ergebnisse oben stammen aus vollständigen erfolgreichen Läufen.
Die erste gezielte Wiederholung von `settings.uis` ohne dessen Kopfzeilenargument
`--config` machte PN-06 sichtbar. Die abschließende Wiederholung verwendet die
vorgesehenen Skriptargumente und den korrigierten, fehlerfreien Startzustand.

### Reproduktion und Artefakte

Aus `RustLib/`:

```text
python build_rs.py app PaintCraft --target all --out build/PaintCraft-security/after -j 4
python build_rs.py app PaintCraft --target all --release --out build/PaintCraft-security/after-release -j 4
python apps/PaintCraft/tests/file_save_check.py --out build/PaintCraft-security/after
node apps/PaintCraft/tests/security_web.mjs build/PaintCraft-security/after
python build_rs.py check
python tools/gendoc.py --check
```

Der native Dateitest erwartet Windows mit WSL-Distribution `Ubuntu-24.04`; der Linux-Teil
arbeitet für Rechte und Groß-/Kleinschreibung auf dem Linux-Dateisystem. Der Browsertest
benötigt Playwright und die beiden Browser. `PLAYWRIGHT_MODULE`, `PLAYWRIGHT_BROWSERS_PATH`
und `CHROME_PATH` können die lokalen Standardpfade überschreiben. Alle Dateien,
Konfigurationen, der lokale HTTP-Server und Browserkontexte gehören zur Testumgebung.

Ergebnisdateien relativ zu `RustLib/`:

- `build/PaintCraft-security/after/file-save-results.json`: 13 finale Erwartungen mit Artefaktpfaden.
- `build/PaintCraft-security/after/security-browser/results.json`: 14 finale Browserergebnisse;
  nebenan Bildschirmaufnahmen und Zustände nach erfolgreichem Speichern.
- `build/PaintCraft-security/logs/after-isolated-*.log`: 118 UI-Läufe.
- `build/PaintCraft-security/logs/after-options-{win,linux}-settings.log`: finale Einstellungsprüfungen.
- `build/PaintCraft-security/logs/normal-final-{dev,release}-build.log`: letzte Buildausgaben.
- `build/PaintCraft-security/normal-review-baseline.json` und `normal-review-final.json`:
  Quellhashes zur Wiederaufnahme und zum Abschluss. Die zusätzliche INI-Ausgangskopie
  liegt unter `baseline/ini-before-normal-fix.rs`.
- `build/PaintCraft-security/INITIAL_NORMAL_REVIEW.md` und die beiden Dateien
  `NORMAL_REVIEW_WITH_*_ADDENDUM.md`: vor der jeweiligen Korrektur gesicherte Befunde.

Die sechs bestätigten Ursachen sind im Quelltext korrigiert. Die offenen Prüfgrenzen
oben bleiben bestehen; dieser Bericht ist kein vollständiges Sicherheitsaudit.

## Vertiefte Prüfung auf erneuten Nutzerauftrag

Am 11. Oktober 2026 wurden auf ausdrücklichen Wunsch zusätzliche lokale Tests ausgeführt.
Die 146 zuvor erfassten Quelldateien waren gegenüber dem letzten Review unverändert.
Für den neuen Durchlauf wurden 416 App-, Bibliotheks- und Browserhostdateien erfasst und
frische Entwicklungsbuilds von PaintCraft (Windows, Linux, WASM) sowie codectest (Windows,
Linux) unter `build/PaintCraft-deeper/` erstellt. Für diese Vertiefung wurden keine
Produktivquellen geändert; Änderungen betreffen Testtreiber und diesen Bericht.

Während des Durchlaufs wurden allerdings sieben gemeinsame Dateien parallel bearbeitet:
`lib/ui/{basic,consts,edit,lib,ribbon,style}.rs` und `lib/doc_view/src/lib.rs`. Ihre
Änderungszeiten liegen nach den PaintCraft-Testbuilds vom 11.10.2026 um 20:59 Uhr.
Diese späteren Änderungen sind nicht durch die hier gemeldeten Ergebnisse abgedeckt.
Die Ergebnisse sind deshalb an die gespeicherten Binaries und deren SHA-256-Hashes
gebunden. Die 146 Dateien des vorherigen Reviews blieben beim Abschlussabgleich unverändert.

### Ergebnisse

| Prüfbereich | Umfang und Ergebnis |
| --- | --- |
| PNG-/PDN-Codec-Prüfungen | **3.781/3.781 bestanden**: 1.096 unter Windows, 2.685 unter Linux. Referenzpixel, Ebenen, Metadaten, Encodieren/erneutes Lesen, PNG-Farbtypen und APNG. |
| Darin: PDN-Strukturgrenzen | **61/61 bestanden**: fehlerhafte Längen, ungültige Referenzen, Zyklen, begrenztes Tabellenbudget und Verschachtelungsgrenze 60/64/65. |
| Darin: Mutationsserien | **1.528/1.528 bestanden**: 1.153 PDN-Fälle, 375 PNG-Fälle; feste Zufallsfolge `20261011`. Eingaben werden gekürzt oder verändert, einschließlich Metadaten und Blockfeldern. Kein unerwarteter Prozessabbruch beobachtet. |
| Paralleles Dateischreiben | **16/16 Erwartungen bestanden**: fünf Runden mit je vier gleichzeitigen Schreibern pro Plattform, vollständige PNG-Bytes statt vermischter Inhalte, Erhalt fremder `.tmp~`-Dateien, Bereinigung eigener Zwischendateien und korrekt abgelehnte Verzeichnisziele. |
| Vorhandene Verknüpfungen | **5/5 bestanden**: Windows-Hardlink und Linux-Symlink innerhalb eigener Testverzeichnisse; die unabhängige Testdatei bleibt erhalten, PNG-Ausgabe gültig, private Linux-Rechte bleiben `0600`. |
| Browserspeicherung | **18/18 bestanden**, je neun in Chromium und Firefox. Zusätzlich zur bisherigen Suite: zurückgehaltene Speicherbestätigung blockiert Bearbeiten, Schließen und erneutes Speichern; „Alles speichern“ setzt die Kette über zwei PNG-Dokumente fort. |
| Beschädigte Einstellungen und Konfigurationspfad | **4/4 Szenarien bestanden**, je zwei pro Plattform, nach Korrektur einer veralteten Testannahme. |
| Kontrollierte Absturzdiagnose | Je ein eingebauter Testabsturz unter Windows und Linux: Exitcode 3, genau ein Bericht, Register und Diagnoseabschnitt, vollständige Aufrufkette bis zum Einstiegspunkt sowie Anzeige des Berichts bestanden. Der Symbolnamenvergleich wurde korrigiert und an den erhaltenen Berichten erneut geprüft. |

Die Decoder-Mutations- und NRBF-Grenztests laufen unter Linux mit **512 MiB Adressraum,
20 Sekunden CPU-Zeit und 30 Sekunden Laufzeit pro Batch**. Batches enthalten höchstens
32 Fälle; der vorhandene Treiber kann einen fehlerhaften Batch zur Eingrenzung teilen.
Fehlgeschlagene Prozessläufe würden ihre Eingabebatches und Ausgaben unter `failures/`
behalten. Die gezielten Absturztests waren auf 20 Sekunden unter Linux und 25 Sekunden
im aufrufenden Treiber begrenzt. Sämtliche Dateien und Konfigurationen gehören zum
Testlauf; der Browserhost lauscht nur auf `127.0.0.1`.

### Korrigierte Testannahmen

Es gab **keinen neuen bestätigten Produktfehler** in diesen Läufen. Zwei bestehende
Prüfwerkzeuge enthielten Erwartungen, die nicht mehr zum aktuellen Verhalten passten:

1. `tools/pn_settings_check.py` verlangte nach dem Bereinigen ungültiger Positionen
   weiterhin absolute `x`-Koordinaten. `settings.rs::st_windows_load/st_windows_save`
   verwendet ohne expliziten Platzierungsmodus aber die automatische Anordnung
   (`dock=0`) und entfernt `x/y` absichtlich. Die Prüfung verlangt nun bei diesem Modus
   das Entfernen beider Koordinaten; ältere Referenzbuilds ohne `dock` behalten ihre
   bisherige Erwartung. Die vier Szenarien wurden anschließend neu ausgeführt.
2. `tools/pn_crash_check.py` erwartete ausschließlich `PaintCraft::diag::…`, während
   `build_rs.py` den Rust-Crate-Namen zu `PaintCraft` normalisiert. Die Prüfung akzeptiert
   beide historischen Schreibweisen, verlangt weiterhin exakt dieselbe Funktionsfolge
   und prüft weiterhin jede Rücksprungadresse gegen den tatsächlichen Aufruf im Binary.
   Die Aufrufketten wurden an den bereits erzeugten Berichten und den zugehörigen
   unveränderten Binaries/Map-Dateien erneut erfolgreich geprüft. Dafür wurden keine
   weiteren Abstürze ausgelöst.

### Umfang und verbleibende Grenzen

- Die Mutationsserien sind begrenzt und reproduzierbar; sie ersetzen weder dauerhaftes
  Fuzzing noch eine Sanitizer-Prüfung. Die PDN-Mutationssuite prüft vor allem die
  Prozessstabilität; semantische Korrektheit wird durch die separaten Referenz- und
  Roundtrip-Suiten geprüft.
- Ausgelassen sind die große PDN-Performance-Datei, eine Million NRBF-Objekte und eine
  10.000-stufige Verschachtelung. Die tatsächliche Grenzprüfung um Tiefe 64 ist enthalten.
- Keine weitergehende Prüfung der nativen Plugin-Loader und keine Tests gegen fremde
  Dienste. Der WASM32-Größenüberlauf aus PN-05 wurde weiterhin ohne riesige Allokation
  anhand des Codes geprüft.
- Ein einzelner kontrollierter Absturz pro Plattform belegt den untersuchten
  Diagnosepfad, nicht jede mögliche Beschädigung oder Speicherknappheit im Fehlerkontext.

### Wiederholbare Testbefehle und Nachweise

Aus `RustLib/`, nach den oben beschriebenen Builds:

```text
python apps/PaintCraft/tests/deeper_check.py codecs --out build/PaintCraft-deeper
python apps/PaintCraft/tests/deeper_check.py files --out build/PaintCraft-deeper
python apps/PaintCraft/tests/deeper_check.py settings --out build/PaintCraft-deeper
python apps/PaintCraft/tests/security_check.py --out build/PaintCraft-deeper --label links
node apps/PaintCraft/tests/security_web.mjs build/PaintCraft-deeper --deeper
```

Die Ergebnisse liegen unter `build/PaintCraft-deeper/`: `deeper-codecs.json`,
`deeper-files.json`, `links-security.json`, `deeper-settings.json`,
`deeper-browser/results.json` und `deeper-crash-reports.json`. Die ursprünglichen
Fehlmeldungen der Testwerkzeuge sind in `*-before-checker-fix.json` und den Logs erhalten;
die erfolgreichen Wiederholungen stehen in `logs/settings-final.log` und
`logs/crash-chain-final.log`. `source-manifest.json` hält die geprüften Quellhashes fest.
`deeper-results.json` fasst die Ergebnisse, Binary-Hashes und den Hinweis auf die
parallelen Quelländerungen zusammen.
