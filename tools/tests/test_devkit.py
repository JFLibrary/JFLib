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
import re
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools"))
sys.path.insert(0, str(ROOT))
import apidiff  # noqa: E402
import changelog  # noqa: E402
import gendoc  # noqa: E402
import i18n_check  # noqa: E402
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
            for rel in [r for r in files if r.endswith(".rs")]:
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
        paths = scaffold.create_app(self.dir, "hello", "tool", "winui", "Hallo", ["ui"], ["winui"], "2026-10-20")
        self.assertIn(self.dir / "apps" / "hello" / "crates" / "hello_core" / "lib.rs", paths)
        self.assertTrue(all(p.exists() and b"\r\n" not in p.read_bytes() for p in paths))
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
        build = (ROOT / "build_rs.py").read_text(encoding="utf-8")
        gd = (ROOT / "tools" / "gendoc.py").read_text(encoding="utf-8")
        self.write("build_rs.py", build.replace("\n", "\r\n"))
        self.write("tools/gendoc.py", gd)
        libs = list(build_rs.LIBS)
        with self.assertRaisesRegex(scaffold.ScaffoldError, "unknown dependencies"):
            scaffold.create_lib(self.dir, "mylib", ["nope"], None, libs, libs)
        paths = scaffold.create_lib(self.dir, "mylib", ["plat", "rt"], "my things", libs, libs)
        nb = (self.dir / "build_rs.py").read_bytes()
        self.assertEqual(nb.count(b"\n"), nb.count(b"\r\n"), "the file keeps its CRLF line ends")
        text = nb.decode("utf-8")
        line = re.search(r'^    "mylib": \("lib/mylib/src/lib\.rs", \["rt", "plat"\]\),  # my things\r$', text, re.M)
        self.assertTrue(line, "LIBS entry in build order")
        libs_block = re.search(r"^LIBS = \{\r\n.*?^\}\r\n", text, re.M | re.S).group(0)
        self.assertTrue(libs_block.endswith("# my things\r\n}\r\n"), "the last entry of LIBS")
        self.assertIn('    ("mylib", "my things", "full"),\n]', (self.dir / "tools" / "gendoc.py").read_text(encoding="utf-8"))
        lib = (self.dir / "lib" / "mylib" / "src" / "lib.rs").read_text(encoding="utf-8")
        self.assertIn("#![forbid(unsafe_code)]", lib)
        self.assertIn(self.dir / "lib" / "mylib" / "src" / "lib.rs", paths)
        with self.assertRaisesRegex(scaffold.ScaffoldError, "exists already"):
            scaffold.create_lib(self.dir, "mylib", [], None, libs, libs)

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
        """the release snapshots in docs/api read back, and lib/VERSION is a version"""
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

    def test_repository_changelog(self):
        """CHANGELOG.md has [Unreleased] and a section of the version in lib/VERSION"""
        text, _ = changelog.read()
        self.assertIn(apidiff.lib_version(), changelog.releases(text))
        self.assertTrue(changelog.UNRELEASED.search(text))


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


if __name__ == "__main__":
    unittest.main(verbosity=1)
