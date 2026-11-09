"""What ARM64 computes differently from x86-64 in rt::x87 (docs/ARM64.md, ARM2): ARM64 has no x87 unit
and takes rt/portable/math.rs.

    python apps/portabletest/tests/math_diff.py [--x86 EXE] [--arm EXE]

Runs `portabletest --math-dump` once for x86-64 (default build/win/portabletest.exe) and once for ARM64
(default build/linux-arm64/portabletest, under qemu-aarch64 in WSL through tools/host_exec.py) and
compares them line by line.  Per group of inputs: how many results are the same bits, and how far the
others are off in units in the last place (integers: the difference).  As a reference for which of
the two is nearer the true value: Python's math module (the C library's sin, cos, atan2, exp2, log2,
pow, within about one ulp), how far each side is from it.  Build both first:

    python build_rs.py app portabletest --target win
    python build_rs.py app portabletest --target linux --arch aarch64

Exit code 1 when the portable code itself gives other bits on the two processors (or a dump is
broken): then the measurement does not hold.  The deviations themselves are information.
"""
import argparse
import math
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from tools.host_exec import run_native  # noqa: E402

INTEGER = {"log10_round", "scale_round"}
REFERENCE = {"exp2": lambda a, b: math.exp2(a), "log2": lambda a, b: math.log2(a), "pow": math.pow,
             "sin": lambda a, b: math.sin(a), "cos": lambda a, b: math.cos(a), "atan2": math.atan2}
REFERENCE.update({"exp2_unit": REFERENCE["exp2"], "rz_exp2": REFERENCE["exp2"], "log2_unit": REFERENCE["log2"],
                  "rz_log2": REFERENCE["log2"], "pow_gamma": REFERENCE["pow"], "sincos_sin": REFERENCE["sin"],
                  "sin_wide": REFERENCE["sin"], "sincos_cos": REFERENCE["cos"], "cos_wide": REFERENCE["cos"]})


def dump(exe):
    target = "win" if exe.suffix == ".exe" else "linux"
    r = run_native(target, exe, ["--math-dump"], timeout=1800)
    if r.returncode:
        raise SystemExit(f"FAIL {exe} --math-dump: exit {r.returncode}\n{r.stderr}")
    rows = [ln.split() for ln in r.stdout.splitlines() if ln.strip()]
    if not rows or any(len(row) != 5 for row in rows):
        raise SystemExit(f"FAIL {exe} --math-dump: unexpected output")
    return [(row[0], *(int(v, 16) for v in row[1:])) for row in rows]


def ordered(bits):
    """f64 bits on a line where neighbouring doubles differ by 1"""
    return -(bits & ~(1 << 63)) if bits >> 63 else bits


def signed(v):
    return v - (1 << 64) if v >> 63 else v


def is_nan(bits):
    return bits & 0x7FF0_0000_0000_0000 == 0x7FF0_0000_0000_0000 and bits & 0x000F_FFFF_FFFF_FFFF != 0


def f64(bits):
    return struct.unpack("<d", struct.pack("<Q", bits))[0]


def bits(v):
    return struct.unpack("<Q", struct.pack("<d", v))[0]


def reference(group, a, b):
    """the C library's result for these inputs as f64 bits (None: no reference for the group)"""
    if group not in REFERENCE:
        return None
    try:
        return bits(REFERENCE[group](f64(a), f64(b)))
    except OverflowError:
        return bits(math.inf)
    except ValueError:
        return bits(math.nan)


def distance(group, a, b):
    if group in INTEGER:
        return abs(signed(a) - signed(b))
    if is_nan(a) or is_nan(b):
        return 0 if is_nan(a) and is_nan(b) else None
    return abs(ordered(a) - ordered(b))


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--x86", default=str(ROOT / "build" / "win" / "portabletest.exe"))
    ap.add_argument("--arm", default=str(ROOT / "build" / "linux-arm64" / "portabletest"))
    a = ap.parse_args()
    x86, arm = dump(Path(a.x86)), dump(Path(a.arm))
    if len(x86) != len(arm):
        raise SystemExit(f"FAIL the dumps differ in length: {len(x86)} and {len(arm)} lines")
    groups, other_bits = {}, 0
    for (g, xa, xb, x87, x86_portable), (ga, aa, ab, arm_x87, arm_portable) in zip(x86, arm):
        if (g, xa, xb) != (ga, aa, ab):
            raise SystemExit(f"FAIL the dumps have other inputs: {g} {xa:x} {xb:x} / {ga} {aa:x} {ab:x}")
        if arm_x87 != arm_portable or x86_portable != arm_portable:
            other_bits += 1                    # ARM64's rt::x87 is portable/math.rs, the same code as on x86-64
        s = groups.setdefault(g, {"n": 0, "same": 0, "dist": [], "nan": 0, "abs": 0.0, "ref_x86": [], "ref_arm": []})
        s["n"] += 1
        if g not in INTEGER and math.isfinite(f64(x87)) and math.isfinite(f64(arm_x87)):
            s["abs"] = max(s["abs"], abs(f64(x87) - f64(arm_x87)))
        ref = reference(g, xa, xb)
        if ref is not None:
            for key, v in (("ref_x86", x87), ("ref_arm", arm_x87)):
                d = distance(g, v, ref)
                s[key].append(d if d is not None else float("inf"))
        d = distance(g, x87, arm_x87)
        if d is None:
            s["nan"] += 1
        elif d == 0:
            s["same"] += 1
        else:
            s["dist"].append(d)
    def stats(values):
        v = sorted(values)
        return (v[-1], v[len(v) // 2], v[int(len(v) * 0.99)]) if v else ("-", "-", "-")
    print("ARM64 against x86-64, where the bits differ: ulp (integers: the difference) and the largest absolute difference")
    print(f"{'group':12} {'inputs':>6} {'same bits':>10} {'max':>11} {'median':>7} {'99 %':>9} {'NaN one':>7} {'max abs':>9}")
    for g, s in groups.items():
        mx, med, p99 = stats(s["dist"])
        ab = f"{s['abs']:.1e}" if g not in INTEGER and s["dist"] else "-"
        print(f"{g:12} {s['n']:6} {100 * s['same'] / s['n']:9.2f}% {mx:>11} {med:>7} {p99:>9} {s['nan']:7} {ab:>9}")
    print()
    print("how far each side is from the C library (ulp)")
    print(f"{'group':12} {'x86 max':>10} {'99 %':>6} {'ARM max':>11} {'99 %':>8}")
    for g, s in groups.items():
        if s["ref_x86"]:
            rx, ra = stats(s["ref_x86"]), stats(s["ref_arm"])
            print(f"{g:12} {rx[0]:>10} {rx[2]:>6} {ra[0]:>11} {ra[2]:>8}")
    if other_bits:
        print(f"FAIL portable/math.rs gives other bits on ARM64 than on x86-64 in {other_bits} lines")
        return 1
    print(f"portable/math.rs: the same bits on both processors in all {len(arm)} lines")
    return 0


if __name__ == "__main__":
    sys.exit(main())
