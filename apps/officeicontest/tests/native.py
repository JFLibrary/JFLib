"""Render every Office icon on both native targets and verify repaint behaviour.

Run after building officeicontest --target both --out build/wordcraft-icons.
The PNG comparisons test the real renderer; they do not regenerate expected art.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
OUT = (ROOT / (sys.argv[1] if len(sys.argv) > 1 else "build/wordcraft-icons")).resolve()
sys.path.insert(0, str(ROOT / "tools"))
from pngutil import read_png


def command(target, args):
    binary = OUT / target / ("officeicontest.exe" if target == "win" else "officeicontest")
    if target == "win" or os.name != "nt":
        return [str(binary), *args]
    def wsl(path):
        value = str(path).replace("\\", "/")
        return "/mnt/" + value[0].lower() + value[2:]
    return ["wsl.exe", "-d", "Ubuntu-24.04", "--cd", wsl(ROOT), "-e", wsl(binary), *args]


def run(target, args):
    result = subprocess.run(command(target, args), cwd=ROOT, text=True, capture_output=True, timeout=180)
    if result.returncode or "UIWARN" in result.stdout or "FAIL" in result.stdout:
        raise AssertionError(f"{target}: {result.stdout}\n{result.stderr}")
    return result.stdout


def verify_palette(path):
    width, height, rgba = read_png(path)
    assert (width, height) == (1280, 1560)
    colors = {bytes(rgba[i:i+3]) for i in range(0, len(rgba), 4)}
    for color in ("3b5bdb", "d13b3b", "f2c811", "2e8b57", "e07b1f", "e86fa0"):
        assert bytes.fromhex(color) in colors, f"missing painted color {color}"
    for left in (408,440):
        # A one-pixel diagonal has partial coverage everywhere. White ink must
        # blend strongly towards white, not disappear into the blue background.
        white = sum(rgba[(y*width+x)*4] > 190 and rgba[(y*width+x)*4+2] > 240
                    for y in range(8,24) for x in range(left,left+16))
        assert white > 4, "on-accent navigation arrow lost its white foreground"
    # Disabled columns must not leak accent/semantic colours, including translucent ink.
    for index in range(261):
        x = 12 + index % 9 * 140 + 96
        y = 44 + index // 9 * 52 + 4
        for yy in range(y, y + 24):
            for xx in range(x, x + 24):
                at = (yy * width + xx) * 4
                red, green, blue = rgba[at:at+3]
                assert red == green == blue, f"disabled icon {index} leaked colour at {xx},{yy}"


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    manifest = []
    for theme in ("light", "dark"):
        for scale in (100, 150, 200):
            images = []
            for target in ("win", "linux"):
                path = OUT / f"atlas-{theme}{scale}-{target}.png"
                run(target, ["--anim", "off", "--theme", theme, "--scale", str(scale),
                             "--shot", path.relative_to(ROOT).as_posix()])
                data = path.read_bytes()
                images.append(data)
                manifest.append({"path": str(path.relative_to(ROOT)), "sha256": hashlib.sha256(data).hexdigest()})
            assert images[0] == images[1], f"native icon render differs: {theme}/{scale}"
            print(f"PASS Win/Linux pixel+PNG identity: {theme} {scale}%", flush=True)
    verify_palette(OUT / "atlas-light100-win.png")
    print("PASS semantic colours and all 261 disabled palettes", flush=True)
    for target in ("win", "linux"):
        output = run(target, ["--anim", "off", "--verify-dirty", "--script", "apps/officeicontest/tests/palette.uis"])
        assert "OFFICE_PALETTE_OK" in output
        print(f"PASS live accent/theme/DPI invalidation: {target}", flush=True)
    (OUT / "icon-manifest.json").write_bytes((json.dumps(manifest, indent=2) + "\n").encode())


if __name__ == "__main__":
    main()
