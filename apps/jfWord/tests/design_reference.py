"""Render every Office tab and compare measured chrome with the supplied screenshots.

The reference has a 16-pixel outer border and a 200% scale. Only equivalent UI
regions are compared: translations, branding and document contents differ. This
is not a claim that an entire application screenshot is pixel-identical.
"""
import argparse
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT), str(ROOT / 'tools')]
import build_rs
from pngutil import read_png, write_png, downscale

TABS = ['Start', 'Einfügen', 'Zeichnen', 'Entwurf', 'Layout', 'Referenzen',
        'Sendungen', 'Überprüfen', 'Ansicht', 'Hilfe']
CASES = [('light', 100, 1424), ('dark', 100, 1424), ('light', 150, 1240),
         ('dark', 150, 1000), ('light', 200, 1920), ('dark', 200, 600)]


def relative(path):
    return path.relative_to(ROOT).as_posix()


def render(build, artifacts):
    results = []
    for theme, scale, width in CASES:
        folder = artifacts / f'{theme}-{scale}-{width}'
        folder.mkdir(parents=True, exist_ok=True)
        lines = [f'size {width} 884', f'scale {scale}', f'theme {theme}',
                 'cmd 61', 'click tab:"Start"']
        for index, tab in enumerate(TABS):
            if index:
                lines += ['focus #2000', 'key right']
            lines += [f'expect tab:"{tab}" selected',
                      f'shot {relative(folder / (str(index) + ".png"))}',
                      f'dump {relative(folder / (str(index) + ".jsonl"))}']
        lines += ['cmd 28', f'shot {relative(folder / "backstage.png")}',
                  f'dump {relative(folder / "backstage.jsonl")}', 'expect_nowarn']
        script = folder / 'matrix.uis'
        script.write_text('\n'.join(lines) + '\n', encoding='utf-8')
        run = build_rs.run_exe('win', build / 'win/jfWord.exe',
                             ['--editor', '--navigation', '--anim', 'off', '--verify-dirty',
                              '--script', relative(script)])
        (folder / 'output.txt').write_text(run.stdout + run.stderr, encoding='utf-8')
        assert not run.returncode and 'FAIL' not in run.stdout and 'UIWARN' not in run.stdout, run.stdout + run.stderr
        results.append(dict(theme=theme, scale=scale, width=width, screens=11, passed=True))
        print(f'PASS {theme}/{scale}%/{width}: eleven Office screens', flush=True)
    (artifacts / 'results.json').write_text(json.dumps(results, indent=2), encoding='utf-8')


def normalized(path):
    width, height, rgba = read_png(path)
    assert (width, height) == (2880, 1800), (path, width, height)
    cropped = b''.join(rgba[(y * width + 16) * 4:(y * width + width - 16) * 4]
                       for y in range(16, height - 16))
    return 1424, 884, downscale(2848, 1768, cropped, 1424, 884)


def compare(expected, actual, points):
    ew, eh, ep = expected
    aw, ah, ap = actual
    assert (ew, eh) == (aw, ah)
    points = list(points)
    mismatch = sum(ep[(y * ew + x) * 4:(y * ew + x) * 4 + 3] !=
                   ap[(y * aw + x) * 4:(y * aw + x) * 4 + 3] for x, y in points)
    color_error = sum(abs(ep[(y * ew + x) * 4 + c] - ap[(y * aw + x) * 4 + c])
                      for x, y in points for c in range(3))
    return dict(compared=len(points), differing=mismatch, equal=len(points) - mismatch,
                absoluteRgbError=color_error)


def rect_points(x, y, width, height):
    return ((px, py) for py in range(y, y + height) for px in range(x, x + width))


def crop(image, x, y, width, height):
    iw, ih, data = image
    assert 0 <= x < x + width <= iw and 0 <= y < y + height <= ih
    return width, height, b''.join(data[(row * iw + x) * 4:(row * iw + x + width) * 4]
                                 for row in range(y, y + height))


def outline(x, y, width, height, corner=4):
    # Omit the rounded corners, whose antialiasing uses a different renderer.
    return ([(px, py) for px in range(x + corner, x + width - corner)
             for py in (y, y + height - 1)] +
            [(px, py) for py in range(y + corner, y + height - corner)
             for px in (x, x + width - 1)])


def measure(build, artifacts, before):
    refs = ROOT / 'apps/jfWord/context/wordcraft/docs/images'
    hero, backstage = normalized(refs / 'hero.png'), normalized(refs / 'backstage.png')
    reference = build / 'reference'
    reference.mkdir(exist_ok=True)
    for name, pic in [('hero', hero), ('backstage', backstage)]:
        write_png(reference / (name + '.png'), *pic)
    current = artifacts / 'light-100-1424'
    old = before / 'light-100-1424'
    images = {'hero': (hero, read_png(old / '0.png'), read_png(current / '0.png')),
              'backstage': (backstage, read_png(old / 'backstage.png'), read_png(current / 'backstage.png'))}
    regions = [
        ('navigation-left-frame', 'hero', list(rect_points(0, 160, 10, 698))),
        ('navigation-right-frame', 'hero', list(rect_points(250, 160, 10, 698))),
        ('navigation-search-outline', 'hero', outline(11, 201, 237, 19)),
        ('navigation-divider', 'hero', list(rect_points(11, 257, 237, 1))),
        ('status-top-divider', 'hero', list(rect_points(0, 858, 1424, 1))),
        ('first-template-outline', 'backstage', outline(240, 82, 150, 194)),
        ('backstage-active-position', 'backstage', list(rect_points(1, 62, 10, 38))),
    ]
    results = []
    for name, key, points in regions:
        ref, previous, present = images[key]
        row = dict(region=name, before=compare(ref, previous, points),
                   after=compare(ref, present, points))
        assert row['after']['differing'] <= row['before']['differing'], row
        results.append(row)
        print(f"{name}: {row['before']['differing']} -> {row['after']['differing']} differing pixels / {len(points)}", flush=True)
    total_before = sum(row['before']['differing'] for row in results)
    total_after = sum(row['after']['differing'] for row in results)
    assert total_after < total_before, (total_before, total_after)
    # A separate band includes rounded corners and the fractional outermost row.
    # Unlike the straight-edge sampling above, this also measures antialiasing.
    edge_points = [(x, y) for x, y in rect_points(238, 80, 154, 198)
                   if not (244 <= x < 386 and 86 <= y < 271)]
    ref, previous, present = images['backstage']
    edges = dict(region='first-template-complete-border-band',
                 before=compare(ref, previous, edge_points), after=compare(ref, present, edge_points))
    assert edges['after']['absoluteRgbError'] < edges['before']['absoluteRgbError'], edges
    print(f"Rounded border band: RGB error {edges['before']['absoluteRgbError']} -> {edges['after']['absoluteRgbError']}", flush=True)
    # Native 200% pixels avoid the one-level rounding of a 200% -> 100% reference.
    native_ref = crop(read_png(refs / 'backstage.png'), 476 + 16, 160 + 16, 308, 396)
    native_before = crop(read_png(before / 'light-200-1920/backstage.png'), 476, 160, 308, 396)
    native_after = crop(read_png(artifacts / 'light-200-1920/backstage.png'), 476, 160, 308, 396)
    native_regions = []
    for name, points in [
        ('straight-edges', outline(4, 3, 300, 388, corner=6)),
        ('complete-border-band', [(x, y) for x, y in rect_points(0, 0, 308, 396)
                                  if not (12 <= x < 296 and 12 <= y < 382)]),
    ]:
        result = dict(region=name, before=compare(native_ref, native_before, points),
                      after=compare(native_ref, native_after, points))
        assert result['after']['absoluteRgbError'] < result['before']['absoluteRgbError'], result
        native_regions.append(result)
        print(f"200% {name}: {result['before']['differing']} -> {result['after']['differing']} differing pixels", flush=True)
    for name, pic in [('reference', native_ref), ('before', native_before), ('after', native_after)]:
        write_png(reference / ('template-200-' + name + '.png'), *pic)
    report = dict(referenceScale=200, outerBorderPx=16, comparisonScale=100,
                  reference='apps/jfWord/context/wordcraft/docs/images',
                  before=relative(before), after=relative(artifacts),
                  regions=results, differingBefore=total_before, differingAfter=total_after,
                  rasterEdges=edges, native200Percent=native_regions,
                  scope='Equivalent chrome regions only; language, document content and branding excluded.')
    (build / 'reference-comparison.json').write_text(json.dumps(report, indent=2), encoding='utf-8')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--build-dir', default='build/wordcraft-align')
    parser.add_argument('--before', default='build/wordcraft-design/design-matrix')
    parser.add_argument('--measure-only', action='store_true')
    args = parser.parse_args()
    build = (ROOT / args.build_dir).resolve()
    artifacts = build / 'design-matrix'
    if not args.measure_only:
        render(build, artifacts)
    measure(build, artifacts, (ROOT / args.before).resolve())


if __name__ == '__main__':
    main()
