"""ssh_interop.py -- netio's SSH transport against OpenSSH's client.

    python tools/ssh_interop.py [--distro Ubuntu-24.04]

coretest --ssh-serve runs a small server on netio::ssh (key exchange, encryption, logins and
exec are the library's); OpenSSH's ssh logs in with a key made by ssh-keygen and with a password
(SSH_ASKPASS, Linux), and runs the test commands.  Windows: ssh.exe against
build/win/coretest.exe; Linux (WSL): ssh against build/linux/coretest.  The fingerprint the
library computes must be the one ssh shows, the strict key exchange must be in use, and the
library must read ssh-keygen's private key file."""
import argparse
import hashlib
import os
import shutil
import socket
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SSH_OPTS = ["-F", "none", "-o", "StrictHostKeyChecking=yes", "-o", "KexAlgorithms=curve25519-sha256",
            "-o", "Ciphers=chacha20-poly1305@openssh.com", "-o", "HostKeyAlgorithms=ssh-ed25519", "-o", "ConnectTimeout=10"]


def win_to_wsl(p):
    p = str(p)
    return "/mnt/" + p[0].lower() + p[2:].replace("\\", "/")


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


class Side:
    """where the programs run: Windows directly, or Linux through wsl.exe"""

    def __init__(self, name, distro, tmp):
        self.name = name
        self.linux = name == "linux"
        self.pre = ["wsl.exe", "-d", distro, "--"] if self.linux else []
        self.coretest = win_to_wsl(ROOT / "build" / "linux" / "coretest") if self.linux else str(ROOT / "build" / "win" / "coretest.exe")
        self.ssh = "ssh" if self.linux else "ssh.exe"
        self.keygen = "ssh-keygen" if self.linux else "ssh-keygen.exe"
        # Linux keeps its files in the distribution's /tmp: ssh refuses keys on /mnt/c (mode 0777)
        self.dir = f"/tmp/{tmp.name}" if self.linux else str(tmp / name)
        if self.linux:
            self.run(["mkdir", "-p", self.dir])
        else:
            Path(self.dir).mkdir()

    def path(self, n):
        return f"{self.dir}/{n}" if self.linux else str(Path(self.dir) / n)

    def run(self, args, **kw):
        kw.setdefault("capture_output", True)
        kw.setdefault("timeout", 120)
        if "input" not in kw:
            kw.setdefault("stdin", subprocess.DEVNULL)
        return subprocess.run(self.pre + args, **kw)

    def write(self, name, text):
        if self.linux:
            self.run(["sh", "-c", f"cat > {self.path(name)}"], input=text.encode(), stdin=None)
        else:
            Path(self.path(name)).write_text(text, newline="\n")

    def read(self, name):
        if self.linux:
            return self.run(["cat", self.path(name)]).stdout.decode()
        return Path(self.path(name)).read_text()

    def cleanup(self):
        if self.linux:
            self.run(["rm", "-rf", self.dir])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--distro", default="Ubuntu-24.04")
    a = ap.parse_args()
    passed = failed = 0

    def check(cond, what):
        nonlocal passed, failed
        print(("  ok   " if cond else "  FAIL ") + what)
        passed += bool(cond)
        failed += not cond

    tmp = Path(tempfile.mkdtemp(prefix="ssh-interop-"))
    try:
        for name in ("windows", "linux"):
            print(f"OpenSSH client ({name}) -> netio::ssh server")
            s = Side(name, a.distro, tmp)
            port = free_port()
            key = s.path("id")
            s.run([s.keygen, "-q", "-t", "ed25519", "-N", "", "-C", "", "-f", key], check=True)
            mine = s.run([s.coretest, "--ssh-key", key], text=True).stdout.split()
            check(mine[:2] == s.read("id.pub").split()[:2], "the library reads ssh-keygen's private key")
            srv = subprocess.Popen(s.pre + [s.coretest, "--ssh-serve", str(port), "--authorized", s.path("id.pub"), "--hostkey-out", s.path("host.pub")],
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                check(srv.stdout.readline().strip() == "ready", "test server up")
                host = s.read("host.pub").split()
                s.write("known", f"[127.0.0.1]:{port} {host[0]} {host[1]}\n")
                fp = s.run([s.keygen, "-lf", s.path("host.pub")], text=True).stdout.split()[1]
                base = [s.ssh] + SSH_OPTS + ["-o", f"UserKnownHostsFile={s.path('known')}", "-p", str(port)]
                withkey = base + ["-o", "BatchMode=yes", "-i", key, "-o", "PreferredAuthentications=publickey", "bc@127.0.0.1"]

                r = s.run(withkey + ["echo hallo von openssh"], text=True)
                check(r.returncode == 0 and r.stdout == "hallo von openssh\n", f"key login + exec ({r.returncode} {r.stderr.strip()[-200:]})")
                data = os.urandom(5 << 20)
                r = s.run(withkey + ["sha256"], input=data)
                want = f"{hashlib.sha256(data).hexdigest()} {len(data)}\n".encode()
                check(r.returncode == 0 and r.stdout == want, f"5 MiB through stdin ({r.returncode} {r.stderr[-200:]!r})")
                r = s.run(withkey + ["fail"], text=True)
                check(r.returncode == 3 and r.stderr.strip() == "boom", f"exit code and stderr ({r.returncode} {r.stderr.strip()!r})")
                r = s.run(base + ["-vvv", "-o", "BatchMode=yes", "-i", key, "bc@127.0.0.1", "echo x"], text=True)
                check(fp in r.stderr, f"ssh sees the fingerprint the library shows ({fp})")
                check("will use strict KEX ordering" in r.stderr, "strict key exchange in use (Terrapin)")
                # OpenSSH checks the library's host key signature: another known key is refused
                s.write("known2", f"[127.0.0.1]:{port} ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIPrRGcKDw5kO91keC8u9HuXbIqD7TVyG+r6hPaF29HAv\n")
                r = s.run([s.ssh] + SSH_OPTS + ["-o", f"UserKnownHostsFile={s.path('known2')}", "-o", "BatchMode=yes", "-p", str(port), "-i", key, "bc@127.0.0.1", "echo x"], text=True)
                check(r.returncode != 0 and "HOST IDENTIFICATION HAS CHANGED" in r.stderr.upper(), "OpenSSH refuses a changed host key")
                other = s.path("other")
                s.run([s.keygen, "-q", "-t", "ed25519", "-N", "", "-f", other], check=True)
                r = s.run(base + ["-o", "BatchMode=yes", "-i", other, "-o", "PreferredAuthentications=publickey", "bc@127.0.0.1", "echo x"], text=True)
                check(r.returncode == 255 and "denied" in r.stderr.lower(), "unknown key refused")
                if s.linux:
                    s.write("askpass.sh", "#!/bin/sh\necho geheim-123\n")
                    s.run(["chmod", "700", s.path("askpass.sh")], check=True)
                    r = s.run(["env", f"SSH_ASKPASS={s.path('askpass.sh')}", "SSH_ASKPASS_REQUIRE=force", "DISPLAY=:0"] + base +
                              ["-o", "PreferredAuthentications=password", "-o", "PubkeyAuthentication=no", "bc@127.0.0.1", "echo mit passwort"], text=True)
                    check(r.returncode == 0 and r.stdout == "mit passwort\n", f"password login ({r.returncode} {r.stderr.strip()[-200:]})")
            finally:
                srv.kill()
                srv.wait()
                if s.linux:
                    s.run(["pkill", "-f", f"ssh-serve {port}"])
                s.cleanup()
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    print(f"ssh_interop: {passed} passed, {failed} failed")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
