"""Compile and run the independent hit-group regression with the project's rustc build."""
import argparse,sys
from pathlib import Path
ROOT=Path(__file__).resolve().parents[3];sys.path.insert(0,str(ROOT))
import build_rs

p=argparse.ArgumentParser();p.add_argument('--out',default='build/wordcraft-exact-rows');p.add_argument('--target',choices=['win','linux','both'],default='both');a=p.parse_args()
for target in ['win','linux'] if a.target=='both' else [a.target]:
    b=build_rs.Builder(target,(ROOT/a.out).resolve(),False,4)
    exe=b.dir/('hit_regions.exe' if target=='win' else 'hit_regions')
    root=ROOT/'lib/ui/tests/hit_regions/src/main.rs';crates=build_rs.source_crates(root)
    b.build_libs(crates+['style_jfoffice'])
    cmd=['rustc',*b.flags(),'--crate-type','bin','--crate-name','hit_regions']
    for crate in crates:cmd+=['--extern',f'{crate}={b.rlib(crate)}']
    cmd+=['--extern',f'style={b.rlib("style_jfoffice")}',*b.native_link_args('jfWord',exe,'CONSOLE'),'-o',exe,root]
    build_rs.run_cmd(cmd)
    r=build_rs.run_exe(target,exe,['--script','lib/ui/tests/hit_regions.uis'],timeout=40)
    assert r.returncode==0 and 'FAIL' not in r.stdout,(r.returncode,r.stdout,r.stderr)
    assert r.stderr.count('UIWARN')==2 and '0x2BD' in r.stderr and '0x2BE' in r.stderr,(r.stdout,r.stderr)
    print(f'PASS {target}: one surface accepted; two surface calls and ordinary duplicate widgets rejected')
