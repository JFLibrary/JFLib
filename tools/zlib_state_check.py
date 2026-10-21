#!/usr/bin/env python3
"""Reusable deflate / inflate states (agent Z1): deflate.s, inflate.s, gzip.s, thr_self.

Drives apps/codectest ops 300..304 (apps/codectest/src/zlib_ops.s) and the classic ops
3 (zlib_deflate), 245 (gzip_encode), 246 (deflate_raw), 1 / 2 (zlib_inflate /
inflate_raw), 244 (gzip_decode), 257 (inflate_raw_ex).

  python tools/zlib_state_check.py --target both --out build/z1
  python tools/zlib_state_check.py --target win --only bench --threads 1,12
  python tools/zlib_state_check.py --base build/z1base    # + byte identity vs an older build

Suites:
  state    one deflate state for many consecutive compressions (op 300, allocating *_st and
           caller-buffer *_to variants, zlib / raw / gzip, levels -1..10, empty / 1-byte /
           64 KB-boundary / random / text / repetitive / multi-block inputs in random order)
           gives exactly the bytes of fresh calls; every stream decodes with Python's zlib
  limit    *_to with caps around the exact size (size + 8 fits, size + 7 overflows for raw;
           tiny caps; outputs end at a guard page, so a write past cap crashes the run)
  inflate  one inflate state for valid, corrupt and truncated zlib / raw / gzip streams
           interleaved (op 301): result, consumed and bytes equal fresh calls, i.e. an error
           never poisons the next call
  self     thr_self (op 304): no two concurrent items share a slot, slots <= workers
  base     (with --base DIR) ops 3 / 245 / 246 of the current build and of DIR give the same
           bytes for the whole corpus (the pre-Z1 build: "bit-identical to today"), and so do
           PDN (op 221, parallel chunks), TIFF Deflate and PNG encodes
  pdndec   (with --base DIR) PDN decoding (parallel chunk decoder): the samples and --fuzz/4
           corrupted multi-chunk files give the baseline's doc record or error code
  bench    history-style benchmarks (ops 302 / 303), 4000x3000 layer = 2961 tiles, per
           --threads value: classic calls vs per-thread states (reported, not checked)
"""

import random
import struct
import sys
import zlib
import gzip as pygzip
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
import imgtest_common as it  # noqa: E402

OP_ZINF, OP_RINF, OP_ZDEF = 1, 2, 3
OP_GZDEC, OP_GZENC, OP_RAWDEF, OP_RAWINFEX = 244, 245, 246, 257
OP_DEFSEQ, OP_INFSEQ, OP_HBENCH, OP_IBENCH, OP_SELF = 300, 301, 302, 303, 304
ERR_OVERFLOW = -6
KINDS = ("zlib", "raw", "gzip")
CLASSIC = {0: OP_ZDEF, 1: OP_RAWDEF, 2: OP_GZENC}
SUITES = "state,limit,inflate,self,base,bench"


def bound(n):
    return n + n // 512 + 128


# ---------------------------------------------------------------- corpus

def corpus(rng, big=True):
    c = {}
    c["empty"] = b""
    c["one"] = b"A"
    c["two"] = b"ab"
    c["three"] = b"abc"
    c["four"] = b"abca"
    words = ("the quick brown fox jumps over a lazy dog deflate inflate state reuse "
             "window hash chain block symbol huffman literal length distance").split()
    text = " ".join(rng.choice(words) for _ in range(30000)).encode()
    c["text"] = text[:150000]
    c["rand1k"] = rng.randbytes(1000)
    c["rand16k"] = rng.randbytes(16384)
    c["zeros70k"] = bytes(70000)
    rep = bytes(rng.randrange(4) for _ in range(300)) * 400
    c["rep120k"] = rep
    for n in (65534, 65535, 65536, 65537, 131071, 131072, 131073):
        c[f"b{n}"] = (text * 2)[:n]
    # many 3-byte matches: more than 16K symbols per block, positions past 32K and 64K
    c["syms"] = bytes((i * 7 + (i >> 5)) & 0x3F for i in range(200000))
    # gradient + noise tile (history-like)
    tile = bytearray()
    for p in range(4096):
        x, y = p & 63, p >> 6
        for ch in range(3):
            tile.append((x + y + ch * 8 + rng.randrange(8)) & 255)
        tile.append(255)
    c["tile"] = bytes(tile)
    if big:
        c["mix1m"] = (text * 8)[:600000] + rng.randbytes(200000) + bytes(300000) + rep[:100000]
    return c


def decode(kind, stream):
    if kind == 0:
        return zlib.decompress(stream)
    if kind == 1:
        return zlib.decompress(stream, -15)
    return pygzip.decompress(stream)


def item_def(level, kind, cap, data):
    return struct.pack("<iIII", level, kind, cap, len(data)) + data


def parse_items(out, n, extra=0):
    """op 300 output: n x {i32 result, u32 k, bytes[k]}; op 301: {i32 result, u32 consumed, bytes}"""
    res, p = [], 0
    for _ in range(n):
        r, k = struct.unpack_from("<iI", out, p)
        p += 8
        if extra:
            ln = max(r, 0)
            res.append((r, k, bytes(out[p:p + ln])))
            p += ln
        else:
            res.append((r, bytes(out[p:p + k])))
            p += k
    if p != len(out):
        raise ValueError(f"output size mismatch {p} != {len(out)}")
    return res


# ---------------------------------------------------------------- suites

def suite_state(runner, rng, target):
    s = it.Suite("state", target)
    c = corpus(rng)
    names = sorted(c)
    levels = list(range(-1, 11))
    # reference: fresh classic calls for every (name, level, kind)
    jobs = [(n, lv, k) for n in names for lv in levels for k in range(3)]
    recs = [it.Rec(CLASSIC[k], lv, c[n], None, f"{KINDS[k]} {n} L{lv}") for n, lv, k in jobs]
    ref = runner.batch(recs)
    refd = {}
    for (n, lv, k), r in zip(jobs, ref):
        if r is None or r[0] != 0:
            s.fail(f"classic {KINDS[k]} {n} L{lv}: {r and it.err_name(r[0])}")
            continue
        out = bytes(r[1])
        try:
            ok = decode(k, out) == c[n]
        except Exception as e:  # noqa: BLE001
            ok = False
            s.fail(f"classic {KINDS[k]} {n} L{lv}: python decode {e}")
        if ok:
            refd[(n, lv, k)] = out
    # sequences through one state: random orders, both APIs, all kinds mixed
    seqs = []
    for rep in range(6):
        order = jobs[:]
        rng.shuffle(order)
        for i in range(0, len(order), 120):
            seqs.append(order[i:i + 120])
    recs = []
    for si, seq in enumerate(seqs):
        data = b"".join(item_def(lv, k, 0, c[n]) for n, lv, k in seq)
        for mode in (0, 1):
            recs.append(it.Rec(OP_DEFSEQ, mode, data, None, f"seq{si} mode{mode}"))
    # one long sequence of tiles at level 1 (the history pattern) on one state
    tiles = [c["tile"], c["rand16k"], c["tile"][::-1], bytes(16384), c["tile"]] * 40
    tdata = b"".join(item_def(1, 0, 0, t) for t in tiles)
    recs.append(it.Rec(OP_DEFSEQ, 1, tdata, None, "tiles mode1"))
    res = runner.batch(recs)
    for rec, r in zip(recs, res):
        if r is None:
            s.fail(f"CRASH {rec.name}")
            continue
        if r[0] != 0:
            s.fail(f"{rec.name}: {it.err_name(r[0])}")
            continue
        if rec.name == "tiles mode1":
            items = parse_items(r[1], len(tiles))
            for i, ((rr, out), t) in enumerate(zip(items, tiles)):
                want = zlib.compress(t, 1)  # only for the decode check below
                s.check(rr == len(out) and zlib.decompress(out) == t, f"tiles #{i}: result {rr}")
                del want
            fresh = runner.batch([it.Rec(OP_ZDEF, 1, t, None, "t") for t in tiles[:5]])
            for i, f in enumerate(fresh):
                s.check(f is not None and bytes(f[1]) == items[i][1], f"tiles #{i}: differs from a fresh call")
            continue
        si = int(rec.name[3:].split()[0])
        seq = seqs[si]
        items = parse_items(r[1], len(seq))
        for (n, lv, k), (rr, out) in zip(seq, items):
            key = (n, lv, k)
            if key not in refd:
                continue
            if rr != len(out):
                s.fail(f"{rec.name} {KINDS[k]} {n} L{lv}: result {it.err_name(rr)}")
            elif out != refd[key]:
                s.fail(f"{rec.name} {KINDS[k]} {n} L{lv}: bytes differ from a fresh call "
                       f"({len(out)} vs {len(refd[key])})")
            else:
                s.ok()
    s.notes.append(f"{len(jobs)} fresh references, {len(seqs)} sequences x 2 APIs")
    return s


def suite_limit(runner, rng, target):
    s = it.Suite("limit", target)
    c = corpus(rng, big=False)
    cases = []
    for n in ("empty", "one", "text", "rand1k", "rand16k", "b65536", "zeros70k", "tile", "syms"):
        for lv in (0, 1, 6, 9):
            for k in range(3):
                cases.append((n, lv, k))
    # exact sizes from the allocating API
    recs = [it.Rec(CLASSIC[k], lv, c[n], None, "ref") for n, lv, k in cases]
    ref = runner.batch(recs)
    items, meta = [], []
    for (n, lv, k), r in zip(cases, ref):
        if r is None or r[0] != 0:
            s.fail(f"ref {KINDS[k]} {n} L{lv}")
            continue
        size = len(r[1])
        # zlib / gzip: the trailer lives in the 8 bytes of slack: stream + 4 / + 0 fits
        slack = {0: 4, 1: 8, 2: 0}[k]
        for cap in (size + slack, size + slack - 1, bound(len(c[n])), 1, 7, 8, 9, 17, 18, 19,
                    size // 2, size + slack + 1):
            if cap <= 0:
                continue                        # cap 0 means deflate_bound in op 300
            items.append(item_def(lv, k, cap, c[n]))
            meta.append((n, lv, k, cap, size + slack <= cap, bytes(r[1])))
    data = b"".join(items)
    r = runner.batch([it.Rec(OP_DEFSEQ, 1, data, None, "limit")])[0]
    if r is None:
        s.fail("CRASH limit sequence (a write past cap hit the guard page?)")
        return s
    for (n, lv, k, cap, fits, want), (rr, out) in zip(meta, parse_items(r[1], len(meta))):
        tag = f"{KINDS[k]} {n} L{lv} cap {cap}"
        if fits:
            s.check(rr == len(want) and out == want, f"{tag}: {it.err_name(rr)} / bytes differ")
        else:
            s.check(rr == ERR_OVERFLOW, f"{tag}: expected OVERFLOW, got {it.err_name(rr)}")
    # temporary state per item (st = 0) gives the same bytes
    r2 = runner.batch([it.Rec(OP_DEFSEQ, 3, data, None, "limit tmp")])[0]
    s.check(r2 is not None and bytes(r2[1]) == bytes(r[1]), "st = 0 per item differs from one state")
    return s


def suite_inflate(runner, rng, target):
    s = it.Suite("inflate", target)
    c = corpus(rng, big=False)
    streams = []
    for n in ("empty", "one", "text", "rand1k", "b65537", "zeros70k", "tile", "syms", "rep120k"):
        d = c[n]
        for lv in (0, 1, 6, 9):
            streams.append((0, zlib.compress(d, lv), len(d)))
            co = zlib.compressobj(lv, zlib.DEFLATED, -15)
            streams.append((1, co.compress(d) + co.flush(), len(d)))
            streams.append((2, pygzip.compress(d, lv, mtime=0), len(d)))
    items = []
    for i in range(1500):
        kind, z, n = rng.choice(streams)
        m = z
        r = rng.random()
        if r < 0.45:
            m = it.mutate(rng, z)
        elif r < 0.55:
            m = z[:rng.randrange(len(z) + 1)]
        elif r < 0.6:
            m = rng.randbytes(rng.randrange(40))
        cap = rng.choice((n, n, n + 100, max(0, n - 1), 5000, 0))
        items.append((kind, cap, m))
    # fresh references: classic ops (one call per record)
    recs = []
    for kind, cap, m in items:
        if kind == 0:
            recs.append(it.Rec(OP_ZINF, cap, m, None, "z"))
        elif kind == 1:
            recs.append(it.Rec(OP_RAWINFEX, cap, m, None, "r"))
        else:
            recs.append(it.Rec(OP_GZDEC, cap, m, None, "g"))
    ref = runner.batch(recs)
    data = b"".join(struct.pack("<III", k, cap, len(m)) + m for k, cap, m in items)
    outs = {}
    for mode in (0, 2):
        r = runner.batch([it.Rec(OP_INFSEQ, mode, data, None, "infseq")])[0]
        if r is None or r[0] != 0:
            s.fail(f"inflate sequence mode {mode}: {'CRASH' if r is None else it.err_name(r[0])}")
            return s
        outs[mode] = parse_items(r[1], len(items), extra=1)
    nerr = 0
    for i, ((kind, cap, m), rf) in enumerate(zip(items, ref)):
        if rf is None:
            s.fail(f"#{i}: classic call crashed")
            continue
        if kind == 0:
            want = (rf[0], None, bytes(rf[1]) if rf[0] >= 0 else b"")
        else:
            if rf[0] >= 0:
                cons = struct.unpack_from("<Q", rf[1])[0]
                want = (rf[0], cons, bytes(rf[1][8:]))
            else:
                want = (rf[0], None, b"")
        nerr += want[0] < 0
        for mode in (0, 2):
            got = outs[mode][i]
            same = got[0] == want[0] and got[2] == want[2] and (want[1] is None or got[1] == want[1])
            s.check(same, f"#{i} {KINDS[kind]} mode {mode}: got {it.err_name(got[0])}/{got[1]}, "
                          f"fresh {it.err_name(want[0])}/{want[1]}")
    s.notes.append(f"{len(items)} streams in one state, {nerr} of them errors")
    return s


def suite_self(runner, target):
    s = it.Suite("self", target)
    for thr in (None, 1, 3):
        rn = runner if thr is None else it.Runner(target, runner.out, extra=["--threads", str(thr)],
                                                    work_name="zlibcheck")
        r = rn.one(OP_SELF, 3000, b"")
        conf, mx, n, bad, seen = struct.unpack_from("<IIIIQ", r[1])
        s.check(r[0] == 0 and conf == 0 and bad == 0 and mx <= n,
                f"threads {thr}: conflicts {conf}, max slot {mx}, workers {n}, bad {bad}")
        s.notes.append(f"threads {thr or 'default'}: workers {n}, slots used {bin(seen).count('1')}, "
                       f"max {mx}")
    return s


def suite_base(runner, base_out, rng, target):
    s = it.Suite("base", target)
    base = it.Runner(target, base_out, work_name="zlibcheck")
    if not base.exe.exists():
        s.notes.append(f"no baseline build at {base.exe}")
        return s
    c = corpus(rng)
    recs = []
    for n in sorted(c):
        for lv in range(-1, 11):
            for k in range(3):
                recs.append(it.Rec(CLASSIC[k], lv, c[n], None, f"{KINDS[k]} {n} L{lv}"))
    nz = len(recs)
    recs += container_recs(rng)
    a = runner.batch(recs)
    b = base.batch(recs)
    for rec, x, y in zip(recs, a, b):
        s.check(x is not None and y is not None and x[0] == y[0] == 0 and bytes(x[1]) == bytes(y[1]),
                f"{rec.name}: differs from the baseline build "
                f"({x and it.err_name(x[0])} / {y and it.err_name(y[0])})")
    s.notes.append(f"{nz} zlib / raw / gzip streams + {len(recs) - nz} PDN / TIFF / PNG files "
                   f"compared with {base.exe}")
    return s


def suite_pdndec(runner, base_out, rng, target, nfuzz):
    """PDN decoding (parallel chunk decoder, Z1): the samples and corrupted multi-chunk files
    give exactly the result of the baseline build (doc record or the same error code)."""
    import numpy as np
    import pdn_check as pc
    s = it.Suite("pdndec", target)
    base = it.Runner(target, base_out, work_name="zlibcheck")
    if not base.exe.exists():
        s.notes.append(f"no baseline build at {base.exe}")
        return s
    recs = []
    for f in sorted(pc.SAMPLES.glob("*.pdn")):
        recs.append(it.Rec(220, 0, f.read_bytes(), None, f"sample {f.name}"))
    r = np.random.default_rng(rng.randrange(1 << 30))
    w, h = 700, 800
    layers = []
    for k in range(3):
        px = r.integers(0, 256, (h, w, 4), dtype=np.uint8)
        px[:, :, 3] = 255
        px[: h // 3] = 0
        layers.append(dict(px=px, name=f"L{k}"))
    rec = pc.make_docrec(w, h, layers)
    enc = runner.one(221, 1 << 8, rec)
    if enc[0] != 0:
        s.fail(f"encode for the fuzz base: {it.err_name(enc[0])}")
        return s
    good = bytes(enc[1])
    recs.append(it.Rec(220, 0, good, None, "multi-chunk"))
    lo = len(good) // 10
    for i in range(nfuzz):
        m = it.mutate(rng, good, keep=lo + rng.randrange(len(good) - lo))
        if i % 2:                               # two faults: the first in file order wins
            m = it.mutate(rng, m, keep=lo + rng.randrange(max(1, len(m) - lo)))
        recs.append(it.Rec(220, 0, m, None, f"fuzz#{i}"))
    a = runner.batch(recs)
    b = base.batch(recs)
    errs = {}
    for rec, x, y in zip(recs, a, b):
        ok = x is not None and y is not None and x[0] == y[0] and bytes(x[1]) == bytes(y[1])
        s.check(ok, f"{rec.name}: {x and it.err_name(x[0])} vs baseline {y and it.err_name(y[0])}")
        if x is not None:
            errs[it.err_name(x[0])] = errs.get(it.err_name(x[0]), 0) + 1
    s.notes.append("results: " + ", ".join(f"{k} {v}" for k, v in sorted(errs.items())))
    return s


def container_recs(rng):
    """PDN (op 221, chunked gzip on the pool), TIFF Deflate (strips) and PNG encodes."""
    import numpy as np
    sys.path.insert(0, str(ROOT / "tools"))
    import pdn_check as pc
    C = it.C
    r = np.random.default_rng(rng.randrange(1 << 30))
    recs = []

    def img(w, h, kind):
        yy, xx = np.mgrid[0:h, 0:w]
        if kind == "noise":
            px = r.integers(0, 256, (h, w, 4), dtype=np.uint8)
        else:
            px = np.stack([(xx * 3) & 255, (yy * 5) & 255, (xx ^ yy) & 255,
                           np.full((h, w), 255)], -1).astype(np.uint8)
            px[..., :3] += r.integers(0, 4, (h, w, 3), dtype=np.uint8)
        if kind == "holes":                     # transparent areas: whole zero chunks
            px[: h // 2] = 0
            px[:, : w // 3] = 0
        return px

    docs = {
        "1x1": (1, 1, [dict(px=img(1, 1, "smooth"), name="Hintergrund", isbg=1)]),
        "300x2": (300, 2, [dict(px=img(300, 2, "noise"))]),
        "600x520": (600, 520, [dict(px=img(600, 520, "smooth")), dict(px=img(600, 520, "holes"))]),
        "1001x777 fmts": (1001, 777, [dict(px=img(1001, 777, "holes"), fmt=C.FMT_PARGB),
                                      dict(px=img(1001, 777, "smooth"), fmt=C.FMT_XRGB),
                                      dict(px=img(1001, 777, "holes"), fmt=C.FMT_ARGB),
                                      dict(px=img(1001, 777, "noise")[..., 0], fmt=C.FMT_A8)]),
        "1500x1400 zero": (1500, 1400, [dict(px=img(1500, 1400, "holes")),
                                        dict(px=np.zeros((1400, 1500, 4), np.uint8))]),
    }
    for name, (w, h, layers) in docs.items():
        rec = pc.make_docrec(w, h, layers)
        for lv in (1, 6, 9):
            recs.append(it.Rec(221, lv << 8, rec, None, f"pdn {name} L{lv}"))
    for w, h, kind in ((1, 1, "smooth"), (4000, 60, "smooth"), (777, 555, "noise"), (1200, 900, "holes")):
        px = img(w, h, kind)
        for bd in (32, 24, 8):
            b = bytearray(C.IO_SIZE)
            struct.pack_into("<I", b, C.IO_FMT, C.IMGF_TIFF)
            struct.pack_into("<I", b, C.IO_BITDEPTH, bd)
            struct.pack_into("<I", b, C.IO_TIFFCOMP, C.TIFC_DEFLATE)
            recs.append(it.Rec(it.OP_ENCODE, 0, bytes(b) + it.surface_input(px, C.FMT_ARGB), None,
                               f"tiff deflate {w}x{h} {kind} {bd}"))
        for eff in (0, 1, 9):
            b = bytearray(C.IO_SIZE)
            struct.pack_into("<I", b, C.IO_FMT, C.IMGF_PNG)
            struct.pack_into("<I", b, C.IO_EFFORT, eff)
            recs.append(it.Rec(it.OP_ENCODE, 0, bytes(b) + it.surface_input(px, C.FMT_ARGB), None,
                               f"png {w}x{h} {kind} effort {eff}"))
    return recs


def suite_bench(runner, target, threads, tiles, reps):
    s = it.Suite("bench", target)
    for thr in threads:
        rn = it.Runner(target, runner.out, extra=["--threads", str(thr)], work_name="zlibcheck")
        for pat, pname in ((0, "noisy gradient"), (1, "random"), (2, "smooth")):
            line = [f"threads {thr:2d} {pname:14s}"]
            hashes = []
            for mode in (0, 1):
                r = rn.one(OP_HBENCH, mode, struct.pack("<IIII", tiles, pat, 1, reps))
                if r[0] != 0:
                    s.fail(f"hbench mode {mode}: {it.err_name(r[0])}")
                    continue
                best, first, nbytes, h, nthr, kept = struct.unpack_from("<QQQQII", r[1])
                hashes.append((h, nbytes, kept))
                line.append(f"deflate {'classic' if mode == 0 else 'state  '} {best / 1000:7.1f} ms "
                            f"(first {first / 1000:6.1f})")
            if len(hashes) == 2:
                s.check(hashes[0] == hashes[1], f"threads {thr} pattern {pat}: kept streams differ "
                                                f"{hashes[0]} vs {hashes[1]}")
                line.append(f"kept {hashes[0][2]}/{tiles} {hashes[0][1] / 1e6:.1f} MB")
            s.notes.append("  ".join(line))
        line = [f"threads {thr:2d} inflate       "]
        for mode in (0, 1):
            r = rn.one(OP_IBENCH, mode, struct.pack("<IIII", tiles, 0, 1, reps))
            if r[0] != 0:
                s.fail(f"ibench mode {mode}: {it.err_name(r[0])}")
                continue
            best, first, errs, nthr = struct.unpack_from("<QQII", r[1])
            s.check(errs == 0, f"ibench mode {mode}: {errs} tiles differ")
            line.append(f"inflate {'classic' if mode == 0 else 'state  '} {best / 1000:7.1f} ms")
        s.notes.append("  ".join(line))
    return s


def main():
    ap = it.argparser(__doc__, SUITES)
    ap.add_argument("--base", default="", help="older build dir for the byte-identity suite")
    ap.add_argument("--threads", default="1,12", help="bench: comma list of --threads values")
    ap.add_argument("--tiles", type=int, default=2961, help="bench: tiles (2961 = 4000x3000)")
    ap.add_argument("--reps", type=int, default=5)
    a = ap.parse_args()
    out = it.out_dir(a)
    targets = it.targets_of(a)
    if not a.no_build:
        it.build_codectest(targets, out)
    only = set(x for x in a.only.split(",") if x) or set(SUITES.split(",")) - {"bench", "base"}
    if a.base:
        only.add("base")
    suites = []
    for t in targets:
        runner = it.Runner(t, out, work_name="zlibcheck")
        rng = random.Random(a.seed)
        if "state" in only:
            suites.append(suite_state(runner, rng, t))
        if "limit" in only:
            suites.append(suite_limit(runner, rng, t))
        if "inflate" in only:
            suites.append(suite_inflate(runner, rng, t))
        if "self" in only:
            suites.append(suite_self(runner, t))
        if "base" in only and a.base:
            suites.append(suite_base(runner, (ROOT / a.base).resolve(), random.Random(a.seed + 7), t))
            suites.append(suite_pdndec(runner, (ROOT / a.base).resolve(), random.Random(a.seed + 8), t,
                                       a.fuzz // 4))
        if "bench" in only:
            thr = [int(x) for x in a.threads.split(",") if x]
            suites.append(suite_bench(runner, t, thr, a.tiles, a.reps))
    return 0 if it.report(suites, "zlib_state_check") else 1


if __name__ == "__main__":
    sys.exit(main())
