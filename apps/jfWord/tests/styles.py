"""Named-style/font workflows through the real JSON and MCP transports and DOCX."""
import argparse
import io
import json
from pathlib import Path
import zipfile
import xml.etree.ElementTree as ET
from agent import ROOT, Session

W = '{http://schemas.openxmlformats.org/wordprocessingml/2006/main}'

def scenario(target, output, mcp):
    app = Session(target, output, mcp)
    try:
        def edit(action, data):
            return app.invoke(action, data, app.state()['revision'])
        edit('document.text.replace', {'paragraph': 1, 'start': 0, 'end': 0, 'text': 'Wissenschaft αβ'})
        parent = {'id': 'jfStyle1', 'name': 'Wissenschaft', 'basedOn': 'Normal',
                  'format': {'fontFamily': 'DejaVu Serif', 'sizeHalfPoints': 28}, 'paragraph': {'after': 240, 'left': 360}}
        edit('document.styles.put', parent)
        child = {'id': 'jfStyle2', 'name': 'Forschungsfrage', 'basedOn': 'jfStyle1', 'format': {'bold': True}, 'paragraph': {'firstLine': -180}, 'outlineLevel': 1}
        edit('document.styles.put', child)
        edit('document.paragraph.style', {'paragraph': 1, 'style': 'jfStyle2'})
        selected = {'paragraph': 1, 'start': 0, 'end': len('Wissenschaft αβ'.encode())}
        def effective(): return app.invoke('document.text.format.inspect', selected)['value']
        assert effective()['fontFamily'] == 'DejaVu Serif' and effective()['bold']
        edit('document.text.format', {**selected, 'format': {'sizeHalfPoints': 31}})
        parent['format'] = {'fontFamily': 'DejaVu Sans Mono', 'sizeHalfPoints': 36, 'italic': True}
        edit('document.styles.put', parent)
        value = effective()
        assert value['sizeHalfPoints'] == 31 and value['fontFamily'] == 'DejaVu Sans Mono' and value['italic']
        before = app.state()['revision']
        if not mcp:
            error = app.invoke('document.styles.put', {**parent, 'basedOn': 'jfStyle2'}, before, error=True)
            assert error and app.state()['revision'] == before
            app.invoke('document.text.format', {**selected, 'format': {'fontFamily': 'bad\nname'}}, before, error=True)
            app.invoke('document.text.format', {**selected, 'format': {'fontFamily': 'bad\uffffname'}}, before, error=True)
            app.invoke('document.styles.put', {**parent, 'name': 'bad\ufffename'}, before, error=True)
            app.invoke('document.paragraph.style', {'paragraph': 1, 'style': 'jfStyle99'}, before, error=True)
            app.invoke('document.styles.put', parent, before - 1, error=True)
            assert app.state()['revision'] == before
        definitions = app.invoke('document.styles.list', {'offset': 4, 'limit': 1})['value']
        assert definitions['total'] == 6 and definitions['nextOffset'] == 5 and len(definitions['styles']) == 1
        exported = app.invoke('document.export')
        data = app.resource(exported['resources'][0]['uri'])
        with zipfile.ZipFile(io.BytesIO(data)) as archive:
            styles = ET.fromstring(archive.read('word/styles.xml'))
            nodes = {s.attrib[W + 'styleId']: s for s in styles.findall(W + 'style')}
            assert nodes['jfStyle2'].find(W+'basedOn').attrib[W+'val'] == 'jfStyle1'
            assert nodes['jfStyle2'].find('.//'+W+'ind').attrib[W+'hanging'] == '180'
            assert nodes['jfStyle1'].find('.//'+W+'rFonts').attrib[W+'ascii'] == 'DejaVu Sans Mono'
            document = ET.fromstring(archive.read('word/document.xml'))
            assert document.find('.//'+W+'pStyle').attrib[W+'val'] == 'jfStyle2'
        folder = ROOT/'build/style-checks'
        folder.mkdir(exist_ok=True)
        path = f'build/style-checks/{target}-{"mcp" if mcp else "json"}.docx'
        edit('file.save', {'path': path, 'overwrite': True})
        edit('document.new', {'discardChanges': True})
        edit('file.open', {'path': path})
        assert effective() == value and app.state()['paragraphs'][0]['style'] == 'jfStyle2'
        edit('document.text.format.reset', selected)
        assert effective()['sizeHalfPoints'] == 36
        edit('document.undo', {})
        assert effective() == value
        print(f'{target} {"MCP" if mcp else "JSON"}: inheritance, overrides, revisions, export and reopen passed')
    finally:
        app.close()

def web_transfer(target, output):
    folder = (ROOT / output / 'style-browser').resolve()
    for engine in ('chromium', 'firefox'):
        for theme in ('light', 'dark'):
            path = folder / f'{engine}-{theme}.docx'
            data = path.read_bytes()
            app = Session(target, output)
            try:
                # A workspace-relative path works for both Windows and WSL.
                app.invoke('file.open', {'path': path.relative_to(ROOT).as_posix()}, app.state()['revision'])
                styles = app.invoke('document.styles.list')['value']['styles']
                child = next(s for s in styles if s['id'] == 'jfStyle2')
                assert child['name'] == 'Forschungsfrage Web' and child['basedOn'] == 'jfStyle1'
                assert child['effectiveFormat']['fontFamily'] == 'DejaVu Serif'
                assert child['effectiveFormat']['sizeHalfPoints'] == 34 and child['effectiveFormat']['bold']
                assert child['paragraph']['firstLine'] == -180
                exported = app.invoke('document.export')
                assert app.resource(exported['resources'][0]['uri']) == data
                print(f'{target} <- {engine}/{theme}: styles, font and exact {len(data)} DOCX bytes')
            finally:
                app.close()

if __name__ == '__main__':
    p = argparse.ArgumentParser()
    p.add_argument('--target', choices=['win', 'linux', 'both'], default='both')
    p.add_argument('--out', default='build/wordcraft-styles')
    p.add_argument('--web-transfer', action='store_true', help='Verify saved styles_browser.mjs files in native apps')
    args = p.parse_args()
    for target in ['win', 'linux'] if args.target == 'both' else [args.target]:
        if args.web_transfer:
            web_transfer(target, args.out)
        else:
            for mcp in [False, True]: scenario(target, args.out, mcp)
