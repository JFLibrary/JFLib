"""Real JSONL service acceptance run. --release also tests and packages Kritzel for both OSes."""
import argparse
import hashlib
import json
from pathlib import Path
import queue
import subprocess
import sys
import threading
import time

ROOT = Path(__file__).resolve().parents[3]


class Service:
    def __init__(self):
        self.process = subprocess.Popen([sys.executable, '-u', 'tools/build_manager.py', '--stdio'], cwd=ROOT,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding='utf-8')
        self.messages = queue.Queue()
        self.last = {}
        self.sequence = 0
        def reader():
            for line in self.process.stdout:
                self.messages.put(json.loads(line))
        self.reader = threading.Thread(target=reader, daemon=True)
        self.reader.start()

    def receive(self, timeout):
        value = self.messages.get(timeout=timeout)
        if value.get('event') == 'job':
            self.last[value['job']['id']] = value['job']
            print(value['job']['app'], value['job']['status'], value['job'].get('message', ''), flush=True)
        elif value.get('event') == 'phase':
            print(value['job_id'], value['phase'], flush=True)
        return value

    def request(self, method, **params):
        self.sequence += 1
        self.process.stdin.write(json.dumps(dict(version=1, id=self.sequence, method=method, params=params)) + '\n')
        self.process.stdin.flush()
        end = time.monotonic() + 120
        while time.monotonic() < end:
            value = self.receive(max(1, end - time.monotonic()))
            if value.get('id') == self.sequence:
                if 'error' in value:
                    raise AssertionError(value['error'])
                return value['result']
        raise AssertionError('Backend response timeout')

    def wait(self, ids):
        end = time.monotonic() + 1800
        while any(self.last.get(i, {}).get('status') not in ('succeeded', 'failed', 'cancelled') for i in ids):
            self.receive(max(1, end - time.monotonic()))
            if time.monotonic() > end:
                raise AssertionError('Build timeout')
        for ident in ids:
            job = self.last[ident]
            assert job['status'] == 'succeeded', (job['app'], job['message'])
            for artifact in job['artifacts']:
                assert hashlib.sha256(Path(artifact['path']).read_bytes()).hexdigest() == artifact['sha256']
        return [self.last[i] for i in ids]

    def close(self):
        self.process.stdin.close()
        self.process.wait(timeout=30)
        self.reader.join(timeout=5)
        error = self.process.stderr.read()
        self.process.stdout.close()
        self.process.stderr.close()
        assert self.process.returncode == 0, error


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--release', action='store_true')
    a = parser.parse_args()
    service = Service()
    try:
        catalog = service.request('catalog')
        assert 'buildManager' in [app['id'] for app in catalog['apps']]
        print('Catalog:', len(catalog['apps']), 'apps;', catalog['capabilities'], flush=True)
        profile = dict(name='Acceptance', apps=['Kritzel'] if a.release else ['template', 'gallery'],
            targets=['win', 'linux'] if a.release else ['win', 'linux', 'web'], styles=['winui'] if a.release else ['winui', 'classic'],
            mode='release' if a.release else 'development', optimization='o2', jobs=8,
            tests=a.release, output='build/manager-integration')
        preview = service.request('preview', profile=profile)
        assert len(preview['jobs']) == len(profile['apps'])
        ids = service.request('enqueue', profile=profile)['ids']
        jobs = service.wait(ids)
        (ROOT / 'build/manager-integration').mkdir(exist_ok=True)
        (ROOT / 'build/manager-integration' / ('release-results.json' if a.release else 'development-results.json')).write_text(json.dumps(jobs, indent=2), encoding='utf-8')
        job = jobs[0]
        index = next(i for i, artifact in enumerate(job['artifacts']) if artifact['kind'] == 'app' and artifact['target'] == 'win')
        start = dict(name='Dunkel · 150 %', app=job['app'], argv=['--theme', 'dark', '--scale', '150'])
        catalog = service.request('catalog')
        saved = service.request('save_profile', kind='starts', profile=start, revision=catalog['profile_revision'])
        restored = next(p for p in saved['profiles']['starts'] if p['app'] == start['app'] and p['name'] == start['name'])
        launched = service.request('run', id=job['id'], artifact=index, argv=restored['argv'] + ['--auto-close', '300', '--dump'])
        service.wait([launched['id']])
        print('PASS: catalog, preview, queue, artifacts, tests and app launch', flush=True)
    finally:
        service.close()


if __name__ == '__main__':
    main()
