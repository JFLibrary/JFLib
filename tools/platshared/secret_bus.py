#!/usr/bin/env python3
"""secret_bus.py -- plat::secret on Linux (lib/plat/linux_secret.rs, linux_dbus.rs) against a
fake Secret Service.  Run inside WSL / Linux from the repository root:

  python3 tools/platshared/secret_bus.py [--rs build] [--only dh,plain,locked,...]

The fake session bus (SASL EXTERNAL, little-endian messages; the marshalling of portal_bus.py)
plays the bus daemon and org.freedesktop.secrets: OpenSession (dh-ietf1024-sha256-aes128-cbc-pkcs7
with Python's pow, HKDF from hmac and AES from the `cryptography` package; plain), ReadAlias,
CreateCollection, SearchItems, Unlock, Collection.CreateItem, Item.GetSecret / Delete,
Session.Close and prompts (Prompt / Dismiss -> Completed).  plat opens a connection per call;
the fake accepts any number of them.  For every mode it starts plattest --test secret-bus
--mode M (which checks its side) and then checks what arrived:

  dh        the round trip of plattest --test secret: every value on the wire AES-CBC (16-byte
            IV, never the plain bytes), attributes xdg:schema / app / service / account, the
            label, the content types, another app's items apart, every session closed, no item
            left
  plain     OpenSession with DH answered NotSupported: plain sessions, the same round trip
  locked    a locked collection with one item that locks itself again whenever a client
            connects: get, set and delete go through Unlock and a completed prompt
  dismiss   the same with every prompt dismissed: nothing changes
  nodefault no default alias: CreateCollection("Default keyring", "default") and its prompt,
            then CreateItem and Delete answer with prompts
  badkey    OpenSession answers the public key 1
  ksecretd  the numbers as KDE's ksecretd (KWallet 6.24, QCA) writes them: its public value with
            a sign byte (129 bytes, the top bit is always set here), client values shorter than
            128 bytes refused, the shared value hashed without leading zeros -- the first
            session's shared value starts with a zero byte, so plat must give it up (and every
            other such one) and open another; then the round trip of dh
  none      the bus has no org.freedesktop.secrets: ServiceUnknown
Exit code 0 = passed.
"""
import argparse, hashlib, hmac, os, socket, subprocess, sys, tempfile, threading
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from portal_bus import R, W, listen_unix, message, parse  # noqa: E402

try:
    from cryptography.hazmat.primitives.ciphers import Cipher, algorithms, modes
except ImportError:
    Cipher = None

MODES = ["dh", "plain", "locked", "dismiss", "nodefault", "badkey", "ksecretd", "none"]
P = int("FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A0879"
        "8E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B"
        "0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE65381FFFFFFFFFFFFFFFF", 16)
DEST = "org.freedesktop.secrets"
ROOT = "/org/freedesktop/secrets"
LOGIN = ROOT + "/collection/login"
NEWCOL = ROOT + "/collection/Default_keyring"
ALGO = "dh-ietf1024-sha256-aes128-cbc-pkcs7"
SCHEMA = "org.freedesktop.Secret.Generic"
fails = 0


def check(name, ok, info=""):
    global fails
    print(("PASS " if ok else "FAIL ") + name + ("" if ok else f": {info}"))
    fails += 0 if ok else 1
    return ok


def aes(key, iv, data, encrypt):
    c = Cipher(algorithms.AES(key), modes.CBC(iv))
    x = c.encryptor() if encrypt else c.decryptor()
    return x.update(data) + x.finalize()


def dh_key(shared, n=128):
    """HKDF-SHA256 without salt and info of the shared value written in n bytes (None: without
    leading zeros, as ksecretd) -> 16 bytes"""
    raw = shared.to_bytes(n or (shared.bit_length() + 7) // 8, "big")
    prk = hmac.new(b"\0" * 32, raw, hashlib.sha256).digest()
    return hmac.new(prk, b"\x01", hashlib.sha256).digest()[:16]


class Fail(Exception):
    def __init__(self, name, text=""):
        super().__init__(name)
        self.name, self.text = name, text


class Store:
    """the fake service's state; one per mode"""

    def __init__(self, mode):
        self.mode = mode
        self.lock = threading.RLock()
        self.items = {}         # path -> {attrs, label, value, ctype, col}
        self.cols = {}          # path -> {label, locked}
        self.alias = {}
        self.sessions = {}      # path -> AES key or None (plain)
        self.closed = set()
        self.prompts = {}       # path -> action() -> result (sig, value)
        self.n = 0
        self.conns = 0
        self.stop = False
        self.calls = []         # (member, facts)
        self.wire = []          # (plain value, iv, value on the wire, content type) of CreateItem
        self.served = []        # (plain, iv, wire) of GetSecret
        self.completed = []     # dismissed flags of the Completed signals
        self.matches = []
        self.threads = []
        self.used = set()       # sessions a value travelled through (CreateItem, GetSecret)
        self.zero_done = False  # ksecretd: the session with a leading zero was made
        self.zero_sessions = []
        if mode != "nodefault":
            self.cols[LOGIN] = {"label": "Login", "locked": mode in ("locked", "dismiss")}
            self.alias["default"] = LOGIN
        if mode in ("locked", "dismiss"):
            self.add(LOGIN, {"xdg:schema": SCHEMA, "app": "plattest", "service": "secret-bus", "account": "locked"},
                     "plattest: secret-bus (locked)", b"from the keyring", "text/plain")

    def path(self, kind):
        self.n += 1
        return f"{ROOT}/{kind}/{self.n}"

    def add(self, col, attrs, label, value, ctype):
        self.n += 1
        p = f"{col}/{self.n}"
        self.items[p] = {"attrs": attrs, "label": label, "value": value, "ctype": ctype, "col": col}
        return p

    def locked(self, item):
        return self.cols[self.items[item]["col"]]["locked"]

    def prompt(self, action):
        p = self.path("prompt")
        self.prompts[p] = action
        return p

    def connected(self):
        with self.lock:
            self.conns += 1
            if self.mode == "locked":
                for c in self.cols.values():
                    c["locked"] = True
            return f":1.{self.conns}"

    # ------------------------------------------------------------ the service's methods
    def secret_in(self, sec):
        session, iv, value, ctype = sec
        iv, value = bytes(iv), bytes(value)
        if session not in self.sessions:
            raise Fail("org.freedesktop.Secret.Error.NoSession", session)
        key = self.sessions[session]
        if key is None:
            return value, iv, value, ctype
        if len(iv) != 16 or len(value) % 16 or not value:
            raise Fail("org.freedesktop.DBus.Error.InvalidArgs", "bad IV or length")
        plain = aes(key, iv, value, False)
        n = plain[-1]
        if not 1 <= n <= 16 or plain[-n:] != bytes([n]) * n:
            raise Fail("org.freedesktop.DBus.Error.InvalidArgs", "bad padding")
        return plain[:-n], iv, value, ctype

    def secret_out(self, session, item):
        it = self.items[item]
        key = self.sessions[session]
        if key is None:
            self.served.append((it["value"], b"", it["value"]))
            return (session, b"", it["value"], it["ctype"])
        iv = os.urandom(16)
        n = 16 - len(it["value"]) % 16
        wire = aes(key, iv, it["value"] + bytes([n]) * n, True)
        self.served.append((it["value"], iv, wire))
        return (session, iv, wire, it["ctype"])

    def call(self, path, iface, member, a):
        """-> (signature, values); raises Fail"""
        m = self.mode
        self.calls.append((member, a))
        if (iface, member) == ("org.freedesktop.Secret.Service", "OpenSession"):
            algo, (vsig, vval) = a
            if algo == ALGO and m == "plain" or algo not in (ALGO, "plain"):
                raise Fail("org.freedesktop.DBus.Error.NotSupported", algo)
            sp = self.path("session")
            if algo == "plain":
                self.sessions[sp] = None
                return "vo", [("s", ""), sp]
            if vsig != "ay":
                raise Fail("org.freedesktop.DBus.Error.InvalidArgs", vsig)
            client = int.from_bytes(bytes(vval), "big")
            if not 1 < client < P - 1 or m == "ksecretd" and len(vval) < 128:
                raise Fail("org.freedesktop.DBus.Error.InvalidArgs", "public key")
            if m == "badkey":
                self.sessions[sp] = b"\0" * 16
                return "vo", [("ay", b"\x01"), sp]
            if m == "ksecretd":
                # QCA's BigInteger::toArray: a sign byte when the top bit is set (always here);
                # the first session's shared value starts with a zero byte
                while True:
                    priv = int.from_bytes(os.urandom(128), "big")
                    mine, shared = pow(2, priv, P), pow(client, priv, P)
                    if mine.bit_length() == 1024 and (self.zero_done or shared.bit_length() <= 1016):
                        break
                self.zero_done = True
                if shared.bit_length() <= 1016:
                    self.zero_sessions.append(sp)
                self.sessions[sp] = dh_key(shared, None)
                self.calls[-1] = (member, [algo, len(bytes(vval))])
                return "vo", [("ay", b"\0" + mine.to_bytes(128, "big")), sp]
            priv = int.from_bytes(os.urandom(128), "big")
            mine = pow(2, priv, P)
            self.sessions[sp] = dh_key(pow(client, priv, P))
            self.calls[-1] = (member, [algo, len(bytes(vval))])
            return "vo", [("ay", mine.to_bytes((mine.bit_length() + 7) // 8, "big")), sp]
        if iface == "org.freedesktop.Secret.Session" and member == "Close":
            self.closed.add(path)
            return "", []
        if (iface, member) == ("org.freedesktop.Secret.Service", "ReadAlias"):
            return "o", [self.alias.get(a[0], "/")]
        if (iface, member) == ("org.freedesktop.Secret.Service", "CreateCollection"):
            props, alias = a
            label = props.get("org.freedesktop.Secret.Collection.Label", ("s", ""))[1]

            def make():
                self.cols[NEWCOL] = {"label": label, "locked": False}
                if alias:
                    self.alias[alias] = NEWCOL
                return ("o", NEWCOL)
            return "oo", ["/", self.prompt(make)]
        if (iface, member) == ("org.freedesktop.Secret.Service", "SearchItems"):
            want = a[0]
            hits = [p for p, it in self.items.items() if all(it["attrs"].get(k) == v for k, v in want.items())]
            return "aoao", [[p for p in hits if not self.locked(p)], [p for p in hits if self.locked(p)]]
        if (iface, member) == ("org.freedesktop.Secret.Service", "Unlock"):
            objs = a[0]
            cols = {o if o in self.cols else self.items[o]["col"] for o in objs if o in self.cols or o in self.items}
            if not any(self.cols[c]["locked"] for c in cols):
                return "aoo", [objs, "/"]

            def unlock():
                for c in cols:
                    self.cols[c]["locked"] = False
                return ("ao", objs)
            return "aoo", [[], self.prompt(unlock)]
        if (iface, member) == ("org.freedesktop.Secret.Collection", "CreateItem"):
            props, sec, replace = a
            if path not in self.cols:
                raise Fail("org.freedesktop.Secret.Error.NoSuchObject", path)
            if self.cols[path]["locked"]:
                raise Fail("org.freedesktop.Secret.Error.IsLocked", path)
            label = props.get("org.freedesktop.Secret.Item.Label", ("s", ""))[1]
            attrs = dict(props.get("org.freedesktop.Secret.Item.Attributes", ("a{ss}", {}))[1])
            plain, iv, wire, ctype = self.secret_in(sec)
            self.used.add(sec[0])
            self.wire.append((plain, iv, wire, ctype, attrs, label, replace))

            def create():
                old = [p for p, it in self.items.items() if it["col"] == path and it["attrs"] == attrs]
                if old and replace:
                    self.items[old[0]].update(value=plain, label=label, ctype=ctype)
                    return ("o", old[0])
                return ("o", self.add(path, attrs, label, plain, ctype))
            if m == "nodefault":
                return "oo", ["/", self.prompt(create)]
            return "oo", [create()[1], "/"]
        if path in self.items and iface == "org.freedesktop.Secret.Item":
            if self.locked(path):
                raise Fail("org.freedesktop.Secret.Error.IsLocked", path)
            if member == "GetSecret":
                self.used.add(a[0])
                return "(oayays)", [self.secret_out(a[0], path)]
            if member == "Delete":
                def remove():
                    self.items.pop(path, None)
                    return ("s", "")
                if m == "nodefault":
                    return "o", [self.prompt(remove)]
                remove()
                return "o", ["/"]
        if path in self.prompts and iface == "org.freedesktop.Secret.Prompt" and member in ("Prompt", "Dismiss"):
            return "", []
        raise Fail("org.freedesktop.Secret.Error.NoSuchObject", f"{path} {iface}.{member}")


def serve(sock, st):
    srv = listen_unix(sock, 8)
    srv.settimeout(0.3)
    while not st.stop:
        try:
            c, _ = srv.accept()
        except socket.timeout:
            continue
        t = threading.Thread(target=connection, args=(c, st), daemon=True)
        st.threads.append(t)
        t.start()
    srv.close()


def connection(c, st):
    uniq = st.connected()
    c.settimeout(60)
    buf = b""
    try:
        authed = False
        while b"BEGIN\r\n" not in buf:
            d = c.recv(4096)
            if not d:
                return
            buf += d
            if b"AUTH EXTERNAL" in buf and b"\r\n" in buf.split(b"AUTH", 1)[1] and not authed:
                authed = True
                c.sendall(b"OK 0123456789abcdef0123456789abcdef\r\n")
        buf = buf.split(b"BEGIN\r\n", 1)[1]
        ser = [100]

        def send(mtype, fields, sig="", vals=()):
            ser[0] += 1
            f = fields + ([(8, "g", sig)] if sig else [])
            c.sendall(message(mtype, ser[0], f, W().all(sig, list(vals)) if sig else b""))

        def reply(m, sig, vals):
            send(2, [(5, "u", m["serial"]), (6, "s", uniq), (7, "s", ":1.0")], sig, vals)

        def error(m, name, text):
            send(3, [(4, "s", name), (5, "u", m["serial"]), (6, "s", uniq), (7, "s", ":1.0")], "s", [text])

        while True:
            m, buf = parse(buf)
            if m is None:
                d = c.recv(65536)
                if not d:
                    return
                buf += d
                continue
            if m["type"] != 1:
                continue
            fl = m["fields"]
            path, iface, member, dest, sig = fl.get(1, ""), fl.get(2, ""), fl.get(3, ""), fl.get(6, ""), fl.get(8, "")
            args = R(m["body"]).all(sig) if sig else []
            no_reply = m["raw"][2] & 1
            if dest == "org.freedesktop.DBus":
                with st.lock:
                    st.calls.append(("bus." + member, args))
                    if member == "Hello":
                        reply(m, "s", [uniq])
                    elif member == "AddMatch":
                        st.matches.append(args[0])
                        reply(m, "", [])
                    elif member == "NameHasOwner":
                        reply(m, "b", [args[0] == DEST and st.mode != "none"])
                    elif member == "ListActivatableNames":
                        reply(m, "as", [["org.freedesktop.DBus"]])
                    else:
                        error(m, "org.freedesktop.DBus.Error.UnknownMethod", member)
                continue
            if dest != DEST or st.mode == "none":
                with st.lock:
                    st.calls.append(("ServiceUnknown", [dest, member]))
                if not no_reply:
                    error(m, "org.freedesktop.DBus.Error.ServiceUnknown", f"The name {dest} was not provided")
                continue
            with st.lock:
                try:
                    rsig, vals = st.call(path, iface, member, args)
                except Fail as e:
                    if not no_reply:
                        error(m, e.name, e.text)
                    continue
                if not no_reply:
                    reply(m, rsig, vals)
                if iface == "org.freedesktop.Secret.Prompt":
                    action = st.prompts.pop(path)
                    dismissed = st.mode == "dismiss" or member == "Dismiss"
                    result = ("s", "") if dismissed else action()
                    st.completed.append(dismissed)
                    send(4, [(1, "o", path), (2, "s", "org.freedesktop.Secret.Prompt"), (3, "s", "Completed"), (7, "s", ":1.0")],
                         "bv", [dismissed, result])
    except (socket.timeout, OSError):
        pass
    finally:
        c.close()


def run(exe, mode):
    st = Store(mode)
    with tempfile.TemporaryDirectory() as d:
        sock = os.path.join(d, "bus")
        t = threading.Thread(target=serve, args=(listen_unix(sock, 8), st), daemon=True)
        t.start()
        env = dict(os.environ, DBUS_SESSION_BUS_ADDRESS=f"unix:path={sock}")
        try:
            p = subprocess.run([str(exe), "--test", "secret-bus", "--mode", mode], env=env, capture_output=True, text=True, timeout=120)
            out, code = p.stdout + p.stderr, p.returncode
        except subprocess.TimeoutExpired as e:
            out, code = (e.stdout or b"").decode() if isinstance(e.stdout, bytes) else (e.stdout or ""), "timeout"
        st.stop = True
        t.join(2)
        for c in st.threads:
            c.join(5)
    print(f"--- mode {mode}")
    for ln in out.splitlines():
        if ln.startswith(("FAIL", "plattest:", "panic", "error")):
            print("   " + ln)
    check(f"{mode}: plattest --test secret-bus --mode {mode} passed", code == 0 and "secret-bus: done" in out, f"exit {code}")
    return st


def verify(mode, st):
    members = [c[0] for c in st.calls]
    opened = [c for c in st.calls if c[0] == "OpenSession"]
    if mode in ("dh", "plain", "locked", "dismiss", "nodefault", "badkey", "ksecretd"):
        check(f"{mode}: every session closed again", set(st.sessions) <= st.closed, (len(st.sessions), len(st.closed)))
    if mode == "dh":
        check("dh: every session used dh-ietf1024 with a public key of at most 128 bytes",
              opened and all(c[1][0] == ALGO and 0 < c[1][1] <= 128 for c in opened), opened[:3])
        check("dh: no plain session", all(k is not None for k in st.sessions.values()))
        enc = [w for w in st.wire if w[1]]
        check("dh: every value AES-CBC with a 16-byte IV", st.wire and len(enc) == len(st.wire) and all(len(w[1]) == 16 and len(w[2]) % 16 == 0 for w in st.wire))
        check("dh: no value travelled in plain", all(w[0] not in w[2] or len(w[0]) < 4 for w in st.wire)
              and all(s[0] not in s[2] or len(s[0]) < 4 for s in st.served))
        check("dh: the 64 KiB value arrived whole", any(len(w[0]) == 65536 for w in st.wire), sorted({len(w[0]) for w in st.wire})[-3:])
        alice = [w for w in st.wire if w[4].get("account") == "alice" and w[4].get("app") == "plattest"]
        check("dh: attributes xdg:schema, app, service, account",
              alice and alice[0][4] == {"xdg:schema": SCHEMA, "app": "plattest", "service": "secret-bus", "account": "alice"}, alice[:1])
        check("dh: the label names app, service and account", alice and alice[0][5] == "plattest: secret-bus (alice)", alice[:1])
        check("dh: replace is set", all(w[6] for w in st.wire))
        types = {bytes(w[0][:1]) == b"\xff": w[3] for w in st.wire if w[4].get("account") in ("alice", "bob")}
        check("dh: content types text/plain (UTF-8) and application/octet-stream (binary)",
              types.get(False) == "text/plain" and types.get(True) == "application/octet-stream", types)
        check("dh: the other app's entry carries app=plattest-other",
              any(w[4].get("app") == "plattest-other" for w in st.wire))
        noacc = [w for w in st.wire if w[4].get("account") == ""]
        check("dh: an empty account is an empty attribute, the label without it",
              noacc and noacc[0][5] == "plattest: secret-bus", noacc[:1])
        check("dh: no item left", not st.items, [i["attrs"] for i in st.items.values()][:4])
    if mode == "plain":
        refused = [c for c in st.calls if c[0] == "OpenSession" and c[1][0] == ALGO]
        check("plain: DH tried first, then plain", refused and all(k is None for k in st.sessions.values()) and st.sessions)
        check("plain: values stored as sent", st.wire and all(w[0] == w[2] for w in st.wire))
        check("plain: no item left", not st.items)
    if mode == "locked":
        check("locked: get, set and delete each went through a prompt", len(st.completed) >= 4 and not any(st.completed), st.completed)
        check("locked: Unlock was asked", "Unlock" in members)
        check("locked: matches for the prompts' Completed",
              st.matches and all("org.freedesktop.Secret.Prompt" in r and "Completed" in r and "path=" in r for r in st.matches), st.matches[:2])
        check("locked: no item left", not st.items)
    if mode == "dismiss":
        check("dismiss: every prompt dismissed", len(st.completed) >= 3 and all(st.completed), st.completed)
        left = list(st.items.values())
        check("dismiss: the keyring is unchanged", len(left) == 1 and left[0]["value"] == b"from the keyring", left)
        check("dismiss: nothing was created", not st.wire)
    if mode == "nodefault":
        check("nodefault: CreateCollection with alias default", "CreateCollection" in members and st.alias.get("default") == NEWCOL)
        check("nodefault: its label", st.cols.get(NEWCOL, {}).get("label") == "Default keyring", st.cols)
        check("nodefault: prompts for the collection, items and Delete completed", len(st.completed) >= 4 and not any(st.completed), st.completed)
        check("nodefault: no item left", not st.items)
    if mode == "ksecretd":
        check("ksecretd: every public value of plat has 128 bytes", opened and all(c[1][1] == 128 for c in opened), opened[:3])
        zs = st.zero_sessions
        # (the first one is made so; a later one by chance, 1 in 256)
        check("ksecretd: every session whose shared value starts with a zero byte was closed unused",
              zs and all(z in st.closed and z not in st.used for z in zs), zs)
        check("ksecretd: values went through the other sessions", st.wire and len(st.sessions) > 1)
        check("ksecretd: no item left", not st.items)
    if mode == "badkey":
        check("badkey: OpenSession only, nothing stored", "OpenSession" in members and not st.wire and "CreateItem" not in members, members)
    if mode == "none":
        check("none: plat asked the bus whether the name has an owner", "bus.NameHasOwner" in members, members)
        check("none: calls to the missing service got ServiceUnknown", "ServiceUnknown" in members, members)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rs", default="build")
    ap.add_argument("--only", default=",".join(MODES))
    a = ap.parse_args()
    exe = Path(a.rs) / "linux" / "plattest"
    if Cipher is None:
        print("FAIL secret_bus needs the cryptography package (python3-cryptography)")
        return 1
    for mode in a.only.split(","):
        if mode not in MODES:
            check(f"known mode {mode}", False)
            continue
        verify(mode, run(exe, mode))
    print("secret_bus:", "all passed" if fails == 0 else f"{fails} FAILED")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
