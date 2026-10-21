"""crypto_check.py -- lib/crypto against independent implementations with random inputs.

    python tools/crypto_check.py [--target win|linux] [-n CASES]

Writes random cases, lets coretest --vectors answer them and compares with Python's hashlib /
hmac and, when installed, the `cryptography` package (ChaCha20-Poly1305, AES-GCM, X25519,
Ed25519, HKDF; RSA PKCS #1 v1.5 / PSS and ECDSA P-256 / P-384 signatures of random keys, also
forged ones that must fail).  Under Windows without `cryptography` the script runs itself again inside WSL (the
Linux build of coretest).  Exit code 1 on any difference."""
import argparse
import hashlib
import hmac
import os
import random
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def have_cryptography():
    try:
        import cryptography  # noqa: F401
        return True
    except ImportError:
        return False


def cases(n, rnd):
    out = []
    b = lambda k: rnd.randbytes(k)  # noqa: E731
    for _ in range(n):
        size = rnd.choice([0, 1, 55, 56, 63, 64, 65, 111, 112, 127, 128, 129, 1000, 4097])
        out.append(("sha256", [b(size)]))
        out.append(("sha512", [b(size)]))
        out.append(("sha384", [b(size)]))
        out.append(("hmac256", [b(rnd.randint(0, 150)), b(size)]))
        out.append(("hmac512", [b(rnd.randint(0, 200)), b(size)]))
        out.append(("blake2b", [b(rnd.randint(0, 64)), b(size), bytes([rnd.randint(1, 64)])]))
        out.append(("hkdf256", [b(rnd.randint(0, 40)), b(rnd.randint(1, 60)), b(rnd.randint(0, 30)), bytes([rnd.randint(1, 255)])]))
        if have_cryptography():
            out.append(("aead", [b(32), b(12), b(rnd.randint(0, 40)), b(size)]))
            out.append(("x25519", [b(32), b(32)]))
            seed = b(32)
            out.append(("ed25519pub", [seed]))
            out.append(("ed25519sign", [seed, b(size)]))
            klen = rnd.choice([16, 32])
            out.append(("aesgcm", [b(klen), b(12), b(rnd.randint(0, 40)), b(size)]))
            out.append(("aesgcmopen", [b(klen), b(12), b(rnd.randint(0, 40)), b(size)]))
    if have_cryptography():
        out += signature_cases(max(4, n // 4), rnd)
    if have_cryptography():
        out += p256_cases(max(4, n // 2), rnd)
    return out


def p256_cases(n, rnd):
    """P-256 public keys and ECDH secrets of random scalars (and a point off the curve)"""
    from cryptography.hazmat.primitives.asymmetric import ec
    order = 0xFFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551
    out = []
    for i in range(n):
        k = rnd.randrange(1, order) if i % 5 else rnd.choice([1, 2, order - 1, order - 2])
        kb = k.to_bytes(32, "big")
        pub = ec.derive_private_key(k, ec.SECP256R1()).public_key().public_numbers()
        out.append(("p256pub", [kb], b"\x04" + pub.x.to_bytes(32, "big") + pub.y.to_bytes(32, "big")))
        peer = ec.generate_private_key(ec.SECP256R1())
        pn = peer.public_key().public_numbers()
        pt = b"\x04" + pn.x.to_bytes(32, "big") + pn.y.to_bytes(32, "big")
        want = ec.derive_private_key(k, ec.SECP256R1()).exchange(ec.ECDH(), peer.public_key())
        out.append(("p256ecdh", [kb, pt], want))
        bad = pt[:40] + bytes([pt[40] ^ 1]) + pt[41:]
        out.append(("p256ecdh", [kb, bad], None))
    return out


_RSA_KEYS = {}


def signature_cases(n, rnd):
    """RSA and ECDSA signatures by OpenSSL; every third one damaged (must not verify)"""
    from cryptography.hazmat.primitives.asymmetric import ec, padding, rsa, utils
    from cryptography.hazmat.primitives import hashes
    hs = {1: hashes.SHA256(), 2: hashes.SHA384(), 3: hashes.SHA512()}
    out = []
    for i in range(n):
        msg = rnd.randbytes(rnd.randint(0, 300))
        hid = rnd.choice([1, 2, 3])
        bits = rnd.choice([2048, 3072, 4096])
        if bits not in _RSA_KEYS:
            _RSA_KEYS[bits] = rsa.generate_private_key(65537, bits)
        key = _RSA_KEYS[bits]
        nb = key.public_key().public_numbers().n.to_bytes(bits // 8, "big")
        h = hs[hid]
        digest = hashlib.new(h.name, msg).digest()
        pkcs = key.sign(msg, padding.PKCS1v15(), h)
        pss = key.sign(msg, padding.PSS(padding.MGF1(h), h.digest_size), h)
        bad = i % 3 == 2
        flip = lambda x: x[:5] + bytes([x[5] ^ 1]) + x[6:] if bad else x  # noqa: E731
        out.append(("rsapkcs1", [nb, b"\x01\x00\x01", bytes([hid]), digest, flip(pkcs)], not bad))
        out.append(("rsapss", [nb, b"\x01\x00\x01", bytes([hid]), digest, flip(pss)], not bad))
        cid, curve, nbytes, hh = rnd.choice([(1, ec.SECP256R1(), 32, hashes.SHA256()), (2, ec.SECP384R1(), 48, hashes.SHA384())])
        k = ec.generate_private_key(curve)
        pn = k.public_key().public_numbers()
        pt = b"\x04" + pn.x.to_bytes(nbytes, "big") + pn.y.to_bytes(nbytes, "big")
        r, s_ = utils.decode_dss_signature(k.sign(msg, ec.ECDSA(hh)))
        dg = hashlib.new(hh.name, msg).digest()
        out.append(("ecdsa", [bytes([cid]), pt, flip(dg) if bad else dg, r.to_bytes(nbytes, "big"), s_.to_bytes(nbytes, "big")], not bad))
    return out


def expect(alg, a):
    if alg in ("rsapkcs1", "rsapss", "ecdsa"):
        raise AssertionError("signature cases carry their answer")
    if alg == "sha384":
        return hashlib.sha384(a[0]).digest()
    if alg == "sha256":
        return hashlib.sha256(a[0]).digest()
    if alg == "sha512":
        return hashlib.sha512(a[0]).digest()
    if alg == "hmac256":
        return hmac.new(a[0], a[1], hashlib.sha256).digest()
    if alg == "hmac512":
        return hmac.new(a[0], a[1], hashlib.sha512).digest()
    if alg == "blake2b":
        return hashlib.blake2b(a[1], key=a[0], digest_size=a[2][0]).digest()
    if alg == "hkdf256":
        salt = a[0] or bytes(32)
        prk = hmac.new(salt, a[1], hashlib.sha256).digest()
        okm, t, i = b"", b"", 1
        while len(okm) < a[3][0]:
            t = hmac.new(prk, t + a[2] + bytes([i]), hashlib.sha256).digest()
            okm += t
            i += 1
        return okm[:a[3][0]]
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric import ed25519, x25519
    from cryptography.hazmat.primitives.ciphers.aead import ChaCha20Poly1305
    raw = serialization.Encoding.Raw, serialization.PublicFormat.Raw
    if alg == "aead":
        return ChaCha20Poly1305(a[0]).encrypt(a[1], a[3], a[2])
    from cryptography.hazmat.primitives.ciphers.aead import AESGCM
    if alg == "aesgcm":
        return AESGCM(a[0]).encrypt(a[1], a[3], a[2])
    if alg == "aesgcmopen":
        return a[3]
    if alg == "x25519":
        u = bytearray(a[1])
        u[31] &= 0x7f  # RFC 7748: the top bit is ignored; cryptography rejects nothing here
        try:
            return x25519.X25519PrivateKey.from_private_bytes(a[0]).exchange(x25519.X25519PublicKey.from_public_bytes(bytes(u)))
        except ValueError:
            return None  # all-zero result: cryptography refuses it
    if alg == "ed25519pub":
        return ed25519.Ed25519PrivateKey.from_private_bytes(a[0]).public_key().public_bytes(*raw)
    if alg == "ed25519sign":
        return ed25519.Ed25519PrivateKey.from_private_bytes(a[0]).sign(a[1])
    raise ValueError(alg)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", default="win" if os.name == "nt" else "linux")
    ap.add_argument("-n", type=int, default=40)
    ap.add_argument("--seed", type=int, default=None)
    opt = ap.parse_args()
    if os.name == "nt" and not have_cryptography():
        # the full matrix needs `cryptography`: WSL usually has it
        wsl = "/mnt/" + str(ROOT)[0].lower() + str(ROOT)[2:].replace("\\", "/")
        r = subprocess.run(["wsl.exe", "-d", "Ubuntu-24.04", "-e", "sh", "-c",
                            f"cd '{wsl}' && python3 tools/crypto_check.py --target linux -n {opt.n}"])
        if r.returncode == 0:
            return 0
        print("crypto_check: WSL run failed; checking hashes only on Windows")
    seed = opt.seed if opt.seed is not None else random.randrange(1 << 30)
    rnd = random.Random(seed)
    cs = cases(opt.n, rnd)
    # open cases get the ciphertext (made by cryptography) instead of the plaintext; the answer
    # is the plaintext, kept as a third element
    fixed = []
    for c in cs:
        if c[0] == "aesgcmopen":
            from cryptography.hazmat.primitives.ciphers.aead import AESGCM
            k, iv, aad, pt = c[1]
            fixed.append(("aesgcmopen", [k, iv, aad, AESGCM(k).encrypt(iv, pt, aad)], pt))
        else:
            fixed.append(c)
    cs = fixed
    exe = ROOT / "build" / opt.target / ("coretest.exe" if opt.target == "win" else "coretest")
    with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as f:
        for c in cs:
            f.write(c[0] + " " + " ".join(x.hex() for x in c[1]) + "\n")
        path = f.name
    try:
        got = subprocess.run([str(exe), "--vectors", path], capture_output=True, text=True, check=True).stdout.split("\n")
    finally:
        os.unlink(path)
    bad = 0
    got += [""] * max(0, len(cs) - len(got))           # a missing answer is a difference
    for c, g in zip(cs, got):
        alg, a = c[0], c[1]
        if len(c) == 3:
            want = c[2] if isinstance(c[2], bytes) or c[2] is None else bytes([int(c[2])])
        else:
            want = expect(alg, a)
        w = "-" if want is None else want.hex()
        if g != w:
            bad += 1
            if bad <= 10:
                print(f"DIFF {alg} {[x.hex()[:32] for x in a]}\n  got  {g[:80]}\n  want {w[:80]}")
    print(f"crypto_check ({opt.target}, seed {seed}): {len(cs) - bad} of {len(cs)} cases agree")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
