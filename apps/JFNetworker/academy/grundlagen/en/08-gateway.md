# The default gateway

A host can reach only devices in its own network directly. For everything else it needs someone who knows the way. That device is called the **default gateway**: it is the address of the router in the host's own network, to which the host sends everything that does not belong to its network. So a host needs to know just one thing about the world: the next door to the outside.

## What the host does

Remember the calculation with the mask. If the destination lies outside, the host

1. takes the gateway's address, not the destination's, and asks by ARP for **its** MAC address;
2. sends the frame to this MAC address, while the packet inside stays addressed to the real destination;
3. leaves the rest to the router.

That is why the gateway has to lie in the host's own network. The host could not find a gateway in a foreign network by ARP. So Linux refuses such an entry right away.

## In the lab

In the lab of this unit, PC3 sits in its own network. Its router is already set up, and still nothing works:

```console
root@PC3:~# ip route
10.2.0.0/24 dev eth0 proto kernel scope link src 10.2.0.10
root@PC3:~# ping -c 1 10.1.0.10
ping: connect: Network is unreachable
```

`ip route` shows only the own network. There is no entry for 10.1.0.10, so PC3 says `Network is unreachable` at once. With the gateway, things look different:

```console
root@PC3:~# ip route add default via 10.2.0.1
root@PC3:~# ip route
default via 10.2.0.1 dev eth0
10.2.0.0/24 dev eth0 proto kernel scope link src 10.2.0.10
root@PC3:~# ping -c 2 10.1.0.10
PING 10.1.0.10 (10.1.0.10) 56(84) bytes of data.
64 bytes from 10.1.0.10: icmp_seq=2 ttl=63 time=0.006 ms

--- 10.1.0.10 ping statistics ---
2 packets transmitted, 1 received, 50% packet loss, time 1000ms
rtt min/avg/max/mdev = 0.006/0.006/0.006/0.000 ms
root@PC3:~# ip neigh
10.2.0.1 dev eth0 lladdr 02:4a:00:00:04:02 REACHABLE
```

`default` stands for "everything else". The first echo is lost, the second gets through. The reason: the router does not know the MAC address of PC1 yet. It asks by ARP and simply throws away the packet that set off the question. Real routers of this kind do the same, so a ping across a fresh router often starts with a loss, and `-c 1` would be a poor test here.

Look at the TTL in the reply as well: PC1 started with 64, and the router took one off. And `ip neigh` shows only the router, not PC1, because no frame goes to PC1 directly. PC3 never needs to know the MAC address of PC1.

## Both directions

Every ping needs two ways to be right. If only PC3 is set up correctly, its echo may arrive, but the reply will not find its way back. So a check always includes the question: does the other side know how to answer, too? In this lab PC1 has already been prepared; in a real network this is a common source of faults, and the course on fault finding returns to it.

Usually a host does not get its gateway by hand at all, but from DHCP. That comes in the next unit.

A host may know more than the gateway, by the way. `ip route add` can send single networks through another router, though hosts seldom need that. Routers, on the other hand, live on their routing table, which is the topic of [the next lesson](step:routing-tabelle).
