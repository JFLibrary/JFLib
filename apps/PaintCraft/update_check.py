"""Embedded update query. Prints text only; never downloads or executes releases."""
import json
import re
import sys
import urllib.request

current, beta = sys.argv[1], sys.argv[2] == "1"
try:
    if len(sys.argv) > 3:
        with open(sys.argv[3], encoding="utf-8") as f:
            releases = json.load(f)
    else:
        req = urllib.request.Request(
            "https://api.github.com/repos/JFLibrary/JFLib/releases?per_page=100",
            headers={"User-Agent": "PaintCraft-update-check", "Accept": "application/vnd.github+json"})
        with urllib.request.urlopen(req, timeout=20) as response:
            payload = response.read(2_000_001)
        if len(payload) > 2_000_000:
            raise ValueError("Antwort zu gross")
        releases = json.loads(payload)
    def version(text):
        match = re.search(r"(?i)(?:paintcraft[- _v]*)?(\d+(?:\.\d+){1,3})", text)
        return tuple(map(int, match[1].split("."))) if match else ()
    candidates = []
    for release in releases:
        if release.get("draft") or (release.get("prerelease") and not beta):
            continue
        tag = release.get("tag_name", "")
        name = release.get("name", "")
        # The repository contains multiple apps: never announce another app's release.
        if "paintcraft" not in (tag + " " + name).lower():
            continue
        v = version(tag) or version(name)
        if v:
            candidates.append((v, tag))
    if not candidates:
        message = "Keine passende PaintCraft-Veroeffentlichung gefunden."
    else:
        newest, tag = max(candidates)
        pad = lambda v: v + (0,) * (4 - len(v))
        message = ("Neue PaintCraft-Version verfuegbar: " + tag if pad(newest) > pad(version(current))
                   else "PaintCraft ist aktuell (" + current + ").")
    sys.stdout.buffer.write(message.encode("utf-8"))
except Exception as error:
    sys.stderr.buffer.write(("Updatepruefung fehlgeschlagen: " + str(error)).encode("utf-8"))
    sys.exit(1)
