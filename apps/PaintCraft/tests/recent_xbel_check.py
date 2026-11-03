"""PaintCraft's files in the system's list of recent files (recently-used.xbel, on every OS through
jfgui_RECENT): opening a file adds it with PaintCraft as the program, Liste leeren takes only
PaintCraft's entries away, other programs' bookmarks stay.

Run from the repository root: python apps/PaintCraft/tests/recent_xbel_check.py [EXE]
"""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

FOREIGN = """<?xml version="1.0" encoding="UTF-8"?>
<xbel version="1.0" xmlns:bookmark="http://www.freedesktop.org/standards/desktop-bookmarks" xmlns:mime="http://www.freedesktop.org/standards/shared-mime-info">
  <bookmark href="file:///tmp/fremd.txt" added="2026-01-01T00:00:00Z" modified="2026-01-01T00:00:00Z" visited="2026-01-01T00:00:00Z">
    <info><metadata owner="http://freedesktop.org"><mime:mime-type type="text/plain"/>
      <bookmark:applications><bookmark:application name="Fremd" exec="&apos;fremd %u&apos;" modified="2026-01-01T00:00:00Z" count="1"/></bookmark:applications>
    </metadata></info>
  </bookmark>
</xbel>
"""
CLEAR = """theme light
click menu:"Datei"
move menuitem:"Zuletzt geöffnet"
click menuitem:"Liste leeren"
"""


def main():
    exe = sys.argv[1] if len(sys.argv) > 1 else "build/" + ("win/PaintCraft.exe" if os.name == "nt" else "linux/PaintCraft")
    exe = str(Path(exe).resolve())
    fails = 0

    def check(ok, what):
        nonlocal fails
        fails += not ok
        print(("ok   " if ok else "FAIL ") + what)

    with tempfile.TemporaryDirectory() as d:
        xbel = Path(d, "recent.xbel")
        xbel.write_text(FOREIGN, encoding="utf-8")
        script = Path(d, "clear.uis")
        script.write_text(CLEAR, encoding="utf-8")
        png = "apps/PaintCraft/tests/fixtures/a1_rgba.png"
        env = dict(os.environ, jfgui_RECENT=str(xbel))
        run = lambda *args: subprocess.run([exe, "--config", d, *args], env=env, capture_output=True, text=True, timeout=60)

        # a script without anything to do: the file of the command line is opened at the start
        empty = Path(d, "empty.uis")
        empty.write_text("theme light\n", encoding="utf-8")
        r = run("--open", png, "--script", str(empty))
        check(r.returncode == 0, f"open: exit {r.returncode} {r.stdout[:200]}")
        text = xbel.read_text(encoding="utf-8")
        entry = text.split("a1_rgba.png", 1)[1] if "a1_rgba.png" in text else ""
        check("a1_rgba.png" in text, "open: the file is in the list")
        check("PaintCraft" in entry.split("</bookmark>", 1)[0], "open: with PaintCraft as the program")
        check("fremd.txt" in text, "open: the other program's bookmark stays")

        r = run("--open", png, "--script", str(script))
        check(r.returncode == 0, f"Liste leeren: exit {r.returncode} {r.stdout[:300]}")
        text = xbel.read_text(encoding="utf-8")
        check("a1_rgba.png" not in text, "Liste leeren: PaintCraft's entry is gone")
        check("fremd.txt" in text and 'name="Fremd"' in text, "Liste leeren: the other program's bookmark stays")
    print("recent_xbel_check: " + ("ok" if not fails else f"{fails} failed"))
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
