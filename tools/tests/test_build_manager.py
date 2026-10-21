"""Build Manager regression tests: edits, isolation, cancellation and release gating."""
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / 'tools'))
import build_rs as build
from buildmanager import model, runner
from buildmanager.server import ManagedProcess, Server


class ModelTests(unittest.TestCase):
    def setUp(self):
        (ROOT / 'build').mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix='manager-unit-', dir=ROOT / 'build')
        self.root = Path(self.temp.name)
        (self.root / 'apps/template').mkdir(parents=True)
        (self.root / 'apps/template/main.rs').write_bytes(b'#![no_std]\n')
        self.project = model.Project(self.root)

    def tearDown(self):
        self.temp.cleanup()

    def profile(self):
        return dict(name='Entwicklung', apps=['template'], targets=['win'], styles=['winui'],
                    mode='development', optimization='size', jobs=2, output='build/manager')

    def test_preserves_unknown_package_comments_and_detects_external_edits(self):
        path = self.root / 'apps/template/PACKAGE'
        path.write_bytes(b'# Keep me\r\ncustom = value\r\nversion=1.0.0\r\n')
        original = model.revision(path)
        result = self.project.save_package('template', {'version': '2.0.0'}, original)
        self.assertEqual(path.read_bytes(), b'# Keep me\r\ncustom = value\r\nversion=2.0.0\r\n')
        self.assertEqual(result['values']['custom'], 'value')
        path.write_bytes(path.read_bytes() + b'new=external\r\n')
        with self.assertRaisesRegex(ValueError, 'extern'):
            self.project.save_package('template', {'version': '3.0.0'}, result['revision'])
        self.assertIn(b'new=external', path.read_bytes())

    def test_profile_roundtrip_and_conflict(self):
        path = self.project.profiles_path
        result = self.project.save_profile('builds', self.profile(), model.revision(path))
        self.assertEqual(self.project.profiles()['builds'][0]['apps'], ['template'])
        self.project.save_profile('starts', dict(name='Dunkel', app='template', argv=['--open', 'ä space.png', '']), result['revision'])
        self.assertEqual(self.project.profiles()['starts'][0]['argv'][-1], '')
        with self.assertRaisesRegex(ValueError, 'extern'):
            self.project.save_profile('builds', self.profile(), result['revision'])

    def test_output_and_app_paths_cannot_escape(self):
        for output in ['../elsewhere', 'build/../apps', str(self.root / 'build/absolute'), 'build']:
            with self.subTest(output=output), self.assertRaises(ValueError):
                self.project.build_profile(dict(self.profile(), output=output))
        with self.assertRaises(ValueError):
            self.project.app_dir('../template')

    def test_package_validates_styles_and_icons(self):
        values = dict(display='Template', version='0.1.0', publisher='ASM', summary='Test', description='Test', default_win='kde')
        self.project.validate_package('template', values, ['winui'], 'win')
        self.assertEqual(values['default_win'], 'winui')
        with self.assertRaisesRegex(ValueError, 'Icon fehlt'):
            self.project.validate_package('template', values, ['winui'], 'linux')

    def test_lock_excludes_second_manager(self):
        lock = model.WorkspaceLock(self.root)
        try:
            with self.assertRaisesRegex(ValueError, 'bereits'):
                model.WorkspaceLock(self.root)
        finally:
            lock.close()
        model.WorkspaceLock(self.root).close()

    def test_interrupted_history_is_not_restarted(self):
        directory = self.root / 'build/manager/jobs'
        directory.mkdir(parents=True)
        model.write_json(directory / 'old.json', dict(id='old', status='running', app='template', created=1, request={}))
        server = Server(self.root, output=io.StringIO())
        try:
            self.assertEqual(server.jobs['old']['status'], 'interrupted')
            self.assertEqual(server.queue, [])
        finally:
            server.close()


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='manager-release-', dir=ROOT / 'build')
        self.root = Path(self.temp.name)
        self.source = self.root / 'source/template.exe'
        self.source.parent.mkdir()
        self.source.write_bytes(b'EXACT TESTED BYTES')
        # every app snapshot ships the library's notices (build_rs.library_notices)
        (self.root / 'THIRD_PARTY_NOTICES.md').write_bytes(b'notices')
        self.spec = dict(id='one', app='template', apps=['template'], styles=['winui'], targets=['win'],
                         output='build/manager', mode='release', optimization='o2', jobs=1, tests=True, package={})
        self.events = []
        self.patches = [patch.object(build, 'ROOT', self.root), patch.object(build, 'lint', return_value=[]),
                        patch.object(build, 'check_toolchain'), patch.object(build, 'build_apps',
                            side_effect=lambda bs, *_: [(bs[0], 'template', self.source)])]
        for p in self.patches:
            p.start()

    def tearDown(self):
        for p in reversed(self.patches):
            p.stop()
        self.temp.cleanup()

    def emit(self, event, **data):
        self.events.append((event, data))

    def test_failing_app_tests_prevent_package(self):
        with patch.object(build, 'run_app_tests', return_value=[dict(status='failed')]), patch.object(runner.package, 'make_setup') as setup:
            with self.assertRaisesRegex(ValueError, 'Kein Paket'):
                runner.execute(self.spec, self.emit)
            setup.assert_not_called()
        self.assertFalse(any(e == 'result' for e, _ in self.events))

    def test_package_receives_tested_snapshot_not_changed_cache(self):
        def tests(name, target, exe, **kw):
            self.assertEqual(exe.read_bytes(), b'EXACT TESTED BYTES')
            self.source.write_bytes(b'CACHE CHANGED LATER')
            record = dict(script='smoke.uis', status='passed')
            kw['report'](record)
            return [record]
        def setup(app, meta, styles, stage, work, *args):
            data = (work / 'winui/win/template.exe').read_bytes()
            self.assertEqual(data, b'EXACT TESTED BYTES')
            self.assertEqual((work / 'winui/win/THIRD_PARTY_NOTICES.md').read_bytes(), b'notices')
            out = stage / 'template_Setup.exe'
            out.write_bytes(data)
            return out
        with patch.object(build, 'run_app_tests', side_effect=tests), patch.object(runner.package, 'make_setup', side_effect=setup):
            result = runner.execute(self.spec, self.emit)
        self.assertEqual([a['kind'] for a in result['artifacts']], ['app', 'installer'])
        self.assertEqual(result['artifacts'][0]['sha256'], result['artifacts'][1]['sha256'])

    def test_no_tests_is_reported_explicitly(self):
        self.spec.update(mode='development', tests=True)
        with patch.object(build, 'run_app_tests', return_value=[]):
            result = runner.execute(self.spec, self.emit)
        self.assertEqual(result['tests'][0]['status'], 'none')

    def test_web_stages_all_compute_variants_once_per_style(self):
        self.spec.update(targets=['web'], styles=['winui', 'kde'], mode='development', tests=False)
        def apps(builders, names, styles):
            self.assertEqual([b.web_mode for b in builders], ['scalar', 'simd', 'threads'])
            built = []
            for b in builders:
                for style in styles:
                    directory = b.web_public / style / names[0]
                    directory.mkdir(parents=True, exist_ok=True)
                    (directory / 'index.html').write_text(style)
                    (directory / 'variants.json').write_text(json.dumps({
                        'scalar': 'template.wasm', 'simd': 'template.simd.wasm', 'threads': 'template.threads.wasm'}))
                    suffix = '' if b.web_mode == 'scalar' else '.' + b.web_mode
                    exe = directory / f'template{suffix}.wasm'
                    exe.write_text(style + ':' + b.web_mode)
                    built.append((b, names[0], exe))
            return built
        with patch('tools.web_sysroot.ensure', side_effect=lambda path: path), patch.object(build, 'build_apps', side_effect=apps):
            result = runner.execute(self.spec, self.emit)
        self.assertEqual([a['style'] for a in result['artifacts']], ['winui', 'kde'])
        for artifact in result['artifacts']:
            directory = Path(artifact['path']).parent
            for mode, filename in json.loads((directory / 'variants.json').read_text()).items():
                self.assertEqual((directory / filename).read_text(), artifact['style'] + ':' + mode)


class ProcessTests(unittest.TestCase):
    def test_nested_packager_has_independent_stdin(self):
        with tempfile.TemporaryDirectory(prefix='manager-nested-', dir=ROOT / 'build') as directory:
            script = Path(directory) / 'parent.py'
            script.write_text('import os,sys,threading\n'
                f'sys.path.insert(0, {str(ROOT)!r})\n'
                'sys.stdin.buffer.readline()\n'
                'threading.Thread(target=lambda: sys.stdin.buffer.read(),daemon=True).start()\n'
                'from tools.package import run\n'
                'run([sys.executable,"-c","print(123)"])\n'
                'sys.stdout.flush()\nos._exit(0)\n', encoding='utf-8')
            managed = ManagedProcess([sys.executable, '-u', str(script)], dict(os.environ))
            try:
                self.assertEqual(managed.process.wait(timeout=15), 0)
                self.assertIn(b'123', managed.process.stdout.read())
            finally:
                managed.close()

    @unittest.skipUnless(os.name == 'nt' and os.environ.get('jfgui_TEST_WSL') == '1', 'optional WSL integration')
    def test_wsl_tree_cancellation(self):
        with tempfile.TemporaryDirectory(prefix='manager-wsl-Grüße ', dir=ROOT / 'build') as directory:
            marker = Path(directory) / 'alive.txt'
            script = Path(directory) / 'parent.py'
            script.write_text('import sys\n'
                f'sys.path.insert(0, {str(ROOT)!r})\n'
                'sys.stdin.buffer.readline()\n'
                'from tools.host_exec import run_linux,linux_path\n'
                f'run_linux(["python3",linux_path({str(ROOT / "apps/buildManager/tests/process_probe.py")!r}),'
                f'"child",linux_path({str(marker)!r})],timeout=None)\n', encoding='utf-8')
            managed = ManagedProcess([sys.executable, '-u', str(script)], dict(os.environ))
            try:
                end = time.monotonic() + 20
                while not marker.exists() and time.monotonic() < end:
                    time.sleep(.05)
                self.assertTrue(marker.exists(), 'WSL descendant did not start')
                managed.terminate()
                managed.process.wait(timeout=5)
                time.sleep(.5)
                before = marker.read_bytes()
                time.sleep(.5)
                self.assertEqual(marker.read_bytes(), before, 'WSL descendant survived cancellation')
            finally:
                managed.close()

    def test_owned_python_tree_cancellation(self):
        with tempfile.TemporaryDirectory(prefix='manager-process-ä ', dir=ROOT / 'build') as directory:
            marker = Path(directory) / 'child.txt'
            script = Path(directory) / 'parent.py'
            child = Path(directory) / 'child.py'
            child.write_text('import pathlib,time,sys\np=pathlib.Path(sys.argv[1])\nwhile True:\n p.write_text(str(time.time()))\n time.sleep(.02)\n', encoding='utf-8')
            script.write_text('import subprocess, sys, time\n'
                'sys.stdin.buffer.readline()\n'
                'subprocess.Popen([sys.executable,sys.argv[2],sys.argv[1]])\n'
                'print("ready",flush=True)\ntime.sleep(60)\n', encoding='utf-8')
            managed = ManagedProcess([sys.executable, str(script), str(marker), str(child)], dict(os.environ))
            try:
                self.assertEqual(managed.process.stdout.readline().strip(), b'ready')
                end = time.monotonic() + 5
                while not marker.exists() and time.monotonic() < end:
                    time.sleep(.02)
                self.assertTrue(marker.exists())
                managed.terminate()
                managed.process.wait(timeout=5)
                before = marker.read_bytes()
                time.sleep(.2)
                self.assertEqual(marker.read_bytes(), before)
            finally:
                managed.close()


if __name__ == '__main__':
    unittest.main()
