"""Regression tests for tools/android.py: a release APK is never debuggable."""
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
from tools.android import debuggable_attribute


class DebuggableTest(unittest.TestCase):
    def test_refuses_debuggable_manifests(self):
        for tag in ('<application android:debuggable="true">', "<application android:debuggable = 'true'>",
                    '<application android:debuggable="@bool/debug">'):
            self.assertIsNotNone(debuggable_attribute(tag), tag)

    def test_allows_manifests_without_it(self):
        self.assertIsNone(debuggable_attribute('<application android:debuggable="false">'))
        self.assertIsNone(debuggable_attribute('<application android:allowBackup="false">'))

    def test_repository_manifests_are_not_debuggable(self):
        for manifest in sorted(ROOT.glob('apps/*/android/AndroidManifest.xml')):
            self.assertIsNone(debuggable_attribute(manifest.read_text(encoding='utf-8')), manifest)


if __name__ == '__main__':
    unittest.main()
