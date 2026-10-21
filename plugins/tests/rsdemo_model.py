#!/usr/bin/env python3
"""rsdemo_model.py -- the expected layer hashes of plugins/tests/rsdemo.uis, computed from the
plugin fixture (tools/plugin_check.py) with the documented math of plugins/rsdemo (not taken
from the app).  `python plugins/tests/rsdemo_model.py` prints them."""
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
import plugin_check as pc  # noqa: E402

M32 = 0xFFFFFFFF


def hash32(x, y, seed, stream):
    """fx_hash32 (PA_HASH32)"""
    h = ((x * 0x8DA6B343) ^ (y * 0xD8163841) ^ (seed * 0xCB1AB31F) ^ (stream * 0x165667B1)) & M32
    h ^= h >> 16
    h = (h * 0x85EBCA6B) & M32
    h ^= h >> 13
    h = (h * 0xC2B2AE35) & M32
    return h ^ (h >> 16)


def stretch(px, per_channel, strength):
    vis = [p for p in px if p[3]]
    lo = [min((p[c] for p in vis), default=255) for c in range(3)]
    hi = [max((p[c] for p in vis), default=0) for c in range(3)]
    if not per_channel:
        lo, hi = [min(lo)] * 3, [max(hi)] * 3
    luts = []
    for c in range(3):
        lut = []
        for v in range(256):
            if hi[c] <= lo[c]:
                st = v
            elif v <= lo[c]:
                st = 0
            elif v >= hi[c]:
                st = 255
            else:
                st = ((v - lo[c]) * 255 + (hi[c] - lo[c]) // 2) // (hi[c] - lo[c])
            lut.append((v * (100 - strength) + st * strength + 50) // 100)
        luts.append(lut)
    return [(luts[0][r], luts[1][g], luts[2][b], a) for r, g, b, a in px]


def grain(px, w, n, seed):
    out = []
    for i, (r, g, b, a) in enumerate(px):
        x, y = i % w, i // w
        k = (((hash32(x, y, seed, 0) & 0xFFFF) * (2 * n + 1)) >> 16) - n
        out.append(tuple(max(0, min(255, c + k)) for c in (r, g, b)) + (a,))
    return out


W, H = 48, 32
FIXTURE = Path(__file__).resolve().parent / "rsdemo.png"


def fixture_pixels():
    """48 x 32 straight RGBA with narrow channel ranges (r 40..200, g 60..150, b 90..120), a
    transparent block (whose colours must not count for the ranges) and a flat band"""
    px = []
    for y in range(H):
        for x in range(W):
            if x < 6 and y < 6:
                px.append((255, 0, 3, 0))
            elif y >= 28:
                px.append((120, 100, 100, 255))
            else:
                px.append((40 + (x * 160) // (W - 1), 60 + (y * 90) // (H - 1), 90 + ((x + y) * 30) // (W + H - 2),
                           255 if (x + y) % 5 else 128))
    return px


def expected():
    px = fixture_pixels()
    return {"original": pc.fnv(px),
            "stretch": pc.fnv(stretch(px, False, 100)),
            "stretch_channels_40": pc.fnv(stretch(px, True, 40)),
            "grain_30_4711": pc.fnv(grain(px, W, 30, 4711)),
            "prepcheck_aux": pc.fnv([(255 - r, 255 - g, 255 - b, a) for r, g, b, a in px])}


if __name__ == "__main__":
    if "--fixture" in sys.argv:
        FIXTURE.write_bytes(pc.png_bytes(fixture_pixels(), W, H))
        print("wrote", FIXTURE)
    for k, v in expected().items():
        print(f"{k:20s} {v}")
