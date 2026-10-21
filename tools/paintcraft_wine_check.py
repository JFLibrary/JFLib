"""End-to-end native Linux editor tests against actual Windows DLLs under Wine.
Supply a configured private Wine prefix, Windows dotnet.exe, Paint.NET-on-Wine,
the deployed helper and an original Pastel.dll (not a replacement implementation).
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import struct
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import build_rs


def main():
    ap = argparse.ArgumentParser(__doc__)
    ap.add_argument('--out', default='build/paintcraft-next')
    for name in ['wine', 'prefix', 'paintnet', 'host', 'pastel', 'fixture']:
        ap.add_argument('--' + name, required=True)
    ap.add_argument('--dotnet', default='C:/Program Files/dotnet/dotnet.exe', help='Only used when --host names a DLL')
    a = ap.parse_args()
    os.chdir(ROOT)
    out = Path(a.out).resolve()
    cases = out / 'wine-check'
    root = cases / 'Plugins Öl & Farbe'
    for sub in ['Effects', 'FileTypes', 'Shapes']:
        (root / sub).mkdir(parents=True, exist_ok=True)
    shutil.copyfile(a.pastel, root / 'Effects/Pastel.dll')
    shutil.copyfile(a.fixture, root / 'FileTypes/Fixture.dll')
    (root / 'FileTypes/broken.dll').write_bytes(b'invalid DLL')
    (root / 'Shapes/catalog.xaml').write_text('<ps:SimpleGeometryShape xmlns:ps="clr-namespace:PaintDotNet.Shapes;assembly=PaintDotNet.Framework" DisplayName="Wine catalog shape" Geometry="M 0,0 L 10,0 10,10 0,10 Z" />', encoding='utf-8')
    pixels = struct.pack('<4I', 0xff123456, 0x80775533, 0xffabcdef, 0xffffffff)
    data = struct.pack('<3I', 0x54465354, 2, 2) + pixels
    source = cases / 'image.pcfixture'
    source.write_bytes(data)
    slow = cases / 'slow.pcfixture'
    slow.write_bytes(b'SLOW' + data)
    relative = lambda p: p.relative_to(ROOT).as_posix()
    def native(p):
        value = p.resolve().as_posix()
        return '/mnt/' + value[0].lower() + value[2:] if os.name == 'nt' else value
    options = ['--pdn-wine', a.wine, '--pdn-wine-prefix', a.prefix, '--pdn-dotnet', a.dotnet,
               '--pdn-install', a.paintnet, '--pdn-host', a.host, '--plugins', relative(root), '--pdn-isolated', '--pdn-defaults']
    results = []

    def run(name, script, extra=()):
        path = cases / f'{name}.uis'
        path.write_text('size 1280 800\ntheme light\n' + script, encoding='utf-8')
        result = build_rs.run_exe('linux', out / 'linux/PaintCraft', [*options, '--config', relative(cases / ('config-' + name)), '--script', relative(path), '--verify-dirty', *extra], timeout=180)
        (cases / f'{name}.log').write_text(result.stdout + result.stderr, encoding='utf-8')
        assert result.returncode == 0, name + ': ' + result.stdout + result.stderr
        results.append(dict(case=name, passed=True))

    run('original-effect-undo', '''click menu:"Effekte"
click menuitem:"Paint.NET-Plugins"
expect menuitem:"Pastel" exists
click menuitem:"Pastel"
sync 120000
expect #9918 text ""
expect #4391 text "Paint.NET-Plugin"
key ctrl+z
cmd 995 0
expect #9990 text "2cc39813e097b672"
key ctrl+y
expect #4391 text "Paint.NET-Plugin"
''', ['--open', 'apps/PaintCraft/tests/fixtures/a6_photo.png'])
    saved = cases / 'saved.pcfixture'
    saved.unlink(missing_ok=True)
    run('filetype-alpha-dpi-roundtrip', f'''expect #9913 value 2
expect #9914 value 2
expect #9915 value 14400
cmd 901 0
expect #9900 text "FF123456"
cmd 901 1
expect #9900 text "80775533"
cmd 103
click #81932
key ctrl+a
type "{native(saved)}"
click button:"Speichern"
sync 120000
expect #9918 text ""
''', ['--open', relative(source)])
    assert saved.read_bytes() == data, 'FileType round trip changed alpha/pixels'
    run('codec-cancel', f'''cmd 101
click #65548
key ctrl+a
type "{native(slow)}"
click button:"Öffnen"
wait 700
click #0x94000
sync 120000
expect #9901 value 1
expect #9905 value 0
cmd 901 0
expect #9900 text "FFFFFFFF"
''')
    run('catalog-shape', '''key o
key o
cmd 795 262174
expect #524546 text "Wine catalog shape"
''')
    (cases / 'results.json').write_text(json.dumps(results, indent=2), encoding='utf-8')
    print(f'{len(results)} native Linux / Windows DLL cases passed')


if __name__ == '__main__':
    main()
