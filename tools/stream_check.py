"""stream_check.py -- compress::stream against the reference tools, both ways.

    python tools/stream_check.py        (runs in WSL / Linux; needs `gzip` and `zstd`, the
                                         latter built with LZ4 support; Python's zlib)

coretest --stream FORMAT c|d IN OUT reads and writes its files in pieces of 64 KiB through
the stream encoders and decoders (lib/compress/stream.rs).  For files of several shapes:
  - ours -> the tool: gzip -d, Python's zlib (zlib and raw DEFLATE), zstd -d,
    zstd -d --format=lz4 must restore them;
  - the tool -> ours: gzip -1 / -6 / -9 and two members in a row, Python's zlib with and
    without Z_SYNC_FLUSH points, zstd at levels 1 to 19, --ultra -22, --long=27, --fast, -T4,
    --no-check, from a pipe (no content size), two frames in a row, zstd --format=lz4;
  - Brotli: the streams inside the WOFF2 fonts of the system (/usr/share/fonts*), decoded
    in pieces, must equal the one-shot decoder's output (--stream brotli d1) and the length
    the font's table directory gives.
Prints the speed of ours next to the tool's."""
import os
import random
import struct
import subprocess
import sys
import tempfile
import time
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def shapes():
    rnd = random.Random(12)
    text = b"Restore-Points werden dedupliziert, komprimiert und verschluesselt. " * 40000
    words = [bytes(rnd.choice(b"abcdefghijklmnopqrstuvwxyz") for _ in range(rnd.randint(2, 9))) for _ in range(3000)]
    prose = b" ".join(rnd.choice(words) for _ in range(400_000))
    return {
        "empty": b"",
        "small": b"abc",
        "text": text,
        "prose": prose,
        "random": rnd.randbytes(700_000),
        "mixed": b"".join(rnd.randbytes(300) + text[:900] for _ in range(3000)),
        "big": bytes(i // 5000 % 256 for i in range(20 << 20)),
    }


def run(args, **kw):
    return subprocess.run([str(a) for a in args], **kw).returncode


def woff2_brotli(path):
    """the Brotli stream of a WOFF2 font and the length of its decoded tables (None: no WOFF2)"""
    b = path.read_bytes()
    if b[:4] != b"wOF2" or b[4:8] == b"ttcf":
        return None
    num = struct.unpack(">H", b[12:14])[0]
    comp = struct.unpack(">I", b[20:24])[0]
    i, total = 48, 0

    def base128(i):
        v = 0
        for _ in range(5):
            c = b[i]
            i += 1
            v = v << 7 | (c & 127)
            if c & 128 == 0:
                return v, i
        raise ValueError

    for _ in range(num):
        flags = b[i]
        i += 1
        tag = flags & 63
        if tag == 63:
            i += 4
        orig, i = base128(i)
        ver = (flags >> 6) & 3
        transformed = (ver == 0) if tag in (10, 11) else (ver != 0)
        if transformed:
            orig, i = base128(i)
        total += orig
    return b[i:i + comp], total


def main():
    if os.name == "nt":
        wsl = "/mnt/" + str(ROOT)[0].lower() + str(ROOT)[2:].replace("\\", "/")
        return subprocess.run(["wsl.exe", "-d", "Ubuntu-24.04", "-e", "sh", "-c", f"cd '{wsl}' && python3 tools/stream_check.py"]).returncode
    exe = ROOT / "build" / "linux" / "coretest"
    if not exe.exists():
        print("stream_check: build/linux/coretest is missing (python build_rs.py app coretest --target linux)")
        return 2
    bad = 0

    def ok(cond, what):
        nonlocal bad
        if not cond:
            bad += 1
            print("FAIL", what)

    with tempfile.TemporaryDirectory() as d:
        d = Path(d)

        def ours(fmt, op, src, dst, level=None):
            t0 = time.perf_counter()
            r = run([exe, "--stream", fmt, op, src, dst] + ([level] if level is not None else []))
            return r == 0, time.perf_counter() - t0

        for name, data in shapes().items():
            src = d / name
            src.write_bytes(data)
            line = [f"{name:6} {len(data):>9}"]
            # gzip
            for level in (1, 6, 9):
                o, back = d / f"{name}.{level}.gz", d / f"{name}.back"
                good, dt = ours("gzip", "c", src, o, level)
                r = run(["gzip", "-dc", o], stdout=open(back, "wb"))
                ok(good and r == 0 and back.read_bytes() == data, f"gzip {name} -{level}: ours -> gzip -d")
                if level == 6:
                    line.append(f"gzip c {len(data) / 1e6 / max(dt, 1e-9):6.0f} MB/s")
                ref = d / f"{name}.{level}.ref.gz"
                run(["gzip", f"-{level}", "-n", "-c", src], stdout=open(ref, "wb"))
                good, dt = ours("gzip", "d", ref, back)
                ok(good and back.read_bytes() == data, f"gzip {name}: gzip -{level} -> ours")
                if level == 6:
                    line.append(f"d {len(data) / 1e6 / max(dt, 1e-9):5.0f} MB/s")
            two = d / f"{name}.two.gz"
            two.write_bytes(gzip_of(data[: len(data) // 2]) + gzip_of(data[len(data) // 2:]))
            good, _ = ours("gzip", "d", two, back)
            ok(good and back.read_bytes() == data, f"gzip {name}: two members")
            # zlib and raw DEFLATE against Python's zlib, with sync points
            for fmt, wbits in (("zlib", 15), ("deflate", -15)):
                o = d / f"{name}.{fmt}"
                good, _ = ours(fmt, "c", src, o, 6)
                dz = zlib.decompressobj(wbits)
                ok(good and dz.decompress(o.read_bytes()) + dz.flush() == data and dz.eof, f"{fmt} {name}: ours -> zlib")
                co = zlib.compressobj(9, zlib.DEFLATED, wbits)
                parts = []
                for k in range(0, len(data), 100_000):
                    parts.append(co.compress(data[k:k + 100_000]))
                    parts.append(co.flush(zlib.Z_SYNC_FLUSH))
                parts.append(co.flush())
                ref = d / f"{name}.ref.{fmt}"
                ref.write_bytes(b"".join(parts))
                good, _ = ours(fmt, "d", ref, back)
                ok(good and back.read_bytes() == data, f"{fmt} {name}: zlib with Z_SYNC_FLUSH -> ours")
            # zstd
            for level in (1, 3, 19):
                if len(data) > 4 << 20 and level == 19:
                    continue
                o = d / f"{name}.{level}.zst"
                good, dt = ours("zstd", "c", src, o, level)
                r = run(["zstd", "-q", "-f", "-d", o, "-o", back])
                ok(good and r == 0 and back.read_bytes() == data, f"zstd {name} -{level}: ours -> zstd -d")
                if level == 3:
                    line.append(f"zstd c {len(data) / 1e6 / max(dt, 1e-9):5.0f} MB/s")
            for opts in (["-1"], ["-3"], ["-19"], ["--ultra", "-22"], ["--long=27"], ["--fast=3"], ["-T4", "-6"], ["--no-check"]):
                if len(data) > 4 << 20 and opts in (["-19"], ["--ultra", "-22"]):
                    continue
                ref = d / f"{name}.ref.zst"
                run(["zstd", "-q", "-f", *opts, src, "-o", ref])
                good, dt = ours("zstd", "d", ref, back)
                ok(good and back.read_bytes() == data, f"zstd {name}: zstd {' '.join(opts)} -> ours")
                if opts == ["-3"]:
                    line.append(f"d {len(data) / 1e6 / max(dt, 1e-9):5.0f} MB/s")
            ref = d / f"{name}.pipe.zst"
            subprocess.run(["zstd", "-q", "-c"], input=data, stdout=open(ref, "wb"))
            good, _ = ours("zstd", "d", ref, back)
            ok(good and back.read_bytes() == data, f"zstd {name}: from a pipe (no content size) -> ours")
            two = d / f"{name}.two.zst"
            two.write_bytes(subprocess.run(["zstd", "-q", "-c"], input=data[:100], capture_output=True).stdout + ref.read_bytes())
            good, _ = ours("zstd", "d", two, back)
            ok(good and back.read_bytes() == data[:100] + data, f"zstd {name}: two frames")
            # LZ4 frames
            o = d / f"{name}.lz4"
            good, _ = ours("lz4", "c", src, o)
            r = run(["zstd", "-q", "-f", "-d", "--format=lz4", o, "-o", back])
            ok(good and r == 0 and back.read_bytes() == data, f"lz4 {name}: ours -> zstd --format=lz4 -d")
            ref = d / f"{name}.ref.lz4"
            run(["zstd", "-q", "-f", "--format=lz4", src, "-o", ref])
            good, _ = ours("lz4", "d", ref, back)
            ok(good and back.read_bytes() == data, f"lz4 {name}: zstd --format=lz4 -> ours")
            print("  ".join(line))
        # Brotli: the streams of WOFF2 fonts
        fonts = sorted(p for root in ("/usr/share/fonts", "/usr/share/fonts-hack", "/usr/share/fonts-ibm-plex") if Path(root).exists() for p in Path(root).rglob("*.woff2"))
        n = 0
        for f in fonts[:60]:
            got = woff2_brotli(f)
            if got is None:
                continue
            stream, size = got
            src = d / "font.br"
            src.write_bytes(stream)
            a, b = d / "font.stream", d / "font.one"
            g1, _ = ours("brotli", "d", src, a)
            g2, _ = ours("brotli", "d1", src, b)
            ok(g1 and g2 and a.read_bytes() == b.read_bytes() and a.stat().st_size == size, f"brotli {f.name}: in pieces as at once")
            n += 1
        print(f"brotli: {n} WOFF2 fonts" + ("" if n else " (none found)"))
    print("stream_check:", "ok" if bad == 0 else f"{bad} failed")
    return 1 if bad else 0


def gzip_of(data):
    return subprocess.run(["gzip", "-n", "-c"], input=data, capture_output=True).stdout


if __name__ == "__main__":
    sys.exit(main())
