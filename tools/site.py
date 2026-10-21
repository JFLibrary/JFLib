#!/usr/bin/env python3
"""site.py -- the static website of JFLib: a landing page, the docs and the API reference

    python build_rs.py site [--out DIR]     write the site (default build/site/; open index.html)
    python build_rs.py site --check         build it into a temporary folder below build/ and check it
    python tools/site.py [--out DIR] [--check] [--summary]
                                            the same without build_rs.py (--summary: one line,
                                            as `build_rs.py test` prints it)

What the site holds (the table DOCS below sets the pages, their titles, groups and languages):
  * index.html: the landing page, read from README.md's sections (the introduction, "The
    library", "Feature tour", "Platforms", the quick starts of "Building apps with Python",
    "Documentation", "License"); when one of them is missing the build stops.
  * one page per Markdown file of DOCS at its path with .html (docs/STYLES.md ->
    docs/STYLES.html; README.md -> README.html).  German pages say so above their text.
  * docs/GUIDE.md: docs/GUIDE.html (introduction and contents) and one page per "## N." section,
    docs/guide/NN-title.html.
  * docs/API.md (tools/gendoc.py): docs/API.html (its contents) and one page per crate,
    docs/api/CRATE.html; a crate with more than API_PAGE_LIMIT bytes of Markdown gets one page
    per ### section (docs/api/CRATE--SECTION.html), such a section above the limit one per ####
    file.  Every item gets an anchor: #fn.run, #struct.Ui, #Ui.text (a method of `impl Ui`).
  * assets/: site.css, site.js, favicon.svg and search-index.js (page titles, headings and API
    items as JSON, loaded when the search field is first used; a script, so that the search also
    works from file://).  The images the pages show are copied to their repository paths.
Every page has the top bar (search, light / dark toggle stored in localStorage, else the
system's prefers-color-scheme), the navigation at the side (a drawer at phone width), "on this
page" headings on wide screens, previous / next links, a <title> and a description.  No
external fonts, scripts or styles.

Links: the GitHub heading anchors (the slug rule of gendoc and GitHub) keep working on the split
pages; a link to a .md file of the site goes to its page and anchor; any other file or folder of
the repository goes to GitHub (REPO/blob/main/PATH, folders /tree/main/; #L12 line anchors as
?plain=1#L12); external links stay.  A link that leaves the repository is shown as text.

Markdown: what the repository's docs use -- ATX headings, paragraphs, **strong**, *emphasis*,
`code` (also across lines), inline links and images, bare https:// URLs, backslash escapes,
entities, GFM tables (alignment, \\| in cells), fenced code (also inside list items), nested
ordered and unordered lists (tight and loose, lazy continuation lines), block quotes, rules,
and the inline HTML <br>, <img src alt width height> and <a id="..."></a>.  Any other HTML is
shown as text.  Code blocks get colours for rust, sh / powershell, json and js.

--check (`build_rs.py test` runs it): the converter's own cases; then a build into a temporary
folder (or --out) and: every page of DOCS, every guide section and every crate has its page;
every internal link and anchor resolves, also every entry of the search index; every image,
script and stylesheet exists; links to
repository files point at files that exist (with the exact case), their #anchors at headings
there; the HTML nests correctly (html.parser); every page has a <title>, a description and a
language; no absolute local path (the checkout, the home folder, the output folder) is in the
output; a second build gives the same bytes.  Exit code 1 on any failure.

The output folder is emptied first, but only when it is empty or holds a site of this tool (the
marker file .jflib-site); nothing is written outside it.  Python standard library only.
"""
import argparse
import hashlib
import html
import json
import os
import posixpath
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import time
from html.entities import html5 as ENTITIES
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import quote, unquote

ROOT = Path(__file__).resolve().parent.parent
REPO = "https://github.com/JFLibrary/JFLib"
BRANCH = "main"
MARKER = ".jflib-site"
API_PAGE_LIMIT = 64 * 1024              # bytes of Markdown per API page before it is split
IMAGE_TYPES = (".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp")
NOT_IN_REPO = ("build",)                # git-ignored: a link there is broken on GitHub

# ---------------------------------------------------------------- configuration
# (source, title in the navigation, group, language of the text); GUIDE.md and API.md are split
DOCS = [
    ("README.md", "README", "Start", "en"),
    ("docs/GUIDE.md", "Developer guide", "Guide", "en"),
    ("docs/API.md", "API reference", "Reference", "en"),
    ("docs/STYLES.md", "Styles", "Reference", "en"),
    ("docs/VFS.md", "Virtual files and assets", "Reference", "en"),
    ("docs/PERMISSIONS.md", "Permissions", "Reference", "en"),
    ("docs/SECRETS.md", "Secrets", "Reference", "en"),
    ("docs/PACKAGING.md", "Packaging", "Reference", "en"),
    ("docs/VERSIONING.md", "Versions", "Reference", "en"),
    ("docs/UPDATES.md", "Updates", "Reference", "en"),
    ("docs/AGENT_WEB.md", "Web and agent control", "Reference", "de"),
    ("docs/WASM_COMPUTE.md", "WASM compute", "Reference", "de"),
    ("apps/Kritzel/README.md", "Kritzel", "Apps", "en"),
    ("apps/PaintCraft/README.md", "PaintCraft", "Apps", "en"),
    ("plugins/README.md", "PaintCraft plugins", "Apps", "en"),
    ("apps/jfWord/README.md", "jfWord", "Apps", "de"),
    ("apps/buildManager/README.md", "Build Manager", "Apps", "en"),
    ("apps/BackupCraft/README.md", "BackupCraft", "Apps", "en"),
    ("apps/JFNetworker/README.md", "JFNetworker", "Apps", "de"),
    ("apps/androidGallery/README.md", "Android Gallery", "Apps", "de"),
    ("styles/jfoffice/README.md", "jfOffice style", "Apps", "de"),
    ("CHANGELOG.md", "Changelog", "Project", "en"),
    ("docs/ROADMAP.md", "Roadmap", "Project", "de"),
    ("THIRD_PARTY_NOTICES.md", "Third-party notices", "Project", "en"),
]
GROUPS = ["Start", "Guide", "Reference", "Apps", "Project"]
LANGUAGES = {"de": "German"}


class SiteError(Exception):
    pass


def esc(s):
    return html.escape(s, quote=False)


def attr(s):
    return html.escape(s, quote=True)


# ---------------------------------------------------------------- Markdown: blocks
FENCE = re.compile(r"^( {0,3})(`{3,}|~{3,})(.*)$")
ATX = re.compile(r"^ {0,3}(#{1,6})(?:[ \t]+(.*?))?(?:[ \t]+#+)?[ \t]*$")
HR = re.compile(r"^ {0,3}([-*_])(?:[ \t]*\1){2,}[ \t]*$")
ITEM = re.compile(r"^( {0,3})([-*+]|\d{1,9}[.)])(?:( +)(.*))?$")
DELIM = re.compile(r"^ {0,3}\|?[ \t]*:?-+:?[ \t]*(?:\|[ \t]*:?-+:?[ \t]*)*\|?[ \t]*$")
QUOTE = re.compile(r"^ {0,3}> ?(.*)$")
BLOCK_START = frozenset("`~#-*_+>0123456789")      # the first characters of the blocks above


def indent(s):
    return len(s) - len(s.lstrip(" "))


def table_start(lines, i):
    return ("|" in lines[i] and i + 1 < len(lines) and "|" in lines[i + 1] and DELIM.match(lines[i + 1])
            and len(cells(lines[i])) == len(cells(lines[i + 1])))


def cells(row):
    """a table row -> its cells ("\\|" is a | inside a cell, also inside `code`)"""
    s = row.strip()
    if s.startswith("|"):
        s = s[1:]
    if s.endswith("|") and not s.endswith("\\|"):
        s = s[:-1]
    return [c.strip().replace("\\|", "|") for c in re.split(r"(?<!\\)\|", s)]


def interrupts(lines, i):
    """does lines[i] end a paragraph (a block that may interrupt one starts there)?"""
    ln = lines[i]
    s = ln.lstrip(" ")
    if not s or (s[0] not in BLOCK_START and "|" not in ln):
        return False
    if FENCE.match(ln) or ATX.match(ln) or HR.match(ln) or QUOTE.match(ln) or table_start(lines, i):
        return True
    m = ITEM.match(ln)
    return bool(m and (m.group(4) or "").strip() and (m.group(2) in "-*+" or m.group(2)[:-1] == "1"))


def parse_blocks(lines):
    """Markdown lines -> (blocks, gap); gap: a blank line stands between two of the blocks.
    A block is a dict: t = h (level, text), p (text), code (lang, text), list (ordered, start,
    items [{blocks, blank_before}], loose), table (aligns, head, rows), hr, quote (blocks)."""
    blocks, gap, blank = [], False, False
    i, n = 0, len(lines)
    while i < n:
        ln = lines[i]
        if not ln.strip():
            blank = True
            i += 1
            continue
        if blank and blocks:
            gap = True
        blank = False
        m = FENCE.match(ln)
        if m and not (m.group(2)[0] == "`" and "`" in m.group(3)):
            ind, fence, body = len(m.group(1)), m.group(2), []
            i += 1
            while i < n:
                c = lines[i]
                s = c.lstrip(" ")
                if indent(c) <= 3 and s.startswith(fence) and not s.rstrip().strip(fence[0]):
                    i += 1
                    break
                body.append(c[min(ind, indent(c)):])
                i += 1
            info = m.group(3).strip()
            blocks.append({"t": "code", "lang": info.split()[0].lower() if info else "", "text": "\n".join(body)})
            continue
        m = ATX.match(ln)
        if m:
            blocks.append({"t": "h", "level": len(m.group(1)), "text": (m.group(2) or "").strip()})
            i += 1
            continue
        if HR.match(ln):
            blocks.append({"t": "hr"})
            i += 1
            continue
        if table_start(lines, i):
            aligns = []
            for c in cells(lines[i + 1]):
                aligns.append("center" if c.startswith(":") and c.endswith(":") else
                              "right" if c.endswith(":") else "left" if c.startswith(":") else "")
            head, rows = cells(ln), []
            i += 2
            while i < n and lines[i].strip() and "|" in lines[i] and not interrupts(lines, i):
                r = cells(lines[i])
                rows.append((r + [""] * len(head))[:len(head)])
                i += 1
            blocks.append({"t": "table", "aligns": aligns, "head": head, "rows": rows})
            continue
        if QUOTE.match(ln):
            q = []
            while i < n and lines[i].strip():
                m = QUOTE.match(lines[i])
                q.append(m.group(1) if m else lines[i].strip())
                i += 1
            blocks.append({"t": "quote", "blocks": parse_blocks(q)[0]})
            continue
        if ITEM.match(ln):
            i = parse_list(lines, i, blocks)
            continue
        para = [ln.strip()]
        i += 1
        while i < n and lines[i].strip() and not interrupts(lines, i):
            para.append(lines[i].strip())
            i += 1
        blocks.append({"t": "p", "text": "\n".join(para)})
    return blocks, gap


def parse_list(lines, i, blocks):
    """the list starting at lines[i] -> blocks; returns the index after it"""
    n = len(lines)
    first = ITEM.match(lines[i]).group(2)
    ordered = first[-1] in ".)"
    key = first[-1] if ordered else first

    def sibling(ln):
        m = ITEM.match(ln)
        return m and (m.group(2)[-1] if m.group(2)[-1] in ".)" else m.group(2)) == key

    items, loose, blank_before = [], False, False
    while i < n and sibling(lines[i]):
        m = ITEM.match(lines[i])
        ind, mk, sp, rest = len(m.group(1)), m.group(2), m.group(3) or "", m.group(4) or ""
        if not rest.strip():
            w, body = ind + len(mk) + 1, [""]
        elif len(sp) > 4:
            w, body = ind + len(mk) + 1, [" " * (len(sp) - 1) + rest]
        else:
            w, body = ind + len(mk) + len(sp), [rest]
        fenced = bool(FENCE.match(body[0]))
        i += 1
        while i < n:
            c = lines[i]
            if not c.strip():
                body.append("")
            elif indent(c) >= w:
                body.append(c[w:])
                if FENCE.match(c[w:].lstrip()):
                    fenced = not fenced
            elif body[-1] == "" or fenced or ITEM.match(c) or interrupts(lines, i):
                break
            else:
                body.append(c.strip())            # a lazy continuation line
            i += 1
        trailing = 0
        while body and not body[-1]:
            body.pop()
            trailing += 1
        i -= trailing
        sub, gap = parse_blocks(body)
        items.append({"blocks": sub, "blank_before": blank_before})
        loose = loose or gap or (blank_before and len(items) > 1)
        j = i
        while j < n and not lines[j].strip():
            j += 1
        if j < n and sibling(lines[j]):
            blank_before = j > i
            i = j
        else:
            break
    start = int(first[:-1]) if ordered else 1
    blocks.append({"t": "list", "ordered": ordered, "start": start, "items": items, "loose": loose})
    return i


def walk(blocks):
    for b in blocks:
        yield b
        if b["t"] == "list":
            for it in b["items"]:
                yield from walk(it["blocks"])
        elif b["t"] == "quote":
            yield from walk(b["blocks"])


def texts(b):
    """the inline Markdown of one block (not of its children)"""
    if b["t"] in ("p", "h"):
        return [b["text"]]
    if b["t"] == "table":
        return b["head"] + [c for r in b["rows"] for c in r]
    return []


def size(blocks):
    return sum(len(t) for b in walk(blocks) for t in texts(b) + [b.get("text", "") if b["t"] == "code" else ""])


def split_at(blocks, level):
    """blocks -> (the blocks before the first heading of `level`, [the sections from each one])"""
    head, secs = [], []
    for b in blocks:
        if b["t"] == "h" and b["level"] == level:
            secs.append([b])
        elif secs:
            secs[-1].append(b)
        else:
            head.append(b)
    return head, secs


# ---------------------------------------------------------------- Markdown: inline
PUNCT = frozenset("!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~")
ENTITY = re.compile(r"&(?:#[0-9]{1,7}|#[xX][0-9a-fA-F]{1,6}|[A-Za-z][A-Za-z0-9]{1,31});")
TAG = re.compile(r"<(/?)([A-Za-z][A-Za-z0-9]*)((?:\s+[A-Za-z_:][-\w:.]*(?:\s*=\s*(?:\"[^\"]*\"|'[^']*'|[^\s\"'=<>`]+))?)*)\s*/?>")
ATTR = re.compile(r"([A-Za-z_:][-\w:.]*)(?:\s*=\s*(?:\"([^\"]*)\"|'([^']*)'|([^\s\"'=<>`]+)))?")
ANCHOR_TAG = re.compile(r"<a\s+(?:id|name)\s*=\s*[\"']([^\"']+)[\"']\s*>\s*</a>")
BARE_URL = re.compile(r"(?<![\w/])https?://[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+(?::\d+)?(?:[/?#][^\s<>\ue000\ue001]*)?")
STRONG = re.compile(r"\*\*(?=[^\s*])(.+?)(?<=[^\s*])\*\*", re.S)
EM = re.compile(r"(?<![*\w\\])\*(?=[^\s*])(.+?)(?<=[^\s*])\*(?![*\w])", re.S)
SLOT = re.compile("\ue000(\\d+)\ue001")
CLOSERS = {}


def closer(n):
    if n not in CLOSERS:
        CLOSERS[n] = re.compile("(?<!`)" + "`" * n + "(?!`)")
    return CLOSERS[n]


class NoTargets:
    """link targets for plain text: links become their text, images their alt text"""
    def href(self, url):
        return None

    def image(self, url):
        return None


class Inline:
    """inline Markdown -> HTML.  ctx.href(url) -> href or None (text only), ctx.image(url) ->
    (src, width, height) or None.  Code spans, escapes, entities, allowed tags, links and images
    become slots (private-use characters) first, so that escaping and emphasis never touch them."""

    def __init__(self, ctx):
        self.ctx = ctx

    def __call__(self, text, links=True):
        self.slots = []
        s = self.finish(self.spans(self.scan(text), links), links)
        while "\ue000" in s:
            s = SLOT.sub(lambda m: self.slots[int(m.group(1))][0], s)
        return s

    def keep(self, h, raw=""):
        self.slots.append((h, raw))
        return "\ue000%d\ue001" % (len(self.slots) - 1)

    def raw(self, s):
        """s with its slots back as source text (link targets, alt texts)"""
        return SLOT.sub(lambda m: self.raw(self.slots[int(m.group(1))][1]), s)

    def scan(self, t):
        out, i, n = [], 0, len(t)
        while i < n:
            c = t[i]
            if c == "\\" and i + 1 < n and t[i + 1] in PUNCT:
                out.append(self.keep(esc(t[i + 1]), t[i + 1]))
                i += 2
            elif c == "`":
                j = i
                while j < n and t[j] == "`":
                    j += 1
                m = closer(j - i).search(t, j)
                if m:
                    code = t[j:m.start()].replace("\n", " ")
                    if len(code) > 1 and code[0] == code[-1] == " " and code.strip():
                        code = code[1:-1]
                    out.append(self.keep("<code>" + esc(code) + "</code>", code))
                    i = m.end()
                else:
                    out.append(self.keep(t[i:j], t[i:j]))
                    i = j
            elif c == "&":
                m = ENTITY.match(t, i)
                if m and (m.group()[1] == "#" or m.group()[1:] in ENTITIES):
                    out.append(self.keep(m.group(), html.unescape(m.group())))
                    i = m.end()
                else:
                    out.append(c)
                    i += 1
            elif c == "<":
                m = TAG.match(t, i)
                h = self.tag(m, t) if m else None
                if h is not None:
                    out.append(self.keep(h[0]))
                    i = h[1]
                else:
                    out.append(c)
                    i += 1
            else:
                j = i + 1
                while j < n and t[j] not in "\\`&<":
                    j += 1
                out.append(t[i:j])
                i = j
        return "".join(out)

    def tag(self, m, t):
        """an allowed HTML tag -> (html, end), else None (shown as text)"""
        name = m.group(2).lower()
        if m.group(1):
            return None
        if name == "br":
            return "<br>", m.end()
        a = {k.lower(): v1 or v2 or v3 for k, v1, v2, v3 in ATTR.findall(m.group(3))}
        if name == "a" and "href" not in a and (a.get("id") or a.get("name")) and t.startswith("</a>", m.end()):
            ident = a.get("id") or a.get("name")
            if ident in getattr(self.ctx, "heading_ids", ()):
                return "", m.end() + 4                  # the heading next to it has that anchor already
            return '<a id="%s"></a>' % attr(ident), m.end() + 4
        if name == "img" and a.get("src"):
            return self.img(a["src"], a.get("alt", ""), a.get("title"), a.get("width"), a.get("height")), m.end()
        return None

    def img(self, url, alt, title=None, width=None, height=None):
        r = self.ctx.image(url)
        if r is None:
            return esc(alt)
        src, iw, ih = r
        if width and not height and width.isdigit() and iw:
            height = str(round(ih * int(width) / iw))
        elif not width and iw:
            width, height = str(iw), str(ih)
        h = '<img src="%s" alt="%s"' % (attr(src), attr(alt))
        h += (' width="%s"' % attr(width) if width else "") + (' height="%s"' % attr(height) if height else "")
        h += (' title="%s"' % attr(title) if title else "")
        return h + ' loading="lazy" decoding="async">'

    def spans(self, s, links):
        """links and images"""
        out, i = [], 0
        while True:
            j = s.find("[", i)
            if j < 0:
                out.append(s[i:])
                return "".join(out)
            end = self.bracket(s, j)
            d = self.dest(s, end + 2) if end > 0 and s.startswith("(", end + 1) else None
            if d is None:
                out.append(s[i:j + 1])
                i = j + 1
                continue
            url, title, after = d
            label = s[j + 1:end]
            if j - 1 >= i and s[j - 1] == "!":
                out.append(s[i:j - 1])
                out.append(self.keep(self.img(self.raw(url), self.text(label), title and self.raw(title))))
            else:
                out.append(s[i:j])
                inner = self.finish(self.spans(label, False), False)
                href = self.ctx.href(self.raw(url)) if links else None
                if href is None:
                    out.append(self.keep(inner))
                else:
                    t = ' title="%s"' % attr(self.raw(title)) if title else ""
                    out.append(self.keep('<a href="%s"%s>%s</a>' % (attr(href), t, inner)))
            i = after

    @staticmethod
    def bracket(s, j):
        depth = 0
        for k in range(j, len(s)):
            if s[k] == "[":
                depth += 1
            elif s[k] == "]":
                depth -= 1
                if not depth:
                    return k
        return -1

    @staticmethod
    def dest(s, k):
        """the (url "title") of a link from s[k] -> (url, title, index after ")") or None"""
        n = len(s)
        while k < n and s[k] in " \n":
            k += 1
        if k < n and s[k] == "<":
            e = s.find(">", k)
            if e < 0:
                return None
            url, k = s[k + 1:e], e + 1
        else:
            st, depth = k, 0
            while k < n and not s[k].isspace():
                if s[k] == "(":
                    depth += 1
                elif s[k] == ")":
                    if not depth:
                        break
                    depth -= 1
                k += 1
            url = s[st:k]
        while k < n and s[k] in " \n":
            k += 1
        title = None
        if k < n and s[k] in "\"'":
            e = s.find(s[k], k + 1)
            if e < 0:
                return None
            title, k = s[k + 1:e], e + 1
            while k < n and s[k] in " \n":
                k += 1
        return (url, title, k + 1) if k < n and s[k] == ")" else None

    def text(self, s):
        """plain text of inline Markdown with slots (alt texts)"""
        h = self.finish(s, False)
        while "\ue000" in h:
            h = SLOT.sub(lambda m: self.slots[int(m.group(1))][0], h)
        return html.unescape(re.sub(r"<[^>]+>", "", h))

    def finish(self, s, links):
        if links:
            s = BARE_URL.sub(self.autolink, s)
        s = esc(s)
        s = STRONG.sub(r"<strong>\1</strong>", s)
        return EM.sub(r"<em>\1</em>", s)

    def autolink(self, m):
        url = m.group()
        cut = len(url)
        while cut and url[cut - 1] in ".,;:!?*_'\")":
            if url[cut - 1] == ")" and url[:cut].count("(") >= url[:cut].count(")"):
                break
            cut -= 1
        return self.keep('<a href="%s">%s</a>' % (attr(url[:cut]), esc(url[:cut]))) + url[cut:]


def plain(md):
    """inline Markdown -> plain text (heading slugs, descriptions, the search index)"""
    return html.unescape(re.sub(r"<[^>]+>", "", Inline(NoTargets())(md)))


def slug(text):
    """GitHub's heading anchor (gendoc.py's rule; repeated ones get -1, -2 ...)"""
    return re.sub(r"[^\w\- ]", "", text.strip().lower()).replace(" ", "-")


def fslug(text):
    return re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")[:60] or "page"


# ---------------------------------------------------------------- code colours
RUST_KW = frozenset("as async await break const continue crate dyn else enum extern false fn for if impl in let "
                    "loop match mod move mut pub ref return self Self static struct super trait true type union "
                    "unsafe use where while".split())
JS_KW = frozenset("async await break case catch class const continue default delete do else export extends false "
                  "finally for function if import in instanceof let new null of return switch this throw true try "
                  "typeof undefined var void while yield".split())
HIGHLIGHT = {
    "rust": (re.compile("|".join([
        r"(?P<c>//[^\n]*|/\*[\s\S]*?\*/)",
        r"(?P<s>(?<!\w)b?r(?P<h>#*)\"[\s\S]*?\"(?P=h)|(?<!\w)b?\"(?:\\[\s\S]|[^\"\\])*\""
        r"|(?<!\w)b?'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]{1,6}\}|[^\n])|[^'\\\n])')",
        r"(?P<l>'[A-Za-z_]\w*)",
        r"(?P<a>#!?\[[^\]\n]*\])",
        r"(?P<n>\b(?:0x[0-9A-Fa-f_]+|0b[01_]+|0o[0-7_]+|\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d+)?)"
        r"(?:[iu](?:8|16|32|64|128|size)|f32|f64)?\b)",
        r"(?P<m>\b[A-Za-z_]\w*!(?=\s*[(\[{]))",
        r"(?P<i>\b[A-Za-z_]\w*\b)"])), RUST_KW),
    "sh": (re.compile(r"(?P<c>(?<!\S)#[^\n]*)|(?P<s>\"(?:\\.|[^\"\\\n])*\"|'[^'\n]*')"), frozenset()),
    "json": (re.compile(r"(?P<t>\"(?:\\.|[^\"\\])*\"(?=\s*:))|(?P<s>\"(?:\\.|[^\"\\])*\")"
                        r"|(?P<n>-?\b\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b)|(?P<k>\b(?:true|false|null)\b)"), frozenset()),
    "js": (re.compile(r"(?P<c>//[^\n]*|/\*[\s\S]*?\*/)|(?P<s>\"(?:\\.|[^\"\\\n])*\"|'(?:\\.|[^'\\\n])*'|`(?:\\[\s\S]|[^`\\])*`)"
                      r"|(?P<n>\b\d+(?:\.\d+)?\b)|(?P<i>\b[A-Za-z_$][\w$]*\b)"), JS_KW),
}
HIGHLIGHT.update(rs=HIGHLIGHT["rust"], bash=HIGHLIGHT["sh"], shell=HIGHLIGHT["sh"], powershell=HIGHLIGHT["sh"],
                 ps1=HIGHLIGHT["sh"], javascript=HIGHLIGHT["js"], mjs=HIGHLIGHT["js"])


def highlight(code, lang):
    rx, kw = HIGHLIGHT.get(lang, (None, None))
    if rx is None:
        return esc(code)
    out, pos = [], 0
    for m in rx.finditer(code):
        cls = m.lastgroup
        if cls == "i":
            w = m.group()
            cls = ("k" if w in kw else None) if lang not in ("rust", "rs") else \
                "k" if w in kw else "n" if w.isupper() and len(w) > 1 else "t" if w[0].isupper() else None
        if cls is None:
            continue
        out.append(esc(code[pos:m.start()]))
        out.append('<span class="%s">%s</span>' % (cls, esc(m.group())))
        pos = m.end()
    out.append(esc(code[pos:]))
    return "".join(out)


# ---------------------------------------------------------------- blocks -> HTML
def render(blocks, inl, demote=0, tight=False):
    out = []
    for b in blocks:
        t = b["t"]
        if t == "p":
            h = inl(b["text"])
            if h.strip():
                out.append(h if tight or re.fullmatch(r'<a id="[^"]*"></a>', h) else "<p>" + h + "</p>")
        elif t == "h":
            lv = max(1, min(6, b["level"] - demote))
            out.append('<h%d id="%s">%s<a class="hl" href="#%s" aria-label="Link to this section">#</a></h%d>'
                       % (lv, attr(b["id"]), inl(b["text"]), attr(b["id"]), lv))
        elif t == "code":
            cls = ' class="lang-%s"' % attr(b["lang"]) if b["lang"] else ""
            out.append('<pre class="code"><code%s>%s</code></pre>' % (cls, highlight(b["text"], b["lang"])))
        elif t == "list":
            tag = "ol" if b["ordered"] else "ul"
            a = ' start="%d"' % b["start"] if b["ordered"] and b["start"] != 1 else ""
            a += ' class="items"' if b.get("api") else ""
            lis = []
            for it in b["items"]:
                ida = ' id="%s"' % attr(it["id"]) if it.get("id") else ""
                lis.append("<li%s>%s</li>" % (ida, render(it["blocks"], inl, demote, not b["loose"])))
            out.append("<%s%s>\n%s\n</%s>" % (tag, a, "\n".join(lis), tag))
        elif t == "table":
            def row(cs, cell):
                return "<tr>" + "".join('<%s%s>%s</%s>' % (cell, ' style="text-align:%s"' % al if al else "", inl(c), cell)
                                        for c, al in zip(cs, b["aligns"])) + "</tr>"
            body = "\n".join(row(r, "td") for r in b["rows"])
            out.append('<div class="table"><table>\n<thead>%s</thead>\n<tbody>\n%s\n</tbody>\n</table></div>'
                       % (row(b["head"], "th"), body))
        elif t == "hr":
            out.append("<hr>")
        elif t == "quote":
            out.append("<blockquote>\n%s\n</blockquote>" % render(b["blocks"], inl, demote))
    return "\n".join(out)


# ---------------------------------------------------------------- the repository
LISTINGS = {}
LINE_ANCHOR = re.compile(r"L\d+(?:-L\d+)?")
EXTERNAL = re.compile(r"^(?:[A-Za-z][A-Za-z0-9+.-]*:|//)")


def repo_kind(rel):
    """'file' / 'dir' for a path of the repository (exact case, as GitHub sees it), else None"""
    if rel in ("", "."):
        return "dir"
    if rel.split("/")[0] in NOT_IN_REPO:
        return None
    p = ROOT
    for part in rel.split("/"):
        key = str(p)
        if key not in LISTINGS:
            try:
                LISTINGS[key] = frozenset(os.listdir(p))
            except OSError:
                LISTINGS[key] = frozenset()
        if part not in LISTINGS[key]:
            return None
        p = p / part
    return "dir" if p.is_dir() else "file"


def github(rel, kind="file", frag=""):
    if rel in ("", "."):
        return REPO
    url = "%s/%s/%s/%s" % (REPO, "tree" if kind == "dir" else "blob", BRANCH, quote(rel))
    if frag and LINE_ANCHOR.fullmatch(frag) and rel.lower().endswith(".md"):
        url += "?plain=1"
    return url + ("#" + frag if frag else "")


MD_ANCHORS = {}


def md_anchors(rel):
    """the heading anchors of a Markdown file of the repository that is not a page of the site"""
    if rel not in MD_ANCHORS:
        text = (ROOT / rel).read_bytes().decode("utf-8", "replace").replace("\r\n", "\n")
        seen = set(ANCHOR_TAG.findall(text))
        for b in walk(parse_blocks(text.split("\n"))[0]):
            if b["t"] == "h":
                s = base = slug(plain(b["text"]))
                k = 0
                while s in seen:
                    k += 1
                    s = "%s-%d" % (base, k)
                seen.add(s)
        MD_ANCHORS[rel] = seen
    return MD_ANCHORS[rel]


def png_size(path):
    with open(path, "rb") as f:
        head = f.read(24)
    if head[:8] == b"\x89PNG\r\n\x1a\n" and head[12:16] == b"IHDR":
        return struct.unpack(">II", head[16:24])
    return None, None


def relpath(page, target):
    """href from the page at `page` to the file `target` (both normalized, relative to the site root)"""
    a, b = page.split("/")[:-1], target.split("/")
    k = 0
    while k < len(a) and k < len(b) - 1 and a[k] == b[k]:
        k += 1
    return "../" * (len(a) - k) + "/".join(b[k:])


# ---------------------------------------------------------------- pages
class Doc:
    """one Markdown file: blocks with GitHub anchors on every heading"""

    def __init__(self, src, title, group, lang):
        self.src, self.title, self.group, self.lang = src, title, group, lang
        text = (ROOT / src).read_bytes().decode("utf-8").replace("\r\n", "\n")
        self.blocks = parse_blocks(text.split("\n"))[0]
        seen = set()
        for b in walk(self.blocks):
            if b["t"] == "h":
                b["plain"] = plain(b["text"])
                s = base = slug(b["plain"])
                k = 0
                while s in seen:
                    k += 1
                    s = "%s-%d" % (base, k)
                seen.add(s)
                b["id"] = s
        self.heading_ids = seen


class Page:
    def __init__(self, path, nav, doc, blocks, demote=0, parent=None):
        self.path, self.nav, self.doc, self.blocks, self.demote = path, nav, doc, blocks, demote
        self.parent, self.kids = parent, []
        if parent:
            parent.kids.append(self)
        self.items = []             # API items: (shown name, kind, anchor)
        self.heads = []             # (level, anchor, text) of the headings on the page
        self.body = self.desc = ""
        self.extra = ""             # HTML after the text: the list of sub pages
        self.prev = self.next = None

    def chain(self):
        p, out = self, []
        while p:
            out.append(p)
            p = p.parent
        return out

    def full_title(self):
        return " · ".join(p.nav for p in self.chain())


class Ctx:
    """link and image targets of the Markdown file `src` shown on the page `page`"""

    def __init__(self, site, src, page):
        self.site, self.src, self.page = site, src, page
        self.heading_ids = site.docs[src].heading_ids

    def href(self, url):
        return self.site.href(self.src, self.page, url)

    def image(self, url):
        return self.site.image(self.src, self.page, url)


def item_key(sig):
    """'pub const fn new(..)' -> ('fn', 'new'); 'main!(..)' -> ('macro', 'main'); else None"""
    s = re.sub(r"^pub(?:\([^)]*\))?\s+", "", sig.strip())
    s = re.sub(r"^(?:(?:const|async|unsafe|default|extern(?:\s+\"[^\"]*\")?)\s+)+(?=fn\b)", "", s)
    m = re.match(r"(fn|struct|enum|union|trait|type|static|const|mod)\s+(?:mut\s+)?([A-Za-z_]\w*)", s)
    if m:
        return m.group(1), m.group(2)
    m = re.match(r"([A-Za-z_]\w*)!", s)
    return ("macro", m.group(1)) if m else None


def impl_type(sig):
    """'impl<'a> Sp<'a>' -> 'Sp'; 'impl Style for WinUi' -> 'WinUi'"""
    s = re.sub(r"^(?:unsafe\s+)?impl\s*", "", sig.strip())
    if s.startswith("<"):
        depth = 0
        for k, c in enumerate(s):
            depth += (c == "<") - (c == ">")
            if not depth:
                s = s[k + 1:]
                break
    if " for " in s:
        s = s.rsplit(" for ", 1)[1]
    m = re.match(r"\s*(?:&\s*)?(?:dyn\s+)?([\w:]+)", s)
    return m.group(1).split("::")[-1] if m else None


def describe(blocks, fallback):
    for b in walk(blocks):
        if b["t"] == "p":
            t = " ".join(plain(b["text"]).split())
            if len(t) >= 40:
                return t if len(t) <= 180 else t[:177].rsplit(" ", 1)[0] + " …"
    return fallback


class Site:
    def __init__(self, out):
        self.out = Path(out).resolve()
        self.docs = {}              # src -> Doc
        self.top = []               # the pages of DOCS (with their kids)
        self.pages = []             # every page in reading order (not the landing page)
        self.anchors = {}           # src -> {anchor: page path}
        self.main = {}              # src -> its first page
        self.images = {}            # repository path of every image shown -> (width, height)
        self.errors, self.notes = [], []
        self.links = 0
        self.files = []             # what was written (site paths)

    # ---- reading and splitting
    def load(self):
        for src, title, group, lang in DOCS:
            if not (ROOT / src).is_file():
                raise SiteError(f"{src} (in the DOCS table of tools/site.py) does not exist")
            d = self.docs[src] = Doc(src, title, group, lang)
            base = src[:-3]
            if src == "docs/GUIDE.md":
                self.top.append(self.split_guide(d, base))
            elif src == "docs/API.md":
                self.top.append(self.split_api(d, base))
            else:
                self.top.append(Page(base + ".html", title, d, d.blocks))
        order = []

        def visit(p):
            order.append(p)
            for k in p.kids:
                visit(k)
        for p in self.top:
            visit(p)
        self.pages = order
        for a, b in zip(order, order[1:]):
            a.next, b.prev = b, a
        for p in order:
            amap = self.anchors.setdefault(p.doc.src, {})
            self.main.setdefault(p.doc.src, p.path)
            for b in walk(p.blocks):
                if b["t"] == "h":
                    amap[b["id"]] = p.path
                    lv = b["level"] - p.demote
                    if 2 <= lv <= 4:
                        p.heads.append((lv, b["id"], b["plain"]))
                for t in texts(b):
                    for a in ANCHOR_TAG.findall(t):
                        amap[a] = p.path
            if p.doc.src == "docs/API.md":
                self.api_items(p)

    def split_guide(self, d, base):
        head, secs = split_at(d.blocks, 2)
        index = Page(base + ".html", d.title, d, head)
        names = set()
        for sec in secs:
            h = sec[0]["plain"]
            m = re.match(r"(\d+)\.\s+(.*)", h)
            name = "%02d-%s" % (int(m.group(1)), fslug(m.group(2))) if m else fslug(h)
            while name in names:
                name += "-x"
            names.add(name)
            Page("%s/%s.html" % (base.lower(), name), h, d, sec, demote=1, parent=index)
        return index

    def split_api(self, d, base):
        head, crates = split_at(d.blocks, 2)
        while crates and crates[0][0]["plain"] == "Contents":
            head += crates.pop(0)
        index = Page(base + ".html", d.title, d, head)
        folder = base.lower()
        for sec in crates:
            crate = sec[0]["plain"]
            cname = re.sub(r"[^A-Za-z0-9_]+", "-", crate)
            cp = Page("%s/%s.html" % (folder, cname), crate, d, sec, demote=1, parent=index)
            if size(sec) <= API_PAGE_LIMIT:
                continue
            cp.blocks, subs = split_at(sec, 3)
            names = set()

            def name_of(h):
                t = re.sub(r"^%s(?:/|\s+)" % re.escape(crate), "", h).replace(".rs", "")
                n = fslug(t)
                while n in names:
                    n += "-x"
                names.add(n)
                return "%s/%s--%s.html" % (folder, cname, n)
            for sub in subs:
                sp = Page(name_of(sub[0]["plain"]), sub[0]["plain"], d, sub, demote=2, parent=cp)
                if size(sub) > API_PAGE_LIMIT and any(b["t"] == "h" and b["level"] == 4 for b in sub):
                    sp.blocks, files = split_at(sub, 4)
                    for f in files:
                        Page(name_of(f[0]["plain"]), f[0]["plain"], d, f, demote=3, parent=sp)
        return index

    def api_items(self, page):
        """anchors for the items of an API page: #fn.name, #struct.Name, #Type.method"""
        used, impl = set(), None
        for b in page.blocks:
            if b["t"] == "p":
                m = re.fullmatch(r"\*\*`([^`]*)`\*\*", b["text"].strip())
                impl = impl_type(m.group(1)) if m and re.match(r"(?:unsafe\s+)?impl\b", m.group(1)) else None
                continue
            if b["t"] != "list":
                impl = None
                continue
            member = impl
            for it in b["items"]:
                if it["blank_before"]:
                    member = None
                first = it["blocks"][0] if it["blocks"] else None
                m = re.match(r"\*\*`([^`]+)`\*\*", first["text"]) if first and first["t"] == "p" else None
                key = m and item_key(m.group(1))
                if not key:
                    continue
                kind, name = key
                ident = base = "%s.%s" % (member or kind, name)
                k = 1
                while ident in used:
                    k += 1
                    ident = "%s-%d" % (base, k)
                used.add(ident)
                it["id"] = ident
                b["api"] = True
                page.items.append(("%s::%s" % (member, name) if member else name,
                                   "method" if member and kind == "fn" else kind, ident))
            impl = None

    # ---- link targets
    def href(self, src, page, url):
        self.links += 1
        if EXTERNAL.match(url):
            return url
        path, _, frag = url.partition("#")
        if path:
            target = posixpath.normpath(posixpath.join(posixpath.dirname(src), unquote(path)))
            if target == ".." or target.startswith("../") or path.startswith("/"):
                self.notes.append(f"{src}: the link {url} leaves the repository (shown as text)")
                return None
            target = "" if target == "." else target
        else:
            target = src
        if target in self.anchors and not (frag and LINE_ANCHOR.fullmatch(frag)):
            dest = self.main[target]
            if frag:
                dest = self.anchors[target].get(frag)
                if dest is None:
                    self.errors.append(f"{src}: the link {url}: {target} has no anchor #{frag}")
                    dest = self.main[target]
            h = "" if dest == page else relpath(page, dest)
            return h + ("#" + frag if frag else "") or "#"
        kind = repo_kind(target)
        if kind is None:
            self.errors.append(f"{src}: the link {url}: {target} is not in the repository")
            return None
        if kind == "file" and target.lower().endswith(IMAGE_TYPES):
            self.add_image(target)
            return relpath(page, target)
        if frag and kind == "file" and target.endswith(".md") and not LINE_ANCHOR.fullmatch(frag) \
                and frag not in md_anchors(target):
            self.errors.append(f"{src}: the link {url}: {target} has no heading #{frag}")
        return github(target, kind, frag)

    def image(self, src, page, url):
        if EXTERNAL.match(url):
            return url, None, None
        target = posixpath.normpath(posixpath.join(posixpath.dirname(src), unquote(url)))
        if repo_kind(target) != "file" or not target.lower().endswith(IMAGE_TYPES):
            self.errors.append(f"{src}: the image {url} is missing ({target})")
            return None
        w, h = self.add_image(target)
        return relpath(page, target), w, h

    def add_image(self, target):
        if target not in self.images:
            self.images[target] = png_size(ROOT / target) if target.lower().endswith(".png") else (None, None)
        return self.images[target]

    # ---- rendering
    def render_pages(self):
        for p in self.pages:
            inl = Inline(Ctx(self, p.doc.src, p.path))
            p.body = render(p.blocks, inl, p.demote)
            if p.kids and (p.parent or p.doc.src != "docs/API.md"):      # API.md has its contents
                lis = "".join('<li><a href="%s">%s</a></li>' % (attr(relpath(p.path, k.path)), esc(k.nav))
                              for k in p.kids)
                p.extra = '<h2 class="sub">Sections</h2>\n<ul class="subpages">%s</ul>' % lis
            chain = [q.nav for q in p.chain()[::-1]]          # API reference, crate, section ...
            fallback = "JFLib %s: %s" % (chain[0], " – ".join(chain[1:])) if p.parent else "JFLib: " + p.doc.title
            p.desc = describe(p.blocks, fallback)

    def landing(self):
        """index.html from README.md's sections"""
        d = self.docs["README.md"]
        inl = Inline(Ctx(self, d.src, "index.html"))
        head, secs = split_at(d.blocks, 2)
        sec = {s[0]["plain"]: s[1:] for s in secs}
        need = ["The library", "Feature tour", "Platforms", "Building apps with Python", "Documentation", "License"]
        missing = [n for n in need if n not in sec]
        if missing:
            raise SiteError("README.md: the landing page reads the sections " + ", ".join(missing))
        title = next((b["plain"] for b in head if b["t"] == "h" and b["level"] == 1), "JFLib")
        tagline, intro, shot = "", [], None
        for b in head:
            if b["t"] != "p":
                continue
            t = b["text"].strip()
            urls = re.findall(r"\]\(([^)\s]+)\)", t)
            m = re.fullmatch(r"!\[([^\]]*)\]\(([^)\s]+)\)", t)
            if m:
                shot = shot or m
            elif urls and all(u.startswith("#") for u in urls):
                continue                                    # the README's jump links
            elif not tagline and t.startswith("**") and t.endswith("**"):
                tagline = t[2:-2]
            else:
                intro.append("<p>%s</p>" % inl(t))
        if not tagline or not shot:
            raise SiteError("README.md: the landing page needs the bold tagline and the first screenshot")
        img = self.image(d.src, "index.html", shot.group(2))
        hero = '<img src="%s" alt="%s" width="%s" height="%s" fetchpriority="high">' % (
            attr(img[0]), attr(shot.group(1)), img[1], img[2]) if img else ""
        guide, api = relpath("index.html", self.main["docs/GUIDE.md"]), relpath("index.html", self.main["docs/API.md"])
        parts = ['<section class="hero"><div class="hero-text"><h1>%s</h1><p class="tagline">%s</p>%s'
                 '<p class="cta"><a class="btn primary" href="%s">Developer guide</a><a class="btn" href="%s">'
                 'API reference</a><a class="btn" href="%s">GitHub</a></p></div><figure class="shot">%s</figure>'
                 '</section>' % (esc(title), inl(tagline), "".join(intro), guide, api, REPO, hero)]
        # the library: lead, a card per feature, the note after the list
        lib = sec["The library"]
        lst = next((b for b in lib if b["t"] == "list"), None)
        if lst is None:
            raise SiteError("README.md: 'The library' has no feature list")
        cards = []
        for it in lst["items"]:
            t = it["blocks"][0]["text"] if it["blocks"] and it["blocks"][0]["t"] == "p" else ""
            m = re.match(r"\*\*(.+?)\*\*:?\s*(.*)", t, re.S)
            if m:
                cards.append('<div class="card"><h3>%s</h3><p>%s</p></div>' % (inl(m.group(1).rstrip(":")), inl(m.group(2))))
        k = lib.index(lst)
        parts.append('<section class="band" id="library"><h2>The library</h2>%s<div class="cards">%s</div>%s</section>' % (
            "".join('<p class="lead">%s</p>' % inl(b["text"]) for b in lib[:k] if b["t"] == "p"), "".join(cards),
            "".join('<p class="small">%s</p>' % inl(b["text"]) for b in lib[k + 1:] if b["t"] == "p")))
        # the feature tour: per table two figures and the paragraph after it
        tour = sec["Feature tour"]
        rows = []
        for k, b in enumerate(tour):
            if b["t"] != "table" or not b["rows"]:
                continue
            figs = "".join('<figure>%s<figcaption>%s</figcaption></figure>' % (inl(c), inl(h))
                           for h, c in zip(b["head"], b["rows"][0]))
            nxt = tour[k + 1] if k + 1 < len(tour) and tour[k + 1]["t"] == "p" else None
            rows.append('<div class="tour">%s</div>%s' % (figs, '<p class="tour-text">%s</p>' % inl(nxt["text"]) if nxt else ""))
        first = next((b for b in tour if b["t"] == "p"), None)
        if not rows:
            raise SiteError("README.md: 'Feature tour' has no tables of screenshots")
        parts.append('<section class="band" id="tour"><h2>Feature tour</h2>%s%s</section>' % (
            '<p class="lead">%s</p>' % inl(first["text"]) if first else "", "".join(rows)))
        # platforms
        parts.append('<section class="band" id="platforms"><h2>Platforms</h2>%s</section>' % render(sec["Platforms"], inl))
        # quick start
        _, sub = split_at(sec["Building apps with Python"], 3)
        sub = {s[0]["plain"]: s[1:] for s in sub}
        code = {}
        for name in ("Windows quick start", "Linux quick start", "Start your own app"):
            blocks = sub.get(name, [])
            c = next((b for b in blocks if b["t"] == "code"), None)
            if c is None:
                raise SiteError(f"README.md: the landing page needs the code of '{name}'")
            code[name] = (c, [b for b in blocks[blocks.index(c) + 1:] if b["t"] == "p"][:1])
        win, lin, own = code["Windows quick start"], code["Linux quick start"], code["Start your own app"]
        prereq = self.href(d.src, "index.html", "#prerequisites")
        parts.append('<section class="band" id="quick-start"><h2>Quick start</h2><p class="lead">Python 3, Rust '
                     'through rustup and LLVM / LLD are all it takes (<a href="%s">prerequisites</a>).</p>'
                     '<div class="qs"><div><h3>Windows</h3>%s</div><div><h3>Linux</h3>%s</div></div>'
                     '<h3>Start your own app</h3>%s%s</section>' % (
                         attr(prereq), render([win[0]], inl), render([lin[0]], inl), render([own[0]], inl),
                         render(own[1], inl)))
        # documentation
        table = next((b for b in sec["Documentation"] if b["t"] == "table"), None)
        if table is None:
            raise SiteError("README.md: 'Documentation' has no table")
        docs = []
        for r in table["rows"]:
            m = re.fullmatch(r"\[(.+?)\]\((.+?)\)", r[0])
            if m:
                h = self.href(d.src, "index.html", m.group(2))
                docs.append('<a class="card" href="%s"><h3>%s</h3><p>%s</p></a>' % (
                    attr(h or REPO), inl(m.group(1), links=False), inl(r[1], links=False)))
        parts.append('<section class="band" id="docs"><h2>Documentation</h2><div class="cards">%s</div></section>'
                     % "".join(docs))
        lic = next((b for b in sec["License"] if b["t"] == "p"), None)
        if lic:
            parts.append('<section class="band license"><h2>License</h2><p>%s</p></section>' % inl(lic["text"]))
        home = Page("index.html", "Home", d, [])
        home.body = "\n".join(parts)
        home.desc = " ".join(plain(tagline).split())
        home.title = "%s – %s" % ("JFLib", home.desc.rstrip("."))
        return home

    # ---- output
    def write(self):
        prepare(self.out)
        home = self.landing()
        self.put("index.html", self.page_html(home))
        for p in self.pages:
            self.put(p.path, self.page_html(p))
        self.put("assets/site.css", site_css())
        self.put("assets/site.js", SITE_JS)
        self.put("assets/favicon.svg", FAVICON)
        self.put("assets/search-index.js", self.search_index())
        for img in sorted(self.images):
            self.put(img, (ROOT / img).read_bytes())
        self.put(MARKER, "A website written by tools/site.py; the next run replaces this folder.\n")

    def put(self, rel, data):
        p = (self.out / rel).resolve()
        if self.out not in p.parents:
            raise SiteError(f"refusing to write {rel} outside the output folder")
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(data if isinstance(data, bytes) else data.encode("utf-8"))
        self.files.append(rel)

    def search_index(self):
        """{pages: [[path, title]], entries: [[text, kind, page, anchor]]}; an item's anchor is 0 when it
        follows from its name (Type::name -> Type.name, else kind.name), as site.js derives it"""
        pages, entries = [["index.html", "Home"]], [["Home", "page", 0, ""]]
        for i, p in enumerate(self.pages, 1):
            pages.append([p.path, p.full_title()])
            entries.append([p.nav, "page", i, ""])
            entries += [[t, "section", i, a] for lv, a, t in p.heads]
            for n, k, a in p.items:
                derived = n.replace("::", ".") if "::" in n else "%s.%s" % (k, n)
                entries.append([n, k, i, 0 if a == derived else a])
        data = json.dumps({"pages": pages, "entries": entries}, ensure_ascii=False, separators=(",", ":"))
        return "window.JFLIB_SEARCH=%s;\n" % data

    def sidebar(self, page):
        cur = set(id(p) for p in page.chain())

        def item(p, label=None):
            a = ' aria-current="page"' if p is page else ""
            lang = p.doc.lang if not p.parent else ""
            badge = '<span class="lang" title="%s">%s</span>' % (LANGUAGES[lang], lang) if lang in LANGUAGES else ""
            h = '<li><a href="%s"%s>%s%s</a>' % (attr(relpath(page.path, p.path)), a, esc(label or p.nav), badge)
            if p.kids and id(p) in cur:
                h += "<ul>%s</ul>" % "".join(item(k) for k in p.kids)
            return h + "</li>"
        out = ['<nav id="jf-nav" class="sidebar" aria-label="Documentation">']
        for g in GROUPS:
            lis = [item(p) for p in self.top if p.doc.group == g]
            if g == "Start":
                a = ' aria-current="page"' if page.path == "index.html" else ""
                lis.insert(0, '<li><a href="%s"%s>Home</a></li>' % (attr(relpath(page.path, "index.html")), a))
            out.append('<p class="grp">%s</p><ul>%s</ul>' % (g, "".join(lis)))
        out.append("</nav>")
        return "\n".join(out)

    def page_html(self, p):
        root = "../" * p.path.count("/")
        home = p.path == "index.html"
        title = getattr(p, "title", None) or p.full_title() + " · JFLib"
        main = []
        if not home:
            crumbs = p.chain()[1:][::-1]
            if crumbs:
                main.append('<nav class="crumbs" aria-label="Breadcrumb">%s</nav>' % " › ".join(
                    '<a href="%s">%s</a>' % (attr(relpath(p.path, c.path)), esc(c.nav)) for c in crumbs))
            lang = p.doc.lang
            if lang in LANGUAGES:
                main.append('<p class="note">This page is in %s, like its source <code>%s</code>.</p>'
                            % (LANGUAGES[lang], esc(p.doc.src)))
            la = ' lang="%s"' % lang if lang != "en" else ""
            api = " api" if p.doc.src == "docs/API.md" else ""
            main.append('<article class="doc%s"%s>\n%s\n%s\n</article>' % (api, la, p.body, p.extra))
            pager = []
            for cls, q, word in (("prev", p.prev, "Previous"), ("next", p.next, "Next")):
                if q:
                    pager.append('<a class="%s" href="%s"><span>%s</span>%s</a>' % (
                        cls, attr(relpath(p.path, q.path)), word, esc(q.nav)))
            if pager:
                main.append('<nav class="pager" aria-label="Previous and next page">%s</nav>' % "".join(pager))
            main.append('<footer class="foot"><p>Source: <a href="%s">%s</a> in the JFLib repository. %s</p></footer>'
                        % (attr(github(p.doc.src)), esc(p.doc.src), LICENSE_LINE))
        else:
            main.append('<div class="landing">%s</div>' % p.body)
            main.append('<footer class="foot wide"><p>%s Generated from the repository by '
                        '<code>tools/site.py</code>.</p></footer>' % LICENSE_LINE)
        toc = ""
        heads = [h for h in p.heads if h[0] <= 3]
        if len(heads) >= 2:
            toc = '<aside class="toc" aria-label="On this page"><p>On this page</p><ul>%s</ul></aside>' % "".join(
                '<li class="l%d"><a href="#%s">%s</a></li>' % (lv, attr(a), esc(t)) for lv, a, t in heads)
        return PAGE.format(
            title=esc(title), desc=attr(p.desc), root=root, body_class="home" if home else "doc-page",
            layout="layout has-toc" if toc else "layout", sidebar=self.sidebar(p), main="\n".join(main), toc=toc,
            guide=attr(relpath(p.path, self.main["docs/GUIDE.md"])), api=attr(relpath(p.path, self.main["docs/API.md"])),
            home=attr(relpath(p.path, "index.html")), repo=REPO, icons=ICONS)


def prepare(out):
    """empty the output folder; only one that is empty or that this tool wrote"""
    if out == ROOT or out in ROOT.parents or out.anchor == str(out):
        raise SiteError(f"refusing to use {out} as the output folder")
    if out.exists():
        if not out.is_dir():
            raise SiteError(f"{out} is a file")
        entries = sorted(out.iterdir())
        if entries and not (out / MARKER).is_file():
            raise SiteError(f"{out} is not empty and holds no site of tools/site.py ({MARKER}); pick another --out")
        for e in entries:
            if e.is_dir() and not e.is_symlink():
                shutil.rmtree(e)
            else:
                e.unlink()
    out.mkdir(parents=True, exist_ok=True)


def build(out):
    site = Site(out)
    site.load()
    site.render_pages()
    site.write()
    return site


# ---------------------------------------------------------------- the look
LICENSE_LINE = ('JFLib is dual-licensed under the <a href="%s">MIT License</a> or the <a href="%s">Apache License '
                '2.0</a>.' % (github("LICENSE-MIT"), github("LICENSE-APACHE")))
THEME_BOOT = ('try{var t=localStorage.getItem("jflib-theme");if(t==="dark"||t==="light")'
              'document.documentElement.setAttribute("data-theme",t)}catch(e){}')
ICONS = {
    "logo": '<svg viewBox="0 0 32 32" aria-hidden="true"><rect width="32" height="32" rx="8" fill="var(--accent)"/>'
            '<text x="16" y="21.5" text-anchor="middle" font-size="14" font-weight="700" fill="#fff" '
            'font-family="system-ui,sans-serif">JF</text></svg>',
    "search": '<svg viewBox="0 0 16 16" aria-hidden="true"><path fill="currentColor" d="M10.7 11.8a6 6 0 1 1 1.1-1.1'
              'l3 3a.8.8 0 1 1-1.1 1.1zM11.5 7a4.5 4.5 0 1 0-9 0 4.5 4.5 0 0 0 9 0"/></svg>',
    "menu": '<svg viewBox="0 0 16 16" aria-hidden="true"><path fill="currentColor" d="M1 2.8a.8.8 0 0 1 .8-.8h12.4a.8.8'
            ' 0 0 1 0 1.5H1.8A.8.8 0 0 1 1 2.8m0 5a.8.8 0 0 1 .8-.8h12.4a.8.8 0 0 1 0 1.5H1.8A.8.8 0 0 1 1 7.8m.8 4.2'
            'h12.4a.8.8 0 0 1 0 1.5H1.8a.8.8 0 0 1 0-1.5"/></svg>',
    "moon": '<svg class="i-moon" viewBox="0 0 16 16" aria-hidden="true"><path fill="currentColor" d="M6.2.6a.7.7 0 0 1'
            ' .1.8 6 6 0 0 0 8.3 8.3.7.7 0 0 1 .9 1A7.6 7.6 0 1 1 5.3.5a.7.7 0 0 1 .9.1"/></svg>',
    "sun": '<svg class="i-sun" viewBox="0 0 16 16" aria-hidden="true"><circle cx="8" cy="8" r="3.2" fill="currentColor"/>'
           '<path stroke="currentColor" stroke-width="1.5" stroke-linecap="round" d="M8 .9v1.6M8 13.5v1.6M.9 8h1.6M13.5'
           ' 8h1.6M3 3l1.1 1.1M11.9 11.9 13 13M3 13l1.1-1.1M11.9 4.1 13 3"/></svg>',
}
FAVICON = ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" rx="8" '
           'fill="#0a64d1"/><text x="16" y="21.5" text-anchor="middle" font-size="14" font-weight="700" fill="#fff" '
           'font-family="system-ui,sans-serif">JF</text></svg>\n')
PAGE = """<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<meta name="description" content="{desc}">
<meta name="color-scheme" content="light dark">
<link rel="icon" href="{root}assets/favicon.svg" type="image/svg+xml">
<link rel="stylesheet" href="{root}assets/site.css">
<script>""" + THEME_BOOT.replace("{", "{{").replace("}", "}}") + """</script>
</head>
<body class="{body_class}" data-root="{root}">
<a class="skip" href="#jf-main">Skip to the content</a>
<header class="topbar">
<button class="iconbtn" id="jf-menu" type="button" aria-label="Navigation" aria-expanded="false" aria-controls="jf-nav">{icons[menu]}</button>
<a class="brand" href="{home}">{icons[logo]}<span>JFLib</span></a>
<nav class="toplinks" aria-label="Main"><a href="{guide}">Guide</a><a href="{api}">API</a><a href="{repo}">GitHub</a></nav>
<div class="search" role="search">{icons[search]}<input id="jf-q" type="search" placeholder="Search the docs" aria-label="Search titles, headings and API items" autocomplete="off" spellcheck="false"><ul id="jf-results" class="results" hidden></ul></div>
<button class="iconbtn" id="jf-theme" type="button" aria-label="Dark theme" aria-pressed="false">{icons[moon]}{icons[sun]}</button>
</header>
<div class="{layout}">
{sidebar}
<div class="scrim" id="jf-scrim"></div>
<main id="jf-main" class="main">
{main}
</main>
{toc}
</div>
<script src="{root}assets/site.js"></script>
</body>
</html>
"""
LIGHT = {
    "bg": "#ffffff", "fg": "#1c2128", "muted": "#58626d", "border": "#d8dee4", "soft": "#f3f5f7", "side": "#f7f8fa",
    "bar": "rgba(255,255,255,.9)", "hover": "#e8ecf0", "link": "#0a5fc2", "accent": "#0a64d1", "accent-strong": "#0550ae",
    "accent-soft": "#e2edfb", "on-accent": "#ffffff", "code-bg": "#f6f8fa", "icode": "rgba(129,139,152,.17)",
    "zebra": "#fafbfc", "card": "#ffffff", "mark": "#fff2bf", "scrim": "rgba(15,20,25,.35)",
    "shadow": "0 1px 2px rgba(27,31,36,.06), 0 8px 24px rgba(27,31,36,.12)",
    "k": "#cf222e", "s": "#0a3069", "c": "#6e7781", "n": "#0550ae", "m": "#8250df", "a": "#116329", "t": "#953800",
    "l": "#8250df",
}
DARK = {
    "bg": "#0e1116", "fg": "#e2e6eb", "muted": "#9ba6b2", "border": "#2b323b", "soft": "#161b22", "side": "#11151b",
    "bar": "rgba(14,17,22,.9)", "hover": "#1d242d", "link": "#58a6ff", "accent": "#1f6feb", "accent-strong": "#8cc0ff",
    "accent-soft": "#14273f", "on-accent": "#ffffff", "code-bg": "#151a21", "icode": "rgba(110,118,129,.32)",
    "zebra": "#12171d", "card": "#141920", "mark": "#3b3315", "scrim": "rgba(0,0,0,.55)",
    "shadow": "0 1px 2px rgba(0,0,0,.4), 0 8px 24px rgba(0,0,0,.5)",
    "k": "#ff7b72", "s": "#a5d6ff", "c": "#8b949e", "n": "#79c0ff", "m": "#d2a8ff", "a": "#7ee787", "t": "#ffa657",
    "l": "#d2a8ff",
}
DRAWER = ("position:fixed;z-index:35;top:56px;left:0;bottom:0;width:min(86vw,320px);height:auto;align-self:normal;"
          "transform:translateX(-105%);transition:transform .18s ease;box-shadow:var(--shadow);visibility:hidden")
CSS = r"""*,*::before,*::after{box-sizing:border-box}
html{-webkit-text-size-adjust:100%;scroll-padding-top:72px}
body{margin:0;background:var(--bg);color:var(--fg);font:16px/1.65 system-ui,-apple-system,"Segoe UI",Roboto,"Helvetica Neue","Noto Sans","Liberation Sans",Arial,sans-serif}
a{color:var(--link);text-decoration:none}a:hover{text-decoration:underline}
code,pre{font-family:ui-monospace,"Cascadia Mono","Cascadia Code",SFMono-Regular,Menlo,Consolas,"Liberation Mono","DejaVu Sans Mono",monospace}
.skip{position:absolute;left:-9999px;top:8px;z-index:99;padding:8px 12px;background:var(--bg);border:1px solid var(--border);border-radius:6px}.skip:focus{left:8px}
:focus-visible{outline:2px solid var(--accent);outline-offset:2px}
.topbar{position:sticky;top:0;z-index:40;display:flex;align-items:center;gap:12px;height:56px;padding:0 20px;background:var(--bar);border-bottom:1px solid var(--border);-webkit-backdrop-filter:saturate(1.4) blur(8px);backdrop-filter:saturate(1.4) blur(8px)}
.brand{display:flex;align-items:center;gap:10px;color:var(--fg);font-weight:700;font-size:17px}.brand:hover{text-decoration:none}.brand svg{width:28px;height:28px;flex:none}
.toplinks{display:flex;gap:2px;margin-left:12px}.toplinks a{padding:6px 10px;border-radius:6px;color:var(--muted);font-size:14px}.toplinks a:hover{color:var(--fg);background:var(--soft);text-decoration:none}
.search{position:relative;flex:1;max-width:440px;margin-left:auto}
.search svg{position:absolute;left:10px;top:10px;width:16px;height:16px;color:var(--muted);pointer-events:none}
.search input{width:100%;height:36px;padding:0 12px 0 34px;border:1px solid var(--border);border-radius:8px;background:var(--soft);color:var(--fg);font:inherit;font-size:14px;-webkit-appearance:none;appearance:none}
.search input:focus{outline:2px solid var(--accent);outline-offset:-1px;background:var(--bg)}
.results{position:absolute;top:44px;left:0;right:0;max-height:min(70vh,560px);overflow:auto;margin:0;padding:6px;list-style:none;background:var(--bg);border:1px solid var(--border);border-radius:10px;box-shadow:var(--shadow)}
.results a{display:block;padding:6px 10px;border-radius:6px;color:var(--fg);line-height:1.35}.results a:hover,.results .sel a{background:var(--accent-soft);text-decoration:none}
.results small{display:block;color:var(--muted);font-size:12px}.results .none{padding:8px 10px;color:var(--muted);font-size:14px}
.iconbtn{display:inline-flex;align-items:center;justify-content:center;flex:none;width:36px;height:36px;padding:0;border:1px solid var(--border);border-radius:8px;background:transparent;color:var(--fg);cursor:pointer}.iconbtn:hover{background:var(--soft)}.iconbtn svg{width:18px;height:18px}
#jf-menu{display:none}.i-sun{display:none}
.layout{display:grid;grid-template-columns:280px minmax(0,1fr)}
.sidebar{position:sticky;top:56px;align-self:start;height:calc(100vh - 56px);overflow-y:auto;padding:12px 14px 40px;border-right:1px solid var(--border);background:var(--side);font-size:14px}
.sidebar .grp{margin:18px 10px 6px;color:var(--muted);font-size:12px;font-weight:600;letter-spacing:.06em;text-transform:uppercase}
.sidebar ul{margin:0;padding:0;list-style:none}.sidebar ul ul{margin:2px 0 6px 12px;padding-left:8px;border-left:1px solid var(--border)}
.sidebar a{display:block;padding:4px 10px;border-radius:6px;color:var(--fg);line-height:1.4;overflow-wrap:anywhere}.sidebar a:hover{background:var(--hover);text-decoration:none}
.sidebar a[aria-current]{background:var(--accent-soft);color:var(--accent-strong);font-weight:600}
.lang{display:inline-block;margin-left:6px;padding:0 5px;border:1px solid var(--border);border-radius:4px;color:var(--muted);font-size:11px;font-weight:400;line-height:16px;vertical-align:1px}
.scrim{display:none}
.main{min-width:0;padding:36px 56px 72px}
.doc{max-width:860px;margin:0 auto;overflow-wrap:break-word}
.toc{display:none}
@media (min-width:1300px){.layout.has-toc{grid-template-columns:280px minmax(0,1fr) 250px}.toc{display:block;position:sticky;top:56px;align-self:start;max-height:calc(100vh - 56px);overflow-y:auto;padding:40px 20px 40px 0;font-size:13px}}
.toc p{margin:0 0 8px;color:var(--muted);font-size:12px;font-weight:600;letter-spacing:.06em;text-transform:uppercase}
.toc ul{margin:0;padding:0;list-style:none}.toc a{display:block;padding:3px 0 3px 10px;border-left:2px solid var(--border);color:var(--muted);line-height:1.35}.toc .l3 a{padding-left:22px}.toc a:hover{color:var(--fg);border-color:var(--muted);text-decoration:none}
.doc h1{margin:0 0 20px;font-size:2.05rem;line-height:1.2;letter-spacing:-.01em}
.doc h2{margin:44px 0 14px;padding-bottom:8px;border-bottom:1px solid var(--border);font-size:1.5rem;line-height:1.3}
.doc h3{margin:32px 0 10px;font-size:1.2rem;line-height:1.35}.doc h4{margin:26px 0 8px;font-size:1.05rem}.doc h5,.doc h6{margin:22px 0 8px;font-size:1rem}
.doc h2.sub{font-size:1.2rem}
.hl{margin-left:8px;color:var(--muted);font-weight:400;opacity:0}.hl:hover{text-decoration:none}:is(h1,h2,h3,h4,h5,h6):hover>.hl,.hl:focus{opacity:1}
.doc p,.doc ul,.doc ol,.doc pre,.doc .table,.doc blockquote{margin:0 0 16px}
.doc ul,.doc ol{padding-left:26px}.doc li{margin:3px 0}.doc li>ul,.doc li>ol{margin:4px 0}.doc li>p{margin:0 0 8px}.doc li>pre{margin:8px 0}
.doc :not(pre)>code{padding:.12em .38em;border-radius:5px;background:var(--icode);font-size:.86em}
pre.code{overflow-x:auto;padding:14px 16px;border:1px solid var(--border);border-radius:10px;background:var(--code-bg);font-size:13.5px;line-height:1.55;tab-size:4}
pre.code code{font-size:inherit}
.k{color:var(--k)}.s{color:var(--s)}.c{color:var(--c);font-style:italic}.n{color:var(--n)}.m{color:var(--m)}.a{color:var(--a)}.t{color:var(--t)}.l{color:var(--l)}
.table{overflow-x:auto}table{border-collapse:collapse;font-size:15px}th,td{padding:7px 12px;border:1px solid var(--border);text-align:left;vertical-align:top}th{background:var(--soft);font-weight:600}tbody tr:nth-child(2n) td{background:var(--zebra)}
.doc img{max-width:100%;height:auto;border-radius:6px}
hr{margin:32px 0;border:0;border-top:1px solid var(--border)}
blockquote{margin-left:0;padding:2px 16px;border-left:4px solid var(--border);color:var(--muted)}
.doc :is(h1,h2,h3,h4,li):target,.api ul.items>li:target{background:var(--mark);border-radius:6px}
.crumbs{margin:0 auto 14px;max-width:860px;color:var(--muted);font-size:14px}.crumbs a{color:var(--muted)}
.note{max-width:860px;margin:0 auto 22px;padding:10px 14px;border:1px solid var(--border);border-left:4px solid var(--accent);border-radius:8px;background:var(--soft);font-size:15px}
.note code{font-size:.86em}
.subpages{columns:2 220px;column-gap:28px}.subpages li{break-inside:avoid}
.api ul.items{padding-left:0;list-style:none}
.api ul.items>li{margin:0 0 12px;padding:10px 14px;border:1px solid var(--border);border-radius:8px;background:var(--card)}
.api ul.items>li>strong:first-child,.api ul.items>li>p:first-child>strong:first-child{display:block;margin-bottom:2px}
.api ul.items>li>strong:first-child>code,.api ul.items>li>p:first-child>strong:first-child>code{padding:0;background:none;font-size:.9em;font-weight:600}
.api ul.items ul{margin-top:6px;font-size:15px}
.pager{display:flex;gap:16px;max-width:860px;margin:48px auto 0}.pager a{flex:1;display:block;min-width:0;padding:12px 16px;border:1px solid var(--border);border-radius:10px;color:var(--fg);overflow-wrap:anywhere}.pager a:hover{border-color:var(--accent);text-decoration:none}.pager span{display:block;color:var(--muted);font-size:12px}.pager .next{margin-left:auto;text-align:right}
.foot{max-width:860px;margin:40px auto 0;padding-top:18px;border-top:1px solid var(--border);color:var(--muted);font-size:13px}.foot.wide{max-width:1240px;margin:24px auto 40px;padding:18px 32px 0}
.home .main{padding:0}.home .layout{display:block}.home #jf-menu{display:inline-flex}.home .sidebar{%(drawer)s}
.nav-open .sidebar{transform:none;visibility:visible}.nav-open .scrim{display:block;position:fixed;inset:56px 0 0;z-index:30;background:var(--scrim)}
.landing h2{margin:0 0 12px;font-size:1.75rem;line-height:1.25}.landing h3{margin:0 0 6px;font-size:1.05rem}
.hero{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1.2fr);gap:48px;align-items:center;max-width:1240px;margin:0 auto;padding:56px 32px 32px}
.hero h1{margin:0 0 10px;font-size:clamp(2.4rem,5vw,3.5rem);line-height:1.1;letter-spacing:-.02em}
.tagline{margin:0 0 14px;font-size:1.3rem;font-weight:600;line-height:1.45}.hero-text p{color:var(--muted)}.hero-text .tagline{color:var(--fg)}
.cta{display:flex;flex-wrap:wrap;gap:10px;margin-top:22px}
.btn{display:inline-block;padding:9px 18px;border:1px solid var(--border);border-radius:8px;color:var(--fg);font-weight:600}.btn:hover{border-color:var(--accent);text-decoration:none}.btn.primary{background:var(--accent);border-color:var(--accent);color:var(--on-accent)}
.shot{margin:0}.shot img{display:block;width:100%;height:auto;border:1px solid var(--border);border-radius:12px;box-shadow:var(--shadow)}
.band{max-width:1240px;margin:0 auto;padding:36px 32px}.landing :not(pre)>code{padding:.12em .38em;border-radius:5px;background:var(--icode);font-size:.86em}.hero-text code{white-space:nowrap}
.lead{max-width:780px;margin:0 0 8px;color:var(--muted);font-size:1.05rem}.small{color:var(--muted);font-size:14px}
.cards{display:grid;grid-template-columns:repeat(auto-fill,minmax(270px,1fr));gap:16px;margin:22px 0}
.card{padding:16px 18px;border:1px solid var(--border);border-radius:12px;background:var(--card)}.card p{margin:0;color:var(--muted);font-size:15px}
a.card{display:block;color:var(--fg)}a.card:hover{border-color:var(--accent);text-decoration:none}a.card h3{color:var(--link)}
.tour{display:grid;grid-template-columns:1fr 1fr;gap:20px;margin:26px 0 10px}.tour figure{margin:0}.tour img{display:block;width:100%;height:auto;border:1px solid var(--border);border-radius:10px}
figcaption{margin-top:8px;font-size:15px;font-weight:600}.tour-text{max-width:900px;margin:0 0 24px;color:var(--muted)}
.qs{display:grid;grid-template-columns:1fr 1fr;gap:20px;margin:18px 0 24px}.qs pre{margin:0}
.hero>*,.tour>*,.qs>*,.cards>*{min-width:0}
.landing .table{overflow-x:auto;margin:16px 0}.landing pre.code{margin:0 0 14px}
@media (max-width:959px){#jf-menu{display:inline-flex}.toplinks{display:none}.layout{display:block}.sidebar{%(drawer)s}.main{padding:22px 16px 56px}.topbar{gap:8px;padding:0 12px}.brand span{display:none}}
@media (max-width:820px){.hero,.tour,.qs{grid-template-columns:1fr}.hero{gap:24px;padding:28px 16px 12px}.band{padding:26px 16px}.foot.wide{padding:18px 16px 0}.doc h1{font-size:1.7rem}.doc h2{font-size:1.3rem}}
@media (prefers-reduced-motion:reduce){.sidebar{transition:none}}
"""


def site_css():
    def vars_(pal):
        return ";".join("--%s:%s" % kv for kv in pal.items()) + ";color-scheme:%s" % ("dark" if pal is DARK else "light")
    dark_rules = "{%s}%%s .i-moon{display:none}%%s .i-sun{display:inline}" % vars_(DARK)
    return (":root{%s}\n" % vars_(LIGHT)
            + "@media (prefers-color-scheme:dark){:root:not([data-theme=light])%s}\n"
            % (dark_rules % (":root:not([data-theme=light])", ":root:not([data-theme=light])"))
            + ":root[data-theme=dark]%s\n" % (dark_rules % (":root[data-theme=dark]", ":root[data-theme=dark]"))
            + CSS.replace("%(drawer)s", DRAWER))


SITE_JS = r"""(function () {
  "use strict";
  var d = document, html = d.documentElement, root = d.body.getAttribute("data-root") || "";
  // light / dark: the system's setting until the button is used, then the stored choice
  var tb = d.getElementById("jf-theme");
  function dark() {
    var t = html.getAttribute("data-theme");
    return t ? t === "dark" : window.matchMedia("(prefers-color-scheme: dark)").matches;
  }
  function sync() {
    var on = dark();
    tb.setAttribute("aria-pressed", String(on));
    tb.setAttribute("aria-label", on ? "Light theme" : "Dark theme");
    tb.title = on ? "Light theme" : "Dark theme";
  }
  tb.addEventListener("click", function () {
    var t = dark() ? "light" : "dark";
    html.setAttribute("data-theme", t);
    try { localStorage.setItem("jflib-theme", t); } catch (e) {}
    sync();
  });
  sync();
  // the navigation as a drawer on narrow screens
  var mb = d.getElementById("jf-menu");
  function nav(open) { d.body.classList.toggle("nav-open", open); mb.setAttribute("aria-expanded", String(open)); }
  mb.addEventListener("click", function () { nav(!d.body.classList.contains("nav-open")); });
  d.getElementById("jf-scrim").addEventListener("click", function () { nav(false); });
  var side = d.getElementById("jf-nav"), here = side.querySelector("a[aria-current]");
  if (here && here.offsetTop > side.clientHeight * 0.6) side.scrollTop = here.offsetTop - side.clientHeight / 3;
  // search over page titles, headings and API items (assets/search-index.js, loaded on first use)
  var q = d.getElementById("jf-q"), box = d.getElementById("jf-results"), data = null, hits = [], sel = -1;
  function load() {
    if (data !== null) return;
    data = false;
    var s = d.createElement("script");
    s.src = root + "assets/search-index.js";
    s.onload = function () { data = window.JFLIB_SEARCH; find(); };
    d.head.appendChild(s);
  }
  function find() {
    var words = q.value.toLowerCase().split(/\s+/).filter(Boolean);
    box.textContent = ""; hits = []; sel = -1;
    if (!words.length || !data) { box.hidden = true; return; }
    for (var i = 0; i < data.entries.length; i++) {
      var e = data.entries[i], t = e[0].toLowerCase(), all = t + " " + data.pages[e[2]][1].toLowerCase(), ok = true;
      for (var w = 0; w < words.length && ok; w++) ok = all.indexOf(words[w]) >= 0;
      if (!ok) continue;
      var at = t.indexOf(words[0]);
      var rank = e[1] === "page" ? 0 : e[1] === "section" ? 1 : 2;
      hits.push([(t === words[0] ? 0 : at === 0 ? 1 : at > 0 ? 2 : 3) * 3 + rank, t.length, i]);
    }
    hits.sort(function (a, b) { return a[0] - b[0] || a[1] - b[1] || a[2] - b[2]; });
    hits = hits.slice(0, 30);
    if (!hits.length) {
      var li = d.createElement("li"); li.className = "none"; li.textContent = "Nothing found"; box.appendChild(li);
    }
    hits.forEach(function (h) {
      var e = data.entries[h[2]], p = data.pages[e[2]], li = d.createElement("li"), a = d.createElement("a"), s = d.createElement("small");
      var at = e[3] === 0 ? (e[0].indexOf("::") >= 0 ? e[0].replace("::", ".") : e[1] + "." + e[0]) : e[3];
      a.href = root + p[0] + (at ? "#" + at : "");
      a.textContent = e[0];
      s.textContent = (e[1] === "page" ? "" : e[1] + " · ") + p[1];
      a.appendChild(s); li.appendChild(a); box.appendChild(li);
    });
    box.hidden = false;
  }
  function mark(k) {
    var items = box.querySelectorAll("li");
    if (!hits.length) return;
    sel = (k + hits.length) % hits.length;
    for (var i = 0; i < items.length; i++) items[i].classList.toggle("sel", i === sel);
    items[sel].scrollIntoView({ block: "nearest" });
  }
  q.addEventListener("focus", load);
  q.addEventListener("input", function () { load(); find(); });
  q.addEventListener("keydown", function (e) {
    if (e.key === "ArrowDown") { e.preventDefault(); mark(sel + 1); }
    else if (e.key === "ArrowUp") { e.preventDefault(); mark(sel - 1); }
    else if (e.key === "Enter" && hits.length) { e.preventDefault(); box.querySelectorAll("a")[Math.max(sel, 0)].click(); }
    else if (e.key === "Escape") { q.value = ""; find(); q.blur(); }
  });
  q.addEventListener("blur", function () { setTimeout(function () { box.hidden = true; }, 200); });
  d.addEventListener("keydown", function (e) {
    var t = d.activeElement && d.activeElement.tagName;
    if (e.key === "/" && t !== "INPUT" && t !== "TEXTAREA") { e.preventDefault(); q.focus(); }
    else if (e.key === "Escape") nav(false);
  });
})();
"""


# ---------------------------------------------------------------- checks
VOID = frozenset("area base br col embed hr img input link meta source track wbr".split())


class Scan(HTMLParser):
    """one page: nesting errors, ids, links, title, description, language"""

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.stack, self.errors, self.ids, self.refs = [], [], [], []
        self.title, self.desc, self.lang, self.in_title = None, None, None, False

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        line = self.getpos()[0]
        if a.get("id"):
            self.ids.append(a["id"])
        if tag == "a" and "href" in a:
            self.refs.append(("link", a["href"], line))
        elif tag in ("img", "script") and "src" in a:
            self.refs.append((tag, a["src"], line))
        elif tag == "link" and "href" in a:
            self.refs.append(("link", a["href"], line))
        if tag == "html":
            self.lang = a.get("lang")
        elif tag == "meta" and a.get("name") == "description":
            self.desc = a.get("content")
        elif tag == "title":
            self.in_title, self.title = True, ""
        if tag not in VOID:
            self.stack.append((tag, line))

    def handle_startendtag(self, tag, attrs):
        self.handle_starttag(tag, attrs)
        if tag not in VOID and self.stack and self.stack[-1][0] == tag:
            self.stack.pop()

    def handle_endtag(self, tag):
        line = self.getpos()[0]
        if tag == "title":
            self.in_title = False
        if tag in VOID:
            self.errors.append(f"line {line}: </{tag}> closes a void element")
        elif not self.stack or self.stack[-1][0] != tag:
            top = self.stack[-1] if self.stack else ("nothing", 0)
            self.errors.append(f"line {line}: </{tag}> closes <{top[0]}> of line {top[1]}")
            if any(t == tag for t, _ in self.stack):
                while self.stack.pop()[0] != tag:
                    pass
        else:
            self.stack.pop()

    def handle_data(self, data):
        if self.in_title:
            self.title += data

    def done(self):
        self.close()
        self.errors += [f"line {line}: <{t}> is never closed" for t, line in self.stack]
        return self


def local_paths(out):
    """the absolute paths of this machine that must not show up in the output"""
    found = set()
    for p in (ROOT, Path.home(), out, Path(tempfile.gettempdir())):
        p = Path(p).resolve()
        if len(p.parts) >= 3:                       # C:\Users\name, /home/name; not / or /root
            found |= {str(p).lower(), p.as_posix().lower()}
    return sorted(found, key=len, reverse=True)


def check_site(site):
    out, fails = site.out, []
    files = {p.relative_to(out).as_posix(): p for p in out.rglob("*") if p.is_file()}
    want = ["index.html"] + [p.path for p in site.pages]
    want += [src[:-3] + ".html" for src, *_ in DOCS]
    guide = site.docs["docs/GUIDE.md"]
    n_guide = sum(1 for b in guide.blocks if b["t"] == "h" and b["level"] == 2)
    n_api = sum(1 for b in site.docs["docs/API.md"].blocks if b["t"] == "h" and b["level"] == 2 and b["plain"] != "Contents")
    got_guide = sum(1 for p in site.pages if p.path.startswith("docs/guide/"))
    got_api = sum(1 for p in site.pages if p.path.startswith("docs/api/") and p.parent and not p.parent.parent)
    if got_guide != n_guide or n_guide < 10:
        fails.append(f"docs/GUIDE.md has {n_guide} sections, the site {got_guide} pages of them")
    if got_api != n_api or n_api < 10:
        fails.append(f"docs/API.md has {n_api} crates, the site {got_api} pages of them")
    fails += [f"{w}: the page is missing" for w in sorted(set(want)) if w not in files]
    scans = {}
    for f in sorted(files):
        if f.endswith(".html"):
            scans[f] = Scan()
            scans[f].feed(files[f].read_bytes().decode("utf-8"))
            scans[f].done()
    ids = {f: set(s.ids) for f, s in scans.items()}
    nref = 0
    for f, s in scans.items():
        fails += [f"{f}: {e}" for e in s.errors]
        if not (s.title or "").strip():
            fails.append(f"{f}: no <title>")
        if not (s.desc or "").strip():
            fails.append(f"{f}: no description")
        if not s.lang:
            fails.append(f"{f}: no lang")
        dup = sorted({i for i in s.ids if s.ids.count(i) > 1}) if len(s.ids) != len(ids[f]) else []
        fails += [f"{f}: the id {i} is used twice" for i in dup]
        for kind, url, line in s.refs:
            if EXTERNAL.match(url):
                continue
            nref += 1
            path, _, frag = url.partition("#")
            target = posixpath.normpath(posixpath.join(posixpath.dirname(f), unquote(path))) if path else f
            if target == ".." or target.startswith("../"):
                fails.append(f"{f}:{line}: {url} leaves the site")
            elif target not in files:
                fails.append(f"{f}:{line}: {url}: {target} does not exist")
            elif kind == "img" and not files[target].stat().st_size:
                fails.append(f"{f}:{line}: the image {target} is empty")
            elif frag and target.endswith(".html") and unquote(frag) not in ids[target]:
                fails.append(f"{f}:{line}: {url}: no #{unquote(frag)} in {target}")
    idx = files.get("assets/search-index.js")
    if idx:
        t = idx.read_bytes().decode("utf-8")
        data = json.loads(t[t.index("=") + 1:t.rindex(";")])
        for text, kind, pi, anchor in data["entries"]:
            page = data["pages"][pi][0]
            if anchor == 0:                                 # as site.js derives it
                anchor = text.replace("::", ".", 1) if "::" in text else "%s.%s" % (kind, text)
            if page not in ids or (anchor and anchor not in ids[page]):
                fails.append(f"search index: {text} -> {page}#{anchor} does not exist")
    needles = local_paths(out)
    for f, p in sorted(files.items()):
        if f.endswith((".html", ".css", ".js", ".svg")):
            low = p.read_bytes().decode("utf-8", "replace").lower()
            fails += [f"{f}: holds the local path {n}" for n in needles if n in low]
    fails += ["generator: " + e for e in site.errors]
    return fails, nref


# ---------------------------------------------------------------- the converter's own cases
class TestTargets:
    def href(self, url):
        return url.replace(".md", ".html")

    def image(self, url):
        return url, None, None


CASES = [
    ("a `x\ny` b", "<p>a <code>x y</code> b</p>"),
    ("Vec<usize>, a &lt;&lt; 8, R&D, AF_\\* and <script>x</script>",
     "<p>Vec&lt;usize&gt;, a &lt;&lt; 8, R&amp;D, AF_* and &lt;script&gt;x&lt;/script&gt;</p>"),
    ("**`ui`** is *the* UI, 2 * 3 * 4", "<p><strong><code>ui</code></strong> is <em>the</em> UI, 2 * 3 * 4</p>"),
    ("[the guide](docs/GUIDE.md#2-skeleton) and ![a cat](cat.png)",
     '<p><a href="docs/GUIDE.html#2-skeleton">the guide</a> and <img src="cat.png" alt="a cat" loading="lazy" '
     'decoding="async"></p>'),
    ("see https://example.com/a. and `https://x.y`",
     '<p>see <a href="https://example.com/a">https://example.com/a</a>. and <code>https://x.y</code></p>'),
    ('<a id="here"></a>\n\nx<br>y <img src="p.png" onerror="alert(1)" width="20">',
     '<a id="here"></a>\n<p>x<br>y <img src="p.png" alt="" width="20" loading="lazy" decoding="async"></p>'),
    ("| a | b |\n|---|:-:|\n| `x \\| y` | [l](u.md) |",
     '<div class="table"><table>\n<thead><tr><th>a</th><th style="text-align:center">b</th></tr></thead>\n<tbody>\n'
     '<tr><td><code>x | y</code></td><td style="text-align:center"><a href="u.html">l</a></td></tr>\n</tbody>\n'
     '</table></div>'),
    ("text:\n- a\n  more\n  - b\nlazy\n- c", "<p>text:</p>\n<ul>\n<li>a\nmore\n<ul>\n<li>b\nlazy</li>\n</ul></li>\n"
     "<li>c</li>\n</ul>"),
    ("1. one\n\n   ```rust\n   let a = \"s\"; // c\n   ```\n2. two",
     '<ol>\n<li><p>one</p>\n<pre class="code"><code class="lang-rust"><span class="k">let</span> a = '
     '<span class="s">"s"</span>; <span class="c">// c</span></code></pre></li>\n<li><p>two</p></li>\n</ol>'),
    ("## Fuß- und Endnoten (SCN)", '<h2 id="fuß--und-endnoten-scn">Fuß- und Endnoten (SCN)<a class="hl" '
     'href="#fuß--und-endnoten-scn" aria-label="Link to this section">#</a></h2>'),
    ("---\n> quoted *text*", "<hr>\n<blockquote>\n<p>quoted <em>text</em></p>\n</blockquote>"),
]


def selftest():
    fails = []
    for md, want in CASES:
        blocks = parse_blocks(md.split("\n"))[0]
        for b in walk(blocks):
            if b["t"] == "h":
                b["id"] = slug(plain(b["text"]))
        got = render(blocks, Inline(TestTargets()))
        if got != want:
            fails.append(f"converter: {md!r}\n    got  {got!r}\n    want {want!r}")
    return fails


# ---------------------------------------------------------------- main
def digest(folder):
    h = hashlib.sha256()
    for p in sorted(folder.rglob("*")):
        if p.is_file():
            h.update(p.relative_to(folder).as_posix().encode() + b"\0" + p.read_bytes())
    return h.hexdigest()


def shown(p):
    try:
        return p.relative_to(ROOT).as_posix()
    except ValueError:
        return str(p)


def run_check(out, summary):
    t0 = time.perf_counter()
    fails = selftest()
    temps, twin = [], None
    try:
        if out is None:
            (ROOT / "build").mkdir(exist_ok=True)
            temps = [tempfile.TemporaryDirectory(prefix="site-check-", dir=ROOT / "build") for _ in range(2)]
            out = Path(temps[0].name)
            # the second build for the byte comparison runs alongside in its own process
            twin = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), "--out", temps[1].name],
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
        site = build(out)
        more, nref = check_site(site)
        fails += more
        if twin is not None:
            log = twin.communicate()[0].decode("utf-8", "replace")
            if twin.returncode:
                fails.append("the second build failed: " + log.strip())
            elif digest(site.out) != digest(Path(temps[1].name)):
                fails.append("two builds of the same sources differ")
        dt = time.perf_counter() - t0
        stats = (f"{len(site.pages) + 1} pages, {nref} internal links and {site.links} Markdown links, "
                 f"{len(site.images)} images")
        if fails:
            print(f"FAIL site: {len(fails)} problems ({stats}; python tools/site.py --check)")
            for f in fails[:60]:
                print("   " + f)
            if len(fails) > 60:
                print(f"   ... and {len(fails) - 60} more")
            return 1
        if summary:
            print(f"site: ok ({stats}; {dt:.1f} s)")
        else:
            for n in site.notes:
                print("note: " + n)
            print(f"site --check: ok ({stats}; built twice, byte-identical: {'yes' if temps else 'not checked'}; "
                  f"{dt:.1f} s)")
        return 0
    finally:
        if twin is not None and twin.poll() is None:
            twin.kill()
            twin.wait()
        for t in temps:
            t.cleanup()


def main(argv=None):
    for s in (sys.stdout, sys.stderr):
        try:
            s.reconfigure(encoding="utf-8", errors="replace")
        except (AttributeError, ValueError):
            pass
    ap = argparse.ArgumentParser(description="the static website of JFLib: landing page, docs, API reference")
    ap.add_argument("--out", help="output folder (default build/site; --check: a temporary folder)")
    ap.add_argument("--check", action="store_true", help="build and check links, anchors, images, HTML, determinism")
    ap.add_argument("--summary", action="store_true", help="--check: one line on success (build_rs.py test)")
    a = ap.parse_args(argv)
    out = Path(a.out).resolve() if a.out else None
    try:
        if a.check:
            return run_check(out, a.summary)
        t0 = time.perf_counter()
        out = out or ROOT / "build" / "site"
        site = build(out)
        for e in site.errors:
            print("warning: " + e)
        total = sum((out / f).stat().st_size for f in site.files)
        print(f"site: {shown(out)}/ ({len(site.pages) + 1} pages, {len(site.images)} images, {total / 1e6:.1f} MB) "
              f"in {time.perf_counter() - t0:.1f} s")
        print(f"open {(out / 'index.html').as_uri()}")
        return 0
    except SiteError as e:
        print(f"site: {e}")
        return 1


if __name__ == "__main__":
    sys.exit(main())
