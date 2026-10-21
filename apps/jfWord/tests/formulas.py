"""Live calculation fields via real JSON/MCP, independent WordprocessingML and transfer checks."""
import argparse
import io
import zipfile
import xml.etree.ElementTree as ET
from agent import ROOT, Session

W='{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def validate_docx(data):
    with zipfile.ZipFile(io.BytesIO(data)) as z:
        root=ET.fromstring(z.read('word/document.xml'))
        fields=[f for f in root.iter(W+'fldSimple') if f.get(W+'instr','').strip().startswith('=')]
        assert fields and root.findall('.//'+W+'tc//'+W+'fldSimple'), 'missing live cell field'
        for field in fields:
            instruction=field.get(W+'instr').strip()
            assert instruction.startswith('=') and not field.get(W+'fldLock'),instruction
            cache=''.join(field.itertext())
            assert cache, 'missing cached visible result'
        return fields


def changed_package(data, transform):
    out=io.BytesIO()
    with zipfile.ZipFile(io.BytesIO(data)) as z,zipfile.ZipFile(out,'w',zipfile.ZIP_STORED) as target:
        for entry in z.infolist():
            payload=z.read(entry.filename)
            if entry.filename=='word/document.xml':
                root=ET.fromstring(payload);transform(root)
                payload=ET.tostring(root,encoding='utf-8',xml_declaration=True)
            target.writestr(entry.filename,payload)
    return out.getvalue()


def complex_first(root):
    for parent in root.iter():
        for index,field in enumerate(parent):
            if field.tag!=W+'fldSimple':continue
            instruction=field.get(W+'instr');nodes=[]
            for tag,value in [('fldChar','begin'),('instrText',instruction),('fldChar','separate')]:
                run=ET.Element(W+'r');child=ET.SubElement(run,W+tag)
                if tag=='fldChar':child.set(W+'fldCharType',value)
                else:child.text=value
                nodes.append(run)
            nodes.extend(list(field));run=ET.Element(W+'r');ET.SubElement(run,W+'fldChar').set(W+'fldCharType','end');nodes.append(run)
            parent.remove(field)
            for i,node in enumerate(nodes):parent.insert(index+i,node)
            return
    raise AssertionError('no field to convert')


def scenario(target,out,mcp):
    app=Session(target,out,mcp);folder=ROOT/out/'formula-tests';folder.mkdir(parents=True,exist_ok=True)
    try:
        def state():return app.state()
        def edit(action,data=None):return app.invoke(action,data,state()['revision'])
        def export(format='docx'):return app.resource(app.invoke('document.export',{'format':format})['resources'][0]['uri'])
        def fields():return app.invoke('formulas.list',{'limit':100})['value']['fields']
        def result(p):return next(f['result'] for f in fields() if f['paragraph']==p)
        def replace(p,text):return edit('document.text.replace',dict(paragraph=p,start=0,end=len(next(x['text'] for x in state()['paragraphs'] if x['id']==p).encode()),text=text))
        def insert(p,s,picture=''):return edit('formulas.insert',dict(paragraph=p,inline=0,offset=0,expression=s,numberFormat=picture))
        table=edit('document.table.insert',dict(paragraph=1,offset=0,rows=4,columns=3))['value']['id']
        rows=app.invoke('document.table.inspect',{'table':table,'limit':32})['value']['rows']
        cell=lambda r,c:rows[r]['cells'][c]['paragraphs'][0]
        a2,b2,c2=cell(1,0),cell(1,1),cell(1,2);a3,b3,c3=cell(2,0),cell(2,1),cell(2,2);a4,c4=cell(3,0),cell(3,2)
        for p,text in [(cell(0,0),'999'),(a2,'2'),(b2,'12,5'),(a3,'3'),(b3,'7.5')]:replace(p,text)
        edit('document.table.change',dict(table=table,operation='headerRows',headerRows=1))
        insert(c2,'=A2*B2','0.00');insert(c3,'=R3C1*R3C2','0.00');insert(a4,'=SUM(ABOVE)','0');insert(c4,'=SUM(ABOVE)','0.00')
        assert [result(p) for p in [c2,c3,a4,c4]]==['25.00','22.50','5','47.50']
        before=export();revision=state()['revision'];validate_docx(before)
        # Preview is read-only and uses the same table context, errors and decimal arithmetic.
        for s,want,code in [('=SUM(A2:B3)','25',None),('=IF(1,7,1/0)','7',None),('=1/0','#DIV/0!','#DIV/0!'),('=Z999','#REF!','#REF!')]:
            value=app.invoke('formulas.evaluate',dict(paragraph=c4,expression=s))['value'];assert value==dict(result=want,error=code),value
        assert state()['revision']==revision and export()==before
        for s in ['=SUM(','=UNKNOWN(1)','=1e3']:
            app.invoke('formulas.update',dict(paragraph=c2,inline=1,expression=s),revision,error=True)
        app.invoke('formulas.update',dict(paragraph=c2,inline=1,expression='=2'),revision-1,error=True)
        app.invoke('formulas.insert',dict(paragraph=c2,inline='bad',expression='=2'),revision,error=True)
        assert state()['revision']==revision and export()==before
        replace(a2,'3');assert result(c2)=='37.50' and result(c4)=='60.00';edit('document.undo');assert export()==before
        edit('document.redo');edit('document.undo');assert export()==before
        edit('formulas.update',dict(paragraph=c2,inline=1,expression='=C3'))
        edit('formulas.update',dict(paragraph=c3,inline=1,expression='=C2'))
        assert result(c2)==result(c3)=='#CYCLE!';assert next(f for f in fields() if f['paragraph']==c2)['error']=='#CYCLE!'
        edit('document.undo');edit('document.undo');assert export()==before
        # Selection cannot replace only part of a generated number.
        app.invoke('document.text.replace',dict(paragraph=c2,start=0,end=1,text='x'),state()['revision'],error=True);assert export()==before
        app.invoke('clipboard.table.copy',{'table':table});edit('document.select',dict(paragraph=1,start=0,end=0));edit('clipboard.paste')
        assert len(fields())==8 and sorted(f['result'] for f in fields())==sorted(['25.00','22.50','5','47.50']*2)
        validate_docx(export());edit('document.undo');assert export()==before
        edit('document.select',dict(paragraph=c2,start=0,end=5));app.invoke('clipboard.copy')
        destination=cell(0,2);edit('document.select',dict(paragraph=destination,start=0,end=0));edit('clipboard.paste')
        assert len(fields())==5;edit('document.undo');assert export()==before
        edit('document.table.change',dict(table=table,operation='removeRow',index=2));assert result(c4)=='25.00';edit('document.undo');assert export()==before
        # Nested calculation fields resolve in their own logical table.
        inner=edit('document.table.insert',dict(paragraph=a2,offset=0,rows=2,columns=1))['value']['id']
        ir=app.invoke('document.table.inspect',{'table':inner,'limit':32})['value']['rows'];ip=ir[0]['cells'][0]['paragraphs'][0];iq=ir[1]['cells'][0]['paragraphs'][0]
        replace(ip,'11');insert(iq,'=SUM(ABOVE)');assert result(iq)=='11' and result(c4)=='47.50'
        # The actual dialog blocks semantic mutation and previews the selected field.
        edit('document.select',dict(paragraph=c2,start=0,end=5));edit('ui.command',dict(id=130))
        app.invoke('formulas.update',dict(paragraph=c2,inline=1,expression='=100'),state()['revision'],error=True)
        edit('ui.key',dict(key=27))
        # Current results are materialized for the existing bounded external profiles.
        assert b'47.50' in export('html') and b'47.50' in export('rtf')
        file=folder/f'{target}-{"mcp" if mcp else "json"}.docx';file.write_bytes(export());validate_docx(file.read_bytes())
        edit('file.open',dict(path=file.relative_to(ROOT).as_posix(),discardChanges=True));assert state()['mode']=='native' and export()==file.read_bytes()
        original=file.read_bytes()
        expected=fields()
        foreign=folder/f'{target}-complex.docx';foreign.write_bytes(changed_package(original,complex_first))
        edit('file.open',dict(path=foreign.relative_to(ROOT).as_posix(),discardChanges=True))
        assert state()['mode']=='native' and fields()==expected and export()==foreign.read_bytes()
        # Valid switches are interpreted, malformed/unknown switches stay preserved.
        switched=folder/f'{target}-switch.docx'
        switched.write_bytes(changed_package(original,lambda root:root.find('.//'+W+'fldSimple').set(W+'instr','=SUM(ABOVE) \\# "0.0000" \\* MERGEFORMAT')))
        edit('file.open',dict(path=switched.relative_to(ROOT).as_posix(),discardChanges=True));assert state()['mode']=='native'
        # Unsupported functions, number pictures and locked fields preserve the original package.
        for name,mutation in [
            ('unknown',lambda f:f.set(W+'instr','=ACOS(1)')),
            ('picture',lambda f:f.set(W+'instr','=1 \\# "€0.00"')),
            ('locked',lambda f:f.set(W+'fldLock','1')),
            ('duplicate-picture',lambda f:f.set(W+'instr','=1 \\# "" \\# "0"')),
            ('missing-switch',lambda f:f.set(W+'instr','=1 \\# "0" * MERGEFORMAT')),
        ]:
            foreign=folder/f'{target}-{name}.docx'
            foreign.write_bytes(changed_package(original,lambda root:mutation(root.find('.//'+W+'fldSimple'))))
            edit('file.open',dict(path=foreign.relative_to(ROOT).as_posix(),discardChanges=True));assert state()['mode']=='preserved'
            assert export()==foreign.read_bytes()
        print(f'PASS {target} {"MCP" if mcp else "JSON"}: live decimal/table formulas, errors/revisions, nested clipboard, modal UI and independent DOCX preservation')
    finally:app.close()


def transfer(target,out):
    files=sorted((ROOT/out/'formulas-browser').glob('*.docx'));assert len(files)==6,files
    app=Session(target,out)
    try:
        for file in files:
            validate_docx(file.read_bytes());app.invoke('file.open',dict(path=file.relative_to(ROOT).as_posix(),discardChanges=True),app.state()['revision'])
            assert app.state()['mode']=='native' and app.invoke('formulas.list')['value']['total']>=3
            assert app.resource(app.invoke('document.export')['resources'][0]['uri'])==file.read_bytes()
            print(f'PASS {target} formula DOCX {file.name}')
    finally:app.close()


if __name__=='__main__':
    p=argparse.ArgumentParser();p.add_argument('--target',choices=['win','linux','both'],default='both');p.add_argument('--out',default='build/wordcraft-table-formulas');p.add_argument('--web-transfer',action='store_true');a=p.parse_args()
    for target in (['win','linux'] if a.target=='both' else [a.target]):
        if a.web_transfer:transfer(target,a.out)
        else:
            for mcp in [False,True]:scenario(target,a.out,mcp)
