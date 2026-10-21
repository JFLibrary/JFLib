"""One isolated build job: shared builders, immutable candidates, tests, then packaging."""
import contextlib
import hashlib
import json
import os
import shutil
import sys
import threading
from pathlib import Path

import build_rs as build
from tools import package
from .model import write_json


def prepare_tests(app, target, exe, cache):
    if app != 'PaintCraft':
        return
    scripts = list((build.ROOT / 'apps' / app / 'tests').glob('*.uis'))
    if not any('@ptest' in p.read_text(encoding='utf-8').splitlines()[0] for p in scripts if p.stat().st_size):
        return
    builder = build.PluginBuilder(target, cache / 'test-plugins')
    for folder, names in build.PLUGIN_TEST_SET.items():
        destination = exe.parent / 'ptest' / folder
        destination.mkdir(parents=True, exist_ok=True)
        for name in names:
            shutil.copyfile(builder.build(name), destination / (name + ('.dll' if target == 'win' else '.so')))


def build_arguments(spec, cache):
    targets = spec['targets']
    mode = [] if spec['mode'] == 'development' else package.rs_mode(spec['optimization'])
    # GUI shows the exact argument vectors of each target, without inventing a
    # single CLI --target value for arbitrary subsets such as win + web.
    return [[sys.executable, '-u', str(build.ROOT / 'build_rs.py'), 'app', spec['app'],
             '--target', t, '--style', ','.join(spec['styles']), *mode,
             '--out', str(cache), '-j', str(spec['jobs'])] for t in targets]


def cache_path(spec):
    key = hashlib.sha256(json.dumps(spec['styles'], separators=(',', ':')).encode()).hexdigest()[:12]
    profile = 'development' if spec['mode'] == 'development' else spec['optimization']
    return build.ROOT / spec['output'] / 'cache' / profile / key


def execute(spec, emit):
    if spec.get('launch'):
        launch = spec['launch']
        artifact = launch['artifact']
        emit('phase', phase='App läuft')
        # Development sessions get their own settings just like other tool launches.
        config = build.ROOT / 'build/manager/run-config' / spec['app']
        config.mkdir(parents=True, exist_ok=True)
        os.environ['PaintCraft_CONFIG_DIR'] = str(config)
        os.environ['WSLENV'] = ':'.join([x for x in os.environ.get('WSLENV', '').split(':')
            if x and not x.startswith('PaintCraft_CONFIG_DIR')] + ['PaintCraft_CONFIG_DIR/p'])
        result = build.run_exe(artifact['target'], Path(artifact['path']), launch['argv'], timeout=None)
        print(result.stdout + result.stderr)
        if result.returncode:
            raise ValueError(f'App beendet mit Exitcode {result.returncode}.')
        emit('result', artifacts=[], tests=[])
        return dict(artifacts=[], tests=[])
    app = spec['app']
    release = spec['mode'] == 'release'
    cache = cache_path(spec)
    stage = build.ROOT / spec['output'] / 'artifacts' / spec['id']
    stage.mkdir(parents=True, exist_ok=False)
    emit('phase', phase='Voraussetzungen prüfen')
    bad = build.lint()
    if bad:
        raise ValueError('\n'.join(bad))
    build.check_toolchain()
    # Settings are snapshotted at enqueue time; sources follow the ordinary builder.
    write_json(stage / 'request.json', spec)
    emit('phase', phase='Varianten bauen')
    for args in build_arguments(spec, cache):
        print('$ ' + json.dumps(args, ensure_ascii=False), flush=True)
    build.RUSTC_SLOTS = threading.BoundedSemaphore(spec['jobs'])
    optimization = spec['optimization'] if release and spec['optimization'] != 'release' else None
    builders = build.make_builders(spec['targets'], cache, release, spec['jobs'], optimization)
    built = build.build_apps(builders, [app], spec['styles'])
    artifacts = []
    tests = []
    work = stage / app
    variants = []
    for builder, _, exe in built:
        # One browser artifact contains all three instruction-set variants. Stage
        # it once per style after every variant has finished building.
        if builder.target == 'web' and builder.web_mode != 'scalar':
            continue
        if exe is None:
            raise ValueError(f'{app}: nicht unterstütztes Ziel {builder.target}')
        # build_apps returns builders x apps x styles in a stable order.
        style = spec['styles'][len(variants) % len(spec['styles'])]
        target = builder.target
        destination = work / style / target
        destination.mkdir(parents=True)
        if target == 'web':
            shutil.copytree(exe.parent, destination, dirs_exist_ok=True)
            entry = destination / 'index.html'
            kind = 'web'
        else:
            entry = destination / exe.name
            shutil.copy2(exe, entry)
            for module in package.app_modules(app, target, 'rs'):
                shutil.copy2(exe.parent / module, destination / module)
            for relative, source in package.package_notices([style], 'rs').items():
                (destination / relative).parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, destination / relative)
            kind = 'app'
        variants.append((style, target, entry))
        artifacts.append(dict(kind=kind, app=app, style=style, target=target, path=str(entry), size=entry.stat().st_size))
    emit('phase', phase='App-Tests ausführen' if release or spec['tests'] else 'Ergebnisse übernehmen')
    for style, target, exe in variants:
        if target == 'web':
            tests.append(dict(style=style, target=target, status='not-applicable', reason='Web-Bundle: keine nativen UI-Tests'))
            continue
        if not release and not spec['tests']:
            continue
        prepare_tests(app, target, exe, cache)
        config = stage / 'test-config' / style / target
        config.mkdir(parents=True, exist_ok=True)
        os.environ['PaintCraft_CONFIG_DIR'] = str(config)
        os.environ['ASMINST_SHELL_DIR'] = str(config / 'shell')
        os.environ['WSLENV'] = ':'.join([x for x in os.environ.get('WSLENV', '').split(':')
            if x and not x.startswith(('PaintCraft_CONFIG_DIR', 'ASMINST_SHELL_DIR'))] + ['PaintCraft_CONFIG_DIR/p', 'ASMINST_SHELL_DIR/p'])
        def report(record):
            value = dict(record, target=target, style=style)
            tests.append(value)
            if value.get('output'):
                print(value['output'])
            emit('test', **{k: v for k, v in value.items() if k != 'output'})
        records = build.run_app_tests(app, target, exe, verify_dirty=True, report=report)
        if not records:
            value = dict(style=style, target=target, status='none', reason='Keine App-Tests vorhanden')
            tests.append(value)
            emit('test', **value)
        write_json(stage / 'tests.json', tests)
        if any(r['status'] == 'failed' for r in records):
            raise ValueError(f'{app} / {target} / {style}: App-Tests fehlgeschlagen. Kein Paket erstellt.')
    if release:
        emit('phase', phase='Pakete zusammenstellen')
        meta = spec['package']
        if 'win' in spec['targets']:
            setup = package.make_setup(app, meta, spec['styles'], stage, work, 'rs', cache, spec['optimization'])
            artifacts.append(dict(kind='installer', app=app, target='win', style='', path=str(setup), size=setup.stat().st_size))
        if 'linux' in spec['targets']:
            deb = package.make_deb(app, meta, spec['styles'], stage, work, 'rs')
            artifacts.append(dict(kind='deb', app=app, target='linux', style='', path=str(deb), size=deb.stat().st_size))
    # A record is published only after every requested stage has succeeded.
    for artifact in artifacts:
        path = Path(artifact['path'])
        artifact['sha256'] = hashlib.sha256(path.read_bytes()).hexdigest()
    result = dict(artifacts=artifacts, tests=tests)
    write_json(stage / 'result.json', result)
    emit('result', artifacts=artifacts, tests=[{k: v for k, v in t.items() if k != 'output'} for t in tests])
    return result


class LogWriter:
    def __init__(self, emit):
        self.emit = emit
        self.pending = ''
        self.lock = threading.Lock()

    def write(self, value):
        with self.lock:
            self.pending += value
            while '\n' in self.pending or len(self.pending) >= 8192:
                i = self.pending.find('\n')
                n = i + 1 if 0 <= i < 8192 else min(len(self.pending), 8192)
                self.emit('log', text=self.pending[:n])
                self.pending = self.pending[n:]
        return len(value)

    def flush(self):
        with self.lock:
            if self.pending:
                self.emit('log', text=self.pending)
                self.pending = ''


def worker(path):
    output = sys.stdout
    lock = threading.Lock()
    def emit(event, **data):
        with lock:
            output.write(json.dumps(dict(event=event, **data), ensure_ascii=False) + '\n')
            output.flush()
    spec = json.loads(Path(path).read_text(encoding='utf-8'))
    log = LogWriter(emit)
    with contextlib.redirect_stdout(log), contextlib.redirect_stderr(log):
        try:
            execute(spec, emit)
            log.flush()
            return 0
        except Exception as e:
            log.flush()
            emit('failure', message=str(e))
            return 1
