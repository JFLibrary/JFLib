# What the outputs give away

The same question can get quite different answers, and each of them gives away which layer is at fault. It pays to read the error messages word for word.

## What a ping can say

| Answer | Meaning |
|---|---|
| `64 bytes from …` | The way there and back both work |
| `connect: Network is unreachable` | The host itself knows no way: no route and no gateway. No packet left |
| `From` *own address* `… Destination Host Unreachable` | The destination or gateway lies in the own network, but the ARP question went unanswered |
| `From` *router* `… Destination Net Unreachable` | It got as far as this router, which knows no way to the destination network |
| nothing, only the statistics at the end | Something got lost on the way, or the answer does not find its way back |
| `Temporary failure in name resolution` | No name server answers |
| `Name or service not known` | The name server answers that the name does not exist |

A single message often says where to search next. An example from the network with the pulled cable:

```console
root@PC1:~# ping -c 1 10.0.0.10
PING 10.0.0.10 (10.0.0.10) 56(84) bytes of data.
From 10.0.0.11 icmp_seq=1 Destination Host Unreachable

--- 10.0.0.10 ping statistics ---
1 packets transmitted, 0 received, +1 errors, 100% packet loss, time 0ms
root@PC1:~# ip neigh
```

`Destination Host Unreachable`, and the sender is PC1 itself: it asked and got no answer. So `ip neigh` stays empty. A real Linux shows a line with `FAILED` here for a while; the simulator leaves it out.

## Ask the device itself

On the server's side it looks like this. The line gives the state and, in angle brackets, what the interface reports:

```console
root@SRV1:~# ip -br link
lo               UNKNOWN        00:00:00:00:00:00 <LOOPBACK,UP,LOWER_UP>
eth0             DOWN           02:4a:00:00:04:01 <NO-CARRIER,BROADCAST,MULTICAST,UP>
eth1             DOWN           02:4a:00:00:04:02 <NO-CARRIER,BROADCAST,MULTICAST,UP>
eth2             DOWN           02:4a:00:00:04:03 <NO-CARRIER,BROADCAST,MULTICAST,UP>
```

`DOWN` and `NO-CARRIER` at `eth0`: nobody is at the other end of the cable. `eth1` and `eth2`, the server's other sockets, never had one.

## The port on the switch

A port's state can be bad in two ways, and only one of them has to do with the cable. In the same network, the switch says this about the port of PC1 and that of the server:

```console
SW1>enable
SW1#show interfaces GigabitEthernet1/0/1
GigabitEthernet1/0/1 is up, line protocol is up (connected)
…
SW1#show interfaces GigabitEthernet1/0/3
GigabitEthernet1/0/3 is down, line protocol is down (notconnect)
…
```

The first line of each answer is the important one: `up` and `up` is fine, while a plain `down` means no partner at the other end. A port that is shut down would say `administratively down`, as in the second fault hunt. The rest of the output holds counters. If errors rise there, the cable or the other end is suspect. The `…` stands for the many lines that do not matter here. In the window, the IOS-like console stops long outputs after a page with `--More--`: the space bar pages on, it takes no new commands until then, and `terminal length 0` turns the paging off.

## What the address says and what it does not

`ip addr` shows what a host believes, not what is true. Two hosts with the same address both say contentedly that they are it. Only the MAC address in `ip neigh` gives such a case away: it does not belong to the device you expect. The switch's table then leads to the right port.

## Not only whether, but how well

A network can work and still be bad. A single ping says almost nothing about that. Many pings show losses:

`ping -c 30 10.0.2.80`

A line saying `10% packet loss` is a serious finding on a LAN, even if everything somehow runs: TCP sends lost data again, and each gap costs waiting time. `curl -s -o /dev/null -w "%{time_total}\n" http://10.0.2.80/` shows how long a fetch takes. In the simulator, the Metrics panel also shows a service's waiting times as soon as it serves requests.

That leaves the question of how to keep track of it all: [One change, one test, one note](step:notizen).
