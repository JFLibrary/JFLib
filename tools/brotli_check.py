"""brotli_check.py -- lib/compress Brotli streams against libbrotli, both ways.

    python tools/brotli_check.py        (runs in WSL / Linux; needs libbrotlienc and
                                         libbrotlidec, the shared libraries of the brotli
                                         package, which Python reaches through ctypes)

Files of several shapes, the guide among them (text with the words of the static dictionary):
  - ours -> libbrotli: coretest --stream brotli c1 (compress()) at levels 0 to 11 and c (the
    stream encoder, without a size) at levels 1, 5 and 9; BrotliDecoderDecompress must
    restore them;
  - libbrotli -> ours: BrotliEncoderCompress at qualities 0 to 11, windows of 10, 16, 22 and
    24 bits, the modes generic, text and font; coretest --stream brotli d1 (decompress()) and
    d (the stream decoder) must restore them.
Prints our sizes next to libbrotli's at the same level."""
import ctypes
import ctypes.util
import os
import random
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def libbrotli():
    """(encoder, decoder) of the system's libbrotli, or None"""
    try:
        enc = ctypes.CDLL(ctypes.util.find_library("brotlienc") or "libbrotlienc.so.1")
        dec = ctypes.CDLL(ctypes.util.find_library("brotlidec") or "libbrotlidec.so.1")
    except OSError:
        return None
    enc.BrotliEncoderCompress.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_size_t, ctypes.c_char_p,
                                          ctypes.POINTER(ctypes.c_size_t), ctypes.c_char_p]
    dec.BrotliDecoderDecompress.argtypes = [ctypes.c_size_t, ctypes.c_char_p, ctypes.POINTER(ctypes.c_size_t), ctypes.c_char_p]
    return enc, dec


def shapes():
    rnd = random.Random(14)
    text = b"Restore-Points werden dedupliziert, komprimiert und verschluesselt. " * 20000
    words = [bytes(rnd.choice(b"abcdefghijklmnopqrstuvwxyz") for _ in range(rnd.randint(2, 9))) for _ in range(3000)]
    prose = b" ".join(rnd.choice(words) for _ in range(150_000))
    guide = (ROOT / "docs" / "GUIDE.md").read_bytes()
    return {
        "empty": b"",
        "small": b"abc",
        "words": guide[:2000],
        "guide": guide,
        "text": text,
        "prose": prose,
        "random": rnd.randbytes(300_000),
        "mixed": b"".join(rnd.randbytes(300) + text[:900] for _ in range(1500)),
        "binary": bytes((i * 7 + (i >> 9)) % 251 if i % 1000 < 900 else rnd.randrange(256) for i in range(1_500_000)),
        "big": bytes(i // 5000 % 256 for i in range(20 << 20)),
    }


def main():
    if os.name == "nt":
        wsl = "/mnt/" + str(ROOT)[0].lower() + str(ROOT)[2:].replace("\\", "/")
        return subprocess.run(["wsl.exe", "-d", "Ubuntu-24.04", "-e", "sh", "-c", f"cd '{wsl}' && python3 tools/brotli_check.py"]).returncode
    exe = ROOT / "build" / "linux" / "coretest"
    if not exe.exists():
        print("brotli_check: build/linux/coretest is missing (python build_rs.py app coretest --target linux)")
        return 2
    lib = libbrotli()
    if lib is None:
        print("brotli_check: libbrotlienc / libbrotlidec not found (the brotli package)")
        return 2
    enc, dec = lib

    def ref_decode(b, n):
        out = ctypes.create_string_buffer(n + 1)
        size = ctypes.c_size_t(n + 1)
        ok = dec.BrotliDecoderDecompress(len(b), b, ctypes.byref(size), out) == 1
        return out.raw[:size.value] if ok else None

    def ref_encode(data, quality, lgwin, mode=0):
        cap = len(data) + len(data) // 8 + 1024
        out = ctypes.create_string_buffer(cap)
        size = ctypes.c_size_t(cap)
        assert enc.BrotliEncoderCompress(quality, lgwin, mode, len(data), data, ctypes.byref(size), out) == 1
        return out.raw[:size.value]

    def ours(op, src, dst, level=None):
        return subprocess.run([str(exe), "--stream", "brotli", op, str(src), str(dst)] + ([str(level)] if level is not None else [])).returncode == 0

    bad = 0
    totals = {}
    with tempfile.TemporaryDirectory() as d:
        d = Path(d)
        for name, data in shapes().items():
            src = d / name
            src.write_bytes(data)
            fails = []
            sizes = []
            for level in range(12):
                if len(data) > 4 << 20 and level not in (0, 5, 9):
                    continue
                o = d / f"{name}.{level}.br"
                good = ours("c1", src, o, level)
                b = o.read_bytes() if good else b""
                if not good or ref_decode(b, len(data)) != data:
                    fails.append(f"c1 level {level}")
                ref = ref_encode(data, level, 24 if level >= 10 else 22)
                sizes.append(f"{level}:{len(b)}/{len(ref)}")
                t = totals.setdefault(level, [0, 0])
                t[0] += len(b)
                t[1] += len(ref)
            for level in (1, 5, 9):
                o = d / f"{name}.s{level}.br"
                good = ours("c", src, o, level)
                if not good or ref_decode(o.read_bytes(), len(data)) != data:
                    fails.append(f"stream level {level}")
            # libbrotli -> ours
            for q in range(12):
                for lgwin in (10, 16, 22, 24):
                    if len(data) > 4 << 20 and (q > 9 or lgwin != 22):
                        continue
                    for mode in (0, 1, 2):
                        if mode and lgwin != 22:
                            continue
                        r = d / f"{name}.ref.br"
                        r.write_bytes(ref_encode(data, q, lgwin, mode))
                        for op in ("d1", "d"):
                            back = d / f"{name}.back"
                            good = ours(op, r, back)
                            if not good or back.read_bytes() != data:
                                fails.append(f"q{q} w{lgwin} mode {mode} -> {op}")
            bad += len(fails)
            print(f"{name:7} {len(data):>9}  ours/libbrotli {' '.join(sizes)}  {'ok' if not fails else 'FAIL ' + ', '.join(fails[:6])}", flush=True)
    print("level       ours  libbrotli  ours/libbrotli (all shapes)")
    for level, (a, b) in sorted(totals.items()):
        print(f"{level:5} {a:>10} {b:>10} {a / max(b, 1):9.3f}")
    print("brotli_check:", "all ok" if not bad else f"{bad} failures")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
