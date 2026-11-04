# ARP: von der IP-Adresse zur MAC-Adresse

Wer etwas im Netz sucht, kennt in der Regel die **IP-Adresse** des Ziels; dazu kommen wir in der nächsten Einheit. Auf dem Kabel zählt aber die **MAC-Adresse**, denn nur nach der verteilt der Switch. Zwischen beiden vermittelt **ARP**, das Address Resolution Protocol.

![PC1 ruft ins ganze Netz, wer eine Adresse hat; nur PC2 antwortet, direkt an PC1](img/arp.svg)

## Frage und Antwort

Will PC1 etwas an 192.168.1.20 schicken, ohne die passende MAC-Adresse zu kennen, läuft das so ab:

1. PC1 schickt eine **ARP-Anfrage** als Broadcast: „Wer hat 192.168.1.20? Sag es 192.168.1.10.“ Die Anfrage nennt auch die MAC-Adresse von PC1. Der Switch flutet sie, also sehen sie alle Hosts im Netz.
2. Alle Hosts vergleichen die gesuchte Adresse mit ihrer eigenen. Wer nicht gemeint ist, verwirft die Anfrage.
3. PC2 ist gemeint und schickt eine **ARP-Antwort**, und zwar nur an PC1: „192.168.1.20 ist meine, hier ist meine MAC-Adresse.“ Der Switch kennt PC1 seit der Anfrage und braucht nicht mehr zu fluten.
4. PC1 trägt das Paar in seine **ARP-Tabelle** ein und schickt das, was die ganze Zeit gewartet hat.

Die Tabelle erspart die Frage beim nächsten Mal. Ein Eintrag wird nach einiger Zeit ohne Verkehr wieder vergessen; so kommt ein Host damit zurecht, dass eine Adresse später einem anderen Gerät gehört. Auf einem Linux-PC im Simulator gilt ein Eintrag bis zu einer Minute lang, auf einem Router vier Stunden. Bei realen Linux-Systemen läuft das etwas anders ab, die Größenordnung stimmt aber.

ARP bleibt im eigenen Netz: Die Anfrage ist ein Broadcast, und Broadcasts passieren keinen Router. Wie Hosts in anderen Netzen erreicht werden, zeigt die Einheit über Router.

## Selbst nachsehen

```console
root@PC1:~# ip neigh
root@PC1:~# ping -c 1 192.168.1.20
PING 192.168.1.20 (192.168.1.20) 56(84) bytes of data.
64 bytes from 192.168.1.20: icmp_seq=1 ttl=64 time=0.006 ms

--- 192.168.1.20 ping statistics ---
1 packets transmitted, 1 received, 0% packet loss, time 0ms
rtt min/avg/max/mdev = 0.006/0.006/0.006/0.000 ms
root@PC1:~# ip neigh
192.168.1.20 dev eth0 lladdr 02:4a:00:00:02:01 REACHABLE
```

Beim ersten Mal bleibt die Tabelle leer, nach dem Ping steht PC2 darin. Die Zeile nennt die Adresse, die Schnittstelle, die MAC-Adresse (`lladdr`) und den Zustand. Die Antwortzeit von wenigen Mikrosekunden ist übrigens eine Vereinfachung: Der Simulator rechnet nur die Laufzeit auf dem Kabel und das Senden. Im echten LAN sind es eher einige Zehntel Millisekunden, weil auch die Geräte Zeit brauchen.

## Im Simulationsmodus zusehen

Am besten sieht man ARP Schritt für Schritt, zum Beispiel im nächsten Labor, sobald PC2 seine Adresse hat:

1. Schalten Sie mit **F5** in den Simulationsmodus (Menü Simulation). Die Uhr steht nun.
2. Tippen Sie auf PC1 `ping -c 1 192.168.1.20`. Noch passiert nichts.
3. Gehen Sie mit **F7** einen Schritt vor. Ein bernsteinfarbener Umschlag läuft von PC1 zum Switch: die ARP-Anfrage.
4. Nächster Schritt: Der Switch gibt sie an alle anderen Ports aus. Die Liste im Bereich Ereignisse sagt zu jedem Gerät, was es damit tut.
5. Dann kommt die Antwort zurück, und danach die grünen Umschläge von ICMP: der eigentliche Ping.

Ein zweiter Ping zeigt keinen ARP-Umschlag mehr, denn die Tabelle kennt die Antwort schon. Mit **Umschalt+F5** kommen Sie zurück in die Echtzeit.

Jetzt sind Sie dran: [Zwei PCs und ein Switch](lab:erstes-netz).
