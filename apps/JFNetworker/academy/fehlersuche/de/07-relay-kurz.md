# Kurz erinnert: DHCP über einen Router

Aus dem Kurs Grundlagen wissen Sie: Ein Client beginnt mit einem Broadcast, weil er noch keine Adresse und keinen Server kennt. Und Broadcasts passieren keinen Router. Steht der DHCP-Server im selben Netz, ist das kein Problem. In größeren Netzen steht aber meist ein zentraler Server irgendwo im Haus, und die Clients sitzen in vielen anderen Netzen.

## Das Relay

Die Lösung heißt **DHCP-Relay**. Der Router an der Grenze hört die Broadcasts der Clients und reicht sie gezielt an den Server weiter.

![Ein PC schickt einen Broadcast zum Router; der schickt ihn als gezielte Nachricht an den DHCP-Server weiter](img/relay.svg)

Auf einem IOS-artigen Router genügt dafür eine Zeile an der Schnittstelle zu den Clients:

`ip helper-address 10.0.3.10`

Das heißt: Was an dieser Schnittstelle als DHCP-Broadcast ankommt, geht als gezielte Anfrage an 10.0.3.10.

Zwei Dinge sind dabei schön gelöst:

- Der Router trägt die Adresse **seiner eigenen Schnittstelle** in die Anfrage ein (das Feld heißt `giaddr`). Daran erkennt der Server, aus welchem Netz der Client kommt und aus welchem Pool er die Adresse nehmen soll.
- Die Antwort schickt der Server an diese Adresse zurück. Der Router gibt sie dann im Netz des Clients weiter.

Der Server muss also einen Pool für jedes dieser Netze haben, und er muss den Weg zum Router zurück kennen.

## Was das für die Fehlersuche heißt

Fehlt die Zeile, sieht der Client genau das, was bei einem ausgefallenen Server auch zu sehen wäre:

```console
root@PC2:~# dhclient -v eth0
DHCPDISCOVER on eth0 to 255.255.255.255 port 67 interval 9 (xid=0x56b2032)
DHCPDISCOVER on eth0 to 255.255.255.255 port 67 interval 16 (xid=0x56b2032)
DHCPDISCOVER on eth0 to 255.255.255.255 port 67 interval 17 (xid=0x56b2032)
DHCPDISCOVER on eth0 to 255.255.255.255 port 67 interval 9 (xid=0x56b2032)
No DHCPOFFERS received.
No working leases in persistent database - sleeping.
```

Ein paar Versuche, dann nichts. Es lohnt sich, zwei Fragen zu trennen:

- **Kommt die Anfrage beim Server an?** Das kann der Server nicht von sich aus melden. Hier hilft die Frage, was für andere Netze gilt: Wenn Clients anderswo versorgt werden, lebt der Server.
- **Ist der Router vorbereitet?** `show running-config` auf dem Router zeigt die Zeile je Schnittstelle, und ein Vergleich mit einer Schnittstelle, an der es geht, ist schnell gemacht.

`show running-config` listet die ganze Konfiguration. Bei einem Router mit vielen Schnittstellen lohnt es sich, mit `show ip interface brief` anzufangen und dann gezielt zu lesen.

Ein Hinweis noch: Ein Client, der keine Antwort bekommen hat, wartet. Reale Clients versuchen es nach einigen Minuten wieder; das tut auch der Simulator. Ungeduldige stoßen ihn an: `dhclient eth0`.

Weiter zur Fehlersuche: [Keine Adresse im zweiten Netz](step:relay).
