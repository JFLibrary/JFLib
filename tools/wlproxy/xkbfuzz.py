#!/usr/bin/env python3
# xkbfuzz.py -- run inside WSL from the worktree root: mutate a keymap, parse it with the
# assembly and the Rust linuxtest (--test xkb --xkbdump), compare the outputs (timing masked)
import os, random, re, subprocess, sys

src = open("build/plat-wayland/keymap.xkb", "rb").read()
n = int(sys.argv[1]) if len(sys.argv) > 1 else 200
rng = random.Random(int(sys.argv[2]) if len(sys.argv) > 2 else 1)
SPECIAL = b"{}[];,<>\"/*#\n\\=x0123456789_ aAzZ+()"
os.makedirs("build/plat-wayland/fz", exist_ok=True)
bad = 0


def run(exe, f):
    r = subprocess.run([exe, "--shot", "build/plat-wayland/fz/x.png", "--test", "xkb", "--keymap", f, "--xkbdump"],
                       capture_output=True, timeout=30)
    out = re.sub(rb"in \d+ us", b"in T us", r.stdout)
    return r.returncode, out


for i in range(n):
    d = bytearray(src)
    k = rng.randrange(6)
    if k == 0:
        d = d[: rng.randrange(len(d))]
    elif k == 1:
        for _ in range(rng.randrange(1, 40)):
            d[rng.randrange(len(d))] = rng.choice(SPECIAL)
    elif k == 2:
        a = rng.randrange(len(d))
        b = min(len(d), a + rng.randrange(1, 3000))
        del d[a:b]
    elif k == 3:
        a = rng.randrange(len(d))
        b = min(len(d), a + rng.randrange(1, 500))
        c = rng.randrange(len(d))
        d[c:c] = d[a:b]
    elif k == 4:
        for _ in range(rng.randrange(1, 10)):
            p = rng.randrange(len(d))
            d[p:p] = rng.choice([b"/*", b"*/", b"//", b"#", b"\"", b"<", b">", b"0x", b"U20AC", b"KP_", b"\\", b"\0"])
    else:
        for _ in range(rng.randrange(1, 200)):
            d[rng.randrange(len(d))] = rng.randrange(256)
    f = f"build/plat-wayland/fz/k{i}.xkb"
    open(f, "wb").write(d)
    a = run("./build/rwref/linux/linuxtest", f)
    b = run("./build/linux/linuxtest", f)
    if a != b:
        bad += 1
        print("DIFF", f, a[0], b[0])
        al, bl = a[1].splitlines(), b[1].splitlines()
        for x, y in zip(al, bl):
            if x != y:
                print("  asm:", x[:120])
                print("  rs: ", y[:120])
                break
        if len(al) != len(bl):
            print("  lines", len(al), len(bl))
    else:
        os.remove(f)
print(f"{n} variants, {bad} differ")
