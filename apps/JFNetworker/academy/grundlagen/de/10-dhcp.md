# DHCP: Adressen, die von selbst kommen

Bisher haben Sie Adressen von Hand vergeben. Bei drei PCs geht das, bei dreihundert nicht: Es kostet Zeit, und früher oder später hat ein Gerät dieselbe Adresse wie ein anderes. **DHCP** (Dynamic Host Configuration Protocol) übernimmt das: Ein Server verwaltet einen Vorrat an Adressen, den **Pool**, und verleiht sie an Geräte, die danach fragen. Er gibt nicht nur die Adresse heraus, sondern auch Maske, Gateway und Namensserver.

## Die vier Nachrichten

Ein neuer Client hat noch keine Adresse und kennt keinen Server. Darum beginnt alles mit einem Broadcast.

![Vier Pfeile zwischen Client und Server, abwechselnd hin und zurück: Discover, Offer, Request, Ack](img/dhcp.svg)

1. **Discover** (Client an alle): „Ich brauche eine Adresse, ist hier ein Server?“
2. **Offer** (Server an den Client): „Du könntest 10.10.0.100 haben.“ Es können mehrere Server antworten.
3. **Request** (Client an alle): „Ich nehme 10.10.0.100 von diesem Server.“ Auch das ist ein Broadcast, damit andere Server wissen, dass sie ihr Angebot zurückziehen können.
4. **Ack** (Server an den Client): „Abgemacht. Hier ist alles Weitere, und sie gehört dir für diese Zeit.“

Der Client benutzt die Adresse erst nach dem Ack. Diese Zeit ist die **Lease**. Bei der Hälfte fragt der Client den Server, ob er verlängern darf, und zwar direkt und ohne Broadcast. Ein Gerät, das geht, gibt seine Adresse also spätestens nach Ablauf zurück.

Weil Broadcasts keinen Router passieren, reicht der Discover nur bis zum Rand des Netzes. Steht der Server in einem anderen Netz, braucht es einen Vermittler, das DHCP-Relay. Davon handelt der Kurs Fehlersuche.

## Einen Pool einrichten

Ein Router mit IOS-artiger Befehlszeile kann selbst DHCP-Server sein:

```console
R1>enable
R1#configure terminal
Configuration mode: one command per line; end or Ctrl+Z leaves it.
R1(config)#ip dhcp excluded-address 10.10.0.1 10.10.0.99
R1(config)#ip dhcp pool BUERO
R1(dhcp-config)#network 10.10.0.0 255.255.255.0
R1(dhcp-config)#default-router 10.10.0.1
R1(dhcp-config)#end
*Mar  1 00:00:10.000: %SYS-5-CONFIG_I: configuration changed at the console
```

Die Zeile mit `excluded-address` hält Adressen zurück, die der Pool nie vergeben soll, etwa für den Router selbst und Geräte mit festen Adressen. `network` sagt, aus welchem Netz der Pool vergibt, `default-router` nennt den Clients ihr Gateway. Mit `dns-server 10.30.0.53` käme der Namensserver dazu.

## Zusehen

```console
root@PC1:~# dhclient -v eth0
DHCPDISCOVER on eth0 to 255.255.255.255 port 67 interval 3 (xid=0x684e7a3d)
DHCPOFFER of 10.10.0.100 from 10.10.0.1
DHCPREQUEST for 10.10.0.100 on eth0 to 255.255.255.255 port 67 (xid=0x684e7a3d)
DHCPACK of 10.10.0.100 from 10.10.0.1 (xid=0x684e7a3d)
bound to 10.10.0.100 -- renewal in 32811 seconds.
root@PC1:~# ip route
default via 10.10.0.1 dev eth0
10.10.0.0/24 dev eth0 proto kernel scope link src 10.10.0.100
```

`dhclient -v` nennt die vier Nachrichten beim Namen, und zwischen `DHCPDISCOVER` und `DHCPOFFER` vergeht eine Sekunde: So lange prüft der Server mit zwei Pings, ob die Adresse wirklich frei ist. Das tun echte Geräte dieser Art auch, und zwar genau deshalb, damit keine Adresse doppelt vergeben wird. Die Route `default via` kam mit dem Ack. `bound to` nennt, wann der Client verlängern will: nicht genau bei der Hälfte der 24 Stunden, denn `dhclient` legt den Zeitpunkt mit etwas Zufall davor, damit nicht alle Clients zugleich fragen.

Auf dem Router steht, was er vergeben hat:

```console
R1#show ip dhcp pool

Pool BUERO :
 Network                        : 10.10.0.0/24
 Total addresses                : 254
 Excluded addresses             : 99
 Leased addresses               : 1
 Default router                 : 10.10.0.1
 DNS server                     : -
 Domain name                    : -
 Lease                          : 1 day
```

Im Simulationsmodus sehen Sie die Nachrichten als petrolfarbene Umschläge, und im Paket-Tab zeigt jede ihre Felder. Starten Sie dort `dhclient eth0` und gehen Sie mit **F7** Schritt für Schritt: Sie sehen auch die Pings, mit denen der Server prüft.

Probieren Sie es aus: [Adressen vom Router](lab:dhcp-pool).
