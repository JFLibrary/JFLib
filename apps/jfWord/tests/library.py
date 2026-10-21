"""Independent native-library XML, process locking and stale-writer contracts."""
import copy
from concurrent.futures import ThreadPoolExecutor
import time
import xml.etree.ElementTree as ET
import zipfile

NS = 'urn:jfword:source-library:1'
B = 'http://schemas.openxmlformats.org/officeDocument/2006/bibliography'


def fixture():
    root = ET.Element('{' + NS + '}SourceLibrary', {'version': '1', 'next-id': '901'})
    sources = ET.SubElement(root, '{' + B + '}Sources')
    for tag, title, author in [('jf17', 'Überblick & Methoden', 'Forschung <Nord>'),
                               ('jf300', 'Notes α β 𝄞', 'Institute South')]:
        source = ET.SubElement(sources, '{' + B + '}Source')
        for key, value in [('Tag', tag), ('SourceType', 'Book'), ('Title', title),
                           ('Year', '2026'), ('Publisher', 'Press'), ('City', 'Köln'),
                           ('Edition', '2'), ('Comments', 'Line 1\nLine 2'),
                           ('Guid', 'stable-' + tag), ('LCID', 'de')]:
            ET.SubElement(source, '{' + B + '}' + key).text = value
        people = ET.SubElement(ET.SubElement(source, '{' + B + '}Author'), '{' + B + '}Author')
        ET.SubElement(people, '{' + B + '}Corporate').text = author
    return root


def encode(root, encoding='utf-8'):
    if encoding == 'utf-16-be':
        text = ET.tostring(root, encoding='unicode')
        return b'\xfe\xff' + ('<?xml version="1.0" encoding="UTF-16BE"?>' + text).encode(encoding)
    return ET.tostring(root, encoding=encoding, xml_declaration=True)


def semantic(node):
    children = [semantic(c) for c in node]
    if node.tag == '{' + B + '}Source':
        children.sort(key=lambda child: child[0])
    return node.tag, node.attrib, node.text or '', children


def run(invoke, rel, out, root, target, exe):
    import build_rs
    original = fixture()
    path = out / 'library-independent.jflib'
    normalized = out / 'library-normalized.jflib'
    path.write_bytes(encode(original))
    invoke('--check-library', rel(path), '--output', rel(normalized))
    assert semantic(ET.parse(normalized).getroot()) == semantic(original)
    stable = out / 'library-stable.jflib'
    invoke('--check-library', rel(normalized), '--output', rel(stable))
    assert stable.read_bytes() == normalized.read_bytes()
    for encoding in ['utf-16', 'utf-16-be']:
        path.write_bytes(encode(original, encoding))
        invoke('--check-library', rel(path), '--output', rel(stable))
        assert stable.read_bytes() == normalized.read_bytes()

    cases = []
    for attr, value in [('version', '2'), ('next-id', '300'), ('next-id', '0'),
                        ('next-id', '-1'), ('next-id', '2147483648'), ('future', 'x')]:
        invalid = copy.deepcopy(original); invalid.set(attr, value); cases.append(encode(invalid))
    for attr in ['version', 'next-id']:
        invalid = copy.deepcopy(original); del invalid.attrib[attr]; cases.append(encode(invalid))
    invalid = copy.deepcopy(original); invalid[0][1].find('{' + B + '}Tag').text = 'jf17'; cases.append(encode(invalid))
    invalid = copy.deepcopy(original); ET.SubElement(invalid, '{' + NS + '}Future'); cases.append(encode(invalid))
    invalid = copy.deepcopy(original); ET.SubElement(invalid[0][0], '{' + B + '}Future').text = 'x'; cases.append(encode(invalid))
    invalid = copy.deepcopy(original); invalid[0].set('SelectedStyle', 'x'); cases.append(encode(invalid))
    cases += [b'', b'<broken>', b' ' * ((4 << 20) + 1), encode(original).replace(NS.encode(), b'urn:unknown')]
    for data in cases:
        path.write_bytes(data)
        stable.write_bytes(b'KEEP EXISTING OUTPUT')
        invoke('--check-library', rel(path), '--output', rel(stable), '--expect-reject')
        assert stable.read_bytes() == b'KEEP EXISTING OUTPUT'
    path.write_bytes(encode(original))

    # Real independent processes use the production store, including its platform lock.
    count = 0
    def native(*args, code=0):
        nonlocal count
        result = build_rs.run_exe(target, exe, list(args), timeout=15)
        assert result.returncode == code, (target, args, result.returncode, result.stdout, result.stderr)
        count += 1
        return result

    main = out / 'library-concurrent.jflib'
    ready = out / 'library-ready'
    incoming = out / 'library-incoming.jflib'
    changed = copy.deepcopy(original)
    changed[0][0].find('{' + B + '}Title').text = 'Changed from another process'
    incoming.write_bytes(encode(changed))
    main.write_bytes(normalized.read_bytes())

    def await_ready(future):
        deadline = time.monotonic() + 5
        while not ready.exists():
            if future.done():
                future.result()
                raise AssertionError('Native process exited before readiness')
            assert time.monotonic() < deadline, 'Native process readiness timeout'
            time.sleep(.02)

    with ThreadPoolExecutor(max_workers=1) as pool:
        ready.unlink(missing_ok=True)
        held = pool.submit(native, '--hold-library-lock', rel(main) + '.lock',
                           '--ready', rel(ready), '--wait-ms', '2200')
        await_ready(held)
        blocked = native('--library-save', rel(main), '--library-input', rel(incoming), code=73)
        assert 'Busy' in blocked.stdout
        assert main.read_bytes() == normalized.read_bytes()
        held.result()
        native('--library-save', rel(main), '--library-input', rel(incoming))
        assert semantic(ET.parse(main).getroot()) == semantic(changed)
        assert (out / (main.name + '.bak')).read_bytes() == normalized.read_bytes()

        # A process which read the old snapshot must reject a newer writer even after unlock.
        ready.unlink(missing_ok=True)
        stale = pool.submit(native, '--library-save', rel(main), '--library-input', rel(path),
                            '--ready', rel(ready), '--wait-ms', '2200', code=74)
        await_ready(stale)
        newest = copy.deepcopy(changed)
        newest[0][1].find('{' + B + '}Title').text = 'Newest remains intact'
        incoming.write_bytes(encode(newest))
        native('--library-save', rel(main), '--library-input', rel(incoming))
        assert 'Conflict' in stale.result().stdout
        assert semantic(ET.parse(main).getroot()) == semantic(newest)
        assert semantic(ET.parse(str(main) + '.bak').getroot()) == semantic(changed)
    if target == 'linux':
        # Device/FIFO/socket modes are rejected before opening any non-regular file.
        native('--library-save', '/dev/null', '--library-input', rel(path), code=2)
    return count


def inspect_ui(root):
    def parts(path):
        with zipfile.ZipFile(path) as package:
            assert package.testzip() is None
            return {name: package.read(name) for name in package.namelist()}
    before = parts(root / 'build/jfword-merge-input.docx')
    after = parts(root / 'build/jfword-library-input.docx')
    expected = ET.fromstring(before['customXml/item1.xml'])
    for field, value in [('Title', 'Bibliotheksmethoden'), ('Year', '2030')]:
        expected[0].find('{' + B + '}' + field).text = value
    ET.SubElement(expected[0], '{' + B + '}Publisher').text = 'Bibliotheksverlag'
    assert semantic(ET.fromstring(after['customXml/item1.xml'])) == semantic(expected)
    assert [s.find('{' + B + '}Tag').text for s in expected] == ['jf2', 'jf3', 'jf4', 'jf5']
    w = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
    old_doc = ET.fromstring(before['word/document.xml'])
    new_doc = ET.fromstring(after['word/document.xml'])
    for doc in [old_doc, new_doc]:
        for field in doc.iter(w + 'fldSimple'):
            field[:] = []  # Exclude regenerated display text; field instructions stay intact.
    assert semantic(old_doc) == semantic(new_doc), 'Document text, IDs, references and options retained'
    for name in before.keys() - {'customXml/item1.xml', 'word/document.xml'}:
        assert before[name] == after[name], name

    library = ET.parse(root / 'build/jfword-library-ui/sources.jflib').getroot()
    assert library.tag == '{' + NS + '}SourceLibrary' and library.attrib == {'version': '1', 'next-id': '3'}
    assert len(library[0]) == 2
    source = copy.deepcopy(expected[0]); source.find('{' + B + '}Tag').text = 'jf1'
    assert semantic(library[0][0]) == semantic(source)
    assert library[0][1].find('{' + B + '}Tag').text == 'jf2'
    assert library[0][1].find('{' + B + '}Title').text == 'Globaler Zusatz'
    damaged = ET.parse(root / 'build/jfword-library-ui/sources.jflib.damaged').getroot()
    assert len(damaged[0]) == 0 and damaged.attrib['next-id'] == '2'
    copied = parts(root / 'build/jfword-library-copy.docx')
    copied_sources = ET.fromstring(copied['customXml/item1.xml'])
    assert len(copied_sources) == 1 and semantic(copied_sources[0]) == semantic(expected[0])
    assert not list(ET.fromstring(copied['word/document.xml']).iter(w + 'fldSimple'))


def inspect_recovery(root):
    path = root / 'build/jfword-library-recovery-ui/sources.jflib'
    assert path.read_bytes() == path.with_suffix('.jflib.bak').read_bytes()
    assert path.with_suffix('.jflib.damaged').read_bytes() == b'<broken-library>'
    restored = ET.parse(path).getroot()
    assert restored.attrib['next-id'] == '901'
    assert [s.find('{' + B + '}Tag').text for s in restored[0]] == ['jf17', 'jf300']
