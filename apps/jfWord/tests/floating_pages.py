"""Multi-page floating tables through JSON/MCP, real fonts and independent HTML/OOXML inspection."""
import argparse
import io
import zipfile
import xml.etree.ElementTree as ET
from html.parser import HTMLParser
from agent import ROOT, Session

W='{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
FIX=ROOT/'apps/jfWord/fixtures/floating-pages.html'

class Markup(HTMLParser):
    def __init__(self,data):
        super().__init__();self.tables=[];self.page=None;self.feed(data.decode())
    def handle_starttag(self,tag,attrs):
        a=dict(attrs)
        if tag=='table':self.tables.append(a)
        if 'data-jf-page' in a:self.page=a['data-jf-page']

def scenario(target,out,mcp):
    app=Session(target,out,mcp);folder=ROOT/out/'floating-pages-tests';folder.mkdir(parents=True,exist_ok=True)
    try:
        def edit(name,data=None):return app.invoke(name,data,app.state()['revision'])
        def export(fmt='docx'):
            r=app.invoke('document.export',{'format':fmt})['resources'][0]
            assert r['mimeType']=={'html':'text/html','rtf':'application/rtf','docx':'application/vnd.openxmlformats-officedocument.wordprocessingml.document'}[fmt],r
            return app.resource(r['uri'])
        def inspect(t):return app.invoke('document.table.inspect',{'table':t,'limit':32})['value']
        def layout(t):return app.invoke('document.table.layout',{'table':t,'limit':8})['value']
        def change(t,operation,**data):return edit('document.table.change',dict(table=t,operation=operation,**data))
        edit('file.open',dict(path=FIX.relative_to(ROOT).as_posix(),discardChanges=True))
        d=app.state();t=d['tables'][0]['id'];position=d['tables'][0]['position'];g=layout(t)
        assert d['page']['height']==5760 and g['pageCount']==4,g
        assert all(len([c for c in p['cells'] if c['repeated']])==2 for p in g['pages'][1:])
        originals=[c for p in g['pages'] for c in p['cells'] if not c['repeated']]
        assert len(originals)==24 and len({c['id'] for c in originals})==24
        assert g['pages'][0]['cells'][0]['bounds'][:2]==[900,990]
        assert all(min(c['bounds'][1] for c in p['cells'])==720 and all(c['bounds'][0]>=900 for c in p['cells']) for p in g['pages'][1:])
        source=export();xml=ET.fromstring(zipfile.ZipFile(io.BytesIO(source)).read('word/document.xml'));pos=xml.find('.//'+W+'tblpPr')
        assert pos.get(W+'tblpX')=='180' and pos.get(W+'tblpY')=='270' and pos.get(W+'leftFromText')=='150'
        revision=app.state()['revision'];html=export('html');h=Markup(html)
        assert h.tables[0]['data-jf-table-position']=='1 margin paragraph 180 270 60 120 90 150 0'
        assert h.page=='1 8640 5760 720 720 720 720' and 'float:left' in h.tables[0]['style']
        assert app.state()['revision']==revision and export()==source
        path=folder/f'{target}-{mcp}.html';path.write_bytes(html)
        edit('file.open',dict(path=path.relative_to(ROOT).as_posix(),discardChanges=True));d=app.state();t=d['tables'][0]['id']
        assert d['tables'][0]['position']==position and layout(t)['pageCount']==4
        p=inspect(t)['rows'][10]['cells'][0]['paragraphs'][0]
        before=export();edit('document.text.replace',dict(paragraph=p,start=0,end=0,text='Fortsetzung: '))
        assert any('Fortsetzung' in v['text'] for v in app.state()['paragraphs'])
        edit('document.undo');assert export()==before;edit('document.redo');changed=export()
        change(t,'rowHeight',row=1,height=18000);g=layout(t);assert g['pageCount']>5
        cell=inspect(t)['rows'][1]['cells'][0]['id'];pieces=[c for page in g['pages'] for c in page['cells'] if c['id']==cell]
        assert len(pieces)>3 and all(not any(e['side']=='top' for e in c['edges']) for c in pieces[1:])
        edit('document.undo');assert export()==changed
        for bad in [dict(operation='rowHeight',row=0,height=4320),dict(operation='position',floating=True,y=5000)]:
            rev=app.state()['revision'];app.invoke('document.table.change',dict(table=t,**bad),rev,error=True)
            assert app.state()['revision']==rev and export()==changed
        malformed='<table data-jf-table-position="9 margin paragraph 0 0 0 0 0 0 0"><tr><td>x</td></tr></table>'
        rev=app.state()['revision'];app.invoke('document.import',dict(paragraph=p,start=0,end=0,format='html',text=malformed),rev,error=True)
        assert app.state()['revision']==rev and export()==changed
        # The clipboard private and HTML representations preserve positions through the common codecs.
        app.invoke('clipboard.table.copy',{'table':t});anchor=app.state()['tables'][0]['before']
        edit('document.select',dict(paragraph=anchor,start=0,end=0));edit('clipboard.paste')
        assert app.state()['totalTables']==2 and all(v['position']==position for v in app.state()['tables'])
        edit('document.undo');assert export()==changed
        path=folder/f'{target}-{mcp}.docx';path.write_bytes(changed)
        edit('file.open',dict(path=path.relative_to(ROOT).as_posix(),discardChanges=True));assert export()==changed
        assert layout(app.state()['tables'][0]['id'])['pageCount']==4
        print(f'PASS {target} {"MCP" if mcp else "JSON"}: four floating pages, headers, continuation edits, HTML positions, row splitting, atomic failures and exact DOCX reload')
    finally:app.close()

def transfer(target,out):
    files=sorted((ROOT/out/'floating-pages-browser').glob('*.docx'));assert len(files)==6,files
    app=Session(target,out)
    try:
        for f in files:
            app.invoke('file.open',dict(path=f.relative_to(ROOT).as_posix(),discardChanges=True),app.state()['revision'])
            assert app.state()['mode']=='native'
            assert app.resource(app.invoke('document.export')['resources'][0]['uri'])==f.read_bytes()
            assert app.state()['tables'][0]['position'] is not None
            print(f'PASS {target}: exact floating continuation document {f.name}')
    finally:app.close()

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--target',choices=['win','linux','both'],default='both');p.add_argument('--out',default='build/wordcraft-floating-pages');p.add_argument('--web-transfer',action='store_true');a=p.parse_args()
    for target in ['win','linux'] if a.target=='both' else [a.target]:
        if a.web_transfer:transfer(target,a.out)
        else:
            for mcp in [False,True]:scenario(target,a.out,mcp)
