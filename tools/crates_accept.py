#!/usr/bin/env python3
"""crates_accept.py -- does a project outside this repository build with the crates alone?

    python tools/crates_accept.py [--targets win,linux,web,android,package,watch] [--out DIR]
    python tools/crates_accept.py --targets docs [--docs-toolchain nightly]
                                                 only the docs as docs.rs builds them (below)
    python tools/crates_accept.py --template     write the files of lib/jflib/template that
                                                 come from apps/template and the copies of
                                                 docs/GUIDE.md and docs/API.md in lib/jflib/docs
                                                 (build_rs.py check compares them)

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
          (Playwright, tools/tests/crates_web.mjs; skipped without it), `cargo jflib android --abi
          x86_64`: an APK with the code, the library 16 KiB aligned and the licences (skipped
          without the Android SDK, NDK, JDK or the Rust target x86_64-linux-android), `cargo jflib
          package --styles winui,kde --formats all`: the setup installs the chosen style with a
          script and removes it again (Windows), the .deb passes dpkg-deb and its program runs
          tests/smoke.uis (Linux, WSL on Windows); the AppImage: unsquashfs reads its image, the
          .desktop and metainfo pass their validators, tests/smoke.uis runs through it; the rpm's
          .spec names its tarball's files (the .rpm when rpmbuild is there); the Flatpak manifest
          and, with flatpak-builder, the bundle; the MSIX's files (with MakeAppx).  Signed with
          --sign, --repo and --update by a test certificate (openssl, never installed), a
          throwaway GPG key in Linux and a key from `cargo jflib keygen`: the programs, the setup
          and the MSIX carry Authenticode signatures, gpgv accepts the AppImage's embedded
          signature and the apt repository (apt-get reads it in a folder of its own), the Flatpak
          repository is signed, tools/update_manifest.py verifies every style's channel.
          `cargo jflib watch -- --script tests/watch.uis` (headless, no window): the app runs in
          its session, a saved change builds and starts it again, a broken source keeps the
          old build and fills build.txt, Ctrl+Break ends watch.
  hallo3  made by `cargo jflib new --std` (docs/CRATES.md, CR6) with a std crate from a path:
          `cargo jflib test`; a release program whose std::thread, std::fs and print! work next to
          JFLib, whose std crate's thread_local! counts on a thread of plat, whose panic ends as
          JFLib's crash report (exit 3), and which imports no C runtime DLL (+crt-static); the
          browser (scalar and SIMD), the APK; watch as for hallo2; package refuses the other
          system's formats.  Linux
          only on a Linux machine (a program with std is built there).

Then `cargo test -p jflib-build` and `-p cargo-jflib`.  Output below build/crates_accept; exit 1 when something failed.

`--targets docs` (not part of the default, docs/CRATES.md, CR7) documents every package's library
as docs.rs would after the upload: from the unpacked package, offline, with DOCS_RS=1 and --cfg
docsrs, for the targets and features its [package.metadata.docs.rs] names, with nightly as docs.rs
(--docs-toolchain "" takes the default toolchain).  It never installs a toolchain or a target.
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
FACADE_DOC_NAMES = ("GUIDE.md", "API.md")
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
    for name in FACADE_DOC_NAMES:
        copy, source = FACADE_DOCS / name, ROOT / "docs" / name
        if not copy.is_file() or copy.read_bytes().replace(b"\r\n", b"\n") != source.read_bytes().replace(b"\r\n", b"\n"):
            bad.append(f"lib/jflib/docs/{name}: differs from docs/{name}: python tools/crates_accept.py --template")
    return bad


def write_template():
    for rel, text in derived_template().items():
        p = TEMPLATE / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(text.encode("utf-8"))
        print(f"wrote lib/jflib/template/{rel}")
    facade_docs()
    print(f"wrote lib/jflib/docs/{', '.join(FACADE_DOC_NAMES)}")


# ---------------------------------------------------------------- packing
def facade_docs():
    """docs/GUIDE.md and docs/API.md into the facade's package (lib/jflib/docs, in git, the facade's
    include names it), where an agent finds them below ~/.cargo/registry/src"""
    FACADE_DOCS.mkdir(exist_ok=True)
    for name in FACADE_DOC_NAMES:
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


def forget_changed(out, crates, target_dirs):
    """the packages that differ from the last run -> their names; their Cargo fingerprints are deleted.
    Cargo knows a crate of a directory source by its version alone and a path source (cargo install
    --path) by the times of its files, which `cargo package` sets to 2006 for every file: without this
    a second run would keep what the first one built of a package that has changed since"""
    stamp = out / "packages.json"
    try:
        old = json.loads(stamp.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        old = {}
    new = {c.stem: hashlib.sha256(c.read_bytes()).hexdigest() for c in crates}
    changed = sorted(stem.rsplit("-", 1)[0] for stem, h in new.items() if old.get(stem) != h)
    for d in target_dirs:
        # <profile>/.fingerprint, <triple>/<profile>/.fingerprint and one level more for derived sysroots;
        # a fingerprint is PACKAGE-HASH
        for fp in (f for pat in ("*/.fingerprint", "*/*/.fingerprint", "*/*/*/.fingerprint") for f in d.glob(pat)):
            for f in fp.iterdir():
                if f.name.rsplit("-", 1)[0] in changed:
                    shutil.rmtree(f, ignore_errors=True)
    stamp.write_text(json.dumps(new, indent=1, sort_keys=True), encoding="utf-8")
    return changed


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
    changed = forget_changed(run.out, crates, [run.out / "target", run.out / "target-tool"])
    print(f"ok   {len(changed)} packages changed since the last run" + (" (built again)" if changed else ""), flush=True)
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


def android_missing():
    """what `cargo jflib android --abi x86_64` would miss here, roughly as it looks -> a note or None"""
    home = Path.home()
    sdks = [os.environ.get("ANDROID_SDK_ROOT"), os.environ.get("ANDROID_HOME"),
            os.environ.get("LOCALAPPDATA") and str(Path(os.environ["LOCALAPPDATA"]) / "Android" / "Sdk"),
            str(home / "Android" / "Sdk"), str(home / "Library" / "Android" / "sdk")]
    sdk = next((Path(s) for s in sdks if s and (Path(s) / "build-tools").is_dir()), None)
    if sdk is None:
        return "no Android SDK"
    if not os.environ.get("ANDROID_NDK_HOME") and not any((sdk / "ndk").glob("*/toolchains")):
        return "no Android NDK"
    if not (sdk / "platforms" / "android-36" / "android.jar").is_file():
        return "no Android platform 36"
    if not shutil.which("javac") and not (os.environ.get("JAVA_HOME") and any((Path(os.environ["JAVA_HOME"]) / "bin").glob("javac*"))):
        return "no JDK"
    sysroot = subprocess.run(["rustc", "--print", "sysroot"], capture_output=True, text=True).stdout.strip()
    if not (Path(sysroot) / "lib" / "rustlib" / "x86_64-linux-android").is_dir():
        return "no Rust target x86_64-linux-android (rustup target add x86_64-linux-android)"
    return None


def apk_problems(apk):
    """the APK of cargo jflib android: its code, the library stored and 16 KiB aligned, the licences -> problems"""
    import struct
    import zipfile
    raw = apk.read_bytes()
    z = zipfile.ZipFile(apk)
    names = set(z.namelist())
    bad = [f"{n} missing" for n in ("AndroidManifest.xml", "classes.dex", "lib/x86_64/libjfgui_android.so") if n not in names]
    if not any(n.startswith("assets/licenses/jflib-rt-") for n in names):
        bad.append("assets/licenses/jflib-rt-* missing")
    if "lib/x86_64/libjfgui_android.so" in names:
        i = z.getinfo("lib/x86_64/libjfgui_android.so")
        n, e = struct.unpack("<HH", raw[i.header_offset + 26:i.header_offset + 30])
        if i.compress_type != zipfile.ZIP_STORED or (i.header_offset + 30 + n + e) % 16384:
            bad.append("the library is not stored at a 16 KiB boundary")
        elf = z.read(i)
        if elf[:4] != b"\x7fELF" or struct.unpack("<H", elf[18:20])[0] != 62:
            bad.append("the library is no x86-64 ELF")
    return bad


def setup_entries(setup):
    """the payload of a setup: [(kind, name)] (docs/PACKAGING.md, "Payload layout")"""
    import struct
    from tools import codesign
    sig = codesign.pe_signature_bytes(setup)            # a signed setup: the trailer ends before the signature
    if sig:                                             # and up to 7 bytes of padding (lib/installer/payload.rs)
        setup = setup[:sig[0]]
        for _ in range(7):
            if setup[-24:-16] == b"ASMINST1":
                break
            setup = setup[:-1]
    stub, pay = struct.unpack("<QQ", setup[-16:])
    assert setup[-24:-16] == b"ASMINST1"
    p = setup[stub:stub + pay]
    out = []
    for k in range(struct.unpack("<I", p[4:8])[0]):
        rec = p[8 + 128 * k:8 + 128 * (k + 1)]
        out.append((struct.unpack("<I", rec[:4])[0], rec[32:80].rstrip(b"\0").decode("utf-8")))
    return out


def check_setup(run, setup, want):
    """install the kde variant with a script (ASMINST_SHELL_DIR: no shortcuts, no registry), compare
    the program, remove it again"""
    entries = setup_entries(setup.read_bytes())
    run.check(sorted(n for k, n in entries if k == 3) == ["kde", "winui"], f"the setup has previews of both styles: {entries[:6]}")
    run.check(any(k == 5 and "jflib-style-kde-" in n and "DejaVu" in n for k, n in entries)
              or any(k == 5 and n.startswith("licenses/jflib-style-kde-") for k, n in entries),
              "the setup carries the style's font licence")
    d = run.out / "setup-test"
    shutil.rmtree(d, ignore_errors=True)
    (d / "shell").mkdir(parents=True)
    (d / "inst.uis").write_text('click "Weiter"\nclick "Installieren"\nwait 500\nexpect "Fertigstellen" exists\n', encoding="utf-8")
    (d / "un.uis").write_text('click "Deinstallieren"\nwait 500\nexpect "Schließen" exists\n', encoding="utf-8")
    env = dict(os.environ, ASMINST_SHELL_DIR=str(d / "shell"))
    r = run.cmd("setup-install", [setup, "--script", d / "inst.uis", "--dir", d / "inst", "--style", "kde"], d, env, timeout=300)
    if r:
        run.check((d / "inst" / "hallo2.exe").read_bytes() == want.read_bytes(), "the setup installs the kde program")
        assoc = (d / "shell" / "assoc.txt").read_text(encoding="utf-8", errors="replace") if (d / "shell" / "assoc.txt").is_file() else ""
        run.check(".hal2" in assoc and "hallo2" in assoc, "the setup registers the file type and the URL scheme of PACKAGE (test mode: assoc.txt)")
        run.cmd("setup-uninstall", [d / "inst" / "uninstall.exe", "--uninstall", "--script", d / "un.uis"], d, env, timeout=300)
        run.check(not (d / "inst" / "hallo2.exe").exists(), "uninstall.exe removes it again")


def check_deb(run, deb, app):
    """dpkg-deb reads the .deb (Linux, WSL on Windows); its kde program runs tests/smoke.uis"""
    from tools.host_exec import linux_path, run_linux
    have = run_linux(["sh", "-c", "command -v dpkg-deb"], cwd=app, timeout=60)
    if have.returncode:
        run.notes.append("the .deb is not checked: no dpkg-deb here")
        return
    info = run_linux(["dpkg-deb", "--info", linux_path(deb)], cwd=app, timeout=120)
    run.check(info.returncode == 0 and "Package: hallo2" in info.stdout and "Depends: debconf" in info.stdout
              and all(f"*  {s}" in info.stdout for s in ("config", "postinst", "prerm", "postrm")),
              "dpkg-deb --info: the control file, the maintainer scripts executable" + ("" if info.returncode == 0 else "\n    " + info.stderr.strip()[-400:]))
    files = run_linux(["dpkg-deb", "--contents", linux_path(deb)], cwd=app, timeout=120)
    lines = files.stdout.splitlines()
    want = ["rwxr-xr-x root/root .* ./usr/lib/hallo2/hallo2-kde$", "rw-r--r-- root/root .* ./usr/share/applications/hallo2.desktop$",
            "./usr/share/icons/hicolor/256x256/apps/hallo2.png$", "./usr/share/doc/hallo2/licenses/jflib-style-kde-[0-9.]+/DejaVu-LICENSE.txt$"]
    import re
    missing = [w for w in want if not any(re.search(w, ln) for ln in lines)]
    run.check(files.returncode == 0 and not missing, f"dpkg-deb --contents: {len(lines)} entries" + "".join("\n    missing " + m for m in missing))
    x = "/tmp/jflib-crates-deb"
    r = run_linux(["sh", "-c", f"rm -rf {x} && mkdir -p {x} && dpkg-deb -x {linux_path(deb)} {x} && mkdir -p target && {x}/usr/lib/hallo2/hallo2-kde --script tests/smoke.uis"],
                  cwd=app, timeout=600)
    run.check(r.returncode == 0, "the .deb's kde program runs tests/smoke.uis" + ("" if r.returncode == 0 else "\n    " + (r.stdout + r.stderr).strip()[-600:]))


HALLO2_PACKAGE = """# PACKAGE of hallo2 (crates_accept.py): the keys cargo jflib package reads
display=Hallo Zwei
summary=A window made by cargo jflib new
description=The template of cargo jflib new, packaged in every format by the acceptance test of the JFLib crates.
homepage=https://example.org/hallo2
publisher=JFLib Test
filetypes=hal2|application/x-hallo2|Hallo-Datei|own
schemes=hallo2
"""


class Keys:
    """what signs in the acceptance test: env (the variables for cargo jflib package), the GPG
    home in Linux, the update key's public half"""

    def __init__(self, env, home, public):
        self.env, self.home, self.public = env, home, public


def signing_keys(run, out, app, jenv):
    """a test certificate (openssl, never installed; its subject is the MSIX publisher of
    HALLO2_PACKAGE), a throwaway GPG key in Linux (WSL), an update key from cargo jflib keygen
    -> Keys, or None when one cannot be made here (said in a note)"""
    from tools import pkg_check
    from tools.host_exec import run_linux
    d = out / "keys"
    shutil.rmtree(d, ignore_errors=True)
    d.mkdir(parents=True)
    pfx = pkg_check.test_cert(d, "codesign", "/CN=JFLib Test", "crates") if os.name == "nt" else None
    if os.name == "nt" and pfx is None:
        run.notes.append("not signed: no openssl for the test certificate")
        return None
    home = f"/tmp/jflib-crates-gpg-{os.getpid()}"
    r = run_linux(["python3", pkg_check.lp(ROOT / "tools" / "pkg_check.py"), "--inside-linux", "gpgkey", home], cwd=app, timeout=300)
    fpr = next((ln[9:] for ln in r.stdout.splitlines() if ln.startswith("INFO fpr=")), "")
    if not fpr:
        run.notes.append("not signed: no throwaway GPG key in Linux")
        return None
    r = run.cmd("cargo-jflib-keygen", ["cargo", "jflib", "keygen", d / "update.updkey"], app, jenv)
    public = r.stdout.strip().splitlines()[-1] if r else ""
    run.check(len(public) == 64, f"cargo jflib keygen: the public key {public[:16]}...")
    env = {"jfgui_GPG_KEY": fpr, "jfgui_GPG_HOME": home, "jfgui_UPDATE_KEY_FILE": str(d / "update.updkey")}
    if pfx:
        env.update({"jfgui_SIGN_PFX": str(pfx), "jfgui_SIGN_PFX_PASS": "crates"})
    return Keys(env, home, public)


def check_signed(run, dist, out, app, keys):
    """what --sign, --repo and --update made"""
    import zipfile
    from tools import codesign, pkg_check, update_manifest
    from tools.host_exec import linux_path, run_linux
    if os.name == "nt":
        unsigned = [f.name for f in [dist / "hallo2_Setup.exe", *dist.glob("work/hallo2/*/win/hallo2.exe")]
                    if codesign.pe_signature(f) is None]
        run.check(not unsigned, "Authenticode: the setup and every style's program" + (f": unsigned {unsigned}" if unsigned else ""))
        msix = dist / "hallo2-0.1.0-x64.msix"
        if msix.is_file():
            run.check("AppxSignature.p7x" in zipfile.ZipFile(msix).namelist(), "the MSIX is signed (AppxSignature.p7x)")
    for style in ("winui", "kde"):
        img = dist / f"hallo2-0.1.0-{style}-x86_64.AppImage"
        r = run_linux(["python3", pkg_check.lp(ROOT / "tools" / "pkg_check.py"), "--inside-linux", "appimagesig", linux_path(img), keys.home],
                      cwd=app, timeout=300)
        bad = [ln for ln in r.stdout.splitlines() if ln.startswith("FAIL")]
        run.check(r.returncode == 0 and not bad and "PASS" in r.stdout, f"gpgv accepts the {style} AppImage's embedded signature"
                  + "".join("\n    " + b for b in bad))
    apt = out / "apt"
    script = out / "keys" / "apt.sh"
    script.write_text(f"""set -e
R={linux_path(apt)}
gpgv --keyring "$R/repo.gpg" "$R/InRelease" 2>&1
gpgv --keyring "$R/repo.gpg" "$R/Release.gpg" "$R/Release" 2>&1
T=/tmp/jflib-crates-apt
rm -rf "$T"; mkdir -p "$T/lists/partial" "$T/cache/archives/partial"
echo "deb [signed-by=$R/repo.gpg] file:$R ./" > "$T/sources.list"
O="-o Dir::Etc::SourceList=$T/sources.list -o Dir::Etc::SourceParts=/nonexistent -o Dir::State::Lists=$T/lists -o Dir::Cache=$T/cache -o Debug::NoLocking=1"
apt-get $O update 2>&1
apt-cache $O policy hallo2
rm -rf "$T"
""", encoding="utf-8", newline="\n")
    r = run_linux(["sh", linux_path(script)], cwd=app, timeout=300)
    run.check(r.returncode == 0 and "Good signature" in r.stdout and "Candidate: 0.1.0" in r.stdout,
              "the apt repository: gpgv accepts InRelease and Release.gpg, apt-get reads it (candidate 0.1.0)"
              + ("" if r.returncode == 0 else "\n    " + (r.stdout + r.stderr).strip()[-800:]))
    repo = dist / "flatpak" / "repo"
    run.check((repo / "summary.sig").is_file() and (dist / "flatpak" / "org.example.hallo2.gpg").is_file(),
              "the Flatpak repository is signed (summary.sig) and its key lies next to it")
    for style in ("winui", "kde"):
        problems = update_manifest.check_channel(out / "channel" / style, bytes.fromhex(keys.public))
        m = json.loads((out / "channel" / style / "update.json").read_text(encoding="utf-8"))
        kinds = sorted(f["kind"] for f in m["files"])
        want = ["appimage", "exe", "package"] if os.name == "nt" else ["appimage", "package"]
        run.check(not problems and kinds == want, f"the {style} update channel verifies under the keygen key: {', '.join(kinds)}"
                  + "".join("\n    " + x for x in problems))


def check_watch(run, app, jenv, target_dir):
    """cargo jflib watch with the app headless under a script (no window): built and started in
    its session, a saved change built and started again, a broken source keeps the old build and
    fills build.txt, the fixed one builds again, Ctrl+Break (Ctrl+C) ends watch"""
    import signal
    name = app.name
    tag = "" if name == "hallo2" else f"{name}: "
    (app / "tests").mkdir(exist_ok=True)
    (app / "tests" / "watch.uis").write_text('click "Klick mich"\nexpect text:"Klicks: 1" exists\nshot target/watch.png\n', encoding="utf-8")
    (app / "target").mkdir(exist_ok=True)
    shot, session = app / "target" / "watch.png", target_dir / "jflib-watch" / name
    shot.unlink(missing_ok=True)
    src = app / "src" / "lib.rs"
    original = src.read_bytes()
    log = run.log / f"cargo-jflib-watch-{name}.log"
    text = lambda: log.read_text(encoding="utf-8", errors="replace")  # noqa: E731
    out = open(log, "w", encoding="utf-8")
    proc = subprocess.Popen(["cargo", "jflib", "watch", "--", "--script", "tests/watch.uis"], cwd=app, env=dict(os.environ, **jenv),
                            stdout=out, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
                            creationflags=subprocess.CREATE_NEW_PROCESS_GROUP if os.name == "nt" else 0, start_new_session=os.name != "nt")

    def until(what, cond, secs=600):
        end = time.monotonic() + secs
        while time.monotonic() < end and proc.poll() is None:
            try:
                if cond():
                    run.check(True, tag + what)
                    return True
            except OSError:
                pass
            time.sleep(0.5)
        run.fail(f"{tag}{what} (log/{log.name})\n    " + text().strip()[-600:].replace("\n", "\n    "))
        return False

    code = None
    try:
        if until("cargo jflib watch: built, the app ran in its session (replay.txt, its script's shot)",
                 lambda: shot.is_file() and (session / "replay.txt").is_file()):
            shot.unlink()
            src.write_bytes(original + b"\n// changed by crates_accept.py\n")
            if until("watch: a saved change is built and started again", lambda: shot.is_file() and text().count("watch: build done") >= 2):
                src.write_bytes(original + b"\nfn broken(\n")
                if until("watch: a broken source fails, build.txt holds the error",
                         lambda: "watch: build FAILED" in text() and "error" in (session / "build.txt").read_text(encoding="utf-8")):
                    shot.unlink(missing_ok=True)
                    src.write_bytes(original)
                    until("watch: the fixed source is built and started again, build.txt empty",
                          lambda: shot.is_file() and text().count("watch: build done") >= 3 and not (session / "build.txt").read_text(encoding="utf-8").strip())
    finally:
        src.write_bytes(original)
        if proc.poll() is None:
            proc.send_signal(signal.CTRL_BREAK_EVENT if os.name == "nt" else signal.SIGINT)
            try:
                code = proc.wait(timeout=60)
            except subprocess.TimeoutExpired:
                proc.kill()
        out.close()
    run.check(code == 0 and "watch: ending" in text(), f"{tag}Ctrl+Break ends watch (exit {code})")


def check_appimage(run, image, program, app):
    """the AppImage of the kde style: unsquashfs reads its image (the program is the one built),
    the .desktop and metainfo pass desktop-file-validate and appstreamcli, tests/smoke.uis runs
    through it (Linux, WSL on Windows)"""
    from tools.host_exec import linux_path, run_linux
    if not image.is_file():
        return run.fail(f"{image.name} missing")
    have = run_linux(["sh", "-c", "command -v unsquashfs"], cwd=app, timeout=60)
    if have.returncode:
        run.notes.append("the AppImage is not checked: no unsquashfs here")
        return
    x = "/tmp/jflib-crates-appimage"
    img = linux_path(image)
    r = run_linux(["sh", "-c", f"set -e; chmod +x {img}; off=$({img} --appimage-offset); rm -rf {x}; "
                   f"unsquashfs -q -n -o $off -d {x} {img} >/dev/null; ls -la {x} {x}/usr/lib/hallo2; "
                   f"cmp {x}/usr/lib/hallo2/hallo2 {linux_path(program)} && echo SAME"], cwd=app, timeout=300)
    run.check(r.returncode == 0 and "AppRun -> usr/lib/hallo2/hallo2" in r.stdout and "SAME" in r.stdout,
              "unsquashfs reads the AppImage: AppRun, the kde program" + ("" if r.returncode == 0 else "\n    " + (r.stdout + r.stderr).strip()[-600:]))
    v = run_linux(["sh", "-c", f"desktop-file-validate {x}/org.example.hallo2.desktop && appstreamcli validate --no-net "
                   f"{x}/usr/share/metainfo/org.example.hallo2.appdata.xml"], cwd=app, timeout=300)
    run.check(v.returncode == 0, "the AppImage's .desktop and metainfo pass desktop-file-validate and appstreamcli"
              + ("" if v.returncode == 0 else "\n    " + (v.stdout + v.stderr).strip()[-600:]))
    r = run_linux(["sh", "-c", f"mkdir -p target && {img} --script tests/smoke.uis"], cwd=app, timeout=600)
    run.check(r.returncode == 0, "tests/smoke.uis runs through the kde AppImage" + ("" if r.returncode == 0 else "\n    " + (r.stdout + r.stderr).strip()[-600:]))


def check_rpm(run, dist):
    """the .spec names exactly the files of its tarball"""
    import re
    specs = list((dist / "rpm" / "SPECS").glob("*.spec"))
    tars = list((dist / "rpm" / "SOURCES").glob("*-root.tar.gz"))
    if len(specs) != 1 or len(tars) != 1:
        return run.fail(f"rpm: one .spec and one tarball expected: {specs} {tars}")
    with tarfile.open(tars[0]) as t:
        files = sorted("/" + m.name[2:] for m in t.getmembers() if m.isfile())
    spec = specs[0].read_text(encoding="utf-8")
    listed = sorted(m.replace("%%", "%") for m in re.findall(r'^"(/[^"]+)"$', spec.split("%files", 1)[1], re.M))
    run.check(files == listed and f"Source0:        {tars[0].name}" in spec and "/usr/lib/hallo2/hallo2-kde" in files,
              f"the .spec names the {len(files)} files of its tarball")
    if any(dist.glob("*.rpm")):
        run.check(True, "rpmbuild made the .rpm")
    else:
        run.notes.append("no .rpm: rpmbuild is not there (only the .spec and its tarball)")


def check_flatpak(run, dist):
    man = dist / "flatpak" / "org.example.hallo2.json"
    try:
        m = json.loads(man.read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        return run.fail(f"the Flatpak manifest: {e}")
    run.check(m["id"] == "org.example.hallo2" and m["command"] == "hallo2" and "--device=dri" in m["finish-args"]
              and (dist / "flatpak" / "files" / "lib" / "hallo2" / "hallo2").is_file(), "the Flatpak manifest and its files")
    bundle = dist / "org.example.hallo2.flatpak"
    if bundle.is_file():
        run.check(bundle.read_bytes()[:8] == b"OSTREE\0\0" or bundle.stat().st_size > 100_000, f"flatpak-builder made the bundle ({bundle.stat().st_size} bytes)")
    else:
        run.notes.append("no .flatpak: flatpak-builder or its runtime is not there (only the manifest)")


def check_msix(run, dist):
    import zipfile
    msix = dist / "hallo2-0.1.0-x64.msix"
    if not msix.is_file():
        run.notes.append("no .msix: MakeAppx is not there (only the folder)")
        return
    names = set(zipfile.ZipFile(msix).namelist())
    want = {"AppxManifest.xml", "AppxBlockMap.xml", "hallo2.exe", "Assets/StoreLogo.png", "Assets/Wide310x150Logo.png"}
    run.check(want <= names and any(n.startswith("licenses/jflib-style-") for n in names), f"the MSIX holds the program, the logos, the licences ({len(names)} files)")


# ---------------------------------------------------------------- an app with std (CR6)
STD_CRATE = """//! stdcrate -- an ordinary std crate: a HashMap and thread_local!
use std::cell::Cell;
use std::collections::HashMap;

thread_local! {
    static CALLS: Cell<u32> = const { Cell::new(0) };
}

/// the number of different words, and how often this thread called it so far
pub fn count_words(text: &str) -> (usize, u32) {
    let mut m: HashMap<&str, usize> = HashMap::new();
    for w in text.split_whitespace() {
        *m.entry(w).or_default() += 1;
    }
    let calls = CALLS.with(|c| {
        c.set(c.get() + 1);
        c.get()
    });
    (m.len(), calls)
}
"""

# at the start of ui::main!'s body in hallo3's src/lib.rs: std next to JFLib, without a window
STD_CHECK = """    if jflib::plat::args().iter().any(|a| a == "--std-panic") {
        panic!("std panic check");
    }
    if jflib::plat::args().iter().any(|a| a == "--std-check") {
        let (words, calls) = std::thread::spawn(|| stdcrate::count_words("a b a")).join().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let th = jflib::plat::thread_spawn(0, Box::new(move || {
            let _ = tx.send((stdcrate::count_words("x").1, stdcrate::count_words("y").1));
        }));
        let tls = match th {
            Some(t) => {
                t.join();
                rx.recv().unwrap_or((0, 0))
            }
            None => (0, 0),
        };
        let file = std::env::temp_dir().join("hallo3-std-check.txt");
        let fs = std::fs::write(&file, "std").is_ok() && std::fs::read_to_string(&file).is_ok_and(|s| s == "std");
        print!("std words={words} calls={calls} tls={},{} fs={fs}", tls.0, tls.1);
        return 0;
    }
"""
STD_WANT = "std words=2 calls=1 tls=1,2 fs=true"


def pe_imports(exe):
    """the DLLs a PE program imports"""
    import struct
    d = Path(exe).read_bytes()
    pe = struct.unpack_from("<I", d, 0x3C)[0]
    n, opt = struct.unpack_from("<H", d, pe + 6)[0], struct.unpack_from("<H", d, pe + 20)[0]
    secs = [struct.unpack_from("<8sIIII", d, pe + 24 + opt + 40 * i) for i in range(n)]

    def off(rva):
        return next(rva - va + ro for _, vs, va, rs, ro in secs if va <= rva < va + max(vs, rs))
    o, names = off(struct.unpack_from("<I", d, pe + 24 + 120)[0]), []
    while True:
        entry = struct.unpack_from("<IIIII", d, o)
        if entry[3] == 0:
            return names
        s = off(entry[3])
        names.append(d[s:d.index(b"\0", s)].decode())
        o += 20


def check_std(run, made, vendor, targets, jenv, target_dir):
    """hallo3: `cargo jflib new --std` and a std crate from a path; std's threads, files and output,
    thread_local! on a thread of plat, a panic as JFLib's crash report; the browser and Android"""
    if not run.cmd("cargo-jflib-new-std", ["cargo", "jflib", "new", "--std", "hallo3"], made, jenv):
        return
    app = made / "hallo3"
    run.check(not (app / "PERMISSIONS").exists() and "+crt-static" in (app / ".cargo" / "config.toml").read_text(encoding="utf-8"),
              "cargo jflib new --std: no PERMISSIONS, the C runtime linked in on Windows")
    cfg = app / ".cargo" / "config.toml"
    cfg.write_text(cfg.read_text(encoding="utf-8") + source_config(vendor), encoding="utf-8")
    (app / "stdcrate" / "src").mkdir(parents=True)
    (app / "stdcrate" / "Cargo.toml").write_text('[package]\nname = "stdcrate"\nversion = "0.1.0"\nedition = "2024"\n', encoding="utf-8")
    (app / "stdcrate" / "src" / "lib.rs").write_text(STD_CRATE, encoding="utf-8")
    toml = app / "Cargo.toml"
    toml.write_text(toml.read_text(encoding="utf-8").replace("[dependencies]\n", '[dependencies]\nstdcrate = { path = "stdcrate" }\n', 1),
                    encoding="utf-8")
    lib = app / "src" / "lib.rs"
    text = lib.read_text(encoding="utf-8")
    run.check("ui::main!({\n" in text, "hallo3's src/lib.rs has ui::main!")
    lib.write_text(text.replace("ui::main!({\n", "ui::main!({\n" + STD_CHECK, 1), encoding="utf-8")
    (app / "target").mkdir(exist_ok=True)
    if "win" in targets:
        run.cmd("hallo3-test", ["cargo", "jflib", "test"], app, jenv)
        if run.cmd("hallo3-win-release", ["cargo", "build", "--release", "--target", WIN], app, jenv):
            exe = target_dir / WIN / "release" / "hallo3.exe"
            r = subprocess.run([exe, "--std-check"], cwd=app, capture_output=True, text=True, timeout=120, stdin=subprocess.DEVNULL)
            run.check(r.returncode == 0 and r.stdout == STD_WANT, f"hallo3 --std-check: {r.stdout!r} (exit {r.returncode})")
            r = subprocess.run([exe, "--std-panic"], cwd=app, capture_output=True, text=True, timeout=120, stdin=subprocess.DEVNULL)
            run.check(r.returncode == 3 and "panic: panicked at" in r.stderr and "std panic check" in r.stderr,
                      f"hallo3 --std-panic: JFLib's crash report, exit 3 (exit {r.returncode}: {r.stderr.strip()[-200:]})")
            crt = [n for n in pe_imports(exe) if n.lower().startswith(("vcruntime", "api-ms-win-crt", "ucrtbase", "msvcp"))]
            run.check(not crt, f"hallo3.exe imports no C runtime DLL (+crt-static): {crt}")
    if "linux" in targets:
        if sys.platform.startswith("linux"):
            if run.cmd("hallo3-linux", ["cargo", "build", "--target", LINUX], app, jenv):
                r = subprocess.run([target_dir / LINUX / "debug" / "hallo3", "--std-check"], cwd=app, capture_output=True, text=True, timeout=120)
                run.check(r.returncode == 0 and r.stdout == STD_WANT, f"hallo3 --std-check on Linux: {r.stdout!r}")
        else:
            run.notes.append("hallo3 for Linux: a program with std is built on Linux itself (run this there; Fedora by hand on 2026-11-07)")
    if "web" in targets and run.cmd("hallo3-web", ["cargo", "jflib", "web"], app, jenv):
        site = target_dir / "jflib-web" / "hallo3"
        run.check((site / "hallo3.wasm").is_file() and (site / "hallo3.simd.wasm").is_file() and not (site / "hallo3.threads.wasm").exists(),
                  "cargo jflib web with std: scalar and SIMD, no threads mode")
    if "android" in targets and not android_missing():
        if run.cmd("hallo3-android", ["cargo", "jflib", "android", "--abi", "x86_64"], app, jenv):
            apk = target_dir / "jflib-android" / "hallo3-x86_64-debug.apk"
            bad = apk_problems(apk) if apk.is_file() else [f"{apk} missing"]
            run.check(not bad, "cargo jflib android with std" + "".join("\n    " + b for b in bad))
    if "watch" in targets:
        check_watch(run, app, jenv, target_dir)
    if "package" in targets:
        r = subprocess.run(["cargo", "jflib", "package", "--formats", "deb" if os.name == "nt" else "setup"], cwd=app,
                           env=dict(os.environ, **jenv), capture_output=True, text=True, timeout=600)
        run.check(r.returncode != 0 and "made on" in r.stderr, "cargo jflib package refuses the other system's formats for an app with std")

# ---------------------------------------------------------------- the docs as docs.rs builds them (CR7)
# what docs.rs builds without [package.metadata.docs.rs] targets (docs.rs/about/metadata)
DOCS_RS_DEFAULT = ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin", "x86_64-pc-windows-msvc", "aarch64-unknown-linux-gnu",
                   "i686-pc-windows-msvc"]


def docs_toolchain_missing(toolchain, targets):
    """what the toolchain lacks for these targets -> a message, or None.  Never through `cargo +X` or
    `rustup +X ...` before the toolchain is known to be there: rustup would download it unasked."""
    if not toolchain:
        return None
    r = subprocess.run(["rustup", "toolchain", "list"], capture_output=True, text=True)
    if not any(line.split()[0].startswith(toolchain + "-") or line.split()[0] == toolchain for line in r.stdout.splitlines() if line.strip()):
        return f"rustup has no toolchain {toolchain} (rustup toolchain install {toolchain} --profile minimal)"
    r = subprocess.run(["rustup", "+" + toolchain, "target", "list", "--installed"], capture_output=True, text=True)
    missing = sorted(set(targets) - set(r.stdout.split()))
    return f"the toolchain {toolchain} lacks {', '.join(missing)} (rustup +{toolchain} target add {' '.join(missing)})" if missing else None


def check_docs(run, vendor, toolchain):
    """every package's library documented as docs.rs does it: rustdoc from the unpacked package (its
    own workspace), offline (docs.rs has no network), DOCS_RS=1, --cfg docsrs, the targets and
    features of its [package.metadata.docs.rs]; docs.rs uses nightly, stops rustdoc after 15 minutes
    and keeps 102.4 kB of log.  The facade first into an empty target folder: its time is the one of
    a build from nothing; the others then find their dependencies compiled."""
    import tomllib
    work, target_dir = run.out / "docsrs", run.out / "target-docsrs"
    shutil.rmtree(work, ignore_errors=True)
    shutil.rmtree(target_dir, ignore_errors=True)
    packages = sorted((d for d in vendor.iterdir() if d.name.startswith(("jflib-", "cargo-jflib-"))), key=lambda d: (not d.name.startswith("jflib-0"), d.name))
    plans = []
    for pkg in packages:
        m = tomllib.loads((pkg / "Cargo.toml").read_text(encoding="utf-8"))
        meta = m["package"].get("metadata", {}).get("docs", {}).get("rs", {})
        plans.append((pkg, m, meta, meta.get("targets") or DOCS_RS_DEFAULT))
    missing = docs_toolchain_missing(toolchain, {t for _, m, _, ts in plans if "lib" in m for t in ts})
    if missing:
        run.fail(f"docs: {missing}")
        return
    tc = [f"+{toolchain}"] if toolchain else []
    env = {"CARGO_TARGET_DIR": str(target_dir), "DOCS_RS": "1", "RUSTDOCFLAGS": "--cfg docsrs"}
    slow, chatty, built = [], [], 0
    for pkg, m, meta, targets in plans:
        name = m["package"]["name"]
        if "lib" not in m:
            run.check(True, f"docs: {name} is a program, docs.rs documents no library")
            continue
        run.check(not ({"aarch64-apple-darwin", "i686-pc-windows-msvc"} & set(targets)), f"docs: {name} names its targets ({len(targets)})")
        d = work / name
        shutil.copytree(pkg, d)
        toml = d / "Cargo.toml"
        toml.write_text(toml.read_text(encoding="utf-8") + "\n[workspace]\n", encoding="utf-8")
        (d / ".cargo").mkdir(exist_ok=True)
        (d / ".cargo" / "config.toml").write_text(source_config(vendor), encoding="utf-8")
        args = ["cargo", *tc, "rustdoc", "--lib", "--offline"]
        if meta.get("features"):
            args += ["--features", ",".join(meta["features"])]
        if meta.get("all-features"):
            args.append("--all-features")
        if meta.get("no-default-features"):
            args.append("--no-default-features")
        for t in targets:
            start = time.monotonic()
            r = run.cmd(f"docs-{name}-{t}", args + ["--target", t], d, env)
            took = time.monotonic() - start
            if not r:
                continue
            built += 1
            if not (target_dir / t / "doc" / m["lib"]["name"] / "index.html").is_file():
                run.fail(f"docs: {name} for {t}: no {m['lib']['name']}/index.html")
            log = r.stdout + r.stderr
            if took > 300:
                slow.append(f"{name} {t} {took:.0f} s")
            warnings = sum(1 for line in log.splitlines() if line.startswith("warning:"))
            if len(log.encode()) > 100_000 or warnings > 50:
                chatty.append(f"{name} {t}: {warnings} warnings, {len(log.encode()) // 1000} kB of log")
    run.check(built > 0, f"docs: {built} builds as docs.rs makes them ({toolchain or 'default toolchain'})")
    if slow:
        run.notes.append("docs: slow (docs.rs stops rustdoc after 15 minutes, on slower machines): " + "; ".join(slow))
    if chatty:
        run.notes.append("docs: much output (docs.rs keeps 102.4 kB of log): " + "; ".join(chatty))

def out_file(project, target_dir, triple, profile, name):
    """a file the build script of the project wrote to its OUT_DIR"""
    found = sorted((target_dir / triple / profile / "build").glob(f"{project}-*/out/{name}"), key=lambda p: p.stat().st_mtime)
    return found[-1] if found else None


def accept(targets, out, docs_toolchain="nightly"):
    sys.path.insert(0, str(ROOT))
    sys.dont_write_bytecode = True
    import build_rs
    from tools import assetpack
    from tools.host_exec import linux_path, run_linux
    run = Run(out)
    vendor = pack(run)
    if vendor is None:
        return run
    if "docs" in targets:
        check_docs(run, vendor, docs_toolchain)
        if targets <= {"docs"}:
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
            if "android" in targets:
                missing = android_missing()
                if missing:
                    run.notes.append(f"cargo jflib android not run: {missing}")
                elif run.cmd("cargo-jflib-android", ["cargo", "jflib", "android", "--abi", "x86_64"], app, jenv):
                    apk = target_dir / "jflib-android" / "hallo2-x86_64-debug.apk"
                    bad = apk_problems(apk) if apk.is_file() else [f"{apk} missing"]
                    run.check(not bad, "cargo jflib android: code, library (16 KiB), licences" + "".join("\n    " + b for b in bad))
            if "watch" in targets:
                check_watch(run, app, jenv, target_dir)
            dist = target_dir / "jflib-package"
            if "package" in targets and dist.is_dir():
                # what this run makes, nothing older (the stubs and their builds stay)
                for f in dist.iterdir():
                    if f.is_file():
                        f.unlink()
                    elif f.name in ("rpm", "flatpak"):
                        shutil.rmtree(f, ignore_errors=True)
            keys = None
            if "package" in targets:
                # what a published app says of itself: AppStream wants a homepage and a real description
                (app / "PACKAGE").write_text(HALLO2_PACKAGE, encoding="utf-8")
                keys = signing_keys(run, out, app, jenv)
                for d in ("apt", "channel"):
                    shutil.rmtree(out / d, ignore_errors=True)
            signed = ["--sign", "--repo", out / "apt", "--update", out / "channel"] if keys else []
            if "package" in targets and run.cmd("cargo-jflib-package", ["cargo", "jflib", "package", "--styles", "winui,kde", "--formats", "all", *signed],
                                                app, dict(jenv, **(keys.env if keys else {}))):
                work = dist / "work" / "hallo2" / "kde"
                if os.name == "nt":
                    check_setup(run, dist / "hallo2_Setup.exe", work / "win" / "hallo2.exe")
                    check_msix(run, dist)
                check_deb(run, dist / "hallo2_0.1.0_amd64.deb", app)
                check_appimage(run, dist / "hallo2-0.1.0-kde-x86_64.AppImage", work / "linux" / "hallo2", app)
                check_rpm(run, dist)
                check_flatpak(run, dist)
                if keys:
                    check_signed(run, dist, out, app, keys)
            if keys:
                from tools import pkg_check
                from tools.host_exec import run_linux
                run_linux(["python3", pkg_check.lp(ROOT / "tools" / "pkg_check.py"), "--inside-linux", "gpgdrop", keys.home], cwd=app, timeout=120)
        check_std(run, made, vendor, targets, jenv, target_dir)
    run.cmd("jflib-build-tests", ["cargo", "test", "-p", "jflib-build", "--offline"], ROOT)
    run.cmd("cargo-jflib-tests", ["cargo", "test", "-p", "cargo-jflib", "--offline"], ROOT)
    return run


def main(argv=None):
    sys.stdout.reconfigure(errors="replace")             # the validators' output may hold characters cp1252 lacks
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--targets", default="win,linux,web,android,package,watch")
    ap.add_argument("--out", default=str(ROOT / "build" / "crates_accept"))
    ap.add_argument("--template", action="store_true")
    ap.add_argument("--docs-toolchain", default="nightly", help='--targets docs: the toolchain ("" the default one)')
    a = ap.parse_args(argv)
    if a.template:
        write_template()
        return 0
    start = time.monotonic()
    run = accept(set(a.targets.split(",")), Path(a.out).resolve(), a.docs_toolchain)
    for n in run.notes:
        print(f"note: {n}")
    print(f"crates: {'ok' if not run.fails else 'FAILED'} ({len(run.fails)} failed, {time.monotonic() - start:.0f} s)")
    return 1 if run.fails else 0


if __name__ == "__main__":
    sys.exit(main())
