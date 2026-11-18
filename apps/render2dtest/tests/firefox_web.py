"""lib/render2d in stock Firefox (G6e3): the checks of engine_web.mjs, where Playwright cannot go.  Firefox has
WebGPU only in a real window (its headless mode and Playwright's build have no adapter), so this starts the
installed Firefox itself, shown without activation, with a profile of its own in a temporary folder, and lets the
page report: a small server injects a script into index.html that sends the console's lines by POST and takes
the next address in the answer.  Nothing drives the page (no input), and only Firefox's own processes of that
profile are ended at the end (found by the profile's path on their command line, never by the launcher's pid,
which exits at once).

Per bundle (scalar, simd, threads): the GPU test (?gpuenv=jfgui_GPU_PROBE=test: the coordinates at 1e7 and at 1e-6
units per pixel, the dash pattern far away, the visibility, the atlas's bytes, the ownership rule, judged in the
page by the code of the Vulkan tests; the 91 images sent as hex and compared here with render2dtest --dump of the
native program on this machine, Voll and Basis), the CPU path without WebGPU (?gpu=0, jfgui_GPU_PROBE=cpu: its 91
images byte for byte as the native program's) and the demo without a device (its canvas counted in the page:
paper, the red and the blue axis).  --bench: the measuring scene (jfgui_GPU_PROBE=bench), --runs=N times in turns
with --thin (hairlines) and without.

    python build_rs.py app render2dtest --target win
    python build_rs.py app render2dtest --target web
    python apps/render2dtest/tests/firefox_web.py [--build=build] [--modes=simd,threads,scalar] [--firefox=PATH]
                                                  [--native=EXE] [--save=DIR] [--only=gpu,cpu,demo] [--pref=NAME=VALUE]
                                                  [--bench [--runs=3] [--n=1000000]]

Exit code 1 when a check failed, 0 with SKIP when Firefox or its WebGPU is not there.  Keep to one run per case
(a real window opens on the desktop; it does not take the focus).
"""
import argparse
import ctypes
import ctypes.wintypes as wt
import http.server
import os
import queue
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
MAX_DELTA, MAX_SHARE = 2, 0.002          # as engine_web.mjs: a code may round the other way

REPORT_JS = r"""
(() => {
  const q = [];
  let busy = false;
  let quiet = 0;
  const send = () => {
    if (busy) return;
    // (nothing to say: a beat every second, so that the page learns its next address after its last line)
    if (!q.length && ++quiet < 5) return;
    quiet = 0;
    busy = true;
    const body = q.splice(0, q.length).join('\n');
    fetch('/__report', {method: 'POST', body}).then(r => r.text()).then(t => {
      if (t.startsWith('goto ')) location.href = t.slice(5);
    }).catch(() => {}).finally(() => { busy = false; });
  };
  for (const k of ['log', 'error', 'warn']) {
    const o = console[k].bind(console);
    console[k] = (...a) => { o(...a); q.push(a.map(String).join(' ')); };
  }
  addEventListener('error', e => q.push('pageerror ' + e.message));
  addEventListener('unhandledrejection', e => q.push('pageerror ' + e.reason));
  setInterval(send, 200);
  (async () => {
    try {
      const a = await navigator.gpu?.requestAdapter();
      const i = a?.info;
      q.push('adapter-info ' + (a ? `${i.vendor} ${i.architecture} ${i.device} ${i.description}` : 'none') + ' / ' + navigator.userAgent);
    } catch (e) { q.push('adapter-info ' + e); }
  })();
  // the demo without a device: count the drawing's colours on the canvas after 4 s
  const p = new URLSearchParams(location.search);
  if (p.get('gpu') === '0' && !p.get('gpuenv')) setTimeout(() => {
    try {
      const c = document.getElementById('screen');
      const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
      const count = (r, g, b) => { let n = 0; for (let i = 0; i < d.length; i += 4) if (Math.abs(d[i] - r) <= 2 && Math.abs(d[i + 1] - g) <= 2 && Math.abs(d[i + 2] - b) <= 2) n++; return n; };
      q.push(`democount ${c.width}x${c.height} paper ${count(0xF4, 0xF1, 0xEA)} red ${count(0xD0, 0x20, 0x20)} blue ${count(0x20, 0x40, 0xC0)} grey ${count(0x30, 0x34, 0x38)}`);
    } catch (e) { q.push('democount error ' + e); }
  }, 4000);
})();
"""

PREFS = {
    "browser.shell.checkDefaultBrowser": False, "browser.startup.homepage_override.mstone": "ignore",
    "startup.homepage_welcome_url": "", "startup.homepage_welcome_url.additional": "", "browser.aboutwelcome.enabled": False,
    "browser.startup.page": 0, "browser.newtabpage.enabled": False, "browser.sessionstore.resume_from_crash": False,
    "browser.tabs.warnOnClose": False, "browser.warnOnQuit": False, "toolkit.startup.max_resumed_crashes": -1,
    "datareporting.policy.dataSubmissionEnabled": False, "datareporting.healthreport.uploadEnabled": False,
    "toolkit.telemetry.enabled": False, "toolkit.telemetry.reportingpolicy.firstRun": False, "app.normandy.enabled": False,
    "app.update.auto": False, "app.update.enabled": False, "extensions.update.enabled": False, "browser.search.update": False,
    "media.gmp-manager.updateEnabled": False, "browser.safebrowsing.malware.enabled": False, "browser.safebrowsing.phishing.enabled": False,
    "browser.safebrowsing.downloads.enabled": False, "network.captive-portal-service.enabled": False,
    "network.connectivity-service.enabled": False, "browser.discovery.enabled": False, "extensions.getAddons.cache.enabled": False,
    "services.settings.server": "", "browser.region.network.url": "", "dom.ipc.reportProcessHangs": False,
    "dom.webgpu.enabled": True,
    # a covered window stays visible to the page (its timers and frames run on): no z-order change needed
    "widget.windows.window_occlusion_tracking.enabled": False, "dom.min_background_timeout_value": 4,
    "dom.timeout.enable_budget_timer_throttling": False,
}


def prefs_js():
    def v(x):
        return ("true" if x else "false") if isinstance(x, bool) else (str(x) if isinstance(x, int) else '"' + x + '"')
    return "".join(f'user_pref("{k}", {v(x)});\n' for k, x in PREFS.items())


class Server(http.server.ThreadingHTTPServer):
    daemon_threads = True


def make_handler(root, lines, next_url):
    class H(http.server.BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def do_POST(self):
            n = int(self.headers.get("content-length", 0))
            body = self.rfile.read(n).decode("utf-8", "replace")
            for ln in body.split("\n"):
                lines.put(ln)
            go = next_url[0] if next_url[0] else ""
            if go:
                next_url[0] = None
            data = (f"goto {go}" if go else "ok").encode()
            self.send_response(200)
            self.send_header("content-type", "text/plain")
            self.send_header("content-length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            path = self.path.split("?", 1)[0].lstrip("/") or "index.html"
            if path == "favicon.ico":
                self.send_response(204)
                self.end_headers()
                return
            if path == "__r2d_report.js":
                data, kind = REPORT_JS.encode(), "text/javascript"
            else:
                f = (root / path).resolve()
                if root not in f.parents or not f.is_file():
                    self.send_response(404)
                    self.end_headers()
                    return
                data = f.read_bytes()
                kind = {".js": "text/javascript", ".css": "text/css", ".html": "text/html", ".wasm": "application/wasm",
                        ".json": "application/json"}.get(f.suffix, "application/octet-stream")
                if path == "index.html":
                    data = data.replace(b'<script type="module" src="loader.js">', b'<script src="__r2d_report.js"></script>\n<script type="module" src="loader.js">')
            self.send_response(200)
            self.send_header("content-type", kind)
            self.send_header("content-length", str(len(data)))
            self.send_header("cache-control", "no-store")
            self.send_header("cross-origin-opener-policy", "same-origin")
            self.send_header("cross-origin-embedder-policy", "require-corp")
            self.end_headers()
            self.wfile.write(data)
    return H


def firefox_pids(profile):
    """the processes of Firefox that name this profile on their command line"""
    ps = ["powershell", "-NoProfile", "-Command",
          "Get-CimInstance Win32_Process -Filter \"Name='firefox.exe'\" | ForEach-Object { \"$($_.ProcessId)`t$($_.CommandLine)\" }"]
    out = subprocess.run(ps, capture_output=True, text=True, errors="replace").stdout
    key = str(profile).lower()
    return [int(ln.split("\t", 1)[0]) for ln in out.splitlines() if "\t" in ln and key in ln.lower()]


def start_firefox(exe, profile, url):
    si = subprocess.STARTUPINFO()
    si.dwFlags |= subprocess.STARTF_USESHOWWINDOW
    si.wShowWindow = 4  # SW_SHOWNOACTIVATE: shown without taking the focus
    env = dict(os.environ, MOZ_CRASHREPORTER_DISABLE="1")
    subprocess.Popen([exe, "-no-remote", "-new-instance", "-profile", str(profile), url], startupinfo=si, env=env,
                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def stop_firefox(profile):
    for _ in range(3):
        pids = firefox_pids(profile)
        if not pids:
            return True
        for p in pids:
            subprocess.run(["taskkill", "/PID", str(p), "/T", "/F"], capture_output=True)
        time.sleep(1.0)
    return not firefox_pids(profile)


def compare(a, b):
    if len(a) != len(b):
        return (10 ** 9, 255, [0, 0, 0, 0])
    pixels, largest, hist = 0, 0, [0, 0, 0, 0]
    if a == b:
        return (0, 0, hist)
    for i in range(0, len(a), 4):
        d = max(abs(a[i] - b[i]), abs(a[i + 1] - b[i + 1]), abs(a[i + 2] - b[i + 2]))
        if d:
            pixels += 1
            largest = max(largest, d)
            hist[min(d, 4) - 1] += 1
    return (pixels, largest, hist)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--build", default=str(ROOT / "build"))
    ap.add_argument("--modes", default=None)
    ap.add_argument("--firefox", default=r"C:\Program Files\Mozilla Firefox\firefox.exe")
    ap.add_argument("--native", default=str(ROOT / "build" / "win" / "render2dtest.exe"))
    ap.add_argument("--save", default=None)
    ap.add_argument("--bench", action="store_true")
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--n", default=None)
    ap.add_argument("--timeout", type=int, default=600)
    ap.add_argument("--only", default="gpu,cpu,demo", help="the pages per bundle: gpu, cpu, demo")
    ap.add_argument("--pref", action="append", default=[], help="NAME=VALUE: one more pref for the profile (true, false, numbers, else a string)")
    a = ap.parse_args()
    for kv in a.pref:
        k, v = kv.split("=", 1)
        PREFS[k] = True if v == "true" else False if v == "false" else int(v) if v.lstrip("-").isdigit() else v
    if not Path(a.firefox).is_file():
        print(f"SKIP: no Firefox at {a.firefox}")
        return 0
    web = (Path(a.build) / "web" / "render2dtest").resolve()
    if not (web / "index.html").is_file():
        print(f"FAIL no web build at {web} (python build_rs.py app render2dtest --target web)")
        return 1
    modes = (a.modes or ("simd" if a.bench else "simd,threads,scalar")).split(",")
    here = Path(tempfile.mkdtemp(prefix="render2d-ff-"))
    reference = {}
    if not a.bench:
        (here / "dump").mkdir()
        r = subprocess.run([a.native, "--dump", str(here / "dump")], capture_output=True, text=True, errors="replace",
                           env=dict(os.environ, jfgui_TEST_NOACTIVATE="1"))
        if r.returncode:
            print(f"FAIL the native program ({a.native}): {r.returncode}\n{r.stdout}{r.stderr}")
            shutil.rmtree(here, ignore_errors=True)
            return 1
        print(r.stdout.strip().splitlines()[-1])
        for f in (here / "dump").glob("*.bgra"):
            reference[f.stem] = f.read_bytes()
    profile = here / "profile"
    profile.mkdir()
    (profile / "user.js").write_text(prefs_js(), encoding="utf-8")
    lines, next_url = queue.Queue(), [None]
    srv = Server(("127.0.0.1", 0), make_handler(web, lines, next_url))
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{srv.server_address[1]}/index.html"
    # the pages in order: each ends with its line, then the server sends the next address with its answer
    pages = []
    if a.bench:
        for run in range(a.runs):
            for thin in (False, True):
                env = "jfgui_GPU_PROBE=bench" + (f",jfgui_GPU_PROBE_N={a.n}" if a.n else "") + (",jfgui_GPU_PROBE_THIN=1" if thin else "")
                pages.append((f"bench {'hairlines' if thin else 'styled'} run {run + 1}", modes[0], f"{base}?compute={modes[0]}&gpuenv={env}", "end"))
    else:
        only = a.only.split(",")
        for m in modes:
            if "gpu" in only:
                pages.append(("gpu", m, f"{base}?compute={m}&gpuenv=jfgui_GPU_PROBE=test", "end"))
            if "cpu" in only:
                pages.append(("cpu", m, f"{base}?compute={m}&gpu=0&gpuenv=jfgui_GPU_PROBE=cpu", "end"))
            if "demo" in only:
                pages.append(("demo", m, f"{base}?compute={m}&gpu=0", "democount"))
    failures = ran = 0
    results = []
    started = False
    try:
        for k, (kind, mode, url, until) in enumerate(pages):
            if not started:
                start_firefox(a.firefox, profile, url)
                started = True
            got, images, piece, t0, adapter = [], {}, "", time.time(), ""
            end = None
            while end is None:
                try:
                    ln = lines.get(timeout=1.0)
                except queue.Empty:
                    if time.time() - t0 > a.timeout:
                        end = "timeout"
                    continue
                ln = ln.rstrip()
                if ln.startswith("part "):
                    if ln == "part end":
                        _, name, hx = piece.split(" ", 2)
                        images[name] = bytes.fromhex(hx)
                        piece = ""
                    else:
                        piece += ln[ln.index(" ", 5) + 1:]
                    continue
                if ln.startswith("adapter-info "):
                    adapter = ln[13:]
                    continue
                got.append(ln)
                if kind.startswith("bench") and ln.startswith("bench"):
                    print(f"{kind}: {ln}")
                if until == "end" and (ln.startswith("render2dtest: ") or ln.startswith("SKIP render2dtest")):
                    end = ln
                if until == "democount" and ln.startswith("democount"):
                    end = ln
            # the next page with the answer to the page's next report
            if k + 1 < len(pages):
                next_url[0] = pages[k + 1][2]
            fails = [g for g in got if g.startswith("FAIL") or g.startswith("pageerror") or g.startswith("WebGPU")]
            for f in fails:
                print(f"{mode} {kind}: {f}")
            if kind == "gpu":
                print(f"{mode}: the page's adapter: {adapter}")
                if end.startswith("SKIP"):
                    print(f"SKIP {mode}: {end}")
                    continue
                ran += 1
                good = end.startswith("render2dtest: ") and " 0 failed" in end and not fails
                print(f"{'PASS' if good else 'FAIL'} firefox {mode}: {end} ({next((g for g in got if g.startswith('adapter:')), 'no adapter line')})")
                for g in got:
                    if re.match(r"^ok .*(the drawing at|dash patterns at|dashes at|1e-6 units|seed |culled|blend|atlas)", g):
                        print(f"  {g}")
                total = 0
                for level in ("voll", "basis"):
                    names = [n for n in reference if n.startswith(level + "_")]
                    worst, bad, differing, pixels = None, 0, 0, 0
                    for key in names:
                        name = key[len(level) + 1:]
                        if name not in images:
                            print(f"  FAIL no image {name}")
                            good = False
                            continue
                        c = compare(images[name], reference[key])
                        if a.save and c[0]:
                            Path(a.save).mkdir(parents=True, exist_ok=True)
                            (Path(a.save) / f"ff_{mode}_{name}.bgra").write_bytes(images[name])
                        fine = c[1] <= MAX_DELTA or c[0] <= (len(images[name]) // 4) * MAX_SHARE
                        pixels += len(images[name]) // 4
                        differing += c[0]
                        if not fine:
                            bad += 1
                            print(f"  FAIL {name}: against Vulkan {level}: {c[0]} pixels differ, largest {c[1]}; by 1 / 2 / 3 / more: {' / '.join(map(str, c[2]))}")
                        if c[0] and (worst is None or (c[1], c[0]) > (worst[1][1], worst[1][0])):
                            worst = (name, c)
                    w = f"{worst[0]}: {worst[1][0]} pixels, largest {worst[1][1]}; by 1 / 2 / 3 / more: {' / '.join(map(str, worst[1][2]))}" if worst else "none"
                    print(f"  {'FAIL' if bad else 'ok  '} {len(names)} images against Vulkan {level}: {differing} of {pixels} pixels differ; the worst: {w}")
                    good = good and not bad and len(names) > 0
                    total += len(names)
                if not total:
                    good = False
                results.append((mode, kind, good))
                failures += not good
            elif kind == "cpu":
                ran += 1
                names = [n for n in reference if n.startswith("cpu_")]
                same, differ, worst = 0, 0, None
                for key in names:
                    if key not in images:
                        print(f"  FAIL {mode}: no CPU image {key}")
                        differ += 1
                        continue
                    c = compare(images[key], reference[key])
                    if c[0]:
                        differ += 1
                        worst = worst if worst and worst[1][0] >= c[0] else (key, c)
                    else:
                        same += 1
                good = end.startswith("render2dtest: ") and " 0 failed" in end and names and not differ and not fails
                print(f"{'PASS' if good else 'FAIL'} {mode} CPU path (gpu=0): {same} of {len(names)} images byte for byte as the native program's"
                      f"{f'; the worst: {worst[0]}: {worst[1][0]} pixels, largest {worst[1][1]}' if worst else ''} ({next((g for g in got if g.startswith('ok ')), end)})")
                results.append((mode, kind, good))
                failures += not good
            elif kind == "demo":
                m = re.match(r"democount (\S+) paper (\d+) red (\d+) blue (\d+) grey (\d+)", end or "")
                good = bool(m) and int(m.group(2)) > 50000 and int(m.group(3)) > 1500 and int(m.group(4)) > 1500 and not fails
                print(f"{'PASS' if good else 'FAIL'} {mode} the demo without a device (gpu=0): {end}")
                results.append((mode, kind, good))
                failures += not good
            else:
                results.append((mode, kind, end.startswith("render2dtest: ")))
    finally:
        stopped = stop_firefox(profile) if started else True
        srv.shutdown()
        shutil.rmtree(here, ignore_errors=True)
        print(f"firefox ended: {'yes' if stopped else 'NO: processes of the profile are left'}; the profile removed")
    if not a.bench and not ran:
        print("SKIP: Firefox has no working WebGPU here")
        return 0
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
