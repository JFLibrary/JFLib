# Mask and prefix

The **mask** says where in an address the network part stops and the host part begins. It is 32 bits long as well: from the left, ones for the network part, then zeros for the host part, never mixed up. Since only the number of ones matters, it is often written more briefly as a **prefix**: `/24` means "24 ones".

```text
Address  192.168.1.20    11000000 10101000 00000001 00010100
Mask     255.255.255.0   11111111 11111111 11111111 00000000
Network  192.168.1.0     11000000 10101000 00000001 00000000
```

You get the network by setting the host part to zero. Bit by bit: where the mask has a 1, the address bit stays; where it has a 0, it becomes 0. In computing this is called an AND operation.

## How big is a network?

The zeros of the mask decide the size: with *n* bits for the host part there are 2 to the power of *n* addresses. Two of them are taken in every network:

- All host bits 0: the **network address**, the name of the network.
- All host bits 1: the **broadcast address**, which means everyone in the network.

| Prefix | Mask | Addresses | for hosts |
|---|---|---|---|
| /16 | 255.255.0.0 | 65536 | 65534 |
| /24 | 255.255.255.0 | 256 | 254 |
| /25 | 255.255.255.128 | 128 | 126 |
| /26 | 255.255.255.192 | 64 | 62 |
| /27 | 255.255.255.224 | 32 | 30 |
| /30 | 255.255.255.252 | 4 | 2 |

A /30 suits the link between two routers, because exactly two addresses are needed there.

## A network that does not end on a number's border

With /24, the border falls between two numbers and is easy to see. With /25 it runs through the middle of the last number. The first of the 8 bits still belongs to the network, and this bit is worth 128. That is why 192.168.10.0/24 falls into two halves:

```text
192.168.10.0/25    addresses .0   to .127   hosts .1   to .126
192.168.10.128/25  addresses .128 to .255   hosts .129 to .254
```

You can also read this from what a host says about itself. Here is PC2 from the next lab:

```console
root@PC2:~# ip addr show eth0
2: eth0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc fq_codel state UP group default qlen 1000
    link/ether 02:4a:00:00:02:01 brd ff:ff:ff:ff:ff:ff
    inet 192.168.10.130/25 brd 192.168.10.255 scope global eth0
       valid_lft forever preferred_lft forever
```

The line `inet` gives address and prefix, `brd` the broadcast address: `192.168.10.130/25`, so the upper half, and `192.168.10.255` as its end. An IOS-like console spells out the mask when you set it up (`ip address 10.2.0.1 255.255.255.0`), a Linux-like one uses the prefix (`ip addr add 10.2.0.1/24 dev eth0`). Both mean the same.

What follows from the mask, namely whom a host considers reachable at all, is shown in the next lesson: [Who is in the same network?](step:selbes-netz)
