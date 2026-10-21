"""Native process and UI acceptance matrix. Builds must already exist in --out."""
import argparse
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from tools.host_exec import linux_path, run_native
import build_rs


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--out', default='build/manager-check')
    parser.add_argument('--target', choices=['win', 'linux', 'both'], default='both')
    a = parser.parse_args()
    out = ROOT / a.out
    targets = ['win', 'linux'] if a.target == 'both' else [a.target]
    for target in targets:
        exe = out / target / ('buildManager.exe' if target == 'win' else 'buildManager')
        with tempfile.TemporaryDirectory(prefix='Prozess Grüße ', dir=out) as scratch:
            args = ['--process-check', sys.executable if target == 'win' else 'python3',
                    '--scratch', scratch if target == 'win' else linux_path(scratch)]
            result = run_native(target, exe, args, timeout=30)
            assert result.returncode == 0, result.stdout + result.stderr
            print(target, result.stdout.strip(), flush=True)
        for script in ('smoke.uis', 'profiles.uis'):
            path = ROOT / 'apps/buildManager/tests' / script
            result = run_native(target, exe, build_rs.script_args(path) + ['--script', path.relative_to(ROOT).as_posix(), '--verify-dirty'], timeout=30)
            assert result.returncode == 0, result.stdout + result.stderr
            print(target, script, 'PASS', result.stderr.strip(), flush=True)
        for theme in ('light', 'dark'):
            for scale in ('100', '150'):
                shot = out / f'ui-{target}-{theme}-{scale}.png'
                result = run_native(target, exe, ['--fixture', 'apps/buildManager/tests/fixture.json', '--theme', theme,
                    '--scale', scale, '--shot', shot.relative_to(ROOT).as_posix()], timeout=30)
                assert result.returncode == 0, result.stdout + result.stderr
                assert shot.is_file()
                print(target, theme, scale, 'render PASS', flush=True)


if __name__ == '__main__':
    main()
