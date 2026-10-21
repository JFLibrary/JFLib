"""Independent inspection of copied rich data and imported RTF through the real UI."""
import xml.etree.ElementTree as ET
import zipfile

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
B = '{http://schemas.openxmlformats.org/officeDocument/2006/bibliography}'


def inspect(root):
    fragment = (root/'build/jfword-clipboard-native.bin').read_bytes()
    assert fragment[:8] == b'JFWCLIP\x01'
    rtf = (root/'build/jfword-clipboard-export.rtf').read_bytes()
    assert rtf.startswith(b'{\\rtf1') and b'\\line' in rtf and b'\\par' in rtf
    with zipfile.ZipFile(root/'build/jfword-rich-clipboard.docx') as z:
        doc = ET.fromstring(z.read('word/document.xml'))
    ps = doc.findall(W+'body/'+W+'p')
    assert len(ps) == 2
    assert ps[0].find(W+'pPr/'+W+'pStyle').get(W+'val') == 'Heading1'
    assert ''.join(ps[0].itertext()) == 'Alpha β'
    assert ps[1].find('.//'+W+'br') is not None
    assert ''.join(ps[1].itertext()) == 'Gammaweich'
    assert all(r.find(W+'rPr/'+W+'i').get(W+'val') == '1' for r in ps[1].findall(W+'r') if ''.join(r.itertext()))
    with zipfile.ZipFile(root/'build/jfword-rich-fields.docx') as z:
        doc = ET.fromstring(z.read('word/document.xml'))
        notes = ET.fromstring(z.read('word/footnotes.xml'))
        sources = ET.fromstring(z.read('customXml/item1.xml'))
    assert len(sources.findall(B+'Source')) == 1
    fields = list(doc.iter(W+'fldSimple'))
    assert len(fields) == 1 and '\\p "100"' in fields[0].get(W+'instr')
    refs = list(doc.iter(W+'footnoteReference'))
    bodies = [n for n in notes.findall(W+'footnote') if n.get(W+'type') is None]
    assert len(refs) == len(bodies) == 2
    assert {n.get(W+'id') for n in refs} == {n.get(W+'id') for n in bodies}
    for note in bodies:
        assert [''.join(p.itertext()) for p in note.findall(W+'p')] == ['Notiz α','Beleg β']
        assert all(r.find(W+'rPr/'+W+'i').get(W+'val') == '1' for r in note.findall('.//'+W+'r') if r.find(W+'t') is not None)


def probe_ui(invoke, rel, out):
    rtf = out/'external-clipboard.rtf'
    rtf.write_bytes(br'{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\fnil\fcharset0 Calibri;}}'
        br'{\colortbl;\red198\green40\blue40;}\viewkind4\uc1\pard\ql\f0\fs22 '
        br'A{\b\cf1 B}\u945?\line Soft\par\pard\qc\fi360\i C\u-10179?\u-8704?}')
    output = out/'external-clipboard.docx'
    output.write_bytes(b'overwrite-fixture')
    script = out/'external-clipboard.uis'
    script.write_text(f'clip data rtf {rel(rtf)}\nkey ctrl+v\n'
        'expect textbox:"ABα\\nSoft\\nC😀" focused\n'
        'key ctrl+z\nexpect textbox:"ABα\\nSoft\\nC😀" missing\nkey ctrl+y\n'
        'cmd 4\nclick #1048588\nkey ctrl+a\n'
        f'type "{rel(output)}"\nclick button:"Speichern"\nclick "Ersetzen"\nexpect_nowarn\n', encoding='utf-8')
    invoke('--editor', '--script', rel(script), '--verify-dirty')
    with zipfile.ZipFile(output) as z:
        doc = ET.fromstring(z.read('word/document.xml'))
    ps = doc.findall(W+'body/'+W+'p')
    assert len(ps) == 2
    assert ps[0].find('.//'+W+'br') is not None
    bold = next(r for r in ps[0].findall(W+'r') if ''.join(r.itertext()) == 'B')
    assert bold.find(W+'rPr/'+W+'b').get(W+'val') == '1'
    assert bold.find(W+'rPr/'+W+'color').get(W+'val') == 'C62828'
    assert ps[1].find(W+'pPr/'+W+'ind').get(W+'firstLine') == '360'
    assert ps[1].find(W+'pPr/'+W+'jc').get(W+'val') == 'center'

    bad_fragment = out/'bad-clipboard-fragment.bin'
    bad_fragment.write_bytes(b'JFWCLIP\xffgarbage')
    bad_rtf = out/'bad-clipboard.rtf'
    bad_rtf.write_bytes(br'{\rtf1\b broken')
    plain = out/'clipboard-fallback.txt'
    plain.write_text('Fallback', encoding='utf-8')
    html = out/'clipboard-html.html'
    html.write_text('<p><b>Fallback</b></p>', encoding='utf-8')
    bad_html = out/'bad-clipboard.html'
    bad_html.write_text('<table><tr><td rowspan=3>broken</td></tr></table>', encoding='utf-8')
    script = out/'clipboard-fallback.uis'
    script.write_text('clip text "Original"\nkey ctrl+v ctrl+a\n'
        f'clip data docx {rel(bad_fragment)} text {rel(plain)}\nkey ctrl+v\n'
        'expect textbox:"Original" focused\nexpect textbox:"Original" value 8\nexpect textbox:"Original" max 0\n'
        'key ctrl+shift+v\nexpect textbox:"Fallback" focused\nkey ctrl+z\n'
        'expect textbox:"Original" focused\n'
        f'clip data rtf {rel(bad_rtf)} text {rel(plain)}\nkey ctrl+v\n'
        'expect textbox:"Original" focused\nkey ctrl+shift+v\nexpect textbox:"Fallback" focused\n'
        'key ctrl+z\nexpect textbox:"Original" focused\n'
        f'clip data html {rel(html)} text {rel(plain)}\nkey ctrl+v\n'
        'expect textbox:"Fallback" focused\nkey ctrl+z\nexpect textbox:"Original" focused\n'
        f'clip data html {rel(bad_html)} text {rel(plain)}\nkey ctrl+v\n'
        'expect textbox:"Original" focused\nkey ctrl+shift+v\nexpect textbox:"Fallback" focused\n'
        'expect_nowarn\n', encoding='utf-8')
    invoke('--editor', '--script', rel(script), '--verify-dirty')
