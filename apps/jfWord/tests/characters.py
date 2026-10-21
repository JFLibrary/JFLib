"""Character properties through real JSON/MCP processes, file sessions and browser transfer."""
import argparse
import io
import zipfile
import xml.etree.ElementTree as ET
from agent import ROOT, Session

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'


def scenario(target, output, mcp):
    app = Session(target, output, mcp)
    try:
        def edit(name, data=None):
            return app.invoke(name, data, app.state()['revision'])

        def props(start, end, format):
            return edit('document.text.format', {'paragraph': 1, 'start': start, 'end': end, 'format': format})

        def inspect(start, end, summary=False):
            return app.invoke('document.text.format.summary' if summary else 'document.text.format.inspect',
                              {'paragraph': 1, 'start': start, 'end': end})['value']

        edit('document.text.replace', {'paragraph': 1, 'start': 0, 'end': 0, 'text': 'H2O + x2\nWeiter α'})
        parent = {'id': 'jfStyle1', 'name': 'Formeln', 'basedOn': 'Normal',
                  'format': {'fontFamily': 'DejaVu Serif', 'sizeHalfPoints': 28, 'highlight': 'none', 'positioning': 'baseline'}}
        edit('document.styles.put', parent)
        edit('document.paragraph.style', {'paragraph': 1, 'style': 'jfStyle1'})
        props(1, 2, {'positioning': 'subscript', 'highlight': 'yellow'})
        props(7, 8, {'positioning': 'superscript', 'highlight': 'cyan'})
        value = inspect(1, 2)
        assert value['positioning'] == 'subscript' and value['sizeHalfPoints'] == 28
        assert inspect(0, 8, True)['positioning'] is None and inspect(0, 8, True)['highlight'] is None
        props(0, 8, {'positioning': 'superscript'})
        assert inspect(0, 8, True)['positioning'] == 'superscript' and inspect(0, 8, True)['highlight'] is None
        edit('document.undo')
        assert inspect(1, 2) == value
        parent['format']['sizeHalfPoints'] = 36
        edit('document.styles.put', parent)
        assert inspect(1, 2)['positioning'] == 'subscript' and inspect(1, 2)['sizeHalfPoints'] == 36
        child = {'id': 'jfStyle2', 'name': 'Vererbt', 'basedOn': 'jfStyle1',
                 'format': {'positioning': 'superscript', 'highlight': 'green'}}
        edit('document.styles.put', child)
        second = app.state()['paragraphs'][1]['id']
        edit('document.paragraph.style', {'paragraph': second, 'style': 'jfStyle2'})
        # Range uses UTF-8 story bytes, with a paragraph mark between body paragraphs.
        full = {'paragraph': 1, 'start': 0, 'end': len('H2O + x2\nWeiter α'.encode())}
        edit('document.text.format', {**full, 'format': {'highlight': 'magenta'}})
        assert inspect(0, full['end'], True)['highlight'] == 'magenta'
        edit('document.undo')
        props(9, 10, {'positioning': 'baseline', 'highlight': 'none'})
        assert inspect(9, 10)['positioning'] == 'baseline' and inspect(10, 11)['positioning'] == 'superscript'
        if not mcp:
            revision = app.state()['revision']
            for wrong in ({'highlight': '#ffff00'}, {'positioning': 'both'}, {'highlight': 123}):
                app.invoke('document.text.format', {**full, 'format': wrong}, revision, error=True)
            app.invoke('document.text.format', {**full, 'format': {'positioning': 'baseline'}}, revision-1, error=True)
            assert app.state()['revision'] == revision
        exported = app.invoke('document.export')
        data = app.resource(exported['resources'][0]['uri'])
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            tree = ET.fromstring(archive.read('word/document.xml'))
            runs = {r.find(W+'t').text: r for r in tree.findall('.//'+W+'r') if r.find(W+'t') is not None}
            assert runs['W'].find(W+'rPr/'+W+'highlight').attrib[W+'val'] == 'none'
            assert runs['W'].find(W+'rPr/'+W+'vertAlign').attrib[W+'val'] == 'baseline'
            styles = ET.fromstring(archive.read('word/styles.xml'))
            s = next(s for s in styles.findall(W+'style') if s.attrib[W+'styleId'] == 'jfStyle2')
            assert s.find(W+'rPr/'+W+'vertAlign').attrib[W+'val'] == 'superscript'
        folder = ROOT/output/'character-tests'
        folder.mkdir(parents=True, exist_ok=True)
        path = (folder/f'{target}-{"mcp" if mcp else "json"}.docx').relative_to(ROOT).as_posix()
        edit('file.save', {'path': path, 'overwrite': True})
        expected = app.state()['paragraphs']
        edit('document.new', {'discardChanges': True})
        edit('file.open', {'path': path})
        assert app.state()['paragraphs'] == expected
        assert app.resource(app.invoke('document.export')['resources'][0]['uri']) == data
        # Reset restores the child style, undo restores explicit clearing.
        edit('document.text.format.reset', {'paragraph': 1, 'start': 9, 'end': 10})
        assert inspect(9, 10)['highlight'] == 'green' and inspect(9, 10)['positioning'] == 'superscript'
        edit('document.undo')
        assert inspect(9, 10)['highlight'] == 'none'
        print(f'PASS {target} {"MCP" if mcp else "JSON"}: scripts, palette, styles, mixed ranges, undo and DOCX ({len(data)} bytes)')
    finally:
        app.close()


def web_transfer(target, output):
    for engine in ('chromium', 'firefox'):
        for theme in ('light', 'dark'):
            path = ROOT/output/'character-browser'/f'{engine}-{theme}.docx'
            data = path.read_bytes()
            app = Session(target, output)
            try:
                app.invoke('file.open', {'path': path.relative_to(ROOT).as_posix()}, app.state()['revision'])
                assert app.resource(app.invoke('document.export')['resources'][0]['uri']) == data
                f = app.invoke('document.text.format.inspect', {'paragraph': 1, 'start': 1, 'end': 2})['value']
                assert f['positioning'] == 'subscript' and f['highlight'] == 'yellow'
                print(f'PASS {target} <- {engine}/{theme}: exact {len(data)} DOCX bytes')
            finally:
                app.close()


if __name__ == '__main__':
    p = argparse.ArgumentParser()
    p.add_argument('--target', choices=['win', 'linux', 'both'], default='both')
    p.add_argument('--out', default='build/wordcraft-character')
    p.add_argument('--web-transfer', action='store_true')
    args = p.parse_args()
    for target in ['win', 'linux'] if args.target == 'both' else [args.target]:
        if args.web_transfer:
            web_transfer(target, args.out)
        else:
            for mcp in (False, True):
                scenario(target, args.out, mcp)
