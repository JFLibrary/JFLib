"""Keep the actual document pixels through Office tab/theme changes.

Build jfWord for both native targets first, then run:
    python apps/jfWord/tests/visual.py --out build/wordcraft-design

The test waits for each headless process to finish. It compares the complete
visible paper interior with the initial Home image at the same DPI and zoom,
using the page/scroll rectangles from the widget dump. Office chrome, rulers,
paper borders and shadows deliberately fall outside that document comparison.
No third-party Python packages or golden images are required.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "tools"))
from pngutil import read_png, write_png

PAGE_ID = 0x300000
SCROLL_ID = 110
WINDOW_SIZE = (1424, 884)
STAGES = (
    ("home-light", "Start", "light", ()),
    ("references-light", "Referenzen", "light", ('click tab:"Referenzen"',)),
    ("layout-light", "Layout", "light", ('click tab:"Layout"',)),
    ("design-light", "Entwurf", "light", ('click tab:"Entwurf"',)),
    ("design-dark", "Entwurf", "dark", ("theme dark",)),
    ("insert-dark", "Einfügen", "dark", ('click tab:"Einfügen"',)),
    ("home-dark", "Start", "dark", ('click tab:"Start"',)),
    ("home-light-return", "Start", "light", ("theme light",)),
)


def script_path(path):
    """The script's dump command takes an unquoted path relative to RustLib."""
    value = path.relative_to(ROOT).as_posix()
    if any(char.isspace() for char in value):
        raise ValueError("Use an --artifacts directory without whitespace inside RustLib.")
    return value


def wsl_path(path):
    value = str(path.resolve()).replace("\\", "/")
    return "/mnt/" + value[0].lower() + value[2:] if value[1:2] == ":" else value


def command(binary, target, script):
    args = ["--editor", "--navigation", "--anim", "off", "--verify-dirty",
            "--script", script_path(script)]
    if target == "win" or os.name != "nt":
        return [str(binary), *args]
    return ["wsl.exe", "-d", "Ubuntu-24.04", "--cd", wsl_path(ROOT),
            "-e", wsl_path(binary), *args]


def write_script(directory, scale):
    lines = [f"size {WINDOW_SIZE[0]} {WINDOW_SIZE[1]}", f"scale {scale}",
             "theme light", "cmd 61", 'click tab:"Start"']
    # Selecting Start also removes the blinking editor caret before the baseline.
    # Kind-qualified selectors distinguish the Einfügen tab from the Paste button.
    for stage, tab, _theme, actions in STAGES:
        lines.extend(actions)
        lines.extend((f'expect tab:"{tab}" selected', "expect #3145728 exists",
                      f"shot {script_path(directory / (stage + '.png'))}",
                      f"dump {script_path(directory / (stage + '.jsonl'))}"))
    lines.extend(("expect_nowarn", "print JFWORD_VISUAL_OK"))
    path = directory / "sequence.uis"
    path.write_bytes(("\n".join(lines) + "\n").encode("utf-8"))
    return path


def unique_record(records, widget, kind):
    matches = [r for r in records if r.get("id") == widget and r.get("k") == kind
               and "w" not in r]
    if len(matches) != 1:
        raise AssertionError(f"Expected one {kind} #{widget}, got {len(matches)}")
    return matches[0]


def paper_image(directory, stage, tab, theme, scale):
    records = [json.loads(line) for line in (directory / (stage + ".jsonl"))
               .read_text(encoding="utf-8").splitlines() if line]
    header = records[0]
    assert tuple(header["size"]) == WINDOW_SIZE and header["scale"] == scale, header
    assert header["theme"] == theme, header
    assert any(r.get("k") == "tab" and r.get("t") == tab and "selected" in r["s"]
               for r in records), f"Wrong active tab in {stage}"
    assert any(r.get("k") == "textbox" and "Titel der wissenschaftlichen Arbeit" in r["t"]
               and "Zusammenfassung" in r["t"] for r in records), f"Template missing in {stage}"
    assert any(r.get("k") == "combo" and r.get("id") == 126 and r["v"] == 2
               for r in records), f"Document zoom changed from 100% in {stage}"
    page = unique_record(records, PAGE_ID, "canvas")["r"]
    viewport = unique_record(records, SCROLL_ID, "box")["r"]
    # Dump positions are DIP rounded from native pixels. A three-DIP inset avoids
    # that half-pixel ambiguity at 150%, the paper outline and edge decoration.
    x, y, width, height = page
    vx, vy, vw, vh = viewport
    crop_dip = (max(x, vx) + 3, max(y, vy) + 3,
                min(x + width, vx + vw) - 3, min(y + height, vy + vh) - 3)
    left, top, right, bottom = [v * scale // 100 for v in crop_dip]
    sw, sh, rgba = read_png(directory / (stage + ".png"))
    assert (sw, sh) == tuple(v * scale // 100 for v in WINDOW_SIZE), (stage, sw, sh)
    assert 0 <= left < right <= sw and 0 <= top < bottom <= sh, (stage, crop_dip)
    assert right - left > 700 * scale // 100 and bottom - top > 400 * scale // 100, (
        stage, "Document crop unexpectedly small", crop_dip)
    pixels = b"".join(rgba[(row * sw + left) * 4:(row * sw + right) * 4]
                      for row in range(top, bottom))
    return {"pageDip": page, "viewportDip": viewport, "cropPx": [left, top, right, bottom],
            "width": right - left, "height": bottom - top,
            "sha256": hashlib.sha256(pixels).hexdigest()}, pixels


def assert_same_paper(reference, current, path, label):
    meta, expected = reference
    actual_meta, actual = current
    for key in ("pageDip", "viewportDip", "cropPx"):
        assert actual_meta[key] == meta[key], f"{label}: document geometry changed ({key})"
    if actual == expected:
        return
    width, height = meta["width"], meta["height"]
    difference = bytearray(b"\xff\xff\xff\xff" * (width * height))
    count, left, top, right, bottom = 0, width, height, 0, 0
    for at in range(0, len(expected), 4):
        if actual[at:at + 4] != expected[at:at + 4]:
            index = at // 4
            x, y = index % width, index // width
            count += 1
            left, top = min(left, x), min(top, y)
            right, bottom = max(right, x + 1), max(bottom, y + 1)
            difference[at:at + 4] = b"\xe0\x20\x20\xff"
    write_png(path, width, height, difference)
    raise AssertionError(f"{label}: {count} changed document pixels in "
                         f"[{left},{top},{right},{bottom}]; diff: {path}")


def run_case(output, artifacts, target, scale, timeout):
    directory = artifacts / f"{target}-{scale}"
    directory.mkdir(parents=True, exist_ok=True)
    binary = output / target / ("jfWord.exe" if target == "win" else "jfWord")
    if not binary.is_file():
        raise FileNotFoundError(f"Build jfWord for {target} first: {binary}")
    script = write_script(directory, scale)
    # PowerShell can return early for GUI executables. subprocess.run guarantees
    # all shots/dumps are closed before reading and comparing them.
    process = subprocess.run(command(binary, target, script), cwd=ROOT,
                             text=True, encoding="utf-8", errors="replace",
                             capture_output=True, timeout=timeout)
    (directory / "stdout.txt").write_bytes(process.stdout.encode("utf-8"))
    (directory / "stderr.txt").write_bytes(process.stderr.encode("utf-8"))
    assert process.returncode == 0 and "JFWORD_VISUAL_OK" in process.stdout, (
        target, scale, process.returncode, process.stdout, process.stderr)
    assert not process.stderr and "FAIL" not in process.stdout and "UIWARN" not in process.stdout, (
        target, scale, process.stdout, process.stderr)
    baseline, manifest = None, []
    for stage, tab, theme, _actions in STAGES:
        current = paper_image(directory, stage, tab, theme, scale)
        if baseline is None:
            baseline = current
            # Guard against a blank baseline; the regression itself is exact pixel
            # equality over the whole page, not merely the presence of any ink.
            assert sum(current[1][at:at + 3] != b"\xff\xff\xff"
                       for at in range(0, len(current[1]), 4)) > 100, "Blank Home baseline"
        else:
            assert_same_paper(baseline, current, directory / (stage + "-diff.png"),
                              f"{target}/{scale}%/{stage}")
        manifest.append({"target": target, "scale": scale, "stage": stage, **current[0]})
    print(f"PASS {target} {scale}%: {len(STAGES)} unchanged document images through tabs/themes", flush=True)
    return baseline, manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", default="build/wordcraft-design", help="Existing native build directory")
    parser.add_argument("--artifacts", help="Images/dumps/logs directory inside RustLib (default: OUT/jfword-visual)")
    parser.add_argument("--target", choices=("both", "win", "linux"), default="both")
    parser.add_argument("--timeout", type=int, default=180, help="Seconds per app process")
    args = parser.parse_args()
    output = (ROOT / args.out).resolve()
    artifacts = (ROOT / args.artifacts).resolve() if args.artifacts else output / "jfword-visual"
    script_path(artifacts)
    targets = ("win", "linux") if args.target == "both" else (args.target,)
    manifest = []
    for scale in (100, 150, 200):
        results = {}
        for target in targets:
            results[target], rows = run_case(output, artifacts, target, scale, args.timeout)
            manifest.extend(rows)
        if len(results) == 2:
            assert_same_paper(results["win"], results["linux"], artifacts / f"native-{scale}-diff.png",
                              f"Win/Linux/{scale}%")
            print(f"PASS Win/Linux {scale}%: identical document pixels", flush=True)
    (artifacts / "results.json").write_bytes((json.dumps(manifest, indent=2) + "\n").encode("utf-8"))


if __name__ == "__main__":
    main()
