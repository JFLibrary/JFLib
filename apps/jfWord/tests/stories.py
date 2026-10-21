"""Independent part-byte checks for preserved header/footer/note edits."""
import xml.etree.ElementTree as ET
import compatibility as compat


def run(invoke, rel, out, zip_parts, root):
    (root / 'build/jfword-stories-reference.docx').write_bytes(
        (root / 'build/jfword-stories.docx').read_bytes())
    parts = compat.independent_parts()
    parts['word/footer1.xml'] = (f'<w:ftr xmlns:w="{compat.W}"><w:p><w:r>'
        '<w:rPr><w:i/></w:rPr><w:t>Fußzeile</w:t></w:r></w:p></w:ftr>').encode()
    parts['word/endnotes.xml'] = (f'<w:endnotes xmlns:w="{compat.W}">'
        '<w:endnote w:id="9"><w:p><w:r><w:endnoteRef/></w:r>'
        '<w:r><w:t>Endnote Ω</w:t></w:r></w:p></w:endnote></w:endnotes>').encode()
    parts['[Content_Types].xml'] = parts['[Content_Types].xml'].replace(b'</Types>',
        b'<Override PartName="/word/endnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml"/></Types>')
    parts['word/_rels/document.xml.rels'] = parts['word/_rels/document.xml.rels'].replace(b'</Relationships>',
        f'<Relationship Id="endnotes" Type="{compat.R}/endnotes" Target="endnotes.xml"/></Relationships>'.encode())
    cases = [('word/header1.xml', 5, 'Kopfzeile'), ('word/footer1.xml', 6, 'Fußzeile'),
        ('word/footnotes.xml', 7, 'Unveränderte wissenschaftliche Anmerkung.'),
        ('word/endnotes.xml', 8, 'Endnote Ω')]
    for variant in ['utf8', 'utf16le', 'utf16be', 'strict', 'relocated']:
        data = dict(parts)
        encoding, bom = {'utf16le': ('utf-16-le', b'\xff\xfe'),
            'utf16be': ('utf-16-be', b'\xfe\xff')}.get(variant, ('utf-8', b''))
        if variant == 'strict':
            data = {n: b.replace(compat.W.encode(), b'http://purl.oclc.org/ooxml/wordprocessingml/main')
                .replace(compat.R.encode(), b'http://purl.oclc.org/ooxml/officeDocument/relationships')
                for n, b in data.items()}
        if variant == 'relocated':
            data = {(n.replace('word/', 'content/', 1) if n.startswith('word/') else n):
                (b.replace(b'word/', b'content/') if n in ['_rels/.rels', '[Content_Types].xml'] else b)
                for n, b in data.items()}
        for name, _, _ in cases:
            name = name.replace('word/', 'content/') if variant == 'relocated' else name
            data[name] = bom + data[name].decode('utf-8').encode(encoding)
        for name, index, before in cases:
            if variant == 'relocated': name = name.replace('word/', 'content/')
            filename = out / f'story-{variant}-{index}.docx'
            filename.write_bytes(zip_parts(data))
            expected = out / 'story-before.txt'
            expected.write_bytes(before.encode())
            replacement = out / 'story-after.txt'
            replacement.write_bytes(' Geändert <&> 中文 😀 '.encode())
            saved = out / f'story-{variant}-{index}-saved.docx'
            invoke('--check-preserved', rel(filename), '--output', rel(saved), '--edit-index', str(index),
                '--expected-file', rel(expected), '--replacement-file', rel(replacement))
            changed = compat.read(saved)
            wanted = dict(data)
            wanted[name] = bom + data[name][len(bom):].decode(encoding).replace(
                f'<w:t>{before}</w:t>', '<w:t xml:space="preserve"> Geändert &lt;&amp;&gt; 中文 😀 </w:t>').encode(encoding)
            assert changed == wanted, (variant, name)
            ET.fromstring(changed[name])
            again = out / 'story-copy.docx'
            invoke('--check-preserved', rel(saved), '--output', rel(again))
            assert again.read_bytes() == saved.read_bytes()


def inspect_ui(root):
    before = compat.read(root / 'build/jfword-stories-reference.docx')
    after = compat.read(root / 'build/jfword-stories.docx')
    wanted = dict(before)
    for path, old, new in [('stories/top.xml', 'Kopf α', 'Neue Kopfzeile Ω'),
                          ('stories/end.data', 'Endnote 😀', 'Endnote bearbeitet α')]:
        wanted[path] = wanted[path].replace(f'<w:t>{old}</w:t>'.encode(),
            f'<w:t xml:space="preserve">{new}</w:t>'.encode())
    assert after == wanted, 'UI edits changed unexpected story payloads'


def inspect_paragraphs(root):
    parts = compat.read(root / 'build/jfword-paragraphs.docx')
    document = ET.fromstring(parts['word/document.xml'])
    w = '{' + compat.W + '}'
    paragraphs = document.findall('.//' + w + 'body/' + w + 'p')
    assert len(paragraphs) == 2
    assert [''.join(p.itertext()) for p in paragraphs] == ['Aα!ZΩ', 'Finaler Absatz.']
    assert len(paragraphs[0].findall('.//' + w + 'br')) == 1
