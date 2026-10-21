"""tls_interop.py -- netio's TLS 1.3 against OpenSSL, curl and Python's ssl module.

    python tools/tls_interop.py            (Linux / WSL: needs openssl and curl)

The certificates come from `coretest --tls-pki` (Ed25519, own CA).  Checked:
  our server  <- openssl s_client (with and without a client certificate), curl --cacert,
                 Python ssl, a client that offers P-256 first (HelloRetryRequest)
  our client  -> openssl s_server (with and without requiring a client certificate)
  refusals    curl without the CA, a server requiring a certificate the client lacks"""
import os
import socket
import ssl
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_port(port, timeout=10):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        try:
            socket.create_connection(("127.0.0.1", port), 0.5).close()
            return True
        except OSError:
            time.sleep(0.1)
    return False


def main():
    if os.name == "nt":
        wsl = "/mnt/" + str(ROOT)[0].lower() + str(ROOT)[2:].replace("\\", "/")
        return subprocess.run(["wsl.exe", "-d", "Ubuntu-24.04", "-e", "sh", "-c", f"cd '{wsl}' && python3 tools/tls_interop.py"]).returncode
    exe = str(ROOT / "build" / "linux" / "coretest")
    d = Path(tempfile.mkdtemp(prefix="tls-interop-"))
    subprocess.run([exe, "--tls-pki", str(d)], check=True)
    results = []

    def check(name, ok, detail=""):
        results.append(ok)
        print(f"{'ok  ' if ok else 'FAIL'} {name}" + ("" if ok or not detail else f": {detail.strip()[:300]}"))

    ossl = ["openssl"]
    common = ["-ciphersuites", "TLS_CHACHA20_POLY1305_SHA256", "-groups", "X25519"]
    for mtls in (False, True):
        port = free_port()
        srv = subprocess.Popen([exe, "--tls-serve", str(port)] + (["--mtls"] if mtls else []), stderr=subprocess.PIPE)
        try:
            wait_port(port)
            tag = " (mTLS)" if mtls else ""
            # openssl s_client
            cmd = ossl + ["s_client", "-connect", f"127.0.0.1:{port}", "-servername", "localhost", "-CAfile", str(d / "ca.pem"), "-verify_return_error", "-tls1_3", "-quiet"] + common
            if mtls:
                cmd += ["-cert", str(d / "client.pem"), "-key", str(d / "client.key")]
            r = subprocess.run(cmd, input=b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n", capture_output=True, timeout=20)
            check("openssl s_client -> our server" + tag, b"JFLib tls ok" in r.stdout and (not mtls or b"agent-1" in r.stdout), r.stderr.decode(errors="replace"))
            # curl
            cmd = ["curl", "-sS", "--tlsv1.3", "--cacert", str(d / "ca.pem"), f"https://localhost:{port}/"]
            if mtls:
                cmd += ["--cert", str(d / "client.pem"), "--key", str(d / "client.key")]
            r = subprocess.run(cmd, capture_output=True, timeout=20)
            check("curl -> our server" + tag, b"JFLib tls ok" in r.stdout, r.stderr.decode(errors="replace"))
            # Python ssl
            try:
                ctx = ssl.create_default_context(cafile=str(d / "ca.pem"))
                ctx.minimum_version = ssl.TLSVersion.TLSv1_3
                if mtls:
                    ctx.load_cert_chain(str(d / "client.pem"), str(d / "client.key"))
                with socket.create_connection(("127.0.0.1", port), 5) as s, ctx.wrap_socket(s, server_hostname="localhost") as t:
                    t.sendall(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n")
                    data = b""
                    while chunk := t.recv(4096):
                        data += chunk
                    version = t.version()
                check("python ssl -> our server" + tag, b"JFLib tls ok" in data and version == "TLSv1.3", repr((version, data[:80])))
            except Exception as e:  # noqa: BLE001
                check("python ssl -> our server" + tag, False, repr(e))
            if not mtls:
                # HelloRetryRequest: P-256 key share first
                r = subprocess.run(ossl + ["s_client", "-connect", f"127.0.0.1:{port}", "-servername", "localhost", "-CAfile", str(d / "ca.pem"), "-verify_return_error", "-tls1_3", "-quiet",
                                           "-ciphersuites", "TLS_CHACHA20_POLY1305_SHA256", "-groups", "P-256:X25519"],
                                   input=b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n", capture_output=True, timeout=20)
                check("HelloRetryRequest (P-256 offered first)", b"JFLib tls ok" in r.stdout, r.stderr.decode(errors="replace"))
                r = subprocess.run(["curl", "-sS", "--tlsv1.3", f"https://localhost:{port}/"], capture_output=True, timeout=20)
                check("curl without our CA is refused", r.returncode != 0)
            else:
                r = subprocess.run(["curl", "-sS", "--tlsv1.3", "--cacert", str(d / "ca.pem"), f"https://localhost:{port}/"], capture_output=True, timeout=20)
                check("no client certificate is refused", b"JFLib tls ok" not in r.stdout)
        finally:
            srv.terminate()
            srv.wait(5)
    # our client -> openssl s_server
    for mtls in (False, True):
        port = free_port()
        # (no -naccept: the port probe is a connection too)
        cmd = ossl + ["s_server", "-accept", str(port), "-cert", str(d / "server.pem"), "-key", str(d / "server.key"), "-tls1_3", "-www"] + common
        if mtls:
            cmd += ["-Verify", "1", "-CAfile", str(d / "ca.pem"), "-verify_return_error"]
        srv = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            wait_port(port)
            r = subprocess.run([exe, "--tls-get", str(port), "--host", "localhost"] + (["--mtls"] if mtls else []), capture_output=True, timeout=20)
            ok = b"TLSv1.3" in r.stdout and b"TLS_CHACHA20_POLY1305_SHA256" in r.stdout
            check("our client -> openssl s_server" + (" (mTLS)" if mtls else ""), ok, (r.stderr + r.stdout[:200]).decode(errors="replace"))
        finally:
            srv.terminate()
            srv.wait(5)
    port = free_port()
    srv = subprocess.Popen(ossl + ["s_server", "-accept", str(port), "-cert", str(d / "server.pem"), "-key", str(d / "server.key"), "-tls1_3", "-www",
                                   "-Verify", "1", "-CAfile", str(d / "ca.pem"), "-verify_return_error"] + common, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        wait_port(port)
        r = subprocess.run([exe, "--tls-get", str(port), "--host", "localhost"], capture_output=True, timeout=20)
        check("our client without certificate is refused by openssl", b"TLSv1.3" not in r.stdout)
    finally:
        srv.terminate()
        srv.wait(5)
    n = len(results)
    print(f"tls_interop: {sum(results)} of {n} passed")
    return 0 if all(results) else 1


if __name__ == "__main__":
    sys.exit(main())
