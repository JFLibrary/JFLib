"""Tab inheritance, revisions, paragraph ranges and native/web DOCX interchange."""
import argparse
from agent import ROOT, Session


def scenario(target, output, mcp):
    app = Session(target, output, mcp)
    try:
        def edit(name, data=None):
            return app.invoke(name, data, app.state()['revision'])
        def props():
            return [p['effectiveParagraphFormat'] for p in app.state()['paragraphs']]
        def stop(pos, align='left', leader='none'):
            return dict(position=pos, alignment=align, leader=leader)
        edit('document.text.replace', dict(paragraph=1, start=0, end=0, text='Ware\t12,50\nSumme\t123,45\nEnde'))
        base = dict(id='jfStyle1', name='Beträge', basedOn='Normal', paragraph={'tabs': [stop(2880, 'decimal', 'dot')]})
        edit('document.styles.put', base)
        edit('document.styles.put', dict(id='jfStyle2', name='Zwischensumme', basedOn='jfStyle1', paragraph={'tabs': [stop(1440, 'center')]}))
        for p in app.state()['paragraphs'][:2]:
            edit('document.paragraph.style', dict(paragraph=p['id'], style='jfStyle2'))
        assert [t['position'] for t in props()[0]['tabs']] == [1440, 2880]
        selection = dict(paragraph=1, start=0, end=len('Ware\t12,50\nSumme\t123,45'.encode()))
        tabs = [stop(3600, 'decimal', 'dot'), stop(5000, 'bar')]
        edit('document.paragraph.tabs', {**selection, 'tabs': tabs})
        assert props()[0]['tabs'] == tabs and props()[1]['tabs'] == tabs and props()[2]['tabs'] == []
        assert any(t['alignment'] == 'clear' for t in app.state()['paragraphs'][0]['paragraphFormat']['tabs'])
        edit('document.undo')
        assert len(props()[1]['tabs']) == 2
        edit('document.redo')
        assert props()[1]['tabs'] == tabs
        edit('document.paragraph.format', {**selection, 'format': {'left': 720, 'firstLine': -360}})
        assert props()[0]['left'] == 720 and props()[0]['tabs'] == tabs
        edit('document.paragraph.tabs', {**selection, 'tabs': []})
        assert props()[0]['tabs'] == props()[1]['tabs'] == []
        # Editing the parent at a cleared position must not resurrect that position.
        base['paragraph']['tabs'][0]['leader'] = 'hyphen'
        edit('document.styles.put', base)
        assert props()[0]['tabs'] == []
        edit('document.paragraph.tabs', {**selection, 'tabs': tabs})
        revision = app.state()['revision']
        if not mcp:
            for wrong in ([stop(100, 'bar', 'dot')], [stop(100), stop(100)], [stop(32000)], [stop(100, 'clear')]):
                app.invoke('document.paragraph.tabs', {**selection, 'tabs': wrong}, revision, error=True)
            app.invoke('document.paragraph.tabs', {**selection, 'tabs': []}, revision - 1, error=True)
            assert app.state()['revision'] == revision
        page = app.state()['page']; page['left'] = 1440
        edit('document.page.set', page)
        assert app.state()['page'] == page
        expected = app.state()['paragraphs']
        folder = ROOT / output / 'tabs-tests'; folder.mkdir(parents=True, exist_ok=True)
        path = (folder / f'{target}-{"mcp" if mcp else "json"}.docx').relative_to(ROOT).as_posix()
        edit('file.save', dict(path=path, overwrite=True))
        data = app.resource(app.invoke('document.export')['resources'][0]['uri'])
        edit('document.new', dict(discardChanges=True))
        edit('file.open', dict(path=path))
        assert app.state()['paragraphs'] == expected
        assert app.resource(app.invoke('document.export')['resources'][0]['uri']) == data
        print(f'PASS {target} {"MCP" if mcp else "JSON"}: tabs, inheritance, ranges, undo, revisions and DOCX ({len(data)} bytes)')
    finally:
        app.close()


def transfer(target, output):
    for path in (ROOT / output / 'tabs-browser').glob('*.docx'):
        app = Session(target, output)
        try:
            data = path.read_bytes()
            app.invoke('file.open', dict(path=path.relative_to(ROOT).as_posix()), app.state()['revision'])
            assert app.resource(app.invoke('document.export')['resources'][0]['uri']) == data
            assert app.state()['paragraphs'][0]['effectiveParagraphFormat']['tabs']
            print(f'PASS {target} <- {path.name}: exact {len(data)} DOCX bytes')
        finally:
            app.close()


if __name__ == '__main__':
    p = argparse.ArgumentParser()
    p.add_argument('--target', choices=['win', 'linux', 'both'], default='both')
    p.add_argument('--out', default='build/wordcraft-tabs')
    p.add_argument('--web-transfer', action='store_true')
    args = p.parse_args()
    for target in ['win', 'linux'] if args.target == 'both' else [args.target]:
        if args.web_transfer:
            transfer(target, args.out)
        else:
            for mcp in (False, True):
                scenario(target, args.out, mcp)
