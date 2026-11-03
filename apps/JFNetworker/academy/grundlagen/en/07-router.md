# Why two networks need a router

A switch joins the devices of **one** network. It reads MAC addresses, and those help only as long as everyone is in the same network, because you cannot even ask for the MAC address of a foreign host by ARP: the request stays in its own network.

Why divide into several networks at all? Because every broadcast reaches every device in a network, and all of them have to deal with it. That works well with a few hundred devices and no longer with a hundred thousand. You also want to keep areas apart, such as accounting from the guests, and to join networks in different places. In the end, the internet is nothing but very many networks with routers between them.

![Two networks, each with a switch and PCs, and between them a router with one interface and address in each](img/router.svg)

## What a router does

A **router** stands with one interface in each of its networks and has an address of its own in each. When a packet arrives, it decides by the **destination IP address** where it goes next.

The key point: the frame ends at the router, the packet does not.

1. PC1 sends the packet in a frame to the router's MAC address. The IP address inside names the real destination.
2. The router unpacks the packet, looks up the next network and takes one off the **TTL** in the packet. If that reaches zero, it drops the packet, so nothing circles forever.
3. It puts the packet into a new frame with new MAC addresses and sends it off.

So the MAC addresses change on every leg, while the IP addresses stay the same from start to end. In the simulation mode you can see this in the Packet tab: compare the same ping before and behind the router.

## Setting up a router

An interface of a router starts out shut down and without an address. That is how it stands in the lab of this unit. The overview shows it:

```console
R1>enable
R1#show ip interface brief
Interface              IP-Address      OK? Method Status                Protocol
GigabitEthernet0/0/0   10.1.0.1        YES NVRAM  up                    up
GigabitEthernet0/0/1   unassigned      YES unset  administratively down down
GigabitEthernet0/0/2   unassigned      YES unset  administratively down down
GigabitEthernet0/0/3   unassigned      YES unset  administratively down down
TenGigabitEthernet0/1/0 unassigned      YES unset  administratively down down
TenGigabitEthernet0/1/1 unassigned      YES unset  administratively down down
Serial0/2/0            unassigned      YES unset  administratively down down
Serial0/2/1            unassigned      YES unset  administratively down down
```

`administratively down` means that someone has shut it down, or nobody has ever switched it on. The column `Protocol` would say `down` even with the interface on, as long as no cable reports the other end.

The IOS-like command line has levels, and the prompt shows the current one. From `R1#`, `configure terminal` goes into the configuration, and from there `interface` goes to an interface:

```console
R1#configure terminal
Configuration mode: one command per line; end or Ctrl+Z leaves it.
R1(config)#interface GigabitEthernet0/0/1
R1(config-if)#ip address 10.2.0.1 255.255.255.0
R1(config-if)#no shutdown
*Mar  1 00:00:09.000: %LINK-3-UPDOWN: GigabitEthernet0/0/1 is up
*Mar  1 00:00:09.000: %LINEPROTO-5-UPDOWN: line protocol on GigabitEthernet0/0/1 is up
R1(config-if)#end
*Mar  1 00:00:10.000: %SYS-5-CONFIG_I: configuration changed at the console
R1#show ip interface brief
Interface              IP-Address      OK? Method Status                Protocol
GigabitEthernet0/0/0   10.1.0.1        YES NVRAM  up                    up
GigabitEthernet0/0/1   10.2.0.1        YES manual up                    up
GigabitEthernet0/0/2   unassigned      YES unset  administratively down down
GigabitEthernet0/0/3   unassigned      YES unset  administratively down down
TenGigabitEthernet0/1/0 unassigned      YES unset  administratively down down
TenGigabitEthernet0/1/1 unassigned      YES unset  administratively down down
Serial0/2/0            unassigned      YES unset  administratively down down
Serial0/2/1            unassigned      YES unset  administratively down down
```

The lines with `%LINK` and `%SYS` are the device's messages, which mix in between the commands. The time in front depends on when you type; it counts from the start, because the router's clock was never set. On real devices the message about the interface appears a few seconds after switching it on, in the simulator at once.

A router knows what to do with a packet from its routing table. Before we read that, let us look at the other side: how does a host know it should ask the router? That is the [default gateway](step:gateway).
