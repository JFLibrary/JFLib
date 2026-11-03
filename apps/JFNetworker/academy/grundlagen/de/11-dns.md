# DNS: Namen für Adressen

Menschen merken sich `www.schule.lab` besser als `10.30.0.80`. Und Adressen ändern sich: Zieht ein Webserver um, bleibt sein Name derselbe. **DNS**, das Domain Name System, übersetzt Namen in Adressen. Es ist eine verteilte Datenbank, die jeder befragen kann.

## Ein Baum von Namen

Namen sind hierarchisch und werden von rechts nach links genauer. In `www.schule.lab.` steht der letzte Punkt für die Wurzel des Baums, darunter kommt `lab`, darunter `schule`, darunter der Host `www`. Für jeden Teil des Baums ist jemand zuständig: Eine **Zone** ist der Bereich, den ein Namensserver selbst beantwortet.

## Einträge

Eine Zone besteht aus **Records**. Die wichtigsten:

| Typ | Bedeutung | Beispiel |
|---|---|---|
| A | Name zu IPv4-Adresse | `www` → 10.30.0.80 |
| AAAA | Name zu IPv6-Adresse | |
| CNAME | ein Name ist ein anderer Name für … | `start` → `www` |
| MX | wer die Post dieser Domain annimmt | |
| NS | wer für die Zone zuständig ist | |
| PTR | Adresse zurück zu einem Namen | |

Jeder Eintrag hat eine **TTL**, also eine Zeit in Sekunden, wie lange andere ihn sich merken dürfen. Eine lange TTL entlastet den Server, eine kurze lässt Änderungen schneller ankommen. (Das ist nicht dieselbe TTL wie im IP-Paket; der Name ist nur gleich.)

Im Labor kennt der Dienst `named` auf NS1 die Zone `schule.lab` mit diesen Einträgen:

```text
www    A      10.30.0.80
ns     A      10.30.0.53
start  CNAME  www
```

## Wer fragt wen?

![PC1 fragt den Namensserver nach www; die Antwort nennt die Adresse; danach geht der Ping an diese Adresse](img/dns.svg)

Auf jedem Gerät läuft ein kleiner **Resolver**. Er bekommt von einem Programm den Wunsch „www.schule.lab, bitte“ und fragt den Namensserver, den er kennt. Auf einem Linux-artigen Host steht dieser in der Datei `/etc/resolv.conf`, hineingeschrieben von Hand oder von DHCP. Die Frage geht meist per UDP an Port 53 und besteht aus einem einzigen Paket, die Antwort auch.

Im Internet ist der befragte Server meist ein **rekursiver Resolver**: Er kennt kaum eine Antwort selbst, fragt sich durch den Baum, von der Wurzel über `lab` bis zu dem Server, der für `schule.lab` zuständig ist, und merkt sich das Ergebnis.

In unseren Laboren ist es einfacher: NS1 ist **autoritativ** für seine Zone und kennt die Antworten selbst. Einen Weg durch die Wurzel gibt es nicht, denn es gibt in diesem kleinen Netz keine Wurzelserver. Das ist die Vereinfachung dieser Einheit, und sie ändert nichts an dem, was ein Host tut.

## Was schiefgehen kann

Wichtig für später: DNS ist **ein Dienst im Netz**, keine Eigenschaft der Leitung. Geht ein Ping an die Adresse und einer an den Namen nicht, ist die Leitung in Ordnung, und der Fehler liegt bei der Auflösung. Diesen Unterschied sollten Sie sich merken.

Mit welchem Werkzeug man DNS befragt, zeigt die nächste Lektion: [dig](step:dig).
