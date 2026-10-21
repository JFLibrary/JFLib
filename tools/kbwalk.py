#!/usr/bin/env python3
"""kbwalk -- walk an app with the keyboard (headless) and list where the focus goes.

    python tools/kbwalk.py build/win/gallery.exe
    python tools/kbwalk.py build/win/PaintCraft.exe --size "1280 800" --keys ctrl+f6 -n 8
    python tools/kbwalk.py build/win/PaintCraft.exe --pre "cmd 271" --pre "wait 300" -n 30
    python tools/kbwalk.py EXE --pre-file setup.uis --keys f8 -n 10 --args --lang de

The script it writes runs the setup lines (--pre / --pre-file), then presses the keys
(--keys, default "tab"; several = one chord sequence per step) n times and dumps the widget
tree after every step (build/kbwalk/<name>/NNN.txt).  The listing shows for every step the
window the keys go to (the dump's "keys"), its focused widget (id, kind, label, rect in DIP)
and marks:

    LOST      no widget of the keyboard window has the focus
    NOREC     the focus id has no semantic record (a custom hit box: invisible to screen
              readers, `expect S focused` cannot see it)
    OFF       the focused widget lies outside its window (not scrolled into view)
    SAME      the step did not move the focus
    CYCLE     the focus came back to the first widget (the Tab ring is complete)

At the end it lists the enabled widgets of the base layer that some dump showed but the walk
never focused -- the members of a focus group other than its entry are among them (the arrows
reach them; the dump does not know the groups).
Exit code 0; it is an audit aid, not a test (tests are .uis scripts with `expect S focused`).
"""
import argparse
import json
import os
import subprocess
import sys

# kinds a user acts on (their records should be reachable by the keyboard)
ACTIVE = {"button", "checkbox", "radio", "switch", "slider", "textbox", "menuitem", "list", "listitem", "tab",
          "combo", "swatch", "link", "canvas"}

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def parse_dump(path):
    head, wins, recs = None, [], []
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                o = json.loads(line)
            except ValueError:
                continue
            if head is None and "frame" in o:
                head = o
            elif "window" in o:
                wins.append(o)
            else:
                recs.append(o)
    return head or {}, wins, recs


def focus_of(head, wins, recs):
    """-> (window id, focus id, record or None, window size)"""
    active = head.get("keys", 0)  # the window the keys go to (0 = main)
    size = head.get("size", [0, 0])
    fid = head.get("focus", 0)
    for w in wins:
        if w["window"] == active:
            size = w.get("size", size)
            fid = w.get("focus", 0)
    rec = None
    for r in recs:
        if r.get("w", 0) != active:
            continue
        if "focused" in r.get("s", []) or (fid and r.get("id") == fid):
            rec = r
            if "focused" in r.get("s", []):
                break
    return active, fid, rec, size


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("exe")
    ap.add_argument("--pre", action="append", default=[], help="a script line run before the walk")
    ap.add_argument("--pre-file", help="script lines run before the walk")
    ap.add_argument("--keys", nargs="+", default=["tab"], help="key chords pressed per step")
    ap.add_argument("-n", type=int, default=60, help="steps")
    ap.add_argument("--name", help="output folder name (default: exe name)")
    ap.add_argument("--size", default="900 620", help="window size W H (DIP)")
    ap.add_argument("--all", action="store_true", help="do not stop at the first CYCLE")
    ap.add_argument("--args", nargs=argparse.REMAINDER, default=[], help="further app arguments")
    a = ap.parse_args()

    name = a.name or os.path.splitext(os.path.basename(a.exe))[0]
    out = os.path.join(ROOT, "build", "kbwalk", name)
    os.makedirs(out, exist_ok=True)
    for f in os.listdir(out):
        if f.endswith(".txt"):
            os.remove(os.path.join(out, f))
    rel = os.path.relpath(out, ROOT).replace("\\", "/")
    lines = ["size " + a.size]
    if a.pre_file:
        with open(a.pre_file, encoding="utf-8") as f:
            lines += [l.rstrip("\n") for l in f if l.strip() and not l.startswith("#!")]
    lines += a.pre
    lines.append("dump %s/000.txt" % rel)
    for i in range(1, a.n + 1):
        lines.append("key " + " ".join(a.keys))
        lines.append("dump %s/%03d.txt" % (rel, i))
    lines.append("quit")
    script = os.path.join(out, "walk.uis")
    with open(script, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(lines) + "\n")
    r = subprocess.run([os.path.abspath(a.exe), "--script", script] + a.args, cwd=ROOT, capture_output=True, text=True,
                       encoding="utf-8", errors="replace", timeout=600)
    for l in (r.stdout + r.stderr).splitlines():
        if "FAIL" in l or "UIWARN" in l or "error" in l.lower():
            print("  app: " + l)

    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    first = None
    prev = None
    seen, reached = {}, set()
    for i in range(0, a.n + 1):
        p = os.path.join(out, "%03d.txt" % i)
        if not os.path.exists(p):
            print("%03d  (no dump)" % i)
            break
        head, wins, recs = parse_dump(p)
        win, fid, rec, size = focus_of(head, wins, recs)
        for r in recs:
            if r.get("k") in ACTIVE and r.get("id") and "disabled" not in r.get("s", []) and r.get("l", 0) == 0:
                seen.setdefault((r.get("w", 0), r["id"]), r)
        if rec is not None:
            reached.add((win, rec["id"]))
        marks = []
        if rec is None and not fid:
            marks.append("LOST")
            desc = "-"
        elif rec is None:
            marks.append("NOREC")
            desc = "#%d" % fid
        else:
            r = rec["r"]
            desc = "#%d %s \"%s\" [%d,%d,%d,%d]" % (rec["id"], rec["k"], rec["t"][:40], r[0], r[1], r[2], r[3])
            if r[0] + r[2] <= 0 or r[1] + r[3] <= 0 or r[0] >= size[0] or r[1] >= size[1]:
                marks.append("OFF")
        key = (win, fid, rec["id"] if rec else 0)
        if i > 0 and key == prev:
            marks.append("SAME")
        if i > 0 and first is not None and key == first:
            marks.append("CYCLE")
        print("%03d  w%-5s %s %s" % (i, win, desc, " ".join(marks)))
        if i == 1:
            first = key
        prev = key
        if "CYCLE" in marks and not a.all:
            break
    miss = [r for k, r in seen.items() if k not in reached]
    if miss:
        print("never focused (enabled, base layer, seen in a dump):")
        for r in miss:
            print("     w%-5s #%d %s \"%s\" [%s]" % (r.get("w", 0), r["id"], r["k"], r["t"][:40], ",".join(str(v) for v in r["r"])))
    return 0


if __name__ == "__main__":
    sys.exit(main())
