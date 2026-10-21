"""Bitmap-tip and real standalone G'MIC regression checks (download runtimes separately)."""
import argparse
import json
import os
from pathlib import Path
import sys
import struct
import zlib

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import build_rs

def main():
    ap=argparse.ArgumentParser(); ap.add_argument('--out',default='build/paintcraft-next'); ap.add_argument('--target',default='both'); ap.add_argument('--gmic',action='store_true'); args=ap.parse_args()
    out=Path(args.out).resolve(); cases=out/'creative-check'; cases.mkdir(parents=True,exist_ok=True); results=[]
    def rel(p): return p.relative_to(ROOT).as_posix()
    def chunk(kind,data): return struct.pack('>I',len(data))+kind+data+struct.pack('>I',zlib.crc32(kind+data))
    raw=(b'\0'+bytes([255,0,0,255])*4+bytes(16))*8
    (cases/'tip.png').write_bytes(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('>IIBBBBB',8,8,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(raw))+chunk(b'IEND',b''))
    def run(target,name,script,extra=()):
        test=cases/f'{name}-{target}.uis'; test.write_text('size 1280 800\ntheme light\n'+script,encoding='utf-8')
        # This test owns this config file; don't carry a saved 180-degree tip
        # rotation into a later run of the same regression.
        config=cases/f'config-{name}-{target}'
        (config/'settings.ini').unlink(missing_ok=True)
        exe=out/target/('PaintCraft.exe' if target=='win' else 'PaintCraft')
        r=build_rs.run_exe(target,exe,['--script',rel(test),'--verify-dirty','--config',rel(config),*extra],timeout=180)
        (cases/f'{name}-{target}.log').write_text(r.stdout+r.stderr,encoding='utf-8')
        assert r.returncode==0,name+': '+r.stdout+r.stderr
        results.append(dict(target=target,case=name,passed=True))
    for target in (['win','linux'] if args.target=='both' else [args.target]):
        run(target,'tip-undo-rotation', '''cmd 930 800
click 340,248
cmd 901 6553698
expect #9900 text "FF000000"
cmd 901 6553702
expect #9900 text "FFFFFFFF"
expect #9935 value 2
key ctrl+z
cmd 901 6553698
expect #9900 text "FFFFFFFF"
key ctrl+y
cmd 901 6553698
expect #9900 text "FF000000"
key ctrl+z
click #0x90200
click #0x90203
key ctrl+a
type "180"
key tab
click #0x90209
click 340,248
cmd 901 6553698
expect #9900 text "FFFFFFFF"
cmd 901 6553702
expect #9900 text "FF000000"
''',['--brush-tip',rel(cases/'tip.png')])
        if args.gmic:
            runtime=out/'gmic-runtime'
            program=rel(runtime/'win/gmic-4.0.5-qt-win64/gmic_qt.exe') if target=='win' else rel(runtime/'gmic-qt-linux')
            os.environ['QT_QPA_PLATFORM']='offscreen'
            run(target,'gmic-invert-undo', '''cmd 445
sync 120000
expect #9918 text ""
cmd 901 0
expect #9900 text "FF000000"
expect #9935 value 2
key ctrl+z
cmd 901 0
expect #9900 text "FFFFFFFF"
key ctrl+y
cmd 901 0
expect #9900 text "FF000000"
''',['--gmic-qt',program,'--gmic-command','-split c -negative[0-2] -append c'])
            run(target,'gmic-cancel', '''cmd 445
click #0x90120
sync 120000
expect #9905 value 0
expect #9935 value 1
cmd 901 0
expect #9900 text "FFFFFFFF"
''',['--gmic-qt',program])
            run(target,'gmic-selection', '''cmd 750
drag 250,158 350,258
cmd 445
sync 120000
cmd 901 3276850
expect #9900 text "FF000000"
cmd 901 0
expect #9900 text "FFFFFFFF"
key ctrl+z
cmd 901 3276850
expect #9900 text "FFFFFFFF"
''',['--gmic-qt',program,'--gmic-command','-split c -negative[0-2] -append c'])
            run(target,'gmic-layers', '''cmd 445
sync 120000
expect #9901 value 2
expect #9903 value 2
expect #9905 value 1
''',['--gmic-qt',program,'--gmic-command','-input [0]'])
            run(target,'gmic-failure', '''cmd 445
sync 120000
expect #9905 value 0
expect #9935 value 1
cmd 901 0
expect #9900 text "FFFFFFFF"
''',['--gmic-qt','missing-paintcraft-gmic-executable'])
    (cases/'results.json').write_text(json.dumps(results,indent=2),encoding='utf-8'); print(f'{len(results)} creative regression cases passed')
if __name__=='__main__': main()
