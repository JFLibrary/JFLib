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
  im Fenster mit dem Bereich Metriken, Dienst-Pillen und Sparklines auf der Zeichenfläche und
  Fehlern aus dem Inspektor (siehe [Das Labor im Fenster](#das-labor-im-fenster)), dazu über
  die Shell (`lbctl`, `kvctl` …) und Szenarien (siehe
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
- **Spanning Tree (R4):** STP (IEEE 802.1D) und RSTP (802.1w) auf den IOS-artigen Switches, je
  Switch eingeschaltet: Wahl der Root-Bridge, Rollen und Zustände, Proposal und Agreement,
  Edge-Ports, BPDU Guard, `show spanning-tree`, bernsteinfarbene LEDs an blockierten Ports
  (siehe [Spanning Tree](#spanning-tree)).
- **Routing mit RIP (R2):** Router und L3-Switches sprechen RIPv2 nach RFC 2453 (Version 1 nach
  RFC 1058, wo eingestellt): `router rip` mit `network`, `passive-interface`,
  `default-information originate`, `redistribute`, `timers basic`, `distance` und
  `maximum-paths`; periodische und ausgelöste Updates, Split Horizon mit Poisoned Reverse,
  Timeout, Garbage Collection und Holddown, gleich gute Wege; `show ip rip database`, `show ip
  protocols`, `debug ip rip` (siehe [Routing mit RIP](#routing-mit-rip)).
- **NAT und Zugriffslisten (R1):** Router, L3-Switches und die Firewall übersetzen Adressen
  (PAT, Pools, statische Adressen und Port-Weiterleitungen, ICMP-Fehler nach RFC 5508) und
  filtern mit Standard- und erweiterten Zugriffslisten, nummeriert und benannt, mit Zählern und
  den Befehlen des Vorbilds (siehe [NAT und Zugriffslisten](#nat-und-zugriffslisten)).
- **IPv6 (R5):** auf denselben Interfaces wie IPv4, aus bis ein Gerät dafür eingerichtet wird:
  Link-Local-Adressen aus der MAC, Duplikaterkennung, Router Advertisements und SLAAC mit
  Lebensdauern, der Neighbor Cache mit allen Zuständen, statische Routen, Weiterleiten mit Hop
  Limit, Packet Too Big und die Fragmente des Absenders, ICMPv6 mit den Grenzen des Vorbilds;
  UDP und TCP über beide Familien (ein Socket auf `::` nimmt beide), der Resolver fragt A und
  AAAA und ordnet sie nach RFC 6724, DNS, HTTP, die TCP-Testdienste und Skripte über IPv6;
  `ipv6 …`, `show ipv6 interface|route|neighbors`, `ping ipv6`, `traceroute ipv6`, `debug ipv6
  icmp|nd`, `ip host`, `ipv6 host`, `ip name-server` und unter Linux `ip -6`, `ping6`,
  `traceroute6`, `sysctl net.ipv6.*`, `curl`, `nc` und `ss` mit `-4`/`-6`, `dig`, `host` und
  `nslookup` (siehe [IPv6](#ipv6)).
- **Routing mit OSPF (R3):** Router, L3-Switches und die Firewall sprechen OSPF Version 2 nach
  RFC 2328: `router ospf N` mit `network … area`, `router-id`, `passive-interface`, `auto-cost
  reference-bandwidth`, `area … stub [no-summary]`, `redistribute`, `default-information
  originate`, `distance` und `maximum-paths`, an Interfaces Kosten, Priorität, Timer und
  Netztyp; Hellos, Wahl von DR und BDR, Nachbarn bis Full mit Datenbankabgleich, Flooding mit
  Bestätigungen und Wiederholungen, Altern der LSAs, SPF mit gleich guten Wegen, Bereiche über
  einen ABR, Stub-Bereiche und externe Routen (E1, E2); `show ip ospf
  [neighbor|interface|database]`, `debug ip ospf` (siehe [Routing mit OSPF](#routing-mit-ospf)).
- **Aufgaben mit Bewertung (A1):** Labore mit Kriterien, die Punkte bringen: Namen, Adressen und
  Zustände von Interfaces, VLANs und Modi von Switchports, statische Routen, Kabel, Einstellungen
  von Diensten, Zeilen der Running-Config (Abkürzungen zählen) und Verbindungsprüfungen; die
  Rückmeldung je Kriterium, nur als Punktzahl oder erst nach dem Abgeben, wahlweise mit
  Zeitgrenze; Werkzeuge › Aufgabe erstellen … macht aus einem fertigen Netz eine Aufgabe mit
  Ausgangsnetz; drei Beispiele (siehe [Aufgaben mit Bewertung](#aufgaben-mit-bewertung)).
- **WLAN (W1):** Access Points und der Heimrouter bieten Netze an (SSID, 2,4, 5 oder 6 GHz mit
  Kanal, offen, WPA2- oder WPA3-Personal, VLAN, Gastnetz, Client-Isolation), Laptops, Smartphones
  und andere Stationen melden sich ohne Kabel an: Suche, Authentifizierung, Assoziation und der
  4-Wege-Handshake mit echten EAPOL-Frames, Signal und Rate aus der Entfernung im Maßstab des
  Projekts, Roaming beim Verschieben, Verbindungsverlust außer Reichweite; Funklinien,
  Reichweiten und Signalbalken auf der Zeichenfläche, die Einstellungen im Inspektor, `iw` in der
  Shell, `show dot11 associations` am Access Point (siehe [WLAN](#wlan)).

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
| `metrics.uis` | der Bereich Metriken auf `lab_window.jfnet`: ohne Messwerte, nach 8 s Statuszeile, Kacheln und Diagramme (die Zusammenfassungen ihrer Einträge), die Tastatur im Baum und in den Diagrammen, die Werte beim Überfahren, ein auf der Zeichenfläche gewähltes Gerät, der Simulationsmodus auch in der Vergangenheit, die Sparklines ab 75 % (siehe [Das Labor im Fenster](#das-labor-im-fenster)) |
| `faults.uis` | Fehler aus dem Inspektor auf `lab_window.jfnet`: Strom aus, Rückgängig und Wiederholen, Wiederherstellen und Alle wiederherstellen samt Rückgängig, ein Dienst für 2 s, die Disk, ein Kabel gezogen und verschlechtert, eine Partition, Ablehnungen, das Projekt bleibt gespeichert, die Spur „Fehler“ der Zeitleiste und der Fehler als Bereich im Diagramm von LB1 |
| `lab_shots.uis` | die Bild-Matrix des Labors (Zeichenfläche mit Pillen und Sparklines, Fehlerkarte, Metriken) hell und dunkel mit 100 und 150 % |
| `model.uis`, Gruppe `labui` | was das Labor im Fenster braucht: die Texte des Labors als Meldungen mit Englisch (Statuszeilen, Disk, jede Art von Fehler, ihre Ereignisse), KPIs vergangener Zeiträume, die Tabellen des Bereichs Metriken, Pillen und Sparklines, Fehler als Rückgängig-Schritte samt gespeichertem Zustand |
| `model.uis`, Gruppe `stp` | Spanning Tree (siehe [Spanning Tree](#spanning-tree)): aus ohne Konfiguration (keine BPDUs, die Schleife mit dem alten Text), die ersten BPDUs Byte für Byte, Wahl und Rollen im Dreieck und im Quadrat mit STP und RSTP, die Timer von 802.1D (Learning nach 15 s, Forwarding nach 30 s, nach einem indirekten Ausfall 47 s: Max Age minus Message Age plus 2 × 15 s), Proposal und Agreement in Mikrosekunden, Edge-Ports (portfast sofort, AutoEdge nach 3 s, eine BPDU beendet sie), TCN, TC-Flag für 35 s und kurzes Altern, RSTP löscht MAC-Einträge, STP und RSTP gemischt, BPDU Guard (auch ohne Spanning Tree), Kosten, Prioritäten, 10G, ein Hub (Backup-Port, geteiltes LAN), Befehle, Ausgaben und Debug, LEDs, Simulationsmodus, die Probleme-Liste, Determinismus, Englisch |
| `stp.uis` | Spanning Tree auf der Zeichenfläche (`fixtures/stp_triangle.jfnet`): LEDs bernstein beim Listening und Learning, Edge-Ports grün, nach 30 s nur noch der Alternate-Port von SW3 bernstein, die Kabel „blockiert“ für Screenreader, der Inspektor von SW3; Bilder hell und dunkel (`build/jfnetworker/stp_*.png`) |
| `model.uis`, Gruppe `rip` | RIP (siehe [Routing mit RIP](#routing-mit-rip)): der erste Austausch auf die Nanosekunde (Request, Update ohne das Netz des Interfaces, die Antwort, die das IOS-artige ARP verliert), Konvergenz und Metriken über zwei Hops, periodische Updates mit Zufallsversatz und ausgelöste mit ihrer Pause, Split Horizon mit Poisoned Reverse auf dem Kabel, ein gezogenes Kabel (ausgelöstes Update mit Metrik 16, Holddown, Garbage Collection nach 120 s), ein schweigender Nachbar (Timeout nach 180 s, gelöscht nach 300 s), passive Interfaces, Distanzen gegen statische Routen, gleich gute Wege (ein Weg je Ziel, `maximum-paths`), Version 1 neben 2, `auto-summary`, Standardroute und Redistribution, RIP auf dem VLAN-Interface eines L3-Switches, die Befehlszeile (Modus, Hilfe, `show`, `debug`, `clear`, die Running-Config als Startkonfiguration), Strom, die Texte auf Deutsch und Englisch, Determinismus |
| `model.uis`, Gruppe `nat` | Zugriffslisten und NAT (siehe [NAT und Zugriffslisten](#nat-und-zugriffslisten)): jeder Port-Operator, Wildcards, Protokolle, ICMP-Typen, `established`, spätere Fragmente, die erste Zeile gewinnt, das implizite deny; Zeilennummern, Bemerkungen, `no`, `resequence`; die Konsole mit Modi, Hilfe, `show access-lists` und der Konfiguration, die gelesen wieder dieselbe ist; Listen an Interfaces mit Zählern, Notizen und „administratively prohibited“ unter der Grenze (auf die Nanosekunde), die ausgehende Liste nach NAT, eigene Pakete des Routers; PAT mit zwei Hosts am selben UDP- und TCP-Port (Parität), gleichen ICMP-Kennungen, ein erschöpfter Pool, statische Adressen und Ports mit ARP für die globale Adresse, Filter von außen, ICMP-Fehler in beide Richtungen, Fragmente auch außer der Reihe, die Zeiten der Übersetzungen, Löschen, Strom, `debug ip nat`, deutsche und englische Texte, VLAN-Interfaces, Subinterfaces, die Firewall; die Prüfsummen jedes Frames |
| `model.uis`, Gruppe `ipv6` | IPv6 (siehe [IPv6](#ipv6)): Link-Local-Adressen aus der MAC und ihre Duplikaterkennung mit der Pause aus dem eigenen Zufallsstrom des Geräts auf die Nanosekunde, eine doppelte Adresse unter Linux und IOS-artig (`dadfailed`, `[DUP]`, das Problem), Router Solicitations und Advertisements (Zeiten, Inhalt, Rahmen), SLAAC mit Lebensdauern (veraltet, abgelaufen, die Zwei-Stunden-Regel), der Neighbor Cache durch INCOMPLETE, REACHABLE, STALE, DELAY und PROBE und beide Arten zu scheitern, Ping und Traceroute über zwei Router (statische Routen, ein Link-Local-Next-Hop, Hop Limit, keine Route, eine Schleife), Packet Too Big und die Fragmente des Absenders, die Path MTU nach 10 Minuten, aufgegebene Fragmente, die Grenzen der ICMPv6-Fehler (IOS-artig und Linux), die Befehlszeilen (Hilfe, `show`, `debug`, `clear`, die Konfiguration gelesen wieder dieselbe), Strom und abgeschaltetes Routing, ein Projekt ohne IPv6 wie zuvor, deutsche und englische Texte, die Prüfsummen jedes Rahmens, Determinismus |
| `ipv6_inspector.uis` | IPv6 im Fenster (`fixtures/ipv6_slaac.jfnet`): die Spalte IPv6 im Inspektor und im Gerätefenster, die Adressen des gewählten Interfaces mit Herkunft und Zustand, ein Switch ohne IPv6 behält seine Spalten; ein Bild (`build/jfnetworker/inspector_ipv6.png`) |
| `model.uis`, Gruppe `ospf` | OSPF (siehe [Routing mit OSPF](#routing-mit-ospf)): Hellos auf dem Kabel (an 224.0.0.5 mit TTL 1 und Präzedenz 6, alle 10 s) und das Dead-Intervall, die Wahl von DR und BDR mit Prioritäten (ohne Verdrängen; fällt der DR aus, übernimmt der BDR), eine Nachbarschaft durch jeden Zustand auf die Nanosekunde (Master und Slave, Database Description, Request, Update), am LAN die Wartezeit und die erste DD, die das IOS-artige ARP verliert, ein LSA, das vor MinLSArrival kommt, verworfen und wiederholt, Flooding mit verzögerten und direkten Bestätigungen, MinLSInterval, SPF 200 ms nach einer Änderung, Kosten aus der Referenzbandbreite und je Interface, gleich gute Wege, ein gezogenes Kabel und der neue Weg, passive Interfaces mit `traceroute`, Redistribution (E1, E2, `subnets`) und die Standardroute gegen statische Routen und Distanzen, zwei Bereiche über einen ABR mit Stub und `no-summary`, das Erneuern nach 1800 s und MaxAge nach 3600 s, Unstimmigkeiten (Hello-Intervall, Bereich, MTU), die Befehlszeile (Modus, Hilfe, `show`, `debug`, `clear ip ospf process`, die Running-Config als Startkonfiguration), die Texte auf Deutsch und Englisch, Determinismus |
| `model.uis`, Gruppe `ipv6tcp` | UDP und TCP über IPv6 (siehe [IPv6](#ipv6)): der Handshake Byte für Byte auf die Nanosekunde (Port und ISN aus dem Zufallsstrom des Geräts, MSS 1440 aus der MTU des Pfads, Linux' Fenster 64800, die Prüfsumme über den IPv6-Pseudo-Header), Daten und Abbau bis TIME-WAIT, RST an einem geschlossenen Port, die sechs SYN-Wiederholungen bis 127 s, Packet Too Big von R1 senkt die MSS auf 1220, ICMPv6-Fehler beenden einen Verbindungsaufbau und erreichen verbundene UDP-Sockets (auch über IPv4), nie unverbundene; die Dual-Stack-Sockets (`::` nimmt beide Familien, `0.0.0.0` nur IPv4, eine Adresse nur sich, was sich einen Port teilen darf, `ss -4`/`-6`), der Resolver mit A und AAAA in der Reihenfolge von RFC 6724 (globale IPv6-Adresse: IPv6 zuerst, nur Link-Local: IPv4 zuerst, ohne IPv6 nur A), `curl`, `nc`, der TCP-Client am Echo, ein Skript mit `http.request` und `dns.resolve`, `ping` und `traceroute` per Name, `dig`, `host`, `nslookup` (auch ohne Antwort und abgewiesen), die IOS-artige Konsole (`ip name-server` über IPv6, `ip host`, `ipv6 host`, `ping` per Name, die Running-Config, `show tcp brief`), Masken, Sätze und Mitschnitt, Determinismus |
| `model.uis`, Gruppe `assess` | Aufgaben mit Bewertung (siehe [Aufgaben mit Bewertung](#aufgaben-mit-bewertung)): das Format (was auf seinem Vorgabewert steht, fehlt in der Datei; ohne `points` 1 Punkt), was `Assess::issues` findet, die kanonischen Werte (Masken, Präfixe, IOS-artige und Linux-Schreibweisen von Adressen und Routen), `reaches` auf einer Kopie des Simulators (beantwortet, keine Route, ein ausgeschalteter Router oder PC, ein unbekannter Name, der Lauf unberührt, die Zeile des Runners), jede Art von Kriterium mit ihren Schreibweisen, gleichwertig, genau und einer von mehreren Werten, Zeilen der Konfiguration abgekürzt, mit `no shutdown` und einer abgelehnten Zeile, Punkte und Prozent, ein umbenanntes Gerät über seine Nummer, die Rückmeldungen, wann bewertet wird und die Zeitgrenze, das Erstellen (das laufende Netz als Ziel, der Baum der Kandidaten, das Ausgangsnetz: was ein Kriterium prüft, fehlt dort und gilt im Ziel), die vier Labore wie bisher, die drei Aufgaben ohne Befund, ungelöst mit 0 Punkten, mit ihrem Zielnetz und gelöst wie im Fenster mit 20, dieselben Punkte zweimal |
| `assess_author.uis` | eine Aufgabe erstellen aus dem Router zwischen zwei Netzen: das Ziel, Kriterien aus dem Baum mit Punkten, einem zweiten zulässigen Wert (ein ungültiger abgelehnt) und eigenem Text, eine abgelehnte und eine angenommene Prüfung, die Texte, eine Minute Zeit, das Ausgangsnetz (rückgängig und wieder), die Aufgabe ins Projekt (das Panel „Aufgabe“ folgt Rückgängig und Wiederholen), gespeichert als `build/jfn_assess.jfnet`, die Minute läuft ab, die Datei wieder geöffnet und der Bereich aus ihr gefüllt |
| `assess_solve.uis` | die Aufgaben im Panel: die Liste mit ihnen, „Ein Router für zwei Netze“ mit „Prüfen“ und den Befunden, in zwei Schritten in der Konsole gelöst (7, dann 20 von 20 Punkten), Rückgängig des Namens kostet seine Punkte; „Statische Routen über drei Router“ ohne Rückmeldung bis zum Abgeben (der Countdown, eine Route, die Rückfrage abgebrochen und beantwortet, dann alle Kriterien mit ihren Befunden); „VLANs und ein Trunk zum Router“ nur mit der Punktzahl, früh abgegeben |
| `assess_en.uis` | dasselbe auf Englisch: die Liste, das Panel mit den Texten der Datei, F9 mit den Befunden, der Bereich „Author an assessment“ aus der Aufgabe gefüllt |
| `assess_shots.uis` | die Bilder der Liste, des Panels (halb gelöst nach „Prüfen“) und des Bereichs „Aufgabe erstellen“ hell und dunkel mit 100 und 150 % (`build/jfnetworker/assess_*.png`) |
| `model.uis`, Gruppe `wlan` | WLAN (siehe [WLAN](#wlan)): Signal, Rate, Verlust und Zugang zum Medium aus der Entfernung, Kanäle, Bänder, Reichweiten und Ports der Modelle, was ein Netz ungültig macht; PRF und KDF des 802.11-Standards, PMK und PTK gegen unabhängig gerechnete Werte, MICs mit HMAC-SHA1 und AES-CMAC, der GTK im Key Wrap, SAEs Ersatz-PMK; Suche, Authentifizierung und Assoziation auf die Nanosekunde (offen, WPA2, WPA3 mit SAE), die vier EAPOL-Frames mit ihren Zeiten und von außen nachgerechneten MICs, eine falsche Passphrase (Wiederholungen nach 100 ms und 1 s, Grund 15, Pausen von 10, 20 und 30 s, Aufgeben), ein falsches SAE-Passwort, WPA2 auf 6 GHz; die Wahl von Band und Access Point, DHCP und Ping über WLAN, Roaming während eines Pings ohne Verlust, der Rand der Reichweite, Verbindungsverlust nach sieben Beacons, Access Point aus, umkonfiguriert und mit abgeschaltetem Funk, Client-Isolation und Gastnetze (auch am Heimrouter), Funkkabel älterer Projekte (feste Zuordnungen wie bisher, ausgesteckt sucht die Station), `iw` und `show dot11 associations`, die Probleme-Liste, Datei, Szenario-Schritt `move`, Rückgängig über das Dokument, Texte auf Deutsch und Englisch, Determinismus |
| `wlan_inspector.uis` | der WLAN-Teil des Inspektors auf `fixtures/wlan_office.jfnet`: die Netze von AP1 (Passphrase maskiert, bis sie gezeigt wird), eine neue SSID schickt Phone1 weg und rückgängig gemacht kommt es wieder, offen zu WPA2 erst mit Passphrase, ein Netz hinzugefügt und entfernt; eine Station mit Zustand, Access Point, Signal mit Balken, Rate und den Netzen in Reichweite, ihre Passphrase im Inspektor korrigiert und rückgängig gemacht, Abmelden |
| `wlan_range.uis` | die Reichweiten: ein gewählter Access Point zeigt seine Kreise, Ansicht › WLAN-Reichweiten alle (auch mit Alt+A, W), die Kreise als Texteinträge, der Maßstab im Inspektor des Projekts mit Rückgängig und einer Ablehnung |
| `wlan_drag.uis` | Laptop1 bei gehaltener Maustaste aus der Reichweite gezogen: „außer Reichweite“, nach sieben fehlenden Beacons getrennt, die Station sucht; Loslassen ist ein Rückgängig-Schritt, rückgängig gemacht meldet sie sich wieder bei AP1 an |
| `wlan_shots.uis` | die Bilder von WLAN (Reichweiten, Funklinien, Signalbalken, Inspektor eines Access Points und einer Station) hell und dunkel mit 100 und 150 % (`build/jfnetworker/wlan_*.png`) |

Fixtures in `tests/fixtures/`: `office.jfnet` (ein kleines Büronetz mit jeder Kabelart),
`cables.jfnet` (Geräte ohne Kabel), `pdu_lab.jfnet` (DHCP, DNS-Zone `lab`, TCP-Echo),
`campus.jfnet` (die Abnahme), `lab_window.jfnet` (das Labor der Fenstertests: Cache,
Loadbalancer, zwei Webserver, drei KV-Knoten, Queue mit Worker, DNS, DHCP, ein Skript),
`stp_triangle.jfnet` und `rstp_triangle.jfnet` (drei Switches im Dreieck mit STP bzw. RSTP),
`wlan_office.jfnet` (zwei Access Points, ein Gastnetz, zwei Laptops und ein Smartphone; das
Szenario `wlan_office.scn` und die WLAN-Fensterskripte) und die Netze der Golden-Szenarien. Die
Bausteine aus `lib/` prüft `coretest` (Gruppen `netpkt`, `dhcp`, `dnsmsg`, `ntp`, `httpmsg`,
`rng`, `metrics`, `jmap`, `cmdtree`, `hashring`, `lru`, `stp`, `icmpv6`, `ospf`, `addrsel`,
für WLAN `crypto` und `netpkt`), die Skriptsprache die Gruppe `script`.

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
| Pakete | [`lib/netpkt`](../../lib/netpkt/lib.rs) | Adressen, Prüfsummen, Codecs von Ethernet bis TCP, DHCP, DNS, NTP, Syslog, HTTP/1.1 (inkrementell, [`http.rs`](../../lib/netpkt/http.rs)), RIP ([`rip.rs`](../../lib/netpkt/rip.rs)), OSPF ([`ospf.rs`](../../lib/netpkt/ospf.rs)), Dissektor, pcapng (Library, ohne I/O) |
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
| | [`src/workspace/metrics.rs`](src/workspace/metrics.rs), [`canvas/lab.rs`](src/workspace/canvas/lab.rs), [`inspector/faults.rs`](src/workspace/inspector/faults.rs), [`src/doc/faults.rs`](src/doc/faults.rs), [`src/labuitest.rs`](src/labuitest.rs) | das Labor im Fenster (U6a): der Bereich Metriken, Dienst-Pillen und Sparklines, Fehler aus dem Inspektor als Rückgängig-Schritte, ihr Selbsttest (Gruppe `labui`; siehe [Das Labor im Fenster](#das-labor-im-fenster)) |
| Texte | [`crates/jfn_model/msg.rs`](crates/jfn_model/msg.rs), [`src/locales/engine.tsv`](src/locales/engine.tsv), [`src/locales/ui.tsv`](src/locales/ui.tsv), [`src/msgtest.rs`](src/msgtest.rs) | die deutschen Texte von Modell und Simulator als Vorlagen mit Argumenten (`Msg`, `msg!`; `text_with` übersetzt), die des Fensters (`t()`, `tf()`, die Befehle und Menüs), ihre englische Fassung und deren Selbsttest; Vollständigkeit: `python tools/i18n_check.py apps/JFNetworker crates` |
| Tests | [`tests/`](tests) | Skripte, Fixtures (`fixtures/*.jfnet`), Szenarien mit Golden-Berichten (`scen/*.scn`, `scen/*.report`) |
| Skripte | [`crates/jfn_sim/scriptsvc.rs`](crates/jfn_sim/scriptsvc.rs), [`cli/scripts.rs`](crates/jfn_sim/cli/scripts.rs), [`src/scripttest.rs`](src/scripttest.rs) | Skripte als Dienste in den Hosts: Host-API, Anweisungen als Rechenzeit, der Befehl `script`, ihr Selbsttest (siehe [Skripte in Hosts](#skripte-in-hosts)) |
| Labore | [`crates/jfn_model/lab.rs`](crates/jfn_model/lab.rs), [`crates/jfn_sim/goal.rs`](crates/jfn_sim/goal.rs), [`src/workspace/task.rs`](src/workspace/task.rs), [`src/workspace/devwin/svcedit.rs`](src/workspace/devwin/svcedit.rs), [`assets/labs`](assets/labs), [`src/tasktest.rs`](src/tasktest.rs) | die Aufgabe eines Labors in der Projektdatei, das Urteil über eine Erwartung (Runner und Labor) und die Ziele über einen Lauf, das Panel „Aufgabe“ mit der Liste der Labore, die Einstellungen der Dienste als JSON, die vier Labore, ihr Selbsttest (siehe [Labore und Aufgaben](#labore-und-aufgaben)) |
| Skript-Editor | [`src/workspace/devwin/scripts.rs`](src/workspace/devwin/scripts.rs), [`src/doc/scripts.rs`](src/doc/scripts.rs), [`src/scriptedtest.rs`](src/scriptedtest.rs) | der Reiter Skripte des Gerätefensters, die Skripte eines Geräts im Projekt mit Rückgängig und Vorlagen, ihr Selbsttest (siehe [Skript-Editor](#skript-editor)) |
| Spanning Tree | [`crates/jfn_sim/stp.rs`](crates/jfn_sim/stp.rs), [`stp98.rs`](crates/jfn_sim/stp98.rs), [`rstp.rs`](crates/jfn_sim/rstp.rs), [`cli/stp.rs`](crates/jfn_sim/cli/stp.rs), [`lib/netpkt/stp.rs`](../../lib/netpkt/stp.rs), [`src/stptest.rs`](src/stptest.rs) | STP (802.1D-1998) und RSTP (802.1D-2004) auf den Switches, Einstellungen und Befehle, BPDUs, ihr Selbsttest (siehe [Spanning Tree](#spanning-tree)) |
| Routing | [`crates/jfn_sim/rip.rs`](crates/jfn_sim/rip.rs), [`cli/rip.rs`](crates/jfn_sim/cli/rip.rs), [`src/riptest.rs`](src/riptest.rs), [`lib/netpkt/rip.rs`](../../lib/netpkt/rip.rs) | RIP als Dienst der Router und L3-Switches, seine Befehle und Anzeigen, sein Selbsttest, die Nachrichten Byte für Byte (siehe [Routing mit RIP](#routing-mit-rip)) |
| NAT und Zugriffslisten | [`crates/jfn_sim/acl.rs`](crates/jfn_sim/acl.rs), [`nat.rs`](crates/jfn_sim/nat.rs), [`cli/acl.rs`](crates/jfn_sim/cli/acl.rs), [`cli/nat.rs`](crates/jfn_sim/cli/nat.rs), [`lib/netpkt/nat.rs`](../../lib/netpkt/nat.rs), [`src/nattest.rs`](src/nattest.rs) | Zugriffslisten und Adressübersetzung der IOS-artigen Geräte, ihre Befehle, das Umschreiben der Pakete mit ihren Prüfsummen (Library), ihr Selbsttest (siehe [NAT und Zugriffslisten](#nat-und-zugriffslisten)) |
| IPv6 | [`crates/jfn_sim/ip6.rs`](crates/jfn_sim/ip6.rs), [`nd.rs`](crates/jfn_sim/nd.rs), [`l3v6.rs`](crates/jfn_sim/l3v6.rs), [`jobs6.rs`](crates/jfn_sim/jobs6.rs), [`trace6.rs`](crates/jfn_sim/trace6.rs), [`cli/ip6.rs`](crates/jfn_sim/cli/ip6.rs), [`cli/linux6.rs`](crates/jfn_sim/cli/linux6.rs), [`cli/print6.rs`](crates/jfn_sim/cli/print6.rs), [`cli/names.rs`](crates/jfn_sim/cli/names.rs), [`lib/netpkt/icmpv6.rs`](../../lib/netpkt/icmpv6.rs), [`lib/netpkt/ipv6.rs`](../../lib/netpkt/ipv6.rs), [`lib/netpkt/addrsel.rs`](../../lib/netpkt/addrsel.rs), [`src/ipv6test.rs`](src/ipv6test.rs), [`src/ipv6tcptest.rs`](src/ipv6tcptest.rs) | der IPv6-Teil der Geräte (Adressen, Neighbor Cache, Router, Routen), Neighbor Discovery, DAD und SLAAC, Weiterleiten und ICMPv6, Ping und Traceroute, die Notizen, die Befehle beider Befehlszeilen, Namen an beiden Befehlszeilen (`dig`, `host`, `nslookup`, `ip host`, `ip name-server`), ICMPv6 und Neighbor Discovery Byte für Byte, die Fragmente und die Adressauswahl nach RFC 6724 (Library), die Selbsttests (siehe [IPv6](#ipv6)) |
| OSPF | [`crates/jfn_sim/ospf.rs`](crates/jfn_sim/ospf.rs), [`cli/ospf.rs`](crates/jfn_sim/cli/ospf.rs), [`src/ospftest.rs`](src/ospftest.rs), [`lib/netpkt/ospf.rs`](../../lib/netpkt/ospf.rs) | OSPF als Dienst der Router und L3-Switches, seine Befehle und Anzeigen, sein Selbsttest, die Pakete und LSAs Byte für Byte (siehe [Routing mit OSPF](#routing-mit-ospf)) |
| Aufgaben mit Bewertung | [`crates/jfn_model/lab.rs`](crates/jfn_model/lab.rs), [`crates/jfn_sim/assess.rs`](crates/jfn_sim/assess.rs), [`src/workspace/assess.rs`](src/workspace/assess.rs), [`src/workspace/author.rs`](src/workspace/author.rs), [`src/doc/assess.rs`](src/doc/assess.rs), [`assets/assess`](assets/assess), [`src/assesstest.rs`](src/assesstest.rs) | das Format einer Aufgabe, das Bewerten ihrer Kriterien, die Prüfung `reaches`, das Panel mit Punkten, Zeit und Abgeben, der Bereich zum Erstellen mit seinen Rückgängig-Schritten, die drei Aufgaben, ihr Selbsttest (siehe [Aufgaben mit Bewertung](#aufgaben-mit-bewertung)) |
| WLAN | [`crates/jfn_model/wlan.rs`](crates/jfn_model/wlan.rs), [`crates/jfn_sim/wlan.rs`](crates/jfn_sim/wlan.rs), [`wlankey.rs`](crates/jfn_sim/wlankey.rs), [`wlantrace.rs`](crates/jfn_sim/wlantrace.rs), [`cli/wlan.rs`](crates/jfn_sim/cli/wlan.rs), [`src/doc/wlan.rs`](src/doc/wlan.rs), [`src/workspace/canvas/wlan.rs`](src/workspace/canvas/wlan.rs), [`src/workspace/inspector/wlan.rs`](src/workspace/inspector/wlan.rs), [`src/wlantest.rs`](src/wlantest.rs), [`lib/netpkt/eapol.rs`](../../lib/netpkt/eapol.rs), [`lib/crypto`](../../lib/crypto/lib.rs) (`sha1.rs`, `keywrap.rs`, `cmac.rs`) | die WLAN-Einstellungen im Projekt, Stationen und Access Points im Simulator mit Funk, Schlüsseln und Notizen, `iw` und `show dot11`, die Einstellungen als Rückgängig-Schritte, Funklinien, Reichweiten und Balken, der Inspektor, der Selbsttest, EAPOL Byte für Byte und die Kryptografie von WPA2 und WPA3 (siehe [WLAN](#wlan)) |

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

### Spanning Tree

Die IOS-artigen Switches (JF-S24P, JF-S24L3, JF-S48X) sprechen Spanning Tree nach IEEE: STP
nach 802.1D (der Algorithmus von 802.1D-1998, [`stp98.rs`](crates/jfn_sim/stp98.rs)) oder RSTP
nach 802.1w, heute 802.1D-2004 Abschnitt 17 ([`rstp.rs`](crates/jfn_sim/rstp.rs)); wie die
Bridge ihn nutzt, steht in [`stp.rs`](crates/jfn_sim/stp.rs). **Ohne Konfiguration ist er
aus:** ein Switch ohne `spanning-tree mode …` ist die lernende Bridge von bisher, sendet keine
BPDUs und behält empfangene für sich; jedes bestehende Projekt läuft deshalb wie vorher
(Golden-Berichte und Selbsttests unverändert). Eingeschaltet wird er je Switch in der
Startkonfiguration oder in der Konsole (`spanning-tree mode rstp` oder `stp`; er startet, wenn
die ganze Startkonfiguration gelesen ist), `no spanning-tree mode` schaltet ihn wieder aus. Ob
neue Projekte ihn von Anfang an bekommen, ist eine spätere Entscheidung.

- **Ein Baum für alle VLANs:** BPDUs gehen ungetaggt über jeden Switch-Port (Access und Trunk)
  an 01:80:c2:00:00:00, die Bridge-ID trägt die System-ID-Erweiterung 0. Herstellereigene
  Protokolle mit einem Baum je VLAN gibt es nicht: `pvst` und `rapid-pvst` nimmt die Konsole
  zum Wiedererkennen an und lässt IEEE-STP bzw. -RSTP mit einem Baum laufen, was `show
  spanning-tree` dazuschreibt; ebenso stellen `spanning-tree vlan LISTE priority …` und die
  Timer je VLAN den einen Baum ein (die laufende Konfiguration schreibt die globale Form).
- **Wahl:** Root-Bridge ist die kleinste Bridge-ID (Priorität in Schritten von 4096, dann MAC),
  Root-Port der Port mit den kleinsten Kosten zur Root (dann Bridge- und Port-ID des Nachbarn,
  dann die eigene Port-ID), Designated-Port je LAN der mit dem besten Angebot; die übrigen
  blockieren (Alternate-Port, oder Backup-Port, wenn der Designated-Port des LANs am selben
  Switch liegt, etwa über einen Hub). Pfadkosten nach 802.1D-2004 Tabelle 17-3 aus der
  Geschwindigkeit der Leitung (100M 200 000, 1G 20 000, 2.5G 8 000, 10G 2 000, 25G 800, 100G
  200), außer `spanning-tree cost` setzt sie.
- **STP:** ein Port geht über Listening und Learning (je Forward Delay, 15 s) nach Forwarding,
  also 30 s; die Root sendet alle 2 s (Hello Time) Configuration BPDUs, die anderen reichen sie
  über ihre Designated-Ports weiter, höchstens eine je Sekunde und Port (Hold Time). Fällt der
  Root-Port selbst aus, wählt der Switch sofort neu (wieder 30 s); fällt eine Leitung anderswo
  aus, wartet ein blockierter Port, bis seine gespeicherte Information abläuft (Max Age 20 s
  minus Message Age), also bis zu 50 s. Topologieänderungen: TCN-BPDUs über die Root-Ports, bis
  der Nachbar bestätigt (TCA), dann setzt die Root für Max Age + Forward Delay (35 s) das
  TC-Flag, und alle Switches altern ihre MAC-Einträge so lange nach Forward Delay.
- **RSTP:** Rollen und Zustände (Discarding, Learning, Forwarding) nach den Zustandsmaschinen
  von 802.1D-2004. Auf Punkt-zu-Punkt-Leitungen schlägt ein Designated-Port vor (Proposal), der
  Nachbar synchronisiert seine Ports und stimmt zu (Agreement), beide leiten nach Mikrosekunden
  weiter; ein Alternate-Port wird beim Ausfall des Root-Ports sofort Root-Port. Eine
  Topologieänderung (ein Port, der kein Edge-Port ist, beginnt weiterzuleiten) geht mit dem
  TC-Flag weiter, und jeder Switch vergisst dabei die MAC-Einträge seiner anderen Ports.
  Gemeinsame LANs (an einem Hub, `spanning-tree link-type shared`) bekommen keine Zustimmung und
  warten auf die Timer (fdWhile, beim ersten Mal Max Age).
- **STP und RSTP zusammen:** ein RSTP-Port, der Configuration BPDUs hört, spricht auf diesem
  Port 802.1D (`Peer(STP)` in der Tabelle); ein STP-Switch verwirft RST-BPDUs, wie es
  802.1D-1998 mit einem unbekannten Typ tut. `clear spanning-tree detected-protocols` versucht
  es wieder mit RSTP.
- **Edge-Ports:** `spanning-tree portfast` macht einen Port zum Edge-Port: er leitet sofort
  weiter und löst keine Topologieänderung aus, bis an ihm eine BPDU ankommt. RSTP findet
  Edge-Ports auch selbst (AutoEdge: ein Designated-Port, der 3 s lang keine BPDU hört).
  `spanning-tree bpduguard enable` schaltet einen Port bei der ersten BPDU ab (err-disabled,
  mit `%SPANTREE-2-BLOCK_BPDUGUARD` und `%PM-4-ERR_DISABLE` in der Konsole und
  „(err-disabled)“ in `show interfaces`), `shutdown` und `no shutdown` holen ihn zurück; BPDU
  Guard wirkt auch ohne Spanning Tree.
- **Im Editor:** die LED eines Ports ist bernstein, solange er nicht weiterleitet (Blocking,
  Listening, Discarding und Learning), der Inspektor nennt ihn „blockiert“; ein VLAN-Interface
  ist erst oben, wenn ein Port seines VLANs weiterleitet. Die Probleme-Liste meldet eine
  Schleife nur, solange nicht jede Bridge darin Spanning Tree spricht, und nennt dann die ohne.
  BPDUs sind im Simulationsmodus Frames wie andere (grau, Filter STP), der Paket-Tab zeigt alle
  Felder und Flags, und jede Entscheidung (BPDU empfangen oder gesendet, verworfen, neue Root,
  Rolle, Zustand, Topologieänderung, gelöschte MAC-Einträge, Edge-Port, Migration, err-disabled)
  steht in der Ereignisliste.
- **Zeit:** beide Protokolle zählen in Sekunden-Ticks ab ihrem Start (die Auflösung von
  802.1D-2004); BPDUs, Leitungen und Einstellungen wirken sofort. Nicht dabei: MSTP, Root
  Guard, Loop Guard, BPDU-Filter und die herstellereigenen Beschleuniger.

Befehle: global `spanning-tree mode stp|rstp|pvst|rapid-pvst`, `spanning-tree priority N`,
`spanning-tree hello-time|forward-time|max-age N` (mit 2 × (Forward Delay − 1) ≥ Max Age ≥
2 × (Hello Time + 1)), dieselben als `spanning-tree vlan LISTE …`; am Switch-Port `spanning-tree
cost N`, `port-priority N`, `portfast`, `bpduguard enable`, `link-type point-to-point|shared`;
`show spanning-tree [detail]`, `show spanning-tree interface IF [detail]`, `show spanning-tree
root`, `show spanning-tree vlan LISTE`, `debug spanning-tree events`, `clear spanning-tree
detected-protocols [interface IF]` ([`cli/stp.rs`](crates/jfn_sim/cli/stp.rs)). Die Tabellen und
Konfigurationszeilen folgen dem Vorbild, die Texte darüber, die Hilfe und die Debug-Zeilen sind
eigene Worte. Für Programme: `Engine::stp_view(dev)` (`TreeView` mit Root, Timern und je Port
Rolle, Zustand, Kosten, Port-ID, Edge, Link-Typ, Designated-Werten, Zählern) und
`Action::Stp { dev, set: StpSet }`; die BPDUs baut und zerlegt
[`netpkt::stp`](../../lib/netpkt/stp.rs).

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
| Schleife aus Switches oder Hubs, solange nicht jede Bridge darin Spanning Tree spricht | Fehler |
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
  Server und Resolver sprechen auch IPv6: der Server nimmt Fragen beider Familien auf einem
  Socket, der Resolver fragt wie getaddrinfo A und AAAA (siehe [IPv6](#ipv6)).
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

TCP folgt RFC 9293 und läuft über IPv4 und IPv6 (siehe [IPv6](#ipv6)) als Sockets im
virtuellen Betriebssystem jedes Geräts ([`tcp.rs`](crates/jfn_sim/tcp.rs)). Die Zahlen kommen
vom System des Geräts: Hosts verhalten sich wie Linux, Router und Switches IOS-artig.

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

### Das Labor im Fenster

- **Metriken** (Bereich unten, Strg+Umschalt+M oder Ansicht › Bereiche;
  [`src/workspace/metrics.rs`](src/workspace/metrics.rs)): links ein Baum der Dienste aller
  Geräte (je Gerät seine Dienste mit Art, Name und Zustand: läuft, gestoppt, aus, ohne Strom),
  rechts der gewählte Dienst mit seiner Statuszeile, KPI-Kacheln über die letzten 5 s (Anfragen
  pro Sekunde, p50, p95, p99, Fehlerquote, Wartende, in Arbeit, die Trefferquote eines Caches,
  Last, Lastanteil, Schlüssel und Daten eines KV-Knotens: `Engine::kpi`, `shard_load`) und die
  Disk der Dienste, die speichern; darunter je Gruppe von Zeitreihen ein Liniendiagramm
  (Durchsatz, die Latenzen p50/p95/p99 zusammen, Fehler, Warteschlange …) über 30 s, 1 min oder
  5 min. Die Reihen heißen deutsch („Anfragen/s“ für requests, „p99“ für p99_us), die Werte
  lesen sich wie im Diagramm (ms, µs, kB). Fehler sind rot unterlegte Bereiche mit ihrem Text;
  ein aktiver reicht bis zum Ende der Ansicht, die Karte beim Überfahren nennt ihn mit seiner
  Zeit. In Echtzeit folgen die Diagramme den neuesten Punkten, im Simulationsmodus enden sie
  beim gezeigten Zeitpunkt, auch in der Vergangenheit, und die Kacheln gelten für die 5 s davor
  (`Engine::kpi_between`). Ein auf der Zeichenfläche gewähltes Gerät zeigt seinen ersten
  Dienst. Gelesen wird nur, wenn sich `series_gen`, der gezeigte Zeitpunkt, der Zeitraum, die
  Fehler oder die Wahl ändern; dazwischen zeichnen die Diagramme nicht neu. Tastatur: Baum
  (Pfeile, links und rechts klappen zu und auf), Zeitraum und jedes Diagramm sind Tab-Stopps;
  im Diagramm wandern die Pfeiltasten über die Punkte, und Screenreader hören Punkt und
  Zusammenfassung.
- **Auf der Zeichenfläche** ([`canvas/lab.rs`](src/workspace/canvas/lab.rs)): Dienst-Pillen
  oben an den Geräten (LB, KV, Cache, Q, Worker, HTTP, DNS, DHCP, Skript; eine je Art, ein
  gestoppter Dienst blass, höchstens drei, dann „+n“), ab 50 % wie die Namen; ab 75 %
  Mini-Sparklines unter Loadbalancern, KV-Knoten, Caches und Queues (die letzten 20 Punkte der
  Anfragen pro Sekunde, bei einer Queue der Wartenden; höchstens 24 in der Ansicht, jede mit
  einem Texteintrag wie „KV2: Anfragen/s 13“). Ein Gerät, das ein Fehler ausgeschaltet hat,
  erscheint wie ohne Strom. Die Pillen gehören zum Backing-Store der Leinwand, der nur neu malt,
  wenn sich Dienste oder Strom ändern; die Sparklines liegen als eigene Zeichenbefehle darüber
  und werden nur mit neuen Punkten neu gelesen, Geräte außerhalb der Ansicht kosten nichts.
- **Fehler einspielen** (Eigenschaften; [`inspector/faults.rs`](src/workspace/inspector/faults.rs),
  [`src/doc/faults.rs`](src/doc/faults.rs)): ein Gerät ausschalten, einen seiner Dienste
  stoppen, seine Disk verlangsamen (mehr ms je Zugriff, ein Faktor der Übertragung); ein Kabel
  ziehen oder verschlechtern (mehr Laufzeit, Verlust, höchstens eine Rate); die gewählten
  Geräte vom Rest des Netzes abtrennen (Partition) – jeweils für eine Dauer in Sekunden oder
  ohne Ende. Darunter die aktiven Fehler mit „Wiederherstellen“ und „Alle wiederherstellen“
  (auch Simulation › Alle Fehler beheben); die Seite des Projekts zeigt alle. Jeder Fehler geht
  über die Fehler-Actions von `jfn_sim` (`Action::Fault`, `Heal`, `HealAll`), wird eine
  Markierung (in den Diagrammen; eine eigene Spur „Fehler“ auf der Zeitleiste der Ereignisse)
  und ist ein Schritt der Rückgängig-Liste: Rückgängig heilt einen eingespielten Fehler,
  Wiederholen spielt ihn erneut ein (mit neuer Nummer und ganzer Dauer), ein rückgängig
  gemachtes Wiederherstellen bringt die Fehler mit der Zeit zurück, die sie noch hatten. So
  gilt weiter „Rückgängig arbeitet auf dem Projektmodell, die Engine folgt über Actions“: Fehler
  gehören nicht zum Projekt, ihr Schritt ändert es nicht, und `Doc` schickt beim Rückgängig und
  Wiederholen die Actions selbst (wie bei übernommenen Konfigurationen); das Dokument bleibt
  dabei so gespeichert oder geändert, wie es war. Ein Schritt findet seinen Fehler an Nummer,
  Beginn und Art wieder, ein frischer Simulator (Simulation zurücksetzen) verwechselt nichts.
- **Englisch:** die Statuszeilen von Loadbalancer, KV-Knoten, Cache, Queue und Worker
  (`status_msg`), die Zeile der Disk (`Disk::msg`) und die Texte der Fehler in Markierungen und
  Ereignissen (`FaultMark::msg`, `Note::FaultStart` / `FaultEnd`) sind Meldungen mit englischer
  Zeile in `engine.tsv`; die Berichte der Szenarien behalten ihren deutschen Text, die
  Golden-Berichte sind unverändert.

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
- RIP ([`cli/rip.rs`](crates/jfn_sim/cli/rip.rs)): `router rip` mit eigenem Modus
  `(config-router)#`, `ip rip send|receive version` an Interfaces, `show ip rip database`, `show
  ip protocols`, `show ip route rip|connected|static`, `debug ip rip`, `clear ip route *` (siehe
  [Routing mit RIP](#routing-mit-rip)).
- Zugriffslisten und NAT ([`cli/acl.rs`](crates/jfn_sim/cli/acl.rs),
  [`cli/nat.rs`](crates/jfn_sim/cli/nat.rs)): `access-list`, `ip access-list
  standard|extended` mit den Modi `(config-std-nacl)#` und `(config-ext-nacl)#`, `ip
  access-group`, `show access-lists`, `ip nat …`, `show ip nat translations|statistics`,
  `clear ip nat translation *`, `debug ip nat` (siehe
  [NAT und Zugriffslisten](#nat-und-zugriffslisten)).
- IPv6 ([`cli/ip6.rs`](crates/jfn_sim/cli/ip6.rs)): `ipv6 unicast-routing`, `ipv6 enable`,
  `ipv6 address … [eui-64|anycast|link-local]`, `ipv6 address autoconfig [default]`, `ipv6
  nd …`, `ipv6 mtu`, `ipv6 route`, `ipv6 neighbor`, `show ipv6 interface [brief|IF]`, `show
  ipv6 route`, `show ipv6 neighbors`, `show ipv6 routers`, `ping ipv6`, `traceroute ipv6`,
  `debug ipv6 icmp|nd`, `clear ipv6 neighbors` (siehe [IPv6](#ipv6)); Namen
  ([`cli/names.rs`](crates/jfn_sim/cli/names.rs)): `ip host`, `ipv6 host`, `ip name-server`
  mit Adressen beider Familien, `ping NAME` und `ping ipv6 NAME`.
- OSPF ([`cli/ospf.rs`](crates/jfn_sim/cli/ospf.rs)): `router ospf N` mit eigenem Modus hinter
  demselben Prompt `(config-router)#`, `ip ospf cost|priority|hello-interval|dead-interval`, `ip
  ospf network point-to-point|broadcast` und `ip ospf N area M` an Interfaces, `show ip ospf
  [neighbor|interface [brief|IF]|database]`, `show ip route ospf`, `debug ip ospf
  adj|events|hello`, `clear ip ospf process` (siehe [Routing mit OSPF](#routing-mit-ospf)).

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
`dig`, `host` und `nslookup` fragen den Namensserver selbst; `curl`, `nc`, `ss`, `ping` und
`traceroute` nehmen `-4` und `-6` (siehe [IPv6](#ipv6)).
Router und Switches zeigen `show tcp brief [all]`, `show tcp statistics` und
`show control-plane host open-ports`. Die Dienste des Labors zeigen `lbctl` (Backends mit
Zustand und Zahlen), `kvctl` (Mitglieder, `ring`, `where SCHLÜSSEL` mit den Replikaten und
ihren Versionen, `get`, `keys`), `cachectl`, `mqctl` und `workerctl`, mit `-s NAME` für einen
bestimmten Dienst; ihre Texte sind eigene Worte. Die Skripte eines Hosts zeigt und steuert
`script` (siehe [Skripte in Hosts](#skripte-in-hosts)).
Spanning Tree stellen `spanning-tree …`, `show spanning-tree`, `debug spanning-tree events`
und `clear spanning-tree detected-protocols` ein und zeigen ihn (siehe
[Spanning Tree](#spanning-tree)).

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

`move GERÄT X Y` setzt ein Gerät an eine andere Stelle der Zeichenfläche (Weltkoordinaten der
linken oberen Ecke), wie ein Verschieben im Fenster; für WLAN zählen Reichweite und Signal neu
(siehe [WLAN](#wlan)).

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

Spanning Tree im Dreieck (`stp_triangle.scn` auf `fixtures/stp_triangle.jfnet`, 802.1D): SW1
wird mit Priorität 4096 Root-Bridge, die Ports zwischen den Switches brauchen 30 s, ein Ping
davor scheitert; nach dem Ausfall des Kabels SW1–SW2 hält sich SW2 für die Root-Bridge, SW3
übernimmt erst, als seine Information von SW2 abläuft (17 s später), und leitet 47 s nach dem
Ausfall weiter. Dasselbe mit RSTP (`rstp_triangle.scn` auf `fixtures/rstp_triangle.jfnet`,
eingegeben als `rapid-pvst`): Proposal und Agreement nach Mikrosekunden, der neue Baum drei
BPDUs nach dem Ausfall; beide mit `debug spanning-tree events` auf SW3 und `show
spanning-tree` zu festen Zeiten.

Ein Skript als Schlüssel-Wert-Speicher (`script_kv.scn` auf `fixtures/script_kv.jfnet`) zeigt
die Rechenzeit von Skripten: SRV1 nimmt über HTTP Werte an und legt sie auf seine Disk, jede
Anfrage rechnet rund 1,1 ms (100 ns je Anweisung). 200 Anfragen pro Sekunde kosten gut ein
Fünftel der CPU und warten kaum; bei 1500 pro Sekunde füllt sich die Warteschlange auf 100,
darüber antwortet der Dienst sofort mit 503, und das 99. Perzentil steigt über 100 ms. Ein
zweites Skript auf MON fragt jede Sekunde mit `http.request` nach den Zahlen; im Bericht stehen
`script`, `script status`, `script output`, `top` und `curl`.

Zwei Szenarien zeigen RIP (siehe [Routing mit RIP](#routing-mit-rip)). `rip_ring.scn` auf
`fixtures/rip_ring.jfnet`: vier Router im Ring mit RIPv2 und einem PC hinter jedem; `debug ip
rip` zeigt den ersten Austausch (Requests, die eigenen Tabellen, ausgelöste Updates mit Split
Horizon), `show ip route` die Metriken und die zwei gleich guten Wege zu R3s LAN, `traceroute`
einen davon; dann wird das Kabel R1–R2 gezogen: R1 und R2 halten die verlorenen Netze im
Holddown (`show ip rip database`), ein Ping endet mit „Destination Net Unreachable“, und nach der
Garbage Collection führt der Weg über R4 und R3 (`traceroute` mit fünf Hops). `rip_versions.scn`
auf `fixtures/rip_versions.jfnet`: ein L3-Switch mit zwei VLANs, der RIPv2 nur auf seinem
gerouteten Uplink spricht (`passive-interface default`), ein Router mit Standardroute, der zu
einem alten Router Version 1 spricht (Broadcasts, Klassennetze ohne Maske), eine schwimmende
statische Route, die einspringt, als die RIP-Route nach 180 s verfällt, und `clear ip route *`.
Das Büro hinter NAT (`nat_pat.scn` auf `fixtures/nat_pat.jfnet`, siehe
[NAT und Zugriffslisten](#nat-und-zugriffslisten)): 24 Erwartungen zu PAT, der
Port-Weiterleitung von außen, einer Zugriffsliste, `traceroute` durch PAT, `debug ip nat` und
den abgelaufenen Übersetzungen nach fünf Minuten.
Drei Szenarien zeigen IPv6 (siehe [IPv6](#ipv6)). `ipv6_slaac.scn` auf
`fixtures/ipv6_slaac.jfnet`: R1 kündigt die Präfixe zweier LANs an, die PCs bilden ihre
Adressen per SLAAC (`ip -6 addr`, `ip -6 route` mit dem Router aus den Advertisements), SRV hat
eine feste; `show ipv6 interface`, `show ipv6 route`, Ping und `traceroute6` ins andere LAN,
`show ipv6 neighbors`, `debug ipv6 nd` und eine Minute später die Einträge in STALE.
`ipv6_routing.scn` auf `fixtures/ipv6_routing.jfnet`: zwei Router mit statischen Routen (R2s
Standardroute zur Link-Local-Adresse von R1), Ping und `traceroute6` über beide, Packet Too
Big an der MTU 1280 vor SRV mit den Fragmenten des Absenders, `ping -M do`, Hop Limit 1, ein
Ziel ohne Route, eine doppelte Adresse (`dadfailed`) und R1 ohne IPv6-Routing.
`ipv6_web.scn` auf `fixtures/ipv6_web.jfnet`: PC1 und PC2 holen sich IPv4 per DHCP (mit
Namensserver und Domäne `lab`) und IPv6 per SLAAC, WEB hinter R1 betreibt Namens- und
Webserver auf beiden Familien; `curl` per Name über IPv6 (A und AAAA, IPv6 zuerst) und per
Adresse über IPv4, `ss` mit den Sockets beider Familien, `dig AAAA` und `host`, `ping` und
`traceroute -6` per Name, `ping -4`, und R1 fragt seinen Namensserver über IPv6 (`ip
name-server`, `ping NAME`, `ping ipv6 NAME`, `ipv6 host`).

Zwei Szenarien zeigen OSPF (siehe [Routing mit OSPF](#routing-mit-ospf)).
`ospf_triangle.scn` auf `fixtures/ospf_triangle.jfnet`: drei Router im Dreieck mit einem
LAN-Kabel (DR und BDR nach 40 s), einer seriellen Leitung (Kosten 48) und einer
Ethernet-Verbindung als Punkt-zu-Punkt-Netz, hinter jedem ein PC; `debug ip ospf adj` zeigt die
Nachbarschaften bis Full, `show ip ospf neighbor`, `interface brief`, `database`, `show ip
route` und `show ip protocols` die Lage nach 60 s mit zwei gleich guten Wegen zur seriellen
Leitung; dann fehlt das LAN-Kabel R1–R2: R1 verliert R2 sofort, SPF rechnet 200 ms später, und
`traceroute` führt über R3 und die serielle Leitung; mit dem Kabel kommt nach einer neuen Wahl
der kurze Weg zurück. `ospf_areas.scn` auf `fixtures/ospf_areas.jfnet`: vier Router an einem
Switch wählen DR und BDR nach Priorität (R3 mit 0 nie), R1 ist ABR zu Bereich 1 und zum
Stub-Bereich 2, R4 ASBR mit einer statischen Route und der Standardroute zu einem Router „im
Internet“; Summaries (O IA), externe Routen (O E2) über die ASBR-Summaries, die Standardroute
des Stub-Bereichs (O*IA), Pings und `traceroute` aus beiden Bereichen, der Ausfall des DR (nach
dem Dead-Intervall übernimmt der BDR, R4 wird BDR) und `area 2 stub no-summary`.

`expect PC1 reaches 10.0.2.10` prüft die Verbindung selbst (A1): Auf einer Kopie des Simulators
pingt PC1 die Adresse (oder einen Namen über seinen Resolver) bis zu fünfmal, das nächste Echo
nach der Antwort oder nach einer Sekunde, bis eine Antwort kommt; `not reaches` verlangt, dass
keine kommt. Der Lauf selbst bleibt unberührt. Der Bericht nennt, wie viele Pings beantwortet
wurden, oder warum keiner („keine Antwort: 10.0.1.1 meldet: Netz nicht erreichbar“, „keine
Route zum Ziel“, „der Name … ist unbekannt“, „das Gerät ist ausgeschaltet“).

Ein Szenario zeigt WLAN (siehe [WLAN](#wlan)). `wlan_office.scn` auf `fixtures/wlan_office.jfnet`:
zwei Access Points mit dem Netz „Buero“ (WPA2-Personal, 5 GHz, Kanal 36 und 44), an AP1 dazu das
offene Gastnetz „Gaeste“ (2,4 GHz, VLAN 20), R1 vergibt die Adressen beider Netze. Laptop1 und
Phone1 sind nach 761 ms angemeldet und bekommen ihre Adressen per DHCP (`iw dev wlan0 link`, `ip
-br addr`, `show dot11 associations`, `iw dev wlan0 scan`), Laptop2 mit falscher Passphrase
scheitert am Handshake (`%DOT11-4-MAXRETRIES` nach 3,861 s) und versucht es nach 10, 20 und 30 s
wieder. Laptop1 pingt SRV1 und geht dabei zu AP2: in der Mitte bleibt er, nahe AP2 wechselt er
(alle 12 Antworten kommen); zu weit weg ist er nach 716,8 ms getrennt (`NO-CARRIER`), zurück
meldet er sich sofort wieder an. AP1 aus: Phone1 verliert das Gastnetz, wieder an: beide sind
761 ms später zurück. Laptop2 gibt nach dem vierten Fehlschlag auf und kommt mit `iw dev wlan0
connect Buero psk …` ins Netz; 22 Erwartungen.

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

### Aufgaben mit Bewertung

Ein Labor kann die Arbeit auch bewerten (A1): mit Kriterien, die Punkte bringen, einem
Zielnetz und wahlweise einer Zeitgrenze. Das Projekt selbst ist das Netz, mit dem Lernende
beginnen; das Ziel, die fertige Lösung, steht mit in der Datei. **Wer die Datei öffnet, kann
die Lösung lesen: Aufgaben mit Bewertung sind zum Üben und für Selbstkontrollen gedacht, nicht
für Prüfungen, die manipulationssicher sein müssen.**

- **Kriterien:** der Name eines Geräts, die Adresse eines Interfaces (IPv4 oder IPv6), sein
  Zustand (eingeschaltet oder `shutdown`), das VLAN und der Modus eines Switchports (access,
  trunk, dynamic, geroutet), eine statische Route, eine Einstellung eines Dienstes (ein Wert
  seines JSON, auch tiefer: `pools.0.range`), ein Kabel zwischen zwei Ports (ein Port, der
  fehlt: irgendeiner) und Zeilen der Running-Config; dazu Verbindungsprüfungen in der Sprache
  der Szenarien (siehe [Szenarien](#szenarien-scn)), etwa `PC1 reaches 10.0.2.10`: ein Ping
  auf einer Kopie des Simulators bekommt eine Antwort. Jedes Kriterium hat Punkte, einen Text
  auf Deutsch und Englisch und einen Vergleich: gleichwertig (Groß- und Kleinschreibung,
  Leerzeichen, Masken statt Präfixlängen, `Gi0/1` statt `GigabitEthernet0/1`, IOS-artige und
  Linux-Schreibweisen von Routen, die Reihenfolge der Einstellungen eines Dienstes), genau
  (derselbe Text) oder einer von mehreren Werten (eine Route oder die Standardroute).
- **Zeilen der Konfiguration** gelten, wenn sie auf dem Gerät nichts ändern würden: Eine Kopie
  des Geräts mit seiner Running-Config bekommt sie eingetippt, wie Lernende tippen (`int
  g0/0/1`, `ip add …`, `no shut`), und ihre Running-Config muss bleiben, wie sie ist. Was
  dazukäme, fehlt („es fehlt: interface GigabitEthernet0/0/0.10 › encapsulation dot1Q 10“), was
  wegfiele, ist zu viel („zu viel: … › shutdown“ für `no shutdown` an einem ausgeschalteten
  Port).
- **Bewertung:** die Punkte der erfüllten Kriterien, ein Prozentsatz (abgerundet), je Kriterium
  erfüllt oder nicht und was es fand. Bewertet wird der Stand jetzt: Ein Kriterium kann wieder
  verloren gehen. Ein Kriterium findet sein Gerät über dessen Nummer, auch wenn es umbenannt
  wurde; Prüfungen nennen Geräte wie das Zielnetz.
- **Rückmeldung:** jedes Kriterium erfüllt oder nicht (nach „Prüfen“, F9, auch was fehlt), nur
  die Punktzahl, oder nichts, bis abgegeben ist. Eine Zeitgrenze läuft ab dem Öffnen auf der
  Uhr des Fensters (Anhalten und Tempo der Simulation ändern sie nicht) und wird nicht
  mitgespeichert. Ist sie um oder wurde abgegeben („Abgeben“, Simulation › Aufgabe abgeben, nach
  der Rückfrage „Jetzt abgeben?“), steht die Bewertung fest, und alle Kriterien zeigen, was sie
  fanden.
- **Das Panel „Aufgabe“** zeigt dann die Punkte („7 von 20 Punkten · 35 %“) mit
  Fortschrittsbalken, die verbleibende Zeit und die Kriterien, so weit die Rückmeldung es
  erlaubt. Es bewertet nach jeder Änderung des Projekts (höchstens alle 0,5 s) und, wenn sich
  im Simulator etwas tat (eine Zeile in einer Konsole, Ereignisse), höchstens alle 2 s; steht
  alles still, bewertet es nicht. Für Screenreader ist jedes Kriterium ein Kontrollkästchen
  (angehakt: erfüllt), die Punkte ein Fortschrittsbalken.
- **Aufgabe erstellen** (Werkzeuge › Aufgabe erstellen …, ein Bereich rechts) in sieben
  Schritten: 1 „Aktuelles Netz als Ziel“: das Netz, wie es läuft, mit den Running-Configs,
  wird die Lösung. 2 Ein Baum seiner Einstellungen (Geräte mit Namen, Interfaces mit Adresse,
  Zustand, VLAN und Modus, statische Routen, Dienste und ihre Einstellungen, die Zeilen der
  Konfiguration, die ein neues Gerät nicht hat, die Kabel): ein Häkchen bewertet eine
  Einstellung, darunter stehen ihre Punkte, der Vergleich (mit den weiteren Werten) und der
  Text in beiden Sprachen (bis zu einem eigenen der des Kriteriums). 3 Verbindungsprüfungen mit
  Punkten (eine, die sich nicht lesen lässt, wird mit dem Grund abgelehnt). 4 Titel, Kurztext
  für die Liste, Einleitung, Thema. 5 Rückmeldung und Zeit in Minuten. 6 „Ausgangsnetz
  erzeugen“ nimmt dem Ziel, was die Kriterien prüfen (Adressen, Routen, VLANs, Modi und Zeilen
  aus den Konfigurationen; ein Interface, das eingeschaltet sein soll, wird ausgeschaltet;
  Einstellungen von Diensten entfallen, Kabel werden entfernt, ein Name wird neutral wie
  `JF-R4-1`), als ein Rückgängig-Schritt; oder das Netz wird von Hand zurückgebaut. Darunter
  steht, wie viele Punkte das Projekt jetzt hätte. 7 „Als Aufgabe speichern …“ legt das Labor
  mit der Aufgabe ins Projekt (ein Rückgängig-Schritt) und fragt nach der Datei. Ein Projekt mit
  Aufgabe füllt den Bereich aus seiner Datei.

| Aufgabe | worum es geht |
|---|---|
| `01-router-zwei-netze.jfnet` | einen neuen Router benennen und seine zwei Interfaces einrichten, bis die PCs einander erreichen; jedes Kriterium zeigt sich sofort (20 Punkte) |
| `02-vlans-trunk.jfnet` | zwei VLANs auf SW1, ein Trunk zu R1 und dort ein Subinterface je VLAN (Router-on-a-stick); nur die Punktzahl, 20 Minuten |
| `03-statische-routen.jfnet` | das fehlende Kabel stecken und die statischen Routen dreier Router setzen (für R1 und R3 genügt eine Standardroute); die Bewertung erst nach dem Abgeben, 15 Minuten |

Die Aufgaben liegen in `assets/assess` (im Programm `app://assess`) und stehen in Datei › Labor
öffnen … nach den Laboren, mit ihren Punkten und dem Zeichen „Mit Punkten“. In der Datei ist
die Aufgabe der Abschnitt `assess` des Labors (siehe [`lab.rs`](crates/jfn_model/lab.rs)):

```json
"assess": {
  "feedback": "score",
  "time_limit_s": 1200,
  "items": [
    {"what": "address", "dev": "R1", "id": 2, "port": "GigabitEthernet0/0/0", "value": "10.0.1.1/24", "points": 3,
     "text": ["R1 Gi0/0/0 hat die Adresse 10.0.1.1/24", "R1 Gi0/0/0 has the address 10.0.1.1/24"]},
    {"what": "route", "dev": "R1", "id": 2, "value": ["10.0.3.0/24 via 10.0.12.2", "default via 10.0.12.2"], "cmp": "any", "points": 3},
    {"what": "config", "dev": "R1", "id": 4, "value": ["interface GigabitEthernet0/0/0.10", " encapsulation dot1Q 10"], "points": 3},
    {"what": "cable", "dev": "R2", "port": "GigabitEthernet0/0/1", "peer": "R3", "peer_port": "GigabitEthernet0/0/0", "points": 2},
    {"what": "check", "checks": ["PC1 reaches 10.0.3.10"], "points": 6}
  ],
  "target": {"devices": [...], "links": [...]}
}
```

`feedback` ist `items`, `score` oder `end`, `time_limit_s` 0 ohne Grenze. `what` ist
`hostname`, `address`, `state`, `vlan`, `mode`, `route`, `service` (mit `svc` und dem Pfad
`key`), `cable` (mit `peer` und `peer_port`; ein `value` verlangt eine Kabelart), `config`
(Zeilen, die eines Blocks eingerückt) oder `check` (`checks`: Erwartungen, die zugleich
gelten); `cmp` ist `norm` (Vorgabe), `exact` oder `any` (dann ist `value` eine Liste). Was auf
seinem Vorgabewert steht, fehlt in der Datei; ohne `points` zählt ein Kriterium 1. Ein Labor
ohne Aufgabe hat `"assess": null` und verhält sich wie bisher; ein Labor mit Aufgabe braucht
keine Ziele, kann aber welche haben.

Für Programme: `jfn_model::lab` (`Assess`, `Item`, `ItemKind`, `Compare`, `Feedback`, `Target`,
`canon`, `RouteVal`, `Assess::issues`), `jfn_sim::assess` (`Grader::new(&aufgabe)`,
`grade(&projekt, &engine, ausgabe)` -> `Score` mit `points`, `total`, `percent()` und je
Kriterium `Graded` mit `held` und `found`; `would_change`, `normalise`; zum Erstellen
`capture`, `candidates`, `start_network`, `item_text`, `item_key`) und `jfn_sim::goal::probe`
(die Erwartung `reaches`). Geprüft wird das in der Selbsttest-Gruppe `assess`
([`src/assesstest.rs`](src/assesstest.rs)) und in den Fensterskripten `assess_author.uis`,
`assess_solve.uis`, `assess_en.uis` und `assess_shots.uis` (siehe
[Bauen und testen](#bauen-und-testen)).

Noch offen: Kriterien für ACLs, NAT, OSPF und RIP über ihre Zeilen hinaus (heute über
Zeilen der Konfiguration und Prüfungen), Teilpunkte, Zufallswerte je Lernendem und eine
verschlüsselte Lösung.

### Routing mit RIP

Router, L3-Switches und die Firewall sprechen RIP ([`rip.rs`](crates/jfn_sim/rip.rs)): Version 2
nach RFC 2453, Version 1 nach RFC 1058 auf Interfaces, die so eingestellt sind. RIP ist ein
Dienst des Geräts („rip“; im Gerätefenster unter Dienste mit Statuszeile und Zahlen: Routen,
gelernte, Nachbarn, Updates, ausgelöste, Timeouts …), den die Befehlszeile einrichtet. Er läuft
auf virtueller Zeit mit den Zufallszahlen des Geräts, gleiche Projekte ergeben gleiche Läufe.

- **Einrichten** ([`cli/rip.rs`](crates/jfn_sim/cli/rip.rs)): `router rip` öffnet den Modus
  `(config-router)#` mit `version 1|2`, `network A.B.C.D` (gespeichert als Klasse-A/B/C-Netz;
  RIP läuft auf jedem Interface, dessen primäre Adresse darin liegt), `passive-interface
  IF|default` (das Interface hört zu, sendet aber nichts; mit `default` alle außer denen mit `no
  passive-interface`), `[no] auto-summary`, `default-information originate`, `redistribute
  connected|static [metric N]` (statische Routen erst, wenn sie im RIB gewinnen), `timers basic`,
  `distance`, `maximum-paths`; an Interfaces `ip rip send|receive version 1|2|1 2`. Ein
  L3-Switch braucht `ip routing` (sonst `% IP routing not enabled`), `no ip routing` und `no
  router rip` nehmen den Prozess mit (die Einstellungen der Interfaces bleiben). Alles steht in
  der Running-Config und kommt beim Start durch dieselbe Befehlszeile zurück.
- **Versionen:** ohne `version` sendet RIP Version 2 und nimmt 1 und 2; `version 2` nimmt nur 2,
  `version 1` sendet und nimmt nur 1. Version 2 geht an 224.0.0.9 (die Interfaces treten der
  Gruppe bei, andere Geräte verwerfen die Pakete mit einer Notiz), Version 1 an
  255.255.255.255, beide mit TTL 1 vom Port 520 und der primären Adresse des Interfaces. Version
  1 trägt keine Masken: Subnetze des Hauptnetzes des Interfaces gehen nur mit dessen Maske
  hinaus, andere Hauptnetze als Klassennetz; der Empfänger nimmt die Maske seines Interfaces im
  selben Hauptnetz, sonst die der Klasse, und Host-Routen, wo Host-Bits gesetzt sind. Version 2
  fasst nur mit `auto-summary` an Klassengrenzen zusammen (aus wie bei IOS 15). Eine
  Version-1-Anfrage beantwortet ein Interface, das nur Version 2 sendet, nicht (RFC 2453 4.6).
- **Metriken nach RFC 2453:** Ein angeschlossenes Netz kostet 1, jede empfangene Metrik bekommt
  1 dazu, 16 ist unerreichbar. Ein Router sieht das LAN seines Nachbarn also als `[120/2]` (wie
  FRR; IOS zählt die 1 beim Senden dazu und zeigt dort 1 – auf der Leitung stehen dieselben
  Zahlen, nur eine mit 15 empfangene Route ist hier unerreichbar). Im RIB haben RIP-Routen die
  Distanz 120 (`distance`): eine statische Route (1) gewinnt, eine schwimmende (z. B. 200)
  springt ein, wenn RIP die Route verliert.
- **Updates:** Beim Start und wenn ein Interface zu senden beginnt (es kommt hoch, ist nicht mehr
  passiv, sendet andere Versionen), fragt RIP dort nach der ganzen Tabelle und schickt die
  eigene. Danach alle 30 s, jedes Mal mit einem zufälligen Versatz von bis zu einem Sechstel
  (RFC: ±5 s). Änderungen gehen sofort als ausgelöstes Update hinaus, nur die geänderten Routen;
  danach wartet RIP zufällig 1 bis 5 s, was in der Zeit geändert wird, geht gesammelt, und ein
  fälliges reguläres Update ersetzt es (RFC 2453 3.10.1). Höchstens 25 Einträge je Nachricht.
  Split Horizon mit Poisoned Reverse: Was über ein Interface gelernt wurde, geht dort mit Metrik
  16 zurück; die eigenen Netze eines Interfaces meldet RIP dort gar nicht. Antworten auf einen
  Request gehen an den Fragenden (Unicast, das IOS-artige ARP verliert die erste).
- **Timer** (`timers basic 30 180 180 300`): Eine Route, die 180 s lang niemand bestätigt,
  verfällt; sie wird mit Metrik 16 gemeldet, bis 300 s nach ihrem letzten Update, dann gelöscht –
  das sind die 120 s Garbage Collection des RFC. Meldet der nächste Hop 16 oder fällt das
  Interface aus, beginnt das sofort. Der **Holddown** ist das, was IOS-artige Geräte dazu tun
  (RFC 2453 kennt ihn nicht): 180 s lang nach dem Verlust nimmt RIP von anderen Routern nur
  Angebote mit besserer Metrik als zuvor an, der Router, von dem die Route kam, darf sie
  zurückbringen, und eine gelöschte Route ist wieder neu. In einem Ring kommt eine verlorene
  Route deshalb erst nach der Garbage Collection über die andere Seite zurück; mit einer 0 an
  der Stelle des Holddowns gilt nur das RFC. Ein Router, der so lange schweigt wie die
  Flush-Zeit oder hinter einem ausgefallenen Interface liegt, ist keine Quelle mehr.
- **Gleich gute Wege:** Bis zu `maximum-paths` (4) Router mit derselben besten Metrik geben je
  eine Route ins RIB; weitergeleitet wird je Zieladresse über einen davon, immer denselben
  ([`ip.rs`](crates/jfn_sim/ip.rs): `ecmp_pick`). Fällt einer weg, bleiben die anderen, ohne
  Holddown.
- **Eingang** (RFC 2453 3.9): Antworten vom Port 520, von einem Nachbarn in einem Netz des
  Interfaces, nicht von einer eigenen Adresse; Version 1 mit gesetzten Null-Feldern,
  authentifizierte Nachrichten (Authentifizierung ist nicht einstellbar, RFC 2453 5.2),
  Nachrichten auf Interfaces ohne RIP und Einträge mit falscher Metrik oder Adresse werden mit
  ihrem Grund ignoriert. Passive Interfaces beantworten keine Requests.
- **Anzeigen:** `show ip route [rip|connected|static]` mit dem Code R, `[120/Metrik]`, dem Alter
  des letzten Updates und dem Interface (die Legende nennt R nur auf Geräten mit RIP), `show ip
  rip database` (eigene Netze, gelernte Wege, Unerreichbares mit Holddown und Löschzeit,
  Zusammenfassungen), `show ip protocols` (Timer, Versionen je Interface, Netze, passive
  Interfaces, Quellen mit dem Alter ihres letzten Updates, Distanz), `debug ip rip` (jedes
  Update mit seinen Routen und jede Entscheidung, in eigenen Worten), `clear ip route *` (vergisst
  die gelernten Routen und fragt die Nachbarn neu).
- **Ereignisse:** Jede Entscheidung – Request, Update gesendet und empfangen samt der per Split
  Horizon vergifteten Routen, Route gelernt, geändert, ein weiterer gleich guter Weg, verloren,
  im Holddown abgewiesen, gelöscht, eigene Netze, ignorierte Nachrichten mit ihrem Grund – steht
  mit deutschem Text in der Ereignisliste und im Paket-Tab. Der Simulationsmodus zeigt
  RIP-Rahmen braun (Filter „OSPF/RIP“), der Paket-Tab zerlegt sie mit jedem Eintrag
  ([`netpkt::rip`](../../lib/netpkt/rip.rs), [`decode.rs`](../../lib/netpkt/decode.rs)).

Für Programme: `Action::Rip { dev, set: RipSet }` (`Process`, `Version`, `Network`, `Passive`,
`AutoSummary`, `DefaultOriginate`, `Redistribute`, `Timers`, `Distance`, `MaxPaths`,
`IfVersions`, `Clear`) und `Engine::rip(dev)` mit `cfg`, `table()`, `ifaces()`, `sources()`,
`next_update()` und `age()`. Geprüft wird RIP in der Selbsttest-Gruppe `rip`
([`src/riptest.rs`](src/riptest.rs)) und in den Szenarien `rip_ring.scn` und
`rip_versions.scn` (siehe [Szenarien](#szenarien-scn)).

### NAT und Zugriffslisten

IOS-artige Geräte, die routen (Router, L3-Switches, die Firewall JF-FW), übersetzen Adressen
(NAT, RFC 3022) und filtern mit Zugriffslisten ([`acl.rs`](crates/jfn_sim/acl.rs),
[`nat.rs`](crates/jfn_sim/nat.rs); die Befehle in [`cli/acl.rs`](crates/jfn_sim/cli/acl.rs)
und [`cli/nat.rs`](crates/jfn_sim/cli/nat.rs)). Zugriffslisten kennt jedes IOS-artige Gerät,
ein L2-Switch an seinem VLAN-Interface.

- **Zugriffslisten:** nummeriert (`access-list` 1–99 und 1300–1999 Standard, 100–199 und
  2000–2699 erweitert) oder benannt (`ip access-list standard|extended NAME` mit den Modi
  `(config-std-nacl)#` und `(config-ext-nacl)#`, auch für Nummern), mit Zeilennummern (ohne
  eine: 10 nach der letzten), `remark`, `no` einer Zeile über ihre Nummer oder ihren Wortlaut
  und `ip access-list resequence`; `no access-list N …` löscht wie beim Vorbild die ganze Liste.
  Eine Regel nennt Protokoll (`ip`, `tcp`, `udp`, `icmp` mit Typ und Code oder Namen wie
  `echo-reply`, `gre`, `esp`, eine Nummer …), Quelle und Ziel (`any`, `host A`, Adresse mit
  Wildcard, auch nicht zusammenhängend), Ports (`eq`, `neq`, `gt`, `lt`, `range`, mit den
  Namen des Vorbilds wie `www` oder `domain`), `established` (TCP mit ACK oder RST) und `log`.
  `ip access-group N|NAME in|out` gilt an Ports, Subinterfaces und VLAN-Interfaces.
- **Wann eine Liste greift:** ein ankommendes Paket trifft zuerst die eingehende Liste seines
  Interfaces, auch eines an das Gerät selbst; ein weitergeleitetes trifft nach Routing und NAT
  die ausgehende Liste des Interfaces, über das es geht. Was das Gerät selbst sendet, filtert
  keine ausgehende Liste. Die erste passende Zeile entscheidet, sonst das implizite deny am
  Ende; eine Liste ohne Regel oder eine, die es nicht gibt, lässt alles durch. Ein verworfenes
  Paket bekommt „administratively prohibited“ (ICMP 3/13) vom Interface, an dem es ankam, mit
  der Grenze der anderen Unreachables (eines alle 500 ms): Linux-`ping` zeigt „Packet
  filtered“, IOS-`ping` ein `U`, IOS-`traceroute` `!A`. Spätere Fragmente tragen keine Ports:
  eine Regel mit Ports lässt sie durch, wenn sie erlaubt, und wird übersprungen, wenn sie
  verbietet. Die Treffer zählen `show access-lists` und `show ip access-lists [N|NAME]`;
  `clear access-list counters [N|NAME]` und das Ausschalten setzen sie zurück.
- **NAT:** `ip nat inside|outside` an Interfaces; `ip nat pool NAME ANFANG ENDE netmask M` oder
  `prefix-length P`; dynamisch `ip nat inside source list L pool P [overload]` oder `… interface
  I overload` (PAT); statisch `ip nat inside source static [tcp|udp] LOKAL [PORT]
  GLOBAL|interface I [PORT]` für Adressen und Port-Weiterleitungen; `ip nat translation
  timeout`, `udp-timeout`, `dns-timeout`, `tcp-timeout`, `finrst-timeout`, `syn-timeout` und
  `icmp-timeout` mit Sekunden oder `never`. Ein Outside-Interface beantwortet ARP für die
  globalen Adressen von Pools und statischen Übersetzungen in seinem Netz.
- **Die Reihenfolge** (wie beim Vorbild): die eingehende Liste; auf einem Outside-Interface dann
  die Rückübersetzung des Ziels vor dem Routing, so bestimmt die innere Adresse den Weg (die TTL
  zählt für das Paket, wie es ankam); das Routing; für Pakete von innen nach außen die
  Übersetzung der Quelle; zuletzt die ausgehende Liste, die also die übersetzten Adressen sieht.
  ICMP-Antworten des Routers zitieren das Paket, wie es ankam.
- **Verhalten nach RFC 4787, 5382 und 5508:** eine Zuordnung gehört zum inneren Endpunkt
  (Adresse und Port oder ICMP-Kennung) und gilt für jedes Ziel. Der globale Port behält die
  Nummer des inneren, wenn sie frei ist, sonst den nächsten freien mit gleicher Parität im
  selben Bereich (1–1023, 1024–65535); ICMP-Kennungen ebenso, eigene Ports des Routers sind
  belegt. Ein Pool mit `overload` gibt einem Host seine Adresse wieder, einer ohne gibt jedem
  Host eine eigene. Von außen nehmen statische Übersetzungen und Adressen ohne Ports Pakete von
  jedem, PAT nur von Adressen, an die der Host gesendet hat; eine ICMP-Abfrage nimmt nur ihre
  Antworten. Jeder Fluss hat seine eigene Zeit: UDP 300 s (DNS 60 s), ICMP 60 s, TCP 24 h nach
  dem Handshake, vorher und nach RST oder FIN von beiden Seiten 240 s; eine Adresse ohne Ports
  bleibt 24 h nach ihrem letzten Paket, eine Zuordnung mit Ports so lange wie ihre Flüsse.
  Findet eine neue Übersetzung keine Adresse oder keinen Port, bekommt der Absender
  „administratively prohibited“. ICMP-Fehler über übersetzte Pakete kommen mit
  zurückübersetztem Zitat an, aus dem Inneren mit globaler Quelle; einer, zu dem NAT nichts
  kennt, wird verworfen, und keiner frischt eine Übersetzung auf. Das erste Fragment
  entscheidet für sein Datagramm, spätere, die vor ihm kommen, warten bis zu 30 s und folgen
  ihm in ihrer Reihenfolge. Die Prüfsummen von IPv4, TCP, UDP und ICMP (auch die des Zitats)
  werden nach RFC 1624 nachgeführt ([`netpkt::nat`](../../lib/netpkt/nat.rs)): Mitschnitte
  öffnen ohne Prüfsummenfehler, und der Paket-Tab zeigt übersetzte Frames wie alle anderen.
- **Anzeigen:** `show ip nat translations [verbose]` (Protokoll, Inside global, Inside local,
  Outside local, Outside global; `verbose` mit Alter, Ablauf und Art), `show ip nat
  statistics`, `clear ip nat translation *`, `clear ip nat statistics` und `debug ip nat`. Jede
  Entscheidung ist eine Notiz mit deutschem Text (englisch in `engine.tsv`): Liste und Zeile,
  die erlauben oder verwerfen, neue, übersetzte und abgelaufene Übersetzungen, Pakete ohne
  Übersetzung oder Regel, ein erschöpfter Pool, wartende und verworfene Fragmente.
- **Für Programme:** `Action::Acl`, `AccessGroup`, `AclCounters` und `Nat` (auch die Befehle
  gehen über sie), `Engine::access_lists`, `access_groups`, `nat` und `nat_translations`.

Noch offen: Hairpinning (ein innerer Host erreicht einen anderen über dessen globale Adresse;
RFC 4787 verlangt es, das klassische Vorbild kann es nicht), `ip nat outside source`,
Route-Maps, Helfer für Protokolle mit Adressen im Inhalt (FTP …), Zonen und
zustandsbehaftete Prüfung der Firewall, NAT und Filter auf Linux-Geräten (iptables, der
Heimrouter).

Geprüft wird das in der Selbsttest-Gruppe `nat` ([`src/nattest.rs`](src/nattest.rs)) und im
Golden-Szenario `nat_pat.scn` auf `fixtures/nat_pat.jfnet`: ein Büro hinter R1 mit PAT auf
203.0.113.1, ein Server „im Internet“ hinter ISP, der das Büronetz nicht kennt, die
Port-Weiterleitung von 203.0.113.1:80 auf den Webserver im Büro und eine Zugriffsliste, die PC2
nicht ins Web lässt; Pings, `curl` von innen (SRV sieht 203.0.113.1) und von außen (WEB sieht
EXT), `traceroute` durch PAT, `debug ip nat`, die Tabellen und nach fünf Minuten die
abgelaufenen Übersetzungen.

### IPv6

Geräte sprechen IPv6 auf denselben Interfaces wie IPv4 nach RFC 8200, 4443, 4861, 4862, 4291
und 8201, UDP, TCP und Namen darüber nach RFC 768, 9293, 3596 und 6724
([`ip6.rs`](crates/jfn_sim/ip6.rs), [`nd.rs`](crates/jfn_sim/nd.rs),
[`l3v6.rs`](crates/jfn_sim/l3v6.rs), [`jobs6.rs`](crates/jfn_sim/jobs6.rs), für UDP, TCP und
Namen [`os.rs`](crates/jfn_sim/os.rs), [`tcp.rs`](crates/jfn_sim/tcp.rs) und
[`resolver.rs`](crates/jfn_sim/resolver.rs); die Befehle in
[`cli/ip6.rs`](crates/jfn_sim/cli/ip6.rs), [`cli/linux6.rs`](crates/jfn_sim/cli/linux6.rs) und
[`cli/names.rs`](crates/jfn_sim/cli/names.rs); die Pakete in
[`netpkt::icmpv6`](../../lib/netpkt/icmpv6.rs) und [`netpkt::ipv6`](../../lib/netpkt/ipv6.rs)).

- **Voreinstellung:** IPv6 ist aus, bis ein Gerät dafür eingerichtet wird, damit bestehende
  Projekte genau so weiterlaufen (kein IPv6-Rahmen, keine Notiz, keine andere Zeile im
  Bericht). IOS-artige Geräte schalten es je Interface ein (`ipv6 enable`, eine `ipv6 address`
  oder `ipv6 address autoconfig`), Linux-Geräte mit `sysctl -w
  net.ipv6.conf.all.disable_ipv6=0` (alle Interfaces, auch später angelegte) oder
  `net.ipv6.conf.IF.disable_ipv6=0`; zurück auf 1 nimmt dem Interface wie beim Kernel jede
  IPv6-Adresse. Weiterleiten erst mit `ipv6 unicast-routing` bzw.
  `net.ipv6.conf.all.forwarding=1`.
- **Adressen:** jedes Interface bekommt seine Link-Local-Adresse aus der MAC (EUI-64, RFC 4291
  Anhang A; IOS-artig auch `ipv6 address FE80::1 link-local`), dazu feste Adressen (`ipv6
  address 2001:DB8:1::1/64`, `… eui-64`, `… anycast`, `ip -6 addr add`) und auf Hosts per SLAAC
  eine aus jedem autonomen /64-Präfix der Router (Linux, solange es nicht weiterleitet;
  IOS-artig mit `ipv6 address autoconfig [default]`). Jede Adresse ist zuerst vorläufig: die
  Duplikaterkennung (DAD, RFC 4862) wartet zufällig bis 1 s, fragt einmal und gilt nach 1 s
  ohne Antwort; hat ein anderes Gerät sie, bleibt sie unbenutzt (`dadfailed`, `[DUP]`,
  `%IPV6_ND-4-DUPLICATE`, ein Fehler in der Probleme-Liste). SLAAC-Adressen haben
  Lebensdauern: veraltet nach der Preferred, entfernt nach der Valid Lifetime, mit der
  Zwei-Stunden-Regel von RFC 4862 5.5.3 (e). Adressen stehen nach RFC 5952 da
  (`2001:db8:1:0:4a:ff:fe00:101`: eine einzelne Null wird nicht zu `::`), IOS-artig in
  Großbuchstaben.
- **Neighbor Discovery (RFC 4861):** der Neighbor Cache mit INCOMPLETE (bis drei Neighbor
  Solicitations an die Solicited-Node-Gruppe, eine je Sekunde; Pakete warten, unter Linux bis
  101, IOS-artig eines; danach „Address unreachable“), REACHABLE (die zufällige ReachableTime
  von 15 bis 45 s), STALE, DELAY (5 s) und PROBE (drei direkte Solicitations, dann ist der
  Eintrag weg); feste Einträge mit `ipv6 neighbor` und `ip -6 neigh add`. Router schicken
  Router Advertisements (die ersten drei höchstens 16 s auseinander, dann alle 150 bis 200 s;
  auf Anfrage nach 0 bis 500 ms, nie früher als 3 s nach dem letzten) mit Hop Limit,
  Lebensdauer 1800 s, MTU und dem Präfix jeder globalen Adresse (30 Tage gültig, 7 bevorzugt);
  dazu `ipv6 nd ra suppress [all]`, `ra interval`, `ra lifetime`, `ra mtu suppress`, `prefix
  …`, `dad attempts`, `reachable-time` und `ns-interval`. Hosts fragen bis zu dreimal im
  Abstand von 4 s, sobald ihre Link-Local-Adresse gilt, und führen Standard-Router und
  On-Link-Präfixe mit Ablauf. Schaltet ein Router das Routing ab, sagt er es mit einem
  letzten Advertisement (Lebensdauer 0). Linux-Geräte, die weiterleiten, schicken keine
  Advertisements (dafür bräuchte es radvd). Der Zufall (DAD-Pausen, ReachableTime, Abstände
  der Advertisements) kommt aus einem eigenen Strom je Gerät, in ganzen Millisekunden.
- **Weiterleiten:** die Routing-Tabelle aus angeschlossenen Netzen, eigenen Adressen,
  statischen Routen (`ipv6 route PRÄFIX [IF] [NEXT-HOP] [DISTANZ]`, auch zu einer
  Link-Local-Adresse mit ihrem Interface; `ip -6 route add`) und bei Hosts den Routern und
  Präfixen der Advertisements; das längste Präfix, dann die Distanz, dann die Metrik. Das Hop
  Limit sinkt je Router (Time Exceeded bei 1). Router fragmentieren nie: zu große Pakete
  beantworten sie mit Packet Too Big und der MTU des nächsten Links (`ipv6 mtu`), der Absender
  merkt sich die Path MTU 10 Minuten (RFC 8201) und teilt seine Pakete selbst mit Fragment
  Header; zusammengesetzt wird 60 s lang. Link-Local-Adressen verlassen ihren Link nicht.
- **ICMPv6 (RFC 4443):** Echo (auch an Gruppen), Destination Unreachable (keine Route, Address
  unreachable, Port unreachable, beyond scope of source address), Packet Too Big, Time
  Exceeded und Parameter Problem; nie über Fehler oder Redirects, über Pakete an Gruppen nur
  Packet Too Big und Parameter Problem zu Optionen; das Zitat füllt höchstens 1280 Byte.
  Begrenzt wie beim Vorbild: IOS-artig ein Eimer von 10, alle 100 ms einer mehr; Linux 50 auf
  einmal und 1000 je Sekunde, je Ziel einer pro Sekunde, geteilt nach der Präfixlänge der
  Route zurück (hinter einem /64 alle 250 ms, sechs auf Vorrat), Packet Too Big ohne Grenze.
- **Anzeigen:** jede Entscheidung ist eine Notiz mit deutschem Text (englisch in
  `engine.tsv`): DAD, gebildete, veraltete und abgelaufene Adressen, Solicitations und
  Advertisements, jeder Zustandswechsel im Neighbor Cache, Router und Präfixe, Weiterleiten
  und warum ein Paket verworfen wurde, Fragmente und Path MTU, ICMPv6 mit seiner Grenze.
  Simulationsmodus und Paket-Tab zerlegen IPv6 mit Erweiterungsköpfen und ICMPv6 mit jedem Feld
  und jeder Option (ICMPv6 grün im Filter ICMP), Mitschnitte öffnen ohne Prüfsummenfehler. Bei
  Geräten mit IPv6 haben die Interface-Tabellen im Inspektor und im Gerätefenster eine Spalte
  IPv6, das gewählte Interface listet seine Adressen mit Herkunft und Zustand.
- **UDP und TCP:** dieselben Sockets für beide Familien, wie Linux mit `bindv6only = 0`: ein
  Socket auf `::` (`os::ANY`) nimmt IPv4 und IPv6, einer auf `0.0.0.0` (`ANY4`) nur IPv4, einer
  auf einer Adresse nur diese; ein Partner über IPv4 bleibt eine IPv4-Adresse (keine
  `::ffff:`-Adressen). `::` belegt den Port für beide Familien: eine Adresse darunter oder
  `0.0.0.0` bekommt ihn nicht mehr, eine Adresse der anderen Familie neben einer Adresse
  schon. DNS, HTTP, TCP-Echo und -Discard, die Skripte (`net.listen`, `net.udp`,
  `http.serve`), Loadbalancer und Queue öffnen ihre Sockets auf `::`; `ss` zeigt sie auf
  Geräten ohne IPv6 als `0.0.0.0` wie zuvor, sonst als `*`. TCP über IPv6: die Prüfsumme über
  den Pseudo-Header von RFC 8200 8.1, die MSS aus der MTU des Pfads weniger 60 (1440 auf
  Ethernet, ohne Option 1220, IOS-artig 1220 außerhalb der eigenen Netze), Linux' Fenster
  64800; Packet Too Big senkt Path MTU und MSS (nie unter 1220) und schickt das Verlorene
  sofort neu. ICMPv6-Fehler erreichen die Sockets wie die von IPv4 (RFC 4443 3; wie Linux
  nur die harten): Port unreachable einen verbundenen UDP-Socket (ECONNREFUSED, auch über
  IPv4), jeder Fehler einen Verbindungsaufbau. Flüchtige Ports und ISNs kommen aus den
  Zufallszahlen des Geräts.
- **Namen:** der Resolver arbeitet wie getaddrinfo mit AI_ADDRCONFIG: ein Gerät mit IPv4 und
  IPv6 fragt A und AAAA zugleich (ein Socket, A zuerst, je eigene Kennung), eines ohne IPv6
  nur A wie zuvor, `-4`/`-6` je eine Familie. Die Adressen ordnet er nach einer Teilmenge von
  RFC 6724 ([`netpkt::addrsel`](../../lib/netpkt/addrsel.rs): unbrauchbare Ziele zuletzt,
  passender Scope, keine veraltete Quelle, passendes Label, höhere Präzedenz, kleinerer Scope,
  längstes gemeinsames Präfix zwischen IPv6-Zielen, sonst die Reihenfolge der Antwort; eine
  Antwort nur mit A bleibt wie sie kam): ein Host mit globaler IPv6-Adresse probiert IPv6
  zuerst, einer nur mit Link-Local-Adresse IPv4. Namensserver dürfen IPv6-Adressen haben.
  `curl`, `nc`, der HTTP-Client und der Lastgenerator nehmen die nächste Adresse, wenn die
  erste nicht verbindet (nur wenn beide Familien kamen); URLs mit `[…]` gehen überall, auch in
  Skripten und als Backends von Cache und Worker; der Loadbalancer nimmt Backends mit
  IPv6-Adressen.
- **Befehle:** `curl -4|-6`, `nc -4|-6`, `ss -4|-6` (IPv6 in Klammern, `*` für `::`), `ping
  NAME` und `traceroute NAME` mit AAAA (eine IPv6-Adresse zuerst macht daraus ein Ping über
  IPv6), `ping -6 NAME`, `traceroute -6 NAME`, `dig [@SERVER] [TYP] NAME [+short]`, `host [-t
  TYP] NAME [SERVER]` und `nslookup NAME [SERVER]` (Konsolenprogramme mit eigenen Fragen an
  den Namensserver, die Ausgaben wie BIND 9.18, die erste Zeile von `dig` in eigenen Worten);
  IOS-artig `ip host NAME A.B.C.D`, `ipv6 host NAME X:X::X`, `ip name-server A [B [C]]` (beide
  Familien, in der Running-Config), `ping NAME` (IPv4; IPv6, wenn der Name keine IPv4-Adresse
  hat), `ping ipv6 NAME`, `show tcp brief` und `show control-plane host open-ports` mit
  IPv6-Verbindungen in Großbuchstaben.
- **Anzeigen von UDP und TCP:** Segmente und Datagramme über IPv6 stehen wie die über IPv4 im
  Simulationsmodus, im Paket-Tab und im Mitschnitt, ihre Prüfsummen von netpkt geprüft; ihre
  Notizen zählen im Filter zu IPv6 statt zu IPv4.
- **Für Programme:** `Action::Ip6` mit `Ip6Set` (auch die Befehle gehen darüber),
  `Engine::ip6`, `ip6_routes`, `ip6_neighbors`, `addrs6`, `ping6`, `traceroute6` und
  `IfView::addrs6`.

Noch offen: MLD (Switches fluten Multicast ohnehin), Redirects (der Codec kennt sie, kein
Gerät sendet oder nimmt sie), DHCPv6 und RDNSS (Linux-Hosts bekommen Namensserver nur über
DHCP für IPv4), Privacy Extensions, Router Advertisements auf seriellen Leitungen, RIPng und
OSPFv3; bei UDP und TCP `IPV6_V6ONLY` (ein Socket auf `::` nimmt immer beide Familien),
`::ffff:`-Adressen, Happy Eyeballs (RFC 8305: die Werkzeuge probieren die Adressen der Reihe
nach), die KV-Knoten des Labors, DHCP, NTP, Syslog und RIP (nur IPv4).

Geprüft wird IPv6 in den Selbsttest-Gruppen `ipv6` ([`src/ipv6test.rs`](src/ipv6test.rs)) und
`ipv6tcp` ([`src/ipv6tcptest.rs`](src/ipv6tcptest.rs): UDP, TCP, Namen, Dienste und
Werkzeuge), in den Golden-Szenarien `ipv6_slaac.scn`, `ipv6_routing.scn` und `ipv6_web.scn`
(siehe [Szenarien](#szenarien-scn)), im Fensterskript `ipv6_inspector.uis` und mit den
coretest-Gruppen `icmpv6` (die Nachrichten Byte für Byte) und `addrsel` (die Beispiele von
RFC 6724 10.2).

### Routing mit OSPF

Router, L3-Switches und die Firewall sprechen OSPF Version 2 nach RFC 2328
([`ospf.rs`](crates/jfn_sim/ospf.rs)). OSPF ist ein Dienst des Geräts („ospf“; im Gerätefenster
unter Dienste mit Statuszeile und Zahlen: Nachbarn, davon Full, LSAs, Routen, SPF-Läufe, Hellos,
Updates, Wiederholungen …), den die Befehlszeile einrichtet. Er läuft auf virtueller Zeit mit den
Zufallszahlen des Geräts (die Sequenznummern der Database Description), gleiche Projekte ergeben
gleiche Läufe. Die Pakete gehen als IP-Protokoll 89 mit TTL 1 und Präzedenz 6 an 224.0.0.5
(AllSPFRouters) oder 224.0.0.6 (AllDRouters), was einem Nachbarn auf einem Broadcast-Netz allein
gilt, an seine Adresse.

- **Einrichten** ([`cli/ospf.rs`](crates/jfn_sim/cli/ospf.rs)): `router ospf N` öffnet einen
  eigenen Modus hinter demselben Prompt `(config-router)#` wie RIP mit `router-id`, `network
  ADRESSE WILDCARD area BEREICH` (die genaueste Anweisung gewinnt; der Bereich als Zahl oder in
  Punktschreibweise), `passive-interface IF|default`, `auto-cost reference-bandwidth MBIT/S`
  (100), `area N stub [no-summary]`, `redistribute connected|static|rip [metric N] [metric-type
  1|2] [subnets]`, `default-information originate [always] [metric N] [metric-type 1|2]`,
  `distance`, `maximum-paths` und `log-adjacency-changes [detail]`; an Interfaces `ip ospf cost`,
  `priority`, `hello-interval`, `dead-interval` (ohne: das Vierfache des Hello-Intervalls),
  `network point-to-point|broadcast` und `ip ospf N area M`. Ein Gerät hat einen Prozess. Die
  Router-ID kommt aus `router-id`, sonst von der höchsten Adresse eines Loopbacks, sonst eines
  anderen Interfaces, und wechselt erst mit `clear ip ospf process` oder einem Neustart. Ein
  L3-Switch braucht `ip routing`. Alles steht in der Running-Config (nach dem Block von RIP) und
  kommt beim Start durch dieselbe Befehlszeile zurück.
- **Interfaces und Nachbarn:** Ethernet ist ein Broadcast-Netz, serielle Leitungen sind
  Punkt-zu-Punkt, Loopbacks stehen als Host-Route mit Kosten 0 in der Datenbank. Die Kosten sind
  Referenzbandbreite durch Bandbreite, mindestens 1 (1 Gbit/s: 1, die serielle Leitung mit 2048
  kbit/s: 48). Hellos gehen alle 10 s; ein Nachbar, von dem 40 s lang keiner kommt, ist weg.
  Hellos mit anderem Hello- oder Dead-Intervall, anderer Netzmaske (auf einem Broadcast-Netz),
  anderem Bereich oder anderer Stub-Einstellung werden mit ihrem Grund ignoriert. Interfaces und
  Nachbarn folgen den Automaten des RFC (9.3, 10.3) mit seinen Ereignisnamen.
- **DR und BDR:** Auf einem Broadcast-Netz wartet ein Interface 40 s (oder bis sich ein BDR
  meldet) und wählt dann nach 9.4: höchste Priorität, dann höchste Router-ID; mit Priorität 0
  nimmt ein Router nie teil. Ein gewählter DR oder BDR bleibt, auch wenn ein besserer kommt;
  fällt der DR aus, übernimmt der BDR, und ein neuer BDR wird gewählt. Nachbarschaften bis Full
  gibt es nur zum DR und BDR, die anderen bleiben untereinander in 2-Way. Der DR beschreibt das
  Netz mit einem Network-LSA.
- **Datenbankabgleich:** In ExStart wird der Router mit der höheren Router-ID Master; die
  Database Descriptions tragen die LSA-Köpfe, was dem Nachbarn fehlt oder älter ist, holt er per
  Link State Request. Der Master wiederholt eine unbeantwortete DD alle 5 s, eine DD mit größerer
  MTU als die des Interfaces wird abgewiesen. Auf einem Broadcast-Netz geht die erste DD an die
  Adresse des Nachbarn, das IOS-artige ARP verliert sie, sie kommt 5 s später wieder: eine
  Nachbarschaft am LAN braucht so gut 45 s.
- **Flooding:** Ein neues LSA geht über die passenden Interfaces weiter (am LAN über den DR an
  alle), Bestätigungen gesammelt nach 1 s an die Gruppe oder sofort an den Absender, wo 13.5 das
  verlangt; was unbestätigt bleibt, geht alle 5 s wieder an den Nachbarn. Eine neuere Instanz, die
  weniger als 1 s nach der letzten kommt (MinLSArrival), wird verworfen, eigene LSAs entstehen
  höchstens alle 5 s (MinLSInterval). Eigene LSAs werden nach 1800 s erneuert, fremde mit
  MaxAge (3600 s) gelöscht; eigene, die es nicht mehr geben soll, gehen vorzeitig mit MaxAge
  hinaus. Kommt ein eigenes LSA aus früherer Zeit zurück (nach `clear ip ospf process`), überholt
  der Router es mit der nächsten Sequenznummer.
- **SPF und Routen:** 200 ms nach einer Änderung rechnet der Router die kürzesten Wege nach
  Dijkstra (16.1, mit der Prüfung in beide Richtungen), dann die Routen anderer Bereiche über die
  Summaries der ABRs (16.2) und die externen (16.4: E1 zählt die Kosten zum ASBR dazu und gewinnt
  vor E2, bei E2 entscheidet bei gleicher Metrik die Entfernung zum ASBR). Bis zu
  `maximum-paths` (4) gleich gute nächste Hops kommen ins RIB, mit der Distanz 110.
- **Bereiche:** Ein Router mit Interfaces im Backbone und in anderen Bereichen ist ABR: er meldet
  die Netze jedes Bereichs als Summary-LSAs (Typ 3) in die anderen und die ASBRs mit Typ 4. Ein
  Stub-Bereich nimmt keine externen LSAs; der ABR schickt ihm eine Standardroute (Kosten 1), mit
  `no-summary` nur diese. Wer redistribuiert oder `default-information originate` hat, ist ASBR
  und schickt AS-externe LSAs (Typ 5; ohne Angabe Typ 2 mit Metrik 20, die Standardroute mit 1).
  Ohne `subnets` gehen nur Klassennetze hinaus, die Standardroute nie über `redistribute`,
  sondern über `default-information originate` (mit `always` auch ohne eigene Standardroute).
- **Anzeigen:** `show ip route [ospf]` mit O, O IA, O E1 und O E2, `[110/Kosten]`, dem Alter und
  dem Interface; `show ip ospf` (Prozess, Router-ID, Rollen, SPF-Läufe, Bereiche), `show ip ospf
  neighbor` (Nachbar, Priorität, Zustand mit Rolle, Dead Time, Adresse, Interface), `show ip
  ospf interface [brief|IF]`, `show ip ospf database` (je Bereich und Typ mit Alter,
  Sequenznummer und Prüfsumme), `show ip protocols`, `debug ip ospf adj|events|hello`, `clear ip
  ospf process` (fragt nach, dann beginnt der Prozess neu). Mit `log-adjacency-changes`
  (eingeschaltet) meldet die Konsole jeden Wechsel nach und von Full, mit `detail` jeden.
- **Ereignisse:** Jede Entscheidung – Hellos, ignorierte Pakete mit ihrem Grund, Zustände von
  Interfaces und Nachbarn, die Wahl, Master und Slave, jede DD, jeder Request, jedes Update und
  jede Bestätigung, neue, eigene, geflutete, bestätigte, gealterte und verworfene LSAs, SPF-Läufe
  und Routen – steht mit deutschem Text in der Ereignisliste und im Paket-Tab (englisch in
  `engine.tsv`). Der Simulationsmodus zeigt OSPF-Pakete braun (Filter „OSPF/RIP“), der Paket-Tab
  zerlegt sie mit jedem LSA und prüft die Prüfsummen ([`netpkt::ospf`](../../lib/netpkt/ospf.rs),
  [`decode.rs`](../../lib/netpkt/decode.rs)).

Noch offen: virtuelle Links, NSSA, Zusammenfassen an Bereichsgrenzen (`area range`), NBMA und
Punkt-zu-Mehrpunkt, Authentifizierung (nur Typ 0), mehrere Prozesse auf einem Gerät, Distanzen
je Art der Route und OSPFv3.

Für Programme: `Action::Ospf { dev, set: OspfSet }` (`Process`, `RouterId`, `Network`, `Passive`,
`RefBw`, `DefaultOrig`, `Redistribute`, `Stub`, `Distance`, `MaxPaths`, `LogAdj`, `IfCost`,
`IfPriority`, `IfHello`, `IfDead`, `IfNetwork`, `IfArea`, `Clear`) und `Engine::ospf(dev)` mit
`cfg`, `ifaces()`, `database(area)`, `externals()`, `routes()`, `border_routers()`,
`neighbours()` und `route_of()`. Geprüft wird OSPF in der Selbsttest-Gruppe `ospf`
([`src/ospftest.rs`](src/ospftest.rs)), die Pakete in der coretest-Gruppe `ospf`, dazu die
Szenarien `ospf_triangle.scn` und `ospf_areas.scn` (siehe [Szenarien](#szenarien-scn)).

### WLAN

Access Points und der Heimrouter bieten WLAN-Netze an, Laptops, Smartphones, Tablets, Drucker,
der IoT-Sensor und der Smart-TV melden sich an ([`jfn_model/wlan.rs`](crates/jfn_model/wlan.rs),
[`jfn_sim/wlan.rs`](crates/jfn_sim/wlan.rs)). Statt eines Kabels verbindet eine Zuordnung
(Assoziation) die Station mit ihrem Access Point; der Simulator legt sie an und löst sie wieder,
die Zeichenfläche zeigt sie als dünne, gepunktete Funklinie (gestrichelt, solange der Handshake
läuft, blass, wenn die Station außer Reichweite ist).

- **Einstellungen** (Inspektor, in der Projektdatei `wlan` je Gerät): Ein Access Point hat ein
  oder mehrere Netze mit SSID, Band und Kanal (2,4 GHz Kanal 1 bis 13, 5 GHz 36 bis 64 und 100
  bis 140, 6 GHz 1 bis 93; der JF-AP6E sendet auf allen drei Bändern über je einen Funkport, der
  Heimrouter 2,4 und 5 GHz über `wlan0`), Sicherheit (offen, WPA2-Personal oder WPA3-Personal
  mit einer Passphrase von 8 bis 63 Zeichen; 6 GHz nur mit WPA3), VLAN (auf Access Points: ihre
  Funkports und die Kabelports in der Grundeinstellung werden dafür Trunks mit VLAN 1 ungetaggt;
  der Heimrouter behält seine Stationen in seinem LAN), Gastnetz und Client-Isolation. Eine Station hat das
  Netz, das sie sucht (SSID und Passphrase), oder keins. Jede Änderung ist ein Rückgängig-Schritt,
  der Simulator folgt ohne Neustart des Geräts.
- **Maßstab und Reichweite:** Der Maßstab des Projekts (`dip_per_m`: Einheiten der Zeichenfläche
  je Meter, ohne Angabe 10, also 10 Pixel je Meter bei 100 %) steht im Inspektor des Projekts.
  Ein JF-AP6E reicht 50 m auf 2,4 GHz, 35 m auf 5 GHz und 25 m auf 6 GHz, der Heimrouter 40, 30
  und 20 m, gemessen von Mitte zu Mitte der Geräte.
- **Anmelden:** Eine Station hört alle Kanäle ihrer Bänder ab (20 ms je Kanal, auf 6 GHz die
  sechs bevorzugten; ein Laptop braucht 760 ms), nimmt das Netz ihrer SSID mit der besten
  geschätzten Rate (dann das stärkere Signal, das höhere Band, den Access Point mit der kleineren
  Nummer), authentifiziert sich (Open System; bei WPA3 SAE mit 10 ms Rechenzeit) und assoziiert
  sich (je zwei Management-Frames zu 250 µs; der Access Point vergibt eine AID und meldet die
  Station mit einem Layer-2-Update den Switches). Bei WPA2 und WPA3 folgt der 4-Wege-Handshake
  mit echten EAPOL-Key-Frames ([`wlankey.rs`](crates/jfn_sim/wlankey.rs): PMK mit PBKDF2, PTK,
  MIC, der GTK und bei WPA3 der IGTK im AES Key Wrap); erst danach fließen Daten, und ein
  DHCP-Client des Ports beginnt sofort. Mit falscher Passphrase verwirft der Access Point
  Nachricht 2 (der MIC passt nicht), schickt Nachricht 1 nach 100 ms und dann jede Sekunde
  wieder, viermal insgesamt, und beendet die Zuordnung (Grund 15); die Station versucht es nach
  10, 20 und 30 s wieder und gibt nach dem vierten Fehlschlag auf, bis sich ihre Einstellung,
  ein Netz eines Access Points oder `iw … connect` ändert. Bei WPA3 scheitert schon SAE: der
  Access Point bestätigt nicht, die Station gibt nach 1 s auf.
- **Funk:** Keine Funkphysik, aber ein festes Modell in ganzzahliger Rechnung, auf jedem Ziel
  gleich: am Rand der Reichweite -82 dBm, je Zehntel der Entfernung 30 dB mehr (ein
  Pfadverlust-Exponent von 3); aus dem Signal die Rate nach der Mindestempfindlichkeit von
  802.11ax MCS 0 bis 11 (-82 bis -52 dBm) als Anteil der schnellsten Rate des Bands (2,4 GHz
  573,5 Mbit/s, 5 und 6 GHz 2,4 Gbit/s), begrenzt durch die Funkports beider Seiten. Die
  Funkstrecke trägt zwei Drittel davon, jeder Frame braucht 300 µs (2,4 GHz), 200 µs (5 GHz)
  oder 150 µs (6 GHz) für den Zugang zum Medium plus die Laufzeit; unter -78 dBm gehen bis zu
  2 % der Frames verloren.
- **Bewegen:** Wird eine Station oder ein Access Point verschoben (im Fenster schon während des
  Ziehens, im Szenario mit `move`), zählt der Simulator neu: Außer Reichweite verliert die
  Station nach sieben fehlenden Beacons (716,8 ms) die Verbindung und sucht wieder; unter -70
  dBm und mit einem Access Point ihres Netzes, der 8 dB besser ist, wechselt sie dorthin
  (Roaming: Reassoziation und ein neuer Handshake; ihr Port bleibt dabei oben, so bleiben
  Adressen, Nachbarn und Lease). Ein Access Point, der ausgeschaltet wird, verstummt (seine
  Stationen merken es an den Beacons); einer, dessen Netz sich ändert oder dessen Funkport
  abgeschaltet wird, meldet seine Stationen ab.
- **Weiterleiten:** Der Access Point verbindet seine Stationen mit seiner Kabelseite und
  untereinander, mit Client-Isolation nicht untereinander; ein Gastnetz erreicht keine andere
  Station, am Heimrouter nur den Router selbst (seine Adresse, DHCP, DNS, den Weg nach draußen).
  Daten fahren als Ethernet-II-Frames über die Funkstrecke, so wie der Treiber der Station sie
  ihrem Stack gibt, der Handshake als EAPOL; Management-Frames (Beacons, Suche,
  Authentifizierung, Assoziation) sind keine Frames, sondern Ereignisse mit ihren Zeiten. Die
  Ereignisliste nennt jeden Schritt („WLAN: assoziiert mit AP1 (AID 1, 5 GHz, Kanal 36, -71
  dBm, 576,0 Mbit/s).“), Paket-Tab und pcapng zeigen Daten und EAPOL.
- **Ältere Projekte:** Ein Funkkabel des Projekts bleibt ein Kabel wie bisher, eine feste
  Zuordnung ohne Suche, Reichweite und Schlüssel; WLAN-Einstellungen gelten für Funkports ohne
  Kabel. Wird es ausgesteckt, sucht die Station ihr Netz.
- **Fenster:** Ansicht › WLAN-Reichweiten zeigt die Reichweiten aller Access Points als Kreise
  mit Band und Metern, sonst zeigt sie ein gewählter Access Point. Stationen tragen vier
  Signalbalken (ab -60, -67 und -75 dBm und darunter einen; leer, solange sie nicht verbunden
  sind). Funklinien, Kreise und suchende Stationen sind Texteinträge für Screenreader und
  Skripte („Funk: Laptop1 – AP1 (Buero, 5 GHz, -71 dBm, 576,0 Mbit/s)“, „Reichweite: AP1, 2,4
  GHz 50 m, 5 GHz 35 m“, „Funk: Laptop2 – nicht verbunden („Buero“)“). Der Inspektor eines
  Access Points zeigt seine Netze (die Passphrase maskiert, bis „Passphrase zeigen“ sie
  aufdeckt), warum eines nicht angeboten wird, die Stationen je Netz und die Reichweiten; der
  einer Station ihren Zustand, den Access Point, Netz, Signal und Rate, ihr Netz zum Ändern
  und die Netze in Reichweite.
- **Befehlszeilen** ([`cli/wlan.rs`](crates/jfn_sim/cli/wlan.rs)): unter Linux `iw dev`, `iw dev
  wlan0 info|link|scan|station dump`, `iw dev wlan0 connect SSID [psk PASSPHRASE]` (für den
  Lauf; das Projekt behält seine Einstellung) und `iw dev wlan0 disconnect`; der Heimrouter
  nennt ein zweites Netz auf seinem Funkport `wlan0-1`. `ip link` zeigt `wlan0` mit
  `NO-CARRIER`, solange keine Zuordnung steht; `wpa_supplicant` und `hostapd` schreiben ins
  Protokoll. Der IOS-artige JF-AP6E kennt `show dot11 associations` und meldet Anmeldungen und
  Abschiede als `%DOT11-…`-Zeilen auf der Konsole. Andere IOS-artige Geräte haben laut Katalog
  keine Funkports.
- **Probleme:** Stationen, die kein Netz ihrer SSID finden (keines angeboten: Fehler, außer
  Reichweite: Warnung), Stationen ohne oder mit falscher Passphrase, Netze, die nicht angeboten
  werden können (etwa WPA2 auf 6 GHz), mit dem Grund.

Entscheidungen: SAE (die Dragonfly-Rechnung von WPA3) wird nicht gerechnet; für sein PMK steht
ein HKDF-SHA256 über Passwort, beide MAC-Adressen und je einen Zufallswert beider Seiten (gleich
auf beiden Seiten bei gleichem Passwort, neu bei jeder Zuordnung), alles danach ist echt. Die
Daten werden nicht mit CCMP verschlüsselt (die Schlüssel entstehen, die Frames bleiben lesbar).

Noch offen: Kanäle, die sich gegenseitig stören, Ad-hoc-Netze, WPA-Enterprise (802.1X mit
RADIUS), Band Steering und 802.11r, Netze über die IOS-artige Befehlszeile einrichten.

Für Programme: `Action::Move { dev, x, y }`, `Action::Wlan { dev, wlan }`, `Action::Scale {
dip_per_m }`, `Engine::wlan_assocs()`, `wlan_station(dev)`, `wlan_stations_of(ap)`,
`wlan_survey(dev, ssid)`, `wlan_scale()`, die Notizen `Note::Wlan(WNote)`. Geprüft wird WLAN in
der Selbsttest-Gruppe `wlan` ([`src/wlantest.rs`](src/wlantest.rs)), SHA-1, PBKDF2, Key Wrap und
CMAC in der coretest-Gruppe `crypto`, EAPOL in `netpkt`, dazu das Szenario `wlan_office.scn` und
die Fensterskripte `wlan_inspector.uis`, `wlan_range.uis`, `wlan_drag.uis` und `wlan_shots.uis`.

## Performance

Große Projekte bleiben flüssig: `tests/perf.uis` verlangt mit 500 Geräten, 800 Kabeln und 300
Rahmen gleichzeitig auf den Kabeln in 1920 × 1080 höchstens 33 ms pro Bild (30 fps).

- **Testprojekt:** `JFNetworker --gen-topology 500` öffnet ohne Datei einen synthetischen
  Campus aus Katalogmodellen (`jfn_model::synth`): 7 Kern-Router im 100G-Ring, 19 Pods aus einem
  Router mit zwei Uplinks, zwei Access-Switches, PCs, Drucker, Kamera und Servern mit je einem
  Port an beiden Switches; statische Routen, Adressen aus 10.P.1.0/24 und 10.P.2.0/24. Kabel über
  einen Baum hinaus verbinden nur geroutete Interfaces oder die zwei Netze eines Servers, es gibt
  also keine Schleife auf Schicht 2 (ohne Spanning Tree). `jfn-sim gen 500 --out campus.jfnet`
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
| 2 parallele Stränge | T1 TCP ✓ · T2 HTTP, Lastgenerator, `curl`/`nc`/`ss` ✓ · Skriptsprache `lib/script` (SK1 Lexer und Parser ✓, SK2 Compiler und VM ✓, SK3 Standardbibliothek ✓) · R1 NAT/PAT und Zugriffslisten ✓, R2 RIP ✓, R3 OSPF ✓, R4 Spanning Tree (STP, RSTP) ✓, R5 IPv6 (R5a Kern, R5b UDP, TCP und Dienste) ✓ · L5 Liniendiagramm und L6 Code-Editor ✓ |
| 3 Verteilte Systeme | D1 Loadbalancer, KV-Shards, Cache, Queue, Worker, Fehlerinjektion ✓ · SK4 Skripte in Hosts ✓ · D2 Metrik-Fenster und Zeitreihen ✓ · U6 Labor-Oberfläche (U6a Metriken, Dienst-Pillen, Sparklines, Fehler aus dem Inspektor; U6b Aufgaben und Labore) ✓ · U7 Skript-Editor ✓ |
| 4 Parität | W1 WLAN ✓ · A1 Aufgaben mit Bewertung ✓ · physische Ansicht, Zurückspulen mit Keyframes, Gruppen, weitere Dienste |

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
