# dig: asking the name server

A ping to a name only says that something does not work. `dig` shows what the name server really answers, and so it is the tool for all questions around DNS.

## Without a name server

At the start of the lab, PC1 knows no name server, and the file is empty:

```console
root@PC1:~# cat /etc/resolv.conf
root@PC1:~# ping -c 1 www.schule.lab
ping: www.schule.lab: Temporary failure in name resolution
```

`Temporary failure in name resolution`: the host has nobody to ask, and no packet to www has ever left. The server can be entered like this:

```console
root@PC1:~# echo "nameserver 10.30.0.53" > /etc/resolv.conf
root@PC1:~# cat /etc/resolv.conf
nameserver 10.30.0.53
```

On real Linux systems, another program often manages this file, and what you write there is overwritten later. In the simulator it stays until `dhclient` enters something else.

## Reading an answer

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

The answer looks wild, but only a few lines matter:

- `status: NOERROR`: the server was able to answer the question.
- `flags: qr aa rd`: this is an answer (`qr`), and it comes from the responsible server itself (`aa`, authoritative answer). `rd` means that the one asking wants the server to ask its way through.
- `WARNING: recursion requested but not available` fits that: NS1 does not ask around, it only knows its zone. For our purposes this is no problem.
- `ANSWER SECTION`: the real thing. The name, the TTL of 300 seconds, `IN` (internet), the type `A` and the address.
- `SERVER`: who answered. That helps when several servers could be involved.

The short form gives just the address:

```console
root@PC1:~# dig www.schule.lab +short
10.30.0.80
```

## An alias

`start` is no host, but another name for `www`:

```console
root@PC1:~# dig start.schule.lab +short
www.schule.lab.
10.30.0.80
```

The answer gives both: first the alias's target, then its address. The client does not have to ask again. In the long form, the same content stands in two lines under `ANSWER SECTION`, with `CNAME` and `A`.

## When the name does not exist

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

`NXDOMAIN` is a real answer: the server is reachable and says that the name does not exist. Under `AUTHORITY SECTION` it says who claims this. A ping to that name then reports `Name or service not known`. If no server answers at all, on the other hand, `dig` reports a timeout after a few tries. Both often look the same in a browser, but they have quite different causes.

To ask a particular server, use `@`, even if it is not in `/etc/resolv.conf`: `dig @10.30.0.53 www.schule.lab`. That lets you check whether the server is right or the host's setting is. `nslookup www.schule.lab` is the older tool with terser output, and it is available on Windows too.

On to the lab: [Names instead of numbers](lab:namen).
