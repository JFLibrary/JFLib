"""acvp_check.py -- lib/crypto against NIST's ACVP test vectors (usnistgov/ACVP-Server, gen-val).

    python tools/acvp_check.py [--target linux|win] [--only SUBSTRING] [-v]

Fetches the internalProjection.json files (inputs and expected answers) of the pinned commit into
build/vectors (tools/suite_fetch.py; nothing in git), turns every test case into `coretest
--vectors` lines (apps/coretest/src/crypto.rs) and compares the answers.

Covered, through the API:
  SHA2-224, SHA3-224 / 256 / 384 / 512   sha224, sha3_*: AFT and the standard Monte Carlo test
                     (coretest's hashmct); messages of a whole number of bytes (the API hashes
                     bytes; bit-oriented messages are skipped); LDT (gigabyte messages) skipped
  SHAKE-128 / 256    shake128 / shake256: AFT and VOT with whole bytes in and out
  AES-ECB            aes::Aes encrypt / decrypt with 128-, 192- and 256-bit keys: GFSBox, KeySBox,
                     VarTxt, VarKey, MMT and the Monte Carlo test (AESAVS 6.4.1)
  ML-KEM             mlkem::keygen (d || z), encaps, decaps (valid and modified ciphertexts), the
                     encapsulation key check (encaps refuses) and the decapsulation key check
                     (decaps refuses): ML-KEM-512, 768 and 1024
  DetECDSA SigGen    ecdsa::sign with an empty extra (RFC 6979) on P-256 and P-384, SHA2-224 to
                     SHA2-512 and SHA3-224 to SHA3-512 (other curves, SHA2-512/t and the groups
                     with SP 800-106's randomized hashing of the message skipped)
Exit code 1 on any failure."""
import argparse
import collections
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from suite_fetch import fetch  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
REPO = "usnistgov/ACVP-Server"
COMMIT = "975de31eb83d87039ec88934fdc47d8c312b892d"

# coretest's hash ids (crypto.rs hash_of) and hashlib's names
HASH = {"SHA2-224": (4, "sha224"), "SHA2-256": (1, "sha256"), "SHA2-384": (2, "sha384"), "SHA2-512": (3, "sha512"),
        "SHA3-224": (5, "sha3_224"), "SHA3-256": (6, "sha3_256"), "SHA3-384": (7, "sha3_384"), "SHA3-512": (8, "sha3_512")}
MLKEM = {"ML-KEM-512": 1, "ML-KEM-768": 2, "ML-KEM-1024": 3}
CURVE = {"P-256": (1, 32), "P-384": (2, 48)}

FILES = ["SHA2-224-1.0", "SHA3-224-2.0", "SHA3-256-2.0", "SHA3-384-2.0", "SHA3-512-2.0",
         "SHAKE-128-1.0", "SHAKE-256-1.0", "ACVP-AES-ECB-1.0",
         "ML-KEM-keyGen-FIPS203", "ML-KEM-encapDecap-FIPS203", "DetECDSA-SigGen-FIPS186-5"]


class Case:
    def __init__(self, file, tcid, ops=(), want=None, skip=None):
        self.file, self.tcid, self.ops, self.want, self.skip = file, tcid, list(ops), want, skip
        self.got = None

    def ok(self):
        return self.got == self.want


def line(alg, *args):
    return alg + " " + " ".join(a.hex() for a in args)


def u16(n):
    return n.to_bytes(2, "big")


def hx(s):
    return bytes.fromhex(s)


# ---------------------------------------------------------------- the files
def sha(file, j):
    alg = j["algorithm"]
    hid, _ = HASH[alg]
    op = "sha224" if alg == "SHA2-224" else alg.lower().replace("-", "_")
    for g in j["testGroups"]:
        for t in g["tests"]:
            if g["testType"] == "LDT":
                yield Case(file, t["tcId"], skip="LDT: messages of gigabytes")
            elif t["len"] % 8:
                yield Case(file, t["tcId"], skip="messages of a fraction of a byte: the API hashes bytes")
            elif g["testType"] == "AFT":
                yield Case(file, t["tcId"], [line(op, hx(t["msg"])[:t["len"] // 8])], [t["md"].lower()])
            else:
                kind = bytes([3 if alg.startswith("SHA3") else 2])
                yield Case(file, t["tcId"], [line("hashmct", kind, bytes([hid]), hx(t["msg"]))],
                           ["".join(r["md"].lower() for r in t["resultsArray"])])


def shake(file, j):
    op = "shake128" if j["algorithm"] == "SHAKE-128" else "shake256"
    for g in j["testGroups"]:
        for t in g["tests"]:
            if g["testType"] not in ("AFT", "VOT"):
                yield Case(file, t["tcId"], skip=f"{g['testType']}: not run (the SHAKE Monte Carlo test)")
            elif t["len"] % 8 or t["outLen"] % 8:
                yield Case(file, t["tcId"], skip="a fraction of a byte in or out: the API takes and gives bytes")
            else:
                yield Case(file, t["tcId"], [line(op, hx(t["msg"])[:t["len"] // 8], u16(t["outLen"] // 8))], [t["md"].lower()])


def aes_ecb(file, j):
    for g in j["testGroups"]:
        dec = g["direction"] == "decrypt"
        d = bytes([1 if dec else 0])
        for t in g["tests"]:
            if g["testType"] == "MCT":
                start = hx(t["ct"] if dec else t["pt"])
                want = "".join(r["key"].lower() + (r["ct"] + r["pt"] if dec else r["pt"] + r["ct"]).lower() for r in t["resultsArray"])
                yield Case(file, t["tcId"], [line("aesecbmct", d, hx(t["key"]), start)], [want])
            else:
                src, dst = (t["ct"], t["pt"]) if dec else (t["pt"], t["ct"])
                yield Case(file, t["tcId"], [line("aesecb", d, hx(t["key"]), hx(src))], [dst.lower()])


def mlkem_keygen(file, j):
    for g in j["testGroups"]:
        p = bytes([MLKEM[g["parameterSet"]]])
        for t in g["tests"]:
            yield Case(file, t["tcId"], [line("mlkemkeygen", p, hx(t["d"]) + hx(t["z"]))], [(t["ek"] + t["dk"]).lower()])


def mlkem_encdec(file, j):
    for g in j["testGroups"]:
        p = bytes([MLKEM[g["parameterSet"]]])
        f = g["function"]
        for t in g["tests"]:
            if f == "encapsulation":
                yield Case(file, t["tcId"], [line("mlkemencaps", p, hx(t["ek"]), hx(t["m"]))], [(t["c"] + t["k"]).lower()])
            elif f == "decapsulation":
                yield Case(file, t["tcId"], [line("mlkemdecaps", p, hx(t["dk"]), hx(t["c"]))], [t["k"].lower()])
            elif f == "encapsulationKeyCheck":
                # the check is encaps's: a key it refuses gives "-", one it takes ct || key
                n = CT_LEN[p[0]] + 32
                yield Case(file, t["tcId"], [line("mlkemencaps", p, hx(t["ek"]), bytes(32))], [f"ok:{n}" if t["testPassed"] else "-"])
            else:
                ct = bytes(CT_LEN[p[0]])
                yield Case(file, t["tcId"], [line("mlkemdecaps", p, hx(t["dk"]), ct)], ["ok:32" if t["testPassed"] else "-"])


# ML-KEM's ciphertext length by coretest's parameter id
CT_LEN = {1: 768, 2: 1088, 3: 1568}


def key_check(got, want):
    """a passing key check ("ok:N") is an answer of N bytes in hex: then want, any other answer
    (a refusal, a wrong length, the text "ok:N" itself) marked as wrong; answers to anything
    but a passing check stay as they are"""
    if not want.startswith("ok:"):
        return got
    if len(got) == 2 * int(want[3:]) and all(ch in "0123456789abcdef" for ch in got):
        return want
    return f"not {want}: {got}"


def det_ecdsa(file, j):
    for g in j["testGroups"]:
        if g["curve"] not in CURVE:
            yield from (Case(file, t["tcId"], skip=f"{g['curve']}: ecdsa has P-256 and P-384") for t in g["tests"])
            continue
        if g["hashAlg"] not in HASH:
            yield from (Case(file, t["tcId"], skip=f"{g['hashAlg']}: HashAlg has no SHA-512/t") for t in g["tests"])
            continue
        if g.get("conformance") == "SP800-106":
            yield from (Case(file, t["tcId"], skip="SP 800-106 randomized hashing of the message: not ECDSA's part") for t in g["tests"])
            continue
        cid, nb = CURVE[g["curve"]]
        hid, hname = HASH[g["hashAlg"]]
        d = int(g["d"], 16).to_bytes(nb, "big")
        for t in g["tests"]:
            dg = hashlib.new(hname, hx(t["message"])).digest()
            want = int(t["r"], 16).to_bytes(nb, "big") + int(t["s"], 16).to_bytes(nb, "big")
            yield Case(file, t["tcId"], [line("ecdsasign", bytes([cid]), bytes([hid]), d, dg, b"")], [want.hex()])


def handler(name):
    if name.startswith("SHA2") or name.startswith("SHA3"):
        return sha
    return {"SHAKE-128-1.0": shake, "SHAKE-256-1.0": shake, "ACVP-AES-ECB-1.0": aes_ecb, "ML-KEM-keyGen-FIPS203": mlkem_keygen,
            "ML-KEM-encapDecap-FIPS203": mlkem_encdec, "DetECDSA-SigGen-FIPS186-5": det_ecdsa}[name]


# ---------------------------------------------------------------- run and report
def run_coretest(exe, lines):
    with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
        f.write("\n".join(lines) + "\n")
        path = f.name
    try:
        r = subprocess.run([str(exe), "--vectors", path], capture_output=True, text=True, timeout=3600)
    finally:
        os.unlink(path)
    if r.returncode != 0:
        sys.exit(f"acvp_check: coretest failed ({r.returncode}): {r.stderr.strip()[-2000:]}")
    out = r.stdout.splitlines()
    if len(out) != len(lines):
        sys.exit(f"acvp_check: coretest answered {len(out)} lines to {len(lines)}")
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--target", default="win" if os.name == "nt" else "linux")
    ap.add_argument("--only", default="", help="files whose name contains this")
    ap.add_argument("-v", "--verbose", action="store_true", help="list skipped cases one by one")
    opt = ap.parse_args()
    exe = ROOT / "build" / opt.target / ("coretest.exe" if opt.target == "win" else "coretest")
    if not exe.exists():
        sys.exit(f"acvp_check: {exe.relative_to(ROOT)} is missing (python build_rs.py app coretest --target {opt.target})")
    names = [n for n in FILES if opt.only in n]
    folder = fetch(REPO, COMMIT, [f"gen-val/json-files/{n}/internalProjection.json" for n in names])
    cases = []
    for n in names:
        j = json.loads((folder / "gen-val" / "json-files" / n / "internalProjection.json").read_text(encoding="utf-8"))
        cases += list(handler(n)(n, j))
    lines = [op for c in cases if not c.skip for op in c.ops]
    out = run_coretest(exe, lines) if lines else []
    i = 0
    for c in cases:
        if c.skip:
            continue
        got = out[i:i + len(c.ops)]
        i += len(c.ops)
        c.got = [key_check(g, w) for g, w in zip(got, c.want, strict=True)]

    per = collections.OrderedDict()
    for c in cases:
        per.setdefault(c.file, collections.Counter())["skip" if c.skip else "pass" if c.ok() else "fail"] += 1
    print(f"acvp_check: usnistgov/ACVP-Server @ {COMMIT[:12]}, {len(cases)} cases from {len(names)} files ({opt.target})\n")
    print(f"  {'file':34} {'pass':>5} {'fail':>5} {'skip':>5}")
    for f, n in per.items():
        print(f"  {f:34} {n['pass']:5} {n['fail']:5} {n['skip']:5}")
    tot = collections.Counter("skip" if c.skip else "pass" if c.ok() else "fail" for c in cases)
    print(f"  {'total':34} {tot['pass']:5} {tot['fail']:5} {tot['skip']:5}")
    fails = [c for c in cases if not c.skip and not c.ok()]
    if fails:
        print(f"\nFAILURES ({len(fails)}):")
        for c in fails[:50]:
            print(f"  {c.file} tcId {c.tcid}: got {c.got[0][:64]}\n      want {c.want[0][:64]}")
    skips = collections.Counter((c.file, c.skip) for c in cases if c.skip)
    if skips:
        print("\nskipped (not expressible through the API):")
        for (f, why), n in skips.items():
            print(f"  {n:5}  {f}: {why}")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
