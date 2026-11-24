"""tls_interop.py -- netio's TLS 1.3 against OpenSSL, curl and Python's ssl module.

    python tools/tls_interop.py            (Linux / WSL: needs openssl and curl)
    python tools/tls_interop.py --core [--bin FILE]   netpkt::tls against this Python's ssl module, which
                                           is OpenSSL (coretest --tls-core-*); here, also on Windows

The certificates come from `coretest --tls-pki` (Ed25519, own CA).  Checked:
  our server  <- openssl s_client (with and without a client certificate), curl --cacert,
                 Python ssl, a client that offers P-256 first (HelloRetryRequest)
  our client  -> openssl s_server (with and without requiring a client certificate)
  refusals    curl without the CA, a server requiring a certificate the client lacks
Then for each key kind (`--key p256 / p384 / rsa / ed25519`: CA, server and client keys of that
kind), our server <- openssl s_client with each TLS 1.3 suite, each group (the hybrids with
ML-KEM, X25519, P-256, P-384, and a HelloRetryRequest), ALPN (its choice, no_application_protocol)
and a client certificate; curl --cacert.
Resumption and 0-RTT (`--tickets [--early N]`, `--resume N [--early]`):
  our server  <- openssl s_client -sess_out / -sess_in (each suite, after a HelloRetryRequest,
                 with a client certificate, another server name: no resumption), -early_data
                 (taken; not sent with a ticket of a server without 0-RTT), curl and Python ssl
                 reusing their session
  our client  -> openssl s_server (each suite, a HelloRetryRequest, a client certificate),
                 -early_data (the request taken as 0-RTT; refused after a HelloRetryRequest)"""
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


def core(exe):
    """netpkt::tls, the I/O-free core on a socket, with OpenSSL on the other side"""
    d = Path(tempfile.mkdtemp(prefix="tls-core-"))
    subprocess.run([exe, "--tls-core-pki", str(d)], check=True)
    results = []

    def check(name, ok, detail=""):
        results.append(bool(ok))
        print(f"{'ok  ' if ok else 'FAIL'} {name}" + ("" if ok or not detail else f": {str(detail).strip()[:300]}"))

    print(ssl.OPENSSL_VERSION)
    # the core as the server
    port = free_port()
    srv = subprocess.Popen([exe, "--tls-core-serve", str(port)], stdout=subprocess.DEVNULL)
    try:
        wait_port(port)
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        ctx.minimum_version = ssl.TLSVersion.TLSv1_3
        ctx.load_verify_locations(str(d / "ca.pem"))
        with socket.create_connection(("127.0.0.1", port), 5) as raw, ctx.wrap_socket(raw, server_hostname="web.lab") as s:
            check("OpenSSL's client: TLS 1.3", s.version() == "TLSv1.3", s.version())
            check("OpenSSL's client: the suite", s.cipher()[0] == "TLS_CHACHA20_POLY1305_SHA256", s.cipher())
            check("OpenSSL's client: our chain verified, our name", s.getpeercert()["subject"] == ((("commonName", "web.lab"),),), s.getpeercert())
            s.sendall(b"hallo")
            got = b""
            try:
                while True:
                    b = s.recv(4096)
                    if not b:
                        break
                    got += b
            except ssl.SSLZeroReturnError:
                pass
            check("OpenSSL's client: data both ways, its SNI, close_notify", got == b"sni=web.lab got=hallo", got)
        try:
            with socket.create_connection(("127.0.0.1", port), 5) as raw, ctx.wrap_socket(raw, server_hostname="other.lab"):
                check("OpenSSL's client refuses another name", False)
        except ssl.SSLCertVerificationError as e:
            check("OpenSSL's client refuses another name", "other.lab" in str(e), e)
        other = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
        other.minimum_version = ssl.TLSVersion.TLSv1_3
        try:
            with socket.create_connection(("127.0.0.1", port), 5) as raw, other.wrap_socket(raw, server_hostname="web.lab"):
                check("OpenSSL's client without our CA refuses", False)
        except ssl.SSLCertVerificationError as e:
            check("OpenSSL's client without our CA refuses", True, e)
    finally:
        srv.terminate()
        srv.wait(5)

    # the core as the client
    sctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    sctx.minimum_version = ssl.TLSVersion.TLSv1_3
    sctx.load_cert_chain(str(d / "leaf.pem"), str(d / "leaf.key"))

    def one(host, reply):
        with socket.create_server(("127.0.0.1", 0)) as l:
            port = l.getsockname()[1]
            p = subprocess.Popen([exe, "--tls-core-get", str(port), "--host", host], stdout=subprocess.PIPE)
            l.settimeout(10)
            conn, _ = l.accept()
            seen = b""
            try:
                with sctx.wrap_socket(conn, server_side=True) as s:
                    seen = s.recv(4096)
                    s.sendall(reply)
            except (ssl.SSLError, OSError) as e:
                seen = repr(e).encode()
            out, _ = p.communicate(timeout=10)
            return p.returncode, out, seen

    code, out, seen = one("web.lab", b"hallo von OpenSSL\n")
    check("OpenSSL's server: the core's request", seen == b"hallo vom Kern\n", seen)
    check("OpenSSL's server: its answer, its chain verified", code == 0 and out == b"hallo von OpenSSL\n", (code, out))
    code, out, seen = one("other.lab", b"nein")
    check("OpenSSL's server: the core refuses another name with an alert it reads", code == 2 and b"BAD_CERTIFICATE" in seen.upper(), (code, seen))
    print(f"tls_interop --core: {sum(results)} of {len(results)} passed")
    return 0 if all(results) else 1


def main():
    if "--core" in sys.argv:
        at = sys.argv.index("--bin") + 1 if "--bin" in sys.argv else 0
        exe = str(Path(sys.argv[at]).resolve()) if at else str(ROOT / "build" / ("win" if os.name == "nt" else "linux") / "coretest")
        return core(exe)
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
    server_matrix(exe, check)
    resumption(exe, d, check)
    n = len(results)
    print(f"tls_interop: {sum(results)} of {n} passed")
    return 0 if all(results) else 1


def server_matrix(exe, check):
    """our server with each kind of key against openssl s_client and curl"""
    req = b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n"
    for kind in ("p256", "p384", "rsa", "ed25519"):
        d = Path(tempfile.mkdtemp(prefix=f"tls-{kind}-"))
        subprocess.run([exe, "--tls-pki", str(d), "--key", kind], check=True)
        # the keys netio writes (PKCS #8) as OpenSSL reads them; RSA's CRT numbers checked
        r = subprocess.run(["openssl", "pkey", "-in", str(d / "client.key"), "-check", "-noout"], capture_output=True, text=True)
        check(f"[{kind}] openssl reads and checks netio's PKCS #8 key", r.returncode == 0 and "valid" in (r.stdout + r.stderr).lower(), r.stdout + r.stderr)
        for mtls in (False, True):
            port = free_port()
            srv = subprocess.Popen([exe, "--tls-serve", str(port), "--key", kind, "--alpn", "http/1.1"] + (["--mtls"] if mtls else []), stderr=subprocess.PIPE)
            try:
                wait_port(port)

                def s_client(*opts):
                    cmd = ["openssl", "s_client", "-connect", f"127.0.0.1:{port}", "-servername", "localhost", "-CAfile", str(d / "ca.pem"),
                           "-verify_return_error", "-ign_eof", *opts]
                    if mtls:
                        cmd += ["-cert", str(d / "client.pem"), "-key", str(d / "client.key")]
                    r = subprocess.run(cmd, input=req, capture_output=True, timeout=20)
                    return r.stdout.decode(errors="replace") + r.stderr.decode(errors="replace")

                tag = f"[{kind}{' mTLS' if mtls else ''}]"
                if mtls:
                    out = s_client("-tls1_3")
                    check(f"{tag} s_client with a client certificate", "JFLib tls ok agent-1" in out, out[-300:])
                    continue
                for suite in ("TLS_AES_128_GCM_SHA256", "TLS_AES_256_GCM_SHA384", "TLS_CHACHA20_POLY1305_SHA256"):
                    out = s_client("-tls1_3", "-ciphersuites", suite)
                    check(f"{tag} s_client {suite}", "JFLib tls ok" in out and f"Cipher is {suite}" in out, out[-300:])
                for group, hrr in (("X25519MLKEM768", ""), ("SecP256r1MLKEM768", ""), ("SecP384r1MLKEM1024", ""), ("X25519", ""), ("P-256", ""),
                                   ("P-384", ""), ("P-384", "X448:"), ("SecP384r1MLKEM1024", "X448:")):
                    out = s_client("-tls1_3", "-groups", hrr + group)
                    # OpenSSL names a hybrid as the negotiated group, an ECDHE group as the peer's key
                    seen = {"X25519": "Peer Temp Key: X25519", "P-256": "Peer Temp Key: ECDH, prime256v1", "P-384": "Peer Temp Key: ECDH, secp384r1"}.get(group, f"Negotiated TLS1.3 group: {group}")
                    check(f"{tag} s_client group {hrr}{group}" + (" (HelloRetryRequest)" if hrr else ""), "JFLib tls ok" in out and seen in out, out[-300:])
                out = s_client("-tls1_3", "-alpn", "foo,http/1.1")
                check(f"{tag} s_client ALPN: the server's protocol", "JFLib tls ok" in out and "ALPN protocol: http/1.1" in out, out[-300:])
                out = s_client("-tls1_3", "-alpn", "h2")
                check(f"{tag} s_client ALPN: none in common is refused", "JFLib tls ok" not in out and "no application protocol" in out.lower(), out[-300:])
                r = subprocess.run(["curl", "-sS", "--cacert", str(d / "ca.pem"), f"https://localhost:{port}/"], capture_output=True, timeout=20)
                check(f"{tag} curl", b"JFLib tls ok" in r.stdout, r.stderr.decode(errors="replace"))
            finally:
                srv.terminate()
                srv.wait(5)


def resumption(exe, d, check):
    """session tickets and 0-RTT both ways"""
    req = b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n"
    reqfile = d / "req.txt"
    reqfile.write_bytes(req)
    for mtls in (False, True):
        port = free_port()
        srv = subprocess.Popen([exe, "--tls-serve", str(port), "--tickets", "--early", "16384"] + (["--mtls"] if mtls else []), stderr=subprocess.PIPE)
        try:
            wait_port(port)

            def s_client(*opts, name="localhost", early=False):
                cmd = ["openssl", "s_client", "-connect", f"127.0.0.1:{port}", "-servername", name, "-CAfile", str(d / "ca.pem"), "-tls1_3", "-ign_eof", *opts]
                if mtls:
                    cmd += ["-cert", str(d / "client.pem"), "-key", str(d / "client.key")]
                if early:
                    cmd += ["-early_data", str(reqfile)]
                r = subprocess.run(cmd, input=b"" if early else req, capture_output=True, timeout=20)
                return r.stdout.decode(errors="replace") + r.stderr.decode(errors="replace")

            sess = str(d / "sess.pem")
            if mtls:
                s_client("-sess_out", sess)
                out = s_client("-sess_in", sess)
                check("resume: s_client -> our server with a client certificate", "Reused, TLSv1.3" in out and "JFLib tls ok agent-1 resumed" in out, out[-300:])
                continue
            for suite in ("TLS_AES_128_GCM_SHA256", "TLS_AES_256_GCM_SHA384", "TLS_CHACHA20_POLY1305_SHA256"):
                first = s_client("-ciphersuites", suite, "-sess_out", sess)
                out = s_client("-ciphersuites", suite, "-sess_in", sess)
                check(f"resume: s_client -> our server, {suite}", "New, TLSv1.3" in first and "Reused, TLSv1.3" in out and "JFLib tls ok resumed" in out, out[-300:])
            s_client("-sess_out", sess)
            out = s_client("-groups", "X448:P-256", "-sess_in", sess)
            check("resume: s_client -> our server after a HelloRetryRequest", "Reused, TLSv1.3" in out and "JFLib tls ok resumed" in out, out[-300:])
            s_client("-sess_out", sess)
            out = s_client("-sess_in", sess, name="other.lab")
            check("resume: a ticket of localhost is not taken for another server name", "New, TLSv1.3" in out and "JFLib tls ok" in out and "resumed" not in out, out[-300:])
            s_client("-sess_out", sess)
            out = s_client("-sess_in", sess, early=True)
            check("0-RTT: s_client -> our server: the request as early data, taken", "Early data was accepted" in out and "JFLib tls ok resumed" in out, out[-300:])
            # curl and Python ssl reuse their sessions
            r = subprocess.run(["curl", "-sS", "-H", "Connection: close", "--cacert", str(d / "ca.pem"), f"https://localhost:{port}/a", f"https://localhost:{port}/b"],
                               capture_output=True, timeout=20)
            check("resume: curl, the second request", r.stdout.count(b"JFLib tls ok") == 2 and b"JFLib tls ok resumed" in r.stdout, r.stdout.decode(errors="replace"))
            try:
                ctx = ssl.create_default_context(cafile=str(d / "ca.pem"))

                def get(session=None):
                    with socket.create_connection(("127.0.0.1", port), 5) as s, ctx.wrap_socket(s, server_hostname="localhost", session=session) as t:
                        t.sendall(req)
                        data = b""
                        while chunk := t.recv(4096):
                            data += chunk
                        return t.session, t.session_reused, data

                s1, _, _ = get()
                _, reused, data = get(s1)
                check("resume: python ssl", reused and b"JFLib tls ok resumed" in data, repr(data[:80]))
            except Exception as e:  # noqa: BLE001
                check("resume: python ssl", False, repr(e))
        finally:
            srv.terminate()
            srv.wait(5)
    # a server without 0-RTT: its tickets allow none, s_client sends none
    port = free_port()
    srv = subprocess.Popen([exe, "--tls-serve", str(port), "--tickets"], stderr=subprocess.PIPE)
    try:
        wait_port(port)
        sess = str(d / "sess.pem")
        base = ["openssl", "s_client", "-connect", f"127.0.0.1:{port}", "-servername", "localhost", "-CAfile", str(d / "ca.pem"), "-tls1_3", "-ign_eof"]
        subprocess.run(base + ["-sess_out", sess], input=req, capture_output=True, timeout=20)
        # (the request on stdin too: s_client sends the file only as early data)
        r = subprocess.run(base + ["-sess_in", sess, "-early_data", str(reqfile)], input=req, capture_output=True, timeout=20)
        out = r.stdout.decode(errors="replace")
        check("0-RTT: a ticket of a server without 0-RTT: none sent, resumed", "Early data was not sent" in out and "JFLib tls ok resumed" in out, out[-300:])
    finally:
        srv.terminate()
        srv.wait(5)
    # our client -> openssl s_server (it issues tickets by itself; -www shows "Reused")
    def get(port, *opts):
        r = subprocess.run([exe, "--tls-get", str(port), "--host", "localhost", "--resume", "2", *opts], capture_output=True, timeout=40)
        return (r.stdout + r.stderr).decode(errors="replace")

    for opts, what in (([], ""), (["-ciphersuites", "TLS_AES_256_GCM_SHA384"], ", TLS_AES_256_GCM_SHA384"), (["-groups", "P-384"], " after a HelloRetryRequest"),
                       (["-Verify", "1", "-CAfile", str(d / "ca.pem"), "-verify_return_error"], " with a client certificate")):
        port = free_port()
        srv = subprocess.Popen(["openssl", "s_server", "-accept", str(port), "-cert", str(d / "server.pem"), "-key", str(d / "server.key"), "-tls1_3", "-www", *opts],
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            wait_port(port)
            out = get(port, *(["--mtls"] if "-Verify" in opts else []))
            second = out.split("--- connection 2:", 1)[-1]
            check(f"resume: our client -> openssl s_server{what}", "--- connection 1: resumed false" in out and second.startswith(" resumed true") and "Reused, TLSv1.3" in second, out[-300:])
        finally:
            srv.terminate()
            srv.wait(5)
    # 0-RTT: s_server -early_data echoes nothing; it keeps its stdin open, our client reads until
    # its timeout.  After a HelloRetryRequest it does not resume: its tickets of 0-RTT are single
    # use (anti-replay), and the first ClientHello spent this one (so does OpenSSL's own client)
    for opts, what, want in (([], "the request taken as 0-RTT", " resumed true, early 35"), (["-groups", "P-384"], "refused after a HelloRetryRequest", " resumed false, early 0")):
        port = free_port()
        log = d / f"s_server-early-{port}.log"
        srv = subprocess.Popen(["openssl", "s_server", "-accept", str(port), "-cert", str(d / "server.pem"), "-key", str(d / "server.key"), "-tls1_3", "-early_data", *opts],
                               stdin=subprocess.PIPE, stdout=open(log, "wb"), stderr=subprocess.STDOUT)
        try:
            wait_port(port)
            out = get(port, "--early")
            second = out.split("--- connection 2:", 1)[-1]
            # (s_server writes the early data itself straight to its stdout, before its buffered note)
            seen = log.read_bytes()
            mark = b"Early data received" if want.endswith("35") else b"Early data was rejected"
            check(f"0-RTT: our client -> openssl s_server -early_data: {what}", second.startswith(want) and mark in seen and b"GET / HTTP/1.0" in seen, out[-300:])
        finally:
            srv.terminate()
            srv.wait(5)


if __name__ == "__main__":
    sys.exit(main())
