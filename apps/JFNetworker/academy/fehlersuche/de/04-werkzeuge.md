# Was die Ausgaben verraten

Dieselbe Frage kann ganz verschiedene Antworten bekommen, und jede davon verrät, in welcher Schicht es hakt. Es lohnt sich, die Fehlermeldungen wörtlich zu lesen.

## Was ein Ping sagen kann

| Antwort | Bedeutung |
|---|---|
| `64 bytes from …` | Hin- und Rückweg gehen |
| `connect: Network is unreachable` | Der Host selbst kennt keinen Weg: keine Route und kein Gateway. Es ging kein Paket los |
| `From` *eigene Adresse* `… Destination Host Unreachable` | Das Ziel oder Gateway liegt im eigenen Netz, aber die ARP-Frage blieb ohne Antwort |
| `From` *Router* `… Destination Net Unreachable` | Bis zu diesem Router ging es, und er kennt keinen Weg ins Zielnetz |
| nichts, nur die Statistik am Ende | Etwas ist unterwegs verloren gegangen, oder die Antwort findet nicht zurück |
| `Temporary failure in name resolution` | Es antwortet kein Namensserver |
| `Name or service not known` | Der Namensserver antwortet: Den Namen gibt es nicht |

Eine einzelne Meldung sagt oft schon, wo Sie weitersuchen sollten. Ein Beispiel aus dem Netz mit dem gezogenen Kabel:

```console
root@PC1:~# ping -c 1 10.0.0.10
PING 10.0.0.10 (10.0.0.10) 56(84) bytes of data.
From 10.0.0.11 icmp_seq=1 Destination Host Unreachable

--- 10.0.0.10 ping statistics ---
1 packets transmitted, 0 received, +1 errors, 100% packet loss, time 0ms
root@PC1:~# ip neigh
```

`Destination Host Unreachable`, und der Absender ist PC1 selbst: Er hat gefragt und keine Antwort bekommen. `ip neigh` bleibt darum leer. Ein echtes Linux zeigt hier für eine Weile eine Zeile mit `FAILED`; der Simulator lässt sie weg.

## Fragen Sie das Gerät selbst

Auf der Seite des Servers sieht das so aus. Die Zeile nennt den Zustand und, in spitzen Klammern, was die Schnittstelle meldet:

```console
root@SRV1:~# ip -br link
lo               UNKNOWN        00:00:00:00:00:00 <LOOPBACK,UP,LOWER_UP>
eth0             DOWN           02:4a:00:00:04:01 <NO-CARRIER,BROADCAST,MULTICAST,UP>
eth1             DOWN           02:4a:00:00:04:02 <NO-CARRIER,BROADCAST,MULTICAST,UP>
eth2             DOWN           02:4a:00:00:04:03 <NO-CARRIER,BROADCAST,MULTICAST,UP>
```

`DOWN` und `NO-CARRIER` bei `eth0`: Niemand ist am anderen Ende des Kabels. An `eth1` und `eth2`, den weiteren Anschlüssen des Servers, hing nie eines.

## Der Port am Switch

Der Zustand eines Ports kann auf zwei Arten schlecht sein, und nur eine davon hat mit dem Kabel zu tun. Im selben Netz sagt der Switch zum Port von PC1 und zu dem des Servers:

```console
SW1>enable
SW1#show interfaces GigabitEthernet1/0/1
GigabitEthernet1/0/1 is up, line protocol is up (connected)
…
SW1#show interfaces GigabitEthernet1/0/3
GigabitEthernet1/0/3 is down, line protocol is down (notconnect)
…
```

Die erste Zeile jeder Antwort ist die wichtige: `up` und `up` ist in Ordnung, ein schlichtes `down` heißt: kein Partner am anderen Ende. Bei einem abgeschalteten Port stünde dort `administratively down`, wie in der zweiten Fehlersuche. Der Rest der Ausgabe enthält Zähler. Steigen dort die Fehler, ist das Kabel oder die Gegenstelle verdächtig. Das `…` steht für die vielen Zeilen, die hier nichts zur Sache tun. Im Fenster hält die IOS-artige Konsole lange Ausgaben nach einer Seite mit `--More--` an: Die Leertaste blättert weiter, bis dahin nimmt sie keine neuen Befehle an, und `terminal length 0` schaltet das Blättern ab.

## Was die Adresse sagt und was nicht

`ip addr` zeigt, was ein Host glaubt, nicht, was stimmt. Zwei Hosts mit derselben Adresse sagen beide zufrieden, sie seien es. Einen solchen Fall verrät erst die MAC-Adresse in `ip neigh`: Sie gehört nicht dem Gerät, das Sie erwarten. Die Tabelle des Switches führt dann zum richtigen Port.

## Nicht nur ob, sondern wie gut

Ein Netz kann funktionieren und trotzdem schlecht sein. Ein einzelner Ping sagt dazu fast nichts. Viele Pings zeigen Verluste:

`ping -c 30 10.0.2.80`

Eine Zeile `10% packet loss` ist in einem LAN ein ernster Befund, auch wenn alles irgendwie läuft: TCP schickt Verlorenes nach, und jede Lücke kostet Wartezeit. `curl -s -o /dev/null -w "%{time_total}\n" http://10.0.2.80/` zeigt, wie lange ein Abruf dauert. Im Simulator zeigt auch der Bereich Metriken die Wartezeiten eines Dienstes, sobald er Anfragen bedient.

Bleibt die Frage, wie man bei all dem den Überblick behält: [Eine Änderung, ein Test, eine Notiz](step:notizen).
