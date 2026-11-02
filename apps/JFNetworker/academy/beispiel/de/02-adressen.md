# Adressen

Damit ein Paket sein Ziel findet, braucht jeder Host eine eigene **IP-Adresse**, zum Beispiel
`10.0.0.1`. Der Zusatz `/24` nennt die Größe des Netzes: Die ersten 24 Bit der Adresse sind bei allen
Hosts dieses Netzes gleich, also hier `10.0.0`. Nur die letzte Zahl unterscheidet sie.

![Links ein Netz mit zwei Hosts, rechts ein zweites Netz mit einem Host; die beiden berühren sich nicht](img/adressen.svg)

Hosts im selben Netz erreichen einander direkt über den Switch. Ein Host in einem anderen Netz ist
ohne Router nicht erreichbar, auch wenn er am selben Switch hängt.

Auf einem Host unter Linux setzt dieser Befehl eine Adresse:

```shell
ip addr add 10.0.0.9/24 dev eth0
```

Probieren Sie es aus: Im [Labor](lab:erstes-netz) fehlt einem PC seine Adresse.
