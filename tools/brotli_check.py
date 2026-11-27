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
Prints our sizes next to libbrotli's at the same level.

    python tools/brotli_check.py --vectors [--target linux|win|mac]   (no libbrotli needed)

google/brotli's own test data (tests/testdata at a pinned commit, 3.4 MB without bb.binast, an
encoder's input of 12 MB that has no stream beside it; tools/suite_fetch.py into build/vectors,
read as data only): every *.compressed* stream -- the reference encoder's at every quality and
window, empty streams of every window size, metadata blocks, the 64 KiB backward reference,
streams of the static dictionary's transforms -- must decode with decompress() (d1) and the
stream decoder (d) to its original; every original must come back through compress() at levels
0, 5, 9 and 11 and the stream encoder at 5 (c1, c)."""
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


VECTORS_REPO = "google/brotli"
VECTORS_COMMIT = "717514a67b3f8abf53365d4471a102a60fff8ef5"
VECTORS = ['10x10y', '10x10y.compressed', '64x', '64x.compressed', 'alice29.txt', 'alice29.txt.compressed', 'asyoulik.txt', 'asyoulik.txt.compressed', 'backward65536', 'backward65536.compressed', 'compressed_file', 'compressed_file.compressed', 'compressed_repeated', 'compressed_repeated.compressed', 'cp1251-utf16le', 'cp1251-utf16le.compressed', 'cp852-utf8', 'cp852-utf8.compressed', 'empty', 'empty.compressed', 'empty.compressed.00', 'empty.compressed.01', 'empty.compressed.02', 'empty.compressed.03', 'empty.compressed.04', 'empty.compressed.05', 'empty.compressed.06', 'empty.compressed.07', 'empty.compressed.08', 'empty.compressed.09', 'empty.compressed.10', 'empty.compressed.11', 'empty.compressed.12', 'empty.compressed.13', 'empty.compressed.14', 'empty.compressed.15', 'empty.compressed.16', 'empty.compressed.17', 'empty.compressed.18', 'lcet10.txt', 'lcet10.txt.compressed', 'mapsdatazrh', 'mapsdatazrh.compressed', 'monkey', 'monkey.compressed', 'plrabn12.txt', 'plrabn12.txt.compressed', 'quickfox', 'quickfox.compressed', 'quickfox_repeated', 'quickfox_repeated.compressed', 'random_chunks', 'random_org_10k.bin', 'random_org_10k.bin.compressed', 'ukkonooa', 'ukkonooa.compressed', 'x', 'x.compressed', 'x.compressed.00', 'x.compressed.01', 'x.compressed.02', 'x.compressed.03', 'xyzzy', 'xyzzy.compressed', 'zeros', 'zeros.compressed', 'zerosukkanooa', 'zerosukkanooa.compressed']


def original_of(name):
    """the original a test stream is of: x.compressed.07 -> x"""
    return name.split(".compressed", 1)[0] if ".compressed" in name else None


def vectors(target):
    """google/brotli's test streams through our decoders, their originals through our encoders"""
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from suite_fetch import fetch
    exe = ROOT / "build" / target / ("coretest.exe" if target == "win" else "coretest")
    if not exe.exists():
        print(f"brotli_check: {exe.relative_to(ROOT)} is missing (python build_rs.py app coretest --target {target})")
        return 2
    folder = fetch(VECTORS_REPO, VECTORS_COMMIT, ["tests/testdata/" + f for f in VECTORS]) / "tests" / "testdata"

    def ours(op, src, dst, level=None):
        return subprocess.run([str(exe), "--stream", "brotli", op, str(src), str(dst)] + ([str(level)] if level is not None else [])).returncode == 0

    bad, streams, originals = 0, 0, 0
    with tempfile.TemporaryDirectory() as d:
        d = Path(d)
        for name in VECTORS:
            orig = original_of(name)
            if orig is None:
                continue
            want = (folder / orig).read_bytes()
            fails = []
            for op in ("d1", "d"):
                back = d / "back"
                if not ours(op, folder / name, back) or back.read_bytes() != want:
                    fails.append(op)
            streams += 1
            bad += bool(fails)
            if fails:
                print(f"FAIL {name}: {', '.join(fails)} does not give {orig} ({len(want)} bytes)")
        for name in VECTORS:
            if original_of(name) is not None:
                continue
            data = (folder / name).read_bytes()
            fails = []
            for op, level in (("c1", 0), ("c1", 5), ("c1", 9), ("c1", 11), ("c", 5)):
                o, back = d / "o.br", d / "back"
                if not ours(op, folder / name, o, level) or not ours("d1", o, back) or back.read_bytes() != data:
                    fails.append(f"{op} {level}")
            originals += 1
            bad += bool(fails)
            if fails:
                print(f"FAIL {name}: {', '.join(fails)} does not come back")
    print(f"brotli_check --vectors: {VECTORS_REPO} @ {VECTORS_COMMIT[:12]}: {streams} streams decoded, {originals} originals encoded:",
          "all ok" if not bad else f"{bad} failures")
    return 1 if bad else 0


def main():
    if "--vectors" in sys.argv:
        i = sys.argv.index("--target") + 1 if "--target" in sys.argv else 0
        return vectors(sys.argv[i] if i else "win" if os.name == "nt" else "mac" if sys.platform == "darwin" else "linux")
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
