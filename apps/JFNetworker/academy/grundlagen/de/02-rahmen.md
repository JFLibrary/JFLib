# Rahmen und MAC-Adressen

Die Stücke, die im lokalen Netz über das Kabel laufen, heißen **Rahmen** (englisch *frames*). Ein Rahmen ist wie ein Umschlag: Außen stehen Empfänger und Absender, innen liegt, was transportiert wird.

![Ein Rahmen als Streifen: Ziel-MAC, Quell-MAC, Typ, Daten und Prüfsumme](img/rahmen-de.svg)

## Was in einem Rahmen steht

- **Ziel-MAC** und **Quell-MAC**: an wen und von wem, je 6 Bytes.
- **Typ**: was innen liegt, zum Beispiel IPv4 oder ARP.
- **Daten**: meist bis zu 1500 Bytes.
- **Prüfsumme**: Wer den Rahmen empfängt, rechnet nach und wirft ihn weg, wenn ein Bit auf dem Weg gekippt ist. Der Absender erfährt davon nichts.

Eine **MAC-Adresse** gehört zu einer Netzwerkkarte, nicht zu einem Netz. Sie hat 48 Bits und wird in Hexadezimal geschrieben, etwa `02:4a:00:00:01:01`. Der Hersteller hat sie der Karte mitgegeben; sie sagt also, *wer* ein Gerät ist, aber nicht, *wo* es steht. Die Geräte im Simulator bekommen ihre MAC-Adressen beim Start nach einem festen Muster, darum beginnen sie alle mit `02:4a`. Reale Karten tragen dort die Kennung ihres Herstellers.

Es gibt eine besondere Zieladresse: `ff:ff:ff:ff:ff:ff`, den **Broadcast**. So ein Rahmen ist an alle im Netz gerichtet.

## Wie ein Switch lernt

Ein Switch wird nicht eingerichtet, damit er Rahmen richtig verteilt. Er lernt es beim Zuhören, mit zwei einfachen Regeln:

1. Kommt ein Rahmen an, merkt er sich: Die **Quell-MAC** sitzt hinter diesem Port.
2. Dann schaut er nach der **Ziel-MAC**. Kennt er sie, gibt er den Rahmen nur an diesem einen Port aus. Kennt er sie nicht, oder ist es ein Broadcast, gibt er ihn an allen anderen Ports aus. Das nennt man **fluten**.

![Ein Switch mit einer Tabelle: Ein Rahmen an eine unbekannte Adresse geht an alle Ports, die Antwort geht nur noch an einen](img/switch-lernt.svg)

So füllt sich die Tabelle von selbst, sobald Geräte senden. Ein Eintrag verfällt nach fünf Minuten ohne Verkehr, und er verschwindet sofort, wenn die Verbindung am Port abreißt. So kommt der Switch damit zurecht, dass ein Gerät umgesteckt wird.

## Selbst nachsehen

Vor dem ersten Rahmen ist die Tabelle von SW1 leer. Nach einem Ping stehen beide PCs darin, jeder an seinem Port:

```console
SW1>enable
SW1#show mac address-table
          Mac Address Table
-------------------------------------------

Vlan    Mac Address       Type        Ports
----    -----------       --------    -----
Total Mac Addresses for this criterion: 0
root@PC1:~# ping -c 1 192.168.1.20
PING 192.168.1.20 (192.168.1.20) 56(84) bytes of data.
64 bytes from 192.168.1.20: icmp_seq=1 ttl=64 time=0.006 ms

--- 192.168.1.20 ping statistics ---
1 packets transmitted, 1 received, 0% packet loss, time 0ms
rtt min/avg/max/mdev = 0.006/0.006/0.006/0.000 ms
SW1#show mac address-table
          Mac Address Table
-------------------------------------------

Vlan    Mac Address       Type        Ports
----    -----------       --------    -----
   1    024a.0000.0101    DYNAMIC     Gi1/0/1
   1    024a.0000.0201    DYNAMIC     Gi1/0/2
Total Mac Addresses for this criterion: 2
```

`enable` hebt die IOS-artige Konsole auf die Stufe, von der aus alles zu sehen ist; der Prompt wechselt von `>` zu `#`. Die Spalte `Type` sagt `DYNAMIC`: gelernt, nicht eingetragen.

Woher PC1 aber die MAC-Adresse von PC2 kennt, bevor er seinen ersten Rahmen an ihn schickt, ist noch offen. Das klärt die nächste Lektion: [ARP](step:arp).
