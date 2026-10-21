#!/usr/bin/env python3
"""changelog.py -- CHANGELOG.md (Keep a Changelog 1.1.0) and the library's releases

    python build_rs.py changelog                         show [Unreleased]
    python build_rs.py changelog "TEXT" [--type T] [--area A]
                                                         add "- **A:** TEXT" under [Unreleased] / ### T
                                                         (T: added, changed, deprecated, removed,
                                                         fixed, security; default added)
    python build_rs.py release X.Y.Z [--date D] [--force]
                                                         cut the release: [Unreleased] becomes
                                                         "## [X.Y.Z] - D", lib/VERSION = X.Y.Z, the
                                                         API snapshot docs/api/X.Y.Z.txt

The changelog is the library's (lib/, styles/, the build and its tools, docs); apps keep their
own versions (version= in apps/NAME/PACKAGE) and notes.  `release` checks the step against the
API changes since the last snapshot (tools/apidiff.py): breaking changes need the next MAJOR
(0.x: the next MINOR), additions the next MINOR (0.x: PATCH); --force cuts it anyway.  It never
commits or tags: `git tag vX.Y.Z` is the maintainer's step.
"""
import datetime
import re
import sys
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))
ROOT = TOOLS.parent
FILE = ROOT / "CHANGELOG.md"
TYPES = ("Added", "Changed", "Deprecated", "Removed", "Fixed", "Security")
UNRELEASED = re.compile(r"^## \[Unreleased\][^\n]*\n", re.M)
RELEASE = re.compile(r"^## \[(\d+\.\d+\.\d+[^\]]*)\]", re.M)


class ChangelogError(Exception):
    pass


def read(path=FILE):
    raw = Path(path).read_bytes().decode("utf-8")
    return raw.replace("\r\n", "\n"), "\r\n" in raw


def write(text, crlf, path=FILE):
    Path(path).write_bytes((text.replace("\n", "\r\n") if crlf else text).encode("utf-8"))


def unreleased_span(text):
    """(start of the body, end) of the [Unreleased] section"""
    m = UNRELEASED.search(text)
    if not m:
        raise ChangelogError("CHANGELOG.md has no '## [Unreleased]' section")
    nxt = re.search(r"^## \[", text[m.end():], re.M)
    return m.end(), m.end() + nxt.start() if nxt else len(text)


def unreleased(text):
    a, b = unreleased_span(text)
    return text[a:b].strip("\n")


def releases(text):
    return RELEASE.findall(text)


def add_entry(text, entry, kind="added", area=None):
    """text with "- entry" at the end of [Unreleased] / ### Kind (the subsection made in the
    order of TYPES when it is missing)"""
    kind = kind.capitalize()
    if kind not in TYPES:
        raise ChangelogError(f"unknown type {kind.lower()} ({', '.join(t.lower() for t in TYPES)})")
    entry = " ".join(entry.split())
    if not entry:
        raise ChangelogError("the entry is empty")
    line = f"- **{area}:** {entry}" if area else f"- {entry}"
    a, b = unreleased_span(text)
    body = text[a:b].strip("\n")
    parts = re.split(r"^### ", body, flags=re.M) if body else [""]
    head, subs = parts[0].strip("\n"), {}
    for p in parts[1:]:
        title, _, rest = p.partition("\n")
        subs[title.strip()] = rest.strip("\n")
    subs[kind] = (subs.get(kind, "") + "\n" + line).strip("\n")
    order = [t for t in TYPES if t in subs] + [t for t in subs if t not in TYPES]
    out = ([head, ""] if head else []) + [s for t in order for s in (f"### {t}", "", subs[t], "")]
    return text[:a] + "\n" + "\n".join(out) + "\n" + text[b:].lstrip("\n")


def cut(text, version, date):
    """text with [Unreleased] renamed to [version] - date and a new, empty [Unreleased] above"""
    if version in releases(text):
        raise ChangelogError(f"CHANGELOG.md has a section [{version}] already")
    m = UNRELEASED.search(text)
    if not m:
        raise ChangelogError("CHANGELOG.md has no '## [Unreleased]' section")
    return text[:m.start()] + f"## [Unreleased]\n\n## [{version}] - {date}\n" + text[m.end():]


def release(version, date=None, force=False):
    """the release steps (see the module docstring) -> the lines to print"""
    import apidiff
    date = date or datetime.date.today().isoformat()
    try:
        apidiff.parse_version(version)
        datetime.date.fromisoformat(date)
    except ValueError as e:
        raise ChangelogError(str(e))
    text, crlf = read()
    out = []
    base, path = apidiff.baseline()
    if base is not None:
        if apidiff.parse_version(version) <= apidiff.parse_version(base):
            raise ChangelogError(f"{version} is not newer than the last release {base}")
        d = apidiff.diff(apidiff.read_snapshot(path), apidiff.current_lines())
        level = apidiff.level_of(d)
        out.append(apidiff.summary(d, base))
        if not apidiff.version_ok(version, base, level):
            msg = (f"the changes since {base} need {apidiff.next_version(base, level)} or later "
                   f"({level}); `python build_rs.py api-diff` lists them")
            if not force:
                raise ChangelogError(msg + "; --force cuts it anyway")
            out.append("warning: " + msg)
    if not unreleased(text).strip():
        out.append("note: [Unreleased] is empty")
    write(cut(text, version, date), crlf)
    (ROOT / "lib" / "VERSION").write_bytes(f"{version}\n".encode())
    snap = apidiff.write_snapshot(version)
    out += [f"CHANGELOG.md: [Unreleased] -> [{version}] - {date}", f"lib/VERSION: {version}",
            f"{snap.relative_to(ROOT).as_posix()}: the API of {version}",
            f"next: review, commit, then `git tag v{version}`"]
    return out


def main(argv=None):
    import argparse
    ap = argparse.ArgumentParser(description="CHANGELOG.md: show or add to [Unreleased]")
    ap.add_argument("text", nargs="?")
    ap.add_argument("--type", default="added", help=", ".join(t.lower() for t in TYPES))
    ap.add_argument("--area", help="the part it is about (ui, plat, build, ...)")
    a = ap.parse_args(argv)
    try:
        text, crlf = read()
        if a.text is None:
            print(unreleased(text) or "[Unreleased] is empty")
            return 0
        write(add_entry(text, a.text, a.type, a.area), crlf)
        print(f"CHANGELOG.md: [Unreleased] / {a.type.capitalize()}: {a.text}")
        return 0
    except ChangelogError as e:
        print(e)
        return 1


if __name__ == "__main__":
    sys.exit(main())
