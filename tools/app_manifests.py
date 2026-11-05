"""app_manifests.py -- the Cargo manifests of an app, its further programs and its own crates

An app is a Cargo package in apps/NAME (docs/CRATES.md, CR3): its Cargo.toml names the program
(after the folder), the crates its sources use and its style; its build.rs hands PERMISSIONS,
PACKAGE, assets/ and res/app.ico to the program (lib/jflib_build).  A further program of BINS is a
package of its own in its folder that is part of the app (Build::part_of), an app-local crate in
crates/C one too.  `build_rs.py new` writes them through this module, `build_rs.py check` compares
them with the sources (manifest_lint).
"""
import os
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MENTION = re.compile(r"\b(\w+)::|extern crate (\w+)\b")


def named(text, names):
    """the names of `names` that a source text uses (NAME:: or extern crate NAME), in their order"""
    found = {a or b for a, b in MENTION.findall(text)}
    return [n for n in names if n in found]


def rel(frm, to):
    return Path(os.path.relpath(Path(to), Path(frm))).as_posix()


def _head(name, version):
    return (f'[package]\nname = "{name}"\nversion = "{version}"\nedition.workspace = true\nrust-version.workspace = true\n'
            "publish = false\n")


def _deps(pkg_dir, deps, local):
    """deps: crate names; local: {app-local crate: its folder}, everything else is in the root's
    [workspace.dependencies]"""
    return "".join((f'{d} = {{ path = "{rel(pkg_dir, local[d])}" }}\n' if d in local else f"{d}.workspace = true\n") for d in deps)


def program(pkg_dir, name, version, bin_name, entry, deps, local, style=None, root=ROOT):
    """the Cargo.toml of a program: the app's own (bin_name = the app's name) or one of BINS;
    style: the crate the sources name as `style`"""
    t = _head(name, version) + "autobins = false\nautoexamples = false\nautotests = false\nautobenches = false\n"
    t += f'\n[[bin]]\nname = "{bin_name}"\npath = "{entry}"\ntest = false\nbench = false\n'
    t += "\n[lints]\nworkspace = true\n\n[dependencies]\n" + _deps(pkg_dir, deps, local)
    if style:
        t += f'style = {{ package = "jflib-style-{style}", path = "{rel(pkg_dir, Path(root) / "styles" / style)}" }}\n'
    return t + "\n[build-dependencies]\njflib-build.workspace = true\n"


def build_script(kind="gui", app=None, app_dir=None):
    """the build.rs of a program: the app's own, or one of BINS (app: the app's name, app_dir: the
    app's folder relative to the program's)"""
    head = "// PERMISSIONS, PACKAGE, assets/ and res/app.ico of the app, and the linker arguments (lib/jflib_build)\n"
    if app is None:
        call = "jflib_build::app();" if kind == "gui" else "jflib_build::console();"
    else:
        call = f'jflib_build::Build::{"app" if kind == "gui" else "console"}().part_of("{app}", "{app_dir}").run();'
    return head + f"fn main() {{\n    {call}\n}}\n"


def local_crate(pkg_dir, name, version, entry, deps, local):
    """the Cargo.toml of an app-local crate (apps/NAME/crates/C)"""
    return (_head(name, version) + f'\n[lib]\nname = "{name}"\npath = "{entry}"\ntest = false\ndoctest = false\n'
            + "\n[lints]\nworkspace = true\n" + ("\n[dependencies]\n" + _deps(pkg_dir, deps, local) if deps else ""))
