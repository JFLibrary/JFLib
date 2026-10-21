"""tls_web_check.py -- netio's TLS client for public servers against OpenSSL and the internet.

    python tools/tls_web_check.py [--internet]      (runs in WSL / Linux: needs openssl)

A test PKI made with openssl (an RSA root, an ECDSA intermediate, server certificates with
RSA-2048, RSA-PSS, ECDSA P-256 and P-384 keys) and `openssl s_server -www` in one setting at a
time; `coretest --tls-web URL --ca root.pem` must connect with the expected version and suite,
or refuse with the expected reason:

  TLS 1.3  AES-128-GCM / ChaCha20 x RSA / RSA-PSS / P-256 / P-384 (through the intermediate)
  TLS 1.2  ECDHE-RSA / ECDHE-ECDSA x AES-128-GCM / ChaCha20, without the extended master secret
  both     X25519 and P-256 (TLS 1.3 by HelloRetryRequest), a request for a client certificate
  refused  unknown CA, wrong host name, expired certificate, only P-384 key exchange, TLS 1.1,
           only AES-256-GCM, a server certificate signed with SHA-1

--internet adds public servers (system trust, Windows build too when run from Windows):
www.google.com, github.com, and badssl.com's TLS 1.2-only, RSA-2048/4096, P-256/384 hosts (must work)
and its expired / wrong-host / self-signed / untrusted-root hosts (must fail).  (Hosts of a public test
service: a failure there can also mean it changed.)"""
import os
import socket
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


def sh(cmd, cwd):
    subprocess.run(cmd, cwd=cwd, check=True, capture_output=True, shell=isinstance(cmd, str))


def make_pki(d):
    """root (RSA 3072) -> intermediate (P-384) -> leaves; leaves for localhost"""
    ext = d / "leaf.ext"
    ext.write_text("basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\n"
                   "subjectAltName=DNS:localhost,IP:127.0.0.1\n")
    ca_ext = d / "ca.ext"
    ca_ext.write_text("basicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n")
    sh("openssl req -x509 -newkey rsa:3072 -nodes -keyout root.key -out root.pem -days 30 -subj /CN=Test-Root "
       "-addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign,cRLSign", d)
    sh("openssl ecparam -name secp384r1 -genkey -noout -out int.key", d)
    sh("openssl req -new -key int.key -out int.csr -subj /CN=Test-Intermediate", d)
    sh("openssl x509 -req -in int.csr -CA root.pem -CAkey root.key -CAcreateserial -out int.pem -days 30 -sha384 -extfile ca.ext", d)
    leaves = {
        "rsa": ("openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out rsa.key", "root", "-sha256"),
        "pss": ("openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out pss.key", "root",
                "-sha256 -sigopt rsa_padding_mode:pss -sigopt rsa_pss_saltlen:32"),
        "p256": ("openssl ecparam -name prime256v1 -genkey -noout -out p256.key", "int", "-sha256"),
        "p384": ("openssl ecparam -name secp384r1 -genkey -noout -out p384.key", "int", "-sha384"),
        "sha1": ("openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out sha1.key", "root", "-sha1"),
    }
    for name, (gen, ca, dig) in leaves.items():
        sh(gen, d)
        sh(f"openssl req -new -key {name}.key -out {name}.csr -subj /CN=localhost", d)
        sh(f"openssl x509 -req -in {name}.csr -CA {ca}.pem -CAkey {ca}.key -CAcreateserial -out {name}.pem -days 30 {dig} -extfile leaf.ext", d)
        chain = (d / f"{name}.pem").read_text() + ((d / "int.pem").read_text() if ca == "int" else "")
        (d / f"{name}.chain.pem").write_text(chain)
    # a certificate for another name, and an expired one
    other = d / "other.ext"
    other.write_text(ext.read_text().replace("DNS:localhost,IP:127.0.0.1", "DNS:other.example"))
    sh("openssl req -new -key rsa.key -out other.csr -subj /CN=other.example", d)
    sh("openssl x509 -req -in other.csr -CA root.pem -CAkey root.key -CAcreateserial -out other.pem -days 30 -extfile other.ext", d)
    # expired (2020): made with Python's cryptography (openssl x509 cannot back-date here)
    import datetime
    from cryptography import x509
    from cryptography.hazmat.primitives import hashes, serialization
    from cryptography.x509.oid import NameOID, ExtendedKeyUsageOID
    root_key = serialization.load_pem_private_key((d / "root.key").read_bytes(), None)
    root = x509.load_pem_x509_certificate((d / "root.pem").read_bytes())
    leaf_key = serialization.load_pem_private_key((d / "rsa.key").read_bytes(), None)
    old = (x509.CertificateBuilder().subject_name(x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "localhost")]))
           .issuer_name(root.subject).public_key(leaf_key.public_key()).serial_number(4242)
           .not_valid_before(datetime.datetime(2020, 1, 1)).not_valid_after(datetime.datetime(2020, 2, 1))
           .add_extension(x509.SubjectAlternativeName([x509.DNSName("localhost")]), False)
           .add_extension(x509.ExtendedKeyUsage([ExtendedKeyUsageOID.SERVER_AUTH]), False)
           .sign(root_key, hashes.SHA256()))
    (d / "old.pem").write_bytes(old.public_bytes(serialization.Encoding.PEM))


def main():
    internet = "--internet" in sys.argv
    if os.name == "nt":
        wsl = "/mnt/" + str(ROOT)[0].lower() + str(ROOT)[2:].replace("\\", "/")
        r = subprocess.run(["wsl.exe", "-d", "Ubuntu-24.04", "-e", "sh", "-c", f"cd '{wsl}' && python3 tools/tls_web_check.py" + (" --internet" if internet else "")]).returncode
        if internet:
            r |= internet_checks([str(ROOT / "build" / "win" / "coretest.exe")], "win")
        return r
    exe = str(ROOT / "build" / "linux" / "coretest")
    d = Path(tempfile.mkdtemp(prefix="tls-web-"))
    make_pki(d)
    bad = 0

    conf = d / "no-ems.cnf"
    conf.write_text("openssl_conf = init\n[init]\nssl_conf = ssl\n[ssl]\nsystem_default = sys\n[sys]\nOptions = -ExtendedMasterSecret\n")

    def run(name, cert, server_opts, want, ca=True, host="localhost", env=None):
        nonlocal bad
        port = free_port()
        keyfile = {"other": "rsa", "old": "rsa"}.get(cert, cert)
        srv = subprocess.Popen(["openssl", "s_server", "-accept", str(port), "-www", "-cert", f"{cert}.pem", "-key",
                                f"{keyfile}.key", *(["-cert_chain", "int.pem"] if cert in ("p256", "p384") else []),
                                *server_opts.split()], cwd=d, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                               env=dict(os.environ, **(env or {})))
        try:
            wait_port(port)
            args = [exe, "--tls-web", f"https://{host}:{port}/"] + (["--ca", str(d / "root.pem")] if ca else [])
            out = subprocess.run(args, capture_output=True, text=True, timeout=30).stdout.strip()
        finally:
            srv.terminate()
            srv.wait(5)
        ok = out.startswith(want) if not want.startswith("error") else out.startswith("error") and want[6:] in out
        bad += not ok
        print(f"{'ok  ' if ok else 'FAIL'} {name:46} {out[:100]}")

    v13 = "-tls1_3 -ciphersuites"
    run("1.3 AES-128-GCM, RSA", "rsa", f"{v13} TLS_AES_128_GCM_SHA256", "TLS 1.3 0x1301 200")
    run("1.3 ChaCha20, RSA", "rsa", f"{v13} TLS_CHACHA20_POLY1305_SHA256", "TLS 1.3 0x1303 200")
    run("1.3 AES-128-GCM, RSA-PSS certificate", "pss", f"{v13} TLS_AES_128_GCM_SHA256", "TLS 1.3 0x1301 200")
    run("1.3 AES-128-GCM, P-256 via P-384 intermediate", "p256", f"{v13} TLS_AES_128_GCM_SHA256", "TLS 1.3 0x1301 200")
    run("1.3 ChaCha20, P-384 via intermediate", "p384", f"{v13} TLS_CHACHA20_POLY1305_SHA256", "TLS 1.3 0x1303 200")
    run("1.3 both suites offered", "rsa", f"{v13} TLS_AES_128_GCM_SHA256:TLS_CHACHA20_POLY1305_SHA256", "TLS 1.3 ")
    run("1.2 ECDHE-RSA-AES128-GCM", "rsa", "-tls1_2 -cipher ECDHE-RSA-AES128-GCM-SHA256", "TLS 1.2 0xC02F 200")
    run("1.2 ECDHE-RSA-CHACHA20", "rsa", "-tls1_2 -cipher ECDHE-RSA-CHACHA20-POLY1305", "TLS 1.2 0xCCA8 200")
    run("1.2 ECDHE-ECDSA-AES128-GCM (P-256)", "p256", "-tls1_2 -cipher ECDHE-ECDSA-AES128-GCM-SHA256", "TLS 1.2 0xC02B 200")
    run("1.2 ECDHE-ECDSA-CHACHA20 (P-384)", "p384", "-tls1_2 -cipher ECDHE-ECDSA-CHACHA20-POLY1305", "TLS 1.2 0xCCA9 200")
    run("1.2 RSA-PSS certificate", "pss", "-tls1_2 -cipher ECDHE-RSA-AES128-GCM-SHA256", "TLS 1.2 0xC02F 200")
    run("1.2 without extended master secret", "rsa", "-tls1_2 -cipher ECDHE-RSA-AES128-GCM-SHA256", "TLS 1.2 0xC02F 200",
        env={"OPENSSL_CONF": str(conf)})
    run("1.2 server asks for a client certificate", "rsa", "-tls1_2 -cipher ECDHE-RSA-AES128-GCM-SHA256 -verify 1", "TLS 1.2 0xC02F 200")
    run("1.3 server asks for a client certificate", "rsa", f"{v13} TLS_AES_128_GCM_SHA256 -verify 1", "TLS 1.3 0x1301 200")
    run("refused: unknown CA", "rsa", "", "error not trusted", ca=False)
    run("refused: wrong host name", "other", "", "error another name")
    run("refused: expired certificate", "old", "", "error expired")
    run("refused: SHA-1 signed certificate", "sha1", "-cipher DEFAULT@SECLEVEL=0", "error not trusted")
    run("1.3 only P-256 key exchange (HelloRetryRequest)", "rsa", "-tls1_3 -groups P-256", "TLS 1.3 ")
    run("1.2 only P-256 key exchange", "rsa", "-tls1_2 -groups P-256 -cipher ECDHE-RSA-AES128-GCM-SHA256", "TLS 1.2 0xC02F 200")
    run("1.2 P-256 key exchange, ECDSA", "p256", "-tls1_2 -groups P-256 -cipher ECDHE-ECDSA-CHACHA20-POLY1305", "TLS 1.2 0xCCA9 200")
    run("refused: only P-384 key exchange", "rsa", "-groups P-384", "error ")
    run("refused: TLS 1.1 only", "rsa", "-tls1_1", "error ")
    run("refused: only AES-256-GCM", "rsa", f"{v13} TLS_AES_256_GCM_SHA384", "error ")
    if internet:
        bad += internet_checks([exe], "linux")
    print("tls_web_check:", "all ok" if not bad else f"{bad} failures")
    return 1 if bad else 0


def internet_checks(cmd, tag):
    ok_hosts = ["https://www.google.com/", "https://github.com/", "https://tls-v1-2.badssl.com:1012/", "https://rsa2048.badssl.com/",
                "https://rsa4096.badssl.com/", "https://ecc256.badssl.com/", "https://ecc384.badssl.com/"]
    # sha384.badssl.com: its certificate expired in 2022
    bad_hosts = ["https://expired.badssl.com/", "https://wrong.host.badssl.com/", "https://self-signed.badssl.com/",
                 "https://untrusted-root.badssl.com/", "https://sha384.badssl.com/"]
    bad = 0
    for u in ok_hosts + bad_hosts:
        try:
            out = subprocess.run(cmd + ["--tls-web", u], capture_output=True, text=True, timeout=40).stdout.strip()
        except subprocess.TimeoutExpired:
            out = "timeout"
        want_ok = u in ok_hosts
        ok = out.startswith("TLS") if want_ok else out.startswith("error TLS")
        bad += not ok
        print(f"{'ok  ' if ok else 'FAIL'} [{tag}] {u:42} {out[:90]}")
    return bad


if __name__ == "__main__":
    sys.exit(main())
