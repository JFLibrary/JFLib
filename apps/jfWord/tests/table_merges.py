"""Merged-cell workflows through real processes, with independently inspected DOCX."""
import argparse
import io
import re
import zipfile
import xml.etree.ElementTree as ET
from agent import ROOT, Session

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def parts(payload):
    with zipfile.ZipFile(io.BytesIO(payload)) as z:
        assert z.testzip() is None
        return {name: z.read(name) for name in z.namelist()}


def scenario(target, output, mcp):
    app = Session(target, output, mcp)
    folder = ROOT / output / 'table-merge-tests'
    folder.mkdir(parents=True, exist_ok=True)
    try:
        def edit(name, data=None): return app.invoke(name, data, app.state()['revision'])
        def table(): return app.invoke('document.table.inspect', {'table': tid})['value']
        def change(operation, **kw):
            return edit('document.table.change', dict(table=tid, operation=operation, **kw))
        def export(): return app.resource(app.invoke('document.export')['resources'][0]['uri'])
        def open_path(path):
            edit('document.new', {'discardChanges': True})
            edit('file.open', {'path': path.relative_to(ROOT).as_posix()})
        def save_parts(name, data):
            path = folder / f'{target}-{name}.docx'
            with zipfile.ZipFile(path, 'w', compression=zipfile.ZIP_DEFLATED) as z:
                for key, value in data.items(): z.writestr(key, value)
            return path

        edit('document.table.insert', dict(paragraph=1, offset=0, rows=3, columns=3))
        tid = app.state()['tables'][0]['id']
        original = table()
        for r, row in enumerate(original['rows']):
            for c, cell in enumerate(row['cells']):
                edit('document.text.replace', dict(paragraph=cell['paragraphs'][0], start=0, end=0, text=f'Zelle {r + 1}.{c + 1}'))
        change('cell', row=0, column=0, fill='C2E4F2', alignment='center')
        before = table()
        before_paragraphs = app.state()['paragraphs']
        change('merge', row=0, column=0, rows=2, columns=2)
        merged = table()
        owner = merged['rows'][0]['cells'][0]
        assert (owner['rowSpan'], owner['columnSpan']) == (2, 2)
        assert len(owner['paragraphs']) == 4 and owner['fill'] == 'C2E4F2'
        assert all(merged['rows'][r]['cells'][c]['covered'] for r, c in [(0, 1), (1, 0), (1, 1)])
        assert sum(not c['covered'] for r in merged['rows'] for c in r['cells']) == 6
        assert sorted(p['id'] for p in app.state()['paragraphs']) == sorted(p['id'] for p in before_paragraphs)
        edit('document.undo'); assert table() == before
        edit('document.redo'); assert table() == merged
        if not mcp:
            revision = app.state()['revision']
            for data in [dict(operation='merge', row=0, column=1, rows=2, columns=2),
                         dict(operation='merge', row=0, column=0, rows=4, columns=2),
                         dict(operation='split', row=2, column=2)]:
                app.invoke('document.table.change', dict(table=tid, **data), revision, error=True)
                assert app.state()['revision'] == revision and table() == merged
            app.invoke('document.table.fit', dict(table=tid, mode='page'), revision - 1, error=True)
            app.invoke('document.table.fit', dict(table=tid, mode='unknown'), revision, error=True)
            assert table() == merged and app.state()['revision'] == revision

        change('insertRow', index=1)
        assert table()['rows'][0]['cells'][0]['rowSpan'] == 3
        change('removeRow', index=0)
        assert table()['rows'][0]['cells'][0]['id'] == owner['id']
        assert table()['rows'][0]['cells'][0]['paragraphs'] == owner['paragraphs']
        change('insertColumn', index=1)
        assert table()['rows'][0]['cells'][0]['columnSpan'] == 3
        change('removeColumn', index=0)
        assert table()['rows'][0]['cells'][0]['id'] == owner['id']
        for _ in range(4): edit('document.undo')
        assert table() == merged

        # A split can address a covered coordinate. It keeps content in the owner.
        change('split', row=1, column=1)
        assert all(not c['covered'] for r in table()['rows'] for c in r['cells'])
        assert table()['rows'][0]['cells'][0]['paragraphs'] == owner['paragraphs']
        edit('document.undo'); assert table() == merged

        for mode in ['contents', 'page', 'equal']:
            previous = table()
            revision = app.state()['revision']
            edit('document.table.fit', dict(table=tid, mode=mode))
            widths = table()['table']['columns']
            changed = widths != previous['table']['columns']
            assert app.state()['revision'] == revision + int(changed)
            if mode == 'page':
                page = app.state()['page']
                assert sum(widths) == page['width'] - page['left'] - page['right']
            if mode == 'equal': assert max(widths) - min(widths) <= 1
            if changed: edit('document.undo')
            assert table() == previous

        path = folder / f'{target}-{"mcp" if mcp else "json"}.docx'
        edit('file.save', dict(path=path.relative_to(ROOT).as_posix(), overwrite=True))
        payload = export()
        expected = table(), app.state()['paragraphs']
        package = parts(payload)
        xml = ET.fromstring(package['word/document.xml'])
        rows = xml.find('.//' + W + 'tbl').findall(W + 'tr')
        assert [len(r.findall(W + 'tc')) for r in rows] == [2, 2, 3]
        assert len(xml.findall('.//' + W + 'gridSpan')) == 2
        assert all(n.attrib[W + 'val'] == '2' for n in xml.findall('.//' + W + 'gridSpan'))
        markers = xml.findall('.//' + W + 'vMerge')
        assert [n.get(W + 'val') for n in markers] == ['restart', None]
        text = ''.join(xml.itertext())
        for r in range(3):
            for c in range(3): assert text.count(f'Zelle {r + 1}.{c + 1}') == 1
        open_path(path)
        assert app.state()['mode'] == 'native' and (table(), app.state()['paragraphs']) == expected
        assert export() == payload

        # The public format alone must suffice, including explicit continuation syntax.
        if not mcp:
            standard = dict(package)
            standard.pop('customXml/jfTables.xml')
            standard['word/_rels/document.xml.rels'] = re.sub(rb'<Relationship Id="rId6"[^>]*/>', b'', standard['word/_rels/document.xml.rels'])
            standard['word/document.xml'] = re.sub(rb' w14:paraId="[^"]*"', b'', standard['word/document.xml'])
            standard['word/document.xml'] = standard['word/document.xml'].replace(b'<w:vMerge/>', b'<w:vMerge w:val="continue"/>')
            open_path(save_parts('standard', standard))
            assert app.state()['mode'] == 'native'
            tid = app.state()['tables'][0]['id']
            assert table()['rows'][0]['cells'][0]['rowSpan'] == 2
            assert len(table()['rows'][0]['cells'][0]['paragraphs']) == 4
            change('split', row=1, column=1)
            assert len(table()['rows'][1]['cells'][1]['paragraphs']) == 1
            for name, old, new in [
                ('orphan-continuation', b'<w:vMerge w:val="restart"/>', b'<w:vMerge w:val="continue"/>'),
                ('overflow-grid', b'<w:gridSpan w:val="2"/>', b'<w:gridSpan w:val="4"/>'),
                ('continuation-content', b'<w:p/>', b'<w:p><w:r><w:t>Must survive</w:t></w:r></w:p>'),
            ]:
                altered = dict(standard)
                assert old in altered['word/document.xml']
                altered['word/document.xml'] = altered['word/document.xml'].replace(old, new, 1)
                invalid = save_parts(name, altered)
                open_path(invalid)
                assert app.state()['mode'] == 'preserved', name
                assert export() == invalid.read_bytes(), name
        # Scientific fields remain structured, including through split and re-merge.
        open_path(path)
        tid = app.state()['tables'][0]['id']
        citation_paragraph = table()['rows'][0]['cells'][0]['paragraphs'][-1]
        source = edit('sources.put', dict(kind='book', title='Tabellarische Forschung', year='2026',
                      authors=[dict(given='Ada', family='Lovelace')]))['value']['id']
        edit('citations.insert', dict(paragraph=citation_paragraph, inline=0, offset=0,
                                     items=[dict(source=source, locator='42')]))
        change('split', row=1, column=1)
        change('merge', row=0, column=0, rows=2, columns=2)
        science_path = folder / f'{target}-{"mcp" if mcp else "json"}-science.docx'
        edit('file.save', dict(path=science_path.relative_to(ROOT).as_posix(), overwrite=True))
        science = export()
        fields = ET.fromstring(parts(science)['word/document.xml']).findall('.//' + W + 'fldSimple')
        assert len(fields) == 1 and fields[0].get(W + 'instr').strip().startswith('CITATION ')
        assert citation_paragraph in table()['rows'][0]['cells'][0]['paragraphs']
        open_path(science_path)
        assert export() == science and app.state()['totalSources'] == 1
        print(f'PASS {target} {"MCP" if mcp else "JSON"}: merge/split, covered cells, revision failures, structure, fit/history, citations, standard DOCX and preservation ({len(payload)} bytes)')
    finally:
        app.close()


def transfer(target, output):
    files = sorted((ROOT / output / 'table-merges-browser').glob('*.docx'))
    assert files, 'Run table_merges_browser.mjs first'
    for path in files:
        app = Session(target, output)
        try:
            app.invoke('file.open', dict(path=path.relative_to(ROOT).as_posix()), app.state()['revision'])
            assert app.state()['mode'] == 'native'
            assert app.resource(app.invoke('document.export')['resources'][0]['uri']) == path.read_bytes()
            print(f'PASS {target} <- {path.name}: exact merged DOCX')
        finally:
            app.close()


if __name__ == '__main__':
    p = argparse.ArgumentParser()
    p.add_argument('--target', choices=['win', 'linux', 'both'], default='both')
    p.add_argument('--out', default='build/wordcraft-merged')
    p.add_argument('--web-transfer', action='store_true')
    a = p.parse_args()
    for target in ['win', 'linux'] if a.target == 'both' else [a.target]:
        if a.web_transfer:
            transfer(target, a.out)
        else:
            for mcp in [False, True]: scenario(target, a.out, mcp)
