"""Independent foreign DOCX preservation and surgical-text-edit contract checks.

Fixtures are built with Python ZIP/XML primitives, not the app's native serializer.
No external packages, Word installation, network or private documents are required.
"""
import base64
import io
import zipfile
import xml.etree.ElementTree as ET

W = 'http://schemas.openxmlformats.org/wordprocessingml/2006/main'
R = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'
REL = 'http://schemas.openxmlformats.org/package/2006/relationships'
CT = 'http://schemas.openxmlformats.org/package/2006/content-types'
MAIN = 'application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml'
OLD = 'Fremdes Dokument α & Ω'
NEW = ' Bearbeitet <&> 中文 😀 '
OLD_LEAF = '<w:t>Fremdes Dokument α &amp; Ω</w:t>'
NEW_LEAF = '<w:t xml:space="preserve"> Bearbeitet &lt;&amp;&gt; 中文 😀 </w:t>'


def read(path):
    with zipfile.ZipFile(path) as archive:
        assert archive.testzip() is None
        return {name: archive.read(name) for name in archive.namelist()}


def independent_parts():
    """Conventional OOXML structures outside jfWord's editable model and fixed parts."""
    types = {
        'word/document.xml': MAIN,
        'word/styles.xml': 'application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml',
        'word/settings.xml': 'application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml',
        'word/numbering.xml': 'application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml',
        'word/header1.xml': 'application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml',
        'word/footer1.xml': 'application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml',
        'word/footnotes.xml': 'application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml',
        'docProps/core.xml': 'application/vnd.openxmlformats-package.core-properties+xml',
    }
    content_types = (
        f'<Types xmlns="{CT}">'
        '<Default Extension="xml" ContentType="application/xml"/>'
        '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
        '<Default Extension="png" ContentType="image/png"/>'
        '<Default Extension="bin" ContentType="application/octet-stream"/>'
        + ''.join(f'<Override PartName="/{name}" ContentType="{value}"/>' for name, value in types.items())
        + '</Types>'
    )
    root_rels = (
        f'<Relationships xmlns="{REL}">'
        f'<Relationship Id="doc" Type="{R}/officeDocument" Target="word/document.xml"/>'
        '<Relationship Id="props" Type="http://schemas.openxmlformats.org/package/2006/relationships/'
        'metadata/core-properties" Target="docProps/core.xml"/>'
        '</Relationships>'
    )
    relationships = [
        ('styles', 'styles', 'styles.xml'), ('settings', 'settings', 'settings.xml'),
        ('list', 'numbering', 'numbering.xml'), ('header', 'header', 'header1.xml'),
        ('footer', 'footer', 'footer1.xml'), ('footnote', 'footnotes', 'footnotes.xml'),
        ('image', 'image', 'media/pixel.png'), ('embedded', 'oleObject', 'embeddings/opaque.bin'),
        ('vendor', 'customXml', '../customXml/vendor.xml'),
    ]
    document_rels = f'<Relationships xmlns="{REL}">' + ''.join(
        f'<Relationship Id="{id_}" Type="{R}/{kind}" Target="{target}"/>'
        for id_, kind, target in relationships
    ) + f'<Relationship Id="external" Type="{R}/hyperlink" Target="https://example.invalid/never-fetch" TargetMode="External"/></Relationships>'
    document = (
        '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'
        f'<w:document xmlns:w="{W}" xmlns:r="{R}" '
        'xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" '
        'xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" '
        'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" '
        'xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture">'
        '<w:body><w:p w:rsidR="00123456"><w:pPr><w:pStyle w:val="CustomTitle"/>'
        '<w:spacing w:before="240" w:after="160"/></w:pPr>'
        '<w:r><w:rPr><w:rFonts w:ascii="Aptos" w:hAnsi="Aptos"/><w:b/><w:sz w:val="32"/>'
        '<w:color w:val="345678"/></w:rPr>' + OLD_LEAF + '</w:r>'
        '<w:r><w:t xml:space="preserve"> — Format bleibt erhalten.</w:t></w:r></w:p>'
        '<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="5000" w:type="pct"/></w:tblPr>'
        '<w:tblGrid><w:gridCol w:w="4500"/><w:gridCol w:w="4500"/></w:tblGrid>'
        '<w:tr><w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>Messung A</w:t>'
        '</w:r></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="4500" w:type="dxa"/></w:tcPr>'
        '<w:p><w:r><w:t>12,34</w:t></w:r></w:p></w:tc></w:tr></w:tbl>'
        '<w:p><w:pPr><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr>'
        '<w:r><w:t>Nummerierter Absatz</w:t></w:r><w:r><w:footnoteReference w:id="1"/></w:r></w:p>'
        '<w:p><w:hyperlink r:id="external"><w:r><w:t>Link</w:t></w:r></w:hyperlink>'
        '<w:fldSimple w:instr=" DATE "><w:r><w:t>11.10.2026</w:t></w:r></w:fldSimple>'
        '<m:oMath><m:r><m:t>x + y</m:t></m:r></m:oMath></w:p>'
        '<w:p><w:r><w:drawing><wp:inline><wp:extent cx="9525" cy="9525"/>'
        '<wp:docPr id="1" name="pixel"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture">'
        '<pic:pic><pic:nvPicPr><pic:cNvPr id="0" name="pixel.png"/><pic:cNvPicPr/></pic:nvPicPr>'
        '<pic:blipFill><a:blip r:embed="image"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>'
        '<pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="9525" cy="9525"/></a:xfrm>'
        '<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic>'
        '</a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>'
        '<w:sectPr><w:headerReference w:type="default" r:id="header"/>'
        '<w:footerReference w:type="default" r:id="footer"/>'
        '<w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" '
        'w:bottom="1440" w:left="1440" w:header="720" w:footer="720" w:gutter="0"/>'
        '</w:sectPr></w:body></w:document>'
    )
    parts = {
        '[Content_Types].xml': content_types,
        '_rels/.rels': root_rels,
        'word/_rels/document.xml.rels': document_rels,
        'word/document.xml': document,
        'word/styles.xml': f'<w:styles xmlns:w="{W}"><w:style w:type="paragraph" w:styleId="CustomTitle"><w:name w:val="Custom title"/><w:rPr><w:sz w:val="32"/></w:rPr></w:style></w:styles>',
        'word/settings.xml': f'<w:settings xmlns:w="{W}"><w:zoom w:percent="125"/><w:compat><w:compatSetting w:name="compatibilityMode" w:uri="http://schemas.microsoft.com/office/word" w:val="15"/></w:compat></w:settings>',
        'word/numbering.xml': f'<w:numbering xmlns:w="{W}"><w:abstractNum w:abstractNumId="0"><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>',
        'word/header1.xml': f'<w:hdr xmlns:w="{W}"><w:p><w:r><w:t>Kopfzeile</w:t></w:r></w:p></w:hdr>',
        'word/footer1.xml': f'<w:ftr xmlns:w="{W}"><w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> PAGE </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>1</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p></w:ftr>',
        'word/footnotes.xml': f'<w:footnotes xmlns:w="{W}"><w:footnote w:id="1"><w:p><w:r><w:footnoteRef/></w:r><w:r><w:t>Unveränderte wissenschaftliche Anmerkung.</w:t></w:r></w:p></w:footnote></w:footnotes>',
        'docProps/core.xml': '<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Unabhängiges Testdokument</dc:title><dc:creator>Kompatibilitätstest</dc:creator></cp:coreProperties>',
        'customXml/vendor.xml': '<vendor:unknown xmlns:vendor="urn:test:vendor"><vendor:property key="preserve">Unbekannte Erweiterung</vendor:property></vendor:unknown>',
    }
    parts = {name: text.encode('utf-8') for name, text in parts.items()}
    parts['word/embeddings/opaque.bin'] = bytes(range(256)) * 4
    parts['word/media/pixel.png'] = base64.b64decode(
        'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Zl1sAAAAASUVORK5CYII=')
    return parts


def run(invoke, rel, out, zip_parts, root):
    # The GUI script saves its dedicated native-built foreign fixture in place.
    (root / 'build/jfword-compat-reference.docx').write_bytes(
        (root / 'build/jfword-compat.docx').read_bytes())
    parts = independent_parts()
    for name, data in parts.items():
        if name.endswith(('.xml', '.rels')):
            ET.fromstring(data)
    expected_file = out / 'compat-expected.txt'
    replacement_file = out / 'compat-replacement.txt'

    def probe(name, content, patch=False, reject=False, main='word/document.xml',
              expected=OLD, replacement=NEW, expected_main=None):
        data = content if isinstance(content, bytes) else zip_parts(content)
        path = out / ('compat-' + name + '.docx')
        path.write_bytes(data)
        saved = out / ('compat-' + name + '-saved.docx')
        saved.write_bytes(b'KEEP EXISTING DESTINATION')
        args = ['--check-preserved', rel(path), '--output', rel(saved)]
        if patch:
            expected_file.write_bytes(expected.encode('utf-8'))
            replacement_file.write_bytes(replacement.encode('utf-8'))
            args += ['--edit-index', '0', '--expected-file', rel(expected_file),
                     '--replacement-file', rel(replacement_file)]
        if reject:
            invoke(*args, '--expect-reject')
            assert saved.read_bytes() == b'KEEP EXISTING DESTINATION', name
            return
        response = invoke(*args)
        assert 'Preserved' in response.stdout, (name, response.stdout)
        if not patch:
            assert saved.read_bytes() == data, name
            return
        original_parts = read(path)
        saved_parts = read(saved)
        assert saved_parts.keys() == original_parts.keys(), name
        assert {n: b for n, b in saved_parts.items() if n != main} == {
            n: b for n, b in original_parts.items() if n != main}, name
        assert saved_parts[main] == expected_main, name
        node = ET.fromstring(saved_parts[main])
        namespace = '{' + node.tag[1:].split('}')[0] + '}'
        assert node.find('.//' + namespace + 't').text == replacement, name
        # Reopening an already modified file without another change is exact, too.
        saved_twice = out / ('compat-' + name + '-twice.docx')
        invoke('--check-preserved', rel(saved), '--output', rel(saved_twice))
        assert saved_twice.read_bytes() == saved.read_bytes(), name

    probe('foreign-preserved', parts)
    probe('foreign-edit', parts, patch=True,
          expected_main=parts['word/document.xml'].replace(OLD_LEAF.encode(), NEW_LEAF.encode()))
    probe('stored', zip_parts(parts, compression=zipfile.ZIP_STORED))
    probe('data-descriptors', zip_parts(parts, stream=True))
    # Original zip compression, metadata ordering and archive comment are part of the
    # unchanged-byte guarantee, not merely equality of decoded part payloads.
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, 'w', zipfile.ZIP_DEFLATED) as archive:
        archive.comment = b'Original archive comment remains exact.'
        for name, data in reversed(list(parts.items())):
            metadata = zipfile.ZipInfo(name, date_time=(2020, 2, 3, 4, 5, 6))
            metadata.compress_type = zipfile.ZIP_DEFLATED
            metadata.comment = b'Per-entry metadata'
            archive.writestr(metadata, data)
    probe('archive-metadata', stream.getvalue())

    for encoding, bom in [('utf-8', b'\xef\xbb\xbf'), ('utf-16-le', b'\xff\xfe'),
                          ('utf-16-be', b'\xfe\xff')]:
        data = dict(parts)
        text = data['word/document.xml'].decode('utf-8')
        if encoding != 'utf-8':
            text = text.replace('encoding="UTF-8"', 'encoding="UTF-16"')
        data['word/document.xml'] = bom + text.encode(encoding)
        probe(encoding + '-copy', data)
        probe(encoding + '-edit', data, patch=True,
              expected_main=bom + text.replace(OLD_LEAF, NEW_LEAF).encode(encoding))

    renamed = dict(parts)
    renamed['word/document.xml'] = renamed['word/document.xml'].replace(
        b'xmlns:w=', b'xmlns:word=').replace(b'<w:', b'<word:').replace(
        b'</w:', b'</word:').replace(b' w:', b' word:')
    probe('namespace-alias-edit', renamed, patch=True, expected_main=renamed[
        'word/document.xml'].replace(OLD_LEAF.replace('w:', 'word:').encode(),
                                   NEW_LEAF.replace('w:', 'word:').encode()))
    strict = {name: data.replace(W.encode(), b'http://purl.oclc.org/ooxml/wordprocessingml/main')
              .replace(R.encode(), b'http://purl.oclc.org/ooxml/officeDocument/relationships')
              for name, data in parts.items()}
    probe('strict-copy', strict)
    probe('strict-edit', strict, patch=True, expected_main=strict[
        'word/document.xml'].replace(OLD_LEAF.encode(), NEW_LEAF.encode()))

    relocated = {}
    for name, data in parts.items():
        renamed_path = ('documents/' + name[5:]) if name.startswith('word/') else name
        if name in ('_rels/.rels', '[Content_Types].xml'):
            data = data.replace(b'word/', b'documents/')
        relocated[renamed_path] = data
    probe('relocated-main', relocated, patch=True, main='documents/document.xml',
          expected_main=relocated['documents/document.xml'].replace(
              OLD_LEAF.encode(), NEW_LEAF.encode()))
    probe('stale-expected', parts, patch=True, reject=True, expected='Different text')
    probe('structural-newline', parts, patch=True, reject=True, replacement='Two\nparagraphs')
    probe('invalid-character', parts, patch=True, reject=True, replacement='NUL\0')

    for name, payload in [
        ('complex-field', '<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r></w:p>'),
        ('tracked-insertion', '<w:ins w:id="1" w:author="Test"><w:p><w:r><w:t>Revision</w:t></w:r></w:p></w:ins>'),
    ]:
        blocked = dict(parts)
        blocked['word/document.xml'] = blocked['word/document.xml'].replace(
            b'<w:body>', ('<w:body>' + payload).encode())
        probe(name + '-copy', blocked)
        probe(name + '-edit', blocked, patch=True, reject=True)
    blocked = dict(parts)
    blocked['word/settings.xml'] = blocked['word/settings.xml'].replace(
        b'</w:settings>', b'<w:documentProtection w:enforcement="1"/></w:settings>')
    probe('protected-copy', blocked)
    probe('protected-edit', blocked, patch=True, reject=True)
    signed = dict(parts)
    signed['_xmlsignatures/sig1.xml'] = b'<Signature xmlns="http://www.w3.org/2000/09/xmldsig#"><SignatureValue>TEST</SignatureValue></Signature>'
    probe('signature-copy', signed)
    probe('signature-edit', signed, patch=True, reject=True)

    for name, path, old, new in [
        ('external-main', '_rels/.rels', b'Target="word/document.xml"',
         b'Target="word/document.xml" TargetMode="External"'),
        ('traversal', '_rels/.rels', b'Target="word/document.xml"',
         b'Target="../word/document.xml"'),
        ('missing-main', '_rels/.rels', b'Target="word/document.xml"',
         b'Target="word/missing.xml"'),
        ('wrong-type', '[Content_Types].xml', MAIN.encode(), b'application/xml'),
        ('macro-enabled', '[Content_Types].xml', MAIN.encode(),
         b'application/vnd.ms-word.document.macroEnabled.main+xml'),
        ('duplicate-main', '_rels/.rels', b'</Relationships>',
         f'<Relationship Id="another" Type="{R}/officeDocument" Target="word/document.xml"/></Relationships>'.encode()),
        ('duplicate-id', 'word/_rels/document.xml.rels', b'Id="styles"', b'Id="settings"'),
    ]:
        broken = dict(parts)
        assert old in broken[path], name
        broken[path] = broken[path].replace(old, new)
        probe(name, broken, reject=True)
    broken = dict(parts)
    broken['word/vbaProject.bin'] = b'Unknown VBA payload'
    probe('vba-part', broken, reject=True)
    probe('encrypted-compound', bytes.fromhex('d0cf11e0a1b11ae1') + b'\0' * 512,
          reject=True)
    for suffix, content in [('truncated', zip_parts(parts)[:-10]),
                            ('not-zip', b'This is not a document')]:
        probe(suffix, content, reject=True)


def inspect_ui(root):
    """The GUI edits two leaves; every other payload and XML byte must remain intact."""
    reference = read(root / 'build/jfword-compat-reference.docx')
    saved = read(root / 'build/jfword-compat.docx')
    main = 'content/main.xml'
    assert {n: b for n, b in saved.items() if n != main} == {
        n: b for n, b in reference.items() if n != main}
    expected = reference[main].replace(
        '<w:t>Originaler Text α</w:t>'.encode(),
        '<w:t xml:space="preserve">Geänderter Fremdtext Ω</w:t>'.encode()).replace(
        b'<w:t>Zelle A</w:t>',
        '<w:t xml:space="preserve">Geänderte Tabellenzelle</w:t>'.encode())
    assert saved[main] == expected
    node = ET.fromstring(saved[main])
    texts = [t.text for t in node.iter('{' + W + '}t')]
    assert texts[:3] == ['Geänderter Fremdtext Ω', ' bleibt erhalten.',
                         'Geänderte Tabellenzelle'], texts
