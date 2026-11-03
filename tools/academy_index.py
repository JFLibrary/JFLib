"""academy_index.py -- the list of JFNetworker's courses: what can be installed, and the checksums of its files

    python tools/academy_index.py                writes apps/JFNetworker/academy/index.json from the folders
    python tools/academy_index.py --check        fails (exit 1) when that file is stale, writes nothing
    python tools/academy_index.py --dir DIR ...  another folder with courses (an author's own)

Every folder below DIR with a course.json is a course; a folder whose name starts with a dot is none (an
installation in progress).  The index holds the format, its version and per course its id, title and summary in
both languages, level, duration, the version of its content (course_version), whether it is a sample ("sample":
true in course.json) and every file with path, size and SHA-256 of the bytes on disk.  Stable order: courses by id,
files by path; LF, UTF-8, no byte order mark, so that the same folders give the same bytes on every system.  The
courses' files are "-text" in .gitattributes, so the checksums hold after a checkout.

What the app refuses is refused here first, with exit 2: names of folders and files outside the rules of
jfn_model::academy (safe_id, safe_path), a course.json that is no JSON or of another format.  The app's own selftest
(--academy-dir, group install) checks the file again: every file against the index, every course with jfn-sim
course's checks.  Exit codes: 0 fine, 1 --check found it stale, 2 a folder is not in order.
"""
import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ACADEMY = ROOT / "apps" / "JFNetworker" / "academy"
INDEX_FILE = "index.json"
INDEX_FORMAT = "jfn-academy"
COURSE_FORMAT = "jfn-course"
INDEX_VERSION = 1
COURSE_FILE = "course.json"
ID_MAX = 64
PATH_MAX = 200
RESERVED = re.compile(r"^(con|prn|aux|nul|com[0-9]|lpt[0-9])$")


class Bad(Exception):
    """a folder or file that is not in order: the message says which"""


def reserved(part):
    return bool(RESERVED.match(part.split(".")[0].lower()))


def safe_id(s):
    return (0 < len(s) <= ID_MAX and bool(re.fullmatch(r"[a-z0-9][a-z0-9_-]*", s)) and not reserved(s))


def safe_path(p):
    if len(p) > PATH_MAX or not re.fullmatch(r"[A-Za-z0-9._/-]+", p):
        return False
    return all(s and s != ".." and not s.startswith(".") and not s.endswith(".") and not reserved(s) for s in p.split("/"))


def course_entry(folder):
    """a course's folder -> its entry of the index"""
    cid = folder.name
    if not safe_id(cid):
        raise Bad(f"{cid}: a course's folder takes small letters, digits, - and _ only")
    try:
        c = json.loads((folder / COURSE_FILE).read_bytes().decode("utf-8-sig"))
    except (OSError, ValueError) as e:
        raise Bad(f"{cid}/{COURSE_FILE}: {e}") from None
    if not isinstance(c, dict) or c.get("format") != COURSE_FORMAT:
        raise Bad(f"{cid}/{COURSE_FILE}: no course (format {COURSE_FORMAT!r})")
    files = []
    seen = {}
    for f in sorted(p for p in folder.rglob("*") if not p.is_dir()):
        rel = f.relative_to(folder).as_posix()
        if f.is_symlink() or not f.is_file():
            raise Bad(f"{cid}/{rel}: only plain files belong to a course")
        if not safe_path(rel):
            raise Bad(f"{cid}/{rel}: letters, digits and - _ . / only, no part with a dot at its start or end, no device names")
        if rel.lower() in seen:
            raise Bad(f"{cid}/{rel}: on some systems the same file as {seen[rel.lower()]}")
        seen[rel.lower()] = rel
        data = f.read_bytes()
        files.append({"path": rel, "size": len(data), "sha256": hashlib.sha256(data).hexdigest()})
    files.sort(key=lambda e: e["path"])
    entry = {"id": cid, "title": c.get("title", ""), "summary": c.get("summary", ""), "level": c.get("level", "beginner"),
             "minutes": c.get("minutes", 0), "version": c.get("course_version", 0)}
    if c.get("sample"):
        entry["sample"] = True
    entry["files"] = files
    return entry


def index_of(root):
    """the folder's courses -> the index as a dict"""
    courses = []
    for d in sorted(p for p in root.iterdir() if p.is_dir() and not p.name.startswith(".")):
        if not (d / COURSE_FILE).is_file():
            raise Bad(f"{d.name}: no {COURSE_FILE}")
        courses.append(course_entry(d))
    return {"format": INDEX_FORMAT, "version": INDEX_VERSION, "courses": courses}


def text_of(index):
    """the bytes of index.json: a line per file, so that a change reads well in a diff"""
    dump = lambda v: json.dumps(v, ensure_ascii=False)                         # noqa: E731
    out = ["{", f'  "format": {dump(index["format"])},', f'  "version": {index["version"]},', '  "courses": [']
    for i, c in enumerate(index["courses"]):
        out.append("    {")
        for k, v in c.items():
            if k != "files":
                out.append(f'      "{k}": {dump(v)},')
        out.append('      "files": [')
        for j, f in enumerate(c["files"]):
            out.append(f'        {dump(f)}' + ("," if j + 1 < len(c["files"]) else ""))
        out.append("      ]")
        out.append("    }" + ("," if i + 1 < len(index["courses"]) else ""))
    out += ["  ]", "}", ""]
    return "\n".join(out).encode("utf-8")


def differences(old, new):
    """what changed between the file's text and the folders, by course"""
    try:
        before = {c["id"]: c for c in json.loads(old.decode("utf-8"))["courses"]}
    except (ValueError, KeyError, TypeError):
        return ["the file is no index"]
    after = {c["id"]: c for c in new["courses"]}
    out = [f"{i}: only in the folders" for i in sorted(after.keys() - before.keys())]
    out += [f"{i}: only in the file" for i in sorted(before.keys() - after.keys())]
    for i in sorted(after.keys() & before.keys()):
        a, b = {f["path"]: f for f in before[i].get("files", [])}, {f["path"]: f for f in after[i]["files"]}
        out += [f"{i}/{p}: changed or new" for p in sorted(b) if a.get(p) != b[p]]
        out += [f"{i}/{p}: gone" for p in sorted(a.keys() - b.keys())]
        out += [f"{i}: its {k} changed" for k in after[i] if k != "files" and before[i].get(k) != after[i][k]]
    return out or ["only the form of the file"]


def main():
    ap = argparse.ArgumentParser(description="the list of JFNetworker's courses (index.json)")
    ap.add_argument("--check", action="store_true", help="fail when the file is stale; writes nothing")
    ap.add_argument("--dir", type=Path, default=ACADEMY, help="the folder with the courses")
    a = ap.parse_args()
    try:
        index = index_of(a.dir)
    except (Bad, OSError) as e:
        print(f"academy_index: {e}", file=sys.stderr)
        return 2
    text = text_of(index)
    target = a.dir / INDEX_FILE
    if a.check:
        old = target.read_bytes() if target.is_file() else b""
        if old != text:
            print(f"academy_index: {target} is stale: python tools/academy_index.py", file=sys.stderr)
            for d in differences(old, index):
                print(f"  {d}", file=sys.stderr)
            return 1
        print(f"academy_index: {len(index['courses'])} courses, up to date")
        return 0
    target.write_bytes(text)
    print(f"academy_index: {target}: {len(index['courses'])} courses, {sum(len(c['files']) for c in index['courses'])} files")
    return 0


if __name__ == "__main__":
    sys.exit(main())
