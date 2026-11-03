# A short reminder: VLANs

You need VLANs for the next fault hunt. A course of its own will cover them in detail later; here is the bare minimum.

A **VLAN** splits a switch into several logical switches. Ports in the same VLAN can reach each other, ports in different VLANs cannot. This holds for broadcasts as well: an ARP question stays in its VLAN. To the devices it looks as if they hung on separate switches, and traffic between two VLANs needs a router, just as between two networks.

![A switch with four ports; the two on the left belong to VLAN 10, the two on the right to VLAN 20](img/vlan.svg)

VLANs are used to keep groups apart without buying a switch for each: office and technicians, for example, or guests and staff. Normally each VLAN also gets an IP network of its own; in the lab the two groups exceptionally share one address range, so that the separation hangs on the VLAN alone.

## How a port gets its VLAN

An **access port** belongs to exactly one VLAN. Without any setting that is VLAN 1. This is how a port gets into another:

```console
SW1>enable
SW1#configure terminal
Configuration mode: one command per line; end or Ctrl+Z leaves it.
SW1(config)#interface GigabitEthernet1/0/6
SW1(config-if)#switchport mode access
SW1(config-if)#switchport access vlan 10
SW1(config-if)#end
*Mar  1 00:00:09.000: %SYS-5-CONFIG_I: configuration changed at the console
```

The host device notices none of this. It has neither a setting for it nor an error message. That is what makes this fault so awkward: cable, address and mask are right, everything lights up, and still nobody answers.

## How to check it

`show vlan brief` shows each VLAN with its ports:

```console
SW1#show vlan brief

VLAN Name                             Status    Ports
---- -------------------------------- --------- -------------------------------
1    default                          active    Gi1/0/7, Gi1/0/8, Gi1/0/9
                                                Gi1/0/10, Gi1/0/11, Gi1/0/12
                                                Gi1/0/13, Gi1/0/14, Gi1/0/15
                                                Gi1/0/16, Gi1/0/17, Gi1/0/18
                                                Gi1/0/19, Gi1/0/20, Gi1/0/21
                                                Gi1/0/22, Gi1/0/23, Gi1/0/24
                                                Te1/1/1, Te1/1/2, Te1/1/3
                                                Te1/1/4
10   Buero                            active    Gi1/0/1, Gi1/0/3, Gi1/0/6
20   Technik                          active    Gi1/0/2, Gi1/0/4, Gi1/0/5
1002 fddi-default                     act/unsup
1003 token-ring-default               act/unsup
1004 fddinet-default                  act/unsup
1005 trnet-default                    act/unsup
```

The lines below that with the numbers 1002 to 1005 are an old legacy and you can pass over them.

A typical picture for a port in the wrong VLAN: the device can reach some others, namely those in its wrong VLAN, and just the ones it is looking for it cannot. It helps to ask: whom can this device talk to? The pattern gives away the VLAN.

On to the fault hunt: [In the wrong VLAN](step:vlan).
