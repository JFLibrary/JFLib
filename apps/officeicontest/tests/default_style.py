"""Verify that the opt-in colour icon hook leaves the ordinary Gallery unchanged."""
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT / "tools"))
import parity

parity.RS = (ROOT / (sys.argv[1] if len(sys.argv)>1 else "build/wordcraft-icons")).resolve()
parity.TMP = parity.RS / "default-style-parity"
assert parity.parity_app("gallery", ["win","linux"], scripts=False, matrix=True)
