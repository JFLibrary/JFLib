"""Header/style workflows via native JSON and MCP; independent OOXML assertions."""
import argparse
import io
import re
import zipfile
import xml.etree.ElementTree as ET
from agent import ROOT, Session

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def package(data):
    with zipfile.ZipFile(io.BytesIO(data)) as z:
        assert z.testzip() is None
        return {n: z.read(n) for n in z.namelist()}


def scenario(target, output, mcp):
    app = Session(target, output, mcp)
    folder = ROOT / output / 'table-style-tests'
    folder.mkdir(parents=True, exist_ok=True)
    try:
        def edit(name, data=None): return app.invoke(name, data, app.state()['revision'])
        def table(): return app.invoke('document.table.inspect', {'table': tid, 'limit': 32})['value']
        def change(operation, **kw): return edit('document.table.change', dict(table=tid, operation=operation, **kw))
        def export(): return app.resource(app.invoke('document.export')['resources'][0]['uri'])
        def layout(): return app.invoke('document.table.layout', {'table': tid, 'limit': 8})['value']
        def open_path(path):
            edit('document.new', {'discardChanges': True})
            edit('file.open', {'path': path.relative_to(ROOT).as_posix()})
        def patched(name, data):
            path = folder / f'{target}-{name}.docx'
            with zipfile.ZipFile(path, 'w', compression=zipfile.ZIP_DEFLATED) as z:
                for key, value in data.items(): z.writestr(key, value)
            open_path(path)

        page = app.state()['page']
        page.update(height=7200, top=720, bottom=720)
        edit('document.page.set', page)
        edit('document.table.insert', dict(paragraph=1, offset=0, rows=22, columns=3))
        tid = app.state()['tables'][0]['id']
        for r, row in enumerate(table()['rows']):
            for c, cell in enumerate(row['cells']):
                edit('document.text.replace', dict(paragraph=cell['paragraphs'][0], start=0, end=0,
                    text=['Messreihe', 'Messwert', 'Einheit'][c] if r == 0 else f'Wert {r}.{c + 1}'))
            change('rowHeight', row=r, height=500)
        first = table()['rows'][0]['cells'][0]['paragraphs'][0]
        styles = app.invoke('document.table.styles.list')['value']['styles']
        assert len(styles) == 4 and styles[0]['id'] == 'jfTableBlue'
        assert app.invoke('document.table.styles.list', dict(offset=2, limit=1))['value']['nextOffset'] == 3
        change('headerRows', headerRows=2)
        change('style', style='jfTableBlue')
        state = table()
        assert state['rows'][0]['cells'][0]['effectiveFill'] == '196180'
        assert state['rows'][1]['cells'][0]['effectiveFill'] == '196180'
        assert state['rows'][2]['cells'][0]['effectiveFill'] != state['rows'][3]['cells'][0]['effectiveFill']
        fmt = app.invoke('document.text.format.inspect', dict(paragraph=first, start=0, end=1))['value']
        assert fmt['bold'] and fmt['color'] == 0xffffff, fmt
        pages = layout()['pages']
        assert len([p for p in pages if p['cells']]) >= 3
        for i, p in enumerate(pages):
            if not p['cells']: continue
            headers = [c for c in p['cells'] if c['row'] < 2]
            assert len(headers) == 6 and all(c['repeated'] == (i > 0) for c in headers)
            assert all(c['bounds'][1] >= max(h['bounds'][1] + h['bounds'][3] for h in headers) for c in p['cells'] if c['row'] >= 2)
        assert sum(not c['repeated'] for p in pages for c in p['cells']) == 66
        change('merge', row=0, column=0, rows=2, columns=2)
        assert all(len([c for c in p['cells'] if c['row'] < 2]) == 3 for p in layout()['pages'] if p['cells'])
        before = table()
        put = dict(id='ResearchTable', name='Messreihe · Grün', cloneFrom='jfTableTeal', applyTo=tid,
                   firstRow={'fill': '285942', 'format': {'bold': True, 'color': 0xffffff}}, band1={'fill': 'DAEDE3'})
        edit('document.table.styles.put', put)
        custom = table()
        assert custom['table']['style'] == 'ResearchTable' and custom['rows'][0]['cells'][0]['effectiveFill'] == '285942'
        edit('document.undo'); assert table() == before
        edit('document.redo'); assert table() == custom
        change('cell', row=0, column=0, fill='FFFFFF', alignment='top')
        edit('document.table.styles.put', dict(id='ResearchTable', name='Messreihe · Grün', firstRow={'fill': '345A68'}))
        assert table()['rows'][0]['cells'][0]['effectiveFill'] == 'FFFFFF'
        assert table()['rows'][0]['cells'][2]['effectiveFill'] == '345A68'
        change('cell', row=0, column=0, fill='', alignment='top')
        change('look', look={'bandedRows': False})
        assert table()['rows'][2]['cells'][0]['effectiveFill'] == table()['rows'][3]['cells'][0]['effectiveFill']
        edit('document.undo')
        if not mcp:
            snapshot, revision = table(), app.state()['revision']
            for data in [dict(operation='headerRows', headerRows=1), dict(operation='headerRows', headerRows=23),
                         dict(operation='style', style='unknown'), dict(operation='merge', row=1, column=2, rows=2, columns=1)]:
                app.invoke('document.table.change', dict(table=tid, **data), revision, error=True)
                assert app.state()['revision'] == revision and table() == snapshot
            app.invoke('document.table.styles.put', dict(id='BadStyle', name='bad', body={'fill': 'GGGGGG'}), revision, error=True)
            app.invoke('document.table.change', dict(table=tid, operation='headerRows', headerRows=0), revision - 1, error=True)
            assert table() == snapshot
        path = folder / f'{target}-{"mcp" if mcp else "json"}.docx'
        edit('file.save', dict(path=path.relative_to(ROOT).as_posix(), overwrite=True))
        payload = export(); parts = package(payload)
        xml = ET.fromstring(parts['word/document.xml']); sx = ET.fromstring(parts['word/styles.xml'])
        rows = xml.find('.//' + W + 'tbl').findall(W + 'tr')
        assert [r.find(W + 'trPr/' + W + 'tblHeader') is not None for r in rows] == [True, True] + [False] * 20
        assert xml.find('.//' + W + 'tblStyle').get(W + 'val') == 'ResearchTable'
        definitions = [s for s in sx.findall(W + 'style') if s.get(W + 'type') == 'table']
        assert len(definitions) == 1 and definitions[0].get(W + 'styleId') == 'ResearchTable'
        assert definitions[0].find(W + 'tblStylePr').get(W + 'type') == 'band1Horz'
        assert ''.join(xml.itertext()).count('Messreihe') == 1
        snapshot = table(); open_path(path)
        assert app.state()['mode'] == 'native' and table() == snapshot and export() == payload
        if not mcp:
            standard = dict(parts)
            standard.pop('customXml/jfTables.xml')
            standard['word/_rels/document.xml.rels'] = re.sub(rb'<Relationship Id="rId6"[^>]*/>', b'', standard['word/_rels/document.xml.rels'])
            standard['word/document.xml'] = re.sub(rb' w14:paraId="[^"]*"', b'', standard['word/document.xml'])
            standard['word/document.xml'] = standard['word/document.xml'].replace(b'<w:tblHeader/>', b'<w:tblHeader w:val="true"/>')
            patched('standard-headers', standard)
            assert app.state()['mode'] == 'native'
            tid = app.state()['tables'][0]['id']
            assert table()['table']['headerRows'] == 2 and table()['table']['style'] == 'ResearchTable'
            # Unsupported conditional content must stay losslessly preserved.
            opaque = dict(parts)
            opaque['word/styles.xml'] = opaque['word/styles.xml'].replace(b'<w:tblStylePr w:type="band1Horz">', b'<w:tblStylePr w:type="wholeTable">')
            patched('unsupported-conditional', opaque)
            assert app.state()['mode'] == 'preserved'
            assert package(export())['word/styles.xml'] == opaque['word/styles.xml']
        print(f'{target} {"MCP" if mcp else "JSON"}: header pagination, template edits/undo/precedence, merged heads, standard DOCX and preservation passed')
    finally:
        app.close()


def transfer(target, output):
    app = Session(target, output)
    try:
        for path in sorted((ROOT / output / 'table-styles-browser').glob('*.docx')):
            app.invoke('file.open', dict(path=path.relative_to(ROOT).as_posix(), discardChanges=True), app.state()['revision'])
            assert app.state()['mode'] == 'native'
            payload = app.resource(app.invoke('document.export')['resources'][0]['uri'])
            assert payload == path.read_bytes(), path
            assert app.state()['tables'][0]['headerRows'] == 2
            print(f'{target}: browser header/template roundtrip {path.name}')
    finally: app.close()


if __name__ == '__main__':
    p = argparse.ArgumentParser(); p.add_argument('--target', choices=['win', 'linux', 'both'], default='both')
    p.add_argument('--out', default='build/wordcraft-table-styles'); p.add_argument('--web-transfer', action='store_true'); a = p.parse_args()
    for target in (['win', 'linux'] if a.target == 'both' else [a.target]):
        for mcp in [False, True]: scenario(target, a.out, mcp)
        if a.web_transfer: transfer(target, a.out)
