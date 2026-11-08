# dig: den Namensserver befragen

Ein Ping an einen Namen sagt nur, dass etwas nicht geht. `dig` zeigt, was der Namensserver wirklich antwortet, und ist darum das Werkzeug für alle Fragen rund um DNS.

## Ohne Namensserver

Zu Beginn des Labors kennt PC1 keinen Namensserver, und die Datei ist leer:

```console
root@PC1:~# cat /etc/resolv.conf
root@PC1:~# ping -c 1 www.schule.lab
ping: www.schule.lab: Temporary failure in name resolution
```

`Temporary failure in name resolution`: Der Host hat niemanden, den er fragen könnte, und ein Paket an www ist nie losgegangen. Eintragen lässt sich der Server so:

```console
root@PC1:~# echo "nameserver 10.30.0.53" > /etc/resolv.conf
root@PC1:~# cat /etc/resolv.conf
nameserver 10.30.0.53
```

Auf realen Linux-Systemen verwaltet oft ein anderes Programm diese Datei, und was man hineinschreibt, wird später überschrieben. Im Simulator bleibt es stehen, bis `dhclient` etwas anderes einträgt.

## Eine Antwort lesen

```console
root@PC1:~# dig www.schule.lab

; dig of the JFNetworker shell: www.schule.lab
;; ->>HEADER<<- opcode: QUERY, status: NOERROR, id: 5006
;; flags: qr aa rd; QUERY: 1, ANSWER: 1, AUTHORITY: 0, ADDITIONAL: 1
;; WARNING: recursion requested but not available

;; OPT PSEUDOSECTION:
; EDNS: version: 0, flags:; udp: 1232
;; QUESTION SECTION:
;www.schule.lab.			IN	A

;; ANSWER SECTION:
www.schule.lab.		300	IN	A	10.30.0.80

;; Query time: 0 msec
;; SERVER: 10.30.0.53#53(10.30.0.53) (UDP)
;; WHEN: Thu Jan 01 00:00:09 UTC 2026
;; MSG SIZE  rcvd: 59

```

Die Antwort sieht wüst aus, aber nur wenige Zeilen sind wichtig:

- `status: NOERROR`: Der Server hat die Frage beantworten können.
- `flags: qr aa rd`: Es ist eine Antwort (`qr`), und sie kommt vom zuständigen Server selbst (`aa`, authoritative answer). `rd` heißt: Der Fragende wünscht, dass sich der Server durchfragt.
- `WARNING: recursion requested but not available` passt dazu: NS1 fragt sich nicht durch, er kennt nur seine Zone. Für unsere Zwecke ist das kein Problem.
- `ANSWER SECTION`: das Eigentliche. Der Name, die TTL von 300 Sekunden, `IN` (Internet), der Typ `A` und die Adresse.
- `SERVER`: wer geantwortet hat. Das hilft, wenn mehrere Server in Frage kommen.

Nur die Adresse liefert die Kurzform:

```console
root@PC1:~# dig www.schule.lab +short
10.30.0.80
```

## Ein Alias

`start` ist kein Host, sondern ein anderer Name für `www`:

```console
root@PC1:~# dig start.schule.lab +short
www.schule.lab.
10.30.0.80
```

Die Antwort nennt beides: erst das Ziel des Alias, dann dessen Adresse. Der Client muss nicht noch einmal fragen. In der ausführlichen Form steht derselbe Inhalt in zwei Zeilen unter `ANSWER SECTION`, mit `CNAME` und `A`.

## Wenn es den Namen nicht gibt

```console
root@PC1:~# dig nix.schule.lab

; dig of the JFNetworker shell: nix.schule.lab
;; ->>HEADER<<- opcode: QUERY, status: NXDOMAIN, id: 45506
;; flags: qr aa rd; QUERY: 1, ANSWER: 0, AUTHORITY: 1, ADDITIONAL: 1
;; WARNING: recursion requested but not available

;; OPT PSEUDOSECTION:
; EDNS: version: 0, flags:; udp: 1232
;; QUESTION SECTION:
;nix.schule.lab.			IN	A

;; AUTHORITY SECTION:
schule.lab.		300	IN	SOA	ns.schule.lab. hostmaster.schule.lab. 1 3600 600 86400 300

;; Query time: 0 msec
;; SERVER: 10.30.0.53#53(10.30.0.53) (UDP)
;; WHEN: Thu Jan 01 00:00:13 UTC 2026
;; MSG SIZE  rcvd: 93

```

`NXDOMAIN` ist eine echte Antwort: Der Server ist erreichbar und sagt, dass es den Namen nicht gibt. Unter `AUTHORITY SECTION` nennt er, wer das behauptet. Ein Ping an diesen Namen meldet dann `Name or service not known`. Antwortet dagegen gar kein Server, meldet `dig` nach einigen Versuchen eine Zeitüberschreitung. Beides sieht im Browser oft gleich aus, hat aber ganz verschiedene Ursachen.

Einen bestimmten Server fragen Sie mit `@`, auch wenn er nicht in `/etc/resolv.conf` steht: `dig @10.30.0.53 www.schule.lab`. So können Sie prüfen, ob der Server stimmt oder die Einstellung des Hosts. `nslookup www.schule.lab` ist das ältere Werkzeug mit knapperer Ausgabe, das auch unter Windows vorhanden ist.

Weiter im Labor: [Namen statt Nummern](lab:namen).
