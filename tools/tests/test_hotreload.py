"""Hot reload: `build_rs.py watch` (tools/hotreload.py) and its runtime part (lib/ui/hotreload.rs).

    python tools/tests/test_hotreload.py              everything (builds a temporary app for Windows
                                                      and Linux; a few minutes)
    python tools/tests/test_hotreload.py --quick      the watcher alone (a few seconds, no build)
    python tools/tests/test_hotreload.py --target win the real app on one target

WatchTest drives tools/hotreload.Session with a stand-in program (a Python script acting as the
app: it answers "quit", ends with 3 like a diverged replay, or never ends) and a stand-in build:
the restart after a build, the old program kept running after a failed build (build.txt), a
fresh start after a divergence, a program that does not end killed, the file set and the poller.

ReplayTest writes apps/hrtest<pid> (removed afterwards), builds it with the watcher's build step
and starts it in the session like the watcher, headless under --script: a run records (clicks, a
page, typed text, the window's size and scale, the style, the inspector with a selection); after a
change of the source the next start replays it and the scripts expect the state back; a change
that moves the widgets diverges (exit 3) and the app starts afresh; a failed build keeps the old
program, and its inspector shows the error.

WindowTest (Windows) runs the window loop on a desktop of its own (CreateDesktop: its windows never
appear on the user's screen; posted key messages stand in for the user): the input recorded, the
watcher's "quit" (placement, log and carried state written), the replay of the next start, a
failed build opening the inspector.
"""
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from tools import hotreload  # noqa: E402

TARGETS = ["win", "linux"]

# the stand-in app: jfgui_HOTRELOAD's folder, argv[1] its behaviour
FAKE = r'''
import os, sys, time
d = os.environ["jfgui_HOTRELOAD"]
mode = sys.argv[1] if len(sys.argv) > 1 else "normal"
with open(os.path.join(d, "starts.txt"), "a") as f:
    f.write(f"{os.getpid()} {mode} {os.environ.get('jfgui_STATE', '')}\n")
if mode == "diverge":
    sys.exit(3)
end = time.time() + 60
ctl = os.path.join(d, "ctl")
while time.time() < end:
    if mode != "stubborn" and os.path.exists(ctl):
        if open(ctl).read().startswith("quit"):
            os.remove(ctl)
            with open(os.path.join(d, "ended.txt"), "a") as f:
                f.write(f"{os.getpid()}\n")
            sys.exit(0)
    time.sleep(0.02)
sys.exit(1)
'''

ERROR = """   Compiling ...
error[E0425]: cannot find value `undefined_value` in this scope
  --> apps\\hrfake\\main.rs:21:9
   |
21 |         undefined_value;
   |         ^^^^^^^^^^^^^^^ not found in this scope

error: aborting due to 1 previous error
build failed
"""


def wait_for(cond, timeout=10.0):
    end = time.time() + timeout
    while time.time() < end:
        if cond():
            return True
        time.sleep(0.02)
    return cond()


class WatchTest(unittest.TestCase):
    """the watcher's session with a stand-in app and build"""

    def setUp(self):
        self.name = f"hrfake{os.getpid()}"
        (ROOT / "build").mkdir(exist_ok=True)
        self.tmp = Path(tempfile.mkdtemp(prefix="hrfake-", dir=ROOT / "build"))
        self.fake = self.tmp / "fake.py"
        self.fake.write_text(FAKE, encoding="utf-8")
        self.builds = []
        self.lines = []
        self.s = hotreload.Session(self.name, "win", launcher=self.launch, builder=self.build, say=self.lines.append)

    def tearDown(self):
        if self.s.running():
            self.s.proc.kill()
            self.s.proc.wait()
        shutil.rmtree(self.s.dir, ignore_errors=True)
        for p in (self.s.exe, self.s.run_exe):
            p.unlink(missing_ok=True)
        shutil.rmtree(self.tmp, ignore_errors=True)

    def launch(self, env, args):
        return subprocess.Popen([sys.executable, str(self.fake), *args], env=dict(os.environ, **env), stdin=subprocess.DEVNULL)

    def build(self):
        ok = self.builds.pop(0) if self.builds else True
        if ok:
            self.s.exe.write_bytes(b"stand-in program")
        return ok, "[win] app ...\ndone (1.0 s)\n" if ok else ERROR

    def starts(self):
        p = self.s.path("starts.txt")
        return [ln.split() for ln in p.read_text().splitlines()] if p.exists() else []

    def ended(self):
        p = self.s.path("ended.txt")
        return p.read_text().split() if p.exists() else []

    def test_restart(self):
        self.assertTrue(self.s.on_change())
        self.assertTrue(wait_for(lambda: len(self.starts()) == 1))
        first = self.starts()[0][0]
        self.assertEqual(self.s.run_exe.read_bytes(), b"stand-in program")       # a copy runs
        self.assertEqual(self.starts()[0][2], str(self.s.path("window.ini")))   # jfgui_STATE
        self.assertTrue(self.s.on_change())
        self.assertTrue(wait_for(lambda: len(self.starts()) == 2))
        self.assertEqual(self.ended(), [first])                                 # asked to end, it ended
        self.assertNotEqual(self.starts()[1][0], first)
        self.assertEqual(self.s.path("build.txt").read_text(), "")
        self.assertFalse(self.s.path("ctl").exists())

    def test_failed_build_keeps_the_old_version(self):
        self.s.on_change()
        self.assertTrue(wait_for(lambda: len(self.starts()) == 1))
        proc = self.s.proc
        self.builds = [False]
        self.assertFalse(self.s.on_change())
        time.sleep(0.3)
        self.assertIs(self.s.proc, proc)
        self.assertIsNone(proc.poll())                                          # still running
        err = self.s.path("build.txt").read_text()
        self.assertIn("error[E0425]: cannot find value `undefined_value`", err)
        self.assertNotIn("build failed", err)
        self.assertIn("the running version stays", "\n".join(self.lines))
        self.assertTrue(self.s.on_change())                                     # fixed: restarted
        self.assertTrue(wait_for(lambda: len(self.starts()) == 2))
        self.assertEqual(self.s.path("build.txt").read_text(), "")

    def test_diverged_starts_afresh(self):
        self.s.args = ["diverge"]
        self.s.path("events.log").write_text("e 0 2 1 0 10 10 0 0 7\nb\n")
        self.s.on_change()
        self.assertTrue(wait_for(lambda: self.s.proc.poll() is not None))
        self.s.path("replay.txt").write_text("diverged: a press reached #8 instead of #7\n")
        self.s.args = ["normal"]
        self.assertEqual(self.s.check(), "diverged")
        self.assertFalse(self.s.path("events.log").exists())
        self.assertTrue(self.s.path("events.diverged.log").exists())
        self.assertTrue(wait_for(lambda: len(self.starts()) == 2))
        self.assertEqual(self.starts()[1][1], "normal")
        self.assertTrue(self.s.running())
        self.assertIn("a press reached #8 instead of #7", "\n".join(self.lines))

    def test_ended_by_itself(self):
        self.s.on_change()
        self.assertTrue(wait_for(lambda: len(self.starts()) == 1))
        self.s.path("events.log").write_text("x 10\n")
        self.s.path("ctl").write_text("quit\n")                                 # as if closed by the user
        self.assertTrue(wait_for(lambda: self.s.proc.poll() is not None))
        self.assertEqual(self.s.check(), "ended")
        self.assertTrue(self.s.path("events.ended.log").exists())
        self.assertIsNone(self.s.check())                                       # nothing runs now
        self.assertTrue(self.s.on_change())                                     # the next build starts it
        self.assertTrue(wait_for(lambda: len(self.starts()) == 2))

    def test_stubborn_program_killed(self):
        self.s.args = ["stubborn"]
        self.s.on_change()
        self.assertTrue(wait_for(lambda: len(self.starts()) == 1))
        t0 = time.time()
        code = self.s.stop(wait=0.5)
        self.assertLess(time.time() - t0, 5)
        self.assertNotEqual(code, 0)
        self.assertIn("killed", "\n".join(self.lines))
        self.assertFalse(self.s.running())

    def test_linux_environment(self):
        s = hotreload.Session(self.name, "linux", launcher=self.launch, builder=self.build, say=self.lines.append)
        try:
            env = s.env()
            if os.name == "nt":
                self.assertTrue(env["jfgui_HOTRELOAD"].startswith("/mnt/"))
                self.assertTrue(env["jfgui_STATE"].endswith(f"/build/linux/hot/{self.name}/window.ini"))
            self.assertEqual(s.run_exe.name, self.name + ".hot")
        finally:
            shutil.rmtree(s.dir, ignore_errors=True)

    def test_build_error_text(self):
        t = hotreload.build_error(ERROR)
        self.assertTrue(t.startswith("   Compiling"))
        self.assertIn("aborting due to 1 previous error", t)
        self.assertNotIn("build failed", t)

    def test_watch_files(self):
        rel = {p.relative_to(ROOT).as_posix() for p in hotreload.watch_files("gallery")}
        for f in ("apps/gallery/main.rs", "apps/gallery/sec_win.rs", "lib/ui/inspect.rs", "lib/ui/hotreload.rs",
                  "lib/plat/lib.rs", "lib/gfx/lib.rs", "styles/winui/lib.rs", "lib/text_shape/Cargo.lock"):
            self.assertIn(f, rel)
        self.assertFalse(any(r.startswith(("lib/img_jxl_enc/", "apps/PaintCraft/", "lib/netio/")) for r in rel))
        self.assertFalse(any("/tests/" in r for r in rel))
        rel = {p.relative_to(ROOT).as_posix() for p in hotreload.watch_files("themedemo")}
        self.assertIn("styles/kde/lib.rs", rel)                                 # a style it switches to

    def test_poller(self):
        a, b = self.tmp / "a.rs", self.tmp / "sub" / "b.rs"
        b.parent.mkdir()
        a.write_text("1")
        b.write_text("1")
        p = hotreload.Poller([a, b])
        self.assertEqual(p.changed(), [])
        a.write_text("22")
        self.assertEqual(p.changed(), [a])
        self.assertEqual(p.changed(), [])
        st = b.stat()
        os.utime(b, ns=(st.st_atime_ns, st.st_mtime_ns + 2_000_000_000))
        b.unlink()
        self.assertEqual(p.changed(), [b])                                      # removed
        b.write_text("3")
        a.write_text("333")
        c = self.tmp / "c.rs"
        c.write_text("1")
        p.track([a, b, c])                                                      # pending changes stay
        self.assertEqual(p.changed(), [a, b])
        c.write_text("22")
        self.assertEqual(p.changed(), [c])


APP = r'''// hrtest -- a temporary app of tools/tests/test_hotreload.py (removed by it)
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use ui::*;

const DESC: AppDesc = AppDesc { title: "Hot Reload", flags: AF_RESIZABLE, w: 480, h: 320, min_w: 200, min_h: 160, ..AppDesc::NONE };

#[derive(Default)]
struct T {
    page: i32,
    count: i32,
    name: String,
}

impl App for T {
    fn init(&mut self, ui: &mut Ui) -> bool {
        ui.styles_add(&[&style_kde::STYLE]);
        true
    }

    fn frame(&mut self, ui: &mut Ui) {
        ui.box_begin(0, BOX_V | pad(16), 8);
        ui.text(TS_BODY, "@LABEL@");
        @EXTRA@
        ui.segmented(10, &mut self.page, &["Eins", "Zwei"]);
        if self.page == 0 {
            if ui.button(20, "Mehr") & EV_ACT != 0 {
                self.count += 1;
            }
            ui.text(TS_BODY, &alloc::format!("Zahl: {}", self.count));
        } else {
            ui.textbox(30, Some("Name"), &mut self.name, 64);
        }
        ui.box_end();
    }
}

ui::main!(ui::run(&DESC, &mut T::default(), &style::STYLE));
'''

# the first run: what the next starts must bring back
S1 = """size 600 400
click "Mehr"
click "Mehr"
click "Mehr"
expect text:"Zahl: 3" exists
click "Zwei"
click #30
type "hallo"
wait 200
scale 150
style kde
inspect on
inspect select #30
inspect tab frame
inspect expect selected #30
"""

# after a change that keeps the widgets where they were: the replay brings the state back
S2 = """expect text:"Version B" exists
expect #30 text "hallo"
expect #30 focused
expect window 0 rect * * 600 400
expect style kde
inspect expect open
inspect expect selected #30
inspect expect selector "textbox:\\"hallo\\""
shot {dir}/b.png
click "Eins"
expect text:"Zahl: 3" exists
click "Mehr"
expect text:"Zahl: 4" exists
"""

# after a change that moves the widgets: the first start diverges (exit 3), the second is fresh
S3 = """expect text:"Version C" exists
expect text:"Zahl: 0" exists
expect #20 exists
expect window 0 rect * * 600 400
expect style kde
inspect expect open
"""

# the old program after a failed build: its inspector shows the error
S4 = """inspect expect open
window 2147422208
expect "Bau fehlgeschlagen" exists
shot {dir}/failed.png window 2147422208
"""


@unittest.skipIf("--quick" in sys.argv, "--quick: the watcher only")
class ReplayTest(unittest.TestCase):
    """a temporary app built and started in its session like the watcher does, headless"""

    @classmethod
    def setUpClass(cls):
        cls.name = f"hrtest{os.getpid()}"
        cls.app = ROOT / "apps" / cls.name
        cls.app.mkdir()

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.app, ignore_errors=True)

    def source(self, label, extra=""):
        (self.app / "main.rs").write_text(APP.replace("@LABEL@", label).replace("@EXTRA@", extra), encoding="utf-8")

    def run_app(self, s, script, text, change=False):
        """a start of the session under --script (change: through the watcher's build and
        restart) -> (exit code, output, replay.txt)"""
        f = self.tmp / script
        f.write_text(text.replace("{dir}", self.rel), encoding="utf-8")
        s.out.seek(0)
        s.out.truncate()
        args = ["--script", f"{self.rel}/{script}"]
        if change:
            self.assertTrue(s.on_change(args))
        else:
            s.start(args)
        s.proc.wait(timeout=120)
        s.release()
        code = s.proc.returncode
        s.out.seek(0)
        out = s.out.read().decode("utf-8", "replace")
        rep = s.path("replay.txt").read_text(encoding="utf-8").strip() if s.path("replay.txt").exists() else ""
        return code, out, rep

    def session(self, target):
        self.tmp = Path(tempfile.mkdtemp(prefix=f"hrtest-{target}-", dir=ROOT / "build"))
        self.rel = self.tmp.relative_to(ROOT).as_posix()
        self.addCleanup(shutil.rmtree, self.tmp, True)
        out = tempfile.TemporaryFile()
        self.addCleanup(out.close)
        lines = []
        # (every start of these tests is headless: --script, else --shot)
        s = hotreload.Session(self.name, target, args=["--shot", f"{self.rel}/default.png"], out=out, say=lines.append)
        shutil.rmtree(s.dir, ignore_errors=True)
        s.dir.mkdir(parents=True)
        self.addCleanup(shutil.rmtree, s.dir, True)
        return s, lines

    def check_target(self, target):
        s, lines = self.session(target)
        # A: the first start records
        self.source("Version A")
        ok, out = s.build()
        self.assertTrue(ok, out)
        s.install()
        code, out, rep = self.run_app(s, "s1.uis", S1)
        self.assertEqual((code, rep), (0, "fresh"), out)
        log = s.path("events.log").read_text()
        self.assertIn("\ne ", log)
        self.assertTrue(log.rstrip().splitlines()[-1].startswith("x "))
        carry = s.path("carry.ini").read_text()
        self.assertIn("style=kde", carry)
        self.assertIn("open=1", carry)
        # B: a new label; the replay brings back page, count, text, focus, size, scale, style
        # and the inspector with its selection
        self.source("Version B")
        code, out, rep = self.run_app(s, "s2.uis", S2, change=True)
        self.assertEqual(code, 0, out)
        self.assertRegex(rep, r"^replayed \d+ events$")
        png = (self.tmp / "b.png").read_bytes()
        self.assertEqual((int.from_bytes(png[16:20], "big"), int.from_bytes(png[20:24], "big")), (900, 600))   # 150 %
        # C: a button above the others: the recorded clicks reach other widgets -> diverged
        self.source("Version C", 'ui.button(40, "Neu");')
        ok, out = s.build()
        self.assertTrue(ok, out)
        s.install()
        code, out, rep = self.run_app(s, "s3.uis", S3)
        self.assertEqual(code, hotreload.EXIT_DIVERGED, out)
        self.assertRegex(rep, r"^diverged: a (press|release) reached #\d+ instead of #\d+$")
        self.assertIn("hot reload: the replay diverged", out)
        s.put_aside("diverged")                          # the watcher's check() does this, then starts again
        code, out, rep = self.run_app(s, "s3.uis", S3)
        self.assertEqual((code, rep), (0, "fresh"), out)
        # D: a failed build keeps the old program; its inspector shows the error
        self.source("Version D", "undefined_value;")
        self.assertFalse(s.on_change())
        err = s.path("build.txt").read_text()
        self.assertIn("undefined_value", err)
        code, out, rep = self.run_app(s, "s4.uis", S4)
        self.assertEqual(code, 0, out)
        # E: fixed: build.txt empty again
        self.source("Version E")
        ok, out = s.build()
        self.assertTrue(ok, out)
        self.assertEqual(s.path("build.txt").read_text(), "")

    def test_session(self):
        for t in targets():
            with self.subTest(target=t):
                self.check_target(t)


def targets():
    return [sys.argv[sys.argv.index("--target") + 1]] if "--target" in sys.argv else TARGETS


if os.name == "nt":
    import ctypes
    from ctypes import wintypes as W

    class STARTUPINFOW(ctypes.Structure):
        _fields_ = [("cb", W.DWORD), ("lpReserved", W.LPWSTR), ("lpDesktop", W.LPWSTR), ("lpTitle", W.LPWSTR),
                    ("dwX", W.DWORD), ("dwY", W.DWORD), ("dwXSize", W.DWORD), ("dwYSize", W.DWORD),
                    ("dwXCountChars", W.DWORD), ("dwYCountChars", W.DWORD), ("dwFillAttribute", W.DWORD),
                    ("dwFlags", W.DWORD), ("wShowWindow", W.WORD), ("cbReserved2", W.WORD), ("lpReserved2", ctypes.c_void_p),
                    ("hStdInput", W.HANDLE), ("hStdOutput", W.HANDLE), ("hStdError", W.HANDLE)]

    class PROCESS_INFORMATION(ctypes.Structure):
        _fields_ = [("hProcess", W.HANDLE), ("hThread", W.HANDLE), ("dwProcessId", W.DWORD), ("dwThreadId", W.DWORD)]

    class DesktopProc:
        """a program started on a HiddenDesktop, with Popen's poll / wait / kill / returncode"""

        def __init__(self, k32, pi):
            self.k32, self.h, self.pid, self.stdin, self.returncode = k32, pi.hProcess, pi.dwProcessId, None, None
            k32.CloseHandle(pi.hThread)

        def poll(self):
            if self.returncode is None and self.k32.WaitForSingleObject(self.h, 0) == 0:
                code = W.DWORD()
                self.k32.GetExitCodeProcess(self.h, ctypes.byref(code))
                self.returncode = code.value
            return self.returncode

        def wait(self, timeout=None):
            r = self.k32.WaitForSingleObject(self.h, 0xFFFFFFFF if timeout is None else int(timeout * 1000))
            if r != 0:
                raise subprocess.TimeoutExpired("desktop program", timeout)
            return self.poll()

        def kill(self):
            self.k32.TerminateProcess(self.h, 1)
            self.wait()

    class HiddenDesktop:
        """(Windows) a desktop of its own: the windows of the programs started on it never appear
        on the user's screen; messages posted to them stand in for the user's input"""

        def __init__(self):
            self.k32 = ctypes.WinDLL("kernel32", use_last_error=True)
            self.u32 = ctypes.WinDLL("user32", use_last_error=True)
            self.u32.CreateDesktopW.restype = W.HANDLE
            self.u32.CreateDesktopW.argtypes = [W.LPCWSTR, W.LPCWSTR, ctypes.c_void_p, W.DWORD, W.DWORD, ctypes.c_void_p]
            self.u32.EnumDesktopWindows.argtypes = [W.HANDLE, ctypes.c_void_p, W.LPARAM]
            self.u32.CloseDesktop.argtypes = [W.HANDLE]
            self.u32.PostMessageW.argtypes = [W.HWND, W.UINT, W.WPARAM, W.LPARAM]
            self.k32.WaitForSingleObject.argtypes = [W.HANDLE, W.DWORD]
            self.k32.GetExitCodeProcess.argtypes = [W.HANDLE, ctypes.c_void_p]
            self.k32.TerminateProcess.argtypes = [W.HANDLE, W.UINT]
            self.k32.CloseHandle.argtypes = [W.HANDLE]
            self.name = f"jfgui-hotreload-{os.getpid()}"
            self.h = self.u32.CreateDesktopW(self.name, None, None, 0, 0x10000000, None)    # GENERIC_ALL
            if not self.h:
                raise OSError(ctypes.get_last_error(), "CreateDesktopW")

        def close(self):
            self.u32.CloseDesktop(self.h)

        def launch(self, cmd, env, cwd):
            si = STARTUPINFOW()
            si.cb = ctypes.sizeof(si)
            si.lpDesktop = self.name
            pi = PROCESS_INFORMATION()
            line = ctypes.create_unicode_buffer(subprocess.list2cmdline([str(c) for c in cmd]))
            block = ctypes.create_unicode_buffer("\0".join(f"{k}={v}" for k, v in env.items()) + "\0")
            ok = self.k32.CreateProcessW(None, line, None, None, False, 0x400 | 0x08000000, block, str(cwd),
                                         ctypes.byref(si), ctypes.byref(pi))     # unicode environment, no window
            if not ok:
                raise OSError(ctypes.get_last_error(), "CreateProcessW")
            return DesktopProc(self.k32, pi)

        def windows(self, pid):
            """the visible top-level windows of the program -> [(hwnd, title)]"""
            out = []

            @ctypes.WINFUNCTYPE(W.BOOL, W.HWND, W.LPARAM)
            def each(hwnd, _):
                p = W.DWORD()
                self.u32.GetWindowThreadProcessId(hwnd, ctypes.byref(p))
                if p.value == pid and self.u32.IsWindowVisible(hwnd):
                    t = ctypes.create_unicode_buffer(256)
                    self.u32.GetWindowTextW(hwnd, t, 256)
                    out.append((hwnd, t.value))
                return True
            self.u32.EnumDesktopWindows(self.h, each, 0)
            return out

        def key(self, hwnd, vk):
            self.u32.PostMessageW(hwnd, 0x100, vk, 1)                                  # WM_KEYDOWN
            self.u32.PostMessageW(hwnd, 0x101, vk, 1 | 1 << 30 | 1 << 31)              # WM_KEYUP


@unittest.skipIf("--quick" in sys.argv or os.name != "nt" or "win" not in targets(), "the window test runs on Windows")
class WindowTest(unittest.TestCase):
    """the window loop (Windows) on a desktop of its own, never on the user's screen: the input
    recorded, the watcher's "quit" (window placement, log and carried state written), the replay
    of the next start, a failed build opening the inspector"""

    def test_window(self):
        name = f"hrwin{os.getpid()}"
        app = ROOT / "apps" / name
        app.mkdir()
        self.addCleanup(shutil.rmtree, app, True)
        (app / "main.rs").write_text(APP.replace("@LABEL@", "Fenster").replace("@EXTRA@", ""), encoding="utf-8")
        desk = HiddenDesktop()
        self.addCleanup(desk.close)
        s = hotreload.Session(name, "win", say=lambda m: None,
                              launcher=lambda env, args: desk.launch([s.run_exe] + args, dict(os.environ, **env), ROOT))
        shutil.rmtree(s.dir, ignore_errors=True)
        s.dir.mkdir(parents=True)
        self.addCleanup(shutil.rmtree, s.dir, True)
        self.addCleanup(lambda: s.proc and s.proc.poll() is None and s.proc.kill())
        ok, out = s.build()
        self.assertTrue(ok, out)
        s.install()

        def main_window():
            self.assertTrue(wait_for(lambda: any(t == "Hot Reload" for _, t in desk.windows(s.proc.pid)), 30),
                            desk.windows(s.proc.pid))
            return next(h for h, t in desk.windows(s.proc.pid) if t == "Hot Reload")

        def records():
            p = s.path("events.log")
            return [ln for ln in p.read_text().splitlines() if ln.startswith("e ")] if p.exists() else []

        # recording: Tab to the page switch, Tab to "Mehr", Space presses it
        s.start([])
        hwnd = main_window()
        for vk in (0x09, 0x09, 0x20):
            desk.key(hwnd, vk)
            time.sleep(0.2)
        self.assertTrue(wait_for(lambda: sum(r.split()[2] == "5" for r in records()) >= 3, 10), records())  # E_KEY_DOWN
        self.assertEqual(s.stop(), 0)                                           # "quit": a regular end
        log = s.path("events.log").read_text()
        self.assertTrue(log.rstrip().splitlines()[-1].startswith("x "), log)
        self.assertIn("[window]", s.path("window.ini").read_text())             # placement (winstate)
        self.assertIn("[inspector]", s.path("carry.ini").read_text())
        n = len(records())
        # the next start replays them, then a failed build opens the inspector
        s.start([])
        main_window()
        self.assertTrue(wait_for(lambda: s.path("replay.txt").read_text().strip() == f"replayed {n} events", 10),
                        s.path("replay.txt").read_text())
        s.path("build.txt").write_text("error: a test error\n")
        self.assertTrue(wait_for(lambda: any(t.startswith("Insp") for _, t in desk.windows(s.proc.pid)), 10),
                        desk.windows(s.proc.pid))
        self.assertEqual(s.stop(), 0)
        self.assertIn("open=0", s.path("carry.ini").read_text())                # opened for the error only


if __name__ == "__main__":
    argv = [a for a in sys.argv if a != "--quick"]
    if "--target" in argv:
        i = argv.index("--target")
        del argv[i:i + 2]
    unittest.main(argv=argv)
