"""cookie_check.py -- the cookie tests of http-state (abarth/http-state, the parser tests behind
RFC 6265) through netio's HTTP client: its cookie jar (netpkt::cookie with the public suffix
list), its redirects and its HTTP proxy.

    python tools/cookie_check.py [--target linux|win] [--only NAME,..] [-v]

The suite's files (tests/data/parser/NAME-test and NAME-expected, 222 cases, 28 KB) come once with
tools/suite_fetch.py into build/vectors at a pinned commit.  For each case a fresh client asks for
http://home.example.org:8888/cookie-parser?NAME as http-state's own server has it: the answer is a
302 with the case's header lines (Set-Cookie, sometimes a Location) and a Location of
/cookie-parser-result?NAME when it gave none; the client follows it, and the Cookie header that
arrives there is compared with the case's expected one.  The server here is that test server, run
as the client's HTTP proxy (requests in absolute form), so home.example.org and the other names
of the suite need no /etc/hosts.  Each case runs `coretest --http-get URL --proxy .. --redirects 5
--cookies` (python build_rs.py app coretest --target linux first).

Cases where RFC 6265bis (which netpkt::cookie follows) reads a header otherwise than the 2011
suite are KNOWN, each with the answer it gives and the reason; any other answer of such a case is
a failure, and a case that becomes right while KNOWN is reported too, so the list stays true.
Exit code 1 on any other difference."""
import argparse
import http.server
import re
import subprocess
import sys
import threading
import urllib.parse
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REPO = "abarth/http-state"
COMMIT = "155e45c6168e6de2eb727193010fd12e3c789185"

NAMES = (
    [f"{i:04d}" for i in range(1, 29)]
    + [f"attribute{i:04d}" for i in range(1, 27)]
    + [f"charset{i:04d}" for i in range(1, 5)]
    + [f"chromium{i:04d}" for i in list(range(1, 20)) + [21]]
    + [f"comma{i:04d}" for i in range(1, 8)]
    + ["disabled-chromium0020", "disabled-chromium0022", "disabled-chromium0023", "disabled-path0029"]
    + [f"domain{i:04d}" for i in list(range(1, 30)) + [31] + list(range(33, 43))]
    + [f"mozilla{i:04d}" for i in range(1, 18)]
    + [f"name{i:04d}" for i in range(1, 34)]
    + ["optional-domain0030", "optional-domain0041", "optional-domain0042", "optional-domain0043"]
    + ["ordering0001"]
    + [f"path{i:04d}" for i in range(1, 33)]
    + [f"value{i:04d}" for i in range(1, 7)]
)

# where netio's client gives another answer than the 2011 suite, by case: (that answer, the reason);
# None is the result page reached without a Cookie header
_PAST = (None, "its Expires is 2019, a date that has passed (the suite is of 2011): the cookie is gone")
KNOWN = {
    "0002": _PAST,
    "comma0006": _PAST,
    "comma0007": _PAST,
    "disabled-path0029": (None, "disabled in the suite: the request's %6F is not decoded for path matching (RFC 6265 5.1.4 compares the paths as written)"),
    "disabled-chromium0022": ("(error: error: malformed HTTP answer)", "disabled in the suite: a NUL in a header line makes netio refuse the answer (RFC 9110 5.5 allows rejecting it)"),
}


class Suite(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    folder = None
    seen = {}

    def log_message(self, *a):
        pass

    def answer(self, status, lines, body=b""):
        out = [f"HTTP/1.1 {status} {'Found' if status == 302 else 'OK'}".encode()] + lines
        out.append(f"Content-Length: {len(body)}".encode())
        self.wfile.write(b"\r\n".join(out) + b"\r\n\r\n" + body)

    def do_GET(self):
        u = urllib.parse.urlsplit(self.path)
        if u.path == "/cookie-parser" or u.path.startswith("/cookie-parser/"):
            name = u.query
            lines, located = [], False
            for line in (self.folder / f"{name}-test").read_bytes().splitlines():
                m = re.match(rb"(\S+):\s*(.*)", line)
                if not m:
                    continue
                if m.group(1).lower() == b"location":
                    located = True
                lines.append(m.group(1) + b": " + m.group(2))
            if not located:
                lines.append(f"Location: /cookie-parser-result?{name}".encode())
            self.answer(302, lines)
        elif u.path == "/cookie-parser-result" or u.path.startswith("/cookie-parser-result/"):
            # http.server reads header lines as Latin-1; the client sent UTF-8.  More than one
            # Cookie header is a difference of its own
            got = [v.encode("latin-1").decode("utf-8", "replace") for v in self.headers.get_all("Cookie") or []]
            self.seen[u.query] = got[0] if len(got) == 1 else None if not got else f"({len(got)} Cookie headers: {got!r})"
            self.answer(200, [b"Content-Type: text/plain"], b"ok")
        else:
            self.answer(404, [])


def expected(folder, name):
    found = re.findall(rb"Cookie:\s*(.*)", (folder / f"{name}-expected").read_bytes())
    return found[0].decode("utf-8", "replace").rstrip("\r") if len(found) == 1 else None


def main():
    ap = argparse.ArgumentParser(description="http-state's cookie tests through netio's client")
    ap.add_argument("--target", default="win" if sys.platform == "win32" else "linux")
    ap.add_argument("--only", default="")
    ap.add_argument("-v", action="store_true", help="every case")
    a = ap.parse_args()
    names = [n for n in a.only.split(",") if n] or NAMES
    unknown = [n for n in names if n not in NAMES]
    if unknown:
        ap.error(f"--only: no such case: {', '.join(unknown)}")
    exe = ROOT / "build" / a.target / ("coretest.exe" if a.target == "win" else "coretest")
    if not exe.exists():
        print(f"cookie_check: {exe.relative_to(ROOT)} is missing (python build_rs.py app coretest --target {a.target})")
        return 2
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from suite_fetch import fetch
    files = [f"tests/data/parser/{n}-{k}" for n in NAMES for k in ("test", "expected")]
    folder = fetch(REPO, COMMIT, files, quiet=True) / "tests" / "data" / "parser"
    Suite.folder = folder
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Suite)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    proxy = f"http://127.0.0.1:{srv.server_address[1]}"
    counts = {"pass": 0, "known": 0, "fail": 0}
    for name in names:
        url = f"http://home.example.org:8888/cookie-parser?{name}"
        want = expected(folder, name)
        try:
            p = subprocess.run([str(exe), "--http-get", url, "--proxy", proxy, "--redirects", "5", "--cookies"], capture_output=True, text=True, timeout=60)
            got = Suite.seen.pop(name, "(no request)") if p.returncode == 0 else f"(error: {p.stdout.strip()})"
        except subprocess.TimeoutExpired:
            Suite.seen.pop(name, None)
            got = "(no answer within 60 s)"
        if got == want:
            kind = "pass" if name not in KNOWN else "fail"
            note = "" if name not in KNOWN else "  now right: take it out of KNOWN"
        elif name in KNOWN and got == KNOWN[name][0]:
            kind, note = "known", f"  ({KNOWN[name][1]})"
        else:
            kind = "fail"
            note = f"  (KNOWN gives {KNOWN[name][0]!r})" if name in KNOWN else ""
        counts[kind] += 1
        if a.v or kind == "fail" or note.startswith("  now"):
            print(f"  {kind.upper():<5} {name}: got {got!r}, expected {want!r}{note}")
    srv.shutdown()
    print(f"cookie_check: {len(names)} cases of {REPO} @ {COMMIT[:12]}: {counts['pass']} pass, {counts['known']} known, {counts['fail']} fail")
    return 1 if counts["fail"] else 0


if __name__ == "__main__":
    sys.exit(main())
