#!/usr/bin/env python3
"""JPEG XL decoder checks (ASMLIB jxl*.s against the model tools/jxl_ref.py) through apps/codectest.

    python tools/jxl_check.py [--target win|linux|both] [--only info,...] [--no-build]

Suites:
  info    image header, every frame header and TOC (op 180) == model, for the test files in
          build/jxl (*.jxl and suite/*.jxl; build/jxl/suite.py generates the suite files with ffmpeg)
  mod     every modular frame: the decoded modular image after the inverse transforms (op 181)
          == model, bit-exact
  vdc     every VarDCT frame after the DC groups (op 184): DC planes (after adaptive smoothing),
          strategies, quant field, sharpness, EPF sigma, DC buckets, CfL tiles == model, bit-exact
  vpx     every VarDCT frame: the reconstructed X, Y, B planes after dequantization and the inverse
          transforms, before rendering (op 185) == model, bit-exact
  rnd     every frame through the render pipeline (op 186): colour planes and extra channels after
          the last stage (before blending) == model, bit-exact
  enc     jxl_encode (op 187): lossless and lossy (quality 90 / 70 / 40 / 15) == tools/jxl_enc_ref.py
          byte for byte, for fixtures and synthetic images (gray, alpha, odd sizes, several groups);
          the model decodes the lossless ones exactly
  dec     jxl_decode (op 183) of every displayed frame == the model's 8-bit output (to_rgba8), exact;
          ERR_LIMIT past the last frame; files the decoder does not support yet are listed as notes
"""

import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import imgtest_common as it  # noqa: E402
import jxl_ref as J  # noqa: E402

OP_JXINFO = 180
OP_JXMOD = 181
OP_JXDEC = 183
OP_JXVDC = 184
OP_JXVPX = 185
OP_JXRND = 186
OP_JXENC = 187
JXI_HEAD, JXI_FRAME = 15, 28
FRAME_NAMES = ("type", "modular", "flags", "ct", "ups", "passes", "w", "h", "xb", "yb", "ng", "ndg", "ntoc",
               "base", "end", "is_last", "duration", "saveref", "savebct", "gab", "epf", "x0", "y0", "custom",
               "bmode", "bsrc", "gdim", "tocsum")
HEAD_NAMES = ("w", "h", "bits", "float", "nec", "xyb", "cs", "tf", "orient", "haveprev", "pw", "ph", "haveanim",
              "icclen", "frames")
CT = {"none": 0, "ycbcr": 1, "xyb": 2}


def test_files():
    d = ROOT / "build" / "jxl"
    return sorted(d.glob("*.jxl")) + sorted((d / "suite").glob("*.jxl"))


def model_info(data):
    """expected op 180 record from the model"""
    cs = J.codestream(data)
    b, size, meta, td, icc = J.read_headers(cs)
    head = [size.xsize, size.ysize, meta.bit_depth.bits, int(meta.bit_depth.float), len(meta.extra),
            int(meta.xyb_encoded), meta.color.color_space, meta.color.tf, meta.orientation, int(meta.have_preview),
            meta.preview.xsize if meta.have_preview else 0, meta.preview.ysize if meta.have_preview else 0,
            int(meta.have_animation), len(icc) if icc else 0]
    frames = []
    preview = meta.have_preview
    while True:
        fh = J.FrameHeader().read(b, meta, size, preview)
        n = J.toc_entries(fh)
        sizes, offs = J.read_toc(b, n)
        base = b.p >> 3
        end = base + sum(sizes)
        frames.append([fh.frame_type, int(fh.modular), fh.flags & 0xFFFFFFFF, CT[fh.color_transform],
                       fh.upsampling, fh.num_passes, fh.width, fh.height, fh.xsize_blocks, fh.ysize_blocks,
                       fh.num_groups, fh.num_dc_groups, n, base, end, int(fh.is_last), fh.duration,
                       fh.save_as_reference, int(fh.save_before_ct), int(fh.lf.gab), fh.lf.epf_iters,
                       fh.x0 & 0xFFFFFFFF, fh.y0 & 0xFFFFFFFF, int(fh.custom_size), fh.blending.mode,
                       fh.blending.source, fh.group_dim, sum(sizes) & 0xFFFFFFFF])
        if not preview and fh.is_last:
            break
        preview = False
        if end >= len(cs):
            break
        b = J.Bits(cs, end)
    return head + [len(frames)], frames


def check_info(data):
    try:
        exp_head, exp_frames = model_info(data)
    except J.JXLError as e:
        exp_head = None

    def chk(res, out):
        if exp_head is None:
            return None if res != 0 else "model rejects the file, decoder accepted it"
        if res != 0:
            return f"error {it.err_name(res)}"
        if len(out) < JXI_HEAD * 4:
            return f"short record {len(out)}"
        head = list(struct.unpack_from("<%dI" % JXI_HEAD, out))
        for i, (a, e) in enumerate(zip(head, exp_head)):
            if a != e:
                return f"header {HEAD_NAMES[i]}: {a} != {e}"
        for k, ef in enumerate(exp_frames):
            got = struct.unpack_from("<%dI" % JXI_FRAME, out, (JXI_HEAD + k * JXI_FRAME) * 4)
            for i, (a, e) in enumerate(zip(got, ef)):
                if a != e:
                    return f"frame {k} {FRAME_NAMES[i]}: {a} != {e}"
        return None
    return chk


def model_frames(data):
    """decoded Frame objects of every non-preview frame (with references between them)"""
    cs = J.codestream(data)
    b, size, meta, td, icc = J.read_headers(cs)
    counters = [0, 0]
    if meta.have_preview:
        pf = J.Frame(cs, b, size, meta, td, True, counters=counters)
        b = J.Bits(cs, pf.end)
    refs = [None] * 4
    frames = []
    while True:
        fr = J.Frame(cs, b, size, meta, td, refs=refs, counters=counters)
        fr.decode()
        if fr.ref is not None:
            refs[fr.fh.save_as_reference] = fr.ref
        frames.append(fr)
        if fr.fh.is_last or fr.end >= len(cs):
            break
        b = J.Bits(cs, fr.end)
    return frames


def mod_record(full):
    parts = [struct.pack("<I", len(full.ch))]
    for c in full.ch:
        parts.append(struct.pack("<IIii", c.w, c.h, c.hshift, c.vshift))
    for c in full.ch:
        for row in c.d:
            parts.append(struct.pack("<%di" % c.w, *[J.wrap32(v) for v in row]))
    return b"".join(parts)


def check_mod(exp):
    def chk(res, out):
        if res != 0:
            return f"error {it.err_name(res)}"
        if out == exp:
            return None
        n = struct.unpack_from("<I", out)[0] if len(out) >= 4 else -1
        ne = struct.unpack_from("<I", exp)[0]
        if n != ne:
            return f"{n} channels, expected {ne}"
        hd = out[4:4 + 16 * n]
        he = exp[4:4 + 16 * n]
        for k in range(n):
            if hd[16 * k:16 * k + 16] != he[16 * k:16 * k + 16]:
                return f"channel {k} header {struct.unpack_from('<IIii', hd, 16 * k)} != " \
                       f"{struct.unpack_from('<IIii', he, 16 * k)}"
        p = 4 + 16 * n
        for k in range(n):
            w, h = struct.unpack_from("<II", he, 16 * k)
            a, e = out[p:p + 4 * w * h], exp[p:p + 4 * w * h]
            if a != e:
                i = next(j for j in range(0, len(e), 4) if a[j:j + 4] != e[j:j + 4]) // 4
                return f"channel {k} ({w}x{h}) differs at x {i % w} y {i // w}: " \
                       f"{struct.unpack_from('<i', a, 4 * i)[0]} != {struct.unpack_from('<i', e, 4 * i)[0]}"
            p += 4 * w * h
        return "record differs"
    return chk


def suite_mod(runners, suites):
    recs = []
    for f in test_files():
        data = f.read_bytes()
        try:
            frames = model_frames(data)
        except J.JXLError:
            continue
        for k, fr in enumerate(frames):
            if not fr.fh.modular:
                continue
            recs.append(it.Rec(OP_JXMOD, k, data, check_mod(mod_record(fr.full)), f"{f.name}#{k}"))
    for t, R in runners.items():
        s = it.Suite("mod", t)
        it.run_suite(R, s, recs)
        suites.append(s)


def vdc_record(fr):
    import numpy as np
    fh = fr.fh
    xb, yb = fh.xsize_blocks, fh.ysize_blocks
    parts = [struct.pack("<II", xb, yb)]
    for c in range(3):
        parts.append(np.asarray(fr.dc[c], np.float32).tobytes())
    acs = bytearray(xb * yb)
    for y in range(yb):
        for x in range(xb):
            a = fr.acs[y][x]
            acs[y * xb + x] = 0xFF if a is None else (a[0] << 1) | int(a[1])
    parts.append(bytes(acs))
    parts.append(np.asarray(fr.qf, np.int32).tobytes())
    parts.append(np.asarray(fr.sharp, np.uint8).tobytes())
    parts.append(np.asarray(fr.inv_sigma, np.float32).tobytes())
    parts.append(np.asarray(fr.qdc, np.uint8).tobytes())
    ch, cw = fr.ytox.shape
    parts.append(struct.pack("<II", cw, ch))
    parts.append(np.asarray(fr.ytox, np.int32).tobytes())
    parts.append(np.asarray(fr.ytob, np.int32).tobytes())
    return b"".join(parts)


VDC_PARTS = ("dc X", "dc Y", "dc B", "strategy", "quant field", "sharpness", "inv sigma", "dc bucket")


def check_vdc(exp):
    def chk(res, out):
        if res != 0:
            return f"error {it.err_name(res)}"
        if out == exp:
            return None
        xb, yb = struct.unpack_from("<II", exp)
        n = xb * yb
        sizes = (4 * n, 4 * n, 4 * n, n, 4 * n, n, 4 * n, n)
        p = 8
        for name, sz in zip(VDC_PARTS, sizes):
            a, e = out[p:p + sz], exp[p:p + sz]
            if a != e:
                w = 4 if sz == 4 * n else 1
                i = next(j for j in range(0, sz, w) if a[j:j + w] != e[j:j + w]) // w
                fmt = "<f" if name.startswith("dc ") or name == "inv sigma" else ("<i" if w == 4 else "<B")
                return f"{name} differs at block x {i % xb} y {i // xb}: " \
                       f"{struct.unpack_from(fmt, a, i * w)[0]} != {struct.unpack_from(fmt, e, i * w)[0]}"
            p += sz
        return "CfL tiles differ"
    return chk


def suite_vdc(runners, suites):
    recs = []
    for f in test_files():
        data = f.read_bytes()
        try:
            frames = model_frames(data)
        except J.JXLError:
            continue
        for k, fr in enumerate(frames):
            if fr.fh.modular:
                continue
            recs.append(it.Rec(OP_JXVDC, k, data, check_vdc(vdc_record(fr)), f"{f.name}#{k}"))
    for t, R in runners.items():
        s = it.Suite("vdc", t)
        it.run_suite(R, s, recs)
        suites.append(s)


def vpx_record(fr):
    import numpy as np
    parts = [struct.pack("<II", p.shape[1], p.shape[0]) for p in fr.px]
    parts += [np.asarray(p, np.float32).tobytes() for p in fr.px]
    return b"".join(parts)


def check_vpx(exp):
    def chk(res, out):
        if res != 0:
            return f"error {it.err_name(res)}"
        if out == exp:
            return None
        if out[:24] != exp[:24]:
            return f"plane sizes {struct.unpack_from('<6I', out)} != {struct.unpack_from('<6I', exp)}"
        p = 24
        for c in range(3):
            w, h = struct.unpack_from("<II", exp, 8 * c)
            a, e = out[p:p + 4 * w * h], exp[p:p + 4 * w * h]
            if a != e:
                fa = struct.unpack("<%df" % (w * h), a)
                fe = struct.unpack("<%df" % (w * h), e)
                bad = [i for i in range(w * h) if a[4 * i:4 * i + 4] != e[4 * i:4 * i + 4]]
                i = bad[0]
                md = max(abs(fa[j] - fe[j]) for j in bad)
                return f"plane {c} ({w}x{h}): {len(bad)} samples differ (max {md:.3g}), first x {i % w} y {i // w}: "                        f"{fa[i]!r} != {fe[i]!r}"
            p += 4 * w * h
        return "record differs"
    return chk


def suite_vpx(runners, suites):
    recs = []
    for f in test_files():
        data = f.read_bytes()
        try:
            frames = model_frames(data)
        except J.JXLError:
            continue
        for k, fr in enumerate(frames):
            if fr.fh.modular:
                continue
            recs.append(it.Rec(OP_JXVPX, k, data, check_vpx(vpx_record(fr)), f"{f.name}#{k}"))
    for t, R in runners.items():
        s = it.Suite("vpx", t)
        it.run_suite(R, s, recs)
        suites.append(s)


def rnd_record(fr):
    import numpy as np
    planes = list(fr.planes) + list(fr.ecs)
    alpha = -1
    for i, e in enumerate(fr.meta.extra):
        if e.type == J.ExtraChannelInfo.ALPHA:
            alpha = 3 + i
            break
    parts = [struct.pack("<Ii", len(planes), alpha)]
    parts += [struct.pack("<II", p.shape[1], p.shape[0]) for p in planes]
    parts += [np.ascontiguousarray(p, np.float32).tobytes() for p in planes]
    return b"".join(parts)


def check_rnd(exp):
    def chk(res, out):
        if res != 0:
            return f"error {it.err_name(res)}"
        if out == exp:
            return None
        n, alpha = struct.unpack_from("<Ii", exp)
        if out[:8 + 8 * n] != exp[:8 + 8 * n]:
            return f"header {struct.unpack_from('<Ii', out)} sizes {out[8:8 + 8 * n].hex()} != "                    f"{struct.unpack_from('<Ii', exp)} {exp[8:8 + 8 * n].hex()}"
        p = 8 + 8 * n
        for c in range(n):
            w, h = struct.unpack_from("<II", exp, 8 + 8 * c)
            a, e = out[p:p + 4 * w * h], exp[p:p + 4 * w * h]
            if a != e:
                fa = struct.unpack("<%df" % (w * h), a)
                fe = struct.unpack("<%df" % (w * h), e)
                bad = [i for i in range(w * h) if a[4 * i:4 * i + 4] != e[4 * i:4 * i + 4]]
                i = bad[0]
                md = max(abs(fa[j] - fe[j]) for j in bad)
                return f"plane {c} ({w}x{h}): {len(bad)} samples differ (max {md:.3g}), first x {i % w} y {i // w}: "                        f"{fa[i]!r} != {fe[i]!r}"
            p += 4 * w * h
        return "record differs"
    return chk


def suite_rnd(runners, suites):
    recs = []
    for f in test_files():
        data = f.read_bytes()
        try:
            frames = model_frames(data)
        except J.JXLError:
            continue
        for k, fr in enumerate(frames):
            recs.append(it.Rec(OP_JXRND, k, data, check_rnd(rnd_record(fr)), f"{f.name}#{k}"))
    for t, R in runners.items():
        s = it.Suite("rnd", t)
        it.run_suite(R, s, recs)
        suites.append(s)


def enc_images():
    """(name, w, h, RGBA bytes) test images of the encoder"""
    import numpy as np
    import jxl_enc_ref as E
    out = []
    fx = ROOT / "apps" / "PaintCraft" / "tests" / "fixtures"
    for n in ("a1_rgba.png", "a6_photo.png"):
        w, h, rgba = E.read_png(fx / n)
        out.append((n, w, h, rgba))
    rng = np.random.default_rng(7)

    def synth(name, w, h, kind):
        y, x = np.mgrid[0:h, 0:w]
        r = (x * 255 // max(1, w - 1)).astype(np.uint8)
        g = (y * 255 // max(1, h - 1)).astype(np.uint8)
        b = ((x + y) * 7 % 256).astype(np.uint8)
        a = np.full((h, w), 255, np.uint8)
        if kind == "gray":
            g = b = r
        elif kind == "alpha":
            a = ((x * 3 + y * 5) % 256).astype(np.uint8)
        elif kind == "noise":
            r, g, b = (rng.integers(0, 256, (h, w), dtype=np.uint8) for _ in range(3))
        px = np.stack([r, g, b, a], -1)
        out.append((name, w, h, px.tobytes()))
    synth("grad 300x200", 300, 200, "rgb")
    synth("gray 64x64", 64, 64, "gray")
    synth("alpha 257x13", 257, 13, "alpha")
    synth("noise 33x290", 33, 290, "noise")
    synth("1x1", 1, 1, "rgb")
    synth("gray alpha 520x9", 520, 9, "gray")
    return out


def check_enc(exp):
    def chk(res, out):
        if res != 0:
            return f"error {it.err_name(res)}"
        if out == exp:
            return None
        n = min(len(out), len(exp))
        i = next((k for k in range(n) if out[k] != exp[k]), n)
        return f"{len(out)} bytes, expected {len(exp)}; first difference at byte {i}"
    return chk


def suite_enc(runners, suites):
    import jxl_enc_ref as E
    recs = []
    for name, w, h, rgba in enc_images():
        exp = E.encode_lossless(w, h, rgba)
        size, meta, rgb, alpha = J.decode(exp)
        if J.to_rgba8(size, rgb, alpha) != rgba:
            raise SystemExit(f"model encoder output of {name} does not decode exactly")
        bgra = bytearray(len(rgba))
        bgra[0::4], bgra[1::4], bgra[2::4], bgra[3::4] = rgba[2::4], rgba[1::4], rgba[0::4], rgba[3::4]
        data = struct.pack("<III", w, h, it.C.FMT_ARGB) + bytes(bgra)
        recs.append(it.Rec(OP_JXENC, it.C.IOF_LOSSLESS, data, check_enc(exp), name))
        for q in (90, 70, 40, 15):
            ex = E.encode_vardct(w, h, rgba, E.distance_of_quality(q))
            recs.append(it.Rec(OP_JXENC, q << 16, data, check_enc(ex), f"{name} q{q}"))
    # metadata: EXIF (orientation 6 -> written as 1) and XMP in the container, read back by op 240
    tiff = b"II*\x00" + struct.pack("<IHHHII", 8, 1, 0x0112, 3, 1, 6) + struct.pack("<I", 0)
    tiff1 = tiff[:18] + struct.pack("<H", 1) + tiff[20:]
    xmp = b'<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF/></x:xmpmeta>'
    name, w, h, rgba = enc_images()[0]
    bgra = bytearray(len(rgba))
    bgra[0::4], bgra[1::4], bgra[2::4], bgra[3::4] = rgba[2::4], rgba[1::4], rgba[0::4], rgba[3::4]
    sdata = struct.pack("<III", w, h, it.C.FMT_ARGB) + bytes(bgra)
    trailer = struct.pack("<I", len(tiff)) + tiff + struct.pack("<I", len(xmp)) + xmp
    fl = it.C.IOF_EMBED_EXIF | it.C.IOF_EMBED_XMP
    for arg, cs in ((it.C.IOF_LOSSLESS | fl, E.encode_lossless(w, h, rgba)),
                    (fl | 90 << 16, E.encode_vardct(w, h, rgba, E.distance_of_quality(90)))):
        exp = E.wrap_container(cs, tiff1, xmp)
        if J.codestream(exp) != cs:
            raise SystemExit("container model: codestream not found again")
        recs.append(it.Rec(OP_JXENC, arg, sdata + trailer, check_enc(exp), f"{name} meta {arg & 0xFFFF:#x}"))

        def mchk(res, out, exp=exp):
            if res != 0:
                return f"error {it.err_name(res)}"
            d = it.parse_decex(out)
            if d["exif"] != tiff1 or d["xmp"] != xmp:
                return f"blobs: exif {d['exif'].hex()} xmp {d['xmp']!r}"
            return None
        recs.append(it.Rec(it.OP_DECEX, 0, it.decex_input(exp, flags=it.C.IMGD_STRAIGHT | it.C.IMGD_EXIF |
                                                          it.C.IMGD_XMP), mchk, f"{name} meta read back"))
    for t, R in runners.items():
        s = it.Suite("enc", t)
        it.run_suite(R, s, recs)
        suites.append(s)


def model_bgra(data):
    """the model's 8-bit output of every displayed frame: [(w, h, BGRA bytes)]"""
    size, meta, shown = J.decode_frames(data)
    out = []
    for rgb, alpha, _ in shown:
        rgba = J.to_rgba8(size, rgb, alpha)
        b = bytearray(len(rgba))
        b[0::4], b[1::4], b[2::4], b[3::4] = rgba[2::4], rgba[1::4], rgba[0::4], rgba[3::4]
        out.append((size.xsize, size.ysize, bytes(b)))
    return out


def check_dec(exp, unsup):
    def chk(res, out):
        if res == it.C.ERR_UNSUPPORTED:
            unsup.append(1)
            return None
        if res != 0:
            return f"error {it.err_name(res)}"
        w, h, fmt = struct.unpack_from("<III", out)
        ew, eh, px = exp
        if (w, h) != (ew, eh):
            return f"size {w}x{h} != {ew}x{eh}"
        got = out[12:]
        if got != px:
            d = [abs(a - b) for a, b in zip(got, px)]
            i = next(k for k, v in enumerate(d) if v)
            return f"pixels differ: max {max(d)}, {sum(1 for v in d if v)} bytes, first at " \
                   f"x {(i // 4) % w} y {(i // 4) // w} byte {i % 4}: {got[i]} != {px[i]}"
        return None
    return chk


def suite_dec(runners, suites):
    for t, R in runners.items():
        s = it.Suite("dec", t)
        unsup = []
        recs = []
        for f in test_files():
            data = f.read_bytes()
            try:
                exps = model_bgra(data)
            except J.JXLError:
                continue
            for k, exp in enumerate(exps):
                recs.append(it.Rec(OP_JXDEC, k << 16, data, check_dec(exp, unsup), f"{f.name}#{k}"))
            recs.append(it.Rec(OP_JXDEC, len(exps) << 16, data, it.expect_err(it.C.ERR_LIMIT),
                               f"{f.name}#{len(exps)} (past the end)"))
        it.run_suite(R, s, recs)
        if unsup:
            s.notes.append(f"{len(unsup)} files not supported by the decoder yet")
        suites.append(s)


def suite_info(runners, suites):
    files = test_files()
    for t, R in runners.items():
        s = it.Suite("info", t)
        recs = [it.Rec(OP_JXINFO, 0, f.read_bytes(), check_info(f.read_bytes()), f.name) for f in files]
        it.run_suite(R, s, recs)
        suites.append(s)


def main():
    ap = it.argparser(__doc__, "info,mod,vdc,vpx,rnd,dec,enc")
    a = ap.parse_args()
    out = it.out_dir(a)
    targets = it.targets_of(a)
    if not a.no_build:
        it.build_codectest(targets, out)
    only = set(a.only.split(",")) if a.only else {"info", "mod", "vdc", "vpx", "rnd", "dec", "enc"}
    runners = {t: it.Runner(t, out, work_name="jxl") for t in targets}
    suites = []
    if "info" in only:
        suite_info(runners, suites)
    if "mod" in only:
        suite_mod(runners, suites)
    if "vdc" in only:
        suite_vdc(runners, suites)
    if "vpx" in only:
        suite_vpx(runners, suites)
    if "rnd" in only:
        suite_rnd(runners, suites)
    if "dec" in only:
        suite_dec(runners, suites)
    if "enc" in only:
        suite_enc(runners, suites)
    ok = it.report(suites, "jxl_check")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
