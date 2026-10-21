"""Inspect generated packages and exercise Windows setup only in its isolated test mode."""
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import zlib

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from tools.host_exec import linux_path, run_linux


def main():
    jobs = json.loads((ROOT / 'build/manager-integration/release-results.json').read_text(encoding='utf-8'))
    for job in jobs:
        artifacts = job['artifacts']
        scratch = ROOT / 'build/manager-integration/package-check' / job['id']
        scratch.mkdir(parents=True, exist_ok=True)
        for artifact in artifacts:
            path = Path(artifact['path'])
            if artifact['kind'] == 'installer':
                blob = path.read_bytes()
                magic, stub, size = struct.unpack('<8sQQ', blob[-24:])
                assert magic == b'ASMINST1' and stub + size == len(blob) - 24
                payload = blob[stub:stub + size]
                assert payload[:4] == b'AIPK'
                variants = {}
                for i in range(struct.unpack_from('<I', payload, 4)[0]):
                    record = 8 + 128 * i
                    kind, length, original, flags, offset, _ = struct.unpack_from('<IIIIQQ', payload, record)
                    name = payload[record + 32:record + 80].split(b'\0', 1)[0].decode()
                    data = payload[offset:offset + length]
                    if flags & 1:
                        data = zlib.decompress(data)
                    assert len(data) == original
                    if kind == 2:
                        variants[name] = data
                for candidate in artifacts:
                    if candidate['kind'] == 'app' and candidate['target'] == 'win':
                        assert hashlib.sha256(variants[candidate['style']]).hexdigest() == candidate['sha256']
                if os.name == 'nt':
                    for style in variants:
                        destination = scratch / ('install-' + style)
                        script = scratch / 'install.uis'
                        script.write_text('click "Weiter"\nclick "Installieren"\nwait 500\nexpect "Fertigstellen" exists\n', encoding='utf-8')
                        env = dict(os.environ, ASMINST_SHELL_DIR=str(scratch / ('shell-' + style)))
                        result = subprocess.run([str(path), '--script', str(script), '--dir', str(destination),
                            '--style', style, '--verify-dirty'], env=env, cwd=ROOT, capture_output=True, timeout=30)
                        assert result.returncode == 0, result.stdout + result.stderr
                        assert (destination / (job['app'] + '.exe')).read_bytes() == variants[style]
                print('PASS Windows payload + isolated setup:', path, flush=True)
            elif artifact['kind'] == 'deb':
                for option in ('--info', '--contents'):
                    result = run_linux(['dpkg-deb', option, linux_path(path)])
                    assert result.returncode == 0, result.stderr
                extracted = scratch / 'deb-extracted'
                result = run_linux(['dpkg-deb', '--extract', linux_path(path), linux_path(extracted)])
                assert result.returncode == 0, result.stderr
                request = json.loads((path.parent / 'request.json').read_text(encoding='utf-8'))
                package = request['package'].get('deb_name', job['app'].lower())
                for candidate in artifacts:
                    if candidate['kind'] == 'app' and candidate['target'] == 'linux':
                        installed = extracted / 'usr/lib' / package / (job['app'] + '-' + candidate['style'])
                        assert hashlib.sha256(installed.read_bytes()).hexdigest() == candidate['sha256']
                print('PASS Debian metadata + extracted binaries:', path, flush=True)


if __name__ == '__main__':
    main()
