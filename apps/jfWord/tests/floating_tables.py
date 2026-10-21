"""Real JSON/MCP processes: floating geometry, varying RTF grids and independent OOXML."""
import argparse
import io
import zipfile
import xml.etree.ElementTree as ET
from agent import ROOT, Session

W='{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
FIX=ROOT/'apps/jfWord/fixtures'

def scenario(target,out,mcp):
    app=Session(target,out,mcp)
    folder=ROOT/out/'floating-table-tests';folder.mkdir(parents=True,exist_ok=True)
    try:
        def edit(name,data=None):return app.invoke(name,data,app.state()['revision'])
        def export():return app.resource(app.invoke('document.export')['resources'][0]['uri'])
        def inspect(t):return app.invoke('document.table.inspect',{'table':t,'limit':32})['value']
        def geometry(t):return app.invoke('document.table.layout',{'table':t,'limit':8})['value']
        def change(t,operation,**data):return edit('document.table.change',dict(table=t,operation=operation,**data))
        for name in ['varying-grid-tables','ragged-grid-tables']:
            original=export();edit('document.import',dict(paragraph=1,start=0,end=0,format='rtf',text=(FIX/f'{name}.rtf').read_text()))
            state=app.state();t=state['tables'][0]['id'];info=inspect(t);data=export()
            root=ET.fromstring(zipfile.ZipFile(io.BytesIO(data)).read('word/document.xml'))
            assert root.find('.//'+W+'gridSpan') is not None
            if name.startswith('varying'):
                assert info['table']['columns']==[1800,200,1600,1800]
                assert len(root.findall('.//'+W+'tc'))==5
            else:
                assert info['rows'][1]['gridBefore']==1 and info['rows'][1]['gridAfter']==1
                assert info['rows'][1]['cells'][0]['omitted']
                assert len(root.findall('.//'+W+'tc'))==4
                assert root.find('.//'+W+'gridBefore').get(W+'val')=='1'
                assert root.find('.//'+W+'gridAfter').get(W+'val')=='1'
            assert geometry(t)['pageCount']==1
            edit('document.undo');assert export()==original
            edit('document.redo');assert export()==data
            path=folder/f'{target}-{mcp}-{name}.docx';path.write_bytes(data)
            edit('file.open',dict(path=path.relative_to(ROOT).as_posix(),discardChanges=True));assert export()==data
            edit('document.new',dict(discardChanges=True))
        edit('document.text.replace',dict(paragraph=1,start=0,end=0,text='Text neben der Tabelle und darunter wieder über die volle Seitenbreite. '*85))
        t=edit('document.table.insert',dict(paragraph=1,offset=0,rows=3,columns=2))['value']['id']
        change(t,'columns',widths=[1440,1440]);change(t,'rowHeight',row=0,height=660)
        change(t,'style',style='jfTableBlue');cell=inspect(t)['rows'][0]['cells'][0]['paragraphs'][0]
        edit('document.text.replace',dict(paragraph=cell,start=0,end=0,text='Schwebende Tabelle'))
        inline=export()
        change(t,'position',floating=True,x=0,y=0,horizontal='margin',vertical='paragraph',distance=[120,180,120,180],overlap=False)
        data=export();root=ET.fromstring(zipfile.ZipFile(io.BytesIO(data)).read('word/document.xml'))
        pos=root.find('.//'+W+'tblpPr');assert pos.get(W+'horzAnchor')=='margin' and pos.get(W+'rightFromText')=='180'
        assert root.find('.//'+W+'tblOverlap').get(W+'val')=='never'
        g=geometry(t);first=g['pages'][0];bounds=first['cells'][0]['bounds'];lines=[l['bounds'] for l in first['bodyLines'] if l['paragraph']==1]
        assert lines[0][0]>=bounds[0]+2880+180,(bounds,lines[0])
        assert any(l[0]==app.state()['page']['left'] for l in lines),lines
        assert g['pageCount']>1 and not g['pages'][1]['cells']
        for line in lines:
            for c in first['cells']:
                b=c['bounds'];assert not (line[0]<b[0]+b[2] and b[0]<line[0]+line[2] and line[1]<b[1]+b[3] and b[1]<line[1]+line[3])
        edit('document.undo');assert export()==inline;edit('document.redo');assert export()==data
        # Layout failures, invalid geometry and stale revisions leave bytes/history intact.
        for bad in [dict(operation='position',floating=True,x=31680),dict(operation='rowHeight',row=0,height=31681),dict(operation='position',floating=False,x=0)]:
            rev=app.state()['revision'];app.invoke('document.table.change',dict(table=t,**bad),rev,error=True)
            assert app.state()['revision']==rev and export()==data
        app.invoke('document.table.change',dict(table=t,operation='position',floating=True,x=90),app.state()['revision']-1,error=True)
        assert export()==data
        # Position values may be negative relative to margins while staying on the physical page.
        change(t,'position',floating=True,x=-180,y=720,horizontal='margin',vertical='page')
        assert geometry(t)['pages'][0]['cells'][0]['bounds'][:2]==[app.state()['page']['left']-180,720]
        edit('document.undo');assert export()==data
        app.invoke('clipboard.table.copy',{'table':t});edit('document.select',dict(paragraph=1,start=0,end=0));edit('clipboard.paste')
        assert app.state()['totalTables']==2 and all(t['position'] is not None for t in app.state()['tables'])
        edit('document.undo');assert export()==data
        path=folder/f'{target}-{"mcp" if mcp else "json"}-floating.docx';path.write_bytes(data)
        edit('file.open',dict(path=path.relative_to(ROOT).as_posix(),discardChanges=True));assert export()==data
        edit('document.new',dict(discardChanges=True))
        edit('file.open',dict(path=(FIX/'ragged-grid-tables.rtf').relative_to(ROOT).as_posix(),discardChanges=True))
        assert app.state()['path']=='' and app.state()['dirty'] and inspect(app.state()['tables'][0]['id'])['rows'][1]['gridBefore']==1
        print(f'PASS {target} {"MCP" if mcp else "JSON"}: floating wrap, RTF grids/gaps, XML properties, atomic failures, clipboard, undo and exact reload')
    finally:app.close()

def transfer(target,out):
    files=sorted((ROOT/out/'floating-tables-browser').glob('*.docx'));assert len(files)==6,files
    app=Session(target,out)
    try:
        for f in files:
            app.invoke('file.open',dict(path=f.relative_to(ROOT).as_posix(),discardChanges=True),app.state()['revision'])
            assert app.state()['mode']=='native'
            assert app.resource(app.invoke('document.export')['resources'][0]['uri'])==f.read_bytes()
            print(f'PASS {target}: exact floating browser document {f.name}')
    finally:app.close()

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--target',choices=['win','linux','both'],default='both');p.add_argument('--out',default='build/wordcraft-floating-tables');p.add_argument('--web-transfer',action='store_true');a=p.parse_args()
    for target in ['win','linux'] if a.target=='both' else [a.target]:
        if a.web_transfer:transfer(target,a.out)
        else:
            for mcp in [False,True]:scenario(target,a.out,mcp)
