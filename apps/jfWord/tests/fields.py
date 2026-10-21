"""Independent complex Word field fixtures; native import and conservative rejection."""
import copy
import xml.etree.ElementTree as ET
import zipfile

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
XML = '{http://www.w3.org/XML/1998/namespace}'


def marker(kind):
    run = ET.Element(W + 'r')
    ET.SubElement(run, W + 'fldChar').set(W + 'fldCharType', kind)
    return run


def instruction(text):
    run = ET.Element(W + 'r')
    node = ET.SubElement(run, W + 'instrText', {XML + 'space': 'preserve'})
    node.text = text
    return run


def run(invoke, rel, out, parts, zip_parts, root):
    reordered = ET.fromstring(parts['word/document.xml'])
    body = reordered.find(W+'body')
    bibliography = body[1]
    body.remove(bibliography)
    body.insert(0, bibliography)
    data = dict(parts)
    data['word/document.xml'] = ET.tostring(reordered, encoding='utf-8', xml_declaration=True)
    (root/'build/jfword-fields-before.docx').write_bytes(zip_parts(data))
    def probe(name, rewrite, accepted=True):
        document = ET.fromstring(parts['word/document.xml'])
        for p in document.findall(W + 'body/' + W + 'p'):
            for node in list(p):
                if node.tag == W + 'fldSimple':
                    index = list(p).index(node)
                    replacement = rewrite(node)
                    p.remove(node)
                    for n in reversed(replacement):
                        p.insert(index, n)
        data = dict(parts)
        data['word/document.xml'] = ET.tostring(document, encoding='utf-8', xml_declaration=True)
        path = out / f'complex-{name}.docx'
        path.write_bytes(zip_parts(data))
        saved = out / f'complex-{name}-saved.docx'
        if not accepted:
            saved.write_bytes(b'UNCHANGED')
            invoke('--check-docx', rel(path), '--expect-reject', '--output', rel(saved))
            assert saved.read_bytes() == b'UNCHANGED'
            return
        invoke('--check-docx', rel(path), '--output', rel(saved))
        with zipfile.ZipFile(saved) as z:
            canonical = ET.fromstring(z.read('word/document.xml'))
            fields = list(canonical.iter(W + 'fldSimple'))
            assert len(fields) == 2
            assert fields[0].get(W + 'instr').strip() == 'CITATION jf2 \\p "42–44"'
            assert fields[1].get(W + 'instr').strip() == 'BIBLIOGRAPHY'
            assert 'STALE' not in ''.join(canonical.itertext())
            assert 'Lovelace' in ''.join(canonical.itertext())
        invoke('--check-docx', rel(saved))
        if name == 'split':
            (root / 'build/jfword-complex-fields.docx').write_bytes(path.read_bytes())

    def split(node):
        code = node.get(W + 'instr')
        cache = ET.Element(W + 'r')
        ET.SubElement(cache, W + 't').text = 'STALE'
        return [marker('begin'), *(instruction(c) for c in code), marker('separate'), cache, marker('end')]

    probe('split', split)
    probe('no-cache', lambda n: [marker('begin'), instruction(n.get(W + 'instr')), marker('end')])
    probe('empty-cache', lambda n: [marker('begin'), instruction(n.get(W + 'instr')), marker('separate'), marker('end')])

    def single(n):
        result = ET.Element(W + 'r')
        for part in split(n):
            result.extend(copy.deepcopy(list(part)))
        return [result]
    probe('single-run', single)
    def dirty(n):
        nodes = split(n)
        nodes[0][0].set(W+'dirty', 'true')
        nodes[0][0].set(W+'fldLock', '0')
        return nodes
    probe('dirty-unlocked', dirty)
    def dirty_simple(n):
        n.set(W+'dirty', '1')
        n.set(W+'fldLock', 'false')
        return [n]
    probe('dirty-simple', dirty_simple)

    for name, rewrite in [
        ('unclosed', lambda n: split(n)[:-1]),
        ('orphan-end', lambda n: [marker('end')]),
        ('orphan-instruction', lambda n: [instruction(n.get(W + 'instr'))]),
        ('nested', lambda n: [marker('begin'), *split(n), marker('end')]),
        ('double-separator', lambda n: [*split(n)[:-1], marker('separate'), marker('end')]),
        ('text-before-separator', lambda n: [marker('begin'), *list(n), instruction(n.get(W + 'instr')), marker('end')]),
        ('instruction-after-separator', lambda n: [*split(n)[:-1], instruction(' EXTRA '), marker('end')]),
        ('unknown-field', lambda n: [marker('begin'), instruction(' INCLUDETEXT "secret" '), marker('end')]),
        ('oversized-code', lambda n: [marker('begin'), instruction(' ' * ((16 << 10) + 1)), marker('end')]),
    ]:
        probe(name, rewrite, False)

    def locked(n):
        nodes = split(n)
        nodes[0][0].set(W + 'fldLock', 'true')
        return nodes
    probe('locked', locked, False)
    def styled_result(n):
        nodes = split(n)
        props = ET.Element(W+'rPr')
        ET.SubElement(props, W+'i')
        nodes[-2].insert(0, props)
        return nodes
    probe('formatted-cache', styled_result, False)
    def styled_simple(n):
        props = ET.Element(W+'rPr')
        ET.SubElement(props, W+'i')
        n[0].insert(0, props)
        return [n]
    probe('formatted-simple-cache', styled_simple, False)

    # The same parser is used inside note stories. Keep note markers and surrounding runs.
    with zipfile.ZipFile(root/'build/jfword-notes.docx') as z:
        noted = {n:z.read(n) for n in z.namelist()}
    changed = 0
    for part in ['word/footnotes.xml', 'word/endnotes.xml']:
        if part not in noted:
            continue
        tree = ET.fromstring(noted[part])
        for p in tree.iter(W+'p'):
            for field in list(p):
                if field.tag != W+'fldSimple':
                    continue
                index = list(p).index(field)
                p.remove(field)
                for n in reversed(split(field)):
                    p.insert(index, n)
                changed += 1
        noted[part] = ET.tostring(tree, encoding='utf-8', xml_declaration=True)
    assert changed > 0
    path = out/'complex-notes.docx'
    saved = out/'complex-notes-saved.docx'
    path.write_bytes(zip_parts(noted))
    invoke('--check-docx', rel(path), '--output', rel(saved))
    with zipfile.ZipFile(saved) as z:
        assert sum(len(list(ET.fromstring(z.read(p)).iter(W+'fldSimple'))) for p in ['word/footnotes.xml','word/endnotes.xml'] if p in z.namelist()) == changed
