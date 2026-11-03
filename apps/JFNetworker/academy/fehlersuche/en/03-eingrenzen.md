# Narrowing down instead of searching everywhere

A way through a network has many stations, and not all of them can be checked quickly. If you start at the front and work your way along, device by device, you get there, but slowly. There are three better ways.

## Halve the way

Check in the middle. If a ping gets to the middle router, the fault lies behind it, and the front half is done. If it does not, the fault lies before it. With each test, what is still in question is halved. With eight stations, that is at most three tests instead of eight.

![Five stations in a row; a ping to the middle works, so only the rear half stays in question](img/eingrenzen.svg)

`traceroute` does this in one go: it shows how far answers come back.

## Compare

Two devices, one of which works and one of which does not, are a gift. What differs is suspect. Put the outputs side by side: `ip addr`, `ip route`, `cat /etc/resolv.conf`, the ports on the switch.

A pattern among those affected says something too. If PC2 and PC3 do not work and PC1 does, ask: what do the two have in common? The same room? The same switch? Or addresses in the same half of the network?

## From both sides

A connection has two ends, and often the fault is not where the problem is reported. The one complaining sits at the client; the cause may just as well lie at the server. So look at both sides and see what each believes about the other.

## There and back

That leads to the most treacherous fault: a ping needs **two** ways. The way there may be fine while the way back is missing.

![The way from PC1 through R1 and R2 to the server is green, the way back ends at R2 with a red cross](img/rueckweg.svg)

This is what it looks like: the router R2 knows the server, but not the network of PC1. From the sender's point of view, only silence can be seen. `traceroute` shows the first router, and after that stars:

```console
root@PC1:~# traceroute -n -m 3 -w 1 10.0.2.10
traceroute to 10.0.2.10 (10.0.2.10), 3 hops max, 60 byte packets
 1  10.0.1.1  0.003 ms  0.002 ms  0.002 ms
 2  * * *
 3  * * *
```

Why does R2 not answer? It would like to, but its answer goes to PC1 as well, and it does not know that way. So the end of the answers in `traceroute` is not necessarily the end of the way there.

The ping from R1 to the server, on the other hand, works. That already narrows down the cause, because as a sender R1 asks from a network that R2 knows directly:

```console
R1>enable
R1#ping 10.0.2.10
Ctrl+Shift+6 stops the ping.
Pinging 10.0.2.10 with 5 echo requests of 100 bytes, 2 s timeout each:
!!!!!
Success rate is 100 percent (5/5), round-trip min/avg/max = 0/0/0 ms
```

So a router answers when it asks itself, and stays silent when a host behind it asks. When you see that, look at the routes on the way back: `show ip route` on R2. A typical sentence in such cases is: "But the way there works!" That is true, and that is exactly why it does not help.

The rule from this: whoever checks routes checks them for both directions. The [next lesson](step:werkzeuge) shows what this means in detail in the outputs.
