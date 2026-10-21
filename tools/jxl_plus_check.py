#!/usr/bin/env python3
"""jxl_plus_check.py -- checks of the JPEG XL additions of the Rust rebuild (jxl-plus,
lib/img_jxl: enew / evdct / emod / ... and the decoder's DC frames and threads).

Runs codectest batches of the Rust build (ops 183 decode, 189 encode, 190 encode time,
191 transform self test, 194 decode time, 196 JPEG reconstruction, 240 decode with metadata) and compares with
libjxl 0.7 in WSL (ctypes, the helper below; no cjxl / djxl needed):

  self      forward transforms of every AC strategy against the decoder's inverse, the DC
            from the lowest frequencies, the writer's fast inverse == the decoder's
  lossless  the new writer at efforts 1..9 (and progressive = squeeze) on photos, synthetic
            images (gray, alpha, palette, text, noise, 1x1, several groups): our decoder and
            libjxl give the source exactly; size table against libjxl at the same effort
  lossy     quality 90 / 70 / 40 x efforts 3 / 5 / 7 / 9 (and progressive: DC frame + AC
            passes): our decoder == libjxl within 1 (the decoders round differently), alpha
            exact, PSNR rising with the quality; size / PSNR table against libjxl at the same
            effort and distance
  icc       ICC profiles (sRGB-like: written as sRGB in lossy files; P3 / Adobe-like / gray:
            kept, lossy as YCbCr): the profile read back by our decoder and libjxl, lossless
            exact, lossy within 1 of libjxl
  threads   the files of the pool == --threads 1 == IOF_JXL_1THREAD; the decoder's pixels
            with the pool == one thread
  decode    libjxl 0.7 files our decoder must read like libjxl (within 1): progressive DC
            (DC frames, levels 1 and 2), JPEG transcodes (4:4:4 / 4:2:0 / 4:2:2 / 4:4:0 /
            grey, baseline and progressive, generated with tools/jpeg_check.py's writer),
            responsive, resampling 2 / 4, non-XYB lossy, effort 1 / 9, group order
  recon     JPEG reconstruction (op 196, img_jxl::jpeg::reconstruct): JPEGs (4:4:4 / 4:2:0 /
            4:2:2 / 4:4:0, baseline / progressive, restart intervals, ICC / EXIF / XMP / COM
            markers, data between markers, a tail) transcoded by libjxl give the original file
            byte for byte; grey JPEGs (libjxl 0.7 cannot write their jbrd box) with a jbrd box
            written here; the transcodes' Brotli compressed Exif / xml boxes read back as
            metadata; no reconstruction for other files
  splines   (jxl-tools) the decoder's splines: files with random splines (test-only writer,
            op 197) over lossless, lossy, progressive and YCbCr frames, decoded by our decoder and
            libjxl 0.7: lossless frames identical, lossy XYB frames identical on a flat background
            (a ±1 of the XYB conversion allowed elsewhere); a spline with two equal successive
            control points is refused by both
  noise     (jxl-tools) photon noise (IO_FLAGS bits 24..31 = ISO / 100): the frame carries
            libjxl's noise LUT for that ISO (within one 10-bit step of libjxl 0.7's own photon
            noise files), costs about 10 bytes; our decoder and ffmpeg's libjxl (>= 0.8: libjxl
            0.7 synthesizes the grain differently, so it is compared only without noise) give
            the same pixels (within 1); lossless and the YCbCr path ignore it
  patches   (jxl-tools) patches on text, UI screenshots (the Rust gallery's --shot, light / dark,
            100 / 150 %) and photos: the same patches as libjxl 0.7 finds (positions, rectangles,
            reference frame size) at lossless e7 and d1 e7, none below effort 5 (lossless) / 7
            (lossy) nor with IOF_JXL_NOPATCHES, none in photos; lossless exact with our decoder
            and libjxl, lossy within 1 of libjxl; a size table with / without patches against
            libjxl 0.7 with / without (its option patches); dots (star fields, d >= 3, e7): the
            same dots as libjxl 0.7 (positions and sizes), none below distance 3 or effort 7
  butteraugli (jxl-tools) our Butteraugli (op 199, img_jxl::ebutter) against libjxl 0.7's
            JxlButteraugliCompute (RGB input) on synthetic and coded pairs: the largest value, the
            3-norm and the distance map within 0.1 %; the effort 8 / 9 quantization loop on
            photos, text, UI screenshots: files decoded by our decoder and libjxl (within 1),
            the largest distance below effort 7's on average; a size / Butteraugli / time table
            against libjxl 0.7 at efforts 7 / 8 / 9 and the time of one comparison
  speed     encode / decode times against libjxl (a table, no pass / fail)

  python build_rs.py app codectest --target both
  python tools/jxl_plus_check.py --target both [--only lossy,icc] [--quick] [--no-ext]
--no-ext skips everything that needs WSL / libjxl (the comparisons then only use our decoder).
"""
import json
import os
import random
import struct
import subprocess
import sys
import time
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
import imgtest_common as it  # noqa: E402

OP_JXDEC, OP_JXENC2, OP_JXTIME, OP_JXSELF, OP_JXDTIME, OP_DECEX = 183, 189, 190, 191, 194, 240
OP_JPEGRECON = 196
OP_JXSPLINE = 197      # jxl-tools: the test-only spline writer
OP_JXFEAT = 198        # jxl-tools: per frame: type, flags, size, patches, spline segments, noise LUT
OP_JXBA = 199          # jxl-tools: Butteraugli of two RGBA8 images (img_jxl::ebutter)
IOF_JXL_NOISE_SHIFT = 24
IOF_JXL_NOPATCHES = 0x800000
LJ_PATCHES = 8         # JXL_ENC_FRAME_SETTING_PATCHES
LJ_PHOTON_NOISE = 5    # JXL_ENC_FRAME_SETTING_PHOTON_NOISE (float)
IOF_LOSSLESS, IOF_EMBED_ICC = 1, 32
IOF_JXL_1THREAD, IOF_JXL_PROGRESSIVE = 0x200000, 0x400000
SUITES = "self,lossless,lossy,icc,threads,decode,recon,splines,noise,patches,butteraugli,speed"

# ------------------------------------------------------------------ libjxl in WSL (ctypes)

LJXL = r'''
import ctypes as C, json, struct, sys, time
lib = C.CDLL("libjxl.so.0.7")
try:
    thr = C.CDLL("libjxl_threads.so.0.7")
except OSError:
    thr = None
vp = C.c_void_p
for n, r, a in (("JxlDecoderCreate", vp, [vp]), ("JxlDecoderDestroy", None, [vp]),
                ("JxlDecoderSubscribeEvents", C.c_int, [vp, C.c_int]),
                ("JxlDecoderSetInput", C.c_int, [vp, C.c_char_p, C.c_size_t]),
                ("JxlDecoderCloseInput", None, [vp]), ("JxlDecoderProcessInput", C.c_int, [vp]),
                ("JxlDecoderGetBasicInfo", C.c_int, [vp, vp]),
                ("JxlDecoderGetICCProfileSize", C.c_int, [vp, vp, C.c_int, C.POINTER(C.c_size_t)]),
                ("JxlDecoderGetColorAsICCProfile", C.c_int, [vp, vp, C.c_int, vp, C.c_size_t]),
                ("JxlDecoderImageOutBufferSize", C.c_int, [vp, vp, C.POINTER(C.c_size_t)]),
                ("JxlDecoderSetImageOutBuffer", C.c_int, [vp, vp, vp, C.c_size_t]),
                ("JxlEncoderCreate", vp, [vp]), ("JxlEncoderDestroy", None, [vp]),
                ("JxlEncoderInitBasicInfo", None, [vp]), ("JxlEncoderSetBasicInfo", C.c_int, [vp, vp]),
                ("JxlEncoderFrameSettingsCreate", vp, [vp, vp]),
                ("JxlEncoderFrameSettingsSetOption", C.c_int, [vp, C.c_int, C.c_int64]),
                ("JxlEncoderFrameSettingsSetFloatOption", C.c_int, [vp, C.c_int, C.c_float]),
                ("JxlEncoderSetFrameDistance", C.c_int, [vp, C.c_float]),
                ("JxlEncoderSetFrameLossless", C.c_int, [vp, C.c_int]),
                ("JxlEncoderSetColorEncoding", C.c_int, [vp, vp]),
                ("JxlColorEncodingSetToSRGB", None, [vp, C.c_int]),
                ("JxlEncoderAddImageFrame", C.c_int, [vp, vp, vp, C.c_size_t]),
                ("JxlEncoderAddJPEGFrame", C.c_int, [vp, C.c_char_p, C.c_size_t]),
                ("JxlEncoderStoreJPEGMetadata", C.c_int, [vp, C.c_int]),
                ("JxlEncoderUseContainer", C.c_int, [vp, C.c_int]),
                ("JxlEncoderCloseInput", None, [vp]),
                ("JxlEncoderProcessOutput", C.c_int, [vp, C.POINTER(C.POINTER(C.c_uint8)), C.POINTER(C.c_size_t)])):
    f = getattr(lib, n)
    f.restype = r
    f.argtypes = a
if thr is not None:
    thr.JxlThreadParallelRunnerCreate.restype = vp
    thr.JxlThreadParallelRunnerCreate.argtypes = [vp, C.c_size_t]
    thr.JxlThreadParallelRunnerDestroy.argtypes = [vp]
    lib.JxlEncoderSetParallelRunner.argtypes = [vp, vp, vp]
    lib.JxlDecoderSetParallelRunner.argtypes = [vp, vp, vp]
class Fmt(C.Structure):
    _fields_ = [("n", C.c_uint32), ("t", C.c_int), ("e", C.c_int), ("a", C.c_size_t)]
def u32(b, o):
    return struct.unpack_from("<I", b, o)[0]
def runner(obj, setter, n):
    if thr is None or n <= 0:
        return None
    r = thr.JxlThreadParallelRunnerCreate(None, n)
    setter(obj, thr.JxlThreadParallelRunner, r)
    return r
def dec(j):
    data = open(j["in"], "rb").read()
    d = lib.JxlDecoderCreate(None)
    rn = runner(d, lib.JxlDecoderSetParallelRunner, j.get("threads", 0))
    lib.JxlDecoderSubscribeEvents(d, 0x40 | 0x100 | 0x1000)
    lib.JxlDecoderSetInput(d, data, len(data))
    lib.JxlDecoderCloseInput(d)
    info = C.create_string_buffer(512)
    w = h = 0; nch = 4; buf = None; frames = []; icc = b""
    t0 = time.perf_counter()
    try:
        while True:
            st = lib.JxlDecoderProcessInput(d)
            if st in (1, 2):
                return {"ok": False, "err": "status %d" % st}
            if st == 0x40:
                lib.JxlDecoderGetBasicInfo(d, info)
                w, h = u32(info.raw, 4), u32(info.raw, 8)
                nch = 2 if u32(info.raw, 52) == 1 else 4
            elif st == 0x100:
                n = C.c_size_t()
                if lib.JxlDecoderGetICCProfileSize(d, None, 0, C.byref(n)) == 0 and n.value:
                    b = C.create_string_buffer(n.value)
                    if lib.JxlDecoderGetColorAsICCProfile(d, None, 0, b, n.value) == 0:
                        icc = b.raw
            elif st == 5:
                f = Fmt(nch, 2, 1, 0)
                n = C.c_size_t()
                lib.JxlDecoderImageOutBufferSize(d, C.byref(f), C.byref(n))
                buf = C.create_string_buffer(n.value)
                lib.JxlDecoderSetImageOutBuffer(d, C.byref(f), buf, n.value)
            elif st == 0x1000:
                raw = buf.raw
                if nch == 2:
                    out = bytearray(w * h * 4)
                    out[0::4] = raw[0::2]; out[1::4] = raw[0::2]; out[2::4] = raw[0::2]; out[3::4] = raw[1::2]
                    raw = bytes(out)
                frames.append(raw)
            elif st == 0:
                break
            else:
                return {"ok": False, "err": "status %d" % st}
    finally:
        ms = (time.perf_counter() - t0) * 1000
        lib.JxlDecoderDestroy(d)
        if rn is not None:
            thr.JxlThreadParallelRunnerDestroy(rn)
    with open(j["out"], "wb") as f:
        f.write(struct.pack("<IIII", w, h, len(icc), len(frames)) + icc + b"".join(frames[:1]))
    return {"ok": True, "ms": ms}
def output(e):
    out = bytearray()
    while True:
        b = (C.c_uint8 * (1 << 20))()
        p = C.cast(b, C.POINTER(C.c_uint8))
        av = C.c_size_t(1 << 20)
        st = lib.JxlEncoderProcessOutput(e, C.byref(p), C.byref(av))
        out += bytes(b)[:(1 << 20) - av.value]
        if st == 0:
            return bytes(out)
        if st != 2:
            raise RuntimeError("encoder %d" % st)
def enc(j):
    raw = open(j["in"], "rb").read()
    w, h, ch = j["w"], j["h"], j["ch"]
    e = lib.JxlEncoderCreate(None)
    rn = runner(e, lib.JxlEncoderSetParallelRunner, j.get("threads", 0))
    try:
        info = C.create_string_buffer(512)
        lib.JxlEncoderInitBasicInfo(info)
        b = bytearray(info.raw)
        lossless = j["distance"] == 0
        struct.pack_into("<II", b, 4, w, h)
        struct.pack_into("<I", b, 12, 8)
        struct.pack_into("<i", b, 36, 1 if lossless or j.get("orig") else 0)
        struct.pack_into("<I", b, 52, 3)
        struct.pack_into("<II", b, 56, 1 if ch == 4 else 0, 8 if ch == 4 else 0)
        info = C.create_string_buffer(bytes(b), 512)
        if lib.JxlEncoderSetBasicInfo(e, info) != 0:
            return {"ok": False, "err": "basic info"}
        ce = C.create_string_buffer(256)
        lib.JxlColorEncodingSetToSRGB(ce, 0)
        lib.JxlEncoderSetColorEncoding(e, ce)
        fs = lib.JxlEncoderFrameSettingsCreate(e, None)
        lib.JxlEncoderFrameSettingsSetOption(fs, 0, j["effort"])
        for k, v in j.get("opts", {}).items():
            if lib.JxlEncoderFrameSettingsSetOption(fs, int(k), int(v)) != 0:
                return {"ok": False, "err": "option %s" % k}
        for k, v in j.get("fopts", {}).items():
            if lib.JxlEncoderFrameSettingsSetFloatOption(fs, int(k), C.c_float(v)) != 0:
                return {"ok": False, "err": "float option %s" % k}
        if lossless:
            lib.JxlEncoderSetFrameLossless(fs, 1)
        else:
            lib.JxlEncoderSetFrameDistance(fs, C.c_float(j["distance"]))
        f = Fmt(ch, 2, 1, 0)
        t0 = time.perf_counter()
        if lib.JxlEncoderAddImageFrame(fs, C.byref(f), raw, len(raw)) != 0:
            return {"ok": False, "err": "add frame"}
        lib.JxlEncoderCloseInput(e)
        out = output(e)
        ms = (time.perf_counter() - t0) * 1000
    finally:
        lib.JxlEncoderDestroy(e)
        if rn is not None:
            thr.JxlThreadParallelRunnerDestroy(rn)
    open(j["out"], "wb").write(out)
    return {"ok": True, "ms": ms, "size": len(out)}
def jpeg(j):
    data = open(j["in"], "rb").read()
    e = lib.JxlEncoderCreate(None)
    try:
        lib.JxlEncoderUseContainer(e, 1)
        lib.JxlEncoderStoreJPEGMetadata(e, j.get("meta", 1))
        fs = lib.JxlEncoderFrameSettingsCreate(e, None)
        lib.JxlEncoderFrameSettingsSetOption(fs, 0, j.get("effort", 7))
        if lib.JxlEncoderAddJPEGFrame(fs, data, len(data)) != 0:
            return {"ok": False, "err": "add jpeg"}
        lib.JxlEncoderCloseInput(e)
        out = output(e)
    finally:
        lib.JxlEncoderDestroy(e)
    open(j["out"], "wb").write(out)
    return {"ok": True, "size": len(out)}
def recon(j):
    lib.JxlDecoderSetJPEGBuffer.argtypes = [vp, vp, C.c_size_t]
    lib.JxlDecoderReleaseJPEGBuffer.restype = C.c_size_t
    lib.JxlDecoderReleaseJPEGBuffer.argtypes = [vp]
    data = open(j["in"], "rb").read()
    d = lib.JxlDecoderCreate(None)
    buf, out = None, None
    try:
        lib.JxlDecoderSubscribeEvents(d, 0x2000 | 0x1000)
        lib.JxlDecoderSetInput(d, data, len(data))
        lib.JxlDecoderCloseInput(d)
        while True:
            st = lib.JxlDecoderProcessInput(d)
            if st == 0x2000:
                buf = C.create_string_buffer(len(data) * 4 + 65536)
                lib.JxlDecoderSetJPEGBuffer(d, buf, len(buf))
            elif st in (5, 0x1000, 0) and buf is not None:
                out = buf.raw[:len(buf) - lib.JxlDecoderReleaseJPEGBuffer(d)]
                break
            else:
                return {"ok": False, "err": "status %d" % st}
    finally:
        lib.JxlDecoderDestroy(d)
    open(j["out"], "wb").write(out)
    return {"ok": True, "size": len(out)}
def br(j):
    b = C.CDLL("libbrotlienc.so.1")
    b.BrotliEncoderCompress.argtypes = [C.c_int, C.c_int, C.c_int, C.c_size_t, C.c_char_p, C.POINTER(C.c_size_t), C.c_char_p]
    data = open(j["in"], "rb").read()
    n = C.c_size_t(len(data) * 2 + 1024)
    o = C.create_string_buffer(n.value)
    if not b.BrotliEncoderCompress(9, 22, 0, len(data), data, C.byref(n), o):
        return {"ok": False, "err": "brotli"}
    open(j["out"], "wb").write(o.raw[:n.value])
    return {"ok": True}
def ba(j):
    for n, r, a in (("JxlButteraugliApiCreate", vp, [vp]), ("JxlButteraugliApiDestroy", None, [vp]),
                    ("JxlButteraugliApiSetIntensityTarget", None, [vp, C.c_float]),
                    ("JxlButteraugliApiSetHFAsymmetry", None, [vp, C.c_float]),
                    ("JxlButteraugliCompute", vp, [vp, C.c_uint32, C.c_uint32, vp, vp, C.c_size_t, vp, vp, C.c_size_t]),
                    ("JxlButteraugliResultGetDistance", C.c_float, [vp, C.c_float]),
                    ("JxlButteraugliResultGetMaxDistance", C.c_float, [vp]),
                    ("JxlButteraugliResultGetDistmap", None, [vp, C.POINTER(C.POINTER(C.c_float)), C.POINTER(C.c_uint32)]),
                    ("JxlButteraugliResultDestroy", None, [vp])):
        f = getattr(lib, n)
        f.restype = r
        f.argtypes = a
    a = open(j["a"], "rb").read()
    b = open(j["b"], "rb").read()
    w, h = j["w"], j["h"]
    api = lib.JxlButteraugliApiCreate(None)
    lib.JxlButteraugliApiSetIntensityTarget(api, C.c_float(j.get("intensity", 80.0)))
    lib.JxlButteraugliApiSetHFAsymmetry(api, C.c_float(j.get("hf_asym", 1.0)))
    f = Fmt(j.get("ch", 4), 2, 1, 0)
    t0 = time.perf_counter()
    res = lib.JxlButteraugliCompute(api, w, h, C.byref(f), a, len(a), C.byref(f), b, len(b))
    ms = (time.perf_counter() - t0) * 1000
    try:
        if not res:
            return {"ok": False, "err": "compute"}
        d3 = lib.JxlButteraugliResultGetDistance(res, C.c_float(3.0))
        mx = lib.JxlButteraugliResultGetMaxDistance(res)
        buf = C.POINTER(C.c_float)()
        stride = C.c_uint32()
        lib.JxlButteraugliResultGetDistmap(res, C.byref(buf), C.byref(stride))
        rows = b"".join(C.string_at(C.addressof(buf.contents) + 4 * stride.value * y, 4 * w) for y in range(h))
        open(j["out"], "wb").write(rows)
    finally:
        if res:
            lib.JxlButteraugliResultDestroy(res)
        lib.JxlButteraugliApiDestroy(api)
    return {"ok": True, "d3": d3, "max": mx, "ms": ms}
res = []
for j in json.load(open(sys.argv[1])):
    try:
        res.append({"dec": dec, "enc": enc, "jpeg": jpeg, "recon": recon, "br": br, "ba": ba}[j["op"]](j))
    except Exception as ex:
        res.append({"ok": False, "err": repr(ex)})
    json.dump(res, open(sys.argv[2], "w"))
'''


class Lj:
    """libjxl jobs in WSL; a job that aborts libjxl costs only itself"""

    def __init__(self, work):
        self.work = work
        self.script = work / "ljxl_helper.py"
        self.script.write_text(LJXL, encoding="utf-8", newline="\n")

    def run(self, jobs, timeout=3600):
        out = []
        pos = 0
        while pos < len(jobs):
            jf = self.work / "lj_jobs.json"
            rf = self.work / "lj_res.json"
            jf.write_text(json.dumps(jobs[pos:]))
            if rf.exists():
                rf.unlink()
            subprocess.run(["wsl.exe", "-d", "Ubuntu-24.04", "--cd", str(ROOT), "-e", "python3", rel(self.script),
                            rel(jf), rel(rf)], capture_output=True, timeout=timeout)
            done = json.loads(rf.read_text()) if rf.exists() else []
            out += done
            pos += len(done)
            if pos < len(jobs):
                out.append({"ok": False, "err": "libjxl aborted"})   # the job that crashed
                pos += 1
        return out

    def decode(self, files):
        jobs = [{"op": "dec", "in": rel(f), "out": rel(Path(str(f) + ".lj"))} for f in files]
        res = []
        for f, r in zip(files, self.run(jobs)):
            p = Path(str(f) + ".lj")
            if not r.get("ok") or not p.exists():
                res.append(None)
                continue
            b = p.read_bytes()
            w, h, n, nf = struct.unpack_from("<IIII", b, 0)
            icc = b[16:16 + n]
            px = np.frombuffer(b, np.uint8, count=w * h * 4, offset=16 + n).reshape(h, w, 4) if nf else None
            res.append((px, icc, r.get("ms", 0)))
            p.unlink()
        return res


def rel(p):
    return str(Path(p).resolve().relative_to(ROOT)).replace("\\", "/")


# ------------------------------------------------------------------ test images

def synth(kind, w, h, seed=7):
    rng = np.random.default_rng(seed)
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
    elif kind == "smooth":
        fx, fy = x / max(w, h), y / max(w, h)
        r = (127 + 100 * np.sin(fx * 6) * np.cos(fy * 4)).astype(np.uint8)
        g = (127 + 100 * np.cos(fx * 3 + fy * 2)).astype(np.uint8)
        b = (127 + 100 * np.sin((fx + fy) * 5)).astype(np.uint8)
    elif kind == "text":
        img = np.full((h, w, 3), 240, np.uint8)
        glyphs = [rng.random((9, 7)) > 0.55 for _ in range(6)]
        for row in range(h // 12):
            for col in range(w // 8):
                y0, x0 = 4 + row * 12, 3 + col * 8
                if y0 + 9 <= h and x0 + 7 <= w:
                    img[y0:y0 + 9, x0:x0 + 7][glyphs[(row * 7 + col * 3) % 6]] = (20, 20, 60)
        r, g, b = img[..., 0], img[..., 1], img[..., 2]
    elif kind == "pal":
        cols = rng.integers(0, 256, (12, 3), dtype=np.uint8)
        idx = (x // 13 + y // 7) * 5 % 12
        r, g, b = cols[idx, 0], cols[idx, 1], cols[idx, 2]
    elif kind == "photo":
        import jpeg_check as J
        p = J.photo(random.Random(seed), w, h)
        r, g, b = p[..., 0], p[..., 1], p[..., 2]
    return np.stack([r, g, b, a], -1).astype(np.uint8)


def fixture(name):
    import jxl_enc_ref as E
    w, h, rgba = E.read_png(ROOT / "apps/PaintCraft/tests/fixtures" / name)
    return np.frombuffer(rgba, np.uint8).reshape(h, w, 4).copy()


def images(quick):
    imgs = [("a6_photo", fixture("a6_photo.png")), ("a1_rgba", fixture("a1_rgba.png")),
            ("smooth300x200", synth("smooth", 300, 200)), ("gray64", synth("gray", 64, 64)),
            ("alpha257x13", synth("alpha", 257, 13)), ("noise33x290", synth("noise", 33, 290)),
            ("text520x300", synth("text", 520, 300)), ("pal600x450", synth("pal", 600, 450)),
            ("one1x1", synth("smooth", 1, 1))]
    if not quick:
        imgs.append(("photo1100x700", synth("photo", 1100, 700)))
    return imgs


# ------------------------------------------------------------------ ICC profiles

def icc_profile(name, prim=None, trc=None, gray=False):
    def s15(v):
        return struct.pack(">i", int(round(v * 65536)))

    def xyz(v):
        return b"XYZ \0\0\0\0" + b"".join(s15(c) for c in v)
    d50 = (0.9642, 1.0, 0.8249)
    t = name.encode() + b"\0"
    tags = [(b"desc", b"desc\0\0\0\0" + struct.pack(">I", len(t)) + t + b"\0" * 79), (b"wtpt", xyz(d50)),
            (b"cprt", b"text\0\0\0\0public domain\0")]
    tags += [(b"kTRC", trc)] if gray else [(b"rXYZ", xyz(prim[0])), (b"gXYZ", xyz(prim[1])), (b"bXYZ", xyz(prim[2])),
                                          (b"rTRC", trc), (b"gTRC", trc), (b"bTRC", trc)]
    off = 132 + 12 * len(tags)
    table = data = b""
    for sig, body in tags:
        data += b"\0" * (-(off + len(data)) % 4)
        table += sig + struct.pack(">II", off + len(data), len(body))
        data += body
    hdr = struct.pack(">I", 132 + 12 * len(tags) + len(data)) + b"none" + struct.pack(">I", 0x02100000) + b"mntr"
    hdr += (b"GRAY" if gray else b"RGB ") + b"XYZ " + struct.pack(">6H", 2024, 1, 1, 0, 0, 0) + b"acspMSFT"
    hdr += b"\0" * 4 + b"nonenone" + b"\0" * 12 + b"".join(s15(c) for c in d50) + b"none" + b"\0" * 44
    return hdr + struct.pack(">I", len(tags)) + table + data


def profiles():
    def gamma(g):
        return b"curv\0\0\0\0" + struct.pack(">IH", 1, int(round(g * 256))) + b"\0\0"
    para = b"para\0\0\0\0" + struct.pack(">H", 3) + b"\0\0" + b"".join(
        struct.pack(">i", int(round(v * 65536))) for v in (2.4, 1 / 1.055, 0.055 / 1.055, 1 / 12.92, 0.04045))
    srgb = [(0.4361, 0.2225, 0.0139), (0.3851, 0.7169, 0.0971), (0.1431, 0.0606, 0.7141)]
    p3 = [(0.5151, 0.2412, -0.0011), (0.2920, 0.6922, 0.0419), (0.1571, 0.0666, 0.7841)]
    adobe = [(0.6097, 0.3111, 0.0195), (0.2053, 0.6257, 0.0609), (0.1492, 0.0632, 0.7446)]
    p = {"srgb_para": (icc_profile("sRGB", srgb, para), True), "p3_g22": (icc_profile("P3", p3, gamma(2.2)), False),
         "adobe": (icc_profile("Adobe-like", adobe, gamma(563 / 256)), False),
         "gray_g22": (icc_profile("gray", trc=gamma(2.2), gray=True), False)}
    win = Path("C:/Windows/System32/spool/drivers/color/sRGB Color Space Profile.icm")
    if win.exists():
        p["win_srgb"] = (win.read_bytes(), True)
    return p


# ------------------------------------------------------------------ records

def enc_rec(name, px, flags=0, quality=0, effort=7, icc=b"", op=OP_JXENC2, arg=0, check=None, exif=b"", xmp=b""):
    h, w = px.shape[:2]
    data = struct.pack("<III", flags, quality, effort) + struct.pack("<III", w, h, it.C.FMT_ARGB)
    data += px[..., [2, 1, 0, 3]].tobytes()
    data += struct.pack("<I", len(icc)) + icc + struct.pack("<I", len(exif)) + exif + struct.pack("<I", len(xmp)) + xmp
    return it.Rec(op, arg, data, check, name)


def surface_rgba(out):
    w, h, _ = struct.unpack_from("<III", out, 0)
    return np.frombuffer(out[12:12 + w * h * 4], np.uint8).reshape(h, w, 4)[..., [2, 1, 0, 3]]


def psnr(a, b, premul=False):
    x, y = a[..., :3].astype(np.float64), b[..., :3].astype(np.float64)
    if premul:   # what is visible: the colours times the source alpha
        al = b[..., 3:4].astype(np.float64) / 255
        x, y = x * al, y * al
    m = np.mean((x - y) ** 2)
    return 99.0 if m == 0 else 10 * np.log10(255 * 255 / m)


def dist(q):
    return 0.1 + (100 - q) * 0.09 if q >= 30 else 53 / 3000 * q * q - 23 / 20 * q + 25


class Ctx:
    def __init__(self, target, out, a):
        self.target = target
        self.R = it.Runner(target, out, work_name="jxlplus")
        self.R1 = it.Runner(target, out, extra=["--threads", "1"], work_name="jxlplus")
        self.work = self.R.work
        self.ext = not a.no_ext
        self.quick = a.quick
        self.lj = Lj(self.work) if self.ext else None
        self.notes = []

    def encode(self, recs):
        out = []
        for r, res in zip(recs, self.R.batch(recs)):
            out.append(None if res is None or res[0] != 0 else bytes(res[1]))
        return out

    def save(self, name, data):
        f = self.work / (name.replace(" ", "_") + ".jxl")
        f.write_bytes(data)
        return f

    def decode(self, blobs, runner=None):
        R = runner or self.R
        res = R.batch([it.Rec(OP_JXDEC, 0, b, None, "dec") for b in blobs])
        return [surface_rgba(r[1]) if r is not None and r[0] == 0 else None for r in res]


# ------------------------------------------------------------------ suites

def suite_self(c, s):
    res = c.R.batch([it.Rec(OP_JXSELF, 0, b"", None, "self")])[0]
    if res is None or res[0] != 0:
        s.fail("op 191 failed")
        return
    v = struct.unpack("<%df" % (len(res[1]) // 4), res[1])
    names = "DCT ID DCT2 DCT4 16x16 32x32 16x8 8x16 32x8 8x32 32x16 16x32 4x8 8x4 AFV0 AFV1 AFV2 AFV3 64x64 64x32 32x64"
    for i, n in enumerate(names.split()):
        s.check(v[3 * i] < 1e-5, f"{n}: round trip {v[3 * i]:.3g}")
        s.check(v[3 * i + 1] < 1e-5, f"{n}: DC {v[3 * i + 1]:.3g}")
        s.check(v[3 * i + 2] == 0, f"{n}: fast inverse differs by {v[3 * i + 2]:.3g}")


def suite_lossless(c, s):
    efforts = [1, 3, 5, 7, 9] if c.quick else [1, 2, 3, 4, 5, 6, 7, 8, 9]
    imgs = images(c.quick)
    cases = [(n, px, e, fl) for n, px in imgs for e in efforts for fl in (0, IOF_JXL_PROGRESSIVE)
             if fl == 0 or e in (3, 7, 9)]
    blobs = c.encode([enc_rec(f"{n} e{e} {fl:x}", px, IOF_LOSSLESS | fl, 0, e) for n, px, e, fl in cases])
    files = []
    for (n, px, e, fl), b in zip(cases, blobs):
        s.check(b is not None, f"{n} e{e} flags {fl:#x}: encode failed")
        files.append(c.save(f"ll_{n}_e{e}_{fl:x}", b) if b else None)
    ok = [(k, f) for k, f in zip(cases, files) if f]
    for (n, px, e, fl), f, d in zip([k for k, _ in ok], [f for _, f in ok], c.decode([f.read_bytes() for _, f in ok])):
        s.check(d is not None and d.shape == px.shape and np.array_equal(d, px), f"{n} e{e} {fl:#x}: own decode not exact")
    if not c.ext:
        return
    lres = c.lj.decode([f for _, f in ok])
    for ((n, px, e, fl), f), r in zip(ok, lres):
        s.check(r is not None and r[0] is not None and np.array_equal(r[0], px), f"{n} e{e} {fl:#x}: libjxl decode not exact")
    # sizes against libjxl at the same effort
    jobs, keys = [], []
    for n, px, e, fl in cases:
        if fl:
            continue
        raw = c.work / f"src_{n}.rgba"
        raw.write_bytes(px.tobytes())
        jobs.append({"op": "enc", "in": rel(raw), "out": rel(c.work / f"lj_{n}_e{e}.jxl"), "w": px.shape[1],
                     "h": px.shape[0], "ch": 4, "effort": e, "distance": 0})
        keys.append((n, e))
    lr = c.lj.run(jobs)
    ours = {(k[0], k[2]): f.stat().st_size for k, f in ok if not k[3]}
    tot_o = tot_l = 0
    lines = ["lossless sizes (bytes): image / effort / ours / libjxl 0.7 / ratio"]
    for (n, e), r in zip(keys, lr):
        o, l = ours.get((n, e)), r.get("size") if r.get("ok") else None
        if o and l:
            tot_o += o
            tot_l += l
        lines.append("  %-14s e%d %9s %9s %6s" % (n, e, o, l, "%.3f" % (o / l) if o and l else "-"))
    lines.append("  total ours / libjxl: %.3f" % (tot_o / max(1, tot_l)))
    c.notes += lines


def suite_lossy(c, s):
    efforts = [3, 7] if c.quick else [3, 5, 7, 9]
    quals = [90, 40] if c.quick else [90, 70, 40]
    imgs = images(c.quick)
    cases = [(n, px, q, e, fl) for n, px in imgs for q in quals for e in efforts for fl in (0, IOF_JXL_PROGRESSIVE)
             if fl == 0 or e == 7]
    blobs = c.encode([enc_rec(f"{n} q{q} e{e}", px, fl, q, e) for n, px, q, e, fl in cases])
    ok = []
    for k, b in zip(cases, blobs):
        s.check(b is not None, "%s q%d e%d %#x: encode failed" % (k[0], k[2], k[3], k[4]))
        if b:
            ok.append((k, c.save("ly_%s_q%d_e%d_%x" % (k[0], k[2], k[3], k[4]), b)))
    own = c.decode([f.read_bytes() for _, f in ok])
    ps = {}
    for (k, f), d in zip(ok, own):
        n, px, q, e, fl = k
        if d is None or d.shape != px.shape:
            s.fail(f"{n} q{q} e{e} {fl:#x}: own decode failed")
            continue
        s.check(np.array_equal(d[..., 3], px[..., 3]), f"{n} q{q} e{e} {fl:#x}: alpha not exact")
        ps[(n, q, e, fl)] = (psnr(d, px), f.stat().st_size)
    for n, _ in imgs:
        for e in efforts:
            v = [ps.get((n, q, e, 0)) for q in quals]
            if all(v):
                s.check(v[0][1] >= v[-1][1] * 0.9, f"{n} e{e}: q{quals[0]} smaller than q{quals[-1]}")
                s.check(v[0][0] >= v[-1][0] - 0.5, f"{n} e{e}: PSNR falls with the quality")
    if not c.ext:
        return
    lres = c.lj.decode([f for _, f in ok])
    for ((k, f), d, r) in zip(ok, own, lres):
        n, px, q, e, fl = k
        if d is None:
            continue
        s.check(r is not None and r[0] is not None and r[0].shape == d.shape
                and int(np.abs(r[0].astype(int) - d.astype(int)).max()) <= 1, f"{n} q{q} e{e} {fl:#x}: libjxl differs")
    # sizes / PSNR against libjxl 0.7 at the same effort and distance (not progressive)
    jobs, keys = [], []
    for n, px, q, e, fl in cases:
        if fl:
            continue
        raw = c.work / f"src_{n}.rgba"
        raw.write_bytes(px.tobytes())
        jobs.append({"op": "enc", "in": rel(raw), "out": rel(c.work / f"ljq_{n}_q{q}_e{e}.jxl"), "w": px.shape[1],
                     "h": px.shape[0], "ch": 4, "effort": e, "distance": dist(q)})
        keys.append((n, q, e, px))
    lr = c.lj.run(jobs)
    files = [c.work / f"ljq_{n}_q{q}_e{e}.jxl" for (n, q, e, _), r in zip(keys, lr) if r.get("ok")]
    ld = iter(c.lj.decode(files))
    lines = ["lossy (bytes, PSNR dB): image / q / effort / ours / libjxl 0.7 (same effort, distance) / size ratio"]
    for (n, q, e, px), r in zip(keys, lr):
        lp = None
        if r.get("ok"):
            dd = next(ld)
            lp = psnr(dd[0], px) if dd and dd[0] is not None else None
        o = ps.get((n, q, e, 0))
        lines.append("  %-14s q%d e%d  %8s %6s | %8s %6s  %6s" % (
            n, q, e, o[1] if o else "-", "%.2f" % o[0] if o else "-", r.get("size", "-"),
            "%.2f" % lp if lp else "-", "%.3f" % (o[1] / r["size"]) if o and r.get("size") else "-"))
    c.notes += lines


def suite_icc(c, s):
    prof = profiles()
    imgs = {"a6": fixture("a6_photo.png"), "alpha": synth("alpha", 257, 13), "gray": synth("gray", 64, 64)}
    modes = [("ll", IOF_LOSSLESS, 0, 7), ("q90", 0, 90, 7), ("q90p", IOF_JXL_PROGRESSIVE, 90, 7), ("q60e3", 0, 60, 3)]
    cases = []
    for pn, (icc, srgb) in prof.items():
        for iname, px in imgs.items():
            if (pn == "gray_g22") != (iname == "gray"):
                continue
            for mn, fl, q, e in modes:
                cases.append((pn, iname, mn, px, icc, srgb, fl, q, e))
    blobs = c.encode([enc_rec("%s %s %s" % k[:3], k[3], k[6] | IOF_EMBED_ICC, k[7], k[8], k[4]) for k in cases])
    ok = []
    for k, b in zip(cases, blobs):
        s.check(b is not None, "%s %s %s: encode failed" % k[:3])
        if b:
            ok.append((k, c.save("icc_%s_%s_%s" % k[:3], b)))
    res = c.R.batch([it.Rec(OP_DECEX, 0, it.decex_input(f.read_bytes(), it.C.IMGD_ICC | it.C.IMGD_STRAIGHT), None, "d")
                     for _, f in ok])
    own = []
    for (k, f), r in zip(ok, res):
        pn, iname, mn, px, icc, srgb, fl, q, e = k
        name = f"{pn} {iname} {mn}"
        if r is None or r[0] != 0:
            s.fail(f"{name}: own decode failed")
            own.append(None)
            continue
        d = it.parse_decex(r[1])
        p = d["px"][..., [2, 1, 0, 3]]
        own.append(p)
        lossless = fl & IOF_LOSSLESS
        # lossy sRGB-like profiles are written as sRGB (no profile)
        want = b"" if (srgb and not lossless) else icc
        s.check(d["icc"] == want, f"{name}: profile read back {len(d['icc'])} bytes, expected {len(want)}")
        if lossless:
            s.check(np.array_equal(p, px), f"{name}: not exact")
        else:
            s.check(psnr(p, px, True) > 25, f"{name}: PSNR {psnr(p, px, True):.1f}")
    # ICC + EXIF + XMP together, lossless and lossy: all three read back by our decoder
    exif = it.exif_blob(orient=1)
    xmp = b'<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF/></x:xmpmeta>'
    p3 = prof["p3_g22"][0]
    mrecs = [enc_rec(f"meta {fl}", imgs["a6"], fl | IOF_EMBED_ICC | 64 | 128, q, 7, p3, exif=exif, xmp=xmp)
             for fl, q in ((IOF_LOSSLESS, 0), (0, 85), (IOF_JXL_PROGRESSIVE, 85))]
    for r, res in zip(mrecs, c.R.batch(mrecs)):
        if res is None or res[0] != 0:
            s.fail(f"{r.name}: encode failed")
            continue
        d = c.R.batch([it.Rec(OP_DECEX, 0, it.decex_input(bytes(res[1]), it.C.IMGD_ICC | it.C.IMGD_EXIF | it.C.IMGD_XMP),
                              None, "m")])[0]
        if d is None or d[0] != 0:
            s.fail(f"{r.name}: decode failed")
            continue
        dd = it.parse_decex(d[1])
        s.check(dd["icc"] == p3, f"{r.name}: ICC not read back")
        s.check(dd["exif"].endswith(exif[-16:]) and len(dd["exif"]) >= len(exif) - 8, f"{r.name}: EXIF not read back")
        s.check(dd["xmp"] == xmp, f"{r.name}: XMP not read back")
    if not c.ext:
        return
    lres = c.lj.decode([f for _, f in ok])
    for (k, f), p, r in zip(ok, own, lres):
        pn, iname, mn, px, icc, srgb, fl, q, e = k
        name = f"{pn} {iname} {mn}"
        if r is None or r[0] is None or p is None:
            s.fail(f"{name}: libjxl decode failed")
            continue
        want = b"" if (srgb and not fl & IOF_LOSSLESS) else icc
        if want:
            s.check(r[1] == want, f"{name}: libjxl profile differs")
        s.check(int(np.abs(r[0].astype(int) - p.astype(int)).max()) <= 1, f"{name}: libjxl pixels differ")


def suite_threads(c, s):
    imgs = [("a6", fixture("a6_photo.png")), ("text", synth("text", 1100, 700)), ("pal", synth("pal", 600, 450))]
    if not c.quick:
        imgs.append(("photo", synth("photo", 2100, 1700)))
    modes = [(IOF_LOSSLESS, 0, 3), (IOF_LOSSLESS, 0, 7), (IOF_LOSSLESS | IOF_JXL_PROGRESSIVE, 0, 7), (0, 90, 3),
             (0, 90, 7), (IOF_JXL_PROGRESSIVE, 90, 7), (0, 60, 9)]
    recs = [enc_rec(f"{n} {fl:#x} q{q} e{e}", px, fl, q, e) for n, px in imgs for fl, q, e in modes]
    a = c.R.batch(recs)
    b = c.R1.batch(recs)
    recs3 = [enc_rec(f"{n} {fl:#x} q{q} e{e}", px, fl | IOF_JXL_1THREAD, q, e) for n, px in imgs for fl, q, e in modes]
    c3 = c.R.batch(recs3)
    blobs = []
    for r, x, y, z in zip(recs, a, b, c3):
        same = x is not None and y is not None and z is not None and x[0] == 0 and bytes(x[1]) == bytes(y[1]) == bytes(z[1])
        s.check(same, f"{r.name}: the file depends on the threads")
        if x is not None and x[0] == 0:
            blobs.append((r.name, bytes(x[1])))
    # the decoder: pool == one thread
    dr = [it.Rec(OP_JXDTIME, 1, d, None, n) for n, d in blobs]
    p1 = c.R.batch(dr)
    p2 = c.R1.batch(dr)
    for (n, _), x, y in zip(blobs, p1, p2):
        ok = x is not None and y is not None and x[0] == 0 and y[0] == 0 and bytes(x[1])[8:] == bytes(y[1])[8:]
        s.check(ok, f"{n}: decoded pixels depend on the threads")


def suite_decode(c, s):
    if not c.ext:
        c.notes.append("decode: skipped (--no-ext)")
        return
    import jpeg_check as J
    rng = random.Random(5)
    jobs, names = [], []
    for (w, h) in ((17, 33), (64, 48), (300, 200)) if c.quick else ((17, 33), (64, 48), (101, 37), (300, 200), (520, 390)):
        for samp in ("444", "420", "422", "440", "gray"):
            for mode in ("baseline", "prog"):
                sp = [(1, 1)] if samp == "gray" else J.SAMPLINGS[samp]
                spec, coefs, qtabs, qsel = J.make_case(rng, w, h, sp, "gray" if samp == "gray" else "ycc", quality=80)
                kw = {"mode": "prog", "scans": J.default_script(spec.nc)} if mode == "prog" else {}
                f = c.work / f"j{w}x{h}_{samp}_{mode}.jpg"
                f.write_bytes(bytes(J.write_jpeg(spec, coefs, qtabs, qsel, **kw)))
                jobs.append({"op": "jpeg", "in": rel(f), "out": rel(c.work / (f.stem + ".jxl")), "meta": 0 if samp == "gray" else 1})
                names.append(f.stem)
    px = fixture("a6_photo.png")
    big = synth("photo", 2100, 1700) if not c.quick else synth("photo", 700, 530)
    variants = [("pdc1", 1.0, {19: 1}), ("pdc2", 1.0, {19: 2}), ("pdc_all", 2.5, {17: 1, 18: 1, 19: 2}),
                ("resp", 0, {16: 1}), ("resp_lossy", 1.5, {16: 1, 11: 1}), ("rs2", 2.0, {2: 2}), ("rs4", 3.0, {2: 4}),
                ("orig", 1.0, {}), ("go", 1.0, {13: 1}), ("e9", 0.5, {0: 9}), ("e1", 1.0, {0: 1})]
    for iname, img in (("a6", px), ("big", big)):
        raw = c.work / f"src_{iname}.rgba"
        raw.write_bytes(img.tobytes())
        for vn, d, opts in variants:
            o = {k: v for k, v in opts.items() if k != 0}
            jobs.append({"op": "enc", "in": rel(raw), "out": rel(c.work / f"lj_{iname}_{vn}.jxl"), "w": img.shape[1],
                         "h": img.shape[0], "ch": 4, "effort": opts.get(0, 7), "distance": d, "opts": o,
                         "orig": vn == "orig"})
            names.append(f"lj_{iname}_{vn}")
    lr = c.lj.run(jobs)
    files = []
    for n, j, r in zip(names, jobs, lr):
        if not r.get("ok"):
            s.fail(f"{n}: libjxl could not write it ({r.get('err')})")
            continue
        files.append((n, ROOT / j["out"]))
    own = c.decode([f.read_bytes() for _, f in files])
    ref = c.lj.decode([f for _, f in files])
    for (n, f), d, r in zip(files, own, ref):
        if d is None:
            s.fail(f"{n}: our decoder failed")
            continue
        if r is None or r[0] is None:
            s.fail(f"{n}: libjxl failed to decode it")
            continue
        md = int(np.abs(r[0].astype(int) - d.astype(int)).max()) if r[0].shape == d.shape else 999
        s.check(md <= 1, f"{n}: max |ours - libjxl| = {md}")


# ------------------------------------------------------------------ JPEG reconstruction

D_JBRD = {"apptype": [0, 0, 1, 0, 2, 1, 4, 2], "oneto4": [1, 0, 2, 0, 3, 0, 4, 0], "nhuff": [4, 0, 2, 3, 10, 4, 26, 6],
          "count": [0, 0, 1, 0, 2, 3, 0, 8], "value": [0, 2, 4, 2, 8, 4, 1, 8], "lastpass": [0, 0, 1, 0, 2, 0, 3, 3],
          "runs": [0, 0, 1, 2, 4, 4, 20, 16], "tail": [0, 0, 1, 8, 257, 16, 65793, 22]}


class JbrdBits:
    """LSB-first bit writer with the U32 fields of the JPEGData bundle"""

    def __init__(self):
        self.bits = []

    def f(self, n, v):
        self.bits += [(v >> i) & 1 for i in range(n)]

    def u32(self, name, v):
        dist = D_JBRD[name]
        for k in range(4):
            off, nb = dist[2 * k], dist[2 * k + 1]
            if off <= v < off + (1 << nb):
                self.f(2, k)
                self.f(nb, v - off)
                return
        raise ValueError((name, v))

    def data(self):
        self.bits += [0] * (-len(self.bits) % 8)
        return bytes(sum(b << i for i, b in enumerate(self.bits[k:k + 8])) for k in range(0, len(self.bits), 8))


def jbrd_bundle(jpg):
    """JPEGData bundle (every APP marker 'unknown', no reset points / zero runs / padding bits) of
    a simple JPEG and the bytes for its Brotli stream"""
    assert jpg[:2] == b"\xFF\xD8"
    p, order, app, com, quant, huff, comps, scans, restart = 2, [], [], [], [], [], [], [], 0
    while True:
        m = jpg[p + 1]
        order.append(m)
        if m == 0xD9:
            tail = jpg[p + 2:]
            break
        n = struct.unpack(">H", jpg[p + 2:p + 4])[0]
        body = jpg[p + 4:p + 2 + n]
        if m & 0xF0 == 0xE0:
            app.append(jpg[p + 1:p + 2 + n])
        elif m == 0xFE:
            com.append(jpg[p + 1:p + 2 + n])
        elif m == 0xDB:
            q = 0
            while q < len(body):
                ln = 1 + 64 * (2 if body[q] >> 4 else 1)
                quant.append([body[q] >> 4, body[q] & 15, q + ln >= len(body)])
                q += ln
        elif m == 0xC4:
            q = 0
            while q < len(body):
                counts = [0] + list(body[q + 1:q + 17])
                k = sum(counts)
                vals = list(body[q + 17:q + 17 + k]) + [256]
                counts[max(i for i in range(17) if counts[i])] += 1
                huff.append((body[q] >> 4, body[q] & 15, q + 17 + k >= len(body), counts, vals))
                q += 17 + k
        elif m in (0xC0, 0xC1, 0xC2):
            comps = [(body[6 + 3 * i], body[8 + 3 * i]) for i in range(body[5])]
        elif m == 0xDD:
            restart = struct.unpack(">H", body[:2])[0]
        p += 2 + n
        if m == 0xDA:
            k = body[0]
            ids = [cc[0] for cc in comps]
            sc = [(ids.index(body[1 + 2 * i]), body[2 + 2 * i] & 15, body[2 + 2 * i] >> 4) for i in range(k)]
            scans.append((sc, body[1 + 2 * k], body[2 + 2 * k], body[3 + 2 * k]))
            while not (jpg[p] == 0xFF and jpg[p + 1] != 0 and not 0xD0 <= jpg[p + 1] <= 0xD7):
                p += 1
    bw = JbrdBits()
    bw.f(1, int(len(comps) == 1))
    for m in order:
        bw.f(6, m - 0xC0)
    for a in app:
        bw.u32("apptype", 0)
        bw.f(16, len(a) - 1)
    for a in com:
        bw.f(16, len(a) - 1)
    bw.u32("oneto4", len(quant))
    for pr, idx, last in quant:
        bw.f(1, pr)
        bw.f(2, idx)
        bw.f(1, int(last))
    ids = [cc[0] for cc in comps]
    bw.f(2, 0 if ids == [1] else 1 if ids == [1, 2, 3] else 3)
    if ids not in ([1], [1, 2, 3]):
        bw.u32("oneto4", len(ids))
        for i in ids:
            bw.f(8, i)
    for _, tq in comps:
        bw.f(2, next(k for k in range(len(quant)) if quant[k][1] == tq))
    bw.u32("nhuff", len(huff))
    for ac, tid, last, counts, vals in huff:
        bw.f(1, ac)
        bw.f(2, tid)
        bw.f(1, int(last))
        for v in counts:
            bw.u32("count", v)
        for v in vals:
            bw.u32("value", v)
    for sc, ss, se, a in scans:
        bw.u32("oneto4", len(sc))
        bw.f(6, ss)
        bw.f(6, se)
        bw.f(4, a & 15)
        bw.f(4, a >> 4)
        for ci, ac, dc in sc:
            bw.f(2, ci)
            bw.f(2, ac)
            bw.f(2, dc)
        bw.u32("lastpass", 0)
    if 0xDD in order:
        bw.f(16, restart)
    for _ in scans:
        bw.u32("runs", 0)
        bw.u32("runs", 0)
    bw.u32("tail", len(tail))
    bw.f(1, 0)
    return bw.data(), b"".join(app) + b"".join(com) + tail


def jxl_boxes(d):
    p, out = 0, []
    while p + 8 <= len(d):
        n, t = struct.unpack(">I4s", d[p:p + 8])
        n = n or len(d) - p
        out.append((t, d[p:p + n]))
        p += n
    return out


def suite_recon(c, s):
    import jpeg_check as J
    rng = random.Random(9)
    xmp = b'<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?><x:xmpmeta xmlns:x="adobe:ns:meta/"/><?xpacket end="w"?>'
    profs = profiles()

    def seg(m, b):
        return bytes([0xFF, m]) + struct.pack(">H", len(b) + 2) + b

    def markers(jpg, icc=None, xm=None):
        ins = seg(0xE2, b"ICC_PROFILE\0\x01\x01" + icc) if icc else b""
        ins += seg(0xE1, b"http://ns.adobe.com/xap/1.0/\0" + xm) if xm else b""
        q = 4 + struct.unpack(">H", jpg[4:6])[0] if jpg[2:4] == b"\xFF\xE0" else 2
        return jpg[:q] + ins + jpg[q:]
    sizes = ((17, 33), (300, 200)) if c.quick else ((1, 1), (17, 33), (101, 37), (300, 200), (520, 390))
    variants = [{}, {"restart": 3}, {"exif": 1}, {"huff": "std", "trailing": b"tail"}, {"fill": True},
                {"icc": "p3_g22"}, {"xmp": True}, {"icc": "adobe", "xmp": True, "exif": 6, "extra": True}]
    cases = []      # (name, jpg file, grey)
    for (w, h) in sizes:
        for samp in ("444", "420", "422", "440", "gray"):
            for mode in ("baseline", "prog"):
                sp = [(1, 1)] if samp == "gray" else J.SAMPLINGS[samp]
                spec, coefs, qt, qsel = J.make_case(rng, w, h, sp, "gray" if samp == "gray" else "ycc",
                                                    quality=rng.choice([50, 80, 95]))
                if samp == "gray":
                    qt = {0: qt[0]}             # (a transcode has no unused table that differs)
                for vi, v in enumerate(variants if (w, h) == (300, 200) or not c.quick else variants[:2]):
                    if samp == "gray" and ("icc" in v or "xmp" in v or "exif" in v or "fill" in v):
                        continue                # (the grey jbrd boxes here: no metadata markers, fill bytes)
                    kw = {k: x for k, x in v.items() if k not in ("icc", "xmp", "extra")}
                    if mode == "prog":
                        kw.pop("huff", None)
                        kw.update({"mode": "prog", "scans": J.default_script(spec.nc)})
                    if v.get("extra"):
                        kw["com"] = True
                    icc = profs[v["icc"]][0] if "icc" in v else None
                    jpg = markers(J.write_jpeg(spec, coefs, qt, qsel, **kw), icc, xmp if v.get("xmp") else None)
                    f = c.work / f"jr{w}x{h}_{samp}_{mode}_{vi}.jpg"
                    f.write_bytes(jpg)
                    cases.append((f.stem, f, samp == "gray"))
    if not c.ext:
        c.notes.append("recon: skipped (--no-ext: the transcodes come from libjxl)")
        return
    efforts = (7,) if c.quick else (3, 7, 9)
    jobs, keys = [], []
    for n, f, gray in cases:
        for e in (efforts if not gray else (7,)):
            jobs.append({"op": "jpeg", "in": rel(f), "out": rel(c.work / f"{n}_e{e}.jxl"), "effort": e,
                         "meta": 0 if gray else 1})
            keys.append((f"{n}_e{e}", f, gray))
    lr = c.lj.run(jobs)
    # grey: libjxl's transcode without reconstruction data + a jbrd box written here
    bj, files = [], []
    for (n, f, gray), j, r in zip(keys, jobs, lr):
        if not r.get("ok"):
            s.fail(f"{n}: libjxl could not transcode it ({r.get('err')})")
            continue
        if gray:
            bundle, raw = jbrd_bundle(f.read_bytes())
            (c.work / (n + ".bundle")).write_bytes(bundle)
            (c.work / (n + ".brin")).write_bytes(raw)
            bj.append({"op": "br", "in": rel(c.work / (n + ".brin")), "out": rel(c.work / (n + ".br"))})
        files.append((n, f, ROOT / j["out"], gray))
    for r in c.lj.run(bj):
        s.check(r.get("ok"), f"brotli encode failed ({r.get('err')})")
    for n, f, x, gray in files:
        if gray:
            body = (c.work / (n + ".bundle")).read_bytes() + (c.work / (n + ".br")).read_bytes()
            out, done = b"", False
            for t, b in jxl_boxes(x.read_bytes()):
                if not done and t in (b"jxlc", b"jxlp"):
                    out += struct.pack(">I4s", 8 + len(body), b"jbrd") + body
                    done = True
                out += b
            x.write_bytes(out)
    own = c.R.batch([it.Rec(OP_JPEGRECON, 0, x.read_bytes(), None, n) for n, _, x, _ in files])
    lres = c.lj.run([{"op": "recon", "in": rel(x), "out": rel(Path(str(x) + ".lj.jpg"))} for _, _, x, _ in files])
    nlj = 0
    for (n, f, x, gray), o, r in zip(files, own, lres):
        orig = f.read_bytes()
        if o is None or o[0] != 0:
            s.fail(f"{n}: reconstruction failed ({o and o[0]})")
        else:
            s.check(bytes(o[1]) == orig, f"{n}: the reconstruction differs from the JPEG")
        lj = Path(str(x) + ".lj.jpg")
        nlj += bool(r.get("ok")) and lj.exists() and lj.read_bytes() == orig
    c.notes.append(f"recon: {len(files)} JPEG transcodes; libjxl 0.7 reconstructed {nlj} of them byte-identical")
    # the Brotli compressed Exif / xml boxes of the transcodes as metadata
    meta = [(n, f, x) for n, f, x, gray in files if not gray and any(k in n for k in ("_2_", "_6_", "_7_"))]
    res = c.R.batch([it.Rec(OP_DECEX, 0, it.decex_input(x.read_bytes(), it.C.IMGD_EXIF | it.C.IMGD_XMP), None, n)
                     for n, _, x in meta])
    for (n, f, x), r in zip(meta, res):
        if r is None or r[0] != 0:
            s.fail(f"{n}: decode with metadata failed")
            continue
        d = it.parse_decex(r[1])
        jpg = f.read_bytes()
        k = jpg.find(b"Exif\0\0")
        if k > 0:
            ln = struct.unpack(">H", jpg[k - 2:k])[0]
            s.check(d["exif"] == jpg[k + 6:k - 2 + ln], f"{n}: EXIF of the brob box not read back")
        if b"http://ns.adobe.com/xap/1.0/\0" in jpg:
            s.check(d["xmp"] == xmp, f"{n}: XMP of the brob box not read back")
    # no reconstruction without a jbrd box
    plain = c.encode([enc_rec("q90", synth("photo", 64, 48), 0, 90, 7)])[0]
    r = c.R.batch([it.Rec(OP_JPEGRECON, 0, plain or b"", None, "plain")])[0]
    s.check(r is not None and r[0] != 0, "a file without jbrd box was 'reconstructed'")


# ------------------------------------------------------------------ splines (jxl-tools)

def spline_desc(rng, w, h, n, qadj=0, dup=False):
    """op 197's description of n random splines: short curves (|delta| <= 12 px per control
    point, less on small images), sigma 1..2.7 px -- inside libjxl 0.7's spline area limit
    (tighter than libjxl 0.11's, which our decoder applies)"""
    b = struct.pack("<Ii", n, qadj)
    small = w * h < 2000
    for k in range(n):
        x, y = rng.randrange(w), rng.randrange(h)
        npts = rng.randrange(1, 3 if small else 6)
        b += struct.pack("<iiI", x, y, npts)
        dx = dy = 0
        for j in range(npts):
            m = 6 if small else 12
            ndx, ndy = rng.randrange(-m, m + 1), rng.randrange(-m, m + 1)
            if (ndx, ndy) == (0, 0) and not (dup and k == 0 and j == 0):
                ndx = 5
            if dup and k == 0 and j == 0:
                ndx = ndy = 0
            b += struct.pack("<ii", ndx - dx, ndy - dy)
            dx, dy = ndx, ndy
        for amp in (3, 6, 5):
            co = [rng.randrange(-amp, amp + 1) if i < 4 or rng.random() < 0.2 else 0 for i in range(32)]
            co[0] = rng.randrange(-4 * amp, 4 * amp + 1)
            b += struct.pack("<32i", *co)
        sg = [0] * 32
        sg[0] = rng.randrange(3, 6 if small else 9)
        sg[1] = rng.randrange(-2, 3)
        b += struct.pack("<32i", *sg)
    return b


def suite_splines(c, s):
    rng = random.Random(11)
    sizes = [(17, 23), (64, 48), (200, 150)] + ([] if c.quick else [(300, 260), (520, 390)])
    p3 = profiles()["p3_g22"][0]
    modes = [("ll e7", IOF_LOSSLESS, 0, 7), ("ll e3", IOF_LOSSLESS, 0, 3), ("q90 e7", 0, 90, 7), ("q60 e3", 0, 60, 3),
             ("q85 prog", IOF_JXL_PROGRESSIVE, 85, 7), ("q90 p3", IOF_EMBED_ICC, 90, 7)]
    cases = []      # name, op 197 data, plain record, lossless, flat
    for k, (w, h) in enumerate(sizes * (1 if c.quick else 2)):
        for bg in ("smooth", "flat"):
            px = synth("smooth", w, h, seed=k)
            if bg == "flat":
                px[..., :3] = (90, 140, 200)
            d = spline_desc(rng, w, h, rng.randrange(1, 5), rng.choice([0, 0, 3, -2]))
            for mn, fl, q, e in modes:
                icc = p3 if fl & IOF_EMBED_ICC else b""
                data = struct.pack("<III", fl, q, e) + d + struct.pack("<III", w, h, it.C.FMT_ARGB)
                data += px[..., [2, 1, 0, 3]].tobytes() + struct.pack("<I", len(icc)) + icc + struct.pack("<II", 0, 0)
                cases.append((f"{k} {w}x{h} {bg} {mn}", data, enc_rec("p", px, fl, q, e, icc), bool(fl & IOF_LOSSLESS),
                              bg == "flat" and not fl & IOF_EMBED_ICC))
    res = c.R.batch([it.Rec(OP_JXSPLINE, 0, d, None, n) for n, d, *_ in cases])
    pres = c.R.batch([p for _, _, p, *_ in cases])
    files = []
    for (n, _, _, ll, flat), r, pr in zip(cases, res, pres):
        if r is None or r[0] != 0 or pr is None or pr[0] != 0:
            s.fail(f"{n}: encode failed")
            continue
        f = c.save("spl_" + n, bytes(r[1]))
        g = c.save("spl_" + n + "_plain", bytes(pr[1]))
        files.append((n, f, g, ll, flat))
    own = c.decode([f.read_bytes() for _, f, *_ in files])
    ownp = c.decode([g.read_bytes() for _, _, g, *_ in files])
    vis_total = 0
    for (n, *_), a, ap in zip(files, own, ownp):
        s.check(a is not None and ap is not None, f"{n}: own decode failed")
        if a is not None and ap is not None:
            vis_total += int((a != ap).any(-1).sum())
    s.check(vis_total > 0, "the splines change no pixel")
    # refused by both decoders: two equal successive control points; a spline in a 1x1 image
    # (at most pixels / 2 control points)
    bads = []
    for bn, (w, h), dup in (("dup", (64, 48), True), ("1x1", (1, 1), False)):
        bad = struct.pack("<III", IOF_LOSSLESS, 0, 7) + spline_desc(rng, w, h, 1 + dup, dup=dup)
        bad += struct.pack("<III", w, h, it.C.FMT_ARGB) + synth("smooth", w, h)[..., [2, 1, 0, 3]].tobytes()
        br = c.R.batch([it.Rec(OP_JXSPLINE, 0, bad, None, bn)])[0]
        if br is None or br[0] != 0:
            s.fail(f"{bn}: encode failed")
            continue
        bf = c.save("spl_" + bn, bytes(br[1]))
        s.check(c.decode([bf.read_bytes()])[0] is None, f"{bn}: not refused")
        bads.append((bn, bf))
    if not c.ext:
        c.notes.append(f"splines: {len(files)} files, {vis_total} spline pixels (libjxl skipped: --no-ext)")
        return
    ref = c.lj.decode([f for _, f, *_ in files])
    refp = c.lj.decode([g for _, _, g, *_ in files])
    nsame = nvis = nflat = 0
    for (n, f, g, ll, flat), a, ap, l, lp in zip(files, own, ownp, ref, refp):
        if a is None or ap is None:
            continue
        if l is None or l[0] is None or lp is None or lp[0] is None:
            s.fail(f"{n}: libjxl failed to decode it")
            continue
        a, ap, b, bp = (x.astype(int) for x in (a, ap, l[0], lp[0]))
        vis = (a != ap).any(-1) | (b != bp).any(-1)
        d = np.abs(a - b).max(-1)
        nvis += int(vis.sum())
        nsame += int((vis & (d == 0)).sum())
        if ll:
            s.check(int(d.max()) == 0, f"{n}: lossless, |ours - libjxl| up to {int(d.max())}")
        elif flat:
            nflat += int(vis.sum())
            s.check(int(d.max()) <= 1 and int((d > 0).sum()) <= max(2, int(vis.sum()) // 200),
                    f"{n}: flat lossy: {int((d > 0).sum())} pixels differ (max {int(d.max())})")
        else:
            s.check(int(d.max()) <= 1, f"{n}: |ours - libjxl| up to {int(d.max())}")
    for bn, bf in bads:
        r = c.lj.decode([bf])[0]
        s.check(r is None or r[0] is None, f"{bn}: libjxl decoded it")
    c.notes.append(f"splines: {len(files)} files, {nvis} pixels changed by splines, {nsame} of them identical to "
                   f"libjxl 0.7 ({nflat} on flat lossy backgrounds)")


# ------------------------------------------------------------------ noise (jxl-tools)

def feat_records(out):
    """op 198's records: per frame (type, flags, w, h, patch positions, rects, rect pixels, spline
    segments, noise LUT)"""
    b = bytes(out)
    fr = []
    for k in range(len(b) // 64):
        v = struct.unpack_from("<8I8f", b, k * 64)
        fr.append({"type": v[0], "flags": v[1], "w": v[2], "h": v[3], "npos": v[4], "nrect": v[5], "rectpx": v[6],
                   "segs": v[7], "lut": v[8:16]})
    return fr


def suite_noise(c, s):
    flat = np.zeros((192, 256, 4), np.uint8)
    flat[...] = (128, 128, 128, 255)
    imgs = [("a6", fixture("a6_photo.png")), ("flat", flat), ("smooth", synth("smooth", 300, 200)),
            ("alpha", synth("alpha", 257, 13))]
    isos = [100, 1600, 25500] if c.quick else [100, 400, 1600, 6400, 25500]
    modes = [("q90 e7", 0, 90, 7), ("q60 e3", 0, 60, 3), ("q85 prog", IOF_JXL_PROGRESSIVE, 85, 7)]
    cases = [(n, px, iso, m) for n, px in imgs for iso in [0] + isos for m in modes]
    blobs = c.encode([enc_rec(f"{n} {iso}", px, m[1] | (iso // 100) << IOF_JXL_NOISE_SHIFT, m[2], m[3])
                      for n, px, iso, m in cases])
    got = {}
    for (n, px, iso, m), b in zip(cases, blobs):
        s.check(b is not None, f"{n} iso {iso} {m[0]}: encode failed")
        if b:
            got[(n, iso, m[0])] = b
    feats = c.R.batch([it.Rec(OP_JXFEAT, 0, b, None, "f") for b in got.values()])
    fmap = {k: (feat_records(r[1]) if r is not None and r[0] == 0 else None) for k, r in zip(got, feats)}
    for (n, iso, mn), b in got.items():
        fr = fmap[(n, iso, mn)]
        if not fr:
            s.fail(f"{n} iso {iso} {mn}: op 198 failed")
            continue
        main = fr[-1]
        s.check(bool(main["flags"] & 1) == (iso > 0), f"{n} iso {iso} {mn}: noise flag {main['flags']:#x}")
        p = got.get((n, 0, mn))
        if iso and p:
            s.check(abs(len(b) - len(p) - 10) <= 1, f"{n} iso {iso} {mn}: {len(b) - len(p)} bytes for the noise")
    # lossless and YCbCr (an ICC profile that is not sRGB): no noise
    p3 = profiles()["p3_g22"][0]
    for nm, fl, q, icc in (("lossless", IOF_LOSSLESS, 0, b""), ("ycc", IOF_EMBED_ICC, 90, p3)):
        a, b = c.encode([enc_rec(nm, imgs[0][1], fl, q, 7, icc),
                         enc_rec(nm, imgs[0][1], fl | 16 << IOF_JXL_NOISE_SHIFT, q, 7, icc)])
        s.check(a is not None and a == b, f"{nm}: the noise option changed the file")
    files = {k: c.save("noise_%s_%d_%s" % k, b) for k, b in got.items()}
    own = dict(zip(files, c.decode([f.read_bytes() for f in files.values()])))
    for (n, iso, mn), d in own.items():
        if not iso:
            continue
        p = own.get((n, 0, mn))
        s.check(d is not None and p is not None and not np.array_equal(d, p), f"{n} iso {iso} {mn}: no visible grain")
    # a recent libjxl through ffmpeg: the grain as ours
    nff = 0
    if it.have_exe("ffmpeg"):
        for k, f in files.items():
            d = own[k]
            fd = it.ffmpeg_decode(f, "rgba")
            if k[0] == "alpha" or fd is None or d is None:
                continue
            nff += 1
            s.check(fd.shape == d.shape and int(np.abs(fd.astype(int) - d.astype(int)).max()) <= 1,
                    "%s iso %d %s: ffmpeg's libjxl differs" % k)
        c.notes.append(f"noise: {nff} files compared with ffmpeg's libjxl")
    else:
        c.notes.append("noise: ffmpeg not found (grain not compared with a recent libjxl)")
    if not c.ext:
        return
    plain = [k for k in files if k[1] == 0]
    lres = dict(zip(plain, c.lj.decode([files[k] for k in plain])))
    for k in plain:
        d, r = own[k], lres[k]
        s.check(d is not None and r is not None and r[0] is not None and r[0].shape == d.shape
                and int(np.abs(r[0].astype(int) - d.astype(int)).max()) <= 1, "%s iso %d %s: libjxl differs" % k)
    # libjxl's own photon noise files: the same LUT
    jobs, keys = [], []
    for n, px in imgs[:2]:
        raw = c.work / f"src_{n}.rgba"
        raw.write_bytes(px.tobytes())
        for iso in isos:
            jobs.append({"op": "enc", "in": rel(raw), "out": rel(c.work / f"ljn_{n}_{iso}.jxl"), "w": px.shape[1],
                         "h": px.shape[0], "ch": 4, "effort": 7, "distance": 1.0, "fopts": {LJ_PHOTON_NOISE: iso}})
            keys.append((n, iso))
    lr = c.lj.run(jobs)
    recs = [it.Rec(OP_JXFEAT, 0, (ROOT / j["out"]).read_bytes(), None, "f") for j, r in zip(jobs, lr) if r.get("ok")]
    lf = iter(c.R.batch(recs))
    lines = ["noise LUT (ISO: ours / libjxl 0.7, the 8 values * 1024):"]
    for (n, iso), j, r in zip(keys, jobs, lr):
        if not r.get("ok"):
            s.fail(f"libjxl photon noise {n} {iso}: {r.get('err')}")
            continue
        fr = next(lf)
        lut = feat_records(fr[1])[-1]["lut"] if fr is not None and fr[0] == 0 else None
        ours = fmap.get((n, iso, "q90 e7"))
        ol = ours[-1]["lut"] if ours else None
        ok = lut is not None and ol is not None and all(abs(a - b) <= 1.01 / 1024 for a, b in zip(ol, lut))
        s.check(ok, f"{n} iso {iso}: LUT differs from libjxl's")
        if n == "a6" and ol and lut:
            lines.append("  %5d  %s / %s" % (iso, " ".join("%4d" % round(v * 1024) for v in ol),
                                              " ".join("%4d" % round(v * 1024) for v in lut)))
    c.notes += lines


# ------------------------------------------------------------------ patches (jxl-tools)

def screenshots(c):
    """UI screenshots of the Rust gallery (--shot, built for Windows when missing): light / dark,
    100 / 150 %; [] when it cannot be built or run"""
    import jxl_enc_ref as E
    exe = ROOT / "build/win/gallery.exe"
    if not exe.exists():
        subprocess.run([sys.executable, str(ROOT / "build_rs.py"), "app", "gallery", "--target", "win"],
                       cwd=ROOT, capture_output=True, timeout=1800)
    out = []
    for name, args in (("ui_light", ["--size", "1000x700", "--theme", "light"]), ("ui_dark", ["--size", "1000x700", "--theme", "dark"]),
                       ("ui_light150", ["--size", "900x650", "--scale", "150", "--theme", "light"]),
                       ("ui_dark150", ["--size", "900x650", "--scale", "150", "--theme", "dark"])):
        f = c.work / f"{name}.png"
        if not f.exists() and exe.exists():
            subprocess.run([str(exe), "--shot", rel(f)] + args, cwd=ROOT, capture_output=True, timeout=300)
        if f.exists():
            w, h, rgba = E.read_png(f)
            out.append((name, np.frombuffer(rgba, np.uint8).reshape(h, w, 4).copy()))
    return out


def stars(w, h, n, seed):
    """a star field: Gaussian dots (sigma 0.6..1.3) on a dark gradient with a little noise"""
    rng = np.random.default_rng(seed)
    y, x = np.mgrid[0:h, 0:w].astype(np.float64)
    img = np.full((h, w, 3), 12.0) + (x / w * 10)[..., None]
    for _ in range(n):
        cx, cy, sg, a = rng.uniform(4, w - 4), rng.uniform(4, h - 4), rng.uniform(0.6, 1.3), rng.uniform(80, 240)
        col = np.array([1.0, rng.uniform(0.8, 1.0), rng.uniform(0.7, 1.1)])
        img += (a * np.exp(-((x - cx) ** 2 + (y - cy) ** 2) / (2 * sg * sg)))[..., None] * col
    img += rng.normal(0, 2.0, img.shape)
    a = np.clip(np.round(img), 0, 255).astype(np.uint8)
    return np.concatenate([a, np.full((h, w, 1), 255, np.uint8)], -1)


def patch_positions(out):
    """op 198 arg 1 records: per frame (the 16 values, [(ref, x0, y0, w, h, x, y)])"""
    b = bytes(out)
    res, o = [], 0
    while o + 64 <= len(b):
        v = struct.unpack_from("<8I8f", b, o)
        o += 64
        ps = [struct.unpack_from("<7I", b, o + 28 * k) for k in range(v[4])]
        o += 28 * v[4]
        res.append((v, ps))
    return res


def suite_dots(c, s):
    """part of suite patches: dots"""
    imgs = [("stars300", stars(300, 200, 60, 1)), ("stars512", stars(512, 384, 200, 2)),
            ("stars_dense", stars(400, 300, 600, 3))]
    modes = [("q60 e7", 0, 60, 7, True), ("q40 e7", 0, 40, 7, True), ("q75 e7", 0, 75, 7, False),
             ("q60 e5", 0, 60, 5, False), ("q60 e7 nopatches", IOF_JXL_NOPATCHES, 60, 7, False),
             ("q60 e9", 0, 60, 9, True)]
    cases = [(n, px, m) for n, px in imgs for m in modes]
    blobs = c.encode([enc_rec(f"{n} {m[0]}", px, m[1], m[2], m[3]) for n, px, m in cases])
    ok = [(k, b) for k, b in zip(cases, blobs) if b]
    s.check(len(ok) == len(cases), f"dots: {len(cases) - len(ok)} encodes failed")
    feats = c.R.batch([it.Rec(OP_JXFEAT, 1, b, None, "f") for _, b in ok])
    own = c.decode([b for _, b in ok])
    pos = {}
    for ((n, px, m), b), f, d in zip(ok, feats, own):
        fr = patch_positions(f[1]) if f is not None and f[0] == 0 else None
        if not fr:
            s.fail(f"{n} {m[0]}: op 198 failed")
            continue
        ps = sorted((p[5], p[6], p[3], p[4]) for p in fr[-1][1])
        pos[(n, m[0])] = ps
        s.check(bool(ps) == m[4], f"{n} {m[0]}: {len(ps)} dots")
        s.check(all(w <= 4 and h <= 4 for _, _, w, h in ps), f"{n} {m[0]}: a dot larger than 4x4")
        s.check(d is not None and d.shape == px.shape, f"{n} {m[0]}: own decode failed")
    if not c.ext:
        return
    lres = c.lj.decode([c.save(f"dots_{n}_{m[0]}", b) for (n, px, m), b in ok])
    for ((n, px, m), b), d, r in zip(ok, own, lres):
        s.check(d is not None and r is not None and r[0] is not None and r[0].shape == d.shape
                and int(np.abs(r[0].astype(int) - d.astype(int)).max()) <= 1, f"{n} {m[0]}: libjxl differs")
    jobs, keys = [], []
    for n, px in imgs:
        raw = c.work / f"src_{n}.rgba"
        raw.write_bytes(px.tobytes())
        for mn, fl, q, e, _ in modes:
            if fl:
                continue
            for pt in (1, 0):
                jobs.append({"op": "enc", "in": rel(raw), "out": rel(c.work / f"ljdots_{n}_{mn.replace(' ', '_')}_{pt}.jxl"),
                             "w": px.shape[1], "h": px.shape[0], "ch": 4, "effort": e, "distance": dist(q),
                             "opts": {} if pt else {LJ_PATCHES: 0}})
                keys.append((n, mn, pt))
    lr = c.lj.run(jobs)
    got = [(k, j) for k, j, r in zip(keys, jobs, lr) if r.get("ok") and k[2]]
    lf = c.R.batch([it.Rec(OP_JXFEAT, 1, (ROOT / j["out"]).read_bytes(), None, "f") for _, j in got])
    for (k, j), f in zip(got, lf):
        fr = patch_positions(f[1]) if f is not None and f[0] == 0 else None
        lp = sorted((p[5], p[6], p[3], p[4]) for p in fr[-1][1]) if fr else None
        s.check(lp is not None and pos.get(k[:2]) == lp,
                f"{k[0]} {k[1]}: dots {len(pos.get(k[:2]) or [])}, libjxl {len(lp or [])} (positions / sizes differ)")
    sizes = {k: r["size"] for k, r in zip(keys, lr) if r.get("ok")}
    lines = ["dots: bytes ours with / without | libjxl 0.7 with / without (its patches option), dots found (ours = libjxl)"]
    for ((n, px, m), b) in ok:
        if m[1] or m[0] == "q60 e5":
            continue
        nb = next((bb for (kk, bb) in ok if kk[0] == n and kk[2][0] == "q60 e7 nopatches"), None)
        if m[0] != "q60 e7" or nb is None:
            lines.append("  %-12s %-7s %7d           | %7s / %7s  %d" % (n, m[0], len(b), sizes.get((n, m[0], 1), "-"),
                                                                       sizes.get((n, m[0], 0), "-"), len(pos.get((n, m[0]), []))))
        else:
            lines.append("  %-12s %-7s %7d / %7d | %7s / %7s  %d" % (n, m[0], len(b), len(nb), sizes.get((n, m[0], 1), "-"),
                                                                   sizes.get((n, m[0], 0), "-"), len(pos.get((n, m[0]), []))))
    c.notes += lines


def suite_patches(c, s):
    suite_dots(c, s)
    gt = synth("text", 260, 150)
    gt[..., 1] = gt[..., 0]
    gt[..., 2] = gt[..., 0]
    at = synth("text", 300, 200)
    at[..., 3] = 200
    shots = screenshots(c)
    if not shots:
        c.notes.append("patches: no gallery screenshots (gallery.exe could not be built / run)")
    imgs = [("text", synth("text", 520, 300)), ("graytext", gt), ("alphatext", at)] + (shots[:2] if c.quick else shots)
    imgs += [("a6_photo", fixture("a6_photo.png"))]
    modes = [("ll e3", IOF_LOSSLESS, 0, 3), ("ll e5", IOF_LOSSLESS, 0, 5), ("ll e7", IOF_LOSSLESS, 0, 7),
             ("ll e9", IOF_LOSSLESS, 0, 9), ("q90 e5", 0, 90, 5), ("q90 e7", 0, 90, 7), ("q60 e7", 0, 60, 7),
             ("q90 e7 prog", IOF_JXL_PROGRESSIVE, 90, 7)]
    if c.quick:
        modes = [m for m in modes if m[0] in ("ll e3", "ll e7", "q90 e5", "q90 e7")]
    cases = [(n, px, m, nop) for n, px in imgs for m in modes for nop in (0, IOF_JXL_NOPATCHES)]
    blobs = c.encode([enc_rec(f"{n} {m[0]}", px, m[1] | nop, m[2], m[3]) for n, px, m, nop in cases])
    files = {}
    for (n, px, m, nop), b in zip(cases, blobs):
        s.check(b is not None, f"{n} {m[0]} {nop:#x}: encode failed")
        if b:
            files[(n, m[0], nop)] = c.save("pt_%s_%s_%x" % (n, m[0], nop), b)
    keys = list(files)
    feats = c.R.batch([it.Rec(OP_JXFEAT, 0, files[k].read_bytes(), None, "f") for k in keys])
    fm = {k: (feat_records(r[1]) if r is not None and r[0] == 0 else None) for k, r in zip(keys, feats)}
    own = dict(zip(keys, c.decode([files[k].read_bytes() for k in keys])))
    pxs = dict(imgs)
    mtab = {m[0]: m for m in modes}
    for k in keys:
        n, mn, nop = k
        fr = fm[k]
        if not fr:
            s.fail(f"{n} {mn}: op 198 failed")
            continue
        m = mtab[mn]
        lossless = bool(m[1] & IOF_LOSSLESS)
        want = not nop and m[3] >= (5 if lossless else 7) and n != "a6_photo"
        got = fr[-1]["npos"] > 0
        if lossless and m[3] == 9 and want:
            # effort 9 keeps the smaller of the files with / without patches
            other = files.get((n, mn, IOF_JXL_NOPATCHES))
            s.check(other is not None and files[k].stat().st_size <= other.stat().st_size,
                    f"{n} {mn}: larger than without patches")
        else:
            s.check(got == want, f"{n} {mn} {nop:#x}: patches {'found' if got else 'missing'}")
        if got:
            s.check(len(fr) >= 2 and fr[0]["type"] == 2, f"{n} {mn}: no reference frame before the patches")
        d = own[k]
        if d is None:
            s.fail(f"{n} {mn} {nop:#x}: own decode failed")
        elif lossless:
            s.check(np.array_equal(d, pxs[n]), f"{n} {mn} {nop:#x}: lossless not exact")
    if not c.ext:
        return
    lres = dict(zip(keys, c.lj.decode([files[k] for k in keys])))
    for k in keys:
        d, r = own[k], lres[k]
        if d is None:
            continue
        ok = r is not None and r[0] is not None and r[0].shape == d.shape
        md = int(np.abs(r[0].astype(int) - d.astype(int)).max()) if ok else 999
        lossless = bool(mtab[k[1]][1] & IOF_LOSSLESS)
        s.check(md == 0 if lossless else md <= 1, "%s %s %#x: |ours - libjxl| = %d" % (k + (md,)))
    # libjxl 0.7 at the same effort / distance, with and without its patches
    jobs, jk = [], []
    for n, px in imgs:
        raw = c.work / f"src_{n}.rgba"
        raw.write_bytes(px.tobytes())
        for mn, fl, q, e in modes:
            if fl & IOF_JXL_PROGRESSIVE:
                continue
            for pt in (1, 0):
                jobs.append({"op": "enc", "in": rel(raw), "out": rel(c.work / f"ljpt_{n}_{mn.replace(' ', '_')}_{pt}.jxl"),
                             "w": px.shape[1], "h": px.shape[0], "ch": 4, "effort": e,
                             "distance": 0 if fl & IOF_LOSSLESS else dist(q), "opts": {} if pt else {LJ_PATCHES: 0}})
                jk.append((n, mn, pt))
    lr = c.lj.run(jobs)
    ljf = {k: ROOT / j["out"] for k, j, r in zip(jk, jobs, lr) if r.get("ok")}
    lfe = dict(zip(ljf, c.R.batch([it.Rec(OP_JXFEAT, 0, f.read_bytes(), None, "f") for f in ljf.values()])))
    ljd = dict(zip(ljf, c.lj.decode(list(ljf.values()))))
    # the same patches as libjxl (detection), lossless e7 and q90 e7
    for n, _ in imgs:
        for mn in ("ll e7", "q90 e7"):
            a, b = fm.get((n, mn, 0)), lfe.get((n, mn, 1))
            if a is None or b is None or b[0] != 0:
                continue
            b = feat_records(b[1])
            pa = (a[-1]["npos"], a[-1]["nrect"], a[-1]["rectpx"], a[0]["w"] if len(a) > 1 else 0, a[0]["h"] if len(a) > 1 else 0)
            pb = (b[-1]["npos"], b[-1]["nrect"], b[-1]["rectpx"], b[0]["w"] if len(b) > 1 else 0, b[0]["h"] if len(b) > 1 else 0)
            s.check(pa == pb, f"{n} {mn}: patches (positions, rectangles, pixels, frame w, h) {pa}, libjxl {pb}")
    lines = ["patches: size (bytes; lossy: PSNR dB) ours with / without patches | libjxl 0.7 default / without (same effort, "
             "distance); gain = with / without"]
    tot = {}
    for n, px in imgs:
        for mn, fl, q, e in modes:
            if fl & IOF_JXL_PROGRESSIVE:
                continue
            fa, fb = files.get((n, mn, 0)), files.get((n, mn, IOF_JXL_NOPATCHES))
            la, lb = ljf.get((n, mn, 1)), ljf.get((n, mn, 0))
            if not (fa and fb and la and lb):
                continue
            sz = [f.stat().st_size for f in (fa, fb, la, lb)]
            ps = ""
            if not fl & IOF_LOSSLESS:
                vals = [own.get((n, mn, 0)), own.get((n, mn, IOF_JXL_NOPATCHES)), (ljd.get((n, mn, 1)) or [None])[0],
                        (ljd.get((n, mn, 0)) or [None])[0]]
                ps = " (" + " / ".join("%.2f" % psnr(v, px) if v is not None else "-" for v in vals) + ")"
            lines.append("  %-12s %-7s %8d / %8d  %.3f | %8d / %8d  %.3f  ours/libjxl %.3f%s" % (
                n, mn, sz[0], sz[1], sz[0] / sz[1], sz[2], sz[3], sz[2] / sz[3], sz[0] / sz[2], ps))
            if n != "a6_photo":
                t = tot.setdefault(mn, [0, 0, 0, 0])
                for i in range(4):
                    t[i] += sz[i]
    for mn, t in tot.items():
        lines.append("  %-12s %-7s %8d / %8d  %.3f | %8d / %8d  %.3f  ours/libjxl %.3f" % (
            "total(text+ui)", mn, t[0], t[1], t[0] / t[1], t[2], t[3], t[2] / t[3], t[0] / t[2]))
    c.notes += lines


# ------------------------------------------------------------------ Butteraugli (jxl-tools)

def ba_rec(name, a, b, arg=0):
    """op 199: Butteraugli of b against a (RGBA8, alpha ignored), intensity target 80"""
    h, w = a.shape[:2]
    return it.Rec(OP_JXBA, arg, struct.pack("<II3f", w, h, 80.0, 1.0, 1.0) + a.tobytes() + b.tobytes(), None, name)


def ba_out(out, w, h):
    """op 199's record: (largest value, 3-norm, distance map, microseconds)"""
    b = bytes(out)
    sc, p3 = struct.unpack_from("<fd", b, 0)
    dm = np.frombuffer(b, np.float32, count=w * h, offset=12).reshape(h, w)
    us = struct.unpack_from("<Q", b, 12 + 4 * w * h)[0] if len(b) >= 20 + 4 * w * h else 0
    return sc, p3, dm, us


def ba_jobs(c, pairs, tag):
    """libjxl API jobs (RGB input: libjxl 0.7 takes another path for RGBA)"""
    jobs = []
    for k, (n, a, b) in enumerate(pairs):
        fa, fb = c.work / f"ba_{tag}{k}_a.rgb", c.work / f"ba_{tag}{k}_b.rgb"
        fa.write_bytes(np.ascontiguousarray(a[..., :3]).tobytes())
        fb.write_bytes(np.ascontiguousarray(b[..., :3]).tobytes())
        jobs.append({"op": "ba", "a": rel(fa), "b": rel(fb), "w": a.shape[1], "h": a.shape[0], "intensity": 80.0,
                     "ch": 3, "out": rel(c.work / f"ba_{tag}{k}.dm")})
    return jobs


def suite_butteraugli(c, s):
    opaque = lambda px: np.concatenate([px[..., :3], np.full(px.shape[:2] + (1,), 255, np.uint8)], -1)  # noqa: E731

    def coded(px, q, e=3):
        b = c.encode([enc_rec("ba", px, 0, q, e)])[0]
        d = c.decode([b])[0] if b else None
        return d if d is not None else px.copy()

    # ---- 1. the metric against libjxl's API
    pairs = []
    for L in (20, 128, 240):
        base = np.full((64, 64, 4), L, np.uint8)
        o = base.copy()
        o[..., :3] = L + 2
        pairs.append((f"uniform {L}+2", base, o))
        o = base.copy()
        o[..., 2] = L + 4
        pairs.append((f"uniform {L} B+4", base, o))
    y, x = np.mgrid[0:96, 0:96]
    base = np.full((96, 96, 4), 128, np.uint8)
    for sg in (12, 3):
        g = 30 * np.exp(-((x - 48) ** 2 + (y - 48) ** 2) / (2 * sg * sg))
        o = base.astype(int)
        o[..., :3] += np.round(g[..., None]).astype(int)
        pairs.append((f"bump s{sg}", base, np.clip(o, 0, 255).astype(np.uint8)))
    o = base.astype(int)
    o[..., 1] += ((x + y) % 2) * 8
    pairs.append(("checker G", base, o.astype(np.uint8)))
    a6 = opaque(fixture("a6_photo.png"))
    for q in ((90, 30) if c.quick else (90, 60, 30)):
        pairs.append((f"a6 q{q}", a6, coded(a6, q)))
    sm = synth("smooth", 300, 200)
    rng = np.random.default_rng(1)
    nz = sm.copy()
    nz[..., :3] = np.clip(sm[..., :3].astype(int) + rng.integers(-6, 7, sm[..., :3].shape), 0, 255).astype(np.uint8)
    pairs.append(("smooth+noise", sm, nz))
    tx = synth("text", 260, 150)
    pairs.append(("text q90 e7", tx, coded(tx, 90, 7)))
    pairs.append(("odd 37x23", synth("smooth", 37, 23), coded(synth("smooth", 37, 23), 40)))
    pairs.append(("same", a6, a6))
    pairs.append(("tiny 7x7", synth("smooth", 7, 7), coded(synth("smooth", 7, 7), 40)))
    pairs = [(n, opaque(a), opaque(b)) for n, a, b in pairs]
    own = c.R.batch([ba_rec(n, a, b) for n, a, b in pairs])
    own1 = c.R.batch([ba_rec(n, a, b, 1) for n, a, b in pairs])
    res, us1 = {}, {}
    for (n, a, b), r, r1 in zip(pairs, own, own1):
        h, w = a.shape[:2]
        if r is None or r[0] != 0 or r1 is None or r1[0] != 0:
            s.fail(f"{n}: op 199 failed")
            continue
        res[n] = ba_out(r[1], w, h)
        o1 = ba_out(r1[1], w, h)
        us1[n] = o1[3]
        s.check(np.array_equal(res[n][2], o1[2]), f"{n}: the distance map depends on the threads")
    if "same" in res:
        s.check(res["same"][0] == 0 and not res["same"][2].any(), "identical images: distance not 0")
    if "tiny 7x7" in res:
        s.check(res["tiny 7x7"][0] == 0, "below 8x8: distance not 0 (as libjxl)")
    lines = ["butteraugli: ours / libjxl 0.7 (JxlButteraugliCompute, RGB, intensity 80): largest value, 3-norm, "
             "max |distance map difference| / largest; ms of one comparison (ours: pool / 1 thread | libjxl)"]
    if c.ext:
        jobs = ba_jobs(c, pairs, "m")
        for (n, a, b), j, r in zip(pairs, jobs, c.lj.run(jobs)):
            if n not in res:
                continue
            if not r.get("ok"):
                s.fail(f"{n}: libjxl failed ({r.get('err')})")
                continue
            h, w = a.shape[:2]
            sc, p3, dm, us = res[n]
            ldm = np.frombuffer((ROOT / j["out"]).read_bytes(), np.float32).reshape(h, w)
            rel_dm = float(np.abs(dm - ldm).max()) / max(1e-9, float(ldm.max()))
            close = lambda u, v: abs(u - v) <= 1e-3 * max(abs(v), 1e-6) + 1e-7  # noqa: E731
            s.check(close(sc, r["max"]) and close(p3, r["d3"]) and rel_dm <= 1e-3,
                    f"{n}: ours {sc:.6f} / {p3:.6f}, libjxl {r['max']:.6f} / {r['d3']:.6f}, distance map {rel_dm:.2e}")
            lines.append("  %-14s %4dx%-4d %9.5f / %9.5f  %9.5f / %9.5f  %.1e  %6.1f / %6.1f | %6.1f" % (
                n, w, h, sc, r["max"], p3, r["d3"], rel_dm, us / 1000, us1[n] / 1000, r["ms"]))
    # ---- 2. the effort 8 / 9 quantization loop
    shots = screenshots(c)
    imgs = [("a6_photo", fixture("a6_photo.png")), ("text520x300", synth("text", 520, 300))] + shots[:2]
    if not c.quick:
        grain = synth("photo", 400, 300, seed=9).astype(int)
        grain[..., :3] += np.random.default_rng(5).normal(0, 8, grain[..., :3].shape).round().astype(int)
        imgs += [("photo800x500", synth("photo", 800, 500)), ("smooth300x200", synth("smooth", 300, 200)),
                 ("grain400x300", np.clip(grain, 0, 255).astype(np.uint8))]
    imgs = [(n, opaque(px)) for n, px in imgs]
    quals = [90] if c.quick else [90, 70]
    cases = [(n, px, q, e) for n, px in imgs for q in quals for e in (7, 8, 9)]
    tm = c.R.batch([enc_rec(f"{n} q{q} e{e}", px, 0, q, e, op=OP_JXTIME, arg=1) for n, px, q, e in cases])
    blobs = c.encode([enc_rec(f"{n} q{q} e{e}", px, 0, q, e) for n, px, q, e in cases])
    files, ms = {}, {}
    for (n, px, q, e), b, t in zip(cases, blobs, tm):
        s.check(b is not None, f"{n} q{q} e{e}: encode failed")
        if b:
            files[(n, q, e)] = c.save(f"bq_{n}_q{q}_e{e}", b)
        ms[(n, q, e)] = struct.unpack_from("<IQ", t[1], 0)[1] / 1000 if t is not None and t[0] == 0 else 0
    keys = list(files)
    pxs = dict(imgs)
    own = dict(zip(keys, c.decode([files[k].read_bytes() for k in keys])))
    for k in keys:
        s.check(own[k] is not None and own[k].shape == pxs[k[0]].shape, "%s q%d e%d: own decode failed" % k)
    for n, _ in imgs:
        for q in quals:
            a, b = files.get((n, q, 7)), files.get((n, q, 8))
            s.check(a is not None and b is not None and a.read_bytes() != b.read_bytes(),
                    f"{n} q{q}: effort 8 gives effort 7's file")
    okk = [k for k in keys if own[k] is not None]
    bo = dict(zip(okk, c.R.batch([ba_rec("q", pxs[k[0]], opaque(own[k])) for k in okk])))
    bmax = {k: ba_out(r[1], pxs[k[0]].shape[1], pxs[k[0]].shape[0])[:2] for k, r in bo.items() if r is not None and r[0] == 0}
    for q in quals:
        for e in (8, 9):
            m7 = [bmax[(n, q, 7)][0] for n, _ in imgs if (n, q, 7) in bmax and (n, q, e) in bmax]
            me = [bmax[(n, q, e)][0] for n, _ in imgs if (n, q, 7) in bmax and (n, q, e) in bmax]
            s.check(bool(m7) and sum(me) < sum(m7),
                    f"q{q} e{e}: largest distance {sum(me):.3f} not below effort 7's {sum(m7):.3f}")
    if not c.ext:
        c.notes += lines
        return
    lres = dict(zip(keys, c.lj.decode([files[k] for k in keys])))
    for k in okk:
        r = lres[k]
        s.check(r is not None and r[0] is not None and r[0].shape == own[k].shape
                and int(np.abs(r[0].astype(int) - own[k].astype(int)).max()) <= 1, "%s q%d e%d: libjxl differs" % k)
    jobs = []
    for n, px, q, e in cases:
        raw = c.work / f"src3_{n}.rgb"
        raw.write_bytes(np.ascontiguousarray(px[..., :3]).tobytes())
        jobs.append({"op": "enc", "in": rel(raw), "out": rel(c.work / f"ljbq_{n}_q{q}_e{e}.jxl"), "w": px.shape[1],
                     "h": px.shape[0], "ch": 3, "effort": e, "distance": dist(q), "threads": 8})
    lr = c.lj.run(jobs)
    lfiles = [ROOT / j["out"] for j, r in zip(jobs, lr) if r.get("ok")]
    ld = iter(c.lj.decode(lfiles))
    pairs2, idx = [], {}
    for (n, px, q, e), r in zip(cases, lr):
        if not r.get("ok"):
            s.fail(f"libjxl {n} q{q} e{e}: {r.get('err')}")
            continue
        d = next(ld)
        if d is None or d[0] is None:
            continue
        idx[(n, q, e, "l")] = len(pairs2)
        pairs2.append((n, px, opaque(d[0])))
        if own.get((n, q, e)) is not None:
            idx[(n, q, e, "o")] = len(pairs2)
            pairs2.append((n, px, opaque(own[(n, q, e)])))
    jb = ba_jobs(c, pairs2, "q")
    br = c.lj.run(jb)
    lines.append("butteraugli loop (efforts 8 / 9): bytes, Butteraugli largest / 3-norm (libjxl's API), encode ms (pool) "
                 "| libjxl 0.7 same effort / distance (8 threads) | size ratio")
    tot = {}
    for (n, px, q, e), r in zip(cases, lr):
        io, il = idx.get((n, q, e, "o")), idx.get((n, q, e, "l"))
        if io is None or il is None or not br[io].get("ok") or not br[il].get("ok"):
            continue
        so, sl = files[(n, q, e)].stat().st_size, r["size"]
        lines.append("  %-14s q%d e%d %8d %6.3f %6.3f %7.0f | %8d %6.3f %6.3f %7.0f | %.3f" % (
            n, q, e, so, br[io]["max"], br[io]["d3"], ms[(n, q, e)], sl, br[il]["max"], br[il]["d3"], r["ms"], so / sl))
        t = tot.setdefault((q, e), [0, 0, 0.0, 0.0, 0.0, 0.0, 0])
        t[0] += so
        t[1] += sl
        t[2] += br[io]["max"]
        t[3] += br[il]["max"]
        t[4] += ms[(n, q, e)]
        t[5] += r["ms"]
        t[6] += 1
    for (q, e), t in sorted(tot.items()):
        lines.append("  %-14s q%d e%d %8d %6.3f        %7.0f | %8d %6.3f        %7.0f | %.3f  (mean largest distance)" % (
            "total", q, e, t[0], t[2] / t[6], t[4], t[1], t[3] / t[6], t[5], t[0] / t[1]))
    c.notes += lines


def suite_speed(c, s):
    W, H = (1024, 768) if c.quick else (2048, 1536)
    px = synth("photo", W, H)
    modes = [("lossless e3", IOF_LOSSLESS, 0, 3), ("lossless e7", IOF_LOSSLESS, 0, 7),
             ("lossless e7 prog", IOF_LOSSLESS | IOF_JXL_PROGRESSIVE, 0, 7), ("q90 e3", 0, 90, 3), ("q90 e5", 0, 90, 5),
             ("q90 e7", 0, 90, 7), ("q90 e7 prog", IOF_JXL_PROGRESSIVE, 90, 7), ("q90 e8", 0, 90, 8), ("q90 e9", 0, 90, 9)]
    recs = [enc_rec(n, px, fl, q, e, op=OP_JXTIME, arg=2) for n, fl, q, e in modes]
    t = {}
    for lab, R in (("pool", c.R), ("1thr", c.R1)):
        for (n, *_), r in zip(modes, R.batch(recs)):
            t[(lab, n)] = struct.unpack_from("<IQ", r[1], 0) if r is not None and r[0] == 0 else (0, 0)
    blobs = c.encode([enc_rec(n, px, fl, q, e) for n, fl, q, e in modes])
    dts = {}
    for lab, R in (("pool", c.R), ("1thr", c.R1)):
        dr = R.batch([it.Rec(OP_JXDTIME, 3, b or b"", None, n) for (n, *_), b in zip(modes, blobs)])
        for (n, *_), r in zip(modes, dr):
            dts[(lab, n)] = struct.unpack_from("<Q", r[1], 0)[0] if r is not None and r[0] == 0 else 0
    lj = {}
    if c.ext:
        raw = c.work / "speed.rgba"
        raw.write_bytes(px[..., :3].tobytes())
        jobs = []
        for n, fl, q, e in modes:
            o = ({16: 1} if fl & IOF_LOSSLESS else {17: 1, 19: 1}) if fl & IOF_JXL_PROGRESSIVE else {}
            for th in (0, 8):
                jobs.append({"op": "enc", "in": rel(raw), "out": rel(c.work / f"spd_{len(jobs)}.jxl"), "w": W, "h": H,
                             "ch": 3, "effort": e, "distance": 0 if fl & IOF_LOSSLESS else dist(q), "opts": o, "threads": th})
        lr = c.lj.run(jobs)
        dj = [{"op": "dec", "in": j["out"], "out": j["out"] + ".lj", "threads": j["threads"]} for j in jobs]
        dl = c.lj.run(dj)
        for i, (n, *_) in enumerate(modes):
            lj[n] = (lr[2 * i], lr[2 * i + 1], dl[2 * i], dl[2 * i + 1])
    mp = W * H / 1e6
    lines = [f"speed, {W}x{H} photo ({mp:.1f} MP), ms (best of 2 / 3; machine load matters):",
             "  %-17s %9s %8s %8s %7s %7s | %9s %8s %8s %7s %7s" % ("mode", "bytes", "enc", "enc 1t", "dec", "dec 1t",
                                                                   "libjxl", "enc 1t", "enc 8t", "dec 1t", "dec 8t")]
    for n, *_ in modes:
        sz, us = t[("pool", n)]
        _, us1 = t[("1thr", n)]
        l = lj.get(n)
        f = lambda r, k: "%.0f" % r[k] if r and r.get("ok") and k in r else "-"  # noqa: E731
        lines.append("  %-17s %9d %8.0f %8.0f %7.0f %7.0f | %9s %8s %8s %7s %7s" % (
            n, sz, us / 1000, us1 / 1000, dts[("pool", n)] / 1000, dts[("1thr", n)] / 1000,
            l[0].get("size", "-") if l else "-", f(l[0], "ms") if l else "-", f(l[1], "ms") if l else "-",
            f(l[2], "ms") if l else "-", f(l[3], "ms") if l else "-"))
    c.notes += lines
    s.ok()


def main():
    ap = it.argparser(__doc__, SUITES)
    ap.set_defaults(out="build")
    ap.add_argument("--quick", action="store_true", help="fewer images, efforts and qualities")
    a = ap.parse_args()
    only = [x for x in a.only.split(",") if x] or SUITES.split(",")
    out = (ROOT / a.out).resolve()
    suites = []
    for target in it.targets_of(a):
        c = Ctx(target, out, a)
        for name in only:
            s = it.Suite(name, target)
            t0 = time.time()
            globals()["suite_" + name](c, s)
            s.dt = time.time() - t0
            suites.append(s)
        for n in c.notes:
            print(f"[{target}] {n}")
    sys.exit(0 if it.report(suites, "jxl_plus_check") else 1)


if __name__ == "__main__":
    main()
