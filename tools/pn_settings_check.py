#!/usr/bin/env python3
"""pn_settings_check.py -- PaintCraft settings file (agent S1, M7; apps/PaintCraft/docs/SETTINGS.md).

Runs PaintCraft with a test configuration (--config DIR / $PaintCraft_CONFIG_DIR, always below
build/<out>/pn_settings_check/<target>/) and checks the settings.ini it writes and reads:

  roundtrip   process 1 changes every persisted value and ends with Beenden (Strg+Q = the
              save-on-exit path), the file is checked, recent files / a comment / an unknown
              section are added, process 2 checks every value, saves again, the additions
              must survive
  malformed   a hostile file (bad numbers, NaN, overflow, bad hex, overlong strings, unknown
              names): valid values are taken, invalid ones ignored, sizes clamped
  nocfg       headless without --config: nothing is read or written, the user's real
              configuration (%APPDATA%/PaintCraft, ~/.config/PaintCraft) is never created
  env         $PaintCraft_CONFIG_DIR instead of --config
  autoclose   (--window) --auto-close alone writes the settings (UIC_EXIT, U3)
  window      (--window) real windows: main window outside every monitor -> back on screen,
              tool window far away -> default place, maximized state + normal rectangle
              (Windows; Linux with the X11 backend)

  python tools/pn_settings_check.py [--target win|linux|both] [--out build/s1] [--window]
"""
import argparse
import os
import pathlib
import re
import shutil
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
WSL = ["wsl.exe", "-d", "Ubuntu-24.04", "--cd", str(ROOT), "-e"]


def rel(p):
    return str(pathlib.Path(p).resolve().relative_to(ROOT)).replace("\\", "/")


class Runner:
    def __init__(self, target, out):
        self.target = target
        self.out = out
        self.base = out / "pn_settings_check" / target
        self.fails = 0

    def exe(self):
        if self.target == "win":
            return [str(self.out / "win" / "PaintCraft.exe")]
        return ["./" + rel(self.out / "linux" / "PaintCraft")]

    def run(self, args, env=None, timeout=300, check=True):
        cmd = self.exe() + args
        if self.target == "linux":
            pre = ["env"] + [f"{k}={v}" for k, v in (env or {}).items()]
            cmd = WSL + pre + cmd
            full_env = None
        else:
            full_env = dict(os.environ)
            full_env.pop("PaintCraft_CONFIG_DIR", None)
            full_env.update(env or {})
        r = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, encoding="utf-8",
                           errors="replace", env=full_env, timeout=timeout)
        out = (r.stdout or "") + (r.stderr or "")
        if check and r.returncode != 0:
            self.fail(f"exit {r.returncode}: {' '.join(args)}\n" + "\n".join(out.strip().splitlines()[-20:]))
        return r.returncode, out

    def script(self, name, text, args=(), env=None):
        p = self.base / (name + ".uis")
        p.write_text(text, encoding="utf-8")
        return self.run(list(args) + ["--script", rel(p)], env=env)

    def fail(self, msg):
        self.fails += 1
        print(f"  FAIL [{self.target}] {msg}")

    def expect(self, cond, msg):
        if not cond:
            self.fail(msg)


def read_ini(path):
    """-> {section: {key: value}} (last value wins), plus the raw text"""
    d, sec = {}, None
    text = path.read_text(encoding="utf-8")
    for line in text.splitlines():
        s = line.strip()
        if s.startswith("[") and s.endswith("]"):
            sec = d.setdefault(s[1:-1], {})
        elif "=" in s and sec is not None and not s.startswith((";", "#")):
            k, v = s.split("=", 1)
            sec[k.strip()] = v.strip()
    return d, text


def check_keys(r, ini, want, what):
    for sec, kv in want.items():
        for k, v in kv.items():
            got = ini.get(sec, {}).get(k)
            r.expect(got == v, f"{what}: [{sec}] {k} = {got!r}, expected {v!r}")


# ------------------------------------------------------------------ round trip

WRITE = """size 1280 800
cmd 852
theme light
expect #5900 value 1
key alt+x
expect window 24624 open
click #5530
click listitem:"Dunkel"
click #5531
type "2D7D9A"
key tab
click #5533
expect #5905 value 0
click #5502
click #5540
click listitem:"Zoomen"
click #5503
click #5550
key ctrl+a
type "768"
key tab
click #5504
click #5560
key ctrl+a
type "2"
key tab
click "Schließen"
expect window 24624 closed
expect #5901 value 1
cmd 787
cmd 787
cmd 787
key alt+x
click #5502
click #5541
key esc
expect #5901 value 2
cmd 784
cmd 205
cmd 225
cmd 231 1
movewin 24577 320 180
resizewin 24579 240 280
close window 24578
expect window 24578 closed
expect #5908 text "{tc}"
cmd 107
"""

READ = """size 1280 800
expect #5902 value 1
expect #5903 value 1
expect #5903 text "Dunkel"
expect #5904 text "2D7D9A"
expect #5905 value 0
expect #5905 text "anim_off=1"
expect #5906 text "Zoom"
expect #5907 value 5
expect #5908 text "{tc}"
expect #5909 text "{tc}"
expect #5910 text "FFFFFFFF FF000000"
expect #5910 value 65
expect #5911 text "FF000000 FFFFFFFF"
expect #5912 value 1
expect #5912 text "grid=1 rulers=0"
expect #5913 value 768
expect #5914 value 2
expect #5914 text "budget=768"
expect #5915 value 2
expect #5915 text "C:/Bilder/erstes Bild.png"
expect window 24577 rect 320 180 112 346
expect window 24578 closed
expect window 24579 rect {lx} {ly} 240 280
expect window 24580 open
cmd 850
expect #5901 value 1
"""


def tc_after_three_steps(r):
    """the tool bar text after three `]` steps (the width steps come from tool.s)"""
    code, out = r.script("tc_probe", "size 1280 800\ncmd 852\ncmd 787\ncmd 787\ncmd 787\ndump\n",
                         ["--config", rel(r.base / "probe")])
    m = re.search(r'"id":5909,[^\n]*?"t":"([^"]*)"', out)
    return m.group(1) if m else None


def t_roundtrip(r):
    cfg = r.base / "rt"
    shutil.rmtree(cfg, ignore_errors=True)
    tc = tc_after_three_steps(r)
    r.expect(tc and tc.startswith("w="), f"tool bar probe: {tc!r}")
    tc = tc or ""
    r.script("rt_write", WRITE.replace("{tc}", tc), ["--config", rel(cfg)])
    ini_path = cfg / "settings.ini"
    if not ini_path.exists():
        r.fail("round trip: no settings.ini written")
        return
    ini, text = read_ini(ini_path)
    width = re.search(r"w=(\S+)", tc).group(1) if tc else "?"
    check_keys(r, ini, {
        "PaintCraft": {"version": "1"},
        "UI": {"theme": "dark", "accent": "2D7D9A", "animations": "0"},
        "Tools": {"default": "Zoom", "width": width, "hardness": "75", "pressure": "1"},
        "Colors": {"primary": "FFFFFFFF", "secondary": "FF000000"},
        "View": {"units": "inch", "grid": "1", "rulers": "0"},
        "ToolsWindow": {"visible": "1", "x": "320", "y": "180"},
        "HistoryWindow": {"visible": "0"},
        "LayersWindow": {"visible": "1", "width": "240", "height": "280"},
        "ColorsWindow": {"visible": "1", "expanded": "0"},
        "History": {"budget_mb": "768", "temp_dir": ""},
        "Performance": {"threads": "2"},
    }, "round trip file")
    pal = ini.get("Colors", {}).get("palette", "").split(",")
    r.expect(len(pal) == 65 and pal[-1] == "FFFFFFFF", f"palette: {len(pal)} colours, last {pal[-1:]}")
    r.expect("MainWindow" not in ini, "headless run wrote [MainWindow]")
    r.expect(not (cfg / "settings.ini.tmp~").exists(), "temp file left behind")
    lay = ini.get("LayersWindow", {})
    # recent files, a comment and an unknown section for process 2
    text = "; Kommentar (bleibt erhalten)\n" + text + \
        "\n[RecentFiles]\nfile1=C:/Bilder/erstes Bild.png\nfile2=/home/x/zweites.png\n" \
        "\n[Zukunft]\nschluessel = wert mit Leerzeichen\n"
    ini_path.write_text(text, encoding="utf-8")
    r.script("rt_read", READ.replace("{tc}", tc).replace("{lx}", lay.get("x", "?"))
             .replace("{ly}", lay.get("y", "?")), ["--config", rel(cfg)])
    ini2, text2 = read_ini(ini_path)
    r.expect(text2.startswith("; Kommentar (bleibt erhalten)"), "comment lost")
    check_keys(r, ini2, {
        "Zukunft": {"schluessel": "wert mit Leerzeichen"},
        "RecentFiles": {"file1": "C:/Bilder/erstes Bild.png", "file2": "/home/x/zweites.png"},
        "UI": {"theme": "dark"}, "Tools": {"default": "Zoom"},
    }, "second save")
    r.expect("file3" not in ini2.get("RecentFiles", {}), "extra recent file")


# ------------------------------------------------------------------ malformed input

BAD = """; Kommentar bleibt erhalten
garbage line without equals
[PaintCraft]
version=99
[UI]
theme=purple
accent=GGHHII
animations=7
[Colors]
primary=12345
secondary=FF00FF00
palette=FF112233, nothex, 00FF00 ,FFFFFFFFFF,
[View]
units=parsec
grid=2
rulers=1
[Tools]
default=Hammer
width=nan
hardness=101
spacing=0
antialias=0
blend=15
fill=53
tolerance=-1
flood=1
sampling=1
selection_mode=9
selection_quality=1
pressure=0
smoothing=1
[ToolsWindow]
visible=0
x=99999999999999999999
y=12
[HistoryWindow]
width=100000
height=5
x=-30000
y=40
[LayersWindow]
x=12abc
y=40
[History]
budget_mb=-5
temp_dir={long}
[Performance]
threads=65
[RecentFiles]
file1={longer}
file2=C:/ok.png
file3=
[Unknown]
key=value
"""

BAD_READ = """size 1280 800
expect #5902 value 1
expect #5903 value 2
expect #5904 value 0
expect #5905 value 1
expect #5906 text "Paintbrush"
expect #5907 value 10
expect #5908 text "w=2 h=75 s=15 aa=0 b=0 f=53 t=50 fl=1 smp=1 sel=0 q=1 p=0 sm=1"
expect #5909 text "w=2 h=75 s=15 aa=0 b=0 f=53 t=50 fl=1 smp=1 sel=0 q=1 p=0 sm=1"
expect #5910 text "FF000000 FF00FF00"
expect #5910 value 2
expect #5911 text "FF112233 FF00FF00"
expect #5912 value 0
expect #5912 text "grid=0 rulers=1"
expect #5913 value 0
expect #5913 text ""
expect #5914 value 0
expect #5915 value 1
expect #5915 text "C:/ok.png"
expect window 24577 closed
expect window 24578 open
cmd 850
expect #5901 value 1
"""


def t_malformed(r):
    cfg = r.base / "bad"
    shutil.rmtree(cfg, ignore_errors=True)
    cfg.mkdir(parents=True)
    (cfg / "settings.ini").write_text(BAD.replace("{long}", "a" * 2000).replace("{longer}", "b" * 1100),
                                      encoding="utf-8")
    r.script("bad_read", BAD_READ, ["--config", rel(cfg)])
    ini, text = read_ini(cfg / "settings.ini")
    r.expect(text.startswith("; Kommentar bleibt erhalten\ngarbage line without equals\n"),
             "malformed: comment / unknown line lost")
    check_keys(r, ini, {
        "PaintCraft": {"version": "1"},
        "UI": {"theme": "system", "accent": "", "animations": "1"},
        "Colors": {"primary": "FF000000", "secondary": "FF00FF00", "palette": "FF112233,FF00FF00"},
        "View": {"units": "pixel", "grid": "0", "rulers": "1"},
        "Tools": {"default": "Paintbrush", "width": "2", "hardness": "75", "spacing": "15",
                  "blend": "0", "fill": "53", "tolerance": "50", "selection_mode": "0"},
        "ToolsWindow": {"visible": "0"},
        "HistoryWindow": {"width": "4000", "height": "80"},
        "History": {"budget_mb": "0", "temp_dir": ""},
        "Performance": {"threads": "0"},
        "RecentFiles": {"file1": "C:/ok.png"},
        "Unknown": {"key": "value"},
    }, "malformed")
    for section, invalid in [("HistoryWindow", "-30000"), ("LayersWindow", "12abc")]:
        placement = ini.get(section, {})
        if "dock" in placement:
            # Files without an explicit placement mode use the automatic layout;
            # it deliberately removes stale absolute coordinates on save.
            r.expect(placement["dock"] == "0", f"{section}: expected automatic placement")
            r.expect("x" not in placement and "y" not in placement,
                     f"{section}: automatic placement kept stale coordinates")
        else:
            # Older reference builds persist absolute positions instead.
            r.expect(placement.get("x") not in (None, invalid), f"{section}: x not replaced")
    r.expect("file2" not in ini.get("RecentFiles", {}), "malformed: extra recent file")


# ------------------------------------------------------------------ no persistence / env

NOCFG = """size 1280 800
expect #5900 value 0
expect #5900 text ""
cmd 850
expect #5901 value 0
key alt+x
click "Schließen"
expect #5901 value 0
"""

ENV = """size 1280 800
expect #5900 value 1
cmd 850
expect #5901 value 1
"""


def user_config_exists(r):
    if r.target == "win":
        return (pathlib.Path(os.environ.get("APPDATA", "")) / "PaintCraft").exists()
    p = subprocess.run(WSL + ["sh", "-c", 'test -e "${XDG_CONFIG_HOME:-$HOME/.config}/PaintCraft" && echo yes || echo no'],
                       capture_output=True, text=True)
    return "yes" in p.stdout


def t_nocfg(r):
    before = user_config_exists(r)
    r.script("nocfg", NOCFG)
    r.expect(not user_config_exists(r) or before, "the user's configuration directory was created")
    r.expect(not before, "the user's configuration directory exists (not created by this test)")


def t_env(r):
    cfg = r.base / "env"
    shutil.rmtree(cfg, ignore_errors=True)
    r.script("env", ENV, env={"PaintCraft_CONFIG_DIR": rel(cfg)})
    r.expect((cfg / "settings.ini").exists(), "PaintCraft_CONFIG_DIR: no settings.ini")


# ------------------------------------------------------------------ real windows

def on_a_monitor(x, y):
    import ctypes
    return ctypes.windll.user32.MonitorFromPoint(ctypes.c_longlong((y << 32) | (x & 0xFFFFFFFF)), 0) != 0


def t_window(r):
    extra = {} if r.target == "win" else {"jfgui_BACKEND": "x11"}
    # 1) off-screen main window and tool window -> back on a monitor / default place
    cfg = r.base / "win1"
    shutil.rmtree(cfg, ignore_errors=True)
    cfg.mkdir(parents=True)
    (cfg / "settings.ini").write_text(
        "[PaintCraft]\nversion=1\n[MainWindow]\nx=-50000\ny=-50000\nwidth=1000\nheight=640\nmaximized=0\n"
        "[ToolsWindow]\nvisible=1\nx=-19000\ny=-19000\n", encoding="utf-8")
    r.run(["--config", rel(cfg), "--settings-save-at", "2500", "--auto-close", "3500"], env=extra, timeout=60)
    ini, _ = read_ini(cfg / "settings.ini")
    mw = ini.get("MainWindow", {})
    try:
        x, y = int(mw.get("x")), int(mw.get("y"))
    except (TypeError, ValueError):
        x = y = None
    if r.target == "win":
        r.expect(x is not None and x > -30000 and y > -30000, f"main window not moved back: {mw}")
        if x is not None:
            r.expect(on_a_monitor(x + 50, y + 20), f"main window client origin {x},{y} is on no monitor")
    check_keys(r, ini, {"MainWindow": {"width": "1000", "height": "640", "maximized": "0"}}, "window 1")
    tw = ini.get("ToolsWindow", {})
    if r.target == "win":
        r.expect(tw.get("x") == "8" and tw.get("y") == "140", f"tools window not at its default place: {tw}")
    else:       # the X11 window manager keeps windows on the screen itself
        r.expect(abs(int(tw.get("x", "-99999"))) < 5000 and abs(int(tw.get("y", "-99999"))) < 5000,
                 f"tools window still far away: {tw}")
    # 2) maximized: stays maximized, the normal rectangle is kept
    cfg = r.base / "win2"
    shutil.rmtree(cfg, ignore_errors=True)
    cfg.mkdir(parents=True)
    (cfg / "settings.ini").write_text(
        "[PaintCraft]\nversion=1\n[MainWindow]\nx=120\ny=90\nwidth=1100\nheight=700\nmaximized=1\n",
        encoding="utf-8")
    r.run(["--config", rel(cfg), "--settings-save-at", "2500", "--auto-close", "3500"], env=extra, timeout=60)
    ini, _ = read_ini(cfg / "settings.ini")
    check_keys(r, ini, {"MainWindow": {"maximized": "1", "width": "1100", "height": "700"}}, "window 2")
    if r.target == "win":
        check_keys(r, ini, {"MainWindow": {"x": "120", "y": "90"}}, "window 2")


def t_autoclose(r):
    """(U3) --auto-close ends a real window run through UIC_EXIT: the settings are written
    without --settings-save-at"""
    extra = {} if r.target == "win" else {"jfgui_BACKEND": "x11"}
    cfg = r.base / "autoclose"
    shutil.rmtree(cfg, ignore_errors=True)
    cfg.mkdir(parents=True)
    r.run(["--config", rel(cfg), "--auto-close", "2500"], env=extra, timeout=60)
    r.expect((cfg / "settings.ini").exists(), "--auto-close did not save the settings (UIC_EXIT)")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", default="both", choices=["win", "linux", "both"])
    ap.add_argument("--out", default="build/s1")
    ap.add_argument("--window", action="store_true", help="also the real-window checks")
    a = ap.parse_args()
    out = (ROOT / a.out).resolve()
    targets = ["win", "linux"] if a.target == "both" else [a.target]
    total = 0
    for t in targets:
        r = Runner(t, out)
        r.base.mkdir(parents=True, exist_ok=True)
        tests = [t_roundtrip, t_malformed, t_nocfg, t_env] + ([t_window, t_autoclose] if a.window else [])
        for fn in tests:
            n = r.fails
            fn(r)
            print(f"[{t}] {'PASS' if r.fails == n else 'FAIL'} {fn.__name__[2:]}")
        total += r.fails
    print("pn_settings_check:", "all passed" if total == 0 else f"{total} failures")
    return 1 if total else 0


if __name__ == "__main__":
    sys.exit(main())
