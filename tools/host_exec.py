"""Host-aware native execution; WSL children die when their owner's input pipe closes."""
import os
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def linux_path(path):
    value = str(Path(path).resolve())
    return '/mnt/' + value[0].lower() + value[2:].replace('\\', '/') if value[1:2] == ':' else value


def run_linux(args, cwd=ROOT, timeout=600, env=None):
    if os.name != 'nt':
        return subprocess.run(list(map(str, args)), cwd=cwd, timeout=timeout, env=env,
                              capture_output=True, text=True, encoding='utf-8', errors='replace')
    command = ['wsl.exe', '-d', os.environ.get('jfgui_WSL_DISTRO', 'Ubuntu-24.04'),
               '-e', 'python3', linux_path(ROOT / 'tools/linux_task.py'), '--watch-stdin',
               '--cwd', linux_path(cwd), 'run', *map(str, args)]
    # communicate() would close stdin before completion. Keep the lifetime pipe open
    # while temporary files drain unlimited stdout/stderr without pipe deadlocks.
    with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
        p = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=out, stderr=err,
                             env=env, creationflags=subprocess.CREATE_NO_WINDOW)
        try:
            p.wait(timeout=timeout)
        finally:
            p.stdin.close()
            try:
                p.wait(timeout=10)
            except subprocess.TimeoutExpired:
                p.kill()
                p.wait()
        out.seek(0)
        err.seek(0)
        return subprocess.CompletedProcess(command, p.returncode,
                                           out.read().decode('utf-8', 'replace'),
                                           err.read().decode('utf-8', 'replace'))


def run_native(target, exe, args, timeout=600, env=None):
    if target == 'linux':
        return run_linux([linux_path(exe), *args], timeout=timeout, env=env)
    if os.name != 'nt':
        raise RuntimeError('Windows-Programme können auf diesem Host nicht ausgeführt werden.')
    return subprocess.run([str(exe), *args], cwd=ROOT, timeout=timeout, env=env,
                          capture_output=True, text=True, encoding='utf-8', errors='replace',
                          creationflags=subprocess.CREATE_NO_WINDOW)
