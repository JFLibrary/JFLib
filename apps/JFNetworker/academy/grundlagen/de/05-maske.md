# Maske und Präfix

Die **Maske** sagt, wo in einer Adresse der Netzteil aufhört und der Hostteil beginnt. Sie ist ebenfalls 32 Bits lang: von links Einsen für den Netzteil, dann Nullen für den Hostteil, nie durcheinander. Weil so nur die Zahl der Einsen zählt, schreibt man sie oft kürzer als **Präfix**: `/24` heißt „24 Einsen“.

```text
Adresse  192.168.1.20    11000000 10101000 00000001 00010100
Maske    255.255.255.0   11111111 11111111 11111111 00000000
Netz     192.168.1.0     11000000 10101000 00000001 00000000
```

Das Netz erhält man, indem man den Hostteil auf null setzt. Bit für Bit: wo die Maske eine 1 hat, bleibt das Bit der Adresse stehen; wo sie eine 0 hat, wird es 0. In der Rechnerei heißt das UND-Verknüpfung.

## Wie groß ist ein Netz?

Die Nullen der Maske bestimmen die Größe: Mit *n* Bits für den Hostteil gibt es 2 hoch *n* Adressen. Zwei davon sind in jedem Netz vergeben:

- Alle Hostbits 0: die **Netzadresse**, der Name des Netzes.
- Alle Hostbits 1: die **Broadcast-Adresse**, die alle im Netz meint.

| Präfix | Maske | Adressen | für Hosts |
|---|---|---|---|
| /16 | 255.255.0.0 | 65536 | 65534 |
| /24 | 255.255.255.0 | 256 | 254 |
| /25 | 255.255.255.128 | 128 | 126 |
| /26 | 255.255.255.192 | 64 | 62 |
| /27 | 255.255.255.224 | 32 | 30 |
| /30 | 255.255.255.252 | 4 | 2 |

Ein /30 taugt für die Verbindung zwischen zwei Routern, denn dort braucht es genau zwei Adressen.

## Ein Netz, das nicht auf einer Zahlengrenze endet

Bei /24 fällt die Grenze zwischen zwei Zahlen und ist leicht zu sehen. Bei /25 geht sie mitten durch die letzte Zahl. Das erste der 8 Bits gehört noch zum Netz, und dieses Bit ist 128 wert. Darum zerfällt 192.168.10.0/24 in zwei Hälften:

```text
192.168.10.0/25    Adressen .0  bis .127    Hosts .1   bis .126
192.168.10.128/25  Adressen .128 bis .255   Hosts .129 bis .254
```

Das steht auch in dem, was ein Host über sich sagt. Hier PC2 aus dem nächsten Labor:

```console
root@PC2:~# ip addr show eth0
2: eth0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc fq_codel state UP group default qlen 1000
    link/ether 02:4a:00:00:02:01 brd ff:ff:ff:ff:ff:ff
    inet 192.168.10.130/25 brd 192.168.10.255 scope global eth0
       valid_lft forever preferred_lft forever
```

Die Zeile `inet` nennt Adresse und Präfix, `brd` die Broadcast-Adresse: `192.168.10.130/25`, also die obere Hälfte, und `192.168.10.255` als ihr Ende. Eine IOS-artige Konsole schreibt die Maske beim Einrichten aus (`ip address 10.2.0.1 255.255.255.0`), eine Linux-artige benutzt das Präfix (`ip addr add 10.2.0.1/24 dev eth0`). Gemeint ist dasselbe.

Was aus der Maske folgt, nämlich wen ein Host überhaupt für erreichbar hält, zeigt die nächste Lektion: [Wer ist im selben Netz?](step:selbes-netz)
