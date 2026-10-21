"""Live table-style inheritance and conditional regions, JSON/MCP and independent OOXML."""
import argparse
import re
import zipfile
import xml.etree.ElementTree as ET
from agent import ROOT, Session
from table_styles import package, W


def scenario(target, output, mcp):
    app = Session(target, output, mcp)
    folder = ROOT / output / 'table-graph-tests'
    folder.mkdir(parents=True, exist_ok=True)
    try:
        def edit(name, data=None): return app.invoke(name, data, app.state()['revision'])
        def styles(): return app.invoke('document.table.styles.list', {'limit': 32})['value']['styles']
        def style(key): return next(s for s in styles() if s['id'] == key)
        def table(): return app.invoke('document.table.inspect', dict(table=tid, limit=32))['value']
        def fill(r,c): return table()['rows'][r]['cells'][c]['effectiveFill']
        def change(operation, **kw): return edit('document.table.change', dict(table=tid, operation=operation, **kw))
        def put(key, **kw): return edit('document.table.styles.put', dict(id=key, name=key, **kw))
        def export(): return app.resource(app.invoke('document.export')['resources'][0]['uri'])
        edit('document.table.insert', dict(paragraph=1, offset=0, rows=8, columns=6))
        tid=app.state()['tables'][0]['id']
        for r, row in enumerate(table()['rows']):
            for c, cell in enumerate(row['cells']):
                edit('document.text.replace', dict(paragraph=cell['paragraphs'][0],start=0,end=0,text=f'R{r+1} C{c+1}'))
        put('ResearchBase', basedOn='jfTableBlue', rowBandSize=2, columnBandSize=2,
            band1Vertical={'fill':'D5E9F3'},band2Vertical={'fill':'E4EBD0'},
            topLeft={'fill':'285942'},topRight={'fill':'624F76'},bottomLeft={'fill':'704B28'},bottomRight={'fill':'284C70'})
        put('ResearchChild', basedOn='ResearchBase', applyTo=tid, firstRow={'format':{'bold':False}})
        change('headerRows',headerRows=1)
        change('look',look=dict(firstRow=True,lastRow=True,firstColumn=True,lastColumn=True,bandedRows=True,bandedColumns=True))
        child=style('ResearchChild')
        assert child['basedOn']=='ResearchBase' and child['inheritBorder']
        assert child['effective']['columnBandSize']==2 and not child['effective']['firstRow']['format']['bold']
        assert child['effective']['firstRow']['format']['color']==0xffffff
        assert fill(0,0)=='285942' and fill(0,5)=='624F76' and fill(7,0)=='704B28' and fill(7,5)=='284C70'
        assert fill(1,1)==fill(2,2)=='D5E9F3' and fill(3,3)=='E4EBD0'
        before=table()
        put('ResearchBase', band1Vertical={'fill':'D8EBE5'}, borderWidth=12,borderColor=0x234567)
        assert fill(1,1)=='D8EBE5' and style('ResearchChild')['effective']['borderColor']==0x234567
        edit('document.undo'); assert table()==before
        edit('document.redo'); assert fill(1,1)=='D8EBE5'
        put('ResearchChild', topLeft={'fill':'FFFFFF'})
        put('ResearchBase', topLeft={'fill':'335577'})
        assert fill(0,0)=='FFFFFF'
        put('ResearchChild', topLeft={'fill':''}, firstRow={'format':{}})
        assert fill(0,0)=='335577' and style('ResearchChild')['effective']['firstRow']['format']['bold']
        # A detached copy remains independent when its former ancestor changes.
        put('Snapshot', cloneFrom='ResearchChild')
        assert style('Snapshot')['basedOn'] is None
        put('ResearchBase', band1Vertical={'fill':'D5E9F3'})
        assert style('Snapshot')['band1Vertical']['fill']=='D8EBE5' and fill(1,1)=='D5E9F3'
        revision=app.state()['revision']; snapshot=styles()
        for key, kwargs in [('ResearchBase',{'basedOn':'ResearchChild'}),('ResearchChild',{'basedOn':'missing'}),
                            ('ResearchChild',{'basedOn':'ResearchChild'}),('Bad',{'basedOn':'ResearchBase','cloneFrom':'jfTableBlue'}),
                            ('ResearchChild',{'inheritBorder':True,'borderWidth':4}),('ResearchChild',{'columnBandSize':33})]:
            app.invoke('document.table.styles.put', dict(id=key,name=key,**kwargs),revision,error=True)
            assert app.state()['revision']==revision and styles()==snapshot
        app.invoke('document.table.styles.put',dict(id='ResearchChild',name='ResearchChild',rowBandSize=1),revision-1,error=True)
        assert styles()==snapshot
        data=export(); parts=package(data); root=ET.fromstring(parts['word/styles.xml'])
        nodes={s.get(W+'styleId'):s for s in root.findall(W+'style') if s.get(W+'type')=='table'}
        assert 'jfTableBlue' in nodes
        assert nodes['ResearchChild'].find(W+'basedOn').get(W+'val')=='ResearchBase'
        assert nodes['ResearchChild'].find('.//'+W+'tblBorders') is None
        assert nodes['ResearchBase'].find('.//'+W+'tblStyleRowBandSize').get(W+'val')=='2'
        assert nodes['ResearchBase'].find('.//'+W+'tblStyleColBandSize').get(W+'val')=='2'
        assert {n.get(W+'type') for n in nodes['ResearchBase'].findall(W+'tblStylePr')} >= {'band1Vert','band2Vert','nwCell','neCell','swCell','seCell'}
        document=ET.fromstring(parts['word/document.xml'])
        assert document.find('.//'+W+'tblLook').get(W+'noVBand')=='0'
        path=folder/f'{target}-{"mcp" if mcp else "json"}.docx'
        edit('file.save',dict(path=path.relative_to(ROOT).as_posix(),overwrite=True))
        edit('document.new',{'discardChanges':True});edit('file.open',dict(path=path.relative_to(ROOT).as_posix()))
        assert app.state()['mode']=='native' and export()==data
        if not mcp:
            for label, look, expected in [('bitmask','<w:tblLook w:val="01E0"/>',(True,True,True,True,True,True)),
                    ('explicit-overrides-mask','<w:tblLook w:val="FFFF" w:firstRow="1" w:noVBand="1"/>',(True,False,False,False,True,False))]:
                patched=dict(parts); patched['word/document.xml']=re.sub(rb'<w:tblLook[^>]*/>',look.encode(),patched['word/document.xml'])
                altered=folder/f'{target}-{label}.docx'
                with zipfile.ZipFile(altered,'w',compression=zipfile.ZIP_DEFLATED) as z:
                    for key,value in patched.items(): z.writestr(key,value)
                edit('document.new',{'discardChanges':True});edit('file.open',dict(path=altered.relative_to(ROOT).as_posix()))
                assert app.state()['mode']=='native'
                actual=table()['table']['look']
                assert tuple(actual[k] for k in ['firstRow','lastRow','firstColumn','lastColumn','bandedRows','bandedColumns'])==expected
        print(f'{target} {"MCP" if mcp else "JSON"}: inherited templates, bands/corners, reset/undo, graph rejection, independent OOXML and DOCX passed')
    finally: app.close()


def transfer(target, output):
    app=Session(target,output)
    try:
        files=sorted((ROOT/output/'table-graphs-browser').glob('*.docx'))
        assert len(files)==6, files
        for p in files:
            app.invoke('file.open',dict(path=p.relative_to(ROOT).as_posix(),discardChanges=True),app.state()['revision'])
            assert app.state()['mode']=='native'
            assert app.resource(app.invoke('document.export')['resources'][0]['uri'])==p.read_bytes()
            print(f'{target}: exact browser style-graph roundtrip {p.name}')
    finally: app.close()


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--target',choices=['win','linux','both'],default='both')
    p.add_argument('--out',default='build/wordcraft-table-graphs');p.add_argument('--web-transfer',action='store_true');a=p.parse_args()
    for target in ['win','linux'] if a.target=='both' else [a.target]:
        for mcp in [False,True]:scenario(target,a.out,mcp)
        if a.web_transfer:transfer(target,a.out)
