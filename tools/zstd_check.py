"""zstd_check.py -- lib/compress zstd frames against the reference implementation, both ways.

    python tools/zstd_check.py          (runs in WSL / Linux; needs the `zstd` tool)

Files of several shapes: coretest --zstd c (levels 1, 3, 9, 19) -> zstd -d must restore them,
and zstd at many settings (levels 1..19, --ultra -22, --fast, -T4 multithreaded, --long,
--no-check, a dictionary-free --rsyncable stream, concatenated frames) -> coretest --zstd d
likewise.  Prints the sizes next to the reference's and the speed of both directions.

Dictionaries: 3000 JSON records of a backup log; a dictionary of 16 KiB trained by
`zstd --train` and one by coretest --zstd train on the first 2400; the last 600 compressed
with each dictionary by each side and decompressed by the other (zstd -D), and a raw content
dictionary both ways.  Prints the ratios of the four combinations."""
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
    bad += dictionaries(exe)
    print("zstd_check:", "all ok" if not bad else f"{bad} failures")
    return 1 if bad else 0


def dictionaries(exe):
    import json
    rnd = random.Random(5)
    users = ["anna", "bert", "carla", "dieter", "eva", "frank"]

    def record(i):
        u = rnd.choice(users)
        return json.dumps({
            "id": 100000 + i, "time": f"2026-11-{rnd.randint(1, 28):02}T{rnd.randint(0, 23):02}:{rnd.randint(0, 59):02}:00Z",
            "host": f"srv-{rnd.randrange(20):02}.backup.lan", "user": u,
            "path": f"/home/{u}/" + rnd.choice(["Dokumente", "Bilder", "Musik"]) + "/" + "".join(rnd.choice("abcdefgh") for _ in range(6)),
            "status": rnd.choice(["ok", "ok", "ok", "skipped", "failed: permission denied"]),
            "bytes": rnd.randint(0, 10 ** 9), "chunks": [rnd.randint(0, 99999) for _ in range(rnd.randint(1, 6))],
        }, indent=1).encode()

    fails = []

    def run(*a):
        return subprocess.run([str(x) for x in a]).returncode

    with tempfile.TemporaryDirectory() as d:
        d = Path(d)
        files = []
        for i in range(3000):
            f = d / f"s{i}.json"
            f.write_bytes(record(i))
            files.append(f)
        learn, test = files[:2400], files[2400:]
        zd, od = d / "zstd.dict", d / "ours.dict"
        run("zstd", "-q", "--train", *learn, "-o", zd, "--maxdict=16384")
        if run(exe, "--zstd", "train", od, 16384, *learn) != 0:
            fails.append("ours trains")
        sizes = {k: 0 for k in ("raw", "zstd+zstd's", "ours+zstd's", "zstd+ours", "ours+ours")}
        back, z = d / "back", d / "t.zst"
        for f in test:
            data = f.read_bytes()
            sizes["raw"] += len(data)
            for name, dct in (("zstd's", zd), ("ours", od)):
                run("zstd", "-q", "-f", "-3", "-D", dct, f, "-o", z)
                sizes[f"zstd+{name}"] += z.stat().st_size
                if run(exe, "--zstd", "d", z, back, dct) != 0 or back.read_bytes() != data:
                    fails.append(f"zstd -D {name} -> ours")
                run(exe, "--zstd", "c", f, z, 3, dct)
                sizes[f"ours+{name}"] += z.stat().st_size
                if run("zstd", "-q", "-f", "-d", "-D", dct, z, "-o", back) != 0 or back.read_bytes() != data:
                    fails.append(f"ours -> zstd -d -D {name}")
        raw = d / "raw.dict"
        raw.write_bytes(b"".join(f.read_bytes() for f in learn[:40]))
        data = test[0].read_bytes()
        run(exe, "--zstd", "c", test[0], z, 3, raw)
        if run("zstd", "-q", "-f", "-d", "-D", raw, z, "-o", back) != 0 or back.read_bytes() != data:
            fails.append("raw content ours -> zstd")
        run("zstd", "-q", "-f", "-D", raw, test[0], "-o", z)
        if run(exe, "--zstd", "d", z, back, raw) != 0 or back.read_bytes() != data:
            fails.append("raw content zstd -> ours")
        big = d / "big"
        big.write_bytes(b"".join(f.read_bytes() for f in files))
        for lv in (1, 3, 19):
            run(exe, "--zstd", "c", big, z, lv, od)
            if run("zstd", "-q", "-f", "-d", "-D", od, z, "-o", back) != 0 or back.read_bytes() != big.read_bytes():
                fails.append(f"big with our dictionary, level {lv}")
        r = sizes["raw"]
        print(f"dictionaries ({len(test)} records, {r} bytes): " + "  ".join(f"{k} {r / v:.2f}" for k, v in sizes.items() if k != "raw"))
        print(f"        dictionaries: {'all ok' if not fails else 'FAIL ' + ', '.join(sorted(set(fails)))}")
    return len(fails)


if __name__ == "__main__":
    sys.exit(main())
