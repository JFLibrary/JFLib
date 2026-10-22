"""i18n_check.py -- the translatable texts of the apps and the library against their catalogues
(`python build_rs.py i18n`; docs/GUIDE.md "Languages").

    python build_rs.py i18n                 every app with a catalogue and the library: a line
                                            each, then what is missing, untranslated, unused
                                            or wrong
    python build_rs.py i18n APP [--update]  one app; --update adds a row for every missing text
    python build_rs.py i18n lib             the library's own texts (tr("…") in lib/ against
                                            LIB_TEXTS of lib/ui/i18n.rs)
    python build_rs.py i18n --check         the suite's check: exit code 1 only for errors
    python tools/i18n_check.py [APP | apps/APP [FOLDER ..] | lib] .. [--update] [--check] [--unused]
                                            the same; FOLDER: only these folders of the app
                                            (apps/JFNetworker crates)

Exit code: 1 when anything is reported (--check: only errors, see below).

What is reported
  missing        a text of the sources without a row in any catalogue of the app
  untranslated   a row without a translation for a language (an empty or missing cell: the
                 app shows the source text there)
  unused         a row whose text is no string literal of the app's sources (*test.rs left
                 out) at all, also not one passed through a variable or a table (--unused
                 lists them, else they are counted)
  error          a row with more cells than languages, a text in two rows or catalogues with
                 different translations, a translation whose placeholders or plural forms
                 differ from its source, a message template its formatter cannot fill
--check (build_rs.py test) fails only for errors, and for missing or untranslated texts of the
targets whose catalogues are complete (COMPLETE: BackupCraft, JFNetworker, the library); the
other gaps (PaintCraft's) are counted, they do not fail.

--update adds the missing texts to the app's catalogue (with several: the one whose texts come
from the same folder of the app, src/ or crates/) at the end, in source order, below the comment
"# untranslated: ..." once, with empty cells (untranslated: the app shows the source text).
Existing rows are never changed; a text with a tab or a line break cannot be a row (reported).

Texts of the sources (*.rs below the app; files named *test.rs are left out)
  t("…") tc("…") tr("…")  a text the app translates (ui::i18n::t, its tr.rs, tc with a context
                          after '|', PaintCraft's localization::tr); i18n::tr("…") is the
                          library's and left out
  tf("…", ..)             filled by i18n::fill: only "{}", in order
  msg!("…", ..), Msg::new("…", ..), phrase("…")
                          a template of a message (jfn_model::msg): "{}" and "{0}".."{9}"
  list(&TABLE)            the literals of `static|const TABLE: [&str; N] = [..]` in that file
  Command::new(ID, "…", ..), Mi::Menu("…", ..), Mi::Sub("…", ..), Mi::Item(ID, "…", ..),
  Mi::List(ID, N, "…")    command and menu labels, when the app translates its tables (a
                          `label: t(c.label)` / `=> Mi::Item(id, t(label), icon)` of its own)
  // i18n                 on a line of its own: every literal of the block below it (up to a
                          less indented or empty line, or a line ending in "];")
  // i18n: name title     the same, only the literals after "name:" or "title:"
  … // i18n               at the end of a line: the literals of that line
Only texts with a letter count.  The library: tr("…") in lib/**/*.rs.

Catalogues: apps/NAME/**/locales/*.tsv.  The first line names the languages ("de\ten";
Catalog::from_tsv), or a comment "# languages: de en fr" does (a catalogue the app reads
itself, PaintCraft's); then one row per text: the source text, a tab, the translations; lines
that start with "#" are comments.

Placeholders: "{}" (in order), "{0}".."{9}", "{name}" and the forms of i18n::msg ("{n, number}",
"{d, date, long}", "{n, plural, =0 {..} one {# ..} other {..}}", "{g, select, .. other {..}}";
'{' quotes).  A translation uses the same arguments as its source, with the same kinds (a tf
text: as many "{}" and nothing else; "{}" and "{N}" count as argument numbers, so a template
may reorder them); a plural or select has "other", a plural has every category of its
language and no other (CLDR, as ui::i18n::plural).  A message template has no formats
("{:03}"), named placeholders, tabs or line breaks."""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LETTER = re.compile(r"[^\W\d_]")
LANG = re.compile(r"[a-z]{2,3}(?:[-_][A-Za-z0-9]{2,8})*")
MARK = "# untranslated: rows added by `python build_rs.py i18n --update`; fill in the empty cells"

# the code before a literal -> its kind ("t", "fill", "msg", "label"); the call's name must not
# be a method (".t(") or the library's i18n::tr
CALLS = [
    (re.compile(r"(?<![\w.:])(?:\w+::)*(?:\b(?:t|tc)|(?<!i18n::)\btr)\s*\(\s*$"), "t"),
    (re.compile(r"(?<![\w.:])(?:\w+::)*\btf\s*\(\s*$"), "fill"),
    (re.compile(r"(?<![\w.])(?:\bmsg!|\bphrase|\bMsg::new)\s*\(\s*$"), "msg"),
]
LABELS = [
    re.compile(r"\bCommand::new\(\s*[^,\"();]+,\s*$"),
    re.compile(r"\bMi::(?:Menu|Sub)\(\s*$"),
    re.compile(r"\bMi::Item\(\s*[^,\"();]+,\s*$"),
    re.compile(r"\bMi::List\(\s*[^,\"();]+,\s*[^,\"();]+,\s*$"),
]
TRANSLATES_MENUS = re.compile(r"=>\s*Mi::(?:Item|Menu|Sub|List)\([^;]*?[\w:]+\(\s*label\b")
TRANSLATES_CMDS = re.compile(r"\blabel\s*[:=]\s*[\w:]+\(\s*\w+\.label\b")
FIELD = re.compile(r"\b(\w+)\s*:\s*$")
LIST_CALL = re.compile(r"(?<![\w.])(?:\w+::)*\blist\s*\(\s*&\s*([A-Z_][A-Z0-9_]*)\s*\)")

# the CLDR plural categories ui::i18n::plural gives each language ("other" included)
PLURALS = {
    **dict.fromkeys(("ja", "zh", "ko", "vi", "th", "id", "ms", "lo", "km", "my", "yue"), {"other"}),
    **dict.fromkeys(("ru", "uk", "be", "pl", "cs", "sk", "lt"), {"one", "few", "many", "other"}),
    **dict.fromkeys(("hr", "sr", "bs", "sh", "ro"), {"one", "few", "other"}),
    "sl": {"one", "two", "few", "other"},
    "lv": {"zero", "one", "other"},
    "ar": {"zero", "one", "two", "few", "many", "other"},
    "he": {"one", "two", "other"},
    "iw": {"one", "two", "other"},
}
CATEGORIES = {"zero", "one", "two", "few", "many", "other"}
# complete today (2026-10-22): in --check a missing or untranslated text is an error there too,
# so they stay complete; the other apps' gaps are reported only
COMPLETE = ("BackupCraft", "JFNetworker", "lib")


def plural_categories(lang):
    return PLURALS.get(re.split(r"[-_]", lang)[0].lower(), {"one", "other"})


# ---------------------------------------------------------------- Rust sources
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
    every string literal as (start offset, end offset, value); byte strings are left out"""
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
        elif c == "r" and re.match(r'r#*"', text[i:i + 300]) and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            m = re.match(r'r(#*)"', text[i:])
            end = text.find('"' + m.group(1), i + m.end())
            end = n if end < 0 else end
            if i == 0 or text[i - 1] != "b":
                strings.append((i, end + 1 + len(m.group(1)), text[i + m.end():end]))
            i = end + 1 + len(m.group(1))
        elif c == '"':
            j = i + 1
            while j < n and text[j] != '"':
                j += 2 if text[j] == "\\" else 1
            if i == 0 or text[i - 1] != "b":
                strings.append((i, j + 1, unescape(text[i + 1:j])))
            i = j + 1
        elif c == "'":
            m = re.match(r"'(\\u\{[0-9a-fA-F_]+\}|\\x[0-9a-fA-F]{2}|\\.|[^\\'\n])'", text[i:i + 16])
            if m:
                blank(i, i + m.end())
                i += m.end()
            else:
                i += 1  # a lifetime
        else:
            i += 1
    return "".join(code), strings


def line_starts(text):
    starts = [0]
    for k, ch in enumerate(text):
        if ch == "\n":
            starts.append(k + 1)
    return starts


def line_of(starts, off):
    lo, hi = 0, len(starts) - 1
    while lo < hi:
        mid = (lo + hi + 1) // 2
        if starts[mid] <= off:
            lo = mid
        else:
            hi = mid - 1
    return lo


def marked_lines(lines):
    """the lines a "// i18n" comment marks: line index -> the fields that count (None: all)"""
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
    return marked


def table_literals(code, strings, name):
    """the literals of `static|const NAME: [&str; N] = [ .. ];` in a file (code: lexed)"""
    m = re.search(r"\b(?:static|const)\s+" + name + r"\s*:\s*\[\s*&\s*(?:'static\s+)?str\s*;[^\]]*\]\s*=\s*\[", code)
    if not m:
        return []
    end = code.find("];", m.end())
    end = len(code) if end < 0 else end
    return [s for s in strings if m.end() <= s[0] < end]


class Sources:
    """the texts of a tree of Rust sources: found {text: (where, kinds)}, literals {every string
    literal's value}"""

    def __init__(self, base, folders, library=False, skip=None):
        self.found = {}
        self.literals = set()
        files = []
        for d in folders:
            p = base / d
            files += [p] if p.is_file() else sorted(p.rglob("*.rs"))
        texts = {}
        for f in files:
            if f.name.endswith("test.rs") and not library:
                continue
            texts[f] = f.read_text(encoding="utf-8").replace("\r\n", "\n")
        both = "\n".join(lex(t)[0] for t in texts.values()) if not library else ""
        self.labels = bool(TRANSLATES_MENUS.search(both)), bool(TRANSLATES_CMDS.search(both))
        self.uses_i18n = re.search(r"\bset_catalog\s*\(", both) is not None
        for f, text in texts.items():
            self.scan(f, text, library, skip)

    def add(self, val, where, kind):
        if LETTER.search(val):
            w, kinds = self.found.setdefault(val, (where, set()))
            kinds.add(kind)

    def scan(self, f, text, library, skip):
        code, strings = lex(text)
        starts = line_starts(text)
        marked = {} if library else marked_lines(text.split("\n"))
        rel = f.relative_to(ROOT).as_posix()
        lo, hi = skip(rel, code) if skip else (0, 0)
        tables = set()
        if not library:
            for m in LIST_CALL.finditer(code):
                tables.update(s[0] for s in table_literals(code, strings, m.group(1)))
        menus, cmds = self.labels
        for off, end, val in strings:
            if lo <= off < hi:
                continue                                # the library's own table of texts
            self.literals.add(val)
            line = line_of(starts, off)
            where = f"{rel}:{line + 1}"
            before = code[max(0, off - 80):off]
            if library:
                if re.search(r"(?<![\w.])(?:\w+::)*\btr\s*\(\s*$", before):
                    self.add(val, where, "t")
                continue
            kind = next((k for rx, k in CALLS if rx.search(before)), None)
            if kind is None and off in tables:
                kind = "t"
            if kind is None and (menus or cmds):
                if (cmds and LABELS[0].search(before)) or (menus and any(rx.search(before) for rx in LABELS[1:])):
                    kind = "t"
            if kind is None and line in marked:
                fields = marked[line]
                fm = FIELD.search(code[starts[line]:off])
                if fields is None or (fm and fm.group(1) in fields):
                    kind = "t"
            if kind:
                self.add(val, where, kind)


# ---------------------------------------------------------------- placeholders
ICU = re.compile(r"\{\s*\w+\s*,")                       # "{n, plural, ..": i18n::msg's quoting applies
ARG_HEAD = re.compile(r"\s*([^,{}]*?)\s*([,}])")        # "{name," / "{0}" / "{}" / "{:03}"
ARG_KIND = re.compile(r"\s*(\w+)\s*([,}])")             # "plural," / "number}"
ARG_KEY = re.compile(r"\s*(offset:\s*\d+|=\d+|\w+)\s*")  # "one", "=0", "offset:1"
ARG_END = re.compile(r"\s*\}")


class Ph:
    """the placeholders of a text: positional / numbered argument numbers, named arguments with
    their kind, format specs, plural / select options, problems"""

    def __init__(self):
        self.nums = set()
        self.next = 0
        self.fills = 0            # "{}" alone (i18n::fill)
        self.other = 0            # "{N}" and named ones
        self.named = {}
        self.specs = []
        self.choices = []         # (name, "plural" | "select", [keys])
        self.bad = []


def placeholders(t):
    ph = Ph()
    scan_msg(t, 0, ICU.search(t) is not None, ph, False)
    return ph


def scan_msg(t, i, icu, ph, sub):
    """a message from i up to its end or (sub) the '}' that closes it -> the index of that '}'"""
    n = len(t)
    while i < n:
        c = t[i]
        if icu and c == "'" and i + 1 < n and t[i + 1] in "{}#'":
            if t[i + 1] == "'":
                i += 2
                continue
            j = t.find("'", i + 1)
            i = n if j < 0 else j + 1
            continue
        if not icu and (t.startswith("{{", i) or t.startswith("}}", i)):
            i += 2
            continue
        if c == "}":
            if sub:
                return i
            i += 1                                      # a lone '}' is text (fill, msg)
            continue
        if c != "{":
            i += 1
            continue
        i = scan_arg(t, i + 1, icu, ph)
    if sub:
        ph.bad.append("an open brace")
    return n


def scan_arg(t, i, icu, ph):
    """the placeholder after its '{' -> the index after its '}'"""
    n = len(t)
    m = ARG_HEAD.match(t, i)
    if not m:
        ph.bad.append("an open brace")
        return n
    head, sep = m.group(1), m.group(2)
    i = m.end()
    if head == "" and sep == "}":
        ph.nums.add(ph.next)
        ph.next += 1
        ph.fills += 1
        return i
    if head.isdigit() and sep == "}":
        ph.nums.add(int(head))
        ph.other += 1
        return i
    if not re.fullmatch(r"\w+", head):
        ph.specs.append("{" + head + ("}" if sep == "}" else ","))
        if sep == "}":
            return i
        j = t.find("}", i)
        return n if j < 0 else j + 1
    ph.other += 1
    if sep == "}":
        ph.named.setdefault(head, "")
        return i
    m = ARG_KIND.match(t, i)
    if not m:
        ph.bad.append("{" + head + ", ...")
        j = t.find("}", i)
        return n if j < 0 else j + 1
    kind, sep = m.group(1), m.group(2)
    i = m.end()
    if kind not in ("plural", "select", "selectordinal"):
        style = ""
        if sep == ",":
            j = t.find("}", i)
            style, i = (t[i:] if j < 0 else t[i:j]).strip(), (n if j < 0 else j + 1)
        ph.named[head] = kind + ("," + style if style else "")
        return i
    ph.named[head] = kind
    keys = []
    if sep == "}":
        ph.bad.append(f"{{{head}, {kind}}} without options")
        return i
    while True:
        m = ARG_KEY.match(t, i)
        if not m:
            m2 = ARG_END.match(t, i)
            if m2:
                ph.choices.append((head, kind, keys))
                return m2.end()
            ph.bad.append(f"{{{head}, {kind}, ...}} broken")
            j = t.find("}", i)
            return n if j < 0 else j + 1
        key = m.group(1)
        i = m.end()
        if key.startswith("offset:"):
            continue
        if i >= n or t[i] != "{":
            ph.bad.append(f"{{{head}, {kind}, ...}}: {key} without {{...}}")
            return n
        keys.append(key)
        i = scan_msg(t, i + 1, True, ph, True) + 1


def choice_problems(ph, lang):
    """problems of the plural / select options of a text in lang"""
    out = []
    want = plural_categories(lang)
    for name, kind, keys in ph.choices:
        if "other" not in keys:
            out.append(f"{{{name}, {kind}}} has no \"other\"")
        if kind == "plural":
            cats = {k for k in keys if not k.startswith("=")}
            odd = sorted(cats - CATEGORIES)
            if odd:
                out.append(f"{{{name}, plural}}: {', '.join(odd)} is no plural category")
            extra = sorted((cats & CATEGORIES) - want)
            if extra:
                out.append(f"{{{name}, plural}}: {lang} never uses {', '.join(extra)}")
            lack = sorted(want - cats - {"other"})
            if lack:
                out.append(f"{{{name}, plural}}: {lang} needs {', '.join(lack)}")
    return out


def compare(src, tr, kinds, lang):
    """problems of the translation tr (language lang) of src, used as kinds"""
    a, b = placeholders(src), placeholders(tr)
    out = [f"{x} ({lang})" for x in b.bad if x not in a.bad]
    if "fill" in kinds:
        if b.fills != a.fills or b.other or b.specs:
            out.append(f"placeholders differ ({lang}: i18n::fill takes {a.fills} \"{{}}\" in order and nothing else)")
            return out
    if a.nums != b.nums or {k: v.split(",")[0] for k, v in a.named.items()} != {k: v.split(",")[0] for k, v in b.named.items()} \
            or sorted(a.specs) != sorted(b.specs):
        out.append(f"placeholders differ ({lang})")
    out += choice_problems(b, lang)
    return out


def template_problems(text):
    """a message template (jfn_model::msg): no formats, names, tabs or line breaks"""
    ph = placeholders(text)
    out = []
    if ph.specs or ph.named:
        out.append("no placeholder of a message: " + ", ".join(ph.specs + ["{" + k + "}" for k in ph.named]))
    if "\t" in text or "\n" in text:
        out.append("a tab or line break cannot be in a catalogue")
    return out


# ---------------------------------------------------------------- catalogues
class Catalogue:
    """a TSV catalogue: langs (the source's first), rows {text: (line, cells)}, problems"""

    def __init__(self, path):
        self.path = path
        self.rel = path.relative_to(ROOT).as_posix()
        raw = path.read_bytes().decode("utf-8")
        self.crlf = "\r\n" in raw
        self.ends_nl = raw.endswith("\n")
        self.text = raw.replace("\r\n", "\n")
        lines = self.text.split("\n")
        self.langs, first = None, 0
        if lines and lines[0] and not lines[0].startswith("#") and all(LANG.fullmatch(c.strip()) for c in lines[0].split("\t")):
            self.langs, first = [c.strip() for c in lines[0].split("\t")], 1
        for ln in lines:
            m = re.fullmatch(r"#\s*languages:\s*(.+)", ln.strip())
            if m and self.langs is None:
                words = m.group(1).split()
                self.langs = words[:next((k for k, w in enumerate(words) if not LANG.fullmatch(w)), len(words))]
        self.rows, self.problems = {}, []
        width = 1
        for no, ln in enumerate(lines[first:], first + 1):
            if not ln or ln.startswith("#"):
                continue
            cols = ln.split("\t")
            width = max(width, len(cols))
            if cols[0] in self.rows:
                self.problems.append((f"{self.rel}:{no}", f"twice (also line {self.rows[cols[0]][0]})", cols[0]))
                continue
            self.rows[cols[0]] = (no, cols[1:])
        if self.langs is None:
            self.langs = ["de"] + [f"column {k}" for k in range(2, width + 1)]
        for text, (no, cells) in self.rows.items():
            if len(cells) > len(self.langs) - 1:
                self.problems.append((f"{self.rel}:{no}", f"{len(cells) + 1} cells, {len(self.langs)} languages", text))

    def append(self, texts):
        """add rows with empty cells for texts (never changing a row) -> how many"""
        add = [t for t in texts if t not in self.rows]
        if not add:
            return 0
        body = self.text if self.ends_nl or not self.text else self.text + "\n"
        if MARK not in self.text:
            body += MARK + "\n"
        body += "".join(t + "\t" * (len(self.langs) - 1) + "\n" for t in add)
        if self.crlf:
            body = body.replace("\n", "\r\n")
        self.path.write_bytes(body.encode("utf-8"))
        return len(add)


def lib_catalogue():
    """LIB_TEXTS of lib/ui/i18n.rs as a Catalogue, and the offsets of the table in that file"""
    f = ROOT / "lib" / "ui" / "i18n.rs"
    text = f.read_text(encoding="utf-8").replace("\r\n", "\n")
    code, strings = lex(text)
    starts = line_starts(text)
    cat = Catalogue.__new__(Catalogue)
    cat.path, cat.rel, cat.rows, cat.problems = f, f.relative_to(ROOT).as_posix(), {}, []
    m = re.search(r"const LIB_LANGS: \[&str; \d+\] = \[", code)
    end = code.find("]", m.end())
    cat.langs = [s[2] for s in strings if m.end() <= s[0] < end]
    m = re.search(r"static LIB_TEXTS: &\[\[&str; \d+\]\] = &\[", code)
    lo, hi = m.end(), code.find("\n];", m.end())
    inside = [s for s in strings if lo <= s[0] < hi]
    events = sorted([(s[0], s) for s in inside]
                    + [(k, code[k]) for k in range(lo, hi) if code[k] in "[]" and not any(a <= k < b for a, b, _ in inside)],
                    key=lambda e: e[0])
    row = None
    for _, e in events:
        if e == "[":
            row = []
        elif e == "]":
            if row:
                no = line_of(starts, row[0][0]) + 1
                key = row[0][2]
                if key in cat.rows:
                    cat.problems.append((f"{cat.rel}:{no}", "twice", key))
                cat.rows[key] = (no, [s[2] for s in row[1:]])
                if len(row) != len(cat.langs):
                    cat.problems.append((f"{cat.rel}:{no}", f"{len(row)} cells, {len(cat.langs)} languages", key))
            row = None
        elif row is not None:
            row.append(e)
    return cat, (lo, hi)


# ---------------------------------------------------------------- one app
class Report:
    def __init__(self, name):
        self.name = name
        self.lines = []           # (category, where, what)
        self.count = {"missing": 0, "untranslated": 0, "unused": 0, "error": 0}
        self.texts = 0
        self.catalogues = []
        self.langs = []

    def add(self, cat, where, what):
        self.lines.append((cat, where, what))
        self.count[cat] += 1


def catalogues_of(app):
    return [p for p in sorted(app.rglob("locales/*.tsv")) if "build" not in p.relative_to(app).parts]


def check_rows(rep, rows, langs, found):
    """untranslated cells and the placeholders of every row (rows: [(catalogue, text, line, cells)])"""
    for cat, text, no, cells in rows:
        where = f"{cat.rel}:{no}"
        kinds = found.get(text, ("", {"t"}))[1]
        src_lang = langs[0] if langs else "de"
        for p in choice_problems(placeholders(text), src_lang) + [x for x in placeholders(text).bad]:
            rep.add("error", where, f"{p} (source): {text}")
        if "msg" in kinds:
            for p in template_problems(text):
                rep.add("error", where, f"{p}: {text!r}")
        for k, lang in enumerate(langs[1:]):
            cell = cells[k] if k < len(cells) else ""
            if not cell:
                rep.add("untranslated", where, f"{lang}: {text}")
                continue
            for p in compare(text, cell, kinds, lang):
                rep.add("error", where, f"{p}: {text}  |  {cell}")


def check_app(app, folders=None):
    """the Report of apps/NAME (folders: only these; default: every folder of the app)"""
    rep = Report(app.name)
    cats = [Catalogue(p) for p in catalogues_of(app)]
    rep.catalogues = cats
    if not cats:
        return rep, None
    rep.langs = cats[0].langs
    for c in cats[1:]:
        if c.langs != rep.langs:
            rep.add("error", c.rel, f"languages {' '.join(c.langs)}, {cats[0].rel}: {' '.join(rep.langs)}")
    every = Sources(app, [d.name for d in sorted(app.iterdir()) if d.is_dir() or d.suffix == ".rs"])
    src = every if folders is None else Sources(app, folders)
    rep.texts = len(src.found)
    # ui::i18n::t falls back to the library's texts (an app with Catalog / set_catalog)
    library = set(lib_catalogue()[0].rows) if every.uses_i18n else set()
    rows, seen = [], {}
    for c in cats:
        for where, what, text in c.problems:
            rep.add("error", where, f"{what}: {text}")
        for text, (no, cells) in c.rows.items():
            if text in seen:
                other, ocells, ono = seen[text]
                if [x for x in ocells if x] != [x for x in cells if x] and any(cells):
                    rep.add("error", f"{c.rel}:{no}", f"translated otherwise in {other.rel}:{ono}: {text}")
                continue
            seen[text] = (c, cells, no)
            rows.append((c, text, no, cells))
    check_rows(rep, rows, rep.langs, every.found)
    for text, (where, kinds) in src.found.items():
        if "msg" in kinds and text not in seen:
            for p in template_problems(text):
                rep.add("error", where, f"{p}: {text!r}")
    missing = sorted(((w, t) for t, (w, _) in src.found.items() if t not in seen and t not in library),
                     key=lambda x: (where_key(x[0]), x[1]))
    for where, text in missing:
        rep.add("missing", where, text)
    if folders is None:
        for c, text, no, _ in rows:
            if text not in every.literals:
                rep.add("unused", f"{c.rel}:{no}", text)
    return rep, (cats, src, missing)


def where_key(w):
    f, _, line = w.rpartition(":")
    return f, int(line) if line.isdigit() else 0


def update(app, cats, src, missing):
    """add the missing texts to the catalogues -> lines to print"""
    out = []
    if not cats:
        return [f"{app.name}: no catalogue (apps/{app.name}/src/locales/*.tsv) to add to"]
    top = {}
    for c in cats:
        for text in c.rows:
            if text in src.found:
                part = Path(src.found[text][0]).relative_to(app.relative_to(ROOT)).parts[0]
                top.setdefault(c.rel, {}).setdefault(part, 0)
                top[c.rel][part] += 1
    plan = {}
    for where, text in missing:
        if "\t" in text or "\n" in text or "\r" in text:
            out.append(f"{where}\tnot added (a tab or line break cannot be in a catalogue): {text!r}")
            continue
        part = Path(where).relative_to(app.relative_to(ROOT)).parts[0]
        best = max(cats, key=lambda c: (top.get(c.rel, {}).get(part, 0), -cats.index(c)))
        plan.setdefault(best.rel, (best, []))[1].append(text)
    for rel, (c, texts) in plan.items():
        n = c.append(texts)
        out.append(f"{rel}: {n} row(s) added (untranslated)")
    return out


# ---------------------------------------------------------------- the library
def check_lib():
    rep = Report("lib")
    cat, (lo, hi) = lib_catalogue()
    rep.catalogues, rep.langs = [cat], cat.langs
    i18n_rel = cat.rel

    def skip(rel, code):
        return (lo, hi) if rel == i18n_rel else (0, 0)

    src = Sources(ROOT / "lib", [d.name for d in sorted((ROOT / "lib").iterdir()) if d.is_dir()], library=True, skip=skip)
    rep.texts = len(src.found)
    for where, what, text in cat.problems:
        rep.add("error", where, f"{what}: {text}")
    check_rows(rep, [(cat, t, no, cells) for t, (no, cells) in cat.rows.items()], cat.langs, src.found)
    for text, (where, _) in sorted(src.found.items(), key=lambda x: where_key(x[1][0])):
        if text not in cat.rows:
            rep.add("missing", where, text)
    # unused: no literal in lib/ outside the table, nor in an app (ui::i18n::t falls back to it)
    left = [(t, no) for t, (no, _) in cat.rows.items() if t not in src.literals]
    if left:
        apps_text = "\n".join(f.read_text(encoding="utf-8") for f in sorted((ROOT / "apps").rglob("*.rs")))
        for text, no in left:
            if '"' + text.replace("\\", "\\\\").replace('"', '\\"') + '"' not in apps_text:
                rep.add("unused", f"{cat.rel}:{no}", text)
    return rep


# ---------------------------------------------------------------- command line
def apps_with_catalogues():
    return [d for d in sorted((ROOT / "apps").iterdir()) if d.is_dir() and catalogues_of(d)]


def summary(rep):
    c = rep.count
    cats = ", ".join(x.rel.split("/", 2)[-1] if rep.name != "lib" else "LIB_TEXTS" for x in rep.catalogues)
    return (f"{rep.name}: {rep.texts} texts ({cats}; {' '.join(rep.langs)}): {c['missing']} missing, "
            f"{c['untranslated']} untranslated, {c['unused']} unused, {c['error']} errors")


def main(argv=None):
    for s in (sys.stdout, sys.stderr):
        try:
            s.reconfigure(encoding="utf-8", errors="replace")
        except (AttributeError, ValueError):
            pass
    argv = sys.argv[1:] if argv is None else argv
    flags = {a for a in argv if a.startswith("--")}
    bad = flags - {"--update", "--check", "--unused", "--help"}
    words = [a for a in argv if not a.startswith("--")]
    if bad or "--help" in flags:
        print(__doc__)
        return 2
    targets = []                                        # (name, app dir or None, folders)
    for w in words:
        if targets and targets[-1][1] is not None and (targets[-1][1] / w).exists() and "/" not in w.strip("/"):
            targets[-1][2].append(w)                    # apps/JFNetworker crates
            continue
        if w == "lib":
            targets.append(("lib", None, []))
            continue
        p = Path(w)
        app = p if p.is_absolute() else (ROOT / p if (ROOT / p).is_dir() and len(p.parts) > 1 else ROOT / "apps" / w)
        if not (app / "src").is_dir() and not app.is_dir():
            print(f"i18n: no app {w} (apps/NAME)")
            return 2
        targets.append((app.name, app.resolve(), []))
    every = not targets
    if every:
        targets = [(a.name, a, []) for a in apps_with_catalogues()] + [("lib", None, [])]
    check, upd, unused = "--check" in flags, "--update" in flags, "--unused" in flags
    errors = gaps = 0
    out = []
    for name, app, folders in targets:
        if app is None:
            rep, data = check_lib(), None
        else:
            rep, data = check_app(app, folders or None)
            if not rep.catalogues:
                out.append(f"{name}: no catalogue (apps/{name}/**/locales/*.tsv)")
                continue
        if upd and app is not None and data and rep.count["missing"]:
            for ln in update(app, *data):
                out.append(ln)
            rep, data = check_app(app, folders or None)
        out.append(summary(rep))
        strict = ("missing", "untranslated") if check and name in COMPLETE else ()
        errors += rep.count["error"] + sum(rep.count[c] for c in strict)
        gaps += sum(rep.count[c] for c in ("missing", "untranslated", "unused") if c not in strict)
        for cat, where, what in rep.lines:
            if cat == "error" or cat in strict or (not check and (cat != "unused" or unused or not every)):
                out.append(f"  {where}\t{cat}\t{what}")
    print("\n".join(out))
    if check:
        print(f"i18n: {'ok' if not errors else 'FAILED'} ({errors} errors; {gaps} gaps reported, not errors)")
        return 1 if errors else 0
    return 1 if errors or gaps else 0


if __name__ == "__main__":
    sys.exit(main())
