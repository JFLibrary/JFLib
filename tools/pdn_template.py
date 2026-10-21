#!/usr/bin/env python3
"""Extract the NRBF class skeleton of a real paint.net file into ASMLIB/src/img/pdn.s.

  python tools/pdn_template.py [--src apps/PaintCraft/tests/fixtures/pdn/a_1x1.pdn] [--print]

The writer in pdn.s (pdn_encode) emits the object graph paint.net 5.1 writes; the class
records (names, member names, member types, library references), the library names, the
KeyValuePair array element type and the savedWith version are copied byte for byte from the
source file by this tool, into the block between the lines

    # BEGIN pdn_template.py
    # END pdn_template.py

of pdn.s (replaced in place; --print writes the block to stdout instead).

Template class record (rodata, label pt_c_<name>):
    u8  slot            index into the writer's "first instance" table (NW_FIRST)
    u8  record type     4 SystemClassWithMembersAndTypes / 5 ClassWithMembersAndTypes
    u8  patch count     library ids inside the body
    u8  0
    u32 body length     bytes after the object id: ClassInfo name .. MemberTypeInfo .. LibraryId
    patches             u16 offset in the body, u8 library index (0 Data, 1 Core), u8 0
    body                with every library id zeroed
Libraries: pt_libs = .quad pt_lib0, pt_lib1; each {u32 n, n bytes = LengthPrefixedString}.
The 13 other UserBlendOps classes are derived from NormalBlendOp (same library, no members).
"""

import argparse
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import pdn_check as pc  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
PDN_S = ROOT.parent / "legacy" / "ASMLIB" / "src" / "img" / "pdn.s"
BEGIN = "# BEGIN pdn_template.py"
END = "# END pdn_template.py"

# class name -> (label, slot)
CLASSES = [
    ("PaintDotNet.Document", "doc"),
    ("PaintDotNet.LayerList", "ll"),
    ("System.Version", "ver"),
    ("System.Collections.Generic.KeyValuePair`2", "kvp"),
    ("PaintDotNet.BitmapLayer", "bl"),
    ("PaintDotNet.BitmapLayer+BitmapLayerProperties", "blp"),
    ("PaintDotNet.Surface", "surf"),
    ("PaintDotNet.Layer+LayerProperties", "lp"),
    ("PaintDotNet.LayerBlendMode", "bm"),
    ("PaintDotNet.MemoryBlock", "mb"),
]
# slots: doc ll ver kvp bl surf lp bm, blp0..13, bo0..13, mb
BLEND = ["Normal", "Multiply", "Additive", "ColorBurn", "ColorDodge", "Reflect", "Glow", "Overlay",
         "Difference", "Negation", "Lighten", "Darken", "Screen", "Xor"]


def lps(b):
    out = bytearray()
    n = len(b)
    while True:
        c = n & 0x7F
        n >>= 7
        out.append(c | (0x80 if n else 0))
        if not n:
            break
    return bytes(out) + b


def byte_lines(b, ind="        "):
    lines = []
    for i in range(0, len(b), 16):
        lines.append(ind + ".byte   " + ", ".join("0x%02x" % x for x in b[i:i + 16]))
    return lines


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--src", default=str(pc.SAMPLES / "a_1x1.pdn"))
    ap.add_argument("--print", action="store_true")
    a = ap.parse_args()
    data = Path(a.src).read_bytes()
    doc = pc.read_pdn(data)
    rd = doc.nrbf
    d = rd.d
    # library index by name
    libidx = {}
    libnames = [None, None]
    for lid, name in rd.libs.items():
        k = 0 if name.startswith("PaintDotNet.Data,") else 1 if name.startswith("PaintDotNet.Core,") else None
        if k is None:
            raise SystemExit(f"unexpected library {name}")
        libidx[lid] = k
        libnames[k] = name
    if None in libnames:
        raise SystemExit("both PaintDotNet.Data and PaintDotNet.Core are needed")

    def cls_record(oid):
        rt, b0, b1, libpos = rd.spans[oid]
        body = bytearray(d[b0:b1])
        patches = []
        for p in libpos:
            lid = struct.unpack_from("<i", d, p)[0]
            patches.append((p - b0, libidx[lid]))
            body[p - b0:p - b0 + 4] = b"\0\0\0\0"
        return rt, bytes(body), patches

    found = {}
    for oid, (rt, b0, b1, libpos) in rd.spans.items():
        cls = rd.classes[oid]
        for name, label in CLASSES:
            if cls.name == name or (label == "kvp" and cls.name.startswith(name)):
                found.setdefault(label, oid)
        if cls.name == "PaintDotNet.UserBlendOps+NormalBlendOp":
            found.setdefault("bo0", oid)
    missing = [lb for _, lb in CLASSES if lb not in found] + ([] if "bo0" in found else ["bo0"])
    if missing:
        raise SystemExit(f"classes missing in {a.src}: {missing}")
    # KVP[] element type info (BinaryArray tail) of the document metadata array
    kvp_tail = None
    for oid, (t0, t1) in rd.arr_tails.items():
        arr = rd.objs[oid]
        if isinstance(arr.etype, tuple) and arr.etype[0] == pc.BT_SYSCLASS and "KeyValuePair" in arr.etype[1]:
            kvp_tail = d[t0:t1]
            break
    if kvp_tail is None:
        raise SystemExit("no KeyValuePair array")
    ver = doc.saved_with
    out = [BEGIN + f" (generated from {Path(a.src).name}, savedWith {'.'.join(map(str, ver))};"
           " do not edit)"]
    slots = ([lb for _, lb in CLASSES if lb not in ("blp", "mb")] + ["blp%d" % i for i in range(14)]
             + ["bo%d" % i for i in range(14)] + ["mb"])
    for i, lb in enumerate(slots):
        out.append(f"PTS_{lb.upper():8s}= {i}")
    out.append(f"PT_NSLOT       = {len(slots)}")
    out.append("PT_VERSTR_LEN  = %d" % len(".".join(map(str, ver))))
    out.append("        .section .rodata")
    out.append("        .p2align 3")
    out.append("pt_libs:        .quad   pt_lib0, pt_lib1")
    out.append("pt_blendop:     .quad   " + ", ".join("pt_c_bo%d" % i for i in range(14)))
    out.append("pt_blp:         .quad   " + ", ".join("pt_c_blp%d" % i for i in range(14)))
    out.append("pt_version:     .long   %d, %d, %d, %d" % ver)
    out.append('pt_verstr:      .asciz  "%s"' % ".".join(map(str, ver)))
    for k in range(2):
        b = lps(libnames[k].encode("utf-8"))
        out.append(f"pt_lib{k}:        .long   {len(b)}")
        out.extend(byte_lines(b))
    out.append(f"pt_kvp_tail:    .long   {len(kvp_tail)}")
    out.extend(byte_lines(kvp_tail))

    def emit(label, slot, rt, body, patches):
        out.append(f"        .p2align 2")
        out.append(f"pt_c_{label}:")
        out.append(f"        .byte   {slot}, {rt}, {len(patches)}, 0")
        out.append(f"        .long   {len(body)}")
        for off, k in patches:
            out.append(f"        .short  {off}")
            out.append(f"        .byte   {k}, 0")
        out.extend(byte_lines(body))

    for lb in slots:
        if lb.startswith("bo") and lb != "bo0" or lb.startswith("blp"):
            continue
        rt, body, patches = cls_record(found[lb])
        emit(lb, slots.index(lb), rt, body, patches)
    # BitmapLayerProperties: the member type of blendOp names the UserBlendOps class of the
    # layer, so there is one variant per blend mode (.NET writes a new class record when the
    # member types differ)
    rt, body, patches = cls_record(found["blp"])
    old = lps(b"PaintDotNet.UserBlendOps+NormalBlendOp")
    at = body.find(old)
    if rt != 5 or at < 0 or body.count(old) != 1:
        raise SystemExit("unexpected BitmapLayerProperties record")
    for i in range(14):
        new = lps(f"PaintDotNet.UserBlendOps+{BLEND[i]}BlendOp".encode())
        nb = body[:at] + new + body[at + len(old):]
        delta = len(new) - len(old)
        np_ = [(off + delta if off > at else off, k) for off, k in patches]
        emit(f"blp{i}", slots.index(f"blp{i}"), rt, nb, np_)
    # the other blend ops: name, 0 members, library (same as NormalBlendOp)
    rt0, body0, patches0 = cls_record(found["bo0"])
    if rt0 != 5 or len(patches0) != 1:
        raise SystemExit("unexpected NormalBlendOp record")
    for i in range(1, 14):
        body = lps(f"PaintDotNet.UserBlendOps+{BLEND[i]}BlendOp".encode()) + struct.pack("<ii", 0, 0)
        emit(f"bo{i}", slots.index(f"bo{i}"), 5, body, [(len(body) - 4, patches0[0][1])])
    out.append("        .text")
    out.append(END)
    block = "\n".join(out) + "\n"
    if a.print:
        sys.stdout.write(block)
        return 0
    src = PDN_S.read_text(encoding="utf-8")
    i, j = src.find(BEGIN), src.find(END)
    if i < 0 or j < 0:
        raise SystemExit(f"markers not found in {PDN_S}")
    j = src.index("\n", j) + 1
    PDN_S.write_text(src[:i] + block + src[j:], encoding="utf-8", newline="\n")
    print(f"pdn_template: {len(slots)} class slots written to {PDN_S.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
