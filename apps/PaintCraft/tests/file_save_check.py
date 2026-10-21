"""Small file-save regressions using ordinary files and real PaintCraft binaries.

Run from RustLib: python apps/PaintCraft/tests/file_save_check.py --out build/PaintCraft-review/after
No crash injection, fuzzing, links, or resource stress. Artifacts are retained.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
SENTINEL = b'Existing temporary file from another save.\n'


def native_cases(exe, base):
    base.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix='files-', dir=base))
    env = dict(os.environ, PaintCraft_CONFIG_DIR=str(work / 'config'))
    results = []

    def run(args, label):
        p = subprocess.run([str(exe), '--no-plugins', *args], cwd=work,
                           env=env, capture_output=True, timeout=60)
        (work / (label + '.log')).write_bytes(p.stdout + p.stderr)
        return p.returncode

    target = work / 'saved.png'
    adjacent = work / 'saved.png.tmp~'
    adjacent.write_bytes(SENTINEL)
    code = run(['--shot', str(target), '--size', '400x300'], 'save')
    results.append(dict(test='save-valid-png', ok=code == 0 and target.is_file()
                        and target.read_bytes().startswith(b'\x89PNG'), exit=code))
    results.append(dict(test='preserve-existing-temp-file',
                        ok=adjacent.is_file() and adjacent.read_bytes() == SENTINEL))
    config = work / 'config'
    config.mkdir(exist_ok=True)
    settings = config / 'settings.ini'
    settings.write_text('[Review]\nkept=1\n', encoding='utf-8')
    settings_temp = config / 'settings.ini.tmp~'
    settings_temp.write_bytes(SENTINEL)
    if os.name != 'nt':
        settings.chmod(0o600)
    settings_script = work / 'settings.uis'
    settings_script.write_text('cmd 850\nexpect #5901 text "ok"\n', encoding='utf-8')
    code = run(['--script', str(settings_script), '--verify-dirty'], 'settings')
    results.append(dict(test='save-settings', ok=code == 0 and b'kept=1' in settings.read_bytes(), exit=code))
    results.append(dict(test='preserve-existing-settings-temp',
                        ok=settings_temp.is_file() and settings_temp.read_bytes() == SENTINEL))
    if os.name != 'nt':
        mode = stat.S_IMODE(settings.stat().st_mode)
        results.append(dict(test='preserve-private-settings-mode', ok=mode == 0o600, mode=oct(mode)))
    options_script = work / 'options.uis'
    options_script.write_text('expect #9901 value 1\nexpect #9918 text ""\n', encoding='utf-8')
    code = run(['--config', str(config), '--plugins', str(work / 'plugins'),
                '--settings-save-at', '0', '--crash-dir', str(work / 'reports'),
                '--script', str(options_script), '--verify-dirty'], 'options')
    results.append(dict(test='option-values-are-not-images', ok=code == 0, exit=code))
    if os.name != 'nt':
        target.chmod(0o600)
        code = run(['--shot', str(target), '--size', '400x300'], 'replace')
        mode = stat.S_IMODE(target.stat().st_mode)
        results.append(dict(test='preserve-private-mode', ok=code == 0 and mode == 0o600,
                            mode=oct(mode), exit=code))
        # Distinct valid names on a case-sensitive filesystem must open distinct images.
        fixture = ROOT / 'apps/PaintCraft/tests/fixtures/a1_rgba.png'
        for name in ['Case.png', 'case.png']:
            shutil.copyfile(fixture, work / name)
        script = work / 'case.uis'
        script.write_text('size 1000 700\nexpect #9901 value 1\ncmd 101\n'
                          'click #65548\nkey ctrl+a\ntype "case.png"\n'
                          'click button:"Öffnen"\nexpect #9901 value 2\n'
                          'expect #9911 text "case.png"\n', encoding='utf-8')
        code = run(['--open', str(work / 'Case.png'), '--script', str(script), '--verify-dirty'], 'case')
        results.append(dict(test='case-distinct-documents', ok=code == 0, exit=code))
    for result in results:
        result['artifacts'] = str(work)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', required=True)
    parser.add_argument('--label', default='file-save-results')
    parser.add_argument('--linux-case', action='store_true')
    args = parser.parse_args()
    output = (ROOT / args.out).resolve()
    if args.linux_case:
        work = Path(tempfile.mkdtemp(prefix='jfgui-file-review-'))
        rows = native_cases(output / 'linux/PaintCraft', work)
        saved = output / 'file-save-cases' / work.name
        shutil.copytree(work, saved)
        for row in rows:
            row['artifacts'] = str(saved / Path(row['artifacts']).name)
        print(json.dumps(rows))
        return 0
    results = native_cases(output / 'win/PaintCraft.exe', output / 'file-save-cases')
    for row in results:
        row['platform'] = 'win'
    def linux_path(path):
        return subprocess.check_output(['wsl.exe', '-d', 'Ubuntu-24.04', '-e',
                                        'wslpath', '-a', str(path)], text=True).strip()
    command = ['wsl.exe', '-d', 'Ubuntu-24.04', '-e', 'python3', linux_path(Path(__file__).resolve()),
               '--linux-case', '--out', linux_path(output)]
    proc = subprocess.run(command, capture_output=True, text=True, timeout=180)
    if proc.returncode:
        raise RuntimeError(proc.stdout + proc.stderr)
    linux = json.loads(proc.stdout)
    for row in linux:
        row['platform'] = 'linux'
    results += linux
    (output / (args.label + '.json')).write_text(json.dumps(results, indent=2) + '\n', encoding='utf-8')
    for row in results:
        print(('PASS' if row['ok'] else 'FAIL'), row)
    return int(any(not row['ok'] for row in results))


if __name__ == '__main__':
    sys.exit(main())
