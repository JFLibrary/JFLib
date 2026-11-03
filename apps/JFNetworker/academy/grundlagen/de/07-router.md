# Warum zwei Netze einen Router brauchen

Ein Switch verbindet die Geräte **eines** Netzes. Er liest MAC-Adressen, und die helfen nur, solange alle im selben Netz stehen, denn nach der MAC-Adresse eines fremden Hosts lässt sich per ARP gar nicht fragen: Die Anfrage bleibt im eigenen Netz.

Warum teilt man überhaupt in mehrere Netze? Weil jeder Broadcast jedes Gerät eines Netzes erreicht und alle ihn bearbeiten müssen. Das geht bei einigen hundert Geräten gut und bei hunderttausend nicht mehr. Außerdem will man Bereiche voneinander trennen, etwa die Buchhaltung von den Gästen, und Netze an verschiedenen Orten verbinden. Das Internet ist am Ende nichts anderes als sehr viele Netze und Router dazwischen.

![Zwei Netze, jedes mit Switch und PCs, und dazwischen ein Router mit je einer Schnittstelle und Adresse in beiden Netzen](img/router.svg)

## Was ein Router tut

Ein **Router** steht mit je einer Schnittstelle in jedem seiner Netze und hat dort jeweils eine eigene Adresse. Kommt ein Paket bei ihm an, entscheidet er anhand der **Ziel-IP-Adresse**, wohin es weitergeht.

Das Wichtigste dabei: Der Rahmen endet am Router, das Paket nicht.

1. PC1 schickt das Paket in einem Rahmen an die MAC-Adresse des Routers. Die IP-Adresse darin nennt das echte Ziel.
2. Der Router packt das Paket aus, sucht das nächste Netz und zieht von der **TTL** im Paket eins ab. Erreicht sie null, verwirft er das Paket; so kreist nichts ewig.
3. Er packt das Paket in einen neuen Rahmen mit neuen MAC-Adressen und schickt ihn ab.

Die MAC-Adressen wechseln also auf jedem Teilstück, die IP-Adressen bleiben von Anfang bis Ende gleich. Im Simulationsmodus sehen Sie das im Paket-Tab: Vergleichen Sie denselben Ping vor und hinter dem Router.

## Einen Router einrichten

Eine Schnittstelle eines Routers ist am Anfang abgeschaltet und hat keine Adresse. Das steht so im Labor zu dieser Einheit. Der Überblick zeigt es:

```console
R1>enable
R1#show ip interface brief
Interface              IP-Address      OK? Method Status                Protocol
GigabitEthernet0/0/0   10.1.0.1        YES NVRAM  up                    up
GigabitEthernet0/0/1   unassigned      YES unset  administratively down down
GigabitEthernet0/0/2   unassigned      YES unset  administratively down down
GigabitEthernet0/0/3   unassigned      YES unset  administratively down down
TenGigabitEthernet0/1/0 unassigned      YES unset  administratively down down
TenGigabitEthernet0/1/1 unassigned      YES unset  administratively down down
Serial0/2/0            unassigned      YES unset  administratively down down
Serial0/2/1            unassigned      YES unset  administratively down down
```

`administratively down` heißt: Jemand hat sie abgeschaltet, oder niemand hat sie je eingeschaltet. Die Spalte `Protocol` würde auch bei eingeschalteter Schnittstelle `down` sagen, solange kein Kabel die Gegenseite meldet.

Die IOS-artige Befehlszeile hat Stufen, und der Prompt zeigt die aktuelle. Von `R1#` geht es mit `configure terminal` in die Konfiguration, von dort mit `interface` zu einer Schnittstelle:

```console
R1#configure terminal
Configuration mode: one command per line; end or Ctrl+Z leaves it.
R1(config)#interface GigabitEthernet0/0/1
R1(config-if)#ip address 10.2.0.1 255.255.255.0
R1(config-if)#no shutdown
*Mar  1 00:00:09.000: %LINK-3-UPDOWN: GigabitEthernet0/0/1 is up
*Mar  1 00:00:09.000: %LINEPROTO-5-UPDOWN: line protocol on GigabitEthernet0/0/1 is up
R1(config-if)#end
*Mar  1 00:00:10.000: %SYS-5-CONFIG_I: configuration changed at the console
R1#show ip interface brief
Interface              IP-Address      OK? Method Status                Protocol
GigabitEthernet0/0/0   10.1.0.1        YES NVRAM  up                    up
GigabitEthernet0/0/1   10.2.0.1        YES manual up                    up
GigabitEthernet0/0/2   unassigned      YES unset  administratively down down
GigabitEthernet0/0/3   unassigned      YES unset  administratively down down
TenGigabitEthernet0/1/0 unassigned      YES unset  administratively down down
TenGigabitEthernet0/1/1 unassigned      YES unset  administratively down down
Serial0/2/0            unassigned      YES unset  administratively down down
Serial0/2/1            unassigned      YES unset  administratively down down
```

Die Zeilen mit `%LINK` und `%SYS` sind Meldungen des Geräts, die sich zwischen die Befehle mischen. Die Uhrzeit davor hängt davon ab, wann Sie tippen; sie zählt ab dem Start, denn die Uhr des Routers wurde nie gestellt. Bei echten Geräten erscheint die Meldung über die Schnittstelle einige Sekunden nach dem Einschalten, im Simulator sofort.

Was ein Router mit einem Paket tut, weiß er aus seiner Routingtabelle. Bevor wir sie lesen, sehen wir uns die andere Seite an: Woher weiß ein Host, dass er den Router fragen soll? Das ist das [Standard-Gateway](step:gateway).
