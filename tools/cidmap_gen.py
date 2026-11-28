#!/usr/bin/env python3
"""cidmap_gen.py -- lib/cidmap/data/cidmap.bin from Adobe's CMap resources

The sources, fetched once into build/vectors/ (not in git), all Adobe's (BSD-3-Clause):
  - adobe-type-tools/cmap-resources at a pinned commit: the predefined CMaps of ISO 32000-1
    Table 118 for the character collections Adobe-GB1, -CNS1, -Japan1 and -Korea1 (and what they
    name with usecmap), each collection's cid2code.txt (for --check), LICENSE.md
  - the CID -> Unicode tables Adobe-GB1-UCS2, -CNS1-UCS2, -Japan1-UCS2 and -Korea1-UCS2 (Adobe's,
    2023, Japan1 7), which that repository no longer holds: from the poppler-data 0.4.12 release
    tarball, pinned by its SHA-256

What the file holds (lib/cidmap/src/lib.rs reads it):
  - per CMap: its writing mode, the CMap it uses (usecmap), its code space, its CID ranges and
    characters (as ranges of length 1), its notdef ranges
  - per collection: CID -> Unicode text (a code point, or a sequence of them)
The parts of a collection share one zstd frame (level 19): a reader unpacks a collection once;
see FORMAT below.

  python tools/cidmap_gen.py            write lib/cidmap/data/cidmap.bin
  python tools/cidmap_gen.py --check [--exe build/linux/pdftest]
        the file holds what the sources give (its frames unpacked, so another zstd does not
        matter); every CMap as pdftest --cidmap NAME decodes it equals Adobe's file, and every
        table as pdftest --cidmap Adobe-XX-UCS2 gives it equals the source; only reported: how the
        copies of poppler-data and Ghostscript installed here differ, and how the tables differ
        from cid2code.txt's UniXX-UTF32 column

FORMAT (little-endian)
  "CIDM" 1; u8 frames, per frame u32 its length unpacked, u32 offset in the file, u32 length;
  u16 count, then count entries: u8 kind (0 CMap, 1 CID -> Unicode), u8 collection (0 GB1, 1
  CNS1, 2 Japan1, 3 Korea1; its frame), u8 name length, name, u32 offset in the unpacked frame,
  u32 length; then the frames.
  A CMap: u8 WMode, u16 index of the CMap it uses (0xFFFF none); varint n, n code space ranges
  (u8 length, low bytes, high bytes); varint n, n CID ranges sorted by (length, low): u8 length,
  varint low - the previous range's high - 1 (low itself for a new length), varint high - low,
  varint zigzag(CID - the CID after the previous range); varint n, n notdef ranges: u8 length,
  varint low, varint high - low, varint CID.
  CID -> Unicode: varint n, then per CID 0 to n - 1 a varint t: 0 none; 1 a sequence (varint
  k, k code points); else one code point, zigzag(it - the previous one - 1) = t - 2 (the previous
  one: of the last single code point, -1 at first).
"""
import argparse
import hashlib
import re
import struct
import subprocess
import sys
import tarfile
import urllib.request
from compression import zstd
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
from suite_fetch import fetch  # noqa: E402

REPO = ("adobe-type-tools/cmap-resources", "f5cf3bca7fdfeaceb77aa82847e974f2306c20b4")
POPPLER_DATA = ("https://poppler.freedesktop.org/poppler-data-0.4.12.tar.gz", "c835b640a40ce357e1b83666aabd95edffa24ddddd49b8daff63adb851cdab74")
POPPLER_DIR = ROOT / "build" / "vectors" / "poppler-data-0.4.12"
OUT = ROOT / "lib" / "cidmap" / "data" / "cidmap.bin"
# collection: (the Ordering, its folder at the commit, the prefix of its Unicode columns, Table 118)
COLLECTIONS = [
    ("GB1", "Adobe-GB1-6", "UniGB", """GB-EUC-H GB-EUC-V GBpc-EUC-H GBpc-EUC-V GBK-EUC-H GBK-EUC-V GBKp-EUC-H GBKp-EUC-V
        GBK2K-H GBK2K-V UniGB-UCS2-H UniGB-UCS2-V UniGB-UTF16-H UniGB-UTF16-V"""),
    ("CNS1", "Adobe-CNS1-7", "UniCNS", """B5pc-H B5pc-V HKscs-B5-H HKscs-B5-V ETen-B5-H ETen-B5-V ETenms-B5-H ETenms-B5-V
        CNS-EUC-H CNS-EUC-V UniCNS-UCS2-H UniCNS-UCS2-V UniCNS-UTF16-H UniCNS-UTF16-V"""),
    ("Japan1", "Adobe-Japan1-7", "UniJIS", """83pv-RKSJ-H 90ms-RKSJ-H 90ms-RKSJ-V 90msp-RKSJ-H 90msp-RKSJ-V 90pv-RKSJ-H
        Add-RKSJ-H Add-RKSJ-V EUC-H EUC-V Ext-RKSJ-H Ext-RKSJ-V H V UniJIS-UCS2-H UniJIS-UCS2-V UniJIS-UCS2-HW-H
        UniJIS-UCS2-HW-V UniJIS-UTF16-H UniJIS-UTF16-V"""),
    ("Korea1", "Adobe-Korea1-2", "UniKS", """KSC-EUC-H KSC-EUC-V KSCms-UHC-H KSCms-UHC-V KSCms-UHC-HW-H KSCms-UHC-HW-V
        KSCpc-EUC-H UniKS-UCS2-H UniKS-UCS2-V UniKS-UTF16-H UniKS-UTF16-V"""),
]
INSTALLED = Path("/usr/share/poppler/cMap")


def parse_cmap(text, name):
    """a CMap file -> dict: wmode, ordering, use, codespace [(len, lo, hi)], ranges and notdef
    [(len, lo, hi, cid)]"""
    m = {"name": name, "wmode": 0, "ordering": None, "use": None, "codespace": [], "ranges": [], "notdef": []}
    w = re.search(r"/WMode\s+(\d+)\s+def", text)
    if w:
        m["wmode"] = int(w.group(1))
    o = re.search(r"/Ordering\s+\((\w+)\)", text)
    if o:
        m["ordering"] = o.group(1)
    u = re.search(r"/([\w.-]+)\s+usecmap", text)
    if u:
        m["use"] = u.group(1)
    for blk in re.findall(r"begincodespacerange(.*?)endcodespacerange", text, re.S):
        for lo, hi in re.findall(r"<([0-9A-Fa-f]+)>\s*<([0-9A-Fa-f]+)>", blk):
            m["codespace"].append((len(lo) // 2, bytes.fromhex(lo), bytes.fromhex(hi)))
    for blk in re.findall(r"begincidrange(.*?)endcidrange", text, re.S):
        for lo, hi, cid in re.findall(r"<([0-9A-Fa-f]+)>\s*<([0-9A-Fa-f]+)>\s*(\d+)", blk):
            m["ranges"].append((len(lo) // 2, int(lo, 16), int(hi, 16), int(cid)))
    for blk in re.findall(r"begincidchar(.*?)endcidchar", text, re.S):
        for c, cid in re.findall(r"<([0-9A-Fa-f]+)>\s*(\d+)", blk):
            m["ranges"].append((len(c) // 2, int(c, 16), int(c, 16), int(cid)))
    for blk in re.findall(r"beginnotdefrange(.*?)endnotdefrange", text, re.S):
        for lo, hi, cid in re.findall(r"<([0-9A-Fa-f]+)>\s*<([0-9A-Fa-f]+)>\s*(\d+)", blk):
            m["notdef"].append((len(lo) // 2, int(lo, 16), int(hi, 16), int(cid)))
    m["ranges"].sort(key=lambda r: (r[0], r[1]))
    m["notdef"].sort(key=lambda r: (r[0], r[1]))
    for a, b in zip(m["ranges"], m["ranges"][1:]):
        if a[0] == b[0] and b[1] <= a[2]:
            sys.exit("cidmap_gen: %s: ranges overlap at %x" % (name, b[1]))
    return m


def ucs2_file(ordering):
    """Adobe-<ordering>-UCS2 out of the poppler-data tarball (downloaded once, checked)"""
    member = "poppler-data-0.4.12/cMap/Adobe-%s/Adobe-%s-UCS2" % (ordering, ordering)
    out = POPPLER_DIR / member
    if out.exists():
        return out
    POPPLER_DIR.mkdir(parents=True, exist_ok=True)
    tar = POPPLER_DIR / "poppler-data-0.4.12.tar.gz"
    if not tar.exists():
        print("cidmap_gen: %s" % POPPLER_DATA[0], file=sys.stderr)
        with urllib.request.urlopen(POPPLER_DATA[0], timeout=120) as r:
            tar.write_bytes(r.read())
    if hashlib.sha256(tar.read_bytes()).hexdigest() != POPPLER_DATA[1]:
        sys.exit("cidmap_gen: %s: not the pinned file (SHA-256)" % tar)
    with tarfile.open(tar) as t:
        t.extract(member, POPPLER_DIR, filter="data")
    return out


def ucs2_table(path):
    """an Adobe-XX-UCS2 CMap -> [text per CID] ('' none)"""
    t = path.read_text("latin-1")
    m = {}
    for blk in re.findall(r"beginbfchar(.*?)endbfchar", t, re.S):
        for a, b in re.findall(r"<([0-9A-Fa-f]+)>\s*<([0-9A-Fa-f]+)>", blk):
            m[int(a, 16)] = bytes.fromhex(b).decode("utf-16-be")
    for blk in re.findall(r"beginbfrange(.*?)endbfrange", t, re.S):
        for a, b, c in re.findall(r"<([0-9A-Fa-f]+)>\s*<([0-9A-Fa-f]+)>\s*<([0-9A-Fa-f]+)>", blk):
            first = bytes.fromhex(c).decode("utf-16-be")
            assert len(first) == 1, (path, a)
            for k in range(int(a, 16), int(b, 16) + 1):
                m[k] = chr(ord(first) + k - int(a, 16))
    n = max(m) + 1 if m else 0
    return [m.get(k, "") for k in range(n)]


def utf32_column(path, uni):
    """cid2code.txt -> [the first code point of the UniXX-UTF32 column per CID] (0 none)"""
    col = None
    out = []
    for line in path.read_text("utf-8").splitlines():
        if line.startswith("#") or not line.strip():
            continue
        f = line.split("\t")
        if col is None:
            col = f.index(uni + "-UTF32")
            continue
        v = f[col]
        out.append(0 if v == "*" else int(v.split(",")[0].rstrip("v"), 16))
    return out


def varint(v, out):
    assert v >= 0
    while v >= 0x80:
        out.append(v & 0x7F | 0x80)
        v >>= 7
    out.append(v)


def zigzag(v):
    return v * 2 if v >= 0 else -v * 2 - 1


def encode_cmap(m, index):
    out = bytearray([m["wmode"]])
    out += struct.pack("<H", index[m["use"]] if m["use"] else 0xFFFF)
    varint(len(m["codespace"]), out)
    for n, lo, hi in m["codespace"]:
        out.append(n)
        out += lo + hi
    varint(len(m["ranges"]), out)
    prev_len, prev_end, next_cid = None, 0, 0
    for n, lo, hi, cid in m["ranges"]:
        if n != prev_len:
            prev_len, prev_end = n, 0
        out.append(n)
        varint(lo - prev_end, out)
        varint(hi - lo, out)
        varint(zigzag(cid - next_cid), out)
        prev_end, next_cid = hi + 1, cid + (hi - lo) + 1
    varint(len(m["notdef"]), out)
    for n, lo, hi, cid in m["notdef"]:
        out.append(n)
        varint(lo, out)
        varint(hi - lo, out)
        varint(cid, out)
    return bytes(out)


def encode_unicode(table):
    out = bytearray()
    varint(len(table), out)
    prev = -1
    for s in table:
        if not s:
            out.append(0)
        elif len(s) == 1:
            cp = ord(s)
            varint(zigzag(cp - prev - 1) + 2, out)
            prev = cp
        else:
            out.append(1)
            varint(len(s), out)
            for ch in s:
                varint(ord(ch), out)
    return bytes(out)


def sources():
    """the parts of the file in its order [(kind, collection, name, payload)], the parsed CMaps,
    the CID -> Unicode tables"""
    cmaps = {}
    order = []
    for ci, (ordering, folder, uni, names) in enumerate(COLLECTIONS):
        todo = names.split()
        while todo:
            n = todo.pop(0)
            if n in cmaps:
                continue
            d = fetch(*REPO, [folder + "/CMap/" + n], quiet=True)
            m = parse_cmap((d / folder / "CMap" / n).read_text("latin-1"), n)
            if m["ordering"] != ordering:
                sys.exit("cidmap_gen: %s: Ordering %s, not %s" % (n, m["ordering"], ordering))
            m["collection"] = ci
            cmaps[n] = m
            order.append(n)
            if m["use"] and m["use"] not in cmaps:
                todo.append(m["use"])
    fetch(*REPO, ["LICENSE.md"], quiet=True)
    index = {n: i for i, n in enumerate(order)}
    parts = [(0, cmaps[n]["collection"], n, encode_cmap(cmaps[n], index)) for n in order]
    tables = {}
    for ci, (ordering, folder, uni, names) in enumerate(COLLECTIONS):
        tables[ordering] = ucs2_table(ucs2_file(ordering))
        parts.append((1, ci, "Adobe-%s-UCS2" % ordering, encode_unicode(tables[ordering])))
    return parts, cmaps, tables


def build(parts):
    unpacked = [b"" for _ in COLLECTIONS]
    entries = bytearray()
    for kind, ci, name, raw in parts:
        nb = name.encode()
        entries += bytes([kind, ci, len(nb)]) + nb + struct.pack("<II", len(unpacked[ci]), len(raw))
        unpacked[ci] += raw
    frames = [zstd.compress(u, level=19) for u in unpacked]
    size = 5 + 1 + 12 * len(frames) + 2 + len(entries)
    head = bytearray(b"CIDM\x01") + bytes([len(frames)])
    off = size
    for u, f in zip(unpacked, frames):
        head += struct.pack("<III", len(u), off, len(f))
        off += len(f)
    head += struct.pack("<H", len(parts)) + entries
    assert len(head) == size
    return bytes(head) + b"".join(frames)


def read(blob):
    """the file -> [(kind, collection, name, payload)]"""
    assert blob[:5] == b"CIDM\x01", "not a cidmap file"
    nf = blob[5]
    unpacked = []
    for i in range(nf):
        raw, off, ln = struct.unpack_from("<III", blob, 6 + 12 * i)
        u = zstd.decompress(blob[off:off + ln])
        assert len(u) == raw
        unpacked.append(u)
    p = 6 + 12 * nf
    (count,) = struct.unpack_from("<H", blob, p)
    p += 2
    out = []
    for _ in range(count):
        kind, ci, nl = blob[p], blob[p + 1], blob[p + 2]
        name = blob[p + 3:p + 3 + nl].decode()
        off, ln = struct.unpack_from("<II", blob, p + 3 + nl)
        p += 3 + nl + 8
        out.append((kind, ci, name, unpacked[ci][off:off + ln]))
    return out


def resolved(cmaps, name):
    """code -> CID of a CMap with what it uses: {(len, code): cid} (notdef ranges apart)"""
    m = cmaps[name]
    out = resolved(cmaps, m["use"]) if m["use"] else {}
    for n, lo, hi, cid in m["ranges"]:
        for c in range(lo, hi + 1):
            out[(n, c)] = cid + c - lo
    return out


def installed_copy(name):
    for c in INSTALLED.glob("*/" + name):
        return c
    gs = sorted(Path("/usr/share/ghostscript").glob("*/Resource/CMap/" + name))
    return gs[-1] if gs else None


def check(exe):
    parts, cmaps, tables = sources()
    fails = 0
    have = read(OUT.read_bytes())
    if have != parts:
        names = {n for _, _, n, _ in parts} ^ {n for _, _, n, _ in have}
        print("FAIL %s does not hold what the sources give (%s)" % (OUT.relative_to(ROOT), ", ".join(sorted(names)) or "payloads differ"))
        fails += 1
    print("cidmap: %d CMaps, %d CID -> Unicode tables, %d KB" % (len(cmaps), len(tables), OUT.stat().st_size // 1024))
    # the Rust decoder: pdftest --cidmap NAME prints "len code cid" per code of a CMap, "cid text"
    # (the code points in hex, joined by +) per CID of a table
    if exe and Path(exe).exists():
        bad = 0
        for n in cmaps:
            r = subprocess.run([exe, "--cidmap", n], capture_output=True, text=True)
            got = {}
            for line in r.stdout.splitlines():
                k, code, cid = line.split()
                got[(int(k), int(code, 16))] = int(cid)
            want = resolved(cmaps, n)
            if r.returncode != 0 or got != want:
                bad += 1
                print("FAIL %s: pdftest --cidmap gives %d codes, Adobe's file %d (%d differ)" % (n, len(got), len(want), sum(1 for k in want if got.get(k) != want[k])))
        for ordering, table in tables.items():
            n = "Adobe-%s-UCS2" % ordering
            r = subprocess.run([exe, "--cidmap", n], capture_output=True, text=True)
            got = {}
            for line in r.stdout.splitlines():
                cid, text = line.split()
                got[int(cid)] = "".join(chr(int(h, 16)) for h in text.split("+"))
            want = {k: s for k, s in enumerate(table) if s}
            if r.returncode != 0 or got != want:
                bad += 1
                print("FAIL %s: pdftest --cidmap gives %d CIDs, the source %d (%d differ)" % (n, len(got), len(want), sum(1 for k in want if got.get(k) != want[k])))
        print("decoder: %d of %d parts as their sources" % (len(cmaps) + len(tables) - bad, len(cmaps) + len(tables)))
        fails += bad
    else:
        print("decoder: skipped (no %s)" % exe)
    # the copies installed here (other versions of Adobe's data): only reported
    for n in cmaps:
        p = installed_copy(n)
        if not p:
            continue
        other = {n: parse_cmap(p.read_text("latin-1"), n)}
        u = other[n]["use"]
        while u and u not in other:
            q = installed_copy(u)
            if not q:
                break
            other[u] = parse_cmap(q.read_text("latin-1"), u)
            u = other[u]["use"]
        try:
            theirs = resolved(other, n)
        except KeyError:
            continue
        ours = resolved(cmaps, n)
        diff = sum(1 for k in set(ours) | set(theirs) if ours.get(k) != theirs.get(k))
        if diff:
            print("  %s: %d codes differ from %s" % (n, diff, p))
    for ordering, folder, uni, names in COLLECTIONS:
        d = fetch(*REPO, [folder + "/cid2code.txt"], quiet=True)
        col = utf32_column(d / folder / "cid2code.txt", uni)
        ours = tables[ordering]
        both = [k for k in range(min(len(col), len(ours))) if col[k] and ours[k]]
        diff = [k for k in both if ord(ours[k][0]) != col[k]]
        print("  Adobe-%s-UCS2: %d CIDs with text, %d of %d with a UTF-32 value in cid2code.txt differ from its first one" % (ordering, sum(1 for s in ours if s), len(diff), len(both)))
    print("%d failures" % fails)
    return 1 if fails else 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--exe", default=str(ROOT / "build" / "linux" / "pdftest"))
    a = ap.parse_args()
    if a.check:
        return check(a.exe)
    parts, cmaps, tables = sources()
    OUT.parent.mkdir(parents=True, exist_ok=True)
    blob = build(parts)
    OUT.write_bytes(blob)
    raw = sum(len(p) for _, _, _, p in parts)
    print("wrote %s: %d CMaps, %d tables, %d KB (%d KB unpacked)" % (OUT.relative_to(ROOT), len(cmaps), len(tables), len(blob) // 1024, raw // 1024))
    return 0


if __name__ == "__main__":
    sys.exit(main())
