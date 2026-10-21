"""i18n_check.py -- the texts of an app that its translation catalogues lack.

    python tools/i18n_check.py apps/NAME                list the texts of the app's sources
                                                        that no catalogue src/locales/*.tsv has
                                                        (exit code 1 when there are any, or
                                                        other problems)
    python tools/i18n_check.py apps/NAME --unused       also the catalogue rows no source uses
    python tools/i18n_check.py apps/NAME crates [src]   only these folders of the app (default:
                                                        src and crates)

A text of the sources (*.rs; files named *test.rs are left out) is
  t("…") / tf("…")              a text the app translates (ui::i18n::t, its tr.rs)
  msg!("…", …), Msg::new("…", …), phrase("…")
                                a template of a message (jfn_model::msg)
  // i18n                       on a line of its own: every string literal of the block below
                                it (up to a line that is less indented, an empty line or a
                                line ending in "];"), for tables of names and match arms
  // i18n: name title           the same, only the literals after "name:" or "title:"
  … // i18n                     at the end of a line: the literals of that line
Only texts with a letter count.  A catalogue names its languages in its first line
("de\ten"), then has one row per text: the source text, a tab, the translations; lines that
start with "#" are comments.

Checks: every text has a row with every translation; the translations of a row use the same
arguments as its source ("{}" in order, "{0}".."{9}" by number: the same set of argument
numbers); a template of a message has no formats ("{:03}"), named placeholders ("{host}"), tabs
or line breaks; a text that two catalogues have is translated the same in both."""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CALL = re.compile(r"(?:\btf?|\bmsg!|\bphrase|\bMsg::new)\s*\(\s*$")
FIELD = re.compile(r"\b(\w+)\s*:\s*$")
LETTER = re.compile(r"[^\W\d_]")


def unescape(s):
    """the value of a Rust string literal's body"""
    out = []
    i = 0
    simple = {"n": "\n", "t": "\t", "r": "\r", "0": "\0", "\\": "\\", '"': '"', "'": "'"}
    while i < len(s):
        c = s[i]
        if c != "\\" or i + 1 >= len(s):
            out.append(c)
            i += 1
            continue
        n = s[i + 1]
        if n in simple:
            out.append(simple[n])
            i += 2
        elif n == "u":
            m = re.match(r"\{([0-9a-fA-F_]+)\}", s[i + 2:])
            if not m:
                out.append(n)
                i += 2
                continue
            out.append(chr(int(m.group(1).replace("_", ""), 16)))
            i += 2 + m.end()
        elif n == "x":
            out.append(chr(int(s[i + 2:i + 4], 16)))
            i += 4
        elif n == "\n":
            i += 2
            while i < len(s) and s[i] in " \t\r\n":
                i += 1
        else:
            out.append(n)
            i += 2
    return "".join(out)


def lex(text):
    """(code, strings): the text with comments and char literals blanked (offsets stay), and
    every string literal as (start offset, value); byte strings are left out"""
    code = list(text)
    strings = []
    i, n = 0, len(text)

    def blank(a, b):
        for k in range(a, b):
            if code[k] != "\n":
                code[k] = " "

    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            blank(i, j)
            i = j
        elif text.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif text.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            blank(i, j)
            i = j
        elif c == "r" and re.match(r'r#*"', text[i:]) and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            m = re.match(r'r(#*)"', text[i:])
            end = text.find('"' + m.group(1), i + m.end())
            end = n if end < 0 else end
            if i == 0 or text[i - 1] != "b":
                strings.append((i, text[i + m.end():end]))
            i = end + 1 + len(m.group(1))
        elif c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            if i == 0 or text[i - 1] != "b":
                strings.append((i, unescape(text[i + 1:j])))
            i = j + 1
        elif c == "'":
            m = re.match(r"'(\\u\{[0-9a-fA-F_]+\}|\\x[0-9a-fA-F]{2}|\\.|[^\\'\n])'", text[i:])
            if m:
                blank(i, i + m.end())
                i += m.end()
            else:
                i += 1  # a lifetime
        else:
            i += 1
    return "".join(code), strings


def args_of(t):
    """(argument numbers a template uses, problems): "{}" in order, "{N}" by number"""
    used, bad, nxt, i = set(), [], 0, 0
    while i < len(t):
        if t.startswith("{{", i) or t.startswith("}}", i):
            i += 2
        elif t[i] == "{":
            j = t.find("}", i)
            if j < 0:
                bad.append("an open brace")
                break
            inner = t[i + 1:j]
            if inner == "":
                used.add(nxt)
                nxt += 1
            elif inner.isdigit():
                used.add(int(inner))
            else:
                bad.append("{" + inner + "}")
            i = j + 1
        else:
            i += 1
    return used, bad


def sources(app, folders):
    """{text: (where, kind)} of every text the sources name; kind "msg" for message templates"""
    found = {}
    for d in folders:
        for f in sorted((app / d).rglob("*.rs")):
            if f.name.endswith("test.rs"):
                continue
            text = f.read_text(encoding="utf-8").replace("\r\n", "\n")
            code, strings = lex(text)
            starts = [0]
            for k, ch in enumerate(text):
                if ch == "\n":
                    starts.append(k + 1)
            lines = text.split("\n")
            # the lines marked by "// i18n" (line number -> the fields that count, None: all)
            marked = {}
            k = 0
            while k < len(lines):
                ln = lines[k]
                s = ln.strip()
                m = re.fullmatch(r"// i18n(?::\s*(.*))?", s)
                if m:
                    fields = m.group(1).split() if m.group(1) else None
                    ind = len(ln) - len(ln.lstrip())
                    j = k + 1
                    while j < len(lines):
                        nxt = lines[j]
                        if not nxt.strip() or len(nxt) - len(nxt.lstrip()) < ind:
                            break
                        marked[j] = fields
                        if nxt.rstrip().endswith("];"):
                            break
                        j += 1
                    k = j
                    continue
                if s.endswith("// i18n") and not s.startswith("//"):
                    marked[k] = None
                k += 1
            rel = f.relative_to(ROOT).as_posix()
            for off, val in strings:
                line = next_line(starts, off)
                before = code[max(0, off - 60):off]
                kind = None
                if CALL.search(before):
                    kind = "msg" if re.search(r"(?:msg!|phrase|Msg::new)\s*\(\s*$", before) else "t"
                elif line in marked:
                    fields = marked[line]
                    lstart = starts[line]
                    fm = FIELD.search(code[lstart:off])
                    if fields is None or (fm and fm.group(1) in fields):
                        kind = "table"
                if kind and LETTER.search(val):
                    found.setdefault(val, (f"{rel}:{line + 1}", kind))
    return found


def next_line(starts, off):
    lo, hi = 0, len(starts) - 1
    while lo < hi:
        mid = (lo + hi + 1) // 2
        if starts[mid] <= off:
            lo = mid
        else:
            hi = mid - 1
    return lo


def catalogues(app):
    """(rows {source: (where, translations)}, problems)"""
    rows, problems = {}, []
    for f in sorted((app / "src" / "locales").glob("*.tsv")):
        rel = f.relative_to(ROOT).as_posix()
        lines = f.read_text(encoding="utf-8").replace("\r\n", "\n").split("\n")
        langs = lines[0].split("\t") if lines else []
        for no, ln in enumerate(lines[1:], 2):
            if not ln or ln.startswith("#"):
                continue
            cols = ln.split("\t")
            src, tr = cols[0], cols[1:]
            where = f"{rel}:{no}"
            if len(tr) < len(langs) - 1 or any(not x for x in tr):
                problems.append(f"{where}\tnot translated into every language: {src}")
            sa, sbad = args_of(src)
            for x in tr:
                xa, xbad = args_of(x)
                if xa != sa or xbad != sbad:
                    problems.append(f"{where}\tplaceholders differ: {src}  |  {x}")
            if src in rows:
                if rows[src][1] != tr:
                    problems.append(f"{where}\ttranslated otherwise in {rows[src][0]}: {src}")
                elif rows[src][0].split(":")[0] == rel:
                    problems.append(f"{where}\ttwice (also {rows[src][0]}): {src}")
                continue
            rows[src] = (where, tr)
    return rows, problems


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    sys.stderr.reconfigure(encoding="utf-8")
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if not args:
        print(__doc__)
        return 2
    app = (ROOT / args[0]).resolve() if not Path(args[0]).is_absolute() else Path(args[0])
    folders = args[1:] or [d for d in ("src", "crates") if (app / d).is_dir()]
    src = sources(app, folders)
    cat, problems = catalogues(app)
    for text, (where, kind) in src.items():
        if kind == "msg":
            _, bad = args_of(text)
            if bad:
                problems.append(f"{where}\tno placeholder of a message: {', '.join(bad)} in {text}")
            if "\t" in text or "\n" in text:
                problems.append(f"{where}\ta tab or line break cannot be in a catalogue: {text!r}")
    missing = [(w, t) for t, (w, _) in src.items() if t not in cat]
    for where, text in missing:
        print(f"{where}\t{text}")
    for p in problems:
        print(p)
    if "--unused" in sys.argv:
        for text, (where, _) in cat.items():
            if text not in src:
                print(f"{where}\tunused: {text}")
    print(f"i18n_check: {len(src)} texts, {len(missing)} without a translation, {len(problems)} other problems", file=sys.stderr)
    return 1 if missing or problems else 0


if __name__ == "__main__":
    sys.exit(main())
