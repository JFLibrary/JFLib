# DNS: names for addresses

People remember `www.schule.lab` better than `10.30.0.80`. And addresses change: if a web server moves, its name stays the same. **DNS**, the Domain Name System, translates names into addresses. It is a distributed database that anyone can ask.

## A tree of names

Names are hierarchical and get more exact from right to left. In `www.schule.lab.` the final dot stands for the root of the tree, below that comes `lab`, below that `schule`, and below that the host `www`. Someone is responsible for each part of the tree: a **zone** is the area a name server answers for itself.

## Records

A zone consists of **records**. The main ones:

| Type | Meaning | Example |
|---|---|---|
| A | name to IPv4 address | `www` → 10.30.0.80 |
| AAAA | name to IPv6 address | |
| CNAME | one name is another name for … | `start` → `www` |
| MX | who takes this domain's mail | |
| NS | who is responsible for the zone | |
| PTR | address back to a name | |

Each record has a **TTL**, a time in seconds for how long others may remember it. A long TTL takes load off the server, a short one lets changes arrive sooner. (This is not the same TTL as in the IP packet; only the name is the same.)

In the lab, the service `named` on NS1 knows the zone `schule.lab` with these records:

```text
www    A      10.30.0.80
ns     A      10.30.0.53
start  CNAME  www
```

## Who asks whom?

![PC1 asks the name server for www; the answer gives the address; the ping then goes to that address](img/dns.svg)

A small **resolver** runs on every device. A program asks it for "www.schule.lab, please", and it asks the name server it knows. On a Linux-like host, that server is listed in the file `/etc/resolv.conf`, written there by hand or by DHCP. The question usually goes by UDP to port 53 and is a single packet, as is the answer.

On the internet, the server asked is usually a **recursive resolver**: it knows hardly any answer itself, asks its way through the tree, from the root via `lab` to the server responsible for `schule.lab`, and remembers the result.

In our labs it is simpler: NS1 is **authoritative** for its zone and knows the answers itself. There is no way through the root, because this small network has no root servers. That is the simplification of this unit, and it changes nothing about what a host does.

## What can go wrong

Important for later: DNS is **a service in the network**, not a property of the wire. If a ping to the address works and one to the name does not, the wire is fine and the fault lies with the lookup. Keep this difference in mind.

The next lesson shows the tool for asking DNS: [dig](step:dig).
