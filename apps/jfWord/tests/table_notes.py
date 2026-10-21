"""Live cell notes over JSON/MCP, independent package validation and browser transfers."""
import argparse
import io
import zipfile
import xml.etree.ElementTree as ET
from agent import ROOT, Session

W='{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'

def validate_docx(data):
    with zipfile.ZipFile(io.BytesIO(data)) as z:
        root=ET.fromstring(z.read('word/document.xml'))
        for kind in ['footnote','endnote']:
            refs=[r.get(W+'id') for r in root.iter(W+kind+'Reference')]
            if not refs:continue
            part=ET.fromstring(z.read('word/'+kind+'s.xml'))
            bodies=[n.get(W+'id') for n in part.findall(W+kind) if n.get(W+'type') is None]
            assert sorted(refs)==sorted(bodies) and len(set(refs))==len(refs),(kind,refs,bodies)
        assert root.findall('.//'+W+'tc//'+W+'footnoteReference'), 'missing cell anchor'

def scenario(target,out,mcp):
    app=Session(target,out,mcp);folder=ROOT/out/'table-note-tests';folder.mkdir(parents=True,exist_ok=True)
    try:
        def state():return app.invoke('document.inspect',{'limit':100})['value']
        def edit(action,data=None):return app.invoke(action,data,state()['revision'])
        def export():return app.resource(app.invoke('document.export')['resources'][0]['uri'])
        def inspect(t):return app.invoke('document.table.inspect',{'table':t,'limit':32})['value']
        def layout(t):return app.invoke('document.table.layout',{'table':t,'limit':8})['value']
        def insert(p,rows=1,columns=1):return edit('document.table.insert',dict(paragraph=p,offset=0,rows=rows,columns=columns))['value']['id']
        def note(p,text,kind='footnote'):return edit('notes.insert',dict(paragraph=p,inline=0,offset=0,kind=kind,text=text))['value']['id']
        def change(t,op,**kw):return edit('document.table.change',dict(table=t,operation=op,**kw))
        edit('document.page.set',dict(width=7200,height=6000,top=600,bottom=600,left=600,right=600))
        table=insert(1,18,2);rows=inspect(table)['rows'];p=rows[0]['cells'][0]['paragraphs'][0];q=rows[9]['cells'][0]['paragraphs'][0]
        late=note(q,'Späte Zellnote');early=note(p,'Kopfnotiz α');inner=insert(p);ip=inspect(inner)['rows'][0]['cells'][0]['paragraphs'][0]
        nested=note(ip,'Verschachtelte Notiz');last=rows[-1]['cells'][0]['paragraphs'][0];end=note(last,'Zusätzlicher Beleg','endnote')
        main=note(1,'Anmerkung zum Haupttext');notes={n['id']:n for n in state()['notes']}
        assert [notes[n]['number'] for n in [nested,early,late,main]]==[1,2,3,4]
        assert notes[nested]['anchor']['paragraph']==ip and notes[nested]['anchor']['cell'] is not None
        body=notes[nested]['paragraphs'][0]
        src=edit('sources.put',dict(title='Cell-note source',year='2026'))['value']['id']
        edit('citations.insert',dict(paragraph=body,inline=0,offset=0,items=[dict(source=src,locator='8')]))
        original=export();validate_docx(original)
        rev=state()['revision'];app.invoke('notes.insert',dict(paragraph=body,inline=0,offset=0,kind='footnote'),rev,error=True)
        app.invoke('notes.kind',dict(id=early,kind='endnote'),rev-1,error=True)
        assert export()==original and state()['revision']==rev
        edit('notes.kind',dict(id=early,kind='endnote'));assert next(n for n in state()['notes'] if n['id']==early)['number']==1
        edit('document.undo');assert export()==original;edit('document.redo');edit('document.undo');assert export()==original
        # Removing a row removes its nested table and both cell note bodies, and undo restores all IDs.
        change(table,'removeRow',index=0);assert state()['totalNotes']==3 and state()['totalTables']==1
        edit('document.undo');assert export()==original
        # An external structural edit may remove the note story currently being edited.
        edit('document.select',dict(paragraph=body,start=0,end=0))
        change(table,'delete');assert state()['totalNotes']==1
        edit('ui.command',dict(id=23));assert state()['totalNotes']==2
        edit('document.undo');assert state()['totalNotes']==1
        edit('document.undo');assert export()==original
        app.invoke('clipboard.table.copy',{'table':table});edit('document.select',dict(paragraph=1,start=0,end=0));edit('clipboard.paste')
        assert state()['totalNotes']==9 and state()['totalSources']==1 and state()['totalTables']==4
        validate_docx(export());edit('document.undo');assert export()==original
        # A rich selection containing a note stays a live note inside a different cell.
        s=next(x for x in state()['paragraphs'] if x['id']==q)['text'];edit('document.select',dict(paragraph=q,start=0,end=len(s.encode())))
        app.invoke('clipboard.copy');dest=rows[1]['cells'][0]['paragraphs'][0];edit('document.select',dict(paragraph=dest,start=0,end=0));edit('clipboard.paste')
        assert state()['totalNotes']==6;edit('document.undo');assert export()==original
        change(table,'headerRows',headerRows=1)
        for floating in [False,True]:
            change(table,'position',floating=floating,**({'distance':[0,0,0,0]} if floating else {}));g=layout(table)
            assert g['pageCount']>=2,g['pageCount']
            for n in state()['notes']:
                if n['kind']!='footnote':continue
                hits=[(pg,line) for pg in g['pages'] for line in pg['bodyLines'] if line['paragraph'] in n['paragraphs']]
                assert hits,n
                for pg,line in hits:assert all(c['bounds'][1]+c['bounds'][3]<line['bounds'][1] for c in pg['cells'])
            assert sum(c['rowEnd'] and not c['repeated'] for pg in g['pages'] for c in pg['cells'])==36
        file=folder/f'{target}-{"mcp" if mcp else "json"}.docx';file.write_bytes(export());validate_docx(file.read_bytes())
        edit('file.open',dict(path=file.relative_to(ROOT).as_posix(),discardChanges=True));assert export()==file.read_bytes() and state()['mode']=='native'
        print(f'PASS {target} {"MCP" if mcp else "JSON"}: cell/nested notes, numbering, sources, history, clipboard, page reservation and independent DOCX validation')
    finally:app.close()

def transfer(target,out):
    files=sorted((ROOT/out/'table-notes-browser').glob('*.docx'));assert len(files)==6,files
    app=Session(target,out)
    try:
        for f in files:
            validate_docx(f.read_bytes());app.invoke('file.open',dict(path=f.relative_to(ROOT).as_posix(),discardChanges=True),app.state()['revision'])
            assert app.state()['mode']=='native' and app.state()['totalNotes']>=2
            assert app.resource(app.invoke('document.export')['resources'][0]['uri'])==f.read_bytes()
            print(f'PASS {target} table notes DOCX {f.name}')
    finally:app.close()

if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--target',choices=['win','linux','both'],default='both');p.add_argument('--out',default='build/wordcraft-table-notes');p.add_argument('--web-transfer',action='store_true');a=p.parse_args()
    for target in (['win','linux'] if a.target=='both' else [a.target]):
        if a.web_transfer:transfer(target,a.out)
        else:
            for mcp in [False,True]:scenario(target,a.out,mcp)
