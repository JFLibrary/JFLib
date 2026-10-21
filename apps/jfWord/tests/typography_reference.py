"""Compare identical UI words with original WordCraft pixels at their native 200% scale.

The isolated officeicontest probe renders old SGF/legacy and current TTF/natural
paths side by side. Foreground/background are normalized to coverage; an integer
translation of at most two pixels aligns glyph bounds. No resizing or warping is
used. This measures raster/spacing error, not full application pixel equality.
"""
import argparse
import base64
import hashlib
import html
import json
from pathlib import Path
import sys

import numpy as np

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT), str(ROOT / 'tools')]
import build_rs
from pngutil import read_png, write_png

SAMPLES = [
    ('Navigation', 'hero', (10, 173, 92, 191), 250, 36),
    ('Layout', 'hero', (277, 46, 324, 62), 240, 36),
    ('Design', 'hero', (215, 46, 264, 62), 240, 36),
    ('AutoSave', 'hero', (77, 12, 132, 26), 240, 97),
    ('Clipboard', 'hero', (43, 140, 101, 155), 249, 112),
    ('Good morning', 'backstage', (240, 30, 450, 65), 249, 36),
    ('Blank document', 'backstage', (240, 277, 390, 294), 249, 36),
]


def pixels(path):
    w, h, rgba = read_png(path)
    return np.frombuffer(rgba, np.uint8).reshape(h, w, 4)[:, :, :3].astype(float)


def ink(image, background, foreground):
    coverage = np.clip((background - image[:, :, 0]) / (background - foreground), 0, 1)
    ys, xs = np.where(coverage > .25)
    assert len(xs), 'Expected a visible text sample'
    return coverage[max(0, ys.min() - 2):ys.max() + 3, max(0, xs.min() - 2):xs.max() + 3]


def difference(a, b):
    height, width = max(a.shape[0], b.shape[0]) + 6, max(a.shape[1], b.shape[1]) + 6
    ref = np.zeros((height, width))
    ref[3:3 + a.shape[0], 3:3 + a.shape[1]] = a
    best = None
    for dy in range(-2, 3):
        for dx in range(-2, 3):
            test = np.zeros_like(ref)
            test[3 + dy:3 + dy + b.shape[0], 3 + dx:3 + dx + b.shape[1]] = b
            error = float(np.abs(ref - test).sum())
            if best is None or error < best['coverageError']:
                best = dict(coverageError=error, translation=[dx, dy])
    return best


def save_coverage(path, array):
    gray = np.rint(250 - 214 * array).astype(np.uint8)
    rgba = np.empty((*gray.shape, 4), np.uint8)
    rgba[:, :, :3] = gray[:, :, None]
    rgba[:, :, 3] = 255
    write_png(path, gray.shape[1], gray.shape[0], rgba.tobytes())


def gallery(out, rows):
    def img(name, label):
        encoded = base64.b64encode((out / name).read_bytes()).decode('ascii')
        return f'<img alt="{label}" src="data:image/png;base64,{encoded}">'
    entries = []
    for r in rows:
        row = next(i for i, sample in enumerate(SAMPLES) if sample[0] == r['text'])
        prefix = f'{row}-{r["theme"]}'
        entries.append(f'<tr><th>{html.escape(r["text"])}<small>{r["theme"]}</small></th>'
                       + ''.join('<td>' + img(prefix + '-' + stage + '.png', stage) + '</td>'
                                 for stage in ('reference', 'before', 'after'))
                       + f'<td>{r["before"]["coverageError"]:.2f} → {r["after"]["coverageError"]:.2f}</td></tr>')
    (out / 'comparison.html').write_text('''<!doctype html><html lang="de"><meta charset="utf-8">
<title>jfOffice · Schriftvergleich</title><style>
body{font:15px system-ui,sans-serif;margin:32px;color:#242424;background:#f9f9f9}
h1{font-size:24px}p{max-width:980px;line-height:1.6}table{border-collapse:collapse;background:white}
th,td{padding:16px;border:1px solid #ddd;text-align:left;vertical-align:middle}
small{display:block;font-weight:400;color:#616161}img{display:block;image-rendering:pixelated;zoom:var(--zoom,1)}
label{display:block;margin:20px 0}input{vertical-align:middle}div{overflow:auto}
</style><h1>jfOffice: Schriftabstände und Rasterisierung</h1>
<p>Originalausschnitte aus den bereitgestellten WordCraft-Screenshots bei 200 %, daneben
die bisherigen SGF-Fonts und die neue TrueType-Ausgabe. Für den Vergleich sind Text- und
Hintergrundfarbe auf denselben Deckungsgrad normiert. Die Bilder werden für die Messung
nicht skaliert; die Ausrichtung erlaubt höchstens zwei ganze Pixel Versatz.
Die letzte Spalte summiert die absolute Deckungsgradabweichung zur Referenz.
Dies bewertet Schriftkonturen und Zeichenabstände, nicht die komplette Oberfläche.</p>
<label>Ansichtsvergrößerung <input type="range" min="1" max="4" step="1" value="1"
oninput="document.documentElement.style.setProperty('--zoom',this.value)"></label>
<div><table><thead><tr><th>Text</th><th>WordCraft</th><th>Vorher</th><th>Jetzt</th><th>Abweichung</th></tr></thead>
<tbody>''' + ''.join(entries) + '</tbody></table></div></html>', encoding='utf-8')


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--build-dir', default='build/wordcraft-type')
    ap.add_argument('--target', choices=['win', 'linux', 'both'], default='both')
    args = ap.parse_args()
    build = (ROOT / args.build_dir).resolve()
    out = build / 'typography'
    out.mkdir(parents=True, exist_ok=True)
    targets = ['win', 'linux'] if args.target == 'both' else [args.target]
    decoded = {}
    for target in targets:
        binary = build / target / ('officeicontest.exe' if target == 'win' else 'officeicontest')
        check = build_rs.run_exe(target, binary, ['--render-checks'])
        (out / f'{target}-checks.txt').write_text(check.stdout + check.stderr, encoding='utf-8')
        assert not check.returncode, check.stdout + check.stderr
        print(check.stdout.strip(), flush=True)
        for scale in (100, 150, 200):
            for theme in ('light', 'dark'):
                name = f'{target}-{scale}-{theme}'
                script = out / (name + '.uis')
                shot = out / (name + '.png')
                script.write_text(f'size 560 440\nscale {scale}\ntheme {theme}\ncmd 3 1\n'
                                  f'shot {shot.relative_to(ROOT).as_posix()}\nexpect_nowarn\n')
                run = build_rs.run_exe(target, binary, ['--anim', 'off', '--verify-dirty', '--script', script.relative_to(ROOT).as_posix()])
                assert not run.returncode and 'UIWARN' not in run.stdout, run.stdout + run.stderr
                decoded[(target, scale, theme)] = pixels(shot)
                if target == 'linux' and 'win' in targets:
                    assert np.array_equal(decoded[(target, scale, theme)], decoded[('win', scale, theme)]), name
                print('PASS ' + name, flush=True)
    source = ROOT / 'apps/jfWord/context/wordcraft/docs/images'
    refs = {name: pixels(source / (name + '.png')) for name in ('hero', 'backstage', 'dark')}
    rows = []
    for row, (label, source_name, box, bg, fg) in enumerate(SAMPLES):
        for theme in (('light', 'dark') if row in (1, 2, 3) else ('light',)):
            ref_name = 'dark' if theme == 'dark' else source_name
            if theme == 'dark':
                bg, fg = 31, (200 if row == 3 else 238)
            x0, y0, x1, y1 = (v * 2 + 16 for v in box)
            reference = ink(refs[ref_name][y0:y1, x0:x1], bg, fg)
            probe = decoded[(targets[0], 200, theme)]
            cases = []
            for col in range(2):
                actual = ink(probe[40 + row * 100:100 + row * 100, col * 520:(col + 1) * 520],
                             31 if theme == 'dark' else 250, 238 if theme == 'dark' else 36)
                cases.append(difference(reference, actual))
                save_coverage(out / f'{row}-{theme}-{["before", "after"][col]}.png', actual)
            save_coverage(out / f'{row}-{theme}-reference.png', reference)
            assert cases[1]['coverageError'] < cases[0]['coverageError'], (label, theme, cases)
            rows.append(dict(text=label, theme=theme, reference=ref_name + '.png',
                             referenceCropPx=[x0, y0, x1, y1], before=cases[0], after=cases[1]))
            print(f"PASS {label}/{theme}: {cases[0]['coverageError']:.2f} -> {cases[1]['coverageError']:.2f}", flush=True)
    report = dict(scope=__doc__, samples=rows,
                  nativePixelComparisons=6 if len(targets) == 2 else 0,
                  referenceHashes={n: hashlib.sha256((source / (n + '.png')).read_bytes()).hexdigest() for n in refs},
                  beforeError=sum(r['before']['coverageError'] for r in rows),
                  afterError=sum(r['after']['coverageError'] for r in rows))
    (out / 'results.json').write_text(json.dumps(report, indent=2, ensure_ascii=False), encoding='utf-8')
    gallery(out, rows)


if __name__ == '__main__':
    main()
