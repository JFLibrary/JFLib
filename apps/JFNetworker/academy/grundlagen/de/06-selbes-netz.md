# Wer ist im selben Netz?

Bevor ein Host etwas sendet, entscheidet er: Liegt das Ziel in meinem eigenen Netz oder nicht? Von der Antwort hängt ab, was er als Nächstes tut. Das ist die wichtigste Regel dieser Einheit, und viele Fehler in echten Netzen lassen sich mit ihr erklären.

![Ein Entscheidungsbild: Ziel und eigene Adresse werden mit der eigenen Maske verglichen; gleich heißt direkt per ARP, verschieden heißt über das Gateway](img/selbes-netz-de.svg)

## Die Rechnung

Der Host nimmt **seine eigene Maske** und wendet sie auf zwei Adressen an: auf seine eigene und auf die des Ziels. Kommt beide Male dasselbe Netz heraus, liegt das Ziel im eigenen Netz. Dann fragt er per ARP nach dessen MAC-Adresse und schickt den Rahmen direkt.

Kommt etwas anderes heraus, liegt das Ziel woanders. Dann schickt er den Rahmen an sein Gateway, oder er gibt auf, wenn er keins hat. Das Gateway kommt in der nächsten Einheit dran.

Ein Beispiel mit der Maske /24, in dem nur die ersten drei Zahlen zählen:

```text
ich   192.168.10.10 /24   Netz 192.168.10.0
Ziel  192.168.10.130      Netz 192.168.10.0   gleich: direkt
Ziel  192.168.11.5        Netz 192.168.11.0   anders: Gateway
```

Und mit /25, wo die Grenze durch die letzte Zahl läuft:

```text
ich   192.168.10.130 /25  Netz 192.168.10.128
Ziel  192.168.10.140      Netz 192.168.10.128  gleich: direkt
Ziel  192.168.10.10       Netz 192.168.10.0    anders: Gateway
```

## Jeder rechnet für sich

Niemand prüft, ob die Masken zweier Hosts zusammenpassen. Jeder rechnet mit seiner eigenen, und daraus entstehen seltsame Fehler. Im nächsten Labor sind PC1 und PC3 richtig eingerichtet (/24), PC2 hat /25:

```console
root@PC2:~# ip route
192.168.10.128/25 dev eth0 proto kernel scope link src 192.168.10.130
root@PC2:~# ping -c 1 192.168.10.140
PING 192.168.10.140 (192.168.10.140) 56(84) bytes of data.
64 bytes from 192.168.10.140: icmp_seq=1 ttl=64 time=0.006 ms

--- 192.168.10.140 ping statistics ---
1 packets transmitted, 1 received, 0% packet loss, time 0ms
rtt min/avg/max/mdev = 0.006/0.006/0.006/0.000 ms
root@PC2:~# ping -c 1 192.168.10.10
ping: connect: Network is unreachable
```

PC2 erreicht PC3 in seiner Hälfte. Bei PC1 sagt er sofort `Network is unreachable`: Für ihn liegt das Ziel außerhalb, und ein Gateway hat er nicht. Es ging kein einziges Paket auf die Leitung. Lesen Sie die erste Zeile von `ip route`: Das ist alles, was PC2 kennt.

In der anderen Richtung sieht es anders aus:

```console
root@PC1:~# ping -c 1 192.168.10.130
PING 192.168.10.130 (192.168.10.130) 56(84) bytes of data.

--- 192.168.10.130 ping statistics ---
1 packets transmitted, 0 received, 100% packet loss, time 0ms
```

PC1 rechnet mit /24: gleiches Netz, also direkt. Sein Echo kommt bei PC2 an, und PC2 will antworten, findet aber keinen Weg zurück. Darum wartet PC1 und bekommt nichts, ohne jede Fehlermeldung. Ein Fehler kann also je nach Richtung anders aussehen, und das ist ein Hinweis, dem sich zu folgen lohnt.

`Destination Host Unreachable` wäre noch etwas anderes: Das Ziel liegt im eigenen Netz, aber auf die ARP-Anfrage antwortet niemand. Etwa weil das Gerät aus ist oder gar nicht existiert.

Jetzt können Sie es selbst reparieren: [Eine Maske zu eng](lab:falsche-maske).
