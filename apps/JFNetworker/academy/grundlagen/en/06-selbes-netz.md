# Who is in the same network?

Before a host sends anything, it decides: does the destination lie in my own network or not? What it does next depends on the answer. This is the most important rule of this unit, and it explains many faults in real networks.

![A decision picture: destination and own address are compared using the own mask; equal means directly by ARP, different means through the gateway](img/selbes-netz-en.svg)

## The calculation

The host takes **its own mask** and applies it to two addresses: its own and the destination's. If both give the same network, the destination is in its own network. Then it asks for the destination's MAC address by ARP and sends the frame directly.

If something else comes out, the destination is elsewhere. Then it sends the frame to its gateway, or gives up if it has none. The gateway is the topic of the next unit.

An example with the mask /24, where only the first three numbers count:

```text
me    192.168.10.10 /24   network 192.168.10.0
dest  192.168.10.130      network 192.168.10.0    same: directly
dest  192.168.11.5        network 192.168.11.0    other: gateway
```

And with /25, where the border runs through the last number:

```text
me    192.168.10.130 /25  network 192.168.10.128
dest  192.168.10.140      network 192.168.10.128  same: directly
dest  192.168.10.10       network 192.168.10.0    other: gateway
```

## Everyone calculates alone

Nobody checks whether the masks of two hosts fit together. Each calculates with its own, and strange faults come from that. In the next lab, PC1 and PC3 are set up correctly (/24), and PC2 has /25:

```console
root@PC2:~# ip route
192.168.10.128/25 dev eth0 proto kernel scope link src 192.168.10.130
root@PC2:~# ping -c 1 192.168.10.140
PING 192.168.10.140 (192.168.10.140) 56(84) bytes of data.
64 bytes from 192.168.10.140: icmp_seq=1 ttl=64 time=0.006 ms

--- 192.168.10.140 ping statistics ---
1 packets transmitted, 1 received, 0% packet loss, time 0ms
rtt min/avg/max/mdev = 0.006/0.006/0.006/0.000 ms
root@PC2:~# ping -c 1 192.168.10.10
ping: connect: Network is unreachable
```

PC2 reaches PC3 in its half. For PC1 it says `Network is unreachable` right away: to PC2 the destination lies outside, and it has no gateway. Not a single packet went onto the wire. Read the first line of `ip route`: that is all PC2 knows.

In the other direction, things look different:

```console
root@PC1:~# ping -c 1 192.168.10.130
PING 192.168.10.130 (192.168.10.130) 56(84) bytes of data.

--- 192.168.10.130 ping statistics ---
1 packets transmitted, 0 received, 100% packet loss, time 0ms
```

PC1 calculates with /24: same network, so directly. Its echo reaches PC2, and PC2 wants to answer but finds no way back. So PC1 waits and gets nothing, without any error message. A fault can look different depending on the direction, and that is a clue worth following.

`Destination Host Unreachable` would be something else again: the destination lies in the own network, but nobody answers the ARP request. Perhaps because the device is off or does not exist at all.

Now you can repair it yourself: [A mask too narrow](lab:falsche-maske).
