# Das Standard-Gateway

Ein Host kann nur Geräte im eigenen Netz direkt erreichen. Für alles andere braucht er jemanden, der den Weg kennt. Dieses Gerät heißt **Standard-Gateway**: Es ist die Adresse des Routers im eigenen Netz, an den ein Host alles schickt, was nicht in sein Netz gehört. Der Host muss also nur eine Sache über die Welt wissen: das nächste Tor nach draußen.

## Was der Host tut

Erinnern Sie sich an die Rechnung mit der Maske. Liegt das Ziel außerhalb, dann:

1. nimmt der Host die Adresse des Gateways, nicht die des Ziels, und fragt per ARP nach **ihrer** MAC-Adresse;
2. schickt den Rahmen an diese MAC-Adresse, das Paket darin bleibt an das echte Ziel adressiert;
3. überlässt den Rest dem Router.

Darum muss das Gateway im eigenen Netz des Hosts liegen. Ein Gateway in einem fremden Netz könnte er selbst nicht per ARP finden. Linux lehnt einen solchen Eintrag darum gleich ab.

## Im Labor

Im Labor zu dieser Einheit sitzt PC3 in seinem eigenen Netz. Sein Router ist schon eingerichtet, und trotzdem geht nichts:

```console
root@PC3:~# ip route
10.2.0.0/24 dev eth0 proto kernel scope link src 10.2.0.10
root@PC3:~# ping -c 1 10.1.0.10
ping: connect: Network is unreachable
```

`ip route` zeigt nur das eigene Netz. Für 10.1.0.10 gibt es keinen Eintrag, also sagt PC3 sofort `Network is unreachable`. Mit dem Gateway sieht es anders aus:

```console
root@PC3:~# ip route add default via 10.2.0.1
root@PC3:~# ip route
default via 10.2.0.1 dev eth0
10.2.0.0/24 dev eth0 proto kernel scope link src 10.2.0.10
root@PC3:~# ping -c 2 10.1.0.10
PING 10.1.0.10 (10.1.0.10) 56(84) bytes of data.
64 bytes from 10.1.0.10: icmp_seq=2 ttl=63 time=0.006 ms

--- 10.1.0.10 ping statistics ---
2 packets transmitted, 1 received, 50% packet loss, time 1000ms
rtt min/avg/max/mdev = 0.006/0.006/0.006/0.000 ms
root@PC3:~# ip neigh
10.2.0.1 dev eth0 lladdr 02:4a:00:00:04:02 REACHABLE
```

`default` steht für „alles andere“. Das erste Echo geht verloren, das zweite kommt durch. Der Grund: Der Router kennt die MAC-Adresse von PC1 noch nicht. Er fragt per ARP und wirft das Paket, das die Frage ausgelöst hat, einfach weg. Das tun echte Router dieser Art auch, darum beginnt ein Ping über einen frischen Router oft mit einem Verlust, und `-c 1` wäre hier ein schlechter Test.

Sehen Sie sich auch die TTL in der Antwort an: PC1 hat mit 64 begonnen, der Router hat eins abgezogen. Und `ip neigh` zeigt nur den Router, nicht PC1, denn zu PC1 geht kein Rahmen direkt. Die MAC-Adresse von PC1 braucht PC3 nie zu kennen.

## Beide Richtungen

Für jeden Ping müssen zwei Wege stimmen. Ist nur PC3 richtig eingerichtet, kommt sein Echo vielleicht an, aber die Antwort findet nicht zurück. Darum gehört zu einer Prüfung immer die Frage: Weiß auch die Gegenseite, wie sie antworten kann? In diesem Labor ist PC1 schon vorbereitet; im echten Netz ist das eine häufige Fehlerquelle, der Kurs Fehlersuche kommt darauf zurück.

Üblicherweise bekommt ein Host das Gateway gar nicht von Hand, sondern von DHCP. Das kommt in der nächsten Einheit.

Ein Host darf übrigens mehr als das Gateway kennen. Mit `ip route add` lassen sich einzelne Netze über einen anderen Router schicken, was bei Hosts aber selten nötig ist. Router dagegen leben von ihrer Routingtabelle; ihr gilt [die nächste Lektion](step:routing-tabelle).
