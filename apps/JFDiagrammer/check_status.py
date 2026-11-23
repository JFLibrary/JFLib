#!/usr/bin/env python3
"""check_status.py -- STATUS.md against FEATURES.md (JD15)

    python apps/JFDiagrammer/check_status.py           check: every P1 and P2 row of FEATURES.md
                                                       (the stencils of 4.2 one row each) is in
                                                       STATUS.md once, with its priority, a state
                                                       of fertig / teilweise / fehlt, a Beleg for
                                                       fertig and teilweise, what is missing for
                                                       teilweise; the scripts it names exist; the
                                                       summary is up to date
    python apps/JFDiagrammer/check_status.py --write   the same, and the summary between the
                                                       markers written anew (counts per section,
                                                       the P2 rows that are done)

Exit code 0: all well, 1: findings (one line each), 2: a file is missing.
"""

import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
FEATURES = HERE / "FEATURES.md"
STATUS = HERE / "STATUS.md"
STATES = ("fertig", "teilweise", "fehlt")
BEGIN = "<!-- summary:begin (check_status.py --write) -->"
END = "<!-- summary:end -->"
ID = re.compile(r"^[A-Z][A-Z0-9]?-[A-Z0-9]+$")
# the column that names a row, by the tables of FEATURES.md
# the headings of the sections of FEATURES.md by number ("3.12" -> "3.12 Zeichenfenster")
TITLES = {}
NAME_COLUMNS = ("Feature", "Schablone", "Befehl", "Abschnitt", "Assistent", "Vorlage", "Bereich", "Kategorie", "Register")


def table_rows(text):
    """(line, heading, cells) of every table row whose first cell is an ID, under a header row
    that starts with "ID" """
    heading, header = "", None
    for n, line in enumerate(text.splitlines(), 1):
        m = re.match(r"^#{2,4} (.*)", line)
        if m:
            heading, header = m.group(1).strip(), None
            continue
        if not line.startswith("|"):
            header = None if not line.strip() else header
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if cells and cells[0] == "ID":
            header = cells
            continue
        if header and cells and ID.match(cells[0]):
            yield n, heading, dict(zip(header, cells))


def section_of(heading):
    """"3.12 Zeichenfenster" -> "3.12" ("" when the heading has no number)"""
    m = re.match(r"^(\d+(?:\.\d+)?)\b", heading)
    return m.group(1) if m else ""


def wanted():
    """the P1 and P2 rows of FEATURES.md: {id: (prio, section, name)} in file order"""
    out = {}
    for _, heading, row in table_rows(FEATURES.read_text(encoding="utf-8")):
        sec = section_of(heading)
        TITLES.setdefault(sec, heading)
        prio = row.get("Prio", "")[:2]
        if prio not in ("P1", "P2") or sec.startswith("6") or sec.startswith("8"):
            continue
        name = next((row[k] for k in NAME_COLUMNS if row.get(k)), "")
        out[row["ID"]] = (prio, sec, name)
    return out


def summary(rows, want):
    """the summary block: counts per FEATURES section and the P2 rows that are done"""
    secs = []
    for _, (_, sec, _) in want.items():
        if sec not in secs:
            secs.append(sec)
    count = {}
    for rid, row in rows.items():
        if rid not in want:
            continue
        prio, sec, _ = want[rid]
        key = (sec, prio, row["Stand"])
        count[key] = count.get(key, 0) + 1
    head = "| Abschnitt | P1 fertig | P1 teilweise | P1 fehlt | P2 fertig | P2 teilweise | P2 fehlt |"
    lines = [head, "|---|---:|---:|---:|---:|---:|---:|"]
    total = {}
    for sec in secs:
        cells = []
        for prio in ("P1", "P2"):
            for st in STATES:
                c = count.get((sec, prio, st), 0)
                total[(prio, st)] = total.get((prio, st), 0) + c
                cells.append(str(c))
        lines.append(f"| {TITLES.get(sec, sec)} | " + " | ".join(cells) + " |")
    lines.append("| **zusammen** | " + " | ".join(f"**{total.get((p, s), 0)}**" for p in ("P1", "P2") for s in STATES) + " |")
    done = [rid for rid in want if want[rid][0] == "P2" and rows.get(rid, {}).get("Stand") == "fertig"]
    lines += ["", f"P2-Zeilen, die schon fertig sind ({len(done)}): " + (", ".join(done) if done else "keine") + "."]
    return "\n".join(lines)


def main():
    write = "--write" in sys.argv[1:]
    for f in (FEATURES, STATUS):
        if not f.exists():
            print(f"{f.name} fehlt")
            return 2
    want = wanted()
    text = STATUS.read_text(encoding="utf-8")
    rows, found = {}, []
    for n, _, row in table_rows(text):
        if "Stand" not in row:
            continue
        rid = row["ID"]
        if rid in rows:
            found.append(f"STATUS.md:{n}: {rid} steht doppelt")
            continue
        rows[rid] = row
        if rid not in want:
            found.append(f"STATUS.md:{n}: {rid} ist in FEATURES.md keine P1- oder P2-Zeile")
            continue
        if row.get("Prio", "")[:2] != want[rid][0]:
            found.append(f"STATUS.md:{n}: {rid} hat in FEATURES.md {want[rid][0]}, hier {row.get('Prio')}")
        st = row.get("Stand", "")
        if st not in STATES:
            found.append(f"STATUS.md:{n}: {rid}: Stand „{st}“ ist keiner von {', '.join(STATES)}")
        if st in ("fertig", "teilweise") and not row.get("Beleg", "").strip(" –-"):
            found.append(f"STATUS.md:{n}: {rid}: {st} ohne Beleg")
        if st == "teilweise" and not row.get("Fehlt", "").strip(" –-"):
            found.append(f"STATUS.md:{n}: {rid}: teilweise, aber Fehlt ist leer")
        for script in re.findall(r"([A-Za-z0-9_]+\.uis)", row.get("Beleg", "")):
            if not (HERE / "tests" / script).exists():
                found.append(f"STATUS.md:{n}: {rid}: tests/{script} gibt es nicht")
    for rid, (prio, sec, name) in want.items():
        if rid not in rows:
            found.append(f"STATUS.md: {rid} ({prio}, {sec} {name}) fehlt")
    block = summary(rows, want)
    if BEGIN not in text or END not in text:
        found.append("STATUS.md: die Marken der Zusammenfassung fehlen")
    else:
        a, rest = text.split(BEGIN, 1)
        old, b = rest.split(END, 1)
        if old.strip() != block.strip():
            if write:
                STATUS.write_text(a + BEGIN + "\n" + block + "\n" + END + b, encoding="utf-8", newline="\n")
                print("STATUS.md: Zusammenfassung neu geschrieben")
            else:
                found.append("STATUS.md: die Zusammenfassung ist nicht aktuell (check_status.py --write)")
    for f in found:
        print(f)
    print(f"check_status: {len(want)} Zeilen, {len(found)} Befunde")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
