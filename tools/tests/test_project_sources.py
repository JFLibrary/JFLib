"""Regression tests for entry ambiguity, source exclusions and shared resources."""
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from tools.project_sources import SourceError, entrypoint, project_sources


class ProjectSourcesTest(unittest.TestCase):
    def setUp(self):
        (ROOT / "build").mkdir(exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(prefix="sources-", dir=ROOT / "build")
        self.directory = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()

    def write(self, name, text=""):
        path = self.directory / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(text.encode())
        return path

    def test_legacy_and_modular_and_ambiguity(self):
        legacy = self.write("app/main.rs")
        self.assertEqual(entrypoint(legacy.parent, "app"), legacy)
        modular = self.write("app/src/main.rs")
        with self.assertRaisesRegex(SourceError, "competing entry points"):
            entrypoint(legacy.parent, "app")
        legacy.unlink()
        self.assertEqual(entrypoint(legacy.parent, "app"), modular)

    def test_references_are_not_sources(self):
        root = self.write("app/main.rs", 'mod editor;')
        editor = self.write("app/editor.rs")
        for folder in ("context/wordcraft", "books", "build", "fixtures", "tests"):
            self.write("app/" + folder + "/foreign.rs", "use std::fs;")
        self.assertEqual(project_sources(root), sorted([root.resolve(), editor.resolve()]))

    def test_explicit_shared_sources_and_embedded_assets(self):
        root = self.write("lib/src/lib.rs", '#[path="../../shared/mod.rs"] mod shared;')
        shared = self.write("shared/mod.rs", 'include_bytes!("font.bin");')
        asset = self.write("shared/font.bin", "font-data")
        self.assertEqual(project_sources(root), sorted([root.resolve(), shared.resolve()]))
        self.assertEqual(project_sources(root, assets=True), sorted([root.resolve(), shared.resolve(), asset.resolve()]))

    def test_missing_reference_fails(self):
        root = self.write("lib/src/lib.rs", 'include_bytes!("missing.bin");')
        with self.assertRaisesRegex(SourceError, "does not exist"):
            project_sources(root)


if __name__ == "__main__":
    unittest.main()
