# The routing table and traceroute

For every packet, a router asks the same question: which entry of my table fits the destination address? The **routing table** is a list of networks, each with its next step. Hosts have one too; it just usually has two lines.

```console
R1#show ip route
Codes: C - connected network, L - address of this device, S - static route,
       * - default route

Gateway of last resort is not set

      10.0.0.0/8 is variably subnetted, 4 subnets, 2 masks
C        10.1.0.0/24 is directly connected, GigabitEthernet0/0/0
L        10.1.0.1/32 is directly connected, GigabitEthernet0/0/0
C        10.2.0.0/24 is directly connected, GigabitEthernet0/0/1
L        10.2.0.1/32 is directly connected, GigabitEthernet0/0/1
```

## Where the entries come from

- **C** (connected): the network lies at one of the device's own interfaces. The entry appears by itself as soon as the interface has an address and is running.
- **L** (local): the interface's own address, as a single host with /32.
- **S** (static): someone set the entry by hand, for example like this: `ip route 10.9.0.0 255.255.255.0 10.2.0.10`. That means the network 10.9.0.0/24 lies behind the router 10.2.0.10.
- `*` at an entry means that it is the **default route**, the counterpart of a host's gateway. It carries the network 0.0.0.0/0.

Then there are routes from routing protocols like OSPF, with which routers tell each other their networks. That is material for a later course.

## Which entry wins?

Several may fit. Then the **longest prefix** wins, the most exact entry:

```text
dest 10.9.0.77
  0.0.0.0/0     fits   (0 bits)
  10.0.0.0/8    fits   (8 bits)
  10.9.0.0/24   fits   (24 bits)  wins
  10.9.1.0/24   does not fit
```

If nothing fits, the router drops the packet and sends the sender an ICMP message: `Destination Net Unreachable`, **with the router's address as the sender**. Such a message tells you how far the packet got.

## The way, step by step: traceroute

`traceroute` makes the way visible and uses the TTL to do it. It sends packets with TTL 1, then 2, then 3 and so on. A router that brings the TTL to zero drops the packet and reports that back. So every router on the way gives itself away, one after the other, and at the end the destination itself answers.

![Three packets with TTL 1, 2 and 3: the first ends at the first router, the second at the second, the third reaches the destination](img/traceroute.svg)

```console
root@PC1:~# traceroute -n 10.2.0.10
traceroute to 10.2.0.10 (10.2.0.10), 30 hops max, 60 byte packets
 1  10.1.0.1  0.006 ms  0.003 ms  0.003 ms
 2  * 10.2.0.10  0.006 ms  0.006 ms
```

Each line is a step, and the three times come from three packets for that step. A `*` in place of a time means that this packet got no answer. That often happens only with the very first packet on a way, when a router first has to search by ARP. If a line shows nothing but stars, that is usually the end: the router after that does not answer, or the answer does not find its way back. For fault finding this is worth a great deal.

`-n` leaves out the name lookup. Without `-n`, `traceroute` tries to turn the addresses into names, which can take a while without a name server.

Real `traceroute` programs send UDP or ICMP, depending on the system, and some routers on the internet deliberately do not answer at all. Stars alone are no fault there.

Now set up a router yourself: [Two networks, one router](lab:router-gateway).
