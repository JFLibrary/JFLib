#!/usr/bin/env python3
"""apidiff.py -- the library's public API as a snapshot, and what changed since a release.

    python build_rs.py api-diff                  compare with the newest docs/api/X.Y.Z.txt
    python build_rs.py api-diff --against 0.1.0  ... with that release
    python build_rs.py api-diff --snapshot       write docs/api/<the library's version>.txt (a release)
    python tools/gendoc.py --check --api         the suite's one-line summary (one parse)

The public API (docs/VERSIONING.md) is what docs/API.md documents and the facade jflib shows: the
crates of gendoc's CRATES table but REPO_ONLY (codec crates: their root, their re-exports and
CODEC_EXTRA), every item another crate can name, minus #[doc(hidden)] and #[cfg(test)].  A
snapshot's lines of a REPO_ONLY crate (0.1.0 still has plugin's) are left out of the comparison.  gendoc's parser reads the sources; this
file walks the modules again to find each item's public paths and keeps the shortest one, so
moving an item between modules that are glob re-exported (ui's files) changes nothing.

A `pub use` into another crate of CRATES is followed into that crate: the module, item or macro
it names (`pub use formats::json;`, `pub use compress::{adler32, crc32};`, `pub use
formats::record;`), also through a name the module imports (`pub use json::Value;` after `pub use
formats::json;`), gets that crate's lines -- the item's signature, its fields, variants, methods
and impls, this crate's trait impls for it -- under this crate's path, each item once per crate at
its shortest path, shown as its own crate shows it (a codec crate's kind decides).  So an item that
moved to another crate and stays re-exported under its old path is no change.  A re-export
through which nothing shown is found (a module a codec crate does not show, items a macro writes,
rt::os of SKIP_FILES) stays a `use` line, and so does one of a crate outside CRATES.  Snapshots up
to 0.2.1 hold every re-export of another crate as a `use` line: read_snapshot reads such a line
the same way, as the target's lines of that snapshot (found under the path the use names or,
where it names an alias such as plat::hotkey::Chord, where the target crate shows the item now).

Snapshot lines (sorted; # starts a comment):  PATH <TAB> KIND <TAB> SIGNATURE <TAB> FLAGS
  KIND   fn, method, const, static, type, macro, struct, union, enum, variant, field, trait,
         trait-fn / trait-const / trait-type (a member of a public trait), impl (a derive or an
         `impl Trait for Type` of a public type; SIGNATURE names the trait), use (a re-export of
         a crate outside CRATES, or one of a crate of CRATES that shows nothing)
  FLAGS  comma list: windows / linux / apple (the item's cfg), deprecated, provided (a trait member with a
         default), all-pub (a struct whose fields are all public), non-exhaustive
A line's identity is PATH, KIND, its cfg and, for impl, the trait; signatures compare without
whitespace and trailing commas (rustfmt reflowing a signature is no change).  A module declared
once per target under one name keeps every variant (gendoc); variants under a cfg without a flag
(target_arch, wasm32, Android) give one identity several signatures (rt::kernel::Stat per
target_arch): they compare as a set -- an old signature that is gone is a change, one more is an
addition.

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
(0.x: next PATCH), nothing -> the same or a PATCH.  api-diff exits 1 when the library's version is lower
than the needed version for breaking changes; `build_rs.py release` refuses a too small step.
"""
import re
import sys
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))
import gendoc  # noqa: E402
import manifests  # noqa: E402

ROOT = TOOLS.parent
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
    return manifests.version()


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
    change for a caller); rt::plugin_abi!(fn ...) is the type it stands for on x86-64,
    extern "win64" fn ... (lib/rt/abi.rs: the same type as before ARM4 there, docs/ARM64.md)"""
    s = plugin_abi(re.sub(r"\s+", " ", sig).strip())
    if kind in ("fn", "method", "trait-fn"):
        s = param_types(s)
    s = re.sub(r"\s+(?=\W)|(?<=\W)\s+", "", s)
    return re.sub(r",(?=[)\]}>])", "", s)


def plugin_abi(s):
    """'rt::plugin_abi!(fn(u32) -> u32)' -> 'extern "win64" fn(u32) -> u32' (parentheses matched)"""
    while (i := s.find("plugin_abi!(")) >= 0:
        start = i - len("rt::") if s[:i].endswith("rt::") else i
        j, depth = i + len("plugin_abi!("), 1
        while j < len(s) and depth:
            depth += {"(": 1, ")": -1}.get(s[j], 0)
            j += 1
        s = s[:start] + 'extern "win64" ' + s[i + len("plugin_abi!("):j - 1] + s[j:]
    return s


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
    return {"Windows": "windows", "Linux": "linux", "Apple": "apple"}.get(cfg, "")


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


def api_crates():
    """{name: kind}: the crates of the public API (gendoc's CRATES but REPO_ONLY)"""
    return {name: kind for name, _title, kind in gendoc.CRATES if name not in gendoc.REPO_ONLY}


def exported_macros(crate):
    """[(macro, module)]: a crate's #[macro_export] macros (another crate names them at its root)"""
    return [(it, m) for m in crate.all_modules() for it in m.items if gendoc.is_exported_macro(it)]


def strip_self(segs):
    """`use a::b::{self}` names a::b"""
    return segs[:-1] if len(segs) > 1 and segs[-1] == "self" else segs


class PathWalk:
    """the public paths of a crate's items, the items of the other crates of CRATES it re-exports
    included: id(item) -> (item, module, {paths}, crate); re-exports of crates outside CRATES:
    external [(path, text, module)]; steps into another crate of CRATES: foreign [(path, text,
    module, {id(item) found through it})] (its `use` line when none of them is shown)"""

    def __init__(self, crate):
        self.c = crate
        self.crates = api_crates()
        self.items = {}
        self.external = []
        self.foreign = []
        self.seen = {}                  # (id(module), path) -> {id(item)} exported there
        self.trace = []                 # the id sets that collect what add() adds (seen, foreign)
        self.export(crate, crate.root, crate.name)

    @staticmethod
    def skipped(c, mod):
        """a module of SKIP_FILES (rt::os: the OS calls plat is built on) or inside one"""
        skip = gendoc.SKIP_FILES.get(c.name, set())
        while mod is not None:
            if mod.path.name in skip:
                return True
            mod = mod.parent
        return False

    def add(self, it, mod, path, c):
        self.items.setdefault(id(it), (it, mod, set(), c))[2].add(path)
        for ids in self.trace:
            ids.add(id(it))

    def export(self, c, mod, path):
        """the public names of mod (a module of crate c) are visible as path::NAME"""
        key = (id(mod), path)
        if key in self.seen:
            for ids in self.trace:
                ids |= self.seen[key]
            return
        if self.skipped(c, mod):
            return
        self.seen[key] = got = set()
        self.trace.append(got)
        try:
            for it in mod.items:
                if gendoc.is_exported_macro(it):
                    if c is self.c:                         # another crate's: at that crate's root only
                        self.add(it, mod, f"{c.name}::{it.name}", c)
                elif it.vis == "pub" and it.kind != "impl" and it.name:
                    self.add(it, mod, f"{path}::{it.name}", c)
            if c is not self.c and mod is c.root:           # pub use dep::*: dep's macros come along
                for it, m in exported_macros(c):
                    self.add(it, m, f"{path}::{it.name}", c)
            for name, variants in mod.children.items():
                for ch in variants:
                    if ch.vis == "pub":
                        self.export(c, ch, f"{path}::{name}")
            for vis, trees, text in mod.uses:
                if vis == "pub":
                    for segs, alias, glob in trees:
                        self.reexport(c, mod, strip_self(segs), alias, glob, path)
        finally:
            self.trace.pop()

    def reexport(self, c, mod, segs, alias, glob, path):
        """the `pub use segs` of mod (crate c), its names visible as path::NAME"""
        name = "*" if glob else (alias or (segs[-1] if segs else ""))
        if not name or name in ("self", "_"):
            return
        dest = f"{path}::{name}"
        for tc, target, rest, via in self.targets(c, mod, segs):     # one per cfg variant on the way
            if tc is None:
                self.external.append((dest, "::".join(segs) + ("::*" if glob else ""), mod))
            elif glob and not rest:
                self.cross(c, tc, (dest, via, mod, glob), self.export, tc, target, path)
            elif not glob and len(rest) == 1:
                self.cross(c, tc, (dest, via, mod, glob), self.mark, tc, target, rest[0], dest)
            elif not glob and not rest:
                self.cross(c, tc, (dest, via, mod, glob), self.export, tc, target, dest)

    def cross(self, c, tc, record, fn, *args):
        """fn(*args); a step from crate c into another one (tc) goes into foreign with what it found
        (record: (path, the use path where the other crate was entered, module, glob), or None)"""
        if tc is c:
            fn(*args)
            return
        ids = set()
        self.trace.append(ids)
        try:
            fn(*args)
        finally:
            self.trace.pop()
        if record:
            path, via, mod, glob = record
            self.foreign.append((path, "::".join(via) + ("::*" if glob else ""), mod, ids))

    def targets(self, c, mod, segs, depth=0, via=None):
        """[(crate, module, rest, via)] for a use path of mod in crate c, one per cfg variant of
        the modules on the way.  A name the module imports is followed (`use formats::json;`
        then `pub use json::Value;`), and so is the name of another crate of CRATES (via: the
        use path where it was entered, None while in c); (None, None, segs, None): a crate
        outside CRATES"""
        if depth > 8:
            return [(None, None, segs, None)]
        res = c.resolve(mod, segs)
        if res[0][0] is None:                               # segs[0] names no module of mod
            imp = self.imported(mod, segs[0])
            if imp:
                return self.targets(c, mod, imp + segs[1:], depth + 1, via)
            if segs[0] in self.crates and segs[0] != c.name:
                dep = gendoc.load_crate(segs[0])
                return self.targets(dep, dep.root, ["crate"] + segs[1:], depth + 1, via or segs)
            return [(None, None, segs, None)]
        out = []
        for target, rest in res:
            imp = self.imported(target, rest[0]) if len(rest) > 1 else None
            if imp:                                         # a::b::C with `pub use x::b;` in a
                out += self.targets(c, target, imp + rest[1:], depth + 1, via)
            else:
                out.append((c, target, rest, via))
        return out

    @staticmethod
    def imported(mod, name):
        """the path a `use` of mod (not a glob) brings in as `name`, or None"""
        for _vis, trees, _text in mod.uses:
            for segs, alias, glob in trees:
                segs = strip_self(segs)
                if not glob and segs and (alias or segs[-1]) == name and segs != [name]:
                    return list(segs)
        return None

    def mark(self, c, mod, name, path, depth=0):
        """the item `name` of mod (a module of crate c), re-exported as path"""
        if c is not self.c and self.skipped(c, mod):       # plat's `pub use rt::os::exit`: a use line
            return
        found = False
        for it in mod.items:
            if it.name == name and it.kind != "impl":
                self.add(it, mod, path, c)
                found = True
        if mod is c.root:                                   # pub use dep::some_macro
            for it, m in exported_macros(c):
                if it.name == name:
                    self.add(it, m, path, c)
                    found = True
        for ch in mod.children.get(name, ()):
            self.export(c, ch, path)
            found = True
        if found or depth > 8:
            return
        for vis, trees, text in mod.uses:
            for segs, alias, glob in trees:
                segs = strip_self(segs)
                if not glob and (alias or (segs[-1] if segs else "")) != name:
                    continue
                for tc, target, rest, via in self.targets(c, mod, segs):
                    if tc is None:
                        if not glob:
                            self.external.append((path, "::".join(segs), mod))
                    elif glob and not rest:
                        self.cross(c, tc, None, self.mark, tc, target, name, path, depth + 1)
                    elif not glob and len(rest) == 1:
                        self.cross(c, tc, (path, via, mod, False), self.mark, tc, target, rest[0], path, depth + 1)
                    elif not glob and not rest:
                        self.cross(c, tc, (path, via, mod, False), self.export, tc, target, path)


def best(paths):
    return min(paths, key=lambda p: (p.count("::"), p))


def shows(c, it, mod, kind):
    """whether the public item `it` of crate c (of `kind` in CRATES) is in the snapshot"""
    if hidden(it):
        return False
    if kind != "codec":
        return True
    extra = gendoc.CODEC_EXTRA.get(c.name, {})
    return mod is c.root or id(it) in c.rexp or extra.get(mod.name) == "*" or it.name in extra.get(mod.name, [])


def mod_path(mod):
    """('crate', 'a', 'b') for crate::a::b (the cfg variants of a module share it)"""
    out = []
    while mod is not None:
        out.append(mod.name)
        mod = mod.parent
    return tuple(reversed(out))


def crate_lines(crate, kind="full"):
    """the snapshot lines of one crate: [(path, kind, sig, flags)].  An item of another crate of
    CRATES that this one re-exports is shown as there (its crate's kind decides), under this
    crate's path; the `use` line stays where none of the items found through it is shown"""
    walk = PathWalk(crate)
    kinds = api_crates()
    shown = {}                                      # id(item) -> (item, module, path, crate)
    for it, mod, paths, c in walk.items.values():
        if shows(c, it, mod, kind if c is crate else kinds.get(c.name, "full")):
            shown[id(it)] = (it, mod, best(paths), c)
    types = {}                                      # (crate, name) -> [(module, path)]
    for it, mod, path, c in shown.values():
        if it.kind in TYPE_KINDS:
            types.setdefault((c.name, it.name), []).append((mod, path))
    out = []
    for it, mod, path, _c in shown.values():
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
    # methods and trait impls of the public types: impl blocks in any module of the type's crate
    # (and this crate's traits for another crate's types)
    crates = {crate.name: crate}
    for _it, _mod, _path, c in shown.values():
        crates.setdefault(c.name, c)
    own = {it.name for m in crate.all_modules() for it in m.items if it.kind in TYPE_KINDS}
    for c in crates.values():
        homes = {}                                  # type name -> {module path}: same name, other type?
        if c is not crate:
            for m in c.all_modules():
                for it in m.items:
                    if it.kind in TYPE_KINDS:
                        homes.setdefault(it.name, set()).add(mod_path(m))
        for mod in c.all_modules():
            if walk.skipped(c, mod):
                continue
            for it in mod.items:
                if it.kind != "impl":
                    continue
                ty, is_trait = gendoc.impl_type(it.sig)
                cands = types.get((c.name, ty))
                if not cands and c is crate and is_trait and ty not in own:
                    cands = [mp for (cn, tn), v in types.items() if tn == ty and cn != c.name for mp in v]
                if not cands:
                    continue
                here = next((p for m, p in cands if m is mod), None)
                if here is None and c is not crate and len(homes.get(ty, ())) != 1:
                    continue                        # another crate's type of a name it uses twice
                tpath = here or cands[0][1]
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
    found = {}                                      # (path, text, cfg) -> whether an item is shown
    for path, text, mod, ids in walk.foreign:
        k = (path, text, cfg_flag(mod.cfg))
        found[k] = found.get(k, False) or any(i in shown for i in ids)
    for (path, text, cfg), f in found.items():
        if not f:                                   # a module a codec crate does not show, items a macro writes
            out.append((path, "use", text, [cfg] if cfg else []))
    return out


def current_lines():
    """the snapshot lines of the library as it is now"""
    out = set()
    for name, _title, kind in gendoc.CRATES:
        if name in gendoc.REPO_ONLY:
            continue
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
        if p[0].split("::")[0] in gendoc.REPO_ONLY:
            continue
        out.append(tuple((p + ["", "", "", ""])[:4]))
    return expand_uses(out)


def heads(path):
    """'a::B.f' -> ['a', 'a::B', 'a::B.f']: the paths a line is found under"""
    base, dot, _ = path.partition(".")
    parts = base.split("::")
    return ["::".join(parts[:k]) for k in range(1, len(parts) + 1)] + ([path] if dot else [])


def owner(line):
    """the path of the item a snapshot line belongs to (a field's, variant's, method's type)"""
    path, kind = line[0], line[1]
    if kind == "field":
        return path.rsplit(".", 1)[0]
    if kind in ("variant", "method", "trait-fn", "trait-const", "trait-type"):
        return path.rsplit("::", 1)[0]
    return path


_WALKS = {}


def aliases(src, dst):
    """[(path, path under dst)]: where the target crate shows the items of the use path src now,
    when src is not that path (ui's `pub use plat::hotkey::Chord`, shown as plat::Chord)"""
    name = src.split("::")[0]
    crate = gendoc.load_crate(name)
    if _WALKS.get(name, (None,))[0] is not crate:
        _WALKS[name] = (crate, PathWalk(crate))
    out = []
    for _it, _mod, paths, _c in _WALKS[name][1].items.values():
        b = best(paths)
        for p in paths:
            if p != b and (p == src or p.startswith(src + "::")):
                out.append((b, dst + p[len(src):]))
    return out


def expand_uses(lines):
    """a snapshot's `use` lines into another crate of CRATES, read the way crate_lines writes
    them now (snapshots up to 0.2.1 hold one per such re-export): the target's lines of the same
    snapshot under the re-exporting path, found under the path the use names or, where that is no
    path the target crate shows, where it shows the items now; each item once per crate, at its
    shortest path; the use line stays where no item is found (as crate_lines keeps it)"""
    crates = api_crates()
    have = set(lines)
    for _ in range(8):                              # a use of a use: until nothing changes
        under = {}
        for ln in have:
            for h in heads(ln[0]):
                under.setdefault(h, []).append(ln)
        copies = {}                                 # (crate, item in the target) -> {root: [lines]}
        drop = set()
        for ln in sorted(have):
            path, kind, text, _flags = ln
            if kind != "use" or text.split("::")[0] not in crates:
                continue
            glob = text.endswith("::*")
            if glob != path.endswith("::*"):
                continue
            src, dst = (text[:-3], path[:-3]) if glob else (text, path)
            pairs = [(src, dst, glob)]                  # pub use m::*: what is under m, not m
            if not any(x[1] != "use" for x in under.get(src, ())):
                pairs += [(s, d, False) for s, d in aliases(src, dst)]
            for s, d, inside in pairs:
                for x in under.get(s, ()):
                    if x == ln or (inside and x[0] == s):
                        continue
                    o = owner(x)
                    o = o if o == s or o.startswith(s + "::") else s
                    root = d + o[len(s):]
                    copies.setdefault((path.split("::")[0], o), {}).setdefault(root, []).append(
                        (d + x[0][len(s):],) + x[1:])
                    if x[1] != "use":
                        drop.add(ln)
        new = have - drop
        for roots in copies.values():
            new.update(roots[best(roots)])
        if new == have:
            break
        have = new
    return sorted(have)


# ---------------------------------------------------------------- the diff
def cfg_of(flags):
    """'linux,provided' -> 'linux': the cfg part of a line's flags"""
    return ",".join(f for f in flags.split(",") if f in ("windows", "linux", "apple"))


def key(line):
    path, kind, sig, flags = line
    return (path, kind, cfg_of(flags), canon(sig) if kind == "impl" else "")


def has(line, flag):
    return flag in line[3].split(",")


def by_key(lines):
    """{identity: {canonical signature: line}}: an identity has several signatures when a module
    exists once per target under a cfg without a flag (rt::kernel's `mod arch` per target_arch)"""
    out = {}
    for ln in sorted(lines):
        out.setdefault(key(ln), {})[canon(ln[2], ln[1])] = ln
    return out


def diff(old, new):
    """two snapshots -> {"breaking": [(what, line, detail)], "deprecated": [...], "added": [...],
    "notes": [...]}"""
    o, n = by_key(old), by_key(new)
    res = {"breaking": [], "deprecated": [], "added": [], "notes": []}
    owners = {ln[0]: ln for ln in old if ln[1] in ("struct", "union", "enum", "trait")}
    for k, sigs in o.items():
        if k not in n:
            for ln in sigs.values():
                res["breaking"].append(("removed", ln, "deprecated before" if has(ln, "deprecated") else ""))
    for k, sigs in n.items():
        if k in o:
            olds = o[k]
            gone = [ln for c, ln in olds.items() if c not in sigs]       # signatures of the item that changed
            fresh = [ln for c, ln in sigs.items() if c not in olds]
            for i, was in enumerate(gone):
                now = fresh[min(i, len(fresh) - 1)] if fresh else next(iter(sigs.values()))
                res["breaking"].append(("changed", now, was[2]))
            for ln in fresh[len(gone):]:                                # one more target's signature
                res["added"].append(("added", ln, ""))
            for c in olds.keys() & sigs.keys():
                was, ln = olds[c], sigs[c]
                if has(was, "provided") and not has(ln, "provided"):
                    res["breaking"].append(("required", ln, "the default body was removed"))
                elif not has(was, "non-exhaustive") and has(ln, "non-exhaustive"):
                    res["breaking"].append(("non-exhaustive", ln, "#[non_exhaustive] added"))
                if has(ln, "deprecated") and not has(was, "deprecated"):
                    res["deprecated"].append(("deprecated", ln, ""))
            continue
        for ln in sigs.values():
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
            tag = kind + (f", {cfg_of(flags)}" if cfg_of(flags) else "")       # [method, windows]
            line = f"  {what:<14} {path}  [{tag}] {sig}"
            if what == "changed":
                line = f"  {what:<14} {path}  [{tag}]\n      was: {detail}\n      now: {sig}"
            elif detail:
                line += f"  -- {detail}"
            out.append(line)
        if limit is not None and len(rows) > limit:
            out.append(f"  ... {len(rows) - limit} more")
    ok = version_ok(version, base, level)
    if level == "breaking":
        verdict = (f"breaking changes since {base}: the next release must be {need} or later" +
                   ("" if ok else f" (the library's version is {version})"))
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
    ap.add_argument("--snapshot", action="store_true", help="write docs/api/<the library's version>.txt")
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
    print(f"public API: {base} (docs/api/{base}.txt) -> the sources (version {version})")
    print("\n".join(lines))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
