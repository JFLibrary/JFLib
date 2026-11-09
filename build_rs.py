#!/usr/bin/env python3
"""build_rs.py -- build and test JFLib: the tool on top of Cargo (docs/CRATES.md).
Cargo compiles; this tool checks the sources first, runs one `cargo build` per target and run, puts
the programs into build/<target> with their notices, modules and the browser's host files, makes the
APKs, runs the tests and keeps the rest of the repository's work (packages, site, i18n, releases).
The crates, their dependencies and the library's version come from the Cargo manifests
(tools/manifests.py): one Cargo.toml per crate, app and further program, the workspace in the root.

  python build_rs.py lib    [--target win|linux|both]        every library crate
  python build_rs.py app NAME [--style S]                    apps/NAME -> build/<t>/NAME[.exe]
                                                                (+ its loadable modules, e.g. codectest's jxlenc: section "jxl-module")
  python build_rs.py app NAME --style S1,S2,..               one variant per style: build/<t>/<style>/NAME[.exe]
                                                                (each variant a package of the tool's own)
  python build_rs.py all                                     libraries + every app
  python build_rs.py run NAME [--target win|linux] [-- ARGS] build and start (Linux: WSL)
  python build_rs.py run NAME --bin BIN [-- ARGS]            start one of the app's further executables (BINS)
  python build_rs.py watch NAME [--target win|linux] [-- ARGS]   hot reload (tools/hotreload.py): build again when a
                                                                source of the app or its crates is saved, start the
                                                                app again in its session (input replayed, window,
                                                                style, inspector kept); development builds only
  python build_rs.py test [--quick]                          lint, build, every apps/*/tests/*.uis (+ apps/*/tests),
                                                                then the crates' acceptance test (tools/crates_accept.py)
                                                                and parity with asm (both not with --quick)
  python build_rs.py test NAME                               build one app and run its tests/*.uis
  python build_rs.py check                                   lint only
  python build_rs.py new NAME [--kind window|console|tool|service] [--style S] [--title T]
                                                             a new app apps/NAME that builds and passes its
                                                                test as it is (tools/scaffold.py)
  python build_rs.py new --lib NAME [--deps rt,plat]         a new library crate lib/NAME, entered in the manifests
  python build_rs.py api-diff [--against X.Y.Z] [--snapshot] the public API since a release (tools/apidiff.py,
                                                                docs/VERSIONING.md): exit 1 for breaking changes
                                                                the library's version does not allow
  python build_rs.py changelog ["TEXT" --type T --area A]    show / add to [Unreleased] of CHANGELOG.md
  python build_rs.py release X.Y.Z [--date D] [--force]      CHANGELOG.md, the version in Cargo.toml, docs/api/X.Y.Z.txt
                                                                (tools/changelog.py)
  python build_rs.py site [--out DIR] [--check]              the docs of the website: guides, API reference, changelog
                                                                and roadmap from the Markdown -> build/site/
                                                                (tools/site.py); --check builds into a temporary folder
                                                                and checks links, anchors, images and the HTML (the
                                                                suite runs it)
  python build_rs.py i18n [APP|lib] [--update] [--unused]    the translatable texts against the catalogues
                                                                (tools/i18n_check.py): missing, untranslated, unused,
                                                                placeholders and plural forms; --update adds the
                                                                missing rows (untranslated); --check: the suite's
                                                                check (errors only, the gaps are reported)
  options: --target win|linux|both|web|all|android|mac|ios (default both; run: win), --release, --out DIR, -j N (Cargo's jobs),
           --sim (ios: the simulator's build into ios-sim; mac and ios build on a Mac, docs/APPLE.md; mac: test NAME
                 too, programs with windows start in the logged-in session from SSH),
           --profile size|o2 (release variants, section "r13-measure"),
           --arch x86_64|aarch64 (default x86_64; docs/ARM64.md, no run or test yet)

Cargo: every target builds in a Cargo folder of its own below <out> (cargo, cargo-glibc,
cargo-web-<mode>, cargo-android-<abi>, cargo-modules), side by side.  One `cargo
build` per target and run builds the apps in their own style and their BINS (Builder.cargo_build),
another one each further style; the programs come from Cargo's JSON messages.  The browser and
Android load an app as a library, Cargo cannot swap a dependency for a style variant: for both the
tool makes a package of each app in a workspace of its own (derived_workspace, <out>/cargo-<t>-ws);
the apps themselves stay programs.  The modules link over modrt / modplat (cargo_module).

Output folders (--out overrides): build/<target> for development builds, build/release/<target>
for --release, build/<profile>/<target> for --profile (so the variants never rebuild each
other).  --arch aarch64 puts win-arm64 and linux-arm64 in place of <target>.  The assembly
reference builds into legacy/build (legacy/build.py).

Linux: command-line and server programs are one static file without libc.  A program that names
ui links dynamically against glibc and nothing else (section "glibc"; every GPU driver is a glibc
library; its libraries in <out>/linux/.glibc, the program itself next to the others in <out>/linux):
also `gui` programs of apps/NAME/BINS and an app with a file apps/NAME/GLIBC.  An app with a file
apps/NAME/STATIC stays static, and --static builds the program of this run static (no GPU; for
systems without glibc, such as Alpine).

The profiles are the root Cargo.toml's.  Development builds (default): incremental, 16 codegen
units, opt-level 2 for the compute crates (gfx, text, simd, fx, img_*) and the crates from
crates.io, 1 for the rest (quick rebuilds), stripped; lib/ui/build.rs gives ui --cfg jfgui_dev
(what only a development build has: the live inspector's editing, ui/inspect_edit.rs; hot reload,
ui/hotreload.rs).  --release: opt-level 2, one codegen unit, fat LTO (as fast as opt-level 3 and
6-10 % smaller, rewrite/REPORT.md section 2; R13 measured with 3).  --profile adds --config to
it.  The rustflags are the tool's: -D warnings, no build paths in the panic locations, LLVM's
linkers.  An incremental cache left by a rustc killed during ThinLTO is discarded before the next
development build (incremental_flags).

Rules checked before every build (`check`):
  * every crate root starts with #![no_std]; only alloc / core are `extern crate`d
  * `unsafe` is allowed only in rt, simd, plat, gpu: every other library and style
    crate has #![forbid(unsafe_code)], every app #![deny(unsafe_code)] (its one exception
    is the ui::main! entry point)
  * every app, further program, app-local crate and style is a package whose manifest names the
    crates its sources use, the app's style and version (program_manifest_lint)
  * the facade lib/jflib names every library crate and style, a style's manifest every crate its
    sources name, and the licence files inside a crate equal the ones they copy (manifest_lint)
  * system call numbers only in lib/rt/kernel.rs, the same names for every architecture: no const
    SYS_* elsewhere, no number as the first argument of syscall* (a wrong number compiles and
    calls something else on another architecture, docs/ARM64.md)

Styles: an app is linked with its style (apps/NAME/STYLE or --style) as the crate `style`, and
with every further style crate its sources name (`style_kde::STYLE`, app_styles) under that
name, for the run-time style switch (ui.styles_add / ui.set_style, docs/STYLES.md); their
licence notices go next to the app as well.

Apps with several executables (section "app-bins"): apps/NAME/BINS lists further binaries
("NAME ENTRY cli|gui", e.g. "bc-server server/main.rs cli"), each in a folder of its own; cli
binaries link without a style and with the console subsystem on Windows.  Code shared by them
lives in app-local library crates apps/NAME/crates/C/lib.rs, built and linted like the crates of
LIBS and found by name the same way (source_crates).
"""
import argparse
import hashlib
import ipaddress
import json
import os
import re
import shutil
import shlex
import subprocess
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from tools.project_sources import SourceError, entrypoint, project_sources, resolve_root
from tools.host_exec import run_native
from tools import manifests

ROOT = Path(__file__).resolve().parent              # RustLib/
RS = ROOT
LLVM = Path(r"C:\Program Files\LLVM\bin")
TRIPLE = {"win": "x86_64-pc-windows-msvc", "linux": "x86_64-unknown-linux-gnu", "web": "wasm32-unknown-unknown", "android": "aarch64-linux-android",
          "mac": "aarch64-apple-darwin", "ios": "aarch64-apple-ios"}
# Apple (docs/APPLE.md): built on a Mac only; --sim builds ios for the simulator into ios-sim; the
# oldest systems the programs start on (decision 7)
APPLE = ("mac", "ios")
IOS_SIM_TRIPLE = "aarch64-apple-ios-sim"
APPLE_MIN = {"mac": ("MACOSX_DEPLOYMENT_TARGET", "13.0"), "ios": ("IPHONEOS_DEPLOYMENT_TARGET", "16.0")}
# --arch aarch64 (docs/ARM64.md): the same two targets for ARM64, each in a folder of its own
ARCHS = ("x86_64", "aarch64")
ARM64_TRIPLE = {"win": "aarch64-pc-windows-msvc", "linux": "aarch64-unknown-linux-gnu"}
ARM64_FOLDER = {"win": "win-arm64", "linux": "linux-arm64"}
RUSTC_VERSION = manifests.workspace()["package"]["rust-version"]  # the toolchain this code is written for (checked, minor ignored)
UNSAFE_OK = {"rt", "simd", "plat", "gpu"}   # gpu: the Vulkan FFI (lib/gpu/vk_*.rs)

# library crates: name -> (root file, dependencies), and what a crate needs on one target only:
# lib/*/Cargo.toml, in the order of the root Cargo.toml
LIBS, TARGET_DEPS = manifests.library_crates()
LIB_VERSION = manifests.version()
# rt::VERSION reads what Cargo sets for a crate's rustc; every rustc started from here gets it
os.environ["CARGO_PKG_VERSION"] = LIB_VERSION
STYLES = sorted(d.name for d in (ROOT / "styles").iterdir() if d.is_dir() and entrypoint(d, "lib", False))


class BuildError(Exception):
    pass


def arch_triple(target, arch):
    return ARM64_TRIPLE[target] if arch == "aarch64" else TRIPLE[target]


def arch_folder(target, arch):
    """the target's folder below the output folder, and its name in the build's messages"""
    return ARM64_FOLDER[target] if arch == "aarch64" else target


def tool(name):
    p = LLVM / (name + ".exe")
    return str(p) if os.name == "nt" and p.exists() else name


def run_cmd(cmd, quiet=False, env=None):
    """run a command (the NDK, llvm-rc, a probe's rustc) -> its output, BuildError on failure.  env:
    variables to set (None as a value: removed)"""
    full_env = None
    if env:
        full_env = {k: v for k, v in os.environ.items() if k not in env}
        full_env.update({k: v for k, v in env.items() if v is not None})
    r = subprocess.run([str(c) for c in cmd], capture_output=True, text=True, encoding="utf-8", errors="replace", env=full_env)
    out = (r.stdout + r.stderr).strip()
    if r.returncode != 0:
        raise BuildError(out or f"{cmd[0]} failed")
    if out and not quiet:
        print(out)
    return out


# ---------------------------------------------------------------- crates
def compute_crate(name):
    """crates whose speed matters in development builds too (opt-level 2; Kritzel draws with its
    own pixel loops)"""
    return (name in ("gfx", "text", "simd", "fx", "Kritzel", "crypto", "compress", "cas", "spell")
            or name.startswith("img_"))


def check_intrinsics(name, rlib):
    """a SIMD intrinsic compiled as a function of its own (instead of inlined into the kernel)
    means a kernel part ran outside the AVX2 context of simd::dispatch -- a closure or a function
    pointer inside a Kernel -- and costs a call per instruction: refuse the crate"""
    try:
        r = subprocess.run([tool("llvm-nm"), "--defined-only", str(rlib)], capture_output=True, text=True,
                           encoding="utf-8", errors="replace")
    except OSError:
        return  # no llvm-nm: the check is skipped
    bad = sorted({ln.split()[-1] for ln in r.stdout.splitlines() if "core_arch" in ln})
    if bad:
        sep = "\n  "
        raise BuildError(f"{name}: SIMD intrinsics compiled out of line (a closure or function pointer "
                         f"inside a simd::Kernel?):{sep}" + sep.join(bad[:10]))


def session_lock_held(lock):
    """is a rustc still working in this incremental session?  It holds the session's lock file
    while it runs: LockFileEx on the first byte on Windows, flock on Linux."""
    try:
        with open(lock, "r+b") as f:
            if os.name == "nt":
                import msvcrt
                msvcrt.locking(f.fileno(), msvcrt.LK_NBLCK, 1)
                msvcrt.locking(f.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                import fcntl
                fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except FileNotFoundError:
        return False
    except OSError:
        return True
    return False


def incremental_flags(inc):
    """-C incremental=inc, after discarding the cache if the last rustc there was killed during
    ThinLTO.  A new session (s-*-working, renamed when rustc finishes) starts as hard links to
    the files of the last good one, and ThinLTO overwrites thin-lto-past-keys.bin in place, so
    the kill (Ctrl+C, taskkill) has changed the good session's keys too.  The next rustc takes
    them for its own, keeps old objects although the modules they import from changed, and exits
    0 with an rlib whose objects name symbols nobody defines: every app that links it then fails
    with "undefined symbol: anon.<hash>.N.llvm.<hash>".
    A compile error leaves a -working folder too, but its keys are older than its lock file: the
    cache stays.  Only the newest session counts: an older -working folder shares its keys with
    the sessions after it, and this check has already run before each of them.
    Builder.cargo runs it on Cargo's incremental folder before every development build.  A cache
    that broke without this check (its rustc exited 0, nothing shows it here): delete
    <out>/<cargo folder>/<triple>/debug/incremental."""
    for crate in inc.glob("*"):
        newest = max((d for d in crate.glob("s-*") if d.is_dir()), key=lambda d: d.name, default=None)
        if newest is None or not newest.name.endswith("-working"):
            continue
        lock = newest.with_name(newest.name.removesuffix("-working") + ".lock")
        try:
            rewritten = (newest / "thin-lto-past-keys.bin").stat().st_mtime >= lock.stat().st_mtime
        except OSError:                     # no keys yet, or rustc is just removing the session
            continue
        if rewritten and not session_lock_held(lock):
            print(f"note: rustc was killed during ThinLTO, discarding its incremental cache {inc}")
            shutil.rmtree(inc, ignore_errors=True)
            break
    return ["-C", f"incremental={inc}"]


# a path's first segment (NAME::) and extern crate NAME: whole identifiers, like \bNAME:: and
# extern crate NAME\b per name -- one scan of the sources instead of one per name (all_libs ran
# a regex per crate over every source set: half a minute per app build)
CRATE_MENTION = re.compile(r"\b(\w+)::|extern crate (\w+)\b")


def named_crates(root, names):
    """the crates of names that one project's source set mentions (NAME:: or extern crate NAME)"""
    src = "\n".join(f.read_text(encoding="utf-8") for f in project_sources(root))
    found = {a or b for a, b in CRATE_MENTION.findall(src)}
    return {c for c in names if c in found}


def local_crates():
    """app-bins: the app-local library crates apps/NAME/crates/C/(src/)lib.rs -> {C: (root,
    deps)} in a valid build order.  They build, link and lint like the crates of LIBS; their
    dependencies are the LIBS and other local crates their sources name (transitively, in
    build order).  A local crate's name must not be taken by LIBS or another app's crate."""
    roots = {}
    for app in sorted(p for p in (RS / "apps").iterdir() if (p / "crates").is_dir()):
        for c in sorted(p for p in (app / "crates").iterdir() if p.is_dir()):
            r = entrypoint(c, "lib", False)
            if r is None:
                continue
            if c.name in LIBS or c.name in roots or c.name.startswith("style_"):
                raise BuildError(f"{c.relative_to(RS)}: the crate name {c.name} is already taken")
            roots[c.name] = r
    direct = {n: named_crates(r, list(LIBS) + [m for m in roots if m != n]) for n, r in roots.items()}
    order = []

    def visit(n, path):
        if n in path:
            raise BuildError("app-local crates depend on each other in a cycle: " + " -> ".join(path + [n]))
        if n not in order:
            for d in sorted(direct[n]):
                if d in roots:
                    visit(d, path + [n])
            order.append(n)
    for n in roots:
        visit(n, [])
    out = {}
    for n in order:
        need = set()

        def add(c):
            if c not in need:
                need.add(c)
                for d in (LIBS[c][1] if c in LIBS else out[c][1]):
                    add(d)
        for c in direct[n]:
            add(c)
        out[n] = (roots[n].relative_to(RS).as_posix(), [c for c in list(LIBS) + order if c in need])
    return out


def crate_table():
    """every library crate an app or binary can name: LIBS, then the app-local crates -> {name: deps}"""
    table = {n: deps for n, (_, deps) in LIBS.items()}
    table.update({n: deps for n, (_, deps) in local_crates().items()})
    return table


def source_crates(root):
    """Library dependencies named by one project's actual source set, in build order (LIBS,
    then the app-local crates).  Apps and styles share this discovery so vector styles can use
    the same rendering libraries without a special hard-coded style dependency list."""
    table = crate_table()
    used = named_crates(root, table)
    need = set()

    def add(c):
        if c not in need:
            need.add(c)
            for d in table[c]:
                add(d)
    for c in used:
        add(c)
    return [c for c in table if c in need]


def app_crates(name):
    """The app links only libraries referenced by its sources and their dependencies."""
    return source_crates(entrypoint(RS / "apps" / name, "app"))


def app_styles(name):
    """the style crates an app's sources name besides its own style (style_kde::STYLE ...):
    linked side by side for the run-time style switch (ui.styles_add / set_style, STYLES.md)"""
    return sorted(named_crates(entrypoint(RS / "apps" / name, "app"), [f"style_{s}" for s in STYLES]))


def style_crates():
    return {f"style_{s}": (entrypoint(RS / "styles" / s, "lib").relative_to(RS).as_posix(),
                           source_crates(entrypoint(RS / "styles" / s, "lib"))) for s in STYLES}


def all_libs(target=None):
    """every library crate -> (root, deps); with a target also what it needs there (TARGET_DEPS)"""
    d = {name: (resolve_root(RS / root).relative_to(RS).as_posix(), deps + TARGET_DEPS.get((name, target), []))
         for name, (root, deps) in LIBS.items()}
    d.update(local_crates())
    d.update(style_crates())
    return d


def apps():
    return sorted(p.name for p in (RS / "apps").iterdir() if p.is_dir() and entrypoint(p, "app", False))


def app_targets(name):
    f = ROOT / "apps" / name / "TARGETS"
    if f.exists():
        return f.read_text().split()
    root = entrypoint(ROOT / "apps" / name, "app")
    # Owned GUI runtimes can be hosted in a worker. Native CLI probes must opt in
    # explicitly (portabletest does); they may intentionally exercise OS syscalls.
    web = any("ui::launch(" in p.read_text(encoding="utf-8") for p in project_sources(root) if p.suffix == ".rs")
    return ["win", "linux", "web"] if web else ["win", "linux"]


# ---------------------------------------------------------------- permissions
# apps/NAME/PERMISSIONS: the app's permissions (docs/PERMISSIONS.md, lib/plat/policy.rs).  The
# build checks the syntax and hands the text to rustc as JFGUI_PERMISSIONS; ui::main! and
# rt::main! install it at start-up, in every program of the app (BINS and Android too).  A
# program of BINS may have its own apps/NAME/PERMISSIONS.<bin>.  Without the file the app may
# do everything (as before).  "ask KEY ARGS [\"reason\"]" asks the user at the first access;
# agent.confirm / agent.deny name agent actions (patterns) the user confirms / an agent may not run.
PERM_KEYS_ARGS = ("fs.read", "fs.write", "process", "plugins", "url", "net.connect", "net.listen")
PERM_KEYS_FLAG = ("fs.user", "print", "secrets", "autostart", "audio.output", "input.global", "screen.capture",
                  "location", "sensors")
PERM_KEYS_AGENT = ("agent.confirm", "agent.deny")                  # agent actions: confirmed / refused
PERM_KEYS_SANDBOX = ("sandbox",)                                   # OS sandbox: "sandbox [strict]"
PERM_KEYS_PACKAGE = ("recent",)                                    # only the packages read it (Flatpak)
PERM_ASKABLE = PERM_KEYS_ARGS + PERM_KEYS_FLAG[1:]                  # "ask KEY ARGS [\"reason\"]"
PERM_VARS = ("CONFIG", "TEMP", "HOME", "EXE", "APPIMAGE", "FONTS", "APPCONFIG", "APPDATA", "APPCACHE", "APPLOG",
             "DESKTOP", "DOCUMENTS", "DOWNLOADS", "PICTURES", "MUSIC", "VIDEOS")


def perm_pattern_error(a):
    """None when a is a valid path pattern of a manifest, else the reason"""
    if a == "**":
        return None
    if a.startswith("$"):
        var = re.split(r"[/\\]", a[1:], maxsplit=1)[0]
        return None if var in PERM_VARS else f"unknown variable ${var} ({', '.join('$' + v for v in PERM_VARS)})"
    if re.match(r"^[A-Za-z]:[/\\]", a) or a.startswith(("//", "\\\\", "/")):
        return None
    return f"{a} is not absolute (start it with /, X:/ or a variable)"


def perm_net_error(a, listen):
    """None when a is a valid HOST:PORT of net.connect / net.listen (as net_rule in policy.rs)"""
    bad = f"{a} is not HOST:PORT (example.org:443, *.example.org:*, 10.0.0.0/8:22, [::1]:8080, *:*)"
    host, sep, port = a.rpartition(":")
    if not sep:
        return bad
    lo_hi = port.split("-") if port != "*" else ["0", "65535"]
    if len(lo_hi) > 2 or not all(x.isdigit() and int(x) <= 65535 for x in lo_hi) or int(lo_hi[0]) > int(lo_hi[-1]):
        return bad
    if host == "*" or host.lower() == "localhost":
        return None
    h, slash, bits = host.rpartition("/") if "/" in host else (host, "", "")
    if slash and not bits.isdigit():
        return bad
    bracketed = len(h) >= 2 and h.startswith("[") and h.endswith("]")
    bare = h[1:-1] if bracketed else h
    if not bracketed and ":" in bare:
        return f"{a}: an IPv6 address needs brackets ([::1]:80)"
    try:
        ip = ipaddress.ip_address(bare)
    except ValueError:
        ip = None
    if ip is not None:
        if (ip.version == 4) == bracketed or (slash and int(bits) > ip.max_prefixlen):
            return bad
        return None
    if bracketed or slash:
        return bad
    if listen:
        return f"{a}: net.listen takes an address, a block, localhost or *"
    labels = bare.rstrip(".").lower()
    labels = labels[2:] if labels.startswith("*.") else labels
    if not labels or not all(re.fullmatch(r"[a-z0-9_-]+", x) for x in labels.split(".")):
        return bad
    return None


def perm_split(ln):
    """a manifest line -> (words, reason or None): the reason is a "quoted text" at the end;
    '#' outside it starts a comment (as split_line in policy.rs)"""
    marks = [k for k in (ln.find("#"), ln.find('"')) if k >= 0]
    cut = min(marks) if marks else len(ln)
    if cut == len(ln) or ln[cut] == "#":
        return ln[:cut].split(), None
    rest = ln[cut + 1:]
    end = rest.find('"')
    if end < 0:
        raise ValueError("the reason has no closing quote")
    after = rest[end + 1:].lstrip()
    if after and not after.startswith("#"):
        raise ValueError("nothing but a comment may follow the reason")
    return ln[:cut].split(), rest[:end]


def app_permissions(name, bin=None):
    """the checked text of apps/NAME/PERMISSIONS (a BINS program: its PERMISSIONS.<bin> when
    there is one), or None without the file"""
    f = ROOT / "apps" / name / "PERMISSIONS"
    if bin is not None and (ROOT / "apps" / name / f"PERMISSIONS.{bin}").exists():
        f = ROOT / "apps" / name / f"PERMISSIONS.{bin}"
    if not f.exists():
        return None
    text = f.read_text(encoding="utf-8")
    perm_check(text, f.relative_to(ROOT).as_posix())
    return text


def perm_check(text, where):
    """raise BuildError for the first bad line of a manifest (the rules of parse in policy.rs)"""
    for i, ln in enumerate(text.splitlines(), 1):
        try:
            words, reason = perm_split(ln)
        except ValueError as e:
            raise BuildError(f"{where}:{i}: {e}")
        if words[:1] == ["ask"]:
            words = words[1:]
            if not words:
                raise BuildError(f"{where}:{i}: ask needs a permission (ask fs.read $DOCUMENTS/**)")
            if words[0] not in PERM_ASKABLE:
                raise BuildError(f"{where}:{i}: {words[0]} cannot be asked for ({', '.join(PERM_ASKABLE)})")
            if reason is not None and (len(reason.encode("utf-8")) > 200 or any(ord(c) < 0x20 for c in reason)):
                raise BuildError(f"{where}:{i}: a reason has at most 200 bytes and no control characters")
        elif reason is not None:
            raise BuildError(f"{where}:{i}: a reason belongs to an ask line")
        if not words:
            continue
        key, args = words[0], words[1:]
        if key in PERM_KEYS_AGENT:
            if not args:
                raise BuildError(f"{where}:{i}: {key} needs at least one action name (document.save, document.*, *)")
            for a in args:
                if not re.fullmatch(r"[A-Za-z0-9._*?-]{1,128}", a):
                    raise BuildError(f"{where}:{i}: {a} is not an action name pattern (letters, digits, . _ - and * ?)")
        elif key in PERM_KEYS_SANDBOX:
            if args not in ([], ["strict"]):
                raise BuildError(f"{where}:{i}: sandbox takes nothing or 'strict'")
        elif key in PERM_KEYS_PACKAGE:
            if args:
                raise BuildError(f"{where}:{i}: {key} takes no arguments")
        elif key in PERM_KEYS_FLAG:
            if args:
                raise BuildError(f"{where}:{i}: {key} takes no arguments")
        elif key in PERM_KEYS_ARGS:
            if not args:
                raise BuildError(f"{where}:{i}: {key} needs at least one argument")
            for a in args:
                if key == "url":
                    if a != "*" and not re.fullmatch(r"[A-Za-z0-9+.-]+", a):
                        raise BuildError(f"{where}:{i}: {a} is not a URL scheme (https, mailto, ... or *)")
                elif key in ("net.connect", "net.listen"):
                    if (err := perm_net_error(a, key == "net.listen")):
                        raise BuildError(f"{where}:{i}: {err}")
                elif key == "process" and not re.search(r"[/\\$]", a):
                    continue                                    # a program name looked up in PATH
                elif (err := perm_pattern_error(a)):
                    raise BuildError(f"{where}:{i}: {err}")
        else:
            raise BuildError(f"{where}:{i}: unknown permission {key} "
                             f"({', '.join(PERM_KEYS_ARGS[:2] + PERM_KEYS_FLAG[:1] + PERM_KEYS_ARGS[2:] + PERM_KEYS_FLAG[1:] + PERM_KEYS_SANDBOX + PERM_KEYS_PACKAGE + ('ask',) + PERM_KEYS_AGENT)})")


# manifests and the error perm_check must give (None: valid); `check` runs them, so the build's
# syntax stays the one plattest --test permask checks in policy.rs
PERM_CASES = [
    ('ask fs.read $DOCUMENTS/** "Briefe # öffnen"  # a comment\nask net.connect *:443\nask print\n'
     'agent.confirm doc.* ui.type\nagent.deny *.wipe', None),
    ("ask fs.user", "fs.user cannot be asked for"),
    ("fs.user\nrecent", None),
    ("recent all", "recent takes no arguments"),
    ("ask recent", "recent cannot be asked for"),
    ("print\nask", ":2: ask needs a permission"),
    ('fs.read /x "why"', "a reason belongs to an ask line"),
    ('ask url mailto "open', "no closing quote"),
    ('ask url mailto "a" b', "nothing but a comment may follow"),
    ("ask agent.deny doc.x", "agent.deny cannot be asked for"),
    ("agent.confirm doc/x", "is not an action name pattern"),
    ("agent.deny", "needs at least one action name"),
    ("sandbox\nsandbox strict", None),
    ("sandbox loose", "sandbox takes nothing or 'strict'"),
    ('ask print "a\x07b"', "no control characters"),
    ("fs.read $PLUGIN/x", "unknown variable $PLUGIN"),
    ('audio.output\nask audio.output "Klänge"', None),
    ("audio.output on", "audio.output takes no arguments"),
    ("ask audio.input", "audio.input cannot be asked for"),
]


def perm_lint():
    """the manifest checks against PERM_CASES, then every app's manifests -> problems"""
    bad = []
    for text, want in PERM_CASES:
        try:
            perm_check(text, "case")
            got = None
        except BuildError as e:
            got = str(e)
        if (want is None) != (got is None) or (want and want not in got):
            bad.append(f"build_rs.py: the manifest check of {text!r} gives {got or 'no error'}, not {want or 'no error'}")
    for a in apps():
        for b in [None] + [x[0] for x in app_bins(a)]:
            try:
                app_permissions(a, b)
            except BuildError as e:
                bad.append(str(e))
    return bad


SEMVER = re.compile(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?")


def app_version(name):
    """version= of apps/NAME/PACKAGE (the app's own SemVer version, not the library's), None
    without one; every program of the app gets it as JFGUI_VERSION (rt::app_version!())"""
    f = ROOT / "apps" / name / "PACKAGE"
    if not f.exists():
        return None
    v = next((ln.split("=", 1)[1].strip() for ln in f.read_text(encoding="utf-8").splitlines()
              if ln.strip().startswith("version") and ln.split("=", 1)[0].strip() == "version"), None)
    if v is not None and not SEMVER.fullmatch(v):
        raise BuildError(f"apps/{name}/PACKAGE: version={v} is no MAJOR.MINOR.PATCH version (docs/VERSIONING.md)")
    return v


def app_style(name):
    f = ROOT / "apps" / name / "STYLE"
    s = f.read_text().strip() if f.exists() else "winui"
    return s if s in STYLES else "winui"


# ---------------------------------------------------------------- app-bins
# An app folder builds one GUI exe (src/main.rs or main.rs).  apps/NAME/BINS adds further
# executables next to it, one line each: "BIN ENTRY KIND" (# starts a comment), ENTRY the
# main.rs of a folder of its own below apps/NAME (its source set is that folder), KIND cli
# (no style; Windows: console subsystem, so a shell waits for it and shows its output) or gui
# (the app's style, Windows subsystem).  They link the crates their sources name (LIBS and the
# app's local crates, see local_crates) and are built with the app for the native targets
# (Builder.build_bin, build_app_bins), installed next to its exe by tools/package.py.  Like the
# GUI exe they get the app's name, assets and manifest (Builder.boot_args; their own
# apps/NAME/PERMISSIONS.<bin> when there is one).
BIN_KINDS = ("cli", "gui")


def app_bins(name):
    """apps/NAME/BINS -> [(bin name, entry main.rs, kind)]"""
    f = ROOT / "apps" / name / "BINS"
    if not f.exists():
        return []
    gui = entrypoint(ROOT / "apps" / name, "app", False)
    out = []
    for i, ln in enumerate(f.read_text(encoding="utf-8").splitlines(), 1):
        p = ln.split("#")[0].split()
        if not p:
            continue
        if len(p) != 3 or p[2] not in BIN_KINDS or not re.fullmatch(r"[A-Za-z][A-Za-z0-9_-]*", p[0]):
            raise BuildError(f"{f.relative_to(ROOT)}:{i}: expected BIN ENTRY cli|gui")
        root = (ROOT / "apps" / name / p[1]).resolve()
        if root.name != "main.rs" or not root.is_file():
            raise BuildError(f"{f.relative_to(ROOT)}:{i}: {p[1]} is not a main.rs")
        if gui is not None and resolve_root(gui).parent in (root.parent, *root.parents):
            raise BuildError(f"{f.relative_to(ROOT)}:{i}: {p[1]} lies inside the GUI's source folder")
        if p[0] == name or p[0] in [b for b, _, _ in out]:
            raise BuildError(f"{f.relative_to(ROOT)}:{i}: the name {p[0]} is already taken")
        out.append((p[0], root, p[2]))
    for extra in sorted((ROOT / "apps" / name).glob("PERMISSIONS.*")):       # a program's own manifest
        if extra.name[len("PERMISSIONS."):] not in [b for b, _, _ in out]:
            raise BuildError(f"{extra.relative_to(ROOT)}: {extra.name[len('PERMISSIONS.'):]} is no program of apps/{name}/BINS")
    return out


def bin_file(name, target):
    return name + (".exe" if target == "win" else "")


def app_bin_files(app, target):
    """the file names of an app's further executables (tools/package.py)"""
    return [bin_file(b, target) for b, _, _ in app_bins(app)] if target in ("win", "linux") else []
# ---------------------------------------------------------------- end app-bins


# ---------------------------------------------------------------- lint
KERNEL_TABLE = RS / "lib" / "rt" / "kernel.rs"
KERNEL_X86_ONLY = {"SYS_ARCH_PRCTL"}          # arch_prctl: ARM64 sets the thread pointer without a call
SYSCALL_NUMBER = re.compile(r"\bsyscall\d*\s*\(\s*(?:/\*.*?\*/\s*)?\d")
SYS_CONST = re.compile(r"\bconst\s+SYS_\w+\s*:\s*[iu](?:size|\d+)\b")


def kernel_lint(f, text):
    """the system call rules for one source file -> problems"""
    if f == KERNEL_TABLE:
        return []
    found = [(m, "a number as the system call") for m in SYSCALL_NUMBER.finditer(text)]
    found += [(m, "a system call number outside the table") for m in SYS_CONST.finditer(text)]
    return [f"{f.relative_to(ROOT)}:{text.count(chr(10), 0, m.start()) + 1}: {why} (lib/rt/kernel.rs)" for m, why in found]


def kernel_table_lint():
    """both tables of lib/rt/kernel.rs name the same things (only x86-64 builds today, so the
    compiler would not tell) -> problems"""
    tables = re.split(r'#\[cfg\(target_arch = "\w+"\)\]\s*mod arch', KERNEL_TABLE.read_text(encoding="utf-8"))[1:]
    if len(tables) != 2:
        return ["lib/rt/kernel.rs: expected two tables, x86-64 and aarch64"]
    x86, arm = (set(re.findall(r"pub (?:const (?:fn )?|struct )(\w+)", t)) | set(re.findall(r"pub (st_\w+):", t)) for t in tables)
    return [f"lib/rt/kernel.rs: {n} only in the x86-64 table" for n in sorted(x86 - arm - KERNEL_X86_ONLY)] + \
           [f"lib/rt/kernel.rs: {n} only in the aarch64 table" for n in sorted(arm - x86)]


def same_text(a, b):
    """the same bytes but for line ends (one checkout may hold both forms)"""
    return a.read_bytes().replace(b"\r\n", b"\n") == b.read_bytes().replace(b"\r\n", b"\n")


def program_manifest_lint(members):
    """the manifests outside lib/ against what build_rs.py builds: every style, app, further program
    (BINS) and app-local crate is a member whose manifest names the crates its sources use; an app's
    and its programs' version is PACKAGE's, their `style` the app's STYLE -> problems"""
    bad = []
    names = list(crate_table()) + [f"style_{s}" for s in STYLES]
    mset = set(members)

    def check(member, root, version=None, style=None):
        if member not in mset:
            return bad.append(f"{member}: no member of the workspace (the root Cargo.toml; python build_rs.py new writes both)")
        m = manifests.read(ROOT / member / "Cargo.toml")
        deps = m.get("dependencies", {})
        own = m.get("lib", {}).get("name")
        for d in sorted(named_crates(root, names) - set(deps) - {own}):
            bad.append(f"{member}/Cargo.toml: {d} is missing, its sources name it")
        if version is not None and m["package"]["version"] != version:
            bad.append(f"{member}/Cargo.toml: version {m['package']['version']}, PACKAGE says {version}")
        if style is not None and "style" in deps and deps["style"].get("package") != f"jflib-style-{style}":
            bad.append(f"{member}/Cargo.toml: style is {deps['style'].get('package')}, STYLE says jflib-style-{style}")
        if style is not None and "style" not in deps and named_crates(root, ["style"]):
            bad.append(f"{member}/Cargo.toml: style is missing, its sources name it")
    for s in STYLES:
        check(f"styles/{s}", entrypoint(ROOT / "styles" / s, "lib"))
    for a in apps():
        version = app_version(a) or "0.0.0"
        check(f"apps/{a}", entrypoint(ROOT / "apps" / a, "app"), version, app_style(a))
        for _, root, kind in app_bins(a):
            check(root.parent.relative_to(ROOT).as_posix(), root, version, app_style(a) if kind == "gui" else None)
    for n, (r, _) in local_crates().items():
        check(resolve_root(ROOT / r).parent.relative_to(ROOT).as_posix().removesuffix("/src"), ROOT / r)
    return bad


def manifest_lint():
    """what rustc does not see, Cargo would only find in a build of its own (docs/CRATES.md) ->
    problems: every crate is in the facade, a style's manifest has the crates its sources name
    (build_rs.py finds them by name), every package holds the licences it needs, the template
    of lib/jflib is current"""
    bad = []
    members = manifests.workspace()["members"]
    published = [m for m in members if manifests.read(ROOT / m / "Cargo.toml")["package"].get("publish", True)]
    styles = {f"style_{Path(m).name}": m for m in published if m.startswith("styles/")}
    facade = manifests.read(ROOT / manifests.FACADE / "Cargo.toml")
    named = set(facade["dependencies"]) | {d for t in facade["target"].values() for d in t["dependencies"]}
    # a crate is named by `pub use NAME;` or, a style, by `pub mod NAME { pub use ::NAME::STYLE; }`
    uses = set(re.findall(r"^pub (?:use|mod) (\w+)(?:;| \{)", (ROOT / manifests.FACADE / "src/lib.rs").read_text(encoding="utf-8"), re.M))
    for c in [*LIBS, *styles]:
        if c not in named:
            bad.append(f"{manifests.FACADE}/Cargo.toml: {c} is missing, the facade brings every crate")
        if c not in uses:
            bad.append(f"{manifests.FACADE}/src/lib.rs: pub use {c}; is missing")
    # what the facade shows is what docs/API.md documents: that is what jflib promises
    # (docs/VERSIONING.md, "The crates on crates.io"); a building block is #[doc(hidden)] there
    if str(RS / "tools") not in sys.path:
        sys.path.insert(0, str(RS / "tools"))  # gendoc imports its neighbours by name
    import gendoc
    text = (ROOT / manifests.FACADE / "src/lib.rs").read_text(encoding="utf-8")
    hidden = set(re.findall(r"^#\[doc\(hidden\)\]\s*\npub use (\w+);", text, re.M))
    visible = set(re.findall(r"^pub use (\w+);", text, re.M)) - hidden
    documented = {c[0] for c in gendoc.CRATES} - gendoc.REPO_ONLY
    for c in sorted(visible - documented):
        bad.append(f"{manifests.FACADE}/src/lib.rs: {c} is shown but tools/gendoc.py does not document it: "
                   "add it to CRATES or mark the pub use #[doc(hidden)]")
    for c in sorted(documented - visible):
        bad.append(f"tools/gendoc.py: {c} is documented, but the facade does not show it (pub use {c};)")
    bad += program_manifest_lint(members)
    for m in published:
        for name in ("LICENSE-MIT", "LICENSE-APACHE"):
            copy = ROOT / m / name
            if not copy.is_file() or not same_text(copy, ROOT / name):
                bad.append(f"{m}/{name}: differs from {name}: copy that file over it")
        for copy in sorted(p for p in (ROOT / m / "licenses").glob("*") if p.is_file()):
            source = ROOT / "licenses" / copy.name
            if not source.is_file() or not same_text(copy, source):
                bad.append(f"{copy.relative_to(ROOT).as_posix()}: differs from licenses/{copy.name}: copy that file over it")
    # cargo-jflib carries the template and the browser host in its package: copies of these folders
    for copy_dir, source_dir in (("lib/cargo_jflib/template", "lib/jflib/template"), ("lib/cargo_jflib/web", "web"), ("lib/cargo_jflib/android", "android")):
        files = {p.relative_to(ROOT / d).as_posix() for d in (copy_dir, source_dir) for p in (ROOT / d).rglob("*")
                 if p.is_file() and "target" not in p.relative_to(ROOT / d).parts}
        for rel in sorted(files):
            a, b = ROOT / source_dir / rel, ROOT / copy_dir / rel
            if not a.is_file() or not b.is_file() or not same_text(a, b):
                bad.append(f"{copy_dir}/{rel}: differs from {source_dir}/{rel}: copy that folder over it")
    from tools import crates_accept                 # the template of lib/jflib comes from apps/template
    return bad + crates_accept.template_problems()


def lint():
    """the rules of the module docstring -> list of problems"""
    bad = kernel_table_lint()
    libs = all_libs()
    roots = [(n, RS / r, "lib") for n, (r, _) in libs.items()] + \
            [(a, entrypoint(RS / "apps" / a, "app"), "app") for a in apps()] + \
            [(b, root, "app") for a in apps() for b, root, _ in app_bins(a)]     # app-bins
    for name, root, kind in roots:
        files = project_sources(root)
        head = root.read_text(encoding="utf-8")
        if "#![no_std]" not in head:
            bad.append(f"{root.relative_to(ROOT)}: missing #![no_std]")
        if kind == "lib" and name not in UNSAFE_OK and "#![forbid(unsafe_code)]" not in head:
            bad.append(f"{root.relative_to(ROOT)}: missing #![forbid(unsafe_code)]")
        if kind == "app" and "#![deny(unsafe_code)]" not in head and "#![forbid(unsafe_code)]" not in head:
            bad.append(f"{root.relative_to(ROOT)}: missing #![deny(unsafe_code)]")
        for f in files:
            text = f.read_text(encoding="utf-8")
            bad += kernel_lint(f, text)
            # a program with std (feature std, docs/CRATES.md CR6): rt and plat keep what names std in
            # hosted.rs, compiled only with that feature; std is an extern crate only behind it
            hosted = kind == "lib" and f.name == "hosted.rs" and name in ("rt", "plat")
            prev = ""
            for i, ln in enumerate(text.splitlines(), 1):
                s = ln.strip()
                std_crate = s == "extern crate std;" and prev == '#[cfg(feature = "std")]' and name in ("rt", "plat")
                if s.startswith("extern crate") and not std_crate and s not in (
                        "extern crate alloc;", "extern crate core;", "extern crate modrt as rt;", "extern crate modplat as plat;"):
                    bad.append(f"{f.relative_to(ROOT)}:{i}: only alloc / core may be extern crates")
                if not hosted and (s.startswith("use std") or " std::" in s.split("//")[0]):
                    bad.append(f"{f.relative_to(ROOT)}:{i}: std is not available (no_std)")
                prev = s
                if ln[:2] in ('")', '",', '";'):
                    bad.append(f"{f.relative_to(ROOT)}:{i}: a string literal broken by a line end (a script edit?)")
    return bad + module_lint() + perm_lint() + manifest_lint()   # jxl-module: modules/*; manifests


# ---------------------------------------------------------------- r13-measure: build profiles
# --profile NAME (implies --release): other settings than the default release (opt-level 2
# everywhere, fat LTO), measured in rewrite/REPORT.md (R13, size experiment; the numbers below
# are relative to the R13 release with opt-level 3).  lib = the crates
# that are no compute crate (rt, plat, base, ui, styles), compute = compute_crate() (gfx, text,
# simd, fx, img_*), app = the app crate, which also sets the pipeline of the LTO step (an app
# at s / z slows down every crate's code: rendering up to 2x); lto = fat | off.
#   size  z / 2 / 2, fat LTO: gallery -21 %, Kritzel -19 %, PaintCraft -14 % (win); codecs,
#         effects, rendering unchanged, Kritzel + PaintCraft scripts +4 %, frame build +20 us
#   o2    2 everywhere, no LTO: gallery -11 %, Kritzel -10 %, PaintCraft -13 %; speed unchanged
# opt-level z for the compute crates is not possible: slice helpers over __m128i / __m256i then
# stay out of line (check_intrinsics refuses img_jxl).  Use a separate --out per profile.
PROFILES = {
    "size": {"lib": "z", "compute": "2", "app": "2", "lto": "fat"},
    "o2": {"lib": "2", "compute": "2", "app": "2", "lto": "off"},
}
# ---------------------------------------------------------------- end r13-measure


# ---------------------------------------------------------------- glibc
# Every Linux GPU driver is a glibc library, so a program that uses the GPU needs glibc in its
# process (docs/GPU.md "Linux").  A program that names ui (its windows go through the GPU, lib/ui/
# gpu_out.rs) or an app with a file apps/NAME/GLIBC is built that way: dynamically linked against
# glibc and nothing else, every crate with --cfg jfgui_glibc (lib/plat/linux_glibc.rs: the process
# start, pthread threads, exit, dlopen).  Everything else stays static without libc, and so does
# an app with a file apps/NAME/STATIC or any UI program under --static (gpu_out_off.rs: no Vulkan
# code; its windows through shared memory only; for systems without glibc).
#
# Nothing has to be installed for it: jflib-build writes the three libraries to link against
# (lib/jflib_build/src/target.rs, GLIBC_STUBS).  They only name what the program imports and at which
# version; at run time the system's libraries are used.  Every symbol is asked for at the oldest
# version the architecture has (x86-64 GLIBC_2.2.5, ARM64 GLIBC_2.17): glibc up to 2.33 has these
# functions in libdl and libpthread under that version; 2.34 moved them into libc, keeps that
# version as a compatibility symbol there and libdl / libpthread as empty files, so the same binary
# starts on both.  The program imports no data, only functions (no copy relocations), and exports
# nothing: rt's memcpy never replaces the drivers'.
STATIC_UI = False          # --static


def set_static_ui(on):
    """--static: the UI programs of this run are linked static (no GPU)"""
    global STATIC_UI
    STATIC_UI = on


def app_glibc(name):
    """the app is linked against glibc on Linux: apps/NAME/GLIBC; a program that names ui unless
    apps/NAME/STATIC or --static says otherwise"""
    d = ROOT / "apps" / name
    if (d / "GLIBC").is_file():
        return True
    if STATIC_UI or (d / "STATIC").is_file():
        return False
    return entrypoint(d, "app", False) is not None and "ui" in app_crates(name)


# ---------------------------------------------------------------- end glibc


# ---------------------------------------------------------------- build
class Builder:
    """the programs of one target (and Linux: its glibc twin) through Cargo: one `cargo build` per run
    into <out>/<target dir> (cargo_build), the programs copied to <out>/<folder> as before"""

    def __init__(self, target, out, release, jobs, profile=None, web_mode="scalar", android_abi="arm64-v8a", glibc_base=None,
                 arch="x86_64", ios_sim=False):
        self.target = target
        self.arch = arch
        self.ios_sim = ios_sim and target == "ios"    # Apple: the simulator's build (docs/APPLE.md)
        self.folder = "ios-sim" if self.ios_sim else arch_folder(target, arch)
        self.out = out
        self.profile_name = profile if isinstance(profile, str) else None
        self.dir = out / self.folder
        self.public = self.dir                # where the programs go
        self.glibc = glibc_base is not None   # section "glibc": this Builder makes its programs
        self.base = glibc_base or self        # the static Builder (the modules, BINS)
        self.twin = None
        self.twin_lock = threading.Lock()
        if self.glibc:
            self.dir = self.dir / ".glibc"
        self.android_abi = android_abi
        self.android_output = self.dir
        if target == "android":
            self.dir = self.dir / ".native" / android_abi
        self.web_mode = web_mode
        self.web_public = self.dir
        if target == "web" and web_mode != "scalar":
            self.dir = self.dir / ("." + web_mode)
        self.web_sysroot = None
        if target == "web" and web_mode == "threads":
            from tools.web_sysroot import ensure
            self.web_sysroot = ensure(self.dir / "sysroot")
        self.release = release or profile is not None
        # r13-measure: a name of PROFILES or a dict of the same keys (tools/measure.py's variants)
        self.profile = PROFILES[profile] if isinstance(profile, str) else profile
        self.jobs = jobs
        self.dir.mkdir(parents=True, exist_ok=True)
        self.mod_lock = threading.Lock()      # jxl-module: an app's modules, once per run
        self.mod_ready = set()
        self.cargo_out = {}                   # package -> (its program or library, its map) in Cargo's folder
        self.deps_dir = None                  # cargo_rlibs: the folder of the rlibs (probe_flags)

    def triple(self):
        if self.ios_sim:
            return IOS_SIM_TRIPLE
        return "x86_64-linux-android" if self.target == "android" and self.android_abi == "x86_64" else arch_triple(self.target, self.arch)

    def target_dir(self):
        folder = {"win": "cargo", "linux": "cargo-glibc" if self.glibc else "cargo", "web": f"cargo-web-{self.web_mode}",
                  "android": f"cargo-android-{self.android_abi}", "mac": "cargo-mac",
                  "ios": "cargo-ios-sim" if self.ios_sim else "cargo-ios"}[self.target]
        return self.out / (folder if self.arch == "x86_64" else f"{folder}-{self.arch}")

    def glibc_twin(self):
        """section "glibc": the Builder of the programs linked against glibc (once per Builder): its
        own Cargo folder, its programs next to the static ones"""
        with self.twin_lock:
            if self.twin is None:
                self.twin = Builder(self.target, self.out, self.release, self.jobs, self.profile, glibc_base=self, arch=self.arch)
            return self.twin

    def cargo_flags(self):
        """the rustflags of this target (the profile is Cargo's): warnings are errors, no build paths in
        the panic locations, the linker of LLVM, what the Linux and browser builds need"""
        flags = ["-D", "warnings", f"--remap-path-prefix={ROOT}{os.sep}="] + list((self.profile or {}).get("rustflags", []))
        if self.target == "win":
            return flags + ["-C", f"linker={tool('lld-link')}"]
        if self.target == "android":
            return flags                                   # a static library: tools/android.py links it
        if self.target in APPLE:
            return flags                                   # Apple's linker as rustc calls it (libSystem)
        if self.target == "web":
            flags += ["-C", f"linker={tool('wasm-ld')}"]
            if self.web_mode != "scalar":
                flags += ["-C", "target-feature=+simd128" + (",+atomics,+bulk-memory" if self.web_mode == "threads" else "")]
            if self.web_mode == "threads":                 # LLVM supports wasm atomics on stable; Rust labels them unstable
                flags += ["--cfg", "web_threads", "-A", "unstable-features", "--sysroot", str(self.web_sysroot)]
            return flags
        flags += ["-C", f"linker={tool('ld.lld')}", "-C", "linker-flavor=ld", "-C", "relocation-model=static"]
        return flags + (["--cfg", "jfgui_glibc"] if self.glibc else ["-C", "target-feature=+crt-static"])

    def cargo_profile(self, apps_):
        """--profile (r13-measure): the release profile with other levels, as --config arguments: lib
        for every crate, compute for the compute crates, app for the programs of apps_"""
        p = self.profile
        if not p:
            return []

        def level(v):
            return toml_value(int(v) if str(v).isdigit() else str(v))
        lto = {"fat": '"fat"', "thin": '"thin"', "off": "false"}[p["lto"]]
        args = [f"profile.release.opt-level={level(p['lib'])}", f"profile.release.lto={lto}",
                f"profile.release.codegen-units={int(p.get('cgu', 1))}"]
        args += [f'profile.release.package."jflib-{c.replace("_", "-")}".opt-level={level(p["compute"])}' for c in LIBS if compute_crate(c)]
        args += [f'profile.release.package."{a}".opt-level={level(p["app"])}' for a in apps_]
        return [x for a in args for x in ("--config", a)]

    def cargo(self, cmd, env=None):
        """run cargo with this target's rustflags into its folder -> (stdout, stderr); BuildError on failure"""
        full = {k: v for k, v in os.environ.items() if k != "RUSTFLAGS"}
        full.update({"CARGO_TARGET_DIR": str(self.target_dir()), "CARGO_ENCODED_RUSTFLAGS": "\x1f".join(self.cargo_flags())})
        if self.target in APPLE:                         # the oldest macOS / iOS the program starts on
            full.setdefault(*APPLE_MIN[self.target])
        full.update(env or {})
        if not self.release:             # a rustc killed during ThinLTO poisons Cargo's cache as it did rustc's
            incremental_flags(Path(full["CARGO_TARGET_DIR"]) / self.triple() / "debug" / "incremental")
        cmd = cmd[:2] + ["--target", self.triple(), "--message-format=json-render-diagnostics"] + (["--release"] if self.release else []) + \
            (["-j", str(self.jobs)] if self.jobs else []) + cmd[2:]
        r = subprocess.run(cmd, cwd=ROOT, env=full, capture_output=True, text=True, encoding="utf-8", errors="replace")
        if r.returncode:
            raise BuildError("\n".join(ln for ln in r.stderr.splitlines()
                                       if not ln.lstrip().startswith(("Compiling", "Checking", "Fresh", "Blocking", "warning: profile package spec")))
                             or f"{' '.join(cmd[:4])} failed")
        return r.stdout, r.stderr

    def cargo_packages(self, names, style=None):
        """the packages this Builder's cargo_build builds for these apps: the apps (on Linux the static
        Builder the apps without a UI and every cli program, its glibc twin the others) and their BINS,
        a library or a style variant the package derived_workspace makes"""
        if style is not None or self.target in ("web", "android"):
            return [derived_name(n, self.target, style) for n in names if self.target != "linux" or app_glibc(n) == self.glibc]
        if self.target != "linux":
            return [p for n in names for p in [n] + [b for b, _, _ in app_bins(n)]]
        out = []
        for n in names:
            glibc = app_glibc(n)
            if glibc == self.glibc:
                out.append(n)
            out += [b for b, _, k in app_bins(n) if (k == "gui" and glibc) == self.glibc]
        return out

    def cargo_builder(self, app, kind=None):
        """the Builder whose Cargo run builds the app (kind None) or one of its BINS"""
        if self.target != "linux":
            return self
        base = self.base
        return base.glibc_twin() if app_glibc(app) and kind in (None, "gui") else base

    def cargo_build(self, names, style=None):
        """the programs of these apps and of their BINS in one `cargo build` (style: a variant in that
        style, derived_workspace) -> self.cargo_out {package: (its program or library, its map)}.  The
        compute crates' rlibs that were compiled again go through check_intrinsics."""
        if self.target == "linux" and not self.glibc:      # section "glibc": the twin's share
            twin = [n for n in names if self.glibc_twin().cargo_packages([n], style)]
            if twin:
                self.glibc_twin().cargo_build(twin, style)
        pkgs = self.cargo_packages(names, style)
        if not pkgs:
            return
        cmd = ["cargo", "build", "--locked"]
        if style is not None or self.target in ("web", "android"):     # a workspace of the tool's own
            cmd = ["cargo", "build", "--offline", "--manifest-path", str(derived_workspace(self.out, self.target, names, style))]
        out, _ = self.cargo(cmd + self.cargo_profile(pkgs) + [x for p in pkgs for x in ("-p", p)])
        out_dirs, programs = {}, {}
        for ln in out.splitlines():
            m = json.loads(ln) if ln.startswith("{") else {}
            if m.get("reason") == "build-script-executed":
                out_dirs[m["package_id"]] = Path(m["out_dir"])
            if m.get("reason") != "compiler-artifact":
                continue
            kinds, crate, pkg = m["target"]["kind"], m["target"]["name"], package_of(m["package_id"])
            if "bin" in kinds and m.get("executable"):
                programs[pkg] = (Path(m["executable"]), m["package_id"])
            elif ("cdylib" in kinds or "staticlib" in kinds) and pkg in pkgs:
                lib = next((Path(f) for f in m["filenames"] if f.endswith((".wasm", ".a"))), None)
                if lib is not None:
                    self.cargo_out[pkg] = (lib, None)
            elif "lib" in kinds and not m.get("fresh") and compute_crate(crate) and crate != "simd" and self.target != "web":
                for f in m["filenames"]:
                    if f.endswith(".rlib"):
                        check_intrinsics(crate, Path(f))
        for pkg, (exe, pid) in programs.items():             # jflib-build writes the map to OUT_DIR
            self.cargo_out[pkg] = (exe, out_dirs.get(pid, exe.parent) / "program.map")

    def cargo_copy(self, package, dest):
        """a program Cargo built (and its .map) -> dest"""
        if package not in self.cargo_out:
            raise BuildError(f"cargo built no program {package}")
        src, mapf = self.cargo_out[package]
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(src, dest)
        shutil.copymode(src, dest)             # Linux: the program stays executable
        if mapf is not None and mapf.is_file():    # Windows NAME.map, Linux NAME.map (as they were named before)
            shutil.copyfile(mapf, dest.with_suffix(".map") if self.target == "win" else dest.with_name(dest.name + ".map"))
        return dest

    def cargo_libs(self):
        """`build_rs.py lib`: every library crate of the target (the crate jflib names them all) and the
        styles the apps use -> the packages"""
        pkgs = ["jflib"] + ([f"jflib-style-{s}" for s in ("jfoffice", "backupcraft", "netlab")] if self.target != "web" else [])
        self.cargo(["cargo", "build", "--locked"] + [x for p in pkgs for x in ("-p", p)])
        return pkgs

    def cargo_rlibs(self, crates):
        """the rlibs of these library crates (and what they need) for a probe that rustc builds against
        them (tools/memory_safety_check.py) -> {crate: rlib}; probe_flags() then names their folder"""
        out, _ = self.cargo(["cargo", "build", "--locked"] + [x for c in crates for x in ("-p", f"jflib-{c.replace('_', '-')}")])
        rlibs = {}
        for ln in out.splitlines():
            m = json.loads(ln) if ln.startswith("{") else {}
            if m.get("reason") == "compiler-artifact" and "lib" in m["target"]["kind"]:
                rlib = next((Path(f) for f in m["filenames"] if f.endswith(".rlib")), None)
                if rlib is not None:                     # the packages asked for: copies in debug/, the rest in deps/
                    rlibs[m["target"]["name"]] = rlib
                    self.deps_dir = rlib.parent if rlib.parent.name == "deps" else rlib.parent / "deps"
        return {c: rlibs[c] for c in crates}

    def probe_flags(self):
        """rustc's flags for a probe against cargo_rlibs (tools/memory_safety_check.py)"""
        f = ["--edition", "2024", "--target", self.triple(), "-C", "panic=abort", "-C", "debuginfo=0", "-C", "opt-level=1",
             "-D", "warnings", f"--remap-path-prefix={ROOT}{os.sep}="]
        return f + (["-L", f"dependency={self.deps_dir}"] if self.deps_dir else [])

    def cargo_module(self, name):
        """jxl-module through Cargo: modules/NAME over modrt / modplat (--cfg jflib_module), fat LTO, no
        imports, no entry, into the Builder's folder, checked the way plat's loader checks a library
        -> its path"""
        t = self.target
        out = self.dir / module_file(name, t)
        mapf = out.parent / (out.stem + ".map" if t == "win" else out.name + ".map")
        flags = ["-D", "warnings", f"--remap-path-prefix={ROOT}{os.sep}=", "--cfg", "jflib_module"] + nolib_link_flags(t)
        flags += ["-C", f"link-arg={'/MAP:' if t == 'win' else '-Map='}{mapf}"] + (["-C", "relocation-model=pic"] if t == "linux" else [])
        profile = "RELEASE" if self.release else "DEV"
        env = {"CARGO_TARGET_DIR": str(self.out / "cargo-modules"), "CARGO_ENCODED_RUSTFLAGS": "\x1f".join(flags),
               f"CARGO_PROFILE_{profile}_LTO": "fat", f"CARGO_PROFILE_{profile}_STRIP": "symbols"}
        stdout, _ = self.cargo(["cargo", "build", "--locked"] + self.cargo_profile([f"jflib-module-{name}"]) + ["-p", f"jflib-module-{name}"],
                               env)
        lib = None
        for ln in stdout.splitlines():
            m = json.loads(ln) if ln.startswith("{") else {}
            if m.get("reason") == "compiler-artifact" and "cdylib" in m["target"]["kind"]:
                lib = next(Path(f) for f in m["filenames"] if f.endswith((".dll", ".so")))
        if lib is None:
            raise BuildError(f"cargo built no module {name}")
        shutil.copyfile(lib, out)
        shutil.copymode(lib, out)
        bad = lib_verify(out, t, export=f"{name}_module")
        if bad:
            out.unlink()
            raise BuildError(f"module {name} ({t}) would be rejected by the loader:\n  " + "\n  ".join(bad))
        return out

    def build_app(self, name, style, variant=False):
        """apps/NAME with STYLE -> <dir>/NAME[.exe] (the browser: <dir>/NAME/NAME.wasm, Android: the
        APK); variant (several styles in one run): <dir>/<style>/NAME[.exe], with a copy of the app's
        modules next to it.  build_apps has Cargo build them first, otherwise it happens here."""
        if self.target not in app_targets(name):
            return None
        own = self.cargo_builder(name)
        derived = variant or style != app_style(name)
        pkg = derived_name(name, self.target, style if derived else None) if derived or self.target in ("web", "android") else name
        if pkg not in own.cargo_out:
            self.base.cargo_build([name], style if derived else None)
        extra = app_styles(name)
        if self.target == "android":
            from tools.android import build_apk, build_exe
            archive = own.cargo_out[pkg][0]
            if "ui" not in app_crates(name):
                return build_exe(self, name, archive)
            return build_apk(self, name, style, variant, archive, app_notices([style] + [c[len("style_"):] for c in extra]))
        vdir = (self.web_public / style if variant else self.web_public) / name if self.target == "web" else \
            self.public / style if variant else self.public
        vdir.mkdir(parents=True, exist_ok=True)
        for s in sorted({style, *(c[len("style_"):] for c in extra)}):
            copy_style_notices(name, s, vdir)
        copy_library_notices(vdir)
        if self.target == "web":
            exe = own.cargo_copy(pkg, vdir / (name + ("." + self.web_mode if self.web_mode != "scalar" else "") + ".wasm"))
            web_assets(name, vdir)
            return exe
        exe = own.cargo_copy(pkg, vdir / bin_file(name, self.target))
        if self.target == "ios":                          # the program in NAME.app (tools/apple.py)
            from tools.apple import bundle
            bundle(exe, sim=self.ios_sim)
        if self.target == "mac":                          # NAME.app beside it (tools/apple.py mac_bundle)
            from tools.apple import mac_bundle
            mac_bundle(exe, name)
        if app_bins(name) and any(b not in self.cargo_builder(name, k).cargo_out for b, _, k in app_bins(name)):
            self.base.cargo_build([name])                 # a variant: the BINS stay in the app's own style
        for b, _, kind in app_bins(name):
            self.cargo_builder(name, kind).cargo_copy(b, vdir / bin_file(b, self.target))
        build_app_modules(self.base, name, vdir)          # jxl-module: its modules next to it
        return exe


def make_builders(targets, out, release, jobs, profile=None, web_mode="auto", android_abi="arm64-v8a", arch="x86_64",
                  ios_sim=False):
    """The CLI and Build Manager use the same browser variant selection."""
    return [Builder(target, out, release, jobs, profile, mode, android_abi, arch=arch, ios_sim=ios_sim)
            for target in targets
            for mode in ((["scalar", "simd", "threads"] if web_mode == "auto" else [web_mode])
                         if target == "web" else ["scalar"])]


def library_notices():
    """The library's third-party notices, shipped with every app: THIRD_PARTY_NOTICES.md and
    licenses/ in the repository's layout (the notice's relative paths stay valid), and
    lib/text_shape's notice with its crate licenses in licenses/text_shape/."""
    notices = {Path("THIRD_PARTY_NOTICES.md"): ROOT / "THIRD_PARTY_NOTICES.md"}
    notices.update({source.relative_to(ROOT): source
                    for source in sorted((ROOT / "licenses").rglob("*")) if source.is_file()})
    root = ROOT / "lib/text_shape"
    files = [root / "NOTICE.md", *sorted((root / "licenses").rglob("*"))]
    notices.update({Path("licenses/text_shape") / source.relative_to(root): source
                    for source in files if source.is_file()})
    return notices


# Apps that link in parallel share the library notices of their output folder.
NOTICE_LOCK = threading.Lock()


def copy_notice(source, destination):
    destination.parent.mkdir(parents=True, exist_ok=True)
    data = source.read_bytes()
    if not destination.exists() or destination.read_bytes() != data:
        destination.write_bytes(data)


def copy_library_notices(directory):
    with NOTICE_LOCK:
        for relative, source in library_notices().items():
            copy_notice(source, directory / relative)


EMBED = re.compile(r'include_bytes!\(\s*"([^"]+)"\s*\)')


def embedded_styles(style):
    """The styles whose fonts or icons STYLE embeds (include_bytes! of ../OTHER/...), transitively."""
    styles = ROOT / "styles"
    found, todo = set(), [style]
    while todo:
        current = todo.pop()
        for source in sorted((styles / current).rglob("*.rs")):
            for path in EMBED.findall(source.read_text(encoding="utf-8")):
                try:
                    other = (source.parent / path).resolve().relative_to(styles.resolve()).parts[0]
                except ValueError:
                    continue                      # outside styles/: covered by library_notices()
                if other != style and other not in found and (styles / other).is_dir():
                    found.add(other)
                    todo.append(other)
    return sorted(found)


def own_style_notices(style):
    root = ROOT / "styles" / style
    notices = {}
    for folder in (root, root / "assets"):
        for source in sorted(folder.glob("*")):
            if source.name in ("LICENSE-MIT", "LICENSE-APACHE"):     # the library's own, for the crate's package
                continue
            if source.is_file() and ("license" in source.name.lower()
                                     or source.name.upper().startswith(("OFL", "NOTICE", "COPYING"))):
                relative = Path("licenses") / style / source.name
                if relative in notices:
                    raise BuildError(f"{style}: duplicate notice filename {source.name}")
                notices[relative] = source
    return notices


def style_notices(style):
    """Redistributable notices supplied by a style, independent of the target: its own in
    licenses/STYLE/ and those of the styles whose files it embeds in licenses/OTHER/."""
    notices = own_style_notices(style)
    for other in embedded_styles(style):
        notices.update(own_style_notices(other))
    return notices


def app_notices(styles):
    """Everything an app built with these styles ships: library_notices() and style_notices()."""
    notices = library_notices()
    for style in styles:
        notices.update(style_notices(style))
    return notices


def copy_style_notices(app, style, directory):
    # Per-app directories avoid collisions when several apps link in parallel.
    for relative, source in style_notices(style).items():
        copy_notice(source, directory / "licenses" / app / relative.relative_to("licenses"))


WEB_ASSET_LOCK = threading.Lock()


def web_assets(name, directory):
    # Several instruction-set variants finish in parallel and share their host files.
    with WEB_ASSET_LOCK:
        _web_assets(name, directory)


def _web_assets(name, directory):
    """Shared browser host, emitted beside each app; the GUI stays entirely in WASM."""
    source = ROOT / "web"
    for filename in ("loader.js", "worker.js", "gpu.js", "bridge.js", "storage.js", "secrets.js", "handles.js", "a11y.js", "audio.js", "system.js", "notify.js", "sensors.js", "host.css"):
        shutil.copyfile(source / filename, directory / filename)
    html = (source / "index.html").read_text(encoding="utf-8").replace("{{APP}}", name)
    (directory / "index.html").write_bytes(html.encode("utf-8"))
    variants = {mode: name + suffix for mode, suffix in (("scalar", ".wasm"), ("simd", ".simd.wasm"), ("threads", ".threads.wasm"))
                if (directory / (name + suffix)).exists()}
    (directory / "variants.json").write_text(json.dumps(variants), encoding="utf-8")


# ---------------------------------------------------------------- the apps as libraries, other styles
# The browser and Android load an app as a library, Cargo builds a library only from a [lib]; Cargo
# cannot swap a dependency either, which a style variant needs.  derived_workspace makes such packages
# in a workspace of the tool's own below <out>; the apps themselves stay programs in their own style.
DERIVED_LOCK = threading.Lock()


def package_of(package_id):
    """the package's name in Cargo's id: path+file:///.../DIR#NAME@VERSION, or #VERSION alone when the
    folder has the package's name"""
    path, _, fragment = package_id.partition("#")
    return fragment.split("@")[0] if "@" in fragment else path.rstrip("/").split("/")[-1]


def derived_name(app, target, style=None):
    """the package derived_workspace makes of an app: a library for the browser and Android, a
    program or library in another style"""
    return f"{app}-{target}" + (f"-{style}" if style else "")


def toml_value(v):
    """a value as TOML (strings with a double quote or a backslash as literal strings)"""
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (int, float)):
        return str(v)
    if isinstance(v, str):
        return f"'{v}'" if '"' in v or "\\" in v else f'"{v}"'
    if isinstance(v, list):
        return "[" + ", ".join(toml_value(x) for x in v) + "]"
    return "{ " + ", ".join(f"{k} = {toml_value(x)}" for k, x in v.items()) + " }"


def write_changed(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    if not path.exists() or path.read_text(encoding="utf-8") != text:
        path.write_text(text, encoding="utf-8", newline="\n")


def derived_profiles(members):
    """the root manifest's profiles for derived_workspace, where the library's crates are no members:
    its "*" (the crates from crates.io) named one by one, an app by the packages made of it
    (members: {package: app})"""
    registry = sorted({p["name"] for p in manifests.read(ROOT / "Cargo.lock")["package"] if "source" in p})
    apps_ = set(members.values())
    out = []
    for name, prof in manifests.read(ROOT / "Cargo.toml")["profile"].items():
        out.append(f"[profile.{name}]\n" + "".join(f"{k} = {toml_value(v)}\n" for k, v in prof.items() if k != "package"))
        for pkg, over in prof.get("package", {}).items():
            names = registry if pkg == "*" else sorted(m for m, a in members.items() if a == pkg) if pkg in apps_ else [pkg]
            for p in names:
                out.append(f'[profile.{name}.package."{p}"]\n' + "".join(f"{k} = {toml_value(v)}\n" for k, v in over.items()))
    return "\n".join(out)


def derived_workspace(out, target, names, style=None):
    """a workspace of the tool's own in <out>/cargo-<target>-ws with a package per app: for the browser
    and Android its main.rs as [lib] (cdylib, staticlib), else as a program; its manifest's
    dependencies, `style` the one given; a build.rs that reads the app's files (jflib-build's
    part_of); the root manifest's profiles and lints, Cargo.lock's versions -> its Cargo.toml"""
    ws = out / f"cargo-{target}-ws"
    root = manifests.workspace()
    lib = target in ("web", "android")
    lints = "".join(f"[lints.{tool_}]\n" + "".join(f"{k} = {toml_value(v)}\n" for k, v in table.items())
                    for tool_, table in root.get("lints", {}).items())
    with DERIVED_LOCK:
        index = ws / "members.json"
        members = json.loads(index.read_text(encoding="utf-8")) if index.exists() else {}
        for n in names:
            man = manifests.read(ROOT / "apps" / n / "Cargo.toml")
            deps = man.get("dependencies", {})
            if style is not None and "style" in deps and f"style_{style}" in deps:
                raise BuildError(f"apps/{n} names style_{style} itself: a variant in {style} would name it twice (Cargo refuses that)")
            lines = []
            for key, spec in deps.items():
                s = dict(root["dependencies"][key]) if spec.get("workspace") else dict(spec)
                if "path" in s:
                    s["path"] = ((ROOT if spec.get("workspace") else ROOT / "apps" / n) / s["path"]).resolve().as_posix()
                s.pop("version", None)
                if key == "style" and style is not None:
                    s = {"package": f"jflib-style-{style}", "path": (ROOT / "styles" / style).as_posix()}
                lines.append(f"{key} = {toml_value(s)}\n")
            pkg = derived_name(n, target, style)
            entry = (ROOT / "apps" / n / man["bin"][0]["path"]).resolve().as_posix()
            # named after the package: Cargo puts the programs of all packages into one folder, two of
            # the same name (an app in two styles) would overwrite each other there
            unit = (f'[lib]\nname = "{pkg.replace("-", "_").lower()}"\npath = "{entry}"\ncrate-type = ["{"cdylib" if target == "web" else "staticlib"}"]\n'
                    "test = false\ndoctest = false\n" if lib else f'[[bin]]\nname = "{pkg}"\npath = "{entry}"\ntest = false\nbench = false\n')
            write_changed(ws / pkg / "Cargo.toml",
                          f'[package]\nname = "{pkg}"\nversion = "{man["package"]["version"]}"\nedition = "2024"\npublish = false\n'
                          "autobins = false\nautoexamples = false\nautotests = false\nautobenches = false\n\n" + unit + "\n"
                          + lints + "\n[dependencies]\n" + "".join(lines)
                          + f'\n[build-dependencies]\njflib-build = {{ path = "{(ROOT / "lib" / "jflib_build").as_posix()}" }}\n')
            write_changed(ws / pkg / "build.rs", "// made by build_rs.py (derived_workspace): the app's files and link arguments\n"
                          f'fn main() {{\n    jflib_build::Build::app().part_of("{n}", "{(ROOT / "apps" / n).as_posix()}").run();\n}}\n')
            members[pkg] = n
        write_changed(index, json.dumps(members, indent=1, sort_keys=True) + "\n")
        write_changed(ws / "Cargo.toml", "# made by build_rs.py (derived_workspace): the apps as libraries, other styles\n[workspace]\n"
                      'resolver = "3"\nmembers = [\n' + "".join(f'    "{m}",\n' for m in sorted(members)) + "]\n\n" + derived_profiles(members))
        if not (ws / "Cargo.lock").exists():
            shutil.copyfile(ROOT / "Cargo.lock", ws / "Cargo.lock")
    return ws / "Cargo.toml"


def build_apps(builders, names, styles=None):
    """build apps for every target: one Cargo run per target for the apps in their own style, one per
    further style (derived_workspace), the modules alongside; then every program in its place ->
    [(builder, name, exe or None)] in the order builders x names x styles.  styles: None = each app's
    STYLE; several = one variant per style (Builder.build_app).  The first failure is raised after
    the running builds finished."""
    jobs = [(b, n, s) for b in builders for n in names for s in (styles or [app_style(n)])]
    variant = len(styles or []) > 1
    runs = []
    for b in builders:
        mine = [(n, s) for bb, n, s in jobs if bb is b and b.target in app_targets(n)]
        runs.append((b, sorted({n for n, s in mine if not variant and s == app_style(n)}), None))
        for s in sorted({s for n, s in mine if variant or s != app_style(n)}):
            runs.append((b, sorted({n for n, ss in mine if ss == s and (variant or ss != app_style(n))}), s))
    errors = []
    with ThreadPoolExecutor(max(1, len(runs) + len(builders) * len(names))) as ex:
        futs = [ex.submit(b.cargo_build, ns, s) for b, ns, s in runs if ns]
        futs += [ex.submit(build_app_modules, b, n) for b in builders for n in names
                 if b.target not in ("web", "android", *APPLE) and b.target in app_targets(n) and APP_MODULES.get(n)]
        for f in futs:
            try:
                f.result()
            except BaseException as e:      # noqa: BLE001 -- raised once every build finished
                errors.append(e)
    if errors:
        raise errors[0]
    with ThreadPoolExecutor(max(1, len(jobs))) as ex:
        futs = [(b, n, ex.submit(b.build_app, n, s, variant)) for b, n, s in jobs]
        return [(b, n, f.result()) for b, n, f in futs]


# ---------------------------------------------------------------- run / test
def wsl_path(p):
    p = str(Path(p).resolve())
    return "/mnt/" + p[0].lower() + p[2:].replace("\\", "/") if p[1:2] == ":" else p


def run_exe(target, exe, args, timeout=600):
    name = Path(exe).name
    gui = target == "mac" and name in apps() and "ui" in app_crates(name)   # windows (tools/apple.py gui_run)
    return run_native(target, exe, args, timeout, gui=gui)


def script_args(uis):
    first = uis.read_text(encoding="utf-8").splitlines()[:1]
    return shlex.split(first[0][len("#!args"):]) if first and first[0].startswith("#!args") else []


def pending_scripts(name):
    """scripts apps/NAME/parity.txt marks as not supported yet ("pending FILE.uis WHY")"""
    f = RS / "apps" / name / "parity.txt"
    if not f.exists():
        return set()
    return {p[1] for p in (ln.split("#")[0].split() for ln in f.read_text(encoding="utf-8").splitlines())
            if len(p) >= 2 and p[0] == "pending"}


def run_app_tests(name, target, exe, verify_dirty=False, report=None):
    """Run one app's existing scripts on an already-built binary; return structured results."""
    records = []
    pending = pending_scripts(name)
    for script in sorted((ROOT / "apps" / name / "tests").glob("*.uis")):
        if script.name in pending:
            record = dict(script=script.name, status="skipped", reason="pending")
        else:
            for line in script.read_text(encoding="utf-8").splitlines():
                if line.startswith(("shot ", "dump ")) and len(line.split()) > 1:
                    (ROOT / line.split()[1]).parent.mkdir(parents=True, exist_ok=True)
            args = script_args(script) + ["--script", script.relative_to(ROOT).as_posix()]
            if verify_dirty:
                args.append("--verify-dirty")
            started = time.monotonic()
            result = run_exe(target, exe, args)
            record = dict(script=script.name, status="passed" if result.returncode == 0 else "failed",
                          exit_code=result.returncode, duration=time.monotonic() - started,
                          output=result.stdout + result.stderr)
        records.append(record)
        if report:
            report(record)
    return records


def run_tests(builders, out, quick):
    fails = []
    total = 0
    skipped = 0
    exes = {(b.target, n): exe for b, n, exe in build_apps(builders, apps())}
    for b in builders:
        for name in apps():
            exe = exes[(b.target, name)]
            if exe is None:
                continue
            for record in run_app_tests(name, b.target, exe):
                if record['status'] == 'skipped':
                    skipped += 1
                    continue
                total += 1
                tag = f"[{b.target}] {name}/{record['script']}"
                if record['status'] == 'failed':
                    fails.append(tag)
                    print(f"FAIL {tag} (exit {record['exit_code']})")
                    for ln in record['output'].splitlines():
                        if ln.startswith(("FAIL", "error", "panic", "UIWARN")):
                            print("   " + ln)
    print(f"{total - len(fails)} of {total} scripts passed" + (f" ({skipped} pending)" if skipped else ""))
    # what only runs on a device (G4c): the serial of a device or emulator in jfgui_TEST_ANDROID, its builds
    # with `app androidGallery|gputest --target android`; nobody's phone is touched without it
    serial = os.environ.get("jfgui_TEST_ANDROID")
    if serial:
        for script in ("apps/gputest/tests/android.py", "apps/androidGallery/tests/android_gpu.py"):
            r = subprocess.run([sys.executable, str(RS / script), "--serial", serial], cwd=ROOT, capture_output=True, text=True,
                               encoding="utf-8", errors="replace", stdin=subprocess.DEVNULL)
            total += 1
            if r.returncode:
                fails.append(script)
                print(f"FAIL {script}\n" + (r.stdout + r.stderr).strip())
    else:
        print("note: the GPU tests on Android need jfgui_TEST_ANDROID=SERIAL (apps/gputest/tests/android.py, apps/androidGallery/tests/android_gpu.py)")
    # docs/API.md generated from the Rust doc comments must be current (R13); --api: the API
    # changes since the last release, for information (docs/VERSIONING.md)
    r = subprocess.run([sys.executable, str(RS / "tools" / "gendoc.py"), "--check", "--api"], cwd=ROOT,
                       capture_output=True, text=True, encoding="utf-8", errors="replace")
    if r.returncode:
        fails.append("docs/API.md is stale (python tools/gendoc.py)")
        print("FAIL docs/API.md is stale: run python tools/gendoc.py")
    print("".join(ln for ln in r.stdout.splitlines(True) if ln.startswith("API")).strip())
    # the project generator, the API diff and the changelog helper (tools/tests/test_devkit.py)
    r = subprocess.run([sys.executable, str(RS / "tools" / "tests" / "test_devkit.py")], cwd=ROOT,
                       capture_output=True, text=True, encoding="utf-8", errors="replace")
    if r.returncode:
        fails.append("tools/tests/test_devkit.py")
        print("FAIL tools/tests/test_devkit.py\n" + (r.stdout + r.stderr).strip())
    # hot reload's watcher with a stand-in program (the replay with a real app: without --quick, by hand)
    r = subprocess.run([sys.executable, str(RS / "tools" / "tests" / "test_hotreload.py"), "--quick"], cwd=ROOT,
                       capture_output=True, text=True, encoding="utf-8", errors="replace", stdin=subprocess.DEVNULL)
    if r.returncode:
        fails.append("tools/tests/test_hotreload.py --quick")
        print("FAIL tools/tests/test_hotreload.py --quick\n" + (r.stdout + r.stderr).strip())
    # the package formats (docs/PACKAGING.md "Checks"): manifests, validators, the AppImage runtime
    rt = exes.get(("linux", "appimagert"))
    r = subprocess.run([sys.executable, str(RS / "tools" / "pkg_check.py"), "--quick", "--summary"]
                       + (["--runtime", str(rt)] if rt else []), cwd=ROOT, capture_output=True, text=True,
                       encoding="utf-8", errors="replace", stdin=subprocess.DEVNULL)
    print(r.stdout.strip() + (("\n" + r.stderr.strip()) if r.returncode and r.stderr.strip() else ""))
    if r.returncode:
        fails.append("tools/pkg_check.py --quick")
    # the catalogues (tools/i18n_check.py --check): errors only (placeholders, plural forms,
    # broken rows; the apps whose catalogues are complete stay so); the other gaps are reported
    r = subprocess.run([sys.executable, str(RS / "tools" / "i18n_check.py"), "--check"], cwd=ROOT, capture_output=True,
                       text=True, encoding="utf-8", errors="replace", stdin=subprocess.DEVNULL)
    print((r.stdout.strip() + ("\n" + r.stderr.strip() if r.stderr.strip() else "")).strip())
    if r.returncode:
        fails.append("tools/i18n_check.py --check")
    # the website's docs (tools/site.py --check): every page, internal link, anchor and image, the HTML, two
    # builds byte-identical; a temporary folder below build/
    r = subprocess.run([sys.executable, str(RS / "tools" / "site.py"), "--check", "--summary"], cwd=ROOT,
                       capture_output=True, text=True, encoding="utf-8", errors="replace", stdin=subprocess.DEVNULL)
    print(r.stdout.strip() + (("\n" + r.stderr.strip()) if r.returncode and r.stderr.strip() else ""))
    if r.returncode:
        fails.append("tools/site.py --check")
    if not quick:
        # docs/CRATES.md: a project outside the repository builds from the packed crates alone (Windows,
        # Linux, the browser, an APK when the Android SDK is there), its smoke test passes; about 15
        # minutes from nothing
        targets = sorted({b.target for b in builders if b.target in ("win", "linux")} | {"web", "android", "package", "watch"})
        r = subprocess.run([sys.executable, str(RS / "tools" / "crates_accept.py"), "--targets", ",".join(targets)], cwd=ROOT,
                           capture_output=True, text=True, encoding="utf-8", errors="replace", stdin=subprocess.DEVNULL)
        print("".join(ln for ln in r.stdout.splitlines(True) if ln.startswith(("FAIL", "    ", "note", "crates"))).strip())
        if r.returncode:
            fails.append("tools/crates_accept.py")
        # parity with the assembly reference (apps that exist in both)
        tgt = "both" if len(builders) == 2 else builders[0].target
        for name in apps():
            if not (ROOT.parent / "legacy" / "apps" / name / "src").is_dir():
                continue
            r = subprocess.run([sys.executable, str(RS / "tools" / "parity.py"), "app", name, "--target", tgt],
                               cwd=ROOT, capture_output=True, text=True, encoding="utf-8", errors="replace")
            print(r.stdout.strip())
            if r.returncode:
                fails.append(f"parity {name}")
    return not fails


def run_app_suite(builders, name):
    """test NAME: build one app (with its BINS) and run its tests/*.uis on every target"""
    if name not in apps():
        raise BuildError(f"no app {name} (apps/{name}/src/main.rs or main.rs)")
    fails, total = [], 0
    for b, n, exe in build_apps(builders, [name]):
        if exe is None:
            print(f"[{b.target}] {name}: not built for this target (apps/{name}/TARGETS)")
            continue
        for record in run_app_tests(name, b.target, exe):
            if record["status"] == "skipped":
                continue
            total += 1
            tag = f"[{b.target}] {name}/{record['script']}"
            if record["status"] == "failed":
                fails.append(tag)
                print(f"FAIL {tag} (exit {record['exit_code']})")
                for ln in record["output"].splitlines():
                    if ln.startswith(("FAIL", "error", "panic", "UIWARN")):
                        print("   " + ln)
            else:
                print(f"ok   {tag} ({record['duration']:.1f} s)")
    print(f"{total - len(fails)} of {total} scripts passed")
    return not fails


# ---------------------------------------------------------------- new, api-diff, changelog, release
def dev_command(a):
    """the commands that need no build: new (tools/scaffold.py), api-diff (tools/apidiff.py),
    changelog and release (tools/changelog.py) -> exit code"""
    tools = str(ROOT / "tools")
    if tools not in sys.path:
        sys.path.insert(0, tools)
    if a.cmd == "new":
        import scaffold
        try:
            if a.lib:
                deps = [d for d in (a.deps or "").replace(",", " ").split() if d]
                paths = scaffold.create_lib(ROOT, a.name, deps, a.title, list(LIBS), list(crate_table()))
            else:
                paths = scaffold.create_app(ROOT, a.name, a.kind or "window", a.style or "winui", a.title,
                                            list(crate_table()), STYLES)
        except scaffold.ScaffoldError as e:
            print(f"new: {e}")
            return 1
        for p in paths:
            print(f"wrote {p.relative_to(ROOT).as_posix()}")
        r = subprocess.run(["cargo", "metadata", "--offline", "--format-version", "1"], cwd=ROOT, capture_output=True,   # Cargo.lock
                           text=True, encoding="utf-8", errors="replace")
        if r.returncode:
            print(f"new: Cargo.lock was not updated:\n{r.stderr.strip()}")
            return 1
        if a.lib:
            r = subprocess.run([sys.executable, str(ROOT / "tools" / "gendoc.py")], cwd=ROOT, capture_output=True,
                               text=True, encoding="utf-8", errors="replace")
            print(r.stdout.strip())
            print(f"next: name it in an app ({a.name}::hello), python build_rs.py lib --target win")
        else:
            print(f"next: python build_rs.py test {a.name}     (build it, run tests/smoke.uis)\n"
                  f"      python build_rs.py run {a.name}")
        return 0
    if a.cmd == "api-diff":
        import apidiff
        return apidiff.main((["--against", a.against] if a.against else []) + (["--snapshot"] if a.snapshot else [])
                            + (["--all"] if a.all else []))
    import changelog
    try:
        if a.cmd == "changelog":
            return changelog.main(([a.name] if a.name else []) + ["--type", a.type] + (["--area", a.area] if a.area else []))
        if not a.name:
            raise changelog.ChangelogError("release: the version is missing (python build_rs.py release 0.2.0)")
        print("\n".join(changelog.release(a.name, a.date, a.force)))
        return 0
    except changelog.ChangelogError as e:
        print(f"{a.cmd}: {e}")
        return 1


# ---------------------------------------------------------------- loader checks
# A library built to be loaded by plat's own loader (lib/plat/win_lib.rs, linux_elfload.rs): a
# module of jxl-module, linked against nothing -- Windows: lld-link /NOENTRY /NODEFAULTLIB (no
# imports, one export); Linux: ld.lld -shared -Bsymbolic --no-undefined (no DT_NEEDED, only
# RELATIVE relocations).  lib_verify checks the result the way the loader does: a library it
# would reject fails the build.  (PaintCraft's Rust plugins were built the same way until the
# app moved to its own repository, JFLibrary/JFPainter, in 2026-11.)
def nolib_link_flags(target):
    """rustc's flags for such a library: no entry, no imports"""
    if target == "win":
        args = ["/NOENTRY", "/NODEFAULTLIB", "/DEBUG:NONE", "/Brepro"]
        return ["-C", f"linker={tool('lld-link')}"] + [x for a in args for x in ("-C", "link-arg=" + a)]
    args = ["-Bsymbolic", "--no-undefined", "--hash-style=both", "-z", "noexecstack", "-z", "now", "--build-id=none"]
    return ["-C", f"linker={tool('ld.lld')}", "-C", "linker-flavor=ld"] + [x for a in args for x in ("-C", "link-arg=" + a)]


def lib_verify(path, target, export):
    """the checks of plat's loader -> problems (`export`: the one export expected, jxlenc_module
    for the JPEG XL module)"""
    b = path.read_bytes()
    if len(b) > 64 << 20:
        return ["larger than LIB_FILE_MAX (64 MB)"]
    return pe_verify(b, export) if target == "win" else elf_verify(b, export)


def _u16(b, o):
    return int.from_bytes(b[o:o + 2], "little")


def _u32(b, o):
    return int.from_bytes(b[o:o + 4], "little")


def _u64(b, o):
    return int.from_bytes(b[o:o + 8], "little")


def pe_verify(b, export):
    """win_lib.rs pe_check + lib_sym: PE32+ x86-64 DLL, no imports / delay imports / TLS, no entry
    point, the section table inside the first 4 KB of headers, <= 15 sections, no W+X section,
    one export: `export` in an executable section"""
    if len(b) < 512 or b[:2] != b"MZ":
        return ["not a PE file"]
    bad = []
    nt = _u32(b, 0x3C)
    if not 64 <= nt <= 4096 - 24 - 240:
        return ["e_lfanew outside the first page"]
    if b[nt:nt + 4] != b"PE\0\0" or _u16(b, nt + 4) != 0x8664 or not _u16(b, nt + 22) & 0x2000:
        return ["not an x86-64 DLL"]
    nsec, ohsize, oh = _u16(b, nt + 6), _u16(b, nt + 20), nt + 24
    if _u16(b, oh) != 0x20B or ohsize < 112 + 16 * 8:
        return ["not PE32+"]
    span, hdrs, nrva = _u32(b, oh + 56), _u32(b, oh + 60), _u32(b, oh + 108)
    if not 4096 <= span <= 256 << 20:
        bad.append(f"SizeOfImage {span}")
    if nrva < 14:
        bad.append("fewer than 14 data directories")
    dirs = [(_u32(b, oh + 112 + 8 * i), _u32(b, oh + 116 + 8 * i)) for i in range(16)]
    if dirs[1][1] or dirs[13][1]:
        bad.append("imports other libraries (import / delay-import directory)")
    if dirs[9][1]:
        bad.append("has a TLS directory")
    if _u32(b, oh + 16):
        bad.append("has an entry point (DllMain)")
    sec0 = oh + ohsize
    if sec0 + nsec * 40 > min(hdrs, 4096) or nsec > 15:
        bad.append(f"section table: {nsec} sections")
        return bad
    secs = []
    for k in range(nsec):
        s = sec0 + k * 40
        name = b[s:s + 8].rstrip(b"\0").decode("ascii", "replace")
        vs, va, rs, rp, ch = _u32(b, s + 8), _u32(b, s + 12), _u32(b, s + 16), _u32(b, s + 20), _u32(b, s + 36)
        if ch & 0x20000000 and ch & 0x80000000:
            bad.append(f"section {name} is writable and executable")
        secs.append((va, vs if vs else rs, rp, rs, ch))

    def off(rva, n=1):
        for va, vs, rp, rs, ch in secs:
            if va <= rva and rva + n <= va + min(vs, rs):
                return rp + rva - va
        return None

    def in_exec(rva):
        return any(va <= rva < va + vs and ch & 0x20000000 for va, vs, rp, rs, ch in secs)
    erva, esize = dirs[0]
    eo = off(erva, 40) if esize else None
    if eo is None:
        bad.append("no export directory")
        return bad
    nfun, nnames, afun, anames, aord = (_u32(b, eo + 20), _u32(b, eo + 24), _u32(b, eo + 28),
                                        _u32(b, eo + 32), _u32(b, eo + 36))
    names = []
    for i in range(min(nnames, 1024)):
        no = off(anames + 4 * i, 4)
        po = off(_u32(b, no), 1) if no is not None else None
        oo = off(aord + 2 * i, 2)
        if po is None or oo is None:
            bad.append("export name table outside the image")
            return bad
        nm = b[po:b.index(b"\0", po)].decode("utf-8", "replace")
        ordinal = _u16(b, oo)
        fo = off(afun + 4 * ordinal, 4) if ordinal < nfun else None
        frva = _u32(b, fo) if fo is not None else 0
        names.append(nm)
        if nm == export and not in_exec(frva):
            bad.append(f"{export} is not in an executable section")
    if names != [export] or nfun != 1:
        bad.append(f"exports {names} ({nfun} functions), expected only {export}")
    return bad


def elf_verify(b, export):
    """linux_elfload.rs elf_map / elf_load / elf_rela / lib_sym: ELF64 LE ET_DYN x86-64, 1..64
    program headers, <= 16 PT_LOAD ascending and page-disjoint, never W+X, inside 256 MB, one
    PT_DYNAMIC, no PT_INTERP / PT_TLS, no DT_NEEDED / DT_INIT / init arrays / DT_REL, RELA
    relocations NONE / RELATIVE / 64 / GLOB_DAT / JUMP_SLOT against defined symbols, the one
    defined dynamic symbol `export` in an executable segment"""
    if len(b) < 64 or b[:4] != b"\x7fELF" or b[4] != 2 or b[5] != 1 or b[6] != 1:
        return ["not an ELF64 little-endian file"]
    if _u16(b, 16) != 3 or _u16(b, 18) != 62 or _u32(b, 20) != 1 or _u16(b, 54) != 56:
        return ["not an x86-64 shared object (ET_DYN)"]
    bad = []
    phnum, phoff = _u16(b, 56), _u64(b, 32)
    if not 1 <= phnum <= 64 or phoff + phnum * 56 > len(b):
        return ["program header table"]
    loads, dyn, span = [], None, 0
    for k in range(phnum):
        ph = phoff + k * 56
        typ, fl, o, va, fsz, msz = (_u32(b, ph), _u32(b, ph + 4), _u64(b, ph + 8), _u64(b, ph + 16),
                                    _u64(b, ph + 32), _u64(b, ph + 40))
        if typ == 1 and msz:
            if fsz > msz or o + fsz > len(b) or va + msz > 256 << 20:
                bad.append(f"PT_LOAD {k}: sizes / offsets")
            if fl & 3 == 3:
                bad.append(f"PT_LOAD {k}: writable and executable")
            if va & ~4095 < span:
                bad.append(f"PT_LOAD {k}: overlaps the pages of the previous segment")
            loads.append((va, va + msz, o, fsz, fl))
            span = (va + msz + 4095) & ~4095
        elif typ == 2:
            if dyn is not None:
                bad.append("two PT_DYNAMIC")
            dyn = (o, fsz)
        elif typ in (3, 7):
            bad.append("PT_INTERP / PT_TLS")
    if not loads or len(loads) > 16 or dyn is None:
        return bad + [f"{len(loads)} PT_LOAD, PT_DYNAMIC {'missing' if dyn is None else 'ok'}"]

    def off(va, n=1):
        for s, e, o, fsz, fl in loads:
            if s <= va and va + n <= s + fsz:
                return o + va - s
        return None
    tags = {}
    for k in range(dyn[1] // 16):
        tag, val = _u64(b, dyn[0] + 16 * k), _u64(b, dyn[0] + 16 * k + 8)
        if tag == 0:
            break
        if tag == 1:
            bad.append("DT_NEEDED (needs another library)")
        if tag == 12 or (tag in (27, 33) and val):
            bad.append("DT_INIT / init array (code that runs at load)")
        if tag in (17, 18) and val:
            bad.append("DT_REL relocations")
        tags.setdefault(tag, val)
    if tags.get(11, 24) != 24 or tags.get(9, 24) != 24:
        bad.append("DT_SYMENT / DT_RELAENT")
    if tags.get(2) and tags.get(20) != 7:
        bad.append("DT_PLTREL is not RELA")
    so, sto = off(tags.get(6, 0), 24), off(tags.get(5, 0), tags.get(10, 1))
    if 4 not in tags or so is None or sto is None:
        return bad + ["no DT_HASH / .dynsym / .dynstr"]
    ho = off(tags[4], 8)
    nsym = _u32(b, ho + 4) if ho is not None else 0
    syms = []
    for i in range(nsym):
        s = so + 24 * i
        nm = b[sto + _u32(b, s):b.index(b"\0", sto + _u32(b, s))].decode("utf-8", "replace")
        syms.append((nm, _u16(b, s + 6), _u64(b, s + 8)))
    for rva, rsz in ((tags.get(7, 0), tags.get(8, 0)), (tags.get(23, 0), tags.get(2, 0))):
        if not rsz:
            continue
        ro = off(rva, rsz)
        if ro is None or rsz % 24:
            bad.append("relocation table outside the file")
            continue
        for k in range(rsz // 24):
            info = _u64(b, ro + 24 * k + 8)
            typ, sym = info & 0xFFFFFFFF, info >> 32
            if typ not in (0, 1, 6, 7, 8):
                bad.append(f"relocation type {typ}")
            elif typ in (1, 6, 7) and (sym == 0 or sym >= nsym or syms[sym][1] == 0):
                bad.append(f"relocation against an undefined symbol ({syms[sym][0] if 0 < sym < nsym else sym})")
    defined = [(nm, v) for nm, shndx, v in syms[1:] if shndx and nm]
    if [nm for nm, v in defined] != [export]:
        bad.append(f"exports {[nm for nm, v in defined]}, expected only {export}")
    elif not any(s <= defined[0][1] < e and fl & 1 for s, e, o, fsz, fl in loads):
        bad.append(f"{export} is not in an executable segment")
    return sorted(set(bad), key=bad.index)


# ---------------------------------------------------------------- end loader checks


# ---------------------------------------------------------------- jxl-module
# Loadable modules (rewrite/AGENTS.md "JPEG XL encoder as a loadable module"; the ABI in
# lib/plat/modabi.rs): modules/NAME -> <app folder>/NAME.dll | NAME.so, built with every
# app that loads it (APP_MODULES: codectest --jxlenc FILE loads jxlenc, the JPEG XL writer), in the Builder's
# folder with its flags (dev, --release, --profile).  Cargo builds the package jflib-module-NAME
# with --cfg jflib_module into <out>/cargo-modules: the library crates it contains then name
# modules/modrt as `rt` and modules/modplat as `plat` (the module runtime: the host's allocator,
# worker pool and crash report through a function table); the cdylib links as section "loader checks" says: panic=abort, fat LTO, linked against nothing (lld-link
# /NOENTRY /NODEFAULTLIB; ld.lld -shared -Bsymbolic --no-undefined), a .map next to it, and
# checked the way plat's loader checks a library (lib_verify with the export
# NAME_module).  Lint: #![no_std]; modrt / modplat may contain unsafe (the module's rt / plat),
# a module crate #![deny(unsafe_code)] with module_entry!() as its one exception.
MODULES_RS = RS / "modules"
MOD_RT = {"rt": "modrt", "plat": "modplat"}         # the crates a module gets as rt / plat
APP_MODULES = {"codectest": ["jxlenc"]}               # the modules an app's folder gets


def module_file(name, target):
    return name + (".dll" if target == "win" else ".so")


def module_lint():
    """lint() for modules/*: #![no_std]; modules #![deny(unsafe_code)]; alloc / core only"""
    bad = []
    if not MODULES_RS.is_dir():
        return bad
    shared = MODULES_RS / "modrt" / "shared"             # copies of files of lib/ (docs/CRATES.md)
    for copy in sorted(p for p in shared.rglob("*") if p.is_file()):
        source = ROOT / "lib" / copy.relative_to(shared)
        if not source.is_file() or source.read_bytes() != copy.read_bytes():
            bad.append(f"{copy.relative_to(ROOT)}: differs from {source.relative_to(ROOT)}: copy that file over it")
    for d in sorted(p for p in MODULES_RS.iterdir() if p.is_dir() and entrypoint(p, "lib", False)):
        root = entrypoint(d, "lib")
        head = root.read_text(encoding="utf-8")
        if "#![no_std]" not in head:
            bad.append(f"{root.relative_to(ROOT)}: missing #![no_std]")
        if d.name not in MOD_RT.values() and "#![deny(unsafe_code)]" not in head and "#![forbid(unsafe_code)]" not in head:
            bad.append(f"{root.relative_to(ROOT)}: missing #![deny(unsafe_code)]")
        for f in project_sources(root):
            text = f.read_text(encoding="utf-8")
            bad += kernel_lint(f, text)
            for i, ln in enumerate(text.splitlines(), 1):
                s = ln.strip()
                if s.startswith("extern crate") and s not in ("extern crate alloc;", "extern crate core;"):
                    bad.append(f"{f.relative_to(ROOT)}:{i}: only alloc / core may be extern crates")
                if s.startswith("use std") or " std::" in s.split("//")[0]:
                    bad.append(f"{f.relative_to(ROOT)}:{i}: std is not available (no_std)")
    return bad


def build_app_modules(b, app, vdir=None):
    """the modules of APP_MODULES[app] into the app's folder (Builder b; once per run, also when several
    style variants link at once), copied next to a variant's exe (vdir); the browser and Android load
    none (lib/plat/web_lib.rs), Apple none yet (docs/APPLE.md, decision 6)"""
    if b.target in ("web", "android", *APPLE):
        return
    with b.mod_lock:
        if app not in b.mod_ready:
            for m in APP_MODULES.get(app, []):
                b.cargo_module(m)
            b.mod_ready.add(app)
    if vdir is not None and vdir != b.dir:
        for m in APP_MODULES.get(app, []):
            shutil.copyfile(b.dir / module_file(m, b.target), vdir / module_file(m, b.target))
# ---------------------------------------------------------------- end jxl-module




def check_toolchain():
    v = subprocess.run(["rustc", "--version"], capture_output=True, text=True).stdout.split()
    if len(v) < 2 or not v[1].startswith(RUSTC_VERSION):
        print(f"warning: rustc {v[1] if len(v) > 1 else '?'}, the code is written for {RUSTC_VERSION}")


def main():
    for s in (sys.stdout, sys.stderr):      # redirected output on Windows would be cp1252
        try:
            s.reconfigure(encoding="utf-8", errors="replace")
        except (AttributeError, ValueError):
            pass
    argv = sys.argv[1:]
    extra = []
    if "--" in argv:
        i = argv.index("--")
        argv, extra = argv[:i], argv[i + 1:]
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", choices=["lib", "app", "all", "run", "test", "check",
                                    "new", "api-diff", "changelog", "release", "site", "i18n", "watch"])
    ap.add_argument("name", nargs="?")
    ap.add_argument("--target", choices=["win", "linux", "both", "web", "all", "android", "mac", "ios"])
    ap.add_argument("--sim", action="store_true", help="ios: the simulator's build (ios-sim; docs/APPLE.md)")
    ap.add_argument("--android-abi", choices=["arm64-v8a", "x86_64"], default="arm64-v8a",
                    help="Android native ABI (default ARM64; x86_64 for emulator tests)")
    ap.add_argument("--arch", choices=ARCHS, default="x86_64",
                    help="win and linux: the processor (aarch64 builds into win-arm64 and linux-arm64, docs/ARM64.md)")
    ap.add_argument("--style")
    ap.add_argument("--web-mode", choices=["auto", "scalar", "simd", "threads"], default="auto",
                    help="browser bundle: auto builds scalar, SIMD128 and shared-memory worker variants")
    ap.add_argument("--release", action="store_true")
    ap.add_argument("--static", action="store_true",
                    help="Linux: the UI programs static, without the GPU (section glibc; for systems without glibc)")
    ap.add_argument("--profile", choices=sorted(PROFILES),   # r13-measure: implies --release
                    help="release variant: size (z for UI crates, 2 for compute + app, LTO) or o2 (2, no LTO)")
    ap.add_argument("--out", help="output folder (default build; --release: build/release; --profile P: build/P)")
    ap.add_argument("-j", type=int, default=os.cpu_count() or 8,
                    help="Cargo's jobs (default: the logical CPUs)")
    ap.add_argument("--quick", action="store_true")
    ap.add_argument("--bin", help="run: one of the app's further executables (apps/NAME/BINS)")
    # new, api-diff, changelog, release (dev_command)
    ap.add_argument("--kind", choices=["window", "console", "tool", "service"], help="new: the kind of app")
    ap.add_argument("--title", help="new: the app's title (default: its name)")
    ap.add_argument("--lib", action="store_true", help="new: a library crate under lib/ instead of an app")
    ap.add_argument("--deps", help="new --lib: the crates of LIBS it uses (rt,plat)")
    ap.add_argument("--against", help="api-diff: the release to compare with (default: the newest)")
    ap.add_argument("--snapshot", action="store_true", help="api-diff: write docs/api/<the library's version>.txt")
    ap.add_argument("--all", action="store_true", help="api-diff: list every change")
    ap.add_argument("--type", default="added", help="changelog: added|changed|deprecated|removed|fixed|security")
    ap.add_argument("--area", help="changelog: the part the entry is about (ui, plat, build ...)")
    ap.add_argument("--date", help="release: the release date (default today)")
    ap.add_argument("--force", action="store_true", help="release: also when the API changes need a larger step")
    ap.add_argument("--check", action="store_true", help="site: build into a temporary folder and check it; i18n: errors only")
    ap.add_argument("--update", action="store_true", help="i18n: add the missing texts to the app's catalogue (untranslated)")
    ap.add_argument("--unused", action="store_true", help="i18n: list the unused rows of every app")
    a = ap.parse_args(argv)
    set_static_ui(a.static)                         # section "glibc"
    if a.arch != "x86_64" and a.cmd in ("run", "test", "watch"):
        print(f"{a.cmd}: nothing starts programs of --arch {a.arch} yet (docs/ARM64.md, ARM2)")
        return 1
    if a.cmd == "site":                             # tools/site.py (its own process: `site` is a stdlib name)
        return subprocess.run([sys.executable, str(ROOT / "tools" / "site.py")]
                              + (["--out", str((ROOT / a.out).resolve())] if a.out else [])
                              + (["--check"] if a.check else []), cwd=ROOT).returncode
    if a.cmd == "i18n":                             # tools/i18n_check.py
        if a.update and not a.name:
            print("i18n: --update needs the app (python build_rs.py i18n Kritzel --update)")
            return 1
        return subprocess.run([sys.executable, str(ROOT / "tools" / "i18n_check.py")] + ([a.name] if a.name else [])
                              + [f for f, on in (("--update", a.update), ("--check", a.check), ("--unused", a.unused)) if on],
                              cwd=ROOT).returncode
    if a.cmd == "watch":                            # hot reload (tools/hotreload.py, lib/ui/hotreload.rs)
        if not a.name:
            print("watch: the app is missing (python build_rs.py watch NAME [--target win|linux] [-- ARGS])")
            return 1
        if a.release or a.profile or a.out or (a.target or "win") not in ("win", "linux"):
            print("watch: development builds of win or linux only (no --release, --profile, --out)")
            return 1
        from tools import hotreload
        return hotreload.watch(a.name, a.target or "win", a.style, extra)
    if a.cmd in ("new", "api-diff", "changelog", "release"):
        if a.cmd == "new" and not a.name:
            print("new: the name is missing (python build_rs.py new NAME [--kind window|console|tool|service])")
            return 1
        return dev_command(a)
    bad = lint()
    if bad:
        print("\n".join(bad))
        return 1
    if a.cmd == "check":
        print("lint ok")
        return 0
    check_toolchain()
    tgt = a.target or ("win" if a.cmd == "run" else "both")
    targets = ["win", "linux"] if tgt == "both" else ["win", "linux", "web"] if tgt == "all" else [tgt]
    if a.arch != "x86_64" and not set(targets) <= set(ARM64_TRIPLE):
        raise BuildError(f"--arch {a.arch} is for win and linux (Android: --android-abi).")
    if "android" in targets and a.cmd not in ("app", "lib"):
        raise BuildError("Android supports app/lib builds; install the resulting APK with adb install -r FILE.apk.")
    if set(targets) & set(APPLE):                   # docs/APPLE.md: Apple's SDK and tools are on a Mac only
        if sys.platform != "darwin":
            raise BuildError("mac and ios build on a Mac (Apple's SDK; docs/APPLE.md).")
        if a.cmd not in ("app", "lib", "run") and not (a.cmd == "test" and a.name and targets == ["mac"]):
            raise BuildError("mac and ios support app, lib and run (mac also test NAME); the iPhone itself: "
                             "python tools/apple.py device APP.app")
        if a.cmd == "run" and targets == ["ios"] and not a.sim:
            raise BuildError("run --target ios starts the simulator's build (--sim); the iPhone: python tools/apple.py device APP.app")
    if a.cmd in ("run", "test") and "web" in targets:
        raise BuildError("Use app/all --target web to build browser bundles; serve their index.html for browser tests. Native run/test requires win/linux/both.")
    out = ((ROOT / a.out).resolve() if a.out else ROOT / "build" / a.profile if a.profile
           else ROOT / "build" / "release" if a.release else ROOT / "build")
    builders = make_builders(targets, out, a.release, a.j, a.profile, a.web_mode, a.android_abi, a.arch, a.sim)
    t0 = time.perf_counter()
    try:
        if a.cmd == "lib":
            with ThreadPoolExecutor(len(builders)) as ex:
                res = [(b, ex.submit(b.cargo_libs)) for b in builders]
                for b, f in res:
                    print(f"[{b.folder}] libraries: {', '.join(f.result())} and what they need")
        elif a.cmd in ("app", "run"):
            if not a.name:
                raise BuildError("app name missing")
            styles = a.style.split(",") if a.style else None
            bad = [s for s in styles or [] if s not in STYLES]
            if bad:
                raise BuildError(f"unknown style {', '.join(bad)} (styles: {', '.join(STYLES)})")
            for b, n, exe in build_apps(builders, [a.name], styles):
                if exe:
                    print(f"[{b.folder}] app {n}: {exe.relative_to(ROOT)} ({exe.stat().st_size} bytes)")
            if a.cmd == "run":
                if a.bin:                           # app-bins
                    if a.bin not in [n for n, _, _ in app_bins(a.name)]:
                        raise BuildError(f"{a.name} has no executable {a.bin} (apps/{a.name}/BINS)")
                    exe = exe.parent / bin_file(a.bin, targets[0])
                r = run_exe("ios-sim" if a.sim and targets[0] == "ios" else targets[0], exe, extra, timeout=None)
                sys.stdout.write(r.stdout)
                sys.stderr.write(r.stderr)
                return r.returncode
        elif a.cmd == "all":
            for b, n, exe in build_apps(builders, apps()):
                if exe:
                    print(f"[{b.folder}] app {n}: {exe.relative_to(ROOT)} ({exe.stat().st_size} bytes)")
        elif a.cmd == "test":
            ok = run_app_suite(builders, a.name) if a.name else run_tests(builders, out, a.quick)
            print(f"test: {'ok' if ok else 'FAILED'} ({time.perf_counter() - t0:.1f} s)")
            return 0 if ok else 1
    except BuildError as e:
        print(e)
        print("build failed")
        return 1
    print(f"done ({time.perf_counter() - t0:.2f} s)")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (SourceError, BuildError) as error:
        print(f"source error: {error}", file=sys.stderr)
        sys.exit(1)
