#!/usr/bin/env python3
"""encoding_check.py -- formats::encoding against encoding_rs, Firefox's implementation of the WHATWG
Encoding Standard

    python tools/encoding_check.py            every check below
    python tools/encoding_check.py --quick    fewer random streams, no sweep of gb18030's four bytes

Ours is build/linux/coretest --encoding (python build_rs.py app coretest --target linux; on Windows
build/win/coretest.exe, --target win).  The reference is tools/encoding_ref.rs, built here with Cargo
offline against encoding_rs 0.8.35 or later from Cargo's registry (build/encoding_ref; nothing is
downloaded; 0.8.35 brought GB18030-2022).  Without cargo or such an encoding_rs nothing is compared and
the exit code is 2; a reference that does not build is a failure (exit 1).  Each run works in a folder of
its own under build/, removed when everything agreed.

For Shift_JIS, EUC-JP, ISO-2022-JP, GBK, gb18030, Big5 and EUC-KR:
  encode   every Unicode scalar value alone gives the same bytes or is refused by both; a text of every
           character the encoding has, strict (both succeed, the same bytes); a text with characters it
           lacks, strict (both refuse the same first one) and with character references
  decode   every two-byte sequence after a lead byte (EUC-JP: every three-byte one after 0x8F, ISO-2022-JP:
           every pair in each of its states, each framed by its escape and ESC ( B, gb18030: every
           four-byte one) and random streams of the encoding's bytes (with escape sequences for
           ISO-2022-JP, cut sequences, ASCII after leads), at once and in pieces of 0 to 7 bytes
and the decoders of UTF-8, UTF-16LE, UTF-16BE, windows-1252, IBM866, x-user-defined and the replacement
encoding on random streams, at once, in pieces and with the byte-order mark rule.  A program that fails
(an exit code other than the expected one) is a failed comparison, its output is never read.

One difference is expected: ISO-2022-JP cannot encode SO, SI and ESC; encode here reports the character
itself, encoding_rs U+FFFD (the standard's "error with U+FFFD", which keeps a character reference from
writing an escape).
"""
import os
import random
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MULTI = ["Shift_JIS", "EUC-JP", "ISO-2022-JP", "GBK", "gb18030", "Big5", "EUC-KR"]
OTHERS = ["UTF-8", "UTF-16LE", "UTF-16BE", "windows-1252", "IBM866", "x-user-defined", "iso-2022-kr"]
# the first encoding_rs with GB18030-2022
MIN_VERSION = (0, 8, 35)


class Unavailable(Exception):
    """cargo or a recent enough encoding_rs is not there: nothing can be compared"""


class BuildFailed(Exception):
    """the reference did not build"""


def _version(p):
    v = p.parent.name.split("-")[-1]
    return tuple(int(x) if x.isdigit() else 0 for x in v.split("."))


def reference():
    """build tools/encoding_ref.rs against encoding_rs from Cargo's registry, offline: the program.
    Unavailable without cargo or an encoding_rs from MIN_VERSION on, BuildFailed when it does not build."""
    reg = Path(os.environ.get("CARGO_HOME", Path.home() / ".cargo")) / "registry" / "src"
    found = sorted(reg.glob("*/encoding_rs-*/Cargo.toml"), key=_version) if reg.is_dir() else []
    if not shutil.which("cargo"):
        raise Unavailable("cargo is not on PATH")
    if not found:
        raise Unavailable("encoding_rs is not in Cargo's registry")
    recent = [f for f in found if _version(f) >= MIN_VERSION]
    if not recent:
        raise Unavailable(f"encoding_rs in Cargo's registry is {'.'.join(map(str, _version(found[-1])))}, older than "
                          f"{'.'.join(map(str, MIN_VERSION))} (GB18030-2022)")
    d = ROOT / "build" / "encoding_ref"
    (d / "src").mkdir(parents=True, exist_ok=True)
    toml = ('[package]\nname = "encoding_ref"\nversion = "0.0.0"\nedition = "2021"\npublish = false\n\n'
            f'[dependencies]\nencoding_rs = {{ path = "{recent[-1].parent.as_posix()}" }}\n\n[workspace]\n')
    for path, text in [(d / "Cargo.toml", toml), (d / "src" / "main.rs", (ROOT / "tools" / "encoding_ref.rs").read_text(encoding="utf-8"))]:
        if not path.exists() or path.read_text(encoding="utf-8") != text:
            path.write_text(text, encoding="utf-8")
    target = d / "target"
    r = subprocess.run(["cargo", "build", "--release", "--offline", "--quiet", "--manifest-path", str(d / "Cargo.toml"),
                        "--target-dir", str(target)])
    exe = target / "release" / ("encoding_ref.exe" if os.name == "nt" else "encoding_ref")
    if r.returncode != 0 or not exe.exists():
        raise BuildFailed(f"cargo build of {d / 'Cargo.toml'} failed (exit {r.returncode})")
    return exe


class Check:
    def __init__(self, ref, ours, work):
        self.ref, self.ours, self.work, self.bad, self.n = ref, ours, work, [], 0

    def run(self, exe, *args):
        r = subprocess.run([str(exe), *map(str, args)], capture_output=True, text=True)
        return r.returncode, r.stdout.strip(), r.stderr.strip()

    def output(self, what, exe, *args, out):
        """run a program that writes `out`; its bytes, or None (a failed comparison) when it did not end with 0"""
        if out.exists():
            out.unlink()
        rc, _, err = self.run(exe, *args)
        if rc != 0 or not out.exists():
            self.n += 1
            who = "encoding_rs" if exe == self.ref else "here"
            self.bad.append(f"{what}: {who} ended with exit {rc}" + (f": {err[-300:]}" if err else ""))
            return None
        return out.read_bytes()

    def same(self, what, a, b):
        self.n += 1
        if a == b:
            return True
        at = next((i for i in range(min(len(a), len(b))) if a[i] != b[i]), min(len(a), len(b)))
        self.bad.append(f"{what}: differ at byte {at} of the output ({len(a)} here, {len(b)} encoding_rs): "
                        f"here {a[max(0, at - 12):at + 12]!r}, encoding_rs {b[max(0, at - 12):at + 12]!r}")
        return False

    def decode(self, label, data, name, pieces=True, bom=False):
        f = self.work / f"{name}.bin"
        f.write_bytes(data)
        want_f, got_f = self.work / f"{name}.ref", self.work / f"{name}.ours"
        want = self.output(f"{label} {name} (reference)", self.ref, "decode-bom" if bom else "decode", label, f, want_f, out=want_f)
        if want is None:
            return
        modes = [("ds", 7)] if bom else [("d", None)] + ([("dc", 1), ("dc", 2)] if pieces else [])
        for op, seed in modes:
            got = self.output(f"{label} {name} ({op})", self.ours, "--encoding", op, label, f, got_f, *([seed] if seed else []), out=got_f)
            if got is None or not self.same(f"{label} {name} ({op})", got, want):
                break


def every_char():
    return "".join(chr(c) for c in range(0x110000) if not 0xD800 <= c <= 0xDFFF)


def pairs(leads, trails=range(256), after=b"A"):
    return b"".join(bytes([l, t]) + after for l in leads for t in trails)


def random_stream(rng, label, n):
    """bytes of the encoding's ranges, mixed: leads, trails, ASCII, cut sequences"""
    out = bytearray()
    while len(out) < n:
        k = rng.random()
        if label == "ISO-2022-JP":
            if k < 0.15:
                out += rng.choice([b"\x1b(B", b"\x1b(J", b"\x1b(I", b"\x1b$@", b"\x1b$B", b"\x1b(", b"\x1b$", b"\x1b", b"\x1b(X",
                                   b"\x1b$A", b"\x0e", b"\x0f"])
            elif k < 0.85:
                out.append(rng.randrange(0x21, 0x7F))
            elif k < 0.95:
                out.append(rng.randrange(0, 0x80))
            else:
                out.append(rng.randrange(0x80, 0x100))
        elif label == "gb18030" or label == "GBK":
            if k < 0.3:
                out += bytes([rng.randrange(0x81, 0xFF), rng.randrange(0x30, 0x3A), rng.randrange(0x81, 0xFF), rng.randrange(0x30, 0x3A)])
            elif k < 0.4:
                out += bytes([rng.randrange(0x81, 0xFF), rng.randrange(0x30, 0x3A), rng.randrange(0, 0x100)])
            elif k < 0.6:
                out += bytes([rng.randrange(0x81, 0xFF), rng.randrange(0x40, 0xFF)])
            elif k < 0.8:
                out.append(rng.randrange(0x30, 0x3A))
            else:
                out.append(rng.randrange(0, 0x100))
        elif label == "EUC-JP" and k < 0.2:
            out += bytes([rng.choice([0x8E, 0x8F]), rng.randrange(0x80, 0x100), rng.randrange(0x80, 0x100)])
        elif label in MULTI:
            if k < 0.5:
                out += bytes([rng.randrange(0x81, 0xFF), rng.randrange(0x40, 0xFF)])
            elif k < 0.75:
                out.append(rng.randrange(0x80, 0x100))
            else:
                out.append(rng.randrange(0, 0x80))
        elif label == "UTF-8":
            if k < 0.5:
                out += chr(rng.choice([rng.randrange(0x80, 0x800), rng.randrange(0x800, 0xD800), rng.randrange(0x10000, 0x110000)])).encode()
            else:
                out.append(rng.choice([rng.randrange(0x80, 0xC0), rng.randrange(0xC0, 0x100), 0xE0, 0xED, 0xF0, 0xF4, rng.randrange(0, 0x80)]))
        elif label.startswith("UTF-16"):
            le = label.endswith("LE")
            u = rng.choice([rng.randrange(0, 0x10000), rng.randrange(0xD800, 0xDC00), rng.randrange(0xDC00, 0xE000)])
            out += u.to_bytes(2, "little" if le else "big")
            if k < 0.05:
                out.append(rng.randrange(256))
        else:
            out.append(rng.randrange(256))
    return bytes(out[:n])


def main():
    quick = "--quick" in sys.argv
    win = os.name == "nt"
    ours = ROOT / "build" / ("win" if win else "linux") / ("coretest.exe" if win else "coretest")
    if not ours.exists():
        print(f"encoding_check: {ours.relative_to(ROOT)} is missing (python build_rs.py app coretest --target {'win' if win else 'linux'})")
        return 1
    try:
        ref = reference()
    except Unavailable as e:
        print(f"encoding_check: {e}: nothing compared")
        return 2
    except BuildFailed as e:
        print(f"encoding_check: FAIL the reference: {e}")
        return 1
    (ROOT / "build").mkdir(exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix="encoding_check-", dir=ROOT / "build"))
    c = Check(ref, ours, work)
    rng = random.Random(17)

    # encode: every scalar value alone, then texts (written as bytes: no newline translation)
    chars = every_char()
    allf = work / "every_char.txt"
    allf.write_bytes(chars.encode("utf-8"))
    for label in MULTI:
        a, b = work / f"enc_{label}.ours", work / f"enc_{label}.ref"
        got = c.output(f"{label} encode", ours, "--encoding", "e", label, allf, a, out=a)
        want = c.output(f"{label} encode (reference)", ref, "encode", label, allf, b, out=b)
        if got is None or want is None:
            continue
        got, want = got.decode().split("\n")[:-1], want.decode().split("\n")[:-1]
        c.n += 1
        diff = [i for i in range(min(len(got), len(want))) if got[i] != want[i]]
        if len(got) != len(chars) or len(want) != len(chars) or diff:
            c.bad.append(f"{label} encode: {len(got)} and {len(want)} lines for {len(chars)} characters, {len(diff)} differ, first "
                         + ", ".join(f"U+{ord(chars[i]):04X} here {got[i]} encoding_rs {want[i]}" for i in diff[:5]))
            continue
        mapped = "".join(ch for ch, w in zip(chars, want) if w != "-" and ord(ch) >= 0x80)
        lacks = [ch for ch, w in zip(chars, want) if w == "-"]
        mixed = "".join(rng.choice([rng.choice(mapped), rng.choice(lacks), "a", "\n"]) for _ in range(5000))
        mixed = mixed[:2500] + rng.choice(lacks) + mixed[2500:]
        for name, text in {"mapped": "abc " + mapped + " xyz", "mixed": mixed}.items():
            f = work / f"text_{label}_{name}.txt"
            f.write_bytes(text.encode("utf-8"))
            et1, et2 = work / "et.ours", work / "et.ref"
            for x in (et1, et2):
                if x.exists():
                    x.unlink()
            r1, out1, err1 = c.run(ours, "--encoding", "et", label, f, et1)
            r2, out2, err2 = c.run(ref, "encode-text", label, f, et2)
            if name == "mapped":
                # every character is there: both write, the same bytes
                if r1 == 0 and r2 == 0:
                    c.same(f"{label} encode {name}", et1.read_bytes(), et2.read_bytes())
                else:
                    c.n += 1
                    c.bad.append(f"{label} encode {name}: here exit {r1} {err1[-200:]!r}, encoding_rs exit {r2} {err2[-200:]!r} (want 0 and 0)")
            else:
                # both refuse the first character the encoding lacks
                first = next(ch for ch in text if ch in set(lacks))
                expect = f"{ord(first):x}"
                ok = r1 == 3 and r2 == 3 and out1 == expect and (out2 == expect or label == "ISO-2022-JP" and first in "\x0e\x0f\x1b" and out2 == "fffd")
                c.n += 1
                if not ok:
                    c.bad.append(f"{label} encode {name}: here exit {r1} {out1!r}, encoding_rs exit {r2} {out2!r} (want exit 3 and {expect})")
            h1 = c.output(f"{label} encode with character references {name}", ours, "--encoding", "eh", label, f, work / "eh.ours", out=work / "eh.ours")
            h2 = c.output(f"{label} encode with character references {name} (reference)", ref, "encode-html", label, f, work / "eh.ref", out=work / "eh.ref")
            if h1 is not None and h2 is not None:
                c.same(f"{label} encode with character references {name}", h1, h2)
    print(f"encoding_check: encode: {len(chars)} characters, {len(MULTI)} encodings")

    # decode: the sweeps
    for label in ["Shift_JIS", "EUC-JP", "GBK", "gb18030", "Big5", "EUC-KR"]:
        c.decode(label, pairs(range(0x80, 0x100)), f"pairs_{label}", pieces=False)
    c.decode("EUC-JP", b"".join(bytes([0x8F, b, t, 0x41]) for b in range(0x80, 0x100) for t in range(256)), "triples_EUC-JP", pieces=False)
    # each pair from its state: the escape before it, ESC ( B after it (which also closes a lead byte left open)
    jp = b"".join(esc + bytes([l, t]) + b"\x1b(B" for esc in [b"", b"\x1b(J", b"\x1b(I", b"\x1b$@", b"\x1b$B"]
                  for l in range(0x80) for t in range(0x80))
    c.decode("ISO-2022-JP", jp, "pairs_ISO-2022-JP", pieces=False)
    if not quick:
        four = b"".join(bytes([b1, b2, b3, b4]) for b1 in range(0x81, 0xFF) for b2 in range(0x30, 0x3A) for b3 in range(0x81, 0xFF) for b4 in range(0x30, 0x3A))
        c.decode("gb18030", four, "four_gb18030", pieces=False)
        c.decode("gb18030", b"".join(bytes([b1, b2, b3, b4]) for b1 in range(0x81, 0xFF) for b2 in (0x30, 0x39, 0x2F, 0x3A)
                                     for b3 in (0x80, 0x81, 0xFE, 0xFF, 0x30, 0x41) for b4 in (0x30, 0x39, 0x2F, 0x41, 0x81)), "four_edges_gb18030", pieces=False)
    print("encoding_check: decode: every two-byte sequence, EUC-JP's three-byte ones, ISO-2022-JP's pairs"
          + ("" if quick else ", gb18030's four-byte ones"))

    # decode: random streams
    streams = 20 if quick else 150
    for label in MULTI + OTHERS:
        for i in range(streams):
            data = random_stream(rng, label, rng.choice([1, 2, 3, 7, 64, 1000, 4000]))
            c.decode(label, data, f"random_{label}_{i}")
        for i, pre in enumerate([b"\xef\xbb\xbf", b"\xff\xfe", b"\xfe\xff", b"\xef\xbb", b"\xef", b"\xff", b""]):
            c.decode(label, pre + random_stream(rng, label, rng.choice([0, 1, 2, 50])), f"bom_{label}_{i}", bom=True)
    print(f"encoding_check: decode: {streams} random streams of each of {len(MULTI + OTHERS)} encodings, at once and in pieces;"
          " byte-order marks")

    for b in c.bad[:40]:
        print("FAIL", b)
    print(f"encoding_check: {c.n - len(c.bad)} of {c.n} comparisons agree" + (f", {len(c.bad)} differ" if c.bad else ""))
    if c.bad:
        print(f"encoding_check: the files are in {work}")
        return 1
    shutil.rmtree(work, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
