"""Independent RIS/BibTeX fixtures and loss-rejection checks; Python stdlib only."""
import random
import xml.etree.ElementTree as ET
import zipfile

B = '{http://schemas.openxmlformats.org/officeDocument/2006/bibliography}'
W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'

RIS = '''TY  - BOOK
ID  - external-book
TI  - Bücher & Forschung
AU  - Lovelace, Ada
AU  - du Châtelet, Émilie
PY  - 2026
PB  - Forschungsverlag
CY  - Berlin
ET  - 2
SN  - 978-1-23456-789-0
ER  -

TY  - JOUR
ID  - external-article
TI  - Reproduzierbare Statistik
AU  - Müller, Anna
PY  - 2025
JO  - Journal of Methods
VL  - 12
IS  - 3
SP  - 17
EP  - 29
UR  - https://example.org/article?a=1&b=2
ER  -

TY  - ELEC
ID  - external-web
TI  - Offene Daten
AU  - 李, 美
PY  - 2024
T2  - Forschungsportal
UR  - https://example.org/data
ER  -
'''

BIB = r'''@book{external-book,
  title = {Bücher \& Forschung},
  author = {Lovelace, Ada and du Châtelet, Émilie},
  year = {2026},
  publisher = {Forschungsverlag},
  address = {Berlin},
  edition = {2},
  isbn = {978-1-23456-789-0}
}
@article{external-article,
  title = "Reproduzierbare Statistik",
  author = {Müller, Anna},
  year = 2025,
  journal = {Journal of Methods},
  volume = {12},
  number = {3},
  pages = {17--29},
  url = {https://example.org/article?a=1&b=2}
}
@online{external-web,
  title = {Offene Daten},
  author = {李, 美},
  year = {2024},
  howpublished = {Forschungsportal},
  url = {https://example.org/data}
}
'''


def expected(pages):
    return [
        {'SourceType': 'Book', 'Title': 'Bücher & Forschung', 'Year': '2026',
         'Publisher': 'Forschungsverlag', 'City': 'Berlin', 'Edition': '2',
         'StandardNumber': '978-1-23456-789-0',
         'authors': [{'Last': 'Lovelace', 'First': 'Ada'},
                     {'Last': 'du Châtelet', 'First': 'Émilie'}]},
        {'SourceType': 'JournalArticle', 'Title': 'Reproduzierbare Statistik',
         'Year': '2025', 'JournalName': 'Journal of Methods', 'Volume': '12',
         'Issue': '3', 'Pages': pages,
         'URL': 'https://example.org/article?a=1&b=2',
         'authors': [{'Last': 'Müller', 'First': 'Anna'}]},
        {'SourceType': 'InternetSite', 'Title': 'Offene Daten', 'Year': '2024',
         'InternetSiteTitle': 'Forschungsportal', 'URL': 'https://example.org/data',
         'authors': [{'Last': '李', 'First': '美'}]},
    ]


def records(root):
    """Read the separately specified Word XML representation without exchange code."""
    assert root.tag == B+'Sources', 'exchange output must be a Word Sources document'
    result = []
    for source in root:
        assert source.tag == B+'Source', 'only source records are exported'
        record = {}
        for item in source:
            assert item.tag.startswith(B), 'bibliography namespace must be preserved'
            name = item.tag[len(B):]
            if name == 'Tag':
                continue
            if name == 'Author':
                author = item.find(B+'Author')
                assert author is not None, 'Word author wrapper is required'
                corporate = author.find(B+'Corporate')
                record['authors'] = corporate.text if corporate is not None else [
                    {field.tag[len(B):]: field.text or '' for field in person}
                    for person in author.findall(B+'NameList/'+B+'Person')]
            else:
                assert name not in record, 'duplicate output metadata: '+name
                record[name] = item.text or ''
        result.append(record)
    return result


def run(invoke, rel, out):
    def accepted(name, extension, raw, wanted, roundtrip=False):
        path = out/('exchange-'+name+extension)
        path.write_bytes(raw)
        xml = out/('exchange-'+name+'.xml')
        docx = out/('exchange-'+name+'.docx')
        invoke('--check-sources', rel(path), '--output', rel(xml), '--docx-output', rel(docx))
        root = ET.parse(xml).getroot()
        assert records(root) == wanted, name+': all XML source metadata and author order'
        tags = ['jf'+str(i+2) for i in range(len(wanted))]
        assert [s.find(B+'Tag').text for s in root] == tags, name+': allocate document source IDs'
        with zipfile.ZipFile(docx) as archive:
            assert archive.testzip() is None, name+': ZIP integrity'
            assert records(ET.fromstring(archive.read('customXml/item1.xml'))) == wanted, name+': DOCX source metadata'
            document = ET.fromstring(archive.read('word/document.xml'))
            codes = [node.attrib[W+'instr'].strip() for node in document.iter(W+'fldSimple')]
            assert codes == ['CITATION '+tag for tag in tags]+['BIBLIOGRAPHY'], name+': cite every imported record'
            text = ''.join(document.itertext())
            assert all(record['Title'] in text for record in wanted), name+': rendered bibliography includes every title'
        if roundtrip:
            exported = out/('exchange-'+name+'-export'+extension)
            restored = out/('exchange-'+name+'-restored.xml')
            invoke('--check-sources', rel(xml), '--output', rel(exported))
            invoke('--check-sources', rel(exported), '--output', rel(restored))
            assert records(ET.parse(restored).getroot()) == wanted, name+': own-format export/reimport keeps all metadata'

    def rejected(name, extension, raw):
        path = out/('exchange-bad-'+name+extension)
        path.write_bytes(raw)
        output = out/('exchange-bad-'+name+'-output.xml')
        docx = out/('exchange-bad-'+name+'-output.docx')
        marker = b'KEEP EXISTING FILE: '+name.encode('ascii')
        output.write_bytes(marker)
        docx.write_bytes(marker)
        invoke('--check-sources', rel(path), '--output', rel(output),
               '--docx-output', rel(docx), '--expect-reject')
        assert output.read_bytes() == marker, name+': rejection preserves existing source export'
        assert docx.read_bytes() == marker, name+': rejection preserves existing DOCX export'

    accepted('ris', '.ris', RIS.encode('utf-8'), expected('17-29'), True)
    accepted('bib', '.bib', BIB.encode('utf-8'), expected('17--29'), True)
    accepted('ris-utf16', '.RIS', RIS.replace('\n', '\r\n').encode('utf-16'), expected('17-29'))
    accepted('bib-utf16be', '.bibtex', b'\xfe\xff'+BIB.encode('utf-16-be'), expected('17--29'))
    accepted('ris-bom-aliases', '.ris', ('\ufeff'+RIS.replace('TI  -', 'T1  -')
             .replace('AU  -', 'A1  -').replace('PY  - 2026', 'Y1  - 2026///')).encode('utf-8'), expected('17-29'))
    accepted('bib-corporate', '.bib', '@online{institute, title={Offene Forschung}, author={{Institut für Forschung}}}'.encode('utf-8'),
             [{'SourceType': 'InternetSite', 'Title': 'Offene Forschung', 'authors': 'Institut für Forschung'}], True)

    ris_bad = {
        'missing-end': RIS.rsplit('ER  -', 1)[0],
        'nested-record': RIS.replace('ER  -', '', 1),
        'field-before-record': 'TI  - orphan\n'+RIS,
        'unknown-field': RIS.replace('TI  -', 'ZZ  -', 1),
        'duplicate-title': RIS.replace('TI  - Bücher & Forschung', 'TI  - First\nTI  - Second'),
        'unknown-type': RIS.replace('TY  - BOOK', 'TY  - THES', 1),
        'orphan-end-page': RIS.replace('SP  - 17\n', ''),
        'publication-date-loss': RIS.replace('PY  - 2026', 'PY  - 2026/10/09'),
        'empty-title': RIS.replace('TI  - Bücher & Forschung', 'TI  - '),
        'too-long-title': RIS.replace('Bücher & Forschung', 'x'*4097),
        'too-many-authors': RIS.replace('AU  - Lovelace, Ada', '\n'.join('AU  - Family, Name' for _ in range(65))),
    }
    bib_bad = {
        'missing-end': BIB.rstrip()[:-1],
        'unknown-field': BIB.replace('publisher =', 'x_unknown =', 1),
        'duplicate-field': BIB.replace('year = {2026},', 'year = {2026}, year = {2026},'),
        'unknown-type': BIB.replace('@book', '@inproceedings', 1),
        'duplicate-key': BIB.replace('external-article', 'external-book'),
        'concatenation': BIB.replace('title = {Bücher \\& Forschung}', 'title = {First} # {Second}'),
        'bare-macro': BIB.replace('year = {2026}', 'year = currentyear'),
        'tex-command': BIB.replace('Reproduzierbare Statistik', r'\textbf{Statistik}'),
        'nested-title': BIB.replace('Bücher \\& Forschung', 'Bücher {ABC} Forschung'),
        'author-suffix': BIB.replace('Lovelace, Ada', 'Lovelace, Jr, Ada'),
        'empty-title': BIB.replace('Bücher \\& Forschung', ''),
        'too-long-title': BIB.replace('Bücher \\& Forschung', 'x'*4097),
        'too-many-authors': BIB.replace('Lovelace, Ada', ' and '.join('Family, Name' for _ in range(65))),
    }
    for extension, variants in [('.ris', ris_bad), ('.bib', bib_bad)]:
        for name, value in variants.items():
            rejected(extension[1:]+'-'+name, extension, value.encode('utf-8'))
        seed = RIS if extension == '.ris' else BIB
        rejected(extension[1:]+'-invalid-utf8', extension, b'\xff'+seed.encode('utf-8'))
        rejected(extension[1:]+'-truncated-utf16', extension, seed.encode('utf-16')[:-1])
        # Seeded structural/encoding mutations are all independently invalid. This
        # drives the real process without a second implementation of either parser.
        rng = random.Random(20261010)
        raw = seed.encode('utf-8')
        for index in range(12):
            if index % 3 == 0:
                pos = rng.randrange(len(raw)+1)
                mutated = raw[:pos]+b'\0'+raw[pos:]
            elif index % 3 == 1:
                suffix = str(rng.randrange(1000000)).encode('ascii')
                mutated = (b'TY  - BOOK\nTI  - Mutation\nZZ  - '+suffix+b'\nER  -\n'
                           if extension == '.ris' else b'@book{k, title={Mutation}, x_unknown_'+suffix+b'={1}}')
            else:
                mutated = (raw[:raw.rfind(b'ER  -')]+b'ER' if extension == '.ris' else raw.rstrip()[:-1])
            rejected(extension[1:]+'-mutation-'+str(index), extension, mutated)

    # An XML-only GUID must never silently disappear during a format conversion.
    unsupported = out/'exchange-guid.xml'
    unsupported.write_text('<b:Sources xmlns:b="'+B[1:-1]+'"><b:Source><b:Tag>external</b:Tag>'
                           '<b:SourceType>Book</b:SourceType><b:Title>Keep metadata</b:Title>'
                           '<b:Guid>{12345678-1234-1234-1234-123456789ABC}</b:Guid></b:Source></b:Sources>', encoding='utf-8')
    for extension in ['.ris', '.bib']:
        output = out/('exchange-guid-export'+extension)
        output.write_bytes(b'KEEP UNSUPPORTED METADATA')
        invoke('--check-sources', rel(unsupported), '--output', rel(output), '--expect-reject')
        assert output.read_bytes() == b'KEEP UNSUPPORTED METADATA', extension+': unsupported export preserves existing file'
