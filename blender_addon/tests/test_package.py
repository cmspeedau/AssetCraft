"""The zip builder, because Blender installs what it produces verbatim."""

import importlib.util
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]


def load_package_module():
    spec = importlib.util.spec_from_file_location("assetcraft_package", HERE / "package.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class BuildTest(unittest.TestCase):
    def setUp(self):
        self.package = load_package_module()
        holder = tempfile.TemporaryDirectory()
        self.addCleanup(holder.cleanup)
        self.dest = holder.name
        self.zip_path = self.package.build(self.dest)

    def test_it_builds_a_zip_named_for_the_module(self):
        self.assertTrue(self.zip_path.is_file())
        self.assertEqual(self.zip_path.name, self.package.MODULE + ".zip")
        self.assertTrue(zipfile.is_zipfile(self.zip_path))

    def test_the_zip_holds_the_module_under_its_own_name(self):
        # Blender installs a zip whose single top-level folder becomes the
        # addon's module; anything looser installs wrong or not at all.
        with zipfile.ZipFile(self.zip_path) as archive:
            names = set(archive.namelist())
        self.assertIn("assetcraft_blender/__init__.py", names)
        self.assertIn("assetcraft_blender/exports.py", names)
        for name in names:
            self.assertTrue(name.startswith("assetcraft_blender/"), name)

    def test_no_tests_no_caches(self):
        with zipfile.ZipFile(self.zip_path) as archive:
            names = list(archive.namelist())
        self.assertFalse([n for n in names if "tests" in n])
        self.assertFalse([n for n in names if "__pycache__" in n])

    def test_the_packed_exports_is_importable_and_blender_free(self):
        # What ships must be exactly what was tested: `exports` out of the zip,
        # with no bpy to lean on.
        with tempfile.TemporaryDirectory() as unpacked:
            with zipfile.ZipFile(self.zip_path) as archive:
                archive.extractall(unpacked)
            sys.path.insert(0, str(Path(unpacked) / "assetcraft_blender"))
            try:
                sys.modules.pop("bpy", None)
                import exports

                self.assertTrue(callable(exports.discover))
                self.assertTrue(callable(exports.load_manifest))
            finally:
                sys.path.remove(str(Path(unpacked) / "assetcraft_blender"))
                sys.modules.pop("exports", None)


if __name__ == "__main__":
    unittest.main()
