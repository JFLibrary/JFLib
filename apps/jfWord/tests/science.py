"""Independent Word bibliography fixtures: XML is built with the stdlib, not jfWord."""
import copy
import xml.etree.ElementTree as ET
import zipfile

B = '{http://schemas.openxmlformats.org/officeDocument/2006/bibliography}'
W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'

def element(parent, name, text=None):
    n = ET.SubElement(parent, B+name)
    n.text = text
    return n

def fixture():
    root = ET.Element(B+'Sources', SelectedStyle='APA.XSL', StyleName='APA', URI='https://example.invalid/style')
    for kind, tag, title in [('Book','ExternalBook','Bücher & Forschung'),
                             ('JournalArticle','Article-2026','Reproduzierbare Statistik'),
                             ('InternetSite','InstituteWeb','Offene Daten')]:
        s = element(root,'Source')
        element(s,'Tag',tag); element(s,'SourceType',kind); element(s,'Title',title)
        element(s,'Year','2026'); element(s,'RefOrder','9')
        authors = element(element(s,'Author'),'Author')
        if kind == 'InternetSite':
            element(authors,'Corporate','Institut für Forschung')
            for k,v in [('InternetSiteTitle','Datenportal'),('URL','https://example.org/?a=1&b=2'),
                        ('YearAccessed','2026'),('MonthAccessed','10'),('DayAccessed','9')]: element(s,k,v)
        else:
            names = element(authors,'NameList')
            for given,family in [('Ada','Lovelace'),('Émilie','du Châtelet')]:
                p = element(names,'Person')
                element(p,'First',given); element(p,'Last',family); element(p,'Middle','M.')
        fields = [('Publisher','Verlag'),('City','Berlin'),('Edition','3'),('StandardNumber','ISBN 123')]
        if kind == 'JournalArticle': fields = [
            ('JournalName','Methods Journal'),('Volume','5'),('Issue','2'),('Pages','17–29'),
            ('URL','https://doi.org/10.0000/test'),('Guid','{01234567-1234-1234-1234-0123456789AB}'),
            ('LCID','1031'),('Comments','Notiz\nα β'),('ShortTitle','Statistik')]
        if kind != 'InternetSite':
            for k,v in fields: element(s,k,v)
    return root

def records(root):
    result=[]
    for source in root:
        d={n.tag:n.text or '' for n in source if n.tag not in [B+'Tag',B+'RefOrder',B+'Author']}
        author=source.find(B+'Author/'+B+'Author')
        if author is not None:
            corp=author.find(B+'Corporate')
            d['authors'] = corp.text if corp is not None else [
                {p.tag:p.text or '' for p in person} for person in author.findall(B+'NameList/'+B+'Person')]
        result.append(d)
    return result

def run(invoke, rel, out):
    root=fixture()
    def data(r): return ET.tostring(r, encoding='utf-8', xml_declaration=True)
    def probe(name, raw, reject=False):
        path=out/(name+'.xml'); path.write_bytes(raw)
        output=out/(name+'-export.xml')
        docx=out/(name+'.docx')
        if reject:
            output.write_bytes(b'KEEP EXISTING FILE')
            invoke('--check-sources',rel(path),'--expect-reject','--output',rel(output))
            assert output.read_bytes()==b'KEEP EXISTING FILE'
        else:
            invoke('--check-sources',rel(path),'--output',rel(output),'--docx-output',rel(docx))
            decoded=ET.parse(output).getroot()
            assert records(decoded)==records(root), name
            assert [s.find(B+'Tag').text for s in decoded]==['jf2','jf3','jf4']
            with zipfile.ZipFile(docx) as z:
                assert records(ET.fromstring(z.read('customXml/item1.xml')))==records(root)
                document=ET.fromstring(z.read('word/document.xml'))
                codes=[n.attrib[W+'instr'].strip() for n in document.iter(W+'fldSimple')]
                assert codes==['CITATION jf2','CITATION jf3','CITATION jf4','BIBLIOGRAPHY']
                assert 'Lovelace & du Châtelet' in ''.join(document.itertext())
            again=out/(name+'-roundtrip.docx')
            invoke('--check-docx',rel(docx),'--output',rel(again))
            assert again.read_bytes()==docx.read_bytes()
    probe('word-sources',data(root))
    probe('word-sources-utf16',ET.tostring(root,encoding='utf-16',xml_declaration=True))
    raw=data(root).replace(b'xmlns:ns0=',b'xmlns=').replace(b'ns0:',b'')
    probe('word-sources-default-ns',raw)
    variants={}
    def bad(name, mutate):
        r=copy.deepcopy(root); mutate(r); variants[name]=data(r)
    bad('duplicate-tag',lambda r: setattr(r[1].find(B+'Tag'),'text','ExternalBook'))
    bad('missing-tag',lambda r: r[0].remove(r[0].find(B+'Tag')))
    bad('unknown-type',lambda r: setattr(r[0].find(B+'SourceType'),'text','BookSection'))
    bad('empty-title',lambda r: setattr(r[1].find(B+'Title'),'text',''))
    bad('unknown-field',lambda r: element(r[2],'UnknownField','do not drop'))
    bad('duplicate-field',lambda r: element(r[2],'Title','duplicate'))
    bad('field-namespace',lambda r: ET.SubElement(r[2],'{urn:wrong}Title'))
    bad('hidden-field-child',lambda r: element(r[0].find(B+'Title'),'Title','nested'))
    bad('unknown-role',lambda r: element(r[0].find(B+'Author'),'Editor'))
    bad('mixed-author-mode',lambda r: element(r[2].find(B+'Author/'+B+'Author'),'NameList'))
    bad('unknown-person-component',lambda r: element(r[0].find('.//'+B+'Person'),'Suffix','III'))
    bad('duplicate-person-component',lambda r: element(r[0].find('.//'+B+'Person'),'Last','Duplicate'))
    bad('unknown-root-attribute',lambda r: r.set('arbitrary','x'))
    bad('oversized-field',lambda r: setattr(r[0].find(B+'Title'),'text','x'*4097))
    bad('invalid-reforder',lambda r: setattr(r[0].find(B+'RefOrder'),'text','-1'))
    def many_people(r):
        names=r[0].find('.//'+B+'NameList')
        for _ in range(64): names.append(copy.deepcopy(names[0]))
    bad('too-many-authors',many_people)
    variants['dtd-sources']=b'<!DOCTYPE Sources SYSTEM "file:///private">'+data(root)
    variants['invalid-utf8']=b'\xff'+data(root)
    variants['oversized-source-file']=b' '*(4*1024*1024+1)
    for name,raw in variants.items(): probe(name,raw,True)
    # Old jfWord anonymous-book sources remain readable after the model upgrade.
    old=b'''<b:Sources xmlns:b="http://schemas.openxmlformats.org/officeDocument/2006/bibliography"><b:Source><b:Tag>jf2</b:Tag><b:SourceType>Book</b:SourceType><b:Author><b:Author><b:NameList><b:Person><b:Last/><b:First/></b:Person></b:NameList></b:Author></b:Author><b:Title>Anonymous</b:Title><b:Year/><b:Publisher/></b:Source></b:Sources>'''
    path=out/'old-anonymous.xml'; path.write_bytes(old)
    invoke('--check-sources',rel(path))
