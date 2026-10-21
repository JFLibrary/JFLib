"""Independent OPC/ZIP checks and malformed-input corpus for the actual native binaries.
Build first: python build_rs.py app jfWord --target both
Run: python apps/jfWord/tests/check.py [--target win|linux|both]
Only Python's standard library is required; generated fixtures stay under build/.
"""
import argparse
import io
from pathlib import Path
import random
import struct
import sys
import zipfile
import xml.etree.ElementTree as ET
import science
import formatting
import exchange
import citations
import merging
import library
import notes
import compatibility
import shell
import stories
import rich_text
import fields
import clipboard

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
import build_rs

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'
B = '{http://schemas.openxmlformats.org/officeDocument/2006/bibliography}'

def zip_parts(parts, compression=zipfile.ZIP_DEFLATED, stream=False):
    class Sequential(io.BytesIO):
        def seekable(self): return False
        def seek(self, *args): raise io.UnsupportedOperation()
    out = Sequential() if stream else io.BytesIO()
    with zipfile.ZipFile(out, 'w', compression=compression) as z:
        for name, data in parts.items(): z.writestr(name, data)
    return out.getvalue()

def inspect(path):
    with zipfile.ZipFile(path) as z:
        assert z.testzip() is None
        parts = {n:z.read(n) for n in z.namelist()}
    for name, data in parts.items(): ET.fromstring(data)
    doc = ET.fromstring(parts['word/document.xml'])
    instructions = [n.attrib[W+'instr'].strip() for n in doc.iter(W+'fldSimple')]
    assert instructions == ['CITATION jf2 \\p "42–44"', 'BIBLIOGRAPHY'], instructions
    sources = ET.fromstring(parts['customXml/item1.xml'])
    assert sources.find('.//'+B+'Title').text == 'Notizen & Methoden'
    assert sources.find('.//'+B+'Tag').text == 'jf2'
    assert len(doc.findall('.//'+W+'p')) == 2
    text = ''.join(doc.itertext())
    assert 'Grüße & Forschung <2026>' in text and 'Lovelace' in text
    assert doc.find('.//'+W+'pgSz').attrib[W+'w'] == '11906'
    return parts

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--target', choices=['win','linux','both'], default='both')
    ap.add_argument('--build-dir', default='build', help='Build output directory relative to JFLib')
    args = ap.parse_args()
    targets = ['win','linux'] if args.target == 'both' else [args.target]
    total = 0
    for target in targets:
        out = ROOT/'build/jfword-check'/target
        out.mkdir(parents=True, exist_ok=True)
        exe = ROOT/args.build_dir/target/('jfWord.exe' if target == 'win' else 'jfWord')
        def invoke(*args):
            nonlocal total
            r = build_rs.run_exe(target, exe, list(args), timeout=40)
            if r.returncode or 'FAIL' in r.stdout or 'UIWARN' in r.stdout:
                raise AssertionError(f'{target} {args}: exit {r.returncode}\n{r.stdout}\n{r.stderr}')
            total += 1
            return r
        def rel(p): return p.relative_to(ROOT).as_posix()
        sample = out/'sample.docx'
        invoke('--self-test','--fixture',rel(sample))
        parts = inspect(sample)
        rich_text.prepare(parts, zip_parts, ROOT)
        rich_text.probe_ui(invoke, rel, out)
        clipboard.probe_ui(invoke, rel, out)
        science.run(invoke, rel, out)
        exchange.run(invoke, rel, out)
        formatting.run(invoke, rel, out, parts, zip_parts, ROOT)
        citations.run(invoke, rel, out, parts, zip_parts, ROOT)
        fields.run(invoke, rel, out, parts, zip_parts, ROOT)
        merging.run(invoke, rel, out, ROOT)
        library_processes = library.run(invoke, rel, out, ROOT, target, exe)
        total += library_processes
        notes.run(invoke, rel, out, zip_parts, ROOT)
        compatibility.run(invoke, rel, out, zip_parts, ROOT)
        stories.run(invoke, rel, out, zip_parts, ROOT)
        shell.run(invoke, rel, out, ROOT, sample)
        for name, data in [('deflated',zip_parts(parts)),('descriptor',zip_parts(parts,stream=True)),
                           ('stored',zip_parts(parts,zipfile.ZIP_STORED))]:
            p = out/(name+'.docx'); p.write_bytes(data)
            roundtrip = out/(name+'-roundtrip.docx')
            invoke('--check-docx',rel(p),'--output',rel(roundtrip))
            assert inspect(roundtrip) == parts
        # Namespace prefixes are aliases, not hard-coded parser tokens.
        prefix = dict(parts)
        prefix['word/document.xml'] = prefix['word/document.xml'].replace(b'xmlns:w=',b'xmlns:q=').replace(b'<w:',b'<q:').replace(b'</w:',b'</q:').replace(b' w:',b' q:')
        p = out/'prefix.docx'; p.write_bytes(zip_parts(prefix)); invoke('--check-docx',rel(p))
        # Independently write UTF-16 XML parts inside a conventional deflated ZIP.
        utf16 = {n:b.decode('utf-8').replace('encoding="UTF-8"','encoding="UTF-16"').encode('utf-16') for n,b in parts.items()}
        p = out/'utf16.docx'; p.write_bytes(zip_parts(utf16)); invoke('--check-docx',rel(p))
        bad = {}
        extra = dict(parts); extra['word/vbaProject.bin'] = b'opaque macro data'
        bad['macro'] = zip_parts(extra)
        extra = dict(parts); extra['word/header1.xml'] = b'<header/>'
        bad['unknown-part'] = zip_parts(extra)
        extra = dict(parts); extra['../document.xml'] = b'x'
        bad['traversal'] = zip_parts(extra)
        extra = dict(parts); extra['oversize.xml'] = b'x'*(9 << 20)
        bad['expansion-budget'] = zip_parts(extra)
        for name, old, new in [
            ('table',b'<w:body>',b'<w:body><w:tbl/>'),
            ('unsupported-theme-color',b'<w:r>',b'<w:r><w:rPr><w:color w:val="123456" w:themeColor="accent1"/></w:rPr>'),
            ('unknown-field',b' CITATION ',b' INCLUDETEXT '),
            ('dangling-source',b'CITATION jf2',b'CITATION jf999'),
            ('duplicate-id',b'paraId="00000003"',b'paraId="00000001"'),
            ('dtd',b'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>',b'<!DOCTYPE r SYSTEM "file:///private">'),
            ('oversize-id',b'paraId="00000001"',b'paraId="FFFFFFFF"'),
            ('wrong-namespace',b'wordprocessingml/2006/main',b'wordprocessingml/2006/FAKE'),
        ]:
            edited = dict(parts)
            assert old in edited['word/document.xml'], name
            edited['word/document.xml'] = edited['word/document.xml'].replace(old,new,1)
            bad[name] = zip_parts(edited)
        data = sample.read_bytes()
        bad['truncated'] = data[:-10]
        corrupt = bytearray(data); corrupt[70] ^= 1; bad['crc'] = corrupt
        corrupt = bytearray(data); corrupt[6] |= 1; bad['encrypted'] = corrupt
        corrupt = bytearray(data); struct.pack_into('<I',corrupt,len(corrupt)-6,0); bad['bad-directory'] = corrupt
        dup = io.BytesIO()
        with zipfile.ZipFile(dup,'w') as z:
            z.writestr('word/document.xml',b'x'); z.writestr('WORD/document.xml',b'y')
        bad['duplicate-case'] = dup.getvalue()
        for name,data in bad.items():
            p = out/(name+'.docx'); p.write_bytes(data)
            invoke('--check-docx',rel(p),'--expect-reject')
        # Deterministic truncation and mutation corpus; valid mutations may be accepted,
        # but no variant may crash or hang. Semantic rejections are exit=1.
        rng = random.Random(20261010)
        data = sample.read_bytes()
        for i in range(64):
            b = bytearray(data)
            if i%2: del b[rng.randrange(len(b)):]
            else: b[rng.randrange(len(b))] ^= rng.randrange(1,256)
            p = out/'fuzz.docx'; p.write_bytes(b)
            r = build_rs.run_exe(target,exe,['--check-docx',rel(p)],timeout=10)
            assert r.returncode in [0,1], (target,i,r.returncode,r.stdout,r.stderr)
            total += 1
        for script in sorted((ROOT/'apps/jfWord/tests').glob('*.uis')):
            invoke(*build_rs.script_args(script),'--script',rel(script),'--verify-dirty')
            if script.stem == 'core': rich_text.prepare(parts, zip_parts, ROOT)
            if script.stem == 'source_tools': merging.inspect_ui(ROOT)
            if script.stem == 'library': library.inspect_ui(ROOT)
            if script.stem == 'library_failures': library.inspect_recovery(ROOT)
            if script.stem == 'notes': notes.inspect_ui(ROOT)
            if script.stem == 'preservation': compatibility.inspect_ui(ROOT)
            if script.stem == 'stories': stories.inspect_ui(ROOT)
            if script.stem == 'paragraphs': stories.inspect_paragraphs(ROOT)
        rich_text.inspect(ROOT)
        clipboard.inspect(ROOT)
        for theme,scale in [('light','100'),('dark','100'),('light','150')]:
            invoke('--file',rel(sample),'--shot',rel(out/f'{theme}-{scale}.png'),'--theme',theme,'--scale',scale,'--verify-dirty')
        for view in ['home', 'editor']:
            for theme, scale, size in [('light','100','1000x700'),('light','100','1920x1080'),
                                       ('dark','150','1240x800'),('light','200','1000x700')]:
                extra = ['--editor'] if view == 'editor' else []
                invoke(*extra,'--shot',rel(out/f'{view}-{theme}-{scale}-{size}.png'),
                       '--size',size,'--theme',theme,'--scale',scale,'--verify-dirty')
        print(f'{target}: core, independent ZIP/OPC, rejection corpus, fuzz, UI and theme checks passed')
    print(f'{total} native process checks passed (plus library assertions and independent XML/ZIP assertions).')

if __name__ == '__main__': main()
