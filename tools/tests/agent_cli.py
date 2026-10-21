"""End-to-end UTF-8 editing through the CLI's redirected headless JSON/MCP streams.

Build the native jfWord app first. Only Python's standard library is required.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from tools.host_exec import linux_path


def scenario(target, output, protocol):
    exe = output / target / ('jfWord.exe' if target == 'win' else 'jfWord')
    common = ['--protocol', protocol, '--', '--config',
              (output / 'cli-tests/config.ini').relative_to(ROOT).as_posix()]
    if target == 'win':
        command = [sys.executable, '-X', 'utf8', '-B', str(ROOT / 'tools/jfgui-agent.py'),
                   'headless', str(exe), *common]
    else:
        command = ['python3', str(ROOT / 'tools/jfgui-agent.py'), 'headless', str(exe), *common]
        if os.name == 'nt':
            command = ['wsl.exe', '-d', os.environ.get('jfgui_WSL_DISTRO', 'Ubuntu-24.04'),
                       '--cd', linux_path(ROOT), '-e', 'python3',
                       linux_path(ROOT / 'tools/jfgui-agent.py'), 'headless', linux_path(exe), *common]
    text = 'CLI Grüße • العربية 🧪'
    edit = {'action': 'document.text.replace', 'expectedRevision': 0,
            'input': {'paragraph': 1, 'start': 0, 'end': 0, 'text': text}}
    inspect = {'domain': 'document', 'limit': 20}
    if protocol == 'json':
        requests = [('agent/info', {}), ('actions/list', {'query': 'formulas', 'limit': 10}),
                    ('actions/execute', edit), ('state/inspect', inspect)]
        messages = [dict(jsonrpc='2.0', id=i + 1, method=m, params=p)
                    for i, (m, p) in enumerate(requests)]
    else:
        messages = [
            dict(jsonrpc='2.0', id=1, method='initialize', params={
                'protocolVersion': '2025-11-25', 'capabilities': {},
                'clientInfo': {'name': 'jfgui-cli-check', 'version': '1'}}),
            dict(jsonrpc='2.0', method='notifications/initialized', params={}),
            dict(jsonrpc='2.0', id=2, method='tools/list', params={}),
            dict(jsonrpc='2.0', id=3, method='tools/call', params={
                'name': 'jfgui.actions.execute', 'arguments': edit}),
            dict(jsonrpc='2.0', id=4, method='tools/call', params={
                'name': 'jfgui.state.inspect', 'arguments': inspect}),
        ]
    data = ''.join(json.dumps(m, ensure_ascii=False) + '\n' for m in messages).encode('utf-8')
    result = subprocess.run(command, input=data, capture_output=True, cwd=ROOT, timeout=40,
                            creationflags=subprocess.CREATE_NO_WINDOW if os.name == 'nt' else 0)
    (output / f'cli-tests/{target}-{protocol}.log').write_bytes(result.stdout + result.stderr)
    assert result.returncode == 0, (target, protocol, result.returncode, result.stderr.decode('utf-8'))
    replies = [json.loads(line) for line in result.stdout.splitlines()]
    assert [r['id'] for r in replies] == [1, 2, 3, 4], replies
    assert all('error' not in reply for reply in replies), replies
    if protocol == 'mcp':
        assert all(t['name'].startswith('jfgui.') for t in replies[1]['result']['tools']), replies
        assert not replies[2]['result'].get('isError'), replies
        document = replies[3]['result']['structuredContent']['data']
    else:
        assert 'formulas.insert' in result.stdout.decode('utf-8'), replies
        document = replies[3]['result']
    assert document['paragraphs'][0]['text'] == text, document
    assert document['revision'] == 1, document
    assert result.stderr == b'', result.stderr
    print(f'PASS {target} {protocol}: CLI headless streams, namespace, UTF-8 edit and revision', flush=True)
    return dict(target=target, protocol=protocol, headlessThroughCli=True, replies=4,
                unicodeEdit=True, revision=1, separateStreams=True, ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', choices=['win', 'linux', 'both'], default='both')
    parser.add_argument('--out', default='build/wordcraft-table-formulas')
    args = parser.parse_args()
    output = (ROOT / args.out).resolve()
    (output / 'cli-tests').mkdir(parents=True, exist_ok=True)
    targets = ['win', 'linux'] if args.target == 'both' else [args.target]
    results = [scenario(target, output, protocol) for target in targets for protocol in ['json', 'mcp']]
    (output / 'rename-cli.json').write_text(json.dumps(results, ensure_ascii=False, indent=2) + '\n',
                                            encoding='utf-8')


if __name__ == '__main__':
    main()
