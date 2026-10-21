"""bc_i18n.py -- the console texts of BackupCraft without an English translation.

    python tools/bc_i18n.py            list the German texts of t("...") / tf("...") in
                                       apps/BackupCraft/src that apps/BackupCraft/src/locales/ui.tsv
                                       lacks (exit code 1 when there are any)
    python tools/bc_i18n.py --unused   also the catalogue lines no source uses any more

Texts passed to t() through a variable (tables of &'static str) are found when they appear as a
string literal in a `t(` call or in a table marked by `// i18n` on the line before."""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
APP = ROOT / "apps" / "BackupCraft"
CALL = re.compile(r'\btf?\(\s*"((?:[^"\\]|\\.)*)"')
LIT = re.compile(r'"((?:[^"\\]|\\.)*)"')


def unescape(s):
    return s.encode("utf-8").decode("unicode_escape").encode("latin-1").decode("utf-8")


def texts():
    found = {}
    for f in sorted((APP / "src").rglob("*.rs")):
        lines = f.read_text(encoding="utf-8").splitlines()
        marked = False
        for no, ln in enumerate(lines, 1):
            for m in CALL.finditer(ln):
                found.setdefault(unescape(m.group(1)), f"{f.relative_to(ROOT)}:{no}")
            if marked:
                for m in LIT.finditer(ln):
                    found.setdefault(unescape(m.group(1)), f"{f.relative_to(ROOT)}:{no}")
                if ln.rstrip().endswith(("];", "},")) or not ln.strip():
                    marked = False
            if ln.strip() == "// i18n":
                marked = True
    return found


def catalogue():
    out = {}
    for ln in (APP / "src" / "locales" / "ui.tsv").read_text(encoding="utf-8").splitlines()[1:]:
        if "\t" in ln:
            de, en = ln.split("\t", 1)
            out[de] = en
    return out


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    src = texts()
    cat = catalogue()
    missing = [(k, v) for k, v in src.items() if k not in cat and re.search(r"[A-Za-zÄÖÜäöü]", k)]
    for k, where in missing:
        print(f"{where}\t{k}")
    if "--unused" in sys.argv:
        for k in cat:
            if k not in src:
                print(f"unused\t{k}")
    print(f"bc_i18n: {len(missing)} texts without English", file=sys.stderr)
    return 1 if missing else 0


if __name__ == "__main__":
    sys.exit(main())
