"""Run the unchanged native suite in a fresh working directory, retaining its evidence.

Native --fixture and several .uis scripts intentionally write build/jfword-*.docx.
An isolated cwd prevents concurrent builds/suites from changing these inputs.
"""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
import build_rs
from tools.host_exec import run_linux, linux_path
import check


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--target', choices=['win', 'linux', 'both'], default='both')
    p.add_argument('--build-dir', default='build/wordcraft-exact-rows')
    a = p.parse_args()
    output = (ROOT / a.build_dir).resolve()
    output.mkdir(parents=True, exist_ok=True)
    workspace = Path(tempfile.mkdtemp(prefix='native-checks-', dir=output))
    (workspace / 'build').mkdir()
    for folder in ['tests', 'fixtures']:
        shutil.copytree(ROOT / 'apps/jfWord' / folder, workspace / 'apps/jfWord' / folder,
                        ignore=shutil.ignore_patterns('__pycache__'))
    print(f'Isolated native test workspace: {workspace}', flush=True)

    def invoke(target, exe, args, timeout=600):
        if target == 'linux':
            result = run_linux([linux_path(exe), *args], cwd=workspace, timeout=timeout)
        else:
            if os.name != 'nt':
                raise RuntimeError('Windows tests require a Windows host')
            result = subprocess.run([str(exe), *args], cwd=workspace, timeout=timeout,
                                    capture_output=True, text=True, encoding='utf-8',
                                    errors='replace', creationflags=subprocess.CREATE_NO_WINDOW)
        if result.returncode == 0:
            for script, source, destination in [
                ('exact_rows', 'jfword-exact-rows-dialog', 'dialog'),
                ('formulas', 'jfword-formula-dialog', 'formula-dialog'),
            ]:
                if f'apps/jfWord/tests/{script}.uis' in args:
                    for suffix in ['light', 'dark-150']:
                        shutil.copyfile(workspace / f'build/{source}-{suffix}.png',
                                        output / f'native-{target}-{destination}-{suffix}.png')
        return result

    previous_runner = build_rs.run_exe
    previous_root = check.ROOT
    previous_args = sys.argv
    try:
        build_rs.run_exe = invoke
        check.ROOT = workspace
        sys.argv = ['check.py', '--target', a.target, '--build-dir', str(output)]
        check.main()
    finally:
        build_rs.run_exe = previous_runner
        check.ROOT = previous_root
        sys.argv = previous_args


if __name__ == '__main__':
    main()
