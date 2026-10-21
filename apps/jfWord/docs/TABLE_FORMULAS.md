# Tabellenformeln und Zahlenformate

Stand: 13. Oktober 2026. Implementierter Teil von **TAB-014 / FLD-008** in JFLib
(`GitHub/JFLib`), gemeinsam für Windows, Linux und WASM.

## Bedienung

Eine Textstelle in der Ergebniszelle auswählen und in den Tabellenwerkzeugen auf
**Formel …** klicken. Der gemeinsame Dialog zeigt während der Eingabe das aktuelle
Ergebnis. **Übernehmen** fügt ein geschütztes Formelfeld am Textcursor ein; vorhandener
Text bleibt erhalten. Ein ausgewähltes Formelfeld wird damit bearbeitet. Über die
Feldleiste sind **Formel bearbeiten** und **Feld entfernen** erreichbar.

Beispiele:

| Formel | Verwendung |
| --- | --- |
| `=SUM(ABOVE)` | Summe der numerischen Zellen über der Ergebniszelle |
| `=SUM(LEFT)` | Summe der numerischen Zellen links davon |
| `=A2*B2` | Menge mal Einzelwert |
| `=SUM(A2:B3)` | Rechteckiger Bereich derselben Tabelle |
| `=SUM(R2C1:R3C1)` | Bereich mit expliziten Zeilen-/Spaltenbezügen |
| `=ROUND(SUM(LEFT)/2,1)` | Berechnung mit gerundetem Ergebnis |
| `=IF(A2>0,B2/A2,0)` | Bedingung; nur der gewählte Zweig wird ausgewertet |

Die Ergebnisse werden aus dem aktuellen Dokument berechnet. Änderungen an Zellwerten,
Formeln und Tabellenstruktur sowie Undo/Redo aktualisieren abhängige Ergebnisse ohne
zusätzlichen Undo-Schritt. **F9** zeichnet die aktuellen Ergebnisse erneut. Der Befehl
startet keinen separaten Berechnungsdienst.

## Rechen- und Bezugsprofil

Die Syntax orientiert sich an den dokumentierten Word-Feldformeln
([Microsoft Support](https://support.microsoft.com/en-us/word/use-a-formula-in-a-word-table)).
Dies ist ein ausdrücklich begrenztes, prüfbares Profil:

- Operatoren `+`, `-`, `*`, `/`, `%`, `^`, Klammern und Vergleiche `=`, `<>`, `<`, `>`,
  `<=`, `>=`. Potenzen haben höhere Priorität als das unäre Minus; sie sind rechtsassoziativ.
- Funktionen: `ABS`, `AND`, `AVERAGE`, `COUNT`, `DEFINED`, `FALSE`, `IF`, `INT`, `MAX`,
  `MIN`, `MOD`, `NOT`, `OR`, `PRODUCT`, `ROUND`, `SIGN`, `SUM`, `TRUE`. `DEFINED`
  bedeutet in diesem Profil „Argument numerisch auswertbar“, ohne Textmarkenauflösung.
- Funktionsnamen und Bezüge sind unabhängig von der UI-Sprache und nicht
  groß-/kleinschreibungssensitiv. Argumenttrennung mit Komma oder Semikolon; Dezimalpunkt
  in Formelausdrücken. Numerische Zelltexte dürfen Dezimalpunkt oder Dezimalkomma verwenden.
- `A1`, `R1C1`, rechteckige Bereiche und `ABOVE`, `BELOW`, `LEFT`, `RIGHT`; `R` und `C`
  bezeichnen die aktuelle gesamte Zeile beziehungsweise Spalte. Ein Bezeichner wie `C1`
  wird als A1-Zellbezug interpretiert, nicht als ganze Spalte.
- Richtungsangaben lassen Kopfzeilen aus. Bereiche lassen die eigene Ergebniszelle aus;
  ein direkter oder indirekter zyklischer Einzelbezug zeigt `#CYCLE!`.
- Eine verbundene Zelle wird in Bereichen einmal gezählt. Lücken eines unregelmäßigen
  Rasters werden bei Richtungsangaben übersprungen; ein expliziter Bezug auf eine Lücke
  meldet `#REF!`. Verschachtelte Tabellen besitzen einen eigenen Bezugskontext. Ihre
  Inhalte gehen nicht zusätzlich in den Zahlenwert der äußeren Zelle ein.
- Zellwerte dürfen aus mehreren formatierten Textläufen bestehen. Notenanker verändern
  den Zahlenwert nicht. Leere beziehungsweise nicht numerische Zelltexte werden von
  Bereichsaggregaten ausgelassen; als einzelner arithmetischer Bezug liefern sie `#VALUE!`.
  Felder anderer Art und gemischte Beschriftung/Zahlen sind keine numerischen Werte.
- Bezüge verwenden die **aktuellen logischen Koordinaten**. Einfügen/Löschen von Zeilen
  und Spalten verändert das Raster; explizite Bezugstexte werden nicht wie Excel-Formeln
  automatisch umgeschrieben. Richtungsformeln werden am aktuellen Feldort neu ausgewertet.

Der Kern verwendet geprüfte Ganzzahlarithmetik mit **neun Nachkommastellen**. Division
und Multiplikation begrenzen Zwischenwerte auf diese Genauigkeit; `ROUND` rundet bei
halben Werten von null weg. Keine binären Gleitkomma- oder plattformabhängigen
Transzendentenfunktionen. Numerische Überläufe, Division durch null und Budgetüberschreitung
werden sichtbar, statt zu NaN, Endlosschleifen oder ungültigen Dokumenten zu führen.

Zahlenformate: automatisch, `0`, `0.00` und allgemein bis zu neun feste Nachkommastellen,
`#,##0`, `#,##0.00` sowie Prozentvarianten. Die Ausgabe verwendet Punkt für Dezimal- und
Komma für Gruppentrennung. Wissenschaftliche Exponenten, Währungs-/Locale-Pictures,
Textmarken und tabellenübergreifende Bezüge sind weitere Ausbauprofile.

Fehlercodes sind stabil: `#SYNTAX!`, `#UNSUPPORTED!`, `#REF!`, `#VALUE!`, `#DIV/0!`,
`#CYCLE!`, `#NUM!`, `#LIMIT!`. Syntaktisch nicht unterstützte Eingaben werden vor dem
Commit abgelehnt. Berechnungsfehler gültiger Formeln bleiben als bearbeitbares Feld
sichtbar. Grenzen: 2.048 ASCII-Zeichen, 512 Parserknoten, 32 Parser-/Abhängigkeitsebenen,
64 Auswertungsebenen, 16.384 Schritte je Ergebnis und 128 Formelfelder pro Dokument.

## Gemeinsame Implementierung und Austausch

- `lib/doc_core/formula/{mod,parser,number,eval}.rs`: eigener Feldtyp, Dezimalrechnung,
  Bezüge, Abhängigkeiten, Fehler, atomare Einfüge-/Änderungsoperationen.
- `lib/doc_view/src/formula_editor.rs`: gemeinsame Dialogfelder und Vorschau.
- `doc_layout` und Textprojektion: das generierte Resultat wird dargestellt und als
  unteilbares Feld geschützt. Freies Tippen überschreibt seine Ziffern nicht.
- `doc_io/formulas.rs`: standardmäßige DOCX-`=`-Feldanweisung mit `\#`-Zahlenformat und
  sichtbarem Resultat. Absatzlokale komplexe Word-Felder werden ebenfalls eingelesen.
  Der bekannte `\* MERGEFORMAT`-Schalter wird bei unverändertem Paket erhalten; direkte
  Feldresultatformatierung bleibt im bisherigen Erhaltungsprofil.
- Ganze Tabellen und native Rich-Text-Auswahlen behalten die Formelstruktur. Beim Einfügen
  werden IDs wie bisher neu vergeben und Zellbezüge in der Zieltabelle ausgewertet.
- RTF/HTML exportieren **aktuelle Ergebniswerte als Text**; lebende Formeln werden mit
  diesen begrenzten Austauschprofilen nicht übertragen.

Nicht unterstützte Funktionen, Zahlen-Pictures und gesperrte DOCX-Felder öffnen sich
über den bestehenden Paketerhalt. Unveränderte fremde Pakete bleiben bytegenau erhalten,
einschließlich ihrer gespeicherten Feldresultate. Die Anzeige verwendet bei unterstützten
Feldern den aktuellen Rechenstand; bei Modelländerung schreibt der Codec neue Resultate.
Das ist kein Nachweis vollständiger Word-Feld- oder Layoutkompatibilität.

## Agentensteuerung

`formulas.list` liefert paginierte Felder mit Paragraph, Inline-Index, Tabelle, Ausdruck,
Zahlenformat, Ergebnistext und getrenntem Fehlercode. `formulas.evaluate` prüft einen
Entwurf ohne Änderung. `formulas.insert` und `formulas.update` verlangen die aktuelle
Dokumentrevision und verwenden dieselben Editor-/Layouttransaktionen wie die Oberfläche.
Vorhandene Modalitäts-, Schema- und Ressourcenregeln bleiben wirksam.

```json
{
  "action": "formulas.insert",
  "expectedRevision": 12,
  "input": {
    "paragraph": 7,
    "inline": 0,
    "offset": 0,
    "expression": "=SUM(ABOVE)",
    "numberFormat": "0.00"
  }
}
```

`paragraph` und `inline` werden aus der Dokumentinspektion übernommen; `offset` zählt
UTF-8-Bytes im bearbeitbaren Textlauf. Ein Aktualisieren verwendet dieselbe Zieladresse
ohne `offset`. Die allgemeine Registrierung enthält außerdem den Dialogbefehl `130`
und den Aktualisierungsbefehl `131`.

## Prüfung

```powershell
python -X utf8 -B build_rs.py app jfWord --target all --out build/wordcraft-table-formulas -j 4
python -X utf8 -B apps/jfWord/tests/formulas.py --target both --out build/wordcraft-table-formulas
node apps/jfWord/tests/formulas_browser.mjs build/wordcraft-table-formulas
python -X utf8 -B apps/jfWord/tests/formulas.py --target both --out build/wordcraft-table-formulas --web-transfer
python -X utf8 -B apps/jfWord/tests/isolated_check.py --target both --build-dir build/wordcraft-table-formulas
```

`formula_checks.rs` verwendet unabhängige erwartete Ergebnisse für Dezimalrechnung,
Operatorpriorität, Bedingungen, Fehler, Parsergrenzen, Abhängigkeiten, Header,
verbundene/verschachtelte Zellen, Zwischenablage, DOCX und Undo. `formulas.py` prüft echte
JSON-/MCP-Prozesse, komplexe Feldimporte und Paketerhalt zusätzlich über unabhängiges
ZIP/XML. `formulas.uis` bedient Dialog, Syntaxfehler, Berechnungsfehler und F9. Der
Browsertest tippt in Dialog und Papierseite und prüft schwebende Fortsetzungen,
Tabellenkopien sowie bestätigtes dauerhaftes Speichern und Neuladen.

## Abnahme vom 14. Oktober 2026

Der abschließend neu gebaute Stand mit dem öffentlichen Namen `jfgui` liegt unter
`build/wordcraft-table-formulas`. Die [Umbenennungsprüfung](../../../docs/JFGUI_RENAME.md)
und der Formelausbau verwenden dieselben auslieferbaren Binaries und Browserbundles.

- Windows und Linux: jeweils 10.652 Kernprüfungen, keine Fehler; der vollständige
  jfWord-Satz besteht mit 1.057 nativen Prozessprüfungen und 53 UI-Skripten je Plattform.
- 32 JSON-/MCP-Szenarien für Formeln und die vorhandenen Tabellen-/Notenfunktionen;
  zusätzlich je vier reale CLI- und Grafik-Agentenszenarien auf beiden Plattformen.
- 18 Featurefälle in Chromium/Firefox bei 100/150/200 Prozent, darunter Eingabe im
  Formeldialog, Live-Abhängigkeiten, verschachtelte Tabellen, Zwischenablage,
  schwebende Fortsetzungen und dauerhafte Speicherung mit erneutem Laden.
- 36 bytegleiche DOCX-Übertragungen aus dem Browser in Windows und Linux, einschließlich
  Formeln, Zellnoten und verschachtelten Tabellen.
- Die Formeldialoge sind für Windows/Linux bei hell/100 Prozent und dunkel/150 Prozent
  bytegleiche PNGs. 200 Prozent und kleinere Fenster werden im UI-Verhalten geprüft.
- Gemeinsamer Projekt-Gesamtlauf: 363/363 Skripte; Build-Manager/Quellresolver einschließlich
  WSL-Abbruch: 17/17 Tests. Alle registrierten Windows-/Linux-/Web-Builds bestehen.
  Lint und die API-Dokumentation mit 5.099 Einträgen sind aktuell.

Die Messwerte, Logpfade, SHA-256-Manifest und Bilder stehen in `verification.json`,
`rename-verification.json`, `manifest.json` und `native-formula-images.json` im Build-Ordner.
Die Feature-Browserfälle führen skalare und SIMD-Binaries aus. Der gemeinsame
Shared-Memory-Workerbetrieb wird zusätzlich mit der portablen Rechenprobe und PaintCraft
in beiden Browsern ausgeführt; er ersetzt keine gesonderte Formelfallmatrix für Threads.
