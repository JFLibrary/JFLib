# Eingrenzen statt absuchen

Ein Weg durch ein Netz hat viele Stationen, und nicht jede lässt sich schnell prüfen. Wer vorn anfängt und sich Gerät für Gerät vorarbeitet, kommt zum Ziel, aber langsam. Es gibt drei bessere Wege.

## Den Weg halbieren

Prüfen Sie in der Mitte. Geht ein Ping bis zum mittleren Router, liegt der Fehler dahinter, und die vordere Hälfte ist erledigt. Geht er nicht, liegt der Fehler davor. Mit jedem Test halbiert sich, was noch in Frage kommt. Bei acht Stationen sind das höchstens drei Tests statt acht.

![Fünf Stationen in einer Reihe; ein Ping bis zur Mitte klappt, darum bleibt nur die hintere Hälfte in Frage](img/eingrenzen.svg)

`traceroute` macht das in einem Zug: Es zeigt, bis wohin Antworten zurückkommen.

## Vergleichen

Zwei Geräte, von denen eines geht und eines nicht, sind ein Geschenk. Was sich unterscheidet, ist verdächtig. Legen Sie die Ausgaben nebeneinander: `ip addr`, `ip route`, `cat /etc/resolv.conf`, die Ports am Switch.

Auch ein Muster bei den Betroffenen sagt etwas. Wenn PC2 und PC3 nicht gehen und PC1 schon, fragen Sie: Was haben die beiden gemeinsam? Derselbe Raum? Derselbe Switch? Oder Adressen in derselben Hälfte des Netzes?

## Von beiden Seiten

Eine Verbindung hat zwei Enden, und oft liegt der Fehler nicht dort, wo das Problem gemeldet wird. Wer klagt, sitzt am Client; die Ursache kann ebenso gut am Server liegen. Sehen Sie also auf beiden Seiten nach, was sie jeweils über die andere glauben.

## Hin und zurück

Das führt zum heimtückischsten Fehler: Ein Ping braucht **zwei** Wege. Der Hinweg kann in Ordnung sein, während der Rückweg fehlt.

![Der Weg von PC1 über R1 und R2 zum Server ist grün, der Rückweg endet an R2 mit einem roten Kreuz](img/rueckweg.svg)

So sieht das aus: Der Router R2 kennt den Server, aber nicht das Netz von PC1. Vom Standpunkt des Absenders ist nur Stille zu sehen. `traceroute` zeigt den ersten Router, und danach Sterne:

```console
root@PC1:~# traceroute -n -m 3 -w 1 10.0.2.10
traceroute to 10.0.2.10 (10.0.2.10), 3 hops max, 60 byte packets
 1  10.0.1.1  0.003 ms  0.002 ms  0.002 ms
 2  * * *
 3  * * *
```

Warum antwortet R2 nicht? Er würde gern, aber auch seine Antwort geht an PC1, und diesen Weg kennt er nicht. Darum ist das Ende der Antworten bei `traceroute` nicht unbedingt das Ende des Hinwegs.

Der Ping von R1 zum Server dagegen klappt. Damit ist die Ursache schon eingegrenzt, denn R1 fragt als Absender aus einem Netz, das R2 direkt kennt:

```console
R1>enable
R1#ping 10.0.2.10
Ctrl+Shift+6 stops the ping.
Pinging 10.0.2.10 with 5 echo requests of 100 bytes, 2 s timeout each:
!!!!!
Success rate is 100 percent (5/5), round-trip min/avg/max = 0/0/0 ms
```

Ein Router antwortet also, wenn er selbst fragt, und schweigt, wenn ein Host hinter ihm fragt. Wenn Sie das sehen, sehen Sie sich die Routen auf dem Rückweg an: `show ip route` auf R2. Ein typischer Satz in solchen Fällen ist: „Hin geht es doch!“ Das stimmt, und eben darum hilft es nicht.

Die Regel daraus: Wer Routen prüft, prüft sie für beide Richtungen. Was das im Einzelnen in den Ausgaben heißt, zeigt die [nächste Lektion](step:werkzeuge).
