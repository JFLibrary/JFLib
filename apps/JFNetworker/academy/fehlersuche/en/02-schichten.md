# From the bottom up

A network is built in layers, and each layer relies on the one below. Fetching a web page needs a resolved name, which needs a packet to the name server, which needs a path, which needs frames on a cable. That makes the lower half of the stack the sensible place to start: as long as a cable is missing, all statements about routes and names are worthless.

![Four layers on top of each other, each with its commands: cables and ports, frames, paths, application](img/schichten-en.svg)

## Four questions, from the bottom up

**1. Cables and ports: is there a link?**
On a host, `ip -br link` shows whether the interface is `UP`. `NO-CARRIER` means there is no partner at the other end. On a router or switch, `show ip interface brief` shows the state of each interface, and `show interfaces GigabitEthernet0/0/1` also shows why. `administratively down` is shut down, while a `down` without that addition has no partner. A look at the canvas helps as well: the lights on the cables go on and off.

**2. Frames: can the neighbour be found?**
`ip neigh` shows for which addresses a host knows a MAC address. If the line is missing, the ARP question went unanswered; a real Linux often shows a line with `FAILED` then. `show mac address-table` on the switch shows which address was seen at which port, and `show vlan brief` which port belongs to which VLAN.

**3. Paths: does a packet get there and back?**
`ip addr` and `ip route` show what a host believes about itself. `ping` tests a path, in both directions at once. `traceroute` shows how far answers find their way back. `show ip route` shows what a router knows.

**4. Application: does the service do its job?**
`dig` asks the name server, `curl` fetches a web page, `ss` shows who is listening on which port. `cat /etc/resolv.conf` shows which name server a host asks.

## Do you always have to start at the bottom?

No. The approach is a tool and not a rule. If a ping to the address works and only the name does not, you already know that the lower three questions are answered: the reply did come back. Then you start at the top.

On the other hand, working from the bottom has an advantage you come to value with time: it costs little, and it keeps you from searching routes for an hour while a plug is loose. That happens to experienced people too.

A fault in an upper layer often shows only once the lower ones carry. If you repair a pulled cable and still get no web page, you have not failed. You are one layer further on.

The simulator also has the simulation mode, which shows every packet and what every device does with it. Such a close look costs more in real networks, where it takes captures at the right spot. You may use it in the fault hunts of this course, but try the commands first, because you have those everywhere.

Once it is clear which layer is at fault, the question of where remains. More on that in the [next lesson](step:eingrenzen).
