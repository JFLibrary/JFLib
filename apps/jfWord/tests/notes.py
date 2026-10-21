"""Independent OOXML note graph checks against the real native reader and writer."""
import re
import xml.etree.ElementTree as ET
import zipfile

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
W14 = '{http://schemas.microsoft.com/office/word/2010/wordml}'
JF = '{urn:jfword:notes:1}'


def read(path):
    with zipfile.ZipFile(path) as z:
        assert z.testzip() is None
        return {name: z.read(name) for name in z.namelist()}


def linked_notes(parts):
    """Resolve note identity in its OOXML scope, independent of app IDs and part order."""
    document = ET.fromstring(parts['word/document.xml'])
    notes = {}
    for kind in ('footnote', 'endnote'):
        part = 'word/' + kind + 's.xml'
        if part not in parts:
            continue
        for note in ET.fromstring(parts[part]):
            if note.get(W + 'type') in ('separator', 'continuationSeparator'):
                continue
            key = (kind, note.get(W + 'id'))
            assert key not in notes
            notes[key] = tuple(''.join(p.itertext()) for p in note)
            assert len(list(note.iter(W + kind + 'Ref'))) == 1
    result = []
    for node in document.iter():
        for kind in ('footnote', 'endnote'):
            if node.tag == W + kind + 'Reference':
                result.append((kind, notes.pop((kind, node.get(W + 'id')))))
    assert not notes, 'orphaned notes in saved package'
    return result


def run(invoke, rel, out, zip_parts, root):
    parts = read(root / 'build/jfword-notes.docx')
    assert len(parts) == 11
    settings = ET.fromstring(parts['word/settings.xml'])
    for kind in ('footnote', 'endnote'):
        properties = settings.find(W + kind + 'Pr')
        assert properties is not None
        assert [n.get(W + 'id') for n in properties] == ['-1', '0']
    original = linked_notes(parts)
    assert [kind for kind, _ in original] == ['footnote', 'endnote', 'footnote']
    assert 'vgl.' in original[0][1][0] and 'eigene Übersetzung' in original[0][1][0]
    assert 'Hintergrund Ω' in original[1][1][0] and '中文' in original[1][1][0]
    assert original[2][1][1] == 'Zweiter Absatz.'
    document = ET.fromstring(parts['word/document.xml'])
    fields = list(document.iter(W + 'fldSimple'))
    assert [f.get(W + 'instr').strip() for f in fields] == ['BIBLIOGRAPHY']
    assert 'Noten & Quellen' in ''.join(fields[0].itertext())
    assert 'Forschungsinstitut' in ''.join(fields[0].itertext())

    def probe(name, edited, valid=True, exact=False):
        path = out / ('note-' + name + '.docx')
        path.write_bytes(zip_parts(edited))
        output = out / ('note-' + name + '-saved.docx')
        output.write_bytes(b'KEEP EXISTING FILE')
        if not valid:
            invoke('--check-docx', rel(path), '--output', rel(output), '--expect-reject')
            assert output.read_bytes() == b'KEEP EXISTING FILE', name
            return
        invoke('--check-docx', rel(path), '--output', rel(output))
        decoded = read(output)
        assert linked_notes(decoded) == original, name
        if exact:
            assert decoded == parts, name
        again = out / ('note-' + name + '-twice.docx')
        invoke('--check-docx', rel(output), '--output', rel(again))
        assert again.read_bytes() == output.read_bytes(), name

    probe('native', parts, exact=True)
    renamed = {name: data.replace(b'xmlns:w=', b'xmlns:q=').replace(b'<w:', b'<q:')
               .replace(b'</w:', b'</q:').replace(b' w:', b' q:') for name, data in parts.items()}
    probe('alternate-prefix', renamed, exact=True)
    utf16 = {name: data.decode('utf-8').replace('encoding="UTF-8"', 'encoding="UTF-16"')
             .encode('utf-16') for name, data in parts.items()}
    probe('utf16', utf16, exact=True)
    # OOXML note scopes may both use id 1, independently of the main paraId and source IDs.
    foreign = dict(parts)
    for kind, old in [('footnote', b'30'), ('endnote', b'20')]:
        part = 'word/' + kind + 's.xml'
        foreign[part] = re.sub(rb' (?:jf:order|w14:paraId)="[^"]*"', b'', foreign[part])
        foreign[part] = foreign[part].replace(b'w:id="' + old + b'"', b'w:id="1"')
        foreign['word/document.xml'] = foreign['word/document.xml'].replace(
            kind.encode() + b'Reference w:id="' + old + b'"', kind.encode() + b'Reference w:id="1"')
    probe('scoped-ids-and-allocated-paragraphs', foreign)
    # Reorder XML note records without changing logical ordering or inline anchors.
    reversed_parts = dict(parts)
    data = reversed_parts['word/footnotes.xml']
    records = re.findall(rb'<w:footnote w:id="[^\"]+"[^>]*>.*?</w:footnote>', data)
    assert len(records) == 2
    reversed_parts['word/footnotes.xml'] = data.replace(records[0] + records[1], records[1] + records[0])
    probe('physical-note-order', reversed_parts, exact=True)
    # Cached citation text is stale by definition; loading must regenerate it from sources.
    stale = dict(parts)
    stale['word/footnotes.xml'] = stale['word/footnotes.xml'].replace(
        b'Forschungsinstitut, 2026', b'WRONG CACHE')
    probe('refresh-note-citation-cache', stale, exact=True)

    mutations = [
        ('missing-anchor', 'word/document.xml', b'<w:footnoteReference w:id="30"/>', b''),
        ('dangling-anchor', 'word/document.xml', b'footnoteReference w:id="30"', b'footnoteReference w:id="999"'),
        ('repeated-anchor', 'word/document.xml', b'footnoteReference w:id="10"', b'footnoteReference w:id="30"'),
        ('custom-mark', 'word/document.xml', b'footnoteReference w:id="30"', b'footnoteReference w:id="30" w:customMarkFollows="1"'),
        ('roman-numbering', 'word/document.xml', b'<w:numFmt w:val="decimal"/>', b'<w:numFmt w:val="lowerRoman"/>'),
        ('restart-each-page', 'word/document.xml', b'w:val="continuous"', b'w:val="eachPage"'),
        ('number-start', 'word/document.xml', b'<w:numStart w:val="1"/>', b'<w:numStart w:val="9"/>'),
        ('note-beneath-text', 'word/document.xml', b'w:val="pageBottom"', b'w:val="beneathText"'),
        ('end-of-section', 'word/document.xml', b'w:val="docEnd"', b'w:val="sectEnd"'),
        ('implicit-end-format', 'word/document.xml', b'<w:numFmt w:val="decimal"/>', b''),
        ('missing-body-marker', 'word/footnotes.xml', b'<w:footnoteRef/>', b''),
        ('nested-note', 'word/footnotes.xml', b'<w:footnoteRef/>', b'<w:footnoteRef/><w:footnoteReference w:id="10"/>'),
        ('unsupported-note-format', 'word/footnotes.xml', b'<w:rPr>', b'<w:rPr><w:color w:val="FF0000"/>'),
        ('note-table', 'word/footnotes.xml', b'</w:footnote>', b'<w:tbl/></w:footnote>'),
        ('custom-separator', 'word/footnotes.xml', b'<w:separator/>', b'<w:t>Custom separator text</w:t>'),
        ('unknown-note-element', 'word/endnotes.xml', b'</w:endnote>', b'<w:altChunk/></w:endnote>'),
        ('note-id-alias', 'word/footnotes.xml', b'w:id="30"', b'w:id="010"'),
        ('missing-note-source', 'word/footnotes.xml', b'CITATION jf2', b'CITATION jf999'),
        ('external-part', 'word/_rels/document.xml.rels', b'Target="footnotes.xml"', b'Target="footnotes.xml" TargetMode="External"'),
        ('wrong-part-type', '[Content_Types].xml', b'wordprocessingml.footnotes+xml', b'wordprocessingml.endnotes+xml'),
    ]
    for name, part, old, new in mutations:
        assert old in parts[part], name
        edited = dict(parts)
        edited[part] = edited[part].replace(old, new)
        probe(name, edited, valid=False)
    edited = dict(parts)
    del edited['word/endnotes.xml']
    probe('missing-note-part', edited, valid=False)
    edited = dict(parts)
    edited['word/_rels/endnotes.xml.rels'] = b'<Relationships/>'
    probe('unknown-note-relationships', edited, valid=False)


def inspect_ui(root):
    parts = read(root / 'build/jfword-notes-ui.docx')
    notes = linked_notes(parts)
    assert [kind for kind, _ in notes] == ['endnote', 'footnote'], notes
    assert notes[0][1][0] == 'Anmerkung α zur Quelle.(Forschungsinstitut, 2026)'
    assert notes[0][1][1] == 'Zweiter Notenabsatz.'
    assert notes[1][1] == ('Weitere Erklärung Ω.',)
    document = ET.fromstring(parts['word/document.xml'])
    assert len(list(document.iter(W + 'fldSimple'))) == 1
    assert 'Haupttext α ' in ''.join(document.itertext())
    endnotes = ET.fromstring(parts['word/endnotes.xml'])
    assert len(list(endnotes.iter(W + 'i'))) == 2  # original segment + empty trailing segment
    assert len(list(endnotes.iter(W + 'fldSimple'))) == 1
