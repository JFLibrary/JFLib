# JFNetworker

Ein Netzwerksimulator und Labor für verteilte Systeme, gebaut als Alternative zu Cisco Packet
Tracer: Geräte und Kabel auf einer Zeichenfläche, echte Protokolle Byte für Byte, eine
IOS-artige Befehlszeile für Router und Switches, eine Linux-artige Shell für Hosts, ein
Simulationsmodus Schritt für Schritt mit Paketansicht, und später Dienste, mit denen sich
horizontal skalierte Systeme aufbauen lassen (Loadbalancer, geshardete Datenbanken, Caches,
Queues, Lastgeneratoren, eigene Skripte).

**Stand (2026-11-02): Die Phasen 1 bis 6 sind fertig, Phase 7 läuft** (siehe [Fahrplan](#fahrplan)).

- **Topologie-Editor:** 44 herstellerneutrale Gerätemodelle aus der Palette ziehen, klicken oder
  per Tastatur platzieren; verbinden mit Port-Auswahl und automatischem oder vorher gewähltem
  Kabeltyp (Kupfer gerade und gekreuzt, Glasfaser Multimode und Singlemode, seriell mit
  gewähltem Ende, Konsole, Funk, WAN, Telefon, Koax, DAC, AOC; Kategorien Cat5e bis Cat8 und OM3
  bis OS2), Kabel mit LEDs, Wegpunkten, Verlauf und Farbe, Notizen, Rückgängig für alles, Projekte
  als `.jfnet`.
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
- **IDE (IDE1, vormals Skript-Editor U7):** ein Programm des Desktops eines Hosts, maximiert
  in seinem Fenster: ein Explorer über die Dateien des Geräts und die noch eingebetteten Skripte, Tabs mit Entwürfen,
  die Sprachdienste von SK6 auch über Importe hinweg, Suchen und Ersetzen in Dateien, darunter
  Ausgabe, Terminal, Probleme, Debugger und Tests; eine Datei läuft als Prozess (`jfs DATEI`) oder
  als Dienst, der auf die Datei verweist (`"file"`), mit Haltepunkten, Schritten, Aufrufliste,
  Variablen und Ausdrücken; die `test_…`-Funktionen einer Datei laufen mit `jfs --test` (siehe
  [IDE](#ide)).
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
- **Routing mit BGP (R6):** Router, L3-Switches und die Firewall sprechen BGP-4 nach RFC 4271
  über TCP-Port 179 (4-Byte-AS-Nummern nach RFC 6793, Route Refresh nach RFC 2918): `router bgp
  ASN` mit `bgp router-id`, `neighbor … remote-as|update-source|next-hop-self|ebgp-multihop|
  shutdown|description|weight`, `network … mask …`, `redistribute connected|static|rip|ospf`,
  `timers bgp`, `bgp default local-preference`; die Zustände von Idle bis Established mit
  ConnectRetry, Hold- und Keepalive-Timer und Kollisionen, eBGP mit TTL 1 und iBGP zwischen
  Loopbacks, die Entscheidung Schritt für Schritt, Schleifen im AS-Pfad, Fast External
  Fallover; `show ip bgp [summary|neighbors]`, `show ip route bgp`, `clear ip bgp`, `debug ip
  bgp` (siehe [Routing mit BGP](#routing-mit-bgp)).
- **Dateien, TFTP und FTP (X1a):** jedes Gerät hat Dateien, die mit dem Projekt gespeichert
  werden: `flash:` mit dem Systemabbild und `nvram:startup-config` auf den IOS-artigen Geräten,
  ein kleiner Verzeichnisbaum auf den Linux-Hosts; ein TFTP- und ein FTP-Server als Dienste,
  `copy running-config tftp:` und zurück, `copy … ftp:`, `dir`, `more`, `delete`, in der Shell
  `ls`, `cat`, `cp`, `rm`, `mkdir`, `echo … >`, `tftp` und `curl ftp://`; der Reiter Dateien im
  Gerätefenster mit Import und Export (siehe [Dateien, TFTP und FTP](#dateien-tftp-und-ftp)).
- **Zurückspulen (K1):** im Simulationsmodus zeigen die Geräte zu jedem Zeitpunkt der
  Aufzeichnung ihren Zustand von damals – LEDs und Kabel, Inspektor, Gerätefenster, Metriken
  und Konsolen, die `show`-Befehle aus diesem Zustand beantworten; der Simulator hält dafür
  Keyframes in einer Speichergrenze und spielt den Rest exakt nach. „Von hier fortsetzen“ macht
  einen solchen Zeitpunkt zur Gegenwart (siehe [Zurückspulen](#zurückspulen)).
- **Physische Ansicht (P1):** neben der logischen Zeichenfläche die Orte des Projekts --
  Standorte, Gebäude, Etagen, Räume, Racks und Tische -- als Hierarchie zum Durchwandern, ein
  Raum als Grundriss, ein Rack von vorn mit den Geräten in ihren Höheneinheiten; Geräte werden
  hineingezogen, jedes Kabel bekommt eine Länge (automatisch aus den Orten oder von Hand) mit der
  Grenze seines Mediums nach IEEE 802.3, ein zu langes kommt nicht hoch und ist überall zu sehen
  (siehe [Physische Ansicht](#physische-ansicht)).
- **Mail (X1c):** Mailserver mit SMTP (RFC 5321 mit SIZE, 8BITMIME, PIPELINING, erweiterten
  Statuscodes und AUTH auf dem Submission-Port 587) und POP3 (RFC 1939 mit UIDL und TOP):
  Postfächer für die Benutzer der eigenen Domänen in den Dateien des Geräts, Weiterleiten an
  andere Domänen über ihre MX-Einträge mit Warteschlange, Wiederholungen und
  Unzustellbarkeitsnachricht, Relay-Regeln mit einem Schalter „offener Relay“ zum Lernen; das
  Programm E-Mail im Desktop (Konto, Posteingang, Verfassen, Antworten, Senden, Abrufen), in der
  Shell `curl smtp://`, `curl pop3://` und `mail` (siehe [Mail](#mail-smtp-und-pop3)).
- **Gruppen und Hintergrundbild (G1):** Geräte, Notizen und Gruppen auf der logischen
  Zeichenfläche zu Gruppen zusammenfassen (Strg+G), verschachtelt; aufgeklappt ein beschrifteter
  Rahmen, zugeklappt ein Cluster-Symbol mit Zahl, Befunden und Dienst-Pillen, an dem die Kabel
  nach draußen als eine Linie je Gegenstelle und Art enden; ziehen, umbenennen, auflösen, alles
  rückgängig; darunter ein Hintergrundbild (ein Grundriss), in der Projektdatei eingebettet,
  verschiebbar, skalierbar, sperrbar (siehe [Gruppen und Hintergrundbild](#gruppen-und-hintergrundbild)).
- **SNMP (X1b):** ein SNMP-Agent mit den Versionen 1 und 2c auf den IOS-artigen Geräten
  (`snmp-server community`, `location`, `contact`, `host`, `enable traps`, `show snmp`, `debug
  snmp packets`) und als Dienst `snmpd` auf den Linux-Hosts; die MIB rechnet er bei jeder Anfrage
  aus dem Zustand des Geräts (system, Interfaces mit 64-Bit-Zählern, Adressen, Routen, die Zähler
  von SNMP), mit einer Community `rw` setzt er Kontakt, Standort, Namen und `ifAdminStatus`, er
  meldet coldStart, linkDown, linkUp und authenticationFailure als Traps oder Informs an den
  Dienst `snmptrapd`; in der Shell `snmpget`, `snmpgetnext`, `snmpwalk`, `snmpbulkwalk` und
  `snmpset`, im Desktop ein MIB-Browser (siehe [SNMP](#snmp)).
- **Logins, Telnet und AAA (X1d):** Anmeldung an den IOS-artigen Geräten mit `enable secret`,
  `username … secret`, Passwörtern auf den Leitungen und Rechtestufen, das Passwort beim
  Tippen unsichtbar; AAA mit RADIUS (`aaa new-model`, `radius server`, Anmeldung, `enable`,
  Rechte und Accounting; die lokalen Benutzer nur, wenn kein Server antwortet), der Dienst
  `radius-server` auf den Hosts, `telnet` als Client an beiden Konsolen (siehe
  [Logins, Telnet und AAA mit RADIUS](#logins-telnet-und-aaa-mit-radius)).
- **Shell mit Ordnern und Prozessen (V1):** ein Arbeitsverzeichnis mit `cd` und `pwd`, die
  klassischen Werkzeuge (`grep`, `find`, `sort`, `tree`, `less`, `man` und viele mehr), Pipes,
  Umleitungen, Listen, Variablen und Platzhalter für jeden Befehl, Tab für Pfade; Skripte laufen
  mit `jfs DATEI` als Prozesse, auch im Hintergrund (`&`, `jobs`, `ps`, `kill`); Ordner in
  `flash:` der IOS-artigen Geräte (siehe
  [Die Shell: Ordner, Werkzeuge, Prozesse](#die-shell-ordner-werkzeuge-prozesse)).
- **Module, Debugger und Tests für Skripte (SK7):** ein Skript lädt mit `import("pfad.jfs")`
  Dateien des Geräts; ein Skript-Dienst lässt sich in virtueller Zeit anhalten und schrittweise
  ausführen, mit Haltepunkten, Aufrufliste und Variablen (die Schnittstelle für die kommende
  IDE); `assert` und `assert_eq` für Tests (siehe [Skripte in Hosts](#skripte-in-hosts)).
- **Mini-Desktop (V2):** das Gerätefenster eines Hosts ist ein kleiner PC – Programme in eigenen
  Fenstern (Terminal, Dateimanager, Texteditor, IP-Konfiguration, Webbrowser, E-Mail,
  MIB-Browser, Systemmonitor, Einstellungen), Symbole, eine Taskleiste mit Startmenü und Uhr,
  F11 für das Vollbild (siehe [Desktop der Hosts](#desktop-der-hosts)).
- **Webbrowser (B1 bis B4):** echte kleine Websites aus dem simulierten Netz im Desktop eines
  Hosts: HTML, Markdown und Text von `http-server`-Routen, Dateien (`"root": "/var/www"`) und
  Skripten mit `http.serve`; Links, Bilder, Formulare mit GET und POST, Verlauf, eigene
  Fehlerseiten, Seitenskripte in `jfs`, Stylesheets aus `<link>`, Cookies, HTTPS mit echtem TLS 1.3
  und eine Entwickleransicht (siehe [Webbrowser](#webbrowser)).
- **Sitzungen und Telnet-Server (X1e):** mehrere Befehlszeilen je Gerät mit eigenem Modus,
  Verzeichnis und eigenen Jobs; „Neue Sitzung“ im Terminal-Panel; Telnet auf die
  vty-Leitungen der IOS-artigen Geräte (`transport input`, Anmeldung über Passwort, lokale
  Benutzer oder RADIUS, Sitzungslimit, Leerlaufzeit, `show users`, `clear line`, `terminal
  monitor`) und `telnetd` auf Linux-Hosts (`who`, `w`, `tty`, `ps` mit TTY) (siehe
  [Sitzungen und Telnet-Server](#sitzungen-und-telnet-server)).
- **Software-Architektur (A2):** Dienste als Einheiten auf Linux-Hosts (Unit-Dateien,
  `systemctl`, `journalctl -u`, Neustart-Regeln in virtueller Zeit), aus der IDE auf eine Gruppe
  von Hosts bereitstellen, einer nach dem anderen mit Gesundheitsprüfung und Abbruch beim ersten
  Fehler, ein Lasttest mit den Metriken daneben und das Labor `05-microservices` (siehe
  [Einheiten, Bereitstellen und Lasttest](#einheiten-bereitstellen-und-lasttest)).
- **SSH und SNMPv3 (X1f):** SSH Version 2 mit echtem Schlüsselaustausch und echter Verschlüsselung:
  `crypto key generate`, `ip ssh`, `transport input ssh` und `ssh -l` auf den IOS-artigen Geräten, der
  Dienst `sshd`, `ssh`, `scp` und `ssh-keygen` auf den Hosts mit `known_hosts` und `authorized_keys`;
  SNMPv3 mit Benutzern, Gruppen und Sichten, HMAC und AES, den Report-PDUs und `-v 3` in den Werkzeugen und
  im MIB-Browser (siehe [SSH](#ssh) und [SNMP](#snmp)).

## Bedienung

- **Startseite:** das Fenster öffnet ohne Projekt auf der Startseite: **Neu** (ein leeres Projekt und
  fünf Vorlagen als Karten: Heimnetz, Büro mit VLANs, Campus, Rechenzentrum mit Leaf-Spine, Webdienst
  hinter Loadbalancer; jede ein laufendes Netz ohne Datei, Speichern fragt nach einem Namen),
  **Zuletzt** (die letzten Projekte mit Ordner und Änderungszeit, Öffnen …; eine fehlende Datei ist
  markiert und lässt sich aus der Liste nehmen, eine Datei außerhalb dessen, was das Programm lesen
  darf, öffnet den Dateidialog an ihrer Stelle) und **Lernen** (die Labore und Aufgaben mit
  Bewertung). Tab läuft durch die Karten, Eingabe öffnet; die Leiste einer automatischen Sicherung
  steht über der Seite und hat dann die Tastatur. Datei › Startseite führt zurück (mit der Frage nach
  ungespeicherten Änderungen), die Einstellung „Beim Start anzeigen“ schaltet die Seite ab.
- **Projektinfo** (Datei › Projektinfo …): Beschreibung und Autor des Projekts, gespeichert in der
  Datei, sichtbar oben in den Eigenschaften des Projekts; ein Rückgängig-Schritt.
- **Geräte platzieren:** eine Kachel der Palette auf die Zeichenfläche ziehen, oder anklicken und
  dann auf die Fläche klicken (Umschalt: mehrere), oder per Tastatur: Strg+1 springt in die
  Gerätesuche, Tab zu den Kacheln einer Kategorie, die Pfeiltasten wählen, Eingabe setzt das
  Gerät in die Mitte der Ansicht.
- **Verbinden (C):** von Gerät zu Gerät ziehen oder beide anklicken; per Tastatur Strg+9 (die
  Topologie), Tab zum ersten Gerät, Eingabe, Tab zum zweiten, Eingabe – die Port-Auswahl öffnet
  sich unter dem zweiten Gerät, Eingabe verbindet das beste Paar. Umschalt verbindet sofort.
- **Kabel zuerst wählen:** der Bereich „Kabel“ oben in der Palette: Strg+1, Tab zum Bereich
  (Eingabe klappt ihn auf), Tab zu den Kacheln, die Pfeiltasten wählen, Eingabe setzt das Werkzeug
  Verbinden mit dieser Art; es bleibt bis Esc oder zu einem anderen Werkzeug, die Statusleiste
  nennt die Art.
- **Konsole:** Doppelklick oder Eingabe auf einem Gerät öffnet seine Konsole im Terminal-Bereich
  (Strg+3), mit Umschalt das Gerätefenster.
- **Beschriftung:** Doppelklick auf eine freie Stelle der Zeichenfläche (oder T: in der Mitte der
  Ansicht) öffnet dort ein schmales Eingabefeld. Eingabe oder ein Klick daneben übernimmt den Text
  (ein Schritt zum Rückgängigmachen), Umschalt+Eingabe beginnt eine neue Zeile, Esc verwirft; leer
  wird nichts gespeichert. Die Beschriftung ist nur Text, ohne Karte, in der Textfarbe des Themes;
  ein Rahmen zeigt sich beim Überfahren und wenn sie gewählt ist. Doppelklick oder F2 ändern sie an
  Ort und Stelle, sie verschiebt, löscht und speichert sich wie eine Notiz. Das Werkzeug N legt
  weiter die große Notiz an.
- **Simulation:** F5 Simulationsmodus, Umschalt+F5 Echtzeit, F6 Abspielen/Anhalten, F7 Schritt
  vor, Umschalt+F7 zurück; das Werkzeug P sendet eine einfache PDU (Quelle, dann Ziel – auch mit
  Tab und Eingabe). In der Vergangenheit zeigen die Geräte ihren Zustand von damals, „Von hier
  fortsetzen …“ (Simulation) macht dort weiter.
- **Tastatur:** Strg+1 … 8 zeigen die Bereiche und geben ihnen die Tastatur, Strg+9 die
  Topologie; dort wählt Tab die Geräte und Notizen, dann die Kabel (Eingabe auf einem Kabel: seine
  Eigenschaften). Werkzeugleiste und Palette sind Fokusgruppen (ein Tab-Halt, Pfeiltasten
  innen). F1 zeigt alle Tastenkürzel. F11 schaltet das aktive Gerätefenster (ohne eines das
  Hauptfenster) in den Vollbildmodus und zurück.
- **Einstellungen** (Werkzeuge › Einstellungen, Strg+,): Raster und Rasterweite, Einrasten,
  Minimap, Tempo beim Start, Bewegung (wie das System, reduziert, voll), Sprache (wie das
  System, Deutsch, English), automatisches Sichern (Sekunden, 0 aus), Startseite beim Start
  anzeigen. Sie liegen in
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
  gestrichelt, Singlemode Strich-Punkt, Konsole und Funk gepunktet, seriell mit Blitz, WAN doppelt,
  Telefon Strich-Punkt-Punkt, Koax als Röhre, DAC mit Querstrichen, AOC dick gestrichelt), Pakete nennen
  Inhalt und Weg für Screenreader und in der Ereignisliste; „reduzierte Bewegung“ lässt Pakete
  springen statt gleiten.

## Bauen und testen

```
python build_rs.py app JFNetworker
build/win/JFNetworker.exe                                   # der Editor
build/win/JFNetworker.exe --open apps/JFNetworker/tests/fixtures/campus.jfnet
build/win/JFNetworker.exe --lang en                         # auf Englisch
build/win/JFNetworker.exe --script apps/JFNetworker/tests/start_page.uis --start-page  # Skripte sehen die Startseite nur so
build/win/JFNetworker.exe --selftest                        # Prüfungen von Modell, Simulator, Oberfläche
build/win/JFNetworker.exe --selftest --pcap-out DIR         # dazu die Mitschnitte der Prüfungen
build/win/JFNetworker.exe --script apps/JFNetworker/tests/place.uis --verify-dirty
build/win/JFNetworker.exe --gen-topology 500 --traffic 300  # synthetischer Campus, 300 Pings (Leistung)

build/win/jfn-sim.exe run apps/JFNetworker/tests/fixtures/lan_router.jfnet --scenario apps/JFNetworker/tests/scen/lan_router.scn
build/win/jfn-sim.exe run apps/JFNetworker/tests/fixtures/lab_shards.jfnet --scenario apps/JFNetworker/tests/scen/lab_metrics.scn --series build/lab.csv
build/win/jfn-sim.exe check apps/JFNetworker/tests/fixtures/lan_router.jfnet
build/win/jfn-sim.exe decode "ff ff ff ff ff ff 02 4a 00 00 01 01 08 06 ..."
build/win/jfn-sim.exe gen 500 --out build/campus.jfnet
build/win/jfn-sim.exe rewind apps/JFNetworker/tests/fixtures/lab_window.jfnet 60   # Keyframes und Zeit zum Zurückspulen
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
| `labels.uis` | Beschriftungen (siehe [Bedienung](#bedienung)): Doppelklick und Text, Eingabe als ein Schritt, Esc lässt nichts zurück, ein Klick daneben übernimmt, leer wird nichts gespeichert, Umschalt+Eingabe, Ändern an Ort und Stelle mit Doppelklick und F2, Eigenschaften, Verschieben, Löschen, Rückgängig und Wiederholen, Speichern und Laden, T in der Mitte der Ansicht, eine Notiz daneben bleibt, eine Rahmenauswahl und andere Werkzeuge beginnen keine, kleine Beschriftungen bleiben bei niedrigem Zoom wählbar |
| `labels_rtl.uis` | Beschriftungen im gespiegelten Fenster: das Feld unter dem Zeiger, die Beschriftung, wo geklickt wurde, ein Klick daneben übernimmt |
| `label_shots.uis` | die Bilder der Beschriftungen (neben Geräten, Kabel und Notiz; gewählt und überfahren; das Eingabefeld) hell, dunkel und im Hochkontrast mit 100 und 150 %, gezoomt |
| `model.uis`, Gruppe `labels` | das Modell der Beschriftungen: der Stil im Datei-Hin-und-Zurück, alte Dateien und unbekannte Werte bleiben Notizen, Anlegen samt Text als ein Schritt, Text und Größe gemeinsam, Rückgängig von Verschieben und Löschen, die Szene (Größe, Trefferfläche, Namen) |
| `file.uis`, `file_reopen.uis` | Speichern unter `build/jfn_test.jfnet`, die Rückfrage, Öffnen, zuletzt geöffnete Dateien (`file_reopen.uis` läuft nach `file.uis`) |
| `connect.uis`, `cable_types.uis`, `waypoints.uis`, `inspector.uis`, `engine.uis` | Verbinden und Port-Auswahl, jede Kabelart, Wegpunkte, die Eigenschaften, LEDs und Befunde des Simulators |
| `cable_palette.uis` | das Kabel zuerst wählen: der Bereich „Kabel“ und die Suche, die Grenzen im Tooltip, Verbinden mit der gewählten Art, nur passende Paare, Glas in RJ45 mit Grund abgelehnt, falsches Kupfer unter „Auto-MDI-X aus“ mit seinem Fehler, ein Rückgängig-Schritt je Änderung, das Popover hält die Art beim Portwechsel, Kategorie und Farbe im Inspektor, das Kontextmenü, die Tastatur, Esc |
| `cable_shots.uis` | die Bilder aller Kabelarten (`fixtures/cable_kinds.jfnet`), hell und dunkel, 100 und 150 % |
| `model.uis`, Gruppe `cables` | die Kabelwahl (siehe [Kabel](#kabel)): jede Art auf jedem Paar von Medien, wo sie passt und wo nicht, die Grenzen je Kategorie, die Modems und der Koax-Port des Smart-TV, eine Datei von vor H2 (`fixtures/cables_v1.jfnet`) mit denselben Arten und Link-Zuständen, Speichern und Laden mit Kategorien und Farben, die Engine (`Action::CableGrade`, `Action::StrictMdix`, Note, Problem), `show interfaces`, die Rangfolge und die Gründe für eine gewählte Art, die Liste des Popovers, Kontrast der Farben; das Szenario zweimal |
| `scen_cable_grades.uis` | Golden-Szenario (`fixtures/cable_grades.jfnet`): Cat5e an 10G unten, Cat6 über 55 m unten, Cat6a bis 100 m, DAC über 7 m unten, das DCE-Ende serieller Kabel in `show interfaces` |
| `terminal_ios.uis`, `terminal_linux.uis`, `device_window.uis`, `log.uis` | die Konsolen, Gerätefenster mit Formularen und Desktop, das Protokoll |
| `simulation.uis`, `events.uis`, `packet.uis`, `pdu.uis` | Schritte, Ereignisse und Zeitleiste, Paket-Tab, PDUs und pcapng-Export |
| `a11y.uis` | alles ohne Maus: Kabel und Geräte als Einträge, Tab über Geräte und Kabel, Platzieren aus der Palette, Verbinden per Eingabe, Konsole, Simulation mit F5/F7 und der PDU per Eingabe, die Tastenkürzel |
| `settings.uis` | den Einstellungsdialog: Raster, Einrasten, Minimap, Tempo, Sprache (das Fenster auf Englisch und zurück), Bewegung, Sicherungsintervall |
| `autosave.uis`, `autosave_recover.uis`, `autosave_reopen.uis` | eine Kopie nach dem Intervall und beim Ruhezustand, die Kopie überlebt das Sitzungsende; die nächste Sitzung stellt sie wieder her und hält sie, bis sie eine eigene hat (ein zweiter Absturz verliert nichts); Beenden mit „Verwerfen“ nimmt die Kopien mit (die drei laufen in dieser Reihenfolge) |
| `english.uis` | das Fenster mit `--lang en`: Menüs, Bereiche, Eigenschaften, Simulationsmodus, Dialoge |
| `start_page.uis` | die Startseite (`--start-page`; `--recent=` gibt dem Lauf seine zuletzt geöffneten Dateien, ein Pfad allein ist erlaubt, mit `\|` verbundene sind es nicht): erster Fokus und Eingabe, was ohne Projekt abgeschaltet ist, das leere Projekt, eine Vorlage, Speichern fragt nach ihrem Namen, Datei › Startseite mit der Rückfrage, die Zeilen von Zuletzt (öffnen, fehlend und entfernen, nicht lesbar: der Dateidialog), eine Fehlermeldung auf der Seite, Labor und Aufgabe, Tab und Umschalt+Tab durch die Karten, Projektinfo mit Rückgängig, die Einstellung |
| `start_page_crash.uis`, `start_page_later.uis`, `start_page_recover.uis` | die Wiederherstellung auf der Startseite (in dieser Reihenfolge): eine Sitzung endet mit ungespeicherten Änderungen, die nächste zeigt die Leiste über der Seite mit der Tastatur auf „Wiederherstellen“, „Später“ lässt die Seite stehen und die Kopie liegen, die dritte stellt sie wieder her; danach wieder die Seite ohne Angebot |
| `start_page_en.uis`, `start_page_shots.uis` | die Seite auf Englisch mit einer geöffneten Vorlage; die Bilder hell und dunkel mit 100 und 150 %, im kleinsten Fenster (auch gescrollt), breit, mit Fokus und Hover, die Vorlage Campus und der Dialog Projektinfo (`build/jfnetworker/start_page_*.png`) |
| `model.uis`, Gruppe `startpage` | die fünf Vorlagen (`assets/templates`): die Liste, keine Probleme, Texte in beiden Sprachen, und jede läuft: Hosts erreichen, was sie sollen, per DHCP, DNS, OSPF mit zwei gleich guten Wegen und über den Loadbalancer; die Einstellung, wann die Seite erscheint, die Zeilen von Zuletzt, Zeiten, die Projektinfo in der Datei und als Rückgängig-Schritt, eine alte Datei ohne sie, ein leeres Projekt ohne Timer |
| `rtl.uis` | ein gespiegeltes Fenster: Bereiche gespiegelt, die Topologie wie gezeichnet bedienbar |
| `shots.uis` | die Bild-Matrix der Hauptansichten (Topologie, Konsole, Simulation, Paket-Tab, Gerätefenster) hell und dunkel mit 100 und 150 % |
| `campus.uis`, `scen_campus.uis` | die Abnahme von Phase 1 (unten) |
| `perf.uis` | die Leistung: 500 Geräte, 800 Kabel und 300 Rahmen in 1920 × 1080 mit höchstens 33 ms pro Bild in Echtzeit, beim Abspielen, Verschieben, Zoomen und Ziehen (siehe [Performance](#performance)) |
| `model.uis`, Gruppe `task` | die Labore: der Abschnitt `lab` der Projektdatei (Texte in beiden Sprachen, ohne Labor wie bisher), was `Lab::issues` findet, einzelne Prüfungen, `goal.rs` mit denselben Berichtszeilen wie der Runner und den Gründen samt Englisch, erreichte Ziele, die bleiben, geordnete Labore; die vier Labore aus `assets/labs` ohne Befund, ungelöst ohne erreichtes Ziel, gelöst wie im Fenster mit jedem Ziel, die Quoren der Shards (zwei Replikate mit strong überstehen keinen Ausfall, drei schon), dieselben Zeitpunkte zweimal |
| `task.uis` | das Panel „Aufgabe“ am Labor des Loadbalancers: leer ohne Labor, die Liste der Labore, Titel und Ziele der Reihe nach, die Einleitung zum Einklappen, „Prüfen“ mit den Befunden, Hinweise nacheinander, der Fortschritt; gelöst im Fenster: das Kabel zu WEB3, das dritte Backend in den Einstellungen von LB1 (Text ohne JSON mit Zeile und Spalte abgelehnt, eine Einstellung, die der Simulator ablehnt), alle Ziele erreicht; Rückgängig lässt sie erreicht, „Simulation zurücksetzen“ fängt neu an |
| `labs.uis` | die Labore auf Englisch (`--lang en`): die Liste, das Panel mit den Texten der Labordatei, F9 mit den Befunden, ein Hinweis, Strg+Umschalt+A; das Cache-Labor über die Einstellungen von CACHE1 gelöst, dann (nach „Änderungen speichern?“ mit Verwerfen) das Campus-Labor in den Konsolen von MLS1, R1, PC3 und PC4 |
| `task_shots.uis` | Bilder der Liste der Labore, des Panels (das Labor der Shards nach „Prüfen“ mit einem Hinweis) und der Einstellungen eines KV-Knotens, hell und dunkel mit 100 und 150 % (`build/jfnetworker/task_*.png`) |
| `script_editor.uis` | die eingebetteten Skripte in der IDE (siehe [IDE](#ide)) auf `fixtures/scripts.jfnet`: das maximierte Fenster der IDE auf dem Desktop aus dem Kontextmenü, die Skripte im Explorer, der Laufzeitfehler in Zeile 13 als Marke und in Probleme (ein Klick setzt die Schreibmarke), die Ausgabe live, Tippen mit Prüfung nach einem Augenblick, F8 und Umschalt+F8, Strg+S lehnt einen Fehler ab und speichert, was sich übersetzen lässt (das Skript startet mit seiner Disk neu: „Start 2“), Rückgängig und Wiederholen („Start 3“, „Start 4“), Stoppen, Ausführen, Neu starten, eine Vorlage als neue Datei (als Dienst ausgeführt ihr Fehler: Port 7 ist belegt), Löschen und Rückgängig, der Reiter Dienste |
| `script_editor_en.uis` | die IDE auf Englisch: Kontextmenü, Reiter, Explorer, Statuszeile, Meldungen in Problems, Output, die Namen der Werkzeugleiste, eine Vorlage als neue Datei auf PC1 mit englischen Kommentaren und Texten, die Tasten in der Übersicht (F1) |
| `script_shots.uis` | die Bilder eines eingebetteten Skripts in der IDE hell und dunkel mit 100 und 150 % (`build/jfnetworker/script_shots_*.png`) |
| `script_ls.uis` | die Sprachdienste in der IDE (SK6) auf `zaehler`: Hover über einem Namen des Simulators mit Beschreibung, die Vervollständigung von `metrics.ob` mit der Beschreibung unter der Liste, die Signaturhilfe und ihr Parameter, F12 und Strg+Klick zur Deklaration, Umbenennen über das Flyout (F2, ein Rückgängig-Schritt, die Ablehnung eines Schlüsselworts), Formatieren (Umschalt+Alt+F, ein Rückgängig-Schritt, die Ablehnung mit Syntaxfehler), die Gliederung der Seitenleiste (ein neues `fn`, ein Klick setzt die Schreibmarke) |
| `script_ls_shots.uis` | die Bilder der Sprachdienste: die Vervollständigung von `net.`, die Signaturhilfe von `net.listen` hell und dunkel mit 100 und 150 %, das Flyout „Name umbenennen“ hell und dunkel (`build/jfnetworker/script_ls_*.png`) |
| `model.uis`, Gruppe `scripted` | worauf die Skripte der IDE bauen: der Quelltext einer Definition als Zeilen oder Text und zurück, Namen, die Vorlagen (übersetzbar, Deutsch und Englisch), Übernehmen, Neu, Umbenennen und Löschen über die Rückgängig-Liste mit Disk und Zahlen des laufenden Skripts und ohne Neustart des Geräts, ein Skript, das der Simulator aus der Datei nicht nehmen konnte, `keep_steps`, die Marken auf bearbeitetem Text |

Fixtures in `tests/fixtures/`: `office.jfnet` (ein kleines Büronetz mit jeder Kabelart),
`cables.jfnet` (Geräte ohne Kabel), `pdu_lab.jfnet` (DHCP, DNS-Zone `lab`, TCP-Echo),
`campus.jfnet` (die Abnahme), `scripts.jfnet` (zwei eingebettete Skripte auf SRV1), `ide.jfnet`
(Dateien und ein Dienst aus einer Datei auf SRV1 für die IDE) und
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
| `model.uis`, Gruppe `bgp` | BGP (siehe [Routing mit BGP](#routing-mit-bgp)): eine eBGP-Sitzung durch jeden Zustand auf die Nanosekunde (der erste SYN, den das IOS-artige ARP verliert, die Kollision der beiden Verbindungen, die der höheren BGP-ID bleibt), das OPEN auf dem Kabel (Version, AS, Haltezeit, ID, die drei Capabilities), TTL 1, Keepalives alle 60 s ab dem letzten UPDATE, `timers bgp 10 30` auf einer Seite (Haltezeit 30, Keepalive 10 auf beiden), iBGP zwischen Loopbacks mit `update-source` und TTL 255, ein Next Hop, den das IGP nicht kennt, bis `next-hop-self`, kein iBGP-Weg an einen anderen internen Nachbarn; jeder Schritt der Entscheidung (Gewicht, LOCAL_PREF, selbst erzeugt, AS-Pfad, ORIGIN, MED gleicher und verschiedener Nachbar-ASe, extern vor intern, IGP-Metrik über eine serielle Leitung, BGP-ID, Nachbaradresse); eine Schleife im AS-Pfad; ein falsches `remote-as` mit NOTIFICATION 2/2 auf dem Kabel, der Pause von 60 s und dem Problem, dann behoben; Ausfälle über den Hold Timer hinter einem Switch, über einen RST nach einem Neustart des Nachbarn und über Fast External Fallover; Redistribution mit MED, RIB-Failure (`r`), ein `network` ohne Route als Problem und mit Route nach der Mindestpause; die Befehlszeile (Modus, Hilfe, Tab, Fehlermeldungen, `show`, `clear ip bgp … soft in` mit ROUTE-REFRESH, `clear ip bgp *` mit Cease, `shutdown`, `no router bgp`, die Running-Config als Startkonfiguration, `% IP routing not enabled`); Texte auf Deutsch und Englisch; Determinismus |
| `bgp_console.uis` | BGP an R1s Konsole (`fixtures/bgp_pair.jfnet`, R2 wartet schon): `router bgp` mit seinem Modus, `?` darin, Nachbar und Netz getippt, die Sitzung kommt (die Konsole meldet sie), `show ip bgp summary`, `show ip bgp` und `show ip route bgp` mit der Route von R2 |
| `model.uis`, Gruppe `files` | Dateien, TFTP und FTP (siehe [Dateien, TFTP und FTP](#dateien-tftp-und-ftp)): der Speicher (Pfade beider Arten, Verzeichnisse, das Systemabbild, Grenzen, die Einträge der Projektdatei mit Text und Base64), die Befehle (`dir`, `show flash:`, `more`, `delete`, `copy`; `ls`, `cat`, `echo > DATEI`, `cp`, `mkdir`, `rm`, `rmdir`, `touch`), eine Sicherung per TFTP und das Zurückholen auf die Nanosekunde, ein verlorenes ACK, eine fehlende Datei, ein Server nur zum Lesen, eine Quote, Optionen für die Blockgröße, FTP mit Anmeldung, Liste, Hoch- und Herunterladen passiv und aktiv, ein falsches Passwort, eine volle Quote, die Probleme-Liste, die Projektdatei hin und zurück, Determinismus |
| `files_window.uis` | der Reiter Dateien auf `fixtures/backup.jfnet`: SRV1s Dateien aus dem Projekt mit Text, Exportieren … in eine Datei des Rechners, Löschen als Rückgängig-Schritt und zurück, Importieren … in das gewählte Verzeichnis (die Shell hat die Datei sofort), was die Shell schreibt, kommt ins Projekt; R1s `flash:` mit dem Systemabbild und `nvram:startup-config` |
| `files_services.uis` | TFTP- und FTP-Server im Reiter Dienste: Karten mit Statuszeile, Einstellungen als JSON (nur lesend mit Quote, ein abgelehnter Port, ein Benutzer ohne Namen, ein fehlendes Heimatverzeichnis in der Probleme-Liste), Rückgängig; eine Sicherung landet nur beim schreibbaren Server |
| `files_console.uis` | eine Sicherung an R1s Konsole: `copy running-config tftp:` mit Fragen, Fortschritt und Ergebnis, die Datei als Rückgängig-Schritt auf SRV1, ein falscher Name aus der Sicherung zurückgeholt, eine Kopie nach `flash:` mit `dir`, Rückgängig nimmt sie wieder weg |
| `files_shots.uis` | die Bilder von Dateien, TFTP und FTP (Konsole nach einer Sicherung, der Reiter Dateien, die Einstellungen des TFTP-Servers) hell und dunkel mit 100 und 150 % (`build/jfnetworker/files_*.png`) |
| `model.uis`, Gruppe `keyframes` | Zurückspulen (siehe [Zurückspulen](#zurückspulen)): der erste Keyframe beim Start und seine Größe, der Start und die Gegenwart wiederhergestellt (der Moment 0 mit seinen Ereignissen, ein Moment zwischen zwei Ereignissen), der Lauf bleibt unberührt; ein Campus in Echtzeit mit Konsolenzeilen, einem gezogenen und wieder gesteckten Kabel, einem Ping über `Inline::call` und Schritten des Simulationsmodus zwischen den Ereignissen eines Moments, an 13 Stellen wiederhergestellt und Byte für Byte gleich dem Lauf (der ganze Zustand und die Aufzeichnung der Wiederholung), auch sieben Sekunden vom ersten Keyframe aus; Leases, ARP-Tabellen und LEDs von damals gegen die heutigen; eine Wiederholung in Stücken mit wachsendem Fortschritt; die Speichergrenze nach Zahl (sechs Keyframes mit ihren Zeiten, der erste bleibt, die Abstände schrumpfen zur Gegenwart hin) und nach Bytes, zu knapp für zwei (der Start und die Eingaben davor gehen); die Eingaben mit ihren Zeiten (Uhr, Konsole, Abbruch, Aktion, Aufruf); „Von hier fortsetzen“ gegen einen Lauf, der nie weiterkam (Zustand, Aufzeichnung und Eingaben gleich); im Fenster: der gezeigte Zustand, LEDs von damals und von jetzt, Zeilen in der Vergangenheit ändern nichts, eine Bearbeitung holt in die Gegenwart, Fortsetzen mit einer späteren Änderung am Projekt, Rückgängig und Wiederholen danach, ein Zeitpunkt vor dem ersten Keyframe |
| `rewind.uis` | Zurückspulen im Fenster (`fixtures/lan_router.jfnet`): R1 pingt, bekommt eine statische Route und schaltet Gi0/0/0 ab und wieder an; ein Schritt zurück zeigt die ARP-Tabelle mit PC2, aber die Route noch nicht (der Ping lief damals noch), bei 0,5 s nur die eigenen Adressen, eine Zeile in der Vergangenheit ändert nichts, bei 4,5 s das Kabel PC1 – R1 unten, die Route da und das Netz von Gi0/0/0 weg; „Zurück zu live“; „Von hier fortsetzen“ bei 4,5 s mit Rückfrage (abgebrochen, dann bestätigt): das Kabel bleibt unten, die Konsole sagt, wo es weitergeht, und antwortet wieder live |
| `rewind_shots.uis` | die Bilder des Zurückspulens (die Ansicht bei 4,5 s mit Hinweis, LEDs und Konsole von damals; die Rückfrage) hell und dunkel mit 100 und 150 % (`build/jfnetworker/rewind_*.png`) |
| `model.uis`, Gruppe `phys` | die physische Ansicht (siehe [Physische Ansicht](#physische-ansicht)): die Grenze jedes Mediums bei jeder Speed (Kupfer, SR und LR, Konsole, V.35, WAN und Funk ohne), Bauformen und Höhen der Modelle, Optiken und die Faser, die sie wählen, der Baum ohne Orte, ältere Projekte im Standardraum (sechs Fixtures: 7 m, zur Decke 4 m, zur Wolke 3 m, nichts über der Grenze, Verzögerungen wie bisher), die automatischen Längen des Campus auf den Meter (im Rack, auf dem Tisch, zwischen Räumen, Etagen und Häusern, an der Decke), ein Kabel über seiner Grenze an beiden Enden unten mit Notiz, Protokollzeilen, Problem, `show interfaces` und `ip link`, wieder oben, wenn es kürzer wird (ein anderer Raum, eine Länge von Hand, automatisch), Optiken, die nicht passen, die Laufzeit aus dem Plan, Orte hinzufügen, umbenennen, verschieben, vergrößern, entfernen und Geräte aufstellen als Rückgängig-Schritte, denen der Simulator folgt, die Ablehnungen, die Datei, die Wahl der Ansicht im Layout, die Syntax der Szenarien und ein Szenario zweimal (Determinismus) |
| `phys_view.uis` | die physische Ansicht auf `fixtures/phys_campus.jfnet`: der Schalter der Werkzeugleiste, Ansicht › Logische / Physische Ansicht mit ihren Tastenkürzeln, durch die Orte (Serverraum, Etage, Gebäude, Campus, alle Standorte), Zurück, die Brotkrumen, Doppelklicks auf Orte und Wege, Tab und Eingabe, ein Ort im Inspektor |
| `phys_rack.uis` | Geräte aufstellen: SRV1 im Rack von der Tastatur zehn Höheneinheiten höher (mit Rückgängig), die Höheneinheit im Inspektor (eine über dem Rack abgelehnt), aus dem Rack über die Liste der Orte, aus der Ablage ins Rack gezogen (die unterste freie Einheit), auf die Einheiten von R1 gezogen (abgelehnt), PC4 auf ein Rack (abgelehnt: kein Rack-Gerät) und an einen Platz im Labor, die Höhe des Racks im Inspektor (zu niedrig für seine Geräte abgelehnt) |
| `phys_length.uis` | ein zu langes Kabel, gezeigt und behoben: das Problem, das Kabel „unten“ auf der Zeichenfläche, Länge, Grenze und Ablehnung im Inspektor, das markierte Kabel in der physischen Ansicht, 90 m von Hand (oben), 132 m (unten), Automatisch und Rückgängig, eine Länge, die keine Zahl ist, PC4 über den Inspektor in Büro 1 (23 m, oben) |
| `phys_shots.uis` | die Bilder der physischen Ansicht (Serverraum mit dem markierten Kabel und seinem Inspektor, die Vorderseite von Rack 1, das Erdgeschoss von Haus A, der Campus) hell und dunkel mit 100 und 150 % (`build/jfnetworker/phys_*.png`) |
| `model.uis`, Gruppe `mail` | Mail (siehe [Mail](#mail-smtp-und-pop3)): der Namensserver aus `/etc/resolv.conf` (auch in der Konfiguration), eine Zustellung ins eigene Postfach auf die Nanosekunde (Received, Return-Path, die Notizen), das Weiterleiten über den MX von b.lab mit PIPELINING und zwei Received-Zeilen (R1 verliert den ersten SYN), Unzustellbares für einen unbekannten Benutzer, eine Domäne, die das DNS nicht kennt, und einen MX ohne Adresse, Wiederholungen bei ausgeschaltetem MAILB (der erste Versuch scheitert nach 3 s, der nächste kommt 10 s später, dann wartet der Server 20 s) und das Aufgeben nach 60 s, das Abrufen des Programms E-Mail mit UIDL und DELE (und mit `keep` nur einmal), Senden über 587 mit AUTH und einer Kopie in Gesendet, `curl pop3://` mit LIST, RETR, UIDL, TOP und DELE, falsche Passwörter bei POP3 und AUTH, die Größe (SIZE und nach dem Punkt), abgelehnte Relays und der offene Relay, die Probleme, `mail -H`, die Projektdatei mit den Postfächern und zwei gleiche Läufe |
| `mail_services.uis` | SMTP- und POP3-Server im Reiter Dienste von MAILA (`fixtures/mail_domains.jfnet`): Karten mit Statuszeile, Einstellungen als JSON (eine abgelehnte Domäne, ein offener Relay in der Statuszeile und in der Probleme-Liste, ein Benutzer ohne Namen, einer ohne Postfach), Rückgängig; eine Nachricht aus PC1s Konsole zählt auf der Karte und legt das Postfach im Reiter Dateien an |
| `mail_client.uis` | das Programm E-Mail: PC2 ohne Konto beginnt mit dem Formular, Bobs Konto (587 mit Anmeldung) als Rückgängig-Schritt, eine Nachricht an Anna (Statuszeile, Gesendet); Anna ruft ab, liest und antwortet (Empfänger, „Re: “, der Text zitiert); Bob ruft die Antwort ab, Rückgängig und Wiederholen des Abrufs, Löschen |
| `mail_shots.uis` | die Bilder von Mail (eine abgerufene Nachricht im Programm E-Mail, eine Antwort, das Konto, die Einstellungen des SMTP-Servers) hell und dunkel mit 100 und 150 % (`build/jfnetworker/mail_*.png`) |
| `model.uis`, Gruppe `groups` | Gruppen und Hintergrundbild (siehe [Gruppen und Hintergrundbild](#gruppen-und-hintergrundbild)): die äußersten des Gewählten in die innerste gemeinsame Gruppe, Verschachteln, ein Gerät je Ebene, leer gewordene Gruppen, Nummern, die fehlen (und mit Rückgängig wiederkommen), Zyklen, doppelte Mitglieder, Namen, Aufheben, was ein Zuklappen versteckt; die Datei (Gruppen und die Bytes des Bilds unverändert, ältere Dateien ohne beides, alle Fixtures, Labore und Aufgaben ohne); das Bild (Formate, Grenzen, das Fixture dekodiert, eingepasst); die Rückgängig-Schritte (die Bytes geteilt); die Szene der Zeichenfläche (Rahmen, Cluster in der Mitte, eine Linie je Gegenstelle und Art mit den schlechtesten LEDs, ein einzelnes Kabel behält seine Kennung, Kabel innen ausgeblendet, verschachtelt, was Ziehen bewegt, Aufklappen eines versteckten Geräts); 3:1 der Farbrollen; der Simulator mit und ohne Gruppen und Bild gleich (`lan_router`) |
| `groups.uis` | Gruppen im Fenster auf `fixtures/groups.jfnet`: Strg+G, Rückgängig und Wiederholen, F2, Zuklappen mit dem Knopf (die Linie „2 Kabel“, das Kabel zwischen den Servern ausgeblendet), Doppelklick, Ziehen am Titel um Rasterschritte, der Inspektor (Farbe, Zuklappen, Mitglieder), Gummiband, Strg+A, verschachtelt und zugeklappt, Strg+Umschalt+G, Entf auf einer Gruppe, ein vergebener Name |
| `groups_bg.uis` | das Hintergrundbild: `fixtures/bg_plan.png` über den Dateidialog, eingepasst und gewählt, Rückgängig, Ziehen, der Griff, die Felder (eine Ablehnung), Sperren (Klick und Gummiband darüber), Ansicht › … sperren, anpassen, entfernen, gespeichert und wieder geöffnet, eine Datei, die kein Bild ist |
| `groups_sim.uis` | zugeklappte Mitglieder: das Abzeichen ihrer Befunde, im Simulationsmodus die Kopien einer Flut auf der Linie in den Cluster, die Konsole eines versteckten Geräts, ein Befund in Probleme (ein Klick wählt den Cluster, ein Doppelklick klappt auf) |
| `groups_shots.uis` | die Bilder (gesperrtes Hintergrundbild, eine Gruppe auf, eine zu; verschachtelt mit gewähltem Bild) hell und dunkel mit 100 und 150 % (`build/jfnetworker/groups_*.png`) |
| `model.uis`, Gruppe `snmp` | SNMP (siehe [SNMP](#snmp)): die Gruppe system per Get auf die Nanosekunde, Walks über `ifTable` und `ifXTable` mit den Zählern der Ports bis zur Anfrage, die Reihenfolge über die Tabellen hinweg, GetBulk mit Non-Repeaters, Wiederholungen und der Größengrenze, Set von sysLocation, sysName und `ifAdminStatus` (der Port geht herunter, die Running-Config zeigt es, das Projekt erst nach `copy run start`), was eine Community nur zum Lesen nicht darf, eine falsche Community und eine Zugriffsliste (keine Antwort, der Zähler, die Konsolenzeile, authenticationFailure), die Fehler der Version 1 gegen die von 2c (RFC 3584), tooBig, linkDown- und linkUp-Traps in `snmptrapd` mit seiner Datei, ein Inform über ein Kabel mit Verlust, bis es bestätigt ist, coldStart eines Linux-Agenten beim Start, seine Einstellungen und Ablehnungen, die IOS-artigen Befehle mit ihrer Running-Config gelesen wieder dieselbe, `show snmp`, `debug snmp packets`, die Probleme-Liste, die Zerlegung einer aufgezeichneten Nachricht, Determinismus |
| `snmp_services.uis` | die SNMP-Dienste im Reiter Dienste (`fixtures/snmp_monitor.jfnet`): der Trap-Empfänger von MON mit dem coldStart von PC1 in seiner Statuszeile, seine Einstellungen als JSON (eine Community mit Leerzeichen abgelehnt, dann eine andere Logdatei), der Agent von PC1 (ein Inform der Version 1 abgelehnt, ein neuer Standort, den MON mit `snmpget` liest), Rückgängig bringt die alten Einstellungen zurück |
| `snmp_console.uis` | SNMP an den Konsolen: `snmpwalk` der Gruppe system von R1 auf MON, Tab ergänzt die Werkzeuge, R1 ändert seinen Standort mit `snmp-server location` und MON sieht ihn, `show snmp`; SW1 schaltet einen Port ab, sein linkDown-Trap landet bei `snmptrapd` auf MON (die Logdatei in `/var/log`, die Zeile im Protokoll) |
| `snmp_mib.uis` | der MIB-Browser im Desktop von MON: der Baum von mib-2, die Karte eines Objekts (Name, OID, Syntax, Zugriff), Get von sysName an R1, Walk von system, Set von sysLocation mit der Community, die R1s Zugriffsliste MON erlaubt, `ifTable` mit der Tastatur geöffnet und GetNext von `ifDescr.2`; jede Anfrage läuft als Befehl in MONs Konsole unter dem Baum |
| `snmp_shots.uis` | die Bilder von SNMP (MONs Konsole nach einem Walk, der MIB-Browser nach einem Get, der Trap-Empfänger im Reiter Dienste, die Ereignisse mit dem Chip SNMP) hell und dunkel mit 100 und 150 % (`build/jfnetworker/snmp_*.png`) |
| `model.uis`, Gruppe `aaa` | Logins, AAA und RADIUS (siehe [Logins, Telnet und AAA mit RADIUS](#logins-telnet-und-aaa-mit-radius)): die Anmeldung an der Konsole, `enable` und die Rechtestufen, die gespeicherten Geheimnisse, RADIUS mit Annahme, Ablehnung und Rückfall auf die lokalen Benutzer, Accounting, der Telnet-Client |
| `login_console.uis` | eine Anmeldung an der Konsole von R1 (`login local` mit Banner, `fixtures/aaa_login.jfnet`): das Passwort ohne Echo, ein falsches abgewiesen, der Benutzer mit Stufe 15 gleich bei „R1#“ |
| `telnet_session.uis` | der Telnet-Client in einem Terminal-Tab: eine abgewiesene Verbindung, dann eine Sitzung zu einem Listener auf SRV1, Strg+Umschalt+6 beendet sie |
| `aaa_services.uis` | der RADIUS-Server im Reiter Dienste von SRV1: Karte mit Statuszeile, die Einstellungen als JSON (ein zweiter Benutzer, ein Client ohne Secret abgewiesen) |
| `aaa_shots.uis` | die Bilder (eine Konsole gesperrt bei der Anmeldung, der RADIUS-Server im Reiter Dienste) hell und dunkel mit 100 und 150 % (`build/jfnetworker/aaa_*.png`) |
| `model.uis`, Gruppe `shell` | die Shell mit Ordnern, Werkzeugen und Prozessen (siehe [Die Shell: Ordner, Werkzeuge, Prozesse](#die-shell-ordner-werkzeuge-prozesse)): Arbeitsverzeichnis und `cd` mit allen Sonderfällen, die Grammatik (Listen, `&&`, `\|\|`, `!`, Anführungszeichen, Variablen, `$?`, `!!`, Syntaxfehler), Umleitungen in jeder Form, Pipes auch hinter `ping`, `curl` und `nc`, Strg+C in einer Pipeline, Platzhalter, jedes Werkzeug mit seiner genauen Ausgabe, Tab für Pfade, `jfs` als Prozess (Ausgabe, Timer, Fehler, Strg+C, im Hintergrund), Jobs mit `jobs`, `fg`, `wait`, `kill`, `ps`, `sleep` und den Exit-Codes, Ordner in `flash:` von R1, ein Projekt mit Ordnern gespeichert und geladen, Ausschalten, Determinismus |
| `scen_shell_tour.uis` | Golden-Szenario `shell_tour.scn`: ein Projektbaum auf PC1, ein Skript mit `echo` geschrieben und als Prozess in eine Logdatei gestartet, das Log mit `grep`, Pipes hinter `ping` und `curl`, Jobs mit `jobs`, `ps` und `kill`, Platzhalter, `find`, Umleitungen von stderr, Ordner in `flash:` von R1 |
| `shell_terminal.uis` | PC1s Shell im Terminal-Panel: `mkdir`, `cd` (der Prompt zeigt `~/app`), Tab ergänzt einen Ordner mit `/`, eine Datei mit Leerzeichen, bei mehreren Treffern erst den gemeinsamen Teil und dann die Liste, eine Pipe, ein Skript als Prozess mit Strg+C beendet (`$?` 130), ein Job im Hintergrund mit `[1] PID` und seiner Fertig-Zeile, `less` mit --More-- |
| `shell_shots.uis` | die Bilder der Shell (PC1 mit Ordnern, `tree`, einer Pipeline, `ps` und `ls -l`; R1 mit Ordnern in `flash:`) hell und dunkel mit 100 und 150 % (`build/jfnetworker/shell_*.png`) |
| `model.uis`, Gruppe `scriptmod` | Module und Debugger der Skripte (siehe [Skripte in Hosts](#skripte-in-hosts)): Importe aus den Dateien des Geräts mit absolutem und relativem Pfad, die Zeit der Disk, eine fehlende Datei, ein Zyklus, ein Fehler im Modul in Fehler, Log und Status, ein angehaltener Dienst mit verspäteten Timer-Runden, Schritte, ein Ausdruck im Halt, ein Haltepunkt mit Bedingung, Determinismus |
| `desk_programs.uis` | der Desktop von PC1 (siehe [Desktop der Hosts](#desktop-der-hosts)): das Gerätefenster öffnet auf ihm, Symbole (ein Klick wählt, Doppelklick, Eingabe und Pfeiltasten), das Startmenü mit seinen Gruppen, drei Programme in Fenstern, die Taskleiste (aktivieren, minimieren, zurückholen, ihr Menü schließt ein Fenster), Maximieren und Minimieren mit der Titelleiste, Verschieben, Alt+F6, F11 und Vollbild, das Terminal als Konsole des Geräts (der Terminal-Bereich verweist dorthin), der Desktop wie zuvor, nachdem das Gerätefenster zu und wieder offen war |
| `desk_files.uis` | Texteditor und Dateimanager: die Notizen, ein Text mit „Speichern unter …“ als Datei, die die Shell mit `cat` liest; Neuer Ordner, Umbenennen, Kopieren und Einfügen in den neuen Ordner, Löschen mit Rückgängig und Wiederholen, die Kopie per Doppelklick im Texteditor, Eigenschaften |
| `desk_monitor.uis` | Systemmonitor und Einstellungen: ein Job aus dem Terminal (`sleep 100 &`) unter den Prozessen, mit „Beenden“ (`kill` im Terminal) beendet, `init` nicht; die Dienste von SRV1 mit dem laufenden Webserver, CPU-Last und Speicher; Hintergrund, Symbolgröße und Uhr, dieselben Werte unter Werkzeuge › Einstellungen |
| `desk_terminals.uis` | weitere Terminal-Fenster (V2 mit den Sitzungen von X1e): „Weiteres Terminal“ im Startmenü öffnet „Terminal (2)“ mit eigener Sitzung (eigener Prompt und Verzeichnis, `tty` `/dev/pts/0`, `ps` mit `tty1` und `pts/0`), die Konsole behält ihr Verzeichnis, `who` zählt beide; Schließen des Fensters beendet die Sitzung |
| `desk_ide.uis` | die IDE auf dem Desktop (IDE1 in V2s Fenster, ihr Terminal eine Sitzung von X1e): eine `.jfs`-Datei aus dem Dateimanager öffnet sich in der maximierten IDE, Strg+F5 führt sie im Terminal der IDE aus (`tty` `/dev/pts/0`), `who` an der Konsole zählt beide, Schließen des IDE-Fensters beendet ihre Sitzung |
| `desk_rtl.uis` | der Desktop in einem gespiegelten Fenster: Symbole rechts, Taskleiste und Titelleisten gespiegelt, wie gezeichnet bedienbar; Bilder hell und dunkel mit 150 % |
| `desk_shots.uis` | die Bilder des Desktops (drei Fenster, das Startmenü) hell und dunkel mit 100 und 150 %, Hochkontrast, die Hintergründe Netz und Sand mit großen Symbolen (`build/jfnetworker/desk_*.png`) |
| `model.uis`, Gruppe `desk` | worauf der Desktop baut: die Prozesse von `jfn_sim/procs.rs` gegen `ps` (ein Job mit PID und Befehl, `kill`, ein Dienst, der kein Job ist), die CPU-Zeit je Sekunde gegen die Zahlen des Webservers, freie Namen, Kopien eines Ordnerbaums und Verschieben im Dateimanager, welches Fenster die Konsole zeigt, der Kontrast der Beschriftungen auf jedem Hintergrund in beiden Themes |
| `model.uis`, Gruppe `sessions` | Sitzungen und Telnet-Server (siehe [Sitzungen und Telnet-Server](#sitzungen-und-telnet-server)): zwei lokale Sitzungen von PC1 neben der Konsole mit eigenem Verzeichnis, Variablen, Verlauf und Jobs, ihre Prompt-Meldungen; eine lokale Sitzung von R1 (`tty 1`) mit eigenem Modus, `?` und Tab darin, `%SYS-5-CONFIG_I` nennt sie; eine geschlossene Sitzung (ihr Ping endet, Zeilen abgelehnt, Nummern nicht wieder vergeben), `exit`; Strom aus beendet alle bis auf die Konsole; Telnet von PC1 zu R1 mit Leitungspasswort (verdeckt, ein falsches, `show users` und `show line` von beiden Seiten, `?` über Telnet, `exit`), ohne Passwort abgewiesen, mit `login local` (Rechte 15, drei Fehlversuche schließen), mit AAA und RADIUS; fünf vty-Leitungen, die sechste Verbindung abgewiesen, `clear line`; die Leerlaufzeit (30 s, 10 min als Vorgabe); `show users` mit Leerlauf; `terminal monitor`; `transport input none` und `access-class`; `telnetd` auf SRV1 mit Login, `who`, `w`, `tty`; die TTY-Spalte von `ps`; die Wiederholung der Sitzungsbefehle beim Zurückspulen; Determinismus |
| `scen_sessions.uis` | Golden-Szenario `sessions.scn`: der Admin-PC per Telnet an R1 (Loopback eingerichtet, `show users`, R1s Konsole nennt die vty-Leitung) und an R2 (`login local`), die vergessene Sitzung endet nach der Leerlaufzeit |
| `sessions_terminal.uis` | eine zweite Sitzung von ADMIN im Terminal-Panel („Neue Sitzung“, der Tab „ADMIN (2)“) mit eigenem Prompt und Verzeichnis, `tty`, `who` von der Konsole, Schließen des Tabs beendet sie |
| `telnet_vty.uis` | Telnet von ADMINs Konsole zu R1 und zurück: das Passwort verdeckt, R1s Prompt in der Konsole, `show users`, `exit`, der Prompt der Shell wieder |
| `model.uis`, Gruppe `ssh` | SSH (siehe [SSH](#ssh)): ein PC per SSH an einem Router mit lokalem Benutzer und mit RADIUS samt Rückfall, `transport input` lehnt Telnet oder SSH ab, `show ip ssh`, `show ssh`, `show users`, falsche Passwörter und ihre Grenze, die Zeit für eine Anmeldung, die Leerlaufzeit, exec-Modus mit Status, der IOS-artige Client, RSA- und Ed25519-Schlüssel, `debug ip ssh`, die Running-Config, ein Schlüssel über einen Neustart; `sshd` mit Passwort und mit Schlüssel aus `ssh-keygen` und `authorized_keys`, `known_hosts` (Frage, gemerkt, geänderter Schlüssel abgelehnt, `-R`), ein weiterer Benutzer, `who` und `w`, Strg+C und `~.`, `scp` hin und her; die Pakete im Klaren und verschlüsselt, Zurückspulen und Determinismus |
| `model.uis`, Gruppe `snmpv3` | SNMPv3 (siehe [SNMP](#snmp)): Get, Walk, Bulk und Set auf jeder Stufe, die Discovery, jeder Report, das Zeitfenster und die Boots nach einem Neustart, Sichten und Gruppen, die IOS-artigen Befehle mit Running-Config und einem Start daraus, `snmpd` auf Linux, was auf der Leitung steht, Determinismus |
| `scen_ssh_snmpv3.uis` | Golden-Szenario `ssh_snmpv3.scn`: der Admin-PC per SSH an R1 (Telnet aus, Host-Schlüssel gemerkt, Loopback eingerichtet), exec-Modus an R2 mit RSA-Schlüssel, sein Schlüssel per `scp` auf SRV1 und danach ohne Passwort, `snmpget` und `snmpset` mit Version 3 an SW1, R1 und SRV1 mit Sicht und Reports, zuletzt ein abgelehnter neuer Schlüssel |
| `ssh_session.uis` | SSH von ADMINs Konsole zu R1 und zurück: die Frage nach dem Schlüssel, das Passwort verdeckt, R1s Prompt, `show users`, `exit`, beim zweiten Mal keine Frage, `~.` |
| `snmpv3_mib.uis` | der MIB-Browser mit einem Benutzer der Version 3: Benutzer, Stufe, Signatur und Passwörter statt der Community, Get auf authPriv und authNoPriv, ein falsches Passwort |
| `ssh_shots.uis` | die Bilder von SSH (Konsole mit Sitzung an R1) und SNMPv3 (MIB-Browser) hell und dunkel mit 100 und 150 % (`build/jfnetworker/ssh_*.png`, `snmpv3_*.png`) |
| `sessions_shots.uis` | die Bilder der Sitzungen (ADMIN mit einer zweiten Sitzung, darin per Telnet an R1 mit `show users`) hell und dunkel mit 100 und 150 % (`build/jfnetworker/sessions_*.png`) |
| `model.uis`, Gruppe `ide` | worauf die IDE baut (siehe [IDE](#ide)): ein Dienst aus einer Datei mit Importen neben ihr und `config.file`, `source` und `file` zugleich abgelehnt, eine fehlende Datei und eine, die sich nicht übersetzen lässt (Fehler, Status, `script errors`, Log, das Problem `ScriptFile`), Speichern ändert den laufenden Dienst nicht, ein Neustart liest die Datei, das Umwandeln eines eingebetteten Skripts als ein Schritt mit Rückgängig und Wiederholen (die Disk zählt weiter), Speichern, neue Datei und Ordner, Umbenennen, Löschen, ein Dienst zu einer Datei, Ersetzen in zwei Dateien als ein Schritt, `jfs --test` (Bericht, Ergebnisse, Exit-Code, eine Datei mit Fehler, die Zeitgrenze, ein Port für den nächsten Test), Determinismus, ein altes Projekt mit `source` unverändert, Suchen und Ersetzen mit ganzen Wörtern |
| `ide_explorer.uis` | der Explorer der IDE auf `fixtures/ide.jfnet`: der Baum mit `/root` offen und dem eingebetteten Skript, eine neue Datei über das Flyout, ihr Tab und ihre Zeile mit „●“, Strg+S, F2 benennt um (der Tab geht mit), Entf löscht, Rückgängig des Projekts, ein neuer Ordner, ein Entwurf übersteht seinen geschlossenen Tab, Alle speichern, ein rückgängig gemachtes Speichern im Entwurf, das eingebettete Skript im Tab, „In Datei umwandeln“ (Tab und Dienst gehen mit, die Disk zählt weiter) und sein Rückgängig |
| `ide_language.uis` | die Sprachdienste über Dateien hinweg: Vervollständigung von `lib.` mit dem Doku-Kommentar, Signaturhilfe einer Funktion des Moduls, F12 öffnet `lib.jfs` an der Deklaration, die Gliederung (ein Klick), F2 im Flyout und Rückgängig, Probleme mit einem Klick zur Stelle |
| `ide_search.uis` | Suchen in der Datei (Strg+F) und in Dateien (Strg+Umschalt+F mit dem Wort an der Schreibmarke, die Treffer, ein Klick öffnet die Datei), ganze Wörter, Alle ersetzen (abgelehnt mit ungespeicherten Änderungen, dann ein Schritt), Rückgängig im Menü Bearbeiten |
| `ide_run.uis` | Ausführen: `hallo.jfs` als Prozess im Terminal, `server.jfs` als Dienst `web` mit seiner Ausgabe und einer Anfrage von PC1, Speichern im Lauf mit dem Hinweis und „Neu starten“, Stoppen, Ausführen, der Reiter Dienste |
| `ide_debug.uis` | der Debugger: F9 setzt einen Haltepunkt, eine Anfrage von PC1 hält `web` an (die Datei an der Zeile, Ort und Grund, Aufrufliste, Variablen), F10 zur nächsten Zeile mit neuer Variable, ein Ausdruck, F5 setzt fort und PC1 bekommt die Antwort, ein bedingter Haltepunkt (ein Syntaxfehler abgelehnt, er hält erst bei der dritten Anfrage), Alle Haltepunkte entfernen |
| `ide_tests.uis` | die Tests: Strg+Umschalt+T auf `test_lib.jfs`, der Bereich Tests mit einem bestandenen und einem gescheiterten Test (beide Werte von `assert_eq`), ein Klick zur Zeile, der Bericht im Terminal, nach der Korrektur beide bestanden |
| `ide_shots.uis` | die Bilder der IDE (ein Halt in `server.jfs` mit dem Debugger) hell und dunkel mit 100 und 150 %, normal und maximiert, klein und in hohem Kontrast (`build/jfnetworker/ide_shots_*.png`) |
| `ide_rtl.uis` | die IDE gespiegelt (`--rtl`): die Seitenleiste rechts, der Code links nach rechts, eine Datei aus dem Explorer, die Vervollständigung, Probleme; Bilder hell und dunkel |
| `scen_ide_web.uis` | das Golden-Szenario eines Webservers aus einer Datei (`scen/ide_web.scn`): Anfragen von PC1, eine angehängte Zeile wirkt erst nach `script restart`, `jfs --test`, ein fehlendes Modul und eine fehlende Datei als Fehler des Dienstes |
| `model.uis`, Gruppe `units` | Einheiten und Bereitstellen (siehe [Einheiten, Bereitstellen und Lasttest](#einheiten-bereitstellen-und-lasttest)): Unit-Dateien und ihre Fehler, jedes Wort von `systemctl` mit seinem Exit-Code, erzeugte Einheiten, Einschalten über einen Stromausfall und Speichern und Laden, die Neustart-Regeln mit Startgrenze, `Requires` und `After`, `journalctl` mit `-u`, `-n`, `-f` und Strg+C (auch in einer anderen Sitzung und im Hintergrund), eine Bereitstellung, deren Prüfung auf dem zweiten Host scheitert, eine, die durchgeht, Zurückrollen, Ablehnungen, Abbrechen, Determinismus (zwei Läufe, ein Klon mitten im Lauf, ein Nachspielen aus Keyframes), das Labor `05-microservices` ungelöst, gelöst und mit allen Hosts zugleich verdorben |
| `scen_deploy.uis` | das Golden-Szenario (`scen/deploy.scn`): `systemctl` und `journalctl` auf DEV1, eine Einheit, die scheitert und neu startet, eine Bereitstellung, die an WEB2 scheitert, eine, die durchgeht, ein Stromausfall, ein Zurückrollen |
| `ide_unit.uis` | F5 auf einer Datei, die als Einheit läuft: `systemctl start web` macht die Einheit zum Ziel der IDE, F5 speichert und startet sie neu (PC1 bekommt die neue Fassung, kein zweiter Dienst), Stoppen und Neu starten gehen über die Einheit |
| `deploy_ide.uis` | Bereitstellen aus der IDE auf die Gruppe Webserver: die Vorschläge des Bereichs, der Fortschritt je Host, das Scheitern auf WEB2 mit seinem Grund, der Lauf, der durchgeht, eine Liste von Hosts, eine fehlende Einheit, Einheit anlegen, Abbrechen |
| `load_ide.uis` | der Lasttest aus der IDE: ein Ziel ohne URL, offene Last mit Kacheln und Diagrammen, die Zusammenfassung, geschlossene Last, Stoppen, der Lauf im Panel Metriken |
| `deploy_shots.uis` | die Bilder von Bereitstellen und Lasttest hell und dunkel mit 100 und 150 %, klein und in hohem Kontrast (`build/jfnetworker/deploy_shots_*.png`) |
| `model.uis`, Gruppe `browser` | der Webbrowser (siehe [Webbrowser](#webbrowser)): URLs mit allen Beispielen aus RFC 3986 5.4, Prozent- und Formular-Kodierung, die Adresszeile, jede Fehlerart, die Regeln der Weiterleitungen und ihre Grenze, Typen, Formulare mit GET und POST gegen ein Skript (`req.query`, `req.form`), Dateien mit `"root"`, Verlauf, Fragmente, Anhalten, Strom aus unter einer Anfrage, Wiederholung beim Zurückspulen, Determinismus |
| `browser_pages.uis` | der Webbrowser auf dem Desktop von PC1 (`fixtures/web_site.jfnet`): Adresse tippen (Name über DNS), Seite mit Bild und Titel, Link und zurück (Knöpfe und Tasten), relative Links, Text, Markdown, Fragmente, ein Bild allein, Strg+L; eigene Seiten für einen unbekannten Namen, einen abgelehnten Port, keinen Weg, 404, 500, 503, zu viele Weiterleitungen und `https:`; Schließen verwirft den Zustand |
| `browser_forms.uis` | Formulare gegen das `jfs`-Skript `laden.jfs`: GET mit Query in der Adresse, Zurück bringt das Formular mit dem Eingegebenen, POST mit dem Knopf, Neu laden fragt vor dem zweiten Senden, ein Link mit Weiterleitung, Anhalten und Esc, die angehaltene Simulation, Tab in die Felder |
| `browser_requests.uis` | was der Webbrowser ins Netz schickt, an den Zahlen des Webservers im Reiter Dienste: Seite und Bild sind zwei Anfragen, Zurück und Vor keine, Neu laden fragt wieder (auch nach dem Bild); der Desktop kommt nach dem Schließen des Gerätefensters so zurück, wie er war |
| `browser_shots.uis` | die Bilder des Webbrowsers (Seite mit Bild, Text, Formular, die Frage vor dem erneuten Senden, Fehlerseite) hell und dunkel mit 100 und 150 % (`build/jfnetworker/browser_*.png`) |
| `model.uis`, Gruppe `pagescript` | die Seitenskripte ohne Fenster (`fixtures/web_scripts.jfnet`): Elemente finden, lesen, ändern, anlegen, Ereignisse mit Blase und Standardaktion, Zeitgeber auf einer gegebenen Uhr, `fetch` und `src` durch den HTTP-Client einer Engine, `location`, Fuel, Speicher, Tiefe und Grenzen des Dokuments, Endlosschleifen, Fehler mit Quelle, Zeile und Spalte, die Reihenfolge der Skripte, was eine Seite nicht erreicht, zweimal dasselbe Ergebnis |
| `browser_scripts.uis` | die Seitenskripte im Fenster auf dem Desktop von PC1: Klicks ändern die Seite (mit Titel), Eingaben, eine Liste, ein gestopptes und ein erlaubtes Absenden, die Uhr bei laufender und stehender Simulation, `fetch`, die Konsole mit Zeile und Spalte, eine kaputte Seite, die Endlosschleife lässt das Fenster bedienbar, Zurück, Vor und Neu laden, Zeitgeber warten auf einer Seite, die nicht zu sehen ist |
| `browser_script_shots.uis` | die Bilder der Seitenskripte (Zähler, Liste, Formular, Konsole mit Fehlern) hell und dunkel mit 100 und 150 % (`build/jfnetworker/script_*.png`) |
| `model.uis`, Gruppe `webdev` | B4 des Webbrowsers ohne Fenster (`fixtures/web_dev.jfnet`): Stylesheets und wie der Browser ihre Antworten nimmt, Cookies über das Netz mit jedem Attribut und auf der virtuellen Uhr, `HttpOnly` gegen `document.cookie`, `Secure`, `SameSite`, `Set-Cookie` auf Weiterleitungen und das Cookie des nächsten Sprungs, HTTPS mit jedem Zertifikatsfehler und dem Weitermachen, der Handshake auf dem Kabel und in den Worten des Inspektors, das Netzwerkprotokoll, der Dokumentbaum, zwei gleiche Läufe und eine Wiederholung über die Aufzeichnung |
| `browser_styles.uis` | Stylesheets im Fenster: zwei `<link>` und ein `<style>` in Dokumentreihenfolge (der Reiter Elemente sagt, was ein Element bekam), Zurück fragt nichts, Neu laden alles, eine Seite mit kaputten Stylesheets zeigt sich und die Konsole sagt warum |
| `browser_cookies.uis` | Cookies im Fenster: `Set-Cookie` mit jedem Attribut, die Tabelle der Seite, was die Dose ablehnt, `document.cookie` ohne `HttpOnly`, `fetch` trägt sie, eine Anmeldung, deren Weiterleitung das Cookie setzt, Löschen, was das Schließen des Browsers überlebt |
| `browser_https.uis` | HTTPS im Fenster: das Schloss und die Karte des Zertifikats, die Warnseite für jeden Fehler mit „Trotzdem fortfahren“ und dem Weg zurück, ein Port ohne TLS, die Weiterleitung von http nach https, was das Schließen vergisst |
| `browser_devtools.uis` | die Entwickleransicht: F12 und der Knopf, Netzwerk mit Köpfen und Inhalt, ein Protokoll je Dokument, Elemente lebendig unter einem Skript, Anhalten beendet auch `fetch`, die Konsole bleibt beim Wechsel der Skalierung am Ende |
| `browser_dev_shots.uis` | die Bilder von B4 (Seite mit Stylesheets, Warnung, Karte, die vier Reiter) hell und dunkel mit 100 und 150 % (`build/jfnetworker/dev_*.png`) |
| `scen_web_https.uis` | das Golden-Szenario von HTTPS (`scen/web_https.scn`): `curl` über TLS, der Handshake mit `-v`, jede Ablehnung und was `-k` daraus macht, ein Skript mit TLS, die Weiterleitung nach https, ein Port ohne TLS |
| `scen_web_site.uis` | das Golden-Szenario einer kleinen Website (`scen/web_site.scn`): `curl` holt, was der Browser holt: Seiten über den Namen, die Weiterleitung eines Ordners, Typen, eine fehlende Seite, ein Pfad aus dem Ordner hinaus, Formulare mit GET und POST, Weiterleitungen und ihre Grenze, eine gespeicherte Seite |
| `dot1x_panel.uis` | 802.1X im Fenster auf `fixtures/dot1x_lab.jfnet`: die Kabel der geschlossenen Ports heißen „nicht autorisiert“, das Feld „Anmeldung am Netz (802.1X)“ der IP-Konfiguration auf PC1 (ohne Identität ein Hinweis; EAP-MD5, der Port geht auf), PC2 mit falschem Passwort und EAP-PWD (bleibt zu), der RADIUS-Server im Inspektor eines Enterprise-Netzes, der Laptop im Unternehmens-WLAN |
| `mail_imap.uis` | das Programm E-Mail mit einem IMAP-Konto auf `fixtures/imap_mail.jfnet`: Kontotyp, Ordner und Nachrichten vom Server, Lesen setzt \Seen, Markieren, Löschen in den Papierkorb, ein neuer Ordner und sein Löschen, eine gesendete Nachricht landet in Gesendet |
| `dot1x_shots.uis` | die Bilder von X1g (die gelbe LED des geschlossenen Ports, das 802.1X-Feld, E-Mail mit IMAP-Konto) hell und dunkel mit 100 und 150 % (`build/jfnetworker/x1g_*.png`) |
| `scen_dot1x_imap.uis` | das Golden-Szenario 802.1X, WPA-Enterprise und IMAP Byte für Byte |
| `model.uis`, Gruppe `models` | die Modelle von Welle 17 bei der Arbeit: die Ports, die Bauform und das sysObjectID-Blatt jedes Modells, die alten Vorgabe-Modelle jeder Art, ein Ping durch jeden neuen Switch mit der Geschwindigkeit beider Ports in der Laufzeit, Routing über JF-S48Y und JF-S32C mit gerouteten Ports und VLAN-Interface, die serielle Leitung von JF-R2, ACLs und NAT auf JF-FW2 und JF-FW8, dass die Firewalls ohne Liste alles durchlassen wie ein Router, JF-S8 leitet weiter und hat keine Befehlszeile, der Repeater wiederholt alles und trägt zweimal 90 m (ein Kabel von 180 m bleibt aus), die Bridge lernt und behält Rahmen auf ihrer Seite, ein Laptop an JF-AP7 auf 6 GHz, Dateien, Portvorschläge und die Datei mit allen neuen Modellen |
| `model.uis`, Gruppe `hardware` | CPU und RAM gegen die Formeln: die Warteschlange allein gegen Erlang C (M/M/c, auf 6 %) und M/D/1 (auf 2 %), Sättigung X = Kerne · Tempo / Kosten auf 1 % über eine Lastreihe, M/D/1 über HTTP, der Gipfel des USL, die Sperre, zwei Dienste auf einem und auf zwei Hosts, lange Aufgaben in Stücken, langsame CPU und Speicherfresser, mehr RAM gegen Plattenlast, OOM-Kill mit 137 und Neustart durch die Einheit, Resize mit Ausfall und im Betrieb, jeder Dienst, Skripte, Konsole, Determinismus (zwei Läufe, Klon, Wiederholung aus Keyframes), die Schritte im Szenario, die Rückgängig-Schritte |
| `scen_hw_basic.uis` | das Golden-Szenario Hardware Byte für Byte (`fixtures/hw_basic.jfnet`): Sättigung, Resize mit Ausfall und im Betrieb, langsame CPU, Speicherfresser, OOM, `top`, `free`, `lscpu` |
| `hardware_panel.uis` | „Hardware simulieren“ mit seinem Rückgängig-Schritt, die Hardware-Karte eines Hosts (Annahmen, Eigene Angaben, Ablehnung, Resize mit Marke, im Betrieb), die neuen Fehler, die Zeile im Metriken-Panel |
| `hardware_shots.uis` | die Bilder dazu hell und dunkel, 100 und 150 % |
| `models_palette.uis` | die Palette mit den neuen Modellen: Kacheln in ihren Kategorien, JF-S8 per Ziehen platziert, Tooltips der Firewalls und von JF-AP7, die Suche nach Name, Beschreibung und Id; Bilder hell und dunkel bei 100 und 150 % (`build/jfnetworker/models_palette_*.png`) |
| `model.uis`, Gruppe `fwgui` | der Reiter Firewall: was die Formulare für jede Art von Regel und für NAT tippen und dass die Konsole es nimmt, was sie mit welchem Grund ablehnen, Regeln zurück in Zeilen und Felder, Verschieben mit Zählern und Bemerkungen, `Acls::probe`, der Test gegen eine Tabelle von Paketen, jedes auch wirklich gesendet (eingehend, ausgehend, ausgehend nach NAT), `run_lines_running` mit getippten und ungesicherten Listen, einem Rückgängig-Schritt und einer Ablehnung, die nichts ändert |
| `firewall_panel.uis` | der Reiter im Fenster auf `fixtures/nat_pat.jfnet`: Regel anlegen und binden, ein Ping wird abgewiesen und der Zähler steigt, Rückgängig, eine in der Konsole getippte Regel erscheint, Verschieben, Duplizieren, Bearbeiten, Löschen, Port-Weiterleitung bis zum Server, der Test, Ablehnungen mit Grund, ein Gerät ohne Routing und ohne Strom, die Ansicht in der Vergangenheit |
| `firewall_en.uis`, `firewall_shots.uis` | der Reiter auf Englisch; die Bilder hell, dunkel, schmal, 100 und 150 % (`build/f1/shots/fw_*.png`) |

Fixtures in `tests/fixtures/`: `units.jfnet` (DEV1 mit dem Webdienst als Dateien und seinen Einheiten, die
Gruppe Webserver), `office.jfnet` (ein kleines Büronetz mit jeder Kabelart),
`cables.jfnet` (Geräte ohne Kabel), `pdu_lab.jfnet` (DHCP, DNS-Zone `lab`, TCP-Echo),
`campus.jfnet` (die Abnahme), `lab_window.jfnet` (das Labor der Fenstertests: Cache,
Loadbalancer, zwei Webserver, drei KV-Knoten, Queue mit Worker, DNS, DHCP, ein Skript),
`stp_triangle.jfnet` und `rstp_triangle.jfnet` (drei Switches im Dreieck mit STP bzw. RSTP),
`wlan_office.jfnet` (zwei Access Points, ein Gastnetz, zwei Laptops und ein Smartphone; das
Szenario `wlan_office.scn` und die WLAN-Fensterskripte), `backup.jfnet` (drei Router, SRV1 mit
TFTP- und FTP-Server, PC1; das Szenario `backup.scn` und die Fensterskripte `files_*.uis`),
`phys_campus.jfnet` (die physische Ansicht: zwei Häuser, Serverraum mit zwei Racks, Büros auf
zwei Etagen, ein Labor 180 m entfernt, dessen PC4 mit 208 m Kupfer an SW1 hängt;
`phys_lengths.scn` und die `phys_*.uis`), `snmp_monitor.jfnet` (MON mit `snmptrapd`, R1, SW1
und R2 mit SNMP-Agenten, PC1 mit `snmpd`; `snmp_monitor.scn` und die `snmp_*.uis`), `hw_basic.jfnet` (ein Lastgenerator, zwei Webserver,
Hardware simuliert; `hw_basic.scn` und die `hardware_*.uis`) und die Netze
der Golden-Szenarien. Die
Bausteine aus `lib/` prüft `coretest` (Gruppen `netpkt`, `dhcp`, `dnsmsg`, `ntp`, `httpmsg`,
`rng`, `metrics`, `jmap`, `cmdtree`, `hashring`, `lru`, `stp`, `icmpv6`, `ospf`, `addrsel`,
für WLAN `crypto` und `netpkt`, für BGP `bgp`, für Dateien `tftp` und `ftp`, für SNMP `snmp`), die
Skriptsprache die Gruppe `script`.
Für Mail: `mail_domains.jfnet` (a.lab und b.lab mit je einem Mailserver und einem PC hinter R1,
DNS1 mit ihren MX-Einträgen; das Szenario `mail_domains.scn` und die Fensterskripte
`mail_*.uis`), die coretest-Gruppe `mail` (SMTP, POP3 und Nachrichten Byte für Byte), 802.1X,
WPA-Enterprise und IMAP (die selftest-Gruppen `dot1x` und `imap`, `scen_dot1x_imap.uis`, die
coretest-Gruppe `eap` mit den Vektoren von EAP-PWD und dem Codec von IMAP).
Für Gruppen und Hintergrundbild: `groups.jfnet` (zwei PCs und zwei Server an SW1, die Server mit
einem Kabel unter sich, dessen Masken nicht passen) und `bg_plan.png` (ein Grundriss,
160 × 120); die Rahmen des Diagramm-Widgets prüft `apps/diagramtest/tests/frames.uis`.
Für Logins und AAA: `aaa_radius.jfnet` (R1 und R2 melden ihre Konsolen über den RADIUS-Server
von SRV1 an; das Szenario `aaa_radius.scn`) und `aaa_login.jfnet` (R1 mit `login local` und
Banner, SRV1 mit `radius-server`, PC1; die Fensterskripte `login_console.uis`,
`telnet_session.uis`, `aaa_services.uis` und `aaa_shots.uis`), die coretest-Gruppe `radius`
(RADIUS und Telnet Byte für Byte, MD5 und HMAC-MD5 in `crypto`).
Für den Webbrowser: `web_site.jfnet` (WEB1 mit den Dateien aus `/var/www` und dem Laden als
Skript, DNS1 mit `web.lab`, PC1 mit dem Browser; das Szenario `web_site.scn` und die Fensterskripte
`browser_*.uis`) und `web_scripts.jfnet` (dasselbe Netz mit Seiten mit Skripten: Zähler, Liste,
Formular, Uhr, Daten über `fetch` von `api.jfs`, kaputte Skripte, eine Endlosschleife, die Adresse,
Klicks; die selftest-Gruppe `pagescript` und `browser_scripts.uis`, `browser_script_shots.uis`)
und `web_dev.jfnet` (ein Hofladen mit Stylesheets, Cookies aus `konto.jfs`, einem Zertifikat der
Lab-CA auf Port 443 und je einem mit Fehler auf den Ports 8441 bis 8444; die selftest-Gruppe `webdev`,
das Szenario `web_https.scn` und die Fensterskripte `browser_styles.uis`, `browser_cookies.uis`,
`browser_https.uis`, `browser_devtools.uis`, `browser_dev_shots.uis`).
Für die Shell: `shell_tour.jfnet` (PC1 und SRV1 mit einem Webserver an SW1, R1 als Gateway; das
Szenario `shell_tour.scn`, die selftest-Gruppe `shell` und die Fensterskripte `shell_*.uis`),
die coretest-Gruppe `pattern` (Platzhalter und reguläre Ausdrücke von `base::pattern`).
Für die Sitzungen: `sessions.jfnet` (R1 mit Leitungspasswort, R2 mit `login local` und einer
Minute Leerlaufzeit, ADMIN, SRV1 mit `telnetd` an SW1; das Szenario `sessions.scn` und die
Fensterskripte `sessions_terminal.uis`, `telnet_vty.uis` und `sessions_shots.uis`).
Für SSH und SNMPv3: `ssh_snmpv3.jfnet` (R1 mit Ed25519-Schlüssel, nur SSH, R2 mit RSA-Schlüssel, SW1 mit
Benutzern der Version 3 und einer Sicht, ADMIN, SRV1 mit `sshd` und `snmpd`; das Szenario `ssh_snmpv3.scn`
und die Fensterskripte `ssh_session.uis`, `snmpv3_mib.uis` und `ssh_shots.uis`), die coretest-Gruppe
`sshcore` (`netpkt::ssh` und `netpkt::snmpv3` gegen Vektoren von außen und gegen `netio::ssh` über TCP) und
`python tools/ssh_interop.py --core` (der OpenSSH-Client gegen den Kern, mit jeder Chiffre).

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
| | [`src/workspace`](src/workspace/mod.rs) | Fensterrahmen, Palette, Eigenschaften (`inspector/`: Gerät, Kabel, Projekt), Zeichenfläche (`canvas/`: Szene und Zeichnen, Werkzeuge, Verbinden, Kabel), Simulationsmodus ([`sim/`](src/workspace/sim/mod.rs): Steuerung, Animation, Ereignisse, Paket, PDUs und Export), Symbole und UI-Icons (`icons/`, erzeugt von `tools/vsym.py` und [`mkicons.py`](mkicons.py)), Terminal-Bereich und Konsolenfenster ([`terminal.rs`](src/workspace/terminal.rs)), Protokoll ([`logview.rs`](src/workspace/logview.rs)), Gerätefenster ([`devwin/`](src/workspace/devwin/mod.rs): Übersicht, Schnittstellen, Konfiguration, Firewall, Dienste, CLI, Desktop), die Dialoge der Einstellungen und der Tastenkürzel ([`settings.rs`](src/workspace/settings.rs)) |
| | [`src/doc`](src/doc/mod.rs) | das offene Projekt mit Rückgängig (`edits.rs`), Dateien und Dialoge (`file.rs`), der Simulator dahinter (`sim.rs`), Einstellungen, die die Konfiguration eines Geräts ändern (`config.rs`), die Konsolen der Geräte und die Meldungen des Simulators an sie ([`console.rs`](src/doc/console.rs)), Sicherungen und ihre Wiederherstellung ([`autosave.rs`](src/doc/autosave.rs)) |
| | [`src/sim_thread.rs`](src/sim_thread.rs) | Treiber mit eigenem Thread für Echtzeit und lange Läufe (nur nativ) |
| | [`src/selftest.rs`](src/selftest.rs), [`src/simtest.rs`](src/simtest.rs), [`src/l3test.rs`](src/l3test.rs), [`src/simmodetest.rs`](src/simmodetest.rs), [`src/svctest.rs`](src/svctest.rs), [`src/clitest.rs`](src/clitest.rs), [`src/tcptest.rs`](src/tcptest.rs), [`src/httptest.rs`](src/httptest.rs), [`src/consoletest.rs`](src/consoletest.rs), [`src/pdutest.rs`](src/pdutest.rs), [`src/labtest.rs`](src/labtest.rs), [`src/uitest.rs`](src/uitest.rs), [`src/campustest.rs`](src/campustest.rs), [`src/synthtest.rs`](src/synthtest.rs) | Selbsttests von Modell, Simulator, Befehlszeilen, TCP, HTTP, PDUs, des Labors, dem, worauf die Konsolen bauen, der Oberfläche ohne Fenster (Kontrast, Einstellungen, Sicherungen, Katalog), des Campus und der synthetischen Projekte (`--selftest`) |
| | [`src/seriestest.rs`](src/seriestest.rs) | Selbsttest der Zeitreihen und der Erwartungen über Zeiträume (Gruppe `series`) |
| | [`src/workspace/metrics.rs`](src/workspace/metrics.rs), [`canvas/lab.rs`](src/workspace/canvas/lab.rs), [`inspector/faults.rs`](src/workspace/inspector/faults.rs), [`src/doc/faults.rs`](src/doc/faults.rs), [`src/labuitest.rs`](src/labuitest.rs) | das Labor im Fenster (U6a): der Bereich Metriken, Dienst-Pillen und Sparklines, Fehler aus dem Inspektor als Rückgängig-Schritte, ihr Selbsttest (Gruppe `labui`; siehe [Das Labor im Fenster](#das-labor-im-fenster)) |
| Texte | [`crates/jfn_model/msg.rs`](crates/jfn_model/msg.rs), [`src/locales/engine.tsv`](src/locales/engine.tsv), [`src/locales/ui.tsv`](src/locales/ui.tsv), [`src/msgtest.rs`](src/msgtest.rs) | die deutschen Texte von Modell und Simulator als Vorlagen mit Argumenten (`Msg`, `msg!`; `text_with` übersetzt), die des Fensters (`t()`, `tf()`, die Befehle und Menüs), ihre englische Fassung und deren Selbsttest; Vollständigkeit: `python tools/i18n_check.py apps/JFNetworker crates` |
| Tests | [`tests/`](tests) | Skripte, Fixtures (`fixtures/*.jfnet`), Szenarien mit Golden-Berichten (`scen/*.scn`, `scen/*.report`) |
| Skripte | [`crates/jfn_sim/scriptsvc.rs`](crates/jfn_sim/scriptsvc.rs), [`cli/scripts.rs`](crates/jfn_sim/cli/scripts.rs), [`src/scripttest.rs`](src/scripttest.rs) | Skripte als Dienste in den Hosts: Host-API, Anweisungen als Rechenzeit, der Befehl `script`, ihr Selbsttest (siehe [Skripte in Hosts](#skripte-in-hosts)) |
| Labore | [`crates/jfn_model/lab.rs`](crates/jfn_model/lab.rs), [`crates/jfn_sim/goal.rs`](crates/jfn_sim/goal.rs), [`src/workspace/task.rs`](src/workspace/task.rs), [`src/workspace/devwin/svcedit.rs`](src/workspace/devwin/svcedit.rs), [`assets/labs`](assets/labs), [`src/tasktest.rs`](src/tasktest.rs) | die Aufgabe eines Labors in der Projektdatei, das Urteil über eine Erwartung (Runner und Labor) und die Ziele über einen Lauf, das Panel „Aufgabe“ mit der Liste der Labore, die Einstellungen der Dienste als JSON, die vier Labore, ihr Selbsttest (siehe [Labore und Aufgaben](#labore-und-aufgaben)) |
| IDE | [`src/workspace/ide/`](src/workspace/ide/mod.rs) ([`explorer.rs`](src/workspace/ide/explorer.rs), [`editor.rs`](src/workspace/ide/editor.rs), [`lower.rs`](src/workspace/ide/lower.rs), [`debug.rs`](src/workspace/ide/debug.rs), [`search.rs`](src/workspace/ide/search.rs)), [`src/workspace/devwin/scripts.rs`](src/workspace/devwin/scripts.rs), [`src/workspace/devwin/scriptls.rs`](src/workspace/devwin/scriptls.rs), [`src/doc/ide.rs`](src/doc/ide.rs), [`src/doc/scripts.rs`](src/doc/scripts.rs), [`crates/jfn_sim/cli/jfstest.rs`](crates/jfn_sim/cli/jfstest.rs), [`src/idetest.rs`](src/idetest.rs), [`src/scriptedtest.rs`](src/scriptedtest.rs) | die IDE eines Hosts (Explorer, Tabs, Sprachdienste, Suchen in Dateien, Ausführen, Debugger, Tests) und ihr Reiter im Gerätefenster, die Schritte auf Dateien und Skripten im Projekt mit Rückgängig und Vorlagen, `jfs --test` in der Shell, die Selbsttests (Gruppen `ide`, `scripted`; siehe [IDE](#ide)) |
| Spanning Tree | [`crates/jfn_sim/stp.rs`](crates/jfn_sim/stp.rs), [`stp98.rs`](crates/jfn_sim/stp98.rs), [`rstp.rs`](crates/jfn_sim/rstp.rs), [`cli/stp.rs`](crates/jfn_sim/cli/stp.rs), [`lib/netpkt/stp.rs`](../../lib/netpkt/stp.rs), [`src/stptest.rs`](src/stptest.rs) | STP (802.1D-1998) und RSTP (802.1D-2004) auf den Switches, Einstellungen und Befehle, BPDUs, ihr Selbsttest (siehe [Spanning Tree](#spanning-tree)) |
| Routing | [`crates/jfn_sim/rip.rs`](crates/jfn_sim/rip.rs), [`cli/rip.rs`](crates/jfn_sim/cli/rip.rs), [`src/riptest.rs`](src/riptest.rs), [`lib/netpkt/rip.rs`](../../lib/netpkt/rip.rs) | RIP als Dienst der Router und L3-Switches, seine Befehle und Anzeigen, sein Selbsttest, die Nachrichten Byte für Byte (siehe [Routing mit RIP](#routing-mit-rip)) |
| NAT und Zugriffslisten | [`crates/jfn_sim/acl.rs`](crates/jfn_sim/acl.rs), [`nat.rs`](crates/jfn_sim/nat.rs), [`cli/acl.rs`](crates/jfn_sim/cli/acl.rs), [`cli/nat.rs`](crates/jfn_sim/cli/nat.rs), [`lib/netpkt/nat.rs`](../../lib/netpkt/nat.rs), [`src/nattest.rs`](src/nattest.rs) | Zugriffslisten und Adressübersetzung der IOS-artigen Geräte, ihre Befehle, das Umschreiben der Pakete mit ihren Prüfsummen (Library), ihr Selbsttest (siehe [NAT und Zugriffslisten](#nat-und-zugriffslisten)) |
| IPv6 | [`crates/jfn_sim/ip6.rs`](crates/jfn_sim/ip6.rs), [`nd.rs`](crates/jfn_sim/nd.rs), [`l3v6.rs`](crates/jfn_sim/l3v6.rs), [`jobs6.rs`](crates/jfn_sim/jobs6.rs), [`trace6.rs`](crates/jfn_sim/trace6.rs), [`cli/ip6.rs`](crates/jfn_sim/cli/ip6.rs), [`cli/linux6.rs`](crates/jfn_sim/cli/linux6.rs), [`cli/print6.rs`](crates/jfn_sim/cli/print6.rs), [`cli/names.rs`](crates/jfn_sim/cli/names.rs), [`lib/netpkt/icmpv6.rs`](../../lib/netpkt/icmpv6.rs), [`lib/netpkt/ipv6.rs`](../../lib/netpkt/ipv6.rs), [`lib/netpkt/addrsel.rs`](../../lib/netpkt/addrsel.rs), [`src/ipv6test.rs`](src/ipv6test.rs), [`src/ipv6tcptest.rs`](src/ipv6tcptest.rs) | der IPv6-Teil der Geräte (Adressen, Neighbor Cache, Router, Routen), Neighbor Discovery, DAD und SLAAC, Weiterleiten und ICMPv6, Ping und Traceroute, die Notizen, die Befehle beider Befehlszeilen, Namen an beiden Befehlszeilen (`dig`, `host`, `nslookup`, `ip host`, `ip name-server`), ICMPv6 und Neighbor Discovery Byte für Byte, die Fragmente und die Adressauswahl nach RFC 6724 (Library), die Selbsttests (siehe [IPv6](#ipv6)) |
| OSPF | [`crates/jfn_sim/ospf.rs`](crates/jfn_sim/ospf.rs), [`cli/ospf.rs`](crates/jfn_sim/cli/ospf.rs), [`src/ospftest.rs`](src/ospftest.rs), [`lib/netpkt/ospf.rs`](../../lib/netpkt/ospf.rs) | OSPF als Dienst der Router und L3-Switches, seine Befehle und Anzeigen, sein Selbsttest, die Pakete und LSAs Byte für Byte (siehe [Routing mit OSPF](#routing-mit-ospf)) |
| BGP | [`crates/jfn_sim/bgp.rs`](crates/jfn_sim/bgp.rs), [`bgptrace.rs`](crates/jfn_sim/bgptrace.rs), [`cli/bgp.rs`](crates/jfn_sim/cli/bgp.rs), [`src/bgptest.rs`](src/bgptest.rs), [`lib/netpkt/bgp.rs`](../../lib/netpkt/bgp.rs) | BGP als Dienst der Router, L3-Switches und der Firewall, seine Notizen, Befehle und Anzeigen, sein Selbsttest, die Nachrichten Byte für Byte (siehe [Routing mit BGP](#routing-mit-bgp)) |
| Aufgaben mit Bewertung | [`crates/jfn_model/lab.rs`](crates/jfn_model/lab.rs), [`crates/jfn_sim/assess.rs`](crates/jfn_sim/assess.rs), [`src/workspace/assess.rs`](src/workspace/assess.rs), [`src/workspace/author.rs`](src/workspace/author.rs), [`src/doc/assess.rs`](src/doc/assess.rs), [`assets/assess`](assets/assess), [`src/assesstest.rs`](src/assesstest.rs) | das Format einer Aufgabe, das Bewerten ihrer Kriterien, die Prüfung `reaches`, das Panel mit Punkten, Zeit und Abgeben, der Bereich zum Erstellen mit seinen Rückgängig-Schritten, die drei Aufgaben, ihr Selbsttest (siehe [Aufgaben mit Bewertung](#aufgaben-mit-bewertung)) |
| Physische Ansicht | [`crates/jfn_model/phys.rs`](crates/jfn_model/phys.rs), [`crates/jfn_sim/phys.rs`](crates/jfn_sim/phys.rs), [`src/workspace/phys/`](src/workspace/phys/mod.rs) (`scene.rs`, `paint.rs`, `props.rs`), [`src/doc/phys.rs`](src/doc/phys.rs), [`src/phystest.rs`](src/phystest.rs) | die Orte, wo die Geräte stehen, Kabellängen und die Grenzen der Medien (Modell), Kabel, die zu lang sind oder deren Optiken nicht passen, im Simulator mit Notizen, Protokollzeilen und Problemen, die Ansicht mit ihren Plänen und dem Inspektor, die Änderungen als Rückgängig-Schritte, ihr Selbsttest (siehe [Physische Ansicht](#physische-ansicht)) |
| WLAN | [`crates/jfn_model/wlan.rs`](crates/jfn_model/wlan.rs), [`crates/jfn_sim/wlan.rs`](crates/jfn_sim/wlan.rs), [`wlankey.rs`](crates/jfn_sim/wlankey.rs), [`wlantrace.rs`](crates/jfn_sim/wlantrace.rs), [`cli/wlan.rs`](crates/jfn_sim/cli/wlan.rs), [`src/doc/wlan.rs`](src/doc/wlan.rs), [`src/workspace/canvas/wlan.rs`](src/workspace/canvas/wlan.rs), [`src/workspace/inspector/wlan.rs`](src/workspace/inspector/wlan.rs), [`src/wlantest.rs`](src/wlantest.rs), [`lib/netpkt/eapol.rs`](../../lib/netpkt/eapol.rs), [`lib/crypto`](../../lib/crypto/lib.rs) (`sha1.rs`, `keywrap.rs`, `cmac.rs`) | die WLAN-Einstellungen im Projekt, Stationen und Access Points im Simulator mit Funk, Schlüsseln und Notizen, `iw` und `show dot11`, die Einstellungen als Rückgängig-Schritte, Funklinien, Reichweiten und Balken, der Inspektor, der Selbsttest, EAPOL Byte für Byte und die Kryptografie von WPA2 und WPA3 (siehe [WLAN](#wlan)) |
| Dateien, TFTP und FTP | [`crates/jfn_model/files.rs`](crates/jfn_model/files.rs), [`crates/jfn_sim/files.rs`](crates/jfn_sim/files.rs), [`tftpd.rs`](crates/jfn_sim/tftpd.rs), [`tftpc.rs`](crates/jfn_sim/tftpc.rs), [`ftpd.rs`](crates/jfn_sim/ftpd.rs), [`ftpc.rs`](crates/jfn_sim/ftpc.rs), [`xfertrace.rs`](crates/jfn_sim/xfertrace.rs), [`cli/files.rs`](crates/jfn_sim/cli/files.rs), [`src/workspace/devwin/files.rs`](src/workspace/devwin/files.rs), [`src/filestest.rs`](src/filestest.rs), [`lib/netpkt/tftp.rs`](../../lib/netpkt/tftp.rs), [`lib/netpkt/ftp.rs`](../../lib/netpkt/ftp.rs) | die Dateien eines Geräts im Projekt und im Simulator, TFTP- und FTP-Server und ihre Clients an beiden Befehlszeilen, die Notizen, die Befehle für Dateien, der Reiter Dateien, der Selbsttest, TFTP und FTP Byte für Byte (siehe [Dateien, TFTP und FTP](#dateien-tftp-und-ftp)) |
| Gruppen und Hintergrundbild | [`crates/jfn_model/group.rs`](crates/jfn_model/group.rs), [`background.rs`](crates/jfn_model/background.rs), [`src/workspace/canvas/groups.rs`](src/workspace/canvas/groups.rs), [`backdrop.rs`](src/workspace/canvas/backdrop.rs), [`src/workspace/inspector/group.rs`](src/workspace/inspector/group.rs), [`src/doc/groups.rs`](src/doc/groups.rs), [`src/groupstest.rs`](src/groupstest.rs) | die Gruppen der logischen Zeichenfläche und ihr Hintergrundbild: Modell und Datei, die Szene mit Rahmen, Clustern und ihren Linien, das Zeichnen, der Inspektor, die Rückgängig-Schritte, ihr Selbsttest (siehe [Gruppen und Hintergrundbild](#gruppen-und-hintergrundbild)) |
| Zurückspulen | [`crates/jfn_sim/keyframe.rs`](crates/jfn_sim/keyframe.rs), [`crates/jfn_sim/driver.rs`](crates/jfn_sim/driver.rs), [`src/doc/sim.rs`](src/doc/sim.rs), [`src/workspace/sim/mod.rs`](src/workspace/sim/mod.rs), [`src/keyframetest.rs`](src/keyframetest.rs) | Keyframes in einer Speichergrenze, die Eingaben dazwischen, die Wiederholung bis zu einem Zeitpunkt und ihre Prüfsummen, das Fortsetzen von dort (Simulator); der Zustand von damals im Fenster, Konsolen in der Vergangenheit, „Von hier fortsetzen“ mit Rückfrage; ihr Selbsttest (siehe [Zurückspulen](#zurückspulen)) |
| SNMP | [`crates/jfn_sim/snmpd.rs`](crates/jfn_sim/snmpd.rs), [`snmpmib.rs`](crates/jfn_sim/snmpmib.rs), [`snmptrapd.rs`](crates/jfn_sim/snmptrapd.rs), [`snmptrace.rs`](crates/jfn_sim/snmptrace.rs), [`cli/snmp.rs`](crates/jfn_sim/cli/snmp.rs), [`src/workspace/devwin/mib.rs`](src/workspace/devwin/mib.rs), [`src/snmptest.rs`](src/snmptest.rs), [`lib/netpkt/ber.rs`](../../lib/netpkt/ber.rs), [`lib/netpkt/snmp.rs`](../../lib/netpkt/snmp.rs), [`lib/netpkt/mib.rs`](../../lib/netpkt/mib.rs) | der Agent mit seiner Sicht auf das Gerät, der Trap-Empfänger, die Notizen, die IOS-artigen Befehle und die Werkzeuge der Shell, der MIB-Browser, der Selbsttest, BER, die Nachrichten von SNMPv1 und v2c und die kleine MIB (Library; siehe [SNMP](#snmp)) |
| Shell | [`crates/jfn_sim/cli/sh.rs`](crates/jfn_sim/cli/sh.rs), [`shrun.rs`](crates/jfn_sim/cli/shrun.rs), [`coreutils.rs`](crates/jfn_sim/cli/coreutils.rs), [`man.rs`](crates/jfn_sim/cli/man.rs), [`cli/files.rs`](crates/jfn_sim/cli/files.rs), [`lib/base/pattern.rs`](../../lib/base/pattern.rs), [`src/shelltest.rs`](src/shelltest.rs) | die Shell einer Sitzung (Arbeitsverzeichnis, Variablen, Verlauf, Jobs), ihre Grammatik, die Werkzeuge, Hilfe und Handbuchseiten, Prozesse mit `jfs`, Ordner in `flash:`, Platzhalter und reguläre Ausdrücke, ihr Selbsttest (siehe [Die Shell: Ordner, Werkzeuge, Prozesse](#die-shell-ordner-werkzeuge-prozesse)) |
| Module und Debugger | [`lib/script/module.rs`](../../lib/script/module.rs), [`lib/script/debug.rs`](../../lib/script/debug.rs), [`crates/jfn_sim/scriptsvc.rs`](crates/jfn_sim/scriptsvc.rs) (`Host::load`, `DebugOp`), [`src/scriptmodtest.rs`](src/scriptmodtest.rs) | `import()` aus den Dateien des Geräts, Anhalten und Schritte eines Skript-Dienstes in virtueller Zeit, ihr Selbsttest (siehe [Skripte in Hosts](#skripte-in-hosts)) |
| Desktop der Hosts | [`src/workspace/devwin/desktop.rs`](src/workspace/devwin/desktop.rs), [`progs.rs`](src/workspace/devwin/progs.rs), [`editor.rs`](src/workspace/devwin/editor.rs), [`filemgr.rs`](src/workspace/devwin/filemgr.rs), [`sysmon.rs`](src/workspace/devwin/sysmon.rs), [`crates/jfn_sim/procs.rs`](crates/jfn_sim/procs.rs), [`src/desktoptest.rs`](src/desktoptest.rs) | der Desktop im Gerätefenster eines Hosts: Programme in Fenstern (die Fensterfläche von `lib/ui`), Symbole, Taskleiste mit Startmenü, Uhr und Vollbild; Terminal, Dateimanager, Texteditor, IP-Konfiguration, Webbrowser, E-Mail, MIB-Browser, Systemmonitor, Einstellungen; die Prozesse und die CPU-Zeit je Sekunde aus dem Simulator |
| Einheiten und Bereitstellen | [`crates/jfn_model/unit.rs`](crates/jfn_model/unit.rs), [`crates/jfn_sim/units.rs`](crates/jfn_sim/units.rs), [`deploy.rs`](crates/jfn_sim/deploy.rs), [`cli/units.rs`](crates/jfn_sim/cli/units.rs), [`src/workspace/ide/deploy.rs`](src/workspace/ide/deploy.rs), [`load.rs`](src/workspace/ide/load.rs), [`src/unitstest.rs`](src/unitstest.rs) | Unit-Dateien, `systemctl` und `journalctl`, Neustart-Regeln, die rollende Bereitstellung als Dienst der Engine, die Bereiche Bereitstellen und Lasttest der IDE; Selbsttest-Gruppe `units` |
| Webbrowser | [`src/workspace/devwin/browser.rs`](src/workspace/devwin/browser.rs) (das Programm), [`src/workspace/devwin/pagescript.rs`](src/workspace/devwin/pagescript.rs) (Seitenskripte: der Host der VM, die Konsole), [`src/workspace/devwin/devtools.rs`](src/workspace/devwin/devtools.rs) (die Entwickleransicht), [`crates/jfn_sim/webc.rs`](crates/jfn_sim/webc.rs) (Anfragen und Weiterleitungen in der Engine), [`crates/jfn_sim/tlssim.rs`](crates/jfn_sim/tlssim.rs) (Zertifikate des Labors, TLS-Notizen), [`lib/netpkt/uri.rs`](../../lib/netpkt/uri.rs) (URLs nach RFC 3986), [`lib/netpkt/cookie.rs`](../../lib/netpkt/cookie.rs) (Cookies nach RFC 6265), [`lib/netpkt/tls.rs`](../../lib/netpkt/tls.rs) und [`lib/netpkt/x509.rs`](../../lib/netpkt/x509.rs) (TLS 1.3 ohne I/O) |
| SSH | [`lib/netpkt/ssh.rs`](../../lib/netpkt/ssh.rs), [`crates/jfn_sim/sshd.rs`](crates/jfn_sim/sshd.rs), [`sshtrace.rs`](crates/jfn_sim/sshtrace.rs), [`cli/ssh.rs`](crates/jfn_sim/cli/ssh.rs), [`src/sshtest.rs`](src/sshtest.rs) | das Protokoll ohne I/O, Client und Server auf den Geräten, Schlüssel als Dateien, `known_hosts`, `authorized_keys`, `scp` |
| SNMPv3 | [`lib/netpkt/snmpv3.rs`](../../lib/netpkt/snmpv3.rs), [`crates/jfn_sim/snmpd.rs`](crates/jfn_sim/snmpd.rs), [`cli/snmp.rs`](crates/jfn_sim/cli/snmp.rs), [`src/workspace/devwin/mib.rs`](src/workspace/devwin/mib.rs), [`src/snmpv3test.rs`](src/snmpv3test.rs) | die Nachricht, USM, VACM, die Reports, die Werkzeuge, der MIB-Browser |
| Sitzungen und Telnet-Server | [`crates/jfn_sim/cli/ses.rs`](crates/jfn_sim/cli/ses.rs), [`telnetd.rs`](crates/jfn_sim/telnetd.rs), [`cli/aaa.rs`](crates/jfn_sim/cli/aaa.rs), [`src/doc/console.rs`](src/doc/console.rs), [`src/workspace/terminal.rs`](src/workspace/terminal.rs), [`src/sessiontest.rs`](src/sessiontest.rs) | mehrere Befehlszeilen je Gerät (Konsole, lokale Terminals, Telnet), der Telnet-Server der vty-Leitungen und `telnetd`, `show users`, `who`, die Sitzungen im Terminal-Panel |
| 802.1X und WPA-Enterprise | [`crates/jfn_sim/dot1x.rs`](crates/jfn_sim/dot1x.rs), [`supp.rs`](crates/jfn_sim/supp.rs), [`dot1xtrace.rs`](crates/jfn_sim/dot1xtrace.rs), [`cli/dot1x.rs`](crates/jfn_sim/cli/dot1x.rs), [`lib/eapm`](../../lib/eapm/lib.rs), [`lib/netpkt/eap.rs`](../../lib/netpkt/eap.rs), [`lib/crypto/eappwd.rs`](../../lib/crypto/eappwd.rs) |
| IMAP | [`crates/jfn_sim/imapd.rs`](crates/jfn_sim/imapd.rs), [`mailbox.rs`](crates/jfn_sim/mailbox.rs), [`mailc.rs`](crates/jfn_sim/mailc.rs), [`lib/netpkt/imap.rs`](../../lib/netpkt/imap.rs) |
| Firewall-Reiter | [`src/workspace/devwin/firewall.rs`](src/workspace/devwin/firewall.rs), [`crates/jfn_sim/acl.rs`](crates/jfn_sim/acl.rs) (`Acls::probe`), [`src/doc/console.rs`](src/doc/console.rs) (`run_lines_running`), [`src/fwguitest.rs`](src/fwguitest.rs) | Regeln, NAT und Test im Gerätefenster als Formulare über den Befehlen, sein Selbsttest (siehe [NAT und Zugriffslisten](#nat-und-zugriffslisten)) |
| Startseite | [`src/workspace/startpage.rs`](src/workspace/startpage.rs), [`assets/templates`](assets/templates), [`lib/ui/action_card.rs`](../../lib/ui/action_card.rs), [`src/startpagetest.rs`](src/startpagetest.rs) | die Seite statt Werkzeugleiste, Dock und Statusleiste mit Neu, Zuletzt und Lernen, der Dialog Projektinfo, die fünf Vorlagen, die Karte aus der Bibliothek (getestet in `apps/cardtest`), ihr Selbsttest (Gruppe `startpage`) |
| Hardware der Geräte | [`crates/jfn_sim/res.rs`](crates/jfn_sim/res.rs), [`crates/jfn_model/catalog.rs`](crates/jfn_model/catalog.rs), [`src/workspace/inspector/hardware.rs`](src/workspace/inspector/hardware.rs), [`src/hardwaretest.rs`](src/hardwaretest.rs) |

### Gerätekatalog

Herstellerneutrale, moderne Modelle: PC (2.5G), Laptop, Smartphone und Tablet (WLAN 802.11ax mit 6 GHz),
Drucker, IP-Telefon und IP-Kamera (PoE), IoT-Sensor, Smart-TV; Rack- und Tisch-Switches, Filial-,
Edge- und Core-Router (bis 100G, serielle WAN-Ports), L3-Switches (1G und 10GBASE-T mit 100G QSFP28,
Leaf und Spine), Heimrouter mit WLAN, Hub, Repeater, Bridge; Firewalls; WLAN-Access-Points (802.11ax
und 802.11be, 6 GHz), Modem/ONT, Kabelmodem (Koax), DSL-Modem (RJ11), Internet-Wolke; Rack- und Tower-Server; Loadbalancer, DB-Shard, Cache,
Message Queue, Lastgenerator, Container-Host. Netzwerkgeräte tragen IOS-Interface-Namen
(`GigabitEthernet1/0/1`, Abkürzungen wie `gi1/0/1` gehen überall), Hosts Linux-Namen (`eth0`,
`wlan0`).

| Modell | Art | Ports |
|---|---|---|
| JF-S8 | Tisch-Switch, unmanaged: keine Befehlszeile, keine Adresse | 8× 1G (`port0` bis `port7`) |
| JF-S8P | Kompakt-Switch auf dem Tisch | 8× 1G PoE+, 2× 10G SFP+, Konsole |
| JF-S24 | Access-Switch | 24× 1G, 4× 10G SFP+, Konsole |
| JF-S24P | Access-Switch | 24× 1G PoE+, 4× 10G SFP+, Konsole |
| JF-S48P | Access-Switch | 48× 1G PoE+, 4× 10G SFP+, Konsole |
| JF-S24M | Multi-Gigabit-Switch | 24× 2.5G PoE++, 4× 25G SFP28, Konsole |
| JF-S24L3 | L3-Switch | 24× 1G, 4× 10G SFP+, Konsole |
| JF-S48X | L3-Switch im Rechenzentrum | 48× 10GBASE-T, 6× 100G QSFP28, Konsole |
| JF-S48Y | L3-Switch, Leaf | 48× 25G SFP28, 8× 100G QSFP28, Konsole |
| JF-S32C | L3-Switch, Spine | 32× 100G QSFP28, Konsole |
| JF-R2 | Filial-Router | 2× 1G, 2× Seriell, Konsole |
| JF-R4 | Edge-Router | 4× 1G, 2× 10G SFP+, 2× Seriell, Konsole |
| JF-R8 | Core-Router | 1× 1G, 8× 10G SFP+, 2× 100G QSFP28, Konsole |
| Heimrouter | Router mit WLAN und NAT, Linux-Befehlszeile | 1G WAN, 4× 1G LAN, Funk |
| Hub | Schicht 1, ohne Befehlszeile | 8× 100M (`port0` bis `port7`) |
| Repeater | Schicht 1 wie der Hub, ohne Befehlszeile | 2× 1G (`port0`, `port1`) |
| Bridge | Schicht 2, lernt Adressen, ohne Befehlszeile | 2× 1G (`port0`, `port1`) |
| JF-FW | Firewall | 8× 1G, 2× 10G SFP+, Konsole |
| JF-FW2 | kleine Firewall auf dem Tisch | 1× 1G WAN (`Gi0/0`), 4× 1G LAN (`Gi0/1` bis `Gi0/4`), Konsole |
| JF-FW8 | Firewall im Rechenzentrum | 8× 10G SFP+, 4× 100G QSFP28, Konsole |
| JF-AP6E | Access Point 802.11ax | 1× 2.5G (PoE-gespeist), 3× Funk, Konsole |
| JF-AP7 | Access Point 802.11be | 1× 5G (PoE-gespeist), 3× Funk, Konsole |

Die Modelle haben feste Ports; Steckplätze und Module kommen später. Was neu dazukam, verhält sich so:

- **Die Firewalls sind heute Router mit Access-Listen und NAT.** Ohne Liste lassen sie alles durch,
  auch was von außen beginnt; ihre Titel sagen das. Listen und NAT lassen sich im Reiter Firewall des
  Gerätefensters pflegen (siehe [NAT und Zugriffslisten](#nat-und-zugriffslisten)). Zonen und
  zustandsbehaftetes Filtern kommen mit Meilenstein F2.
- **PoE ist bisher nur eine Angabe am Port** (PoE+, PoE++, PoE-gespeist): Es gibt kein Leistungsbudget.
- **Der Repeater** wiederholt jeden Rahmen an den anderen Port und verlängert so eine Kupferstrecke.
  Die Grenze gilt je Kabel: zweimal 90 m gehen, zweimal 120 m nicht.
- **Die Bridge** lernt die Adressen ihrer beiden Seiten und lässt einen Rahmen, dessen Ziel auf derselben
  Seite liegt, nicht hinüber.
- **JF-AP7** sendet auf 2,4, 5 und 6 GHz mit den Raten von 802.11be (1,3, 5,7 und 11,5 Gbit/s). Die
  Stationen im Katalog sprechen 802.11ax, also begrenzt ihr Funkteil die Rate (ein Laptop bleibt bei
  höchstens 2,4 Gbit/s).

### Kabel

`cable_kind` nennt das richtige Kabel für zwei Ports: Kupfer gerade oder gekreuzt (nach MDI /
MDI-X; mit Auto-MDI-X funktionieren beide), Glasfaser für SFP/SFP+/SFP28 untereinander und
QSFP28 mit QSFP28, seriell, Konsolenkabel (Konsole an USB), Funk (WLAN-Gerät an Access Point)
und WAN-Leitungen, Telefonkabel (RJ11 an RJ11 oder an den WAN-Port des Providers) und
Koaxialkabel (Koax an Koax oder an den WAN-Port). Beim schnellen Verbinden wählt `suggest_ports` das Portpaar: Funk, wenn ein
Access Point beteiligt ist, sonst die schnellste Datenverbindung mit den kleinsten Portnummern.

Dazu kann der Nutzer selbst wählen, und `cable_fits` prüft, ob es steckt:

| Art (`cable` in der Datei) | passt in | Grenze |
|---|---|---|
| Kupfer gerade und gekreuzt (`copper`, `copper-cross`) | RJ45 | nach Kategorie, ohne Kategorie wie bisher |
| Glasfaser Multimode und Singlemode (`fiber-mm`, `fiber-sm`) | SFP, SFP+, SFP28 untereinander, QSFP28 mit QSFP28 | nach Optik und Kategorie |
| Seriell (`serial`), erstes Ende DCE oder DTE (`serial-dce`, `serial-dte`) | seriell | V.35 nach Takt |
| Konsole (`console`), Funk (`wireless`), WAN (`wan`) | Konsole und USB, WLAN und Access Point, WAN | RS-232 15 m, keine, keine |
| Telefon (`phone`) | RJ11, WAN | 1000 m (VDSL2) |
| Koax (`coax`) | Koax, WAN | 500 m (RG-6) |
| DAC (`dac`) | SFP+, SFP28 untereinander, QSFP28 mit QSFP28 | 7 m, ohne Optik |
| AOC (`aoc`) | wie DAC | 100 m, ohne Optik |

Ein falsches Kupferkabel steckt, unter „Auto-MDI-X aus“ (Projektinspektor, `strict_mdix`, ein
Rückgängig-Schritt) bleibt die Leitung dann unten und das Problem nennt das richtige Kabel; Glas
in RJ45 steckt nicht. Ein gewähltes serielles Ende ändert nur, was man sieht: den Punkt und
„DCE“ an diesem Ende auf der Zeichenfläche und die Zeile `Serial cable end: DCE` oder `DTE` in
`show interfaces`. Einen Befehl `clock rate` kennt die Engine nicht, und kein Ende liefert einen
Takt; das unveränderte `serial` älterer Dateien nennt kein Ende.

Die **Kategorie** (`grade` in der Datei) ist ein Feld des Kabels statt weiterer Arten; ohne sie
gelten die Grenzen wie bisher:

| Kategorie | bis 5G | 10G | 25G, 40G |
|---|---|---|---|
| Cat5e | 100 m | trägt es nicht | trägt es nicht |
| Cat6 | 100 m | 55 m | trägt es nicht |
| Cat6a, Cat7 | 100 m | 100 m | trägt es nicht |
| Cat8 | 100 m | 100 m | 30 m |

| Kategorie | Optik | 1G | 10G | 25G | 40G | 100G |
|---|---|---|---|---|---|---|
| OM3 (wie ohne Kategorie) | SR | 550 m | 300 m | 70 m | 100 m | 70 m |
| OM4 | SR | 550 m | 400 m | 100 m | 150 m | 100 m |
| OS2 (wie ohne Kategorie) | LR | 5 km | 10 km | 10 km | 10 km | 10 km |

Trägt eine Kategorie die Speed nicht, bleibt die Leitung unten wie bei einem zu langen Kabel, mit
Notiz, Logzeile und Problem (`ProblemKind::CableGrade`); eine Kategorie, die nicht zur Art passt,
zählt nicht und wird in `check()` gewarnt. Die **Farbe** (`color`, Blau, Türkis, Violett, Pink,
Braun, Schiefer; Vorgabe: die der Art) ist nur fürs Auge und ersetzt die Farbe der Art, nicht ihr
Muster; der Simulator sieht sie nicht, den Zustand zeigen weiter die LEDs.

### Im Editor: Verbinden, Kabel, Inspektor

- **Verbinden** (Werkzeug C): von einem Gerät auf ein anderes ziehen oder beide nacheinander
  anklicken. Das Ziel ist in Akzentfarbe umrandet, wenn ein freier Port dazu passt, sonst rot
  mit dem Tooltip „Kein passender Port“. Beim Loslassen öffnet sich am Zeiger die Port-Auswahl:
  beide Geräte mit allen Ports (Typ-Symbol, Name, Speed, Zustand), belegte mit ihrer
  Gegenstelle, Ports ohne Partner mit dem Grund; das beste Paar ist gewählt, darunter das Kabel
  („Automatisch: Kupfer, gerade“ oder ein anderes, das passt); Enter verbindet. Umschalt beim
  Loslassen oder ein Zug von Port-Punkt zu Port-Punkt (beim Überfahren erscheint je
  Portgruppe der nächste freie Port als Punkt am Rand) verbindet sofort. Das Popover hält die
  gewählte Art, nicht ihren Platz in der Liste: passt sie zu einem anderen Portpaar nicht, gilt
  dort „Automatisch“, und zurück ist sie wieder da.
- **Kabel zuerst wählen** (Bereich „Kabel“ der Palette): „Automatisch“ und eine Kachel je Art mit
  einem Stück dieses Kabels; der Tooltip nennt die Grenzen, die Suche findet die Arten. Die Wahl
  setzt das Werkzeug Verbinden mit dieser Art (die Statusleiste nennt sie), bis Esc oder zu einem
  anderen Werkzeug; ein zweiter Klick auf die Kachel geht zurück zu Auswahl. Das Ziel ist dann nur
  in Akzentfarbe umrandet, wenn die Art dort passt, und die Port-Auswahl zeigt nur die Ports, in
  die sie steckt, und bietet nur passende Paare an; was gar nicht passt, bleibt rot mit dem Grund
  im Tooltip („Glasfaser, Multimode passt nicht in einen RJ45-Port.“), beim Loslassen kommt die
  Warnung mit dem Grund, keine Port-Auswahl. „Automatisch“ verhält sich wie ohne Wahl.
- **Kabel** laufen vom Rand des einen Geräts zum Rand des anderen, gerade, rechtwinklig oder
  als Kurve durch ihre Wegpunkte, und zeigen ihre Art nicht nur über die Farbe: Kupfer
  durchgezogen, gekreuzt gestrichelt, Glasfaser orange (Multimode durchgezogen, Singlemode
  Strich-Punkt), seriell mit Zickzack (ein gewähltes Ende: ein Punkt vor dem DCE-Ende, „DCE“ am
  Portnamen), Konsole gepunktet, Funk dünn gepunktet, WAN doppelt, Telefon violett
  Strich-Punkt-Punkt, Koax als Röhre, DAC mit Querstrichen, AOC dick gestrichelt; eine gewählte
  Farbe ersetzt nur die Farbe der Art. 12 Einheiten vor jedem Ende sitzt die LED des
  Ports aus dem Simulator (grün aktiv, rot unten, grau ohne Strom, bernstein für blockierend
  und lernend); ab 100 % stehen die Portnamen daneben. Der Tooltip nennt Ports, Art, Speed,
  Zustand, Pakete pro Sekunde, Kategorie und Grenze. Ein gewähltes Kabel hat „+“-Griffe in der Mitte seiner
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
  Befehlszeile als Tab. Jedes Gerät hat eine Konsole (dazu weitere Sitzungen: „Neue Sitzung“,
  siehe [Sitzungen und Telnet-Server](#sitzungen-und-telnet-server)) mit 5000 Zeilen Rückblick, Verlauf
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
  sichern“), Firewall (bei IOS-artigen Geräten, die routen: Zugriffslisten, was welche Schnittstelle
  schützt, NAT, welche Regel ein Paket trifft; siehe [NAT und Zugriffslisten](#nat-und-zugriffslisten)),
  Dienste (DHCP, DNS, Syslog, NTP, TCP-Testdienste mit Zustand und Zahlen; die des
  Projekts schalten sich sofort ein und aus, auch rückgängig ohne Neustart, und ihre
  „Einstellungen …“ sind JSON, siehe [Labore und Aufgaben](#labore-und-aufgaben)), CLI (dieselbe
  Konsole wie im Terminal-Tab) und bei Hosts ein Desktop mit Programmen in eigenen Fenstern,
  auf dem ihr Gerätefenster öffnet (siehe [Desktop der Hosts](#desktop-der-hosts)), darunter der
  [Webbrowser](#webbrowser). F11 zeigt ein Gerätefenster mit jedem Reiter
  im Vollbild. Eine Konsole erscheint an einer Stelle: Zeigt das Gerätefenster sie,
  verweist der Tab dorthin. Offen sind höchstens zwei Gerätefenster und drei Konsolenfenster.
  Der Reiter Dateien zeigt die Dateien des Geräts mit Import und Export (siehe
  [Dateien, TFTP und FTP](#dateien-tftp-und-ftp)).
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
[Labore und Aufgaben](#labore-und-aufgaben)), ein freies Projekt dort `null`. Die physische
Ansicht (siehe [Physische Ansicht](#physische-ansicht)) bringt `places` (die Orte), `phys` und
`optics` je Gerät, `len_m` je Kabel und im Layout `phys` und `phys_at` (die gezeigte Ansicht
und ihr Ort) mit; ältere Dateien haben nichts davon und laufen wie bisher. Ein Kabel kann
`grade` (seine Kategorie, siehe [Kabel](#kabel)) und `color` (eine Farbe fürs Auge) haben; was
eine Datei nicht sagt, bleibt, wie es war.
Gruppen und Hintergrundbild (siehe [Gruppen und Hintergrundbild](#gruppen-und-hintergrundbild))
stehen in `groups` (`[]` ohne) und `background` (`null` ohne); auch sie fehlen älteren Dateien.
`description` sagt, worum es geht, `author`, wer es gemacht hat (Datei › Projektinfo); älteren
Dateien fehlt `author`, dann bleibt er leer.

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

Die Dateien eines Geräts stehen in `files`, ein Eintrag je Datei oder selbst angelegtem
Verzeichnis, nach Pfad geordnet: Text als `text`, alles andere als `base64`, die Zeit in
Sekunden seit 1970 nach der Uhr des Geräts als `mtime` (siehe
[Dateien, TFTP und FTP](#dateien-tftp-und-ftp)). Ein Gerät ohne `files` hat nur seine
Verzeichnisse und sein Systemabbild.

```json
"files": [
  { "path": "/srv/ftp/upload", "dir": true },
  { "path": "/srv/tftp/r1-config", "text": "hostname R1\n!\nend\n", "mtime": 1767225605 },
  { "path": "/srv/tftp/logo.bin", "base64": "iVBORw0KGgo=" }
]
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
Pakete zu diesem Zeitpunkt, die Geräte zeigen ihren Zustand von damals (siehe
[Zurückspulen](#zurückspulen)). Der nächste Schritt geht durch die Aufzeichnung wieder
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
  Zeichenfläche „Ansicht bei t = …“ mit den Knöpfen „Von hier fortsetzen …“ und „Zurück zu
  live“. Die Statusleiste nennt Modus, Uhr und Ereignisse.
- **Pakete auf der Leinwand:** kleine Umschläge in gedämpften Farben je Protokoll (ICMP grün,
  ARP bernstein, DNS violett, TCP/HTTP blau, UDP cyan, DHCP petrol, STP grau, OSPF/RIP braun
  wie BGP, Rest schiefergrau), unter 50 % Zoom Punkte, dort auf dem Kabel, wo der Frame zum
  gezeigten Zeitpunkt ist. Ein Schritt bewegt sie vom alten zum neuen Zeitpunkt (Ankommende laufen in ihr
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

### Zurückspulen

Steht die Ansicht im Simulationsmodus in der Vergangenheit (Schritt zurück, Zeitleiste,
Ereignisliste), zeigen die Geräte ihren Zustand von damals
([`keyframe.rs`](crates/jfn_sim/keyframe.rs), [`src/doc/sim.rs`](src/doc/sim.rs)).

- **Keyframes:** der Simulator merkt sich während des Laufs Schnappschüsse seines ganzen
  Zustands. Ein Keyframe ist eine Kopie der Engine ohne ihre Aufzeichnung: was sich nie ändert,
  teilen sich alle Kopien (die Bytes der Frames, die Modelle des Katalogs, die Befehlsbäume,
  der übersetzte Code der Skripte); von der Aufzeichnung behält er nur die Zähler und die
  Frames, die gerade auf einem Kabel sind. Was spätere Meilensteine an Diensten und Zustand
  dazubringen, kopiert `Clone` ohne eigenen Code mit.
- **Eingaben:** alles, was den Simulator von außen ändert, wird mit seiner Position
  aufgezeichnet (virtuelle Zeit und Zahl der bis dahin gelaufenen Ereignisse): Aktionen
  (Bearbeiten, Strom, Kabel, Fehler, Verschieben, Schritte eines Szenarios), Zeilen und
  Abbrüche an den Konsolen, PDUs, Pings und Abfragen (`Inline::call`) und die Stellen, an denen
  ein Lauf die Uhr anhält und die Zeitreihen eine Probe nehmen. Der Zustand zum Zeitpunkt t ist
  der nächste Keyframe davor und die Wiederholung der Ereignisse und Eingaben bis t. Sie ist
  exakt, weil der Simulator deterministisch ist: der Selbsttest vergleicht Zustand und
  Aufzeichnung der Wiederholung Byte für Byte mit dem Lauf.
- **Speichergrenze** (`Budget::default()`): Abstände zählen in Ereignissen, denn die kostet
  eine Wiederholung (eine ruhige Minute ist sofort nachgespielt, ein Labor rechnet Tausende
  Ereignisse pro Sekunde). Ein Keyframe nach 20 000 Ereignissen, in langen Läufen erst nach
  1/64 der bisher gelaufenen (je länger der Lauf, desto seltener); zusammen höchstens 64 MiB
  (geschätzt aus Tabellen, Puffern, Logs, Warteschlangen und Zeitreihen, `Service::footprint`
  für die Dienste; `Footprint::of` nennt die Teile) und 256 Stück. Darüber fällt der Keyframe,
  dessen Lücke im Verhältnis zu ihrem Alter am kleinsten wäre: die jüngste Vergangenheit bleibt
  dicht, ältere Keyframes werden zuerst ausgedünnt, ihre Abstände wachsen mit dem Alter. Der
  erste und der neueste bleiben; reicht der Platz nicht einmal für diese zwei, fällt der erste,
  und für frühere Zeitpunkte zeigt das Fenster nur die Aufzeichnung (der Hinweis sagt es).
- **Was die Vergangenheit zeigt:** LEDs und Zustand der Kabel auf der Zeichenfläche, den
  Inspektor (Schnittstellen, Adressen, VLANs, WLAN), die Gerätefenster (Übersicht,
  Schnittstellen, Konfiguration, Dienste, Desktop), Dienst-Pillen und Sparklines und im Bereich
  Metriken Baum, Statuszeile und Disk (Diagramme und Kacheln enden ohnehin beim gezeigten
  Zeitpunkt). Eine kurze Wiederholung läuft sofort (bis 50 000 Ereignisse), eine längere in
  Stücken von 20 000 je Takt; dann zeigt der Hinweis „Zustand wird berechnet … n %“, und bis
  dahin bleibt der vorige Zeitpunkt zu sehen. Geht die Ansicht vorwärts, rechnet der Simulator
  vom gezeigten Zustand aus weiter statt vom Keyframe.
- **Konsolen in der Vergangenheit:** Prompt, `?` und Tab kommen aus dem Zustand von damals; eine
  Zeile läuft auf einer Kopie, die danach verworfen wird. `show ip route`, `show arp`, `show mac
  address-table`, `ip neigh` … zeigen also die Tabellen von damals, Konfiguration und laufende
  Befehle bleiben ohne Wirkung (eine graue Zeile sagt es einmal je Zeitpunkt); Strg+C und
  Strg+Z tun dort nichts.
- **Bearbeiten in der Vergangenheit:** jede Änderung, die zum Simulator geht (Bearbeiten,
  Einstellungen, Formulare, Fehler, PDUs, Strom), holt die Ansicht zuerst in die Gegenwart;
  das Verschieben von Geräten nicht.
- **Von hier fortsetzen** (im Hinweis und im Menü Simulation, nach einer Rückfrage): der
  gezeigte Zustand wird die Gegenwart. Die Aufzeichnung danach und alles, was danach von außen
  kam (Konsolen, PDUs, Fehler), geht verloren; später gestartete PDUs gelten als
  zurückgesetzt, Konsolen und Protokoll vermerken, wo es weitergeht. Das Projekt und seine
  Rückgängig-Liste bleiben, wie sie sind: der Simulator bekommt den Stand des Projekts, den er
  damals kannte, und folgt dann dem heutigen, Änderungen am Projekt nach dem Zeitpunkt gelten
  also ab ihm. Rückgängig und Wiederholen arbeiten weiter auf dem Projekt; ein Fehler-Schritt,
  dessen Fehler es nicht mehr gibt, findet nichts zu heilen, und erreichte Ziele eines Labors
  bleiben erreicht.
- `jfn-sim rewind PROJEKT [SEKUNDEN] [--traffic K]` lässt ein Projekt einmal ohne und einmal mit
  Keyframes laufen und nennt Zahl und Größe der Keyframes und die Zeit, zehn Zeitpunkte
  wiederherzustellen.

Messwerte auf dem Entwicklungsrechner (Windows, `jfn-sim rewind`, mit anderen Builds
parallel):

| Projekt | Lauf | Keyframes | je Keyframe | Wiederherstellen (min / Median / max) |
|---|---|---|---|---|
| `campus.jfnet` (7 Geräte, ruhig) | 60 s, 286 Ereignisse | 1 | 140 KiB | 0,24 / 0,52 / 0,64 ms |
| `lab_window.jfnet` (Labor: Cache, Loadbalancer, KV, Queue, Skript) | 60 s, 236 470 Ereignisse | 12 | 0,3–1,3 MiB, zusammen 10,6 MiB | 1,9 / 22 / 35 ms (bis 18 000 Ereignisse) |
| `lab_shards.jfnet` (KV-Speicher unter Last) | 60 s, 125 415 Ereignisse | 7 | 0,25–1,1 MiB | 1,3 / 25 / 33 ms |
| synthetischer Campus (`jfn-sim gen 500`, `--traffic 300`) | 10 s, 20 962 Ereignisse | 2 | 7,4–8,1 MiB | 2,0 / 2,3 / 3,1 ms |

Mit und ohne Keyframes läuft der Simulator gleich schnell (im Rahmen der Messgenauigkeit, das
Labor: 476 gegen 481 ms für 60 s). Von einem Keyframe aus kostet die Wiederholung im Labor rund
2 µs je Ereignis; die Größe bestimmen vor allem die Zeitreihen, die TCP-Puffer, die
Ereigniswarteschlange und die Tabellen der Geräte.

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
  405 mit `Allow`, 413, 414, 431 und 505. Mit `"root": "/var/www"` antwortet er zusätzlich aus
  den Dateien des Geräts (siehe [Webbrowser](#webbrowser)).
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
- BGP ([`cli/bgp.rs`](crates/jfn_sim/cli/bgp.rs)): `router bgp ASN` mit eigenem Modus hinter
  demselben Prompt `(config-router)#`, `show ip bgp [A.B.C.D|summary|neighbors [X [routes|
  advertised-routes]]]`, `show ip route bgp`, `clear ip bgp *|X [soft [in|out]]`, `debug ip bgp
  [events|updates]` (siehe [Routing mit BGP](#routing-mit-bgp)).
- Dateien ([`cli/files.rs`](crates/jfn_sim/cli/files.rs)): `dir [flash:|nvram:]`, `show
  flash:`, `more DATEI`, `delete [/force] DATEI`, `copy QUELLE ZIEL` zwischen
  `running-config`, `startup-config`, `flash:`, `tftp:` und `ftp:`, `ip ftp username|password`
  (siehe [Dateien, TFTP und FTP](#dateien-tftp-und-ftp)).
- SNMP ([`cli/snmp.rs`](crates/jfn_sim/cli/snmp.rs)): `snmp-server community NAME [ro|rw
  [ACL]]`, `snmp-server location|contact TEXT`, `snmp-server host ADRESSE [traps|informs]
  [version 1|2c] COMMUNITY`, `snmp-server enable traps [snmp [authentication] [linkdown]
  [linkup] [coldstart]]`, `no snmp-server`, `show snmp [community]`, `debug snmp packets`
  (siehe [SNMP](#snmp)).

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
Dateien zeigen und ändern `ls`, `cat`, `cp`, `rm`, `mkdir`, `rmdir`, `touch` und `echo … >`,
übertragen `tftp` und `curl ftp://` (siehe [Dateien, TFTP und FTP](#dateien-tftp-und-ftp)).
Die Shell hat ein Arbeitsverzeichnis, Pipes und Umleitungen für jeden Befehl, Platzhalter, die
klassischen Werkzeuge und Prozesse (siehe
[Die Shell: Ordner, Werkzeuge, Prozesse](#die-shell-ordner-werkzeuge-prozesse)).
Mail schicken `mail` und `curl smtp://`, `curl pop3://` holt sie ab (siehe
[Mail](#mail-smtp-und-pop3)); `echo "nameserver ADRESSE" > /etc/resolv.conf` stellt den
Namensserver von Hand ein.
Agenten fragen und ändern `snmpget`, `snmpgetnext`, `snmpwalk`, `snmpbulkwalk` und `snmpset`
(siehe [SNMP](#snmp)).

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

`place GERÄT ORT [HE]` stellt ein Gerät in einen anderen Raum, auf einen Tisch oder in ein Rack
der physischen Ansicht, `cable GERÄT PORT length M` (oder `length auto`) gibt einem Kabel eine
Länge von Hand; `expect cable GERÄT PORT length [VERGLEICH] M` und `expect cable GERÄT PORT up`
(`down`) prüfen Länge und Zustand (siehe [Physische Ansicht](#physische-ansicht)).

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

Ein Szenario zeigt BGP (siehe [Routing mit BGP](#routing-mit-bgp)). `bgp_isp.scn` auf
`fixtures/bgp_isp.jfnet`: Kunde A (AS 65010, CA mit PC-A) hängt an zwei Providern (ISP1 in AS
100, ISP2 in AS 200), Kunde B (AS 65020, CB mit PC-B, `redistribute connected`) nur an ISP2;
die Provider verbindet der Transit-Provider AS 300 aus TR1 und TR2 mit iBGP zwischen ihren
Loopbacks (OSPF darunter, `update-source`, `next-hop-self`). Alle Sitzungen kommen nach 1 s mit
einer Kollision ihrer beiden Verbindungen, die iBGP-Sitzung, sobald OSPF die Loopbacks kennt;
nach 120 s (neue Wege gehen höchstens alle 30 s an externe Nachbarn) kennt jeder jedes Netz:
`show ip bgp` mit `>i` bei TR1 und `r` bei ISP2, `show ip bgp summary`, `show ip bgp
neighbors`, der Weg von PC-A zu PC-B über ISP2. Dann fehlt das Kabel CA–ISP2: die Sitzung endet
sofort (Fast External Fallover), der Weg führt über ISP1 und AS 300 (`traceroute`), und ISP2
findet CAs Netz über AS 300; mit dem Kabel kommt die Sitzung zurück, was ISP2 noch über AS 300
kannte, fällt bei CA als Schleife weg (65010 im AS-Pfad), und der kurze Weg gilt wieder.

Ein Szenario zeigt Dateien, TFTP und FTP (siehe [Dateien, TFTP und FTP](#dateien-tftp-und-ftp)).
`backup.scn` auf `fixtures/backup.jfnet`: R1, R2 und R3 sichern ihre Konfiguration per TFTP auf
SRV1 (`copy running-config tftp:` mit den Fragen nach Host und Datei, die URL in einer Zeile,
R3 seine Startkonfiguration über ein Kabel, das 30 % der Rahmen verliert: Blöcke und ACKs gehen
noch einmal); R1 bekommt einen falschen Namen und holt die Sicherung zurück, auch als
Startkonfiguration; `copy running-config flash:`, `dir`, `delete` mit Rückfrage und `show
flash:`; eine fehlende Datei auf dem Server. PC1 lädt als anna per FTP hoch, listet und lädt
herunter, scheitert mit einem falschen Passwort, holt anonym `pub/readme.txt` und mit `tftp`
eine Sicherung; 24 Erwartungen. FTP läuft dort in Schritten von 2 s, weil `run until idle` bis
zum Leerlauf-Timeout des Servers (300 s) liefe.

Ein Szenario zeigt Mail (siehe [Mail](#mail-smtp-und-pop3)). `mail_domains.scn` auf
`fixtures/mail_domains.jfnet`: Anna (PC1, a.lab) schreibt Bob mit `curl -v smtp://` über MAILA,
der den MX von b.lab bei DNS1 nachschlägt und die Nachricht an MAILB weitergibt (R1 verliert den
ersten SYN, während er nach MAILB fragt: sie ist nach gut 1 s da); Bob listet sie, holt sie mit
`curl -v pop3://` (seine erste DNS-Frage verliert R1 ebenso, der Resolver fragt nach 5 s noch
einmal) und löscht sie; eine Nachricht an einen unbekannten Benutzer von b.lab (`echo … |
mail`) kommt als Unzustellbarkeitsnachricht zu Anna zurück, die sie abholt; 14 Erwartungen.

Ein Szenario zeigt AAA mit RADIUS (siehe
[Logins, Telnet und AAA mit RADIUS](#logins-telnet-und-aaa-mit-radius)). `aaa_radius.scn` auf
`fixtures/aaa_radius.jfnet`: R1 und R2 melden ihre Konsolen über den RADIUS-Server von SRV1 an
(Benutzer admin mit Stufe 15), R2 scheitert mit einem falschen Passwort, und nachdem SRV1
ausgeschaltet ist, fällt R1 auf seinen lokalen Benutzer zurück; 7 Erwartungen.

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
| `http.serve(port, fn(req, res))` | HTTP-Server, einmal je Skript: `req` ist `{method, path, target, query, headers, body, peer, peer_port}` (`query`: die Felder der URL, dekodiert), bei einem Inhalt vom Typ `application/x-www-form-urlencoded` dazu `form` mit dessen Feldern (sonst fehlt es: `req.form ?? {}`), `res.send(status, body, headers)` und `res.json(status, wert, headers)` gehen auch später aus einem anderen Rückruf; gibt der Handler einen Text, Bytes, eine Liste, eine Map oder einen Status zurück, ohne `res` zu nutzen, ist das die Antwort |
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
`ScriptService::view`, `errors`, `output`, `source`, `file`; im Fenster die IDE (unten).

**Dienste aus Dateien** (IDE1): statt des Quelltexts kann eine Definition eine Datei des Geräts
nennen, `{"kind": "script", "name": "web", "config": {"file": "/root/app/server.jfs", "port":
80}}` (ein relativer Pfad gilt ab `/root`; `source` und `file` zugleich lehnt der Simulator ab).
Der Dienst liest die Datei bei jedem Start – mit dem Gerät, nach einem Stopp, bei einem Neustart
– und übersetzt sie unter ihrem Pfad: ihre relativen Importe beginnen in ihrem Ordner, ihre
Fehler nennen die Datei. Speichern ändert am laufenden Programm nichts; erst der nächste Start
liest die gespeicherte Fassung (die IDE sagt das in ihrer Statuszeile und bietet „Neu starten“).
Fehlt die Datei, ist sie kein Text oder lässt sie sich nicht übersetzen, scheitert der Start als
Fehler des Dienstes mit der Datei (und Zeile und Spalte, wenn es eine Stelle gibt): in der
Statuszeile („beendet nach einem Fehler: /root/app/server.jfs: die Datei gibt es nicht“), in
`script errors`, im Log und in der Ereignisliste, und `Engine::problems` meldet `ScriptFile`
(„Skript web auf SRV1: die Datei /root/app/server.jfs gibt es nicht.“). `config.file` nennt die
Datei wie bei `jfs DATEI`. Projekte mit `source` laden und laufen wie bisher.

**Tests in der Shell** ([`cli/jfstest.rs`](crates/jfn_sim/cli/jfstest.rs)): `jfs --test
DATEI.jfs …` führt die Funktionen `test_…` der Dateien nacheinander in einem Prozess der Shell
aus, jede in einem frischen Programm (die Datei wie ein Dienst sie liefe, mit
`stop_on_error`: erst ihr Hauptprogramm, dann der Test, dann seine Timer und Rückrufe in
virtueller Zeit). Ein Test besteht, wenn nichts mehr zu tun ist oder seine Zeit (10 s) um ist,
und scheitert am ersten Fehler; `assert` und `assert_eq` werfen einen mit beiden Werten.
Zwischen zwei Tests schließt, was der vorige offen hatte (`ScriptService::release`). Die
Ausgabe folgt dem Runner `jfs` der Library („ok      NAME (DATEI)“, „FEHLER  NAME
(DATEI:ZEILE:SPALTE): …“ mit dem, was der Test ausgab, „N Tests: P bestanden, F
fehlgeschlagen“), der Exit-Code ist 1, wenn ein Test scheiterte, eine Datei nicht lud oder kein
Test da war. `Engine::jfs_tests(dev)` hält den letzten Lauf eines Geräts (die Tests mit ihren
Zeilen, die Ergebnisse, ob er fertig ist) für die IDE.
Geprüft wird das in der Selbsttest-Gruppe `script` ([`src/scripttest.rs`](src/scripttest.rs):
Antwortzeiten auf die Nanosekunde aus dem Fuel, Timer, UDP, TCP, HTTP, DNS, Disk, Metriken,
Endlosschleife, Grenzen, Fehler, `script`, `top`, Determinismus, ein neuer Quelltext, der Disk,
Zahlen und Fehler behält, Umbenennen im Lauf) und im Golden-Szenario oben.

**Module aus den Dateien des Geräts** (SK7): `let util = import("/home/user/lib/util.jfs")`
liest eine Datei des Geräts (die Dateien des Reiters Dateien; ein relativer Pfad gilt ab der
Datei, die importiert, im Quelltext des Dienstes ab `/root` wie in der Shell, in einem Prozess
`jfs DATEI` ab dem Ordner von DATEI) und gibt ihre
obersten `fn`s und `let`s als Map; Namen mit `_` bleiben im Modul. Jedes Modul läuft einmal je
Start des Dienstes, im Rückruf, der es zuerst importiert, mit dessen Rechenzeit; das Lesen
kostet die Zeit der Disk des Geräts (`"disk": {...}`), in der das Programm wartet. Fehler eines
Moduls nennen seine Datei: `ScriptErr::file`, die Statuszeile („… in /home/user/lib/util.jfs,
Zeile 7, Spalte 12: …“), `script errors`, die Zeile im Log und der Text der Notiz; eine fehlende
Datei, ein Zyklus und ein Modul, das sich nicht übersetzen lässt, sind fangbare Fehler.

**Der Debugger eines Skripts** (die Stellen, an die die IDE andockt, siehe [IDE](#ide)):
`Action::ScriptDebug { dev, svc, op }` mit `DebugOp::Pause` (anhalten an der nächsten
Anweisung, im laufenden Rückruf oder im nächsten), `Continue`, `Step(Into | Over | Out)`,
`Break { file, line, cond }` (`file` leer: der Quelltext des Dienstes, sonst der Pfad eines
importierten Moduls; `cond` ein Ausdruck in jfs), `Unbreak` und `ClearBreaks`. Haltepunkte gehören
zum Dienst und gelten für jeden Start. `ScriptService::paused` sagt, wo der Rückruf hält,
`debug_vm` gibt die VM für Aufrufliste, Variablen und Werte, `debug_eval` wertet einen Ausdruck
auf einer Kopie der VM aus, `ScriptView::paused` und die Statuszeile („angehalten in Zeile 3,
Spalte 3“) zeigen es. Angehalten wird in virtueller Zeit: die Simulation läuft weiter, die CPU
des Skripts ist frei (keine Rechenzeit), Ereignisse warten in der Warteschlange wie hinter einer
belegten CPU (ist sie voll, gehen Datagramme verloren und Anfragen bekommen 503), TCP-Daten
bleiben im Socket, fällige Timer stehen an und kommen verspätet (`every` überspringt verpasste
Runden), was der Rückruf vor dem Halt nach außen tat, geschieht zu seiner Zeit. Weiter geht es
im Augenblick von `Continue` oder `Step`: dort beginnt die restliche Rechenzeit des Rückrufs.
Stoppen, Strom aus und ein neuer Quelltext beenden einen Halt mit dem Programm. Geprüft wird das
in der Selbsttest-Gruppe `scriptmod` ([`src/scriptmodtest.rs`](src/scriptmodtest.rs): Module
mit absolutem und relativem Pfad, die Zeit der Disk, eine fehlende Datei, ein Zyklus, ein Fehler
im Modul in Fehler, Log und Status, ein angehaltenes Skript mit verspäteten Timer-Runden, ein
Schritt und ein Ausdruck im Halt, ein Haltepunkt mit Bedingung, Ablehnungen, Determinismus
zweimal und mit einem Klon im Halt).

### IDE

Die Programme eines Hosts schreibt, startet, prüft und debuggt die **IDE**, ein Programm seines
[Desktops](#desktop-der-hosts), das maximiert in seinem Fenster startet (Kontextmenü eines
Hosts › „IDE …“ öffnet das Gerätefenster auf dem Desktop mit der IDE vorn, ebenso das Symbol
und das Startmenü; [`src/workspace/ide/`](src/workspace/ide/mod.rs), im Fenster ein Aufruf von
`ide::view` in `desktop::ide_window`). Eine `.jfs`-Datei, die der Dateimanager öffnet, kommt
als Tab der IDE. Oben stehen die Menüs Datei,
Bearbeiten, Ausführen, Debuggen und Ansicht (jede Zeile nennt ihre Tasten) und die
Werkzeugleiste; links die Seitenleiste mit Explorer, Suchen und Gliederung, rechts der Editor
mit seinen Tabs, darunter Ausgabe, Terminal, Probleme, Debugger, Tests, Bereitstellen und Lasttest. Die Teiler behalten
ihre Lage, Seitenleiste (Strg+B) und untere Leiste (Strg+J) lassen sich ausblenden und weichen,
wenn das Fenster klein ist; F6 geht von Bereich zu Bereich.

- **Explorer** ([`explorer.rs`](src/workspace/ide/explorer.rs)): die Dateien des Geräts als
  Baum (Ordner zuerst; `/root` und die Ordner offener Dateien sind offen), `.jfs`-Dateien mit
  Sprachdiensten, andere Textdateien öffnen sich ebenso; neben einer Datei stehen die Dienste,
  die sie ausführen, ein „●“ markiert ungespeicherte Änderungen. Darüber die Skripte, deren
  Quelltext noch in der Projektdatei steht („Eingebettete Skripte“). Doppelklick oder Eingabe
  öffnet, „Neue Datei …“, „Neuer Ordner …“ (ein Flyout fragt nach dem Namen), Umbenennen (F2)
  und Löschen (Entf) sind Rückgängig-Schritte des Projekts ([`doc/ide.rs`](src/doc/ide.rs)),
  Datei › „Neues Skript: …“ legt eine `.jfs`-Datei aus einer Vorlage an. Ein Dienst behält den
  Pfad, den er nennt: eine umbenannte oder gelöschte Datei fehlt ihm beim nächsten Start.
- **In Datei umwandeln:** ein eingebettetes Skript wird zur Datei `/root/scripts/NAME.jfs`
  (`NAME2.jfs` …, wenn es sie gibt) und seine Definition nennt `"file"` statt `"source"` – ein
  Rückgängig-Schritt, der das Gerät laufen lässt: der Simulator bekommt erst die Datei, dann die
  Definition, das Skript startet aus der Datei und behält Disk und Zahlen; Rückgängig bringt den
  eingebetteten Quelltext zurück. Tab, Entwurf und Haltepunkte gehen mit.
- **Tabs und Entwürfe** ([`editor.rs`](src/workspace/ide/editor.rs)): jede offene Datei hat
  einen Entwurf mit eigenem Rückgängig, der den Wechsel der Tabs, das Schließen des Tabs und des
  Fensters übersteht; ein unveränderter folgt dem Projekt (ein Rückgängig des Speicherns zeigt
  den Text davor), ein Tab zeigt „●“ hinter dem Namen, solange er ungespeichert ist. Strg+S
  speichert die Datei als einen Rückgängig-Schritt des Projekts (ein eingebettetes Skript: in
  seine Definition, ein Quelltext mit Fehlern wird mit dem ersten abgelehnt), „Alle speichern“
  (Strg+Umschalt+S) alle Dateien in einem Schritt; Strg+W schließt den Tab, Strg+Bild ab / auf
  wechselt.
- **Sprachdienste (SK6)** ([`devwin/scriptls.rs`](src/workspace/devwin/scriptls.rs) auf
  `script::ls`): Prüfung beim Tippen (höchstens alle 250 ms), Vervollständigung mit den Namen
  des Simulators und den Namen importierter Module (die IDE liest die Dateien des Geräts, auch
  ungespeicherte Entwürfe), Hover, Signaturhilfe, Falten, F12 und Strg+Klick zur Deklaration –
  bei `lib.greet` in die andere Datei, die sich dort öffnet –, F2 benennt in der Datei um,
  Umschalt+Alt+F formatiert; die Gliederung der Seitenleiste zeigt die Funktionen und globalen
  Variablen der offenen Datei. **Probleme** listet die Meldungen der offenen Dateien und die
  Fehler des laufenden Dienstes, ein Klick geht hin; die Fehler des Laufs stehen als Marken im
  Text, ihre Zeilen folgen dem bearbeiteten Text.
- **Suchen** ([`search.rs`](src/workspace/ide/search.rs)): Strg+F sucht in der Datei (die
  Suchleiste des Editors), Strg+Umschalt+F in allen `.jfs`-Dateien des Geräts (Groß- und
  Kleinschreibung, ganze Wörter; was ein Entwurf zeigt, wo er ungespeichert ist), die Treffer
  als Liste, ein Klick öffnet die Datei an der Stelle; „Alle ersetzen“ ändert jede Datei in
  einem Rückgängig-Schritt und lehnt ab, solange eine Datei mit Treffern ungespeicherte
  Änderungen hat.
- **Ausführen** ([`debug.rs`](src/workspace/ide/debug.rs); zuvor speichert es alles):
  „Als Prozess ausführen“ (Strg+F5) tippt `jfs DATEI` ins **Terminal** der IDE: eine eigene
  Befehlszeilen-Sitzung neben der Konsole des Geräts (siehe
  [Sitzungen und Telnet-Server](#sitzungen-und-telnet-server); `pts/N`, geöffnet, wenn sie zum
  ersten Mal gebraucht wird, nach dem Ausschalten des Geräts eine neue; sie endet mit dem
  Fenster der IDE). „Als Dienst
  ausführen“ (F5) startet den Skript-Dienst, der auf die Datei verweist, neu – gibt es keinen,
  legt es einen als Rückgängig-Schritt an (`{"file": PFAD}`, nach der Datei benannt) –, bei einem
  eingebetteten Skript dessen Dienst; **Ausgabe** zeigt, was er im aktuellen Lauf druckt. Läuft
  die Datei schon als [Einheit](#einheiten-bereitstellen-und-lasttest), startet F5 diese Einheit
  neu und legt keinen zweiten Dienst an, der am Port scheiterte; ebenso eine gescheiterte Einheit,
  solange kein Dienst auf die Datei verweist. Eine Einheit, die nie lief, bleibt außen vor.
  Stoppen (Umschalt+F5) und Neu starten (Strg+Umschalt+F5) wirken im Simulator wie
  `script stop|restart`, bei einer Einheit wie `systemctl stop|restart`. Läuft der Dienst einer
  gespeicherten Datei noch mit der Fassung davor,
  sagt die Statuszeile das mit einem Knopf „Neu starten“.
- **Debugger:** ein Klick links neben eine Zeilennummer oder F9 setzt einen Haltepunkt,
  Strg+Umschalt+F9 einen mit Bedingung (ein Flyout, die Bedingung ist ein Ausdruck in jfs).
  Die Haltepunkte gehören je Datei zur IDE und gehen an den Dienst, den sie ausführt
  (`DebugOp::Break`, auch in importierten Dateien). Hält er, öffnet sich die Datei an der Zeile
  (der Ausführungspfeil in der Leiste) und der Bereich **Debugger** zeigt Ort und Grund, die
  Aufrufliste (ein Klick wählt den Aufruf und geht zu seiner Zeile), die Variablen des Aufrufs
  (lokal, erfasst, global; Listen und Maps klappen auf) und Ausdrücke, die im Aufruf gelesen
  werden. F5 setzt fort, Strg+F9 hält an, F10 geht über, F7 hinein, Umschalt+F7 heraus (F11
  bleibt dem Fenster). Angehalten wird in virtueller Zeit, die Simulation läuft weiter.
- **Bereitstellen und Lasttest:** Ausführen › Bereitstellen und › Lasttest öffnen die
  gleichnamigen Bereiche unten (siehe
  [Einheiten, Bereitstellen und Lasttest](#einheiten-bereitstellen-und-lasttest)).
- **Tests:** „Tests ausführen“ (Strg+Umschalt+T) tippt `jfs --test DATEI` ins Terminal; der
  Bereich **Tests** zeigt den Lauf (`Engine::jfs_tests`): bestanden und gescheitert mit Datei,
  Zeile, Spalte und Meldung, ein Klick geht zur Stelle des Fehlers.

Solange die IDE die Tastatur hat, gelten ihre Tasten vor denen des Hauptfensters (F5, F7 und F9
der Simulation und des Labors); die Übersicht der Tastenkürzel (F1) führt sie unter „IDE“.
Was die IDE ändert, ist ein Schritt des Projekts mit Rückgängig; Haltepunkte, Tabs und Entwürfe
gehören zur Sitzung. Der Simulator ([`scriptsvc.rs`](crates/jfn_sim/scriptsvc.rs)) bietet dafür
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
`keep_steps`; die Dateien eines Schritts vor seinen Diensten), so laufen Bearbeiten,
Rückgängig und Wiederholen ohne Neustart des Geräts.

### Einheiten, Bereitstellen und Lasttest

Hier läuft der ganze Weg eines Dienstes: als Dateien schreiben, debuggen, als Einheit auf mehrere
Hosts bringen, unter Last testen und messen.

**Einheiten** ([`jfn_model/unit.rs`](crates/jfn_model/unit.rs),
[`jfn_sim/units.rs`](crates/jfn_sim/units.rs), [`cli/units.rs`](crates/jfn_sim/cli/units.rs)): Auf
Linux-Hosts beschreibt eine Datei `/etc/systemd/system/NAME.service` einen Dienst (`mkdir -p
/etc/systemd/system` legt den Ordner an, die IDE tut es selbst):

```ini
[Unit]
Description=Die Bestell-API
After=network.target kv.service
Requires=kv.service

[Service]
ExecStart=jfs /root/app/server.jfs --port 8080
Environment=MODE=test "NAME=a b"
Restart=on-failure
RestartSec=2
Settings={"ns_per_op": 5}

[Install]
WantedBy=multi-user.target
```

- `ExecStart` startet zwei Programme: `jfs DATEI ARGUMENTE` ein Skript des Geräts (`config.args`,
  `config.env` aus `Environment`, `config.unit`, `config.file`; das Übrige aus `Settings`) und
  `jfn-service ART [DATEI]` einen fertigen Dienst des Simulators (`http-server`, `lb`, `kv`,
  `cache` …: die Arten aus [Dienste und virtuelles Betriebssystem](#dienste-und-virtuelles-betriebssystem))
  mit seinen JSON-Einstellungen aus der Datei und `Settings`. Pfade sind absolut.
- Eine Einheit läuft als ein Dienst unter ihrem Namen: er steht wie jeder andere unter Metriken,
  `top`, `ss -p`, Systemmonitor und Dienste; Stoppen und Starten behalten seine Zahlen, seine Disk
  und seine Zeitreihen. Was die Datei falsch macht, sagen `systemctl status` und das Log: unbekannte
  Abschnitte und Schlüssel sind Warnungen, ein fehlendes oder unbrauchbares `ExecStart` hält sie vom
  Start ab. Eine geladene Einheit behält ihren Text bis `systemctl daemon-reload`, und `status`
  sagt, wenn die Datei neuer ist. Nur die `.service`-Dateien selbst zählen; `Requires=` startet
  vorher, was fehlt, und stoppt mit ihm, `After=` ordnet den Start beim Hochfahren.
- **Dienste aus dem Projekt** erscheinen als erzeugte, nur lesbare Einheiten (`is-enabled` sagt
  `generated`): `start`, `stop`, `restart` und `status` wirken auf sie, `enable` und `disable` lehnen
  ab, denn ob sie starten, steht im Projekt. Ein `stop` gilt bis zum nächsten Einschalten. Heißt eine
  Datei wie ein Dienst des Projekts, gewinnt die Datei: er läuft dann nur als diese Einheit, und nur
  wenn sie eingeschaltet ist. Projekte ohne Einheiten laufen wie bisher, ohne jede Änderung.
- **Einschalten** ist eine Datei: `systemctl enable` legt
  `/etc/systemd/system/multi-user.target.wants/NAME.service` an (je `WantedBy`-Ziel eine, mit dem Pfad
  der Einheit als Inhalt, weil der Dateispeicher keine symbolischen Links kennt). Deshalb übersteht
  es Stromausfälle und das Speichern des Projekts; wer die Datei löscht, hat sie ausgeschaltet.
  Beim Hochfahren starten alle Einheiten aller `*.wants`-Ordner.
- **Neustart-Regeln** in virtueller Zeit: ein Skript als Einheit endet, wenn nichts mehr zu tun ist
  (Status 0: kein Timer, Socket, Server, keine offene Anfrage), oder nach einem Fehler, der es beendet
  (Status 1: im Hauptprogramm, beim Laden seiner Datei, mit `"stop_on_error": true` jeder). `no` lässt
  es dabei, `on-failure` startet nach `RestartSec` (Vorgabe 100 ms) neu, wenn der Status nicht 0 war,
  `always` in beiden Fällen. Ein `stop` startet nie neu. Wie beim Vorbild gibt die Einheit nach
  `StartLimitBurst` Starten (5) in `StartLimitIntervalSec` (10 s) auf und bleibt `failed`.

| Befehl | was es tut, Exit-Code |
|---|---|
| `systemctl start\|stop\|restart EINHEIT …` | 0; 5, wenn es sie nicht gibt; 1, wenn der Start scheitert (fehlende Datei, Fehler in Einheit oder Skript) |
| `systemctl status [EINHEIT …]` | eigene Darstellung: Zustand, Quelle, Programm, Neustarts, PID, Status, Grund, die letzten Log-Zeilen (`-n N`); 0 läuft, 3 läuft nicht, 4 gibt es nicht |
| `systemctl enable\|disable [--now] EINHEIT …` | die Dateien oben; 1 ohne Datei, und für eine erzeugte Einheit |
| `systemctl is-active\|is-failed\|is-enabled EINHEIT …` | ein Wort je Einheit (`-q`: keins); is-active 0 oder 3, die anderen 0 oder 1 |
| `systemctl list-units [--all] [--failed]`, `list-unit-files` | Tabellen in eigener Form |
| `systemctl daemon-reload`, `reset-failed [EINHEIT]`, `cat EINHEIT` | neu lesen, Fehler vergessen, die Datei zeigen |
| `journalctl [-u EINHEIT] [-n N] [-f] [-r]` | das Log des Geräts (die letzten 1000 Zeilen, `Monat Tag Zeit HOST TAG: TEXT`), mit `-u` die Zeilen einer Einheit: was ihr Dienst schrieb, bei Skripten auch `print`, und ihr Starten und Enden; `-f` folgt, bis Strg+C (130), in jeder Sitzung und im Hintergrund |

`man systemctl`, `man journalctl` und `--help` antworten in eigenen Worten; `Engine::units(dev)`,
`Engine::unit(dev, name)` und `Action::Unit { dev, name, op }` bedienen die Oberfläche. Die
Szenarien prüfen mit `expect WEB1 unit web active|inactive|failed|enabled|disabled`.

**Bereitstellen** ([`jfn_sim/deploy.rs`](crates/jfn_sim/deploy.rs),
[`ide/deploy.rs`](src/workspace/ide/deploy.rs)): In der IDE öffnet Ausführen › Bereitstellen den
gleichnamigen Bereich unten. Die Hosts sind eine Gruppe der Zeichenfläche, die Auswahl oder eine
Liste von Namen; dazu die Dateien und Ordner (zuerst der Ordner der gezeigten Datei), die Einheit
(zuerst die, die diese Datei ausführt; „Einheit anlegen“ schreibt ihre Datei und öffnet sie), die
Prüfung (Einheit aktiv, TCP-Port oder HTTP-Anfrage mit Pfad und Status), die Zeit dafür und die
Wartezeit danach. Bereitstellen speichert alle Dateien und schickt `Action::Deploy`: die Engine führt
die Bereitstellung selbst aus, als versteckter Dienst des Geräts (in `ps` als `deploy-N`), also auch
in der Aufzeichnung von K1. Host für Host, nacheinander: Dateien kopieren (auf der Ebene der Engine,
ein Ereignis je Host: 50 ms und die Bytes bei 100 Mbit/s; das Ziel muss eingeschaltet sein, ein Netz
braucht es dafür nicht), Einheit lesen, einschalten und neu starten, prüfen (alle 500 ms, bis es hält
oder die Zeit um ist, TCP und HTTP vom Quellgerät über das simulierte Netz), warten, nächster Host.
Der Fortschritt steht je Host daneben. Hält eine Prüfung nicht, endet der Lauf: die folgenden Hosts
bleiben unberührt, die fertigen behalten die neue Fassung; mit „Beim ersten Fehler zurückrollen“
bekommen der gescheiterte und dann die fertigen Hosts (der letzte zuerst) ihre alten Dateien zurück,
eine neue Datei verschwindet wieder, und die Einheit läuft wie zuvor. Abbrechen und ein Quellgerät,
das ausgeht, beenden den Lauf, wo er ist. `Engine::deployments(dev)` gibt die Läufe mit Hosts und
Log. Über das Netz kopiert die Bereitstellung noch nicht: sie schreibt die Dateien im Simulator direkt auf
den Host, `scp` (siehe [SSH](#ssh)) nutzt sie nicht.

Im Szenario, mit einer Gruppe oder einer Liste, und den Zeilen des Laufs im Bericht:

```
deploy DEV1 Webserver unit web files /root/app health http 80 /health 200 wait 500ms timeout 2s
deploy DEV1 WEB1,WEB2 unit api files /root/api health tcp 8080 rollback
expect DEV1 deploy done
```

**Lasttest** ([`ide/load.rs`](src/workspace/ide/load.rs)): Ausführen › Lasttest startet den
Lastgenerator (`http-load`, siehe [HTTP](#http)) auf dem Host der IDE: das Ziel (vorgeschlagen: der
Loadbalancer des Projekts, sonst der erste Host der letzten Bereitstellung), offene Last mit einer
Rate oder geschlossene mit Nutzern und Denkzeit, die Dauer. Daneben laufen die Kacheln (Antworten pro
Sekunde, p50, p95, p99, Fehlerquote, in Arbeit) und die Diagramme aus den Zeitreihen, wie im Panel
Metriken, und am Ende steht die Zusammenfassung. Der Lauf ist ein Dienst `lasttest` nur der Engine
und kein Teil des Projekts: er geht mit einem Neustart des Geräts und ersetzt den vorigen.

Geprüft wird das in der Selbsttest-Gruppe `units` ([`src/unitstest.rs`](src/unitstest.rs)), im
Golden-Szenario `deploy`, in `deploy_ide.uis` und `load_ide.uis` und am Labor
`05-microservices` (siehe [Labore und Aufgaben](#labore-und-aufgaben)).

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
| `05-microservices.jfnet` | Verteilte Systeme | zwei `jfs`-Dienste als Einheiten auf fünf Hosts hinter einem Loadbalancer, mit KV-Speicher und Cache, unter Last: Version 2 von DEV1 aus nacheinander ausrollen (IDE, Ausführen › Bereitstellen), ohne dass eine Anfrage scheitert; alle Hosts auf einmal neu zu starten lässt Anfragen scheitern |

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
`key`), `cable` (mit `peer` und `peer_port`; ein `value` verlangt eine Kabelart, `serial` nimmt auch beide gewählten Enden), `config`
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
  gehen über sie), `Engine::access_lists`, `access_groups`, `nat` und `nat_translations`;
  `Acls::probe` fragt eine Schnittstelle, was ihre Liste zu einem Paket sagt, ohne etwas zu zählen.

**Im Fenster: der Reiter Firewall** ([`devwin/firewall.rs`](src/workspace/devwin/firewall.rs)). Das
Gerätefenster von Firewalls, Routern und L3-Switches pflegt Listen und NAT in Formularen. Sie sind
eine Oberfläche über den Befehlen: Jeder Schritt läuft sichtbar in der Konsole, wird zusammen mit der
laufenden Konfiguration zur Startkonfiguration und ist ein Rückgängig-Schritt; was der Reiter zeigt,
liest er aus der Engine, eine in der Konsole getippte Regel steht also sofort da. Das Verhalten der
Engine ändert er nicht.

- **Regeln:** oben, was welche Schnittstelle schützt (eine Liste je Richtung, ein Punkt sagt, ob sie
  wirkt), darunter eine Liste mit Position, Aktion, Protokoll, Quelle, Ziel, Port, Log, Treffern und
  Bemerkung und dem impliziten deny als letzter Zeile. Der Text über dem Raster sagt, was die Liste
  gerade bedeutet: ungebunden oder ohne Regeln passiert alles. Neu, Bearbeiten, Duplizieren, nach oben,
  nach unten, Löschen, Listen anlegen und löschen, Zähler löschen. Das Formular kennt die Protokolle
  mit Namen, Adressen als `any`, `10.0.0.5`, `10.0.0.0/24` oder mit Wildcard-Bits, Port-Operatoren,
  `established`, ICMP-Nachrichten, `log` und eine Bemerkung; eine Regel, die es nicht fassen kann (eine
  Protokollnummer, ICMP-Typ und -Code ohne Namen), steht als ihre Zeile im Raster und lässt sich als
  Zeile bearbeiten, die eine Konsole zur Probe liest.
- **Verschieben** tippt, was auch die Befehlszeile täte: `no 30`, `15 permit …` (davor `ip access-list
  resequence`, wenn die Nummern nicht 10, 20, … sind). Nur die verschobene Regel beginnt wieder bei 0
  Treffern, die anderen behalten ihre. Eine Regel geht mit den Bemerkungen direkt über ihr. Weil eine
  Bemerkung nur am Ende einer Liste hinzukommt, muss für eine Bemerkung mitten in der Liste (und wenn
  beim Tausch beide Regeln Bemerkungen haben) neu getippt werden, was folgt: Dann beginnen auch diese
  Regeln bei 0. Beim Löschen einer Regel gehen die Bemerkungen über ihr mit.
- **NAT:** die Seite jeder Schnittstelle, Pools, dynamische Regeln mit PAT, statische Übersetzungen und
  Port-Weiterleitungen, darunter die laufenden Übersetzungen mit einer Schaltfläche zum Löschen.
- **Test:** Schnittstelle, Richtung, Quelle, Ziel, Protokoll, Port geben die Liste, die Regel, die
  trifft, und das Ergebnis, auch das implizite deny, ohne ein Paket zu senden. Die Antwort kommt aus
  `Acls::probe`, derselben Funktion, nach der die Engine filtert; ausgehende Listen sehen das Paket
  nach NAT, dort zählt also die übersetzte Quelle.
- **Befehle:** unter allen Teilen die Zeilen, die der letzte Schritt getippt hat.

Unmögliches lehnt der Reiter mit dem Grund ab: ein Gerät, das nicht routet (mit einer Schaltfläche, die
Routing einschaltet), eine Adresse, die keine ist, eine Netzmaske statt Wildcard-Bits, eine Regel, die es
schon gibt, eine Liste, die noch filtert oder aus der NAT auswählt, ein Pool, den eine Regel liest. Während
die Ansicht in der Vergangenheit steht, und auf einem Gerät ohne Strom, lässt sich nur lesen. Rückgängig
startet das Gerät mit der alten Konfiguration neu, also beginnen danach alle Zähler und Übersetzungen
bei 0. Was in der Konsole getippt und noch nicht gesichert war, sichert der nächste Schritt des Reiters
mit: Sonst könnte er diese Regeln nicht verschieben oder löschen.

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
  mit einer Passphrase von 8 bis 63 Zeichen, WPA2- oder WPA3-Enterprise mit einem RADIUS-Server,
  siehe [802.1X und WPA-Enterprise](#8021x-und-wpa-enterprise); 6 GHz nur mit WPA3), VLAN (auf Access Points: ihre
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

Noch offen: Kanäle, die sich gegenseitig stören, Ad-hoc-Netze, Band Steering und 802.11r, Netze
über die IOS-artige Befehlszeile einrichten.

Für Programme: `Action::Move { dev, x, y }`, `Action::Wlan { dev, wlan }`, `Action::Scale {
dip_per_m }`, `Engine::wlan_assocs()`, `wlan_station(dev)`, `wlan_stations_of(ap)`,
`wlan_survey(dev, ssid)`, `wlan_scale()`, die Notizen `Note::Wlan(WNote)`. Geprüft wird WLAN in
der Selbsttest-Gruppe `wlan` ([`src/wlantest.rs`](src/wlantest.rs)), SHA-1, PBKDF2, Key Wrap und
CMAC in der coretest-Gruppe `crypto`, EAPOL in `netpkt`, dazu das Szenario `wlan_office.scn` und
die Fensterskripte `wlan_inspector.uis`, `wlan_range.uis`, `wlan_drag.uis` und `wlan_shots.uis`.

### Routing mit BGP

Router, L3-Switches und die Firewall sprechen BGP-4 nach RFC 4271
([`bgp.rs`](crates/jfn_sim/bgp.rs)), mit 4-Byte-AS-Nummern nach RFC 6793 und Route Refresh nach RFC 2918. BGP ist ein Dienst des
Geräts („bgp“; im Gerätefenster unter Dienste mit Statuszeile und Zahlen: Nachbarn, davon
Established, Netze, Wege, OPEN, UPDATE, KEEPALIVE und NOTIFICATION in beide Richtungen,
Verbindungsversuche, abgewiesene Verbindungen, Sitzungsenden, verworfene Wege), den die
Befehlszeile einrichtet. Seine Sitzungen laufen über TCP-Port 179 durch die Sockets des Geräts
(der TCP von [TCP](#tcp)), auf virtueller Zeit; gleiche Projekte ergeben gleiche Läufe.

- **Einrichten** ([`cli/bgp.rs`](crates/jfn_sim/cli/bgp.rs)): `router bgp ASN` (1 bis 4294967295)
  öffnet einen eigenen Modus hinter demselben Prompt `(config-router)#` wie RIP und OSPF mit `bgp
  router-id`, `bgp log-neighbor-changes` (an), `bgp default local-preference` (100), `timers bgp
  KEEPALIVE HALTEZEIT` (60 180; gilt für neue Sitzungen), `network A.B.C.D [mask M]`
  (Klassennetze ohne Maske), `redistribute connected|static|rip [metric N]` und `redistribute
  ospf N [metric N]`, je Nachbar `neighbor X remote-as N` (das eigene AS: iBGP, ein anderes:
  eBGP), `description`, `update-source IF`, `next-hop-self`, `ebgp-multihop [TTL]`, `shutdown`
  und `weight`; `no neighbor X` nimmt ihn weg, die anderen Befehle eines Nachbarn brauchen
  zuerst sein `remote-as` (`% Specify remote-as or peer-group commands first`). Ein L3-Switch
  braucht `ip routing`, ein Gerät hat einen Prozess (`% BGP is already running; AS is …`), `no
  ip routing` und `no router bgp ASN` nehmen ihn mit. Die Running-Config listet nach dem Block
  von OSPF, was vom Vorgabewert abweicht, und kommt beim Start durch dieselbe Befehlszeile
  zurück. Was die Konfiguration ändert, gilt sofort (IOS will dafür bei Gewicht,
  `next-hop-self` und lokaler Präferenz ein `clear ip bgp … soft`); eine neue Router-ID, ein
  anderes `remote-as` oder `update-source` beginnt die Sitzung neu.
- **Sitzungen:** Jeder Nachbar durchläuft die Zustände des RFC: Idle, Connect (der eigene
  Verbindungsversuch läuft), Active (er wartet auf den Nachbarn oder auf ConnectRetry, 120 s),
  OpenSent, OpenConfirm, Established. Ein externer Nachbar beginnt nur, wenn seine Adresse in
  einem angeschlossenen Netz liegt oder `ebgp-multihop` gesetzt ist (sonst bleibt er Idle: „nicht
  direkt verbunden“), einer mit `update-source`, wenn das Interface eine Adresse hat. Die
  Verbindung geht von der Adresse der `update-source` aus, sonst von der des Wegs hinaus, mit
  TTL 1 zu externen Nachbarn (oder dem Wert von `ebgp-multihop`) und 255 im eigenen AS. Ohne
  Route zum Nachbarn wartet er Active und versucht es sofort, wenn eine Route kommt (etwa wenn
  OSPF die Loopbacks kennt). Verbindungen von Adressen, die kein Nachbar sind (oder einem
  abgeschalteten), setzt BGP gleich nach dem Handshake zurück. Fangen beide Seiten zugleich an
  – wie beim gemeinsamen Start nach 1 s, wenn der erste SYN am IOS-artigen ARP hängen blieb –,
  bleibt nach RFC 4271 6.8 die Verbindung, die die Seite mit der höheren BGP-ID geöffnet hat; die
  andere endet mit Cease / Connection Collision Resolution.
- **OPEN:** Version 4, das AS (AS_TRANS über 65535, das echte in der Capability), Haltezeit,
  Router-ID und die Capabilities IPv4 Unicast, Route Refresh und 4-Byte-AS. Abgelehnt wird mit
  der NOTIFICATION des RFC: eine andere Version, ein AS, das nicht `remote-as` ist (2/2 Bad Peer
  AS), die eigene Router-ID (2/3), eine Haltezeit von 1 oder 2 s (2/6), ein unbekannter
  Parameter (2/4). Die Haltezeit ist die kleinere beider Seiten, das Keepalive-Intervall das
  eingestellte, höchstens ein Drittel davon (`timers bgp 10 30` auf einer Seite: 30 und 10 auf
  beiden). Was ankommt, startet den Hold Timer neu, jedes gesendete KEEPALIVE oder UPDATE den
  Keepalive-Timer. Kein Timer wird zufällig verschoben (RFC 4271 10 schlägt es vor).
- **Ende einer Sitzung:** eine NOTIFICATION (gesendet oder empfangen), eine zurückgesetzte, nicht
  mehr beantwortete oder geschlossene TCP-Verbindung, der abgelaufene Hold Timer (NOTIFICATION
  4/0). Die Wege des Nachbarn verschwinden sofort, er beginnt wieder – sofort nach einer Sitzung,
  60 s später (Pause, unsere Wahl) nach einem abgelehnten OPEN, damit ein Nachbar mit falschem AS
  nicht im Sekundentakt flattert. Fällt das Interface zu einem direkt verbundenen externen
  Nachbarn aus, endet die Sitzung sofort (Fast External Fallover, wie IOS). `clear ip bgp`
  schickt Cease / Administrative Reset, `neighbor … shutdown` Cease / Administrative Shutdown,
  `no neighbor` Cease / Peer De-configured; `clear ip bgp X soft in` fragt mit ROUTE-REFRESH nach
  den Wegen, `soft out` schickt die eigenen noch einmal, `soft` beides. Die Konsole meldet
  `%BGP-5-ADJCHANGE` (mit `bgp log-neighbor-changes`) und jede NOTIFICATION
  (`%BGP-3-NOTIFICATION`).
- **UPDATE:** Die Prüfungen von RFC 4271 6.3 (Längen, Flags und Länge jedes Attributs, die
  Pflichtattribute ORIGIN, AS_PATH und NEXT_HOP, ORIGIN-Werte, AS_PATH-Segmente, Präfixe) mit
  ihrer NOTIFICATION; von einem externen Nachbarn muss das erste AS des Pfads seines sein
  (Malformed AS_PATH, wie IOS' enforce-first-as). Wege mit dem eigenen AS im Pfad fallen weg
  (Schleife), ebenso Wege, deren Next Hop eine eigene Adresse oder keine Unicast-Adresse ist;
  LOCAL_PREF zählt nur von internen Nachbarn. Unbekannte optionale transitive Attribute gehen mit
  dem Partial-Bit weiter, COMMUNITIES und AGGREGATOR wie empfangen.
- **Eigene Wege:** `network` erzeugt ein Netz, solange der Routing-Table genau dieses Präfix aus
  einer anderen Quelle hat (ORIGIN IGP, MED die Metrik dieser Route); `redistribute` nimmt die
  Netze der Interfaces, die oben sind, oder die Routen der Quelle, die im Routing-Table gewinnen
  (ORIGIN incomplete, MED ihre Metrik oder die angegebene). Eigene Wege haben Next Hop 0.0.0.0
  (auf dem Kabel die Adresse der Sitzung), Gewicht 32768 und die eigene lokale Präferenz.
- **Die Entscheidung:** Ein Weg zählt, wenn sein Next Hop über eine Route aufgelöst wird, die
  nicht von BGP stammt (deren Interface und Metrik: die IGP-Metrik). Unter den Wegen eines
  Präfixes behält jeder Schritt die besten, der Schritt, der einen übrig lässt, steht in der
  Notiz und in `show ip bgp A.B.C.D`: 1. höchstes Gewicht (`neighbor … weight`, eigene 32768),
  2. höchste LOCAL_PREF, 3. selbst erzeugt, 4. kürzester AS-Pfad (ein AS_SET zählt eins), 5.
  niedrigster ORIGIN (IGP, EGP, incomplete), 6. niedrigste MED unter Wegen vom selben
  Nachbar-AS (ohne MED: 0), 7. extern vor intern, 8. niedrigste IGP-Metrik zum Next Hop, 9.
  niedrigste BGP-ID des Nachbarn, 10. niedrigste Nachbaradresse. Den Schritt „ältester externer
  Weg“ von IOS gibt es nicht. Der beste Weg kommt mit Distanz 20 (extern) oder 200 (intern) und
  seiner MED als Metrik ins RIB, mit seinem Next Hop, den die Weiterleitung über das IGP
  auflöst (`show ip route bgp`: B). Hat eine andere Quelle das Präfix mit besserer Distanz,
  zeigt `show ip bgp` ein `r`.
- **Weitergeben:** Jeder Nachbar in Established bekommt den besten Weg jedes Präfixes, außer
  dem, von dem er kam; was ein interner Nachbar brachte, geht nicht an andere interne (kein
  Route Reflector). Externe Nachbarn bekommen das eigene AS vorn im Pfad, die Adresse der
  Sitzung als Next Hop, keine LOCAL_PREF und eine MED nur für Wege aus dem eigenen AS; interne
  den Next Hop des Wegs (mit `next-hop-self` und für eigene Wege die Adresse der Sitzung),
  LOCAL_PREF und MED. Gleiche Attribute teilen sich ein UPDATE (bis 4096 Bytes). Rücknahmen
  gehen sofort hinaus, neue Wege höchstens alle 30 s an einen externen Nachbarn
  (MinRouteAdvertisementInterval wie IOS, je Nachbar statt je Präfix; intern ohne Pause), die
  erste Tabelle nach Established sofort.
- **Anzeigen:** `show ip bgp` (Tabellenversion, Router-ID, die Spalten Network, Next Hop, Metric,
  LocPrf, Weight, Path mit `*`, `>`, `i`, `r`), `show ip bgp A.B.C.D` (die Wege eines Netzes mit
  dem entscheidenden Schritt), `show ip bgp summary` (die Tabelle des Vorbilds mit Zuständen wie
  `Idle (Admin)`), `show ip bgp neighbors [X [routes|advertised-routes]]`, `show ip route bgp`,
  `show ip protocols`, `debug ip bgp [events|updates]` (in eigenen Worten).
- **Ereignisse:** Jede Entscheidung – Router-ID, Zustände, Verbindungen (geöffnet, angenommen,
  zurückgesetzt, gescheitert), OPEN und was es vereinbart, KEEPALIVE, UPDATE, ROUTE-REFRESH und
  NOTIFICATION in beide Richtungen, Kollisionen, der Hold Timer, das Ende einer Sitzung mit
  Grund, verworfene Wege, beste Wege mit ihrem Schritt, verlorene, eigene und die Pause vor neuen
  Wegen – steht mit deutschem Text in der Ereignisliste und im Paket-Tab
  ([`bgptrace.rs`](crates/jfn_sim/bgptrace.rs), englisch in `engine.tsv`). Der Simulationsmodus
  zeigt BGP braun wie OSPF und RIP, der Paket-Tab zerlegt jede Nachricht eines Segments
  ([`netpkt::bgp`](../../lib/netpkt/bgp.rs), [`decode.rs`](../../lib/netpkt/decode.rs)).
- **Probleme:** ein Nachbar, der nicht Established wird, mit dem Grund, den die Konfiguration
  beider Seiten verrät: nicht direkt verbunden ohne `ebgp-multihop`, keine Route, kein Gerät mit
  der Adresse, dort kein BGP, das falsche AS, die Gegenseite kennt diesen Router nicht, die
  Adressen von `update-source` passen auf keiner Seite; sonst die letzte NOTIFICATION. Dazu ein
  `network` ohne seine Route (Warnung).

Noch offen: Route-Maps, Präfix-Listen und andere Richtlinien (`neighbor … route-map`,
`distribute-list`, `filter-list`, `prefix-list`; es gibt sie im Simulator noch nicht), eigene
Communities, `aggregate-address`, Route Reflectors, Konföderationen, Peer-Gruppen,
MD5-Authentifizierung, `maximum-prefix`, BGP über IPv6 und andere Adressfamilien, Graceful
Restart, Dampening, Synchronisation und mehrere gleich gute Wege.

Für Programme: `Action::Bgp { dev, set: BgpSet }` (`Process`, `RouterId`, `LogChanges`, `Timers`,
`LocalPref`, `Network`, `Redistribute`, `RemoteAs`, `Descr`, `UpdateSource`, `NextHopSelf`,
`Multihop`, `Shutdown`, `Weight`, `Clear`) und `Engine::bgp(dev)` mit `cfg`, `router_id()`,
`peers()` / `peer(addr)` (Zustand, was vereinbart ist, Zähler, Adj-RIB-In und -Out),
`loc_rib()` (Kandidaten, bester Weg, Schritt), `local()` und `version()`; die Notizen als
`Note::Bgp(BNote)`. Geprüft wird BGP in der Selbsttest-Gruppe `bgp`
([`src/bgptest.rs`](src/bgptest.rs)), die Nachrichten in der coretest-Gruppe `bgp`, dazu das
Szenario `bgp_isp.scn` (siehe [Szenarien](#szenarien-scn)) und das Fensterskript
`bgp_console.uis`.

### Dateien, TFTP und FTP

Jedes Gerät hat Dateien ([`crates/jfn_model/files.rs`](crates/jfn_model/files.rs)), die mit dem
Projekt gespeichert werden und die der Simulator mit ihm abgleicht wie die Startkonfiguration:
was eine Konsole oder ein Dienst schreibt, kommt als `Report::Files` an die Oberfläche und wird
ein Rückgängig-Schritt; Rückgängig und Wiederholen ändern die Dateien auch im laufenden Gerät
(`Action::Files`, ohne Neustart). Wie viel Platz ein Gerät hat, sagt sein Modell
(`capacity_of`: 256 MiB beim Router, 128 MiB beim Switch, 512 GiB beim PC, 2 TiB beim Server …);
was nicht mehr passt, wird ganz abgelehnt.

- **IOS-artige Geräte:** `flash:` mit Ordnern (seit V1, siehe
  [Die Shell: Ordner, Werkzeuge, Prozesse](#die-shell-ordner-werkzeuge-prozesse)) und dem Systemabbild, das `show version`
  nennt (`jf-r4.bin` mit 40 MiB beim JF-R4: es belegt Platz, hat aber keinen Inhalt und lässt
  sich weder löschen noch exportieren), und `nvram:startup-config`, das die Startkonfiguration
  selbst ist und keine Datei daneben.
- **Linux-Hosts:** `/`, `/etc`, `/home`, `/home/user`, `/root` (das `~` des Prompts), `/srv`,
  `/srv/ftp`, `/srv/tftp`, `/tmp`, `/var` und `/var/log` gibt es von Anfang an; die Dateien in
  `/etc`, die die Shell kennt (`resolv.conf`, `hosts`, `hostname`), entstehen weiter aus dem
  Zustand des Geräts.
- **IOS-artige Befehle** ([`cli/files.rs`](crates/jfn_sim/cli/files.rs)): `dir [flash:|nvram:]`,
  `show flash:`, `more DATEI`, `delete [/force] DATEI` (fragt nach), `copy QUELLE ZIEL` mit
  `running-config`, `startup-config`, `flash:NAME`, `tftp://HOST/NAME`,
  `ftp://[BENUTZER[:PASSWORT]@]HOST/NAME` und den kurzen Formen `flash:`, `tftp:` und `ftp:`,
  nach denen die Konsole Host und Namen erfragt (in Klammern die Vorgabe: der zuletzt genutzte
  Host, für eine Konfiguration `HOSTNAME-config`), `ip ftp username|password` für die
  Anmeldung beim FTP-Server (in der Running-Config). Eine Konfiguration nach `running-config`
  wird eingespielt, als wäre sie getippt: sie ergänzt und ändert, was sie nennt, wie beim
  Vorbild, und meldet sich mit `%SYS-5-CONFIG_I`; eine nach `startup-config` wird die
  Startkonfiguration (ein Rückgängig-Schritt wie `copy run start`); eine Datei in `flash:` wird
  erst nach „y“ ersetzt. Die Fragen, der Fortschritt (`#` je Block, `.` je Timeout) und das
  Ergebnis (`Done: … bytes in … ms`) sind eigene Worte, Fehler kurze Zeilen wie `%Error opening
  tftp://10.0.0.10/nope (File not found)`. Strg+Umschalt+6 bricht eine Übertragung ab.
- **Shell:** `ls [-l] [-a]`, `cat`, `cp`, `rm [-f] [-r]`, `mkdir [-p]`, `rmdir`, `touch`, `echo
  … > DATEI` und `>> DATEI` mit den Meldungen von coreutils und bash, `tftp -g|-p [-l DATEI] [-r
  DATEI] [-b GRÖSSE] HOST [PORT]` in der Art von busybox und `curl` mit `ftp://` (`-u`, `-T`,
  `-o`, `-O`, `-l`, `-P`, `--disable-epsv`, `-v` mit jedem Befehl und jeder Antwort, die
  Exit-Codes von curl wie 67 für eine abgelehnte Anmeldung oder 78 für eine fehlende Datei).
- **TFTP-Server** (Dienst `tftp-server`, [`tftpd.rs`](crates/jfn_sim/tftpd.rs)): UDP-Port 69,
  die Dateien unter `root` (`/srv/tftp`, auf IOS-artigen Geräten `flash:`), nur zum Lesen oder
  mit `writable` auch zum Schreiben, `overwrite`, `quota_kb`. Jede Übertragung bekommt einen
  eigenen Port; die Blöcke gehen im Gleichschritt mit ihren ACKs, ein fehlendes ACK holt den
  Block nach `timeout_ms` (1 s) noch einmal, nach `retries` (5) gibt der Server auf, ein
  doppeltes ACK schickt nichts doppelt, das letzte ACK bleibt eine Weile bereit. Die Fehler 1
  bis 6 und 8 (eine fehlende Datei, kein Zugriff, kein Platz, ein fremdes Paket, ein
  unbekannter Port, eine vorhandene Datei, eine abgelehnte Option), die Optionen `blksize`,
  `timeout` und `tsize` (abschaltbar mit `options`), `netascii`. Lesen und Schreiben kosten
  Zeit auf der Disk des Geräts.
- **FTP-Server** (Dienst `ftp-server`, [`ftpd.rs`](crates/jfn_sim/ftpd.rs)): TCP-Port 21 in der
  Art von vsftpd: Benutzer mit Passwort und Heimatverzeichnis, in das sie eingeschlossen sind
  (`users`), anonym mit `anonymous` (Schreiben mit `anon_write`); ein falsches Passwort wird
  nach `fail_delay_ms` abgelehnt, nach `max_fails` endet die Verbindung. Passiv (PASV, EPSV)
  und aktiv (PORT, EPRT von Port 20, abschaltbar mit `active`), dazu USER, PASS, LIST, NLST,
  RETR, STOR, APPE, DELE, MKD, RMD, CWD, CDUP, PWD, TYPE, SIZE, MDTM, SYST, FEAT, NOOP, ABOR und
  QUIT; ein Leerlauf-Timeout (`idle_ms`, 300 s), eine Quote (`quota_kb`). Die Antworten haben
  die Codes und Formate der RFCs, ihre Texte sind eigene Worte.
- **Im Fenster:** der Reiter Dateien des Gerätefensters
  ([`devwin/files.rs`](src/workspace/devwin/files.rs)) listet die Dateien mit Größe und Zeit,
  zeigt den freien Platz und den Text der gewählten Datei, löscht als Rückgängig-Schritt,
  exportiert eine Datei auf den Rechner und importiert eine in das gewählte Verzeichnis. Die
  Server stehen im Reiter Dienste mit Statuszeile und Zahlen, ihre Einstellungen als JSON. Der
  Paket-Tab zerlegt TFTP und die Steuerverbindung von FTP Feld für Feld.
- **Notizen und Probleme:** jede Anfrage, Wiederholung, Zeitüberschreitung, jeder Fehler und
  jedes Ende als Notiz (`Note::Xfer`) und als Log-Zeile (`tftpd`, `ftpd`); die Probleme-Liste
  nennt einen Server, dessen Verzeichnis fehlt.

Entscheidungen: Eine Konfiguration, die nach `running-config` kopiert wird, startet nichts neu
(Interfaces bleiben, wie sie sind; was sie nicht nennt, bleibt stehen). Das Systemabbild ist
nur ein Eintrag mit Größe; gebootet wird es nicht. Die Daten einer passiven FTP-Verbindung
laufen über einen beliebigen Port und erscheinen im Paket-Tab als TCP. Bei einem Ausfall
während einer Übertragung bleibt nichts Halbes liegen: die Datei landet erst mit ihrem letzten
Block. Unter Linux überträgt `curl` per FTP; ein eigenes `ftp`-Programm gibt es nicht.

Noch offen: Unterverzeichnisse in `flash:`, `cd` und `mv` in der Shell, der IOS-Befehl
`tftp-server`, die Notizen des Desktops (`notizen.txt`) als Datei, und die weiteren Dienste von
X1: SNMP und Mail (AAA mit RADIUS ist fertig, siehe unten).

Für Programme: `Action::Files { dev, files }`, `Report::Files { dev, at, path, files }`,
`Engine::files(dev)`, `Engine::file_store(dev)`, für Dienste `OsApi::file_read`, `file_write`,
`file_append`, `file_remove`, `file_list`, `file_stat`, die Notizen `Note::Xfer(XNote)`.
Geprüft wird in der Selbsttest-Gruppe `files` ([`src/filestest.rs`](src/filestest.rs)), TFTP
und FTP Byte für Byte in den coretest-Gruppen `tftp` und `ftp`, dazu das Szenario `backup.scn`
und die Fensterskripte `files_window.uis`, `files_services.uis`, `files_console.uis` und
`files_shots.uis`.

### Die Shell: Ordner, Werkzeuge, Prozesse

Die Shell der Hosts und Server (`jfn-sh`, [`cli/sh.rs`](crates/jfn_sim/cli/sh.rs),
[`cli/shrun.rs`](crates/jfn_sim/cli/shrun.rs), [`cli/coreutils.rs`](crates/jfn_sim/cli/coreutils.rs))
fühlt sich an wie die eines kleinen Linux-PCs. Alles, was zu einer Sitzung gehört –
Arbeitsverzeichnis, Variablen, Verlauf, Jobs –, steht zusammen in `CliDev::sh`; ein Gerät, das
ausgeschaltet wird, verliert es samt dem Befehl im Vordergrund und beginnt danach mit einer
frischen Shell.

**Ordner.** Jede Sitzung beginnt in `/root` (`~`, `$HOME`); der Prompt zeigt das Verzeichnis wie
die Vorgabe von bash: `root@PC1:~# `, `root@PC1:~/app# `, `root@PC1:/srv/ftp# `. `cd` geht ohne
Argument nach Hause, mit `-` zurück (und zeigt das Ziel), nimmt `..`, `~`, relative und absolute
Pfade; Fehler sehen aus wie bei bash (`jfn-sh: cd: x: No such file or directory`, `… Not a
directory`, `too many arguments`, `OLDPWD not set`). `pwd`, `$PWD` und `$OLDPWD` stimmen. Jeder
Befehl mit Pfaden nimmt relative (auch `ls`, `cat`, `cp`, `rm`, `mkdir`, `touch`, `tftp -l`,
`curl -o` und `-T`), die Meldungen nennen den Pfad wie getippt.

**Die Grammatik** gilt für jeden Befehl:

- Listen `;`, `&&`, `||` und `&` (im Hintergrund), Pipelines `|`, `!` vor einer Pipeline;
- Umleitungen `>`, `>>`, `<`, `2>`, `2>>`, `2>&1`, `1>&2`, `&>`, `&>>` und `/dev/null`; die
  Reihenfolge zählt wie bei bash (`> f 2>&1` gegen `2>&1 > f`). Eine Datei, die sich nicht
  schreiben lässt, lehnt die Shell vor dem Befehl ab (`jfn-sh: /x/y: No such file or
  directory`); geschrieben wird, wenn der Befehl endet – ein Schritt des Projekts wie jede
  geänderte Datei, `/etc/resolv.conf` stellt wie bisher den Resolver ein;
- Wörter: `'…'` wörtlich, `"…"` mit `$`-Ersetzung und `\` vor `$ " \`, `\` außerhalb von
  Anführungszeichen, `#` am Wortanfang beginnt einen Kommentar;
- Variablen: `NAME=WERT`, `NAME=WERT befehl` (nur für diesen Befehl), `$NAME`, `${NAME}`, `$?`,
  `$$` (100, die Shell), `$!`, `$0`, `$#`, `~` und `~root`; unquotierte Ersetzungen werden an
  Leerraum geteilt;
- Platzhalter `*`, `?` und `[…]` (mit `!`/`^` und Klassen) über die Dateien des Geräts,
  Komponente für Komponente, Dateien mit Punkt nur mit Punkt im Muster, sortiert nach Bytes wie
  bash mit `LC_COLLATE=C`; ohne Treffer bleibt das Wort stehen;
- `!!`, `!N` und `!-N` aus dem Verlauf (die Zeile wird gezeigt), `history [N] | -c`;
- Exit-Codes wie bei den Vorbildern: 0, 1 bei einem Fehler, 2 bei falschem Aufruf, 127 für einen
  unbekannten Befehl, `grep` 1 ohne Treffer, `ping` 1 ohne Antwort und 2 bei einem unbekannten
  Namen, `curl` seine Codes, 130 nach Strg+C, 128 + Signal nach `kill`.

Nicht dabei: Unterschalen `( )`, `$( )` und Backticks, Arithmetik, Funktionen, `if`, `for` und
`while` – diese Zeichen sind gewöhnliche (`ss`-Filter mit Klammern gehen deshalb wie bisher).
Pipes sammeln: die rechte Seite bekommt die Ausgabe der linken, wenn diese endet (ein `ping` links
liefert also am Ende); ein Befehl, der über die Zeit läuft (`ping`, `curl`, `nc`, `dig`,
`traceroute`, `tftp`, `snmpwalk`, `telnet`, `sleep`, `jfs` …), hält die Zeile an seinem Platz.
Strg+C beendet ihn; der Rest seiner Pipeline filtert noch, was gesammelt war (`ping x | grep ttl`
zeigt die Antworten), der Rest der Zeile entfällt, `$?` ist 130. Fehlermeldungen von `ls`, `cat`,
`rm`, der Netzwerkprogramme bei ihrem Start, der Skripte und der Shell gehen nach stderr; was ein
Programm später während seines Laufs meldet (eine Zeile von `ping`), steht in seiner Ausgabe.

**Die Werkzeuge** verhalten sich, wo man es sieht, wie GNU coreutils, grep und findutils (die
gebräuchlichen Optionen, Exit-Codes, die Form der Meldungen): `mv` (`-n`, in Ordner, Ordner mit
Inhalt), `head` und `tail` (`-n N`, `-N`, `-n -N`, `-n +N`, `-c`), `grep` (`-i -v -n -c -l -r -E -F
-w -x -o -q -H -h -e`, `egrep`), `find` (`-name`, `-iname`, `-type f|d`, `-maxdepth`,
`-mindepth`; in Namensfolge, Dateien mit Punkt eingeschlossen), `wc` (`-l -w -c -m` mit den
Spaltenbreiten von GNU), `sort` (`-r -n -u -f -k -t`, nach Bytes), `uniq` (`-c -d -u -i`), `cut`
(`-d -f -c -s`), `tree` (`-a -d -L`), `du` (`-s -h -a -c`), `df` (`-h`: die Kapazität des
Gerätemodells), `less` und `more` (die Konsole blättert mit --More--), `file`, `stat`,
`basename`, `dirname`, `date` (die Uhr des Geräts in virtueller Zeit, `+FORMAT`), `sleep`, `seq`,
`tee`, `test` / `[`, `xargs`, `env`, `export`, `unset`, `which`, `type`, `help` und `man`.
`ls` zeigt Dateien mit Punkt nur mit `-a`/`-A` und schreibt in eine Pipe einen Namen je Zeile.
`grep` versteht reguläre Ausdrücke nach POSIX ohne Rückverweise (`base::pattern`, eine Pike-VM
ohne exponentielle Fälle): `. [abc] [^a-z] [[:digit:]] ^ $ *`, als BRE `\( \) \| \{m,n\} \+ \?`,
mit `-E` dieselben ohne Backslash, dazu `\w \W \s \S \< \> \b \B`; die Fehlermeldungen lauten wie
bei GNU grep. Lesen ohne Datei heißt: aus der Pipe oder von `<` – die Konsole ist kein Terminal,
aus dem ein Programm lesen könnte. `help` listet alle Befehle mit einer Zeile, `man BEFEHL` zeigt
eine kurze Seite (NAME, AUFRUF, BESCHREIBUNG, OPTIONEN, WO), `man sh` die Shell selbst, `BEFEHL
--help` den Aufruf; diese Texte sind eigene deutsche Worte ([`cli/man.rs`](crates/jfn_sim/cli/man.rs)),
`which` und `type` nennen Pfade wie `/usr/bin/grep` (die Shell kennt sie, im Dateibaum stehen
sie nicht).

**Tab** ergänzt Befehlswörter (die Liste von `linux.rs`), die Objekte von `ip` und sonst Pfade
relativ zum Arbeitsverzeichnis (auch `~/`, `/`, `..`): ein Ordner bekommt ein `/`, eine Datei ein
Leerzeichen, mehrere Treffer erst ihren gemeinsamen Anfang und mit dem nächsten Tab die Liste;
Dateien mit Punkt nur, wenn der Anfang einen Punkt hat, Leerzeichen in Namen mit `\`.

**Prozesse.** `jfs DATEI [ARGUMENTE]` startet ein Skript aus den Dateien des Geräts als Prozess
im Terminal ([`cli/shrun.rs`](crates/jfn_sim/cli/shrun.rs) `JfsProc`): derselbe `ScriptService`
mit demselben Host wie ein Dienst (net, http, dns, timer, disk, metrics, `log` ins Journal des
Geräts), `print` schreibt ins Terminal (und in Pipes und Umleitungen), `config.args` hält die
Argumente, `config.file` den Pfad; `import("lib/util.jfs")` gilt ab dem Ordner der Datei
(`ScriptService::new_file`). Das Programm endet, wenn nichts mehr zu tun ist (kein Timer,
Socket, Server, keine offene Anfrage; Status 0), nach einem Fehler (`DATEI:ZEILE:SPALTE: Fehler:
…` auf stderr, Status 1) oder mit Strg+C (130); ein Übersetzungsfehler meldet sich wie beim Runner
`jfs`. `BEFEHL &` läuft im Hintergrund: sofort `[N] PID`, die Ausgabe kommt ins Terminal, wann
immer sie entsteht, und vor dem nächsten Prompt `[N]+  Done                    BEFEHL` (`Exit N`,
`Terminated`, `Killed`) wie bei bash. `jobs [-l] [-p]` zeigt die Jobs, `fg [%N]` und `wait`
warten auf einen (Strg+C beendet ihn), `kill [-SIGNAL] PID|%N` beendet ihn (TERM, KILL, INT, HUP; `kill -l`),
`sleep` läuft als Job über die virtuelle Zeit. `ps` zeigt init (1), die Shell (100), die
laufenden Dienste mit 100 + ihrer Nummer – dieselben PIDs wie `top` und `ss -p` – mit ihrer
Rechenzeit und die Jobs; ein Job ohne eigenen Dienst bekommt eine Nummer aus demselben Vorrat.
Dienste beendet `kill` nicht (`Operation not permitted`), sie haben ihre eigenen Befehle. Die
Jobs gehören zum Zustand des Geräts (Keyframes zählen sie), laufen deterministisch und enden mit
dem Strom.

**IOS-artig** haben `flash:` jetzt Ordner: `mkdir`, `rmdir`, `rename QUELLE ZIEL` (auch in einen
Ordner), `cd [ORDNER|..]` und `pwd` (`flash:/sicherung/`) mit einem Arbeitsverzeichnis je
Sitzung; `dir` ohne Argument zeigt dieses (Ordner als `drw-`), bloße Namen in `copy`, `delete`,
`more`, `dir` und in den Fragen von `copy` gelten darin, `flash:NAME` vom Anfang von `flash:`.
Die Zeilen von `mkdir` und `rmdir` sind eigene Worte, die `%Error`-Zeilen folgen dem Vorbild.
Ordner in `flash:` stehen im Projekt wie die der Hosts (`{"path": "flash:/sicherung", "dir":
true}`).

### Logins, Telnet und AAA mit RADIUS

IOS-artige Geräte kennen jetzt Passwörter. Bis hierher fragte nichts danach; eine
Konfiguration ohne `login`-Zeilen, `aaa new-model` oder Benutzer öffnet ihre Konsole
weiterhin genau wie bisher.

- **Lokale Anmeldungen** ([`crates/jfn_sim/auth.rs`](crates/jfn_sim/auth.rs),
  [`cli/aaa.rs`](crates/jfn_sim/cli/aaa.rs)): `enable secret` und `enable password` (auch
  `level N`), `username NAME [privilege N] secret|password …`, auf den Leitungen `password`,
  `login`, `login local`, `exec-timeout`, `banner motd`, `service password-encryption`;
  Rechtestufen 1 und 15 mit `enable`/`disable`. Die Konsole (Terminal-Tab und Gerätefenster)
  zeigt `Username:` und `Password:` und tippt das Passwort ohne Echo (drei Versuche, eigene
  Worte bei Fehlern). `show users`, `show aaa servers`, `show radius statistics`.
- **Gespeicherte Geheimnisse mit Typnummer in der Running-Config:** `enable secret` und
  `username … secret` werden mit PBKDF2-SHA-256 gehasht (Salz aus dem Zufallsstrom des
  Geräts, 1000 Runden), angezeigt als `8 $8$<salt>$<hash>` (eigene Base64). Ein `password`
  bleibt lesbar (Typ 0), es sei denn, `service password-encryption` schreibt es als Typ 7 um
  (eine umkehrbare, eigene Verschleierung, kein Herstellerverfahren). Eine gespeicherte
  Konfiguration bootet gleich (die Hashes bleiben, nichts wird neu gehasht).
- **Telnet** ([`lib/netpkt/telnet.rs`](../../lib/netpkt/telnet.rs),
  [`telnetd.rs`](crates/jfn_sim/telnetd.rs)): der NVT-Codec (Befehle, Optionsaushandlung
  RFC 854/855/857/858, IAC-Escaping, ein Leser für Bytes in Stücken) mit Zerlegung im
  Paket-Tab (Port 23); der Client `telnet HOST [PORT]` auf der IOS-artigen Konsole und in der
  Linux-Shell verbindet sich, gibt die Gegenseite aus und meldet `% Connection refused by
  remote host` und ähnliche `%`-Zeilen; die Flucht ist Strg+Umschalt+6.
- **AAA mit RADIUS** ([`aaa.rs`](crates/jfn_sim/aaa.rs),
  [`radiusd.rs`](crates/jfn_sim/radiusd.rs)): `aaa new-model`, `radius server NAME` mit
  `address ipv4 ADDR [auth-port P] [acct-port P]`, `key`, `timeout`, `retransmit` (und das
  ältere `radius-server host ADDR key SECRET`), `aaa group server radius NAME`,
  `aaa authentication login {default|LISTE} METHODE…` (group radius | group NAME | local |
  line | enable | none), `aaa authentication enable`, `aaa authorization exec` (die
  Rechtestufe aus der Antwort), `aaa accounting exec default start-stop group radius`,
  `login authentication LISTE` auf einer Leitung, `test aaa group radius USER PASS legacy`.
  Die nächste Methode wird nur versucht, wenn ein Server nicht antwortet (nach seinen
  Wiederholungen), nie nach einem Reject. Die NAS-Seite ist der verborgene Dienst
  `radius-nas` (eigener Socket und Timer), das Verstecken des User-Password nach RFC 2865
  §5.2, der Request- und Response-Authenticator und der Message-Authenticator nach RFC 3579,
  MD5 und HMAC-MD5 aus `lib/crypto`.
- **RADIUS-Server auf Hosts** (Dienst `radius-server`, UDP 1812 / 1813): Clients (Adresse
  oder Präfix und ein gemeinsames Geheimnis), Benutzer (Passwort, Rechtestufe für die
  Antwort, optionaler Text), Accounting in die Dateien des Geräts
  (`/var/log/radius/accounting`); ein unbekannter Client oder ein falscher
  Message-Authenticator wird still verworfen (mit Log-Zeile), wie RFC 2865 es verlangt. Die
  Einstellungen stehen als JSON im Dienste-Editor und im Inspektor.
- **Debug und Notizen:** `debug aaa authentication`, `debug radius`; jede Anmeldung,
  Ablehnung, Anfrage, Antwort, Zeitüberschreitung, jeder Rückfall und jedes
  Accounting-Paket sind eine Notiz (`Note::Aaa`, [`aaatrace.rs`](crates/jfn_sim/aaatrace.rs))
  und eine Log-Zeile. Die Probleme-Liste bleibt dafür noch offen.

Entscheidungen: Die Rechtestufe kommt bei RADIUS aus der Antwort (Service-Type
Administrative-User → 15, sonst 1, oder ein eigenes Vendor-Attribut `priv-lvl=N`), wenn
`aaa authorization exec` sie anfordert, sonst 1. Ein einstelliges Zahlenpasswort gibt man
als `password 0 ZIFFER` ein (eine führende Ziffer ist sonst eine Typnummer, wie beim
Vorbild).

Der **vty-Telnet-Server** (eine Sitzung je vty-Leitung, `transport input`, die Anmeldung der
Leitung über dieses Gate und AAA, Sitzungslimit, Leerlaufzeit) kam mit X1e, siehe
[Sitzungen und Telnet-Server](#sitzungen-und-telnet-server); 802.1X und WPA-Enterprise mit X1g,
siehe [802.1X und WPA-Enterprise](#8021x-und-wpa-enterprise).

Für Programme: `Engine::cli_secret(dev)` (das Passwort ohne Echo), die CLI-Zustände in
`CliDev` (`auth`, `aaa`, `gate`, `priv_level`), `netpkt::telnet`, `netpkt::radius`,
`crypto::{md5, hmac_md5}`. Geprüft wird in der Selbsttest-Gruppe `aaa`
([`src/aaatest.rs`](src/aaatest.rs)), Telnet und RADIUS Byte für Byte in der coretest-Gruppe
`radius` (MD5 und HMAC-MD5 in `crypto`), dazu das Szenario `aaa_radius.scn` und die
Fensterskripte `login_console.uis`, `telnet_session.uis`, `aaa_services.uis` und
`aaa_shots.uis`.

### Physische Ansicht

Neben der logischen Zeichenfläche hat das Fenster eine zweite Ansicht: wo die Geräte stehen
([`jfn_model/phys.rs`](crates/jfn_model/phys.rs), [`jfn_sim/phys.rs`](crates/jfn_sim/phys.rs),
[`src/workspace/phys/`](src/workspace/phys/mod.rs), [`src/doc/phys.rs`](src/doc/phys.rs)).
Umgeschaltet wird mit „Logisch | Physisch“ in der Werkzeugleiste oder über Ansicht › Logische
Ansicht (Strg+Umschalt+L) und Physische Ansicht (Strg+Umschalt+P); die Wahl und der gezeigte
Ort werden mit dem Layout gespeichert.

- **Orte:** ein Baum aus Standorten (ein Campus, eine Stadt), Gebäuden, Etagen, Räumen und in
  den Räumen Racks (Höhe in Höheneinheiten, ohne Angabe 42) und Tischen. Position (die linke
  obere Ecke im übergeordneten Ort, nach Osten und Süden) und Größe stehen in Dezimetern; Etagen
  liegen übereinander (ihre Nummer, 0 das Erdgeschoss) und haben den Grundriss ihres Gebäudes.
  In der Datei `places`, zum Beispiel `{"id": 5, "kind": "rack", "name": "Rack 1", "parent": 4,
  "x_dm": 10, "y_dm": 10, "w_dm": 6, "d_dm": 10, "units": 42}`; Orte zählen ihre Nummern
  eigenständig, Geräte und Kabel behalten ihre.
- **Geräte:** `phys` nennt den Raum, das Rack (mit der untersten Höheneinheit, die das Gerät
  belegt) oder den Tisch, im Raum auch einen Platz (`"phys": {"place": 5, "unit": 20}`,
  `{"place": 11, "x_dm": 50, "y_dm": 40, "placed": true}`). Jedes Modell hat eine Bauform:
  Rack-Gerät (1 HE, Core-Router, DB-Shard und Container-Host 2 HE), Tischgerät, Mobilgerät,
  Decken- oder Wandgerät (Access Point, Kamera, Sensor), Standgerät (Tower-Server) oder ohne Ort
  (die Internet-Wolke); nur Rack-Geräte kommen in ein Rack.
- **Ältere Projekte:** Ohne `places` gilt der Baum Standort › Gebäude › Erdgeschoss › Raum, alle
  Geräte stehen ohne Platz in diesem Raum, jedes Kabel ist 7 m lang (zu einem Access Point oder
  einer Kamera an der Decke 4 m, zur Wolke 3 m) und alles läuft wie bisher. Die erste Änderung in der physischen Ansicht speichert den Baum.
- **Die Ansicht:** oben Zurück (Umschalt+Rücktaste), Eine Ebene höher (Rücktaste) und die
  Brotkrumen „Alle Standorte › Campus › Haus A › EG › Serverraum“, rechts Ort umbenennen (F2)
  und entfernen (Entf: seine Geräte kommen ohne Platz in den ersten Raum, der letzte Raum
  bleibt), darunter die Orte, die hier dazukommen können. Alle Standorte und ein Standort sind
  Karten, ein Gebäude zeigt seine Etagen übereinander, eine Etage ihren Grundriss mit Räumen,
  Racks, Tischen und frei stehenden Geräten, ein Raum seinen Plan (Racks, Tische mit ihren
  Geräten, Geräte an ihrem Platz, darunter die Ablage „Im Raum, ohne Platz“), ein Rack seine
  Vorderseite mit den Höheneinheiten von 1 unten an und den Geräten in ihren Einschüben, rechts
  daneben die Rack-Geräte des Raums, die in keinem Rack stecken. Doppelklick oder Eingabe öffnet
  einen Ort; ein Kabel, dessen anderes Ende woanders liegt, führt zu einer Karte „→ Haus B“ am
  rechten Rand, die dorthin führt. Zwischen Standorten, Gebäuden und Etagen stehen die Kabel
  zusammengefasst („2 Kabel · 203–208 m · 1 zu lang oder falsch“), auf Plänen von Etagen und
  Räumen einzeln mit ihrer Länge.
- **Ziehen:** Orte verschieben sich auf ihrem Plan; Geräte gehen im Raum an einen Platz, auf
  einen Tisch, in ein Rack (die unterste freie Höheneinheit) oder zurück in die Ablage, auf
  einer Etage in einen anderen Raum, im Rack in andere Höheneinheiten oder nach rechts hinaus.
  Jedes Ziehen ist ein Rückgängig-Schritt; eine Ablehnung (ein Tischgerät ins Rack, belegte
  Einheiten) stellt alles zurück und sagt über dem Plan, warum. Die Kabel bleiben an ihren Ports,
  ihre automatischen Längen folgen.
- **Inspektor:** ein Ort mit Name, Lage, Größe, Höhe des Racks oder Nummer der Etage (Setzen
  ist ein Schritt), Öffnen und Entfernen; ein Gerät zuletzt mit Ort und Platz, einer Liste aller
  Räume, Racks und Tische zum Umstellen, der Höheneinheit, „Im Plan zeigen“ und der Optik jedes
  Schachts; ein Kabel zuletzt mit seiner Länge (automatisch oder von Hand; „0“ oder Automatisch
  macht sie wieder automatisch), der Grenze seines Mediums mit der Norm, von wo nach wo und der
  Laufzeit aus der Länge.
- **Tastatur und Screenreader:** wie auf der logischen Zeichenfläche (Strg+9, Tab über Orte,
  Geräte und Wege, dann die Kabel, Pfeiltasten verschieben, Eingabe öffnet); die Einträge
  heißen „Rack Rack 1, 2 Geräte“, „SW1, Rack 1, HE 20“, „Weg nach Gebäude Haus B“ und „Kabel PC4
  eth0 – SW1 Gi1/0/5, 208 m, erlaubt 100 m (1000BASE-T), zu lang“.

**Kabellänge:** von Hand (`len_m` des Kabels) oder automatisch aus den Orten beider Enden, in
ganzen Metern aufgerundet. Ein Ende steht in der Mitte seines Racks, seines Tisches, seines
Platzes oder (ohne Platz) seines Raums, in einer Höhe über seiner Etage: im Rack 0,1 m plus
44,45 mm je Höheneinheit darunter, auf dem Tisch 0,75 m, an der Decke 2,8 m, sonst am Boden. Im
selben Rack ist es ein Patchkabel von 0,5 m plus dem Höhenunterschied, auf demselben Tisch 2 m;
sonst läuft das Kabel hinauf zu den Trassen unter der Decke (2,8 m), im rechten Winkel über die
Pläne (|Δx| + |Δy| in den Koordinaten aller Standorte), 3,5 m je Etage (zwischen Gebäuden
hinunter ins Erdgeschoss und wieder hinauf) und hinunter zum anderen Ende, dazu 10 % Reserve.
Die Internet-Wolke hat keinen Ort (3 m), Funk keine Länge.

**Grenzen** nach den Reichweiten-Tabellen von IEEE 802.3 (Kupfer als Cat6A, Multimode als OM3; ein Kabel mit Kategorie: siehe [Kabel](#kabel)),
für die Speed, mit der das Kabel läuft (die kleinere beider Ports und seiner Einstellung):

| Medium | Speed | Norm | Grenze |
|---|---|---|---|
| Kupfer (gerade, gekreuzt) | 10M bis 10G | 10BASE-T, 100BASE-TX, 1000BASE-T, 2.5GBASE-T, 5GBASE-T, 10GBASE-T | 100 m |
| Kupfer | 25G, 40G | 25GBASE-T, 40GBASE-T | 30 m |
| Glasfaser, Multimode (SR-Optik) | 1G / 10G / 25G / 40G / 100G | 1000BASE-SX / 10GBASE-SR / 25GBASE-SR / 40GBASE-SR4 / 100GBASE-SR4 | 550 / 300 / 70 / 100 / 70 m |
| Glasfaser, Singlemode (LR-Optik) | 1G / 10G bis 100G | 1000BASE-LX / 10GBASE-LR, 25GBASE-LR, 40GBASE-LR4, 100GBASE-LR4 | 5 km / 10 km |
| Konsole | | RS-232 bei 9600 bit/s | 15 m |
| Seriell | bis 2,4 / 4,8 / 9,6 / 19,2 / 38,4 / 64 kbit/s, darüber (T1, E1) | V.35, EIA-530 | 1250 / 625 / 312 / 156 / 78 / 51 m, 15 m |
| WAN-Leitung, Funk | | | keine |

**Optiken:** Jeder SFP-, SFP+-, SFP28- und QSFP28-Schacht hat ab Werk eine SR-Optik (Multimode,
850 nm); im Inspektor lässt sich eine LR-Optik (Singlemode, 1310 nm) einsetzen (`optics` des
Geräts, nur was abweicht). Ein automatisches Kabel folgt: LR an beiden Enden ist Singlemode,
sonst Multimode. Optiken, die nicht zueinander oder zur Faser passen (SR an einem Ende, LR am
anderen; eine von Hand gewählte Singlemode-Faser zwischen SR-Optiken), stecken zwar, die Leitung
bleibt aber aus. Das ändert nur Projekte mit einer solchen Faser von Hand; die Fixtures, Labore
und Aufgaben haben keine.

**Zu lang:** Entscheidung: Ein Kabel über seiner Grenze kommt gar nicht hoch (statt mit
Fehlern zu laufen), weil die Norm jenseits ihrer Reichweite nichts zusagt und ein klarer Zustand
besser lehrt. Zu sehen ist es überall: beide LEDs rot, der Port unten (`show interfaces`:
„down, line protocol is down“ und die Zeile „Cable is 208 m long, 1000BASE-T allows 100 m: no
link“; unter Linux `NO-CARRIER` in `ip link`), eine Notiz an beiden Enden („Kabel an Gi1/0/5 zu
lang: 208 m, erlaubt 100 m (1000BASE-T); die Leitung bleibt aus.“), eine Protokollzeile
(IOS-artig `%PHY-4-CABLE_LENGTH: …`, unter Linux die des Kernels), das Problem „Kabel zu lang:
208 m, erlaubt 100 m (1000BASE-T zwischen PC4 eth0 und SW1 Gi1/0/5); die Leitung bleibt aus.“
und in der physischen Ansicht ein gestricheltes Kabel mit Warndreieck und „zu lang, erlaubt 100
m“. Wird es kürzer (ein Gerät woanders, eine Länge von Hand), kommt es hoch wie ein
eingestecktes Kabel. Optiken, die nicht passen, zeigen sich genauso (`%PHY-4-OPTICS`, das
Problem „Optiken passen nicht: …“).

**Laufzeit:** Ein Kupfer- oder Glasfaserkabel, dessen Verzögerung auf dem Vorgabewert seiner
Art steht und dessen Länge aus dem Plan kommt (von Hand gesetzt oder beide Enden in der
physischen Ansicht aufgestellt), läuft mit 5 ns je Meter; jedes andere behält seine
Verzögerung. So bleiben ältere Projekte und alle Golden-Berichte unverändert.

**WLAN** rechnet seine Reichweiten weiter mit den Abständen der logischen Zeichenfläche und dem
Maßstab `dip_per_m` (siehe [WLAN](#wlan)); die Plätze der physischen Ansicht gehen dort noch
nicht ein, weil frei stehende Geräte und solche auf Tischen sonst andere Signale bekämen als in
älteren Projekten.

**Szenarien:** `place GERÄT ORT [HE]` stellt ein Gerät in einen anderen Raum, auf einen Tisch
oder in ein Rack (ohne HE die unterste freie), `cable GERÄT PORT length M` gibt dem Kabel am
Port eine Länge von Hand (`length auto`: wieder automatisch); der Bericht nennt die Kabel mit
ihren neuen Längen. `expect cable GERÄT PORT length [VERGLEICH] M` prüft die Länge, `expect
cable GERÄT PORT up|down`, ob es trägt. Das Golden-Szenario `phys_lengths.scn` auf
`fixtures/phys_campus.jfnet` prüft die Längen des Campus, PC4s Kabel von 208 m (unten, `show
interfaces`, `ip link`), im Büro 1 23 m mit einem Ping, 132 m von Hand mit seiner
Protokollzeile, wieder automatisch und SW2 in einer anderen Höheneinheit.

Noch offen: Kabel mit Fehlern statt ganz aus, Medien-Konverter und Patchfelder, Trassen und
Durchbrüche als eigene Wege, Kabel im Plan ziehen statt auf der logischen Fläche, die
WLAN-Reichweite aus den Plätzen, neue Geräte aus der Palette direkt in den Plan.

Für Programme: `jfn_model::phys` (`Tree`, `length`, `delay_ns`, `reach`, `link_reach`,
`optic_fault`, `issues`, die Änderungen `add_place`, `rename_place`, `remove_place`,
`set_place`, `place_device`, `set_len`, `set_optic`), `catalog::form`, `rack_units`, `Optic`,
`Action::CableLen { link, len_m }`, `Action::Optic { dev, port, optic }`,
`Engine::cable_phys(link)`, `link_idx_up(link)`, `Note::Phys(PhysNote)`,
`ProblemKind::CableTooLong`, `OpticMismatch` und `CableGrade`. Geprüft wird das in der Selbsttest-Gruppe
`phys` ([`src/phystest.rs`](src/phystest.rs)), mit `scen_phys_lengths.uis` und den
Fensterskripten `phys_view.uis`, `phys_rack.uis`, `phys_length.uis` und `phys_shots.uis`.

### Mail (SMTP und POP3)

Mailserver sind Dienste wie die anderen ([`smtpd.rs`](crates/jfn_sim/smtpd.rs),
[`pop3d.rs`](crates/jfn_sim/pop3d.rs), [`mailc.rs`](crates/jfn_sim/mailc.rs),
[`mailbox.rs`](crates/jfn_sim/mailbox.rs)); die Nachrichten liegen in den Dateien der Geräte
und werden mit dem Projekt gespeichert. Die Bytes baut und liest der Codec der Library
([`netpkt::smtp`](../../lib/netpkt/smtp.rs), [`pop3`](../../lib/netpkt/pop3.rs),
[`imf`](../../lib/netpkt/imf.rs)); der Paket-Tab zeigt jede Zeile beider Protokolle mit Befehl,
Code und erweitertem Statuscode, der Simulationsmodus hat für sie die Kategorie „Mail“ in einem
gedämpften Beerenrot (3:1 gegen die Fläche in beiden Themes und im Hochkontrast).

- **SMTP-Server** (Dienst `smtp-server`): Port 25 und der Submission-Port 587 (RFC 6409), EHLO
  mit SIZE, 8BITMIME, PIPELINING und ENHANCEDSTATUSCODES, auf 587 AUTH PLAIN und LOGIN (RFC
  4954) mit den Benutzern des Dienstes; ohne Anmeldung nimmt 587 kein MAIL an (530). Er nimmt
  Mail für seine Domänen (`domains`) an, wenn es den Benutzer gibt (`users`; `postmaster` immer,
  sonst 550 5.1.1), und Mail an andere Domänen nur von Clients, die weiterleiten lassen dürfen:
  aus `networks` (ohne Angabe die Netze seiner Interfaces), angemeldet, oder mit `open_relay`
  von jedem (sonst 554 5.7.1; der Schalter ist zum Lernen da und steht als Problem in der
  Liste). `max_size_kb` begrenzt die Größe (ein zu großes SIZE schon bei MAIL, sonst nach dem
  Punkt: 552 5.3.4). Jede Nachricht bekommt eine Received-Zeile und kommt nach dem Schreiben
  auf die Disk des Geräts in die Warteschlange; 250 nennt ihre Nummer.
- **Zustellen:** in ein Postfach des Geräts nach einem zweiten Schreiben (Maildir:
  `/var/mail/BENUTZER/new/SEKUNDEN.MµsQn.GERÄT` mit `Return-Path` oben und den Zeilenenden von
  Unix; die Verzeichnisse entstehen mit der ersten Nachricht); an andere Domänen über deren
  MX-Einträge, die der Resolver des Geräts findet (ohne MX die Domäne selbst), die Adressen
  jedes Mailservers der Reihe nach mit `connect_timeout_ms` (30 s) und PIPELINING, wenn der
  nächste Server es anbietet. 2xx ist zugestellt; 5xx oder eine Domäne, die das DNS nicht
  kennt, ist endgültig; alles andere (keine Verbindung, 4xx, ein DNS ohne Antwort) wird
  zurückgestellt: der nächste Versuch nach `retry_ms` (60 s), dann jeweils doppelt so spät bis
  `retry_max_ms`, nach `give_up_ms` (10 min) wird aufgegeben (RFC 5321 nennt mindestens 30 min
  und 4 bis 5 Tage; hier verkürzt und einstellbar). Ein MX, der auf den Server selbst zeigt,
  ist eine Schleife. Wer abgewiesen oder aufgegeben wurde, steht in einer
  Unzustellbarkeitsnachricht an den Absender (multipart/report nach RFC 3464 mit Text,
  `Final-Recipient`, `Status`, `Remote-MTA`, `Diagnostic-Code` und dem Kopf der Nachricht) von
  `MAILER-DAEMON` mit leerem Absender; scheitert sie selbst, wird sie verworfen. Die
  Warteschlange lebt im Speicher des Dienstes und geht mit dem Strom verloren.
- **POP3-Server** (Dienst `pop3-server`, Port 110): Benutzer mit Passwort (`users`), USER und
  PASS (ein falsches Passwort nach `fail_delay_ms`, nach `max_fails` ist Schluss), STAT, LIST,
  UIDL (der eindeutige Name der Datei), RETR und TOP nach der Lesezeit der Disk, DELE, RSET,
  NOOP, CAPA und QUIT, das erst dann die markierten Nachrichten löscht (ein Rückgängig-Schritt
  des Projekts); ein Postfach hält nur eine Sitzung (`[IN-USE]`).
- **IMAP-Server** ([`imapd.rs`](crates/jfn_sim/imapd.rs), Dienst `imap-server`, Port 143; der Codec:
  [`netpkt::imap`](../../lib/netpkt/imap.rs)): dieselben Maildirs wie POP3, mit Ordnern (`.Sent`,
  `.Trash`, weitere als `.Name`), Flags im Dateinamen (`:2,FRST`, ein Flag schiebt die Nachricht von
  `new/` nach `cur/`) und UIDs in der Datei `.uidlist` des Ordners. CAPABILITY, LOGIN, AUTHENTICATE
  PLAIN, LOGOUT, NOOP; LIST, LSUB, SELECT, EXAMINE, STATUS, CREATE, DELETE, RENAME; FETCH (FLAGS,
  UID, RFC822.SIZE, INTERNALDATE, ENVELOPE, BODY[], BODY[HEADER], BODY[TEXT],
  BODY[HEADER.FIELDS (…)], .PEEK, RFC822, ALL, FAST, FULL), STORE, EXPUNGE, CLOSE, UNSELECT, COPY,
  MOVE, APPEND, SEARCH (ALL, UNSEEN, FROM, SUBJECT, TEXT, NOT, OR und mehr), die UID-Formen und IDLE
  (eine neue Nachricht meldet sich nach höchstens 2 s). Mehrere Sitzungen und POP3 dürfen dasselbe
  Postfach halten; was eine andere entfernt, kommt beim nächsten NOOP als EXPUNGE. Die erste
  Anmeldung legt Sent und Trash an. Die Begrüßung nennt unseren eigenen Server.

  ```json
  {"kind": "imap-server", "name": "imap", "config": {"users": [{"name": "anna", "password": "geheim"}]}}
  ```
- **E-Mail im Desktop** ([`devwin/email.rs`](src/workspace/devwin/email.rs)): das Konto (Name,
  Adresse, SMTP-Server und Port, POP3-Server und Port, Benutzer, Passwort, Nachrichten auf dem
  Server lassen) liegt als `/root/Mail/account` auf dem Gerät; Posteingang (`/root/Mail/inbox`)
  und Gesendet (`/root/Mail/sent`) mit Absender, Betreff und Datum, die gewählte Nachricht
  darunter; Verfassen (An, Cc, Betreff, Text), Antworten („Re: “, an den Absender, der Text
  zitiert), Löschen, Senden und Abrufen. Senden und Abrufen gehen als `Action::MailSend` /
  `MailFetch` an den Simulator (so spielt das Zurückspulen sie nach), der ein Programm des
  Geräts damit beauftragt; abgerufen wird, was die UIDL noch nicht kannte (`/root/Mail/.uidl`).
  Zwei Zeilen sagen, was das letzte Senden und Abrufen ergab. Mit dem Kontotyp IMAP bleibt die Mail
  auf dem Server: die Ordner des Servers stehen zur Wahl, die Spalte Status zeigt seine Flags (Neu,
  ★), Abrufen gleicht den gezeigten Ordner ab (`/root/Mail/imap/ORDNER/UID.eml` ist nur die Kopie
  fürs Fenster), Lesen setzt \Seen, Markieren \Flagged, Löschen verschiebt nach Trash (dort endgültig),
  Gesendetes landet per APPEND in Sent, Ordner werden angelegt und gelöscht; je eine Sitzung über
  `Action::MailImap`.
- **Shell:** `curl smtp://SERVER[:PORT][/HELO] --mail-from A --mail-rcpt B -T DATEI [-u
  BENUTZER:PASSWORT] [-v]` und `curl pop3://SERVER[/N] -u BENUTZER:PASSWORT [-X BEFEHL] [-o
  DATEI] [-v]` und `curl imap://SERVER[/ORDNER[;UID=N][?SUCHE]] -u BENUTZER:PASSWORT [-X BEFEHL] [-o
  DATEI] [-v]` mit den Schritten, Meldungen und Exit-Codes von curl; `mail` einmalig: `echo
  TEXT | mail -s BETREFF [-c ADRESSE] [-r ABSENDER] EMPFÄNGER` übergibt eine Nachricht an den
  SMTP-Server des Kontos (oder an einen auf dem Gerät selbst), `mail -H` listet den
  Posteingang. Ein Mailprogramm in der Konsole (lesen, antworten) gibt es nicht, das macht
  E-Mail im Desktop. Server und Hosts mit fester Adresse bekommen ihren Namensserver mit `echo
  "nameserver ADRESSE" > /etc/resolv.conf` (`>>` für weitere Zeilen, auch `search`); so
  schreibt ihn auch ihre Konfiguration.
- **Notizen und Probleme:** Sitzungen, Anmeldungen, Absender und Empfänger, abgelehnte Befehle
  und Relays, die Warteschlange, Zustellungen, MX-Einträge, Weitergaben, Zurückstellungen mit
  dem nächsten Versuch und Unzustellbares als Notiz (`Note::Mail`) und Log-Zeile (`smtpd`,
  `pop3d`, `e-mail`); die Probleme-Liste nennt einen MX-Eintrag ohne Adresse (oder ohne
  SMTP-Server dort), einen offenen Relay und einen POP3-Benutzer, dessen Mail nie ankommen kann.
- **Zahlen:** der SMTP-Server zählt `sessions`, `active`, `logins`, `failed_logins`, `received`,
  `delivered`, `relayed`, `deferred`, `bounced`, `relay_denied`, `rejected`, `too_big`, `queued`
  und `bytes_received`, der POP3-Server `sessions`, `active`, `logins`, `failed_logins`,
  `retrieved`, `deleted` und `bytes_sent` (für `expect … svc NAME metric …` und den Bereich
  Metriken, der für beide eigene Diagramme hat).

Ein Mailserver in der Projektdatei:

```json
{"kind": "smtp-server", "name": "smtp", "config": {"hostname": "mail.a.lab", "domains": ["a.lab"],
 "users": [{"name": "anna", "password": "geheim"}], "networks": ["10.0.1.0/24"]}},
{"kind": "pop3-server", "name": "pop3", "config": {"users": [{"name": "anna", "password": "geheim"}]}}
```

Entscheidungen: Maildir statt mbox, weil jede Nachricht eine Datei im Reiter Dateien ist, UIDL
ihr Name und DELE das Entfernen einer Datei. Die Antworten der Server, ihre Begrüßung, die
Unzustellbarkeitsnachricht und die Log-Zeilen sind eigene Worte auf Englisch wie bei den
anderen Servern; Codes und Formate folgen den RFCs. `mail` ist einmalig gehalten. Kein STARTTLS.
Bei IMAP gilt eine Nachricht als \Recent, solange sie in `new/` liegt: das Postfach bleibt, wie der
Server es vorfand, bis jemand etwas ändert. Keine BODYSTRUCTURE und keine Teilabrufe, Schlüsselwörter
werden nicht gespeichert.

Noch offen: STARTTLS und SMTPS, Weiterleitungen und Aliase außer postmaster, eine
Warteschlange auf der Disk, die einen Neustart übersteht, Anhänge im Programm E-Mail, SPF und
DKIM.

Für Programme: `Action::MailSend { dev, draft: MailDraft }`, `Action::MailFetch { dev }`,
`Action::MailImap { dev, op: ImapOp }`, `mailc::{imap_dir, imap_folders, imap_marks, uid_of_path}`,
`Engine::mail_client(dev)`, `jfn_sim::mailc` (`Account`, `folder`, `read_mail`, `INBOX`,
`SENT`, `ACCOUNT`), `Note::Mail(MNote)`, `ProblemKind::MailMxNowhere`, `MailOpenRelay` und
`MailNoMailbox`. Geprüft wird in der Selbsttest-Gruppe `mail` ([`src/mailtest.rs`](src/mailtest.rs)),
in der coretest-Gruppe `mail` (die Codecs Byte für Byte), mit `scen_mail_domains.uis` und den
Fensterskripten `mail_services.uis`, `mail_client.uis` und `mail_shots.uis`; IMAP in der
Selbsttest-Gruppe `imap` ([`src/imaptest.rs`](src/imaptest.rs)), der coretest-Gruppe `eap`, mit
`scen_dot1x_imap.uis` und `mail_imap.uis`.

### Gruppen und Hintergrundbild

Auf der logischen Zeichenfläche lassen sich Geräte, Notizen und Gruppen zu **Gruppen**
zusammenfassen, und unter allem kann ein **Hintergrundbild** liegen, etwa ein Grundriss
([`jfn_model/group.rs`](crates/jfn_model/group.rs),
[`background.rs`](crates/jfn_model/background.rs),
[`src/workspace/canvas/groups.rs`](src/workspace/canvas/groups.rs),
[`backdrop.rs`](src/workspace/canvas/backdrop.rs), [`src/doc/groups.rs`](src/doc/groups.rs),
[`inspector/group.rs`](src/workspace/inspector/group.rs)). Beides sind Ansichtsdaten: Der
Simulator sieht sie nie, Keyframes und alle Golden-Berichte bleiben, wie sie sind.

- **Gruppieren** (Bearbeiten › Gruppieren, Strg+G; im Inspektor bei mehreren gewählten
  Dingen): Die gewählten Geräte, Notizen und Gruppen kommen in eine neue Gruppe („Gruppe 1“, die
  Farbrollen reihum), und zwar die äußersten (ein Mitglied einer mitgewählten Gruppe bleibt
  darin) in die innerste Gruppe, die sie alle enthält, an die Stelle des ersten. Gruppen dürfen
  verschachtelt sein; jedes Ding gehört höchstens einer Gruppe direkt an, Zyklen gibt es nicht,
  eine Gruppe, die dabei leer wird, verschwindet. **Gruppierung aufheben** (Strg+Umschalt+G)
  löst die gewählten Gruppen auf, ihre Mitglieder nehmen ihren Platz ein und sind gewählt; Entf
  auf einer Gruppe tut dasselbe (Geräte löscht Entf auf den Geräten).
- **Aufgeklappt** ist eine Gruppe ein beschrifteter Rahmen um ihre Mitglieder, mit Platz für
  ihre Namen und Dienst-Pillen: oben eine Titelleiste mit Name, Anzahl und dem Knopf „−“ zum
  Zuklappen, in einer gedämpften Farbrolle (Grau, Blau, Grün, Ocker, Ziegelrot, Violett; mit
  3:1 gegen die Fläche in beiden Themes, nie die Farbe allein: Name und Rahmen tragen sie),
  getönt, sodass Raster und Hintergrundbild durchscheinen. Getroffen wird der Rahmen nur an
  Titelleiste und Rand: Ein Klick ins Innere ist Hintergrund (das Gummiband beginnt dort),
  Mitglieder klickt man durch ihn hindurch.
- **Zugeklappt** ist sie ein Cluster-Symbol in der Mitte ihrer Mitglieder: Karten hintereinander
  mit dem Symbol der häufigsten Geräteart, darunter ihr Name, in der Ecke die Zahl ihrer Geräte,
  links oben die Zahl der Befunde ihrer Mitglieder aus Probleme („! 2“, bernstein, rot bei
  Fehlern), entlang der Oberkante die Dienst-Pillen ihrer Geräte. Kabel zwischen drinnen und
  draußen enden am Cluster: eine Linie je Gegenstelle und Kabelart, bei mehreren mit ihrer Zahl
  („2×“) und den LEDs des schlechtesten; ein einzelnes Kabel bleibt es selbst (Inspektor,
  Wegpunkte). Kabel innerhalb sind ausgeblendet. Ein Doppelklick oder Eingabe klappt die Gruppe
  auf, an Ort und Stelle (entschieden gegen eine eigene Ansicht mit Brotkrumen: Die Kabel
  bleiben, wo sie sind, und es gibt nichts, wohin man zurück müsste).
- **Bearbeiten:** Ziehen am Titel oder am Cluster verschiebt alle Mitglieder um ganze
  Rasterschritte, sie bleiben auf dem Raster; F2 oder ein Doppelklick auf den Titel benennt um
  (1 bis 64 Zeichen, eindeutig). Der Inspektor zeigt Name, Zustand, wo sie liegt, ihren Inhalt,
  die Farbe, Zuklappen / Aufklappen, Gruppierung aufheben und die Mitglieder (ein Klick wählt
  eines und zeigt es). Jede Änderung ist ein Rückgängig-Schritt, auch das Zu- und Aufklappen.
- **Versteckte Mitglieder:** Gerätefenster, Konsole und ein Doppelklick in Probleme oder im
  Protokoll klappen die Gruppen um ein verstecktes Gerät zuerst auf (ein Schritt); ein einfacher
  Klick auf einen Befund wählt den Cluster. Eine offene Konsole arbeitet weiter, während ihr
  Gerät versteckt ist. Im Simulationsmodus laufen Pakete entlang der Linie in einen zugeklappten
  Cluster hinein; was darin läuft, zeigt die Fläche nicht. „Nur Auswahl“ zählt die Geräte einer
  gewählten Gruppe mit.
- **Tastatur und Screenreader:** Rahmen und Cluster sind Einträge („Gruppe „Serverraum“,
  5 Geräte“, „…, zugeklappt“, eine Linie „2 Kabel Serverraum – SW1, Kupfer, gerade, aktiv“),
  Tab läuft über sie, Strg+A, Gummiband, Minimap (Rahmen als Umriss) und Alles zeigen schließen
  sie ein.

**Hintergrundbild** (Ansicht › Hintergrundbild …, oder im Inspektor des Projekts): PNG, JPEG,
BMP oder GIF (die Formate, die jedes Fenster dekodiert; WebP bräuchte einen eigenen Codec) bis
8 MiB, 8192 Pixel je Seite und 32 Megapixel; größere Bilder werden mit ihrer Größe abgelehnt,
statt sie ungefragt zu verkleinern. Das Bild kommt in die Mitte der Ansicht, auf neun Zehntel von
ihr eingepasst, mit 60 % Deckkraft, ungesperrt und gewählt. Ungesperrt verschiebt Ziehen es wie
eine Notiz, sein Griff unten rechts skaliert es (das Seitenverhältnis bleibt); der Inspektor
stellt Links, Oben (in Einheiten der Fläche), Maßstab und Deckkraft ein und hat „An Ansicht
anpassen“, „Ersetzen …“ und „Entfernen“. **Sperren** (Ansicht › Hintergrundbild sperren) macht es
unwählbar: Ein Klick darauf ist Hintergrund, das Gummiband geht darüber. Gezeichnet wird es unter
dem Raster in die statische Fläche der Zeichenfläche: eine Kopie von höchstens 4096 Pixeln je
Seite mit Mip-Stufen, für jeden Zoom einmal bilinear skaliert und behalten (während einer Geste
ungefiltert aus der passenden Stufe), im dunklen Theme mit 60 % seiner Deckkraft, damit ein
weißer Plan nicht blendet.

**In der Datei:** `groups` hält je Gruppe ihre Nummer (vom Zähler der Geräte, Kabel und Notizen),
Namen, Farbrolle, Mitglieder (Geräte, Notizen, Gruppen) und ob sie zugeklappt ist; ein Mitglied,
das in keiner Gruppe steht, liegt oben. `background` hält die Datei des Bilds unverändert (Format
und Name, die Bytes als Base64), so bleibt eine `.jfnet` eine Datei, die automatische Sicherung,
Wiederherstellung und Rückgängig ganz tragen (Rückgängig-Schritte teilen die Bytes).

```json
"groups": [
  { "id": 12, "name": "Serverraum", "color": "blue", "members": [3, 4, 13], "collapsed": true },
  { "id": 13, "name": "Rack 2", "color": "green", "members": [6, 7], "collapsed": false }
],
"background": { "format": "png", "name": "plan.png", "w": 1200, "h": 800, "x": -600, "y": -400,
  "scale_pm": 1000, "opacity": 60, "locked": true, "base64": "iVBORw0KGgo…" }
```

Beim Lesen fallen Mitglieder weg, die es nicht gibt, ein Mitglied zweier Gruppen gehört der
ersten, Zyklen werden gebrochen, Gruppen ohne Gerät oder Notiz fallen weg (die Prüfung des
Projekts meldet solche Dateien). Im Fenster bleibt die Nummer eines gelöschten Geräts in seiner
Gruppe stehen, damit Rückgängig es dorthin zurückbringt.

Noch offen: ein Hintergrundbild je Ort der physischen Ansicht, Gruppen in der physischen Ansicht,
eine Suche auf der Zeichenfläche (es gibt keine; Tab, die Einträge und der Inspektor der Gruppen
finden ihre Mitglieder).

Für Programme: `jfn_model::group` (`Group`, `GroupColor`, `Groups::of` mit `parent`, `chain`,
`members`, `leaves`, `hidden_by`, `rep`; `group`, `ungroup`, `rename`, `set_collapsed`,
`set_color`, `normalize`, `issues`), `jfn_model::background` (`Background`, `sniff`, `BgErr`, die
Grenzen) und im Diagramm-Widget der Library die Knoten-Flags `DN_FRAME`, `DN_TOGGLE`, `DN_STEP`,
`DN_RESIZE`, `DN_OUTLINE` mit `DEv::Toggle`, `DEv::Resize`, `dg_toggle_rect` und
`DgPaint::paint_under` (docs/GUIDE.md). Geprüft wird das in der Selbsttest-Gruppe `groups`
([`src/groupstest.rs`](src/groupstest.rs)), mit den Fensterskripten `groups.uis`,
`groups_bg.uis`, `groups_sim.uis`, `groups_shots.uis` und `apps/diagramtest/tests/frames.uis`.

### SNMP

Router, Switches, die Firewall und die anderen IOS-artigen Geräte bekommen mit `snmp-server`
einen SNMP-Agenten, Linux-Hosts mit dem Dienst `snmpd`; beide sprechen SNMPv1 (RFC 1157) und
SNMPv2c (RFC 1901, RFC 3416) über UDP-Port 161 ([`snmpd.rs`](crates/jfn_sim/snmpd.rs),
[`snmpmib.rs`](crates/jfn_sim/snmpmib.rs)). Ohne Konfiguration gibt es keinen Agenten; ältere
Projekte laufen wie bisher.

- **IOS-artige Befehle** ([`cli/snmp.rs`](crates/jfn_sim/cli/snmp.rs)): `snmp-server community
  NAME [ro|rw [ACL]]` (eine Standardliste mit Nummer 1–99 oder 1300–1999 oder mit Namen),
  `snmp-server location|contact TEXT`, `snmp-server host ADRESSE [traps|informs] [version 1|2c]
  COMMUNITY`, `snmp-server enable traps [snmp [authentication] [linkdown] [linkup]
  [coldstart]]` (ohne Wörter alle vier), die `no`-Formen und `no snmp-server`; alles steht in
  der Running-Config. `show snmp` zeigt Zähler, Kontakt, Standort und Empfänger, `show snmp
  community` die Communities mit ihrem Zugriff, `debug snmp packets` jede Anfrage, Antwort und
  Meldung des Agenten als Konsolenzeile; diese Texte sind eigene Worte.
- **Linux-Hosts:** der Dienst `snmpd` mit Einstellungen als JSON (`communities` mit `name`,
  `access` `ro` oder `rw` und `source`, dem Netz, für das sie gilt; `location`, `contact`,
  `sinks` mit `host`, `type` `trap` oder `inform`, `version`, `community` und `port`; `notify`
  aus `coldstart`, `linkdown`, `linkup` und `authentication`, ohne nur coldStart; `port`,
  `inform_timeout_ms`, `inform_retries`).
- **Die MIB** rechnet der Agent bei jeder Anfrage aus dem Zustand des Geräts: die Gruppe system
  (sysDescr in eigenen Worten wie `show version`, sysObjectID unter der Enterprise-Nummer 32473
  für Dokumentation aus RFC 5612 mit einer Nummer je Modell, sysUpTime seit dem Start des
  Agenten, Kontakt, Name, Standort, sysServices), `ifNumber`, `ifTable` und `ifXTable` mit
  `ifName`, `ifAlias` aus der Beschreibung, `ifHighSpeed` und den 64-Bit-Zählern der Ports
  (Unicast, Multicast und Broadcast getrennt), `ipForwarding`, `ipDefaultTTL`, `ipAddrTable`,
  `ipCidrRouteTable` (die Routen, die das Gerät gerade benutzt) und die Zähler der Gruppe snmp.
  Die Interfaces zählen auf IOS-artigen Geräten die Ports der Reihe nach (ohne Konsole und USB),
  danach Subinterfaces, Loopbacks und VLAN-Interfaces; unter Linux wie `ip link`. GetNext und
  GetBulk folgen der Reihenfolge der OIDs über alle Tabellen hinweg.
- **Anfragen** nach RFC 3416 und RFC 3584: Get, GetNext, GetBulk (Non-Repeaters,
  Max-Repetitions, früher Schluss am Ende der Sicht, höchstens 1472 Byte) und Set; in Version 2c
  noSuchObject, noSuchInstance und endOfMibView in der Bindung, in Version 1 noSuchName mit
  Index (GetNext überspringt dort Counter64), tooBig in beiden. Set prüft erst alle Bindungen
  (notWritable, wrongType, wrongLength, wrongValue, noCreation; in Version 1 noSuchName und
  badValue) und ändert alle oder keine. Schreiben lassen sich sysContact, sysLocation, sysName
  (benennt das Gerät um wie `hostname`), `ifAdminStatus` (wie `shutdown` und `no shutdown`),
  `ifAlias` und `snmpEnableAuthenTraps`, auf IOS-artigen Geräten in die Running-Config, ins
  Projekt erst mit `copy run start`.
- **Ablehnen:** eine unbekannte Community oder ein Absender, den die Zugriffsliste
  (IOS-artig) oder `source` (Linux) nicht zulässt, bekommt keine Antwort; das zählt als
  snmpInBadCommunityNames, schreibt `%SNMP-3-AUTHFAIL` auf die Konsole (unter Linux eine
  Log-Zeile von `snmpd`) und meldet authenticationFailure, wenn das eingeschaltet ist. Set mit
  einer Community nur zum Lesen bekommt noAccess (Version 1: noSuchName).
- **Meldungen:** coldStart beim Start des Agenten, linkDown und linkUp mit ifIndex,
  ifAdminStatus und ifOperStatus, wenn ein Port seinen Link verliert oder bekommt, und
  authenticationFailure; als Trap der Version 1 (Enterprise sysObjectID, Agent-Adresse,
  generic und specific trap) oder als SNMPv2-Trap oder Inform der Version 2c, von einem eigenen
  Port. Ein Inform geht wieder, bis seine Antwort kommt (IOS-artig nach 15 s bis zu dreimal,
  Linux nach 1 s bis zu fünfmal). Das IOS-artige ARP verliert eine Meldung, deren nächster Hop
  noch keinen Eintrag hat, wie den ersten Ping; ein Inform kommt deshalb später an.
- **Manager in der Shell:** `snmpget`, `snmpgetnext`, `snmpwalk`, `snmpbulkwalk` und `snmpset`
  mit `-v 1|2c` (ohne: 2c), `-c COMMUNITY`, `-r` (5), `-t` (1 s), `-On`, `-Oq`, `-Ov`, `-Os`,
  `-Cr N` und `-Cn N`; Namen wie `sysName.0`, `IF-MIB::ifDescr` oder Ziffern
  (`.1.3.6.1.2.1.1.5.0`), bei `snmpset` die Typen `i u t a o s x d n` und `=` für den der MIB.
  Die Ausgaben haben die Form von net-snmp (`SNMPv2-MIB::sysName.0 = STRING: R1`), Hilfe und
  Fehler sind eigene Worte. `snmpwalk` fragt ohne Wurzel mib-2 ab.
- **Trap-Empfänger:** der Dienst `snmptrapd` ([`snmptrapd.rs`](crates/jfn_sim/snmptrapd.rs))
  auf UDP-Port 162 nimmt Traps beider Versionen und Informs (die er bestätigt), filtert nach
  `communities` (leer: alle) und schreibt jede Meldung als Zeile ins Protokoll (Quelle
  `snmptrapd`), in seine Logdatei (`log`, ohne: `/var/log/snmptrapd.log`, in der Form von
  net-snmp; als Datei des Geräts ein Rückgängig-Schritt wie jede geschriebene Datei) und in seine
  Statuszeile.
- **Im Fenster:** beide Dienste stehen im Reiter Dienste mit Statuszeile, Zahlen und den
  Einstellungen als JSON. Der Desktop eines Hosts hat einen MIB-Browser
  ([`devwin/mib.rs`](src/workspace/devwin/mib.rs)): der Baum der eingebauten MIB, zu einem Objekt
  Name, OID, Syntax und Zugriff; Get, GetNext, Walk und Set laufen als Befehle in der Konsole
  des Geräts darunter. Der Paket-Tab zerlegt SNMP Feld für Feld mit den Namen der Objekte, die
  Ereignisse haben einen eigenen Chip „SNMP“ in gedecktem Oliv.
- **Notizen und Probleme:** jede Anfrage, Antwort, Ablehnung, Änderung und Meldung als Notiz
  (`Note::Snmp`); die Probleme-Liste warnt vor einem Empfänger, zu dem es keine Route gibt, und
  vor einer Community `rw` ohne Zugriffsliste oder `source`.

Entscheidungen: Der Agent hält keine Kopie der MIB; jede Anfrage sieht das Gerät, wie es gerade
ist. Die Routen stehen in `ipCidrRouteTable` (RFC 2096), nicht in der neueren
`inetCidrRouteTable`. Was ein Link beim Start des Geräts tut, meldet der Agent nicht (der
coldStart sagt es schon). Ein Absender, den die Zugriffsliste abweist, zählt wie eine unbekannte
Community. Unter Linux gilt ein Set, solange der Dienst läuft; die Einstellungen ändert es nicht.
sysObjectID nennt nie die Nummer eines Herstellers.

Noch offen: Traps der Version 3, `snmp-server inform retries|timeout`, `show snmp host`, Meldungen über
IPv6, Tabellen über die Ressourcen der Hosts, ein Agent für den MIB-Browser ohne die Konsole.

Für Programme: `Action::Snmp { dev, set }` mit `SnmpSet`, `Engine::snmp_agent(dev)`,
`snmp_running(dev)`, `snmp_view(dev)`, `snmp_traps(dev)`, `SnmpAgent`, `SnmpCfg`, `Community`,
`TrapHost`, `TrapReceiver`, `TrapRow`, `SnmpView`, `IfRow`, `Note::Snmp(SNote)`,
`ProblemKind::SnmpTrapHost` und `SnmpOpenRw`; in der Library `netpkt::ber`, `netpkt::snmp` und
`netpkt::mib`. Geprüft wird in der Selbsttest-Gruppe `snmp`
([`src/snmptest.rs`](src/snmptest.rs)), BER und SNMP Byte für Byte in der coretest-Gruppe
`snmp`, dazu das Szenario `snmp_monitor.scn` und die Fensterskripte `snmp_services.uis`,
`snmp_console.uis`, `snmp_mib.uis` und `snmp_shots.uis`.

**SNMPv3** (RFC 3411–3415, 3826, 7860; [`lib/netpkt/snmpv3.rs`](../../lib/netpkt/snmpv3.rs)): dieselben
Agenten sprechen auch Version 3 mit dem User-based Security Model und VACM; ohne Benutzer antworten sie
nur auf die Discovery.

- **Sicherheitsstufen:** noAuthNoPriv, authNoPriv (HMAC-MD5-96, HMAC-SHA-96, die SHA-2-Varianten
  SHA-256, SHA-384 und SHA-512 von RFC 7860), authPriv (dazu AES-128-CFB). DES gibt es nicht, weil
  `lib/crypto` keines hat. Die Schlüssel werden aus dem Passwort abgeleitet und für die Engine
  lokalisiert (RFC 3414 A.2).
- **IOS-artig:** `snmp-server engineID local HEX`, `snmp-server view NAME OID included|excluded`,
  `snmp-server group NAME v3 noauth|auth|priv [read SICHT] [write SICHT]` (ohne `read`: alles unter
  1.3.6.1, ohne `write`: nichts), `snmp-server user NAME GRUPPE v3 [auth md5|sha|sha256|sha384|sha512
  PASSWORT [priv aes 128 PASSWORT]]` (Passwörter mit mindestens 8 Zeichen), die `no`-Formen, `show snmp
  user`, `show snmp group`, `show snmp view`, `show snmp engineID`. In der Running-Config steht ein
  Benutzer mit `encrypted` und den lokalisierten Schlüsseln, nie mit dem Passwort; eine gespeicherte
  Konfiguration bootet so. Ändert sich die Engine-ID, passen die Schlüssel nicht mehr, wie beim
  Vorbild: der Benutzer muss neu eingegeben werden.
- **Linux:** in den Einstellungen von `snmpd` `"engine_id": "80007ed9…"` und `"users": [{"name": "admin",
  "level": "authPriv", "auth": "SHA", "auth_password": "…", "priv": "AES", "priv_password": "…", "access":
  "rw", "subtree": "1.3.6.1.2.1.1"}]`.
- **Werkzeuge:** `snmpget`, `snmpgetnext`, `snmpwalk`, `snmpbulkwalk` und `snmpset` nehmen `-v 3 -u BENUTZER
  [-l noAuthNoPriv|authNoPriv|authPriv] [-a MD5|SHA|SHA-256|SHA-384|SHA-512] [-A PASSWORT] [-x AES] [-X
  PASSWORT] [-e ENGINE] [-Z BOOTS,ZEIT] [-n KONTEXT]`. Ohne `-e` kommt erst die Discovery; ein signierter
  Report über die Zeit setzt ihre Uhr, die Anfrage geht noch einmal; jeder andere Report endet mit einem
  eigenen Satz.
- **Reports:** ein Agent prüft in der Reihenfolge des RFC und antwortet mit einem Report-PDU:
  usmStatsUnknownEngineIDs (das ist die Discovery), usmStatsUnknownUserNames, usmStatsUnsupportedSecLevels,
  usmStatsWrongDigests (dazu `%SNMP-3-AUTHFAIL` und authenticationFailure, wo das an ist),
  usmStatsNotInTimeWindows (Boots oder mehr als 150 s Unterschied, signiert), usmStatsDecryptionErrors,
  snmpUnknownContexts; ohne das Report-Flag bleibt es bei der Zählung. Boots zählt jeden Start des Agenten
  (es bleibt, solange das Projekt offen ist), die Zeit läuft seit diesem Start.
- **Sichten:** außerhalb der Sicht antwortet Get mit noSuchObject, GetNext und GetBulk gehen darüber hinweg,
  Set bekommt noAccess; ein Benutzer ohne passende Gruppe bekommt authorizationError.
- **Im Fenster:** der MIB-Browser hat die Version 3 mit Benutzer, Stufe, Signatur und den Passwörtern
  (verdeckt getippt). Der Paket-Tab zeigt msgID, msgMaxSize, msgFlags, das Modell, Engine-ID, Boots, Zeit,
  Benutzer und die Parameter, und das scopedPDU oder „verschlüsselt“.

Entscheidungen: Die Engine-ID ist ohne Einstellung Format 3 mit der MAC des ersten Interfaces unter der
Enterprise-Nummer für Dokumentation. Eine Nachricht der Version 3, die sich nicht lesen lässt, zählt wie
eine Version, die der Agent nicht spricht. Traps und Informs bleiben bei den Versionen 1 und 2c.
Die usmStats-Zähler stehen nur im Report, nicht in der MIB des Agenten (ein Walk über 1.3.6.1.6.3 fände
sie nicht).

Geprüft wird in der Selbsttest-Gruppe `snmpv3` ([`src/snmpv3test.rs`](src/snmpv3test.rs)), Byte für Byte
(RFC 3414 A.3, RFC 3826) in der coretest-Gruppe `sshcore`.

### Desktop der Hosts

Das Gerätefenster eines Hosts öffnet auf seinem **Desktop**
([`src/workspace/devwin/desktop.rs`](src/workspace/devwin/desktop.rs), Meilenstein V2): ein
kleiner PC mit Programmen in eigenen Fenstern, Symbolen auf einem eigenen Hintergrund und einer
Taskleiste. Die Fenster sind die der Library (`ui.desk_begin`, L7): Titelleiste, Verschieben,
Größe an Rändern und Ecken, Maximieren (auch Doppelklick auf die Titelleiste), Minimieren,
Einrasten an den Rändern und Hälften, Alt+F6 / Alt+Umschalt+F6 zum nächsten / vorigen Fenster,
Alt+F10, Alt+F9 und Alt+Minus für das Menü des Fensters; für Screenreader ist jedes Fenster ein
Dialog mit seinem Titel.

- **Symbole und Start:** die Programme als Symbole in Spalten von links oben; ein Klick wählt
  eines, ein Doppelklick oder Eingabe öffnet es. Die Symbole sind ein Tab-Halt (die Pfeiltasten
  wählen innen) und für Screenreader eine Liste „Desktop“. „Start“ öffnet die Programme als Menü
  in Gruppen: Netzwerk (IP-Konfiguration, Webbrowser, E-Mail, MIB-Browser), System (Terminal,
  Dateimanager, Systemmonitor, Einstellungen), Entwicklung (Texteditor, IDE).
- **Taskleiste:** Start, eine Schaltfläche je offenem Fenster (ein Klick holt es nach vorn, ein
  zweiter minimiert es, ein Rechtsklick öffnet sein Menü), Vollbild und die Uhr des Geräts in der
  Simulation.
- **Vollbild:** F11 schaltet das aktive Gerätefenster in den Vollbildmodus und zurück (ohne
  aktives Gerätefenster das Hauptfenster), ebenso Vollbild in der Taskleiste; jeder Reiter lässt
  sich so zeigen. Ein neues Fenster öffnet rechts der Symbole so groß, wie Platz ist; E-Mail,
  MIB-Browser und IDE starten maximiert.
- **Terminal:** die Konsole des Geräts – derselbe `TermState` wie im Reiter CLI und im
  Terminal-Bereich, der dorthin verweist, solange das Fenster sie zeigt. „Weiteres Terminal“ im
  Startmenü öffnet ein weiteres Terminal-Fenster mit einer eigenen Befehlszeilen-Sitzung
  (siehe [Sitzungen und Telnet-Server](#sitzungen-und-telnet-server): `pts/N` auf Linux-Hosts,
  `tty N` IOS-artig), betitelt „Terminal (2)“; Schließen des Fensters beendet die Sitzung, nach
  dem Ausschalten des Geräts sagt das Fenster, dass sie endete.
- **Dateimanager** ([`filemgr.rs`](src/workspace/devwin/filemgr.rs)): die Ordner als Baum, die
  Einträge des Ordners mit Größe, Art und Zeit; Neuer Ordner, Umbenennen, Löschen, Kopieren,
  Ausschneiden, Einfügen (ein vergebener Name bekommt „(Kopie)“), Öffnen (ein Ordner, eine Datei
  in ihrem Programm, `.jfs`-Skripte in der IDE, alles andere im Texteditor), Eigenschaften,
  Importieren … und Exportieren … über die Dateidialoge des Reiters Dateien – so kommt ein
  `.jfs`-Skript herein und hinaus. In der Liste: Entf, F2, Strg+C / X / V, Eingabe. Jede
  Änderung ist ein Rückgängig-Schritt des Projekts, dem der Simulator sofort folgt.
- **Texteditor** ([`editor.rs`](src/workspace/devwin/editor.rs)): ohne Wahl die Notizen des
  Geräts als `notizen.txt` – wie bisher im Projekt gespeichert, ältere Projekte bleiben, wie sie
  waren; Öffnen … und Speichern unter … wählen Dateien des Geräts über eine kleine Auswahl, in
  der die Notizen ein eigener Eintrag sind. Jede Speicherung ist ein Rückgängig-Schritt, den die
  Shell sofort sieht (`cat`).
- **Systemmonitor** ([`sysmon.rs`](src/workspace/devwin/sysmon.rs)): die Prozesse, wie `ps` sie
  zeigt (PID, Benutzer, TTY, CPU-Zeit, Befehl; `Engine::processes` in
  [`procs.rs`](crates/jfn_sim/procs.rs)) – „Beenden“ tippt `kill PID` ins Terminal, nur bei
  Hintergrund-Jobs der Konsole (Jobs eines weiteren Terminals beendet dessen eigenes `kill`); die Dienste mit Art, Zustand und CPU-Zeit; Leistung: die CPU-Last der
  letzten Minute (`Engine::cpu_secs`, die vier CPUs von `top`) und der Speicher der Skripte (die
  Zeitreihe `heap_kb`) als kleine Diagramme.
- **Webbrowser** ([`browser.rs`](src/workspace/devwin/browser.rs)): Seiten aus dem simulierten
  Netz, siehe [Webbrowser](#webbrowser).
- **IP-Konfiguration** ([`progs.rs`](src/workspace/devwin/progs.rs)),
  **E-Mail** ([`email.rs`](src/workspace/devwin/email.rs)), **MIB-Browser**
  ([`mib.rs`](src/workspace/devwin/mib.rs)) wie zuvor, jetzt in Fenstern; der MIB-Browser zeigt
  die Konsole unter dem Baum, solange das Terminal-Fenster nicht über ihm liegt.
- **Einstellungen:** der Hintergrund (Schiefer, Salbei, Sand, Nachtblau, Punktraster, Netz; im
  Hochkontrast die Fläche des Fensters ohne Muster), die Größe der Symbole und die Form der Uhr.
  Sie sind Einstellungen des Benutzers für alle Desktops (`config://settings.ini`, auch unter
  Werkzeuge › Einstellungen › Desktop), keine Konfiguration der simulierten Geräte.
- **IDE:** startet maximiert, siehe [IDE](#ide); ihr Terminal ist eine eigene Sitzung, die mit
  dem Fenster endet. Das Gerätefenster hat keinen eigenen Reiter für Skripte mehr.
- **Wo der Zustand liegt:** Fenster, offene Programme und was sie zeigen gehören zur Sitzung,
  nicht ins Projekt und nicht ins Rückgängig: je Gerät, solange die App läuft. Ein geschlossenes
  und wieder geöffnetes Gerätefenster zeigt den Desktop wie zuvor; ein neues oder geladenes
  Projekt beginnt ohne.

Geprüft wird der Desktop in der Selbsttest-Gruppe `desk`
([`src/desktoptest.rs`](src/desktoptest.rs)) und mit den Fensterskripten `desk_programs.uis`,
`desk_files.uis`, `desk_monitor.uis`, `desk_rtl.uis` und `desk_shots.uis`; die Skripte von
E-Mail und MIB-Browser öffnen ihre Programme dort.
### Sitzungen und Telnet-Server

Ein Gerät hat jetzt mehrere Befehlszeilen: die Konsole (Sitzung 0, immer da), weitere Terminals
der Oberfläche und Telnet-Sitzungen über das Netz. Jede Sitzung hat ihren eigenen Zustand: Modus
und Kontext (Interfaces, VLANs, Leitungsblock, Pool, Zugriffsliste, RADIUS-Gruppe und -Server),
den laufenden Befehl mit vorab Getipptem, eine offene Frage, Rechtestufe, Login-Gate und Benutzer,
den Dialog von `copy` und die Shell mit Verzeichnis, Variablen, Verlauf und Hintergrund-Jobs.
Pro Gerät bleiben die Konfiguration, die `debug`-Schalter und die Zustände, die die Meldungen
vergleichen; Meldungen (`%LINK`, `%SYS`, Debug-Ausgabe) gehen immer an die Konsole und an jede
Sitzung mit `terminal monitor`.

- **Lokale Sitzungen** ([`crates/jfn_sim/cli/ses.rs`](crates/jfn_sim/cli/ses.rs)):
  `Engine::cli_open(dev)` öffnet eine (Linux `pts/N`, IOS-artig `tty N`, die sich anmeldet, wie
  `line con 0` es verlangt), `cli_close(dev, s)` beendet sie, `cli_sessions(dev)` listet alle.
  Zeilen, `?`, Tab, Strg+C, Prompt, Verzeichnis und Blättern gibt es je Sitzung
  (`cli_ses_line`, `cli_ses_help`, `cli_ses_complete`, `cli_ses_interrupt`, `cli_ses_prompt`,
  `cli_ses_busy`, `cli_ses_secret`, `cli_ses_reads`, `cli_ses_cwd`, `cli_ses_pager`); die API
  ohne `ses` ist die der Konsole wie bisher. Eingaben sind Befehle des Treibers (`Cmd::SesOpen`,
  `SesClose`, `SesLine`, `SesInterrupt`), die das Zurückspulen wiederholt; Ausgaben kommen als
  `Report::SesOut`, `SesPrompt` und `SesClosed`. `exit` beendet eine weitere Sitzung (die
  Konsole bleibt), Ausschalten beendet alle bis auf die Konsole, die frisch beginnt.
- **Terminal-Panel:** „Neue Sitzung“ (das Plus in der Leiste des gezeigten Tabs) öffnet eine
  weitere Sitzung seines Geräts als eigenen Tab „PC1 (2)“; Schließen des Tabs beendet sie. Der
  CLI-Reiter des Gerätefensters bleibt die Konsole. Für Desktop und IDE gibt es
  `console::open_session(n, dev)`, `view_session(n, ui, dev, s)` und `close_session(n, dev, s)`.
  Läuft in einer Konsole ein Programm, das Zeilen liest (`telnet`, `nc`), bleibt die
  Eingabezeile ohne eigenen Prompt; fragt die Gegenseite nach einem Passwort, wird es verdeckt
  getippt.
- **vty-Leitungen** ([`crates/jfn_sim/telnetd.rs`](crates/jfn_sim/telnetd.rs)): jeder Block
  `line vty A B` gibt die Leitungen A bis B (Vorgabe `vty 0 4`: fünf); eine Verbindung auf TCP 23
  nimmt die niedrigste freie Leitung, deren `transport input` Telnet erlaubt (Vorgabe `all`,
  wie die klassischen Übungen es erwarten; `ssh` allein oder `none` lehnen Telnet ab; siehe
  [SSH](#ssh)) und deren
  `access-class LISTE in` die Quelle zulässt; sonst eine kurze Absage und Schluss. Angemeldet
  wird wie an der Konsole: `password` mit `login`, `login local`, `login authentication LISTE`
  über AAA und RADIUS, ohne `login` gar nicht; drei Fehlversuche schließen die Verbindung.
  `exec-timeout M [S]` beendet eine Sitzung nach so langer Ruhe (Vorgabe 10 Minuten, `0 0` nie;
  nicht, solange ein Befehl läuft). Der Server lauscht nur, solange eine Leitung jemanden
  anmelden kann; die Vorgabe `line vty 0 4 / login` ohne Passwort lehnt Telnet sofort ab (IOS
  sagte vorher „Password required, but none set“). `show users` (jede Leitung mit Benutzer,
  Rechten, Leerlauf und Herkunft, die eigene mit `*`), `show line`, `clear line N | vty N |
  console 0 | tty N`, `terminal [no] monitor`; `%SYS-5-CONFIG_I` nennt die Sitzung, von der die
  Konfiguration kam. Der Server bietet `WILL SUPPRESS-GO-AHEAD` an und `WILL ECHO` nur, während
  er ein Passwort erfragt (der Client hört dann mit dem Echo auf), wie Login-Programme es mit
  einem zeilenweisen Client tun; Prompts und Ausgaben laufen als NVT-Text, `Interrupt Process`
  bricht den laufenden Befehl ab.
- **telnetd auf Linux-Hosts** (Dienst `telnetd`, `{"port": 23, "password": "…"}`): Anmeldung als
  `root` mit dem Passwort des Dienstes, dann eine eigene Shell auf `pts/N`. `who`, `w` und `tty`
  zeigen die Sitzungen; `ps` hat die Spalte `TTY` (`tty1` die Konsole, `pts/N`, `?` für init und
  Dienste) und listet die Shells und Jobs jeder Sitzung.

Entscheidungen: Eine weitere Sitzung eines IOS-artigen Geräts aus der Oberfläche ist eine
Leitung `tty N` mit den Anmelderegeln von `line con 0` (wie ein zweites Terminal am
Konsolenanschluss); die Nummern der Sitzungen werden nicht wieder vergeben. Weitere Sitzungen
sprechen immer mit der Gegenwart (beim Zurückspulen antwortet nur die Konsole aus der
Vergangenheit). Nicht nachgebildet: `aux`-Leitungen, die Zeichen-für-Zeichen-Bearbeitung
von Telnet (unser Client schickt ganze Zeilen).

Für Programme: `Engine::cli_open`, `cli_close`, `cli_sessions` (`SesView`, `SesKind`), die
`cli_ses_*`-Formen und `cli_reads`, `Inline::open_session`, `TelnetServer`. Geprüft wird in der
Selbsttest-Gruppe `sessions` ([`src/sessiontest.rs`](src/sessiontest.rs)), dazu das Szenario
`sessions.scn` und die Fensterskripte `sessions_terminal.uis`, `telnet_vty.uis` und
`sessions_shots.uis`.

### Webbrowser

Der Webbrowser ([`src/workspace/devwin/browser.rs`](src/workspace/devwin/browser.rs), B1 bis B4) ist
ein Programm des Desktops: Er zeigt echte kleine Websites aus dem simulierten Netz mit dem
`ui.html_view` der Library, nicht mit einer eigenen HTML/CSS-Engine. Ein echtes Internet gibt es
nicht. Seiten liefern `http-server`-Routen, Dateien des Geräts und Skripte mit `http.serve`, wie
PHP oder Node. Beispiel: [`tests/fixtures/web_site.jfnet`](tests/fixtures/web_site.jfnet) mit einer
statischen Website und einem Laden als Skript.

- **Bedienung:** Adresszeile (Eingabe öffnet, Strg+L springt hinein; ohne Schema gilt `http://`),
  Zurück und Vor (auch Alt+Links, Alt+Rechts), Neu laden (F5; während des Ladens Anhalten, auch
  Esc). Die Statuszeile nennt beim Laden das Ziel, danach Status, Typ, Größe, Dauer und
  Weiterleitungen und unter dem Zeiger das Ziel eines Links; der Seitentitel steht im Fenstertitel
  und in der Taskleiste. Steht die Simulation, sagt die Statuszeile, dass die Antwort mit ihr
  kommt; im Simulationsmodus gehen die Pakete mit den Schritten.
- **Inhalte:** `text/html` im Dokument der Library (mit deren CSS-Teilmenge und Formularen),
  `text/markdown`, `text/plain` und anderer Text als vorformatierter Text, Bilder allein auf einer
  kleinen Seite, alles andere als Hinweis mit Typ und Größe. Der Titel kommt aus `<title>`, sonst
  aus der ersten Überschrift, sonst aus dem Dateinamen.
- **Links und Bilder:** relative Adressen löst [`netpkt::uri`](../../lib/netpkt/uri.rs) nach
  RFC 3986 auf. Ein Link auf ein Fragment springt innerhalb der Seite (das Element mit der `id`
  oder `<a name>`, in Markdown die Überschrift mit diesem Kürzel) und ist ein eigener Schritt im
  Verlauf. Bilder holt der Browser mit eigenen GET-Anfragen (`data:` entschlüsselt er selbst) und
  merkt sie sich je Fenster, bis Neu laden sie verwirft; die Seite liest keine Datei des echten
  Rechners (`MI_APP`).
- **Verlauf:** je Fenster 30 Einträge mit ihrem Dokument: Zurück und Vor zeigen die Seite sofort
  ohne Anfrage, an derselben Stelle und mit den eingegebenen Feldwerten, auch wenn die Simulation
  steht. Neu laden fragt den Server wieder. Das Schließen des Fensters verwirft alles.
- **Formulare (B2):** Senden geht als GET mit der Query oder als POST mit
  `application/x-www-form-urlencoded` an die `action`, aufgelöst gegen die Seite; die Antwort ist
  ein neuer Eintrag. Neu laden einer POST-Antwort fragt erst („Erneut senden“), denn die Anfrage
  kann etwas auslösen (die zweite Bestellung). Ein Skript liest die Felder aus `req.query` und
  `req.form` (das es nur bei einem Formular-Inhalt gibt, daher `req.form ?? {}`, wo auch anderes
  ankommt).
- **Fehlerseiten:** eigene Seiten mit Grund, Adresse und Hinweis: Verbindung abgelehnt,
  Zeitüberschreitung, Name unbekannt, kein Weg, abgebrochene Verbindung, zu viele Weiterleitungen,
  4xx und 5xx ohne eigene Seite des Servers (eine HTML-Seite des Servers zeigt der Browser, wie
  sie ist), ein Port ohne TLS; für ein Zertifikat mit Fehler die Warnseite (unten).

**Im Simulator** ([`crates/jfn_sim/webc.rs`](crates/jfn_sim/webc.rs)): Das Programm ist ein
versteckter Dienst „webbrowser“ des Hosts über demselben HTTP-Client wie `curl`, also mit DNS,
TCP, Trace und Simulationsmodus. `Cmd::Apply(Action::WebFetch { dev, req })` schickt eine Anfrage
(`WebReq`: id, Methode, URL, Kopfzeilen, Inhalt), `Action::WebCancel { dev, id }` gibt sie auf.
Die Antwort kommt als `Report::Web { dev, id, at, result }`: `WebResp` (endgültige URL, Status,
Kopfzeilen, Inhalt, Weiterleitungen, Dauer) oder `WebFail` (`WebErr`, URL, Grund). Weil die
Anfragen Aktionen sind, spielt das Zurückspulen sie mit. Weiterleitungen (301, 302, 303, 307, 308,
höchstens 10) folgt der Dienst selbst: 301 und 302 machen aus POST ein GET (wie Browser), 303 aus
allem außer GET und HEAD, 307 und 308 behalten Methode und Inhalt; jede ist eine Notiz im Trace.
Eine Anfrage hat 30 s virtuelle Zeit je Sprung. Im Reiter Dienste erscheint der Dienst nicht.

**Dateien als Website:** `http-server` mit `"root": "/var/www"` beantwortet GET und HEAD, die keine
Route nimmt, aus diesem Ordner des Geräts: `index.html` im Ordner, 301 auf den Ordner mit `/`, der
Typ aus der Endung (`netpkt::http::mime_of`), andere Methoden 405. Der Pfad bleibt im Ordner (auch
`%2e%2e`), das Lesen kostet die Zeit der Platte, eine gespeicherte Seite gilt sofort. Ohne `root`
und ohne Routen antwortet der Server wie bisher mit „Hallo von {host}“.

Entscheidungen: Zurück und Vor zeigen die gemerkte Seite statt neu zu fragen (so geht es auch bei
stehender Simulation, und das Netz sieht nur, was ein Browser mit Cache auch schickte). Bilder
kommen über `MI_APP` statt `MI_ASK`, damit relative Pfade einer Seite nicht als Dateien des echten
Rechners gelesen werden. Nicht nachgebildet: Datei-Felder und Mehrfachauswahl in Formularen.

**Seitenskripte (B3, [`src/workspace/devwin/pagescript.rs`](src/workspace/devwin/pagescript.rs)):**
Eine Seite aus dem simulierten Netz führt `<script type="text/jfs">` aus, im Dokument oder mit `src`,
in der Sprache von `lib/script`; JavaScript gibt es nicht, nur dieser `type` läuft. Die Skripte
laufen in der Reihenfolge des Dokuments und teilen sich ihre obersten Namen; ein `src`-Skript wird
zuerst geholt, die nächsten warten darauf, ein Fehler beim Holen steht in der Konsole und das
nächste läuft.

| Name | Was es tut |
|---|---|
| `document` | `by_id(id)`, `find(sel)`, `find_all(sel)` (Selektoren wie in CSS: Tag, `#id`, `.klasse`, `[attr=v]`, Nachfahre, Kind), `create(tag)`, `create_text(s)`, `body()`, `title()`, `set_title(s)`, `on(typ, f)`, `off(typ, f)` |
| ein Element | `find`, `find_all`, `matches(sel)`, `parent()`, `children()`, `next()`, `prev()`, `text()`, `set_text(x)`, `html()`, `set_html(s)`, `attr(n)`, `set_attr(n, x)`, `remove_attr(n)`, `has_class(k)`, `add_class(k)`, `remove_class(k)`, `toggle_class(k)`, `append(n)`, `insert(n, vor)`, `remove()`, bei Feldern `value()`, `set_value(x)`, `checked()`, `set_checked(b)`, dazu `on`, `off`, `same(anderes)` und die Felder `tag`, `kind` |
| Ereignisse | `click` (ein Klick auf Text, Link, Knopf, Kästchen; Eingabe auf einem Link), `input` (Text, Liste, Kästchen), `submit` (ein Formular, `ev.data` mit seinen Feldern); `ev.target`, `ev.current`, `ev.type`; `ev.prevent()` hält einen Link oder ein Formular auf, `ev.stop()` den Weg nach oben |
| `timer` | `after(ms, f)`, `every(ms, f)` geben eine Nummer, `cancel(nummer)` |
| `fetch` | `fetch(url oder {url, method, headers, body}, fn(err, resp))`: `resp` hat `status`, `reason`, `headers`, `body`, `url`, `redirects`; `err` ist der Grund als Text, wenn keine Antwort kam (ein 404 ist eine Antwort) |
| `location` | `href`, `scheme`, `host`, `hostname`, `port`, `path`, `query`, `fragment`; `assign(url)` oder `location.href = url` geht dorthin, `reload()` lädt neu |
| `print`, `log`, `log.warn`, `log.error` | in die Konsole des Browsers |
| `now()`, `rand()` | die Zeit der Simulation in ms; Zufall aus dem Seed des Projekts, dem Gerät, der Adresse und der Anfrage |

Dazu kommt die Standardbibliothek (`str`, `list`, `map`, `json`, `math`). **Mehr bekommt eine Seite
nicht:** keine Dateien des Geräts und keine des echten Rechners, kein `import`, kein `net`, `disk`
oder `http`, keine Zeit und kein Zufall von außen. Skripte, die später ins Dokument kommen
(`set_html`, `create`), laufen nicht. `fetch` darf jede `http`-Adresse fragen: Eine Same-Origin-Regel
gibt es hier nicht.

Ereignisse steigen vom Ziel über seine Eltern bis zum Dokument auf, so genügt für eine Liste ein
Handler an der Liste; eine Fangphase gibt es nicht. Ein Klick trifft das innerste Element unter dem
Zeiger. Nur die Standardaktion von Links und Formularen lässt sich aufhalten. Ein Formular geht mit
den Werten und der Adresse, die seine Handler ihm gelassen haben.

Die VM läuft auf dem UI-Thread, und keine Seite kann das Programm anhalten: Jedes Skript, jeder
Handler, Zeitgeber und jede Antwort hat 2 000 000 Schritte Fuel; danach endet dieser Aufruf mit einem
Fehler in der Konsole, und die Seite bleibt bedienbar. Ein Bild führt höchstens 4 000 000 Schritte
aus, der Rest wartet aufs nächste. Dazu Grenzen für Speicher (8 MiB), Aufruftiefe (200), das Dokument
(200 000 Knoten, 64 MiB, 1000 Ebenen), Zeitgeber, Handler und offene Anfragen. Fehler nennen die
Quelle (Adresse der Seite oder `src`) mit Zeile und Spalte in der Datei, auch beim Übersetzen; ein
Fehler hält das nächste Skript nicht auf. Die Statuszeile nennt die Zahl der Fehler, solange die
Konsole zu ist.

Alles läuft auf der Uhr der Simulation: Steht sie, warten Zeitgeber (Antworten auf Anfragen auch).
Zwei Läufe desselben Fensterskripts geben dasselbe Ergebnis. Anfragen sind Aktionen
(`Action::WebFetch`), das Zurückspulen spielt sie also mit. Die VM selbst gehört zum Fenster, nicht
zum Projekt: kein Rückgängig-Schritt, keine Änderung am Projekt, sie geht mit dem Fenster.

Die Konsole öffnet der Knopf neben „Öffnen“ (Strg+Umschalt+J); sie erscheint nur bei Seiten mit
Skripten.

Entscheidungen: **Eine VM je Dokument** statt je Fenster, denn der Verlauf behält je Eintrag sein
Dokument. Sie gehört zum Dokument wie die Feldwerte und die Stelle: Zurück und Vor bringen eine Seite
mit ihrem Zustand zurück, ohne Skripte neu zu starten; Neu laden macht eine neue. Solange eine Seite
nicht zu sehen ist, läuft nichts: Fällige Zeitgeber feuern beim Zurückkommen einmal, `timer.every`
macht von dort aus weiter, Antworten warten der Reihe nach. **Klicks** nennt die Ansicht der Library
als `MdEv::click`; das Element unter dem Zeiger sucht `MdDoc::element_at`, ohne dass sich an den Blöcken
etwas ändert. **Gemeinsame Namen** kommen aus `Vm::add_script`: ein Skript sieht die Namen der Skripte
davor, nicht die der späteren.

**Stylesheets (B4):** Jedes `<link rel="stylesheet" href>` holt der Browser einmal wie ein Bild und
gibt den Text dem Dokument der Library (`MdDoc::set_sheet`); er zählt dort, wo der `<link>` zwischen
den `<style>`-Elementen steht, mit Spezifität und Reihenfolge der CSS-Teilmenge der Library. Auch ein
`<link>`, den ein Skript einfügt, wird geholt. Medien außer `screen` und `all` bleiben draußen. Ein
Stylesheet, das nicht kommt (404, kein Server, ein anderer Typ als `text/css`, über 1 MiB), ist eine
Zeile in der Konsole, die Seite bleibt; die Statuszeile nennt die Zahl der Fehler. `@import` gehört
nicht zur Teilmenge. Holen kostet eine Anfrage je Adresse, bis Neu laden den Speicher des Fensters leert.
Seiten ohne Stylesheets rendern wie zuvor.

**Cookies (B4):** Die Dose gehört zum Browser des Geräts: Zustand des Fensters, nicht des Projekts.
[`netpkt::cookie`](../../lib/netpkt/cookie.rs) liest `Set-Cookie` nach RFC 6265 (`Path`, `Domain`,
`Expires`, `Max-Age`, `Secure`, `HttpOnly`) und `SameSite` aus RFC 6265bis, soweit ein kleiner Browser es
braucht: `Strict` nur innerhalb der Site, `Lax` und kein Attribut auch bei einer Navigation mit GET,
`None` immer, aber nur mit `Secure`. Die Site sind die letzten zwei Namensteile (ohne Public-Suffix-Liste;
ein `Domain` ohne Punkt zählt als öffentliches Suffix). Ablauf und `Max-Age` laufen auf der Uhr des Geräts,
also auf virtueller Zeit. Schließt man den Browser, gehen die Sitzungs-Cookies und die übernommenen
Zertifikate, Cookies mit Datum bleiben beim Gerätefenster. Was die Dose ablehnt, steht mit Grund in der
Konsole. Skripte lesen `document.cookie` ohne die `HttpOnly`-Cookies; eine Zuweisung
(`document.cookie = "a=1; Path=/"`) gilt am Ende des Aufrufs, wie `location.href`, denn `jfs` hat keine
Setter; `document.set_cookie(s)` gilt sofort.

Weiterleitungen folgt die Engine, also trägt jede Anfrage einen Schnappschuss der Dose
(`WebReq::jar`, ein `Arc`): Jeder Sprung liest seine `Cookie`-Zeile aus der eigenen Kopie, und ein
`Set-Cookie` unterwegs ändert sie für den nächsten Sprung. Im `Report::Web` kommen alle Sprünge mit ihren
Kopfzeilen und der Uhrzeit des Geräts zurück, und das Fenster legt sie der Reihe nach in die echte
Dose. Die Engine behält danach nichts. Weil der Schnappschuss in der Aktion steckt, spielt das
Zurückspulen genau dieselben Anfragen ab.

**HTTPS (B4): echtes TLS 1.3.** Auf dem Kabel liegen echte Records und ein echter Handshake:
ClientHello und ServerHello im Klartext, danach alles mit ChaCha20-Poly1305 verschlüsselt, das
Zertifikat echtes X.509 mit Ed25519, die Signatur und beide Finished geprüft. Der Kern steht ohne I/O in
[`netpkt::tls`](../../lib/netpkt/tls.rs) und [`netpkt::x509`](../../lib/netpkt/x509.rs) und ist in
`coretest` (Gruppe `tlscore`) gegen die RFC-8448-Werte und in beide Richtungen gegen `netio::tls` geprüft,
mit `python tools/tls_interop.py --core` auch auf echten Sockets gegen OpenSSL als Client und als Server.
Warum ein zweiter Kern: `netio::tls` nennt `plat` (Zufall, Uhr, Sockets), den darf der Simulator nicht
nennen; `lib/netpkt/ssh.rs` hat es ebenso gemacht.

**Was daran simuliert ist:** Die Schlüssel kommen aus dem Startwert des Projekts: Sie schützen nichts Echtes,
und eine Wiederholung ist Bit für Bit gleich. Der Handshake kostet keine virtuelle Rechenzeit, nur die des
Netzes. Einzige Wurzel ist die „JFNetworker Lab CA“, der jedes Gerät vertraut und die es nur hier gibt.
Es gibt nur eine Suite (`TLS_CHACHA20_POLY1305_SHA256`), X25519 und Ed25519, kein HelloRetryRequest, keine
Client-Zertifikate, keine Wiederaufnahme, keine Sperrlisten.

- **Server:** `http-server` mit `"tls"` spricht TLS, sein Port ist dann 443, wenn `"port"` nichts anderes
  sagt. `{"names": ["web.lab", "10.0.0.1"], "issuer": "lab" | "self", "org": "..", "days": 365, "age_days": 0,
  "starts_in_days": 0}` (alles darf fehlen; ohne `names` gilt der Gerätename, nur diese Namen zählen, nicht
  der Common Name; die Tage zählen vom Start der Simulation, so bleibt das Zertifikat bei einem Neustart
  gleich). Ein Skript: `http.serve(443, f, {tls: {names: ["web.lab"]}})`. Im Reiter Dienste steht
  „Port 443 (TLS)“, die Metriken sind `tls_handshakes` und `tls_failed`.
- **Browser:** Das Schloss neben der Adresse öffnet die Karte des Zertifikats (Inhaber, Aussteller, Zeiten,
  Namen, Fingerabdruck, Suite). Bei selbst signiert, unbekanntem Aussteller, abgelaufen, noch nicht gültig
  oder falschem Namen schickt der Browser den Alert, wie TLS es will, und zeigt die Warnseite: was nicht
  stimmt, „Trotzdem fortfahren“ und „Zurück zur sicheren Seite“. Weitermachen merkt sich dieses eine
  Zertifikat (SHA-256 des Blatts) im Fenster und fragt neu; das Schloss wird dann zum Warnzeichen.
- **Inspektor und Simulationsmodus:** Der Dissektor zeigt Records und was im Klartext geht, Feld für Feld
  (Version, Random, Suites, SNI, Key Share, Alerts), alles Verschlüsselte als `verschlüsselt`. Die
  Entscheidungen sind Notizen („TLS: Handshake mit … fertig“, „Zertifikat … abgelehnt“). TLS zählt zur
  Kategorie TCP/HTTP, eine neue Kategorie braucht es nicht.
- **curl:** `curl https://…` mit Exit 60 bei einem Zertifikatsfehler und 35 bei anderen TLS-Fehlern, `-k`
  macht trotzdem weiter, `-v` zeigt das Zertifikat, `-w` kennt `%{scheme}`, `%{ssl_verify_result}`,
  `%{time_appconnect}`. Weiterleitungen über das Schema hinweg gehen mit `-L`.

**Entwickleransicht (B4, [`src/workspace/devwin/devtools.rs`](src/workspace/devwin/devtools.rs)):** F12 oder der
Knopf neben „Öffnen“, unter der Seite mit einem Teiler, Zustand des Fensters ohne Rückgängig-Schritt und ohne
Änderung am Projekt.

- **Konsole:** was B3 hatte; ab jetzt auch auf Seiten ohne Skripte (Stylesheets, Cookies). Strg+Umschalt+J
  und der Knopf der Skriptseiten öffnen sie direkt. Am Ende bleibt sie auch, wenn die Skalierung wechselt.
- **Netzwerk:** jede Anfrage der Seite mit Methode, Adresse, Status, Typ, Größe, Zeit und wer fragte
  (Adresszeile, Link, Formular, `<img>`, `<link>`, `<script src>`, `fetch`, Skript); eine Weiterleitung ist
  eine eigene Zeile. Eine gewählte Zeile zeigt die Köpfe von Anfrage und Antwort, TLS und den Inhalt (Text bis
  32 KiB, sonst die ersten Bytes). Jedes Dokument hat sein Protokoll, es kommt mit Zurück wieder.
- **Elemente:** der Dokumentbaum, lebendig unter den Skripten, mit Attributen; ein gewähltes Element zeigt, was
  die Stylesheets ihm geben (`MdDoc::style_of`, ohne Geerbtes).
- **Cookies:** die Cookies dieser Seite mit Domain, Pfad, Ablauf und Merkmalen, einzeln oder alle zu löschen.

Anhalten (Esc) beendet alles, was unterwegs ist: die Seite, Bilder, Stylesheets, `fetch` und `src`; die
Rückruf-Funktion hört „Angehalten“.

Geprüft wird in den Selbsttest-Gruppen `browser` ([`src/browsertest.rs`](src/browsertest.rs)),
`pagescript` ([`src/pagescripttest.rs`](src/pagescripttest.rs)) und `webdev`
([`src/webdevtest.rs`](src/webdevtest.rs)), den coretest-Gruppen `cookie` und `tlscore`, dazu die Szenarien
`web_site.scn` und `web_https.scn` und die Fensterskripte `browser_pages.uis`, `browser_forms.uis`,
`browser_requests.uis`, `browser_shots.uis`, `browser_scripts.uis`, `browser_script_shots.uis`,
`browser_styles.uis`, `browser_cookies.uis`, `browser_https.uis`, `browser_devtools.uis` und
`browser_dev_shots.uis`.
### SSH

SSH Version 2 läuft auf den Geräten wie alles andere Byte für Byte: echte Schlüssel, echter
Schlüsselaustausch, echte Verschlüsselung, im Mitschnitt echte SSH-Pakete. Das Protokoll steht ohne
I/O in der Library ([`lib/netpkt/ssh.rs`](../../lib/netpkt/ssh.rs)): curve25519-sha256, Host- und
Benutzerschlüssel ssh-ed25519 und rsa-sha2-256, chacha20-poly1305@openssh.com, aes128-ctr und
aes256-ctr mit hmac-sha2-256, Anmeldung mit none, password und publickey, Sitzungskanäle mit
pty-req, shell, exec, Fenstern und exit-status. Geprüft ist es auch gegen den OpenSSH-Client
(`python tools/ssh_interop.py --core`). Der Kennungstext ist `SSH-2.0-JFNetworker_1`. Ohne
Schlüssel lauscht niemand; ältere Projekte laufen wie bisher.

- **IOS-artiger Server** ([`sshd.rs`](crates/jfn_sim/sshd.rs), [`cli/ssh.rs`](crates/jfn_sim/cli/ssh.rs)):
  `ip domain-name NAME` (auch `ip domain name`), `crypto key generate rsa [general-keys] [modulus
  N]` (1024 bis 4096, ohne: 2048), `crypto key generate ed25519`, `crypto key zeroize [rsa|ed25519]`,
  `ip ssh version 2`, `ip ssh time-out SEK` (120), `ip ssh authentication-retries N` (3), die
  `no`-Formen; auf den Leitungen `transport input ssh|telnet|all|none` (ohne: all, wie bei Telnet).
  Der Server lauscht auf TCP 22, solange ein Host-Schlüssel da ist und eine Leitung SSH nimmt und
  jemanden anmelden kann. Eine Verbindung ist eine Sitzung wie bei Telnet, auf der niedrigsten
  freien vty-Leitung; angemeldet wird über dasselbe Gate: `password` mit `login`, `login local`,
  `login authentication LISTE` über AAA und RADIUS. `access-class` und `exec-timeout` gelten
  genauso. `show ip ssh`, `show ssh`, `show crypto key mypubkey [rsa|ed25519]`, `show users`
  (Herkunft mit `(ssh)`), `debug ip ssh`.
- **IOS-artiger Client:** `ssh [-v 2] [-c CHIFFRE] -l BENUTZER [-p PORT] HOST`; er nimmt jeden
  Host-Schlüssel, wie die Vorgabe der Geräte.
- **Linux-Server** (Dienst `sshd`): `{"port": 22, "password": "…", "users": [{"name": "deploy",
  "password": "…"}], "password_auth": true, "key_auth": true}`. Ohne Passwort kommt root nur mit
  Schlüssel hinein; `~/.ssh/authorized_keys` (`/root`, sonst `/home/NAME`) nennt die Schlüssel. Seinen
  Host-Schlüssel legt er beim ersten Start in `/etc/ssh/ssh_host_ed25519_key(.pub)` an, als Datei des
  Geräts (ein Rückgängig-Schritt wie jede geschriebene Datei); ohne die Datei gibt der nächste Start
  einen neuen. Die Sitzung ist eine Shell auf `pts/N`; `who`, `w`, `tty` und `ps` zeigen sie, ein
  weiterer Benutzer hat seinen Namen, sein Heimatverzeichnis und den Prompt `NAME@HOST:~$ `.
- **Linux-Client:** `ssh [-p PORT] [-l BENUTZER] [-i DATEI] [-c CHIFFRE] [-o OPTION=WERT] [-q]
  [BENUTZER@]HOST [BEFEHL …]`. Das erste Mal fragt er in eigenen Worten, ob er den Host-Schlüssel
  merken soll (`~/.ssh/known_hosts` im Format von OpenSSH, `[host]:port` bei anderem Port); ein
  geänderter Schlüssel wird abgelehnt, ohne Frage. `-o StrictHostKeyChecking=yes|no|accept-new|ask`,
  `-o BatchMode=yes`, `-o UserKnownHostsFile=DATEI`. Er meldet sich mit `~/.ssh/id_ed25519` oder
  `~/.ssh/id_rsa` an, wenn es sie gibt, sonst mit einem Passwort (verdeckt getippt, drei Versuche).
  Sein Status ist der der Gegenseite (`exit N`, der Befehl), 255 bei einem Fehler. Strg+C wirkt auf
  der Gegenseite, die Zeile `~.` beendet die Verbindung.
- **`ssh-keygen`:** `[-t ed25519|rsa] [-b BITS] [-f DATEI] [-N ""] [-C KOMMENTAR] [-q]`, `-l -f DATEI`,
  `-y -f DATEI`, `-R HOST`; die Dateien haben das Format von OpenSSH (ohne Passwort).
- **`scp`:** `[-P PORT] [-i DATEI] [-q] [-o OPTION=WERT] QUELLE ZIEL`, eine Seite `[BENUTZER@]HOST:PFAD`:
  eine Datei hin oder her über das alte rcp-Protokoll (`scp -t`, `scp -f`), das der Linux-Server selbst
  spricht.
- **Paket-Tab:** Kennungstext, Paketlänge, Padding und Name der Nachricht, solange die Verbindung
  im Klaren läuft (die Listen eines KEXINIT, die Werte des Austauschs), danach „verschlüsselt“.
  Die Kategorie bleibt TCP/HTTP. **Notizen:** `Note::Ssh(SshNote)` für alles, was Server und Client
  entscheiden; die Probleme-Liste bleibt dafür offen.

Entscheidungen: Eine Verbindung ohne Anmeldung belegt die vty-Leitung schon, wie beim Vorbild. Eine
Leitung ohne `login` lässt jeden hinein, eine mit `login` und Leitungspasswort nimmt das Passwort bei
jedem Benutzernamen. Ein Schlüssel ist eine Datei in `nvram:` (`dir nvram:` zeigt ihn), keine Zeile der
Running-Config: er bleibt über einen Neustart und liegt im Projekt; eine Zeile `crypto key generate`
in einer Startkonfiguration erzeugt nur, solange keiner da ist, und meldet beim Start nichts ans
Projekt (derselbe Seed ergibt denselben Schlüssel). Ein neuer Schlüssel ersetzt den alten ohne
Frage. Der Schlüssel braucht einen eigenen Gerätenamen und `ip domain-name`, wie in den klassischen
Übungen. Unser Client schickt ganze Zeilen und zeigt, was kommt (wie bei Telnet): Terminalsteuerung
wird nicht ausgewertet, eine Zeile, die auf „assword: “ endet, verdeckt die nächste Eingabe. Virtuelle
CPU-Zeit kostet der Schlüsselaustausch nicht. Zufall kommt aus dem Strom des Geräts: 32 Bytes je
Verbindung als Seed des Kerns, daraus Schlüssel, Nonces und Padding.

Nicht nachgebildet: Port-Weiterleitung, Agent, Kompression, Schlüsseldateien mit Passwort, DSA und
ECDSA, `ip ssh pubkey-chain` (IOS-artig nur Passwörter), die strikte Schlüsselaushandlung von OpenSSH,
Host-Schlüssel je Adresse.

Für Programme: `SshServer`, `SshCfg`, `SshNote`, `Conn::ssh`, `SesInfo::account`; in der Library
`netpkt::ssh` (`Core`, `Event`, `KeyPair`, `Cipher`, `dissect`) und `crypto::rsa::PrivateKey`. Geprüft wird
in der Selbsttest-Gruppe `ssh` ([`src/sshtest.rs`](src/sshtest.rs)) und der coretest-Gruppe `sshcore`,
dazu das Szenario `ssh_snmpv3.scn` und die Fensterskripte `ssh_session.uis`, `snmpv3_mib.uis` und
`ssh_shots.uis`.
### 802.1X und WPA-Enterprise

Ein Port eines IOS-artigen Switches kann die Anmeldung verlangen, bevor er Daten durchlässt, und
ein Access Point kann sein Netz über denselben Weg schützen (IEEE 802.1X-2004, RFC 3748, 3579,
5931). Auch hier fließen echte Bytes: EAPOL auf dem Kabel oder der Funkstrecke, EAP in RADIUS,
echte Prüfsummen und Schlüssel; der Paket-Tab zeigt alles Feld für Feld, der Simulationsmodus hat
für EAPOL die Kategorie „802.1X“ in einem gedämpften Rostrot.

- **Am Switch** ([`dot1x.rs`](crates/jfn_sim/dot1x.rs), [`cli/dot1x.rs`](crates/jfn_sim/cli/dot1x.rs)):
  `aaa new-model`, `aaa authentication dot1x default group radius` (auch `group NAME`, `local`),
  `dot1x system-auth-control`, am Port `authentication port-control
  auto|force-authorized|force-unauthorized`, `dot1x pae authenticator`, `dot1x timeout
  tx-period|quiet-period|reauth-period|supp-timeout N`, `dot1x max-req N`, `authentication
  periodic`, `authentication timer reauthenticate N`. Ein Port unter `auto` ist geschlossen, bis der
  Server zustimmt: nur EAPOL kommt durch, nichts wird gelernt, seine LED leuchtet gelb und das
  Kabel heißt „nicht autorisiert“. Ablauf: EAP-Request/Identity an die Gruppenadresse, sobald der
  Link steht, dann alle `tx-period`; die Antworten des Supplicants gehen in Access-Requests mit
  State und Message-Authenticator zum Server, jede Access-Challenge kommt als nächste Anfrage
  zurück (wiederholt nach `supp-timeout`, bis `max-req`). Nach Access-Accept ist der Port offen
  (`show dot1x`, `show authentication sessions` in eigenem Aufbau, `debug dot1x`, `%DOT1X-…` auf
  der Konsole), mit `authentication periodic` wird nach `reauth-period` oder dem Session-Timeout des
  Servers neu gefragt, der Port bleibt dabei offen. Nach Access-Reject bleibt er für `quiet-period`
  zu. `dot1x re-authenticate interface IF` und `clear authentication sessions` fragen sofort neu.
- **VLAN vom Server:** Tunnel-Type, Tunnel-Medium-Type und Tunnel-Private-Group-ID (RFC 3580) legen
  den Port für die Dauer der Sitzung in dieses VLAN, seine Einstellung bleibt; ein VLAN, das der
  Switch nicht kennt, lässt den Port zu.
- **RADIUS-Server** ([`radiusd.rs`](crates/jfn_sim/radiusd.rs)): antwortet EAP mit seinen Benutzern,
  `eap_methods` (ohne Angabe MD5, dann PWD), `eap_id`, je Benutzer `vlan` und `session_timeout`:

  ```json
  {"kind": "radius-server", "name": "radius", "config": {"clients": [{"address": "10.0.0.0/24",
   "secret": "testing123"}], "users": [{"name": "anna", "password": "geheim", "vlan": 20}]}}
  ```
- **Methoden** ([`lib/eapm`](../../lib/eapm/lib.rs), [`crypto::eappwd`](../../lib/crypto/eappwd.rs)):
  EAP-MD5 für Kabelports, ohne Schlüssel; EAP-PWD (RFC 5931 auf P-256 mit SHA-256): nur ein
  Passwort, keine Zertifikate, und es ergibt MSK und EMSK. Bei NAS-Port-Type Wireless-802.11 bietet
  der Server nur Methoden mit Schlüsseln an.
- **Supplicant** ([`supp.rs`](crates/jfn_sim/supp.rs)): `/etc/wpa_supplicant/wpa_supplicant.conf` mit
  `network={ … }`-Blöcken (`key_mgmt=IEEE8021X` am Kabel, `key_mgmt=WPA-EAP` und `ssid` im WLAN,
  `eap=MD5|PWD`, `identity`, `password`). Die Datei ist ein Schritt des Projekts; wie beim echten
  Programm zählt eine Änderung erst beim Start oder nach `wpa_cli reconfigure`, Rückgängig beendet
  also keine laufende Anmeldung. Liegt die Datei auf dem Gerät, läuft er von selbst;
  `wpa_supplicant [-B] -i IF -c DATEI`, `wpa_cli status|list_networks|reconfigure|logon|logoff|
  reauthenticate`. Antwortet niemand auf drei EAPOL-Start (alle 30 s), läuft der Port ohne 802.1X;
  nach einer Ablehnung wartet er 60 s. Nach dem Erfolg startet ein DHCP-Client des Ports ohne Lease
  sofort. Im Desktop schreibt das Feld „Anmeldung am Netz (802.1X)“ der **IP-Konfiguration** diese
  Datei (Methode, Identität, Passwort, am Kabel und fürs WLAN) und zeigt den Zustand. Alle
  Endgeräte haben die Shell und damit den Supplicant; das IP-Telefon hat nur Brückenports und
  meldet sich nicht an.
- **WPA-Enterprise** ([`wlan.rs`](crates/jfn_sim/wlan.rs)): Sicherheit `wpa2-enterprise` (AKM 1) und
  `wpa3-enterprise` (AKM 5, mit AES-128-CMAC und KDF-SHA256, auch auf 6 GHz); ein Netz nennt
  `radius`, `radius_key`, `radius_port`, im Inspektor RADIUS-Server und Schlüssel. Eine Station hat
  `identity` und `eap` neben der Passphrase als Passwort, oder einen Block der Datei. Nach der
  Assoziation läuft EAP über die Funkstrecke (die Anfrage kommt nach 3 s wieder), der Access Point
  fragt seinen Server über sein BVI; das MSK kommt in MS-MPPE-Recv-Key und -Send-Key (RFC 2548) zu
  ihm und aus der Methode zur Station, die ersten 32 Bytes sind das PMK, und der echte 4-Wege-Handshake
  folgt. Eine Ablehnung oder ein Erfolg ohne Schlüssel beendet die Zuordnung (Grund 23) und zählt
  wie eine falsche Passphrase. Beim Roaming läuft EAP neu, es gibt kein PMK-Caching.
- **Probleme:** `Dot1xNoServer` (auto ohne Methodenliste oder Server: der Port bleibt zu, Fehler),
  `Dot1xOff` (auto ohne `dot1x system-auth-control`: der Port bleibt offen, Warnung).

Entscheidungen: PEAP-MSCHAPv2 und EAP-TLS gibt es nicht: TLS liegt in `lib/netio` und nennt `plat`
(ein Kern ohne I/O wäre ein Umbau der Library), MSCHAPv2 bräuchte auch MD4 und DES. EAP-PWD ist die
schlüsselerzeugende Methode, weil es ohne Zertifikate auskommt. Beide Rollen sind verborgene Dienste
(`dot1x-auth`, `wpa_supplicant`) mit Dienst-Timern, es gibt keinen festen Timer-Slot. Eine
Port-Zeile `authentication port-control auto` genügt, `dot1x pae authenticator` steht nur auf
Wunsch in der Konfiguration, `no dot1x pae authenticator` schaltet den Port aus. „local“ in der
Methodenliste heißt: der Switch prüft EAP-MD5 selbst gegen `username NAME password …` (ein Hash
kann keine Challenge beantworten); ohne Server und ohne „local“ bleibt der Port zu. Ein Port ist
„multi-host“: wer dahinter hängt, kommt mit durch.

Noch offen: Host-Modi (single-host, multi-domain für IP-Telefone), MAB, Guest- und Auth-Fail-VLAN,
Accounting für 802.1X, TLS-Methoden, PMK-Caching und 802.11r.

Für Programme: `Action::Supplicant { dev, conf }`, `Engine::dot1x_sessions(dev)`,
`dot1x_port_closed(dev, port)`, `supp_views(dev)`, `supp_conf(dev)`, `wlan_creds(dev, ssid)`,
`Led::Unauth`, `Note::Dot1x(DNote)`, `jfn_sim::supp` (`NetBlock`, `parse_conf`, `conf_text`),
`netpkt::eap`, `netpkt::eapol::{start, logoff, eap_pdu, PAE_GROUP}`, `netpkt::radius` (`push_eap`,
`push_vlan`, `push_msk`, `mppe_seal`, …), `crypto::p256` (die Gruppe), `eapm::{Peer, Server}`. Geprüft
wird in der Selbsttest-Gruppe `dot1x` ([`src/dot1xtest.rs`](src/dot1xtest.rs)), in der coretest-Gruppe
`eap` (Codecs und die Vektoren einer unabhängigen Python-Referenz von EAP-PWD), mit
`scen_dot1x_imap.uis` und den Fensterskripten `dot1x_panel.uis` und `dot1x_shots.uis`.

### Hardware: CPU und RAM der Geräte

Netz, TCP, HTTP und Warteschlangen sind echt; ohne Hardware kostet Rechenzeit aber nichts, und ein
Host mit zehn Diensten hat zehnmal so viel Kapazität. Mit Hardware lässt sich fragen, wann ein größerer
Rechner mehrere kleine schlägt. Das ist das Fundament des Software-Architektur-Labors.

**Ein Projekt ohne Hardware läuft Byte für Byte wie zuvor:** kein neues Ereignis, keine Zufallszahl, keine
Reihe, und `"hw"` und `"metered"` stehen nicht in der Datei, solange sie nicht gesetzt sind. Alle Golden-Berichte
prüfen das.

Einschalten: **Hardware simulieren** auf der Projektseite des Inspektors (`Project::metered`, ein
Rückgängig-Schritt, ohne Neustart) gibt jedem Host die Werte seines Modells (`catalog::hw_of`, eine Tabelle neben
den Modellen). Ein Gerät kann eigene haben (`Device::hw`, auch ohne den Schalter). Lastgeneratoren, Netzgeräte und
Smartphones bleiben ohne.

| Modell | Kerne | Tempo | RAM | Swap | Disk |
|---|---|---|---|---|---|
| Rack-Server S | 4 | 110 % | 16 GiB | 2 GiB | SATA-SSD 480 GiB |
| Rack-Server M | 8 | 100 % | 32 GiB | 4 GiB | NVMe-SSD 960 GiB |
| Rack-Server L | 16 | 95 % | 64 GiB | 8 GiB | NVMe-SSD 1920 GiB |
| Rack-Server XL | 32 | 90 % | 256 GiB | 16 GiB | schnelle NVMe-SSD 3840 GiB |
| NAS | 2 | 70 % | 8 GiB | 2 GiB | Festplatte 8000 GiB |
| Rack-Server 1U, 1U SFP28 | 8 | 100 % | 32 GiB | 4 GiB | SATA-SSD 960 GiB |
| Tower-Server, PC | 4 | 100 % | 16 GiB | 2 GiB | SATA-SSD |
| Laptop | 4 | 80 % | 16 GiB | 2 GiB | NVMe-SSD 480 GiB |
| Load Balancer | 8 | 100 % | 16 GiB | 0 | SATA-SSD 240 GiB |
| DB-Shard | 16 | 100 % | 64 GiB | 8 GiB | NVMe-SSD 1920 GiB |
| Cache | 4 | 100 % | 64 GiB | 0 | SATA-SSD 240 GiB |
| Message Queue | 4 | 100 % | 16 GiB | 2 GiB | NVMe-SSD 480 GiB |
| Container-Host | 16 | 100 % | 64 GiB | 8 GiB | NVMe-SSD 960 GiB |

Die Zahlen sind unsere **Annahmen** in runden Werten, keine Messungen an einem Produkt; Tooltip der Palette und
Inspektor sagen das. Mehr Kerne kommen mit weniger Takt, damit „größer“ nicht einfach „mehr von allem“ ist.

**Das Modell** ([`res.rs`](crates/jfn_sim/res.rs), gebaut wie [`disk.rs`](crates/jfn_sim/disk.rs)):

- **CPU:** Wer zuerst kommt, mahlt zuerst, über alle Kerne. Eine Aufgabe nimmt den Kern, der als erster frei wird, und ihr
  Ende steht beim Einreihen fest; das ist genau eine zentrale FIFO-Warteschlange (M/G/c), kostet O(Kerne) und kein
  Ereignis. Kosten gelten für einen Referenzkern: Tempo 100 % heißt, 1 ms kostet 1 ms und ein Skriptbefehl seine 5 ns.
  Lange Aufgaben kommen in Stücken von 4 ms, damit ein Scan kurze Anfragen nicht um seine ganze Länge aufhält.
- **Konkurrenz:** Echte Warteschlangen, wo die Sache modelliert ist (die Sperre `serial_us`, die Disk). Dazu auf Wunsch je
  Dienst der Faktor des Universal Scalability Law, `"usl": {"sigma": 0.05, "kappa": 0.02}`: die Kosten mal
  1 + σ(N−1) + κN(N−1), N die mit dieser Aufgabe belegten Kerne. Das gibt fallende Kurven mit dem Gipfel bei √((1−σ)/κ). Es
  ist die Formel, keine Messung von Sperren und Cache-Zeilen.
- **Speicher:** Jeder Dienst hält eine Grundmenge, je Verbindung und seine Daten (`"mem_mb"`, `"conn_kb"`, `"data_mb"`; das OS
  fragt `Service::mem` nach jedem Aufruf). Über den RAM hinaus steigen die Kosten linear bis zum 20-Fachen am Ende des Swaps, und
  Seiten kommen über die echte Disk herein (je 10 µs Rechenzeit eine berührte Seite, davon der Anteil außerhalb des RAMs; ein Zähler
  entscheidet, keine Zufallszahl). Über RAM und Swap hinaus endet der größte Dienst mit Status 137, was Restart-Regeln einer Einheit
  sehen.
- **Größe ändern:** `Action::Resize { dev, hw, downtime }`, im Fenster und mit `resize` im Szenario. Standard ist ein Neustart mit
  30 s Ausfall, eine Marke auf der Zeitleiste (`Fault::Resize`); was auf der Disk liegt, bleibt, Kerne und Speicher starten leer. Ohne
  Ausfall (`downtime: 0`, „Im Betrieb ändern“) nimmt das laufende System die Werte: neue Aufgaben zahlen das neue Tempo, Kerne, die
  gehen, geben ihre wartende Arbeit weiter.
- **Laute Nachbarn:** Die Fehler `Fault::SlowCpu` (alle Rechenzeit mal Faktor) und `Fault::MemHog` (etwas anderes hält so viel
  Speicher) im Inspektor und mit `fault cpu`, `fault mem` im Szenario.

**Was die Dienste zahlen:**

| Dienst | Rechenzeit | Speicher (Grund, je Verbindung) |
|---|---|---|
| `http-server` | `cpu_ms` je Route oder Server, zuerst und auf den Kernen (Standard 20 µs); `time_ms` bleibt reine Wartezeit; `serial_us`: eine Sperre, einer zur Zeit, im Tempo der Kerne; `usl` | 32 MiB, 64 KiB, `data_mb` |
| `lb` | `cpu_us` je HTTP-Anfrage, bevor ein Backend gewählt wird (Standard 50 µs); TCP-Durchleitung kostet nichts | 16 MiB, 64 KiB |
| `cache` | `hit_us` je Treffer auf den Kernen (ohne Hardware ein reiner Zeitgeber wie bisher) | 16 MiB, 64 KiB, die gehaltenen Antworten |
| `kv` | `cpu_us` je Anfrage (Standard 30 µs) | 64 MiB, 64 KiB, die gehaltenen Daten |
| `worker` | `cpu_ms` je Nachricht (Standard 0); `time_ms` bleibt Wartezeit | 16 MiB, 64 KiB |
| Skripte | jedes Stück (`slice`) nimmt sich seinen Platz in der Warteschlange der Kerne; `ns_per_op` gilt am Referenzkern | 8 MiB, 64 KiB, `data_mb`, der Heap |
| `http-load`, Protokolldienste | nichts: Lastgeneratoren zahlen nie | keiner |

**Sehen und prüfen:**

- Inspektor eines Hosts: Karte **Hardware** mit CPU, Warteschlange und Speicher von jetzt, Vorlagen aus dem Katalog, Kerne, Tempo, RAM,
  Swap, Disk und dem Schalter „Im Betrieb ändern“. Darunter im Fehlerkasten „Bremsen“ und „Belegen“.
- Panel **Metriken**: eine eigene Zeile „Hardware“ unter den Diensten des Geräts, mit Kacheln und Verläufen für CPU, Warteschlange,
  Speicher und Disk.
- Konsole: `top` zeigt die echten Kerne, Last und Speicher; `free` und `lscpu` gibt es nur auf Geräten mit Hardware.
- Szenario: Die Reihen heißen wie ein Dienst `_host`: `expect WEB1 svc _host metric cpu_pm >= 990 over 3s`. Reihen: `cpu_pm`, `running`,
  `runq`, `mem_bytes`, `swap_bytes`, `pageins`, `oom_kills`, `disk_bytes`; ohne Zeitraum auch `cores`, `speed_pm`, `ram_bytes`,
  `busy_ms`, `jobs`, `wait_ms`.
- Schritte: `resize DEV model server-l`, `resize DEV cores 8 speed 1.5 ram 32G [swap 4G] [disk nvme] [disk_gb 960] [down 10s] [live]`,
  `fault cpu DEV factor 3 [for 5s]`, `fault mem DEV 12G [for 5s]`, `heal cpu DEV`, `heal mem DEV`.

**Was es nicht modelliert** (bewusst, damit niemand die Zahlen für mehr hält, als sie sind):

- Eine Reservierung lässt sich nicht zurücknehmen: Die wartende Arbeit eines beendeten Dienstes hält ihre Kerne, bis sie um wäre (Strom aus
  beendet sie).
- Keine CPU je Rahmen: Leitungsrate und Warteschlangen der Kabel sind dort der Engpass.
- Keine Prozessor-Aufteilung: Die Sättigung ist dieselbe, aber Enden würden sich bei jeder Ankunft verschieben. Innerhalb eines Stücks von
  4 ms läuft eine Aufgabe durch, und ein Stück eines Skripts überholt die schon zugesagten Aufgaben ein wenig (seine Arbeit zählt voll).
- Keine NUMA-, Cache- oder Hyper-Threading-Effekte außer dem, was `usl` ausdrückt; kein Speicher des Betriebssystems selbst; Caches werden unter Druck
  nicht freigegeben.
- Die Queue `mq` rechnet nicht und hält nur ihre Grundmenge; ein `db`-Dienst mit Puffer und Index kommt mit SA4.

Für Programme: `Action::Resize`, `Action::Metered`, `Fault::SlowCpu`, `Fault::MemHog`, `Fault::Resize`, `Engine::res_view(dev)`,
`Engine::res_of(dev)`, `Service::mem`, `OsApi::cpu`, `cpu_usl`, `cpu_or_timer`, `cpu_slice`, `cpu_slice_usl`, `metered`, `cpu_turn`, `cpu_charge`,
`cpu_ps`, `res::CpuGate`, `res::SvcMem`, `res::Usl`, `catalog::{Hw, DiskClass, hw_of, hw_presets}`. Geprüft wird in der Selbsttest-Gruppe
`hardware` ([`src/hardwaretest.rs`](src/hardwaretest.rs)), mit `scen_hw_basic.uis` und den Fensterskripten `hardware_panel.uis` und
`hardware_shots.uis`. `JFNetworker --selftest --only=hardware --numbers` schreibt jeden Vergleich mit einer Formel mit seinen Zahlen.

**Tempo** (Windows, Entwicklungs-Build, Mitschnitt an, Rechner nicht frei; offene Last, je 10 s virtuelle Zeit, ein Thread):

| Netz | Last | Laufzeit | Ereignisse/s | Anfragen je Wandsekunde | Ereignisse je Anfrage |
|---|---|---|---|---|---|
| Generator am Kabel eines Servers, 1 ms Rechenzeit, 32 Kerne | 1000/s | 249 ms | 330 000 | 40 000 | 8 |
| dasselbe | 5000/s | 1244 ms | 323 000 | 40 000 | 8 |
| `lab_shards`: Loadbalancer, drei Webserver, drei KV-Knoten | 2000/s | 2528 ms | 397 000 | 7500 | 53 |
| dasselbe | 10 000/s | 14 012 ms | 351 000 | 6800 | 52 |

Hardware selbst kostet wenig: Gegenüber demselben Lauf ohne Hardware, aber mit 1 ms Wartezeit (`time_ms`), sind es ohne neue Ereignisse rund 3 %. In `lab_shards`
kommen rund 8 % Ereignisse dazu, weil `lb` und `kv` eine Anfrage bis zum Ende ihrer Rechenzeit halten.

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
| 4 Parität | W1 WLAN ✓ · A1 Aufgaben mit Bewertung ✓ · R6 BGP ✓ · K1 Zurückspulen mit Keyframes ✓ · P1 physische Ansicht ✓ · G1 Gruppen und Hintergrundbild ✓ · X1 weitere Dienste (X1a Dateien, TFTP und FTP ✓; X1b SNMP ✓; X1c Mail mit SMTP, POP3 und dem Programm E-Mail ✓; X1d Logins, Telnet-Client und AAA mit RADIUS ✓; der Telnet-Server auf den vty-Leitungen folgt mit X1e in Phase 6; X1f SSH und SNMPv3 ✓; X1g 802.1X, WPA-Enterprise und IMAP ✓) |
| 5 Skripte und Web | SK5 `.jfs`-Dateien: außerhalb ausführbar mit dem Runner `jfs` der Library, aufgegangen in IDE1 und V2 (Dateien auf dem Gerät, Import und Export im Dateimanager) ✓ · B1 Webbrowser: Seiten über HTTP laden und mit `ui.html_view` zeigen, Links, Verlauf, Fehlerseiten ✓ · B2 Formulare (GET / POST) ✓ · B3 Seitenskripte `<script type="text/jfs">` mit Browser-Host (`document`, Ereignisse, `fetch`) ✓ · B4 CSS-Teilmenge, Cookies, HTTPS mit echtem TLS 1.3 im Simulator, Entwickleransicht ✓ (Runner und Library-Bausteine: [ROADMAP](../../docs/ROADMAP.md) Abschnitt 7) |
| 6 Virtueller Arbeitsplatz | L7 Fenster in einer Fläche, Taskleiste und Vollbild (Library) ✓ · SK6 Sprachdienste für jfs und IDE-Funktionen des Code-Editors ✓ · SK7 Module, Debug-Schnittstelle und Tests für jfs ✓ · V1 Shell mit Ordnern, klassischen Werkzeugen und Prozessen ✓ · X1e mehrere Befehlszeilen-Sitzungen je Gerät, Telnet-Server auf den vty-Leitungen ✓ · V2 Mini-Desktop: Programme in Fenstern, Taskleiste, Startmenü, Dateimanager, Systemmonitor ✓ · IDE1 IDE für jfs statt des Reiters Skripte: Explorer, Tabs, Debugger, Testlauf ✓ · A2 Software-Architektur: Dienste als Units, Bereitstellen auf Gruppen von Hosts, Lasttests, Beispiel-Labor ✓ |
| 7 Breite und Tiefe | Z1 Dock-Reiter erst nach 0,3 s Halten ziehen ✓ · H1 mehr Gerätemodelle ✓ · H2 Kabelauswahl und neue Kabelarten ✓ · H3 schmale Beschriftung ✓ · ST1 Startseite mit Vorlagen ✓ · F1 Firewall-Oberfläche · F2 Zonen und Stateful · AK1 bis AK4 Akademie (Kurse, Tests und Fehlersuche, Installieren, Inhalte) · SA1 bis SA7 Architektur-Labor (Ressourcen, Sweeps, Experimente, Datenbank, Replikation, Verkehrssteuerung, Autoscaler) · H4 Steckplätze |

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
