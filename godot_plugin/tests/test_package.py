"""The zip builder and the plugin's static contract.

Godot installs what this builder produces, so the zip's shape is a contract:
one `addons/assetcraft_godot/` folder holding the plugin, its configuration,
and the manifest reader — and nothing else.
"""

import importlib.util
import tomllib
import unittest
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
ROOT = HERE.parent


def load_package_module():
    spec = importlib.util.spec_from_file_location("assetcraft_godot_package", HERE / "package.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def read_plugin_cfg() -> dict:
    values = {}
    for line in (HERE / "assetcraft_godot" / "plugin.cfg").read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("[") or line.startswith(";") or "=" not in line:
            continue
        key, _, value = line.partition("=")
        values[key.strip()] = value.strip().strip('"')
    return values


class BuildTest(unittest.TestCase):
    def setUp(self):
        self.package = load_package_module()
        holder = __import__("tempfile").TemporaryDirectory()
        self.addCleanup(holder.cleanup)
        self.zip_path = self.package.build(holder.name)
        with zipfile.ZipFile(self.zip_path) as archive:
            self.names = set(archive.namelist())

    def test_it_builds_a_zip_named_for_the_module(self):
        self.assertTrue(self.zip_path.is_file())
        self.assertEqual(self.zip_path.name, "assetcraft_godot.zip")
        self.assertTrue(zipfile.is_zipfile(self.zip_path))

    def test_the_zip_holds_the_module_under_addons(self):
        self.assertEqual(
            self.names,
            {
                "addons/assetcraft_godot/plugin.cfg",
                "addons/assetcraft_godot/plugin.gd",
                "addons/assetcraft_godot/exports.gd",
                "addons/assetcraft_godot/importer.gd",
                "addons/assetcraft_godot/README.md",
            },
        )

    def test_no_tests_no_caches(self):
        self.assertFalse([name for name in self.names if "tests" in name])
        self.assertFalse([name for name in self.names if "__pycache__" in name])


class PluginContractTest(unittest.TestCase):
    def test_the_config_names_the_script_and_the_project_version(self):
        cfg = read_plugin_cfg()
        self.assertEqual(cfg["script"], "plugin.gd")
        self.assertEqual(cfg["name"], "AssetCraft")

        with open(ROOT / "Cargo.toml", "rb") as handle:
            workspace = tomllib.load(handle)
        self.assertEqual(cfg["version"], workspace["workspace"]["package"]["version"])

    def test_the_plugin_is_an_editor_plugin(self):
        text = (HERE / "assetcraft_godot" / "plugin.gd").read_text()
        self.assertTrue(text.startswith("@tool"))
        self.assertIn("extends EditorPlugin", text)

    def test_the_reader_is_editor_free(self):
        # The reader is what the headless tests exercise; it must not reach for
        # editor types, or those tests stop meaning anything.
        text = (HERE / "assetcraft_godot" / "exports.gd").read_text()
        self.assertNotIn("EditorPlugin", text)
        self.assertNotIn("EditorInterface", text)
        self.assertIn('const MANIFEST_NAME := "assetcraft_export.json"', text)


if __name__ == "__main__":
    unittest.main()
