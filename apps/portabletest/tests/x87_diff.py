"""Two x86-64 machines' x87 against each other (docs/INTEL_MAC.md, IM0 and IM4): the dumps of
`portabletest --math-dump` from both, compared in the x87 column (rt::x87) and the portable one.

    python apps/portabletest/tests/x87_diff.py A.txt B.txt

such as Windows x86-64 (build/win/portabletest.exe --math-dump > A.txt) against an Intel Mac or Rosetta 2
(build/mac-x86_64/portabletest --math-dump > B.txt; the job "intel" of .github/workflows/apple.yml uploads
its dump).  Per group of inputs: how many x87 results differ and by how many units in the last place
(integers: the difference).  The x87's transcendental instructions differ between processor makers by a
unit now and then; the decimal conversions (scale, log10_round, scale_round) must not differ.  Exit code 1
when the portable column differs, a dump is broken, or a decimal conversion differs.
"""
import sys
from collections import Counter

INTEGER = {"log10_round", "scale_round"}
DECIMAL = {"scale", "log10_round", "scale_round"}


def signed(v):
    return v - (1 << 64) if v >= 1 << 63 else v


def main():
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    a = [ln.split() for ln in open(sys.argv[1], encoding="utf-8") if ln.strip()]
    b = [ln.split() for ln in open(sys.argv[2], encoding="utf-8") if ln.strip()]
    if len(a) != len(b) or any(len(x) != 5 or x[:3] != y[:3] for x, y in zip(a, b)):
        print("the dumps do not match line by line (other inputs, or broken)")
        return 1
    total, differ, worst, example = Counter(), Counter(), Counter(), {}
    portable = 0
    for x, y in zip(a, b):
        g = x[0]
        total[g] += 1
        portable += x[4] != y[4]
        if x[3] != y[3]:
            differ[g] += 1
            p, q = int(x[3], 16), int(y[3], 16)
            d = abs(signed(p) - signed(q)) if g in INTEGER else abs(p - q)
            worst[g] = max(worst[g], d)
            example.setdefault(g, f"a={x[1]} b={x[2]}: {x[3]} / {y[3]}")
    for g in total:
        print(f"{g:14} {total[g]:6}  differ {differ[g]:5}  max {worst[g]}  {example.get(g, '')}")
    print(f"portable: {portable} differ (must be 0)")
    return 1 if portable or any(differ[g] for g in DECIMAL) else 0


if __name__ == "__main__":
    sys.exit(main())
