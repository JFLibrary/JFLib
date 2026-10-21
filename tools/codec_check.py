#!/usr/bin/env python3
"""Codec test driver for jfguiLib (img/: inflate, deflate, checksums, PNG, BMP).

Builds apps/codectest, generates test corpora in Python (stdlib zlib + numpy),
runs them through the assembly codecs in one batch process per target and
compares every result with Python's reference.

  python tools/codec_check.py --target both          # everything, summary at the end
  python tools/codec_check.py --target win --only zlib,png --no-bench
  python tools/codec_check.py --out build/agent-codec --no-build

Suites: sums (CRC32/Adler32 vs zlib, lengths 0..4096), inflate (zlib levels
0-9 x strategies, raw deflate, flushes, window sizes; byte-exact), fuzz (bit
flips / truncations / garbage -> error code, never a crash; anything Python's
zlib accepts must decode identically), deflate (our streams round-trip through
our inflate and zlib.decompress; ratio vs zlib level 6), png (every colour
type / depth / interlace / tRNS combination + fuzz), pngenc (our encoder,
decoded by a Python PNG reader), bmp / bmpenc (decoder variants + encoder),
imgio (img_load / img_save round trips PNG <-> BMP, error codes), bench (MB/s
of inflate / deflate on a ~10 MB sample vs Python's zlib).

Exit code 0 when everything passed.  Crashing records are isolated by
bisection and reported by name.
"""

import argparse
import random
import struct
import subprocess
import sys
import time
import zlib
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent.parent / "legacy"))  # R13: build.py is in legacy/
import build  # noqa: E402  (build.py: exe paths, WSL command lines)
import testenv  # noqa: E402

OP_ZINFLATE, OP_RINFLATE, OP_DEFLATE, OP_CRC, OP_ADLER = 1, 2, 3, 4, 5
OP_PNGDEC, OP_BMPDEC, OP_PNGENC, OP_BMPENC, OP_IMGDEC = 6, 7, 8, 9, 10

ERR_NAMES = {0: "OK", -1: "TRUNC", -2: "HDR", -3: "BLOCK", -4: "CODE", -5: "DIST",
             -6: "OVERFLOW", -7: "CHECKSUM", -8: "NOMEM", -9: "UNSUPPORTED", -10: "CRC",
             -11: "CHUNK", -12: "DIMS", -13: "IO"}


# ---------------------------------------------------------------- batch runner

class Rec:
    __slots__ = ("op", "arg", "data", "check", "name")

    def __init__(self, op, arg, data, check, name):
        self.op, self.arg, self.data, self.check, self.name = op, arg, bytes(data), check, name


def encode_batch(recs):
    parts = [b"CTB1"]
    for r in recs:
        parts.append(struct.pack("<III", r.op, r.arg & 0xFFFFFFFF, len(r.data)))
        parts.append(r.data)
    return b"".join(parts)


def decode_results(blob, n):
    if blob[:4] != b"CTR1":
        raise ValueError("bad result magic")
    out, p = [], 4
    for _ in range(n):
        res, ln = struct.unpack_from("<iI", blob, p)
        p += 8
        out.append((res, blob[p:p + ln]))
        p += ln
    if p != len(blob):
        raise ValueError(f"result size mismatch ({p} != {len(blob)})")
    return out


class Runner:
    def __init__(self, target, out):
        self.target = target
        self.exe = build.exe_path("codectest", target, out)
        self.work = out / target / "codec"
        self.work.mkdir(parents=True, exist_ok=True)
        self.seq = 0

    def rel(self, p):
        return str(Path(p).resolve().relative_to(ROOT)).replace("\\", "/")

    def cmd(self, args):
        dummy = self.rel(self.work / "unused.png")
        return build.run_cmd_for(self.exe, self.target, ["--shot", dummy] + args)

    def run(self, args, timeout=600):
        return subprocess.run(self.cmd(args), cwd=ROOT, capture_output=True, text=True,
                              encoding="utf-8", errors="replace", timeout=timeout)

    def _batch_once(self, recs):
        self.seq += 1
        bin_ = self.work / f"batch{self.seq}.bin"
        res = self.work / f"batch{self.seq}.res"
        bin_.write_bytes(encode_batch(recs))
        if res.exists():
            res.unlink()
        p = self.run(["--batch", self.rel(bin_), "--result", self.rel(res)])
        ok = p.returncode == 0 and res.exists()
        out = decode_results(res.read_bytes(), len(recs)) if ok else None
        bin_.unlink()
        if res.exists():
            res.unlink()
        return out, p

    def batch(self, recs, crashes):
        """Run records; returns list of results (None for a record that crashed)."""
        if not recs:
            return []
        out, p = self._batch_once(recs)
        if out is not None:
            return out
        if len(recs) == 1:
            crashes.append(f"{recs[0].name}: exit {p.returncode} {(p.stdout + p.stderr).strip()[:200]}")
            return [None]
        mid = len(recs) // 2
        return self.batch(recs[:mid], crashes) + self.batch(recs[mid:], crashes)


class Suite:
    def __init__(self, name):
        self.name, self.passed, self.failed, self.msgs, self.notes = name, 0, 0, [], []

    def ok(self):
        self.passed += 1

    def fail(self, msg):
        self.failed += 1
        if len(self.msgs) < 25:
            self.msgs.append(msg)


def run_suite(runner, suite, recs):
    crashes = []
    t0 = time.time()
    results = runner.batch(recs, crashes)
    for r, res in zip(recs, results):
        if res is None:
            suite.fail(f"CRASH {r.name}")
            continue
        try:
            msg = r.check(res[0], res[1])
        except Exception as e:  # noqa: BLE001
            msg = f"checker exception {e!r}"
        if msg:
            suite.fail(f"{r.name}: {msg}")
        else:
            suite.ok()
    for c in crashes[:5]:
        suite.msgs.append("crash detail: " + c)
    return time.time() - t0


# ---------------------------------------------------------------- data sets

WORDS = ("the of and to in is was that for on with as by at from this be are or an have not "
         "window button surface pixel render layout theme scroll canvas brush layer image "
         "assembly register memory stack pointer buffer inflate deflate huffman").split()


def text_data(rng, n):
    out = []
    size = 0
    while size < n:
        w = rng.choice(WORDS)
        if rng.random() < 0.08:
            w = w.capitalize() + (". " if rng.random() < 0.5 else ",\n")
        else:
            w += " "
        out.append(w)
        size += len(w)
    return "".join(out).encode()[:n]


def image_data(rng, w, h, kind=0):
    """Synthetic screenshot-like RGB bytes (flat panels, text-like strokes, gradients)."""
    img = np.zeros((h, w, 3), np.uint8)
    img[:] = (243, 243, 243) if kind == 0 else (32, 32, 32)
    r = np.random.default_rng(rng.randrange(1 << 30))
    for _ in range(max(4, w * h // 20000)):
        x0, y0 = r.integers(0, w), r.integers(0, h)
        x1, y1 = min(w, x0 + r.integers(10, w // 2 + 11)), min(h, y0 + r.integers(10, h // 2 + 11))
        img[y0:y1, x0:x1] = r.integers(0, 256, 3)
    gy = np.linspace(0, 255, h).astype(np.uint8)[:, None]
    img[: h // 6, :, 2] = gy[: h // 6]
    for _ in range(max(10, w * h // 3000)):   # "text": small dark glyph-ish blobs
        x0, y0 = r.integers(0, max(1, w - 8)), r.integers(0, max(1, h - 12))
        glyph = r.integers(0, 2, (12, 8), dtype=np.uint8) * r.integers(60, 200)
        sub = img[y0:y0 + 12, x0:x0 + 8]
        sub[:] = np.minimum(sub, (255 - glyph[: sub.shape[0], : sub.shape[1], None]))
    return img


def png_filter_rows(img):
    """PNG-style Up-filtered bytes of an RGB image (what our encoder feeds deflate)."""
    raw = img.reshape(img.shape[0], -1).astype(np.int16)
    up = np.vstack([np.zeros((1, raw.shape[1]), np.int16), raw[:-1]])
    f = ((raw - up) & 255).astype(np.uint8)
    return b"".join(b"\x02" + row.tobytes() for row in f)


def datasets(rng, big=False):
    d = {
        "empty": b"",
        "one": b"a",
        "zero1": b"\x00",
        "two": b"ab",
        "run64k": b"\x00" * 65536,
        "abab": b"ab" * 40000,
        "rand100": rng.randbytes(100),
        "rand70k": rng.randbytes(70000),
        "text200k": text_data(rng, 200000),
        "text65535": text_data(rng, 65535),
        "text65536": text_data(rng, 65536),
        "text65537": text_data(rng, 65537),
        "img": png_filter_rows(image_data(rng, 320, 200)),
        "imgraw": image_data(rng, 256, 160, 1).tobytes(),
        "lowent": bytes(rng.choice(b"aaaaabbbc") for _ in range(30000)),
        "allbytes": bytes(range(256)) * 300,
    }
    if big:
        d["text1m"] = text_data(rng, 1 << 20)
        d["img1m"] = png_filter_rows(image_data(rng, 800, 440))
    return d


# ---------------------------------------------------------------- checks

def expect_bytes(exp, cap=None):
    def chk(res, out):
        if cap is not None and len(exp) > cap:
            return None if res == -6 else f"expected OVERFLOW, got {res}"
        if res < 0:
            return f"error {res} ({ERR_NAMES.get(res)})"
        if res != len(exp) or out != exp:
            n = next((i for i in range(min(len(out), len(exp))) if out[i] != exp[i]), None)
            return f"mismatch: got {res} bytes, want {len(exp)} (first diff at {n})"
        return None
    return chk


def zcompress(data, level, strategy=0, wbits=15, mem=8, flushes=()):
    c = zlib.compressobj(level, zlib.DEFLATED, wbits, mem, strategy)
    if not flushes:
        return c.compress(data) + c.flush()
    out, p = [], 0
    for at, mode in flushes:
        out.append(c.compress(data[p:at]))
        out.append(c.flush(mode))
        p = at
    out.append(c.compress(data[p:]))
    out.append(c.flush())
    return b"".join(out)


STRATS = {"default": 0, "filtered": zlib.Z_FILTERED, "huffonly": zlib.Z_HUFFMAN_ONLY,
          "rle": zlib.Z_RLE, "fixed": zlib.Z_FIXED}


def suite_sums(rng):
    recs = []
    for n in range(0, 4097):
        data = rng.randbytes(n)
        init = 0 if n % 3 else rng.getrandbits(32)
        crc = zlib.crc32(data, init)
        recs.append(Rec(OP_CRC, init, data, lambda r, o, c=crc: None if r == 0 and o == struct.pack("<I", c)
                        else f"crc {o.hex()} want {c:08x}", f"crc32 len={n} init={init:x}"))
        ainit = 1 if n % 3 else (rng.getrandbits(16) | (rng.getrandbits(16) % 65521) << 16) % (65521 << 16)
        ad = zlib.adler32(data, ainit)
        recs.append(Rec(OP_ADLER, ainit, data, lambda r, o, a=ad: None if r == 0 and o == struct.pack("<I", a)
                        else f"adler {o.hex()} want {a:08x}", f"adler32 len={n} init={ainit:x}"))
    for n in (100000, 1 << 20):
        data = rng.randbytes(n)
        c, a = zlib.crc32(data), zlib.adler32(data)
        recs.append(Rec(OP_CRC, 0, data, lambda r, o, c=c: None if o == struct.pack("<I", c) else "crc", f"crc32 len={n}"))
        recs.append(Rec(OP_ADLER, 1, data, lambda r, o, a=a: None if o == struct.pack("<I", a) else "adler", f"adler32 len={n}"))
    ones = b"\xff" * 100000   # Adler worst case (sums overflow checks)
    recs.append(Rec(OP_ADLER, 1, ones, lambda r, o, a=zlib.adler32(ones): None if o == struct.pack("<I", a) else "adler ff",
                    "adler32 0xff*100000"))
    return recs


def suite_inflate(rng):
    recs = []
    ds = datasets(rng)
    for name, data in ds.items():
        for level in range(10):
            for sname, strat in STRATS.items():
                if level == 0 and strat:
                    continue
                z = zcompress(data, level, strat)
                recs.append(Rec(OP_ZINFLATE, len(data), z, expect_bytes(data), f"inflate {name} L{level} {sname}"))
        z = zcompress(data, 6)
        raw = zcompress(data, 6, wbits=-15)
        recs.append(Rec(OP_RINFLATE, len(data), raw, expect_bytes(data), f"raw {name}"))
        recs.append(Rec(OP_ZINFLATE, len(data) + 100, z, expect_bytes(data), f"inflate {name} cap+100"))
        if data:
            recs.append(Rec(OP_ZINFLATE, len(data) - 1, z, expect_bytes(data, len(data) - 1), f"inflate {name} cap-1"))
        for wb in (9, 10, 12):
            recs.append(Rec(OP_ZINFLATE, len(data), zcompress(data, 6, wbits=wb), expect_bytes(data), f"inflate {name} wbits{wb}"))
        for mem in (1, 9):
            recs.append(Rec(OP_ZINFLATE, len(data), zcompress(data, 9, mem=mem), expect_bytes(data), f"inflate {name} mem{mem}"))
        if len(data) > 10:
            fl = [(len(data) // 3, zlib.Z_SYNC_FLUSH), (len(data) // 2, zlib.Z_FULL_FLUSH),
                  (len(data) // 2 + 1, zlib.Z_SYNC_FLUSH), (2 * len(data) // 3, zlib.Z_BLOCK)]
            recs.append(Rec(OP_ZINFLATE, len(data), zcompress(data, 6, flushes=fl), expect_bytes(data), f"inflate {name} flushes"))
    # hand-made streams
    stored_empty = bytes([0x78, 0x01, 0x01, 0x00, 0x00, 0xFF, 0xFF, 0, 0, 0, 1])
    recs.append(Rec(OP_ZINFLATE, 0, stored_empty, expect_bytes(b""), "stored empty"))
    return recs


def err_or_valid(cap, exp=None, raw=False):
    """Fuzz check: an error code in range, or (when valid) sane output."""
    def chk(res, out):
        if res < 0:
            return None if -8 <= res <= -1 else f"bad error code {res}"
        if res > cap:
            return f"result {res} > cap {cap}"
        if exp is not None and len(out) != res:
            return "length mismatch"
        return None
    return chk


def mutate(rng, z):
    z = bytearray(z)
    kind = rng.randrange(6)
    if kind == 0 and z:
        for _ in range(rng.randint(1, 4)):
            i = rng.randrange(len(z))
            z[i] ^= 1 << rng.randrange(8)
    elif kind == 1:
        z = z[:rng.randrange(len(z) + 1)]
    elif kind == 2 and z:
        i = rng.randrange(len(z))
        z[i] = rng.randrange(256)
    elif kind == 3 and len(z) > 4:
        i = rng.randrange(2, len(z))
        z[i:i + rng.randint(1, 8)] = rng.randbytes(rng.randint(1, 8))
    elif kind == 4 and len(z) > 4:
        i = rng.randrange(2, len(z))
        del z[i:i + rng.randint(1, 16)]
    else:
        i = rng.randrange(len(z) + 1)
        z[i:i] = rng.randbytes(rng.randint(1, 16))
    return bytes(z)


def zlib_reference(z, cap):
    """What Python's zlib makes of a (possibly corrupt) zlib stream: bytes or None."""
    try:
        d = zlib.decompressobj()
        out = d.decompress(z, cap + 1)
        if not d.eof or len(out) > cap:
            return None
        return out
    except zlib.error:
        return None


def suite_fuzz(rng, count):
    recs = []
    base = []
    for name, data in datasets(rng).items():
        if len(data) > 70000:
            data = data[:70000]
        for level, strat in ((1, 0), (6, 0), (9, 0), (6, zlib.Z_FIXED), (6, zlib.Z_HUFFMAN_ONLY), (0, 0)):
            base.append((f"{name}/L{level}s{strat}", data, zcompress(data, level, strat)))
    stats = {"zlib_ok_we_err": 0}
    for i in range(count):
        name, data, z = base[rng.randrange(len(base))]
        m = mutate(rng, z)
        cap = len(data) + rng.choice((0, 0, 0, 1, 64, 4096))
        ref = zlib_reference(m, cap)

        def chk(res, out, ref=ref, cap=cap):
            if res < 0:
                if not -8 <= res <= -1:
                    return f"bad error code {res}"
                if ref is not None:
                    stats["zlib_ok_we_err"] += 1
                    return f"zlib accepts ({len(ref)} bytes) but we give {res}"
                return None
            if res > cap or len(out) != res:
                return f"bad length {res} (cap {cap})"
            if ref is not None and out != ref:
                return "zlib accepts but output differs"
            return None
        recs.append(Rec(OP_ZINFLATE, cap, m, chk, f"fuzz#{i} {name}"))
    for i in range(count // 4):   # raw deflate on garbage / mutated raw streams
        if i % 2:
            m = rng.randbytes(rng.randint(0, 300))
        else:
            name, data, z = base[rng.randrange(len(base))]
            m = mutate(rng, z[2:-4])
        cap = rng.choice((0, 1, 100, 5000, 70000))
        recs.append(Rec(OP_RINFLATE, cap, m, err_or_valid(cap), f"rawfuzz#{i}"))
    return recs


def suite_deflate(rng, big):
    """Round trip of our deflate: batch 1 compresses, results are checked with
    zlib.decompress; batch 2 (built by the caller) feeds them to our inflate."""
    recs = []
    ds = datasets(rng, big)
    info = {}
    for name, data in ds.items():
        for level in range(10):
            def chk(res, out, data=data, name=name, level=level):
                if res != 0:
                    return f"deflate failed {res}"
                try:
                    back = zlib.decompress(out)
                except zlib.error as e:
                    return f"zlib.decompress: {e}"
                if back != data:
                    return "zlib.decompress output differs"
                info[(name, level)] = out
                return None
            recs.append(Rec(OP_DEFLATE, level, data, chk, f"deflate {name} L{level}"))
    return recs, info, ds


# ---------------------------------------------------------------- PNG helpers

PNG_SIG = b"\x89PNG\r\n\x1a\n"
CHANS = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}
ADAM7 = [(0, 0, 8, 8), (4, 0, 8, 8), (0, 4, 4, 8), (2, 0, 4, 4), (0, 2, 2, 4), (1, 0, 2, 2), (0, 1, 1, 2)]


def png_chunk(t, d):
    return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)


def pack_row(samples, depth):
    """samples: 1-D int array (row of w*channels samples) -> bytes (MSB-first bit packing)."""
    samples = np.asarray(samples)
    if depth == 16:
        return samples.astype(">u2").tobytes()
    if depth == 8:
        return samples.astype(np.uint8).tobytes()
    per = 8 // depth
    n = len(samples)
    pad = (-n) % per
    s = np.concatenate([samples.astype(np.uint16), np.zeros(pad, np.uint16)]).reshape(-1, per)
    out = np.zeros(len(s), np.uint16)
    for k in range(per):
        out |= s[:, k] << (8 - depth * (k + 1))
    return out.astype(np.uint8).tobytes()


def filter_row(ft, row, prior, bpp):
    x = np.frombuffer(row, np.uint8).astype(np.int16)
    b = np.frombuffer(prior, np.uint8).astype(np.int16) if prior is not None else np.zeros_like(x)
    a = np.concatenate([np.zeros(bpp, np.int16), x])[: len(x)]
    c = np.concatenate([np.zeros(bpp, np.int16), b])[: len(b)]
    if ft == 0:
        f = x
    elif ft == 1:
        f = x - a
    elif ft == 2:
        f = x - b
    elif ft == 3:
        f = x - ((a + b) >> 1)
    else:
        p = a + b - c
        pa, pb, pc = np.abs(p - a), np.abs(p - b), np.abs(p - c)
        pred = np.where((pa <= pb) & (pa <= pc), a, np.where(pb <= pc, b, c))
        f = x - pred
    return bytes([ft]) + (f & 255).astype(np.uint8).tobytes()


def png_make(samples, ctype, depth, interlace=0, plte=None, trns=None, filt="cycle", level=6,
             idat_split=0, extra_chunks=(), rng=None):
    """samples: (h, w, channels) int array -> PNG file bytes (Python reference writer)."""
    h, w, ch = samples.shape
    bpp = max(1, depth * ch // 8)
    raw = []
    rowno = 0
    for (x0, y0, dx, dy) in (ADAM7 if interlace else [(0, 0, 1, 1)]):
        sub = samples[y0::dy, x0::dx]
        if sub.shape[0] == 0 or sub.shape[1] == 0:
            continue
        prior = None
        for r in sub:
            row = pack_row(r.reshape(-1), depth)
            ft = rowno % 5 if filt == "cycle" else rng.randrange(5) if filt == "random" else filt
            raw.append(filter_row(ft, row, prior, bpp))
            prior = row
            rowno += 1
    z = zlib.compress(b"".join(raw), level)
    out = [PNG_SIG, png_chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, depth, ctype, 0, 0, interlace))]
    out += [png_chunk(t, d) for t, d in extra_chunks]
    if plte is not None:
        out.append(png_chunk(b"PLTE", bytes(np.asarray(plte, np.uint8).reshape(-1))))
    if trns is not None:
        out.append(png_chunk(b"tRNS", trns))
    step = idat_split or max(1, len(z))
    for i in range(0, max(1, len(z)), step):
        out.append(png_chunk(b"IDAT", z[i:i + step]))
    out.append(png_chunk(b"IEND", b""))
    return b"".join(out)


def premul_bgra(r, g, b, a):
    """straight R, G, B, A arrays -> expected PARGB pixels as (.., 4) uint8 B,G,R,A."""
    a = np.asarray(a).astype(np.int64)
    pm = lambda c: (np.asarray(c).astype(np.int64) * a + 127) // 255  # noqa: E731
    return np.stack([pm(b), pm(g), pm(r), a], -1).astype(np.uint8)


def png_expect(samples, ctype, depth, plte=None, trns=None):
    h, w, _ = samples.shape
    s = samples.astype(np.int64)
    full = np.full((h, w), 255, np.int64)

    def to8(v):
        if depth == 16:
            return (v * 255 + 32895) >> 16      # round(v / 257), like img_u16_to_u8
        if depth < 8:
            return v * 255 // ((1 << depth) - 1)
        return v
    if ctype == 0:
        g = to8(s[..., 0])
        a = full.copy()
        if trns is not None and len(trns) >= 2:
            a[s[..., 0] == struct.unpack(">H", trns[:2])[0]] = 0
        return premul_bgra(g, g, g, a)
    if ctype == 2:
        a = full.copy()
        if trns is not None and len(trns) >= 6:
            kr, kg, kb = struct.unpack(">HHH", trns[:6])
            a[(s[..., 0] == kr) & (s[..., 1] == kg) & (s[..., 2] == kb)] = 0
        return premul_bgra(to8(s[..., 0]), to8(s[..., 1]), to8(s[..., 2]), a)
    if ctype == 3:
        pal = np.zeros((256, 4), np.int64)
        pal[:, 3] = 255
        p = np.asarray(plte, np.int64).reshape(-1, 3)
        pal[:len(p), :3] = p
        if trns is not None:
            t = np.frombuffer(trns, np.uint8)[:len(p)]
            pal[:len(t), 3] = t
        e = pal[s[..., 0]]
        return premul_bgra(e[..., 0], e[..., 1], e[..., 2], e[..., 3])
    if ctype == 4:
        g, a = to8(s[..., 0]), to8(s[..., 1])
        return premul_bgra(g, g, g, a)
    r, g, b, a = (to8(s[..., k]) for k in range(4))
    return premul_bgra(r, g, b, a)


def expect_image(exp):
    """Checker for decode ops: exp = (h, w, 4) uint8 B,G,R,A array."""
    h, w, _ = exp.shape

    def chk(res, out):
        if res != 0:
            return f"error {res} ({ERR_NAMES.get(res)})"
        ow, oh = struct.unpack_from("<II", out)
        if (ow, oh) != (w, h):
            return f"size {ow}x{oh}, want {w}x{h}"
        got = np.frombuffer(out, np.uint8, offset=8).reshape(h, w, 4)
        if not np.array_equal(got, exp):
            d = np.argwhere(np.any(got != exp, -1))
            y, x = d[0]
            return f"{len(d)} pixels differ, first ({x},{y}) got {got[y, x].tolist()} want {exp[y, x].tolist()}"
        return None
    return chk


def expect_err(*codes):
    def chk(res, out):
        return None if res in codes else f"got {res} ({ERR_NAMES.get(res)}), want {codes}"
    return chk


def any_result(res, out):
    """Fuzz checker: a valid error code, or a well-formed image."""
    if res == 0:
        if len(out) < 8:
            return "short output"
        w, h = struct.unpack_from("<II", out)
        return None if len(out) == 8 + w * h * 4 else "size mismatch"
    return None if -13 <= res <= -1 else f"bad error code {res}"


def rand_samples(rng, h, w, ch, depth, smooth=False):
    r = np.random.default_rng(rng.randrange(1 << 30))
    mx = (1 << depth) - 1
    if smooth:
        yy, xx = np.mgrid[0:h, 0:w]
        s = np.stack([(xx * (k + 1) * 7 + yy * (3 - k) * 5) for k in range(ch)], -1) % (mx + 1)
        return np.clip(s + r.integers(0, 3, (h, w, ch)), 0, mx)
    return r.integers(0, mx + 1, (h, w, ch))


PNG_TYPES = [(0, 1), (0, 2), (0, 4), (0, 8), (0, 16), (2, 8), (2, 16), (3, 1), (3, 2), (3, 4), (3, 8),
             (4, 8), (4, 16), (6, 8), (6, 16)]
PNG_SIZES = [(1, 1), (2, 1), (1, 3), (3, 2), (7, 5), (9, 9), (17, 13), (33, 8), (64, 3), (5, 40)]


def suite_png(rng, fuzz):
    recs, valid = [], []
    for ctype, depth in PNG_TYPES:
        ch = CHANS[ctype]
        for (w, h) in PNG_SIZES:
            for interlace in (0, 1):
                s = rand_samples(rng, h, w, ch, depth, (w * h) % 2 == 0)
                plte = trns = None
                if ctype == 3:
                    npal = 1 << depth
                    plte = [(rng.randrange(256), rng.randrange(256), rng.randrange(256)) for _ in range(npal)]
                    trns = bytes(rng.randrange(256) for _ in range(rng.randint(0, npal))) or None
                elif ctype in (0, 2) and (w * h + interlace) % 3 == 0:
                    trns = b"".join(struct.pack(">H", int(x)) for x in s[0, 0][:ch])
                filt = rng.choice(["cycle", "random", 0, 1, 2, 3, 4])
                data = png_make(s, ctype, depth, interlace, plte, trns, filt, level=rng.choice((0, 1, 6, 9)),
                                idat_split=rng.choice((0, 0, 1, 7, 100)), rng=rng)
                name = f"png ct{ctype} d{depth} {w}x{h} il{interlace} f{filt}"
                recs.append(Rec(OP_PNGDEC, 0, data, expect_image(png_expect(s, ctype, depth, plte, trns)), name))
                valid.append((name, data))
    s = rand_samples(rng, 9, 11, 1, 8)
    plte = [(i, 255 - i, i // 2) for i in range(100)]
    data = png_make(s, 3, 8, plte=plte, extra_chunks=[(b"gAMA", b"\0\0\xb1\x8f"), (b"tEXt", b"k\0v"), (b"zzZz", b"x")])
    recs.append(Rec(OP_PNGDEC, 0, data, expect_image(png_expect(s, 3, 8, plte)), "png short palette + ancillary"))
    s = rand_samples(rng, 300, 401, 4, 8, smooth=True)
    data = png_make(s, 6, 8, filt="random", rng=rng)
    recs.append(Rec(OP_PNGDEC, 0, data, expect_image(png_expect(s, 6, 8)), "png 401x300 rgba"))
    recs.append(Rec(OP_IMGDEC, 0, data, expect_image(png_expect(s, 6, 8)), "img_decode png"))
    s = rand_samples(rng, 257, 129, 3, 16, smooth=True)
    data = png_make(s, 2, 16, interlace=1, filt="random", rng=rng)
    recs.append(Rec(OP_PNGDEC, 0, data, expect_image(png_expect(s, 2, 16)), "png 129x257 rgb16 adam7"))

    # ---- error cases
    base = png_make(rand_samples(rng, 5, 6, 3, 8), 2, 8)
    ihdr_end = 8 + 25

    def with_ihdr(w, h, depth, ctype, il=0, comp=0):
        return PNG_SIG + png_chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, depth, ctype, comp, 0, il)) + base[ihdr_end:]
    recs.append(Rec(OP_PNGDEC, 0, b"\x89PNG\r\n\x1a\x0b" + base[8:], expect_err(-2), "png bad signature"))
    recs.append(Rec(OP_PNGDEC, 0, base[:5], expect_err(-1), "png 5 bytes"))
    recs.append(Rec(OP_PNGDEC, 0, PNG_SIG, expect_err(-1), "png signature only"))
    bad = bytearray(base)
    bad[20] ^= 1
    recs.append(Rec(OP_PNGDEC, 0, bytes(bad), expect_err(-10), "png IHDR crc"))
    bad = bytearray(base)
    bad[ihdr_end + 10] ^= 0x40
    recs.append(Rec(OP_PNGDEC, 0, bytes(bad), expect_err(-10), "png IDAT crc"))
    for cut in (ihdr_end - 3, ihdr_end + 5, len(base) - 20, len(base) - 13):
        recs.append(Rec(OP_PNGDEC, 0, base[:cut], expect_err(-1), f"png truncated at {cut}"))
    recs.append(Rec(OP_PNGDEC, 0, base[:-12], any_result, "png without IEND"))
    recs.append(Rec(OP_PNGDEC, 0, with_ihdr(0, 5, 8, 2), expect_err(-12), "png width 0"))
    recs.append(Rec(OP_PNGDEC, 0, with_ihdr(40000, 5, 8, 2), expect_err(-12), "png width 40000"))
    recs.append(Rec(OP_PNGDEC, 0, with_ihdr(30000, 30000, 8, 2), expect_err(-12), "png 30000^2"))
    recs.append(Rec(OP_PNGDEC, 0, with_ihdr(6, 5, 3, 2), expect_err(-2), "png depth 3"))
    recs.append(Rec(OP_PNGDEC, 0, with_ihdr(6, 5, 16, 3), expect_err(-2), "png palette 16 bit"))
    recs.append(Rec(OP_PNGDEC, 0, with_ihdr(6, 5, 8, 5), expect_err(-2), "png colour type 5"))
    recs.append(Rec(OP_PNGDEC, 0, with_ihdr(6, 5, 8, 2, il=2), expect_err(-2), "png interlace 2"))
    recs.append(Rec(OP_PNGDEC, 0, with_ihdr(6, 5, 8, 2, comp=1), expect_err(-2), "png compression 1"))
    recs.append(Rec(OP_PNGDEC, 0, with_ihdr(6, 6, 8, 2), expect_err(-1), "png IDAT too short"))
    recs.append(Rec(OP_PNGDEC, 0, with_ihdr(6, 4, 8, 2), expect_err(-6), "png IDAT too long"))
    recs.append(Rec(OP_PNGDEC, 0, with_ihdr(6, 5, 8, 3), expect_err(-11), "png palette missing"))
    s = rand_samples(rng, 4, 4, 3, 8)
    raw = b"".join(bytes([5]) + pack_row(r.reshape(-1), 8) for r in s)
    nf = (PNG_SIG + png_chunk(b"IHDR", struct.pack(">IIBBBBB", 4, 4, 8, 2, 0, 0, 0)) +
          png_chunk(b"IDAT", zlib.compress(raw)) + png_chunk(b"IEND", b""))
    recs.append(Rec(OP_PNGDEC, 0, nf, expect_err(-3), "png filter type 5"))
    recs.append(Rec(OP_PNGDEC, 0, base[:ihdr_end] + png_chunk(b"ABCD", b"xy") + base[ihdr_end:], expect_err(-9),
                    "png unknown critical chunk"))
    recs.append(Rec(OP_PNGDEC, 0, PNG_SIG + base[ihdr_end:], expect_err(-11), "png IDAT before IHDR"))
    s = rand_samples(rng, 3, 3, 3, 8)
    tr = bytearray(png_make(s, 2, 8, trns=struct.pack(">HHH", *[int(v) for v in s[0, 0]])))
    tr[tr.index(b"tRNS") + 4] ^= 0xFF
    recs.append(Rec(OP_PNGDEC, 0, bytes(tr), expect_image(png_expect(s, 2, 8)), "png damaged tRNS ignored"))
    recs.append(Rec(OP_IMGDEC, 0, b"II*\x00....", expect_err(-9), "img_decode unknown (TIFF)"))
    recs.append(Rec(OP_IMGDEC, 0, b"B", expect_err(-1), "img_decode 1 byte"))

    # ---- fuzz: raw mutations (mostly CRC errors) and CRC-fixed mutations of chunk contents
    for i in range(fuzz):
        name, data = valid[rng.randrange(len(valid))]
        m = mutate(rng, data) if i % 2 else png_fuzz_fixcrc(rng, data)
        recs.append(Rec(OP_PNGDEC, 0, m, any_result, f"pngfuzz#{i} {name}"))
    return recs


def png_chunks(data):
    p, out = 8, []
    while p + 12 <= len(data):
        n = struct.unpack_from(">I", data, p)[0]
        out.append((data[p + 4:p + 8], data[p + 8:p + 8 + n]))
        p += 12 + n
    return out


def png_fuzz_fixcrc(rng, data):
    """Mutate chunk contents (IHDR fields, pixel data, PLTE, tRNS) and recompute the CRCs."""
    chunks = png_chunks(data)
    k = rng.randrange(len(chunks))
    t, d = chunks[k]
    if t == b"IDAT" and rng.random() < 0.6:
        try:
            raw = bytearray(zlib.decompress(b"".join(c[1] for c in chunks if c[0] == b"IDAT")))
            for _ in range(rng.randint(1, 6)):     # corrupt filter bytes / samples, recompress
                if raw:
                    raw[rng.randrange(len(raw))] = rng.randrange(256)
            if rng.random() < 0.3:
                raw = raw[:rng.randrange(len(raw) + 1)]
            elif rng.random() < 0.2:
                raw += rng.randbytes(rng.randint(1, 50))
            chunks = [c for c in chunks if c[0] != b"IDAT"]
            chunks.insert(len(chunks) - 1, (b"IDAT", zlib.compress(bytes(raw))))
        except zlib.error:
            pass
    else:
        chunks[k] = (t, mutate(rng, d) if d else rng.randbytes(4))
    return PNG_SIG + b"".join(png_chunk(t, d) for t, d in chunks)


# ---------------------------------------------------------------- PNG reader (encoder tests)

def png_read(data):
    """Reader for our encoder's output (8-bit, colour types 0/2/6, no interlace), checking
    every CRC.  Returns (colour type, (h, w, channels) uint8 array)."""
    if data[:8] != PNG_SIG:
        raise ValueError("signature")
    p, idat, hdr, end = 8, [], None, False
    while p < len(data):
        n = struct.unpack_from(">I", data, p)[0]
        t = data[p + 4:p + 8]
        d = data[p + 8:p + 8 + n]
        if struct.unpack_from(">I", data, p + 8 + n)[0] != zlib.crc32(t + d) & 0xFFFFFFFF:
            raise ValueError(f"CRC of {t}")
        if t == b"IHDR":
            hdr = struct.unpack(">IIBBBBB", d)
        elif t == b"IDAT":
            idat.append(d)
        elif t == b"IEND":
            end = True
        p += 12 + n
        if end:
            break
    if not end or p != len(data):
        raise ValueError("IEND / trailing data")
    w, h, depth, ctype, comp, flt, il = hdr
    if depth != 8 or il or comp or flt or ctype not in (0, 2, 6):
        raise ValueError(f"unexpected IHDR {hdr}")
    ch = CHANS[ctype]
    raw = zlib.decompress(b"".join(idat))
    rb = w * ch
    if len(raw) != h * (rb + 1):
        raise ValueError("raw size")
    out = np.zeros((h, rb), np.uint8)
    prior = [0] * rb
    for y in range(h):
        ft = raw[y * (rb + 1)]
        x = list(raw[y * (rb + 1) + 1: (y + 1) * (rb + 1)])
        if ft > 4:
            raise ValueError(f"filter {ft}")
        for i in range(rb):
            a = x[i - ch] if i >= ch else 0
            b = prior[i]
            c = prior[i - ch] if i >= ch else 0
            if ft == 0:
                pr = 0
            elif ft == 1:
                pr = a
            elif ft == 2:
                pr = b
            elif ft == 3:
                pr = (a + b) >> 1
            else:
                pp = a + b - c
                pa, pb, pc = abs(pp - a), abs(pp - b), abs(pp - c)
                pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
            x[i] = (x[i] + pr) & 255
        out[y] = x
        prior = x
    return ctype, out.reshape(h, w, ch)


def unpremul_ref(bgra):
    """PARGB (B,G,R,A) -> straight R,G,B,A with round(c * 255 / a), clamped."""
    a = bgra[..., 3].astype(np.int64)
    rgb = bgra[..., 2::-1].astype(np.int64)
    safe = np.maximum(a, 1)[..., None]
    st = np.minimum(255, (rgb * 255 + safe // 2) // safe)
    st = np.where(a[..., None] == 0, 0, np.where(a[..., None] == 255, rgb, st))
    return np.concatenate([st, a[..., None]], -1).astype(np.uint8)


def make_surface(rng, w, h, fmt, alpha_kind):
    """Random surface for encode ops: (packed bytes, array) -- A8: (h, w); else (h, w, 4) B,G,R,A."""
    r = np.random.default_rng(rng.randrange(1 << 30))
    yy, xx = np.mgrid[0:h, 0:w]
    if fmt == 2:
        px = np.where((xx + yy) % 3 == 0, r.integers(0, 256, (h, w)), (xx * 3 + yy) % 256).astype(np.uint8)
        return px.tobytes(), px
    rgb = r.integers(0, 256, (h, w, 3))
    rgb[: h // 2] = np.stack([(xx * 5) % 256, (yy * 3) % 256, (xx + yy) % 256], -1)[: h // 2]
    if alpha_kind == "opaque":
        a = np.full((h, w), 255)
    elif alpha_kind == "random":
        a = r.integers(0, 256, (h, w))
        a[0, 0] = 0
    else:
        a = np.where(r.random((h, w)) < 0.5, 255, r.integers(0, 3, (h, w)) * 127)
    if fmt == 1:
        rgb = (rgb * a[..., None] + 127) // 255
    else:
        a = r.integers(0, 256, (h, w))           # XRGB: alpha byte is garbage
    bgra = np.concatenate([rgb[..., ::-1], a[..., None]], -1).astype(np.uint8)
    return bgra.tobytes(), bgra


ENC_SIZES = [(1, 1), (2, 3), (5, 1), (1, 7), (16, 16), (17, 5), (31, 9), (100, 37), (257, 3)]


def suite_pngenc(rng):
    recs = []
    cases = [(w, h, f, ak) for (w, h) in ENC_SIZES
             for (f, ak) in ((0, "x"), (1, "opaque"), (1, "random"), (1, "mixed"), (2, ""))]
    cases.append((640, 360, 1, "mixed"))
    for (w, h, fmt, ak) in cases:
        packed, arr = make_surface(rng, w, h, fmt, ak)

        def chk(res, out, fmt=fmt, arr=arr, ak=ak):
            if res != 0:
                return f"encode failed {res}"
            try:
                ctype, img = png_read(out)
            except (ValueError, zlib.error, struct.error) as e:
                return f"reader: {e}"
            if fmt == 2:
                want_ct, want = 0, arr[..., None]
            elif fmt == 0 or ak == "opaque":
                want_ct, want = 2, arr[..., 2::-1]
            else:
                want_ct, want = 6, unpremul_ref(arr)
            if ctype != want_ct:
                return f"colour type {ctype}, want {want_ct}"
            if not np.array_equal(img, want):
                d = np.argwhere(np.any(img != want, -1))
                y, x = d[0]
                return f"{len(d)} px differ, first ({x},{y}) got {img[y, x].tolist()} want {want[y, x].tolist()}"
            return None
        recs.append(Rec(OP_PNGENC, 0, struct.pack("<III", w, h, fmt) + packed, chk, f"pngenc {w}x{h} fmt{fmt} {ak}"))
    return recs


# ---------------------------------------------------------------- BMP

def bmp_make(pix, bpp, *, pal=None, clr_used=0, hsz=40, comp=0, masks=None, topdown=False, core=False):
    """Reference BMP writer. pix: (h, w) indices for bpp <= 8, (h, w) packed values for 16/32,
    (h, w, 3) B,G,R for 24."""
    h, w = pix.shape[:2]
    stride = ((w * bpp + 31) // 32) * 4
    rows = []
    for y in range(h):
        r = pix[y]
        if bpp <= 8:
            b = pack_row(np.asarray(r), bpp)
        elif bpp == 16:
            b = np.asarray(r).astype("<u2").tobytes()
        elif bpp == 24:
            b = np.asarray(r).astype(np.uint8).tobytes()
        else:
            b = np.asarray(r).astype("<u4").tobytes()
        rows.append(b + b"\0" * (stride - len(b)))
    if not topdown:
        rows = rows[::-1]
    data = b"".join(rows)
    if core:
        hdr = struct.pack("<IHHHH", 12, w, h, 1, bpp)
        palb = b"".join(bytes([bb, gg, rr]) for (rr, gg, bb) in (pal or []))
    else:
        hdr = struct.pack("<IiiHHIIiiII", hsz, w, -h if topdown else h, 1, bpp, comp, len(data), 2835, 2835,
                          clr_used, 0)
        after = b""
        if masks is not None:
            mb = struct.pack("<IIII", *(list(masks) + [0] * (4 - len(masks))))
            if hsz >= 56:
                hdr += mb
            elif hsz == 52:
                hdr += mb[:12]
            else:
                after = mb[:4 * len(masks)]
        hdr += b"\0" * (hsz - len(hdr)) + after
        palb = b"".join(bytes([bb, gg, rr, 0]) for (rr, gg, bb) in (pal or []))
    off = 14 + len(hdr) + len(palb)
    return b"BM" + struct.pack("<IHHI", off + len(data), 0, 0, off) + hdr + palb + data


def mask_scale(v, mask):
    if mask == 0:
        return None
    sh = (mask & -mask).bit_length() - 1
    bits = mask.bit_length() - sh
    v = (v & mask) >> sh
    if bits > 8:
        return v >> (bits - 8)
    mx = (1 << bits) - 1
    return (v * 255 + mx // 2) // mx


def bmp_expect_masked(px, masks):
    px = px.astype(np.int64)
    ch = []
    for k in range(4):
        s = mask_scale(px, masks[k] if k < len(masks) else 0)
        ch.append(np.full(px.shape, 255 if k == 3 else 0, np.int64) if s is None else s)
    return premul_bgra(ch[0], ch[1], ch[2], ch[3])


def suite_bmp(rng, fuzz):
    recs, valid = [], []

    def add(data, exp, name):
        recs.append(Rec(OP_BMPDEC, 0, data, expect_image(exp), name))
        valid.append((name, data))
    r = np.random.default_rng(rng.randrange(1 << 30))
    for (w, h) in [(1, 1), (3, 2), (5, 4), (7, 3), (31, 5), (33, 17), (64, 2)]:
        opaque = np.full((h, w), 255)
        for td in (False, True):
            for bpp in (1, 4, 8):
                n = 1 << bpp
                pal = [(rng.randrange(256), rng.randrange(256), rng.randrange(256)) for _ in range(n)]
                idx = r.integers(0, n, (h, w))
                npal = n if bpp == 1 else rng.choice((n, max(1, n // 2)))
                full = np.array(pal, np.int64)
                full[npal:] = 0
                e = full[idx]
                add(bmp_make(idx, bpp, pal=pal[:npal], clr_used=0 if npal == n else npal, topdown=td),
                    premul_bgra(e[..., 0], e[..., 1], e[..., 2], opaque), f"bmp {bpp}bit {w}x{h} td{td} pal{npal}")
            bgr = r.integers(0, 256, (h, w, 3))
            add(bmp_make(bgr, 24, topdown=td), premul_bgra(bgr[..., 2], bgr[..., 1], bgr[..., 0], opaque),
                f"bmp 24bit {w}x{h} td{td}")
            px = r.integers(0, 1 << 32, (h, w), dtype=np.uint64)
            add(bmp_make(px, 32, topdown=td), bmp_expect_masked(px, [0xFF0000, 0xFF00, 0xFF]),
                f"bmp 32bit BI_RGB {w}x{h} td{td}")
            p16 = r.integers(0, 1 << 16, (h, w))
            add(bmp_make(p16, 16, topdown=td), bmp_expect_masked(p16, [0x7C00, 0x3E0, 0x1F]),
                f"bmp 16bit 555 {w}x{h} td{td}")
            m565 = [0xF800, 0x7E0, 0x1F]
            add(bmp_make(p16, 16, comp=3, masks=m565, topdown=td), bmp_expect_masked(p16, m565),
                f"bmp 16bit 565 {w}x{h} td{td}")
            m4444 = [0xF00, 0xF0, 0xF, 0xF000]
            add(bmp_make(p16, 16, comp=6, masks=m4444, topdown=td), bmp_expect_masked(p16, m4444),
                f"bmp 16bit 4444 {w}x{h} td{td}")
            for hsz, masks, comp in ((108, [0xFF0000, 0xFF00, 0xFF, 0xFF000000], 3),
                                     (124, [0xFF0000, 0xFF00, 0xFF, 0xFF000000], 3),
                                     (56, [0xFF, 0xFF00, 0xFF0000, 0xFF000000], 3),
                                     (52, [0xFF0000, 0xFF00, 0xFF], 3),
                                     (40, [0x3FF00000, 0xFFC00, 0x3FF], 3),
                                     (40, [0xFF000000, 0xFF0000, 0xFF00, 0xFF], 6)):
                add(bmp_make(px, 32, hsz=hsz, comp=comp, masks=masks, topdown=td), bmp_expect_masked(px, masks),
                    f"bmp 32bit hsz{hsz} comp{comp} {w}x{h} td{td}")
        idx = r.integers(0, 256, (h, w))
        pal = [(rng.randrange(256), rng.randrange(256), rng.randrange(256)) for _ in range(256)]
        e = np.array(pal)[idx]
        add(bmp_make(idx, 8, pal=pal, core=True), premul_bgra(e[..., 0], e[..., 1], e[..., 2], opaque),
            f"bmp core 8bit {w}x{h}")
        bgr = r.integers(0, 256, (h, w, 3))
        add(bmp_make(bgr, 24, core=True), premul_bgra(bgr[..., 2], bgr[..., 1], bgr[..., 0], opaque),
            f"bmp core 24bit {w}x{h}")
    bgr = r.integers(0, 256, (3, 5, 3))
    exp = premul_bgra(bgr[..., 2], bgr[..., 1], bgr[..., 0], np.full((3, 5), 255))
    good = bmp_make(bgr, 24)
    add(good[:-1], exp, "bmp unpadded last row")
    recs.append(Rec(OP_IMGDEC, 0, good, expect_image(exp), "img_decode bmp"))
    # errors
    recs.append(Rec(OP_BMPDEC, 0, b"BX" + good[2:], expect_err(-2), "bmp bad magic"))
    recs.append(Rec(OP_BMPDEC, 0, good[:10], expect_err(-1), "bmp 10 bytes"))
    recs.append(Rec(OP_BMPDEC, 0, good[:40], expect_err(-1), "bmp truncated header"))
    recs.append(Rec(OP_BMPDEC, 0, good[:-10], expect_err(-1), "bmp truncated pixels"))
    idx8 = r.integers(0, 4, (3, 5))
    # RLE8 is decoded since the CA package (tools/bmp_check.py tests it); raw rows labelled RLE8
    # are garbage RLE data: any error code or a well-formed image
    recs.append(Rec(OP_BMPDEC, 0, bmp_make(idx8, 8, pal=[(1, 2, 3)] * 4, comp=1), any_result, "bmp RLE8 garbage"))
    recs.append(Rec(OP_BMPDEC, 0, bmp_make(idx8, 8, pal=[(1, 2, 3)] * 4, comp=12), expect_err(-9), "bmp CMYK RLE8"))
    recs.append(Rec(OP_BMPDEC, 0, bmp_make(idx8, 2, pal=[(1, 2, 3)] * 4), expect_err(-9), "bmp 2 bit"))
    recs.append(Rec(OP_BMPDEC, 0, bmp_make(bgr, 24, comp=3, masks=[1, 2, 4]), expect_err(-9), "bmp 24 bit bitfields"))
    z = bytearray(good)
    z[18:22] = struct.pack("<i", 0)
    recs.append(Rec(OP_BMPDEC, 0, bytes(z), expect_err(-12), "bmp width 0"))
    z[18:22] = struct.pack("<i", 40000)
    recs.append(Rec(OP_BMPDEC, 0, bytes(z), expect_err(-12), "bmp width 40000"))
    z[18:22] = struct.pack("<i", 5)
    z[22:26] = struct.pack("<i", -(1 << 31))
    recs.append(Rec(OP_BMPDEC, 0, bytes(z), expect_err(-12), "bmp height INT_MIN"))
    z = bytearray(good)
    z[14:18] = struct.pack("<I", 20)
    recs.append(Rec(OP_BMPDEC, 0, bytes(z), expect_err(-2), "bmp header size 20"))
    for i in range(fuzz):
        name, data = valid[rng.randrange(len(valid))]
        m = mutate(rng, data) if i % 3 else bmp_fuzz_header(rng, data)
        recs.append(Rec(OP_BMPDEC, 0, m, any_result, f"bmpfuzz#{i} {name}"))
    return recs


def bmp_fuzz_header(rng, data):
    d = bytearray(data)
    for _ in range(rng.randint(1, 3)):
        off = rng.choice((2, 10, 14, 18, 22, 26, 28, 30, 46, 54, 58, 62, 66))
        if off + 4 <= len(d):
            d[off:off + 4] = struct.pack("<I", rng.choice((0, 1, 0xFFFFFFFF, 0x7FFFFFFF, 40, 124, 32, 8, 16,
                                                          rng.getrandbits(32), rng.getrandbits(8))))
    return bytes(d)


def bmp_read(data):
    """Parse our encoder's BMP output -> (bpp, (h, w, 4) B,G,R,A array)."""
    if data[:2] != b"BM":
        raise ValueError("magic")
    size, _, _, off = struct.unpack_from("<IHHI", data, 2)
    if size != len(data):
        raise ValueError("file size field")
    hsz, w, h, planes, bpp, comp = struct.unpack_from("<IiiHHI", data, 14)
    stride = ((w * bpp + 31) // 32) * 4
    if off + stride * h != len(data):
        raise ValueError("pixel data size")
    out = np.zeros((h, w, 4), np.uint8)
    for y in range(h):
        row = data[off + (h - 1 - y) * stride: off + (h - y) * stride]
        if bpp == 24:
            if hsz != 40 or comp != 0:
                raise ValueError("24-bit header")
            out[y, :, :3] = np.frombuffer(row, np.uint8, w * 3).reshape(w, 3)
            out[y, :, 3] = 255
        elif bpp == 32:
            masks = struct.unpack_from("<IIII", data, 54)
            if hsz != 108 or comp != 3 or masks != (0xFF0000, 0xFF00, 0xFF, 0xFF000000):
                raise ValueError(f"32-bit header {hsz} {comp} {masks}")
            out[y] = np.frombuffer(row, np.uint8, w * 4).reshape(w, 4)
        else:
            raise ValueError(f"bpp {bpp}")
    return bpp, out


def suite_bmpenc(rng):
    recs = []
    for (w, h) in [(1, 1), (2, 3), (5, 1), (7, 7), (33, 4), (100, 37)]:
        for fmt, ak in ((0, "x"), (1, "opaque"), (1, "random"), (1, "mixed"), (2, "")):
            packed, arr = make_surface(rng, w, h, fmt, ak)

            def chk(res, out, fmt=fmt, arr=arr, ak=ak):
                if res != 0:
                    return f"encode failed {res}"
                try:
                    bpp, img = bmp_read(out)
                except (ValueError, struct.error) as e:
                    return f"reader: {e}"
                if fmt == 2:
                    want_bpp, want = 24, np.stack([arr, arr, arr, np.full(arr.shape, 255, np.uint8)], -1)
                elif fmt == 0 or ak == "opaque":
                    want_bpp, want = 24, arr.copy()
                    want[..., 3] = 255
                else:
                    st = unpremul_ref(arr)
                    want_bpp, want = 32, np.stack([st[..., 2], st[..., 1], st[..., 0], st[..., 3]], -1)
                if bpp != want_bpp:
                    return f"bpp {bpp}, want {want_bpp}"
                if not np.array_equal(img, want):
                    d = np.argwhere(np.any(img != want, -1))
                    y, x = d[0]
                    return f"{len(d)} px differ, first ({x},{y}) got {img[y, x].tolist()} want {want[y, x].tolist()}"
                return None
            recs.append(Rec(OP_BMPENC, 0, struct.pack("<III", w, h, fmt) + packed, chk, f"bmpenc {w}x{h} fmt{fmt} {ak}"))
    return recs


# ---------------------------------------------------------------- img_load / img_save

def premul_ref(st):
    """straight (h, w, 4) R,G,B,A -> premultiplied (h, w, 4) R,G,B,A."""
    a = st[..., 3:4].astype(np.int64)
    return np.concatenate([(st[..., :3].astype(np.int64) * a + 127) // 255, a], -1).astype(np.uint8)


def suite_imgio(runner, rng, suite):
    """PNG -> BMP -> PNG (-> BMP) through img_load / img_save (--convert), plus error codes."""
    w, h = 37, 23
    st = rand_samples(rng, h, w, 4, 8, smooth=True).astype(np.uint8)
    st[::3, ::2, 3] = 255
    st[1::4, 1::3, 3] = 0
    st[2::5, ::4, 3] = rng.randrange(1, 255)
    work = runner.work
    src = work / "io_src.png"
    src.write_bytes(png_make(st.astype(np.int64), 6, 8, filt="random", rng=rng))
    steps = [("io_src.png", "io_a.bmp"), ("io_a.bmp", "io_b.PNG"), ("io_b.PNG", "io_c.Bmp")]
    pm = premul_ref(st)                        # what the surface holds after every load
    want_st = unpremul_ref(np.stack([pm[..., 2], pm[..., 1], pm[..., 0], pm[..., 3]], -1))
    for a, b in steps:
        p = runner.run(["--convert", runner.rel(work / a), "--to", runner.rel(work / b)])
        if p.returncode != 0 or not (work / b).exists():
            suite.fail(f"convert {a} -> {b}: {p.stdout} {p.stderr}")
            return
        data = (work / b).read_bytes()
        try:
            if b.lower().endswith(".png"):
                ct, img = png_read(data)
                ok = ct == 6 and np.array_equal(img, want_st)
            else:
                bpp, img = bmp_read(data)
                ok = bpp == 32 and np.array_equal(img[..., [2, 1, 0, 3]], want_st)
        except (ValueError, zlib.error, struct.error) as e:
            ok = False
            p = f"reader: {e}"
        if ok:
            suite.ok()
        else:
            suite.fail(f"convert {a} -> {b}: pixels differ {p if isinstance(p, str) else ''}")
    for args, code, name in (
            (["--convert", runner.rel(work / "io_src.png"), "--to", runner.rel(work / "io_x.tif")], -9, "save .tif"),
            (["--convert", runner.rel(work / "does_not_exist.png"), "--to", runner.rel(work / "io_y.png")], -13,
             "load missing file"),
            (["--convert", runner.rel(work / "io_src.png"), "--to", runner.rel(work / "no_dir" / "x.png")], -13,
             "save into a missing directory")):
        p = runner.run(args)
        got = p.stderr.strip().rsplit(" ", 1)[-1] if p.returncode else "0"
        if p.returncode == 1 and got == str(code):
            suite.ok()
        else:
            suite.fail(f"{name}: exit {p.returncode}, stderr {p.stderr.strip()!r}, want error {code}")
    for f in ("io_src.png", "io_a.bmp", "io_b.PNG", "io_c.Bmp"):
        if (work / f).exists():
            (work / f).unlink()


# ---------------------------------------------------------------- benchmark

def bench_sample(rng):
    """~10 MB: 3 MB text, filtered screenshot rows (4.8 MB), photo-like rows (2 MB)."""
    text = text_data(rng, 3 << 20)
    shot = png_filter_rows(image_data(rng, 1600, 1000))
    r = np.random.default_rng(7)
    photo = (np.cumsum(r.integers(-3, 4, (700, 1000, 3)), axis=1) % 256).astype(np.uint8)
    return text + shot + png_filter_rows(photo)[: 2 << 20]


def run_bench(runner, rng):
    sample = bench_sample(rng)
    raw, zf = runner.work / "bench.bin", runner.work / "bench.z"
    raw.write_bytes(sample)
    zf.write_bytes(zlib.compress(sample, 6))
    lines = []
    for lv in (1, 6, 9):
        p = runner.run(["--bench-deflate", runner.rel(raw), "--level", str(lv), "--reps", "2"])
        m = dict(kv.split("=") for kv in p.stdout.split()[1:]) if p.returncode == 0 else {}
        if m:
            t0 = time.perf_counter()
            zl = len(zlib.compress(sample, lv))
            zt = time.perf_counter() - t0
            lines.append(f"deflate L{lv}: {int(m['in']) / int(m['us']):6.1f} MB/s, {int(m['out']):9d} bytes"
                         f"   (Python zlib L{lv}: {len(sample) / zt / 1e6:6.1f} MB/s, {zl:9d} bytes)")
        else:
            lines.append(f"deflate L{lv}: failed {p.stdout} {p.stderr}")
    p = runner.run(["--bench-inflate", runner.rel(zf), "--reps", "5"])
    if p.returncode == 0:
        m = dict(kv.split("=") for kv in p.stdout.split()[1:])
        zdata = zf.read_bytes()
        t0 = time.perf_counter()
        for _ in range(5):
            zlib.decompress(zdata)
        zt = (time.perf_counter() - t0) / 5
        lines.append(f"inflate   : {int(m['out']) / int(m['us']):6.1f} MB/s (output)"
                     f"            (Python zlib: {len(sample) / zt / 1e6:6.1f} MB/s)")
    else:
        lines.append(f"inflate: failed {p.stdout} {p.stderr}")
    raw.unlink()
    zf.unlink()
    return f"sample {len(sample) / 1e6:.1f} MB", lines


# ---------------------------------------------------------------- main

def build_targets(targets, out):
    for t in targets:
        subprocess.run(testenv.build_cmd("codectest", t, out), cwd=ROOT, check=True)   # Rust; asm for rwref


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--out", default="build", help="build directory (default: build)")
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--only", default="", help="comma list of suites")
    ap.add_argument("--fuzz", type=int, default=3000, help="fuzz cases per fuzz suite")
    ap.add_argument("--no-bench", action="store_true")
    ap.add_argument("--seed", type=int, default=1234)
    a = ap.parse_args()
    out = (ROOT / a.out).resolve()
    targets = ["win", "linux"] if a.target == "both" else [a.target]
    only = set(filter(None, a.only.split(",")))
    want = lambda s: not only or s in only  # noqa: E731
    if not a.no_build:
        build_targets(targets, out)

    all_ok = True
    for target in targets:
        runner = Runner(target, out)
        suites = []
        print(f"==== {target}")
        if want("sums"):
            s = Suite("sums")
            dt = run_suite(runner, s, suite_sums(random.Random(a.seed)))
            suites.append((s, dt))
        if want("inflate"):
            s = Suite("inflate")
            dt = run_suite(runner, s, suite_inflate(random.Random(a.seed + 1)))
            suites.append((s, dt))
        if want("fuzz"):
            s = Suite("fuzz")
            dt = run_suite(runner, s, suite_fuzz(random.Random(a.seed + 2), a.fuzz))
            suites.append((s, dt))
        if want("deflate"):
            s = Suite("deflate")
            recs, info, ds = suite_deflate(random.Random(a.seed + 3), big=True)
            dt = run_suite(runner, s, recs)
            back = [Rec(OP_ZINFLATE, len(ds[n]), z, expect_bytes(ds[n]), f"own-inflate {n} L{lv}")
                    for (n, lv), z in info.items()]
            dt += run_suite(runner, s, back)
            for n, data in ds.items():
                if (n, 6) in info and len(data) >= 1000:
                    ours, ref = len(info[(n, 6)]), len(zlib.compress(data, 6))
                    ratio = ours / ref
                    lv9 = len(info[(n, 9)]) / max(1, len(zlib.compress(data, 9))) if (n, 9) in info else 0
                    lv1 = len(info[(n, 1)]) / max(1, len(zlib.compress(data, 1))) if (n, 1) in info else 0
                    s.notes.append(f"{n:10s} {len(data):8d} B  ours L6 {ours:8d}  zlib L6 {ref:8d}  "
                                   f"ratio {ratio:.3f} (L1 {lv1:.3f}, L9 {lv9:.3f})")
                    if ratio > 1.15:
                        s.fail(f"{n}: L6 size {ours} > 1.15 x zlib {ref}")
            suites.append((s, dt))
        for name, fn in (("png", lambda r: suite_png(r, a.fuzz)), ("pngenc", suite_pngenc),
                         ("bmp", lambda r: suite_bmp(r, a.fuzz)), ("bmpenc", suite_bmpenc)):
            if want(name):
                s = Suite(name)
                dt = run_suite(runner, s, fn(random.Random(a.seed + len(name))))
                suites.append((s, dt))
        if want("imgio"):
            s = Suite("imgio")
            t0 = time.time()
            suite_imgio(runner, random.Random(a.seed + 9), s)
            suites.append((s, time.time() - t0))
        if want("bench") and not a.no_bench:
            s = Suite("bench")
            t0 = time.time()
            head, lines = run_bench(runner, random.Random(5))
            s.notes = [head] + lines
            s.passed = 1
            suites.append((s, time.time() - t0))
        for s, dt in suites:
            status = "PASS" if s.failed == 0 else "FAIL"
            print(f"[{target}] {status} {s.name:8s} {s.passed:6d} passed {s.failed:5d} failed  ({dt:.1f}s)")
            for n in s.notes:
                print("        " + n)
            for m in s.msgs:
                print("        ! " + m)
            all_ok &= s.failed == 0
    print("codec_check:", "ALL PASSED" if all_ok else "FAILURES")
    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())
