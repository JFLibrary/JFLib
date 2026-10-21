# PaintCraft: Umsetzung des Handbuchabgleichs

Stand: 12. Oktober 2026. Quellen und Builds liegen im **JFLib**-Repository.
Der [ursprüngliche Audit](MANUAL_FEATURE_AUDIT.md) bleibt als Ausgangsbefund erhalten.
Diese Umsetzung schließt mehrere konkrete Lücken, **keine vollständige
Paint.NET-API- oder Photoshop-Kompatibilität**.

## Aktueller Erledigungsstand

| Befund | Umsetzung und verbleibende Grenze |
| --- | --- |
| PC-MAN-01 Textmodi | Behoben: Scharf Klassisch rundet Kerning und Zeichenabstände auf Gerätepixel; Modern erhält gebrochene Abstände. Messung und Ausgabe nutzen dieselbe Regel. Drei unterschiedliche Render-Ergebnisse auf Windows und Linux nachgewiesen. |
| PC-MAN-02 Räumliches Kippen | Behoben: X/Y-Kippachsen, inverse perspektivische Abbildung, vier Samplingmodi und vorhandene Kachel-/Auswahlpfade. Vorschau, Abbrechen, Übernahme und Undo getestet. |
| PC-MAN-03 Zusatzeffekte und Plugins | Teilweise geschlossen: elf native Ergänzungen und separater Host für originale klassische CPU-Effekt-DLLs sowie FileType-Factories. Ordnererkennung, Menü-/Dateifilter und Fehlerliste sind angebunden. Moderne Bitmap-/GPU-APIs bleiben offen. |
| PC-MAN-04 Smartphone | Behoben: eigene Smartphone-Vorlage als 30. eingebaute Form; bestehender XAML-Import bleibt nutzbar. |
| PC-MAN-05 Scanner/Kamera | WIA-Import unter Windows und SANE `scanimage --format=png` unter Linux implementiert, mit Hintergrundprozess und Abbruch. Geräteunabhängiger Importpfad getestet, kein echter Scanner/Kameratreiber abgenommen. Linux verwendet das SANE-Standardgerät. |
| PC-MAN-06 Hilfe/Updates | F1, Tutorials und Pluginhilfe öffnen eine lokale Anleitung; Projekt/Feedback sind verlinkt. Optionale HTTPS-Updateprüfung über Python 3, automatische Prüfung standardmäßig aus, Betafilter und gespeicherte Optionen. Kein eigener Forum-/Spenden-Endpunkt. |
| PC-MAN-07 Branding | Info und Diagnose nennen Rust, JFLib und die tatsächliche Plattform. |
| Arbeitsfläche | Hintergrund, beide Schachbrettfarben und Bildschatten einstellbar und persistiert. |
| Neues Bild | Weiß, Transparent, Primärfarbe oder Sekundärfarbe im Neu-Dialog. Transparente Pixel getestet. |
| Textbedienung | UTF-8-sichere Wortnavigation, gebrochene Punktgrößen, Textmarkierung per Umschalt+Navigation und Maus, Strg+A/C/X/V sowie Ersetzen/Löschen einer Markierung. |
| Auswahl | Eigene Federung der A8-Auswahlmaske mit Radius 1–128 px, als Operation der Auswahlhistorie. Abbrechen, Undo/Redo, weiche Werte und anschließendes Invertieren getestet. |
| Große Dateien öffnen | Öffnen, zuletzt verwendete Dateien und Drag-and-drop lesen/dekodieren unter Windows und Linux in einem Hintergrundjob. Abbrechen verwirft den laufenden Import und die Warteschlange; vollständige Ergebnisse werden auf dem UI-Thread übernommen. Startdateien auf der Kommandozeile und der skalare Web-Build bleiben synchron. |
| PSD/PSB | Rasterebenen statt ausschließlich Gesamtbild: Pixel, Unicode-Namen, Sichtbarkeit, Deckkraft und zehn entsprechende Blend-Modi. PSD-Export erhält den 8-Bit-Rasterstapel. PSD/PSB-Import akzeptiert 8/16/32 Bit, Raw/RLE/ZIP/Prediction. Einfache Masken werden auf Alpha angewendet. Photoshop-Gruppen, Vektormasken und Effekte verwenden das gespeicherte Gesamtbild; vollständige Dokumenttreue bleibt offen. |
| G’MIC-Qt | Externer Standalone-Prozess unter Windows und Linux, aktive Ebene als PNG, Auswahlbegrenzung, Undo, Abbruch und mehrere Ergebnisebenen. Der Austausch ist derzeit 8 Bit und erhält keine vollständige mehrlagige Eingabe oder HDR-Daten. |
| Bildpinsel | Native Bildspitzen mit Drehung, Streuung, Größen-/Winkelvariation, Fluss, Deckungsgrenze und Ausrichtung am Strich. JSON/XML-Import unterstützter Brush-Factory-/Dynamic-Draw-Presets mit sechs Originalspitzen; unbekannte/aktive nicht unterstützte Parameter blockieren die Anwendung. Vollständige Plugin-Nachbildung bleibt offen. |
| Schriftformung | HarfRust/GSUB/GPOS, Unicode-Bidi, Schriftausweichliste pro Schriftlauf, Cluster-basierte Messung/Treffertest und getrennte Auswahlrechtecke bei gemischtem Bidi-Text. Vollständige visuelle Navigation und alle Schriften sind nicht abgenommen. |
| IME | Kompositionsanzeige getrennt vom Bild und Editierpuffer; Windows IMM und Wayland text-input-v3 angebunden. Bestätigungen werden unter Wayland als zusammenhängendes UTF-8-Ereignis übertragen. Synthetische Anzeige/Bestätigungen getestet; echte OS-Kandidatensitzungen und X11-IME noch offen. |
| Sprachen | Deutsch, Englisch und Französisch auswählbar und gespeichert. Menüs und viele Bedienelemente übersetzt; fehlende Texte fallen auf Deutsch zurück. |
| Farbtiefe/HDR | Quellen mit höherer Farbtiefe werden als neue 8-Bit-Dokumente importiert, damit normales Speichern die Quelle nicht überschreibt. 16-/32-Bit-Bearbeitung, HDR-Tonemapping und HDR-Ausgabe sind weiterhin nicht implementiert. |

Die elf nativen Ergänzungen sind Farbbalance, Schwarzweiß erweitert, Traum,
Ölgemälde erweitert, Pastell, Weiche Objektkante, Innerer Schatten, Abgeflachte
Kante, Abwedeln/Nachbelichten, inhaltsbasiertes Füllen und Porträtretusche. Die ersten fünf stellen Bausteine für die
Tutorials 10, 11, 27, 29 und 30 bereit. Es sind eigene Algorithmen und Parameter;
eine pixelgleiche Reproduktion fremder Plugins oder der Buchbilder ist nicht
nachgewiesen. Objektkanten-Effekte arbeiten auf dem Alpha der Ebene. Die separate Funktion
„Bearbeiten > Weiche Auswahlkante“ arbeitet dagegen auf der Auswahlmaske. Das
inhaltsbasierte Füllen gleicht kleine Pixelumgebungen ab und füllt die Auswahl von
außen nach innen aus nicht markierten Bildstellen; für den Arbeitsbereich gilt
aktuell eine Grenze von 25 Millionen Pixeln. Es ist kein generatives Modell.
Porträtretusche bietet Farbglättung mit Kantenschutz, keine automatische Erkennung
von Gesichtern und keine identische Implementierung von TR Portrait Retouch.

## Originale Paint.NET-DLLs

Die vom Nutzer zusätzlich gewünschte DLL-Ausführung ist für klassische
`PaintDotNet.Effects.Effect`-Plugins und `FileType`/`IFileTypeFactory` umgesetzt. Der native Windows-Pfad benötigt .NET 9 und eine vorhandene
Paint.NET-5.1-Installation; Linux nutzt den separaten Host unter Wine mit dem unten
beschriebenen Paint.NET-on-Wine-Paket. Paint.NET-Assemblies werden nicht
mitgeliefert. [Build, Benutzung und Kompatibilitätsmatrix](../../../tools/paintcraft-pdn-host/README.md).

Die Ausführung übernimmt Auswahl, Farben und Pinselbreite; Ergebnisse werden
erst nach vollständiger Prüfung mit einem Undo-Schritt übernommen. Für
PropertyBased-Effekte gibt es einen eigenen Parametereditor, keinen vollständigen
Nachbau von Paint.NETs IndirectUI. Getestet wurden Original-DLLs für Invertieren,
BoltBait Pastel und Oil Painting+, einschließlich Editoraufbau für die beiden
BoltBait-Effekte. Ordnererkennung und Menüaufruf werden zusätzlich mit der
unveränderten Pastel-DLL getestet. Die Download-Fixtures bleiben im ignorierten Build-Ordner.

Der Startscan berücksichtigt Effects/FileTypes/Shapes im nativen/portablen Root,
Dokumente/PaintCraft App Files, Dokumente/Paint.NET App Files, die lokale Paint.NET-
Installation sowie freigeschaltete AdditionalPluginDirectoryRoots mit expandierten
Umgebungsvariablen. Archive müssen entpackt werden, neue Plugins benötigen einen
Neustart. Jede DLL wird isoliert geprüft; Fehler und nicht unterstützte moderne
Effekte erscheinen in den Einstellungen. Die Dateifilter erhalten getrennte
Lese-/Schreib-Endungen. Die acht nativen PNAPI-Formatkennungen bleiben unverändert;
32 zusätzliche Kennungen stehen dem externen Codec-Host zur Verfügung.

Dateiformat-Plugins übertragen bisher zusammengefasste Pixel und DPI. Ihre Dateien
werden deshalb als neue, ungespeicherte Bilder importiert. Der Export verwendet den
Standardtoken des Plugins. Schichten, erweiterte Metadaten und eigene Speicher-
Dialoge werden über diese Brücke noch nicht übertragen. Der API-Vertrag wird mit
einer separat kompilierten FileTypeFactory-Fixture geprüft; das ist kein Nachweis
für beliebige Drittanbieter-Dateiformate.

## PSD, Farbtiefe und Schutz der Quelldatei

Die neue Dokument-API erhält unterstützte Rasterebenen unabhängig vom gespeicherten
Gesamtbild. Einfache Pixelmasken werden mit Alpha verrechnet. Off-Canvas-Pixel werden
beschnitten. PSB, Masken, höhere Farbtiefen und zusätzliche Photoshop-Strukturen
markieren den Import als konvertiert: Das Ergebnis hat keinen ursprünglichen
Speicherpfad und verlangt einen neuen Namen. Nicht unterstützte Gruppen, Clipping,
Vektormasken, Ebeneneffekte und Blend-Modi verwenden, soweit vorhanden, das gespeicherte
Gesamtbild mit entsprechendem Hinweis. CMYK und Lab werden nicht dekodiert.

Die PSD-Ausgabe enthält 8-Bit-Rasterebenen und ein Kompatibilitäts-Gesamtbild.
Unvertretbare Blend-Modi werden beim Export abgewiesen. PSB-Ausgabe, editierbare
Photoshop-Texte, Smart Objects und nicht-destruktive Masken werden nicht erhalten.

**Höhere Farbtiefe lesen ist nicht höhere Farbtiefe bearbeiten.** 16-Bit-Werte werden
auf 8 Bit gerundet. 32-Bit-Fließkommawerte werden für die SDR-Anzeige auf 0–1 begrenzt
und aus linearem RGB nach sRGB übertragen; Alpha bleibt linear. Das ist kein
HDR-Tonemapping und keine Garantie für die Profiltreue beliebiger 32-Bit-Dateien.
Auch andere Codecs, die höhere Präzision/HDR melden, importiert PaintCraft mit einem
neuen Dokumentpfad. Die Originaldatei behält ihre ursprünglichen Daten.

Für echte 16-/32-Bit-Bearbeitung müssen Ebenenspeicher, Historie, Auswahl-/Pinsel-
Komposition, Filter, Datei-Ausgabe und Plugin-Transport durchgängig angepasst werden.
HDR-Anzeige braucht zusätzlich eine geeignete Plattform-Ausgabe und einen definierten
SDR-Vorschau-/Tonemappingpfad. Diese Architekturarbeit wird durch den neuen Decoder
nicht als erledigt markiert.

## DLLs unter Linux

Die Endung `.dll` allein bedeutet nicht zwangsläufig Windows-exklusiven Code;
verwaltete .NET-Assemblies können plattformunabhängig sein. Die originalen
Paint.NET-Plugins hängen aber zusätzlich an Paint.NET und dessen Windows-Diensten.
Der native Linux-Lader kann sie nicht als `.so` verwenden.

PaintCraft startet dafür einen separaten Windows-Host unter Wine.
Das [Paint.NET-on-Wine-Projekt des Paint.NET-Entwicklers](https://github.com/paintdotnet/Paint.NET-on-Wine)
ist experimentell und nennt ComputeSharp-GPU-Plugins weiterhin als nicht unterstützt.
Die implementierte Brücke und ihre Prüfungen sind im neuen Abschnitt weiter unten
beschrieben. [Die Host-Anleitung](../../../tools/paintcraft-pdn-host/README.md#linuxwine-setup)
erklärt Einrichtung, getestete Versionen und verbleibende Grenzen.
G’MIC arbeitet über seine native Standalone-Anwendung und braucht diese Brücke nicht.

## Ausgeführte Prüfungen dieser Erweiterung

| Prüfung | Ergebnis / Umfang |
| --- | --- |
| Bestehende UI-Skripte | 61 Windows- und 18 Linux-Läufe bestanden, mit `--verify-dirty`. Die zwei PNAPI-Skripte sind im Baseline-Lauf weiterhin separat ausgespart. |
| PSD/PSB | 84 Läufe auf Windows/Linux: vier Kompressionsarten, Ebenen-Roundtrip, Namen/Deckkraft/Sichtbarkeit, Masken, Gesamtbild-Fallback, 16-/32-Bit-Gesamtbilder und Lr16/Lr32-Ebenenblöcke, PSB-Längen, ungültige Daten, Float-Grenzwerte und Quellschutz für PNG16. |
| G’MIC/Bildpinsel | 12 Läufe mit G’MIC-Qt Standalone 4.0.5: Bildspitze/Drehung/Undo, echter Filter, Auswahlbegrenzung, mehrere Ebenen, Prozessabbruch und fehlendes Programm. |
| Text/Sprache | 16 UI-Läufe: Englisch/Französisch und Persistenz, Preedit, 200-Zeichen-IME-Bestätigung, Bidi-Auswahl, Zwischenablage und Undo/Redo. |
| Shaping-Bibliothek | Fünf Tests: arabische Verbindungen/Lam-Alef, gemischtes Hebräisch/Latein, kombinierende Akzente, erzwungener Font-Fallback und ungültige Schriftdatei. |
| Vorherige Handbuch-Ergänzungen | 18 Läufe aus `paintcraft_manual_check.py` und zehn aus `paintcraft_remaining_check.py` ohne optionale Windows-Plugin-Fixtures erneut bestanden. |

Windows- und Linux-Entwicklungs-/Release-Builds sowie der skalare Web-Build sind
erfolgreich. Zusätzlich bestanden zehn Tests direkt mit den Release-Programmen:
Werkzeugoptionen, Bidi-Auswahl, 32-Bit-PSB-Ebenenimport, Bildspitzen und G’MIC je
Plattform. Ergebnis: `build/paintcraft-next-release/release-tests.json`. Die
portablen Archive unter `build/paintcraft-next-release/` enthalten Programm,
JPEG-XL-Modul, Kurzbeschreibung und Lizenztexte. `artifacts.json` enthält ihre
SHA-256-Prüfsummen. `python build_rs.py check` meldet `lint ok`.

Die frühere Prüfung des Windows-DLL-Hosts und der PNAPI-Fixtures ist im vorherigen
Stand (`589a4fcd`) dokumentiert. Diese ursprünglichen 123 Prüfungen werden nicht
noch einmal zu den aktuellen Zahlen addiert. Der Plugin-Host selbst wurde in dieser
Erweiterung nicht verändert. Echte Scanner, echte IME-Kandidatensitzungen, beliebige
Photoshop-Dateien und sämtliche fremden G’MIC-Dialoge sind damit nicht abgenommen.

Reproduktion im Repository-Root (Python 3, Rust/Cargo und LLVM/LLD):

```text
python build_rs.py app PaintCraft --target both --out build/paintcraft-next -j 6
python build_rs.py app PaintCraft --target both --release --out build/paintcraft-next-release -j 6
python build_rs.py app PaintCraft --target web --web-mode scalar --out build/paintcraft-next-web -j 4
python tools/paintcraft_psd_check.py --out build/paintcraft-next --target both
python tools/paintcraft_creative_check.py --out build/paintcraft-next --target both --gmic
python tools/paintcraft_text_language_check.py --out build/paintcraft-next --target both
python tools/paintcraft_manual_check.py --out build/paintcraft-next --target both
python tools/paintcraft_remaining_check.py --out build/paintcraft-next --target both
```

[CREATIVE_TOOLS.md](CREATIVE_TOOLS.md) beschreibt die separate G’MIC-Installation
für den optionalen Test. [TEXT.md](TEXT.md) beschreibt die Font-Fixture und den
Cargo-Testaufruf. Cargo verwendet den eingecheckten Lockfile; die Lizenztexte werden
mit den Binärdateien beziehungsweise Installationspaketen bereitgestellt.

Lokale Ergebnisdateien stehen unter `build/paintcraft-next/`:
`baseline-results.json`, `psd-check/results.json`, `creative-check/results.json`,
`text-language-check/results.json`, `manual-check/results.json`,
`remaining-check/results.json` und `shaping-tests.log`. Downloads und Testfonts
bleiben im ignorierten Build-Verzeichnis. Der Web-Build ist eine Build-Prüfung,
keine interaktive Browser-Abnahme der neuen Funktionen.

## Modulare Rust-Struktur und erneute Prüfung (12.10.2026)

PaintCraft liegt vollständig unter `src/`, gegliedert in `app`, `document`,
`tools`, `effects`, `workspace`, `integrations`, `preferences` und `diagnostics`.
Die [Architekturübersicht](ARCHITECTURE.md) beschreibt Zuständigkeiten und
Build-Einstiege. Das Text-Shaping-Crate verwendet `lib/text_shape/src/lib.rs`;
der PSD-Codec liegt in `lib/img_all/psd/`.

Der für den Commit vorgemerkte Stand wurde in einer separaten lokalen Kopie
geprüft, unabhängig von gleichzeitig laufenden Änderungen an anderen Apps und
der UI-Bibliothek:

- Windows-, Linux- und skalarer WebAssembly-Build erfolgreich.
- 79 bestehende Editorfälle, 84 PSD-Fälle, 12 Kreativwerkzeugfälle einschließlich
  echtem G'MIC-Qt, 16 Text-/Sprachfälle, 18 Handbuchfälle, 10 weitere Regressionen
  und 5 Shaping-Tests bestanden: insgesamt **224**.
- Zusätzlich **4** Tests für Quellerkennung, Ressourcen und mehrdeutige
  Einstiegspunkte bestanden; Build-Lint und `git diff --check` ohne Fehler.
- `plugins.uis` und `plugins_crash.uis` bleiben in diesem Lauf ausgelassen,
  weil sie separat gebaute Plugin-Testdateien benötigen.

Die Modul- und Ressourcenprüfung erfasst 76 Rust-Dateien sowie Hilfetext,
Sprachkatalog und Update-Helfer. Die Umstrukturierung verändert keine
Befehls-IDs, Einstellungsnamen, Dateiformate oder Plugin-ABI. Die unten genannten
Funktionsgrenzen bestehen weiterhin.

## Linux-DLL-Brücke und Preset-Import (12.10.2026, nach dem Struktur-Commit)

Der native Linux-Editor kann den Windows-DLL-Host jetzt über Wine starten.
Effekte und Dateiformate verwenden denselben begrenzten Transport wie unter
Windows, einschließlich Unicode-/Leerzeichen-Pfaden, Katalog, XAML-Formen,
Auswahlübernahme, Undo/Redo und Abbruch. Eine zusätzliche Heartbeat-Datei beendet
Host und Katalog-Kinder bei ausbleibenden Lebenszeichen; sie deckt auch Wine-
Prozesse ab, die wineserver außerhalb der Unix-Prozessgruppe gestartet hat.

Geprüft wurde Wine 11.19 unter Ubuntu 24.04/WSL mit dem speziellen
Paint.NET-on-Wine-Paket 5.200.9775.2252 und dessen gebündelter .NET-11-Laufzeit.
Die normale Windows-Installation 5.1 scheiterte unter Wine an der Direct2D-
Farbprofilinitialisierung. Der Helper lässt sich mit dem neuen
`tools/paintcraft-pdn-host/deploy_wine.py` in ein privates entpacktes Wine-Paket
einrichten. Der Windows-EXE-Launcher benötigt dort kein separat installiertes
`dotnet.exe`. Details und Versionsgrenzen stehen in der
[Host-Anleitung](../../../tools/paintcraft-pdn-host/README.md#linuxwine-setup).

Die unveränderte BoltBait-`Pastel.dll` erzeugte im direkten Windows-/Wine-Vergleich
identische, gegenüber der Eingabe veränderte Pixel. Vier Prüfungen im nativen
Linux-Editor decken Menüausführung mit Undo/Redo, Dateiformat-Roundtrip einschließlich
Alpha/DPI, Abbruch und XAML-Formen ab (`tools/paintcraft_wine_check.py`). Die
separat gebaute FileType-Fixture verwendet die originale Paint.NET-API; sie
beweist keine Kompatibilität mit beliebigen fremden Codecs.

Ein blockierter Codec blieb unter laufendem Heartbeat aktiv und beendete sich
unter Windows und Wine ungefähr fünf Sekunden nach dessen Stillstand, ohne
Ergebnisbytes zu liefern (`tools/paintcraft_pdn_watchdog_check.py`). Dynamic Draw
v4.0 bestand auf beiden Plattformen das Laden der Original-DLL, die Konstruktion
ihres eigenen Dialogs und den Render-Einstieg. Interaktive Plugin-Bedienung und
Tablets sind damit noch nicht abgenommen.

Der native Pinseldialog importiert nun Dynamic-Draw-/Brush-Factory-JSON sowie
ältere DataContract-XML-Sammlungen. Unterstützte Werte umfassen Bildspitze,
Größe, Dichte, Farbe, Mischmodus, Rotation, Strichrichtung, exakten Byte-Fluss und
eine separate Deckungsgrenze. Sechs Originalspitzen werden mit MIT-Lizenz
eingebettet. Der Parser und die Preset-Oberfläche liegen getrennt in
`tools/brushes/presets.rs` und `preset_ui.rs`. Aktive nicht unterstützte oder
unbekannte Felder werden benannt; ein solches Preset verändert den aktuellen
Pinsel nicht. Das ist **teilweise Preset-Kompatibilität**, keine vollständige
Nachbildung der Brush-Factory-API. Einzelheiten:
[Kreativwerkzeuge](CREATIVE_TOOLS.md#brush-factory--dynamic-draw-presets).

Prüfungen des neuen Stands, getrennt von den älteren Zahlen weiter oben:

- Windows-, Linux- und skalarer WebAssembly-Build sowie Build-Lint bestanden.
- 5 Preset-Parser- und 2 Wine-Pfadtests bestanden.
- 14 Preset-Editorläufe auf Windows/Linux bestanden: JSON/XML, echte Exporte der
  originalen Dynamic-Draw-4.0.0.41103-Serializer, eingebettete Originalspitze,
  Flussaufbau, Deckungsgrenze, Undo/Redo und unveränderter Pinsel bei Ablehnung.
- 4 Linux/Wine-Editorfälle und 2 Watchdog-Lebenszyklustests bestanden.
- 16 bestehende Regressionen einschließlich Windows-DLL-Host und 2 bestehende
  Bildpinsel-Regressionen bestanden.

Das sind 45 gezielte Prüfungen; die früheren vollständigen PSD-/Text-/G’MIC-
Abnahmen werden nicht erneut hinzugezählt. Zusätzlich sind der direkte
Pastel-Pixelvergleich und die Originaldialog-Konstruktion in den lokalen
Wine-Testprotokollen festgehalten. Der Web-Build ist weiterhin keine interaktive
Browser-Abnahme. Ergebnisdateien liegen unter `build/paintcraft-next/`, insbesondere
`brush-preset-check/results.json`, `wine-check/results.json`, `watchdog-win.log`,
`watchdog-linux.log`, `remaining-check/results.json` und `creative-check/results.json`.

## Noch offen

- Vollständige Paint.NET-Bitmap-/GPU-API, Host-Services, fremde Speicher-Dialoge,
  Metadaten-/Ebenentransport über den DLL-Host sowie breitere interaktive Abnahmen
  fremder Plugins unter Windows und Wine.
- Vollständige Photoshop-Dokumenttreue: editierbare Masken/Gruppen/Texte/Smart Objects,
  alle Mischmethoden, CMYK/Lab und PSB-Ausgabe.
- Durchgängige 16-/32-Bit-Pixelbearbeitung und Speicherung; HDR-Tonemapping, HDR-Ausgabe
  und GPU-Berechnung. Der neue Importpfad konvertiert bewusst nach 8-Bit-SDR.
- Vollständige mehrlagige/HDR-G’MIC-Eingabe sowie die bisher nicht unterstützten
  Brush-Factory-Preset-Felder, vollständige Plugin-API-Kompatibilität und frei
  konfigurierbare Dynamikkurven.
- Vollständige visuelle Bidi-Navigation und Graphem-Bearbeitung, weitere Font-/Script-
  Abnahmen, echte IME-Kandidatensitzungen, X11-IME und Surrounding-Text-Rekonversion.
- Vollständige englische/französische Lokalisierung einschließlich Hilfetexten,
  Fehlermeldungen und gemeinsamer Dateidialoge; weitere Sprachen.
- Asynchrones Laden von Startdateien und im skalaren Web-Build.

Die Differenzenliste ist damit weiterhin **nicht auf null**. Die neuen Raster-,
Kreativ- und Textfunktionen sind nutzbare Erweiterungen mit dokumentierten Grenzen.
