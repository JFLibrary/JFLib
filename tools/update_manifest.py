"""update_manifest.py -- the signed update channel of an app (lib/update, docs/UPDATES.md)

    python tools/update_manifest.py keygen FILE          a new signing key in FILE; prints the
                                                         public key for update::Config
    python tools/update_manifest.py pubkey [--key FILE]  the public key of a signing key
    python tools/update_manifest.py sign --app ID --version V --out DIR
            --file TARGET=PATH[,kind=exe|bundle|package][,name=NAME] ...
            [--notes TEXT] [--expires-days N] [--base-url URL] [--key FILE]
                                                         copies the files into DIR and writes
                                                         DIR/update.json + update.json.sig
    python tools/update_manifest.py verify DIR --pub HEX check a channel folder

The signing key comes from --key FILE, else from the environment: jfgui_UPDATE_KEY_FILE (a
path) or jfgui_UPDATE_KEY (the key text).  A key file holds one line "JFUPDKEY1 <64 hex digits>"
(the Ed25519 seed).  Keep it out of the repository (*.updkey is ignored by git) -- whoever has
it can ship code to every installation.

TARGET is "windows-x86_64", "linux-x86_64", "linux-aarch64", ... (update::target()); kind
"exe" (default) replaces the program file, "bundle" is a JFA1 bundle of files relative to the
program's folder (tools/assetpack.py pack_files), "package" an installer the app hands to the
user.  With --base-url the manifest names absolute addresses, otherwise the file names
(relative to update.json).

Ed25519 (RFC 8032) is implemented here in plain Python: the standard library has none, and
the tool must run without extra packages.  It is slow (some milliseconds per signature) but
only signs one manifest; lib/crypto verifies.
"""
import argparse
import base64
import hashlib
import json
import os
import shutil
import sys
import time
from pathlib import Path

KEY_TAG = "JFUPDKEY1"

# ---------------------------------------------------------------- Ed25519 (RFC 8032)
# Written from the algorithm of RFC 8032 section 5.1 (key expansion 5.1.5, signing 5.1.6, the
# point encoding 5.1.2 and the decoding steps 5.1.3) and the twisted Edwards formulas of the
# Explicit-Formulas Database for a = -1 (add-2008-hwcd-3, dbl-2008-hwcd).  Points are
# extended coordinates (X, Y, Z, T): x = X / Z, y = Y / Z, x y = T / Z.
FIELD = 2 ** 255 - 19
ORDER = 2 ** 252 + 27742317777372353535851937790883648493    # of the base point
CURVE_D = -121665 * pow(121666, FIELD - 2, FIELD) % FIELD
CURVE_2D = 2 * CURVE_D % FIELD
ROOT_M1 = pow(2, (FIELD - 1) // 4, FIELD)                       # a square root of -1
NEUTRAL = (0, 1, 1, 0)


def _ext_add(p1, p2):
    """p1 + p2 (add-2008-hwcd-3)"""
    x1, y1, z1, t1 = p1
    x2, y2, z2, t2 = p2
    a = (y1 - x1) * (y2 - x2) % FIELD
    b = (y1 + x1) * (y2 + x2) % FIELD
    c = t1 * CURVE_2D % FIELD * t2 % FIELD
    d = 2 * z1 * z2 % FIELD
    e, f, g, h = b - a, d - c, d + c, b + a
    return (e * f % FIELD, g * h % FIELD, f * g % FIELD, e * h % FIELD)


def _ext_double(pt):
    """2 pt (dbl-2008-hwcd; T is not needed)"""
    x, y, z, _ = pt
    xx, yy = x * x % FIELD, y * y % FIELD
    zz2 = 2 * z * z % FIELD
    e = ((x + y) * (x + y) - xx - yy) % FIELD
    g = yy - xx
    f, h = g - zz2, -xx - yy
    return (e * f % FIELD, g * h % FIELD, f * g % FIELD, e * h % FIELD)


def _scalar_mul(k, pt):
    """k pt, double and add from the most significant bit"""
    out = NEUTRAL
    for bit in bin(k)[2:] if k > 0 else "":
        out = _ext_double(out)
        if bit == "1":
            out = _ext_add(out, pt)
    return out


def _point_bytes(pt):
    """5.1.2: y (little endian) with the low bit of x in the top bit"""
    x, y, z, _ = pt
    zinv = pow(z, FIELD - 2, FIELD)
    return (y * zinv % FIELD | (x * zinv % FIELD & 1) << 255).to_bytes(32, "little")


def _bytes_point(data):
    """5.1.3, strict: None for a non-canonical y, a y without a point, or x = 0 with the sign
    bit set.  x = u v^3 (u v^7)^((p - 5) / 8) with u = y^2 - 1, v = d y^2 + 1."""
    if len(data) != 32:
        return None
    n = int.from_bytes(data, "little")
    sign, y = n >> 255, n & ((1 << 255) - 1)
    if y >= FIELD:
        return None
    yy = y * y % FIELD
    u, v = (yy - 1) % FIELD, (CURVE_D * yy + 1) % FIELD
    v3 = v * v % FIELD * v % FIELD
    x = u * v3 % FIELD * pow(u * v3 % FIELD * v3 % FIELD * v % FIELD, (FIELD - 5) // 8, FIELD) % FIELD
    check = v * x % FIELD * x % FIELD
    if check == (FIELD - u) % FIELD and check != u:
        x = x * ROOT_M1 % FIELD
    elif check != u:
        return None
    if x == 0 and sign:
        return None
    if x & 1 != sign:
        x = FIELD - x
    return (x, y, 1, x * y % FIELD)


BASE = _bytes_point(bytes.fromhex("58" + "66" * 31))            # (x, 4/5), x even


def _secret_scalar(seed):
    """5.1.5: the pruned scalar of SHA-512(seed) and the second half of the hash"""
    digest = hashlib.sha512(seed).digest()
    s = int.from_bytes(digest[:32], "little") & ~7 & ((1 << 255) - 1) | 1 << 254
    return s, digest[32:]


def _hash_scalar(*parts):
    """SHA-512 of the parts in order, as a little-endian number mod ORDER"""
    return int.from_bytes(hashlib.sha512(b"".join(parts)).digest(), "little") % ORDER


def public_key(seed):
    """the 32-byte public key of a 32-byte seed"""
    return _point_bytes(_scalar_mul(_secret_scalar(seed)[0], BASE))


def sign(seed, msg):
    """the 64-byte Ed25519 signature of msg (5.1.6)"""
    s, prefix = _secret_scalar(seed)
    pub = _point_bytes(_scalar_mul(s, BASE))
    r = _hash_scalar(prefix, msg)
    r_bytes = _point_bytes(_scalar_mul(r, BASE))
    k = _hash_scalar(r_bytes, pub, msg)
    return r_bytes + ((r + k * s) % ORDER).to_bytes(32, "little")


def verify(pub, msg, sig):
    """whether sig is a valid signature of msg under pub (cofactorless, like lib/crypto)"""
    if len(sig) != 64:
        return False
    a_pt, r_pt = _bytes_point(pub), _bytes_point(sig[:32])
    s = int.from_bytes(sig[32:], "little")
    if a_pt is None or r_pt is None or s >= ORDER:
        return False
    k = _hash_scalar(sig[:32], pub, msg)
    return _point_bytes(_scalar_mul(s, BASE)) == _point_bytes(_ext_add(r_pt, _scalar_mul(k, a_pt)))


# ---------------------------------------------------------------- keys
def parse_key(text):
    """the seed of a key text ("JFUPDKEY1 <hex>" or the bare hex)"""
    t = text.strip()
    if t.startswith(KEY_TAG):
        t = t[len(KEY_TAG):].strip()
    try:
        seed = bytes.fromhex(t)
    except ValueError:
        seed = b""
    if len(seed) != 32:
        raise SystemExit("update_manifest: the signing key is not a JFUPDKEY1 key (64 hex digits)")
    return seed


def load_key(path=None):
    """the signing seed: path, else jfgui_UPDATE_KEY_FILE, else jfgui_UPDATE_KEY -> None when none
    is given"""
    path = path or os.environ.get("jfgui_UPDATE_KEY_FILE")
    if path:
        if not Path(path).is_file():
            raise SystemExit(f"update_manifest: no signing key at {path}")
        return parse_key(Path(path).read_text(encoding="utf-8"))
    text = os.environ.get("jfgui_UPDATE_KEY")
    return parse_key(text) if text else None


def keygen(path):
    seed = os.urandom(32)
    p = Path(path)
    if p.exists():
        raise SystemExit(f"update_manifest: {p} exists (a key is never overwritten)")
    fd = os.open(p, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8", newline="\n") as f:
        f.write(f"{KEY_TAG} {seed.hex()}\n")
    return public_key(seed)


# ---------------------------------------------------------------- manifests
def manifest_bytes(app, version, files, notes="", date=None, expires=0):
    """update.json: files = [(target, kind, url, size, sha256 hex)]"""
    m = {"format": 1, "app": app, "version": version, "date": int(time.time()) if date is None else date}
    if expires:
        m["expires"] = expires
    m["notes"] = notes
    m["files"] = [{"target": t, "kind": k, "url": u, "size": n, "sha256": h} for t, k, u, n, h in files]
    return (json.dumps(m, ensure_ascii=False, indent=1) + "\n").encode("utf-8")


def signature_text(seed, data):
    """update.json.sig: the Base64 signature and a line end"""
    return (base64.b64encode(sign(seed, data)).decode("ascii") + "\n").encode("ascii")


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def write_channel(out, app, version, files, seed, notes="", expires_days=0, base_url=""):
    """files = [(target, kind, path, name)]: each copied to out/name; out/update.json and
    out/update.json.sig written -> the manifest's path"""
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    entries = []
    for target, kind, path, name in files:
        if kind not in ("exe", "bundle", "package"):
            raise SystemExit(f"update_manifest: unknown kind {kind!r} (exe, bundle, package)")
        dest = out / name
        if Path(path).resolve() != dest.resolve():
            shutil.copyfile(path, dest)
        url = base_url.rstrip("/") + "/" + name if base_url else name
        entries.append((target, kind, url, dest.stat().st_size, sha256_file(dest)))
    expires = int(time.time()) + expires_days * 86400 if expires_days else 0
    data = manifest_bytes(app, version, entries, notes, expires=expires)
    (out / "update.json").write_bytes(data)
    (out / "update.json.sig").write_bytes(signature_text(seed, data))
    return out / "update.json"


def check_channel(folder, pub):
    """problems of a channel folder (empty: valid)"""
    folder = Path(folder)
    data = (folder / "update.json").read_bytes()
    try:
        sig = base64.b64decode((folder / "update.json.sig").read_text(encoding="ascii").strip(), validate=True)
    except ValueError:
        return ["update.json.sig is no Base64"]
    if not verify(pub, data, sig):
        return ["the signature does not match the key"]
    out = []
    m = json.loads(data)
    for f in m.get("files", []):
        name = f["url"].rsplit("/", 1)[-1]
        p = folder / name
        if not p.is_file():
            out.append(f"{name}: missing")
        elif p.stat().st_size != f["size"] or sha256_file(p) != f["sha256"]:
            out.append(f"{name}: size or SHA-256 differ from the manifest")
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    k = sub.add_parser("keygen")
    k.add_argument("file")
    pk = sub.add_parser("pubkey")
    pk.add_argument("--key")
    s = sub.add_parser("sign")
    s.add_argument("--app", required=True)
    s.add_argument("--version", required=True)
    s.add_argument("--out", required=True)
    s.add_argument("--file", action="append", required=True, help="TARGET=PATH[,kind=K][,name=N]")
    s.add_argument("--notes", default="")
    s.add_argument("--expires-days", type=int, default=0)
    s.add_argument("--base-url", default="")
    s.add_argument("--key")
    v = sub.add_parser("verify")
    v.add_argument("dir")
    v.add_argument("--pub", required=True)
    a = ap.parse_args(argv)
    if a.cmd == "keygen":
        print(keygen(a.file).hex())
    elif a.cmd == "pubkey":
        seed = load_key(a.key)
        if seed is None:
            raise SystemExit("update_manifest: no signing key (--key FILE, jfgui_UPDATE_KEY_FILE or jfgui_UPDATE_KEY)")
        print(public_key(seed).hex())
    elif a.cmd == "sign":
        seed = load_key(a.key)
        if seed is None:
            raise SystemExit("update_manifest: no signing key (--key FILE, jfgui_UPDATE_KEY_FILE or jfgui_UPDATE_KEY)")
        files = []
        for spec in a.file:
            target, _, rest = spec.partition("=")
            parts = rest.split(",")
            opts = dict(p.split("=", 1) for p in parts[1:] if "=" in p)
            path = parts[0]
            if not target or not Path(path).is_file():
                raise SystemExit(f"update_manifest: bad --file {spec!r} (TARGET=PATH, the file must exist)")
            files.append((target, opts.get("kind", "exe"), path, opts.get("name", Path(path).name)))
        m = write_channel(a.out, a.app, a.version, files, seed, a.notes, a.expires_days, a.base_url)
        print(f"{m} and {m}.sig written (key {public_key(seed).hex()})")
    elif a.cmd == "verify":
        problems = check_channel(a.dir, bytes.fromhex(a.pub))
        for p in problems:
            print("FAIL", p)
        if problems:
            return 1
        print("ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
