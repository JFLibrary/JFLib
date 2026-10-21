#!/usr/bin/env python3
# trcmp.py TAG_A TAG_B -- compare two inject_run.py app logs: the event lines exactly, the
# jfgui_WLDEBUG trace with serials / times masked
import re, sys

d = "build/plat-wayland"


def load(tag):
    ev, tr = [], []
    for ln in open(f"{d}/inj_{tag}_app.log", encoding="utf-8"):
        ln = ln.rstrip("\n")
        if ln.startswith("event type") or ln.startswith("  event"):
            ev.append(ln)
        elif ln.startswith("[wl]"):
            ln = re.sub(r"done\(\d+\)", "done(N)", ln)
            ln = re.sub(r"(configure|ping|pong|ack_configure|enter|leave|key|modifiers|button|motion|axis|frame|move|resize|set_cursor|set_selection)\((\d+)", r"\1(N", ln)
            tr.append(ln)
    return ev, tr


ea, ta = load(sys.argv[1])
eb, tb = load(sys.argv[2])
print("events:", "identical" if ea == eb else "DIFFER", len(ea), len(eb))
if ea != eb:
    for x, y in zip(ea, eb):
        if x != y:
            print("  a:", x)
            print("  b:", y)
            break
import difflib
diff = list(difflib.unified_diff(ta, tb, lineterm="", n=1))
print("trace lines:", len(ta), len(tb), "diff lines:", len(diff))
for ln in diff[:60]:
    print(ln)
