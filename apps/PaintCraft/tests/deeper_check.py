"""Bounded local regression checks. Run from RustLib after building PaintCraft/codectest.

python apps/PaintCraft/tests/deeper_check.py codecs --out build/PaintCraft-deeper
python apps/PaintCraft/tests/deeper_check.py files --out build/PaintCraft-deeper
python apps/PaintCraft/tests/deeper_check.py settings --out build/PaintCraft-deeper
Decoder mutations run in Linux with 512 MiB address space, 20 CPU seconds and a
30-second wall limit per batch. Seed and failing batch bytes are retained.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import json
import os
from pathlib import Path
import random
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
SEED = 20261011
sys.path.insert(0, str(ROOT / 'tools'))


def wsl_path(path):
    return subprocess.check_output(['wsl.exe', '-d', 'Ubuntu-24.04', '-e',
                                    'wslpath', '-a', str(path)], text=True).strip()


def save_results(out, name, rows):
    path = out / (name + '.json')
    path.write_text(json.dumps(rows, indent=2) + '\n', encoding='utf-8')


def codecs(out):
    import imgtest_common as it
    import pdn_check as pdn
    import png_check as png

    linux_exe = wsl_path(out / 'linux/codectest')
    # Keep the corpus small; the large performance fixture is outside this run.
    samples = out / 'pdn-fixtures'
    samples.mkdir(exist_ok=True)
    for p in pdn.SAMPLES.glob('*.pdn'):
        if p.stat().st_size <= 2 * 1024 * 1024:
            shutil.copy2(p, samples / p.name)
    pdn.SAMPLES = samples

    class BoundedRunner(it.Runner):
        def cmd(self, args):
            items = ['--shot', self.rel(self.work / 'unused.png'), *self.extra, *args]
            if self.target == 'win':
                return [str(self.exe), *items]
            limits = ('import os,resource,sys; '
                      'resource.setrlimit(resource.RLIMIT_AS,(536870912,536870912)); '
                      'resource.setrlimit(resource.RLIMIT_CPU,(20,21)); '
                      'resource.setrlimit(resource.RLIMIT_CORE,(0,0)); '
                      'os.execv(sys.argv[1],sys.argv[1:])')
            return ['wsl.exe', '-d', 'Ubuntu-24.04', '--cd', str(ROOT), '-e',
                    'timeout', '--signal=KILL', '30s', 'python3', '-c', limits, linux_exe, *items]

        def run(self, args, timeout=40):
            env = dict(os.environ, PaintCraft_CONFIG_DIR=str(out / 'codec-config'))
            command = self.cmd(args)
            try:
                result = subprocess.run(command, cwd=ROOT, env=env, capture_output=True,
                                        text=True, encoding='utf-8', errors='replace', timeout=40)
            except subprocess.TimeoutExpired:
                result = subprocess.CompletedProcess(command, -999, '', 'Batch exceeded 40-second host timeout')
            if result.returncode:
                failure = out / 'failures'
                failure.mkdir(exist_ok=True)
                source = ROOT / args[args.index('--batch') + 1]
                shutil.copy2(source, failure / (self.target + '-' + source.name))
                (failure / (self.target + '-' + source.stem + '.log')).write_text(
                    result.stdout + result.stderr, encoding='utf-8')
            return result

        def batch(self, recs, crashes=None):
            if len(recs) <= 32:
                return super().batch(recs, crashes)
            results = []
            for start in range(0, len(recs), 32):
                results.extend(super().batch(recs[start:start + 32], crashes))
            return results

    rows = []

    def record(s):
        it.report([s], s.name)
        rows.append(dict(target=s.target, suite=s.name, passed=s.passed, failed=s.failed,
                         seconds=round(s.dt, 3), messages=s.msgs, notes=s.notes))
        save_results(out, 'deeper-codecs', dict(seed=SEED, suites=rows))
        sys.stdout.flush()

    for target in ['win', 'linux']:
        runner = BoundedRunner(target, out, work_name='bounded-codecs')
        for name in ['samples', 'roundtrip', 'flat', 'legacy', 'v3']:
            s = getattr(pdn, 'suite_' + name)(it, runner, target)
            s.name = 'pdn-' + name
            record(s)
        for n, name in enumerate(['straight', 'meta', 'apng']):
            s = it.Suite('png-' + name, target)
            it.run_suite(runner, s, getattr(png, 'suite_' + name)(random.Random(SEED + n)))
            record(s)
        if target != 'linux':
            continue
        mutants = pdn.nrbf_mutants
        # Retain structural limits/references/cycles; omit the million-object and
        # 10,000-level stress fixtures. Depth 60/64/65 still tests the depth boundary.
        pdn.nrbf_mutants = lambda: [r for r in mutants()
                                   if r[0] not in {'1M+1 objects', 'nesting 10000'}]
        try:
            s = pdn.suite_nrbf(it, runner, target)
        finally:
            pdn.nrbf_mutants = mutants
        s.name = 'pdn-nrbf-bounds'
        record(s)
        s = pdn.suite_fuzz(it, runner, target, 200, SEED)
        s.name = 'pdn-mutations'
        record(s)
        s = it.Suite('png-mutations', target)
        recs = png.suite_fuzz(random.Random(SEED), 300)
        for r in recs:
            r.check = png.fuzz_check
        it.run_suite(runner, s, recs)
        record(s)
    return int(any(r['failed'] for r in rows))


def native_files(out, platform):
    base = out / 'concurrent-files'
    base.mkdir(exist_ok=True)
    if platform == 'linux':
        work = Path(tempfile.mkdtemp(prefix='PaintCraft-deeper-'))
        assert work.resolve().parent == Path(tempfile.gettempdir()).resolve()
    else:
        work = Path(tempfile.mkdtemp(prefix=platform + '-', dir=base))
        assert work.resolve().is_relative_to(out.resolve())
    exe = out / platform / ('PaintCraft.exe' if platform == 'win' else 'PaintCraft')
    rows = []

    def run(path, theme, tag):
        env = dict(os.environ, PaintCraft_CONFIG_DIR=str(work / ('config-' + tag)))
        result = subprocess.run([str(exe), '--no-plugins', '--theme', theme, '--size', '400x300',
                                 '--shot', str(path)], cwd=work, env=env, capture_output=True, timeout=30)
        (work / (tag + '.log')).write_bytes(result.stdout + result.stderr)
        return result.returncode

    hashes = set()
    for theme in ['light', 'dark']:
        p = work / (theme + '.png')
        code = run(p, theme, 'reference-' + theme)
        if code or not p.exists():
            raise RuntimeError('Could not generate reference PNG')
        hashes.add(hashlib.sha256(p.read_bytes()).hexdigest())
    target = work / 'shared.png'
    sentinel = work / 'shared.png.tmp~'
    sentinel.write_bytes(b'Independent file must survive.\n')
    with ThreadPoolExecutor(max_workers=4) as pool:
        for round_no in range(5):
            futures = [pool.submit(run, target, ['light', 'dark'][n % 2], f'round-{round_no}-{n}')
                       for n in range(4)]
            codes = [f.result() for f in futures]
            digest = hashlib.sha256(target.read_bytes()).hexdigest() if target.exists() else None
            rows.append(dict(test='concurrent-complete-write', round=round_no, platform=platform,
                             ok=all(c == 0 for c in codes) and digest in hashes, exits=codes))
    rows.append(dict(test='concurrent-preserves-unrelated-temp', platform=platform,
                     ok=sentinel.read_bytes() == b'Independent file must survive.\n'))
    rows.append(dict(test='concurrent-cleans-owned-temp', platform=platform,
                     ok=not list(work.glob('shared.png.tmp-*'))))
    # A directory cannot be replaced by a PNG. No temporary file should remain.
    blocked = work / 'directory.png'
    blocked.mkdir()
    code = run(blocked, 'light', 'invalid-target')
    rows.append(dict(test='failed-replace-keeps-directory', platform=platform,
                     ok=code != 0 and blocked.is_dir() and not list(work.glob('directory.png.tmp-*')), exit=code))
    artifacts = work
    if platform == 'linux':
        artifacts = base / work.name
        shutil.copytree(work, artifacts)
    for row in rows:
        row['artifacts'] = str(artifacts)
    return rows


def files(out, linux_child=False):
    if linux_child:
        print(json.dumps(native_files(out, 'linux')))
        return 0
    rows = native_files(out, 'win')
    proc = subprocess.run(['wsl.exe', '-d', 'Ubuntu-24.04', '-e', 'python3',
                           wsl_path(Path(__file__).resolve()), 'files', '--linux-child', '--out', wsl_path(out)],
                          capture_output=True, text=True, timeout=180)
    if proc.returncode:
        raise RuntimeError(proc.stdout + proc.stderr)
    rows += json.loads(proc.stdout)
    save_results(out, 'deeper-files', rows)
    for row in rows:
        print(('PASS' if row['ok'] else 'FAIL'), row)
    return int(any(not r['ok'] for r in rows))


def settings(out):
    import pn_settings_check as check
    rows = []
    for target in ['win', 'linux']:
        for name in ['malformed', 'env']:
            runner = check.Runner(target, out)
            runner.base = Path(tempfile.mkdtemp(prefix=f'settings-{target}-{name}-', dir=out))
            assert runner.base.resolve().is_relative_to(out.resolve())
            # Both checker functions create children of this fresh directory.
            getattr(check, 't_' + name)(runner)
            rows.append(dict(target=target, test=name, failed=runner.fails,
                             artifacts=str(runner.base)))
            print(target, name, 'failures:', runner.fails, flush=True)
    save_results(out, 'deeper-settings', rows)
    return int(any(row['failed'] for row in rows))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=['codecs', 'files', 'settings'])
    parser.add_argument('--out', default='build/PaintCraft-deeper')
    parser.add_argument('--linux-child', action='store_true')
    args = parser.parse_args()
    out = (ROOT / args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    if args.mode == 'codecs':
        return codecs(out)
    if args.mode == 'settings':
        return settings(out)
    return files(out, args.linux_child)


if __name__ == '__main__':
    sys.exit(main())
