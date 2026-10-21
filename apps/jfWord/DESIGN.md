# jfWord – Zielbild und Stand der Designüberarbeitung

Stand: 11. Oktober 2026. Grundlage sind die drei vom Nutzer bereitgestellten
Bildreferenzen: Word-Startansicht, Word-Dokumentfenster und der aktuelle jfWord-Stand.
Diese Anforderungen konkretisieren den [Featurekatalog](FEATURES.md#6-funktionskatalog).
Der [README](README.md) beschreibt weiterhin den tatsächlich implementierten Stand.

Die erste strukturelle Überarbeitung ist umgesetzt, das **Enddesign bleibt offen**.
Start-/Dateiansicht mit eigenen Vorlagen, letzten/angehefteten Dateien und Suche, kompakter
Fensterkopf, gruppiertes Menüband mit Überlauf und Formatgalerie, ausblendbare Seitenbereiche
sowie weiße Papierseiten ersetzen den ursprünglichen Aufbau. Gemischte Textsegmente,
ausgerichtete und mehrseitige Absätze sowie Notentext sind direkt auf der Seite bearbeitbar.
Eine Auswahl reicht über Formate, Absätze und Seiten des Haupttexts beziehungsweise einer Note. Felder bleiben
geschützt; eine Kontextleiste öffnet Zitate und Anmerkungen direkt von der Seite aus.
Absatzübergreifendes Ersetzen, Formatieren und strukturiertes Einfügen ist atomar.
Die Rich-Text-Zwischenablage erhält innerhalb jfWord Formatierung und Quellen-/Notenbezüge;
für andere Programme stehen RTF, HTML und Klartext bereit. HTML-Import bleibt offen.

Gemeinsame Bausteine liegen in `lib/ui/ribbon.rs` und `lib/ui/text_surface.rs`.
Dokumentprojektion und atomare Bereichsänderungen liegen in `lib/doc_core/text_range.rs` und `story_range.rs`;
Dokumentfragmente und deren Austauschformate liegen in `doc_core` und `doc_io`.
die Cursorpositionen folgen den UTF-8-Quellbereichen von `doc_layout`.
Die App verwendet weiterhin ausschließlich jfguiLib. Eigene geometrische Icons ersetzen
unspezifische Platzhalter in den vorhandenen Menübandaktionen.

## Leitbild und Grenzen

- Vertraute Anordnung und Arbeitsabläufe einer umfangreichen Textverarbeitung:
  Dateiablage, Register, Befehlsgruppen, Papierfläche, kontextbezogene Bereiche, Status.
- Eigenständiger `jfoffice`-Style auf der nativen jfguiLib; keine zusätzliche Weboberfläche.
- Eigene Markenführung, Icons, Illustrationen und Vorlagen; Assets mit dokumentierter
  Herkunft und passenden Lizenzen. Die Referenzbilder werden nicht als Produktassets übernommen.
- Ruhige Flächen, klare Typografie und eigener Akzent; Bedeutung darf nicht nur an Farbe hängen.
- Die Bildreferenzen dienen zum Vergleich von Aufbau und Bedienbarkeit. Sie sind keine
  Freigabe zur Übernahme geschützter Gestaltungselemente. Offene Veröffentlichungsprüfungen
  aus [FEATURES.md](FEATURES.md) bleiben bestehen; dieses Zielbild ist keine Rechtsfreigabe.
- Quellenverwaltung und wissenschaftliche Arbeitsabläufe bleiben vollwertige Kernfunktionen.
  Ihre Erreichbarkeit darf durch die aufgeräumte Standardansicht nicht schlechter werden.

## Zielaufbau

| Bereich | Gewünschter Aufbau | Überprüfbare Abnahme |
|---|---|---|
| Startansicht | Schmale linke Navigation; neues leeres Dokument und eigene Vorlagen oben; darunter zuletzt verwendete und angeheftete Dateien mit Suche | Neues Dokument ohne Konto; Dateiname, Speicherort und Änderungsdatum gut unterscheidbar; lange Namen bleiben zugänglich (`UI-001`) |
| Datei-/Backstage-Ansicht | Klar abgegrenzter Bereich für Neu, Öffnen, Speichern, Exportieren, Drucken und Einstellungen; Rückkehr zum Dokument | Verlassen erhält Auswahl und Scrollposition; ungespeicherte Änderungen bleiben erkennbar (`UI-001`, `UI-012`) |
| Fensterkopf | Kompakter Dateiname mit Speicherstatus, Schnellzugriff und Befehlssuche | Keine doppelte dominante Produkt-/Dokumentüberschrift; gleichnamige Dateien unterscheidbar; Quelle über Suche erreichbar (`UI-002`, `UI-005`, `UI-007`) |
| Registerleiste | Datei, Start und weitere funktionsbezogene Register; Auswahlkontext ergänzt passende Register | Registerwechsel verändert keine Dokumentauswahl; noch nicht implementierte Funktionen erscheinen nicht als scheinbar verfügbare Befehle (`UI-003`, `UI-004`) |
| Menüband | Dichte, beschriftete Gruppen mit großen Hauptaktionen, kompakten Nebenaktionen und Dialogstartern | Start gruppiert Zwischenablage, Schrift, Absatz, Formatvorlagen und Bearbeiten; alle Befehle bei geringer Breite über Überlauf erreichbar (`UI-003`) |
| Formatvorlagen | Visuelle Galerie mit erkennbarer Typografie und eindeutigem angewendetem Stil | Vorschau, Auswahl und direkte Formatabweichungen unterscheidbar; Vorschauabbruch verändert keine Historie (`UI-011`, `FMT-010`) |
| Dokumentfläche | Zentrierte weiße Papierseiten auf ruhigem Hintergrund, sichtbare Seitenabstände und durchgehendes Scrollen | Schreiben, Cursor und Auswahl direkt auf den gesetzten Seiten; keine separate Formularansicht als regulärer Schreibmodus (`EDIT-003`, `EDIT-004`, `VIEW-001`) |
| Seitenbereiche | Navigation, Quellen, Formatvorlagen und Kommentare bei Bedarf öffnen; Breite anpassbar | Neues Dokument startet ohne belegte Seitenbereiche; Referenzen öffnet Quellen direkt; Schließen verändert keinen Inhalt (`UI-009`) |
| Statusleiste | Eine schmale Zeile mit Seite, Wörtern, Sprache, Änderungs-/Prüfstatus sowie Ansichts- und Zoomsteuerung | Werte folgen Dokument und Auswahl; Zoom und Seitenbreite ohne Registerwechsel erreichbar (`UI-010`, `VIEW-003`) |

## Direktes Schreiben auf der Seite

Das vollständige Ziel der Papieransicht verbindet Satz und Bearbeitung. Ein Wechsel zwischen
Textformular und Vorschau darf zum normalen Schreiben nicht mehr erforderlich sein.

- Cursor per Klick präzise positionieren; Auswahl über Format-, Absatz- und Seitengrenzen.
- Eingabe, Löschen, Zwischenablage und Undo/Redo erhalten Formatierung und Feldbezüge.
- Scrollen folgt dem Cursor und einer gezogenen Auswahl, ohne unerwartet zu springen.
- Seitenbreite, ganze Seite und individuelle Zoomwerte erhalten die aktuelle Textstelle.
- Lineale, Absatzzeichen und Begrenzungen lassen sich einschalten, ohne den Ausdruck zu ändern.
- Quellen- und Zitatfelder bleiben strukturierte Objekte; ihr Aussehen passt zum Dokumenttext.
- Editor, Druckvorschau und spätere Ausgabe nutzen denselben Layoutstand (`OUT-001`).

## Wissenschaftliche Arbeit im aufgeräumten Fenster

Das Register Referenzen enthält erkennbare Gruppen für Zitate, Quellen und Verzeichnisse.
Die Quellenverwaltung öffnet als kontextbezogener Arbeitsbereich; bei Bedarf erhält sie
einen größeren Dialog für Importprüfung, Metadaten und Vergleiche. Bibliografische Details
müssen nicht permanent die Schreibfläche verkleinern, bleiben aber direkt erreichbar.

- Quelle suchen, bearbeiten und zitieren ohne Verlust der Einfügestelle im Dokument.
- Aus einem Zitat zur Quelle und zurück zur konkreten Verwendungsstelle navigieren (`CITE-019`).
- Lange Titel, Autorenlisten und Importfehler erhalten Platz, ohne andere Eingaben zu verdecken.
- Auswahl-, Filter- und Importzustände sind eindeutig; aktive Quellen nicht allein farbig markieren.
- Literaturverzeichnis und Zitatstil werden im Dokument geprüft; Hilfsbereiche ersetzen diese Ansicht nicht.

## Darstellung, Größen und Tastatur

- Helles und dunkles Anwendungsthema; Papier bleibt standardmäßig weiß und der Dokumentinhalt
  farblich unverändert. Ein späterer Lesemodus muss klar von gespeicherten Seitenfarben getrennt sein.
- Einheitliche eigene Icons, Strichstärken, Abstände und Zustände für Hover, Fokus, Druck,
  Auswahl, deaktivierte Befehle und Fehler in allen Widgetfamilien (`UI-016`).
- Menübandgruppen verkleinern sich stufenweise und wechseln dann in beschrifteten Überlauf.
  Die gesamte Befehlszeile darf weder abgeschnitten werden noch seitlich aus dem Fenster laufen.
- Prüfraster: Fenstergrößen 1000×700, 1240×800 und 1920×1080 DIP, jeweils bei 100 und
  150 Prozent Skalierung, hellem und dunklem Thema, auf Windows und Linux.
- Zusätzlich 200 Prozent für den vollständigen Style gemäß `UI-016`; keine Überlagerung,
  abgeschnittenen Pflichtfelder oder unerreichbaren Dialogaktionen.
- Vollständige Tastaturwege: Dokument → Menüband → Quellenbereich → Dialog → Dokument;
  sichtbarer Fokus, sinnvolle Tabreihenfolge und konsistente Rückkehr mit Escape (`UI-008`).
- Zugängliche Rollen, Namen, Zustände und Auswahl bereitstellen; Symbole besitzen verständliche
  Beschriftungen oder Hilfetexte. Kontrast und Fokus auch ohne Akzentfarbe prüfen (`VIEW-008`, `VIEW-010`).

## Reihenfolge und Abnahme

1. **Entwurf:** Startansicht und Dokumentfenster als eigene Gestaltung ausarbeiten; die drei
   Fensterbreiten mit geöffnetem/geschlossenem Quellenbereich darstellen und gegen dieses Zielbild prüfen.
2. **Gemeinsame GUI-Grundlagen:** Wiederverwendbares gruppiertes Menüband, Überlauf,
   Formatgalerien, andockbare Bereiche und kompakte Status-/Zoomsteuerung in der GUI-Lib ergänzen.
3. **Fensterstruktur:** Eigenen Style vervollständigen; Kopf, Register, Menüband, Papierfläche
   und Statusleiste zusammenführen; Start-/Dateiansicht und kontextbezogene Bereiche anschließen.
4. **Seiteneditor:** Direkte Eingabe, Auswahl und Navigation auf mehrseitigem Layout umsetzen;
   den bisherigen Formulareditor erst nach funktionaler Abnahme als Standardablauf ablösen.
5. **Gesamtabnahme:** Wissenschaftliche Aufgaben, Dateiarbeit und Tastaturabläufe durchspielen;
   Screenshots des Größenrasters prüfen und automatische UI-/Dirty-Rendering-Tests ergänzen.

- [x] Neues Dokument startet als leere bearbeitbare Papierseite; Formatwechsel, Ausrichtung, mehrseitige Absätze und Notentext sind direkt bearbeitbar.
- [x] Auswahlformatierung, direkte Farben, Durchstreichen und Einfügeformat funktionieren über Textsegmente und Absätze desselben Haupttext-/Notenbereichs hinweg.
- [x] Eigene Zwischenablagegruppe im Menüband; formatierte Auswahl und verknüpfte Felder über Dokumentgrenzen übertragen, ausdrücklich als Klartext einfügen.
- [x] Kontextbezogene Zitat-/Noten-/Verzeichnisaktionen direkt über der Papieransicht, Feldnavigation per Menüband und F11; Fokus kehrt nach Dialogen zur Auswahl zurück.
- [x] Enter/Zeilenwechsel und Verbinden an Absatzgrenzen sind direkt per Tastatur bedienbar; Cursor und Auswahl werden bei strukturellem Undo wiederhergestellt.
- [x] Erhaltene DOCX-Dateien haben einen Filter für Haupttext, Kopf-/Fußzeilen und Noten; der Textdialog benennt seinen Bereich und passt ins Fenster.
- [x] Erste strukturelle Hierarchie mit kompaktem Kopf, gruppiertem Menüband, Papier und einer Statuszeile umgesetzt.
- [ ] Start-/Dateiansicht ist vollständig nutzbar; keine übernommenen fremden Vorlagen oder Bildassets.
- [ ] Quellenverwaltung bleibt mit wenigen klaren Aktionen erreichbar und erhält die Dokumentauswahl.
- [ ] Alle Größen-/Themenfälle sind ohne abgeschnittene Bedienwege nutzbar.
- [ ] Typografie, Fokus, Tastatur und zugängliche Namen sind geprüft; Kernabläufe bestehen auf beiden Plattformen.
- [x] README und Screenshots trennen implementierten Stand und noch offene Zielanforderungen eindeutig.
