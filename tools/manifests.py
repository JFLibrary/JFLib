"""manifests.py -- the Cargo manifests as build_rs.py and the tools read them

The manifests are the one place for the library's version, its crates and their dependencies
(docs/CRATES.md).  build_rs.py still compiles with rustc itself; what it needs to know comes from
here: LIBS and TARGET_DEPS (library_crates), the version it hands to rustc as CARGO_PKG_VERSION
(version, read by rt::VERSION).  `build_rs.py release` writes the version (set_version).
"""
import re
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FACADE = "lib/jflib"            # names every crate, no crate of its own for an app to name
NOT_LIBS = (FACADE, "lib/jflib_build", "lib/cargo_jflib", "lib/installer", "lib/appimagert", "lib/plugin")   # the build helper (std); the plugins' ABI
# the cfg of a [target] table -> the build it belongs to: None for every build of build_rs.py,
# which never compiles inside a module with these names (ModuleBuilder passes modrt as rt itself)
TARGET_CFG = {
    "cfg(windows)": "win",
    'cfg(all(target_os = "linux", jfgui_glibc))': "linux-glibc",
    'cfg(target_os = "linux")': "linux",
    'cfg(target_os = "android")': "android",
    'cfg(target_arch = "wasm32")': "web",
    'cfg(target_os = "macos")': "mac",
    'cfg(target_os = "ios")': "ios",
    "cfg(not(jflib_module))": None,
}
MODULE_CFG = "cfg(jflib_module)"


class ManifestError(Exception):
    pass


def read(path):
    return tomllib.loads(Path(path).read_text(encoding="utf-8"))


def workspace(root=ROOT):
    """the root manifest's [workspace]"""
    return read(Path(root) / "Cargo.toml")["workspace"]


def version(root=ROOT):
    """the library's version"""
    return workspace(root)["package"]["version"]


def lib_members(root=ROOT):
    """the folders of lib/ that are library crates, in the order of the root manifest"""
    return [m for m in workspace(root)["members"] if m.startswith("lib/") and m not in NOT_LIBS]


def library_crates(root=ROOT):
    """the crates of lib/ -> (LIBS, TARGET_DEPS): {name: (root file, dependencies)} in the order
    of the root manifest's members (dependencies in that order too) and {(name, target):
    dependencies} of what a crate needs on one target only"""
    manifests = {}
    for member in lib_members(root):
        m = read(Path(root) / member / "Cargo.toml")
        manifests[m["lib"]["name"]] = (member, m)
    order = {name: i for i, name in enumerate(manifests)}

    def in_order(names):
        return sorted((n for n in names if n in order), key=order.get)
    libs, target_deps = {}, {}
    for name, (member, m) in manifests.items():
        deps = set(m.get("dependencies", {}))
        for cfg, table in m.get("target", {}).items():
            if cfg == MODULE_CFG:
                continue
            if cfg not in TARGET_CFG:
                raise ManifestError(f"{member}/Cargo.toml: build_rs.py does not know [target.'{cfg}'] (tools/manifests.py)")
            if TARGET_CFG[cfg] is None:
                deps |= set(table["dependencies"])
            else:
                target_deps[(name, TARGET_CFG[cfg])] = in_order(table["dependencies"])
        libs[name] = (f"{member}/{m['lib']['path']}", in_order(deps))
    return libs, target_deps


def external_lock(root=ROOT):
    """what Cargo.lock says about the crates from outside: the part of it that changes what
    text_shape builds from.  The workspace's own entries change with every new crate and release."""
    blocks = Path(root, "Cargo.lock").read_text(encoding="utf-8").split("[[package]]")
    return "".join(b for b in blocks[1:] if "source = " in b)


def set_version(new, root=ROOT):
    """write the version: [workspace.package], the exact versions of [workspace.dependencies] and
    modplat's own, the workspace's entries of Cargo.lock, and MAJOR.MINOR where an app's
    dependency names it (DEP_VERSION_FILES: the template, the guide)"""
    root = Path(root)

    def edit(rel, change):
        path = root / rel
        raw = path.read_bytes().decode("utf-8")
        path.write_bytes(change(raw).encode("utf-8"))

    def pins(text):
        return re.sub(r'version = "=[^"]*"', f'version = "={new}"', text)
    edit("Cargo.toml", lambda t: pins(re.sub(r'^version = "[^"]*"', f'version = "{new}"', t, count=1, flags=re.M)))
    edit("modules/modplat/Cargo.toml", pins)
    # every published package: jflib, jflib-*, and cargo-jflib (forgotten before 1.0.0: `--locked` then fails)
    edit("Cargo.lock", lambda t: re.sub(r'(name = "(?:cargo-)?jflib(?:-[a-z0-9-]+)?"\r?\nversion = )"[^"]*"', rf'\g<1>"{new}"', t))
    # what `cargo jflib new` writes and the guide shows: jflib = "MAJOR.MINOR" (found before 0.2.0: the
    # template still asked for 0.1 after the release)
    for rel in DEP_VERSION_FILES:
        if (root / rel).is_file():
            edit(rel, lambda t: set_dep_versions(t, new))


# the files that name the library's version as an app's dependency does: `jflib = "X.Y"`,
# `jflib-build = "X.Y"`, `jflib = { features = [..], version = "X.Y" }` (dep_version_problems)
DEP_VERSION_FILES = ("lib/jflib/template/Cargo.toml.in", "lib/cargo_jflib/template/Cargo.toml.in",
                     "docs/GUIDE.md", "lib/jflib/docs/GUIDE.md")
DEP_VERSION = re.compile(r'(\bjflib(?:-build)?\s*=\s*(?:\{[^}\n]*?version\s*=\s*)?")(\d+\.\d+)(")')


def set_dep_versions(text, version):
    """text with every dependency on jflib / jflib-build asking for MAJOR.MINOR of `version`"""
    mm = ".".join(version.split(".")[:2])
    return DEP_VERSION.sub(lambda m: m.group(1) + mm + m.group(3), text)


def dep_version_problems(root=ROOT):
    """the files of DEP_VERSION_FILES that ask for another MAJOR.MINOR than the library's version"""
    root = Path(root)
    want = ".".join(version(root).split(".")[:2])
    bad = []
    for rel in DEP_VERSION_FILES:
        p = root / rel
        if p.is_file():
            for v in sorted({m.group(2) for m in DEP_VERSION.finditer(p.read_text(encoding="utf-8"))} - {want}):
                bad.append(f"{rel}: asks for jflib {v}, the library is {version(root)} (build_rs.py release writes it)")
    return bad
