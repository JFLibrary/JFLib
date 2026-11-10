"""safari_tunnel.py -- Safari on a Mac for a browser test that runs on this machine (docs/APPLE.md AP1).

    python tools/tests/safari_tunnel.py USER@MAC -- node tools/tests/web_apps.mjs build/safari-web --engine=safari --port=8720

One SSH connection starts safaridriver on the Mac and carries both directions: the driver's port
comes here (-L), and the test's HTTP server here reaches Safari there under the same port (-R), so
the pages load from http://127.0.0.1:PORT on both sides.  The command gets SAFARIDRIVER_URL; when it
ends, the connection and the driver go.  Once on the Mac: `safaridriver --enable` and, in Safari,
Develop -> "Allow Remote Automation" (Safari may need a restart after that: a session that times out
"while connecting to a Safari instance" is that).

    --driver-port N   safaridriver's port (default 4719)
    --port N          the test server's port to carry over (default 8720)
    SSH=PATH          the ssh program (default: Windows' own OpenSSH there, else ssh from PATH)
"""
from pathlib import Path
import os
import shutil
import subprocess
import sys
import time
import urllib.request


def ssh_program():
    if os.environ.get("SSH"):
        return os.environ["SSH"]
    win = Path(os.environ.get("SystemRoot", "C:/Windows")) / "System32" / "OpenSSH" / "ssh.exe"
    return str(win) if sys.platform == "win32" and win.exists() else (shutil.which("ssh") or "ssh")


def main():
    args = sys.argv[1:]
    if "--" not in args or args.index("--") < 1:
        print(__doc__)
        return 2
    cut = args.index("--")
    own, command = args[:cut], args[cut + 1:]
    host, driver, port = None, 4719, 8720
    i = 0
    while i < len(own):
        if own[i] == "--driver-port":
            driver, i = int(own[i + 1]), i + 2
        elif own[i] == "--port":
            port, i = int(own[i + 1]), i + 2
        else:
            host, i = own[i], i + 1
    if not host or not command:
        print(__doc__)
        return 2
    tunnel = subprocess.Popen([ssh_program(), "-q", "-o", "BatchMode=yes", "-o", "ExitOnForwardFailure=yes", "-o", "ServerAliveInterval=30",
                               "-L", f"{driver}:127.0.0.1:{driver}", "-R", f"{port}:127.0.0.1:{port}", host,
                               # a driver left on this port goes first; this one ends when the
                               # connection's input closes (without a terminal there is no hangup)
                               f"pkill -f 'safaridriver --port {driver}'; /usr/bin/safaridriver --port {driver} & p=$!; "
                               f"cat > /dev/null; kill $p"], stdin=subprocess.PIPE)
    url = f"http://127.0.0.1:{driver}"
    try:
        deadline = time.monotonic() + 30
        while True:
            try:
                urllib.request.urlopen(url + "/status", timeout=3).read()
                break
            except OSError:
                if tunnel.poll() is not None or time.monotonic() > deadline:
                    print(f"safari_tunnel: no safaridriver on {host} (ssh ended with {tunnel.poll()})", file=sys.stderr)
                    return 1
                time.sleep(0.5)
        return subprocess.run(command, env={**os.environ, "SAFARIDRIVER_URL": url}).returncode
    finally:
        tunnel.stdin.close()               # the driver on the Mac ends, then the connection
        try:
            tunnel.wait(timeout=10)
        except subprocess.TimeoutExpired:
            tunnel.kill()


if __name__ == "__main__":
    sys.exit(main())
