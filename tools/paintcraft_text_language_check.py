"""Document text shaping/preedit and live locale switching on both desktop targets."""
import argparse
import json
import os
from pathlib import Path
import shutil
import sys
ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT))
import build_rs

def main():
    ap=argparse.ArgumentParser(); ap.add_argument('--out',default='build/paintcraft-next'); ap.add_argument('--target',choices=['win','linux','both'],default='both'); ap.add_argument('--font',default=os.environ.get('PAINTCRAFT_SHAPING_FONT')); args=ap.parse_args()
    out=Path(args.out).resolve(); cases=out/'text-language-check'; cases.mkdir(parents=True,exist_ok=True); results=[]
    def rel(p): return p.relative_to(ROOT).as_posix()
    # Local, untracked fixture; not redistributed with PaintCraft.
    source_font=Path(args.font) if args.font else next((p for p in [Path('C:/Windows/Fonts/arial.ttf'),Path('/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf')] if p.exists()),None)
    if source_font is None: raise SystemExit('Use --font with an installed TrueType font supporting Latin, Arabic and Hebrew')
    font=cases/'shaping.ttf'; shutil.copyfile(source_font,font)
    def run(target,name,script,extra=()):
        path=cases/f'{name}-{target}.uis'; path.write_text('size 1280 800\ntheme light\n'+script,encoding='utf-8')
        exe=out/target/('PaintCraft.exe' if target=='win' else 'PaintCraft')
        r=build_rs.run_exe(target,exe,['--script',rel(path),'--verify-dirty',*extra],timeout=120)
        assert r.returncode==0,f'{target}/{name}: '+r.stdout+r.stderr
        results.append(dict(target=target,case=name,passed=True))
    for target in (['win','linux'] if args.target=='both' else [args.target]):
        for code,close,brush in [('en','Close','Brush tip…'),('fr','Fermer','Pointe du pinceau…')]:
            config=cases/f'config-{target}-{code}'
            run(target,'locale-'+code,f'''click button:"{brush}"
click #0x90209
cmd 710
expect #5532 enabled
expect #5532 value {1 if code=='en' else 2}
expect #0xffff7866 text "{close}"
click #0xffff7866
''',['--language',code,'--config',rel(config)])
            run(target,'locale-persist-'+code,f'''click button:"{brush}"
click #0x90209
cmd 710
expect #5532 value {1 if code=='en' else 2}
expect #0xffff7866 text "{close}"
click #0xffff7866
''',['--config',rel(config)])
        run(target,'preedit', '''key t
click 290,198
cmd 844 4
expect #532488 text "にほん"
expect #532488 value 6
expect #532480 text ""
expect #9905 value 0
cmd 844 5
expect #532488 text ""
type "日本語"
expect #532480 text "日本語"
key esc
expect #9905 value 0
expect #9935 value 1
''')
        run(target,'long-ime-commit', 'key t\nclick 290,198\ncmd 844 6\nsync 1000\nexpect #532481 value 600\nkey ctrl+a\nkey ctrl+x\nexpect #532480 text ""\nkey ctrl+v\nexpect #532481 value 600\nkey esc\nexpect #9905 value 0\n')
        run(target,'bidi-selection', '''key t
cmd 841 32
click 290,198
type "abc אבג xyz"
cmd 844 2
key shift+right
key shift+right
key shift+right
key shift+right
key shift+right
expect #532489 value 2
key ctrl+a
expect #532489 value 1
key esc
''',['--text-font',rel(font)])
        run(target,'shaped-edit-undo', '''key t
cmd 841 24
click 290,198
type "abc سلام אבג 123"
expect #532480 text "abc سلام אבג 123"
key home
key shift+end
key ctrl+c
key backspace
expect #532480 text ""
key ctrl+v
expect #532480 text "abc سلام אבג 123"
cmd 790
expect #9935 value 2
key ctrl+z
expect #9905 value 0
key ctrl+y
expect #9905 value 1
''',['--text-font',rel(font)])
    (cases/'results.json').write_text(json.dumps(results,indent=2),encoding='utf-8'); print(f'{len(results)} text/locale cases passed')
if __name__=='__main__': main()
