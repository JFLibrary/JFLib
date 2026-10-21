"""Row fragmentation and cross-format policy preservation through native JSON/MCP."""
import argparse
import io
import zipfile
import xml.etree.ElementTree as ET
from agent import ROOT, Session

W='{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
FIX=ROOT/'apps/jfWord/fixtures/row-breaks.html'

def scenario(target,out,mcp):
    app=Session(target,out,mcp);folder=ROOT/out/'row-break-tests';folder.mkdir(parents=True,exist_ok=True)
    try:
        def edit(name,data=None):return app.invoke(name,data,app.state()['revision'])
        def export(fmt='docx'):return app.resource(app.invoke('document.export',{'format':fmt})['resources'][0]['uri'])
        def inspect(t):return app.invoke('document.table.inspect',{'table':t,'limit':32})['value']
        def layout(t):return app.invoke('document.table.layout',{'table':t,'limit':8})['value']
        def change(t,operation,**data):return edit('document.table.change',dict(table=t,operation=operation,**data))
        edit('file.open',dict(path=FIX.relative_to(ROOT).as_posix(),discardChanges=True))
        t=app.state()['tables'][0]['id'];rows=inspect(t)['rows'];cell=rows[0]['cells'][0]['id'];paragraph=rows[0]['cells'][0]['paragraphs'][0]
        assert rows[0]['allowSplit'] and any(c['id']==cell and not c['rowEnd'] for c in layout(t)['pages'][0]['cells'])
        old=export();change(t,'rowBreak',row=0,allowSplit=False);kept=export();g=layout(t)
        assert not inspect(t)['rows'][0]['allowSplit'] and not any(c['id']==cell for c in g['pages'][0]['cells'])
        assert len([c for p in g['pages'] for c in p['cells'] if c['id']==cell])==1
        edit('document.undo');assert export()==old;edit('document.redo');assert export()==kept
        for args in [dict(operation='rowBreak',row=100,allowSplit=False),dict(operation='rowBreak',row=0),dict(operation='rowBreak',row=0,allowSplit='no')]:
            rev=app.state()['revision'];app.invoke('document.table.change',dict(table=t,**args),rev,error=True);assert export()==kept and app.state()['revision']==rev
        xml=ET.fromstring(zipfile.ZipFile(io.BytesIO(kept)).read('word/document.xml'));tr=xml.find('.//'+W+'tr')
        assert tr.find(W+'trPr/'+W+'cantSplit') is not None
        for fmt in ['html','rtf']:
            data=export(fmt);assert (b'break-inside:avoid' if fmt=='html' else b'\\trkeep ') in data
            f=folder/f'{target}-{mcp}.{fmt}';f.write_bytes(data);edit('file.open',dict(path=f.relative_to(ROOT).as_posix(),discardChanges=True))
            t=app.state()['tables'][0]['id'];r=inspect(t)['rows'];assert not r[0]['allowSplit'] and r[1]['allowSplit']
        # Standard Word boolean encodings, including absent values and explicit false.
        with zipfile.ZipFile(io.BytesIO(kept)) as z:parts={n:z.read(n) for n in z.namelist()}
        for value,expected in [(None,False),('true',False),('on',False),('1',False),('false',True),('off',True),('0',True)]:
            tag=b'<w:cantSplit/>' if value is None else f'<w:cantSplit w:val="{value}"/>'.encode()
            data=io.BytesIO()
            with zipfile.ZipFile(data,'w') as z:
                for name,content in parts.items():z.writestr(name,content.replace(b'<w:cantSplit/>',tag) if name=='word/document.xml' else content)
            f=folder/f'{target}-{mcp}-boolean.docx';f.write_bytes(data.getvalue());edit('file.open',dict(path=f.relative_to(ROOT).as_posix(),discardChanges=True))
            assert app.state()['mode']=='native';t=app.state()['tables'][0]['id'];assert inspect(t)['rows'][0]['allowSplit']==expected
        change(t,'rowBreak',row=0,allowSplit=False);change(t,'rowHeight',row=0,height=14000)
        g=layout(t);rows=inspect(t)['rows'];cell=rows[0]['cells'][0]['id'];paragraph=rows[0]['cells'][0]['paragraphs'][0]
        assert g['pageCount']>=4 and not any(c['id']==cell for c in g['pages'][0]['cells'])
        assert len([c for p in g['pages'] for c in p['cells'] if c['id']==cell and c['rowEnd']])==1
        edit('document.text.replace',dict(paragraph=paragraph,start=0,end=0,text='Fortsetzung: '));before=export()
        app.invoke('clipboard.table.copy',{'table':t});anchor=app.state()['tables'][0]['before'];edit('document.select',dict(paragraph=anchor,start=0,end=0));edit('clipboard.paste')
        assert app.state()['totalTables']==2 and all(not inspect(v['id'])['rows'][0]['allowSplit'] for v in app.state()['tables'])
        edit('document.undo');assert export()==before
        conflict=ROOT/'apps/jfWord/fixtures/interleaved-row-breaks.html'
        edit('file.open',dict(path=conflict.relative_to(ROOT).as_posix(),discardChanges=True))
        assert layout(app.state()['tables'][0]['id'])['pageCount']>1
        print(f'PASS {target} {"MCP" if mcp else "JSON"}: row splits/keep, tall fallback, XML booleans, HTML/RTF, clipboard, undo and atomic errors')
    finally:app.close()

def transfer(target,out):
    files=sorted((ROOT/out/'row-breaks-browser').glob('*.docx'));assert len(files)==6,files
    app=Session(target,out)
    try:
        for f in files:
            app.invoke('file.open',dict(path=f.relative_to(ROOT).as_posix(),discardChanges=True),app.state()['revision'])
            assert app.state()['mode']=='native';t=app.state()['tables'][0]['id']
            assert not app.invoke('document.table.inspect',{'table':t})['value']['rows'][0]['allowSplit']
            assert app.resource(app.invoke('document.export')['resources'][0]['uri'])==f.read_bytes()
            print(f'PASS {target} exact row-policy DOCX {f.name}')
    finally:app.close()

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--target',choices=['win','linux','both'],default='both');p.add_argument('--out',default='build/wordcraft-row-breaks');p.add_argument('--web-transfer',action='store_true');a=p.parse_args()
    for target in ['win','linux'] if a.target=='both' else [a.target]:
        if a.web_transfer:transfer(target,a.out)
        else:
            for mcp in [False,True]:scenario(target,a.out,mcp)
