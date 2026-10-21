#!/usr/bin/env python3
"""apidiff.py -- the library's public API as a snapshot, and what changed since a release.

    python build_rs.py api-diff                  compare with the newest docs/api/X.Y.Z.txt
    python build_rs.py api-diff --against 0.1.0  ... with that release
    python build_rs.py api-diff --snapshot       write docs/api/<lib/VERSION>.txt (a release)
    python tools/gendoc.py --check --api         the suite's one-line summary (one parse)

The public API (docs/VERSIONING.md) is what docs/API.md documents: the crates of gendoc's
CRATES table (codec crates: their root, their re-exports and CODEC_EXTRA), every item another
crate can name, minus #[doc(hidden)] and #[cfg(test)].  gendoc's parser reads the sources; this
file walks the modules again to find each item's public paths and keeps the shortest one, so
moving an item between modules that are glob re-exported (ui's files) changes nothing.

Snapshot lines (sorted; # starts a comment):  PATH <TAB> KIND <TAB> SIGNATURE <TAB> FLAGS
  KIND   fn, method, const, static, type, macro, struct, union, enum, variant, field, trait,
         trait-fn / trait-const / trait-type (a member of a public trait), impl (a derive or an
         `impl Trait for Type` of a public type; SIGNATURE names the trait), use (a re-export of
         another crate's item)
  FLAGS  comma list: windows / linux (the item's cfg), deprecated, provided (a trait member with a
         default), all-pub (a struct whose fields are all public), non-exhaustive
A line's identity is PATH, KIND, its cfg and, for impl, the trait; signatures compare without
whitespace and trailing commas (rustfmt reflowing a signature is no change).

Classification (diff):
  breaking    removed items, changed signatures, a new variant of an exhaustive enum, a new
              required member of a public trait, a provided member that became required, a
              struct or enum that became non-exhaustive, an item that lost its cfg-free form
  deprecated  items that got #[deprecated] (allowed in any release; removal is breaking)
  added       everything new (a minor release; 0.x: a patch release)
  notes       new public fields of all-pub structs (literal constructions without
              `..X::NONE` / `..Default::default()` stop compiling: the library's structs offer
              NONE / Default and the guide asks for them, so this is no break here)
The needed version: breaking -> next MAJOR (0.x: next MINOR), added / deprecated -> next MINOR
(0.x: next PATCH), nothing -> the same or a PATCH.  api-diff exits 1 when lib/VERSION is lower
than the needed version for breaking changes; `build_rs.py release` refuses a too small step.
"""
import re
import sys
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))
import gendoc  # noqa: E402

ROOT = TOOLS.parent
VERSION_FILE = ROOT / "lib" / "VERSION"
SNAP_DIR = ROOT / "docs" / "api"
SEMVER = re.compile(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?")
TYPE_KINDS = ("struct", "enum", "union", "trait", "type")


# ---------------------------------------------------------------- versions
def parse_version(v):
    """'1.2.3' / '1.2.3-rc.1+b' -> (1, 2, 3, pre) for comparisons (a pre-release sorts first)"""
    m = SEMVER.fullmatch(v.strip())
    if not m:
        raise ValueError(f"{v!r} is no MAJOR.MINOR.PATCH version")
    pre = m.group(4)
    return int(m.group(1)), int(m.group(2)), int(m.group(3)), (1, "") if pre is None else (0, pre)


def lib_version():
    return VERSION_FILE.read_text(encoding="utf-8").strip()


def next_version(base, level):
    """the smallest version after base that may carry `level` (breaking / added / none)"""
    ma, mi, pa, _ = parse_version(base)
    if level == "breaking":
        return f"{ma + 1}.0.0" if ma else f"0.{mi + 1}.0"
    if level == "added":
        return f"{ma}.{mi + 1}.0" if ma else f"0.{mi}.{pa + 1}"
    return f"{ma}.{mi}.{pa + 1}"


def level_of(d):
    return "breaking" if d["breaking"] else "added" if d["added"] or d["deprecated"] else "none"


def version_ok(version, base, level):
    """whether `version` may carry the changes of `level` made since `base`"""
    if level == "none":
        return parse_version(version) >= parse_version(base)
    return parse_version(version) >= parse_version(next_version(base, level))


def snapshots():
    """[(version, path)] of docs/api, oldest first"""
    out = []
    for p in SNAP_DIR.glob("*.txt") if SNAP_DIR.is_dir() else []:
        try:
            out.append((parse_version(p.stem), p.stem, p))
        except ValueError:
            continue
    return [(v, p) for _, v, p in sorted(out)]


def snapshot_path(version):
    return SNAP_DIR / f"{version}.txt"


# ---------------------------------------------------------------- the snapshot
def canon(sig, kind=""):
    """a signature without layout: whitespace only between two word characters, no trailing
    commas; a function's parameter names are no part of it (renaming `_id` to `id` is no
    change for a caller)"""
    s = re.sub(r"\s+", " ", sig).strip()
    if kind in ("fn", "method", "trait-fn"):
        s = param_types(s)
    s = re.sub(r"\s+(?=\W)|(?<=\W)\s+", "", s)
    return re.sub(r",(?=[)\]}>])", "", s)


def param_types(sig):
    """'fn f(a: u32, (x, y): (i32, i32), &mut self)' -> 'fn f(u32, (i32, i32), &mut self)'"""
    m = re.search(r"\bfn\s+\w+\s*", sig)
    if not m:
        return sig

    def close(s, i):
        """the index of the bracket that closes the one at i ('->' is no bracket)"""
        depth = 0
        for k in range(i, len(s)):
            ch = s[k]
            if ch in "([{<":
                depth += 1
            elif ch in ")]}" or (ch == ">" and s[k - 1] != "-"):
                depth -= 1
            if depth == 0:
                return k
        return len(s)
    i = m.end()
    if sig[i:i + 1] == "<":                         # fn f<T: Fn(u32)>(..): past the generics
        i = close(sig, i) + 1
    i = sig.find("(", i)
    if i < 0:
        return sig
    j = close(sig, i)
    params, cur, depth = [], "", 0
    for k in range(i + 1, j):
        ch = sig[k]
        if ch in "([{<":
            depth += 1
        elif ch in ")]}" or (ch == ">" and sig[k - 1] != "-"):
            depth -= 1
        if ch == "," and depth == 0:
            params.append(cur)
            cur = ""
        else:
            cur += ch
    params.append(cur)
    out = []
    for p in (p.strip() for p in params):
        if not p:
            continue
        depth, cut = 0, -1
        for k, ch in enumerate(p):
            if ch in "([{<":
                depth += 1
            elif ch in ")]}" or (ch == ">" and p[k - 1:k] != "-"):
                depth -= 1
            elif ch == ":" and depth == 0 and p[k + 1:k + 2] != ":" and p[k - 1:k] != ":":
                cut = k
                break
        out.append(p[cut + 1:].strip() if cut >= 0 else p)
    return sig[:i + 1] + ", ".join(out) + sig[j:]


def bare(sig):
    """'pub(crate) unsafe fn x' -> 'unsafe fn x'; one line, no trailing ';' / '{'"""
    s = gendoc.squash(sig)
    s = re.sub(r"^pub(\s*\([^)]*\))?\s+", "", s)
    return s.rstrip(";{ ").strip()


def item_sig(it):
    if it.kind == "macro":
        return f"macro_rules! {it.name}"
    s = bare(it.sig)
    if it.kind in ("const", "static"):
        s = re.sub(r"\s*=(?!=).*$", "", s)        # the value is no part of the API
    return s


def cfg_flag(cfg):
    return {"Windows": "windows", "Linux": "linux"}.get(cfg, "")


def flags_of(it, extra=()):
    f = [cfg_flag(it.cfg)] + list(extra)
    if any(a.startswith("#[deprecated") for a in it.attrs):
        f.append("deprecated")
    if any("non_exhaustive" in a for a in it.attrs):
        f.append("non-exhaustive")
    return [x for x in f if x]


def hidden(it):
    return any(re.match(r"#\[doc\s*\(\s*hidden", a) for a in it.attrs)


def derives(it):
    out = []
    for a in it.attrs:
        m = re.match(r"#\[derive\s*\((.*)\)\s*\]$", a)
        if m:
            out += [d.strip().split("::")[-1] for d in m.group(1).split(",") if d.strip()]
    return out


def impl_head(sig):
    """'impl<T: Into<String>> From<T> for X' -> 'From<T> for X'"""
    s = re.sub(r"^(unsafe\s+)?impl\s*", "", gendoc.squash(sig))
    if s.startswith("<"):
        depth = 0
        for k, ch in enumerate(s):
            depth += (ch == "<") - (ch == ">")
            if depth == 0:
                return s[k + 1:].strip()
    return s


class PathWalk:
    """the public paths of a crate's items: id(item) -> (item, module, {paths}); external
    re-exports: [(path, text, module)]"""

    def __init__(self, crate):
        self.c = crate
        self.skip = gendoc.SKIP_FILES.get(crate.name, set())
        self.items = {}
        self.external = []
        self.seen = set()
        self.export(crate.root, crate.name)

    def skipped(self, mod):
        """a module of SKIP_FILES (rt::os: the OS calls plat is built on) or inside one"""
        while mod is not None:
            if mod.path.name in self.skip:
                return True
            mod = mod.parent
        return False

    def add(self, it, mod, path):
        self.items.setdefault(id(it), (it, mod, set()))[2].add(path)

    def export(self, mod, path):
        """mod's public names are visible as path::NAME"""
        key = (id(mod), path)
        if key in self.seen or self.skipped(mod):
            return
        self.seen.add(key)
        for it in mod.items:
            if gendoc.is_exported_macro(it):
                self.add(it, mod, f"{self.c.name}::{it.name}")
            elif it.vis == "pub" and it.kind != "impl" and it.name:
                self.add(it, mod, f"{path}::{it.name}")
        for name, ch in mod.children.items():
            if ch.vis == "pub":
                self.export(ch, f"{path}::{name}")
        for vis, trees, text in mod.uses:
            if vis != "pub":
                continue
            for segs, alias, glob in trees:
                target, rest = self.c.resolve(mod, segs)
                if target is None:
                    name = "*" if glob else (alias or (segs[-1] if segs else ""))
                    if name and name != "self":
                        self.external.append((f"{path}::{name}", "::".join(segs) + ("::*" if glob else ""), mod))
                elif glob and not rest:
                    self.export(target, path)
                elif not glob and len(rest) == 1:
                    self.mark(target, rest[0], f"{path}::{alias or rest[0]}")
                elif not glob and not rest:
                    self.export(target, f"{path}::{alias or segs[-1]}")

    def mark(self, mod, name, path, depth=0):
        """the item `name` of mod, re-exported as path"""
        found = False
        for it in mod.items:
            if it.name == name and it.kind != "impl":
                self.add(it, mod, path)
                found = True
        if name in mod.children:
            self.export(mod.children[name], path)
            found = True
        if found or depth > 8:
            return
        for vis, trees, text in mod.uses:
            for segs, alias, glob in trees:
                target, rest = self.c.resolve(mod, segs)
                if target is None:
                    if not glob and (alias or (segs[-1] if segs else "")) == name:
                        self.external.append((path, "::".join(segs), mod))
                    continue
                if glob and not rest:
                    self.mark(target, name, path, depth + 1)
                elif not glob and len(rest) == 1 and (alias or rest[0]) == name:
                    self.mark(target, rest[0], path, depth + 1)


def best(paths):
    return min(paths, key=lambda p: (p.count("::"), p))


def crate_lines(crate, kind="full"):
    """the snapshot lines of one crate: [(path, kind, sig, flags)]"""
    walk = PathWalk(crate)
    extra = gendoc.CODEC_EXTRA.get(crate.name, {})
    shown = {}                                      # id(item) -> (item, module, path)
    for it, mod, paths in walk.items.values():
        if hidden(it):
            continue
        if kind == "codec" and not (mod is crate.root or id(it) in crate.rexp
                                    or extra.get(mod.name) == "*" or it.name in extra.get(mod.name, [])):
            continue
        shown[id(it)] = (it, mod, best(paths))
    types = {}                                      # name -> [(module, path)]
    for it, mod, path in shown.values():
        if it.kind in TYPE_KINDS:
            types.setdefault(it.name, []).append((mod, path))
    out = []
    for it, mod, path in shown.values():
        sig = item_sig(it)
        if it.kind in ("struct", "union"):
            fields = [f for f in it.fields]
            all_pub = it.body is not None and all(v == "pub" for v, _, _, _ in fields) \
                and not any("non_exhaustive" in a for a in it.attrs)
            out.append((path, it.kind, sig, flags_of(it, ["all-pub"] if all_pub and fields else [])))
            for vis, text, _, _ in fields:
                if vis == "pub":
                    out.append((f"{path}.{text.split(':')[0].strip()}", "field", bare(text), flags_of(it)))
        elif it.kind == "enum":
            out.append((path, "enum", sig, flags_of(it)))
            for _, text, _, _ in it.fields:
                vname = re.match(r"[A-Za-z_]\w*", text)
                if vname:
                    out.append((f"{path}::{vname.group(0)}", "variant", re.sub(r"\s*=.*$", "", bare(text)), flags_of(it)))
        elif it.kind == "trait":
            out.append((path, "trait", sig, flags_of(it)))
            for m in it.members:
                if m.kind not in ("fn", "const", "type") or hidden(m):
                    continue
                provided = m.body is not None if m.kind == "fn" else "=" in m.sig
                msig = bare(m.sig)
                if m.kind == "const":
                    msig = re.sub(r"\s*=(?!=).*$", "", msig)
                out.append((f"{path}::{m.name}", f"trait-{m.kind}", msig,
                            flags_of(it) + flags_of(m) + (["provided"] if provided else [])))
        else:
            out.append((path, it.kind, sig, flags_of(it)))
        for d in derives(it) if it.kind in ("struct", "union", "enum") else []:
            out.append((path, "impl", f"impl {d}", flags_of(it)))
    # methods and trait impls of the public types (impl blocks in any module of the crate)
    for mod in crate.all_modules():
        if walk.skipped(mod):
            continue
        for it in mod.items:
            if it.kind != "impl":
                continue
            ty, is_trait = gendoc.impl_type(it.sig)
            cands = types.get(ty)
            if not cands:
                continue
            tpath = next((p for m, p in cands if m is mod), cands[0][1])
            if is_trait:
                trait = re.split(r"\bfor\b", impl_head(it.sig), maxsplit=1)[0]
                out.append((tpath, "impl", "impl " + trait.strip(), flags_of(it)))
                continue
            for m in it.members:
                if m.vis == "pub" and m.kind in ("fn", "const", "type") and not hidden(m):
                    out.append((f"{tpath}::{m.name}", "method" if m.kind == "fn" else m.kind,
                                item_sig(m), flags_of(it) + flags_of(m)))
    for path, text, mod in walk.external:
        out.append((path, "use", text, [cfg_flag(mod.cfg)] if cfg_flag(mod.cfg) else []))
    return out


def current_lines():
    """the snapshot lines of the library as it is now"""
    out = set()
    for name, _title, kind in gendoc.CRATES:
        for path, k, sig, flags in crate_lines(gendoc.load_crate(name), kind):
            out.add((path, k, sig, ",".join(sorted(set(flags)))))
    return sorted(out)


def write_snapshot(version, lines=None, path=None):
    lines = current_lines() if lines is None else lines
    path = path or snapshot_path(version)
    path.parent.mkdir(parents=True, exist_ok=True)
    head = [f"# JFLib public API {version} (python build_rs.py api-diff --snapshot; docs/VERSIONING.md)",
            "# PATH\tKIND\tSIGNATURE\tFLAGS"]
    path.write_bytes(("\n".join(head + ["\t".join(ln) for ln in lines]) + "\n").encode("utf-8"))
    return path


def read_snapshot(path):
    out = []
    for ln in Path(path).read_text(encoding="utf-8").splitlines():
        if not ln or ln.startswith("#"):
            continue
        p = ln.split("\t")
        out.append(tuple((p + ["", "", "", ""])[:4]))
    return out


# ---------------------------------------------------------------- the diff
def key(line):
    path, kind, sig, flags = line
    cfg = ",".join(f for f in flags.split(",") if f in ("windows", "linux"))
    return (path, kind, cfg, canon(sig) if kind == "impl" else "")


def has(line, flag):
    return flag in line[3].split(",")


def diff(old, new):
    """two snapshots -> {"breaking": [(what, line, detail)], "deprecated": [...], "added": [...],
    "notes": [...]}"""
    o = {key(ln): ln for ln in old}
    n = {key(ln): ln for ln in new}
    res = {"breaking": [], "deprecated": [], "added": [], "notes": []}
    owners = {ln[0]: ln for ln in old if ln[1] in ("struct", "union", "enum", "trait")}
    for k, ln in o.items():
        if k not in n:
            res["breaking"].append(("removed", ln, "deprecated before" if has(ln, "deprecated") else ""))
    for k, ln in n.items():
        if k in o:
            was = o[k]
            if canon(was[2], was[1]) != canon(ln[2], ln[1]):
                res["breaking"].append(("changed", ln, was[2]))
            elif has(was, "provided") and not has(ln, "provided"):
                res["breaking"].append(("required", ln, "the default body was removed"))
            elif not has(was, "non-exhaustive") and has(ln, "non-exhaustive"):
                res["breaking"].append(("non-exhaustive", ln, "#[non_exhaustive] added"))
            if has(ln, "deprecated") and not has(was, "deprecated"):
                res["deprecated"].append(("deprecated", ln, ""))
            continue
        parent = ln[0].rsplit("::", 1)[0] if ln[1] in ("variant", "trait-fn", "trait-const", "trait-type") \
            else ln[0].rsplit(".", 1)[0] if ln[1] == "field" else None
        pline = owners.get(parent) if parent else None
        if ln[1] == "variant" and pline and pline[1] == "enum" and not has(pline, "non-exhaustive"):
            res["breaking"].append(("variant", ln, "a new variant of an exhaustive enum (matches without _ break)"))
        elif ln[1].startswith("trait-") and pline and pline[1] == "trait" and not has(ln, "provided"):
            res["breaking"].append(("trait", ln, "a new required member (implementations break)"))
        elif ln[1] == "field" and pline and has(pline, "all-pub"):
            res["added"].append(("added", ln, ""))
            res["notes"].append(("field", ln, "literal constructions without ..NONE / ..Default::default() break"))
        else:
            res["added"].append(("added", ln, ""))
    for v in res.values():
        v.sort(key=lambda r: (r[1][0], r[1][1]))
    return res


def report(d, base, version, limit=None):
    """the diff as text lines"""
    out = []
    level = level_of(d)
    need = next_version(base, level) if level != "none" else base
    for title, rows in (("Breaking", d["breaking"]), ("Deprecated", d["deprecated"]), ("Added", d["added"]),
                        ("Notes", d["notes"])):
        if not rows:
            continue
        out.append(f"{title} ({len(rows)}):")
        for what, (path, kind, sig, flags), detail in rows[:limit]:
            line = f"  {what:<14} {path}  [{kind}] {sig}"
            if what == "changed":
                line = f"  {what:<14} {path}  [{kind}]\n      was: {detail}\n      now: {sig}"
            elif detail:
                line += f"  -- {detail}"
            out.append(line)
        if limit is not None and len(rows) > limit:
            out.append(f"  ... {len(rows) - limit} more")
    ok = version_ok(version, base, level)
    if level == "breaking":
        verdict = (f"breaking changes since {base}: the next release must be {need} or later" +
                   ("" if ok else f" (lib/VERSION is {version})"))
    elif level == "added":
        verdict = f"compatible additions since {base}: the next release needs at least {need}"
    else:
        verdict = (f"no API change since {base}" + (f" ({len(d['notes'])} notes)" if d["notes"] else "")
                   + f": the next release can be a patch release ({next_version(base, 'none')})")
    out.append(verdict)
    return out, ok or level != "breaking"


def summary(d, base):
    return (f"API since {base}: {len(d['breaking'])} breaking, {len(d['deprecated'])} deprecated, "
            f"{len(d['added'])} added (python build_rs.py api-diff)")


def baseline(against=None):
    """(version, path) of the snapshot to compare with: `against`, else the newest"""
    snaps = snapshots()
    if against:
        p = snapshot_path(against)
        if not p.exists():
            raise SystemExit(f"no snapshot docs/api/{against}.txt (releases: {', '.join(v for v, _ in snaps) or 'none'})")
        return against, p
    if not snaps:
        return None, None
    return snaps[-1]


def main(argv=None):
    import argparse
    ap = argparse.ArgumentParser(description="the public API since a release (docs/VERSIONING.md)")
    ap.add_argument("--against", help="the release to compare with (default: the newest docs/api snapshot)")
    ap.add_argument("--snapshot", action="store_true", help="write docs/api/<lib/VERSION>.txt")
    ap.add_argument("--all", action="store_true", help="list every change (default: 200 per group)")
    a = ap.parse_args(argv)
    version = lib_version()
    if a.snapshot:
        p = write_snapshot(version)
        print(f"wrote {p.relative_to(ROOT).as_posix()} ({sum(1 for _ in read_snapshot(p))} items)")
        return 0
    base, path = baseline(a.against)
    if base is None:
        print("no release snapshot in docs/api yet: python build_rs.py api-diff --snapshot")
        return 0
    d = diff(read_snapshot(path), current_lines())
    lines, ok = report(d, base, version, None if a.all else 200)
    print(f"public API: {base} (docs/api/{base}.txt) -> the sources (lib/VERSION {version})")
    print("\n".join(lines))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
