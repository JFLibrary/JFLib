"""Exact row sizing through native JSON/MCP, independent DOCX XML and clipboard exchange."""
import argparse
import io
import zipfile
import xml.etree.ElementTree as ET
from agent import ROOT, Session

W='{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
FIX=ROOT/'apps/jfWord/fixtures/exact-row-heights.html'

def scenario(target,out,mcp):
    app=Session(target,out,mcp);folder=ROOT/out/'exact-row-tests';folder.mkdir(parents=True,exist_ok=True)
    try:
        def edit(name,data=None):return app.invoke(name,data,app.state()['revision'])
        def export(fmt='docx'):return app.resource(app.invoke('document.export',{'format':fmt})['resources'][0]['uri'])
        def inspect(t):return app.invoke('document.table.inspect',{'table':t,'limit':32})['value']
        def layout(t):return app.invoke('document.table.layout',{'table':t,'limit':8})['value']
        def change(t,**data):return edit('document.table.change',dict(table=t,operation='rowHeight',row=0,**data))
        def height(t,cell):return sum(c['bounds'][3] for p in layout(t)['pages'] for c in p['cells'] if c['id']==cell and not c['repeated'])
        def openfile(path):return edit('file.open',dict(path=path.relative_to(ROOT).as_posix(),discardChanges=True))
        openfile(FIX);t=app.state()['tables'][0]['id'];rows=inspect(t)['rows'];cell=rows[0]['cells'][0]['id'];p=rows[0]['cells'][0]['paragraphs'][0]
        text=next(x['text'] for x in app.state()['paragraphs'] if x['id']==p)
        assert rows[0]['heightRule']=='exact' and height(t,cell)==480
        assert rows[1]['heightRule']=='atLeast' and height(t,rows[1]['cells'][0]['id'])>480
        original=export();change(t,height=480,heightRule='atLeast');assert height(t,cell)>480
        minimum=export();edit('document.undo');assert export()==original;edit('document.redo');assert export()==minimum
        change(t,height=350,heightRule='exact');exact=export();assert height(t,cell)==350
        xml=ET.fromstring(zipfile.ZipFile(io.BytesIO(exact)).read('word/document.xml'))
        h=xml.find('.//'+W+'tr/'+W+'trPr/'+W+'trHeight');assert h.attrib=={W+'val':'350',W+'hRule':'exact'}
        rev=app.state()['revision']
        for bad in [dict(height=0,heightRule='exact'),dict(height=31681,heightRule='exact'),dict(height=400,heightRule='wrong'),dict(height=400,heightRule=1)]:
            app.invoke('document.table.change',dict(table=t,operation='rowHeight',row=0,**bad),rev,error=True)
            assert export()==exact and app.state()['revision']==rev
        app.invoke('document.table.change',dict(table=t,operation='rowHeight',row=0,height=600),rev-1,error=True)
        assert export()==exact
        change(t,height=710);assert inspect(t)['rows'][0]['heightRule']=='exact' and height(t,cell)==710
        edit('document.undo');assert export()==exact
        # Hidden content remains fully editable and survives increasing the height.
        edit('document.text.replace',dict(paragraph=p,start=len(text.encode()),end=len(text.encode()),text=' UNSICHTBAR'))
        assert height(t,cell)==350
        change(t,height=350,heightRule='atLeast');assert height(t,cell)>350
        assert next(x['text'] for x in app.state()['paragraphs'] if x['id']==p).endswith('UNSICHTBAR')
        change(t,height=350,heightRule='exact');exact=export()
        for fmt in ['html','rtf']:
            data=export(fmt);assert (b'data-jf-row-height="1 exact 350"' if fmt=='html' else b'\\trrh-350 ') in data
            f=folder/f'{target}-{mcp}.{fmt}';f.write_bytes(data);openfile(f);t=app.state()['tables'][0]['id'];rows=inspect(t)['rows'];cell=rows[0]['cells'][0]['id']
            assert rows[0]['heightRule']=='exact' and rows[0]['height']==350 and height(t,cell)==350
            assert rows[1]['heightRule']=='atLeast' and b'UNSICHTBAR' in export('html')
        # Standard OOXML heights: exact, atLeast, explicit auto and default auto.
        with zipfile.ZipFile(io.BytesIO(exact)) as z:parts={n:z.read(n) for n in z.namelist()}
        for attrs,rule,value in [('w:val="347" w:hRule="exact"','exact',347),('w:val="347" w:hRule="atLeast"','atLeast',347),('w:val="347" w:hRule="auto"','atLeast',0),('w:val="347"','atLeast',0),('','atLeast',0)]:
            data=io.BytesIO();tag=f'<w:trHeight {attrs}/>'.encode()
            with zipfile.ZipFile(data,'w') as z:
                for name,content in parts.items():z.writestr(name,content.replace(b'<w:trHeight w:val="350" w:hRule="exact"/>',tag,1) if name=='word/document.xml' else content)
            f=folder/f'{target}-{mcp}-height.docx';f.write_bytes(data.getvalue());openfile(f)
            assert app.state()['mode']=='native';t=app.state()['tables'][0]['id'];r=inspect(t)['rows'][0];assert (r['heightRule'],r['height'])==(rule,value)
        change(t,height=347,heightRule='exact');before=export();app.invoke('clipboard.table.copy',{'table':t})
        anchor=app.state()['tables'][0]['before'];edit('document.select',dict(paragraph=anchor,start=0,end=0));edit('clipboard.paste')
        assert app.state()['totalTables']==2 and all(inspect(t['id'])['rows'][0]['heightRule']=='exact' for t in app.state()['tables'])
        edit('document.undo');assert export()==before
        edit('document.table.change',dict(table=t,operation='position',floating=True,distance=[0,0,0,0]));cell=inspect(t)['rows'][0]['cells'][0]['id']
        assert height(t,cell)==347;change(t,height=15000);g=layout(t)
        assert g['pageCount']>=3 and height(t,cell)==15000
        assert len([c for page in g['pages'] for c in page['cells'] if c['id']==cell and c['rowEnd']])==1
        f=folder/f'{target}-{mcp}-final.docx';f.write_bytes(export());openfile(f);assert export()==f.read_bytes()
        print(f'PASS {target} {"MCP" if mcp else "JSON"}: exact/minimum, overflow retention, errors/revisions, XML rules, HTML/RTF, clipboard, floating pages and exact reload')
    finally:app.close()

def transfer(target,out):
    files=sorted((ROOT/out/'exact-rows-browser').glob('*.docx'));assert len(files)==6,files
    app=Session(target,out)
    try:
        for f in files:
            app.invoke('file.open',dict(path=f.relative_to(ROOT).as_posix(),discardChanges=True),app.state()['revision'])
            assert app.state()['mode']=='native';t=app.state()['tables'][0]['id']
            assert app.invoke('document.table.inspect',{'table':t})['value']['rows'][0]['heightRule']=='exact'
            assert app.resource(app.invoke('document.export')['resources'][0]['uri'])==f.read_bytes()
            print(f'PASS {target} exact row-height DOCX {f.name}')
    finally:app.close()

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--target',choices=['win','linux','both'],default='both');p.add_argument('--out',default='build/wordcraft-exact-rows');p.add_argument('--web-transfer',action='store_true');a=p.parse_args()
    for target in (['win','linux'] if a.target=='both' else [a.target]):
        if a.web_transfer:transfer(target,a.out)
        else:
            for mcp in [False,True]:scenario(target,a.out,mcp)
