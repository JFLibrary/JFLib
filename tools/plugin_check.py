#!/usr/bin/env python3
"""plugin_check.py -- PaintCraft plugin system check (agent P1).

Builds PaintCraft and the example / test plugins (build.py app + build.py plugin), prepares
test plugin folders under <out>/<target>/ptest (never the real exe folder), runs the headless
scripts apps/PaintCraft/tests/plugins*.uis (also with --verify-dirty), checks the QOI files the
script wrote against an independent Python QOI codec, and fuzzes the loader:
  * Linux: >= 1000 mutated ELF objects (bit flips anywhere, header / program header / dynamic
    flips, truncations) through --plugin-fuzz (plat_lib_open / sym / close, guarded)
  * Windows: mutated DLLs the same way (LoadLibraryExW + own export parser)
  * both: data-only mutants (PLUGININFO / descriptor bytes) through the full load path
    (pn_plugin_query + validation) in batches -- the app must start and quit normally.

  python tools/plugin_check.py [--target win|linux|both] [--out build/p1] [--fuzz N]
  python tools/plugin_check.py --fixtures      regenerate the committed test fixtures

Expected values (hashes) are computed here from the fixture with the plugins' documented
integer math, not taken from the app.
"""

import argparse
import os
import random
import re
import shutil
import struct
import subprocess
import sys
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIX = ROOT / "apps" / "PaintCraft" / "tests" / "fixtures"
TESTS = ROOT / "apps" / "PaintCraft" / "tests"
WSL = os.environ.get("jfgui_WSL_DISTRO", "Ubuntu-24.04")
STD_SET = {"Effects": ["swapchan", "sepiastr", "crash", "crashpf", "crashq", "toonew", "badinfo"],
           "FileTypes": ["qoi"], "Shapes": ["pshapes"]}
FW, FH = 64, 48


# ------------------------------------------------------------------ fixture
def fixture_pixels():
    """64 x 48 straight RGBA: a gradient, a flat band (QOI runs), a transparent block."""
    px = []
    for y in range(FH):
        for x in range(FW):
            if x >= 50 and y < 10:
                px.append((0, 0, 0, 0))
            elif y >= 40:
                px.append((200, 100, 50, 255))
            elif x < 8 and y < 8:
                px.append((x * 30, y * 30, 255 - x * 20, 255))   # small steps: DIFF / LUMA ops
            else:
                px.append(((x * 4) & 255, (y * 5) & 255, ((x + y) * 3) & 255, 255))
    return px


def png_bytes(px, w, h):
    raw = b"".join(b"\0" + bytes(c for p in px[y * w:(y + 1) * w] for c in p) for y in range(h))

    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


# ------------------------------------------------------------------ QOI (reference, qoiformat.org)
def qoi_hash(p):
    r, g, b, a = p
    return (r * 3 + g * 5 + b * 7 + a * 11) % 64


def qoi_encode(px, w, h):
    out = bytearray(b"qoif" + struct.pack(">II", w, h) + bytes([4, 0]))
    index = [(0, 0, 0, 0)] * 64
    prev, run = (0, 0, 0, 255), 0
    for i, p in enumerate(px):
        if p == prev:
            run += 1
            if run == 62 or i == len(px) - 1:
                out.append(0xC0 | (run - 1))
                run = 0
            continue
        if run:
            out.append(0xC0 | (run - 1))
            run = 0
        k = qoi_hash(p)
        if index[k] == p:
            out.append(k)
        else:
            index[k] = p
            if p[3] == prev[3]:
                vr = ((p[0] - prev[0] + 128) & 255) - 128
                vg = ((p[1] - prev[1] + 128) & 255) - 128
                vb = ((p[2] - prev[2] + 128) & 255) - 128
                vgr, vgb = vr - vg, vb - vg
                if -2 <= vr <= 1 and -2 <= vg <= 1 and -2 <= vb <= 1:
                    out.append(0x40 | (vr + 2) << 4 | (vg + 2) << 2 | (vb + 2))
                elif -32 <= vg <= 31 and -8 <= vgr <= 7 and -8 <= vgb <= 7:
                    out += bytes([0x80 | (vg + 32), (vgr + 8) << 4 | (vgb + 8)])
                else:
                    out += bytes([0xFE, p[0], p[1], p[2]])
            else:
                out += bytes([0xFF, p[0], p[1], p[2], p[3]])
        prev = p
    return bytes(out + b"\0" * 7 + b"\1")


def qoi_decode(data):
    assert data[:4] == b"qoif", "no QOI header"
    w, h = struct.unpack(">II", data[4:12])
    px, index, p, run, pos, end = [], [(0, 0, 0, 0)] * 64, (0, 0, 0, 255), 0, 14, len(data) - 8
    for _ in range(w * h):
        if run:
            run -= 1
        elif pos < end:
            b1 = data[pos]
            pos += 1
            r, g, b, a = p
            if b1 == 0xFE:
                p = (data[pos], data[pos + 1], data[pos + 2], a)
                pos += 3
            elif b1 == 0xFF:
                p = tuple(data[pos:pos + 4])
                pos += 4
            elif b1 >> 6 == 0:
                p = index[b1]
            elif b1 >> 6 == 1:
                p = ((r + (b1 >> 4 & 3) - 2) & 255, (g + (b1 >> 2 & 3) - 2) & 255,
                     (b + (b1 & 3) - 2) & 255, a)
            elif b1 >> 6 == 2:
                b2 = data[pos]
                pos += 1
                vg = (b1 & 63) - 32
                p = ((r + vg - 8 + (b2 >> 4)) & 255, (g + vg) & 255, (b + vg - 8 + (b2 & 15)) & 255, a)
            else:
                run = b1 & 63
            index[qoi_hash(p)] = p
        px.append(p)
    return w, h, px


# ------------------------------------------------------------------ expected results
def premul(p):
    r, g, b, a = p
    return (0, 0, 0, 0) if a == 0 else p          # the fixture only has alpha 0 / 255


def fnv(px):
    h = 0xCBF29CE484222325
    for r, g, b, a in px:
        for c in (b, g, r, a):
            h = ((h ^ c) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"{h:016x}"


def swap(p, mode=0):
    r, g, b, a = p
    return [(b, g, r, a), (g, b, r, a), (b, r, g, a)][mode]


def sepia(p, strength=60):
    r, g, b, a = p
    w = (strength * 256 + 50) // 100
    t = (77 * r + 150 * g + 29 * b + 128) >> 8
    mix = lambda s, d: (s * (256 - w) + d * w + 128) >> 8
    return (mix(r, min(255, t + 40)), mix(g, min(255, t + 20)), mix(b, max(0, t - 20)), a)


def expected():
    """PaintCraft layers hold straight ARGB (the effect result is stored as the kernel wrote it,
    also for alpha 0); the fixture's transparent pixels are (0, 0, 0, 0)."""
    src = fixture_pixels()
    return {"original": fnv(src),
            "swapchan": fnv([swap(p) for p in src]),
            "swapchan2": fnv([swap(p, 2) for p in src]),
            "sepiastr": fnv([sepia(p) for p in src])}


def write_fixtures():
    px = fixture_pixels()
    (FIX / "p1_plugin.png").write_bytes(png_bytes(px, FW, FH))
    (FIX / "p1_plugin.qoi").write_bytes(qoi_encode(px, FW, FH))
    print("fixtures written:", FIX / "p1_plugin.png", FIX / "p1_plugin.qoi")
    for k, v in expected().items():
        print(f"  {k:10s} {v}")


# ------------------------------------------------------------------ build / run helpers
def build(targets, out):
    for t in targets:
        for cmd in (["app", "PaintCraft"], ["plugin"]):
            r = subprocess.run([sys.executable, str(ROOT.parent / "legacy" / "build.py")] + cmd +
                               ["--target", t, "--out", out], cwd=ROOT, capture_output=True, text=True,
                               encoding="utf-8", errors="replace")
            if r.returncode:
                sys.exit(f"build failed: {' '.join(cmd)} {t}\n{r.stdout}{r.stderr}")


def ext(t):
    return ".dll" if t == "win" else ".so"


def plugin_file(out, t, name):
    return ROOT / out / t / "plugins" / (name + ext(t))


def setup_dir(out, t, dirname, sets):
    d = ROOT / out / t / dirname
    if d.exists():
        shutil.rmtree(d)
    for folder, names in sets.items():
        (d / folder).mkdir(parents=True, exist_ok=True)
        for n in names:
            src = n if isinstance(n, Path) else plugin_file(out, t, n)
            shutil.copy(src, d / folder / src.name)
    for folder in ("Effects", "FileTypes", "Shapes"):
        (d / folder).mkdir(parents=True, exist_ok=True)
    return d


def run_app(out, t, args, timeout=600):
    exe = ROOT / out / t / ("PaintCraft.exe" if t == "win" else "PaintCraft")
    if t == "win":
        cmd = [str(exe)] + args
    else:
        cmd = ["wsl.exe", "-d", WSL, "--cd", str(ROOT), "-e",
               "./" + str(exe.relative_to(ROOT)).replace("\\", "/")] + args
    env = dict(os.environ, MSYS_NO_PATHCONV="1")
    r = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, encoding="utf-8",
                       errors="replace", timeout=timeout, env=env)
    return r.returncode, (r.stdout or "") + (r.stderr or "")


def rel(p):
    return str(Path(p).relative_to(ROOT)).replace("\\", "/")


# ------------------------------------------------------------------ checks
class Report:
    def __init__(self):
        self.fails = 0

    def check(self, ok, what, detail=""):
        print(f"  {'ok  ' if ok else 'FAIL'} {what}" + (f"  ({detail})" if detail and not ok else ""))
        self.fails += 0 if ok else 1


def run_scripts(rep, out, t):
    d = setup_dir(out, t, "ptest", STD_SET)
    for script in sorted(TESTS.glob("plugins*.uis")):
        for extra in ([], ["--verify-dirty"]):
            args = ["--plugins=" + rel(d), "--script", rel(script)] + extra
            first = script.read_text(encoding="utf-8").splitlines()[0]
            if first.startswith("#!args "):
                args = [a for a in first[7:].split() if not a.startswith("--plugins")
                        and not a.startswith("@")] + args
            rc, text = run_app(out, t, args)
            rep.check(rc == 0, f"[{t}] {script.name} {' '.join(extra)}".rstrip(), text[-3000:])
    # the plugin type in the Open (decoders) and Save As (encoders) lists: cmd 146 -> #0x82403
    (ROOT / out / "p1_filter.uis").write_text("size 800 600\ncmd 146 1\ndump\ncmd 146 2\ndump\n",
                                             encoding="utf-8")
    rc, text = run_app(out, t, ["--plugins=" + rel(d), "--script", rel(ROOT / out / "p1_filter.uis")])
    lists = re.findall(r'"id":533507,"k":"text","t":"([^"]*)"', text)
    rep.check(rc == 0 and len(lists) == 2 and all("|qoi=QOI|" in "|" + x for x in lists),
              f"[{t}] QOI in the decoder and encoder filter lists", str(lists))
    # the QOI file written by plugins.uis: an independent decode must give the fixture
    q = ROOT / "build" / "p1_plugins_out.qoi"
    if q.exists():
        w, h, px = qoi_decode(q.read_bytes())
        want = [premul(p) for p in fixture_pixels()]
        got = [premul(p) for p in px]
        rep.check((w, h) == (FW, FH) and got == want, f"[{t}] QOI written by the plugin decodes to the image")
        q.unlink()
    else:
        rep.check(False, f"[{t}] QOI written by the plugin exists", str(q))


def mutate(data, rng, kind):
    b = bytearray(data)
    if kind == "trunc":
        return bytes(b[:rng.randrange(0, len(b))])
    if kind == "head":                              # the first 4 KB: headers, dynamic, tables
        n = min(len(b), 4096)
    else:
        n = len(b)
    for _ in range(rng.choice((1, 1, 2, 3, 8))):
        i = rng.randrange(0, n)
        if rng.random() < 0.3:
            b[i] = rng.choice((0, 0xFF, 0x7F, 0x80, rng.randrange(256)))
        else:
            b[i] ^= 1 << rng.randrange(8)
    return bytes(b)


def fuzz_loader(rep, out, t, count, seed=1):
    rng = random.Random(seed)
    d = ROOT / out / t / "pfuzz"
    if d.exists():
        shutil.rmtree(d)
    d.mkdir(parents=True)
    bases = [plugin_file(out, t, n).read_bytes() for n in ("swapchan", "qoi", "pshapes", "crashpf")]
    for i in range(count):
        kind = rng.choice(("any", "head", "head", "trunc"))
        (d / f"m{i:05d}{ext(t)}").write_bytes(mutate(rng.choice(bases), rng, kind))
    rc, text = run_app(out, t, ["--plugin-fuzz=" + rel(d), "--script", rel(ROOT / out / "p1_empty.uis")],
                       timeout=1800)
    m = re.search(r"plugin-fuzz: files,loaded,rejected,faults = (\d+),(\d+),(\d+),(\d+)", text)
    ok = rc == 0 and m is not None and int(m.group(1)) == count
    rep.check(ok, f"[{t}] loader fuzz: {count} mutants, no crash",
              text[-2000:])
    for line in text.splitlines():
        if line.startswith("plugin-fuzz: fault in"):
            print("        ", line)
    if m:
        print(f"         files {m.group(1)}, loaded {m.group(2)}, rejected {m.group(3)}, "
              f"guarded faults {m.group(4)}")
        rep.check(int(m.group(4)) == 0, f"[{t}] loader fuzz: the loader itself never faulted")
    if m and int(m.group(4)) == 0:
        shutil.rmtree(d, ignore_errors=True)        # (kept for a look when the loader faulted)


def fuzz_full(rep, out, t, batches, seed=7):
    """Data-only mutants through the whole load path (query + validation + registration)."""
    rng = random.Random(seed)
    base = plugin_file(out, t, "swapchan").read_bytes()
    qbase = plugin_file(out, t, "qoi").read_bytes()
    # the data section of the plugin: find the PLUGININFO magic "PNPI"
    for b0 in (base, qbase):
        assert b0.find(b"PNPI") > 0, "no PLUGININFO found"
    n_ok = 0
    for k in range(batches):
        muts = []
        for i in range(30):
            src = rng.choice((base, qbase))
            p = src.find(b"PNPI")
            b = bytearray(src)
            if i < 4:                               # a few truncated / cut-off files as well
                b = b[:rng.randrange(0, len(b))]
            for _ in range(rng.choice((1, 2, 4))):
                j = p + rng.randrange(0, 256)
                if j < len(b):
                    b[j] ^= 1 << rng.randrange(8)
            f = ROOT / out / t / f"pmut{i:02d}{ext(t)}"
            f.write_bytes(bytes(b))
            muts.append(f)
        setup_dir(out, t, "pfull", {"Effects": muts})
        for f in muts:
            f.unlink()
        rc, text = run_app(out, t, ["--plugins=" + rel(ROOT / out / t / "pfull"),
                                    "--script", rel(ROOT / out / "p1_empty.uis")])
        n_ok += rc == 0
        if rc:
            print(text[-2000:])
    rep.check(n_ok == batches, f"[{t}] data mutants through the full load path: {batches * 30} "
              "plugins in {batches} runs, no crash".replace("{batches}", str(batches)))
    shutil.rmtree(ROOT / out / t / "pfull", ignore_errors=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--out", default="build/p1")
    ap.add_argument("--fuzz", type=int, default=0, help="loader mutants per target "
                    "(default 1200)")
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--fixtures", action="store_true", help="write the fixtures and exit")
    a = ap.parse_args()
    if a.fixtures:
        write_fixtures()
        return 0
    targets = ["win", "linux"] if a.target == "both" else [a.target]
    if not a.no_build:
        build(targets, a.out)
    (ROOT / a.out / "p1_empty.uis").write_text("size 800 600\nwait 10\n", encoding="utf-8")
    rep = Report()
    exp = expected()
    print("expected hashes:", exp)
    for t in targets:
        print(f"[{t}]")
        run_scripts(rep, a.out, t)
        fuzz_loader(rep, a.out, t, a.fuzz or 1200)
        fuzz_full(rep, a.out, t, 4)
    print("plugin_check:", "all passed" if rep.fails == 0 else f"{rep.fails} failed")
    return 1 if rep.fails else 0


if __name__ == "__main__":
    sys.exit(main())
