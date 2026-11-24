"""pebble_check.py -- netio::acme against Pebble, Let's Encrypt's ACME test server.

    python tools/pebble_check.py [--version v2.10.1] [--nonce-reject PERCENT] [--keep]

Pebble and pebble-challtestsrv (github.com/letsencrypt/pebble, MPL-2.0) come as release binaries
into build/pebble/VERSION once (gh release download; nothing installed system-wide).  Pebble
gets a config of this run: its https certificate the server certificate of `coretest
--tls-pki DIR --key p256` (the ACME client trusts that CA), the challenge ports of the client's
servers in coretest, Retry-After of a second; challtestsrv is its DNS server (every name
127.0.0.1, the dns-01 TXT records the client sets through its management API) with its own
challenge servers off.  `coretest --acme-pebble` runs the checks: http-01, tls-alpn-01, dns-01
with a wildcard, an IP address, a profile, an invalid challenge, the account keys ES256, ES384,
RS256 and EdDSA, renewal information and an order that replaces, revocation by the account and by
the certificate's key, key change, contact, deactivation; every chain verified to Pebble's root.
A second Pebble that requires an external account binding checks EAB.  Pebble reuses
authorizations only now and then (PEBBLE_AUTHZREUSE=0; it still may, about once in a hundred),
and the check of an IP address by tls-alpn-01 orders on an account of its own, so it proves its
name by the challenge it is about; Pebble refuses 1 % of good nonces (--nonce-reject; Pebble's own default is 5 %, where three
refusals of one request in a row would fail a run now and then): the client tries again with
the nonce of the refusal.  The run's folder (PKI, config, logs) goes at the end unless --keep;
the log of a Pebble whose checks failed is shown before.  Linux; exit code 1 on a failure."""
import argparse
import json
import os
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXE = ROOT / "build" / "linux" / "coretest"


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_port(port, timeout=20):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        try:
            socket.create_connection(("127.0.0.1", port), 0.5).close()
            return True
        except OSError:
            time.sleep(0.1)
    return False


def fetch(version):
    """the two binaries of a release, each archive into a folder of its own"""
    base = ROOT / "build" / "pebble" / version
    out = {}
    for name in ("pebble", "pebble-challtestsrv"):
        exe = base / f"bin-{name}" / f"{name}-linux-amd64" / "linux" / "amd64" / name
        if not exe.exists():
            dl = base / f"dl-{name}"
            dl.mkdir(parents=True, exist_ok=True)
            subprocess.run(["gh", "release", "download", version, "--repo", "letsencrypt/pebble", "--pattern", f"{name}-linux-amd64.tar.gz", "--dir", str(dl), "--clobber"], check=True)
            (base / f"bin-{name}").mkdir(exist_ok=True)
            subprocess.run(["tar", "-xzf", str(dl / f"{name}-linux-amd64.tar.gz"), "-C", str(base / f"bin-{name}")], check=True)
        exe.chmod(0o755)
        out[name] = str(exe)
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", default="v2.10.1")
    ap.add_argument("--nonce-reject", type=int, default=1, help="the percentage of good nonces Pebble refuses")
    ap.add_argument("--keep", action="store_true", help="keep the run's folder")
    args = ap.parse_args()
    bins = fetch(args.version)
    if args.keep:
        d = Path(tempfile.mkdtemp(prefix="pebble-"))
        print(f"keeping {d}")
        return run(args, bins, d)
    with tempfile.TemporaryDirectory(prefix="pebble-") as td:
        return run(args, bins, Path(td))


def run(args, bins, d):
    subprocess.run([str(EXE), "--tls-pki", str(d), "--key", "p256"], check=True)
    dns, chall = free_port(), free_port()
    cs = subprocess.Popen([bins["pebble-challtestsrv"], "-defaultIPv4", "127.0.0.1", "-defaultIPv6", "", "-dnsserver", f"127.0.0.1:{dns}", "-http01", "", "-https01", "",
                           "-tlsalpn01", "", "-doh", "", "-management", f"127.0.0.1:{chall}"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    results = []
    try:
        wait_port(chall)
        for eab in (None, ("kid-1", "zWNDZM6eQGHWpSRTPal5eIUYFTu7EajVIoguysqZ9wG44nMEtx3MUAsUDkMTQ12W")):
            listen, mgmt, http, tls = free_port(), free_port(), free_port(), free_port()
            cfg = {"pebble": {
                "listenAddress": f"127.0.0.1:{listen}", "managementListenAddress": f"127.0.0.1:{mgmt}",
                "certificate": str(d / "server.pem"), "privateKey": str(d / "server.key"),
                "httpPort": http, "tlsPort": tls, "ocspResponderURL": "", "externalAccountBindingRequired": eab is not None,
                "domainBlocklist": ["blocked-domain.example"], "retryAfter": {"authz": 1, "order": 1}, "keyAlgorithm": "ecdsa",
                "profiles": {"default": {"description": "90 days", "validityPeriod": 7776000}, "shortlived": {"description": "6 days", "validityPeriod": 518400}}}}
            if eab:
                cfg["pebble"]["externalAccountMACKeys"] = {eab[0]: eab[1]}
            (d / "pebble.json").write_text(json.dumps(cfg))
            env = dict(os.environ, PEBBLE_VA_NOSLEEP="1", PEBBLE_AUTHZREUSE="0", PEBBLE_WFE_NONCEREJECT=str(args.nonce_reject))
            log = open(d / f"pebble-{listen}.log", "wb")
            pb = subprocess.Popen([bins["pebble"], "-config", str(d / "pebble.json"), "-dnsserver", f"127.0.0.1:{dns}"], env=env, stdout=log, stderr=subprocess.STDOUT)
            try:
                if not wait_port(listen):
                    print("FAIL pebble did not start: " + (d / f"pebble-{listen}.log").read_text()[-500:])
                    results.append(False)
                    continue
                cmd = [str(EXE), "--acme-pebble", str(d), "--directory", f"https://127.0.0.1:{listen}/dir", "--roots", f"https://127.0.0.1:{mgmt}/roots/0",
                       "--http", str(http), "--tls", str(tls), "--chall", f"http://127.0.0.1:{chall}"]
                if eab:
                    cmd += ["--eab", f"{eab[0]}:{eab[1]}"]
                r = subprocess.run(cmd, capture_output=True, timeout=900)
                out = r.stdout.decode(errors="replace")
                print(out, end="")
                if r.stderr:
                    print(r.stderr.decode(errors="replace")[-500:])
                got = [line.startswith("ok") for line in out.splitlines() if line[:4] in ("ok  ", "FAIL")]
                results += got
                if r.returncode not in (0, 1):
                    results.append(False)
                if not all(got) or r.returncode != 0:
                    log.flush()
                    print(f"--- the end of Pebble's log ({'EAB' if eab else 'plain'}):")
                    print((d / f"pebble-{listen}.log").read_text(errors="replace")[-2000:])
            finally:
                pb.terminate()
                pb.wait(5)
                log.close()
    finally:
        cs.terminate()
        cs.wait(5)
    print(f"pebble_check (Pebble {args.version}): {sum(results)} of {len(results)} passed")
    return 0 if results and all(results) else 1


if __name__ == "__main__":
    sys.exit(main())
