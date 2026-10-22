#!/usr/bin/env python3
"""build_rs.py -- build and test JFLib.
rustc builds the library and app crates directly; the locked no_std text_shape
dependencies are built through Cargo and linked as static .rlib files.

  python build_rs.py lib    [--target win|linux|both]        every library crate
  python build_rs.py app NAME [--style S]                    apps/NAME -> build/<t>/NAME[.exe]
                                                                (+ its loadable modules, e.g. PaintCraft's jxlenc: section "jxl-module")
  python build_rs.py app NAME --style S1,S2,..               one variant per style: build/<t>/<style>/NAME[.exe]
                                                                (libraries once, the variants linked in parallel)
  python build_rs.py all                                     libraries + every app
  python build_rs.py run NAME [--target win|linux] [-- ARGS] build and start (Linux: WSL)
  python build_rs.py run NAME --bin BIN [-- ARGS]            start one of the app's further executables (BINS)
  python build_rs.py test [--quick]                          lint, build, every apps/*/tests/*.uis (+ apps/*/tests),
                                                                then parity with asm (not with --quick)
  python build_rs.py test NAME                               build one app and run its tests/*.uis
  python build_rs.py check                                   lint only
  python build_rs.py new NAME [--kind window|console|tool|service] [--style S] [--title T]
                                                             a new app apps/NAME that builds and passes its
                                                                test as it is (tools/scaffold.py)
  python build_rs.py new --lib NAME [--deps rt,plat]         a new library crate lib/NAME, entered in LIBS
  python build_rs.py api-diff [--against X.Y.Z] [--snapshot] the public API since a release (tools/apidiff.py,
                                                                docs/VERSIONING.md): exit 1 for breaking changes
                                                                lib/VERSION does not allow
  python build_rs.py changelog ["TEXT" --type T --area A]    show / add to [Unreleased] of CHANGELOG.md
  python build_rs.py release X.Y.Z [--date D] [--force]      CHANGELOG.md, lib/VERSION, docs/api/X.Y.Z.txt
                                                                (tools/changelog.py)
  python build_rs.py site [--out DIR] [--check]              the website: landing page, docs and API reference from
                                                                the Markdown -> build/site/ (tools/site.py); --check
                                                                builds into a temporary folder and checks links,
                                                                anchors, images and the HTML (the suite runs it)
  python build_rs.py i18n [APP|lib] [--update] [--unused]    the translatable texts against the catalogues
                                                                (tools/i18n_check.py): missing, untranslated, unused,
                                                                placeholders and plural forms; --update adds the
                                                                missing rows (untranslated); --check: the suite's
                                                                check (errors only, the gaps are reported)
  python build_rs.py ide                                    write rust-project.json
  python build_rs.py plugin [NAME]                           plugins/NAME -> <out>/<t>/plugins/NAME.dll|.so
                                                                (PaintCraft plugins: section "rust-plugin")
  options: --target win|linux|both (default both; run: win), --release, --out DIR, -j N,
           --profile size|o2 (release variants, section "r13-measure")

Both targets build side by side.  Every crate starts as soon as its own dependencies are done
(no waves), an app waits only for the crates its sources name (app_crates: PaintCraft does not
wait for img_jxl_enc), the apps' modules build alongside the libraries, and the apps link in
parallel once their libraries exist (a release link is one single-threaded LTO step: PaintCraft
about 80 s).  -j N (default: the logical CPUs) bounds the rustc processes running at once over
everything.

Output folders (--out overrides): build/<target> for development builds, build/release/<target>
for --release, build/<profile>/<target> for --profile (so the variants never rebuild each
other).  The assembly reference builds into legacy/build (legacy/build.py).

Development builds (default): incremental, 16 codegen units, opt-level 2 for the compute
crates (gfx, text, simd, fx, img_*) and 1 for the rest (quick rebuilds).  --release:
opt-level 2, one codegen unit, fat LTO (as fast as opt-level 3 and 6-10 % smaller,
rewrite/REPORT.md section 2; R13 measured with 3).  A crate is rebuilt only when
its sources, its dependencies or the flags changed (hash stamps in <out>/<target>/.stamp).

Rules checked before every build (`check`):
  * every crate root starts with #![no_std]; only alloc / core are `extern crate`d
  * `unsafe` is allowed only in rt, simd, plat, plugin: every other library and style
    crate has #![forbid(unsafe_code)], every app #![deny(unsafe_code)] (its one exception
    is the ui::main! entry point)
  * own crates use --extern; text_shape's pinned Cargo dependencies are the exception

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
from tools import assetpack

ROOT = Path(__file__).resolve().parent              # RustLib/
RS = ROOT
LLVM = Path(r"C:\Program Files\LLVM\bin")
TRIPLE = {"win": "x86_64-pc-windows-msvc", "linux": "x86_64-unknown-linux-gnu", "web": "wasm32-unknown-unknown", "android": "aarch64-linux-android"}
RUSTC_VERSION = "1.98"          # the toolchain this code is written for (checked, minor ignored)
UNSAFE_OK = {"rt", "simd", "plat", "plugin"}

# library crates: name -> (root file, dependencies); order = a valid build order
LIBS = {
    "rt": ("lib/rt/lib.rs", []),
    "agent_core": ("lib/agent_core/src/lib.rs", ["rt"]),
    "plat": ("lib/plat/lib.rs", ["rt", "compress", "crypto"]),  # vfs: LZ4 of the app:// bundles; secret: DH, AES, SHA-256
    "base": ("lib/base/lib.rs", ["rt", "plat"]),
    "agent_io": ("lib/agent_io/src/lib.rs", ["rt", "plat", "agent_core"]),
    "simd": ("lib/simd/lib.rs", ["rt"]),
    "gfx": ("lib/gfx/lib.rs", ["rt", "simd"]),
    "img_core": ("lib/img_core/lib.rs", ["rt", "simd", "gfx"]),
    "archive": ("lib/archive/lib.rs", ["rt", "img_core", "plat"]),  # vfs: ZipFs
    "doc_core": ("lib/doc_core/lib.rs", ["rt", "base"]),
    "doc_layout": ("lib/doc_layout/lib.rs", ["rt", "doc_core"]),
    "doc_io": ("lib/doc_io/lib.rs", ["rt", "base", "plat", "archive", "doc_core"]),
    "img_png": ("lib/img_png/lib.rs", ["rt", "simd", "gfx", "img_core"]),
    "text_shape": ("lib/text_shape/src/lib.rs", []),
    "text": ("lib/text/lib.rs", ["rt", "plat", "base", "simd", "gfx", "img_core", "img_png", "text_shape"]),
    "img_jpeg": ("lib/img_jpeg/lib.rs", ["rt", "simd", "gfx", "img_core"]),
    "img_bmp": ("lib/img_bmp/lib.rs", ["rt", "simd", "gfx", "img_core", "img_png", "img_jpeg"]),  # R6-bgt: BI_PNG / BI_JPEG
    "img_gif": ("lib/img_gif/lib.rs", ["rt", "simd", "gfx", "img_core"]),
    "img_tga": ("lib/img_tga/lib.rs", ["rt", "simd", "gfx", "img_core"]),
    "img_tiff": ("lib/img_tiff/lib.rs", ["rt", "simd", "gfx", "img_core", "img_jpeg",
                                          "plat"]),  # R6-tiff: plat for the parallel decoder
    # R9 codec crates (main agent's skeleton; the R9 agents may add dependencies)
    "img_webp": ("lib/img_webp/lib.rs", ["rt", "simd", "gfx", "img_core", "plat"]),
    "img_hevc": ("lib/img_hevc/lib.rs", ["rt", "simd", "gfx", "img_core", "plat"]),
    "img_av1": ("lib/img_av1/lib.rs", ["rt", "simd", "gfx", "img_core", "plat"]),
    "img_heif": ("lib/img_heif/lib.rs", ["rt", "simd", "gfx", "img_core", "plat", "img_hevc", "img_av1"]),
    "img_avif": ("lib/img_avif/lib.rs", ["rt", "simd", "gfx", "img_core", "plat", "img_heif", "img_av1"]),
    "img_jxl": ("lib/img_jxl/lib.rs", ["rt", "simd", "gfx", "img_core", "plat", "img_jpeg"]),
    # jxl-module: the new JPEG XL writer (linked into codectest; PaintCraft loads it as modules/jxlenc)
    "img_jxl_enc": ("lib/img_jxl_enc/lib.rs", ["rt", "simd", "gfx", "img_core", "plat", "img_jpeg", "img_jxl"]),
    "img_jxr": ("lib/img_jxr/lib.rs", ["rt", "simd", "gfx", "img_core", "plat"]),
    "img_dds": ("lib/img_dds/lib.rs", ["rt", "simd", "gfx", "img_core", "plat"]),
    "img_pdn": ("lib/img_pdn/lib.rs", ["rt", "simd", "gfx", "img_core", "plat", "img_png"]),
    "fx": ("lib/fx/lib.rs", ["rt", "simd", "gfx", "img_core", "plat"]),  # R10 (main agent's skeleton)
    # svg: SVG images and icons drawn through gfx's paths (the codec IMGF_SVG; ui.svg, ui.image_view)
    "img_svg": ("lib/img_svg/lib.rs", ["rt", "gfx", "img_core"]),
    "img_all": ("lib/img_all/lib.rs", ["rt", "gfx", "img_core", "img_png", "img_jpeg", "img_bmp", "img_gif", "img_tga",
                                        "img_tiff", "img_webp", "img_heif", "img_avif", "img_jxl", "img_jxr",
                                        "img_dds", "img_pdn"]),
    # spell checking: Hunspell dictionaries, the system's checker (plat::spell), the personal word list
    "spell": ("lib/spell/lib.rs", ["rt", "plat"]),
    "ui": ("lib/ui/lib.rs", ["rt", "base", "gfx", "text", "plat", "img_core", "img_png", "img_all", "agent_core", "agent_io",
           "datetime", "spell", "img_svg"]),  # i18n: dates in the locale's format; spell: TB_SPELL; svg: ui.svg / image_view
    "doc_view": ("lib/doc_view/src/lib.rs", ["rt", "doc_core", "doc_layout", "gfx", "text", "ui"]),
    # BackupCraft's foundations (usable by every app): calendar time and cron schedules
    "datetime": ("lib/datetime/lib.rs", []),
    "crypto": ("lib/crypto/lib.rs", []),
    "compress": ("lib/compress/lib.rs", []),
    "netpkt": ("lib/netpkt/lib.rs", []),  # JFNetworker: addresses, checksums, header codecs, pcapng
    "datastore": ("lib/datastore/lib.rs", ["rt", "plat", "crypto", "agent_core"]),
    "netio": ("lib/netio/lib.rs", ["rt", "plat", "crypto", "agent_core"]),
    "cas": ("lib/cas/lib.rs", ["rt", "plat", "crypto", "compress", "agent_core"]),
    # signed app updates with rollback (docs/UPDATES.md)
    "update": ("lib/update/lib.rs", ["rt", "plat", "crypto", "agent_core", "netio"]),
    "script": ("lib/script/lib.rs", ["rt", "agent_core"]),  # JFNetworker's .jfs scripts (SK3: json via agent_core)
}
STYLES = sorted(d.name for d in (ROOT / "styles").iterdir() if d.is_dir() and entrypoint(d, "lib", False))


class BuildError(Exception):
    pass


def tool(name):
    p = LLVM / (name + ".exe")
    return str(p) if os.name == "nt" and p.exists() else name


RUSTC_SLOTS = threading.BoundedSemaphore(os.cpu_count() or 8)     # rustc processes at once (main(): -j)


def run_rustc_pipelined(cmd, on_meta):
    """rustc with JSON messages: on_meta() as soon as the crate's metadata file is written (its
    dependents may start), the diagnostics as rustc renders them -> (exit code, output)"""
    p = subprocess.Popen([str(c) for c in cmd] + ["--error-format=json", "--json=artifacts"],
                         stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, encoding="utf-8",
                         errors="replace")
    lines = []
    for ln in p.stdout:
        try:
            msg = json.loads(ln) if ln.startswith("{") else None
        except ValueError:
            msg = None
        if not isinstance(msg, dict):
            lines.append(ln)
        elif "artifact" in msg:
            if msg.get("emit") == "metadata":
                on_meta()
        elif msg.get("rendered"):
            lines.append(msg["rendered"])
    return p.wait(), "".join(lines)


def run_cmd(cmd, quiet=False, on_meta=None, env=None):
    """run a command (rustc: within RUSTC_SLOTS; on_meta: pipelined, run_rustc_pipelined) ->
    its output, BuildError on failure.  env: variables to set (None as a value: removed)"""
    full_env = None
    if env:
        full_env = {k: v for k, v in os.environ.items() if k not in env}
        full_env.update({k: v for k, v in env.items() if v is not None})
    if cmd[0] == "rustc" and on_meta:
        with RUSTC_SLOTS:
            code, out = run_rustc_pipelined(cmd, on_meta)
            r = subprocess.CompletedProcess(cmd, code, out, "")
    elif cmd[0] == "rustc":
        with RUSTC_SLOTS:
            r = subprocess.run([str(c) for c in cmd], capture_output=True, text=True, encoding="utf-8",
                               errors="replace", env=full_env)
    else:
        r = subprocess.run([str(c) for c in cmd], capture_output=True, text=True, encoding="utf-8", errors="replace",
                           env=full_env)
    out = (r.stdout + r.stderr).strip()
    if r.returncode != 0:
        raise BuildError(out or f"{cmd[0]} failed")
    if out and not quiet:
        print(out)
    return out


# ---------------------------------------------------------------- crates
def compute_crate(name):
    """crates whose speed matters in development builds too (opt-level 2; Kritzel and PaintCraft
    draw with their own pixel loops)"""
    return (name in ("gfx", "text", "simd", "fx", "Kritzel", "PaintCraft", "crypto", "compress", "cas", "spell")
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


def run_dag(nodes):
    """nodes: {name: (deps, fn)} -> fn(name, ready) for every node, each as soon as its
    dependencies inside nodes are ready (not in waves: a long crate holds up only what depends
    on it), independent ones at once (RUSTC_SLOTS bounds the rustc processes).  fn may call
    ready() before it returns: a crate's metadata is written, its dependents compile against
    it while its code is still generated (pipelining, as Cargo does).  After a failure no
    further node starts; the first failure is raised once every node finished.  Returns the
    names whose fn returned True (rebuilt)."""
    done = {n: threading.Event() for n in nodes}
    errors, built = [], []

    def work(n):
        deps, fn = nodes[n]
        try:
            for d in deps:
                if d in done:
                    done[d].wait()
            if not errors and fn(n, done[n].set):
                built.append(n)
        except BaseException as e:      # noqa: BLE001 -- raised by the caller's thread below
            errors.append(e)
        finally:
            done[n].set()
    threads = [threading.Thread(target=work, args=(n,), daemon=True) for n in nodes]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    if errors:
        raise errors[0]
    return [n for n in nodes if n in built]


def compile_rlib(cmd, root, out, deps, piped, ready):
    """compile a library crate (cmd: rustc, flags, crate type and name) to out =
    <dir>/lib<name>.rlib; deps: [(extern name, its rlib)].  Pipelined: rustc writes the crate's
    .rmeta first and ready() lets its dependents start; they compile against the .rmeta of the
    crates compiled in this run (piped), as Cargo does.  The old .rlib / .rmeta are removed
    first: no stale file lies next to the new metadata.  The apps and modules linked from these
    rlibs are byte for byte the ones of a build without pipelining (checked on PaintCraft and
    jxlenc, both targets)."""
    rmeta = out.with_suffix(".rmeta")
    for f in (out, rmeta):
        f.unlink(missing_ok=True)
    ext = [x for nm, rl in deps for x in ("--extern", f"{nm}={rl.with_suffix('.rmeta') if rl in piped else rl}")]
    piped.add(out)                  # before ready(): the dependents look it up when they start
    # -o, not --out-dir: it keeps the object names inside the rlib (lib<name>.<name>.<hash>-
    # cgu.0.rcgu.o); with --out-dir they lose the "lib" and the LTO step lays out the exe
    # differently
    run_cmd(cmd + ext + [f"--emit=metadata={rmeta},link", "-o", out, root], on_meta=ready)


def crate_dir(root_file):
    return resolve_root(RS / root_file).parent


def named_crates(root, names):
    """the crates of names that one project's source set mentions (NAME:: or extern crate NAME)"""
    src = "\n".join(f.read_text(encoding="utf-8") for f in project_sources(root))
    return {c for c in names if re.search(rf"\b{c}::|extern crate {c}\b", src)}


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


def all_libs():
    d = {name: (resolve_root(RS / root).relative_to(RS).as_posix(), deps)
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
PERM_KEYS_FLAG = ("fs.user", "print", "secrets", "autostart", "audio.output", "input.global", "screen.capture")
PERM_KEYS_AGENT = ("agent.confirm", "agent.deny")                  # agent actions: confirmed / refused
PERM_KEYS_SANDBOX = ("sandbox",)                                   # OS sandbox: "sandbox [strict]"
PERM_ASKABLE = PERM_KEYS_ARGS + PERM_KEYS_FLAG[1:]                  # "ask KEY ARGS [\"reason\"]"
PERM_VARS = ("CONFIG", "TEMP", "HOME", "EXE", "FONTS", "APPCONFIG", "APPDATA", "APPCACHE", "APPLOG",
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
                             f"({', '.join(PERM_KEYS_ARGS[:2] + PERM_KEYS_FLAG[:1] + PERM_KEYS_ARGS[2:] + PERM_KEYS_FLAG[1:] + PERM_KEYS_SANDBOX + ('ask',) + PERM_KEYS_AGENT)})")


# manifests and the error perm_check must give (None: valid); `check` runs them, so the build's
# syntax stays the one plattest --test permask checks in policy.rs
PERM_CASES = [
    ('ask fs.read $DOCUMENTS/** "Briefe # öffnen"  # a comment\nask net.connect *:443\nask print\n'
     'agent.confirm doc.* ui.type\nagent.deny *.wipe', None),
    ("ask fs.user", "fs.user cannot be asked for"),
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


# apps/NAME/PACKAGE: what ui::main! registers on --register (lib/plat/assoc.rs, docs/GUIDE.md
# "Desktop integration"): display, app_id, categories, filetypes and schemes, checked as
# tools/pkgmeta.py checks them for the packages and handed to rustc as JFGUI_PACKAGE.
def app_package(name):
    """the JFGUI_PACKAGE text of apps/NAME/PACKAGE ("key=value" lines), None without the file"""
    f = ROOT / "apps" / name / "PACKAGE"
    if not f.exists():
        return None
    tools = str(ROOT / "tools")
    if tools not in sys.path:
        sys.path.append(tools)
    import pkgmeta
    meta = pkgmeta.kv_file(f)
    try:
        types = pkgmeta.filetypes(meta)
        schemes = pkgmeta.schemes(meta)
        aid = pkgmeta.app_id(meta, name)
    except SystemExit as e:
        raise BuildError(f"apps/{name}/PACKAGE: {e}")
    ft = "; ".join(f"{','.join(exts)}|{mime}|{desc}" + ("|own" if own else "") for exts, mime, desc, own in types)
    lines = [f"display={meta.get('display', name)}", f"app_id={aid}", f"categories={meta.get('categories', 'Utility;')}",
             f"filetypes={ft}", f"schemes={' '.join(schemes)}"]
    return "\n".join(lines) + "\n"


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


# ---------------------------------------------------------------- assets
# apps/NAME/assets: the app's files, packed by tools/assetpack.py (JFA1, LZ4) and built into
# the program; ui::main! mounts them as app:// (lib/plat/vfs.rs, docs/VFS.md).


def app_assets_hash(name):
    """a digest of apps/NAME/assets (names and contents), or None without the folder"""
    d = ROOT / "apps" / name / "assets"
    if not d.is_dir():
        return None
    h = hashlib.sha1()
    for rel, path in sorted(assetpack.collect(d).items()):
        h.update(rel.encode("utf-8") + b"\0")
        h.update(path.read_bytes())
    return h.hexdigest()


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
def build_app_bins(b, app, vdir=None):
    """app-bins: the BINS of an app into the Builder's folder (once per run, independent ones in
    parallel), copied next to a style variant's exe (vdir) -> their paths"""
    if b.target not in ("win", "linux") or b.target not in app_targets(app):
        return []
    with b.bin_lock:
        if app not in b.bin_out:
            bins = app_bins(app)
            with ThreadPoolExecutor(max(1, len(bins))) as ex:
                futs = [ex.submit(b.build_bin, app, n, r, k) for n, r, k in bins]
                b.bin_out[app] = [f.result() for f in futs]
    if vdir is not None and vdir != b.dir:
        for exe in b.bin_out[app]:
            shutil.copyfile(exe, vdir / exe.name)
    return b.bin_out[app]
# ---------------------------------------------------------------- end app-bins


# ---------------------------------------------------------------- lint
def lint():
    """the rules of the module docstring -> list of problems"""
    bad = []
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
            for i, ln in enumerate(f.read_text(encoding="utf-8").splitlines(), 1):
                s = ln.strip()
                if s.startswith("extern crate") and s not in ("extern crate alloc;", "extern crate core;"):
                    bad.append(f"{f.relative_to(ROOT)}:{i}: only alloc / core may be extern crates")
                if s.startswith("use std") or " std::" in s.split("//")[0]:
                    bad.append(f"{f.relative_to(ROOT)}:{i}: std is not available (no_std)")
                if ln[:2] in ('")', '",', '";'):
                    bad.append(f"{f.relative_to(ROOT)}:{i}: a string literal broken by a line end (a script edit?)")
    return bad + plugin_lint() + module_lint() + perm_lint()   # rust-plugin: lib/plugin, plugins/*; jxl-module: modules/*; manifests


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


# ---------------------------------------------------------------- build
class Builder:
    def __init__(self, target, out, release, jobs, profile=None, web_mode="scalar", android_abi="arm64-v8a"):
        self.target = target
        self.dir = out / target
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
        self.profile = PROFILES[profile] if profile else None     # r13-measure
        self.jobs = jobs
        self.dir.mkdir(parents=True, exist_ok=True)
        (self.dir / ".stamp").mkdir(exist_ok=True)
        self.hashes = {}
        self.lib_lock = threading.Lock()      # build_libs from parallel app builds
        self.ready = set()                    # crates checked / built in this run
        self.res_lock = threading.Lock()      # an app's icon resource (style variants link at once)
        self.mod_lock = threading.Lock()      # jxl-module: an app's modules, once per run
        self.mod_ready = set()
        self.bin_lock = threading.Lock()      # app-bins: an app's further executables, once per run
        self.bin_out = {}
        self.asset_lock = threading.Lock()    # vfs: apps/NAME/assets packed once per run (app, bins, variants)
        self.asset_out = {}
        self.piped = set()                    # rlibs compiled in this run (their .rmeta is current)

    def flags(self, name=""):
        triple = "x86_64-linux-android" if self.target == "android" and self.android_abi == "x86_64" else TRIPLE[self.target]
        f = ["--edition", "2024", "--target", triple, "-C", "panic=abort", "-C", "debuginfo=0",
             "-D", "warnings", "-L", f"dependency={self.dir}",
             f"--remap-path-prefix={ROOT}{os.sep}="]       # panic locations: rs/..., no build paths
        if self.release and self.profile:                 # r13-measure: --profile
            p = self.profile
            opt = p["app"] if name == "" else p["compute"] if compute_crate(name) else p["lib"]
            f += ["-C", f"opt-level={opt}", "-C", "codegen-units=1"]
        elif self.release:
            # opt-level 2: no slower than 3 in any measurement, 6-10 % smaller (REPORT.md, variant "2")
            f += ["-C", "opt-level=2", "-C", "codegen-units=1"]
        else:
            # development: compute crates optimised (pixels, codecs stay fast in tests), the
            # rest quick to compile
            f += ["-C", f"opt-level={2 if compute_crate(name) else 1}", "-C", "codegen-units=16"]
        if self.target == "web" and self.web_mode != "scalar":
            f += ["-C", "target-feature=+simd128" + (",+atomics,+bulk-memory" if self.web_mode == "threads" else "")]
        if self.target == "web" and self.web_mode == "threads":
            # LLVM supports wasm atomics on stable; Rust still labels the feature unstable.
            f += ["--cfg", "web_threads", "-A", "unstable-features"]
            f += ["--sysroot", str(self.web_sysroot)]
        return f

    def rlib(self, name):
        return self.dir / f"lib{name}.rlib"

    def src_hash(self, root, deps, extra="", name=""):
        h = hashlib.sha1()
        files = project_sources(root, assets=True)
        for f in files:
            h.update(f.relative_to(RS).as_posix().encode())
            h.update(f.read_bytes())
        for d in deps:
            h.update(self.hashes[d].encode())
        h.update(" ".join(self.flags(name)).encode())
        h.update(extra.encode())
        # include_bytes! assets of styles
        if "styles" in root.parts:
            for f in sorted((RS / "styles" / root.parent.name / "assets").glob("*")) + sorted((RS / "styles" / "winui" / "assets").glob("*")):
                h.update(f.read_bytes())
        return h.hexdigest()

    def up_to_date(self, name, digest, out):
        st = self.dir / ".stamp" / name
        return out.exists() and st.exists() and st.read_text() == digest

    def stamp(self, name, digest):
        (self.dir / ".stamp" / name).write_text(digest)

    def build_lib(self, name, root, deps, ready=None):
        if name == "text_shape":
            return self.build_text_shape(root, ready)
        digest = self.src_hash(RS / root, deps, name=name)
        self.hashes[name] = digest
        out = self.rlib(name)
        if self.up_to_date(name, digest, out):
            return False
        cmd = ["rustc"] + self.flags(name) + ["--crate-type", "rlib", "--crate-name", name]
        if not self.release:
            cmd += ["-C", f"incremental={self.dir / 'inc' / name}"]
        compile_rlib(cmd, RS / root, out, [(d, self.rlib(d)) for d in deps], self.piped, ready)
        if self.target in ("win", "linux") and compute_crate(name) and name != "simd":
            check_intrinsics(name, out)
        self.stamp(name, digest)
        return True

    def build_text_shape(self, root, ready):
        """Pinned no_std OpenType/Unicode dependencies, linked into our static binary."""
        manifest = RS / "lib/text_shape/Cargo.toml"
        lock = manifest.with_name("Cargo.lock")
        digest = self.src_hash(RS / root, [], extra=manifest.read_text()+lock.read_text()+"cargo-shaping-v2", name="text_shape")
        self.hashes["text_shape"] = digest
        if self.up_to_date("text_shape", digest, self.rlib("text_shape")):
            return False
        triple = "x86_64-linux-android" if self.target == "android" and self.android_abi == "x86_64" else TRIPLE[self.target]
        cargo_dir = self.dir / "cargo-shaping"
        env = os.environ.copy()
        # Embedded LLVM bitcode is required by JFLib's release LTO link.
        env["RUSTFLAGS"] = "-C panic=abort -C embed-bitcode=yes"
        if self.target == "web" and self.web_mode == "threads":
            env["RUSTFLAGS"] += f" --sysroot {self.web_sysroot} -C target-feature=+atomics,+bulk-memory,+simd128 -A unstable-features"
        command = ["cargo", "build", "--locked", "--release", "--manifest-path", str(manifest),
                   "--target", triple, "--target-dir", str(cargo_dir)]
        result = subprocess.run(command, cwd=ROOT, env=env, capture_output=True, text=True, encoding="utf-8", errors="replace")
        if result.returncode:
            raise RuntimeError(result.stdout + result.stderr)
        artifacts = cargo_dir / triple / "release"
        for artifact in (artifacts / "deps").glob("*.rlib"):
            if not artifact.name.startswith("libtext_shape-"):
                shutil.copy2(artifact, self.dir / artifact.name)
        # Proc-macro metadata is a host DLL/so even when the output targets Linux/Web.
        for suffix in ("*.dll", "*.so", "*.dylib"):
            for artifact in (cargo_dir / "release/deps").glob(suffix):
                shutil.copy2(artifact, self.dir / artifact.name)
        shutil.copy2(artifacts / "libtext_shape.rlib", self.rlib("text_shape"))
        self.stamp("text_shape", digest)
        if ready is not None:
            ready()
        return True

    def build_libs(self, names):
        """build the crates and their dependencies, each as soon as its dependencies are done,
        independent ones in parallel (once per run: the app builds that run in parallel find
        them ready)"""
        with self.lib_lock:
            libs = all_libs()
            need = set()

            def add(n):
                if n not in need:
                    need.add(n)
                    for d in libs[n][1]:
                        add(d)
            for n in names:
                add(n)
            todo = [n for n in libs if n in need and n not in self.ready]

            def one(n, ready):
                r = self.build_lib(n, libs[n][0], libs[n][1], ready)
                self.ready.add(n)
                return r
            return run_dag({n: (libs[n][1], one) for n in todo})

    def boot_inputs(self, app, bin=None):
        """permissions / vfs: what a program of the app gets built in -> (manifest text or None,
        assets digest or None, digest text)"""
        perms = app_permissions(app, bin)                 # permissions: the app's (or bin's) manifest
        assets = app_assets_hash(app)                     # vfs: apps/NAME/assets -> app://
        package = app_package(app)                        # assoc: PACKAGE's file types and schemes
        version = app_version(app)                        # PACKAGE version=: rt::app_version!()
        return perms, assets, (("" if perms is None else " permissions=" + perms) + ("" if assets is None else " assets=" + assets)
                               + ("" if package is None else " package=" + package)
                               + ("" if version is None else " version=" + version))

    def boot_args(self, app, perms, assets):
        """the rustc arguments and environment of boot_inputs: JFGUI_APP, JFGUI_PERMISSIONS,
        JFGUI_PACKAGE (ui::main! only), JFGUI_VERSION (rt::app_version!()) and JFGUI_ASSETS
        (apps/APP/assets packed into <dir>/assets/APP.jfa, --cfg jfgui_assets), read by ui::main!
        and rt::main! -> (args, env)"""
        env = {"JFGUI_PERMISSIONS": perms, "JFGUI_APP": app, "JFGUI_ASSETS": None, "JFGUI_PACKAGE": app_package(app),
               "JFGUI_VERSION": app_version(app)}
        if assets is None:
            return [], env
        jfa = self.dir / "assets" / f"{app}.jfa"
        with self.asset_lock:
            if self.asset_out.get(app) != assets or not jfa.exists():
                jfa.parent.mkdir(parents=True, exist_ok=True)
                jfa.write_bytes(assetpack.pack(ROOT / "apps" / app / "assets"))
                self.asset_out[app] = assets
        env["JFGUI_ASSETS"] = str(jfa.resolve())
        return ["--cfg", "jfgui_assets"], env

    def build_app(self, name, style, variant=False):
        """apps/NAME with STYLE -> <dir>/NAME[.exe]; variant (several styles in one run):
        <dir>/<style>/NAME[.exe], with a copy of the app's modules next to it"""
        if self.target not in app_targets(name):
            return None
        scrate = f"style_{style}"
        crates = app_crates(name)
        extra = app_styles(name)                       # further styles to switch to at run time
        self.build_libs(crates + sorted({scrate, *extra}))
        root = entrypoint(RS / "apps" / name, "app")
        deps = crates + sorted({scrate, *extra})
        if self.target == "android":
            from tools.android import build_apk
            return build_apk(self, name, style, variant, root, crates + extra, run_cmd,
                             app_notices([style] + [c[len("style_"):] for c in extra]))
        lto = self.profile["lto"] if self.release and self.profile else "fat"      # r13-measure
        perms, assets, boot = self.boot_inputs(name)     # permissions, vfs: manifest and assets
        digest = self.src_hash(root, deps, extra=style + ("" if lto == "fat" else f" lto={lto}")
                               + (" wasm-variant-link-v1" if self.target == "web" else "") + boot)
        key = "app_" + name + (f"@{style}" if variant else "")
        vdir = self.dir / style if variant else self.dir
        if self.target == "web":
            vdir = (self.web_public / style if variant else self.web_public) / name
        vdir.mkdir(parents=True, exist_ok=True)
        for s in sorted({style, *(c[len("style_"):] for c in extra)}):
            copy_style_notices(name, s, vdir)
        copy_library_notices(vdir)
        web_suffix = ("." + self.web_mode if self.web_mode != "scalar" else "") + ".wasm"
        exe = vdir / (name + (".exe" if self.target == "win" else web_suffix if self.target == "web" else ""))
        if self.up_to_date(key, digest, exe):
            build_app_modules(self, name, vdir)           # jxl-module: its modules next to it
            if self.target == "web":
                web_assets(name, vdir)
            return exe
        cmd = ["rustc"] + self.flags() + ["--crate-type", "cdylib" if self.target == "web" else "bin", "--crate-name", name.replace("-", "_").lower(), "-C", "strip=symbols"]
        if self.release:
            cmd += ["-C", "lto" if lto == "fat" else f"lto={lto}"]
        if self.target == "web":
            # All browser variants share the public bundle directory. Give their
            # intermediate codegen objects distinct names during parallel linking.
            cmd += ["-C", f"metadata=jfgui_{self.web_mode}",
                    "-C", f"linker={tool('wasm-ld')}", "-C", "link-arg=--no-entry",
                    "-C", "link-arg=--export-memory", "-C", "link-arg=-zstack-size=8388608"]
            if self.web_mode == "threads":
                cmd += [x for arg in ("--import-memory", "--shared-memory", "--max-memory=2147483648",
                                      "--initial-memory=33554432", "--export=__stack_pointer")
                        for x in ("-C", "link-arg=" + arg)]
        elif not self.release:
            cmd += ["-C", f"incremental={self.dir / 'inc' / key}"]
        for d in crates:
            cmd += ["--extern", f"{d}={self.rlib(d)}"]
        cmd += ["--extern", f"style={self.rlib(scrate)}"]
        for c in extra:
            cmd += ["--extern", f"{c}={self.rlib(c)}"]
        cmd += self.native_link_args(name, exe, "WINDOWS")
        boot_args, env = self.boot_args(name, perms, assets)
        cmd += boot_args
        # rustc's temporary object filenames use the crate name, not all output
        # suffixes. Separate link directories prevent simultaneous variants from
        # overwriting each other's objects before publishing the final WASM.
        link_root = self.dir / "link" / style if variant else self.dir / "link"
        linked = link_root / name / exe.name if self.target == "web" else exe
        linked.parent.mkdir(parents=True, exist_ok=True)
        cmd += ["-o", linked, root]
        run_cmd(cmd, env=env)
        if linked != exe:
            shutil.copyfile(linked, exe)
        pdb = vdir / f"{name}.pdb"                # only this app's (a glob NAME* would match longer app names too)
        if pdb.exists():
            pdb.unlink()
        self.stamp(key, digest)
        build_app_modules(self, name, vdir)               # jxl-module: its modules next to it
        build_app_bins(self, name, vdir)                  # app-bins: its further executables next to it
        if self.target == "web":
            web_assets(name, vdir)
        return exe

    def native_link_args(self, app, exe, subsystem):
        """the linker arguments of a native executable (Windows: /SUBSYSTEM:WINDOWS or CONSOLE and
        the app's icon resource)"""
        if self.target == "win":
            args = ["-C", f"linker={tool('lld-link')}", "-C", "link-arg=/ENTRY:mainCRTStartup",
                    "-C", f"link-arg=/SUBSYSTEM:{subsystem}", "-C", "link-arg=/NODEFAULTLIB", "-C", "link-arg=/DEBUG:NONE",
                    "-C", f"link-arg=/MAP:{exe.with_suffix('.map')}"]
            res = self.app_res(app)
            if res:
                args += ["-C", f"link-arg={res}"]
            return args
        if self.target == "linux":
            return ["-C", f"linker={tool('ld.lld')}", "-C", "linker-flavor=ld", "-C", "relocation-model=static",
                    "-C", "target-feature=+crt-static", "-C", "link-arg=-static", "-C", "link-arg=-nostdlib",
                    "-C", "link-arg=-e", "-C", "link-arg=_start", "-C", "link-arg=--gc-sections",
                    "-C", "link-arg=-z", "-C", "link-arg=noexecstack", "-C", f"link-arg=-Map={exe}.map"]
        return []

    def build_bin(self, app, name, root, kind):
        """app-bins: one executable of apps/APP/BINS -> <dir>/NAME[.exe] (cli: no style, console
        subsystem; gui: the app's style)"""
        crates = source_crates(root)
        style = app_style(app) if kind == "gui" else None
        deps = crates + ([f"style_{style}"] if style else [])
        self.build_libs(deps)
        exe = self.dir / bin_file(name, self.target)
        key = f"bin_{app}_{name}"
        perms, assets, boot = self.boot_inputs(app, name)   # permissions, vfs: as the app's exe
        digest = self.src_hash(root, deps, extra=f"bin {kind} {style or ''} {'lto' if self.release else ''}" + boot)
        if self.up_to_date(key, digest, exe):
            return exe
        cmd = ["rustc"] + self.flags() + ["--crate-type", "bin", "--crate-name", name.replace("-", "_").lower(),
                                          "-C", "strip=symbols"]
        if self.release:
            cmd += ["-C", "lto"]
        else:
            cmd += ["-C", f"incremental={self.dir / 'inc' / key}"]
        for d in crates:
            cmd += ["--extern", f"{d}={self.rlib(d)}"]
        if style:
            cmd += ["--extern", f"style={self.rlib(f'style_{style}')}"]
        cmd += self.native_link_args(app, exe, "WINDOWS" if kind == "gui" else "CONSOLE")
        boot_args, env = self.boot_args(app, perms, assets)
        cmd += boot_args + ["-o", exe, root]
        run_cmd(cmd, env=env)
        pdb = self.dir / f"{name}.pdb"
        if pdb.exists():
            pdb.unlink()
        self.stamp(key, digest)
        return exe

    def app_res(self, name):
        """apps/NAME/res/app.ico -> the icon resource of the exe (as build.py does it)"""
        ico = ROOT / "apps" / name / "res" / "app.ico"
        if not ico.exists():
            return None
        objdir = self.dir / "res" / name
        objdir.mkdir(parents=True, exist_ok=True)
        res = objdir / "app.res"
        with self.res_lock:
            if not res.exists() or res.stat().st_mtime < ico.stat().st_mtime:
                rc = objdir / "app.rc"
                rc.write_text(f'1 ICON "{ico.as_posix()}"\n', encoding="utf-8")
                run_cmd([tool("llvm-rc"), "/nologo", "/fo", res, rc], quiet=True)
        return res


def make_builders(targets, out, release, jobs, profile=None, web_mode="auto", android_abi="arm64-v8a"):
    """The CLI and Build Manager use the same browser variant selection."""
    return [Builder(target, out, release, jobs, profile, mode, android_abi)
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
    for filename in ("loader.js", "worker.js", "bridge.js", "storage.js", "secrets.js", "handles.js", "a11y.js", "audio.js", "host.css"):
        shutil.copyfile(source / filename, directory / filename)
    html = (source / "index.html").read_text(encoding="utf-8").replace("{{APP}}", name)
    (directory / "index.html").write_bytes(html.encode("utf-8"))
    variants = {mode: name + suffix for mode, suffix in (("scalar", ".wasm"), ("simd", ".simd.wasm"), ("threads", ".threads.wasm"))
                if (directory / (name + suffix)).exists()}
    (directory / "variants.json").write_text(json.dumps(variants), encoding="utf-8")


def build_apps(builders, names, styles=None):
    """build apps for every target: the libraries of all targets side by side, and with them the
    apps' modules (which need none of them), then every app link in parallel (RUSTC_SLOTS bounds
    the processes) -> [(builder, name, exe or None)] in the order builders x names x styles.
    styles: None = each app's STYLE; several = one variant per style (Builder.build_app).  The
    first failure is raised after the running builds finished."""
    jobs = [(b, n, s) for b in builders for n in names for s in (styles or [app_style(n)])]
    variant = len(styles or []) > 1

    def libs(b):
        mine = [(n, s) for bb, n, s in jobs if bb is b and b.target in app_targets(n)]
        crates = {c for n, _ in mine for c in app_crates(n) + app_styles(n)} | {f"style_{s}" for _, s in mine}
        return b.build_libs(sorted(crates)) if mine else []
    with ThreadPoolExecutor(max(1, 2 * len(builders) * len(names))) as ex:
        mods = [ex.submit(build_app_modules, b, n) for b in builders for n in names
                if b.target != "web" and b.target in app_targets(n) and APP_MODULES.get(n)]
        mods += [ex.submit(build_app_bins, b, n) for b in builders for n in names       # app-bins
                 if b.target in ("win", "linux") and b.target in app_targets(n) and app_bins(n)]
        for f in [ex.submit(libs, b) for b in builders]:
            f.result()
        with ThreadPoolExecutor(max(1, len(jobs))) as ex2:
            futs = [(b, n, ex2.submit(b.build_app, n, s, variant)) for b, n, s in jobs]
            res = [(b, n, f.result()) for b, n, f in futs]
        for f in mods:
            f.result()
    return res


# ---------------------------------------------------------------- run / test
def wsl_path(p):
    p = str(Path(p).resolve())
    return "/mnt/" + p[0].lower() + p[2:].replace("\\", "/") if p[1:2] == ":" else p


def run_exe(target, exe, args, timeout=600):
    return run_native(target, exe, args, timeout)


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


# the test plugins of apps/PaintCraft/tests/plugins*.uis (`--plugins @ptest`): build.py's
# plugin_test_dir, filled from the asm reference plugins (<ref>/<target>/plugins, built by
# `python legacy/build.py plugin --out build/rwref`); never the real exe folder
PLUGIN_TEST_SET = {"Effects": ["swapchan", "sepiastr", "crash", "crashpf", "crashq", "toonew", "badinfo"],
                   "FileTypes": ["qoi"], "Shapes": ["pshapes"]}


def plugin_test_dir(base, target, ref=None):
    """<base>/<target>/ptest/{Effects,FileTypes,Shapes} from the reference plugins -> False when
    they are missing (the plugins scripts then fail on both sides)"""
    ref = Path(ref or os.environ.get("jfgui_REF") or ROOT / "build" / "rwref")
    src = ref / target / "plugins"
    ext = ".dll" if target == "win" else ".so"
    if not all((src / (n + ext)).exists() for names in PLUGIN_TEST_SET.values() for n in names):
        return False
    d = Path(base) / target / "ptest"
    for folder, names in PLUGIN_TEST_SET.items():
        (d / folder).mkdir(parents=True, exist_ok=True)
        for f in (d / folder).iterdir():
            f.unlink()
        for n in names:
            shutil.copy(src / (n + ext), d / folder)
    return True


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
            if name == "PaintCraft" and not plugin_test_dir(exe.parent.parent, b.target):
                print(f"[{b.target}] note: no reference plugins for PaintCraft's ptest (python legacy/build.py plugin --out build/rwref)")
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
        n, f = rust_plugin_scripts(b)               # rust-plugin: plugins*.uis with the Rust plugins
        total += n
        fails += f
    print(f"{total - len(fails)} of {total} scripts passed" + (f" ({skipped} pending)" if skipped else ""))
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
    # the website (tools/site.py --check): every page, internal link, anchor and image, the HTML, two
    # builds byte-identical; a temporary folder below build/
    r = subprocess.run([sys.executable, str(RS / "tools" / "site.py"), "--check", "--summary"], cwd=ROOT,
                       capture_output=True, text=True, encoding="utf-8", errors="replace", stdin=subprocess.DEVNULL)
    print(r.stdout.strip() + (("\n" + r.stderr.strip()) if r.returncode and r.stderr.strip() else ""))
    if r.returncode:
        fails.append("tools/site.py --check")
    if not quick:
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


def write_ide():
    """rust-project.json: rust-analyzer without Cargo (Windows target)"""
    sysroot = subprocess.run(["rustc", "--print", "sysroot"], capture_output=True, text=True).stdout.strip()
    libs = all_libs()
    bins = {f"bin:{a}/{b}": (a, root, kind) for a in apps() for b, root, kind in app_bins(a)}     # app-bins
    names = list(libs) + [f"app:{a}" for a in apps()] + list(bins)
    idx = {n: i for i, n in enumerate(names)}
    crates = []
    for n in names:
        if n.startswith("bin:"):
            a, root, kind = bins[n]
            deps = [{"crate": idx[d], "name": d} for d in source_crates(root)]
            if kind == "gui":
                deps.append({"crate": idx[f"style_{app_style(a)}"], "name": "style"})
        elif n.startswith("app:"):
            root = entrypoint(RS / "apps" / n[4:], "app")
            deps = [{"crate": idx[d], "name": d} for d in app_crates(n[4:]) + app_styles(n[4:])] + [{"crate": idx[f"style_{app_style(n[4:])}"], "name": "style"}]
        else:
            root = RS / libs[n][0]
            deps = [{"crate": idx[d], "name": d} for d in libs[n][1]]
        crates.append({"root_module": str(root), "edition": "2024", "deps": deps, "is_workspace_member": True,
                       "cfg": ["target_os=\"windows\"", "windows"], "target": TRIPLE["win"]})
    (RS / "rust-project.json").write_text(json.dumps({"sysroot": sysroot, "crates": crates}, indent=1))
    print(f"wrote rust-project.json ({len(crates)} crates)")


# ---------------------------------------------------------------- rust-plugin
# PaintCraft plugins written in Rust (plugins/README.md; ABI crate lib/plugin):
#   python build_rs.py plugin [NAME] [--target win|linux|both] [--out DIR]
# plugins/NAME/lib.rs -> <out>/<t>/plugins/NAME.dll | NAME.so: a no_std cdylib that depends
# only on the crate `plugin` (+ core), panic=abort with the plugin's own #[panic_handler], fat
# LTO (core included: only what the plugin uses), linked against nothing -- Windows: lld-link
# /NOENTRY /NODEFAULTLIB (no imports, export pn_plugin_query only); Linux: ld.lld -shared
# -Bsymbolic --no-undefined (no DT_NEEDED, only RELATIVE relocations).  Each binary is then
# checked the way PaintCraft's loader checks it (plugin_verify: lib/plat/win_lib.rs,
# linux_elfload.rs) -- a plugin the loader would reject fails the build.  The plugin crate's
# rlib lives in <out>/<t>/plugin_lib.  `test` also runs apps/PaintCraft/tests/plugins*.uis on the
# Rust PaintCraft with the Rust plugins (<out>/<t>/rsptest, the layout of PLUGIN_TEST_SET).
PLUGIN_LIB_ROOT = RS / "lib" / "plugin" / "lib.rs"
PLUGINS_RS = RS / "plugins"


def rs_plugin_names():
    return sorted(p.name for p in PLUGINS_RS.iterdir() if p.is_dir() and entrypoint(p, "lib", False)) if PLUGINS_RS.is_dir() else []


def plugin_lint():
    """the rules of lint() for the plugin crate and every plugins/NAME: #![no_std]; plugins
    #![deny(unsafe_code)] (entry!() is their one exception) or forbid; alloc / core only"""
    bad = []
    crates = [(resolve_root(PLUGIN_LIB_ROOT), True)] + [(entrypoint(PLUGINS_RS / n, "lib"), False) for n in rs_plugin_names()]
    for root, is_lib in crates:
        if not root.exists():
            continue
        head = root.read_text(encoding="utf-8")
        if "#![no_std]" not in head:
            bad.append(f"{root.relative_to(ROOT)}: missing #![no_std]")
        if not is_lib and "#![deny(unsafe_code)]" not in head and "#![forbid(unsafe_code)]" not in head:
            bad.append(f"{root.relative_to(ROOT)}: missing #![deny(unsafe_code)]")
        for f in project_sources(root):
            for i, ln in enumerate(f.read_text(encoding="utf-8").splitlines(), 1):
                s = ln.strip()
                if s.startswith("extern crate") and s not in ("extern crate alloc;", "extern crate core;"):
                    bad.append(f"{f.relative_to(ROOT)}:{i}: only alloc / core may be extern crates")
                if s.startswith("use std") or " std::" in s.split("//")[0]:
                    bad.append(f"{f.relative_to(ROOT)}:{i}: std is not available (no_std)")
                if ln[:2] in ('")', '",', '";'):
                    bad.append(f"{f.relative_to(ROOT)}:{i}: a string literal broken by a line end (a script edit?)")
    return bad


class PluginBuilder:
    """the plugin crate (rlib) and the plugins of one target"""

    def __init__(self, target, out):
        self.target = target
        self.dir = out / target / "plugins"
        self.lib_dir = out / target / "plugin_lib"
        self.dir.mkdir(parents=True, exist_ok=True)
        self.lib_dir.mkdir(parents=True, exist_ok=True)
        self.lib_digest = None

    def flags(self):
        f = ["--edition", "2024", "--target", TRIPLE[self.target], "-C", "panic=abort", "-C", "debuginfo=0",
             "-D", "warnings", "-C", "opt-level=3", "-C", "codegen-units=1", f"--remap-path-prefix={ROOT}{os.sep}="]
        if self.target == "linux":
            f += ["-C", "relocation-model=pic"]
        return f

    def link_flags(self):
        if self.target == "win":
            args = ["/NOENTRY", "/NODEFAULTLIB", "/DEBUG:NONE", "/Brepro"]
            return ["-C", f"linker={tool('lld-link')}"] + [x for a in args for x in ("-C", "link-arg=" + a)]
        args = ["-Bsymbolic", "--no-undefined", "--hash-style=both", "-z", "noexecstack", "-z", "now", "--build-id=none"]
        return ["-C", f"linker={tool('ld.lld')}", "-C", "linker-flavor=ld"] + [x for a in args for x in ("-C", "link-arg=" + a)]

    @staticmethod
    def digest(files, extra):
        h = hashlib.sha1()
        for f in files:
            h.update(Path(os.path.relpath(f, ROOT)).as_posix().encode())
            h.update(f.read_bytes())
        h.update(extra.encode())
        return h.hexdigest()

    def rlib(self):
        return self.lib_dir / "libplugin.rlib"

    def build_lib(self):
        if self.lib_digest:
            return self.lib_digest
        root = resolve_root(PLUGIN_LIB_ROOT)
        d = self.digest(project_sources(root, assets=True), " ".join(self.flags()))
        st = self.lib_dir / "plugin.stamp"
        if not (self.rlib().exists() and st.exists() and st.read_text() == d):
            run_cmd(["rustc"] + self.flags() + ["--crate-type", "rlib", "--crate-name", "plugin",
                                                "-o", self.rlib(), root])
            st.write_text(d)
        self.lib_digest = d
        return d

    def path(self, name):
        return self.dir / (name + (".dll" if self.target == "win" else ".so"))

    def build(self, name):
        """plugins/NAME -> the checked plugin library (rebuilt when its sources, the plugin
        crate or the flags changed)"""
        root = entrypoint(PLUGINS_RS / name, "lib")
        lib = self.build_lib()
        d = self.digest(project_sources(root, assets=True), lib + " ".join(self.flags() + self.link_flags()))
        out = self.path(name)
        st = self.lib_dir / f"{name}.stamp"
        if out.exists() and st.exists() and st.read_text() == d:
            return out
        cmd = ["rustc"] + self.flags() + ["--crate-type", "cdylib", "--crate-name", name, "-C", "lto=fat",
                                          "-C", "strip=symbols", "--extern", f"plugin={self.rlib()}",
                                          "-L", f"dependency={self.lib_dir}"] + self.link_flags() + ["-o", out, root]
        run_cmd(cmd)
        for extra in (out.parent / (out.name + ".lib"), out.parent / (out.name + ".exp"), out.with_suffix(".pdb"),
                      out.with_suffix(".lib"), out.with_suffix(".exp")):
            if extra.exists():                      # import library / PDB: not needed
                extra.unlink()
        bad = plugin_verify(out, self.target)
        if bad:
            out.unlink()
            raise BuildError(f"plugin {name} ({self.target}) would be rejected by PaintCraft's loader:\n  " + "\n  ".join(bad))
        st.write_text(d)
        return out


def plugin_verify(path, target, export="pn_plugin_query"):
    """the checks of PaintCraft's plugin loader (and the rules of PLUGINS.md §8) -> problems
    (jxl-module: `export` = the one export expected, jxlenc_module for the JPEG XL module)"""
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


def pe_verify(b, export="pn_plugin_query"):
    """win_lib.rs pe_check + lib_sym: PE32+ x86-64 DLL, no imports / delay imports / TLS, no entry
    point, the section table inside the first 4 KB of headers, <= 15 sections, no W+X section,
    one export: pn_plugin_query in an executable section"""
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


def elf_verify(b, export="pn_plugin_query"):
    """linux_elfload.rs elf_map / elf_load / elf_rela / lib_sym: ELF64 LE ET_DYN x86-64, 1..64
    program headers, <= 16 PT_LOAD ascending and page-disjoint, never W+X, inside 256 MB, one
    PT_DYNAMIC, no PT_INTERP / PT_TLS, no DT_NEEDED / DT_INIT / init arrays / DT_REL, RELA
    relocations NONE / RELATIVE / 64 / GLOB_DAT / JUMP_SLOT against defined symbols, the one
    defined dynamic symbol pn_plugin_query in an executable segment"""
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


def build_plugins(targets, out, name):
    for t in targets:
        pb = PluginBuilder(t, out)
        for n in [name] if name else rs_plugin_names():
            p = pb.build(n)
            print(f"[{t}] plugin {n}: {p.relative_to(ROOT) if p.is_relative_to(ROOT) else p} ({p.stat().st_size} bytes)")


def rust_plugin_scripts(b):
    """apps/PaintCraft/tests/plugins*.uis on the Rust PaintCraft with the Rust ports of the asm
    plugins in <out>/<t>/rsptest (`--plugins=@ptest` -> `@rsptest`), then plugins/tests/*.uis
    with the other Rust plugins in <out>/<t>/rsplug/Effects -> (scripts run, failed tags);
    nothing while PaintCraft or one of the plugins of PLUGIN_TEST_SET is missing"""
    exe = b.dir / ("PaintCraft.exe" if b.target == "win" else "PaintCraft")
    names = [n for ns in PLUGIN_TEST_SET.values() for n in ns]
    if not exe.exists() or not all(entrypoint(PLUGINS_RS / n, "lib", False) for n in names):
        return 0, []
    pb = PluginBuilder(b.target, b.dir.parent)
    sets = {"rsptest": PLUGIN_TEST_SET, "rsplug": {"Effects": [n for n in rs_plugin_names() if n not in names]}}
    for dname, layout in sets.items():
        for folder, ns in layout.items():
            (b.dir / dname / folder).mkdir(parents=True, exist_ok=True)
            for f in (b.dir / dname / folder).iterdir():
                f.unlink()
            for n in ns:
                shutil.copy(pb.build(n), b.dir / dname / folder)
    total, fails = 0, []
    for uis in sorted((ROOT / "apps" / "PaintCraft" / "tests").glob("plugins*.uis")) + sorted((PLUGINS_RS / "tests").glob("*.uis")):
        args = [a.replace("--plugins=@ptest", "--plugins=@rsptest") for a in script_args(uis)]
        total += 1
        r = run_exe(b.target, exe, args + ["--script", uis.relative_to(ROOT).as_posix()])
        tag = f"[{b.target}] PaintCraft/{uis.name} (Rust plugins)" if uis.parent.parent != PLUGINS_RS else \
            f"[{b.target}] PaintCraft/rs/plugins/tests/{uis.name}"
        if r.returncode != 0:
            fails.append(tag)
            print(f"FAIL {tag} (exit {r.returncode})")
            for ln in (r.stdout + r.stderr).splitlines():
                if ln.startswith(("FAIL", "error", "panic", "UIWARN")):
                    print("   " + ln)
    return total, fails
# ---------------------------------------------------------------- end rust-plugin


# ---------------------------------------------------------------- jxl-module
# Loadable modules (rewrite/AGENTS.md "JPEG XL encoder as a loadable module"; the ABI in
# lib/plat/modabi.rs): modules/NAME -> <app folder>/NAME.dll | NAME.so, built with every
# app that loads it (APP_MODULES: PaintCraft loads jxlenc, the JPEG XL writer), in the Builder's
# folder with its flags (dev, --release, --profile).  The library crates a module contains
# (MODULE_LIBS[NAME] and its dependencies from LIBS) are compiled once more into
# <out>/<t>/mod_NAME with modules/modrt passed as `rt` and modules/modplat as `plat` (the
# module runtime: the host's allocator, worker pool and crash report through a function table),
# then the cdylib like a Rust plugin: panic=abort, fat LTO, linked against nothing (lld-link
# /NOENTRY /NODEFAULTLIB; ld.lld -shared -Bsymbolic --no-undefined), a .map next to it, and
# checked the way PaintCraft's loader checks a library (plugin_verify with the export
# NAME_module).  Lint: #![no_std]; modrt / modplat may contain unsafe (the module's rt / plat),
# a module crate #![deny(unsafe_code)] with module_entry!() as its one exception.
MODULES_RS = RS / "modules"
MOD_RT = {"rt": "modrt", "plat": "modplat"}         # the crates a module gets as rt / plat
MODULE_LIBS = {"jxlenc": "img_jxl_enc"}             # module -> the library crate it wraps
APP_MODULES = {"PaintCraft": ["jxlenc"]}              # the modules an app's folder gets


def module_file(name, target):
    return name + (".dll" if target == "win" else ".so")


def module_lint():
    """lint() for modules/*: #![no_std]; modules #![deny(unsafe_code)]; alloc / core only"""
    bad = []
    if not MODULES_RS.is_dir():
        return bad
    for d in sorted(p for p in MODULES_RS.iterdir() if p.is_dir() and entrypoint(p, "lib", False)):
        root = entrypoint(d, "lib")
        head = root.read_text(encoding="utf-8")
        if "#![no_std]" not in head:
            bad.append(f"{root.relative_to(ROOT)}: missing #![no_std]")
        if d.name not in MOD_RT.values() and "#![deny(unsafe_code)]" not in head and "#![forbid(unsafe_code)]" not in head:
            bad.append(f"{root.relative_to(ROOT)}: missing #![deny(unsafe_code)]")
        for f in project_sources(root):
            for i, ln in enumerate(f.read_text(encoding="utf-8").splitlines(), 1):
                s = ln.strip()
                if s.startswith("extern crate") and s not in ("extern crate alloc;", "extern crate core;"):
                    bad.append(f"{f.relative_to(ROOT)}:{i}: only alloc / core may be extern crates")
                if s.startswith("use std") or " std::" in s.split("//")[0]:
                    bad.append(f"{f.relative_to(ROOT)}:{i}: std is not available (no_std)")
    return bad


class ModuleBuilder:
    """the library crates of one module (over modrt / modplat) and the module of one Builder"""

    def __init__(self, b, name):
        self.b = b
        self.name = name
        self.dir = b.dir / f"mod_{name}"
        (self.dir / ".stamp").mkdir(parents=True, exist_ok=True)
        self.digests = {}

    def crates(self):
        """[(crate, root, deps as crate names, deps as extern names)] in build order"""
        libs = all_libs()
        order = []

        def add(n):
            if n not in order:
                for d in libs[n][1]:
                    add(d)
                order.append(n)
        add(MODULE_LIBS[self.name])
        out = []
        for n in order:
            if n in MOD_RT:
                m = MOD_RT[n]
                deps = ["rt"] if m == "modplat" else []
                root = entrypoint(MODULES_RS / m, "lib").relative_to(RS).as_posix()
                out.append((m, root, [MOD_RT[d] for d in deps], deps))
            else:
                out.append((n, libs[n][0], [MOD_RT.get(d, d) for d in libs[n][1]], libs[n][1]))
        return out

    def flags(self, name):
        return [f"dependency={self.dir}" if x == f"dependency={self.b.dir}" else x for x in self.b.flags(name)]

    def rlib(self, crate):
        return self.dir / f"lib{crate}.rlib"

    def digest(self, crate, root, deps, extra=""):
        h = hashlib.sha1()
        files = project_sources(RS / root, assets=True)
        for f in files:
            h.update(f.relative_to(RS).as_posix().encode())
            h.update(f.read_bytes())
        for d in deps:
            h.update(self.digests[d].encode())
        h.update(" ".join(self.flags(crate)).encode())
        h.update(extra.encode())
        return h.hexdigest()

    def fresh(self, key, digest, out):
        st = self.dir / ".stamp" / key
        return out.exists() and st.exists() and st.read_text() == digest

    def build(self):
        """the module file in the app's folder (rebuilt when a source, a flag or a crate changed)"""
        info = {c[0]: c for c in self.crates()}

        def one(crate, ready):
            _, root, deps, names = info[crate]
            d = self.digest(crate, root, deps)
            self.digests[crate] = d
            out = self.rlib(crate)
            if self.fresh(crate, d, out):
                return False
            cmd = ["rustc"] + self.flags(crate) + ["--crate-type", "rlib", "--crate-name", crate]
            if not self.b.release:
                cmd += ["-C", f"incremental={self.dir / 'inc' / crate}"]
            compile_rlib(cmd, RS / root, out, [(nm, self.rlib(dep)) for dep, nm in zip(deps, names)],
                         self.b.piped, ready)
            if compute_crate(crate) and crate != "simd":
                check_intrinsics(crate, out)
            (self.dir / ".stamp" / crate).write_text(d)
            return True
        run_dag({c: (info[c][2], one) for c in info})     # each crate once its dependencies are done
        t = self.b.target
        out = self.b.dir / module_file(self.name, t)
        root = entrypoint(MODULES_RS / self.name, "lib")
        top = [c for c, _, _, _ in self.crates()]
        names = [{v: k for k, v in MOD_RT.items()}.get(c, c) for c in top]
        pb = PluginBuilder.__new__(PluginBuilder)   # its link flags (no imports, no entry)
        pb.target = t
        flags = self.flags(MODULE_LIBS[self.name])  # a compute crate's opt-level for the LTO step
        if t == "linux":
            flags += ["-C", "relocation-model=pic"]
        mapf = out.parent / (out.stem + ".map" if t == "win" else out.name + ".map")
        link = pb.link_flags() + ["-C", f"link-arg={'/MAP:' if t == 'win' else '-Map='}{mapf}"]
        h = hashlib.sha1(("".join(self.digests[c] for c in top) + " ".join(flags + link)).encode())
        for source in project_sources(root, assets=True):
            h.update(source.relative_to(RS).as_posix().encode())
            h.update(source.read_bytes())
        d = h.hexdigest()
        if self.fresh("module", d, out):
            return out
        cmd = ["rustc"] + flags + ["--crate-type", "cdylib", "--crate-name", self.name, "-C", "lto=fat",
                                   "-C", "strip=symbols"]
        for c, nm in zip(top, names):
            cmd += ["--extern", f"{nm}={self.rlib(c)}"]
        cmd += link + ["-o", out, root]
        run_cmd(cmd)
        for extra in (out.parent / (out.name + ".lib"), out.parent / (out.name + ".exp"), out.with_suffix(".pdb"),
                      out.with_suffix(".lib"), out.with_suffix(".exp")):
            if extra.exists():                      # import library / PDB: not needed
                extra.unlink()
        bad = plugin_verify(out, t, export=f"{self.name}_module")
        if bad:
            out.unlink()
            raise BuildError(f"module {self.name} ({t}) would be rejected by the loader:\n  " + "\n  ".join(bad))
        (self.dir / ".stamp" / "module").write_text(d)
        return out


def build_app_modules(b, app, vdir=None):
    """the modules of APP_MODULES[app] into the app's folder (Builder b; once per run, also
    when several style variants link at once), copied next to a variant's exe (vdir)"""
    if b.target == "web":
        return
    with b.mod_lock:
        if app not in b.mod_ready:
            for m in APP_MODULES.get(app, []):
                ModuleBuilder(b, m).build()
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
    ap.add_argument("cmd", choices=["lib", "app", "all", "run", "test", "check", "ide", "plugin",
                                    "new", "api-diff", "changelog", "release", "site", "i18n"])
    ap.add_argument("name", nargs="?")
    ap.add_argument("--target", choices=["win", "linux", "both", "web", "all", "android"])
    ap.add_argument("--android-abi", choices=["arm64-v8a", "x86_64"], default="arm64-v8a",
                    help="Android native ABI (default ARM64; x86_64 for emulator tests)")
    ap.add_argument("--style")
    ap.add_argument("--web-mode", choices=["auto", "scalar", "simd", "threads"], default="auto",
                    help="browser bundle: auto builds scalar, SIMD128 and shared-memory worker variants")
    ap.add_argument("--release", action="store_true")
    ap.add_argument("--profile", choices=sorted(PROFILES),   # r13-measure: implies --release
                    help="release variant: size (z for UI crates, 2 for compute + app, LTO) or o2 (2, no LTO)")
    ap.add_argument("--out", help="output folder (default build; --release: build/release; --profile P: build/P)")
    ap.add_argument("-j", type=int, default=os.cpu_count() or 8,
                    help="rustc processes at once (libraries and app links; default: the logical CPUs)")
    ap.add_argument("--quick", action="store_true")
    ap.add_argument("--bin", help="run: one of the app's further executables (apps/NAME/BINS)")
    # new, api-diff, changelog, release (dev_command)
    ap.add_argument("--kind", choices=["window", "console", "tool", "service"], help="new: the kind of app")
    ap.add_argument("--title", help="new: the app's title (default: its name)")
    ap.add_argument("--lib", action="store_true", help="new: a library crate under lib/ instead of an app")
    ap.add_argument("--deps", help="new --lib: the crates of LIBS it uses (rt,plat)")
    ap.add_argument("--against", help="api-diff: the release to compare with (default: the newest)")
    ap.add_argument("--snapshot", action="store_true", help="api-diff: write docs/api/<lib/VERSION>.txt")
    ap.add_argument("--all", action="store_true", help="api-diff: list every change")
    ap.add_argument("--type", default="added", help="changelog: added|changed|deprecated|removed|fixed|security")
    ap.add_argument("--area", help="changelog: the part the entry is about (ui, plat, build ...)")
    ap.add_argument("--date", help="release: the release date (default today)")
    ap.add_argument("--force", action="store_true", help="release: also when the API changes need a larger step")
    ap.add_argument("--check", action="store_true", help="site: build into a temporary folder and check it; i18n: errors only")
    ap.add_argument("--update", action="store_true", help="i18n: add the missing texts to the app's catalogue (untranslated)")
    ap.add_argument("--unused", action="store_true", help="i18n: list the unused rows of every app")
    a = ap.parse_args(argv)
    if a.cmd == "ide":
        write_ide()
        return 0
    if a.cmd == "site":                             # tools/site.py (its own process: `site` is a stdlib name)
        return subprocess.run([sys.executable, str(ROOT / "tools" / "site.py")]
                              + (["--out", str((ROOT / a.out).resolve())] if a.out else [])
                              + (["--check"] if a.check else []), cwd=ROOT).returncode
    if a.cmd == "i18n":                             # tools/i18n_check.py
        if a.update and not a.name:
            print("i18n: --update needs the app (python build_rs.py i18n PaintCraft --update)")
            return 1
        return subprocess.run([sys.executable, str(ROOT / "tools" / "i18n_check.py")] + ([a.name] if a.name else [])
                              + [f for f, on in (("--update", a.update), ("--check", a.check), ("--unused", a.unused)) if on],
                              cwd=ROOT).returncode
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
    if "android" in targets and a.cmd not in ("app", "lib"):
        raise BuildError("Android supports app/lib builds; install the resulting APK with adb install -r FILE.apk.")
    if a.cmd in ("run", "test", "plugin") and "web" in targets:
        raise BuildError("Use app/all --target web to build browser bundles; serve their index.html for browser tests. Native run/test/plugin requires win/linux/both.")
    out = ((ROOT / a.out).resolve() if a.out else ROOT / "build" / a.profile if a.profile
           else ROOT / "build" / "release" if a.release else ROOT / "build")
    builders = make_builders(targets, out, a.release, a.j, a.profile, a.web_mode, a.android_abi)
    global RUSTC_SLOTS
    RUSTC_SLOTS = threading.BoundedSemaphore(max(1, a.j))
    t0 = time.perf_counter()
    try:
        if a.cmd == "lib":
            with ThreadPoolExecutor(len(builders)) as ex:
                res = [(b, ex.submit(b.build_libs, list(all_libs()))) for b in builders]
                for b, f in res:
                    built = f.result()
                    print(f"[{b.target}] libraries: {len(built)} rebuilt ({', '.join(built) or 'none'})")
        elif a.cmd in ("app", "run"):
            if not a.name:
                raise BuildError("app name missing")
            styles = a.style.split(",") if a.style else None
            bad = [s for s in styles or [] if s not in STYLES]
            if bad:
                raise BuildError(f"unknown style {', '.join(bad)} (styles: {', '.join(STYLES)})")
            for b, n, exe in build_apps(builders, [a.name], styles):
                if exe:
                    print(f"[{b.target}] app {n}: {exe.relative_to(ROOT)} ({exe.stat().st_size} bytes)")
            if a.cmd == "run":
                if a.bin:                           # app-bins
                    if a.bin not in [n for n, _, _ in app_bins(a.name)]:
                        raise BuildError(f"{a.name} has no executable {a.bin} (apps/{a.name}/BINS)")
                    exe = exe.parent / bin_file(a.bin, targets[0])
                r = run_exe(targets[0], exe, extra, timeout=None)
                sys.stdout.write(r.stdout)
                sys.stderr.write(r.stderr)
                return r.returncode
        elif a.cmd == "all":
            for b, n, exe in build_apps(builders, apps()):
                if exe:
                    print(f"[{b.target}] app {n}: {exe.relative_to(ROOT)} ({exe.stat().st_size} bytes)")
        elif a.cmd == "plugin":                     # rust-plugin
            build_plugins(targets, out, a.name)
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
