"""The developer tools without a build: `build_rs.py new` (tools/scaffold.py), the API snapshot and
diff (tools/apidiff.py), the changelog helper (tools/changelog.py) and the catalogue check
(`build_rs.py i18n`, tools/i18n_check.py).

    python tools/tests/test_devkit.py        (build_rs.py test runs it; about a second)

That the generated apps build and pass their smoke tests is checked by building them:
apps/scaffoldtest is the generator's output (test_fixture_is_generator_output keeps it so), and
the suite builds and runs it like every app.
"""
import contextlib
import io
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(ROOT))
import apidiff  # noqa: E402
import changelog  # noqa: E402
import gendoc  # noqa: E402
import i18n_check  # noqa: E402
import manifests  # noqa: E402
import pkgmeta  # noqa: E402
import scaffold  # noqa: E402
import build_rs  # noqa: E402


class Temp(unittest.TestCase):
    def setUp(self):
        (ROOT / "build").mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix="devkit-", dir=ROOT / "build")
        self.dir = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def write(self, rel, text):
        p = self.dir / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_bytes(text.encode("utf-8"))
        return p


# ---------------------------------------------------------------- build_rs.py new
class ScaffoldTest(Temp):
    INFO = {"homepage": "https://github.com/Some-One/Repo", "owner": ("github.com", "Some-One"), "publisher": "Some One"}

    def test_every_kind(self):
        need = {"window": {"src/main.rs", "STYLE"}, "console": {"src/main.rs"},
                "tool": {"src/main.rs", "cli/main.rs", "crates/my_app_core/lib.rs", "BINS", "STYLE"},
                "service": {"src/main.rs"}}
        for kind in scaffold.KINDS:
            files = scaffold.app_files("my-App", kind, "kde", "Mein Programm", self.INFO, "2026-10-20")
            self.assertLessEqual(need[kind] | {"PACKAGE", "PERMISSIONS", "TARGETS", "README.md", "tests/smoke.uis"},
                                 set(files), kind)
            for rel, text in files.items():
                self.assertNotRegex(text, r"@[A-Z_]+@", f"{kind} {rel}: a template key is left")
            for rel in [r for r in files if r.endswith(".rs") and not r.endswith("build.rs")]:     # build.rs: a Cargo build script
                head = files[rel]
                self.assertIn("#![no_std]", head, rel)
                self.assertTrue("#![deny(unsafe_code)]" in head or "#![forbid(unsafe_code)]" in head, rel)
                self.assertNotIn("std::", head.replace("core::", ""), rel)
            build_rs.perm_check(files["PERMISSIONS"], f"{kind}/PERMISSIONS")       # the build's syntax
            meta = pkgmeta.kv_file(self.write(f"{kind}/PACKAGE", files["PACKAGE"]))
            self.assertEqual(meta["version"], "0.1.0")
            self.assertNotEqual(meta["summary"], meta["display"])     # desktop-file-validate warns
            self.assertEqual(pkgmeta.app_id(meta, "my-App"), "io.github.Some_One.my_App")
            self.assertEqual(meta["homepage"], "https://github.com/Some-One/Repo")
            self.assertTrue(build_rs.SEMVER.fullmatch(meta["version"]))
            self.assertIn("Mein Programm", files["src/main.rs"])
            self.assertIn("rt::app_version!()", files["src/main.rs"])
            self.assertEqual(files["TARGETS"].split(), ["win", "linux", "web"] if kind == "window" else ["win", "linux"])
            if "STYLE" in files:
                self.assertEqual(files["STYLE"].strip(), "kde")
            first = files["tests/smoke.uis"].splitlines()[0]
            if kind in ("console", "service"):
                self.assertTrue(first.startswith("#!args"), kind)
        tool = scaffold.app_files("my-App", "tool", "winui", None, self.INFO)
        self.assertRegex(tool["BINS"], r"(?m)^my-App-cli\s+cli/main\.rs\s+cli\b")
        self.assertIn("my_app_core::count", tool["src/main.rs"])
        self.assertNotIn("homepage", scaffold.app_files("x", "window", info={"homepage": "", "owner": ""})["PACKAGE"])
        self.assertIn("app_id=org.jflib.x", scaffold.app_files("x", "window")["PACKAGE"])

    def test_names(self):
        (self.dir / "apps" / "Paint").mkdir(parents=True)
        crates = ["ui", "plat", "rt", "bc_proto", "demo_core"]
        for bad, why in (("paint", "exists already"), ("PAINT", "differ only in case"), ("9lives", "no app name"),
                         ("a b", "no app name"), ("x" * 41, "no app name"), ("ui", "taken by a library crate"),
                         ("Plat", "taken"), ("style_x", "taken"), ("demo", "taken"), ("fn", "keyword"),
                         ("core", "reserved"), ("", "no app name")):
            with self.assertRaisesRegex(scaffold.ScaffoldError, why, msg=bad):
                scaffold.check_app_name(self.dir, bad, crates)
        scaffold.check_app_name(self.dir, "Paint2-go_x", crates)
        for bad in ("Caps", "1x", "with-dash", "ui", "style_kde", "self", "std"):
            with self.assertRaises(scaffold.ScaffoldError, msg=bad):
                scaffold.check_lib_name(self.dir, bad, crates)
        for bad in ('Say "hi"', "back\\slash", "a\nb", "", "x" * 81):
            with self.assertRaises(scaffold.ScaffoldError, msg=bad):
                scaffold.check_title(bad)

    def test_create_app(self):
        self.write("Cargo.toml", (ROOT / "Cargo.toml").read_text(encoding="utf-8"))
        paths = scaffold.create_app(self.dir, "hello", "tool", "winui", "Hallo", ["ui"], ["winui"], "2026-10-20")
        self.assertIn(self.dir / "apps" / "hello" / "crates" / "hello_core" / "lib.rs", paths)
        self.assertTrue(all(p.exists() and b"\r\n" not in p.read_bytes() for p in paths if p.name != "Cargo.toml" or p.parent != self.dir))
        members = manifests.read(self.dir / "Cargo.toml")["workspace"]["members"]
        self.assertEqual(members[-3:], ["apps/hello", "apps/hello/cli", "apps/hello/crates/hello_core"])
        cli = manifests.read(self.dir / "apps" / "hello" / "cli" / "Cargo.toml")
        self.assertEqual(cli["bin"][0]["name"], "hello-cli")
        self.assertIn("hello_core", cli["dependencies"])
        self.assertIn('part_of("hello", "..")', (self.dir / "apps" / "hello" / "cli" / "build.rs").read_text(encoding="utf-8"))
        with self.assertRaisesRegex(scaffold.ScaffoldError, "exists already"):
            scaffold.create_app(self.dir, "Hello", "window", "winui", None, ["ui"], ["winui"])
        with self.assertRaisesRegex(scaffold.ScaffoldError, "unknown style"):
            scaffold.create_app(self.dir, "other", "window", "nope", None, ["ui"], ["winui"])
        self.assertFalse((self.dir / "apps" / "other").exists())
        with self.assertRaisesRegex(scaffold.ScaffoldError, "unknown kind"):
            scaffold.app_files("x", "daemon")

    def test_remote(self):
        cases = {
            "https://github.com/Graphsto/JFLib.git": ("https://github.com/Graphsto/JFLib", ("github.com", "Graphsto")),
            "https://user:ghp_secret@github.com/a-b/c/": ("https://github.com/a-b/c", ("github.com", "a-b")),
            "git@github.com:Graphsto/JFLib.git": ("https://github.com/Graphsto/JFLib", ("github.com", "Graphsto")),
            "ssh://git@gitlab.com:2222/grp/sub/proj.git": ("https://gitlab.com/grp/sub/proj", ("gitlab.com", "grp")),
            "https://git.example.org/team/app": ("https://git.example.org/team/app", ""),
            "": ("", ""), "C:/repos/local": ("", ""), "/srv/git/app.git": ("", ""),
        }
        for url, (home, owner) in cases.items():
            r = scaffold.parse_remote(url)
            self.assertEqual((r["homepage"], r["owner"]), (home, owner), url)
            self.assertNotIn("secret", r["homepage"])
        self.assertEqual(scaffold.app_id_for("My-App", {"owner": ("gitlab.com", "9grp")}), "io.gitlab._9grp.My_App")

    def test_lib(self):
        for rel in ("Cargo.toml", "lib/jflib/Cargo.toml", "lib/jflib/src/lib.rs", "LICENSE-MIT", "LICENSE-APACHE"):
            self.write(rel, (ROOT / rel).read_text(encoding="utf-8"))
        self.write("Cargo.toml", (ROOT / "Cargo.toml").read_text(encoding="utf-8").replace("\n", "\r\n"))
        self.write("tools/gendoc.py", (ROOT / "tools" / "gendoc.py").read_text(encoding="utf-8"))
        libs = list(build_rs.LIBS)
        with self.assertRaisesRegex(scaffold.ScaffoldError, "unknown dependencies"):
            scaffold.create_lib(self.dir, "mylib", ["nope"], None, libs, libs)
        paths = scaffold.create_lib(self.dir, "mylib", ["plat", "rt"], "my things", libs, libs)
        nb = (self.dir / "Cargo.toml").read_bytes()
        self.assertEqual(nb.count(b"\n"), nb.count(b"\r\n"), "the file keeps its CRLF line ends")
        root = manifests.read(self.dir / "Cargo.toml")["workspace"]
        self.assertEqual(root["members"][root["members"].index("lib/jflib") - 1], "lib/mylib", "the last crate of lib/")
        self.assertEqual(root["dependencies"]["mylib"], {"package": "jflib-mylib", "path": "lib/mylib", "version": "=" + root["package"]["version"]})
        made = manifests.read(self.dir / "lib" / "mylib" / "Cargo.toml")
        self.assertEqual(made["package"]["description"], "my things")
        self.assertEqual(list(made["dependencies"]), ["rt", "plat"], "in the order of the root manifest")
        self.assertIn("mylib", manifests.read(self.dir / "lib" / "jflib" / "Cargo.toml")["dependencies"])
        self.assertIn("pub use mylib;\n", (self.dir / "lib" / "jflib" / "src" / "lib.rs").read_text(encoding="utf-8"))
        self.assertIn('    ("mylib", "my things", "full"),\n]', (self.dir / "tools" / "gendoc.py").read_text(encoding="utf-8"))
        lib = (self.dir / "lib" / "mylib" / "src" / "lib.rs").read_text(encoding="utf-8")
        self.assertIn("#![forbid(unsafe_code)]", lib)
        self.assertIn(self.dir / "lib" / "mylib" / "src" / "lib.rs", paths)
        self.assertTrue((self.dir / "lib" / "mylib" / "LICENSE-MIT").is_file())
        readme = (self.dir / "lib" / "mylib" / "README.md").read_text(encoding="utf-8")
        self.assertTrue(readme.startswith("# jflib-mylib\n") and changelog.PLACEHOLDER in readme, "a frame check refuses")
        log = (self.dir / "lib" / "mylib" / "CHANGELOG.md").read_text(encoding="utf-8")
        version = root["package"]["version"]
        self.assertIn(f"## [Unreleased]\n\nFirst release on crates.io.\n\n[Unreleased]: {changelog.REPO}/compare/v{version}...HEAD\n", log)
        with self.assertRaisesRegex(scaffold.ScaffoldError, "exists already"):
            scaffold.create_lib(self.dir, "mylib", [], None, libs, libs)

    def test_manifests_set_version(self):
        """a release changes the version everywhere it stands, and nothing of the crates from outside"""
        for rel in ("Cargo.toml", "Cargo.lock", "modules/modplat/Cargo.toml", *manifests.DEP_VERSION_FILES):
            self.write(rel, (ROOT / rel).read_text(encoding="utf-8"))
        self.assertEqual(manifests.dep_version_problems(self.dir), [], "the repository's files ask for its version")
        guide = self.write("docs/GUIDE.md", 'jflib = { features = ["std"], version = "0.1" }\njflib-build = "0.1"\n'
                                            'jflib-netlab = "0.1"\nother = "0.1"\n')
        self.assertEqual(manifests.dep_version_problems(self.dir), [] if manifests.version().startswith("0.1.") else
                         [f"docs/GUIDE.md: asks for jflib 0.1, the library is {manifests.version()} (build_rs.py release writes it)"])
        before = manifests.external_lock(self.dir)
        manifests.set_version("7.8.9", self.dir)
        self.assertEqual(guide.read_text(encoding="utf-8"), 'jflib = { features = ["std"], version = "7.8" }\njflib-build = "7.8"\n'
                                                            'jflib-netlab = "0.1"\nother = "0.1"\n')
        template = (self.dir / "lib/jflib/template/Cargo.toml.in").read_text(encoding="utf-8")
        self.assertIn('jflib = "7.8"', template)
        self.assertIn('jflib-build = "7.8"', template)
        self.assertEqual(manifests.dep_version_problems(self.dir), [])
        root = manifests.read(self.dir / "Cargo.toml")["workspace"]
        self.assertEqual(root["package"]["version"], "7.8.9")
        self.assertEqual({d["version"] for d in root["dependencies"].values() if "version" in d}, {"=7.8.9"})  # the unpublished: none
        self.assertEqual(manifests.read(self.dir / "modules/modplat/Cargo.toml")["dependencies"]["rt"]["version"], "=7.8.9")
        lock = manifests.read(self.dir / "Cargo.lock")["package"]
        self.assertEqual({p["version"] for p in lock if p["name"].startswith("jflib")}, {"7.8.9"})
        self.assertEqual([p["version"] for p in lock if p["name"] == "cargo-jflib"], ["7.8.9"], "cargo-jflib is published too")
        self.assertEqual(manifests.external_lock(self.dir), before)

    def test_publish_plan(self):
        """tools/publish.py: the sparse index's paths, an order where each crate follows what it needs,
        crates.io's retry time out of its 429 text, which crates are there, new or a new version"""
        from tools import publish
        self.assertEqual([publish.index_path(n) for n in ("a", "ab", "abc", "jflib", "Cargo-JFLib")],
                         ["1/a", "2/ab", "3/a/abc", "jf/li/jflib", "ca/rg/cargo-jflib"])
        pkgs = {"jflib": {"version": "0.1.0", "deps": ["ui", "rt"]}, "ui": {"version": "0.1.0", "deps": ["rt"]},
                "rt": {"version": "0.1.0", "deps": []}, "cargo-jflib": {"version": "0.1.0", "deps": []}}
        self.assertEqual(publish.order(pkgs), ["cargo-jflib", "rt", "ui", "jflib"])
        with self.assertRaisesRegex(publish.PublishError, "cycle"):
            publish.order({"a": {"version": "1", "deps": ["b"]}, "b": {"version": "1", "deps": ["a"]}})
        when = publish.retry_after("error: ... (status 429 Too Many Requests): You have published too many new crates in a "
                                   "short period of time. Please try again after Mon, 09 Nov 2026 12:34:56 GMT and see "
                                   "https://crates.io/docs/rate-limits for more details.")
        self.assertEqual((when.year, when.month, when.day, when.hour, when.minute, when.utcoffset().total_seconds()), (2026, 11, 9, 12, 34, 0))
        self.assertIsNone(publish.retry_after("error: something else"))
        have = {"rt": {"0.1.0"}, "ui": {"0.0.9"}}
        self.assertEqual(publish.plan(pkgs, ["rt", "ui", "jflib"], have.get),
                         [("rt", "0.1.0", "there"), ("ui", "0.1.0", "update"), ("jflib", "0.1.0", "new")])

    def test_publish_waits_out_the_limit(self):
        """tools/publish.py --publish: a 429 waits until crates.io's time and tries the same crate
        again, a crate that is there is skipped, another failure stops (start again goes on)"""
        from unittest import mock
        from tools import publish
        answers = iter([(0, "Uploaded"), (101, "error: (status 429 Too Many Requests): ... Please try again after "
                                               "Mon, 09 Nov 2026 12:34:56 GMT and see ..."), (0, "Uploaded"),
                        (101, "error: crate b@0.1.0 already exists on crates.io index"), (101, "error: network down")])
        calls, waits = [], []
        with mock.patch.object(publish, "cargo_publish", lambda n, nv: calls.append(n) or next(answers)), \
             mock.patch.object(publish, "wait_in_index", lambda n, v: None), \
             mock.patch.object(publish, "wait_until", lambda when, why: waits.append(when)), \
             mock.patch.object(publish, "say", lambda text: None):
            rows = [("a", "0.1.0", "new"), ("z", "0.1.0", "there"), ("c", "0.1.0", "new"), ("b", "0.1.0", "new"), ("d", "0.1.0", "new")]
            with self.assertRaisesRegex(publish.PublishError, "cargo publish -p d failed"):
                publish.publish({}, rows, False)
        self.assertEqual(calls, ["a", "c", "c", "b", "d"])
        self.assertEqual((waits[0].hour, waits[0].minute, waits[0].second), (12, 35, 11), "the retry time plus 15 s")

    def test_publish_logs_cargo(self):
        """tools/publish.py: Cargo's output goes to build/publish.log too (a failed upload of the
        first run of 2026-11-08 left only "exit 101" there)"""
        import io
        import tempfile
        from unittest import mock
        from tools import publish

        class Proc:
            stdout = io.StringIO("    Updating crates.io index\nerror: no token found\n")

            def wait(self):
                return 101
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "publish.log"
            with mock.patch.object(publish, "LOG", log), mock.patch.object(publish.subprocess, "Popen", lambda *a, **k: Proc()), \
                 mock.patch("builtins.print"):
                code, out = publish.cargo_publish("cargo-jflib", False)
            self.assertEqual(code, 101)
            self.assertIn("error: no token found", out)
            text = log.read_text(encoding="utf-8")
            self.assertIn("$ cargo publish -p cargo-jflib --locked", text)
            self.assertIn("    error: no token found\n", text)

    def test_manifests_docs_rs(self):
        """every crate that goes to crates.io names its docs.rs targets (docs.rs's five defaults
        include macOS and 32-bit Windows, none of JFLib's targets) and what crates.io shows of it:
        description, license, repository, homepage (docs/CRATES.md, CR7)"""
        import subprocess
        import tomllib
        files = subprocess.run(["git", "ls-files", "*Cargo.toml"], cwd=manifests.ROOT, capture_output=True, text=True, check=True).stdout.split()
        published = 0
        for rel in files:
            m = tomllib.loads((manifests.ROOT / rel).read_text(encoding="utf-8"))
            if "package" not in m or m["package"].get("publish", True) is False:
                continue
            published += 1
            targets = m["package"].get("metadata", {}).get("docs", {}).get("rs", {}).get("targets")
            self.assertTrue(targets, f"{rel}: no [package.metadata.docs.rs] targets")
            self.assertFalse({"aarch64-apple-darwin", "i686-pc-windows-msvc"} & set(targets), rel)
            for field in ("description", "license", "repository", "homepage"):
                self.assertIn(field, m["package"], f"{rel}: no {field} (crates.io shows it; the root's through {field}.workspace)")
        self.assertGreater(published, 50)

    def test_manifests_library_crates(self):
        """the facade is no library crate, a module's names and one-target crates come out right, an
        unknown [target] table is an error, not a dependency that build_rs.py silently misses"""
        libs, target = manifests.library_crates()
        self.assertNotIn("jflib", libs)
        self.assertEqual(libs["rt"], ("lib/rt/lib.rs", []))
        self.assertEqual(libs["text_shape"], ("lib/text_shape/src/lib.rs", []), "its crates from outside are Cargo's")
        self.assertIn("rt", libs["img_jpeg"][1])
        self.assertNotIn("modrt", libs["img_jpeg"][1])
        self.assertEqual(target[("ui", "win")], ["shader", "gpu"])
        self.assertNotIn("gpu", libs["ui"][1])
        self.write("Cargo.toml", '[workspace]\nmembers = ["lib/a"]\n')
        self.write("lib/a/Cargo.toml", '[lib]\nname = "a"\npath = "lib.rs"\n\n[target.\'cfg(unix)\'.dependencies]\nrt = "1"\n')
        with self.assertRaisesRegex(manifests.ManifestError, "cfg.unix"):
            manifests.library_crates(self.dir)

    def test_fixture_is_generator_output(self):
        """apps/scaffoldtest is `new scaffoldtest --kind tool --title Scaffold-Test`: a change of the
        templates must be made there too (regenerate it), and a library change that breaks the
        fixture's build must be fixed in the templates as well"""
        app = ROOT / "apps" / "scaffoldtest"
        meta = pkgmeta.kv_file(app / "PACKAGE")
        files = scaffold.app_files("scaffoldtest", "tool", meta["default_win"], meta["display"])
        for rel, text in files.items():
            if rel in ("README.md", "PACKAGE"):
                continue
            have = (app / rel).read_bytes().decode("utf-8").replace("\r\n", "\n")
            self.assertEqual(have, text, f"apps/scaffoldtest/{rel} differs from the generator's output "
                             "(python build_rs.py new scaffoldtest --kind tool --title Scaffold-Test, "
                             "into a scratch copy)")
        gen = pkgmeta.kv_file(self.write("PACKAGE", files["PACKAGE"]))
        for k in set(gen) - {"publisher", "homepage", "app_id"}:
            self.assertEqual(meta.get(k), gen[k], f"PACKAGE {k}=")


# ---------------------------------------------------------------- api-diff
V1 = {
    "lib.rs": """
//! demo
pub mod shapes;
mod inner;
pub use inner::*;
#[doc(hidden)]
pub fn hidden_one() {}
pub fn keep(a: u32, b: &str) -> u32 { 0 }
pub fn gone() {}
pub fn retype(a: u32) {}
pub const LIMIT: u32 = 10;
#[derive(Clone, Copy, Debug)]
pub struct Desc { pub w: u32, pub h: u32 }
pub struct Opaque { inner: u32 }
impl Opaque { pub fn new() -> Opaque { Opaque { inner: 0 } } fn private(&self) {} }
pub enum Mode { A, B(u32) }
#[non_exhaustive]
pub enum Open { X }
pub trait Hook { fn need(&self); fn have(&self) {} fn soon(&self) {} }
impl Default for Desc { fn default() -> Self { Desc { w: 0, h: 0 } } }
#[macro_export]
macro_rules! shout { () => {} }
""",
    "inner.rs": "pub fn moved_around(x: i32) -> i32 { x }\n",
    "shapes.rs": "pub fn area(w: u32, h: u32) -> u32 { w * h }\npub(crate) fn internal() {}\n",
}
V2 = dict(V1)
V2["lib.rs"] = (V1["lib.rs"]
                .replace("pub fn keep(a: u32, b: &str) -> u32 { 0 }", "pub fn keep(\n    first: u32,\n    second: &str,\n) -> u32 { 0 }")
                .replace("pub fn gone() {}\n", "")
                .replace("pub fn retype(a: u32) {}", "pub fn retype(a: u64) {}")
                .replace("pub struct Desc { pub w: u32, pub h: u32 }", "pub struct Desc { pub w: u32, pub h: u32, pub d: u32 }")
                .replace("pub enum Mode { A, B(u32) }", "pub enum Mode { A, B(u32), C }")
                .replace("pub enum Open { X }", "pub enum Open { X, Y }")
                .replace("fn soon(&self) {} }", "fn soon(&self); fn later(&self) {} fn must(&self); }")
                .replace("#[derive(Clone, Copy, Debug)]", "#[derive(Clone, Debug)]")
                .replace("pub const LIMIT: u32 = 10;", "pub const LIMIT: u32 = 20;\n#[deprecated(since = \"0.2.0\", note = \"x\")]\npub fn added() {}")
                .replace("mod inner;\npub use inner::*;", "mod inner2;\npub use inner2::*;"))
V2["inner2.rs"] = V2.pop("inner.rs")


class ApiDiffTest(Temp):
    def crate(self, files, name="demo"):
        d = self.dir / "lib" / name
        if d.exists():
            shutil.rmtree(d)
        for rel, text in files.items():
            self.write(f"lib/{name}/{rel}", text)
        old = gendoc.LIB
        gendoc.LIB = self.dir / "lib"
        try:
            lines = apidiff.crate_lines(gendoc.Crate(name))
        finally:
            gendoc.LIB = old
        return sorted({(p, k, s, ",".join(sorted(set(f)))) for p, k, s, f in lines})

    def test_snapshot_and_diff(self):
        old = self.crate(V1)
        paths = {(p, k) for p, k, _, _ in old}
        for want in (("demo::keep", "fn"), ("demo::moved_around", "fn"), ("demo::shapes::area", "fn"),
                     ("demo::Desc", "struct"), ("demo::Desc.w", "field"), ("demo::Desc", "impl"),
                     ("demo::Opaque::new", "method"), ("demo::Mode::B", "variant"), ("demo::Hook::have", "trait-fn"),
                     ("demo::shout", "macro"), ("demo::LIMIT", "const")):
            self.assertIn(want, paths)
        names = {p for p, _ in paths}
        for hidden in ("demo::hidden_one", "demo::Opaque.inner", "demo::Opaque::private", "demo::shapes::internal",
                       "demo::inner::moved_around"):
            self.assertNotIn(hidden, names)
        flags = {(p, k): f for p, k, _, f in old}
        self.assertEqual(flags[("demo::Desc", "struct")], "all-pub")
        self.assertEqual(flags[("demo::Hook::have", "trait-fn")], "provided")
        self.assertEqual(flags[("demo::Open", "enum")], "non-exhaustive")
        snap = apidiff.write_snapshot("0.1.0", old, self.dir / "api" / "0.1.0.txt")
        self.assertEqual(apidiff.read_snapshot(snap), old)

        d = apidiff.diff(old, self.crate(V2))
        got = {(what, ln[0]) for what, ln, _ in d["breaking"]}
        self.assertEqual(got, {("removed", "demo::gone"), ("changed", "demo::retype"), ("variant", "demo::Mode::C"),
                               ("trait", "demo::Hook::must"), ("required", "demo::Hook::soon"),
                               ("removed", "demo::Desc")})                    # the Copy derive
        self.assertEqual({ln[0] for _, ln, _ in d["added"]},
                         {"demo::Desc.d", "demo::Open::Y", "demo::Hook::later", "demo::added"})
        self.assertEqual([ln[0] for _, ln, _ in d["notes"]], ["demo::Desc.d"])
        self.assertEqual([ln[0] for _, ln, _ in d["deprecated"]], [])     # deprecated when added: just new
        self.assertEqual(apidiff.level_of(d), "breaking")
        lines, ok = apidiff.report(d, "0.1.0", "0.1.0")
        self.assertFalse(ok)
        self.assertIn("0.2.0", lines[-1])
        self.assertTrue(apidiff.report(d, "0.1.0", "0.2.0")[1])
        same = apidiff.diff(old, old)
        self.assertEqual(apidiff.level_of(same), "none")

    def test_deprecation(self):
        old = self.crate(V1)
        new = self.crate({**V1, "lib.rs": V1["lib.rs"].replace("pub fn gone()", "#[deprecated]\npub fn gone()")})
        d = apidiff.diff(old, new)
        self.assertEqual([ln[0] for _, ln, _ in d["deprecated"]], ["demo::gone"])
        self.assertEqual((d["breaking"], apidiff.level_of(d)), ([], "added"))
        d = apidiff.diff(new, old)                   # the deprecation taken back: no break
        self.assertEqual(d["breaking"], [])

    def test_module_per_platform(self):
        """a module declared once per platform under one name (plat/process.rs: three `mod native`)
        keeps every variant, each with its cfg; variants under a cfg without a flag (target_arch)
        give one identity several signatures, compared as a set"""
        native = "pub struct Child { h: u32 }\nimpl Child {\n    /// wait\n    pub fn wait(&mut self) -> {} { 0 }\n}\n"
        files = {
            "lib.rs": "//! demo\npub mod process;\npub mod kernel;\n",
            "process.rs": "\n".join(f'#[cfg({c})]\n#[path = "process_{n}.rs"]\nmod native;' for c, n in (
                ("windows", "win"), ('target_os = "linux"', "linux"), ('target_vendor = "apple"', "apple")))
            + "\npub use native::Child;\n",
            "process_win.rs": native.replace("{}", "u32"),
            "process_linux.rs": native.replace("{}", "i32"),
            "process_apple.rs": native.replace("{}", "i32"),
            "kernel.rs": "pub use arch::*;\n#[cfg(target_arch = \"x86_64\")]\nmod arch { pub struct Stat { pub n: u64 } }\n"
                         "#[cfg(target_arch = \"aarch64\")]\nmod arch { pub struct Stat { pub n: u32 } }\n",
        }
        lines = self.crate(files)
        got = {(p, k, f) for p, k, _, f in lines if p.startswith("demo::process")}
        self.assertEqual(got, {(p, k, f) for f in ("windows", "linux", "apple")
                               for p, k in (("demo::process::Child", "struct"), ("demo::process::Child::wait", "method"))})
        self.assertEqual({s for p, _, s, _ in lines if p == "demo::kernel::Stat.n"}, {"n: u64", "n: u32"})
        old = gendoc.LIB
        gendoc.LIB = self.dir / "lib"
        try:
            crate = gendoc.Crate("demo")                    # API.md: a section per variant, each marked
            w = gendoc.Writer()
            for f in crate.file_list():
                if gendoc.has_public(crate, f):
                    w.file_section(crate, f, 3, lambda it: id(it) in crate.reach)
        finally:
            gendoc.LIB = old
        text = "\n".join(w.md)
        for mark in ("process_win.rs` (Windows)", "process_linux.rs` (Linux)", "process_apple.rs` (Apple)"):
            self.assertIn(mark, text)
        self.assertEqual(text.count("**`struct Child`**"), 3)

        no_apple = self.crate({**files, "process.rs": files["process.rs"].replace(
            '#[cfg(target_vendor = "apple")]\n#[path = "process_apple.rs"]\nmod native;\n', "")})
        d = apidiff.diff(lines, no_apple)
        self.assertEqual({(what, ln[0], ln[3]) for what, ln, _ in d["breaking"]},
                         {("removed", "demo::process::Child", "apple"), ("removed", "demo::process::Child::wait", "apple")})
        self.assertEqual(d["added"], [])
        one_arch = [ln for ln in lines if ln[2] != "n: u64"]          # a snapshot that saw one target only
        d = apidiff.diff(one_arch, lines)
        self.assertEqual((d["breaking"], [ln[2] for _, ln, _ in d["added"]]), ([], ["n: u64"]))
        d = apidiff.diff(lines, one_arch)                               # a target's signature gone
        self.assertEqual([(what, ln[0], detail) for what, ln, detail in d["breaking"]],
                         [("changed", "demo::kernel::Stat.n", "n: u64")])

    def test_versions(self):
        nv = apidiff.next_version
        self.assertEqual((nv("0.1.0", "breaking"), nv("0.1.0", "added"), nv("0.1.0", "none")), ("0.2.0", "0.1.1", "0.1.1"))
        self.assertEqual((nv("1.4.2", "breaking"), nv("1.4.2", "added"), nv("1.4.2", "none")), ("2.0.0", "1.5.0", "1.4.3"))
        ok = apidiff.version_ok
        self.assertTrue(ok("0.2.0", "0.1.0", "breaking") and ok("1.0.0", "0.1.0", "breaking"))
        self.assertFalse(ok("0.1.9", "0.1.0", "breaking") or ok("0.2.0-rc.1", "0.1.0", "breaking"))
        self.assertTrue(ok("0.1.0", "0.1.0", "none"))
        self.assertLess(apidiff.parse_version("1.0.0-alpha"), apidiff.parse_version("1.0.0"))
        for bad in ("1.0", "01.0.0", "v1.0.0", "1.0.0.0"):
            with self.assertRaises(ValueError, msg=bad):
                apidiff.parse_version(bad)

    def test_canon(self):
        c = apidiff.canon
        self.assertEqual(c("fn f(a: u32, b: &str,) -> u32", "fn"), c("fn f( x : u32,\n  y: &str ) -> u32", "fn"))
        self.assertNotEqual(c("fn f(a: u32)", "fn"), c("fn f(a: u64)", "fn"))
        self.assertEqual(c("fn g<T: Fn(u32) -> bool>(f: T)", "fn"), "fn g<T:Fn(u32)->bool>(T)")
        self.assertEqual(apidiff.param_types("fn h(&mut self, (x, y): (i32, i32))"), "fn h(&mut self, (i32, i32))")

    def test_library_snapshot(self):
        """the release snapshots in docs/api read back, and the library's version is a version"""
        apidiff.parse_version(apidiff.lib_version())
        for v, p in apidiff.snapshots():
            lines = apidiff.read_snapshot(p)
            self.assertGreater(len(lines), 1000, p)
            self.assertTrue(all(ln[0] and ln[1] for ln in lines), p)


# ---------------------------------------------------------------- changelog
LOG = """# Changelog

Intro.

## [Unreleased]

## [0.1.0] - 2026-10-20

### Added

- first
"""


class ChangelogTest(unittest.TestCase):
    def test_add_and_cut(self):
        t = changelog.add_entry(LOG, "Fixed the thing", "fixed", "ui")
        t = changelog.add_entry(t, "A new widget", "added")
        t = changelog.add_entry(t, "Another  widget\n", "Added")
        self.assertIn("## [Unreleased]\n\n### Added\n\n- A new widget\n- Another widget\n\n### Fixed\n\n"
                      "- **ui:** Fixed the thing\n\n## [0.1.0] - 2026-10-20", t)
        self.assertEqual(changelog.unreleased(t).splitlines()[0], "### Added")
        c = changelog.cut(t, "0.2.0", "2026-11-01")
        self.assertIn("## [Unreleased]\n\n## [0.2.0] - 2026-11-01\n\n### Added", c)
        self.assertEqual(changelog.releases(c), ["0.2.0", "0.1.0"])
        self.assertEqual(changelog.unreleased(c), "")
        for bad in (lambda: changelog.add_entry(LOG, "x", "improved"), lambda: changelog.add_entry(LOG, "  ", "added"),
                    lambda: changelog.cut(LOG, "0.1.0", "2026-10-21"), lambda: changelog.unreleased("# none\n")):
            with self.assertRaises(changelog.ChangelogError):
                bad()

    def test_crate_cut(self):
        """a crate's changelog: its first entry replaces "no changes", a cut moves the link references
        on, an [Unreleased] without entries is cut as "no changes\""""
        head = "# Changelog of jflib-x\n\nIntro.\n\n## [Unreleased]\n\n"
        refs = (f"## [0.1.0] - 2026-11-08\n\nFirst release on crates.io.\n\n[Unreleased]: {changelog.REPO}/compare/v0.1.0...HEAD\n"
                f"[0.1.0]: {changelog.REPO}/releases/tag/v0.1.0\n")
        quiet = head + "No changes in this crate. It is released with the next version of JFLib.\n\n" + refs
        self.assertTrue(changelog.without_entries(quiet))
        t = changelog.add_entry(quiet, "A thing", "fixed")
        self.assertIn("## [Unreleased]\n\n### Fixed\n\n- A thing\n\n## [0.1.0]", t)
        self.assertFalse(changelog.without_entries(t))
        c = changelog.cut_crate(t, "0.2.0", "2026-12-01")
        self.assertIn("## [Unreleased]\n\n## [0.2.0] - 2026-12-01\n\n### Fixed\n\n- A thing\n", c)
        self.assertIn(f"[Unreleased]: {changelog.REPO}/compare/v0.2.0...HEAD\n[0.2.0]: {changelog.REPO}/compare/v0.1.0...v0.2.0\n"
                      f"[0.1.0]: {changelog.REPO}/releases/tag/v0.1.0\n", c)
        c = changelog.cut_crate(quiet, "0.2.0", "2026-12-01")
        self.assertIn("## [0.2.0] - 2026-12-01\n\nNo changes in this crate. It was released with the others", c)
        self.assertEqual(changelog.unreleased(c), "")
        new = changelog.cut_crate(changelog.crate_log("jflib-y", "0.2.0"), "0.3.0", "2027-01-01")
        self.assertIn("## [0.3.0] - 2027-01-01\n\nFirst release on crates.io.", new)
        self.assertIn(f"[0.3.0]: {changelog.REPO}/compare/v0.2.0...v0.3.0", new)

    def test_repository_changelog(self):
        """CHANGELOG.md has [Unreleased] and a section of the library's version"""
        text, _ = changelog.read()
        self.assertIn(apidiff.lib_version(), changelog.releases(text))
        self.assertTrue(changelog.UNRELEASED.search(text))


class CrateDocsTest(Temp):
    """every crate's README.md and CHANGELOG.md (tools/changelog.py, crate_doc_problems)"""

    def tree(self, readme_b, include=None):
        """a workspace with the crates a (all as it should be) and b (its README readme_b, an include list)"""
        self.write("Cargo.toml", '[workspace]\nmembers = ["lib/a", "lib/b", "apps/x"]\n[workspace.package]\nversion = "0.1.0"\n')
        self.write("CHANGELOG.md", "# Changelog\n\n## [Unreleased]\n\n## [0.1.0] - 2026-11-08\n")
        self.write("apps/x/Cargo.toml", '[package]\nname = "x"\npublish = false\n')
        self.write("lib/a/Cargo.toml", '[package]\nname = "jflib-a"\n[lib]\nname = "a"\n')
        self.write("lib/a/README.md", "# jflib-a\n\nText with [a link](https://jflib.org).\n\n## What's new in 0.2.0\n\nNew.\n")
        self.write("lib/a/CHANGELOG.md", changelog.crate_log("jflib-a", "0.1.0"))
        self.write("lib/b/Cargo.toml", '[package]\nname = "jflib-b"\n' + (f"include = {include}\n" if include else ""))
        self.write("lib/b/README.md", readme_b)

    def test_problems(self):
        self.tree("# jflib-b\n\nSee [the guide](docs/GUIDE.md) and [here](#below).\n\n```\n[x](not/a/link)\n```\n",
                  '["src/**", "README.md"]')
        self.assertEqual(sorted(changelog.crate_doc_problems(self.dir)), sorted([
            'lib/b/CHANGELOG.md is missing (docs/CRATES.md, "README und CHANGELOG je Crate")',
            "lib/b/README.md: the link docs/GUIDE.md is relative (crates.io and docs.rs show the text elsewhere)",
            "lib/b/README.md: needs one section \"## What's new in X.Y.Z\" (has 0)"]))
        self.write("lib/b/CHANGELOG.md", "# Changelog\n\n## [0.0.9] - 2026-01-01\n")
        self.write("lib/b/README.md", "# b\n\n" + changelog.PLACEHOLDER + " -->\n\n## What's new in 0.0.9\n")
        self.assertEqual(sorted(changelog.crate_doc_problems(self.dir)), sorted([
            "lib/b/Cargo.toml: include lacks CHANGELOG.md, the package would not carry it",
            "lib/b/README.md: the first line is not '# jflib-b'",
            "lib/b/README.md: still the placeholder of build_rs.py new --lib: write the README",
            "lib/b/README.md: \"What's new in 0.0.9\" is older than the library's version 0.1.0",
            "lib/b/CHANGELOG.md has no '## [Unreleased]' section",
            "lib/b/CHANGELOG.md: [0.0.9] is no release of CHANGELOG.md"]))

    def test_release_stage(self):
        self.tree("# jflib-b\n\n## What's new in 0.2.0\n")
        self.write("lib/b/CHANGELOG.md", changelog.crate_log("jflib-b", "0.1.0"))
        self.assertEqual(changelog.crate_doc_problems(self.dir), [])
        self.assertEqual(changelog.crate_doc_problems(self.dir, release="0.2.0"), [])
        self.assertEqual(changelog.crate_doc_problems(self.dir, release="0.3.0"), [
            "lib/a/README.md: \"What's new in 0.2.0\", the release is 0.3.0",
            "lib/b/README.md: \"What's new in 0.2.0\", the release is 0.3.0"])
        self.assertEqual(changelog.crate_doc_problems(self.dir, release="0.2.0", cut=True), [
            "lib/a/CHANGELOG.md has no [0.2.0]: python build_rs.py release 0.2.0",
            "lib/b/CHANGELOG.md has no [0.2.0]: python build_rs.py release 0.2.0"])
        readme = ('# jflib-b\n\n```toml\njflib = "0.2"\njflib-build = { version = "=0.1.0" }\n```\n\n'
                  "cp -r jflib-0.3.*/template .\n\n## What's new in 0.2.0\n")
        self.write("lib/b/README.md", readme)
        self.assertEqual(changelog.crate_doc_problems(self.dir), [
            "lib/b/README.md: an example names version 0.1, the README is for 0.2.0",
            "lib/b/README.md: an example names version 0.3, the README is for 0.2.0"])
        self.assertEqual(changelog.crate_of("b", self.dir), ("lib/b", "jflib-b", "jflib_b"))
        self.assertEqual(changelog.crate_of("jflib-a", self.dir)[0], "lib/a")
        with self.assertRaises(changelog.ChangelogError):
            changelog.crate_of("x", self.dir)


# ---------------------------------------------------------------- build_rs.py i18n
class I18nTest(Temp):
    """tools/i18n_check.py on a small app: what counts as a text, the report, --update"""
    MAIN = """use ui::i18n::{self, Catalog};
static NAMES: [&str; 2] = ["Rot", "Grün"];
static CMDS: [Command; 2] = [Command::new(CMD_OPEN, "Ö&ffnen", 0, 0, 0, 0), Command::new(CMD_X + 1, "&Neu", 0, 0, 0, 0)];
static MENU: [Mi; 2] = [Mi::Menu("&Datei", &M_FILE), Mi::Item(ID_INFO, "&Info", 0)];
fn install() {
    i18n::set_catalog(Catalog::from_tsv(include_str!("locales/ui.tsv")));
}
fn commands() -> Vec<Command> {
    CMDS.iter().map(|c| Command { label: t(c.label), ..*c }).collect()
}
fn menu(it: Mi) -> Mi {
    match it {
        Mi::Item(id, label, icon) => Mi::Item(id, t(label), icon),
        other => other,
    }
}
fn frame(ui: &mut Ui) {
    ui.text(i18n::t("Hallo"));
    ui.text(&tf("{} Dateien", &[&n]));
    ui.combobox(&list(&NAMES));
    ui.text(i18n::tr("Öffnen")); // the library's own text
    ui.text(t("Abbrechen")); // a library text: t falls back to it
    x.t("Methode");
    let s = "Nur ein Wert";
    // t("Kommentar")
}
// i18n
static TIPS: [&str; 2] = [
    "Erster Tipp",
    "Zweiter Tipp",
];
"""
    CORE = 'pub fn cable() -> Msg {\n    msg!("Kabel {} fehlt", 3)\n}\npub fn port() -> Msg {\n    msg!("Port {} belegt", 80)\n}\n'
    UI = ("de\ten\nHallo\tHello\n{} Dateien\t{0} files\nRot\tRed\nÖ&ffnen\t&Open\n&Datei\t&File\nNur ein Wert\tOnly one value\n"
          "Veraltet\tOld\nErster Tipp\t\n{n, plural, one {# Datei} other {# Dateien}}\t{n, plural, other {# files}}\n")
    ENGINE = "de\ten\n# the model's templates\nKabel {} fehlt\tCable {} missing\n"

    def app(self):
        self.write("demo/src/main.rs", self.MAIN)
        self.write("demo/crates/demo_core/lib.rs", self.CORE)
        self.write("demo/src/locales/ui.tsv", self.UI.replace("\n", "\r\n"))
        self.write("demo/src/locales/engine.tsv", self.ENGINE)
        return self.dir / "demo"

    def test_report(self):
        rep, _ = i18n_check.check_app(self.app())
        got = {c: sorted(w for k, _, w in rep.lines if k == c) for c in rep.count}
        self.assertEqual(rep.texts, 13)
        self.assertEqual(got["missing"], ["&Info", "&Neu", "Grün", "Port {} belegt", "Zweiter Tipp"])
        self.assertEqual(got["untranslated"], ["en: Erster Tipp"])
        self.assertEqual(got["unused"], ["Veraltet", "{n, plural, one {# Datei} other {# Dateien}}"])
        self.assertEqual(len(got["error"]), 2, got["error"])
        self.assertIn("i18n::fill takes 1", got["error"][0] + got["error"][1])
        self.assertIn("en needs one", got["error"][0] + got["error"][1])

    def test_update(self):
        app = self.app()
        ui, engine = app / "src/locales/ui.tsv", app / "src/locales/engine.tsv"
        before = ui.read_bytes()
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(i18n_check.main([str(app), "--update"]), 1)
        after = ui.read_bytes()
        self.assertTrue(after.startswith(before))
        added = after[len(before):].decode("utf-8")
        self.assertEqual(added, "\r\n".join([i18n_check.MARK, "Grün\t", "&Neu\t", "&Info\t", "Zweiter Tipp\t", ""]))
        self.assertTrue(engine.read_text(encoding="utf-8").endswith("Kabel {} fehlt\tCable {} missing\n" + i18n_check.MARK + "\nPort {} belegt\t\n"))
        rep, _ = i18n_check.check_app(app)
        self.assertEqual((rep.count["missing"], rep.count["untranslated"]), (0, 6))
        with contextlib.redirect_stdout(io.StringIO()):
            i18n_check.main([str(app), "--update"])
        self.assertEqual(ui.read_bytes(), after)        # nothing more to add

    def test_catalogue_headers(self):
        f = self.write("x/locales/a.tsv", "# languages: de en fr (read by the app)\n&Datei\t&File\t&Fichier\nNeu\tNew\n")
        c = i18n_check.Catalogue(f)
        self.assertEqual((c.langs, sorted(c.rows)), (["de", "en", "fr"], ["&Datei", "Neu"]))
        c = i18n_check.Catalogue(self.write("x/locales/b.tsv", "Ja\tYes\tOui\nJa\tYes\tOui\n"))
        self.assertEqual(c.langs, ["de", "column 2", "column 3"])
        self.assertIn("twice", c.problems[0][1])

    def test_placeholders(self):
        ok = [("{} von {}", "{1} of {0}", {"msg"}, "en"), ("Datei '{}' fehlt", "File '{}' is missing", {"fill"}, "en"),
              ("{n, plural, =0 {Keine} one {# Datei} other {# Dateien}} in {dir}", "{n, plural, =0 {None} one {# file} other {# files}} in {dir}", {"t"}, "en"),
              ("{n, plural, one {# Datei} other {# Dateien}}", "{n, plural, one {# plik} few {# pliki} many {# plików} other {# pliku}}", {"t"}, "pl"),
              ("{n, number} B", "{n, number} bytes", {"t"}, "en"), ("'{'x'}' {}", "'{'x'}' {}", {"t"}, "en")]
        for src, tr, kinds, lang in ok:
            self.assertEqual(i18n_check.compare(src, tr, kinds, lang), [], (src, tr))
        bad = [("{} Dateien", "{0} files", {"fill"}, "en", "i18n::fill"), ("{} von {}", "{0} of {0}", {"msg"}, "en", "differ"),
               ("Hallo {name}", "Hello {nom}", {"t"}, "en", "differ"), ("{n, number} B", "{n} bytes", {"t"}, "en", "differ"),
               ("{n, plural, one {# Datei} other {# Dateien}}", "{n, plural, one {# plik} other {# pliku}}", {"t"}, "pl", "pl needs few, many"),
               ("{n, plural, one {# Datei} other {# Dateien}}", "{n, plural, one {# file} few {# files} other {# files}}", {"t"}, "en", "never uses few"),
               ("{g, select, f {sie} other {es}}", "{g, select, f {she}}", {"t"}, "en", 'no "other"')]
        for src, tr, kinds, lang, why in bad:
            self.assertIn(why, " ".join(i18n_check.compare(src, tr, kinds, lang)), (src, tr))
        self.assertTrue(i18n_check.template_problems("Kabel {name}") and i18n_check.template_problems("{:>4}"))
        self.assertEqual(i18n_check.template_problems("Kabel {} an {0}"), [])

    def test_library(self):
        """LIB_TEXTS has every tr("…") of lib/ and the same placeholders in every language"""
        rep = i18n_check.check_lib()
        self.assertEqual([x for x in rep.lines if x[0] in ("missing", "untranslated", "error")], [])
        self.assertGreater(rep.texts, 100)

    def test_app_outside(self):
        """a cargo app outside the repository, given by its path: its places from the folder that
        holds it; build output (target/), hidden folders and git submodules do not count"""
        with tempfile.TemporaryDirectory(prefix="i18n-outside-") as tmp:
            app = Path(tmp) / "Netz"
            files = {
                "src/main.rs": 'fn frame() {\n    ui.text(tr("Hallo"));\n    ui.text(tr("Fehlt"));\n}\n',
                "src/locales/ui.tsv": "de\ten\nHallo\tHello\n",
                "Cargo.toml": '[package]\nname = "Netz"\n',
                "target/debug/build/netz-1/out/gen.rs": 'fn x() { tr("Erzeugt"); }\n',
                "target/package/Netz-0.1.0/src/locales/ui.tsv": "de\ten\nAlt\tOld\n",
                "engine/.git": "gitdir: ../.git/modules/engine\n",
                "engine/src/lib.rs": 'fn y() { t("Motor"); }\n',
                "engine/locales/engine.tsv": "de\ten\nMotor\t\n",
                ".cargo/x.rs": 'fn z() { t("Versteckt"); }\n',
            }
            for rel, text in files.items():
                (app / rel).parent.mkdir(parents=True, exist_ok=True)
                (app / rel).write_text(text, encoding="utf-8", newline="\n")
            self.assertEqual(i18n_check.app_folders(app), ["src"])
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                self.assertEqual(i18n_check.main([str(app)]), 1)
            got = out.getvalue()
            self.assertIn("Netz: 2 texts (src/locales/ui.tsv; de en): 1 missing, 0 untranslated, 0 unused, 0 errors", got)
            self.assertIn("Netz/src/main.rs:3\tmissing\tFehlt", got)
            for other in ("Erzeugt", "Motor", "Versteckt", "Alt"):
                self.assertNotIn(other, got)
            with contextlib.redirect_stdout(io.StringIO()):
                i18n_check.main([str(app), "--update"])
            self.assertTrue((app / "src/locales/ui.tsv").read_text(encoding="utf-8").endswith("Fehlt\t\n"))
            # relative paths from the current folder ("." inside the app, "Netz" and "./Netz"
            # beside it); a bare name of the repository's apps stays apps/NAME
            old = os.getcwd()
            try:
                for cwd, word in ((app, "."), (app / "src", ".."), (Path(tmp), "Netz"), (Path(tmp), "./Netz")):
                    os.chdir(cwd)
                    self.assertEqual(i18n_check.app_path(Path(word)).resolve(), app.resolve(), (cwd, word))
                    out = io.StringIO()
                    with contextlib.redirect_stdout(out):
                        self.assertNotEqual(i18n_check.main([word]), 2, (cwd, word))
                    self.assertIn("Netz: 2 texts (src/locales/ui.tsv; de en): 0 missing", out.getvalue())
                os.chdir(Path(tmp))
                self.assertEqual(i18n_check.app_path(Path("gallery")), i18n_check.ROOT / "apps" / "gallery")
                self.assertEqual(i18n_check.app_path(Path("apps/gallery")), i18n_check.ROOT / "apps" / "gallery")
            finally:
                os.chdir(old)


class ScriptTimeout(Temp):
    """build_rs.run_app_tests: a script that does not end has failed, the next one still runs"""

    def test_timeout_fails_only_that_script(self):
        tests = self.dir / "apps" / "x" / "tests"
        tests.mkdir(parents=True)
        (tests / "a.uis").write_text("#!args --a\n", encoding="utf-8")
        (tests / "b.uis").write_text("#!args --b\n#!timeout 7\n", encoding="utf-8")
        limits = {}

        def run(_target, _exe, args, timeout):
            limits[args[0]] = timeout
            if args[0] == "--a":
                raise subprocess.TimeoutExpired("x", timeout, output=b"partial\n")
            return subprocess.CompletedProcess("x", 0, "", "")

        with mock.patch.object(build_rs, "ROOT", self.dir), mock.patch.object(build_rs, "run_exe", run):
            a, b = build_rs.run_app_tests("x", "linux", "x")
        self.assertEqual((a["status"], a["exit_code"], b["status"]), ("failed", "timeout", "passed"))
        self.assertEqual(limits, {"--a": build_rs.SCRIPT_TIMEOUT, "--b": 7})
        self.assertIn("partial", a["output"])
        self.assertTrue(a["output"].startswith("FAIL"))


class TestJobs(Temp):
    """build_rs.py test side by side (section "test-jobs"): the markers, apps/NAME/tests/jobs.txt, the pool's rules"""

    def app(self, name, scripts, jobs=None):
        for file, text in scripts.items():
            self.write(f"apps/{name}/tests/{file}", text)
        if jobs is not None:
            self.write(f"apps/{name}/tests/jobs.txt", jobs)

    def test_markers_and_rules(self):
        self.app("x", {"a.uis": "#!args --a\n#!serial flaky under load\n", "b.uis": "#!lock desktop audio\nclick 1,1\n\n\n\n#!serial\n",
                       "c.uis": "# none\n"}, "independent   # they share nothing\nlock audio\n")
        self.app("y", {"a.uis": "#!seral\n", "b.uis": "#!lock gpu\n"}, "serial the whole app\n")
        self.app("z", {"a.uis": ""}, "parallel\n")
        with mock.patch.object(build_rs, "ROOT", self.dir), mock.patch.object(build_rs, "apps", lambda: ["x", "y", "z"]):
            t = self.dir / "apps" / "x" / "tests"
            self.assertEqual(build_rs.script_markers(t / "a.uis"), (True, set()))
            self.assertEqual(build_rs.script_markers(t / "b.uis"), (False, {"desktop", "audio"}))     # line 6: no marker
            self.assertEqual(build_rs.app_test_rules("x"), dict(independent=True, serial=False, locks={"audio"}))
            self.assertEqual(build_rs.app_test_rules("y"), dict(independent=False, serial=True, locks=set()))
            self.assertRaises(build_rs.BuildError, build_rs.app_test_rules, "z")
            bad = build_rs.test_marker_lint()
        self.assertEqual(len(bad), 3, bad)
        self.assertIn("apps/y/tests/a.uis:1: unknown marker #!seral", bad[0])
        self.assertIn("apps/y: unknown test lock gpu", bad[1])
        self.assertIn("apps/z/tests/jobs.txt:1:", bad[2])

    def run_pool(self, jobs, times=None):
        """apps c (in order), i (independent), d1 / d2 (one script each holding the desktop lock), s (one serial
        script in the middle of its app's order) on win and linux -> [(tag, start, end)] in the order they started"""
        self.app("c", {f"{n}.uis": "" for n in "abc"})
        self.app("i", {f"{n}.uis": "" for n in "abcd"}, "independent\n")
        self.app("d1", {"a.uis": "#!lock desktop\n"})
        self.app("d2", {"a.uis": "#!lock desktop\n"})
        self.app("s", {"a.uis": "", "b.uis": "#!serial under load\n", "c.uis": ""})
        spans, lock = [], build_rs.threading.Lock()

        def run(target, exe, script):
            start = time.monotonic()
            time.sleep(0.15)
            with lock:
                spans.append((f"{target} {script.parent.parent.name}/{script.name}", start, time.monotonic()))
            return dict(script=script.name, status="passed", exit_code=0, start=start, duration=0.15, output="")

        runs = [(t, t, n, "exe") for t in ("win", "linux") for n in ("c", "d1", "d2", "i", "s")]
        with mock.patch.object(build_rs, "ROOT", self.dir), mock.patch.object(build_rs, "run_script", run):
            units, skipped = build_rs.test_units(runs, jobs)
            records = build_rs.run_test_units(build_rs.order_test_units(units, times or {}, jobs), jobs, lambda u, r: None)
        self.assertEqual((len(units), skipped, len(records)), (24, 0, 24))
        return sorted(spans, key=lambda s: s[1])

    @staticmethod
    def overlap(a, b):
        return a[1] < b[2] and b[1] < a[2]

    def test_pool_rules(self):
        spans = self.run_pool(4)
        by = {s[0]: s for s in spans}
        order = [s[0] for s in spans]
        for t in ("win", "linux"):                          # an app's order on each target
            self.assertLess(order.index(f"{t} c/a.uis"), order.index(f"{t} c/b.uis"))
            self.assertLess(order.index(f"{t} c/b.uis"), order.index(f"{t} c/c.uis"))
            self.assertLess(order.index(f"{t} s/a.uis"), order.index(f"{t} s/c.uis"))
        for app in ("c", "s"):                              # never two scripts of an app at once, on any target
            mine = [s for s in spans if s[0].split()[1].startswith(app + "/") and not s[0].endswith("s/b.uis")]
            self.assertFalse([(a[0], b[0]) for a in mine for b in mine if a is not b and self.overlap(a, b)])
        desk = [s for s in spans if s[0].split()[1].startswith("d")]
        self.assertFalse([(a[0], b[0]) for a in desk for b in desk if a is not b and self.overlap(a, b)])
        ind = [s for s in spans if " i/" in s[0]]           # independent: side by side, but never one script twice
        self.assertTrue(any(self.overlap(a, b) for a in ind for b in ind if a is not b))
        for n in "abcd":
            self.assertFalse(self.overlap(by[f"win i/{n}.uis"], by[f"linux i/{n}.uis"]))
        serial = [by["win s/b.uis"], by["linux s/b.uis"]]   # alone, after the rest, in the suite's order
        self.assertEqual(order[-2:], ["win s/b.uis", "linux s/b.uis"])
        self.assertTrue(all(s[1] >= max(o[2] for o in spans if o not in serial) for s in serial))

    def test_one_job_keeps_the_suites_order(self):
        order = [s[0] for s in self.run_pool(1, times={"win/i/a.uis": 99.0})]
        expect = [f"{t} {n}" for t in ("win", "linux") for n in
                  ("c/a.uis", "c/b.uis", "c/c.uis", "d1/a.uis", "d2/a.uis", "i/a.uis", "i/b.uis", "i/c.uis", "i/d.uis",
                   "s/a.uis", "s/b.uis", "s/c.uis")]
        self.assertEqual(order, expect)

    def test_longest_work_first(self):
        units = [build_rs.TestUnit("win", "win", app, Path(f"{n}.uis"), "x", False, {f"app:{app}"}, i)
                 for i, (app, n) in enumerate([("a", "1"), ("a", "2"), ("b", "1"), ("c", "1")])]
        times = {"win/a/1.uis": 1.0, "win/a/2.uis": 1.0, "win/b/1.uis": 30.0}     # c: the median (1 s)
        self.assertEqual([u.key for u in build_rs.order_test_units(units, times, 4)],
                         ["win/b/1.uis", "win/a/1.uis", "win/a/2.uis", "win/c/1.uis"])
        self.assertEqual(build_rs.order_test_units(units, times, 1), units)
        with mock.patch.dict(os.environ, {"jfgui_TEST_JOBS": "3"}):
            self.assertEqual((build_rs.test_jobs(), build_rs.test_jobs(5)), (3, 5))
        with mock.patch.dict(os.environ, {"jfgui_TEST_JOBS": ""}):
            self.assertEqual(build_rs.test_jobs(), max(1, (os.cpu_count() or 2) // 2))


class IncrementalCache(Temp):
    """build_rs.incremental_flags: the state a rustc killed during ThinLTO leaves goes, the others stay"""

    def session(self, stem, suffix, keys_age=None):
        """inc/spell/spell-x/s-STEM-SUFFIX with its lock file (100 s old) and keys (KEYS_AGE s old)"""
        now = time.time()
        d = self.dir / "inc" / "spell" / "spell-x" / f"s-{stem}-{suffix}"
        d.mkdir(parents=True)
        lock = d.with_name(f"s-{stem}.lock")
        lock.touch()
        os.utime(lock, (now - 100, now - 100))
        if keys_age is not None:
            (d / "thin-lto-past-keys.bin").touch()
            os.utime(d / "thin-lto-past-keys.bin", (now - keys_age, now - keys_age))
        return lock

    def check(self):
        inc = self.dir / "inc" / "spell"
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(build_rs.incremental_flags(inc), ["-C", f"incremental={inc}"])
        return inc.exists(), out.getvalue()

    def test_no_cache_yet(self):
        self.assertEqual(self.check(), (False, ""))

    def test_finished_session_stays(self):
        self.session("hnr7aaaaaa-0000001", "svh", keys_age=0)
        self.assertEqual(self.check(), (True, ""))

    def test_compile_error_keeps_the_cache(self):
        self.session("hnr7aaaaaa-0000001", "svh", keys_age=500)
        self.session("hnr7bbbbbb-0000002", "working", keys_age=500)
        self.assertEqual(self.check(), (True, ""))

    def test_killed_before_thinlto_keeps_the_cache(self):
        self.session("hnr7bbbbbb-0000002", "working")
        self.assertEqual(self.check(), (True, ""))

    def test_killed_during_thinlto_discards_the_cache(self):
        self.session("hnr7aaaaaa-0000001", "svh", keys_age=50)
        self.session("hnr7bbbbbb-0000002", "working", keys_age=50)
        kept, out = self.check()
        self.assertFalse(kept)
        self.assertIn("killed during ThinLTO", out)

    def test_compile_error_before_a_finished_build_keeps_the_cache(self):
        """the -working folder shares its keys with the later session, so they look rewritten"""
        self.session("hnr7aaaaaa-0000001", "working", keys_age=0)
        self.session("hnr7bbbbbb-0000002", "svh", keys_age=0)
        self.assertEqual(self.check(), (True, ""))

    def test_running_rustc_keeps_the_cache(self):
        lock = self.session("hnr7bbbbbb-0000002", "working", keys_age=50)
        with open(lock, "r+b") as f:
            if os.name == "nt":
                import msvcrt
                msvcrt.locking(f.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.assertEqual(self.check(), (True, ""))


class AndroidManifest(unittest.TestCase):
    """an APK must not require Vulkan: the phones without it have the Bitmap path (tools/android.py)"""
    def test_vulkan_not_required(self):
        import android as tool
        need = '<uses-feature android:name="android.hardware.vulkan.level" />'
        optional = '<uses-feature android:name="android.hardware.vulkan.version" android:required="false" />'
        self.assertIn("requires Vulkan", tool.manifest_gaps(None, need))
        self.assertIsNone(tool.manifest_gaps(None, optional))
        self.assertIsNone(tool.manifest_gaps(None, '<uses-feature android:name="android.hardware.touchscreen" />'))


if __name__ == "__main__":
    unittest.main(verbosity=1)
