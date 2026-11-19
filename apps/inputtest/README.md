# Eingabetest

What a game needs of the input, to try by hand (stage G7a of [GAME.md](../../docs/GAME.md)): the
pointer locked to the window with its raw relative motion, keys by where they sit on the keyboard
whatever the layout prints on them, and game pads. The calls are described in
[GUIDE.md](../../docs/GUIDE.md) section 39.

- **Zeiger fangen** (or a press in the field) locks the pointer: it disappears and stays put, its
  raw motion adds up in *Summe* and moves the dot in the field. **Freigeben**, Esc, switching to
  another window, minimizing or closing let it go. *Zeiger:* says the state (aus, wartet, gefangen,
  nicht möglich, abgelehnt).
- With the field focused, every key is listed by its place (*Tasten*: `+KeyW` down, `-KeyW` up, the
  browser's code names of the USB HID usages); *Letzte Taste* shows its usage, the platform's scan
  code, the character and what the layout prints on the key. *WASD gehalten* shows which of the
  keys at the places of W, A, S and D are held; *Beschriftung der Lagen* what the layout prints on
  them.
- **Gamepads** lists every pad connected (`ui.gamepads`): *Pad N: name · style* (Xbox,
  PlayStation, Nintendo, andere; *eigene Belegung* for a browser pad without the standard
  mapping), *Knöpfe* the buttons held in the standard layout's names (A B X Y LB RB LT RT Back
  Start LS RS Up Down Left Right Home; the triggers with their value), *Sticks* both sticks as
  numbers (right and down positive, raw: no dead zone) and as dots in two circles. *Pad-Drücke*
  lists the buttons pressed in order (`1:B` for pad 1), *Pad-Ereignisse* the pads that came (`+0`)
  and went (`-0`).

## Build, run, test

```sh
python build_rs.py run inputtest                 # Windows
python build_rs.py app inputtest --target linux  # build/linux/inputtest
python build_rs.py app inputtest --target web    # build/web/inputtest (serve build/web)
python build_rs.py test inputtest                # tests/*.uis headless, Windows and Linux
```

The scripts play the lock (`motion DX DY`, `keydown z code 26`) and pads (`pad connect 0 xbox
NAME`, `pad press 0 a`, `pad axis 0 lx 0.5`: tests/pad.uis); `plattest --test input` checks the key
tables and the decoding of raw records, `plattest --test pad` each platform's pad backend with
injected states (Windows XINPUT_STATE records, Linux a pipe as a pad's node, macOS a controller of
GameController's own), `node tools/tests/web_input.mjs build` the browser host with Playwright's
synthetic input, `node tools/tests/web_pad.mjs build` its pads with a fake `navigator.getGamepads`.
A real mouse and a real pad need a person:

## Check list by hand

On each platform: start the program, click **Zeiger fangen**, then

1. the cursor is gone and does not leave the window, moving the mouse changes *Summe* (right and
   down positive) and moves the dot; slow and fast moves feel unaccelerated (Windows, Linux, the
   browser in Chrome on Windows/macOS) or as the desktop's pointer (macOS, the browser without
   `unadjustedMovement`); clicks and the wheel still work in the window;
2. **Esc** lets it go: *Zeiger: aus*, the cursor is back where it was before the lock (the
   browser: where the browser puts it), *Tasten* shows no `Escape`; click **Zeiger fangen** again;
3. **Alt+Tab** (macOS: **Cmd+Tab**) to another program lets it go; coming back does not lock again
   until you click;
4. lock again, then minimize with the keyboard (Windows: **Win+Down** twice; KDE: the window
   menu's shortcut) -- the lock is gone, the cursor free;
5. click into the field and press the keys at W A S D: *WASD gehalten* shows them by place (on a
   French layout Z Q S D); *Beschriftung der Lagen* shows the layout's letters; switch the
   keyboard layout (e.g. German -> French) and press again: the places stay, the letters change;
6. close the window while locked (Alt+F4 / Cmd+Q): the cursor comes back;
7. game pads (an Xbox pad; where you have one, a PlayStation or Switch Pro pad too): connect it
   while the program runs -- *Pad 0: ...* appears with the right style (in the browser only after
   a button was pressed in the page); every button shows its name in *Knöpfe* while held and lands
   in *Pad-Drücke* (A at the bottom, Y at the top on every pad: on a PlayStation pad cross is A,
   triangle Y; on a Switch pad the bottom button B is A); the triggers count up from 0.00 to 1.00
   and are listed from the first touch; the sticks move their dots the way you push them (up is
   negative in *Sticks*), both reach the circle's edge; the directional pad shows Up Down Left
   Right; the middle buttons Back and Start, the Xbox / PS / Home button Home (Windows: only with
   xinput1_4.dll);
8. switch to another program while holding a button: *Knöpfe* goes to `–`, the pad stays listed;
   back in the window it shows what is held again;
9. unplug the pad (or switch it off): *Pad-Ereignisse* shows `-0`, the pad goes from the list;
   plug it in again: `+0`; with two pads both are listed with their own index.

Where and how:

| Platform | Command | Look for |
| --- | --- | --- |
| Windows | `python build_rs.py run inputtest` | all of the above; with a remote desktop session (absolute pointer) the motion still comes, in pixels |
| Linux, KDE Wayland (ThinkPad) | copy `build/linux/inputtest` and its `.glibc` folder to the laptop, `./inputtest` | *Zeiger: wartet* turns into *gefangen* once the compositor confirms; KDE may show its own "pointer locked" note and has its own unlock shortcut -- that must end the lock too (*aus*) |
| Linux, X11 (ThinkPad, "Plasma (X11)" session or `jfgui_BACKEND=x11` under XWayland) | `jfgui_BACKEND=x11 ./inputtest` | as above; after the release the cursor is back where it was |
| macOS (Mac mini) | `python tools/apple.py gui build/mac/inputtest` after `python build_rs.py app inputtest --target mac` on the Mac | the cursor stays where it was (moved into the window first if it was outside); on an ISO keyboard (German) the key left of 1 shows `Backquote`, the one left of Y `IntlBackslash` |
| Chrome, Firefox | `python build_rs.py app inputtest --target web`, `python tools/serve_web.py build/web`, open http://127.0.0.1:8766/inputtest/ | **Zeiger fangen** locks at once (the click's activation); after Esc the browser may want a second before it locks again; a press in the field locks too; Firefox shows its own note at the top |

The pads per platform: Windows reads XInput (Xbox-compatible pads only; a PlayStation pad shows
up only through Steam Input or a driver that makes it an Xbox pad), Linux the evdev nodes of game
controllers (an Xbox pad over USB through xpad, a PlayStation pad through hid-playstation, a
Switch Pro pad through hid-nintendo; the KDE desktop gives the logged-in user access to them, in
a Flatpak they need `--device=input`), macOS GameController (Xbox, PlayStation, Switch Pro and MFi
pads over USB or Bluetooth), the browser the Gamepad API (Chrome maps known pads to the standard
layout; Firefox on Linux may show a pad with *eigene Belegung*, then the names do not fit).
