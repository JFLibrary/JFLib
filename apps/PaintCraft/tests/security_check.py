"""PaintCraft security regressions against real binaries, using disposable files only.

Run from RustLib: python apps/PaintCraft/tests/security_check.py --out build/PaintCraft-security/after
The original vulnerable build is expected to fail; --label distinguishes its evidence.
"""
import argparse
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
SENTINEL = b'PaintCraft SECURITY REVIEW: UNRELATED FILE MUST SURVIVE\n'


def native_cases(exe, base):
    results = []
    base.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='case-', dir=base) as directory:
        work = Path(directory)
        # No reference points outside the newly created test directory.
        assert work.resolve().is_relative_to(base.resolve())
        sentinel = work / 'unrelated.txt'
        sentinel.write_bytes(SENTINEL)
        target = work / 'screenshot.png'
        link = work / 'screenshot.png.tmp~'
        if os.name == 'nt':
            os.link(sentinel, link)
        else:
            link.symlink_to(sentinel)
        env = dict(os.environ, PaintCraft_CONFIG_DIR=str(work / 'config'))
        command = [str(exe), '--no-plugins', '--shot', str(target), '--size', '1000x600']
        run = subprocess.run(command, cwd=work, env=env, capture_output=True, timeout=45)
        results.append({'test':'atomic-write-preexisting-link', 'ok': sentinel.read_bytes() == SENTINEL,
                        'exit':run.returncode,'sentinel_bytes':sentinel.stat().st_size})
        results.append({'test':'atomic-write-valid-output', 'ok':run.returncode==0 and target.is_file() and target.read_bytes().startswith(b'\x89PNG')})
        # A private existing image must not become readable by other users on save.
        if os.name != 'nt':
            private = work / 'private.png'
            private.write_bytes(SENTINEL)
            private.chmod(0o600)
            run = subprocess.run([str(exe), '--no-plugins', '--shot', str(private), '--size', '1000x600'],
                                 cwd=work,env=env,capture_output=True,timeout=45)
            mode = stat.S_IMODE(private.stat().st_mode)
            results.append({'test':'atomic-write-private-permissions','ok':run.returncode==0 and mode & 0o077 == 0,'mode':oct(mode)})
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out',default='build/PaintCraft-security/after')
    parser.add_argument('--label',default='regression')
    parser.add_argument('--linux-case',action='store_true')
    args=parser.parse_args()
    output=(ROOT/args.out).resolve()
    if args.linux_case:
        with tempfile.TemporaryDirectory(prefix='PaintCraft-security-') as directory:
            result=native_cases(output/'linux'/'PaintCraft',Path(directory))
        print(json.dumps(result))
        return 0
    result=native_cases(output/'win'/'PaintCraft.exe',output/'security-cases-win')
    for row in result: row['platform']='win'
    script=subprocess.check_output(['wsl.exe','-d','Ubuntu-24.04','-e','wslpath','-a',str(Path(__file__).resolve())],text=True).strip()
    converted=subprocess.check_output(['wsl.exe','-d','Ubuntu-24.04','-e','wslpath','-a',str(output)],text=True).strip()
    # Linux permissions and symlinks need a Linux filesystem, not a Windows mount.
    command=['wsl.exe','-d','Ubuntu-24.04','--cd',str(ROOT),'-e','python3',script,'--linux-case','--out',converted]
    run=subprocess.run(command,capture_output=True,text=True,timeout=150)
    if run.returncode: raise RuntimeError(run.stdout+run.stderr)
    linux=json.loads(run.stdout)
    for row in linux: row['platform']='linux'
    result+=linux
    target=output/(args.label+'-security.json')
    target.write_bytes((json.dumps(result,indent=2)+'\n').encode())
    for row in result: print(('PASS' if row['ok'] else 'FAIL'),row)
    return int(any(not row['ok'] for row in result))


if __name__=='__main__': sys.exit(main())
