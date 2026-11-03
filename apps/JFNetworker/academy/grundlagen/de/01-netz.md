# Was ein Netz ist

Ein Netz verbindet Geräte, damit sie einander Daten schicken können. Dabei läuft nie eine ganze Datei am Stück über die Leitung. Jede Nachricht wird in kleine Stücke zerlegt, und jedes Stück trägt die Angabe, wohin es soll und woher es kommt. So können viele Geräte dieselbe Leitung abwechselnd benutzen, und ein verlorenes Stück lässt sich einzeln neu schicken, ohne dass alles von vorn beginnt.

![Zwei PCs und ein Server, jeder mit einem eigenen Kabel an einem Switch in der Mitte](img/stern.svg)

## Die Beteiligten

Ein **Host** ist ein Gerät, das Daten sendet und empfängt, um selbst etwas damit anzufangen: ein PC, ein Laptop, ein Server, ein Drucker. Bei Hosts beginnt und endet jede Nachricht.

Ein **Switch** ist der Verteiler in der Mitte. Er hat viele Anschlüsse, die **Ports**, und reicht das, was an einem Port ankommt, an einem anderen weiter. Für sich selbst will er nichts von den Daten.

Ein **Kabel** verbindet genau zwei Ports. In Büros ist es meist Kupfer mit RJ45-Steckern, über größere Strecken Glasfaser.

## Warum ein Switch?

Man könnte jeden Host direkt mit jedem anderen verkabeln. Bei vier Geräten wären das schon sechs Kabel, bei zwanzig Geräten 190, und jeder PC bräuchte 19 Anschlüsse. Mit einem Switch braucht jeder Host genau ein Kabel. Darum sehen Netze in Gebäuden fast immer so aus wie im Bild: ein Stern, und mehrere Sterne sind über ihre Switches verbunden.

## Im Simulator

Ein Labor öffnet ein Netz auf der Zeichenfläche. Die Lämpchen an den Kabelenden zeigen, ob eine Verbindung steht. Ein Doppelklick auf ein Gerät öffnet seine Konsole, und dort tippen Sie Befehle wie auf einem echten Gerät. PCs und Server haben eine Linux-artige Befehlszeile, Router und verwaltete Switches eine IOS-artige. So fragen Sie einen PC, welche Adressen er hat:

```console
root@PC1:~# ip -br addr
lo               UNKNOWN        127.0.0.1/8
eth0             UP             192.168.1.10/24
```

`lo` ist die Schnittstelle, über die ein Gerät mit sich selbst spricht. `eth0` ist der Anschluss für das Kabel, `UP` heißt: Es steckt ein Kabel, und die Gegenseite lebt.

Der Simulator vereinfacht manches, und wir sagen es jeweils dazu. Hier die erste Vereinfachung: Ein Port ist im Simulator sofort bereit, sobald das Kabel steckt. Ein echter Switch braucht dafür ein bis zwei Sekunden, mit eingeschaltetem Spanning Tree auch eine halbe Minute.

Als Nächstes sehen wir uns an, was auf dem Kabel unterwegs ist: [Rahmen und MAC-Adressen](step:rahmen).
