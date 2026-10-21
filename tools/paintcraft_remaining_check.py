"""Regression checks for plugin discovery/file types, live text selection and feathering.
Run after building PaintCraft; --plugins additionally requires Windows/.NET/Paint.NET.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import build_rs


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('--out', default='build/paintcraft-parity')
    parser.add_argument('--target', choices=['win', 'linux', 'both'], default='both')
    parser.add_argument('--plugins', action='store_true')
    args = parser.parse_args()
    os.chdir(ROOT)
    out = Path(args.out).resolve()
    cases = out / 'remaining-check'
    cases.mkdir(parents=True, exist_ok=True)
    results = []
    def resource(tag, data):
        return b'8BIM' + struct.pack('>H2xI', tag, len(data)) + data + bytes(len(data) % 2)
    # Exif/XMP are opaque payloads, including odd length/padding and Unicode.
    exif=b'II*\0\x08\0\0\0\0\0\0\0\0\0'
    xmp='<x:xmpmeta xmlns:x="adobe:ns:meta/">Ä</x:xmpmeta>'.encode()
    resolution=struct.pack('>IHHIHH',300<<16,1,1,150<<16,1,1)
    resources=resource(1005,resolution)+resource(1058,exif)+resource(1060,xmp)
    icc=Path('C:/Windows/System32/spool/drivers/color/sRGB Color Space Profile.icm')
    if icc.exists():
        resources=resource(1005,resolution)+resource(1039,icc.read_bytes())+resource(1058,exif)+resource(1060,xmp)
    psd=cases/'metadata.psd'
    psd.write_bytes(b'8BPS'+struct.pack('>H6xHIIHH',1,3,2,2,8,3)+struct.pack('>II',0,len(resources))+resources+bytes(6)+bytes([128]*12))

    def relative(path):
        return path.relative_to(ROOT).as_posix()

    def run(target, name, script, extra=()):
        path = cases / f'{name}-{target}.uis'
        path.write_text(script, encoding='utf-8')
        exe = out / target / ('PaintCraft.exe' if target == 'win' else 'PaintCraft')
        result = build_rs.run_exe(target, exe, [*extra, '--config', relative(cases / f'config-{name}-{target}'),
                                  '--script', relative(path), '--verify-dirty'], timeout=120)
        assert result.returncode == 0, name + ': ' + result.stdout + result.stderr
        results.append(dict(target=target, case=name, passed=True))

    for target in (['win', 'linux'] if args.target == 'both' else [args.target]):
        run(target, 'async-open', '''size 1280 800
theme light
drop 600 400 apps/PaintCraft/tests/fixtures/a1_rgba.png apps/PaintCraft/tests/fixtures/a1_rgb.bmp
cmd 174 1
sync
expect #9901 value 3
expect #9911 text "a1_rgb.bmp"
cmd 900 0
expect #9900 text "FFFF0000"
drop 600 400 apps/PaintCraft/tests/fixtures/a1_rgba.png apps/PaintCraft/tests/fixtures/a1_rgb.bmp
cmd 174 1
sync
expect #9901 value 3
expect #9911 text "a1_rgb.bmp"
drop 600 400 apps/PaintCraft/tests/fixtures/a1_rgb.jpg apps/PaintCraft/tests/fixtures/a1_rgb.bmp
cmd 174 1
sync
expect #9901 value 4
expect #9911 text "a1_rgb.bmp"
drop 600 400 apps/PaintCraft/tests/fixtures/pdn/c_layers.pdn
cmd 174 1
sync
expect #9901 value 5
expect #9903 value 5
expect #9911 text "c_layers.pdn"
''')
        saved=cases/f'metadata-{target}.psd'
        saved.unlink(missing_ok=True)
        run(target,'psd-metadata',f'''size 1280 800
expect #9915 value 30000
cmd 103
click #81932
key ctrl+a
type "{saved.name}"
click button:"Speichern"
''',['--open',relative(psd)])
        exported=saved.read_bytes()
        size=struct.unpack_from('>I',exported,30)[0]
        assert exported[34:34+size]==resources, 'PSD resource metadata changed'
        run(target, 'text-selection', '''size 1280 800
theme light
key t
cmd 840 0
cmd 841 32
click 290,198
type "Hallo schöne Welt"
key ctrl+shift+left
type "Erde"
expect #532480 text "Hallo schöne Erde"
key shift+home
key ctrl+c
key ctrl+x
expect #532480 text ""
key ctrl+v
expect #532480 text "Hallo schöne Erde"
key ctrl+a
type "Äpfel"
expect #532480 text "Äpfel"
key shift+left
key backspace
expect #532480 text "Äpfe"
key ctrl+a
key delete
expect #532480 text ""
type "A"
click #3002
key ctrl+a
type "12.5"
key enter
expect #3002 text "12,50"
cmd 790
expect #4391 text "Text"
key ctrl+z
cmd 901 65537
expect #9900 text "FFFFFFFF"
''')
        run(target, 'feather', '''size 1280 800
theme light
cmd 750
drag 250,158 350,258
cmd 185
click #0x93002
cmd 980 3276810
expect #9980 value 255
cmd 185
click #0x93000
key ctrl+a
type "2"
key enter
click #0x93001
cmd 980 3276810
expect #9980 value 153
cmd 980 3276809
expect #9980 value 102
key ctrl+z
cmd 980 3276810
expect #9980 value 255
cmd 980 3276809
expect #9980 value 0
key ctrl+y
cmd 980 3276810
expect #9980 value 153
key ctrl+i
cmd 980 3276810
expect #9980 value 102
''')
        run(target,'content-fill','''size 1280 800
theme light
cmd 902 1
cmd 903 655370
cmd 904 1966110
cmd 982 655370
cmd 983 1966110
cmd 984 0
cmd 981 0
cmd 577
sync
click button:"Abbrechen"
cmd 901 1310740
expect #9900 text "FFFF0000"
cmd 577
sync
click button:"OK"
sync
cmd 901 1310740
expect #9900 text "FFFFFFFF"
cmd 901 65537
expect #9900 text "FFFFFFFF"
key ctrl+z
cmd 901 1310740
expect #9900 text "FFFF0000"
''')

    if args.plugins:
        env = dict(os.environ, DOTNET_ReadyToRun='0')
        host = out / 'win/pdn-host/PaintCraft.PdnHost.dll'
        root = cases / 'plugins'
        filetypes = root / 'FileTypes'
        (root/'Shapes').mkdir(parents=True,exist_ok=True)
        (root/'Shapes/catalog.xaml').write_text('<ps:SimpleGeometryShape xmlns:ps="clr-namespace:PaintDotNet.Shapes;assembly=PaintDotNet.Framework" DisplayName="Catalog test shape" Geometry="M 0,0 L 10,0 10,10 0,10 Z" />',encoding='utf-8')
        subprocess.run(['dotnet', 'build', 'tools/paintcraft-pdn-host/tests/Fixture.csproj', '-c', 'Release',
                        '-o', str(filetypes), '--nologo', '-v:q'], check=True)
        fixture = filetypes / 'Fixture.dll'
        # One malformed DLL must not keep the valid factory from being discovered.
        (filetypes / 'broken.dll').write_bytes(b'not a DLL')
        pastel=out/'boltbait-test/Pastel.dll'
        if pastel.exists():
            (root/'Effects').mkdir(exist_ok=True)
            shutil.copyfile(pastel,root/'Effects/Pastel.dll')
        catalog = subprocess.run(['dotnet', str(host), '--catalog', '--isolated', '--root', str(root),
                                  '--root', str(root)], capture_output=True, env=env, check=True).stdout.decode()
        rows = [line.split('\t') for line in catalog.splitlines()]
        assert len([r for r in rows if r[0] == 'F']) == 4, catalog
        assert len([r for r in rows if r[0] == 'X']) == 1, catalog
        caps = {bytes.fromhex(r[5]).decode(): int(r[4]) for r in rows if r[0] == 'F'}
        assert caps == {'pcfixture': 3, 'pcread': 1, 'pcwrite': 2, 'pcbad': 1}
        pixels = struct.pack('<4I', 0xff123456, 0x80775533, 0xffabcdef, 0xffffffff)
        data = struct.pack('<3I', 0x54465354, 2, 2) + pixels
        source = cases / 'image.pcfixture'
        source.write_bytes(data)
        slow = cases / 'slow.pcfixture'
        slow.write_bytes(b'SLOW' + data)
        run('win', 'async-plugin-cancel', f'''size 1280 800
theme light
cmd 101
click #65548
key ctrl+a
type "{slow.as_posix()}"
click button:"Öffnen"
expect window 24810 open
wait 500
click #0x94000
sync
expect window 24810 closed
expect #9901 value 1
expect #9911 text "Unbenannt"
expect #9905 value 0
cmd 101
click #65548
key ctrl+a
type "{source.as_posix()}"
click button:"Öffnen"
sync
expect #9901 value 2
expect #9913 value 2
cmd 901 0
expect #9900 text "FF123456"
''', ['--plugins', str(root), '--pdn-isolated'])
        command = ['dotnet', str(host), str(fixture), 'FixtureFactory#0']
        decoded = subprocess.run(command + ['--decode'], input=data, capture_output=True, env=env, check=True).stdout
        assert decoded == struct.pack('<6I', 0x4d494350, 2, 2, 144 << 16, 192 << 16, 1) + pixels
        encoded = subprocess.run(command + ['--encode'], input=decoded, capture_output=True, env=env, check=True).stdout
        assert encoded == data
        for damaged in [b'', b'PCIM', struct.pack('<6I', 0x4d494350, 0xffffffff, 2, 0, 0, 1)]:
            result = subprocess.run(command + ['--encode'], input=damaged, capture_output=True, env=env)
            assert result.returncode != 0 and not result.stdout
        results.append(dict(target='win', case='filetype-host-roundtrip-and-errors', passed=True))
        saved = cases / 'export.pcfixture'
        saved.unlink(missing_ok=True)
        run('win', 'filetype-ui', f'''size 1280 800
theme light
expect #9913 value 2
expect #9914 value 2
expect #9905 value 1
cmd 901 0
expect #9900 text "FF123456"
cmd 901 1
expect #9900 text "80775533"
cmd 103
click #81932
key ctrl+a
type "{saved.as_posix()}"
click button:"Speichern"
sync
expect #9905 value 0
''', ['--plugins', str(root), '--pdn-isolated', '--open', relative(source)])
        assert saved.read_bytes() == data
        run('win','catalog-shape','''size 1280 800
key o
key o
cmd 795 262174
expect #524546 text "Catalog test shape"
''',['--plugins',str(root),'--pdn-isolated'])
        run('win','catalog-disabled','''size 1280 800
theme light
click menu:"Effekte"
click menuitem:"Paint.NET-Plugins"
expect menuitem:"Pastel" missing
''',['--plugins',str(root),'--pdn-isolated','--no-plugins'])
        if pastel.exists():
            run('win','original-plugin-menu','''size 1280 800
theme light
click menu:"Effekte"
click menuitem:"Paint.NET-Plugins"
expect menuitem:"Pastel" exists
click menuitem:"Pastel"
sync
expect #4391 text "Paint.NET-Plugin"
key ctrl+z
cmd 995 0
expect #9990 text "2cc39813e097b672"
''',['--plugins',str(root),'--pdn-isolated','--pdn-defaults','--open','apps/PaintCraft/tests/fixtures/a6_photo.png'])
        results.append(dict(target='win', case='catalog-duplicates-and-filter-capabilities', passed=True))
    (cases / 'results.json').write_text(json.dumps(results, indent=2), encoding='utf-8')
    print(f'{len(results)} regression cases passed')


if __name__ == '__main__':
    main()
