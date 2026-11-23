"""tlsanvil_check.py -- netio's TLS client and server against TLS-Anvil (github.com/tls-attacker/TLS-Anvil).
    python tools/tlsanvil_check.py [--server [--key rsa|p256|p384|ed25519]] [--strength N] [--parallel N] [--package P] [--tags T,..] [--setup-only]
TLS-Anvil runs about 450 test templates derived from the RFCs (TLS 1.2 and 1.3: the handshake,
extensions, alerts, record layer, cipher suites, key exchange groups, signature algorithms,
combinations of them), some for clients, some for servers.  Client (the default): TLS-Anvil plays
the server and starts `coretest --tls-anvil-client PORT` for each handshake (netio's web client,
Trust::Custom taking TLS-Anvil's made-up certificates, in the test program only).  --server:
TLS-Anvil connects to `coretest --tls-serve --key KIND` (netio's server, TLS 1.3, a certificate
of that kind of coretest's PKI: RSA by default; TLS-Scanner's probes offer no Ed25519).
TLS-Anvil judges what netio sends.
Everything comes into build/tlsanvil once, pinned and checked: Eclipse Temurin 21 (SHA-256 of
Adoptium's API), Maven 3.9.9 (SHA-512 of Apache's archive), TLS-Anvil at a commit, built with
Maven into its own repository (build/tlsanvil/m2); nothing is installed system-wide.  The results
are in build/tlsanvil/run (TLS-Anvil's JSON, one file per test template); this script sums them
up by result and lists the tests that failed, KNOWN apart (defects of TLS-Anvil itself).
run/client.log has a line from the client for every connection (how it ended: the error, or the
bytes read and whether a close_notify came).  Linux on x86-64; needs curl, tar, git and the Linux
coretest (python build_rs.py app coretest --target linux).  Exit code 1 when a test fails that
KNOWN does not explain."""
import argparse
import json
import os
import shutil
import socket
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "build" / "tlsanvil"
JDK = ("https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.13.1%2B2/OpenJDK21U-jdk_x64_linux_hotspot_21.0.13.1_2.tar.gz",
       "bb8a93f9ae6e7a693b2b721e85dbeff26ae1a77447985a8d7afcdd9c7cf6222e")
MAVEN = "https://archive.apache.org/dist/maven/maven-3/3.9.9/binaries/apache-maven-3.9.9-bin.tar.gz"
ANVIL = ("https://github.com/tls-attacker/TLS-Anvil", "c1762f4")
# failures that are TLS-Anvil's, not netio's (checked by hand; docs/EXTERNAL_SUITES.md)
KNOWN = {
    "ClientAuthentication.serverSendsCertificateRequestWithNonZeroPostHandshakeAuthInHandshake":
        "the CertificateRequest TLS-Anvil sends has an empty context: the test's change does not reach the message "
        "(every CertificateRequest of the package, as netio's client logged them)",
    "Fragmentation.recordFragmentationSupported":
        "TLS-Scanner's probe runs with TLS-Attacker's default configuration, not with what the client offers; "
        "netio's client takes records of 1, 50 and 111 bytes (tlslite-ng's server; coretest: client records)",
}


def fetch(url, dest):
    if not dest.exists():
        subprocess.run(["curl", "-sSL", "-o", str(dest), url], check=True)


def setup():
    dl = OUT / "dl"
    dl.mkdir(parents=True, exist_ok=True)
    jdk, maven, src = OUT / "jdk", OUT / "maven", OUT / "src"
    if not (jdk / "bin" / "java").exists():
        fetch(JDK[0], dl / "jdk.tar.gz")
        subprocess.run(["sha256sum", "-c", "-"], input=f"{JDK[1]}  {dl / 'jdk.tar.gz'}\n", text=True, check=True)
        jdk.mkdir(exist_ok=True)
        subprocess.run(["tar", "-xzf", str(dl / "jdk.tar.gz"), "-C", str(jdk), "--strip-components=1"], check=True)
    if not (maven / "bin" / "mvn").exists():
        fetch(MAVEN, dl / "maven.tar.gz")
        fetch(MAVEN + ".sha512", dl / "maven.sha512")
        want = (dl / "maven.sha512").read_text().split()[0]
        subprocess.run(["sha512sum", "-c", "-"], input=f"{want}  {dl / 'maven.tar.gz'}\n", text=True, check=True)
        maven.mkdir(exist_ok=True)
        subprocess.run(["tar", "-xzf", str(dl / "maven.tar.gz"), "-C", str(maven), "--strip-components=1"], check=True)
    if not (src / ".git").is_dir():
        subprocess.run(["git", "clone", "-q", ANVIL[0], str(src)], check=True)
        subprocess.run(["git", "checkout", "-q", ANVIL[1]], cwd=src, check=True)
    jar = src / "apps" / "TLS-Anvil.jar"
    if not jar.exists():
        env = dict(os.environ, JAVA_HOME=str(jdk))
        subprocess.run([str(maven / "bin" / "mvn"), "-q", "-f", str(src / "pom.xml"), f"-Dmaven.repo.local={OUT / 'm2'}", "install",
                        "-DskipTests", "-Dspotless.apply.skip", "-Dspotless.check.skip"], env=env, check=True)
    return jdk / "bin" / "java", jar


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def summarize(run):
    """TLS-Anvil's results (Results_*/results/ID/_testRun.json, one per test template):
    {result: count}, [(test, result, why)] of those that did not succeed"""
    counts, failed = {}, []
    for f in run.glob("Results_*/results/*/_testRun.json"):
        try:
            r = json.loads(f.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue
        res = r.get("Result") or "?"
        counts[res] = counts.get(res, 0) + 1
        if res not in ("STRICTLY_SUCCEEDED", "CONCEPTUALLY_SUCCEEDED", "DISABLED"):
            name = f"{r.get('TestClass', '').rsplit('.', 1)[-1]}.{r.get('TestMethod', '')}"
            why = " ".join((r.get("FailedReason") or r.get("DisabledReason") or "").split())
            failed.append((name, res, why.replace("org.opentest4j.AssertionFailedError: ", "")[:160]))
    return counts, sorted(failed)


def main():
    ap = argparse.ArgumentParser(description="netio's TLS client or server against TLS-Anvil")
    ap.add_argument("--server", action="store_true", help="test netio's server (coretest --tls-serve) instead of its client")
    ap.add_argument("--key", default="rsa", help="--server: the kind of the server's key (rsa, p256, p384, ed25519)")
    ap.add_argument("--strength", type=int, default=1, help="t-way strength of TLS-Anvil's combinatorial tests")
    ap.add_argument("--parallel", type=int, default=2, help="tests and handshakes at a time")
    ap.add_argument("--package", default="", help="only the tests of one package (e.g. de.rub.nds.tlstest.suite.tests.client.tls13.rfc8446)")
    ap.add_argument("--tags", default="", help="only tests with one of these tags (their @Tag annotations, e.g. new, rfc8446-coverage)")
    ap.add_argument("--setup-only", action="store_true")
    a = ap.parse_args()
    exe = ROOT / "build" / "linux" / "coretest"
    if not exe.exists():
        sys.exit("tlsanvil_check: build coretest first (python build_rs.py app coretest --target linux)")
    java, jar = setup()
    if a.setup_only:
        return 0
    run = OUT / "run"
    shutil.rmtree(run, ignore_errors=True)
    run.mkdir(parents=True)
    port = free_port()
    # no packet capture (tcpdump would need root); the results as JSON in run; no cache of the
    # scan (run is new each time, and TLS-Anvil fails to serialize a scan with ML-KEM key shares:
    # NotSerializableException MlKemPublicKey in TestPreparator.saveToCache)
    cmd = [str(java), "-jar", str(jar), "-disableTcpDump", "-ignoreCache", "-parallelHandshakes", str(a.parallel), "-parallelTests", str(a.parallel),
           "-strength", str(a.strength), "-identifier", "netio-server" if a.server else "netio-client"] + \
          (["-testPackage", a.package] if a.package else []) + (["-tags"] + a.tags.split(",") if a.tags else [])
    srv = None
    if a.server:
        srv = subprocess.Popen([str(exe), "--tls-serve", str(port), "--key", a.key], stdout=subprocess.DEVNULL, stderr=open(run / "server.log", "w"))
        cmd += ["server", "-connect", f"localhost:{port}"]
    else:
        cmd += ["client", "-port", str(port), "-triggerScript", "sh", "-c", f"exec '{exe}' --tls-anvil-client {port} 2>>'{run / 'client.log'}'"]
    try:
        with open(run / "tlsanvil.log", "w", encoding="utf-8") as log:
            p = subprocess.run(cmd, cwd=run, stdout=log, stderr=subprocess.STDOUT)
    finally:
        if srv is not None:
            srv.terminate()
            srv.wait(5)
    counts, failed = summarize(run)
    if not counts:
        print(f"tlsanvil_check: no results (exit {p.returncode}); see {(run / 'tlsanvil.log').relative_to(ROOT)}")
        return 1
    print("tlsanvil_check: " + ", ".join(f"{v} {k.lower().replace('_', ' ')}" for k, v in sorted(counts.items(), key=lambda kv: -kv[1])))
    bad = 0
    for name, res, why in failed:
        if name in KNOWN:
            print(f"  known  {res:<18} {name}: {KNOWN[name]}")
        else:
            bad += 1
            print(f"  {res:<25} {name}  {why}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
