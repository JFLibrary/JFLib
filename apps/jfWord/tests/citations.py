"""Independent Word CITATION instruction fixtures and lossless supported-field checks."""
import copy
import xml.etree.ElementTree as ET
import zipfile

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
B = '{http://schemas.openxmlformats.org/officeDocument/2006/bibliography}'
for prefix, namespace in [
    ('w', W[1:-1]),
    ('w14', 'http://schemas.microsoft.com/office/word/2010/wordml'),
    ('mc', 'http://schemas.openxmlformats.org/markup-compatibility/2006'),
]:
    ET.register_namespace(prefix, namespace)


def run(invoke, rel, out, parts, zip_parts, root):
    del root  # The generated base package is the only input fixture needed here.
    sources = ET.fromstring(parts['customXml/item1.xml'])
    second = copy.deepcopy(sources[0])
    for name, value in [('Tag', 'jf4'), ('Title', 'Computing'), ('Year', '1952')]:
        second.find(B + name).text = value
    second.find('.//' + B + 'First').text = 'Grace'
    second.find('.//' + B + 'Last').text = 'Hopper'
    sources.append(second)
    source_bytes = ET.tostring(sources, encoding='utf-8', xml_declaration=True)

    def probe(name, instruction, expected=None, mutate=None):
        document = ET.fromstring(parts['word/document.xml'])
        field = document.find('.//' + W + 'fldSimple')
        field.set(W + 'instr', instruction)
        field.find('.//' + W + 't').text = 'STALE CACHE'
        if mutate is not None:
            mutate(field)
        edited = dict(parts)
        edited['customXml/item1.xml'] = source_bytes
        edited['word/document.xml'] = ET.tostring(document, encoding='utf-8', xml_declaration=True)
        path = out / ('citation-' + name + '.docx')
        path.write_bytes(zip_parts(edited))
        output = out / ('citation-' + name + '-export.docx')
        if expected is None:
            output.write_bytes(b'KEEP EXISTING FILE')
            invoke('--check-docx', rel(path), '--expect-reject', '--output', rel(output))
            assert output.read_bytes() == b'KEEP EXISTING FILE', name
            return
        invoke('--check-docx', rel(path), '--output', rel(output))
        with zipfile.ZipFile(output) as package:
            decoded = ET.fromstring(package.read('word/document.xml'))
            fields = list(decoded.iter(W + 'fldSimple'))
            assert fields[0].get(W + 'instr') == ' ' + expected + ' ', name
            assert fields[1].get(W + 'instr') == ' BIBLIOGRAPHY ', name
            cache = ''.join(fields[0].itertext())
            assert 'STALE CACHE' not in cache, name
            assert ET.fromstring(package.read('customXml/item1.xml')).findall(B + 'Source')[1].find(B + 'Tag').text == 'jf4'
        again = out / ('citation-' + name + '-roundtrip.docx')
        invoke('--check-docx', rel(output), '--output', rel(again))
        assert again.read_bytes() == output.read_bytes(), name
        return cache

    valid = [
        ('legacy-locator', 'CITATION jf2 \\p "42–44"', 'CITATION jf2 \\p "42–44"'),
        ('quoted-tag-and-spaces', '\tCITATION\t"jf2"  \\p 42–44 ', 'CITATION jf2 \\p "42–44"'),
        ('empty-default-options', 'CITATION jf2 \\p "" \\f "" \\s ""', 'CITATION jf2'),
        ('single-options', 'CITATION jf2 \\t \\y \\n \\s "Anhang A" \\f "vgl. " \\p "9"',
         'CITATION jf2 \\p "9" \\f "vgl. " \\s "Anhang A" \\n \\y \\t'),
        ('cluster', 'CITATION jf2 \\m jf4', 'CITATION jf2 \\m jf4'),
        ('ordered-cluster', 'CITATION jf4 \\m jf2', 'CITATION jf4 \\m jf2'),
        ('same-source-twice', 'CITATION jf2 \\y \\m jf2 \\n \\t', 'CITATION jf2 \\y \\m jf2 \\n \\t'),
        ('xml-unicode', 'CITATION jf2 \\f "α < β & γ " \\s " Ergänzung"',
         'CITATION jf2 \\f "α < β & γ " \\s " Ergänzung"'),
        ('max-scalar-bytes', 'CITATION jf2 \\f "' + 'é' * 64 + '"',
         'CITATION jf2 \\f "' + 'é' * 64 + '"'),
        ('max-items', 'CITATION jf2' + ' \\m jf2' * 31,
         'CITATION jf2' + ' \\m jf2' * 31),
    ]
    for name, instruction, expected in valid:
        probe(name, instruction, expected)
    scoped = probe('switch-scope',
                   'CITATION jf2 \\y \\p "9" \\m jf4 \\n \\p "12"',
                   'CITATION jf2 \\p "9" \\y \\m jf4 \\p "12" \\n')
    assert 'Lovelace' in scoped and '1843' not in scoped, scoped
    assert 'Hopper' not in scoped and '1952' in scoped, scoped
    assert scoped.index('9') < scoped.index('12'), scoped

    invalid = {
        'no-source': 'CITATION',
        'no-initial-source': 'CITATION \\m jf2',
        'missing-added-source': 'CITATION jf2 \\m',
        'dangling-added-source': 'CITATION jf2 \\m jf999',
        'zero-source': 'CITATION jf0',
        'untyped-source': 'CITATION 2',
        'invalid-source-digits': 'CITATION jf+2',
        'source-overflow': 'CITATION jf18446744073709551616',
        'unknown-switch': 'CITATION jf2 \\q value',
        'unsupported-locale': 'CITATION jf2 \\l 1031',
        'unsupported-volume': 'CITATION jf2 \\v 4',
        'unsupported-mergeformat': 'CITATION jf2 \\* MERGEFORMAT',
        'extra-token': 'CITATION jf2 jf4',
        'extra-flag-value': 'CITATION jf2 \\n false',
        'missing-value': 'CITATION jf2 \\p',
        'switch-as-value': 'CITATION jf2 \\p \\n',
        'quoted-switch': 'CITATION jf2 "\\n"',
        'quoted-keyword': '"CITATION" jf2',
        'unterminated-quote': 'CITATION jf2 \\f "unfinished',
        'escaped-quote': 'CITATION jf2 \\f "before\\"after"',
        'backslash-value': 'CITATION jf2 \\f "before\\after"',
        'quote-in-bare-value': 'CITATION jf2 \\f be"fore',
        'joined-quoted-token': 'CITATION jf2 \\p "9"\\n',
        'joined-source-switch': 'CITATION jf2\\n',
        'split-switch': 'CITATION jf2 \\ p 9',
        'newline-in-value': 'CITATION jf2 \\f "line\nnext"',
        'oversized-value': 'CITATION jf2 \\f "' + 'é' * 65 + '"',
        'too-many-items': 'CITATION jf2' + ' \\m jf2' * 32,
        'oversized-instruction': 'CITATION jf2' + ' ' * (16 << 10) + '\\n',
    }
    for switch in ['p', 'f', 's', 'n', 'y', 't']:
        arg = ' "9"' if switch in ['p', 'f', 's'] else ''
        invalid['duplicate-' + switch] = 'CITATION jf2' + (' \\' + switch + arg) * 2
        invalid['duplicate-after-m-' + switch] = 'CITATION jf2 \\m jf4' + (' \\' + switch + arg) * 2
    for name, instruction in invalid.items():
        probe(name, instruction)
    probe('unknown-field-attribute', 'CITATION jf2', mutate=lambda field: field.set(W + 'fldLock', 'true'))
    probe('nested-field-cache', 'CITATION jf2', mutate=lambda field: ET.SubElement(field, W + 'fldSimple'))
    probe('hyperlink-cache', 'CITATION jf2', mutate=lambda field: ET.SubElement(field, W + 'hyperlink'))
    probe('unsupported-cache-format', 'CITATION jf2',
          mutate=lambda field: ET.SubElement(ET.SubElement(field[0], W + 'rPr'), W + 'color').set(W + 'val', '123456'))
