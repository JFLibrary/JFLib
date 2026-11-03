"""academy_e2e.py -- installing courses over real HTTP: jfn-sim against a local server in this script

    python tools/academy_e2e.py [--sim PATH] [--keep]

Not in `build_rs.py test JFNetworker`: those are .uis scripts against the GUI program, whose PERMISSIONS name only
the courses' host and ask about it, and no process.  Letting the shipped window reach localhost and start a server
without a question would widen what it may do for the sake of a test.  jfn-sim has its own rules
(apps/JFNetworker/PERMISSIONS.jfn-sim: the courses' host and localhost), so this script drives that: the same code
(jfn_run::install on update::fetch) on the same sockets as the window.  It needs jfn-sim built, by default in
build/<any>/<win|linux>/, the newest one found; --sim names one.

The server (127.0.0.1, a free port) serves a course this script writes, its index made by tools/academy_index.py,
under these roots:
    /good/    everything right                       /flaky/    the big file's first transfer stops half way
    /badhash/ one file differs in one byte           /short/    a file is shorter than the index says
    /long/    a file is longer than the index says   /slow/     the big file comes slowly (cancel)
    /missing/ a file answers 404                     /redirect/ 302 to /good/
    /noindex/ no index.json                          /badindex/ an index that is no JSON
    /hugeindex/ an index over the limit              /newer/    a second version of the course (update)
    /badcourse/ a newer version whose course.json names a lab that is not there
    /flakynew/ that newer version, its big file stops half way every time
Every request is logged with its Range header, so a resume is seen, not assumed.

Checked: an install and its bytes, a resume with Range, each refusal with its kind in brackets on stderr and its exit
code, cancel with --stop-after and what it leaves, an update that fails and keeps the old course, a working update,
course-list, course-remove, a redirect, a host PERMISSIONS.jfn-sim does not name (no connection is made).
Writes only below a fresh folder in build/academy_e2e, removed at the end unless --keep.  Exit code: the number of
failed checks.
"""
import argparse
import hashlib
import json
import os
import shutil
import socket
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(Path(__file__).resolve().parent))
import academy_index  # noqa: E402

BIG = 512 << 10
CHUNK = 16 << 10
ROUTES = {}
REQUESTS = []
LOCK = threading.Lock()
FLAKY_ONCE = set()


def course_files(version, broken=False):
    """the files of the test's course in a version"""
    steps = [{"id": "eins", "kind": "lesson", "title": ["Eins", "One"], "file": "eins.md"}]
    if broken:
        steps.append({"id": "labor", "kind": "lab", "title": ["Labor", "Lab"], "file": "labs/fehlt.jfnet"})
    course = {"format": "jfn-course", "version": 1, "course_version": version, "title": ["Probe", "Probe"], "summary": ["Kurz", "Short"],
              "level": "beginner", "minutes": 1, "units": [{"title": ["Teil", "Part"], "steps": steps}]}
    big = (f"Fassung {version} ".encode() * (BIG // 10))[:BIG]
    return {"course.json": json.dumps(course).encode(), "de/eins.md": f"# Eins\n\nFassung {version}.\n".encode(),
            "en/eins.md": f"# One\n\nVersion {version}.\n".encode(), "img/gross.bin": big}


def index_of(files, version):
    return {"format": "jfn-academy", "version": 1, "courses": [{
        "id": "probe", "title": ["Probe", "Probe"], "summary": ["Kurz", "Short"], "level": "beginner", "minutes": 1, "version": version,
        "files": [{"path": p, "size": len(b), "sha256": hashlib.sha256(b).hexdigest()} for p, b in sorted(files.items())]}]}


def channel(name, files, version=1, listed=None):
    """a root: its files and its index (listed: the files the index is made from, when they differ)"""
    ROUTES[f"/{name}/index.json"] = academy_index.text_of(index_of(listed or files, version))
    for p, b in files.items():
        ROUTES[f"/{name}/probe/{p}"] = b


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def send(self, status, body=b"", extra=()):
        self.send_response(status)
        self.send_header("Content-Length", str(len(body)))
        for k, v in extra:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        path = self.path.split("?")[0]
        rng = self.headers.get("Range", "")
        with LOCK:
            REQUESTS.append((path, rng))
        root = path.split("/")[1] if path.count("/") > 1 else ""
        if root == "redirect":
            return self.send(302, b"", [("Location", path.replace("/redirect/", "/good/", 1))])
        data = ROUTES.get(path)
        if data is None:
            return self.send(404, b"not found")
        start = 0
        if rng.startswith("bytes=") and rng.endswith("-"):
            start = int(rng[6:-1])
            if start >= len(data):
                return self.send(416, b"", [("Content-Range", f"bytes */{len(data)}")])
        self.send_response(206 if start else 200)
        if start:
            self.send_header("Content-Range", f"bytes {start}-{len(data) - 1}/{len(data)}")
        self.send_header("Content-Length", str(len(data) - start))
        self.end_headers()
        body = data[start:]
        big = path.endswith("/img/gross.bin")
        stop = big and (root == "flakynew" or (root == "flaky" and path not in FLAKY_ONCE))
        if stop:
            FLAKY_ONCE.add(path)
            self.wfile.write(body[:len(body) // 2])
            self.wfile.flush()
            self.connection.shutdown(socket.SHUT_RDWR)
            self.close_connection = True
            return
        if big and root == "slow":
            for i in range(0, len(body), CHUNK):
                try:
                    self.wfile.write(body[i:i + CHUNK])
                    self.wfile.flush()
                except OSError:
                    return
                time.sleep(0.02)
            return
        self.wfile.write(body)


class Run:
    def __init__(self, sim, base, work):
        self.sim, self.base, self.work, self.failed, self.ok = str(sim), base, work, 0, 0

    def check(self, name, cond, detail=""):
        if cond:
            self.ok += 1
            print(f"ok   {name}")
        else:
            self.failed += 1
            print(f"FAIL {name} {detail}")

    def sim_run(self, *args):
        r = subprocess.run([self.sim, *args], cwd=ROOT, capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=120, stdin=subprocess.DEVNULL)
        return r.returncode, r.stdout, r.stderr

    def folder(self, name):
        d = self.work / name
        d.mkdir(parents=True)
        return str(d)

    def install(self, root, into, course="probe", *more):
        return self.sim_run("course-install", f"{self.base}/{root}/", course, "--into", into, *more)

    def refused(self, name, root, kind, code=1):
        into = self.folder(name)
        rc, _, err = self.install(root, into)
        self.check(f"{name}: exit {code}, [{kind}]", rc == code and f"[{kind}]" in err, f"exit {rc}: {err.strip()}")
        self.check(f"{name}: no course", not (Path(into) / "probe").exists())
        return into, err


def ranges(path):
    with LOCK:
        return [r for p, r in REQUESTS if p == path]


def tree(d):
    return {p.relative_to(d).as_posix(): p.read_bytes() for p in Path(d).rglob("*") if p.is_file()}


def find_sim():
    exe = "jfn-sim.exe" if os.name == "nt" else "jfn-sim"
    found = sorted((ROOT / "build").glob(f"**/{exe}"), key=lambda p: p.stat().st_mtime)
    return found[-1] if found else None


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--sim", type=Path, help="the jfn-sim to test")
    ap.add_argument("--keep", action="store_true", help="keep build/academy_e2e")
    a = ap.parse_args()
    sim = a.sim or find_sim()
    if not sim or not Path(sim).is_file():
        print("academy_e2e: no jfn-sim (python build_rs.py app JFNetworker, or --sim PATH)", file=sys.stderr)
        return 2
    work = ROOT / "build" / "academy_e2e" / str(os.getpid())
    work.mkdir(parents=True)

    v1, v2 = course_files(1), course_files(2)
    channel("good", v1)
    channel("flaky", v1)
    channel("slow", v1)
    channel("newer", v2, 2)
    channel("flakynew", v2, 2)
    channel("badcourse", course_files(2, broken=True), 2)
    bad = dict(v1)
    bad["de/eins.md"] = bytes([v1["de/eins.md"][0] ^ 1]) + v1["de/eins.md"][1:]
    channel("badhash", bad, 1, v1)
    channel("short", {**v1, "de/eins.md": v1["de/eins.md"][:-3]}, 1, v1)
    channel("long", {**v1, "de/eins.md": v1["de/eins.md"] + b"mehr"}, 1, v1)
    gone = dict(v1)
    del gone["en/eins.md"]
    channel("missing", gone, 1, v1)
    ROUTES["/badindex/index.json"] = b"{ kein JSON"
    ROUTES["/hugeindex/index.json"] = b" " * ((4 << 20) + 1)

    srv = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    srv.daemon_threads = True
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    run = Run(sim, f"http://127.0.0.1:{srv.server_address[1]}", work)
    want = {f"probe/{p}": b for p, b in v1.items()}
    try:
        # ---- the list
        rc, out, err = run.sim_run("course-list", f"{run.base}/good/")
        run.check("course-list", rc == 0 and out.startswith("probe Version 1, 4 Dateien"), out + err)
        # ---- a course, byte for byte
        into = run.folder("good")
        rc, out, err = run.install("good", into)
        run.check("install", rc == 0 and "installiert" in out, out + err)
        got = {k: v for k, v in tree(into).items() if not k.startswith(".")}
        run.check("  byte for byte", got == want, str(sorted(got)))
        run.check("  nothing beside it", sorted(p.name for p in Path(into).iterdir() if p.name != ".install-lock") == ["probe"])
        rc, out, err = run.sim_run("course", f"{into}/probe")
        run.check("  jfn-sim course likes it", rc == 0, out + err)
        rc, out, err = run.sim_run("course-list", f"{run.base}/newer/", "--into", into)
        run.check("  a newer version is seen", rc == 0 and "neuere Version" in out, out + err)
        # ---- a redirect
        into2 = run.folder("redirect")
        rc, out, err = run.install("redirect", into2)
        run.check("a redirect", rc == 0, out + err)
        # ---- a transfer that stops half way goes on with Range, by itself
        into2 = run.folder("flaky")
        rc, out, err = run.install("flaky", into2)
        run.check("a transfer that stops half way", rc == 0, out + err)
        rs = ranges("/flaky/probe/img/gross.bin")
        run.check("  went on with a Range", len(rs) == 2 and rs[0] == "" and rs[1].startswith("bytes=") and int(rs[1][6:-1]) > 0, str(rs))
        run.check("  byte for byte", (Path(into2) / "probe" / "img" / "gross.bin").read_bytes() == v1["img/gross.bin"])
        # ---- cancel: what came stays, the next run goes on
        into2 = run.folder("slow")
        rc, out, err = run.install("slow", into2, "probe", "--stop-after", str(64 << 10))
        run.check("cancelled: exit 3", rc == 3 and "[cancelled]" in err, f"exit {rc}: {out} {err}")
        part = Path(into2) / ".probe.part" / "img" / "gross.bin"
        run.check("  no course", not (Path(into2) / "probe").exists())
        have = part.stat().st_size if part.exists() else 0
        run.check("  a part stays", 0 < have < BIG, str(have))
        with LOCK:
            REQUESTS.clear()
        rc, out, err = run.install("slow", into2)
        run.check("  the next run finishes", rc == 0, out + err)
        rs = ranges("/slow/probe/img/gross.bin")
        run.check("  from where it was", len(rs) == 1 and rs[0] == f"bytes={have}-", str(rs))
        with LOCK:
            asked = [p for p, _ in REQUESTS if p.endswith("/course.json")]
        run.check("  what was whole is not asked for again", not asked, str(asked))
        run.check("  byte for byte", {k: v for k, v in tree(into2).items() if not k.startswith(".")} == want)
        # ---- each refusal
        into2, err = run.refused("wrong checksum", "badhash", "checksum")
        run.check("  says which", "de/eins.md" in err, err)
        run.check("  the wrong file does not stay", not (Path(into2) / ".probe.part" / "de" / "eins.md").exists())
        run.refused("a file too short", "short", "checksum")
        run.refused("a file too long", "long", "checksum")
        run.refused("a file that is gone", "missing", "missing")
        rc, _, err = run.sim_run("course-list", f"{run.base}/noindex/")
        run.check("no list at all: [missing]", rc == 1 and "[missing]" in err, err)
        rc, _, err = run.sim_run("course-list", f"{run.base}/badindex/")
        run.check("no JSON: [bad-index]", rc == 1 and "[bad-index]" in err, err)
        rc, _, err = run.sim_run("course-list", f"{run.base}/hugeindex/")
        run.check("a list too large: [too-large]", rc == 1 and "[too-large]" in err, err)
        # ---- nobody answers
        s = socket.socket()
        s.bind(("127.0.0.1", 0))
        closed = s.getsockname()[1]
        s.close()
        rc, _, err = run.sim_run("course-list", f"http://127.0.0.1:{closed}/x/")
        run.check("nobody answers: [offline]", rc == 1 and "[offline]" in err, err)
        # ---- another host: refused before anything leaves
        rc, _, err = run.sim_run("course-list", "https://example.org/kurse/")
        run.check("another host: [other-host]", rc == 1 and "[other-host]" in err and "PERMDENY net.connect example.org" in err, err)
        # ---- an update: what fails keeps the old course
        rc, out, err = run.install("flakynew", into)
        run.check("an update that breaks off every time fails", rc == 1 and "[interrupted]" in err, out + err)
        run.check("  the old course stands", {k: v for k, v in tree(into).items() if k.startswith("probe/")} == want)
        rc, out, err = run.install("badcourse", into)
        run.check("an update that is no good course: [bad-course]", rc == 1 and "[bad-course]" in err and "labs/fehlt.jfnet" in err, out + err)
        run.check("  the old course stands", {k: v for k, v in tree(into).items() if k.startswith("probe/")} == want)
        rc, out, err = run.install("newer", into)
        run.check("the update that works", rc == 0, out + err)
        run.check("  byte for byte", {k: v for k, v in tree(into).items() if k.startswith("probe/")} == {f"probe/{p}": b for p, b in v2.items()})
        run.check("  nothing beside it", sorted(p.name for p in Path(into).iterdir() if p.name != ".install-lock") == ["probe"])
        # ---- and out again
        rc, out, err = run.sim_run("course-remove", "probe", "--into", into)
        run.check("course-remove", rc == 0 and not (Path(into) / "probe").exists(), out + err)
        # ---- a folder source, for comparison: the same bytes
        rc, out, err = run.sim_run("course-install", "apps/JFNetworker/academy", "all", "--into", into)
        run.check("the courses of the repository from their folder", rc == 0, out + err)
    finally:
        srv.shutdown()
        if not a.keep:
            shutil.rmtree(work, ignore_errors=True)
    print(f"academy_e2e: {run.ok} ok, {run.failed} failed ({sim})")
    return run.failed


if __name__ == "__main__":
    sys.exit(main())
