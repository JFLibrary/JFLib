# One change, one test, one note

A good guess is of little use if you no longer know what you have already done. That is why three habits belong to the method.

## One change at a time

Always change just one thing, and test right after. If you change three things at once and it works afterwards, you do not know which of them helped, and you may leave two needless changes in the network. If it does not work, you know even less.

This applies even when you are sure. A test takes seconds, a false trail an hour.

## Always the same test

Decide beforehand how you will measure success, and always use exactly this test: the same source, the same destination. A ping from the router says something different from a ping from the PC, as the previous lesson showed. If you switch to another test midway, you compare apples with pears.

And watch out for states that linger. An ARP table remembers even a wrong entry for a while, and a client with a DHCP search under way does not fetch a new address at once. So after a repair it may take a moment or a fresh push. `ip neigh flush all` empties the table, and `dhclient eth0` asks anew.

## Write it down

Note what you have checked and what you have changed, including the guesses that proved wrong. That helps you if you are interrupted. It helps the colleague who takes over. And it helps you when undoing things: what had no effect, you take back.

It need not be anything big. A few lines are enough:

```text
10:05  PC1 -> SRV1: Destination Host Unreachable
10:07  ip -br link on SRV1: eth0 DOWN
10:09  cable SW1-SRV1 replaced
10:10  PC1 -> SRV1: answer. PC2 -> SRV1: answer.
```

In the end, the answer to the question of what the cause was stands there as well. That is not the same as what helped: a restart often helps without anyone knowing the cause, and then the fault comes back.

## How a fault hunt works in this course

Each fault hunt opens a broken network. The Problems panel, the list of faults under Properties and the answer are hidden, because you are to search as in a real network.

- At the top are the complaint and what you are to do. The hints open one after the other, and they cost nothing.
- You repair as in real life. A pulled or bad cable you delete and connect the ends again. A setting you change at the console.
- Under **Diagnosis** you name the device or cable to blame, from the list or with a click on the canvas. That counts only when you hand in.
- **Hand in** grades the state at that moment. After that you see the points and an explanation of what was wrong and how you could have found it.
- **Start over** on the bar loads the network again in its broken state. Trying things out is expressly allowed.

Repairing alone is not enough to pass: the diagnosis is part of it. A repair you do not understand is only luck.

Let us begin with something simple. The quiz asks about the method once more, and then it starts: [The file server is gone](step:kabel).
