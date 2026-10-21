# Fuß- und Endnoten in Tabellenzellen

Stand: 13. Oktober 2026. Ausbau von **REF-001 bis REF-004** und des Tabellenkerns
in JFLib (`GitHub/JFLib`). Die vorhandene Notenbearbeitung gilt nun auch für
normale, verbundene, verschachtelte und schwebende Tabellenzellen.

## Bedienung

1. Eine Textstelle in einer Tabellenzelle auswählen.
2. Unter **Referenzen** auf **Fußnote einfügen** oder **Endnote einfügen** klicken.
3. Den Notentext direkt auf der Papierseite schreiben und formatieren. Quellenverweise
   können wie im Haupttext eingefügt werden.
4. **Zur Textstelle** führt zur ursprünglichen Zelle zurück. Die Navigation zeigt
   beide Notenarten; Umwandlung, Entfernen und Undo verwenden die bestehenden Abläufe.

Das Kopieren einer ganzen Tabelle nimmt ihre Noten, deren Absätze und darin zitierte
Quellen mit. Beim Einfügen werden neue IDs vergeben, identische Quellen wiederverwendet
und vorhandene Noten im Zieldokument erhalten. Dasselbe gilt für eine Rich-Text-Auswahl
mit vollständigem Notenanker innerhalb einer Zelle. Entfernen einer Zeile, Spalte oder
Tabelle entfernt nur die dabei weggefallenen Noten. Undo stellt den gesamten Zusammenhang her.

## Gemeinsame Bibliotheken

- `doc_core` verwaltet Anker, Notenkörper und Quellen unabhängig von Oberfläche und
  Betriebssystem. Die Nummerierung folgt der Lesereihenfolge einschließlich innerer
  Tabellen; Speicherreihenfolge und IDs beeinflussen sie nicht.
- `doc_layout` führt die Ankerpositionen durch Zellmessung, vertikale Ausrichtung,
  Zeilenteilung und schwebende Seitenfragmente. Fußnoten reservieren Platz unterhalb
  der Tabelle. Bereits zugeordnete Anker dürfen bei späterer Platzreservierung nicht
  von ihrem Notentext getrennt werden.
- Wiederholte Kopfzeilen zeigen den vorhandenen Verweis erneut, erzeugen jedoch weder
  eine neue Nummer noch einen zweiten Notenkörper.
- Bei **genauer Zeilenhöhe** kann der Marker unsichtbar werden. Seine Anmerkung bleibt
  erhalten und gehört zum letzten sichtbaren Fragment der Zelle. Das ist eine bewusst
  festgelegte JFLib-Regel, kein Nachweis einer identischen Behandlung in jeder Word-Version.
- Lange Noten setzen sich über Seiten fort. In Dokumenten mit Zellfußnoten werden
  schwebende Tabellen während der Notenfortsetzung zurückgestellt. Endnoten folgen
  dem Dokument einschließlich aller noch ausstehenden schwebenden Tabellenfragmente.
- `doc_io` verwendet die vorhandenen DOCX-Notenparts und den versionierten nativen
  Zwischenablagecodec. Es gibt keine getrennte Implementierung für WASM.

Noten innerhalb von Noten, Tabellen in Notenkörpern, abschnittsweise Neustarts,
benutzerdefinierte Nummernzeichen und beliebige Word-Layouts bleiben außerhalb dieses
Profils. Eine Ankerzeile mit ihrem erforderlichen Notenanfang muss auf eine Seite passen;
unmögliche Gruppen melden einen Layoutfehler. RTF und HTML behalten ihr bisheriges
begrenztes Austauschprofil: Notenmarker erscheinen als Text, lebende Notenmetadaten
werden damit nicht übertragen. **DOCX und die native Rich-Text-Zwischenablage** erhalten
die vollständigen Zellnoten einschließlich ihrer Quellenfelder.

## Agentenschnittstelle

`notes.insert`, `notes.kind` und `notes.remove` gelten auch für Zellabsätze. Sie verwenden
dieselben Editortransaktionen und Revisionsprüfungen wie die Oberfläche. Beispiel:

```json
{
  "action": "notes.insert",
  "expectedRevision": 12,
  "input": {
    "paragraph": 7,
    "inline": 0,
    "offset": 4,
    "kind": "footnote",
    "text": "Erklärung zum Messwert."
  }
}
```

IDs werden aus dem geöffneten Dokument abgefragt; `offset` zählt UTF-8-Bytes im
angegebenen Textlauf. `document.inspect` ergänzt pro Note `number` und `anchor` mit
`paragraph`, `inline` und `cell` (bei Haupttextankern `null`). Nach strukturellen Änderungen
bleibt keine Auswahl in einem inzwischen entfernten Zell- oder Notenkörper aktiv.

## Prüfung und Builds

```powershell
python -X utf8 -B build_rs.py app jfWord --target all --out build/wordcraft-table-notes -j 4
python -X utf8 -B apps/jfWord/tests/table_notes.py --target both --out build/wordcraft-table-notes
node apps/jfWord/tests/table_notes_browser.mjs build/wordcraft-table-notes
python -X utf8 -B apps/jfWord/tests/table_notes.py --target both --out build/wordcraft-table-notes --web-transfer
python -X utf8 -B apps/jfWord/tests/isolated_check.py --target both --build-dir build/wordcraft-table-notes
```

`table_notes_checks.rs` prüft Modell, Quellen, Kopiergraph, Undo, DOCX und die
Seitengeometrie mit festen Schriftmaßen. `table_notes.py` verwendet echte JSON-/MCP-
Prozesse und prüft die DOCX-Verweise unabhängig über ZIP/XML. `table_notes.uis` bedient
Zelle, Notentext, Rücksprung, Umwandlung und Entfernen über die native Oberfläche.
Der Browsertest tippt in den Notenkörper, prüft verschachtelte und schwebende Tabellen
sowie bestätigtes dauerhaftes Speichern mit Neuladen in Chromium und Firefox.

Builds und Prüfergebnisse liegen unter `build/wordcraft-table-notes`. Die nächstfolgenden
Tabellenarbeiten sind Tabellenformeln und weitere Import-/Layoutprofile; vollständige
Word-Kompatibilität bleibt gesondert nachzuweisen.

## Abnahme dieses Stands

- Windows und Linux: jeweils **10.572 Kernprüfungen**, davon 128 neue Assertions dieser
  Etappe; die vollständige native Suite besteht **1.055 Prozessprüfungen** einschließlich
  der vorhandenen ZIP-/XML-, Negativkorpus-, Fuzz-, UI- und Theme-Prüfungen.
- **28 Agentenszenarien** über JSON und MCP auf beiden Plattformen: Zellnoten sowie die
  bisherigen Zellrahmen-, Zwischenablage-, Verschachtelungs-, Positions- und Zeilenprofile.
- **30 Browserfälle** in Chromium und Firefox bei 100, 150 und 200 Prozent. Je sechs
  Fälle betreffen Zellnoten, genaue Zeilenhöhen, Zeilenumbruchregeln, mehrseitige schwebende
  Tabellen und verschachtelte Tabellen. Die Varianten bei 100 Prozent verwenden den
  skalaren WASM-Pfad; die übrigen Varianten prüfen tatsächlich gewähltes WASM-SIMD.
- **60 Übertragungen Browser → Windows/Linux**: Nach bestätigtem Speichern und Neuladen
  lassen sich die erzeugten DOCX-Dateien nativ öffnen und bytegenau erneut speichern.
- Die neuen nativen Notenansichten sind zwischen Windows und Linux bei 100 Prozent hell
  und 150 Prozent dunkel bytegleich. Der UI-Ablauf prüft zusätzlich Größenänderung und
  200 Prozent ohne UI-Warnungen. Screenshots wurden visuell kontrolliert.
- Lint und generierte API-Dokumentation sind aktuell. Windows, Linux und alle drei
  WASM-Varianten sind gebaut; die Threads-Variante ist in dieser Etappe **nicht** im
  Browser ausgeführt worden.

Ein erster Firefox-Regressionslauf konnte WASM beim Start nicht initialisieren; zusätzlich
trat ein Fehler beim Schließen des Browserkontexts auf. Ein vollständiger neuer Lauf der
vier betroffenen Regressionssuiten bestand unverändert. Das erste Fehlerprotokoll bleibt
neben den erfolgreichen Ergebnissen erhalten; es wurde kein Test übersprungen.

Maschinenlesbare Nachweise: `verification.json`, `manifest.json` und
`feature-sources.json` im Build-Ordner. `changes.patch` dokumentiert die Änderungen
gegenüber den vor dieser Etappe gesicherten Dateien.
