"""tlsfuzzer_check.py -- netio's TLS 1.3 server against tlsfuzzer's TLS 1.3 scripts.
    python tools/tlsfuzzer_check.py [--only SCRIPT,..] [--list]
tlsfuzzer (github.com/tlsfuzzer/tlsfuzzer) sends a server well-formed and deliberately broken
handshakes and checks every answer against RFC 8446: alerts, the state machine, record layer,
lengths, padding, key updates, version and group negotiation.  The pinned commits of tlsfuzzer
and tlslite-ng come into build/tlsfuzzer once (git), python-ecdsa and six through pip --target;
nothing is installed system-wide.  The server is `coretest --tls-serve` (netio's server with the
Ed25519 certificate of coretest's PKI, HTTP behind it); the scripts of ECHO, which check the
length of what comes back, get `coretest --tls-echo` (each line back in one record).
SCRIPTS below says for each script what to expect: run (every probe must pass, the probes named
there are expected to fail for the reason given) or not applicable (a feature netio does not have:
RSA / ECDSA / ML-DSA certificates, PSK and resumption, FFDHE, brainpool, ML-KEM, post-handshake
auth).  Linux (from Windows: runs itself again in WSL); needs git, pip and the Linux coretest
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

# netio's test server has an Ed25519 certificate only, while most scripts offer RSA / ECDSA
# signatures alone.  For those (patch True) a sitecustomize appends ed25519 to every
# signature_algorithms list that names RSA or ECDSA schemes; nothing else of the script changes.
# Scripts about signature negotiation run without it.
PATCH = """# written by tools/tlsfuzzer_check.py -- see PATCH there
try:
    import tlslite.extensions as _e
    from tlslite.constants import SignatureScheme as _S, SignatureAlgorithm as _A

    def _ours(alg):
        name = _S.toRepr(alg)
        if name:
            return name.startswith(("rsa_", "ecdsa_"))
        return isinstance(alg, tuple) and alg[1] in (_A.rsa, _A.ecdsa)

    _orig = _e._SigListExt.create

    def _create(self, sigalgs=None):
        if sigalgs and _S.ed25519 not in sigalgs and any(_ours(a) for a in sigalgs):
            sigalgs = list(sigalgs) + [_S.ed25519]
        return _orig(self, sigalgs)

    _e._SigListExt.create = _create
except ImportError:
    pass
"""

# script -> (extra arguments, None to run it or the reason it does not apply, with the Ed25519
# patch); probes expected to fail go in XFAIL with their reason
NO_RSA = "the server has an Ed25519 certificate only (RSA / ECDSA certificates: phase 2b)"
NO_PSK = "no PSK, session tickets or resumption"
NO_CLIENT_CERT = "needs a client key and certificate for tlsfuzzer and a server asking for them"
SCRIPTS = {
    "test-tls13-conversation": ([], None, True),
    "test-tls13-ccs": ([], None, True),
    "test-tls13-count-tickets": ([], None, True),
    "test-tls13-dhe-shared-secret-padding": ([], None, True),
    "test-tls13-empty-alert": ([], None, True),
    "test-tls13-finished": ([], None, True),
    "test-tls13-finished-plaintext": ([], None, True),
    "test-tls13-hrr": ([], None, True),
    "test-tls13-invalid-ciphers": ([], None, True),
    "test-tls13-keyshare-omitted": ([], None, True),
    "test-tls13-keyupdate": ([], None, True),
    "test-tls13-keyupdate-from-server": ([], None, True),
    "test-tls13-large-number-of-extensions": ([], None, True),
    "test-tls13-legacy-version": ([], None, True),
    "test-tls13-minerva": ([], None, True),
    "test-tls13-multiple-ccs-messages": ([], None, True),
    "test-tls13-nociphers": ([], None, True),
    "test-tls13-no-unknown-groups": ([], None, True),
    "test-tls13-record-layer-limits": ([], None, True),
    "test-tls13-record-padding": ([], None, True),
    "test-tls13-shuffled-extentions": ([], None, True),
    "test-tls13-unencrypted-alert": ([], None, True),
    "test-tls13-zero-content-type": ([], None, True),
    "test-tls13-zero-length-data": ([], None, True),
    "test-tls13-0rtt-garbage": ([], None, True),
    "test-tls13-connection-abort": ([], None, True),
    "test-tls13-crfg-curves": ([], None, True),
    "test-tls13-ecdhe-curves": ([], None, True),
    "test-tls13-obsolete-curves": ([], None, True),
    "test-tls13-serverhello-random": ([], None, True),
    "test-tls13-symetric-ciphers": ([], None, True),
    "test-tls13-unrecognised-groups": ([], None, True),
    "test-tls13-version-negotiation": ([], None, True),
    "test-tls13-eddsa": ([], None, False),
    "test-tls13-lengths": ([], None, True),
    "test-tls13-certificate-compression": ([], "no certificate compression (RFC 8879)", True),
    "test-tls13-client-certificate-compression": ([], "no certificate compression (RFC 8879)", True),
    "test-tls13-certificate-request": ([], NO_CLIENT_CERT, True),
    "test-tls13-certificate-verify": ([], NO_CLIENT_CERT, True),
    "test-tls13-post-handshake-auth": ([], NO_CLIENT_CERT + "; no post-handshake auth", True),
    "test-tls13-ecdsa-support": ([], NO_RSA, False),
    "test-tls13-ecdsa-in-certificate-verify": ([], NO_RSA, False),
    "test-tls13-ecdsa-brainpool-in-certificate-verify": ([], NO_RSA, False),
    "test-tls13-eddsa-in-certificate-verify": ([], NO_CLIENT_CERT, False),
    "test-tls13-mldsa-in-certificate-verify": ([], "no ML-DSA", False),
    "test-tls13-pkcs-signature": ([], NO_RSA, False),
    "test-tls13-rsa-signatures": ([], NO_RSA, False),
    "test-tls13-rsapss-signatures": ([], NO_RSA, False),
    "test-tls13-signature-algorithms": ([], NO_RSA, False),
    "test-tls13-ecdhe-brainpool-curves": ([], "no brainpool groups", True),
    "test-tls13-ffdhe-groups": ([], "no FFDHE groups", True),
    "test-tls13-ffdhe-sanity": ([], "no FFDHE groups", True),
    "test-tls13-mlkem": ([], "no ML-KEM (phase 2)", True),
    "test-tls13-non-support": ([], "for servers without TLS 1.3", True),
    "test-tls13-psk_dhe_ke": ([], NO_PSK, True),
    "test-tls13-psk_ke": ([], NO_PSK, True),
    "test-tls13-session-resumption": ([], NO_PSK, True),
}
# the groups netio's server does not have (probes named "TLS 1.3 with GROUP")
GROUPS_NOT = "secp384r1|secp521r1|x448|ffdhe|brainpool"
# scripts that expect the server to send back what they sent, of the same length
ECHO = {"test-tls13-lengths"}
XFAIL = {
    "test-tls13-finished": [("TLS_AES_256_GCM_SHA384", "no TLS_AES_256_GCM_SHA384 (needs SHA-384 in the key schedule)")],
    "test-tls13-symetric-ciphers": [("TLS_AES_256_GCM_SHA384|CCM", "only TLS_AES_128_GCM_SHA256 and TLS_CHACHA20_POLY1305_SHA256")],
    "test-tls13-record-layer-limits": [("^max size of ClientHello msg", "the Ed25519 patch makes the ClientHello 4 bytes larger than the 2^14 the script fills it to")],
    "test-tls13-keyupdate-from-server": [("post-handshake KeyUpdate msg from server", "the server does not start key updates by itself")],
    "test-tls13-connection-abort": [("After NewSessionTicket", "no session tickets")],
    "test-tls13-crfg-curves": [("x448", "no x448")],
    "test-tls13-ecdhe-curves": [("secp384r1|secp521r1|x448", "X25519 and P-256 only")],
    "test-tls13-obsolete-curves": [(".", "a client listing obsolete groups (RFC 8446 4.2.7) is not refused; netio takes X25519 or P-256 from the list")],
    "test-tls13-serverhello-random": [(GROUPS_NOT, "X25519 and P-256 only")],
    "test-tls13-dhe-shared-secret-padding": [(GROUPS_NOT, "X25519 and P-256 only")],
    "test-tls13-0rtt-garbage": [("downgrade to TLS 1\\.2", "the server speaks TLS 1.3 only")],
    "test-tls13-unrecognised-groups": [("ffdhe", "no FFDHE groups")],
    "test-tls13-version-negotiation": [("1\\.2|TLS 1\\.2|\\(3, 3\\)|fallback|in client hello legacy field",
                                        "the server speaks TLS 1.3 only (a ClientHello without supported_versions asks for TLS 1.2)")],
    "test-tls13-eddsa": [("ed448", "no Ed448")],
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
    patch = OUT / "ed25519_patch"
    patch.mkdir(exist_ok=True)
    (patch / "sitecustomize.py").write_text(PATCH, encoding="utf-8")
    plain = os.pathsep.join([str(lite), str(pylib), str(fuzzer)])
    return fuzzer, plain, os.pathsep.join([str(patch), plain])


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
        for name, (args, why, patch) in SCRIPTS.items():
            print(f"{name:<52} " + ("run" + (" " + " ".join(args) if args else "") + (" (Ed25519 added)" if patch else "") if why is None else f"n/a: {why}"))
            for rx, reason in XFAIL.get(name, []):
                print(f"    expected to fail: /{rx}/: {reason}")
        return 0
    exe = ROOT / "build" / "linux" / "coretest"
    if not exe.exists():
        sys.exit("tlsfuzzer_check: build coretest first (python build_rs.py app coretest --target linux)")
    OUT.mkdir(parents=True, exist_ok=True)
    fuzzer, plain, patched = setup()
    only = [s for s in a.only.split(",") if s]
    port, echo_port = free_port(), free_port()
    srv = subprocess.Popen([str(exe), "--tls-serve", str(port)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    echo = subprocess.Popen([str(exe), "--tls-echo", str(echo_port)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    bad = 0
    try:
        if not wait_port(port) or not wait_port(echo_port):
            sys.exit("tlsfuzzer_check: the test servers did not start")
        logs = OUT / "logs"
        logs.mkdir(exist_ok=True)
        for name, (args, why, patch) in SCRIPTS.items():
            if only and name not in only:
                continue
            if why is not None:
                print(f"  n/a  {name:<50} {why}")
                continue
            counts, out = run_script(fuzzer, patched if patch else plain, echo_port if name in ECHO else port, name, args, a.timeout)
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
        for s in (srv, echo):
            s.terminate()
            s.wait(5)
    print("tlsfuzzer_check: " + ("all as expected" if not bad else f"{bad} script(s) not as expected (logs in {(OUT / 'logs').relative_to(ROOT)})"))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
