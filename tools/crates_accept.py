#!/usr/bin/env python3
"""crates_accept.py -- does a project outside this repository build with the crates alone?

    python tools/crates_accept.py [--targets win,linux,web] [--out DIR]
    python tools/crates_accept.py --template     write the files of lib/jflib/template that
                                                 come from apps/template (build_rs.py check
                                                 compares them)

The acceptance test of docs/CRATES.md (CR2), run by the full suite (`build_rs.py test` without
--quick).  It packs every crate as `cargo package` would publish it, unpacks the packages into a
local source next to the crates from outside (`cargo vendor`), and builds two projects that know
only `jflib = "0.1"`, offline, with nothing of this repository on their paths:

  hallo   the template lib/jflib/template: Windows (development and release, through rust-lld,
          also as on a machine without Visual Studio's SDK), Linux against glibc; tests/smoke.uis
          runs on both.  Its icon must be the .res llvm-rc makes of it.
  probe   a command-line program with PERMISSIONS, PACKAGE and assets/: Windows and one static
          Linux file; it prints what reached it (app://, a refused write, its version and name).
          Its bundle must be tools/assetpack.py's byte for byte.

  hallo2  made by `cargo jflib new` with cargo-jflib installed from the packages: `cargo jflib
          test` on Windows, `cargo jflib web` (scalar, SIMD, threads) and a click in the browser
          (Playwright, tools/tests/crates_web.mjs; skipped without it).

Then `cargo test -p jflib-build` and `-p cargo-jflib`.  Output below build/crates_accept; exit 1 when something failed.
"""
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tarfile
import time
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
ROOT = TOOLS.parent
TEMPLATE = ROOT / "lib" / "jflib" / "template"
FACADE_DOCS = ROOT / "lib" / "jflib" / "docs"
WIN, LINUX, WEB = "x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu", "wasm32-unknown-unknown"


# ---------------------------------------------------------------- the template
def derived_template():
    """the files of lib/jflib/template that come from apps/template -> {relative path: text}"""
    app = ROOT / "apps" / "template"
    src = (app / "main.rs").read_text(encoding="utf-8").replace("\r\n", "\n")
    head, sep, body = src.partition("#![no_std]\n")
    assert sep and "#![no_main]\n" in body and "extern crate alloc;\n" in body, "apps/template/main.rs changed its shape"
    lib = ("// hallo -- a JFLib app with a window: the template of the crate jflib, from apps/template of the\n"
           "// JFLib repository (README.md of jflib).\n"
           "//\n"
           "// Build and run:   cargo run\n"
           "// Test script:     target/debug/hallo --script tests/smoke.uis\n"
           "//\n"
           + "".join(ln for ln in head.splitlines(True)[6:])
           + sep + body.replace("#![no_main]\n", "", 1).replace("extern crate alloc;\n", "extern crate alloc;\n\nuse jflib::{style, ui};\n", 1))
    smoke = (app / "tests" / "smoke.uis").read_text(encoding="utf-8").replace("\r\n", "\n")
    assert "shot build/template.png" in smoke
    return {
        "src/lib.rs": lib,
        "src/locales/ui.tsv": (app / "locales" / "ui.tsv").read_text(encoding="utf-8").replace("\r\n", "\n"),
        "tests/smoke.uis": smoke.replace("# template smoke test", "# hallo smoke test").replace("shot build/template.png", "shot target/smoke.png"),
    }


def template_problems():
    """build_rs.py check: the derived files of the template are current -> problems"""
    bad = []
    for rel, text in derived_template().items():
        p = TEMPLATE / rel
        if not p.is_file() or p.read_bytes().decode("utf-8").replace("\r\n", "\n") != text:
            bad.append(f"lib/jflib/template/{rel}: differs from apps/template: python tools/crates_accept.py --template")
    return bad


def write_template():
    for rel, text in derived_template().items():
        p = TEMPLATE / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(text.encode("utf-8"))
        print(f"wrote lib/jflib/template/{rel}")


# ---------------------------------------------------------------- packing
def facade_docs():
    """docs/GUIDE.md and docs/API.md into the facade's package (lib/jflib/docs, not in git: the
    facade's include names it), where an agent finds them below ~/.cargo/registry/src"""
    FACADE_DOCS.mkdir(exist_ok=True)
    for name in ("GUIDE.md", "API.md"):
        shutil.copyfile(ROOT / "docs" / name, FACADE_DOCS / name)


class Run:
    def __init__(self, out):
        self.out = out
        self.fails = []
        self.notes = []
        self.log = out / "log"
        self.log.mkdir(parents=True, exist_ok=True)

    def cmd(self, name, args, cwd, env=None, timeout=3600):
        """a command, its output in log/NAME.log -> True when it exits 0"""
        start = time.monotonic()
        full = dict(os.environ, **(env or {}))
        r = subprocess.run([str(a) for a in args], cwd=cwd, env=full, capture_output=True, text=True, encoding="utf-8",
                           errors="replace", timeout=timeout, stdin=subprocess.DEVNULL)
        (self.log / f"{name}.log").write_text(f"$ {' '.join(map(str, args))}\n{r.stdout}\n{r.stderr}", encoding="utf-8")
        took = time.monotonic() - start
        if r.returncode:
            tail = [ln for ln in (r.stdout + r.stderr).splitlines() if ln.startswith(("error", "FAIL", "warning: "))][:8]
            self.fail(f"{name} (exit {r.returncode}, {took:.0f} s; log/{name}.log)" + "".join("\n    " + ln for ln in tail))
            return False
        print(f"ok   {name} ({took:.0f} s)", flush=True)
        return r

    def fail(self, what):
        self.fails.append(what)
        print(f"FAIL {what}", flush=True)

    def check(self, ok, what):
        if ok:
            print(f"ok   {what}", flush=True)
        else:
            self.fail(what)


def pack(run):
    """the packages and the crates from outside as one local source -> its folder"""
    facade_docs()
    target = run.out / "target-package"
    from tools import manifests
    published = [m["package"]["name"] for m in (manifests.read(ROOT / p / "Cargo.toml") for p in manifests.workspace()["members"])
                 if m["package"].get("publish", True)]
    if not run.cmd("package", ["cargo", "package", *(x for p in published for x in ("-p", p)), "--no-verify",
                               "--allow-dirty", "--offline", "--target-dir", target], ROOT):
        return None
    vendor = run.out / "vendor"
    shutil.rmtree(vendor, ignore_errors=True)
    if not run.cmd("vendor", ["cargo", "vendor", "--offline", "--versioned-dirs", vendor], ROOT):
        return None
    crates = sorted((target / "package").glob("*.crate"))
    for crate in crates:
        with tarfile.open(crate) as t:
            t.extractall(vendor, filter="data")
        (vendor / crate.stem / ".cargo-checksum.json").write_text(
            json.dumps({"files": {}, "package": hashlib.sha256(crate.read_bytes()).hexdigest()}), encoding="utf-8")
    run.check(len(crates) >= 57, f"{len(crates)} packages, {sum(c.stat().st_size for c in crates) / 1e6:.1f} MB")
    facade = vendor / f"jflib-{crates[0].stem.rsplit('-', 1)[1]}"
    run.check((facade / "docs" / "API.md").is_file() and (facade / "template" / "Cargo.toml.in").is_file(),
              "the facade's package holds docs/ and template/")
    return vendor


def source_config(vendor):
    return (f'\n# crates_accept.py: the packages instead of crates.io\n[source.crates-io]\nreplace-with = "packed"\n'
            f'[source.packed]\ndirectory = "{vendor.as_posix()}"\n[net]\noffline = true\n')


# ---------------------------------------------------------------- the projects
def make_hallo(run, vendor):
    d = run.out / "hallo"
    shutil.rmtree(d, ignore_errors=True)
    shutil.copytree(TEMPLATE, d)
    (d / "Cargo.toml.in").rename(d / "Cargo.toml")
    cfg = d / ".cargo" / "config.toml"
    cfg.write_text(cfg.read_text(encoding="utf-8") + source_config(vendor), encoding="utf-8")
    ico = ROOT / "apps" / "Kritzel" / "res" / "app.ico"
    (d / "res").mkdir()
    shutil.copyfile(ico, d / "res" / "app.ico")
    (d / "target").mkdir()                       # tests/smoke.uis writes its shot there; the build goes elsewhere
    return d


PROBE = r'''// probe -- what jflib-build hands to rt::main!: app://, the manifest, the version, the name
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use jflib::{plat, rt};

#[cfg(windows)]
const OUTSIDE: &str = "C:/Windows/Temp/jflib-crates-probe.txt";
#[cfg(not(windows))]
const OUTSIDE: &str = "/tmp/jflib-crates-probe.txt";

rt::main!({
    let assets = plat::read_file("app://hallo.txt").as_deref() == Some(&b"Hallo aus app://"[..]);
    let big = plat::read_file("app://sub/big.txt").map_or(0, |b| b.len());
    let denied = !plat::write_file(OUTSIDE, b"x");
    let line = alloc::format!("assets={assets} big={big} denied={denied} version={} app={}\n", rt::app_version!(),
                              option_env!("JFGUI_APP").unwrap_or("?"));
    plat::write_stdout(line.as_bytes());
    0
});
'''


def make_probe(run, vendor):
    d = run.out / "probe"
    shutil.rmtree(d, ignore_errors=True)
    (d / "src").mkdir(parents=True)
    (d / ".cargo").mkdir()
    (d / "assets" / "sub").mkdir(parents=True)
    toml = (TEMPLATE / "Cargo.toml.in").read_text(encoding="utf-8").replace('name = "hallo"', 'name = "probe"').replace(
        'version = "0.1.0"', 'version = "2.5.0"')
    (d / "Cargo.toml").write_text(toml, encoding="utf-8")
    (d / "build.rs").write_text("fn main() {\n    jflib_build::console();\n}\n", encoding="utf-8")
    (d / "src" / "main.rs").write_text(PROBE, encoding="utf-8")
    (d / "PERMISSIONS").write_text("# probe: the files the user opens, nothing else\nfs.user\n", encoding="utf-8")
    (d / "PACKAGE").write_text("display=Probe\nversion=2.5.0\napp_id=org.example.Probe\n", encoding="utf-8")
    (d / "assets" / "hallo.txt").write_bytes(b"Hallo aus app://")
    (d / "assets" / "sub" / "big.txt").write_bytes(b"Eine Zeile, die sich wiederholt.\n" * 300)
    (d / "assets" / ".hidden").write_bytes(b"not in the bundle")
    (d / ".cargo" / "config.toml").write_text(
        f'[target.{WIN}]\nlinker = "rust-lld"\n\n'
        f'# one static file\n[target.{LINUX}]\n'
        'rustflags = ["-C", "linker=rust-lld", "-C", "linker-flavor=ld.lld", "-C", "relocation-model=static", "-C", "target-feature=+crt-static"]\n'
        + source_config(vendor), encoding="utf-8")
    return d


def out_file(project, target_dir, triple, profile, name):
    """a file the build script of the project wrote to its OUT_DIR"""
    found = sorted((target_dir / triple / profile / "build").glob(f"{project}-*/out/{name}"), key=lambda p: p.stat().st_mtime)
    return found[-1] if found else None


def accept(targets, out):
    sys.path.insert(0, str(ROOT))
    sys.dont_write_bytecode = True
    import build_rs
    from tools import assetpack
    from tools.host_exec import linux_path, run_linux
    run = Run(out)
    vendor = pack(run)
    if vendor is None:
        return run
    hallo, probe = make_hallo(run, vendor), make_probe(run, vendor)
    target_dir = out / "target"                  # shared: the probe finds the crates compiled for hallo
    env = {"CARGO_TARGET_DIR": str(target_dir)}
    shutil.rmtree(target_dir / WIN / "debug" / "build", ignore_errors=True)
    # as on a machine without Visual Studio and Windows SDK: lld finds them on its own even without
    # LIB (/lldignoreenv alone does not stop that), an empty /winsysroot does; a crate that links an
    # import library (#[link] without kind = "raw-dylib") then fails with "could not open X.lib"
    no_sdk = out / "no-sdk"
    no_sdk.mkdir(exist_ok=True)
    novs = ["--config", f"target.{WIN}.rustflags=['-C', 'link-arg=/lldignoreenv', '-C', 'link-arg=/winsysroot:{no_sdk.as_posix()}']"]
    if "win" in targets:
        if run.cmd("hallo-win-dev-nosdk", ["cargo", "build", "--target", WIN] + novs, hallo, env):
            exe = target_dir / WIN / "debug" / "hallo.exe"
            run.cmd("hallo-win-dev-smoke", [exe, "--script", "tests/smoke.uis"], hallo)
            res = out_file("hallo", target_dir, WIN, "debug", "app.res")
            rc = out / "llvm-rc"
            rc.mkdir(exist_ok=True)
            (rc / "app.rc").write_text(f'1 ICON "{(hallo / "res" / "app.ico").as_posix()}"\n', encoding="utf-8")
            if run.cmd("llvm-rc", [build_rs.tool("llvm-rc"), "/nologo", "/fo", rc / "app.res", rc / "app.rc"], rc):
                run.check(res is not None and res.read_bytes() == (rc / "app.res").read_bytes(), "the icon's .res is llvm-rc's")
        if run.cmd("hallo-win-release", ["cargo", "build", "--release", "--target", WIN], hallo, env):
            exe = target_dir / WIN / "release" / "hallo.exe"
            run.cmd("hallo-win-release-smoke", [exe, "--script", "tests/smoke.uis"], hallo)
            n = exe.read_bytes().count(str(Path.home()).encode())
            if n:      # docs/CRATES.md, "Risiken": trim-paths is not stable, README.md of jflib names the remedy
                run.notes.append(f"the release program names {Path.home()} {n} times (panic locations)")
        if run.cmd("probe-win-nosdk", ["cargo", "build", "--target", WIN] + novs, probe, env):
            r = run.cmd("probe-win-run", [target_dir / WIN / "debug" / "probe.exe"], probe)
            want = "assets=true big=9900 denied=true version=2.5.0 app=probe"
            run.check(r and r.stdout.strip() == want, f"probe on Windows: {r.stdout.strip() if r else '-'}")
            jfa = out_file("probe", target_dir, WIN, "debug", "app.jfa")
            run.check(jfa is not None and jfa.read_bytes() == assetpack.pack(probe / "assets"), "the probe's bundle is assetpack.py's")
    if "linux" in targets:
        if run.cmd("hallo-linux", ["cargo", "build", "--target", LINUX], hallo, env):
            exe = target_dir / LINUX / "debug" / "hallo"
            r = run_linux([linux_path(exe), "--script", "tests/smoke.uis"], cwd=hallo, timeout=600)
            run.check(r.returncode == 0, f"hallo-linux-smoke (exit {r.returncode})" + ("" if r.returncode == 0 else "\n    " + (r.stdout + r.stderr).strip()[-600:]))
        if run.cmd("probe-linux", ["cargo", "build", "--target", LINUX], probe, env):
            r = run_linux([linux_path(target_dir / LINUX / "debug" / "probe")], cwd=probe, timeout=120)
            want = "assets=true big=9900 denied=true version=2.5.0 app=probe"
            run.check(r.stdout.strip() == want, f"probe on Linux, one static file: {r.stdout.strip() or r.stderr.strip()[-300:]}")
    # cargo jflib, installed from the packages as a user would: a project of its own (new), its tests, the browser
    tool = out / "tool"
    # from the unpacked package, what crates.io would hand out (it has no dependencies): cargo install finds a
    # package in a directory source but then fails to fetch it ("failed to find package with id")
    package = next(vendor.glob("cargo-jflib-*"))
    if run.cmd("cargo-jflib-install", ["cargo", "install", "--path", package, "--offline", "--force", "--root", tool], out,
               {"CARGO_TARGET_DIR": str(out / "target-tool")}):
        jenv = dict(env, PATH=str(tool / "bin") + os.pathsep + os.environ.get("PATH", ""))
        made = out / "made"
        shutil.rmtree(made, ignore_errors=True)
        made.mkdir()
        if run.cmd("cargo-jflib-new", ["cargo", "jflib", "new", "hallo2"], made, jenv):
            app = made / "hallo2"
            cfg = app / ".cargo" / "config.toml"
            cfg.write_text(cfg.read_text(encoding="utf-8") + source_config(vendor), encoding="utf-8")
            if "win" in targets:
                run.cmd("cargo-jflib-test", ["cargo", "jflib", "test"], app, jenv)
            if "web" in targets and run.cmd("cargo-jflib-web", ["cargo", "jflib", "web"], app, jenv):
                site = target_dir / "jflib-web" / "hallo2"
                run.check(all((site / f).is_file() for f in ("hallo2.wasm", "hallo2.simd.wasm", "hallo2.threads.wasm", "index.html",
                                                             "loader.js", "variants.json")) and any((site / "licenses").glob("jflib-rt-*")),
                          "cargo jflib web: three modules, the host, the licences")
                r = subprocess.run(["node", "-e", "import('./tools/tests/playwright.mjs').then(m=>m.loadPlaywright())"], cwd=ROOT,
                                   capture_output=True, text=True)
                if r.returncode:
                    run.notes.append("the browser run needs Playwright (PLAYWRIGHT_MODULE, tools/tests/playwright.mjs): only built")
                else:
                    run.cmd("hallo2-web-click", ["node", ROOT / "tools" / "tests" / "crates_web.mjs", site], ROOT, timeout=300)
    run.cmd("jflib-build-tests", ["cargo", "test", "-p", "jflib-build", "--offline"], ROOT)
    run.cmd("cargo-jflib-tests", ["cargo", "test", "-p", "cargo-jflib", "--offline"], ROOT)
    return run


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--targets", default="win,linux,web")
    ap.add_argument("--out", default=str(ROOT / "build" / "crates_accept"))
    ap.add_argument("--template", action="store_true")
    a = ap.parse_args(argv)
    if a.template:
        write_template()
        return 0
    start = time.monotonic()
    run = accept(set(a.targets.split(",")), Path(a.out).resolve())
    for n in run.notes:
        print(f"note: {n}")
    print(f"crates: {'ok' if not run.fails else 'FAILED'} ({len(run.fails)} failed, {time.monotonic() - start:.0f} s)")
    return 1 if run.fails else 0


if __name__ == "__main__":
    sys.exit(main())
