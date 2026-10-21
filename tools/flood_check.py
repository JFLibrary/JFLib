#!/usr/bin/env python3
"""Checks of flood_fill_mask (gfx/flood.s) against a numpy / BFS reference, driven through
apps/fxtest (runner in tools/blend_check.py).

  python tools/flood_check.py --target both
  python tools/flood_check.py --target win --no-bench --no-build

Colour test (paint.net CheckColor, a = seed, b = candidate):
  sum = sum_{c in R,G,B} floor((1 + (a.c - b.c)^2) * a.A / 256) + (a.A - b.A)^2
  tol = round(pct * 255 / 100), match when sum <= tol * tol * 4
Contiguous mode = 4-connected region of matching pixels around the seed, FF_GLOBAL = every
matching pixel.  Checks are bit-exact (output 0 / 255), including tile sparsity of the output
mask, masks larger than the image, bad seeds, snakes / mazes (deep stacks), every tolerance
step on noise, and timings (4000 x 3000 full fill must take < 150 ms).
"""

import argparse
import struct
import sys
import time
from collections import deque
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
from blend_check import FxRunner, Rec, Report, ROOT  # noqa: E402
from tmask_check import T_ALLOC, T_STORE, T_COUNT, T_FILL, stored, tile_kinds  # noqa: E402
sys.path.insert(0, str(Path(__file__).resolve().parent.parent.parent / "legacy"))  # R13: build.py is in legacy/
import build  # noqa: E402

T_FLOOD = 50
FF_GLOBAL = 1


def ref_match(img, seed, pct):
    pct = min(max(pct, 0), 100)
    tol = (pct * 255 + 50) // 100
    a = [(int(seed) >> s) & 255 for s in (0, 8, 16, 24)]
    b = img.astype(np.int64)
    s = np.zeros(img.shape, np.int64)
    for k, sh in enumerate((0, 8, 16)):
        d = a[k] - ((b >> sh) & 255)
        s += ((1 + d * d) * a[3]) >> 8
    d = a[3] - ((b >> 24) & 255)
    s += d * d
    return s <= tol * tol * 4


def ref_flood(img, x, y, pct, flags):
    m = ref_match(img, img[y, x], pct)
    if flags & FF_GLOBAL:
        return m
    h, w = m.shape
    out = np.zeros_like(m)
    out[y, x] = True
    q = deque([(x, y)])
    while q:
        cx, cy = q.popleft()
        for nx, ny in ((cx - 1, cy), (cx + 1, cy), (cx, cy - 1), (cx, cy + 1)):
            if 0 <= nx < w and 0 <= ny < h and m[ny, nx] and not out[ny, nx]:
                out[ny, nx] = True
                q.append((nx, ny))
    return out


def regions(rng, w, h, ncol=5, noise=0):
    """Voronoi-like colour regions plus optional per-channel noise."""
    cols = rng.integers(0, 2**32, ncol, dtype=np.uint64).astype(np.int64)
    cols[0] |= 0xFF000000
    cx = rng.integers(0, w, 12)
    cy = rng.integers(0, h, 12)
    lab = rng.integers(0, ncol, 12)
    yy, xx = np.mgrid[0:h, 0:w]
    d = (xx[None] - cx[:, None, None]) ** 2 + (yy[None] - cy[:, None, None]) ** 2
    img = cols[lab[d.argmin(0)]]
    if noise:
        for sh in (0, 8, 16, 24):
            c = (img >> sh) & 255
            c = np.clip(c + rng.integers(-noise, noise + 1, (h, w)), 0, 255)
            img = (img & ~(255 << sh)) | (c << sh)
    return img.astype(np.uint32)


def maze(w, h):
    """A spiral snake of matching pixels (long, narrow region)."""
    img = np.full((h, w), 0xFF000000, np.uint32)
    x0, y0, x1, y1 = 0, 0, w - 1, h - 1
    while x0 <= x1 and y0 <= y1:
        img[y0, x0:x1 + 1] = 0xFFFFFFFF
        img[y0:y1 + 1, x1] = 0xFFFFFFFF
        if y1 > y0:
            img[y1, x0:x1 + 1] = 0xFFFFFFFF
        if x0 + 2 <= x1:
            img[y0 + 2:y1 + 1, x0] = 0xFFFFFFFF
        x0, y0, x1, y1 = x0 + 2, y0 + 2, x1 - 2, y1 - 2
        if x0 <= x1 and y0 - 1 <= y1:
            img[y0 - 1, x0 - 2] = 0xFFFFFFFF if x0 - 2 >= 0 else img[y0 - 1, 0]
    return img


def run_case(run, rep, name, img, x, y, pct, flags, mw=None, mh=None, expect_res=0, pad=0, rng=None):
    h, w = img.shape
    mw, mh = mw or w, mh or h
    data = img
    if pad:                             # row padding filled with junk that must be ignored
        data = np.concatenate([img, rng.integers(0, 2**32, (h, pad), dtype=np.uint64).astype(np.uint32)], 1)
    recs = [Rec(T_ALLOC, [0, mw, mh], [], "alloc"),
            Rec(T_FILL, [0, 0, 0, mw, mh, 99], [], "prefill"),
            Rec(T_FLOOD, [0, w, h, x, y, pct, flags, 1, pad], [(np.ascontiguousarray(data).tobytes(), 0)], name),
            Rec(T_STORE, [0], [], "store"), Rec(T_COUNT, [0], [], "count")]
    res = run.run(recs)
    got = stored(res[3], mw, mh)
    exp = np.zeros((mh, mw), np.uint8)
    if expect_res == 0:
        exp[:h, :w] = ref_flood(img, x, y, pct, flags) * 255
    ok = res[2].res == expect_res and np.array_equal(got, exp)
    rep.check(ok, f"flood {name}", f"res {res[2].res}, {int((got != exp).sum())} px differ")
    rep.check(struct.unpack("<II", res[4].extra) == tile_kinds(exp), f"flood {name} sparsity",
              f"{struct.unpack('<II', res[4].extra)} vs {tile_kinds(exp)}")


def suite_flood(run, rep, rng):
    t0 = time.time()
    for w, h in ((1, 1), (1, 70), (70, 1), (63, 65), (64, 64), (65, 64), (130, 7), (300, 200), (1000, 3)):
        img = regions(rng, w, h)
        for _ in range(3):
            x, y = int(rng.integers(0, w)), int(rng.integers(0, h))
            pct = int(rng.choice([0, 10, 50, 100]))
            run_case(run, rep, f"regions {w}x{h} ({x},{y}) {pct}%", img, x, y, pct, 0)
            run_case(run, rep, f"regions global {w}x{h} ({x},{y}) {pct}%", img, x, y, pct, FF_GLOBAL)
    # every tolerance step on noisy regions
    img = regions(rng, 180, 140, 4, noise=40)
    for pct in range(0, 101, 5):
        run_case(run, rep, f"noise tol {pct}%", img, 90, 70, pct, 0)
    for pct in (0, 3, 33, 77, 100):
        run_case(run, rep, f"noise global tol {pct}%", img, 17, 5, pct, FF_GLOBAL)
    # alpha: transparent seed (colour terms vanish), varying alpha
    img = regions(rng, 150, 90, 6, noise=10)
    img[:, :60] &= 0x00FFFFFF
    run_case(run, rep, "transparent seed", img, 10, 10, 20, 0)
    run_case(run, rep, "alpha edge", img, 100, 40, 30, 0)
    # connectivity: checkerboard (diagonals do not connect), snake, mazes
    yy, xx = np.mgrid[0:100, 0:130]
    chk = np.where((xx + yy) % 2 == 0, 0xFF000000, 0xFFFFFFFF).astype(np.uint32)
    run_case(run, rep, "checkerboard", chk, 50, 50, 0, 0)
    run_case(run, rep, "checkerboard global", chk, 50, 50, 0, FF_GLOBAL)
    run_case(run, rep, "spiral 257x131", maze(257, 131), 0, 0, 0, 0)
    run_case(run, rep, "spiral 64x640", maze(64, 640), 0, 0, 0, 0)
    comb = np.full((200, 200), 0xFF000000, np.uint32)
    comb[:, ::2] = 0xFFFFFFFF
    comb[0, :] = 0xFFFFFFFF
    run_case(run, rep, "comb", comb, 0, 0, 0, 0)
    noise = rng.integers(0, 2, (300, 300)).astype(np.uint32) * 0xFFFFFFFF
    noise[150, 150] = 0xFFFFFFFF
    run_case(run, rep, "binary noise", noise, 150, 150, 0, 0)
    # padded rows (stride > w * 4) with junk in the padding
    for w, h, pad in ((61, 40, 3), (64, 33, 16), (100, 70, 28), (130, 20, 1)):
        img = regions(rng, w, h, 3, noise=5)
        run_case(run, rep, f"padded {w}x{h}+{pad}", img, w // 2, h // 2, 15, 0, pad=pad, rng=rng)
        run_case(run, rep, f"padded global {w}x{h}+{pad}", img, 1, 1, 15, FF_GLOBAL, pad=pad, rng=rng)
    # mask larger than the image, bad seeds
    img = regions(rng, 70, 50)
    run_case(run, rep, "larger mask", img, 5, 5, 20, 0, mw=100, mh=190)
    run_case(run, rep, "seed outside x", img, 70, 5, 20, 0, expect_res=-1)
    run_case(run, rep, "seed outside y", img, 5, -1, 20, 0, expect_res=-1)
    res = run.run([Rec(T_ALLOC, [0, 30, 30], [], "small mask"),
                   Rec(T_FLOOD, [0, 70, 50, 1, 1, 20, 0], [(img.tobytes(), 0)], "mask too small")])
    rep.check(res[1].res == -1, "flood mask too small", str(res[1].res))
    rep.done("flood", t0)


def suite_bench(run, rep, rng):
    t0 = time.time()
    w, h = 4000, 3000
    uni = np.full((h, w), 0xFF336699, np.uint32)
    reg = regions(rng, w, h, 3, noise=6)
    lines = []
    for name, img, x, y, pct, fl in (("uniform contiguous", uni, 2000, 1500, 0, 0),
                                     ("uniform global", uni, 0, 0, 0, FF_GLOBAL),
                                     ("regions+noise contiguous 20%", reg, 2000, 1500, 20, 0),
                                     ("regions+noise global 20%", reg, 2000, 1500, 20, FF_GLOBAL),
                                     ("regions+noise contiguous 3%", reg, 10, 10, 3, 0)):
        res = run.run([Rec(T_ALLOC, [0, w, h], [], "a"),
                       Rec(T_FLOOD, [0, w, h, x, y, pct, fl, 5], [(img.tobytes(), 0)], name),
                       Rec(T_COUNT, [0], [], "count")])
        ms = res[1].time_us() / 5 / 1000
        lines.append(f"    {name:<30} {ms:6.1f} ms  (tiles, full) = {struct.unpack('<II', res[2].extra)}")
        if name == "uniform contiguous":
            rep.check(ms < 150 and struct.unpack("<II", res[2].extra) == (0, 63 * 47),
                      "4000x3000 contiguous full fill < 150 ms", f"{ms:.1f} ms")
    print(f"[{rep.target}] flood bench 4000 x 3000:")
    print("\n".join(lines))
    rep.done("bench", t0)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--out", default="build/g1")
    ap.add_argument("--no-build", action="store_true")
    ap.add_argument("--no-bench", action="store_true")
    ap.add_argument("--seed", type=int, default=7)
    args = ap.parse_args()
    out = (ROOT / args.out).resolve()
    targets = ["win", "linux"] if args.target == "both" else [args.target]
    fails = 0
    for t in targets:
        if not args.no_build:
            build.build_app("fxtest", t, out)
        run = FxRunner(t, out)
        rng = np.random.default_rng(args.seed)
        rep = Report(t)
        suite_flood(run, rep, rng)
        fails += rep.fails
        if not args.no_bench:
            rep = Report(t)
            suite_bench(run, rep, rng)
            fails += rep.fails
    print("flood_check:", "all passed" if fails == 0 else f"{fails} FAILED")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
