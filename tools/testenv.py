"""testenv.py -- shared environment for tools that start real app windows.

Apps with a catalogue follow the system's languages in a real window, the tools look for German
texts: importing this module sets jfgui_LANG=de (unless it is already set) and passes it on to WSL
processes (WSLENV).  (It also kept PaintCraft's settings in build/test_cfg through
PaintCraft_CONFIG_DIR, until the app moved to its own repository, JFLibrary/JFPainter, in 2026-11.)
"""
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
os.environ.setdefault("jfgui_LANG", "de")
_wslenv = [e for e in os.environ.get("WSLENV", "").split(":") if e and not e.startswith("jfgui_LANG")]
os.environ["WSLENV"] = ":".join(_wslenv + ["jfgui_LANG"])


def build_cmd(app, target, out):
    """the command that builds APP for TARGET into OUT: the frozen assembly (legacy/build.py) for
    reference folders (legacy/..., build/rwref*), the Rust build (build_rs.py) for every other
    folder -- build/ holds the Rust build since R13"""
    import sys
    out = Path(out).resolve()
    legacy = ROOT.parent / "legacy"
    ref = out.is_relative_to(legacy) or (out.is_relative_to(ROOT / "build") and out != ROOT / "build"
                                         and out.relative_to(ROOT / "build").parts[0].startswith("rwref"))
    if ref:
        return [sys.executable, str(legacy / "build.py"), "app", app, "--target", target, "--out", str(out)]
    return [sys.executable, str(ROOT / "build_rs.py"), "app", app, "--target", target, "--out", str(out)]
