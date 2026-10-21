"""Complete table clipboard and per-cell edges over native JSON/MCP, with independent OOXML checks."""
import argparse
import io
import json
import re
import zipfile
import xml.etree.ElementTree as ET
from agent import ROOT, Session
from table_styles import package, W


def scenario(target, output, mcp):
    app=Session(target,output,mcp)
    folder=ROOT/output/'cell-border-tests';folder.mkdir(parents=True,exist_ok=True)
    try:
        def edit(name,data=None):return app.invoke(name,data,app.state()['revision'])
        def export():return app.resource(app.invoke('document.export')['resources'][0]['uri'])
        def table(t):return app.invoke('document.table.inspect',{'table':t,'limit':32})['value']
        def change(t,operation,**kw):return edit('document.table.change',dict(table=t,operation=operation,**kw))
        edit('document.table.insert',dict(paragraph=1,offset=0,rows=5,columns=3));tid=app.state()['tables'][0]['id']
        for r,row in enumerate(table(tid)['rows']):
            for c,cell in enumerate(row['cells']):edit('document.text.replace',dict(paragraph=cell['paragraphs'][0],start=0,end=0,text=f'Messwert {r+1}.{c+1}'))
        change(tid,'merge',row=0,column=0,rows=2,columns=2)
        change(tid,'headerRows',headerRows=2)
        edit('document.table.styles.put',dict(id='StudyBase',name='Studie',basedOn='jfTableBlue',band1={'fill':'DDEBDE'}))
        edit('document.table.styles.put',dict(id='StudyChild',name='Messwerte',basedOn='StudyBase',applyTo=tid))
        edges=dict(top=dict(style='double',width=12,color=0x264B78),right=dict(style='single',width=8,color=0x356A62),bottom=dict(style='dashed',width=12,color=0x9C4B48),left=dict(style='dotted',width=8,color=0x665C8E))
        change(tid,'borders',row=0,column=0,borders=edges)
        for r,width in [(0,8),(1,24)]:change(tid,'borders',row=r,column=2,borders={'left':dict(style='single',width=width,color=0x123456)})
        assert table(tid)['rows'][0]['cells'][0]['borders']==edges
        layout=app.invoke('document.table.layout',{'table':tid})['value']
        shared=[e for e in layout['pages'][0]['cells'][0]['edges'] if e['side']=='right']
        assert len(shared)==2 and [e['border']['width'] for e in shared]==[8,24]
        snapshot=export();rev=app.state()['revision']
        for bad in [{'top':{'style':'single','width':0}}, {'top':{'style':'nil','color':1}}, {'left':{'style':'inherit','width':8}}, {}, {'diagonal':{'style':'single'}}]:
            app.invoke('document.table.change',dict(table=tid,operation='borders',row=0,column=0,borders=bad),rev,error=True)
            assert export()==snapshot and app.state()['revision']==rev
        app.invoke('document.table.change',dict(table=tid,operation='borders',row=0,column=0,borders={'top':{'style':'nil'}}),rev-1,error=True)
        assert export()==snapshot
        change(tid,'borders',row=0,column=0,borders={'top':{'style':'inherit'},'left':{'style':'nil'}})
        assert table(tid)['rows'][0]['cells'][0]['borders']['top'] is None
        edit('document.undo');assert export()==snapshot
        edit('document.redo');edit('document.undo');assert export()==snapshot
        first=table(tid)['rows'][0]['cells'][0]['paragraphs'][0]
        sid=edit('sources.put',dict(title='Tabellenmessung',year='2026',authors=[dict(given='Ada',family='Lovelace')]))['value']['id']
        edit('citations.insert',dict(paragraph=first,inline=0,offset=0,items=[dict(source=sid,locator='42')]))
        original=export();parts=package(original);root=ET.fromstring(parts['word/document.xml']);tc=root.find('.//'+W+'tc')
        assert [n.tag.removeprefix(W) for n in tc.find(W+'tcPr/'+W+'tcBorders')]==['top','left','bottom','right']
        assert tc.find(W+'tcPr/'+W+'tcBorders/'+W+'top').get(W+'val')=='double'
        assert tc.find(W+'tcPr/'+W+'tcBorders/'+W+'left').get(W+'sz')=='8'
        path=folder/f'{target}-{"mcp" if mcp else "json"}.docx';path.write_bytes(original)
        app.invoke('clipboard.table.copy',{'table':tid})
        # Nested paste fits the copied table to the containing cell and remains one undo step.
        app.invoke('document.select',dict(paragraph=first,start=0,end=0))
        edit('clipboard.paste')
        assert app.state()['totalTables']==2 and app.state()['tables'][1]['parent']==tid
        assert table(app.state()['tables'][1]['id'])['rows'][0]['cells'][0]['borders']==edges
        edit('document.undo');assert export()==original
        edit('document.new',{'discardChanges':True})
        edit('document.table.styles.put',dict(id='StudyBase',name='Studie',body={'fill':'FFCCCC'}))
        edit('document.text.replace',dict(paragraph=1,start=0,end=0,text='VorNach'))
        app.invoke('document.select',dict(paragraph=1,start=3,end=3));before=export()
        edit('clipboard.paste');pasted=export();tables=app.state()['tables'];assert len(tables)==1
        pasted_id=tables[0]['id'];assert table(pasted_id)['rows'][0]['cells'][0]['borders']==edges
        assert table(pasted_id)['table']['headerRows']==2
        doc=ET.fromstring(package(pasted)['word/document.xml']);body=doc.find(W+'body')
        assert ''.join(body[0].itertext())=='Vor' and body[1].tag==W+'tbl' and ''.join(body[2].itertext())=='Nach'
        styles=app.invoke('document.table.styles.list',{'limit':32})['value']['styles']
        child=next(s for s in styles if s['id']==tables[0]['style'])
        assert child['basedOn']!='StudyBase'
        assert next(s for s in styles if s['id']=='StudyBase')['body']['fill']=='FFCCCC'
        edit('document.undo');assert export()==before
        edit('document.redo');assert export()==pasted
        app.invoke('clipboard.table.copy',{'table':pasted_id})
        app.invoke('document.select',dict(paragraph=tables[0]['before'],start=0,end=0))
        edit('clipboard.paste');assert len(app.state()['tables'])==2
        assert len(app.invoke('document.table.styles.list',{'limit':32})['value']['styles'])==len(styles)
        cut_id=app.state()['tables'][1]['id'];before=export()
        edit('clipboard.table.cut',{'table':cut_id});assert len(app.state()['tables'])==1
        edit('document.undo');assert export()==before
        edit('document.redo');edit('clipboard.paste');assert len(app.state()['tables'])==2
        final=export();saved=folder/f'{target}-pasted-{"mcp" if mcp else "json"}.docx'
        edit('file.save',dict(path=saved.relative_to(ROOT).as_posix(),overwrite=True))
        edit('document.new',{'discardChanges':True});edit('file.open',dict(path=saved.relative_to(ROOT).as_posix()));assert export()==final
        if not mcp:
            # Unknown cell drawing features must stay in preservation mode, not vanish.
            altered=dict(parts);altered['word/document.xml']=parts['word/document.xml'].replace(b'</w:tcBorders>',b'<w:tl2br w:val="single" w:sz="8" w:color="000000"/></w:tcBorders>',1)
            unknown=folder/f'{target}-diagonal.docx'
            with zipfile.ZipFile(unknown,'w',compression=zipfile.ZIP_DEFLATED) as z:
                for key,value in altered.items():z.writestr(key,value)
            edit('file.open',dict(path=unknown.relative_to(ROOT).as_posix(),discardChanges=True));assert app.state()['mode']!='native'
        print(f'PASS {target} {"MCP" if mcp else "JSON"}: cell edges, schema/revisions, merged segments, full table copy/cut/paste, collisions, undo and DOCX')
    finally:app.close()


def transfer(target,output):
    app=Session(target,output)
    try:
        files=sorted((ROOT/output/'cell-borders-browser').glob('*.docx'));assert len(files)==6,files
        for p in files:
            app.invoke('file.open',dict(path=p.relative_to(ROOT).as_posix(),discardChanges=True),app.state()['revision'])
            assert app.state()['mode']=='native'
            assert app.resource(app.invoke('document.export')['resources'][0]['uri'])==p.read_bytes()
            print(f'PASS {target}: exact browser table transfer {p.name}')
    finally:app.close()


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--target',choices=['win','linux','both'],default='both');p.add_argument('--out',default='build/wordcraft-cell-borders');p.add_argument('--web-transfer',action='store_true');a=p.parse_args()
    for target in ['win','linux'] if a.target=='both' else [a.target]:
        if a.web_transfer:transfer(target,a.out)
        else:
            for mcp in [False,True]:scenario(target,a.out,mcp)
