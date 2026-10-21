# PaintCraft: Abgleich mit dem Paint.NET-Handbuch

> **Historischer Ausgangsbefund vor der Umsetzung.** Der unten beschriebene Stand
> wurde inzwischen erweitert. Den aktuellen, nachgetesteten Erledigungsstand und
> die noch offenen Grenzen beschreibt [MANUAL_PARITY.md](MANUAL_PARITY.md).

Stand: 12. Oktober 2026. Geprüft: **JFLib / PaintCraft 0.7**, Commit
`d3f8d682498d74ab54d59977ee7deb212b87e071` plus vorhandener Arbeitsstand.
Die PaintCraft-Quellen waren beim Prüfabschluss gegenüber diesem Commit unverändert.
jfguiLib wurde ausschließlich als Ablage der Referenzunterlagen gelesen.

## Ergebnis

Die Kernfunktionen für Rasterbearbeitung, Ebenen, Auswahl, Verlauf, Malwerkzeuge,
Bildoperationen und eingebaute Effekte sind weitgehend vorhanden. **74 Testläufe mit
bestehenden UI-Skripten bestanden** auf den frisch gebauten Windows- und Linux-Versionen, jeweils
mit `--verify-dirty`. Diese Tests belegen die geprüften Abläufe, keine vollständige
Paint.NET-Kompatibilität.

Bestätigte Lücken im Handbuchumfang sind die fehlende Unterscheidung zweier
Text-Rendering-Modi, das fehlende räumliche Kippen im Rotationszoom, Scannerimport,
Updateprüfung sowie mehrere Erweiterungsfunktionen und eine zusätzliche Form.
Der Info-/Diagnosetext beschreibt außerdem noch die ehemalige Assembly-Anwendung.
Ein Teil dieser Grenzen ist bereits in den bestehenden App-Dokumenten vermerkt;
die Liste ist deshalb keine Behauptung, alle Befunde seien neu entstanden.

## Referenzen und Prüftiefe

- Phillip Whitt, *Practical Paint.NET*, Ausgabe mit Copyright 2022, 342 PDF-Seiten.
  Lokales Exemplar: `../jfguiLib/booksAndContext/paintNET/Practical Paint.NET.pdf`
  relativ zum JFLib-Repository. Das Buch zeigt unter anderem Paint.NET 4.2.14
  und verwendet zusätzlich Drittanbieter-Plugins; es ist kein reines Verzeichnis
  der eingebauten Funktionen von Paint.NET 5.1.
- Ergänzend: `../jfguiLib/booksAndContext/paintNET/paintdotnet_feature_specification.md`.
  Diese Spezifikation orientiert sich an 5.1 und enthält 380 atomare Anforderungen.
  Sie wurde zur Einordnung zusätzlicher Lücken verwendet, nicht als vollständig
  abgenommene Anforderungsliste.
- Inhaltsverzeichnis, Funktionskapitel, die Abhängigkeiten aller 31 Tutorials und
  der Plugin-Anhang wurden gegen aktuelle Implementierung und Tests abgeglichen.
  Die Buchabbildung zum Rotationszoom auf gedruckter Seite 48 / PDF-Seite 66
  wurde zusätzlich visuell geprüft.
- Die 31 Tutorials wurden **nicht** mit allen Originalbildern vollständig
  nachgezeichnet. „Bausteine vorhanden“ bedeutet Quellcode- und Funktionstestbelege
  für die benötigten Funktionen, keine visuelle Gleichheit mit dem Buchresultat.
- Buchseiten und PDF-Seiten sind getrennt angegeben: Der Versatz ändert sich im
  Dokument. Die PDF-Seiten sind einsbasiert.

## Bestätigte Befunde

### PC-MAN-01: Zwei Text-Rendering-Modi haben dieselbe Wirkung

**Teilimplementierung; mittlere Priorität.** Das Buch nennt Glatt, Scharf (Modern)
und Scharf (Klassisch), gedruckte Seite 106 / PDF-Seite 123. PaintCraft bietet alle
drei an, setzt aber für beide scharfen Varianten denselben Schalter `FS_SHARP`.

Belege: [Optionsliste](../t_text.rs#L109),
[Fontauflösung](../t_text.rs#L783),
[bereits dokumentierte Einschränkung](TEXT.md#L73).
Es gibt keine nachfolgende Unterscheidung der beiden scharfen Varianten im Textwerkzeug.

**Reproduktion:** Eingebettete Schrift, 17 pt, gleicher Text und Anker, Commit und
Wechsel zum Pinsel. Die vollständigen PNGs für Modern/Klassisch sind auf Windows
und Linux bytegleich. Glatt erzeugt dagegen ein anderes PNG. Damit sind sowohl
der wirkungslose Wechsel als auch ein tatsächlich gerendertes Vergleichsbild belegt.

**Abnahme einer Korrektur:** Eigenständige klassische Metrik/Rasterisierung mit
passenden Pixeltests; alternativ bis dahin die zusätzliche Option verständlich
als nicht verfügbar kennzeichnen. Die Spezifikation `TEXT-011` verlangt nur zwei
Modi und ist dadurch weniger streng als die drei Optionen des Buches und der UI.

### PC-MAN-02: Rotationszoom kann nicht räumlich kippen

**Teilimplementierung; mittlere Priorität.** Abbildung 2-9 auf gedruckter Seite 48 /
PDF-Seite 66 zeigt drei Roll-/Rotationsparameter und die räumliche Steuerung.
PaintCraft zeigt zwar „Kippen / Rotieren“, besitzt dort aber nur einen Winkel.
Die Parameter enthalten Winkel, Zoom und Verschiebung; berechnet wird eine
zweidimensionale affine Transformation.

Belege: [Parameter](../dlg_rotzoom.rs#L75),
[affine Verarbeitung](../dlg_rotzoom.rs#L317),
[Dialog](../dlg_rotzoom.rs#L412). Dialogdump und Screenshot bestätigen die UI.
Normales Drehen, Zoomen, Verschieben, Kacheln und die Vorschau funktionieren;
`layer_rotzoom.uis` besteht auf beiden Plattformen.

**Abnahme einer Korrektur:** Zusätzliche Kippachsen mit perspektivischer Abbildung,
Vorschau, Auswahlbegrenzung, Abbrechen und einem gemeinsamen Undo-Schritt.

### PC-MAN-03: Die Plugin-basierten Buchabläufe sind nicht direkt verfügbar

**Kompatibilitäts- und Umfangslücke; mittlere Priorität, Architekturentscheidung.**
PaintCraft hat ein eigenes natives Plugin-System. Es erwartet `pn_plugin_query`
und PNAPI; es implementiert keinen Loader für die Paint.NET-Plugin-API.
Die Installation der im Buch genannten Paint.NET-DLLs ist deshalb kein verfügbarer
Weg, diese Funktionen in PaintCraft zu ergänzen.

Belege: [Loader-Vertrag](../plugins.rs#L1206),
[Rust-Plugin-Vertrag](../../../plugins/README.md#L1).
Das vorhandene Plugin-System wird hier ausdrücklich nicht als fehlend bewertet.

Folgende Tutorials benötigen konkrete, nicht mitgelieferte Erweiterungseffekte:

| Tutorial | Fehlende Buchfunktion | PDF-Seite | Abgrenzung |
| --- | --- | ---: | --- |
| 10 | BoltBait Color Balance | 176 | Temperatur/Tönung und Kurven sind vorhanden, aber nicht dieser Dialog und Parametersatz. |
| 11 | Black and White+ | 178 | Eingebautes Schwarzweiß ist vorhanden; die Plugin-Presets und Regler fehlen. |
| 27 | Dream | 303 | Kein entsprechender eingebauter Effekt registriert. |
| 29 | Oil Painting+ | 308 | Normales Ölgemälde ist vorhanden; das „+“-Plugin wird nicht mitgeliefert. |
| 30 | Pastel | 311 | Kein entsprechender eingebauter Effekt registriert. |

Vergleich: [Korrekturen](../../../lib/fx/adj.rs#L688),
[künstlerische Effekte](../../../lib/fx/artistic.rs#L140),
[Registrierung der Effektgruppen](../../../lib/fx/lib.rs#L543).

Weitere Erweiterungen aus Funktionskapiteln und Anhang sind ebenfalls nicht enthalten:
Feather/Bevel/Inner Shadow für Auswahl oder Objekte, Content Aware Fill, TR's
Dodge/Burn, TR's Portrait Retouch, G'MIC-Qt und dynamische Pinsel aus Brush Factory.
Ein vorhandener ähnlicher Effekt, etwa Porträtweichzeichnung oder Schlagschatten,
ist kein Nachweis für den jeweiligen Plugin-Ablauf.

**PSD:** Das Buch führt Photoshop-Dateien ausdrücklich über ein Plugin ein
(gedruckte Seite 314 / PDF-Seite 327). Ein PSD-Codec ist in
[der vollständigen Codec-Registrierung](../../../lib/img_all/lib.rs#L25) nicht
enthalten. PDN mit Ebenen ist vorhanden und wurde getestet; PSD-Kompatibilität
folgt daraus nicht.

**Nächster Schritt:** Entscheiden, welche konkreten Funktionen nativ nachgebaut
werden sollen. Eine Kompatibilitätsschicht für fremde Paint.NET-Plugins wäre ein
eigenes größeres Vorhaben; der native PNAPI-Vertrag allein reicht dafür nicht aus.

### PC-MAN-04: Dem Formenbestand fehlt die Smartphone-Vorlage aus Tutorial 23

**Fehlendes Asset; niedrige Priorität.** Das Tutorial verlangt die Form „Smart Phone“
(gedruckte Seiten 244-245 / PDF-Seiten 259-260). Sie gehört nicht zu den
[29 eingebauten Formen](../shapes_lib.rs#L788) und wird im geprüften App-/Pluginbestand
nicht mitgeliefert. Die spätere Blitzform ist hingegen als „Blitz“ vorhanden.

Der [XAML-Import für eigene Formen](../shapes_lib.rs#L1) existiert und
`tools_shapes.uis` besteht. Das ist eine fehlende Vorlage, kein fehlendes Formenwerkzeug.
Eine passende, selbst erstellte XAML-Form wäre ein kleinerer Lösungsweg als eine
DLL-Kompatibilitätsschicht.

### PC-MAN-05: Scanner-/Kameraimport bleibt deaktiviert

**Fehlende Funktion; niedrige Priorität, bisher bewusst ausgeschlossen.**
Das Buch beschreibt „Acquire“ auf gedruckter Seite 12 / PDF-Seite 30.
Der Befehl wird dauerhaft deaktiviert registriert und hat keinen Implementierungspfad.

Belege: [Befehl](../cmds.rs#L29),
[explizite Umfangsentscheidung](../consts.rs#L33).
Ein gezielter UI-Test bestätigt den deaktivierten Menüeintrag.
Eine Umsetzung würde eine passende Plattformanbindung für die Bilderfassung benötigen.

### PC-MAN-06: Updateprüfung und Benutzerhilfe sind nicht nutzbar

**Fehlende Produktfunktionen; niedrige Priorität.**
Die Updatefunktion aus gedruckter Seite 31 / PDF-Seite 49 ist als Einstellungsseite
vorhanden, aber automatische Prüfung, Betaprüfung und „Jetzt prüfen“ sind deaktiviert.
Die Anwendung erklärt diese Einschränkung im Dialog.

Beleg: [Aktualisierungsseite](../dlg_settings.rs#L524).
Die deaktivierten Bedienelemente wurden im UI-Test bestätigt.

Zusätzlich sind Dokumentation/F1, Lernprogramme und weitere Hilfelinks deaktiviert:
[Befehlstabelle](../cmds.rs#L175). Der
[historische Grund](../dlg_misc.rs#L4) war, nicht auf die Seiten des fremden Produkts
zu verweisen. Mit JFLib als eigenem Projekt fehlt nun eine passende eigene
Hilfeseite beziehungsweise lokale Hilfe. Das Hilfe-Menü und „Info“ selbst funktionieren.

### PC-MAN-07: Info und kopierbare Diagnose nennen noch Assembly und jfguiLib

**Bestätigter UI-/Dokumentationsfehler; niedrige Priorität.**
Im tatsächlich gerenderten Infofenster steht weiterhin, PaintCraft sei vollständig
in x86-64-Assembler auf jfguiLib geschrieben. Die kopierbare Diagnose wiederholt
dies. Das beschreibt nicht den geprüften Rust-/JFLib-Stand.

Belege: [Info-Text](../dlg_misc.rs#L74),
[Diagnose-Text](../dlg_settings.rs#L646). Screenshot und UI-Dump bestätigen den
Info-Text; der kopierbare Diagnose-Text wurde im Quellcode geprüft.
Die Plattformkennung des Infofensters unterscheidet zudem nur Windows/sonstige
Plattformen; die Folgen für Browserausgaben wurden in diesem Lauf nicht getestet.

## Zusätzliche Lücken gegenüber der 5.1-orientierten Spezifikation

Diese Punkte sind nicht automatisch fehlende Funktionen der Buch-Tutorials.
Sie sind getrennt aufgeführt, damit der größere Sollumfang nicht mit dem Buch
oder mit einem nachgewiesenen Laufzeitfehler verwechselt wird.

| Bereich | Bestätigter Stand | Anforderung / Beleg |
| --- | --- | --- |
| Konfigurierbare Arbeitsfläche | Hintergrund und Schachbrettfarben sind Konstanten, Schatten wird bei geöffnetem Dokument immer gezeichnet. Es gibt keine entsprechenden Benutzeroptionen. | `CANVAS-011..013`; [Farben](../view.rs#L70), [Schatten](../view.rs#L704), [persistierte Ansichtseinstellungen](../settings.rs#L419). |
| Neues Bild mit transparentem/farbigem Hintergrund | Die interne Dokumentfunktion nimmt eine Hintergrundfarbe an, der Neu-Dialog übergibt fest Weiß und bietet keine Farbauswahl. Nachträgliches Löschen/Füllen bleibt möglich. | `DOC-010/011`; [Dialogaktion](../dlg_new.rs#L344). Das Buch verlangt hier nur ein weißes Standardbild. |
| Vollständige Unicode-Typografie | UTF-8, Systemfonts und Color-Fonts existieren. Automatischer Font-Fallback, GSUB/komplexe Schriftformung fehlen; fehlende Zeichen landen im Ersatzglyph. | `TEXT-017`; [codepunktweise Ausgabe](../../../lib/text/lib.rs#L228), [Glyphwahl](../../../lib/text/registry.rs#L223), [bekannte Grenzen](TEXT.md#L82). Keine neue internationale Schrift-Testmatrix in diesem Lauf. |
| Feine Textbedienung | Schriftgrößen sind ganzzahlig; Wortnavigation per Strg+Pfeil und eine IME-Kompositionsanzeige fehlen laut Implementierung/Dokumentation. | [Größenbereich](../t_text.rs#L104), [bestehende Einschränkungen](TEXT.md#L73). Die Aussage „Paint.NET hat keine Textselektion“ dort ist als unbestätigt markiert und wurde hier nicht als Befund übernommen. |
| GPU / erweiterte Displayausgabe | CPU/SIMD und mehrere Threads sind vorhanden. Keine GPU-Auswahl/-Berechnung, kein HDR-Ausgabepfad. ICC-Zuweisung, Konvertierung und Monitorprofile dürfen damit nicht gleichgesetzt werden: deren vorhandene Tests bestehen. | `PERF-003/004`, Inventur 2.12; [Grafikseite](../dlg_settings.rs#L489), [HDR-Hinweis](../dlg_settings.rs#L516). Bewusste bisherige Architekturgrenze. |
| Weitere UI-Sprachen | Nur Deutsch; Sprachwahl deaktiviert. | [UI-Einstellungen](../dlg_settings.rs#L365). Bereits als allgemeines Vorhaben in `Todo.md` vorhanden. |
| Asynchrones Öffnen großer Dateien | Der reguläre Öffnen-Pfad liest und dekodiert direkt im Aufrufer; dort kein eigener Fortschritts-/Abbruchdialog. Große Dateien wurden dafür nicht neu zeitlich vermessen. | `CORE-019/020`, `PERF-011`; [Öffnen](../fileio.rs#L411). Statischer Architekturhinweis, kein gemessener Hang. |

## Zuordnung aller Buch-Tutorials

„Vorhanden“ bezieht sich auf benötigte Bausteine. Bei externen Plugins bedeutet
„fehlt“ fehlende direkte Nachvollziehbarkeit des Buchablaufs, nicht zwangsläufig,
dass sich kein ähnliches Bild mit anderen Arbeitsschritten erzeugen lässt.

| Nr. | Thema, sinngemäß | PDF-Start | Ergebnis |
| ---: | --- | ---: | --- |
| 1 | Belichtung automatisch korrigieren | 133 | Vorhanden: Auto-Level. |
| 2 | Helligkeit und Kontrast | 135 | Vorhanden. |
| 3 | Kontrast mit Kurven korrigieren | 142 | Vorhanden. |
| 4 | Kleine Tonwertkorrektur mit Kurven | 144 | Vorhanden. |
| 5 | Unterbelichtung mit Levels korrigieren | 150 | Vorhanden. |
| 6 | Helle Bildbereiche abdunkeln | 152 | Vorhanden: Overlay-Ebene, weicher Pinsel; Buchwiderspruch siehe unten. |
| 7 | Unterschiedlich belichtete Bereiche korrigieren | 158 | Vorhanden: Overlay-Ebene, Deckkraft und Pinsel. |
| 8 | Verblasste Farben mit Levels auffrischen | 166 | Vorhanden. |
| 9 | Farbstich mit Kurven und Verlauf korrigieren | 170 | Vorhanden. |
| 10 | Color Balance aus dem Plugin-Pack | 176 | Fehlt als entsprechender Plugin-Dialog, PC-MAN-03. |
| 11 | Schwarzweiß-Konvertierung mit Black and White+ | 178 | Erweiterte Plugin-Funktion fehlt, PC-MAN-03. |
| 12 | Sepia | 183 | Vorhanden. |
| 13 | Schwarzweißfoto kolorieren | 186 | Vorhanden: Ebenen, Farbe, Pinsel und Blend-Modi. |
| 14 | Störende Objekte mit dem Kopierstempel entfernen | 199 | Vorhanden. |
| 15 | Foto gerade ausrichten | 203 | Verwendete 2D-Rotation, Zoom, Zuschnitt und Klonen vorhanden; PC-MAN-02 betrifft darüber hinausgehendes Kippen. |
| 16 | Zähne aufhellen | 211 | Vorhandene Auswahl-/Korrektur-/Pinselbausteine. |
| 17 | Hautunreinheiten retuschieren | 215 | Vorhanden: Kopierstempel. Kein Healing-Werkzeug als Pflicht angenommen. |
| 18 | Kratzer und Schäden reparieren | 219 | Vorhanden: Kopierstempel und Ebenen. |
| 19 | Elemente in ein Bild montieren | 226 | Vorhanden: Import, Auswahl, Transformieren und Ebenen. |
| 20 | Hintergrund austauschen | 235 | Vorhandene Auswahl-, Ebenen- und Retuschebausteine. |
| 21 | Einfache Skizze zeichnen | 246 | Vorhanden: Stift/Pinsel, Linien und Farben. |
| 22 | Herz auf strukturiertem Hintergrund | 252 | Herz, Füllungen und verwendete Zeichenbausteine vorhanden. |
| 23 | Smartphone zeichnen | 258 | Mitgelieferte Smartphone-Form fehlt, PC-MAN-04; übrige Zeichenbausteine vorhanden. |
| 24 | Sonnenuntergang illustrieren | 270 | Vorhandene Verläufe, Formen, Auswahl und Ebenen. |
| 25 | Kleeblatt zeichnen | 281 | Vorhandene Formen, Linien, Ebenen und 2D-Transformation. |
| 26 | Marienkäfer zeichnen | 290 | Vorhandene Zeichen- und Ebenenbausteine. |
| 27 | Dream-Effekt | 303 | Plugin-Effekt fehlt, PC-MAN-03. |
| 28 | Tuschezeichnung | 306 | Vorhanden: Ink Sketch. |
| 29 | Oil Painting+ | 308 | „+“-Plugin fehlt; eingebautes Ölgemälde vorhanden, PC-MAN-03. |
| 30 | Pastell-Effekt | 311 | Plugin-Effekt fehlt, PC-MAN-03. |
| 31 | Bleistiftskizze | 312 | Vorhanden: Pencil Sketch. |

### Widerspruch im Buch: Soft Light / Overlay

Die Einleitungen von Tutorial 6 und 7 sagen „Soft Light“. Die konkreten Schritte
verlangen jedoch **Overlay**, ebenso die dazugehörige Ebenenabbildung
(gedruckte Seiten 136-137 und 142 / PDF-Seiten 152-153 und 158).
Overlay ist in [PaintCrafts Ebenenmodi](../dlg_layerprops.rs#L32) implementiert.
Ein fehlender Soft-Light-Modus wird deshalb nicht als Blocker dieser Tutorials gezählt.

## Ausgeführte Prüfungen und Reproduktion

Im Repository-Root:

```text
python build_rs.py app PaintCraft --target both --out build/paintcraft-manual-audit -j 6
python build/paintcraft-manual-audit/run_baseline.py
python build/paintcraft-manual-audit/check_features.py
```

| Prüfung | Ergebnis |
| --- | --- |
| Frische Builds für Windows und Linux | Beide erfolgreich; `jxlenc` jeweils mitgebaut. |
| Bestehende Windows-UI-Skripte | 59 bestanden, 0 fehlgeschlagen. |
| Ausgewählte Linux-UI-Skripte unter Ubuntu 24.04/WSL | 15 bestanden, 0 fehlgeschlagen. |
| Native Plugin-Fixture-Skripte | `plugins.uis` und `plugins_crash.uis` ausgelassen; separate Fixtures nicht aufgebaut. |
| Textmodi | Je drei Renderläufe auf Windows und Linux; Modern/Klassisch identisch, Glatt abweichend. |
| Zusätzliche UI-Inspektion | Scanner, Hilfe und Updatezustände bestätigt; Rotationszoom und Info als Dump/Screenshot gesichert. |

Alle ausgeführten UI-Skripte nutzten `--verify-dirty`. Die Linux-Auswahl umfasst
`colorprofile`, `docs`, `files`, `fx_all`, `history`, `info`, `jxlmod`,
`layer_import_pdn`, `layer_rotzoom`, `layers_basic`, `sel_move`, `settings`,
`skeleton`, `tools_brush` und `tools_text`.

Lokale, git-ignorierte Nachweise:

- `build/paintcraft-manual-audit/baseline-results.json`: Ergebnisse und Ausgaben.
- `build/paintcraft-manual-audit/feature-results.json`: Textmodi-Hashes und UI-Test.
- `build/paintcraft-manual-audit/source-manifest.json`: Hashes von 175 geprüften
  beziehungsweise relevanten Quelldateien und beiden App-Binaries, beim Prüfabschluss erfasst.
- `build/paintcraft-manual-audit/text-{win,linux}-{smooth,modern,classic}.png`.
- `build/paintcraft-manual-audit/{rotzoom,about}.{png,dump}`.
- Die beiden Python-Treiber und erzeugten `.uis`-Skripte liegen im selben Ordner.

Keine neue Prüfung gegen echte Scanner/Drucker, fremde Plugin-Binaries,
HDR-Monitore oder alle Browser. `print.uis` prüft den Testpfad; es wurde kein
Druckauftrag an ein reales Gerät gesendet. Vorhandene Decoder-Sicherheitstests
wurden nicht als neuer Handbuchtest wiederholt. Am Produktquellcode wurde für
diese Prüfung nichts geändert.

## Empfohlene Reihenfolge

1. Die UI-Versprechen bereinigen: klassische Textdarstellung und Kippen entweder
   vollständig implementieren oder die derzeitige Einschränkung sichtbar machen.
2. Info/Diagnose auf Rust und JFLib korrigieren und eine eigene Hilfe verlinken.
3. Die Smartphone-Form ergänzen, wenn alle Buchübungen ohne zusätzliche Assets
   nachvollziehbar sein sollen.
4. Für die fünf fehlenden Effekt-Tutorials native Entsprechungen priorisieren;
   PSD, Content Aware Fill, Auswahl-Federung und dynamische Pinsel als separate
   Funktionspakete bewerten.
5. Die darüber hinausgehenden Spezifikationswünsche für Arbeitsflächenoptionen,
   Typografie, Updates, Scanner und GPU/HDR gesondert planen.
