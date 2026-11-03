# ARP: from the IP address to the MAC address

Anyone looking for something in a network usually knows the **IP address** of the destination; we come to that in the next unit. On the cable, however, the **MAC address** is what counts, because that is what the switch goes by. **ARP**, the Address Resolution Protocol, links the two.

![PC1 calls to the whole network who has an address; only PC2 answers, directly to PC1](img/arp.svg)

## Question and answer

If PC1 wants to send something to 192.168.1.20 without knowing the matching MAC address, this is what happens:

1. PC1 sends an **ARP request** as a broadcast: "Who has 192.168.1.20? Tell 192.168.1.10." The request also names the MAC address of PC1. The switch floods it, so all hosts in the network see it.
2. Every host compares the address asked for with its own. Whoever is not meant drops the request.
3. PC2 is meant and sends an **ARP reply**, to PC1 only: "192.168.1.20 is mine, here is my MAC address." The switch has known PC1 since the request and has no need to flood.
4. PC1 enters the pair in its **ARP table** and sends what has been waiting all along.

The table saves the question next time. An entry is forgotten after a while without traffic; that way a host copes with an address later belonging to another device. On a Linux PC in the simulator an entry lasts up to a minute, on a router four hours. Real Linux systems handle this a little differently, but the order of magnitude is right.

ARP stays inside its own network: the request is a broadcast, and broadcasts do not pass a router. The unit on routers shows how hosts in other networks are reached.

## See for yourself

```console
root@PC1:~# ip neigh
root@PC1:~# ping -c 1 192.168.1.20
PING 192.168.1.20 (192.168.1.20) 56(84) bytes of data.
64 bytes from 192.168.1.20: icmp_seq=1 ttl=64 time=0.006 ms

--- 192.168.1.20 ping statistics ---
1 packets transmitted, 1 received, 0% packet loss, time 0ms
rtt min/avg/max/mdev = 0.006/0.006/0.006/0.000 ms
root@PC1:~# ip neigh
192.168.1.20 dev eth0 lladdr 02:4a:00:00:02:01 REACHABLE
```

The first time the table stays empty; after the ping, PC2 is in it. The line gives the address, the interface, the MAC address (`lladdr`) and the state. The reply time of a few microseconds is a simplification, by the way: the simulator only counts the time on the cable and for sending. On a real LAN it tends to be a few tenths of a millisecond, because the devices take time too.

## Watch it in the simulation mode

ARP is best seen step by step, for example in the next lab, as soon as PC2 has its address:

1. Press **F5** to switch to the simulation mode (menu Simulation). The clock now stands still.
2. On PC1, type `ping -c 1 192.168.1.20`. Nothing happens yet.
3. Press **F7** to go one step forward. An amber envelope travels from PC1 to the switch: the ARP request.
4. Next step: the switch sends it out of all other ports. The list in the Events panel says for each device what it does with it.
5. Then the reply comes back, followed by the green envelopes of ICMP: the actual ping.

A second ping shows no ARP envelope any more, because the table already knows the answer. **Shift+F5** takes you back to real time.

Now it is your turn: [Two PCs and a switch](lab:erstes-netz).
