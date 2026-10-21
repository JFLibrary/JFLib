#!/usr/bin/env python3
"""Checks of the JPEG XR decoder (ASMLIB/src/img/jxr.s, jxr_ent.s, jxr_pct.s, jxr_color.s)
through apps/codectest.

Suites
  wic      WPF WmpBitmapEncoder files (seeded matrix: ImageQualityLevel 0.1 / 0.5 / 0.9 / 1.0,
           Lossless, QualityLevel, OverlapLevel 0..2, SubsamplingLevel 0..3, tile slices,
           FrequencyOrder, InterleavedAlpha, ImageDataDiscardLevel 0..3, AlphaQualityLevel,
           AlphaDataDiscardLevel, Rotation / flips; pixel formats Bgr24 Bgr32 Bgra32 Pbgra32
           Gray8 BlackWhite Gray16 Rgb48 Rgba64 Prgba64 Rgba128Float Prgba128Float
           Rgb128Float Gray32Float Cmyk32 Bgr565 Bgr555 Bgr101010; sizes 1x1 .. 1001x777)
           -> op 240 (straight BGRA) against WIC's Bgra32 conversion; premultiplied sources
           also as PARGB against WIC's native Pbgra32 bytes
  native   op 200 mode 3: the samples of the codestream's pixel format (before the 8-bit
           conversion) against WIC's native CopyPixels -- bit-exact for every format
  meta     IMGMETA (size, bits, model, channels, IMF_ALPHA / SRC_PREMUL / LOSSLESS / HDR /
           FLOAT, orientation, DPI), ICC / XMP / EXIF blobs (tags added to WIC files by this
           script), IMGD_PROBE
  errors   malformed containers and codestreams, ID_MAXPX, truncation -> errors / any result
  fuzz     mutated files (truncation, bit flips, runs, header and IFD fields): any result,
           no crash, no hang (both targets)
  perf     decode time of a 12 MP lossy Bgr24 file (op 201, single thread; target < 400 ms)

  python tools/jxr_check.py --target both --out build/cx
  python tools/jxr_check.py --target win --only wic,native --no-build

WIC files and WIC's decodes are cached in <out>/jxr_cache (key = the job), so reruns are fast.

8-bit conversion rules (exactly WIC unless noted): 8-bit samples as decoded, 16-bit RGB / alpha
v >> 8, BD5 / BD6 bit replication, BD1 0 / 255; 16-bit grey round(v * 255 / 65535) and 10-bit
round(v * 255 / 1023) (WIC goes through linear light: +-1), float / half / fixed point linear ->
sRGB with a 4096-entry table (WIC +-1); CMYK (255 - c) * (255 - k) / 255 like tiff.s and
jpeg (WIC converts colour-managed: compared with that formula on WIC's native CMYK samples);
premultiplied sources: 8 bit unpremultiplied like WIC (min(255, c * floor(255 * 65536 / a) >> 16)),
16 bit unpremultiplied at 16 bits (rounded) then >> 8 (WIC +-1 on ~0.2 % of the samples), float
c / a (a == 0 -> 0) then sRGB; PARGB output: colour clamped to alpha (8 bit) or premultiplied
again after the conversion.  WIC loses the interleaved alpha of some files (4:0:0 / 4:2:0 internal
formats, its alpha is 0 or far from the source): such cases compare colour only (listed as a
note); our alpha follows the codestream there (cross-checked with an independent decoder).
"""

import hashlib
import os
import random
import struct
import subprocess
import sys
import time
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import imgtest_common as it  # noqa: E402
import jxr_enc_ref as JREF  # noqa: E402

C = it.C
Rec = it.Rec
OP_JDUMP, OP_JTIME = 200, 201
OP_ENCMETA = 260                     # img_encode with an IMGMETA (meta_ops.s)
STRAIGHT = C.IMGD_STRAIGHT

# ================================================================== WIC (WPF) raw-format jobs

_PS = r"""
param([string]$jobs)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName PresentationCore
Add-Type -AssemblyName WindowsBase
foreach ($line in [IO.File]::ReadAllLines($jobs)) {
  if ($line.Length -eq 0) { continue }
  $a = $line.Split("`t")
  try {
    if ($a[0] -eq 'decode') {
      $bytes = [IO.File]::ReadAllBytes($a[1])
      $ms = New-Object IO.MemoryStream(,$bytes)
      $opt = [System.Windows.Media.Imaging.BitmapCreateOptions]::PreservePixelFormat -bor [System.Windows.Media.Imaging.BitmapCreateOptions]::IgnoreColorProfile
      $dec = [System.Windows.Media.Imaging.BitmapDecoder]::Create($ms, $opt, [System.Windows.Media.Imaging.BitmapCacheOption]::OnLoad)
      $f = $dec.Frames[0]
      $nat = $f.Format.ToString()
      $bpp = $f.Format.BitsPerPixel
      $w = $f.PixelWidth; $h = $f.PixelHeight
      $nst = [int][Math]::Floor(($w * $bpp + 7) / 8)
      $nbuf = New-Object byte[] ($nst * $h)
      $f.CopyPixels($nbuf, $nst, 0)
      $c = New-Object System.Windows.Media.Imaging.FormatConvertedBitmap($f, [System.Windows.Media.PixelFormats]::Bgra32, $null, 0)
      $st = $w * 4
      $buf = New-Object byte[] ($st * $h)
      $c.CopyPixels($buf, $st, 0)
      $nb = [Text.Encoding]::ASCII.GetBytes($nat.PadRight(32).Substring(0, 32))
      $hdr = [BitConverter]::GetBytes([int]$w) + [BitConverter]::GetBytes([int]$h) + [BitConverter]::GetBytes([int]$bpp) + [BitConverter]::GetBytes([int]$nst) + [BitConverter]::GetBytes([double]$f.DpiX) + [BitConverter]::GetBytes([double]$f.DpiY)
      [IO.File]::WriteAllBytes($a[2], [byte[]]($hdr + $nb + $buf + $nbuf))
    } elseif ($a[0] -eq 'encode') {
      $fmt = $a[1]; $w = [int]$a[3]; $h = [int]$a[4]; $st = [int]$a[6]; $dpi = [double]::Parse($a[7], [Globalization.CultureInfo]::InvariantCulture)
      $raw = [IO.File]::ReadAllBytes($a[2])
      $pf = [System.Windows.Media.PixelFormats]::$fmt
      $pal = $null
      if ($fmt -eq 'BlackWhite') { $pal = [System.Windows.Media.Imaging.BitmapPalettes]::BlackAndWhite }
      $src = [System.Windows.Media.Imaging.BitmapSource]::Create($w, $h, $dpi, $dpi, $pf, $pal, $raw, $st)
      $enc = New-Object System.Windows.Media.Imaging.WmpBitmapEncoder
      for ($i = 8; $i -lt $a.Length; $i++) {
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
    [IO.File]::WriteAllText(($a[2] + '.err'), $_.Exception.ToString())
  }
}
"""


class WicRaw:
    """Batch WPF jobs: encode raw pixels of any WPF pixel format with WmpBitmapEncoder,
    decode to Bgra32 + the native pixel format."""

    def __init__(self, work):
        self.work = Path(work)
        self.work.mkdir(parents=True, exist_ok=True)
        self.jobs = []

    def encode(self, raw, fmt, w, h, stride, out_path, dpi=96.0, **props):
        rp = Path(str(out_path) + ".in")
        rp.write_bytes(bytes(raw))
        extra = "".join(f"\t{k}={v}" for k, v in props.items())
        self.jobs.append(f"encode\t{fmt}\t{rp.resolve()}\t{w}\t{h}\t{Path(out_path).resolve()}\t{stride}\t{dpi}{extra}")
        return rp

    def decode(self, path, out_path):
        self.jobs.append(f"decode\t{Path(path).resolve()}\t{Path(out_path).resolve()}")

    def run(self, timeout=3600):
        if not self.jobs:
            return
        ps = self.work / "jxr_wic.ps1"
        ps.write_text(_PS, encoding="utf-8")
        jf = self.work / f"jxr_wic_{os.getpid()}.jobs"
        jf.write_text("\n".join(self.jobs) + "\n", encoding="utf-8")
        self.jobs = []
        subprocess.run(["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(ps), str(jf)],
                       capture_output=True, timeout=timeout)
        jf.unlink()


def read_wic_decode(path):
    b = Path(path).read_bytes()
    w, h, bpp, nst = struct.unpack_from("<iiii", b, 0)
    dx, dy = struct.unpack_from("<dd", b, 16)
    nat = b[32:64].decode().strip()
    bgra = np.frombuffer(b, np.uint8, count=w * h * 4, offset=64).reshape(h, w, 4)
    native = b[64 + w * h * 4:]
    return dict(w=w, h=h, bpp=bpp, stride=nst, fmt=nat, bgra=bgra, native=native, dpi=(dx, dy))


# ================================================================== test images

def base_image(rng, w, h, kind):
    """(h, w, 3) uint8 RGB and (h, w) uint8 alpha."""
    y, x = np.mgrid[0:h, 0:w].astype(np.float64)
    if kind == "noise":
        rgb = rng.integers(0, 256, (h, w, 3))
    else:
        r = 128 + 100 * np.sin(x / 7.0 + y / 13.0)
        g = 128 + 90 * np.cos(x / 11.0 - y / 5.0)
        b = (x * 3 + y * 2) % 256
        rgb = np.stack([r, g, b], -1) + rng.normal(0, 6, (h, w, 3))
        edge = ((x // 9 + y // 7) % 3 == 0)
        rgb[edge] = rgb[edge] * 0.3 + 160
    a = 255 - (np.abs(x - w / 2) + np.abs(y - h / 2)) * 400 / max(w + h, 1)
    a = a + rng.normal(0, 8, (h, w))
    a[(x + y) % 11 == 0] = 0
    a[(x * 3 + y) % 13 == 0] = 255
    return np.clip(rgb, 0, 255).astype(np.uint8), np.clip(a, 0, 255).astype(np.uint8)


FORMATS = ["Bgr24", "Bgr32", "Bgra32", "Pbgra32", "Gray8", "BlackWhite", "Gray16", "Rgb48", "Rgba64",
           "Prgba64", "Rgba128Float", "Prgba128Float", "Rgb128Float", "Gray32Float", "Cmyk32", "Bgr565",
           "Bgr555", "Bgr101010"]
ALPHA_FORMATS = {"Bgra32", "Pbgra32", "Rgba64", "Prgba64", "Rgba128Float", "Prgba128Float"}


def raw_for(fmt, rgb, a, rng):
    """Raw pixels (bytes, stride) of a WPF pixel format."""
    h, w = a.shape
    r, g, b = (rgb[..., i].astype(np.int64) for i in range(3))
    a64 = a.astype(np.int64)
    if fmt == "Bgr24":
        return np.stack([b, g, r], -1).astype(np.uint8).tobytes(), w * 3
    if fmt == "Bgr32":
        return np.stack([b, g, r, rng.integers(0, 256, (h, w))], -1).astype(np.uint8).tobytes(), w * 4
    if fmt == "Bgra32":
        return np.stack([b, g, r, a64], -1).astype(np.uint8).tobytes(), w * 4
    if fmt == "Pbgra32":
        p = np.stack([b, g, r], -1) * a64[..., None] // 255
        return np.concatenate([p, a64[..., None]], -1).astype(np.uint8).tobytes(), w * 4
    if fmt == "Gray8":
        return ((r * 77 + g * 150 + b * 29) >> 8).astype(np.uint8).tobytes(), w
    if fmt == "BlackWhite":
        bits = (((r + g + b) // 3 + rng.integers(-60, 60, (h, w))) > 127).astype(np.uint8)
        st = (w + 7) // 8
        out = np.zeros((h, st), np.uint8)
        for i in range(w):
            out[:, i // 8] |= bits[:, i] << (7 - i % 8)
        return out.tobytes(), st
    if fmt == "Gray16":
        v = ((r * 77 + g * 150 + b * 29) + rng.integers(0, 256, (h, w))).clip(0, 65535)
        return v.astype("<u2").tobytes(), w * 2
    if fmt in ("Rgb48", "Rgba64", "Prgba64"):
        c = (np.stack([r, g, b], -1) * 257 + rng.integers(-200, 200, (h, w, 3))).clip(0, 65535)
        if fmt == "Rgb48":
            return c.astype("<u2").tobytes(), w * 6
        a16 = (a64 * 257 + rng.integers(-100, 100, (h, w))).clip(0, 65535)
        if fmt == "Prgba64":
            c = c * a16[..., None] // 65535
        return np.concatenate([c, a16[..., None]], -1).astype("<u2").tobytes(), w * 8
    if fmt in ("Rgba128Float", "Prgba128Float", "Rgb128Float", "Gray32Float"):
        c = (np.stack([r, g, b], -1) / 255.0) ** 2.2 * 1.05 + rng.normal(0, 0.002, (h, w, 3))
        if fmt == "Gray32Float":
            return c[..., 1].astype("<f4").tobytes(), w * 4
        af = a64 / 255.0
        if fmt == "Prgba128Float":
            c = c * af[..., None]
        if fmt == "Rgb128Float":
            af = np.ones_like(af)
        return np.concatenate([c, af[..., None]], -1).astype("<f4").tobytes(), w * 16
    if fmt == "Cmyk32":
        k = rng.integers(0, 90, (h, w))
        return np.stack([255 - r, 255 - g, 255 - b, k], -1).astype(np.uint8).tobytes(), w * 4
    if fmt in ("Bgr565", "Bgr555"):
        if fmt == "Bgr565":
            v = (r >> 3 << 11) | (g >> 2 << 5) | (b >> 3)
        else:
            v = (r >> 3 << 10) | (g >> 3 << 5) | (b >> 3)
        return v.astype("<u2").tobytes(), w * 2
    if fmt == "Bgr101010":
        c = np.stack([r, g, b], -1) * 4 + rng.integers(0, 4, (h, w, 3))
        v = (c[..., 0] << 20) | (c[..., 1] << 10) | c[..., 2]
        return v.astype("<u4").tobytes(), w * 4
    raise ValueError(fmt)


# ================================================================== WIC native samples

def wic_native(ref):
    """WIC's native pixels -> (h, w, n) int64 in the order of op 200 mode 3 (channels 0.., alpha
    last); float formats as IEEE bit patterns; None for formats without a comparison."""
    fmt, w, h, st = ref["fmt"], ref["w"], ref["h"], ref["stride"]
    raw = np.frombuffer(ref["native"], np.uint8).reshape(h, st)
    if fmt == "Gray8":
        return raw[:, :w].astype(np.int64)[..., None]
    if fmt in ("Bgr24", "Bgr32", "Bgra32", "Pbgra32"):
        n = 3 if fmt == "Bgr24" else 4
        v = raw[:, :w * n].reshape(h, w, n).astype(np.int64)
        out = v[..., [2, 1, 0]]
        return np.concatenate([out, v[..., 3:4]], -1) if fmt in ("Bgra32", "Pbgra32") else out
    if fmt == "Gray16":
        return raw[:, :w * 2].copy().view("<u2").astype(np.int64)[..., None]
    if fmt in ("Rgb48", "Rgba64", "Prgba64"):
        n = 3 if fmt == "Rgb48" else 4
        return raw[:, :w * 2 * n].copy().view("<u2").reshape(h, w, n).astype(np.int64)
    if fmt in ("Rgba128Float", "Prgba128Float", "Rgb128Float"):
        v = raw[:, :w * 16].copy().view("<u4").reshape(h, w, 4).astype(np.int64)
        return v[..., :3] if fmt == "Rgb128Float" else v
    if fmt == "Gray32Float":
        return raw[:, :w * 4].copy().view("<u4").astype(np.int64)[..., None]
    if fmt == "Cmyk32":
        return raw[:, :w * 4].reshape(h, w, 4).astype(np.int64)
    if fmt == "BlackWhite":
        return np.unpackbits(raw, axis=1)[:, :w].astype(np.int64)[..., None]
    if fmt in ("Bgr565", "Bgr555"):
        v = raw[:, :w * 2].copy().view("<u2").astype(np.int64)
        if fmt == "Bgr565":
            return np.stack([v >> 11, (v >> 5) & 63, v & 31], -1)
        return np.stack([(v >> 10) & 31, (v >> 5) & 31, v & 31], -1)
    if fmt == "Bgr101010":
        v = raw[:, :w * 4].copy().view("<u4").astype(np.int64)
        return np.stack([(v >> 20) & 1023, (v >> 10) & 1023, v & 1023], -1)
    return None


def tol_of(fmt):
    """Allowed 8-bit difference against WIC per native pixel format."""
    if fmt in ("Gray16", "Bgr101010", "Prgba64") or "Float" in fmt or "Half" in fmt or "Fixed" in fmt:
        return 1
    return 0


def cmyk_rgb(nat):
    """(h, w, 4) CMYK -> (h, w, 4) BGRA by the library formula."""
    c, m, y, k = (nat[..., i] for i in range(4))
    ik = 255 - k

    def d(v):
        x = (255 - v) * ik + 128
        return (x + (x >> 8)) >> 8
    out = np.stack([d(y), d(m), d(c), np.full_like(c, 255)], -1)
    return out.astype(np.uint8)


# ================================================================== case matrix

class Case:
    def __init__(self, name, fmt, w, h, props, kind="smooth", dpi=96.0, seed=0):
        self.name, self.fmt, self.w, self.h, self.props = name, fmt, w, h, dict(props)
        self.kind, self.dpi, self.seed = kind, dpi, seed
        self.data, self.ref, self.err = None, None, None
        self.wic_alpha_bad = False

    def key(self):
        s = repr((self.fmt, self.w, self.h, sorted(self.props.items()), self.kind, self.dpi, self.seed, 3))
        return hashlib.sha1(s.encode()).hexdigest()[:16]


def codec_props(r, fmt):
    p = dict(UseCodecOptions=True)
    p["QualityLevel"] = r.choice([1, 1, 2, 5, 10, 25, 40, 70, 100, 160, 255])
    p["OverlapLevel"] = r.choice([0, 1, 2])
    if fmt not in ("Gray8", "Gray16", "Gray32Float", "BlackWhite", "Cmyk32"):
        p["SubsamplingLevel"] = r.choice([0, 1, 2, 3])
    if r.random() < 0.4:
        p["HorizontalTileSlices"] = r.choice([1, 2, 3, 5])
    if r.random() < 0.4:
        p["VerticalTileSlices"] = r.choice([1, 2, 4])
    if r.random() < 0.35:
        p["FrequencyOrder"] = True
    if fmt in ALPHA_FORMATS:
        p["InterleavedAlpha"] = r.choice([True, False])
        if r.random() < 0.5:
            p["AlphaQualityLevel"] = r.choice([1, 10, 60, 200])
        if r.random() < 0.3:
            p["AlphaDataDiscardLevel"] = r.choice([0, 1, 2, 3])
    if r.random() < 0.3:
        p["ImageDataDiscardLevel"] = r.choice([0, 1, 2, 3])
    if r.random() < 0.15:
        p["IgnoreOverlap"] = True
    return p


def build_cases(seed):
    r = random.Random(seed)
    cases = []
    # default options: ImageQualityLevel / Lossless over the 8-bit formats
    for fmt in ("Bgr24", "Bgra32", "Pbgra32", "Gray8"):
        for q in (0.1, 0.5, 0.9, 1.0):
            cases.append(Case(f"iq{q}_{fmt}", fmt, 67, 45, dict(ImageQualityLevel=q)))
        cases.append(Case(f"lossless_{fmt}", fmt, 67, 45, dict(Lossless=True)))
    # every format: default, lossless, codec options
    for fmt in FORMATS:
        cases.append(Case(f"fmt_{fmt}_def", fmt, 53, 37, dict(ImageQualityLevel=0.8)))
        cases.append(Case(f"fmt_{fmt}_ll", fmt, 37, 29, dict(Lossless=True)))
        cases.append(Case(f"fmt_{fmt}_q25", fmt, 37, 29, dict(UseCodecOptions=True, QualityLevel=25, OverlapLevel=1)))
        if fmt in ALPHA_FORMATS:
            cases.append(Case(f"fmt_{fmt}_ia", fmt, 41, 23, dict(UseCodecOptions=True, QualityLevel=10,
                                                                  InterleavedAlpha=True, OverlapLevel=2)))
    # subsampling x overlap x quality
    for ss in range(4):
        for ol in range(3):
            for ql in (1, 30):
                cases.append(Case(f"ss{ss}_ol{ol}_q{ql}", "Bgr24", 49, 35,
                                  dict(UseCodecOptions=True, QualityLevel=ql, OverlapLevel=ol, SubsamplingLevel=ss)))
    # discard levels, frequency order, tiles
    for dl in range(4):
        for fo in (False, True):
            cases.append(Case(f"discard{dl}_fo{int(fo)}", "Bgra32", 50, 42,
                              dict(UseCodecOptions=True, QualityLevel=8, ImageDataDiscardLevel=dl,
                                   AlphaDataDiscardLevel=dl, FrequencyOrder=fo, OverlapLevel=1)))
    for hs, vs in ((1, 0), (0, 1), (2, 3), (5, 1)):
        for fo in (False, True):
            cases.append(Case(f"tiles{hs}x{vs}_fo{int(fo)}", "Bgr24", 120, 90,
                              dict(UseCodecOptions=True, QualityLevel=12, HorizontalTileSlices=hs,
                                   VerticalTileSlices=vs, FrequencyOrder=fo, OverlapLevel=2, SubsamplingLevel=1)))
    # orientation
    for rot in ("Rotate90", "Rotate180", "Rotate270"):
        cases.append(Case(f"rot_{rot}", "Bgr24", 41, 27, dict(Rotation=rot)))
    cases.append(Case("flip_h", "Bgr24", 41, 27, dict(FlipHorizontal=True)))
    cases.append(Case("flip_v", "Bgr24", 41, 27, dict(FlipVertical=True)))
    cases.append(Case("flip_hv_r90", "Bgra32", 41, 27, dict(FlipHorizontal=True, FlipVertical=True, Rotation="Rotate90")))
    # sizes
    for (w, h) in ((1, 1), (1, 17), (17, 1), (2, 3), (15, 15), (16, 16), (17, 17), (31, 33), (100, 75), (257, 130)):
        cases.append(Case(f"size{w}x{h}", r.choice(["Bgr24", "Bgra32", "Gray8"]), w, h,
                          dict(UseCodecOptions=True, QualityLevel=r.choice([1, 20]), OverlapLevel=r.choice([0, 1, 2]))))
    cases.append(Case("big_lossy", "Bgra32", 1001, 777, dict(ImageQualityLevel=0.7), kind="smooth"))
    cases.append(Case("big_ll", "Bgr24", 1001, 777, dict(Lossless=True), kind="smooth"))
    cases.append(Case("noise_lossy", "Bgr24", 96, 64, dict(UseCodecOptions=True, QualityLevel=3), kind="noise"))
    cases.append(Case("noise_ll", "Bgra32", 96, 64, dict(Lossless=True, InterleavedAlpha=True), kind="noise"))
    # random combinations
    for i in range(110):
        fmt = r.choice(FORMATS + ["Bgr24", "Bgra32", "Bgr24", "Gray8"])
        w, h = r.randint(1, 140), r.randint(1, 110)
        cases.append(Case(f"rnd{i}_{fmt}", fmt, w, h, codec_props(r, fmt), kind=r.choice(["smooth", "noise"]),
                          seed=i))
    for c in cases:
        if "dpi" not in c.name and r.random() < 0.1:
            c.dpi = r.choice([72.0, 150.0, 300.0, 299.5])
    return cases


def materialise(cases, cache):
    """Encode (WIC) and decode (WIC) every case, cached by key."""
    cache.mkdir(parents=True, exist_ok=True)
    wic = WicRaw(cache)
    todo = []
    for c in cases:
        f = cache / f"{c.key()}.jxr"
        if not f.exists():
            rng = np.random.default_rng(1000 + c.seed)
            rgb, a = base_image(rng, c.w, c.h, c.kind)
            raw, st = raw_for(c.fmt, rgb, a, rng)
            wic.encode(raw, c.fmt, c.w, c.h, st, f, dpi=c.dpi, **c.props)
            todo.append(c)
    wic.run()
    for c in cases:
        f = cache / f"{c.key()}.jxr"
        d = cache / f"{c.key()}.wic"
        if f.exists() and not d.exists():
            wic.decode(f, d)
    wic.run()
    for c in cases:
        f = cache / f"{c.key()}.jxr"
        d = cache / f"{c.key()}.wic"
        e = Path(str(f) + ".in.err")
        for p in (Path(str(f) + ".in"),):
            if p.exists():
                p.unlink()
        if f.exists():
            c.data = f.read_bytes()
        if d.exists():
            c.ref = read_wic_decode(d)
        if e.exists():
            c.err = e.read_text(errors="replace").splitlines()[0][:160]
        if c.ref is not None and c.fmt in ALPHA_FORMATS and c.ref["fmt"] in ALPHA_FORMATS:
            # WIC's decoder loses the interleaved alpha plane of some files (4:0:0 / 4:2:0 internal
            # formats): its alpha is far from the source while our decode follows the codestream
            rng = np.random.default_rng(1000 + c.seed)
            a = base_image(rng, c.w, c.h, c.kind)[1].astype(int)
            wa = c.ref["bgra"][..., 3].astype(int)
            lossless = c.props.get("Lossless") or (c.props.get("QualityLevel") == 1 and
                                                    c.props.get("AlphaQualityLevel", 1) == 1)
            lossless = lossless and not c.props.get("ImageDataDiscardLevel") and not c.props.get("AlphaDataDiscardLevel")
            if wa.shape == a.shape and ((wa.max() == 0 and a.max() > 0) or (lossless and np.abs(wa - a).mean() > 30)):
                c.wic_alpha_bad = True
    return cases


# ================================================================== checks

def check_wic(c, flags=STRAIGHT):
    ref = c.ref
    nat = ref["fmt"]
    tol = tol_of(nat)
    if nat == "Cmyk32":
        exp = cmyk_rgb(wic_native(ref))
    else:
        exp = ref["bgra"]
    nch = 3 if c.wic_alpha_bad else 4                   # WIC alpha unusable: colour only

    def chk(res, out):
        if res != 0:
            return f"error {it.err_name(res)} ({nat})"
        d = it.parse_decex(out)
        if d["px"] is None or d["px"].shape != exp.shape:
            return f"size {None if d['px'] is None else d['px'].shape} != {exp.shape}"
        e = np.abs(d["px"][..., :nch].astype(int) - exp[..., :nch].astype(int))
        if e.max() > tol:
            y, x = np.argwhere(e.max(-1) > tol)[0]
            return (f"{nat}: max |d| {e.max()} > {tol} ({int((e > tol).sum())} samples) at ({x},{y}): "
                    f"got {list(d['px'][y, x])} exp {list(exp[y, x])}")
        return None
    return chk


def check_pargb(c):
    """Premultiplied source, PARGB output: WIC's native Pbgra32 with colour clamped to alpha."""
    nat = np.frombuffer(c.ref["native"], np.uint8).reshape(c.ref["h"], c.ref["stride"])[:, :c.ref["w"] * 4]
    nat = nat.reshape(c.ref["h"], c.ref["w"], 4).astype(int)
    exp = nat.copy()
    exp[..., :3] = np.minimum(exp[..., :3], exp[..., 3:4])

    def chk(res, out):
        if res != 0:
            return f"error {it.err_name(res)}"
        d = it.parse_decex(out)
        if d["fmt"] != C.FMT_PARGB:
            return f"fmt {d['fmt']} != FMT_PARGB"
        e = np.abs(d["px"].astype(int) - exp)
        if e.max():
            y, x = np.argwhere(e.max(-1) > 0)[0]
            return f"PARGB max |d| {e.max()} at ({x},{y}): got {list(d['px'][y, x])} exp {list(exp[y, x])}"
        return None
    return chk


def check_native(c):
    wn = wic_native(c.ref)

    def chk(res, out):
        if res != 0:
            return f"error {it.err_name(res)}"
        w, h = struct.unpack_from("<2I", out, 0)
        a = np.frombuffer(out, np.int32, w * h * 5, 8).reshape(h, w, 5).astype(np.int64) & 0xFFFFFFFF
        n = wn.shape[2]
        has_alpha = c.ref["fmt"] in ALPHA_FORMATS
        if has_alpha:
            mine = np.concatenate([a[..., :n - 1], a[..., 4:5]], -1)
            if tag_value(c.data, 0xBCC2) is not None or c.wic_alpha_bad:
                mine, ref = mine[..., :n - 1], wn[..., :n - 1]     # planar alpha: not in the dump
            else:
                ref = wn
        else:
            mine, ref = a[..., :n], wn
        if mine.shape != ref.shape:
            return f"shape {mine.shape} != {ref.shape}"
        e = mine != ref
        if e.any():
            y, x, ch = np.argwhere(e)[0]
            return (f"{c.ref['fmt']}: {int(e.sum())} samples differ, first ({x},{y}) ch {ch}: "
                    f"{int(mine[y, x, ch])} != {int(ref[y, x, ch])}")
        return None
    return chk


def is_premul(fmt):
    return fmt.startswith("P")


def suite_wic(runners, cases, suites):
    for t, R in runners.items():
        s = it.Suite("wic", t)
        recs = []
        for c in cases:
            if c.data is None or c.ref is None:
                continue
            if c.wic_alpha_bad and is_premul(c.ref["fmt"]):
                continue                                  # colour depends on WIC's broken alpha
            recs.append(Rec(it.OP_DECEX, 0, it.decex_input(c.data, flags=STRAIGHT), check_wic(c), c.name))
            if is_premul(c.ref["fmt"]) and c.ref["fmt"] == "Pbgra32":
                recs.append(Rec(it.OP_DECEX, 0, it.decex_input(c.data, flags=0), check_pargb(c), c.name + "/pargb"))
        it.run_suite(R, s, recs)
        bad = [c.name for c in cases if c.wic_alpha_bad]
        if bad:
            s.notes.append(f"WIC alpha unusable (colour compared only / premultiplied skipped): {', '.join(bad)}")
        skipped = [c.name + (f" ({c.err})" if c.err else "") for c in cases if c.data is None or c.ref is None]
        if skipped:
            s.notes.append(f"{len(skipped)} cases not produced by WIC: " + ", ".join(skipped[:8]))
        suites.append(s)


def suite_native(runners, cases, suites):
    for t, R in runners.items():
        s = it.Suite("native", t)
        recs = []
        for c in cases:
            if c.data is None or c.ref is None or wic_native(c.ref) is None:
                continue
            if any(k in c.props for k in ("Rotation", "FlipHorizontal", "FlipVertical")):
                continue                                  # WIC returns the transformed image
            recs.append(Rec(OP_JDUMP, 3, c.data, check_native(c), c.name))
        it.run_suite(R, s, recs)
        suites.append(s)


# ---------------------------------------------------------------- container editing

def parse_ifd(data):
    """-> (ifd offset, [(tag, type, count, value/offset field bytes)])"""
    off = struct.unpack_from("<I", data, 4)[0]
    n = struct.unpack_from("<H", data, off)[0]
    ents = [struct.unpack_from("<HHI4s", data, off + 2 + 12 * i) for i in range(n)]
    return off, ents


def add_tags(data, tags):
    """Rewrite the first IFD with extra entries {tag: (type, bytes)} appended after the file."""
    off, ents = parse_ifd(data)
    body = bytearray(data)
    new = [e for e in ents if e[0] not in tags]
    for tag, (typ, val) in tags.items():
        size = {1: 1, 2: 1, 3: 2, 4: 4, 7: 1}[typ]
        cnt = len(val) // size
        if len(val) <= 4:
            field = bytes(val).ljust(4, b"\0")
        else:
            while len(body) & 3:
                body.append(0)
            field = struct.pack("<I", len(body))
            body += val
        new.append((tag, typ, cnt, field))
    new.sort()
    while len(body) & 3:
        body.append(0)
    ifd = len(body)
    body += struct.pack("<H", len(new))
    for e in new:
        body += struct.pack("<HHI4s", *e)
    body += struct.pack("<I", 0)
    struct.pack_into("<I", body, 4, ifd)
    return bytes(body)


def set_field(data, tag, value):
    """Overwrite the value field of an existing entry (LONG / SHORT inline values)."""
    off, ents = parse_ifd(data)
    b = bytearray(data)
    for i, (t, typ, cnt, f) in enumerate(ents):
        if t == tag:
            p = off + 2 + 12 * i
            if typ == 3:
                struct.pack_into("<H", b, p + 8, value & 0xFFFF)
            else:
                struct.pack_into("<I", b, p + 8, value & 0xFFFFFFFF)
            return bytes(b)
    return bytes(b)


def set_count(data, tag, cnt):
    off, ents = parse_ifd(data)
    b = bytearray(data)
    for i, e in enumerate(ents):
        if e[0] == tag:
            struct.pack_into("<I", b, off + 2 + 12 * i + 4, cnt & 0xFFFFFFFF)
    return bytes(b)


def tag_value(data, tag):
    off, ents = parse_ifd(data)
    for t, typ, cnt, f in ents:
        if t == tag:
            return struct.unpack("<I", f)[0] if typ == 4 else struct.unpack("<H", f[:2])[0]
    return None


def suite_meta(runners, cases, suites):
    by = {c.name: c for c in cases if c.data is not None and c.ref is not None}
    recs = []

    def want(c, **kv):
        def chk(res, out):
            if res != 0:
                return f"error {it.err_name(res)}"
            d = it.parse_decex(out)
            m = d["meta"]
            for k, v in kv.items():
                got = m[k] if k in m else d[k]
                if callable(v):
                    msg = v(got)
                    if msg:
                        return f"{k}: {msg}"
                elif got != v:
                    return f"{k} = {got}, expected {v}"
            return None
        return chk

    def flags_has(on=0, off=0):
        return lambda f: None if (f & on) == on and (f & off) == 0 else f"flags {f:#x} (on {on:#x} off {off:#x})"

    expected_bits = {"Bgr24": 8, "Bgra32": 8, "Gray8": 8, "Gray16": 16, "Rgb48": 16, "Rgba64": 16,
                     "Rgba128Float": 32, "BlackWhite": 1, "Bgr565": 6, "Bgr555": 5, "Bgr101010": 10, "Cmyk32": 8}
    for name, c in by.items():
        nat = c.ref["fmt"]
        if not name.startswith("fmt_") or nat not in expected_bits:
            continue
        ch = {"Gray8": 1, "Gray16": 1, "BlackWhite": 1, "Cmyk32": 4, "Bgra32": 4, "Rgba64": 4,
              "Rgba128Float": 4}.get(nat, 3)
        model = C.IMM_GRAY if nat in ("Gray8", "Gray16", "BlackWhite") else C.IMM_CMYK if nat == "Cmyk32" else C.IMM_RGB
        on = (C.IMF_ALPHA if nat in ALPHA_FORMATS else 0) | (C.IMF_HDR if expected_bits[nat] > 8 else 0) | \
            (C.IMF_FLOAT if "Float" in nat else 0)
        off = (0 if nat in ALPHA_FORMATS else C.IMF_ALPHA)
        if name.endswith("_ll") and "Float" not in nat:
            on |= C.IMF_LOSSLESS
        elif name.endswith("_q25"):
            off |= C.IMF_LOSSLESS
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(c.data, flags=STRAIGHT | C.IMGD_PROBE),
                        want(c, w=c.w, h=c.h, bits=expected_bits[nat], channels=ch, model=model,
                             flags=lambda f, on=on, off=off & ~C.IMF_LOSSLESS: flags_has(on & ~C.IMF_LOSSLESS, off)(f)),
                        f"{name}/probe"))
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(c.data, flags=STRAIGHT),
                        want(c, flags=flags_has(on, off)), f"{name}/flags"))
    for name in ("lossless_Bgr24", "lossless_Pbgra32", "iq1.0_Bgr24"):
        if name in by:
            on = C.IMF_LOSSLESS | (C.IMF_ALPHA | C.IMF_SRC_PREMUL if "Pbgra" in name else 0)
            recs.append(Rec(it.OP_DECEX, 0, it.decex_input(by[name].data, flags=STRAIGHT),
                            want(by[name], flags=flags_has(on)), f"{name}/lossless"))
    for name in ("iq0.5_Bgr24", "ss1_ol1_q1", "discard1_fo0"):
        if name in by:
            recs.append(Rec(it.OP_DECEX, 0, it.decex_input(by[name].data, flags=STRAIGHT),
                            want(by[name], flags=flags_has(0, C.IMF_LOSSLESS)), f"{name}/lossy"))
    # DPI
    for c in by.values():
        if c.dpi != 96.0 and c.ref["fmt"] in ("Bgr24", "Bgra32", "Gray8"):
            dp = round(c.dpi * 65536)
            recs.append(Rec(it.OP_DECEX, 0, it.decex_input(c.data, flags=STRAIGHT | C.IMGD_PROBE),
                            want(c, dpix=lambda v, dp=dp: None if abs(v - dp) <= 2 else f"{v} != {dp}",
                                 dpiy=lambda v, dp=dp: None if abs(v - dp) <= 2 else f"{v} != {dp}"),
                            f"{c.name}/dpi"))
    # orientation: the dispatcher applies IM_ORIENT; WIC returns the stored orientation
    for name, o in (("rot_Rotate90", 6), ("rot_Rotate180", 3), ("rot_Rotate270", 8), ("flip_h", 2), ("flip_v", 4)):
        if name in by:
            recs.append(Rec(it.OP_DECEX, 0, it.decex_input(by[name].data, flags=STRAIGHT | C.IMGD_PROBE),
                            want(by[name], orient=o), f"{name}/orient"))
    # ICC / XMP / EXIF blobs
    base = by.get("iq0.9_Bgr24")
    if base is not None:
        icc = bytes(range(256)) * 3
        xmp = b'<?xpacket begin=""?><x:xmpmeta xmlns:x="adobe:ns:meta/"><test>jxr</test></x:xmpmeta><?xpacket end="w"?>'
        exif_ifd = struct.pack("<H", 1) + struct.pack("<HHI4s", 0x9000, 7, 4, b"0230") + struct.pack("<I", 0)
        d = add_tags(base.data, {0x8773: (7, icc), 0x02BC: (1, xmp)})
        d = add_tags(d, {0x8769: (4, struct.pack("<I", len(d) + (-len(d) & 3)))})
        d = d + b"\0" * (-len(d) & 3)
        exif_off = len(d)
        d = set_field(d, 0x8769, exif_off) + exif_ifd

        def blobs(res, out):
            if res != 0:
                return f"error {it.err_name(res)}"
            r = it.parse_decex(out)
            if r["icc"] != icc:
                return f"ICC {len(r['icc'])} bytes"
            if r["xmp"] != xmp:
                return f"XMP {r['xmp'][:40]!r}"
            ex = r["exif"]
            if len(ex) < 8 or ex[:4] != b"II*\0":
                return f"EXIF {ex[:8]!r}"
            if b"0230" not in ex:
                return "EXIF IFD missing"
            return None
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(d, flags=STRAIGHT | C.IMGD_ICC | C.IMGD_XMP | C.IMGD_EXIF),
                        blobs, "blobs"))
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(d, flags=STRAIGHT), check_wic(base), "blobs/pixels"))
    for t, R in runners.items():
        s = it.Suite("meta", t)
        it.run_suite(R, s, recs)
        suites.append(s)


# ---------------------------------------------------------------- errors / fuzz

def bad_files(cases):
    by = {c.name: c for c in cases if c.data is not None and c.ref is not None}
    good = by.get("iq0.9_Bgr24") or next(iter(by.values()))
    d = good.data
    out = []
    err = it.expect_any(it.expect_err(*[v for k, v in C.__dict__.items() if k.startswith("ERR_")]))
    anyr = it.any_result
    out.append(("magic", b"II\xbc\x02" + d[4:], err))
    out.append(("mm", b"MM\x01\xbc" + d[4:], err))
    out.append(("ifd_past_end", d[:4] + struct.pack("<I", len(d) + 100) + d[8:], err))
    out.append(("ifd_huge", d[:4] + struct.pack("<I", 0xFFFFFFF0) + d[8:], err))
    off, ents = parse_ifd(d)
    out.append(("ifd_count0", d[:off] + b"\0\0" + d[off + 2:], err))
    out.append(("ifd_count_huge", d[:off] + b"\xff\xff" + d[off + 2:], err))
    ioff = tag_value(d, 0xBCC0)
    ilen = tag_value(d, 0xBCC1)
    out.append(("img_off_past", set_field(d, 0xBCC0, len(d) + 5), err))
    out.append(("img_len_huge", set_field(d, 0xBCC1, 0xFFFFFFFF), anyr))
    out.append(("img_len_0", set_field(d, 0xBCC1, 0), err))
    out.append(("img_off_wrap", set_field(d, 0xBCC0, 0xFFFFFFFF), err))
    out.append(("no_pixfmt", set_count(d, 0xBC01, 3), anyr))
    out.append(("width_0", set_field(d, 0xBC80, 0), anyr))
    out.append(("width_huge", set_field(d, 0xBC80, 0x7FFFFFFF), anyr))
    out.append(("height_huge", set_field(d, 0xBC81, 0xFFFFFFFF), anyr))
    b = bytearray(d)
    b[ioff:ioff + 8] = b"WMPHOTX\0"
    out.append(("cs_magic", bytes(b), err))
    b = bytearray(d)
    b[ioff + 8] = 0x91                                   # version 9
    out.append(("cs_version", bytes(b), anyr))
    b = bytearray(d)
    b[ioff + 11] = 0x7B                                  # RGB / reserved bit depth 11
    out.append(("cs_bitdepth", bytes(b), anyr))
    for cut in (8, 16, 64, ioff + 4, ioff + 12, ioff + 20, ioff + ilen // 3, ioff + ilen - 1):
        out.append((f"trunc{cut}", d[:cut], anyr))
    return out


def suite_errors(runners, cases, suites):
    by = {c.name: c for c in cases if c.data is not None and c.ref is not None}
    recs = [Rec(it.OP_DECEX, 0, it.decex_input(data, flags=STRAIGHT), chk, name) for name, data, chk in bad_files(cases)]
    # HARD_TILING_FLAG: several tiles with overlap filtering -> ERR_UNSUPPORTED; one tile -> as usual
    for name, chk in (("tiles2x3_fo0", it.expect_err(C.ERR_UNSUPPORTED)), ("iq0.9_Bgr24", None)):
        c = by.get(name)
        if c is not None:
            b = bytearray(c.data)
            ioff = tag_value(c.data, 0xBCC0)
            b[ioff + 8] |= 0x08
            recs.append(Rec(it.OP_DECEX, 0, it.decex_input(bytes(b), flags=STRAIGHT), chk or check_wic(c),
                            f"hardtile/{name}"))
    big = by.get("big_lossy")
    if big is not None:
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(big.data, flags=STRAIGHT, maxpx=1000 * 700),
                        it.expect_err(C.ERR_LIMIT, C.ERR_DIMS), "maxpx"))
        recs.append(Rec(it.OP_DECEX, 0, it.decex_input(big.data, flags=STRAIGHT, cancel=1),
                        it.expect_err(C.ERR_CANCEL), "cancel"))
    for t, R in runners.items():
        s = it.Suite("errors", t)
        it.run_suite(R, s, recs)
        suites.append(s)


def mutants(cases, n, seed):
    r = random.Random(seed)
    pool = [c for c in cases if c.data is not None and len(c.data) < 60000]
    out = []
    for i in range(n):
        c = r.choice(pool)
        d = bytearray(c.data)
        ioff = tag_value(bytes(d), 0xBCC0) or 0
        kind = r.randrange(7)
        if kind == 0:
            d = d[:r.randrange(1, len(d))]
        elif kind == 1:
            for _ in range(r.randint(1, 8)):
                p = r.randrange(ioff, len(d)) if ioff < len(d) else r.randrange(len(d))
                d[p] ^= 1 << r.randrange(8)
        elif kind == 2:
            p = r.randrange(len(d))
            ln = r.randint(1, 64)
            d[p:p + ln] = bytes([r.choice([0, 0xFF, r.randrange(256)])]) * min(ln, len(d) - p)
        elif kind == 3:                                   # codestream header bytes
            if ioff + 40 < len(d):
                for _ in range(r.randint(1, 3)):
                    d[ioff + 8 + r.randrange(32)] = r.randrange(256)
        elif kind == 4:                                   # IFD entry fields
            off, ents = parse_ifd(bytes(d))
            if ents:
                k = r.randrange(len(ents))
                p = off + 2 + 12 * k + r.choice([4, 8])
                struct.pack_into("<I", d, p, r.choice([0, 1, 0xFFFF, 0x7FFFFFFF, 0xFFFFFFFF, r.randrange(1 << 32)]))
        elif kind == 5:                                   # index table / packet area
            if ioff + 64 < len(d):
                p = ioff + r.randrange(16, min(len(d) - ioff, 400))
                d[p:p + 4] = struct.pack("<I", r.randrange(1 << 32))
        else:                                             # many random bytes in the codestream
            for _ in range(r.randint(10, 60)):
                p = r.randrange(ioff, len(d)) if ioff < len(d) else r.randrange(len(d))
                d[p] = r.randrange(256)
        out.append((f"fuzz{i}_{c.name}_k{kind}", bytes(d)))
    return out


def suite_fuzz(runners, cases, suites, n, seed):
    ms = mutants(cases, n, seed)
    for t, R in runners.items():
        s = it.Suite("fuzz", t)
        recs = [Rec(it.OP_DECEX, 0, it.decex_input(d, flags=STRAIGHT), it.any_result, nm) for nm, d in ms]
        recs += [Rec(OP_JDUMP, 3, d, it.any_result, nm + "/dump") for nm, d in ms[: n // 4]]
        t0 = time.time()
        it.run_suite(R, s, recs)
        s.notes.append(f"{len(recs)} mutants in {time.time() - t0:.1f}s")
        suites.append(s)


# ---------------------------------------------------------------- perf

def suite_perf(runners, cache, suites):
    w, h = 4000, 3000
    cs = [Case("perf12mp", "Bgr24", w, h, dict(ImageQualityLevel=0.9)),
          Case("perf12mp_ol2", "Bgr24", w, h, dict(UseCodecOptions=True, QualityLevel=20, OverlapLevel=2,
                                                   SubsamplingLevel=1))]
    materialise(cs, cache)
    for t, R in runners.items():
        s = it.Suite("perf", t)
        for c in cs:
            if c.data is None:
                s.fail(f"{c.name}: WIC did not produce the file ({c.err})")
                continue
            reps = 3
            res, out = R.one(OP_JTIME, reps | (STRAIGHT << 16), c.data)
            if res != 0:
                s.fail(f"{c.name}: error {it.err_name(res)}")
                continue
            ms = struct.unpack("<Q", out)[0] / 1000.0 / reps
            s.notes.append(f"{c.name}: {ms:.0f} ms per decode ({len(c.data) // 1024} KB)")
            s.check(ms < 400 or c.name != "perf12mp", f"{c.name}: {ms:.0f} ms >= 400 ms")
        suites.append(s)


# ================================================================== encoder (jxr_enc.s)

def _enc_image(rng, h, w, alpha=False, noise=6.0):
    y, x = np.mgrid[0:h, 0:w]
    img = np.zeros((h, w, 4), np.uint8)
    for c in range(3):
        f = 128 + 100 * np.sin(x / (7 + 3 * c) + y / (11 - c)) + rng.normal(0, noise, (h, w))
        img[:, :, c] = np.clip(f, 0, 255)
    img[:, :, 3] = 255
    if alpha:
        img[:, :, 3] = np.clip(x * 255 // max(1, w - 1), 0, 255)
        img[: h // 3, : w // 3, 3] = 0
    return img


def enc_cases(seed):
    """(name, bgra straight, quality, flags, bitdepth, chroma, meta) -- meta = (icc, xmp, dpix, dpiy)"""
    rng = np.random.default_rng(seed)
    cs = []
    flat = np.full((16, 16, 4), 128, np.uint8)
    flat[:, :, 3] = 255
    cs.append(("flat_ll", flat, 0, C.IOF_LOSSLESS, 0, 0, None))
    cs.append(("flat_q", flat, 90, 0, 0, 0, None))
    noise = rng.integers(0, 256, (37, 50, 4), dtype=np.uint8)
    noise[:, :, 3] = 255
    cs.append(("noise_ll", noise, 0, C.IOF_LOSSLESS, 0, 0, None))
    cs.append(("noise_q50", noise, 50, 0, 0, 0, None))
    im = _enc_image(rng, 70, 90)
    cs.append(("sm_ll", im, 0, C.IOF_LOSSLESS, 0, 0, None))
    for q in (100, 90, 75, 50, 20, 1):
        cs.append((f"sm_q{q}", im, q, 0, 0, 0, None))
    cs.append(("sm_default", im, 0, 0, 0, 0, None))
    big = _enc_image(rng, 133, 301)
    cs.append(("big_q90", big, 90, 0, 0, 0, None))
    cs.append(("big_ll", big, 0, C.IOF_LOSSLESS, 0, 0, None))
    al = _enc_image(rng, 45, 61, alpha=True)
    cs.append(("alpha_q80", al, 80, 0, 0, 0, None))
    cs.append(("alpha_ll", al, 0, C.IOF_LOSSLESS, 0, 0, None))
    cs.append(("alpha_bd24", al, 80, 0, 24, 0, None))
    cs.append(("opaque_bd32", im, 80, 0, 32, 0, None))
    for (h, w) in ((1, 1), (1, 40), (40, 1), (17, 33), (16, 48)):
        img = _enc_image(rng, h, w, alpha=(w > 20))
        cs.append((f"s{w}x{h}_q", img, 70, 0, 0, 0, None))
        cs.append((f"s{w}x{h}_ll", img, 0, C.IOF_LOSSLESS, 0, 0, None))
    for ch in (420, 422):
        cs.append((f"c{ch}_q90", im, 90, 0, 0, ch, None))
        cs.append((f"c{ch}_q30", big, 30, 0, 0, ch, None))
        cs.append((f"c{ch}_alpha", al, 60, 0, 0, ch, None))
        cs.append((f"c{ch}_ll", im, 0, C.IOF_LOSSLESS, 0, ch, None))   # lossless forces 4:4:4
    icc = bytes(range(256)) * 3
    xmp = b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">jxr</x:xmpmeta>"
    cs.append(("meta", im, 85, C.IOF_EMBED_ICC | C.IOF_EMBED_XMP, 0, 0, (icc, xmp, 72 << 16, 300 << 16)))
    cs.append(("meta_odd", al, 85, C.IOF_EMBED_ICC | C.IOF_EMBED_XMP, 0, 0, (icc[:301], xmp + b" ", 0, 0)))
    return cs


def _enc_ref(c):
    name, img, q, flags, bd, ch, meta = c
    alpha = {24: False, 32: True}.get(bd)
    icc = xmp = dpi = None
    if meta:
        icc, xmp = meta[0], meta[1]
        dpi = (meta[2], meta[3])
    return JREF.encode(img, quality=q or 90, lossless=bool(flags & C.IOF_LOSSLESS), chroma=ch or 444,
                       alpha=alpha, icc=icc, xmp=xmp, dpi=dpi)


def _enc_rec(c):
    from png_check import encmeta_input, imgopt
    name, img, q, flags, bd, ch, meta = c
    opt = imgopt(C.IMGF_JXR, q, flags, bd, chroma=ch)
    if meta:
        return Rec(OP_ENCMETA, 0, encmeta_input(opt, img, C.FMT_ARGB, meta[2], meta[3], C.IMR_INCH, meta[0], b"",
                                                meta[1]), None, name)
    return Rec(it.OP_ENCODE, 0, opt + it.surface_input(img, C.FMT_ARGB), None, name)


def suite_enc(runners, suites, seed, no_ext):
    """encoder: bytes == tools/jxr_enc_ref.py; our decoder: lossless exact (alpha always exact),
    lossy PSNR; WIC decodes every file to the same pixels as our decoder; metadata round trip; errors"""
    from png_check import imgopt
    cs = enc_cases(seed)
    refs = [_enc_ref(c) for c in cs]
    wic_px = {}
    if os.name == "nt" and not no_ext and "win" in runners:
        work = runners["win"].work / "wic_enc"
        work.mkdir(parents=True, exist_ok=True)
        wic = it.Wic(work)
        ids = []
        for c, ref in zip(cs, refs):
            pth = work / f"{c[0]}.jxr"
            pth.write_bytes(ref)
            ids.append(wic.decode(pth))
        wic.run()
        for c, i in zip(cs, ids):
            r = wic.result(i)
            wic_px[c[0]] = None if r is None else r[3]
    for t, R in runners.items():
        s = it.Suite("enc", t)
        res = R.batch([_enc_rec(c) for c in cs])
        dec_recs, dec_cases = [], []
        for c, ref, r in zip(cs, refs, res):
            if r is None:
                s.fail(f"{c[0]}: CRASH")
                continue
            rv, out = r
            if rv != 0:
                s.fail(f"{c[0]}: error {it.err_name(rv)}")
                continue
            out = bytes(out)
            if out != ref:
                n = min(len(out), len(ref))
                d = next((i for i in range(n) if out[i] != ref[i]), n)
                s.fail(f"{c[0]}: bytes differ from jxr_enc_ref at {d} ({len(out)} / {len(ref)} B)")
                continue
            s.ok()
            dec_recs.append(Rec(it.OP_DECEX, 0, it.decex_input(out, flags=STRAIGHT | C.IMGD_ICC | C.IMGD_XMP), None,
                                c[0]))
            dec_cases.append(c)
        for c, r in zip(dec_cases, R.batch(dec_recs)):
            name, img, q, flags, bd, ch, meta = c
            if r is None or r[0] != 0:
                s.fail(f"{name}: decode {'crash' if r is None else it.err_name(r[0])}")
                continue
            d = it.parse_decex(r[1])
            px = d["px"]
            if px is None or px.shape != img.shape:
                s.fail(f"{name}: decoded size {None if px is None else px.shape}")
                continue
            has_a = bd == 32 or (bd != 24 and (img[:, :, 3] != 255).any())
            exp_a = img[:, :, 3] if has_a else np.full(img.shape[:2], 255, np.uint8)
            msg = []
            if not np.array_equal(px[:, :, 3], exp_a):
                msg.append("alpha differs")
            if flags & C.IOF_LOSSLESS:
                if not np.array_equal(px[:, :, :3], img[:, :, :3]):
                    msg.append(f"lossless colour max diff {np.abs(px[:, :, :3].astype(int) - img[:, :, :3]).max()}")
            else:
                p = it.psnr(px[:, :, :3], img[:, :, :3])
                lim = 40 if (q or 90) >= 90 and not ch and not name.startswith("noise") else 18
                if p < lim:
                    msg.append(f"PSNR {p:.1f} < {lim}")
            m = d["meta"]
            if meta and meta[2]:
                if m["dpix"] != meta[2] or m["dpiy"] != meta[3]:
                    msg.append(f"dpi {m['dpix']:#x} / {m['dpiy']:#x}")
            if meta:
                if d["icc"] != meta[0]:
                    msg.append("ICC differs")
                if d["xmp"] != meta[1]:
                    msg.append("XMP differs")
            if name in wic_px:
                w = wic_px[name]
                if w is None:
                    msg.append("WIC could not decode the file")
                elif not np.array_equal(w, px):
                    msg.append(f"WIC differs (max {np.abs(w.astype(int) - px.astype(int)).max()})")
            if msg:
                s.fail(f"{name}: " + ", ".join(msg))
            else:
                s.ok()
        # errors: empty surface
        e = R.batch([Rec(it.OP_ENCODE, 0, imgopt(C.IMGF_JXR, 90) + struct.pack("<III", 0, 0, C.FMT_ARGB),
                         None, "empty")])
        s.check(e[0] is not None and e[0][0] == C.ERR_DIMS, f"empty surface: {e[0] and it.err_name(e[0][0])}")
        if wic_px:
            s.notes.append(f"WIC decoded {sum(v is not None for v in wic_px.values())} / {len(wic_px)} files")
        suites.append(s)


# ================================================================== main

def main():
    ap = it.argparser(__doc__, "wic,native,meta,errors,fuzz,perf,enc")
    a = ap.parse_args()
    out = it.out_dir(a)
    targets = it.targets_of(a)
    if not a.no_build:
        it.build_codectest(targets, out)
    only = set(a.only.split(",")) if a.only else {"wic", "native", "meta", "errors", "fuzz", "perf", "enc"}
    runners = {t: it.Runner(t, out, work_name="jxr") for t in targets}
    cache = out / "jxr_cache"
    cases = []
    if only & {"wic", "native", "meta", "errors", "fuzz"}:
        cases = materialise(build_cases(a.seed), cache)
    suites = []
    if "wic" in only:
        suite_wic(runners, cases, suites)
    if "native" in only:
        suite_native(runners, cases, suites)
    if "meta" in only:
        suite_meta(runners, cases, suites)
    if "errors" in only:
        suite_errors(runners, cases, suites)
    if "fuzz" in only:
        suite_fuzz(runners, cases, suites, a.fuzz, a.seed)
    if "perf" in only:
        suite_perf(runners, cache, suites)
    if "enc" in only:
        suite_enc(runners, suites, a.seed, a.no_ext)
    ok = it.report(suites, "jxr_check")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
