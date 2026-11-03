"""Additional handbook regression tests. Requires fresh PaintCraft builds, no third-party plugins.

python tools/paintcraft_manual_check.py --out build/paintcraft-parity --target both
Use --pdn-host on Windows to also test the separately built host and installed Paint.NET.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import time
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
            modern_plugins(out, cases, run)
    (cases / "results.json").write_text(json.dumps(results, indent=2), encoding="utf-8")
    print(f"{len(results)} cases passed; distinct text rendering, PSD pixels and effect preview/commit equality verified.")


MENU = 'size 1280 800\ntheme light\nclick menu:"Effekte"\nclick menuitem:"Paint.NET-Plugins"\n'


def modern_plugins(out, cases, run):
    """v5 effects from a subfolder through the menu, on paint.net's own runtime; what the menu
    says when something is missing; the search of a normal start"""
    exe = out / "win" / "PaintCraft.exe"
    host = out / "win" / "pdn-host" / "PaintCraft.PdnHost.dll"
    root = cases / "pdn-plugins"
    shutil.rmtree(root, ignore_errors=True)
    effects = root / "Effects" / "EffectFixture"
    filetypes = root / "FileTypes"
    # only our two fixtures and the helper land here: Private=false keeps paint.net's files out
    for project, folder in [("EffectFixture", effects), ("Fixture", filetypes)]:
        subprocess.run(["dotnet", "build", f"tools/paintcraft-pdn-host/tests/{project}.csproj", "-c", "Release", "-o", str(folder),
                        "--nologo", "-v:q"], check=True, stdin=subprocess.DEVNULL)
    plugins = ["--pdn-plugins", str(root), "--pdn-isolated"]
    script = ROOT / "apps/PaintCraft/tests/integration/pdn_modern.uis"
    run("win", "modern-effects", script.read_text(encoding="utf-8"), [*plugins, "--pdn-defaults"])
    # WARP, for machines and sessions without a usable GPU, gives the same black
    run("win", "modern-software", MENU + 'click menuitem:"Fixture GPU Image"\nsync 60000\ncmd 901 65537\nexpect #9900 text "FF000000"\n',
        [*plugins, "--pdn-defaults", "--pdn-gpu", "software"])

    # The search of a normal start does not block; effects and formats arrive with its end.
    image = cases / "image.pcfixture"
    image.write_bytes(struct.pack("<5I", 0x54465354, 1, 1, 0xff123456, 0))
    run("win", "modern-background", f'''size 1280 800
theme light
sync 60000
click menu:"Effekte"
click menuitem:"Paint.NET-Plugins"
expect menuitem:"Fixture GPU Image" enabled
key esc
key esc
cmd 101
click #65548
key ctrl+a
type "{image.as_posix()}"
click button:"Öffnen"
sync 60000
expect #9913 value 1
cmd 901 0
expect #9900 text "FF123456"
''', [*plugins, "--pdn-defaults", "--pdn-background"])

    # The reasons the menu gives instead of effects.
    empty = cases / "pdn-empty"
    empty.mkdir(exist_ok=True)
    for name, text, extra in [
        ("missing-paintnet", "paint.net 5.1 ist nicht installiert", ["--pdn-install", str(cases / "no-paint.net")]),
        ("missing-host", "Der Plugin-Host fehlt in dieser Installation", ["--pdn-host", str(cases / "no-host.dll")]),
        ("no-plugins", "Plugins sind abgeschaltet", ["--no-plugins"]),
        ("nothing-found", "(Keine kompatiblen Effekte gefunden)", ["--pdn-plugins", str(empty), "--pdn-isolated"]),
    ]:
        run("win", name, MENU + f'expect menuitem:"{text}" disabled\n', extra)
    broken = cases / "pdn-broken" / "Effects"
    broken.mkdir(parents=True, exist_ok=True)
    (broken / "broken.dll").write_bytes(b"not a DLL")
    run("win", "only-errors", MENU + 'expect menuitem:"Keine Effekte geladen, siehe Einstellungen > Plugin-Fehler" disabled\n',
        ["--pdn-plugins", str(cases / "pdn-broken"), "--pdn-isolated"])

    # The host itself, with no .NET on the PATH: paint.net's runtime is all it needs.
    fixture = effects / "EffectFixture.dll"
    w, h = 4, 4
    packet = struct.pack("<6I", 0x58464350, w, h, 0xffff0000, 0xff0000ff, 0x40000000) + b"\xff" * (w * h * 4) + b"\xff" * (w * h)
    bare = {k: v for k, v in os.environ.items() if k.upper() != "PATH" and not k.upper().startswith("DOTNET_")}
    bare["PATH"] = os.path.join(os.environ["SystemRoot"], "System32")
    install = os.environ.get("PAINTDOTNET_DIR") or os.path.join(os.environ["ProgramFiles"], "paint.net")
    clr = [str(exe), "--pdn-clr", install, str(host)]
    seconds = {}
    for kind, extra, pixel in [("BitmapInvert", [], "000000ff"), ("BitmapFloat", [], "000000ff"), ("BitmapForm", [], "000000ff"), ("BitmapPlain", [], "000000ff"),
                               ("ClassicPlain", [], "000000ff"), ("ClassicLocalized", [], "000000ff"), ("ClassicClipboard", [], "000000ff"),
                               ("BitmapOwnLibrary", [], "000000ff"), ("BitmapServices", [], "000000ff"),
                               ("GpuImage", [], "000000ff"), ("GpuImage", ["--gpu", "software"], "000000ff"), ("GpuDrawing", [], "000000ff"), ("GpuShader", [], "cdcdcdff")]:
        started = time.time()
        result = subprocess.run([*clr, str(fixture), kind, "--defaults", *extra], input=packet, capture_output=True, env=bare, timeout=90)
        assert result.returncode == 0, (kind, result.stderr)
        seconds[kind] = time.time() - started
        assert len(result.stdout) == 12 + w * h * 4 and result.stdout[12:16].hex() == pixel, (kind, extra, result.stdout[:16].hex())
    # paint.net's clipboard retries for 12 seconds when it has no window to open with; measured
    # against a plain effect, because a busy machine slows every process
    assert seconds["ClassicClipboard"] < seconds["ClassicPlain"] + 8, seconds
    for kind in ["BitmapInvert", "BitmapForm", "GpuImage", "ClassicPlain", "ClassicLocalized"]:
        result = subprocess.run([*clr, str(fixture), kind, "--validate-dialog"], capture_output=True, env=bare, stdin=subprocess.DEVNULL, timeout=90)
        assert result.returncode == 0 and not result.stdout, (kind, result.stderr)
        assert kind != "BitmapForm" or b"form-filled:255" in result.stderr, result.stderr
        # the plugin's own words and order, without what it leaves out of its UI
        assert kind != "BitmapInvert" or (b"parameter:Mask=Fixture-Maske" in result.stderr and b"Unlisted" not in result.stderr), result.stderr
        # colors and coordinates as a person can read and type them
        assert kind != "BitmapInvert" or (b"[Int32] #FF8000" in result.stderr and b"; 0" in result.stderr), result.stderr
        assert kind != "ClassicLocalized" or "parameter:Channels=Fixture-Kanäle".encode() in result.stderr, result.stderr
    # A damaged image never leaves result bytes behind.
    result = subprocess.run([*clr, str(fixture), "BitmapInvert", "--defaults"], input=packet[:40], capture_output=True, env=bare, timeout=90)
    assert result.returncode == 1 and not result.stdout, result.stderr

    # The catalog re-enters the same way for every DLL and reads the subfolder.
    catalog = subprocess.run([*clr, "--catalog", "--isolated", "--root", str(root)], capture_output=True, env=bare, stdin=subprocess.DEVNULL, timeout=90)
    assert catalog.returncode == 0, catalog.stderr
    rows = [line.split("\t") for line in catalog.stdout.decode().splitlines()]
    names = {bytes.fromhex(r[3]).decode(): r[4] for r in rows if r[0] == "E"}
    assert names["Fixture Bitmap"] == "1" and names["Fixture GPU Image"] == "2" and len(names) == 14, names
    assert not [r for r in rows if r[0] in "UX"], rows
    assert len([r for r in rows if r[0] == "F"]) == 5, rows
    # Nothing of paint.net is copied next to the host.
    copied = [f.name for f in host.parent.iterdir() if f.name.lower().startswith("paintdotnet")]
    assert not copied, copied


if __name__ == "__main__":
    main()
