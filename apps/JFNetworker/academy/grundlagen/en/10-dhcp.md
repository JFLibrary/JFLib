# DHCP: addresses that come by themselves

So far you have assigned addresses by hand. That works with three PCs, not with three hundred: it takes time, and sooner or later one device has the same address as another. **DHCP** (Dynamic Host Configuration Protocol) takes that over: a server manages a stock of addresses, the **pool**, and lends them to devices that ask. It hands out not only the address, but also the mask, gateway and name server.

## The four messages

A new client has no address yet and knows no server. So everything starts with a broadcast.

![Four arrows between client and server, taking turns back and forth: Discover, Offer, Request, Ack](img/dhcp.svg)

1. **Discover** (client to all): "I need an address. Is there a server here?"
2. **Offer** (server to the client): "You could have 10.10.0.100." Several servers may answer.
3. **Request** (client to all): "I will take 10.10.0.100 from this server." This is a broadcast too, so that other servers know they can withdraw their offers.
4. **Ack** (server to the client): "Agreed. Here is everything else, and it is yours for this long."

The client uses the address only after the Ack. That length of time is the **lease**. Halfway through, the client asks the server whether it may renew, directly and without a broadcast. So a device that leaves gives its address back when the lease runs out at the latest.

Since broadcasts do not pass a router, the Discover reaches only as far as the edge of the network. If the server stands in another network, an intermediary is needed, the DHCP relay. The course on fault finding deals with that.

## Setting up a pool

A router with an IOS-like command line can be a DHCP server itself:

```console
R1>enable
R1#configure terminal
Configuration mode: one command per line; end or Ctrl+Z leaves it.
R1(config)#ip dhcp excluded-address 10.10.0.1 10.10.0.99
R1(config)#ip dhcp pool BUERO
R1(dhcp-config)#network 10.10.0.0 255.255.255.0
R1(dhcp-config)#default-router 10.10.0.1
R1(dhcp-config)#end
*Mar  1 00:00:10.000: %SYS-5-CONFIG_I: configuration changed at the console
```

The line with `excluded-address` holds back addresses the pool must never hand out, for example for the router itself and devices with fixed addresses. `network` says which network the pool hands out from, and `default-router` tells clients their gateway. `dns-server 10.30.0.53` would add the name server.

## Watching it

```console
root@PC1:~# dhclient -v eth0
DHCPDISCOVER on eth0 to 255.255.255.255 port 67 interval 3 (xid=0x684e7a3d)
DHCPOFFER of 10.10.0.100 from 10.10.0.1
DHCPREQUEST for 10.10.0.100 on eth0 to 255.255.255.255 port 67 (xid=0x684e7a3d)
DHCPACK of 10.10.0.100 from 10.10.0.1 (xid=0x684e7a3d)
bound to 10.10.0.100 -- renewal in 32811 seconds.
root@PC1:~# ip route
default via 10.10.0.1 dev eth0
10.10.0.0/24 dev eth0 proto kernel scope link src 10.10.0.100
```

`dhclient -v` calls the four messages by name, and a second passes between `DHCPDISCOVER` and `DHCPOFFER`: that is how long the server checks with two pings whether the address is really free. Real devices of this kind do that as well, precisely so that no address is handed out twice. The route `default via` came with the Ack. `bound to` says when the client wants to renew: not exactly at half of the 24 hours, because `dhclient` puts the moment a little ahead of that with some randomness, so that not all clients ask at once.

The router shows what it has handed out:

```console
R1#show ip dhcp pool

Pool BUERO :
 Network                        : 10.10.0.0/24
 Total addresses                : 254
 Excluded addresses             : 99
 Leased addresses               : 1
 Default router                 : 10.10.0.1
 DNS server                     : -
 Domain name                    : -
 Lease                          : 1 day
```

In the simulation mode you see the messages as teal envelopes, and in the Packet tab each one shows its fields. Start `dhclient eth0` there and go step by step with **F7**: you also see the pings with which the server checks.

Try it out: [Addresses from the router](lab:dhcp-pool).
