"""Independently inspect all source fields and every reference after native reconciliation."""
import copy
import re
import xml.etree.ElementTree as ET
import zipfile

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
B = '{http://schemas.openxmlformats.org/officeDocument/2006/bibliography}'


def parts(path):
    with zipfile.ZipFile(path) as package:
        assert package.testzip() is None
        return {name: package.read(name) for name in package.namelist()}


def tree(node):
    return node.tag, node.attrib, node.text or '', [tree(child) for child in node]


def run(invoke, rel, out, root):
    before = parts(root / 'build/jfword-merge-input.docx')
    merged = root / 'build/jfword-merged.docx'
    after = parts(merged)
    old_sources = ET.fromstring(before['customXml/item1.xml'])
    new_sources = ET.fromstring(after['customXml/item1.xml'])
    expected = copy.deepcopy(old_sources)
    expected[0] = copy.deepcopy(old_sources[1])
    expected[0].find(B + 'Tag').text = 'jf2'
    expected.remove(expected[1])
    assert tree(new_sources) == tree(expected), 'Every metadata field and creator component retained'
    assert [s.find(B + 'Tag').text for s in new_sources] == ['jf2', 'jf4', 'jf5']
    assert new_sources[0].find(B + 'Comments') is None, 'Explicit empty incoming field clears old value'

    old_doc = ET.fromstring(before['word/document.xml'])
    new_doc = ET.fromstring(after['word/document.xml'])
    old_fields = list(old_doc.iter(W + 'fldSimple'))
    new_fields = list(new_doc.iter(W + 'fldSimple'))
    assert len(old_fields) == len(new_fields) == 4
    for old, new in zip(old_fields, new_fields):
        old_instruction = old.attrib[W + 'instr']
        expected_instruction = re.sub(r'\bjf3\b', 'jf2', old_instruction)
        assert new.attrib[W + 'instr'] == expected_instruction
        # Compare the complete document separately from regenerated field caches.
        old.attrib[W + 'instr'] = expected_instruction
        old[:] = []
        new[:] = []
    assert tree(old_doc) == tree(new_doc), 'Text, paragraph IDs, layout and citation positions retained'
    cluster = new_fields[1].attrib[W + 'instr']
    assert cluster.count('jf2') == 3 and '\\m jf4' in cluster
    assert '\\p "8–10"' in cluster and '\\f "vgl. "' in cluster
    assert '\\s "; ergänzt"' in cluster and '\\n' in cluster and '\\y' in cluster and '\\t' in cluster
    for name in before.keys() - {'customXml/item1.xml', 'word/document.xml'}:
        assert before[name] == after[name], name

    exported = out / 'merge-independent-roundtrip.docx'
    invoke('--check-docx', rel(merged), '--output', rel(exported))
    assert exported.read_bytes() == merged.read_bytes()


def inspect_ui(root):
    # The GUI deliberately selects the same result as the native fixture, then saves/reloads.
    assert parts(root / 'build/jfword-merge-input.docx') == parts(root / 'build/jfword-merged.docx'), \
        'GUI-selected fields and remapped citations must match the independently inspected model result'
