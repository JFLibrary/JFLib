# G’MIC-Qt, Bildpinsel und Dynamik

Stand: 12. Oktober 2026. Diese Funktionen gehören zur nativen PaintCraft-Anwendung
im JFLib-Repository. Sie ersetzen keine vollständige Paint.NET-Plugin-API.

## G’MIC-Qt einrichten

1. Die **Standalone-Version** von [G’MIC-Qt](https://gmic.eu/download.html)
   installieren beziehungsweise vollständig entpacken, einschließlich ihrer Qt-
   Bibliotheken und Plugins. Das GIMP-Plugin allein ist nicht ausreichend.
2. `gmic_qt.exe` unter Windows oder `gmic_qt` unter Linux in den Suchpfad aufnehmen,
   oder den vollständigen Pfad beim Start von PaintCraft übergeben:

   ```powershell
   .\build\win\PaintCraft.exe --gmic-qt "C:\Tools\gmic-qt\gmic_qt.exe"
   ```

   ```sh
   ./build/linux/PaintCraft --gmic-qt /opt/gmic/bin/gmic_qt
   ```

   Alternativ gilt die Umgebungsvariable `PAINTCRAFT_GMIC_QT`. Ein Linux-Wrapper
   kann bei einer portablen Installation deren Qt-/Bibliothekspfade setzen.
3. Ein Bild öffnen und **Effekte > G’MIC-Qt** wählen. Der Filterdialog erscheint
   als eigenes Qt-Fenster. PaintCraft wartet mit einer Abbrechen-Schaltfläche.

PaintCraft übergibt die aktive Ebene als PNG. G’MIC verwendet seinen regulären
[Standalone-Dateivertrag](https://github.com/c-koi/gmic-qt/blob/master/STANDALONE.md)
mit `--output output-%l.png`. Ein einzelnes gleich großes Ergebnis wird durch die
aktuelle Auswahlmaske übernommen und erzeugt einen Undo-Schritt. Mehrere Ergebnisse
oder geänderte Abmessungen öffnen ein neues Dokument; `%l` wird von G’MICs Reihenfolge
„oben zuerst“ in PaintCrafts Ebenenreihenfolge übersetzt. Kleinere Ergebnisbilder
werden transparent auf die gemeinsame Leinwand aufgefüllt.

Abbruch beendet den Kindprozess einschließlich seiner Prozessgruppe. Ein fehlendes
Programm, fehlerhafter Exit oder ungültige Ergebnisdateien ändern das Quelldokument
nicht. Die aktive Ebene und deren Dokumentversion werden vor der Übernahme geprüft.
PaintCraft löscht anschließend nur die eigenen Austauschdateien im temporären
Auftragsordner. Maximal 256 Ergebnisse und insgesamt 100 Millionen Ergebnis-Pixel
sind erlaubt; eine einzelne Datei darf höchstens 512 MB groß sein.

Die aktuelle Brücke transportiert **8-Bit-Pixel einer aktiven Ebene**, keine
vollständige mehrlagige G’MIC-Eingabe, ICC-Metadaten oder HDR-Fließkommadaten.
G’MIC ist eine separate Installation und wird nicht mit PaintCraft ausgeliefert.
`--gmic-command "…"` übergibt für Automatisierung zusätzlich `--command` und
`--apply`; reguläre Benutzer benötigen diese Option nicht.

## Bildspitzen und Pinselparameter

Beim Farbpinsel und Radierer öffnet das Pinselsymbol neben den Größenreglern den
Dialog **Pinselspitze und Dynamik**. Eine PNG-, BMP- oder TIFF-Datei kann als Spitze
geladen werden. Enthält sie Transparenz, bildet der Alphakanal die Deckung. Ein
vollständig deckendes Bild verwendet die umgekehrte Helligkeit: Schwarz zeichnet,
Weiß bleibt frei. Die Grenze beträgt 4.194.304 Pixel und 64 MB dekodierten Speicher.
Das Seitenverhältnis bleibt beim Stempeln erhalten; die Spitze wird bilinear abgetastet.

| Einstellung | Bereich / Verhalten |
| --- | --- |
| Drehwinkel | 0–359 Grad |
| Streuung | 0–200 % der Pinselbreite |
| Größenvariation | 0–95 % |
| Winkelvariation | bis ±180 Grad je Abdruck |
| Deckkraft pro Abdruck | 0–100 % |
| Bewegungsrichtung | Spitze zusätzlich am Strichverlauf ausrichten |
| Farbfluss aufbauen | Überlappende Abdrücke akkumulieren statt nur das Maximum zu behalten |
| Maximale Strichdeckung | Deckung auf 0–255 begrenzen; getrennt vom Fluss |

Größe, Abstand, Härte der runden Spitze, Farbe, Auswahl, Füllmethode und Stiftdruck
bleiben Teil der vorhandenen Pinselbedienung. Eigene Bildspitzen liefern ihre
Kantenweichheit über ihre Pixel. **Runde Spitze / Zurücksetzen** entfernt die
Bildspitze und setzt die Dynamik zurück. Pfad und Parameter werden beim Schließen
des Dialogs in `[BrushTip]` gespeichert. Eine fehlgeschlagene neue Dateiauswahl
ersetzt die bisherige Spitze nicht. `--brush-tip PFAD` lädt eine Spitze beim Start.

Ein Strich bleibt ein Undo-Schritt. Die Streuung verwendet eine reproduzierbare
Zufallsfolge pro Strich. Ohne **Farbfluss aufbauen** wird wie bisher das Maximum
gesammelt. Mit dieser Option erhöht jeder überlappende Abdruck die Deckung bis
zur eingestellten Grenze. Ein stillstehender Stift erzeugt keine zeitgesteuerten
Airbrush-Abdrücke. Richtung, Neigung, Geschwindigkeit und Farbe haben keine frei
konfigurierbare Kurven-/Dynamikmatrix. ABR/GBR werden nicht geladen.

## Brush Factory / Dynamic Draw Presets

Im Pinseldialog **Brush-Factory-Presets importieren…** wählen, eine UTF-8-JSON-
oder XML-Datei öffnen, einen Eintrag auswählen und **Preset anwenden** drücken.
Unterstützt wird die `CustomBrushes`-Sammlung des originalen
[Dynamic Draw v4.0](https://github.com/NinthDesertDude/Dynamic-Draw), des Nachfolgers
von Brush Factory, sowie dessen älteres DataContract-XML-Format. Die Datei darf
höchstens 4 MiB und 1024 Presets enthalten. Doppelte Schlüssel, ungültige Werte,
DTD und externe XML-Entitäten werden zurückgewiesen.

Übernommen werden Bildspitze, Größe, manuelle/automatische Dichte, RGB-Farbe,
15 Mischmodi, Rotation, Ausrichtung am Strich, Byte-Fluss und maximale
Strichdeckung. Überlappende Abdrücke akkumulieren; `BrushOpacity` begrenzt die
aufgebaute Deckung, während `BrushFlow` jeden einzelnen Abdruck bestimmt.
Die Dichte wird für den Strich als Bruchwert behalten. Bei einer späteren Änderung
des nativen Abstandreglers gilt wieder dessen Prozentwert.

Die sechs Originalspitzen **Circle Light, Circle Hard, Circle Med, Rough,
Segmented und Sketchy** sind unverändert eingebettet. Ihre MIT-Lizenz ist im
Pinseldialog und unter `resources/brush-factory/` enthalten. Andere Bildpfade
werden relativ zur Preset-Datei aufgelöst; absolute Pfade müssen auf dem Zielsystem
existieren. Windows-Pfade zu eigenen Bildern müssen nach einem Wechsel zu Linux
in der Preset-Datei angepasst werden.

Unbekannte Parameter oder aktive, noch nicht unterstützte Einstellungen werden
pro Preset angezeigt und verhindern dessen Anwendung. Dazu gehören unter anderem
Farbjitter, gesperrte Farbkanäle, Symmetrie, nahtloses Zeichnen, Effektpinsel und
beliebige Druckkurven. Ein fehlendes Bild oder ein nicht anwendbares Preset verändert
den aktiven Pinsel nicht. Tastenkürzel, Palette und allgemeine Plugin-Voreinstellungen
aus derselben Datei werden nicht importiert. Der native Rasterizer ist keine
pixelidentische Nachbildung sämtlicher Brush-Factory-Dynamiken.

Automatisierung:

```text
PaintCraft --brush-presets DynamicDrawSettings.json --brush-preset "Red ink"
```

Ohne `--brush-preset` wird nur die Bibliothek geladen. Für die unveränderte
Plugin-DLL steht außerdem der [Paint.NET-Host](../../../tools/paintcraft-pdn-host/README.md)
zur Verfügung. Dynamic Draw v4.0 wurde auf Windows und Linux/Wine geladen und mit
seinem Originaldialog konstruiert; interaktives Zeichnen, Tablets und alle
Plugin-Funktionen sind damit noch nicht vollständig abgenommen.

## Tests

```text
python tools/paintcraft_creative_check.py --out build/paintcraft-next --target both --gmic
python tools/paintcraft_brush_preset_check.py --out build/paintcraft-next --target both
```

Mit `--original-plugin /pfad/zu/DynamicDraw.dll` erzeugt der Preset-Test zusätzlich
JSON und XML mit den originalen v4.0-Serializer-Typen der DLL. Dazu benötigt der
Testexporter Windows/.NET; die erzeugten Dateien werden anschließend von beiden
nativen PaintCraft-Versionen eingelesen und zum Zeichnen benutzt. Die Original-DLL
wird weder verändert noch ins Repository kopiert.

Der Test erwartet die separat bereitgestellte Standalone-Version 4.0.5 im ignorierten
`build/paintcraft-next/gmic-runtime/`: Windows unter
`win/gmic-4.0.5-qt-win64/gmic_qt.exe`, Linux über `gmic-qt-linux`. Der Linux-Wrapper
setzt bei der portablen Testinstallation `LD_LIBRARY_PATH`, `QT_PLUGIN_PATH` und
`QT_QPA_PLATFORM=offscreen`. Ohne `--gmic` werden nur die Bildpinsel geprüft.

Zwölf Desktop-Läufe prüfen Bildspitze, 180-Grad-Drehung und Undo/Redo sowie echte
G’MIC-Filterausführung, Auswahlbegrenzung, Mehrfachausgabe, Abbruch und Startfehler.
Die Filtertests verwenden den automatischen Standalone-Pfad; eine interaktive
Abnahme sämtlicher Qt-Filterdialoge oder G’MIC-Filter ist damit nicht nachgewiesen.
