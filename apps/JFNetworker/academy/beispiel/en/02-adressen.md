# Addresses

For a packet to find its destination, every host needs an **IP address** of its own, `10.0.0.1` for
example. The suffix `/24` gives the size of the network: the first 24 bits of the address are the same
on all hosts of this network, here `10.0.0`. Only the last number tells them apart.

![On the left a network with two hosts, on the right a second network with one host; the two do not touch](img/adressen.svg)

Hosts in the same network reach each other directly through the switch. A host in another network
cannot be reached without a router, even if it hangs off the same switch.

On a Linux host this command sets an address:

```shell
ip addr add 10.0.0.9/24 dev eth0
```

Try it out: in the [lab](lab:erstes-netz) a PC is missing its address.
