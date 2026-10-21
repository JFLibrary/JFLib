"""Additional handbook regression tests. Requires fresh PaintCraft builds, no third-party plugins.

python tools/paintcraft_manual_check.py --out build/paintcraft-parity --target both
Use --pdn-host on Windows to also test the separately built host and installed Paint.NET.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import struct
import sys
import zlib

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import build_rs


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument("--out", default="build/paintcraft-parity")
    parser.add_argument("--target", choices=["win", "linux", "both"], default="both")
    parser.add_argument("--pdn-host", action="store_true")
    args = parser.parse_args()
    os.chdir(ROOT)
    out = Path(args.out).resolve()
    cases = out / "manual-check"
    cases.mkdir(parents=True, exist_ok=True)
    results = []

    def rel(path):
        return path.relative_to(ROOT).as_posix()

    def run(target, name, source, extra=()):
        script = cases / f"{name}-{target}.uis"
        script.write_text(source, encoding="utf-8")
        exe = out / target / ("PaintCraft.exe" if target == "win" else "PaintCraft")
        result = build_rs.run_exe(target, exe, [*extra, "--config", rel(cases / f"config-{name}-{target}"),
                                 "--script", rel(script), "--verify-dirty"], timeout=90)
        if result.returncode:
            raise AssertionError(name + ": " + result.stdout + result.stderr)
        results.append({"target": target, "case": name, "passed": True})

    # Independent PSD fixtures: distinct planar channels and nontrivial alpha.
    w, h = 7, 5
    planes = [bytes((c * 63 + x * 11 + y * 7) % 256 for y in range(h) for x in range(w)) for c in range(4)]
    header = b"8BPS" + struct.pack(">H6xHIIHH", 1, 4, h, w, 8, 3)
    for compression in range(4):
        rows = [p[y*w:(y+1)*w] for p in planes for y in range(h)]
        if compression == 0:
            body = b"".join(planes)
        elif compression == 1:
            packed = [bytes([w-1]) + row for row in rows]
            body = b"".join(struct.pack(">H", len(row)) for row in packed) + b"".join(packed)
        else:
            if compression == 3:
                rows = [bytes([row[0]]) + bytes((row[x] - row[x-1]) % 256 for x in range(1, w)) for row in rows]
            body = zlib.compress(b"".join(rows))
        (cases / f"image-{compression}.psd").write_bytes(header + bytes(12) + struct.pack(">H", compression) + body)
    # A PackBits row claiming more pixels than its row width must be rejected.
    (cases / "bad.psd").write_bytes(header + bytes(12) + b"\0\1" + struct.pack(">H", 2) * (4*h) + bytes([128-w, 1]) * (4*h))

    for target in (["win", "linux"] if args.target == "both" else [args.target]):
        hashes = []
        for mode in range(3):
            shot = cases / f"text-{target}-{mode}.png"
            run(target, f"text-{mode}", f'''size 1280 800
theme light
key t
cmd 840 0
cmd 841 17
cmd 842 {mode << 6}
click 290,198
type "Hamburgefonts AV fi 0123456789"
cmd 790
key b
shot {rel(shot)}
''')
            hashes.append(hashlib.sha256(shot.read_bytes()).hexdigest())
        assert len(set(hashes)) == 3, (target, "Text modes must render differently", hashes)

        for compression in range(4):
            source = "size 1280 800\ntheme light\nexpect #9913 value 7\nexpect #9914 value 5\n"
            for x, y in [(0, 0), (6, 4), (2, 3)]:
                i = y*w+x
                value = (planes[3][i] << 24) | (planes[0][i] << 16) | (planes[1][i] << 8) | planes[2][i]
                source += f'cmd 901 {x | y << 16}\nexpect #9900 text "{value:08X}"\n'
            if compression == 0:
                saved = cases / f"export-{target}.psd"
                # Remove only this test's exact previous output, avoiding overwrite prompts.
                saved.unlink(missing_ok=True)
                source += f'cmd 103\nclick #81932\nkey ctrl+a\ntype "{saved.name}"\nclick button:"Speichern"\n'
            run(target, f"psd-{compression}", source, ["--open", rel(cases / f"image-{compression}.psd")])
            if compression == 0:
                encoded = saved.read_bytes()
                resources_size = struct.unpack_from('>I', encoded, 30)[0]
                layer_offset = 34 + resources_size
                layer_size = struct.unpack_from('>I', encoded, layer_offset)[0]
                assert encoded[:26] == header and encoded[layer_offset+4+layer_size:] == b'\0\0' + b''.join(planes), "PSD writer changed channel/alpha data"
        run(target, "psd-invalid", 'size 1280 800\nexpect #9913 value 800\nexpect #9914 value 600\n',
            ["--open", rel(cases / "bad.psd")])

        source = "size 1280 800\ntheme light\n"
        for cmd in [393, 394, 453, 454, 455, 551, 552, 553, 576, 578]:
            before, after = (cases / f"fx-{target}-{cmd}-{n}.dump" for n in ["preview", "commit"])
            source += f'''cmd {cmd}
sync
key esc
cmd 995 0
expect #9990 text "2cc39813e097b672"
cmd {cmd}
'''
            if cmd == 393:
                source += "cmd 996 5120\n"  # red balance +20
            source += f'''sync
cmd 995 2
dump {rel(before)}
click button:"OK"
sync
cmd 995 0
dump {rel(after)}
key ctrl+z
cmd 995 0
expect #9990 text "2cc39813e097b672"
'''
        run(target, "effects", source, ["--open", "apps/PaintCraft/tests/fixtures/a6_photo.png"])
        def pixel_hash(file):
            entries = [json.loads(line) for line in file.read_text(encoding="utf-8").splitlines() if line.startswith("{")]
            return next(item["t"] for item in entries if item.get("id") == 9990)
        for cmd in [393, 394, 453, 454, 455, 551, 552, 553, 576, 578]:
            assert pixel_hash(cases / f"fx-{target}-{cmd}-preview.dump") == pixel_hash(cases / f"fx-{target}-{cmd}-commit.dump"), cmd

        if args.pdn_host and target == "win":
            script = ROOT / "apps/PaintCraft/tests/integration/pdn_host.uis"
            run(target, "original-plugin", script.read_text(encoding="utf-8"), build_rs.script_args(script))
    (cases / "results.json").write_text(json.dumps(results, indent=2), encoding="utf-8")
    print(f"{len(results)} cases passed; distinct text rendering, PSD pixels and effect preview/commit equality verified.")


if __name__ == "__main__":
    main()
