#!/usr/bin/env python3
"""extract the literal tables of the BC assembly sources into lib/img_dds/tables.rs (r9-dds)

    python tools/gen_dds.py
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "ASMLIB" / "src" / "img"


def table(fname, label):
    lines = (SRC / fname).read_text(encoding="utf-8").splitlines()
    vals = []
    kind = None
    on = False
    for ln in lines:
        s = ln.split("#")[0].strip()
        if not s:
            continue
        m = re.match(r"^([A-Za-z_][A-Za-z0-9_]*):\s*(.*)$", s)
        if m:
            if on:
                break
            if m.group(1) == label:
                on = True
                s = m.group(2).strip()
                if not s:
                    continue
            else:
                continue
        if not on:
            continue
        if s.startswith((".p2align", ".globl", ".section", ".if", ".error", ".endif", ".set", ".rept", ".endr", ".macro")):
            if s.startswith((".section", ".macro")):
                break
            continue
        m = re.match(r"^\.(byte|short|long|quad)\s+(.*)$", s)
        if not m:
            break
        k = m.group(1)
        if kind is None:
            kind = k
        for v in m.group(2).split(","):
            v = v.strip()
            vals.append(int(v, 0))
    return kind, vals


def emit(name, rtype, vals, per=16, hexfmt=False):
    out = [f"pub static {name}: [{rtype}; {len(vals)}] = ["]
    for i in range(0, len(vals), per):
        row = vals[i:i + per]
        if hexfmt:
            out.append("    " + ", ".join(f"0x{v & 0xFFFFFFFF:08X}" if rtype == "u32" else f"0x{v & 0xFFFF:04X}" for v in row) + ",")
        else:
            out.append("    " + ", ".join(str(v) for v in row) + ",")
    out.append("];")
    return "\n".join(out)


def main():
    parts = []
    hdr = """// tables -- literal tables of the BC codecs (extracted from the assembly by a script)
//                                  [src/img/bc_dec.s, bc_enc.s, bc6h_enc.s, bc7_enc.s]
//
//   PART2 / PART3     BC6H / BC7 partitions, 2 bits per pixel (pixel i at bits 2i+1..2i)
//   ANC2 / ANC3A / ANC3B   anchor pixel of subset 1 (2 subsets), subsets 1 and 2 (3 subsets)
//   BC6_MODEMAP       5-bit BC6H mode value -> mode index 0..13 (255 reserved)
//   BC6_MODES         per mode index: transformed, endpoint bits, delta bits R G B, header
//                     bits after the mode bits, u16 offset into BC6_LAYOUT
//   BC6_LAYOUT        BC6H header bits in stream order: field * 16 + bit
//   S4_5 / S3_5 / S4_6 / S3_6   single-colour BC1 tables (a, b, |error|) per 8-bit value
//   H6_HALF           IEEE half of v / 255 (round to nearest even)
//   B7_QT             BC7 endpoint codes per 8-bit value: e = 5..8 x {any, p = 0, p = 1}
//   B7_SINGLE         mode 5 colour endpoints (lo, hi) whose index-1 mix is the value
//   B7_M2 / B7_M3     pixel masks of subset 1 (2 subsets), subsets 1 and 2 (3 subsets)
"""
    parts.append(hdr)
    spec = [
        ("bc_dec.s", "bc_part2", "PART2", "u32", 4, True),
        ("bc_dec.s", "bc_part3", "PART3", "u32", 4, True),
        ("bc_dec.s", "bc_anc2", "ANC2", "u8", 16, False),
        ("bc_dec.s", "bc_anc3a", "ANC3A", "u8", 16, False),
        ("bc_dec.s", "bc_anc3b", "ANC3B", "u8", 16, False),
        ("bc_dec.s", "bc6_modemap", "BC6_MODEMAP", "u8", 16, False),
        ("bc_dec.s", "bc6_layout", "BC6_LAYOUT", "u8", 20, False),
        ("bc_enc.s", "bc1_s4_5", "S4_5", "u8", 24, False),
        ("bc_enc.s", "bc1_s3_5", "S3_5", "u8", 24, False),
        ("bc_enc.s", "bc1_s4_6", "S4_6", "u8", 24, False),
        ("bc_enc.s", "bc1_s3_6", "S3_6", "u8", 24, False),
        ("bc6h_enc.s", "h6_half", "H6_HALF", "u16", 8, True),
        ("bc7_enc.s", "b7_qt", "B7_QT", "u8", 16, False),
        ("bc7_enc.s", "b7_single", "B7_SINGLE", "u8", 16, False),
        ("bc7_enc.s", "b7_m2", "B7_M2", "u16", 8, True),
        ("bc7_enc.s", "b7_m3", "B7_M3", "u16", 8, True),
    ]
    for f, label, name, rt, per, hx in spec:
        kind, vals = table(f, label)
        parts.append(emit(name, rt, vals, per, hx))
    # BC6H modes: 6 bytes + u16 each
    lines = (SRC / "bc_dec.s").read_text(encoding="utf-8").splitlines()
    i = next(k for k, l in enumerate(lines) if l.startswith("bc6_modes:"))
    modes = []
    cur = None
    for l in lines[i + 1:]:
        s = l.split("#")[0].strip()
        if s.startswith(".byte"):
            cur = [int(v.strip(), 0) for v in s[5:].split(",")]
        elif s.startswith(".short"):
            cur.append(int(s[6:].strip(), 0))
            modes.append(cur)
            cur = None
        elif s.endswith(":"):
            break
    assert len(modes) == 14, modes
    out = ["pub static BC6_MODES: [[u16; 7]; 14] = ["]
    for m in modes:
        out.append("    [" + ", ".join(str(v) for v in m) + "],")
    out.append("];")
    parts.append("\n".join(out))
    text = "\n\n".join(parts) + "\n"
    dst = ROOT / "lib" / "img_dds" / "tables.rs"
    dst.write_bytes(text.encode("utf-8"))
    print(f"wrote {dst} ({len(text)} bytes)")
    # sanity
    for f, label, name, rt, per, hx in spec:
        kind, vals = table(f, label)
        print(name, len(vals))
    a, b = table("bc_dec.s", "bc_anc3a")[1], table("bc_dec.s", "bc_anc3b")[1]
    print("anc3 equal pairs:", [k for k in range(64) if a[k] == b[k]])


if __name__ == "__main__":
    main()
