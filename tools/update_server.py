"""update_server.py -- the update channels of `updatetest --test update / apply` (lib/update)

    python tools/update_server.py --dir DIR --exe PROGRAM --target windows-x86_64 [--app updatetest]

Starts an HTTP server on 127.0.0.1 (a free port) with signed channels made with
tools/update_manifest.py and a key generated for this run, then writes DIR/pubkey.txt (the
public key, hex) and DIR/port.txt (written last: the test waits for it).  The payloads are
PROGRAM (the test's own executable) with a marker appended ("#JFUPDTEST <version> <mode>#",
read back by updatetest --child), so a "new version" is a real program.

Channels (/<name>/update.json, update.json.sig and the files):
  good        2.0.0, kind exe                      bad         3.0.0, its start fails (mode crash)
  badsig      2.0.0 signed with another key        tampered    2.0.0 changed after signing
  badhash     2.0.0, the file differs in one byte  flaky       2.0.0, the first transfer of the
                                                               file stops half way (Range works)
  old         0.9.0                                otherapp    a manifest for another app
  expired     2.0.0, expired an hour ago           noplatform  2.0.0 for another system only
  deniedfile  2.0.0, the file on download.invalid  redirect    302 to good (manifest and sig)
  bundle      2.0.0, kind bundle (two files)       package     2.0.0, kind package (setup)
  big         2.0.0, 4 MiB sent slowly (cancel)
GET /stats returns the requests so far as JSON ([path, Range header or ""]).

The server ends when its standard input closes (the test's pipe) or after 10 minutes.
"""
import argparse
import hashlib
import json
import os
import socket
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import assetpack  # noqa: E402
import update_manifest as um  # noqa: E402

ROUTES = {}            # path -> bytes
REDIRECTS = {}         # path -> location
SPECIAL = {}           # path -> "flaky" | "slow"
REQUESTS = []          # (path, range)
LOCK = threading.Lock()


def sha(b):
    return hashlib.sha256(b).hexdigest()


def channel(name, app, version, files, seed, expires=0, tamper=False):
    """files = [(target, kind, url, data)]; a url without '/' is served under /<name>/"""
    entries = []
    for target, kind, url, data in files:
        entries.append((target, kind, url, len(data), sha(data)))
        if "/" not in url:
            ROUTES[f"/{name}/{url}"] = data
    man = um.manifest_bytes(app, version, entries, notes=f"Version {version}: Testkanal {name}.", expires=expires)
    sig = um.signature_text(seed, man)
    if tamper:
        man = man.replace(b"Testkanal", b"Testkan4l")
    ROUTES[f"/{name}/update.json"] = man
    ROUTES[f"/{name}/update.json.sig"] = sig


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def do_GET(self):
        path = self.path.split("?")[0]
        rng = self.headers.get("Range", "")
        with LOCK:
            REQUESTS.append([path, rng])
            first = sum(1 for p, _ in REQUESTS if p == path) == 1
        if path == "/stats":
            with LOCK:
                body = json.dumps(REQUESTS).encode()
            return self.send(200, body)
        if path in REDIRECTS:
            self.send_response(302)
            self.send_header("Location", REDIRECTS[path])
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        data = ROUTES.get(path)
        if data is None:
            return self.send(404, b"not found")
        start = 0
        if rng.startswith("bytes=") and rng.endswith("-"):
            try:
                start = int(rng[6:-1])
            except ValueError:
                start = -1
            if not 0 <= start < len(data):
                self.send_response(416)
                self.send_header("Content-Range", f"bytes */{len(data)}")
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
        kind = SPECIAL.get(path)
        self.send_response(206 if start else 200)
        if start:
            self.send_header("Content-Range", f"bytes {start}-{len(data) - 1}/{len(data)}")
        self.send_header("Content-Length", str(len(data) - start))
        self.send_header("Content-Type", "application/octet-stream")
        self.end_headers()
        body = data[start:]
        if kind == "flaky" and first:
            # half of the file, then the connection breaks
            self.wfile.write(body[:len(body) // 2])
            self.wfile.flush()
            self.connection.shutdown(socket.SHUT_RDWR)
            self.close_connection = True
            return
        if kind == "slow":
            for i in range(0, len(body), 16384):
                try:
                    self.wfile.write(body[i:i + 16384])
                    self.wfile.flush()
                except OSError:
                    return
                time.sleep(0.02)
            return
        self.wfile.write(body)

    def send(self, status, body):
        self.send_response(status)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", required=True)
    ap.add_argument("--exe", required=True)
    ap.add_argument("--target", required=True)
    ap.add_argument("--app", default="updatetest")
    a = ap.parse_args()
    out = Path(a.dir)
    out.mkdir(parents=True, exist_ok=True)
    exe = Path(a.exe).read_bytes()
    seed, other = os.urandom(32), os.urandom(32)
    t, app = a.target, a.app

    def program(version, mode="ok"):
        return exe + f"#JFUPDTEST {version} {mode}#".encode()

    srv = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    srv.daemon_threads = True
    port = srv.server_address[1]
    good = program("2.0.0")
    channel("good", app, "2.0.0", [(t, "exe", "app-2.0.0.bin", good)], seed)
    channel("bad", app, "3.0.0", [(t, "exe", "app-3.0.0.bin", program("3.0.0", "crash"))], seed)
    channel("badsig", app, "2.0.0", [(t, "exe", "app-2.0.0.bin", good)], other)
    channel("tampered", app, "2.0.0", [(t, "exe", "app-2.0.0.bin", good)], seed, tamper=True)
    channel("badhash", app, "2.0.0", [(t, "exe", "app-2.0.0.bin", good)], seed)
    wrong = bytearray(good)
    wrong[len(wrong) // 3] ^= 0x5A
    ROUTES["/badhash/app-2.0.0.bin"] = bytes(wrong)
    channel("flaky", app, "2.0.0", [(t, "exe", "app-2.0.0.bin", good)], seed)
    SPECIAL["/flaky/app-2.0.0.bin"] = "flaky"
    channel("old", app, "0.9.0", [(t, "exe", "app-0.9.0.bin", program("0.9.0"))], seed)
    channel("otherapp", "otherapp", "2.0.0", [(t, "exe", "app-2.0.0.bin", good)], seed)
    channel("expired", app, "2.0.0", [(t, "exe", "app-2.0.0.bin", good)], seed, expires=int(time.time()) - 3600)
    channel("noplatform", app, "2.0.0", [("plan9-mips", "exe", "app-2.0.0.bin", good)], seed)
    channel("deniedfile", app, "2.0.0", [(t, "exe", f"http://download.invalid:{port}/good/app-2.0.0.bin", good)], seed)
    REDIRECTS["/redirect/update.json"] = "/good/update.json"
    REDIRECTS["/redirect/update.json.sig"] = f"http://127.0.0.1:{port}/good/update.json.sig"
    bundle = assetpack.pack_files({"app.bin": b"Programm 2.0.0\n" * 100, "mods/extra.dat": b"Modul 2.0.0\n"})
    channel("bundle", app, "2.0.0", [(t, "bundle", "app-2.0.0.jfa", bundle)], seed)
    channel("package", app, "2.0.0", [(t, "package", "updatetest-2.0.0-setup.exe", b"MZ setup 2.0.0" * 1000)], seed)
    channel("big", app, "2.0.0", [(t, "exe", "big-2.0.0.bin", os.urandom(4 << 20))], seed)
    SPECIAL["/big/big-2.0.0.bin"] = "slow"
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    (out / "pubkey.txt").write_text(um.public_key(seed).hex() + "\n", encoding="ascii")
    tmp = out / "port.tmp"
    tmp.write_text(str(port), encoding="ascii")
    os.replace(tmp, out / "port.txt")

    def watch():
        try:
            while sys.stdin.buffer.read(4096):
                pass
        except (OSError, ValueError):
            pass
        os._exit(0)
    threading.Thread(target=watch, daemon=True).start()
    time.sleep(600)
    os._exit(0)


if __name__ == "__main__":
    main()
