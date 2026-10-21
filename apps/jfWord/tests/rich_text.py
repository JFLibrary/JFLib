"""Independent XML checks for the continuous editor and multi-page imported run fixture."""
import zipfile
import xml.etree.ElementTree as ET

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
TEXT = 'Forschung αβ bleibt prüfbar. ' * 180

def prepare(parts, zip_parts, root):
    data = dict(parts)
    doc = data['word/document.xml'].decode()
    begin = doc.index('<w:body>') + len('<w:body>')
    end = doc.index('<w:sectPr>')
    content = ('<w:p><w:pPr><w:jc w:val="both"/></w:pPr>'
        '<w:r><w:rPr><w:color w:val="2459A6"/><w:b/></w:rPr><w:t>Start α </w:t></w:r>'
        '<w:r><w:t xml:space="preserve">' + TEXT + '</w:t></w:r>'
        '<w:r><w:rPr><w:strike/><w:i/></w:rPr><w:t>Schluss Ω</w:t></w:r></w:p>')
    data['word/document.xml'] = (doc[:begin]+content+doc[end:]).encode()
    (root/'build/jfword-rich-pages.docx').write_bytes(zip_parts(data))
    (root/'build/jfword-rich-citations.docx').write_bytes(zip_parts(parts))
    # All save scripts exercise the explicit overwrite path on clean builds as well.
    for name in ['jfword-rich-ui.docx','jfword-rich-pages-edited.docx','jfword-rich-story.docx',
                 'jfword-rich-clipboard.docx','jfword-rich-fields.docx']:
        (root/'build'/name).write_bytes(zip_parts(parts))

def inspect(root):
    with zipfile.ZipFile(root/'build/jfword-rich-ui.docx') as archive:
        doc = ET.fromstring(archive.read('word/document.xml'))
        notes = ET.fromstring(archive.read('word/footnotes.xml'))
    runs = doc.findall('.//'+W+'body/'+W+'p/'+W+'r')
    texts = [''.join(r.itertext()) for r in runs]
    assert ''.join(texts) == 'Alpha βeta Gamma Ende', texts
    beta = next(r for r in runs if ''.join(r.itertext()) == 'βeta')
    assert beta.find(W+'rPr/'+W+'b') is not None
    assert beta.find(W+'rPr/'+W+'u').get(W+'val') == 'single'
    assert beta.find(W+'rPr/'+W+'strike').get(W+'val') == '1'
    assert beta.find(W+'rPr/'+W+'color').get(W+'val') == '2459A6'
    plain = next(r for r in runs if ''.join(r.itertext()) == 'Alp')
    assert plain.find(W+'rPr') is None
    suffix = next(r for r in runs if ''.join(r.itertext()) == ' Ende')
    assert suffix.find(W+'rPr/'+W+'i').get(W+'val') == '1'
    assert len(doc.findall('.//'+W+'footnoteReference')) == 1
    assert ''.join(notes.itertext()).strip() == 'Fußnote α bleibt wissenschaftlich.'
    with zipfile.ZipFile(root/'build/jfword-rich-pages-edited.docx') as archive:
        doc = ET.fromstring(archive.read('word/document.xml'))
    assert ''.join(doc.find(W+'body/'+W+'p').itertext()) == 'Start α ' + TEXT + 'Schluss Ω ENDE'
    assert doc.find('.//'+W+'color').get(W+'val') == '2459A6'
    assert doc.find('.//'+W+'strike').get(W+'val') == '1'
    with zipfile.ZipFile(root/'build/jfword-rich-story.docx') as archive:
        doc = ET.fromstring(archive.read('word/document.xml'))
        footnotes = ET.fromstring(archive.read('word/footnotes.xml'))
        endnotes = ET.fromstring(archive.read('word/endnotes.xml'))
    paragraphs = doc.findall(W+'body/'+W+'p')
    assert [''.join(p.itertext()) for p in paragraphs] == ['Aα', 'Bβ', 'CΩ']
    for paragraph, expected in zip(paragraphs, [('A', 'α'), ('Bβ',), ('C', 'Ω')]):
        assert tuple(''.join(r.itertext()) for r in paragraph.findall(W+'r') if r.find(W+'t') is not None and ''.join(r.itertext())) == expected
    assert all(p.find(W+'pPr/'+W+'jc').get(W+'val') == 'center' for p in paragraphs[:2])
    assert all(p.find(W+'pPr/'+W+'ind').get(W+'firstLine') == '360' for p in paragraphs[:2])
    assert paragraphs[2].find(W+'pPr/'+W+'ind') is None
    runs = paragraphs[0].findall(W+'r')
    assert runs[0].find(W+'rPr') is None
    assert runs[1].find(W+'rPr/'+W+'b').get(W+'val') == '1'
    assert paragraphs[1].find(W+'r/'+W+'rPr/'+W+'b').get(W+'val') == '1'
    runs = [r for r in paragraphs[2].findall(W+'r') if r.find(W+'t') is not None]
    assert runs[0].find(W+'rPr/'+W+'b').get(W+'val') == '1'
    assert all(r.find(W+'rPr/'+W+'i').get(W+'val') == '1' for r in runs)
    assert runs[1].find(W+'rPr/'+W+'b') is None
    for root, tag, texts, prop in [(footnotes, 'footnote', ['Notiz α','Beleg β'], 'u'), (endnotes, 'endnote', ['Ende α','Quelle β'], 'b')]:
        bodies = [n for n in root.findall(W+tag) if n.get(W+'type') is None]
        assert len(bodies) == 1
        ps = bodies[0].findall(W+'p')
        assert [''.join(p.itertext()) for p in ps] == texts
        assert all(r.find(W+'rPr/'+W+prop) is not None for p in ps for r in p.findall(W+'r') if r.find(W+'t') is not None)


def probe_ui(invoke, rel, out):
    clipboard = out/'story-clipboard.txt'
    clipboard.write_bytes(b'A\rB\r\nC')
    oversized = out/'story-clipboard-limit.txt'
    oversized.write_bytes(b'x' * (4 << 20))
    model_limit = out/'story-model-limit.txt'
    model_limit.write_bytes(b'x' * ((2 << 20) + 1))
    path = out/'story-cr-paste.uis'
    path.write_text(f'clip utf8 {rel(clipboard)}\n'
        'key ctrl+v\nexpect #10000 text "A\\nB\\nC"\n'
        'key ctrl+z\nexpect textbox:"A\\nB\\nC" missing\n'
        'clip text "Safe"\nkey ctrl+v\n'
        'key ctrl+home right shift+right shift+right\n'
        f'clip utf8 {rel(oversized)}\nkey ctrl+v\n'
        'expect textbox:"Safe" value 3\nexpect textbox:"Safe" max 1\n'
        f'clip utf8 {rel(model_limit)}\nkey ctrl+v\n'
        'expect textbox:"Safe" value 3\nexpect textbox:"Safe" max 1\n'
        'key ctrl+z\nexpect textbox:"Safe" missing\nexpect_nowarn\n', encoding='utf-8')
    invoke('--editor', '--script', rel(path), '--verify-dirty')
