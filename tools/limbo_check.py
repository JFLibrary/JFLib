"""limbo_check.py -- netio::x509::verify_chain against x509-limbo (C2SP/x509-limbo).

    python tools/limbo_check.py [--target linux|win] [--id SUBSTRING] [--show ID] [--list]

Fetches limbo.json of the pinned commit into build/vectors (tools/suite_fetch.py; nothing in
git), hands each testcase to `coretest --vectors` (x509chain, apps/coretest/src/suitevec.rs) the
way the TLS client does it -- leaf and intermediates parsed with Cert::parse (one unreadable: the
chain is refused, as tls/conn.rs does), anchors that do not parse left out (roots.rs),
verify_chain(chain, anchors, validation time, expected peer name, server / client usage), which
checks the name with Cert::names -- and holds the answer against expected_result.  A chain of
more than 10 certificates is judged by verify_chain and marked: tls/conn.rs refuses a
Certificate message that long.

Skipped, with the reason: testcases whose features netio states it lacks (x509.rs, header:
revocation / CRLs, certificate policies, name constraints other than DNS / IP; also ML-DSA keys,
a caller-chosen chain depth, e-mail peer names, extended key usages other than serverAuth /
clientAuth).  Optional behaviour (the pedantic-* features, rfc5280-incompatible-with-webpki) is
run, and its failures are counted apart.  PASS / FAIL / SKIP per testcase id with --list;
exit code 1 on a failure that is not optional."""
import argparse
import base64
import collections
import datetime
import json
import os
import re
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from suite_fetch import fetch  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
REPO = "C2SP/x509-limbo"
COMMIT = "b12423782a88abead01141fa6582254e3eb49e74"

# features netio does not have (x509.rs: "Not checked: revocation (CRL / OCSP), certificate
# policies; name constraints cover DNS names and IP addresses")
LACKS = {
    "has-crl": "revocation (CRL / OCSP) is not checked (x509.rs header)",
    "has-policy-constraints": "certificate policies are not checked (x509.rs header)",
    "has-cert-policies": "certificate policies are not checked (x509.rs header)",
    "no-cert-policies": "certificate policies are not checked (x509.rs header)",
    "name-constraint-dn": "name constraints cover DNS names and IP addresses only (x509.rs header)",
    "has-mldsa": "ML-DSA keys: netio knows Ed25519, RSA, P-256 and P-384",
    "max-chain-depth": "no chain depth of the caller's choosing (verify_chain has its own limit)",
}
OPTIONAL = {"pedantic-public-suffix-wildcard", "pedantic-webpki-subscriber-key", "pedantic-webpki-eku",
            "pedantic-serial-number", "pedantic-rfc5280", "rfc5280-incompatible-with-webpki"}


def pem_ders(text):
    return [base64.b64decode("".join(m.split())) for m in
            re.findall(r"-----BEGIN CERTIFICATE-----(.*?)-----END CERTIFICATE-----", text, re.S)]


def skip_reason(t):
    for f in t["features"]:
        if f in LACKS:
            return LACKS[f]
    if t["max_chain_depth"] is not None:
        return LACKS["max-chain-depth"]
    if t["crls"]:
        return LACKS["has-crl"]
    if t["signature_algorithms"]:
        return "verify_chain takes no list of signature algorithms"
    if set(t["key_usage"]) - {"digitalSignature"}:
        return "verify_chain checks the leaf's digitalSignature key usage only"
    want = {"SERVER": "serverAuth", "CLIENT": "clientAuth"}[t["validation_kind"]]
    if set(t["extended_key_usage"]) - {want}:
        return "verify_chain checks serverAuth / clientAuth (the validation kind's) only"
    p = t["expected_peer_name"]
    if p and p["kind"] not in ("DNS", "IP"):
        return f"{p['kind']} peer names: Cert::names knows DNS names and IP addresses"
    if t["expected_peer_names"]:
        return "expected_peer_names (client names checked by a server): verify_chain has no such check"
    return None


def line(t):
    """the x509chain line of a testcase (suitevec.rs)"""
    usage = b"\x01" if t["validation_kind"] == "CLIENT" else b"\x00"
    vt = t["validation_time"]
    now = int(datetime.datetime.fromisoformat(vt).timestamp()) if vt else int(time.time())
    p = t["expected_peer_name"]
    name = b"\x01" + p["value"].encode() if p else b"\x00"
    leaf = pem_ders(t["peer_certificate"])
    inter = [d for x in t["untrusted_intermediates"] for d in pem_ders(x)]
    trust = [d for x in t["trusted_certs"] for d in pem_ders(x)]
    args = [usage, now.to_bytes(8, "big", signed=True), name, len(inter).to_bytes(2, "big")] + leaf[:1] + inter + trust
    return "x509chain " + " ".join(a.hex() for a in args)


def verdict(out):
    """coretest's answer -> (accepted, why)"""
    if out == "01":
        return True, ""
    b = bytes.fromhex(out) if out and out != "-" else b""
    if not b:
        return False, "no answer"
    if b[0] == 0:
        return False, "verify_chain: " + b[1:].decode("utf-8", "replace")
    if b[0] == 2:
        return False, ("leaf" if b[1] == 0 else f"intermediate {b[1]}") + " unreadable (Cert::parse)"
    return False, out


def run_batch(exe, lines, timeout):
    with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
        f.write("\n".join(lines) + "\n")
        path = f.name
    try:
        r = subprocess.run([str(exe), "--vectors", path], capture_output=True, text=True, timeout=timeout)
    finally:
        os.unlink(path)
    if r.returncode != 0:
        raise RuntimeError(f"coretest failed ({r.returncode}): {r.stderr.strip()[-500:]}")
    return r.stdout.split("\n")[:len(lines)]


def run_all(exe, lines, timeout):
    """answers for all lines; a batch that takes too long is halved until the slow case is alone
    (its answer: 'timeout'); a crash likewise ('crash: ...')"""
    try:
        return run_batch(exe, lines, timeout)
    except (subprocess.TimeoutExpired, RuntimeError) as e:
        if len(lines) == 1:
            return ["timeout" if isinstance(e, subprocess.TimeoutExpired) else "crash: " + str(e)]
        h = len(lines) // 2
        return run_all(exe, lines[:h], timeout) + run_all(exe, lines[h:], timeout)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--target", default="win" if os.name == "nt" else "linux")
    ap.add_argument("--id", default="", help="testcases whose id contains this")
    ap.add_argument("--show", help="print one testcase's description and what netio said")
    ap.add_argument("--list", action="store_true", help="PASS / FAIL / SKIP for every testcase id")
    ap.add_argument("--timeout", type=float, default=120, help="seconds per batch of testcases")
    opt = ap.parse_args()
    exe = ROOT / "build" / opt.target / ("coretest.exe" if opt.target == "win" else "coretest")
    if not exe.exists():
        sys.exit(f"limbo_check: {exe.relative_to(ROOT)} is missing (python build_rs.py app coretest --target {opt.target})")
    folder = fetch(REPO, COMMIT, ["limbo.json"])
    tcs = json.loads((folder / "limbo.json").read_text(encoding="utf-8"))["testcases"]
    if opt.show:
        tcs = [t for t in tcs if t["id"] == opt.show]
    elif opt.id:
        tcs = [t for t in tcs if opt.id in t["id"]]
    run = [t for t in tcs if not skip_reason(t)]
    outs = []
    for i in range(0, len(run), 500):
        outs += run_all(exe, [line(t) for t in run[i:i + 500]], opt.timeout)
    got = {t["id"]: verdict(o) if not o.startswith(("timeout", "crash")) else (None, o) for t, o in zip(run, outs)}

    rows = []
    for t in tcs:
        why = skip_reason(t)
        if why:
            rows.append((t, "SKIP", why))
            continue
        acc, msg = got[t["id"]]
        ok = acc is not None and acc == (t["expected_result"] == "SUCCESS")
        if 1 + sum(len(pem_ders(x)) for x in t["untrusted_intermediates"]) > 10:
            # verify_chain takes any number; a TLS Certificate message of more is refused
            msg = (msg + "; " if msg else "") + "over 10 certificates: tls/conn.rs would refuse the message"
        rows.append((t, "PASS" if ok else "FAIL", msg))

    if opt.show:
        for t, st, msg in rows:
            print(f"{t['id']} [{', '.join(t['features']) or 'no features'}] expected {t['expected_result']}: {st} {msg}\n")
            print(t["description"])
        return 0
    if opt.list:
        for t, st, msg in rows:
            print(f"{st} {t['id']}" + (f"  ({msg})" if msg and (st != "PASS" or "tls/conn.rs" in msg) else ""))
        print()

    def optional(t):
        return bool(OPTIONAL & set(t["features"]))
    print(f"limbo_check: C2SP/x509-limbo @ {COMMIT[:12]}, {len(tcs)} testcases ({opt.target})\n")
    print(f"  {'group':14} {'pass':>5} {'fail':>5} {'skip':>5}")
    by = collections.OrderedDict()
    for t, st, _ in sorted(rows, key=lambda r: r[0]["id"]):
        by.setdefault(t["id"].split("::")[0], collections.Counter())[st] += 1
    for g, n in by.items():
        print(f"  {g:14} {n['PASS']:5} {n['FAIL']:5} {n['SKIP']:5}")
    tot = collections.Counter(st for _, st, _ in rows)
    print(f"  {'total':14} {tot['PASS']:5} {tot['FAIL']:5} {tot['SKIP']:5}")

    hard = [(t, m) for t, st, m in rows if st == "FAIL" and not optional(t)]
    soft = [(t, m) for t, st, m in rows if st == "FAIL" and optional(t)]
    for title, lst in (("FAILURES", hard), ("failures of optional behaviour (pedantic / RFC 5280 beyond the web PKI)", soft)):
        if lst:
            print(f"\n{title} ({len(lst)}):")
            for t, m in lst:
                got_ = "SUCCESS" if t["expected_result"] == "FAILURE" else "FAILURE"
                feats = f" [{', '.join(t['features'])}]" if t["features"] else ""
                print(f"  {t['id']}{feats}: expected {t['expected_result']}, netio {got_}" + (f" ({m})" if m else ""))
    sk = collections.Counter(m for _, st, m in rows if st == "SKIP")
    if sk:
        print("\nskipped:")
        for m, k in sk.most_common():
            print(f"  {k:5}  {m}")
    return 1 if hard else 0


if __name__ == "__main__":
    sys.exit(main())
