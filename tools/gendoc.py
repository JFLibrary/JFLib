#!/usr/bin/env python3
"""gendoc.py -- docs/API.md from the doc comments of the Rust library (lib).

    python tools/gendoc.py             # writes docs/API.md
    python tools/gendoc.py --check     # writes nothing; exit 1 when docs/API.md is stale
    python tools/gendoc.py --missing   # also list the public items without a comment
    python tools/gendoc.py --check --api   # ... and sum up the API changes since the last
                                            # release (tools/apidiff.py, docs/VERSIONING.md)

What ends up in API.md, per crate (the CRATES table below sets order and grouping):
  * the crate header: the // comment block at the top of lib.rs;
  * per module file: its // header block (verbatim, in a code block), then every public item
    defined in the file, in source order -- functions, the pub methods of public types (each
    `impl` block), traits with all their methods, structs with their pub fields, enums with
    their variants, type aliases, statics, exported macros, inline pub modules, and constants
    (compact code blocks, grouped by the `// ---- title` separators of the source);
  * re-exports of other crates (`pub use rt::os::{exit, ...}`) as one line each.
"Public" means reachable from another crate: pub items of the crate root and of `pub mod`
modules, items re-exported with `pub use` (globs too, through private modules), pub methods
of public types, every method of a public trait.  An item's text is its /// comment; consts,
fields and variants also take the // comment that ends their line, consts the plain //
lines right above them.  Files of #[cfg(windows)] / #[cfg(target_os = "linux")] modules are
marked (Windows) / (Linux).

ui: the files are grouped by area (UI_AREAS), with an index of the Ui methods per area.
Codec crates (img_png ... img_pdn): the crate root, the items re-exported there, the items
named in CODEC_EXTRA and a table of the other pub modules (their first header line): those
are the codecs' building blocks, used only by the codec test app.

The parser is a small lexer (comments, strings, char literals vs lifetimes, raw strings)
plus bracket matching -- no rustc involved.  The output is deterministic (file order from
the tables below and the `mod` declarations, items in source order, GitHub heading anchors),
so a rerun writes the same bytes; --check compares them.  `build_rs.py test` can run
--check as a lint step.
"""
import re
import sys
from pathlib import Path
from project_sources import entrypoint, project_sources

ROOT = Path(__file__).resolve().parents[1]
LIB = ROOT / "lib"
OUT = ROOT / "docs" / "API.md"

# ---------------------------------------------------------------- configuration
# ui: areas (files of lib/ui); a file not named here lands in "Other modules"
UI_AREAS = [
    ("App, AppDesc, Ui, run, async tasks", ["lib.rs", "runtime.rs", "tasks.rs"]),
    ("Layout", ["layout.rs"]),
    ("Context: input, focus, timers, commands, semantics", ["ctx.rs"]),
    ("Keyboard: Tab order, focus groups, panes and windows (F6), splitters (F8)", ["kbnav.rs"]),
    ("Widgets", ["basic.rs"]),
    ("Popups, menus, command table, tooltips", ["popup.rs"]),
    ("Text boxes and the clipboard", ["edit.rs", "edit_spell.rs"]),
    ("Rich-text and code editors", ["ed_core.rs", "rich_doc.rs", "rich_html.rs", "rich_edit.rs", "code_doc.rs", "highlight.rs",
                                    "code_edit.rs"]),
    ("Lists, scrolling, tabs, expanders", ["list.rs"]),
    ("Data views: grid, tree, badges, meters", ["data.rs"]),
    ("Shell parts: navigation, command bar, splitter, breadcrumb, wizard steps", ["shell.rs"]),
    ("Docking: panels at the sides, tab groups, floating tool windows", ["dock.rs"]),
    ("Charts", ["chart.rs"]),
    ("Toasts, spinners, progress", ["notify.rs"]),
    ("Date and time input", ["datepick.rs"]),
    ("Languages", ["i18n.rs"]),
    ("Vector icons", ["vicon.rs"]),
    ("Values and colours", ["value.rs", "color.rs"]),
    ("Windows", ["win.rs"]),
    ("Chrome: title bar, dialogs, info and status bars", ["chrome.rs"]),
    ("File dialog", ["filedlg.rs"]),
    ("Drawing commands", ["cmd.rs"]),
    ("Scripts and the widget dump", ["auto.rs"]),
    ("Theme", ["theme.rs"]),
    ("Cursors, drag and drop, clipboard images, dirty regions", ["cursor.rs", "drop.rs", "dnd.rs", "clipimg.rs", "dirty.rs"]),
    ("Style contract", ["style.rs"]),
    ("Stock painters of the data and shell parts", ["stock.rs"]),
    ("Constants", ["consts.rs"]),
    ("Key codes", ["keys.rs"]),
]

# (crate, title, kind): "full" = every public item, "codec" = see the docstring
CRATES = [
    ("ui", "the immediate-mode UI: runtime, layout, widgets, windows, style contract", "full"),
    ("agent_core", "transport-independent actions, schema validation, JSON-RPC, MCP and tasks", "full"),
    ("agent_io", "explicitly enabled stdio and current-user native agent transports", "full"),
    ("gfx", "surfaces, the draw context, blend kernels, paths, strokes, masks, gradients", "full"),
    ("text", "fonts (SGF1, TrueType, CFF, COLR), glyph cache, text runs", "full"),
    ("spell", "spell checking: Hunspell dictionaries, the system's checker, a personal word list", "full"),
    ("img_core", "image I/O dispatch, records, metadata, colour, resampling, image operations", "full"),
    ("img_all", "registration of every codec", "full"),
    ("img_png", "PNG", "codec"),
    ("img_jpeg", "JPEG", "codec"),
    ("img_bmp", "BMP, ICO, CUR", "codec"),
    ("img_gif", "GIF", "codec"),
    ("img_tga", "TGA", "codec"),
    ("img_tiff", "TIFF", "codec"),
    ("img_webp", "WebP", "codec"),
    ("img_heif", "HEIF container (HEIC)", "codec"),
    ("img_hevc", "HEVC (H.265) intra coding", "codec"),
    ("img_av1", "AV1 intra coding", "codec"),
    ("img_avif", "AVIF", "codec"),
    ("img_jxl", "JPEG XL", "codec"),
    ("img_jxl_enc", "the new JPEG XL writer (linked into codectest; PaintCraft loads it as the module jxlenc)",
     "codec"),                                                                             # jxl-module
    ("img_jxr", "JPEG XR", "codec"),
    ("img_dds", "DDS and BC1..BC7 block compression", "codec"),
    ("img_pdn", "paint.net documents (.pdn)", "codec"),
    ("img_svg", "SVG images and icons: parsed once, drawn at any size through gfx's paths (the format IMGF_SVG)",
     "full"),                                                                              # svg
    ("fx", "effects and adjustments", "full"),
    ("plat", "the platform layer: process, files, threads, windows (Windows and Linux)", "full"),
    ("base", "UTF-8, command line, settings files, numbers and text", "full"),
    ("datetime", "timestamps, civil dates, RFC 3339, IANA time zones with DST, cron schedules", "full"),
    ("script", "the JFNetworker scripting language (.jfs): lexer, parser, editor diagnostics, compiler, a VM "
     "with fuel and a garbage collector, the standard library (str, list, map, bytes, math, json), float "
     "arithmetic and math in software", "full"),
    ("crypto", "hashes, MACs, KDFs, Argon2id, AEAD, X25519, Ed25519, sealed boxes (no randomness inside)", "full"),
    ("compress", "LZ4 block and frame, xxHash32, codec ids", "full"),
    ("datastore", "an embedded journal database: JSON collections, transactions, snapshots, crash recovery",
     "full"),
    ("netio", "HTTP/1.1 server and client, TLS 1.3 subset, X.509 of a private CA, SSH-2, DNS", "full"),
    ("netpkt", "network packets byte for byte: addresses, checksums, Ethernet to TCP codecs, DHCP, DNS, NTP, "
     "syslog, a dissector, pcapng", "full"),
    ("cas", "content-addressed backup repositories: chunking, packs, signed encrypted snapshots, GC",
     "full"),
    ("update", "signed app updates: check, download, apply at the next start, health marker, rollback",
     "full"),
    ("archive", "bounded ZIP32 containers (stored and DEFLATE)", "full"),
    ("doc_core", "structured documents, sources, citations and undo/redo", "full"),
    ("doc_layout", "typographic paragraph flow and page layout", "full"),
    ("doc_view", "shared document fonts, page painting, thumbnails and page coordinates", "full"),
    ("doc_io", "conservative DOCX interchange and bounded file I/O", "full"),
    ("rt", "the runtime parts apps use: SSE / x87 float semantics, CPU features, locks, crash records",
     "full"),
    ("plugin", "the PaintCraft plugin ABI for plugins written in Rust", "full"),
]
SKIP_FILES = {"rt": {"os.rs", "mem.rs"}}            # rt::os: the OS calls plat is built on
# codec crates: further items to show, per module ("*" = all public items of the module)
CODEC_EXTRA = {
    # jxl-module: the writer's options and flags are img_jxl::writer's, the writer img_jxl_enc
    "img_jxl": {"writer": ["IOF_JXL_COMPAT", "IOF_JXL_1THREAD", "IOF_JXL_PROGRESSIVE", "IOF_JXL_NOPATCHES",
                           "IOF_JXL_NOISE_SHIFT", "IOF_JXL_NOISE_MASK", "Opts", "Kind", "set_static", "use_module",
                           "kind", "write"],
                "jpeg": ["has_jbrd", "reconstruct"]},
    "img_jxl_enc": {"enew": ["encode", "encode_px", "encode_with_splines"]},
}
CFG_LABEL = [(re.compile(r'cfg\(\s*windows\s*\)'), "Windows"),
             (re.compile(r'cfg\(\s*target_os\s*=\s*"linux"\s*\)'), "Linux")]
VALUE_MAX = 60                                      # longer const values are cut


# ---------------------------------------------------------------- lexer
RAW_RE = re.compile(r'[bc]?r(#*)"')


def lex(src):
    """-> (clean, nocom): `clean` has comments and the contents of string / char literals
    blanked, `nocom` only the comments (both keep every position and newline of src)"""
    clean, nocom = list(src), list(src)
    n = len(src)

    def blank(arr, a, b):
        for k in range(a, b):
            if src[k] != "\n":
                arr[k] = " "

    i = 0
    while i < n:
        c = src[i]
        if c == "/" and src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j < 0 else j
            blank(clean, i, j)
            blank(nocom, i, j)
            i = j
        elif c == "/" and src.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif src.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            blank(clean, i, j)
            blank(nocom, i, j)
            i = j
        elif c in "rbc" and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")):
            m = RAW_RE.match(src, i)
            if m:                                   # r"..", r#".."#, br"..": no escapes
                close = '"' + m.group(1)
                j = src.find(close, m.end())
                j = n if j < 0 else j
                blank(clean, m.end(), j)
                i = j + len(close)
            elif c in "bc" and i + 1 < n and src[i + 1] in "\"'":
                i += 1                              # b"..", b'.', c"..": the literal follows
            else:
                while i < n and (src[i].isalnum() or src[i] == "_"):
                    i += 1
        elif c.isalpha() or c == "_":
            while i < n and (src[i].isalnum() or src[i] == "_"):
                i += 1
        elif c == '"':
            j = i + 1
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            blank(clean, i + 1, j)
            i = j + 1
        elif c == "'":
            if i + 1 < n and src[i + 1] == "\\":    # '\n', '\'', '\u{..}'
                j = src.find("'", i + 3)
                j = n if j < 0 else j
                blank(clean, i + 1, j)
                i = j + 1
            elif i + 2 < n and src[i + 2] == "'":   # 'x'
                blank(clean, i + 1, i + 2)
                i += 3
            else:
                i += 1                              # a lifetime 'a
        else:
            i += 1
    return "".join(clean), "".join(nocom)


def match_close(clean, i):
    """index of the bracket closing the one at i"""
    depth = 0
    for j in range(i, len(clean)):
        ch = clean[j]
        if ch in "([{":
            depth += 1
        elif ch in ")]}":
            depth -= 1
            if depth == 0:
                return j
    return len(clean) - 1


def scan_to(clean, i, end, stops, braces):
    """first index >= i of a char in `stops` at bracket depth 0; braces count as brackets
    only when `braces` (else a '{' is a stop when listed, or its block is skipped)"""
    depth = 0
    j = i
    while j < end:
        ch = clean[j]
        if depth == 0 and ch in stops:
            return j
        if ch in "([" or (braces and ch == "{"):
            depth += 1
        elif ch in ")]" or (braces and ch == "}"):
            depth -= 1
        elif ch == "{":
            j = match_close(clean, j)
        j += 1
    return end


def squash(s):
    return re.sub(r"\s+", " ", s).strip()


def strip_doc(s):
    """'/// text' -> 'text'"""
    s = s.strip()[3:]
    return s[1:] if s[:1] == " " else s


# ---------------------------------------------------------------- items
class Item:
    def __init__(self, **kw):
        self.cfg = ""
        self.members = []                   # impl / trait: items of the block
        self.fields = []                    # struct / enum: (vis, text, doc, trailing)
        self.__dict__.update(kw)


def classify(h):
    """(vis, kind, name) from the cleaned, squashed start of an item"""
    m = re.match(r"(pub(?:\s*\([^)]*\))?)\s+", h)
    vis = squash(m.group(1)) if m else ""
    rest = h[m.end():] if m else h
    toks = re.findall(r'"[^"]*"|[A-Za-z_][A-Za-z_0-9]*!?|\S', rest)
    k = 0
    while k < len(toks):
        t = toks[k]
        if t in ("async", "unsafe", "default", "safe"):
            k += 1
        elif t == "const" and k + 1 < len(toks) and toks[k + 1] in ("fn", "unsafe", "extern", "async"):
            k += 1
        elif t == "extern":
            if k + 1 < len(toks) and toks[k + 1] == "crate":
                return vis, "extern_crate", ""
            k += 2 if k + 1 < len(toks) and toks[k + 1].startswith('"') else 1
            if k < len(toks) and toks[k] == "{":
                return vis, "extern_block", ""
        else:
            break
    if k >= len(toks):
        return vis, "other", ""
    t = toks[k]
    name = toks[k + 1] if k + 1 < len(toks) else ""
    if t in ("fn", "struct", "enum", "union", "trait", "type", "mod", "const"):
        return vis, t, name
    if t == "static":
        return vis, "static", toks[k + 2] if name == "mut" and k + 2 < len(toks) else name
    if t in ("use", "impl"):
        return vis, t, ""
    if t == "macro_rules!":
        return vis, "macro", name
    return vis, "other", ""


def trailing_comment(src, pos):
    """the // comment after position pos on its line"""
    e = src.find("\n", pos)
    rest = src[pos:e if e >= 0 else len(src)]
    k = rest.find("//")
    if k < 0 or rest.startswith("///", k):
        return ""
    return rest[k + 2:].strip()


def sep_title(line):
    """'// ----- widget events' -> 'widget events'; None when the line is no separator"""
    m = re.match(r"\s*//\s*[-=]{3,}\s*(.*?)\s*[-=]*\s*$", line)
    return m.group(1) if m else None


class Parser:
    """one source file: lexed text, line table, items"""

    def __init__(self, path):
        self.path = path
        self.src = path.read_bytes().decode("utf-8")
        self.clean, self.nocom = lex(self.src)
        self.lines = self.src.split("\n")
        self.line_starts = [0]
        for ln in self.lines:
            self.line_starts.append(self.line_starts[-1] + len(ln) + 1)
        self.header_lines = 0

    def line_of(self, pos):
        lo, hi = 0, len(self.line_starts) - 1
        while lo < hi:
            mid = (lo + hi + 1) // 2
            if self.line_starts[mid] <= pos:
                lo = mid
            else:
                hi = mid - 1
        return lo

    def header(self):
        """the // block at the top of the file (comment prefix removed)"""
        out = []
        for ln in self.lines:
            s = ln.strip()
            if not s.startswith("//") or s.startswith("///") or s.startswith("//!"):
                break
            out.append(s[2:][1:] if s[2:3] == " " else s[2:])
        self.header_lines = len(out)
        while out and not out[-1].strip():
            out.pop()
        return [ln.rstrip() for ln in out]

    def docs(self, first_line, kw_line):
        """/// lines directly above first_line (attribute lines and plain // lines skipped, as
        rustc attaches them) and between the attributes and the keyword line"""
        out = []
        j = first_line - 1
        while j >= 0:
            s = self.lines[j].strip()
            if s.startswith("///"):
                out.append(strip_doc(s))
            elif not (s.startswith("#[") or s.startswith("#![") or (s.startswith("//") and not s.startswith("//!"))):
                break
            j -= 1
        out.reverse()
        for li in range(first_line, kw_line):
            if self.lines[li].strip().startswith("///"):
                out.append(strip_doc(self.lines[li]))
        while out and not out[0].strip():
            out.pop(0)
        while out and not out[-1].strip():
            out.pop()
        return out

    def gap(self, a_line, b_line):
        """(separator title, plain // lines right above b_line) of the lines a_line..b_line-1"""
        title, notes = None, []
        for li in range(max(a_line, self.header_lines), b_line):
            s = self.lines[li].strip()
            t = sep_title(s) if s.startswith("//") else None
            if t is not None:
                title, notes = t, []
            elif s.startswith("//") and not s.startswith("///"):
                notes.append(s[2:].strip())
            elif s.startswith("///") or s.startswith("#["):
                continue
            else:
                notes = []
        return title, notes

    def items(self, start, end):
        """the items between start and end (the file or a block body)"""
        c = self.clean
        out = []
        pos = start
        prev_line = self.line_of(start)
        while True:
            while pos < end and c[pos].isspace():
                pos += 1
            if pos >= end:
                break
            if c[pos] in ";,":
                pos += 1
                continue
            istart = pos
            attrs = []
            while c.startswith("#[", pos) or c.startswith("#![", pos):
                e = match_close(c, c.index("[", pos))
                attrs.append(squash(self.src[pos:e + 1]))
                pos = e + 1
                while pos < end and c[pos].isspace():
                    pos += 1
            if pos >= end:
                break
            if attrs and all(a.startswith("#!") for a in attrs):    # #![no_std] ...
                prev_line = self.line_of(pos)
                continue
            vis, kind, name = classify(squash(c[pos:pos + 400]))
            body = None
            if kind in ("const", "static", "use", "type", "extern_crate"):
                hend = scan_to(c, pos, end, ";", True)
            else:
                hend = scan_to(c, pos, end, "{;", False)
                if hend < end and c[hend] == "{":
                    body = (hend + 1, match_close(c, hend))
            first = self.line_of(istart)
            title, notes = self.gap(prev_line, first)
            it = Item(kind=kind, name=name, vis=vis, attrs=attrs, sig=squash(self.nocom[pos:hend]), body=body,
                      doc=self.docs(first, self.line_of(pos)), title=title, notes=notes, trailing="")
            if body is None:
                it.trailing = trailing_comment(self.src, min(hend + 1, end))
            if kind in ("impl", "trait") and body:
                it.members = self.items(*body)
            elif kind in ("struct", "enum", "union") and body:
                it.fields = self.fields(body[0], body[1], kind)
            out.append(it)
            pos = (body[1] + 1) if body else hend + 1
            prev_line = self.line_of(pos) + 1
        return out

    def fields(self, start, end, kind):
        """struct fields / enum variants: [(vis, text, doc, trailing)]"""
        c = self.clean
        out = []
        pos = start
        while pos < end:
            while pos < end and (c[pos].isspace() or c[pos] == ","):
                pos += 1
            if pos >= end:
                break
            fstart = pos
            while c.startswith("#[", pos):
                pos = match_close(c, c.index("[", pos)) + 1
                while pos < end and c[pos].isspace():
                    pos += 1
            depth, j = 0, pos                       # the field ends at a ',' at depth 0
            while j < end:
                ch = c[j]
                if ch in "([{<":
                    depth += 1
                elif ch in ")]}" or (ch == ">" and c[j - 1] != "-"):
                    depth -= 1
                elif ch == "," and depth == 0:
                    break
                j += 1
            text = squash(self.nocom[pos:j])
            m = re.match(r"(pub(?:\s*\([^)]*\))?)\s+", text)
            vis = "pub" if kind == "enum" else (squash(m.group(1)) if m else "")
            if m:
                text = text[m.end():]
            if text:
                out.append((vis, text, self.docs(self.line_of(fstart), self.line_of(pos)),
                            trailing_comment(self.src, min(j + 1, end))))
            pos = j + 1
        return out


def parse_use(text):
    """'pub use a::b::{c, d as e, f::*}' -> [(segments, alias, glob)]"""
    t = re.sub(r"^pub(\s*\([^)]*\))?\s+", "", squash(text))
    t = re.sub(r"^use\s+", "", t).rstrip(";").strip()
    out = []

    def walk(prefix, s):
        s = s.strip()
        if not s:
            return
        if s.endswith("}") and "{" in s:
            k = s.index("{")
            head = s[:k].rstrip(":").strip()
            pre = prefix + [p for p in head.split("::") if p]
            depth, cur, parts = 0, "", []
            for ch in s[k + 1:-1]:
                depth += (ch == "{") - (ch == "}")
                if ch == "," and depth == 0:
                    parts.append(cur)
                    cur = ""
                else:
                    cur += ch
            parts.append(cur)
            for p in parts:
                walk(pre, p)
            return
        alias = None
        m = re.match(r"(.*)\s+as\s+(\w+)$", s)
        if m:
            s, alias = m.group(1).strip(), m.group(2)
        segs = prefix + [p for p in s.split("::") if p]
        if segs and segs[-1] == "*":
            out.append((segs[:-1], None, True))
        else:
            out.append((segs, alias, False))

    walk([], t)
    return out


def cfg_of(attrs):
    for a in attrs:
        if re.search(r"cfg\(\s*test\s*\)", a):
            return "test"
        for rx, label in CFG_LABEL:
            if rx.search(a):
                return label
    return ""


# ---------------------------------------------------------------- modules
class Module:
    def __init__(self, path, name, parent, cfg, vis, item=None):
        self.path, self.name, self.parent = path, name, parent
        self.cfg, self.vis, self.item = cfg, vis, item
        self.items = []         # every item but mod / use (source order)
        self.children = {}      # name -> Module
        self.inline = []        # inline child modules (mod x { .. }) in source order
        self.uses = []          # (vis, [(segments, alias, glob)], text)
        self.header = []


def load_module(path, name, parent, cfg, vis, files, item=None):
    mod = Module(path, name, parent, cfg, vis, item)
    p = Parser(path)
    files.append(mod)
    mod.header = p.header()
    child_dir = path.parent if path.name in ("lib.rs", "mod.rs") else path.parent / path.stem
    fill_module(mod, p, p.items(0, len(p.clean)), child_dir, path.parent, files)
    return mod


def fill_module(mod, p, items, child_dir, file_dir, files):
    for it in items:
        cfg = cfg_of(it.attrs)
        if cfg == "test":
            continue
        it.cfg = cfg or mod.cfg
        if it.kind == "mod":
            if it.body is not None:                             # mod name { .. }
                sub = Module(mod.path, it.name, mod, it.cfg, it.vis, it)
                mod.children[it.name] = sub
                mod.inline.append(sub)
                fill_module(sub, p, p.items(*it.body), child_dir / it.name, file_dir, files)
                continue
            pattr = next((a for a in it.attrs if a.startswith("#[path")), None)
            if pattr:
                f = file_dir / re.search(r'"([^"]+)"', pattr).group(1)
            else:
                f = child_dir / (it.name + ".rs")
                if not f.exists():
                    f = child_dir / it.name / "mod.rs"
            if f.exists():
                mod.children[it.name] = load_module(f, it.name, mod, it.cfg, it.vis, files, it)
        elif it.kind == "use":
            mod.uses.append((it.vis, parse_use(it.sig), it.sig))
        elif it.kind not in ("extern_crate", "extern_block", "other"):
            mod.items.append(it)


def is_exported_macro(it):
    return it.kind == "macro" and any("macro_export" in a for a in it.attrs)


class Crate:
    """a crate's modules and which of their items another crate can reach"""

    def __init__(self, name):
        self.name = name
        self.files = []                         # file modules in mod declaration order
        root = entrypoint(LIB / name, "lib")
        project_sources(root)  # same validation and reference traversal as build/lint/IDE
        self.root = load_module(root, name, None, "", "pub", self.files)
        self.reach = set()                      # id(item): public
        self.rexp = set()                       # id(item): public through a pub use
        self.ns = set()                         # id(module): reachable as a path
        self.done = set()
        self.external = []                      # (module, text): pub use of another crate
        self.visit_ns(self.root)
        self.types = {it.name for m in self.all_modules() for it in m.items
                      if id(it) in self.reach and it.kind in ("struct", "enum", "union", "trait", "type")}

    def all_modules(self):
        out = []

        def walk(m):
            out.append(m)
            for ch in m.children.values():
                walk(ch)
        walk(self.root)
        return out

    def resolve(self, mod, segs):
        """(module, rest of the path) for a use path; module None = another crate"""
        cur, k = mod, 0
        if not segs:
            return mod, []
        if segs[0] == "crate":
            cur, k = self.root, 1
        elif segs[0] == "self":
            k = 1
        elif segs[0] == "super":
            while k < len(segs) and segs[k] == "super":
                cur = cur.parent or cur
                k += 1
        elif segs[0] not in mod.children:
            return None, segs
        while k < len(segs) and segs[k] in cur.children:
            cur = cur.children[segs[k]]
            k += 1
        return cur, segs[k:]

    def visit_ns(self, mod):
        if id(mod) not in self.ns:
            self.ns.add(id(mod))
            self.flatten(mod, False)

    def flatten(self, mod, via_use):
        """mod's public names are visible from outside (via_use: through a glob pub use)"""
        key = (id(mod), via_use)
        if key in self.done:
            return
        self.done.add(key)
        for it in mod.items:
            if it.vis == "pub" or is_exported_macro(it):
                self.reach.add(id(it))
                if via_use:
                    self.rexp.add(id(it))
        for ch in mod.children.values():
            if ch.vis == "pub":
                self.visit_ns(ch)
        for vis, trees, text in mod.uses:
            if vis != "pub":
                continue
            ext = False
            for segs, alias, glob in trees:
                target, rest = self.resolve(mod, segs)
                if target is None:
                    ext = True
                elif glob and not rest:
                    self.flatten(target, True)
                elif not glob and len(rest) == 1:
                    self.mark(target, rest[0])
                elif not glob and not rest:
                    self.visit_ns(target)
            if ext and not via_use and (mod, text) not in self.external:
                self.external.append((mod, text))

    def mark(self, mod, name, depth=0):
        """the item `name` of mod is re-exported"""
        found = False
        for it in mod.items:
            if it.name == name:
                self.reach.add(id(it))
                self.rexp.add(id(it))
                found = True
        if name in mod.children:
            self.visit_ns(mod.children[name])
            found = True
        if found or depth > 8:
            return
        for vis, trees, text in mod.uses:
            for segs, alias, glob in trees:
                target, rest = self.resolve(mod, segs)
                if target is None:
                    continue
                if glob and not rest:
                    self.mark(target, name, depth + 1)
                elif not glob and len(rest) == 1 and (alias or rest[0]) == name:
                    self.mark(target, rest[0], depth + 1)

    def file_list(self):
        skip = SKIP_FILES.get(self.name, set())
        return [f for f in self.files if f.path.name not in skip]


_CRATES = {}


def load_crate(name):
    """Crate(name), parsed once per process (generate() and tools/apidiff.py share it)"""
    if name not in _CRATES:
        _CRATES[name] = Crate(name)
    return _CRATES[name]


def impl_type(sig):
    """'impl<'a> Sp<'a>' -> ('Sp', False); 'impl Style for WinUi' -> ('WinUi', True)"""
    s = re.sub(r"^(unsafe\s+)?impl\s*", "", sig)
    if s.startswith("<"):
        depth = 0
        for k, ch in enumerate(s):
            depth += (ch == "<") - (ch == ">")
            if depth == 0:
                s = s[k + 1:].strip()
                break
    trait = re.search(r"\bfor\b", s) is not None
    if trait:
        s = re.split(r"\bfor\b", s, maxsplit=1)[1].strip()
    s = re.sub(r"<.*", "", s.split(" where ")[0]).strip()
    return s.split("::")[-1].strip("&* "), trait


def public_methods(crate, it):
    """the members of an inherent impl block of a public type that are pub"""
    ty, trait = impl_type(it.sig)
    if trait or ty not in crate.types:
        return []
    return [m for m in it.members if m.vis == "pub" and m.kind in ("fn", "const", "type")]


# ---------------------------------------------------------------- markdown
def esc(text):
    """markdown-escape doc text outside `code` spans: * and < (emphasis, HTML), _ at the
    start of a word, backslashes"""
    parts = text.split("`")
    if len(parts) % 2 == 0:                     # an unpaired backtick: escape everything
        parts = [text.replace("`", "\\`")]
    for k in range(0, len(parts), 2):
        s = parts[k].replace("\\", "\\\\").replace("*", "\\*").replace("<", "&lt;")
        parts[k] = re.sub(r"(?<![A-Za-z0-9_\\])_(?=[A-Za-z0-9])", r"\\_", s)
    return "`".join(parts)


def code(s):
    """an inline code span holding s"""
    return f"`` {s} ``" if "`" in s else f"`{s}`"


def doc_md(doc, indent):
    """doc lines as markdown under a list item: prose lines escaped (markdown joins them),
    lines indented by two or more blanks (examples, aligned tables) as a text block"""
    out, block = [], []

    def flush():
        if block:
            cut = min(len(b) - len(b.lstrip(" ")) for b in block)
            out.append(indent + "```text")
            out.extend(indent + b[cut:] for b in block)
            out.append(indent + "```")
            block.clear()

    for d in doc:
        if d.startswith("  ") and d.strip():
            block.append(d.rstrip())
            continue
        flush()
        out.append((indent + esc(d.strip())) if d.strip() else "")
    flush()
    while out and not out[-1]:
        out.pop()
    return out


def rel(p):
    return p.relative_to(ROOT).as_posix()


def link(p):
    """a link from docs/API.md to a source file"""
    return f"[`{rel(p)}`](../{rel(p)})"


def cut_value(sig):
    """'X: T = long value' -> 'X: T = long va ...'"""
    m = re.match(r"(.*?[^=!<>])=(?!=)\s*(.*)$", sig)
    if not m:
        return sig
    val = m.group(2)
    if len(val) > VALUE_MAX:
        val = val[:VALUE_MAX - 3].rstrip() + " ..."
    return f"{m.group(1).rstrip()} = {val}"


def show_sig(it):
    s = re.sub(r"^pub(\s*\([^)]*\))?\s+", "", it.sig)
    if it.kind == "macro":
        return f"{it.name}!(...)"
    if it.kind == "static":
        return re.sub(r"\s*=.*$", "", s)
    if it.kind == "const":
        return cut_value(s)
    return s


def summary_line(f):
    """the first sentence-ish of a module header without 'name -- ' and the [asm source]"""
    if not f.header:
        return ""
    parts = [f.header[0]]
    for ln in f.header[1:]:
        if not ln.strip() or ln.startswith(" "):
            break
        parts.append(ln)
    s = " ".join(p.strip() for p in parts)
    s = re.sub(r"^\w+\s+--\s+", "", s)
    s = re.sub(r"\s*\[[^\]]*\]", "", s)
    s = squash(s)
    if len(s) > 160:
        s = s[:157].rsplit(" ", 1)[0] + " ..."
    return s


class Writer:
    def __init__(self):
        self.md = []
        self.slugs = set()
        self.missing = []
        self.count = 0

    def p(self, *lines):
        self.md.extend(lines)

    def heading(self, level, text):
        """a heading -> its GitHub anchor"""
        s = re.sub(r"[^\w\- ]", "", text.replace("`", "").strip().lower()).replace(" ", "-")
        base, n = s, 0
        while s in self.slugs:
            n += 1
            s = f"{base}-{n}"
        self.slugs.add(s)
        self.p("#" * level + " " + text, "")
        return s

    def note(self, f, what, documented):
        self.count += 1
        if not documented:
            self.missing.append(f"{rel(f.path)}: {what}")

    # ---- items
    def items(self, crate, f, items, show):
        """the items of one module that `show(item)` accepts, in source order"""
        run = []
        for it in items:
            if it.kind == "impl":
                ms = public_methods(crate, it)
                if not ms:
                    continue
                self.consts(f, run)
                self.p(f"**{code(re.sub(r'^unsafe ', '', it.sig))}**", "")
                for m in ms:
                    self.item(f, m)
                self.p("")
            elif show(it):
                if it.kind == "const":
                    run.append(it)
                else:
                    self.consts(f, run)
                    self.item(f, it)
                    self.p("")
        self.consts(f, run)

    def consts(self, f, run):
        """a run of consts as rust code blocks, split at separator titles"""
        if not run:
            return
        block, title = [], None
        for it in run:
            if it.title and it.title != title:
                if block:
                    self.p("```rust", *block, "```", "")
                    block = []
                title = it.title
                self.p(f"*{esc(title)}*", "")
            block += ["// " + n if n else "//" for n in it.notes]
            block += ["/// " + d if d else "///" for d in it.doc]
            line = cut_value(re.sub(r"^(pub(\s*\([^)]*\))?\s+)?const\s+", "", it.sig))
            block.append(line + ("  // " + it.trailing if it.trailing else ""))
            self.note(f, it.name, it.doc or it.trailing or it.notes)
        self.p("```rust", *block, "```", "")
        run.clear()

    def item(self, f, it):
        self.note(f, it.name or it.sig, it.doc or it.trailing)
        cfg = f" *({it.cfg})*" if it.cfg and it.cfg != f.cfg else ""
        self.p(f"- **{code(show_sig(it))}**{cfg}")
        self.p(*doc_md(it.doc or ([it.trailing] if it.trailing else []), "  "))
        if it.kind == "trait":
            for m in it.members:
                if m.kind in ("fn", "const", "type"):
                    self.note(f, f"{it.name}::{m.name}", m.doc)
                    self.p(f"  - {code(re.sub(r'^pub\s+', '', m.sig))}")
                    self.p(*doc_md(m.doc, "    "))
        elif it.kind in ("struct", "union", "enum"):
            for vis, text, doc, trailing in it.fields:
                if vis != "pub":
                    continue
                d = doc + ([trailing] if trailing else [])
                self.p(f"  - {code(text)}" + (f" -- {esc(d[0].strip())}" if d else ""))
                self.p(*doc_md(d[1:], "    "))

    # ---- sections
    def module_body(self, crate, f, show, header=True):
        """header block, re-exports of other crates, items, inline modules"""
        if header and f.header:
            self.p("```text", *f.header, "```", "")
        for m, text in crate.external:
            if m is f:
                self.p(f"Re-exports: {code(re.sub(r'^pub use ', '', text))}", "")
        self.items(crate, f, f.items, show)
        for sub in f.inline:
            if id(sub) not in crate.ns:
                continue
            self.p(f"**{code('mod ' + sub.name)}**" + (f" *({sub.cfg})*" if sub.cfg and sub.cfg != f.cfg else ""),
                   "")
            if sub.item is not None and sub.item.doc:
                self.p(*doc_md(sub.item.doc, ""), "")
            self.module_body(crate, sub, show, header=False)

    def file_section(self, crate, f, level, show):
        name = f"`{crate.name}/{f.path.relative_to(LIB / crate.name).as_posix()}`"
        slug = self.heading(level, name + (f" ({f.cfg})" if f.cfg else ""))
        self.p(f"Source: {link(f.path)}", "")
        self.module_body(crate, f, show, header=f is not crate.root)
        return slug


def has_public(crate, f):
    for it in f.items:
        if id(it) in crate.reach or (it.kind == "impl" and public_methods(crate, it)):
            return True
    return any(m is f for m, _ in crate.external) or any(id(s) in crate.ns and has_public(crate, s)
                                                       for s in f.inline)


def ui_section(w, crate):
    files = crate.file_list()
    byname = {f.path.name: f for f in files}
    reach = lambda it: id(it) in crate.reach  # noqa: E731
    w.p("**`Ui` methods by area** (signatures and descriptions in the sections below):", "")
    glance_at = len(w.md)
    w.p("")
    entries, glance, done = [], [], set()
    for title, names in UI_AREAS:
        fs = [byname[n] for n in names if n in byname]
        if not fs:
            continue
        done.update(n for n in names)
        slug = w.heading(3, title)
        entries.append((title, slug))
        methods = [m.name for f in fs for it in f.items if it.kind == "impl" and impl_type(it.sig)[0] == "Ui"
                   for m in public_methods(crate, it) if m.kind == "fn"]
        if methods:
            glance.append(f"- [{title}](#{slug}): " + ", ".join(f"`{m}`" for m in methods))
        if len(fs) == 1:
            w.p(f"Source: {link(fs[0].path)}", "")
            w.module_body(crate, fs[0], reach, header=fs[0] is not crate.root)
        else:
            for f in fs:
                w.file_section(crate, f, 4, reach)
    rest = [f for f in files if f.path.name not in done and has_public(crate, f)]
    if rest:
        entries.append(("other", w.heading(3, "Other modules")))
        for f in rest:
            w.file_section(crate, f, 4, reach)
    w.md[glance_at:glance_at] = glance
    return entries


def codec_section(w, crate):
    extra = CODEC_EXTRA.get(crate.name, {})
    root = crate.root
    entries = [("API", w.heading(3, f"`{crate.name}` API"))]
    w.module_body(crate, root, lambda it: id(it) in crate.reach, header=False)
    mods = []
    for f in crate.file_list():
        if f is root:
            continue
        want = extra.get(f.name, [])

        def show(it, want=want):
            return id(it) in crate.rexp or (id(it) in crate.reach and (want == "*" or it.name in want))
        if any(show(it) for it in f.items):
            where = f" (`{crate.name}::{f.name}`)" if id(f) in crate.ns else ""
            w.p(f"From {link(f.path)}{where}:", "")
            w.items(crate, f, f.items, show)
        if id(f) in crate.ns and f.parent is root:
            mods.append(f)
    if mods:
        entries.append(("modules", w.heading(3, f"`{crate.name}` modules")))
        w.p("The codec's building blocks (public for the codec test app `codectest`; see their "
            "source headers):", "")
        w.p("| Module | Content |", "|---|---|")
        for f in mods:
            w.p(f"| [`{crate.name}::{f.name}`](../{rel(f.path)}) | {esc(summary_line(f)).replace('|', '/')} |")
        w.p("")
    return entries


def generate():
    w = Writer()
    w.p("# jfguiLib API reference", "",
        "Generated by `python tools/gendoc.py` from the doc comments of `lib` -- do not edit by "
        "hand; `python tools/gendoc.py --check` tells whether this file is up to date.", "",
        "Every crate is `#![no_std]`. An app depends on `ui` (which brings `gfx`, `text`, `plat`, "
        "`base`, `img_core` and the codecs), links one style crate as `style` and calls `ui::run` "
        "(docs/GUIDE.md). Colours are `0xAARRGGBB`, straight alpha in the API, premultiplied "
        "(`FMT_PARGB`) in images. UI sizes are DIP (96 dpi = 100 %), drawing works in pixels. "
        "A name in a description such as `ui_box_begin` or `[img_save]` is the assembly routine the "
        "function was ported from; the algorithm specs in `legacy/ASMLIB/docs/modules/*.md` describe "
        "both versions.", "",
        "Not listed here: `simd` (the SSE2 / AVX2 kernel layer, see `README.md`, \"SIMD kernels\"), "
        "the style crates (docs/STYLES.md), plugins (`plugins/README.md`).", "")
    toc_at = len(w.md) + 2
    w.heading(2, "Contents")
    toc = []
    for name, title, kind in CRATES:
        crate = load_crate(name)
        slug = w.heading(2, name)
        w.p(f"*{title}* -- crate root {link(crate.root.path)}", "")
        if crate.root.header:
            w.p("```text", *crate.root.header, "```", "")
        if name == "ui":
            entries = ui_section(w, crate)
        elif kind == "codec":
            entries = codec_section(w, crate)
        else:
            entries = []
            reach = lambda it, c=crate: id(it) in c.reach  # noqa: E731
            for f in crate.file_list():
                if not has_public(crate, f):
                    continue
                if f is crate.root:
                    entries.append(("lib.rs", w.heading(3, f"`{name}` crate root")))
                    w.module_body(crate, f, reach, header=False)
                else:
                    label = f.path.relative_to(LIB / name).as_posix() + (f" ({f.cfg})" if f.cfg else "")
                    entries.append((label, w.file_section(crate, f, 3, reach)))
        toc.append(f"- [{name}](#{slug}) -- {title}")
        if entries:
            toc.append("  <br>" + " · ".join(f"[{label}](#{s})" for label, s in entries))
    w.md[toc_at:toc_at] = toc + [""]
    text = re.sub(r"\n{3,}", "\n\n", "\n".join(w.md).rstrip() + "\n")
    return text, w


def main():
    args = sys.argv[1:]
    if "-h" in args or "--help" in args:
        print(__doc__)
        return 0
    text, w = generate()
    data = text.encode("utf-8")
    if "--check" in args:
        old = OUT.read_bytes().replace(b"\r\n", b"\n") if OUT.exists() else b""
        stale = old != data
        print(f"{rel(OUT)}: {'OUT OF DATE -- run python tools/gendoc.py' if stale else 'up to date'} "
              f"({w.count} items, {len(w.missing)} without a comment)")
        rc = 1 if stale else 0
    else:
        OUT.parent.mkdir(parents=True, exist_ok=True)
        OUT.write_bytes(data)
        print(f"wrote {rel(OUT)}: {len(data)} bytes, {w.count} items, {len(w.missing)} without a comment")
        rc = 0
    if "--missing" in args:
        for m in w.missing:
            print("  " + m)
    if "--api" in args:                         # the API since the last release (apidiff.py)
        import apidiff
        base, path = apidiff.baseline()
        if base is None:
            print("API: no release snapshot in docs/api yet (python build_rs.py api-diff --snapshot)")
        else:
            print(apidiff.summary(apidiff.diff(apidiff.read_snapshot(path), apidiff.current_lines()), base))
    return rc


if __name__ == "__main__":
    sys.exit(main())
