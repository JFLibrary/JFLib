# jfOffice

jfOffice zeichnet die angeforderte WordCraft-Oberflächenreferenz mit eigenen jfguiLib-
Paintern nach. Maß-, Farb- und Typografiewerte stammen aus den lokal bereitgestellten
Referenzbildern und den zugehörigen Designtokens. Es wird kein fremder App-, egui- oder
Widgetcode eingebunden. Alle Style-Verträge bleiben lokal auf `Office`; andere Styles
werden nicht umgestellt. Dokumentfonts und Dokumentfarben bleiben unabhängig in `doc_view`.

## Visuelle Werte

Referenz: `context/wordcraft/docs/images/{hero,references,dark,layout,backstage}.png`,
2.880 × 1.800 Pixel beziehungsweise 1.440 × 900 DIP bei 200 %, einschließlich 8-DIP-Rahmen.
Die App verantwortet Außenrahmen, Arbeitsbereiche und Befehlsanordnung; der Style liefert
Typografie, Farben, Widgets, Chrome und die Geometrie des gemeinsamen Ribbon-Bausteins.

| Rolle | Hell | Dunkel |
| --- | --- | --- |
| Fensterkopf, Register, Status | `#F0F0F0` | `#1F1F1F` |
| Menüband | `#F9F9F9` | `#292929` |
| Dokumentarbeitsfläche | `#E6E6E6` | `#141414` |
| Navigations-/Seitenbereich | `#FAFAFA` | `#262626` |
| Text / Sekundärtext | `#242424` / `#616161` | `#EEEEEE` / `#C8C8C8` |
| Symbol / deaktivierter Text | `#424242` / `#A8A8A8` | `#DDDDDD` / `#6A6A6A` |
| Akzent / Akzenttext | `#3B5BDB` / `#2B47B5` | `#7A93F0` / `#9CB0F5` |
| Auswahlfläche | `#DDE3F8` | `#2E3A66` |
| Trennlinie / Eingaberand | `#E1E1E1` / `#C7C7C7` | `#3A3A3A` / `#585858` |
| Gruppenbeschriftung | `#707070` | `#A0A0A0` |

- Fensterkopf 38 DIP, Register 30 DIP, Statusleiste 26 DIP.
- Menüband 92 DIP, Inhalt 66 DIP, Gruppenbeschriftung 16 DIP, Innenrand 8/4 DIP.
  Die gemeinsame Ribbon-Engine scrollt vollständige Gruppen horizontal.
  `Ui::ribbon_button_width` misst vollständige Aktionsbeschriftungen inklusive Symbol und
  Dropdown, sodass übersetzte Spalten die benötigte Breite erhalten.
- Kleine Aktionen 22 DIP hoch, Symbol 17 DIP; große Aktionen zeigen ein 32-DIP-Symbol
  bei y=4 und Beschriftungen in 13-DIP-Zeilen: einzeilig ab y=43, zweizeilig ab y=40.
  Formatvorlagen zeigen ihre Dokumentprobe auf Weiß; Auswahl und Beschriftung verwenden
  die jeweilige helle oder dunkle Oberfläche.
- Inter Regular 12,5 DIP für UI-Text, 11,5 DIP für Ribbonaktionen, 11 DIP für Beschriftungen.
  Register verwenden Medium, aktive Register und hervorgehobene UI-Texte Semibold.
- `Ui::tabs_scrolling` hält sämtliche Register auch in schmalen Fenstern erreichbar.
  Pfeile und horizontales Scrollen behalten die Auswahl bei; Links/Rechts und Pos1/Ende
  wählen ein Register und scrollen es in den sichtbaren Bereich.
- `Style::ts_size_fx` erhält halbe DIP-Größen. Bestehende Styles behalten durch den
  Standardwert ihr bisheriges Raster; `StyleList::tab_font` hält Messung und Malen konsistent.
- `ST_MENU` ergänzt einen Dropdownindikator; `ST_ON_ACCENT` zeichnet normale semantische
  Schaltflächen auf farbigen Navigationsflächen. `ST_COMPACT` zeichnet Register als 22-DIP-
  Chips und Schalter als 30 × 16 DIP. Befehle, Fokus und Popups bleiben in `ui`.
- `ST_ROUND` zeichnet runde Identitätsaktionen. Symbol-Textfelder verwenden Inter 11,5 DIP,
  6-DIP-Ecken und ein reserviertes Symbolfeld; Text, Auswahl und IME teilen dieselben Metriken.
  `Ui::label_button` erlaubt bündige Vorlagenbeschriftungen und gewichtete Gliederungseinträge.
- Vorschaukarten und Formatvorlagengalerien verwenden eigene gemeinsame UI-Bausteine.
  Vorlagenrahmen sind 1 DIP stark und verwenden den kräftigen Randtoken; Fokus und Hover
  bleiben normale Widgetzustände. Galeriefläche 66 DIP, Innenrand 3 DIP, Kacheln 74 × 60 DIP.
  Vorschaukarten liegen vertikal auf einer halben DIP-Position. `Shapes::panel_fx` kombiniert
  Füll- und Randabdeckung in einem Schritt; der Inhalt wird innerhalb des Rahmens beschnitten,
  auch beim kräftigeren Hoverrand. Die Dokumentprobe behält dabei ihre Skalierung.
- Dateititel 26/36 DIP und Navigationszeilen 38 DIP mit 4 DIP Zwischenraum. Die rahmenlose
  Zoomauswahl respektiert `ST_SUBTLE`; der gemeinsame positive logarithmische Sliderpfad
  wird mit `RG_LOG` aktiviert. Lineare Schieber anderer Apps benötigen keine Anpassung.

Der aktuelle direkte Screenshotvergleich steht unter
`apps/jfWord/docs/VISUAL_REFERENCE.md`; Bilder und Messwerte unter `build/wordcraft-type`.

Ein identischer Palettewert ist nachmessbar. Eine vollständige Pixelidentität mit einem
fremden Renderer ist damit nicht zugesichert: Rasterisierung, Textformung, die jeweils
verfügbaren App-Funktionen und die Dokumentdarstellung werden gesondert geprüft.

## Schriften und Lizenz

Der Style bettet die offen lizenzierten Inter-Schriften unverändert als TrueType ein:

- `assets/Inter-Regular.ttf`
- `assets/Inter-Medium.ttf`
- `assets/Inter-SemiBold.ttf`

Dadurch bleiben die GPOS-Paarabstände für die vorhandene TrueType-Implementierung der
Library erhalten. Diese Inter-Dateien enthalten keine ältere `kern`-Tabelle; der bisherige
SGF-Konverter übertrug die GPOS-Abstände nicht. Die SGF-Versionen bleiben als reproduzierbarer
Vorher-Zustand für `officeicontest` erhalten. Das Symbolfont bleibt SGF.

`Style::font_raster` aktiviert `FontRaster::Natural` nur für die drei UI-Schriften:
unverzerrte Outline-Skalierung und Rundung auf die nächste Viertelpixelposition.
Die bisherige x-Höhen-Anpassung und abrundende Positionierung bleiben der Standard für
andere Styles und nachgeladene Dokumentfonts. Ein Profilwechsel leert den Glyphcache;
er verändert weder Textbreiten noch Auswahl-/Caret-Koordinaten. TrueType-Hinting und eine
vollständige komplexe Textformung sind damit noch nicht implementiert.

Copyright © 2016 The Inter Project Authors. Die vollständige SIL Open Font License 1.1
liegt in [assets/OFL-Inter.txt](assets/OFL-Inter.txt) und ist mit den Fonts auszuliefern.
Keine Dateien aus `docs/brand`, Microsoft-Office-/Segoe-Fonts oder Herstellerlogos werden
für diesen Style verwendet. Die SGF-Vergleichsfonts enthalten je 2.852 Unicode-Zuordnungen und
2.877 Glyphen einschließlich Komponenten. Der Konverter prüft die SGF1-Outlines gegen den
TrueType-Eingang. CJK-Fallback und komplexe Schriftformung sind gesonderte Library-Aufgaben.

Reproduktion der SGF-Vergleichsfonts und Symbole ab `RustLib/`:

```powershell
python tools/ttf2bin.py --font styles/jfoffice/assets/Inter-Regular.ttf --charset all --out styles/jfoffice/assets/font_reg.bin
python tools/ttf2bin.py --font styles/jfoffice/assets/Inter-Medium.ttf --charset all --out styles/jfoffice/assets/font_medium.bin
python tools/ttf2bin.py --font styles/jfoffice/assets/Inter-SemiBold.ttf --charset all --out styles/jfoffice/assets/font_bold.bin
python styles/jfoffice/mkicons.py
```

Schriftvergleich mit zehn Originalausschnitten sowie Prüfung von Cache, Koordinaten,
Transparenz, Teilrepaint und 100/150/200-%-Ausgabe:

```powershell
python build_rs.py app officeicontest --target all --out build/wordcraft-type
python apps/jfWord/tests/typography_reference.py --build-dir build/wordcraft-type --target both
node apps/officeicontest/tests/browser.mjs build/wordcraft-type
```

## Symbole

`mkicons.py`, `icons.rs` und die erzeugten Symbolassets bilden semantische Office-Rollen auf
Code-native Vektorgeometrie ab. Der Style verwendet den allgemeinen `Style::paint_icon`-
Hook; andere Styles und App-eigene Icon-Hooks behalten ihren bisherigen Pfad. Die GUI-
Semantik verwendet weiterhin stabile `IC_*`-Rollen statt App-koordinatengebundener Grafiken.

Die Motive und Farbpositionen orientieren sich am MIT-lizenzierten Symbolwerk von
WordCraft. Die vollständige Herkunft steht in [LICENSE-icons.txt](LICENSE-icons.txt).
Der Build legt alle Style-Lizenzhinweise bei nativen und Web-Ausgaben ab; das gemeinsame
Paketwerkzeug übernimmt sie auch in Windows- und Debian-Pakete.
