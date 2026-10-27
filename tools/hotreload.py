"""hotreload -- `python build_rs.py watch NAME`: an app built again when a source was saved and
started again in its session (the runtime part: lib/ui/hotreload.rs; docs/GUIDE.md section 30)

  python build_rs.py watch NAME [--target win|linux] [--style S] [-- ARGS]

Watches the app's sources, its assets and build files and the sources of every library and
style crate it links (the modification times, polled: one os.scandir per folder, a few
milliseconds a round), builds a development build when files were saved (`build_rs.py app
NAME --target T`, incremental; never --release) and starts the app again:
  * the running app is asked to end (DIR/ctl "quit"; killed after STOP_WAIT s): it writes
    the end of its input log (DIR/events.log), its window placement (DIR/window.ini through
    jfgui_STATE) and the style, the text scale and the inspector (DIR/carry.ini)
  * the new build runs as a copy next to the build (NAME.hot[.exe]: the next build can write
    NAME[.exe] while it runs; its folder is the build's) with jfgui_HOTRELOAD=DIR: it replays
    the log -- the app's state comes back through the app's own code -- sets the carried
    state again and goes on recording
  * the replay diverged (an event reached another widget than when it was recorded: exit
    code 3): the log is put aside (events.diverged.log), the app starts afresh
  * the build failed: the old version keeps running; the error goes to the console and to
    DIR/build.txt, which the app shows at the top of its inspector (opened for it)
  * the app ended by itself (closed, crashed): its log is put aside, it starts afresh after
    the next change
DIR is build/<target>/hot/NAME; ARGS go to every start.  Ctrl+C ends watch and the app.
"""
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXIT_DIVERGED = 3           # lib/ui/hotreload.rs HR_EXIT_DIVERGED
POLL_S = 0.3                # a look at the files
QUIET_S = 0.3               # saving is over when nothing changed for this long
STOP_WAIT = 5.0             # the app's time to end when asked
DISTRO = os.environ.get("jfgui_WSL_DISTRO", "Ubuntu-24.04")


def linux_path(p):
    p = str(Path(p).resolve())
    return "/mnt/" + p[0].lower() + p[2:].replace("\\", "/") if p[1:2] == ":" else p


def build_module():
    if str(ROOT) not in sys.path:
        sys.path.insert(0, str(ROOT))
    import build_rs
    return build_rs


def crate_roots():
    """every crate an app can link -> {name: entry file}: LIBS, the styles (style_NAME), the
    app-local crates (apps/*/crates/C)"""
    b = build_module()
    from tools.project_sources import entrypoint, resolve_root
    roots = {n: resolve_root(ROOT / r) for n, (r, _) in b.LIBS.items()}
    roots.update({f"style_{s}": entrypoint(ROOT / "styles" / s, "lib") for s in b.STYLES})
    for c in sorted((ROOT / "apps").glob("*/crates/*")):
        r = entrypoint(c, "lib", False)
        if r is not None:
            roots.setdefault(c.name, r)
    return roots


def named(sources, names):
    """the crates of `names` the sources name (NAME:: or extern crate NAME): one look at the
    text (build_rs.named_crates searches crate by crate)"""
    src = "\n".join(f.read_text(encoding="utf-8", errors="replace") for f in sources if f.suffix == ".rs")
    used = set(re.findall(r"\b([A-Za-z_]\w*)::", src)) | set(re.findall(r"\bextern crate ([A-Za-z_]\w*)", src))
    return used & set(names)


def watch_files(name, style=None):
    """the files a development build of apps/NAME reads: the sources of the app and of its
    further programs (BINS), its assets and build files, the sources of every library and
    style crate it links (and of theirs) -> sorted list of Paths"""
    b = build_module()
    from tools.project_sources import entrypoint, project_sources
    app = ROOT / "apps" / name
    files = set(project_sources(entrypoint(app, "app"), assets=True))
    for _, root, _ in b.app_bins(name):
        files.update(project_sources(root, assets=True))
    files.update(p for p in app.iterdir() if p.is_file() and (p.name in ("PERMISSIONS", "PACKAGE", "STYLE", "BINS", "TARGETS")
                                                               or p.name.startswith("PERMISSIONS.")))
    for d in ("assets", "res"):
        if (app / d).is_dir():
            files.update(p for p in (app / d).rglob("*") if p.is_file())
    roots = crate_roots()
    todo = list(named(files, roots) | {f"style_{style or b.app_style(name)}"})
    need = set()
    while todo:
        c = todo.pop()
        if c in need or c not in roots:
            continue
        need.add(c)
        src = project_sources(roots[c], assets=True)
        files.update(src)
        todo.extend(b.LIBS[c][1] if c in b.LIBS else named(src, roots))
        if c.startswith("style_"):
            files.update(p for p in (ROOT / "styles" / c[len("style_"):] / "assets").glob("*") if p.is_file())
    if "text_shape" in need:
        files.update(ROOT / "lib" / "text_shape" / f for f in ("Cargo.toml", "Cargo.lock"))
    return sorted(Path(f).resolve() for f in files)


class Poller:
    """the modification times and sizes of a file set: one os.scandir per folder (on Windows
    the listing carries them: no call per file)"""

    def __init__(self, files):
        self.state = {}
        self.track(files)

    def track(self, files):
        """watch these files (from now on); files no longer in the set are dropped, the state
        of the others stays (a change since the last look is still reported)"""
        files = {Path(f) for f in files}
        self.dirs = {}
        for f in files:
            self.dirs.setdefault(f.parent, set()).add(f.name)
        now = self.scan()
        self.state = {f: self.state.get(f, now.get(f)) for f in files}

    def scan(self):
        out = {}
        for d, names in self.dirs.items():
            try:
                with os.scandir(d) as it:
                    for e in it:
                        if e.name in names:
                            try:
                                st = e.stat()
                            except OSError:
                                continue
                            out[d / e.name] = (st.st_mtime_ns, st.st_size)
            except OSError:
                pass
        return out

    def changed(self):
        """the files changed, created or removed since the last call"""
        now = self.scan()
        ch = sorted(f for f in self.state if now.get(f) != self.state[f])
        for f in ch:
            self.state[f] = now.get(f)
        return ch


def build_error(out):
    """the part of a failed build's output to show: without the summary lines"""
    keep = [ln for ln in out.replace("\r", "").splitlines() if ln.strip() not in ("build failed",)
            and not ln.startswith(("done (", "warning: rustc "))]
    while keep and not keep[-1].strip():
        keep.pop()
    return "\n".join(keep[:400])


class Session:
    """one app's session: its folder (build/<target>/hot/NAME), the running program, the
    build.  launcher(env, args) -> Popen and builder() -> (ok, output) replace the real ones
    in tests."""

    def __init__(self, name, target="win", style=None, args=(), launcher=None, builder=None, out=None, say=print):
        self.name, self.target, self.style, self.args = name, target, style, list(args)
        exe = name + (".exe" if target == "win" else "")
        self.exe = ROOT / "build" / target / exe
        self.run_exe = self.exe.with_name(name + ".hot" + (".exe" if target == "win" else ""))
        self.dir = ROOT / "build" / target / "hot" / name
        self.dir.mkdir(parents=True, exist_ok=True)
        self.launcher = launcher or self._launch
        self.builder = builder or self._build
        self.out = out                  # the app's output (None: this console)
        self.say = say
        self.proc = None
        self.replay_seen = None

    # ------------------------------------------------------------ files
    def path(self, n):
        return self.dir / n

    def set_build(self, ok, out):
        """build.txt: empty after a build that worked, else its error (the app shows it)"""
        self.path("build.txt").write_text("" if ok else build_error(out) + "\n", encoding="utf-8")

    def put_aside(self, why):
        """the session's log out of the way (the next start is fresh)"""
        log = self.path("events.log")
        if log.exists():
            os.replace(log, self.path(f"events.{why}.log"))

    def replay_report(self):
        """replay.txt when it changed since the last call, else None"""
        p = self.path("replay.txt")
        try:
            st = p.stat()
            key = (st.st_mtime_ns, st.st_size)
        except OSError:
            return None
        if key == self.replay_seen:
            return None
        self.replay_seen = key
        return p.read_text(encoding="utf-8", errors="replace").strip()

    # ------------------------------------------------------------ build
    def _build(self):
        cmd = [sys.executable, str(ROOT / "build_rs.py"), "app", self.name, "--target", self.target]
        if self.style:
            cmd += ["--style", self.style]
        r = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, encoding="utf-8", errors="replace",
                           stdin=subprocess.DEVNULL)
        return r.returncode == 0 and self.exe.exists(), r.stdout + r.stderr

    def build(self):
        """a development build -> (ok, output); build.txt follows it"""
        t0 = time.perf_counter()
        ok, out = self.builder()
        self.set_build(ok, out)
        self.say(f"watch: build {'done' if ok else 'FAILED'} ({time.perf_counter() - t0:.1f} s)")
        if not ok:
            self.say(build_error(out))
        return ok, out

    # ------------------------------------------------------------ the app
    def env(self):
        conv = linux_path if self.target == "linux" and os.name == "nt" else str
        return {"jfgui_HOTRELOAD": conv(self.dir), "jfgui_STATE": conv(self.path("window.ini"))}

    def _launch(self, env, args):
        kw = dict(cwd=ROOT, stdout=self.out, stderr=subprocess.STDOUT if self.out else None)
        if self.target == "linux" and os.name == "nt":
            cmd = ["wsl.exe", "-d", DISTRO, "-e", "python3", linux_path(ROOT / "tools" / "linux_task.py"), "--watch-stdin",
                   "--cwd", linux_path(ROOT), "run", "env"] + [f"{k}={v}" for k, v in env.items()] + [linux_path(self.run_exe)] + args
            return subprocess.Popen(cmd, stdin=subprocess.PIPE, creationflags=subprocess.CREATE_NO_WINDOW, **kw)
        return subprocess.Popen([str(self.run_exe)] + args, env=dict(os.environ, **env), stdin=subprocess.DEVNULL, **kw)

    def install(self):
        """the build copied to NAME.hot[.exe] (the old program's file may stay locked a moment
        after it ended)"""
        for _ in range(50):
            try:
                shutil.copy2(self.exe, self.run_exe)
                return
            except PermissionError:
                time.sleep(0.1)
        shutil.copy2(self.exe, self.run_exe)

    def running(self):
        return self.proc is not None and self.proc.poll() is None

    def start(self, args=None):
        """the installed build started in the session"""
        self.path("ctl").unlink(missing_ok=True)
        self.release()
        self.proc = self.launcher(self.env(), list(self.args if args is None else args))

    def release(self):
        """the ended program's pipe closed"""
        if self.proc is not None and self.proc.poll() is not None and self.proc.stdin:
            self.proc.stdin.close()

    def stop(self, wait=STOP_WAIT):
        """ask the app to end, kill it after `wait` s -> its exit code (None: none ran)"""
        if self.proc is None:
            return None
        if self.proc.poll() is None:
            self.path("ctl").write_text("quit\n", encoding="utf-8")
            try:
                self.proc.wait(timeout=wait)
            except subprocess.TimeoutExpired:
                self.say("watch: the app did not end; killed")
                self.kill()
        code = self.proc.returncode
        self.release()
        self.proc = None
        self.path("ctl").unlink(missing_ok=True)
        return code

    def kill(self):
        if self.proc.stdin:
            try:
                self.proc.stdin.close()     # WSL: linux_task.py ends the program's group
                self.proc.wait(timeout=5)
            except (OSError, subprocess.TimeoutExpired):
                pass
        if self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait()

    def restart(self, args=None):
        """the old program ended (asked), the new build installed and started"""
        self.stop()
        self.install()
        self.start(args)

    def check(self, args=None):
        """the app ended by itself? -> what happened: None (runs / none ran), "diverged" (it was
        started again afresh), "ended" (its log is put aside; it starts after the next build)"""
        if self.proc is None or self.proc.poll() is None:
            return None
        code = self.proc.returncode
        self.release()
        self.proc = None
        if code == EXIT_DIVERGED:
            self.say(f"watch: the replay {self.replay_report() or 'diverged'}; the app starts afresh")
            self.put_aside("diverged")
            self.start(args)
            return "diverged"
        self.say(f"watch: the app ended (exit code {code}); it starts afresh after the next change")
        self.put_aside("ended")
        return "ended"

    def on_change(self, args=None):
        """a build; it worked: the app started again in its session (or afresh when none ran);
        it failed: the old version keeps running -> the build's success"""
        ok, _ = self.build()
        if ok:
            self.restart(args)
        elif self.running():
            self.say("watch: the running version stays (the error is in its inspector)")
        return ok


def watch(name, target="win", style=None, args=()):
    """the watch loop of `build_rs.py watch` -> exit code"""
    if not (ROOT / "apps" / name).is_dir():
        print(f"watch: no app {name}")
        return 1
    try:
        files = watch_files(name, style)
    except Exception as e:  # noqa: BLE001 -- a source error of the project (build_rs.py names it)
        print(f"watch: {e}")
        return 1
    s = Session(name, target, style, args)
    poll = Poller(files)
    print(f"watch: {name} [{target}], {len(files)} files, session {s.dir.relative_to(ROOT)}; Ctrl+C ends")
    s.put_aside("old")                  # a new session: no replay of an earlier one
    if s.build()[0]:
        s.install()
        s.start()
    try:
        while True:
            time.sleep(POLL_S)
            s.check()
            rep = s.replay_report()
            if rep:
                print(f"watch: start: {rep}")
            ch = poll.changed()
            if not ch:
                continue
            while True:             # until the saving is over
                time.sleep(QUIET_S)
                more = poll.changed()
                if not more:
                    break
                ch += more
            shown = ", ".join(sorted({p.name for p in ch})[:6]) + (" ..." if len(ch) > 6 else "")
            print(f"watch: changed {shown}")
            s.on_change()
            try:
                poll.track(watch_files(name, style))    # new modules, other crates
            except Exception as e:  # noqa: BLE001
                print(f"watch: {e}")
    except KeyboardInterrupt:
        print("watch: ending")
        s.stop()
        return 0
