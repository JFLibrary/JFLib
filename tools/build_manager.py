#!/usr/bin/env python3
"""jfguiLib Build Manager: --stdio service or an internally supervised --job worker."""
import argparse
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / 'tools'))


def main():
    for stream in (sys.stdout, sys.stderr):
        stream.reconfigure(encoding='utf-8', errors='replace')
    p = argparse.ArgumentParser(description=__doc__)
    modes = p.add_mutually_exclusive_group(required=True)
    modes.add_argument('--stdio', action='store_true')
    modes.add_argument('--job')
    p.add_argument('--wsl-distro', default='Ubuntu-24.04')
    a = p.parse_args()
    os.chdir(ROOT)
    if a.job:
        if sys.stdin.buffer.readline() != b'go\n':
            return 1
        # A killed server closes this pipe. Linux workers have their own session,
        # so they must explicitly terminate that session on owner disappearance.
        import signal
        import threading
        def owner_lifetime():
            while sys.stdin.buffer.read(4096):
                pass
            if os.name != 'nt':
                os.killpg(os.getpgrp(), signal.SIGKILL)
            else:
                os._exit(1)
        threading.Thread(target=owner_lifetime, daemon=True).start()
        from buildmanager.runner import worker
        code = worker(a.job)
        sys.stdout.flush()
        sys.stderr.flush()
        # Avoid interpreter teardown racing the blocked owner-lifetime reader.
        os._exit(code)
    from buildmanager.server import serve
    return serve(a.wsl_distro)


if __name__ == '__main__':
    sys.exit(main())
