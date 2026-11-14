#!/usr/bin/env python3
"""jxl_epf_fixtures.py -- the fixtures of codectest --jxlref (apps/codectest/jxlref.rs): JPEG XL
files whose EPF runs on chroma subsampled planes, and libjxl 0.7's pixels of them.

    python tools/jxl_epf_fixtures.py

For each W x H and subsampling (420, 422, 440) a JPEG (tools/jpeg_check.py's writer, seeded) is
transcoded by libjxl 0.7 in WSL (as in tools/jxl_plus_check.py).  libjxl writes JPEG frames
without EPF, so the frame header is rewritten here: epf_iters 1 / 2 / 3, a custom sharpness LUT
of 64.0 (JPEG transcodes carry sharpness 0, whose default LUT entry 0 keeps the filter off), the
other EPF parameters at their defaults; the TOC follows byte aligned, the groups unchanged.  The
files are bare codestreams; libjxl decodes each to RGB8.  Prints how far libjxl's EPF output lies
from the file without EPF (the filter must change the picture for the test to mean anything).
Writes apps/codectest/fixtures/jxl/NAME.jxl and NAME.rgb for the names jxlref.rs reads.
"""
import random
import sys
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import jxl_plus_check as X  # noqa: E402
import jxl_ref as R  # noqa: E402

PICK = ["e40x24_420_epf1", "e40x24_422_epf3", "e40x24_440_epf2", "e101x37_420_epf2"]
SIZES = [(40, 24), (101, 37)]


def bits_of(data):
    return [(data[i >> 3] >> (i & 7)) & 1 for i in range(len(data) * 8)]


def bytes_of(bits):
    assert len(bits) % 8 == 0
    out = bytearray(len(bits) // 8)
    for i, v in enumerate(bits):
        out[i >> 3] |= v << (i & 7)
    return bytes(out)


def set_epf(cs, k):
    """the codestream cs (one VarDCT frame, loop filter gab 0 / epf 0) with epf_iters = k, the
    sharpness LUT 64.0 and default EPF parameters otherwise"""
    at = {}
    orig = R.LoopFilter.read

    def spy(self, b, modular):
        at["lf"] = b.p
        return orig(self, b, modular)

    R.LoopFilter.read = spy
    try:
        b, size, meta, _, _ = R.read_headers(cs)
        fh = R.FrameHeader().read(b, meta, size, False)
    finally:
        R.LoopFilter.read = orig
    assert not fh.modular and not fh.lf.gab and fh.lf.epf_iters == 0 and fh.is_last
    hdr_end = b.p
    n = R.toc_entries(fh)
    assert b.f(1) == 0, "TOC permutation"
    b.byte_align()
    toc_start = b.p
    src = bits_of(cs)
    p = at["lf"]  # all_default 0, gab 0, epf_iters 00
    assert src[p:p + 4] == [0, 0, 0, 0]
    lut = [(0x5400 >> i) & 1 for i in range(16)]  # f16 64.0
    out = src[:p + 2] + [k & 1, (k >> 1) & 1] + [1] + lut * 8 + [0, 0] + src[p + 4:hdr_end] + [0]
    out += [0] * (-len(out) % 8)
    out += src[toc_start:]
    new = bytes_of(out)
    b2, size2, meta2, _, _ = R.read_headers(new)
    fh2 = R.FrameHeader().read(b2, meta2, size2, False)
    assert fh2.lf.epf_iters == k and R.toc_entries(fh2) == n and fh2.lf.epf_sharp_lut == [64.0] * 8
    return new


def main():
    import jpeg_check as J
    work = ROOT / "build" / "jxlepf"
    work.mkdir(parents=True, exist_ok=True)
    dst = ROOT / "apps" / "codectest" / "fixtures" / "jxl"
    dst.mkdir(parents=True, exist_ok=True)
    lj = X.Lj(work)
    rng = random.Random(11)
    jobs, plain = [], []
    for (w, h) in SIZES:
        for samp in ("420", "422", "440"):
            spec, coefs, qtabs, qsel = J.make_case(rng, w, h, J.SAMPLINGS[samp], "ycc", quality=80)
            jp = work / f"e{w}x{h}_{samp}.jpg"
            jp.write_bytes(bytes(J.write_jpeg(spec, coefs, qtabs, qsel)))
            out = work / f"e{w}x{h}_{samp}_t.jxl"
            jobs.append({"op": "jpeg", "in": X.rel(jp), "out": X.rel(out), "meta": 0})
            plain.append(out)
    for f, r in zip(plain, lj.run(jobs)):
        assert r.get("ok"), (f.name, r)
    files = []
    for f in plain:
        cs = R.codestream(f.read_bytes())
        for k in (1, 2, 3):
            g = work / f.name.replace("_t.jxl", f"_epf{k}.jxl")
            g.write_bytes(set_epf(cs, k))
            files.append(g)
    ref = {f.stem: d[0] for f, d in zip(plain, lj.decode(plain)) if d is not None}
    bad = 0
    for f, d in zip(files, lj.decode(files)):
        if f.stem not in PICK:
            continue
        if d is None:
            print(f.name, "libjxl decode failed")
            bad += 1
            continue
        px = d[0]
        assert (px[..., 3] == 255).all()
        (dst / f.name).write_bytes(f.read_bytes())
        (dst / (f.stem + ".rgb")).write_bytes(px[..., :3].tobytes())
        base = ref[f.stem.rsplit("_", 1)[0] + "_t"]
        diff = np.abs(px[..., :3].astype(int) - base[..., :3].astype(int))
        print(f"{f.stem}: {f.stat().st_size} bytes; EPF against none: max {diff.max()}, mean {diff.mean():.2f}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
