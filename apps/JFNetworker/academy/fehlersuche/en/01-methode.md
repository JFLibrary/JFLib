# Think first, then type

Faced with a broken network, you want to change something at once: swap a cable, restart a router, try a setting. Sometimes that even helps. But then you do not know why, and next time you are back at the start. If it does not help, you may well have built in a second fault.

This course practises the opposite: a method that works even if you do not know the network.

## What exactly does not work?

"The network is down" is not yet a description. Before you touch anything, settle four questions:

- **What** does not work? One particular page, all pages, only the name or the address as well?
- **Who** is affected? One device, everyone in a room, everyone in the building?
- **Since when?** And what has changed since? Changes are the most common cause of all.
- **What still works?** People often forget this, and it is the most valuable question. What still works rules out whole parts of the network as the cause.

Check what you are told as you go. "The cable is fine" often only means that someone looked at it. A ping takes seconds.

## Guess, check, conclude

The method is a loop:

1. **Make a guess:** what would explain this behaviour? For example: PC2 has a wrong gateway.
2. **Make a prediction:** if that is right, what should I see? For example: `ip route` on PC2 shows a different address from PC1.
3. **Check**, by reading first, without changing anything.
4. **Conclude:** if the prediction holds, the guess is supported. If it does not, the guess is wrong, and that takes you further too.

The second step is the most important. If you consider beforehand what you expect, you notice when something does not fit. Without a prediction, you read into any output what you already believe.

## A guess that does not fit is no failure

If the error message stays the same after a change, the guess was wrong, or there is a second fault. Then undo the change before you search on. Otherwise you add to the mess you are trying to clear up.

If the error message changes, that is good news, even if it does not work yet. A `Network is unreachable` that turns into silence means that the host now has a way, and the problem lies further along.

Where do you start when everything is in question? That is the topic of the [next lesson](step:schichten).
