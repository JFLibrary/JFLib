"""tlsfuzzer_check.py -- netio's TLS 1.3 server against tlsfuzzer's TLS 1.3 scripts.
    python tools/tlsfuzzer_check.py [--only SCRIPT,..] [--list]
tlsfuzzer (github.com/tlsfuzzer/tlsfuzzer) sends a server well-formed and deliberately broken
handshakes and checks every answer against RFC 8446: alerts, the state machine, record layer,
lengths, padding, key updates, version and group negotiation.  The pinned commits of tlsfuzzer
and tlslite-ng come into build/tlsfuzzer once (git), python-ecdsa and six through pip --target;
nothing is installed system-wide.  The servers are `coretest --tls-serve --key KIND` (netio's
server, HTTP behind it); the scripts of ECHO, which check the length of what comes back, get
`coretest --tls-echo` (each line back in one record).
SCRIPTS below says for each script what to expect and which server it talks to (RSA, ECDSA P-256
or Ed25519 certificates of coretest's PKI, one asking for a client certificate; all of them send
session tickets): run (every probe must pass, the probes named there are expected to fail for the
reason given) or not applicable (a feature netio does not have: ML-DSA, external PSKs, FFDHE,
brainpool, post-handshake auth, certificate compression).  Linux (from Windows: runs itself again in WSL); needs git, pip and the Linux coretest
(python build_rs.py app coretest --target linux).  Exit code 1 on any unexpected result."""
import argparse
import os
import re
import socket
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "build" / "tlsfuzzer"
TLSFUZZER = ("https://github.com/tlsfuzzer/tlsfuzzer", "51aad6e9ae42f5b045fa9b76129fe0746631784d")
TLSLITE = ("https://github.com/tlsfuzzer/tlslite-ng", "7e95ea1a86c0d42d8d931ff14134d31f01ce1d76")
PIP = ["ecdsa==0.19.2", "six==1.17.0"]

# the servers: coretest --tls-serve with keys of a kind (--key), one asking for a client
# certificate of coretest's CA (--mtls; the scripts get its client key and certificate)
# (all with session tickets, two per handshake)
SERVERS = {
    "rsa": ["--key", "rsa", "--tickets"],
    # a P-256 and a P-384 certificate, chosen by the client's signature schemes
    "ecdsa": ["--key", "p256,p384", "--tickets"],
    "ed25519": ["--key", "ed25519", "--tickets"],
    # asks for a client certificate of coretest's CA, takes a client without one
    "rsa-mtls": ["--key", "rsa", "--mtls-optional", "--tickets"],
}
# a script's arguments may name the client's key and certificate of the PKI
CLIENT = ["-k", "{pki}/client.key", "-c", "{pki}/client.pem"]
# the signature schemes of the server's CertificateRequest, in its order
CR_SIGALGS = ["-s", "ecdsa_secp256r1_sha256 ecdsa_secp384r1_sha384 rsa_pss_rsae_sha256 rsa_pss_rsae_sha384 rsa_pss_rsae_sha512 ed25519 "
              "rsa_pkcs1_sha256 rsa_pkcs1_sha384 rsa_pkcs1_sha512"]

# script -> (extra arguments, None to run it or the reason it does not apply, the server);
# probes expected to fail go in XFAIL with their reason
NO_EXT_PSK = "no external PSKs (resumption with tickets: test-tls13-session-resumption)"
SCRIPTS = {
    "test-tls13-conversation": ([], None, "rsa"),
    "test-tls13-ccs": ([], None, "rsa"),
    "test-tls13-count-tickets": (["-t", "2"], None, "rsa"),
    "test-tls13-dhe-shared-secret-padding": ([], None, "rsa"),
    "test-tls13-empty-alert": ([], None, "rsa"),
    "test-tls13-finished": ([], None, "rsa"),
    "test-tls13-finished-plaintext": ([], None, "rsa"),
    "test-tls13-hrr": ([], None, "rsa"),
    "test-tls13-invalid-ciphers": ([], None, "rsa"),
    "test-tls13-keyshare-omitted": ([], None, "rsa"),
    "test-tls13-keyupdate": ([], None, "rsa"),
    "test-tls13-keyupdate-from-server": ([], None, "rsa"),
    "test-tls13-large-number-of-extensions": ([], None, "rsa"),
    "test-tls13-legacy-version": ([], None, "rsa"),
    "test-tls13-minerva": ([], None, "ecdsa"),
    "test-tls13-multiple-ccs-messages": ([], None, "rsa"),
    "test-tls13-nociphers": ([], None, "rsa"),
    "test-tls13-no-unknown-groups": ([], None, "rsa"),
    "test-tls13-record-layer-limits": ([], None, "rsa"),
    "test-tls13-record-padding": ([], None, "rsa"),
    "test-tls13-shuffled-extentions": ([], None, "rsa"),
    "test-tls13-unencrypted-alert": ([], None, "rsa"),
    "test-tls13-zero-content-type": ([], None, "rsa"),
    "test-tls13-zero-length-data": ([], None, "rsa"),
    "test-tls13-0rtt-garbage": ([], None, "rsa"),
    "test-tls13-connection-abort": ([], None, "rsa"),
    "test-tls13-crfg-curves": ([], None, "rsa"),
    "test-tls13-ecdhe-curves": ([], None, "rsa"),
    "test-tls13-obsolete-curves": ([], None, "rsa"),
    "test-tls13-serverhello-random": ([], None, "rsa"),
    "test-tls13-symetric-ciphers": ([], None, "rsa"),
    "test-tls13-unrecognised-groups": ([], None, "rsa"),
    "test-tls13-version-negotiation": ([], None, "rsa"),
    "test-tls13-eddsa": ([], None, "ed25519"),
    "test-tls13-lengths": ([], None, "rsa"),
    "test-tls13-mlkem": ([], "needs kyber-py, which the tool does not fetch yet (ML-KEM: tools/tls_interop.py, OpenSSL 3.5)", "rsa"),
    "test-tls13-ecdsa-support": ([], None, "ecdsa"),
    "test-tls13-pkcs-signature": ([], None, "rsa"),
    "test-tls13-rsa-signatures": ([], None, "rsa"),
    "test-tls13-rsapss-signatures": (["-b"], None, "rsa"),
    "test-tls13-signature-algorithms": ([], None, "rsa"),
    "test-tls13-certificate-request": (CLIENT + CR_SIGALGS, None, "rsa-mtls"),
    "test-tls13-certificate-verify": (CLIENT + CR_SIGALGS, None, "rsa-mtls"),
    "test-tls13-ecdsa-in-certificate-verify": ([], "needs an ECDSA client key and certificate of the server's CA for tlsfuzzer", "rsa-mtls"),
    "test-tls13-ecdsa-brainpool-in-certificate-verify": ([], "no brainpool curves", "rsa-mtls"),
    "test-tls13-eddsa-in-certificate-verify": ([], "needs an Ed25519 client key and certificate of the server's CA for tlsfuzzer", "rsa-mtls"),
    "test-tls13-mldsa-in-certificate-verify": ([], "no ML-DSA", "rsa-mtls"),
    "test-tls13-post-handshake-auth": ([], "no post-handshake authentication", "rsa-mtls"),
    "test-tls13-certificate-compression": ([], "no certificate compression (RFC 8879)", "rsa"),
    "test-tls13-client-certificate-compression": ([], "no certificate compression (RFC 8879)", "rsa"),
    "test-tls13-ecdhe-brainpool-curves": ([], "no brainpool groups", "rsa"),
    "test-tls13-ffdhe-groups": ([], "no FFDHE groups", "rsa"),
    "test-tls13-ffdhe-sanity": ([], "no FFDHE groups", "rsa"),
    "test-tls13-non-support": ([], "for servers without TLS 1.3", "rsa"),
    "test-tls13-psk_dhe_ke": ([], NO_EXT_PSK, "rsa"),
    "test-tls13-psk_ke": ([], NO_EXT_PSK, "rsa"),
    # psk_dhe_ke only: psk_ke would give up forward secrecy
    "test-tls13-session-resumption": (["--client-pke", "psk_dhe_ke"], None, "rsa"),
}
# the groups netio's server does not have (probes named "TLS 1.3 with GROUP")
GROUPS_NOT = "secp521r1|x448|ffdhe|brainpool"
# scripts that expect the server to send back what they sent, of the same length
ECHO = {"test-tls13-lengths"}
XFAIL = {
    # a Finished padded past the largest message: the server refuses it at its header, sends
    # decode_error and closes while tlslite still sends; when that send fails first, tlsfuzzer's
    # runner skips the ExpectAlert and reads the alert in ExpectClose (runner.py, socket.error on
    # send) -- a race of the script, either answer can come
    "test-tls13-finished": [("pad_left 0, pad_right (196608|1677\\d{4})$", "a race in tlsfuzzer's runner when the server refuses a message of 192 KiB or more at its header")],
    "test-tls13-symetric-ciphers": [("CCM", "no CCM suites")],
    "test-tls13-keyupdate-from-server": [("post-handshake KeyUpdate msg from server", "the server does not start key updates by itself")],
    "test-tls13-crfg-curves": [("x448", "no x448")],
    "test-tls13-ecdhe-curves": [("secp521r1|x448", "no secp521r1, no x448")],
    "test-tls13-obsolete-curves": [(".", "a client listing obsolete groups (RFC 8446 4.2.7) is not refused; netio takes a group it has from the list")],
    "test-tls13-serverhello-random": [(GROUPS_NOT, "no secp521r1, x448, FFDHE, brainpool")],
    "test-tls13-dhe-shared-secret-padding": [(GROUPS_NOT, "no secp521r1, x448, FFDHE, brainpool")],
    "test-tls13-0rtt-garbage": [("downgrade to TLS 1\\.2", "the server speaks TLS 1.3 only")],
    "test-tls13-session-resumption": [("TLS 1\\.2", "the server speaks TLS 1.3 only (no TLS 1.2 tickets)")],
    "test-tls13-unrecognised-groups": [("ffdhe", "no FFDHE groups")],
    "test-tls13-version-negotiation": [("1\\.2|TLS 1\\.2|\\(3, 3\\)|fallback|in client hello legacy field",
                                        "the server speaks TLS 1.3 only (a ClientHello without supported_versions asks for TLS 1.2)")],
    "test-tls13-eddsa": [("ed448", "no Ed448")],
    "test-tls13-rsapss-signatures": [("rsa_pss_pss_", "no certificate with an RSASSA-PSS key (one with an rsaEncryption key signs with rsa_pss_rsae_*)")],
    "test-tls13-ecdsa-support": [("brainpool|secp521r1", "no brainpool curves, no P-521")],
    "test-tls13-no-unknown-groups": [("long list of unknown \\(between 4356 and 4611\\)",
                                      "the range holds SecP256r1MLKEM768, X25519MLKEM768 and SecP384r1MLKEM1024, which the server has: a HelloRetryRequest for the first")],
}


def checkout(url, commit):
    d = OUT / f"{url.rsplit('/', 1)[1]}-{commit[:12]}"
    if not (d / ".git").is_dir():
        d.mkdir(parents=True, exist_ok=True)
        for cmd in (["git", "init", "-q"], ["git", "remote", "add", "origin", url], ["git", "fetch", "-q", "--depth", "1", "origin", commit],
                    ["git", "checkout", "-q", "FETCH_HEAD"]):
            subprocess.run(cmd, cwd=d, check=True)
    return d


def setup():
    fuzzer = checkout(*TLSFUZZER)
    lite = checkout(*TLSLITE)
    pylib = OUT / "pylib"
    if not (pylib / "ecdsa").is_dir():
        subprocess.run([sys.executable, "-m", "pip", "install", "-q", "--target", str(pylib)] + PIP, check=True)
    return fuzzer, os.pathsep.join([str(lite), str(pylib), str(fuzzer)])


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


def run_script(fuzzer, pypath, port, name, args, timeout):
    cmd = [sys.executable, str(fuzzer / "scripts" / f"{name}.py"), "-h", "localhost", "-p", str(port)] + args
    env = dict(os.environ, PYTHONPATH=pypath)
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=timeout, cwd=OUT)
        out = p.stdout + p.stderr
    except subprocess.TimeoutExpired as e:
        return {"error": f"more than {timeout} s"}, (e.stdout or b"").decode(errors="replace") if isinstance(e.stdout, bytes) else str(e.stdout or "")
    counts = {k: int(v) for k, v in re.findall(r"^(TOTAL|SKIP|PASS|XFAIL|FAIL|XPASS): (\d+)$", out, re.M)}
    failed = re.findall(r"^\t'(.+)'$", out.split("FAILED:")[-1], re.M) if "FAILED:" in out else []
    xpassed = re.findall(r"^\t'(.+)'$", out.split("XPASSED:")[-1].split("FAILED:")[0], re.M) if "XPASSED:" in out else []
    counts["failed"], counts["xpassed"] = failed, xpassed
    if "TOTAL" not in counts:
        counts["error"] = f"exit {p.returncode}, no summary"
    return counts, out


def main():
    if os.name == "nt":
        wsl = "/mnt/" + str(ROOT)[0].lower() + str(ROOT)[2:].replace("\\", "/")
        rest = " ".join("'" + a.replace("'", "'\\''") + "'" for a in sys.argv[1:])
        return subprocess.run(["wsl.exe", "-d", "Ubuntu-24.04", "-e", "sh", "-c", f"cd '{wsl}' && python3 tools/tlsfuzzer_check.py {rest}"]).returncode
    ap = argparse.ArgumentParser(description="netio's TLS 1.3 server against tlsfuzzer")
    ap.add_argument("--only", default="", help="scripts, comma-separated (test-tls13-...)")
    ap.add_argument("--list", action="store_true", help="what each script is expected to do, and why")
    ap.add_argument("--timeout", type=int, default=900, help="seconds per script")
    a = ap.parse_args()
    if a.list:
        for name, (args, why, server) in SCRIPTS.items():
            print(f"{name:<52} " + (f"run against {server}" + (" " + " ".join(args) if args else "") if why is None else f"n/a: {why}"))
            for rx, reason in XFAIL.get(name, []):
                print(f"    expected to fail: /{rx}/: {reason}")
        return 0
    exe = ROOT / "build" / "linux" / "coretest"
    if not exe.exists():
        sys.exit("tlsfuzzer_check: build coretest first (python build_rs.py app coretest --target linux)")
    OUT.mkdir(parents=True, exist_ok=True)
    fuzzer, pypath = setup()
    only = [s for s in a.only.split(",") if s]
    pki = OUT / "pki"
    pki.mkdir(exist_ok=True)
    subprocess.run([str(exe), "--tls-pki", str(pki), "--key", "rsa"], check=True)
    ports = {k: free_port() for k in SERVERS}
    echo_port = free_port()
    procs = [subprocess.Popen([str(exe), "--tls-serve", str(ports[k])] + v, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL) for k, v in SERVERS.items()]
    procs.append(subprocess.Popen([str(exe), "--tls-echo", str(echo_port), "--key", "rsa", "--tickets"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
    bad = 0
    try:
        if not all(wait_port(p) for p in list(ports.values()) + [echo_port]):
            sys.exit("tlsfuzzer_check: the test servers did not start")
        logs = OUT / "logs"
        logs.mkdir(exist_ok=True)
        for name, (args, why, server) in SCRIPTS.items():
            if only and name not in only:
                continue
            if why is not None:
                print(f"  n/a  {name:<50} {why}")
                continue
            args = [x.format(pki=pki) for x in args]
            counts, out = run_script(fuzzer, pypath, echo_port if name in ECHO else ports[server], name, args, a.timeout)
            (logs / f"{name}.log").write_text(out, encoding="utf-8")
            expected = XFAIL.get(name, [])
            # a failed probe whose name an expected pattern covers is a feature netio does not have
            unexpected = [f for f in set(counts.get("failed", [])) if not any(re.search(rx, f) for rx, _ in expected)]
            known = len(counts.get("failed", [])) - len([f for f in counts.get("failed", []) if f in unexpected])
            if "error" in counts or unexpected:
                bad += 1
                what = counts.get("error") or f"{len(unexpected)} unexpected failure(s): {sorted(unexpected)[:5]}"
                print(f"  FAIL {name:<50} {what}")
            else:
                print(f"  ok   {name:<50} {counts.get('PASS', 0)} passed" + (f", {known} failed as expected" if known else ""))
    finally:
        for s in procs:
            s.terminate()
            s.wait(5)
    print("tlsfuzzer_check: " + ("all as expected" if not bad else f"{bad} script(s) not as expected (logs in {(OUT / 'logs').relative_to(ROOT)})"))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
