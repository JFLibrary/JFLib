# A short reminder: DHCP across a router

You know from the basics course that a client starts with a broadcast, because it knows no address and no server yet. And broadcasts do not pass a router. If the DHCP server stands in the same network, that is no problem. In larger networks, though, there is usually one central server somewhere in the building, while the clients sit in many other networks.

## The relay

The solution is called a **DHCP relay**. The router at the border hears the clients' broadcasts and passes them straight on to the server.

![A PC sends a broadcast to the router, which passes it on as a directed message to the DHCP server](img/relay.svg)

On an IOS-like router, one line at the interface towards the clients is enough:

`ip helper-address 10.0.3.10`

That means: whatever arrives at this interface as a DHCP broadcast goes as a directed request to 10.0.3.10.

Two things are neatly solved here:

- The router enters the address of **its own interface** in the request (the field is called `giaddr`). From that, the server knows which network the client comes from and which pool to take the address from.
- The server sends the answer back to this address. The router then passes it on in the client's network.

So the server needs a pool for each of these networks, and it must know the way back to the router.

## What this means for fault finding

If the line is missing, the client sees exactly what it would also see with a failed server:

```console
root@PC2:~# dhclient -v eth0
DHCPDISCOVER on eth0 to 255.255.255.255 port 67 interval 9 (xid=0x56b2032)
DHCPDISCOVER on eth0 to 255.255.255.255 port 67 interval 16 (xid=0x56b2032)
DHCPDISCOVER on eth0 to 255.255.255.255 port 67 interval 17 (xid=0x56b2032)
DHCPDISCOVER on eth0 to 255.255.255.255 port 67 interval 9 (xid=0x56b2032)
No DHCPOFFERS received.
No working leases in persistent database - sleeping.
```

A few tries, then nothing. It pays to separate two questions:

- **Does the request reach the server?** The server cannot report that by itself. What helps here is the question of what applies to other networks: if clients elsewhere are served, the server is alive.
- **Is the router prepared?** `show running-config` on the router shows the line per interface, and a comparison with an interface where it works is quickly made.

`show running-config` lists the whole configuration. On a router with many interfaces it pays to start with `show ip interface brief` and then read selectively.

One more hint: a client that got no answer waits. Real clients try again after a few minutes, and so does the simulator. The impatient give it a push: `dhclient eth0`.

On to the fault hunt: [No address in the second network](step:relay).
