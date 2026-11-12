# VoiceOver-Plan: der Screenreader auf Mac und iPhone

Stand 2026-11-12, auf Basis von `main` ce54de4c. Umgesetzt ist VO0 (die Brücke für macOS mit Rollen, Werten,
Aktionen und dem Test im Prozess; die Hand-Probe des Nutzers steht aus). Die vier Entscheidungen unten hat der Nutzer
am 2026-11-12 alle wie empfohlen getroffen.

## Ziel

Wer mit VoiceOver arbeitet, kann JFLib-Apps auf dem Mac und auf dem iPhone so bedienen wie unter Windows mit
Narrator oder NVDA, unter Linux mit Orca und unter Android mit TalkBack: jedes Widget hat eine Rolle, einen Namen,
seinen Zustand und Wert, VoiceOver folgt dem Fokus, liest Text mit dem Cursor und kann klicken, umschalten,
auswählen, Werte setzen und Text bearbeiten. Andere Hilfsmittel von Apple, die dieselbe Schnittstelle lesen
(Schaltersteuerung, Sprachsteuerung, Tastaturbedienung, das Zoom-Fenster, das dem Cursor folgt), profitieren mit.

Nicht Teil dieses Plans: die Textgröße des Systems auf dem iPhone (Dynamic Type, ein offener Punkt von AP5), Brailleschrift
über das hinaus, was VoiceOver aus dem Baum selbst macht, und Apps, die eigenen Inhalt in eine Leinwand zeichnen
(dort gilt `ui.sem_rect`, wie auf den anderen Plattformen).

## Ausgangslage

Die Bibliothek hat den Baum schon; es fehlt nur die Brücke zu Apple ([GUIDE.md](GUIDE.md) §19).

- `lib/ui/a11y.rs` macht aus den semantischen Einträgen jedes Frames (`sem()`) einen `plat::a11y::Snapshot`: pro
  Fenster die Knoten mit Rolle (`Role`: Group, Label, Button, CheckBox, Radio, Switch, Slider, Progress, TextBox,
  Link, List, ListItem, MenuBar, Menu, MenuItem, TabList, Tab, Combo, Image, Canvas, Dialog, Heading), Zuständen
  (`S_*`), Aktionen (`A_*`), Rechteck, Name, Text, Wert/Min/Max/Schritt, Cursor, Auswahl und Cursor-Rechteck
  (`Node::char_rect`: genau am Cursor, sonst geschätzt). Der Schlüssel eines Knotens bleibt von Frame zu Frame gleich.
- `lib/plat/a11y.rs` nimmt den Baum an (`publish`), macht aus den Unterschieden Ereignisse (Fokus, Zustände, Namen,
  Werte, Text, Kinder, Fenster) und gibt die Wünsche des Screenreaders an die UI zurück (`request` / `take`).
- Brücken gibt es für Windows (UI Automation, `win_uia.rs`, `win_uia_text.rs`), Linux (AT-SPI2, `linux_atspi.rs`),
  den Browser und Android (`web_a11y.rs` mit dem Spiegel aus `a11y_wire.rs`). Auf Apple ist `a11y::active()` immer
  falsch, die UI baut also keinen Baum.
- Getestet wird mit `plattest --test a11y` (ohne Fenster: Baum, Aktionen, Unterschiede), `--test a11y-bus` (Linux,
  ein falscher Screenreader auf einem eigenen Bus), `--test uia` (Windows, ein UIA-Client im Testprozess) und dem
  Instrumentierungstest von `androidGallery`.

## Die Brücken für Apple

**macOS (`apple_mac_a11y.rs`):** NSAccessibility. Unsere View `JFView` meldet ihre Kinder; jeder Knoten wird ein
Objekt einer eigenen Unterklasse von `NSAccessibilityElement`, das nach dem Schlüssel des Knotens im zuletzt
veröffentlichten Baum nachschlägt (wie die Elemente unter Windows). Rolle und Untrolle, Bezeichnung, Wert, Rahmen
in Bildschirmkoordinaten, Eltern und Kinder, aktiv, fokussiert, ausgewählt; Aktionen über
`accessibilityPerformPress`, `…Increment`/`…Decrement`, `setAccessibilityValue`, `setAccessibilitySelectedTextRange`,
`setAccessibilityFocused` an `a11y::request`; Textfelder über die Text-Attribute (Wert, Auswahl, Einfügepunkt, Zeilen,
`accessibilityFrameForRange` über `Node::char_rect`). Die Ereignisse des Baums werden zu Mitteilungen
(`NSAccessibilityPostNotification`: Fokus, Wert, Titel, Auswahl, Layout, erzeugt / zerstört). Popups und Dialoge sind
eigene Fenster (`NSPanel`) mit eigenem Baum. Die App-Menüs in der Menüleiste sind seit dem 10.11. echte `NSMenu`s:
VoiceOver liest sie ohne Zutun.

**iOS (`apple_ios_a11y.rs`):** UIAccessibility. `JFView` ist selbst kein Element und meldet `accessibilityElements`:
je Knoten ein Objekt einer Unterklasse von `UIAccessibilityElement` mit Rahmen, Bezeichnung, Wert und Merkmalen
(Traits); Doppeltippen ruft `accessibilityActivate` (Klick, Umschalten, Auswählen), Wischen hoch/runter auf einem
Schieberegler `accessibilityIncrement`/`…Decrement`. Mitteilungen über `UIAccessibilityPostNotification`
(Bildschirm geändert beim Dialog, Layout geändert, Ansage). Textfelder: Doppeltippen fokussiert das Feld und holt die
Bildschirmtastatur (der Weg aus AP5); damit VoiceOver im Text navigieren kann, braucht die View `UITextInput` statt
nur `UIKeyInput` (siehe Entscheidung 4).

**Rollen** (VO0 hat die Spalte macOS festgelegt; T: der Name ist der Titel, B: die Beschreibung `AXDescription`,
W: der Wert):

| Rolle | macOS (Rolle / Untrolle, Name, Wert) | iOS (Traits) |
|---|---|---|
| Label | AXStaticText, W | staticText |
| Heading | AXHeading, T, Wert die Ebene, Rollenbeschreibung „Überschrift“ in der UI-Sprache (zu prüfen: der Rotor außerhalb von Web-Inhalten) | header |
| Button | AXButton, T | button |
| CheckBox | AXCheckBox, T, Wert 0/1/2 (gemischt) | button, Wert „ausgewählt“ |
| Radio | AXRadioButton, T, Wert 0/1 | button, selected |
| Switch | AXCheckBox / AXSwitch, T, Wert 0/1 | toggleButton (iOS 17) |
| Slider | AXSlider, B, Wert, Min, Max, Erhöhen/Verringern um 5 % des Bereichs (wie Android) | adjustable |
| Progress | AXProgressIndicator, B, Wert (keiner, solange unbekannt), Min, Max | updatesFrequently, Wert |
| TextBox | AXTextField / AXTextArea (mehrzeilig) / AXSecureTextField (Passwort), B, Wert der Text | über `UITextInput` |
| Link | AXLink, T | link |
| List / ListItem | AXList, B / AXStaticText, W, ausgewählt | –, selected |
| MenuBar / Menu / MenuItem | AXMenuBar / ein Eintrag der Leiste AXMenuBarItem, ein Popup-Menü AXMenu / AXMenuItem, T | button |
| TabList / Tab | AXTabGroup, B / AXRadioButton (AXTabButton), T, Wert 0/1 | tabBar / button, selected |
| Combo | AXPopUpButton, B, Wert die Wahl, aufgeklappt | button, Wert |
| Dialog | AXGroup, T, Rollenbeschreibung „Dialog“ (ein Dialog im Fenster; ein Dialogfenster ist ein eigenes NSWindow) | modale Gruppe (`accessibilityViewIsModal`) |
| Image | AXImage, B | image |
| Group, Canvas | AXGroup, B | Gruppe (`accessibilityContainerType`) |

## Stufen

### VO0: Abbildung und Probe (S)

- Die Rollentabelle oben endgültig, mit den Zuständen und Aktionen; `a11y::active()` für Apple (Entscheidung 2).
- Eine Probe auf dem Mac: die Gallery mit einem Fenster aus Knöpfen, die VoiceOver vorliest (Hand-Probe des
  Nutzers), dazu die offenen Punkte der Tabelle (Überschriften, Schalter).

**Fertig, wenn:** VoiceOver auf dem Mac die Knöpfe eines Fensters mit Rolle und Namen vorliest und klicken kann.

**Umgesetzt (2026-11-12), Hand-Probe ausstehend:** [`apple_mac_a11y.rs`](../lib/plat/apple_mac_a11y.rs) mit der
Klasse `JFAXElement` (Unterklasse von `NSAccessibilityElement`): ein Objekt je Knoten, beim ersten Fragen gemacht und
behalten, solange der Knoten lebt; die Instanzvariable nennt einen Platz in einer Tabelle (Plattformfenster,
Schlüssel). `JFView` bleibt selbst ignoriert und meldet als Kinder die obersten Knoten, so hängen sie direkt am
Fenster; dazu das fokussierte Element und der Treffertest. Rollen, Namen, Werte, Zustände, Rahmen, Drücken,
Erhöhen/Verringern, Werte setzen, Fokus und Auswahl setzen, die Mitteilungen für Fokus, Werte, Titel, Auswahl und
Layout, „zerstört“ für weggefallene Knoten. Die erste Frage eines Hilfsmittels an eine View schaltet
`a11y::active()` ein (Entscheidung 2). `plattest --test a11y-mac` (46 Prüfungen) liest und bedient die Demo im
echten Fenster; `plattest --test a11y-show` zeigt die Demo für Hand-Proben.

- **`NSAccessibilityElement` antwortet aus einem eigenen Speicher.** Sein `accessibilityAttributeValue:` liest nur
  diesen Speicher (für unser Element: keine Rolle), Aktionsnamen und Setzer kennt es gar nicht. VoiceOver bekommt die
  Antworten über die Methoden des Protokolls, gefiltert mit `isAccessibilitySelectorAllowed:`. Deshalb überschreibt
  die Klasse auch Methoden, die sonst still aus dem leeren Speicher antworten würden (Kinder in
  Navigationsreihenfolge, sichtbare Kinder, Registerkarten, der Aktivierungspunkt), und lässt je Rolle nur die
  passenden Attribute und Aktionen zu (eine Taste hat keinen Wert, kein Element „Menü zeigen“).
- **Der Test im Prozess** liest das Fenster über AppKits `accessibilityAttributeValue:` (dort hängt die ignorierte
  View ihre Knoten ein) und die Elemente über dieselben Protokoll-Methoden nach derselben Regel (`respondsToSelector:`
  und `isAccessibilitySelectorAllowed:`), aus einem Test-Thread per `dispatch_sync` auf dem Hauptthread. Ein
  `AXUIElement` auf den eigenen Prozess geht zwar ohne die Freigabe „Bedienungshilfen“, liefert aber für jedes
  Element nur die App zurück; als Test taugt das nicht.
- **Popups** (Menüs, Listen) bleiben im Baum ihres Besitzers, wie unter Windows; ihr Panel beantwortet nur den
  Treffertest. Ob VoiceOver damit gut zurechtkommt, zeigt die Hand-Probe.

### VO1: Der Baum auf macOS (M)

- Alle Rollen, Namen, Werte, Zustände, die Hierarchie (Fenster, Gruppen, Listen, Tabs), Rahmen, der Fokus.
- Die Ereignisse des Baums als Mitteilungen; Popups und Dialoge als eigene Fenster.

**Fertig, wenn:** `plattest --test a11y-mac` (Entscheidung 3) den Baum der Demo so liest, wie VoiceOver ihn fragt.

### VO2: Aktionen und Text auf macOS (M)

- Drücken, Umschalten, Auswählen, Werte setzen, Aufklappen, Fokus; Textfelder mit Wert, Auswahl, Einfügepunkt,
  Zeilen und Rechtecken (`char_rect`), Auswahl setzen.

**Fertig, wenn:** der Test jede Aktion der Demo auslöst und ihr Ergebnis im nächsten Baum findet, und VoiceOver in der
Gallery einen Text Zeichen für Zeichen liest (Hand-Probe).

### VO3: macOS fertig (S)

- Die Hand-Probe des Nutzers mit VoiceOver: Gallery, ein Dialog, ein Menü, ein Textfeld, die Tastaturbedienung.
- Doku: GUIDE §19 (VoiceOver neben Narrator, Orca, TalkBack), CHANGELOG.

**Fertig, wenn:** die Hand-Probe auf dem Mac nichts Wichtiges mehr findet.

### VO4: iOS (M–L)

- Die Elemente unter `JFView`, Traits, Aktivieren und Einstellen, Mitteilungen; `a11y::active()` folgt dem Schalter
  von VoiceOver (`UIAccessibilityIsVoiceOverRunning` und seine Mitteilung).
- Textfelder: Fokus und Tastatur über VoiceOver; Navigation im Text über `UITextInput` (Entscheidung 4).

**Fertig, wenn:** der Test im Simulator den Baum der Demo über `accessibilityElements` liest und Aktionen auslöst.

### VO5: iPhone fertig (S)

- Die Hand-Probe des Nutzers mit VoiceOver auf dem iPhone 14: Wischen durch die Pocket Gallery, Doppeltippen,
  Schieberegler, Textfeld, Drehen.
- Doku und Website (Roadmap: VoiceOver erledigt, Vergleich ohne die Lücke).

**Fertig, wenn:** die Hand-Probe auf dem iPhone nichts Wichtiges mehr findet.

## Reihenfolge

```text
VO0 -> VO1 -> VO2 -> VO3 (macOS)
              VO4 -> VO5 (iOS), nach VO1, parallel zu VO2/VO3 möglich
```

Aufwand zusammen L. Die Arbeit liegt fast ganz in `lib/plat` (zwei neue Dateien, Anschluss in `apple_mac.rs` und
`apple_ios.rs`); `lib/ui` ändert sich nur, wenn eine Rolle oder ein Zustand fehlt.

## Prüfung

- **Im Prozess, ohne Freigabe:** Ein Test ruft die Methoden unserer Elemente so auf, wie VoiceOver es tut (Kinder,
  Rolle, Wert, Aktionen), und prüft danach den Baum und die Wünsche an die UI. Auf dem Mac in einem echten Fenster
  (`tools/apple.py gui`), auf iOS im Simulator. Höchstens drei Runden pro Variante, keine echte Eingabe.
- **Von außen (wahlweise):** Ein kleiner AX-Client (`AXUIElement`) sieht den Baum genau wie VoiceOver. Dafür braucht
  das Programm, das ihn startet, die Freigabe „Bedienungshilfen“ in den Systemeinstellungen, die nur der Nutzer
  erteilen kann.
- **Von Hand:** VoiceOver auf dem Mac (⌘F5) und auf dem iPhone (Einstellungen → Bedienungshilfen → VoiceOver, oder
  dreimal die Seitentaste) prüft der Nutzer; nur er hört, ob es sich richtig anfühlt.

## Fallen und Risiken

- AppKit fragt die Elemente auf dem Hauptthread, auf dem auch die UI läuft: Die Antworten kommen aus dem zuletzt
  veröffentlichten Baum, nie aus einem laufenden Frame.
- Überschriften: AppKit kennt keine eigene Rolle dafür; Web-Inhalte melden „AXHeading“. Ob der Rotor das außerhalb
  von Safari findet, klärt VO0.
- iOS: Ohne `UITextInput` kann VoiceOver im Text nicht navigieren; das ist dieselbe Arbeit wie der offene Punkt
  „markierter Text der Tastatur“ aus AP5 (Diktat, Vorschläge, zusammengesetzte Zeichen).
- Rahmen: macOS will Bildschirmpunkte mit dem Ursprung unten links, iOS Bildschirmkoordinaten; beide aus den
  Client-Pixeln der Knoten und der Skala des Fensters.

## Entscheidungen

Entschieden (Nutzer, 2026-11-12): alle wie empfohlen, also 1a, 2a, 3a und 4a.

1. **Reihenfolge:** (a, empfohlen) erst macOS (VO0–VO3), dann iOS; (b) beide gleichzeitig.
2. **Wann der Baum gebaut wird:** (a, empfohlen) sobald ein Hilfsmittel fragt, wie unter Windows (`WM_GETOBJECT`),
   auf iOS, solange VoiceOver läuft; (b) immer, wenn eine App ein Fenster hat (einfacher, kostet bei jedem Frame).
3. **Prüfung:** (a, empfohlen) die Tests im Prozess als Gate, dazu die Hand-Proben; der AX-Client von außen nur,
   wenn der Nutzer die Freigabe erteilen mag; (b) zusätzlich den AX-Client als festen Test.
4. **iOS-Text:** (a, empfohlen) `UITextInput` als Teil von VO4: VoiceOver kann im Text navigieren, und Diktat,
   Vorschläge und zusammengesetzte Zeichen kommen mit; (b) zuerst nur Fokus und Tastatur, `UITextInput` später.
