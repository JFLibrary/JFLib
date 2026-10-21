# Interaktive Lineale und individuelle Tabstopps

Stand: 12. Oktober 2026. Fortsetzung der [Zeichenformatetappe](CHARACTER_FORMATTING.md).
Build: `RustLib/build/wordcraft-tabs/{win,linux,web}`.

## Bedienung

- Im horizontalen Lineal verschiebt das obere Dreieck die erste Zeile. Das untere
  linke Dreieck verschiebt die Folgezeilen bei feststehender erster Zeile; das Rechteck
  darunter verschiebt beide gemeinsam. Das rechte Dreieck setzt den rechten Einzug.
- Die Übergänge zwischen grauem Rand und weißem Textbereich verändern die Seitenränder.
  Horizontal sind sie bei überlagernden Einzugsmarken am oberen Linealrand erreichbar.
  Das vertikale Lineal verändert oberen und unteren Rand.
- Der Knopf am Linealursprung wechselt zwischen linksbündigen, zentrierten,
  rechtsbündigen, dezimalbündigen Tabstopps und senkrechten Linien. Ein Klick auf
  eine freie Linealposition setzt den gewählten Typ.
- Vorhandene Tabstopps lassen sich verschieben oder zum Löschen aus dem Lineal ziehen.
  Ein ausgewählter Stopp kann auch mit Entf gelöscht werden. Ein Zug wird erst beim
  Loslassen als ein Undo-Schritt übernommen. Escape bricht ab; Alt hebt das Raster auf.
  Das Raster beträgt 1/16 Zoll (90 Twips). Die Vorschau im Lineal verändert noch kein Dokument.
- **Start → Absatz-Dialogstarter → Tabstopps …** öffnet die genaue Bearbeitung.
  Positionen werden in Punkt eingegeben; 0,05 Punkt entspricht einem Twip.
  **Setzen** übernimmt eine Position in den Entwurf, **Übernehmen** den gesamten Entwurf
  in die ausgewählten Absätze. **Alle löschen** unterdrückt auch geerbte Stopps.
  Der Tabstopps-Dialog wird mit dem zuletzt gespeicherten Absatzstand geöffnet; noch
  nicht übernommene Änderungen im vorherigen Absatzdialog werden nicht gespeichert.
- Tab oder Strg+Tab fügt im Schreibbereich ein echtes Tabulatorzeichen ein. In Dialogen
  bleibt Tab die Fokusnavigation. Formate, Quellenfelder und Noten bleiben strukturiert.
- Der Formatvorlagendialog besitzt ein eigenes Register **Tabstopps**. Dort ergänzen
  Stopps die Basisvorlage; **Geerbten Stopp löschen** unterdrückt eine einzelne Position.

## Gemeinsame Bibliotheken

`doc_core` definiert `TabStop`, `TabStops`, `TabAlignment` und `TabLeader`. Positionen
beziehen sich auf den linken Textseitenrand, unabhängig vom Absatzeinzug. Vererbung
führt Positionen zusammen; `Clear` entfernt eine geerbte Position. Absatzänderungen,
Bereichsauswahl, Validierung und Undo/Redo bleiben Operationen des vorhandenen Editors.
`ParagraphChange::Tabs` ersetzt die effektiven Stopps jedes ausgewählten Absatzes und
berechnet dessen eigene notwendigen Löschdeklarationen.

Es sind höchstens 64 Deklarationen pro Liste und 64 aufgelöste Stopps möglich. Eine
Überfüllung durch Vererbung wird als Fehler zurückgegeben; die Transaktion bleibt
unverändert. Die feste Speicherung erhält den bisherigen `Copy`-Vertrag des Absatzformats.

`doc_layout` misst den Text nach einem Tab über Formatwechsel hinweg. Rechts- und
Zentrierung verwenden die gesamte Breite dieses Felds, Dezimaltabs den Text vor dem
ersten Punkt oder Komma. Senkrechte Linien sind eigenständige Zeilengeometrie.
Füllzeichen bleiben rein visuell: Auswahl, Zwischenablage und Caret sehen weiterhin
genau ein Tabzeichen. Haupttext, Fußnoten und Endnoten verwenden denselben Algorithmus.

`doc_view` zeichnet Stopps, Füllzeichen, Linien und skalierte Linealgriffe.
`RulerController` liefert eine abgeschlossene `RulerEdit` an die Hostanwendung.
Der gemeinsame `TabEditor` und der ohne Schriftzeichen gezeichnete Typwähler funktionieren
auf allen Plattformen. jfWord bindet diese Bausteine mit kleinen Adaptern an seine Auswahl an.
`ui::canvas` liefert nun auch nach einem außerhalb liegenden Loslassen das Release-Ereignis
an den zuvor aktiven Canvas; das gilt für alle Anwendungen.

## Austausch und Agenten

DOCX liest und schreibt `w:tabs/w:tab` in Absätzen, Vorlagen und Noten. Die additive
Vererbung und die Bedeutung von `clear` folgen der
[WordprocessingML-Tabulatorbeschreibung](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.tabs)
und der [Definition einzelner Tabstopps](https://learn.microsoft.com/en-us/dotnet/api/documentformat.openxml.wordprocessing.tabstop).
Die private Zwischenablage verwendet weiterhin denselben DOCX-Codec.

RTF führt Position, Ausrichtung und Füllzeichen über `tx`, `tb`, `tqc`, `tqr`, `tqdec`
und die `tl…`-Steuerwörter. Gruppenzustände, der nächste Stopp und `pard` werden getrennt
zurückgesetzt. HTML enthält entsprechende Word-`tab-stops`-Deklarationen. Gewöhnliche
HTML-Renderer unterstützen diese Erweiterung nicht zuverlässig; eine identische Darstellung
individueller Stopps in fremden HTML-Programmen wird damit nicht zugesichert.

Neue Fachaktionen, nativ über CLI/JSON/MCP und über den aktivierten Browserdispatcher:

| Aktion | Verhalten |
| --- | --- |
| `document.paragraph.format` | Patch auf die Absätze einer UTF-8-Story-Auswahl; `tabs` führt Positionen zusammen |
| `document.paragraph.tabs` | Effektive Tabstopps der ausgewählten Absätze ersetzen; leere Liste löscht alle |
| `document.page.set` | Seitengröße und Ränder validieren und als einen Undo-Schritt setzen |
| `document.inspect` | Enthält nun direkte und effektive Absatzformate sowie Seitenmaße |
| `document.styles.put` | `paragraph.tabs` ergänzt die bestehenden Vorlageneigenschaften |

Beispiel für `document.paragraph.tabs`, zusätzlich zur erwarteten Dokumentrevision:

```json
{
  "paragraph": 1, "start": 0, "end": 0,
  "tabs": [
    {"position": 2880, "alignment": "decimal", "leader": "dot"},
    {"position": 4320, "alignment": "right", "leader": "none"}
  ]
}
```

Werte sind Twips. Ausrichtungen: `left`, `center`, `right`, `decimal`, `bar`; in
Vererbungspatches zusätzlich `clear`. Füllzeichen: `none`, `dot`, `hyphen`, `underscore`,
`heavy`, `middleDot`. `bar` und `clear` verlangen `none`. Doppelte Positionen und
unzulässige Kombinationen werden abgewiesen. Änderungen verlangen weiterhin eine
aktuelle Revision und umgehen keine offenen Dialoge.

## Prüfbarkeit und verbleibende Grenzen

Die Tests liegen in `src/checks/tab_checks.rs`, `tests/tabs.py`, `tests/tabs.uis`,
`tests/rulers.uis` und `tests/tabs_browser.mjs`. Sie prüfen Ausrichtung und Füllzeichen, Vorlagen und Löschungen,
übergreifende Auswahlen, Noten, Codec-Roundtrips, Eingabe, atomare Fehler, Undo/Redo,
echte Zeigergesten und den präzisen Dialog. Der Browserlauf verwendet Chromium und
Firefox, 100/150/200 % Geräteskalierung, hell/dunkel, zusätzlich Dokumentzoom sowie
native DOCX-Dateien und bestätigte dauerhafte Speicherung mit erneutem Laden.

```powershell
# Im Verzeichnis RustLib
python -X utf8 build_rs.py app jfWord --target all --out build/wordcraft-tabs -j 6
python -X utf8 apps/jfWord/tests/tabs.py --target both --out build/wordcraft-tabs
node apps/jfWord/tests/tabs_browser.mjs build/wordcraft-tabs
python -X utf8 apps/jfWord/tests/tabs.py --target both --out build/wordcraft-tabs --web-transfer
python -X utf8 apps/jfWord/tests/check.py --target both --build-dir build/wordcraft-tabs
```

Aufnahmen, Browserdateien und Ergebnislisten liegen in `build/wordcraft-tabs/tabs-browser`.
`tabs-build-manifest.json` enthält Größe und SHA-256 der drei ausgelieferten Programme.
Die vorhandenen Referenzbuilds werden nicht ersetzt. Ohne individuelle Stopps bleibt
das bisherige absatzbezogene Raster von 36 Punkt erhalten; nach einer expliziten Liste
setzt es sich am linken Textseitenrand fort. Hängende Einzüge haben einen impliziten
Stopp am regulären Absatzeinzug. Tabhaltige Zeilen werden an ihren Tabpositionen
ausgerichtet und nicht zusätzlich im Blocksatz gedehnt. Übergroße Tabfelder umbrechen
normal, ohne vorangehenden Text zu überdecken.

Dokumentspezifische Standardtabweiten, sprachabhängige Dezimal-/Tausendertrennung,
bidirektionale Tabulatoren, Abschnitte und pixelidentischer Microsoft-Word-Umbruch bleiben
separate Kompatibilitätsarbeiten. Diese Etappe bedeutet keine vollständige Word-Kompatibilität.

## Abnahme dieser Etappe

Die Builds unter `build/wordcraft-tabs` sind für Windows, Linux und WASM erfolgreich
erstellt; die Quellprüfung meldet `lint ok`, die API-Dokumentation wurde aktualisiert.

| Prüfung | Ergebnis |
| --- | --- |
| Native Gesamtprüfung | 1.032 Prozessprüfungen für Windows/Linux bestanden; ZIP/XML, Ablehnungskorpus, Fuzz, bestehende UI-/Theme-Abläufe und neue Dialog-/Linealfälle. Der während des Laufs ergänzte Linealmaustest wurde für Windows separat erfolgreich ausgeführt. |
| Kernprüfungen | Windows und Linux jeweils 9.707 Assertions bestanden, 0 Fehler |
| Tabstopps über JSON/MCP | 4/4 Abläufe unter Windows/Linux bestanden |
| Bestehende Agentenabläufe | 4/4 Abläufe unter Windows/Linux bestanden, einschließlich Quellen, Noten, Revisionen und Undo |
| Chromium/Firefox | 6/6 Fälle mit echten Zeigergesten, genauer Eingabe, Undo/Abbruch, Zoom, 100/150/200 % und dauerhaftem Speichern/Neuladen bestanden |
| Browser → native DOCX | 12/12 Übertragungen; die sechs Browserdateien lassen sich unter Windows und Linux bytegleich importieren und wieder ausgeben |

Die lokale Vorschau unter `http://127.0.0.1:8765/jfWord/` liefert diesen Build.
Ein bereits geöffneter Tab bleibt bis zum Neuladen auf seiner geladenen Version;
ungespeicherte Änderungen vorher sichern.
