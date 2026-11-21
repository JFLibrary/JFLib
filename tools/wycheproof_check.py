"""wycheproof_check.py -- lib/crypto against Project Wycheproof's test vectors (C2SP/wycheproof).

    python tools/wycheproof_check.py [--target linux|win] [--only SUBSTRING] [--acceptable] [-v]

Fetches the vector files of the pinned commit into build/vectors (tools/suite_fetch.py; nothing
in git), turns every test case into `coretest --vectors` lines (apps/coretest/src/suitevec.rs),
runs coretest once and holds each answer against the case's `result`:

  valid       must be accepted (and give the expected bytes)
  invalid     must be refused
  acceptable  may go either way (listed by flag; --acceptable lists each case); a wrong value is
              a failure

Covered, through the API the library's programs call:
  X25519             x25519::shared (low-order points: None)
  Ed25519            x509::Cert::verify -> ed25519::verify (strict)
  ECDSA P-256/P-384  DER: x509::Cert::verify (netio's Ecdsa-Sig-Value decoding) -> ecdsa::verify;
                     P1363: ecdsa::verify(r, s) on the two halves (JFLib has no P1363 decoder:
                     signatures of another length are skipped)
  AES-GCM            gcm::AesGcm seal and open: 12-byte nonces, 16-byte tags, AES-128 / AES-256
                     (other nonce sizes and AES-192 are skipped: the API does not take them)
  (X)ChaCha20-Poly1305  aead::seal / open, x_seal / x_open (12- / 24-byte nonces only)
  RSA PKCS #1 v1.5   x509::Cert::verify -> rsa::PublicKey::verify_pkcs1; signing:
                     rsa::PrivateKey::sign_pkcs1 (the sig_gen files)
  RSA-PSS            x509::Cert::verify -> verify_pss with the group's salt length (MGF1 of the
                     message hash only, like the API)
  HKDF, HMAC         hmac::hkdf_extract / hkdf_expand and Hmac over SHA-1 / 256 / 384 / 512
Hashes other than SHA-256 / 384 / 512 (SHA-1 / 224, SHA-512/t, SHA-3, SHAKE) are skipped for
signatures: HashAlg has no others.  Exit code 1 on any failure."""
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
REPO = "C2SP/wycheproof"
COMMIT = "65ca494727a52944c204441c860b039e6786b99b"

FILES = (["x25519", "ed25519", "aes_gcm", "chacha20_poly1305", "xchacha20_poly1305"]
         + [f"ecdsa_secp256r1_{h}" for h in ("sha256", "sha512", "sha256_p1363", "sha512_p1363")]
         + [f"ecdsa_secp384r1_{h}" for h in ("sha256", "sha384", "sha512", "sha384_p1363", "sha512_p1363")]
         + [f"rsa_signature_{b}_{h}" for b in (2048, 3072, 4096, 8192) for h in ("sha256", "sha384", "sha512")]
         + [f"rsa_pkcs1_{b}_sig_gen" for b in (1024, 1536, 2048, 3072, 4096)]
         + [f"rsa_pss_{x}" for x in ("2048_sha256_mgf1_0", "2048_sha256_mgf1_32", "2048_sha384_mgf1_48", "3072_sha256_mgf1_32",
                                     "4096_sha256_mgf1_32", "4096_sha384_mgf1_48", "4096_sha512_mgf1_32", "4096_sha512_mgf1_64", "misc")]
         + [f"{k}_{h}" for k in ("hkdf", "hmac") for h in ("sha1", "sha256", "sha384", "sha512")])

SIG_HASH = {"SHA-256": 1, "SHA-384": 2, "SHA-512": 3}          # suitevec.rs: hash ids
MAC_HASH = {"SHA-1": 0, "SHA-256": 1, "SHA-384": 2, "SHA-512": 3}
HASHLIB = {"SHA-256": "sha256", "SHA-384": "sha384", "SHA-512": "sha512"}


class Case:
    """one test case: the coretest lines it needs and how their answers make a verdict"""
    def __init__(self, file, t, ops=(), judge=None, skip=None, size=None):
        self.file, self.tcid, self.result = file, t["tcId"], t["result"]
        self.flags, self.comment = t.get("flags", []), t.get("comment", "")
        # skip: why the API cannot take the case ("{}": the sizes, listed once per reason)
        self.ops, self.judge, self.skip, self.size = list(ops), judge, skip, size
        self.verdict = self.detail = None


def line(alg, *args):
    return alg + " " + " ".join(a.hex() for a in args)


def u16(n):
    return n.to_bytes(2, "big")


def accept_byte(outs):
    """01 / 00 of a verification"""
    return ("accept" if outs[0] == "01" else "reject"), outs[0]


def value(want):
    """a computation: the expected bytes, '-' a refusal, anything else a wrong value"""
    def judge(outs):
        if outs[0] == "-":
            return "reject", "-"
        return ("accept" if outs[0] == want.hex() else "wrong"), outs[0][:64]
    return judge


# ---------------------------------------------------------------- the files' groups
def x25519(file, j):
    for g in j["testGroups"]:
        for t in g["tests"]:
            k, u, s = (bytes.fromhex(t[x]) for x in ("private", "public", "shared"))
            if len(k) != 32 or len(u) != 32:
                yield Case(file, t, skip="key not 32 bytes: x25519::shared takes [u8; 32]")
                continue
            yield Case(file, t, [line("x25519shared", k, u)], value(s))


def ed25519(file, j):
    for g in j["testGroups"]:
        pk = bytes.fromhex(g["publicKey"]["pk"])
        for t in g["tests"]:
            if len(pk) != 32:
                yield Case(file, t, skip="public key not 32 bytes: PubKey::Ed25519 holds [u8; 32]")
                continue
            yield Case(file, t, [line("certverify", b"\x01", b"\x01", b"", b"", pk, b"", bytes.fromhex(t["msg"]), bytes.fromhex(t["sig"]))], accept_byte)


def ecdsa(file, j):
    for g in j["testGroups"]:
        curve = g["publicKey"]["curve"]
        kind, nb = {"secp256r1": (3, 32), "secp384r1": (4, 48)}[curve]
        pt = bytes.fromhex(g["publicKey"]["uncompressed"])
        h = g["sha"]
        for t in g["tests"]:
            msg, sig = bytes.fromhex(t["msg"]), bytes.fromhex(t["sig"])
            if h not in SIG_HASH:
                yield Case(file, t, skip=f"{h}: HashAlg has SHA-256/384/512 only")
            elif g["type"] == "EcdsaVerify":
                yield Case(file, t, [line("certverify", bytes([kind]), b"\x04", bytes([SIG_HASH[h]]), b"", pt, b"", msg, sig)], accept_byte)
            elif len(sig) != 2 * nb:
                yield Case(file, t, skip=f"P1363 signatures of {{}} bytes, not {2 * nb}: JFLib has no P1363 decoder (ecdsa::verify takes r, s)", size=len(sig))
            else:
                d = hashlib.new(HASHLIB[h], msg).digest()
                yield Case(file, t, [line("ecdsa", bytes([kind - 2]), pt, d, sig[:nb], sig[nb:])], accept_byte)


def aead(file, j):
    alg = j["algorithm"]
    for g in j["testGroups"]:
        for t in g["tests"]:
            key, iv, aad, msg, ct, tag = (bytes.fromhex(t[x]) for x in ("key", "iv", "aad", "msg", "ct", "tag"))
            if alg == "AES-GCM":
                seal, open_, ivlen = "aesgcm", "aesgcmopen", 12
                if len(key) not in (16, 32):
                    yield Case(file, t, skip=f"AES-{len(key) * 8}: aes.rs takes 16- and 32-byte keys only")
                    continue
            elif alg == "CHACHA20-POLY1305":
                seal, open_, ivlen = "aead", "aeadopen", 12
            else:
                seal, open_, ivlen = "xaead", "xaeadopen", 24
            if len(iv) != ivlen:
                yield Case(file, t, skip=f"nonces of {{}} bytes: the API takes [u8; {ivlen}]", size=len(iv))
                continue
            if len(tag) != 16:
                yield Case(file, t, skip="tags of {} bytes: the API takes [u8; 16]", size=len(tag))
                continue
            ops = [line(open_, key, iv, aad, ct + tag)]
            if t["result"] == "valid":
                ops.append(line(seal, key, iv, aad, msg))

            def judge(outs, msg=msg, sealed=ct + tag):
                if outs[0] == "-":
                    return "reject", "open refused"
                if outs[0] != msg.hex():
                    return "wrong", "open gave other plaintext " + outs[0][:48]
                if len(outs) > 1 and outs[1] != sealed.hex():
                    return "wrong", "seal gave " + outs[1][:48]
                return "accept", ""
            yield Case(file, t, ops, judge)


def rsa_n_e(g):
    k = g["publicKey"] if "publicKey" in g else g["privateKey"]
    return bytes.fromhex(k["modulus"]), bytes.fromhex(k["publicExponent"])


def der_ints(b):
    """the INTEGERs of a DER SEQUENCE (enough for PKCS #8 / RSAPrivateKey)"""
    def tlv(b, i):
        tag, n = b[i], b[i + 1]
        i += 2
        if n & 0x80:
            k = n & 0x7f
            n = int.from_bytes(b[i:i + k], "big")
            i += k
        return tag, b[i:i + n], i + n
    tag, body, _ = tlv(b, 0)
    out, i = [], 0
    while i < len(body):
        tag, v, i = tlv(body, i)
        out.append((tag, v))
    return out


def rsa_private(g):
    """n, e, d, p, q from the group's PKCS #8 key"""
    items = der_ints(bytes.fromhex(g["privateKeyPkcs8"]))
    inner = [v for tag, v in der_ints(items[2][1])]           # OCTET STRING { RSAPrivateKey }
    return inner[1:6]


def rsa_pkcs1(file, j):
    for g in j["testGroups"]:
        n, e = rsa_n_e(g)
        h = g["sha"]
        for t in g["tests"]:
            msg, sig = bytes.fromhex(t["msg"]), bytes.fromhex(t["sig"])
            if h not in SIG_HASH:
                yield Case(file, t, skip=f"{h}: HashAlg has SHA-256/384/512 only")
                continue
            ops = [line("certverify", b"\x02", b"\x02", bytes([SIG_HASH[h]]), b"", n, e, msg, sig)]
            if g["type"] == "RsassaPkcs1Generate":
                nn, ee, d, p, q = rsa_private(g)
                ops.append(line("rsasign", bytes([SIG_HASH[h]]), nn, ee, d, p, q, hashlib.new(HASHLIB[h], msg).digest()))

            def judge(outs, sig=sig):
                if len(outs) > 1 and outs[1] != sig.hex():
                    return ("reject" if outs[1] == "-" else "wrong"), "sign_pkcs1 gave " + outs[1][:48]
                return accept_byte(outs)
            yield Case(file, t, ops, judge)


def rsa_pss(file, j):
    for g in j["testGroups"]:
        n, e = rsa_n_e(g)
        h, mh = g["sha"], g["mgfSha"]
        for t in g["tests"]:
            if h not in SIG_HASH:
                yield Case(file, t, skip=f"{h}: HashAlg has SHA-256/384/512 only")
            elif g["mgf"] != "MGF1" or mh != h:
                yield Case(file, t, skip=f"MGF1 with {mh} for {h}: verify_pss uses MGF1 of the message hash")
            else:
                yield Case(file, t, [line("certverify", b"\x02", b"\x03", bytes([SIG_HASH[h]]), u16(g["sLen"]), n, e,
                                          bytes.fromhex(t["msg"]), bytes.fromhex(t["sig"]))], accept_byte)


def hkdf(file, j):
    h = j["algorithm"].removeprefix("HKDF-")
    for g in j["testGroups"]:
        for t in g["tests"]:
            yield Case(file, t, [line("hkdf", bytes([MAC_HASH[h]]), *(bytes.fromhex(t[x]) for x in ("salt", "ikm", "info")), u16(t["size"]))],
                       value(bytes.fromhex(t["okm"])))


def hmac(file, j):
    h = {"HMACSHA1": "SHA-1", "HMACSHA256": "SHA-256", "HMACSHA384": "SHA-384", "HMACSHA512": "SHA-512"}[j["algorithm"]]
    for g in j["testGroups"]:
        n = g["tagSize"] // 8
        for t in g["tests"]:
            tag = bytes.fromhex(t["tag"])

            # the library gives the full tag; comparing a truncated one is the caller's part
            def judge(outs, tag=tag, n=n):
                return ("accept" if len(tag) == n and outs[0][:2 * n] == tag.hex() else "reject"), outs[0][:2 * n]
            yield Case(file, t, [line("hmac", bytes([MAC_HASH[h]]), bytes.fromhex(t["key"]), bytes.fromhex(t["msg"]))], judge)


def handler(name):
    for prefix, fn in (("x25519", x25519), ("ed25519", ed25519), ("ecdsa", ecdsa), ("aes_gcm", aead), ("chacha", aead), ("xchacha", aead),
                       ("rsa_signature", rsa_pkcs1), ("rsa_pkcs1", rsa_pkcs1), ("rsa_pss", rsa_pss), ("hkdf", hkdf), ("hmac", hmac)):
        if name.startswith(prefix):
            return fn
    raise ValueError(name)


# ---------------------------------------------------------------- run and report
def run_coretest(exe, lines):
    with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
        f.write("\n".join(lines) + "\n")
        path = f.name
    try:
        r = subprocess.run([str(exe), "--vectors", path], capture_output=True, text=True, timeout=1800)
    finally:
        os.unlink(path)
    if r.returncode != 0:
        sys.exit(f"wycheproof_check: coretest failed ({r.returncode}): {r.stderr.strip()[-2000:]}")
    out = r.stdout.split("\n")
    if len(out) < len(lines):
        sys.exit(f"wycheproof_check: coretest answered {len(out)} of {len(lines)} lines")
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--target", default="win" if os.name == "nt" else "linux")
    ap.add_argument("--only", default="", help="files whose name contains this")
    ap.add_argument("--acceptable", action="store_true", help="list every acceptable case and what lib/crypto did")
    ap.add_argument("-v", "--verbose", action="store_true", help="list skipped cases one by one")
    opt = ap.parse_args()
    exe = ROOT / "build" / opt.target / ("coretest.exe" if opt.target == "win" else "coretest")
    if not exe.exists():
        sys.exit(f"wycheproof_check: {exe.relative_to(ROOT)} is missing (python build_rs.py app coretest --target {opt.target})")
    names = [n for n in FILES if opt.only in n]
    folder = fetch(REPO, COMMIT, [f"testvectors_v1/{n}_test.json" for n in names])
    cases = []
    for n in names:
        file = f"{n}_test.json"
        j = json.loads((folder / "testvectors_v1" / file).read_text(encoding="utf-8"))
        cases += list(handler(n)(file, j))
    lines = [op for c in cases if not c.skip for op in c.ops]
    out = run_coretest(exe, lines) if lines else []
    i = 0
    for c in cases:
        if c.skip:
            continue
        outs = out[i:i + len(c.ops)]
        i += len(c.ops)
        c.verdict, c.detail = c.judge(outs)

    def status(c):
        if c.skip:
            return "skip"
        if c.verdict == "wrong":
            return "fail"
        if c.result == "valid":
            return "pass" if c.verdict == "accept" else "fail"
        if c.result == "invalid":
            return "pass" if c.verdict == "reject" else "fail"
        return "pass"

    per_file = collections.OrderedDict()
    for c in cases:
        per_file.setdefault(c.file, collections.Counter())[status(c)] += 1
    print(f"wycheproof_check: C2SP/wycheproof @ {COMMIT[:12]}, {len(cases)} cases from {len(names)} files ({opt.target})\n")
    print(f"  {'file':44} {'pass':>5} {'fail':>5} {'skip':>5}")
    for f, n in per_file.items():
        print(f"  {f:44} {n['pass']:5} {n['fail']:5} {n['skip']:5}")
    tot = collections.Counter(status(c) for c in cases)
    print(f"  {'total':44} {tot['pass']:5} {tot['fail']:5} {tot['skip']:5}")

    fails = [c for c in cases if status(c) == "fail"]
    if fails:
        print(f"\nFAILURES ({len(fails)}):")
        for c in fails:
            print(f"  {c.file} tcId {c.tcid}: expected {c.result}, lib {c.verdict} {c.detail or ''}\n      flags {c.flags}  {c.comment}")

    skips = collections.OrderedDict()
    for c in cases:
        if c.skip:
            skips.setdefault((c.file.split('_test')[0], c.skip), []).append(c.size)
    if skips:
        print("\nskipped (not expressible through the API):")
        for (f, why), sizes in skips.items():
            print(f"  {len(sizes):5}  {f}: {why.format(', '.join(str(x) for x in sorted(set(sizes))))}")
        if opt.verbose:
            for c in cases:
                if c.skip:
                    print(f"    {c.file} tcId {c.tcid} ({c.result}) {c.flags}: {c.skip}")

    acc = [c for c in cases if c.result == "acceptable" and not c.skip]
    if acc:
        print("\nacceptable cases (either answer passes), by flag set:")
        by = collections.Counter((c.file.split('_test')[0], " ".join(sorted(c.flags)), c.verdict) for c in acc)
        for (f, flags, v), k in sorted(by.items()):
            print(f"  {k:5}  {f}: [{flags}] -> {v}ed" if v != "wrong" else f"  {k:5}  {f}: [{flags}] -> WRONG")
        if opt.acceptable:
            for c in acc:
                print(f"    {c.file} tcId {c.tcid} {c.flags}: {c.verdict} ({c.comment})")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
