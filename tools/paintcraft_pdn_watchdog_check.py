"""A stalled original-API codec must exit when its editor heartbeat stops."""
import argparse
import os
from pathlib import Path
import struct
import subprocess
import time
import threading


def main():
    parser = argparse.ArgumentParser(__doc__)
    parser.add_argument('--host', required=True)
    parser.add_argument('--fixture', required=True)
    parser.add_argument('--paintnet', required=True)
    parser.add_argument('--out', default='build/paintcraft-next/watchdog-check')
    parser.add_argument('--wine', help='Wine executable on Linux; requires running this test inside Linux')
    parser.add_argument('--dotnet', default='dotnet')
    parser.add_argument('--prefix')
    a = parser.parse_args()
    out = Path(a.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    path = out / 'editor.watch'
    def mapped(path):
        path = str(Path(path).resolve())
        return 'Z:' + path.replace('/', '\\') if a.wine else path
    env = dict(os.environ, DOTNET_ReadyToRun='0', PAINTDOTNET_DIR=mapped(a.paintnet))
    if a.prefix:
        env['WINEPREFIX'] = a.prefix
    command = ([a.wine, a.dotnet] if a.wine else [a.dotnet]) + [mapped(a.host), mapped(a.fixture), 'FixtureFactory#0', '--decode', '--watchdog', mapped(path)]
    path.write_bytes(struct.pack('<Q', time.monotonic_ns()))
    proc = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env)
    ready = threading.Event()
    errors = []
    def read_errors():
        for line in proc.stderr:
            errors.append(line)
            if b'fixture-waiting' in line:
                ready.set()
    reader = threading.Thread(target=read_errors, daemon=True)
    reader.start()
    try:
        proc.stdin.write(b'SLOW' + struct.pack('<4I', 0x54465354, 1, 1, 0xff123456))
        proc.stdin.close()
        proc.stdin = None
        # Updating in place also verifies compatibility with the host's sharing mode.
        started = time.monotonic()
        end = started + 30
        ready_at = None
        with path.open('r+b', buffering=0) as heartbeat:
            while time.monotonic() < end:
                heartbeat.seek(0)
                heartbeat.write(struct.pack('<Q', time.monotonic_ns()))
                assert proc.poll() is None, 'Host exited while the editor was alive: ' + b''.join(errors).decode(errors='replace')
                if ready.is_set() and ready_at is None:
                    ready_at = time.monotonic()
                    end = ready_at + 7
                time.sleep(.2)
        assert ready_at is not None, 'Codec never reached its blocking operation'
        stopped = time.monotonic()
        proc.wait(timeout=12)
        elapsed = time.monotonic() - stopped
        stdout = proc.stdout.read()
        reader.join(timeout=5)
        stderr = b''.join(errors)
        (out / 'stderr.log').write_bytes(stderr)
        assert proc.returncode == 2 and not stdout, (proc.returncode, stderr.decode(errors='replace'))
        assert elapsed < 8, f'Watchdog exit took {elapsed:.2f}s'
        print(f'Heartbeat kept codec alive; stopped editor caused clean exit in {elapsed:.2f}s, without output.')
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()
        path.unlink(missing_ok=True)


if __name__ == '__main__':
    main()
