"""Host-aware native execution; WSL children die when their owner's input pipe closes.

A Linux program of another processor (docs/ARM64.md: an ARM64 build on an x86-64 PC) starts through
QEMU's user-mode emulation, qemu-<arch> in WSL: static programs only, WSL has no glibc for ARM64.  The
processor comes from the program's own header, so every caller gets this without asking for it."""
import os
import platform
import subprocess
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ELF_MACHINE = {0x3E: 'x86_64', 0xB7: 'aarch64'}
PE_MACHINE = {0x8664: 'x86_64', 0xAA64: 'aarch64'}


def exe_arch(exe):
    """the processor a program is built for, from its ELF or PE header (None: unknown, not a program)"""
    try:
        with open(exe, 'rb') as f:
            head = f.read(1024)
    except OSError:
        return None
    if head[:4] == b'\x7fELF':
        return ELF_MACHINE.get(int.from_bytes(head[18:20], 'little'))
    pe = int.from_bytes(head[0x3C:0x40], 'little') if head[:2] == b'MZ' else len(head)
    if head[pe:pe + 4] == b'PE\0\0':
        return PE_MACHINE.get(int.from_bytes(head[pe + 4:pe + 6], 'little'))
    return None


def host_arch():
    """the processor of this machine (and so of its WSL); on Windows the native one, also for a Python that runs
    under the x64 emulation of Windows on ARM (platform.machine() answers AMD64 there)"""
    if os.name == 'nt':
        import ctypes
        process, native = ctypes.c_ushort(), ctypes.c_ushort()
        try:
            k32 = ctypes.windll.kernel32
            k32.GetCurrentProcess.restype = ctypes.c_void_p
            k32.IsWow64Process2.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_ushort), ctypes.POINTER(ctypes.c_ushort)]
            if k32.IsWow64Process2(k32.GetCurrentProcess(), ctypes.byref(process), ctypes.byref(native)):
                if native.value in PE_MACHINE:
                    return PE_MACHINE[native.value]
        except (AttributeError, OSError):    # before Windows 10 1709
            pass
    m = platform.machine().lower()
    return {'amd64': 'x86_64', 'x64': 'x86_64', 'arm64': 'aarch64'}.get(m, m)


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
        timed_out = False
        try:
            p.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
        finally:
            p.stdin.close()
            try:
                p.wait(timeout=10)
            except subprocess.TimeoutExpired:
                p.kill()
                p.wait()
        out.seek(0)
        err.seek(0)
        stdout, stderr = out.read().decode('utf-8', 'replace'), err.read().decode('utf-8', 'replace')
        if timed_out:       # as subprocess.run: the program is gone, what it wrote comes along
            raise subprocess.TimeoutExpired(command, timeout, output=stdout, stderr=stderr)
        return subprocess.CompletedProcess(command, p.returncode, stdout, stderr)


class LinuxRunner:
    """one WSL process that runs Linux programs one after the other for one thread (tools/linux_task.py serve):
    what run_linux does without a wsl.exe and two Pythons per program (build_rs.py test, section "test-jobs").
    run() answers as run_linux; RuntimeError when the process is gone (the caller can fall back to run_linux)"""

    def __init__(self, cwd=ROOT):
        import queue
        import threading
        self.p = subprocess.Popen(
            ['wsl.exe', '-d', os.environ.get('jfgui_WSL_DISTRO', 'Ubuntu-24.04'), '-e', 'python3',
             linux_path(ROOT / 'tools/linux_task.py'), '--cwd', linux_path(cwd), 'serve'],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0))
        self.lines = queue.Queue()

        def read():
            for line in iter(self.p.stdout.readline, b''):
                self.lines.put(line)
            self.lines.put(b'')
        threading.Thread(target=read, daemon=True).start()

    def run(self, args, timeout=600):
        import json
        import queue
        try:
            self.p.stdin.write((json.dumps({'args': list(map(str, args)), 'timeout': timeout}) + '\n').encode('utf-8'))
            self.p.stdin.flush()
            line = self.lines.get(timeout=(timeout or 86400) + 60)
        except (OSError, queue.Empty) as e:
            self.close()
            raise RuntimeError(f'linux_task serve: {e!r}') from e
        if not line:
            self.close()
            raise RuntimeError('linux_task serve: the WSL process ended')
        try:
            a = json.loads(line.decode('utf-8'))
        except ValueError as e:     # (UnicodeDecodeError too) not our answer: wsl.exe's own message (UTF-16)
            self.close()
            raise RuntimeError(f'linux_task serve: {line[:200]!r}') from e
        if a['timeout']:
            raise subprocess.TimeoutExpired(args, timeout, output=a['out'], stderr=a['err'])
        return subprocess.CompletedProcess(args, a['code'], a['out'], a['err'])

    def close(self):
        try:
            self.p.stdin.close()
        except OSError:
            pass
        try:
            self.p.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.p.kill()
            self.p.wait()


def run_native(target, exe, args, timeout=600, env=None, gui=False):
    """run a built program of target here; gui: it opens windows (the Mac from SSH starts it in the
    logged-in session, tools/apple.py gui_run); a Linux program for another processor runs under
    qemu (docs/ARM64.md), Mach-O programs have no known arch here and run as they are"""
    arch = exe_arch(exe)
    if arch not in (None, host_arch()):
        if target != 'linux':
            raise RuntimeError(f'{Path(exe).name} is a Windows program for {arch}: it only starts on such a machine '
                               '(docs/ARM64.md, windows-11-arm).')
        return run_linux([f'qemu-{arch}', linux_path(exe), *args], timeout=timeout, env=env)

    if target == 'linux':
        return run_linux([linux_path(exe), *args], timeout=timeout, env=env)
    if target == 'mac':                     # docs/APPLE.md: on the Mac itself
        if gui and os.environ.get('SSH_CONNECTION'):
            from tools.apple import gui_run
            return gui_run(exe, args, timeout=timeout, env=env)
        return subprocess.run([str(exe), *args], cwd=ROOT, timeout=timeout, env=env,
                              capture_output=True, text=True, encoding='utf-8', errors='replace')
    if target == 'ios-sim':                 # in a simulator without a window (tools/apple.py)
        from tools.apple import sim_run
        return sim_run(exe, args, timeout=timeout)
    if os.name != 'nt':
        raise RuntimeError('Windows-Programme können auf diesem Host nicht ausgeführt werden.')
    return subprocess.run([str(exe), *args], cwd=ROOT, timeout=timeout, env=env,
                          capture_output=True, text=True, encoding='utf-8', errors='replace',
                          creationflags=subprocess.CREATE_NO_WINDOW)
