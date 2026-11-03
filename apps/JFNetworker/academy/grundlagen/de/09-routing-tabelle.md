# Die Routingtabelle und traceroute

Für jedes Paket stellt ein Router dieselbe Frage: Welcher Eintrag meiner Tabelle passt zur Ziel-Adresse? Die **Routingtabelle** ist eine Liste von Netzen mit dem jeweils nächsten Schritt. Hosts haben auch eine, sie besteht nur meist aus zwei Zeilen.

```console
R1#show ip route
Codes: C - connected network, L - address of this device, S - static route,
       * - default route

Gateway of last resort is not set

      10.0.0.0/8 is variably subnetted, 4 subnets, 2 masks
C        10.1.0.0/24 is directly connected, GigabitEthernet0/0/0
L        10.1.0.1/32 is directly connected, GigabitEthernet0/0/0
C        10.2.0.0/24 is directly connected, GigabitEthernet0/0/1
L        10.2.0.1/32 is directly connected, GigabitEthernet0/0/1
```

## Woher die Einträge kommen

- **C** (connected): Das Netz liegt an einer eigenen Schnittstelle. Der Eintrag kommt von selbst, sobald die Schnittstelle eine Adresse hat und läuft.
- **L** (local): die eigene Adresse der Schnittstelle, als einzelner Host mit /32.
- **S** (static): Jemand hat den Eintrag von Hand gesetzt, zum Beispiel so: `ip route 10.9.0.0 255.255.255.0 10.2.0.10`. Das heißt: Netz 10.9.0.0/24 liegt hinter dem Router 10.2.0.10.
- `*` bei einem Eintrag bedeutet: Das ist die **Standardroute**, das Gegenstück zum Gateway eines Hosts. Sie trägt das Netz 0.0.0.0/0.

Dazu kommen Routen aus Routing-Protokollen wie OSPF, mit denen Router einander ihre Netze mitteilen. Das ist Stoff eines späteren Kurses.

## Welcher Eintrag gewinnt?

Es können mehrere passen. Dann gewinnt der **längste Präfix**, also der genaueste Eintrag:

```text
Ziel 10.9.0.77
  0.0.0.0/0     passt   (0 Bits)
  10.0.0.0/8    passt   (8 Bits)
  10.9.0.0/24   passt   (24 Bits)  gewinnt
  10.9.1.0/24   passt nicht
```

Passt gar nichts, verwirft der Router das Paket und schickt dem Absender eine ICMP-Meldung zurück: `Destination Net Unreachable`, und zwar **mit der Adresse des Routers als Absender**. So eine Meldung sagt Ihnen, wie weit das Paket kam.

## Der Weg, Schritt für Schritt: traceroute

`traceroute` macht den Weg sichtbar und benutzt dazu die TTL. Es schickt Pakete mit TTL 1, dann 2, dann 3 und so weiter. Ein Router, der die TTL auf null bringt, verwirft das Paket und meldet das zurück. So verrät sich jeder Router auf dem Weg, einer nach dem anderen; am Ende antwortet das Ziel selbst.

![Drei Pakete mit TTL 1, 2 und 3: das erste endet am ersten Router, das zweite am zweiten, das dritte erreicht das Ziel](img/traceroute.svg)

```console
root@PC1:~# traceroute -n 10.2.0.10
traceroute to 10.2.0.10 (10.2.0.10), 30 hops max, 60 byte packets
 1  10.1.0.1  0.006 ms  0.003 ms  0.003 ms
 2  * 10.2.0.10  0.006 ms  0.006 ms
```

Jede Zeile ist ein Schritt, und die drei Zeiten stammen von drei Paketen für diesen Schritt. Ein `*` statt einer Zeit heißt: Dieses Paket blieb ohne Antwort. Das passiert oft nur beim allerersten Paket eines Weges, wenn ein Router erst per ARP suchen muss. Stehen in einer Zeile nur noch Sterne, ist dort meist das Ende: Der Router danach antwortet nicht, oder die Antwort findet nicht zurück. Das ist für die Fehlersuche Gold wert.

`-n` lässt die Namensauflösung weg. Ohne `-n` versucht `traceroute`, die Adressen in Namen zu übersetzen; das kann ohne Namensserver dauern.

Echte `traceroute`-Programme schicken UDP oder ICMP, das hängt vom System ab, und manche Router im Internet antworten absichtlich gar nicht. Sterne allein sind dort noch kein Fehler.

Jetzt richten Sie selbst einen Router ein: [Zwei Netze, ein Router](lab:router-gateway).
