"""check_lang.py -- JFDiagrammer's language check (FEATURES.md 3.17: LG-06, LG-08, LG-11)

    python apps/JFDiagrammer/check_lang.py [--strict]

What it checks
  catalogue   tools/i18n_check.py for the app: every text of the sources has a row, every
              row all five cells (de en es it fr), placeholders and plural forms the same
  cells       every catalogue below locales/: the five columns, no empty cell
  glossary    lang/glossary.tsv: five filled cells per term; a catalogue row whose German
              text has a term should carry the term of each other language (a warning: the
              word may be inflected or the sentence built differently)
  forbidden   the glossary's forbidden terms (Visio's own) in no catalogue cell of their
              language and in no German source text
  shortcuts   lang/shortcuts.tsv: every command a command of src/commands.rs, every key a
              valid chord, no two commands on one chord in a language

Exit code 1 for an error (with --strict also for a warning), else 0.
"""
import re
import subprocess
import sys
from pathlib import Path

APP = Path(__file__).resolve().parent
ROOT = APP.parent.parent
LANGS = ["de", "en", "es", "it", "fr"]
KEYS = {"Ctrl", "Shift", "Alt"}
NAMED = {"PgUp", "PgDn", "Del", "Ins", "Home", "End", "Enter", "Left", "Right", "Up", "Down", "=", "<", ">"}


def rows(path):
    """(line number, cells) of a TSV file's data rows (comments and blank lines left out)"""
    out = []
    for n, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip() or line.startswith("#"):
            continue
        out.append((n, line.split("\t")))
    return out


def chord_ok(c):
    parts = c.split("+")
    key = [p for p in parts if p not in KEYS]
    if len(key) != 1 or len(set(parts)) != len(parts):
        return False
    k = key[0]
    return (len(k) == 1 and k.isalnum()) or k in NAMED or bool(re.fullmatch(r"F([1-9]|1[0-9]|2[0-4])", k))


def norm(c):
    """a chord with its modifiers in one order"""
    parts = c.split("+")
    mods = sorted(p for p in parts if p in KEYS)
    return "+".join(mods + [p for p in parts if p not in KEYS])


def stem(word):
    w = word.lower()
    return w[:-1] if len(w) > 4 else w


def main():
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    strict = "--strict" in sys.argv[1:]
    errors, warnings = [], []

    # catalogue against the sources (the repository's check)
    r = subprocess.run([sys.executable, str(ROOT / "tools" / "i18n_check.py"), "JFDiagrammer"],
                       capture_output=True, text=True, encoding="utf-8", cwd=ROOT)
    summary = [l for l in r.stdout.splitlines() if l.startswith("JFDiagrammer:")]
    print(summary[0] if summary else r.stdout.strip())
    m = re.search(r"(\d+) missing, (\d+) untranslated, (\d+) unused, (\d+) errors", r.stdout)
    if not m:
        errors.append("i18n_check: no summary")
    else:
        missing, untranslated, unused, errs = map(int, m.groups())
        if missing or untranslated or errs:
            errors.append(f"i18n_check: {missing} missing, {untranslated} untranslated, {errs} errors")
            print("\n".join(l for l in r.stdout.splitlines()[1:] if "unused" not in l))
        if unused:
            warnings.append(f"i18n_check: {unused} unused rows")

    # the catalogues' cells
    catalogue = []
    for tsv in sorted((APP / "locales").glob("*.tsv")):
        lines = tsv.read_text(encoding="utf-8").splitlines()
        if not lines or lines[0].split("\t") != LANGS:
            errors.append(f"{tsv.name}: the first line must be {' '.join(LANGS)}")
            continue
        for n, cells in rows(tsv)[1:]:
            if len(cells) != len(LANGS) or any(not c.strip() for c in cells):
                errors.append(f"{tsv.name}:{n}: {len(LANGS)} filled cells expected: {cells[0]}")
            catalogue.append((tsv.name, n, cells))

    # the glossary
    terms, forbidden = [], []
    for n, cells in rows(APP / "lang" / "glossary.tsv"):
        if cells[0] == "!":
            if len(cells) != 3:
                errors.append(f"glossary.tsv:{n}: a forbidden term is !, language, term")
            else:
                forbidden.append((cells[1], cells[2]))
            continue
        if len(cells) != 1 + len(LANGS) or any(not c.strip() for c in cells):
            errors.append(f"glossary.tsv:{n}: the key and {len(LANGS)} filled cells expected")
            continue
        terms.append(cells)
    for name, n, cells in catalogue:
        src = cells[0].split("|")[0]
        for term in terms:
            de = term[1]
            if not re.search(r"(?i)" + re.escape(stem(de)), src):
                continue
            for k, lang in enumerate(LANGS[1:], 1):
                if k >= len(cells):
                    continue
                want = term[1 + k]
                if not re.search(re.escape(stem(want)), cells[k], re.I):
                    warnings.append(f"{name}:{n}: '{src}' has '{de}', its {lang} text '{cells[k]}' lacks '{want}'")
    for lang, term in forbidden:
        pat = re.compile(r"(?i)(?<![\w-])" + re.escape(term) + r"(?![\w])")
        for name, n, cells in catalogue:
            for k, l in enumerate(LANGS):
                if (lang == "*" or lang == l) and k < len(cells) and pat.search(cells[k]):
                    errors.append(f"{name}:{n}: the forbidden term '{term}' in the {l} text '{cells[k]}'")

    # the shortcuts
    src = (APP / "src" / "commands.rs").read_text(encoding="utf-8")
    names = set(re.findall(r"^\s+([A-Z][A-Z0-9_]*) = 0x[0-9A-Fa-f]+,", src, re.M))
    seen = {l: {} for l in LANGS}
    for n, cells in rows(APP / "lang" / "shortcuts.tsv"):
        if len(cells) != 1 + len(LANGS):
            errors.append(f"shortcuts.tsv:{n}: the command and {len(LANGS)} cells expected")
            continue
        if cells[0] not in names:
            errors.append(f"shortcuts.tsv:{n}: no command {cells[0]} in src/commands.rs")
        for k, lang in enumerate(LANGS):
            for chord in filter(None, (c.strip() for c in cells[1 + k].split("|"))):
                if not chord_ok(chord):
                    errors.append(f"shortcuts.tsv:{n}: {lang}: not a chord: {chord}")
                    continue
                key = norm(chord)
                other = seen[lang].get(key)
                if other and other != cells[0]:
                    errors.append(f"shortcuts.tsv:{n}: {lang}: {chord} is {other} and {cells[0]}")
                seen[lang][key] = cells[0]

    print(f"glossary: {len(terms)} terms, {len(forbidden)} forbidden; shortcuts: "
          + ", ".join(f"{l} {len(seen[l])}" for l in LANGS))
    for w in warnings:
        print("warning:", w)
    for e in errors:
        print("error:", e)
    print(f"{len(errors)} errors, {len(warnings)} warnings")
    return 1 if errors or (strict and warnings) else 0


if __name__ == "__main__":
    sys.exit(main())
