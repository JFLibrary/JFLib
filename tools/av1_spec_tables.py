#!/usr/bin/env python3
"""Extract the constant tables of the AV1 specification into tools/av1_tables.py.

    python tools/av1_spec_tables.py [booksAndContext/specs/av1-spec.pdf]

The PDF (https://aomediacodec.github.io/av1-spec/av1-spec.pdf, kept locally, not in git) is read
with pypdf.  In its code blocks every run of digits / punctuation comes out twice ("1558815588,,"
for "15588,"); identifiers come out once.  The extractor undoes that, finds every
`Name[ dims ] = { ... }` table and writes nested Python lists (dimension expressions kept as text
for the size check of tools/av1_ref.py).
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "tools" / "av1_tables.py"
CACHE = ROOT / "build" / "av1" / "spec.txt"


def pdf_text(pdf):
    if CACHE.exists() and CACHE.stat().st_mtime > Path(pdf).stat().st_mtime:
        return CACHE.read_text(encoding="utf-8")
    import pypdf
    r = pypdf.PdfReader(str(pdf))
    txt = "\n".join("=== PAGE %d ===\n" % (i + 1) + p.extract_text() for i, p in enumerate(r.pages))
    CACHE.parent.mkdir(parents=True, exist_ok=True)
    CACHE.write_text(txt, encoding="utf-8")
    return txt


NOISE = re.compile(r"^(=== PAGE \d+ ===|AV1 Bitstream & Decoding Process Specification|Section: .* Page \d+ of \d+)$")


def undouble(line):
    """undo the doubled digit / punctuation runs of a code line"""
    out = []
    i = 0
    n = len(line)
    while i < n:
        c = line[i]
        if c.isalpha() or c == "_":
            j = i
            while j < n and (line[j].isalnum() or line[j] == "_"):
                j += 1
            out.append(line[i:j])
            i = j
        elif c.isdigit():
            j = i
            while j < n and line[j].isdigit():
                j += 1
            run = line[i:j]
            if len(run) % 2 == 0 and run[:len(run) // 2] == run[len(run) // 2:]:
                run = run[:len(run) // 2]
            out.append(run)
            i = j
        elif c.isspace():
            out.append(c)
            i += 1
        else:
            j = i
            while j < n and not (line[j].isalnum() or line[j] == "_" or line[j].isspace()):
                j += 1
            out.append(split_doubled(line[i:j]))
            i = j
    return "".join(out)


def split_doubled(run):
    """a punctuation run as consecutive doubled pieces T+T -> the Ts (run unchanged if impossible)"""
    def rec(i):
        if i == len(run):
            return ""
        for k in range(1, (len(run) - i) // 2 + 1):
            if run[i:i + k] == run[i + k:i + 2 * k]:
                r = rec(i + 2 * k)
                if r is not None:
                    return run[i:i + k] + r
        return None
    r = rec(0)
    return run if r is None else r


START = re.compile(r"^([A-Z][A-Za-z0-9_]*)\s*((?:\[[^\]]*\]\s*)+)=\s*\{(.*)$")


CONST = {}


def strip_comment(u):
    u = re.sub(r"/\*.*?\*/", " ", u)
    u = re.sub(r"//.*$", "", u)
    u = re.sub(r"(^|\s)/(\s.*)?$", "", u)
    return u


def extract(txt):
    lines = [l for l in txt.splitlines() if not NOISE.match(l.strip())]
    tables = {}
    i = 0
    while i < len(lines):
        raw = lines[i]
        # a table head broken over lines: join until "==" and "{{"
        if re.match(r"^[A-Z][A-Za-z0-9_]*\[\[", raw) and ("==" not in raw or "{{" not in raw.split("==", 1)[1]):
            k = i + 1
            while k < len(lines) and k < i + 4 and ("==" not in raw or "{{" not in raw.split("==", 1)[1]):
                raw = raw + " " + lines[k]
                k += 1
            if "==" in raw and "{{" in raw.split("==", 1)[1]:
                lines[i:k] = [raw]
            else:
                raw = lines[i]
        if "[[" in raw and "==" in raw:
            l = undouble(raw)
            m = START.match(l.strip())
            if m:
                name, dims, rest = m.group(1), m.group(2), m.group(3)
                body = ["{" + strip_comment(rest)]
                depth = body[0].count("{") - body[0].count("}")
                j = i + 1
                while depth > 0 and j < len(lines):
                    u = strip_comment(undouble(lines[j]))
                    body.append(u)
                    depth += u.count("{") - u.count("}")
                    j += 1
                # a missing comma at a line end ("TX_32X32" / "TX_4X8" in Split_Tx_Size)
                for q in range(len(body) - 1):
                    a_, b_ = body[q].rstrip(), body[q + 1].lstrip()
                    if a_ and b_ and (a_[-1].isalnum() or a_[-1] == "_") and (b_[0].isalnum() or b_[0] == "-"):
                        body[q] = a_ + ","
                text = " ".join(body)
                text = text[:text.rfind("}") + 1]
                py = text.replace("{", "[").replace("}", "]")
                try:
                    val = eval(py, {}, CONST)
                except Exception as e:  # noqa: BLE001
                    print(f"skip {name}: {e}", file=sys.stderr)
                    i += 1
                    continue
                dl = [d.strip() for d in re.findall(r"\[([^\]]*)\]", dims)]
                if name in tables:
                    print(f"duplicate {name}", file=sys.stderr)
                tables[name] = (dl, val)
                i = j
                continue
        i += 1
    return tables


def constants(txt):
    """symbol values: "NAME value description" (section 3) and "value NAME" enumeration rows"""
    c, conflicts = {}, []
    for l in txt.splitlines():
        l = l.strip()
        m = re.match(r"^([A-Z][A-Z0-9_]*[A-Z0-9_])\s+(-?\d+)(\s|$)", l)
        if m and ("_" in m.group(1) or len(m.group(1)) > 3):
            k, v = m.group(1), int(m.group(2))
        else:
            m = re.match(r"^(\d+)\s+([A-Z][A-Z0-9_]*_[A-Z0-9_]*)$", l)
            if m:
                k, v = m.group(2), int(m.group(1))
            else:
                m = re.match(r"^\d+\s+(\d+)\s+([A-Z][A-Z0-9_]*_[A-Z0-9_]*)$", l)
                if not m:
                    continue
                k, v = m.group(2), int(m.group(1))
        if k in c and c[k] != v:
            conflicts.append((k, c[k], v))
            continue
        c.setdefault(k, v)
    c.update(EXPR_CONST)
    return c, conflicts


# section-3 constants whose value column is an expression (the row regex sees only its first number)
EXPR_CONST = {"MAX_TILE_AREA": 4096 * 2304, "WARPEDMODEL_TRANS_CLAMP": 1 << 23,
              "WARPEDMODEL_NONDIAGAFFINE_CLAMP": 1 << 13, "WARPEDPIXEL_PREC_SHIFTS": 1 << 6,
              "SUPERRES_FILTER_SHIFTS": 1 << 6}


def flat_len(v):
    return sum(flat_len(x) for x in v) if isinstance(v, list) else 1


def main():
    pdf = sys.argv[1] if len(sys.argv) > 1 else ROOT.parent / "booksAndContext" / "specs" / "av1-spec.pdf"
    txt = pdf_text(pdf)
    c, conflicts = constants(txt)
    CONST.update(c)
    for k, a, b in conflicts:
        print(f"constant {k}: {a} / {b} (first kept)", file=sys.stderr)
    tables = extract(txt)
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write('"""Constant tables of the AV1 specification (generated by tools/av1_spec_tables.py from\n'
                'av1-spec.pdf; do not edit).  DIMS[name] = the dimension expressions of the spec."""\n\n')
        f.write("CONST = {\n")
        for k, v in sorted(CONST.items()):
            f.write(f"    {k!r}: {v},\n")
        f.write("}\n\n")
        f.write("DIMS = {\n")
        for k, (d, v) in tables.items():
            f.write(f"    {k!r}: {d!r},\n")
        f.write("}\n\n")
        for k, (d, v) in tables.items():
            f.write(f"{k} = {v!r}\n\n")
    print(f"{len(tables)} tables -> {OUT}")
    for k, (d, v) in sorted(tables.items()):
        print(f"  {k:45s} {str(d):60s} {flat_len(v)}")


if __name__ == "__main__":
    main()
