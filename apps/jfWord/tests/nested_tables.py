"""Real native JSON/MCP sessions and independent nested OOXML inspection."""
import argparse
import io
import json
import zipfile
import xml.etree.ElementTree as ET
from agent import ROOT, Session

W='{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
FIX=ROOT/'apps/jfWord/fixtures'

def scenario(target,out,mcp):
    app=Session(target,out,mcp)
    folder=ROOT/out/'nested-table-tests';folder.mkdir(parents=True,exist_ok=True)
    try:
        def edit(action,data=None):return app.invoke(action,data,app.state()['revision'])
        def export():return app.resource(app.invoke('document.export')['resources'][0]['uri'])
        def table(t):return app.invoke('document.table.inspect',{'table':t,'limit':32})['value']
        edit('document.text.replace',dict(paragraph=1,start=0,end=0,text='VorNach'))
        for fmt,count in [('html',3),('rtf',2)]:
            before=export();content=(FIX/f'nested-tables.{fmt}').read_text(encoding='utf-8')
            edit('document.import',dict(paragraph=1,start=3,end=3,format=fmt,text=content))
            state=app.state();assert state['totalTables']==count,state
            assert max(t['depth'] for t in state['tables'])==count
            assert state['tables'][0]['parent'] is None and state['tables'][1]['parent']==state['tables'][0]['id']
            data=export();root=ET.fromstring(zipfile.ZipFile(io.BytesIO(data)).read('word/document.xml'))
            assert len(list(root.iter(W+'tbl')))==count
            assert root.find('.//'+W+'tc/'+W+'tbl') is not None
            assert root.find('.//'+W+'tblHeader') is not None
            assert 'Vor' in ''.join(root.itertext()) and 'Nach' in ''.join(root.itertext())
            edit('document.undo');assert export()==before
            edit('document.redo');assert export()==data
            for bad in ['<table><tr><td rowspan=999>x</table>','<table><tr><td><img src="https://invalid.invalid"></table>']:
                rev=app.state()['revision'];app.invoke('document.import',dict(paragraph=1,start=0,end=0,format='html',text=bad),rev,error=True)
                assert export()==data and app.state()['revision']==rev
            path=folder/f'{target}-{"mcp" if mcp else "json"}-{fmt}.docx';path.write_bytes(data)
            edit('file.open',dict(path=path.relative_to(ROOT).as_posix(),discardChanges=True));assert export()==data
            # Complete child subtree copy/paste into another cell, retaining rich runs.
            root_id=app.state()['tables'][0]['id'];child=app.state()['tables'][1]['id']
            destination=table(root_id)['rows'][1]['cells'][1]['paragraphs'][0]
            app.invoke('clipboard.table.copy',{'table':child});edit('document.select',dict(paragraph=destination,start=0,end=0))
            edit('clipboard.paste');assert app.state()['totalTables']==count+(count-1)
            pasted=export();edit('document.undo');assert export()==data;edit('document.redo');assert export()==pasted
            edit('document.undo');rev=app.state()['revision']
            app.invoke('document.table.change',dict(table=root_id,operation='columns',widths=[420,420]),rev,error=True)
            assert export()==data and app.state()['revision']==rev
            # Opening an interchange file leaves it untouched and starts an unsaved DOCX.
            original=(FIX/f'nested-tables.{fmt}').read_bytes()
            edit('file.open',dict(path=(FIX/f'nested-tables.{fmt}').relative_to(ROOT).as_posix(),discardChanges=True))
            assert app.state()['path']=='' and app.state()['dirty'] and app.state()['totalTables']==count
            assert (FIX/f'nested-tables.{fmt}').read_bytes()==original
            edit('document.new',dict(discardChanges=True));edit('document.text.replace',dict(paragraph=1,start=0,end=0,text='VorNach'))
        print(f'PASS {target} {"MCP" if mcp else "JSON"}: nested RTF/HTML, OOXML tree, fitted clipboard, atomic failures, file import and undo')
    finally:app.close()

def transfer(target,out):
    files=sorted((ROOT/out/'nested-tables-browser').glob('*.docx'));assert len(files)==6,files
    app=Session(target,out)
    try:
        for f in files:
            app.invoke('file.open',dict(path=f.relative_to(ROOT).as_posix(),discardChanges=True),app.state()['revision'])
            assert app.state()['mode']=='native'
            assert app.resource(app.invoke('document.export')['resources'][0]['uri'])==f.read_bytes()
            print(f'PASS {target}: exact nested browser document {f.name}')
    finally:app.close()

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--target',choices=['win','linux','both'],default='both');p.add_argument('--out',default='build/wordcraft-nested-tables');p.add_argument('--web-transfer',action='store_true');a=p.parse_args()
    for target in ['win','linux'] if a.target=='both' else [a.target]:
        if a.web_transfer:transfer(target,a.out)
        else:
            for mcp in [False,True]:scenario(target,a.out,mcp)
