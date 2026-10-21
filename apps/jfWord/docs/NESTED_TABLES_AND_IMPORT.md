# Verschachtelte Tabellen und RTF-/HTML-Import

Dieser Ausbau liegt ausschließlich in JFLib. jfWord verwendet denselben Dokumentkern,
dieselben Codecs, dieselbe Seitendarstellung und dieselben Transaktionen auf Windows,
Linux und `wasm32-unknown-unknown`. Es gibt keine zweite HTML-Dokumentoberfläche.

## Bedienung

- In einer Tabellenzelle **Einfügen → Tabelle** wählen. Die neue Tabelle steht am
  Textcursor; Text vor und nach ihr bleibt in der umgebenden Zelle.
- Innere Zellen werden direkt auf der Seite bearbeitet. Tab/Shift+Tab wechseln die
  Zelle innerhalb der aktiven Tabelle, Enter erzeugt einen Absatz, Strg+Tab einen Tabulator.
- Die Tabellenwerkzeuge zeigen bei Untertabellen die Ebene an. **Zur äußeren Zelle**
  fokussiert den Absatz nach der inneren Tabelle.
- **Tabelle kopieren/ausschneiden** nimmt den gesamten Teilbaum mit. Beim Einfügen
  werden Identitäten neu vergeben und Quellen/Formatvorlagen kollisionsfrei übernommen.
  Zu breite Tabellen werden in die Zielzelle eingepasst. Reicht deren Breite nicht
  für die Mindestmaße, bleibt das Dokument unverändert.
- **Öffnen** akzeptiert zusätzlich `.rtf`, `.html` und `.htm`. Der Import erzeugt ein
  ungespeichertes Dokument; die Eingabedatei wird nicht überschrieben. Speichern erfolgt
  als `.docx`. Dasselbe gilt für Dateiauswahl und Drag-and-drop im Browser.
- Normales Einfügen erkennt natives jfWord-DOCX, RTF und HTML. Im Browser reicht der
  gemeinsame Host die MIME-Daten des Paste-Ereignisses an den Worker weiter. Externe
  Rich-Text-Daten ersetzen dabei alle älteren lokalen Zwischenablageformate.

## Gemeinsame Bausteine

| Bibliothek | Verantwortung |
|---|---|
| `doc_core/table_tree.rs` | Verankerung in Haupttext/Zellen, Eltern und Ebenen, Lesereihenfolge, Breiten und Begrenzungen |
| `doc_core/table_fragment.rs` | Gemischte Absätze/Tabellen einfügen, Unterbäume kopieren, Identitäten und Stilgraphen übertragen |
| `doc_layout/notes.rs` | Rekursive Zellinhalte messen, äußere Zeilen vergrößern, Seitenfortsetzungen und innere Kopfzeilen |
| `doc_layout/table_fit.rs` | Verfügbare Breite der umgebenden Zelle und Mindestbreite enthaltener Tabellen |
| `doc_view/src/table.rs` | Tiefste Zelle treffen, innere Tabellen direkt vergrößern/verkleinern |
| `doc_io/tables.rs` | Standard-OOXML: echte `w:tbl` innerhalb von `w:tc`, stabile interne Identitäten |
| `doc_io/import_blocks.rs` | Gemeinsamer begrenzter Block-/Rasteraufbau für beide Importformate |
| `doc_io/rtf_tables.rs` | RTF-Zeilen, Zellgrenzen, Tabellenebenen und Verbünde |
| `doc_io/html_import.rs`, `html_css.rs` | Statischer HTML-Tokenizer, Tabellenmodell und unterstützte CSS-Werte |
| `plat/web.rs`, `web/loader.js`, `web/worker.js` | Atomare Übergabe externer Zwischenablageformate für alle Apps |

`Table.before` darf jetzt auf einen Absatz einer Tabellenzelle zeigen. Die Tabellen bleiben
in einem gemeinsamen Register und werden in Blockreihenfolge serialisiert. Die Validierung
verwirft Zyklen, verwaiste Verankerungen und mehr als acht Ebenen. Löschen einer Tabelle,
Zeile oder Spalte entfernt auch die Untertabellen in tatsächlich entfernten Zellen; Undo
stellt den ganzen Teilbaum wieder her. Verbinden erhält Absatz- und Tabellenanker.

Auf Folgeseiten können Kopfzeilen innerer Tabellen innerhalb einer fortgesetzten äußeren
Zelle erscheinen. Zellwände werden fortgesetzt, ohne zusätzliche horizontale Rahmen an
Seitenumbrüchen zu erfinden. Kopfzeilen und Text bleiben über ihre ursprünglichen
Dokumentidentitäten bearbeitbar.

## Importprofil und Grenzen

Unterstützt sind verschachtelte Tabellen, Zeilen-/Spaltenverbünde, führende Kopfzeilen,
Spaltenbreiten, Mindestzeilenhöhen, einheitliche Zellabstände, vertikale Ausrichtung,
Füllfarben und vier individuelle Zellrahmen. Text bleibt in bearbeitbaren Absätzen mit
direkter Schriftfamilie, Größe, Farbe, Fettdruck, Kursivschrift und Unterstreichung.
Die interne Zwischenablage erhält zusätzlich die semantischen Quellen-, Feld- und Stilbezüge;
RTF/HTML übertragen die sichtbaren Feldtexte und direkte Formatierung.

RTF liest `trowd`, `cellx`, `cell/row`, `intbl/itap`, `nestcell/nestrow`, die
`nesttableprops`-Destination sowie horizontale und vertikale Merge-Controls. Ignorierbare
Metadaten bleiben ignorierbar; unbekannte sichtbare Inhalte werden abgelehnt. Das Profil
bildet seit der [folgenden Etappe](FLOATING_TABLES_AND_RTF_GRIDS.md) unterschiedliche
Zeilenraster und Zeileneinzüge geometrisch ab. Absolute Zeilenhöhen, nicht unterstützte Zeichensätze, Bilder, Objekte und Listen werden
nicht als angeblich vollständig bearbeitbarer Inhalt importiert.

HTML unterstützt `table`, `thead/tbody/tfoot`, `tr`, `td/th`, `colgroup/col`, `rowspan`
und `colspan`, übliche ausgelassene Absatz-/Zellen-/Zeilen-Endtags, UTF-8 und die häufigen
HTML-Zeichenreferenzen. Breiten verwenden Pixel, Punkte, Zoll, Zentimeter oder Millimeter;
Tabellenbreiten dürfen prozentual sein. `rowspan=0` reicht bis zum Ende der Zeilengruppe.
CSS wird statisch aus Inline-Deklarationen gelesen. Externe Stylesheets, beliebige CSS-
Layoutregeln und eine vollständige Browser-HTML-Fehlerkorrektur gehören nicht zu diesem
Importprofil. Uneinheitliche Zellabstände, Tabellenbeschriftungen und eingebettete Bilder/
Objekte werden derzeit abgelehnt. Skripte und Metadaten werden nicht ausgeführt; es gibt
keine Netzwerkzugriffe aus dem Importer.

Grenzen: 16 MiB Eingabe, 2 MiB Text, 64 HTML-/RTF-Strukturebenen, acht Tabellenebenen,
128 Tabellen, 256 Zeilen und 32 Spalten je Tabelle. Das ist ein ausdrücklich begrenztes
Importprofil und kein Nachweis vollständiger Word-, RTF- oder HTML-Kompatibilität.

Der Browser hält beim eigenen Kopieren strukturierte Daten lokal vor. Falls der Browser
beim späteren Paste nur denselben Klartext liefert, wird diese lokale Rich-Text-Kopie
weiterverwendet. Ein extern kopierter identischer Klartext ist in diesem Fall technisch
nicht vom eigenen Klartext zu unterscheiden. HTML/RTF-Paste-Daten ersetzen den Cache
immer; geänderter Klartext verwirft ebenfalls die älteren Rich-Text-Daten.
LF/CRLF-Unterschiede durch die Windows-Zwischenablage bleiben dabei ohne Einfluss auf
die Wiedererkennung. `Strg+Umschalt+V` verwendet auch im Browser ausdrücklich Klartext.

## Agenten und Prüfung

Die Aktion `document.import` nimmt `paragraph`, `start`, `end`, `format` (`html`/`rtf`)
und `text` entgegen. Sie verwendet die erwartete Dokumentrevision und dieselbe atomare
`ReplaceStoryFragment`-Transaktion wie Einfügen auf der Seite. `document.table.inspect`
und die Dokumentübersicht liefern zusätzlich `parent` und `depth`.

Quellen für die Syntax: [Microsoft RTF-Spezifikation](https://learn.microsoft.com/en-us/previous-versions/office/developer/office2000/aa140277(v=office.10))
und [WHATWG-Tabellenmodell](https://html.spec.whatwg.org/multipage/tables.html).

Reproduzierbare Prüfungen nach dem Build:

```powershell
python -X utf8 -B build_rs.py app jfWord --target all --out build/wordcraft-nested-tables -j 6
python -X utf8 -B apps/jfWord/tests/check.py --target both --build-dir build/wordcraft-nested-tables
python -X utf8 -B apps/jfWord/tests/nested_tables.py --target both --out build/wordcraft-nested-tables
node apps/jfWord/tests/nested_tables_browser.mjs build/wordcraft-nested-tables
python -X utf8 -B apps/jfWord/tests/nested_tables.py --target both --out build/wordcraft-nested-tables --web-transfer
```

Die Browserprüfung verwendet Chromium und Firefox bei 100/150/200 %, hellem/dunklem
Style, skalarem und automatisch gewähltem WASM-Backend. Sie prüft Canvas-Klicks in
innere Zellen, Tastatureingabe, den Tabellen-Dialog, externe HTML-/RTF-Paste-Ereignisse,
Cache-Ersetzung, Dateiauswahl, Undo und bestätigtes dauerhaftes Speichern/Neuladen.
Die anschließend erzeugten DOCX-Dateien werden bytegenau nativ wieder geöffnet/exportiert.
Resultate, Screenshots und Build-Manifest liegen unter `build/wordcraft-nested-tables/`.

Abnahme dieses Stands:

| Prüfung | Ergebnis |
|---|---|
| Native Gesamtregression | 1.045 Prozessprüfungen unter Windows und Linux bestanden |
| Tabellenkern und Codecs | Je Plattform 379 Tabellen- und 82 Zwischenablageprüfungen bestanden |
| JSON/MCP | Acht Szenarien für Import, Zellrahmen, Teilbäume, Revisionen, Undo und Dateirundlauf bestanden |
| Chromium/Firefox | Sechs neue Import-/Verschachtelungsfälle plus sechs bestehende Tabellen-Zwischenablagefälle bestanden |
| Native/Web-Übertragung | Zwölf Browser-Dokumente auf beiden nativen Zielen bytegenau geöffnet/exportiert: 24 Vergleiche |
| Build/Struktur | Windows, Linux, skalares/SIMD-/Threads-WASM gebaut; Build-Lint und API-Prüfung erfolgreich |

Die Browserläufe verwenden kein SharedArrayBuffer; das Threads-WASM wurde gebaut,
aber in dieser Matrix nicht zur Laufzeit geprüft. Die Paste-Ereignisse mit fremden
HTML-/RTF-MIME-Daten werden im Test erzeugt; das eigene Kopieren/Einfügen läuft zusätzlich
über die Browser-Systemzwischenablage. Ein Rundlauf mit einer echten Microsoft-Word-
Installation wurde nicht durchgeführt.

Maschinenlesbarer Nachweis: `nested-tables-results.json`; Build-Prüfsummen:
`nested-tables-build-manifest.json`, jeweils im genannten Ausgabeverzeichnis.
