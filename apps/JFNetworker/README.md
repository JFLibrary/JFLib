# JFNetworker

Ein Netzwerksimulator und Labor für verteilte Systeme, gebaut als Alternative zu Cisco Packet
Tracer: Geräte und Kabel auf einer Zeichenfläche, echte Protokolle Byte für Byte, eine
IOS-artige Befehlszeile für Router und Switches, eine Linux-artige Shell für Hosts, ein
Simulationsmodus Schritt für Schritt mit Paketansicht, und später Dienste, mit denen sich
horizontal skalierte Systeme aufbauen lassen (Loadbalancer, geshardete Datenbanken, Caches,
Queues, Lastgeneratoren, eigene Skripte).

**Stand (2026-10-20): Phase 1 ist fertig** (siehe [Fahrplan](#fahrplan)).

- **Topologie-Editor:** 26 herstellerneutrale Gerätemodelle aus der Palette ziehen, klicken oder
  per Tastatur platzieren; verbinden mit Port-Auswahl und automatischem Kabeltyp (Kupfer,
  gekreuzt, Glasfaser, seriell, Konsole, Funk, WAN), Kabel mit LEDs, Wegpunkten und Verlauf,
  Notizen, Rückgängig für alles, Projekte als `.jfnet`.
- **Simulator im Editor:** läuft in Echtzeit mit und folgt jeder Änderung. Switching mit VLANs,
  IPv4 mit ARP, Routing (auch L3-Switches), ICMP, Ping und Traceroute, UDP und TCP (RFC 9293),
  DHCP mit Relay, DNS, Syslog, NTP, HTTP mit Lastgenerator; die Probleme-Liste zeigt doppelte
  Adressen, fehlende Gateways, Schleifen und anderes.
- **Konsolen:** IOS-artig auf Routern und Switches (Modi, `?`, Tab, `show`-Befehle, `ip dhcp
  pool`, `copy run start` als Rückgängig-Schritt), eine Linux-Shell auf Hosts (`ip`, `ping`,
  `traceroute`, `dhclient`, `curl`, `nc`, `ss`, `top`), als Tab, eigenes Fenster oder im
  Gerätefenster mit Formularen und Desktop.
- **Simulationsmodus:** Pakete als Umschläge auf den Kabeln, Schritt vor und zurück entlang der
  Aufzeichnung, Ereignisliste mit Zeitleiste, Paket-Tab mit Schichten, Feldern, Bytes und
  Prüfsummen, eigene PDUs, Export als pcapng.
- **Labor für verteilte Systeme:** Loadbalancer, geshardeter KV-Speicher mit Replikaten und
  Quoren, Cache, Message Queue mit Workern und Fehlerinjektion mit Marken auf der Zeitleiste,
  bisher über die Shell (`lbctl`, `kvctl` …) und Szenarien (siehe
  [Labor für verteilte Systeme](#labor-für-verteilte-systeme)).
- **Politur (U5):** Deutsch und Englisch, Einstellungen, automatische Sicherung mit
  Wiederherstellung, vollständige Tastaturbedienung, Geräte und Kabel für Screenreader,
  geprüfter Kontrast, Tastenkürzel-Übersicht, App-Symbol und Paketierung (Dateityp `.jfnet`);
  flüssig auch mit 500 Geräten, 800 Kabeln und 300 Paketen in Bewegung (unter 33 ms pro Bild in
  1080p, siehe [Performance](#performance)).
- **Skripte in Hosts (SK4):** eigene Programme in der Skriptsprache `.jfs` laufen als Dienste:
  UDP, TCP, HTTP-Server und -Client, DNS, Timer, Disk und Metriken, ihre Anweisungen kosten
  Rechenzeit wie auf einer CPU mit einem Thread; `script` in der Shell zeigt und steuert sie
  (siehe [Skripte in Hosts](#skripte-in-hosts)).
- **Labore und Aufgaben (U6b):** Datei › Labor öffnen … bringt vier Labore mit (ein VLAN mit
  DHCP-Relay im Campus, ein dritter Webserver hinter dem Loadbalancer, ein KV-Speicher, der
  einen Ausfall übersteht, ein Cache vor einem langsamen Server); das Panel „Aufgabe“ zeigt
  Einleitung, Ziele als Checkliste, die der Simulator laufend prüft, Hinweise und Fortschritt;
  die Einstellungen der Dienste lassen sich im Gerätefenster als JSON bearbeiten (siehe
  [Labore und Aufgaben](#labore-und-aufgaben)).
- **Skript-Editor (U7):** der Reiter Skripte im Gerätefenster eines Hosts: Code-Editor mit
  Prüfung beim Tippen und den Fehlern des laufenden Programms an ihrer Zeile, Ausführen,
  Stoppen, Übernehmen als Rückgängig-Schritt (das Skript behält Disk und Zahlen), Ausgabe und
  Fehler des Laufs, Vorlagen (siehe [Skript-Editor](#skript-editor)).

## Bedienung

- **Geräte platzieren:** eine Kachel der Palette auf die Zeichenfläche ziehen, oder anklicken und
  dann auf die Fläche klicken (Umschalt: mehrere), oder per Tastatur: Strg+1 springt in die
  Gerätesuche, Tab zu den Kacheln einer Kategorie, die Pfeiltasten wählen, Eingabe setzt das
  Gerät in die Mitte der Ansicht.
- **Verbinden (C):** von Gerät zu Gerät ziehen oder beide anklicken; per Tastatur Strg+9 (die
  Topologie), Tab zum ersten Gerät, Eingabe, Tab zum zweiten, Eingabe – die Port-Auswahl öffnet
  sich unter dem zweiten Gerät, Eingabe verbindet das beste Paar. Umschalt verbindet sofort.
- **Konsole:** Doppelklick oder Eingabe auf einem Gerät öffnet seine Konsole im Terminal-Bereich
  (Strg+3), mit Umschalt das Gerätefenster.
- **Simulation:** F5 Simulationsmodus, Umschalt+F5 Echtzeit, F6 Abspielen/Anhalten, F7 Schritt
  vor, Umschalt+F7 zurück; das Werkzeug P sendet eine einfache PDU (Quelle, dann Ziel – auch mit
  Tab und Eingabe).
- **Tastatur:** Strg+1 … 8 zeigen die Bereiche und geben ihnen die Tastatur, Strg+9 die
  Topologie; dort wählt Tab die Geräte und Notizen, dann die Kabel (Eingabe auf einem Kabel: seine
  Eigenschaften). Werkzeugleiste und Palette sind Fokusgruppen (ein Tab-Halt, Pfeiltasten
  innen). F1 zeigt alle Tastenkürzel.
- **Einstellungen** (Werkzeuge › Einstellungen, Strg+,): Raster und Rasterweite, Einrasten,
  Minimap, Tempo beim Start, Bewegung (wie das System, reduziert, voll), Sprache (wie das
  System, Deutsch, English), automatisches Sichern (Sekunden, 0 aus). Sie liegen in
  `config://settings.ini`.
- **Automatische Sicherung:** ungespeicherte Änderungen landen alle 120 s (einstellbar) und
  sofort beim Ruhezustand oder Abmelden in `data://autosave/`; nach einem Absturz bietet die
  nächste Sitzung sie in einer Leiste über der Zeichenfläche an (Wiederherstellen, Verwerfen,
  Später). Jede laufende Sitzung hält eine Sperre, ein zweites Fenster bietet ihre Kopie nicht an;
  eine wiederhergestellte Kopie bleibt, bis die neue Sitzung ihre eigene geschrieben hat. Wer
  beim Beenden „Verwerfen“ wählt, verwirft auch die Kopie.
- **Barrierefreiheit:** Geräte und Kabel sind für Screenreader Listeneinträge („Kabel PC1 eth0 –
  SW1 Gi1/0/1, Kupfer, gerade, aktiv“), die Leinwand heißt „Topologie“; die Farben der
  Pakete und Kabel halten in beiden Themes und im Hochkontrast mindestens 3:1 gegen die Fläche
  (Selbsttest). Die meisten Kabelarten unterscheiden sich auch in der Linie (gekreuzt
  gestrichelt, Konsole und Funk gepunktet, seriell mit Blitz, WAN doppelt), Pakete nennen
  Inhalt und Weg für Screenreader und in der Ereignisliste; „reduzierte Bewegung“ lässt Pakete
  springen statt gleiten.

## Bauen und testen

```
python build_rs.py app JFNetworker
build/win/JFNetworker.exe                                   # der Editor
build/win/JFNetworker.exe --open apps/JFNetworker/tests/fixtures/campus.jfnet
build/win/JFNetworker.exe --lang en                         # auf Englisch
build/win/JFNetworker.exe --selftest                        # Prüfungen von Modell, Simulator, Oberfläche
build/win/JFNetworker.exe --selftest --pcap-out DIR         # dazu die Mitschnitte der Prüfungen
build/win/JFNetworker.exe --script apps/JFNetworker/tests/place.uis --verify-dirty
build/win/JFNetworker.exe --gen-topology 500 --traffic 300  # synthetischer Campus, 300 Pings (Leistung)

build/win/jfn-sim.exe run apps/JFNetworker/tests/fixtures/lan_router.jfnet --scenario apps/JFNetworker/tests/scen/lan_router.scn
build/win/jfn-sim.exe run apps/JFNetworker/tests/fixtures/lab_shards.jfnet --scenario apps/JFNetworker/tests/scen/lab_metrics.scn --series build/lab.csv
build/win/jfn-sim.exe check apps/JFNetworker/tests/fixtures/lan_router.jfnet
build/win/jfn-sim.exe decode "ff ff ff ff ff ff 02 4a 00 00 01 01 08 06 ..."
build/win/jfn-sim.exe gen 500 --out build/campus.jfnet
build/win/JFNetworker.exe apps/JFNetworker/tests/fixtures/two_pcs.jfnet --scenario apps/JFNetworker/tests/scen/two_pcs.scn
python tools/i18n_check.py apps/JFNetworker crates            # jeder Text der Crates hat sein Englisch
```

`python build_rs.py test --quick` führt alle Skripte in `tests/` aus, die Fensterskripte mit
`--verify-dirty` (Teil- und Vollbild pixelgleich); die meisten nehmen Bilder hell, dunkel und
mit 150 % nach `build/jfnetworker/` auf.

| Skript | prüft |
|---|---|
| `model.uis` | den Selbsttest: Katalog, Interface-Namen, Kabelregeln, Projekt, Datei, Szenarien und die Gruppen `sim`, `l3`, `simmode`, `svc`, `cli`, `tcp`, `http`, `console`, `pdu`, `lab` (das Labor für verteilte Systeme), `ui` (Kontrast, Einstellungen, Sicherungen, englischer Katalog), `msg` (Texte von Modell und Simulator), `campus` und `synth` (die synthetischen Projekte der Leistungstests), `script` (Skripte in Hosts) |
| `model.uis`, Gruppe `series` | die Zeitreihen: Raten, Stände und Latenzen jeder Sekunde eines Testdienstes auf den Wert genau, Zeiträume, Intervall und Anzahl, Lücken, spätere, ersetzte und entfernte Dienste, Kabel, CSV, dass das Abtasten am Lauf nichts ändert, die Szenario-Syntax, KPIs und Shard-Last im Labor |
| `scen_*.uis` | die Golden-Szenarien (siehe [Szenarien](#szenarien-scn)), byte-gleich auf Windows und Linux |
| `shell.uis` | Bereiche, Menüs, Ansicht, Info |
| `catalog.uis`, `place.uis`, `edit.uis`, `links.uis`, `undo.uis` | Palette und Suche, Platzieren, Verschieben, Rahmenauswahl, Löschen, Umbenennen, Notizen, Kabel beim Löschen und Wiederherstellen, Rückgängig auf allen Wegen |
| `file.uis`, `file_reopen.uis` | Speichern unter `build/jfn_test.jfnet`, die Rückfrage, Öffnen, zuletzt geöffnete Dateien (`file_reopen.uis` läuft nach `file.uis`) |
| `connect.uis`, `cable_types.uis`, `waypoints.uis`, `inspector.uis`, `engine.uis` | Verbinden und Port-Auswahl, jede Kabelart, Wegpunkte, die Eigenschaften, LEDs und Befunde des Simulators |
| `terminal_ios.uis`, `terminal_linux.uis`, `device_window.uis`, `log.uis` | die Konsolen, Gerätefenster mit Formularen und Desktop, das Protokoll |
| `simulation.uis`, `events.uis`, `packet.uis`, `pdu.uis` | Schritte, Ereignisse und Zeitleiste, Paket-Tab, PDUs und pcapng-Export |
| `a11y.uis` | alles ohne Maus: Kabel und Geräte als Einträge, Tab über Geräte und Kabel, Platzieren aus der Palette, Verbinden per Eingabe, Konsole, Simulation mit F5/F7 und der PDU per Eingabe, die Tastenkürzel |
| `settings.uis` | den Einstellungsdialog: Raster, Einrasten, Minimap, Tempo, Sprache (das Fenster auf Englisch und zurück), Bewegung, Sicherungsintervall |
| `autosave.uis`, `autosave_recover.uis`, `autosave_reopen.uis` | eine Kopie nach dem Intervall und beim Ruhezustand, die Kopie überlebt das Sitzungsende; die nächste Sitzung stellt sie wieder her und hält sie, bis sie eine eigene hat (ein zweiter Absturz verliert nichts); Beenden mit „Verwerfen“ nimmt die Kopien mit (die drei laufen in dieser Reihenfolge) |
| `english.uis` | das Fenster mit `--lang en`: Menüs, Bereiche, Eigenschaften, Simulationsmodus, Dialoge |
| `rtl.uis` | ein gespiegeltes Fenster: Bereiche gespiegelt, die Topologie wie gezeichnet bedienbar |
| `shots.uis` | die Bild-Matrix der Hauptansichten (Topologie, Konsole, Simulation, Paket-Tab, Gerätefenster) hell und dunkel mit 100 und 150 % |
| `campus.uis`, `scen_campus.uis` | die Abnahme von Phase 1 (unten) |
| `perf.uis` | die Leistung: 500 Geräte, 800 Kabel und 300 Rahmen in 1920 × 1080 mit höchstens 33 ms pro Bild in Echtzeit, beim Abspielen, Verschieben, Zoomen und Ziehen (siehe [Performance](#performance)) |
| `model.uis`, Gruppe `task` | die Labore: der Abschnitt `lab` der Projektdatei (Texte in beiden Sprachen, ohne Labor wie bisher), was `Lab::issues` findet, einzelne Prüfungen, `goal.rs` mit denselben Berichtszeilen wie der Runner und den Gründen samt Englisch, erreichte Ziele, die bleiben, geordnete Labore; die vier Labore aus `assets/labs` ohne Befund, ungelöst ohne erreichtes Ziel, gelöst wie im Fenster mit jedem Ziel, die Quoren der Shards (zwei Replikate mit strong überstehen keinen Ausfall, drei schon), dieselben Zeitpunkte zweimal |
| `task.uis` | das Panel „Aufgabe“ am Labor des Loadbalancers: leer ohne Labor, die Liste der Labore, Titel und Ziele der Reihe nach, die Einleitung zum Einklappen, „Prüfen“ mit den Befunden, Hinweise nacheinander, der Fortschritt; gelöst im Fenster: das Kabel zu WEB3, das dritte Backend in den Einstellungen von LB1 (Text ohne JSON mit Zeile und Spalte abgelehnt, eine Einstellung, die der Simulator ablehnt), alle Ziele erreicht; Rückgängig lässt sie erreicht, „Simulation zurücksetzen“ fängt neu an |
| `labs.uis` | die Labore auf Englisch (`--lang en`): die Liste, das Panel mit den Texten der Labordatei, F9 mit den Befunden, ein Hinweis, Strg+Umschalt+A; das Cache-Labor über die Einstellungen von CACHE1 gelöst, dann (nach „Änderungen speichern?“ mit Verwerfen) das Campus-Labor in den Konsolen von MLS1, R1, PC3 und PC4 |
| `task_shots.uis` | Bilder der Liste der Labore, des Panels (das Labor der Shards nach „Prüfen“ mit einem Hinweis) und der Einstellungen eines KV-Knotens, hell und dunkel mit 100 und 150 % (`build/jfnetworker/task_*.png`) |
| `script_editor.uis` | den Skript-Editor (siehe [Skript-Editor](#skript-editor)) auf `fixtures/scripts.jfnet`: der Reiter aus dem Kontextmenü, die Liste mit Zustand und Fehlern, der Laufzeitfehler in Zeile 13 als Marke und in der Fehlerliste (ein Klick setzt die Schreibmarke), die Ausgabe live, Tippen mit Prüfung nach einem Augenblick, F8 und Umschalt+F8, Strg+S lehnt einen Fehler ab und übernimmt, was sich übersetzen lässt (das Skript startet mit seiner Disk neu: „Start 2“), Rückgängig und Wiederholen („Start 3“, „Start 4“), Stoppen, Ausführen, Neu starten, Verwerfen, eine Vorlage (ihr Fehler: Port 7 ist belegt), Umbenennen, Löschen und Rückgängig, der Reiter Dienste |
| `script_editor_en.uis` | den Skript-Editor auf Englisch: Kontextmenü, Reiter, Liste, Statuszeile, Meldungen, Ausgabe und Fehler, eine Vorlage auf PC1 mit englischen Kommentaren und Texten, die Tasten in der Übersicht (F1) |
| `script_shots.uis` | die Bilder des Skript-Editors hell und dunkel mit 100 und 150 % (`build/jfnetworker/script_shots_*.png`) |
| `model.uis`, Gruppe `scripted` | worauf der Skript-Editor baut: der Quelltext einer Definition als Zeilen oder Text und zurück, Namen, die Vorlagen (übersetzbar, Deutsch und Englisch), Übernehmen, Neu, Umbenennen und Löschen über die Rückgängig-Liste mit Disk und Zahlen des laufenden Skripts und ohne Neustart des Geräts, ein Skript, das der Simulator aus der Datei nicht nehmen konnte, `keep_steps`, die Marken auf bearbeitetem Text |

Fixtures in `tests/fixtures/`: `office.jfnet` (ein kleines Büronetz mit jeder Kabelart),
`cables.jfnet` (Geräte ohne Kabel), `pdu_lab.jfnet` (DHCP, DNS-Zone `lab`, TCP-Echo),
`campus.jfnet` (die Abnahme), `scripts.jfnet` (zwei Skripte auf SRV1 für den Skript-Editor) und
die Netze der Golden-Szenarien. Die Bausteine aus `lib/` prüft
`coretest` (Gruppen `netpkt`, `dhcp`, `dnsmsg`, `ntp`, `httpmsg`, `rng`, `metrics`, `jmap`,
`cmdtree`, `hashring`, `lru`), die Skriptsprache die Gruppe `script`.

**Abnahme Phase 1: der Campus** (`tests/fixtures/campus.jfnet`). MLS1, ein L3-Switch, routet
VLAN 10 (Staff, PC1 und PC2) und VLAN 20 (Students, PC3 und PC4) über seine VLAN-Interfaces und
reicht DHCP mit `ip helper-address` an R1 weiter. R1 hängt über 10G-Glasfaser an MLS1, vergibt
die Adressen aus zwei Pools (`ip dhcp pool` mit Gateway, DNS-Server und Domäne `lab`,
ausgeschlossene Bereiche) und führt ins Servernetz zu SRV1 (DNS-Zone `lab` mit `server.lab`,
Zeitquelle für R1 und MLS1). Drei Tests gehören dazu: das Golden-Szenario `scen_campus.uis`
(siehe [Szenarien](#szenarien-scn)), die Selbsttest-Gruppe `campus` (das Fixture selbst: Leases
über das Relay, Namen, `dhclient`, die DHCP-Befehle, NTP, ARP vor ICMP mit korrekten
Prüfsummen auf jedem Kabel) und `campus.uis` im Fenster: die Leases, PC1s Shell mit `dhclient -v`,
`/etc/resolv.conf`, `ping` und `traceroute` per Name, `show ip route` und `show mac
address-table` auf MLS1, `show ip dhcp binding` auf R1, im Simulationsmodus eine einfache PDU
von PC2 zu SRV1 Schritt für Schritt (ARP-Anfrage, Antwort von MLS1, dann das Echo, das MLS1 an
R1 weiterroutet) und der Paket-Tab mit den Prüfsummen von IPv4 und ICMP. Die Bilder für die
Abnahme landen als `build/jfnetworker/campus_*.png` (Topologie, Konsolen, `show`-Ausgaben,
Simulationsschritte, Paket-Tab; einige dunkel und mit 150 %).

## Aufbau

| Teil | Ort | Inhalt |
|---|---|---|
| Pakete | [`lib/netpkt`](../../lib/netpkt/lib.rs) | Adressen, Prüfsummen, Codecs von Ethernet bis TCP, DHCP, DNS, NTP, Syslog, HTTP/1.1 (inkrementell, [`http.rs`](../../lib/netpkt/http.rs)), Dissektor, pcapng (Library, ohne I/O) |
| Metriken | [`lib/base/metrics.rs`](../../lib/base/metrics.rs) | Counter, Gauge, log-lineares Histogramm, Fenster 300 × 1 s für p50/p95/p99 |
| Zufall | [`lib/base/rng.rs`](../../lib/base/rng.rs) | deterministische Zufallszahlen (PCG32) für reproduzierbare Läufe |
| Befehlsbaum | [`lib/base/cmdtree.rs`](../../lib/base/cmdtree.rs) | IOS-artige Befehlszeilen: Abkürzungen, `?`, Tab, Modi, `no`, typisierte Parameter, Fehlerposition |
| Hash-Ring, LRU | [`lib/base/hashring.rs`](../../lib/base/hashring.rs), [`lib/base/lru.rs`](../../lib/base/lru.rs) | Consistent Hashing mit vnodes und Präferenzlisten; LRU-Tabelle nach Anzahl und Gewicht |
| JSON | [`lib/agent_core/src/jmap.rs`](../../lib/agent_core/src/jmap.rs) | `record!` / `wire_enum!` für das Projektformat |
| Datenmodell | [`crates/jfn_model`](crates/jfn_model/lib.rs) | Gerätekatalog, Interface-Namen, Kabelregeln, Projekt, Szenarien, synthetische Testprojekte ([`synth.rs`](crates/jfn_model/synth.rs)) |
| Simulator | [`crates/jfn_sim`](crates/jfn_sim/lib.rs) | Ereignis-Kern, Kabel mit Laufzeit/Verlust/Warteschlangen, Bridge mit VLANs, Hub, IPv4 mit ARP, Routing und ICMP, Ping- und Traceroute-Jobs, virtuelles OS mit UDP- und TCP-Sockets ([`tcp.rs`](crates/jfn_sim/tcp.rs)) und Diensten (DHCP, DNS, Resolver, Syslog, NTP, TCP-Testdienste in [`tcpsvc.rs`](crates/jfn_sim/tcpsvc.rs), HTTP in [`http.rs`](crates/jfn_sim/http.rs) und [`httpsvc.rs`](crates/jfn_sim/httpsvc.rs)), Trace, Zustand für die Leinwand, Treiber; Simulationsmodus, pcapng-Export, Probleme-Liste; Befehlszeilen in [`cli/`](crates/jfn_sim/cli/mod.rs) (`curl`, `nc`, `ss`, `top` in [`tools.rs`](crates/jfn_sim/cli/tools.rs), `lbctl`, `kvctl` … in [`lab.rs`](crates/jfn_sim/cli/lab.rs)), Szenarien in [`runner.rs`](crates/jfn_sim/runner.rs); das Labor: Loadbalancer, KV-Speicher, Cache, Queue und Worker ([`lb.rs`](crates/jfn_sim/lb.rs), [`kv.rs`](crates/jfn_sim/kv.rs), [`cache.rs`](crates/jfn_sim/cache.rs), [`mq.rs`](crates/jfn_sim/mq.rs)), Disk ([`disk.rs`](crates/jfn_sim/disk.rs)), Fehlerinjektion ([`fault.rs`](crates/jfn_sim/fault.rs)) |
| | [`crates/jfn_sim/series.rs`](crates/jfn_sim/series.rs) | Zeitreihen auf virtueller Zeit: jede Zahl jedes Dienstes und der Verkehr der Kabel jede Sekunde, KPIs, Shard-Last, Fehler als Markierungen, Werte über Zeiträume für Szenarien, CSV (siehe [Metriken und Zeitreihen](#metriken-und-zeitreihen)) |
| Runner | [`crates/jfn_run`](crates/jfn_run/lib.rs), [`sim/main.rs`](sim/main.rs) | `jfn-sim` und `--scenario` der GUI-Exe: Dateien lesen, Bericht, pcapng, Zeitreihen als CSV, Vergleich mit Golden-Dateien |
| App | [`src/app`](src/app/mod.rs) | Start (auch `--selftest` und `--scenario`), Zustand (`Net`), Befehle und Menüs, Kennungen, Einstellungen ([`prefs.rs`](src/app/prefs.rs): Schema über `base::settings`), Sprachen ([`tr.rs`](src/app/tr.rs): Katalog, übersetzte Befehle und Menüs) |
| | [`src/workspace`](src/workspace/mod.rs) | Fensterrahmen, Palette, Eigenschaften (`inspector/`: Gerät, Kabel, Projekt), Zeichenfläche (`canvas/`: Szene und Zeichnen, Werkzeuge, Verbinden, Kabel), Simulationsmodus ([`sim/`](src/workspace/sim/mod.rs): Steuerung, Animation, Ereignisse, Paket, PDUs und Export), Symbole und UI-Icons (`icons/`, erzeugt von `tools/vsym.py` und [`mkicons.py`](mkicons.py)), Terminal-Bereich und Konsolenfenster ([`terminal.rs`](src/workspace/terminal.rs)), Protokoll ([`logview.rs`](src/workspace/logview.rs)), Gerätefenster ([`devwin/`](src/workspace/devwin/mod.rs): Übersicht, Schnittstellen, Konfiguration, Dienste, CLI, Desktop), die Dialoge der Einstellungen und der Tastenkürzel ([`settings.rs`](src/workspace/settings.rs)) |
| | [`src/doc`](src/doc/mod.rs) | das offene Projekt mit Rückgängig (`edits.rs`), Dateien und Dialoge (`file.rs`), der Simulator dahinter (`sim.rs`), Einstellungen, die die Konfiguration eines Geräts ändern (`config.rs`), die Konsolen der Geräte und die Meldungen des Simulators an sie ([`console.rs`](src/doc/console.rs)), Sicherungen und ihre Wiederherstellung ([`autosave.rs`](src/doc/autosave.rs)) |
| | [`src/sim_thread.rs`](src/sim_thread.rs) | Treiber mit eigenem Thread für Echtzeit und lange Läufe (nur nativ) |
| | [`src/selftest.rs`](src/selftest.rs), [`src/simtest.rs`](src/simtest.rs), [`src/l3test.rs`](src/l3test.rs), [`src/simmodetest.rs`](src/simmodetest.rs), [`src/svctest.rs`](src/svctest.rs), [`src/clitest.rs`](src/clitest.rs), [`src/tcptest.rs`](src/tcptest.rs), [`src/httptest.rs`](src/httptest.rs), [`src/consoletest.rs`](src/consoletest.rs), [`src/pdutest.rs`](src/pdutest.rs), [`src/labtest.rs`](src/labtest.rs), [`src/uitest.rs`](src/uitest.rs), [`src/campustest.rs`](src/campustest.rs), [`src/synthtest.rs`](src/synthtest.rs) | Selbsttests von Modell, Simulator, Befehlszeilen, TCP, HTTP, PDUs, des Labors, dem, worauf die Konsolen bauen, der Oberfläche ohne Fenster (Kontrast, Einstellungen, Sicherungen, Katalog), des Campus und der synthetischen Projekte (`--selftest`) |
| | [`src/seriestest.rs`](src/seriestest.rs) | Selbsttest der Zeitreihen und der Erwartungen über Zeiträume (Gruppe `series`) |
| Texte | [`crates/jfn_model/msg.rs`](crates/jfn_model/msg.rs), [`src/locales/engine.tsv`](src/locales/engine.tsv), [`src/locales/ui.tsv`](src/locales/ui.tsv), [`src/msgtest.rs`](src/msgtest.rs) | die deutschen Texte von Modell und Simulator als Vorlagen mit Argumenten (`Msg`, `msg!`; `text_with` übersetzt), die des Fensters (`t()`, `tf()`, die Befehle und Menüs), ihre englische Fassung und deren Selbsttest; Vollständigkeit: `python tools/i18n_check.py apps/JFNetworker crates` |
| Tests | [`tests/`](tests) | Skripte, Fixtures (`fixtures/*.jfnet`), Szenarien mit Golden-Berichten (`scen/*.scn`, `scen/*.report`) |
| Skripte | [`crates/jfn_sim/scriptsvc.rs`](crates/jfn_sim/scriptsvc.rs), [`cli/scripts.rs`](crates/jfn_sim/cli/scripts.rs), [`src/scripttest.rs`](src/scripttest.rs) | Skripte als Dienste in den Hosts: Host-API, Anweisungen als Rechenzeit, der Befehl `script`, ihr Selbsttest (siehe [Skripte in Hosts](#skripte-in-hosts)) |
| Labore | [`crates/jfn_model/lab.rs`](crates/jfn_model/lab.rs), [`crates/jfn_sim/goal.rs`](crates/jfn_sim/goal.rs), [`src/workspace/task.rs`](src/workspace/task.rs), [`src/workspace/devwin/svcedit.rs`](src/workspace/devwin/svcedit.rs), [`assets/labs`](assets/labs), [`src/tasktest.rs`](src/tasktest.rs) | die Aufgabe eines Labors in der Projektdatei, das Urteil über eine Erwartung (Runner und Labor) und die Ziele über einen Lauf, das Panel „Aufgabe“ mit der Liste der Labore, die Einstellungen der Dienste als JSON, die vier Labore, ihr Selbsttest (siehe [Labore und Aufgaben](#labore-und-aufgaben)) |
| Skript-Editor | [`src/workspace/devwin/scripts.rs`](src/workspace/devwin/scripts.rs), [`src/doc/scripts.rs`](src/doc/scripts.rs), [`src/scriptedtest.rs`](src/scriptedtest.rs) | der Reiter Skripte des Gerätefensters, die Skripte eines Geräts im Projekt mit Rückgängig und Vorlagen, ihr Selbsttest (siehe [Skript-Editor](#skript-editor)) |

### Gerätekatalog

Herstellerneutrale, moderne Modelle: PC (2.5G), Laptop, Smartphone und Tablet (WLAN 802.11ax mit 6 GHz),
Drucker, IP-Telefon und IP-Kamera (PoE), IoT-Sensor, Smart-TV; Access-Switch 24× 1G PoE+ mit
4× 10G SFP+, L3-Switches (1G und 10GBASE-T mit 100G QSFP28), Edge- und Core-Router (bis 100G,
serielle WAN-Ports), Heimrouter mit WLAN, Hub; Next-Generation-Firewall; WLAN-Access-Point (802.11ax, 6 GHz),
Modem/ONT, Internet-Wolke; Rack- und Tower-Server; Loadbalancer, DB-Shard, Cache, Message Queue,
Lastgenerator, Container-Host. Netzwerkgeräte tragen IOS-Interface-Namen
(`GigabitEthernet1/0/1`, Abkürzungen wie `gi1/0/1` gehen überall), Hosts Linux-Namen (`eth0`,
`wlan0`).

### Kabel

`cable_kind` nennt das richtige Kabel für zwei Ports: Kupfer gerade oder gekreuzt (nach MDI /
MDI-X; mit Auto-MDI-X funktionieren beide), Glasfaser für SFP/SFP+/SFP28 untereinander und
QSFP28 mit QSFP28, seriell, Konsolenkabel (Konsole an USB), Funk (WLAN-Gerät an Access Point)
und WAN-Leitungen. Beim schnellen Verbinden wählt `suggest_ports` das Portpaar: Funk, wenn ein
Access Point beteiligt ist, sonst die schnellste Datenverbindung mit den kleinsten Portnummern.

### Im Editor: Verbinden, Kabel, Inspektor

- **Verbinden** (Werkzeug C): von einem Gerät auf ein anderes ziehen oder beide nacheinander
  anklicken. Das Ziel ist in Akzentfarbe umrandet, wenn ein freier Port dazu passt, sonst rot
  mit dem Tooltip „Kein passender Port“. Beim Loslassen öffnet sich am Zeiger die Port-Auswahl:
  beide Geräte mit allen Ports (Typ-Symbol, Name, Speed, Zustand), belegte mit ihrer
  Gegenstelle, Ports ohne Partner mit dem Grund; das beste Paar ist gewählt, darunter das Kabel
  („Automatisch: Kupfer, gerade“ oder ein anderes, das passt); Enter verbindet. Umschalt beim
  Loslassen oder ein Zug von Port-Punkt zu Port-Punkt (beim Überfahren erscheint je
  Portgruppe der nächste freie Port als Punkt am Rand) verbindet sofort.
- **Kabel** laufen vom Rand des einen Geräts zum Rand des anderen, gerade, rechtwinklig oder
  als Kurve durch ihre Wegpunkte, und zeigen ihre Art nicht nur über die Farbe: Kupfer
  durchgezogen, gekreuzt gestrichelt, Glasfaser orange, seriell mit Zickzack, Konsole
  gepunktet, Funk dünn gepunktet, WAN doppelt. 12 Einheiten vor jedem Ende sitzt die LED des
  Ports aus dem Simulator (grün aktiv, rot unten, grau ohne Strom, bernstein für blockierend
  und lernend); ab 100 % stehen die Portnamen daneben. Der Tooltip nennt Ports, Art, Speed,
  Zustand und Pakete pro Sekunde. Ein gewähltes Kabel hat „+“-Griffe in der Mitte seiner
  Abschnitte (ziehen fügt einen Wegpunkt ein, Doppelklick auf einen Wegpunkt entfernt ihn),
  das Kontextmenü wechselt Verlauf und Kabeltyp und trennt; Entf oder das × neben einem
  überfahrenen Kabel trennt es. Alles ist rückgängig zu machen.
- **Inspektor:** ein Gerät mit Name, Modell, Strom und seinen Schnittstellen, wie der Simulator
  sie führt (Status in der Farbe der LED, IPv4-Adresse, Gegenstelle, Speed, VLAN, MAC); die
  gewählte Schnittstelle bekommt schnelle Einstellungen (Adresse mit Präfix, DHCP, aktiv, bei
  Hosts das Standard-Gateway). Sie gelten sofort und landen in der Startkonfiguration des
  Geräts, die der Simulator dafür schreibt; Rückgängig startet das Gerät mit der vorigen neu.
  Ein Kabel zeigt seine Enden, Zustand, Typ („Automatisch“ oder gewählt), Bandbreite,
  Verzögerung, Verlust, Stecker und Verlauf; mehrere Objekte lassen sich gemeinsam ein- und
  ausschalten.
- **Simulator im Editor** ([`src/doc/sim.rs`](src/doc/sim.rs)): `Engine::from_project` beim
  Öffnen und bei Neu, danach wird jede Änderung über die Rückgängig-Liste als `Action`
  nachgeführt (Geräte, Namen, Strom, Kabel, Kabeltyp, Stecker, Bandbreite, Verzögerung,
  Verlust); Rückgängig und Wiederholen schicken die Gegen-Actions, ein gelöschtes Gerät kommt
  mit seiner laufenden Konfiguration zurück. `jfn_sim::Inline` läuft über einen UI-Timer in
  Echtzeit, in Skriptläufen auf der virtuellen Uhr (`wait`), daher sind die Tests
  wiederholbar. Alle Befehle an den Simulator gehen über `Sim::send`, seine Meldungen holt
  `Sim::take_reports` ab (für die Konsolen und den Simulationsmodus).

### Konsolen, Gerätefenster, Protokoll

- **Konsolen** (Terminal-Bereich; [`src/workspace/terminal.rs`](src/workspace/terminal.rs),
  [`src/doc/console.rs`](src/doc/console.rs)): Ein Doppelklick auf ein Gerät (auch Enter auf
  der Zeichenfläche, „Konsole öffnen“ im Kontextmenü oder im Inspektor) öffnet seine
  Befehlszeile als Tab. Jedes Gerät hat genau eine Konsole mit 5000 Zeilen Rückblick, Verlauf
  (↑/↓), Suche (Strg+F) und den Meldungen seit dem Start. Eine Zeile geht als `Cmd::Cli` an das
  Gerät, seine Ausgabe kommt als `Report::CliOut`; Prompt und Modus liefert das Gerät, ein
  laufender Befehl hält die Eingabe zurück (vorab Getipptes läuft danach). IOS-artig: `?` zeigt
  die Hilfe aus dem Befehlsbaum der Engine, Tab vervollständigt, Fehler zeigen mit `^` auf die
  Stelle, Strg+Umschalt+6 bricht `ping` und `traceroute` ab, Strg+Z verlässt die
  Konfiguration, und was ein Befehl ausgibt, blättert mit `--More--` (24 Zeilen pro Seite;
  `terminal length N` ändert das, `0` schaltet es ab). Linux-Shell: Tab listet mehrere
  Möglichkeiten, Strg+C bricht ab, Strg+L leert. Tastenkürzel des Editors und Rückgängig
  bleiben dabei außen vor. Tabs lassen sich umbenennen, schließen und in eigene Fenster lösen
  („Konsolen nebeneinander“ verteilt alle über die untere Fensterhälfte; Schließen eines
  Konsolenfensters dockt die Konsole wieder an). Ein ausgeschaltetes Gerät sagt es in seiner
  Konsole und lässt sich dort einschalten.
- **Startkonfiguration:** `copy running-config startup-config` (und `write memory`) landet als
  Rückgängig-Schritt im Projekt; der Titel zeigt die ungespeicherte Änderung, und beim
  Speichern geht die Konfiguration in die Datei. Rückgängig und Wiederholen ändern nur, womit
  das Gerät startet: Seine laufende Konfiguration bleibt (`Doc::apply_kept`, `Sim::keep`).
  `hostname` benennt das Gerät im Projekt um; einen Namen, den das Projekt ablehnt, nimmt das
  Gerät zurück.
- **Gerätefenster** (Umschalt+Doppelklick, „Gerätefenster …“ im Kontextmenü oder Inspektor;
  [`src/workspace/devwin/`](src/workspace/devwin/mod.rs)): Übersicht (Modell, Ports, Strom,
  Zustand, Adressen), Schnittstellen (Zustand, IPv4, Gegenstelle, Speed, VLAN, MAC,
  Paketzähler), Konfiguration (Formulare für Name, Adresse, Beschreibung, aktiv, Access-VLAN,
  DHCP, Standardroute und statische Routen; sie tippen die passenden Befehle in die Konsole –
  `interface …`, `ip address …`, `ip route …` bzw. `ip addr add …`, `ip route add default via
  …` – und werden ein Rückgängig-Schritt in der Startkonfiguration; „Laufende Konfiguration
  sichern“), Dienste (DHCP, DNS, Syslog, NTP, TCP-Testdienste mit Zustand und Zahlen; die des
  Projekts schalten sich sofort ein und aus, auch rückgängig ohne Neustart, und ihre
  „Einstellungen …“ sind JSON, siehe [Labore und Aufgaben](#labore-und-aufgaben)), CLI (dieselbe
  Konsole wie im Terminal-Tab) und bei Hosts ein Desktop: IP-Konfiguration (statisch oder
  DHCP), Eingabeaufforderung, Texteditor (`notizen.txt`: die Notizen des Geräts, gespeichert
  im Projekt, bis der Simulator ein Dateisystem hat) und ein Text-Browser, der schon Adressen
  und Ports auflöst und Seiten zeigt, sobald der Simulator HTTP spricht. Eine Konsole erscheint
  an einer Stelle: Zeigt das Gerätefenster sie, verweist der Tab dorthin. Offen sind höchstens
  zwei Gerätefenster und drei Konsolenfenster.
- **Protokoll** ([`src/workspace/logview.rs`](src/workspace/logview.rs)): die Log-Zeilen aller
  Geräte als `Report::Log` – Syslog-Meldungen ihrer Dienste (dhclient, dhcpd, NTP …), die
  IOS-artigen Meldungen `%LINK`, `%LINEPROTO` und `%SYS` mit der Schwere aus ihrem Code und mit
  `debug` jede Entscheidung (Schwere 7, das Teilsystem als Quelle) – mit Zeit, Gerät, Schwere,
  Quelle und Meldung; Filter nach Gerät, Schwere („… und wichtiger“) und Text, „Neueste zeigen“
  folgt neuen Zeilen, ein Doppelklick zeigt das Gerät auf der Zeichenfläche. Die Schrift der
  Konsolen ist DejaVu Sans Mono (Lizenz: `lib/doc_view/assets/DejaVu-LICENSE.txt`).

### Projektdatei `.jfnet`

Eingerücktes JSON; fehlende Felder gelten als Vorgabe, unbekannte werden übersprungen. Die
Konfiguration eines Geräts ist sein startup-config-Text, den der Simulator durch die
Befehlszeile einspielt. Positionen sind Weltkoordinaten (DIP bei 100 %): bei Geräten die linke
obere Ecke ihres 64 × 48 großen Symbols, bei Notizen die der Karte. `layout` hält die Ansicht
beim Speichern (`zoom_pm` in Promille, Mitte `cx`, `cy`, Raster `grid`); fehlt sie, zeigt das
Programm beim Öffnen alles. Ein Labor hat dazu den Abschnitt `lab` mit seiner Aufgabe (siehe
[Labore und Aufgaben](#labore-und-aufgaben)), ein freies Projekt dort `null`.

```json
{
  "format": "jfn",
  "version": 1,
  "title": "Büro",
  "seed": 42,
  "next_id": 5,
  "devices": [
    { "id": 1, "name": "PC1", "model": "pc", "x": 0, "y": 0, "startup_config": "" },
    { "id": 3, "name": "SW1", "model": "jf-s24p", "x": 50, "y": 100 }
  ],
  "links": [
    { "id": 4, "a": { "dev": 1, "port": "eth0" }, "b": { "dev": 3, "port": "GigabitEthernet1/0/1" },
      "cable": "copper", "kbps": 1000000, "delay_ns": 50 }
  ]
}
```

### Simulator

Ein Ereignis-Kern über virtueller Zeit in Nanosekunden; gleiche Projekte und gleicher Seed
ergeben byte-gleiche Läufe. Ein Kabel braucht für einen Frame die echte Sendezeit (Frame,
mindestens 60 Byte, plus FCS, Präambel und Pause: 1 Gbit/s und 100 Byte ergeben 992 ns) plus
Laufzeit; Switches speichern und leiten weiter, lernen MAC-Adressen (Aging 300 s), fluten
unbekannte Ziele nur an aktive Ports und trennen VLANs (Access, Trunk mit Native-VLAN und
erlaubter Liste, Tags auf dem Trunk). Router-Ports starten wie unter IOS abgeschaltet. Jeder
Frame auf jedem Kabel und jede Entscheidung eines Geräts landet in der Aufzeichnung, aus der
später der Simulationsmodus, der PDU-Inspektor und der pcapng-Export lesen.

IPv4 läuft auf Ports, 802.1Q-Subinterfaces (Router-on-a-stick), den Switch-eigenen
Interfaces (`Vlan10`, unter Linux `br0`), gerouteten Switch-Ports und Loopbacks:

- **ARP:** drei Anfragen im Sekundenabstand. Linux hält bis zu 101 Pakete zurück und meldet
  danach „Host unreachable“. IOS verwirft das Paket, das die Anfrage auslöst; daher kommt das
  bekannte `.!!!!` beim ersten Ping.
- **Routing:** der längste Präfix gewinnt, dann die Distanz, dann die Metrik. Statische Routen
  gehen über einen nächsten Hop, auch rekursiv, oder über ein Interface. Ein L3-Switch routet
  erst nach `ip routing`.
- **Weiterleiten:** TTL, Fragmentieren und Zusammensetzen, „Fragmentation needed“ mit der MTU
  des nächsten Abschnitts. Gerichtete Broadcasts bleiben draußen. IOS-Interfaces beantworten
  ARP für Hosts hinter einem anderen Interface (Proxy-ARP). Serielle Leitungen tragen
  Cisco-HDLC.
- **ICMP-Fehler:** nach RFC 1812, also nie über Fehler, Broadcasts oder Folgefragmente.
  Absender ist das Eingangs-Interface. Linux zitiert bis zu 576 Byte des Pakets, IOS den Kopf
  und 8 Byte. Die Rate ist begrenzt wie im Original: IOS schickt höchstens alle 500 ms ein
  Unreachable, daher `U.U` und Sternchen im Traceroute. Linux schickt 50 auf einmal und dann
  1000 pro Sekunde, an ein Ziel 6 auf einmal und dann eins pro Sekunde.
- **Ping und Traceroute:** Jobs über virtuelle Zeit mit exakten Laufzeiten. Ein Ping über einen
  Switch mit 1 Gbit/s braucht 4104 ns.
- **Loopback:** auf Linux-Geräten ist ganz 127.0.0.0/8 das Gerät selbst. Kommt so eine Adresse
  über ein Kabel, wird das Paket verworfen.

### Simulationsmodus

Im Simulationsmodus läuft die Uhr nur auf Knopfdruck. Ein Schritt geht bis zum nächsten
Frame, der auf ein Kabel geht, auf Wunsch nur für bestimmte Protokolle (ARP, ICMP, …) oder
Geräte. „Schritt zurück“ wandert die Aufzeichnung entlang und zeigt Kabel, Ereignisse und
Pakete zu diesem Zeitpunkt; die Geräte behalten dabei ihren heutigen Zustand (den Zustand von
damals bringen Keyframes in Phase 4). Der nächste Schritt geht durch die Aufzeichnung wieder
vor, „Zurück zu live“ springt sofort in die Gegenwart. Für die Oberfläche liefert der
Simulator:

- **Paketanimation:** welche Frames zu einem Zeitpunkt auf welchem Kabel sind und wie weit sie
  zwischen Senden und Empfang gekommen sind (`frames_at`), dazu, wo etwas verworfen wurde
  (`drops_between`, das rote ×).
- **Ereignisliste:** jede Entscheidung eines Geräts als Zeile mit Zeit, „Von“, „Bei“,
  Protokoll, Schicht und deutschem Text (`event_rows`), filterbar nach Protokollen und Geräten.
  Hosts melden auch, dass sie eine ARP-Anfrage nach einer fremden Adresse verwerfen.
- **Paket-Tab:** was ein Gerät mit einem Frame gemacht hat, also eingehende PDU, Entscheidungen
  und ausgehende PDUs (`step_of`, `hop_steps`). Jeder Frame kennt den Frame, der ihn ausgelöst
  hat. `hop_layers` zerlegt die Bytes, bei seriellen Leitungen samt Cisco-HDLC-Kopf.
- **Eigene PDUs** ([`pdu.rs`](crates/jfn_sim/pdu.rs)): ein Dienst schickt UDP-Datagramme mit
  eigener TTL und Größe oder öffnet TCP-Verbindungen, einmal, n-mal oder bis er gestoppt wird,
  auch an Namen, die der Resolver des Geräts erst auflöst. ICMP-Echos sind Ping-Jobs,
  DNS-Abfragen Lookups des Resolvers. `Cmd::Seek(t)` stellt die Ansicht auf jeden Zeitpunkt der
  Aufzeichnung.

**Im Fenster** ([`src/workspace/sim`](src/workspace/sim/mod.rs)):

- **Steuerung:** in der Werkzeugleiste Echtzeit | Simulation, Abspielen/Anhalten (F6), Schritt
  zurück (Umschalt+F7) und vor (F7), Tempo 0,25× bis 8×; dasselbe im Menü Simulation, dazu
  „Zurück zu live“, „Nur Auswahl“ und „Simulation zurücksetzen“ (ein frischer Simulator aus dem
  Projekt). In Echtzeit läuft die Uhr mit dem Tempo, im Simulationsmodus geht jeder Schritt zum
  nächsten Zeitpunkt, an dem ein gezeigter Frame ein Gerät verlässt; Abspielen macht einen
  Schritt pro Sekunde bei 1×. Steht die Ansicht in der Vergangenheit, sagt ein Hinweis über der
  Zeichenfläche „Ansicht bei t = …“ mit dem Knopf „Zurück zu live“. Die Statusleiste nennt
  Modus, Uhr und Ereignisse.
- **Pakete auf der Leinwand:** kleine Umschläge in gedämpften Farben je Protokoll (ICMP grün,
  ARP bernstein, DNS violett, TCP/HTTP blau, UDP cyan, DHCP petrol, STP grau, OSPF/RIP braun,
  Rest schiefergrau), unter 50 % Zoom Punkte, dort auf dem Kabel, wo der Frame zum gezeigten
  Zeitpunkt ist. Ein Schritt bewegt sie vom alten zum neuen Zeitpunkt (Ankommende laufen in ihr
  Ziel und verblassen, neue erscheinen am Absender); Verworfenes zeigt ein rotes ×, das danach
  ausblendet. Reduzierte Bewegung springt ohne Übergang. Ein Klick wählt den Frame und öffnet den
  Paket-Tab. Gezeichnet wird über der Leinwand; ihr eigenes Bild bleibt im Cache, ein
  Animations-Timer läuft nur, solange sich etwas bewegt oder abgespielt wird.
- **Ereignisse:** Chips je Protokoll und „Nur Auswahl“ blenden Frames überall aus (Liste,
  Zeitleiste, Leinwand, Schritte); die Auswahl der Protokolle speichert das Projekt. Darunter
  die Zeitleiste mit einer Spur je Protokoll und dem Abspielkopf, darunter die Liste (#, Zeit
  in Sekunden auf die Nanosekunde, Von, Bei, Typ, Info). Beide folgen einander: eine gewählte
  Zeile zeigt ihren Zeitpunkt, ein bewegter Abspielkopf wählt die letzte Zeile davor. Im
  Simulationsmodus passt sich die Zeitleiste den Ereignissen an, bis man selbst zoomt.
- **Paket:** links der Schritt des Geräts (eingehender Frame, Entscheidungen mit Schicht,
  ausgehende Frames; ein Klick zeigt den jeweiligen Frame), in der Mitte der Schichtenbaum (L1
  mit Ports, Zeiten, Länge und Kabel, dann Ethernet II, 802.1Q, IPv4, TCP/UDP/ICMP und die
  Anwendung mit allen Feldern und Problemen wie falschen Prüfsummen), rechts die Hex-Ansicht in
  den Farben der Schichten. Ein Feld im Baum wählt seine Bytes, ein Klick auf ein Byte sein
  Feld; die Zeile über den Bytes nennt beides.
- **PDUs:** das Bedienfeld „PDUs“ (rechts, erscheint mit dem Simulationsmodus) listet die
  eigenen PDUs mit Zustand („läuft: 1 von 2 gesendet …“, „fertig: …“, „gescheitert: …“) und
  Knöpfen zum erneuten Senden, Stoppen und Entfernen. Einfache PDU: Werkzeug P, Quelle und Ziel
  anklicken, ein ICMP-Echo an die Adresse des Ziels. Komplexe PDU: Quelle, Ziel als Gerät,
  Adresse oder Name, ICMP-Echo, UDP, TCP oder DNS-Abfrage, Ports, Größe, TTL, Anzahl, Abstand,
  periodisch.
- **Export:** Datei › Exportieren › „Aufzeichnung als pcapng …“ schreibt alle aufgezeichneten
  Frames für Wireshark.

### Mitschnitt (pcapng)

`Engine::pcapng` schreibt die Aufzeichnung als pcapng-Datei für Wireshark: alles (eine
Schnittstelle pro Kabel, „PC1 eth0 – SW1 Gi1/0/1“), einen gefilterten Teil (Protokolle,
Geräte, Zeitfenster), ein Kabel oder ein Gerät (eine Schnittstelle pro Port; Gesendetes mit
der Sende-, Empfangenes mit der Empfangszeit). Zeitstempel sind die virtuelle Zeit in
Nanosekunden. Ethernet-Kabel bekommen den Linktyp Ethernet, serielle Leitungen Cisco HDLC. Ein
Kommentar pro Frame nennt beide Enden und Verluste auf der Leitung. Die Mitschnitte der
Selbsttests (`--selftest --pcap-out DIR`) zerlegt tshark 4.6 vollständig und ohne
Prüfsummenfehler.

### Probleme

`Engine::problems()` prüft den Aufbau und nennt Gerät, Port, Schweregrad und einen deutschen
Text. Netze sind dabei die Broadcast-Domänen der verkabelten Topologie (Kabel, VLANs, Trunks,
Tags, Hubs); für Schleifen zählen nur Kabel, die gerade aktiv sind.

| Problem | Schweregrad |
|---|---|
| doppelte IP-Adresse im selben Netz | Fehler |
| kein Standard-Gateway, obwohl ein Router im Netz ist | Warnung |
| Gateway außerhalb der eigenen Netze / im Netz nicht zu finden | Fehler / Warnung |
| Netzmasken passen im selben Netz nicht zusammen | Warnung |
| Native VLAN verschieden, Access-Ports in verschiedenen VLANs | Warnung |
| Trunk ohne gemeinsames erlaubtes VLAN | Fehler |
| Access-Port gegen Trunk oder gegen Router-Subinterfaces | Warnung |
| falsches Kupferkabel bei strengem MDI-X | Fehler |
| Schleife aus Switches oder Hubs (Spanning Tree fehlt noch) | Fehler |
| Interface mit Adresse, aber abgeschaltet | Warnung |

### Treiber

`jfn_sim::Inline` treibt den Simulator auf dem UI-Thread, deterministisch auch in Skripten.
`src/sim_thread.rs` ist die Variante mit eigenem Thread (nur Windows und Linux): dieselben
Befehle, Echtzeit und lange Läufe (`RunFor`, `RunIdle`) laufen im Hintergrund, `plat::wake()`
weckt die Oberfläche (`UIC_WAKE`), `pending()` dient Testskripten als Sync-Hook.
### Dienste und virtuelles Betriebssystem

Jedes Gerät mit IP hat ein kleines Betriebssystem: UDP-Sockets (ein an ein Interface
gebundener Socket geht vor einem allgemeinen), Dienste, die nur auf Ereignisse reagieren
(Datagramm, Timer, Ping-Antwort, ARP-Konflikt, fertige Namensauflösung, Link), ein Log mit
Syslog-Zielen, eine Hosts-Tabelle, einen Resolver und eine eigene Uhr, die anfangs bis zu 2 s
falsch geht und um bis zu 30 ppm driftet. UDP an einen Port ohne Socket bekommt weiter „Port
unreachable“. Konfiguriert wird über `Action`s (`DhcpClient`, `DhcpPool`, `DhcpHelper`,
`DnsZone`, `DnsRecord`, `Resolver`, `LogHost`, `NtpServer`, `NtpMaster` …) oder über die
Dienste eines Geräts in der Projektdatei (`"services": [{"kind": "dns-server", "config":
{"zones": [{"origin": "lab", "records": ["www CNAME web", "web A 10.0.2.20"]}]}}]`).

- **DHCP-Client:** unter Linux wie `dhclient`: DISCOVER sofort, Wiederholung nach 3 s mit
  zufällig wachsendem Abstand (höchstens rund 15 s), nach 60 s ohne Angebot 300 s Pause; das
  erste Angebot wird genommen; nach dem ACK zwei ARP-Proben im Sekundenabstand, eine Antwort
  führt zu DHCPDECLINE und 10 s später zu einem neuen Versuch; Verlängerung zufällig zwischen
  ¾ und 1 × T1 (unicast), nach T2 Rebind (broadcast), bei Ablauf ist die Adresse weg; nach
  einem Neustart fragt er per INIT-REBOOT nach der alten Adresse. IOS (`ip address dhcp`)
  setzt das Broadcast-Flag, nennt sich `cisco-<MAC>-<Interface>`, prüft nicht per ARP und
  trägt die Default-Route mit Distanz 254 ein.
- **DHCP-Server:** IOS vergibt die kleinste freie Adresse, pingt sie vorher zweimal mit je
  500 ms und sendet T1/T2 mit; ISC dhcpd vergibt die höchste Adresse des Bereichs, pingt einmal
  1 s lang und antwortet mit NAK nur, wenn er `authoritative` ist. Antwortet jemand auf den
  Ping, wird die Adresse als Konflikt gesperrt. Ausgeschlossene Adressen, Leases (IOS 1 Tag,
  dhcpd 600 s), Bindings, Konflikte, DECLINE, RELEASE, INFORM. Was ohne Relay ankommt, aber
  eine Adresse des Clients trägt (Erneuerung, Rückgabe, INFORM, auch über Router hinweg),
  ordnet der Server über diese Adresse (ciaddr) seinem Pool zu.
- **DHCP-Relay** (`ip helper-address`): setzt giaddr auf die Adresse des Eingangs-Interfaces,
  zählt die Hops und schickt per Unicast zum Server; die Antwort geht als Broadcast oder an die
  MAC des Clients zurück. IOS hält Server und Relay in einem Dienst und bedient ein Netz mit
  eigenem Pool selbst.
- **DNS:** autoritativer Server mit Zonen (SOA, NS, A, AAAA, CNAME, PTR, MX, TXT), CNAME-Ketten,
  Delegation mit Glue, NXDOMAIN und NODATA mit SOA, REFUSED außerhalb der Zonen oder
  Weiterleitung über den eigenen Resolver. Der Resolver jedes Geräts arbeitet wie glibc mit
  Suchdomänen (`ndots`), Timeout 5 s (IOS 3 s) und zwei Durchgängen über alle Server und hält
  Antworten für ihre TTL im Cache, negative für das SOA-Minimum. `ping www` geht damit per Name.
- **Syslog:** IOS schickt `<189>12: *Oct 19 12:00:05.123: %LINK-3-UPDOWN: …`, Linux das
  BSD-Format mit Hostname und Programm; ein Syslog-Server sammelt die Meldungen.
- **NTP:** Client und Server mit der virtuellen Uhr: die erste Antwort stellt die Uhr, später
  wird nachgeführt und die Drift korrigiert; ein Gerät mit `ntp master` liefert seine eigene
  Zeit. Eine asymmetrische Strecke verfälscht die Messung wie im echten Netz.

Abfragen für die Befehlszeilen: `dhcp_leases`, `dhcp_bindings`, `dhcp_pools`, `dhcp_helpers`,
`dns_zones`, `lookup` / `lookup_state`, `resolver_cache`, `resolv_conf`, `syslog_messages`,
`log_lines`, `ntp_peers`, `clock_state`, `sockets`, `services`, `metric`, für TCP `tcp_conns`,
`tcp_conn`, `tcp_stats` und `tcp_cfg`.

Das Szenario im Selbsttest: PC1 und PC2 holen sich über R1 (Relay) Adressen vom Server SRV1 im
Netz dahinter, lösen `www` über die Suchdomäne `lab` und den CNAME auf `web.lab` auf und pingen
per Name, alles mit exakten virtuellen Zeiten. Den ersten weitergeleiteten DISCOVER verwirft R1,
weil er erst nach der MAC von SRV1 fragt (IOS hält nichts zurück); PC1 bekommt seine Adresse
deshalb erst nach der Wiederholung.
### TCP

TCP folgt RFC 9293 und läuft als Sockets im virtuellen Betriebssystem jedes Geräts
([`tcp.rs`](crates/jfn_sim/tcp.rs)). Die Zahlen kommen vom System des Geräts: Hosts verhalten
sich wie Linux, Router und Switches IOS-artig.

- **Zustände:** alle elf, mit Simultaneous Open (die SYNs kreuzen sich, beide antworten mit
  SYN-ACK) und Simultaneous Close (CLOSING). Ein SYN an einen Port ohne Listener bekommt RST,
  ACK. TIME-WAIT dauert 60 s; ein wiederholter FIN startet sie neu, ein neuer SYN mit höherer
  Folgenummer öffnet die Verbindung sofort wieder. ISNs und flüchtige Ports kommen aus den
  Zufallszahlen des Geräts (Linux: 32768 bis 60999, gerade Ports zuerst).
- **Segmente und Fenster:** die MSS-Option aus der MTU der Route (IOS-artig 536 zu Zielen
  außerhalb der eigenen Netze); „Fragmentation needed“ senkt die MSS. Linux setzt DF, bietet
  ohne Window Scaling 64240 Byte an (65535, abgerundet auf die MSS) und rundet Fenster auf
  ganze Segmente; IOS-artig sind es 4128 Byte mit der SWS-Regel des Empfängers aus dem RFC.
- **Neuübertragung:** RTO nach RFC 6298 mit den Grenzen des Systems (Linux 200 ms bis 120 s,
  IOS-artig ab 300 ms), Karns Regel, doppelter Abstand nach jedem Timeout. Linux gibt Daten
  nach 15 Wiederholungen auf (924,6 s), einen SYN nach sechs (127 s); IOS-artig endet ein
  Verbindungsversuch nach 30 s. Segmente außer der Reihe werden gehalten und sofort mit einem
  doppelten ACK beantwortet.
- **Staukontrolle:** Reno nach RFC 5681, in Bytes gerechnet: Slow Start (Linux mit 10
  Segmenten Anfangsfenster), Congestion Avoidance, Fast Retransmit nach drei doppelten ACKs
  und Fast Recovery, Limited Transmit, Neustart nach einer Pause. Nach einem verlorenen SYN
  beginnt die Verbindung mit einem Segment.
- **ACKs:** verzögert (Linux 40 ms, mit Quick-ACKs am Anfang und im Ping-Pong-Modus auf der
  Antwort; IOS-artig 200 ms), sofort bei mehr als einem vollen Segment, bei Lücken und
  Duplikaten. Nagle (Linux nach Minshall), Fenster-Updates nach dem Lesen, Zero-Window-Probes
  über den Persist-Timer (Linux ohne Daten knapp unter SND.UNA, IOS-artig mit einem Byte),
  Challenge-ACKs nach RFC 5961, Keepalive nur auf Wunsch (Linux nach 7200 s, dann alle 75 s,
  neunmal, dann RST).
- **Sockets für Dienste:** `tcp_listen` (mit Backlog; wie bei Linux warten bis zu Backlog + 1
  Verbindungen, weitere SYNs werden verworfen), `tcp_accept`, `tcp_connect`, `tcp_send`,
  `tcp_recv`, `tcp_shutdown`, `tcp_close`, `tcp_abort`, `tcp_setopts` (TCP_NODELAY,
  SO_KEEPALIVE, Puffergrößen) mit Ereignissen wie bei UDP: `TcpAccept`, `TcpConnected`,
  `TcpData`, `TcpWritable`, `TcpEof`, `TcpClosed`; nach `TcpClosed` bleibt der Socket lesbar,
  bis der Dienst ihn schließt. Schließt ein Dienst mit ungelesenen Bytes oder kommen nach dem
  Schließen noch Daten, setzt das Gerät die Verbindung zurück (wie Linux, RFC 2525); ein
  beendeter Dienst schließt seine Sockets wie ein Prozess, ein ausgeschaltetes Gerät vergisst
  alles.
- **Testdienste** ([`tcpsvc.rs`](crates/jfn_sim/tcpsvc.rs)): `tcp-echo` (Port 7),
  `tcp-discard` (Port 9, auf Wunsch liest er nicht oder erst später) und `tcp-client` (schickt
  N Bytes, in Stücken und Abständen, liest das Echo und endet mit FIN, RST oder gar nicht),
  über `Action::Service` oder die Dienste in der Projektdatei, z. B. `{"kind": "tcp-client",
  "config": {"server": "10.0.2.10", "port": 9, "bytes": 1000000}}`.
- **Abfragen:** `tcp_conns` / `tcp_conn` (Zustand, Adressen, Warteschlangen, Fenster, cwnd,
  ssthresh, SRTT, RTO, der nächste Timer und Zähler wie bei `ss -tin`; Zustandsnamen auch für
  „show tcp brief“), `tcp_stats` (Zähler nach RFC 4022). Jeder Zustandswechsel und jede
  Entscheidung (verzögertes ACK, doppelte Bestätigung, Fast Retransmit, Zeitüberschreitung,
  Fenstersonde, Nagle, RST) steht mit deutschem Text in der Ereignisliste und im Paket-Tab.

Nicht in v1: SACK, Window Scaling, Timestamps, Urgent-Daten (URG zählt als normale Daten),
TCP Fast Open, ECN und SYN-Cookies. Linux rechnet eigentlich mit CUBIC; hier nutzen alle
Systeme Reno. `ss`, `nc` und `curl` zeigen und nutzen die Sockets in der Linux-Shell (siehe
[Befehlszeilen](#befehlszeilen)).

### HTTP

HTTP/1.1 läuft als Dienst auf den TCP-Sockets ([`http.rs`](crates/jfn_sim/http.rs),
[`httpsvc.rs`](crates/jfn_sim/httpsvc.rs)). Die Bytes baut und zerlegt der Codec
[`netpkt::http`](../../lib/netpkt/http.rs) ohne I/O: `Parser::push` nimmt Bytes in beliebigen
Stücken und meldet Kopf, Daten und Ende oder dass er mehr Bytes braucht; Content-Length,
chunked, Keep-Alive, Pipelining und die Grenzen (Kopf 16 KiB, 100 Kopfzeilen) folgen den Regeln
von [`lib/netio/http.rs`](../../lib/netio/http.rs). Der Dissektor zeigt Anfragen und Antworten
(auch mehrere in einem Segment) mit ihren Kopfzeilen und Chunks; der Filter heißt `http`.

- **Webserver** (`http-server`): Routen nach Pfad (genau, `/api/*` als Präfix, `*`) und
  Methode mit Status, Inhalt (`{host}`, `{path}`, `{client}` …), Typ, Größe, eigenen
  Kopfzeilen und Chunks; Rechenzeit mit Streuung, Worker mit Warteschlange (voll: sofort 503
  mit `Retry-After`), Fehlerrate pro Server oder Route; Keep-Alive mit Leerlauf-Timeout (75 s)
  und höchstens 1000 Anfragen pro Verbindung, Timeout für den Kopf, 100-continue, 400, 404,
  405 mit `Allow`, 413, 414, 431 und 505.
- **Client** (`http-client`): N Anfragen an eine URL in Abständen. Der Agent dahinter
  (auch für `curl` und den Lastgenerator) löst Namen auf, hält Verbindungen pro Host offen,
  pipelinet auf Wunsch, kennt Timeouts für Verbindung und Anfrage und wiederholt eine Anfrage
  einmal, wenn eine wiederverwendete Verbindung schon zu war.
- **Lastgenerator** (`http-load`): geschlossene Schleife (Nutzer mit fester oder exponentieller
  Denkzeit) oder offene (Zielrate gleichmäßig oder als Poisson-Prozess, Rampe von `start_rps`
  auf `rps`; die Latenz zählt ab dem geplanten Zeitpunkt, also ohne Coordinated Omission), ein
  Mix aus Methoden und Pfaden mit Gewichten, mehrere Ziele reihum oder zufällig, Dauer oder
  Anzahl. Latenzen und Fehler gehen in Fenster aus `base::metrics`: p50/p95/p99, Mittel und
  Maximum über fünf Minuten und je Sekunde (`Engine::http_load_view` und `http_server_view`
  für die Labor-Oberfläche). `Action::HttpLoadSet` (Rate, Nutzer) und `Action::HttpServerSet`
  (Fehlerrate, Rechenzeit, Worker) ändern laufende Dienste, ohne ihre Zahlen zu verlieren.
- **Metriken** für `expect … svc NAME metric WERT`: Server `requests`, `responses`,
  `status_2xx` … `status_5xx`, `errors`, `rejected`, `connections`, `active`, `inflight`,
  `queued`, `p50_us`, `p95_us`, `p99_us`, `busy_ms` …; Lastgenerator `sent`, `responses`, `ok`,
  `http_errors`, `failed`, `timeouts`, `inflight`, `connections`, `rate_mrps`, `rps`, `err_pm`,
  `p50_us`, `p95_us`, `p99_us`, `mean_us`, `max_us`.

Gerechnet wird nur mit ganzen Zahlen (Poisson-Abstände über einen Festkomma-Logarithmus, die
Rampe über das Integral der Rate mit ganzzahliger Wurzel), deshalb sind die Läufe auf jedem
Ziel gleich. Ein Lastgenerator in der Projektdatei:

```json
{"kind": "http-load", "name": "load", "config": {"targets": ["http://10.0.2.11/", "http://10.0.2.12/"],
 "mode": "open", "rps": 200, "arrival": "poisson", "duration_ms": 10000,
 "mix": [{"path": "/", "weight": 4}, {"path": "/api", "weight": 1}]}}
```

### Labor für verteilte Systeme

Die Dienste des Labors laufen wie die anderen auf den TCP-Sockets der Geräte und sprechen
HTTP, damit Lastgenerator, `curl` und Paket-Tab sie verstehen. Mit ihnen entstehen horizontal
skalierte Systeme: ein Lastgenerator vor einem Loadbalancer, mehrere Webserver, ein
geshardeter KV-Speicher, Caches, Queues und Worker. Konfiguriert werden sie wie die anderen
Dienste in der Projektdatei oder mit `Action::Service`; laufende Dienste ändern
`Action::LbBackend`, `LbAlgorithm`, `KvSet`, `CacheClear` und `WorkerSet`
([`lab.rs`](crates/jfn_sim/lab.rs)).

- **Loadbalancer** (`lb`, [`lb.rs`](crates/jfn_sim/lb.rs)): HTTP (L7: jede Anfrage über
  gehaltene Verbindungen an ein Backend, mit `X-Forwarded-For` und in der Antwort
  `X-Served-By`) oder TCP (L4: jede Verbindung Byte für Byte an ein Backend); `round-robin`,
  `least-conn`, `weighted` (gewichtet und gleichmäßig verteilt) und `hash` (Consistent Hashing
  mit vnodes über Pfad, Client-Adresse oder eine Kopfzeile). Health Checks (`tcp` oder
  `http`) mit Intervall, Timeout und den Schwellen `rise` und `fall`; eine Anfrage, die ihr
  Backend nicht erreicht, geht an das nächste (GET und HEAD auch nach einem Timeout).
  Entfernen leert ein Backend erst (Drain, mit Frist). Zahlen gesamt und je Backend
  (`b1_requests`, `b1_active`, `b1_up`, `b1_errors` …).
- **KV-Speicher** (`kv`, [`kv.rs`](crates/jfn_sim/kv.rs)): Knoten auf einem Hash-Ring mit
  vnodes ([`base::hashring`](../../lib/base/hashring.rs)), Replikationsfaktor, ein Leader je
  Schlüssel (der erste erreichbare Knoten seiner Präferenzliste; er vergibt
  Lamport-Versionen, die neuere gewinnt), Schreib- und Lesequoren `W` und `R` mit `strong`
  (je eine Mehrheit, W + R > RF) und `eventual` (1 und 1), pro Anfrage überschreibbar
  (`?consistency=`, `?w=`, `?r=`), Read Repair, Heartbeats über UDP. Wer eine Anfrage nicht
  beantwortet oder zu lange schweigt, gilt als ausgefallen, bis er wieder zu hören ist;
  antwortet der Leader nicht, führt ein Replikat den Schreibvorgang selbst. Die Daten liegen
  im Dienst und überleben Strom aus; jedes Gerät hat eine Disk
  ([`disk.rs`](crates/jfn_sim/disk.rs)) mit Latenz, Durchsatz, IOPS und Kapazität (voll: 507).
  HTTP: `GET`, `PUT` und `DELETE` auf `/kv/SCHLÜSSEL`, zwischen den Knoten `/_kv/SCHLÜSSEL`.
- **Webserver mit Backend:** eine Route mit `"backend": "kv"` liest und schreibt den Schlüssel,
  der in ihrem Pfad folgt; der Client im Webserver kennt den Ring, fragt das erste nicht
  ausgefallene Replikat und wechselt nach einem Timeout zum nächsten (`failovers`). Eine URL
  als Backend reicht die Anfrage an einen anderen Server weiter. Der Worker bleibt so lange
  belegt.
- **Cache** (`cache`, [`cache.rs`](crates/jfn_sim/cache.rs)): HTTP-Cache vor einem Backend, LRU
  nach Anzahl und Größe ([`base::lru`](../../lib/base/lru.rs)), Lebensdauer (oder `max-age` des
  Backends), `X-Cache: HIT` oder `MISS`, zusammengelegte Fehlschläge, Invalidierung durch
  Schreiben.
- **Message Queue** (`mq`) und **Worker** (`worker`, beide in [`mq.rs`](crates/jfn_sim/mq.rs)):
  Topics, at-least-once mit Bestätigung, Rückgabe, Sichtbarkeits-Timeout und erneuter
  Zustellung, Dead Letters nach `max_deliveries`, Long-Polling; eine dauerhafte Queue legt ihre
  Nachrichten auf die Disk. Worker arbeiten mit mehreren Plätzen, Bearbeitungszeit und
  Fehlerquote.
- **Lastgenerator:** Pfade und Inhalte dürfen `{seq}` (die Nummer der Anfrage ab 0),
  `{rand:N}` und `{hot:N}` (vier von fünf Anfragen auf dem ersten Fünftel) enthalten.

**Fehlerinjektion** ([`fault.rs`](crates/jfn_sim/fault.rs)): `Action::Fault { fault, dur }` mit
`Unplug` (Kabel ziehen), `Impair` (mehr Laufzeit und Jitter, Verlust, weniger Bandbreite),
`Power`, `Stop` (ein Dienst), `SlowDisk` und `Partition` (die Geräte zweier Seiten verwerfen
gegenseitig ihre IPv4-Pakete, eine Seite allein steht gegen alle anderen). Jeder Fehler bekommt
eine Nummer und eine Markierung mit Start, geplantem Ende und Text (`Engine::faults()` für
Zeitleisten und Diagramme); `Action::Heal { id }` und `HealAll` machen genau das rückgängig,
was er getan hat, auch wenn sich Fehler überlagern, und mit `dur` heilt er von selbst. Fehler
legen sich über die Werte von Kabeln und Disks, statt sie zu überschreiben.

**Abfragen für die Labor-Oberfläche:** `lb_view` (Backends mit Zustand, Prüfungen, Latenzen,
Ring-Anteil), `kv_view` (Mitglieder, wie ein Knoten sie sieht, Quoren, Zähler), `kv_ring`,
`kv_where` (die Replikate eines Schlüssels und was jedes hält), `cache_view`, `mq_view`,
`worker_view`, `faults` und `active_faults`, dazu `services` und `metric`.
Den Verlauf jeder Zahl geben die Zeitreihen (siehe unten).

```json
{"kind": "lb", "name": "lb", "config": {"backends": [{"name": "web1", "addr": "10.0.2.11"},
 {"name": "web2", "addr": "10.0.2.12"}], "health": {"type": "http", "path": "/health"}}}
{"kind": "http-server", "name": "web", "config": {"routes": [{"path": "/item/*", "method": "*",
 "backend": "kv"}], "kv": {"nodes": ["10.0.2.21", "10.0.2.22", "10.0.2.23"], "rf": 2}}}
{"kind": "kv", "name": "kv", "config": {"nodes": ["10.0.2.21", "10.0.2.22", "10.0.2.23"],
 "rf": 2, "consistency": "eventual"}}
```

### Metriken und Zeitreihen

Der Simulator nimmt jede Sekunde virtueller Zeit einen Punkt jeder Zahl jedes Dienstes und des
Verkehrs jedes Kabels ([`series.rs`](crates/jfn_sim/series.rs)) und hält 300 Punkte pro Reihe,
also fünf Minuten (`Engine::set_series_cfg`: Intervall in ganzen Sekunden, Anzahl der Punkte,
Kabel ja oder nein). Daraus zeichnet das Labor seine Diagramme und KPI-Kacheln, und Szenarien
prüfen Zeiträume (siehe [Szenarien](#szenarien-scn)).

- **Was aus einer Zahl wird:** ein Zähler (`requests`, `errors`, `bytes_in`, `busy_ms` …) wird
  zur Rate pro Sekunde, ein Stand (`active`, `queued`, `depth`, `keys`, `members_up`,
  `b1_up` …) bleibt sein Wert am Ende der Sekunde, und aus den Latenz-Fenstern der Dienste
  werden p50, p95 und p99 jeder Sekunde in µs (beim Lastgenerator auch Mittel und Maximum,
  beim Cache auch die der Treffer und Fehlschläge, beim Webserver die seines Backends). Sie
  stehen für die gleichnamigen Zahlen, die über fünf Minuten gehen. Was eine Zahl ist, sagt
  der Dienst (`Service::metric_kind`, sonst ihr Name), seine Fenster nennt `Service::windows`.
- **Kabel:** Rahmen und Bytes pro Sekunde in jede Richtung (`frames_ab`, `bytes_ab`,
  `frames_ba`, `bytes_ba`; a ist das Ende, das das Projekt zuerst nennt).
- **Genau und billig:** ein Punkt deckt die Sekunde davor ab und entsteht vor dem ersten
  Ereignis danach; er liest nur, plant kein Ereignis und schreibt nichts in die Aufzeichnung,
  deshalb bleiben Läufe, Golden-Berichte und Selbsttests, wie sie waren. Pro Reihe und Punkt
  bleibt eine Zahl, nichts pro Anfrage. Die erste Abtastung nach dem Start nimmt nur die
  Zählerstände, Punkte gibt es ab der nächsten; ein später hinzugekommener oder ersetzter
  Dienst zählt von 0, ein neues Kabel ab seiner zweiten Abtastung. Eine lange Pause ohne
  Ereignisse wird nicht Sekunde für Sekunde abgetastet, nur so weit, wie Punkte bleiben.
- **Für die Oberfläche:** `series_services` (welche Dienste welche Reihen haben, mit Art und
  Einheit), `series` und `series_one` (die Punkte), `cable_series`, `kpi` und `kpis` (über die
  letzten Sekunden: Anfragen pro Sekunde, p50/p95/p99, Fehler pro Tausend, Warteschlange, in
  Arbeit, Treffer eines Caches, Last, Schlüssel und Bytes eines KV-Knotens), `shard_load` (die
  Last jedes KV-Knotens und ihr Anteil), `chart_marks` (die Fehler eines Zeitraums als
  Markierungen), `series_value` (ein Wert über einen Zeitraum), `series_gen` (ändert sich nur
  mit neuen Punkten: dann neu zeichnen) und `series_csv`.

### Befehlszeilen

Jedes Gerät mit Befehlszeile hat eine Konsole (`Engine::cli_line`; die Oberfläche schickt
`Cmd::Cli` und bekommt `Report::CliOut`). Router, Switches, Firewall und Access Point haben eine
IOS-artige Befehlszeile:

- Modi `R1>`, `R1#`, `R1(config)#`, `(config-if)#`, `(config-subif)#`, `(config-vlan)#` und
  `(config-line)#` mit `enable`, `configure terminal`, `end`, `exit` und `do`; Abkürzungen wie
  `sh ip int br`, `?` und Tab, Fehler mit `^`, `% Incomplete command.` und
  `% Ambiguous command:`; wie beim Vorbild fällt eine Zeile aus einem Untermodus auf die globale
  Konfiguration zurück, und unbekannte Wörter im EXEC gelten als Hostname.
- Konfiguration: `hostname`, `interface` (Ports, `Gi0/0/0.10`, `Vlan10`, `Loopback0`),
  `ip address … [secondary]`, `[no] shutdown`, `description`, `encapsulation dot1Q`,
  `switchport mode|access vlan|trunk native vlan|trunk allowed vlan`, `[no] switchport`,
  `vlan` mit `name`, `ip route`, `[no] ip routing`, `[no] ip proxy-arp`, `ip mtu`, `arp`,
  `[no] ip domain-lookup`, `line con 0` / `line vty 0 4`.
- `show running-config|startup-config|ip interface brief|ip route|arp|mac address-table|vlan
  [brief]|interfaces [NAME|trunk]|debugging|version`, `copy running-config startup-config`,
  `write memory`, `clear arp-cache`.
- `ping` (5 × 100 Byte, Timeout 2 s, `!` `.` `U` `M` `&`, Erfolgsquote mit min/avg/max) und
  `traceroute` im IOS-artigen Format, Abbruch mit Strg+Umschalt+6; `debug ip icmp` und
  `debug arp` aus den Entscheidungen der Geräte; Meldungen wie `%LINK-3-UPDOWN`,
  `%LINEPROTO-5-UPDOWN` und `%SYS-5-CONFIG_I` mit Zeitstempel.
- DHCP und NTP ([`netsvc.rs`](crates/jfn_sim/cli/netsvc.rs)): `ip dhcp pool NAME` mit eigenem
  Modus `(dhcp-config)#` (`network` mit Maske oder `/nn`, `default-router`, `dns-server`,
  `domain-name`, `lease TAGE [STUNDEN [MINUTEN]]|infinite`; ein Pool ohne `network` wartet
  auf der Befehlszeile), `ip dhcp excluded-address`, `ip helper-address` an Interfaces,
  `show ip dhcp binding|pool|conflict`, `clear ip dhcp binding|conflict *|ADRESSE`,
  `ntp server`, `ntp master [STRATUM]`; alles steht in der Running-Config.

Hosts und Server haben eine Linux-Shell (`root@PC1:~#`): `ip addr|link|route|neigh` mit den
Ausgaben und Fehlermeldungen von iproute2, `ping` wie iputils (bis Strg+C oder `-c`; `-i`,
`-s`, `-t`, `-W`, `-M do`), `traceroute`, `hostname` und `sysctl net.ipv4.ip_forward`.
Dazu Programme auf den Sockets, die die Konsole belegen, bis sie enden (Strg+C bricht ab):
`curl` (`-v`, `-i`, `-I`, `-s`/`-S`, `-o`, `-w`, `-L`, `-f`, `-d`, `-X`, `-H`, `-m`,
`--connect-timeout`, `-0`; mehrere URLs über eine Verbindung, Fortschrittsanzeige und
Fehlermeldungen mit den Exit-Codes wie curl 8.5), `nc` (Client, `-l` mit `-k`, `-u`, `-z`
mit Port-Bereichen, `-v`, `-C`, `-q`, `-w`, `-N`), `ss` (TCP und UDP mit Zuständen, `-t`,
`-u`, `-l`, `-a`, `-n`, `-p`, `-o`, `-i`, `-s`, `-H`, Filter wie `state established` und
`( sport = :80 or dst 10.0.0.1 )`, Spalten wie iproute2 ohne Terminal), `top` (die Dienste
des Geräts mit CPU-Anteil, `-b`, `-n`, `-d`) und `printf` mit `|` in ein Programm.
Router und Switches zeigen `show tcp brief [all]`, `show tcp statistics` und
`show control-plane host open-ports`. Die Dienste des Labors zeigen `lbctl` (Backends mit
Zustand und Zahlen), `kvctl` (Mitglieder, `ring`, `where SCHLÜSSEL` mit den Replikaten und
ihren Versionen, `get`, `keys`), `cachectl`, `mqctl` und `workerctl`, mit `-s NAME` für einen
bestimmten Dienst; ihre Texte sind eigene Worte. Die Skripte eines Hosts zeigt und steuert
`script` (siehe [Skripte in Hosts](#skripte-in-hosts)).

Namen löst die Shell über den Resolver des Geräts auf (`ping server.lab`, `traceroute
server.lab`, auch kurz mit der Suchdomäne): die erste Zeile nennt wie getaddrinfo das Ende der
CNAME-Kette, eine gescheiterte Suche meldet sich wie glibc („Name or service not known“,
„Temporary failure in name resolution“); Traceroute fragt nicht rückwärts nach Namen und
zeigt einen Hop deshalb als „ADRESSE (ADRESSE)“. `dhclient [-v] [-r] [IF]` holt oder erneuert
die Adresse (eine Erneuerung geht direkt an den Server, auch über Router) und hält die Konsole,
bis der Client gebunden ist oder aufgibt; mit `-v` zeigt er die Zeilen des Clients, `-r` gibt
die Adresse zurück. `cat /etc/resolv.conf` schreibt die Einstellungen des Resolvers so, wie
Debians dhclient-script die Datei schreibt (`domain`, `search`, `nameserver`), dazu
`/etc/hosts` und `/etc/hostname`.

Hilfetexte (`?`, `ip … help`), Modus-Titel, Statuszeilen wie die ersten Zeilen von `ping` und
`traceroute`, die Texte hinter Meldungen wie `%LINK-3-UPDOWN`, die Debug-Zeilen und
`show version` sind eigene Worte (siehe die Regel unter [Fahrplan](#fahrplan)). Dem Vorbild folgen
die Befehlswörter und ihre Syntax, Interface-Namen, Konfigurationszeilen, kurze
`%`-Fehlermeldungen, die Tabellen der `show`-Befehle und die Ergebniszeilen von `ping`
(`!!!!!`, `Success rate is …`), iproute2 und iputils.

Die Konfiguration steckt in den Teilen des Geräts (Ports, Bridge, IPv4). `show running-config`
schreibt sie als IOS-artige Konfiguration, `Engine::running_config` bei Linux-Geräten als
Shell-Befehle. Beim Laden läuft die startup-config jedes Geräts durch dieselbe Befehlszeile;
es gibt also nur eine Quelle der Wahrheit (ein Port, den die Startkonfiguration nennt, ist an,
solange dort kein `shutdown` steht; nur ungenannte Router-Ports bleiben wie ab Werk aus). Was
dabei scheitert, zeigen die Konsole und
`jfn-sim check`. `copy running-config startup-config` meldet die neue Fassung als
`Report::Startup` an die Oberfläche, die sie als Rückgängig-Schritt ins Projekt übernimmt;
Meldungen der Konsole, Debug-Zeilen und die Logs der Dienste kommen zusätzlich als
`Report::Log` (Gerät, Zeit, Schwere, Quelle, Text) für das Protokoll.

### Szenarien `.scn`

Für Tests und Läufe ohne Fenster: Befehle zu virtuellen Zeitpunkten und Erwartungen.

```
seed 7
at 1s PC1: ping -c 4 10.0.0.2
run until idle
R1: enable
R1: show ip route
PC1: ^C
impair R1 Gi0/1 loss 5% delay 20ms
at 6s fault power KV2 for 5s
fault partition KV1 KV2 | KV3
heal all
expect PC1 output contains "4 received"
expect R1 route 10.0.2.0/24 via 10.0.1.2
expect hops icmp >= 8
expect LB1 svc lb metric requests >= 99.5 over 5s
expect LB1 svc lb metric max p99_us < 5000 between 14s and 19s
series build/run.csv
pcap build/run.pcapng
```

`jfn-sim run PROJEKT --scenario S` spielt ein Szenario ab und schreibt einen Bericht, der nur
virtuelle Zeiten enthält und deshalb auf jedem Rechner gleich ist; der Exit-Code ist 0 oder 1.
Jede Zeile trägt die Zeit, das Gerät (mit seiner Konsolenausgabe und den getippten Zeilen samt
Prompt) oder `--` für einen Schritt und `ok` / `FEHLER` für eine Erwartung. `expect … output
contains` prüft, was das Gerät seit seinem letzten Befehl ausgegeben hat. `--report F` schreibt
den Bericht in eine Datei, `--golden F` vergleicht ihn mit einer gespeicherten Fassung,
`--pcap F` schreibt die Aufzeichnung als pcapng. Die GUI-Exe nimmt dieselben Argumente
(`JFNetworker.exe PROJEKT --scenario S`). Weil die App nur Dateien öffnen darf, die auf der
Befehlszeile stehen (`fs.user`), gehören Projekt, Vergleichsdatei und pcap-Datei dorthin.

`fault` spielt einen Fehler des Labors ein: `link GERÄT PORT`, `impair GERÄT PORT delay …
jitter … loss … rate …`, `power GERÄT`, `stop GERÄT DIENST`, `disk GERÄT delay … factor …`,
`partition A B | C` (ohne `|` gegen alle anderen), jeweils mit `for DAUER`, wenn er von selbst
heilen soll. `heal NUMMER`, `heal all` oder `heal` mit Art und Ziel (`heal power KV2`,
`heal partition`) heilt. Der Bericht nennt Nummer und Text jedes Fehlers; heilt einer von
selbst, steht „Fehler N behoben“ mit der Zeit dabei.

`expect … svc NAME metric WERT VERGLEICH ZAHL over 5s` (oder `… between 6s and 12s`) prüft
statt des Werts von jetzt seine Zeitreihe
(siehe [Metriken und Zeitreihen](#metriken-und-zeitreihen)): die Punkte der letzten 5 s oder
die nach 6 s bis 12 s genommenen. Ohne weiteres Wort gilt, was zur Reihe passt: bei einem
Zähler seine Rate pro Sekunde über den Zeitraum, bei einem Stand sein Mittel, bei einer Latenz
das Perzentil aller Anfragen des Zeitraums. `min`, `max` und `mean` vor dem Namen gehen über
die Punkte:
`expect LB1 svc lb metric max p99_us < 5000 between 14s and 19s` heißt, keine Sekunde hatte
ein p99 über 5 ms. Die Zahl darf Nachkommastellen haben; der Bericht schreibt den gemessenen
Wert mit bis zu drei dazu, und ein Zeitraum, der erst nach der Erwartung endet, ist ein Fehler.
`series DATEI` schreibt am Ende alle Zeitreihen als CSV (eine Spalte pro Reihe mit Einheit,
eine Zeile pro Punkt) und nennt im Bericht Reihen, Werte und die CRC-32 der Datei, so prüft der
Golden-Bericht auch sie; `--series F` auf der Befehlszeile schreibt nach F, auch ohne Schritt.

Die Golden-Szenarien in `tests/scen/` laufen über `tests/scen_*.uis` in `build_rs.py test`:
zwei PCs am Switch, ein Router zwischen zwei Netzen (mit `.!!!!`), Router-on-a-stick, ein
L3-Switch mit SVIs, eine serielle Strecke und TCP über eine langsame Leitung mit 1 % Verlust
(der erste SYN geht verloren, während der Router nach ARP fragt; 1 MB an den Discard-Dienst und
20 kB Echo mit Fast Retransmits) und HTTP-Last auf zwei Webserver hinter einem Router mit 2 %
Verlust auf einer Leitung (200 Anfragen pro Sekunde, die Neuübertragungen nur im 99.
Perzentil; `top`, `ss -s`, `curl`) und das Labor: ein Lastgenerator vor einem Loadbalancer,
drei Webservern und drei KV-Shards mit Replikationsfaktor 2, von denen einer für 6 s ausfällt
(keine Anfrage geht verloren, die Clients der Webserver weichen auf das zweite Replikat aus,
Read Repair holt den Shard nach; `lbctl`, `kvctl`). Ihre Berichte (`*.report`) sind unter Windows und Linux
byte-gleich. `expect … svc NAME metric WERT` prüft die Zahlen eines Dienstes und schreibt den
gemessenen Wert in Klammern dazu.
Die Zeitreihen des Labors prüft `lab_metrics.scn` auf demselben Netz (`lab_shards.jfnet`, KV2
fällt von 6 s bis 12 s aus): 22 Erwartungen über Zeiträume vor, während und nach dem Ausfall
(rund 100 Anfragen pro Sekunde, KV2 bekommt nichts, KV1 übernimmt, die Mitglieder fallen auf
zwei, das p99 der sechs Sekunden steigt auf eine Viertelsekunde und das einzelner Sekunden auf
fast eine, danach ist alles wie vorher, keine Anfrage geht verloren) und die CSV aller 251
Reihen mit ihrer CRC-32 im Bericht.

Dazu kommt der Campus der Abnahme (`campus.scn` auf `fixtures/campus.jfnet`, siehe
[Bauen und testen](#bauen-und-testen)): die Leases der vier PCs über das Relay von MLS1,
`dhclient -v` (PC1 erneuert direkt bei R1, über MLS1 hinweg), `ip addr`, `ip route` und
`/etc/resolv.conf`, `ping -c 3 server.lab` aus beiden VLANs ohne Verlust, `traceroute
server.lab` mit drei Hops (VLAN-Interface von MLS1, R1, SRV1), ein unbekannter Name, ein Ping
zwischen den VLANs, `show ip dhcp binding` und `show ip route` auf R1, `show ip route`,
`show mac address-table` und `show vlan brief` auf MLS1 und die Pools in R1s Running-Config;
42 Erwartungen, dazu die Zähler von DHCP-Server und Relay.

Ein Skript als Schlüssel-Wert-Speicher (`script_kv.scn` auf `fixtures/script_kv.jfnet`) zeigt
die Rechenzeit von Skripten: SRV1 nimmt über HTTP Werte an und legt sie auf seine Disk, jede
Anfrage rechnet rund 1,1 ms (100 ns je Anweisung). 200 Anfragen pro Sekunde kosten gut ein
Fünftel der CPU und warten kaum; bei 1500 pro Sekunde füllt sich die Warteschlange auf 100,
darüber antwortet der Dienst sofort mit 503, und das 99. Perzentil steigt über 100 ms. Ein
zweites Skript auf MON fragt jede Sekunde mit `http.request` nach den Zahlen; im Bericht stehen
`script`, `script status`, `script output`, `top` und `curl`.

### Skripte in Hosts

Eigene Programme laufen als Dienste in den Hosts ([`scriptsvc.rs`](crates/jfn_sim/scriptsvc.rs)):
ein Skript in der Sprache von [`lib/script`](../../lib/script/lib.rs) (`.jfs`, siehe
[GUIDE §29](../../docs/GUIDE.md)), das nur auf die Ereignisse reagiert, um die es gebeten hat –
Datagramme, Verbindungen, Daten, HTTP-Anfragen, Antworten, Timer, Namen, Zugriffe auf die Disk.
Es steht wie die anderen Dienste in der Projektdatei (`"kind": "script"`) oder kommt mit
`Action::Service`; der Quelltext ist ein Text oder eine Liste von Zeilen, die übrige
Konfiguration sieht das Skript als `config` (`config.port`):

```json
{"kind": "script", "name": "echo", "config": {"source": [
  "net.udp(config.port, fn(s, m) { s.send(m.from, m.port, m.data) })"], "port": 7,
  "ns_per_op": 5, "limits": {"fuel": 10000000, "heap_kb": 16384, "depth": 500, "stack": 100000}}}
```

- **Rechenzeit:** Ein Skript hat eine CPU. Jede Anweisung kostet virtuelle Zeit (`ns_per_op`,
  Standard 5 ns, auch mit Nachkommastellen); solange ein Rückruf rechnet, warten alle anderen
  Ereignisse, wie bei einem Server mit einem Thread. Was ein Rückruf nach außen tut (senden,
  verbinden, schließen, antworten, fragen, auf die Disk zugreifen, ins Log schreiben),
  geschieht in dem Augenblick seiner Rechenzeit, an dem er es tut: eine Schleife vor dem
  Senden verzögert die Antwort um ihre Anweisungen. Ein Rückruf läuft in Scheiben (`slice`,
  Standard 100 000 Anweisungen), dazwischen geht die Simulation weiter; eine Endlosschleife ist
  also 100 % CPU (in `top` zu sehen) statt ein Hänger, bis `limits.fuel` den Rückruf beendet (0:
  nie). Solange die CPU rechnet, bleiben TCP-Daten im Socket (das Fenster füllt sich) und
  Verbindungen im Backlog; ist die Warteschlange voll (`queue`, Standard 1000 Ereignisse),
  verwirft der Dienst Datagramme und beantwortet Anfragen sofort mit 503. Rückrufe, um die das
  Skript gebeten hat (Timer, Disk, Antworten), kommen trotzdem.
- **Grenzen und Fehler:** Speicher (`heap_kb`), Aufruftiefe (`depth`), Stapel (`stack`) und
  Anweisungen je Rückruf (`fuel`) sind hart; eine Ausnahme, die niemand fängt, oder eine Grenze
  nennt Zeile und Spalte in der Statuszeile, in `script errors`, im Log des Geräts und in der
  Ereignisliste. Ein Fehler im Hauptprogramm beendet den Dienst, mit `"stop_on_error": true`
  jeder; sonst läuft das Skript weiter, und ein HTTP-Handler, der scheitert, antwortet 500.
  Stoppen und Strom aus beenden das Programm samt Speicher; was es auf die Disk legte, bleibt.
- **Determinismus:** keine Uhr außer der virtuellen, Zufall aus einem eigenen Strom je Dienst
  (vom Gerät und Seed abgeleitet), Gleitkomma in Software: gleiche Projekte ergeben auf jedem
  Ziel byte-gleiche Läufe, auch mitten in einem Rückruf geklont.

| Name | Was es tut |
|---|---|
| `log(x, ..)`, `log.debug`, `log.info`, `log.warn`, `log.error` | eine Zeile im Log des Geräts mit dem Namen des Dienstes; `print` schreibt in die Ausgabe des Skripts (`script output`) |
| `now()` | die virtuelle Zeit in ms, als Kommazahl auf die Nanosekunde, die Rechenzeit eingerechnet |
| `rand()`, `rand(n)`, `rand(a, b)` | eine Zahl in [0, 1), eine ganze Zahl von 0 bis n − 1, eine von a bis b |
| `config` | die Konfiguration des Dienstes ohne `source` |
| `timer.after(ms, f)`, `timer.every(ms, f)`, `timer.cancel(id)` | Timer; `every` höchstens alle 10 µs |
| `net.listen(port, {accept, data, close, binary})` | TCP-Server; eine Verbindung ist `{kind, id, peer, peer_port, port}` mit `send(daten)`, `close()` (nach dem, was noch wartet) und `abort()` (RST) |
| `net.connect(adresse, port, {connect, data, close, binary})` | TCP-Client (Namen löst `dns.resolve` auf) |
| `net.udp(port, fn(sock, msg), {binary})` | UDP-Socket mit `send(adresse, port, daten)` und `close()`; `msg` ist `{data, from, port}`, Port 0 ein flüchtiger |
| `dns.resolve(name, fn(err, adressen))` | der Resolver des Geräts (Suchdomänen, Cache) |
| `http.serve(port, fn(req, res))` | HTTP-Server, einmal je Skript: `req` ist `{method, path, target, query, headers, body, peer, peer_port}`, `res.send(status, body, headers)` und `res.json(status, wert, headers)` gehen auch später aus einem anderen Rückruf; gibt der Handler einen Text, Bytes, eine Liste, eine Map oder einen Status zurück, ohne `res` zu nutzen, ist das die Antwort |
| `http.request(url, fn(err, resp))`, `http.request({url, method, headers, body, timeout_ms}, ..)` | HTTP-Client mit gehaltenen Verbindungen, Namen über den Resolver, ohne Angabe 30 s Timeout; `resp` ist `{status, headers, body}` |
| `disk.get(key, fn(err, wert))`, `disk.put(key, wert, fn(err))`, `disk.delete(key, fn(err, war_da))`, `disk.scan(präfix, fn(err, einträge))` | JSON-Werte auf der Disk des Geräts mit ihren Zugriffszeiten und ihrer Kapazität (`"disk": {...}` wie beim KV-Knoten) |
| `metrics.inc(name, n)`, `metrics.gauge(name, wert)`, `metrics.observe(name, wert)` | Zahlen des Dienstes für `expect … svc NAME metric X`; `observe` ergibt `NAME_count`, `NAME_p50`, `NAME_p95`, `NAME_p99` und `NAME_max` über fünf Minuten |
| `json`, `str`, `list`, `map`, `math`, `bytes` | die Standardbibliothek |

Daten kommen als Text (UTF-8; ein Zeichen, das zwei Segmente teilen, wartet auf seinen Rest),
mit `binary: true` als `bytes`; Fehler bekommen Rückrufe als deutschen Text. `close(conn, why)`
kommt genau einmal je Verbindung, wer sie auch beendet (`closed`, `reset`, `refused`,
`timeout`, `unreachable`, `aborted`); nach dem FIN der Gegenseite schließt die Verbindung, sobald
dieser Rückruf zurück ist. Die Zahlen eines Skripts sind seine eigenen Metriken, dann `calls`,
`errors`, `fuel`, `busy_us`, `queued`, `max_queued`, `dropped`, `rejected`, `timers`, `conns` und
`heap_kb`.

In der Linux-Shell ([`cli/scripts.rs`](crates/jfn_sim/cli/scripts.rs)) zeigt `script` die
Skripte des Geräts (Zustand, Rückrufe, Fehler, Rechenzeit, offene Ports), `script status NAME`
eines davon (Rechenzeit und Preis einer Anweisung, Warteschlange, Grenzen, Offenes, Metriken,
letzter Fehler), `script errors NAME` die letzten Fehler mit Zeit, Zeile, Spalte und Aufrufen,
`script output NAME` die Ausgabe von `print`; `script start|stop|restart NAME` steuert es, und
`top` zeigt seine Rechenzeit. Für die Oberfläche gibt es `Engine::scripts(dev)` und
`ScriptService::view`, `errors`, `output`, `source`; im Fenster der Skript-Editor (unten).
Geprüft wird das in der Selbsttest-Gruppe `script` ([`src/scripttest.rs`](src/scripttest.rs):
Antwortzeiten auf die Nanosekunde aus dem Fuel, Timer, UDP, TCP, HTTP, DNS, Disk, Metriken,
Endlosschleife, Grenzen, Fehler, `script`, `top`, Determinismus, ein neuer Quelltext, der Disk,
Zahlen und Fehler behält, Umbenennen im Lauf) und im Golden-Szenario oben.

### Skript-Editor

Die Skripte eines Hosts bearbeitet der Reiter **Skripte** seines Gerätefensters (Kontextmenü
eines Hosts › „Skripte …“; [`devwin/scripts.rs`](src/workspace/devwin/scripts.rs),
[`doc/scripts.rs`](src/doc/scripts.rs)):

- **Liste:** die Skripte des Geräts in der Reihenfolge des Projekts mit Zustand (läuft, rechnet,
  gestoppt, mit Fehler beendet, „übersetzt nicht“, wenn der Simulator die Definition aus der
  Datei nicht nehmen konnte), CPU-Anteil der letzten Sekunden und Fehlern; „geändert“ markiert
  nicht übernommene Änderungen. „Neu“ legt ein Skript aus einer Vorlage an (leer, UDP-Echo,
  HTTP-Server, eine Abfrage, die regelmäßig eine URL abfragt und die Antwortzeit misst, ein
  Schlüssel-Wert-Speicher auf der Disk; Kommentare und Texte in der Sprache des Fensters),
  „Umbenennen“ und „Löschen“ ändern das Projekt. Jeder dieser Schritte ist ein
  Rückgängig-Schritt, der das laufende Gerät behält: ein umbenanntes Skript läuft weiter (mit
  Disk, Zahlen und Zeitreihen), ein gelöschtes nimmt mit, was es auf der Disk abgelegt hatte
  (Rückgängig bringt das Skript zurück, seine Disk beginnt leer).
- **Editor:** der Code-Editor der Library mit Hervorhebung für `.jfs`. Beim Tippen prüft er
  höchstens alle 250 ms (Syntaxfehler, unbekannte Namen, `break` außerhalb einer Schleife …), und
  die Fehler des laufenden Programms stehen mit Zeile und Spalte als Marken im Text – nur die
  des aktuellen Laufs, ein Neustart räumt sie ab; ihre Zeilen folgen dem bearbeiteten Text, eine
  geänderte Zeile verliert ihre Marke. F8 und Umschalt+F8 springen von Meldung zu Meldung, die
  Statuszeile nennt die Stelle, die Zahl der Fehler und Warnungen und die Meldung an der
  Schreibmarke. Jedes Skript hat einen Entwurf mit eigenem Rückgängig (Strg+Z im Editor), der
  den Wechsel zu einem anderen Skript und das Schließen des Fensters übersteht, bis er
  übernommen oder verworfen wird oder ein anderes Projekt kommt; ein unveränderter Entwurf
  folgt dem Projekt.
- **Übernehmen** (Strg+S im Editor): ein Quelltext, der sich übersetzen lässt, geht als ein
  Rückgängig-Schritt ins Projekt (die `source` der Dienstdefinition, als Liste von Zeilen oder
  als Text, wie sie war) und in das laufende Skript: es startet mit dem neuen Programm, Disk,
  Zahlen, Fehler, Ausgabe und Zeitreihen bleiben, das Gerät startet nicht neu. Rückgängig und
  Wiederholen tauschen das Programm genauso. Einen Quelltext mit Fehlern lehnt es mit dem
  ersten ab. Strg+S speichert hier also nicht die Projektdatei.
- **Ausführen, Stoppen, Neu starten** wirken nur im Simulator, wie `script start|stop|restart`
  in der Shell; ob ein Skript mit dem Gerät startet, bleibt Sache des Projekts (Reiter Dienste).
- **Unten:** die Ausgabe des aktuellen Laufs (`print`, folgt live) und seine Fehler mit Zeit,
  Zeile und Spalte (ein Klick setzt die Schreibmarke dorthin); darüber die Kennzahlen: Zustand,
  CPU, Warteschlange, Rückrufe, Fehler, Speicher und was das Skript offen hat.

Der Simulator ([`scriptsvc.rs`](crates/jfn_sim/scriptsvc.rs)) bietet dafür
`Action::ScriptSource { dev, svc, config }`: das Skript bekommt eine neue Definition und bleibt
derselbe Dienst (Nummer, Disk, Zahlen, Fehler, Ausgabe, Zeitreihen); eines, das laufen soll
(läuft, nach einem Fehler beendet oder wartet auf Strom), startet sofort mit dem neuen Programm,
ein gestopptes bleibt gestoppt, eine Definition, die sich nicht übersetzen lässt, wird mit Zeile
und Spalte abgelehnt und das alte Programm läuft weiter. `Action::RenameService` benennt einen
Dienst um, ohne ihn anzuhalten (seine Zeitreihen gehen mit). `ScriptService::run_errors()` und
`run_output(from)` geben Fehler und Ausgabe des aktuellen Laufs (`run_output` ab einer Stelle,
damit eine Anzeige nur Neues liest), `ScriptView::runs` zählt die Starts. Ersetzt
`Action::Service` einen Dienst gleichen Namens, gibt der alte seine Bytes auf der Disk frei. Die
Oberfläche schickt diese Actions über `Sim::keep` ([`src/doc/sim.rs`](src/doc/sim.rs),
`keep_steps`), so laufen Bearbeiten, Rückgängig und Wiederholen ohne Neustart des Geräts.

### Labore und Aufgaben

Ein Labor ist ein Projekt mit einer Aufgabe: das Netz, wie es anfängt, eine Einleitung, Ziele,
die der Simulator selbst prüft, und Hinweise. Datei › Labor öffnen … listet die Labore der App
(`assets/labs/*.jfnet`, im Programm als `app://labs`) mit Thema, Titel, Zahl der Ziele und
einer Zeile Beschreibung; Öffnen (auch Doppelklick oder Eingabe) fragt erst nach ungespeicherten
Änderungen und lädt das Labor als Projekt ohne Datei (der Titel nennt die Datei des Labors,
Speichern legt eine Kopie an, die ihr Labor behält).

| Labor | Thema | worum es geht |
|---|---|---|
| `01-campus-vlan.jfnet` | Netzwerk | der Campus ohne VLAN 20: das VLAN und sein Interface auf MLS1 mit `ip helper-address`, die Ports von PC3 und PC4, die Route zurück auf R1, dann DHCP über das Relay und ein Ping per Name (alles in den IOS-artigen Konsolen) |
| `02-lastverteilung.jfnet` | Verteilte Systeme | 360 Anfragen pro Sekunde auf zwei Webserver am Limit (p99 um 340 ms, 503er): WEB3 verkabeln und in die Backends von LB1 aufnehmen, dann p99 um 45 ms ohne Fehler |
| `03-shards.jfnet` | Verteilte Systeme | drei KV-Knoten mit Replikationsfaktor 1 und strong: erst Replikate, dann KV2 ausschalten, ohne dass eine Anfrage scheitert (zwei Replikate mit strong reichen nicht, drei schon, oder eventual), dann holt KV2 über Read Repair auf |
| `04-cache.jfnet` | Verteilte Systeme | ein Cache mit 20 Einträgen und 200 ms vor einem Server, der kaum 100 Anfragen pro Sekunde schafft: größer und länger einstellen, bis vier von fünf Anfragen aus dem Cache kommen und der Server Luft hat |

In den drei Laboren der verteilten Systeme zählen die Ziele der Reihe nach: ein Ergebnis wie „p99
unter 100 ms“ oder „keine Fehler“ zählt erst nach dem Umbau, nicht schon in den ersten Sekunden,
bevor die Last die Server füllt.

- **Das Panel „Aufgabe“** (rechts als Reiter neben den Eigenschaften, Ansicht › Bereiche ›
  Aufgabe, Strg+Umschalt+A; es kommt mit einem Labor nach vorn, die rechte Seite wird dann
  mindestens 340 DIP breit): Titel, Thema, „2 von 5 Zielen erreicht“ mit Fortschrittsbalken
  und „Prüfen“ (F9, auch Simulation › Ziele prüfen), die Einleitung zum Einklappen, die Ziele
  als Checkliste und die Hinweise. Der Simulator prüft die Ziele jede Sekunde virtueller Zeit
  und nach jeder Änderung des Projekts (höchstens alle 250 ms); ein erreichtes Ziel bleibt
  erreicht und nennt den Zeitpunkt. „Prüfen“ prüft sofort, und von da an zeigt jedes offene Ziel
  seine Prüfungen, die nicht halten, mit dem, was sie fanden („LB1 svc lb metric backends_up ==
  3: gemessen: 2“). Hinweise öffnen sich nacheinander: die eines Ziels unter ihm, die der
  ganzen Aufgabe darunter (Simulation › Nächster Hinweis nimmt den nächsten des ersten offenen
  Ziels). In einem geordneten Labor wartet ein Ziel, bis die davor erreicht sind. Sind alle
  erreicht, sagt es „Geschafft“. Ohne Labor zeigt das Panel einen leeren Zustand, der zur
  Liste führt. „Simulation zurücksetzen“ und ein neu geöffnetes Labor beginnen von vorn.
  Für Screenreader ist jedes Ziel ein Kontrollkästchen (angehakt: erreicht), der Fortschritt
  ein Fortschrittsbalken.
- **Einstellungen der Dienste:** Im Gerätefenster (Umschalt+Doppelklick) hat jeder Dienst des
  Projekts unter „Dienste“ den Knopf „Einstellungen …“: seine Konfiguration als eingerücktes
  JSON in einem Code-Editor. Übernehmen liest das JSON (ein Fehler wird mit Zeile und Spalte
  genannt und im Editor markiert), lässt dann einen eigenen Simulator den Dienst auf einem
  leeren Gerät desselben Modells bauen (was der Simulator ablehnt, kommt nicht ins Projekt) und
  ist dann ein Rückgängig-Schritt; das Gerät startet mit den neuen Einstellungen neu. So
  entstehen das dritte Backend, die Replikate und die Größe des Caches der Labore.

Die Aufgabe steht im Abschnitt `lab` der Projektdatei; ein Projekt ohne Labor hat dort `null`
wie bisher. Ein Text ist ein String (Deutsch, in jeder Sprache) oder ein Paar `["Deutsch",
"English"]`; eine Prüfung ist eine Erwartung wie in Szenarien ohne das `expect` davor (siehe
[Szenarien](#szenarien-scn): Ausgabe, Route, keine Route, Frames, Metrik, Metrik über einen
Zeitraum), und ein Ziel ist erreicht, wenn alle seine Prüfungen in einem Moment halten.
`output contains` liest im Fenster die Konsole des Geräts.

```json
"lab": {
  "title": ["Lastverteilung: der dritte Webserver", "Load balancing: the third web server"],
  "summary": ["Eine Zeile für die Liste", "One line for the list"],
  "topic": "ds",
  "intro": [["Ein Absatz der Einleitung", "A paragraph of the introduction"]],
  "ordered": false,
  "goals": [{"text": ["LB1 verteilt auf alle drei Webserver", "LB1 spreads the load over all three"],
             "checks": ["LB1 svc lb metric backends_up == 3", "WEB3 svc web metric requests >= 100 over 5s"],
             "hints": [["Erster Hinweis", "First hint"], ["Zweiter Hinweis", "Second hint"]]}],
  "hints": [["Ein Hinweis zur ganzen Aufgabe", "A hint for the whole task"]]
}
```

Für Programme: `jfn_model::lab` (`Lab`, `Goal`, `Text`, `Lab::issues`,
`scenario::parse_expectation`) und `jfn_sim::goal`: `judge(&projekt, &engine, &erwartung,
ausgabe)` urteilt über eine Erwartung zum jetzigen Zeitpunkt der Engine, genau wie der Runner
(der seine Erwartungen jetzt darüber prüft; seine Berichte sind byte-gleich geblieben), mit
`Verdict::report()` (die Berichtszeile) und `Verdict::reason()` (was sie fand, als Meldung für
den Katalog); `LabRun` führt die Ziele eines Labors über einen Lauf. Beides liest nur und ist
deterministisch. Geprüft wird das in der Selbsttest-Gruppe `task`
([`src/tasktest.rs`](src/tasktest.rs)): jedes Labor ungelöst (kein Ziel wird erreicht) und
gelöst wie im Fenster (das Projekt geändert, der Simulator folgt ihm wie in `doc/sim.rs`), und
in den Fensterskripten `task.uis`, `labs.uis` und `task_shots.uis`.

## Performance

Große Projekte bleiben flüssig: `tests/perf.uis` verlangt mit 500 Geräten, 800 Kabeln und 300
Rahmen gleichzeitig auf den Kabeln in 1920 × 1080 höchstens 33 ms pro Bild (30 fps).

- **Testprojekt:** `JFNetworker --gen-topology 500` öffnet ohne Datei einen synthetischen
  Campus aus Katalogmodellen (`jfn_model::synth`): 7 Kern-Router im 100G-Ring, 19 Pods aus einem
  Router mit zwei Uplinks, zwei Access-Switches, PCs, Drucker, Kamera und Servern mit je einem
  Port an beiden Switches; statische Routen, Adressen aus 10.P.1.0/24 und 10.P.2.0/24. Kabel über
  einen Baum hinaus verbinden nur geroutete Interfaces oder die zwei Netze eines Servers, es gibt
  also keine Schleife auf Schicht 2 (noch kein STP). `jfn-sim gen 500 --out campus.jfnet`
  schreibt dasselbe Projekt als Datei; gleiche Größe, gleiches Projekt.
- **Verkehr:** `--traffic 300` startet 300 Pings auf einmal (`Engine::ping_many`), deren
  ARP-Anfragen zur Zeit 0 alle auf den Kabeln liegen; der Simulationsmodus zeigt sie als 300
  bewegte Rahmen (die Zahl steht im Semantik-Eintrag „Rahmen auf den Kabeln“).
- **Messung:** `bench N anim|drag|wheel … 33000` (ui/auto.rs) misst mit der echten Uhr die
  Durchläufe und das Neuzeichnen der schmutzigen Zellen pro Bild; der Vergleich von
  `--verify-dirty` läuft danach, ungezählt. Bestanden ist ein Lauf, dessen Median höchstens 33 ms
  beträgt, mit bis zu drei Läufen gegen Last auf dem Rechner.

Was es schnell macht:

- **Backing-Store der Leinwand** (`ui::diagram`): Raster, Kabel und Geräte werden einmal pro
  Version (Projekt, Ansicht, LEDs, Theme) in eine eigene Fläche gemalt; jedes schmutzige Rechteck
  kopiert nur noch seinen Teil heraus. Bewegte Rahmen, Hover und Auswahl malen die Szene nicht
  mehr neu.
- **Culling über den räumlichen Index:** der Painter fragt die Buckets des Widgets
  (`DgFrame::find`) und rechnet nur für sichtbare Kabel LEDs und Portnamen.
- **Detailstufe während Gesten:** solange Verschieben, Zoomen mit dem Rad oder das Ziehen von
  Geräten läuft (`DgFrame::moving`) und mehr als 150 Geräte und Kabel zu sehen sind, entfallen
  Namen, LEDs und Portnamen, Kabel sind schlichte 1-px-Linien (`dg_hairline`); danach zeichnet die
  Leinwand wieder in voller Qualität.
- **Rahmen:** ihre Beschriftungen („Rahmen N: A → B · Info“) werden einmal dekodiert und
  gemerkt, Rahmen außerhalb der Ansicht nicht gezeichnet, doppelte Kennungen (Nummer % 1024)
  nicht vergeben.
- **Szene:** `Scene::build` sucht Geräte, Modelle und Ports über Tabellen statt linear.
- **Engine:** jede Weiterleitung schlug die Routingtabelle neu zusammen; jetzt hält `IpStack`
  die Tabelle der letzten Suche mit dem, woraus sie entstand (Zustand und Adressen der Interfaces,
  statische Routen), und baut sie nur bei einer Änderung neu. Determinismus und Golden-Berichte
  bleiben unverändert.

Messwerte auf dem Entwicklungsrechner (Windows, Median von drei Läufen in ms, 1920 × 1080):

| Szene | vorher | nachher |
|---|---|---|
| Echtzeit, die 300 Pings laufen | 1,3–1,4 | 1,3–1,5 |
| Simulation, ganzer Campus (17 %), Rahmen bewegen sich | 7,7–8,4 | 3,7–4,6 |
| … Verschieben mit der mittleren Taste | 9,1–10,6 | 7,1–8,3 |
| … Zoom mit Strg + Rad | 9,2–10,7 | 7,2–8,1 |
| 100 % über zwei Pods, Rahmen bewegen sich | 2,3–2,6 | 2,2–2,3 |
| … Verschieben | 8,9–10,4 | 6,9–8,0 |
| … Zoom | 9,6–12,2 | 7,2–9,0 |
| … einen Server ziehen | 10,9–12,8 | 8,4–10,1 |
| Abspielen mit 8× | 28,6–35,0 | 3,3–3,4 |

Ein ganzes Fenster neu zeichnen dauert statt 17–21 ms noch 3–4 ms; 300 Pings mit gelerntem ARP
(9 460 Ereignisse) rechnet die Engine in 6–7 statt 14 ms. Was ein Bild jetzt noch kostet, sind
vor allem die Durchläufe der übrigen Oberfläche (Panels, zwei Durchläufe pro Bild während einer
Geste).

## Fahrplan

| Phase | Meilensteine |
|---|---|
| 1 Netzwerk-Kern | E1 Paket-Library ✓ · E2 Datenmodell ✓ · E3 Simulationskern mit Switching ✓ · S1 Style `netlab` ✓ · L1 Diagramm-, L2 Terminal-, L3 Symbol-Widget ✓ · L4 Zeitleiste und Hex-Ansicht ✓ · E4 IPv4, ARP, ICMP, Ping, Traceroute ✓ · E5 Befehlszeilen und Szenario-Runner ✓ · E6 UDP, DHCP, DNS, Syslog, NTP ✓ · E7 Simulationsmodus (Engine), Mitschnitte, Probleme ✓ · U1 Topologie-Editor ✓ · U2 Simulator im Editor, Verbinden, Kabel, Inspektor ✓ · U3 Konsolen, Gerätefenster, Protokoll ✓ · U4 Simulationsmodus im Fenster, PDUs ✓ · U5 Politur und Abnahme ✓ |
| 2 parallele Stränge | T1 TCP ✓ · T2 HTTP, Lastgenerator, `curl`/`nc`/`ss` ✓ · Skriptsprache `lib/script` (SK1 Lexer und Parser ✓, SK2 Compiler und VM ✓, SK3 Standardbibliothek ✓) · NAT/ACL, RIP, OSPF, STP, IPv6 · Liniendiagramm und Code-Editor |
| 3 Verteilte Systeme | D1 Loadbalancer, KV-Shards, Cache, Queue, Worker, Fehlerinjektion ✓ · SK4 Skripte in Hosts ✓ · D2 Metrik-Fenster und Zeitreihen ✓ · U6 Labor-Oberfläche (U6a Metriken, Badges, Fehlerinjektion; U6b Aufgaben und Labore) ✓ · U7 Skript-Editor ✓ |
| 4 Parität | WLAN, physische Ansicht, Aufgaben, Zurückspulen mit Keyframes, Gruppen, weitere Dienste |

**Regel für E5 und alle späteren Befehlszeilen:** Hilfetexte (`?`, `help`), Statusmeldungen,
Banner und Versionsausgaben der IOS-artigen und der Linux-artigen Befehlszeile schreiben wir
immer in eigenen Worten. Nichts davon wird aus Herstellerdokumentation, echten Geräteausgaben
oder Büchern übernommen. Dem Vorbild folgen dürfen nur die Befehlswörter, ihre Syntax,
Interface-Namen, Konfigurationszeilen und kurze Fehlermeldungen wie `% Incomplete command.`,
weil Lernende sie wiedererkennen sollen. `apps/termtest` zeigt das Muster.

## Marken

Cisco, IOS und Packet Tracer sind Marken von Cisco Systems, Inc. JFNetworker ist ein
eigenständiges Projekt und steht in keiner Verbindung zu Cisco; es wird von Cisco weder
unterstützt noch empfohlen. Die Namen stehen hier nur, um Befehlszeilen-Konventionen und
vergleichbare Programme zu beschreiben.
