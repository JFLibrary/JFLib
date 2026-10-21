"""zstd_check.py -- lib/compress zstd frames against the reference implementation, both ways.

    python tools/zstd_check.py          (runs in WSL / Linux; needs the `zstd` tool)

Files of several shapes: coretest --zstd c (levels 1, 3, 9, 19) -> zstd -d must restore them,
and zstd at many settings (levels 1..19, --ultra -22, --fast, -T4 multithreaded, --long,
--no-check, a dictionary-free --rsyncable stream, concatenated frames) -> coretest --zstd d
likewise.  Prints the sizes next to the reference's and the speed of both directions."""
import os
import random
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main():
    if os.name == "nt":
        wsl = "/mnt/" + str(ROOT)[0].lower() + str(ROOT)[2:].replace("\\", "/")
        return subprocess.run(["wsl.exe", "-d", "Ubuntu-24.04", "-e", "sh", "-c", f"cd '{wsl}' && python3 tools/zstd_check.py"]).returncode
    exe = ROOT / "build" / "linux" / "coretest"
    rnd = random.Random(11)
    text = (b"Restore-Points werden dedupliziert, komprimiert und verschluesselt. " * 40000)
    words = [bytes(rnd.choice(b"abcdefghijklmnopqrstuvwxyz") for _ in range(rnd.randint(2, 9))) for _ in range(3000)]
    prose = b" ".join(rnd.choice(words) for _ in range(400_000))
    shapes = {
        "empty": b"",
        "small": b"abc",
        "text": text,
        "prose": prose,
        "random": rnd.randbytes(700_000),
        "mixed": b"".join(rnd.randbytes(300) + text[:900] for _ in range(3000)),
        "binary": bytes((i * 7 + (i >> 9)) % 251 if i % 1000 < 900 else rnd.randrange(256) for i in range(2_000_000)),
        "big": bytes(i // 5000 % 256 for i in range(12 << 20)),
    }
    own = ROOT / "build" / "linux" / "coretest"
    if not own.exists():
        print("zstd_check: build/linux/coretest is missing (python build_rs.py app coretest --target linux)")
        return 2
    bad = 0
    with tempfile.TemporaryDirectory() as d:
        d = Path(d)
        for name, data in shapes.items():
            src = d / name
            src.write_bytes(data)
            # ours -> zstd
            line = [f"{name:7} {len(data):>9}"]
            for level in (1, 3, 9, 19):
                ours = d / f"{name}.{level}.zst"
                back = d / f"{name}.{level}.back"
                t0 = time.perf_counter()
                r1 = subprocess.run([str(exe), "--zstd", "c", str(src), str(ours), str(level)]).returncode
                dt = time.perf_counter() - t0
                r2 = subprocess.run(["zstd", "-q", "-f", "-d", str(ours), "-o", str(back)]).returncode
                ok = r1 == 0 and r2 == 0 and back.read_bytes() == data
                bad += not ok
                ref = d / f"{name}.{level}.ref.zst"
                subprocess.run(["zstd", "-q", "-f", f"-{level}", str(src), "-o", str(ref)])
                mbs = len(data) / 1e6 / max(dt, 1e-6)
                line.append(f"L{level} {'ok' if ok else 'FAIL'} {ours.stat().st_size if ours.exists() else 0}/{ref.stat().st_size} {mbs:.0f}MB/s")
            print("  ".join(line))
            # zstd -> ours
            settings = [["-1"], ["-3"], ["-9"], ["-19"], ["--ultra", "-22"], ["--fast=5"], ["-T4", "-5"], ["--long", "-3"],
                        ["--no-check", "-3"], ["--rsyncable", "-3"], ["-3", "--no-content-size"]]
            fails = []
            for k, s in enumerate(settings):
                ref = d / f"{name}.r{k}.zst"
                back = d / f"{name}.r{k}.back"
                r3 = subprocess.run(["zstd", "-q", "-f", *s, str(src), "-o", str(ref)]).returncode
                t0 = time.perf_counter()
                r4 = subprocess.run([str(exe), "--zstd", "d", str(ref), str(back)]).returncode
                if r3 != 0 or r4 != 0 or back.read_bytes() != data:
                    fails.append(" ".join(s))
            # concatenated frames (zstd writes one per file with -c)
            cat = d / f"{name}.cat.zst"
            with open(cat, "wb") as f:
                for lv in ("-1", "-12"):
                    f.write(subprocess.run(["zstd", "-q", "-c", lv, str(src)], capture_output=True).stdout)
            back = d / f"{name}.cat.back"
            if subprocess.run([str(exe), "--zstd", "d", str(cat), str(back)]).returncode != 0 or back.read_bytes() != data + data:
                fails.append("concatenated")
            bad += len(fails)
            print(f"        zstd->ours: {'all ok' if not fails else 'FAIL ' + ', '.join(fails)}")
    print("zstd_check:", "all ok" if not bad else f"{bad} failures")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
