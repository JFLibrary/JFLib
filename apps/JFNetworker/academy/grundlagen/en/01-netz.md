# What a network is

A network connects devices so that they can send each other data. A whole file never crosses the wire in one piece. Every message is cut into small pieces, and each piece says where it is going and where it came from. That way many devices can take turns on the same wire, and a lost piece can be sent again on its own without starting everything over.

![Two PCs and a server, each with its own cable to a switch in the middle](img/stern.svg)

## Who takes part

A **host** is a device that sends and receives data to do something with it itself: a PC, a laptop, a server, a printer. Every message starts and ends at a host.

A **switch** is the distributor in the middle. It has many sockets, called **ports**, and passes on through one port what arrives at another. It wants nothing from the data for itself.

A **cable** joins exactly two ports. In offices it is usually copper with RJ45 plugs, over longer distances fibre.

## Why a switch?

You could wire every host directly to every other one. Four devices would already need six cables, twenty devices 190, and each PC would need 19 sockets. With a switch, every host needs exactly one cable. That is why networks in buildings nearly always look like the picture: a star, with several stars joined through their switches.

## In the simulator

A lab opens a network on the canvas. The small lights at the ends of a cable show whether a link is up. A double click on a device opens its console, where you type commands as on a real device. PCs and servers have a Linux-like command line, routers and managed switches an IOS-like one. This is how you ask a PC which addresses it has:

```console
root@PC1:~# ip -br addr
lo               UNKNOWN        127.0.0.1/8
eth0             UP             192.168.1.10/24
```

`lo` is the interface through which a device talks to itself. `eth0` is the socket for the cable, and `UP` means that a cable is plugged in and the other end is alive.

The simulator simplifies some things, and we say so each time. Here is the first one: in the simulator, a port is ready the moment the cable is in. A real switch takes a second or two for that, and with spanning tree switched on as much as half a minute.

Next we look at what travels along the cable: [frames and MAC addresses](step:rahmen).
