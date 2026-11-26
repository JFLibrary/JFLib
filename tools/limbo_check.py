"""limbo_check.py -- netio::x509::verify_chain against x509-limbo (C2SP/x509-limbo).

    python tools/limbo_check.py [--target linux|win|mac] [--system] [--id SUBSTRING] [--show ID] [--list]

Fetches limbo.json of the pinned commit into build/vectors (tools/suite_fetch.py; nothing in
git), hands each testcase to `coretest --vectors` (x509chain, apps/coretest/src/suitevec.rs) the
way the TLS client does it -- leaf and intermediates parsed with Cert::parse (an unreadable leaf: the
chain is refused; an unreadable intermediate is left out, as tls/conn.rs does), anchors that do
not parse left out (roots.rs),
verify_chain(chain, anchors, validation time, expected peer name, server / client usage), which
checks the name with Cert::names -- and holds the answer against expected_result.  A chain of
more than 20 certificates is judged by verify_chain and marked: tls/conn.rs refuses a
Certificate message that long.

Skipped, with the reason: testcases whose features netio states it lacks (lib/x509/lib.rs,
header: CRLs, certificate policies; also ML-DSA keys,
a caller-chosen chain depth, e-mail peer names, extended key usages other than serverAuth /
clientAuth).  Optional behaviour (the pedantic-* features, rfc5280-incompatible-with-webpki) is
run, and its failures are counted apart.  PASS / FAIL / SKIP per testcase id with --list;
exit code 1 on a failure that is not optional.

--system: the same testcases through the system's checks (`coretest --vectors`, x509system:
netio::roots::verify_system_roots -- Windows CryptoAPI with the testcase's trusted certificates
as its only roots, Apple's Security framework with them as its only anchors, on Linux
verify_chain), as Trust::System decides on these machines: the system's answer, then netio's
check of the leaf's name.  A server's chains only (the system checks those).  Each answer is
held against limbo's and against netio's own (x509chain): "same" where the system and netio
answer alike, otherwise than limbo (netio's differences, docs/EXTERNAL_SUITES.md); where the
system answers otherwise than both, SYSTEM_KNOWN says why (by testcase id), and a difference
not listed there is a failure.  The reason of each refusal (unknown CA, expired, name, usage,
another) is shown with --list."""
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

# features netio does not have (lib/x509/lib.rs: "Not checked: CRLs,
# certificate policies"); name constraints on directory names it processes
LACKS = {
    "has-crl": "CRLs are not checked (lib/x509/lib.rs header; OCSP is x509::ocsp, which limbo does not test)",
    "has-policy-constraints": "certificate policies are not checked (lib/x509/lib.rs header)",
    "has-cert-policies": "certificate policies are not checked (lib/x509/lib.rs header)",
    "no-cert-policies": "certificate policies are not checked (lib/x509/lib.rs header)",
    "has-mldsa": "ML-DSA keys: netio knows Ed25519, RSA, P-256 and P-384",
    "max-chain-depth": "no chain depth of the caller's choosing (verify_chain has its own limit)",
}
# where a system answers otherwise than limbo and netio's own check, and why: target ->
# [(regex on the testcase id, reason)]; --system --list shows each answer
SYSTEM_KNOWN = {
    "linux": [],
    "win": [],
    "mac": [],
}
REASONS = {0: "another reason", 1: "unknown CA", 2: "expired", 3: "name", 4: "revoked", 5: "usage"}

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


def line(t, op="x509chain"):
    """the x509chain (or x509system) line of a testcase (suitevec.rs)"""
    usage = b"\x01" if t["validation_kind"] == "CLIENT" else b"\x00"
    vt = t["validation_time"]
    now = int(datetime.datetime.fromisoformat(vt).timestamp()) if vt else int(time.time())
    p = t["expected_peer_name"]
    name = b"\x01" + p["value"].encode() if p else b"\x00"
    leaf = pem_ders(t["peer_certificate"])
    inter = [d for x in t["untrusted_intermediates"] for d in pem_ders(x)]
    trust = [d for x in t["trusted_certs"] for d in pem_ders(x)]
    args = [usage, now.to_bytes(8, "big", signed=True), name, len(inter).to_bytes(2, "big")] + leaf[:1] + inter + trust
    return op + " " + " ".join(a.hex() for a in args)


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
    if b[0] == 3 and len(b) > 1:
        return False, f"system ({REASONS.get(b[1], b[1])}): " + b[2:].decode("utf-8", "replace")
    return False, out


def system_skip(t):
    """why --system leaves a testcase out (beside skip_reason)"""
    if t["validation_kind"] != "SERVER":
        return "the system checks a server's chain (Trust::System), not a client's"
    return None


def known(target, t):
    """the reason a system's other answer is listed with, or None"""
    for pat, why in SYSTEM_KNOWN.get(target, []):
        if re.search(pat, t["id"]):
            return why
    return None


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


def answers(exe, run, op, timeout):
    """testcase id -> (accepted, why) for these testcases through op (x509chain / x509system)"""
    outs = []
    for i in range(0, len(run), 500):
        outs += run_all(exe, [line(t, op) for t in run[i:i + 500]], timeout)
    return {t["id"]: verdict(o) if not o.startswith(("timeout", "crash")) else (None, o) for t, o in zip(run, outs)}


def optional(t):
    return bool(OPTIONAL & set(t["features"]))


def system_main(opt, exe, tcs):
    """--system: the system's answer against limbo's, beside netio's own (x509chain)"""
    def skip(t):
        return skip_reason(t) or system_skip(t)
    run = [t for t in tcs if not skip(t)]
    sysg = answers(exe, run, "x509system", opt.timeout)
    own = answers(exe, run, "x509chain", opt.timeout)
    rows = []
    for t in tcs:
        why = skip(t)
        if why:
            rows.append((t, "SKIP", why))
            continue
        want = t["expected_result"] == "SUCCESS"
        acc, msg = sysg[t["id"]]
        if acc == want:
            rows.append((t, "PASS", msg))
        elif acc is not None and own[t["id"]][0] == acc:
            # netio's own check answers the same: one of its differences (docs/EXTERNAL_SUITES.md)
            rows.append((t, "SAME", msg))
        elif acc is not None and known(opt.target, t):
            rows.append((t, "KNOWN", msg + ("; " if msg else "") + "listed: " + known(opt.target, t)))
        else:
            rows.append((t, "FAIL", msg))
    if opt.show:
        for t, st, msg in rows:
            print(f"{t['id']} [{', '.join(t['features']) or 'no features'}] expected {t['expected_result']}: {st} {msg}\n")
            print(t["description"])
        return 0
    if opt.list:
        for t, st, msg in rows:
            print(f"{st} {t['id']}" + (f"  ({msg})" if msg and st != "PASS" else ""))
        print()
    print(f"limbo_check --system: C2SP/x509-limbo @ {COMMIT[:12]}, {len(tcs)} testcases ({opt.target}, through the system's checks)\n")
    print("  same: the system and netio's own check answer alike, otherwise than limbo (docs/EXTERNAL_SUITES.md)")
    print("  known: the system answers otherwise than netio, listed in SYSTEM_KNOWN with the reason\n")
    cols = ["PASS", "SAME", "KNOWN", "FAIL", "SKIP"]
    print(f"  {'group':14} " + " ".join(f"{c.lower():>5}" for c in cols))
    by = collections.OrderedDict()
    for t, st, _ in sorted(rows, key=lambda r: r[0]["id"]):
        by.setdefault(t["id"].split("::")[0], collections.Counter())[st] += 1
    tot = collections.Counter(st for _, st, _ in rows)
    for g, n in list(by.items()) + [("total", tot)]:
        print(f"  {g:14} " + " ".join(f"{n[c]:5}" for c in cols))
    hard = [(t, m) for t, st, m in rows if st == "FAIL" and not optional(t)]
    soft = [(t, m) for t, st, m in rows if st == "FAIL" and optional(t)]
    for title, lst in (("the system answers otherwise than netio and limbo, not listed (FAILURES)", hard),
                       ("the same in optional behaviour (pedantic / RFC 5280 beyond the web PKI)", soft)):
        if lst:
            print(f"\n{title} ({len(lst)}):")
            for t, m in lst:
                got_ = "SUCCESS" if t["expected_result"] == "FAILURE" else "FAILURE"
                feats = f" [{', '.join(t['features'])}]" if t["features"] else ""
                print(f"  {t['id']}{feats}: expected {t['expected_result']}, the system {got_}" + (f" ({m})" if m else ""))
    kn = collections.Counter(m.split("listed: ", 1)[1] for _, st, m in rows if st == "KNOWN")
    if kn:
        print("\nknown differences of the system:")
        for m, k in kn.most_common():
            print(f"  {k:5}  {m}")
    return 1 if hard else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--target", default="win" if os.name == "nt" else "mac" if sys.platform == "darwin" else "linux")
    ap.add_argument("--id", default="", help="testcases whose id contains this")
    ap.add_argument("--show", help="print one testcase's description and what netio said")
    ap.add_argument("--list", action="store_true", help="PASS / FAIL / SKIP for every testcase id")
    ap.add_argument("--timeout", type=float, default=120, help="seconds per batch of testcases")
    ap.add_argument("--system", action="store_true", help="through the system's checks (netio::roots::verify_system_roots)")
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
    if opt.system:
        return system_main(opt, exe, tcs)
    run = [t for t in tcs if not skip_reason(t)]
    got = answers(exe, run, "x509chain", opt.timeout)

    rows = []
    for t in tcs:
        why = skip_reason(t)
        if why:
            rows.append((t, "SKIP", why))
            continue
        acc, msg = got[t["id"]]
        ok = acc is not None and acc == (t["expected_result"] == "SUCCESS")
        if 1 + sum(len(pem_ders(x)) for x in t["untrusted_intermediates"]) > 20:
            # verify_chain takes any number; a TLS Certificate message of more is refused
            msg = (msg + "; " if msg else "") + "over 20 certificates: tls/conn.rs would refuse the message"
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
