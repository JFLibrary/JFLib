#!/usr/bin/env python3
"""jp2_check.py -- lib/img_jp2 (JPEG 2000) against OpenJPEG (through Pillow) and SerenityOS's
test images, through `codectest --jp2`

Suites
  serenity  SerenityOS's JPEG 2000 test images (BSD-2-Clause, fetched once at a pinned commit
            into build/vectors/, not in git): every code-block style (JasPer), Kakadu files
            with CMYK, CIELab, ICC profiles, alpha and 16 bits, OpenJPEG files with ROI, PLT,
            TLM, SOP, EPH, empty packets, a palette, 4 bits, BGRA channel definitions and all
            five progressions.  Against Pillow's decode (OpenJPEG 2.5); where Pillow does
            something else (it ignores channel definitions, scales 4 and 16 bits its own way,
            cannot open two of the files) against the reference PNG.  Lossless exactly, lossy
            within 1.
  made      files written here by Pillow (OpenJPEG) with the options it has: lossless and
            lossy, 1 to 4 components, 8 and 16 bits, image and tile offsets, tiles, 1 to 8
            resolutions, code-blocks from 4 to 64, precincts, the five progressions, quality
            layers, PLT, MCT on and off, JP2 and raw J2K; against Pillow's decode, also at
            reduced resolutions (Pillow's `reduce`): lossless exactly, lossy within 1 at 8 bits
            and within 8 (1 / 8192 of the range) at 16 bits
  mutate    damaged copies (bytes flipped, cut, spans repeated or zeroed; codectest
            --jp2-mutate) of a lossless and a lossy file: no crash, none slower than 2 s
  bench     a 2500 x 3500 RGB page (lossy, 6 resolutions): our decode against Pillow's

A checksum over every decoded sample closes the run; it is the same on every target (run with
--exe build/win/codectest.exe --wine to compare).

  python tools/jp2_check.py [--exe build/linux/codectest] [--wine] [--only SUITE] [--keep DIR]

Exit code 1 when a check fails.
"""
import argparse
import hashlib
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tools"))
from suite_fetch import fetch  # noqa: E402

try:
    from PIL import Image
except ImportError:  # the suites that need Pillow are skipped
    Image = None

SERENITY = ("SerenityOS/serenity", "835d22663febc14427577fc91468c1e4c1fcc914")
SER_DIR = "Tests/LibGfx/test-inputs/jpeg2000/"
# name: how it is checked ("pillow": Pillow's decode; "lossy": Pillow's, within 1; "ref": ref.png;
# "ref16": ref.png against the samples' high bytes; "shift": Pillow's 8 bits against the
# samples shifted up; "alpha": only the last channel against ref.png's alpha)
SER_FILES = {
    "buggie-gray.jpf": "lossy",
    "cmyk-small-icc.jpf": "pillow",
    "cmyk-small.jpf": "pillow",
    "indexed-small.jp2": "pillow",
    "jasper-rgba-u8-cbstyle-01-bypass-finer-layers.jp2": "pillow",
    "jasper-rgba-u8-cbstyle-01-bypass-layers.jp2": "pillow",
    "jasper-rgba-u8-cbstyle-01-bypass.jp2": "pillow",
    "jasper-rgba-u8-cbstyle-02-resetprob.jp2": "pillow",
    "jasper-rgba-u8-cbstyle-04-termall-layers.jp2": "pillow",
    "jasper-rgba-u8-cbstyle-04-termall.jp2": "pillow",
    "jasper-rgba-u8-cbstyle-05-bypass-termall.jp2": "pillow",
    "jasper-rgba-u8-cbstyle-06-resetprob-termall.jp2": "pillow",
    "jasper-rgba-u8-cbstyle-08-vcausal.jp2": "pillow",
    "jasper-rgba-u8-cbstyle-16-pterm.jp2": "pillow",
    "jasper-rgba-u8-cbstyle-32-segsym.jp2": "pillow",
    "jasper-rgba-u8-cbstyle-36-termall-segsym.jp2": "pillow",
    "jasper-rgba-u8-cbstyle-59-all-but-termall.jp2": "pillow",
    "jasper-rgba-u8-cbstyle-63-all.jp2": "pillow",
    "jasper-rgba-u8-solid-alpha-cbstyle-04-termall.jp2": "pillow",
    "jasper-tile4x2-res5.jp2": "pillow",
    "kakadu-lossless-cmyk-u8-prog1-layers1-res6.jp2": "pillow",
    "kakadu-lossless-cmyka-u8-prog1-layers1-res6.jp2": "alpha",
    "kakadu-lossless-gray-alpha-u8-prog1-layers1-res6.jp2": "pillow",
    "kakadu-lossless-gray-u8-prog1-layers1-res6-icc.jp2": "pillow",
    "kakadu-lossless-gray-u8-prog1-layers1-res6.jp2": "pillow",
    "kakadu-lossless-lab-alpha-u8-prog1-layers1-res6.jp2": "pillow",
    "kakadu-lossless-lab-u8-prog1-layers1-res6.jp2": "pillow",
    "kakadu-lossless-rgb-u8-prog1-layers1-res6-mct.jp2": "pillow",
    "kakadu-lossless-rgba-u16-prog1-layers1-res6.jp2": "ref16",
    "kakadu-lossless-rgba-u8-prog1-layers1-res6-mct.jp2": "pillow",
    "kakadu-lossy-rgba-u8-prog0-layers1-res6-mct.jp2": "lossy",
    "openjpeg-lossless-RGN.jp2": "pillow",
    "openjpeg-lossless-bgra-u8.jp2": "ref",
    "openjpeg-lossless-indexed-u8-rgb-u8.jp2": "pillow",
    "openjpeg-lossless-rgba-u4.jp2": "shift",
    "openjpeg-lossless-rgba-u8-PLT.jp2": "pillow",
    "openjpeg-lossless-rgba-u8-TLM.jp2": "pillow",
    "openjpeg-lossless-rgba-u8-prog0-EPH-SOP.jp2": "pillow",
    "openjpeg-lossless-rgba-u8-prog0-EPH-empty-packets.jp2": "pillow",
    "openjpeg-lossless-rgba-u8-prog0-EPH.jp2": "pillow",
    "openjpeg-lossless-rgba-u8-prog0-SOP.jp2": "pillow",
    "openjpeg-lossless-rgba-u8-prog0-tile-part-index-overflow.jp2": "ref",
    "openjpeg-lossless-rgba-u8-prog0-tile4x2-cblk4x16-tp3-layers3-res2.jp2": "pillow",
    "openjpeg-lossless-rgba-u8-prog1-tile4x2-cblk4x16-tp3-layers3-res2.jp2": "pillow",
    "openjpeg-lossless-rgba-u8-prog2-tile4x2-cblk4x16-tp3-layers3-res2.jp2": "pillow",
    "openjpeg-lossless-rgba-u8-prog3-tile4x2-cblk4x16-tp3-layers3-res2.jp2": "pillow",
    "openjpeg-lossless-rgba-u8-prog4-tile4x2-cblk4x16-tp3-layers3-res2.jp2": "pillow",
    "openjpeg-lossy-quantization-scalar-derived.jp2": "lossy",
}
SER_REFS = ["ref.png"]

FAILS = []
DIGEST = hashlib.sha256()


def fail(msg):
    FAILS.append(msg)
    print("FAIL " + msg)


class Exe:
    def __init__(self, exe, wine):
        self.exe = str(exe)
        self.wine = wine

    def cmd(self, *args):
        c = [self.exe, *map(str, args)]
        return ["wine", *c] if self.wine else c

    def decode(self, path, work, reduce=0):
        """our decode: (header, [component dicts]) or (None, error)"""
        out = Path(work) / "out.j2d"
        env = dict(os.environ, WINEDEBUG="-all")
        r = subprocess.run(self.cmd("--jp2", path, "--to", out, "--reduce", reduce), capture_output=True, text=True, timeout=120, env=env)
        if r.returncode != 0:
            return None, (r.stderr.strip() or "exit code %d" % r.returncode)
        b = out.read_bytes()
        if b[:4] != b"J2D1":
            return None, "bad output"
        DIGEST.update(b)
        w, h, n, col, alpha, pre = struct.unpack_from("<6I", b, 4)
        p = 28
        comps = []
        for _ in range(n):
            cw, ch, dx, dy, prec, sg = struct.unpack_from("<6I", b, p)
            p += 24
            comps.append({"w": cw, "h": ch, "dx": dx, "dy": dy, "prec": prec, "signed": sg, "data": struct.unpack_from("<%di" % (cw * ch), b, p)})
            p += 4 * cw * ch
        return {"w": w, "h": h, "color": col, "alpha": alpha, "pre": pre}, comps


def pillow_bands(path, reduce=0):
    im = Image.open(path)
    if reduce:
        im.reduce = reduce
    im.load()
    if im.mode == "P":
        im = im.convert("RGBA" if "A" in im.palette.mode else "RGB")
    if im.mode.startswith("I;16"):
        return im.size, [list(im.get_flattened_data())], 16
    return im.size, [list(b.get_flattened_data()) for b in im.split()], 8


def compare(name, comps, size, bands, tol, conv=lambda c, v: v):
    if len(comps) < len(bands):
        fail("%s: %d components, Pillow %d" % (name, len(comps), len(bands)))
        return
    worst = 0
    for k, ref in enumerate(bands):
        c = comps[k]
        if (c["w"], c["h"]) != tuple(size):
            fail("%s: component %d is %dx%d, Pillow %dx%d" % (name, k, c["w"], c["h"], size[0], size[1]))
            return
        for a, b in zip(c["data"], ref):
            d = abs(conv(c, a) - b)
            if d > worst:
                worst = d
    if worst > tol:
        fail("%s: samples differ by up to %d (allowed %d)" % (name, worst, tol))
    return worst


def suite_serenity(exe, work):
    folder = fetch(*SERENITY, [SER_DIR + n for n in list(SER_FILES) + SER_REFS] + ["LICENSE"])
    d = folder / SER_DIR
    ref = Image.open(d / "ref.png").convert("RGBA") if Image else None
    ref_bands = [list(b.get_flattened_data()) for b in ref.split()] if ref else None
    n = 0
    for name, how in SER_FILES.items():
        hdr, comps = exe.decode(d / name, work)
        if hdr is None:
            fail("serenity %s: %s" % (name, comps))
            continue
        n += 1
        if Image is None:
            continue
        if how in ("pillow", "lossy"):
            size, bands, _ = pillow_bands(d / name)
            compare("serenity " + name, comps, size, bands, 1 if how == "lossy" else 0)
        elif how == "ref":
            compare("serenity " + name, comps, ref.size, ref_bands, 0)
        elif how == "ref16":
            compare("serenity " + name, comps, ref.size, ref_bands, 0, lambda c, v: v >> 8)
        elif how == "shift":
            size, bands, _ = pillow_bands(d / name)
            compare("serenity " + name, comps, size, bands, 0, lambda c, v: v << (8 - c["prec"]))
        elif how == "alpha":
            a = comps[hdr["alpha"] - 1] if hdr["alpha"] else None
            if a is None or list(a["data"]) != ref_bands[3]:
                fail("serenity %s: the opacity channel is not ref.png's alpha" % name)
    print("serenity: %d of %d files decoded and compared" % (n, len(SER_FILES)))


def pattern(w, h, mode, seed):
    """a test image: gradients, edges and noise"""
    rnd = random.Random(seed)
    im = Image.new(mode, (w, h))
    px = im.load()
    nch = len(mode) if mode in ("L", "LA", "RGB", "RGBA") else 1
    top = 65535 if mode == "I;16" else 255
    for y in range(h):
        for x in range(w):
            vals = []
            for c in range(nch):
                v = (x * (37 + 11 * c) + y * (23 + 7 * c)) % (top + 1)
                if (x // 7 + y // 5 + c) % 3 == 0:
                    v = top - v
                v = (v + rnd.randrange(0, max(top // 16, 2))) % (top + 1)
                vals.append(v)
            px[x, y] = vals[0] if nch == 1 else tuple(vals)
    return im


def made_cases():
    """(name, mode, size, save options) -- systematic cases, then random combinations"""
    cases = []
    for mode in ("L", "LA", "RGB", "RGBA", "I;16"):
        for irr in (False, True):
            cases.append(("%s-%s" % (mode, "97" if irr else "53"), mode, (97, 61), {"irreversible": irr}))
    for prog in ("LRCP", "RLCP", "RPCL", "PCRL", "CPRL"):
        cases.append(("prog-" + prog, "RGB", (83, 77), {"progression": prog, "tile_size": (32, 48), "num_resolutions": 4, "precinct_size": (32, 32), "codeblock_size": (8, 8)}))
    cases.append(("tiny-1x1", "L", (1, 1), {"num_resolutions": 1}))
    cases.append(("tiny-2x3", "RGB", (2, 3), {"num_resolutions": 2}))
    cases.append(("tiny-3x1-97", "L", (3, 1), {"num_resolutions": 2, "irreversible": True}))
    cases.append(("offset", "RGB", (71, 53), {"offset": (13, 7), "tile_offset": (5, 3), "tile_size": (24, 20)}))
    cases.append(("cblk-4x4", "L", (64, 64), {"codeblock_size": (4, 4)}))
    cases.append(("cblk-64x16", "RGBA", (100, 90), {"codeblock_size": (64, 16)}))
    cases.append(("layers", "RGB", (120, 90), {"irreversible": True, "quality_layers": [60, 30, 10], "quality_mode": "rates"}))
    cases.append(("layers-53", "RGB", (120, 90), {"quality_layers": [40, 10, 0], "quality_mode": "rates"}))
    cases.append(("plt", "RGB", (90, 70), {"plt": True}))
    cases.append(("no-mct", "RGB", (90, 70), {"mct": 0}))
    cases.append(("mct", "RGB", (90, 70), {"mct": 1, "irreversible": True}))
    cases.append(("res8", "L", (300, 260), {"num_resolutions": 8}))
    rnd = random.Random(19)
    for i in range(60):
        mode = rnd.choice(["L", "LA", "RGB", "RGBA", "I;16"])
        w, h = rnd.randrange(1, 160), rnd.randrange(1, 140)
        opt = {"irreversible": rnd.random() < 0.5, "progression": rnd.choice(["LRCP", "RLCP", "RPCL", "PCRL", "CPRL"])}
        nres = rnd.randrange(1, 7)
        while nres > 1 and (w >> (nres - 1)) < 1 or (h >> (nres - 1)) < 1:
            nres -= 1
        opt["num_resolutions"] = max(nres, 1)
        cb = rnd.choice([(4, 4), (8, 16), (16, 8), (32, 32), (64, 64), (16, 64)])
        opt["codeblock_size"] = cb
        if rnd.random() < 0.5:
            pp = rnd.choice([64, 128, 256])
            opt["precinct_size"] = (pp, pp)
        if opt["irreversible"]:
            # OpenJPEG's 9/7 encoder asserts on a signal of one sample: no tiles (their last
            # column or row can be one sample wide) and at least two samples at the lowest level
            while opt["num_resolutions"] > 1 and min(w, h) >> (opt["num_resolutions"] - 1) < 2:
                opt["num_resolutions"] -= 1
            if min(w, h) < 2:
                opt["irreversible"] = False
        else:
            if rnd.random() < 0.4:
                opt["tile_size"] = (rnd.randrange(16, 80), rnd.randrange(16, 80))
            if rnd.random() < 0.3:
                opt["offset"] = (rnd.randrange(0, 20), rnd.randrange(0, 20))
        if rnd.random() < 0.3:
            opt["quality_layers"] = sorted([rnd.randrange(5, 80) for _ in range(rnd.randrange(1, 4))], reverse=True)
            opt["quality_mode"] = "rates"
        if mode in ("RGB", "RGBA"):
            opt["mct"] = rnd.choice([0, 1])
        cases.append(("rand%02d" % i, mode, (w, h), opt))
    return cases


def pillow_job(args):
    """(in a child process: OpenJPEG crashes on some options) write case `args` with Pillow and
    its decodes at each reduction, as raw files next to it"""
    name, mode, size, opt, path, reduces = args
    im = pattern(size[0], size[1], mode, sum(map(ord, name)))
    opt = {k: tuple(v) if isinstance(v, list) and k != "quality_layers" else v for k, v in opt.items()}
    im.save(path, **opt)
    for r in reduces:
        try:
            psize, bands, bits = pillow_bands(path, r)
        except OSError:
            continue
        with open("%s.ref%d" % (path, r), "wb") as f:
            f.write(struct.pack("<4I", psize[0], psize[1], bits, len(bands)))
            for b in bands:
                f.write(struct.pack("<%d%s" % (len(b), "H" if bits == 16 else "B"), *b))


def read_ref(p):
    b = Path(p).read_bytes()
    w, h, bits, n = struct.unpack_from("<4I", b, 0)
    o, sz = 16, w * h
    bands = []
    for _ in range(n):
        if bits == 16:
            bands.append(list(struct.unpack_from("<%dH" % sz, b, o)))
            o += 2 * sz
        else:
            bands.append(list(b[o:o + sz]))
            o += sz
    return (w, h), bands, bits


def suite_made(exe, work):
    if Image is None:
        print("made: skipped (no Pillow)")
        return
    import json
    n = 0
    worst_lossy = 0
    for name, mode, size, opt in made_cases():
        for kind in ("jp2", "j2k"):
            path = Path(work) / ("%s.%s" % (name, kind))
            nres = opt.get("num_resolutions", 6)
            reduces = [0, 1] if nres > 1 and min(size) > 2 else [0]
            job = json.dumps([name, mode, list(size), opt, str(path), reduces])
            r = subprocess.run([sys.executable, "-I", __file__, "--pillow-job", job], capture_output=True, text=True, timeout=300)
            if r.returncode != 0 or not path.exists():
                why = r.stderr.strip().splitlines()[-1][:70] if r.stderr.strip() else "exit code %d" % r.returncode
                print("made %s.%s: OpenJPEG did not write it (%s)" % (name, kind, why))
                continue
            lossy = opt.get("irreversible") or "quality_layers" in opt and any(q for q in opt["quality_layers"])
            for reduce in reduces:
                hdr, comps = exe.decode(path, work, reduce)
                if hdr is None:
                    fail("made %s.%s reduce %d: %s" % (name, kind, reduce, comps))
                    continue
                refp = "%s.ref%d" % (path, reduce)
                if not os.path.exists(refp):
                    print("made %s.%s reduce %d: Pillow cannot read it; ours %dx%d" % (name, kind, reduce, hdr["w"], hdr["h"]))
                    continue
                psize, bands, bits = read_ref(refp)
                # lossy: within 1 at 8 bits, within 8 (1 / 8192 of the range) at 16 bits
                tol = (8 if bits == 16 else 1) if lossy else 0
                w = compare("made %s.%s reduce %d" % (name, kind, reduce), comps, psize, bands, tol)
                if lossy and w is not None:
                    worst_lossy = max(worst_lossy, w)
                n += 1
    print("made: %d decodes compared (lossy ones within %d)" % (n, worst_lossy))


def suite_codec(exe, work):
    """img_all's registration: codectest --convert-ex finds the format by its probe and decodes it
    through img_jp2::CODEC (8-bit ARGB) into a PNG"""
    if Image is None:
        print("codec: skipped (no Pillow)")
        return
    folder = fetch(*SERENITY, [SER_DIR + n for n in SER_FILES])
    d = folder / SER_DIR
    names = [("openjpeg-lossless-rgba-u8-PLT.jp2", "RGBA"), ("kakadu-lossless-gray-u8-prog1-layers1-res6.jp2", "L"), ("kakadu-lossless-rgb-u8-prog1-layers1-res6-mct.jp2", "RGB"), ("cmyk-small.jpf", "RGB")]
    for name, mode in names:
        out = Path(work) / "codec.png"
        r = subprocess.run(exe.cmd("--convert-ex", d / name, "--to", out, "--straight"), capture_output=True, text=True, timeout=120, env=dict(os.environ, WINEDEBUG="-all"))
        if r.returncode != 0 or not out.exists():
            fail("codec %s: --convert-ex exit %d" % (name, r.returncode))
            continue
        ours = Image.open(out).convert(mode)
        if mode == "RGB" and name.startswith("cmyk"):
            # CMYK as (255 - c)(255 - k) / 255 ...: only the size and that it decoded
            if ours.size != (4, 2):
                fail("codec %s: size %s" % (name, ours.size))
            continue
        ref = Image.open(d / name)
        ref.load()
        ref = ref.convert(mode)
        diff = sum(1 for x, y in zip(ours.get_flattened_data(), ref.get_flattened_data()) if x != y)
        if diff:
            fail("codec %s: %d pixels differ from Pillow" % (name, diff))
    print("codec: %d files through img_all" % len(names))


def suite_mutate(exe, work):
    if Image is None:
        print("mutate: skipped (no Pillow)")
        return
    for name, opt in (("lossless", {"tile_size": (32, 32), "num_resolutions": 3, "precinct_size": (32, 32), "codeblock_size": (8, 8)}), ("lossy", {"irreversible": True, "quality_layers": [30, 10], "quality_mode": "rates"})):
        path = Path(work) / ("mutate-%s.jp2" % name)
        pattern(64, 48, "RGB", 7).save(path, **opt)
        r = subprocess.run(exe.cmd("--jp2-mutate", path, "--count", 3000, "--seed", 11), capture_output=True, text=True, timeout=900, env=dict(os.environ, WINEDEBUG="-all"))
        line = r.stdout.strip().splitlines()[-1] if r.stdout.strip() else ""
        if r.returncode != 0 or "slowest" not in line:
            fail("mutate %s: exit %d %s" % (name, r.returncode, (r.stderr or "")[-200:]))
            continue
        slow = int(line.split("slowest ")[1].split(" ")[0])
        print("mutate %s: %s" % (name, line.split(": ", 1)[1]))
        if slow > 2000:
            fail("mutate %s: a damaged copy took %d ms" % (name, slow))


def suite_bench(exe, work):
    if Image is None:
        print("bench: skipped (no Pillow)")
        return
    path = Path(work) / "page.jp2"
    im = pattern(500, 700, "RGB", 3).resize((2500, 3500))
    im.save(path, irreversible=True, quality_layers=[20], quality_mode="rates", num_resolutions=6)
    r = subprocess.run(exe.cmd("--jp2-bench", path, "--reps", 3), capture_output=True, text=True, timeout=900, env=dict(os.environ, WINEDEBUG="-all"))
    ours = r.stdout.strip()
    best = None
    for _ in range(3):
        t = time.perf_counter()
        Image.open(path).load()
        dt = time.perf_counter() - t
        best = dt if best is None else min(best, dt)
    print("bench: %s; Pillow (OpenJPEG) %.1f ms; %d bytes" % (ours.split(": ", 1)[-1], best * 1000, path.stat().st_size))


def print_fixtures():
    """the JPEG 2000 images of pdftest's render_image (rdraw.rs), as Rust constants"""
    import io
    top, bottom = (40, 160, 200), (200, 40, 90)

    def halves(mode, w=8, h=8):
        im = Image.new(mode, (w, h))
        px = im.load()
        for y in range(h):
            for x in range(w):
                up = y < h // 2
                if mode == "L":
                    px[x, y] = 60 if up else 220
                elif mode == "RGBA":
                    px[x, y] = (top if up else bottom) + ((255 if up else 0),)
                else:
                    px[x, y] = top if up else bottom
        return im

    def enc(im, **opt):
        b = io.BytesIO()
        im.save(b, format="JPEG2000", **opt)
        return b.getvalue()

    out = {
        "J2K_RGB": enc(halves("RGB"), no_jp2=True, num_resolutions=2),
        "JP2_RGB": enc(halves("RGB"), num_resolutions=2),
        "JP2_GRAY": enc(halves("L"), num_resolutions=2),
        "JP2_RGBA": enc(halves("RGBA"), num_resolutions=2),
        "JP2_LOSSY": enc(halves("RGB", 16, 16), num_resolutions=3, irreversible=True, quality_layers=[8], quality_mode="rates"),
    }
    for k, v in out.items():
        print("const %s: &[u8] = &[%s];  // %d bytes" % (k, ", ".join("0x%02X" % b for b in v), len(v)))


def main():
    if len(sys.argv) == 3 and sys.argv[1] == "--pillow-job":
        import json
        pillow_job(json.loads(sys.argv[2]))
        return 0
    ap = argparse.ArgumentParser()
    ap.add_argument("--exe", default=str(ROOT / "build" / "linux" / "codectest"))
    ap.add_argument("--wine", action="store_true")
    ap.add_argument("--only")
    ap.add_argument("--keep")
    ap.add_argument("--fixtures", action="store_true", help="print the JPEG 2000 images of apps/pdftest/src/rdraw.rs")
    a = ap.parse_args()
    if a.fixtures:
        print_fixtures()
        return 0
    exe = Exe(a.exe, a.wine)
    work = a.keep or tempfile.mkdtemp(prefix="jp2_check_")
    os.makedirs(work, exist_ok=True)
    suites = {"serenity": suite_serenity, "made": suite_made, "codec": suite_codec, "mutate": suite_mutate, "bench": suite_bench}
    for name, f in suites.items():
        if a.only and a.only != name:
            continue
        f(exe, work)
    if not a.keep:
        shutil.rmtree(work, ignore_errors=True)
    print("checksum %s" % DIGEST.hexdigest()[:16])
    print("%d failures" % len(FAILS))
    return 1 if FAILS else 0


if __name__ == "__main__":
    sys.exit(main())
