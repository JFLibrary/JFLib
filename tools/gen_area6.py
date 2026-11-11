#!/usr/bin/env python3
"""lib/render2d/stroke.wgsl: writes the function area6, unrolled (no loops, no arrays of planes: a GPU pays a
hundred times more for those; docs/RENDER2D.md "Der Shader").

    python tools/gen_area6.py           replaces the function in stroke.wgsl
    python tools/gen_area6.py --check   exit code 1 when the file's function is not what this writes
    python tools/gen_area6.py --print   the function on stdout

The comment above the function stays as it is.  After writing, the SPIR-V follows with
`render2dtest --gen-shaders lib/render2d/shaders_spv.rs`.  Only the Python standard library.
"""
import argparse
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
TARGET = ROOT / "lib" / "render2d" / "stroke.wgsl"
START = "fn area6("
# what follows the function in the file
AFTER = "\n// a half plane with everything inside"
# the planes it takes
N = 6
# the unit square's sides, counter-clockwise: a corner and the direction along the side
SIDES = [("-0.5, -0.5", "1.0, 0.0"), ("0.5, -0.5", "0.0, 1.0"), ("0.5, 0.5", "-1.0, 0.0"), ("-0.5, 0.5", "0.0, -1.0")]
# the square's four sides as half planes (normal, offset): narrowing a cutting plane's line to the square
SQUARE = ["1.0, 0.0, -0.5", "-1.0, 0.0, -0.5", "0.0, 1.0, -0.5", "0.0, -1.0, -0.5"]


def generate() -> str:
    """The text of area6, from `fn` to its closing brace, lines ending in \\n"""
    o: list[str] = []
    w = o.append
    planes = range(N)
    w("fn area6(" + ", ".join(f"p{i}: vec3f" for i in planes) + ") -> f32 {")
    for i in planes:
        w(f"    let k{i} = kind_of(p{i});")
    w("    if " + " || ".join(f"k{i} == 2u" for i in planes) + " {")
    w("        return 0.0;")
    w("    }")
    w("    var n = 0u;")
    for i in planes:
        w(f"    var c{i} = ALL;")
    # the planes that cut the pixel, moved to the front
    for i in planes:
        w(f"    if k{i} == 1u {{")
        for j in planes:
            w(f"        c{j} = select(c{j}, p{i}, n == {j}u);")
        w("        n += 1u;")
        w("    }")
    w("    if n == 0u {")
    w("        return 1.0;")
    w("    }")
    w("    if n == 1u {")
    w("        return cov(c0.z, c0.xy);")
    w("    }")
    w("    if n == 2u && dot(c0.xy, c1.xy) < -0.99999 {")
    w("        return max(cov(c0.z, c0.xy) + cov(c1.z, c1.xy) - 1.0, 0.0);")
    w("    }")
    w("    var a = 0.0;")
    w("    // the square's sides")
    for corner, along in SIDES:
        w("    {")
        w(f"        let sp = vec2f({corner});")
        w(f"        let sd = vec2f({along});")
        w("        var t = vec2f(0.0, 1.0);")
        for j in planes:
            w(f"        if n > {j}u {{")
            w(f"            t = narrow(sp, sd, c{j}, t);")
            w("        }")
        w("        a += piece(sp, sd, t);")
        w("    }")
    w("    // the cutting planes' lines")
    for i in planes:
        w(f"    if n > {i}u {{")
        w(f"        let lp = -c{i}.z * c{i}.xy;")
        w(f"        let ld = vec2f(-c{i}.y, c{i}.x);")
        w("        var t = vec2f(-1.0, 1.0);")
        for side in SQUARE:
            w(f"        t = narrow(lp, ld, vec3f({side}), t);")
        for j in planes:
            if j == i:
                continue
            # (two planes on one line: the earlier one bounds the region, the later one not)
            earlier = "true" if j < i else "false"
            w(f"        if n > {j}u {{")
            w(f"            t = narrow_line(lp, ld, c{i}.xy, c{j}, {earlier}, t);")
            w("        }")
        w("        a += piece(lp, ld, t);")
        w("    }")
    w("    return clamp(0.5 * a, 0.0, 1.0);")
    w("}")
    return "\n".join(o) + "\n"


def locate(text: str) -> tuple[int, int]:
    """Where the function stands in the file's text (only \\n line ends)"""
    a = text.index(START)
    b = text.index(AFTER, a)
    return a, b


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--check", action="store_true", help="compare, change nothing")
    ap.add_argument("--print", action="store_true", dest="show", help="print the function")
    args = ap.parse_args()
    made = generate()
    if args.show:
        sys.stdout.write(made)
        return 0
    raw = TARGET.read_bytes().decode("utf-8")
    crlf = "\r\n" in raw
    text = raw.replace("\r\n", "\n")
    a, b = locate(text)
    if args.check:
        if text[a:b] == made:
            print(f"{TARGET.name}: area6 is what tools/gen_area6.py writes ({made.count(chr(10))} lines)")
            return 0
        print(f"FAIL {TARGET.name}: area6 is not what tools/gen_area6.py writes (python tools/gen_area6.py writes it)")
        return 1
    new = text[:a] + made + text[b:]
    TARGET.write_bytes((new.replace("\n", "\r\n") if crlf else new).encode("utf-8"))
    print(f"{TARGET.name}: area6 written ({made.count(chr(10))} lines)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
