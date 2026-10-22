# Gemeinsame Agentensteuerung und Browserport

Stand: 12. Oktober 2026. Externe Programme und KI-Agenten steuern jfguiLib-Anwendungen
über gemeinsame, explizit aktivierte Schnittstellen. In der Anwendung läuft dafür kein
Sprachmodell. Der Browserport nutzt denselben App-Code, die vorhandenen Widgets und den
Software-Renderer. Diese Übersicht trennt implementierte und geprüfte Funktionen von den
noch offenen Office-Funktionen und Browserdiensten.

## Aufbau

| Baustein | Verantwortung |
| --- | --- |
| `lib/agent_core/src` | Aktionsverträge, Schemas, strukturierte Fehler, Revisionen, JSON-RPC, MCP, Stapel und Aufgaben |
| `lib/agent_io/src` | Begrenzte Zeilenkodierung, stdio, benutzergebundene native IPC-Verbindungen |
| `lib/ui/agent.rs` | Gemeinsame Widget-/Fensterinspektion, Eingabe, Befehle und Bildressourcen; Adapter zu `App` |
| `lib/ui/runtime.rs` | Native Ereignisschleife und schrittweise `Runtime` für gemeinsame Eingabe und Darstellung |
| `lib/plat/capabilities.rs` | Explizite Plattformfähigkeiten und konkrete Gründe für fehlende Dienste |
| `lib/plat/web*.rs` | Browser-Plattformadapter, virtuelle Dateien, Hostanfragen, Bibliotheksfenster und Rechen-Worker |
| `lib/rt`, `lib/simd` | Architekturabhängige native Pfade und portable Implementierungen für WASM |
| `lib/doc_view/src` | Theme-unabhängige Dokumentfonts, Seitendarstellung, Vorschaubilder und Koordinaten |
| `tools/jfgui-agent.py`, `tools/agent_bridge.py` | Gemeinsame CLI, Instanzsuche, native Verbindung, stdio-Gateway und explizite Browser-Bridge |
| `web/loader.js`, `worker.js`, `bridge.js` | Gemeinsamer Browserhost, DOM-Eingabe, Worker, Canvas und aktivierbarer Agentenzugang |
| `web/a11y.js` | ARIA-Schattenbaum über dem Canvas: Spiegel des Barrierefreiheitsbaums, Aktionen von Screenreadern (GUIDE.md, Abschnitt 19) |
| `web/secrets.js` | Geheimnisspeicher des Hosts für `plat::secret`: nicht exportierbarer WebCrypto-Schlüssel pro App und verschlüsselte Werte in IndexedDB (SECRETS.md) |
| `web/audio.js` | Klangausgabe für `plat::audio`: AudioContext und ein AudioWorklet je Stream auf der Seite, die Pumpe im Worker (SharedArrayBuffer-Ring oder gepostete Puffer; GUIDE.md, Abschnitt 33) |

Neue Bibliotheksprojekte nutzen `src/lib.rs`. jfWord nutzt `src/main.rs` mit `app`, `ui`,
`workflows` und `checks`. Andere bestehende Projekte dürfen ihre flachen Einstiegspunkte
behalten. Der zentrale Quellresolver verfolgt Rust-Module und eingebettete Assets vom
Einstiegspunkt aus. Ein gleichzeitiger flacher und modularer Einstieg wird abgelehnt.
Referenzprojekte, Bücher und Build-Ausgaben werden nicht als zusätzliche App-Quellen
eingelesen. Build, Abhängigkeiten, Hashes, Lint, API-Erzeugung und IDE-Projekt verwenden
diese Quellendefinition.

## Aktivierung und native Zugänge

Die allgemeine UI-Runtime unterstützt:

| Startoption | Verhalten |
| --- | --- |
| keine Agentenoption | Kein externer Steuerungsendpunkt |
| `--agent` | Lokaler Endpunkt für die laufende native Anwendung |
| `--agent-json` | Headless-Sitzung mit JSON-RPC über stdin/stdout |
| `--agent-mcp` | Headless-Sitzung mit MCP über stdin/stdout |

Anwendungen können `Ui::set_agent_enabled` auch an eine eigene Einstellung binden. Die
Deaktivierung beendet den Listener und verbundene Sitzungen. `Ui::agent_enabled` und
`Ui::agent_connected` stehen für eine sichtbare Anzeige bereit; jfWord zeigt diese im
Fensterkopf. Protokollantworten gehen über stdout, Diagnosen über stderr.

Unter Windows verwendet der Transport eine lokale Named Pipe mit einer geschützten DACL
für den Benutzer-SID des Prozesses und weist entfernte Clients ab. Unter Linux verwendet
er einen geprüften privaten Ordner `/tmp/jfgui-agent-UID`, Unix-Sockets und eine Prüfung
der Benutzeridentität des Peers. Mehrere aktive Instanzen haben getrennte Adressen und
Protokollzustände. Die native Verbindung öffnet keinen TCP-Port.

Beispiele ab `RustLib`:

```powershell
python tools/jfgui-agent.py list
python tools/jfgui-agent.py actions ENDPOINT --query document
python tools/jfgui-agent.py describe ENDPOINT document.text.replace
python tools/jfgui-agent.py call ENDPOINT document.text.replace --input-file request.json --expected-revision 12
python tools/jfgui-agent.py inspect ENDPOINT --input-file query.json
python tools/jfgui-agent.py stdio ENDPOINT --protocol mcp
python tools/jfgui-agent.py headless build/win/jfWord.exe --protocol json
```

`ENDPOINT` ist die von `list` zurückgegebene genaue Adresse. Die CLI übergibt Argumente als
Argumentliste; sie evaluiert keinen Shelltext. Der `stdio`-Modus verbindet einen externen
Client mit einer bereits aktivierten Instanz. `headless` startet eine neue, ausdrücklich
aktivierte Sitzung. Derzeit ist das Werkzeug ein Python-Standardbibliotheksprogramm;
ein eigenständiger installierter Binärstarter ist noch kein Bestandteil dieser Etappe.

## Aktionsvertrag und Protokolle

`ActionSpec` beschreibt einen stabilen Namen, Beschreibung, Eingabe-/Ausgabeschema,
Plattformfähigkeiten, Lesen/Ändern, Undo sowie Transaktions- und Abbruchunterstützung.
Der `Host` liefert die tatsächliche Bedeutung und prüft Verfügbarkeit. Der `Dispatcher`
prüft Schema, Fähigkeiten, Zustand und Revision, bevor der Handler läuft.

Die UI-Runtime stellt jeder Anwendung sieben allgemeine Aktionen bereit:
`ui.state`, `ui.command`, `ui.focus`, `ui.click`, `ui.type`, `ui.key` und `ui.render`.
Die Eingaben laufen durch die vorhandene Ereignis-/Befehlskette. Deaktivierte Widgets und
durch Modalität blockierte Aktionen werden abgelehnt. Die Inspektion enthält Fenster,
Fokus, Befehle, Fähigkeiten und paginierte Widgetdaten. Die Bibliothek leitet daraus keine
fachliche Bedeutung wie eine Quelle oder Bildebene ab.

Apps registrieren ihre Fachaktionen über `App::actions`, `action_available`, `action`,
`action_revision` und `action_inspect`. Bestehende numerische Befehle können über
`ActionSpec::command_id` zugeordnet werden. Die Umstellung sämtlicher historischer
GUI-/Tastaturbefehle auf registrierte Fachaktionen bleibt appweise Arbeit; die allgemeine
UI-Steuerung erfordert diesen vollständigen Umbau nicht.

JSON-Steuerprotokoll **1** verwendet jeweils ein UTF-8-kodiertes JSON-RPC-2.0-Objekt pro
Zeile. Standardmäßig gilt Version 1; `params.protocolVersion` kann sie ausdrücklich wählen.
Unbekannte Versionen werden abgelehnt. Einzelne Nachrichten sind auf 1 MiB begrenzt;
der Parser begrenzt auch Verschachtelung, Werte, Mitglieder und Stringlängen.

```json
{"jsonrpc":"2.0","id":1,"method":"actions/list","params":{"query":"sources","limit":20}}
{"jsonrpc":"2.0","id":2,"method":"actions/describe","params":{"action":"document.text.replace"}}
{"jsonrpc":"2.0","id":3,"method":"actions/execute","params":{"action":"document.text.replace","input":{"paragraph":1,"start":0,"end":0,"text":"Belegter Text"},"expectedRevision":12}}
```

Weitere gemeinsame Methoden sind `agent/info`, `actions/batch`, `state/inspect`,
`tasks/get`, `tasks/cancel`, `resources/read` und `view/render`. Bilder werden als
Ressourcenreferenzen geliefert und separat begrenzt gelesen. Es gibt keinen allgemeinen
Shell-Ausführungsbefehl und keine freie Dateisystemadresse als Ressourcenkennung.

Verändernde Fachaktionen verlangen die zuletzt gelesene Revision. Stapel laufen geordnet;
ein atomarer Stapel wird vor Ausführung abgelehnt, wenn keine passende gemeinsame
Transaktion registriert ist. Aufgabe, Fortschritt und Abbruch sind gemeinsame Verträge;
lange App-Operationen müssen ihre Unterbrechungspunkte und Commit-Prüfungen implementieren.
Die bloße Registrierung macht einen synchronen Handler nicht nachträglich unterbrechbar.

Der MCP-Adapter unterstützt ausdrücklich **2026-07-28** und die ältere Initialisierung
**2025-11-25**. Moderne Anfragen tragen den Versionseintrag und Clientfähigkeiten in `_meta`;
ältere Clients verwenden `initialize` und `notifications/initialized`. Neun stabile
Werkzeuge dienen Suche, Schemabeschreibung, Ausführung, Stapeln, Inspektion, Aufgabenstatus,
Abbruch, Ressourcen und Darstellung. Fachliche Aktionen werden erst bei Bedarf entdeckt.
Details und konkrete moderne Umschläge stehen in
[agent_core/README.md](../lib/agent_core/README.md). Die Versionsverträge beziehen sich auf
die [MCP-stdio-Spezifikation](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/stdio).

## jfWord und gemeinsame Dokumentdarstellung

jfWord bietet fachliche Aktionen für Dokumentzustand, Textbereiche, Zeichenformatierung,
Absatzvorlagen, Quellen, Zitate, Feldentfernung, Fuß-/Endnoten, Rich-Text-Zwischenablage und
DOCX-Dateisitzung. Diese benutzen die bestehenden `doc_core`-/`doc_io`- und App-Abläufe.
Im Paket-Erhaltungsmodus sind nicht verfügbare strukturierte Bearbeitungsaktionen
entsprechend gesperrt. Das aktuell angebotene Schema ist über die Aktionsbeschreibung
verbindlich; die Protokollanbindung erweitert nicht das unterstützte Dateiformatprofil.

`doc_view` wird von Papieransicht, Vorschau und echten Vorlagen-Vorschaubildern benutzt.
Seine Standard-Dokumentfonts sind unabhängig vom verlinkten GUI-Style eingebettet.
Ein Wechsel von hell/dunkel, UI-Fonts, DPI oder Zoom verändert weder Dokumentformatierung
noch Zeilenumbruch. Der gemeinsame Koordinatenadapter prüft Geometrie und sättigt extreme
Werte, während Vorschaubilder eine feste maximale Pixelzahl einhalten.
Freie Schriftfamilienauflösung und vollständiger Glyphenfallback sind noch offen.

Der eigene jfOffice-Style hat blaue Akzente, einen integrierten Fensterkopf, vollständige
Registerstruktur, ein dichtes Menüband und neutrale Dokumentumgebung. Fehlende Office-
Funktionen bleiben sichtbar deaktiviert. Die Darstellung wurde anhand der Referenzanordnung
neu mit der eigenen Library umgesetzt; es wurden keine WordCraft-Quelltexte oder Assets
übernommen. Bestehende Dokumentfarben und andere Styles bleiben erhalten.

Die gemeinsame Ribbon-API bietet große Aktionen mit Symbol über der Beschriftung,
geteilte Aktionen mit separat fokussierbarem Dropdown sowie benannte Dialogstarter.
Sie nutzt normale Controls, Popup-Verträge und semantische Widget-IDs. jfWord bindet
formatiertes/Nur-Text-Einfügen, Zeichenformat aufnehmen/anwenden/löschen und den Absatzdialog
daran. Formatübertragung ersetzt die sechs vorhandenen effektiven Zeichenwerte, während
Löschen wieder vom Absatzstil erbt. Text, Felder und Systemzwischenablage bleiben erhalten.
Direkte Schriftgrößeneingabe und vorhandene Vorwahlen verwenden denselben Formatweg;
4–144 pt in halben Punkten werden bei erhaltener Textauswahl unterstützt.

Die Navigation zeigt Überschriften, echte Seitenbilder und Suchergebnisse einschließlich
Notentext; Treffer führen zur bestehenden UTF-8-Auswahl im Schreibkern. Fokusmodus blendet
die Bedienbereiche vorübergehend aus und stellt sie wieder her. Zoom 10–500 Prozent,
direkte Prozentangabe, Schieberegler und Seitenbreite verwenden denselben Ansichtszustand.
Diese Funktionen ändern keine Dokumenteigenschaften. `ST_ALIGN_START` ergänzt den
allgemeinen Widgetvertrag; jfOffice stellt damit Navigationszeilen linksbündig dar und
kürzt lange sichtbare Beschriftungen an einer UTF-8-Grenze, ohne ihre Semantik zu kürzen.

Weitere Office-Etappen bleiben offen: frei gewählte Schriftfamilien, benannte/vererbbare
Formatvorlagen, Hervorhebung und Hoch-/Tiefstellung, Formatierungszeichen, interaktive Lineale,
individuelle Tabstopps und mehrere Seiten nebeneinander. Die bisherigen vier Absatzvorlagen
und Seitenansichten bleiben erhalten; ein vollständiger Vorlagenkatalog oder alle in der
Referenz sichtbaren Funktionen werden damit nicht behauptet.

## WebAssembly-Unterbau und Abnahmegrenzen

`build_rs.py app APP --target web` baut für `wasm32-unknown-unknown` mit direktem `rustc`.
`--target all` umfasst Windows, Linux und Web; `both` behält Windows plus Linux.
Das Rust-Ziel und `rust-src` müssen lokal installiert sein. Ausgaben liegen unter
`build/web/APP/`, bei `--out build/NAME` unter `build/NAME/web/APP/`: WASM, gemeinsamer
Loader, Worker, HTML-Einstieg und Host-Styles. Bereitstellung erfolgt über HTTP(S), nicht
über einen lokalen `file:`-URL-Aufruf.
Das Ziel bietet `core` und `alloc`; Betriebssystemdienste kommen vom eigenen Hostadapter
([Rust-Zielbeschreibung](https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-unknown.html)).

Der gemeinsame Browserhost startet die App im Worker und überträgt ihre Software-
Bildausgabe in ein Canvas. `ui::Runtime` verarbeitet Ereignisse und Zeitschritte mit
denselben Layout- und Zeichenpässen. Ein Browserport soll keine zweite HTML-Oberfläche
oder separate Fachlogik einer App erhalten. Der Standard-Build erzeugt drei Varianten:
Skalar, SIMD128 und SIMD128 mit gemeinsamem Speicher/Atomics. Der Host prüft die Features
und lädt automatisch die passende Variante. `lib/simd/wasm.rs` implementiert die gemeinsame
Vektorschnittstelle; vorhandene Grafik-, Codec- und Effektkerne nutzen sie ohne App-Umbau.
Der Thread-Build verwendet den bestehenden `lib/plat/pool.rs`-Scheduler mit Browser-
Semaphoren und vorab gestarteten Web Workern. Jeder Worker hat einen eigenen Stack,
Bilddaten und Job-Daten liegen im gemeinsamen linearen Speicher. `parallel_for`, `Job`,
Abbruch und verschachtelte Schleifen behalten ihre bisherigen Verträge. Die anderen
Varianten verwenden weiterhin den kooperativen `web_pool.rs`-Fallback.

Gemeinsamer Speicher benötigt Cross-Origin-Isolation. `python tools/serve_web.py
build/web/PaintCraft` setzt die benötigten COOP-/COEP-Header auf localhost. Ohne Isolation
bleibt SIMD mit einem App-Worker verfügbar. Der Standard nutzt bis zu acht zusätzliche
Worker (Hardware-Threads minus eins); `?compute=scalar|simd|threads&workers=4` erlaubt
kontrollierte Vergleiche. Details zu Build, Lebensdauer, Grenzen und reproduzierbaren
Tests stehen in [WASM_COMPUTE.md](WASM_COMPUTE.md). Numerische Gleichheit wird mit dem
portablen Corpus und denselben PaintCraft-Bildern geprüft.

Der Host sendet höchstens einen quittierten Eingabestapel gleichzeitig. Die Ausgabe wird
am Browser-Bildtakt angestoßen; überholte, aufeinanderfolgende Hover-Positionen dürfen sich
ersetzen. Gedrückte Maus-/Stiftbewegungen, Druckwerte, Tasten und Texteingaben bleiben in
Reihenfolge erhalten. `jfgui_input` führt ihre App-/Layoutpässe aus, ohne Zwischenbilder
zu rasterisieren; `jfgui_tick` zeichnet anschließend den aktuellen Zustand. Das verhindert
eine Warteschlange alter Bilder bei schnellen Eingabefolgen. Verdeckte Tabs verwenden
einen Timer, damit Speicherbestätigungen auch ohne sichtbaren Animationsframe ankommen.

Der Fensteradapter hält Fenster- und Kompositionspuffer vor und berücksichtigt die von
`ui` gelieferten Änderungsrechtecke. Der Worker konvertiert und überträgt nur diese
Bildbereiche; der Host gibt deren Puffer nach `putImageData` zur Wiederverwendung zurück.
Die Canvas-Abmessungen werden ausschließlich bei echten Größenänderungen gesetzt.
Fenster-/Popup-Geometrieänderungen erzwingen weiterhin eine vollständige Komposition,
damit zuvor verdeckte Bereiche zuverlässig wieder erscheinen. Native Ereignisschleifen
behalten ihre bisherige direkte Darstellung.

`plat::Capabilities` unterscheidet Dateilesen/-schreiben, dauerhaften Speicher,
Zwischenablageformen, Druck, Systemfonts, native Plugins, lokale IPC, Fenster, Threads und
Systemintegration. Fehlende Fähigkeiten liefern eine konkrete Begründung. Die virtuelle
Dateiablage und Request-/Completion-Schnittstellen verbinden vorhandene Codecs mit dem
Browserhost. Ein Schreiben in flüchtige Dateien ist kein Nachweis dauerhafter Speicherung.
Erfolgreiche Persistenz darf erst nach bestätigtem Abschluss gemeldet werden.

Template, Gallery, jfWord, Kritzel und PaintCraft sind in Chromium und Firefox mit dem gemeinsamen
Browserhost geprüft. Die Prüfungen umfassen echte Rust-Canvas-Ausgabe, Größenänderung und DPI, Pointer-
Ereignisse, Bibliotheksdialoge, DOM-Texteingabe und Composition-Ereignisse sowie wissenschaftliche
Dokumentaktionen und bestätigtes Speichern. jfWord markiert eine Browserdatei erst nach
erfolgreicher Persistenzbestätigung als gespeichert; inzwischen erfolgte neue Änderungen
bleiben ungespeichert. Die verfügbaren Dokumentfonts decken CJK noch nicht vollständig ab:
korrekt übernommener Unicode-Text ist kein Nachweis vollständiger Glyphendarstellung.
PaintCraft verwendet im Browser statisch eingebundene Filter und Codecs; externe native
DLL-/SO-Plugins werden dort nicht geladen. Grafikaktionen prüfen Ebenen, Größenänderung,
Undo und PNG-Bildressourcen gegen die tatsächlich dargestellten Canvas-Pixel.
Zusätzliche Tests lösen Quota- und Zugriffsfehler am echten IndexedDB-Zugang aus und
verzögern dessen Commit-Bestätigung während Bearbeitung beziehungsweise Dokumentwechsel.
Alle acht Fälle erhalten den korrekten ungespeicherten Zustand. Sie belegen die Behandlung
der Hostfehler; sie ersetzen keine Prüfung jeder Browser-/Betriebssystem-Speicherpolitik.

Die vollständige browserseitige Dienstabnahme bleibt offen. Echte betriebssystemseitige
IME-Sitzungen und komplexe Schriften, alle Drag-and-drop-Varianten,
systemweite Zwischenablage, Schriftzugriff und Drucken benötigen
weitere Nachweise. Native DLL-/SO-Erweiterungen, Registry und Betriebssysteminstallation
werden nicht als Browserfunktionen behauptet. Ein vorhandener Adapter, ein erfolgreiches
WASM-Build oder die unten genannten Abläufe bedeuten keine vollständige
Plattformgleichheit.

## Aktivierter Browserzugang und optionale Bridge

Der Browserhost aktiviert Agentensteuerung über seine sichtbare Schaltfläche oder die
ausdrückliche URL-Option `?agent=1`. Nur dann ist `window.jfguiAgent` verfügbar. `request`
übergibt JSON-RPC an denselben Rust-Dispatcher; `connect(port)` bindet einen ausdrücklich
übergebenen `MessagePort`. `disconnect()` deaktiviert die Schnittstelle und schließt
zugehörige Verbindungen. Unterschiedliche Sitzungen besitzen getrennte Protokollzustände.

CLI/MCP kann sich über die optionale lokale Bridge verbinden. Sie startet ausschließlich
mit `python tools/jfgui-agent.py browser-bridge --origin ORIGIN`, bindet an `127.0.0.1`
und erzeugt ein frisches Zugriffstoken. Die ausgegebene Adresse und das Token werden einer
bereits aktivierten Browser-App ausdrücklich mit `jfguiAgent.connectBridge({url,token})`
übergeben. `browser-call` und `browser-stdio` verbinden externe Clients. Ein genauer
Originvergleich, begrenzte Warteschlangen und ein Token schützen diesen eigenen Transport;
er bietet keine Shellaktion. Die normale Web-App benötigt diesen Dienst nicht.
Aufrufdetails und Grenzen stehen in [agent_core/README.md](../lib/agent_core/README.md).

## Prüfungen und aktuelle Nachweise

Die Browsertests (`*.mjs` mit Playwright) laden Playwright über `tools/tests/playwright.mjs`:
zuerst `PLAYWRIGHT_MODULE` (Paketordner, z. B. `.../node_modules/playwright`), sonst die
normale Node-Modulsuche ab Arbeitsverzeichnis und Repository (`npm install playwright`,
bei globaler Installation mit `NODE_PATH`). Firefox kommt aus `build/browser-engines`
(`PLAYWRIGHT_BROWSERS_PATH`). Welches Chromium startet, entscheidet `chromiumOptions()` aus
derselben Datei: `CHROME_PATH` (Pfad zu einer Chrome- oder Chromium-Programmdatei), falls
gesetzt; sonst ein installiertes Google Chrome (Playwrights Kanal `chrome`, an den üblichen
Orten unter Windows, Linux und macOS); sonst das von Playwright installierte Chromium
(`npx playwright install chromium`; mit `PLAYWRIGHT_BROWSERS_PATH=build/browser-engines`
landet es wie Firefox dort).

- `apps/agenttest`: Schema-, Revisions-, Stapel-, Aufgaben-, Ressourcen- und Protokollverträge.
- `apps/agenttest/tests/ipc_check.py`: getrennte native Prozesse, private Endpunkte, Instanzen,
  CLI und MCP-Gateway. Unter Windows und Linux jeweils mit dem jeweiligen Build ausführen.
- `apps/jfWord/tests/agent.py`: echte Prozesse über JSON und MCP; Bearbeiten, Formatieren,
  Quelle, Zitat, Note, Undo/Redo, DOCX-Speichern/Öffnen und Bildressource.
- `tools/tests/web_apps.mjs`: Chromium-/Firefox-Prüfungen für gemeinsame Darstellung,
  Eingaben und App-Aktionen für alle fünf GUI-Apps. Ergebnisse und Browserbilder liegen
  unter `build/office-universal/browser-tests`.
- `tools/tests/web_frames.mjs BUILD`: echte WASM-/Worker-Prüfung der Reihenfolge von
  Zeichenbewegungen, Stiftdruck, Undo/Redo und Größenänderungen. Stapel und einzeln
  verarbeitete Ereignisse müssen dieselben Pixel ergeben; Teilbildkomposition wird
  gegen erzwungene Vollbildkomposition geprüft, auch bei 150/200 Prozent DPI.
- `tools/tests/web_responsiveness.mjs BUILD [BASELINE_WEB]`: Chromium/Firefox, fünf
  GUI-Apps, Canvas-Reset-Zähler, schnelle Hover-Folgen, künstlich verzögerter Worker,
  Begrenzung auf einen laufenden Stapel und 200-Prozent-DPI. Der optionale Vergleich
  verwendet einen vorhandenen alten Web-Bundle-Ordner. Ergebnisse unter
  `BUILD/responsiveness/`; Zeitmessungen sind Szenarienwerte, keine allgemeine FPS-Garantie.
- `apps/agenttest/tests/bridge_check.py` und `browser_bridge.mjs`: unabhängige Transport-
  beziehungsweise echte Browserprüfung der expliziten CLI/MCP-Bridge.
- `jfWord --self-test`: nach der Struktur-/Darstellungsetappe unter Windows und Linux
  jeweils **9410 bestanden, 0 fehlgeschlagen**. Enthält neue Vergleiche für unabhängige Dokumentfonts,
  identische Layout-/Thumbnailpixel, Koordinatengrenzen und wiederholten Dokumentwechsel.
- Native jfWord-Darstellung: 16 Prüfkonfigurationen für Start-/Papieransicht, hell/dunkel,
  100/150/200 Prozent sowie 1000/1240/1920 DIP Breite. `--verify-dirty` und `expect_nowarn`
  bestanden; repräsentative Bilder wurden visuell geprüft.
- `apps/jfWord/tests/navigation.uis`: Überschriften- und Seitensprünge, Suche und genaue
  UTF-8-Auswahl, Groß-/Kleinschreibung und ganze Wörter, formatübergreifende Notensuche,
  direkte Zoomangabe und Grenzen 10/500 Prozent, Fokusmodus und Wiederherstellung der
  Seitenbereiche. Der Test läuft im normalen nativen jfWord-Prüfsatz mit.
- Der vollständige native jfWord-UI-Satz unter `build/jfoffice-view`: **35 Skripte je Ziel,
  70 erfolgreiche Prozessläufe**, jeweils `--verify-dirty`, ohne `UIWARN`. Bestehende
  Mausauswahlprüfungen verwenden die neue Seitengeometrie; Rich-Text-, Feld-, Noten-,
  Quellen- und Dateifunktionen bleiben Bestandteil des Satzes.
- `ribbon_actions.uis` prüft große/geteilte Aktionen, Dropdown-/Menütastatur und Rückkehr,
  den Absatz-Dialogstarter, Formatübertragung und Undo sowie deaktivierte Aktionen.
  Verschachtelte Überläufe und 600/1000/1240 DIP bei 100/150/200 Prozent wurden angesehen.
- `font_size.uis` prüft direkte Größen mit halben Punkten, Eingabevalidierung und Grenzen,
  Undo/Redo, erhaltene Auswahl sowie die gemeinsame Aktualisierung mit der Vorwahlliste.
- `build_rs.py test --quick --target both --out build/office-universal`: **295/295 Skripte**
  bestanden, einschließlich nativer Plugins und aktueller API-Dokumentation. Der danach
  ergänzte Formatkürzel-Fall in `ribbon_actions.uis` wurde auf beiden Zielen erneut bestanden.
- `apps/jfWord/tests/check.py --target both --build-dir build/office-universal`:
  **1017 native Prozessprüfungen**, ergänzt um unabhängige OPC-/ZIP-/XML-Auswertung und
  beschädigte Eingaben. Die neuen Formatübertragungsprüfungen untersuchen außerdem
  DOCX, native Fragmente, RTF und HTML.
- Die native Assembly-Referenz bleibt unverändert: **158/158 UI-Vergleiche** für Gallery,
  Template und Kritzel sowie **78.414 bytegleiche Rechenkern-/Codec-Ergebnisse**. Dies umfasst
  Windows/Linux, hell/dunkel, 100/150 Prozent und Standard/Classic/KDE/macOS. Details und
  Binärhashes: `build/agent-review/NATIVE_PARITY.md`.
- Agentenkern: **249 Prüfungen je nativem Ziel**, IPC **18 Windows/19 Linux**, wirkliche
  Gallery-/Kritzel-Verbindungen **15 je Ziel**, Bridge-Transport **26** und echte Browser-
  CLI-/MCP-Verbindungen **14 je Browser**. jfWord-Protokollabläufe bestehen unter JSON und
  MCP auf beiden nativen Zielen; zusätzliche Revisions-/Fehlerregressionen **8 je Ziel**.
- Der portable Korpus liefert unter Windows, Linux und WASM denselben Digest
  `5a22a1f803872742`. Transzendente Funktionen besitzen gesonderte Toleranzprüfungen;
  dies ist keine Behauptung bitgleicher Ergebnisse beliebiger Fließkommafunktionen.
- Finale Browsermatrix: **18/18 Fälle** in Chromium und Firefox, zusätzlich **8/8**
  IndexedDB-Fehler-/Konkurrenzfälle. Dokumentkürzel werden über DOM-Tastenereignisse bis
  zur Rust-Seite geprüft; Abschalten widerruft auch bereits erhaltene Agentenreferenzen.
- `tools/tests/web_transfer.py`: **4/4** Browser-DOCX aus beiden Browsern werden unter
  Windows und Linux geöffnet und bytegleich wieder exportiert (je 5038 Bytes). Das
  Prüfdokument enthält Unicode, Zeichenformate und Quellen; ein beliebiges fremdes
  Word-Dokument wird durch dieses begrenzte Beispiel nicht als vollständig unterstützt belegt.

Die vorhandenen nativen Format- und Darstellungsprüfungen bleiben erhalten. Vollständige
Word-Kompatibilität, vollständige UI-/CLI-/MCP-Gleichheit jedes Befehls und universelle
Browserabnahme bleiben ausdrücklich nachzuweisende Ziele.

Die etappenbezogene Zuordnung steht in
[jfWord/docs/IMPLEMENTATION.md](../apps/jfWord/docs/IMPLEMENTATION.md).
