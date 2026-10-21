"""The same transport drives actual graphics operations and the ordinary undo history."""
import argparse
import importlib.util
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('word_agent_test', ROOT / 'apps/jfWord/tests/agent.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def scenario(target, output, mcp):
    app = module.Session(target, output, mcp, name='Kritzel')
    def state():
        return app.invoke('image.inspect')['value']
    def edit(action, data=None):
        return app.invoke(action, data, state()['revision'])
    try:
        initial = state()
        if not mcp:
            err = app.invoke('image.layer.remove', {}, initial['revision'], error=True)
            assert 'unavailable' in json.dumps(err)
            app.invoke('image.layer.fill', {'argb': 0xff123456}, initial['revision'] + 99, error=True)
        edit('image.layer.fill', {'argb': 0xff123456})
        assert app.invoke('image.pixel', {'x': 0, 'y': 0})['value']['argb'] == 0xff123456
        edit('image.layer.add')
        edit('image.layer.fill', {'argb': 0xffc02030})
        assert state()['totalLayers'] == 2
        edit('image.layer.duplicate')
        assert state()['totalLayers'] == 3
        edit('image.layer.remove')
        edit('image.resize', {'width': 37, 'height': 23})
        edit('image.rotate.right')
        s = state()
        assert (s['width'], s['height']) == (23, 37), s
        assert app.invoke('image.pixel', {'x': 22, 'y': 36})['value']['argb'] == 0xffc02030
        edit('image.undo')
        s = state()
        assert (s['width'], s['height']) == (37, 23), s
        edit('image.redo')
        edit('image.layer.merge')
        assert state()['totalLayers'] == 1
        render = app.invoke('ui.render')
        assert render['resources'][0]['mimeType'] == 'image/png'
        print(f'{target} {"MCP" if mcp else "JSON"}: graphics layers/pixels/resize/rotate/history/resource passed')
    finally:
        app.close()


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--target', choices=['win', 'linux', 'both'], default='both')
    parser.add_argument('--build-dir', default='build/office-universal')
    args = parser.parse_args()
    for target in ['win', 'linux'] if args.target == 'both' else [args.target]:
        for mcp in [False, True]:
            scenario(target, args.build_dir, mcp)
