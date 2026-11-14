#!/usr/bin/env python3
"""changelog.py -- CHANGELOG.md (Keep a Changelog 1.1.0) and the library's releases

    python build_rs.py changelog                         show [Unreleased]
    python build_rs.py changelog "TEXT" [--type T] [--area A] [--crate C,..]
                                                         add "- **A:** TEXT" under [Unreleased] / ### T
                                                         (T: added, changed, deprecated, removed,
                                                         fixed, security; default added); with
                                                         --crate also "- TEXT" in each crate's
                                                         CHANGELOG.md (A defaults to their names)
    python build_rs.py changelog --crate C               show [Unreleased] of that crate
    python build_rs.py release X.Y.Z [--date D] [--force]
                                                         cut the release: [Unreleased] becomes
                                                         "## [X.Y.Z] - D" here and in every crate's
                                                         CHANGELOG.md, the version X.Y.Z in the
                                                         root Cargo.toml and Cargo.lock, the
                                                         API snapshot docs/api/X.Y.Z.txt

The changelog is the library's (lib/, styles/, the build and its tools, docs); apps keep their
own versions (version= in apps/NAME/PACKAGE) and notes.  `release` checks the step against the
API changes since the last snapshot (tools/apidiff.py): breaking changes need the next MAJOR
(0.x: the next MINOR), additions the next MINOR (0.x: PATCH); --force cuts it anyway.  It never
commits or tags: `git tag vX.Y.Z` is the maintainer's step.

Every published crate also has a README.md written by hand, the page crates.io and docs.rs show
(what the crate is, how it is used, a section "What's new in X.Y.Z"), and a CHANGELOG.md of its
own in this file's format, its entries written from the crate's side (docs/CRATES.md, "README
und CHANGELOG je Crate").  crate_doc_problems checks both (build_rs.py check, tools/publish.py);
`release` refuses READMEs whose "What's new" names another version.
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
# the crates' own files (the module docstring)
REPO = "https://github.com/JFLibrary/JFLib"
NO_CHANGES = "No changes in this crate"            # the text of an [Unreleased] without entries
WHATS_NEW = re.compile(r"^## What's new in (\S+)[ \t]*$", re.M)
PLACEHOLDER = "<!-- write this README by hand"     # build_rs.py new --lib
LINK = re.compile(r"\]\(\s*<?([^)\s>]+)")
# a version a README names in an example: jflib = "0.2", jflib-build = { version = "=0.2.0" }, jflib-0.2.*
VERSION_USE = re.compile(r'\bjflib[a-z0-9_-]*\s*=\s*(?:\{[^}\n]*?version\s*=\s*)?"=?(\d+\.\d+)|\bjflib[a-z0-9-]*-(\d+\.\d+)\.\*')
FENCE = re.compile(r"^```.*?^```", re.M | re.S)


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
    if head.startswith(NO_CHANGES):                 # a crate's "no changes" gives way to its first entry
        head = ""
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


# ---------------------------------------------------------------- the crates' own files
def without_entries(text):
    """whether [Unreleased] holds no entry (empty, or only the "no changes" text)"""
    body = unreleased(text).strip()
    return not body or (body.startswith(NO_CHANGES) and "\n### " not in "\n" + body)


def cut_crate(text, version, date):
    """a crate's CHANGELOG.md cut as cut() does: an [Unreleased] without entries becomes the
    release's "no changes" text, and the link references follow ([Unreleased] compares from the new
    tag, [version] compares from where [Unreleased] did, or names the tag)"""
    if without_entries(text):
        a, b = unreleased_span(text)
        text = (text[:a] + f"\n{NO_CHANGES}. It was released with the others because all of JFLib's crates "
                f"move together.\n\n" + text[b:].lstrip("\n"))
    text = cut(text, version, date)
    m = re.search(r"^\[Unreleased\]: *(\S+)[ \t]*$", text, re.M)
    if m:
        base = re.search(r"/compare/(.+)\.\.\.HEAD$", m.group(1))
        ref = f"{REPO}/compare/{base.group(1)}...v{version}" if base else f"{REPO}/releases/tag/v{version}"
        text = text[:m.start()] + f"[Unreleased]: {REPO}/compare/v{version}...HEAD\n[{version}]: {ref}" + text[m.end():]
    return text


def crate_log(package, version):
    """the CHANGELOG.md of a new crate (build_rs.py new --lib): its first release is the next one"""
    return (f"# Changelog of {package}\n\n"
            "What changed in this crate. All of JFLib's crates share one version and are released together;\n"
            f"the [library's changelog]({REPO}/blob/main/CHANGELOG.md) has the\n"
            "whole picture. The format follows [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/),\n"
            "versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html) as JFLib's\n"
            f"[VERSIONING.md]({REPO}/blob/main/docs/VERSIONING.md) describes.\n\n"
            "## [Unreleased]\n\nFirst release on crates.io.\n\n"
            f"[Unreleased]: {REPO}/compare/v{version}...HEAD\n")


def crates(root=ROOT):
    """the published packages -> [(folder, package, lib name)] in the order of the root manifest"""
    import manifests
    out = []
    for member in manifests.workspace(root)["members"]:
        m = manifests.read(Path(root) / member / "Cargo.toml")
        pkg = m.get("package", {})
        if pkg.get("publish", True) is False:
            continue
        out.append((member, pkg["name"], m.get("lib", {}).get("name", pkg["name"].replace("-", "_"))))
    return out


def crate_of(name, root=ROOT):
    """a crate named by its folder (lib/ui), package (jflib-ui), lib name (ui) or folder name (kde)
    -> (folder, package, lib name)"""
    for c in crates(root):
        if name in (*c, c[0].rsplit("/", 1)[-1]):
            return c
    raise ChangelogError(f"no published crate {name} (a folder, package or lib name)")


def crate_doc_problems(root=ROOT, release=None, cut=False):
    """every published crate's README.md and CHANGELOG.md -> problems: both there and in its package,
    the README's first line `# PACKAGE`, no placeholder of build_rs.py new --lib, only absolute links
    (crates.io and docs.rs show the text away from the repository), one "## What's new in X.Y.Z" not
    older than the library's version (with `release`: that version) and examples that name X.Y
    (`jflib = "X.Y"`, `jflib-X.Y.*`), a CHANGELOG with [Unreleased]
    and only releases this file has (with `cut`: a section [release])"""
    import apidiff
    import manifests
    root = Path(root)
    lib_version = manifests.version(root)
    known = set(releases(read(root / "CHANGELOG.md")[0]))
    bad = []
    for member, pkg, _ in crates(root):
        folder = root / member
        m = manifests.read(folder / "Cargo.toml")["package"]
        if m.get("readme", "README.md") != "README.md":
            bad.append(f"{member}/Cargo.toml: readme = {m['readme']!r}, the crate's page is README.md")
        for name in ("README.md", "CHANGELOG.md"):
            if not (folder / name).is_file():
                bad.append(f'{member}/{name} is missing (docs/CRATES.md, "README und CHANGELOG je Crate")')
            elif "include" in m and name not in m["include"]:
                bad.append(f"{member}/Cargo.toml: include lacks {name}, the package would not carry it")
        if (folder / "README.md").is_file():
            text = read(folder / "README.md")[0]
            if text.split("\n", 1)[0].strip() != f"# {pkg}":
                bad.append(f"{member}/README.md: the first line is not '# {pkg}'")
            if PLACEHOLDER in text:
                bad.append(f"{member}/README.md: still the placeholder of build_rs.py new --lib: write the README")
            for url in LINK.findall(FENCE.sub("", text)):
                if not url.startswith(("https://", "http://", "mailto:", "#")):
                    bad.append(f"{member}/README.md: the link {url} is relative (crates.io and docs.rs show the text elsewhere)")
            new = WHATS_NEW.findall(text)
            if len(new) != 1:
                bad.append(f"{member}/README.md: needs one section \"## What's new in X.Y.Z\" (has {len(new)})")
            elif release is not None and new[0] != release:
                bad.append(f"{member}/README.md: \"What's new in {new[0]}\", the release is {release}")
            elif release is None and apidiff.parse_version(new[0]) < apidiff.parse_version(lib_version):
                bad.append(f"{member}/README.md: \"What's new in {new[0]}\" is older than the library's version {lib_version}")
            if len(new) == 1:                       # the examples name the version the README is for
                want = ".".join(new[0].split(".")[:2])
                for v in sorted({m.group(1) or m.group(2) for m in VERSION_USE.finditer(text)} - {want}):
                    bad.append(f"{member}/README.md: an example names version {v}, the README is for {new[0]}")
        if (folder / "CHANGELOG.md").is_file():
            text = read(folder / "CHANGELOG.md")[0]
            if not UNRELEASED.search(text):
                bad.append(f"{member}/CHANGELOG.md has no '## [Unreleased]' section")
            own = releases(text)
            bad += [f"{member}/CHANGELOG.md: [{v}] is no release of CHANGELOG.md" for v in own if v not in known]
            if cut and release not in own:
                bad.append(f"{member}/CHANGELOG.md has no [{release}]: python build_rs.py release {release}")
    return bad


def release(version, date=None, force=False):
    """the release steps (see the module docstring) -> the lines to print"""
    import apidiff
    import manifests
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
    # the crates' files: every README tells what is new in this version, every CHANGELOG can be cut
    docs = crate_doc_problems(release=version)
    if docs and not force:
        raise ChangelogError("\n".join(docs) + "\n(--force cuts it anyway)")
    out += [f"warning: {p}" for p in docs]
    logs = []
    for member, _, _ in crates():
        path = ROOT / member / "CHANGELOG.md"
        if path.is_file():
            log, log_crlf = read(path)
            if version in releases(log):
                raise ChangelogError(f"{member}/CHANGELOG.md has a section [{version}] already")
            logs.append((path, log, log_crlf))
    write(cut(text, version, date), crlf)
    quiet = 0
    for path, log, log_crlf in logs:
        quiet += without_entries(log)
        write(cut_crate(log, version, date), log_crlf, path)
    manifests.set_version(version)
    snap = apidiff.write_snapshot(version)
    out += [f"CHANGELOG.md: [Unreleased] -> [{version}] - {date}",
            f"the crates' CHANGELOG.md: {len(logs)} cut, {quiet} of them without changes of their own",
            f"Cargo.toml, Cargo.lock: {version}",
            f"{snap.relative_to(ROOT).as_posix()}: the API of {version}",
            f"next: review, commit, then `git tag v{version}`"]
    return out


def main(argv=None):
    import argparse
    ap = argparse.ArgumentParser(description="CHANGELOG.md: show or add to [Unreleased]")
    ap.add_argument("text", nargs="?")
    ap.add_argument("--type", default="added", help=", ".join(t.lower() for t in TYPES))
    ap.add_argument("--area", help="the part it is about (ui, plat, build, ...); with --crate their lib names")
    ap.add_argument("--crate", help="the crates it changes, comma-separated (folder, package or lib name): "
                                    "the entry goes into their CHANGELOG.md too, written for them")
    a = ap.parse_args(argv)
    try:
        chosen = [crate_of(n) for n in (a.crate or "").replace(",", " ").split()]
        logs = []
        for member, _, _ in chosen:
            path = ROOT / member / "CHANGELOG.md"
            if not path.is_file():
                raise ChangelogError(f"{member} has no CHANGELOG.md")
            logs.append((path, *read(path)))
        if a.text is None:
            if not logs:
                print(unreleased(read()[0]) or "[Unreleased] is empty")
            for path, log, _ in logs:
                print(f"{path.relative_to(ROOT).as_posix()}:\n{unreleased(log) or '[Unreleased] is empty'}")
            return 0
        text, crlf = read()
        area = a.area or (", ".join(c[2] for c in chosen) if chosen else None)
        new = [(path, add_entry(log, a.text, a.type), log_crlf) for path, log, log_crlf in logs]
        write(add_entry(text, a.text, a.type, area), crlf)
        for path, log, log_crlf in new:
            write(log, log_crlf, path)
        print(f"CHANGELOG.md: [Unreleased] / {a.type.capitalize()}: {a.text}")
        for path, _, _ in new:
            print(f"{path.relative_to(ROOT).as_posix()}: [Unreleased] / {a.type.capitalize()}: the same text "
                  "(reword it there from the crate's side where that reads better)")
        return 0
    except ChangelogError as e:
        print(e)
        return 1


if __name__ == "__main__":
    sys.exit(main())
