"""lz4_check.py -- lib/compress LZ4 frames against the reference implementation, both ways.

    python tools/lz4_check.py          (runs in WSL / Linux; needs `zstd` built with LZ4 support)

Files of several shapes: coretest --lz4 c -> zstd -d --format=lz4 must restore them, and
zstd --format=lz4 (linked 64 KiB blocks, content checksum) -> coretest --lz4 d likewise."""
import os
import random
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main():
    if os.name == "nt":
        wsl = "/mnt/" + str(ROOT)[0].lower() + str(ROOT)[2:].replace("\\", "/")
        return subprocess.run(["wsl.exe", "-d", "Ubuntu-24.04", "-e", "sh", "-c", f"cd '{wsl}' && python3 tools/lz4_check.py"]).returncode
    exe = ROOT / "build" / "linux" / "coretest"
    rnd = random.Random(7)
    text = (b"Restore-Points werden dedupliziert, komprimiert und verschluesselt. " * 40000)
    shapes = {
        "empty": b"",
        "small": b"abc",
        "text": text,
        "random": rnd.randbytes(700_000),
        "mixed": b"".join(rnd.randbytes(300) + text[:900] for _ in range(3000)),
        "big": bytes(i // 5000 % 256 for i in range(6 << 20)),
    }
    bad = 0
    with tempfile.TemporaryDirectory() as d:
        d = Path(d)
        for name, data in shapes.items():
            src = d / name
            src.write_bytes(data)
            ours = d / (name + ".ours.lz4")
            back = d / (name + ".back")
            r1 = subprocess.run([str(exe), "--lz4", "c", str(src), str(ours)]).returncode
            r2 = subprocess.run(["zstd", "-q", "-f", "-d", "--format=lz4", str(ours), "-o", str(back)]).returncode
            ok1 = r1 == 0 and r2 == 0 and back.read_bytes() == data
            ref = d / (name + ".ref.lz4")
            back2 = d / (name + ".back2")
            r3 = subprocess.run(["zstd", "-q", "-f", "--format=lz4", str(src), "-o", str(ref)]).returncode
            r4 = subprocess.run([str(exe), "--lz4", "d", str(ref), str(back2)]).returncode
            ok2 = r3 == 0 and r4 == 0 and back2.read_bytes() == data
            print(f"{name:7} ours->zstd {'ok' if ok1 else 'FAIL'}   zstd->ours {'ok' if ok2 else 'FAIL'}   "
                  f"{len(data)} -> {ours.stat().st_size if ours.exists() else 0} (zstd lz4: {ref.stat().st_size if ref.exists() else 0})")
            bad += (not ok1) + (not ok2)
    print("lz4_check:", "all ok" if not bad else f"{bad} failures")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
