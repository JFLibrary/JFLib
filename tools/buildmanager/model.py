"""Discovery, portable profiles and conflict-aware edits of existing package metadata."""
import hashlib
import json
import os
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

import build_rs as build
from tools import package
from tools.host_exec import run_linux
from tools.project_sources import entrypoint

VERSION = 1
PACKAGE_KEYS = ('display', 'version', 'publisher', 'summary', 'description', 'generic',
                'categories', 'deb_name', 'default_win', 'default_linux', 'styles')


def revision(path):
    return hashlib.sha256(path.read_bytes() if path.exists() else b'').hexdigest()


def atomic_write(path, data, expected=None):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    if expected is not None and revision(path) != expected:
        raise ValueError('Die Datei wurde extern geändert. Bitte neu laden und Änderungen erneut übernehmen.')
    fd, temporary = tempfile.mkstemp(prefix='.' + path.name + '-', dir=path.parent)
    try:
        with os.fdopen(fd, 'wb') as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        if expected is not None and revision(path) != expected:
            raise ValueError('Die Datei wurde während des Speicherns extern geändert.')
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def write_json(path, value, expected=None):
    atomic_write(path, (json.dumps(value, ensure_ascii=False, indent=2) + '\n').encode('utf-8'), expected)


def read_json(path, default):
    return json.loads(path.read_text(encoding='utf-8')) if path.exists() else default


def text(value, name, limit=4096):
    if not isinstance(value, str) or len(value) > limit or '\0' in value:
        raise ValueError(f'{name}: ungültiger Text.')
    return value


def names(value, name):
    if not isinstance(value, list) or len(value) > 128:
        raise ValueError(f'{name}: eine Liste wird erwartet.')
    return list(dict.fromkeys(text(x, name, 128) for x in value))


def probe(command):
    try:
        r = subprocess.run(command, capture_output=True, timeout=20,
                           creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
        return r.returncode == 0, r.stdout.decode('utf-8', 'replace').strip()
    except (OSError, subprocess.TimeoutExpired):
        return False, ''


def capabilities(distro):
    rust, version = probe(['rustc', '--version'])
    result = {}
    wsl = False
    deb = False
    if os.name == 'nt':
        try:
            ok = subprocess.run(['wsl.exe', '-d', distro, '-e', 'python3', '--version'],
                                capture_output=True, timeout=20, creationflags=subprocess.CREATE_NO_WINDOW)
            wsl = ok.returncode == 0
            if wsl:
                deb = subprocess.run(['wsl.exe', '-d', distro, '-e', 'dpkg-deb', '--version'],
                                     capture_output=True, timeout=20, creationflags=subprocess.CREATE_NO_WINDOW).returncode == 0
        except (OSError, subprocess.TimeoutExpired):
            pass
    else:
        deb = bool(shutil.which('dpkg-deb'))
    for target, linker in [('win', 'lld-link'), ('linux', 'ld.lld'), ('web', 'wasm-ld')]:
        ok, libdir = probe(['rustc', '--print', 'target-libdir', '--target', build.TRIPLE[target]]) if rust else (False, '')
        core = ok and bool(list(Path(libdir).glob('libcore-*')))
        tools = bool(shutil.which(build.tool(linker))) and (target != 'win' or bool(shutil.which(build.tool('llvm-rc'))))
        can_build = bool(rust and core and tools)
        can_run = (os.name == 'nt' if target == 'win' else (os.name != 'nt' or wsl) if target == 'linux' else False)
        can_package = can_build and (os.name == 'nt' if target == 'win' else deb if target == 'linux' else True)
        result[target] = dict(build=can_build, run=can_run, package=can_package,
            reason='' if can_build else f'{target}: Rust-Target oder LLVM-Werkzeug fehlt.',
            run_reason='' if can_run else ('Keine native Windows-Ausführung auf Linux.' if target == 'win' else
                'WSL mit Python 3 fehlt.' if target == 'linux' else 'Browser-Vorschau gehört nicht zu dieser Version.'),
            package_reason='' if can_package else 'Paketwerkzeug oder ausführbarer Ziel-Host fehlt.')
    return dict(host='win' if os.name == 'nt' else 'linux', rust=version, targets=result, wsl_distro=distro)


class Project:
    def __init__(self, root):
        self.root = Path(root).resolve()
        self.profiles_path = self.root / 'build-profiles.json'

    def app_dir(self, app):
        text(app, 'App', 128)
        if not re.fullmatch(r'[A-Za-z0-9_-]+', app):
            raise ValueError('Ungültige App-ID.')
        directory = self.root / 'apps' / app
        if not directory.is_dir() or not directory.resolve().is_relative_to((self.root / 'apps').resolve()):
            raise ValueError(f'Unbekannte App: {app}')
        return directory

    def profiles(self):
        data = read_json(self.profiles_path, dict(version=VERSION, builds=[], starts=[]))
        if data.get('version') != VERSION or not isinstance(data.get('builds'), list) or not isinstance(data.get('starts'), list):
            raise ValueError('Unbekanntes oder ungültiges Profilformat.')
        return data

    def package_data(self, app):
        path = self.app_dir(app) / 'PACKAGE'
        return dict(values=package.kv_file(path) if path.exists() else {}, revision=revision(path))

    def save_package(self, app, values, expected):
        path = self.app_dir(app) / 'PACKAGE'
        if not isinstance(values, dict) or any(k not in PACKAGE_KEYS for k in values):
            raise ValueError('Unbekannte Paketfelder.')
        for k, v in values.items():
            if any(c in text(v, k) for c in '\r\n'):
                raise ValueError(f'{k}: nur eine Zeile erlaubt.')
        raw = path.read_bytes() if path.exists() else b''
        bom = b'\xef\xbb\xbf' if raw.startswith(b'\xef\xbb\xbf') else b''
        lines = raw[len(bom):].decode('utf-8').splitlines(keepends=True)
        eol = '\r\n' if b'\r\n' in raw else '\n'
        seen = set()
        for i, line in enumerate(lines):
            if line.lstrip().startswith(('#', ';')) or '=' not in line:
                continue
            key, _ = line.split('=', 1)
            if key.strip() in values:
                k = key.strip()
                lines[i] = key + '=' + values[k] + (eol if line.endswith(('\n', '\r')) else '')
                seen.add(k)
        if lines and not lines[-1].endswith(('\n', '\r')):
            lines[-1] += eol
        lines.extend(f'{k}={v}{eol}' for k, v in values.items() if k not in seen)
        atomic_write(path, bom + ''.join(lines).encode('utf-8'), text(expected, 'Revision', 64))
        return self.package_data(app)

    def save_profile(self, kind, value, expected):
        if kind not in ('builds', 'starts') or not isinstance(value, dict):
            raise ValueError('Ungültiger Profiltyp.')
        name = text(value.get('name'), 'Profilname', 128).strip()
        if not name:
            raise ValueError('Bitte einen Profilnamen eingeben.')
        if kind == 'builds':
            profile = self.build_profile(value)
        else:
            app = value.get('app')
            self.app_dir(app)
            profile = dict(name=name, app=app, argv=self.arguments(value.get('argv', [])))
        profile['name'] = name
        data = self.profiles()
        data[kind] = [p for p in data[kind] if not (p.get('name') == name and (kind == 'builds' or p.get('app') == profile['app']))]
        data[kind].append(profile)
        write_json(self.profiles_path, data, text(expected, 'Revision', 64))
        return dict(profiles=data, revision=revision(self.profiles_path))

    @staticmethod
    def arguments(args):
        if not isinstance(args, list) or len(args) > 256:
            raise ValueError('Ungültige Argumentliste.')
        return [text(v, 'Argument', 8192) for v in args]

    def build_profile(self, value):
        apps = names(value.get('apps', []), 'Apps')
        if not apps:
            raise ValueError('Bitte mindestens eine App auswählen.')
        for app in apps:
            entrypoint(self.app_dir(app), 'app')
        targets = names(value.get('targets', []), 'Zielplattformen')
        if not targets or any(t not in build.TRIPLE for t in targets):
            raise ValueError('Bitte gültige Zielplattformen auswählen.')
        styles = names(value.get('styles', []), 'Styles')
        if any(s not in build.STYLES for s in styles):
            raise ValueError('Ein ausgewählter Style existiert nicht mehr.')
        mode = value.get('mode', 'development')
        optimization = value.get('optimization', 'size')
        if mode not in ('development', 'release') or optimization not in ('size', 'release', 'o2'):
            raise ValueError('Unbekanntes Buildprofil.')
        jobs = value.get('jobs', os.cpu_count() or 1)
        if type(jobs) is not int or not 1 <= jobs <= 256:
            raise ValueError('Parallelität muss zwischen 1 und 256 liegen.')
        output = text(value.get('output', 'build/manager'), 'Ausgabepfad', 256).replace('\\', '/')
        path = self.root / output
        if Path(output).is_absolute() or not path.resolve().is_relative_to((self.root / 'build').resolve()) or path.resolve() == (self.root / 'build').resolve():
            raise ValueError('Ausgaben müssen in einem Unterordner von build liegen.')
        return dict(name=value.get('name', ''), apps=apps, targets=targets, styles=styles, mode=mode,
                    optimization=optimization, jobs=jobs, tests=bool(value.get('tests', False)), output=output)

    def catalog(self, caps):
        build.STYLES[:] = sorted(p.name for p in (self.root / 'styles').iterdir() if p.is_dir() and entrypoint(p, 'lib', False))
        apps = []
        for p in sorted((self.root / 'apps').iterdir()):
            if not p.is_dir():
                continue
            try:
                if not entrypoint(p, 'app', False):
                    continue
                meta = self.package_data(p.name)
                apps.append(dict(id=p.name, name=meta['values'].get('display', p.name),
                                 targets=build.app_targets(p.name), style=build.app_style(p.name),
                                 package=meta, error=''))
            except (ValueError, OSError) as e:
                apps.append(dict(id=p.name, name=p.name, targets=[], style='winui', package=dict(values={}, revision=''), error=str(e)))
        return dict(apps=apps, styles=[dict(id=s, name=package.style_info(s)[0]) for s in build.STYLES],
                    profiles=self.profiles(), profile_revision=revision(self.profiles_path), capabilities=caps,
                    workspace=str(self.root))

    def resolve(self, value, caps):
        profile = self.build_profile(value)
        result = []
        for app in profile['apps']:
            meta = self.package_data(app)['values']
            styles = profile['styles'] or (meta.get('styles', '').split() if profile['mode'] == 'release' else []) or [build.app_style(app)]
            if any(s not in build.STYLES for s in styles):
                raise ValueError(f'{app}: ungültige Paket-Styles.')
            for target in profile['targets']:
                if target not in build.app_targets(app):
                    raise ValueError(f'{app}: Ziel {target} wird nicht unterstützt.')
                cap = caps['targets'][target]
                if not cap['build']:
                    raise ValueError(cap['reason'])
                if target != 'web' and (profile['tests'] or profile['mode'] == 'release') and not cap['run']:
                    raise ValueError(cap['run_reason'])
                if profile['mode'] == 'release' and target != 'web':
                    if not cap['package']:
                        raise ValueError(cap['package_reason'])
                    self.validate_package(app, meta, styles, target)
            p = dict(profile, apps=[app], app=app, styles=styles, package=meta)
            result.append(p)
        return result

    def validate_package(self, app, meta, styles, target):
        missing = [k for k in ('display', 'version', 'publisher', 'summary', 'description') if not meta.get(k)]
        if missing:
            raise ValueError(f'{app}: Paketdaten fehlen: {", ".join(missing)}.')
        if any('\n' in v or '\r' in v for v in meta.values()):
            raise ValueError('Paketfelder dürfen keine Zeilenumbrüche enthalten.')
        if not re.fullmatch(r'[0-9][A-Za-z0-9.+:~_-]*', meta['version']):
            raise ValueError('Ungültige Paketversion.')
        # An explicitly selected style subset uses its first style when the PACKAGE
        # default is outside that subset; the original metadata is never changed.
        key = 'default_win' if target == 'win' else 'default_linux'
        if meta.get(key) not in styles:
            meta[key] = styles[0]
        if target == 'linux':
            if not re.fullmatch(r'[a-z0-9][a-z0-9+.-]+', meta.get('deb_name', app.lower())):
                raise ValueError('Ungültiger Debian-Paketname.')
            for size in (48, 128, 256):
                icon = self.app_dir(app) / 'res' / f'app_{size}.png'
                if not icon.is_file():
                    raise ValueError(f'{app}: Icon fehlt: res/app_{size}.png')


class WorkspaceLock:
    def __init__(self, root):
        path = Path(root) / 'build/manager/workspace.lock'
        path.parent.mkdir(parents=True, exist_ok=True)
        self.file = path.open('a+b')
        self.file.seek(0)
        if os.fstat(self.file.fileno()).st_size == 0:
            self.file.write(b'0')
            self.file.flush()
        self.file.seek(0)
        try:
            if os.name == 'nt':
                import msvcrt
                msvcrt.locking(self.file.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(self.file, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError:
            self.file.close()
            raise ValueError('Dieses Projekt wird bereits von einem Build Manager verwaltet.') from None

    def close(self):
        self.file.close()
