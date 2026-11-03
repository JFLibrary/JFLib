# Von unten nach oben

Ein Netz ist in Schichten gebaut, und jede Schicht verlässt sich auf die darunter. Ein Webabruf braucht einen aufgelösten Namen; der braucht ein Paket zum Namensserver; das braucht einen Weg; der braucht Rahmen auf einem Kabel. Darum ist die untere Hälfte des Stapels der vernünftige Anfang: Solange ein Kabel fehlt, sind alle Aussagen über Routen und Namen wertlos.

![Vier Schichten übereinander, jede mit ihren Befehlen: Kabel und Ports, Rahmen, Wege, Anwendung](img/schichten-de.svg)

## Vier Fragen, von unten nach oben

**1. Kabel und Ports: Steht die Verbindung?**
Auf einem Host zeigt `ip -br link`, ob die Schnittstelle `UP` ist. `NO-CARRIER` heißt: kein Partner am anderen Ende. Auf einem Router oder Switch zeigt `show ip interface brief` den Zustand je Schnittstelle, und `show interfaces GigabitEthernet0/0/1` zeigt auch, warum. Ein `administratively down` ist abgeschaltet, ein `down` ohne diesen Zusatz hat keinen Partner. Außerdem hilft der Blick auf die Zeichenfläche: Die Lämpchen an den Kabeln gehen an und aus.

**2. Rahmen: Findet man den Nachbarn?**
`ip neigh` zeigt, zu welchen Adressen ein Host eine MAC-Adresse kennt. Fehlt die Zeile, blieb die ARP-Frage ohne Antwort; ein echtes Linux zeigt dann oft eine Zeile mit `FAILED`. `show mac address-table` auf dem Switch zeigt, welche Adresse an welchem Port gesehen wurde, und `show vlan brief`, welcher Port zu welchem VLAN gehört.

**3. Wege: Kommt ein Paket hin und zurück?**
`ip addr` und `ip route` zeigen, was ein Host über sich glaubt. `ping` prüft einen Weg, und zwar beide Richtungen auf einmal. `traceroute` zeigt, wie weit Antworten zurückfinden. `show ip route` zeigt, was ein Router weiß.

**4. Anwendung: Tut der Dienst, was er soll?**
`dig` fragt den Namensserver, `curl` holt eine Webseite, `ss` zeigt, wer auf welchem Port lauscht. `cat /etc/resolv.conf` zeigt, welchen Namensserver ein Host fragt.

## Muss man immer ganz unten anfangen?

Nein. Das Vorgehen ist ein Werkzeug und keine Vorschrift. Wenn ein Ping auf die Adresse klappt und nur der Name nicht, wissen Sie schon, dass die unteren drei Fragen beantwortet sind: Die Antwort kam ja zurück. Dann fangen Sie oben an.

Umgekehrt hat das Vorgehen von unten einen Vorteil, den man erst mit der Zeit schätzt: Es kostet wenig, und es verhindert, dass Sie eine Stunde in Routen suchen, während ein Stecker lose ist. Das passiert auch Erfahrenen.

Ein Fehler in einer oberen Schicht ist oft erst zu sehen, wenn die unteren tragen. Wer ein gezogenes Kabel repariert und dann immer noch keine Webseite bekommt, hat nicht versagt. Er ist eine Schicht weiter.

Im Simulator gibt es noch den Simulationsmodus: Er zeigt jedes Paket und was jedes Gerät damit tut. Ein so genauer Blick ist in echten Netzen teurer; dort braucht es Mitschnitte an der richtigen Stelle. In den Fehlersuchen dieses Kurses dürfen Sie ihn benutzen, aber versuchen Sie es zuerst mit den Befehlen, denn die haben Sie überall.

Wenn klar ist, in welcher Schicht es hakt, bleibt die Frage nach dem Wo. Dazu mehr in der [nächsten Lektion](step:eingrenzen).
