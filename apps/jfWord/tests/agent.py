"""Real process JSON/MCP tests of the existing editing, fields and file session."""
import argparse
import base64
import io
import json
import queue
import subprocess
import sys
import threading
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
import build_rs


class Session:
    def __init__(self, target, output, mcp=False, name='jfWord'):
        exe = ROOT / output / target / (name + ('.exe' if target == 'win' else ''))
        args = ['--agent-mcp' if mcp else '--agent-json', '--config', 'build/agent-tests/config.ini']
        command = [str(exe), *args] if target == 'win' else ['wsl.exe', '-d', 'Ubuntu-24.04', '--cd', build_rs.wsl_path(ROOT), '-e', build_rs.wsl_path(exe), *args]
        self.process = subprocess.Popen(command, cwd=ROOT, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.responses = queue.Queue()
        self.serial = 0
        self.mcp = mcp
        def read():
            for line in self.process.stdout:
                try:
                    self.responses.put(json.loads(line))
                except ValueError:
                    self.responses.put(AssertionError(f'Non-protocol stdout: {line!r}'))
        threading.Thread(target=read, daemon=True).start()
        if mcp:
            result = self.request('initialize', {'protocolVersion': '2025-11-25', 'capabilities': {}, 'clientInfo': {'name': 'jfWord-test', 'version': '1'}})
            assert result['protocolVersion'] == '2025-11-25', result
            self.notify('notifications/initialized', {})

    def notify(self, method, params):
        self.process.stdin.write((json.dumps({'jsonrpc': '2.0', 'method': method, 'params': params}, ensure_ascii=False) + '\n').encode())
        self.process.stdin.flush()

    def request(self, method, params, error=False):
        self.serial += 1
        self.process.stdin.write((json.dumps({'jsonrpc': '2.0', 'id': self.serial, 'method': method, 'params': params}, ensure_ascii=False) + '\n').encode())
        self.process.stdin.flush()
        try:
            response = self.responses.get(timeout=30)
        except queue.Empty:
            raise AssertionError(f'No response to {method}; process status {self.process.poll()}')
        if isinstance(response, Exception):
            raise response
        assert response.get('id') == self.serial, response
        if error:
            assert 'error' in response, response
            return response['error']
        assert 'error' not in response, response
        return response['result']

    def invoke(self, action, data=None, revision=None, error=False):
        params = {'action': action, 'input': data or {}}
        if revision is not None:
            params['expectedRevision'] = revision
        if self.mcp:
            result = self.request('tools/call', {'name': 'jfgui.actions.execute', 'arguments': params})
            if error:
                assert result.get('isError') and result['structuredContent']['ok'] is False, result
                return result['structuredContent']['error']
            assert not result.get('isError'), result
            return result['structuredContent']['data']
        return self.request('actions/execute', params, error=error)

    def state(self):
        return self.invoke('document.inspect')['value']

    def resource(self, uri):
        data, offset = bytearray(), 0
        while True:
            args = {'uri': uri, 'offset': offset, 'limit': 257}
            if self.mcp:
                reply = self.request('tools/call', {'name': 'jfgui.resources.read', 'arguments': args})
                assert not reply.get('isError'), reply
                part = reply['structuredContent']['data']
            else:
                part = self.request('resources/read', args)
            assert part['offset'] == offset and part['encoding'] == 'base64', part
            chunk = base64.b64decode(part['data'], validate=True)
            assert len(chunk) <= args['limit'] and part['nextOffset'] == offset + len(chunk), part
            data.extend(chunk)
            offset = part['nextOffset']
            if part['eof']:
                assert offset == part['resource']['size'], part
                return bytes(data)
            assert chunk, part

    def close(self):
        self.process.stdin.close()
        code = self.process.wait(timeout=30)
        stderr = self.process.stderr.read().decode(errors='replace')
        assert code == 0 and not stderr, (code, stderr)


def scenario(target, output, mcp):
    app = Session(target, output, mcp)
    try:
        if not mcp:
            catalogue = app.request('actions/list', {'limit': 100})
            assert any(a['name'] == 'sources.put' for a in catalogue['actions'])
            schema = app.request('actions/describe', {'action': 'document.text.replace'})
            assert schema['revisionRequired'] and schema['undo']
            rejected = app.invoke('document.text.replace', {'paragraph': 1, 'start': 0, 'end': 0, 'text': 'stale'}, 99, error=True)
            assert 'revision_conflict' in json.dumps(rejected), rejected
            app.invoke('document.text.replace', {'paragraph': 1, 'start': 0, 'end': 0, 'text': 'missing revision'}, error=True)
        state = app.state()
        revision = state['revision']
        text = 'Grüße aus der Forschung.\nZweiter Absatz.'
        result = app.invoke('document.text.replace', {'paragraph': 1, 'start': 0, 'end': 0, 'text': text}, revision)
        revision = result['revision']
        result = app.invoke('document.text.format', {'paragraph': 1, 'start': 0, 'end': len('Grüße'.encode()), 'format': {'bold': True}}, revision)
        revision = result['revision']
        source_range = {'paragraph': 1, 'start': 0, 'end': len('Grüße'.encode())}
        captured = app.invoke('document.text.format.inspect', source_range)['value']
        assert captured['bold'] and captured['sizeHalfPoints'] == 22, captured
        revision = app.invoke('document.text.format.reset', source_range, revision)['revision']
        assert app.state()['paragraphs'][0]['inlines'][0]['format']['bold'] is None
        revision = app.invoke('document.undo', revision=revision)['revision']
        target_range = {'paragraph': 1, 'start': len(text.split('\n')[0].encode()) + 1, 'end': len(text.encode())}
        app.invoke('document.select', source_range)
        app.invoke('ui.command', {'id': 80}, revision)
        app.invoke('document.select', target_range)
        revision = app.invoke('ui.command', {'id': 81}, revision)['revision']
        assert app.invoke('document.text.format.inspect', target_range)['value'] == captured
        revision = app.invoke('document.undo', revision=revision)['revision']
        # The semantic action and the normal GUI-command adapter produce the same format.
        revision = app.invoke('document.text.format.replace', {**target_range, 'format': captured}, revision)['revision']
        assert app.invoke('document.text.format.inspect', target_range)['value'] == captured
        revision = app.invoke('document.undo', revision=revision)['revision']
        result = app.invoke('sources.put', {'kind': 'book', 'title': 'Methoden & Belege', 'year': '2026', 'authors': [{'given': 'Ada', 'family': 'Lovelace'}]}, revision)
        revision, source = result['revision'], result['value']['id']
        state = app.state()
        first = state['paragraphs'][0]
        # The bold prefix is its own run; citation insertion is at that editable run's end.
        result = app.invoke('citations.insert', {'paragraph': 1, 'inline': 0, 'offset': len('Grüße'.encode()), 'items': [{'source': source, 'locator': '42'}]}, revision)
        revision = result['revision']
        result = app.invoke('notes.insert', {'paragraph': 1, 'inline': 0, 'offset': 0, 'kind': 'footnote', 'text': 'Ein wissenschaftlicher Hinweis.'}, revision)
        revision = result['revision']
        final_paragraph = app.state()['paragraphs'][1]['id']
        result = app.invoke('bibliography.insert', {'id': final_paragraph}, revision)
        revision = result['revision']
        state = app.state()
        assert state['totalSources'] == 1 and state['totalNotes'] == 1, state
        assert 'Lovelace' in state['paragraphs'][0]['text'], state
        before_undo = state['totalParagraphs']
        revision = app.invoke('document.undo', revision=revision)['revision']
        assert app.state()['totalParagraphs'] == before_undo - 1
        revision = app.invoke('document.redo', revision=revision)['revision']
        if not mcp:
            app.request('actions/batch', {'atomic': True, 'calls': [{'action': 'document.paragraph.add', 'input': {'id': 1}, 'expectedRevision': revision}]}, error=True)
            assert app.state()['revision'] == revision
            app.invoke('sources.put', {'title': 'Unexpected', 'unknown': 1}, revision, error=True)
            assert app.state()['revision'] == revision
        dest = f'build/agent-tests/{target}-{"mcp" if mcp else "json"}.docx'
        unsaved_revision = revision
        exported = app.invoke('document.export')
        assert exported['revision'] == revision and app.state()['dirty'], exported
        resource = exported['resources'][0]
        assert resource['mimeType'] == 'application/vnd.openxmlformats-officedocument.wordprocessingml.document', resource
        exported_bytes = app.resource(resource['uri'])
        with zipfile.ZipFile(io.BytesIO(exported_bytes)) as package:
            assert package.testzip() is None and 'word/footnotes.xml' in package.namelist()
        revision = app.invoke('file.save', {'path': dest, 'overwrite': True}, revision)['revision']
        assert revision != unsaved_revision and not app.state()['dirty']
        after_save = app.invoke('document.inspect', {'sinceRevision': unsaved_revision})['value']
        assert not after_save.get('unchanged') and after_save['path'] == dest and not after_save['dirty'], after_save
        assert (ROOT / dest).read_bytes() == exported_bytes
        revision = app.invoke('file.open', {'path': dest}, revision)['revision']
        state = app.state()
        assert state['totalSources'] == 1 and state['totalNotes'] == 1 and not state['dirty'], state
        with zipfile.ZipFile(ROOT / dest) as package:
            assert package.testzip() is None
            assert 'word/footnotes.xml' in package.namelist()
        image = app.invoke('ui.render')
        assert image['resources'][0]['mimeType'] == 'image/png', image
        print(f'{target} {"MCP" if mcp else "JSON"}: editing, formatting, sources, citations, notes, undo, save revisions, DOCX resource chunks and render passed', flush=True)
    finally:
        app.close()


if __name__ == '__main__':
    ap = argparse.ArgumentParser()
    ap.add_argument('--target', choices=['win', 'linux', 'both'], default='both')
    ap.add_argument('--build-dir', default='build/office-universal')
    args = ap.parse_args()
    (ROOT / 'build/agent-tests').mkdir(parents=True, exist_ok=True)
    for target in (['win', 'linux'] if args.target == 'both' else [args.target]):
        for mcp in (False, True):
            scenario(target, args.build_dir, mcp)
