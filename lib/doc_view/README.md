# doc_view

Gemeinsame Darstellung für seitenbasierte Dokumente. Die Bibliothek verbindet `doc_core`
und `doc_layout` mit dem bestehenden Text- und Software-Renderer. Seitendarstellung,
Vorschaubilder und Koordinatenumrechnung haben damit dieselbe Implementierung in allen
Apps und auf allen Plattformen. `ui::Draw` verbindet diese Darstellung mit einem Canvas.

`Fonts` registriert einmal pro `text::Text` die bisherigen DejaVu-Sans-Standardformen.
DejaVu Serif und DejaVu Sans Mono sind ebenfalls eingebettet und werden bei Bedarf geladen:
normal, fett sowie synthetisch kursiv und fett-kursiv. Native Apps verwenden zusätzlich die
vorhandene `text::fontdb::FontDb` für installierte, unterstützte Schriftfamilien. Der Browser
benötigt für die drei eingebetteten Familien keine lokale Schriftfreigabe.

`prepare` löst Dokumentvorlagen und direkte Zeichenformate vor der Messung auf;
`ensure` lädt eine einzelne Familie für Galerie-/Dialogproben. `families` liefert die
verfügbaren Namen, `substituted` kennzeichnet Ersatzschriften. Unbekannte Familien behalten
ihren Namen im Modell. Der Cache hält höchstens 64 zusätzliche Familien und bleibt bei
Dokumentwechseln erhalten. `Fonts::clone` teilt Cache und Handles desselben Text-Engines.
Freies Entladen dieser Fonts oder Übertragen auf einen anderen Text-Engine ist nicht erlaubt.

`paginate` misst in Twips und stellt den zuvor ausgewählten GUI-Font anschließend wieder
her. `PageDraw` verwendet dieselben Font-IDs und Dokumentfarben. Anwendungsfarben, Theme
und Zoom ändern keine Formatierung im Dokument. Die bisherigen Seiten- und Textpixel
bleiben für den Standardfont unverändert.

Skripte verwenden `ResolvedFormat::display_size_half_points()` und `baseline_shift()`.
`doc_layout` hält die verschobenen Glyphen innerhalb der Zeilenextents; `PageDraw` zeichnet
ihre exakten Hervorhebungsfarben zuerst und danach Text und Dekorationen. Caret-Messung,
Miniaturen und Browser verwenden dieselbe Geometrie. `ribbon_sample` liefert diese
Eigenschaften für die gemeinsame Vorlagengalerie, mit begrenzter Vorschaugröße.

`PageTransform` rechnet für Canvas, Zeigerpositionen und Vorschaubilder mit einer gemeinsamen
Skalierung. Ungültige Größen werden abgelehnt; extreme Koordinaten sättigen am i32-Bereich.
`zoom_width` unterstützt 10–500 Prozent. `PageDraw::thumbnail` liefert eine eigenständige
Pixelressource und begrenzt die Ausgabe auf 2048 × 4096 Pixel. Eine fehlende Seite oder
ungültige Größe führt zu `PageError`.

`RulerDraw::version()` beschreibt den sichtbaren Linealinhalt einschließlich Einzügen,
Seitengeometrie, Schrift und Palette. Der Wert gehört als Version an `Ui::cmd_custom`:
Ein Wechsel des ausgewählten Absatzes kann die Markierungen ohne Dokumentänderung versetzen.

Vollständiger Schrift-Fallback anhand einzelner Zeichen und komplexes Shaping sind weitere
Ausbauschritte. Ersatzschriften können andere Metriken besitzen; Umbruchparität setzt
dieselben verfügbaren Familien voraus.

`assets/font_reg.bin` und `assets/font_bold.bin` sind SGF1-Konvertierungen von DejaVu Sans
2.37 (`DejaVuSans.ttf`, `DejaVuSans-Bold.ttf`) mit allen Zeichen der Schrift. Die TTFs liegen
nicht im Repository; `tools/ttf2bin.py` erzeugt aus ihnen dieselben Bytes:

```text
python tools/ttf2bin.py --font DejaVuSans.ttf --out lib/doc_view/assets/font_reg.bin --charset all
python tools/ttf2bin.py --font DejaVuSans-Bold.ttf --out lib/doc_view/assets/font_bold.bin --charset all
```

Die zusätzlichen Serif-/Mono-TTFs sind unverändertes DejaVu 2.37, mit zugehöriger
[Lizenz](assets/DejaVu-LICENSE.txt).
Die unabhängige Ablage vermeidet eine Abhängigkeit von `styles/jfoffice`.
