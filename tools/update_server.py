"""update_server.py -- the update channels of `updatetest --test update / apply` (lib/update)

    python tools/update_server.py --dir DIR --exe PROGRAM --target windows-x86_64 [--app updatetest]
                                  [--runtime APPIMAGERT]

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
On Linux with gpg (a throwaway GNUPGHOME per key in DIR, its agent ended before serving) also
the AppImage channels: fake AppImages (a small ELF made a type-2 runtime by
pkg_linux.runtime_image, then the program as the "image"), signed as appimagetool --sign does
(pkg_linux.sign_appimage) with key a (Ed25519), b (RSA) or c (ECDSA P-256):
  appimage            2.0.0: a .deb (kind package) and the AppImage signed with a
  appimage-bad        3.0.0 signed with a, its start fails      appimage-otherkey  signed with b
  appimage-unsigned   2.0.0 unsigned                            appimage-notai     no AppImage
  appimage-tampered   signed with a, a byte changed after signing (the manifest has its hash)
  appimage-rsa        2.0.0 signed with b                       appimage-ecdsa     signed with c
and DIR/appimage/: running-{a,b,c,unsigned}.AppImage (1.0.0, the "installed" files),
update-a-2.0.0.AppImage, tampered.AppImage and fpr-{a,b,c}.txt (the fingerprints).  With
--runtime (apps/appimagert) and mksquashfs also real AppImages of PROGRAM (an AppDir with
AppRun -> the program, SquashFS zstd behind the runtime, signed with a): DIR/appimage/
real-1.0.0.AppImage and the channels aiapply-good (2.0.0) and aiapply-bad (3.0.0, its start
fails) for updatetest --test apply.
GET /stats returns the requests so far as JSON ([path, Range header or ""]).

The server ends when its standard input closes (the test's pipe) or after 10 minutes.
"""
import argparse
import hashlib
import tempfile
import json
import os
import shutil
import socket
import struct
import subprocess
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


def tiny_elf():
    """a small ELF64 file (a .text and the section names) to make a fake AppImage runtime of"""
    names = b"\0.text\0.shstrtab\0"
    text = b"\xc3" * 16
    shoff = 64 + len(text) + len(names)
    shoff += -shoff % 8
    head = bytearray(64)
    head[:16] = b"\x7fELF\x02\x01\x01" + b"\0" * 9
    struct.pack_into("<HHIQQQIHHHHHH", head, 16, 2, 62, 1, 0, 0, shoff, 0, 64, 56, 0, 64, 3, 2)
    body = bytes(head) + text + names
    body += b"\0" * (shoff - len(body))
    sh = bytes(64)
    sh += struct.pack("<IIQQQQIIQQ", 1, 1, 6, 0, 64, len(text), 0, 0, 16, 0)
    sh += struct.pack("<IIQQQQIIQQ", 7, 3, 0, 0, 64 + len(text), len(names), 0, 0, 1, 0)
    return body + sh


class Gpg:
    """a throwaway GnuPG home with one key (algo: ed25519, rsa2048, nistp256)"""

    def __init__(self, home, name, algo):
        self.home = Path(home)
        self.home.mkdir(mode=0o700, parents=True)
        self.base = ["gpg", "--homedir", str(self.home), "--batch", "--yes", "--quiet"]
        subprocess.run(self.base + ["--passphrase", "", "--quick-gen-key", f"Updatetest {name} <{name}@localhost>", algo, "sign", "never"],
                       check=True, capture_output=True)
        out = subprocess.run(self.base + ["--list-keys", "--with-colons"], check=True, capture_output=True, text=True).stdout
        self.fpr = next(ln.split(":")[9] for ln in out.splitlines() if ln.startswith("fpr:"))

    def sign(self, data):
        f = self.home / "digest"
        f.write_bytes(data)
        sig = subprocess.run(self.base + ["--local-user", self.fpr, "--armor", "--detach-sign", "--output", "-", str(f)],
                             check=True, capture_output=True).stdout.decode("ascii")
        key = subprocess.run(self.base + ["--armor", "--export", self.fpr], check=True, capture_output=True).stdout.decode("ascii")
        return sig, key

    def close(self):
        subprocess.run(["gpgconf", "--homedir", str(self.home), "--kill", "gpg-agent"], capture_output=True)


def real_appimage(rt, program, version, gpg, mode="ok"):
    """an AppImage of the test program: AppRun -> the program (marked), SquashFS behind rt, signed"""
    import pkg_linux
    with tempfile.TemporaryDirectory(prefix="updatetest-ai-") as tmp:
        appdir = Path(tmp) / "AppDir"
        appdir.mkdir()
        (appdir / "updatetest").write_bytes(program(version, mode))
        (appdir / "updatetest").chmod(0o755)
        os.symlink("updatetest", appdir / "AppRun")
        img = Path(tmp) / "image.squashfs"
        subprocess.run(["mksquashfs", str(appdir), str(img), "-all-root", "-noappend", "-comp", "zstd", "-quiet", "-no-progress"],
                       check=True, capture_output=True)
        f = Path(tmp) / "out.AppImage"
        f.write_bytes(rt + img.read_bytes())
        pkg_linux.sign_appimage(f, gpg.sign)
        return f.read_bytes()


def appimage_channels(out, t, app, seed, program, runtime=None):
    """the AppImage channels (Linux with gpg; see the module text); gpg or mksquashfs failing
    twice: DIR/appimage/error.txt says why (the test reports it), the other channels stay"""
    if not sys.platform.startswith("linux") or not shutil.which("gpg"):
        return
    why = ""
    for _ in range(2):
        try:
            return make_appimage_channels(out, t, app, seed, program, runtime)
        except (subprocess.CalledProcessError, OSError) as e:
            why = f"{e} {(getattr(e, 'stderr', b'') or b'').decode('utf-8', 'replace').strip()[-400:]}"
            for g in [out / "appimage", *out.glob("gnupg-*")]:
                shutil.rmtree(g, ignore_errors=True)
    (out / "appimage").mkdir(parents=True, exist_ok=True)
    (out / "appimage" / "error.txt").write_text(why + "\n", encoding="utf-8")


def make_appimage_channels(out, t, app, seed, program, runtime):
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import pkg_linux
    d = out / "appimage"
    d.mkdir(parents=True, exist_ok=True)
    rt = pkg_linux.runtime_image(tiny_elf())
    keys = {}
    try:
        for k, algo in (("a", "ed25519"), ("b", "rsa2048"), ("c", "nistp256")):
            keys[k] = Gpg(out / f"gnupg-{k}", k, algo)
            (d / f"fpr-{k}.txt").write_text(keys[k].fpr + "\n", encoding="ascii")

        def appimage(version, key=None, mode="ok"):
            f = d / "tmp.AppImage"
            f.write_bytes(rt + program(version, mode))
            if key:
                pkg_linux.sign_appimage(f, keys[key].sign)
            return f.read_bytes()
        for k in ("a", "b", "c"):
            (d / f"running-{k}.AppImage").write_bytes(appimage("1.0.0", k))
        (d / "running-unsigned.AppImage").write_bytes(appimage("1.0.0"))
        good = appimage("2.0.0", "a")
        (d / "update-a-2.0.0.AppImage").write_bytes(good)
        deb = b"!<arch>\ndebian-binary   " + b"0" * 2000
        channel("appimage", app, "2.0.0", [(t, "package", "updatetest_2.0.0_amd64.deb", deb),
                                           (t, "appimage", "updatetest-2.0.0-x86_64.AppImage", good)], seed)
        channel("appimage-bad", app, "3.0.0", [(t, "appimage", "updatetest-3.0.0-x86_64.AppImage", appimage("3.0.0", "a", "crash"))], seed)
        channel("appimage-otherkey", app, "2.0.0", [(t, "appimage", "updatetest-2.0.0-x86_64.AppImage", appimage("2.0.0", "b"))], seed)
        channel("appimage-unsigned", app, "2.0.0", [(t, "appimage", "updatetest-2.0.0-x86_64.AppImage", appimage("2.0.0"))], seed)
        tampered = bytearray(good)
        tampered[len(rt) + 100] ^= 0x01
        (d / "tampered.AppImage").write_bytes(bytes(tampered))
        channel("appimage-tampered", app, "2.0.0", [(t, "appimage", "updatetest-2.0.0-x86_64.AppImage", bytes(tampered))], seed)
        channel("appimage-notai", app, "2.0.0", [(t, "appimage", "updatetest-2.0.0-x86_64.AppImage", program("2.0.0"))], seed)
        channel("appimage-rsa", app, "2.0.0", [(t, "appimage", "updatetest-2.0.0-x86_64.AppImage", appimage("2.0.0", "b"))], seed)
        channel("appimage-ecdsa", app, "2.0.0", [(t, "appimage", "updatetest-2.0.0-x86_64.AppImage", appimage("2.0.0", "c"))], seed)
        (d / "tmp.AppImage").unlink()
        if runtime and Path(runtime).is_file() and shutil.which("mksquashfs"):
            real = pkg_linux.runtime_image(Path(runtime).read_bytes())
            (d / "real-1.0.0.AppImage").write_bytes(real_appimage(real, program, "1.0.0", keys["a"]))
            good = real_appimage(real, program, "2.0.0", keys["a"])
            (d / "real-2.0.0.AppImage").write_bytes(good)
            channel("aiapply-good", app, "2.0.0", [(t, "appimage", "updatetest-2.0.0-x86_64.AppImage", good)], seed)
            channel("aiapply-bad", app, "3.0.0", [(t, "appimage", "updatetest-3.0.0-x86_64.AppImage",
                                                   real_appimage(real, program, "3.0.0", keys["a"], "crash"))], seed)
    finally:
        for g in keys.values():
            g.close()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", required=True)
    ap.add_argument("--exe", required=True)
    ap.add_argument("--target", required=True)
    ap.add_argument("--app", default="updatetest")
    ap.add_argument("--runtime", help="the AppImage runtime (apps/appimagert) for real AppImages of PROGRAM")
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
    appimage_channels(out, t, app, seed, program, a.runtime)
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
