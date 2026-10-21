"""Real JSON/MCP table workflows and independent XML/ZIP validation."""
import argparse
import io
import zipfile
import xml.etree.ElementTree as ET
from agent import ROOT, Session

def scenario(target, output, mcp):
    app=Session(target,output,mcp)
    try:
        def edit(name,data=None): return app.invoke(name,data,app.state()['revision'])
        def table(): return app.invoke('document.table.inspect',dict(table=tid))['value']
        edit('document.text.replace',dict(paragraph=1,start=0,end=0,text='Nach der Tabelle'))
        edit('document.table.insert',dict(paragraph=1,offset=0,rows=3,columns=3))
        tid=app.state()['tables'][0]['id']
        def change(operation,**kw): return edit('document.table.change',dict(table=tid,operation=operation,**kw))
        initial=table()
        for r,row in enumerate(initial['rows']):
            for c,cell in enumerate(row['cells']):
                p=cell['paragraphs'][0]
                value=[['Artikel','Menge','Preis'],['Äpfel','12','1,50 €'],['Birnen','7','2,10 €']][r][c]
                edit('document.text.replace',dict(paragraph=p,start=0,end=0,text=value))
                if r==0:
                    edit('document.text.format',dict(paragraph=p,start=0,end=len(value.encode()),format={'bold':True}))
                    change('cell',row=r,column=c,fill='C2E4F2',alignment='center')
        change('rowHeight',row=0,height=720)
        change('columns',widths=[3200,2000,2000])
        original=table()
        change('insertRow',index=1);assert table()['table']['rowCount']==4
        edit('document.undo');assert table()==original
        edit('document.redo');assert table()['table']['rowCount']==4
        change('removeRow',index=1)
        change('insertColumn',index=1);assert len(table()['table']['columns'])==4
        change('removeColumn',index=1);assert len(table()['table']['columns'])==3
        change('columns',widths=[3200,2000,2000])
        p=table()['rows'][1]['cells'][0]['paragraphs'][0]
        edit('document.text.replace',dict(paragraph=p,start=6,end=6,text='\nBio'))
        assert len(table()['rows'][1]['cells'][0]['paragraphs'])==2
        revision=app.state()['revision']
        if not mcp:
            app.invoke('document.table.change',dict(table=tid,operation='columns',widths=[1,1,1]),revision,error=True)
            app.invoke('document.table.change',dict(table=tid,operation='removeRow',index=300),revision,error=True)
            app.invoke('document.table.change',dict(table=tid,operation='delete'),revision-1,error=True)
            assert app.state()['revision']==revision
        expected=(table(),app.state()['paragraphs'])
        folder=ROOT/output/'table-tests';folder.mkdir(parents=True,exist_ok=True)
        path=(folder/f'{target}-{"mcp" if mcp else "json"}.docx').relative_to(ROOT).as_posix()
        edit('file.save',dict(path=path,overwrite=True))
        payload=app.resource(app.invoke('document.export')['resources'][0]['uri'])
        with zipfile.ZipFile(io.BytesIO(payload)) as z:
            assert z.testzip() is None
            xml=ET.fromstring(z.read('word/document.xml'))
            ns={'w':'http://schemas.openxmlformats.org/wordprocessingml/2006/main'}
            assert len(xml.findall('.//w:tbl',ns))==1 and len(xml.findall('.//w:tc',ns))==9
            assert len(xml.findall('.//w:tblGrid/w:gridCol',ns))==3
            assert b'urn:jfoffice:table-identities:1' in z.read('customXml/jfTables.xml')
        edit('document.new',dict(discardChanges=True));edit('file.open',dict(path=path))
        assert (table(),app.state()['paragraphs'])==expected
        assert app.resource(app.invoke('document.export')['resources'][0]['uri'])==payload
        # A standards-only table has no application IDs; allocate stable fresh identities.
        if not mcp:
            import re
            with zipfile.ZipFile(io.BytesIO(payload)) as archive:
                parts={name:archive.read(name) for name in archive.namelist() if name!='customXml/jfTables.xml'}
            parts['word/_rels/document.xml.rels']=re.sub(rb'<Relationship Id="rId6"[^>]*/>',b'',parts['word/_rels/document.xml.rels'])
            parts['word/document.xml']=re.sub(rb' w14:paraId="[^"]*"',b'',parts['word/document.xml'])
            def write_parts(name, data):
                fixture=folder/name
                with zipfile.ZipFile(fixture,'w',compression=zipfile.ZIP_DEFLATED) as archive:
                    for key,value in data.items(): archive.writestr(key,value)
                return fixture.relative_to(ROOT).as_posix()
            external=write_parts('standard-'+target+'.docx',parts)
            edit('document.new',dict(discardChanges=True));edit('file.open',dict(path=external))
            assert app.state()['mode']=='native' and len(app.state()['tables'])==1
            assert any(p['text']=='Äpfel' for p in app.state()['paragraphs'])
            # This span overflows the existing row grid; preserve malformed tables unchanged.
            parts['word/document.xml']=parts['word/document.xml'].replace(b'<w:tcPr>',b'<w:tcPr><w:gridSpan w:val="2"/>',1)
            unsupported=write_parts('merged-'+target+'.docx',parts)
            edit('document.new',dict(discardChanges=True));edit('file.open',dict(path=unsupported))
            assert app.state()['mode']=='preserved'
            assert app.resource(app.invoke('document.export')['resources'][0]['uri'])==(ROOT/unsupported).read_bytes()
            edit('document.new',dict(discardChanges=True));edit('file.open',dict(path=path))
        change('delete');assert app.state()['tables']==[]
        edit('document.undo');assert table()==expected[0]
        print(f'PASS {target} {"MCP" if mcp else "JSON"}: rich cells, structure, geometry, atomic errors, history and exact DOCX ({len(payload)} bytes)')
    finally:app.close()

def transfer(target,output):
    for path in (ROOT/output/'tables-browser').glob('*.docx'):
        app=Session(target,output)
        try:
            payload=path.read_bytes()
            app.invoke('file.open',dict(path=path.relative_to(ROOT).as_posix()),app.state()['revision'])
            assert app.state()['tables']
            assert app.resource(app.invoke('document.export')['resources'][0]['uri'])==payload
            print(f'PASS {target} <- {path.name}: exact DOCX bytes')
        finally:app.close()

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--target',choices=['win','linux','both'],default='both');p.add_argument('--out',default='build/wordcraft-tables');p.add_argument('--web-transfer',action='store_true');a=p.parse_args()
    for target in ['win','linux'] if a.target=='both' else [a.target]:
        if a.web_transfer:transfer(target,a.out)
        else:
            for mcp in [False,True]:scenario(target,a.out,mcp)
