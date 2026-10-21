# jfOffice: Abgleich mit der WordCraft-Referenz

Stand: 11. Oktober 2026. Diese Designetappe entwickelt den vorhandenen jfWord-Code
weiter. Fensterkopf, Menüband, Navigation, Lineale, Status und Dateioberfläche sind
an die vom Nutzer bereitgestellten WordCraft-Bilder angepasst. Die Implementierung
verwendet jfguiLib; fremder App-, egui- oder Widgetcode wird nicht eingebunden.

Die Referenzbilder unter `context/wordcraft/docs/images` haben 2.880 × 1.800 Pixel:
1.440 × 900 DIP bei 200 Prozent, einschließlich eines schwarzen 8-DIP-Außenrahmens.
Die vergleichbare App-Fläche beträgt 1.424 × 884 DIP. Der schwarze Rahmen gehört
nicht zum jfOffice-Style. Die Screenshots sind eine Gestaltungsreferenz und keine
Anweisung, die darin sichtbaren Dokumente oder fremden Marken zu übernehmen.

## Schrift- und Rasterabgleich

Der aktuelle Build liegt unter `build/wordcraft-type/{win,linux,web}`. Als Vorher-Zustand
dient `build/wordcraft-align`. Die neue Schriftprüfung vergleicht identische englische
Beschriftungen mit den Originalausschnitten bei **200 % ohne Größenänderung**. Dafür gibt
es eine isolierte Vergleichsfläche in `officeicontest`; jfWord behält seine deutschen
Beschriftungen. Text-/Hintergrundfarben werden auf Deckungsgrad normiert und Konturen um
höchstens zwei ganze Pixel ausgerichtet. Das misst Konturen und Zeichenabstände,
nicht die Position sämtlicher App-Elemente oder eine komplette Oberfläche.

### Änderungen in der Library und im Style

- `Text::new` akzeptiert eingebettete TrueType/OpenType- und SGF-Schriften gemeinsam.
  jfOffice lädt die drei unveränderten Inter-TTFs. Ihre GPOS-Paarabstände werden vom
  vorhandenen TrueType-Parser ausgewertet. Die bisherige SGF-Konvertierung hatte diese
  Abstände verloren, weil Inter keine ältere `kern`-Tabelle mitbringt.
- `Style::font_raster` aktiviert `FontRaster::Natural` pro UI-Schrift. Die Konturen behalten
  ihre natürliche Skalierung; horizontale Positionen werden zur nächsten Viertelpixelposition
  gerundet. Profilwechsel invalidieren Glyphmasken, aber keine Text- oder Caret-Metriken.
  Andere Styles und nachgeladene Dokumentfonts behalten das bisherige Rasterprofil.
- `Shapes::panel_fx` und `Ui::cmd_panel_fx` unterstützen Geometrie in 24.8 Pixeln. Füllung
  und Innenrahmen werden aus ihren Flächenabdeckungen gemeinsam zusammengesetzt, damit
  keine mehrfach geglätteten Säume entstehen. Temporäre Masken sind auf 64-Zeilen-Bänder
  begrenzt; Clipping, Ursprung, Deckkraft und A8-/PARGB-Ausgabe werden geprüft.
- `Path::set_tolerance` erlaubt feinere Kurven für diese kleinen Rundungen. Der vorhandene
  Standard von 0,2 Pixeln bleibt für andere Pfade, einschließlich Schriftkonturen, erhalten.
- Die Vorschaukarten beginnen vertikal auf einer halben DIP-Position und verwenden einen
  Außenradius von 2,5 DIP. Das entspricht einem 2-DIP-Radius auf der Mittellinie des normalen
  1-DIP-Rahmens. Die Inhaltsbegrenzung rundet nach innen; die Vorschau übermalt auch bei
  Hover keine Rahmenpixel und ändert dabei nicht ihre Skalierung.

### Direkte Messung

| Schriftprobe | Thema | Deckungsgradabweichung vorher | Jetzt |
| --- | --- | ---: | ---: |
| Navigation | hell | 231,39 | 127,91 |
| Layout | hell | 141,53 | 43,46 |
| Layout | dunkel | 155,12 | 74,05 |
| Design | hell | 82,07 | 47,46 |
| Design | dunkel | 102,16 | 84,49 |
| AutoSave | hell | 117,24 | 76,21 |
| AutoSave | dunkel | 147,15 | 119,35 |
| Clipboard | hell | 95,20 | 48,87 |
| Good morning | hell | 469,58 | 180,61 |
| Blank document | hell | 131,14 | 86,98 |

Alle zehn Proben verbessern sich. Die summierte absolute Deckungsgradabweichung sinkt
von **1.672,58 auf 889,38**, um rund **47 %**. Die Zuordnung der Schriftgewichte, Größen,
Originalausschnitte und Bildprüfsummen steht in `build/wordcraft-type/typography/results.json`.
Die selbstständige `typography/comparison.html` stellt Referenz, Vorher und Jetzt nebeneinander
dar; eine Vergrößerung dient nur der Ansicht, nicht der Messung.

| Vorlagenrahmen bei nativen 200 % | Verglichene Pixel | Abweichungen vorher | Jetzt |
| --- | ---: | ---: | ---: |
| Gerade Kanten, ohne Rundungen | 1.328 | 288 | **0** |
| Vollständiges Rahmenband einschließlich Rundungen | 16.888 | 1.220 | **20** |

Die 20 verbleibenden Pixel liegen an den vier Rundungen und unterscheiden sich um maximal
vier Helligkeitsstufen je Farbkanal. Der summierte RGB-Fehler dieses Rahmenbandes sinkt
von 185.880 auf 120. Ausschnitte und Messwerte liegen unter `reference/template-200-*.png`
und `reference-comparison.json`. Der ältere Vergleich mit auf 100 % verkleinerter Referenz
bleibt zusätzlich erhalten: Seine 142 verbleibenden Abweichungen an der oberen Rahmenkante
sind jeweils eine Helligkeitsstufe; das Abrunden beim Verkleinern ist keine native 100-%-Referenz.

**Verbleibende Grenzen:** Eine vollständige Pixelidentität der Schrift besteht weiterhin
nicht. Kontur-Hinting und komplexe Schriftformung sind eigenständige Aufgaben der Text-Library.
Die Messung bestätigt ausgewählte vergleichbare Bereiche und Proben; übersetzte Texte,
Produktkennung, unterschiedliche Dokumentinhalte und noch fehlende Funktionen bleiben
außerhalb des pixelweisen Vergleichs. Die Word-Dateikompatibilität wird dadurch nicht erweitert.

### Prüfung und Reproduktion

- Windows, Linux und WebAssembly gebaut; 1.025 native Prozessprüfungen bestanden,
  einschließlich 37 UI-Skripten je Plattform. Der abschließende Build besteht jeweils
  9.356 Kernprüfungen; Hover, Fokus, Theme- und DPI-Wechsel der neuen Karten werden mit
  `fractional_panels.uis` und vollständigem Vergleich der Teilrepaints erneut geprüft.
- 22 Raster-/Fontprüfungen je nativer Plattform: GPOS, Cachewechsel, unveränderte
  Caret-Metriken, wiederverwendete Fontslots, Clip/Ursprung, Deckkraft, A8 und nahtlose
  Maskenbänder bei transparenten Farben und versetzten Teilrepaints.
- Zwölf native Schriftaufnahmen in hell/dunkel bei 100/150/200 %, mit sechs bytegleichen
  Windows-/Linux-Vergleichen. Weitere zwölf Vergleiche in Chromium und Firefox liefern
  dieselben Pixel innerhalb der semantisch bestimmten Schriftvergleichsfläche; native
  Betriebssystemtitel und der zusätzliche Browser-Fensterkopf sind nicht Teil dieser Fläche.
- 66 Aufnahmen aller Register und der Dateioberfläche ohne Dirty-Render-Abweichung.
  48 Dokumentbilder bleiben zwischen UI-Zuständen und Plattformen unverändert und stimmen
  einschließlich Geometrie/Pixelprüfsumme mit `wordcraft-align` überein.
- Sechs Browser-App-Fälle plus acht Speicherfehler-/Konkurrenzfälle bestehen. Vier
  Browser/native-DOCX-Übertragungen sind jeweils bytegleich mit 5.038 Bytes.
- Vier Gallery-Bildvergleiche des Standardstyles sind unverändert; Quellresolver,
  Repository-Lint und API-Generierung bestehen.

```powershell
python build_rs.py app jfWord --target all --out build/wordcraft-type -j 6
python build_rs.py app officeicontest --target all --out build/wordcraft-type -j 6
python apps/jfWord/tests/typography_reference.py --build-dir build/wordcraft-type --target both
node apps/officeicontest/tests/browser.mjs build/wordcraft-type
python apps/jfWord/tests/design_reference.py --build-dir build/wordcraft-type --before build/wordcraft-align/design-matrix
python apps/jfWord/tests/check.py --target both --build-dir build/wordcraft-type
python apps/jfWord/tests/visual.py --out build/wordcraft-type
node tools/tests/web_apps.mjs build/wordcraft-type --engine=chromium,firefox --only=jfWord
python tools/tests/web_transfer.py build/wordcraft-type
```

Die Schriftmessung verwendet NumPy zusätzlich zu den vorhandenen PNG-Werkzeugen;
die Browserprüfung verwendet dieselbe lokale Playwright-/Browser-Installation wie die
übrigen Web-Tests. Builds, Prüfsummen und Ergebnisdateien sind in
`build/wordcraft-type/VALIDATION.json` zusammengefasst.

## Vorherige Nachkorrektur anhand der tatsächlichen PNGs

Der damalige Build liegt unter `build/wordcraft-align/{win,linux,web}`. Die vorherige
Etappe `build/wordcraft-design` bleibt als Vergleich erhalten. Der neue reproduzierbare
Test `tests/design_reference.py` entfernt ausschließlich den schwarzen Außenrahmen
der Originalbilder und normiert die Referenz auf 1.424 × 884 Pixel bei 100 Prozent.
Er vergleicht gleiche UI-Bereiche; fremde Dokumentinhalte, englische Beschriftungen
und Marken sind nicht Teil dieses pixelweisen Vergleichs.

Korrigiert wurden der versehentliche 10-DIP-Innenrahmen der Navigation, ihre Kontur,
Suchfeldgröße und Trennlinie, die Gewichtung der Überschriften, die Registerposition,
die Suchfeld-Ikone mit korrektem Texteingabe-Innenabstand, der runde jfWord-Schalter,
die 66-DIP-Formatvorlagengalerie und ihre drei Pfeil-/Menüfelder. Die Dateioberfläche
verwendet jetzt einen 26-DIP-Titel, 42-DIP-Zeilenabstände links, bündige 12,5-DIP-
Vorlagenbeschriftungen und Vorschaukarten mit eigenem Rahmen und Tastaturfokus.
Der Status hat eine obere Trennlinie und eine rahmenlose Zoomauswahl. Der Zoomregler
arbeitet logarithmisch, während API, Tastatur und Zahlenanzeige weiterhin Prozentwerte
verwenden. Ein 14-DIP-Scrollbarbereich wird vor der Seitenzentrierung abgezogen;
Papierformat, Seitenränder und Dokumentumbruch ändern sich dadurch nicht.

| Gemessener Bereich | Abweichende Pixel vorher | Abweichende Pixel jetzt |
| --- | ---: | ---: |
| Linker Navigationsrahmen (6.980 Pixel) | 6.980 | 0 |
| Rechter Navigationsrahmen (6.980 Pixel) | 6.980 | 0 |
| Kontur der Dokumentsuche (480 Pixel) | 251 | 0 |
| Navigationstrennlinie (237 Pixel) | 237 | 0 |
| Obere Statuslinie (1.424 Pixel) | 1.424 | 0 |
| Erster Vorlagenrahmen (656 Pixel, ohne gerundete Ecken) | 656 | 284 |
| Position der aktiven Dateizeile (380 Pixel) | 40 | 0 |

Damit sinkt die Abweichung **in diesen 17.137 ausgewählten Pixeln** von 16.568 auf 284.
Die verbleibenden 284 Pixel liegen an der oberen/unteren Vorlagenkante: Die Referenz
rasterisiert sie auf einer halben DIP-Position. Das ist ausdrücklich keine Messung
der gesamten Oberfläche. Abweichende Textformung, Kantenglättung, verfügbare Funktionen
und Dokumentfonts bleiben sichtbar und sind nicht durch diesen Teilvergleich abgenommen.
Die genauen Bildpfade und Ergebnisse stehen in `build/wordcraft-align/reference-comparison.json`.

Wiederverwendbare Ergänzungen liegen in der Library: `Ui::textbox_icon`,
`StyleEdit::textbox_font`, `Ui::label_button`, `Ui::preview_card`,
`Ui::ribbon_gallery_begin`, `ST_ROUND`, `RG_LOG` sowie `Range::position/value_at`.
`PageDraw::without_frame` vermeidet doppelte Konturen in Vorschaukarten. Comboboxen
reichen ihre Style-Flags jetzt auch an Messung und Painter weiter. Vorhandene normale
Textfelder und lineare Schieber behalten ihre Bedienung.

```powershell
python build_rs.py app jfWord --target all --out build/wordcraft-align -j 6
python apps/jfWord/tests/design_reference.py --build-dir build/wordcraft-align
python apps/jfWord/tests/check.py --target both --build-dir build/wordcraft-align
python apps/jfWord/tests/visual.py --out build/wordcraft-align
node tools/tests/web_apps.mjs build/wordcraft-align --engine=chromium,firefox --only=jfWord
```

Die neue `office_controls.uis` prüft Unicode-Eingabe und Zwischenablage bei gescrollter
Befehlssuche, Tastaturaktivierung der Vorschau und Textaktionen sowie logarithmisches
Ziehen, lineare Tastaturschritte und Zoomauswahl. Unabhängige Logarithmuswerte, Rundung,
monotone Abbildung und große/ungültige Grenzen werden zusätzlich im nativen Selbsttest
geprüft. Die beiden bestehenden Mausauswahltests wurden um den gemessenen 7-DIP-Versatz
der Seitenzentrierung korrigiert; ihre erwarteten UTF-8-Auswahlgrenzen bleiben unverändert.

### Abnahme dieser Nachkorrektur

- Windows, Linux und WebAssembly erfolgreich unter `build/wordcraft-align` gebaut.
- 1.023 native Prozessprüfungen bestanden, einschließlich 36 UI-Skripten je Plattform.
- Der abschließende Build besteht jeweils 9.356 Kernprüfungen auf Windows und Linux.
  Nach der letzten Rundungskorrektur des Zoomreglers wurden diese Selbsttests sowie
  `office_controls.uis` auf beiden Plattformen erneut ausgeführt.
- 66 Aufnahmen der Register-/Fenstergrößenmatrix ohne Dirty-Render-Abweichungen.
- 48 Dokumentbilder: 42 Vergleiche zwischen UI-Zuständen und drei Vergleiche zwischen
  Windows und Linux zeigen identische Dokumentpixel bei 100/150/200 Prozent.
- Sechs Chromium-/Firefox-Fälle und acht Speicherfehler-/Konkurrenzfälle bestanden.
- Vier Browser/native-DOCX-Übertragungen sind jeweils bytegleich (5.038 Bytes).
- Vier Gallery-Bildvergleiche unter Windows bleiben gegenüber dem eingefrorenen
  Standardstyle unverändert; Quellresolver-Tests, Repository-Lint und API-Generator
  sind erfolgreich.

Buildgrößen, SHA-256-Prüfsummen und die zugehörigen Ergebnisdateien sind in
`build/wordcraft-align/VALIDATION.json` zusammengefasst. Die weiter unten stehende
Abnahme unter `build/wordcraft-design` beschreibt die vorherige Designetappe.

## Umgesetzte Oberfläche

| Bereich | Umsetzung |
| --- | --- |
| Fensterkopf | 38 DIP; Schnellzugriff, mittiger Dokument-/Speicherstatus, Befehlssuche, jfWord-Kennung und native Fensteraktionen |
| Register | 30 DIP; Datei, Start, Einfügen, Zeichnen, Entwurf, Layout, Referenzen, Sendungen, Überprüfen, Ansicht, Hilfe; Unterstreichung des aktiven Registers |
| Menüband | 92 DIP; 66 DIP Inhalt, 16 DIP Gruppenbeschriftung, große 32-DIP-Symbole und kleine 17-DIP-Symbole; Trennlinien, Dialogstarter und geteilte Aktionen |
| Typografie | Eingebettetes Inter Regular/Medium/Semibold; UI 12,5 DIP, Aktionen 11,5 DIP, Gruppenbeschriftungen 11 DIP |
| Formatvorlagen | Dokumentprobe auf weißem Hintergrund, eigene Beschriftungszeile und themenabhängiger Auswahlrand; bestehende vier Absatzvorlagen bleiben funktional |
| Navigation | 260 DIP; Dokumentsuche, Überschriften/Seiten/Ergebnisse, Ebeneneinzug und vorhandene Such-/Sprungfunktionen |
| Dokumentfläche | Neutrale Arbeitsfläche, weiße Seiten, gemeinsame horizontale und vertikale Lineale; getrennte UI- und Dokumentfonts |
| Status | 26 DIP; Seite, Wortanzahl, Sprache, vorhandene Quellenanzeige sowie Fokus-, Ansichts- und Zoomsteuerung |
| Dateioberfläche | 200-DIP-Seitenleiste; fünf echte Vorlagenvorschauen von 150 × 194 DIP, zuletzt verwendete Dateien und vorhandene Dateiaktionen |

Die elf Register sind auch bei 600 DIP Breite erreichbar. Die gemeinsame Library
scrollt die Register mit Pfeilen und hält eine per Links/Rechts oder Pos1/Ende
gewählte Registerkarte sichtbar. Das Menüband scrollt vollständige Gruppen.
Übersetzte Aktionsspalten werden mit der tatsächlichen Schrift gemessen. Schmale
Fenster verwenden einen verkürzten Fensterkopf, ohne Dokumenttitel und Suche zu
überlagern. Zweizeilige große Aktionen bleiben über der Gruppenbeschriftung.

Es gibt 261 semantische Office-Symbolrollen und 105 Zuordnungen vorhandener
Library-Symbole. Die mehrfarbigen Vektoren werden mit eigenen jfguiLib-Pfaden
gezeichnet. Hell, Dunkel, deaktivierte Zustände und Akzentflächen werden getrennt
behandelt. Noch nicht implementierte Fachfunktionen bleiben sichtbar deaktiviert.

## Farben und nachgemessene Übereinstimmung

| Fläche | Hell | Dunkel |
| --- | --- | --- |
| Fensterkopf, Register, Status | `#F0F0F0` | `#1F1F1F` |
| Menüband | `#F9F9F9` | `#292929` |
| Arbeitsfläche | `#E6E6E6` | `#141414` |
| Navigation | `#FAFAFA` | `#262626` |
| Text | `#242424` | `#EEEEEE` |
| Akzent | `#3B5BDB` | `#7A93F0` |
| Auswahl | `#DDE3F8` | `#2E3A66` |

Neun direkt aus den Referenz- und jfWord-PNGs gelesene Flächenproben stimmen exakt
überein: fünf helle und vier dunkle. Die dunkle Referenz blendet die Navigation
aus; deren dunkler Wert ist deshalb aus dem Designtoken übernommen, nicht aus
diesem Screenshot gemessen. Koordinaten, Skalierung, Bildpfade und Farbwerte stehen
in `build/wordcraft-design/reference-colors.json`.

Weitere Farben und Metriken sind in der [Style-Dokumentation](../../../styles/jfoffice/README.md)
aufgeführt. Diese konkreten Übereinstimmungen sind kein Nachweis einer vollständigen
Pixelidentität der gesamten Anwendung. Deutsche Beschriftungen, jfWord-Kennung,
native Fensteraktionen, vorhandene Funktionen und Dokumentinhalte unterscheiden
sich von der Referenz. Antialiasing und Textformung haben einen eigenen Renderer.
Insbesondere werden bestehende Dokumentfarben und -schriften durch einen
Oberflächenwechsel nicht verändert.

## Gemeinsame Library-Bausteine

- `lib/ui/ribbon.rs`: Style-Metriken, vollständiger Gruppenüberlauf, große/kleine
  Aktionen, geteilte Schaltflächen, Dialogstarter, Breitenmessung und Vorlagenkacheln.
- `lib/ui/list.rs`: `Ui::tabs_scrolling` mit stabilen Tab-IDs, Tastatursteuerung und
  automatischer Sichtbarkeit der Auswahl; bestehendes `Ui::tabs` bleibt verfügbar.
- `lib/ui/office_gallery.rs`: wiederverwendbare Dokumentthemen-Galerie.
- `lib/ui/style.rs` und `cmd.rs`: optionale Style-Verträge für Vektorsymbole,
  Typografie und Ribbon-Vorschau. Andere Styles behalten ihre bisherigen Vorgaben.
- `lib/ui/theme.rs`: vollständige Invalidierung geöffneter und gespeicherter
  Fensterbilder bei einem Theme-/Akzentwechsel.
- `lib/doc_view/src/ruler.rs`: gemeinsame Darstellung der Seitenlineale.
- `styles/jfoffice`: Palette, Typografie, Symbole und eigene Widget-Painter in
  getrennten Modulen. `apps/jfWord/src/ui` ordnet die fachlichen Befehle an.

Die vorhandenen Text-, Absatz-, Quellen-, Zitat-, Noten-, Feld- und Dateifunktionen
bleiben über ihre bisherigen Dokumentoperationen angeschlossen. Der separate
Fehlerfalltest für atomisches Speichern verwendet jetzt ein blockiertes Ziel
statt eines fest angenommenen temporären Dateinamens. Die ursprünglichen
gespeicherten Bytes und das unveränderte Bibliotheksmodell werden weiterhin geprüft.

## Abnahme

| Prüfung | Ergebnis |
| --- | --- |
| Builds | Windows, Linux und WebAssembly erfolgreich unter `build/wordcraft-design` |
| Native Datei-/Codec-/UI-Prüfung | 1.021 erfolgreiche Prozessprüfungen mit unabhängigen ZIP/XML-, Ablehnungs-, Fuzz- und Roundtrip-Prüfungen |
| Native Selbsttests | Je 9.411 bestanden, 0 fehlgeschlagen auf Windows und Linux |
| UI-Skripte | 35 je Plattform, insgesamt 70/70; `--verify-dirty`, keine `UIWARN` |
| Register-/Fenstergrößenmatrix | 66 Aufnahmen: alle Register plus Dateioberfläche in sechs Hell-/Dunkel-/DPI-/Breitenkombinationen; 600–1.920 DIP, 100/150/200 Prozent |
| Dokumentpixel bei UI-Wechseln | 48 Bilder: zwei native Plattformen × drei DPI-Werte × acht Zustände; 42 Übergangsvergleiche und drei Plattformvergleiche pixelgleich |
| Direkter Theme-Kaltstart | Vier Starts, Windows/Linux jeweils hell/dunkel: identische Dokumentpixel, keine Renderwarnungen |
| Symbole nativ | Sechs vollständige Windows/Linux-Bildpaare identisch, hell/dunkel bei 100/150/200 Prozent |
| Symbole im Browser | Zwölf Chromium-/Firefox-Vergleiche identisch zum nativen Symbolbereich |
| Andere Styles | Acht Gallery-Vergleiche mit dem eingefrorenen WinUI-Referenzstand unverändert, einschließlich Widget-Dumps |
| jfWord im Browser | Sechs Chromium-/Firefox-Fälle plus acht Speicherfehler-/Konkurrenzfälle bestanden |
| Browser/native DOCX | Vier Kombinationen: Unicode-, Format- und Quellenprüfdokument jeweils bytegleich, 5.038 Bytes |
| Quellresolver und API | Vier Resolver-Tests, Repository-Lint und API-Neugenerierung erfolgreich |
| Lizenzbeigaben | Alle neun Kopien der drei Hinweise in Windows/Linux/Web bytegleich mit ihren Quellen |

Belege unter `build/wordcraft-design`: `final-check.txt`, `ui-results.json`,
`self-test-{win,linux}.txt`, `design-matrix/`, `jfword-visual-final/results.json`,
`initial-theme-probe/results.json`, `browser-tests/results.json`,
`browser-tests/native-transfer.json` und `license-results.json`.
Der Symbolnachweis liegt zusätzlich unter `build/wordcraft-icons/ICON_VALIDATION.md`.
Die Bildvergleiche prüfen unsere nativen und portablen Renderpfade sowie den Erhalt
der Dokumentdarstellung; sie vergleichen keine unterschiedlichen Dokumente miteinander.

### Reproduzieren

Ab `RustLib/`, mit den im Repository eingerichteten nativen und WASM-Werkzeugen:

```powershell
python build_rs.py app jfWord --target all --out build/wordcraft-design -j 6
python apps/jfWord/tests/check.py --target both --build-dir build/wordcraft-design
python apps/jfWord/tests/visual.py --out build/wordcraft-design --artifacts build/wordcraft-design/jfword-visual-final
node tools/tests/web_apps.mjs build/wordcraft-design --engine=chromium,firefox --only=jfWord
python tools/tests/web_transfer.py build/wordcraft-design
python tools/gendoc.py
python build_rs.py check
```

Die Tests erzeugen isolierte Fixtures unter `build/`; sie verwenden kein persönliches
Quellenprofil. Die Bildmatrix speichert ihre ausführbaren `.uis`-Skripte neben den
Screenshots. Der Browserzugang für Agenten bleibt bis zur expliziten Aktivierung aus.

## Herkunft und verbleibende Produktarbeit

Die Inter-Fonts werden mit der unveränderten SIL OFL ausgeliefert. Die Symbolmotive
und Farbpositionen sind vom MIT-lizenzierten WordCraft-Symbolwerk abgeleitet;
[LICENSE-icons.txt](../../../styles/jfoffice/LICENSE-icons.txt) enthält Herkunft
und Lizenz. Es werden keine WordCraft-, ArtCraft-, Discord- oder Microsoft-Logos
eingebunden. Build und Paketwerkzeug übernehmen die Style-Lizenzhinweise automatisch.

Diese Designetappe ergänzt keine bislang fehlenden Dokumentfunktionen allein durch
deren Sichtbarkeit im Menüband. Freie Schriftfamilien, allgemeine Formatvorlagen,
interaktive Einzüge/Tabstopps, Formatierungszeichen, mehrere Seiten nebeneinander
und die weitergehenden Office-Funktionen bleiben im [Umsetzungsstand](IMPLEMENTATION.md)
ausgewiesen. Vollständige Word-Dateikompatibilität benötigt weiterhin eigene
Implementierung und reale Word-Referenzprüfungen.
