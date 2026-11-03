# Frames and MAC addresses

The pieces that cross the cable in a local network are called **frames**. A frame is like an envelope: the recipient and the sender are written on the outside, and what is being carried lies inside.

![A frame as a strip: destination MAC, source MAC, type, data and checksum](img/rahmen-en.svg)

## What a frame holds

- **Destination MAC** and **source MAC**: to whom and from whom, 6 bytes each.
- **Type**: what lies inside, for example IPv4 or ARP.
- **Data**: usually up to 1500 bytes.
- **Checksum**: whoever receives the frame recomputes it and throws the frame away if a bit flipped on the way. The sender never hears about it.

A **MAC address** belongs to a network card, not to a network. It has 48 bits and is written in hexadecimal, such as `02:4a:00:00:01:01`. The manufacturer gave it to the card, so it says *who* a device is, but not *where* it stands. Devices in the simulator get their MAC addresses from a fixed pattern when they start, which is why all of them begin with `02:4a`. Real cards carry their maker's code there.

There is one special destination: `ff:ff:ff:ff:ff:ff`, the **broadcast**. Such a frame is meant for everyone in the network.

## How a switch learns

Nobody has to set up a switch for it to deliver frames correctly. It learns by listening, with two simple rules:

1. When a frame arrives, it notes that the **source MAC** sits behind this port.
2. Then it looks up the **destination MAC**. If it knows it, the frame leaves through that one port only. If it does not, or if it is a broadcast, the frame leaves through all the other ports. This is called **flooding**.

![A switch with a table: a frame to an unknown address goes to all ports, the answer goes to just one](img/switch-lernt.svg)

So the table fills by itself as soon as devices send. An entry expires after five minutes without traffic, and it goes at once if the link on its port drops. That is how a switch copes with a device being plugged in elsewhere.

## See for yourself

Before the first frame, the table of SW1 is empty. After a ping, both PCs are in it, each at its port:

```console
SW1>enable
SW1#show mac address-table
          Mac Address Table
-------------------------------------------

Vlan    Mac Address       Type        Ports
----    -----------       --------    -----
Total Mac Addresses for this criterion: 0
root@PC1:~# ping -c 1 192.168.1.20
PING 192.168.1.20 (192.168.1.20) 56(84) bytes of data.
64 bytes from 192.168.1.20: icmp_seq=1 ttl=64 time=0.006 ms

--- 192.168.1.20 ping statistics ---
1 packets transmitted, 1 received, 0% packet loss, time 0ms
rtt min/avg/max/mdev = 0.006/0.006/0.006/0.000 ms
SW1#show mac address-table
          Mac Address Table
-------------------------------------------

Vlan    Mac Address       Type        Ports
----    -----------       --------    -----
   1    024a.0000.0101    DYNAMIC     Gi1/0/1
   1    024a.0000.0201    DYNAMIC     Gi1/0/2
Total Mac Addresses for this criterion: 2
```

`enable` lifts the IOS-like console to the level from which everything can be seen; the prompt changes from `>` to `#`. The column `Type` says `DYNAMIC`: learned, not entered by hand.

But how does PC1 know the MAC address of PC2 before sending it its first frame? That is still open, and the next lesson settles it: [ARP](step:arp).
