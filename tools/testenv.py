"""testenv.py -- shared environment for tools that start real app windows.

PaintCraft saves its settings when a real window closes (apps/PaintCraft/docs/SETTINGS.md).  Test
tools must never create the user's real configuration (%APPDATA%/PaintCraft, ~/.config/PaintCraft),
so importing this module points PaintCraft_CONFIG_DIR at build/test_cfg (unless it is already set)
and passes it on to WSL processes (WSLENV, path translated).
"""
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
os.environ.setdefault("PaintCraft_CONFIG_DIR", str(ROOT / "build" / "test_cfg"))
_wslenv = [e for e in os.environ.get("WSLENV", "").split(":") if e and not e.startswith("PaintCraft_CONFIG_DIR")]
os.environ["WSLENV"] = ":".join(_wslenv + ["PaintCraft_CONFIG_DIR/p"])


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
