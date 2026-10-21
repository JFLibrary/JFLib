#!/usr/bin/env python3
"""Shared infrastructure for the image codec checkers (tools/<codec>_check.py).

Import it from a checker in tools/:

    import imgtest_common as it
    ap = it.argparser(__doc__)                    # --target --out --no-build --only --seed ...
    a = ap.parse_args(); out = it.out_dir(a)
    if not a.no_build: it.build_codectest(targets, out)
    runner = it.Runner("win", out)                # also "linux" (runs through WSL)
    suite = it.Suite("tiff")
    recs = [it.Rec(it.OP_DECEX, 0, it.decex_input(data, flags=it.C.IMGD_STRAIGHT),
                   it.expect_decex(ref_bgra), "name")]
    it.run_suite(runner, suite, recs); it.report([...])

Contents
  constants   C.<NAME> for every `NAME = value` in jfgui.inc and img.inc (ERR_*, IMGF_*,
              FMT_*, IMGD_*, IM_*, IO_*, IOF_*, IFC_*, IOM_*, ...); ERR_NAMES; codectest op
              numbers OP_* (ops 240..259 from meta_ops.s) and the op range of every codec
              (OP_BASE["tiff"] = 60, ...)
  batch       Rec, Runner (codectest --batch on win / linux, crashing records isolated by
              bisection, extra program args e.g. ["--core-only"]), Suite, run_suite, report,
              build_codectest, argparser, out_dir
  records     decex_input / parse_decex (op 240), parse_meta_rec (80-byte meta record),
              surface_input / parse_surface (w, h, fmt + packed pixels), expect_* checkers
  pixels      premul / unpremul (exact library formulas), orient (EXIF 1..8), psnr,
              max_abs, mean_abs, plane_stats (per channel), compare(a, b, max_abs, psnr_min),
              rand_bgra; checkers expect_err / expect_bytes / expect_decex / expect_any
  references  ffmpeg_decode / ffmpeg_encode / ffprobe_size (Windows ffmpeg on PATH),
              Wic.decode / Wic.encode (PowerShell + WPF: PNG, JPEG, TIFF, BMP, GIF, JPEG XR
              via WmpBitmapEncoder/-Decoder, whatever WIC codecs are installed; one
              PowerShell process per batch), wsl_python(code, stdin) (python3 in WSL, e.g.
              ctypes on libwebp / libjxl / libheif / libdav1d)
  containers  riff / riff_chunk, bmff_box / bmff_fullbox / ftyp, tiff_bytes (classic or
              BigTIFF, several IFDs, sub-IFD references), exif_blob, jpeg_seg /
              jpeg_insert, png_chunk / png_insert / png_make (RGBA8 / RGB8 / grey)
  mutation    mutate(rng, data) -> truncations, bit flips, byte runs, garbage

Everything is stdlib + numpy.  Inputs are generated; nothing is downloaded.
"""

import argparse
import os
import random
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import zlib
from pathlib import Path

import numpy as np

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent.parent / "legacy"))  # R13: build.py is in legacy/
import build  # noqa: E402  (build.py: exe paths, WSL command lines)
import testenv  # noqa: E402


# ================================================================== constants from the includes

def _load_constants():
    names = {}
    pat = re.compile(r"^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*([^#]+?)\s*(?:#.*)?$")
    for inc in ("jfgui.inc", "img.inc"):
        p = ROOT.parent / "legacy" / "ASMLIB" / "inc" / inc
        if not p.exists():
            continue
        for line in p.read_text(encoding="utf-8").splitlines():
            m = pat.match(line)
            if not m:
                continue
            expr = m.group(2).replace("~", " ~")
            try:
                names[m.group(1)] = int(eval(expr, {"__builtins__": {}}, dict(names)))  # noqa: S307
            except Exception:  # noqa: BLE001 -- not a plain integer expression
                pass
    return names


class _C:
    def __init__(self, d):
        self.__dict__.update(d)

    def __getitem__(self, k):
        return self.__dict__[k]


C = _C(_load_constants())
ERR_NAMES = {v: k[4:] for k, v in C.__dict__.items() if k.startswith("ERR_")}
ERR_NAMES[0] = "OK"
ERR_NAMES[-1] = "TRUNC"
ERR_NAMES[-100] = "UNKNOWN-OP/BAD-RECORD"
IMGF_NAMES = {v: k[5:] for k, v in C.__dict__.items() if k.startswith("IMGF_") and k != "IMGF_COUNT"}


def err_name(r):
    return ERR_NAMES.get(r, str(r))


# codectest op numbers (apps/codectest/src/meta_ops.s)
OP_DECEX, OP_PROBE, OP_FMTINFO, OP_FROMPATH, OP_GZDEC, OP_GZENC, OP_DEFRAW = range(240, 247)
OP_EXIFREAD, OP_EXIF1, OP_ICCCOL, OP_ICCSEG, OP_ICCCHK, OP_ENCODE, OP_CONVERT = range(247, 254)
OP_ROWCONV, OP_ORIENT, OP_GETROW, OP_INFRAW, OP_USEALL, OP_FILTER = range(254, 260)
OP_IMGDEC = 10                      # classic img_decode (main.s)
OP_BASE = {"jpeg": 20, "gif": 40, "tiff": 60, "tga": 80, "dds": 100, "webp": 120, "avif": 140,
           "heic": 160, "jxl": 180, "jxr": 200, "pdn": 220, "meta": 240, "png": 260, "bmp": 280}


# ================================================================== batch runner

class Rec:
    """One codectest batch record: op, arg (u32), data, check(res, out) -> None / message."""
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
    """Runs codectest --batch on one target.  `extra` = additional program arguments
    (e.g. ["--core-only"] to leave the non-core codecs unregistered)."""

    def __init__(self, target, out, extra=(), work_name="imgtest"):
        self.target = target
        self.out = Path(out)
        self.exe = build.exe_path("codectest", target, self.out)
        self.work = self.out / target / work_name
        self.work.mkdir(parents=True, exist_ok=True)
        self.extra = list(extra)
        self.seq = 0

    @staticmethod
    def rel(p):
        return str(Path(p).resolve().relative_to(ROOT)).replace("\\", "/")

    def cmd(self, args):
        dummy = self.rel(self.work / "unused.png")
        return build.run_cmd_for(self.exe, self.target, ["--shot", dummy] + self.extra + list(args))

    def run(self, args, timeout=900):
        return subprocess.run(self.cmd(args), cwd=ROOT, capture_output=True, text=True,
                              encoding="utf-8", errors="replace", timeout=timeout)

    def _batch_once(self, recs):
        self.seq += 1
        tag = f"{os.getpid()}_{self.seq}"
        bin_ = self.work / f"batch{tag}.bin"
        res = self.work / f"batch{tag}.res"
        bin_.write_bytes(encode_batch(recs))
        if res.exists():
            res.unlink()
        try:
            p = self.run(["--batch", self.rel(bin_), "--result", self.rel(res)])
        except subprocess.TimeoutExpired as e:
            p = subprocess.CompletedProcess(e.cmd, -999, "", "timeout")
        ok = p.returncode == 0 and res.exists()
        out = decode_results(res.read_bytes(), len(recs)) if ok else None
        bin_.unlink()
        if res.exists():
            res.unlink()
        return out, p

    def batch(self, recs, crashes=None):
        """Run records; returns a list of (result, bytes), None for a record that crashed."""
        if crashes is None:
            crashes = []
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

    def one(self, op, arg, data):
        """Run a single record, return (result, bytes) (raises on a crash)."""
        r = self.batch([Rec(op, arg, data, None, "one")])[0]
        if r is None:
            raise RuntimeError(f"codectest crashed on op {op}")
        return r


class Suite:
    def __init__(self, name, target=""):
        self.name, self.target = name, target
        self.passed, self.failed, self.msgs, self.notes, self.dt = 0, 0, [], [], 0.0

    def ok(self):
        self.passed += 1

    def fail(self, msg):
        self.failed += 1
        if len(self.msgs) < 30:
            self.msgs.append(msg)

    def check(self, cond, msg):
        if cond:
            self.ok()
        else:
            self.fail(msg)


def run_suite(runner, suite, recs):
    """Run recs, apply their checks; crashes count as failures."""
    crashes = []
    t0 = time.time()
    results = runner.batch(recs, crashes)
    for r, res in zip(recs, results):
        if res is None:
            suite.fail(f"CRASH {r.name}")
            continue
        if r.check is None:
            suite.ok()
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
    suite.dt += time.time() - t0
    return results


def report(suites, title):
    ok = True
    for s in suites:
        status = "PASS" if s.failed == 0 else "FAIL"
        print(f"[{s.target}] {status} {s.name:10s} {s.passed:6d} passed {s.failed:5d} failed  ({s.dt:.1f}s)")
        for n in s.notes:
            print("        " + n)
        for m in s.msgs:
            print("        ! " + m)
        ok &= s.failed == 0
    print(f"{title}:", "ALL PASSED" if ok else "FAILURES")
    return ok


def argparser(doc, suites=""):
    ap = argparse.ArgumentParser(description=doc, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--out", default=os.environ.get("jfgui_BUILD_DIR", "build"),
                    help="build directory (default: build, or $jfgui_BUILD_DIR)")
    ap.add_argument("--no-build", action="store_true", help="use the existing codectest binaries")
    ap.add_argument("--only", default="", help="comma list of suites" + (f" ({suites})" if suites else ""))
    ap.add_argument("--fuzz", type=int, default=2000, help="fuzz cases")
    ap.add_argument("--seed", type=int, default=1234)
    ap.add_argument("--no-ext", action="store_true", help="skip external references (ffmpeg, WIC, WSL)")
    return ap


def out_dir(a):
    return (ROOT / a.out).resolve()


def targets_of(a):
    return ["win", "linux"] if a.target == "both" else [a.target]


def build_codectest(targets, out):
    for t in targets:
        subprocess.run(testenv.build_cmd("codectest", t, out), cwd=ROOT, check=True)   # Rust; asm for rwref


# ================================================================== record formats

MR_FIELDS = ("fmt", "w", "h", "bits", "model", "orient", "channels", "flags", "frames", "dpix",
             "dpiy", "cicp", "gamma", "icclen", "exiflen", "xmplen", "delay", "loops", "quality",
             "resunit")


def parse_meta_rec(b):
    """80-byte meta record (meta_ops.s) -> dict."""
    v = struct.unpack_from("<III4BIIIIIIQQQIIII", b, 0)
    return dict(zip(MR_FIELDS, v))


def decex_input(data, flags=0, hint=0, frame=0, usemeta=1, maxpx=0, cancel=0):
    """Op 240 record data."""
    return struct.pack("<IIIIQII", flags, hint, frame, usemeta, maxpx, cancel, 0) + bytes(data)


def parse_decex(out):
    """Op 240 result bytes -> dict(w, h, fmt, sflags, meta, icc, exif, xmp, px (h, w, 4) uint8 or None)."""
    w, h, f = struct.unpack_from("<III", out, 0)
    meta = parse_meta_rec(out[12:92])
    p = 92
    blobs = []
    for k in ("icclen", "exiflen", "xmplen"):
        n = meta[k]
        blobs.append(bytes(out[p:p + n]))
        p += n
    px = None
    if w and h:
        px = np.frombuffer(out, np.uint8, count=w * h * 4, offset=p).reshape(h, w, 4)
        p += w * h * 4
    if p != len(out):
        raise ValueError(f"op 240 result: {len(out)} bytes, parsed {p}")
    return dict(w=w, h=h, fmt=f & 0xFF, sflags=f >> 8, meta=meta, icc=blobs[0], exif=blobs[1],
                xmp=blobs[2], px=px)


def surface_input(px, fmt=None):
    """numpy (h, w, 4) uint8 BGRA (or (h, w) for FMT_A8) -> {u32 w, h, fmt, packed pixels}."""
    px = np.ascontiguousarray(px, np.uint8)
    if fmt is None:
        fmt = C.FMT_A8 if px.ndim == 2 else C.FMT_PARGB
    h, w = px.shape[:2]
    return struct.pack("<III", w, h, fmt) + px.tobytes()


def parse_surface(out):
    """ct_surface_out record -> (w, h, fmt, flags, px (h, w, 4) or (h, w))."""
    w, h, f = struct.unpack_from("<III", out, 0)
    fmt = f & 0xFF
    if fmt == C.FMT_A8:
        px = np.frombuffer(out, np.uint8, count=w * h, offset=12).reshape(h, w)
    else:
        px = np.frombuffer(out, np.uint8, count=w * h * 4, offset=12).reshape(h, w, 4)
    return w, h, fmt, f >> 8, px


def expect_err(*codes):
    def chk(res, out):
        if res not in codes:
            return f"result {err_name(res)}, expected {'/'.join(err_name(c) for c in codes)}"
        return None
    return chk


def expect_bytes(exp):
    exp = bytes(exp)

    def chk(res, out):
        if res < 0:
            return f"error {err_name(res)}"
        if bytes(out) != exp:
            n = min(len(out), len(exp))
            d = next((i for i in range(n) if out[i] != exp[i]), n)
            return f"bytes differ ({len(out)} vs {len(exp)} expected, first difference at {d})"
        return None
    return chk


def expect_decex(exp_px, fmt=None, tol=0, meta=None):
    """Op 240 check: pixels (h, w, 4) BGRA equal to exp_px within tol; optional result
    fmt and meta field subset."""
    def chk(res, out):
        if res != 0:
            return f"error {err_name(res)}"
        d = parse_decex(out)
        if fmt is not None and d["fmt"] != fmt:
            return f"fmt {d['fmt']} != {fmt}"
        if meta:
            for k, v in meta.items():
                if d["meta"][k] != v:
                    return f"meta {k} = {d['meta'][k]}, expected {v}"
        if exp_px is not None:
            e = np.asarray(exp_px, np.uint8)
            if d["px"] is None or d["px"].shape != e.shape:
                return f"size {None if d['px'] is None else d['px'].shape} != {e.shape}"
            m = max_abs(d["px"], e)
            if m > tol:
                idx = np.argwhere(np.abs(d["px"].astype(int) - e.astype(int)).max(-1) > tol)[0]
                y, x = idx
                return f"max |d| {m} > {tol} at ({x},{y}): got {list(d['px'][y, x])} exp {list(e[y, x])}"
        return None
    return chk


def any_result(res, out):
    return None


def expect_any(*checks):
    """Passes when one of the checks passes (e.g. an interim and the final exact result)."""
    def chk(res, out):
        msgs = [c(res, out) for c in checks]
        return None if any(m is None for m in msgs) else " | ".join(msgs)
    return chk


# ================================================================== pixel helpers

def premul(bgra):
    """Straight BGRA -> premultiplied, c * a / 255 rounded (library rounding)."""
    a = bgra[..., 3:4].astype(np.int64)
    c = (bgra[..., :3].astype(np.int64) * a + 127) // 255
    return np.concatenate([c, a], -1).astype(np.uint8)


_RCP = np.array([0] + [(255 * (1 << 24) + a - 1) // a for a in range(1, 256)], np.int64)


def unpremul(pbgra):
    """Premultiplied BGRA -> straight exactly like png.s unpremul_row / imgmeta.s:
    min(255, (c * ceil(255 * 2^24 / a) + 2^23) >> 24); a = 0 -> all zero."""
    p = pbgra.astype(np.int64)
    a = p[..., 3:4]
    c = np.minimum(255, (p[..., :3] * _RCP[a] + (1 << 23)) >> 24)
    out = np.concatenate([c, a], -1)
    out[(a[..., 0] == 0)] = 0
    return out.astype(np.uint8)


def orient(img, o):
    """Apply EXIF orientation o (1..8) to an (h, w, ...) array for display."""
    if o == 2:
        return img[:, ::-1]
    if o == 3:
        return img[::-1, ::-1]
    if o == 4:
        return img[::-1]
    if o == 5:
        return img.swapaxes(0, 1)
    if o == 6:
        return img[::-1].swapaxes(0, 1)
    if o == 7:
        return img[::-1, ::-1].swapaxes(0, 1)
    if o == 8:
        return img[:, ::-1].swapaxes(0, 1)
    return img


def max_abs(a, b):
    return int(np.abs(np.asarray(a, np.int64) - np.asarray(b, np.int64)).max()) if np.size(a) else 0


def mean_abs(a, b):
    return float(np.abs(np.asarray(a, np.float64) - np.asarray(b, np.float64)).mean()) if np.size(a) else 0.0


def psnr(a, b, peak=255.0):
    mse = float(((np.asarray(a, np.float64) - np.asarray(b, np.float64)) ** 2).mean())
    return 99.0 if mse == 0 else 10 * np.log10(peak * peak / mse)


def plane_stats(a, b, names="BGRA"):
    """Per-channel comparison of two (h, w, c) arrays -> {name: (max |d|, mean |d|, PSNR)}."""
    a, b = np.asarray(a, np.float64), np.asarray(b, np.float64)
    out = {}
    for i in range(a.shape[-1]):
        d = np.abs(a[..., i] - b[..., i])
        mse = float((d * d).mean()) if d.size else 0.0
        out[names[i] if i < len(names) else str(i)] = (
            int(d.max()) if d.size else 0, float(d.mean()) if d.size else 0.0,
            99.0 if mse == 0 else 10 * np.log10(255.0 * 255.0 / mse))
    return out


def compare(a, b, max_tol=None, psnr_min=None):
    """None when a and b agree within the limits, else a message."""
    a, b = np.asarray(a), np.asarray(b)
    if a.shape != b.shape:
        return f"shape {a.shape} != {b.shape}"
    if max_tol is not None and max_abs(a, b) > max_tol:
        return f"max |d| {max_abs(a, b)} > {max_tol}"
    if psnr_min is not None and psnr(a, b) < psnr_min:
        return f"PSNR {psnr(a, b):.2f} < {psnr_min}"
    return None


def rand_bgra(rng, w, h, alpha="mixed"):
    """Random straight BGRA test image: alpha 'opaque', 'mixed' (0 / 255 / random) or 'random'."""
    r = np.random.default_rng(rng.randrange(1 << 30))
    px = r.integers(0, 256, (h, w, 4), dtype=np.uint8)
    if alpha == "opaque":
        px[..., 3] = 255
    elif alpha == "mixed":
        sel = r.integers(0, 3, (h, w))
        px[..., 3] = np.where(sel == 0, 0, np.where(sel == 1, 255, px[..., 3]))
    return px


# ================================================================== external references

def have_exe(name):
    return shutil.which(name) is not None


def ffprobe_size(path):
    p = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries",
                        "stream=width,height", "-of", "csv=p=0", str(path)],
                       capture_output=True, text=True, timeout=120)
    if p.returncode != 0 or not p.stdout.strip():
        return None
    w, h = p.stdout.strip().splitlines()[0].split(",")[:2]
    return int(w), int(h)


def ffmpeg_decode(path, pix_fmt="bgra", extra=()):
    """Decode an image file with ffmpeg -> numpy (h, w, 4) (bgra) or None."""
    size = ffprobe_size(path)
    if size is None:
        return None
    w, h = size
    p = subprocess.run(["ffmpeg", "-v", "error", "-i", str(path), *extra, "-frames:v", "1",
                        "-f", "rawvideo", "-pix_fmt", pix_fmt, "-"], capture_output=True, timeout=300)
    ch = {"bgra": 4, "rgba": 4, "rgb24": 3, "bgr24": 3, "gray": 1}[pix_fmt]
    if p.returncode != 0 or len(p.stdout) < w * h * ch:
        return None
    return np.frombuffer(p.stdout, np.uint8, count=w * h * ch).reshape(h, w, ch) if ch > 1 else \
        np.frombuffer(p.stdout, np.uint8, count=w * h).reshape(h, w)


def ffmpeg_encode(bgra, out_path, args, in_pix="bgra"):
    """Encode a (h, w, 4) BGRA array with ffmpeg: ffmpeg -f rawvideo ... <args> out_path.
    Returns True on success."""
    h, w = bgra.shape[:2]
    p = subprocess.run(["ffmpeg", "-y", "-v", "error", "-f", "rawvideo", "-pix_fmt", in_pix,
                        "-s", f"{w}x{h}", "-i", "-", *args, str(out_path)],
                       input=np.ascontiguousarray(bgra).tobytes(), capture_output=True, timeout=600)
    return p.returncode == 0 and Path(out_path).exists()


_WIC_PS = r"""
param([string]$jobs)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName PresentationCore
Add-Type -AssemblyName WindowsBase
foreach ($line in [IO.File]::ReadAllLines($jobs)) {
  $a = $line.Split("`t")
  try {
    if ($a[0] -eq 'decode') {
      $bytes = [IO.File]::ReadAllBytes($a[1])
      $ms = New-Object IO.MemoryStream(,$bytes)
      $opt = [System.Windows.Media.Imaging.BitmapCreateOptions]::PreservePixelFormat -bor [System.Windows.Media.Imaging.BitmapCreateOptions]::IgnoreColorProfile
      $dec = [System.Windows.Media.Imaging.BitmapDecoder]::Create($ms, $opt, [System.Windows.Media.Imaging.BitmapCacheOption]::OnLoad)
      $fi = [int]$a[3]
      $f = $dec.Frames[$fi]
      $c = New-Object System.Windows.Media.Imaging.FormatConvertedBitmap($f, [System.Windows.Media.PixelFormats]::Bgra32, $null, 0)
      $w = $c.PixelWidth; $h = $c.PixelHeight; $st = $w * 4
      $buf = New-Object byte[] ($st * $h)
      $c.CopyPixels($buf, $st, 0)
      $hdr = [BitConverter]::GetBytes([int]$w) + [BitConverter]::GetBytes([int]$h) + [BitConverter]::GetBytes([int]$dec.Frames.Count)
      [IO.File]::WriteAllBytes($a[2], [byte[]]($hdr + $buf))
    } elseif ($a[0] -eq 'encode') {
      $w = [int]$a[3]; $h = [int]$a[4]; $st = $w * 4
      $raw = [IO.File]::ReadAllBytes($a[2])
      $src = [System.Windows.Media.Imaging.BitmapSource]::Create($w, $h, 96, 96, [System.Windows.Media.PixelFormats]::Bgra32, $null, $raw, $st)
      $enc = New-Object ("System.Windows.Media.Imaging." + $a[1])
      for ($i = 6; $i -lt $a.Length; $i++) {
        $kv = $a[$i].Split('=')
        $p = $enc.GetType().GetProperty($kv[0])
        $v = $kv[1]
        if ($p.PropertyType.IsEnum) { $v = [Enum]::Parse($p.PropertyType, $v) }
        else { $v = [Convert]::ChangeType($v, $p.PropertyType, [Globalization.CultureInfo]::InvariantCulture) }
        $p.SetValue($enc, $v, $null)
      }
      $enc.Frames.Add([System.Windows.Media.Imaging.BitmapFrame]::Create($src))
      $fs = [IO.File]::Create($a[5]); $enc.Save($fs); $fs.Close()
    }
  } catch {
    [IO.File]::WriteAllText(($a[2] + '.err'), $_.Exception.Message)
  }
}
"""


class Wic:
    """Batch WIC (Windows Imaging Component) access through PowerShell + WPF.

        wic = Wic(workdir)
        i1 = wic.decode(path)                             # queue jobs
        i2 = wic.encode(bgra, "WmpBitmapEncoder", out, ImageQualityLevel=0.9)
        wic.run()                                         # one PowerShell process
        wic.result(i1) -> (w, h, frames, px (h, w, 4) BGRA straight) or None
        wic.result(i2) -> True / False (file written)

    Decoding ignores embedded colour profiles; frame = page / frame index."""

    def __init__(self, work):
        self.work = Path(work)
        self.work.mkdir(parents=True, exist_ok=True)
        self.jobs, self.kinds, self.outs = [], [], []

    def decode(self, path, frame=0):
        i = len(self.jobs)
        out = self.work / f"wic_{os.getpid()}_{i}.raw"
        self.jobs.append(f"decode\t{Path(path).resolve()}\t{out.resolve()}\t{frame}")
        self.kinds.append("decode")
        self.outs.append(out)
        return i

    def encode(self, bgra, encoder, out_path, **props):
        i = len(self.jobs)
        h, w = bgra.shape[:2]
        raw = self.work / f"wic_{os.getpid()}_{i}.in"
        raw.write_bytes(np.ascontiguousarray(bgra, np.uint8).tobytes())
        extra = "".join(f"\t{k}={v}" for k, v in props.items())
        self.jobs.append(f"encode\t{encoder}\t{raw.resolve()}\t{w}\t{h}\t{Path(out_path).resolve()}{extra}")
        self.kinds.append("encode")
        self.outs.append(Path(out_path))
        return i

    def run(self, timeout=1800):
        if not self.jobs:
            return
        ps = self.work / "wic_jobs.ps1"
        ps.write_text(_WIC_PS, encoding="utf-8")
        jf = self.work / f"wic_{os.getpid()}.jobs"
        jf.write_text("\n".join(self.jobs) + "\n", encoding="utf-8")
        for o in self.outs:
            if o.exists():
                o.unlink()
        subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(ps),
                        str(jf)], capture_output=True, timeout=timeout)

    def result(self, i):
        o = self.outs[i]
        if self.kinds[i] == "encode":
            return o.exists()
        if not o.exists():
            return None
        b = o.read_bytes()
        w, h, n = struct.unpack_from("<iii", b, 0)
        px = np.frombuffer(b, np.uint8, count=w * h * 4, offset=12).reshape(h, w, 4)
        return w, h, n, px


def wsl_python(code, stdin=b"", timeout=600, distro=None):
    """Run python3 -c code inside WSL (cwd = the repository); returns CompletedProcess
    with bytes stdout / stderr.  Useful with ctypes for libwebp / libjxl / libheif / libdav1d:
        wsl_python("import ctypes,sys; lib=ctypes.CDLL('libwebp.so.7'); ...", data)"""
    d = distro or build.WSL_DISTRO
    return subprocess.run(["wsl.exe", "-d", d, "--cd", str(ROOT), "-e", "python3", "-c", code],
                          input=stdin, capture_output=True, timeout=timeout)


# ================================================================== containers

def riff_chunk(fourcc, data):
    data = bytes(data)
    return fourcc + struct.pack("<I", len(data)) + data + (b"\0" if len(data) & 1 else b"")


def riff(form, chunks):
    """RIFF file: 'RIFF' size form + chunks (each chunk: riff_chunk bytes)."""
    body = form + b"".join(chunks)
    return b"RIFF" + struct.pack("<I", len(body)) + body


def bmff_box(typ, payload=b"", large=False):
    payload = bytes(payload)
    if large:
        return struct.pack(">I", 1) + typ + struct.pack(">Q", 16 + len(payload)) + payload
    return struct.pack(">I", 8 + len(payload)) + typ + payload


def bmff_fullbox(typ, version, flags, payload=b""):
    return bmff_box(typ, struct.pack(">I", (version << 24) | flags) + bytes(payload))


def ftyp(major, minor=0, compat=()):
    return bmff_box(b"ftyp", major + struct.pack(">I", minor) + b"".join(compat))


def tiff_bytes(ifds, bo="<", big=False):
    """TIFF structure.  ifds: list of dicts {'entries': [(tag, type, values)], 'next': index or
    None}.  values: list of ints (RATIONAL / SRATIONAL: (num, den) pairs), bytes (BYTE / ASCII /
    UNDEFINED), or ('ifd', index) for a pointer to another IFD of the list.  IFD 0 is the first
    IFD.  Types: 1 BYTE 2 ASCII 3 SHORT 4 LONG 5 RATIONAL 7 UNDEFINED 13 IFD 16 LONG8 18 IFD8.
    big=True writes BigTIFF (II+ / MM+, 8-byte counts and offsets, 20-byte entries)."""
    tsz = {1: 1, 2: 1, 3: 2, 4: 4, 5: 8, 6: 1, 7: 1, 8: 2, 9: 4, 10: 8, 11: 4, 12: 8, 13: 4,
           16: 8, 17: 8, 18: 8}
    fmt = {1: "B", 2: "B", 3: "H", 4: "I", 6: "b", 7: "B", 8: "h", 9: "i", 11: "f", 12: "d",
           13: "I", 16: "Q", 17: "q", 18: "Q"}
    esz, inl, cnt_sz, nxt_sz, hdr_len = (20, 8, 8, 8, 16) if big else (12, 4, 2, 4, 8)
    ofmt = "Q" if big else "I"

    def is_ref(v):
        return isinstance(v, tuple) and len(v) == 2 and v[0] == "ifd"

    def enc(typ, v):
        if isinstance(v, (bytes, bytearray)):
            return bytes(v)
        if is_ref(v):
            return b"\0" * (8 if typ in (16, 18) else 4)
        out = b""
        for x in v:
            if typ in (5, 10):
                out += struct.pack(bo + ("II" if typ == 5 else "ii"), *x)
            else:
                out += struct.pack(bo + fmt[typ], x)
        return out

    offs, vals_all, pos = [], [], hdr_len
    for d in ifds:
        offs.append(pos)
        vals = [enc(t, v) for (_, t, v) in d["entries"]]
        vals_all.append(vals)
        extra = sum(len(b) + (len(b) & 1) for b in vals if len(b) > inl)
        pos += cnt_sz + len(vals) * esz + nxt_sz + extra
    out = bytearray(b"II" if bo == "<" else b"MM")
    first = offs[0] if offs else 0
    out += struct.pack(bo + "HHHQ", 43, 8, 0, first) if big else struct.pack(bo + "HI", 42, first)
    for i, d in enumerate(ifds):
        n = len(d["entries"])
        data_pos = offs[i] + cnt_sz + n * esz + nxt_sz
        ent, data = bytearray(), bytearray()
        for (tag, typ, v), b in zip(d["entries"], vals_all[i]):
            if is_ref(v):
                b = struct.pack(bo + ("Q" if typ in (16, 18) else "I"), offs[v[1]])
            ent += struct.pack(bo + "HH" + ofmt, tag, typ, len(b) // tsz[typ])
            if len(b) <= inl:
                ent += b + b"\0" * (inl - len(b))
            else:
                ent += struct.pack(bo + ofmt, data_pos + len(data))
                data += b + (b"\0" if len(b) & 1 else b"")
        nxt = d.get("next")
        out += struct.pack(bo + ("Q" if big else "H"), n) + ent
        out += struct.pack(bo + ofmt, offs[nxt] if nxt is not None else 0) + data
    return bytes(out)


def exif_blob(orient=None, xres=None, yres=None, unit=None, colorspace=None, bo="<", extra_ifd1=False,
              cycle=False):
    """EXIF TIFF structure with IFD0 (Orientation / X / YResolution / ResolutionUnit), an Exif
    IFD (ColorSpace) and optionally IFD1; cycle=True points IFD1's next back to IFD0."""
    e0 = [(0x010F, 2, b"jfgui\0")]
    if orient is not None:
        e0.append((0x0112, 3, [orient]))
    if xres is not None:
        e0.append((0x011A, 5, [xres]))
    if yres is not None:
        e0.append((0x011B, 5, [yres]))
    if unit is not None:
        e0.append((0x0128, 3, [unit]))
    ifds = [{"entries": e0, "next": None}]
    if colorspace is not None:
        e0.append((0x8769, 4, ("ifd", len(ifds))))
        ifds.append({"entries": [(0xA001, 3, [colorspace])], "next": None})
    if extra_ifd1 or cycle:
        ifds[0]["next"] = len(ifds)
        ifds.append({"entries": [(0x0103, 3, [6])], "next": 0 if cycle else None})
    e0.sort(key=lambda t: t[0])
    return tiff_bytes(ifds, bo)


def jpeg_seg(marker, payload):
    return bytes([0xFF, marker]) + struct.pack(">H", len(payload) + 2) + bytes(payload)


def jpeg_insert(jpeg, segs, after_app0=True):
    """Insert complete marker segments after SOI (and after a leading APP0 when present)."""
    pos = 2
    if after_app0 and jpeg[2:4] == b"\xff\xe0":
        pos = 4 + struct.unpack(">H", jpeg[4:6])[0]
    return jpeg[:pos] + b"".join(segs) + jpeg[pos:]


def png_chunk(typ, data):
    data = bytes(data)
    return struct.pack(">I", len(data)) + typ + data + struct.pack(">I", zlib.crc32(typ + data) & 0xFFFFFFFF)


def png_make(px, ctype=None, extra_chunks=(), after_idat=()):
    """PNG from a numpy array: (h, w) grey, (h, w, 3) RGB or (h, w, 4) RGBA (8 bit), filter 0."""
    px = np.ascontiguousarray(px, np.uint8)
    h, w = px.shape[:2]
    if ctype is None:
        ctype = 0 if px.ndim == 2 else (6 if px.shape[2] == 4 else 2)
    raw = b"".join(b"\0" + px[y].tobytes() for y in range(h))
    ihdr = struct.pack(">IIBBBBB", w, h, 8, ctype, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + png_chunk(b"IHDR", ihdr) + b"".join(extra_chunks) +
            png_chunk(b"IDAT", zlib.compress(raw, 6)) + b"".join(after_idat) + png_chunk(b"IEND", b""))


def mutate(rng, data, keep=0):
    """One random corruption of data (bytes): truncation, bit flips, byte runs, insertion or
    deletion; the first `keep` bytes stay intact."""
    b = bytearray(data)
    if not b:
        return bytes(b)
    k = rng.randrange(6)
    lo = min(keep, len(b) - 1)
    if k == 0:
        return bytes(b[:rng.randrange(lo, len(b))])
    if k == 1:
        for _ in range(rng.randrange(1, 9)):
            i = rng.randrange(lo, len(b))
            b[i] ^= 1 << rng.randrange(8)
    elif k == 2:
        i = rng.randrange(lo, len(b))
        v = rng.choice([0, 0xFF, 0x7F, 0x80, rng.randrange(256)])
        for j in range(i, min(len(b), i + rng.randrange(1, 16))):
            b[j] = v
    elif k == 3:
        i = rng.randrange(lo, len(b))
        b[i:i] = bytes(rng.randrange(256) for _ in range(rng.randrange(1, 32)))
    elif k == 4:
        i = rng.randrange(lo, len(b))
        del b[i:i + rng.randrange(1, 32)]
    else:
        i = rng.randrange(lo, len(b))
        b[i] = rng.randrange(256)
    return bytes(b)
